//! Serializable UI state (read and driven by the control channel).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vectorcraft_geom::Point;

use crate::theme::Brightness;

/// Per-document view (camera).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct View {
    /// Screen points per document point (1.0 = 100%).
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
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: 1.0, center: Point::new(306.0, 396.0), fitted: false, rotation: 0.0, artboard: 0 }
    }
}

impl View {
    /// The view a document opens at: the one it was saved with, else fitted on first display.
    pub fn of(st: &vectorcraft_engine::DocState) -> Self {
        st.view.as_ref().and_then(Self::saved).unwrap_or_default()
    }

    /// The view `v` saved (the one a document was saved with, or a View → New View… one), within
    /// the zoom range; `None` when the canvas can't show it (a zoom or centre that isn't finite, a
    /// zoom of 0 or less).
    pub fn saved(v: &vectorcraft_engine::doc::SavedView) -> Option<Self> {
        v.is_usable().then(|| Self {
            zoom: v.zoom.clamp(0.0313, 640.0),
            center: v.center,
            fitted: true,
            rotation: if v.rotation.is_finite() { v.rotation } else { 0.0 },
            artboard: 0,
        })
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

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Zoom stop stepping reads this (set each UI frame and before commands) so canvas wheel zoom
/// matches the status bar when Display Print Size at 100% Zoom is on.
static ZOOM_STOPS_PRINT_SIZE: AtomicBool = AtomicBool::new(false);

pub fn sync_zoom_stops_display_print_size(v: bool) {
    ZOOM_STOPS_PRINT_SIZE.store(v, Ordering::Relaxed);
}

/// Illustrator's preset zoom stops, in percent.
pub const ZOOM_STOPS: [f64; 26] = [
    3.13, 4.17, 6.25, 8.33, 12.5, 16.67, 25.0, 33.33, 50.0, 66.67, 100.0, 150.0, 200.0, 300.0, 400.0, 600.0, 800.0, 1200.0, 1600.0, 2400.0, 3200.0,
    4800.0, 6400.0, 12800.0, 25600.0, 64000.0,
];

/// Document zoom at 100% in the status bar: one screen point per document point, or with
/// Preferences › General › Display Print Size at 100% Zoom, one document inch in 96 screen points.
pub fn zoom_baseline(display_print_size: bool) -> f64 {
    if display_print_size { 96.0 / 72.0 } else { 1.0 }
}

pub fn zoom_display_percent(z: f64, display_print_size: bool) -> f64 {
    z / zoom_baseline(display_print_size) * 100.0
}

pub fn zoom_from_display_percent(pct: f64, display_print_size: bool) -> f64 {
    pct / 100.0 * zoom_baseline(display_print_size)
}

pub fn next_zoom(z: f64, up: bool) -> f64 {
    next_zoom_with_display(z, up, ZOOM_STOPS_PRINT_SIZE.load(Ordering::Relaxed))
}

pub fn next_zoom_with_display(z: f64, up: bool, display_print_size: bool) -> f64 {
    let pct = zoom_display_percent(z, display_print_size);
    let stop = if up {
        ZOOM_STOPS.iter().find(|s| **s > pct + 0.01).copied().unwrap_or(64000.0)
    } else {
        ZOOM_STOPS.iter().rev().find(|s| **s < pct - 0.01).copied().unwrap_or(3.13)
    };
    zoom_from_display_percent(stop, display_print_size)
}

/// Format like Illustrator's tab/zoom field: up to 2 decimals, no trailing zeros.
pub fn zoom_label(z: f64, display_print_size: bool) -> String {
    let s = format!("{:.2}", zoom_display_percent(z, display_print_size));
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
    (crate::panels::links::ID, "Links", "link"),
    (crate::panels::asset_export::ID, "Asset Export", "share-2"),
    (crate::panels::css_properties::ID, "CSS Properties", "globe"),
    (crate::panels::variables::ID, "Variables", "chart-column"),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub outline: bool,
    pub pixel_preview: bool,
    pub snap_to_grid: bool,
    pub snap_to_point: bool,
    pub artboards: bool,
    pub edges: bool,
    /// View → Trim View: hide everything outside the artboards.
    pub trim_view: bool,
    /// View → Hide/Show Corner Widget (live corner widgets on rectangles).
    pub corner_widgets: bool,
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
            outline: false,
            pixel_preview: false,
            snap_to_grid: false,
            snap_to_point: true,
            artboards: true,
            edges: true,
            trim_view: false,
            corner_widgets: true,
            snap_to_pixel: false,
            snap_to_glyph: false,
            text_threads: true,
            gradient_annotator: true,
            live_paint_gaps: false,
            hidden_chars: false,
            templates: true,
        }
    }
}

/// An open modal dialog. `fields` are string-valued so agents can set them uniformly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dialog {
    pub kind: String,
    pub fields: serde_json::Map<String, Value>,
}

impl Dialog {
    pub fn new(kind: &str, fields: Value) -> Self {
        Self { kind: kind.to_string(), fields: fields.as_object().cloned().unwrap_or_default() }
    }
    pub fn f64(&self, k: &str, d: f64) -> f64 {
        match self.fields.get(k) {
            Some(Value::Number(n)) => n.as_f64().unwrap_or(d),
            Some(Value::String(s)) => vectorcraft_doc::Unit::Points.parse(s).unwrap_or(d),
            _ => d,
        }
    }
    pub fn str(&self, k: &str) -> String {
        match self.fields.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        }
    }
    pub fn bool(&self, k: &str) -> bool {
        self.fields.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// The interface language older versions saved here (`ja`, `cs`…). It now lives in the
    /// `interfaceLanguage` preference, which [`crate::prefs_dialog::restore`] carries it over to;
    /// it is never written back.
    #[serde(rename = "language", skip_serializing)]
    pub legacy_language: Option<String>,
    pub brightness: Brightness,
    pub dock_tab: DockTab,
    /// Tab order in the dock's tabbed group when it differs from [`DockTab::ALL`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dock_tab_order: Vec<DockTab>,
    /// The dock's tabbed group (Properties | Layers | Libraries) is collapsed to icons at the top of
    /// the icon column (the dock's double arrow, `window.collapseDock`).
    #[serde(default)]
    pub dock_collapsed: bool,
    /// Icon panel currently popped out of the collapsed column (also `properties`, `layers` or
    /// `libraries` while the dock is collapsed).
    pub open_panel: Option<String>,
    pub control_bar: bool,
    pub toolbar: bool,
    pub toolbar_double: bool,
    /// Advanced toolbar (every tool group) instead of the categorized Basic toolbar.
    #[serde(default)]
    pub toolbar_advanced: bool,
    #[serde(default = "yes")]
    pub task_bar: bool,
    /// Where the Contextual Task Bar was dragged or pinned. Not saved, as in Illustrator: the bar
    /// starts under the selection at every launch.
    #[serde(skip)]
    pub task_bar_place: TaskBarPlace,
    /// The locked objects under the pointer at the last right-click on the canvas, with the
    /// document's `uid`: the context menu's Unlock submenu lists them while the menu is open. Not
    /// saved.
    #[serde(skip)]
    pub context_locked: Option<(u64, Vec<vectorcraft_doc::NodeId>)>,
    /// Last tool shown in each toolbar slot (keyed by the slot's first tool id).
    #[serde(default)]
    pub slot_tool: std::collections::BTreeMap<String, String>,
    /// Tool flyouts torn off the toolbar into floating panels.
    #[serde(default)]
    pub floating_flyouts: Vec<FloatingFlyout>,
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
    /// Floating panel groups as files from before the canvas joined [`Self::docking`] saved
    /// them. Read only to convert those files ([`crate::layout_migration`]), never written.
    #[serde(default, skip_serializing)]
    pub floating_panels: Vec<FloatingPanels>,
    /// Shared panel membership and placement; older preferences use the legacy dock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docking: Option<craft_ui::docking::Layout<String>>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub docking_hidden: std::collections::BTreeMap<String, craft_ui::docking::Location<String>>,
    /// Panels dropped on the icon column, in older files (read only, like [`Self::floating_panels`]).
    #[serde(default, skip_serializing)]
    pub docking_icons: std::collections::BTreeSet<String>,
    /// The panels shown by dock groups collapsed to their tabs, in older files (read only, like
    /// [`Self::floating_panels`]); a group showing another of its panels was expanded.
    #[serde(default, skip_serializing)]
    pub docking_collapsed: std::collections::BTreeSet<String>,
    /// Invalidates in-flight gestures when a workspace is replaced.
    #[serde(skip)]
    pub docking_generation: u64,
    /// The main dock column's width in screen points while the workspace has no layout of its
    /// own (`docking` is none); a customized layout keeps its widths in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock_width: Option<f32>,
    /// The Tools panel floats with its top-left corner here (dragged out by its title bar); `None`:
    /// docked at the window's left edge.
    #[serde(default)]
    pub toolbar_pos: Option<[f32; 2]>,
    pub status_bar: bool,
    pub dock: bool,
    /// The bars that Tab (`window.dock`) hid, which the next Tab shows again; none when Tab has not
    /// hidden them since it last showed them, or a workspace was applied since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_hid: Option<TabHid>,
    pub view: ViewFlags,
    pub dialog: Option<Dialog>,
    /// Messages to show in a dialog once the open one is closed (`dialogs::confirm::tell`), in
    /// the order they came.
    #[serde(skip)]
    pub told: Vec<Dialog>,
    /// A control request is being answered: its caller gets what fails as its reply, so no
    /// dialog says it too (`io::report_write_error`).
    #[serde(skip)]
    pub answering_control: bool,
    /// Toolbar group whose flyout is open.
    pub flyout: Option<usize>,
    /// Last tool shown for each toolbar group (flyout selection sticks).
    pub group_tool: Vec<String>,
    pub status: String,
    pub palette_open: bool,
    pub palette_query: String,
    /// Screen mode: 0 normal, 1 full screen with menu, 2 full screen, 3 presentation. Not saved:
    /// the app always starts in Normal Screen Mode, with its menus and panels (#472: a saved
    /// Presentation Mode came back on restart with no way out).
    #[serde(skip)]
    pub screen_mode: u8,
    /// Draw Normal / Behind / Inside.
    pub draw_mode: u8,
    pub about: bool,
    /// Recorded actions (Actions panel), persisted with the UI preferences.
    #[serde(default = "crate::panels::actions::default_sets")]
    pub action_sets: Vec<crate::panels::actions::ActionSet>,
    /// The current set of the Actions panel (its name clicked, one of its actions clicked, or made
    /// with New Set): a recording goes into it and Set Options renames it. Not saved.
    #[serde(skip)]
    pub action_set: Option<usize>,
    /// Recording in progress: (set index, action name, journal length when recording started).
    #[serde(skip)]
    pub recording: Option<(usize, String, usize)>,
    /// Keyboard shortcut overrides: command id, `tool:<id>` or `panel:<id>` → chord ("" = none),
    /// stored in the form every earlier version reads.
    #[serde(default, with = "crate::keymap::stored")]
    pub shortcut_overrides: craft_keymap::Overrides,
    /// Name of the shortcut set (preset or "Custom").
    #[serde(default = "default_shortcut_set", deserialize_with = "crate::shortcut_editor::deserialize_set_name")]
    pub shortcut_set: String,
    /// Current workspace (Window → Workspace).
    #[serde(default = "default_workspace")]
    pub workspace: String,
    /// Workspace before Preferences › Devices › Enable Touch Workspace switched to Touch (#394).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touch_workspace_restore: Option<String>,
    /// User-saved workspaces (New Workspace…).
    #[serde(default)]
    pub custom_workspaces: Vec<crate::workspaces::Workspace>,
    /// Layout changes made in a workspace since it was saved (or last reset), by workspace name:
    /// switching back to it brings them back; Reset <name> drops them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_changes: Vec<crate::workspaces::Workspace>,
    /// File → Open Recent Files, most recent first.
    #[serde(default)]
    pub recent_files: Vec<String>,
    /// Type → Recent Fonts, most recent first.
    #[serde(default)]
    pub recent_fonts: Vec<String>,
    /// Families starred in the font menus (the ★ filter shows only these).
    #[serde(default)]
    pub favorite_fonts: Vec<String>,
    /// Engine preferences (Edit → Preferences), persisted alongside the UI state.
    #[serde(default)]
    pub engine_prefs: Value,
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
    /// Color Guide panel: Limit to Library, a swatch library id, "document" (the document's
    /// swatches) or "" (none) (`ui.colorGuideLimit`).
    #[serde(default)]
    pub color_guide_limit: String,
    /// Align panel: the Align To choice, Selection or Artboard, which Object → Align and the align
    /// buttons follow ([`crate::panels::align::with_align_to`]).
    #[serde(default, deserialize_with = "crate::panels::align::deserialize_align_to")]
    pub align_to: crate::panels::align::AlignTo,
    /// Align panel: the Distribute Spacing value in points, which the Distribute Objects and
    /// Distribute Spacing buttons and their commands send while there is a key object or a key
    /// guide ([`crate::panels::align::run_button`]). Not saved.
    #[serde(skip)]
    pub align_spacing: f64,
    /// Color panel: the mode picked last from its menu (`None`: never picked), in which the panel
    /// shows the defaults for new art ([`crate::panels::color::deserialize_mode`]).
    #[serde(default, deserialize_with = "crate::panels::color::deserialize_mode")]
    pub color_mode: Option<crate::panels::color::Mode>,
    /// Color panel: Hide Options, which leaves just the proxy and the spectrum (`ui.colorPanel`,
    /// [`crate::panels::color::deserialize_hide_options`]).
    #[serde(default, deserialize_with = "crate::panels::color::deserialize_hide_options")]
    pub color_hide_options: bool,
    /// The SVG Options chosen last (`svg` object of the export/save commands; null: never used).
    #[serde(default)]
    pub svg_options: Value,
    /// File → Place: Link is on (the Place dialog remembers it).
    #[serde(default = "yes")]
    pub place_link: bool,
    /// The file the open dialog reads, kept out of its JSON fields (Import PDF); dropped when no
    /// dialog is open.
    #[serde(skip)]
    pub dialog_file: Option<std::sync::Arc<crate::dialogs::import_pdf::DialogFile>>,
    /// The documents (uids) whose Missing Fonts dialog waits until no dialog is open, the next one to
    /// show first ([`crate::dialogs::settle`]).
    #[serde(skip)]
    pub pending_fonts: Vec<u64>,
    /// The document (uid) whose Missing Fonts dialog is on show. When another dialog takes its
    /// place, [`crate::dialogs::settle`] puts the document back at the front of `pending_fonts`.
    #[serde(skip)]
    pub fonts_dialog: Option<u64>,
    /// The search the Missing Fonts dialog started (`text.findFontFiles`'s `id`), stopped once the
    /// dialog is gone.
    #[serde(skip)]
    pub dialog_search: Option<u64>,
    /// The search the Find Linked Files dialog started (`links.findFiles`'s `id`), stopped once the
    /// dialog is gone.
    #[serde(skip)]
    pub links_search: Option<u64>,
    /// The DXF Options chosen last (`document.exportDxf` options; null: never used).
    #[serde(default)]
    pub dxf_options: Value,
    /// The EPS Options chosen last (`document.exportEps` options; null: never used).
    #[serde(default)]
    pub eps_options: Value,
    /// The Offset Path fields confirmed last (offset, joins, miterLimit; null: never used): its
    /// dialog opens on them (#1152).
    #[serde(default)]
    pub offset_path: Value,
    /// The Simplify dialog's fields confirmed last with "Open this dialog with these settings next
    /// time" ticked (null: not ticked): Object › Path › Simplify opens the dialog on them instead
    /// of the Simplify slider.
    #[serde(default)]
    pub simplify: Value,
    /// The Move dialog's checkboxes as it was last confirmed with (copy, objects, preview; null:
    /// never used): it opens with them.
    #[serde(default)]
    pub move_options: Value,
    /// The Transform Each fields it was last confirmed with (scale, move, rotate, reflect, random,
    /// reference point, Transform Objects and Preview, not Copy; null: never used): it opens on
    /// them (#1732).
    #[serde(default)]
    pub transform_each: Value,
    /// The DXF Import Options chosen last (fit, scaleLineweights, center, mergeLayers; null:
    /// never used).
    #[serde(default)]
    pub dxf_import: Value,
    /// The Home screen is shown over the open documents (`app.home`): the active document's uid
    /// and the document count when it opened. Choosing a tab, or a document opening, closing or
    /// becoming active, leaves it.
    #[serde(skip)]
    pub home: Option<(Option<u64>, usize)>,
    /// The Image Trace objects the canvas draws with the Source Image view while the Image Trace
    /// panel's eye button is held down ([`vectorcraft_render::RenderOptions::trace_source`]).
    #[serde(skip)]
    pub trace_hold_source: Option<std::sync::Arc<Vec<vectorcraft_doc::NodeId>>>,
    /// Layers panel › Panel Options… (row size, thumbnails, Show Layers Only, Locate Selected Objects).
    #[serde(default)]
    pub layers_panel: crate::panels::layers::PanelOptions,
    /// The Layers panel's Filter (`ui.layersFilter`). It is not saved and lasts the session, as the
    /// Search All text does.
    #[serde(skip)]
    pub layers_filter: crate::panels::layers::Filter,
    /// The Image Trace panel's Advanced section is open (as it was last left).
    #[serde(default = "yes")]
    pub image_trace_advanced: bool,
    /// The Image Trace panel's Preview: a setting changed in the panel traces the selected Image
    /// Trace object again at once. Off, the changes wait in the panel until Trace applies them.
    #[serde(default = "yes")]
    pub image_trace_preview: bool,
    /// Glyphs panel preview step, kept with the other panel choices. `0` is the default 30 pt
    /// cell; `1`..=`7` is a step from smallest to largest (`crates/ui-egui/src/panels/glyphs.rs`).
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub glyphs_preview: u8,
    /// The desktop window's size, position and maximized state, saved when the app quits and
    /// restored at the next launch (the desktop host reads and writes it; none on the web).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowGeometry>,
}

/// The desktop window's geometry, kept across launches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    /// Top-left corner of the window frame, in physical pixels on the desktop (none where the
    /// system doesn't tell windows where they are).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[i32; 2]>,
    /// Size of the window's contents in logical pixels (points at 100% UI scaling).
    pub size: [f32; 2],
    /// The window was maximized; `pos` and `size` are where un-maximizing puts it.
    #[serde(default)]
    pub maximized: bool,
}

/// A tool group's flyout torn off the toolbar: it floats as its own panel until its × puts it back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloatingFlyout {
    /// The group's tools in flyout order; the first one names the toolbar slot.
    pub tools: Vec<String>,
    /// Top-left corner in screen points.
    pub pos: [f32; 2],
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

/// Where the Contextual Task Bar sits once its handle has moved it (`window.taskBar.pin`,
/// `window.taskBar.reset`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TaskBarPlace {
    /// More Options › Pin Bar Position: the bar stays where it is instead of following the selection.
    pub pinned: bool,
    /// Where a pinned bar's top-left corner sits from the canvas's top-left (none when it was
    /// pinned before it ever showed, until it is drawn). Unpinning leaves it for the next frame
    /// to turn into `offset`.
    pub pin_at: Option<egui::Vec2>,
    /// Where the bar was last drawn, from the canvas's top-left: where pinning holds it.
    pub shown_at: Option<egui::Vec2>,
    /// How far an unpinned bar was dragged from its place under the selection, which it keeps while
    /// it follows the selection, and the document (`DocState::uid`) it was moved in: in another
    /// document the bar starts under the selection again.
    pub offset: Option<(u64, egui::Vec2)>,
}

/// Which of the panels, the toolbar and the Control bar showed when Tab (`window.dock`) hid them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabHid {
    pub dock: bool,
    pub toolbar: bool,
    pub control_bar: bool,
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

/// Drop a Tools panel position that isn't a number (a hand-edited preferences file or workspace).
pub fn sanitize_toolbar_pos(toolbar_pos: &mut Option<[f32; 2]>) {
    if toolbar_pos.is_some_and(|p| !p.iter().all(|v| v.is_finite())) {
        *toolbar_pos = None;
    }
}

impl UiState {
    /// Clear transient state after loading saved preferences.
    pub fn sanitized(mut self) -> Self {
        self.dialog = None;
        self.flyout = None;
        self.tools_drawer = false;
        self.palette_open = false;
        self.status.clear();
        self.about = false;
        self.open_panel = None;
        if self.group_tool.len() != vectorcraft_tools::TOOL_GROUPS.len() {
            self.group_tool = UiState::default().group_tool;
        }
        // One strip per group, of known tools (a hand-edited preferences file).
        let mut seen = std::collections::BTreeSet::new();
        self.floating_flyouts.retain_mut(|f| {
            f.tools.retain(|id| vectorcraft_tools::tool_info(id).is_some());
            f.tools.first().is_some_and(|k| seen.insert(k.clone()))
        });
        sanitize_toolbar_pos(&mut self.toolbar_pos);
        crate::edit_toolbar::sanitize(&mut self.toolbar_edits);
        // `read_preferences` converted the file's older layouts already; this covers a state read
        // any other way. What it reports lost was dropped there.
        let _ = crate::layout_migration::migrate_ui(&mut self);
        self.docking = self.docking.filter(crate::panel_docking::valid);
        // Changes kept for workspaces that still exist, under their saved spelling and one per
        // workspace whatever the case (a hand-edited file).
        let mut seen = std::collections::BTreeSet::new();
        let saved: Vec<String> =
            self.custom_workspaces.iter().map(|w| w.name.clone()).chain(crate::workspaces::builtins().into_iter().map(|w| w.name)).collect();
        self.workspace_changes.retain_mut(|w| {
            let Some(name) = saved.iter().find(|n| n.eq_ignore_ascii_case(&w.name)) else { return false };
            w.name = name.clone();
            seen.insert(name.to_ascii_lowercase())
        });
        self.workspace_changes.truncate(64);
        // Overrides that can't fire (modifier-only chords recorded by older versions, #487) give
        // the default back.
        let unusable: Vec<String> = self
            .shortcut_overrides
            .iter()
            .filter(|(_, texts)| !texts.iter().all(|t| craft_keymap::Binding::parse(t).is_ok()))
            .map(|(k, _)| k.to_string())
            .collect();
        for k in unusable {
            self.shortcut_overrides.remove(&k);
        }
        self
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            legacy_language: None,
            brightness: Brightness::MediumDark,
            dock_tab: DockTab::Properties,
            dock_tab_order: Vec::new(),
            dock_collapsed: false,
            open_panel: None,
            control_bar: false,
            toolbar: true,
            toolbar_double: false,
            toolbar_advanced: false,
            task_bar: true,
            task_bar_place: TaskBarPlace::default(),
            context_locked: None,
            slot_tool: Default::default(),
            floating_flyouts: vec![],
            toolbar_edits: ToolbarEdits::default(),
            toolbar_show: ToolbarShow::default(),
            tools_drawer: false,
            floating_panels: vec![],
            docking: None,
            docking_hidden: Default::default(),
            docking_icons: Default::default(),
            docking_collapsed: Default::default(),
            docking_generation: 0,
            dock_width: None,
            toolbar_pos: None,
            status_bar: true,
            dock: true,
            tab_hid: None,
            view: ViewFlags::default(),
            dialog: None,
            told: vec![],
            answering_control: false,
            flyout: None,
            group_tool: vectorcraft_tools::TOOL_GROUPS.iter().map(|g| g[0].id.to_string()).collect(),
            status: String::new(),
            palette_open: false,
            palette_query: String::new(),
            screen_mode: 0,
            draw_mode: 0,
            about: false,
            action_sets: crate::panels::actions::default_sets(),
            action_set: None,
            recording: None,
            shortcut_overrides: Default::default(),
            shortcut_set: default_shortcut_set(),
            workspace: default_workspace(),
            touch_workspace_restore: None,
            custom_workspaces: vec![],
            workspace_changes: vec![],
            recent_files: vec![],
            recent_fonts: vec![],
            favorite_fonts: vec![],
            engine_prefs: Value::Null,
            color_guide: Default::default(),
            library_panels: vec![],
            flattener_preview: Default::default(),
            color_guide_limit: String::new(),
            align_to: Default::default(),
            align_spacing: 0.0,
            color_mode: None,
            color_hide_options: false,
            svg_options: Value::Null,
            place_link: true,
            dialog_file: None,
            pending_fonts: vec![],
            fonts_dialog: None,
            dialog_search: None,
            links_search: None,
            dxf_options: Value::Null,
            eps_options: Value::Null,
            offset_path: Value::Null,
            simplify: Value::Null,
            move_options: Value::Null,
            transform_each: Value::Null,
            dxf_import: Value::Null,
            home: None,
            trace_hold_source: None,
            layers_panel: Default::default(),
            layers_filter: Default::default(),
            image_trace_advanced: true,
            image_trace_preview: true,
            glyphs_preview: 0,
            window: None,
        }
    }
}

fn yes() -> bool {
    true
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

fn default_shortcut_set() -> String {
    crate::shortcut_editor::PRESETS[0].to_string()
}

fn default_workspace() -> String {
    crate::workspaces::ESSENTIALS.to_string()
}

/// One UI pass key: the context identity, the viewport and that viewport's cumulative frame.
pub(crate) type InputFrame = (u64, egui::ViewportId, u64);

/// Record `frame` unless this host iteration already took it. A discarded rerun and any
/// interleaved pass order stay exact-once because the key, not the last-seen slot, is compared.
/// The list is cleared at every host-iteration boundary, so it stays as small as that iteration's
/// pass count.
pub(crate) fn take_frame_once(frames: &mut Vec<InputFrame>, frame: InputFrame) -> bool {
    if frames.contains(&frame) {
        return false;
    }
    frames.push(frame);
    true
}

/// The document, dialog and focus an injected batch belongs to: if any changed before a UI pass
/// took the batch, the clicks or text no longer belong to the new state and are dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct InputGeneration {
    pub(crate) document: u64,
    pub(crate) dialog: Option<egui::Id>,
    pub(crate) focus: Option<egui::Id>,
}

/// The window geometry a capture belongs to (logical and screen bounds, DPI and zoom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeometryStamp {
    pub(crate) viewport: egui::ViewportId,
    pub(crate) content: egui::Rect,
    pub(crate) inner: Option<egui::Rect>,
    pub(crate) pixels_per_point: f32,
    pub(crate) zoom: f32,
}

/// What the issued capture must still match when it completes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureEpoch {
    pub(crate) geometry: u64,
    pub(crate) viewport: egui::ViewportId,
    /// The active document (uid, revision) at issuance: a screenshot never answers with pixels of
    /// a document, undo state or selection that replaced it while the capture was in flight.
    pub(crate) document: Option<(u64, u64)>,
    /// The image size the issuing geometry predicts (`width * height`, checked).
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// An admitted capture waiting for its settled frame (`frames`) and animation delay (`due_ms`).
pub(crate) struct QueuedScreenshot {
    pub(crate) id: u64,
    pub(crate) due_ms: f64,
    pub(crate) frames: u32,
}

/// A window capture asked for through the control channel (`ui.screenshot`), answered once a
/// frame is presented; a window that isn't presented never delivers one, hence the deadline.
pub(crate) struct PendingScreenshot {
    pub(crate) id: u64,
    pub(crate) path: Option<String>,
    /// Send the PNG back as `pngBase64`.
    pub(crate) data: bool,
    pub(crate) reply: Sender<ControlResponse>,
    /// When to give up (ms, as [`now_ms`], monotonic).
    pub(crate) deadline_ms: f64,
    /// The checked bytes this request reserved before it was admitted.
    pub(crate) reserved_bytes: usize,
    /// The active document at admission: switching documents before issuance fails the request
    /// rather than capturing a different document.
    pub(crate) document: Option<(u64, u64)>,
    /// The live one-shot completion slot while the command is outstanding.
    pub(crate) slot: Option<craft_ui::capture::CaptureSlot>,
    /// The geometry and expected image size at issuance.
    pub(crate) issued: Option<CaptureEpoch>,
    /// Fresh slots made after a geometry change (a resize while the capture was in flight).
    pub(crate) attempts: u8,
}

/// At most this many captures admitted (pending or queued) at once.
pub(crate) const MAX_PENDING_SCREENSHOTS: usize = 4;

/// The largest single capture admitted (128 MiB: a 4K viewport at 2x).
pub(crate) const MAX_SCREENSHOT_BYTES: usize = 128 * 1024 * 1024;

/// The aggregate budget of admitted captures (256 MiB).
pub(crate) const MAX_SCREENSHOT_TOTAL_BYTES: usize = 256 * 1024 * 1024;

/// Settle frames a capture waits for after a control request (and the animation time).
pub(crate) const SCREENSHOT_SETTLE_FRAMES: u32 = 3;

/// How long after settling an issued capture may take (ms).
pub(crate) const SCREENSHOT_TIMEOUT_MS: f64 = 8000.0;

/// The timeout's answer: no file, no second reply.
pub(crate) const SCREENSHOT_TIMEOUT_ERROR: &str =
    "no frame was presented (screen locked, window minimized or fully covered); ui.render still renders the artboard";

/// Seconds between two looks at the system clipboard for [`VectorcraftApp::system_paste`].
pub(crate) const SYSTEM_CLIPBOARD_POLL: f64 = 0.25;

/// Seconds between looks for fonts a font manager activated while the app stays in front (#2576).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const FONT_INSTALL_POLL: f64 = 2.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps() {
        assert_eq!(next_zoom_with_display(1.0, true, false), 1.5);
        assert!((next_zoom_with_display(1.0, false, false) - 0.6667).abs() < 1e-9);
        assert_eq!(next_zoom_with_display(640.0, true, false), 640.0);
        assert_eq!(zoom_label(0.6667, false), "66.67%");
        assert_eq!(zoom_label(1.0, false), "100%");
        assert_eq!(zoom_label(0.125, false), "12.5%");
    }

    /// storytold/vectorcraft#2719: 100% in the zoom field matches Actual Size when Display Print Size is on.
    #[test]
    fn zoom_percent_honors_display_print_size_baseline() {
        let b = zoom_baseline(true);
        assert!((zoom_from_display_percent(100.0, true) - b).abs() < 1e-12);
        assert_eq!(zoom_from_display_percent(100.0, false), 1.0);
        assert_eq!(zoom_label(b, true), "100%");
        assert_eq!(next_zoom_with_display(b, true, true), zoom_from_display_percent(150.0, true));
    }

    #[test]
    fn dialog_fields() {
        let d = Dialog::new("rectangle", serde_json::json!({"width": "1in", "height": 50}));
        assert_eq!(d.f64("width", 0.0), 72.0);
        assert_eq!(d.f64("height", 0.0), 50.0);
    }
}
