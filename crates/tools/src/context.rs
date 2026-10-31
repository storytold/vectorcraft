use super::{Document, NodeId, Paint, Point, Selection, Unit, Vec2, distort, select, xform};

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

/// The Character panel's Snap to Glyph options (`view.snapToGlyph.options`): which guides View ›
/// Snap to Glyph gives. All on by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GlyphSnap {
    /// The baseline of a single line of type.
    pub baseline: bool,
    /// The x-height: the top of its lowercase letters.
    pub x_height: bool,
    /// Glyph Bounds: each glyph's top, bottom, left and right edges.
    pub bounds: bool,
    /// Proximity Guides: across the baseline at both edges of each character's width.
    pub proximity: bool,
    /// Angular Guides: along a glyph's slanted straight segments, and every guide of rotated type
    /// (which runs at its angle).
    pub angular: bool,
    /// Anchor Point: the anchor points of each glyph's outline.
    pub anchors: bool,
}

impl Default for GlyphSnap {
    fn default() -> Self {
        Self { baseline: true, x_height: true, bounds: true, proximity: true, angular: true, anchors: true }
    }
}

impl GlyphSnap {
    /// These options with the ones `p` names (camelCase booleans), or why `p` can't set them.
    pub fn merged(self, p: &serde_json::Value) -> Result<Self, String> {
        let mut o = serde_json::to_value(self).map_err(|e| e.to_string())?;
        match (p, o.as_object_mut()) {
            (serde_json::Value::Null, _) => {}
            (serde_json::Value::Object(given), Some(cur)) => {
                for (k, v) in given {
                    let slot = cur.get_mut(k).ok_or_else(|| format!("unknown Snap to Glyph option `{k}`"))?;
                    *slot = serde_json::Value::Bool(v.as_bool().ok_or_else(|| format!("`{k}` must be true or false"))?);
                }
            }
            _ => return Err("the Snap to Glyph options must be an object".into()),
        }
        serde_json::from_value(o).map_err(|e| e.to_string())
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
    /// View → Snap to Glyph: drawn points snap to live-text glyph bounds, and on a single line of type
    /// to its baseline, x-height, character widths and glyph anchor points (with Smart Guides).
    pub snap_to_glyph: bool,
    /// Which of Snap to Glyph's guides there are (the Character panel's Snap to Glyph options).
    pub glyph_snap: GlyphSnap,
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
    /// Star tool drag: outer (R1) and inner (R2) radii from the centre to the pointer.
    pub fn star_radii_label(&self, outer: f64, inner: f64) -> String {
        format!("R1: {}\nR2: {}", self.len(outer), self.len(inner))
    }
    /// The move an arrow key nudges by: General › Keyboard Increment (ten times with Shift),
    /// along General › Constrain Angle's axes; `None` for other keys.
    pub fn arrow_nudge(&self, key: crate::ToolKey, mods: crate::Mods) -> Option<Vec2> {
        use crate::ToolKey;
        let step = self.keyboard_increment * if mods.shift { 10.0 } else { 1.0 };
        let d = match key {
            ToolKey::Left => Point::new(-step, 0.0),
            ToolKey::Right => Point::new(step, 0.0),
            ToolKey::Up => Point::new(0.0, -step),
            ToolKey::Down => Point::new(0.0, step),
            _ => return None,
        };
        Some(vectorcraft_geom::constrain_axes(self.constrain_angle).map_or(d, |axes| axes * d).to_vec2())
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
