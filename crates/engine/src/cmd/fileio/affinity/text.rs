use super::layout::{align_lines, fit_lines, font_style, justify, line_gap, place_cell};
use super::nodes::affine;
use super::*;

/// Where text goes: a rigid placement (translation and rotation) and the scale that maps node
/// space into it (applied to the frame's geometry, not the type), plus the type's own scale.
pub(super) struct Placement {
    /// Node space to the frame's space, before `place`.
    pub(super) to_frame: Affine,
    pub(super) place: Affine,
    /// Points per node unit of type size.
    pub(super) size_scale: f64,
    /// Horizontal scale of the type relative to `size_scale`.
    pub(super) h_scale: f64,
}

impl Builder {
    /// Text → its object and the opacity its characters' shared transparency gives the node.
    pub(super) fn text(&mut self, t: &af::Text, to_doc: Affine) -> Option<(TextObject, f32)> {
        let p = self.placement(to_doc * affine(t.transform), to_doc * affine(t.text_transform))?;
        let alpha = self.text_alpha(&t.runs);
        let mut obj = self.type_object(&t.runs, &p)?;
        obj.para.justify = justify(t.align);
        if let Some(first) = obj.paras.first_mut() {
            first.justify = obj.para.justify;
        }
        match t.frame {
            Some(f) => {
                let (a, b) = (p.to_frame * Point::new(f.x0, f.y0), p.to_frame * Point::new(f.x1, f.y1));
                let r = Rect::from_points(a, b);
                obj.xf = p.place * Affine::translate(r.origin().to_vec2());
                obj.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, r.width(), r.height())) };
                // Affinity hangs a frame's first line from its capitals' tops, not the font's ascent.
                obj.area.first_baseline = FirstBaseline::CapHeight;
                if fit_lines(&mut obj, r.width(), r.height()) {
                    self.warn("text frames whose last lines hang out of them (made taller so the lines show)");
                }
                // Affinity shows a line in a frame shorter than it (#1423); here none of it would draw.
                if vectorcraft_text::fit_first_line(vectorcraft_text::FontDb::global(), &mut obj) {
                    self.warn("text frames too short for their first line (made taller so the line shows)");
                }
            }
            None => {
                align_lines(&mut obj);
                obj.xf = p.place * Affine::translate((p.to_frame * Point::new(t.anchor.x, t.anchor.y)).to_vec2());
            }
        }
        Some((obj, alpha))
    }

    /// The opacity text takes from its characters' colour: VectorCraft's characters have none of
    /// their own, so text whose characters share one becomes an object with that opacity.
    fn text_alpha(&mut self, runs: &[af::TextRun]) -> f32 {
        let alphas: Vec<f64> =
            runs.iter().filter_map(|r| if let ap::Paint::Solid(c) = &r.fill { Some(c.alpha()) } else { None }).filter(|a| a.is_finite()).collect();
        let (lo, hi) = alphas.iter().fold((1.0f64, 0.0f64), |(lo, hi), a| (lo.min(*a), hi.max(*a)));
        if alphas.is_empty() || lo >= 1.0 {
            return 1.0;
        }
        if hi - lo > 0.01 {
            self.warn("text with characters of different transparency (imported opaque)");
            return 1.0;
        }
        lo.clamp(0.0, 1.0) as f32
    }

    /// The characters and paragraphs of `runs` as a text object at the origin of text space.
    fn type_object(&mut self, runs: &[af::TextRun], p: &Placement) -> Option<TextObject> {
        let (v, h) = (p.size_scale, p.size_scale * p.h_scale);
        let mut obj = TextObject::point(Point::ZERO, "", CharStyle::default());
        let mut formats: Vec<af::Paragraph> = Vec::new();
        let mut out: Vec<TextRun> = Vec::new();
        let mut paragraph = 0usize;
        // Whether each paragraph here continues the one before after a line break (U+2028):
        // VectorCraft has no line break inside a paragraph, so the lines become paragraphs.
        let mut continues = vec![false];
        for r in runs {
            for c in r.text.chars() {
                match c {
                    '\u{2028}' => continues.push(true),
                    '\u{2029}' | '\r' | '\n' => continues.push(false),
                    _ => {}
                }
            }
            // Affinity separates paragraphs with U+2029 and lines with U+2028; other control
            // characters don't show.
            let text: String = r
                .text
                .chars()
                .filter_map(|c| match c {
                    '\u{2029}' | '\u{2028}' | '\r' | '\n' => Some('\n'),
                    '\t' => Some('\t'),
                    c if c.is_control() => None,
                    c => Some(c),
                })
                .collect();
            if text.is_empty() {
                continue;
            }
            for c in text.chars() {
                if formats.len() <= paragraph {
                    formats.push(r.paragraph);
                }
                if c == '\n' {
                    paragraph += 1;
                }
            }
            let mut style = self.char_style(r, (v, p.h_scale));
            if style.leading.is_none() {
                style.leading = r.paragraph.leading.map(|l| l * v).filter(|l| l.is_finite() && *l > 0.0).map(|l| l.min(5000.0));
            }
            out.push(TextRun::new(text, style));
        }
        // Affinity ends a story with a paragraph mark; VectorCraft's last paragraph has none.
        if let Some(last) = out.last_mut()
            && last.text.ends_with('\n')
        {
            last.text.pop();
            if last.text.is_empty() {
                out.pop();
            }
        }
        obj.runs = out;
        let count = obj.runs.iter().map(|r| r.text.matches('\n').count()).sum::<usize>() + 1;
        let mut paras = Vec::with_capacity(count);
        let mut previous_after = 0.0;
        // The run of each paragraph's first character.
        let mut first_runs = Vec::with_capacity(count);
        {
            let mut at_start = true;
            for (i, r) in obj.runs.iter().enumerate() {
                for c in r.text.chars() {
                    if at_start {
                        first_runs.push(i);
                        at_start = false;
                    }
                    if c == '\n' {
                        at_start = true;
                    }
                }
            }
        }
        // Line gaps by font, each looked up once.
        let mut gaps: std::collections::HashMap<(&str, &str), f64> = std::collections::HashMap::new();
        for i in 0..count {
            let f = formats.get(i).or(formats.last()).copied().unwrap_or_default();
            let pt = |x: f64, k: f64| if (x * k).is_finite() { (x * k).clamp(-5000.0, 5000.0) } else { 0.0 };
            let mut ps = vectorcraft_doc::text::ParaStyle { justify: justify(f.align), ..Default::default() };
            // Affinity places the first line and the others from the left edge; VectorCraft's
            // first-line indent is relative to its left indent.
            ps.left_indent = pt(f.left, h);
            ps.first_line_indent = pt(f.first - f.left, h);
            ps.right_indent = pt(f.right, h);
            // Between two paragraphs Affinity keeps the larger of the space after the first and
            // before the second; VectorCraft adds them.
            let (mut before, mut after) = (pt(f.space_before, v).max(0.0), pt(f.space_after, v).max(0.0));
            // The lines of one Affinity paragraph: no space between them, and the first-line
            // indent only on its first.
            if continues.get(i).copied().unwrap_or(false) {
                before = 0.0;
                ps.first_line_indent = 0.0;
            }
            if continues.get(i + 1).copied().unwrap_or(false) {
                after = 0.0;
            }
            ps.space_before = if i == 0 { before } else { (before - previous_after).max(0.0) };
            ps.space_after = after;
            previous_after = after;
            // Automatic leading: the font size plus the font's line gap, times the paragraph's
            // factor, for artistic and frame text alike. Fitted on Affinity's thumbnails: Bebas
            // Neue Pro and Impact lines about 1 em apart, DIN Condensed (a gap of 0.2 em) 1.2 em,
            // and the public Arial frame fixture fits this better than the font's line height.
            let (family, style) =
                first_runs.get(i).and_then(|r| obj.runs.get(*r)).map_or(("", ""), |r| (r.style.font_family.as_str(), r.style.font_style.as_str()));
            let gap = *gaps.entry((family, style)).or_insert_with(|| line_gap(family, style));
            let ratio = (1.0 + gap) * f.auto_leading;
            // Affinity documents use tiny factors for spacer paragraphs (0.15 of a line); VectorCraft
            // takes 0–500 %.
            if ratio.is_finite() && (0.01..=5.0).contains(&ratio) {
                ps.justification.auto_leading = ratio * 100.0;
            }
            paras.push(ps);
        }
        if let Some(first) = paras.first() {
            obj.para = first.clone();
        }
        obj.paras = paras;
        obj.normalize_paras();
        Some(obj)
    }

    /// A run's character style; `(size_scale, h_scale)` the type's vertical scale to points and
    /// its horizontal scale relative to that.
    fn char_style(&mut self, r: &af::TextRun, (size_scale, h_scale): (f64, f64)) -> CharStyle {
        let finite = |v: f64, d: f64| if v.is_finite() { v } else { d };
        let mut style = CharStyle::default();
        if !r.family.is_empty() {
            style.font_family = r.family.clone();
        } else if !r.postscript.is_empty() {
            style.font_family = r.postscript.clone();
        } else {
            self.warn("text without a font name uses the default font");
        }
        // A family/weight pair cannot identify width variants such as Arial Narrow.
        // Resolve a known PostScript identity to the exact installed family and style.
        if let Some((family, face_style)) = vectorcraft_text::FontDb::global().by_postscript_name(&r.postscript) {
            style.font_family = family;
            style.font_style = face_style;
        } else {
            style.font_style = font_style(r.weight, r.italic);
        }
        style.features = r.features.clone();
        if r.features.iter().any(|feature| !vectorcraft_text::OtFeatures::known_tag(feature)) {
            self.warn("unsupported text OpenType features (retained but not rendered)");
        }
        style.all_caps = r.all_caps;
        style.underline = r.underline;
        style.strikethrough = r.strikethrough;
        style.size = finite(r.size * size_scale, 12.0).clamp(0.1, 1296.0);
        style.leading = r.leading.map(|l| finite(l * size_scale, 0.0)).filter(|l| *l > 0.0).map(|l| l.min(5000.0));
        style.tracking = finite(r.tracking * 1000.0, 0.0).clamp(-1000.0, 10000.0);
        // Text squeezed or stretched by its transform keeps that as horizontal scale, on top of
        // its own.
        let hs = h_scale * r.h_scale;
        if hs.is_finite() && (hs - 1.0).abs() > 1e-3 {
            style.h_scale = (hs * 100.0).clamp(1.0, 1000.0);
        }
        // Transparency is the object's (see `text_alpha`); the paint's transform doesn't matter
        // for a solid colour, and gradients on type are laid out in text space.
        style.fill = match self.paint(&r.fill, Affine::scale(size_scale)) {
            Some((p, _)) => p,
            None => Paint::None,
        };
        if let Some(st) = &r.stroke {
            let w = st.width * size_scale;
            if let Some((paint, _)) = self.paint(&st.paint, Affine::scale(size_scale))
                && w.is_finite()
                && w > 0.0
            {
                style.stroke = paint;
                style.stroke_width = w.min(1000.0);
            }
        }
        style
    }

    /// A table: its cells as area type and the lines on their edges, as VectorCraft has no
    /// tables.
    pub(super) fn table(&mut self, t: &af::Table, to_doc: Affine) -> Vec<Arc<Node>> {
        let mut out = Vec::new();
        let (ncols, nrows) = (t.ncols(), t.nrows());
        if ncols == 0 || nrows == 0 {
            return out;
        }
        self.warn("tables (imported as text frames and lines)");
        let m = to_doc * affine(t.transform);
        let Some(p) = self.placement(m, to_doc * affine(t.text_transform)) else { return out };
        let col = |i: usize| t.columns.get(i).copied().unwrap_or(0.0);
        let row = |i: usize| t.rows.get(i).copied().unwrap_or(0.0);
        for (i, cell) in t.cells.iter().enumerate() {
            if cell.merged_left || cell.runs.is_empty() {
                continue;
            }
            let (r, c) = (i / ncols, i % ncols);
            // Cells to the right merged into this one widen it.
            let span = 1 + t.cells.iter().skip(i + 1).take(ncols - c - 1).take_while(|n| n.merged_left).count();
            let [l, tp, rt, bt] = cell.inset;
            let node = af::Rect { x0: col(c) + l, y0: row(r) + tp, x1: col(c + span) - rt, y1: row(r + 1) - bt };
            let (a, b) = (p.to_frame * Point::new(node.x0, node.y0), p.to_frame * Point::new(node.x1, node.y1));
            let rect = Rect::from_points(a, b);
            if !(rect.width() > 0.0 && rect.height() > 0.0) {
                continue;
            }
            let Some(mut obj) = self.type_object(&cell.runs, &p) else { continue };
            obj.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, rect.width(), rect.height())) };
            obj.area.vertical_align = match cell.valign {
                af::VAlign::Top => VerticalAlign::Top,
                af::VAlign::Center => VerticalAlign::Center,
                af::VAlign::Bottom => VerticalAlign::Bottom,
            };
            obj.area.first_baseline = FirstBaseline::CapHeight;
            let dy = place_cell(&mut obj, rect.width(), rect.height());
            obj.xf = p.place * Affine::translate(rect.origin().to_vec2() + Vec2::new(0.0, dy));
            // A cell's text never vanishes for want of a line's height.
            vectorcraft_text::fit_first_line(vectorcraft_text::FontDb::global(), &mut obj);
            let alpha = self.text_alpha(&cell.runs);
            let mut n = Node::new(self.doc.alloc_id(), NodeKind::Text(Box::new(obj)));
            n.opacity = alpha;
            out.push(Arc::new(n));
        }
        // Lines over the cells: the vertical edges row by row (none inside merged cells), then
        // the horizontal ones.
        let mut line = |b: &mut Self, s: &Option<ap::Stroke>, from: (f64, f64), to: (f64, f64)| {
            let Some(s) = s else { return };
            let Some(layer) = b.stroke_layer(s, to_doc) else { return };
            let mut bp = BezPath::new();
            bp.move_to(m * Point::new(from.0, from.1));
            bp.line_to(m * Point::new(to.0, to.1));
            let mut n = Node::new(
                b.doc.alloc_id(),
                NodeKind::Path { path: PathData::from_bezpath(&bp), rule: FillRule::NonZero, live: None, clipping: false, guide: false },
            );
            n.appearance = Appearance { items: vec![AppearanceItem::Stroke(layer)], ..Appearance::default() }.into();
            out.push(Arc::new(n));
        };
        for r in 0..nrows {
            for k in 0..=ncols {
                let merged = k > 0 && k < ncols && t.cells.get(r * ncols + k).is_some_and(|c| c.merged_left);
                if merged {
                    continue;
                }
                if let Some(s) = t.vertical.get(r * (ncols + 1) + k) {
                    line(self, s, (col(k), row(r)), (col(k), row(r + 1)));
                }
            }
        }
        for j in 0..=nrows {
            for k in 0..ncols {
                if let Some(s) = t.horizontal.get(j * ncols + k) {
                    line(self, s, (col(k), row(j)), (col(k + 1), row(j)));
                }
            }
        }
        out
    }
}
