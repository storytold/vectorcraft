//! Data the program reads: from itself (`currentfile`) through decoding filters, and images
//! sampled from it, from strings or from procedures.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::cmyk::{Inks, jpeg_components};
use vectorcraft_doc::{ImageBlob, ImageObject, Node, NodeId, NodeKind, OpacityMask};
use vectorcraft_geom::Affine;

use super::graphics::{Space, process};
use super::interp::{Interp, matrix_of};
use super::lex::{Lexer, find, hex_decode};
use super::obj::{DictRef, Key, Obj, Op, PsError, Res, Shared, ps_err};
use crate::ps;

/// Most bytes one filter or image reads.
const MAX_DATA: usize = 1 << 28;
/// Most pixels an image may have.
const MAX_PIXELS: usize = 1 << 26;
/// Why an image is missing.
const TOO_LARGE: &str = "images too large to read were left out";
/// Most distinct colours an image converts through a tint transform.
const MAX_TINTS: usize = 1 << 16;

/// A file object: the program itself, or data a filter decoded.
#[derive(Debug)]
pub(crate) enum Stream {
    Current,
    /// Filters over the program's own data, decoded when first read.
    Pending(Vec<Filter>),
    Buf {
        data: Vec<u8>,
        pos: usize,
        /// The samples are 8-bit gray, RGB or CMYK (1, 3 or 4 channels) whatever the image's
        /// color space: a decoded JPEG.
        rgb: Option<u8>,
    },
}

fn io<T>(at: &str) -> Res<T> {
    ps_err("ioerror", at)
}

/// The bytes `RunLengthDecode` makes of `raw`, and how many it read.
fn run_length(raw: &[u8]) -> Res<(Vec<u8>, usize)> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(&n) = raw.get(i) {
        i += 1;
        match n {
            128 => break,
            0..=127 => {
                let len = usize::from(n) + 1;
                out.extend_from_slice(raw.get(i..i + len).ok_or(PsError::Ps("ioerror", "RunLengthDecode".into()))?);
                i += len;
            }
            _ => {
                let b = *raw.get(i).ok_or(PsError::Ps("ioerror", "RunLengthDecode".into()))?;
                i += 1;
                out.extend(std::iter::repeat_n(b, 257 - usize::from(n)));
            }
        }
        if out.len() > MAX_DATA {
            return Err(PsError::Limit("decoded data is too large"));
        }
    }
    Ok((out, i))
}

/// zlib data inflated, and how many bytes of `raw` it took.
fn flate(raw: &[u8]) -> Res<(Vec<u8>, usize)> {
    let mut z = flate2::Decompress::new(true);
    let mut out = Vec::with_capacity(raw.len().saturating_mul(3).min(MAX_DATA));
    loop {
        let before = (z.total_in(), z.total_out());
        if out.capacity() - out.len() < 64 * 1024 {
            out.reserve(256 * 1024);
        }
        let input = raw.get(z.total_in() as usize..).unwrap_or_default();
        let status = z.decompress_vec(input, &mut out, flate2::FlushDecompress::None).map_err(|_| PsError::Ps("ioerror", "FlateDecode".into()))?;
        if out.len() > MAX_DATA {
            return Err(PsError::Limit("decoded data is too large"));
        }
        match status {
            flate2::Status::StreamEnd => break,
            _ if (z.total_in(), z.total_out()) == before => break,
            _ => {}
        }
    }
    Ok((out, z.total_in() as usize))
}

/// LZW data (`EarlyChange` 1) decoded, and how many bytes of `raw` it took.
fn lzw(raw: &[u8]) -> Res<(Vec<u8>, usize)> {
    let mut table: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    table.push(vec![]);
    table.push(vec![]);
    let (mut out, mut prev): (Vec<u8>, Option<Vec<u8>>) = (vec![], None);
    let (mut acc, mut bits, mut width, mut i) = (0u32, 0u32, 9u32, 0usize);
    loop {
        while bits < width {
            let Some(&b) = raw.get(i) else { return Ok((out, i)) };
            i += 1;
            acc = (acc << 8) | u32::from(b);
            bits += 8;
        }
        let code = ((acc >> (bits - width)) & ((1 << width) - 1)) as usize;
        bits -= width;
        match code {
            256 => {
                table.truncate(258);
                width = 9;
                prev = None;
            }
            257 => return Ok((out, i)),
            _ => {
                let entry = match (table.get(code), &prev) {
                    (Some(e), _) if !e.is_empty() => e.clone(),
                    (_, Some(p)) if code == table.len() => {
                        let mut e = p.clone();
                        e.extend(p.first());
                        e
                    }
                    _ => return io("LZWDecode"),
                };
                out.extend_from_slice(&entry);
                if let Some(mut p) = prev.take()
                    && table.len() < 4096
                {
                    p.extend(entry.first());
                    table.push(p);
                }
                width = match table.len() + 1 {
                    n if n >= 2048 => 12,
                    n if n >= 1024 => 11,
                    n if n >= 512 => 10,
                    _ => 9,
                };
                prev = Some(entry);
                if out.len() > MAX_DATA {
                    return Err(PsError::Limit("decoded data is too large"));
                }
            }
        }
    }
}

/// A JPEG's pixels (gray, RGB or CMYK, 8 bits) with its channel count, and how many bytes of
/// `raw` it took. A CMYK JPEG gives its samples as stored, for the image's Decode array to read
/// (Adobe's CMYK JPEGs store them inverted, which a Decode array of `[1 0 1 0 1 0 1 0]` undoes).
fn dct(raw: &[u8]) -> Res<(Vec<u8>, usize, u8)> {
    let end = find(raw, &[0xFF, 0xD9]).map_or(raw.len(), |e| e + 2);
    let jpeg = raw.get(..end).unwrap_or_default();
    // `cmyk` gives the ink amounts of inverted samples: inverted back, they are the samples.
    if jpeg_components(jpeg) == Some(4)
        && let Some(inks) = ImageBlob::new("image/jpeg", jpeg.to_vec()).cmyk()
    {
        return Ok((inks.data.into_iter().map(|v| 255 - v).collect(), end, 4));
    }
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).map_err(|_| PsError::Ps("ioerror", "DCTDecode".into()))?;
    if img.color().channel_count() == 1 { Ok((img.to_luma8().into_raw(), end, 1)) } else { Ok((img.to_rgb8().into_raw(), end, 3)) }
}

/// A decoding filter and its parameters.
#[derive(Clone, Debug)]
pub(crate) struct Filter {
    name: Rc<str>,
    /// `SubFileDecode`'s count and end-of-data string.
    sub: (usize, Vec<u8>),
    /// `FlateDecode`'s and `LZWDecode`'s `Predictor`: what it undoes.
    predictor: Option<Predictor>,
}

/// A `Predictor` (2: TIFF, 10–15: PNG) with its `Colors`, `BitsPerComponent` and `Columns`.
#[derive(Clone, Copy, Debug)]
struct Predictor {
    kind: u8,
    colors: usize,
    bpc: usize,
    columns: usize,
}

impl Predictor {
    /// The predictor of a filter's parameters, if it has one.
    fn of(d: &DictRef) -> Res<Option<Self>> {
        let num = |k: &str, default: f64| d.borrow().get(&Key::name(k)).and_then(Obj::as_num).unwrap_or(default);
        let kind = num("Predictor", 1.0);
        if kind <= 1.0 {
            return Ok(None);
        }
        let (colors, bpc, columns) = (num("Colors", 1.0), num("BitsPerComponent", 8.0), num("Columns", 1.0));
        if !(kind == 2.0 || (10.0..=15.0).contains(&kind))
            || !(1.0..=32.0).contains(&colors)
            || ![1.0, 2.0, 4.0, 8.0, 16.0].contains(&bpc)
            || !(1.0..=1e6).contains(&columns)
        {
            return ps_err("rangecheck", "Predictor");
        }
        Ok(Some(Self { kind: kind as u8, colors: colors as usize, bpc: bpc as usize, columns: columns as usize }))
    }

    /// The data the predictor made `data` from.
    fn undo(self, data: &[u8]) -> Vec<u8> {
        // A row longer than the data is the data.
        let row = (self.colors * self.bpc * self.columns).div_ceil(8).min(data.len().max(1));
        // Bytes per pixel (at least one) is how far back the left neighbour is.
        let bpp = (self.colors * self.bpc).div_ceil(8).max(1);
        if self.kind == 2 {
            let mut out = data.to_vec();
            // Horizontal differencing, for 8-bit components (the common case).
            if self.bpc == 8 {
                for line in out.chunks_mut(row.max(1)) {
                    for i in bpp..line.len() {
                        let left = line.get(i - bpp).copied().unwrap_or(0);
                        if let Some(b) = line.get_mut(i) {
                            *b = b.wrapping_add(left);
                        }
                    }
                }
            }
            return out;
        }
        // PNG: each row starts with its filter type.
        let mut out: Vec<u8> = Vec::with_capacity(data.len());
        let mut prev = vec![0u8; row];
        for chunk in data.chunks(row + 1) {
            let Some((&kind, line)) = chunk.split_first() else { break };
            let mut cur = line.to_vec();
            cur.resize(row, 0);
            for i in 0..row {
                let a = if i >= bpp { cur.get(i - bpp).copied().unwrap_or(0) } else { 0 };
                let b = prev.get(i).copied().unwrap_or(0);
                let c = if i >= bpp { prev.get(i - bpp).copied().unwrap_or(0) } else { 0 };
                let add = match kind {
                    1 => a,
                    2 => b,
                    3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                    4 => paeth(a, b, c),
                    _ => 0,
                };
                if let Some(v) = cur.get_mut(i) {
                    *v = v.wrapping_add(add);
                }
            }
            out.extend_from_slice(cur.get(..line.len().min(row)).unwrap_or_default());
            prev = cur;
        }
        out
    }
}

/// The PNG Paeth predictor.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i16::from(a), i16::from(b), i16::from(c));
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// The bytes `f` decodes from `raw`, how many it read, and the channels of a decoded JPEG.
fn decode(f: &Filter, raw: &[u8]) -> Res<(Vec<u8>, usize, Option<u8>)> {
    let (data, used) = match &*f.name {
        "ASCIIHexDecode" => {
            let end = raw.iter().position(|b| *b == b'>');
            let body = raw.get(..end.unwrap_or(raw.len())).unwrap_or_default();
            (hex_decode(body).ok_or(PsError::Ps("ioerror", "ASCIIHexDecode".into()))?, end.map_or(raw.len(), |e| e + 1))
        }
        "ASCII85Decode" => {
            let end = find(raw, b"~>");
            let mut text = String::from_utf8_lossy(raw.get(..end.unwrap_or(raw.len())).unwrap_or_default()).into_owned();
            text.push_str("~>");
            (ps::ascii85_decode(&text).ok_or(PsError::Ps("ioerror", "ASCII85Decode".into()))?, end.map_or(raw.len(), |e| e + 2))
        }
        "RunLengthDecode" => run_length(raw)?,
        "FlateDecode" | "LZWDecode" => {
            let (data, used) = if &*f.name == "FlateDecode" { flate(raw)? } else { lzw(raw)? };
            match f.predictor {
                Some(p) => (p.undo(&data), used),
                None => (data, used),
            }
        }
        "DCTDecode" => {
            let (d, used, channels) = dct(raw)?;
            return Ok((d, used, Some(channels)));
        }
        "SubFileDecode" => {
            let (count, eod) = &f.sub;
            if eod.is_empty() {
                // A count of 0 without an end string sets no end: the data passes through whole.
                let n = if *count == 0 { raw.len() } else { (*count).min(raw.len()) };
                (raw.get(..n).unwrap_or_default().to_vec(), n)
            } else {
                let (mut at, mut seen) = (0, 0);
                loop {
                    match find(raw.get(at..).unwrap_or_default(), eod) {
                        Some(i) if seen < *count => {
                            seen += 1;
                            at += i + eod.len();
                        }
                        Some(i) => break (raw.get(..at + i).unwrap_or_default().to_vec(), at + i + eod.len()),
                        None => break (raw.to_vec(), raw.len()),
                    }
                }
            }
        }
        // Reads all of its source (up to the end of the filters below it) and keeps it.
        "NullEncode" | "ReusableStreamDecode" => (raw.to_vec(), raw.len()),
        other => return ps_err("undefined", other),
    };
    Ok((data, used, None))
}

/// What a source of image data gives.
#[derive(Clone)]
enum Source {
    File(Rc<RefCell<Stream>>),
    Str(Vec<u8>),
    Proc(Obj),
}

impl Interp<'_> {
    pub fn data_op(&mut self, op: Op) -> Res {
        use Op::*;
        match op {
            CurrentFile => self.push(Obj::File { stream: Rc::new(RefCell::new(Stream::Current)), exec: false })?,
            Filter => self.filter()?,
            ReadHexString | ReadString => {
                let s = self.pop_str()?;
                let Obj::File { stream: f, .. } = self.pop()? else { return ps_err("typecheck", "") };
                let n = s.len();
                let got = if op == ReadString { self.read(&f, n)? } else { self.read_hex(&f, n)? };
                self.fill(s, &got)?;
                self.push(Obj::Bool(got.len() == n))?;
            }
            ReadLine => {
                let s = self.pop_str()?;
                let Obj::File { stream: f, .. } = self.pop()? else { return ps_err("typecheck", "") };
                let n = s.len();
                let mut line = vec![];
                let mut ended = false;
                while line.len() < n {
                    match self.read(&f, 1)?.first() {
                        Some(b'\n') | None => {
                            ended = true;
                            break;
                        }
                        Some(b'\r') => {
                            ended = true;
                            break;
                        }
                        Some(b) => line.push(*b),
                    }
                }
                self.fill(s, &line)?;
                self.push(Obj::Bool(ended))?;
            }
            FlushFile => {
                // An input file is read to its end: a filter over the program's data skips it
                // (metadata read with `flushfile`). The program's own file goes on being run.
                if let Obj::File { stream, .. } = self.pop()? {
                    self.materialize(&stream)?;
                }
            }
            CloseFile => {
                self.pop()?;
            }
            File => {
                self.pop()?;
                let name = self.pop()?.text().unwrap_or_else(|| Rc::from(""));
                // The standard files take what programs print (and give nothing); there is no
                // file system to open others in.
                if !matches!(&*name, "%stdout" | "%stderr" | "%stdin" | "%lineedit" | "%statementedit") {
                    return ps_err("invalidfileaccess", "file");
                }
                self.push(Obj::File { stream: Rc::new(RefCell::new(Stream::Buf { data: vec![], pos: 0, rgb: None })), exec: false })?;
            }
            Status => match self.pop()? {
                Obj::File { .. } => self.push(Obj::Bool(true))?,
                // A file name: there are no files.
                Obj::Str(_) => self.push(Obj::Bool(false))?,
                _ => return ps_err("typecheck", ""),
            },
            BytesAvailable => {
                let Obj::File { stream: f, .. } = self.pop()? else { return ps_err("typecheck", "") };
                self.materialize(&f)?;
                let n = match &*f.borrow() {
                    Stream::Current => self.lex.rest().len(),
                    Stream::Buf { data, pos, .. } => data.len().saturating_sub(*pos),
                    Stream::Pending(_) => 0,
                };
                self.push(Obj::Int(n as i64))?;
            }
            ResetFile => {
                // A decoded (reusable) stream reads again from its start.
                if let Obj::File { stream: f, .. } = self.pop()?
                    && let Stream::Buf { pos, .. } = &mut *f.borrow_mut()
                {
                    *pos = 0;
                }
            }
            Write | WriteString | WriteHexString => {
                self.pop()?;
                self.pop()?;
            }
            Read => {
                let Obj::File { stream: f, .. } = self.pop()? else { return ps_err("typecheck", "") };
                match self.read(&f, 1)?.first() {
                    Some(b) => {
                        self.push(Obj::Int(i64::from(*b)))?;
                        self.push(Obj::Bool(true))?;
                    }
                    None => self.push(Obj::Bool(false))?,
                }
            }
            FilePosition => {
                let Obj::File { stream: f, .. } = self.pop()? else { return ps_err("typecheck", "") };
                let Stream::Buf { pos, .. } = &*f.borrow() else { return ps_err("ioerror", "") };
                self.push(Obj::Int(*pos as i64))?;
            }
            SetFilePosition => {
                let at = self.pop_count()?;
                let Obj::File { stream: f, .. } = self.pop()? else { return ps_err("typecheck", "") };
                let Stream::Buf { data, pos, .. } = &mut *f.borrow_mut() else { return ps_err("ioerror", "") };
                *pos = at.min(data.len());
            }
            // There are no files to delete, rename or run.
            DeleteFile | Run => {
                self.pop_str()?;
                return ps_err("undefinedfilename", "");
            }
            RenameFile => {
                self.pop_str()?;
                self.pop_str()?;
                return ps_err("undefinedfilename", "");
            }
            Token => self.token()?,
            Eexec => self.eexec()?,
            Image | ImageMask | ColorImage => self.image(op)?,
            _ => self.text_op(op)?,
        }
        Ok(())
    }

    /// `token`: the next object read from a string (and the rest of it) or a file.
    fn token(&mut self) -> Res {
        match self.pop()? {
            Obj::Str(s) => {
                let bytes = s.to_vec();
                let mut lex = Lexer::new(&bytes);
                let Some(o) = lex.next()? else { return self.push(Obj::Bool(false)) };
                let o = self.scanned(o, lex.immediate)?;
                // The whitespace after the token is read with it.
                let used = bytes.len() - lex.data().len();
                self.push_interval(Obj::Str(s), used, bytes.len() - used)?;
                self.push(o)?;
            }
            Obj::File { stream: f, .. } => {
                self.materialize(&f)?;
                let (o, immediate) = match &mut *f.borrow_mut() {
                    Stream::Current => (self.lex.next()?, self.lex.immediate),
                    Stream::Buf { data, pos, .. } => {
                        let rest = data.get(*pos..).unwrap_or_default();
                        let mut lex = Lexer::new(rest);
                        let o = lex.next()?;
                        *pos += rest.len() - lex.data().len();
                        (o, lex.immediate)
                    }
                    Stream::Pending(_) => (None, false),
                };
                let Some(o) = o else { return self.push(Obj::Bool(false)) };
                let o = self.scanned(o, immediate)?;
                self.push(o)?;
            }
            _ => return ps_err("typecheck", "token"),
        }
        self.push(Obj::Bool(true))
    }

    /// Up to `n` bytes of file `f`.
    fn read(&mut self, f: &Rc<RefCell<Stream>>, n: usize) -> Res<Vec<u8>> {
        self.materialize(f)?;
        Ok(match &mut *f.borrow_mut() {
            Stream::Current => {
                let rest = self.lex.data();
                let k = n.min(rest.len());
                let v = rest.get(..k).unwrap_or_default().to_vec();
                self.lex.advance(k);
                v
            }
            Stream::Buf { data, pos, .. } => {
                let start = (*pos).min(data.len());
                let end = start.saturating_add(n).min(data.len());
                *pos = end;
                data.get(start..end).unwrap_or_default().to_vec()
            }
            Stream::Pending(_) => vec![],
        })
    }

    /// Up to `n` bytes read as hexadecimal digits from `f` (other characters skipped).
    fn read_hex(&mut self, f: &Rc<RefCell<Stream>>, n: usize) -> Res<Vec<u8>> {
        let mut out = Vec::with_capacity(n);
        let mut high: Option<u8> = None;
        while out.len() < n {
            let Some(&b) = self.read(f, 1)?.first() else { break };
            let Some(v) = char::from(b).to_digit(16) else { continue };
            match high.take() {
                Some(h) => out.push(h << 4 | v as u8),
                None => high = Some(v as u8),
            }
        }
        Ok(out)
    }

    /// Read bytes `got` into string `s` and push the part of `s` they fill.
    fn fill(&mut self, s: Shared<u8>, got: &[u8]) -> Res {
        if !s.write(0, got) {
            return ps_err("rangecheck", "");
        }
        self.push_interval(Obj::Str(s), 0, got.len())
    }

    /// `filter`: a file that decodes its source. Over the program's own data it decodes when
    /// first read (the data follows the operator that reads it); over other data, at once.
    fn filter(&mut self) -> Res {
        let name = self.pop()?.text().ok_or(PsError::Ps("typecheck", "filter".into()))?;
        let sub = if &*name == "SubFileDecode" {
            let eod = self.pop_str()?.to_vec();
            let count = self.pop_count()?;
            (count, eod)
        } else {
            (0, vec![])
        };
        let mut predictor = None;
        if let Some(Obj::Dict(d)) = self.stack.last().cloned() {
            self.pop()?;
            predictor = Predictor::of(&d)?;
        }
        let spec_name = name.clone();
        let spec = Filter { name, sub, predictor };
        let stream = match self.pop()? {
            Obj::File { stream: f, .. } => {
                if let Stream::Buf { data, pos, .. } = &*f.borrow() {
                    self.alloc(data.len().saturating_sub(*pos))?;
                }
                let chain = match &*f.borrow() {
                    Stream::Current => Some(vec![]),
                    Stream::Pending(c) => Some(c.clone()),
                    Stream::Buf { .. } => None,
                };
                match chain {
                    Some(mut c) => {
                        // The new filter reads a pending one's data: it has none left of its own
                        // (`flushfile` on it skips nothing more).
                        if !c.is_empty() {
                            *f.borrow_mut() = Stream::Buf { data: vec![], pos: 0, rgb: None };
                        }
                        c.push(spec);
                        Stream::Pending(c)
                    }
                    None => {
                        let mut b = f.borrow_mut();
                        let Stream::Buf { data, pos, .. } = &mut *b else { return io("filter") };
                        let (out, used, rgb) = decode(&spec, data.get(*pos..).unwrap_or_default())?;
                        *pos = pos.saturating_add(used);
                        Stream::Buf { data: out, pos: 0, rgb }
                    }
                }
            }
            Obj::Str(s) => {
                self.alloc(s.borrow().len())?;
                let (data, _, rgb) = decode(&spec, &s.borrow())?;
                Stream::Buf { data, pos: 0, rgb }
            }
            p @ Obj::Array { .. } => {
                let raw = self.gather(&p, MAX_DATA)?;
                let (data, _, rgb) = decode(&spec, &raw)?;
                Stream::Buf { data, pos: 0, rgb }
            }
            _ => return ps_err("typecheck", "filter"),
        };
        let stream = Rc::new(RefCell::new(stream));
        // A reusable stream reads its source when it is made: the data follows the operator.
        if &*spec_name == "ReusableStreamDecode" {
            self.materialize(&stream)?;
        }
        self.push(Obj::File { stream, exec: false })
    }

    /// The data an executable file runs as a program: what is left of it. `None` for the
    /// program's own file, which is being run already.
    pub(super) fn program_of(&mut self, f: &Rc<RefCell<Stream>>) -> Res<Option<Vec<u8>>> {
        if matches!(&*f.borrow(), Stream::Current) {
            return Ok(None);
        }
        self.read(f, MAX_DATA).map(Some)
    }

    /// The decoded data left in file `f`, without reading it (a shading's or function's data).
    pub(super) fn peek_all(&mut self, f: &Rc<RefCell<Stream>>) -> Res<Vec<u8>> {
        self.materialize(f)?;
        Ok(match &*f.borrow() {
            Stream::Buf { data, pos, .. } => data.get(*pos..).unwrap_or_default().to_vec(),
            _ => vec![],
        })
    }

    /// Decode the filters a file waits to run over the program's data, from where it is now.
    fn materialize(&mut self, f: &Rc<RefCell<Stream>>) -> Res {
        let chain = match &*f.borrow() {
            Stream::Pending(c) => c.clone(),
            _ => return Ok(()),
        };
        let mut buf: Option<(Vec<u8>, Option<u8>)> = None;
        for spec in &chain {
            buf = Some(match buf.take() {
                None => {
                    let raw = self.lex.data();
                    let (data, used, rgb) = decode(spec, raw)?;
                    self.alloc(data.len())?;
                    self.lex.advance(used);
                    (data, rgb)
                }
                Some((prev, was)) => {
                    let (data, _, rgb) = decode(spec, &prev)?;
                    self.alloc(data.len())?;
                    (data, rgb.or(was))
                }
            });
        }
        let (data, rgb) = buf.unwrap_or_default();
        *f.borrow_mut() = Stream::Buf { data, pos: 0, rgb };
        Ok(())
    }

    /// The bytes a data procedure gives, called until it gives an empty string or `n` bytes.
    fn gather(&mut self, p: &Obj, n: usize) -> Res<Vec<u8>> {
        Ok(self.read_sources(&[Source::Proc(p.clone())], n, n)?.pop().unwrap_or_default())
    }

    /// `n` bytes of image data from `src` (zeros past its end).
    fn source_bytes(&mut self, src: &Source, n: usize) -> Res<Vec<u8>> {
        let mut v = self.read_sources(std::slice::from_ref(src), n, n)?.pop().unwrap_or_default();
        v.resize(n, 0);
        Ok(v)
    }

    /// Up to `n` bytes from each of `sources`, read in turn as an image reads several data sources
    /// (PLRM 3rd ed., §4.10 "Images" and `image`/`colorimage` in chapter 8: the sources are read
    /// in turn, the same amount from each): each procedure called once a round (its string,
    /// whatever its length), each file and string giving `step` bytes a round, until each has
    /// given `n` bytes or ended. Procedures that read one file get their own rows that way.
    fn read_sources(&mut self, sources: &[Source], n: usize, step: usize) -> Res<Vec<Vec<u8>>> {
        let mut out: Vec<Vec<u8>> = vec![vec![]; sources.len()];
        let mut ended = vec![false; sources.len()];
        while out.iter().zip(&ended).any(|(o, e)| o.len() < n && !e) {
            for ((src, o), e) in sources.iter().zip(out.iter_mut()).zip(ended.iter_mut()) {
                if *e || o.len() >= n {
                    continue;
                }
                let want = step.min(n - o.len());
                let chunk = match src {
                    Source::File(f) => self.read(f, want)?,
                    // A string gives its bytes over and over.
                    Source::Str(s) => {
                        let at = o.len() % s.len().max(1);
                        s.get(at..).unwrap_or_default().iter().chain(s.iter().cycle()).take(want).copied().collect()
                    }
                    Source::Proc(p) => {
                        self.call(p.clone())?;
                        let s = self.pop_str()?.to_vec();
                        self.alloc(s.len())?;
                        s
                    }
                };
                *e = chunk.is_empty();
                o.extend_from_slice(chunk.get(..chunk.len().min(n - o.len())).unwrap_or_default());
            }
        }
        Ok(out)
    }

    /// `eexec`: an encrypted font program follows. It is skipped (type is drawn in the app's
    /// fonts, by name): the font dictionary it would complete is defined as it is.
    fn eexec(&mut self) -> Res {
        self.pop()?;
        if let Some(Obj::Dict(d)) = self.stack.last().cloned() {
            let name = d.borrow().get(&Key::name("FontName")).cloned();
            if let Some(k) = name.and_then(|n| n.key()) {
                self.pop()?;
                self.fonts.borrow_mut().insert(k, Obj::Dict(d));
            }
        }
        let rest = self.lex.data();
        let at = find(rest, b"cleartomark").ok_or(PsError::Ps("ioerror", "eexec".into()))?;
        self.lex.advance(at);
        self.push(Obj::Mark)
    }

    // ---------- images ----------

    fn image(&mut self, op: Op) -> Res {
        let mask = op == Op::ImageMask;
        let img = match self.stack.last() {
            Some(Obj::Dict(_)) if op != Op::ColorImage => {
                let d = self.pop_dict()?;
                let kind = d.borrow().get(&Key::name("ImageType")).and_then(Obj::as_num);
                if kind == Some(3.0) && !mask {
                    self.masked_image(&d)?
                } else {
                    let spec = self.image_dict(&d, mask)?;
                    self.decode_image(spec)?
                }
            }
            _ => {
                let spec = self.image_operands(op)?;
                self.decode_image(spec)?
            }
        };
        let Some(img) = img else { return Ok(()) };
        self.place_image(img)
    }

    /// ImageType 3: an image and the mask that says where it paints (where the mask's samples
    /// read below one half through its `Decode`, as an image mask paints its 0 bits), in separate
    /// sources (`InterleaveType` 3) or interleaved with the image's by row (2) or by sample (1).
    fn masked_image(&mut self, d: &DictRef) -> Res<Option<Pixels>> {
        let get = |k: &str| d.borrow().get(&Key::name(k)).cloned();
        let (Some(Obj::Dict(data)), Some(Obj::Dict(mask))) = (get("DataDict"), get("MaskDict")) else {
            return ps_err("typecheck", "ImageType 3");
        };
        let interleave = get("InterleaveType").and_then(|o| o.as_num()).unwrap_or(1.0);
        let mut image = self.image_dict(&data, false)?;
        let mut stencil = self.image_dict(&mask, false)?;
        stencil.space = Space::Gray;
        let n = image.space.n();
        if image.w.saturating_mul(image.h) > MAX_PIXELS || stencil.w.saturating_mul(stencil.h) > MAX_PIXELS {
            self.out.warn(TOO_LARGE);
            return Ok(None);
        }
        if interleave != 3.0 {
            let src = image.sources.first().cloned().ok_or(PsError::Ps("undefined", "DataSource".into()))?;
            let (pixels, samples) = if interleave == 2.0 {
                let (mrow, drow) = (stencil.row_bytes(1), image.row_bytes(n));
                let total = mrow.saturating_mul(stencil.h).saturating_add(drow.saturating_mul(image.h));
                self.alloc(total)?;
                let raw = self.source_bytes(&src, total.min(MAX_DATA))?;
                by_rows(&raw, (mrow, stencil.h), (drow, image.h))
            } else {
                // Each sample: the mask's value, then the colour's, as many bits each.
                stencil.bpc = image.bpc;
                (stencil.w, stencil.h) = (image.w, image.h);
                let row = (image.w * (n + 1) * usize::from(image.bpc)).div_ceil(8);
                self.alloc(row.saturating_mul(image.h))?;
                let raw = self.source_bytes(&src, row.saturating_mul(image.h).min(MAX_DATA))?;
                by_samples(&raw, image.w, image.h, n, image.bpc)
            };
            stencil.sources = vec![Source::Str(pixels)];
            image.sources = vec![Source::Str(samples)];
        }
        let Some(mut img) = self.decode_image(image)? else { return Ok(None) };
        let Some(m) = self.decode_image(stencil)? else { return Ok(Some(img)) };
        let (w, h, mw, mh) = (img.w as usize, img.h as usize, m.w as usize, m.h as usize);
        for (i, px) in img.rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % w, i / w);
            // The mask's pixel over this one (in 64 bits: rows times rows overflow 32).
            let at = (y as u64 * mh as u64 / h.max(1) as u64) * mw as u64 + x as u64 * mw as u64 / w.max(1) as u64;
            if usize::try_from(at * 4).ok().and_then(|i| m.rgba.get(i)).is_none_or(|v| *v >= 128)
                && let Some(a) = px.get_mut(3)
            {
                *a = 0;
            }
        }
        Ok(Some(img))
    }

    /// The dictionary form of `image` and `imagemask`.
    fn image_dict(&mut self, d: &DictRef, mask: bool) -> Res<Spec> {
        let get = |k: &str| d.borrow().get(&Key::name(k)).cloned();
        let num = |k: &str| get(k).and_then(|o| o.as_num());
        let kind = num("ImageType").unwrap_or(1.0);
        if kind != 1.0 && kind != 4.0 {
            return ps_err("rangecheck", "ImageType");
        }
        let size = |k: &str| num(k).filter(|v| *v >= 1.0 && *v <= 65_535.0).map(|v| v as usize).ok_or(PsError::Ps("rangecheck", k.to_string()));
        let (w, h) = (size("Width")?, size("Height")?);
        let bpc = if mask { 1 } else { num("BitsPerComponent").unwrap_or(8.0) as u8 };
        let matrix =
            get("ImageMatrix").and_then(|o| o.items().and_then(|i| matrix_of(&i.borrow()))).ok_or(PsError::Ps("typecheck", "ImageMatrix".into()))?;
        let decode: Vec<f64> = get("Decode").and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())).unwrap_or_default();
        let space = if mask { Space::Gray } else { (*self.g.space).clone() };
        let multi = get("MultipleDataSources").is_some_and(|o| matches!(o, Obj::Bool(true)));
        let sources = match (get("DataSource"), multi) {
            (None, _) => vec![],
            (Some(Obj::Array { items, exec: false }), true) => items.borrow().iter().map(source).collect::<Res<Vec<_>>>()?,
            (Some(src), _) => vec![source(&src)?],
        };
        let mask_color: Option<Vec<u32>> = (kind == 4.0)
            .then(|| {
                get("MaskColor").and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(|v| v.as_num().map(|v| v.max(0.0) as u32)).collect()))
            })
            .flatten();
        // An image mask paints its 0 bits with Decode [0 1], its 1 bits with [1 0].
        let stencil = mask.then(|| decode.first().is_some_and(|v| *v > 0.5));
        Ok(Spec { w, h, bpc, space, decode, matrix, sources, mask_color, stencil })
    }

    /// The operand forms: `w h bpc matrix src image`, `w h polarity matrix src imagemask`, and
    /// `w h bpc matrix src… multi ncomp colorimage`.
    fn image_operands(&mut self, op: Op) -> Res<Spec> {
        let (n, multi) = if op == Op::ColorImage {
            let n = self.pop_count()?;
            (n, self.pop_bool()?)
        } else {
            (1, false)
        };
        let space = match n {
            1 => Space::Gray,
            3 => Space::Rgb,
            4 => Space::Cmyk,
            _ => return ps_err("rangecheck", "colorimage"),
        };
        let mut sources = vec![];
        for _ in 0..if multi { n } else { 1 } {
            let o = self.pop()?;
            sources.push(source(&o)?);
        }
        sources.reverse();
        let matrix = self.pop_matrix()?;
        let (bpc, stencil) = if op == Op::ImageMask { (1, Some(self.pop_bool()?)) } else { (self.pop_int()?.clamp(1, 16) as u8, None) };
        let size = |v: i64| usize::try_from(v).ok().filter(|v| (1..=65_535).contains(v)).ok_or(PsError::Ps("rangecheck", "image".into()));
        let h = size(self.pop_int()?)?;
        let w = size(self.pop_int()?)?;
        Ok(Spec { w, h, bpc, space, decode: vec![], matrix, sources, mask_color: None, stencil })
    }

    /// The image's pixels, straight RGBA, rows from the first one read, and a CMYK image's ink
    /// amounts.
    fn decode_image(&mut self, s: Spec) -> Res<Option<Pixels>> {
        if !matches!(s.bpc, 1 | 2 | 4 | 8 | 12 | 16) {
            return ps_err("rangecheck", "BitsPerComponent");
        }
        if s.w.saturating_mul(s.h) > MAX_PIXELS {
            self.out.warn(TOO_LARGE);
            return Ok(None);
        }
        self.alloc(s.w * s.h * 4)?;
        let n = if s.stencil.is_some() { 1 } else { s.space.n() };
        let bpc = usize::from(s.bpc);
        for src in &s.sources {
            if let Source::File(f) = src {
                self.materialize(f)?;
            }
        }
        // A decoded JPEG gives 8-bit gray, RGB or CMYK samples whatever the color space says.
        let jpeg = s.sources.iter().find_map(|src| match src {
            Source::File(f) => match &*f.borrow() {
                Stream::Buf { rgb, .. } => *rgb,
                _ => None,
            },
            _ => None,
        });
        let (n, bpc, space) = match jpeg {
            Some(3) => (3, 8, Space::Rgb),
            Some(4) => (4, 8, Space::Cmyk),
            Some(_) => (1, 8, Space::Gray),
            None => (n, bpc, s.space.clone()),
        };
        let multi = s.sources.len() > 1;
        let row_bits = if multi { s.w * bpc } else { s.w * n * bpc };
        let row = row_bits.div_ceil(8);
        // Several sources are read in turn, a row at a time each (zeros past their ends).
        let sources = if multi { &s.sources[..] } else { s.sources.get(..1).ok_or(PsError::Ps("undefined", "DataSource".into()))? };
        let mut planes = self.read_sources(sources, row * s.h, if multi { row } else { row * s.h })?;
        for p in &mut planes {
            p.resize(row * s.h, 0);
        }
        let max = ((1u32 << bpc.min(16)) - 1) as f64;
        let sample = |plane: &[u8], y: usize, i: usize| -> u32 {
            let bit = y * row * 8 + i * bpc;
            let byte = bit / 8;
            match bpc {
                8 => u32::from(plane.get(byte).copied().unwrap_or(0)),
                16 => u32::from(plane.get(byte).copied().unwrap_or(0)) << 8 | u32::from(plane.get(byte + 1).copied().unwrap_or(0)),
                12 => {
                    let v = u32::from(plane.get(byte).copied().unwrap_or(0)) << 8 | u32::from(plane.get(byte + 1).copied().unwrap_or(0));
                    if bit.is_multiple_of(8) { v >> 4 } else { v & 0xfff }
                }
                _ => (u32::from(plane.get(byte).copied().unwrap_or(0)) >> (8 - bpc - bit % 8)) & ((1 << bpc) - 1),
            }
        };
        let default_decode: Vec<f64> = match &space {
            Space::Indexed { .. } => vec![0.0, max],
            _ => (0..n).flat_map(|_| [0.0, 1.0]).collect(),
        };
        // The Decode array applies to a CMYK JPEG's samples as stored; gray and RGB JPEGs are read
        // with the default Decode array.
        let decode = if s.decode.len() >= 2 * n && jpeg.is_none_or(|c| c == 4) { s.decode.clone() } else { default_decode };
        let paint = self.g.paint.color().map(|c| c.to_rgba8(1.0));
        let mut rgba = Vec::with_capacity(s.w * s.h * 4);
        // CMYK samples keep their ink amounts, as the Decode array reads them.
        let mut inks = if matches!(space, Space::Cmyk) && s.stencil.is_none() {
            self.alloc(s.w * s.h * 4)?;
            Some(Vec::with_capacity(s.w * s.h * 4))
        } else {
            None
        };
        let mut cache: HashMap<Vec<u32>, [u8; 3]> = HashMap::new();
        let mut raw = vec![0u32; n];
        let mut comps = vec![0f64; n];
        for y in 0..s.h {
            for x in 0..s.w {
                for (c, slot) in raw.iter_mut().enumerate() {
                    *slot = match planes.as_slice() {
                        [one] => sample(one, y, x * n + c),
                        many => many.get(c).map_or(0, |p| sample(p, y, x)),
                    };
                }
                if let Some(paint_ones) = s.stencil {
                    let on = (raw.first().copied().unwrap_or(0) == 1) == paint_ones;
                    let [r, g, b, _] = paint.unwrap_or([0, 0, 0, 255]);
                    rgba.extend([r, g, b, if on { 255 } else { 0 }]);
                    continue;
                }
                let masked = s.mask_color.as_ref().is_some_and(|m| masked(m, &raw));
                for (c, slot) in comps.iter_mut().enumerate() {
                    let (d0, d1) = (decode.get(2 * c).copied().unwrap_or(0.0), decode.get(2 * c + 1).copied().unwrap_or(1.0));
                    *slot = d0 + f64::from(*raw.get(c).unwrap_or(&0)) * (d1 - d0) / max;
                }
                if let Some(inks) = inks.as_mut() {
                    inks.extend(comps.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
                }
                let rgb = match (&space, raw.as_slice()) {
                    (Space::Gray, [v]) if bpc == 8 && decode == [0.0, 1.0] => [*v as u8; 3],
                    (Space::Rgb, [r, g, b]) if bpc == 8 && decode == [0.0, 1.0, 0.0, 1.0, 0.0, 1.0] => [*r as u8, *g as u8, *b as u8],
                    _ => match cache.get(&raw) {
                        Some(c) => *c,
                        None => {
                            let c = self.sample_rgb(&space, &comps)?;
                            if cache.len() < MAX_TINTS {
                                cache.insert(raw.clone(), c);
                            }
                            c
                        }
                    },
                };
                rgba.extend([rgb[0], rgb[1], rgb[2], if masked { 0 } else { 255 }]);
            }
        }
        Ok(Some(Pixels { w: s.w as u32, h: s.h as u32, rgba, inks, matrix: s.matrix }))
    }

    /// One pixel's colour in `space` as display RGB.
    fn sample_rgb(&mut self, space: &Space, comps: &[f64]) -> Res<[u8; 3]> {
        let color = match space {
            Space::Gray | Space::Rgb | Space::Cmyk => process(space, comps),
            _ => match self.paint_of(space, comps)? {
                Paint::Solid { color, .. } => Some(color),
                _ => None,
            },
        };
        let [r, g, b, _] = color.unwrap_or(Color::BLACK).to_rgba8(1.0);
        Ok([r, g, b])
    }

    /// Place decoded pixels in user space (its unit square through the image matrix).
    fn place_image(&mut self, img: Pixels) -> Res {
        let m = img.matrix;
        if m.determinant().abs() < 1e-12 {
            return Ok(());
        }
        let xf = self.xf() * m.inverse();
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            return Ok(());
        }
        if self.g.null {
            return Ok(());
        }
        // CMYK samples keep their ink amounts as a CMYK TIFF, as the PDF importer keeps an 8- or
        // 16-bit CMYK image's samples, and transparent pixels (`MaskColor`, an ImageType 3 mask)
        // become its opacity mask.
        if let Some(blob) = img.inks.and_then(|i| Inks::new(img.w, img.h, i)).and_then(|i| ImageBlob::cmyk_tiff(&i)) {
            let alpha: Vec<u8> = img.rgba.iter().skip(3).step_by(4).copied().collect();
            let mask = self.alpha_mask(&alpha, img.w, img.h, xf);
            return self.add_image(blob, img.w, img.h, xf, mask);
        }
        let Some(rgba) = image::RgbaImage::from_raw(img.w, img.h, img.rgba) else { return Ok(()) };
        let mut png = vec![];
        if rgba.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            self.out.warn("an image that couldn't be read was left out");
            return Ok(());
        }
        self.add_image(ImageBlob::new("image/png", png), img.w, img.h, xf, None)
    }

    /// Add image `blob` (`w` × `h` pixels, mapped into user space by `xf`) with `mask` as its
    /// opacity mask.
    fn add_image(&mut self, blob: ImageBlob, w: u32, h: u32, xf: Affine, mask: Option<OpacityMask>) -> Res {
        let key = blob.content_key();
        self.out.doc.images.entry(key.clone()).or_insert(blob);
        let im = ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default() };
        let clips = self.g.clips.clone();
        self.out.push(Node::new(NodeId(0), NodeKind::Image(Box::new(im))).with_mask(mask.map(Box::new)), &clips);
        Ok(())
    }

    /// An image's alpha (`w` × `h` bytes) as a grayscale image over the image (`xf`): its opacity
    /// mask, as in PDF import. `None` when it hides nothing.
    fn alpha_mask(&mut self, alpha: &[u8], w: u32, h: u32, xf: Affine) -> Option<OpacityMask> {
        if alpha.iter().all(|v| *v == 255) {
            return None;
        }
        let mut png = Vec::new();
        image::GrayImage::from_raw(w, h, alpha.to_vec())?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        self.out.doc.images.entry(key.clone()).or_insert(blob);
        let im = ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default() };
        Some(OpacityMask::new(Node::new(self.out.doc.alloc_id(), NodeKind::Image(Box::new(im))), true))
    }
}

/// An image to decode.
struct Spec {
    w: usize,
    h: usize,
    bpc: u8,
    space: Space,
    decode: Vec<f64>,
    /// User space → image space.
    matrix: Affine,
    /// One source, or one per component.
    sources: Vec<Source>,
    /// `MaskColor` (ImageType 4): samples that are transparent.
    mask_color: Option<Vec<u32>>,
    /// An image mask: whether its 1 bits paint.
    stencil: Option<bool>,
}

impl Spec {
    /// Bytes a row of `n` components takes.
    fn row_bytes(&self, n: usize) -> usize {
        (self.w * n * usize::from(self.bpc)).div_ceil(8)
    }
}

/// Data of an ImageType 3 interleaved by row, split into the mask's and the image's: the one
/// with fewer rows gives one row before each run of the other's (`(row bytes, rows)` each).
fn by_rows(raw: &[u8], (mrow, mh): (usize, usize), (drow, dh): (usize, usize)) -> (Vec<u8>, Vec<u8>) {
    let room = |row: usize, rows: usize| row.saturating_mul(rows).min(raw.len());
    let (mut mask, mut data) = (Vec::with_capacity(room(mrow, mh)), Vec::with_capacity(room(drow, dh)));
    let mut at = 0;
    let mut take = |into: &mut Vec<u8>, n: usize| {
        into.extend_from_slice(raw.get(at..at + n).unwrap_or_default());
        at += n;
    };
    let (blocks, masks, rows) = if mh >= dh { (dh, mh / dh.max(1), 1) } else { (mh, 1, dh / mh.max(1)) };
    for _ in 0..blocks {
        for _ in 0..masks {
            take(&mut mask, mrow);
        }
        for _ in 0..rows {
            take(&mut data, drow);
        }
    }
    (mask, data)
}

/// Data of an ImageType 3 interleaved by sample (the mask's value first, then `n` colour
/// components, `bpc` bits each, rows from whole bytes), split into the mask's and the image's.
fn by_samples(raw: &[u8], w: usize, h: usize, n: usize, bpc: u8) -> (Vec<u8>, Vec<u8>) {
    let bpc = usize::from(bpc);
    let bit = |at: usize| raw.get(at / 8).is_some_and(|b| b & (0x80 >> (at % 8)) != 0);
    let (mrow, drow, row) = ((w * bpc).div_ceil(8), (w * n * bpc).div_ceil(8), (w * (n + 1) * bpc).div_ceil(8));
    let (mut mask, mut data) = (vec![0u8; mrow * h], vec![0u8; drow * h]);
    let put = |into: &mut [u8], at: usize| {
        if let Some(b) = into.get_mut(at / 8) {
            *b |= 0x80 >> (at % 8);
        }
    };
    for y in 0..h {
        for x in 0..w {
            let from = y * row * 8 + x * (n + 1) * bpc;
            for k in 0..bpc {
                if bit(from + k) {
                    put(&mut mask, y * mrow * 8 + x * bpc + k);
                }
            }
            for k in 0..n * bpc {
                if bit(from + bpc + k) {
                    put(&mut data, y * drow * 8 + x * n * bpc + k);
                }
            }
        }
    }
    (mask, data)
}

/// Decoded pixels.
struct Pixels {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
    /// A CMYK image's ink amounts: C, M, Y and K bytes a pixel, 0 is no ink.
    inks: Option<Vec<u8>>,
    matrix: Affine,
}

fn source(o: &Obj) -> Res<Source> {
    Ok(match o {
        Obj::File { stream: f, .. } => Source::File(f.clone()),
        Obj::Str(s) => Source::Str(s.to_vec()),
        Obj::Array { items, .. } => Source::Proc(Obj::Array { items: items.clone(), exec: true }),
        _ => return ps_err("typecheck", "DataSource"),
    })
}

/// Do samples `raw` match `MaskColor` `m` (one value per component, or a range)?
fn masked(m: &[u32], raw: &[u32]) -> bool {
    if m.len() == raw.len() {
        return m.iter().zip(raw).all(|(a, b)| a == b);
    }
    m.len() == raw.len() * 2
        && raw.iter().enumerate().all(|(i, v)| (m.get(2 * i).copied().unwrap_or(0)..=m.get(2 * i + 1).copied().unwrap_or(0)).contains(v))
}
