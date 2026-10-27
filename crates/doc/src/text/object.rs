use super::area::{AreaOptions, PathAlign, PathEffect};
use super::char_style::CharStyle;
use super::para_style::ParaStyle;
use super::runs::{RubySpan, TextKind, TextRun, move_anchors, reverse_from_end};
use super::tabs_lists::ParaList;
use super::*;

impl TextObject {
    /// None for collapsed or unrepresentable transforms, whose type dimensions cannot
    /// be edited by compensating the stored attributes.
    pub fn style_scale(&self) -> Option<TextStyleScale> {
        let [a, b, c, d, e, f] = self.xf.as_coeffs();
        if ![a, b, c, d, e, f].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (sx, sy) = (a.hypot(b), c.hypot(d));
        let determinant = (a / sx) * (d / sy) - (b / sx) * (c / sy);
        let horizontal = sx / sy;
        if !sy.is_finite() || sy <= 0.0 || !horizontal.is_finite() || horizontal <= 0.0 || !determinant.is_finite() || determinant.abs() <= 1e-12 {
            return None;
        }
        // Avoid a pure rotation turning otherwise identical fields into mixed values.
        let unit = |v: f64| if (v - 1.0).abs() <= 1e-12 { 1.0 } else { v };
        Some(TextStyleScale { points: unit(sy), horizontal: unit(horizontal) })
    }

    /// Character attributes as presented by controls (stored runs stay local for layout
    /// and rich-text clipboard operations). Explicit vertical character scale stays separate.
    pub fn effective_char_style(&self, style: &CharStyle) -> Option<CharStyle> {
        let scale = self.style_scale()?;
        let mut style = style.clone();
        style.size *= scale.points;
        style.leading = style.leading.map(|v| v * scale.points);
        style.baseline_shift *= scale.points;
        style.h_scale *= scale.horizontal;
        if ![style.size, style.leading.unwrap_or(0.0), style.baseline_shift, style.h_scale].iter().all(|v| v.is_finite()) {
            return None;
        }
        Some(style)
    }

    /// Bakes the object's scale ([`style_scale`]) into character attributes and removes that
    /// scale from `xf`, so panels show document-point sizes without a separate transform factor.
    /// Returns `false` when there is nothing to bake (identity scale or an unrepresentable
    /// transform).
    pub fn scale_text_attributes(&mut self) -> bool {
        let Some(scale) = self.style_scale() else { return false };
        if (scale.points - 1.0).abs() <= 1e-12 && (scale.horizontal - 1.0).abs() <= 1e-12 {
            return false;
        }
        let [a, b, c, d, _, _] = self.xf.as_coeffs();
        let (sx, sy) = (a.hypot(b), c.hypot(d));
        if sx <= 1e-12 || sy <= 1e-12 {
            return false;
        }
        let mut baked = Vec::with_capacity(self.runs.len());
        for run in &self.runs {
            let Some(eff) = self.effective_char_style(&run.style) else { return false };
            baked.push(eff);
        }
        for (run, style) in self.runs.iter_mut().zip(baked) {
            run.style = style;
        }
        // Character strokes are drawn in text space: widen them by the vertical scale we remove.
        self.scale_char_strokes(1.0 / sy);
        self.xf *= Affine::scale_non_uniform(1.0 / sx, 1.0 / sy);
        self.cached_bounds = None;
        self.cached_baselines.clear();
        true
    }

    pub fn point(origin: Point, text: &str, style: CharStyle) -> Self {
        Self {
            vertical: false,
            kind: TextKind::Point,
            xf: Affine::translate(origin.to_vec2()),
            runs: vec![TextRun::new(text, style)],
            ruby: Vec::new(),
            para: ParaStyle::default(),
            paras: Vec::new(),
            first_para_continues: false,
            area: AreaOptions::default(),
            path_effect: PathEffect::default(),
            path_align: PathAlign::default(),
            path_spacing: 0.0,
            wrap: Vec::new(),
            optical_margin: false,
            system_layout: false,
            fractional_widths: true,
            list_count: 0,
            cached_bounds: None,
            cached_baselines: Vec::new(),
        }
    }
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
    /// Clamp the ruby spans to the plain text, drop empty/empty-reading ones and sort them.
    ///
    /// Called after loading and editing text: ranges from a file or an edit are untrusted, and a
    /// range past the text or an inverted one would lay out nothing (or panic in a silly place).
    pub fn normalize_ruby(&mut self) {
        let len: usize = self.runs.iter().map(|r| r.text.len()).sum();
        for r in &mut self.ruby {
            r.start = r.start.min(len);
            r.end = r.end.min(len);
            if r.start > r.end {
                std::mem::swap(&mut r.start, &mut r.end);
            }
            r.text = r.text.trim().to_string();
        }
        self.ruby.retain(|r| r.start < r.end && !r.text.is_empty());
        self.ruby.sort_by_key(|r| (r.start, r.end));
    }
    /// The ruby span covering byte `byte` of the plain text, if any.
    pub fn ruby_at(&self, byte: usize) -> Option<&RubySpan> {
        self.ruby.iter().find(|r| r.start <= byte && byte < r.end)
    }
    /// Ruby spans touching the plain-text byte range `start..end`, with their ranges ordered.
    pub fn ruby_in(&self, start: usize, end: usize) -> impl Iterator<Item = &RubySpan> {
        let (a, b) = (start.min(end), start.max(end));
        self.ruby.iter().filter(move |r| r.start < b && a < r.end)
    }
    /// Keep the ruby spans over the text after replacing plain-text bytes `start..end` with
    /// `inserted` bytes of text: spans after the edit move with the text, ones the edit runs into
    /// keep their start and end up where the replaced part ended. Call it after every edit of the
    /// runs (`vectorcraft_text::edit`), then [`Self::normalize_ruby`] has the last word.
    pub fn ruby_after_replace(&mut self, start: usize, end: usize, inserted: usize) {
        let (start, end) = (start.min(end), start.max(end));
        let delta = inserted as i64 - (end.saturating_sub(start)) as i64;
        let shift = |v: usize| -> usize { (v as i64 + delta).max(0) as usize };
        for r in &mut self.ruby {
            if r.end <= start {
                continue;
            }
            if r.start >= end {
                r.start = shift(r.start);
                r.end = shift(r.end);
            } else {
                r.start = r.start.min(start);
                r.end = shift(r.end).max(r.start);
            }
        }
        self.normalize_ruby();
    }
    /// Number of paragraphs (`\n`-separated; empty text has one).
    pub fn paragraph_count(&self) -> usize {
        1 + self.runs.iter().map(|r| r.text.bytes().filter(|&b| b == b'\n').count()).sum::<usize>()
    }
    /// Attributes of paragraph `i` (the last paragraph's past the end).
    pub fn para_at(&self, i: usize) -> &ParaStyle {
        self.paras.get(i).or(self.paras.last()).unwrap_or(&self.para)
    }
    /// Every paragraph's attributes, one per paragraph.
    pub fn paragraph_styles(&self) -> Vec<ParaStyle> {
        (0..self.paragraph_count()).map(|i| self.para_at(i).clone()).collect()
    }
    /// Set every paragraph's attributes from `styles` (one per paragraph; a short list repeats its
    /// last entry, a long one is cut).
    pub fn set_paragraph_styles(&mut self, styles: Vec<ParaStyle>) {
        if let Some(first) = styles.first() {
            self.para = first.clone();
        }
        self.paras = styles;
        self.normalize_paras();
    }
    /// The same attributes for every paragraph.
    pub fn set_all_paras(&mut self, style: ParaStyle) {
        self.para = style;
        self.paras.clear();
    }
    /// Re-establish the `paras` invariant: one entry per paragraph (cut, or extended with the
    /// last), `paras[0] == para`, and empty when every paragraph is alike.
    pub fn normalize_paras(&mut self) {
        if self.paras.is_empty() {
            return;
        }
        let n = self.paragraph_count();
        let last = self.paras.last().cloned().unwrap_or_default();
        self.paras.resize(n, last);
        if let Some(first) = self.paras.first_mut() {
            first.clone_from(&self.para);
        }
        if self.paras.iter().all(|p| *p == self.para) {
            self.paras.clear();
        }
    }
    /// Indices of the paragraphs that byte range `start..end` of the plain text touches (a caret
    /// touches its paragraph).
    pub fn paragraphs_in(&self, start: usize, end: usize) -> std::ops::Range<usize> {
        let text = self.plain_text();
        let (a, b) = (start.min(end).min(text.len()), start.max(end).min(text.len()));
        let index = |byte: usize| text.as_bytes().get(..byte).map_or(0, |s| s.iter().filter(|&&c| c == b'\n').count());
        index(a)..index(b) + 1
    }
    /// The list that the characters before byte `at` start ([`ParaList::typed`]) with the byte
    /// where their paragraph starts: when the plain text from the start of the paragraph up to
    /// `at` is a list's start followed by a space, and the paragraph is in no list. None otherwise.
    pub fn typed_list_at(&self, at: usize) -> Option<(usize, ParaList)> {
        let text = self.plain_text();
        let before = text.get(..at)?;
        let start = before.rfind('\n').map_or(0, |i| i + 1);
        let list = ParaList::typed(before[start..].strip_suffix(' ')?)?;
        let index = before.bytes().filter(|&b| b == b'\n').count();
        self.para_at(index).list.is_none().then_some((start, list))
    }
    /// Apply `f` to the attributes of paragraphs `range` (indices; None = every paragraph).
    pub fn edit_paras(&mut self, range: Option<std::ops::Range<usize>>, mut f: impl FnMut(&mut ParaStyle)) {
        match range {
            None => {
                f(&mut self.para);
                for p in &mut self.paras {
                    f(p);
                }
            }
            Some(r) => {
                let mut v = self.paragraph_styles();
                for p in v.iter_mut().take(r.end).skip(r.start) {
                    f(p);
                }
                self.set_paragraph_styles(v);
            }
        }
        self.normalize_paras();
    }
    /// Every stored paragraph style (`para` and the per-paragraph ones), for scans and renames.
    pub fn para_styles_mut(&mut self) -> impl Iterator<Item = &mut ParaStyle> {
        std::iter::once(&mut self.para).chain(self.paras.iter_mut())
    }
    /// Every stored paragraph style (`para` and the per-paragraph ones).
    pub fn para_styles(&self) -> impl Iterator<Item = &ParaStyle> {
        std::iter::once(&self.para).chain(self.paras.iter())
    }
    /// Update the paragraph styles for replacing bytes `start..end` of the current plain text by
    /// `insert` (call before changing the runs). The paragraph the range starts in keeps its
    /// style (merging paragraphs: the first one's wins); paragraphs the insertion starts take that
    /// style too (Return continues the paragraph it splits); later paragraphs keep theirs.
    pub fn splice_paras(&mut self, start: usize, end: usize, insert: &str) {
        if self.paras.is_empty() {
            return;
        }
        let r = self.paragraphs_in(start, end);
        let v = self.paragraph_styles();
        let Some(keep) = v.get(r.start).cloned() else { return };
        let added = insert.bytes().filter(|&b| b == b'\n').count();
        let mut out: Vec<ParaStyle> = v.iter().take(r.start + 1).cloned().collect();
        out.extend(std::iter::repeat_n(keep, added));
        out.extend(v.iter().skip(r.end).cloned());
        self.paras = out;
        if let Some(first) = self.paras.first() {
            self.para = first.clone();
        }
    }
    pub fn first_style(&self) -> CharStyle {
        self.runs.first().map(|r| r.style.clone()).unwrap_or_default()
    }
    /// Approximate bounds when no layout cache is available (0.55 em average advance).
    pub fn estimate_bounds(&self) -> Rect {
        let st = self.first_style();
        let text = self.plain_text();
        let lines: Vec<&str> = text.split('\n').collect();
        let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * st.size * 0.55;
        let lead = st.effective_leading();
        Rect::new(0.0, -st.size * 0.8, w.max(1.0), -st.size * 0.8 + lead * lines.len().max(1) as f64)
    }
    pub fn bounds(&self) -> Option<Rect> {
        match &self.kind {
            TextKind::Area { frame } => frame.bounds().map(|b| self.xf.transform_rect_bbox(b)),
            TextKind::OnPath { path, .. } => path.bounds().map(|b| self.xf.transform_rect_bbox(b)),
            TextKind::Point => Some(self.xf.transform_rect_bbox(self.cached_bounds.unwrap_or_else(|| self.estimate_bounds()))),
        }
    }
    /// How far the characters' strokes paint past their glyphs, in document space: they scale
    /// with the type.
    pub fn char_stroke_outset(&self) -> f64 {
        let k = crate::node::Scaling::factor(self.xf).unwrap_or(1.0);
        self.runs.iter().map(|r| r.style.stroke_outset()).fold(0.0, f64::max) * k
    }
    /// How far the characters' strokes can paint past their glyphs, their miter spikes and square
    /// caps included ([`StrokeLayer::reach`]), in document space: what a change of the type must
    /// redraw. They stretch with the type, so this is as far as its transform takes them on either
    /// axis. The visual bounds take [`Self::char_stroke_outset`].
    pub fn char_stroke_reach(&self) -> f64 {
        let reach = self.runs.iter().filter(|r| r.style.has_stroke()).map(|r| r.style.stroke_layer().reach()).fold(0.0, f64::max);
        let [a, b, c, d, ..] = self.xf.as_coeffs();
        reach * (a.abs() + c.abs()).max(b.abs() + d.abs())
    }
    pub fn transform(&mut self, a: Affine) {
        self.xf = a * self.xf;
    }
    /// Type on a path's path in document space; None for other type.
    pub fn type_path(&self) -> Option<PathData> {
        match &self.kind {
            TextKind::OnPath { path, .. } => Some(path.transformed(self.xf)),
            _ => None,
        }
    }
    /// Flip type on a path to the other side of its path (Type on a Path Options › Flip, or its
    /// centre bracket dragged across the path): the path runs the other way and the brackets swap
    /// ends, so the type keeps its stretch of the path. False, changing nothing, for other type.
    pub fn flip_on_path(&mut self) -> bool {
        let Some((s, e)) = self.kind.path_span() else { return false };
        let TextKind::OnPath { path, start, end } = &mut self.kind else { return false };
        reverse_from_end(path);
        if path.is_closed() {
            *start = (1.0 - e).rem_euclid(1.0);
            *end = (e - s < 1.0 - 1e-9).then(|| (1.0 - s).rem_euclid(1.0));
        } else {
            *start = 1.0 - e;
            *end = (s > 1e-9).then_some(1.0 - s);
        }
        true
    }
    /// Area type's frame (the type area) in document space; None for other type.
    pub fn area_frame(&self) -> Option<PathData> {
        match &self.kind {
            TextKind::Area { frame } => Some(frame.transformed(self.xf)),
            _ => None,
        }
    }
    /// Reshape area type's frame by `a`, a document-space transform, keeping the type at its size
    /// (a bounding-box resize: the text reflows, it isn't scaled). False, changing nothing, for
    /// other type, a degenerate `xf` or a frame that would not be finite.
    pub fn transform_area(&mut self, a: Affine) -> bool {
        let xf = self.xf;
        self.reshape_area(|frame, to_text| {
            frame.transform(to_text * a * xf);
            true
        })
    }
    /// Move anchors `refs` (subpath, anchor) of area type's frame by `d` in document space, with
    /// their handles (a Direct Selection drag of a frame corner or edge). False, changing
    /// nothing, when an anchor doesn't exist, or as [`Self::transform_area`].
    pub fn move_area_anchors(&mut self, refs: &[(usize, usize)], d: Vec2) -> bool {
        self.reshape_area(|frame, to_text| move_anchors(frame, to_text, refs, d))
    }
    /// Move anchors `refs` (subpath, anchor) of type on a path's path by `d` in document space,
    /// with their handles (a Direct Selection drag of an anchor); the brackets keep their
    /// fractions of the path. False, changing nothing, for other type, when an anchor doesn't
    /// exist, a degenerate `xf` or a path that would not be finite.
    pub fn move_path_anchors(&mut self, refs: &[(usize, usize)], d: Vec2) -> bool {
        matches!(self.kind, TextKind::OnPath { .. }) && self.reshape_local(|path, to_text| move_anchors(path, to_text, refs, d))
    }
    /// Edit a copy of the area frame with `f` (given the document → text space map) and keep it
    /// when `f` succeeds and the result is finite.
    fn reshape_area(&mut self, f: impl FnOnce(&mut PathData, Affine) -> bool) -> bool {
        matches!(self.kind, TextKind::Area { .. }) && self.reshape_local(f)
    }
    /// [`Self::reshape_area`] for the area frame or type on a path's path.
    fn reshape_local(&mut self, f: impl FnOnce(&mut PathData, Affine) -> bool) -> bool {
        let det = self.xf.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return false;
        }
        let to_text = self.xf.inverse();
        let (TextKind::Area { frame } | TextKind::OnPath { path: frame, .. }) = &mut self.kind else { return false };
        let mut next = frame.clone();
        if !f(&mut next, to_text) || !next.anchors().all(|(_, _, a)| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite()) {
            return false;
        }
        *frame = next;
        true
    }
    /// Scale the character strokes' weights and dashes by `s` (they are drawn in text space, so
    /// `1 / scale` keeps their weight through a transform that scales the type).
    pub fn scale_char_strokes(&mut self, s: f64) {
        for r in &mut self.runs {
            r.style.stroke_width *= s;
            if let Some(d) = &mut r.style.stroke_dash {
                d.scale(s);
            }
        }
    }
    /// Layout bounds in text space (the layout cache, else the estimate): the box an unplaced run
    /// gradient fits, as the renderers lay it out.
    pub fn local_bounds(&self) -> Rect {
        self.cached_bounds.unwrap_or_else(|| self.estimate_bounds())
    }
    /// The baselines in text space (the layout cache): for point type with none cached, the first
    /// one across [`Self::local_bounds`] (down the first column's centre line for vertical type).
    pub fn baselines(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        let first = (self.cached_baselines.is_empty() && matches!(self.kind, TextKind::Point)).then(|| {
            let b = self.local_bounds();
            if self.vertical { (Point::new(0.0, b.y0), Point::new(0.0, b.y1)) } else { (Point::new(b.x0, 0.0), Point::new(b.x1, 0.0)) }
        });
        self.cached_baselines.iter().copied().chain(first)
    }
    /// Keeps every run's size, leading, baseline shift and scales in range
    /// ([`CharStyle::keep_dimensions_in_range`]) and every paragraph's indents, spacing and tab
    /// stops ([`ParaStyle::keep_spacing_in_range`]).
    pub fn keep_type_values_in_range(&mut self) {
        for r in &mut self.runs {
            r.style.keep_dimensions_in_range();
        }
        for p in self.para_styles_mut() {
            p.keep_spacing_in_range();
        }
    }
}

impl crate::Document {
    /// Keeps the character dimensions, indents, paragraph spacing and tab stops in range
    /// ([`TextObject::keep_type_values_in_range`]) in the type of the layers, symbols, graph
    /// designs, pattern swatches and the pattern definition that pattern editing's Cancel puts
    /// back (opacity-mask art included), and the same attributes of the character and paragraph
    /// styles. It recurses through the art: call it on a document no deeper than
    /// [`crate::depth::MAX_DEPTH`].
    pub fn keep_type_values_in_range(&mut self) {
        let mut fix = |n: &mut crate::Node| {
            if let crate::NodeKind::Text(t) = &mut n.kind {
                t.keep_type_values_in_range();
            }
        };
        let roots = self
            .layers
            .iter_mut()
            .chain(self.symbols.iter_mut().map(|s| &mut s.art))
            .chain(self.graph_designs.iter_mut().map(|g| &mut g.art))
            .chain(self.patterns.iter_mut().flat_map(|p| p.art.iter_mut()))
            .chain(self.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()).flat_map(|p| p.art.iter_mut()));
        for root in roots {
            crate::placed_document::each_mut(std::sync::Arc::make_mut(root), &mut fix);
        }
        // Styles store the attributes they set by their serialized names (a number, or null for
        // Auto leading); a number is rewritten only when it changes.
        for def in self.char_styles.iter_mut() {
            let num = |k: &str| def.attrs.get(k).and_then(serde_json::Value::as_f64);
            let mut st = CharStyle { leading: num("leading"), ..CharStyle::default() };
            st.size = num("size").unwrap_or(st.size);
            st.h_scale = num("h_scale").unwrap_or(st.h_scale);
            st.v_scale = num("v_scale").unwrap_or(st.v_scale);
            st.baseline_shift = num("baseline_shift").unwrap_or(st.baseline_shift);
            st.keep_dimensions_in_range();
            let kept = [
                ("size", Some(st.size)),
                ("h_scale", Some(st.h_scale)),
                ("v_scale", Some(st.v_scale)),
                ("baseline_shift", Some(st.baseline_shift)),
                ("leading", st.leading),
            ];
            for (k, v) in kept {
                if let Some(old) = def.attrs.get_mut(k)
                    && old.is_number()
                    && old.as_f64() != v
                {
                    *old = v.into();
                }
            }
        }
        for def in self.para_styles.iter_mut() {
            for k in ["left_indent", "right_indent", "first_line_indent", "space_before", "space_after"] {
                if let Some(v) = def.attrs.get_mut(k)
                    && let Some(x) = v.as_f64()
                    && x.abs() > crate::MAX_COORD
                {
                    *v = x.clamp(-crate::MAX_COORD, crate::MAX_COORD).into();
                }
            }
            for t in def.attrs.get_mut("tabs").and_then(serde_json::Value::as_array_mut).into_iter().flatten() {
                if let Some(v) = t.get_mut("position")
                    && let Some(x) = v.as_f64()
                    && !(0.0..=100_000.0).contains(&x)
                {
                    *v = x.clamp(0.0, 100_000.0).into();
                }
            }
        }
    }

    /// Clamps the ruby spans of the type in the layers, symbols, graph designs, pattern swatches
    /// and the pattern definition that pattern editing's Cancel puts back (opacity-mask art
    /// included) to their text ([`TextObject::normalize_ruby`]). Loaded files and edits carry
    /// ranges from outside: a range past the text, an inverted one or an empty reading would lay
    /// out nothing. It recurses through the art: call it on a document no deeper than
    /// [`crate::depth::MAX_DEPTH`].
    pub fn fix_ruby(&mut self) {
        let mut fix = |n: &mut crate::Node| {
            if let crate::NodeKind::Text(t) = &mut n.kind {
                t.normalize_ruby();
            }
        };
        let roots = self
            .layers
            .iter_mut()
            .chain(self.symbols.iter_mut().map(|s| &mut s.art))
            .chain(self.graph_designs.iter_mut().map(|g| &mut g.art))
            .chain(self.patterns.iter_mut().flat_map(|p| p.art.iter_mut()))
            .chain(self.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()).flat_map(|p| p.art.iter_mut()));
        for root in roots {
            crate::placed_document::each_mut(std::sync::Arc::make_mut(root), &mut fix);
        }
    }
}
