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
    for label in ["Exposure…", "Desaturate", "Invert Colors"] {
        assert!(at(&plugins, label).is_some(), "Object › Plug-ins › Color Effects › {label}");
        assert!(at(&["Effect", "Color Effects"], label).is_some(), "Effect › Color Effects › {label}");
    }
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
    for s in ["Exposure", "Offset", "Gamma", "Add as Effect", "Apply as Filter"] {
        assert!(text.contains(s), "{s} in {text}");
    }
    app.run("select.none", json!({})).unwrap();
    assert!(painted_text(&mut app, show).contains("No selection"));
}
