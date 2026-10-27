use super::exporter::{GlyphLine, MAX_INLINE_DEPTH, in_range, norm, to_path, writable, xf};
use super::images::embed;
use super::*;

impl Exporter<'_> {
    ///
    /// Inline graphics ([`vectorcraft_doc::TextRun::inline`]) are drawn as their symbols' art
    /// (vector paths, like symbol instances) where the layout placed them; their characters
    /// write no glyph.
    pub(super) fn text(&mut self, s: &mut Surface, n: &Node, t: &TextObject, page: Rect) {
        let doc = self.doc;
        let resolved = doc.inline_resolved(t);
        let t = &*resolved;
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let tb = t.xf.transform_rect_bbox(layout.bounds);
        // The object's own fills and strokes paint the whole outline: those below the Characters
        // row under the characters, the others over them.
        let (below, above) = n.appearance.split_contents();
        let all = (!n.appearance.items.is_empty()).then(|| {
            let mut all = layout.to_bezpath();
            for (_, bar) in vectorcraft_text::decorations(&layout, vectorcraft_text::FontDb::global(), t) {
                all.extend(bar.iter());
            }
            all.apply_affine(t.xf);
            to_path(s, &all).map(|p| (all, p))
        });
        let all = all.flatten();
        if let Some((bp, path)) = &all {
            self.text_items(s, below, bp, path, page, tb);
        }
        // Real text: the characters' fills as text in embedded fonts. The type's transform is
        // pushed for each fill and stroke of the characters, inside its form when the fill or
        // stroke overprints.
        let text_space = xf(t.xf);
        // Ruby can't be real text in one run (it isn't characters of the base text): the type's
        // characters go out as outlines where ruby is present, ruby included.
        let plain = (!self.outline_text && t.ruby.is_empty()).then(|| t.plain_text());
        let bars = vectorcraft_text::decorations(&layout, vectorcraft_text::FontDb::global(), t);
        for (i, run) in t.runs.iter().enumerate() {
            let mut bp = BezPath::new();
            for g in layout.glyphs.iter().filter(|g| g.run == i).chain(layout.ruby.iter().filter(|g| g.run == i)) {
                bp.extend(g.outline.iter());
            }
            // Underline and strikethrough bars are paths, with real text too (#847).
            let mut run_bars = BezPath::new();
            for (_, bar) in bars.iter().filter(|(r, _)| *r == i) {
                run_bars.extend(bar.iter());
            }
            bp.extend(run_bars.iter());
            let fill = &run.style.fill;
            // Colour bitmap glyphs (emoji) real text doesn't write: all of them, or those it leaves.
            let mut images: Option<Vec<usize>> = None;
            self.overprinting(s, run.style.overprint_fill, fill, |ex, s| {
                s.push_transform(&text_space);
                // The outlines left to fill as paths: all of them, or those real text leaves.
                let outlines = match plain.as_deref().filter(|_| !fill.is_none() && !ex.area_paint(fill, true)) {
                    Some(text) => {
                        let (mut left, unwritten) = ex.glyph_text(s, &layout, i, text, fill, true, Some(false));
                        images = Some(unwritten);
                        left.extend(run_bars.iter());
                        Cow::Owned(left)
                    }
                    None => Cow::Borrowed(&bp),
                };
                let opaque = (1.0, BlendMode::Normal);
                if let Some(path) = to_path(s, &outlines)
                    && !ex.area(s, &path, krilla::paint::FillRule::NonZero, fill, opaque, layout.bounds, outlines.bounding_box(), true)
                    && let Some(paint) = ex.paint(s, fill, layout.bounds)
                {
                    s.set_stroke(None);
                    s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
                    s.draw_path(&path);
                }
                s.pop();
            });
            // They paint themselves, whatever the fill.
            let images = images.unwrap_or_else(|| layout.glyphs.iter().enumerate().filter(|(_, g)| g.run == i).map(|(gi, _)| gi).collect());
            s.push_transform(&text_space);
            for g in images.iter().filter_map(|&gi| layout.glyphs.get(gi)) {
                self.glyph_image(s, g);
                // COLR glyphs: their layers, as art.
                if let Some(art) = g.placed_colour(Affine::IDENTITY) {
                    self.node(s, &art, page, true);
                }
            }
            s.pop();
            // Characters with neither fill nor stroke (a scanned page's recognized text), or whose
            // fill is drawn as art (a dithered gradient's image, a pattern: outlines, not text),
            // are still text: written invisible (rendering mode 3), so they can still be selected
            // and searched.
            let unpainted = fill.is_none() && !run.style.has_stroke();
            if let Some(text) = plain.as_deref().filter(|_| unpainted || self.area_paint(fill, true)) {
                self.invisible = true;
                crate::forms::form(s, Mark::Invisible, |s| {
                    s.push_transform(&text_space);
                    // The paint only lets the writer write the text; the glyphs it can't write
                    // as text draw nothing, and colour bitmap glyphs are drawn already.
                    self.glyph_text(s, &layout, i, text, &Paint::solid(Color::gray(0.0)), false, None);
                    s.pop();
                });
            } else if let Some(text) = plain.as_deref().filter(|_| layout.glyphs.iter().any(|g| g.run == i && g.bend.is_some())) {
                // Glyphs bent along a path were drawn as their outlines (text is set straight):
                // they are written again as invisible text, straight where they stand, so they can
                // still be selected, searched and copied.
                self.invisible = true;
                crate::forms::form(s, Mark::Invisible, |s| {
                    s.push_transform(&text_space);
                    self.glyph_text(s, &layout, i, text, &Paint::solid(Color::gray(0.0)), false, Some(true));
                    s.pop();
                });
            }
            s.push_transform(&text_space);
            let path = run.style.has_stroke().then(|| to_path(s, &bp)).flatten();
            s.pop();
            if let Some(path) = path {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                let st = run.style.stroke_layer();
                self.overprinting(s, st.overprint, &st.paint, |ex, s| {
                    s.push_transform(&text_space);
                    ex.stroke(s, &bp, &path, FillRule::NonZero, &st, page, layout.bounds);
                    s.pop();
                });
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
        if self.inline_depth < MAX_INLINE_DEPTH {
            self.inline_depth += 1;
            for ig in &layout.inlines {
                let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
                let Some(sym) = doc.symbols.iter().find(|x| x.name == art.symbol) else { continue };
                let mut node = (*sym.art).clone();
                // Strokes scale with the art, as on the canvas.
                node.transform(t.xf * ig.xf * doc.symbol_natural_xf(&art.symbol), true);
                self.node(s, &node, page, true);
            }
            self.inline_depth -= 1;
        }
        if let Some((bp, path)) = &all {
            self.text_items(s, above, bp, path, page, tb);
        }
    }

    /// Some of a type object's own fills and strokes (`items`) on its glyph outlines `bp` (`path`;
    /// `tb`: their bounds). Strokes take every stroke option, as on paths.
    #[allow(clippy::too_many_arguments)]
    fn text_items(&mut self, s: &mut Surface, items: &[AppearanceItem], bp: &BezPath, path: &Path, page: Rect, tb: Rect) {
        for item in items {
            match item {
                AppearanceItem::Fill(fl) if fl.visible => self.overprinting(s, fl.overprint, &fl.paint, |ex, s| {
                    if !ex.area(s, path, krilla::paint::FillRule::NonZero, &fl.paint, (fl.opacity, fl.blend), tb, bp.bounding_box(), true)
                        && let Some(paint) = ex.paint(s, &fl.paint, tb)
                    {
                        s.set_stroke(None);
                        s.set_fill(Some(Fill { paint, opacity: norm(fl.opacity), rule: krilla::paint::FillRule::NonZero }));
                        s.draw_path(path);
                    }
                }),
                AppearanceItem::Stroke(st) if st.visible && st.width > 0.0 => {
                    self.overprinting(s, st.overprint, &st.paint, |ex, s| ex.stroke(s, bp, path, FillRule::NonZero, st, page, tb))
                }
                _ => {}
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
    }

    /// Fill the glyphs of run `run` of `layout` with `fill` as text in their fonts, embedded (in
    /// text space; `text` is the plain text, whose characters the glyphs stand for, which makes
    /// the text selectable and searchable) → the outlines of the glyphs left to draw as paths:
    /// those of fonts that can't be embedded, and missing glyphs. Glyphs sharing a font, a frame
    /// (size, scale, rotation) and a baseline are written together. Colour glyphs (bitmap emoji,
    /// COLR, SVG), with `images`, are written as text too, which draws their images or layers; the
    /// indices of those it can't write or draw (SVG glyphs) are returned with the outlines. The
    /// hyphen the layout adds where it breaks a line inside a word ([`break_hyphen`]) is written
    /// as the character its glyph stands for, on its own after the glyphs before it. The glyphs of
    /// a list marker are written as the marker's characters ([`marker_chars`]), apart from the
    /// plain text's glyphs; a marker glyph that stands for none of them is drawn as a path.
    ///
    /// `bent`: `Some(false)` writes the glyphs bent along a path ([`vectorcraft_text::GlyphBend`])
    /// as outlines, not text (text is set straight); `Some(true)` writes only those, as text where
    /// they stand straight; `None` writes all of them as text.
    #[allow(clippy::too_many_arguments)]
    fn glyph_text(
        &mut self,
        s: &mut Surface,
        layout: &TextLayout,
        run: usize,
        text: &str,
        fill: &Paint,
        images: bool,
        bent: Option<bool>,
    ) -> (BezPath, Vec<usize>) {
        let (mut rest, mut unwritten) = (BezPath::new(), vec![]);
        let mut line: Option<GlyphLine> = None;
        // The characters the glyphs stand for: the plain text, then, when the run has glyphs that
        // stand for no character of it, the characters a line-break hyphen can stand for.
        let written: Cow<str> =
            if layout.glyphs.iter().any(|g| g.run == run && g.len == 0) { Cow::Owned(format!("{text}{BREAK_HYPHENS}")) } else { Cow::Borrowed(text) };
        let written_text: &str = &written;
        // The text a line's glyphs stand for: `written`, or its list marker's characters.
        let text_of = |l: &GlyphLine| l.marker.and_then(|m| layout.marker_of(m)).map_or(written_text, |m| m.text.as_str());
        // The characters of the list marker being written that each of its glyphs stands for.
        let mut marker: Option<(usize, Vec<Option<std::ops::Range<usize>>>)> = None;
        for (gi, g) in layout.glyphs.iter().enumerate().filter(|(_, g)| g.run == run) {
            let coloured = g.image.is_some() || g.colour.is_some();
            if coloured && !images {
                continue;
            }
            match (bent, g.bend.is_some()) {
                (Some(false), true) => {
                    rest.extend(g.outline.iter());
                    continue;
                }
                (Some(true), false) => continue,
                _ => {}
            }
            // Of the glyphs with no length, only a list marker's and line-break hyphens are written
            // as text; the others, such as kashidas, are drawn as paths. A list marker's glyphs are
            // not line-break hyphens, whatever the marker.
            let in_marker = (g.len == 0).then(|| layout.marker_of(gi)).flatten();
            let hyphen = if in_marker.is_some() { None } else { break_hyphen(g) };
            let (range, ch) = match in_marker {
                Some(m) => {
                    if marker.as_ref().is_none_or(|(start, _)| *start != m.glyphs.start) {
                        marker = Some((m.glyphs.start, marker_chars(layout, m)));
                    }
                    let range = marker.as_ref().and_then(|(start, chars)| chars.get(gi.checked_sub(*start)?).cloned().flatten());
                    let ch = range.clone().and_then(|r| m.text.get(r)).and_then(|t| t.chars().next());
                    (range.unwrap_or(0..0), ch)
                }
                None => {
                    let range = hyphen.clone().map_or(g.byte..g.byte + g.len, |h| text.len() + h.start..text.len() + h.end);
                    let ch = written.get(range.clone()).and_then(|t| t.chars().next());
                    (range, ch)
                }
            };
            let marker_at = in_marker.map(|m| m.glyphs.start);
            // Spaces are written too (text extraction needs them); control characters and soft
            // hyphens draw nothing.
            let blank = g.outline.elements().is_empty() && !coloured;
            if blank && !ch.is_some_and(|c| c.is_whitespace() && !c.is_control()) {
                continue;
            }
            // The glyph at the font size of its vertical scale: `frame` maps that size's glyph
            // space (y down, krilla's) to text space.
            let [a, b, c, d, ..] = g.xf.as_coeffs();
            let (sx, sy) = (a.hypot(b), c.hypot(d));
            let font = if g.gid != 0 && ch.is_some() { self.font(g.font_id) } else { None };
            let det = g.xf.determinant();
            let Some((font, upem)) = font.filter(|(_, upem)| det.is_finite() && det.abs() > 1e-9 && (1e-3..=1e5).contains(&(upem * sy))) else {
                rest.extend(g.outline.iter());
                if coloured {
                    unwritten.push(gi);
                }
                continue;
            };
            // SVG glyphs are written as text that draws nothing (see the serialize settings): their
            // art is drawn, or else their outline.
            if is_svg_glyph(g) {
                if coloured {
                    unwritten.push(gi);
                } else {
                    rest.extend(g.outline.iter());
                }
            }
            let size = upem * sy;
            let frame = g.xf * Affine::scale(1.0 / sy);
            // Its pen position along the frame's baseline, when it continues the line (a
            // line-break hyphen never does).
            let x = line
                .as_ref()
                .filter(|l| hyphen.is_none() && l.face == g.font_id && (l.size - size).abs() < 1e-6 && l.marker == marker_at)
                .and_then(|l| {
                    let [a, b, c, d, e, f] = (l.frame.inverse() * frame).as_coeffs();
                    let same = (a - 1.0).abs() < 1e-6 && b.abs() < 1e-6 && c.abs() < 1e-6 && (d - 1.0).abs() < 1e-6 && f.abs() < 1e-4;
                    same.then_some(e)
                });
            let x = match x {
                Some(x) => x,
                None => {
                    if let Some(l) = line.take() {
                        let text = text_of(&l);
                        self.draw_glyph_line(s, l, text, fill, layout.bounds);
                    }
                    line = Some(GlyphLine { face: g.font_id, font, size, frame, glyphs: vec![], marker: marker_at });
                    0.0
                }
            };
            // The glyph's advance in frame units (the last one's ends the line).
            let advance = g.advance * sy / sx;
            if let Some(l) = line.as_mut() {
                l.glyphs.push((g.gid, x, advance, range));
            }
            // A line-break hyphen is written on its own.
            if hyphen.is_some()
                && let Some(l) = line.take()
            {
                self.draw_glyph_line(s, l, &written, fill, layout.bounds);
            }
        }
        if let Some(l) = line {
            let text = text_of(&l);
            self.draw_glyph_line(s, l, text, fill, layout.bounds);
        }
        (rest, unwritten)
    }

    /// Draw colour bitmap glyph `g`'s image (on `s` in text space), its PNG written once.
    fn glyph_image(&mut self, s: &mut Surface, g: &vectorcraft_text::PositionedGlyph) {
        let Some((image, to_text)) = g.placed_image() else { return };
        if !writable(s, to_text, image.rect()) {
            return;
        }
        let cache = format!("\u{0}glyph\u{0}{}", image.blob.content_key());
        let img = match self.images.get(&cache) {
            Some(img) => img.clone(),
            None => {
                let img = embed(&image.blob, self.interpolate, self.sixteen_bit);
                self.images.insert(cache, img.clone());
                img
            }
        };
        let (Some(img), Some(size)) = (img, Size::from_wh(image.width as f32, image.height as f32)) else { return };
        s.push_transform(&xf(to_text));
        s.draw_image(img, size);
        s.pop();
    }

    /// Draw glyphs `line` (of `text`), filled with `fill` (whose space is text space, where
    /// `bounds` is). A line that would be written with a pen position, size or spacing (in
    /// thousandths of an em) beyond [`MAX_COORD`] is left out.
    fn draw_glyph_line(&mut self, s: &mut Surface, line: GlyphLine, text: &str, fill: &Paint, bounds: Rect) {
        let size = line.size;
        let glyphs: Vec<KrillaGlyph> = line
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, (gid, x, advance, range))| {
                let next = line.glyphs.get(i + 1).map_or(x + advance, |n| n.1);
                KrillaGlyph::new(GlyphId::new(*gid), ((next - x) / size) as f32, 0.0, 0.0, 0.0, range.clone(), None)
            })
            .collect();
        let pens =
            line.glyphs.iter().fold(Rect::new(0.0, -size, 0.0, size), |r, (_, x, advance, _)| r.union_pt((*x, 0.0)).union_pt((x + advance, 0.0)));
        let spacing: Vec<f64> = glyphs.iter().map(|g| f64::from(g.x_advance(1.0)) * 1000.0).collect();
        if !(writable(s, line.frame, pens) && in_range(&spacing)) {
            return;
        }
        s.push_transform(&xf(line.frame));
        if let Some(paint) = self.paint_in(s, fill, bounds, line.frame.inverse()) {
            s.set_stroke(None);
            s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
            s.draw_glyphs(krilla::geom::Point::from_xy(0.0, 0.0), &glyphs, line.font, text, size as f32, false);
        }
        s.pop();
    }

    /// The font of face `id` for real text and its units per em; `None` (with a warning) when its
    /// licence doesn't allow embedding it as a subset, or it can't be read.
    fn font(&mut self, id: u32) -> Option<(krilla::text::Font, f64)> {
        if let Some(f) = self.fonts.get(&id) {
            return f.clone();
        }
        let Some(face) = vectorcraft_text::FontDb::global().face_by_id(id) else {
            self.fonts.insert(id, None);
            return None;
        };
        let name = format!("{} {}", face.family, face.style);
        let font = match face.embedding() {
            Embedding::Subset => {
                // A named instance of a variable font is embedded instanced (its own outlines and
                // widths, not the default instance's).
                match write::font(face.file_data(), face.face_index(), Some(face.variations())) {
                    Ok(font) => Some((font, face.units_per_em())),
                    Err(error) => {
                        self.warn(format!("the font “{name}” can't be embedded ({error}): its text is exported as outlines"));
                        None
                    }
                }
            }
            Embedding::Whole => {
                self.warn(format!(
                    "the font “{name}” may only be embedded whole, and fonts are embedded as subsets: its text is exported as outlines"
                ));
                None
            }
            Embedding::Forbidden => {
                self.warn(format!("the licence of the font “{name}” doesn't allow embedding: its text is exported as outlines"));
                None
            }
        };
        self.fonts.insert(id, font.clone());
        font
    }
}

/// Is `g` drawn from its font's SVG document, which the PDF writer can't draw as a font's glyph?
fn is_svg_glyph(g: &vectorcraft_text::PositionedGlyph) -> bool {
    vectorcraft_text::FontDb::global().is_svg_glyph(g.font_id, g.gid)
}

/// The characters a line-break hyphen can stand for, as real text: '-' and U+2010 (HYPHEN).
const BREAK_HYPHENS: &str = "-\u{2010}";

/// The bytes of [`BREAK_HYPHENS`] that glyph `g` stands for when it is a hyphen the layout adds
/// where it breaks a line inside a word: a glyph that stands for no character of the text (length
/// 0) and that its face's character map gives to '-' or, failing that, to U+2010. `None` for any
/// other glyph, kashida tatweels included.
fn break_hyphen(g: &vectorcraft_text::PositionedGlyph) -> Option<std::ops::Range<usize>> {
    if g.len != 0 || g.gid == 0 {
        return None;
    }
    let face = vectorcraft_text::FontDb::global().face_by_id(g.font_id)?;
    BREAK_HYPHENS.char_indices().find(|&(_, c)| face.glyph_for(c) == g.gid).map(|(i, c)| i..i + c.len_utf8())
}

/// The characters of list marker `m` of `layout` that each of its glyphs stands for, as bytes of
/// the marker's text, in the order of [`ListMarker::glyphs`]: `None` for its tab and for a glyph
/// that stands for none of them. A glyph stands for the first character left that the glyph's
/// font maps to that glyph in its character map. When the glyphs that no character maps to (an
/// OpenType feature such as oldstyle figures replaced them) are as many as the characters left,
/// they stand for those characters, in order.
fn marker_chars(layout: &TextLayout, m: &ListMarker) -> Vec<Option<std::ops::Range<usize>>> {
    let db = vectorcraft_text::FontDb::global();
    let mut left: Vec<(std::ops::Range<usize>, char)> = m.text.char_indices().map(|(i, c)| (i..i + c.len_utf8(), c)).collect();
    let mut chars = vec![None; m.glyphs.len()];
    let mut unmapped = vec![];
    for (k, gi) in m.glyphs.clone().enumerate() {
        let Some(g) = layout.glyphs.get(gi).filter(|g| gi != m.tab && g.gid != 0) else { continue };
        let face = db.face_by_id(g.font_id);
        match (left.iter().position(|(_, c)| face.as_ref().is_some_and(|f| f.glyph_for(*c) == g.gid)), chars.get_mut(k)) {
            (Some(at), Some(slot)) => *slot = Some(left.remove(at).0),
            _ => unmapped.push(k),
        }
    }
    if unmapped.len() == left.len() {
        for (k, (range, _)) in unmapped.into_iter().zip(left) {
            if let Some(slot) = chars.get_mut(k) {
                *slot = Some(range);
            }
        }
    }
    chars
}
