//! What panels drag onto art and onto each other.

use vectorcraft_color::Paint;

/// What panels drag onto art and onto each other. One payload type, so the canvas has one drop
/// handler for all of them (`canvas::panel_drop`); a chip of a dragged paint follows the pointer.
/// Lists whose rows drag their own key (styles, symbols, brushes, libraries) carry a `ListDrag`
/// instead.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelDrag {
    /// A paint: Swatches panel tiles, a Fill/Stroke proxy, a Recent Colors chip, a Color Guide
    /// color or the Gradient panel's thumbnail. Art it is dropped on takes `params` (`{swatch}`,
    /// `{color}`, `{gradient}`…; null for a colour group, which paints nothing) through the active
    /// proxy's `paint.setFill`/`paint.setStroke`; the Gradient panel's ramp takes its colour as a
    /// stop; the Swatches panel moves `rows`, or makes a swatch of a paint dragged from elsewhere.
    /// `library`: the Libraries panel colour it is (library id, colour name), which that panel
    /// moves to where it is dropped.
    Paint { paint: Paint, params: serde_json::Value, rows: Option<SwatchRows>, library: Option<(String, String)> },
    /// The Appearance panel's thumbnail: art it is dropped on takes object `0`'s appearance
    /// (`appearance.copyFrom`); the Graphic Styles panel makes a style of it, or with Alt held
    /// replaces the style it is dropped on with it (`graphicStyle.redefine`).
    Appearance(vectorcraft_doc::NodeId),
    /// The selected art, dragged off the canvas with the Selection tool (`document`: the uid of the
    /// document it came from): the Graphic Styles panel makes a style of the first object
    /// (`graphicStyle.new`), or with Alt held replaces the style it is dropped on with that
    /// object's look (`graphicStyle.redefine`), the Symbols panel a symbol of it all
    /// (`symbol.new`), the Swatches panel a pattern swatch of a copy (`object.pattern.make`) and
    /// the Brushes panel an Art brush (`brush.new`).
    Art { document: u64, ids: Vec<vectorcraft_doc::NodeId> },
    /// A Libraries panel graphic (`item`: its id in `library`): the canvas places a copy centred
    /// where it is dropped (`library.use`); the panel moves it to where it is dropped.
    LibraryGraphic { library: String, item: String },
    /// A Libraries panel text style (`item`: its name in `library`), which the panel moves to where
    /// it is dropped.
    LibraryItem { library: String, kind: vectorcraft_engine::cmd::library::ItemKind, item: String },
}

/// The Swatches panel rows a drag from that panel moves: the swatch, None, Registration or colour
/// group under the pointer when the drag started (`grabbed`) and `names`, the panel selection when
/// the grabbed one is part of it (only colour groups, or only swatches, like the grabbed one), in
/// panel order.
#[derive(Clone, Debug, PartialEq)]
pub struct SwatchRows {
    /// The document whose rows the drag started on.
    pub document: u64,
    pub grabbed: String,
    pub names: Vec<String>,
    /// The names are colour groups.
    pub groups: bool,
    /// Whether the grabbed swatch's paint can travel as an unlinked process color.
    pub unlinked_process_color: bool,
}

impl PanelDrag {
    /// `paint` dragged from a Fill/Stroke proxy, a Recent Colors chip, a Color Guide color or the
    /// Gradient panel's thumbnail. A gradient carries no placement: it fits the art it lands on
    /// (freeform points are placed afresh there, coloured like the dragged ones).
    pub fn paint(mut paint: Paint) -> Self {
        if let Paint::Gradient(g) = &mut paint {
            g.geom = None;
            g.freeform = None;
        }
        Self::Paint { params: crate::panels::paint_params(&paint), paint, rows: None, library: None }
    }

    /// The dragged colour (a solid paint's), which the Gradient panel's ramp takes.
    pub fn color(&self) -> Option<vectorcraft_color::Color> {
        match self {
            Self::Paint { paint, .. } => paint.color(),
            _ => None,
        }
    }
}
