//! Liquify Tool Options (double-click a Liquify tool: Warp, Twirl, Pucker, Bloat, Scallop,
//! Crystallize, Wrinkle): the Global Brush Dimensions the seven tools share, the tool's own options
//! and Show Brush Size. OK sets them with `tool.setOption {tool, values}` (they last across tool
//! switches and are saved with the preferences).
//!
//! Fields: `tool`; `width`, `height` (points), `angle` (degrees), `intensity` (%), `usePressure`;
//! `detail` (1–10); `simplify` (1–100) and `simplifyOn` (Warp, Twirl, Pucker, Bloat); `rate`
//! (Twirl, −180–180°); `complexity` (0–15), `affectAnchors`, `affectIn`, `affectOut` (Scallop,
//! Crystallize, Wrinkle); `horizontal`, `vertical` (Wrinkle, %); `showBrush`.

use serde_json::{Map, Value, json};
use vectorcraft_tools::distort::liquify::LiquifyKind;

use super::document_setup::check;
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Liquify Tool Options.
pub const KIND: &str = "liquifyOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// Width of the label column.
const LABEL_W: f32 = 104.0;

/// The options shown as percentages (the tool keeps them as 0..1).
const PERCENT: [&str; 3] = ["intensity", "horizontal", "vertical"];

/// The options the dialog edits (the tool's options; `tool` names it).
const KEYS: [&str; 16] = [
    "width",
    "height",
    "angle",
    "intensity",
    "usePressure",
    "detail",
    "simplify",
    "simplifyOn",
    "rate",
    "complexity",
    "horizontal",
    "vertical",
    "affectAnchors",
    "affectIn",
    "affectOut",
    "showBrush",
];

fn kind(d: &Dialog) -> LiquifyKind {
    LiquifyKind::parse(&d.str("tool")).unwrap_or(LiquifyKind::Warp)
}

fn heading(d: &Dialog) -> String {
    crate::i18n::fmt(tl!("{tool} Tool Options"), &[("tool", tl!(kind(d).label()))])
}

/// Open the options of Liquify tool `tool` with its current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    let k = LiquifyKind::parse(tool).ok_or_else(|| format!("`{tool}` isn't a Liquify tool"))?;
    let o = app.session.tool_options_of(k.id());
    let mut fields: Map<String, Value> = KEYS.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    for key in PERCENT {
        let v = fields.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        fields.insert(key.into(), json!((v * 100.0).round()));
    }
    fields.insert("tool".into(), json!(k.id()));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({ "dialog": KIND }))
}

/// A slider row for the whole number `d.fields[key]`.
fn slider(ui: &mut egui::Ui, d: &mut Dialog, (key, label): (&str, &str), range: std::ops::RangeInclusive<f64>, suffix: &str) {
    let rail = Tokens::get(ui.ctx()).input_border;
    form::slider_w(ui, d, (key, label, LABEL_W), range, suffix, &|_| rail);
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let k = kind(d);
    let unit = app.session.general_unit();
    widgets::subheader(ui, tl!("Global Brush Dimensions"));
    ui.add_space(4.0);
    for (key, label) in [("width", tl!("Width:")), ("height", tl!("Height:"))] {
        widgets::label_row(ui, label, LABEL_W, |ui| {
            form::length(ui, d, key, unit, 110.0);
        });
    }
    widgets::label_row(ui, tl!("Angle:"), LABEL_W, |ui| {
        if let Some(v) = widgets::range_field(ui, "lq-angle", d.f64("angle", 0.0), -360.0..=360.0, "°", 1, 110.0) {
            d.fields.insert("angle".into(), json!(v));
        }
    });
    let pressure = d.bool("usePressure");
    widgets::label_row(ui, tl!("Intensity:"), LABEL_W, |ui| {
        ui.add_enabled_ui(!pressure, |ui| {
            if let Some(v) = widgets::range_field(ui, "lq-intensity", d.f64("intensity", 50.0), 1.0..=100.0, "%", 0, 110.0) {
                d.fields.insert("intensity".into(), json!(v));
            }
        });
    });
    widgets::label_row(ui, "", LABEL_W, |ui| check(ui, d, "usePressure", tl!("Use Pressure Pen")));
    ui.add_space(10.0);
    widgets::subheader(ui, &crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(k.label()))]));
    ui.add_space(4.0);
    if k == LiquifyKind::Twirl {
        slider(ui, d, ("rate", tl!("Twirl Rate:")), -180.0..=180.0, "°");
    }
    if k == LiquifyKind::Wrinkle {
        slider(ui, d, ("horizontal", tl!("Horizontal:")), 0.0..=100.0, "%");
        slider(ui, d, ("vertical", tl!("Vertical:")), 0.0..=100.0, "%");
    }
    if k.has_affects() {
        slider(ui, d, ("complexity", tl!("Complexity:")), 0.0..=15.0, "");
    }
    slider(ui, d, ("detail", tl!("Detail:")), 1.0..=10.0, "");
    if k.simplifies() {
        widgets::label_row(ui, "", LABEL_W, |ui| check(ui, d, "simplifyOn", tl!("Simplify")));
        let on = d.bool("simplifyOn");
        ui.add_enabled_ui(on, |ui| slider(ui, d, ("simplify", tl!("Simplify:")), 1.0..=100.0, ""));
    } else {
        ui.add_space(4.0);
        widgets::label_row(ui, tl!("Brush Affects:"), LABEL_W, |ui| {
            ui.vertical(|ui| {
                for (key, label) in
                    [("affectAnchors", tl!("Anchor Points")), ("affectIn", tl!("In Tangent Handles")), ("affectOut", tl!("Out Tangent Handles"))]
                {
                    check(ui, d, key, label);
                }
            });
        });
    }
    ui.add_space(10.0);
    check(ui, d, "showBrush", tl!("Show Brush Size"));
    let hint = crate::i18n::fmt(tl!("{shortcut} with the tool sizes the brush"), &[("shortcut", crate::menus::pretty_shortcut("Alt+Drag").as_str())]);
    ui.label(egui::RichText::new(hint).size(11.5).color(Tokens::get(ui.ctx()).text_dim));
    false
}

/// OK: set the tool's options (percentages back to 0..1).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut values: Map<String, Value> = KEYS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
    for key in PERCENT {
        if values.contains_key(key) {
            values.insert(key.into(), json!(d.f64(key, 0.0) / 100.0));
        }
    }
    run_and_close(app, "tool.setOption", json!({ "tool": kind(d).id(), "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    }

    #[test]
    fn each_tool_shows_its_options_and_ok_sets_them() {
        let mut app = app();
        app.select_tool("twirl");
        app.run("tool.options", json!({"tool": "twirl"})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        for label in [
            "Twirl Tool Options",
            "Global Brush Dimensions",
            "Width:",
            "Height:",
            "Angle:",
            "Intensity:",
            "Use Pressure Pen",
            "Twirl Options",
            "Twirl Rate:",
            "Detail:",
            "Simplify",
            "Show Brush Size",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        assert!(!text.contains("Brush Affects:") && !text.contains("Complexity:"), "{text}");
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.f64("intensity", 0.0), d.f64("rate", 0.0), d.bool("showBrush")), (50.0, 40.0, true));
        for (k, v) in
            [("width", json!(80)), ("intensity", json!(30)), ("rate", json!(-120)), ("simplifyOn", json!(false)), ("showBrush", json!(false))]
        {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let o = app.session.tool_options();
        assert_eq!((o["width"].as_f64(), o["intensity"].as_f64(), o["rate"].as_f64()), (Some(80.0), Some(0.3), Some(-120.0)));
        assert_eq!((o["simplifyOn"].as_bool(), o["showBrush"].as_bool()), (Some(false), Some(false)));
        // The brush is shared: the Warp tool has it too, though it is not the active tool.
        app.run("tool.options", json!({"tool": "warp"})).unwrap();
        assert_eq!((app.ui.dialog.as_ref().unwrap().f64("width", 0.0), app.ui.dialog.as_ref().unwrap().f64("intensity", 0.0)), (80.0, 30.0));
    }

    #[test]
    fn scallop_crystallize_and_wrinkle_have_brush_affects_and_no_simplify() {
        let mut app = app();
        for (tool, extra) in [("scallop", None), ("crystallize", None), ("wrinkle", Some("Horizontal:"))] {
            app.run("tool.options", json!({"tool": tool})).unwrap();
            let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
            for label in ["Complexity:", "Detail:", "Brush Affects:", "Anchor Points", "In Tangent Handles", "Out Tangent Handles"] {
                assert!(text.contains(label), "{tool}: {label} in {text}");
            }
            assert!(!text.contains("Simplify"), "{tool}: {text}");
            assert_eq!(text.contains("Horizontal:"), extra.is_some(), "{tool}");
            app.ui.dialog.as_mut().unwrap().fields.insert("affectIn".into(), json!(false));
            super::super::confirm(&mut app).unwrap();
            assert_eq!(app.session.tool_options_of(tool)["affectIn"], json!(false), "{tool}");
        }
    }

    #[test]
    fn type_under_the_brush_is_reported_in_the_status_bar() {
        let mut app = app();
        app.run("text.create", json!({"x": 100, "y": 100, "text": "Hi"})).unwrap();
        app.run("select.set", json!({"ids": []})).unwrap();
        app.select_tool("bloat");
        let view = app.view_info();
        crate::canvas::dispatch(&mut app, &vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Down, 105.0, 95.0), view);
        assert!(app.ui.status.contains("Bloat left 1 object under the brush"), "{}", app.ui.status);
    }
}
