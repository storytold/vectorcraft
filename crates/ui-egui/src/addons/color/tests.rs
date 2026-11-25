use craft::ui::dialog::HeldOk;
use serde_json::json;
use vectorcraft_engine::Session;

use super::{ADJUSTMENTS, Picked};
use crate::VectorcraftApp;

/// An app in a window with a selected red rectangle.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    crate::testing::windowed(app)
}

/// The fields of the hosted dialog open on top and its id.
// PLAN: `app.dialogs()` and the host's `open_dialogs`/`set_json_field`/`confirm_by_id` are taken from the hosted-dialog
// trials of other sets (craft-ui-panels `dialog/host.rs`); widgets-w10 owns the app side of `dialogs()`.
fn top(app: &VectorcraftApp) -> (craft::ui::dialog::DialogId, serde_json::Value) {
    let dialogs = app.dialogs().expect("a window");
    dialogs.with(|host| host.open_dialogs().pop().map(|d| (d.id, d.fields.unwrap_or_default()))).expect("an open dialog")
}

/// Opens the effect dialog of `p` as the Appearance panel and the menus do, and runs its preview.
fn open(app: &mut VectorcraftApp, ctx: &egui::Context, p: serde_json::Value) {
    app.run("effect.dialog", p).unwrap();
    crate::dialogs::host::pump(app, ctx);
}

/// Sets `field` of the top dialog.
fn set(app: &VectorcraftApp, ctx: &egui::Context, field: &str, value: serde_json::Value) {
    let (id, _) = top(app);
    app.dialogs().expect("a window").with(|host| host.set_json_field(ctx, id, field, value)).unwrap();
}

/// OK on the top dialog, then the pump that takes its answer.
fn ok(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let (id, _) = top(app);
    // A held OK reports `false` and files its answer for the app; the dialog closes at its next pass.
    assert!(app.dialogs().expect("a window").with(|host| host.confirm_by_id(ctx, id) || host.held_state(id) == Some(HeldOk::Waiting)));
    crate::dialogs::host::pump(app, ctx);
    crate::tests_synthetic::key_frame(app, ctx, vec![]);
}

/// The effects of the selected object: (id, params).
fn effects(app: &VectorcraftApp) -> Vec<(String, serde_json::Value)> {
    let st = app.session.active().unwrap();
    st.doc.node(st.selection.objects[0]).unwrap().appearance.effects.iter().map(|fx| (fx.id.clone(), fx.params.clone())).collect()
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
fn the_halftone_effect_dialog_has_the_command_fields_and_edits_the_effect() {
    let mut app = app();
    let ctx = app.ui_ctx.clone().expect("a window");
    open(&mut app, &ctx, json!({"effect": "addon.color.halftone"}));
    let (_, fields) = top(&app);
    // Preview is the dialog's own switch, not a value of the effect.
    let shown: std::collections::BTreeSet<&str> = fields.as_object().unwrap().keys().map(String::as_str).filter(|k| *k != "preview").collect();
    let info = vectorcraft_effects::effect_info("addon.color.halftone").unwrap();
    let live: std::collections::BTreeSet<&str> = info.defaults.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(shown, live, "the effect's own values");
    assert!(!shown.contains("keepOriginal"), "the effect keeps its object");
    assert!(app.session.in_interaction(), "the preview runs");
    // The art's colours instead of one.
    set(&app, &ctx, "fromArt", json!(true));
    ok(&mut app, &ctx);
    let fx = effects(&app);
    assert_eq!((fx.len(), fx[0].0.as_str(), &fx[0].1["fromArt"]), (1, "addon.color.halftone", &json!(true)));
    // The Appearance panel reopens it with its values.
    open(&mut app, &ctx, json!({"effect": "addon.color.halftone", "index": 0}));
    assert_eq!(top(&app).1["fromArt"], json!(true));
}

#[test]
fn the_duotone_dialog_has_a_row_per_tone_and_the_curve_of_one() {
    let mut app = app();
    let ctx = app.ui_ctx.clone().expect("a window");
    app.run("swatch.new", json!({"name": "Ink", "color": "#e07020", "spot": true})).unwrap();
    open(&mut app, &ctx, json!({"effect": "addon.color.duotone"}));
    assert!(app.session.in_interaction(), "the preview runs");
    // Four tones while the dialog is open, the ones after the first off.
    let mut tones = top(&app).1["tones"].as_array().unwrap().clone();
    assert_eq!(tones.iter().map(|t| t["active"].as_bool().unwrap_or(true)).collect::<Vec<_>>(), [true, false, false, false]);
    // A spot tone with an inverse curve.
    tones[0] = json!({"color": "#e07020", "swatch": "Ink", "curve": "0,100 100,0", "active": true});
    set(&app, &ctx, "tones", json!(tones));
    ok(&mut app, &ctx);
    let fx = effects(&app);
    assert_eq!((fx.len(), fx[0].0.as_str()), (1, "addon.color.duotone"));
    assert_eq!((fx[0].1["tones"][0]["swatch"].clone(), fx[0].1["tones"].as_array().unwrap().len()), (json!("Ink"), 4));
    assert!(!fx[0].1.as_object().unwrap().contains_key("tone"), "dialog-only");
    // The Appearance panel reopens it with its values.
    open(&mut app, &ctx, json!({"effect": "addon.color.duotone", "index": 0}));
    assert_eq!(top(&app).1["tones"][0]["curve"], json!("0,100 100,0"));
}
