//! Magic Wand panel: which attributes the Magic Wand tool compares, with a tolerance for each.
//! The settings live in the engine (`magicWand.set` / `magicWand.options`), so they survive tool
//! switches and agents can set them too.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::wand::color_tolerance_max;

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

fn options(app: &mut VectorcraftApp) -> Value {
    app.session.execute("magicWand.options", &json!({})).unwrap_or_default()
}

/// A tolerance: a plain number with a suffix, or a stroke weight (in the Stroke unit).
#[derive(Clone, Copy)]
enum Tol {
    Plain(&'static str),
    Weight,
}

/// One row: checkbox + tolerance field.
fn row(app: &mut VectorcraftApp, ui: &mut Ui, o: &Value, label: &str, flag: &str, tol: Option<(&str, Tol, f64)>) {
    let on = o[flag].as_bool().unwrap_or(false);
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        if widgets::check(ui, tl!(label), on, true) {
            app.run("magicWand.set", json!({ flag: !on })).ok();
        }
        if let Some((key, kind, max)) = tol {
            let stroke_unit = app.session.stroke_unit();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(on, |ui| {
                    let v = o[key].as_f64().unwrap_or(0.0);
                    let n = match kind {
                        Tol::Plain(suffix) => widgets::spin_plain(ui, ("wand", key), v, suffix, 0, 64.0, 1.0, 0.0, &[]),
                        Tol::Weight => widgets::spin_field(ui, ("wand", key), Some(v), stroke_unit, 64.0, 1.0, 0.0, &[]),
                    };
                    if let Some(n) = n {
                        app.run("magicWand.set", json!({ key: n.clamp(0.0, max) })).ok();
                    }
                });
                widgets::dim_label(ui, tl!("Tolerance:"));
            });
        }
    });
}

/// The largest color tolerance in the active document: 100 in a CMYK document, else 255.
fn color_max(app: &VectorcraftApp) -> f64 {
    color_tolerance_max(app.session.active().map(|d| d.doc.color_mode).unwrap_or_default())
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let o = options(app);
    let cmax = color_max(app);
    row(app, ui, &o, tl!("Fill Color"), "fillColor", Some(("fillTolerance", Tol::Plain(""), cmax)));
    if !pstate::<bool>(ui.ctx(), "wand-hide-stroke") {
        widgets::divider(ui);
        row(app, ui, &o, tl!("Stroke Color"), "strokeColor", Some(("strokeTolerance", Tol::Plain(""), cmax)));
        row(app, ui, &o, tl!("Stroke Weight"), "strokeWeight", Some(("weightTolerance", Tol::Weight, 1000.0)));
    }
    if !pstate::<bool>(ui.ctx(), "wand-hide-transparency") {
        widgets::divider(ui);
        row(app, ui, &o, tl!("Opacity"), "opacity", Some(("opacityTolerance", Tol::Plain("%"), 100.0)));
        row(app, ui, &o, tl!("Blending Mode"), "blendingMode", None);
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    for (label, key) in [(tl!("Stroke Options"), "wand-hide-stroke"), (tl!("Transparency Options"), "wand-hide-transparency")] {
        let hidden: bool = pstate(ui.ctx(), key);
        if menu_item(ui, &format!("{} {label}", if hidden { tl!("Show") } else { tl!("Hide") }), true, false) {
            set_pstate(ui.ctx(), key, !hidden);
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Reset"), true, false) {
        app.run("magicWand.set", json!({ "reset": true })).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_and_menu_draw_headless() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        for _ in 0..2 {
            let ctx = egui::Context::default();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(&mut app, ui);
                menu(&mut app, ui);
            });
            out.textures_delta.clear();
        }
    }

    #[test]
    fn color_tolerance_fields_go_to_100_in_a_cmyk_document() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert_eq!(color_max(&app), 255.0);
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        assert_eq!(color_max(&app), 255.0);
        app.session.execute("magicWand.set", &json!({"fillTolerance": 200})).unwrap();
        app.session.execute("file.new", &json!({"width": 100, "height": 100, "colorMode": "cmyk"})).unwrap();
        assert_eq!(color_max(&app), 100.0);
        assert_eq!(options(&mut app)["fillTolerance"], 100.0);
    }
}
