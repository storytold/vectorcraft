//! Symbolism Tools Options (double-click a Symbolism tool): the brush the eight tools share, which
//! lasts across tool switches and is saved with the preferences. OK sets it with
//! `tool.setOption {tool, values}`, and gives a new Symbol Set Density to the selected symbol sets
//! with `symbol.setDensity`.
//!
//! Fields: `tool` (the tool whose options show; the Tool row changes it); `diameter` (points),
//! `intensity` and `density` (Symbol Set Density), 1–10; `usePressure`; `method` (`userDefined`,
//! `average` or `random`); `showBrush`; `sprayScrunch`, `spraySize`, `spraySpin`, `sprayScreen`,
//! `sprayStain` and `sprayStyle` (Symbol Sprayer, each `average` or `userDefined`); `proportional`
//! and `affectsDensity` (Symbol Sizer); `densityWas` (the density the dialog opened with).

use serde_json::{Map, Value, json};
use vectorcraft_tools::settings::SYMBOLISM;

use super::{DialogResult, DialogSpec, form, modal, set_tool_options_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Symbolism Tools Options.
pub const KIND: &str = "symbolismOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Symbolism Tools Options").into(), body, confirm, min_width: 340.0, ..DialogSpec::FORM };

/// The numbers the dialog edits.
const NUMBERS: [&str; 3] = ["diameter", "intensity", "density"];

/// The checkboxes the dialog edits.
const SWITCHES: [&str; 4] = ["usePressure", "showBrush", "proportional", "affectsDensity"];

/// The Symbol Sprayer options.
const SPRAY: [&str; 6] = ["sprayScrunch", "spraySize", "spraySpin", "sprayScreen", "sprayStain", "sprayStyle"];

/// The density the dialog opened with: OK spaces the selected symbol sets from it to the new one.
const DENSITY_WAS: &str = "densityWas";

/// Width of the label column.
const LABEL_W: f32 = 130.0;

/// Width of the dropdowns.
const CHOICE_W: f32 = 170.0;

/// Open the options of Symbolism tool `tool` (one of `vectorcraft_tools::settings::SYMBOLISM`)
/// with their current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Value {
    let o = app.session.tool_options_of(tool);
    let keys = NUMBERS.iter().chain(&SWITCHES).chain(&SPRAY).chain(&["method"]);
    let mut fields: Map<String, Value> = keys.filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    if let Some(density) = fields.get("density").cloned() {
        fields.insert(DENSITY_WAS.into(), density);
    }
    fields.insert("tool".into(), json!(tool));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    // The rows scroll when the window is shorter than they are, so the heading and the buttons
    // stay in view.
    modal::scrolled_rows(ui, "symbolism-options-rows", |ui| rows(app, ui, d));
    false
}

fn rows(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let unit = app.session.general_unit();
    // The tool whose options show below (the eight tools share one toolbar icon, so they are listed
    // by name).
    let tools: Vec<(&str, &str)> = SYMBOLISM.iter().filter_map(|id| Some((*id, vectorcraft_tools::tool_info(id)?.label))).collect();
    form::choice(ui, d, "tool", tl!("Tool:"), (LABEL_W, CHOICE_W), &tools);
    let tool = d.str("tool");
    ui.add_space(6.0);
    widgets::label_row(ui, tl!("Diameter:"), LABEL_W, |ui| {
        form::length(ui, d, "diameter", unit, 100.0);
    });
    // The Symbol Sprayer uses neither Intensity nor pen pressure.
    let sprayer = tool == "symbolSprayer";
    let pressure = d.bool("usePressure");
    widgets::label_row(ui, tl!("Intensity:"), LABEL_W, |ui| {
        ui.add_enabled_ui(!pressure && !sprayer, |ui| {
            if let Some(v) = widgets::range_field(ui, ("symbolism", "intensity"), d.f64("intensity", 5.0), 1.0..=10.0, "", 0, 100.0) {
                d.fields.insert("intensity".into(), json!(v));
            }
        });
    });
    widgets::label_row(ui, "", LABEL_W, |ui| {
        ui.add_enabled_ui(!sprayer, |ui| form::check(ui, d, "usePressure", tl!("Use Pressure Pen")));
    });
    widgets::label_row(ui, tl!("Symbol Set Density:"), LABEL_W, |ui| {
        if let Some(v) = widgets::range_field(ui, ("symbolism", "density"), d.f64("density", 5.0), 1.0..=10.0, "", 0, 100.0) {
            d.fields.insert("density".into(), json!(v));
        }
    });
    // The Symbol Sprayer and the Symbol Shifter have no Method.
    ui.add_enabled_ui(!["symbolSprayer", "symbolShifter"].contains(&tool.as_str()), |ui| {
        let methods = [("userDefined", tl!("User Defined")), ("average", tl!("Average")), ("random", tl!("Random"))];
        form::choice(ui, d, "method", tl!("Method:"), (LABEL_W, CHOICE_W), &methods);
    });
    if tool == "symbolSprayer" {
        ui.add_space(10.0);
        widgets::subheader(ui, tl!("Symbol Sprayer Options"));
        ui.add_space(4.0);
        let modes = [("average", tl!("Average")), ("userDefined", tl!("User Defined"))];
        let labels = [tl!("Scrunch:"), tl!("Size:"), tl!("Spin:"), tl!("Screen:"), tl!("Stain:"), tl!("Style:")];
        for (key, label) in SPRAY.iter().zip(labels) {
            form::choice(ui, d, key, label, (LABEL_W, CHOICE_W), &modes);
        }
    }
    if tool == "symbolSizer" {
        ui.add_space(10.0);
        widgets::subheader(ui, tl!("Symbol Sizer Options"));
        ui.add_space(4.0);
        form::check(ui, d, "proportional", tl!("Proportional Resizing"));
        form::check(ui, d, "affectsDensity", tl!("Resizing Affects Density"));
    }
    ui.add_space(10.0);
    form::check(ui, d, "showBrush", tl!("Show Brush Size and Intensity"));
}

/// The selected symbol sets that can be edited (a locked or hidden one keeps its density).
fn selected_sets(app: &VectorcraftApp) -> Vec<u64> {
    let Some(st) = app.session.active() else { return vec![] };
    let editable = st.doc.editable();
    st.selection
        .objects
        .iter()
        .filter(|id| st.doc.node(**id).is_some_and(vectorcraft_engine::cmd::is_symbol_set) && editable(**id))
        .map(|id| id.0)
        .collect()
}

/// OK: set the brush the Symbolism tools share, after giving a new Symbol Set Density to the
/// selected symbol sets. A value outside its range is refused and changes nothing.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    // Each value goes to the check as it is stored, so one of the wrong kind is refused rather than
    // read as 0 or false. A number stored as text is read as the field reads it, and other text is
    // refused.
    let stored = |k: &str| match d.fields.get(k) {
        Some(Value::String(s)) => Some(vectorcraft_doc::Unit::Points.parse(s).map_or_else(|| json!(s), |n| json!(n))),
        v => v.cloned(),
    };
    let mut values: Map<String, Value> = NUMBERS.iter().filter_map(|k| Some((k.to_string(), stored(k)?))).collect();
    values.extend(SWITCHES.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))));
    values.extend(SPRAY.iter().chain(&["method"]).filter(|k| d.fields.contains_key(**k)).map(|k| (k.to_string(), json!(d.str(k)))));
    // A tool or a value `tool.setOption` refuses changes no density either: the values go through
    // both of its checks, the kinds and then each option's range, before the density changes.
    let tool = d.str("tool");
    if !SYMBOLISM.contains(&tool.as_str()) {
        return Err(format!("unknown tool `{tool}`"));
    }
    vectorcraft_tools::symbolism::check_options(&tool, &values)?;
    vectorcraft_tools::option_check::check_values(&vectorcraft_tools::symbolism::SymbolismTool::new(&tool), &values)?;
    // The density the sets had when the dialog opened is read as a number too.
    if stored(DENSITY_WAS).is_some_and(|v| !v.is_number()) {
        return Err(format!("`{DENSITY_WAS}` must be a number"));
    }
    let (from, to) = (d.f64(DENSITY_WAS, 0.0).clamp(1.0, 10.0), d.f64("density", 0.0).clamp(1.0, 10.0));
    let sets = selected_sets(app);
    if d.fields.contains_key(DENSITY_WAS) && from != to && !sets.is_empty() {
        app.run("symbol.setDensity", json!({ "from": from, "to": to, "ids": sets }))?;
    }
    set_tool_options_and_close(app, json!({ "tool": tool, "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    }

    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    /// In a short window the rows scroll, and the heading, OK and Cancel stay on the screen.
    #[test]
    fn the_buttons_stay_in_view_in_a_short_window() {
        let mut app = app();
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 800.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(4);
        for (physical, zoom) in [(egui::vec2(1366.0, 768.0), 1.5), (egui::vec2(1280.0, 720.0), 1.5), (egui::vec2(800.0, 600.0), 1.0)] {
            h.ctx.set_zoom_factor(zoom);
            for _ in 0..12 {
                h.set_size(physical / zoom);
                h.run_steps(1);
            }
            let screen = h.ctx.content_rect();
            let heading = h.get_by_role_and_label(egui::accesskit::Role::Label, "Symbolism Tools Options").rect();
            assert!(screen.contains_rect(heading), "{physical:?} at {zoom}: the heading {heading:?} in {screen:?}");
            for label in ["OK", "Cancel"] {
                let rect = h.get_by_label(label).rect();
                assert!(screen.contains_rect(rect), "{physical:?} at {zoom}: {label} {rect:?} in {screen:?}");
            }
        }
    }

    #[test]
    fn the_symbolism_tools_share_the_options_ok_sets() {
        let mut app = app();
        app.run("tool.options", json!({"tool": "symbolSizer"})).unwrap();
        let text = painted(&mut app);
        for label in ["Symbolism Tools Options", "Diameter:", "Intensity:", "Symbol Set Density:"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.str("tool"), d.f64("diameter", 0.0), d.f64("intensity", 0.0)), ("symbolSizer".to_string(), 80.0, 5.0));
        for (k, v) in [("diameter", json!(120)), ("intensity", json!(9)), ("density", json!(2))] {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        // The Sprayer has them too: one brush for the eight tools.
        let o = app.session.tool_options_of("symbolSprayer");
        assert_eq!((o["diameter"].as_f64(), o["intensity"].as_f64(), o["density"].as_f64()), (Some(120.0), Some(9.0), Some(2.0)));
    }

    /// Every tool shows the general options; the Symbol Sprayer and the Symbol Sizer show their own
    /// sections, and the Tool row's choice decides which.
    #[test]
    fn the_sprayer_and_the_sizer_show_their_sections() {
        let mut app = app();
        let general = ["Tool:", "Use Pressure Pen", "Method:", "User Defined", "Show Brush Size and Intensity"];
        let sprayer = ["Symbol Sprayer Options", "Scrunch:", "Size:", "Spin:", "Screen:", "Stain:", "Style:"];
        let sizer = ["Symbol Sizer Options", "Proportional Resizing", "Resizing Affects Density"];
        for (tool, own, others) in
            [("symbolSprayer", &sprayer[..], &sizer[..]), ("symbolSizer", &sizer[..], &sprayer[..]), ("symbolStainer", &[][..], &sprayer[..])]
        {
            app.run("tool.options", json!({"tool": tool})).unwrap();
            let text = painted(&mut app);
            for label in general.iter().chain(own) {
                assert!(text.contains(label), "{tool}: {label} in {text}");
            }
            for label in others {
                assert!(!text.contains(label), "{tool}: no {label} in {text}");
            }
        }
        // The Stainer's dialog, switched to the Sizer with the Tool row.
        app.ui.dialog.as_mut().unwrap().fields.insert("tool".into(), json!("symbolSizer"));
        let text = painted(&mut app);
        assert!(text.contains("Proportional Resizing") && !text.contains("Scrunch:"), "{text}");
    }

    /// The Symbol Sprayer uses neither Intensity nor pen pressure: its Use Pressure Pen is disabled,
    /// and the other tools' is not.
    #[test]
    fn the_sprayer_disables_use_pressure_pen() {
        let mut app = app();
        for (tool, off) in [("symbolSprayer", true), ("symbolSizer", false), ("symbolShifter", false)] {
            app.run("tool.options", json!({"tool": tool})).unwrap();
            let tree = crate::widgets::accessibility_tests::tree(|ui| super::super::show(&mut app, ui.ctx()));
            let pen = crate::widgets::accessibility_tests::named(&tree, "Use Pressure Pen");
            assert_eq!(pen.is_disabled(), off, "{tool}");
        }
    }

    /// OK sets Method, Use Pressure Pen, Show Brush Size and Intensity and the Sprayer's and the
    /// Sizer's options for the eight tools, and the dialog opens with them again.
    #[test]
    fn ok_sets_the_method_the_pen_and_the_tool_sections() {
        let mut app = app();
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.str("method"), d.bool("usePressure"), d.bool("showBrush")), ("userDefined".to_string(), false, true));
        assert_eq!((d.str("sprayStain"), d.bool("proportional"), d.bool("affectsDensity")), ("average".to_string(), true, false));
        for (k, v) in [
            ("method", json!("random")),
            ("usePressure", json!(true)),
            ("showBrush", json!(false)),
            ("sprayStain", json!("userDefined")),
            ("spraySpin", json!("userDefined")),
            ("proportional", json!(false)),
            ("affectsDensity", json!(true)),
        ] {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        let o = app.session.tool_options_of("symbolScreener");
        assert_eq!((o["method"].as_str(), o["usePressure"].as_bool(), o["showBrush"].as_bool()), (Some("random"), Some(true), Some(false)));
        assert_eq!(
            (o["sprayStain"].as_str(), o["spraySpin"].as_str(), o["sprayScrunch"].as_str()),
            (Some("userDefined"), Some("userDefined"), Some("average"))
        );
        assert_eq!((o["proportional"].as_bool(), o["affectsDensity"].as_bool()), (Some(false), Some(true)));
        app.run("tool.options", json!({"tool": "symbolSizer"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("method"), d.bool("usePressure"), d.bool("affectsDensity")), ("random".to_string(), true, true));
    }

    /// A new Symbol Set Density spaces the instances of the selected symbol set, as one undo step;
    /// the same density, or no set selected, adds none.
    #[test]
    fn ok_gives_a_new_density_to_the_selected_symbol_set() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        app.run("symbol.new", json!({"name": "Dot"})).unwrap();
        app.run("select.set", json!({"ids": []})).unwrap();
        let points: Vec<Value> = (0..8).map(|i| json!([40.0 + 40.0 * f64::from(i), 150.0])).collect();
        let set = app.run("symbol.spray", json!({"name": "Dot", "points": points, "radius": 4})).unwrap()["id"].as_u64().unwrap();
        let width = |app: &VectorcraftApp| {
            let st = app.session.active().unwrap();
            st.doc.node(vectorcraft_doc::NodeId(set)).and_then(|n| n.geometric_bounds()).map(|b| b.width()).unwrap()
        };
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let (was, steps) = (width(&app), undo(&app));
        // The same density: the set stays as it is.
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        super::super::confirm(&mut app).unwrap();
        assert_eq!((width(&app), undo(&app)), (was, steps));
        // An option that `tool.setOption` refuses changes neither the density nor the set.
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        let fields = &mut app.ui.dialog.as_mut().unwrap().fields;
        fields.insert("density".into(), json!(9));
        fields.insert("method".into(), json!("smooth"));
        assert!(super::super::confirm(&mut app).is_err());
        assert_eq!((width(&app), undo(&app)), (was, steps));
        assert_eq!(app.session.tool_options_of("symbolSizer")["density"].as_f64(), Some(5.0));
        // So does a checkbox or a number stored as a value of the wrong kind, or a number outside its
        // range.
        for (k, v) in [
            ("diameter", json!(1)),
            ("intensity", json!(11)),
            ("usePressure", json!("invalid")),
            ("proportional", json!(1)),
            ("intensity", json!("strong")),
            ("diameter", json!(true)),
            ("tool", json!("pen")),
            ("tool", json!(null)),
        ] {
            app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
            let fields = &mut app.ui.dialog.as_mut().unwrap().fields;
            fields.insert("density".into(), json!(9));
            fields.insert(k.into(), v);
            assert!(super::super::confirm(&mut app).is_err(), "{k}");
            assert_eq!((width(&app), undo(&app)), (was, steps), "{k}");
            let o = app.session.tool_options_of("symbolSizer");
            assert_eq!((o["density"].as_f64(), o["usePressure"].as_bool()), (Some(5.0), Some(false)), "{k}");
        }
        // A selected set that is locked keeps its density, and OK still sets the options.
        let lock = |app: &mut VectorcraftApp, on: bool| {
            app.session
                .edit("Lock", |d, _| {
                    d.node_mut(vectorcraft_doc::NodeId(set)).unwrap().locked = on;
                    Ok(())
                })
                .unwrap();
        };
        lock(&mut app, true);
        let steps_locked = undo(&app);
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("density".into(), json!(8));
        super::super::confirm(&mut app).unwrap();
        assert_eq!((width(&app), undo(&app)), (was, steps_locked));
        assert_eq!(app.session.tool_options_of("symbolSizer")["density"].as_f64(), Some(8.0));
        lock(&mut app, false);
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("density".into(), json!(5));
        super::super::confirm(&mut app).unwrap();
        let steps = undo(&app);
        // A higher density packs the set closer, as one undo step.
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("density".into(), json!(9));
        super::super::confirm(&mut app).unwrap();
        assert!(width(&app) < was * 0.8, "{} of {was}", width(&app));
        assert_eq!(undo(&app), steps + 1);
        assert_eq!(app.session.tool_options_of("symbolSizer")["density"].as_f64(), Some(9.0));
        // An option the tools do not have: neither the set nor the options change.
        let (was, steps) = (width(&app), undo(&app));
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("density".into(), json!(3));
        d.fields.insert("method".into(), json!("smooth"));
        assert!(super::super::confirm(&mut app).is_err());
        assert_eq!((width(&app), undo(&app)), (was, steps));
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("density".into(), json!(3));
        d.fields.insert("usePressure".into(), json!(1));
        assert!(super::super::confirm(&mut app).is_err());
        assert_eq!((width(&app), undo(&app)), (was, steps));
        // A tool that is not a Symbolism tool, a density that is not a number, or a number out of
        // its range.
        for (k, v) in
            [("tool", json!("pen")), ("density", json!("three")), ("densityWas", json!(null)), ("diameter", json!(1)), ("intensity", json!(11))]
        {
            app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("density".into(), json!(3));
            d.fields.insert(k.into(), v);
            assert!(super::super::confirm(&mut app).is_err(), "{k}");
            assert_eq!((width(&app), undo(&app)), (was, steps), "{k}");
        }
        // A length with its unit is a diameter.
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("diameter".into(), json!("12 pt"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.tool_options_of("symbolSizer")["diameter"].as_f64(), Some(12.0));
        assert_eq!(app.session.tool_options_of("symbolSizer")["density"].as_f64(), Some(9.0));
        app.ui.dialog = None;
        // Without a set selected only the option changes.
        app.run("select.set", json!({"ids": []})).unwrap();
        let (was, steps) = (width(&app), undo(&app));
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("density".into(), json!(2));
        super::super::confirm(&mut app).unwrap();
        assert_eq!((width(&app), undo(&app)), (was, steps));
        assert_eq!(app.session.tool_options_of("symbolSizer")["density"].as_f64(), Some(2.0));
    }

    /// A locked symbol set in the selection keeps its spacing, and OK still sets the options.
    #[test]
    fn ok_sets_the_options_with_a_locked_symbol_set_selected() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        app.run("symbol.new", json!({"name": "Dot"})).unwrap();
        app.run("select.set", json!({"ids": []})).unwrap();
        let set = app.run("symbol.spray", json!({"name": "Dot", "points": [[40, 150], [120, 150]], "radius": 4})).unwrap()["id"].as_u64().unwrap();
        app.run("object.lock", json!({})).unwrap();
        app.session.select(|_, sel| sel.set([vectorcraft_doc::NodeId(set)])).unwrap();
        let steps = app.session.active().unwrap().history.undo.len();
        app.run("tool.options", json!({"tool": "symbolSprayer"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("density".into(), json!(9));
        d.fields.insert("diameter".into(), json!(30));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
        let o = app.session.tool_options_of("symbolSizer");
        assert_eq!((o["density"].as_f64(), o["diameter"].as_f64()), (Some(9.0), Some(30.0)));
    }
}
