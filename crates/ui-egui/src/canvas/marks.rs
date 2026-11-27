//! Marks on the art: the hidden characters of Show Hidden Characters, Auto Spell Check's waves, Draw
//! Inside's corner arms, Simplify's original-path band, the print tiling, the slices and the
//! artboards' fade, bleed and marks. [`marks_of`] gathers what one pass shows; [`show`] puts it on the
//! canvas through the framework's controls. Threads and Live Paint gaps stay lists of [`Overlay`]s
//! for the tool adapter's painter.

use std::collections::HashMap;
use std::sync::Arc;

use craft::behavior::{CanvasView, Input, Metrics, Sense};
// PLAN: assumed ui-egui depends on the `craft` facade with the draw, pages and ui features (manifest owned by hub-w1).
use craft::draw::ui::slice::{SliceItem, SliceOrigin, SliceOverlay, SliceRules};
use craft::pages::ui::marks::{HiddenMark, MarkAnchor, PlacedMark, TextFrameMarks};
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::canvas::CanvasSpace;
use craft::ui::canvas_controls::{SelectionOutline, path_runs};
use craft::ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props, Role, Show as _};
use craft::ui::fonts::FontRef;
use craft::ui::primitives::{DocRun, DocText, Draw, PaintShape, StrokeWidth};
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, Pos2, Response, Vec2};
use vectorcraft_doc::{Node, NodeId, NodeKind};
use vectorcraft_engine::DocState;
use vectorcraft_geom::{BezPath, Point, Rect};
use vectorcraft_pdf::TilingPage;
use vectorcraft_tools::Overlay;

use super::BodyKind;
use super::artboards;
use super::grid::pref_color;
use super::outline::OVERLAY_MAX_ANCHORS;
use crate::VectorcraftApp;
use crate::doc_windows::Win;

#[cfg(test)]
mod tests;

/// The one colour of the hidden-character marks, whatever the layer.
const HIDDEN_BLUE: Color32 = Color32::from_rgb(0x4f, 0x9d, 0xff);

/// The print tiling's lines: a neutral grey that reads on the paper and on the pasteboard.
const PRINT_TILING: Color32 = Color32::from_gray(96);

/// The share of a tile's strength a tile outside the tile range keeps.
const UNPRINTED_SHARE: f32 = 0.45;

/// The fade outside the artboards: black at 40% opacity.
const FADE_OUTSIDE: DataColor = DataColor::new(Color32::from_black_alpha(102));

/// The share of its layer colour Simplify's original-path band is drawn at.
const BAND_SHARE: f32 = 0.4;

/// The size of a print tile's number: document annotation text, not an interface size.
const TILE_NUMBER_SIZE: f32 = 10.0;

/// A spelling wave's half-length and height in screen points.
const WAVE_STEP: f32 = 2.0;
const WAVE_HIGH: f32 = 1.5;

/// The most waves on one line, however far the view is zoomed in.
const WAVE_MAX: usize = 4_000;

/// The most words underlined in one text object: past it the rest of the object isn't checked, so a
/// long story of unknown words costs one check's worth.
const MAX_UNDERLINED: usize = 1_000;

/// The radius of a missing link's badge and the size of its question mark, in screen points: the
/// same at every zoom.
const MISSING_BADGE_RADIUS: f32 = 7.0;
const MISSING_BADGE_TEXT: f32 = 11.0;

/// The marks Show Hidden Characters drew on each text object in the last pass
/// ([`crate::CanvasCache::hidden_chars`]). Each is kept with the node it was laid out from: an edited
/// object is a new `Arc` and is laid out again, and all type is laid out again when the fonts or the
/// hyphenation exceptions change.
#[derive(Default)]
pub struct HiddenChars {
    /// The fonts' generation ([`vectorcraft_text::FontDb::generation`]) and the hyphenation
    /// exceptions the marks were laid out with.
    pub(super) inputs: (u64, String),
    /// Each text object shown, by id: the node it was laid out from and its marks in document space.
    pub(super) marks: HashMap<NodeId, (Arc<Node>, Arc<Vec<PlacedMark>>)>,
}

/// What Auto Spell Check's kept lines depend on beyond the text object itself: when any of it
/// changes, all type is checked again.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SpellingInputs {
    /// The fonts' generation ([`vectorcraft_text::FontDb::generation`]).
    fonts: u64,
    /// The hyphenation exceptions, which move line breaks.
    hyphenation: String,
    /// The custom dictionary's generation ([`vectorcraft_engine::cmd::spelling::dictionary_generation`]).
    dictionary: u64,
    /// The document the lines are of.
    doc: u64,
    /// How many Ignore All words the document has.
    ignored: usize,
}

/// One text object's Auto Spell Check lines: the document-space ends of each.
pub(super) type Underlines = Arc<Vec<(Point, Point)>>;

/// The lines Auto Spell Check drew under each text object in the last pass
/// ([`crate::CanvasCache::spelling`]). Each is kept with the node it was checked in: an edited object
/// is a new `Arc` and is checked again.
#[derive(Default)]
pub struct SpellingMarks {
    pub(super) inputs: Option<SpellingInputs>,
    /// Each text object shown, by id: the node it was checked in and its lines.
    pub(super) lines: HashMap<NodeId, (Arc<Node>, Underlines)>,
}

/// The visible text objects of a document, depth first in paint order ([`TextNodes`]).
pub(super) type TextList = Arc<Vec<Arc<Node>>>;

/// The visible text objects of the active document, for (document uid, revision): Show Hidden
/// Characters and Auto Spell Check draw over them every pass ([`crate::CanvasCache::text_nodes`]).
/// A pass that draws neither drops them ([`marks_of`]).
#[derive(Default)]
pub struct TextNodes {
    pub(super) list: Option<((u64, u64), TextList)>,
}

/// One print tile on the screen: its paper edge and its number.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tile {
    /// The paper's edge as a closed ring of screen points.
    edge: Vec<Pos2>,
    /// Where the number's text starts.
    at: Pos2,
    /// The tile's number as drawn.
    pub(crate) label: String,
    /// Whether the tile prints (tiles outside the tile range don't).
    printed: bool,
}

/// Everything one pass shows on the art, in the order the pass gathers it.
#[derive(Default)]
pub(crate) struct Marks {
    /// The hidden-character marks of the type in view, in document space.
    pub(crate) hidden: Vec<PlacedMark>,
    /// The Auto Spell Check segments of the type in view, in document space.
    pub(crate) spelling: Vec<(Point, Point)>,
    /// Draw Inside's corner arms in screen points, and the layer colour they take.
    pub(crate) arms: Vec<[Pos2; 2]>,
    arm_color: [u8; 3],
    /// Simplify's original paths with their layer colours.
    original: Vec<(BezPath, [u8; 3])>,
    /// The print tiles, and the imageable boxes of the printed and of the other tiles.
    pub(crate) tiles: Vec<Tile>,
    tile_boxes: Vec<BezPath>,
    tile_boxes_dim: Vec<BezPath>,
    /// The slices as the control takes them, the numbers of the selected ones, their colour and
    /// whether they are numbered.
    slices: Vec<SliceItem>,
    slice_selected: Vec<usize>,
    slice_color: Option<Srgba>,
    slice_numbers: bool,
    /// The fade outside the artboards, the bleed and the artboards' marks.
    fade: Vec<[Pos2; 4]>,
    bleed: Vec<BezPath>,
    artboard_marks: Vec<BezPath>,
    /// View › Show Missing Links: each visible missing linked image or placed document in view, as
    /// its box on the screen and where its badge sits (the box's top left corner in the document).
    pub(crate) missing: Vec<([Pos2; 4], Pos2)>,
}

/// Drops the hidden-character and spelling memos and the text nodes they hold.
pub(crate) fn release_text_marks(app: &mut VectorcraftApp) {
    app.canvas.hidden_chars.marks.clear();
    app.canvas.spelling = SpellingMarks::default();
    app.canvas.text_nodes = TextNodes::default();
}

/// The marks of one canvas pass over `view`. The memos of [`crate::CanvasCache`] are kept while their
/// gate is on and dropped, with the text nodes they hold, when it is off.
pub(crate) fn marks_of(app: &mut VectorcraftApp, view: &CanvasView) -> Marks {
    let mut m = Marks { original: original_band(app), ..Marks::default() };
    let revolve_edit = super::revolve_editing(app);
    let edges = app.ui.view.edges && !revolve_edit;
    let hidden_gate = edges && app.ui.view.hidden_chars;
    let spelling_gate = app.ui.view.auto_spell_check && !revolve_edit;
    let inside_gate = edges && app.session.draw_mode == vectorcraft_engine::DrawMode::Inside;
    let tiling_gate = app.session.active().is_some_and(|d| d.print_tiling) || app.session.tool_id() == "printTiling";
    let slices_gate = !app.session.slices_hidden();
    if !hidden_gate {
        app.canvas.hidden_chars.marks.clear();
    }
    if !spelling_gate {
        app.canvas.spelling = SpellingMarks::default();
    }
    if !hidden_gate && !spelling_gate {
        app.canvas.text_nodes = TextNodes::default();
    }
    let fades = artboards::fades_outside_artboards(app);
    let shows_artboards = artboards::shows_artboards(app);
    let Some(st) = app.session.active() else {
        release_text_marks(app);
        return m;
    };
    let key = (st.uid, st.revision);
    // The link watcher's last scan: nothing here reads files.
    m.missing = missing_links(st, view);
    if fades && let Some(shown) = app.view() {
        m.fade = artboards::fade_quads(view, &artboards::frames(app, st, shown, shown.outline));
    }
    if shows_artboards {
        m.bleed = artboards::bleed_paths(st);
        m.artboard_marks = artboards::mark_paths(st, view.zoom);
    }
    if hidden_gate || spelling_gate {
        // Both draw over the same text objects, found among the document's other objects once
        // for each revision.
        let texts = cached(&mut app.canvas.text_nodes.list, key, || text_nodes(&st.doc.layers));
        if hidden_gate {
            collect_hidden(&texts, view, &mut app.canvas.hidden_chars, &mut m.hidden);
        }
        if spelling_gate {
            collect_spelling(st, &texts, view, &mut app.canvas.spelling, &mut m.spelling);
        }
    }
    if inside_gate
        && let Some(id) = app.session.draw_inside
        && let Some(bounds) = st.doc.node(id).and_then(|n| n.visual_bounds())
    {
        m.arms = inside_arms(view, bounds);
        m.arm_color = st.doc.layer_color(id);
    }
    if tiling_gate {
        // Settings that can't print have no pages to show.
        let pages = cached(&mut app.canvas.print_tiling, key, || vectorcraft_engine::cmd::printtiling::pages(&st.doc).unwrap_or_default());
        m.tiles = tiles_of(&pages, view);
        for page in pages.iter() {
            let boxes = if page.printed { &mut m.tile_boxes } else { &mut m.tile_boxes_dim };
            boxes.push(rect_path(page.imageable));
        }
    }
    if slices_gate {
        let layout = cached(&mut app.canvas.slices, key, || st.doc.slice_layout());
        let prefs = &app.session.prefs;
        m.slice_color = Some(Srgba::from(pref_color(&prefs.slice_line_color, Color32::from_rgb(0xff, 0x3f, 0x3f))));
        m.slice_numbers = prefs.show_slice_numbers;
        for a in layout.iter() {
            let origin = match a.source {
                vectorcraft_doc::SliceSource::User => SliceOrigin::User,
                vectorcraft_doc::SliceSource::Object => SliceOrigin::Layer,
                vectorcraft_doc::SliceSource::Auto => SliceOrigin::Auto,
            };
            // PLAN: `SliceItem::id` is `Option<u32>`; a stored slice's node id is `NodeId(u64)`.
            m.slices.push(SliceItem::new(a.number, a.id.and_then(|id| u32::try_from(id.0).ok()), a.rect, origin));
            if a.id.is_some_and(|id| st.selection.slices.contains(&id) || st.selection.objects.contains(&id)) {
                m.slice_selected.push(a.number);
            }
        }
    }
    m
}

/// Puts the marks of one pass on the canvas body of `space`, over the art and under the tool's
/// widgets. Nothing shows in Presentation Mode. The marks follow the active document in every body.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, space: &CanvasSpace, _win: Win, _kind: BodyKind) {
    if app.ui.screen_mode == 3 {
        return;
    }
    let view = space.view;
    let marks = marks_of(app, &view);
    let rect = ui.max_rect();
    ArtMarks::new("Art marks", view, &marks).show_at(ui, rect);
    for (boxes, share) in [(&marks.tile_boxes, 1.0), (&marks.tile_boxes_dim, UNPRINTED_SHARE)] {
        if !boxes.is_empty() {
            SelectionOutline::dashed("Print tiling", boxes).color(DataColor::new(PRINT_TILING.gamma_multiply(share))).show_at(ui, rect);
        }
    }
    if let (false, Some(color)) = (marks.slices.is_empty(), marks.slice_color) {
        SliceOverlay::new("Slices", &marks.slices)
            .color(color)
            .also_selected(&marks.slice_selected)
            .numbers(marks.slice_numbers)
            .pick(false)
            .rules(SliceRules::ILLUSTRATOR)
            .show_at(ui, rect);
    }
    if !marks.hidden.is_empty() {
        // No frame is drawn, so no port takes the paper colour.
        let paper = DataColor::new(Color32::TRANSPARENT);
        TextFrameMarks::new("Hidden characters", view, &[], DataColor::new(HIDDEN_BLUE), paper).hidden_characters(&marks.hidden).show_at(ui, rect);
    }
}

/// The cached value of `slot` for `key`, made by `make` when the slot holds another key.
fn cached<T>(slot: &mut Option<((u64, u64), Arc<T>)>, key: (u64, u64), make: impl FnOnce() -> T) -> Arc<T> {
    if let Some((k, v)) = slot
        && *k == key
    {
        return v.clone();
    }
    let v = Arc::new(make());
    *slot = Some((key, v.clone()));
    v
}

/// The text objects in `layers`, depth first in paint order, leaving out hidden objects and what
/// is inside them.
fn text_nodes(layers: &[Arc<Node>]) -> Vec<Arc<Node>> {
    fn walk(nodes: &[Arc<Node>], out: &mut Vec<Arc<Node>>) {
        for n in nodes.iter().filter(|n| n.visible) {
            if matches!(n.kind, NodeKind::Text(_)) {
                out.push(n.clone());
            }
            if let Some(children) = n.children() {
                walk(children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(layers, &mut out);
    out
}

/// The missing links [`DocState::marked_missing_links`] marks in view: each one's box on the screen
/// (following a turned view) and its badge's place.
fn missing_links(st: &DocState, view: &CanvasView) -> Vec<([Pos2; 4], Pos2)> {
    st.marked_missing_links()
        .into_iter()
        .filter_map(|id| st.doc.node(id))
        .filter(|n| in_view(n, view))
        .filter_map(|n| n.geometric_bounds())
        .map(|b| (view.quad(b), view.to_screen(Point::new(b.x0, b.y0))))
        .collect()
}

/// Whether `n`'s box meets the part of the document the view shows (the box round the view's
/// corners, which a turned view makes larger than the canvas).
fn in_view(n: &Node, view: &CanvasView) -> bool {
    let v = view.visible();
    n.geometric_bounds().is_some_and(|b| b.x0 <= v.x1 && v.x0 <= b.x1 && b.y0 <= v.y1 && v.y0 <= b.y1)
}

/// The marks of the visible text objects in view, laid out again only for the objects the memo
/// doesn't hold as they are now. The objects not shown this pass are dropped from the memo.
fn collect_hidden(texts: &[Arc<Node>], view: &CanvasView, kept: &mut HiddenChars, out: &mut Vec<PlacedMark>) {
    let mut last = std::mem::take(&mut kept.marks);
    // The inputs are read before any layout: when fonts load during this pass, the next pass lays
    // all type out again.
    let inputs = (vectorcraft_text::FontDb::global().generation(), vectorcraft_text::hyphenation_exceptions());
    if kept.inputs != inputs {
        last.clear();
        kept.inputs = inputs;
    }
    for n in texts {
        let NodeKind::Text(tx) = &n.kind else { continue };
        if !in_view(n, view) {
            continue;
        }
        let marks = match last.remove(&n.id) {
            Some((was, marks)) if Arc::ptr_eq(&was, n) => marks,
            _ => Arc::new(HiddenMarks::of(tx)),
        };
        out.extend_from_slice(&marks);
        kept.marks.insert(n.id, (n.clone(), marks));
    }
}

/// The Auto Spell Check segments of the visible text objects in view, checked again only for the
/// objects the memo doesn't hold as they are now.
fn collect_spelling(st: &DocState, texts: &[Arc<Node>], view: &CanvasView, kept: &mut SpellingMarks, out: &mut Vec<(Point, Point)>) {
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
    for n in texts {
        let NodeKind::Text(tx) = &n.kind else { continue };
        if !auto_spell_checks(n) || !in_view(n, view) {
            continue;
        }
        let lines = match last.remove(&n.id) {
            Some((was, lines)) if Arc::ptr_eq(&was, n) => lines,
            _ => Arc::new(underlines(tx, &st.spelling_ignored)),
        };
        out.extend_from_slice(&lines);
        kept.lines.insert(n.id, (n.clone(), lines));
    }
}

/// Draw Inside's arms on the screen: round the corners of `bounds` (a few pixels outside the
/// object, clear of its stroke), each arm a third of its side at most and 16 points at most, so the
/// rectangle stays open. The corners follow a turned view.
pub(crate) fn inside_arms(view: &CanvasView, bounds: Rect) -> Vec<[Pos2; 2]> {
    let pad = 4.0 / view.zoom.max(1e-9);
    let [a, b, c, d] = view.quad(bounds.inflate(pad, pad));
    let mut arms = Vec::with_capacity(8);
    for (corner, sides) in [(a, [b, d]), (b, [c, a]), (c, [d, b]), (d, [a, c])] {
        for next in sides {
            let side = next - corner;
            let len = side.length();
            if len.is_finite() && len > 1e-3 {
                arms.push([corner, corner + side / len * (len / 3.0).min(16.0)]);
            }
        }
    }
    arms
}

/// Simplify's Show Original Path: the paths its preview ran on, as they were before it, with their
/// layers' colours. Nothing when they have more than [`OVERLAY_MAX_ANCHORS`] anchor points.
fn original_band(app: &VectorcraftApp) -> Vec<(BezPath, [u8; 3])> {
    let paths = crate::dialogs::path_ops::original_paths(app);
    let Some(st) = app.session.active().filter(|_| paths.iter().map(|(_, path)| path.anchor_count()).sum::<usize>() <= OVERLAY_MAX_ANCHORS) else {
        return vec![];
    };
    paths.into_iter().map(|(id, path)| (path.to_bezpath(), st.doc.layer_color(id))).collect()
}

/// The rectangle `[x0, y0, x1, y1]` as a closed path.
fn rect_path(r: [f64; 4]) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((r[0], r[1]));
    p.line_to((r[2], r[1]));
    p.line_to((r[2], r[3]));
    p.line_to((r[0], r[3]));
    p.close_path();
    p
}

/// The print tiles of `pages` on the screen under `view`: the paper edge and the number, which sits
/// at the top left of the imageable box.
pub(crate) fn tiles_of(pages: &[TilingPage], view: &CanvasView) -> Vec<Tile> {
    let quad = |r: [f64; 4]| view.quad(Rect::new(r[0], r[1], r[2], r[3]));
    pages
        .iter()
        .map(|page| {
            let edge = quad(page.page);
            let corner = quad(page.imageable).into_iter().reduce(|a, b| Pos2::new(a.x.min(b.x), a.y.min(b.y)));
            Tile {
                edge: edge.iter().chain(edge.first()).copied().collect(),
                at: corner.unwrap_or_default() + Vec2::new(3.0, 2.0),
                label: page.number.to_string(),
                printed: page.printed,
            }
        })
        .collect()
}

/// Type → Show Hidden Characters' placing: the characters that print no ink, and the ends of
/// paragraphs, lines and stories, as [`PlacedMark`]s in document space.
pub(crate) struct HiddenMarks;

impl HiddenMarks {
    /// Lay out `tx` and place its marks: a mark for each space, tab, nonbreaking space, space of a
    /// set width and discretionary hyphen, one after each line a forced line break ends, and ¶ or #
    /// after the lines that end a paragraph or the story. Type on a path has the marks over its
    /// characters, ¶ and the break mark at its breaks, and # after its last character when the path
    /// holds all of the text.
    pub(crate) fn of(tx: &vectorcraft_doc::TextObject) -> Vec<PlacedMark> {
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx);
        let text = tx.plain_text();
        // The ascent of the line holding `byte`, which sizes the mark.
        let ascent = |byte: usize| lay.lines.iter().find(|l| (l.start..l.end).contains(&byte)).or(lay.lines.first()).map_or(0.0, |l| l.ascent);
        let place = |mark: HiddenMark, at: Point, anchor: MarkAnchor, byte: usize| PlacedMark::new(mark, tx.xf * at, anchor, ascent(byte));
        let along = |angle: f64| vectorcraft_geom::Vec2::new(angle.cos(), angle.sin());
        let mut out = vec![];
        // A list marker's glyphs and a hyphen the layout adds at a break (length 0) stand for no
        // character. Glyphs before the start of an open path draw nothing. A character drawn with
        // several glyphs gets one mark.
        let mut marked = None;
        for g in lay.glyphs.iter().filter(|g| g.len > 0 && !g.off_path) {
            let Some(c) = text.get(g.byte..).and_then(|s| s.chars().next()) else { continue };
            if marked == Some(g.byte) {
                continue;
            }
            let d = along(g.angle);
            let (mark, at, anchor) = match c {
                // Type on a path is one line: its breaks are marked where their glyphs are.
                '\n' | '\u{2028}' if lay.on_path => {
                    (if c == '\n' { HiddenMark::ParagraphEnd } else { HiddenMark::LineBreak }, g.origin, MarkAnchor::Start)
                }
                '\n' | '\u{2028}' => continue,
                _ => match HiddenMark::of(c) {
                    Some(HiddenMark::Tab) => (HiddenMark::Tab, g.origin, MarkAnchor::Start),
                    Some(mark) => (mark, g.origin + d * (g.advance / 2.0), MarkAnchor::Centre),
                    None => continue,
                },
            };
            out.push(place(mark, at, anchor, g.byte));
            marked = Some(g.byte);
        }
        if lay.on_path {
            // # after the last character, when the path holds all of the text.
            let last = lay.glyphs.iter().filter(|g| g.len > 0 && !g.off_path).max_by_key(|g| g.byte + g.len);
            if let Some(g) = last.filter(|g| g.byte + g.len >= text.len() && !lay.overflow) {
                let d = along(g.angle);
                let at = if g.rtl { g.origin - d } else { g.origin + d * (g.advance + 1.0) };
                out.push(place(HiddenMark::StoryEnd, at, MarkAnchor::Start, g.byte));
            }
            return out;
        }
        // Lines are in line space: `physical_point` turns them into columns in vertical type.
        let at = |x: f64, y: f64| lay.physical_point(Point::new(x, y));
        for (i, line) in lay.lines.iter().enumerate() {
            let last = i + 1 == lay.lines.len();
            // A right-to-left line ends at its left edge.
            let (x, anchor) = match (lay.vertical, line.rtl) {
                (true, true) => (line.x0 - 1.0, MarkAnchor::Top),
                (true, false) => (line.x1 + 1.0, MarkAnchor::Top),
                (false, true) => (line.x0 - 1.0, MarkAnchor::End),
                (false, false) => (line.x1 + 1.0, MarkAnchor::Start),
            };
            let soft = text.get(line.start..line.end).is_some_and(|s| s.ends_with('\u{2028}'));
            let mark = if last && line.end >= text.len() {
                HiddenMark::StoryEnd
            } else if soft {
                HiddenMark::LineBreak
            } else if text.get(line.end..).is_some_and(|s| s.starts_with('\n')) {
                HiddenMark::ParagraphEnd
            } else {
                continue;
            };
            out.push(PlacedMark::new(mark, tx.xf * at(x, line.baseline), anchor, line.ascent));
        }
        out
    }
}

/// The style of [`ArtMarks`].
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ArtMarksStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The artboards' bleed outline.
    bleed: StyleColor,
    /// The artboards' marks (cross hairs, centre mark, video-safe areas).
    marks: StyleColor,
    /// The spelling waves.
    bad: StyleColor,
}

/// The marks the art carries, painted over the page after its content and under the tools' widgets:
/// the fade outside the artboards, the bleed and the artboards' marks, Simplify's original-path
/// band, Auto Spell Check's waves, Draw Inside's arms and the print tiles' edges and numbers.
/// Nothing here takes input or focus.
#[derive(Component, Props)]
#[component(canvas_control)]
pub(crate) struct ArtMarks<'a> {
    #[common]
    common: Common,
    /// The view that maps document points to the screen.
    view: CanvasView,
    /// The fade outside the artboards, the bleed and the artboards' marks.
    fade: &'a [[Pos2; 4]],
    bleed: &'a [BezPath],
    artboard_marks: &'a [BezPath],
    /// Simplify's original paths with their layer colours.
    original: &'a [(BezPath, [u8; 3])],
    /// The Auto Spell Check segments, in document space.
    spelling: &'a [(Point, Point)],
    /// Draw Inside's corner arms in screen points, and the layer colour they take.
    arms: &'a [[Pos2; 2]],
    arm_color: [u8; 3],
    /// The print tiles.
    tiles: &'a [Tile],
    /// The missing links' boxes and badges.
    missing: &'a [([Pos2; 4], Pos2)],
}

impl<'a> ArtMarks<'a> {
    /// The marks of one pass seen through `view`, named `label`.
    pub(crate) fn new(label: impl Into<Label>, view: CanvasView, marks: &'a Marks) -> Self {
        ArtMarks {
            common: Common::new(),
            view,
            fade: &marks.fade,
            bleed: &marks.bleed,
            artboard_marks: &marks.artboard_marks,
            original: &marks.original,
            spelling: &marks.spelling,
            arms: &marks.arms,
            arm_color: marks.arm_color,
            tiles: &marks.tiles,
            missing: &marks.missing,
        }
        .label(label)
    }
}

impl Component for ArtMarks<'_> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = ArtMarksStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ArtMarksStyleKey) -> ArtMarksStyle {
        ArtMarksStyle {
            metrics: Metrics::new(),
            bleed: cx.color(cx.tokens.color.status.bad.solid),
            marks: cx.color(cx.tokens.color.overlay.grid),
            bad: cx.tone(Tone::Bad),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = &cx.style;
        let draw = ui.draw();
        for quad in self.fade {
            draw.data_fill(&PaintShape::Polygon(quad.to_vec()), FADE_OUTSIDE);
        }
        let bleed = self.bleed.iter().map(|path| (path, style.bleed));
        for (path, color) in bleed.chain(self.artboard_marks.iter().map(|path| (path, style.marks))) {
            for (points, closed) in path_runs(&self.view, path) {
                draw.stroke(&PaintShape::Path(ring(points, closed)), StrokeWidth::Thin, color);
            }
        }
        draw.faded(BAND_SHARE, |d| {
            for (path, rgb) in self.original {
                for (points, closed) in path_runs(&self.view, path) {
                    d.stroke(&PaintShape::Path(ring(points, closed)), StrokeWidth::Thick, layer(*rgb));
                }
            }
        });
        for &(a, b) in self.spelling {
            draw.stroke(&PaintShape::Path(wave(self.view.to_screen(a), self.view.to_screen(b))), StrokeWidth::Thin, style.bad);
        }
        for arm in self.arms {
            draw.dashed(arm, draw.dashes(), StrokeWidth::Thin, layer(self.arm_color));
        }
        for tile in self.tiles {
            paint_tile(&draw, tile);
        }
        for (quad, at) in self.missing {
            paint_missing(&draw, quad, *at, style.bad);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A layer's colour as data.
fn layer(rgb: [u8; 3]) -> DataColor {
    DataColor::new(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
}

/// `points`, closed back to the first when the run closes.
fn ring(mut points: Vec<Pos2>, closed: bool) -> Vec<Pos2> {
    if let (true, Some(&first)) = (closed, points.first()) {
        points.push(first);
    }
    points
}

/// One print tile: the paper's edge and the number, dimmer when the tile doesn't print.
fn paint_tile(draw: &Draw<'_>, tile: &Tile) {
    let share = if tile.printed { 1.0 } else { UNPRINTED_SHARE };
    let color = DataColor::new(PRINT_TILING.gamma_multiply(share));
    draw.stroke(&PaintShape::Path(tile.edge.clone()), StrokeWidth::Hairline, color);
    let run = [DocRun::new(tile.label.as_str(), FontRef::proportional(), TILE_NUMBER_SIZE, color)];
    let room = egui::Rect::from_min_size(tile.at, Vec2::new(TILE_NUMBER_SIZE * 0.7 * tile.label.len().max(1) as f32, TILE_NUMBER_SIZE * 1.4));
    draw.doc(room, &DocText::new(&run));
}

/// One missing link: a thick outline round its box and, at the box's corner, a filled badge with a
/// question mark, so the mark doesn't rely on its colour alone.
fn paint_missing(draw: &Draw<'_>, quad: &[Pos2; 4], at: Pos2, color: StyleColor) {
    draw.stroke(&PaintShape::Path(ring(quad.to_vec(), true)), StrokeWidth::Thick, color);
    draw.fill(&PaintShape::Circle(at, MISSING_BADGE_RADIUS), color);
    let run = [DocRun::new("?", FontRef::proportional(), MISSING_BADGE_TEXT, DataColor::new(Color32::WHITE))];
    let room = egui::Rect::from_min_size(at - Vec2::new(MISSING_BADGE_TEXT * 0.3, MISSING_BADGE_TEXT * 0.7), Vec2::splat(MISSING_BADGE_TEXT * 1.4));
    draw.doc(room, &DocText::new(&run));
}

/// A wavy line from `a` to `b` on screen, waves 4 pixels long and 1.5 high, down the page from the
/// line whichever way the type runs; empty for a segment under a pixel long or not finite.
pub(crate) fn wave(a: Pos2, b: Pos2) -> Vec<Pos2> {
    let d = b - a;
    let len = d.length();
    if !len.is_finite() || len < 1.0 {
        return vec![];
    }
    let along = d / len;
    let across = Vec2::new(-along.y, along.x);
    let n = ((len / WAVE_STEP) as usize).clamp(1, WAVE_MAX);
    let step = len / n as f32;
    (0..=n).map(|i| a + along * (step * i as f32) + across * if i % 2 == 0 { 0.0 } else { WAVE_HIGH }).collect()
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
    let mut top: Option<(NodeId, bool)> = None;
    // Back to front as drawn at `p`: an Intertwine group's members by their tier there, those of
    // one tier in their stacking order ([`vectorcraft_doc::intertwine`]).
    fn walk_at(nodes: &[Arc<Node>], p: Point, locked: bool, f: &mut impl FnMut(&Node, bool)) {
        for n in nodes.iter().filter(|n| n.visible) {
            let locked = locked || n.locked;
            f(n, locked);
            let Some(children) = n.children() else { continue };
            if matches!(n.kind, NodeKind::Intertwine { .. }) {
                let mut drawn = children.to_vec();
                drawn.sort_by_key(|c| vectorcraft_doc::intertwine::tier_at(c, p));
                walk_at(&drawn, p, locked, f);
            } else {
                walk_at(children, p, locked, f);
            }
        }
    }
    // A later hit is on top.
    walk_at(&st.doc.layers, p, false, &mut |n, locked| {
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

/// View › Show Text Threads: for each thread with a selected frame, a line in its first frame's
/// layer colour from each frame's out port to the next frame's in port, both showing the arrow of a
/// threaded port, where the tools draw the ports of selected type ([`vectorcraft_tools::text`]).
/// Only frames with ports ([`vectorcraft_tools::text::ported`]) are joined: a link to or from a
/// frame in perspective, which has none, gets no line. With more than
/// [`vectorcraft_tools::text::PORTS_MAX`] such frames selected the tools draw no ports, and only
/// the lines are drawn here. `zoom`: screen pixels to the document unit.
pub(in crate::canvas) fn thread_overlays(app: &VectorcraftApp, zoom: f64) -> Vec<Overlay> {
    use std::collections::HashSet;
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
pub(in crate::canvas) fn live_paint_gap_overlays(app: &mut VectorcraftApp) -> Vec<Overlay> {
    let Some(st) = app.session.active() else { return vec![] };
    let key = (st.uid, st.revision);
    let gaps = match &app.canvas.live_paint_gaps {
        Some((k, gaps)) if *k == key => gaps.clone(),
        _ => {
            let gaps = Arc::new(crate::dialogs::live_paint_gaps::selected_gaps(app));
            app.canvas.live_paint_gaps = Some((key, gaps.clone()));
            gaps
        }
    };
    gaps.to_vec()
}
