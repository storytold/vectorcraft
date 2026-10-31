//! Type in the legacy format (version 7 specification §15): text objects `type To` … `TO`, their
//! text path (`a b c d tx ty startPt Tp`, an area's frame or the path type runs along, `TP`), the
//! modal character and paragraph attributes (`Tr Tf Ts Tz Tt TA Tl Ta Ti`), the paint style, and
//! the characters (`Tx`, `Tj`, overflow `TX`; `\r` ends a paragraph). Final-form operators (the
//! computed positions, §15.1) are skipped: the type is laid out again. Strings are decoded by the
//! file's `TE` encoding, else as Windows-1252; a re-encoded font `/_name` is the font its
//! `%AI3_BeginEncoding` names (else `name`).

use kurbo::{Affine, BezPath, ParamCurve, ParamCurveArclen};
use vectorcraft_color::Paint;
use vectorcraft_doc::{CharStyle, Justify, NodeKind, ParaStyle, TextKind, TextObject, TextRun};
use vectorcraft_geom::PathData;

use super::super::obj::V;
use super::{FLIP, GState, Reader};
use crate::latin;

/// Most characters of type read in one file.
const MAX_CHARS: usize = 1 << 22;
/// Largest type size and indent read (points).
const MAX_SIZE: f64 = 1e4;

/// A text object being read.
#[derive(Debug, Default)]
pub(super) struct LegacyText {
    /// `To`'s type: 0 point type, 1 area type, 2 type on a path.
    kind: i64,
    /// `Tp`'s matrix: text space (y up) → art space.
    matrix: Option<Affine>,
    /// Where type on a path starts: a segment's index plus how far along it.
    start: f64,
    /// Between `Tp` and `TP`: the path read is the text path.
    in_path: bool,
    /// The frame or path, in art space.
    path: Option<BezPath>,
    runs: Vec<TextRun>,
    /// Each paragraph's attributes, as they were at its first character.
    paras: Vec<ParaStyle>,
    /// The current paragraph's attributes are in `paras`.
    para_taken: bool,
    /// An overflow paragraph break (`(\r) TX`) not yet followed by type: the story's end.
    pending_break: bool,
    /// The state when it began (lock, visibility), and whether it began on a `%_` line.
    gs: GState,
    hidden: bool,
}

/// The modal type attributes of the legacy format, which last from one text object to the next.
#[derive(Debug)]
pub(super) struct TypeState {
    style: CharStyle,
    para: ParaStyle,
    /// `Tr`.
    render: i64,
    /// The encoding strings are in, by code (`TE`); None where it says nothing.
    encoding: Vec<Option<char>>,
    /// Re-encoded fonts' original names (`%AI3_BeginEncoding: _new old`).
    fonts: Vec<(String, String)>,
    chars: usize,
}

impl Default for TypeState {
    fn default() -> Self {
        Self { style: CharStyle::default(), para: ParaStyle::default(), render: 0, encoding: vec![None; 256], fonts: vec![], chars: 0 }
    }
}

impl TypeState {
    /// `%AI3_BeginEncoding: _new old` (`line` the comment).
    pub(super) fn encoding_comment(&mut self, line: &str) {
        let Some((_, names)) = line.split_once(':') else { return };
        let mut words = names.split_whitespace();
        if let (Some(new), Some(old)) = (words.next(), words.next())
            && self.fonts.len() < 4096
        {
            self.fonts.push((new.trim_start_matches('/').to_string(), old.trim_start_matches('/').to_string()));
        }
    }
}

fn num(vals: &[V], i: usize) -> Option<f64> {
    vals.iter().filter_map(V::num).nth(i).filter(|v| v.is_finite())
}

impl Reader<'_> {
    /// `[encodingPairs TE`: codes, each followed by the glyph names from it on (§3.3.1).
    pub(super) fn te(&mut self, vals: &[V]) {
        let mut code: Option<usize> = None;
        for v in vals {
            match v {
                V::Num(n) if n.is_finite() && (0.0..256.0).contains(n) => code = Some(*n as usize),
                V::Name(name) => {
                    if let Some(c) = code {
                        if let (Some(slot), Ok(b)) = (self.type_state.encoding.get_mut(c), u8::try_from(c)) {
                            *slot = latin::glyph(b, name);
                        }
                        code = Some(c + 1);
                    }
                }
                _ => {}
            }
        }
    }

    /// Is a text object's path being read (so the path painted is its frame or path)?
    pub(super) fn in_text_path(&self) -> bool {
        self.text.as_ref().is_some_and(|t| t.in_path)
    }

    /// `type To`: a text object begins.
    pub(super) fn begin_text(&mut self, vals: &[V], hidden: bool) {
        self.end_path_quietly();
        let kind = num(vals, 0).map_or(0, |k| k as i64);
        self.text = Some(LegacyText { kind, gs: self.gs.clone(), hidden, ..LegacyText::default() });
    }

    /// `a b c d tx ty startPt Tp`: the text path begins.
    pub(super) fn begin_text_path(&mut self, vals: &[V]) {
        self.end_path_quietly();
        let n: Vec<f64> = vals.iter().filter_map(V::num).collect();
        let Some(t) = self.text.as_mut() else { return };
        if let Some(m) = n.get(..6).and_then(|m| <[f64; 6]>::try_from(m).ok()) {
            let m = Affine::new(m);
            let det = m.determinant();
            if m.as_coeffs().iter().all(|v| v.is_finite()) && det.is_finite() && det.abs() > 1e-12 {
                t.matrix = Some(m);
            }
        }
        t.start = n.get(6).copied().filter(|s| s.is_finite() && *s >= 0.0).unwrap_or(0.0);
        t.in_path = true;
    }

    /// The path painted (or `TP`, ending the text path): the text object's frame or path.
    pub(super) fn text_path(&mut self) {
        let path = std::mem::take(&mut self.path);
        self.end_path_quietly();
        if let Some(t) = self.text.as_mut()
            && t.path.is_none()
            && !path.elements().is_empty()
        {
            t.path = Some(path);
        }
    }

    /// `TP`.
    pub(super) fn end_text_path(&mut self) {
        self.text_path();
        if let Some(t) = self.text.as_mut() {
            t.in_path = false;
        }
    }

    /// A character or paragraph attribute.
    pub(super) fn text_attr(&mut self, w: &str, vals: &[V]) {
        let st = &mut self.type_state;
        let n0 = num(vals, 0);
        match w {
            "Tr" => st.render = n0.map_or(0, |v| v as i64),
            "Tf" => {
                if let Some(name) = vals.iter().find_map(|v| if let V::Name(n) = v { Some(n.clone()) } else { None }) {
                    let name = st
                        .fonts
                        .iter()
                        .find(|(new, _)| *new == name)
                        .map_or_else(|| name.trim_start_matches('_').to_string(), |(_, old)| old.clone());
                    let (family, style) = super::super::super::text::family_style(&name);
                    st.style.font_family = family;
                    st.style.font_style = style;
                    st.style.font_version = None;
                }
                if let Some(size) = num(vals, 0).filter(|s| *s > 0.0) {
                    st.style.size = size.min(MAX_SIZE);
                }
            }
            "Ts" => st.style.baseline_shift = n0.unwrap_or(0.0).clamp(-MAX_SIZE, MAX_SIZE),
            "Tz" => {
                let scale = |v: f64| v.clamp(1.0, 10_000.0);
                st.style.h_scale = n0.map_or(100.0, scale);
                st.style.v_scale = num(vals, 1).map_or(100.0, scale);
            }
            "Tt" => st.style.tracking = n0.unwrap_or(0.0).clamp(-10_000.0, 10_000.0),
            "TA" => st.style.kerning = if n0 == Some(0.0) { Some(0.0) } else { None },
            "Tl" => {
                if let Some(l) = n0 {
                    let l = l.clamp(0.0, MAX_SIZE);
                    // The automatic leading, 120% of the size, stays automatic.
                    st.style.leading = if (l - st.style.size * 1.2).abs() < 1e-3 { None } else { Some(l) };
                }
                st.para.space_before = num(vals, 1).unwrap_or(0.0).clamp(-MAX_SIZE, MAX_SIZE);
            }
            "Ta" => {
                st.para.justify = match n0.map(|v| v as i64) {
                    Some(1) => Justify::Center,
                    Some(2) => Justify::Right,
                    Some(3) => Justify::JustifyLeft,
                    Some(4) => Justify::JustifyAll,
                    _ => Justify::Left,
                }
            }
            "Ti" => {
                let indent = |i: usize| num(vals, i).unwrap_or(0.0).clamp(-MAX_SIZE, MAX_SIZE);
                (st.para.first_line_indent, st.para.left_indent, st.para.right_indent) = (indent(0), indent(1), indent(2));
            }
            _ => {}
        }
    }

    /// `(…) Tx` (`Tj`, overflow `TX`): characters in the current style and paint.
    pub(super) fn text_body(&mut self, w: &str, vals: &[V]) {
        let Some(bytes) = vals.iter().rev().find_map(|v| if let V::Str(s) = v { Some(s.clone()) } else { None }) else { return };
        let st = &mut self.type_state;
        let Some(t) = self.text.as_mut() else { return };
        let decoded: String = bytes
            .iter()
            .map(|&b| if b == b'\r' { '\n' } else { st.encoding.get(usize::from(b)).copied().flatten().unwrap_or_else(|| latin::decode(b)) })
            .collect();
        if decoded.is_empty() {
            return;
        }
        // Overflow text ending the story with a paragraph break (`(\r) TX`) adds no paragraph.
        if w == "TX" && decoded == "\n" {
            t.pending_break = true;
            return;
        }
        if st.chars >= MAX_CHARS {
            return;
        }
        st.chars += decoded.chars().count();
        let gs = &self.gs;
        let mut style = st.style.clone();
        let (fill, stroke) = match st.render {
            0 | 4 => (true, false),
            1 | 5 => (false, true),
            2 | 6 => (true, true),
            _ => (false, false),
        };
        style.fill = if fill { gs.fill.clone() } else { Paint::None };
        style.stroke = if stroke { gs.stroke.clone() } else { Paint::None };
        style.stroke_width = if stroke { gs.width } else { 0.0 };
        (style.stroke_cap, style.stroke_join, style.stroke_miter_limit, style.stroke_dash) = (gs.cap, gs.join, gs.miter, gs.dash.clone());
        (style.overprint_fill, style.overprint_stroke) = (gs.overprint_fill, gs.overprint_stroke);
        let mut text = String::new();
        if std::mem::take(&mut t.pending_break) {
            text.push('\n');
            t.para_taken = false;
        }
        for c in decoded.chars() {
            if c != '\n' && !t.para_taken {
                t.paras.push(st.para.clone());
                t.para_taken = true;
            }
            text.push(c);
            if c == '\n' {
                if !t.para_taken {
                    t.paras.push(st.para.clone());
                }
                t.para_taken = false;
            }
        }
        match t.runs.last_mut() {
            Some(last) if last.style == style => last.text.push_str(&text),
            _ => t.runs.push(TextRun::new(text, style)),
        }
    }

    /// `TO`: the text object ends.
    pub(super) fn end_text(&mut self, hidden: bool) -> Result<(), String> {
        let Some(t) = self.text.take() else { return Ok(()) };
        if t.runs.is_empty() {
            return Ok(());
        }
        let m = t.matrix.unwrap_or(Affine::IDENTITY);
        let to_doc = self.space();
        // Art space → text space (y down).
        let to_text = FLIP * m.inverse();
        let kind = match (t.kind, &t.path) {
            (1, Some(p)) => TextKind::Area { frame: PathData::from_bezpath(&(to_text * p.clone())) },
            (2, Some(p)) => TextKind::OnPath { path: PathData::from_bezpath(&(to_text * p.clone())), start: start_fraction(p, t.start), end: None },
            _ => TextKind::Point,
        };
        let mut obj = TextObject::point(kurbo::Point::ZERO, "", CharStyle::default());
        obj.kind = kind;
        obj.xf = to_doc * m * FLIP;
        obj.runs = t.runs;
        let mut paras = t.paras;
        if paras.is_empty() {
            paras.push(self.type_state.para.clone());
        }
        obj.set_paragraph_styles(paras);
        let node = self.new_node(NodeKind::Text(Box::new(obj)), &t.gs);
        self.add(node, t.hidden || hidden)
    }
}

/// Where type on `path` starts (`index.fraction`: a segment and how far along it, §15.5.2), as a
/// fraction of its length.
fn start_fraction(path: &BezPath, start: f64) -> f64 {
    const ACCURACY: f64 = 1e-3;
    let segments: Vec<kurbo::PathSeg> = path.segments().collect();
    let lengths: Vec<f64> = segments.iter().map(|s| s.arclen(ACCURACY)).collect();
    let total: f64 = lengths.iter().sum();
    if !(total.is_finite() && total > 0.0 && start.is_finite()) {
        return 0.0;
    }
    let index = start.floor().max(0.0) as usize;
    let Some(seg) = segments.get(index) else { return 1.0 };
    let before: f64 = lengths.iter().take(index).sum();
    let along = seg.subsegment(0.0..(start - start.floor()).clamp(0.0, 1.0)).arclen(ACCURACY);
    ((before + along) / total).clamp(0.0, 1.0)
}
