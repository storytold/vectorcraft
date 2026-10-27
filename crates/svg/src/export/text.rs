use super::output::xml_text;
use super::*;

/// `chars` (sorted) as a CSS `unicode-range` (`U+41-5A,U+61`).
fn unicode_range(chars: &BTreeSet<char>) -> String {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for c in chars.iter().map(|c| u32::from(*c)) {
        match runs.last_mut() {
            Some((_, end)) if c == *end + 1 => *end = c,
            _ => runs.push((c, c)),
        }
    }
    let one = |(a, b): (u32, u32)| if a == b { format!("U+{a:X}") } else { format!("U+{a:X}-{b:X}") };
    runs.into_iter().map(one).collect::<Vec<_>>().join(",")
}

/// The characters type uses from one face, under one `@font-face` description: the family the
/// type names and the weight, italic and width its style asks for.
pub(super) struct FontUse {
    family: String,
    weight: u16,
    italic: bool,
    /// The `font-stretch` percentage ([`css::font_stretch`]), as bits; `None` for the normal width.
    stretch: Option<u64>,
    face: Arc<FontFace>,
    chars: BTreeSet<char>,
}

impl Writer<'_> {
    /// The `<text>`/`<tspan>` properties of a character style. `space`: the text's layout bounds
    /// and the map from text space to user space, which character gradients resolve against
    /// (as on the canvas).
    fn char_props(&mut self, st: &CharStyle, space: (Rect, Affine)) -> Props {
        let decimals = self.opts.decimals;
        let len = |v: f64| fmt_num(v, decimals);
        let mut p = css::font_props(st, &len);
        // Character paints resolve in the text's space (`to_user` maps it into user space).
        let (bounds, to_user) = space;
        let saved = std::mem::replace(&mut self.xf, to_user);
        if let Paint::Gradient(g) = &st.fill
            && g.gradient.dither
            && g.gradient.kind != GradientKind::Freeform
        {
            self.warn("dithered gradients on characters are written without dither");
        }
        let fill = self.paint(&st.fill, Some(bounds));
        p.push(("fill", fill));
        // An inside or outside stroke is drawn on the glyph outlines instead
        // ([`Self::aligned_char_strokes`]): SVG strokes text centred only.
        if st.has_stroke() && !aligned(st) {
            // Weight, cap, join, miter limit and dashes as for object strokes; a gradient spans
            // the text grown by half the weight, as on the canvas.
            let layer = st.stroke_layer();
            self.stroke_props(&layer, layer.width, Some(layer.paint_bounds(bounds)), &mut p);
        }
        self.xf = saved;
        p.extend(css::type_props(st, &len));
        p
    }

    /// Type: its characters (live text, or glyph outlines when text is exported as outlines) and,
    /// as the canvas paints them, the object's own fills and strokes on the glyph outlines: those
    /// below the Characters row under the characters, the others over them.
    ///
    /// Inline graphics ([`vectorcraft_doc::TextRun::inline`]) are written after the characters as
    /// instances of their symbols (a `<use>` of the symbol's def where it can be shared), all in
    /// one group carrying the object's transparency. The live text skips their characters and
    /// places the text after each one; type on a path with inline graphics is written as outlines
    /// (a `<textPath>` couldn't leave room for the art).
    pub(super) fn text_node(&mut self, n: &Node, t: &TextObject) {
        let doc = self.doc;
        let resolved = doc.inline_resolved(t);
        let t = &*resolved;
        let inline = t.runs.iter().any(|r| r.inline.is_some());
        // Ruby is written as outlines with its base text: a reader's <text> can't place it. So are
        // small capitals, superscript and subscript and tab leaders, which SVG text doesn't draw (#3869).
        let unwritable = t.runs.iter().any(|r| r.style.small_caps.is_some() || r.style.position != vectorcraft_doc::CharPosition::Normal)
            || std::iter::once(&t.para).chain(&t.paras).any(|p| p.tabs.iter().any(|tab| !tab.leader.is_empty()));
        let outlined =
            self.opts.outline_text || t.vertical || !t.ruby.is_empty() || (inline && matches!(t.kind, TextKind::OnPath { .. })) || unwritable;
        let chars = |w: &mut Self, n: &Node| if outlined { w.text_outlines(n, t) } else { w.text(n, t) };
        let painted = n.appearance.items.iter().any(|i| i.visible() && !i.paint().is_none());
        // Live type with an inside or outside character stroke writes that stroke apart from the
        // text, in the group.
        let apart = !outlined && t.runs.iter().any(|r| aligned(&r.style));
        if !painted && !inline && !apart {
            return chars(self, n);
        }
        let id = self.id_attr(n);
        let a = self.attrs(&css::transparency(n));
        self.line(&format!("<g{id}{a}>"));
        self.depth += 1;
        let db = vectorcraft_text::FontDb::global();
        let lay = vectorcraft_text::layout(db, t);
        let mut all = lay.to_bezpath();
        for (_, bar) in vectorcraft_text::decorations(&lay, db, t) {
            all.extend(bar.iter());
        }
        let pd = PathData::from_bezpath(&all).transformed(t.xf);
        let d = self.path_d(&pd, self.xf);
        let tb = Some(t.xf.transform_rect_bbox(lay.bounds));
        let (below, above) = n.appearance.split_contents();
        let glyphs = |w: &mut Self, items: &[AppearanceItem]| {
            if !items.is_empty() {
                let ap = vectorcraft_doc::Appearance { items: items.to_vec(), ..Default::default() };
                w.shape(&Node::path(NodeId(u64::MAX), pd.clone(), ap), &d, &[&pd], FillRule::NonZero, tb);
            }
        };
        glyphs(self, below);
        // The group carries the id and the object's transparency.
        let mut bare = n.clone();
        (bare.opacity, bare.blend, bare.isolate) = (1.0, BlendMode::Normal, false);
        let anonymous = std::mem::replace(&mut self.anonymous, true);
        chars(self, &bare);
        for ig in &lay.inlines {
            let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
            let inst = Node::new(
                NodeId(u64::MAX),
                NodeKind::SymbolInstance { symbol: art.symbol.clone(), xf: t.xf * ig.xf * doc.symbol_natural_xf(&art.symbol) },
            );
            self.node(&inst);
        }
        glyphs(self, above);
        self.anonymous = anonymous;
        self.depth -= 1;
        self.line("</g>");
    }

    /// Text as glyph outlines: one compound path per run, painted like the run (gradients span
    /// the whole text), over its colour glyphs, as on the canvas.
    fn text_outlines(&mut self, n: &Node, t: &TextObject) {
        let db = vectorcraft_text::FontDb::global();
        let lay = vectorcraft_text::layout(db, t);
        let mut runs = vec![kurbo::BezPath::new(); t.runs.len()];
        // Underline and strikethrough bars join their run's outlines; ruby glyphs are painted
        // with the run of the text they annotate.
        let bars = vectorcraft_text::decorations(&lay, db, t);
        for (run, outline) in lay
            .glyphs
            .iter()
            .map(|g| (g.run, &g.outline))
            .chain(lay.ruby.iter().map(|g| (g.run, &g.outline)))
            .chain(bars.iter().map(|(r, b)| (*r, b)))
        {
            if let Some(bp) = runs.get_mut(run) {
                bp.extend(outline.iter());
            }
        }
        let id = self.id_attr(n);
        let a = self.attrs(&css::transparency(n));
        // Glyphs bent along a path (joined scripts on a curve) can only be outlines: the group
        // names its text for screen readers and search.
        let label = if lay.glyphs.iter().any(|g| g.bend.is_some()) {
            format!(" role=\"img\" aria-label=\"{}\"", xml_escape(&t.plain_text()))
        } else {
            String::new()
        };
        self.line(&format!("<g{id}{a}{label}>"));
        self.depth += 1;
        // The glyphs stay in text space; the text's transform joins the document's meanwhile.
        let saved = self.xf;
        self.xf = saved * t.xf;
        for (i, (bp, run)) in runs.iter().zip(&t.runs).enumerate() {
            // Colour glyphs paint themselves, under their run's paint: bitmap glyphs (emoji) as
            // their images, COLR and SVG glyphs as their art.
            for g in lay.glyphs.iter().filter(|g| g.run == i) {
                if let Some((image, to_text)) = g.placed_image() {
                    let m = self.matrix(self.xf * to_text);
                    let href = self.blob_href(&image.blob);
                    self.line(&format!(
                        "<image width=\"{}\" height=\"{}\" transform=\"{m}\" preserveAspectRatio=\"none\" xlink:href=\"{}\"/>",
                        image.width,
                        image.height,
                        xml_escape(&href)
                    ));
                }
                if let Some(art) = g.placed_colour(Affine::IDENTITY) {
                    self.node(&art);
                }
            }
            if bp.elements().is_empty() {
                continue;
            }
            let pd = PathData::from_bezpath(bp);
            // An unnamed id: the pieces carry no id attribute (the group has it).
            let glyphs = Node::path(NodeId(u64::MAX), pd.clone(), run.style.appearance());
            let d = self.path_d(&pd, self.xf);
            self.shape(&glyphs, &d, &[&pd], FillRule::NonZero, Some(lay.bounds));
        }
        self.xf = saved;
        self.depth -= 1;
        self.line("</g>");
    }

    /// Point and area type, every laid-out line at its own position: wraps, indents, tabs and
    /// spacing come out as on the canvas. Centred and right-aligned lines are anchored
    /// (`text-anchor`) at their centre or right end, so they stay aligned in a viewer whose font
    /// differs; other lines are placed where each style run (and, justified, each word) starts.
    fn text(&mut self, n: &Node, t: &TextObject) {
        // Live SVG text is written in logical order from left-aligned pieces: bidirectional text
        // (and right-to-left paragraphs) keep their look as outlines until it is written with
        // `direction`/`unicode-bidi`. So do runs set Right to Left (Character Direction), whose
        // `direction="rtl"` would anchor their pieces at the right end.
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let rtl_runs = t.runs.iter().any(|r| r.style.char_direction == Some(vectorcraft_doc::ParaDirection::RightToLeft));
        if lay.glyphs.iter().any(|g| g.rtl) || lay.lines.iter().any(|l| l.rtl) || rtl_runs {
            self.warn("bidirectional text is outlined to preserve shaping and visual order");
            return self.text_outlines(n, t);
        }
        if let TextKind::OnPath { path, start, end } = &t.kind {
            // A reader lays a `<textPath>`'s glyphs straight: joined glyphs bent along the path
            // keep their look as outlines.
            if lay.glyphs.iter().any(|g| g.bend.is_some()) {
                self.warn("joined type bent along a path is outlined to keep its look");
                return self.text_outlines(n, t);
            }
            // `<textPath>` has a start offset only: an end bracket, Align to Path and Spacing
            // keep their look as outlines.
            if end.is_some() || t.path_align != vectorcraft_doc::PathAlign::Baseline || t.path_spacing != 0.0 {
                self.warn("type on a path with an end bracket, Align to Path or Spacing is outlined to keep its look");
                return self.text_outlines(n, t);
            }
            // It cannot hang glyphs outside the start bracket either.
            if hangs_on_path(t, &lay) {
                self.warn("type on a path with glyphs hanging outside its brackets is outlined to keep its look");
                return self.text_outlines(n, t);
            }
            // A `<textPath>` holds the characters of the text, not the bullets or numbers drawn
            // before them.
            if !lay.markers.is_empty() {
                self.warn("type on a path with bullets or numbers is outlined to keep its look");
                return self.text_outlines(n, t);
            }
            return self.text_on_path(n, t, path, *start);
        }
        self.note_fonts(t, &lay);
        let lines = text_lines(t, &lay, self.opts.fewer_tspans);
        // `drawn` holds the styles the layout draws the text in: Shrink Text to Fit scales its sizes, explicit leading and baseline shifts.
        let fitted = (lay.fit_scale != 1.0).then(|| vectorcraft_text::scaled(t, lay.fit_scale));
        let drawn = fitted.as_ref().unwrap_or(t);
        // Each line is anchored as its paragraph is aligned. A tab starts a new chunk at its
        // stop, and so does the text after an inline graphic: lines with tabs, and text with
        // inline graphics, are never anchored.
        let has_inline = t.runs.iter().any(|r| r.inline.is_some());
        let anchors: Vec<Option<(&str, f64)>> = lines
            .iter()
            .map(|l| {
                match t.para_at(l.para).justify {
                    Justify::Center => Some(("middle", 0.5)),
                    Justify::Right => Some(("end", 1.0)),
                    _ => None,
                }
                .filter(|_| !has_inline && !l.segs.iter().any(|s| s.brk))
            })
            .collect();
        // Every line anchored alike (the text's only paragraph style, typically): the <text>
        // carries the anchor. Otherwise each anchored line is a <tspan> carrying its own.
        let uniform = anchors.first().copied().filter(|a| a.is_some() && anchors.iter().all(|b| b == a)).flatten();
        // The <text> element's user space is text space.
        let space = (lay.bounds, Affine::IDENTITY);
        let mut base_props = self.char_props(&drawn.first_style(), space);
        // A `<tspan>` cannot undo the decoration of the `<text>` around it: with bullets, which
        // have none, each run's `<tspan>` carries its own.
        if !lay.markers.is_empty() {
            base_props.retain(|(k, _)| *k != "text-decoration");
        }
        let base = TextBase { props: base_props, space, anchored: uniform.is_some() };
        let mut props = base.props.clone();
        if let Some((a, _)) = uniform {
            props.push(("text-anchor", a.into()));
        }
        props.extend(css::transparency(n));
        let id = self.id_attr(n);
        let a = self.attrs(&props);
        let m = self.xf * t.xf;
        let tr = if m == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(m)) };
        let starts: Vec<Option<(f64, f64)>> = lines.iter().zip(&anchors).map(|(l, a)| line_start(&l.segs, a.map(|a| a.1))).collect();
        // The `<text>` carries the first line's position too, so readers that place text by its
        // own x/y start where the first line does.
        let at = match starts.iter().flatten().next() {
            Some((x, y)) => format!(" x=\"{}\" y=\"{}\"", self.num(*x), self.num(*y)),
            None => String::new(),
        };
        let mut s = format!("<text{id}{tr}{at} xml:space=\"preserve\"{a}>");
        for ((line, start), anchor) in lines.iter().zip(starts).zip(&anchors) {
            let Some(start) = start else { continue };
            match anchor.filter(|_| uniform.is_none()) {
                // A line anchored on its own: one positioned <tspan> with the anchor around it.
                Some((a, _)) => {
                    s.push_str(&format!("<tspan x=\"{}\" y=\"{}\" text-anchor=\"{a}\">", self.num(start.0), self.num(start.1)));
                    let inner = TextBase { anchored: true, ..base.clone() };
                    self.text_line(&mut s, drawn, &line.segs, start, &inner, false);
                    s.push_str("</tspan>");
                }
                None => self.text_line(&mut s, drawn, &line.segs, start, &base, true),
            }
        }
        s.push_str("</text>");
        self.line(&s);
        self.aligned_char_strokes(t);
    }

    /// The inside and outside character strokes of live type `t`, over its text (which goes
    /// without them, see [`Self::char_props`]): each such run's stroke on its glyph outlines, drawn
    /// twice as wide and clipped or masked to its side as aligned path strokes are
    /// ([`Self::side_attr`]). The text stays live; only the stroke follows the outlines.
    fn aligned_char_strokes(&mut self, t: &TextObject) {
        if !t.runs.iter().any(|r| aligned(&r.style)) {
            return;
        }
        let db = vectorcraft_text::FontDb::global();
        let lay = vectorcraft_text::layout(db, t);
        let bars = vectorcraft_text::decorations(&lay, db, t);
        // The glyphs stay in text space; the text's transform joins the document's meanwhile.
        let saved = self.xf;
        self.xf = saved * t.xf;
        for (i, run) in t.runs.iter().enumerate().filter(|(_, r)| aligned(&r.style)) {
            let mut bp = BezPath::new();
            for g in lay.glyphs.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
            }
            for (_, bar) in bars.iter().filter(|(r, _)| *r == i) {
                bp.extend(bar.iter());
            }
            if bp.elements().is_empty() {
                continue;
            }
            let pd = PathData::from_bezpath(&bp);
            let ap = vectorcraft_doc::Appearance { items: vec![AppearanceItem::Stroke(run.style.stroke_layer())], ..Default::default() };
            let d = self.path_d(&pd, self.xf);
            self.shape(&Node::path(NodeId(u64::MAX), pd.clone(), ap), &d, &[&pd], FillRule::NonZero, Some(lay.bounds));
        }
        self.xf = saved;
    }

    /// One line of laid-out text starting at `start`. Default: a positioned `<tspan>` per segment
    /// (only the first of an anchored line). Fewer tspans: one positioned `<tspan>` per line with
    /// the style changes nested in it, positioned again only after tabs and inline graphics.
    /// Baseline shifts are relative (`dy`), undone by the next segment that isn't shifted.
    /// `positioned`: false when the caller already positioned the line (an outer `<tspan>`).
    fn text_line(&mut self, s: &mut String, t: &TextObject, line: &[Segment], start: (f64, f64), base: &TextBase, positioned: bool) {
        let fewer = self.opts.fewer_tspans && positioned;
        if fewer {
            s.push_str(&format!("<tspan x=\"{}\" y=\"{}\">", self.num(start.0), self.num(start.1)));
        }
        let mut place = !fewer && positioned;
        let mut first = true;
        // The baseline shift the current text position carries.
        let mut shift = 0.0;
        for seg in line {
            let Some(run_style) = t.runs.get(seg.run).map(|r| &r.style) else { continue };
            let marker_style;
            let st = if seg.marker {
                marker_style = vectorcraft_doc::text::ParaList::marker_style(run_style);
                &marker_style
            } else {
                run_style
            };
            let mut attrs = String::new();
            if place {
                let (x, y) = if first { start } else { (seg.x, seg.y) };
                attrs.push_str(&format!(" x=\"{}\" y=\"{}\"", self.num(x), self.num(y)));
                shift = 0.0;
            }
            first = false;
            if st.baseline_shift != shift {
                attrs.push_str(&format!(" dy=\"{}\"", self.num(shift - st.baseline_shift)));
                shift = st.baseline_shift;
            }
            // After a tab, an inline graphic or a justified word space, and before a character that a
            // reader would draw elsewhere, the next segment is placed.
            place = seg.brk || (!fewer && !base.anchored && positioned);
            if (st.h_scale - st.v_scale).abs() > 1e-9 || seg.scale != 1.0 {
                attrs.push_str(&format!(" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\"", self.num(seg.advance * seg.scale)));
            }
            if st.rotation != 0.0 {
                attrs.push_str(&format!(" rotate=\"{}\"", self.num(-st.rotation)));
            }
            let mut diff = run_diff(&base.props, self.char_props(st, base.space));
            diff.extend(direction_props(st));
            attrs.push_str(&self.attrs(&diff));
            let text = xml_text(&seg.text);
            if attrs.is_empty() && fewer {
                s.push_str(&text);
            } else {
                s.push_str(&format!("<tspan{attrs}>{text}</tspan>"));
            }
        }
        if fewer {
            s.push_str("</tspan>");
        }
    }

    /// Record the characters of laid-out type `t` under the faces that supply them, for
    /// [`Self::font_faces`] (when fonts are embedded).
    fn note_fonts(&mut self, t: &TextObject, lay: &vectorcraft_text::TextLayout) {
        if !self.opts.embed_fonts || self.opts.outline_text {
            return;
        }
        if self.tiny() {
            return self.warn("SVG Tiny 1.2 has no web fonts: fonts are not embedded");
        }
        let db = FontDb::global();
        let plain = t.plain_text();
        let mut face: Option<Arc<FontFace>> = None;
        for (i, g) in lay.glyphs.iter().enumerate() {
            let Some(st) = t.runs.get(g.run).map(|r| &r.style) else { continue };
            // A colour font's subset would hold no bitmaps or COLR layers: its emoji are left to the
            // viewer.
            if g.image.is_some() || g.colour.is_some() {
                self.warn("colour emoji fonts are not embedded: viewers show emoji in their own fonts");
                continue;
            }
            if face.as_ref().is_none_or(|f| f.id() != g.font_id) {
                face = db.face_by_id(g.font_id);
            }
            let Some(f) = &face else { continue };
            let (weight, italic) = font_descriptor(st);
            let stretch = css::font_stretch(st).map(f64::to_bits);
            let at = self
                .fonts
                .iter()
                .position(|u| u.face.id() == f.id() && (u.weight, u.italic, u.stretch) == (weight, italic, stretch) && u.family == st.font_family);
            let at = at.unwrap_or_else(|| {
                let family = st.font_family.clone();
                self.fonts.push(FontUse { family, weight, italic, stretch, face: f.clone(), chars: BTreeSet::new() });
                self.fonts.len() - 1
            });
            let Some(chars) = self.fonts.get_mut(at).map(|u| &mut u.chars) else { continue };
            // As written: a list marker's characters, a hyphen where a line breaks, capitals for All
            // Caps, the style's Digits.
            let src = lay.marker_text(i).unwrap_or_else(|| if g.len == 0 { "-" } else { plain.get(g.byte..g.byte + g.len).unwrap_or("") });
            for c in src.chars().filter(|c| !c.is_control() && *c != '\u{ad}').map(|c| st.digits.map(c)) {
                if st.all_caps {
                    chars.extend(c.to_uppercase());
                } else {
                    chars.insert(c);
                }
            }
        }
    }

    /// The `@font-face` rules of the faces type uses ([`ExportOptions::embed_fonts`]): each face
    /// subset to its characters (whole when its licence forbids subsetting, left out with a
    /// warning when it forbids embedding). Faces sharing a description cover their own
    /// characters (`unicode-range`); where two of them share characters as well, a warning says
    /// that browsers draw those in one of the faces.
    pub(super) fn font_faces(&mut self) -> Vec<String> {
        let uses = std::mem::take(&mut self.fonts);
        let mut rules = Vec::new();
        fn described(u: &FontUse) -> (&str, u16, bool, Option<u64>) {
            (&u.family, u.weight, u.italic, u.stretch)
        }
        let mut clashing: Vec<&String> = vec![];
        for (i, u) in uses.iter().enumerate() {
            let clash = uses
                .get(..i)
                .unwrap_or_default()
                .iter()
                .any(|o| described(o) == described(u) && o.face.id() != u.face.id() && !o.chars.is_disjoint(&u.chars));
            if clash && !clashing.contains(&&u.family) {
                clashing.push(&u.family);
            }
        }
        for family in clashing {
            self.warn(&format!(
                "type in {family} uses faces of one weight, style and width that share characters: browsers draw those characters in one of the embedded faces"
            ));
        }
        for u in &uses {
            let chars: Vec<char> = u.chars.iter().copied().collect();
            let name = format!("{} {}", u.face.family, u.face.style);
            let Some(font) = u.face.embed(&chars) else {
                self.warn(&format!("the licence of the font {name} doesn't allow embedding: viewers show its type in their own fonts"));
                continue;
            };
            if !font.subset {
                self.warn(&format!("the font {name} is embedded whole: its licence doesn't allow subsetting"));
            }
            let shared = uses.iter().filter(|o| described(o) == described(u)).count() > 1;
            let range = if shared { format!(";unicode-range:{}", unicode_range(&u.chars)) } else { String::new() };
            let stretch = u.stretch.map(|w| format!(";font-stretch:{}%", fmt_num(f64::from_bits(w), 3))).unwrap_or_default();
            let (mime, format) = if font.cff { ("font/otf", "opentype") } else { ("font/ttf", "truetype") };
            rules.push(format!(
                "@font-face{{font-family:{};font-weight:{};font-style:{}{stretch};src:url(data:{mime};base64,{}) format(\"{format}\"){range}}}",
                css_string(&u.family),
                u.weight,
                if u.italic { "italic" } else { "normal" },
                base64_encode(&font.data)
            ));
        }
        rules
    }

    /// Type on a path: a `<textPath>` along the path (a def).
    fn text_on_path(&mut self, n: &Node, t: &TextObject, path: &PathData, start: f64) {
        let id = self.id_attr(n);
        let pid = self.fresh_id("text-path");
        let d = self.path_d(path, self.xf * t.xf);
        self.def(1, &format!("<path id=\"{pid}\" d=\"{d}\"/>"));
        // The <text> element's user space is the document's; gradients span the laid-out text.
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        self.note_fonts(t, &lay);
        let space = (lay.bounds, self.xf * t.xf);
        let base = self.char_props(&t.first_style(), space);
        let mut props = base.clone();
        match t.para_at(0).justify {
            Justify::Center | Justify::JustifyCenter => props.push(("text-anchor", "middle".into())),
            Justify::Right | Justify::JustifyRight => props.push(("text-anchor", "end".into())),
            _ => {}
        }
        props.extend(css::transparency(n));
        let a = self.attrs(&props);
        let offset = fmt_num(start * 100.0, 3);
        let mut s = format!("<text{id} xml:space=\"preserve\"{a}><textPath xlink:href=\"#{pid}\" startOffset=\"{offset}%\">");
        for run in &t.runs {
            let mut diff = run_diff(&base, self.char_props(&run.style, space));
            diff.extend(direction_props(&run.style));
            let a = self.attrs(&diff);
            s.push_str(&format!("<tspan{a}>{}</tspan>", xml_text(&run.text.replace('\n', " "))));
        }
        s.push_str("</textPath></text>");
        self.line(&s);
        self.aligned_char_strokes(t);
    }
}

/// Does a character style stroke inside or outside its glyphs (not centred)?
fn aligned(st: &CharStyle) -> bool {
    st.has_stroke() && st.stroke_align != StrokeAlign::Center
}

/// Does type on a path `t`, laid out as `lay`, hang glyphs outside its brackets? It does when
/// Optical Margin Alignment or its first paragraph's Roman Hanging Punctuation is on and its
/// glyphs sit elsewhere with both off.
fn hangs_on_path(t: &TextObject, lay: &vectorcraft_text::TextLayout) -> bool {
    if !t.optical_margin && !t.para_at(0).roman_hanging {
        return false;
    }
    let mut flush = t.clone();
    flush.optical_margin = false;
    let mut styles = flush.paragraph_styles();
    for p in &mut styles {
        p.roman_hanging = false;
    }
    flush.set_paragraph_styles(styles);
    let flush = vectorcraft_text::layout(FontDb::global(), &flush);
    lay.glyphs.len() != flush.glyphs.len() || lay.glyphs.iter().zip(&flush.glyphs).any(|(a, b)| a.origin != b.origin)
}

/// The `direction` and `unicode-bidi` of a `<tspan>` of characters in style `st`: an embedding in
/// its Character Direction, and nothing without one. Each `<tspan>` of characters with a direction
/// carries them, and the `<text>` does not. Type with a run set Right to Left is written as
/// outlines ([`Writer::text`]), so live text writes `direction="ltr"` only.
fn direction_props(st: &CharStyle) -> Props {
    let Some(d) = st.char_direction else { return Props::new() };
    let direction = if d == vectorcraft_doc::ParaDirection::RightToLeft { "rtl" } else { "ltr" };
    vec![("direction", direction.into()), ("unicode-bidi", "embed".into())]
}

/// What the `<tspan>` of a run whose properties are `run` sets over its `<text>`'s `base`: the
/// properties that differ, and the initial value of each one `base` sets and the run doesn't.
fn run_diff(base: &Props, run: Props) -> Props {
    let reset: Props =
        base.iter().filter(|(k, _)| !run.iter().any(|(r, _)| r == k)).filter_map(|(k, _)| initial_value(k).map(|v| (*k, v.to_string()))).collect();
    run.into_iter().filter(|kv| !base.contains(kv)).chain(reset).collect()
}

/// The initial value of an inherited property a run may leave unset (decorations can't be undone
/// on a `<tspan>`).
fn initial_value(k: &str) -> Option<&'static str> {
    Some(match k {
        "font-weight" | "font-style" | "font-stretch" | "font-feature-settings" | "font-variation-settings" => "normal",
        "stroke" | "stroke-dasharray" => "none",
        "stroke-width" | "stroke-opacity" => "1",
        "stroke-linecap" => "butt",
        "stroke-linejoin" => "miter",
        "stroke-miterlimit" => "4",
        "stroke-dashoffset" | "letter-spacing" => "0",
        "font-kerning" => "auto",
        _ => return None,
    })
}

/// How closely stroke outlines that clip an image follow the stroke (points).
pub(super) const OUTLINE_TOLERANCE: f64 = 0.01;

/// What the lines of one `<text>` share.
#[derive(Clone)]
struct TextBase {
    /// The `<text>` element's character properties, which each `<tspan>` differs from.
    props: Props,
    /// The text's layout bounds and the map from text space to user space (character paints).
    space: (Rect, Affine),
    /// Lines anchored at their centre or right end (`text-anchor`), positioned once each.
    anchored: bool,
}

/// One laid-out line: its paragraph (index) and its segments.
struct Line {
    para: usize,
    segs: Vec<Segment>,
}

/// Characters of one style on one line, written as one `<tspan>`.
struct Segment {
    /// Index into `TextObject::runs`.
    run: usize,
    /// Where a reader starts drawing the segment (text space; `y` is the baseline).
    x: f64,
    y: f64,
    text: String,
    /// The width a reader gives the segment (tracking included). On a line that is justified or
    /// set with Mojikumi (unless `fewer`), that is the width without justification and Mojikumi
    /// ([`reader_advances`]), for horizontally scaled type also without its last character's
    /// kerning against the character after it ([`end_segment`]); elsewhere, the laid-out width.
    advance: f64,
    /// The next segment needs its own position (after a tab, a word space of a justified line, or
    /// the next segment starts with a character the layout draws elsewhere than a reader would).
    brk: bool,
    /// Glyph Scaling of the segment's one glyph (Justification…), which a reader stretches it to
    /// (`textLength`); 1 for other segments.
    scale: f64,
    /// A list marker's characters and tab, written in
    /// [`vectorcraft_doc::text::ParaList::marker_style`] of the run's style.
    marker: bool,
}

/// How far a reader may draw a character from where the layout draws it before the character is
/// given its own position (points).
const PLACE_TOLERANCE: f64 = 1e-6;

/// The laid-out lines of `t` as segments: a new segment at every style change, after tabs and,
/// on justified lines, after each word space (unless `fewer`). The text comes from the clusters
/// the layout placed: soft hyphens draw nothing, a line broken by hyphenation ends in `-`, all
/// caps are upper case. Unless `fewer`, a line that is justified or composed with Mojikumi also
/// starts a new segment at each character the layout draws elsewhere than a reader continuing
/// the segment would ([`reader_advances`]): a Justify All line's letter gaps, the gaps between
/// the characters of a justified Japanese line, and the spacing Mojikumi changes.
fn text_lines(t: &TextObject, lay: &vectorcraft_text::TextLayout, fewer: bool) -> Vec<Line> {
    let plain = t.plain_text();
    let mut lines = Vec::with_capacity(lay.lines.len());
    let (mut kerned, mut unkerned) = (None, None);
    for line in &lay.lines {
        let para = plain.as_bytes().get(..line.start).map_or(0, |b| b.iter().filter(|&&c| c == b'\n').count());
        let style = t.para_at(para);
        let split_words = !fewer && !matches!(style.justify, Justify::Auto | Justify::Left | Justify::Center | Justify::Right);
        // Each cluster's advance and drawing offset as a reader gives them, by its byte offset.
        // Justification…'s Desired word and letter spacing and glyph scaling move characters from
        // where a reader draws them, on every line.
        let spaced = !fewer && !style.justification.desired_is_default();
        let reader = if split_words || spaced || (!fewer && !style.mojikumi.is_none()) {
            Some(&*kerned.get_or_insert_with(|| reader_advances(t, lay, true)))
        } else {
            None
        };
        // Where a reader draws the next character of the last segment, and the byte offset and
        // reader's advance of the last segment's last cluster.
        let mut pen = 0.0;
        let mut tail = None;
        let mut segs: Vec<Segment> = Vec::new();
        let mut cluster = None;
        // A list marker's glyphs write its characters and then a tab (glyph `i` of the layout).
        let source = |i: usize, g: &vectorcraft_text::PositionedGlyph| {
            lay.marker_text(i).unwrap_or_else(|| if g.len == 0 { "-" } else { plain.get(g.byte..g.byte + g.len).unwrap_or("") })
        };
        let mut glyphs = lay.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or_default();
        // Spaces where the line wrapped (it doesn't end its paragraph) don't belong to the line.
        if plain.get(line.end..).is_some_and(|rest| !rest.is_empty() && !rest.starts_with('\n')) {
            while let Some((_, rest)) =
                glyphs.split_last().filter(|(g, rest)| source(line.glyph_start + rest.len(), g).chars().all(|c| c.is_whitespace() && c != '\t'))
            {
                glyphs = rest;
            }
        }
        let mut after_inline = false;
        for (k, g) in glyphs.iter().enumerate() {
            let Some(run) = t.runs.get(g.run) else { continue };
            // A list marker before an inline graphic is in the graphic's run, and is text.
            if run.inline.is_some() && lay.marker_of(line.glyph_start + k).is_none() {
                // An inline graphic (written as art): the text after it is placed anew.
                if let Some(last) = segs.last_mut() {
                    last.brk = true;
                }
                after_inline = true;
                cluster = None;
                continue;
            }
            // The cluster as a reader sets it (a hyphen the layout adds is set as laid out).
            let set = reader.and_then(|r| r.get(&g.byte)).filter(|_| g.len > 0);
            if g.len > 0 && cluster == Some(g.byte) {
                // Another glyph of the same cluster (a reader's advance covers the whole cluster).
                if let Some(last) = segs.last_mut().filter(|_| set.is_none()) {
                    last.advance += g.advance;
                    pen += g.advance;
                }
                continue;
            }
            // A glyph that stands for no character (a list marker before the paragraph's first
            // character) starts no cluster.
            cluster = (g.len > 0).then_some(g.byte);
            let (advance, x) = match set {
                Some(&(advance, offset)) => {
                    // A glyph the layout draws before its pen (Mojikumi) starts that much earlier.
                    let x = g.xf.translation().x - offset;
                    (advance, if (x - g.origin.x).abs() > PLACE_TOLERANCE { x } else { g.origin.x })
                }
                None => (g.advance, g.origin.x),
            };
            let src: String = source(line.glyph_start + k, g).chars().filter(|c| *c != '\u{ad}').map(|c| run.style.digits.map(c)).collect();
            let piece = if run.style.all_caps { src.to_uppercase() } else { src };
            let brk = piece == "\t" || (split_words && !piece.is_empty() && piece.chars().all(char::is_whitespace));
            // A glyph Glyph Scaling widens or narrows is a segment of its own, stretched to its width.
            let scale = if reader.is_some() && (g.scale - 1.0).abs() > 1e-9 && g.scale.is_finite() { g.scale } else { 1.0 };
            let fresh = std::mem::take(&mut after_inline) || scale != 1.0 || segs.last().is_some_and(|last| last.scale != 1.0);
            // The layout draws the character elsewhere than a reader continuing the line would.
            let moved = reader.is_some()
                && segs.last().is_some_and(|last| (x - pen).abs() > PLACE_TOLERANCE || (g.origin.y - last.y).abs() > PLACE_TOLERANCE);
            pen = x + advance;
            // A list marker is a segment of its own, and so is the text after it. In horizontally
            // scaled type a tab is a segment apart from the characters before it, so a reader
            // stretching those characters to their width (`textLength`) stretches them alone.
            let marker = lay.marker_of(line.glyph_start + k).is_some();
            let tab_apart = piece == "\t" && (run.style.h_scale - run.style.v_scale).abs() > 1e-9;
            match segs.last_mut() {
                Some(last) if last.run == g.run && !last.brk && !fresh && !moved && last.marker == marker && !tab_apart => {
                    last.text.push_str(&piece);
                    last.advance += advance;
                    last.brk = brk;
                }
                last => {
                    if let Some(last) = last {
                        last.brk |= moved || last.marker != marker;
                        end_segment(last, tail, t, lay, &mut unkerned);
                    }
                    segs.push(Segment { run: g.run, x, y: g.origin.y, text: piece, advance, brk, scale, marker });
                }
            }
            tail = set.map(|_| (g.byte, advance));
        }
        if let Some(last) = segs.last_mut() {
            end_segment(last, tail, t, lay, &mut unkerned);
        }
        segs.retain(|s| !s.text.is_empty());
        if !segs.is_empty() {
            lines.push(Line { para, segs });
        }
    }
    lines
}

/// Ends segment `seg` of horizontally scaled type (or a glyph Glyph Scaling scales), which a reader stretches to its width
/// (`textLength`): its last cluster (`tail`: byte offset and reader's advance) takes the advance
/// a reader gives it at the end of a `<tspan>`, without the font's kerning against the character
/// after it. `unkerned` holds [`reader_advances`] without kerning once a segment needs them.
fn end_segment(
    seg: &mut Segment,
    tail: Option<(usize, f64)>,
    t: &TextObject,
    lay: &vectorcraft_text::TextLayout,
    unkerned: &mut Option<HashMap<usize, (f64, f64)>>,
) {
    let Some((byte, advance)) = tail else { return };
    if seg.scale == 1.0 && t.runs.get(seg.run).is_none_or(|r| (r.style.h_scale - r.style.v_scale).abs() <= 1e-9) {
        return;
    }
    if let Some(&(alone, _)) = unkerned.get_or_insert_with(|| reader_advances(t, lay, false)).get(&byte) {
        seg.advance += alone - advance;
    }
}

/// The advance a reader of live SVG text gives each character cluster of `t` and how far from
/// its pen it draws the cluster's first glyph, keyed by the cluster's byte offset: `t` laid out
/// as point type (which isn't justified) without Mojikumi, scaled as `t`'s layout `lay` is by
/// Shrink Text to Fit. Without `kerning`, the font's pair kerning is left out (as manual
/// kerning leaves it out).
fn reader_advances(t: &TextObject, lay: &vectorcraft_text::TextLayout, kerning: bool) -> HashMap<usize, (f64, f64)> {
    let mut set = TextObject { kind: TextKind::Point, ..t.clone() };
    for p in std::iter::once(&mut set.para).chain(set.paras.iter_mut()) {
        p.mojikumi = vectorcraft_doc::Mojikumi::None;
        // A reader spaces words and letters as the font does.
        p.justification = vectorcraft_doc::JustificationOptions::default();
    }
    if !kerning {
        for r in &mut set.runs {
            r.style.kerning.get_or_insert(0.0);
        }
    }
    let mut out: HashMap<usize, (f64, f64)> = HashMap::new();
    for g in vectorcraft_text::layout(FontDb::global(), &set).glyphs.iter().filter(|g| g.len > 0) {
        let (advance, _) = out.entry(g.byte).or_insert((0.0, (g.xf.translation().x - g.origin.x) * lay.fit_scale));
        *advance += g.advance * lay.fit_scale;
    }
    out
}

/// Where a line of segments starts: where its first segment starts or, `anchor`ed (0.5: center, 1:
/// right end), that point of its width. `None` for an empty line.
fn line_start(line: &[Segment], anchor: Option<f64>) -> Option<(f64, f64)> {
    let (first, last) = (line.first()?, line.last()?);
    let x = match anchor {
        Some(f) => first.x + f * (last.x + last.advance - first.x),
        None => first.x,
    };
    Some((x, first.y))
}
