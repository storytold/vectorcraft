//! VectorCraft tools: pointer events in → commands and overlays out.
//!
//! Tools never mutate the document directly. They emit [`Action`]s that the engine executes:
//! `Begin` snapshots the document, each `Preview` re-applies one command on top of that snapshot
//! (replacing the previous preview), and `Commit` records the last preview as a single undo step
//! and journal entry. One-shot `Exec` actions run immediately. This makes every gesture replayable
//! by the control channel and MCP, and keeps tools testable without a UI.
#![forbid(unsafe_code)]

pub mod bbox;
pub mod builder;
pub mod catalog;
pub mod corners;
pub mod cropimage;
pub mod cut;
pub mod dimension;
pub mod direct;
pub mod distort;
pub mod draw2;
pub mod extra;
pub mod guides;
pub mod intertwine;
pub mod meshblend;
pub mod meshedit;
pub mod mirror;
pub mod objects_on_path;
pub mod params;
pub mod pathtype;
pub mod pen;
pub mod pie;
pub mod place;
pub mod printtiling;
pub mod repeat_grid;
pub mod rulerguide;
pub mod select;
pub mod settings;
pub mod shape;
pub mod shaperwidget;
pub mod slice;
pub mod symbolism;
pub mod text;
pub mod touchtype;
pub mod typewidget;
pub mod xform;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, NodeId, Selection, Unit};
use vectorcraft_geom::{BezPath, Point, Rect, Vec2};

pub use catalog::{TOOL_GROUPS, ToolInfo, tool_info};

mod context;
mod cursor;
mod input;

pub use context::{GlyphSnap, PaintDefaults, ScreenFrame, ToolContext};
pub use cursor::Cursor;
pub use input::{Action, Mods, PointerEvent, PointerKind, ToolKey};

/// Visual feedback drawn by the UI in screen space (coordinates here are document space).
#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    /// Marquee / rubber band rectangle.
    Marquee(Rect),
    /// A path preview (e.g. pen rubber band) in a colour (RGB).
    Path { path: BezPath, color: [u8; 3], width: f32, dashed: bool },
    /// A line segment (smart guide, handle line).
    Line { a: Point, b: Point, color: [u8; 3], dashed: bool },
    /// An anchor square: filled = selected.
    Anchor { p: Point, color: [u8; 3], filled: bool, size: f32 },
    /// A Curvature tool point, drawn as a circle; filled = the last touched point.
    CurvePoint { p: Point, color: [u8; 3], filled: bool, size: f32 },
    /// A handle end circle.
    Handle { p: Point, color: [u8; 3] },
    /// Smart-guide style label (e.g. "anchor", "W: 100 pt").
    Label { p: Point, text: String, color: [u8; 3] },
    /// Measurement pill near the cursor (grey box with white text).
    Measure { p: Point, text: String },
    /// A Distance Guides measurement: a line from `a` to `b` with a tick across each end, and
    /// `text` (its length) in a pill of `color` on its middle.
    Distance { a: Point, b: Point, text: String, color: [u8; 3] },
    /// Translucent filled quad (text selection highlight), RGBA.
    Highlight { quad: [Point; 4], color: [u8; 4] },
    /// A colour chip of fixed screen size (a gradient stop), RGBA; ringed when selected.
    Swatch { p: Point, color: [u8; 4], selected: bool },
    /// A hairline in a translucent colour (perspective gridlines), RGBA.
    GridLine { a: Point, b: Point, color: [u8; 4] },
}

/// What an IME's "delete the text around the cursor" counts: UTF-8 bytes (winit's raw
/// `DeleteSurroundingBytes`) or Unicode scalar values (egui's `DeleteSurrounding`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurroundingUnit {
    Bytes,
    Chars,
}

/// The committed logical text around an IME cursor, or the documented reason it is unavailable.
///
/// This type stays free of any UI dependency: offsets are UTF-8 bytes into `text`, `cursor` is the
/// primary selection end and `anchor` the other (so `cursor > anchor` is a reversed selection).
/// `text` is already bounded to the caller's request and has the pre-edit removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeSurrounding {
    Available { text: String, cursor: usize, anchor: usize },
    Unavailable(&'static str),
}

impl Default for ImeSurrounding {
    /// Report surrounding text as unavailable because no document is active.
    fn default() -> Self {
        Self::Unavailable("no document is active")
    }
}

/// A tool state machine.
pub trait Tool: Send {
    fn id(&self) -> &'static str;
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action>;
    fn key(&mut self, _cx: &ToolContext, _key: ToolKey, _mods: Mods) -> Vec<Action> {
        vec![]
    }
    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        vec![]
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Arrow
    }
    /// Tool options as JSON (shown by the Control bar / tool options dialog).
    fn options(&self) -> Value {
        Value::Null
    }
    fn set_option(&mut self, _key: &str, _value: &Value) {}
    /// Is an interaction in progress (drag, open pen path)?
    fn busy(&self) -> bool {
        false
    }
    /// Is a drag moving, scaling or rotating the selection (the canvas then hides the bounding box,
    /// so only the art is seen going with the pointer)?
    fn transforming(&self) -> bool {
        false
    }
    /// Does the tool take `key` now, ahead of the command shortcuts bound to it (the Gradient tool
    /// with a stop selected takes Delete and the arrows)?
    fn claims_key(&self, _cx: &ToolContext, _key: ToolKey) -> bool {
        false
    }
    /// Does the tool want typed text (Type tool editing)? Single-key shortcuts are suppressed.
    fn wants_text(&self) -> bool {
        false
    }
    fn text_input(&mut self, _cx: &ToolContext, _s: &str) -> Vec<Action> {
        vec![]
    }
    /// Would [`Tool::text_input`] insert `text` exactly as given? False when it filters control
    /// characters, normalizes newlines, applies Typographer's Quotes or otherwise rewrites the
    /// insertion. An IME commit the application rewrites is an application edit: its receipt must
    /// be `Other`, never `InputMethod`.
    fn text_input_is_verbatim(&self, _cx: &ToolContext, _text: &str) -> bool {
        true
    }
    /// Does the tool's open typing session still match the document? False when an external edit
    /// replaced the text under it: the next IME commit starts a fresh session, which is an
    /// application-side reset, not an accepted IME insertion.
    fn ime_session_matches_document(&self, _cx: &ToolContext) -> bool {
        true
    }
    /// IME composition (marked text) for the tool that wants text: `text` replaces the previous
    /// marked text (or the selection); an empty `text` ends the composition. `active_chars` is the
    /// clause being converted, in characters of `text`. The committed result arrives through
    /// [`Tool::text_input`].
    fn ime_preedit(&mut self, _cx: &ToolContext, _text: &str, _active_chars: Option<std::ops::Range<usize>>) -> Vec<Action> {
        vec![]
    }
    /// An input method replaces text around its composition: delete `before` and `after` units
    /// (bytes or characters, per [`SurroundingUnit`]) around the selection or pre-edit, in the
    /// same typing session as the commit that follows. The selection/pre-edit itself is preserved
    /// (the following commit replaces it), its direction is kept, and a unit count ending inside a
    /// UTF-8 scalar leaves that scalar whole.
    fn ime_delete_surrounding(&mut self, _cx: &ToolContext, _before: usize, _after: usize, _unit: SurroundingUnit) -> Vec<Action> {
        vec![]
    }
    /// Stable IME owner data for the tool's current edit target: `(document uid, target id,
    /// replacement epoch)`. The owner survives ordinary edits and caret moves; the replacement
    /// epoch changes when the same target is started afresh (a new edit session).
    fn ime_target(&self, _cx: &ToolContext) -> Option<(u64, u64, u64)> {
        None
    }
    /// The committed logical text around the IME cursor, with UTF-8 byte offsets (`cursor` is the
    /// primary end, `anchor` the other): the source for an outgoing surrounding-text payload
    /// (egui's E21b). Pre-edit text is excluded and the offsets are remapped into the remaining
    /// text; at most `max_bytes` bytes are copied (a wider selection is explicitly unavailable).
    fn ime_surrounding(&self, _cx: &ToolContext, _max_bytes: usize) -> ImeSurrounding {
        ImeSurrounding::Unavailable("the active tool has no IME text context")
    }
    /// Is uncommitted IME text being shown? Keys, shortcuts and Undo wait while it is.
    fn composing(&self) -> bool {
        false
    }
    /// Where the IME candidate window goes: the caret line (top, bottom) in document space.
    fn ime_caret(&self, _cx: &ToolContext) -> Option<(Point, Point)> {
        None
    }
    fn notify(&mut self, _cx: &ToolContext, _what: &str) {}
    /// The pointer stopped hovering for the tool while it holds no press: it left the canvas (for a
    /// panel or out of the window), went under a floating panel, menu, popup or dialog, or a press
    /// started panning, zooming or turning the view. Drop what shows only for the hover (the
    /// Selection tool's Distance Guides).
    fn hover_ended(&mut self, _cx: &ToolContext) {}
    /// Whether what the tool shows for the hover depends on what lies under a still pointer: then
    /// the canvas hovers it again where the pointer is when the view, the document or the
    /// selection changes (the Selection tool while Alt shows Distance Guides).
    fn wants_rehover(&self) -> bool {
        false
    }
    /// Called when the user switches away (finish pending work).
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        vec![]
    }
    /// The tool, already active, is chosen again (its toolbar button, its shortcut, `tool.select`).
    /// Most tools keep their state; the Blend tool finishes the blend it is adding to.
    fn chosen_again(&mut self, _cx: &ToolContext) -> Vec<Action> {
        vec![]
    }
    /// A command from outside the tool ran (a menu, a panel, an agent): finish work it ended (the
    /// Type tool stops editing text the command took out of the selection).
    fn after_command(&mut self, _cx: &ToolContext) -> Vec<Action> {
        vec![]
    }
    /// Time passes while the pointer button is held (`dt` seconds since the last tick, whether or
    /// not the pointer moved): tools that keep working while the brush holds still (Twirl, Pucker,
    /// Bloat) act on it. The host supplies the time, so tests and agents drive it exactly.
    fn tick(&mut self, _cx: &ToolContext, _dt: f64) -> Vec<Action> {
        vec![]
    }
    /// Does the tool want [`Tool::tick`]s now (the host keeps time only then)?
    fn wants_ticks(&self) -> bool {
        false
    }
    /// Does the tool build a stroke from the drag's samples (the Pencil and the other freehand
    /// tools)? The host then hands it every position the pointer passed between two frames, not
    /// only the last, so a fast stroke keeps its curve. Every other tool gets one drag a frame, so
    /// one whose drag preview is costly (moving a large selection, Free Distort, the Width tool)
    /// doesn't redo it for each sample.
    fn wants_samples(&self) -> bool {
        false
    }
    /// Is the tool waiting for work it runs on another thread (the planar map of a large selection
    /// for the Shape Builder and the Live Paint Bucket)? It highlights no region under the pointer
    /// until the work is done. While this holds, the desktop app calls [`Tool::poll`] each frame,
    /// and hosts without frames call it with `wait` after a [`PointerKind::Move`].
    fn waiting(&self) -> bool {
        false
    }
    /// Checks on the work [`Tool::waiting`] reports, waiting for it to finish with `wait`. Once it
    /// is done, the tool shows the highlight under the pointer where it last moved.
    fn poll(&mut self, _cx: &ToolContext, _wait: bool) {}
    /// The Shape Builder's planar map of the selection, with the outline edges, and the gap it
    /// closes, when the tool has one. `shapeBuilder.merge` uses it in place of a map of its own
    /// when the map's inputs are the merge's inputs.
    fn shape_builder_map(&self) -> Option<(std::sync::Arc<builder::BuilderMap>, f64)> {
        None
    }
}

/// Create a tool by id. Unknown or not-yet-implemented tools fall back to a no-op tool that keeps the id.
pub fn create(id: &str) -> Box<dyn Tool> {
    match id {
        "selection" => Box::new(select::SelectionTool::default()),
        "directSelection" => Box::new(direct::DirectSelectionTool::new(false)),
        "groupSelection" => Box::new(direct::DirectSelectionTool::new(true)),
        "pen" => Box::new(pen::PenTool::default()),
        "dimension" => Box::new(dimension::DimensionTool::default()),
        "type" | "areaType" | "typeOnPath" | "verticalType" | "verticalAreaType" | "verticalTypeOnPath" => Box::new(text::TypeTool::new(id)),
        "rectangle" | "roundedRectangle" | "ellipse" | "polygon" | "star" | "lineSegment" => Box::new(shape::ShapeTool::new(id)),
        // Not in the toolbar: `file.place.queue` loads it.
        "place" => Box::new(place::PlaceTool::default()),
        other => symbolism::create(other)
            .or_else(|| builder::create(other))
            .or_else(|| draw2::create(other))
            .or_else(|| xform::create(other))
            .or_else(|| meshblend::create(other))
            .or_else(|| distort::create(other))
            .or_else(|| extra::create(other))
            .or_else(|| slice::create(other))
            .or_else(|| printtiling::create(other))
            .or_else(|| cropimage::create(other))
            .or_else(|| cut::create(other))
            .or_else(|| intertwine::create(other))
            .or_else(|| objects_on_path::create(other))
            .or_else(|| touchtype::create(other))
            .unwrap_or_else(|| Box::new(NoopTool(tool_info(other).map(|t| t.id).unwrap_or("selection")))),
    }
}

/// Placeholder for tools whose behaviour hasn't landed yet (selecting them still works).
pub struct NoopTool(&'static str);

impl Tool for NoopTool {
    fn id(&self) -> &'static str {
        self.0
    }
    fn pointer(&mut self, _cx: &ToolContext, _ev: &PointerEvent) -> Vec<Action> {
        vec![]
    }
}

/// The most samples a drawn stroke (freehand, Liquify) keeps: later drags add nothing to it.
pub(crate) const MAX_STROKE_SAMPLES: usize = 100_000;

/// Samples up to which every drag previews the whole stroke; past it, a drag previews it every
/// `n / PREVIEW_ALL_UNTIL` samples. Each preview carries every sample so far, so previewing at
/// every sample cost the square of the stroke's length (a gesture of many drags sent at once
/// spent seconds); this way it is about `PREVIEW_ALL_UNTIL` per sample.
const PREVIEW_ALL_UNTIL: usize = 1000;

/// Whether a drag that made the stroke `n` samples long previews it ([`PREVIEW_ALL_UNTIL`]).
pub(crate) fn previews_at(n: usize) -> bool {
    n <= PREVIEW_ALL_UNTIL || n.is_multiple_of(n / PREVIEW_ALL_UNTIL)
}

pub(crate) fn json_ids(ids: &[NodeId]) -> Value {
    Value::Array(ids.iter().map(|i| Value::from(i.0)).collect())
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::shapes;

    pub fn doc_with_rect() -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(100.0, 100.0, 200.0, 200.0)), Appearance::default_art())).unwrap();
        (d, id)
    }

    /// [`doc_with_rect`] plus a clip group: a filled 200 × 200 square (350, 350)–(550, 550)
    /// clipped to an unpainted rectangle (400, 400)–(500, 500), so the art lies over the mask's
    /// edges. The group, the clipping path and the art.
    pub fn doc_with_masked_square() -> (Document, NodeId, NodeId, NodeId) {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let (g, clip, art) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let mut path = Node::path(clip, shapes::rectangle(Rect::new(400.0, 400.0, 500.0, 500.0)), Default::default());
        if let vectorcraft_doc::NodeKind::Path { clipping, .. } = &mut path.kind {
            *clipping = true;
        }
        let picture = Node::path(art, shapes::rectangle(Rect::new(350.0, 350.0, 550.0, 550.0)), Appearance::default_art());
        let group =
            Node::new(g, vectorcraft_doc::NodeKind::Group { children: vec![std::sync::Arc::new(path), std::sync::Arc::new(picture)], clip: true });
        d.insert(Some(l), 1, group).unwrap();
        (d, g, clip, art)
    }

    /// A new layer of `d`, above the others, with the unpainted rectangle `mask` as its clipping
    /// mask and `art` clipped by it: the layer and the clipping mask.
    pub fn add_masked_layer(d: &mut Document, mask: Rect, art: Node) -> (NodeId, NodeId) {
        let l = d.add_layer(Some("Masked"));
        let clip = d.alloc_id();
        let mut path = Node::path(clip, shapes::rectangle(mask), Default::default());
        if let vectorcraft_doc::NodeKind::Path { clipping, .. } = &mut path.kind {
            *clipping = true;
        }
        d.insert(Some(l), 0, path).unwrap();
        d.insert(Some(l), 1, art).unwrap();
        d.node_mut(l).unwrap().set_clips(true);
        (l, clip)
    }

    /// A layer clipped to (250, 250)–(360, 400), its mask's right edge through 120 × 40 area type
    /// at (300, 300): the type, and a point on it just inside the mask's edge, within the
    /// selection tolerance, where the selection tools take the mask.
    pub fn doc_with_type_on_a_layer_masks_edge() -> (Document, NodeId, Point) {
        let (mut d, _) = doc_with_rect();
        let id = d.alloc_id();
        let mut t = vectorcraft_doc::TextObject::point(Point::new(300.0, 300.0), "Some words", Default::default());
        t.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 120.0, 40.0)) };
        add_masked_layer(&mut d, Rect::new(250.0, 250.0, 360.0, 400.0), Node::new(id, vectorcraft_doc::NodeKind::Text(Box::new(t))));
        (d, id, Point::new(358.5, 320.0))
    }

    /// [`doc_with_rect`] plus 120 × 40 area type at (300, 300).
    pub fn doc_with_area_type() -> (Document, NodeId) {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut t = vectorcraft_doc::TextObject::point(Point::new(300.0, 300.0), "Some words", Default::default());
        t.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 120.0, 40.0)) };
        d.insert(Some(l), 1, Node::new(id, vectorcraft_doc::NodeKind::Text(Box::new(t)))).unwrap();
        (d, id)
    }

    /// [`doc_with_rect`] plus type on a 300 pt path from (100, 300) to the right, from 20 % on.
    pub fn doc_with_path_type() -> (Document, NodeId) {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut t = vectorcraft_doc::TextObject::point(Point::ZERO, "Path type", Default::default());
        let path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
            vectorcraft_geom::PathEl::MoveTo(Point::new(100.0, 300.0)),
            vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
        ]));
        t.kind = vectorcraft_doc::TextKind::OnPath { path, start: 0.2, end: None };
        d.insert(Some(l), 1, Node::new(id, vectorcraft_doc::NodeKind::Text(Box::new(t)))).unwrap();
        (d, id)
    }

    /// [`doc_with_rect`] plus a Shaper Group of two source squares (x from 0 to 100 and from 50 to
    /// 150, y from 300 to 400) whose visible face is the first square: the document, the group, its
    /// source shapes and the face.
    pub fn doc_with_shaper_group() -> (Document, NodeId, NodeId, NodeId) {
        use std::sync::Arc;
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let square = |x: f64| shapes::rectangle(Rect::new(x, 300.0, x + 100.0, 400.0));
        let a = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
        let b = Node::path(d.alloc_id(), square(50.0), Appearance::default_art());
        let mut sources = Node::group(d.alloc_id(), vec![Arc::new(a), Arc::new(b)]);
        sources.name = Some(vectorcraft_doc::shaper::SOURCES.into());
        sources.opacity = 0.0;
        let mut face = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
        face.name = Some(format!("{}0", vectorcraft_doc::shaper::FILL_PREFIX));
        let (sources_id, face_id) = (sources.id, face.id);
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(sources), Arc::new(face)]);
        group.extras_mut().shaper = Some(Box::default());
        let group_id = group.id;
        d.insert(Some(l), 1, group).unwrap();
        (d, group_id, sources_id, face_id)
    }

    pub fn paint() -> PaintDefaults {
        PaintDefaults::default()
    }

    /// An 800 × 600 window on a view at 100% turned `deg` degrees clockwise (Rotate View): at 90°
    /// one pixel right on screen is one point up the page.
    pub fn turned_view(deg: f64) -> Option<ScreenFrame> {
        let (sn, cs) = deg.to_radians().sin_cos();
        Some(ScreenFrame { origin: Point::ORIGIN, right: Vec2::new(cs, -sn), down: Vec2::new(sn, cs), size: (800.0, 600.0) })
    }

    pub fn cx<'a>(d: &'a Document, s: &'a Selection, p: &'a PaintDefaults) -> ToolContext<'a> {
        ToolContext {
            doc: d,
            ruler_origin: d.ruler_origin_for(0),
            revision: (0, 0),
            selection: s,
            zoom: 1.0,
            isolation: None,
            paint: p,
            outline: false,
            smart_guides: true,
            guides: true,
            snap_to_grid: false,
            show_bbox: true,
            snap_to_pixel: false,
            snap_to_point: true,
            snap_tangent: true,
            snap_perpendicular: true,
            snap_to_glyph: false,
            glyph_snap: GlyphSnap::default(),
            corner_widgets: true,
            fill_active: true,
            gradient_stop: None,
            appearance_item: None,
            constrain_angle: 0.0,
            keyboard_increment: 1.0,
            freeform_point: None,
            raster_sample: 1,
            preview_bounds: false,
            preview_memo: None,
            unit: Unit::Points,
            stroke_unit: Unit::Points,
            paste_plain_text: false,
            slices_hidden: false,
            slices_locked: false,
            auto_add_delete: true,
            selection_tolerance: 3.0,
            anchor_size: 3,
            path_only: false,
            type_path_only: false,
            double_click_isolate: true,
            select_behind: true,
            highlight_anchors: true,
            snap_tolerance: 2.0,
            handles_multiple: true,
            corner_widget_max_angle: 177.0,
            move_locked_with_artboard: false,
            pen_rubber_band: true,
            curvature_rubber_band: true,
            placeholder_text: false,
            smart_guide_color: guides::MAGENTA,
            alignment_guides: true,
            anchor_path_labels: true,
            measurement_labels: true,
            transform_tools_guides: true,
            spacing_guides: true,
            distance_guides: true,
            snapping_tolerance: 4.0,
            construction_angles: guides::construction_angles(guides::DEFAULT_CONSTRUCTION_ANGLES),
            screen: None,
            plane_widget: Some(Default::default()),
        }
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::testutil::{cx, doc_with_rect, paint};
    use super::*;

    fn cursor_at(id: &str, d: &Document, p: Point) -> Cursor {
        let (s, paint) = (Selection::default(), paint());
        create(id).cursor(&cx(d, &s, &paint), p, Mods::default())
    }

    /// Each of these tools has a cursor of its own (borrowing the crosshair until it's drawn), and
    /// Use Precise Cursors turns it into the crosshair.
    #[test]
    fn drawing_tools_show_their_own_cursor() {
        let (d, _) = doc_with_rect();
        let away = Point::new(400.0, 400.0);
        for (id, c) in [
            ("lasso", Cursor::Lasso),
            ("magicWand", Cursor::MagicWand),
            ("paintbrush", Cursor::Paintbrush),
            ("blobBrush", Cursor::BlobBrush),
            ("eraser", Cursor::Eraser),
            ("smooth", Cursor::Smooth),
            ("pathEraser", Cursor::PathEraser),
            ("join", Cursor::Join),
            ("knife", Cursor::Knife),
            ("shaper", Cursor::Shaper),
            ("livePaintBucket", Cursor::LivePaintBucket),
            ("measure", Cursor::Measure),
            ("printTiling", Cursor::PrintTiling),
            ("symbolSprayer", Cursor::SymbolSprayer),
            ("symbolShifter", Cursor::SymbolShifter),
            ("symbolScruncher", Cursor::SymbolScruncher),
            ("symbolSizer", Cursor::SymbolSizer),
            ("symbolSpinner", Cursor::SymbolSpinner),
            ("symbolStainer", Cursor::SymbolStainer),
            ("symbolScreener", Cursor::SymbolScreener),
            ("symbolStyler", Cursor::SymbolStyler),
            ("mesh", Cursor::MeshAdd),
        ] {
            assert_eq!(cursor_at(id, &d, away), c, "{id}");
            assert_eq!(c.precise(), Cursor::Crosshair, "{id}");
            assert_eq!(c.stand_in(), Some(if id == "mesh" { Cursor::PenAdd } else { Cursor::Crosshair }), "{id}");
        }
        // Away from a selected path the Pencil shows its new-path cursor, drawn with a star; its
        // own cursor, for drawing and for a selected path under the pointer, borrows the crosshair.
        assert_eq!(cursor_at("pencil", &d, away), Cursor::PencilNew);
        assert_eq!(Cursor::PencilNew.precise(), Cursor::Crosshair);
        assert_eq!(Cursor::PencilNew.stand_in(), None);
        assert_eq!(Cursor::Pencil.precise(), Cursor::Crosshair);
        assert_eq!(Cursor::Pencil.stand_in(), Some(Cursor::Crosshair));
    }

    #[test]
    fn the_vertical_type_tools_show_the_turned_i_beam() {
        let (d, _) = doc_with_rect();
        let away = Point::new(400.0, 400.0);
        for id in ["verticalType", "verticalAreaType", "verticalTypeOnPath"] {
            assert_eq!(cursor_at(id, &d, away), Cursor::TypeVertical, "{id}");
        }
        assert_eq!(cursor_at("type", &d, away), Cursor::Text);
        assert_eq!(Cursor::TypeVertical.stand_in(), Some(Cursor::Text));
    }

    #[test]
    fn the_scissors_say_whether_a_click_cuts() {
        let (d, _) = doc_with_rect();
        assert_eq!(cursor_at("scissors", &d, Point::new(100.0, 150.0)), Cursor::Scissors, "over a segment");
        assert_eq!(cursor_at("scissors", &d, Point::new(400.0, 400.0)), Cursor::ScissorsNotAllowed, "away from paths");
        assert_eq!(Cursor::ScissorsNotAllowed.stand_in(), Some(Cursor::NotAllowed));
        assert_eq!(Cursor::ScissorsNotAllowed.precise(), Cursor::ScissorsNotAllowed, "nothing to aim at");
    }
}
