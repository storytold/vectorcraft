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
        match &st.view {
            Some(v) if v.zoom.is_finite() && v.zoom > 0.0 && v.center.x.is_finite() && v.center.y.is_finite() => Self {
                zoom: v.zoom.clamp(0.0313, 640.0),
                center: v.center,
                fitted: true,
                rotation: if v.rotation.is_finite() { v.rotation } else { 0.0 },
                artboard: 0,
            },
            _ => Self::default(),
        }
    }
}

/// Illustrator's preset zoom stops, in percent.
pub const ZOOM_STOPS: [f64; 26] = [
    3.13, 4.17, 6.25, 8.33, 12.5, 16.67, 25.0, 33.33, 50.0, 66.67, 100.0, 150.0, 200.0, 300.0, 400.0, 600.0, 800.0, 1200.0, 1600.0, 2400.0, 3200.0,
    4800.0, 6400.0, 12800.0, 25600.0, 64000.0,
];

pub fn next_zoom(z: f64, up: bool) -> f64 {
    let pct = z * 100.0;
    let stop = if up {
        ZOOM_STOPS.iter().find(|s| **s > pct + 0.01).copied().unwrap_or(64000.0)
    } else {
        ZOOM_STOPS.iter().rev().find(|s| **s < pct - 0.01).copied().unwrap_or(3.13)
    };
    stop / 100.0
}

/// Format like Illustrator's tab/zoom field: up to 2 decimals, no trailing zeros.
pub fn zoom_label(z: f64) -> String {
    let s = format!("{:.2}", z * 100.0);
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
    ("colorThemes", "Color Themes", "sun"),
    (crate::panels::links::ID, "Links", "link"),
    (crate::panels::asset_export::ID, "Asset Export", "share-2"),
    (crate::panels::css_properties::ID, "CSS Properties", "globe"),
];

/// Groups of icon panels separated by dividers in the collapsed column.
pub const ICON_PANEL_GROUPS: &[&[&str]] = &[
    &["color", "colorGuide"],
    &["swatches", "brushes", "symbols", "patternOptions"],
    &["stroke", "gradient", "transparency"],
    &["appearance", "graphicStyles"],
    &["artboards"],
    &["transform", "align", "pathfinder"],
    &["character", "paragraph", "glyphs"],
    &["history", "actions", "info", "navigator"],
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewFlags {
    pub rulers: bool,
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
    /// View → Show/Hide Text Threads.
    pub text_threads: bool,
    /// View → Show/Hide Gradient Annotator.
    pub gradient_annotator: bool,
    /// Type → Show Hidden Characters.
    pub hidden_chars: bool,
}

impl Default for ViewFlags {
    fn default() -> Self {
        Self {
            rulers: false,
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
            text_threads: true,
            gradient_annotator: true,
            hidden_chars: false,
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
    /// Last tool shown in each toolbar slot (keyed by the slot's first tool id).
    #[serde(default)]
    pub slot_tool: std::collections::BTreeMap<String, String>,
    /// Tool flyouts torn off the toolbar into floating panels.
    #[serde(default)]
    pub floating_flyouts: Vec<FloatingFlyout>,
    /// Panels dragged out of the dock: each group floats as its own stack of tabs.
    #[serde(default)]
    pub floating_panels: Vec<FloatingPanels>,
    /// The Tools panel floats with its top-left corner here (dragged out by its title bar); `None`:
    /// docked at the window's left edge.
    #[serde(default)]
    pub toolbar_pos: Option<[f32; 2]>,
    pub status_bar: bool,
    pub dock: bool,
    pub view: ViewFlags,
    pub dialog: Option<Dialog>,
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
    /// Recording in progress: (set index, action name, journal length when recording started).
    #[serde(skip)]
    pub recording: Option<(usize, String, usize)>,
    /// Keyboard shortcut overrides: command id or `tool:<id>` → chord ("" = none).
    #[serde(default)]
    pub shortcut_overrides: std::collections::BTreeMap<String, String>,
    /// Name of the shortcut set (preset or "Custom").
    #[serde(default = "default_shortcut_set", deserialize_with = "crate::shortcut_editor::deserialize_set_name")]
    pub shortcut_set: String,
    /// Current workspace (Window → Workspace).
    #[serde(default = "default_workspace")]
    pub workspace: String,
    /// User-saved workspaces (New Workspace…).
    #[serde(default)]
    pub custom_workspaces: Vec<crate::workspaces::Workspace>,
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
    /// The library open in the library panel (Window → Swatch Libraries).
    #[serde(default)]
    pub library_panel: Option<crate::panels::library_panel::OpenLibrary>,
    /// Flattener Preview panel: highlight, overprints, preset and options (`ui.flattenerPreview`).
    #[serde(default)]
    pub flattener_preview: crate::panels::flattener_preview::Settings,
    /// Color Guide panel: Limit to Library, a swatch library id, "document" (the document's
    /// swatches) or "" (none) (`ui.colorGuideLimit`).
    #[serde(default)]
    pub color_guide_limit: String,
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
    /// The DXF Options chosen last (`document.exportDxf` options; null: never used).
    #[serde(default)]
    pub dxf_options: Value,
    /// The EPS Options chosen last (`document.exportEps` options; null: never used).
    #[serde(default)]
    pub eps_options: Value,
    /// The DXF Import Options chosen last (fit, scaleLineweights, center, mergeLayers; null:
    /// never used).
    #[serde(default)]
    pub dxf_import: Value,
    /// The Home screen is shown over the open documents (`app.home`): the active document's uid
    /// and the document count when it opened. Choosing a tab, or a document opening, closing or
    /// becoming active, leaves it.
    #[serde(skip)]
    pub home: Option<(Option<u64>, usize)>,
    /// Layers panel › Panel Options… (row size, thumbnails, Show Layers Only).
    #[serde(default)]
    pub layers_panel: crate::panels::layers::PanelOptions,
    /// The Image Trace panel's Advanced section is open (as it was last left).
    #[serde(default = "yes")]
    pub image_trace_advanced: bool,
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

/// A group of panels dragged out of the dock: it floats as a stack of tabs, one panel shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
}

impl FloatingPanels {
    /// Keep known panels, each in one group, drop the groups left empty and the toolbar position
    /// that isn't a number (a hand-edited preferences file or workspace).
    pub fn sanitize(groups: &mut Vec<FloatingPanels>, toolbar_pos: &mut Option<[f32; 2]>) {
        let mut seen = std::collections::BTreeSet::new();
        groups.retain_mut(|g| {
            g.panels.retain(|id| all_panels().any(|(p, _)| p == id) && seen.insert(id.clone()));
            g.active = g.active.min(g.panels.len().saturating_sub(1));
            !g.panels.is_empty()
        });
        if toolbar_pos.is_some_and(|p| !p.iter().all(|v| v.is_finite())) {
            *toolbar_pos = None;
        }
    }
}

impl UiState {
    /// Clear transient state after loading saved preferences.
    pub fn sanitized(mut self) -> Self {
        self.dialog = None;
        self.flyout = None;
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
        FloatingPanels::sanitize(&mut self.floating_panels, &mut self.toolbar_pos);
        // Overrides that can't fire (modifier-only chords recorded by older versions, #487) give
        // the default back.
        self.shortcut_overrides.retain(|_, c| c.is_empty() || crate::shortcut_editor::normalize(c).is_some());
        self
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            legacy_language: None,
            brightness: Brightness::MediumDark,
            dock_tab: DockTab::Properties,
            dock_collapsed: false,
            open_panel: None,
            control_bar: false,
            toolbar: true,
            toolbar_double: false,
            toolbar_advanced: false,
            task_bar: true,
            task_bar_place: TaskBarPlace::default(),
            slot_tool: Default::default(),
            floating_flyouts: vec![],
            floating_panels: vec![],
            toolbar_pos: None,
            status_bar: true,
            dock: true,
            view: ViewFlags::default(),
            dialog: None,
            flyout: None,
            group_tool: vectorcraft_tools::TOOL_GROUPS.iter().map(|g| g[0].id.to_string()).collect(),
            status: String::new(),
            palette_open: false,
            palette_query: String::new(),
            screen_mode: 0,
            draw_mode: 0,
            about: false,
            action_sets: crate::panels::actions::default_sets(),
            recording: None,
            shortcut_overrides: Default::default(),
            shortcut_set: default_shortcut_set(),
            workspace: default_workspace(),
            custom_workspaces: vec![],
            recent_files: vec![],
            recent_fonts: vec![],
            favorite_fonts: vec![],
            engine_prefs: Value::Null,
            color_guide: Default::default(),
            library_panel: None,
            flattener_preview: Default::default(),
            color_guide_limit: String::new(),
            svg_options: Value::Null,
            place_link: true,
            dialog_file: None,
            pending_fonts: vec![],
            fonts_dialog: None,
            dialog_search: None,
            dxf_options: Value::Null,
            eps_options: Value::Null,
            dxf_import: Value::Null,
            home: None,
            layers_panel: Default::default(),
            image_trace_advanced: true,
            window: None,
        }
    }
}

fn yes() -> bool {
    true
}

fn default_shortcut_set() -> String {
    crate::shortcut_editor::PRESETS[0].to_string()
}

fn default_workspace() -> String {
    crate::workspaces::ESSENTIALS.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps() {
        assert_eq!(next_zoom(1.0, true), 1.5);
        assert!((next_zoom(1.0, false) - 0.6667).abs() < 1e-9);
        assert_eq!(next_zoom(640.0, true), 640.0);
        assert_eq!(zoom_label(0.6667), "66.67%");
        assert_eq!(zoom_label(1.0), "100%");
        assert_eq!(zoom_label(0.125), "12.5%");
    }

    #[test]
    fn dialog_fields() {
        let d = Dialog::new("rectangle", serde_json::json!({"width": "1in", "height": 50}));
        assert_eq!(d.f64("width", 0.0), 72.0);
        assert_eq!(d.f64("height", 0.0), 50.0);
    }
}
