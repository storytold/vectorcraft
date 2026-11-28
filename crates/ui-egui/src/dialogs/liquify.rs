//! Liquify Tool Options (double-click a Liquify tool: Warp, Twirl, Pucker, Bloat, Scallop,
//! Crystallize, Wrinkle): the Global Brush Dimensions the seven tools share, the tool's own options
//! and Show Brush Size. OK sets them with `tool.setOption {tool, values}` (they last across tool
//! switches and are saved with the preferences).
//!
//! Fields: `width`, `height` (points), `angle` (degrees), `intensity` (%), `usePressure`;
//! `detail` (1–10); `simplify` (1–100) and `simplifyOn` (Warp, Twirl, Pucker, Bloat); `rate`
//! (Twirl, −180–180°); `complexity` (0–15), `affectAnchors`, `affectIn`, `affectOut` (Scallop,
//! Crystallize, Wrinkle); `horizontal`, `vertical` (Wrinkle, %); `showBrush`.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::value_drag::SliderScale;
use serde_json::{Map, Value, json};
use vectorcraft_tools::distort::liquify::LiquifyKind;

use super::{form, host, tool_options};
use crate::VectorcraftApp;

/// The dialog kind of the Liquify Tool Options.
pub const KIND: &str = "liquifyOptions";

/// The options shown as percentages (the tool keeps them as 0..1).
const PERCENT: [&str; 3] = ["intensity", "horizontal", "vertical"];

/// The options the dialog edits (the tool's options).
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

/// A slider row over `min..=max` with a unit.
fn slider(key: &str, label: &'static str, (min, max, default): (f64, f64, f64), unit: UnitKind) -> Param {
    Param::new(key, label, ParamKind::range(min, max, default, SliderScale::Linear)).unit(unit)
}

/// A check box row.
fn check(key: &str, label: &'static str, default: bool) -> Param {
    Param::new(key, label, ParamKind::bool(default))
}

/// The rows of the dialog of tool `k`, in the order the tool's options are laid out.
fn params(k: LiquifyKind, lengths: LengthUnits) -> Vec<Param> {
    let length = |key: &str, label: &'static str| Param::number(key, label, 50.0, 1.0, 0).length(lengths);
    let mut rows = vec![
        Param::section(tl!("Global Brush Dimensions")),
        length("width", tl!("Width:")),
        length("height", tl!("Height:")),
        Param::number("angle", tl!("Angle:"), 0.0, 1.0, 1).unit(UnitKind::Degrees).bounds(Some(-360.0), Some(360.0)),
        Param::number("intensity", tl!("Intensity:"), 50.0, 1.0, 0)
            .unit(UnitKind::Percent)
            .bounds(Some(1.0), Some(100.0))
            .enabled_when("usePressure", false),
        check("usePressure", tl!("Use Pressure Pen"), false),
        Param::section(crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(k.label()))])),
    ];
    if k == LiquifyKind::Twirl {
        rows.push(slider("rate", tl!("Twirl Rate:"), (-180.0, 180.0, 40.0), UnitKind::Degrees));
    }
    if k == LiquifyKind::Wrinkle {
        rows.push(slider("horizontal", tl!("Horizontal:"), (0.0, 100.0, 50.0), UnitKind::Percent));
        rows.push(slider("vertical", tl!("Vertical:"), (0.0, 100.0, 50.0), UnitKind::Percent));
    }
    if k.has_affects() {
        rows.push(slider("complexity", tl!("Complexity:"), (0.0, 15.0, 4.0), UnitKind::Times));
    }
    rows.push(slider("detail", tl!("Detail:"), (1.0, 10.0, 2.0), UnitKind::Times));
    if k.simplifies() {
        rows.push(check("simplifyOn", tl!("Simplify"), true));
        rows.push(slider("simplify", tl!("Simplify:"), (1.0, 100.0, 50.0), UnitKind::Times).enabled_when("simplifyOn", true));
    } else {
        rows.push(check("affectAnchors", tl!("Anchor Points"), true));
        rows.push(check("affectIn", tl!("In Tangent Handles"), false));
        rows.push(check("affectOut", tl!("Out Tangent Handles"), false));
    }
    rows.push(check("showBrush", tl!("Show Brush Size"), true));
    let hint = crate::i18n::fmt(tl!("{shortcut} with the tool sizes the brush"), &[("shortcut", crate::keymap::chord_text("Alt+Drag").as_str())]);
    rows.push(Param::read_only_text("hint", "", hint).full_width());
    rows
}

/// Open the options of Liquify tool `tool` with its current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    let k = LiquifyKind::parse(tool).ok_or_else(|| format!("`{tool}` isn't a Liquify tool"))?;
    let params = params(k, crate::canvas::general_lengths(app)); // PLAN: assumed path of cv.B5b
    let o = app.session.tool_options_of(k.id());
    let mut values: Map<String, Value> = KEYS.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    for key in PERCENT {
        if let Some(v) = values.get(key).and_then(Value::as_f64) {
            values.insert(key.into(), json!((v * 100.0).round()));
        }
    }
    let fields = form::fields_of(&params, &values);
    let title = crate::i18n::fmt(tl!("{tool} Tool Options"), &[("tool", tl!(k.label()))]);
    let shown = params.clone();
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| {
        let mut values = form::json_of(&params, &result.fields);
        for key in PERCENT {
            if let Some(v) = values.get(key).and_then(Value::as_f64) {
                values.insert(key.into(), json!(v / 100.0));
            }
        }
        tool_options::set_options(app, k.id(), values)
    };
    let opened = app.ui_ctx.clone().and_then(|ctx| {
        let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |f| ParamDialog::new(KIND, title.clone(), shown.clone()).values(Bind::bound(f)));
        host::open_held(app, dialog, ok)
    });
    if opened.is_none() {
        return Err(tl!("No window to show the dialog in.").to_string());
    }
    Ok(json!({ "dialog": KIND }))
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
