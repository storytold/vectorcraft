//! Workspace Switcher: Hot-swappable UI layouts, panel anchoring, and keyboard profiles
//! (Illustrator default, CorelDRAW Classic, and Print Production).

use std::collections::HashMap;

use craft_keymap::{Keymap, Overrides};
use egui::{Color32, Sense, Stroke, Ui, vec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::VectorcraftApp;
use crate::dock;
use crate::keymap::{self, overrides_from};
use crate::shortcut_editor;
use crate::theme::Tokens;

/// Supported high-level workflow profiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum WorkspaceProfile {
    #[default]
    VectorCraftDefault,
    CorelDrawClassic,
    PrintProduction,
}

impl WorkspaceProfile {
    pub const ALL: [WorkspaceProfile; 3] = [
        WorkspaceProfile::VectorCraftDefault,
        WorkspaceProfile::CorelDrawClassic,
        WorkspaceProfile::PrintProduction,
    ];

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::VectorCraftDefault => "Default (Illustrator style)",
            Self::CorelDrawClassic => "CorelDRAW Classic",
            Self::PrintProduction => "Print Production",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::VectorCraftDefault => "Standard dual-column dock, tabbed layout, and Illustrator shortcuts",
            Self::CorelDrawClassic => "Right-docked inspectors, vertical color palette, and Corel hotkeys",
            Self::PrintProduction => "Prepress dockers, separations preview, flattener, and proofing tools",
        }
    }
}

/// Dynamic workspace configuration controlling docker visibility, anchoring side, and shortcuts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub active_profile: WorkspaceProfile,
    pub panel_visibility: HashMap<String, bool>,
    #[serde(skip, default = "default_panel_alignment")]
    pub panel_alignment: HashMap<String, egui::panel::Side>,
    #[serde(skip, default = "default_keymap")]
    pub active_keymap: Keymap,
}

fn default_panel_alignment() -> HashMap<String, egui::panel::Side> {
    let mut alignment = HashMap::new();
    alignment.insert("properties".into(), egui::panel::Side::Right);
    alignment.insert("layers".into(), egui::panel::Side::Right);
    alignment.insert("transform".into(), egui::panel::Side::Right);
    alignment.insert("color_palette".into(), egui::panel::Side::Right);
    alignment
}

fn default_keymap() -> Keymap {
    keymap::template().clone()
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self::for_profile(WorkspaceProfile::VectorCraftDefault)
    }
}

impl WorkspaceConfig {
    /// Factory for building presets.
    pub fn for_profile(profile: WorkspaceProfile) -> Self {
        match profile {
            WorkspaceProfile::VectorCraftDefault => Self::default_vectorcraft(),
            WorkspaceProfile::CorelDrawClassic => Self::coreldraw_classic(),
            WorkspaceProfile::PrintProduction => Self::print_production(),
        }
    }

    /// Default VectorCraft / Illustrator-style workspace.
    pub fn default_vectorcraft() -> Self {
        let mut visibility = HashMap::new();
        visibility.insert("properties".into(), true);
        visibility.insert("layers".into(), true);
        visibility.insert("libraries".into(), false);

        let mut alignment = HashMap::new();
        alignment.insert("properties".into(), egui::panel::Side::Right);
        alignment.insert("layers".into(), egui::panel::Side::Right);

        Self {
            active_profile: WorkspaceProfile::VectorCraftDefault,
            panel_visibility: visibility,
            panel_alignment: alignment,
            active_keymap: default_keymap(),
        }
    }

    /// CorelDRAW Classic preset:
    /// - All dockers anchor to the right.
    /// - Enables "Object Properties", "Object Manager" (Layers), and "Transform".
    /// - Vertical color palette on the extreme right edge with Left-Click (Fill) & Right-Click (Outline).
    /// - Full CorelDRAW keyboard shortcut mapping.
    pub fn coreldraw_classic() -> Self {
        let mut visibility = HashMap::new();
        visibility.insert("properties".into(), true); // Object Properties
        visibility.insert("layers".into(), true);     // Object Manager
        visibility.insert("transform".into(), true);  // Transform docker
        visibility.insert("align".into(), false);
        visibility.insert("color_palette".into(), true); // Extreme right vertical palette

        let mut alignment = HashMap::new();
        alignment.insert("properties".into(), egui::panel::Side::Right);
        alignment.insert("layers".into(), egui::panel::Side::Right);
        alignment.insert("transform".into(), egui::panel::Side::Right);
        alignment.insert("color_palette".into(), egui::panel::Side::Right);

        Self {
            active_profile: WorkspaceProfile::CorelDrawClassic,
            panel_visibility: visibility,
            panel_alignment: alignment,
            active_keymap: coreldraw_keymap(),
        }
    }

    /// Print Production preset:
    /// - Enables Separations Preview, Flattener Preview, Document Info, and Color Swatches.
    pub fn print_production() -> Self {
        let mut visibility = HashMap::new();
        visibility.insert("separations".into(), true);
        visibility.insert("flattenerPreview".into(), true);
        visibility.insert("docInfo".into(), true);
        visibility.insert("swatches".into(), true);
        visibility.insert("properties".into(), true);

        let mut alignment = HashMap::new();
        alignment.insert("separations".into(), egui::panel::Side::Right);
        alignment.insert("flattenerPreview".into(), egui::panel::Side::Right);
        alignment.insert("docInfo".into(), egui::panel::Side::Right);
        alignment.insert("swatches".into(), egui::panel::Side::Right);
        alignment.insert("properties".into(), egui::panel::Side::Right);

        Self {
            active_profile: WorkspaceProfile::PrintProduction,
            panel_visibility: visibility,
            panel_alignment: alignment,
            active_keymap: default_keymap(),
        }
    }

    /// Instantly mutates the configuration to a new profile, reloading layout and keymap.
    pub fn apply_profile(&mut self, app: &mut VectorcraftApp, profile: WorkspaceProfile) {
        *self = Self::for_profile(profile);
        self.sync_to_app(app);
    }

    /// Pushes the active profile's overrides and panel layout into the app state.
    pub fn sync_to_app(&self, app: &mut VectorcraftApp) {
        match self.active_profile {
            WorkspaceProfile::CorelDrawClassic => {
                app.ui.control_bar = true;
                app.ui.view.rulers = true;
                app.ui.toolbar_advanced = true;
                app.ui.open_panel = Some("properties".into());
                app.ui.shortcut_overrides = coreldraw_overrides();
                app.ui.shortcut_set = "CorelDRAW Classic".into();
            }
            WorkspaceProfile::PrintProduction => {
                app.ui.control_bar = true;
                app.ui.view.rulers = true;
                app.ui.toolbar_advanced = true;
                app.ui.open_panel = Some("separations".into());
                app.ui.shortcut_set = "Print Production".into();
            }
            WorkspaceProfile::VectorCraftDefault => {
                app.ui.shortcut_overrides.clear();
                app.ui.shortcut_set = "VectorCraft Defaults".into();
            }
        }
        shortcut_editor::sync(&app.ui);
    }
}

// ---------------------------------------------------------------------------
// CorelDRAW Keyboard Shortcuts Mapping
// ---------------------------------------------------------------------------

/// Overrides mapping CorelDRAW shortcuts to VectorCraft tools and engine commands.
pub fn coreldraw_overrides() -> Overrides {
    overrides_from(&[
        // Tools
        ("tool:select", "Space"),        // Pick tool
        ("tool:directSelect", "F10"),    // Shape tool
        ("tool:rectangle", "F6"),       // Rectangle tool
        ("tool:ellipse", "F7"),         // Ellipse tool
        ("tool:type", "F8"),            // Text tool
        ("tool:pencil", "F5"),          // Freehand tool
        ("tool:polygon", "Y"),          // Polygon tool
        ("tool:gradient", "G"),         // Interactive Fill
        ("tool:mesh", "M"),             // Mesh Fill
        ("tool:hand", "H"),             // Pan / Hand tool
        ("tool:zoom", "F2"),            // Zoom tool

        // Zoom Navigation
        ("view.fitAllInWindow", "F4"),             // Zoom to All Objects
        ("view.fitSelectionInWindow", "Shift+F2"), // Zoom to Selected
        ("view.fitArtboardInWindow", "Shift+F4"),  // Zoom to Page
        ("view.zoomOut", "F3"),                    // Zoom Out

        // Alignment (Single-key Corel alignment)
        ("align.horizontalVerticalCenter", "P"),   // Center to Page
        ("align.horizontalCenter", "E"),           // Align Centers Horizontally
        ("align.verticalCenter", "C"),             // Align Centers Vertically
        ("align.verticalTop", "T"),                // Align Top
        ("align.verticalBottom", "B"),             // Align Bottom
        ("align.horizontalLeft", "L"),             // Align Left
        ("align.horizontalRight", "R"),            // Align Right

        // Object Operations
        ("type.createOutlines", "Cmd+Q"),          // Convert to Curves (Ctrl+Q)
        ("object.compoundPath.release", "Cmd+K"),  // Break Apart (Ctrl+K)
        ("object.compoundPath.make", "Cmd+L"),     // Combine (Ctrl+L)
        ("object.group", "Cmd+G"),                 // Group
        ("object.ungroup", "Cmd+U"),               // Ungroup
        ("object.transformAgain", "Cmd+D"),        // Duplicate

        // Arranging & Stacking Order
        ("object.arrange.bringForward", "Cmd+PageUp"),    // Order Forward One
        ("object.arrange.sendBackward", "Cmd+PageDown"),  // Order Backward One
        ("object.arrange.bringToFront", "Shift+PageUp"),  // Order to Front
        ("object.arrange.sendToBack", "Shift+PageDown"),  // Order to Back

        // File & Production
        ("file.place", "Cmd+I"),                   // Import (Ctrl+I)
        ("file.exportAs", "Cmd+E"),                // Export (Ctrl+E)
        ("file.print", "Cmd+P"),                   // Print (Ctrl+P)
        ("panel:properties", "Alt+Enter"),         // Object Properties docker
    ])
}

pub fn coreldraw_keymap() -> Keymap {
    keymap::keymap_with(&coreldraw_overrides())
}

// ---------------------------------------------------------------------------
// SidePanel Docker & Vertical Color Palette Renderers
// ---------------------------------------------------------------------------

/// Palette swatches: CMYK/RGB standard print values.
const COREL_PALETTE: &[[u8; 3]] = &[
    [0, 0, 0],       // Black
    [50, 50, 50],    // 80% Black
    [100, 100, 100], // 60% Black
    [160, 160, 160], // 40% Black
    [210, 210, 210], // 20% Black
    [255, 255, 255], // White
    [0, 160, 230],   // Cyan
    [230, 0, 125],   // Magenta
    [255, 235, 0],   // Yellow
    [230, 30, 30],   // Red
    [0, 160, 70],    // Green
    [30, 60, 170],   // Blue
    [255, 120, 0],   // Orange
    [150, 50, 150],  // Violet
    [100, 60, 30],   // Brown
];

/// Extreme-right vertical Color Palette strip.
/// Left-Click  -> Assigns Fill
/// Right-Click -> Assigns Outline (Stroke)
pub fn show_corel_color_palette(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let swatch_size = vec2(20.0, 16.0);

    egui::ScrollArea::vertical()
        .id_salt("corel_palette_scroll")
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);

            // "None" (transparent) swatch tile at the top (cross/diagonal mark)
            let (rect, resp) = ui.allocate_exact_size(swatch_size, Sense::click());
            ui.painter().rect_filled(rect, 1.0, t.panel);
            ui.painter().rect_stroke(rect, 1.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
            ui.painter().line_segment([rect.left_top(), rect.right_bottom()], Stroke::new(1.2, Color32::RED));
            ui.painter().line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.2, Color32::RED));

            if resp.clicked() {
                let _ = app.run("paint.setFill", json!({ "none": true }));
            }
            if resp.secondary_clicked() {
                let _ = app.run("paint.setStroke", json!({ "none": true }));
            }
            resp.on_hover_text("None (Left-click: No Fill, Right-click: No Outline)");

            // Palette swatches
            for rgb in COREL_PALETTE {
                let col = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                let (rect, resp) = ui.allocate_exact_size(swatch_size, Sense::click());
                ui.painter().rect_filled(rect, 1.0, col);
                ui.painter().rect_stroke(rect, 1.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);

                let hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
                if resp.clicked() {
                    let _ = app.run("paint.setFill", json!({ "color": hex }));
                }
                if resp.secondary_clicked() {
                    let _ = app.run("paint.setStroke", json!({ "color": hex }));
                }
                resp.on_hover_text(format!("{hex}\nLeft-click: Set Fill\nRight-click: Set Outline"));
            }
        });
}

/// Renders dockers inside the right-hand SidePanel for the CorelDRAW Classic layout.
pub fn show_corel_dockers(app: &mut VectorcraftApp, ui: &mut Ui, config: &WorkspaceConfig) {
    let width = ui.available_width();
    let tall = (ui.available_height() - 40.0).max(100.0);

    egui::ScrollArea::vertical()
        .id_salt("corel_dockers_scroll")
        .show(ui, |ui| {
            // Docker 1: Object Properties
            if config.panel_visibility.get("properties").copied().unwrap_or(true) {
                ui.collapsing("Object Properties", |ui| {
                    dock::panel_body(app, ui, "properties", width, tall);
                });
                ui.separator();
            }

            // Docker 2: Object Manager (Layers)
            if config.panel_visibility.get("layers").copied().unwrap_or(true) {
                ui.collapsing("Object Manager", |ui| {
                    dock::panel_body(app, ui, "layers", width, tall);
                });
                ui.separator();
            }

            // Docker 3: Transform
            if config.panel_visibility.get("transform").copied().unwrap_or(true) {
                ui.collapsing("Transform", |ui| {
                    dock::panel_body(app, ui, "transform", width, tall);
                });
                ui.separator();
            }
        });
}

/// Main layout dispatcher for CorelDRAW Classic mode using egui::SidePanel.
pub fn show_corel_workspace(app: &mut VectorcraftApp, ui: &mut egui::Ui, tokens: &Tokens) {
    // 1. Extreme right: Vertical Color Palette
    egui::SidePanel::right("side_color_palette")
        .resizable(false)
        .exact_width(26.0)
        .frame(egui::Frame::NONE.fill(tokens.panel))
        .show_inside(ui, |ui| {
            show_corel_color_palette(app, ui);
        });

    // 2. Right dockers: Object Properties, Object Manager, Transform
    let config_clone = app.ui.workspace_config.clone();
    egui::SidePanel::right("side_corel_dockers")
        .resizable(true)
        .default_width(280.0)
        .width_range(200.0..=500.0)
        .frame(egui::Frame::NONE.fill(tokens.panel))
        .show_inside(ui, |ui| {
            show_corel_dockers(app, ui, &config_clone);
        });

    // 3. Central area: Canvas
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(tokens.pasteboard))
        .show_inside(ui, |ui| {
            app.canvas_area(ui);
        });
}

