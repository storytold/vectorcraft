//! Tool cursors as framework cursors: the tools' [`Cursor`] names a look, [`tool_cursor`] maps it to
//! the cursor service's system or painted cursor, and the glyphs the framework has no art for are
//! the SVGs of [`CURSOR_ICONS`] (`assets/cursors`).

// PLAN: this file needs the `craft` facade (feature `behavior`) in crates/ui-egui/Cargo.toml (not in this set's Files table).

use craft::behavior as cb;
use craft::behavior::{CountMark, CursorBadge, CursorIcon, CursorPriority, IconId, PaintedCursor, PointerGlyph};
use craft::ui::primitives::AppIcons;
use egui::Vec2;
use vectorcraft_tools::Cursor;

/// The set of cursor glyphs the framework's catalog does not have. Every name starts with
/// `cursor-` so the catalog and the app's icon set never hold the same name. Registered beside
/// the app's icon set in the host build.
pub static CURSOR_ICONS: AppIcons = AppIcons::new(
    "vectorcraft-cursors",
    None,
    &[
        ("cursor-anchor-point", include_bytes!("../../../assets/cursors/cursor-anchor-point.svg")),
        ("cursor-direct-anchor", include_bytes!("../../../assets/cursors/cursor-direct-anchor.svg")),
        ("cursor-direct-anchor-selected", include_bytes!("../../../assets/cursors/cursor-direct-anchor-selected.svg")),
        ("cursor-loaded-text", include_bytes!("../../../assets/cursors/cursor-loaded-text.svg")),
        ("cursor-loaded-text-link", include_bytes!("../../../assets/cursors/cursor-loaded-text-link.svg")),
        ("cursor-mark-bracket", include_bytes!("../../../assets/cursors/cursor-mark-bracket.svg")),
        ("cursor-mark-edge", include_bytes!("../../../assets/cursors/cursor-mark-edge.svg")),
        ("cursor-mark-line-level", include_bytes!("../../../assets/cursors/cursor-mark-line-level.svg")),
        ("cursor-mark-line-rising", include_bytes!("../../../assets/cursors/cursor-mark-line-rising.svg")),
        ("cursor-mark-ring", include_bytes!("../../../assets/cursors/cursor-mark-ring.svg")),
        ("cursor-mark-square", include_bytes!("../../../assets/cursors/cursor-mark-square.svg")),
        ("cursor-mark-star", include_bytes!("../../../assets/cursors/cursor-mark-star.svg")),
        ("cursor-pen-convert", include_bytes!("../../../assets/cursors/cursor-pen-convert.svg")),
        ("cursor-pen-curvature", include_bytes!("../../../assets/cursors/cursor-pen-curvature.svg")),
        ("cursor-pen-join", include_bytes!("../../../assets/cursors/cursor-pen-join.svg")),
        ("cursor-pen-new", include_bytes!("../../../assets/cursors/cursor-pen-new.svg")),
        ("cursor-reshape-segment", include_bytes!("../../../assets/cursors/cursor-reshape-segment.svg")),
    ],
);

/// Where the glyphs that start at the box's upper left have their tip, in the icon's 24-unit box.
const TIP: Vec2 = Vec2::new(2.0, 2.0);
/// The tip of the glyphs that point down-left (a pipette, a brush, a bucket, a knife).
const LOWER_LEFT: Vec2 = Vec2::new(2.0, 22.0);
/// The middle of the Anchor Point caret's square.
const CARET: Vec2 = Vec2::new(12.0, 6.0);

fn system(icon: CursorIcon) -> cb::Cursor {
    cb::Cursor::System(icon)
}

/// A tool's icon on the pointer, with its `hotspot` and an optional badge.
fn tool(icon: &'static str, hotspot: Vec2, badge: Option<CursorBadge>) -> cb::Cursor {
    PaintedCursor::tool(IconId::new(icon), hotspot, badge).into()
}

/// The pointer `pointer` with the small icon `mark` below and right of it.
fn marked(pointer: PointerGlyph, mark: &'static str) -> cb::Cursor {
    PaintedCursor::pointer_tool(pointer, IconId::new(mark)).into()
}

/// The system arrow with a badge.
fn badged(badge: CursorBadge) -> cb::Cursor {
    PaintedCursor::badged(CursorIcon::Default, badge).into()
}

/// The rotate pointer beside a box, at `octant` from the box's centre (0 up, clockwise in 45° steps).
/// Its arc bends round the corner it is at, so it opens toward the centre: opposite the octant.
fn turn_toward(octant: u8) -> cb::Cursor {
    let angle = f32::from(octant % 8) * std::f32::consts::FRAC_PI_4;
    PaintedCursor::turn(Vec2::new(-angle.sin(), angle.cos())).into()
}

/// The cursor service's cursor for the tools' cursor `c`. A cursor not drawn yet shows the one it
/// borrows ([`Cursor::stand_in`]).
pub fn tool_cursor(c: Cursor) -> cb::Cursor {
    use PointerGlyph::{Arrow, Crosshair};
    match c {
        Cursor::Arrow => PaintedCursor::arrow(false).into(),
        Cursor::ArrowHollow => PaintedCursor::arrow(true).into(),
        Cursor::DirectAnchor => tool("cursor-direct-anchor", TIP, None),
        Cursor::DirectAnchorSelected => tool("cursor-direct-anchor-selected", TIP, None),
        Cursor::ReshapeSegment => tool("cursor-reshape-segment", TIP, None),
        Cursor::GroupSelection => badged(CursorBadge::Add),
        Cursor::SelectionCopy => badged(CursorBadge::Copy),
        Cursor::Move => system(CursorIcon::Move),
        Cursor::Crosshair => PaintedCursor::Crosshair.into(),
        Cursor::ResizeH => system(CursorIcon::ResizeHorizontal),
        Cursor::ResizeV => system(CursorIcon::ResizeVertical),
        Cursor::ResizeNwSe => system(CursorIcon::ResizeNwSe),
        Cursor::ResizeNeSw => system(CursorIcon::ResizeNeSw),
        Cursor::ShearH => PaintedCursor::skew(Vec2::new(1.0, 0.0)).into(),
        Cursor::ShearV => PaintedCursor::skew(Vec2::new(0.0, -1.0)).into(),
        Cursor::Rotate => turn_toward(0),
        Cursor::RotateToward(octant) => turn_toward(octant),
        Cursor::CornerRadius => badged(CursorBadge::Corner),
        Cursor::Pen => tool("cursor-pen-new", TIP, None),
        Cursor::PenDrawing => tool("pen-tool", TIP, None),
        Cursor::PenAdd => tool("pen-tool", TIP, Some(CursorBadge::Add)),
        Cursor::PenDelete => tool("pen-tool", TIP, Some(CursorBadge::Subtract)),
        Cursor::PenClose => tool("pen-tool", TIP, Some(CursorBadge::Close)),
        Cursor::PenContinue => tool("pen-tool", TIP, Some(CursorBadge::Continue)),
        Cursor::PenJoin => tool("cursor-pen-join", TIP, None),
        Cursor::PenConvert => tool("cursor-pen-convert", TIP, None),
        Cursor::Curvature => tool("cursor-pen-curvature", TIP, None),
        Cursor::AnchorPoint => tool("cursor-anchor-point", CARET, None),
        Cursor::Text => system(CursorIcon::Text),
        Cursor::TypeVertical | Cursor::TypeAreaVertical | Cursor::TypeOnPathVertical => system(CursorIcon::VerticalText),
        Cursor::TypeArea => PaintedCursor::type_on(false).into(),
        Cursor::TypeOnPath => PaintedCursor::type_on(true).into(),
        Cursor::Hand => PaintedCursor::Hand.into(),
        Cursor::HandGrab => system(CursorIcon::Grabbing),
        Cursor::ZoomIn => PaintedCursor::zoom(false).into(),
        Cursor::ZoomOut => PaintedCursor::zoom(true).into(),
        Cursor::Eyedropper => tool("pipette", LOWER_LEFT, None),
        Cursor::NotAllowed => system(CursorIcon::NotAllowed),
        Cursor::Busy => system(CursorIcon::Progress),
        Cursor::AddStop => badged(CursorBadge::Add),
        Cursor::RemoveStop => badged(CursorBadge::Subtract),
        Cursor::Slice => marked(Crosshair, "slice"),
        Cursor::SliceSelect => marked(Arrow, "slice"),
        Cursor::Width => marked(Arrow, "dc-width"),
        Cursor::WidthAdd => badged(CursorBadge::Count(CountMark::Add)),
        Cursor::WidthPoint => badged(CursorBadge::Count(CountMark::Move)),
        Cursor::Blend => marked(Crosshair, "square"),
        Cursor::BlendObject => marked(Crosshair, "cursor-mark-square"),
        Cursor::BlendAnchor => marked(Crosshair, "circle"),
        Cursor::PathBracket => marked(Arrow, "cursor-mark-bracket"),
        Cursor::TypeWidget => marked(Arrow, "type"),
        Cursor::LoadedText => tool("cursor-loaded-text", TIP, None),
        Cursor::LoadedTextLink => tool("cursor-loaded-text-link", TIP, None),
        Cursor::ShapeBuilder => marked(Crosshair, "plus"),
        Cursor::ShapeBuilderErase => marked(Crosshair, "minus"),
        Cursor::PencilNew => marked(Crosshair, "cursor-mark-star"),
        Cursor::PencilClose | Cursor::PaintbrushClose => marked(Crosshair, "cursor-mark-ring"),
        Cursor::PencilStraight => marked(Crosshair, "cursor-mark-line-level"),
        Cursor::PencilContinue => marked(Crosshair, "cursor-mark-line-rising"),
        // Only the centre: the tool draws the tip's outline round it, at the zoom.
        Cursor::BlobBrush | Cursor::Eraser => PaintedCursor::circle(1.25).into(),
        Cursor::Knife => tool("dc-knife", LOWER_LEFT, None),
        Cursor::LivePaintBucket => tool("paint-bucket", LOWER_LEFT, None),
        Cursor::LivePaintBucketFace => tool("dc-live-bucket", LOWER_LEFT, None),
        Cursor::LivePaintBucketEdge => tool("paintbrush", LOWER_LEFT, None),
        Cursor::LivePaintSelectFace => marked(Arrow, "cursor-mark-square"),
        Cursor::LivePaintSelectEdge => marked(Arrow, "cursor-mark-edge"),
        Cursor::LivePaintSelectOutside => marked(Arrow, "x"),
        Cursor::Lasso
        | Cursor::MagicWand
        | Cursor::Pencil
        | Cursor::Paintbrush
        | Cursor::Smooth
        | Cursor::PathEraser
        | Cursor::Join
        | Cursor::Shaper
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
        | Cursor::SmartPen => c.stand_in().map_or_else(|| PaintedCursor::Crosshair.into(), tool_cursor),
    }
}

/// While a long operation runs (a file opens, a background save or export), the busy pointer
/// over the whole window, above every component's and the tools' cursors.
pub(crate) fn request_busy(app: &crate::VectorcraftApp, ctx: &egui::Context) {
    if crate::opening::long_operation_busy(app) {
        craft::ui::runtime::cursor::request(ctx, system(CursorIcon::Progress), CursorPriority::Drag);
    }
}

/// Every cursor with a look: its own, or the one it borrows.
#[cfg(test)]
const GLYPHS: [Cursor; 99] = [
    Cursor::Hand,
    Cursor::HandGrab,
    Cursor::ZoomIn,
    Cursor::ZoomOut,
    Cursor::Arrow,
    Cursor::ArrowHollow,
    Cursor::DirectAnchor,
    Cursor::DirectAnchorSelected,
    Cursor::ReshapeSegment,
    Cursor::GroupSelection,
    Cursor::SelectionCopy,
    Cursor::Move,
    Cursor::Crosshair,
    Cursor::ResizeH,
    Cursor::ResizeV,
    Cursor::ResizeNwSe,
    Cursor::ResizeNeSw,
    Cursor::ShearH,
    Cursor::ShearV,
    Cursor::Rotate,
    Cursor::RotateToward(1),
    Cursor::CornerRadius,
    Cursor::Pen,
    Cursor::PenDrawing,
    Cursor::PenAdd,
    Cursor::PenDelete,
    Cursor::PenClose,
    Cursor::PenContinue,
    Cursor::PenJoin,
    Cursor::PenConvert,
    Cursor::AnchorPoint,
    Cursor::Curvature,
    Cursor::Text,
    Cursor::Eyedropper,
    Cursor::AddStop,
    Cursor::RemoveStop,
    Cursor::Slice,
    Cursor::SliceSelect,
    Cursor::Width,
    Cursor::WidthAdd,
    Cursor::WidthPoint,
    Cursor::Blend,
    Cursor::BlendObject,
    Cursor::BlendAnchor,
    Cursor::ObjectsOnPath,
    Cursor::PathBracket,
    Cursor::TypeWidget,
    Cursor::LoadedText,
    Cursor::LoadedTextLink,
    Cursor::ShapeBuilder,
    Cursor::ShapeBuilderErase,
    Cursor::Lasso,
    Cursor::MagicWand,
    Cursor::Pencil,
    Cursor::PencilNew,
    Cursor::PencilClose,
    Cursor::PencilStraight,
    Cursor::PencilContinue,
    Cursor::Paintbrush,
    Cursor::PaintbrushClose,
    Cursor::BlobBrush,
    Cursor::Eraser,
    Cursor::Smooth,
    Cursor::PathEraser,
    Cursor::Join,
    Cursor::Knife,
    Cursor::Shaper,
    Cursor::LivePaintBucket,
    Cursor::LivePaintBucketFace,
    Cursor::LivePaintBucketEdge,
    Cursor::LivePaintSelectFace,
    Cursor::LivePaintSelectEdge,
    Cursor::LivePaintSelectOutside,
    Cursor::Measure,
    Cursor::PrintTiling,
    Cursor::SymbolSprayer,
    Cursor::SymbolShifter,
    Cursor::SymbolScruncher,
    Cursor::SymbolSizer,
    Cursor::SymbolSpinner,
    Cursor::SymbolStainer,
    Cursor::SymbolScreener,
    Cursor::SymbolStyler,
    Cursor::TypeVertical,
    Cursor::TypeArea,
    Cursor::TypeAreaVertical,
    Cursor::TypeOnPath,
    Cursor::TypeOnPathVertical,
    Cursor::Scissors,
    Cursor::MeshAdd,
    Cursor::PuppetPin,
    Cursor::AnchorEraser,
    Cursor::ExtendPath,
    Cursor::MeasureInspect,
    Cursor::ParametricShape,
    Cursor::CornerTool,
    Cursor::SlideAnchor,
    Cursor::SharedEdge,
    Cursor::SmartPen,
];

#[cfg(test)]
mod tests;
