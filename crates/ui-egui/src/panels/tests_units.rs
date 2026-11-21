//! Preferences ▸ Units: the General unit in the Transform panel, the Control bar, dialogs and the
//! Preferences dialog; the Type unit in the Character panel; the change showing in the next frame.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;

use super::tests_appearance::{app_with_rect, run};
use super::*;
use crate::tests::panel_harness::Panel;
use craft::ui::dialog::StackDialog;
use craft::ui::patterns::form_dialogs::HostedParamDialog;

/// Runs menu command `id` as a click on its item does, in a window, so a dialog it opens has a host.
pub(crate) fn run_menu(app: &mut VectorcraftApp, id: &str) {
    if app.ui_ctx.is_none() {
        app.adopt_context(&crate::testing::ui_context());
    }
    crate::menus::invoke(app, id, json!({}));
}

/// `f` over the open hosted dialog, with the window's context; `None` when none is open.
fn with_dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut HostedParamDialog, &egui::Context) -> R) -> Option<R> {
    let ctx = app.ui_ctx.clone()?;
    app.dialogs()?.with(|host| host.get_mut::<HostedParamDialog>().map(|d| f(d, &ctx)))
}

/// The open hosted dialog's field `name`, as the dialog shows it.
pub(crate) fn dialog_field(app: &mut VectorcraftApp, name: &str) -> Option<Value> {
    with_dialog(app, |d, _| d.json_fields()?.get(name).cloned()).flatten()
}

/// Sets the open hosted dialog's field `name`, as typing in it does.
pub(crate) fn set_dialog_field(app: &mut VectorcraftApp, name: &str, value: Value) {
    let set = with_dialog(app, |d, ctx| d.set_json_field(ctx, name, value));
    assert_eq!(set, Some(Ok(())), "{name}");
}

/// OK in the open hosted dialog; the app's dialog pass runs what it answered.
pub(crate) fn confirm_dialog(app: &mut VectorcraftApp) {
    close_dialog(app, true);
}

/// Cancel in the open hosted dialog.
pub(crate) fn cancel_dialog(app: &mut VectorcraftApp) {
    close_dialog(app, false);
}

fn close_dialog(app: &mut VectorcraftApp, ok: bool) {
    let (Some(ctx), Some(dialogs)) = (app.ui_ctx.clone(), app.dialogs()) else { panic!("no window") };
    dialogs.with(|host| {
        let id = host.get::<HostedParamDialog>().map(StackDialog::id).expect("a hosted dialog is open");
        if ok { host.confirm_by_id(&ctx, id) } else { host.cancel_by_id(&ctx, id) }
    });
    crate::dialogs::host::pump(app, &ctx);
}

fn bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    transform::object_bar_fields(app, ui);
}

/// The 100 pt square of [`app_with_rect`] in millimetres.
fn side_mm() -> String {
    Unit::Millimeters.format(100.0)
}

fn general_mm(app: &mut VectorcraftApp) {
    app.run("prefs.set", json!({"key": "unitsGeneral", "value": "Millimeters"})).unwrap();
}

#[test]
fn panels_show_millimetres_after_the_preference_changes() {
    let mut p = Panel::new(app_with_rect(), transform::show);
    assert_eq!(side_mm(), "35.2778 mm");
    assert_eq!(p.value("Width"), Some(Unit::Points.format(100.0)), "points first");
    general_mm(&mut p.app());
    // The next frame: no restart, nothing cached.
    p.frame();
    assert_eq!(p.value("Width"), Some(side_mm()), "Transform panel");
    let mut b = Panel::new(p.into_app(), bar);
    assert_eq!(b.value("Width"), Some(side_mm()), "Control bar");
    // Document Setup back to points: everything follows the document.
    run(&mut b.app(), "document.setUnits", json!({"units": "Points"}));
    b.frame();
    assert_eq!(b.value("Width"), Some(Unit::Points.format(100.0)));
}

#[test]
fn dialog_lengths_show_in_the_general_unit() {
    let mut app = app_with_rect();
    general_mm(&mut app);
    run_menu(&mut app, "object.move");
    assert_eq!(dialog_field(&mut app, "dx"), Some(json!(Unit::Millimeters.format(0.0))), "Move");
    // A typed unit wins; the move is in points.
    set_dialog_field(&mut app, "dx", json!("10 mm"));
    confirm_dialog(&mut app);
    let st = app.session.active().unwrap();
    let b = st.doc.bounds_of(&st.selection.objects, false).unwrap();
    assert!((b.x0 - (10.0 + 10.0 * 72.0 / 25.4)).abs() < 1e-6, "{b:?}");
    // A shape tool's size dialog and an effect's distances.
    crate::dialogs::open_tool_dialog(&mut app, "rectangle", json!({"x": 0, "y": 0}));
    assert_eq!(dialog_field(&mut app, "width"), Some(json!(side_mm())), "Rectangle");
    cancel_dialog(&mut app);
    app.run("effect.dialog", json!({"effect": "stylize.dropShadow"})).unwrap();
    let x = vectorcraft_effects::default_params("stylize.dropShadow").unwrap()["x"].as_f64().unwrap();
    assert_eq!(dialog_field(&mut app, "x"), Some(json!(Unit::Millimeters.format(x))), "Drop Shadow");
    cancel_dialog(&mut app);
}

#[test]
fn type_sizes_follow_units_type() {
    let mut app = app_with_rect();
    let id = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 24}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    general_mm(&mut app);
    let mut p = Panel::new(app, character::show);
    // PLAN: assumed `character::type_unit(&VectorcraftApp) -> Unit` (type-w10 note 12) and a "Font Size" control.
    assert_eq!(character::type_unit(&p.app()), Unit::Points, "General leaves type alone");
    assert_eq!(p.value("Font Size"), Some(Unit::Points.format(24.0)));
    p.app().run("prefs.set", json!({"key": "unitsType", "value": "inches"})).unwrap();
    p.frame();
    assert_eq!(character::type_unit(&p.app()), Unit::Inches);
    assert_eq!(p.value("Font Size"), Some(Unit::Inches.format(24.0)));
}

#[test]
fn the_preferences_dialog_shows_the_documents_units() {
    let mut app = app_with_rect();
    run(&mut app, "document.setUnits", json!({"units": "Millimeters"}));
    crate::prefs_dialog::open(&mut app, Some("General"));
    assert_eq!(crate::prefs_dialog::fields(&app).unwrap()["unitsGeneral"], json!("millimeters"));
    let inc = app.session.prefs.keyboard_increment;
    let mut p = Panel::new(app, |app, ui| crate::prefs_dialog::show(app, ui));
    assert_eq!(p.value("Keyboard Increment"), Some(Unit::Millimeters.format(inc)), "Keyboard Increment in mm");
    // Choosing another General unit in the dialog shows at once, and OK applies it.
    crate::prefs_dialog::set_field(&mut p.app(), "unitsGeneral", json!("inches")).unwrap();
    p.frame();
    assert_eq!(p.value("Keyboard Increment"), Some(Unit::Inches.format(inc)));
    crate::prefs_dialog::confirm(&mut p.app()).unwrap();
    let app = p.app();
    assert_eq!(app.session.active().unwrap().doc.units, Unit::Inches);
    assert_eq!(app.session.prefs.units_general, "inches");
}

#[test]
fn ruler_labels_have_the_steps_decimals() {
    use crate::canvas::ruler_label;
    assert_eq!(ruler_label(100.0000001, 100.0), "100");
    assert_eq!(ruler_label(0.30000000004, 0.1), "0.3");
    assert_eq!(ruler_label(1.25, 0.05), "1.25");
    assert_eq!(ruler_label(-0.0000001, 0.5), "0");
    assert_eq!(ruler_label(-2.5, 0.5), "-2.5");
    // An inch ruler's binary steps keep their exact value: rounding the step's magnitude wrote
    // 1/16 as "0.06" and 1/8 as "0.13" (#2827).
    assert_eq!(ruler_label(0.0625, 0.0625), "0.0625");
    assert_eq!(ruler_label(0.125, 0.125), "0.125");
    assert_eq!(ruler_label(0.75, 0.25), "0.75");
    assert_eq!(ruler_label(1.0, 0.0625), "1");
    assert_eq!(ruler_label(1.1875, 0.0625), "1.1875");
}
