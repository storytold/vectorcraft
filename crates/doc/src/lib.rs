//! VectorCraft document model: artboards, layers and objects, appearance, swatches (pure data + serde).
//!
//! Documents are persistent trees: children are `Arc<Node>`, and edits go through
//! [`Document::node_mut`], which clones only the nodes on the path from the root to the edited node.
//! Keeping the previous `Document` value around is therefore a cheap undo snapshot.
#![forbid(unsafe_code)]

pub mod appearance;
pub mod assets;
pub mod blend;
pub mod cancel;
pub mod clipnest;
pub mod cmyk;
pub mod corners;
pub mod depth;
pub mod dimension;
pub mod graph;
pub mod hit;
mod id_index;
pub mod inks;
mod inline;
pub mod intertwine;
pub mod shared;
mod symbol_export_type;
mod symbol_nine_slice;
mod symbol_registration;
pub use inline::{SYMBOL_HALF, SYMBOL_SIZES};
pub use shared::Shared;
pub use symbol_export_type::{SYMBOL_EXPORT_TYPE, SymbolExportType};
pub use symbol_nine_slice::{SYMBOL_NINE_SLICE, SymbolNineSlice};
pub use symbol_registration::{SYMBOL_REGISTRATION, SymbolRegistration};
pub mod links;
pub mod live;
pub mod live_paint;
pub mod marks;
mod mem;
pub mod metadata;
pub mod node;
pub mod objects_on_path;
pub mod orient;
pub mod overprint;
pub mod pattern;
pub mod perspective;
mod pixels;
pub use pixels::MAX_RASTER_ALLOC;
pub mod placed_document;
pub mod plugin_data;
pub mod profiles;
pub mod puppet;
pub mod range;
pub mod rastersettings;
mod reach;
pub mod recolor;
pub mod selection;
pub mod setup;
pub mod shaper;
pub mod slices;
pub mod style_libs;
pub mod swatches;
mod symbols;
pub use symbols::SymbolBoxes;
pub mod text;
pub mod trace;
pub mod variables;
pub mod xml_id;

use std::collections::BTreeMap;
use std::sync::Arc;

/// `skip_serializing_if` predicates: fields equal to their serde default are not written.
pub(crate) mod skip;

pub use appearance::StrokeGradientMode;
pub use appearance::{
    Appearance, AppearanceItem, ArrowAlign, Arrowhead, Dash, Effect, FillLayer, LineCap, LineJoin, PressureProfile, ProfilePreset, SavedProfile,
    StrokeAlign, StrokeLayer, WidthProfile,
};
pub use assets::ExportAsset;
pub use corners::LiveCorners;
pub use depth::{MAX_DEPTH, MAX_SYMBOL_NODES};
pub use graph::{
    ColumnDesign, ColumnType, Fractions, GraphDesign, GraphKind, GraphSpec, LabelStyles, MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES, PiePosition,
    PieSort, SeriesPaint, TickLength, ValueAxisSide,
};
pub use hit::{Hit, HitKind};
pub use links::{LinkInfo, PlacementOptions};
pub use live::{BlendEase, BlendEaseType, BlendOrientation, BlendSpacing, BlendSpec, EnvelopeKind, GradientMesh, MeshPoint};
pub use live_paint::{GapOptions, GapSize};
pub use metadata::{CopyrightStatus, DocMetadata};
pub use node::Knockout;
pub use node::Scaling;
pub use node::{Extras, ExtrasMut, ImageObject, LAYER_COLORS, LayerColor, LiveShape, Node, NodeExtras, NodeId, NodeKind, OpacityMask};
pub use node::{ForeignAttribute, ImageMap, MAX_SVG_SCRIPTS, ObjectAttributes, SVG_EVENTS};
pub use orient::OrientedBox;
pub use pattern::{Overlap, PatternDef, PatternEdit, RepeatKind, RepeatSpec, TileType};
pub use perspective::PerspectiveAttachment;
pub use placed_document::{PageBox, PlacedDocument};
pub use profiles::ColorProfiles;
pub use puppet::{PuppetPin, PuppetPins};
pub use rastersettings::{RasterColorModel, RasterEffectsSettings};
pub use selection::{AnchorRef, SegmentRef, Selection};
pub use setup::{Background, DocSetup, ExportText, GridSize, Quotes};
pub use slices::{CellAlign, CellVAlign, Slice, SliceArea, SliceKind, SliceOptions, SliceSource};
pub use style_libs::StyleLibrary;
pub use text::{
    AreaFit, AreaOptions, Burasagari, CharAlign, CharPosition, CharStyle, Composer, Digits, FirstBaseline, HyphenationOptions, InlineArt,
    JustificationOptions, Justify, Kinsoku, LeadingModel, Mojikumi, ParaDirection, ParaStyle, PathAlign, PathEffect, RubyAlign, RubyPosition,
    RubySpan, ScriptMetrics, SingleWordJustify, TabAlign, TabStop, TextKind, TextObject, TextRun, TextStyleDef, TextWrap, VerticalAlign, WrapShape,
};
pub use trace::TraceView;
pub use variables::{DataSet, DataValue, Variable, VariableKind, Variables};
pub use vectorcraft_color as color;
pub use vectorcraft_geom as geom;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Swatch, SwatchGroup};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

mod artboard;
mod document;
mod document_ext;
mod document_guides;
mod graphic_style;
mod guides;
mod limits;
mod records;
mod units;

pub use artboard::{Artboard, LockedArtboards, RetiredArtboardId, VIDEO_PIXEL_ASPECT_RANGE, VIDEO_SAFE_INSETS};
#[cfg(test)]
use document_ext::free_id;
pub use graphic_style::{DEFAULT_GRAPHIC_STYLE, GraphicStyle};
pub use guides::{GridPrefs, Guide, RulerMode};
#[cfg(test)]
use limits::MAX_NEXT_ID;
pub use limits::{DocError, FORMAT_VERSION, MAX_COORD};
pub use records::{ImageBlob, MaskEdit, NewArtIds, SavedSelection, SavedView, Symbol};
pub use units::{ColorMode, Unit, parse_number};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    #[serde(default)]
    pub title: String,
    /// Saved with File → Save as Template: opening it starts a new untitled document.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub template: bool,
    #[serde(default)]
    pub units: Unit,
    /// The rulers' zero point in document coordinates. Moving it changes measurements, never art.
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub ruler_origin: Point,
    /// View › Rulers › Change to Global / Artboard Rulers: whether the rulers and position fields
    /// measure from [`Self::ruler_origin`] (global) or from the active artboard's own origin.
    /// Documents saved before the choice existed measured globally and open that way.
    #[serde(default = "RulerMode::legacy")]
    pub rulers: RulerMode,
    /// Artboard rulers' zero point of each artboard (by [`Artboard::id`]), as an offset from its
    /// top-left corner; an artboard not listed measures from that corner.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artboard_ruler_origins: BTreeMap<u32, Vec2>,
    #[serde(default)]
    pub color_mode: ColorMode,
    pub artboards: Vec<Artboard>,
    /// Top-level layers, bottom first.
    pub layers: Vec<Arc<Node>>,
    // The document-wide resources below are [`Shared`]: an edit copies the document (copy-on-write)
    // and the undo history keeps every version, so what an edit doesn't change stays one copy.
    // Reads, writes and the saved form are those of the value itself.
    #[serde(default)]
    pub swatches: Shared<Vec<Swatch>>,
    #[serde(default)]
    pub swatch_groups: Shared<Vec<SwatchGroup>>,
    #[serde(default)]
    pub graphic_styles: Shared<Vec<GraphicStyle>>,
    /// Character styles (besides the built-in [Normal Character Style], which may be redefined here).
    #[serde(default, skip_serializing_if = "skip::is_empty_vec")]
    pub char_styles: Shared<Vec<TextStyleDef>>,
    /// Paragraph styles (besides the built-in [Normal Paragraph Style]).
    #[serde(default, skip_serializing_if = "skip::is_empty_vec")]
    pub para_styles: Shared<Vec<TextStyleDef>>,
    /// Threaded text: area-type frames one story flows through, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_threads: Vec<Vec<NodeId>>,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// Object › Graph › Design…: art graphs can draw as markers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graph_designs: Vec<GraphDesign>,
    #[serde(default)]
    pub guides: Vec<Guide>,
    /// View → New View… (at most [`SavedView::MAX`]); saved with the document.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<SavedView>,
    /// Select → Save Selection… (at most [`SavedSelection::MAX`]); saved with the document.
    #[serde(default, skip_serializing_if = "skip::is_empty_vec")]
    pub saved_selections: Shared<Vec<SavedSelection>>,
    /// Variables (data merge): named values bound to objects, applied per dataset.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub variables: Shared<Variables>,
    #[serde(default)]
    pub grid: GridPrefs,
    #[serde(default = "ppi72")]
    pub raster_effects_ppi: f64,
    #[serde(default)]
    pub images: BTreeMap<String, ImageBlob>,
    /// Pattern swatch definitions (Object → Pattern).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<PatternDef>,
    /// Pattern editing mode, while active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern_edit: Option<PatternEdit>,
    /// Opacity-mask editing mode, while active. Never saved (see [`Document::without_edit_modes`]);
    /// still read so files saved mid-edit by older versions load without the editing layer.
    #[serde(default, skip_serializing)]
    pub mask_edit: Option<MaskEdit>,
    next_id: u64,
    /// Foreign data preserved on round-trip.
    #[serde(default, skip_serializing_if = "skip::is_empty_map")]
    pub unknown: Shared<serde_json::Map<String, serde_json::Value>>,
    /// Transparency panel → Page Isolated Blending: the page is an isolated transparency group,
    /// so top-level blend modes don't blend with what lies under the page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_isolate: bool,
    /// Transparency panel → Page Knockout Group: the page's elements (its layers; neutral layers
    /// pass their contents through) knock each other out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_knockout: bool,
    /// Swatches panel → Spot Colors: spot colours defined in Lab show and separate from their Lab
    /// values (on, the default) or from their working-CMYK equivalents (off)
    /// ([`Document::linked_color`]).
    #[serde(default = "yes", skip_serializing_if = "skip::is_true")]
    pub spot_use_lab: bool,
    /// File → Document Setup (bleed, transparency grid, paper, type options).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub setup: DocSetup,
    /// Layers panel → Paste Remembers Layers: pasted objects go back into the layers (by name)
    /// they were copied from instead of the current layer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paste_remembers_layers: bool,
    /// File → File Info (the title is [`Document::title`]).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub metadata: DocMetadata,
    /// Effect → Document Raster Effects Settings besides the resolution
    /// ([`Document::raster_effects_ppi`]).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub raster_effects: RasterEffectsSettings,
    /// The view (zoom, centre, rotation) the document was saved with; it reopens there. Written at
    /// save time only, so changing the view never marks the document modified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_view: Option<SavedView>,
    /// View → Show Transparency Grid when the document was saved. Written at save time only,
    /// like [`Document::last_view`]; older files open with the grid hidden.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub transparency_grid: bool,
    /// The layers, sublayers and groups open in the Layers panel when the document was saved; they
    /// reopen that way. `None`: the default, only the top-level layers open. Written at save time
    /// only, like [`Document::last_view`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers_open: Option<Vec<NodeId>>,
    /// Edit → Assign Profile: the profiles the document is tagged with (files before format v3
    /// kept them in `unknown`, see [`Document::migrate_color_profiles`]).
    #[serde(default, skip_serializing_if = "ColorProfiles::is_empty")]
    pub color_profiles: ColorProfiles,
    /// File → Export for Screens: the settings it last exported with (`document.exportForScreens`
    /// params), so the dialog reopens on them.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub export_settings: serde_json::Map<String, serde_json::Value>,
    /// Top-level keys this version doesn't know (written by a newer one), kept so saving doesn't
    /// lose them. Separate from [`Document::unknown`], which holds foreign data by design.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
    /// User slices (Slice tool, Object → Slice); object slices are [`Node::slice`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<Slice>,
    /// Object → Slice → Clip to Artboard: slices are clipped to the artboards and auto slices fill
    /// them (on, the default); off, auto slices cover the art and the slices.
    #[serde(default = "yes", skip_serializing_if = "skip::is_true")]
    pub slices_clip_to_artboard: bool,
    /// File → Print: the print settings saved with the document (`vectorcraft_pdf::PrintSettings`
    /// as JSON: the print engine sits above this crate); `None` until they are set up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_setup: Option<serde_json::Value>,
    /// Window → Asset Export: art collected for export ([`ExportAsset`]), in panel order. Their
    /// export settings are Export for Screens' ([`Document::export_settings`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<ExportAsset>,
    /// Window › SVG Interactivity › Link JavaScript Files: the JavaScript files (paths or URLs, as
    /// given) SVG export links with `<script>` elements, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svg_scripts: Vec<String>,
    /// Puppet Warp pins on the selected artwork while the tool edits it ([`PuppetPins`]): editing
    /// state, never saved.
    #[serde(skip)]
    pub puppet: Option<Arc<PuppetPins>>,
    /// New art on a locked artboard: objects of layers on a locked artboard that an edit made and
    /// left selected (a group with selected members counts), kept while they stay selected and on
    /// one. They and their contents can be selected and edited ([`LockedArtboards::holds`]). The
    /// engine sets it from the selection's ([`Selection::new_art`]). Editing state, never saved.
    #[serde(skip)]
    pub new_art: NewArtIds,
    /// The highest artboard id given out ([`Document::next_artboard_id`]) or deleted from the
    /// document ([`Document::retire_artboard_id`]): a new artboard takes a higher one. Shared by
    /// every copy of the document, the undo history's included, so an undo does not lower it.
    /// Never saved.
    #[serde(skip)]
    pub retired_artboard_id: RetiredArtboardId,
    /// Some object is a dimension (the Dimension tool): edits refresh dimensions only while this
    /// is set, so documents without any pay nothing for them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dimensions: bool,
    /// Where each node is in the tree, for lookups by id ([`id_index`]): a cache, never saved.
    #[serde(skip)]
    index: id_index::IdIndex,
}

fn ppi72() -> f64 {
    72.0
}

fn yes() -> bool {
    true
}

/// Is point `p` on the artboard at `rect`, its edges included, within a millionth of a point? This
/// is the rule [`Document::art_on_artboard`] applies to art flush with an edge (#890), where
/// `Rect::contains` leaves out the right and bottom edges.
pub fn artboard_holds(rect: Rect, p: Point) -> bool {
    let r = rect.inflate(1e-6, 1e-6);
    r.x0 <= p.x && p.x <= r.x1 && r.y0 <= p.y && p.y <= r.y1
}

/// Listed nodes by parent: the parent's index path (empty for the top-level layers), then the index
/// of each listed child in it, in order, with the child's place in the list.
type Places = BTreeMap<Vec<usize>, Vec<(usize, usize)>>;

#[cfg(test)]
mod tests;
