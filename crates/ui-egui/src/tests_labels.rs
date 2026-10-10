//! Neutral wording in menus and panels: what users read names VectorCraft's own features.

use egui::epaint::Shape;
use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::menus::{self, Item};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app
}

/// Every string painted by `f` in a headless frame (app fonts installed), one per line. The second
/// of two frames: windows size themselves invisibly in their first.
pub(crate) fn painted_text(app: &mut VectorcraftApp, mut f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui)) -> String {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.run_ui(egui::RawInput::default(), |ui| f(app, ui)).textures_delta.clear();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(app, ui));
    out.textures_delta.clear();
    shapes_text(&out)
}

/// Every string a frame painted, one per line.
pub(crate) fn shapes_text(out: &egui::FullOutput) -> String {
    fn collect(s: &Shape, out: &mut String) {
        match s {
            Shape::Text(t) => {
                out.push_str(t.galley.text());
                out.push('\n');
            }
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut text = String::new();
    for c in &out.shapes {
        collect(&c.shape, &mut text);
    }
    text
}

#[test]
fn effect_menu_has_a_vector_effects_section() {
    let tree = menus::menu_tree();
    let (_, effect) = tree.iter().find(|(t, _)| *t == "Effect").expect("Effect menu");
    assert!(effect.iter().any(|i| matches!(i, Item::Header("Vector Effects"))));
}

#[test]
fn effect_menu_lists_the_pixelate_filters_between_blur_and_sharpen() {
    let app = app();
    let entries = menus::menu_entries(&app);
    let pixelate: Vec<_> = entries.iter().filter(|e| e.path == ["Effect", "Pixelate"]).collect();
    let labels: Vec<&str> = pixelate.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Color Halftone…", "Crystallize…", "Mezzotint…", "Pointillize…"]);
    assert!(pixelate.iter().all(|e| e.command.as_deref() == Some("effect.dialog")));
    let raster: Vec<&str> =
        entries.iter().filter_map(|e| e.path.get(1).filter(|_| e.path.len() == 2 && e.path[0] == "Effect")).map(String::as_str).collect();
    let at = |name: &str| raster.iter().position(|s| *s == name).unwrap();
    assert!(at("Blur") < at("Pixelate") && at("Pixelate") < at("Sharpen"));
}

/// Effect › Video, after Sharpen: De-Interlace opens its dialog, NTSC Colors (no options) applies.
#[test]
fn effect_menu_lists_the_video_filters_last() {
    let app = app();
    let entries = menus::menu_entries(&app);
    let video: Vec<_> = entries.iter().filter(|e| e.path == ["Effect", "Video"]).collect();
    let items: Vec<(&str, Option<&str>)> = video.iter().map(|e| (e.label.as_str(), e.command.as_deref())).collect();
    assert_eq!(items, [("De-Interlace…", Some("effect.dialog")), ("NTSC Colors", Some("effect.apply"))]);
    let raster: Vec<&str> =
        entries.iter().filter_map(|e| e.path.get(1).filter(|_| e.path.len() == 2 && e.path[0] == "Effect")).map(String::as_str).collect();
    let at = |name: &str| raster.iter().position(|s| *s == name).unwrap();
    assert!(at("Sharpen") < at("Video"));
}

/// Effect › Distort, between Blur and Pixelate: each filter opens its dialog.
#[test]
fn effect_menu_lists_the_distort_filters_between_blur_and_pixelate() {
    let app = app();
    let entries = menus::menu_entries(&app);
    let distort: Vec<_> = entries.iter().filter(|e| e.path == ["Effect", "Distort"]).collect();
    let labels: Vec<&str> = distort.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Diffuse Glow…", "Glass…", "Ocean Ripple…"]);
    assert!(distort.iter().all(|e| e.command.as_deref() == Some("effect.dialog")));
    let raster: Vec<&str> =
        entries.iter().filter_map(|e| e.path.get(1).filter(|_| e.path.len() == 2 && e.path[0] == "Effect")).map(String::as_str).collect();
    let at = |name: &str| raster.iter().position(|s| *s == name).unwrap();
    assert!(at("Blur") < at("Distort") && at("Distort") < at("Pixelate"));
    // The vector Distort & Transform stays its own submenu.
    assert!(raster.contains(&"Distort & Transform"));
}

/// Effect › Brush Strokes, between Blur and Distort: each filter opens its dialog.
#[test]
fn effect_menu_lists_the_brush_strokes_filters_between_blur_and_distort() {
    let app = app();
    let entries = menus::menu_entries(&app);
    let strokes: Vec<_> = entries.iter().filter(|e| e.path == ["Effect", "Brush Strokes"]).collect();
    let labels: Vec<&str> = strokes.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Accented Edges…", "Angled Strokes…", "Crosshatch…", "Dark Strokes…", "Ink Outlines…", "Spatter…", "Sprayed Strokes…", "Sumi-e…"]
    );
    assert!(strokes.iter().all(|e| e.command.as_deref() == Some("effect.dialog")));
    let raster: Vec<&str> =
        entries.iter().filter_map(|e| e.path.get(1).filter(|_| e.path.len() == 2 && e.path[0] == "Effect")).map(String::as_str).collect();
    let at = |name: &str| raster.iter().position(|s| *s == name).unwrap();
    assert!(at("Blur") < at("Brush Strokes") && at("Brush Strokes") < at("Distort"));
}

/// Effect › Texture, between Sharpen and Video: each filter opens its dialog.
#[test]
fn effect_menu_lists_the_texture_filters_between_sharpen_and_video() {
    let app = app();
    let entries = menus::menu_entries(&app);
    let texture: Vec<_> = entries.iter().filter(|e| e.path == ["Effect", "Texture"]).collect();
    let labels: Vec<&str> = texture.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Craquelure…", "Grain…", "Mosaic Tiles…", "Patchwork…", "Stained Glass…", "Texturizer…"]);
    assert!(texture.iter().all(|e| e.command.as_deref() == Some("effect.dialog")));
    let raster: Vec<&str> =
        entries.iter().filter_map(|e| e.path.get(1).filter(|_| e.path.len() == 2 && e.path[0] == "Effect")).map(String::as_str).collect();
    let at = |name: &str| raster.iter().position(|s| *s == name).unwrap();
    assert!(at("Sharpen") < at("Texture") && at("Texture") < at("Video"));
}

#[test]
fn library_submenus_are_disabled_placeholders() {
    let app = app();
    let entries = menus::menu_entries(&app);
    for lib in ["Brush Libraries", "Symbol Libraries"] {
        let items: Vec<_> = entries.iter().filter(|e| e.path == ["Window", lib]).collect();
        let labels: Vec<&str> = items.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["Built-in Libraries", "User Defined", "Other Library…"], "{lib}");
        assert!(items.iter().all(|e| !e.enabled && e.command.is_none()), "{lib}");
    }
}

#[test]
fn stroke_profiles_have_descriptive_names() {
    let labels: Vec<&str> = vectorcraft_doc::WidthProfile::PRESETS.iter().map(|p| p.label).collect();
    assert_eq!(labels, ["Uniform", "Lens", "Taper Start", "Taper End", "Pinch", "Teardrop", "Wave"]);
}

#[test]
fn swatches_menu_offers_save_swatch_library() {
    let mut app = app();
    let text = painted_text(&mut app, crate::panels::swatches::menu);
    assert!(text.contains("Save Swatch Library…"), "{text}");
    assert!(!text.contains("ASE"), "{text}");
}

#[test]
fn edit_menu_lists_pdf_presets() {
    let app = app();
    // Implemented (M4.44): enabled, and it opens Edit → PDF Presets.
    assert!(
        menus::menu_entries(&app)
            .iter()
            .any(|e| e.path == ["Edit"] && e.label == "PDF Presets…" && e.enabled && e.command.as_deref() == Some("ui.pdfPresetsDialog"))
    );
}

#[test]
fn clipboard_preferences_name_the_legacy_format_neutrally() {
    let label = |key: &str| vectorcraft_engine::cmd::prefscmds::PREF_SPECS.iter().find(|p| p.key == key).unwrap().label;
    assert_eq!(label("copyAicb"), "Legacy vector clipboard (no transparency)");
    assert_eq!(label("aicbMode"), "Legacy vector clipboard");
}

#[test]
fn shortcut_set_old_name_is_accepted() {
    use crate::shortcut_editor::{PRESETS, import_json};
    let (old, _) = crate::shortcut_editor::LEGACY_PRESETS[0];
    assert_eq!(PRESETS, ["VectorCraft Defaults", "Classic Defaults"]);
    // UI preferences saved by an earlier version load (and save again) under the current name.
    let ui: crate::state::UiState = serde_json::from_value(json!({ "shortcut_set": old })).unwrap();
    assert_eq!(ui.shortcut_set, "Classic Defaults");
    let saved = serde_json::to_value(&ui).unwrap();
    assert_eq!(saved["shortcut_set"], "Classic Defaults");
    let back: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(back.shortcut_set, "Classic Defaults");
    let default: crate::state::UiState = serde_json::from_value(json!({})).unwrap();
    assert_eq!(default.shortcut_set, PRESETS[0]);
    // The command and imported sets take the old name too.
    let mut app = app();
    app.run("shortcuts.preset", json!({ "name": old })).unwrap();
    assert_eq!(app.ui.shortcut_set, "Classic Defaults");
    app.run("shortcuts.preset", json!({ "name": "VectorCraft Defaults" })).unwrap();
    assert_eq!(app.ui.shortcut_set, "VectorCraft Defaults");
    assert!(app.run("shortcuts.preset", json!({ "name": "Nope" })).is_err());
    let (set, _) = import_json(&json!({ "set": old, "overrides": {} })).unwrap();
    assert_eq!(set, "Classic Defaults");
}
