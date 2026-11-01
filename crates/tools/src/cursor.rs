use super::{Deserialize, Point, Serialize, Vec2};

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
    /// The Pencil tool before a press that draws a new freehand path: the pointer is not over a
    /// selected path that the press would continue or redraw. A crosshair with a small star.
    PencilNew,
    /// The Pencil tool with Alt pressed after the drag starts and Alt key toggles to Smooth Tool off:
    /// releasing the mouse button with Alt still held closes the path. A drag started with Shift or
    /// Alt held draws an open straight line instead. Also while a freehand stroke is drawn back near
    /// its start, where releasing the mouse button closes the path. A crosshair with a ring.
    PencilClose,
    /// The Pencil tool before a press that draws a straight line (Shift or Alt held). A crosshair with a
    /// short straight line.
    PencilStraight,
    /// The Pencil tool before a press that draws a straight line (Shift or Alt held), over an end of
    /// a selected open path: the line continues the path from that end. A crosshair with a short
    /// straight line leaving a square end point.
    PencilContinue,
    /// The Paintbrush tool.
    Paintbrush,
    /// The Paintbrush tool with Alt held: releasing the mouse button with Alt still held closes
    /// the path. Also while a stroke is drawn back near its start, where releasing the mouse button
    /// closes the path. A crosshair with a ring.
    PaintbrushClose,
    /// The Blob Brush tool: a dot at the brush's centre, the tool drawing the tip's outline round
    /// it on the canvas.
    BlobBrush,
    /// The Eraser tool: a dot at the tip's centre, the tool drawing the tip's outline round it on
    /// the canvas.
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
    /// The Anchor Eraser Brush (Path Precision Tools add-on); the brush's circle is drawn round it.
    AnchorEraser,
    /// The Extend Path tool (Path Precision Tools add-on).
    ExtendPath,
    /// The Measure & Inspect tool (Path Precision Tools add-on).
    MeasureInspect,
    /// The Parametric Shape Tool (Path Precision Tools add-on): a drag draws a shape out from its
    /// centre; a selected shape's handles edit it.
    ParametricShape,
    /// The Corner Tool (Path Precision Tools add-on): a click or drag on an anchor cuts a corner;
    /// a corner's marker resizes it.
    CornerTool,
    /// The Slide Anchor Along Path tool (Path Precision Tools add-on): a drag runs an anchor along
    /// the path.
    SlideAnchor,
    /// The Shared Edge Merge Tool (Path Precision Tools add-on): a click merges the two paths'
    /// touching edges under the brush, or makes an end there.
    SharedEdge,
    /// The Smart Pen Tool (Smart Pen add-on) away from paths, where a click starts a new path (over
    /// a path or while drawing it shows the Pen's and Direct Selection's cursors).
    SmartPen,
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
            | Cursor::PencilNew
            | Cursor::PencilClose
            | Cursor::PencilStraight
            | Cursor::PencilContinue
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
            | Cursor::PuppetPin
            | Cursor::AnchorEraser
            | Cursor::ExtendPath
            | Cursor::MeasureInspect
            | Cursor::ParametricShape
            | Cursor::CornerTool
            | Cursor::SlideAnchor
            | Cursor::SharedEdge
            | Cursor::SmartPen => Cursor::Crosshair,
            c => c,
        }
    }

    /// The cursor this one borrows until it's drawn: the generic crosshair for the
    /// tools, the I-beam for vertical type, the Pen's add cursor for the Mesh tool and Puppet
    /// Warp, not-allowed for the Scissors away from a path. `None` for a cursor with a look of
    /// its own.
    pub fn stand_in(self) -> Option<Self> {
        match self {
            Cursor::Lasso
            | Cursor::MagicWand
            | Cursor::Pencil
            | Cursor::Paintbrush
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
            | Cursor::Scissors
            | Cursor::AnchorEraser
            | Cursor::ExtendPath
            | Cursor::MeasureInspect
            | Cursor::ParametricShape
            | Cursor::CornerTool
            | Cursor::SlideAnchor
            | Cursor::SharedEdge => Some(Cursor::Crosshair),
            Cursor::TypeVertical => Some(Cursor::Text),
            Cursor::MeshAdd | Cursor::PuppetPin => Some(Cursor::PenAdd),
            Cursor::SmartPen => Some(Cursor::Pen),
            Cursor::ScissorsNotAllowed => Some(Cursor::NotAllowed),
            _ => None,
        }
    }
}
