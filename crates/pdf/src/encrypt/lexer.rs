// ---------- PDF syntax ----------

pub(crate) fn is_white(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_regular(b: u8) -> bool {
    !is_white(b) && !b"()<>[]{}/%".contains(&b)
}

#[derive(Debug)]
pub(crate) enum Tok<'a> {
    DictOpen,
    DictClose,
    ArrayOpen,
    ArrayClose,
    Name(&'a [u8]),
    Str(Vec<u8>),
    /// Numbers and keywords (`true`, `R`, `obj`, `stream`…).
    Word(&'a [u8]),
}

/// A token with its byte span.
pub(crate) type Spanned<'a> = (Tok<'a>, usize, usize);

/// Deepest nesting of arrays and dictionaries read.
const MAX_DEPTH: usize = 64;

/// A parsed object, with what encryption needs: the strings and dictionaries with their spans.
#[derive(Debug)]
pub(crate) enum Obj<'a> {
    /// `start` is where `<<` is, `end` just after `>>`.
    Dict {
        entries: Vec<(&'a [u8], Obj<'a>)>,
        start: usize,
        end: usize,
    },
    Array(Vec<Obj<'a>>),
    Str {
        bytes: Vec<u8>,
        start: usize,
        end: usize,
    },
    Name(&'a [u8]),
    Int {
        value: i64,
        start: usize,
        end: usize,
    },
    Ref(u32, u16),
    /// Reals, booleans, null.
    Other,
}

impl<'a> Obj<'a> {
    pub(crate) fn get(&self, key: &[u8]) -> Option<&Obj<'a>> {
        match self {
            Self::Dict { entries, .. } => entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn name(&self, key: &[u8]) -> Option<&'a [u8]> {
        match self.get(key)? {
            Self::Name(n) => Some(n),
            _ => None,
        }
    }

    pub(crate) fn int(&self, key: &[u8]) -> Option<i64> {
        match self.get(key)? {
            Self::Int { value, .. } => Some(*value),
            _ => None,
        }
    }

    pub(super) fn string(&self, key: &[u8]) -> Option<&[u8]> {
        match self.get(key)? {
            Self::Str { bytes, .. } => Some(bytes),
            _ => None,
        }
    }

    /// The first string of the array at `key` (a file's ID).
    pub(super) fn first_string(&self, key: &[u8]) -> Option<&[u8]> {
        match self.get(key)? {
            Self::Array(items) => items.iter().find_map(|i| match i {
                Self::Str { bytes, .. } => Some(bytes.as_slice()),
                _ => None,
            }),
            _ => None,
        }
    }

    /// Every string in this object, in file order.
    pub(super) fn strings<'s>(&'s self, out: &mut Vec<&'s Obj<'a>>) {
        match self {
            Self::Str { .. } => out.push(self),
            Self::Dict { entries, .. } => entries.iter().for_each(|(_, v)| v.strings(out)),
            Self::Array(items) => items.iter().for_each(|v| v.strings(out)),
            _ => {}
        }
    }
}

pub(crate) fn int(w: &[u8]) -> Option<i64> {
    std::str::from_utf8(w).ok()?.parse().ok()
}

#[derive(Clone)]
pub(crate) struct Lexer<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn at(buf: &'a [u8], pos: usize) -> Self {
        Self { buf, pos }
    }

    fn peek_byte(&self, ahead: usize) -> Option<u8> {
        self.buf.get(self.pos + ahead).copied()
    }

    fn skip_space(&mut self) {
        while let Some(b) = self.peek_byte(0) {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while self.peek_byte(0).is_some_and(|b| b != b'\n' && b != b'\r') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub(crate) fn next(&mut self) -> Option<Spanned<'a>> {
        self.skip_space();
        let start = self.pos;
        let b = self.peek_byte(0)?;
        let tok = match (b, self.peek_byte(1)) {
            (b'<', Some(b'<')) => {
                self.pos += 2;
                Tok::DictOpen
            }
            (b'>', Some(b'>')) => {
                self.pos += 2;
                Tok::DictClose
            }
            (b'<', _) => Tok::Str(self.hex()?),
            (b'(', _) => Tok::Str(self.literal()?),
            (b'[', _) => {
                self.pos += 1;
                Tok::ArrayOpen
            }
            (b']', _) => {
                self.pos += 1;
                Tok::ArrayClose
            }
            (b'/', _) => {
                self.pos += 1;
                self.word();
                Tok::Name(self.buf.get(start + 1..self.pos)?)
            }
            _ if !is_regular(b) => {
                // A stray delimiter.
                self.pos += 1;
                Tok::Word(self.buf.get(start..self.pos)?)
            }
            _ => {
                self.word();
                Tok::Word(self.buf.get(start..self.pos)?)
            }
        };
        Some((tok, start, self.pos))
    }

    fn word(&mut self) {
        while self.peek_byte(0).is_some_and(is_regular) {
            self.pos += 1;
        }
    }

    fn hex(&mut self) -> Option<Vec<u8>> {
        self.pos += 1;
        let (mut out, mut high) = (vec![], None);
        loop {
            let b = self.peek_byte(0)?;
            self.pos += 1;
            if b == b'>' {
                out.extend(high.map(|h: u8| h << 4));
                return Some(out);
            }
            if is_white(b) {
                continue;
            }
            let v = char::from(b).to_digit(16)? as u8;
            match high.take() {
                Some(h) => out.push(h << 4 | v),
                None => high = Some(v),
            }
        }
    }

    fn literal(&mut self) -> Option<Vec<u8>> {
        self.pos += 1;
        let (mut out, mut depth) = (vec![], 1u32);
        loop {
            let b = self.peek_byte(0)?;
            self.pos += 1;
            match b {
                b'\\' => {
                    let e = self.peek_byte(0)?;
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        // A backslash before an end of line continues the string.
                        b'\r' => {
                            if self.peek_byte(0) == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek_byte(0) {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xFF) as u8);
                        }
                        // `\(`, `\)`, `\\`; other escapes stand for the character.
                        _ => out.push(e),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(out);
                    }
                    out.push(b);
                }
                // Ends of lines read as `\n`.
                b'\r' => {
                    if self.peek_byte(0) == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
    }

    /// One object (a reference `n g R` reads as one).
    pub(crate) fn object(&mut self, depth: usize) -> Option<Obj<'a>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let (tok, start, end) = self.next()?;
        Some(match tok {
            Tok::DictOpen => {
                let mut entries = vec![];
                loop {
                    let mut look = self.clone();
                    match look.next()? {
                        (Tok::DictClose, _, end) => {
                            *self = look;
                            return Some(Obj::Dict { entries, start, end });
                        }
                        (Tok::Name(k), ..) => {
                            *self = look;
                            entries.push((k, self.object(depth + 1)?));
                        }
                        _ => return None,
                    }
                }
            }
            Tok::ArrayOpen => {
                let mut items = vec![];
                loop {
                    let mut look = self.clone();
                    if matches!(look.next()?, (Tok::ArrayClose, ..)) {
                        *self = look;
                        return Some(Obj::Array(items));
                    }
                    items.push(self.object(depth + 1)?);
                }
            }
            Tok::Str(bytes) => Obj::Str { bytes, start, end },
            Tok::Name(n) => Obj::Name(n),
            Tok::Word(w) => match int(w) {
                Some(value) => {
                    let mut look = self.clone();
                    match (look.next(), look.next()) {
                        (Some((Tok::Word(g), ..)), Some((Tok::Word(b"R"), ..))) => {
                            match (u32::try_from(value), int(g).and_then(|g| u16::try_from(g).ok())) {
                                (Ok(n), Some(g)) => {
                                    *self = look;
                                    Obj::Ref(n, g)
                                }
                                _ => Obj::Int { value, start, end },
                            }
                        }
                        _ => Obj::Int { value, start, end },
                    }
                }
                None => Obj::Other,
            },
            Tok::DictClose | Tok::ArrayClose => return None,
        })
    }

    /// The header `num generation obj` of an indirect object.
    pub(crate) fn header(&mut self) -> Option<(u32, u16)> {
        let (Some((Tok::Word(n), ..)), Some((Tok::Word(g), ..)), Some((Tok::Word(b"obj"), ..))) = (self.next(), self.next(), self.next()) else {
            return None;
        };
        Some((u32::try_from(int(n)?).ok()?, u16::try_from(int(g)?).ok()?))
    }
}
