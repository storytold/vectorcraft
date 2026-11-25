//! Tabs panel: tab-stop alignment, X position (Y in vertical type), leader and "align on", and the
//! framework's text ruler numbered in the General unit, on which a click adds a stop, dragging moves
//! one and dragging it off the ruler deletes it. Ctrl-dragging a stop (Command on macOS) moves all the
//! stops by the same distance. A stop lands on the ruler's step, or free with Alt held. While a stop is
//! dragged, the canvas draws a guide through the selected text where the stop would land.

use std::collections::HashSet;

use craft::keymap::CommandRef;
use craft::text::ui::DocUnit;
use craft::text::ui::ruler::{RulerItem, RulerValues, TabAlign as RulerAlign, TabLeader, TabStop as RulerStop, TextRuler};
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::canvas::CanvasSpace;
use craft::ui::canvas_controls::{SmartGuides, SmartLine};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::layout::RowAlign;
use craft::ui::layout_components::Row;
use craft::ui::menu::PanelRow;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Label;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, ParaStyle, TabAlign, TabStop, TextKind, TextObject};
use vectorcraft_geom::{Point, Vec2};

use crate::VectorcraftApp;
use crate::canvas::rulers::{general_lengths, unit_kind};

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

/// The largest stop position `text.tabs.set` takes, in points.
const MAX_STOP: f64 = 100_000.0;

/// The salt of the ruler's id: the panel's own pass and the canvas's stop guide name the same ruler by it
/// (`TextRuler::dragging`).
pub(crate) const RULER_ID: &str = "tabs-ruler";

const CLEAR_ALL: &str = "ui.tabs.clearAll";
const DELETE_TAB: &str = "ui.tabs.deleteTab";
const REPEAT_TAB: &str = "ui.tabs.repeatTab";
const COMMANDS: [&str; 3] = [CLEAR_ALL, DELETE_TAB, REPEAT_TAB];

/// What the panel remembers between frames: the stop the fields show, the marker picked on the ruler
/// (it takes the fields over while it is set), the alignment a new stop takes and the text the panel
/// was last placed above.
#[derive(Clone, Debug, Default)]
pub(crate) struct TabsPanel {
    /// The picked stop, clamped to the paragraph's last stop where it is read ([`Self::stop_in`]).
    stop: usize,
    /// The indent marker picked on the ruler; the fields show and edit it in place of the stop.
    marker: Option<RulerItem>,
    /// The alignment the buttons show while no stop is picked, and the one a new stop takes.
    align: TabAlign,
    /// The document and text Position Panel Above Text last sized the ruler for.
    above: Option<(u64, NodeId)>,
}

impl TabsPanel {
    /// The picked stop within a paragraph of `count` stops.
    fn stop_in(&self, count: usize) -> usize {
        self.stop.min(count.saturating_sub(1))
    }
}

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
/// the ruler shows it. Without the Type tool editing text, the panel shows the first paragraph of
/// the first selected text object, also when that text is in a selected group or follows a
/// selected object that is not text.
fn current_para(app: &VectorcraftApp) -> Option<(Node, ParaStyle, f64)> {
    let editing = super::character::text_editing(app);
    let n = match editing {
        Some((id, _, _)) => app.session.active()?.doc.node(id)?.clone(),
        None => super::character::first_selected_text_node(app)?.clone(),
    };
    let NodeKind::Text(t) = &n.kind else { return None };
    let para = shown_paragraph(app, t);
    let span = match &t.kind {
        TextKind::Area { frame } => frame.bounds().map_or(360.0, |b| b.width()),
        _ => 360.0,
    };
    let para = t.para_at(para);
    let para = ParaStyle { left_indent: para.laid_out_left_indent(), ..para.clone() };
    Some((n, para, span.max(72.0)))
}

/// The index of the paragraph of text `t` whose stops the panel shows: the paragraph of the Type
/// tool's insertion point, else the first.
fn shown_paragraph(app: &VectorcraftApp, t: &TextObject) -> usize {
    super::character::text_editing(app).map_or(0, |(_, a, _)| t.paragraphs_in(a, a).start)
}

/// [`above_text`], then note the active document and text `n`. A floating panel is
/// at least [`crate::panel_docking::PANEL_MIN_WIDTH`] wide, so for narrow text at a low zoom the
/// panel comes out wider than asked; the ruler then draws text `n` at [`screen_scale`] and runs
/// on past the text's width ([`ruler_scale`]).
fn place_above_text(app: &mut VectorcraftApp, n: &Node, span: f64) {
    if above_text(app, n, span).is_some() {
        app.ui.tabs_panel.above = app.session.active().map(|doc| (doc.uid, n.id));
    }
}

/// Text `n`'s bounds on the screen under `view`.
fn on_screen_bounds(view: &craft::behavior::CanvasView, n: &Node) -> Option<egui::Rect> {
    Some(egui::Rect::from_points(&view.quad(n.geometric_bounds()?)))
}

/// Screen points per point of text `n` at the canvas zoom, as [`above_text`] sizes the ruler:
/// across the frame for area type, the zoom for point type.
fn screen_scale(app: &VectorcraftApp, n: &Node, span: f64) -> Option<f64> {
    let view = app.canvas_view()?;
    let per = match &n.kind {
        NodeKind::Text(t) if matches!(t.kind, TextKind::Area { .. }) => f64::from(on_screen_bounds(&view, n)?.width()) / span,
        _ => view.zoom,
    };
    (per.is_finite() && per > 0.0).then_some(per)
}

/// Position Panel Above Text: float the panel just above text `n`, as wide as its ruler needs to
/// put 0 on the text's left edge and its marks at the canvas zoom, so each stop sits over the
/// column it sets (#729). `span`: the ruler's length in the text's points.
fn above_text(app: &mut VectorcraftApp, n: &Node, span: f64) -> Option<()> {
    let view = app.canvas_view()?;
    let on_screen = on_screen_bounds(&view, n)?;
    let per = screen_scale(app, n, span)? as f32;
    let width = span as f32 * per + RULER_LEFT * 2.0;
    let height = crate::panel_docking::floating_height(app, "tabs");
    // Text near the top of the window: the panel goes just below the text when that place is in
    // the window and the panel fits there in the canvas, else at the window's top, so its bar
    // stays where it can be grabbed rather than above the screen.
    let above = on_screen.top() - height - 6.0;
    let below = on_screen.bottom() + 6.0;
    let top = if above >= 0.0 {
        above
    } else if below >= 0.0 && below + height <= view.rect.bottom() {
        below
    } else {
        0.0
    };
    let pos = egui::pos2(on_screen.left() - RULER_LEFT, top);
    crate::panel_docking::place_floating(app, "tabs", pos, width).ok()?;
    Some(())
}

/// Screen points per point of the ruler for a ruler `w` screen points wide over text `node`. After
/// Position Panel Above Text on this text, while the panel floats no wider than the narrowest
/// floating window, a ruler wider than the text is on screen draws the text at its screen scale and
/// runs on past its width. Any other ruler fits the span to its width.
fn ruler_scale(app: &VectorcraftApp, node: &Node, span: f64, w: f32) -> f64 {
    let w = f64::from(w);
    let at = app.session.active().map(|doc| (doc.uid, node.id));
    let narrowest = crate::panel_docking::layout_of(&app.ui)
        .floating
        .iter()
        .find(|f| f.contains("tabs"))
        .is_some_and(|f| f.rect.width() <= crate::panel_docking::PANEL_MIN_WIDTH + 0.5);
    app.ui
        .tabs_panel
        .above
        .filter(|above| Some(*above) == at && narrowest)
        .and_then(|_| screen_scale(app, node, span))
        .filter(|per| span * per + 8.0 < w - 0.5)
        .unwrap_or((w - 8.0) / span)
}

fn stops_json(stops: &[TabStop]) -> Value {
    Value::Array(
        stops
            .iter()
            .map(|s| json!({"position": s.position, "align": align_id(s.align), "leader": s.leader, "alignOn": s.align_on.to_string()}))
            .collect(),
    )
}

/// Run `text.tabs.set` with `stops` on the text the panel acts on: with the Selection tool, every
/// frame of the selected stories ([`super::character::selected_stories`]). Returns false when it
/// fails.
fn apply(app: &mut VectorcraftApp, stops: &[TabStop]) -> bool {
    let Ok(editing) = super::character::text_editing_kept(app) else { return false };
    if let Some((id, a, b)) = editing {
        let p = match super::character::text_story(app) {
            // After Select All in threaded text: every paragraph of every frame.
            Some(story) => json!({"stops": stops_json(stops), "ids": story.iter().map(|f| f.0).collect::<Vec<_>>()}),
            None => json!({"stops": stops_json(stops), "ids": [id.0], "start": a, "end": b}),
        };
        app.run("text.tabs.set", p).is_ok()
    } else {
        let mut p = json!({"stops": stops_json(stops)});
        super::character::target_selected_stories(app, &mut p);
        app.run("text.tabs.set", p).is_ok()
    }
}

/// Move stop `i` to `position`, sort the stops by position and set them. Returns the index of the
/// first stop within 1e-6 pt of `position`, the stop `text.tabs.set` keeps of those it merges, or
/// `None` when the stops are not set and `stops` stays as it was.
fn apply_moved(app: &mut VectorcraftApp, stops: &mut Vec<TabStop>, i: usize, position: f64) -> Option<usize> {
    let mut moved = stops.clone();
    moved.get_mut(i)?.position = position;
    moved.sort_by(|a, b| a.position.total_cmp(&b.position));
    let sel = moved.iter().position(|s| (s.position - position).abs() < 1e-6).unwrap_or(0);
    apply(app, &moved).then(|| {
        *stops = moved;
        sel
    })
}

/// The guide through text `t` for paragraph `para`'s stop at 0 on the ruler, in document space:
/// its two ends and how far it moves per point of stop position. For area type it crosses the frame
/// cell the paragraph starts in, and the ruler's 0 is where the frame lets the paragraph's first
/// line start (the cell's edge inside the inset, or past a wrap object); for point
/// type it crosses the paragraph's lines, and the ruler's 0 is the type's origin. In vertical type
/// it runs across the columns. `None` for type on a path and for a paragraph that does not fit in
/// the frame.
fn guide_axis(t: &TextObject, para: usize) -> Option<(Point, Point, Vec2)> {
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    if lay.on_path {
        return None;
    }
    let text = t.plain_text();
    let start = if para == 0 { 0 } else { text.match_indices('\n').nth(para - 1)?.0 + 1 };
    let end = text.get(start..)?.find('\n').map_or(text.len(), |i| start + i);
    let mut lines = lay.lines.iter().filter(|l| l.start >= start && l.start <= end);
    let to_doc = |x: f64, y: f64| t.xf * lay.physical_point(Point::new(x, y));
    let (x, top, bottom) = match &t.kind {
        TextKind::Area { .. } => {
            // The layout measures a line's stops from where the frame lets the line start (past a
            // wrap object, or in a frame that is not a rectangle): the paragraph's first line's
            // room less its indent, mirrored for a right-to-left list.
            let first = lines.next()?;
            let style = t.para_at(para);
            let indent = if first.rtl && style.list.is_some() {
                style.right_indent
            } else {
                style.laid_out_left_indent() + if para == 0 && t.first_para_continues { 0.0 } else { style.first_line_indent }
            };
            let cell = lay.line_xf.inverse().transform_rect_bbox(*lay.frames.get(first.region)?);
            (first.avail.0 - indent, cell.y0, cell.y1)
        }
        TextKind::Point => {
            let (top, bottom) =
                lines.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), l| (a.min(l.baseline - l.ascent), b.max(l.baseline + l.descent)));
            (0.0, top, bottom)
        }
        _ => return None,
    };
    let (a, b) = (to_doc(x, top), to_doc(x, bottom));
    let along = to_doc(x + 1.0, top) - a;
    [a.x, a.y, b.x, b.y, along.x, along.y].iter().all(|v| v.is_finite()).then_some((a, b, along))
}

/// The guide's two document points for a stop at `position` points from the ruler's 0, on the
/// `axis` [`guide_axis`] gives.
fn guide_at(axis: (Point, Point, Vec2), position: f64) -> (Point, Point) {
    let (a, b, along) = axis;
    (a + along * position, b + along * position)
}

/// The canvas control that shows where a dragged tab stop would land: a guide through the selected
/// text while the ruler reports a dragged stop (`TextRuler::dragging`, which is `None` once the drag
/// ends, is cancelled, leaves the ruler for deleting the stop, or the panel stops showing). Paint
/// only, and none for type on a path.
pub(crate) fn stop_guide(app: &mut VectorcraftApp, ui: &mut Ui, _space: &CanvasSpace) {
    // PLAN: `_space` is unused: `SmartGuides` reads the canvas view from the canvas body itself; canvas-w10 still passes it (ty.S13).
    let Some(drag) = TextRuler::dragging(ui.ctx(), egui::Id::unique(RULER_ID)).filter(|d| matches!(d.item, RulerItem::Stop(_))) else { return };
    let Some(revision) = app.session.active().map(|doc| (doc.uid, doc.revision)) else { return };
    let Some((node, _, _)) = current_para(app) else { return };
    let NodeKind::Text(t) = &node.kind else { return };
    let paragraph = shown_paragraph(app, t);
    // Lay the text out once per document change, not in every frame of the drag.
    let key = (revision.0, node.id, paragraph, revision.1);
    // PLAN: assumed `app.canvas.tabs_guide_axis: Option<((u64, NodeId, usize, u64), Option<(Point, Point, Vec2)>)>` (hub-w1, ty.S18)
    let axis = match &app.canvas.tabs_guide_axis {
        Some((k, axis)) if *k == key => *axis,
        _ => {
            let axis = guide_axis(t, paragraph);
            app.canvas.tabs_guide_axis = Some((key, axis));
            axis
        }
    };
    let Some(axis) = axis else { return };
    let (a, b) = guide_at(axis, f64::from(drag.position));
    let line = [SmartLine::new(a, b)];
    SmartGuides::new(&line, &[]).show(ui);
}

/// The ruler's values for `para` and its `stops`: the stops, and the indents the ruler draws its
/// markers at. The ruler's stops carry no "align on" character and no leader text beyond its five
/// leaders ([`ruler_leader`]), so [`apply_ruler_stops`] keeps the app's stops for the ones it shows.
fn ruler_values(para: &ParaStyle, stops: &[TabStop]) -> RulerValues {
    let mut values = RulerValues::default();
    values.stops = stops.iter().map(|s| RulerStop::new(s.position as f32, ruler_align(s.align), ruler_leader(&s.leader))).collect();
    values.left_indent = para.left_indent as f32;
    values.first_line_indent = para.first_line_indent as f32;
    values.right_indent = para.right_indent as f32;
    values
}

fn ruler_align(a: TabAlign) -> RulerAlign {
    match a {
        TabAlign::Left => RulerAlign::Left,
        TabAlign::Center => RulerAlign::Center,
        TabAlign::Right => RulerAlign::Right,
        TabAlign::Decimal => RulerAlign::Decimal,
    }
}

/// The ruler's leader nearest the app's leader text: its own five fills, none for any other text.
fn ruler_leader(leader: &str) -> TabLeader {
    match leader {
        "." => TabLeader::Dot,
        "-" => TabLeader::Hyphen,
        "_" => TabLeader::Underscore,
        "·" => TabLeader::MiddleDot,
        _ => TabLeader::None,
    }
}

/// The indent marker the ruler's `item` stands for; the hanging triangle and the box both move the
/// left indent.
fn indent_marker(item: RulerItem) -> Option<IndentMarker> {
    match item {
        RulerItem::FirstLine => Some(IndentMarker::Upper),
        RulerItem::Hanging | RulerItem::LeftBox => Some(IndentMarker::Lower),
        _ => None,
    }
}

/// Where the ruler draws `item`, in points from the ruler's 0: the stop's position, an indent
/// marker's, or the right indent's distance from the paragraph's end edge.
fn picked_position(item: RulerItem, para: &ParaStyle, span: f64) -> Option<f64> {
    match item {
        RulerItem::Right => Some(span - para.right_indent),
        RulerItem::Stop(i) => para.tabs.get(i).map(|s| s.position),
        marker => indent_marker(marker).map(|m| marker_position(m, para)),
    }
}

/// For each stop of `after`, the index in `old` of the stop it continues. Stops that kept their
/// position come first; the stops left on both sides then continue each other in order, when as
/// many are left on each (a moved stop, or all of them moved together). A stop with no partner is
/// new.
fn pair_stops(old: &[TabStop], after: &[RulerStop]) -> Vec<Option<usize>> {
    let mut taken = vec![false; old.len()];
    let mut paired: Vec<Option<usize>> = after
        .iter()
        .map(|s| {
            let i = old.iter().enumerate().find_map(|(i, o)| (taken.get(i) == Some(&false) && o.position as f32 == s.position).then_some(i))?;
            if let Some(t) = taken.get_mut(i) {
                *t = true;
            }
            Some(i)
        })
        .collect();
    let free: Vec<usize> = taken.iter().enumerate().filter(|(_, t)| !**t).map(|(i, _)| i).collect();
    let loose: Vec<usize> = paired.iter().enumerate().filter(|(_, p)| p.is_none()).map(|(i, _)| i).collect();
    if free.len() == loose.len() {
        for (at, old_at) in loose.into_iter().zip(free) {
            if let Some(p) = paired.get_mut(at) {
                *p = Some(old_at);
            }
        }
    }
    paired
}

/// Set the stops the ruler shows after an edit: a stop that continues one of the paragraph's keeps
/// its leader and "align on"; a new one takes the panel's alignment. A full list takes no more
/// stops: the edit leaves the stops as they are and the status bar shows the limit. The pick follows
/// the moved stop, or goes to the added one.
fn apply_ruler_stops(app: &mut VectorcraftApp, para: &ParaStyle, after: &[RulerStop]) {
    let max = TabStop::MAX_STOPS;
    if after.len() > max {
        app.status(format!("at most {max} tab stops"));
        return;
    }
    let paired = pair_stops(&para.tabs, after);
    let align = app.ui.tabs_panel.align;
    let stops: Vec<TabStop> = after
        .iter()
        .zip(&paired)
        .map(|(s, partner)| {
            let position = f64::from(s.position);
            match partner.and_then(|i| para.tabs.get(i)) {
                Some(old) => TabStop { position, ..old.clone() },
                None => TabStop { position, align, leader: String::new(), align_on: '.' },
            }
        })
        .collect();
    if !apply(app, &stops) {
        return;
    }
    let panel = &mut app.ui.tabs_panel;
    if let Some(added) = paired.iter().position(Option::is_none) {
        panel.marker = None;
        panel.stop = added;
    } else if let Some(moved) = paired.iter().position(|p| *p == Some(panel.stop)) {
        panel.stop = moved;
    }
}

/// Apply the ruler's whole new values `after` of a finished edit (`before`: the values it showed) to
/// the paragraph `para` of text `node`, whose ruler is `span` points long: the stops, then the
/// indents. A drag has no preview, so this runs once, when the edit commits.
pub(crate) fn apply_ruler_edit(app: &mut VectorcraftApp, node: &Node, para: &ParaStyle, span: f64, before: &RulerValues, after: &RulerValues) {
    if before == after {
        return;
    }
    if before.stops != after.stops {
        apply_ruler_stops(app, para, &after.stops);
    }
    let targets = indent_targets(app, node);
    if after.left_indent == before.left_indent && after.first_line_indent != before.first_line_indent {
        let target = para.left_indent + f64::from(after.first_line_indent);
        set_marker(app, IndentMarker::Upper, target, para, span, &targets, false);
    } else if after.left_indent != before.left_indent {
        // The first line keeps its place unless it moved with the left indent.
        let together = after.first_line_indent == before.first_line_indent;
        set_marker(app, IndentMarker::Lower, f64::from(after.left_indent), para, span, &targets, together);
    }
    if after.right_indent != before.right_indent {
        run_on_targets(app, &targets, json!({"rightIndent": f64::from(after.right_indent)}));
    }
}

/// The X (Y in vertical type) field's `value`: a picked marker moves to it, else the picked stop
/// does and stays picked after the stops are sorted. A position past [`MAX_STOP`] is refused by
/// `text.tabs.set` and nothing changes. `picked`: the stop and the marker of [`TabsPanel`].
pub(crate) fn set_position(app: &mut VectorcraftApp, node: &Node, para: &ParaStyle, span: f64, picked: (usize, Option<RulerItem>), value: f64) {
    let (stop, marker) = picked;
    match marker {
        Some(RulerItem::Right) => {
            let targets = indent_targets(app, node);
            run_on_targets(app, &targets, json!({"rightIndent": (span - value).max(0.0)}));
        }
        Some(item) => {
            if let Some(marker) = indent_marker(item) {
                let targets = indent_targets(app, node);
                set_marker(app, marker, value, para, span, &targets, false);
            }
        }
        None => {
            let mut stops = para.tabs.clone();
            if let Some(i) = apply_moved(app, &mut stops, stop, value.max(0.0)) {
                app.ui.tabs_panel.stop = i;
            }
        }
    }
}

/// The Tabs panel. It reads no frame input itself: every edit comes from a component's events, and
/// a component reports only its own pass's input, so a second pass in a frame applies nothing again.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((node, para, span)) = current_para(app) else {
        Notice::new(Tone::Neutral, tl!("Select a text object to set its tab stops."))
            .layout(NoticeLayout::Centred)
            .title(tl!("No text selected"))
            .show(ui);
        return;
    };
    let vertical = matches!(&node.kind, NodeKind::Text(text) if text.vertical);
    let targets = indent_targets(app, &node);
    let mixed_left = targets.iter().any(|p| p.left != para.left_indent);
    let panel = &mut app.ui.tabs_panel;
    // Paragraphs with different left indents have no one left-indent marker to pick.
    if mixed_left && matches!(panel.marker, Some(RulerItem::Hanging | RulerItem::LeftBox)) {
        panel.marker = None;
    }
    panel.stop = panel.stop_in(para.tabs.len());
    let (stop, marker, align) = (panel.stop, panel.marker, panel.align);
    let mut stops = para.tabs.clone();

    let shown_align = if marker.is_some() { align } else { stops.get(stop).map_or(align, |s| s.align) };
    let row = Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
        let buttons = Row::new().show(ui, |ui, _| {
            ALIGNS.iter().fold(None, |hit, (a, glyph)| {
                let name = align_name(*a, vertical);
                let out = ToggleButton::new(*glyph, Bind::given(shown_align == *a)).spoken(Label::from(name)).tooltip(name).show(ui);
                hit.or(out.events.committed().map(|_| *a))
            })
        });
        let above = Button::new(tl!("Above Text")).tooltip(tl!("Position Panel Above Text")).show(ui).clicked();
        (buttons.inner.flatten(), above)
    });
    let (clicked, above) = row.inner.unwrap_or_default();
    if let Some(a) = clicked {
        app.ui.tabs_panel.align = a;
        if marker.is_none()
            && let Some(s) = stops.get_mut(stop)
        {
            s.align = a;
            apply(app, &stops);
        }
    }
    if above {
        place_above_text(app, &node, span);
    }

    let lengths = general_lengths(app);
    let label = if vertical { tl!("Y:") } else { tl!("X:") };
    let x = marker.map_or_else(|| stops.get(stop).map(|s| s.position), |item| picked_position(item, &para, span));
    let typed = Field::new(label)
        .show(ui, |ui, _| NumberField::length(label, Bind::given(x), lengths).range(0.0..=MAX_STOP).show(ui))
        .inner
        .and_then(|out| out.edit.events.committed().copied().flatten());
    if let Some(value) = typed {
        set_position(app, &node, &para, span, (stop, marker), value);
    }

    let on_stop = marker.is_none() && stop < stops.len();
    let leader = stops.get(stop).filter(|_| marker.is_none()).map(|s| s.leader.clone()).unwrap_or_default();
    let typed = Field::new(tl!("Leader:"))
        .show(ui, |ui, _| TextField::new(tl!("Leader:"), Bind::given(leader.clone())).limit(8_usize).enabled(on_stop).show(ui))
        .inner
        .and_then(|out| out.events.committed().cloned());
    if let (Some(leader), Some(s)) = (typed, stops.get_mut(stop)) {
        s.leader = leader;
        apply(app, &stops);
    }

    let decimal = marker.is_none() && stops.get(stop).is_some_and(|s| s.align == TabAlign::Decimal);
    let on = stops.get(stop).map_or_else(|| ".".to_string(), |s| s.align_on.to_string());
    let typed = Field::new(tl!("Align On:"))
        .show(ui, |ui, _| TextField::new(tl!("Align On:"), Bind::given(on.clone())).limit(1_usize).enabled(decimal).show(ui))
        .inner
        .and_then(|out| out.events.committed().and_then(|text| text.chars().next()));
    if let (Some(c), Some(s)) = (typed, stops.get_mut(stop)) {
        s.align_on = c;
        apply(app, &stops);
    }

    let values = ruler_values(&para, &para.tabs);
    let w = ui.available_width().max(120.0);
    let scale = ruler_scale(app, &node, span, w);
    // Points are pixels in a document: 72 pixels to the inch.
    let unit = DocUnit::new(unit_kind(app.session.general_unit()), 72.0);
    let mut ruler = TextRuler::new(tl!("Tabs"), Bind::given(values.clone()), span as f32)
        .id(egui::Id::unique(RULER_ID))
        .unit(unit)
        .scale(scale as f32)
        .tooltip(tl!("Click to add a tab stop, drag to move it, drag it off the ruler to delete it"));
    if marker.is_none() && !para.tabs.is_empty() {
        ruler = ruler.picked_stop(stop);
    }
    let out = ruler.show(ui);
    crate::edits::apply_edits(app, &out.edit, crate::edits::EditPath::Scope, |app, after, _| {
        apply_ruler_edit(app, &node, &para, span, &values, after);
    });
    match out.picked {
        Some(RulerItem::Stop(i)) => {
            app.ui.tabs_panel.stop = i;
            app.ui.tabs_panel.marker = None;
        }
        Some(item) => app.ui.tabs_panel.marker = Some(item),
        None => {}
    }
}

/// Whether a paragraph of the whole story Clear All Tabs acts on has tab stops: after Select All in
/// threaded text, every frame of the edited story; with the Selection tool, every frame of the
/// selected stories ([`super::character::selected_stories`]).
fn story_has_tabs(app: &VectorcraftApp) -> bool {
    let has_tabs = |t: &TextObject| (0..t.paragraph_count()).any(|i| !t.para_at(i).tabs.is_empty());
    match (super::character::text_editing(app), super::character::text_story(app), app.session.active()) {
        (Some(_), Some(story), Some(st)) => {
            story.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if has_tabs(t)))
        }
        (Some(_), _, _) => false,
        (None, _, _) => super::character::selected_story_texts(app).into_iter().any(has_tabs),
    }
}

/// The panel menu's rows: Clear All Tabs, Delete Tab and Repeat Tab.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    COMMANDS.iter().map(|id| PanelRow::Command(CommandRef::new(*id))).collect()
}

/// Enable or disable the panel menu's commands as [`can`] says.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for id in COMMANDS {
        states.set(CommandRef::new(id), CommandState::new().enabled(can(app, id)));
    }
}

/// Whether the panel command `id` can run now. With no text all are off. Clear All needs a stop in
/// the shown paragraph or tab stops anywhere in the story; Delete Tab and Repeat Tab need a picked
/// stop and no picked marker.
fn can(app: &VectorcraftApp, id: &str) -> bool {
    let Some((_, para, _)) = current_para(app) else { return false };
    let panel = &app.ui.tabs_panel;
    let on_stop = panel.marker.is_none() && panel.stop_in(para.tabs.len()) < para.tabs.len();
    match id {
        CLEAR_ALL => !para.tabs.is_empty() || story_has_tabs(app),
        DELETE_TAB | REPEAT_TAB => on_stop,
        _ => false,
    }
}

/// Run the panel command `id` (`None`: not one of this panel's). A command that [`can`] not run
/// returns an error and changes nothing, whether it was asked for by its menu row, the palette or
/// automation.
// PLAN: assumed `run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>>` (hub-w2, ty.S8)
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    if !COMMANDS.contains(&id) {
        return None;
    }
    if !can(app, id) {
        return Some(Err(format!("{id} is not available")));
    }
    Some(match id {
        CLEAR_ALL => super::paragraph::run_on_paragraphs(app, "text.tabs.clear", json!({})),
        DELETE_TAB => delete_tab(app),
        _ => repeat_tab(app),
    })
}

/// Set `stops` as the panel's stops.
fn set_stops(app: &mut VectorcraftApp, stops: &[TabStop]) -> Result<Value, String> {
    apply(app, stops).then_some(Value::Null).ok_or_else(|| "the tab stops were not set".to_string())
}

/// Remove the picked stop; the pick moves to the new last stop when it was the last.
fn delete_tab(app: &mut VectorcraftApp) -> Result<Value, String> {
    let (_, para, _) = current_para(app).ok_or("no text")?;
    let mut stops = para.tabs;
    let at = app.ui.tabs_panel.stop_in(stops.len());
    if at >= stops.len() {
        return Err("no tab stop to delete".into());
    }
    stops.remove(at);
    let done = set_stops(app, &stops)?;
    app.ui.tabs_panel.stop = app.ui.tabs_panel.stop_in(stops.len());
    Ok(done)
}

/// Copies of the picked stop across the ruler, spaced by its distance from the previous stop, or
/// from the paragraph's left indent plus its list level's indent when it is the first stop. The
/// copies stop at the 256-stop limit, which the status bar then shows.
fn repeat_tab(app: &mut VectorcraftApp) -> Result<Value, String> {
    let (_, para, _) = current_para(app).ok_or("no text")?;
    let mut stops = para.tabs;
    let at = app.ui.tabs_panel.stop_in(stops.len());
    let base = stops.get(at).cloned().ok_or("no tab stop to repeat")?;
    let previous = at.checked_sub(1).and_then(|p| stops.get(p)).map(|s| s.position);
    let step = match previous {
        None => (base.position - para.left_indent).abs(),
        Some(previous) => base.position - previous,
    }
    .max(1.0);
    stops.truncate(at + 1);
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
    set_stops(app, &stops)
}

#[cfg(test)]
mod tests;
