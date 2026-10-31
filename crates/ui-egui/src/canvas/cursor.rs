use super::{CacheKey, Cursor, Mods, Point, VectorcraftApp};

/// The active tool's cursor at the pointer, as `mods` make it. While the view moves (`still`
/// false), the cursor the tool last gave for the same tool and keys stays: asking the tool again
/// would hit-test the art under the pointer at every frame of the move (#2452).
pub(super) fn hover_cursor(app: &mut VectorcraftApp, mods: Mods, view: vectorcraft_engine::ViewInfo, still: bool) -> Option<Cursor> {
    let id = app.session.tool_id();
    if !still
        && let Some((was, with, c)) = app.canvas.hover_cursor
        && (was, with) == (id, mods)
    {
        return Some(c);
    }
    let c = app.session.cursor(app.hover_doc?, mods, view);
    app.canvas.hover_cursor = Some((id, mods, c));
    Some(c)
}

/// How long (seconds) the view must stay still before a canvas with
/// [`crate::CanvasCache::waits_for_still`] set renders a heavy document again.
pub const STILL_SECS: f64 = 0.2;

/// How long (seconds) the view must stay still before the canvas hit-tests the art under the
/// pointer again, for Object Highlighting and the tool's cursor. While the view pans, zooms or
/// turns under a resting pointer, the document point under it changes every frame, and hit-testing
/// detailed art each time slowed every frame of the move (#2452).
pub const HOVER_STILL_SECS: f64 = 0.15;

/// The view [`hover_wait`] watches: the canvas's rectangle and the view's zoom, centre and rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewKey {
    pub(super) rect: egui::Rect,
    pub(super) zoom: f64,
    pub(super) center: Point,
    pub(super) rotation: f64,
}

/// Note the view `key` at time `now` (seconds, egui's input time) → how many more seconds the
/// canvas leaves the art under the pointer alone: the view changed less than [`HOVER_STILL_SECS`]
/// ago. The first view noted counts as still.
pub(super) fn hover_wait(moved: &mut Option<(ViewKey, f64)>, key: ViewKey, now: f64) -> Option<f64> {
    match moved {
        Some((k, at)) if *k != key => (*k, *at) = (key, now),
        Some(_) => {}
        None => *moved = Some((key, f64::NEG_INFINITY)),
    }
    let waited = moved.map_or(f64::INFINITY, |(_, at)| now - at);
    // The clock went back (a new egui context): don't wait for it to catch up.
    (0.0..HOVER_STILL_SECS).contains(&waited).then_some(HOVER_STILL_SECS - waited)
}

/// Note that the canvas asks for `key` at time `now` (seconds, egui's input time) → how many more
/// seconds it draws the frame `shown` reprojected instead of rendering `key`: while the last
/// render was `slow`, only the view differs between the two, and the canvas has asked for `key`
/// for less than [`STILL_SECS`]. `still` holds the key last asked for and when the canvas first
/// asked for it. None: the canvas renders `key` now, unless it is the frame shown.
pub(super) fn still_wait(still: &mut Option<(CacheKey, f64)>, shown: Option<&CacheKey>, key: &CacheKey, slow: bool, now: f64) -> Option<f64> {
    if still.as_ref().is_none_or(|(k, _)| k != key) {
        *still = Some((key.clone(), now));
    }
    let waited = still.as_ref().map_or(0.0, |(_, at)| now - at);
    // The clock went back (a new egui context): don't wait for it to catch up.
    let waiting = (0.0..STILL_SECS).contains(&waited);
    (slow && waiting && shown.is_some_and(|s| s != key && only_view_differs(s, key))).then_some(STILL_SECS - waited)
}

/// Whether two cache keys differ only in the view (zoom, center, rotation, canvas size, pixel
/// density and Pixel Preview's document pixels), with Pixel Preview on in both or off in both.
pub(super) fn only_view_differs(a: &CacheKey, b: &CacheKey) -> bool {
    a.pixel.is_some() == b.pixel.is_some()
        && *a == CacheKey { zoom: a.zoom, cx: a.cx, cy: a.cy, w: a.w, h: a.h, ppp: a.ppp, rot: a.rot, pixel: a.pixel, ..b.clone() }
}

pub(super) fn cursor_icon(c: Cursor) -> egui::CursorIcon {
    use egui::CursorIcon as C;
    match c {
        Cursor::Arrow
        | Cursor::ArrowHollow
        | Cursor::DirectAnchor
        | Cursor::DirectAnchorSelected
        | Cursor::ReshapeSegment
        | Cursor::GroupSelection
        | Cursor::CornerRadius
        | Cursor::PathBracket
        | Cursor::TypeWidget
        | Cursor::LoadedText => C::Default,
        Cursor::SelectionCopy => C::Copy,
        Cursor::Move => C::Move,
        Cursor::Crosshair => C::Crosshair,
        Cursor::ResizeH | Cursor::ShearH => C::ResizeHorizontal,
        Cursor::ResizeV | Cursor::ShearV => C::ResizeVertical,
        Cursor::ResizeNwSe => C::ResizeNwSe,
        Cursor::ResizeNeSw => C::ResizeNeSw,
        Cursor::Rotate | Cursor::RotateToward(_) => C::Alias,
        Cursor::Pen
        | Cursor::PenDrawing
        | Cursor::PenAdd
        | Cursor::PenDelete
        | Cursor::PenClose
        | Cursor::PenContinue
        | Cursor::PenJoin
        | Cursor::PenConvert
        | Cursor::AnchorPoint
        | Cursor::Curvature => C::Crosshair,
        Cursor::Text => C::Text,
        Cursor::Hand => C::Grab,
        Cursor::HandGrab => C::Grabbing,
        Cursor::ZoomIn => C::ZoomIn,
        Cursor::ZoomOut => C::ZoomOut,
        Cursor::Eyedropper => C::Crosshair,
        Cursor::NotAllowed => C::NotAllowed,
        Cursor::Busy => C::Progress,
        Cursor::AddStop => C::Copy,
        Cursor::RemoveStop => C::NotAllowed,
        Cursor::Slice => C::Crosshair,
        Cursor::SliceSelect => C::Default,
        Cursor::Width | Cursor::WidthAdd => C::Crosshair,
        Cursor::WidthPoint => C::Move,
        Cursor::PencilNew => C::Crosshair,
        Cursor::Blend | Cursor::BlendObject | Cursor::BlendAnchor | Cursor::ShapeBuilder | Cursor::ShapeBuilderErase => C::Crosshair,
        Cursor::PencilClose | Cursor::PencilStraight | Cursor::PencilContinue | Cursor::PaintbrushClose => C::Crosshair,
        Cursor::TypeVertical => C::VerticalText,
        // Not drawn yet: the cursor each borrows.
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
        | Cursor::PrintTiling
        | Cursor::SymbolSprayer
        | Cursor::SymbolShifter
        | Cursor::SymbolScruncher
        | Cursor::SymbolSizer
        | Cursor::SymbolSpinner
        | Cursor::SymbolStainer
        | Cursor::SymbolScreener
        | Cursor::SymbolStyler
        | Cursor::ObjectsOnPath
        | Cursor::Scissors
        | Cursor::ScissorsNotAllowed
        | Cursor::MeshAdd
        | Cursor::PuppetPin
        | Cursor::AnchorEraser
        | Cursor::ExtendPath
        | Cursor::MeasureInspect
        | Cursor::ParametricShape
        | Cursor::CornerTool
        | Cursor::SlideAnchor
        | Cursor::SharedEdge
        | Cursor::SmartPen => cursor_icon(c.stand_in().unwrap_or(Cursor::Crosshair)),
    }
}
