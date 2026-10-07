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
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: 1.0, center: Point::new(306.0, 396.0), fitted: false, rotation: 0.0 }
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
    ("blend", "Blend", "dc-blend"),
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
    /// Icon panel currently popped out of the collapsed column.
    pub open_panel: Option<String>,
    pub control_bar: bool,
    pub toolbar: bool,
    pub toolbar_double: bool,
    /// Advanced toolbar (every tool group) instead of the categorized Basic toolbar.
    #[serde(default)]
    pub toolbar_advanced: bool,
    #[serde(default = "yes")]
    pub task_bar: bool,
    /// Last tool shown in each toolbar slot (keyed by the slot's first tool id).
    #[serde(default)]
    pub slot_tool: std::collections::BTreeMap<String, String>,
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
    /// Screen mode: 0 normal, 1 full screen with menu, 2 full screen, 3 presentation.
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
        self
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            legacy_language: None,
            brightness: Brightness::MediumDark,
            dock_tab: DockTab::Properties,
            open_panel: None,
            control_bar: false,
            toolbar: true,
            toolbar_double: false,
            toolbar_advanced: false,
            task_bar: true,
            slot_tool: Default::default(),
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
            engine_prefs: Value::Null,
            color_guide: Default::default(),
            library_panel: None,
            flattener_preview: Default::default(),
            color_guide_limit: String::new(),
            svg_options: Value::Null,
            place_link: true,
            dialog_file: None,
            dxf_options: Value::Null,
            eps_options: Value::Null,
            dxf_import: Value::Null,
            home: None,
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
