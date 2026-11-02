use super::filters::{Filter, Predictor, Stream, decode, fax_settings, io};
use super::pixels::{Pixels, Spec, by_rows, by_samples, masked, source};
use super::{
    Affine, Color, DictRef, HashMap, ImageBlob, ImageObject, Inks, Interp, Key, Lexer, Node, NodeId, NodeKind, Obj, Op, OpacityMask, Paint, PsError,
    Rc, RefCell, Res, Shared, Space, find, matrix_of, process, ps_err,
};

/// Most bytes one filter or image reads.
pub(super) const MAX_DATA: usize = 1 << 28;

/// Most pixels an image may have.
const MAX_PIXELS: usize = 1 << 26;

/// Why an image is missing.
const TOO_LARGE: &str = "images too large to read were left out";

/// Most distinct colours an image converts through a tint transform.
const MAX_TINTS: usize = 1 << 16;

/// What a source of image data gives.
#[derive(Clone)]
pub(super) enum Source {
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
                            self.skip_lf(&f);
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

    /// After `readline` consumes CR, consume only an immediately following LF. PostScript
    /// treats CR/LF as one line ending (PLRM §3.8); raw reads must keep every other byte.
    /// The preceding `read` has already materialized any pending filter.
    fn skip_lf(&mut self, f: &Rc<RefCell<Stream>>) {
        match &mut *f.borrow_mut() {
            Stream::Current => {
                if self.lex.rest().first() == Some(&b'\n') {
                    self.lex.advance(1);
                }
            }
            Stream::Buf { data, pos, .. } => {
                if data.get(*pos) == Some(&b'\n') {
                    *pos += 1;
                }
            }
            Stream::Pending(_) => {}
        }
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

    /// Count `n` decoded bytes against the memory budget, and the time to decode them.
    fn charge_decoded(&mut self, n: usize) -> Res {
        self.alloc(n)?;
        self.spend((n / 4096) as u64)
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
        let (mut predictor, mut params) = (None, None);
        if let Some(Obj::Dict(d)) = self.stack.last().cloned() {
            self.pop()?;
            predictor = Predictor::of(&d)?;
            params = Some(d);
        }
        let fax = if &*name == "CCITTFaxDecode" { Some(fax_settings(params.as_ref())?) } else { None };
        let spec_name = name.clone();
        let spec = Filter { name, sub, predictor, fax };
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
                        self.charge_decoded(out.len())?;
                        *pos = pos.saturating_add(used);
                        Stream::Buf { data: out, pos: 0, rgb }
                    }
                }
            }
            Obj::Str(s) => {
                self.alloc(s.borrow().len())?;
                let (data, _, rgb) = decode(&spec, &s.borrow())?;
                self.charge_decoded(data.len())?;
                Stream::Buf { data, pos: 0, rgb }
            }
            p @ Obj::Array { .. } => {
                let raw = self.gather(&p, MAX_DATA)?;
                let (data, _, rgb) = decode(&spec, &raw)?;
                self.charge_decoded(data.len())?;
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
    pub(in super::super) fn program_of(&mut self, f: &Rc<RefCell<Stream>>) -> Res<Option<Vec<u8>>> {
        if matches!(&*f.borrow(), Stream::Current) {
            return Ok(None);
        }
        self.read(f, MAX_DATA).map(Some)
    }

    /// The decoded data left in file `f`, without reading it (a shading's or function's data).
    pub(in super::super) fn peek_all(&mut self, f: &Rc<RefCell<Stream>>) -> Res<Vec<u8>> {
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
                    self.charge_decoded(data.len())?;
                    self.lex.advance(used);
                    (data, rgb)
                }
                Some((prev, was)) => {
                    let (data, _, rgb) = decode(spec, &prev)?;
                    self.charge_decoded(data.len())?;
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
        if sources.len() > n {
            return ps_err("rangecheck", "DataSource");
        }
        let plane = row.checked_mul(s.h).filter(|p| *p <= MAX_DATA).ok_or(PsError::Limit("decoded data is too large"))?;
        self.alloc(plane.saturating_mul(sources.len()))?;
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
        let im = ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None };
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
        let im = ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None };
        Some(OpacityMask::new(Node::new(self.out.doc.alloc_id(), NodeKind::Image(Box::new(im))), true))
    }
}
