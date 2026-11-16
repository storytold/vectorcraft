//! Writing a file as a stream: the JSON is serialized straight into the output (compressed or
//! not) and the data after it is copied in from the document's own buffers, so a save holds no
//! copy of the file in memory besides what the caller's writer keeps.

use std::io::{self, Write};

use craft_gzip::Gz;

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
            Out::Gz(gz) => Some(gz.written()),
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
