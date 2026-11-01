//! Operand values, and the dictionaries the editing data writes postfix: `/Type :` opens one,
//! `values key ,` adds an entry (a type name such as `/Real` may stand before the key; in an
//! `/Array` there is no key), `;` closes it (after a last `values /key` entry, or a bare value: its
//! content), leaving it as a value.

use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum V {
    Num(f64),
    Str(Vec<u8>),
    Name(String),
    Arr(Vec<V>),
    Obj(Rc<Obj>),
    /// `[`, until its `]`.
    Mark,
}

impl V {
    pub fn num(&self) -> Option<f64> {
        match self {
            V::Num(v) => Some(*v),
            _ => None,
        }
    }
    pub fn obj(&self) -> Option<&Obj> {
        match self {
            V::Obj(o) => Some(o),
            _ => None,
        }
    }
    /// A string's or name's text.
    pub fn text(&self) -> Option<String> {
        match self {
            V::Str(s) => Some(text(s)),
            V::Name(n) => Some(n.clone()),
            _ => None,
        }
    }
}

/// Text from the data's bytes: UTF-8, else Windows-1252 (Latin-1 for its gaps).
pub(super) fn text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|b| cp1252(*b)).collect(),
    }
}

/// The encoding names in the editing data that aren't UTF-8 are read in (layers, objects,
/// artboards, colors, gradients, patterns, graphic styles, swatches). Legacy files store them in
/// the code page of the system that wrote them and don't say which.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameEncoding {
    /// Windows-1252, Latin-1 for its gaps.
    #[default]
    Windows1252,
    /// Mac OS Roman.
    Macintosh,
    /// Korean: Windows code page 949 (EUC-KR and Unified Hangul Code).
    Korean,
    /// Japanese: Shift_JIS (Windows code page 932).
    ShiftJis,
    /// Simplified Chinese: GBK (Windows code page 936).
    Gbk,
    /// Traditional Chinese: Big5 (Windows code page 950).
    Big5,
}

impl NameEncoding {
    /// The labels [`NameEncoding::from_label`] reads, one for each encoding.
    pub const LABELS: [&str; 6] = ["windows-1252", "macintosh", "windows-949", "shift_jis", "gbk", "big5"];

    /// The encoding with label `label` (any case): one of [`Self::LABELS`], or `euc-kr` for
    /// [`NameEncoding::Korean`].
    pub fn from_label(label: &str) -> Option<Self> {
        let label = label.trim().to_ascii_lowercase();
        Some(match label.as_str() {
            "windows-1252" => Self::Windows1252,
            "macintosh" => Self::Macintosh,
            "windows-949" | "euc-kr" => Self::Korean,
            "shift_jis" => Self::ShiftJis,
            "gbk" => Self::Gbk,
            "big5" => Self::Big5,
            _ => return None,
        })
    }

    /// The encoding's label, as [`Self::LABELS`] gives it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Windows1252 => "windows-1252",
            Self::Macintosh => "macintosh",
            Self::Korean => "windows-949",
            Self::ShiftJis => "shift_jis",
            Self::Gbk => "gbk",
            Self::Big5 => "big5",
        }
    }

    /// `bytes` as text: UTF-8 as it is, else read in this encoding, with each sequence it doesn't
    /// define as U+FFFD (Windows-1252 defines every byte).
    fn decode(self, bytes: &[u8]) -> Decoded {
        if let Ok(s) = std::str::from_utf8(bytes) {
            return Decoded { text: s.to_string(), fallback: false, replaced: false };
        }
        let other = match self {
            Self::Windows1252 => return Decoded { text: bytes.iter().map(|b| cp1252(*b)).collect(), fallback: true, replaced: false },
            Self::Macintosh => encoding_rs::MACINTOSH,
            Self::Korean => encoding_rs::EUC_KR,
            Self::ShiftJis => encoding_rs::SHIFT_JIS,
            Self::Gbk => encoding_rs::GBK,
            Self::Big5 => encoding_rs::BIG5,
        };
        let (text, replaced) = other.decode_without_bom_handling(bytes);
        Decoded { text: text.into_owned(), fallback: true, replaced }
    }
}

/// A name's hash, for counting the names past the notes.
fn hash(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

/// Bytes read as text by [`NameEncoding::decode`].
struct Decoded {
    text: String,
    /// The bytes weren't UTF-8.
    fallback: bool,
    /// Some weren't valid in the encoding either, and became U+FFFD.
    replaced: bool,
}

/// Most names that weren't UTF-8 each named in a note; the rest are counted in one.
const MAX_NAME_NOTES: usize = 3;
/// Most other names counted (the count stops there).
const MAX_NAMES_COUNTED: usize = 10_000;

/// The names read in a [`NameEncoding`], and the notes on those that weren't UTF-8: one for each
/// name (the first [`MAX_NAME_NOTES`]), then one counting the others (up to
/// [`MAX_NAMES_COUNTED`]).
#[derive(Debug, Default)]
pub(super) struct Names {
    pub encoding: NameEncoding,
    noted: Vec<(String, bool)>,
    /// Hashes of the other names.
    more: std::collections::HashSet<u64>,
}

impl Names {
    pub fn new(encoding: NameEncoding) -> Self {
        Self { encoding, ..Self::default() }
    }

    /// A string's or name's text as a name: a string that isn't UTF-8 read in the encoding,
    /// with a note.
    pub fn text(&mut self, v: &V) -> Option<String> {
        match v {
            V::Str(s) => Some(self.bytes(s)),
            V::Name(n) => Some(n.clone()),
            _ => None,
        }
    }

    /// `bytes` read as a name (see [`Names::text`]).
    pub fn bytes(&mut self, bytes: &[u8]) -> String {
        let d = self.encoding.decode(bytes);
        if d.fallback {
            self.note(&d.text, d.replaced);
        }
        d.text
    }

    /// The first text value of `o`'s entry `key`, as a name.
    pub fn entry(&mut self, o: &Obj, key: &str) -> Option<String> {
        let v = o.get(key)?.iter().find(|v| matches!(v, V::Str(_)))?;
        self.text(v)
    }

    fn note(&mut self, name: &str, replaced: bool) {
        if self.noted.iter().any(|(n, _)| n == name) {
            return;
        }
        if self.noted.len() < MAX_NAME_NOTES {
            self.noted.push((name.to_string(), replaced));
        } else {
            self.count(hash(name));
        }
    }

    fn count(&mut self, h: u64) {
        if self.more.len() < MAX_NAMES_COUNTED {
            self.more.insert(h);
        }
    }

    /// Take the notes of `other`, a reader's for part of the same data.
    pub fn absorb(&mut self, other: Names) {
        for (name, replaced) in other.noted {
            self.note(&name, replaced);
        }
        for h in other.more {
            self.count(h);
        }
    }

    /// The notes on the names that weren't UTF-8.
    pub fn notes(&self) -> Vec<String> {
        self.notes_of(self.noted.iter().collect(), self.more.len())
    }

    /// The notes on the names in `used` that weren't UTF-8 (names read but then not used, such as
    /// the layers of data whose layers aren't used, left out).
    pub fn notes_among(&self, used: &[&str]) -> Vec<String> {
        let noted: Vec<&(String, bool)> = self.noted.iter().filter(|(n, _)| used.contains(&n.as_str())).collect();
        let others: std::collections::HashSet<u64> =
            used.iter().filter(|n| !noted.iter().any(|(m, _)| m == *n)).map(|n| hash(n)).filter(|h| self.more.contains(h)).collect();
        self.notes_of(noted, others.len())
    }

    fn notes_of(&self, noted: Vec<&(String, bool)>, more: usize) -> Vec<String> {
        let label = self.encoding.label();
        let mut out: Vec<String> = noted
            .into_iter()
            .map(|(name, replaced)| {
                if *replaced {
                    format!(
                        "the name `{name}` isn't UTF-8, and parts of it aren't valid {label} either: open the file with another legacyNameEncoding"
                    )
                } else {
                    format!("the name `{name}` isn't UTF-8 and was read as {label}: if it looks wrong, open the file with another legacyNameEncoding")
                }
            })
            .collect();
        if more > 0 {
            out.push(format!("more names that aren't UTF-8 were read as {label} ({count})", count = more));
        }
        out
    }
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or('?'),
        _ => char::from(b),
    }
}

/// A dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Obj {
    pub ty: String,
    /// Entries in order: the key (`None` in an array) and the values before it.
    pub entries: Vec<(Option<String>, Vec<V>)>,
    /// Values closed without a key.
    pub content: Vec<V>,
    /// Vector art enclosed by `X=` … `X+` in an opacity-mask dictionary.
    pub art: Option<std::sync::Arc<vectorcraft_doc::Node>>,
    /// What that art has that isn't read, reported once the mask is known to be enabled.
    pub unread: Vec<String>,
}

impl Obj {
    /// The values of entry `key` (the first one).
    pub fn get(&self, key: &str) -> Option<&[V]> {
        self.entries.iter().find(|(k, _)| k.as_deref() == Some(key)).map(|(_, v)| v.as_slice())
    }
    /// The numbers of entry `key`.
    pub fn nums(&self, key: &str) -> Vec<f64> {
        self.get(key).unwrap_or_default().iter().filter_map(V::num).collect()
    }
    /// The first text value of entry `key`.
    pub fn text(&self, key: &str) -> Option<String> {
        self.get(key)?.iter().find(|v| matches!(v, V::Str(_))).and_then(V::text)
    }
    /// The dictionary value of entry `key`.
    pub fn obj(&self, key: &str) -> Option<&Obj> {
        self.get(key)?.iter().find_map(V::obj)
    }
    /// The dictionaries an array holds, in order.
    pub fn items(&self) -> impl Iterator<Item = &Obj> {
        self.entries.iter().flat_map(|(_, v)| v.iter()).chain(self.content.iter()).filter_map(V::obj)
    }
    /// The first text of the content.
    pub fn content_text(&self) -> Option<String> {
        self.content.iter().find_map(V::text)
    }
}

/// Add the values on `stack` above `base` to `o` as one entry (`,`): the last is the key, except
/// in an array.
pub(super) fn add_entry(o: &mut Obj, stack: &mut Vec<V>, base: usize) {
    let mut vals: Vec<V> = stack.drain(base.min(stack.len())..).collect();
    if o.ty == "Array" {
        o.entries.push((None, vals));
        return;
    }
    let key = match vals.last() {
        Some(V::Str(_) | V::Name(_)) => vals.pop().and_then(|k| k.text()),
        _ => None,
    };
    o.entries.push((key, vals));
}

/// Close `o` (`;`): a last entry ending with a name is an entry, other values are its content.
pub(super) fn close(o: &mut Obj, stack: &mut Vec<V>, base: usize) {
    if stack.len() <= base {
        return;
    }
    if o.ty != "Array" && matches!(stack.last(), Some(V::Name(_))) {
        add_entry(o, stack, base);
    } else {
        let vals: Vec<V> = stack.drain(base.min(stack.len())..).collect();
        o.content.extend(vals);
    }
}

/// An object name from its XML id (`AI10_ArtUID`).
pub(super) fn xml_name(id: &str) -> String {
    vectorcraft_doc::xml_id::to_object_name(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_xml_ids() {
        assert_eq!(xml_name("Layer_1"), "Layer 1");
        assert_eq!(xml_name("My_Rect__x23_2"), "My Rect #2");
        assert_eq!(xml_name("_x31_abc__x28_paren_x29_"), "1abc (paren)");
        assert_eq!(xml_name("Ünï_日本"), "Ünï 日本");
        assert_eq!(xml_name("S_00000092429200007574234970000005974767392385616558_"), "S");
        assert_eq!(xml_name("a_x_"), "a x ");
        assert_eq!(xml_name("_xD800_"), " xD800 ");
    }

    #[test]
    fn entries_and_content() {
        let mut o = Obj { ty: "XMLUID".into(), ..Obj::default() };
        let mut st = vec![V::Num(9.0), V::Str(b"Main".to_vec())];
        close(&mut o, &mut st, 1);
        assert_eq!(o.content_text().as_deref(), Some("Main"));
        assert_eq!(st, vec![V::Num(9.0)]);
        let mut d = Obj { ty: "Dictionary".into(), ..Obj::default() };
        let mut st = vec![V::Num(1.0), V::Num(2.0), V::Name("RealPoint".into()), V::Str(b"P".to_vec())];
        add_entry(&mut d, &mut st, 0);
        let mut st = vec![V::Name("Def".into())];
        close(&mut d, &mut st, 0);
        assert_eq!(d.nums("P"), vec![1.0, 2.0]);
        assert_eq!(d.get("Def"), Some(&[][..]));
        let mut a = Obj { ty: "Array".into(), ..Obj::default() };
        let mut st = vec![V::Obj(Rc::new(d.clone()))];
        add_entry(&mut a, &mut st, 0);
        assert_eq!(a.items().count(), 1);
    }

    #[test]
    fn latin_text() {
        assert_eq!(text(b"caf\xe9 \x80"), "café €");
        assert_eq!(text("ü".as_bytes()), "ü");
    }

    /// The default reads every byte as Windows-1252 does, the five bytes it leaves undefined as the
    /// C1 controls of Latin-1, as `text` always has.
    #[test]
    fn names_default_to_windows_1252_with_latin_1_in_its_gaps() {
        let all: Vec<u8> = (0x80..=0xff).collect();
        let mut names = Names::default();
        assert_eq!(names.bytes(&all), text(&all));
        let gaps: String = [0x81u8, 0x8d, 0x8f, 0x90, 0x9d].iter().map(|b| names.bytes(&[*b])).collect();
        assert_eq!(gaps, "\u{81}\u{8d}\u{8f}\u{90}\u{9d}");
        assert_eq!(names.bytes(b"\x80\x9f"), "€Ÿ");
    }

    /// A name that is UTF-8 is read as it is in every encoding, without a note.
    #[test]
    fn utf8_names_need_no_encoding() {
        for label in NameEncoding::LABELS {
            let mut names = Names::new(NameEncoding::from_label(label).unwrap());
            assert_eq!(names.bytes("레이어 1".as_bytes()), "레이어 1");
            assert_eq!(names.text(&V::Str(b"Layer 1".to_vec())).as_deref(), Some("Layer 1"));
            assert!(names.notes().is_empty(), "{label}");
        }
    }

    /// Each encoding reads the bytes of a name written in it.
    #[test]
    fn names_in_each_encoding() {
        for (label, bytes, want) in [
            ("windows-1252", &b"Caf\xe9"[..], "Café"),
            ("macintosh", b"Caf\x8e", "Café"),
            ("windows-949", b"\xb7\xb9\xc0\xcc\xbe\xee 1", "레이어 1"),
            ("euc-kr", b"\xb7\xb9\xc0\xcc\xbe\xee 1", "레이어 1"),
            ("shift_jis", b"\x83\x8c\x83\x43\x83\x84\x81\x5b 1", "レイヤー 1"),
            ("gbk", b"\xcd\xbc\xb2\xe3 1", "图层 1"),
            ("big5", b"\xb9\xcf\xbch 1", "圖層 1"),
        ] {
            let mut names = Names::new(NameEncoding::from_label(label).unwrap());
            assert_eq!(names.bytes(bytes), want, "{label}");
            assert_eq!(
                names.notes(),
                vec![format!(
                    "the name `{want}` isn't UTF-8 and was read as {}: if it looks wrong, open the file with another legacyNameEncoding",
                    names.encoding.label()
                )]
            );
        }
        assert_eq!(NameEncoding::from_label(" Windows-949 "), Some(NameEncoding::Korean));
        assert_eq!(NameEncoding::from_label("utf-8"), None);
        for label in NameEncoding::LABELS {
            assert_eq!(NameEncoding::from_label(label).map(NameEncoding::label), Some(label));
        }
    }

    /// The Korean bytes read by default come in as Windows-1252, with a note naming what they
    /// became.
    #[test]
    fn korean_bytes_by_default_come_in_as_windows_1252_with_a_note() {
        let mut names = Names::default();
        let name = names.bytes(b"\xb7\xb9\xc0\xcc\xbe\xee 1");
        assert_eq!(name, "·¹ÀÌ¾î 1");
        assert_eq!(
            names.notes(),
            vec!["the name `·¹ÀÌ¾î 1` isn't UTF-8 and was read as windows-1252: if it looks wrong, open the file with another legacyNameEncoding"]
        );
    }

    /// Bytes the encoding doesn't define come in as U+FFFD, and the note says so.
    #[test]
    fn malformed_names_keep_a_replacement_character() {
        let mut names = Names::new(NameEncoding::Korean);
        let name = names.bytes(b"\xb7\xb9\xc0 1");
        assert!(name.contains('\u{fffd}'), "{name:?}");
        assert!(name.starts_with('레'), "{name:?}");
        let notes = names.notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("isn't UTF-8, and parts of it aren't valid windows-949 either:"), "{notes:?}");
    }

    /// The notes name the first names once each, and count the others.
    #[test]
    fn name_notes_are_capped() {
        let mut names = Names::new(NameEncoding::Macintosh);
        for i in 0..10u8 {
            let bytes = [b'a' + i, 0x8e];
            names.bytes(&bytes);
            names.bytes(&bytes);
        }
        let notes = names.notes();
        assert_eq!(notes.len(), MAX_NAME_NOTES + 1);
        assert!(notes[0].contains("`aé`"), "{notes:?}");
        assert_eq!(notes.last().map(String::as_str), Some("more names that aren't UTF-8 were read as macintosh (7)"));
        let mut all = Names::new(NameEncoding::Macintosh);
        all.absorb(names);
        assert_eq!(all.notes(), notes);
        // The count stops at its limit, whatever the file holds.
        let mut many = Names::new(NameEncoding::Macintosh);
        for i in 0..MAX_NAME_NOTES + MAX_NAMES_COUNTED + 50 {
            let mut bytes = i.to_string().into_bytes();
            bytes.push(0x8e);
            many.bytes(&bytes);
        }
        assert_eq!(many.notes().last().map(String::as_str), Some("more names that aren't UTF-8 were read as macintosh (10000)"));
    }
}
