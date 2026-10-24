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
    /// A letter key a–z, lowercase (E while dragging a Selection or Group Selection marquee makes it
    /// select only the objects entirely inside it, and E again switches it back).
    Letter(char),
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
    /// The rulers' zero point with the view's active artboard (see
    /// [`Document::ruler_origin_for`]): where ruler ticks and measured positions count from.
    pub ruler_origin: Point,
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
    /// View → Snapping → Snap to Tangent: with Smart Guides on, a line drawn
    /// from an anchor lands where it touches a nearby curve.
    pub snap_tangent: bool,
    /// View → Snapping → Snap to Perpendicular: likewise where it meets a nearby segment square.
    pub snap_perpendicular: bool,
    /// View → Snap to Glyph: drawn points snap to live-text baselines and glyph bounds (with Smart Guides).
    pub snap_to_glyph: bool,
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
    /// General → Keyboard Increment: arrow-key nudges in document units (Shift multiplies by 10).
    pub keyboard_increment: f64,
    /// The selected freeform gradient point (`paint.freeform.selectPoint`), marked on the
    /// freeform annotator.
    pub freeform_point: Option<usize>,
    /// Eyedropper Options' raster sample size: the pixels square averaged when the Eyedropper
    /// samples an image (1, 3 or 5).
    pub raster_sample: u32,
    /// General → Use Preview Bounds: the bounding box measures visual bounds (strokes included).
    pub preview_bounds: bool,
    /// The session's memo of the selection tools' measurements for the document as it is
    /// ([`select::PreviewMemo`]); without one they measure on every call.
    pub preview_memo: Option<&'a select::PreviewMemo>,
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
    /// Smart Guides → Invoke Distance Guides: with the Selection tool, Alt/Option held over another
    /// object shows how far the selection is from it, and over an empty part of an artboard how
    /// far it is from the artboard's edges ([`guides::distance_guides`]).
    pub distance_guides: bool,
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
    /// How far the view is turned (Rotate View), in degrees clockwise on screen; 0 without a
    /// window.
    pub fn view_angle(&self) -> f64 {
        // `right` is one screen pixel to the right, in document units.
        self.screen.map(|s| (-s.right.y).atan2(s.right.x).to_degrees()).filter(|a| a.is_finite()).unwrap_or(0.0)
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
    /// A segment label while drawing: distance, angle and offsets between two points.
    pub fn segment_measure_label(&self, a: Point, b: Point) -> String {
        let (dist, ang, dx, dy) = xform::measure(a, b);
        format!("D: {}\nAngle: {ang:.1}°\n{}", self.len(dist), self.offset_label(dx, dy))
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
    pub fn hit_options(&self) -> vectorcraft_doc::hit::HitOptions {
        vectorcraft_doc::hit::HitOptions {
            tol: self.pick_tol(),
            outline: self.outline,
            path_only: self.path_only,
            type_path_only: self.type_path_only,
            scope: self.isolation,
            skip_guides: !self.guides,
            clip_outlines: false,
        }
    }

    /// [`Self::hit_options`] for the selection tools (Selection, Direct Selection, Group
    /// Selection): a clipping path is also hit on its outline ([`HitOptions::clip_outlines`]).
    /// Only for what a press or a hover selects: a double-click, loaded text or a guide's release
    /// acts on what is drawn and takes [`Self::hit_options`].
    ///
    /// [`HitOptions::clip_outlines`]: vectorcraft_doc::hit::HitOptions::clip_outlines
    pub fn selection_hit_options(&self) -> vectorcraft_doc::hit::HitOptions {
        vectorcraft_doc::hit::HitOptions { clip_outlines: true, ..self.hit_options() }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cursor {
    #[default]
    Arrow,
    ArrowHollow,
    /// Direct Selection over an anchor of a path that isn't selected: the white arrow with a
    /// hollow square.
    DirectAnchor,
    /// Direct Selection over an anchor of a selected path: the white arrow with a filled square.
    DirectAnchorSelected,
    /// Over a segment a drag reshapes (Direct Selection, the Anchor Point tool, the Pen with Alt):
    /// an arrowhead with a small curve between two anchor marks.
    ReshapeSegment,
    /// The Group Selection tool, and Direct Selection with Alt held: the white arrow with a plus.
    GroupSelection,
    /// Alt-dragging a copy with the Selection or Direct Selection tool: a double arrowhead.
    SelectionCopy,
    Move,
    Crosshair,
    ResizeH,
    ResizeV,
    ResizeNwSe,
    ResizeNeSw,
    /// Free Transform with Cmd/Ctrl over the top or bottom side, where a drag shears the object
    /// sideways: two arrows side by side, pointing opposite ways along that side.
    ShearH,
    /// Free Transform with Cmd/Ctrl over the left or right side, where a drag shears the object up
    /// or down: the ShearH arrows turned upright.
    ShearV,
    Rotate,
    /// The rotate cursor beside a bounding box, bowed toward the pointer's side of the box: off
    /// its centre in direction `0..8` (0 straight up, then clockwise in 45° steps). Its arc wraps
    /// the corner it is at, so at the top right it opens toward the bottom left. Make it with
    /// [`Cursor::rotate_about`].
    RotateToward(u8),
    /// Over a Live Corners widget (drag to round the corners).
    CornerRadius,
    /// The Pen with the new-path asterisk: a click starts a new path.
    Pen,
    /// The Pen without a badge, while a path is being drawn: a click adds its next anchor.
    PenDrawing,
    PenAdd,
    PenDelete,
    PenClose,
    PenContinue,
    /// The Anchor Point tool, and the Pen with Alt held (which lends its gestures): a caret, its tip
    /// on the anchor it converts.
    AnchorPoint,
    /// The Curvature tool: the pen nib with a curve badge.
    Curvature,
    /// While drawing, over an end of another open path (a click joins the two).
    PenJoin,
    /// Over the last anchor of the path being drawn: a click retracts its outgoing handle.
    PenConvert,
    Text,
    Hand,
    HandGrab,
    ZoomIn,
    ZoomOut,
    Eyedropper,
    NotAllowed,
    /// A long operation (opening a file, a background save or export): the OS wait cursor.
    Busy,
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
    /// The Objects on Path tool over a path the selected objects can attach to (a click attaches
    /// them).
    ObjectsOnPath,
    /// Over a bracket of selected type on a path (a drag moves it): the arrow with a bracket.
    PathBracket,
    /// Over the type widget of selected type (a double-click converts point type to area type and
    /// back): the arrow with a type badge.
    TypeWidget,
    /// The Selection tool loaded with the text of area type or type on a path by a click on its out
    /// or in port (a click threads it into a path or a new frame, a drag draws the new frame).
    LoadedText,
    /// The Shape Builder: a crosshair with a plus (merge mode)...
    ShapeBuilder,
    /// ...or, with Alt held, a minus (erase mode).
    ShapeBuilderErase,
    /// The Lasso tool.
    Lasso,
    /// The Magic Wand tool.
    MagicWand,
    /// The Pencil tool.
    Pencil,
    /// The Pencil tool with Alt pressed after the drag starts and Alt key toggles to Smooth Tool off:
    /// releasing the mouse button with Alt still held closes the path. A drag started with Shift or
    /// Alt held draws an open straight line instead. Also while a freehand stroke is drawn back near
    /// its start, where releasing the mouse button closes the path. A crosshair with a ring.
    PencilClose,
    /// The Pencil tool before a press that draws a straight line (Shift or Alt held). A crosshair with a
    /// short straight line.
    PencilStraight,
    /// The Paintbrush tool.
    Paintbrush,
    /// The Paintbrush tool with Alt held: releasing the mouse button with Alt still held closes
    /// the path. Also while a stroke is drawn back near its start, where releasing the mouse button
    /// closes the path. A crosshair with a ring.
    PaintbrushClose,
    /// The Blob Brush tool.
    BlobBrush,
    /// The Eraser tool.
    Eraser,
    /// The Smooth tool.
    Smooth,
    /// The Path Eraser tool.
    PathEraser,
    /// The Join tool.
    Join,
    /// The Knife tool.
    Knife,
    /// The Shaper tool.
    Shaper,
    /// The Live Paint Bucket.
    LivePaintBucket,
    /// The Measure tool.
    Measure,
    /// The Print Tiling tool.
    PrintTiling,
    /// The Symbol Sprayer tool.
    SymbolSprayer,
    /// The Symbol Shifter tool.
    SymbolShifter,
    /// The Symbol Scruncher tool.
    SymbolScruncher,
    /// The Symbol Sizer tool.
    SymbolSizer,
    /// The Symbol Spinner tool.
    SymbolSpinner,
    /// The Symbol Stainer tool.
    SymbolStainer,
    /// The Symbol Screener tool.
    SymbolScreener,
    /// The Symbol Styler tool.
    SymbolStyler,
    /// The vertical type tools: a turned I-beam.
    TypeVertical,
    /// The Scissors over a path segment (a click splits the path there)...
    Scissors,
    /// ...or away from every segment (a click does nothing).
    ScissorsNotAllowed,
    /// The Mesh tool off a mesh point: a click adds one.
    MeshAdd,
    /// Puppet Warp over the selection's mesh where a pin can go: a click adds one.
    PuppetPin,
}

impl Cursor {
    /// The rotate cursor for the pointer at `p` turning art about `center`: [`Cursor::RotateToward`]
    /// the pointer's side of it, or the plain [`Cursor::Rotate`] on the centre itself.
    pub fn rotate_about(center: Point, p: Point) -> Self {
        let d = p - center;
        if !(d.x.is_finite() && d.y.is_finite()) || d.hypot() < 1e-9 {
            return Cursor::Rotate;
        }
        // y points down the page: atan2(x, -y) is 0 straight up and grows clockwise.
        let octant = (d.x.atan2(-d.y) / std::f64::consts::FRAC_PI_4).round() as i64;
        Cursor::RotateToward(octant.rem_euclid(8) as u8)
    }

    /// [`Cursor::rotate_about`] in a view turned `view` degrees clockwise
    /// ([`ToolContext::view_angle`]): the pointer's side of `center` as it shows on screen. The
    /// pointer's direction turns with the view before it rounds to the nearest 45°.
    pub fn rotate_about_in_view(center: Point, p: Point, view: f64) -> Self {
        let (sn, cs) = if view.is_finite() { view.to_radians().sin_cos() } else { (0.0, 1.0) };
        let d = p - center;
        // y points down the page, so this turns `d` clockwise on the page.
        Cursor::rotate_about(center, center + Vec2::new(d.x * cs - d.y * sn, d.x * sn + d.y * cs))
    }

    /// The cursor as it shows in a view turned `deg` degrees clockwise (Rotate View): the resize
    /// cursors and [`Cursor::RotateToward`] turn with the view to the nearest 45°, and every other
    /// cursor stays as it is.
    pub fn turned(self, deg: f64) -> Self {
        const RESIZE: [Cursor; 4] = [Cursor::ResizeH, Cursor::ResizeNeSw, Cursor::ResizeV, Cursor::ResizeNwSe];
        let k = if deg.is_finite() { (deg / 45.0).round() as i64 } else { 0 };
        match self {
            Cursor::RotateToward(o) => Cursor::RotateToward((i64::from(o) + k).rem_euclid(8) as u8),
            c => match RESIZE.iter().position(|r| *r == c) {
                Some(i) => RESIZE.get((i as i64 - k).rem_euclid(4) as usize).copied().unwrap_or(c),
                None => c,
            },
        }
    }

    /// General › Use Precise Cursors: the drawing tools' pointers (the Pen's in every state, the
    /// Anchor Point and Curvature tools', the Eyedropper's, the Slice and Blend tools') become a plain crosshair at the hotspot.
    pub fn precise(self) -> Self {
        match self {
            Cursor::Pen
            | Cursor::PenDrawing
            | Cursor::PenAdd
            | Cursor::PenDelete
            | Cursor::PenClose
            | Cursor::PenContinue
            | Cursor::PenJoin
            | Cursor::PenConvert
            | Cursor::AnchorPoint
            | Cursor::Curvature
            | Cursor::Eyedropper
            | Cursor::Slice
            | Cursor::Blend
            | Cursor::BlendObject
            | Cursor::BlendAnchor
            | Cursor::ShapeBuilder
            | Cursor::ShapeBuilderErase
            | Cursor::Lasso
            | Cursor::MagicWand
            | Cursor::Pencil
            | Cursor::PencilClose
            | Cursor::PencilStraight
            | Cursor::Paintbrush
            | Cursor::PaintbrushClose
            | Cursor::BlobBrush
            | Cursor::Eraser
            | Cursor::Smooth
            | Cursor::PathEraser
            | Cursor::Join
            | Cursor::Knife
            | Cursor::Shaper
            | Cursor::LivePaintBucket
            | Cursor::Measure
            | Cursor::PrintTiling
            | Cursor::SymbolSprayer
            | Cursor::SymbolShifter
            | Cursor::SymbolScruncher
            | Cursor::SymbolSizer
            | Cursor::SymbolSpinner
            | Cursor::SymbolStainer
            | Cursor::SymbolScreener
            | Cursor::SymbolStyler
            | Cursor::Scissors
            | Cursor::MeshAdd
            | Cursor::PuppetPin => Cursor::Crosshair,
            c => c,
        }
    }

    /// The cursor this one borrows until it's drawn: the generic crosshair for the
    /// tools, the I-beam for vertical type and loaded text, the Pen's add cursor for the Mesh tool and Puppet
    /// Warp, not-allowed for the Scissors away from a path. `None` for a cursor with a look of
    /// its own.
    pub fn stand_in(self) -> Option<Self> {
        match self {
            Cursor::Lasso
            | Cursor::MagicWand
            | Cursor::Pencil
            | Cursor::Paintbrush
            | Cursor::BlobBrush
            | Cursor::Eraser
            | Cursor::Smooth
            | Cursor::PathEraser
            | Cursor::Join
            | Cursor::Knife
            | Cursor::Shaper
            | Cursor::LivePaintBucket
            | Cursor::Measure
            | Cursor::ObjectsOnPath
            | Cursor::PrintTiling
            | Cursor::SymbolSprayer
            | Cursor::SymbolShifter
            | Cursor::SymbolScruncher
            | Cursor::SymbolSizer
            | Cursor::SymbolSpinner
            | Cursor::SymbolStainer
            | Cursor::SymbolScreener
            | Cursor::SymbolStyler
            | Cursor::Scissors => Some(Cursor::Crosshair),
            Cursor::TypeVertical | Cursor::LoadedText => Some(Cursor::Text),
            Cursor::MeshAdd | Cursor::PuppetPin => Some(Cursor::PenAdd),
            Cursor::ScissorsNotAllowed => Some(Cursor::NotAllowed),
            _ => None,
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
            ("pencil", Cursor::Pencil),
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
