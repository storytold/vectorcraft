//! Type in the legacy format (version 7 specification §15): text objects `type To` … `TO`, their
//! text paths (`a b c d tx ty startPt Tp`, an area's frame, one for each linked area, or the path
//! type runs along, `TP`), the modal character and paragraph attributes
//! (`Tr Tf Ts Tz Tt TA Tl Ta Ti`, and §15.4.7 and §22's letter and word spacing `TC TW`, hanging
//! punctuation `Tq` and hyphenation `Th`), the paint style, and the characters (`Tx`, `Tj`,
//! overflow `TX`; `\r` ends a paragraph). Final-form operators (the
//! computed positions, §15.1) are skipped: the type is laid out again. Strings are decoded by the
//! encoding pairs of the current font's `TZ`, then by the file's `TE` encoding unless that `TZ`'s
//! `useDefault` is 0, else as Windows-1252; a re-encoded font `/_name` is the font its `%AI3_BeginEncoding` or `TZ` names
//! (else `name`).

use kurbo::{Affine, BezPath, ParamCurve, ParamCurveArclen};
use vectorcraft_color::Paint;
use vectorcraft_doc::{CharStyle, Justify, NodeKind, ParaStyle, TextKind, TextObject, TextRun};
use vectorcraft_geom::AnchorKind;

use super::super::obj::V;
use super::art::path_data;
use super::{FLIP, GState, HashSet, Node, NodeId, Reader};
use crate::latin;

/// Most characters of type read in one file.
const MAX_CHARS: usize = 1 << 22;
/// Largest type size and indent read (points).
const MAX_SIZE: f64 = 1e4;
/// Most re-encoded fonts read in one file, for their names and for their encodings each.
const MAX_FONTS: usize = 4096;
/// Most values read before one operator in a re-encoding section.
const MAX_ENCODING_VALUES: usize = 1024;
/// Most text paths read in one text object (its linked areas); the frames of others are left out,
/// with a warning.
const MAX_TEXT_PATHS: usize = 4096;
/// Most frames of linked areas times characters of their story: the story is laid out again
/// through its frames when the file opens. The frames of the areas past it are left out, with a
/// warning.
const MAX_THREAD_WORK: usize = MAX_CHARS;
/// The warning when [`MAX_TEXT_PATHS`] or [`MAX_THREAD_WORK`] leaves frames of linked areas out.
const LINKED_AREAS_LEFT_OUT: &str = "linked type areas past VectorCraft's limits were left out";

/// A text path (`Tp` … `TP`).
#[derive(Debug, Default)]
struct TextPath {
    /// `Tp`'s matrix: text space (y up) → art space.
    matrix: Option<Affine>,
    /// Where type on a path starts: a segment's index plus how far along it.
    start: f64,
    /// The frame or path, in art space, and the kind of the anchor each of its segments ends on.
    path: Option<(BezPath, Vec<AnchorKind>)>,
}

/// A text object being read.
#[derive(Debug, Default)]
pub(super) struct LegacyText {
    /// `To`'s type: 0 point type, 1 area type, 2 type on a path.
    kind: i64,
    /// Its text paths in file order: area type has one for each linked area (§15.5.3).
    paths: Vec<TextPath>,
    /// Between `Tp` and `TP`: the path read is the text path.
    in_path: bool,
    /// It has more than [`MAX_TEXT_PATHS`] text paths: the paths read past them are left out.
    past_limit: bool,
    /// The story: the characters after every text path, in file order.
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
    /// Re-encoded fonts' original names (`%AI3_BeginEncoding: _new old`, or `TZ`'s).
    fonts: Vec<(String, String)>,
    /// Re-encoded fonts' own encodings (`TZ`), by the new font's name: what its encoding pairs
    /// change, by code (None where they say nothing), and whether `TE`'s encoding applies to it.
    font_encodings: Vec<(String, Vec<Option<char>>, bool)>,
    /// The font `Tf` set last, as the file names it.
    font: Option<String>,
    chars: usize,
}

impl Default for TypeState {
    fn default() -> Self {
        Self {
            style: CharStyle::default(),
            para: ParaStyle::default(),
            render: 0,
            encoding: vec![None; 256],
            fonts: vec![],
            font_encodings: vec![],
            font: None,
            chars: 0,
        }
    }
}

impl TypeState {
    /// `%AI3_BeginEncoding: _new old` (`line` the comment).
    pub(super) fn encoding_comment(&mut self, line: &str) {
        let Some((_, names)) = line.split_once(':') else { return };
        let mut words = names.split_whitespace();
        if let (Some(new), Some(old)) = (words.next(), words.next()) {
            self.font_names(new.trim_start_matches('/'), old.trim_start_matches('/'));
        }
    }

    /// Re-encoded font `new` is font `old`. The first statement for a name is used.
    fn font_names(&mut self, new: &str, old: &str) {
        if self.fonts.len() < MAX_FONTS && !self.fonts.iter().any(|(n, _)| n == new) {
            self.fonts.push((new.to_string(), old.to_string()));
        }
    }

    /// A string's bytes, in the current font.
    fn decode(&self, bytes: &[u8]) -> String {
        let own = self.font.as_ref().and_then(|f| self.font_encodings.iter().find(|(n, _, _)| n == f));
        let (own, te) = own.map_or((None, true), |(_, e, te)| (Some(e.as_slice()), *te));
        bytes
            .iter()
            .map(|&b| {
                if b == b'\r' {
                    return '\n';
                }
                own.and_then(|e| e.get(usize::from(b)).copied().flatten())
                    .or_else(|| if te { self.encoding.get(usize::from(b)).copied().flatten() } else { None })
                    .unwrap_or_else(|| latin::decode(b))
            })
            .collect()
    }
}

/// Encoding pairs (§3.3.1): codes, each followed by the glyph names from it on, into `table`.
fn encoding_pairs(table: &mut [Option<char>], vals: &[V]) {
    let mut code: Option<usize> = None;
    for v in vals {
        match v {
            V::Num(n) if n.is_finite() && (0.0..256.0).contains(n) => code = Some(*n as usize),
            V::Name(name) => {
                if let Some(c) = code {
                    if let (Some(slot), Ok(b)) = (table.get_mut(c), u8::try_from(c)) {
                        *slot = latin::glyph(b, name);
                    }
                    code = Some(c + 1);
                }
            }
            _ => {}
        }
    }
}

/// An operator's `N` operands, the last `N` values before it, when each is a finite number.
fn nums<const N: usize>(vals: &[V]) -> Option<[f64; N]> {
    let operands = vals.get(vals.len().checked_sub(N)?..)?;
    let mut out = [0.0; N];
    for (o, v) in out.iter_mut().zip(operands) {
        *o = v.num().filter(|n| n.is_finite())?;
    }
    Some(out)
}

/// A flag operand: 0 or 1.
fn flag(v: f64) -> Option<bool> {
    match v {
        0.0 => Some(false),
        1.0 => Some(true),
        _ => None,
    }
}

impl Reader<'_> {
    /// `[encodingPairs TE`: codes, each followed by the glyph names from it on (§3.3.1).
    pub(super) fn te(&mut self, vals: &[V]) {
        encoding_pairs(&mut self.type_state.encoding, vals);
    }

    /// `[encodingPairs newFontName oldFontName direction fontScript useDefault TZ`, with a Multiple
    /// Master weight vector `[w0 … wn]` after it, and without the encoding pairs or the direction
    /// in some versions (§3.3.1): the new font's own encoding pairs, which strings in it are
    /// decoded by first. `useDefault` 1 (or no number after the names) decodes the codes the pairs
    /// leave out by `TE`'s encoding, and 0 leaves `TE`'s encoding out for the font. The font names
    /// are the last two names; only numbers and the weight vector follow them. The first `TZ` for
    /// a font is used.
    pub(super) fn tz(&mut self, vals: &[V]) {
        let mut names = vals.iter().enumerate().filter_map(|(i, v)| if let V::Name(n) = v { Some((i, n)) } else { None }).rev();
        let (Some((old_at, old)), Some((new_at, new))) = (names.next(), names.next()) else { return };
        if new_at + 1 != old_at {
            return;
        }
        let tail = vals.get(old_at + 1..).unwrap_or_default();
        let weights = matches!(tail.last(), Some(V::Arr(_)));
        let numbers = tail.get(..tail.len() - usize::from(weights)).unwrap_or_default();
        if !numbers.iter().all(|v| matches!(v, V::Num(_))) {
            return;
        }
        let te = !matches!(numbers.last(), Some(V::Num(n)) if *n == 0.0);
        let st = &mut self.type_state;
        st.font_names(new, old);
        if st.font_encodings.iter().any(|(n, _, _)| n == new) || st.font_encodings.len() >= MAX_FONTS {
            return;
        }
        // The pairs start after the `[` that opens them.
        let pairs = vals.get(..new_at).unwrap_or_default();
        let pairs = pairs.iter().rposition(|v| matches!(v, V::Mark)).map_or(pairs, |m| pairs.get(m + 1..).unwrap_or_default());
        let mut table = vec![None; 256];
        encoding_pairs(&mut table, pairs);
        st.font_encodings.push((new.clone(), table, te));
    }

    /// What is between `%AI3_BeginEncoding` and `%AI3_EndEncoding` (§3.3): its `TZ` is read, and
    /// every other operator is skipped with its operands.
    pub(super) fn encoding_section(&mut self, section: &[u8]) {
        let mut lex = super::Lexer::new(section);
        let mut vals: Vec<V> = Vec::new();
        let mut marks: Vec<usize> = Vec::new();
        // `[`s past the value limit, which are not kept: as many `]`s after them are skipped.
        let mut dropped: usize = 0;
        while let Some(t) = lex.next_token() {
            let v = match t.tok {
                super::Tok::Num(n) => V::Num(n),
                super::Tok::Name(n) => V::Name(super::obj::text(n)),
                super::Tok::Str(s) | super::Tok::Hex(s) => V::Str(s),
                super::Tok::Word(b"[") => {
                    // Each `[` kept is a value, so the open arrays are no more than the values.
                    if vals.len() >= MAX_ENCODING_VALUES {
                        dropped = dropped.saturating_add(1);
                        continue;
                    }
                    marks.push(vals.len());
                    V::Mark
                }
                super::Tok::Word(b"]") => {
                    if dropped > 0 {
                        dropped -= 1;
                        continue;
                    }
                    let Some(m) = marks.pop() else { continue };
                    let items: Vec<V> = vals.split_off(m.min(vals.len())).into_iter().skip(1).collect();
                    V::Arr(items)
                }
                super::Tok::Word(w) => {
                    if w == b"TZ" {
                        self.tz(&vals);
                    }
                    vals.clear();
                    marks.clear();
                    dropped = 0;
                    continue;
                }
                _ => continue,
            };
            if vals.len() < MAX_ENCODING_VALUES {
                vals.push(v);
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
        if self.pattern.is_some() {
            self.unreadable("type in a pattern", false);
        }
        let kind = nums::<1>(vals).map_or(0, |[k]| k as i64);
        self.text = Some(LegacyText { kind, gs: self.gs.clone(), hidden, ..LegacyText::default() });
    }

    /// `a b c d tx ty startPt Tp`: the text path begins.
    pub(super) fn begin_text_path(&mut self, vals: &[V]) {
        self.end_path_quietly();
        let Some(t) = self.text.as_mut() else { return };
        // Seven operands, or the matrix alone.
        let (m, start) = match vals.len() {
            7 => (vals.get(..6).and_then(nums::<6>), vals.get(6).and_then(V::num)),
            6 => (nums::<6>(vals), None),
            _ => (None, None),
        };
        let matrix = m.map(Affine::new).filter(|m| {
            let det = m.determinant();
            det.is_finite() && det.abs() > 1e-12
        });
        let start = start.filter(|s| s.is_finite() && *s >= 0.0).unwrap_or(0.0);
        t.past_limit |= t.paths.len() >= MAX_TEXT_PATHS;
        if !t.past_limit {
            t.paths.push(TextPath { matrix, start, path: None });
        }
        t.in_path = true;
    }

    /// The path painted (or `TP`, ending the text path): the text path's frame or path.
    pub(super) fn text_path(&mut self) {
        let path = std::mem::take(&mut self.path);
        let kinds = std::mem::take(&mut self.path_kinds);
        self.end_path_quietly();
        if let Some(p) = self.text.as_mut().filter(|t| !t.past_limit).and_then(|t| t.paths.last_mut())
            && p.path.is_none()
            && !path.elements().is_empty()
        {
            p.path = Some((path, kinds));
        }
    }

    /// `TP`.
    pub(super) fn end_text_path(&mut self) {
        self.text_path();
        if let Some(t) = self.text.as_mut() {
            t.in_path = false;
        }
    }

    /// A character or paragraph attribute. An attribute whose operands are missing, are not
    /// numbers, or are flags other than 0 and 1 stays as it was.
    pub(super) fn text_attr(&mut self, w: &str, vals: &[V]) {
        let st = &mut self.type_state;
        let n0 = nums::<1>(vals).map(|[v]| v);
        match w {
            "Tr" => {
                if let Some(v) = n0 {
                    st.render = v as i64;
                }
            }
            "Tf" => {
                // `/name size Tf`, or `/name size ascent descent Tf`.
                let Some(at) = vals.iter().position(|v| matches!(v, V::Name(_))) else { return };
                if let Some(V::Name(name)) = vals.get(at) {
                    st.font = Some(name.clone());
                    let name =
                        st.fonts.iter().find(|(new, _)| new == name).map_or_else(|| name.trim_start_matches('_').to_string(), |(_, old)| old.clone());
                    let (family, style) = super::super::super::text::family_style(&name);
                    st.style.font_family = family;
                    st.style.font_style = style;
                    st.style.font_version = None;
                }
                if let Some(size) = vals.get(at + 1).and_then(V::num).filter(|s| s.is_finite() && *s > 0.0) {
                    st.style.size = size.min(MAX_SIZE);
                }
            }
            "Ts" => {
                if let Some(v) = n0 {
                    st.style.baseline_shift = v.clamp(-MAX_SIZE, MAX_SIZE);
                }
            }
            "Tz" => {
                // `parallelScale perpendicularScale Tz`; before version 7, `scale Tz`.
                let scale = |v: f64| v.clamp(1.0, 10_000.0);
                let scales = match vals.len() {
                    2 => nums::<2>(vals),
                    1 => n0.map(|h| [h, 100.0]),
                    _ => None,
                };
                if let Some([h, v]) = scales {
                    (st.style.h_scale, st.style.v_scale) = (scale(h), scale(v));
                }
            }
            "Tt" => {
                if let Some(v) = n0 {
                    st.style.tracking = v.clamp(-10_000.0, 10_000.0);
                }
            }
            "TA" => {
                if let Some(auto) = n0.and_then(flag) {
                    st.style.kerning = if auto { None } else { Some(0.0) };
                }
            }
            "Tl" => {
                // `leading paragraphLeading Tl`; before version 7, `leading Tl`.
                let leading = match vals.len() {
                    2 => nums::<2>(vals),
                    1 => n0.map(|l| [l, 0.0]),
                    _ => None,
                };
                if let Some([l, before]) = leading {
                    let l = l.clamp(0.0, MAX_SIZE);
                    // The automatic leading, 120% of the size, stays automatic.
                    st.style.leading = if (l - st.style.size * 1.2).abs() < 1e-3 { None } else { Some(l) };
                    st.para.space_before = before.clamp(-MAX_SIZE, MAX_SIZE);
                }
            }
            "Ta" => {
                let justify = match n0 {
                    Some(0.0) => Some(Justify::Left),
                    Some(1.0) => Some(Justify::Center),
                    Some(2.0) => Some(Justify::Right),
                    Some(3.0) => Some(Justify::JustifyLeft),
                    Some(4.0) => Some(Justify::JustifyAll),
                    _ => None,
                };
                if let Some(j) = justify {
                    st.para.justify = j;
                }
            }
            "Ti" => {
                if let Some(i) = nums::<3>(vals) {
                    let [first, left, right] = i.map(|v| v.clamp(-MAX_SIZE, MAX_SIZE));
                    (st.para.first_line_indent, st.para.left_indent, st.para.right_indent) = (first, left, right);
                }
            }
            // `min opt max TC` (letter spacing) and `min opt max TW` (word spacing), in percent of
            // a space's width, as the paragraph's Justification options take them.
            "TC" | "TW" => {
                if let Some(v) = nums::<3>(vals) {
                    let j = &mut st.para.justification;
                    if w == "TC" {
                        j.letter_spacing = v;
                    } else {
                        j.word_spacing = v;
                    }
                    j.keep_in_range();
                }
            }
            // `hangingQuotes Tq`.
            "Tq" => {
                if let Some(on) = n0.and_then(flag) {
                    st.para.roman_hanging = on;
                }
            }
            // `hyphenate limitHyphLines minLeadHyphen minTailHyphen maxHyphLines Th`, kept to the
            // ranges `text.setFormat` takes.
            "Th" => {
                let Some([on, limit, lead, tail, max]) = nums::<5>(vals) else { return };
                let (Some(on), Some(limit)) = (flag(on), flag(limit)) else { return };
                let letters = |v: f64, lo: f64, hi: f64| v.round().clamp(lo, hi) as u32;
                st.para.hyphenate = on;
                let h = &mut st.para.hyphenation;
                h.after_first = letters(lead, 1.0, 15.0);
                h.before_last = letters(tail, 1.0, 15.0);
                h.hyphen_limit = if limit { letters(max, 0.0, 25.0) } else { 0 };
            }
            _ => {}
        }
    }

    /// `(…) Tx` (`Tj`, overflow `TX`): characters in the current style and paint.
    pub(super) fn text_body(&mut self, w: &str, vals: &[V]) {
        let Some(bytes) = vals.iter().rev().find_map(|v| if let V::Str(s) = v { Some(s.clone()) } else { None }) else { return };
        let st = &mut self.type_state;
        let Some(t) = self.text.as_mut() else { return };
        let decoded = st.decode(&bytes);
        if decoded.is_empty() {
            return;
        }
        // Overflow text ending the story with a paragraph break (`(\r) TX`) adds no paragraph.
        if w == "TX" && decoded == "\n" {
            t.pending_break = true;
            return;
        }
        // As many characters as the file's allowance has left.
        let room = MAX_CHARS.saturating_sub(st.chars);
        if room == 0 {
            return;
        }
        let decoded: String = decoded.chars().take(room).collect();
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
        style.stroke_width = if stroke { gs.line_width() } else { 0.0 };
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

    /// `TO`: the text object ends. Area type with several linked areas becomes one frame for each
    /// area, threaded in file order: the first frame holds the story, and the flow from area to
    /// area (final-form, §15.1) is laid out again. The frames are added last to first, so the
    /// object's dictionary that follows names the first one ([`Reader::each_linked_frame`]).
    /// Point type is read at its first text path; area type and type on a path at the first one
    /// that has a frame or path (else at the first one, as point type).
    pub(super) fn end_text(&mut self, hidden: bool) -> Result<(), String> {
        let Some(t) = self.text.take() else { return Ok(()) };
        if t.runs.is_empty() {
            return Ok(());
        }
        let to_doc = self.space();
        let chars: usize = t.runs.iter().map(|r| r.text.chars().count()).sum();
        let most = (MAX_THREAD_WORK / chars.max(1)).max(1);
        let all_areas = t.paths.iter().filter(|p| p.path.is_some()).count();
        let areas: Vec<&TextPath> = t.paths.iter().filter(|p| p.path.is_some()).take(most).collect();
        let placed: Vec<(Affine, TextKind)> = match t.kind {
            1 if areas.len() > 1 => areas.into_iter().map(|p| placed(1, Some(p))).collect(),
            0 => vec![placed(0, t.paths.first())],
            kind => vec![placed(kind, areas.first().copied().or(t.paths.first()))],
        };
        if t.kind == 1 && (t.past_limit || all_areas > placed.len()) {
            self.warn(LINKED_AREAS_LEFT_OUT);
        }
        let mut paras = t.paras;
        if paras.is_empty() {
            paras.push(self.type_state.para.clone());
        }
        let last_para = paras.last().cloned().unwrap_or_default();
        let last_style = t.runs.last().map(|r| r.style.clone()).unwrap_or_default();
        let mut story = Some((t.runs, paras));
        let mut nodes = Vec::with_capacity(placed.len());
        for (m, kind) in placed {
            let mut obj = TextObject::point(kurbo::Point::ZERO, "", last_style.clone());
            obj.kind = kind;
            obj.xf = to_doc * m * FLIP;
            match story.take() {
                Some((runs, paras)) => {
                    obj.runs = runs;
                    obj.set_paragraph_styles(paras);
                }
                None => obj.set_paragraph_styles(vec![last_para.clone()]),
            }
            nodes.push(self.new_node(NodeKind::Text(Box::new(obj)), &t.gs));
        }
        let frames: Vec<NodeId> = nodes.iter().map(|n| n.id).collect();
        for node in nodes.into_iter().rev() {
            self.add(node, t.hidden || hidden)?;
        }
        if frames.len() > 1 {
            self.doc.text_threads.push(frames);
        }
        Ok(())
    }

    /// Call `f` on the other frames of the threaded text object just read, whose first frame is
    /// the last object ([`Reader::end_text`]): the opacity mask and text wrap of the object's
    /// dictionary apply to every frame.
    pub(super) fn each_linked_frame(&mut self, mut f: impl FnMut(&mut Node)) {
        if !self.after_object {
            return;
        }
        let Some(first) = self.frames.last().and_then(|fr| fr.children.last()).map(|n| n.id) else { return };
        let Some(thread) = self.doc.text_threads.last().filter(|t| t.first() == Some(&first)) else { return };
        let rest: HashSet<NodeId> = thread.iter().skip(1).copied().collect();
        let Some(fr) = self.frames.last_mut() else { return };
        for n in fr.children.iter_mut().rev().skip(1).take(rest.len()).filter(|n| rest.contains(&n.id)) {
            f(n);
        }
    }
}

/// A text object of type `kind` on text path `p` (none: at the origin): the matrix from text space
/// (y up) to art space, and the kind of type with its frame or path in text space (y down).
fn placed(kind: i64, p: Option<&TextPath>) -> (Affine, TextKind) {
    let m = p.and_then(|p| p.matrix).unwrap_or(Affine::IDENTITY);
    // Art space → text space (y down).
    let to_text = FLIP * m.inverse();
    let kind = match (kind, p.and_then(|p| p.path.as_ref().map(|path| (path, p.start)))) {
        (1, Some(((path, kinds), _))) => TextKind::Area { frame: path_data(&(to_text * path.clone()), kinds) },
        (2, Some(((path, kinds), start))) => {
            TextKind::OnPath { path: path_data(&(to_text * path.clone()), kinds), start: start_fraction(path, start), end: None }
        }
        _ => TextKind::Point,
    };
    (m, kind)
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
