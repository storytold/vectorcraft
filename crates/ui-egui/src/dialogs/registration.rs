//! File → Registration → Summa and Zünd: the cutter registration marks, previewed live on the
//! canvas; OK keeps them as one undo step (`registration.summa`, `registration.zund`). Sizes stay
//! inside each system's range: a value typed outside it snaps to the nearest end.
//!
//! Fields: `system` (`summa` or `zund`) and `preview`; Summa: `mode`, `size`, `gap`, `xDistance`
//! (points); Zünd: `diameter`, `inset` (points) and `fifth` (percent of the left edge up from the
//! bottom-left dot).

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::registration::{self as reg, MM, OposMode};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the registration marks dialog.
pub const KIND: &str = "registrationMarks";

const LABEL_W: f32 = 110.0;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.str("system") == "zund" { tl!("Zünd Registration Dots").into() } else { tl!("Summa OPOS Marks").into() },
    body,
    confirm,
    min_width: 360.0,
    preview: true,
    ..DialogSpec::FORM
};

/// The dialog's fields as it opens for `system` (`summa` or `zund`), Summa in `mode`.
pub fn fields(system: &str, mode: &str) -> Value {
    if system == "zund" {
        json!({"system": "zund", "diameter": reg::ZUND_DOT_MM.2 * MM, "inset": reg::ZUND_GAP_MM * MM, "fifth": reg::ZUND_FIFTH * 100.0, "preview": true})
    } else {
        let mode = OposMode::parse(mode).unwrap_or(OposMode::Opos);
        let size = reg::SUMMA_SIZE_MM.2;
        json!({"system": "summa", "mode": mode.id(), "size": size * MM, "gap": size * 4.0 * MM, "xDistance": reg::SUMMA_X_DISTANCE_MM.0 * MM, "preview": true})
    }
}

/// Open the dialog (`ui.registrationDialog`: `{system, mode?}`).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let system = p.get("system").and_then(Value::as_str).unwrap_or("summa");
    if !matches!(system, "summa" | "zund") {
        return Err(format!("unknown system `{system}` (summa or zund)"));
    }
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("opos");
    app.ui.dialog = Some(Dialog::new(KIND, fields(system, mode)));
    Ok(Value::Null)
}

/// The command and its parameters for the fields.
fn command(d: &Dialog) -> (&'static str, Value) {
    if d.str("system") == "zund" {
        let fifth = d.f64("fifth", reg::ZUND_FIFTH * 100.0) / 100.0;
        ("registration.zund", json!({"diameter": d.f64("diameter", 0.0), "inset": d.f64("inset", 0.0), "fifth": fifth}))
    } else {
        let mode = OposMode::parse(&d.str("mode")).unwrap_or(OposMode::Opos);
        ("registration.summa", json!({"mode": mode.id(), "size": d.f64("size", 0.0), "gap": d.f64("gap", 0.0), "xDistance": d.f64("xDistance", 0.0)}))
    }
}

/// A distance row in mm, kept inside `[lo, hi]` mm.
fn length_row(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, (lo, hi): (f64, f64)) {
    widgets::label_row(ui, label, LABEL_W, |ui| {
        form::length(ui, d, key, Unit::Millimeters, 110.0);
    });
    let v = d.f64(key, lo * MM);
    let clamped = if v.is_finite() { v.clamp(lo * MM, hi * MM) } else { lo * MM };
    if clamped != v {
        d.fields.insert(key.into(), json!(clamped));
    }
}

fn note(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).size(11.0).color(t.text_dim));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    if d.str("system") == "zund" {
        let (lo, hi, _) = reg::ZUND_DOT_MM;
        length_row(ui, d, "diameter", tl!("Dot Diameter"), (lo, hi));
        note(ui, &format!("{} {:.2}–{:.2} mm (0.2–0.4 in)", tl!("Allowed:"), lo, hi));
        ui.add_space(4.0);
        length_row(ui, d, "inset", tl!("Distance from Edge"), (0.0, 1000.0));
        form::slider_w(ui, d, ("fifth", tl!("Fifth Dot"), LABEL_W), 5.0..=95.0, "%", &|x| crate::panels::c32(&vectorcraft_color::Color::gray(x)));
        note(ui, tl!("Four corner dots, plus a fifth up the left edge, this far from the bottom-left dot, to mark the registration corner."));
        note(ui, tl!("The dots go at the corners of the artboard, not the artwork."));
        note(ui, &format!("{} \"{}\" ({})", tl!("Layer:"), reg::ZUND_LAYER, tl!("locked")));
    } else {
        let modes: Vec<(&str, &str)> = OposMode::ALL.iter().map(|m| (m.id(), tl!(m.label()))).collect();
        form::choice(ui, d, "mode", tl!("Method"), (LABEL_W, 160.0), &modes);
        let (lo, hi, _) = reg::SUMMA_SIZE_MM;
        length_row(ui, d, "size", tl!("Mark Size"), (lo, hi));
        note(ui, &format!("{} {lo}–{hi} mm", tl!("Allowed:")));
        ui.add_space(4.0);
        let size_mm = d.f64("size", 0.0) / MM;
        length_row(ui, d, "gap", tl!("Distance from Art"), (size_mm * reg::SUMMA_CLEAR, 1000.0));
        note(ui, tl!("Summa needs white space of 3 to 4 times the mark size around each mark."));
        ui.add_space(4.0);
        length_row(ui, d, "xDistance", tl!("Max X Distance"), (size_mm * (1.0 + reg::SUMMA_CLEAR), reg::SUMMA_X_DISTANCE_MM.1));
        note(ui, &format!("{} \"{}\" ({})", tl!("Layer:"), reg::SUMMA_LAYER, tl!("locked")));
    }
    let (cmd, p) = command(d);
    let label = if cmd == "registration.zund" { "Zünd Registration Dots" } else { "Summa OPOS Marks" };
    form::preview(app, ui, d, label, cmd, p);
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (cmd, p) = command(d);
    form::commit_preview(app, cmd, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(Session::new(), Default::default())
    }

    /// One headless frame of the dialog layer.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn summa_dialog_previews_and_commits_marks() {
        let mut app = app();
        app.run("file.new", json!({"width": 2000, "height": 2000})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 600, "height": 300})).unwrap();
        app.run("ui.registrationDialog", json!({"system": "summa", "mode": "oposXY2"})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.kind.as_str(), d.str("mode")), (KIND, "oposXY2".to_string()));
        frame(&mut app);
        assert!(app.session.in_interaction(), "the marks preview while the dialog is open");
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let doc = &app.session.doc().unwrap().doc;
        let layer = doc.layers.iter().find(|l| l.name.as_deref() == Some(reg::SUMMA_LAYER)).unwrap();
        assert!(layer.children().unwrap().len() >= 5);
    }

    #[test]
    fn zund_dialog_keeps_the_dot_in_range() {
        let mut app = app();
        app.run("file.new", json!({})).unwrap();
        app.run("shape.ellipse", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
        app.run("ui.registrationDialog", json!({"system": "zund"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("diameter".into(), json!(1.0));
        frame(&mut app);
        let d = app.ui.dialog.clone().unwrap();
        assert!((d.f64("diameter", 0.0) - reg::ZUND_DOT_MM.0 * MM).abs() < 1e-9, "snapped up to the smallest dot");
        crate::dialogs::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        let layer = doc.layers.iter().find(|l| l.name.as_deref() == Some(reg::ZUND_LAYER)).unwrap();
        assert_eq!(layer.children().unwrap().len(), 5);
    }
}
