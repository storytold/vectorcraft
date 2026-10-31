//! Writing a file as a stream: the JSON is serialized straight into the output (compressed or
//! not) and the data after it is copied in from the document's own buffers, so a save holds no
//! copy of the file in memory besides what the caller's writer keeps.

use std::collections::HashSet;
use std::io::{self, Write};

use flate2::{Compress, Compression, Crc, FlushCompress, Status};

/// The output of a save: plain, or gzip ([`Gz`]).
pub(crate) enum Out<W: Write> {
    Plain(W),
    Gz(Box<Gz<W>>),
}

impl<W: Write> Out<W> {
    /// What is written next is compressed already (JPEG, PNG…) or not ([`Gz::packed`]).
    pub(crate) fn packed(&mut self, packed: bool) -> io::Result<()> {
        match self {
            Out::Plain(_) => Ok(()),
            Out::Gz(gz) => gz.packed(packed),
        }
    }

    /// Bytes written so far (before compression).
    pub(crate) fn written(&self) -> Option<u64> {
        match self {
            Out::Plain(_) => None,
            Out::Gz(gz) => Some(gz.total),
        }
    }

    /// Everything written: the gzip trailer goes out, and the writer is flushed.
    pub(crate) fn finish(self) -> io::Result<()> {
        match self {
            Out::Plain(mut w) => w.flush(),
            Out::Gz(gz) => gz.finish(),
        }
    }
}

impl<W: Write> Write for Out<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Out::Plain(w) => w.write(buf),
            Out::Gz(gz) => gz.write(buf),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        match self {
            Out::Plain(w) => w.write_all(buf),
            Out::Gz(gz) => gz.write_all(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Out::Plain(w) => w.flush(),
            Out::Gz(gz) => gz.flush(),
        }
    }
}

/// Bytes of compressed output gathered before they go to the writer.
const OUT_CHUNK: usize = 64 << 10;

/// A gzip stream, written as [`flate2::write::GzEncoder`] at the default level writes it (the
/// same header and the same data; the deflate blocks are the same too where flate2 deflates with
/// miniz_oxide, not with zlib-rs, which writes them by how the data is fed), except that data which
/// is compressed already ([`Gz::packed`]) is stored as it is: the default level spends a second
/// or more per photo on it for next to nothing, and makes it slower to unpack. Where the level
/// changes the deflate blocks so far end with a sync flush and a fresh compressor writes on,
/// which makes one valid deflate stream that every reader unpacks.
pub(crate) struct Gz<W: Write> {
    out: W,
    deflate: Compress,
    /// What `deflate` writes: stored blocks (packed data), else the default level.
    stored: bool,
    crc: Crc,
    buf: Vec<u8>,
    /// Bytes written (unpacked).
    total: u64,
}

impl<W: Write> Gz<W> {
    pub(crate) fn new(mut out: W) -> io::Result<Self> {
        // flate2's header for the default level: no name, no time, unknown OS.
        out.write_all(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255])?;
        Ok(Self {
            out,
            deflate: Compress::new(Compression::default(), false),
            stored: false,
            crc: Crc::new(),
            buf: Vec::with_capacity(OUT_CHUNK),
            total: 0,
        })
    }

    /// Run the compressor over `input` with `flush` until it has taken it all (and, flushing,
    /// written everything out).
    fn deflate(&mut self, mut input: &[u8], flush: FlushCompress) -> io::Result<()> {
        loop {
            if self.buf.len() == self.buf.capacity() {
                self.out.write_all(&self.buf)?;
                self.buf.clear();
            }
            let (before_in, before_out) = (self.deflate.total_in(), self.deflate.total_out());
            let status = self.deflate.compress_vec(input, &mut self.buf, flush).map_err(io::Error::other)?;
            let taken = usize::try_from(self.deflate.total_in() - before_in).unwrap_or(input.len()).min(input.len());
            input = input.get(taken..).unwrap_or_default();
            let made = self.deflate.total_out() - before_out;
            let full = self.buf.len() == self.buf.capacity();
            match flush {
                FlushCompress::None if input.is_empty() => return Ok(()),
                FlushCompress::Finish if status == Status::StreamEnd => return Ok(()),
                // A flush is done once it left room in the output.
                FlushCompress::Sync if input.is_empty() && !full => return Ok(()),
                _ => {}
            }
            if taken == 0 && made == 0 && !full {
                return Err(io::Error::other("the compressor made no progress"));
            }
        }
    }

    /// What is written next is compressed already (stored as it is), or not (compressed at the
    /// default level).
    fn packed(&mut self, packed: bool) -> io::Result<()> {
        if packed == self.stored {
            return Ok(());
        }
        self.deflate(&[], FlushCompress::Sync)?;
        self.deflate = Compress::new(if packed { Compression::none() } else { Compression::default() }, false);
        self.stored = packed;
        Ok(())
    }

    fn finish(mut self) -> io::Result<()> {
        self.deflate(&[], FlushCompress::Finish)?;
        self.out.write_all(&self.buf)?;
        self.out.write_all(&self.crc.sum().to_le_bytes())?;
        self.out.write_all(&self.crc.amount().to_le_bytes())?;
        self.out.flush()
    }
}

impl<W: Write> Write for Gz<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.crc.update(buf);
        self.total += buf.len() as u64;
        self.deflate(buf, FlushCompress::None)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Flushing would end a deflate block: the stream is flushed when it is finished.
        Ok(())
    }
}

/// Indents JSON text written through it one level (two spaces after each newline), as a member
/// of the file's top-level object; newlines only occur between tokens, inside strings they are
/// escaped.
pub(crate) struct Indent<W: Write> {
    pub(crate) inner: W,
    pub(crate) on: bool,
}

impl<W: Write> Write for Indent<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if !self.on {
            return self.inner.write_all(buf);
        }
        let mut lines = buf.split(|&b| b == b'\n');
        self.inner.write_all(lines.next().unwrap_or_default())?;
        for line in lines {
            self.inner.write_all(b"\n  ")?;
            self.inner.write_all(line)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The member that names an image.
const TAG: &[u8] = b"\"key\":";

/// Passes JSON text through, collecting the string values of every `"key": "…"` member at any
/// depth (what the images a document uses are named by). Quotes inside JSON strings are escaped,
/// so every match is a real member.
pub(crate) struct KeyScan<W: Write> {
    pub(crate) inner: W,
    pub(crate) keys: HashSet<String>,
    /// Looking for keys (else it only passes the text on).
    pub(crate) active: bool,
    /// How much of [`TAG`] the last bytes matched.
    matched: usize,
    /// Right after a tag: skipping whitespace to its value.
    after_tag: bool,
    /// The value string so far, quotes included.
    value: Option<Vec<u8>>,
    escaped: bool,
}

impl<W: Write> KeyScan<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self { inner, keys: HashSet::new(), active: true, matched: 0, after_tag: false, value: None, escaped: false }
    }

    fn scan(&mut self, buf: &[u8]) {
        if !self.active {
            return;
        }
        let mut i = 0;
        while let Some(&b) = buf.get(i) {
            // Nothing in progress: skip to the next quote.
            if self.matched == 0 && !self.after_tag && self.value.is_none() {
                match buf.get(i..).and_then(|r| r.iter().position(|&c| c == b'"')) {
                    Some(at) => i += at,
                    None => return,
                }
                self.matched = 1;
                i += 1;
                continue;
            }
            self.step(b);
            i += 1;
        }
    }

    fn step(&mut self, b: u8) {
        if let Some(v) = self.value.as_mut() {
            v.push(b);
            if self.escaped {
                self.escaped = false;
            } else if b == b'\\' {
                self.escaped = true;
            } else if b == b'"'
                && let Some(v) = self.value.take()
                && let Ok(s) = serde_json::from_slice::<String>(&v)
            {
                self.keys.insert(s);
            }
        } else if self.after_tag && !b.is_ascii_whitespace() {
            self.after_tag = false;
            if b == b'"' {
                self.value = Some(vec![b]);
            }
        }
        if TAG.get(self.matched) == Some(&b) {
            self.matched += 1;
            if self.matched == TAG.len() {
                self.matched = 0;
                self.after_tag = true;
                self.value = None;
                self.escaped = false;
            }
        } else {
            self.matched = usize::from(b == b'"');
        }
    }
}

impl<W: Write> Write for KeyScan<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.scan(buf);
        self.inner.write_all(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// How the keys were found before saves were streamed: in the whole JSON text at once.
    fn string_members(json: &[u8]) -> HashSet<String> {
        let mut out = HashSet::new();
        let mut rest = json;
        while let Some(at) = rest.windows(TAG.len()).position(|w| w == TAG) {
            rest = rest.get(at + TAG.len()..).unwrap_or_default();
            let value = rest.trim_ascii_start();
            if value.first() == Some(&b'"')
                && let Some(Ok(s)) = serde_json::Deserializer::from_slice(value).into_iter::<String>().next()
            {
                out.insert(s);
            }
        }
        out
    }

    /// The keys `KeyScan` finds in `json` written in pieces of `chunk` bytes.
    fn scanned(json: &[u8], chunk: usize) -> HashSet<String> {
        let mut scan = KeyScan::new(Vec::new());
        for piece in json.chunks(chunk.max(1)) {
            scan.write_all(piece).unwrap();
        }
        assert_eq!(scan.inner, json, "the text passes through");
        scan.keys
    }

    #[test]
    fn finds_the_keys_the_whole_text_search_found() {
        let v = serde_json::json!({"key": "a", "x": [{"key": "b \"q\" \\ é"}, {"monkey": "no"}, {"k": "key"}], "s": "\"key\": \"no\"", "key2": "n", "o": {"key": 3}});
        for json in [serde_json::to_vec(&v).unwrap(), serde_json::to_vec_pretty(&v).unwrap()] {
            let want = string_members(&json);
            assert_eq!(want, HashSet::from(["a".to_string(), "b \"q\" \\ é".to_string()]));
            for chunk in [1, 2, 3, 7, 64, json.len()] {
                assert_eq!(scanned(&json, chunk), want, "chunk {chunk}");
            }
        }
    }

    fn arb_json() -> impl Strategy<Value = serde_json::Value> {
        let key = prop::sample::select(vec!["key", "k", "kkey", "key ", "x"]).prop_map(str::to_string);
        let leaf = prop_oneof![
            Just(serde_json::Value::Null),
            any::<i32>().prop_map(serde_json::Value::from),
            "[a-z\"\\\\: ké\n]{0,8}".prop_map(serde_json::Value::from),
        ];
        leaf.prop_recursive(4, 32, 6, move |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..5).prop_map(serde_json::Value::from),
                prop::collection::vec((key.clone(), inner), 0..5).prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn scanning_in_pieces_finds_what_the_whole_text_search_found(v in arb_json(), chunk in 1usize..40, pretty in any::<bool>()) {
            let json = if pretty { serde_json::to_vec_pretty(&v) } else { serde_json::to_vec(&v) }.unwrap();
            prop_assert_eq!(scanned(&json, chunk), string_members(&json));
        }
    }
}
