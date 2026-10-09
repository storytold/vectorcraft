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
pub mod direct;
pub mod distort;
pub mod draw2;
pub mod extra;
pub mod guides;
pub mod meshblend;
pub mod meshedit;
pub mod params;
pub mod pathtype;
pub mod pen;
pub mod place;
pub mod printtiling;
pub mod rulerguide;
pub mod select;
pub mod settings;
pub mod shape;
pub mod slice;
pub mod symbolism;
pub mod text;
pub mod typewidget;
pub mod xform;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, NodeId, Selection, Unit};
use vectorcraft_geom::{BezPath, Point, Rect, Vec2};

pub use catalog::{TOOL_GROUPS, ToolInfo, tool_info};

/// Modifier keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mods {
    #[serde(default)]
    pub shift: bool,
    /// Option on macOS.
    #[serde(default)]
    pub alt: bool,
    /// Command on macOS, Ctrl elsewhere.
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub space: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerKind {
    /// Button pressed.
    Down,
    /// Moved with the button held.
    Drag,
    /// Button released.
    Up,
    /// Moved with no button (hover).
    Move,
    DoubleClick,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointerEvent {
    pub kind: PointerKind,
    /// Document coordinates.
    pub pos: Point,
    #[serde(default)]
    pub mods: Mods,
    #[serde(default = "one")]
    pub pressure: f32,
}

fn one() -> f32 {
    1.0
}

impl PointerEvent {
    pub fn new(kind: PointerKind, x: f64, y: f64) -> Self {
        Self { kind, pos: Point::new(x, y), mods: Mods::default(), pressure: 1.0 }
    }
    pub fn with_mods(mut self, m: Mods) -> Self {
        self.mods = m;
        self
    }
    /// The pen `pressure` of a pointer event given as JSON (control channel, MCP): 0..1, default 1.
    pub fn json_pressure(e: &Value) -> f32 {
        e.get("pressure").and_then(Value::as_f64).filter(|f| f.is_finite()).map_or(1.0, |f| f.clamp(0.0, 1.0) as f32)
    }
    /// How long the pointer then holds still, in seconds, from a JSON pointer event's `holdMs`
    /// (0..60000; default 0): the time [`Tool::tick`] gets.
    pub fn json_hold(e: &Value) -> f64 {
        e.get("holdMs").and_then(Value::as_f64).filter(|f| f.is_finite()).map_or(0.0, |ms| ms.clamp(0.0, 60_000.0) / 1000.0)
    }
}

/// Keys tools care about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKey {
    Enter,
    Escape,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    /// Illustrator: ↑/↓ while drawing a polygon/star change sides/points; `[`/`]` change sizes.
    BracketLeft,
    BracketRight,
    Tab,
    Home,
    End,
    /// A digit key 0–9 (5 while dragging with the Perspective Selection tool moves perpendicular
    /// to the plane).
    Digit(u8),
}

/// What a tool asks the engine to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Start an interaction (snapshot the document). The label becomes the undo name.
    Begin(String),
    /// Replace the current preview with this command, applied to the snapshot.
    Preview(String, Value),
    /// Finish the interaction: keep the last preview as one undo step.
    Commit,
    /// Abort the interaction: restore the snapshot.
    Cancel,
    /// Execute a command immediately (its own undo step, if it edits).
    Exec(String, Value),
    /// Ask the UI to open a dialog (e.g. click with the Rectangle tool → size dialog).
    Dialog(String, Value),
    /// Switch to another tool (e.g. after placing a text frame).
    SwitchTool(String),
    /// Call the tool's `notify` after the preceding actions ran (e.g. to pick up a created object).
    Notify(String),
}

/// Paint defaults for new art (the fill/stroke proxy) and the rest of the new-art template.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintDefaults {
    pub fill: Paint,
    pub stroke: Paint,
    pub stroke_width: f64,
    /// The appearance new art takes (stroke options, more fills and strokes, effects), its top
    /// fill and stroke repainted with `fill`, `stroke` and `stroke_width`; `None`: one plain fill
    /// and stroke. Its placed gradients are relative to the unit box.
    pub appearance: Option<vectorcraft_doc::Appearance>,
    /// New art's opacity and blend mode.
    pub opacity: f32,
    pub blend: vectorcraft_color::BlendMode,
    /// The graphic style the template came from (by name): new art is linked to it.
    pub style: Option<String>,
    /// The template is the last selection's appearance (New Art Has Basic Appearance off): with
    /// the option on again, new art takes only its basic fill and stroke.
    pub inherited: bool,
}

impl Default for PaintDefaults {
    /// White fill, 1 pt black stroke, nothing else.
    fn default() -> Self {
        Self {
            fill: Paint::solid(vectorcraft_color::Color::WHITE),
            stroke: Paint::solid(vectorcraft_color::Color::BLACK),
            stroke_width: 1.0,
            appearance: None,
            opacity: 1.0,
            blend: Default::default(),
            style: None,
            inherited: false,
        }
    }
}

/// The document window on screen, in document coordinates: widgets that stay put on screen (the
/// Plane Switching Widget) are placed with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenFrame {
    /// The window's top-left corner.
    pub origin: Point,
    /// One screen pixel to the right and one down.
    pub right: Vec2,
    pub down: Vec2,
    /// The window's size in screen pixels.
    pub size: (f64, f64),
}

impl ScreenFrame {
    /// The document point `x`, `y` screen pixels from the window's top-left corner.
    pub fn at(&self, x: f64, y: f64) -> Point {
        self.origin + self.right * x + self.down * y
    }
    /// Where the document point `p` is, in screen pixels from the window's top-left corner (the
    /// inverse of [`Self::at`]); none for a degenerate frame.
    pub fn to_screen(&self, p: Point) -> Option<(f64, f64)> {
        let (r, d, v) = (self.right, self.down, p - self.origin);
        let det = r.x * d.y - d.x * r.y;
        (det.is_finite() && det.abs() > 1e-12).then(|| ((v.x * d.y - d.x * v.y) / det, (r.x * v.y - v.x * r.y) / det))
    }
}

/// Read-only context a tool sees.
pub struct ToolContext<'a> {
    pub doc: &'a Document,
    /// Which open document `doc` is (its process-unique id) and its revision, which moves with
    /// every change of the document or the selection: what a tool keeps between pointer events
    /// for the document as it is (the snap targets of hovering) is keyed on it.
    pub revision: (u64, u64),
    pub selection: &'a Selection,
    /// Screen pixels per document point.
    pub zoom: f64,
    pub isolation: Option<NodeId>,
    pub paint: &'a PaintDefaults,
    pub outline: bool,
    pub smart_guides: bool,
    /// View → Show Guides with Lock Guides off: the selection tools pick and drag ruler guides.
    pub guides: bool,
    pub snap_to_grid: bool,
    /// Bounding box shown (View → Show/Hide Bounding Box).
    pub show_bbox: bool,
    /// View → Snap to Pixel.
    pub snap_to_pixel: bool,
    /// View → Snap to Point: with Smart Guides off, dragged selections, drawn points and picked
    /// points (a transform's reference point) land on anchors and ruler guides within
    /// [`Self::snap_tolerance`].
    pub snap_to_point: bool,
    /// View → Show Corner Widget: paths show draggable Live Corners widgets in their corners.
    pub corner_widgets: bool,
    /// Which paint proxy is in front (true = Fill): the one the Gradient tool edits.
    pub fill_active: bool,
    /// The selected gradient stop (`gradient.selectStop`), marked on the gradient annotator.
    pub gradient_stop: Option<usize>,
    /// The Appearance panel's active fill/stroke (paint-order index into the first selected
    /// object's stack); paint tools show and edit that item.
    pub appearance_item: Option<usize>,
    /// General → Constrain Angle (degrees): Shift constrains drags to 45° steps from it.
    pub constrain_angle: f64,
    /// The selected freeform gradient point (`paint.freeform.selectPoint`), marked on the
    /// freeform annotator.
    pub freeform_point: Option<usize>,
    /// Eyedropper Options' raster sample size: the pixels square averaged when the Eyedropper
    /// samples an image (1, 3 or 5).
    pub raster_sample: u32,
    /// General → Use Preview Bounds: the bounding box measures visual bounds (strokes included).
    pub preview_bounds: bool,
    /// Units ▸ General: the unit measurement labels show lengths in.
    pub unit: Unit,
    /// Units ▸ Stroke: the unit measurement labels show stroke widths in.
    pub stroke_unit: Unit,
    /// Clipboard Handling → When pasting text: Keep Plain Text. Text pasted while typing takes
    /// the style at the caret, even the text the Type tool copied with its formatting.
    pub paste_plain_text: bool,
    /// View → Hide Slices: the Slice Selection tool can't pick hidden slices.
    pub slices_hidden: bool,
    /// View → Lock Slices: the Slice Selection tool leaves locked slices alone.
    pub slices_locked: bool,
    /// General → Disable Auto Add/Delete is off: the Pen adds an anchor on a selected path's
    /// segment and deletes one of its anchors.
    pub auto_add_delete: bool,
    /// Selection & Anchor Display → Tolerance: how near (screen pixels) a click must be to a path
    /// or an anchor to pick it.
    pub selection_tolerance: f64,
    /// Selection & Anchor Display → Size (1–7, default 3): how big anchors and handles are drawn.
    pub anchor_size: u32,
    /// Selection & Anchor Display → Object Selection by Path Only: a click inside a filled path
    /// doesn't pick it, only one on its path does.
    pub path_only: bool,
    /// Type → Type Object Selection by Path Only: type is picked on its type path only (point
    /// type's baseline, area type's frame, type on a path's path), not anywhere in its bounds.
    pub type_path_only: bool,
    /// General → Double Click To Isolate: a double-click on a group with the Selection tool
    /// isolates it.
    pub double_click_isolate: bool,
    /// Selection & Anchor Display → Command Click to Select Objects Behind: Cmd/Ctrl-click with the
    /// Selection tool selects the object under the selected one, the next click the one under
    /// that.
    pub select_behind: bool,
    /// Selection & Anchor Display → Highlight anchors on mouse over: Direct Selection marks the
    /// anchor under the pointer.
    pub highlight_anchors: bool,
    /// Selection & Anchor Display → Snap to Point (screen pixels): how near an anchor or a ruler
    /// guide pulls the pointer while View → Snap to Point is on.
    pub snap_tolerance: f64,
    /// Selection & Anchor Display → Show handles when multiple anchors are selected: off, Direct
    /// Selection shows and drags handles only while a single anchor is selected.
    pub handles_multiple: bool,
    /// Selection & Anchor Display → Hide Corner Widget for angles greater than (degrees): corners
    /// wider than this show no Live Corners widget.
    pub corner_widget_max_angle: f64,
    /// Selection & Anchor Display → Move Locked and Hidden Artwork with Artboard.
    pub move_locked_with_artboard: bool,
    /// Selection & Anchor Display → Enable Rubber Band for Pen Tool: the Pen previews the next
    /// segment to the pointer.
    pub pen_rubber_band: bool,
    /// Selection & Anchor Display → Enable Rubber Band for Curvature Tool.
    pub curvature_rubber_band: bool,
    /// Type → Fill New Type Objects With Placeholder Text: type the Type tools place starts with
    /// placeholder text, selected.
    pub placeholder_text: bool,
    /// Smart Guides → Color: the smart guides' lines and labels (RGB).
    pub smart_guide_color: [u8; 3],
    /// Smart Guides → Alignment Guides: the lines along the edges and centres the art lines up
    /// with show. Off, the art still snaps into line.
    pub alignment_guides: bool,
    /// Smart Guides → Anchor/Path Labels: the "anchor", "center", "path"… labels show.
    pub anchor_path_labels: bool,
    /// Smart Guides → Measurement Labels: the size and offset readouts while drawing and moving.
    pub measurement_labels: bool,
    /// Smart Guides → Transform Tools: the readouts while scaling, rotating and shearing.
    pub transform_tools_guides: bool,
    /// Smart Guides → Spacing Guides: a moved selection snaps to spacing equal to the gap between
    /// two other objects in its row or column, or evenly between its two neighbours, and the
    /// equal gaps show.
    pub spacing_guides: bool,
    /// Smart Guides → Snapping Tolerance (screen pixels): how near a smart guide target pulls the
    /// pointer, a dragged edge or a drawn point.
    pub snapping_tolerance: f64,
    /// Smart Guides → Construction Guides: the angles (degrees, counter-clockwise from the x axis
    /// as seen on the page) of the lines through the point a drawn point leaves from, onto which a
    /// point near one lands ([`guides::construction_angles`]); none with the option off.
    pub construction_angles: &'static [f64],
    /// The document window (none headless): screen-fixed widgets sit in it.
    pub screen: Option<ScreenFrame>,
    /// Where the Plane Switching Widget sits (Perspective Grid Options); None while it's hidden.
    pub plane_widget: Option<distort::perspective::widget::WidgetCorner>,
}

impl ToolContext<'_> {
    /// Tolerance in document units for `px` screen pixels.
    pub fn tol(&self, px: f64) -> f64 {
        px / self.zoom.max(1e-9)
    }
    /// How near (document units) a smart guide target pulls: Smart Guides → Snapping Tolerance.
    pub fn snap_tol(&self) -> f64 {
        self.tol(self.snapping_tolerance)
    }
    /// A length as measurement labels show it, in the General unit (`12.50 mm`).
    pub fn len(&self, v: f64) -> String {
        self.unit.readout(v)
    }
    /// A size label: `W: …` over `H: …`.
    pub fn size_label(&self, w: f64, h: f64) -> String {
        format!("W: {}\nH: {}", self.len(w), self.len(h))
    }
    /// A move label: `dX: …` over `dY: …`.
    pub fn offset_label(&self, dx: f64, dy: f64) -> String {
        format!("dX: {}\ndY: {}", self.len(dx), self.len(dy))
    }
    /// What Snap to Grid snaps to: the grid's subdivisions (or its lines without any).
    pub fn grid_step(&self) -> f64 {
        self.doc.grid.spacing / self.doc.grid.subdivisions.max(1) as f64
    }
    /// The selection tolerance in document units ([`Self::selection_tolerance`]).
    pub fn pick_tol(&self) -> f64 {
        self.tol(self.selection_tolerance)
    }
    /// How near (document units) a press picks an anchor or a direction handle's end: the
    /// selection tolerance, but at least 2 px past the drawn point, so a point is easy to catch
    /// and wins over the segments through it, which are tested after it (#593).
    pub fn point_tol(&self) -> f64 {
        // Anchors are drawn 5 px wide at the default Size 3, a pixel more or less per step.
        let half = (5.0 + f64::from(self.anchor_size.clamp(1, 7)) - 3.0) / 2.0;
        self.tol(self.selection_tolerance.max(half + 2.0))
    }
    /// What a press at `p` picks: a selected compound-shape member anywhere in its own shape (even
    /// in a hole it cuts, [`vectorcraft_doc::hit::selected_member_at`]), else the topmost object
    /// ([`vectorcraft_doc::hit::hit_test`]).
    pub fn hit(&self, p: Point) -> Option<vectorcraft_doc::hit::Hit> {
        vectorcraft_doc::hit::selected_member_at(self.doc, p, self.hit_options(), &self.selection.objects)
            .or_else(|| vectorcraft_doc::hit::hit_test(self.doc, p, self.hit_options()))
    }

    pub fn hit_options(&self) -> vectorcraft_doc::hit::HitOptions {
        vectorcraft_doc::hit::HitOptions {
            tol: self.pick_tol(),
            outline: self.outline,
            path_only: self.path_only,
            type_path_only: self.type_path_only,
            scope: self.isolation,
        }
    }
}

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
    /// A handle end circle.
    Handle { p: Point, color: [u8; 3] },
    /// Smart-guide style label (e.g. "anchor", "W: 100 pt").
    Label { p: Point, text: String, color: [u8; 3] },
    /// Measurement pill near the cursor (grey box with white text).
    Measure { p: Point, text: String },
    /// Translucent filled quad (text selection highlight), RGBA.
    Highlight { quad: [Point; 4], color: [u8; 4] },
    /// A colour chip of fixed screen size (a gradient stop), RGBA; ringed when selected.
    Swatch { p: Point, color: [u8; 4], selected: bool },
    /// A hairline in a translucent colour (perspective gridlines), RGBA.
    GridLine { a: Point, b: Point, color: [u8; 4] },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cursor {
    #[default]
    Arrow,
    ArrowHollow,
    Move,
    Crosshair,
    ResizeH,
    ResizeV,
    ResizeNwSe,
    ResizeNeSw,
    Rotate,
    /// Over a Live Corners widget (drag to round the corners).
    CornerRadius,
    Pen,
    PenAdd,
    PenDelete,
    PenClose,
    PenContinue,
    /// While drawing, over an end of another open path (a click joins the two).
    PenJoin,
    /// Over the last anchor of the path being drawn (a click retracts its outgoing handle), or with
    /// Alt held over a selected path's handle or anchor (the Anchor Point tool's gesture).
    PenConvert,
    Text,
    Hand,
    HandGrab,
    ZoomIn,
    ZoomOut,
    Eyedropper,
    NotAllowed,
    /// Over the gradient annotator's bar: a click adds a stop.
    AddStop,
    /// A gradient stop dragged off the bar: releasing deletes it.
    RemoveStop,
    /// The Slice tool: a crosshair with a blade.
    Slice,
    /// The Slice Selection tool: the arrow with a slice badge.
    SliceSelect,
    /// The Width tool away from strokes.
    Width,
    /// The Width tool over a stroke: a drag adds a width point.
    WidthAdd,
    /// The Width tool over a width point or a handle end: a drag moves or widens it.
    WidthPoint,
    /// The Blend tool away from art: a crosshair with a hollow square.
    Blend,
    /// The Blend tool over an object it can blend: a crosshair with a filled square.
    BlendObject,
    /// The Blend tool over an anchor point (the blend starts there): a crosshair with a target.
    BlendAnchor,
    /// Over a bracket of selected type on a path (a drag moves it): the arrow with a bracket.
    PathBracket,
    /// Over the type widget of selected type (a double-click converts point type to area type and
    /// back): the arrow with a type badge.
    TypeWidget,
    /// The Shape Builder: a crosshair with a plus (merge mode)...
    ShapeBuilder,
    /// ...or, with Alt held, a minus (erase mode).
    ShapeBuilderErase,
}

impl Cursor {
    /// General › Use Precise Cursors: the drawing tools' pointers (the Pen's in every state, the
    /// Eyedropper's, the Slice and Blend tools') become a plain crosshair at the hotspot.
    pub fn precise(self) -> Self {
        match self {
            Cursor::Pen
            | Cursor::PenAdd
            | Cursor::PenDelete
            | Cursor::PenClose
            | Cursor::PenContinue
            | Cursor::PenConvert
            | Cursor::Eyedropper
            | Cursor::Slice
            | Cursor::Blend
            | Cursor::BlendObject
            | Cursor::BlendAnchor
            | Cursor::ShapeBuilder
            | Cursor::ShapeBuilderErase => Cursor::Crosshair,
            c => c,
        }
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
    /// IME composition (marked text) for the tool that wants text: `text` replaces the previous
    /// marked text (or the selection); an empty `text` ends the composition. `active_chars` is the
    /// clause being converted, in characters of `text`. The committed result arrives through
    /// [`Tool::text_input`].
    fn ime_preedit(&mut self, _cx: &ToolContext, _text: &str, _active_chars: Option<std::ops::Range<usize>>) -> Vec<Action> {
        vec![]
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
    /// Called when the user switches away (finish pending work).
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
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
}

/// Create a tool by id. Unknown or not-yet-implemented tools fall back to a no-op tool that keeps the id.
pub fn create(id: &str) -> Box<dyn Tool> {
    match id {
        "selection" => Box::new(select::SelectionTool::default()),
        "directSelection" => Box::new(direct::DirectSelectionTool::new(false)),
        "groupSelection" => Box::new(direct::DirectSelectionTool::new(true)),
        "pen" => Box::new(pen::PenTool::default()),
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

    pub fn paint() -> PaintDefaults {
        PaintDefaults::default()
    }

    pub fn cx<'a>(d: &'a Document, s: &'a Selection, p: &'a PaintDefaults) -> ToolContext<'a> {
        ToolContext {
            doc: d,
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
            corner_widgets: true,
            fill_active: true,
            gradient_stop: None,
            appearance_item: None,
            constrain_angle: 0.0,
            freeform_point: None,
            raster_sample: 1,
            preview_bounds: false,
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
            snapping_tolerance: 4.0,
            construction_angles: guides::construction_angles(guides::DEFAULT_CONSTRUCTION_ANGLES),
            screen: None,
            plane_widget: Some(Default::default()),
        }
    }
}
