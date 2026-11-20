use super::grid::{clip_segment, pref_color};
use super::outline::{OVERLAY_MAX_ANCHORS, c32, stroke_path};
use super::{Color32, CornerRadius, Node, NodeKind, Overlay, Point, Pos2, Rect, Shape, Stroke, StrokeKind, Tokens, VectorcraftApp, Xf, pos2, vec2};

/// Type → Show Hidden Characters: the characters that print no ink, and the ends of paragraphs,
/// lines and stories ([`HiddenMarks`]). The marks of each text object are kept from one frame to
/// the next ([`HiddenChars`]).
pub(super) fn hidden_chars_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let clip = p.clip_rect();
    let color = Color32::from_rgb(0x4f, 0x9d, 0xff);
    let stroke = Stroke::new(1.0, color);
    let font = egui::FontId::proportional(((10.0 * xf.zoom) as f32).clamp(7.0, 18.0));
    let kept = &mut app.canvas.hidden_chars;
    // `last` holds the marks of the last frame. The marks of the objects this frame doesn't draw
    // are dropped at the end.
    let mut last = std::mem::take(&mut kept.marks);
    // The inputs are read before any layout: when fonts load during this frame, the next frame
    // lays all type out again.
    let inputs = (vectorcraft_text::FontDb::global().generation(), vectorcraft_text::hyphenation_exceptions());
    if kept.inputs != inputs {
        last.clear();
        kept.inputs = inputs;
    }
    // `walk` visits each node's `Arc` in the order `Document::walk` visits the nodes (depth first,
    // in paint order).
    fn walk(nodes: &[std::sync::Arc<Node>], f: &mut impl FnMut(&std::sync::Arc<Node>)) {
        for n in nodes {
            f(n);
            if let Some(children) = n.children() {
                walk(children, f);
            }
        }
    }
    walk(&st.doc.layers, &mut |n| {
        let NodeKind::Text(tx) = &n.kind else { return };
        // The box round the object's corners on screen, turned with the view.
        if !n.visible || n.geometric_bounds().is_none_or(|b| !egui::Rect::from_points(&xf.quad(b)).intersects(clip)) {
            return;
        }
        let marks = match last.remove(&n.id) {
            Some((was, marks)) if std::sync::Arc::ptr_eq(&was, n) => marks,
            _ => std::sync::Arc::new(HiddenMarks::of(tx)),
        };
        for &at in &marks.spaces {
            p.circle_filled(xf.to_screen(at), 1.2, color);
        }
        for &(mark, a, b) in &marks.chars {
            draw_char_mark(p, mark, xf.to_screen(a), xf.to_screen(b), stroke, &font);
        }
        for &(at, along, up) in &marks.breaks {
            let at = xf.to_screen(at);
            let u = unit_or(xf.to_screen(along) - at, vec2(1.0, 0.0));
            let n = unit_or(xf.to_screen(up) - at, vec2(0.0, -1.0));
            // A bent arrow: down from above the line's end, then back along the line.
            let corner = at + u * 7.0 + n * 2.0;
            let tip = at + u + n * 2.0;
            p.add(Shape::line(vec![at + u * 7.0 + n * 8.0, corner, tip], stroke));
            p.add(Shape::line(vec![tip + u * 3.0 + n * 3.0, tip, tip + u * 3.0 - n * 3.0], stroke));
        }
        for &(at, glyph, align) in &marks.ends {
            p.text(xf.to_screen(at), align, glyph, font.clone(), color);
        }
        kept.marks.insert(n.id, (n.clone(), marks));
    });
}

/// `v` scaled to length 1, or `or` when `v` has no length.
fn unit_or(v: egui::Vec2, or: egui::Vec2) -> egui::Vec2 {
    let len = v.length();
    if len > 1e-3 { v / len } else { or }
}

/// Draws the mark of one character on screen. `a` and `b` are the start and end of the
/// character's advance, in the direction the text reads, above its baseline.
fn draw_char_mark(p: &egui::Painter, mark: CharMark, a: Pos2, b: Pos2, stroke: Stroke, font: &egui::FontId) {
    let u = unit_or(b - a, vec2(1.0, 0.0));
    let mid = a + (b - a) * 0.5;
    match mark {
        CharMark::Tab => {
            // An arrow across the tab's advance, at least 6 px long.
            let end = if (b - a).length() < 6.0 { a + u * 6.0 } else { b };
            let n = vec2(u.y, -u.x);
            p.line_segment([a, end], stroke);
            p.add(Shape::line(vec![end - u * 3.0 + n * 2.5, end, end - u * 3.0 - n * 2.5], stroke));
        }
        CharMark::NoBreakSpace => {
            p.circle_stroke(mid, 2.0, stroke);
        }
        CharMark::NarrowNoBreakSpace => {
            p.circle_stroke(mid, 1.4, stroke);
        }
        CharMark::FixedSpace => {
            // A short tick across the baseline's direction.
            let n = vec2(u.y, -u.x) * 2.5;
            p.line_segment([mid - n, mid + n], stroke);
        }
        CharMark::DiscretionaryHyphen => {
            p.text(mid, egui::Align2::CENTER_BOTTOM, "¬", font.clone(), stroke.color);
        }
    }
}

/// The Show Hidden Characters marks of the text objects drawn in the last frame
/// ([`crate::CanvasCache::hidden_chars`]). Each is kept with the object it was laid out from: an
/// edited object is a new `Arc` and is laid out again, and all type is laid out again when the
/// fonts or the hyphenation exceptions change.
#[derive(Default)]
pub struct HiddenChars {
    /// The fonts' generation ([`vectorcraft_text::FontDb::generation`]) and the hyphenation
    /// exceptions the marks were laid out with.
    pub(super) inputs: (u64, String),
    /// Each text object drawn, by id: the node it was laid out from and its marks.
    pub(super) marks: std::collections::HashMap<vectorcraft_doc::NodeId, (std::sync::Arc<Node>, std::sync::Arc<HiddenMarks>)>,
}

/// The mark Show Hidden Characters draws over a character other than a plain space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CharMark {
    /// A tab (U+0009): an arrow across its advance, to the tab stop.
    Tab,
    /// A nonbreaking space (U+00A0): a ring.
    NoBreakSpace,
    /// A narrow nonbreaking space (U+202F): a smaller ring.
    NarrowNoBreakSpace,
    /// The spaces of a set width (U+2000 to U+200A, U+205F, U+3000): a short tick.
    FixedSpace,
    /// A discretionary hyphen (U+00AD): ¬, where the word can break.
    DiscretionaryHyphen,
}

impl CharMark {
    /// The mark of `c`, `None` for a character with no mark of its own.
    pub(super) fn of(c: char) -> Option<Self> {
        Some(match c {
            '\t' => Self::Tab,
            '\u{00A0}' => Self::NoBreakSpace,
            '\u{202F}' => Self::NarrowNoBreakSpace,
            '\u{2000}'..='\u{200A}' | '\u{205F}' | '\u{3000}' => Self::FixedSpace,
            '\u{00AD}' => Self::DiscretionaryHyphen,
            _ => return None,
        })
    }
}

/// One text object's marks in document space: a dot over each space, a [`CharMark`] over each
/// tab, nonbreaking space, space of a set width and discretionary hyphen, a bent arrow after
/// each line a forced line break (U+2028) ends, and ¶ or # after the lines that end a paragraph
/// or the story. Type on a path has the marks over characters, ¶ and the bent arrow at its
/// breaks, and # after its last character when all of its text is drawn.
#[derive(Default)]
pub(super) struct HiddenMarks {
    pub(super) spaces: Vec<Point>,
    /// Each mark over a character, with the start and end of the character's advance 3 pt above
    /// its baseline, in the direction the text reads.
    pub(super) chars: Vec<(CharMark, Point, Point)>,
    /// Each forced line break: where its arrow starts, a point 1 pt further along the line and a
    /// point 1 pt above the baseline from there.
    pub(super) breaks: Vec<(Point, Point, Point)>,
    /// ¶ and #, where they are drawn and how the text is aligned to that point.
    pub(super) ends: Vec<(Point, &'static str, egui::Align2)>,
}

impl HiddenMarks {
    /// Lay out `tx` and place its marks.
    pub(super) fn of(tx: &vectorcraft_doc::TextObject) -> Self {
        let mut marks = Self::default();
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx);
        let text = tx.plain_text();
        let to = |q: Point| tx.xf * q;
        // The direction a glyph's baseline runs, and up from it (y points down).
        let along = |angle: f64| vectorcraft_geom::Vec2::new(angle.cos(), angle.sin());
        let up = |angle: f64| {
            let d = along(angle);
            vectorcraft_geom::Vec2::new(d.y, -d.x)
        };
        // A list marker's glyphs and a hyphen the layout adds at a break (length 0) stand for no
        // character. Glyphs before the start of an open path draw nothing. A character drawn with
        // several glyphs gets one mark.
        let mut marked = None;
        for g in lay.glyphs.iter().filter(|g| g.len > 0 && !g.off_path) {
            let Some(c) = text.get(g.byte..).and_then(|s| s.chars().next()) else { continue };
            if marked == Some(g.byte) {
                continue;
            }
            let (d, n) = (along(g.angle), up(g.angle));
            let start = g.origin + n * 3.0;
            let end = start + d * g.advance;
            if c == ' ' {
                marks.spaces.push(to(start + d * (g.advance / 2.0)));
            } else if let Some(mark) = CharMark::of(c) {
                let (a, b) = if g.rtl { (end, start) } else { (start, end) };
                marks.chars.push((mark, to(a), to(b)));
            } else if lay.on_path && (c == '\n' || c == '\u{2028}') {
                // Type on a path is one line: its breaks are marked where their glyphs are.
                if c == '\n' {
                    marks.ends.push((to(g.origin), "¶", egui::Align2::LEFT_BOTTOM));
                } else {
                    marks.breaks.push((to(g.origin), to(g.origin + d), to(g.origin + d + n)));
                }
            } else {
                continue;
            }
            marked = Some(g.byte);
        }
        if lay.on_path {
            // # after the last character, when the path holds all of the text.
            let last = lay.glyphs.iter().filter(|g| g.len > 0 && !g.off_path).max_by_key(|g| g.byte + g.len);
            if let Some(g) = last.filter(|g| g.byte + g.len >= text.len() && !lay.overflow) {
                let d = along(g.angle);
                let at = if g.rtl { g.origin - d } else { g.origin + d * (g.advance + 1.0) };
                marks.ends.push((to(at), "#", egui::Align2::LEFT_BOTTOM));
            }
            return marks;
        }
        // Lines are in line space: `physical_point` turns them into columns in vertical type.
        let at = |x: f64, y: f64| to(lay.physical_point(Point::new(x, y)));
        for (i, line) in lay.lines.iter().enumerate() {
            let last = i + 1 == lay.lines.len();
            // A right-to-left line ends at its left edge.
            let (x, dx) = if line.rtl { (line.x0 - 1.0, -1.0) } else { (line.x1 + 1.0, 1.0) };
            let align = match (lay.vertical, line.rtl) {
                (true, _) => egui::Align2::CENTER_TOP,
                (false, true) => egui::Align2::RIGHT_BOTTOM,
                (false, false) => egui::Align2::LEFT_BOTTOM,
            };
            let soft = text.get(line.start..line.end).is_some_and(|s| s.ends_with('\u{2028}'));
            if last && line.end >= text.len() {
                marks.ends.push((at(x, line.baseline), "#", align));
            } else if soft {
                marks.breaks.push((at(x, line.baseline), at(x + dx, line.baseline), at(x, line.baseline - 1.0)));
            } else if text.get(line.end..).is_some_and(|s| s.starts_with('\n')) {
                marks.ends.push((at(x, line.baseline), "¶", align));
            }
        }
        marks
    }
}

/// Edit › Spelling › Auto Spell Check: a red wavy line under each word of the visible type its
/// dictionaries don't know ([`vectorcraft_engine::cmd::spelling::misspellings`], what
/// `text.spelling.check` lists), in the run's language, without the active document's Ignore All
/// words. Type with effects isn't underlined
/// ([Check and correct spelling](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/check-and-correct-spelling.html)).
/// Each text object's lines are kept from one frame to the next ([`SpellingMarks`]).
pub(super) fn spelling_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let kept = &mut app.canvas.spelling;
    let Some(st) = app.session.active() else {
        *kept = SpellingMarks::default();
        return;
    };
    let clip = p.clip_rect();
    // `last` holds the lines of the last frame; the objects this frame doesn't draw are dropped.
    let mut last = std::mem::take(&mut kept.lines);
    let inputs = SpellingInputs {
        fonts: vectorcraft_text::FontDb::global().generation(),
        hyphenation: vectorcraft_text::hyphenation_exceptions(),
        dictionary: vectorcraft_engine::cmd::spelling::dictionary_generation(),
        doc: st.uid,
        // Ignore All only adds words while the document is open.
        ignored: st.spelling_ignored.len(),
    };
    if kept.inputs.as_ref() != Some(&inputs) {
        last.clear();
        kept.inputs = Some(inputs);
    }
    let stroke = Stroke::new(1.0, Color32::from_rgb(0xe0, 0x20, 0x20));
    // Depth first in paint order, leaving out hidden objects and what is inside them.
    fn walk(nodes: &[std::sync::Arc<Node>], f: &mut impl FnMut(&std::sync::Arc<Node>)) {
        for n in nodes.iter().filter(|n| n.visible) {
            f(n);
            if let Some(children) = n.children() {
                walk(children, f);
            }
        }
    }
    walk(&st.doc.layers, &mut |n| {
        let NodeKind::Text(tx) = &n.kind else { return };
        if !auto_spell_checks(n) || n.geometric_bounds().is_none_or(|b| !egui::Rect::from_points(&xf.quad(b)).intersects(clip)) {
            return;
        }
        let lines = match last.remove(&n.id) {
            Some((was, lines)) if std::sync::Arc::ptr_eq(&was, n) => lines,
            _ => std::sync::Arc::new(underlines(tx, &st.spelling_ignored)),
        };
        for &(a, b) in lines.iter() {
            wavy_line(p, xf.to_screen(a), xf.to_screen(b), stroke);
        }
        kept.lines.insert(n.id, (n.clone(), lines));
    });
}

/// Whether Auto Spell Check marks `n`'s words: not with an effect shown (the Appearance panel's
/// eye) on the type or on one of its fills or strokes, as the helpx page says "Auto spell check
/// isn't available for text with effects applied".
fn auto_spell_checks(n: &Node) -> bool {
    let shown = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible);
    !shown(&n.appearance.effects) && !n.appearance.items.iter().any(|i| shown(i.effects()))
}

/// The word with Auto Spell Check's red line at `p` (document space): in the topmost visible type
/// object whose glyphs `p` is on, when that object isn't locked and Auto Spell Check marks it and
/// its word at `p` is one it underlines, with what its language's dictionary suggests instead. For the context menu's suggestions, Ignore All and Add
/// to Dictionary ([Check and correct spelling](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/check-and-correct-spelling.html)).
pub(crate) fn misspelled_word_at(app: &mut VectorcraftApp, p: Point) -> Option<crate::state::ContextSpelling> {
    let st = app.session.active()?;
    // The topmost type whose glyphs `p` is on, and whether it or a layer or group holding it is
    // locked: a word under other type isn't the one right-clicked, whether that type's word there
    // is right or not.
    let mut top: Option<(vectorcraft_doc::NodeId, bool)> = None;
    // Back to front as drawn at `p`: an Intertwine group's members by their tier there, those of
    // one tier in their stacking order ([`vectorcraft_doc::intertwine`]).
    fn walk(nodes: &[std::sync::Arc<Node>], p: Point, locked: bool, f: &mut impl FnMut(&Node, bool)) {
        for n in nodes.iter().filter(|n| n.visible) {
            let locked = locked || n.locked;
            f(n, locked);
            let Some(children) = n.children() else { continue };
            if matches!(n.kind, NodeKind::Intertwine { .. }) {
                let mut drawn = children.to_vec();
                drawn.sort_by_key(|c| vectorcraft_doc::intertwine::tier_at(c, p));
                walk(&drawn, p, locked, f);
            } else {
                walk(children, p, locked, f);
            }
        }
    }
    // A later hit is on top.
    walk(&st.doc.layers, p, false, &mut |n, locked| {
        let NodeKind::Text(tx) = &n.kind else { return };
        if n.geometric_bounds().is_none_or(|b| !b.contains(p)) {
            return;
        }
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx);
        let on = vectorcraft_text::selection_quads(&lay, 0, tx.plain_text().len()).iter().any(|q| in_quad(q.map(|c| tx.xf * c), p));
        if on {
            top = Some((n.id, locked));
        }
    });
    let (id, locked) = top?;
    let n = st.doc.node(id)?;
    let NodeKind::Text(tx) = &n.kind else { return None };
    if locked || !auto_spell_checks(n) {
        return None;
    }
    let (words, _) = vectorcraft_engine::cmd::spelling::misspellings(tx, &st.spelling_ignored, MAX_UNDERLINED);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx);
    let w = words
        .into_iter()
        .find(|w| vectorcraft_text::selection_quads(&lay, w.range.start, w.range.end).iter().any(|q| in_quad(q.map(|c| tx.xf * c), p)))?;
    let word = tx.plain_text().get(w.range.clone())?.to_string();
    let uid = st.uid;
    let suggestions = app
        .session
        .execute("text.spelling.suggest", &serde_json::json!({ "word": word, "lang": w.lang.tag() }))
        .ok()
        .and_then(|r| r.get("suggestions").and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()))
        .unwrap_or_default();
    Some(crate::state::ContextSpelling { uid, id, start: w.range.start, end: w.range.end, word, suggestions })
}

/// Whether `p` is inside the convex quad `q` (its corners in order, either way round) or on its
/// edge.
fn in_quad(q: [Point; 4], p: Point) -> bool {
    let sides = (0..4).map(|i| {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
    });
    let sides: Vec<f64> = sides.collect();
    sides.iter().all(|s| *s >= 0.0) || sides.iter().all(|s| *s <= 0.0)
}

/// The most words underlined in one text object: past it the rest of the object isn't checked,
/// so a long story of unknown words costs one check's worth.
const MAX_UNDERLINED: usize = 1_000;

/// The document-space segments under the unknown words of `tx`: for each word, the bottom edge of
/// each piece of it on a line, from the start of the line to its end.
fn underlines(tx: &vectorcraft_doc::TextObject, ignored: &std::collections::BTreeSet<String>) -> Vec<(Point, Point)> {
    let (words, _) = vectorcraft_engine::cmd::spelling::misspellings(tx, ignored, MAX_UNDERLINED);
    if words.is_empty() {
        return vec![];
    }
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx);
    words
        .iter()
        .flat_map(|w| vectorcraft_text::selection_quads(&lay, w.range.start, w.range.end))
        // Clockwise from the top left: q[3] and q[2] are the bottom left and right.
        .map(|q| (tx.xf * q[3], tx.xf * q[2]))
        .collect()
}

/// A wavy line from `a` to `b` on screen, waves 4 pixels long and 1.5 high.
fn wavy_line(p: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    const STEP: f32 = 2.0;
    const HIGH: f32 = 1.5;
    let d = b - a;
    let len = d.length();
    if !len.is_finite() || len < 1.0 {
        return;
    }
    let along = d / len;
    // Down the page from the line, whichever way the type runs.
    let across = vec2(-along.y, along.x);
    // At most 2000 waves on one line, however far the view is zoomed in.
    let n = ((len / STEP) as usize).clamp(1, 4_000);
    let step = len / n as f32;
    let points: Vec<Pos2> = (0..=n).map(|i| a + along * (step * i as f32) + across * if i % 2 == 0 { 0.0 } else { HIGH }).collect();
    p.add(Shape::line(points, stroke));
}

/// What Auto Spell Check's kept lines depend on beyond the text object itself: when any of it
/// changes, all type is checked again.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SpellingInputs {
    /// The fonts' generation ([`vectorcraft_text::FontDb::generation`]).
    fonts: u64,
    /// The hyphenation exceptions, which move line breaks.
    hyphenation: String,
    /// The custom dictionary's generation
    /// ([`vectorcraft_engine::cmd::spelling::dictionary_generation`]).
    dictionary: u64,
    /// The document the lines are of.
    doc: u64,
    /// How many Ignore All words the document has.
    ignored: usize,
}

/// The lines Auto Spell Check drew under each text object in the last frame
/// ([`crate::CanvasCache::spelling`]). Each is kept with the object it was found in: an edited
/// object is a new `Arc` and is checked again.
#[derive(Default)]
pub struct SpellingMarks {
    pub(super) inputs: Option<SpellingInputs>,
    /// Each text object drawn, by id: the node it was checked in and its lines in document space.
    pub(super) lines: std::collections::HashMap<vectorcraft_doc::NodeId, (std::sync::Arc<Node>, Underlines)>,
}

/// One text object's Auto Spell Check lines: the document-space ends of each.
pub(super) type Underlines = std::sync::Arc<Vec<(Point, Point)>>;

/// View › Show Text Threads: for each thread with a selected frame, a line in its first frame's
/// layer colour from each frame's out port to the next frame's in port, both showing the arrow of a
/// threaded port, where the tools draw the ports of selected type ([`vectorcraft_tools::text`]).
/// Only frames with ports ([`vectorcraft_tools::text::ported`]) are joined: a link to or from a
/// frame in perspective, which has none, gets no line. With more than
/// [`vectorcraft_tools::text::PORTS_MAX`] such frames selected the tools draw no ports, and only
/// the lines are drawn here. `zoom`: screen pixels to the document unit.
pub(super) fn thread_overlays(app: &VectorcraftApp, zoom: f64) -> Vec<Overlay> {
    use std::collections::HashSet;
    use vectorcraft_doc::NodeId;
    use vectorcraft_tools::text::{Port, PortState, port_centre, ported, ports_shown, push_port};
    let Some(st) = app.session.active() else { return vec![] };
    let px = 1.0 / zoom.max(1e-9);
    let boxes = ports_shown(&st.doc, &st.selection.objects);
    let selection = &st.selection.objects;
    let mut selected: Option<HashSet<NodeId>> = None;
    // The first two full searches need no index.
    let mut full_searches = 0;
    let mut is_selected = |id: &NodeId| {
        if selection.len() <= 1 {
            return selection.contains(id);
        }
        if full_searches == 0 {
            full_searches = 1;
            return selection.contains(id);
        }
        // Two early selection hits need no index.
        if selected.is_none() && selection[..2].contains(id) {
            return true;
        }
        if selected.is_none() && full_searches < 2 {
            full_searches += 1;
            return selection.contains(id);
        }
        selected.get_or_insert_with(|| selection.iter().copied().collect::<HashSet<_>>()).contains(id)
    };
    let mut o = vec![];
    for thread in &st.doc.text_threads {
        if !thread.iter().any(&mut is_selected) {
            continue;
        }
        let color = st.doc.layer_color(thread[0]);
        for w in thread.windows(2) {
            let [a, b] = [w[0], w[1]];
            let (Some(from), Some(to)) = (ported(&st.doc, a), ported(&st.doc, b)) else { continue };
            let (Some(out), Some(input)) = (port_centre(from, Port::Out, px), port_centre(to, Port::In, px)) else { continue };
            o.push(Overlay::Line { a: out, b: input, color, dashed: false });
            if boxes {
                push_port(&mut o, out, PortState::Threaded, st.doc.layer_color(a), px);
                push_port(&mut o, input, PortState::Threaded, st.doc.layer_color(b), px);
            }
        }
    }
    o
}

/// View → Show Live Paint Gaps: the gaps of the selected Live Paint groups in each group's Gap
/// Preview Color ([`crate::dialogs::live_paint_gaps::selected_gaps`]). Cached per revision.
pub(super) fn live_paint_gap_overlays(app: &mut VectorcraftApp) -> Vec<Overlay> {
    let Some(st) = app.session.active() else { return vec![] };
    let key = (st.uid, st.revision);
    let gaps = match &app.canvas.live_paint_gaps {
        Some((k, gaps)) if *k == key => gaps.clone(),
        _ => {
            let gaps = std::sync::Arc::new(crate::dialogs::live_paint_gaps::selected_gaps(app));
            app.canvas.live_paint_gaps = Some((key, gaps.clone()));
            gaps
        }
    };
    gaps.to_vec()
}

/// Simplify's Show Original Path: the paths its preview ran on, as they were before it, in a band
/// 3 screen points wide of their layer's color at 40%, under the selection's outlines. Nothing is drawn
/// when they have more than [`OVERLAY_MAX_ANCHORS`] anchor points.
pub(super) fn original_path_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let paths = crate::dialogs::path_ops::original_paths(app);
    let Some(st) = app.session.active().filter(|_| paths.iter().map(|(_, path)| path.anchor_count()).sum::<usize>() <= OVERLAY_MAX_ANCHORS) else {
        return;
    };
    for (id, path) in paths {
        stroke_path(p, &path.to_bezpath(), xf, Stroke::new(3.0, c32(st.doc.layer_color(id)).gamma_multiply(0.4)));
    }
}

/// The 9-slice guides of the symbol edited in symbol editing mode: four thin dotted lines across
/// its art's box, with any tool, whether ruler guides are shown or hidden. The Selection tool drags
/// them ([`vectorcraft_tools::nineslice`]) at the places drawn here.
pub(super) fn nine_slice_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(e) = app.session.active().and_then(|st| st.doc.edited_nine_slice()) else { return };
    let color = Tokens::get(p.ctx()).selection;
    for (_, a, b) in e.lines() {
        let (a, b) = (xf.to_screen(a), xf.to_screen(b));
        if !a.is_finite() || !b.is_finite() {
            continue;
        }
        if let Some((a, b)) = clip_segment(a, b, xf.rect) {
            p.extend(Shape::dashed_line(&[a, b], Stroke::new(1.0, color), 2.0, 2.0));
        }
    }
}

/// The slices (unless View → Hide Slices): user and object slices outlined in the Slices
/// preferences' line colour, auto slices dashed and dimmer, each numbered at its top left (Show
/// Slice Numbers); selected slices have a bolder outline. The layout is cached per revision.
pub(super) fn slice_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let key = (st.uid, st.revision);
    let slices = match &app.canvas.slices {
        Some((k, s)) if *k == key => s.clone(),
        _ => {
            let s = std::sync::Arc::new(st.doc.slice_layout());
            app.canvas.slices = Some((key, s.clone()));
            s
        }
    };
    if slices.is_empty() {
        return;
    }
    let prefs = &app.session.prefs;
    let line = pref_color(&prefs.slice_line_color, Color32::from_rgb(0xff, 0x3f, 0x3f));
    let dim = line.gamma_multiply(0.55);
    let selected = &st.selection.slices;
    let objects = &st.selection.objects;
    for a in slices.iter() {
        let auto = a.source == vectorcraft_doc::SliceSource::Auto;
        let on = a.id.is_some_and(|id| selected.contains(&id) || objects.contains(&id));
        let q = xf.quad(a.rect);
        let ring: Vec<Pos2> = q.iter().chain(q.first()).copied().collect();
        if auto {
            p.extend(Shape::dashed_line(&ring, Stroke::new(1.0, dim), 3.0, 3.0));
        } else {
            p.add(Shape::line(ring, Stroke::new(if on { 2.0 } else { 1.0 }, line)));
        }
        if prefs.show_slice_numbers
            && let Some(tl) = q.first()
        {
            let font = egui::FontId::proportional(9.0);
            let galley = p.layout_no_wrap(format!("{:02}", a.number), font, Color32::WHITE);
            let badge = egui::Rect::from_min_size(*tl + vec2(1.0, 1.0), galley.size() + vec2(6.0, 2.0));
            p.rect_filled(badge, CornerRadius::ZERO, if auto { dim } else { line });
            p.galley(badge.min + vec2(3.0, 1.0), galley, Color32::WHITE);
        }
    }
}

/// The print tiling (View → Show Print Tiling, or the Print Tiling tool): each page of the
/// document's print settings as `print.preview` lays it out, its paper edge and its imageable
/// area dashed, numbered at its top left; tiles outside the tile range dimmer. Cached per revision.
pub(super) fn print_tiling_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let key = (st.uid, st.revision);
    let pages = match &app.canvas.print_tiling {
        Some((k, pages)) if *k == key => pages.clone(),
        _ => {
            // Settings that can't print have no pages to show.
            let pages = std::sync::Arc::new(vectorcraft_engine::cmd::printtiling::pages(&st.doc).unwrap_or_default());
            app.canvas.print_tiling = Some((key, pages.clone()));
            pages
        }
    };
    let font = egui::FontId::proportional(10.0);
    let ring = |r: [f64; 4]| {
        let q = xf.quad(Rect::new(r[0], r[1], r[2], r[3]));
        q.iter().chain(q.first()).copied().collect::<Vec<Pos2>>()
    };
    for page in pages.iter() {
        let color = if page.printed { PRINT_TILING } else { PRINT_TILING.gamma_multiply(0.45) };
        p.add(Shape::line(ring(page.page), Stroke::new(0.75, color)));
        let inner = ring(page.imageable);
        p.extend(Shape::dashed_line(&inner, Stroke::new(1.0, color), 4.0, 3.0));
        if let Some(corner) = inner.iter().copied().reduce(|a, b| pos2(a.x.min(b.x), a.y.min(b.y))) {
            p.text(corner + vec2(3.0, 2.0), egui::Align2::LEFT_TOP, page.number.to_string(), font.clone(), color);
        }
    }
}

/// The print tiling's lines: a neutral grey that reads on the paper and on the pasteboard.
const PRINT_TILING: Color32 = Color32::from_gray(96);

/// A Shaper Group's mode widget (Shaper and Selection tools), drawn with selection edges shown or
/// hidden: an arrow pointing right in Face Selection mode and left in Construction mode.
pub(super) fn shaper_widget_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    if matches!(app.session.tool_id(), "shaper" | "selection")
        && let Some(w) = vectorcraft_tools::shaperwidget::ShaperWidget::of(&st.doc, &st.selection, st.isolation, xf.zoom)
    {
        let (c, r) = (xf.to_screen(w.at), vectorcraft_tools::shaperwidget::RADIUS_PX);
        let color = c32(st.doc.layer_color(w.group));
        p.circle_filled(c, r, Color32::WHITE);
        p.circle_stroke(c, r, Stroke::new(1.0, color));
        let d = if w.construction { -1.0 } else { 1.0 };
        let arrow = vec![c + vec2(3.0 * d, 0.0), c + vec2(-2.0 * d, -3.0), c + vec2(-2.0 * d, 3.0)];
        p.add(Shape::convex_polygon(arrow, color, Stroke::NONE));
    }
}

/// High-contrast live controls, using the same geometry as the Selection tool's hit testing.
pub(super) fn mirror_axis_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    if app.session.tool_id() != "selection" || !app.ui.view.bounding_box {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(axis) = vectorcraft_tools::mirror::MirrorAxis::of(&st.doc, &st.selection, xf.zoom) else { return };
    let t = Tokens::get(p.ctx());
    let ends = axis.ends.map(|q| xf.to_screen(q));
    let center = xf.to_screen(axis.center);
    if !center.is_finite() || !ends.iter().all(|p| p.is_finite()) {
        return;
    }
    if let Some((a, b)) = clip_segment(ends[0], ends[1], xf.rect) {
        p.line_segment([a, b], Stroke::new(5.0, t.measure_bg));
        p.line_segment([a, b], Stroke::new(2.0, t.selection));
    }
    for end in ends {
        p.circle_filled(end, vectorcraft_tools::mirror::HANDLE_RADIUS, t.measure_bg);
        p.circle_stroke(end, vectorcraft_tools::mirror::HANDLE_RADIUS, Stroke::new(2.0, t.selection));
    }
    let handle = egui::Rect::from_center_size(center, vec2(12.0, 12.0));
    p.rect_filled(handle, 2.0, t.selection);
    p.rect_stroke(handle, 2.0, Stroke::new(2.0, t.measure_bg), StrokeKind::Outside);
}

/// The Split view's divider of the selected Image Trace object, where the Selection tool's hit
/// testing finds it: a line across the source image with a grip in its middle, at its place in the
/// zoomed, scrolled or turned view. Hidden with the bounding box and in Outline mode, as the hit
/// testing is.
pub(super) fn trace_split_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    if app.session.tool_id() != "selection" || !app.ui.view.bounding_box || app.outline_view() {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(d) = vectorcraft_tools::trace_split::TraceDivider::of(&st.doc, &st.selection, xf.zoom) else { return };
    let ends = d.ends.map(|q| xf.to_screen(q));
    if !ends.iter().all(|q| q.is_finite()) {
        return;
    }
    let t = Tokens::get(p.ctx());
    if let Some((a, b)) = clip_segment(ends[0], ends[1], xf.rect) {
        p.line_segment([a, b], Stroke::new(4.0, t.measure_bg));
        p.line_segment([a, b], Stroke::new(1.5, t.selection));
    }
    let along = ends[1] - ends[0];
    let length = along.length();
    if length > 1.0 {
        let mid = ends[0] + along * 0.5;
        let half = along / length * vectorcraft_tools::trace_split::GRIP_HALF.min(length * 0.5);
        p.line_segment([mid - half, mid + half], Stroke::new(7.0, t.measure_bg));
        p.line_segment([mid - half, mid + half], Stroke::new(4.0, t.selection));
    }
}

/// A selected grid repeat's row, column and corner handles and its spacing sliders (#2691), at
/// the places the Selection tool's hit testing finds them.
pub(super) fn grid_repeat_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    use vectorcraft_tools::repeat_grid::{GridHandles, GridPart, HANDLE_RADIUS, SLIDER_HALF};
    if app.session.tool_id() != "selection" || !app.ui.view.bounding_box {
        return;
    }
    let frame = app.selection_box();
    let Some(st) = app.session.active() else { return };
    let Some(grid) = GridHandles::of(&st.doc, &st.selection, frame, xf.zoom) else { return };
    let t = Tokens::get(p.ctx());
    let half = f64::from(SLIDER_HALF) / xf.zoom;
    for (part, at) in grid.parts() {
        let c = xf.to_screen(at);
        if !c.is_finite() {
            continue;
        }
        if part.is_slider() {
            // A bar across the gap's middle, square to the spacing it sets.
            let along = if part == GridPart::HSpacing { vectorcraft_geom::Vec2::new(0.0, half) } else { vectorcraft_geom::Vec2::new(half, 0.0) };
            let ends = [xf.to_screen(at - along), xf.to_screen(at + along)];
            p.line_segment(ends, Stroke::new(6.0, t.measure_bg));
            p.line_segment(ends, Stroke::new(2.5, t.selection));
        } else {
            p.circle_filled(c, HANDLE_RADIUS, t.measure_bg);
            p.circle_stroke(c, HANDLE_RADIUS, Stroke::new(2.0, t.selection));
        }
    }
}

/// A selected radial repeat's circle with its radius handle and its splitter on it, and its
/// instances handle below it, at the places the Selection tool's hit testing finds them. The
/// splitter also has a short line across the circle.
pub(super) fn radial_repeat_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    use vectorcraft_tools::repeat_radial::{HANDLE_RADIUS, RadialHandles, RadialPart};
    if app.session.tool_id() != "selection" || !app.ui.view.bounding_box {
        return;
    }
    let frame = app.selection_box();
    let Some(st) = app.session.active() else { return };
    let Some(h) = RadialHandles::of(&st.doc, &st.selection, frame, xf.zoom) else { return };
    let t = Tokens::get(p.ctx());
    // The circle the instances' centers sit on, while it is a sensible size on screen.
    let (c, r) = (xf.to_screen(h.center), (h.radius * xf.zoom) as f32);
    if c.is_finite() && r.is_finite() && r > 0.5 && r < 20_000.0 {
        p.circle_stroke(c, r, Stroke::new(3.0, t.measure_bg));
        p.circle_stroke(c, r, Stroke::new(1.0, t.selection));
    }
    for (part, at) in h.parts() {
        let c = xf.to_screen(at);
        let across = (c - xf.to_screen(h.center)).normalized() * (HANDLE_RADIUS + 4.0);
        if part == RadialPart::Splitter && c.is_finite() && across.is_finite() {
            p.line_segment([c - across, c + across], Stroke::new(3.0, t.measure_bg));
            p.line_segment([c - across, c + across], Stroke::new(1.5, t.selection));
        }
        if c.is_finite() {
            p.circle_filled(c, HANDLE_RADIUS, t.measure_bg);
            p.circle_stroke(c, HANDLE_RADIUS, Stroke::new(2.0, t.selection));
        }
    }
}

/// The radius of the Rotate View dial, in screen points.
pub(super) const DIAL_RADIUS: f32 = 40.0;

/// The dial a Rotate View drag shows, centered on `c`: a tick every 15° (longer every 90°), a
/// needle pointing where the document's top points in a view turned by `rotation` degrees, and the
/// angle under the center, rounded to whole degrees as in the status bar.
pub(super) fn rotate_view_dial(p: &egui::Painter, c: Pos2, rotation: f64, t: &Tokens) {
    // `dir(deg)` is the screen direction `deg` degrees clockwise from straight up.
    let dir = |deg: f32| vec2(deg.to_radians().sin(), -deg.to_radians().cos());
    p.circle(c, DIAL_RADIUS, t.panel.gamma_multiply(0.85), Stroke::new(1.0, t.border));
    for i in 0..24u8 {
        let len = if i % 6 == 0 { 8.0 } else { 4.0 };
        let d = dir(f32::from(i) * 15.0);
        p.line_segment([c + d * (DIAL_RADIUS - len), c + d * DIAL_RADIUS], Stroke::new(1.0, t.text_dim));
    }
    p.line_segment([c, c + dir(rotation as f32) * DIAL_RADIUS], Stroke::new(2.0, t.accent));
    p.circle_filled(c, 2.5, t.accent);
    p.text(c + vec2(0.0, 8.0), egui::Align2::CENTER_TOP, format!("{rotation:.0}°"), egui::FontId::proportional(11.0), t.text);
}
