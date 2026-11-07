//! Fonts and type: fonts are found by name (their programs aren't run) and type becomes point
//! type in the app's font of that name, sized and turned as the font matrix and the current
//! transform say.

use std::rc::Rc;

use std::sync::Arc;
use vectorcraft_color::Paint;

use vectorcraft_doc::{CharStyle, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Affine, Point, Vec2};
use vectorcraft_text::TextLayout;

use super::graphics::MAX_APART;
use super::interp::{Interp, matrix_obj, matrix_of};
use super::obj::{Dict, DictRef, Key, Obj, Op, PsError, Res, ps_err};

/// The font a program uses before it sets one.
const DEFAULT_FONT: &str = "Helvetica";

/// A word of a style as PostScript names abbreviate it (`AkzidenzGroteskPro-BoldCnIt`), in full.
fn style_word(w: &str) -> String {
    match w {
        "Lt" => "Light",
        "Md" | "Med" => "Medium",
        "Bd" => "Bold",
        "XBd" => "Extra Bold",
        "Blk" => "Black",
        "Hv" => "Heavy",
        "Th" => "Thin",
        "Rg" => "Regular",
        "Sb" | "Smbd" => "Semibold",
        "Cn" | "Cond" => "Condensed",
        "Ext" | "Ex" => "Extended",
        "It" => "Italic",
        "Obl" => "Oblique",
        w => w,
    }
    .to_string()
}

/// A font's family and style from its PostScript name, without a subset prefix (`ABCDEF+`) or the
/// `*1` of a re-encoded copy (`ABCDEF+RollerBabyBV*1`): an installed or bundled face of that
/// exact name gives its own (a family can hold hyphens: `rounded-x-mplus-1c-black` is Rounded-X
/// M+ 1c, black); else the name is split, `Helvetica-BoldOblique` → (`Helvetica`, `Bold
/// Oblique`), `TimesNewRomanPS-BoldMT` → (`Times New Roman`, `Bold`).
pub fn family_style(name: &str) -> (String, String) {
    let name = match name.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 && prefix.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => name,
    };
    let name = match name.rsplit_once('*') {
        Some((base, n)) if !base.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => name,
    };
    if let Some(face) = vectorcraft_text::FontDb::global().find_postscript(name) {
        return (face.family.clone(), face.style.clone());
    }
    let (family, style) = name.split_once('-').unwrap_or((name, ""));
    let trim = |s: &str| -> String {
        let s = ["PSMT", "MT", "PS"].iter().find_map(|suffix| s.strip_suffix(suffix)).unwrap_or(s);
        // Words: a capital after a small letter starts one.
        let mut out = String::with_capacity(s.len() + 4);
        let mut prev_lower = false;
        for ch in s.chars() {
            if ch.is_ascii_uppercase() && prev_lower {
                out.push(' ');
            }
            prev_lower = ch.is_ascii_lowercase();
            out.push(ch);
        }
        out
    };
    let style = match trim(style).as_str() {
        "" | "Roman" | "Regular" | "Book" | "Normal" => "Regular".to_string(),
        s => s.split(' ').map(style_word).collect::<Vec<_>>().join(" "),
    };
    (trim(family), style)
}

impl Interp<'_> {
    pub fn text_op(&mut self, op: Op) -> Res {
        use Op::*;
        match op {
            FindFont => {
                let k = self.pop()?;
                let font = self.find_font(&k)?;
                self.push(Obj::Dict(font))?;
            }
            DefineFont => {
                let font = self.pop_dict()?;
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", "definefont".into()))?;
                font.borrow_mut().entry(Key::name("FontName")).or_insert_with(|| k.obj());
                self.fonts.borrow_mut().insert(k, Obj::Dict(font.clone()));
                self.push(Obj::Dict(font))?;
            }
            UndefineFont => {
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", "undefinefont".into()))?;
                self.fonts.borrow_mut().remove(&k);
            }
            ScaleFont | MakeFont => {
                let m = if op == ScaleFont {
                    let s = self.pop_num()?;
                    Affine::scale(s)
                } else {
                    self.pop_matrix()?
                };
                let font = self.pop_dict()?;
                self.push(Obj::Dict(transformed(&font, m)))?;
            }
            SetFont => {
                let font = self.pop_dict()?;
                self.g.font = Some(font);
            }
            SelectFont => {
                let m = match self.pop()? {
                    Obj::Array { items, .. } => matrix_of(&items.borrow()).ok_or(PsError::Ps("typecheck", "selectfont".into()))?,
                    o => Affine::scale(o.as_num().ok_or(PsError::Ps("typecheck", "selectfont".into()))?),
                };
                let k = self.pop()?;
                let font = self.find_font(&k)?;
                self.g.font = Some(transformed(&font, m));
            }
            CurrentFont => {
                let font = match self.g.font.clone() {
                    Some(f) => f,
                    None => self.find_font(&Obj::name(DEFAULT_FONT))?,
                };
                self.push(Obj::Dict(font))?;
            }
            FindResource => {
                let category = self.pop()?;
                let k = self.pop()?;
                if category.text().as_deref() == Some("Font") {
                    let font = self.find_font(&k)?;
                    return self.push(Obj::Dict(font));
                }
                let key = k.key().ok_or(PsError::Ps("typecheck", "findresource".into()))?;
                let v = self.category(&category)?.borrow().get(&key).cloned();
                self.push(v.ok_or_else(|| PsError::Ps("undefinedresource", k.text().map(|t| t.to_string()).unwrap_or_default()))?)?;
            }
            DefineResource => {
                let category = self.pop()?;
                let v = self.pop()?;
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", "defineresource".into()))?;
                // A category is defined by its implementation dictionary.
                if category.text().as_deref() == Some("Category") && !matches!(v, Obj::Dict(_)) {
                    return ps_err("typecheck", "defineresource");
                }
                self.instances(&category)?.borrow_mut().insert(k, v.clone());
                self.push(v)?;
            }
            UndefineResource => {
                let category = self.pop()?;
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", "undefineresource".into()))?;
                self.instances(&category)?.borrow_mut().remove(&k);
            }
            ResourceStatus => {
                let category = self.pop()?;
                let k = self.pop()?.key().ok_or(PsError::Ps("typecheck", "resourcestatus".into()))?;
                let known = self.instances(&category)?.borrow().contains_key(&k);
                if known {
                    self.push(Obj::Int(1))?;
                    self.push(Obj::Int(0))?;
                }
                self.push(Obj::Bool(known))?;
            }
            ResourceForAll => self.resource_for_all()?,
            Show | AShow | WidthShow | AWidthShow | XShow | YShow | XYShow | KShow => {
                let (s, spacing) = self.show_operands(op)?;
                if let Some(glyphs) = self.composite_glyphs(&s) {
                    self.show_composite(&glyphs, &spacing)?;
                } else {
                    match self.type3_font() {
                        Some(font) => self.show_type3(&font, &s, &spacing)?,
                        None => self.show_plain(&s, &spacing)?,
                    }
                }
            }
            CShow => {
                // The procedure shows or places each character: it gets its code and width.
                let s = self.pop_str()?.to_vec();
                let proc = self.pop_proc()?;
                for b in s {
                    let v = self.width_of(&[b])?;
                    self.push(Obj::Int(i64::from(b)))?;
                    self.push_num(v.x)?;
                    self.push_num(v.y)?;
                    if !self.body(&proc)? {
                        break;
                    }
                }
            }
            GlyphShow => {
                let name = self.pop()?.text().unwrap_or_else(|| Rc::from(""));
                match self.type3_font() {
                    Some(font) => {
                        let start = self.out.drawn.len();
                        let at = self.user_point()?;
                        let w = self.type3_glyph(&font, Glyph::Name(name.clone()), at, true)?;
                        self.g.cur = Some(self.xf() * (at + w));
                        self.group_glyphs(start, &name);
                    }
                    None => {
                        let ch = glyph_char(&name);
                        self.show(ch.encode_utf8(&mut [0; 4]).as_bytes())?;
                    }
                }
            }
            StringWidth => {
                let s = self.pop_str()?.to_vec();
                let v = self.width_of(&s)?;
                self.push_num(v.x)?;
                self.push_num(v.y)?;
            }
            CharPath => {
                self.pop_bool()?;
                let s = self.pop_str()?.to_vec();
                self.char_path(&s)?;
            }
            // A glyph procedure gives its width (`w0x w0y` first).
            SetCacheDevice => self.glyph_width = Some(self.nums::<6>()?[..2].try_into().unwrap_or_default()),
            SetCharWidth => self.glyph_width = Some(self.nums::<2>()?),
            SetCacheDevice2 => self.glyph_width = Some(self.nums::<10>()?[..2].try_into().unwrap_or_default()),
            RootFont => {
                let font = match self.g.font.clone() {
                    Some(f) => f,
                    None => self.find_font(&Obj::name(DEFAULT_FONT))?,
                };
                self.push(Obj::Dict(font))?;
            }
            _ => return ps_err("undefined", op.name()),
        }
        Ok(())
    }

    /// The string of a show operator and how it spaces its glyphs.
    fn show_operands(&mut self, op: Op) -> Res<(Vec<u8>, Spacing)> {
        use Op::*;
        let num_vec = |[x, y]: [f64; 2]| Vec2::new(x, y);
        if matches!(op, XShow | YShow | XYShow) {
            // A number array (an encoded number string spaces glyphs by their widths).
            let widths = match self.pop()? {
                Obj::Array { items, .. } => items.borrow().iter().filter_map(Obj::as_num).collect(),
                _ => vec![],
            };
            let s = self.pop_str()?.to_vec();
            return Ok((s, Spacing::Widths(widths, op)));
        }
        let s = self.pop_str()?.to_vec();
        let spacing = match op {
            AShow => Spacing::Add(num_vec(self.nums()?), None),
            WidthShow => {
                let ch = self.pop_int()?;
                Spacing::Add(Vec2::ZERO, Some((ch, num_vec(self.nums()?))))
            }
            AWidthShow => {
                let a = num_vec(self.nums()?);
                let ch = self.pop_int()?;
                Spacing::Add(a, Some((ch, num_vec(self.nums()?))))
            }
            KShow => Spacing::Kern(self.pop_proc()?),
            _ => Spacing::Plain,
        };
        Ok((s, spacing))
    }

    /// The current font when it is a Type 3 font: its glyphs are drawn by its own procedures.
    fn type3_font(&self) -> Option<DictRef> {
        let font = self.g.font.clone()?;
        let f = font.borrow();
        let builds = f.contains_key(&Key::name("BuildGlyph")) || f.contains_key(&Key::name("BuildChar"));
        let type3 = f.get(&Key::name("FontType")).and_then(Obj::as_num) == Some(3.0) && builds;
        drop(f);
        type3.then_some(font)
    }

    /// How far `s` moves the current point (user space).
    fn width_of(&mut self, s: &[u8]) -> Res<Vec2> {
        if let Some(glyphs) = self.composite_glyphs(s) {
            let mut w = Vec2::ZERO;
            for g in &glyphs {
                w += self.composite_glyph(g, Point::new(w.x, w.y), Draw::No)?;
            }
            return Ok(w);
        }
        let Some(font) = self.type3_font() else {
            if let Some((_, glyphs)) = self.type42_plain(s)? {
                let (_, f1) = self.font()?;
                return Ok(glyphs.iter().map(|g| f1 * Point::new(g.width, 0.0) - f1 * Point::ZERO).sum());
            }
            return Ok(self.set_type(s, Point::ZERO)?.2);
        };
        let mut w = Vec2::ZERO;
        for &code in s {
            w += self.type3_glyph(&font, Glyph::Code(code), Point::new(w.x, w.y), false)?;
        }
        Ok(w)
    }

    /// `s` shown in Type 3 font `font`, glyph by glyph, spaced as `spacing` says.
    fn show_type3(&mut self, font: &DictRef, s: &[u8], spacing: &Spacing) -> Res {
        let start = self.out.drawn.len();
        for (i, &code) in s.iter().enumerate() {
            let at = self.user_point()?;
            let w = self.type3_glyph(font, Glyph::Code(code), at, true)?;
            let d = spacing.advance(i, i64::from(code), w);
            self.g.cur = Some(self.xf() * (at + d));
            if let (Spacing::Kern(p), Some(next)) = (spacing, s.get(i + 1)) {
                self.push(Obj::Int(i64::from(code)))?;
                self.push(Obj::Int(i64::from(*next)))?;
                self.call(p.clone())?;
            }
        }
        let text: String = s.iter().map(|b| char::from(*b)).filter(|c| !c.is_control()).collect();
        self.group_glyphs(start, &text);
        Ok(())
    }

    /// Glyph `glyph` of Type 3 font `font` drawn by its `BuildGlyph` (or `BuildChar`) procedure at
    /// user point `at` (only measured unless `paint`) → its width (user space).
    fn type3_glyph(&mut self, font: &DictRef, glyph: Glyph, at: Point, paint: bool) -> Res<Vec2> {
        self.type3_glyph_in(font, glyph, at, paint, Affine::IDENTITY)
    }

    /// [`Self::type3_glyph`] for a descendant of a composite font whose matrix is `outer`.
    fn type3_glyph_in(&mut self, font: &DictRef, glyph: Glyph, at: Point, paint: bool, outer: Affine) -> Res<Vec2> {
        let (fm, build_glyph, build_char, encoding) = {
            let f = font.borrow();
            let get = |k: &str| f.get(&Key::name(k)).cloned();
            let fm = outer * font_matrix(&f, 0.001);
            (fm, get("BuildGlyph"), get("BuildChar"), get("Encoding"))
        };
        let code_of = |name: &str| -> Option<u8> {
            let items = encoding.as_ref()?.items()?.to_vec();
            items.iter().position(|o| o.text().as_deref() == Some(name)).and_then(|i| u8::try_from(i).ok())
        };
        let (name, code) = match glyph {
            Glyph::Code(c) => {
                let name = encoding.as_ref().and_then(|e| e.items()?.get(usize::from(c))).and_then(|o| o.text());
                (name.unwrap_or_else(|| Rc::from(".notdef")), Some(c))
            }
            Glyph::Name(n) => {
                let code = code_of(&n);
                (n, code)
            }
        };
        let arg = match (build_glyph, build_char, code) {
            (Some(p), ..) => Some((p, Obj::Name(name))),
            (None, Some(p), Some(c)) => Some((p, Obj::Int(i64::from(c)))),
            _ => None,
        };
        let Some((proc, arg)) = arg else { return Ok(Vec2::ZERO) };
        // A glyph that shows type in its own font would draw itself without end.
        if self.out.apart >= MAX_APART {
            return ps_err("limitcheck", "a Type 3 glyph");
        }
        self.gsave()?;
        let depth = self.saved.len();
        self.g.ctm = self.g.ctm * Affine::translate(at.to_vec2()) * fm;
        self.take_path();
        self.g.null |= !paint;
        self.glyph_width = None;
        self.out.apart += 1;
        let r = self.push(Obj::Dict(font.clone())).and_then(|()| self.push(arg)).and_then(|()| self.call(proc));
        self.out.apart -= 1;
        let [wx, wy] = self.glyph_width.take().unwrap_or_default();
        self.saved.truncate(depth);
        self.grestore();
        r?;
        Ok(fm * Point::new(wx, wy) - fm * Point::ZERO)
    }

    /// The objects drawn since `start` (one show's glyphs) as one group named `text`, when they
    /// are several under the same clips.
    fn group_glyphs(&mut self, start: usize, text: &str) {
        let here = self.out.chain(&self.g.clips);
        let Some(drawn) = self.out.drawn.get(start..) else { return };
        let same = drawn.iter().all(|(c, _)| c.iter().map(|c| c.id).eq(here.iter().map(|c| c.id)));
        if drawn.len() < 2 || !same {
            return;
        }
        let children: Vec<Arc<Node>> = self.out.drawn.drain(start..).map(|(_, n)| Arc::new(n)).collect();
        let mut group = Node::group(NodeId(0), children);
        group.name = Some(text.chars().take(64).collect());
        let clips = self.g.clips.clone();
        self.out.push(group, &clips);
    }

    /// The font named `k`: one the program defined, else a font of that name.
    fn find_font(&mut self, k: &Obj) -> Res<DictRef> {
        let key = k.key().ok_or(PsError::Ps("typecheck", "findfont".into()))?;
        if let Some(Obj::Dict(d)) = self.fonts.borrow().get(&key) {
            return Ok(d.clone());
        }
        let mut d = Dict::new();
        d.insert(Key::name("FontName"), key.obj());
        d.insert(Key::name("FontType"), Obj::Int(1));
        d.insert(Key::name("FontMatrix"), matrix_obj(Affine::scale(0.001)));
        d.insert(Key::name("FontBBox"), Obj::array(vec![Obj::Int(0), Obj::Int(-250), Obj::Int(1000), Obj::Int(1000)]));
        d.insert(Key::name("Encoding"), Obj::array(vec![Obj::name(".notdef"); 256]));
        let font = DictRef::new(d);
        self.fonts.borrow_mut().insert(key, Obj::Dict(font.clone()));
        Ok(font)
    }

    /// The current font's name and its matrix for a font of one unit (user space).
    fn font(&mut self) -> Res<(String, Affine)> {
        let font = match self.g.font.clone() {
            Some(f) => f,
            None => self.find_font(&Obj::name(DEFAULT_FONT))?,
        };
        let f = font.borrow();
        let name = f.get(&Key::name("FontName")).and_then(Obj::text).map_or_else(|| DEFAULT_FONT.to_string(), |n| n.to_string());
        // A composite font's glyphs are its descendants', in their glyph space under its matrix
        // (PLRM 3rd ed., §5.10 "Composite Fonts"): the first descendant stands for them all.
        if font_type(&f) == Some(0.0) {
            let first = f.get(&Key::name("FDepVector")).and_then(|v| v.items()?.get(0));
            if let Some(Obj::Dict(d)) = first {
                let d = d.borrow();
                return Ok((name, font_matrix(&f, 1.0) * font_matrix(&d, 0.001) * Affine::scale(units(&d))));
            }
        }
        Ok((name, font_matrix(&f, 0.001) * Affine::scale(units(&f))))
    }

    /// `s` (read as Latin-1) set in the current font at user point `at`: point type placed there,
    /// its layout in text space, and how far it moves the current point (user space).
    fn set_type(&mut self, s: &[u8], at: Point) -> Res<(TextObject, TextLayout, Vec2)> {
        let text: String = s.iter().map(|b| char::from(*b)).filter(|c| !c.is_control()).collect();
        self.set_text(&text, at)
    }

    /// [`Self::set_type`] for the characters `text`.
    fn set_text(&mut self, text: &str, at: Point) -> Res<(TextObject, TextLayout, Vec2)> {
        // Laying out type costs about as much as 300 operations, and 64 more a character
        // (measured), so `{ (a) stringwidth pop pop } loop` or `cshow` on a long string ends
        // within the budget like any other loop instead of running for minutes.
        self.spend((text.len() as u64).saturating_mul(64).saturating_add(300))?;
        let (name, f1) = self.font()?;
        // Text space (y down, `size` points an em) onto the document.
        let m = self.xf() * Affine::translate(at.to_vec2()) * f1;
        let size = m.determinant().abs().sqrt();
        if !(size.is_finite() && size > 1e-3 && size < 1e5) {
            return ps_err("undefinedresult", "a font size");
        }
        let (family, font_style) = family_style(&name);
        // The available family's own spelling: `MicrosoftYaHei` is Microsoft YaHei, not "Microsoft Ya Hei".
        let font_family = vectorcraft_text::FontDb::global().find_family(&family).unwrap_or(family);
        let fill = if self.g.paint.is_none() { Paint::solid(vectorcraft_color::Color::BLACK) } else { self.g.paint.clone() };
        let style = CharStyle { font_family, font_style, size, fill, stroke: Paint::None, ..CharStyle::default() };
        let mut t = TextObject::point(Point::ZERO, text, style);
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        t.xf = m * Affine::scale_non_uniform(1.0 / size, -1.0 / size);
        t.cached_bounds = Some(layout.bounds);
        let adv: f64 = layout.glyphs.iter().map(|g| g.advance).sum();
        Ok((t, layout, f1 * Point::new(adv / size, 0.0) - f1 * Point::ZERO))
    }

    /// `show`: point type at the current point, which moves past it.
    fn show(&mut self, s: &[u8]) -> Res {
        self.show_plain(s, &Spacing::Plain)
    }

    /// `s` shown in a font that isn't a Type 3 or composite font, spaced as `spacing` says: point
    /// type at the current point, which moves past it as far as the file says, each character
    /// where the file puts it. A Type 42 font that carries its TrueType font gives each code its
    /// character (by its glyph name, else the character its glyph stands for in the font's own
    /// character map) and its glyph's width; type in a font found by name is spaced by the
    /// installed font's advances. Where the file's spacing differs from the advances of the
    /// installed (or fallback) font, each character's tracking makes up the difference, so type
    /// in a font that isn't installed keeps the file's line lengths and justified lines don't
    /// drift, as PDF import does. Glyphs that stand for no character are drawn from the font's
    /// outlines (the whole string, as one group).
    fn show_plain(&mut self, s: &[u8], spacing: &Spacing) -> Res {
        let at = self.user_point()?;
        let (_, f1) = self.font()?;
        let plain = self.type42_plain(s)?;
        if let Some((data, glyphs)) = &plain
            && glyphs.iter().any(|g| g.ch.is_none() && g.gid != 0)
        {
            return self.show_type42_outlines(data, glyphs, s, spacing, at, f1);
        }
        // Each code's character, if it has one that shows (control codes don't).
        let chars: Vec<Option<char>> = match &plain {
            Some((_, glyphs)) => glyphs.iter().map(|g| g.ch).collect::<Vec<_>>(),
            None => s.iter().map(|b| Some(char::from(*b))).collect(),
        }
        .into_iter()
        .map(|c| c.filter(|c| !c.is_control()))
        .collect();
        let text: String = chars.iter().flatten().collect();
        let (mut t, mut layout, _) = self.set_text(&text, at)?;
        let size = t.runs.first().map_or(1.0, |r| r.style.size);
        let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        // The file places each character's glyph: where the layout joins characters in a
        // ligature, the type is set without ligatures, so each character has its own advance.
        if starts.iter().any(|b| !layout.glyphs.iter().any(|g| g.byte == *b)) {
            for run in &mut t.runs {
                run.style.features.push("-liga".into());
            }
            layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
            t.cached_bounds = Some(layout.bounds);
        }
        // Each character's advance in the layout (text space).
        let mut natural = vec![0.0; starts.len()];
        for g in &layout.glyphs {
            if let Some(n) = starts.binary_search(&g.byte).ok().and_then(|k| natural.get_mut(k)) {
                *n += g.advance;
            }
        }
        // Each code's move (user space) and that move in text space, along the baseline and across.
        let em = |x: f64| f1 * Point::new(x, 0.0) - f1 * Point::ZERO;
        let inv = f1.inverse();
        let mut moves = Vec::with_capacity(s.len());
        let mut k = 0;
        for (i, (&code, c)) in s.iter().zip(&chars).enumerate() {
            let own = match &plain {
                Some((_, glyphs)) => glyphs.get(i).map_or(0.0, |g| g.width),
                None if c.is_some() => natural.get(k).copied().unwrap_or(0.0) / size,
                None => 0.0,
            };
            k += usize::from(c.is_some());
            let d = spacing.advance(i, i64::from(code), em(own));
            let e = inv * Point::new(d.x, d.y) - inv * Point::ZERO;
            if ![d.x, d.y, e.x * size, e.y * size].iter().all(|v| v.is_finite()) {
                return ps_err("undefinedresult", "a show");
            }
            moves.push((d, e.x * size, e.y * size));
        }
        let total: Vec2 = moves.iter().map(|m| m.0).sum();
        if !(total.x.is_finite() && total.y.is_finite()) {
            return ps_err("undefinedresult", "a show");
        }
        let across = moves.iter().any(|m| m.2.abs() > size * 1e-3);
        // Nothing shows on the null device (`nulldevice`, a Type 3 glyph only measured).
        let painted = !self.g.paint.is_none() && !self.g.null;
        if across {
            // Moves across the baseline (`yshow`, `xyshow`): each character where the file puts it.
            let mut pen = at;
            for ((d, ..), c) in moves.iter().zip(&chars) {
                if let Some(c) = c
                    && painted
                    && !c.is_whitespace()
                {
                    let (t, ..) = self.set_text(c.encode_utf8(&mut [0; 4]), pen)?;
                    let clips = self.g.clips.clone();
                    self.out.push(Node::new(NodeId(0), NodeKind::Text(Box::new(t))), &clips);
                }
                pen += *d;
            }
        } else if painted && !text.trim().is_empty() {
            // The moves of codes before the first character move the type; those of codes after
            // a character, before the next, are that character's.
            let mut lead = Vec2::ZERO;
            let mut target: Vec<f64> = Vec::with_capacity(natural.len());
            for ((d, along, _), c) in moves.iter().zip(&chars) {
                match (c, target.last_mut()) {
                    (Some(_), _) => target.push(*along),
                    (None, Some(last)) => *last += along,
                    (None, None) => lead += *d,
                }
            }
            let tracking: Vec<f64> = target
                .iter()
                .zip(&natural)
                // Hundredths of a thousandth of an em: no drift along a line, no noise in the panels.
                .map(|(want, has)| (((want - has) / size * 1000.0).clamp(-1000.0, 10000.0) * 100.0).round() / 100.0)
                .collect();
            if tracking.iter().any(|t| *t != 0.0)
                && let Some(run) = t.runs.first().cloned()
            {
                let mut runs: Vec<vectorcraft_doc::TextRun> = Vec::new();
                for (c, tr) in text.chars().zip(&tracking) {
                    match runs.last_mut() {
                        Some(last) if last.style.tracking == *tr => last.text.push(c),
                        _ => runs.push(vectorcraft_doc::TextRun {
                            text: c.to_string(),
                            style: CharStyle { tracking: *tr, ..run.style.clone() },
                            inline: run.inline.clone(),
                        }),
                    }
                }
                t.runs = runs;
                t.cached_bounds = Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds);
            }
            if lead != Vec2::ZERO {
                t.xf = Affine::translate(self.xf() * Point::new(lead.x, lead.y) - self.xf() * Point::ZERO) * t.xf;
            }
            let clips = self.g.clips.clone();
            self.out.push(Node::new(NodeId(0), NodeKind::Text(Box::new(t))), &clips);
        }
        self.g.cur = Some(self.xf() * (at + total));
        Ok(())
    }

    /// `s` in a Type 42 font, some of whose glyphs stand for no character, drawn from the font's
    /// outlines at the current point (one group), spaced as `spacing` says.
    fn show_type42_outlines(&mut self, data: &super::type42::Font, glyphs: &[Plain], s: &[u8], spacing: &Spacing, at: Point, f1: Affine) -> Res {
        let start = self.out.drawn.len();
        let mut pen = at;
        let mut text = String::new();
        for (i, (g, &code)) in glyphs.iter().zip(s).enumerate() {
            if let Some(outline) = super::type42::glyph(&data.data, g.gid).map(|o| o.0) {
                self.type42_paint(outline, pen, f1, Draw::Fill)?;
            }
            let d = spacing.advance(i, i64::from(code), f1 * Point::new(g.width, 0.0) - f1 * Point::ZERO);
            pen += d;
            text.push(g.ch.unwrap_or('\u{2022}'));
        }
        self.g.cur = Some(self.xf() * pen);
        self.group_glyphs(start, &text);
        Ok(())
    }

    /// Glyph outline `outline` (glyph space) under font matrix `fm` at user point `at`: filled, or
    /// added to the current path (`charpath`).
    fn type42_paint(&mut self, mut outline: vectorcraft_geom::BezPath, at: Point, fm: Affine, paint: Draw) -> Res {
        // Measuring a glyph reads its outline too, so `stringwidth` in a loop pays for it.
        self.spend((outline.elements().len() as u64).saturating_add(16))?;
        if paint == Draw::No {
            return Ok(());
        }
        outline.apply_affine(self.xf() * Affine::translate(at.to_vec2()) * fm);
        if paint == Draw::Fill {
            return self.fill_path(outline, vectorcraft_geom::FillRule::NonZero);
        }
        if self.g.path.elements().len().saturating_add(outline.elements().len()) > super::graphics::MAX_PATH {
            return Err(PsError::Limit("a path has too many points"));
        }
        self.g.path.extend(outline);
        Ok(())
    }

    /// `charpath`: the glyph outlines of `s` added to the path.
    fn char_path(&mut self, s: &[u8]) -> Res {
        if let Some(glyphs) = self.composite_glyphs(s) {
            for g in &glyphs {
                let at = self.user_point()?;
                let w = self.composite_glyph(g, at, Draw::Path)?;
                self.g.cur = Some(self.xf() * (at + w));
            }
            return Ok(());
        }
        if let Some((data, glyphs)) = self.type42_plain(s)? {
            let (_, f1) = self.font()?;
            let mut pen = self.user_point()?;
            for g in &glyphs {
                if let Some((outline, _)) = super::type42::glyph(&data.data, g.gid) {
                    self.type42_paint(outline, pen, f1, Draw::Path)?;
                }
                pen += f1 * Point::new(g.width, 0.0) - f1 * Point::ZERO;
            }
            self.g.cur = Some(self.xf() * pen);
            return Ok(());
        }
        let at = self.user_point()?;
        let (t, layout, adv) = self.set_type(s, at)?;
        for g in &layout.glyphs {
            self.grow()?;
            let mut o = g.outline.clone();
            o.apply_affine(t.xf);
            self.g.path.extend(o);
        }
        self.g.cur = Some(self.xf() * (at + adv));
        Ok(())
    }
}

/// How a show operator spaces its glyphs (Type 3 fonts; type in other fonts is laid out as point
/// type).
enum Spacing {
    /// By their widths.
    Plain,
    /// `ashow`, `widthshow`, `awidthshow`: by their widths plus a displacement, and another for one
    /// character code.
    Add(Vec2, Option<(i64, Vec2)>),
    /// `xshow`, `yshow`, `xyshow` (the operator): by the numbers given.
    Widths(Vec<f64>, Op),
    /// `kshow`: by their widths, the procedure run between each two.
    Kern(Obj),
}

/// A glyph of a Type 3 font, by character code or by name (`glyphshow`).
enum Glyph {
    Code(u8),
    Name(Rc<str>),
}

impl Spacing {
    /// How far glyph `i` (character code `code`, width `w`) moves the current point.
    fn advance(&self, i: usize, code: i64, w: Vec2) -> Vec2 {
        match self {
            Self::Plain | Self::Kern(_) => w,
            Self::Add(a, c) => w + *a + c.filter(|(ch, _)| *ch == code).map_or(Vec2::ZERO, |(_, v)| v),
            Self::Widths(v, op) => {
                let n = |k: usize| v.get(k).copied();
                match op {
                    Op::XShow => n(i).map(|x| Vec2::new(x, 0.0)),
                    Op::YShow => n(i).map(|y| Vec2::new(0.0, y)),
                    _ => n(2 * i).zip(n(2 * i + 1)).map(|(x, y)| Vec2::new(x, y)),
                }
                .unwrap_or(w)
            }
        }
    }
}

/// A character of a composite font: the descendant font that draws it, its code there, and the
/// composite font's matrix (over the descendant's own).
struct Mapped {
    font: DictRef,
    /// The byte that picked the descendant (the font number).
    pick: u8,
    code: u8,
    outer: Affine,
}

/// What drawing a glyph does with it.
#[derive(Clone, Copy, PartialEq)]
enum Draw {
    /// Fills it (`show`).
    Fill,
    /// Adds its outline to the current path (`charpath`).
    Path,
    /// Only measures it (`stringwidth`).
    No,
}

/// Font `f`'s `FontType`.
fn font_type(f: &Dict) -> Option<f64> {
    f.get(&Key::name("FontType")).and_then(Obj::as_num)
}

/// Font `f`'s matrix, `default` times the identity when it has none.
fn font_matrix(f: &Dict, default: f64) -> Affine {
    f.get(&Key::name("FontMatrix")).and_then(|o| o.items().and_then(|i| matrix_of(&i.borrow()))).unwrap_or(Affine::scale(default))
}

/// Font `f`'s glyph units in its matrix's space: Type 1 glyphs are in 1000-unit space; a Type 42
/// font's glyph space is one unit an em (its FontMatrix, usually the identity, is already the
/// em's).
fn units(f: &Dict) -> f64 {
    if font_type(f) == Some(42.0) { 1.0 } else { 1000.0 }
}

impl Interp<'_> {
    /// The characters of `s` in the current font when it is a composite font with the 8/8 mapping
    /// (`FMapType` 2: a byte picking the descendant through `Encoding` and `FDepVector`, then the
    /// code in it: PLRM 3rd ed., §5.10.1 "Type 0 Font Dictionaries"); `None` for other fonts and
    /// mappings, which are set as type in the font of the composite font's name.
    fn composite_glyphs(&self, s: &[u8]) -> Option<Vec<Mapped>> {
        let font = self.g.font.clone()?;
        let f = font.borrow();
        if font_type(&f) != Some(0.0) || f.get(&Key::name("FMapType")).and_then(Obj::as_num) != Some(2.0) {
            return None;
        }
        let (encoding, fonts) = (f.get(&Key::name("Encoding"))?.items()?.clone(), f.get(&Key::name("FDepVector"))?.items()?.clone());
        let (encoding, fonts) = (encoding.borrow(), fonts.borrow());
        let outer = font_matrix(&f, 1.0);
        let mut out = Vec::with_capacity(s.len() / 2);
        for &[pick, code] in s.as_chunks::<2>().0 {
            let index = encoding.get(usize::from(pick)).and_then(Obj::as_num).filter(|i| *i >= 0.0 && i.fract() == 0.0);
            // A character whose descendant is missing (or not a base font) shows nothing.
            if let Some(Obj::Dict(d)) = index.and_then(|i| fonts.get(i as usize))
                && font_type(&d.borrow()) != Some(0.0)
            {
                out.push(Mapped { font: d.clone(), pick, code, outer });
            }
        }
        Some(out)
    }

    /// The characters of a composite font shown at the current point, spaced as `spacing` says.
    fn show_composite(&mut self, glyphs: &[Mapped], spacing: &Spacing) -> Res {
        let start = self.out.drawn.len();
        let mut text = String::new();
        for (i, g) in glyphs.iter().enumerate() {
            let at = self.user_point()?;
            let w = self.composite_glyph(g, at, Draw::Fill)?;
            // `widthshow` and `awidthshow` match a composite font's character by its font number
            // times 256 plus its code (PLRM 3rd ed., `widthshow` in chapter 8).
            let d = spacing.advance(i, i64::from(g.pick) * 256 + i64::from(g.code), w);
            self.g.cur = Some(self.xf() * (at + d));
            let c = code_char(&g.font.borrow(), g.code);
            if c != '\u{2022}' {
                text.push(c);
            }
        }
        if text.trim().is_empty()
            && let Some(name) = self.g.font.as_ref().and_then(|f| f.borrow().get(&Key::name("FontName")).and_then(Obj::text))
        {
            text = name.to_string();
        }
        self.group_glyphs(start, &text);
        Ok(())
    }

    /// Character `g` of a composite font at user point `at`, drawn by its descendant: a Type 3
    /// font's procedure, a Type 42 font's TrueType outline, else as type in the font of the
    /// descendant's name (the character its glyph name stands for) → its width (user space).
    fn composite_glyph(&mut self, g: &Mapped, at: Point, paint: Draw) -> Res<Vec2> {
        let builds = {
            let f = g.font.borrow();
            font_type(&f) == Some(3.0) && (f.contains_key(&Key::name("BuildGlyph")) || f.contains_key(&Key::name("BuildChar")))
        };
        if builds {
            return self.type3_glyph_in(&g.font, Glyph::Code(g.code), at, paint == Draw::Fill, g.outer);
        }
        let fm = g.outer * font_matrix(&g.font.borrow(), 1.0);
        if let Some((outline, advance)) = self.type42_glyph(&g.font, g.code)? {
            self.type42_paint(outline, at, fm, paint)?;
            return Ok(fm * Point::new(advance, 0.0) - fm * Point::ZERO);
        }
        // Set as type in the descendant's font, under the composite font's matrix.
        let c = code_char(&g.font.borrow(), g.code);
        let Ok(byte) = u8::try_from(u32::from(c)) else { return Ok(Vec2::ZERO) };
        let saved = self.g.font.replace(transformed(&g.font, g.outer));
        let r = match paint {
            Draw::Fill => {
                self.g.cur = Some(self.xf() * at);
                self.show(&[byte]).and_then(|()| self.user_point()).map(|p| p - at)
            }
            Draw::Path => {
                self.g.cur = Some(self.xf() * at);
                self.char_path(&[byte]).and_then(|()| self.user_point()).map(|p| p - at)
            }
            Draw::No => self.set_type(&[byte], at).map(|t| t.2),
        };
        self.g.font = saved;
        r
    }

    /// Glyph `code` of Type 42 font `font` from the TrueType font it carries: its outline and
    /// width in its glyph space (ems), or `None` when it isn't a Type 42 font with one.
    fn type42_glyph(&mut self, font: &DictRef, code: u8) -> Res<Option<(vectorcraft_geom::BezPath, f64)>> {
        let Some(data) = self.type42_data(font)? else { return Ok(None) };
        let gid = {
            let f = font.borrow();
            let name = glyph_name(&f, code);
            char_string(&f, &name).or_else(|| char_string(&f, ".notdef")).unwrap_or(0)
        };
        Ok(super::type42::glyph(&data.data, gid))
    }

    /// The codes of `s` in the current font when it is a Type 42 font that carries its TrueType
    /// font (not a descendant of a composite font): the font, and each code's glyph.
    fn type42_plain(&mut self, s: &[u8]) -> Res<Option<(Rc<super::type42::Font>, Vec<Plain>)>> {
        let Some(font) = self.g.font.clone() else { return Ok(None) };
        let Some(data) = self.type42_data(&font)? else { return Ok(None) };
        self.spend((s.len() as u64).saturating_mul(4))?;
        let f = font.borrow();
        let glyphs = s
            .iter()
            .map(|&code| {
                let (ch, gid) = type42_code(&f, &data, code);
                Plain { ch, gid, width: super::type42::advance(&data.data, gid) }
            })
            .collect();
        drop(f);
        // Reading the font's character map (once) visits each of its mappings.
        self.spend(data.take_char_map_work())?;
        Ok(Some((data, glyphs)))
    }

    /// The TrueType font Type 42 font `font` carries (read once for each `sfnts` array), or
    /// `None` when it isn't a Type 42 font with one.
    fn type42_data(&mut self, font: &DictRef) -> Res<Option<Rc<super::type42::Font>>> {
        let sfnts = {
            let f = font.borrow();
            if font_type(&f) != Some(42.0) {
                return Ok(None);
            }
            let Some(Obj::Array { items: sfnts, .. }) = f.get(&Key::name("sfnts")).cloned() else { return Ok(None) };
            sfnts
        };
        // The least recently used font goes first: a hit moves to the end.
        if let Some(i) = self.type42.iter().position(|(k, _)| k.same(&sfnts)) {
            let hit = self.type42.remove(i);
            let data = hit.1.clone();
            self.type42.push(hit);
            return Ok(data);
        }
        // Fonts whose data can't be read are kept too, so they are copied only once.
        let data = super::type42::font_data(&sfnts.borrow()).map(|d| Rc::new(super::type42::Font::new(d)));
        if let Some(d) = &data {
            if self.type42_counted.insert(sfnts.clone()) {
                self.alloc(d.data.len())?;
            }
            self.spend((d.data.len() / 4096) as u64)?;
        }
        if self.type42.len() >= MAX_TYPE42 {
            self.type42.remove(0);
        }
        self.type42.push((sfnts, data.clone()));
        Ok(data)
    }
}

/// A character code of a Type 42 font: the character it stands for (`None` when neither its
/// glyph name nor the font says), its glyph and the glyph's width (ems).
struct Plain {
    ch: Option<char>,
    gid: u32,
    width: f64,
}

/// The glyph index `CharStrings` gives glyph name `name` in Type 42 font `f`.
fn char_string(f: &Dict, name: &str) -> Option<u32> {
    let Some(Obj::Dict(d)) = f.get(&Key::name("CharStrings")) else { return None };
    d.borrow().get(&Key::name(name)).and_then(Obj::as_num).filter(|g| (0.0..=f64::from(u16::MAX)).contains(g)).map(|g| g as u32)
}

/// The character and glyph of code `code` in Type 42 font `f`, carrying TrueType font `data`:
/// by the glyph name its `Encoding` gives it (a name such as `A`, `uni20AC` or `Euro` says the
/// character; else the glyph `CharStrings` gives it stands for one in the font's character map
/// or `post` names). A code the encoding leaves `.notdef` (the standard encodings here have no
/// names) is read as Latin-1, drawn by that character's glyph.
fn type42_code(f: &Dict, data: &super::type42::Font, code: u8) -> (Option<char>, u32) {
    let name = glyph_name(f, code);
    if &*name == ".notdef" {
        let c = char::from(code);
        return (Some(c), data.gid_of(c).or_else(|| char_string(f, ".notdef")).unwrap_or(0));
    }
    let named = named_char(code, &name);
    let gid = char_string(f, &name).or_else(|| named.and_then(|c| data.gid_of(c))).or_else(|| char_string(f, ".notdef")).unwrap_or(0);
    (named.or_else(|| data.char_of(gid)), gid)
}

/// Most Type 42 fonts whose TrueType data is kept between glyphs.
const MAX_TYPE42: usize = 16;

/// The character code `code` of font `f` stands for: its glyph name's, or the code read as
/// Latin-1 when the font names no glyph for it (a font found by name, whose program isn't run).
fn code_char(f: &Dict, code: u8) -> char {
    match &*glyph_name(f, code) {
        ".notdef" => char::from(code),
        name => glyph_char(name),
    }
}

/// The glyph name font `f`'s `Encoding` gives code `code` (`.notdef` without one).
fn glyph_name(f: &Dict, code: u8) -> Rc<str> {
    f.get(&Key::name("Encoding")).and_then(|e| e.items()?.get(usize::from(code))).and_then(|o| o.text()).unwrap_or_else(|| Rc::from(".notdef"))
}

/// The character glyph name `name` (at code `code`) says: a single character or `uniXXXX`; a
/// name of the standard encoding's printable ASCII range (`hyphen`, `quotedbl`: PLRM 3rd ed.,
/// Appendix E.6); one of the Windows-1252 names; else `None`.
fn named_char(code: u8, name: &str) -> Option<char> {
    match glyph_char(name) {
        '\u{2022}' if name != "bullet" => {}
        c => return Some(c),
    }
    use skrifa::raw::ps::encoding::PredefinedEncoding;
    if let Some(c) = (0x20u8..0x7F).find(|c| PredefinedEncoding::Standard.name(*c) == name) {
        // The standard encoding has curly quotes where ASCII has the straight quote and the grave.
        return Some(match c {
            0x27 => '\u{2019}',
            0x60 => '\u{2018}',
            c => char::from(c),
        });
    }
    crate::latin::glyph(code, name)
}

/// A font scaled or transformed by `m` (`scalefont`, `makefont`).
fn transformed(font: &DictRef, m: Affine) -> DictRef {
    let mut d = font.borrow().clone();
    let fm = d.get(&Key::name("FontMatrix")).and_then(|o| o.items().and_then(|i| matrix_of(&i.borrow()))).unwrap_or(Affine::scale(0.001));
    d.insert(Key::name("FontMatrix"), matrix_obj(m * fm));
    DictRef::new(d)
}

/// The character a glyph name stands for (`A`, `space`, `uni20AC`), else a bullet.
pub(super) fn glyph_char(name: &str) -> char {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c;
    }
    if let Some(hex) = name.strip_prefix("uni").or_else(|| name.strip_prefix('u'))
        && hex.chars().all(|c| c.is_ascii_hexdigit())
        && let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
    {
        return c;
    }
    match name {
        "space" => ' ',
        _ => '\u{2022}',
    }
}

#[cfg(test)]
mod tests {
    use super::glyph_char;

    #[test]
    fn glyph_char_rejects_plus_in_hex() {
        assert_eq!(glyph_char("uni20AC"), '€');
        assert_eq!(glyph_char("u1F600"), '\u{1F600}');
        assert_eq!(glyph_char("uni+041"), '\u{2022}');
        assert_eq!(glyph_char("u+41"), '\u{2022}');
    }
}
