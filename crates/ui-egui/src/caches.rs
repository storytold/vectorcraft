use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::Value;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{Point, Vec2};

use super::{canvas, render_worker};

/// Cached canvas raster.
pub struct CanvasCache {
    pub renderer: vectorcraft_render::Renderer,
    /// The text caret shown last (its ends' coordinates' bits) and when it got there (seconds,
    /// egui's input time): it blinks from that moment (`blink_caret` in the canvas's overlays).
    pub caret: Option<([u64; 4], f64)>,
    pub texture: Option<egui::TextureHandle>,
    pub key: Option<CacheKey>,
    /// The document the frame shown was rendered from, and the options it was rendered with, with
    /// [`Self::key`]: when only the document changes, the canvas renders again only where it
    /// changed, measured by [`Self::renderer`], which drew it ([`vectorcraft_render::Renderer::changed`]).
    pub shown: Option<(std::sync::Arc<vectorcraft_doc::Document>, vectorcraft_render::RenderOptions)>,
    pub last_ms: f64,
    /// The frame shown drew raster effects from rasters made before their objects moved, or at a
    /// drag's lower resolution ([`vectorcraft_render::FrameStats::moved`]): the next frame outside
    /// an interaction runs them again, on the worker when there is one.
    pub moved: bool,
    /// The frame shown was rendered on the worker: an interaction's frames stay there
    /// ([`canvas::show`]).
    pub from_worker: bool,
    /// The frame shown drew art in a way that depends on its size, or a part of it drawn again on
    /// this thread would not have matched the frame around it. Edits that reach the affected area
    /// render whole, on the worker when the frame is heavy ([`canvas::show`]).
    pub whole_frames: bool,
    /// The shown web frame can still be patched outside this enclosure in its own raster pixels.
    /// Present only when all frame-size-dependent output has a known bound.
    pub view_sized_bounds: Option<vectorcraft_geom::Rect>,
    /// Draws for a patch against a web frame with bounded view-sensitive art.
    /// The renderer of the whole frame keeps its raster and derived-art caches across these patches.
    pub web_patch_renderer: Option<Box<vectorcraft_render::Renderer>>,
    /// The frame shown holds pixels the worker copied from its last frame, and when it was shown
    /// (seconds, egui's input time): it renders whole once it has been shown for
    /// [`canvas::STILL_SECS`] with the view unmoved.
    pub reused: Option<f64>,
    /// Set in the web build, which has no render worker: while the view changes on a document whose
    /// last render took more than 8 ms, the canvas draws the frame shown reprojected and renders
    /// once the view has been still for [`canvas::STILL_SECS`].
    pub waits_for_still: bool,
    /// The cache key the canvas last asked for and when it first asked for its view (seconds,
    /// egui's input time), kept while [`Self::waits_for_still`] is set.
    pub still: Option<(CacheKey, f64)>,
    /// The low-resolution frame that covers the parts of the canvas the frame shown leaves
    /// uncovered while it is drawn for a view it was not rendered for ([`canvas::Fill`]).
    pub fill: Option<canvas::Fill>,
    /// The thread that renders [`Self::fill`], beside [`Self::worker`]. None on wasm, which has no
    /// fill.
    pub fill_worker: Option<render_worker::Worker>,
    pub worker: Option<render_worker::Worker>,
    pub worker_started: bool,
    /// The slices as laid out for (document uid, revision): the canvas draws them every frame.
    pub slices: Option<SliceCache>,
    /// The print tiling's pages for (document uid, revision) (View → Show Print Tiling).
    pub print_tiling: Option<PrintTilingCache>,
    /// The gap lines View → Show Live Paint Gaps draws for (document uid, revision).
    pub live_paint_gaps: Option<LivePaintGapsCache>,
    /// Type → Find Font's list of the document's fonts and whether the document misses one, for a
    /// version of the document's content and of the available fonts: the dialog reads them every
    /// frame.
    pub find_font: Option<FindFontCache>,
    /// The marks Type → Show Hidden Characters drew on each text object in the last frame. A frame
    /// that does not draw them drops them ([`canvas::show`]).
    pub hidden_chars: canvas::marks::HiddenChars,
    /// The words Edit › Spelling › Auto Spell Check underlined on each text object in the last
    /// frame; dropped while it is off.
    pub spelling: canvas::marks::SpellingMarks,
    /// The active document's visible text objects for (document uid, revision), which Show Hidden
    /// Characters and Auto Spell Check draw over. A pass that draws neither drops them
    /// ([`canvas::marks::marks_of`]).
    pub text_nodes: canvas::marks::TextNodes,
    /// The Hand, Zoom and Rotate View tools the Space key and its chords lend the canvas, between
    /// passes; the tool adapter takes it out for its call and puts it back.
    pub momentary: Option<craft::ui::canvas::Momentary>,
    /// [`VectorcraftApp::selection_box`] for (document uid, revision, Use Preview Bounds and
    /// Measure Type by Glyph Bounds).
    pub selection_box: Option<((u64, u64, vectorcraft_effects::preview::Measure), Option<vectorcraft_doc::OrientedBox>)>,
    /// [`VectorcraftApp::selection_bounds`] for (document uid, revision).
    pub selection_bounds: Option<((u64, u64), Option<vectorcraft_geom::Rect>)>,
    /// [`VectorcraftApp::content_bounds`] for (document uid, revision).
    pub content_bounds: Option<((u64, u64), Option<vectorcraft_geom::Rect>)>,
    /// Whether the 3D and Materials panel's Export 3D Object button is available (a selected
    /// object has a 3D effect that `assets.add3D` exports) for (document uid, revision).
    pub three_d_exportable: Option<((u64, u64), bool)>,
    /// The Transform panel's Align to Pixel Grid state (on, off or mixed, and whether every
    /// selected object can be aligned) for (document uid, revision).
    pub pixel_aligned: Option<((u64, u64), Option<PixelAligned>)>,
    /// The symbol instances in the selection and its groups, for (document uid, revision): the
    /// Properties and Symbols panels read them every frame.
    pub symbol_instances: Option<((u64, u64), SymbolInstances)>,
    /// [`CacheKey::plugins`] by document uid, with the plug-in revision
    /// ([`vectorcraft_render::effects::plugin_revision`]) it was last checked at.
    pub plugin_keys: std::collections::HashMap<u64, (u64, u64)>,
    /// [`CacheKey::placed`] by document uid ([`canvas::raster::placed_key`]).
    pub placed_keys: std::collections::HashMap<u64, PlacedKey>,
    /// The view the canvas last drew and when it last changed (egui's input time): while it moves,
    /// the canvas doesn't hit-test the art under the pointer ([`canvas::hover_wait`], #2452).
    pub view_moved: Option<(canvas::ViewKey, f64)>,
    /// The object the Eyedropper's click at the pointer would act on, which Object Highlighting
    /// outlines, with what it was found for: finding it again, which can sample a color, waits
    /// until one of those changes.
    pub hover_object: Option<(canvas::HoverObjectKey, Option<vectorcraft_doc::NodeId>)>,
    /// Where the press of the gesture running on the canvas landed, in document points: a move carried out of the
    /// canvas drops its art there relative to the pointer (`canvas::drops`).
    pub grab: Option<Point>,
    /// The uid of the document a tab rest showed while art carried off the canvas waits over it: only its canvas takes
    /// the art, so a target closed or left by other means carries nothing (`canvas::drops`).
    pub carry_target: Option<u64>,
    /// The last drag the tool was sent and the screen point it was at: when the view scrolls under the held
    /// pointer, the drag goes on at that point in the scrolled view.
    pub last_drag: Option<(egui::Pos2, vectorcraft_tools::PointerEvent)>,
    /// Escape cancelled the tool's press while the button is still down: its release still sends the tool its Up,
    /// which gives back a selection tool Cmd lent.
    pub escaped_press: bool,
    /// The host's Caps Lock state at the last canvas pass, which `VectorcraftApp::follow_caps_lock` compares with the
    /// next reading; `None` before the first pass.
    pub host_caps_lock: Option<bool>,
    /// A press on the canvas committed a panel field's running preview before the tool took the press, as the field's
    /// commit came first in production: the field's own commit later in the pass has nothing left to run.
    pub committed_preview: bool,
    /// Window › Arrange › Tile: the renders the panes of the other documents draw.
    pub panes: canvas::Panes,
    /// Window › Arrange › Float in Window: what the floating document windows keep between frames.
    pub windows: crate::doc_windows::Windows,
    /// Whether the selection holds images beside other art, for (document uid, revision): the
    /// Contextual Task Bar reads it every frame.
    pub images_with_other_art: Option<((u64, u64), bool)>,
    /// Whether the Smooth tool can work on the selection's paths, for (document uid, revision).
    pub smooth_paths: Option<((u64, u64), bool)>,
    /// Whether the selection mixes colours, for (document uid, revision): the Properties panel's
    /// Recolor row reads it.
    pub multi_color: Option<((u64, u64), bool)>,
    /// How many objects Start Global Edit would edit, for the key the Properties panel builds.
    pub(crate) global_edit_count: Option<(crate::panels::properties::GlobalEditKey, usize)>,
    /// The selected live shape's angle, for (document uid, revision, kind of shape).
    pub shape_angle: Option<ShapeAngleCache>,
    /// The Image Trace presets, for the session's preset revision.
    pub(crate) trace_presets: Option<(u64, Arc<crate::panels::image_trace::PresetList>)>,
    /// What the Image Trace panel traces, for (document uid, revision).
    pub(crate) trace_target: Option<((u64, u64), Arc<crate::panels::image_trace::Target>)>,
    /// The Attributes panel's values and whether they apply, for the panel's key.
    pub(crate) attributes_info: Option<(crate::panels::attributes::AttrKey, (vectorcraft_engine::cmd::AttributesInfo, bool))>,
    /// The SVG Interactivity panel's values, for the panel's key.
    pub(crate) svg_info: Option<(crate::panels::svg_interactivity::SelKey, crate::panels::svg_interactivity::Info)>,
    /// The CSS Properties panel's code, for the panel's key.
    pub(crate) css_shown: Option<(crate::panels::css_properties::CssKey, Arc<crate::panels::css_properties::Shown>)>,
    /// The SVG Options dialog's SVG code, for every input it reads (compared with `==`).
    pub(crate) svg_code: Option<(crate::dialogs::svg_options::SvgCodeKey, Arc<str>)>,
    /// The Export for Screens dialog's artboard thumbnails, for (document uid, revision).
    pub artboard_thumbs: Option<ArtboardThumbsCache>,
    /// The families the document uses, for the document's fonts key.
    pub document_families: Option<(FontsKey, Arc<[String]>)>,
    /// The families installed lately: the font generation, when it was asked, the families and
    /// the newest install time among them.
    pub recently_added: Option<RecentlyAddedCache>,
    /// The kerning at the Type tool's caret, for (document uid, revision, text object, byte).
    pub caret_kerning: Option<((u64, u64, NodeId, usize), f64)>,
    /// The alternates the Glyphs panel found at the selection.
    pub(crate) glyphs_found: crate::panels::glyphs::Found,
    /// The faces the Glyphs panel's positional forms come from: (document uid, revision, font
    /// generation, the selection's object and range, the selected objects).
    pub glyph_form_faces: Option<GlyphFormFacesCache>,
    /// The selected characters' outlines on the page for the alternates row: (document uid,
    /// revision, text object, range) to the quads and whether they are cut.
    pub character_quads: Option<CharacterQuadsCache>,
    /// The Tabs panel's guide line, for (document uid, text object, paragraph, revision): layout is
    /// too slow to run on every frame of a stop's drag.
    pub tabs_guide_axis: Option<TabsGuideAxisCache>,
    /// The Appearance panel's summary of the selection, which holds the document, revision and
    /// subjects it was made for.
    pub(crate) appearance_summary: Option<Arc<crate::panels::appearance::Summary>>,
    /// The Transparency panel's values of the selection and its first object, for the panel's key.
    pub(crate) transparency_state:
        Option<(crate::panels::transparency::StateKey, vectorcraft_engine::cmd::TransparencyInfo, Option<Arc<vectorcraft_doc::Node>>)>,
    /// The Navigator's view region, for (document uid, revision, View Artboard Contents Only).
    pub nav_region: Option<((u64, u64, bool), vectorcraft_geom::Rect)>,
    /// The Document Info panel's values, for (document uid, revision, selection only, options).
    pub doc_info: Option<DocInfoCache>,
    /// The Variables panel's rows, for the panel's key.
    pub(crate) variable_rows: Option<(crate::panels::variables::RowsKey, Rc<crate::panels::variables::Rows>)>,
    /// The Map Art surfaces of the 3D dialog's object.
    pub map_surfaces: Option<MapSurfacesCache>,
    /// The command palette's built rows, for the palette's build key.
    pub(crate) palette: Option<(crate::palette::BuildKey, Arc<crate::palette::Built>)>,
}

/// [`CanvasCache::doc_info`]: (document uid, revision, selection only, the Artboard tool's options)
/// and the `document.info` result fetched for them.
pub type DocInfoCache = ((u64, u64, bool, Value), Arc<Value>);

/// [`CanvasCache::map_surfaces`]: the surfaces and what they were computed for.
pub type MapSurfacesCache = (MapSurfacesKey, Arc<Vec<vectorcraft_effects::MapSurface>>);

/// What the Map Art surfaces depend on: the document's version, the 3D object and the appearance
/// item, the effect and its parameters. Compared with `==`, never hashed.
#[derive(Clone, Debug, PartialEq)]
pub struct MapSurfacesKey {
    /// The document's uid.
    pub uid: u64,
    /// The document's revision.
    pub revision: u64,
    /// The 3D object.
    pub object: NodeId,
    /// The appearance item the effect is on, when not the object's own.
    pub item: Option<usize>,
    /// The effect's index in its list.
    pub index: Option<usize>,
    /// The effect's id.
    pub effect: String,
    /// The effect's parameters.
    pub params: Value,
}

/// [`CanvasCache::placed_keys`]: a document's [`CacheKey::placed`] and the placed-document bitmap
/// generations ([`vectorcraft_render::placed_document::generation`]) it was checked at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedKey {
    /// The last generation the document was checked at.
    pub checked: u64,
    /// The document's [`CacheKey::placed`]: the generation of its first check, or of the last check
    /// that found a newly made bitmap the document may show.
    pub key: u64,
    /// The value before it last changed, and the last generation that value was checked at: no
    /// bitmap of a placed document the document shows was made between the two.
    pub before: (u64, u64),
}

/// [`CanvasCache::slices`]: the layout of the slices of (document uid, revision).
pub type SliceCache = ((u64, u64), std::sync::Arc<Vec<vectorcraft_doc::SliceArea>>);

/// [`CanvasCache::live_paint_gaps`]: the gap lines of the selected Live Paint groups of (document
/// uid, revision).
pub type LivePaintGapsCache = ((u64, u64), std::sync::Arc<Vec<vectorcraft_tools::Overlay>>);

/// [`CanvasCache::find_font`]: the document's fonts as `text.fonts` lists them, and whether
/// [`vectorcraft_engine::cmd::fontfiles::missing_fonts`] finds any (`None` until it is asked).
#[derive(Clone, Debug)]
pub struct FindFontCache {
    /// The document and the content and fonts the answers were worked out from.
    pub key: FontsKey,
    pub fonts: std::sync::Arc<Vec<serde_json::Value>>,
    pub missing: Option<bool>,
}

/// A document's uid, a version of its content (compared by identity) and the font database's
/// generation ([`vectorcraft_text::FontDb::generation`]). The weak reference keeps the version's
/// allocation, so a later version is never placed at the same address while the key is kept.
#[derive(Clone, Debug)]
pub struct FontsKey {
    pub uid: u64,
    pub doc: std::sync::Weak<vectorcraft_doc::Document>,
    pub generation: u64,
}

impl PartialEq for FontsKey {
    fn eq(&self, other: &Self) -> bool {
        self.uid == other.uid && self.generation == other.generation && std::sync::Weak::ptr_eq(&self.doc, &other.doc)
    }
}

/// [`CanvasCache::pixel_aligned`]: the selected objects' Align to Pixel Grid, on or off when they
/// agree and `None` when they differ, with whether every one can be aligned.
pub type PixelAligned = (Option<bool>, bool);

/// [`CanvasCache::symbol_instances`]: the symbol instances in the selection and its groups.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SymbolInstances {
    /// Each instance's id, in the order the selection's subtrees list them.
    pub ids: Vec<vectorcraft_doc::NodeId>,
    /// One of them has appearance changes of its own (`symbol_overrides`).
    pub changed: bool,
    /// One of them has a scale, rotation or shear that Reset Transformation would change; measured
    /// on the first read.
    pub transformed: Option<bool>,
}

/// [`CanvasCache::print_tiling`]: the print tiling of (document uid, revision).
pub type PrintTilingCache = ((u64, u64), std::sync::Arc<Vec<vectorcraft_pdf::TilingPage>>);

/// [`CanvasCache::shape_angle`]: the selected live shape's angle, for (document uid, revision, kind
/// of shape).
pub type ShapeAngleCache = ((u64, u64, std::mem::Discriminant<vectorcraft_doc::LiveShape>), Option<f64>);

/// [`CanvasCache::artboard_thumbs`]: the artboard thumbnails of (document uid, revision).
pub type ArtboardThumbsCache = ((u64, u64), Vec<Option<egui::TextureHandle>>);

/// [`CanvasCache::recently_added`]: the font generation, when it was asked, the families and the
/// newest install time among them.
pub type RecentlyAddedCache = (u64, SystemTime, Arc<[String]>, Option<SystemTime>);

/// [`CanvasCache::glyph_form_faces`]: the faces, for (document uid, revision, font generation, the
/// selection's object and range, the selected objects).
pub type GlyphFormFacesCache = ((u64, u64, u64, Option<(NodeId, usize, usize)>, Vec<NodeId>), Vec<u32>);

/// [`CanvasCache::character_quads`]: the quads and whether they are cut, for (document uid,
/// revision, text object, range).
pub type CharacterQuadsCache = ((u64, u64, NodeId, usize, usize), (Vec<[Point; 4]>, bool));

/// [`CanvasCache::tabs_guide_axis`]: the guide line, for (document uid, text object, paragraph,
/// revision).
pub type TabsGuideAxisCache = ((u64, NodeId, usize, u64), Option<(Point, Point, Vec2)>);

#[derive(Clone, Debug, PartialEq)]
pub struct CacheKey {
    pub doc: usize,
    pub revision: u64,
    /// An interaction is in progress ([`vectorcraft_render::RenderOptions::interacting`]).
    pub interacting: bool,
    /// A tool's drag on the canvas is in progress ([`vectorcraft_render::RenderOptions::dragging`]).
    pub dragging: bool,
    pub zoom: f64,
    pub cx: f64,
    pub cy: f64,
    pub w: u32,
    pub h: u32,
    pub outline: bool,
    pub trim: bool,
    /// View › Hide Template: template layers left out ([`vectorcraft_render::RenderOptions::hide_templates`]).
    pub templates: bool,
    /// View › Show/Hide Guides: whether guide paths are visible in the artwork raster.
    pub guides: bool,
    pub ppp: f32,
    pub hidden: Vec<u64>,
    pub rot: f64,
    /// General › Anti-aliased Artwork.
    pub anti_alias: bool,
    /// Type › Highlight Alternate Glyphs (#394).
    pub highlight_alternate_glyphs: bool,
    /// Type › Highlight Substituted Fonts.
    pub highlight_substituted_fonts: bool,
    /// A [`vectorcraft_render::placed_document::generation`] that changes when the frame may show a
    /// newly made bitmap of a placed document ([`canvas::raster::placed_key`]). Bitmaps of files that only
    /// other documents hold leave it as it is, unless more bitmaps were made since the last check
    /// than are recorded and the document shows a placed document.
    pub placed: u64,
    /// The plug-in revision ([`vectorcraft_render::effects::plugin_revision`]) of the last plug-in
    /// install or removal, or of the last time deferred plug-in effect stages gave results, after
    /// which the document drew a plug-in effect ([`vectorcraft_render::draws_plugin_effects`]): a
    /// frame drawn before it draws those effects as the plug-ins installed then did. 0 until then.
    pub plugins: u64,
    /// View › Pixel Preview: the document pixels rendered (x0, y0, x1, y1), one per point, shown
    /// with hard edges. None: the art is rendered for the screen.
    pub pixel: Option<[i64; 4]>,
    /// Images sampled smoothly ([`vectorcraft_render::RenderOptions::smooth_images`]).
    pub smooth_images: bool,
    /// Isolation mode's group or layer ([`vectorcraft_render::RenderOptions::isolated`]).
    pub isolated: Option<u64>,
    /// Appearance of Black › On Screen › Display All Blacks as Rich Black
    /// ([`vectorcraft_render::RenderOptions::rich_black`]).
    pub rich_black: bool,
    /// Colour and style of guide paths ([`vectorcraft_render::RenderOptions::guide`]).
    pub guide: ([u8; 3], bool),
    /// The Image Trace objects drawn with their source image
    /// ([`vectorcraft_render::RenderOptions::trace_source`]).
    pub trace_source: Option<std::sync::Arc<Vec<vectorcraft_doc::NodeId>>>,
    /// The display profile the frame is converted into ([`crate::display_profile::DisplayProfile::key`]),
    /// 0 for none.
    pub display: u64,
    /// The paper under the art that translucent art is blended with before the conversion
    /// ([`crate::display_profile::convert_over`]). None without a display profile, and in Outline.
    pub backdrop: Option<std::sync::Arc<crate::display_profile::Backdrop>>,
    /// The active color settings ([`vectorcraft_color::cms::active_settings`]), the working color
    /// settings ([`vectorcraft_color::cms::working_settings`]) and the loaded profiles' generation
    /// ([`vectorcraft_color::cms::profiles_generation`]) the frame's colors were converted with:
    /// Edit › Color Settings, a profile loaded again under its name or removed, and the front
    /// document's assigned profiles change them without changing the document. A proof of the
    /// working CMYK space converts through the working settings, which can change while the active
    /// settings stay the same.
    pub color: (vectorcraft_color::cms::ColorSettings, vectorcraft_color::cms::ColorSettings, u64),
}

impl CacheKey {
    /// [`Self::backdrop`] with the zoom its grid's cells are measured at, as
    /// [`crate::display_profile::convert_over`] takes it.
    pub(crate) fn over(&self) -> Option<(&crate::display_profile::Backdrop, f64)> {
        self.backdrop.as_deref().map(|b| (b, self.zoom))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Perf {
    pub frame_ms: f64,
    pub render_ms: f64,
    pub fps: f64,
}
