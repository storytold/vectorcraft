use serde_json::json;
use vectorcraft_engine::Session;

use super::{ADJUSTMENTS, ID, Picked, STATE, show};
use crate::panels::set_pstate;
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

/// An app with a selected red rectangle.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app
}

#[test]
fn the_menus_list_the_effects_and_open_the_panel() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let at = |path: &[&str], label: &str| entries.iter().find(|e| e.path == path && e.label == label);
    let plugins = ["Object", "Plug-ins", "Color Effects"];
    let item = at(&plugins, "Color Effects Panel").expect("listed");
    for label in ["Exposure…", "Desaturate", "Duotone…", "Invert Colors", "Vector Halftone…"] {
        assert!(at(&plugins, label).is_some(), "Object › Plug-ins › Color Effects › {label}");
        assert!(at(&["Effect", "Color Effects"], label).is_some(), "Effect › Color Effects › {label}");
    }
    assert!(at(&plugins, "Print Ink Fixes…").is_some(), "Object › Plug-ins › Color Effects › Print Ink Fixes…");
    // The existing submenus are as they were.
    assert!(at(&["Effect", "Color Adjustments"], "Exposure…").is_none());
    menus::invoke(&mut app, "window.panel", item.params.clone());
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
}

#[test]
fn every_adjustment_offered_is_a_colour_adjustment_with_its_defaults() {
    for (i, (id, _, fields)) in ADJUSTMENTS.iter().enumerate() {
        assert!(vectorcraft_effects::is_adjustment(id), "{id}");
        let defaults = vectorcraft_effects::effect_info(id).unwrap().defaults;
        let (picked, params) = Picked { index: i, ..Picked::default() }.params();
        assert_eq!(picked, *id);
        for (key, ..) in fields.iter() {
            assert_eq!(params[key], defaults[key], "{id} {key}");
        }
    }
}

#[test]
fn the_panel_shows_the_values_and_its_buttons() {
    let mut app = app();
    let exposure = ADJUSTMENTS.iter().position(|a| a.0 == "addon.color.exposure").unwrap();
    let text = painted_text(&mut app, |app, ui| {
        set_pstate(ui.ctx(), STATE, Picked { index: exposure, ..Picked::default() });
        show(app, ui);
    });
    for s in ["Exposure", "Offset", "Gamma", "Add as Effect", "Apply as Filter", "Inks", "Duotone…", "Print Ink Fixes…"] {
        assert!(text.contains(s), "{s} in {text}");
    }
    app.run("select.none", json!({})).unwrap();
    assert!(painted_text(&mut app, show).contains("No selection"));
}

#[test]
fn the_halftone_effect_dialog_has_the_command_fields_and_edits_the_effect() {
    let mut app = app();
    let text = |app: &mut VectorcraftApp| painted_text(app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    app.run("effect.dialog", json!({"effect": "addon.color.halftone"})).unwrap();
    let shown = text(&mut app);
    for s in ["Vector Halftone", "Shape", "Frequency", "Angle", "Screens", "Dot Color from Art", "Color", "Invert", "Clip to Art", "Preview"] {
        assert!(shown.lines().any(|l| l == s), "{s} in {shown}");
    }
    assert!(!shown.lines().any(|l| l == "Keep Original"), "the effect keeps its object");
    assert!(app.session.in_interaction(), "the preview runs");
    // The art's colours instead of one: no colour field.
    app.ui.dialog.as_mut().unwrap().fields.insert("fromArt".into(), json!(true));
    assert!(!text(&mut app).lines().any(|l| l == "Color"));
    crate::dialogs::confirm(&mut app).unwrap();
    let st = app.session.active().unwrap();
    let fx = st.doc.node(st.selection.objects[0]).unwrap().appearance.effects.clone();
    assert_eq!((fx.len(), fx[0].id.as_str(), &fx[0].params["fromArt"]), (1, "addon.color.halftone", &json!(true)));
    // The Appearance panel reopens it with its values.
    app.run("effect.dialog", json!({"effect": "addon.color.halftone", "index": 0})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["fromArt"], json!(true));
}

#[test]
fn the_duotone_dialog_has_a_row_per_tone_and_the_curve_of_one() {
    let mut app = app();
    let text = |app: &mut VectorcraftApp| painted_text(app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    app.run("swatch.new", json!({"name": "Ink", "color": "#e07020", "spot": true})).unwrap();
    app.run("effect.dialog", json!({"effect": "addon.color.duotone"})).unwrap();
    let shown = text(&mut app);
    for s in ["Duotone", "Tone 1", "Tone 2", "Tone 3", "Tone 4", "Process Color", "Curve", "Points", "Reset to Linear", "Reset to Inverse"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    assert!(shown.contains("Skip Objects with Spot Colors") && shown.contains("Preview"), "{shown}");
    assert!(app.session.in_interaction(), "the preview runs");
    // Four tones while the dialog is open, the ones after the first off.
    let tones = app.ui.dialog.as_ref().unwrap().fields["tones"].as_array().unwrap().clone();
    assert_eq!(tones.iter().map(|t| t["active"].as_bool().unwrap_or(true)).collect::<Vec<_>>(), [true, false, false, false]);
    // A spot tone with an inverse curve; the second tone's curve shows when picked.
    {
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields["tones"][0] = json!({"color": "#e07020", "swatch": "Ink", "curve": "0,100 100,0", "active": true});
        d.fields.insert("__tone".into(), json!(1));
    }
    assert!(text(&mut app).contains("Ink"), "the swatch is named");
    crate::dialogs::confirm(&mut app).unwrap();
    let st = app.session.active().unwrap();
    let fx = st.doc.node(st.selection.objects[0]).unwrap().appearance.effects.clone();
    assert_eq!((fx.len(), fx[0].id.as_str()), (1, "addon.color.duotone"));
    assert_eq!((fx[0].params["tones"][0]["swatch"].clone(), fx[0].params["tones"].as_array().unwrap().len()), (json!("Ink"), 4));
    assert!(!fx[0].params.as_object().unwrap().contains_key("__tone"), "dialog-only");
    // The Appearance panel reopens it with its values.
    app.run("effect.dialog", json!({"effect": "addon.color.duotone", "index": 0})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["tones"][0]["curve"], json!("0,100 100,0"));
}

#[test]
fn print_ink_fixes_opens_its_options_and_runs_on_ok() {
    let mut app = app();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("paint.setFill", json!({"color": {"c": 100, "m": 100, "y": 100, "k": 100}})).unwrap();
    let entries = menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Color Effects"] && e.label == "Print Ink Fixes…").unwrap();
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(&mut app, &id, p);
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Print Ink Fixes", "Ink Limit", "Exclude Pure Black", "Remove White Overprint", "Overprint Black", "Text Only"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    crate::dialogs::confirm(&mut app).unwrap();
    let st = app.session.active().unwrap();
    let fill = st.doc.node(st.selection.objects[0]).unwrap().appearance.fill_paint().color().unwrap();
    assert_eq!(fill, vectorcraft_color::Color::cmyk(0.75, 0.75, 0.75, 0.75), "scaled to 300 %");
}
