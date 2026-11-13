//! Tabs panel: tab-stop alignment, X position (Y in vertical type), leader and "align on", and a
//! ruler numbered in the General unit on which a click adds a stop, dragging moves one and dragging
//! it off the ruler deletes it. Ctrl-dragging a stop (Command on macOS) moves all the stops by the
//! same distance. A stop placed or dragged on the ruler lands on the nearest whole point. With Snap
//! to Unit on, or Shift held, it lands on the ruler's nearest tick.

use std::collections::HashSet;

use egui::{Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, ParaStyle, TabAlign, TabStop, TextKind, TextObject};

use super::{first_selected, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

const ALIGNS: [(TabAlign, &str); 4] = [(TabAlign::Left, "⇥"), (TabAlign::Center, "⇹"), (TabAlign::Right, "⇤"), (TabAlign::Decimal, ".⇥")];

/// The name of alignment `a`'s button: its tool tip and what screen readers call it. A line of
/// vertical type runs from top to bottom, so in vertical type a Left stop is named Top-Justified
/// and a Right stop is named Bottom-Justified.
pub(crate) fn align_name(a: TabAlign, vertical: bool) -> &'static str {
    match (a, vertical) {
        (TabAlign::Left, false) => tl!("Left-Justified Tab"),
        (TabAlign::Left, true) => tl!("Top-Justified Tab"),
        (TabAlign::Center, _) => tl!("Center-Justified Tab"),
        (TabAlign::Right, false) => tl!("Right-Justified Tab"),
        (TabAlign::Right, true) => tl!("Bottom-Justified Tab"),
        (TabAlign::Decimal, _) => tl!("Decimal-Justified Tab"),
    }
}

fn align_id(a: TabAlign) -> &'static str {
    match a {
        TabAlign::Left => "left",
        TabAlign::Center => "center",
        TabAlign::Right => "right",
        TabAlign::Decimal => "decimal",
    }
}

/// The panel body's margin and the ruler's own inset: the ruler's 0 is this far right of the panel.
const RULER_LEFT: f32 = 10.0 + 4.0;

/// The most ticks the ruler draws.
const MAX_TICKS: usize = 10_000;

/// The largest stop position `text.tabs.set` takes, in points.
const MAX_STOP: f64 = 100_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IndentMarker {
    Upper,
    Lower,
}

#[derive(Clone)]
struct IndentTarget {
    id: u64,
    start: usize,
    /// The left indent as laid out: the paragraph's own plus its list level's ([`Self::level`]).
    left: f64,
    first_line: f64,
    /// How much further in the paragraph's list level sets it than its own left indent.
    level: f64,
}

fn indent_target(id: NodeId, text: &TextObject, i: usize, start: usize) -> IndentTarget {
    let para = text.para_at(i);
    let left = para.laid_out_left_indent();
    IndentTarget { id: id.0, start, left, first_line: para.first_line_indent, level: left - para.left_indent }
}

fn append_indent_targets(node: &Node, range: std::ops::Range<usize>, targets: &mut Vec<IndentTarget>) {
    let NodeKind::Text(text) = &node.kind else { return };
    let plain = text.plain_text();
    let mut starts = vec![0];
    starts.extend(plain.bytes().enumerate().filter_map(|(i, c)| (c == b'\n').then_some(i + 1)));
    targets.extend(range.filter_map(|i| Some(indent_target(node.id, text, i, *starts.get(i)?))));
}

/// One target for each paragraph of `story` (its frames in thread order), in story order. Where
/// one frame ends and the next begins, the last paragraph of the one and the first of the next are
/// pieces of one story paragraph: a paragraph that runs on into the next frame, or an empty piece
/// after a frame's last paragraph break. Such a paragraph is targeted once, by its last piece with
/// text, the piece whose style the story keeps when it flows again. An empty paragraph takes its
/// style from its last piece when the story flows again, so it is targeted by that piece. Locked and
/// hidden frames are targeted like the others.
fn append_story_targets(doc: &Document, story: &[NodeId], targets: &mut Vec<IndentTarget>) {
    /// The part of a story paragraph in one frame: paragraph `index` there, starting at byte `start`.
    struct Piece<'a> {
        id: NodeId,
        text: &'a TextObject,
        index: usize,
        start: usize,
        has_text: bool,
    }
    // Each story paragraph's pieces, in thread order.
    let mut paragraphs: Vec<Vec<Piece<'_>>> = vec![];
    for frame in story.iter().filter_map(|id| doc.node(*id)) {
        let NodeKind::Text(text) = &frame.kind else { continue };
        let plain = text.plain_text();
        let mut start = 0;
        for (index, part) in plain.split('\n').enumerate() {
            let piece = Piece { id: frame.id, text, index, start, has_text: !part.is_empty() };
            start += part.len() + 1;
            match paragraphs.last_mut() {
                Some(last) if index == 0 => last.push(piece),
                _ => paragraphs.push(vec![piece]),
            }
        }
    }
    for pieces in &paragraphs {
        let pick = pieces.iter().rev().find(|p| p.has_text).or_else(|| pieces.last());
        if let Some(p) = pick {
            targets.push(indent_target(p.id, p.text, p.index, p.start));
        }
    }
}

/// Paragraphs touched by the Type selection (every paragraph of the story after Select All in
/// threaded text), or all paragraphs of selected text objects. A selected frame of a threaded story
/// selects the whole story, its locked and hidden frames included. Each paragraph of a threaded
/// story is listed once, in story order ([`append_story_targets`]).
fn indent_targets(app: &VectorcraftApp, node: &Node) -> Vec<IndentTarget> {
    let mut targets = Vec::new();
    if let Some((_, a, b)) = super::character::text_editing(app) {
        if let (Some(story), Some(st)) = (super::character::text_story(app), app.session.active()) {
            append_story_targets(&st.doc, &story, &mut targets);
        } else if let NodeKind::Text(text) = &node.kind {
            append_indent_targets(node, text.paragraphs_in(a, b), &mut targets);
        }
    } else if let Some(st) = app.session.active() {
        // The selected text objects and the text in selected groups, each once.
        let mut frames = Vec::new();
        let mut taken = HashSet::new();
        for selected in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
            selected.walk(&mut |child| {
                if matches!(child.kind, NodeKind::Text(_)) && taken.insert(child.id) {
                    frames.push(child.id);
                }
            });
        }
        let mut done = HashSet::new();
        for id in frames {
            if done.contains(&id) {
                continue;
            }
            let story = st.doc.story_of(id);
            done.extend(story.iter().copied());
            append_story_targets(&st.doc, &story, &mut targets);
        }
    }
    targets
}

fn marker_position(marker: IndentMarker, para: &ParaStyle) -> f64 {
    match marker {
        IndentMarker::Upper => para.left_indent + para.first_line_indent,
        IndentMarker::Lower => para.left_indent,
    }
}

fn bounded_marker_position(marker: IndentMarker, target: f64, span: f64, targets: &[IndentTarget], both: bool) -> Option<f64> {
    if !target.is_finite() || !span.is_finite() || span < 0.0 || targets.is_empty() {
        return None;
    }
    let (low, high) = match marker {
        IndentMarker::Upper => {
            (targets.iter().map(|p| p.left - 1296.0).fold(0.0_f64, f64::max), targets.iter().map(|p| p.left + 1296.0).fold(span, f64::min))
        }
        // A paragraph's own left indent stays within 0 to 1296 pt, its list level's indent added.
        IndentMarker::Lower if both => (
            targets.iter().map(|p| p.level.max(-p.first_line)).fold(0.0_f64, f64::max),
            targets.iter().map(|p| (span - p.first_line).min(p.level + 1296.0)).fold(span, f64::min),
        ),
        IndentMarker::Lower => (
            targets.iter().map(|p| p.level.max(p.left + p.first_line - 1296.0)).fold(0.0_f64, f64::max),
            targets.iter().map(|p| (p.left + p.first_line + 1296.0).min(p.level + 1296.0)).fold(span, f64::min),
        ),
    };
    (low <= high).then(|| target.clamp(low, high))
}

fn hit_marker(pos: egui::Pos2, rect: egui::Rect, para: &ParaStyle, span: f64, mixed_left: bool, to_x: impl Fn(f64) -> f32) -> Option<IndentMarker> {
    if pos.y < rect.top() || pos.y > rect.top() + 16.0 {
        return None;
    }
    let upper = (to_x(marker_position(IndentMarker::Upper, para).clamp(0.0, span)) - pos.x).abs() <= 6.0;
    let lower = !mixed_left && (to_x(marker_position(IndentMarker::Lower, para).clamp(0.0, span)) - pos.x).abs() <= 6.0;
    match (upper, lower) {
        (true, true) if pos.y >= rect.top() + 8.0 => Some(IndentMarker::Lower),
        (true, _) => Some(IndentMarker::Upper),
        (_, true) => Some(IndentMarker::Lower),
        _ => None,
    }
}

struct MarkerPaint<'a> {
    rect: egui::Rect,
    span: f64,
    para: &'a ParaStyle,
    mixed_left: bool,
    selected: Option<IndentMarker>,
    drag: Option<(IndentMarker, bool)>,
    preview: Option<(IndentMarker, f64, bool)>,
}

fn paint_markers(ui: &mut Ui, painter: &egui::Painter, tokens: &Tokens, view: MarkerPaint<'_>, to_x: impl Fn(f64) -> f32) {
    for marker in [IndentMarker::Upper, IndentMarker::Lower] {
        if marker == IndentMarker::Lower && view.mixed_left {
            continue;
        }
        let value = match view.preview {
            Some((IndentMarker::Upper, x, _)) if marker == IndentMarker::Upper => x,
            Some((IndentMarker::Lower, x, _)) if marker == IndentMarker::Lower => x,
            Some((IndentMarker::Lower, x, true)) if marker == IndentMarker::Upper => marker_position(marker, view.para) + x - view.para.left_indent,
            _ => marker_position(marker, view.para),
        };
        let x = to_x(value.clamp(0.0, view.span));
        let top = view.rect.top() + 1.0;
        let points = if marker == IndentMarker::Upper {
            vec![egui::pos2(x - 5.0, top), egui::pos2(x + 5.0, top), egui::pos2(x, top + 7.0)]
        } else {
            vec![egui::pos2(x - 5.0, top + 15.0), egui::pos2(x + 5.0, top + 15.0), egui::pos2(x, top + 8.0)]
        };
        let selected = view.selected == Some(marker) || view.drag.is_some_and(|(m, _)| m == marker);
        painter.add(egui::Shape::convex_polygon(points, tokens.text, Stroke::new(1.5, if selected { tokens.accent } else { tokens.text_dim })));
        let name = match marker {
            IndentMarker::Upper => tl!("First-line Left Indent"),
            IndentMarker::Lower => tl!("Left Indent"),
        };
        let hit =
            egui::Rect::from_center_size(egui::pos2(x, view.rect.top() + if marker == IndentMarker::Upper { 4.0 } else { 12.0 }), vec2(12.0, 8.0));
        let hover = ui.interact(hit, ui.scope_id().with(("tabs-indent", marker as u8)), Sense::hover());
        hover.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, name));
        hover.on_hover_text(name);
    }
}

/// Apply an absolute ruler position through paragraph-format commands. When the paragraphs need
/// values of their own, one command sets each paragraph's ([`set_each_indent`]), and threaded text
/// flows again once.
fn set_marker(app: &mut VectorcraftApp, marker: IndentMarker, target: f64, para: &ParaStyle, span: f64, targets: &[IndentTarget], both: bool) {
    if targets.is_empty() || !target.is_finite() {
        return;
    }
    let Some(target) = bounded_marker_position(marker, target, span, targets, both) else { return };
    if marker == IndentMarker::Upper {
        if targets.iter().all(|p| p.left == para.left_indent) {
            run_on_targets(app, targets, json!({"firstLineIndent": target - para.left_indent}));
        } else {
            let changes = targets.iter().map(|p| (p, None, Some(target - p.left))).collect();
            set_each_indent(app, tl!("First-line Left Indent"), changes);
        }
    } else {
        // The ruler shows each paragraph's left indent with its list level's added; the paragraph
        // keeps its own.
        let upper = marker_position(IndentMarker::Upper, para);
        let level = targets.first().map(|p| p.level).filter(|l| targets.iter().all(|p| p.level == *l));
        let params = match level {
            Some(level) if both => Some(json!({"leftIndent": target - level})),
            Some(level) if targets.iter().all(|p| p.left + p.first_line == upper) => {
                Some(json!({"leftIndent": target - level, "firstLineIndent": upper - target}))
            }
            _ => None,
        };
        if let Some(params) = params {
            run_on_targets(app, targets, params);
            return;
        }
        let changes = targets.iter().map(|p| (p, Some(target - p.level), (!both).then_some(p.left + p.first_line - target))).collect();
        set_each_indent(app, tl!("Left Indent"), changes);
    }
}

/// Run one `text.setFormat` with `params` on the paragraphs the Paragraph panel acts on while typing,
/// and with the Selection tool on every type object of `targets`, so a selected frame of a threaded
/// story sets the whole story ([`indent_targets`]).
fn run_on_targets(app: &mut VectorcraftApp, targets: &[IndentTarget], mut params: Value) {
    if super::character::text_editing(app).is_none() {
        let mut seen = HashSet::new();
        params["ids"] = json!(targets.iter().map(|p| p.id).filter(|id| seen.insert(*id)).collect::<Vec<_>>());
    }
    if let Err(e) = super::paragraph::run_on_paragraphs(app, "text.setFormat", params) {
        app.status(e);
    }
}

/// Give each paragraph its own left and first-line indents (None: as it is) in one `text.setFormat`
/// with `paragraphIndents`, one undo step named `label`. Paragraphs that keep their indents are
/// left out, and when every paragraph keeps them nothing is run.
fn set_each_indent(app: &mut VectorcraftApp, label: &str, changes: Vec<(&IndentTarget, Option<f64>, Option<f64>)>) {
    // As `text.setFormat` keeps a paragraph's indents, within 1296 pt either way.
    let bound = |v: Option<f64>| v.map(|v| v.clamp(-1296.0, 1296.0));
    let entries: Vec<Value> = changes
        .into_iter()
        .filter_map(|(p, left, first_line)| {
            let (left, first_line) = (bound(left), bound(first_line));
            let own_left = p.left - p.level;
            if left.is_none_or(|v| v == own_left) && first_line.is_none_or(|v| v == p.first_line) {
                return None;
            }
            let mut entry = json!({"id": p.id, "start": p.start});
            if let Some(v) = left {
                entry["leftIndent"] = json!(v);
            }
            if let Some(v) = first_line {
                entry["firstLineIndent"] = json!(v);
            }
            Some(entry)
        })
        .collect();
    if entries.is_empty() {
        return;
    }
    super::character::end_typing(app);
    let command = json!({"command": "text.setFormat", "params": {"paragraphIndents": entries}});
    if let Err(e) = app.run("command.batch", json!({"label": label, "commands": [command]})) {
        app.status(e);
    }
}

/// The text object, the stops of the active paragraph and the ruler span for panel tests.
#[cfg(test)]
fn current(app: &VectorcraftApp) -> Option<(Node, Vec<TabStop>, f64)> {
    current_para(app).map(|(n, para, span)| (n, para.tabs, span))
}

/// As [`current`], with all the attributes of the paragraph whose stops the panel shows, its left
/// indent as laid out ([`ParaStyle::laid_out_left_indent`]: its list level's indent added), where
/// the ruler shows it.
fn current_para(app: &VectorcraftApp) -> Option<(Node, ParaStyle, f64)> {
    let editing = super::character::text_editing(app);
    let n = match editing {
        Some((id, _, _)) => app.session.active()?.doc.node(id)?.clone(),
        None => first_selected(app)?,
    };
    let NodeKind::Text(t) = &n.kind else { return None };
    let para = editing.map_or(0, |(_, a, _)| t.paragraphs_in(a, a).start);
    let span = match &t.kind {
        TextKind::Area { frame } => frame.bounds().map_or(360.0, |b| b.width()),
        _ => 360.0,
    };
    let para = t.para_at(para);
    let para = ParaStyle { left_indent: para.laid_out_left_indent(), ..para.clone() };
    Some((n, para, span.max(72.0)))
}

/// [`above_text`], then note the active document and text `n`. A floating panel is
/// at least [`crate::panel_docking::PANEL_MIN_WIDTH`] wide, so for narrow text at a low zoom the
/// panel comes out wider than asked; the ruler then draws text `n` at [`screen_scale`] and runs
/// on past the text's width.
fn place_above_text(app: &mut VectorcraftApp, ctx: &egui::Context, n: &Node, span: f64) {
    if above_text(app, ctx, n, span).is_some() {
        set_pstate(ctx, "tabs-above", app.session.active().map(|doc| (doc.uid, n.id)));
    }
}

/// Screen points per point of text `n` at the canvas zoom, as [`above_text`] sizes the ruler:
/// across the frame for area type, the zoom for point type.
fn screen_scale(app: &VectorcraftApp, n: &Node, span: f64) -> Option<f64> {
    let xf = app.canvas_xf()?;
    let per = match &n.kind {
        NodeKind::Text(t) if matches!(t.kind, TextKind::Area { .. }) => f64::from(xf.rect_to_screen(n.geometric_bounds()?).width()) / span,
        _ => xf.zoom,
    };
    (per.is_finite() && per > 0.0).then_some(per)
}

/// Position Panel Above Text: float the panel just above text `n`, as wide as its ruler needs to
/// put 0 on the text's left edge and its marks at the canvas zoom, so each stop sits over the
/// column it sets (#729). `span`: the ruler's length in the text's points.
fn above_text(app: &mut VectorcraftApp, ctx: &egui::Context, n: &Node, span: f64) -> Option<()> {
    let xf = app.canvas_xf()?;
    let on_screen = xf.rect_to_screen(n.geometric_bounds()?);
    // Screen points per point of the text: across the frame for area type (so a scaled frame
    // lines up too), the zoom for point type.
    let per = match &n.kind {
        NodeKind::Text(t) if matches!(t.kind, TextKind::Area { .. }) => on_screen.width() / span as f32,
        _ => xf.zoom as f32,
    };
    if !per.is_finite() || per <= 0.0 {
        return None;
    }
    let width = span as f32 * per + RULER_LEFT * 2.0;
    let height = crate::panel_docking::layout_of(&app.ui)
        .floating
        .iter()
        .find(|w| w.contains(&"tabs".to_string()))
        .map_or(crate::panel_docking::floating_size("tabs").y, |w| w.rect[3]);
    // Text near the top of the window: the panel goes just below the text when that place is in
    // the window and the panel fits there in the canvas, else at the window's top, so its bar
    // stays where it can be grabbed rather than above the screen.
    let above = on_screen.top() - height - 6.0;
    let below = on_screen.bottom() + 6.0;
    let top = if above >= 0.0 {
        above
    } else if below >= 0.0 && below + height <= app.canvas_rect.map_or(f32::NEG_INFINITY, |c| c.bottom()) {
        below
    } else {
        0.0
    };
    let pos = egui::pos2(on_screen.left() - RULER_LEFT, top);
    let origin = ctx.input(|i| i.raw.viewports.get(&egui::ViewportId::ROOT).and_then(|v| v.inner_rect)).map(|r| r.min);
    crate::panel_docking::place_floating(app, "tabs", pos, width, origin).ok()?;
    Some(())
}

fn stops_json(stops: &[TabStop]) -> Value {
    Value::Array(
        stops
            .iter()
            .map(|s| json!({"position": s.position, "align": align_id(s.align), "leader": s.leader, "alignOn": s.align_on.to_string()}))
            .collect(),
    )
}

/// Run `text.tabs.set` with `stops` on the text the panel acts on. Returns false when it fails.
fn apply(app: &mut VectorcraftApp, stops: &[TabStop]) -> bool {
    if let Some((id, a, b)) = super::character::text_editing(app) {
        super::character::end_typing(app);
        let p = match super::character::text_story(app) {
            // After Select All in threaded text: every paragraph of every frame.
            Some(story) => json!({"stops": stops_json(stops), "ids": story.iter().map(|f| f.0).collect::<Vec<_>>()}),
            None => json!({"stops": stops_json(stops), "ids": [id.0], "start": a, "end": b}),
        };
        app.run("text.tabs.set", p).is_ok()
    } else {
        app.run("text.tabs.set", json!({"stops": stops_json(stops)})).is_ok()
    }
}

/// Move stop `i` to `position`, sort the stops by position and set them, then select the first
/// stop within 1e-6 pt of `position` on the ruler, the stop `text.tabs.set` keeps of those it
/// merges. Returns its index, or `None` when the stops are not set and `stops` and the selection
/// stay as they were.
fn apply_moved(app: &mut VectorcraftApp, ctx: &egui::Context, stops: &mut Vec<TabStop>, i: usize, position: f64) -> Option<usize> {
    let mut moved = stops.clone();
    moved.get_mut(i)?.position = position;
    moved.sort_by(|a, b| a.position.total_cmp(&b.position));
    let sel = moved.iter().position(|s| (s.position - position).abs() < 1e-6).unwrap_or(0);
    apply(app, &moved).then(|| {
        *stops = moved;
        set_pstate(ctx, "tabs-sel", sel);
        sel
    })
}

/// `stops` with every stop moved by the distance that takes stop `i` to `position`, or by less
/// when that would put a stop below 0 or past [`MAX_STOP`], so the leftmost stop ends at 0 or the
/// rightmost at [`MAX_STOP`] and the stops keep their spacing. The stops keep their order,
/// alignment, leader and "align on". `None` when there is no stop `i`.
fn moved_all(stops: &[TabStop], i: usize, position: f64) -> Option<Vec<TabStop>> {
    let from = stops.get(i)?.position;
    let lowest = stops.iter().map(|s| s.position).fold(f64::INFINITY, f64::min);
    let highest = stops.iter().map(|s| s.position).fold(f64::NEG_INFINITY, f64::max);
    let offset = (position - from).min(MAX_STOP - highest).max(-lowest);
    Some(stops.iter().map(|s| TabStop { position: s.position + offset, ..s.clone() }).collect())
}

/// Move every stop as [`moved_all`] does, in one `text.tabs.set`, and select stop `i`. Returns its
/// index, or `None` when the stops are not set and `stops` and the selection stay as they were.
fn apply_moved_all(app: &mut VectorcraftApp, ctx: &egui::Context, stops: &mut Vec<TabStop>, i: usize, position: f64) -> Option<usize> {
    let moved = moved_all(stops, i, position)?;
    apply(app, &moved).then(|| {
        *stops = moved;
        set_pstate(ctx, "tabs-sel", i);
        i
    })
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some((node, para, span)) = current_para(app) else {
        super::empty_state(ui, "pilcrow", tl!("No text selected"), tl!("Select a text object to set its tab stops."));
        return;
    };
    let mut stops = para.tabs.clone();
    let targets = indent_targets(app, &node);
    let mixed_left = targets.iter().any(|p| p.left != para.left_indent);
    let stored_marker: Option<IndentMarker> = pstate(ui.ctx(), "tabs-marker-sel");
    let selected_marker = stored_marker.filter(|marker| !mixed_left || *marker != IndentMarker::Lower);
    if selected_marker != stored_marker {
        set_pstate::<Option<IndentMarker>>(ui.ctx(), "tabs-marker-sel", None);
    }
    let mut sel: usize = pstate(ui.ctx(), "tabs-sel");
    if sel >= stops.len() {
        sel = stops.len().saturating_sub(1);
    }
    let mut align_idx: usize = pstate(ui.ctx(), "tabs-align");
    let vertical = matches!(&node.kind, NodeKind::Text(text) if text.vertical);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (i, (a, label)) in ALIGNS.iter().enumerate() {
            let on = if selected_marker.is_some() { align_idx == i } else { stops.get(sel).map_or(align_idx == i, |s| s.align == *a) };
            let tip = align_name(*a, vertical);
            let resp = ui.add(egui::Button::new(egui::RichText::new(*label).size(13.0)).selected(on).min_size(vec2(28.0, 24.0)));
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, tip));
            if resp.on_hover_text(tip).clicked() {
                align_idx = i;
                set_pstate(ui.ctx(), "tabs-align", i);
                if let Some(s) = stops.get_mut(sel).filter(|_| selected_marker.is_none()) {
                    s.align = *a;
                    apply(app, &stops);
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Above Text")).on_hover_text(tl!("Position Panel Above Text")).clicked() {
                place_above_text(app, ui.ctx(), &node, span);
            }
        });
    });
    ui.add_space(4.0);
    egui::Grid::new("tabs-grid").num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
        ui.label(egui::RichText::new(if vertical { tl!("Y:") } else { tl!("X:") }).color(t.text));
        let x = selected_marker.map(|m| marker_position(m, &para)).or_else(|| stops.get(sel).map(|s| s.position));
        if let Some(v) = widgets::spin_field(ui, "tabs-x", x, app.session.general_unit(), 90.0, 1.0, 0.0, &[]) {
            if let Some(marker) = selected_marker {
                set_marker(app, marker, v, &para, span, &targets, false);
            } else if let Some(i) = apply_moved(app, ui.ctx(), &mut stops, sel, v.max(0.0)) {
                sel = i;
            }
        }
        ui.end_row();
        ui.label(egui::RichText::new(tl!("Leader:")).color(t.text));
        // The fields keep what is typed until Enter or a click away commits it (#924).
        let leader = stops.get(sel).filter(|_| selected_marker.is_none()).map(|s| s.leader.clone()).unwrap_or_default();
        let typed = ui
            .add_enabled_ui(stops.get(sel).is_some() && selected_marker.is_none(), |ui| {
                widgets::exact_text_field_named(ui, "tabs-leader", Some(tl!("Leader:")), &leader, 90.0)
            })
            .inner;
        if let (Some(leader), Some(s)) = (typed, stops.get_mut(sel)) {
            // Up to eight characters repeat in the leader.
            s.leader = leader.chars().take(8).collect();
            apply(app, &stops);
        }
        ui.end_row();
        ui.label(egui::RichText::new(tl!("Align On:")).color(t.text));
        let decimal = selected_marker.is_none() && stops.get(sel).is_some_and(|s| s.align == TabAlign::Decimal);
        let on = stops.get(sel).map(|s| s.align_on.to_string()).unwrap_or_else(|| ".".into());
        let typed = ui.add_enabled_ui(decimal, |ui| widgets::exact_text_field_named(ui, "tabs-align-on", Some(tl!("Align On:")), &on, 30.0)).inner;
        if let (Some(c), Some(s)) = (typed.and_then(|t| t.chars().next()), stops.get_mut(sel)) {
            s.align_on = c;
            apply(app, &stops);
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    // The ruler.
    let w = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click_and_drag());
    let p = ui.painter_at(rect.expand(2.0));
    p.rect_filled(rect, t.cr(), t.input);
    p.rect_stroke(rect, t.cr(), Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    // After Position Panel Above Text on this text, while the panel floats no wider than the
    // narrowest floating window, a ruler wider than the text is on screen draws the text at its
    // screen scale and runs on past its width. Any other ruler fits the span to its width.
    let above: Option<(u64, vectorcraft_doc::NodeId)> = pstate(ui.ctx(), "tabs-above");
    let narrowest = || {
        let floating = app.ui.docking.as_ref().and_then(|l| l.floating.iter().find(|f| f.contains(&"tabs".to_string())));
        floating.is_some_and(|f| f.rect[2] <= crate::panel_docking::PANEL_MIN_WIDTH + 0.5)
    };
    let scale = above
        .filter(|at| Some(*at) == app.session.active().map(|doc| (doc.uid, node.id)) && narrowest())
        .and_then(|_| screen_scale(app, &node, span))
        .filter(|per| span * per + 8.0 < w as f64 - 0.5)
        .unwrap_or((w as f64 - 8.0) / span);
    let length = span.max((w as f64 - 8.0) / scale);
    let to_x = |pt: f64| rect.left() + 4.0 + (pt * scale) as f32;
    let to_pt = |x: f32| (((x - rect.left() - 4.0) as f64) / scale).max(0.0);
    // The ruler is numbered in the General unit, with the canvas rulers' label step at its scale
    // and a tick every tenth of a step.
    let unit = app.session.general_unit();
    let step = unit.ruler_step(scale);
    let minor = step / 10.0;
    for k in 0..=((unit.from_pt(length) / minor + 1e-6).floor() as usize).min(MAX_TICKS) {
        let v = k as f64 * minor;
        let x = to_x(unit.to_pt(v));
        let len = match k % 10 {
            0 => 10.0,
            5 => 7.0,
            _ => 4.0,
        };
        p.line_segment([egui::pos2(x, rect.bottom() - len), egui::pos2(x, rect.bottom())], Stroke::new(1.0, t.text_dim));
        if k % 10 == 0 {
            p.text(
                egui::pos2(x + 2.0, rect.top() + 1.0),
                egui::Align2::LEFT_TOP,
                crate::canvas::ruler_label(v, step),
                egui::FontId::proportional(9.0),
                t.text_dim,
            );
        }
    }
    // A stop placed or dragged on the ruler lands on the nearest whole point. With Snap to Unit on,
    // or Shift held, it lands on the ruler's nearest tick, which can be a fraction of a point.
    let snap = pstate::<bool>(ui.ctx(), "tabs-snap") || ui.input(|i| i.modifiers.shift);
    let place = |x: f32| if snap { unit.snap_to_ruler_tick(to_pt(x), scale).max(0.0) } else { to_pt(x).round() };
    let stored_drag: Option<(IndentMarker, bool)> = pstate(ui.ctx(), "tabs-marker-drag");
    let marker_drag = stored_drag.filter(|(marker, _)| !mixed_left || *marker != IndentMarker::Lower);
    if marker_drag != stored_drag {
        set_pstate::<Option<(IndentMarker, bool)>>(ui.ctx(), "tabs-marker-drag", None);
    }
    let preview = marker_drag.and_then(|(marker, both)| {
        let pointer = ui.input(|i| i.pointer.interact_pos())?;
        let value = bounded_marker_position(marker, place(pointer.x), span, &targets, both)?;
        Some((marker, value, both))
    });
    paint_markers(ui, &p, &t, MarkerPaint { rect, span, para: &para, mixed_left, selected: selected_marker, drag: marker_drag, preview }, to_x);
    let drag: Option<usize> = pstate(ui.ctx(), "tabs-drag");
    let drag_all = drag.is_some() && pstate::<bool>(ui.ctx(), "tabs-drag-all");
    // While a Ctrl-drag (Command on macOS) is on the ruler, the stops are drawn where the drop puts them.
    let all_preview = drag.filter(|_| drag_all).and_then(|i| {
        let pointer = ui.input(|input| input.pointer.any_down().then(|| input.pointer.interact_pos()).flatten())?;
        (pointer.y <= rect.bottom() + 20.0).then(|| moved_all(&stops, i, place(pointer.x))).flatten()
    });
    for (i, s) in all_preview.as_deref().unwrap_or(&stops).iter().enumerate() {
        let x = to_x(s.position);
        let c = if i == sel && selected_marker.is_none() { t.accent } else { t.text };
        let y = rect.bottom() - 2.0;
        // An original marker: a stem with a foot showing which way the text runs.
        p.line_segment([egui::pos2(x, y - 12.0), egui::pos2(x, y)], Stroke::new(1.5, c));
        let foot = match s.align {
            TabAlign::Left => [egui::pos2(x, y), egui::pos2(x + 6.0, y)],
            TabAlign::Right => [egui::pos2(x - 6.0, y), egui::pos2(x, y)],
            TabAlign::Center | TabAlign::Decimal => [egui::pos2(x - 4.0, y), egui::pos2(x + 4.0, y)],
        };
        p.line_segment(foot, Stroke::new(1.5, c));
        if s.align == TabAlign::Decimal {
            p.circle_filled(egui::pos2(x + 3.0, y - 6.0), 1.5, c);
        }
    }
    // The stop within 5 px of `x`.
    let hit_at = |x: f32| stops.iter().position(|s| (to_x(s.position) - x).abs() <= 5.0);
    let cancel_drag = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if cancel_drag || (marker_drag.is_some() && !resp.drag_stopped() && !ui.input(|i| i.pointer.any_down())) {
        set_pstate::<Option<(IndentMarker, bool)>>(ui.ctx(), "tabs-marker-drag", None);
        set_pstate::<Option<usize>>(ui.ctx(), "tabs-drag", None);
    }
    // egui reports the drag only once the pointer is a few pixels from the press, past the hit
    // tolerance, so hit-test where the button went down (#1458).
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        if let Some(marker) = hit_marker(pos, rect, &para, span, mixed_left, to_x) {
            let both = marker == IndentMarker::Lower && ui.input(|i| i.modifiers.command);
            set_pstate(ui.ctx(), "tabs-marker-sel", Some(marker));
            set_pstate(ui.ctx(), "tabs-marker-drag", Some((marker, both)));
            set_pstate::<Option<usize>>(ui.ctx(), "tabs-drag", None);
        } else {
            let hit = hit_at(pos.x);
            set_pstate(ui.ctx(), "tabs-drag", hit);
            set_pstate(ui.ctx(), "tabs-drag-all", hit.is_some() && ui.input(|i| i.modifiers.command));
            if let Some(i) = hit {
                set_pstate::<Option<IndentMarker>>(ui.ctx(), "tabs-marker-sel", None);
                set_pstate(ui.ctx(), "tabs-sel", i);
            }
        }
    }
    if resp.drag_stopped()
        && !cancel_drag
        && let Some((marker, both)) = marker_drag
    {
        set_pstate::<Option<(IndentMarker, bool)>>(ui.ctx(), "tabs-marker-drag", None);
        if let Some(pos) = resp.interact_pointer_pos() {
            set_marker(app, marker, place(pos.x), &para, span, &targets, both);
        }
    } else if resp.drag_stopped()
        && !cancel_drag
        && let (Some(i), Some(pos)) = (drag, resp.interact_pointer_pos())
    {
        set_pstate::<Option<usize>>(ui.ctx(), "tabs-drag", None);
        if i < stops.len() {
            if pos.y > rect.bottom() + 20.0 {
                // Dragged off the ruler: delete.
                stops.remove(i);
                apply(app, &stops);
            } else if drag_all {
                apply_moved_all(app, ui.ctx(), &mut stops, i, place(pos.x));
            } else {
                apply_moved(app, ui.ctx(), &mut stops, i, place(pos.x));
            }
        }
    } else if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        if let Some(marker) = hit_marker(pos, rect, &para, span, mixed_left, to_x) {
            set_pstate(ui.ctx(), "tabs-marker-sel", Some(marker));
        } else {
            match hit_at(pos.x) {
                Some(i) => {
                    set_pstate::<Option<IndentMarker>>(ui.ctx(), "tabs-marker-sel", None);
                    set_pstate(ui.ctx(), "tabs-sel", i);
                }
                None => {
                    let position = place(pos.x);
                    // A click that lands on a stop's whole point or tick selects that stop and records no
                    // undo step.
                    let added = !stops.iter().any(|s| s.position == position);
                    let max = TabStop::MAX_STOPS;
                    if added && stops.len() >= max {
                        // A full list takes no more stops: the click leaves the stops and the
                        // selection as they are and shows the limit.
                        app.status(format!("at most {max} tab stops"));
                    } else {
                        if added {
                            stops.push(TabStop { position, align: ALIGNS[align_idx.min(3)].0, leader: String::new(), align_on: '.' });
                            stops.sort_by(|a, b| a.position.total_cmp(&b.position));
                        }
                        let i = stops.iter().position(|s| s.position == position).unwrap_or(0);
                        set_pstate::<Option<IndentMarker>>(ui.ctx(), "tabs-marker-sel", None);
                        set_pstate(ui.ctx(), "tabs-sel", i);
                        if added {
                            apply(app, &stops);
                        }
                    }
                }
            }
        }
    }
    resp.on_hover_text(tl!("Click to add a tab stop, drag to move it, drag it off the ruler to delete it"));
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((_, para, _)) = current_para(app) else {
        ui.add_enabled(false, egui::Button::new(tl!("Select text")).frame(false));
        snap_to_unit_item(ui);
        return;
    };
    // `current_para` gives the left indent with the list level's indent added.
    let left_indent = para.left_indent;
    let mut stops = para.tabs;
    let selected_marker: Option<IndentMarker> = pstate(ui.ctx(), "tabs-marker-sel");
    // Clamp the selection to the last stop, as `show` does for the ruler.
    let mut sel: usize = pstate(ui.ctx(), "tabs-sel");
    if sel >= stops.len() {
        sel = stops.len().saturating_sub(1);
    }
    if menu_item(ui, tl!("Clear All Tabs"), !stops.is_empty(), false)
        && let Err(e) = super::paragraph::run_on_paragraphs(app, "text.tabs.clear", json!({}))
    {
        app.status(e);
    }
    if menu_item(ui, tl!("Delete Tab"), sel < stops.len() && selected_marker.is_none(), false) {
        stops.remove(sel);
        apply(app, &stops);
    }
    // Repeat Tab: copies of the selected stop across the ruler, spaced by its distance from the
    // previous stop, or from the paragraph's left indent plus its list level's indent when it is
    // the first stop. The copies stop at the 256-stop limit, which the status bar then shows.
    if menu_item(ui, tl!("Repeat Tab"), sel < stops.len() && selected_marker.is_none(), false) {
        let step = match sel {
            0 => (stops[0].position - left_indent).abs(),
            _ => stops[sel].position - stops[sel - 1].position,
        }
        .max(1.0);
        let base = stops[sel].clone();
        stops.truncate(sel + 1);
        let max = TabStop::MAX_STOPS;
        let mut pos = base.position + step;
        while pos <= base.position + step * 20.0 && pos < 2000.0 {
            if stops.len() >= max {
                app.status(format!("at most {max} tab stops"));
                break;
            }
            stops.push(TabStop { position: pos, ..base.clone() });
            pos += step;
        }
        apply(app, &stops);
    }
    snap_to_unit_item(ui);
}

/// The panel menu's Snap to Unit, checked while it is on: stops placed or dragged on the ruler land
/// on its nearest tick.
fn snap_to_unit_item(ui: &mut Ui) {
    ui.separator();
    let on: bool = pstate(ui.ctx(), "tabs-snap");
    if menu_item(ui, tl!("Snap to Unit"), true, on) {
        set_pstate(ui.ctx(), "tabs-snap", !on);
    }
}

#[cfg(test)]
mod tests;
