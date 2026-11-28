//! Serializable UI state (read and driven by the control channel).

use std::collections::{BTreeSet, HashMap, HashSet};

use craft::behavior::canvas::{CanvasView, ZoomScheme};
use craft::behavior::selection::Selection;
use craft::ui::canvas::ViewRequest;
use craft::ui::dialog::DialogId;
use craft::ui::shell::{RecentSort, RecentView, ScreenMode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use vectorcraft_geom::Point;

use crate::theme::Brightness;

/// A one-shot Image Trace Ignore Color sampler, bound to the unchanged selected source.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceIgnoreSampler {
    pub doc: u64,
    pub target: vectorcraft_doc::NodeId,
    pub revision: u64,
    pub back: String,
}

/// A Brush Options Key Color click, owned by its dialog and document until release or cancel.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushKeyColorPick {
    pub doc: u64,
    pub back: String,
    /// The Brush Options dialog the picked colour goes back to.
    pub dialog: DialogId,
}

/// How the canvas moves to a view the app set: eased from where it was, or at once.
#[derive(Clone, Copy, Debug)]
pub enum ViewMove {
    /// The canvas eases from this view to the new one (Animated Zoom).
    Ease(CanvasView),
    /// The canvas jumps to the new view.
    Jump,
}

/// Rearrange for selected artboards, confirmed and waiting for the click on the canvas that places
/// the grid's top-left: the document (`DocState::uid`) and the tool it was confirmed in, and the
/// `artboard.rearrange` parameters, `indices` included, with the `Artboard::id` of each listed
/// artboard (`ids`), so a placement whose artboards moved in the list or were removed is dropped.
/// `armed` is false until the canvas has seen one frame of it, so the Enter that confirmed the
/// dialog does not also place the artboards.
#[derive(Clone, Debug)]
pub struct RearrangePlacement {
    pub doc: u64,
    pub reverts: u64,
    pub tool: String,
    pub params: Value,
    pub ids: Vec<u32>,
    pub armed: bool,
}

/// Per-document view (camera).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct View {
    /// The zoom (1.0 = 100%): points of the screen per document point, counted before
    /// Preferences › User Interface › UI Scaling, which scales the interface and not the art.
    /// [`Self::canvas`] gives the zoom in the interface's points.
    pub zoom: f64,
    /// Document point at the centre of the canvas.
    pub center: Point,
    /// False until the view was fitted to the artboard on first display.
    pub fitted: bool,
    /// View rotation in degrees (Rotate View tool).
    pub rotation: f64,
    /// The artboard the status bar's navigator is on (an index): Fit Artboard in Window and Actual
    /// Size show it.
    #[serde(default)]
    pub artboard: usize,
    /// View › Outline: the document's art is drawn as outlines. Each document has its own, and a
    /// document opens in Preview.
    #[serde(default)]
    pub outline: bool,
    /// The density of the display the view was last shown on ([`crate::Services::screen_ppi`]),
    /// which its zoom percentage counts from with Display Print Size at 100% Zoom; `None` until
    /// the view was first shown ([`crate::VectorcraftApp::settle_screen_ppi`]).
    #[serde(skip)]
    pub(crate) shown_ppi: Option<crate::ScreenPpi>,
    /// The zoom a fit to the window, or the document's saved view, gave the view. While the view
    /// still has it, the first reading of its display leaves the zoom alone; a zoom chosen as a
    /// percentage keeps its percentage then ([`crate::VectorcraftApp::settle_screen_ppi`]).
    #[serde(skip)]
    pub(crate) fit_zoom: Option<f64>,
    /// The fit that gave [`Self::fit_zoom`] was made while the view's display was being read: the
    /// view counts from the last reading meanwhile, and the reading, when it comes, leaves the zoom
    /// alone while the view still has it.
    #[serde(skip)]
    pub(crate) fit_unread: bool,
    /// The canvas on screen, inside the rulers, as the last pass laid it out (screen points).
    #[serde(skip, default = "no_rect")]
    pub rect: egui::Rect,
    /// A view command or placement the canvas runs on its next pass, with the document area it
    /// fits when the command fits one.
    #[serde(skip)]
    pub request: Option<(ViewRequest, Option<vectorcraft_geom::Rect>)>,
    /// How the canvas reaches the view the app last set ([`Self::set_canvas`] callers).
    #[serde(skip)]
    pub moved: Option<ViewMove>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            center: Point::new(306.0, 396.0),
            fitted: false,
            rotation: 0.0,
            artboard: 0,
            outline: false,
            shown_ppi: None,
            fit_zoom: None,
            fit_unread: false,
            rect: egui::Rect::NOTHING,
            request: None,
            moved: None,
        }
    }
}

impl View {
    /// The view a document opens at: the one it was saved with, else fitted on first display, on
    /// the artboard the opened file names as its active one ([`vectorcraft_doc::Document::opened_artboard`]),
    /// else the first.
    pub fn of(st: &vectorcraft_engine::DocState) -> Self {
        let artboard = st.doc.opened_artboard.and_then(|id| st.doc.artboards.iter().position(|a| a.id == id)).unwrap_or(0);
        Self { artboard, ..st.view.as_ref().and_then(Self::saved).unwrap_or_default() }
    }

    /// The view `v` saved (the one a document was saved with, or a View → New View… one), within
    /// the zoom range; `None` when the canvas can't show it (a zoom or centre that isn't finite, a
    /// zoom of 0 or less).
    pub fn saved(v: &vectorcraft_engine::doc::SavedView) -> Option<Self> {
        let zoom = v.zoom.clamp(0.0313, 640.0);
        v.is_usable().then(|| Self {
            zoom,
            fit_zoom: Some(zoom),
            center: v.center,
            fitted: true,
            rotation: if v.rotation.is_finite() { v.rotation } else { 0.0 },
            ..Self::default()
        })
    }

    /// The view as the canvas shows it in `area` while the interface is zoomed by `ui_scaling`
    /// (Preferences › User Interface › UI Scaling): the zoom counts the interface's points, so a
    /// document point covers the same device pixels at every UI Scaling.
    pub fn canvas(&self, ui_scaling: f32, area: egui::Rect) -> CanvasView {
        CanvasView::new(area, self.center, self.zoom / f64::from(ui_scaling)).rotation(self.rotation)
    }

    /// Takes the view the canvas reports back, the inverse of [`Self::canvas`].
    pub fn set_canvas(&mut self, view: &CanvasView, ui_scaling: f32) {
        self.center = view.center;
        self.zoom = Self::zoom_of_canvas(view, ui_scaling);
        self.rotation = view.rotation;
        self.rect = view.rect;
    }

    /// The zoom [`Self::zoom`] holds for the canvas view `view` at `ui_scaling`, the inverse of the
    /// zoom in [`Self::canvas`]: the zoom `view.setZoom` and the View menu set and the status bar shows.
    pub fn zoom_of_canvas(view: &CanvasView, ui_scaling: f32) -> f64 {
        view.zoom * f64::from(ui_scaling)
    }
}

/// The status bar's angle menu and View › Rotate View list these preset view angles, each as
/// (label, degrees clockwise on screen).
pub const ROTATE_VIEW_ANGLES: [(&str, f64); 12] = [
    ("0°", 0.0),
    ("15°", 15.0),
    ("30°", 30.0),
    ("45°", 45.0),
    ("60°", 60.0),
    ("90°", 90.0),
    ("180°", 180.0),
    ("-15°", -15.0),
    ("-30°", -30.0),
    ("-45°", -45.0),
    ("-60°", -60.0),
    ("-90°", -90.0),
];

/// Screen points in an inch when the system reports no physical size for the display: the
/// reference density that display scaling is set against (a CSS inch).
pub const REFERENCE_PPI: f64 = 96.0;

/// The pixel densities a measured display may have, in pixels per inch, before the display scale
/// divides them into screen points. A size outside them is a wrong report (a projector or
/// television that gives its aspect ratio as its size, or none).
pub const SCREEN_PPI_RANGE: std::ops::RangeInclusive<f64> = 30.0..=1000.0;

/// The screen points one inch takes at 100% zoom: `None` with Preferences › General › Display
/// Print Size at 100% Zoom off, else the measured density of the display (`screen_ppi`,
/// [`crate::Services::screen_ppi`]), or [`REFERENCE_PPI`] when the display has none. A measured
/// density is taken when it is finite and positive: the host checks the display's pixels against
/// [`SCREEN_PPI_RANGE`] before the display scale divides them into points, so a point density can
/// fall outside that range.
pub fn print_ppi(display_print_size: bool, screen_ppi: Option<f64>) -> Option<f64> {
    display_print_size.then(|| screen_ppi.filter(|p| p.is_finite() && *p > 0.0).unwrap_or(REFERENCE_PPI))
}

/// Document zoom at 100% in the status bar: one screen point per document point, or with
/// Preferences › General › Display Print Size at 100% Zoom ([`print_ppi`] `Some`), one document
/// inch (72 pt) in an inch of the screen, `print` screen points.
pub fn zoom_baseline(print: Option<f64>) -> f64 {
    print.map_or(1.0, |ppi| ppi / 72.0)
}

/// The zoom stops with 100% at [`zoom_baseline`]: the one scheme the status bar, the zoom
/// commands and the canvas step through.
fn zoom_stops(print: Option<f64>) -> ZoomScheme {
    ZoomScheme::ILLUSTRATOR.with_hundred(zoom_baseline(print))
}

/// The zoom `z` as the status bar shows it, in percent of [`zoom_baseline`].
pub fn zoom_display_percent(z: f64, print: Option<f64>) -> f64 {
    zoom_stops(print).percent(z)
}

/// The zoom the status bar's `pct` stands for.
pub fn zoom_from_display_percent(pct: f64, print: Option<f64>) -> f64 {
    zoom_stops(print).device(pct)
}

/// The zoom stop after `z`, up or down, within the zoom range. The range has a fixed floor, so
/// the document's size does not enter. The stops count from [`zoom_baseline`]`(print)`; past the
/// last stop in that direction, the zoom stays where it is.
pub fn next_zoom(z: f64, up: bool, print: Option<f64>) -> f64 {
    let stop = zoom_stops(print).step(z, if up { 1 } else { -1 }, vectorcraft_geom::Size::ZERO).clamp(0.0313, 640.0);
    if up { stop.max(z) } else { stop.min(z) }
}

/// Format for the tab/zoom field: up to 2 decimals, no trailing zeros.
pub fn zoom_label(z: f64, print: Option<f64>) -> String {
    let s = format!("{:.2}", zoom_display_percent(z, print));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}%")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockTab {
    #[default]
    Properties,
    Layers,
    Libraries,
}

impl DockTab {
    /// The tab's panel id (`window.panel`), English label and icon (when the dock is collapsed).
    pub fn info(self) -> (&'static str, &'static str, &'static str) {
        match self {
            DockTab::Properties => ("properties", "Properties", "dc-options"),
            DockTab::Layers => ("layers", "Layers", "layers"),
            DockTab::Libraries => ("libraries", "Libraries", "library"),
        }
    }

    pub const ALL: [DockTab; 3] = [DockTab::Properties, DockTab::Layers, DockTab::Libraries];

    /// The tab whose panel id is `id`.
    pub fn from_id(id: &str) -> Option<DockTab> {
        DockTab::ALL.into_iter().find(|t| t.info().0 == id)
    }
}

/// Every panel `window.panel` shows, as (id, English label): the dock tabs, then the icon panels.
pub fn all_panels() -> impl Iterator<Item = (&'static str, &'static str)> {
    let tabs = DockTab::ALL.into_iter().map(|t| (t.info().0, t.info().1));
    tabs.chain(ICON_PANELS.iter().map(|&(id, label, _)| (id, label)))
}

/// Panel `id`'s id, English label and icon if it can live in the icon column: an icon panel, or
/// one of the tabbed group's panels dropped there.
pub fn rail_panel(id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    ICON_PANELS.iter().find(|entry| entry.0 == id).copied().or_else(|| DockTab::from_id(id).map(DockTab::info))
}

/// Panels that live as collapsed icons in the dock (Essentials Classic).
pub const ICON_PANELS: &[(&str, &str, &str)] = &[
    ("color", "Color", "palette"),
    ("colorGuide", "Color Guide", "dc-color-guide"),
    ("swatches", "Swatches", "swatch-book"),
    ("brushes", "Brushes", "paintbrush"),
    ("symbols", "Symbols", "spray-can"),
    ("patternOptions", "Pattern Options", "grid-3x3"),
    ("stroke", "Stroke", "dc-stroke"),
    ("gradient", "Gradient", "dc-gradient"),
    ("transparency", "Transparency", "dc-transparency"),
    ("appearance", "Appearance", "dc-appearance"),
    ("graphicStyles", "Graphic Styles", "dc-graphic-styles"),
    ("artboards", "Artboards", "dc-artboards"),
    ("transform", "Transform", "dc-transform-panel"),
    ("align", "Align", "dc-align"),
    ("pathfinder", "Pathfinder", "dc-pathfinder"),
    (crate::panels::blend::ID, "Blend", "dc-blend"),
    ("character", "Character", "type"),
    ("paragraph", "Paragraph", "pilcrow"),
    ("glyphs", "Glyphs", "text-cursor-input"),
    ("charStyles", "Character Styles", "type"),
    ("openType", "OpenType", "dc-touch-type"),
    ("paraStyles", "Paragraph Styles", "pilcrow"),
    ("tabs", "Tabs", "align-horizontal-justify-start"),
    ("history", "History", "history"),
    ("actions", "Actions", "dc-actions"),
    ("info", "Info", "info"),
    ("docInfo", "Document Info", "dc-list-view"),
    ("navigator", "Navigator", "map"),
    ("separations", "Separations Preview", "printer"),
    ("imageTrace", "Image Trace", "image"),
    ("magicWand", "Magic Wand", "wand-sparkles"),
    (crate::panels::flattener_preview::ID, "Flattener Preview", "eye"),
    ("attributes", "Attributes", "settings"),
    (crate::panels::svg_interactivity::ID, "SVG Interactivity", "spline-pointer"),
    ("colorThemes", "Color Themes", "sun"),
    (crate::panels::color_wheel::ID, "Color Wheel", "circle"),
    (crate::panels::links::ID, "Links", "link"),
    (crate::panels::preflight::ID, "Production Preflight", "dc-list-view"),
    (crate::panels::asset_export::ID, "Asset Export", "share-2"),
    (crate::panels::css_properties::ID, "CSS Properties", "globe"),
    (crate::panels::three_d::ID, "3D and Materials", "dc-cube"),
    (crate::panels::variables::ID, "Variables", "chart-column"),
    (crate::panels::script_console::ID, "Script Console", "message-circle"),
    (crate::panels::learn::ID, "Learn", "graduation-cap"),
    (crate::panels::discover::ID, "VectorCraft Help…", "search"),
    (crate::panels::retype::ID, "Retype", "type"),
    (crate::addons::cleanup::ID, "Artwork Cleanup", "sparkles"),
    (crate::addons::color::ID, "Color Effects", "sun"),
    (crate::addons::packaging::ID, "Fold Lines", "scissors"),
    (crate::addons::packaging::preview::ID, "Fold Preview", "dc-cube"),
    (crate::addons::pathtools::ID, "Anchor Inspector", "dc-anchor-smooth"),
    (crate::addons::pathtools::shape::ID, "Parametric Shape", "dc-parametric-shape"),
    (crate::addons::pen::ID, "Smart Pen", "spline-pointer"),
    (crate::addons::units::ID, "Unit Converter", "ruler"),
];

/// Groups of icon panels separated by dividers in the collapsed column.
///
/// This, not [`ICON_PANELS`], is what the collapsed column draws: a panel listed there but in
/// no group can be opened from the Window menu and `window.panel`, but never appears as an
/// icon. A panel that belongs in the column needs a row here as well as its entry above.
pub const ICON_PANEL_GROUPS: &[&[&str]] = &[
    &["color", "colorGuide"],
    &["swatches", "brushes", "symbols", "patternOptions"],
    &["stroke", "gradient", "transparency"],
    &["appearance", "graphicStyles"],
    &["artboards"],
    &["transform", "align", "pathfinder"],
    &["character", "paragraph", "glyphs"],
    &["history", "actions", "info", "navigator"],
    &["variables"],
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewFlags {
    pub rulers: bool,
    /// View › Rulers › Show Video Rulers: while the rulers show, they count the active artboard in
    /// video pixels, honouring its Video Ruler Pixel Aspect Ratio.
    #[serde(default)]
    pub video_rulers: bool,
    pub grid: bool,
    pub guides: bool,
    pub smart_guides: bool,
    pub bounding_box: bool,
    pub pixel_preview: bool,
    pub snap_to_grid: bool,
    pub snap_to_point: bool,
    pub artboards: bool,
    pub edges: bool,
    /// View → Trim View: hide everything outside the artboards.
    pub trim_view: bool,
    /// View → Hide/Show Corner Widget (live corner widgets on rectangles).
    pub corner_widgets: bool,
    /// The Control bar's Hide/Show Shape Widgets with a shape tool: selected live shapes' corner, pie
    /// and side widgets with the Selection tool and the shape tools.
    #[serde(default = "yes")]
    pub shape_widgets: bool,
    /// View → Snap to Pixel.
    pub snap_to_pixel: bool,
    /// View → Snap to Glyph.
    #[serde(default)]
    pub snap_to_glyph: bool,
    /// View → Show/Hide Text Threads.
    pub text_threads: bool,
    /// View → Show/Hide Gradient Annotator.
    pub gradient_annotator: bool,
    /// View → Show/Hide Live Paint Gaps: draw the gaps of the selected Live Paint groups in their Gap
    /// Preview Color.
    #[serde(default)]
    pub live_paint_gaps: bool,
    /// Type → Show Hidden Characters.
    pub hidden_chars: bool,
    /// Edit › Spelling › Auto Spell Check: a red underline under the words of the type its
    /// dictionaries don't know.
    #[serde(default)]
    pub auto_spell_check: bool,
    /// View → Hide/Show Template: template layers drawn (dimmed) or left out of the canvas.
    #[serde(default = "yes")]
    pub templates: bool,
}

impl Default for ViewFlags {
    fn default() -> Self {
        Self {
            rulers: false,
            video_rulers: false,
            grid: false,
            guides: true,
            smart_guides: true,
            bounding_box: true,
            pixel_preview: false,
            snap_to_grid: false,
            snap_to_point: true,
            artboards: true,
            edges: true,
            trim_view: false,
            corner_widgets: true,
            shape_widgets: true,
            snap_to_pixel: false,
            snap_to_glyph: false,
            text_threads: true,
            gradient_annotator: true,
            live_paint_gaps: false,
            hidden_chars: false,
            auto_spell_check: false,
            templates: true,
        }
    }
}

impl ViewFlags {
    /// Whether the ruler guides and guide paths draw: View › Show Guides, unless Trim View hides
    /// them while it is on (#8363). `guides` keeps the choice, so they come back as they were.
    pub fn guides_shown(&self) -> bool {
        self.guides && !self.trim_view
    }

    /// Whether the grid draws: View › Show Grid, unless Trim View hides it while it is on (#8363).
    pub fn grid_shown(&self) -> bool {
        self.grid && !self.trim_view
    }
}

/// The last Replace Font request whose warning the status bar shows, to refresh it as the
/// document changes.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplaceFontWarning {
    pub document_uid: u64,
    pub revision: u64,
    pub request: serde_json::Value,
    pub selection: Option<Vec<u64>>,
    pub text: String,
    pub styles: Vec<(String, String)>,
}

/// A misspelled word right-clicked on the canvas ([`UiState::context_spelling`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ContextSpelling {
    /// The document's `uid`.
    pub uid: u64,
    /// The type object.
    pub id: vectorcraft_doc::NodeId,
    /// The word's bytes in the object's plain text, which `text.editRange` replaces.
    pub start: usize,
    pub end: usize,
    pub word: String,
    /// What the dictionary of the word's language suggests instead, best first.
    pub suggestions: Vec<String>,
}

/// Two states are equal when they save the same: the settings store writes only a changed saved form,
/// and the panels' session state is no change to save.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// The interface language older versions saved here (`ja`, `cs`…). It now lives in the
    /// `interfaceLanguage` preference, which [`crate::prefs_dialog::restore`] carries it over to;
    /// it is never written back.
    #[serde(rename = "language", skip_serializing)]
    pub legacy_language: Option<String>,
    pub brightness: Brightness,
    /// The panel arrangement: docks, floating windows, closed panels and where each goes back. One
    /// `Arc` the dock area keeps across frames; writers change it through `Arc::make_mut`.
    #[serde(rename = "dock")]
    pub dock_workspace: std::sync::Arc<craft::behavior::dock::Workspace<String>>,
    /// The tool each slot of the Tools panel showed last.
    pub tools_last_used: craft::ui::rails::LastUsed<String>,
    /// By a tool rail's panel: the strips torn off it that closed when it was hidden, which come
    /// back when it shows again ([`crate::toolbar::strips_follow`]).
    #[serde(default)]
    pub strips_hidden_with: std::collections::BTreeMap<String, Vec<String>>,
    /// The saved workspaces, the current one and the changes made in each since it was saved.
    pub workspaces: crate::workspaces::Workspaces,
    /// The panel fields of files from before the shared dock: read once to build
    /// [`Self::dock_workspace`] ([`crate::layout_migration::migrate_ui`]), never written.
    #[serde(flatten)]
    pub legacy: LegacyDock,
    /// Icon panel currently popped out of the collapsed column (also `properties`, `layers` or
    /// `libraries` while the dock is collapsed).
    pub open_panel: Option<String>,
    /// The panel a row's link opened as a popup (the Appearance panel's Stroke:, Character:,
    /// Opacity:…): it shows beside the Appearance panel at the clicked row until a press outside
    /// it, Escape, or the panel showing elsewhere puts it away.
    #[serde(skip)]
    pub transient_panel: Option<String>,
    /// The Appearance panel's rows and selection, kept while the app runs.
    #[serde(skip)]
    pub(crate) appearance_panel: crate::panels::appearance::AppearancePanel,
    /// The Stroke panel's state, kept while the app runs.
    #[serde(skip)]
    pub(crate) stroke_panel: crate::panels::stroke::StrokePanel,
    /// The Transparency panel's state, kept while the app runs.
    #[serde(skip)]
    pub(crate) transparency_panel: crate::panels::transparency::TransparencyPanel,
    /// The Graphic Styles panel's state, kept while the app runs.
    #[serde(skip)]
    // PLAN: assumed `crate::panels::graphic_styles::GraphicStylesPanel` (ap.AS17 names no type; appearance-w10).
    pub(crate) graphic_styles_panel: crate::panels::graphic_styles::GraphicStylesPanel,
    /// The symbols the Symbols panel has picked (Select All Unused, a click), by name, with the uid
    /// of the document they were picked in.
    #[serde(skip)]
    pub symbol_picks: (u64, Vec<String>),
    /// The Brushes panel's multi-selection and the click it belongs to: another document, other
    /// objects or another current brush ends it.
    #[serde(skip)]
    pub brush_selection: Option<(crate::panels::brushes::ClickKey, Selection<String>)>,
    /// The Brushes panel's view choices, kept while the app runs.
    #[serde(skip)]
    pub(crate) brushes_view: crate::panels::brushes::BrushesView,
    /// The Brush Options dialog that is open, while it is.
    #[serde(skip)]
    pub brush_options: Option<DialogId>,
    /// The selection of each open library panel, by panel id: its items and folders. Not saved.
    #[serde(skip)]
    pub(crate) library_selection: HashMap<String, Selection<crate::panels::library_panel::LibraryKey>>,
    /// The Libraries panel lists its items (on) instead of showing icons (off).
    #[serde(skip)]
    pub libraries_list_view: bool,
    /// The Libraries panel's groups that are closed, as (library id, group name).
    #[serde(skip)]
    pub library_groups_closed: HashSet<(String, String)>,
    /// The Libraries panel's search text.
    #[serde(skip)]
    pub library_search: String,
    /// The Separations Preview panel is still to scroll its Color Settings section into view:
    /// Edit › Color Settings… and Assign Profile… show the panel for it, and the section sits below
    /// the plates and Proof Setup, out of sight in a short window (#6419).
    #[serde(skip)]
    pub color_settings_scroll: bool,
    /// Type › Find Font…'s last Find In choice (`find_font::FIND_IN`), kept while the app runs.
    #[serde(skip)]
    pub find_font_in: String,
    /// Ellipse Properties' Constrain Pie Angle link (Transform and Properties panels): a Pie Start
    /// or End Angle typed while it is on keeps each selected ellipse's pie span. Kept while the app
    /// runs; off at every launch.
    #[serde(skip)]
    pub constrain_pie_angle: bool,
    /// The Learn panel's tab, skill and tutorial, kept while the app runs.
    #[serde(skip)]
    pub learn: crate::panels::learn::View,
    #[serde(skip)]
    pub discover: crate::panels::discover::View,
    /// The Retype panel's list of matching fonts and its sample settings, kept while the app runs.
    #[serde(skip)]
    pub retype: crate::panels::retype::View,
    /// The tool whose place a Learn tutorial step's link shows (`learn.locateTool`), while its mark
    /// is on screen. Not saved.
    #[serde(skip)]
    pub learn_locate: Option<crate::panels::learn::locate::Locate>,
    /// The tutorials whose practice samples were opened, kept while the app runs.
    #[serde(skip)]
    pub learn_practice: crate::panels::learn::Practice,
    /// The Learn panel's tutorials finished and steps marked done, saved with the preferences.
    pub learn_progress: crate::panels::learn::progress::Progress,
    pub control_bar: bool,
    /// The Control bar is docked to the bottom of the window instead of the top
    /// (`window.control.dock`).
    pub control_bar_bottom: bool,
    /// The Control bar floats over the window at `control_bar_float_pos` instead of docking to an
    /// edge (`window.control.dock {edge: "float"}`); `control_bar_bottom` keeps the edge it last
    /// docked to.
    #[serde(default)]
    pub control_bar_floating: bool,
    /// The top-left corner of the floating Control bar in window points, kept while the bar is
    /// docked; none: the bar's first floating place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_bar_float_pos: Option<[f32; 2]>,
    /// The groups of the Control bar's options (`chrome::CONTROL_BAR_GROUPS`) that its panel menu
    /// has off (`window.control.options`); none: every group shows.
    #[serde(deserialize_with = "crate::chrome::read_hidden_groups")]
    pub control_bar_hidden_groups: BTreeSet<String>,
    /// Smaller cells and panel width for the main Tools panel, independent of its column count.
    #[serde(default)]
    pub toolbar_compact: bool,
    /// Advanced toolbar (every tool group) instead of the categorized Basic toolbar.
    #[serde(default)]
    pub toolbar_advanced: bool,
    #[serde(default = "yes")]
    pub task_bar: bool,
    /// Where the Contextual Task Bar was dragged or pinned, and the document it was moved in. Not
    /// saved: the bar starts under the selection at every launch.
    #[serde(skip)]
    pub task_bar_place: crate::canvas::bar_place::KeyedPlace,
    /// The Contextual Task Bar's actions (`canvas::TASK_BAR_ACTIONS`) that More Options › Bar
    /// Actions has off (`window.taskBar.actions`); none: the bar shows every action it offers.
    #[serde(deserialize_with = "crate::canvas::read_hidden_task_bar_actions")]
    pub task_bar_hidden_actions: BTreeSet<String>,
    /// Window › Quick Pen Toolbar: the Quick Pen tool shows its toolbar on the canvas. Hide Toolbar
    /// turns it off until the tool is selected again. Not saved.
    #[serde(skip, default = "yes")]
    pub quick_pen_bar: bool,
    /// Where the Quick Pen toolbar was dragged or pinned, and the document and kind of place it
    /// was moved in. Not saved: the toolbar starts beside the drawing at every launch.
    #[serde(skip)]
    pub quick_pen_bar_place: crate::canvas::bar_place::KeyedPlace,
    /// The Pathfinder panel's Shape Mode buttons make a compound shape on a click, and give the
    /// permanent Pathfinder result on Alt-click (off, the default: the other way round).
    #[serde(default)]
    pub shape_modes_compound: bool,
    /// The locked objects under the pointer at the last right-click on the canvas, with the
    /// document's `uid`: the context menu's Unlock submenu lists them while the menu is open. Not
    /// saved.
    #[serde(skip)]
    pub context_locked: Option<(u64, Vec<vectorcraft_doc::NodeId>)>,
    /// What the last right-click on the canvas with the Intertwine tool arranges, with the
    /// document's `uid`: the Intertwine group, the point and the object nearest to it where three
    /// or more objects overlap ([`vectorcraft_tools::intertwine::menu_target`]). The context menu
    /// lists Bring to Front, Bring Forward, Send Backward and Send to Back for that object while it
    /// is open. Not saved.
    #[serde(skip)]
    pub context_intertwine: Option<(u64, vectorcraft_doc::NodeId, vectorcraft_geom::Point, vectorcraft_doc::NodeId)>,
    /// What the last right-click on the canvas with the Artboard tool measured, for the context menu
    /// while it is open ([`crate::panels::artboards::MenuFacts`]). Not saved.
    #[serde(skip)]
    pub context_artboards: Option<crate::panels::artboards::MenuFacts>,
    /// The word with Auto Spell Check's red line under the last right-click on the canvas, for the
    /// context menu's suggestions, Ignore All and Add to Dictionary while it is open. Not saved.
    #[serde(skip)]
    pub context_spelling: Option<ContextSpelling>,
    /// Which link statuses the active document's images and placed documents have, for Select ›
    /// Linked, Embedded, Missing and Modified to be on only when some do (refreshed by
    /// [`crate::addons::select::link_status::refresh`]). Not saved.
    #[serde(skip)]
    pub link_statuses: crate::addons::select::link_status::Present,
    /// The Basic and Advanced toolbars as Edit Toolbar left them (none: as they ship).
    #[serde(default)]
    pub toolbar_edits: ToolbarEdits,
    /// The controls under the toolbar's tools that the All Tools drawer's Show section shows
    /// (`toolbar.show`).
    #[serde(default)]
    pub toolbar_show: ToolbarShow,
    /// Edit Toolbar's All Tools drawer is open beside the toolbar (`window.editToolbar`).
    #[serde(skip)]
    pub tools_drawer: bool,
    /// The toolbars made with New Toolbar, in the order they were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_toolbars: Vec<CustomToolbar>,
    /// The toolbar made with New Toolbar that the open All Tools drawer edits (none: the Tools
    /// panel's toolbar).
    #[serde(skip)]
    pub drawer_toolbar: Option<String>,
    /// Tabs closed together by panel shortcuts, kept with their workspace return locations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docking_hidden_groups: Vec<Vec<String>>,
    pub status_bar: bool,
    /// What the status bar's info field shows (its Show menu, `view.statusBarShow`): Current Tool by
    /// default and in state saved without it ([`crate::chrome::deserialize_status_bar_show`]).
    #[serde(default, deserialize_with = "crate::chrome::deserialize_status_bar_show")]
    pub status_bar_show: crate::chrome::StatusBarShow,
    pub help_bar: bool,
    pub view: ViewFlags,
    #[serde(skip)]
    pub brush_key_color_pick: Option<BrushKeyColorPick>,
    /// Rearrange for selected artboards waiting for its click on the canvas
    /// (`dialogs::rearrange_artboards`).
    #[serde(skip)]
    pub rearrange_placement: Option<RearrangePlacement>,
    /// A control request is being answered: its caller gets what fails as its reply, so no
    /// dialog says it too (`io::report_write_error`).
    #[serde(skip)]
    pub answering_control: bool,
    /// Last tool shown for each toolbar group (flyout selection sticks).
    pub group_tool: Vec<String>,
    /// The drawing tool last chosen in this window, which a two-finger tap in the Touch workspace
    /// returns to from the Selection tool ([`crate::touch::toggle_mode`]). Not saved.
    #[serde(skip)]
    pub touch_drawing_tool: Option<String>,
    pub status: String,
    /// How many times [`crate::VectorcraftApp::status`] has set `status`, so that work finishing
    /// later can tell whether the line it started with is still the latest one. Not saved.
    #[serde(skip)]
    pub status_sets: u64,
    /// The Replace Font request behind the status bar's warning ([`ReplaceFontWarning`]). Not saved.
    #[serde(skip)]
    pub replace_font_warning: Option<ReplaceFontWarning>,
    /// The last Find and Replace search ([`crate::dialogs::find_replace`]): the dialog's Find and
    /// Replace With text, its options and the place the search reached (the walk, `found`), which
    /// Edit › Find Next goes on from while the dialog is closed. Not saved.
    #[serde(skip)]
    pub last_find: Option<serde_json::Value>,
    /// The notes of the last open that gave any, or of the last save or export, with the status
    /// text that names the first of them: hovering that text lists them when there are several.
    /// Not saved.
    #[serde(skip)]
    pub status_notes: Option<(String, Vec<String>)>,
    /// What the startup scripts' round under way left out ([`crate::script_runs`]): the runs that
    /// could not start and the notes on the Startup Scripts folder, shown again when its last run
    /// ends. Not saved.
    #[serde(skip)]
    pub startup_notes: Vec<String>,
    /// The status text a plug-in wrote, with the app's words before and after it: while the status
    /// is these three together, the status bar translates the app's words and shows the plug-in's
    /// as they are ([`crate::plugin_runs::show_notice`]). Not saved.
    #[serde(skip)]
    pub status_kept: Option<(String, String, String)>,
    pub palette_open: bool,
    pub palette_query: String,
    /// Set when the palette's shortcut, its command or the search icon opens or closes the palette,
    /// so that the palette starts at its first row the next time it is drawn, even when it closed
    /// and opened again before then. Not saved.
    #[serde(skip)]
    pub palette_toggled: bool,
    /// Screen mode: 0 normal, 1 full screen with menu, 2 full screen, 3 presentation
    /// ([`Self::screen_mode`] maps it to the shell's mode). Not saved: the app always starts in
    /// Normal Screen Mode, with its menus and panels (#472: a saved Presentation Mode came back on
    /// restart with no way out).
    #[serde(skip)]
    pub screen_mode: u8,
    /// Window › Arrange › Tile: the open documents share the document area, one pane each, until
    /// Consolidate All Windows. Not saved: the reference app does not save windows with a document.
    #[serde(skip)]
    pub tiled: bool,
    /// How Tile lays the panes out: in a grid, side by side, stacked, or 2-up to 6-up. Not saved,
    /// as `tiled` is not.
    #[serde(skip)]
    pub tile_layout: crate::canvas::TileLayout,
    /// Window › Arrange › Float in Window, Float All in Windows and Cascade: the documents in
    /// floating windows of their own, out of the tab strip, until Consolidate All Windows. Not
    /// saved, as `tiled` is not.
    #[serde(skip)]
    pub floating: Vec<crate::doc_windows::DocWindow>,
    /// The window (its document's uid and its view's number) the tab strip shows while the current
    /// window is a floating one: the tabbed window that was current last.
    #[serde(skip)]
    pub tab_document: Option<(u64, u32)>,
    /// Draw Normal / Behind / Inside.
    pub draw_mode: u8,
    /// The Home screen's recent files as cards or a list. Not saved.
    #[serde(skip)]
    pub home_view: RecentView,
    /// The Home screen's recent files' order. Not saved.
    #[serde(skip)]
    pub home_sort: RecentSort,
    /// Recorded actions (Actions panel), persisted with the UI preferences under their old key.
    #[serde(rename = "action_sets", default = "crate::panels::actions::default_list", with = "crate::panels::actions::prefs")]
    pub actions: craft::automation::actions::ActionList,
    /// The current set of the Actions panel by name (its name clicked, one of its actions clicked,
    /// or made with New Set): a recording goes into it and Set Options renames it. An index saved
    /// by older versions is turned into the name when the file is read
    /// ([`crate::settings::parse_legacy`]).
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::panels::actions::deserialize_current_set")]
    pub action_set: Option<String>,
    /// The Actions panel's view: its selection, open rows and Button Mode. Button Mode is kept
    /// with the UI preferences under its old key; the rest lasts the session.
    #[serde(rename = "action_button_mode", default, with = "crate::panels::actions::prefs::button_mode")]
    pub actions_panel: craft::automation::ui::actions::ActionsPanelState,
    /// The Script Console's selected entries. Not saved.
    #[serde(skip)]
    pub script_console_selection: Selection<u64>,
    /// Recording in progress: (set index, action name, journal length when recording started).
    #[serde(skip)]
    pub recording: Option<(usize, String, usize)>,
    /// The name of the action the recording in progress records into, the one New Action made or
    /// the one selected when Begin Recording started it: Stop gives it the recorded steps. None
    /// when Stop adds a new action, as after Begin Recording with no action selected.
    #[serde(skip)]
    pub recording_into: Option<String>,
    /// Where in the action [`UiState::recording_into`] names Stop puts the recorded steps: before
    /// the step at this index, or None after its last step.
    #[serde(skip)]
    pub recording_at: Option<usize>,
    /// The length of the action set file that Stop Playing/Recording would leave without the steps
    /// recorded so far, once measured; None until then, and again after an app command, which may
    /// change the sets.
    #[serde(skip)]
    pub recording_base: Option<usize>,
    /// The bytes that the steps recorded so far add to that file, at most, as last counted: (the
    /// journal's rewrites, the recording's first entry, the journal's length, the bytes)
    /// ([`vectorcraft_engine::Journal::rewrites`]).
    #[serde(skip)]
    pub recording_tail: Option<(u64, usize, usize, usize)>,
    /// The journal as the recording in progress last checked it against the limits of an action
    /// set file: (its rewrites, its length) ([`crate::panels::actions::after_journaled_edit`]).
    #[serde(skip)]
    pub recording_checked: Option<(u64, usize)>,
    /// How actions play (Actions panel menu › Playback Options…), kept with the UI preferences.
    #[serde(default, deserialize_with = "crate::panels::actions::deserialize_playback")]
    pub action_playback: crate::panels::actions::PlaybackOptions,
    /// The playback at Step by Step or Pause For in progress, if any. Not saved.
    #[serde(skip)]
    pub action_playing: Option<crate::panels::actions::Playing>,
    /// The keyboard shortcut sets and which one is active (Edit › Keyboard Shortcuts).
    #[serde(default, with = "crate::keymap::stored::profiles")]
    pub shortcut_sets: craft::keymap::Profiles,
    /// The shortcut keys of files from before [`Self::shortcut_sets`]: read once into it
    /// ([`Self::sanitized`]), never written.
    #[serde(flatten)]
    pub legacy_shortcuts: LegacyShortcuts,
    /// Workspace before Preferences › Devices › Enable Touch Workspace switched to Touch (#394).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touch_workspace_restore: Option<String>,
    /// File → Open Recent Files, most recent first.
    #[serde(default)]
    pub recent_files: Vec<String>,
    /// The folder each kind of file pick chose last, by the pick's purpose: the next pick of that
    /// kind starts there (the desktop host's file service).
    #[serde(default)]
    pub last_folders: HashMap<String, std::path::PathBuf>,
    /// Type → Recent Fonts, most recent first.
    #[serde(default)]
    pub recent_fonts: Vec<String>,
    /// Help › Search Commands' Recent Commands list: the last picks, most recent first.
    #[serde(default, deserialize_with = "deserialize_recent_commands")]
    pub recent_commands: Vec<craft::keymap::CommandRef>,
    /// How many times each command was picked in Help › Search Commands, which ranks the commands
    /// picked most often first among results that match as well.
    #[serde(default, deserialize_with = "crate::palette::deserialize_command_uses")]
    pub command_uses: crate::palette::CommandUses,
    /// Families starred in the font menus (the ★ filter shows only these).
    #[serde(default)]
    pub favorite_fonts: Vec<String>,
    /// Engine preferences (Edit → Preferences), persisted alongside the UI state.
    #[serde(default)]
    pub engine_prefs: Value,
    /// Native-picker-only raster editor; never exposed as an engine preference or dialog field.
    #[serde(default)]
    pub original_editor: crate::OriginalEditorChoice,
    /// Color Guide panel: variation kind, steps and amount (Color Guide Options).
    #[serde(default)]
    pub color_guide: vectorcraft_color::harmony::GuideOptions,
    /// The library panels' settings (Window → Swatch Libraries, Graphic Style Libraries): of each
    /// library opened, and of the persistent ones, which reopen at startup, also once closed
    /// ([`crate::panels::library_panel`], `ui.libraryPanel`).
    #[serde(default)]
    pub library_panels: Vec<crate::panels::library_panel::LibraryPanel>,
    /// Flattener Preview panel: highlight, overprints, preset and options (`ui.flattenerPreview`).
    #[serde(default)]
    pub flattener_preview: crate::panels::flattener_preview::Settings,
    /// Fold Preview window: fold amount, view, zoom, perspective, inside color, fold lines and
    /// base panel (`ui.foldPreview`).
    #[serde(default)]
    pub fold_preview: crate::addons::packaging::preview::Settings,
    /// Color Guide panel: Limit to Library, a swatch library id, "document" (the document's
    /// swatches) or "" (none) (`ui.colorGuideLimit`).
    #[serde(default)]
    pub color_guide_limit: String,
    /// Color Guide panel: the base color, set by a color chosen in the Color panel, the Swatches
    /// panel or the Color Picker and by the panel's base chip (none: the current color). Not
    /// saved.
    #[serde(skip)]
    pub color_guide_base: Option<vectorcraft_color::Color>,
    /// Color Guide panel: the spot color swatch the base is a tint of, when it was chosen from one
    /// ([`crate::panels::color_guide::SpotBase`]). Not saved.
    #[serde(skip)]
    pub color_guide_spot: Option<crate::panels::color_guide::SpotBase>,
    /// Align panel: the Align To choice, Selection or Artboard, which Object → Align and the align
    /// buttons follow ([`crate::panels::align::with_align_to`]).
    #[serde(default, deserialize_with = "crate::panels::align::deserialize_align_to")]
    pub align_to: crate::panels::align::AlignTo,
    /// Align panel: the Distribute Spacing value in points, which the Distribute Objects and
    /// Distribute Spacing buttons and their commands send while there is a key object or a key
    /// guide ([`crate::panels::align::run_button`]). Not saved.
    #[serde(skip)]
    pub align_spacing: f64,
    /// Color panel: its layout, ramp, Hide Options, Show Recent Colors and RGB in percent
    /// (`ui.colorPanel`); a value that does not read gives the defaults
    /// ([`crate::panels::color::deserialize_panel_state`]).
    #[serde(default, deserialize_with = "crate::panels::color::deserialize_panel_state")]
    pub color_panel: craft::color_ui::ui::ColorPanelState,
    /// The Color Wheel panel's session state. Not saved.
    #[serde(skip)]
    pub color_wheel_panel: crate::panels::color_wheel::WheelUi,
    /// The Color Themes panel's session state. Not saved.
    #[serde(skip)]
    pub color_themes_panel: crate::panels::color_themes::ThemesUi,
    /// The Color Guide panel's session state. Not saved.
    #[serde(skip)]
    pub color_guide_panel: crate::panels::color_guide::Model,
    /// The Gradient panel's session state. Not saved.
    #[serde(skip)]
    pub gradient_panel: crate::panels::gradient::GradientUi,
    /// The Swatches panel's selection, kept swatch and focus. Not saved.
    #[serde(skip)]
    pub(crate) swatches_panel: crate::panels::swatches::Model,
    /// The open Swatch Options dialog's record. Not saved.
    #[serde(skip)]
    pub(crate) swatch_options: Option<crate::dialogs::swatch_options::Open>,
    /// The open New Swatch dialog's record. Not saved.
    #[serde(skip)]
    pub(crate) new_swatch: Option<crate::dialogs::new_swatch::Open>,
    /// Color Wheel panel: the wheel or the box, the box's channel, the slider rows, the
    /// out-of-gamut shading, Show Opacity, Show Contrast, Contrast For and Contrast Background, and
    /// the color vision row (`ui.colorWheel`, [`crate::panels::color_wheel::deserialize_settings`]).
    #[serde(default, deserialize_with = "crate::panels::color_wheel::deserialize_settings")]
    pub color_wheel: crate::panels::color_wheel::Settings,
    /// Swatches panel: Show Recent Colors in its panel menu, which draws the panel's Recent Colors
    /// row (#6617). Separate from the Color panel's.
    #[serde(default = "yes", deserialize_with = "crate::panels::color::deserialize_show_recent_colors")]
    pub swatches_show_recent_colors: bool,
    /// The SVG Options chosen last (`svg` object of the export/save commands; null: never used).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub svg_options: Value,
    /// File → Place: Link is on (the Place dialog remembers it).
    #[serde(default = "yes")]
    pub place_link: bool,
    /// File → Place: Show Import Options is on (the Place dialog remembers it). Each placed file
    /// with import options (a PDF's page and box, a DXF drawing, a text file) asks for them first;
    /// off, it is placed with the options last confirmed for its format.
    #[serde(default = "yes")]
    pub place_import_options: bool,
    /// The Text Import Options confirmed last (`file.place`'s `text` options; null: never used).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub text_import: Value,
    /// The documents (uids) whose Missing Fonts dialog waits until no dialog is open, the next one to
    /// show first.
    #[serde(skip)]
    pub pending_fonts: Vec<u64>,
    /// The plug-ins the last launch was loading when it stopped (`{item, items}`, each
    /// `{id, file, name, label}`, `item` the first of `items`), while the user has not chosen what
    /// to do about them.
    #[serde(skip)]
    pub stopped_plugin: Option<Value>,
    /// The open Proof Setup dialog's record: the proof view it found and the one its preview
    /// shows on the canvas.
    #[serde(skip)]
    pub(crate) proof_setup: Option<crate::dialogs::proof_setup::Open>,
    /// The search the Missing Fonts dialog started (`text.findFontFiles`'s `id`), stopped once the
    /// dialog is gone.
    #[serde(skip)]
    pub dialog_search: Option<u64>,
    /// The search the Find Linked Files dialog started (`links.findFiles`'s `id`), stopped once the
    /// dialog is gone.
    #[serde(skip)]
    pub links_search: Option<u64>,
    /// The DXF Options chosen last (`document.exportDxf` options; null: never used).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub dxf_options: Value,
    /// The EPS Options chosen last (`document.exportEps` options; null: never used).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub eps_options: Value,
    /// The HP-GL Options chosen last (`document.export` HP-GL options; null: never used).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub hpgl_options: Value,
    /// The Recolor Artwork Preserve checkboxes confirmed last (`preserveWhite`, `preserveBlack`,
    /// `preserveGrays`; null: never used): its dialog opens with them (#2960).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub recolor_preserve: Value,
    /// The spacing used the last time Rearrange All Artboards was confirmed.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub rearrange_artboards_spacing: Value,
    /// The DXF Import Options chosen last (fit, scaleLineweights, center, mergeLayers; null:
    /// never used).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub dxf_import: Value,
    /// The PSD Import Options chosen last (layers, hiddenLayers, importSlices; null: never used).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub psd_import: Value,
    /// The Home screen is shown over the open documents (`app.home`): the active document's uid
    /// and the document count when it opened. Choosing a tab, or a document opening, closing or
    /// becoming active, leaves it.
    #[serde(skip)]
    pub home: Option<(Option<u64>, usize)>,
    /// The Image Trace objects the canvas draws with the Source Image view while the Image Trace
    /// panel's eye button is held down ([`vectorcraft_render::RenderOptions::trace_source`]).
    #[serde(skip)]
    pub trace_hold_source: Option<std::sync::Arc<Vec<vectorcraft_doc::NodeId>>>,
    /// Ignore Color's one-shot canvas sampler; independent of the eye button's press lifetime.
    #[serde(skip)]
    pub trace_ignore_sampler: Option<TraceIgnoreSampler>,
    /// Layers panel › Panel Options… (row size, thumbnails, Show Layers Only, Locate Selected Objects).
    #[serde(default)]
    pub layers_panel: crate::panels::layers::PanelOptions,
    /// Last AI layer naming settings other than the shared Ollama connection (scope and depth).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub layer_naming_settings: Value,
    /// Ollama server, vision model and name language, shared by the AI dialogs.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub ollama_settings: Value,
    /// Navigator panel › Panel Options… (the view box color, Draw Dashed Lines as Solid Lines) and
    /// the panel menu's View Artboard Contents Only.
    #[serde(default)]
    pub navigator_panel: crate::panels::navigator::PanelOptions,
    /// Attributes panel › Panel Options… (how many recent URLs the URL menu lists).
    #[serde(default)]
    pub attributes_panel: crate::panels::attributes::PanelOptions,
    /// The Layers panel's session state: its Filter (`ui.layersFilter`), selection and open rows.
    /// It is not saved and lasts the session, as the Search All text does.
    #[serde(skip)]
    pub layers_ui: crate::panels::layers::LayersUi,
    /// The Artboards panel's session state. Not saved.
    #[serde(skip)]
    pub artboards_ui: crate::panels::artboards::ArtboardsUi,
    /// The Info panel's Hide Options. Not saved.
    #[serde(skip)]
    pub info_hide_options: bool,
    /// The Document Info panel's view. Not saved.
    #[serde(skip)]
    pub doc_info: crate::panels::doc_info::DocInfoView,
    /// The Links panel's options, selection and scroll. Not saved.
    #[serde(skip)]
    pub(crate) links: crate::panels::links::LinksModel,
    /// The Swatches, Graphic Styles and Symbols panels' views, chosen in their panel menus
    /// (`ui.panelView`). They are not saved and last the session.
    #[serde(skip)]
    pub swatches_view: crate::panels::swatches::View,
    #[serde(skip)]
    pub graphic_styles_view: crate::panels::graphic_styles::View,
    #[serde(skip)]
    pub symbols_view: crate::panels::symbols::View,
    /// The Image Trace panel's Advanced section is open (as it was last left).
    #[serde(default = "yes")]
    pub image_trace_advanced: bool,
    /// The Image Trace panel's Preview: a setting changed in the panel traces the selected Image
    /// Trace object again at once. Off, the changes wait in the panel until Trace applies them.
    #[serde(default = "yes")]
    pub image_trace_preview: bool,
    /// The Glyphs panel's category, zoom and recent glyphs.
    #[serde(default)]
    pub glyphs_panel: craft::font_manager::glyphs::GlyphsPanelSettings,
    /// The family and style the Glyphs panel shows when it does not follow the selection. Not saved.
    #[serde(skip)]
    pub glyphs_font: Option<(String, String)>,
    /// The glyph the Glyphs panel marked last. Not saved.
    #[serde(skip)]
    pub glyphs_marked: Option<craft::font_manager::glyphs::GlyphCell>,
    /// The font menus' filters (class, favourites, similar, script). Not saved.
    #[serde(skip)]
    pub font_filters: craft::font_manager::picker::FontFilters,
    /// The Character panel's Hide Options. Not saved.
    #[serde(skip)]
    pub character_hide_options: bool,
    /// The Character panel's font height reference. Not saved.
    #[serde(skip)]
    pub character_font_height: bool,
    /// The Snap to Glyph options show in the Character panel. Not saved.
    #[serde(skip)]
    pub snap_to_glyph_options: bool,
    /// The Snap to Glyph options are still to scroll into view. Not saved.
    #[serde(skip)]
    pub snap_to_glyph_reveal: bool,
    /// The Paragraph panel's Hide Options. Not saved.
    #[serde(skip)]
    pub paragraph_hide_options: bool,
    /// The Tabs panel's session state. Not saved.
    #[serde(skip)]
    pub(crate) tabs_panel: crate::panels::tabs::TabsPanel,
    /// The Character and Paragraph Styles panels' session state. Not saved.
    #[serde(skip)]
    pub(crate) text_styles: crate::panels::text_styles::StylesPanels,
    /// The Properties panel's open sections. Not saved.
    #[serde(skip)]
    pub properties_panel: crate::panels::properties::PropertiesUi,
    /// The Transform panel's reference point and popover. Not saved.
    #[serde(skip)]
    pub transform_panel: crate::panels::transform::TransformUi,
    /// The panel parts a panel menu's Hide Options turned off. Not saved.
    #[serde(skip)]
    pub hidden_parts: BTreeSet<&'static str>,
    /// The open Fill/Stroke chip popover of the Control bar or the Properties panel. Not saved.
    #[serde(skip)]
    pub paint_popover: Option<crate::panels::PaintPopover>,
    /// The Image Trace panel's shown settings and preset picker. Not saved.
    #[serde(skip)]
    pub(crate) image_trace_panel: crate::panels::image_trace::ImageTraceUi,
    /// The Asset Export panel's selection, applied preset and Save As name, per document. Not saved.
    #[serde(skip)]
    pub(crate) asset_export_panel: crate::panels::asset_export::AssetExportPanel,
    /// The Pattern Options panel's chosen pattern. Not saved.
    #[serde(skip)]
    pub(crate) pattern_options_panel: crate::panels::pattern_options::PatternOptionsPanel,
    /// The SVG Interactivity panel's chosen event. Not saved.
    #[serde(skip)]
    pub(crate) svg_interactivity_panel: crate::panels::svg_interactivity::SvgInteractivityPanel,
    /// The CSS Properties panel's options. Not saved.
    #[serde(skip)]
    pub(crate) css_properties_panel: crate::panels::css_properties::CssPropertiesPanel,
    /// The Production Preflight panel's options and recheck cache. Not saved.
    #[serde(skip)]
    pub(crate) preflight_panel: crate::panels::preflight::PreflightPanel,
    /// The press count of the last press that was no click (an Image Trace pick took it, or it dragged), which offsets
    /// the presses of its run. Not saved.
    #[serde(skip)]
    pub trace_pick_clicks: u32,
    /// This frame ended a stale Image Trace pick while the primary button went down: the pick takes that press. Not saved.
    #[serde(skip)]
    pub(crate) trace_pick_stale: bool,
    /// Where the canvas's context menu opened (screen points), while it is open. Not saved.
    #[serde(skip)]
    pub context_menu_at: Option<egui::Pos2>,
}

/// The panel fields older files saved before the shared dock: read through aliases (a workspace
/// record spells them in camelCase) and never written. [`crate::layout_migration::migrate_ui`]
/// turns them into [`UiState::dock_workspace`] and [`UiState::tools_last_used`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacyDock {
    /// The arrangement once the canvas joined the dock.
    #[serde(skip_serializing)]
    pub docking: Option<craft::behavior::dock::Layout<String>>,
    /// Closed panels and floating panels, with the place each goes back to.
    #[serde(skip_serializing, alias = "dockingHidden")]
    pub docking_hidden: std::collections::BTreeMap<String, craft::behavior::dock::Location<String>>,
    /// Panels dropped on the icon column.
    #[serde(skip_serializing, alias = "dockingIcons")]
    pub docking_icons: BTreeSet<String>,
    /// The panels shown by dock groups collapsed to their tabs.
    #[serde(skip_serializing, alias = "dockingCollapsed")]
    pub docking_collapsed: BTreeSet<String>,
    /// Floating panel groups from before the canvas joined the dock.
    #[serde(skip_serializing, alias = "floatingPanels")]
    pub floating_panels: Vec<FloatingPanels>,
    /// The tab in front in the dock's tabbed group.
    #[serde(skip_serializing, alias = "dockTab")]
    pub dock_tab: DockTab,
    /// Tab order in the dock's tabbed group when it differs from [`DockTab::ALL`].
    #[serde(skip_serializing, alias = "dockTabOrder")]
    pub dock_tab_order: Vec<DockTab>,
    /// The dock's tabbed group was collapsed to icons.
    #[serde(skip_serializing, alias = "dockCollapsed")]
    pub dock_collapsed: bool,
    /// The main dock column's width in screen points.
    #[serde(skip_serializing, alias = "dockWidth")]
    pub dock_width: Option<f32>,
    /// The Tools panel floated with its top-left corner here.
    #[serde(skip_serializing, alias = "toolbarPos")]
    pub toolbar_pos: Option<[f32; 2]>,
    /// The docked Tools panel stood on the right.
    #[serde(skip_serializing, alias = "toolbarRight")]
    pub toolbar_right: bool,
    /// The docked Tools panel stood between the left-side panels and the canvas.
    #[serde(skip_serializing, alias = "toolsBesideCanvas")]
    pub tools_beside_canvas: bool,
    /// The Tools panel showed two columns.
    #[serde(skip_serializing, alias = "toolbarDouble")]
    pub toolbar_double: bool,
    /// Tool flyouts torn off the toolbar into floating strips.
    #[serde(skip_serializing, alias = "floatingFlyouts")]
    pub floating_flyouts: Vec<FloatingFlyout>,
    /// The tool each toolbar slot showed last, keyed by the slot's first tool id.
    #[serde(skip_serializing, alias = "slotTool")]
    pub slot_tool: std::collections::BTreeMap<String, String>,
}

/// The shortcut keys older files saved: the overrides, the set's name and the hold shortcuts. Read
/// once into [`UiState::shortcut_sets`] and never written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacyShortcuts {
    /// Command id, `tool:<id>` or `panel:<id>` → chord text, as earlier versions wrote them.
    #[serde(rename = "shortcut_overrides", skip_serializing)]
    pub overrides: Value,
    /// The shortcut set's name.
    #[serde(rename = "shortcut_set", skip_serializing)]
    pub set: Option<String>,
    /// Hold shortcuts: command id, `tool:<id>` or `panel:<id>` → a character key.
    #[serde(rename = "shortcut_holds", skip_serializing)]
    pub holds: Value,
}

impl LegacyShortcuts {
    /// Whether the file had none of the old keys.
    fn is_empty(&self) -> bool {
        self.overrides.is_null() && self.set.is_none() && self.holds.is_null()
    }
}

/// Reads the Recent Commands list: a command with its parameters, or a command id as older
/// versions saved it. Search Commands itself is never listed.
fn deserialize_recent_commands<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<craft::keymap::CommandRef>, D::Error> {
    let values = Vec::<Value>::deserialize(d)?;
    Ok(values
        .into_iter()
        .filter_map(|value| match value {
            Value::String(id) => Some(crate::keymap::command_of(&id)),
            other => serde_json::from_value(other).ok(),
        })
        .filter(|command| command.id != "help.commandPalette")
        .collect())
}

/// A tool group's flyout torn off the toolbar: it floats as its own panel until its × puts it back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloatingFlyout {
    /// The group's tools in flyout order; the first one names the toolbar slot.
    pub tools: Vec<String>,
    /// Top-left corner in screen points.
    pub pos: [f32; 2],
}

impl FloatingFlyout {
    /// Keep strips of known tools, one per group (a hand-edited preferences file).
    pub fn sanitize(strips: &mut Vec<FloatingFlyout>) {
        let mut seen = std::collections::BTreeSet::new();
        strips.retain_mut(|f| {
            f.tools.retain(|id| vectorcraft_tools::tool_info(id).is_some());
            f.tools.first().is_some_and(|k| seen.insert(k.clone()))
        });
    }
}

/// A toolbar made with Window › Toolbars › New Toolbar (`window.toolbar.new`), filled with tools
/// from the All Tools drawer and kept with the workspace it was made in. Where it shows is the
/// dock's: its panel is `toolbar:<id>` in [`UiState::dock_workspace`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomToolbar {
    /// Names its panel (`toolbar:<id>`); 0 in files saved before toolbars had ids, until the
    /// conversion gives it one.
    #[serde(default)]
    pub id: u32,
    /// The name it is listed under in Window › Toolbars.
    pub name: String,
    /// Its slots, each a tool group (the first tool is the slot's default).
    #[serde(default)]
    pub slots: Vec<Vec<String>>,
    /// The tool each slot showed last.
    #[serde(default)]
    pub last_used: craft::ui::rails::LastUsed<String>,
    /// Older files: it showed. Read by the conversion, never written.
    #[serde(default = "yes", skip_serializing)]
    pub shown: bool,
    /// Older files: its top-left corner while it floated. Read by the conversion, never written.
    #[serde(default, skip_serializing)]
    pub pos: [f32; 2],
    /// Older files: its place in the stack of toolbars docked along an edge. Read by the
    /// conversion, never written.
    #[serde(default, skip_serializing)]
    pub dock: Option<u32>,
    /// Older files: it docked along the right edge. Read by the conversion, never written.
    #[serde(default, skip_serializing)]
    pub dock_right: bool,
    /// Older files: the tool each slot showed, keyed by the slot's first tool. Read by the
    /// conversion, never written.
    #[serde(default, skip_serializing)]
    pub slot_tool: std::collections::BTreeMap<String, String>,
    /// Older files: its tools showed in two columns. Read by the conversion, never written.
    #[serde(default, skip_serializing)]
    pub double: bool,
    /// Older files: the groups torn off it. Read by the conversion, never written.
    #[serde(default, skip_serializing)]
    pub floating: Vec<FloatingFlyout>,
    /// Its command buttons, after its tools: Command Palette result ids (a command's id, or
    /// `plugin:`, `effect:` or `panel:` and an id). A command that is gone, such as an uninstalled
    /// plug-in's, is kept and shows as unavailable until it comes back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

/// A category of an edited toolbar: its name (one of the Basic toolbar's; none in the Advanced
/// toolbar) and its slots, each a tool group (the first tool is the slot's default).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolbarCategory {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slots: Vec<Vec<String>>,
}

/// The controls under the toolbar's tools, each shown or hidden from the All Tools drawer's Show
/// section: Fill and Stroke (the proxy), Coloring (Color, Gradient, None), Draw Mode and Screen
/// Mode. All shown as it ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolbarShow {
    #[serde(default = "yes")]
    pub fill_stroke: bool,
    #[serde(default = "yes")]
    pub coloring: bool,
    #[serde(default = "yes")]
    pub draw_mode: bool,
    #[serde(default = "yes")]
    pub screen_mode: bool,
}

impl Default for ToolbarShow {
    fn default() -> Self {
        Self { fill_stroke: true, coloring: true, draw_mode: true, screen_mode: true }
    }
}

impl ToolbarShow {
    /// The controls as (`toolbar.show` name, label, icon), in the Show section's order.
    pub const CONTROLS: [(&'static str, &'static str, &'static str); 4] = [
        ("fillStroke", "Fill and Stroke Controls", "dc-stroke"),
        ("coloring", "Coloring Controls", "dc-gradient"),
        ("drawMode", "Draw Mode Controls", "dc-draw-normal"),
        ("screenMode", "Screen Mode Controls", "dc-screen-mode"),
    ];

    /// The flag of the control named `name`.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut bool> {
        match name {
            "fillStroke" => Some(&mut self.fill_stroke),
            "coloring" => Some(&mut self.coloring),
            "drawMode" => Some(&mut self.draw_mode),
            "screenMode" => Some(&mut self.screen_mode),
            _ => None,
        }
    }

    pub fn get(self, name: &str) -> bool {
        let mut s = self;
        s.get_mut(name).is_some_and(|on| *on)
    }

    pub fn json(self) -> serde_json::Value {
        serde_json::json!({"fillStroke": self.fill_stroke, "coloring": self.coloring, "drawMode": self.draw_mode, "screenMode": self.screen_mode})
    }
}

/// The toolbars Edit Toolbar changed: each one is `None` while it is as it ships.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolbarEdits {
    #[serde(default)]
    pub basic: Option<Vec<ToolbarCategory>>,
    #[serde(default)]
    pub advanced: Option<Vec<ToolbarCategory>>,
}

/// A group of panels dragged out of the dock, as files from before the canvas joined the panel
/// layout saved it: it floated as a stack of tabs, one panel shown.
///
/// Groups sharing a `column` floated as one set, stacked top to bottom in list order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FloatingPanels {
    /// Panel ids (`window.panel`), in tab order.
    pub panels: Vec<String>,
    /// The tab shown.
    #[serde(default)]
    pub active: usize,
    /// Top-left corner in screen points.
    pub pos: [f32; 2],
    /// A width the group was given (the Tabs panel sized over its text), else its panels' own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// The set this group is stacked in with others (the same number), top to bottom in list
    /// order; none when it floats on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Collapsed to its tabs (a double-click on a tab), its panel hidden.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    /// Docked as a column of the dock (with the rest of its set) instead of floating: the columns
    /// stand left of the icon column, 1 next to it, counting left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docked: Option<u32>,
}

/// Drop a floating position of the Control bar that isn't a number (a hand-edited preferences
/// file or workspace).
pub fn sanitize_toolbar_pos(toolbar_pos: &mut Option<[f32; 2]>) {
    if toolbar_pos.is_some_and(|p| !p.iter().all(|v| v.is_finite())) {
        *toolbar_pos = None;
    }
}

impl UiState {
    /// Clear transient state after loading saved preferences.
    pub fn sanitized(mut self) -> Self {
        self.tools_drawer = false;
        self.drawer_toolbar = None;
        crate::custom_toolbars::sanitize(&mut self.custom_toolbars);
        self.palette_open = false;
        self.status.clear();
        self.open_panel = None;
        if self.group_tool.len() != vectorcraft_tools::TOOL_GROUPS.len() {
            self.group_tool = UiState::default().group_tool;
        }
        sanitize_toolbar_pos(&mut self.control_bar_float_pos);
        crate::edit_toolbar::sanitize(&mut self.toolbar_edits);
        // `read_preferences` converted the file's older layouts already; this covers a state read
        // any other way. What it reports lost was dropped there.
        let _ = crate::layout_migration::migrate_ui(&mut self);
        crate::panel_docking::reconcile(&mut self.dock_workspace);
        self.workspaces.sanitize();
        // A file from before the shortcut sets: its overrides, set name and holds become the sets.
        let legacy = std::mem::take(&mut self.legacy_shortcuts);
        if !legacy.is_empty() {
            self.shortcut_sets = crate::keymap::stored::legacy(&legacy.overrides, legacy.set.as_deref(), &legacy.holds);
        }
        self
    }

    /// The shell's screen mode for [`Self::screen_mode`]'s number: Presentation (3) is full screen
    /// with the reveal off, which the frame passes to the shell beside this.
    pub fn screen_mode(&self) -> ScreenMode {
        match self.screen_mode {
            0 => ScreenMode::Standard,
            1 => ScreenMode::FullScreenWithMenuBar,
            _ => ScreenMode::FullScreen,
        }
    }

    /// Whether the screen mode is Presentation Mode: the body only, and Tab reveals nothing.
    pub fn presenting(&self) -> bool {
        self.screen_mode >= 3
    }

    /// Takes the mode the shell moved to (its screen-mode commands, a reveal's end); a move also
    /// leaves Presentation Mode, which the shell knows only as full screen.
    pub fn follow_screen_mode(&mut self, mode: ScreenMode) {
        self.screen_mode = match mode {
            ScreenMode::Standard => 0,
            ScreenMode::FullScreenWithMenuBar => 1,
            _ => 2,
        };
    }
}

impl PartialEq for UiState {
    fn eq(&self, other: &Self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(other).ok()
    }
}

impl Default for UiState {
    /// The interface of a first start.
    fn default() -> Self {
        Self {
            legacy_language: None,
            brightness: Brightness::MediumDark,
            dock_workspace: std::sync::Arc::new(crate::panel_docking::default_workspace(&[])),
            tools_last_used: Default::default(),
            strips_hidden_with: Default::default(),
            workspaces: Default::default(),
            legacy: LegacyDock::default(),
            open_panel: None,
            transient_panel: None,
            appearance_panel: Default::default(),
            stroke_panel: Default::default(),
            transparency_panel: Default::default(),
            graphic_styles_panel: Default::default(),
            symbol_picks: (0, Vec::new()),
            brush_selection: None,
            brushes_view: Default::default(),
            brush_options: None,
            library_selection: HashMap::new(),
            libraries_list_view: false,
            library_groups_closed: HashSet::new(),
            library_search: String::new(),
            color_settings_scroll: false,
            find_font_in: String::new(),
            constrain_pie_angle: false,
            learn: Default::default(),
            discover: Default::default(),
            retype: Default::default(),
            learn_locate: None,
            learn_practice: Default::default(),
            learn_progress: Default::default(),
            control_bar: false,
            control_bar_bottom: false,
            control_bar_floating: false,
            control_bar_float_pos: None,
            control_bar_hidden_groups: Default::default(),
            toolbar_compact: false,
            toolbar_advanced: false,
            task_bar: true,
            task_bar_place: Default::default(),
            task_bar_hidden_actions: Default::default(),
            quick_pen_bar: true,
            quick_pen_bar_place: Default::default(),
            shape_modes_compound: false,
            context_locked: None,
            context_intertwine: None,
            context_artboards: None,
            context_spelling: None,
            link_statuses: Default::default(),
            toolbar_edits: ToolbarEdits::default(),
            toolbar_show: ToolbarShow::default(),
            tools_drawer: false,
            custom_toolbars: vec![],
            drawer_toolbar: None,
            docking_hidden_groups: Vec::new(),
            status_bar: true,
            status_bar_show: crate::chrome::StatusBarShow::CurrentTool,
            help_bar: true,
            view: ViewFlags::default(),
            brush_key_color_pick: None,
            rearrange_placement: None,
            answering_control: false,
            group_tool: vectorcraft_tools::TOOL_GROUPS.iter().map(|g| g[0].id.to_string()).collect(),
            touch_drawing_tool: None,
            status: String::new(),
            status_sets: 0,
            replace_font_warning: None,
            last_find: None,
            status_notes: None,
            startup_notes: Vec::new(),
            status_kept: None,
            palette_open: false,
            palette_query: String::new(),
            palette_toggled: false,
            screen_mode: 0,
            tiled: false,
            tile_layout: crate::canvas::TileLayout::Grid,
            floating: Vec::new(),
            tab_document: None,
            draw_mode: 0,
            home_view: RecentView::default(),
            home_sort: RecentSort::default(),
            actions: crate::panels::actions::default_list(),
            action_set: None,
            actions_panel: Default::default(),
            script_console_selection: Selection::default(),
            recording: None,
            recording_into: None,
            recording_at: None,
            recording_base: None,
            recording_tail: None,
            recording_checked: None,
            action_playback: Default::default(),
            action_playing: None,
            shortcut_sets: Default::default(),
            legacy_shortcuts: LegacyShortcuts::default(),
            touch_workspace_restore: None,
            recent_files: vec![],
            last_folders: HashMap::new(),
            recent_fonts: vec![],
            recent_commands: vec![],
            command_uses: Default::default(),
            favorite_fonts: vec![],
            engine_prefs: Value::Null,
            original_editor: Default::default(),
            color_guide: Default::default(),
            library_panels: vec![],
            flattener_preview: Default::default(),
            fold_preview: Default::default(),
            color_guide_limit: String::new(),
            color_guide_base: None,
            color_guide_spot: None,
            align_to: Default::default(),
            align_spacing: 0.0,
            color_panel: Default::default(),
            color_wheel_panel: Default::default(),
            color_themes_panel: Default::default(),
            color_guide_panel: Default::default(),
            gradient_panel: Default::default(),
            swatches_panel: Default::default(),
            swatch_options: None,
            new_swatch: None,
            color_wheel: Default::default(),
            swatches_show_recent_colors: true,
            svg_options: Value::Null,
            place_link: true,
            place_import_options: true,
            text_import: Value::Null,
            pending_fonts: vec![],
            stopped_plugin: None,
            proof_setup: None,
            dialog_search: None,
            links_search: None,
            dxf_options: Value::Null,
            eps_options: Value::Null,
            hpgl_options: Value::Null,
            recolor_preserve: Value::Null,
            rearrange_artboards_spacing: Value::Null,
            dxf_import: Value::Null,
            psd_import: Value::Null,
            home: None,
            trace_hold_source: None,
            trace_ignore_sampler: None,
            layers_panel: Default::default(),
            layer_naming_settings: Value::Null,
            ollama_settings: Value::Null,
            navigator_panel: Default::default(),
            attributes_panel: Default::default(),
            layers_ui: Default::default(),
            artboards_ui: Default::default(),
            info_hide_options: false,
            doc_info: Default::default(),
            links: Default::default(),
            swatches_view: Default::default(),
            graphic_styles_view: Default::default(),
            symbols_view: Default::default(),
            image_trace_advanced: true,
            image_trace_preview: true,
            glyphs_panel: Default::default(),
            glyphs_font: None,
            glyphs_marked: None,
            font_filters: Default::default(),
            character_hide_options: false,
            character_font_height: false,
            snap_to_glyph_options: false,
            snap_to_glyph_reveal: false,
            paragraph_hide_options: false,
            tabs_panel: Default::default(),
            text_styles: Default::default(),
            properties_panel: Default::default(),
            transform_panel: Default::default(),
            hidden_parts: BTreeSet::new(),
            paint_popover: None,
            image_trace_panel: Default::default(),
            asset_export_panel: Default::default(),
            pattern_options_panel: Default::default(),
            svg_interactivity_panel: Default::default(),
            css_properties_panel: Default::default(),
            preflight_panel: Default::default(),
            trace_pick_clicks: 0,
            trace_pick_stale: false,
            context_menu_at: None,
        }
    }
}

fn yes() -> bool {
    true
}

fn no_rect() -> egui::Rect {
    egui::Rect::NOTHING
}

/// Seconds between two in-line looks at the system clipboard for `VectorcraftApp::system_paste`,
/// where no probe thread looks for the app.
pub(crate) const SYSTEM_CLIPBOARD_POLL: f64 = 0.25;

/// How often the font catalog looks for fonts a font manager activated while the app stays in
/// front (#2576).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const FONT_INSTALL_POLL: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;

    /// storytold/vectorcraft#2719: 100% in the zoom field matches Actual Size when Display Print Size is on.
    #[test]
    fn zoom_percent_honors_display_print_size_baseline() {
        let print = zoom_baseline(print_ppi(true, None));
        assert!(print > zoom_baseline(None), "an inch in 96 points zooms in past one point per point");
        for display_print_size in [None, print_ppi(true, None)] {
            let baseline = zoom_baseline(display_print_size);
            assert!((zoom_display_percent(baseline, display_print_size) - 100.0).abs() < 1e-9, "{display_print_size:?}");
            assert!((zoom_from_display_percent(100.0, display_print_size) - baseline).abs() < 1e-9, "{display_print_size:?}");
            let up = next_zoom(baseline, true, display_print_size);
            let down = next_zoom(baseline, false, display_print_size);
            assert!(up > baseline && down < baseline, "{display_print_size:?}: {down} < {baseline} < {up}");
            assert!(zoom_display_percent(up, display_print_size) > 100.0, "{display_print_size:?}");
        }
    }

    /// #9051: with Display Print Size on, 100% follows the measured density of the display, so an
    /// inch of the document takes as many screen points as an inch of that display holds. A
    /// density that is not finite and positive counts as no measurement.
    #[test]
    fn print_size_follows_the_displays_measured_density() {
        assert_eq!(print_ppi(false, Some(127.0)), None, "the preference off ignores the display");
        let (dense, sparse) = (print_ppi(true, Some(127.0)), print_ppi(true, Some(91.0)));
        assert!(zoom_baseline(dense) > zoom_baseline(print_ppi(true, None)), "a denser display than the reference zooms 100% further in");
        assert!(zoom_baseline(sparse) < zoom_baseline(print_ppi(true, None)), "a sparser one zooms it out");
        assert!((zoom_baseline(dense) * 72.0 - 127.0).abs() < 1e-9, "an inch of the document fills an inch of the display");
        for wrong in [Some(0.0), Some(-5.0), Some(f64::NAN), Some(f64::INFINITY), Some(f64::NEG_INFINITY), None] {
            assert_eq!(print_ppi(true, wrong), print_ppi(true, None), "{wrong:?} is no measurement");
        }
        assert_eq!(print_ppi(true, Some(23.15)), Some(23.15), "a display at a large scale factor has few points to an inch");
        assert_eq!(print_ppi(true, Some(1600.0)), Some(1600.0), "and one at a scale below 1 many");
        assert_eq!(zoom_label(zoom_from_display_percent(100.0, dense), dense), "100%");
        let thin = print_ppi(true, Some(40.0));
        assert!(next_zoom(0.0313, false, thin) >= 0.0313, "the last stop down stays in the zoom range");
        assert!(next_zoom(600.0, true, print_ppi(true, Some(900.0))) <= 640.0, "and the last stop up");
        let reference = print_ppi(true, None);
        assert!(next_zoom(0.0313, false, reference) <= 0.0313, "past the last stop, zooming out does not zoom in");
        assert!(next_zoom(640.0, true, thin) >= 640.0, "nor zooming in out");
    }

    #[test]
    fn a_document_opens_on_the_artboard_its_file_names_as_active() {
        let mut doc = vectorcraft_doc::Document::new(100.0, 100.0);
        let first = doc.artboards[0].clone();
        for id in [2, 3] {
            let x0 = f64::from(id - 1) * 150.0;
            doc.artboards.push(vectorcraft_doc::Artboard { id, rect: vectorcraft_geom::Rect::new(x0, 0.0, x0 + 100.0, 100.0), ..first.clone() });
        }
        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        for (opened, active) in [(Some(3), 2), (None, 0), (Some(9), 0)] {
            let mut d = doc.clone();
            d.opened_artboard = opened;
            app.session.add_document(d, None);
            app.sync_views();
            assert_eq!(app.view().map(|v| v.artboard), Some(active), "{opened:?}");
        }
        // With the view the document was saved with too.
        doc.opened_artboard = Some(2);
        doc.last_view = Some(vectorcraft_doc::SavedView { name: String::new(), center: Point::new(400.0, 50.0), zoom: 2.0, rotation: 0.0 });
        app.session.add_document(doc, None);
        app.sync_views();
        assert_eq!(app.view().map(|v| (v.artboard, v.zoom)), Some((1, 2.0)));
    }
}
