//! Flare Tool Options (double-click the Flare tool, or click with it): the options the Flare tool
//! keeps, which draw the next flare dragged out, last across tool switches and are saved with the
//! preferences. OK sets them with `tool.setOption {tool: "flare", values}`; opened by a click, it
//! accepts a flare preview there with them (`shape.flare`, on the active perspective plane while the
//! grid shows). While Alt (Option on macOS) is held, Cancel reads Reset, which puts every option
//! back to the tool's default and leaves the dialog open (`ui.dialogReset`). A click-opened
//! dialog previews its settings live; Reset refreshes that preview and Cancel rolls it back.
//!
//! Fields: the options (`vectorcraft_tools::extra::FLARE_OPTIONS`): `diameter` and `pathLength`
//! (points), `opacity`, `brightness`, `growth`, `fuzziness`, `longest`, `rayFuzziness` and
//! `largest` (%), `rays` and `rings` (counts), `direction` (degrees); `raysOn` and `ringsOn`; and
//! `x`, `y` where a click opened it, with `preview` on initially for that click.

use serde_json::{Map, Value, json};
use vectorcraft_geom::Point;
use vectorcraft_tools::extra::{FLARE_OPTIONS, flare_params};

use super::document_setup::check;
use super::{DialogResult, DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Flare Tool Options.
pub const KIND: &str = "flareOptions";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Flare Tool Options").into(),
    body,
    confirm,
    reset: Some(reset),
    preview: true,
    min_width: 440.0,
    ..DialogSpec::FORM
};

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
        fields.insert("preview".into(), json!(true));
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
    if let Some((cmd, params)) = preview_command(app, d) {
        form::preview(app, ui, d, "Flare", &cmd, params);
    }
    false
}

/// A click-created flare previews at that original click; toolbar options draw no artwork.
fn preview_command(app: &VectorcraftApp, d: &Dialog) -> Option<(String, Value)> {
    let at = Point::new(d.fields.get("x")?.as_f64().filter(|v| v.is_finite())?, d.fields.get("y")?.as_f64().filter(|v| v.is_finite())?);
    // Use the same clamping and whole-count conversion as accepted tool settings.
    let mut tool = vectorcraft_tools::create("flare");
    for (key, default, ..) in FLARE_OPTIONS {
        tool.set_option(key, &json!(d.f64(key, default)));
    }
    for key in ["raysOn", "ringsOn"] {
        tool.set_option(key, &json!(d.bool(key)));
    }
    let params = flare_params(&tool.options(), at);
    Some(vectorcraft_engine::perspective_click(&app.session, "shape.flare", &params, at).unwrap_or_else(|| ("shape.flare".into(), params)))
}

/// Reset (Cancel with Alt or Option held): every option goes back to the Flare tool's default
/// (`FLARE_OPTIONS`, with Rays and Rings on). The dialog keeps a click's `x` and `y`, so OK still draws there.
fn reset(_: &VectorcraftApp, d: &mut Dialog) {
    if let Value::Object(defaults) = vectorcraft_tools::create("flare").options() {
        d.fields.extend(defaults);
    }
}

/// OK: set the tool's options; opened by a click, draw a flare there with them.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let command = preview_command(app, d);
    if command.is_none() && (d.fields.contains_key("x") || d.fields.contains_key("y") || d.fields.contains_key(form::PREVIEWED)) {
        return Err("Flare click coordinates must be finite numbers".into());
    }
    let mut values: Map<String, Value> =
        FLARE_OPTIONS.iter().filter(|(k, ..)| d.fields.contains_key(*k)).map(|(k, default, ..)| (k.to_string(), json!(d.f64(k, *default)))).collect();
    for key in ["raysOn", "ringsOn"] {
        values.insert(key.into(), json!(d.bool(key)));
    }
    if let Some((cmd, params)) = command {
        // Accept the preview once; option persistence follows successful artwork creation.
        let result = form::commit_preview(app, &cmd, params)?;
        app.run("tool.setOption", json!({ "tool": "flare", "values": values }))?;
        Ok(result)
    } else {
        run_and_close(app, "tool.setOption", json!({ "tool": "flare", "values": values }))
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

    /// Invalid control-channel coordinates must not turn a click dialog into toolbar options.
    #[test]
    fn invalid_click_coordinates_keep_the_dialog_and_preview_cancellable() {
        for (key, value) in [("x", Value::Null), ("y", Value::Null), ("x", json!("invalid"))] {
            let mut app = app();
            let original = app.session.doc().unwrap().doc.layers.clone();
            let depth = app.session.doc().unwrap().history.undo.len();
            let options = app.session.tool_options_of("flare");
            open(&mut app, Some(Point::new(220.0, 330.0)));
            painted(&mut app);
            let preview = app.session.doc().unwrap().doc.layers.clone();
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert(key.into(), value);
            d.fields.insert("rings".into(), json!(4));
            assert_eq!(super::super::confirm(&mut app).unwrap_err(), "Flare click coordinates must be finite numbers");
            assert!(app.ui.dialog.is_some());
            assert!(app.session.in_interaction());
            assert_eq!(app.session.doc().unwrap().doc.layers, preview);
            assert_eq!(app.session.doc().unwrap().history.undo.len(), depth);
            assert_eq!(app.session.tool_options_of("flare"), options);
            super::super::cancel(&mut app);
            assert!(!app.session.in_interaction());
            assert_eq!(app.session.doc().unwrap().doc.layers, original);
        }
    }

    #[test]
    fn cancelling_toolbar_options_preserves_an_existing_interaction() {
        let mut app = app();
        app.session.begin_interaction("Existing gesture").unwrap();
        app.session.preview("shape.rectangle", &json!({"x":10,"y":10,"width":40,"height":40})).unwrap();
        let previewed = app.session.doc().unwrap().doc.layers.clone();
        open(&mut app, None);
        painted(&mut app);
        super::super::cancel(&mut app);
        assert!(app.session.in_interaction());
        assert_eq!(app.session.doc().unwrap().doc.layers, previewed);
        app.session.cancel_interaction().unwrap();
    }

    #[test]
    fn reset_updates_live_preview_and_cancel_discards_art_and_options() {
        let mut app = app();
        app.run("tool.setOption", json!({"tool":"flare", "values":{"rings":3}})).unwrap();
        let original = app.session.doc().unwrap().doc.layers.clone();
        let depth = app.session.doc().unwrap().history.undo.len();
        let options = app.session.tool_options_of("flare");
        open(&mut app, Some(Point::new(220.0, 330.0)));
        let text = painted(&mut app);
        assert!(text.contains("Preview"));
        assert_eq!(flare_parts(&app).iter().filter(|s| *s == "Ring").count(), 3);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth);
        app.ui.dialog.as_mut().unwrap().fields.insert("rings".into(), json!(4.0));
        painted(&mut app);
        assert_eq!(flare_parts(&app).iter().filter(|s| *s == "Ring").count(), 4);
        app.run("ui.dialogReset", json!({})).unwrap();
        painted(&mut app);
        assert_eq!(flare_parts(&app).iter().filter(|s| *s == "Ring").count(), 10);
        assert_eq!(app.session.tool_options_of("flare"), options, "preview doesn't persist options");
        super::super::cancel(&mut app);
        assert_eq!(app.session.doc().unwrap().doc.layers, original);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth);
        assert_eq!(app.session.tool_options_of("flare"), options);
        // Accepting the same reset preview produces one flare and one Undo step.
        open(&mut app, Some(Point::new(220.0, 330.0)));
        painted(&mut app);
        app.run("ui.dialogReset", json!({})).unwrap();
        painted(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
        assert_eq!(flare_parts(&app).iter().filter(|s| *s == "Ring").count(), 10);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.layers, original);
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

    #[test]
    fn ui_dialog_reset_puts_the_defaults_back_and_keeps_where_a_click_opened_it() {
        let mut app = app();
        assert_eq!(app.run("ui.dialogReset", json!({})).unwrap_err(), "no dialog open");
        app.run("tool.setOption", json!({"tool": "flare", "values": {"rings": 3, "raysOn": false, "largest": 120}})).unwrap();
        crate::dialogs::open_tool_dialog(&mut app, "flare", json!({"x": 300, "y": 300}));
        let d = app.run("ui.dialogReset", json!({})).unwrap();
        assert_eq!((d["kind"].as_str(), d["fields"]["rings"].as_f64(), d["fields"]["raysOn"].as_bool()), (Some(KIND), Some(10.0), Some(true)));
        let d = app.ui.dialog.as_ref().expect("the dialog stays open");
        assert_eq!((d.f64("largest", 0.0), d.f64("x", 0.0), d.f64("y", 0.0)), (50.0, 300.0, 300.0));
        // The tool keeps its options until OK, which draws the flare with the defaults where the click was.
        assert_eq!(app.session.tool_options_of("flare")["rings"].as_u64(), Some(3));
        super::super::confirm(&mut app).unwrap();
        let parts = flare_parts(&app);
        assert_eq!((parts.iter().filter(|p| *p == "Ring").count(), parts.iter().any(|p| p == "Rays")), (10, true), "{parts:?}");
        // A dialog without a Reset refuses it.
        app.ui.dialog = Some(Dialog::new("move", json!({"dx": 1})));
        assert_eq!(app.run("ui.dialogReset", json!({})).unwrap_err(), "the open dialog has no Reset");
    }
}
