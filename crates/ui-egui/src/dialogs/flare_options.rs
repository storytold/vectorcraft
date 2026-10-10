//! Flare Tool Options (double-click the Flare tool, or click with it): the options the Flare tool
//! keeps, which draw the next flare dragged out, last across tool switches and are saved with the
//! preferences. OK sets them with `tool.setOption {tool: "flare", values}`; opened by a click, it
//! then draws a flare there with them (`shape.flare`, on the active perspective plane while the
//! grid shows).
//!
//! Fields: the options (`vectorcraft_tools::extra::FLARE_OPTIONS`): `diameter` and `pathLength`
//! (points), `opacity`, `brightness`, `growth`, `fuzziness`, `longest`, `rayFuzziness` and
//! `largest` (%), `rays` and `rings` (counts), `direction` (degrees); `raysOn` and `ringsOn`; and
//! `x`, `y` where a click opened it.

use serde_json::{Map, Value, json};
use vectorcraft_geom::Point;
use vectorcraft_tools::extra::{FLARE_OPTIONS, flare_params};

use super::document_setup::check;
use super::{DialogResult, DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Flare Tool Options.
pub const KIND: &str = "flareOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Flare Tool Options").into(), body, confirm, min_width: 440.0, ..DialogSpec::FORM };

/// Width of the label column and of the fields.
const LABEL_W: f32 = 78.0;
const FIELD_W: f32 = 90.0;

/// Open the Flare Tool Options with the tool's values; `at`: where a click opened them, to draw a
/// flare there on OK.
pub fn open(app: &mut VectorcraftApp, at: Option<Point>) -> Value {
    let mut fields = match app.session.tool_options_of("flare") {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    if let Some(p) = at {
        fields.insert("x".into(), json!(p.x));
        fields.insert("y".into(), json!(p.y));
    }
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

/// A row for option `key`, kept within its range: a distance in `unit`, else a number.
fn field(ui: &mut egui::Ui, d: &mut Dialog, unit: vectorcraft_doc::Unit, (key, label): (&str, &str)) {
    let Some(&(_, default, lo, hi)) = FLARE_OPTIONS.iter().find(|(k, ..)| *k == key) else { return };
    widgets::label_row(ui, label, LABEL_W, |ui| match key {
        "diameter" | "pathLength" => {
            if form::length(ui, d, key, unit, FIELD_W) {
                let v = d.f64(key, default).clamp(lo, hi);
                d.fields.insert(key.into(), json!(v));
            }
        }
        _ => {
            let suffix = match key {
                "rays" | "rings" => "",
                "direction" => "°",
                _ => "%",
            };
            if let Some(v) = widgets::range_field(ui, ("flare", key), d.f64(key, default), lo..=hi, suffix, 0, FIELD_W) {
                d.fields.insert(key.into(), json!(v));
            }
        }
    });
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    // Center and Halo, then Rays and Rings, side by side as two rows of groups.
    egui::Grid::new("flare-groups").num_columns(2).spacing([28.0, 14.0]).show(ui, |ui| {
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Center"));
            for f in [("diameter", tl!("Diameter:")), ("opacity", tl!("Opacity:")), ("brightness", tl!("Brightness:"))] {
                field(ui, d, unit, f);
            }
        });
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Halo"));
            for f in [("growth", tl!("Growth:")), ("fuzziness", tl!("Fuzziness:"))] {
                field(ui, d, unit, f);
            }
        });
        ui.end_row();
        ui.vertical(|ui| {
            check(ui, d, "raysOn", tl!("Rays"));
            let on = d.bool("raysOn");
            ui.add_enabled_ui(on, |ui| {
                for f in [("rays", tl!("Number:")), ("longest", tl!("Longest:")), ("rayFuzziness", tl!("Fuzziness:"))] {
                    field(ui, d, unit, f);
                }
            });
        });
        ui.vertical(|ui| {
            check(ui, d, "ringsOn", tl!("Rings"));
            let on = d.bool("ringsOn");
            ui.add_enabled_ui(on, |ui| {
                for f in [("pathLength", tl!("Path:")), ("rings", tl!("Number:")), ("largest", tl!("Largest:")), ("direction", tl!("Direction:"))] {
                    field(ui, d, unit, f);
                }
            });
        });
        ui.end_row();
    });
    false
}

/// OK: set the tool's options; opened by a click, draw a flare there with them.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let mut values: Map<String, Value> =
        FLARE_OPTIONS.iter().filter(|(k, ..)| d.fields.contains_key(*k)).map(|(k, default, ..)| (k.to_string(), json!(d.f64(k, *default)))).collect();
    for key in ["raysOn", "ringsOn"] {
        values.insert(key.into(), json!(d.bool(key)));
    }
    let at = d.fields.get("x").and_then(Value::as_f64).zip(d.fields.get("y").and_then(Value::as_f64)).map(|(x, y)| Point::new(x, y));
    let options = run_and_close(app, "tool.setOption", json!({ "tool": "flare", "values": values }))?;
    let Some(at) = at else { return Ok(options) };
    let params = flare_params(&options, at);
    match vectorcraft_engine::perspective_click(&app.session, "shape.flare", &params, at) {
        Some((id, p)) => app.run(&id, p),
        None => app.run("shape.flare", params),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 600, "height": 600})).unwrap();
        app
    }

    /// The labels the open dialog paints.
    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    /// The children of the newest flare (a group named Flare), by name.
    fn flare_parts(app: &VectorcraftApp) -> Vec<String> {
        let doc = &app.session.doc().unwrap().doc;
        let flare = doc.layers[0].children().unwrap().iter().rev().find(|n| n.name.as_deref() == Some("Flare")).expect("a flare");
        let NodeKind::Group { children, .. } = &flare.kind else { panic!("a flare is a group") };
        children.iter().filter_map(|c| c.name.clone()).collect()
    }

    #[test]
    fn the_toolbar_opens_the_options_and_ok_keeps_them_for_the_next_drag() {
        let mut app = app();
        app.run("tool.options", json!({"tool": "flare"})).unwrap();
        let text = painted(&mut app);
        for label in ["Flare Tool Options", "Center", "Diameter:", "Halo", "Growth:", "Rays", "Longest:", "Rings", "Path:", "Largest:", "Direction:"]
        {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.f64("diameter", 0.0), d.f64("rays", 0.0), d.bool("raysOn"), d.f64("direction", 0.0)), (100.0, 15.0, true, 45.0));
        for (k, v) in [("rays", json!(4)), ("ringsOn", json!(false)), ("growth", json!(80))] {
            d.fields.insert(k.into(), v);
        }
        let objects = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(objects(&app), 0, "OK from the toolbar draws nothing");
        let o = app.session.tool_options_of("flare");
        assert_eq!((o["rays"].as_u64(), o["ringsOn"].as_bool(), o["growth"].as_f64()), (Some(4), Some(false), Some(80.0)));
        // A drag draws with them: four rays, no rings.
        app.select_tool("flare");
        let view = app.view_info();
        for (kind, x, y) in [(PointerKind::Down, 200.0, 200.0), (PointerKind::Drag, 240.0, 200.0), (PointerKind::Up, 240.0, 200.0)] {
            crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, x, y), view);
        }
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 400.0, 400.0), view);
        }
        let parts = flare_parts(&app);
        assert!(!parts.iter().any(|p| p == "Ring"), "{parts:?}");
        let doc = &app.session.doc().unwrap().doc;
        let flare = doc.layers[0].children().unwrap().last().unwrap();
        let rays = flare.children().unwrap().iter().find(|c| c.name.as_deref() == Some("Rays")).unwrap();
        assert_eq!(rays.path_data().unwrap().subpaths.len(), 4);
    }

    #[test]
    fn a_click_fills_in_the_options_and_draws_a_flare_there() {
        let mut app = app();
        app.run("tool.setOption", json!({"tool": "flare", "values": {"rings": 3}})).unwrap();
        crate::dialogs::open_tool_dialog(&mut app, "flare", json!({"x": 300, "y": 300}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.f64("rings", 0.0), d.f64("x", 0.0)), (KIND, 3.0, 300.0));
        super::super::confirm(&mut app).unwrap();
        let parts = flare_parts(&app);
        assert_eq!(parts.iter().filter(|p| *p == "Ring").count(), 3, "{parts:?}");
        // Out-of-range values are kept within the option's range.
        app.run("tool.setOption", json!({"tool": "flare", "values": {"opacity": 900, "diameter": -4}})).unwrap();
        let o = app.session.tool_options_of("flare");
        assert_eq!((o["opacity"].as_f64(), o["diameter"].as_f64()), (Some(100.0), Some(0.0)));
    }
}
