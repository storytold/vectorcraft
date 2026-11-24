//! Symbolism Tools Options (double-click a Symbolism tool): the brush the eight tools share, which
//! lasts across tool switches and is saved with the preferences. OK sets it with
//! `tool.setOption {tool, values}`, and gives a new Symbol Set Density to the selected symbol sets
//! with `symbol.setDensity`.
//!
//! Fields: `tool` (the tool whose options show; the Tool row changes it); `diameter` (points),
//! `intensity` and `density` (Symbol Set Density), 1–10; `usePressure`; `method` (`userDefined`,
//! `average` or `random`); `showBrush`; `sprayScrunch`, `spraySize`, `spraySpin`, `sprayScreen`,
//! `sprayStain` and `sprayStyle` (Symbol Sprayer, each `average` or `userDefined`); `proportional`
//! and `affectsDensity` (Symbol Sizer).

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{Label, LengthUnits};
use craft::ui::value_drag::SliderScale;
use serde_json::{Value, json};
use vectorcraft_tools::settings::SYMBOLISM;

use super::{form, host, tool_options};
use crate::VectorcraftApp;

/// The dialog kind of the Symbolism Tools Options.
pub const KIND: &str = "symbolismOptions";

/// The numbers the dialog edits.
const NUMBERS: [&str; 3] = ["diameter", "intensity", "density"];

/// The checkboxes the dialog edits.
const SWITCHES: [&str; 4] = ["usePressure", "showBrush", "proportional", "affectsDensity"];

/// The Symbol Sprayer options.
const SPRAY: [&str; 6] = ["sprayScrunch", "spraySize", "spraySpin", "sprayScreen", "sprayStain", "sprayStyle"];

/// The tools other than the ones in `without`.
fn tools_but(without: &[&str]) -> Vec<&'static str> {
    SYMBOLISM.iter().copied().filter(|t| !without.contains(t)).collect()
}

/// The rows of the dialog. The Symbol Sprayer uses neither Intensity nor pen pressure, and it and
/// the Symbol Shifter have no Method.
fn params(lengths: LengthUnits) -> Vec<Param> {
    let tools = SYMBOLISM.iter().filter_map(|id| Some((id.to_string(), Label::from(tl!(vectorcraft_tools::tool_info(id)?.label))))).collect();
    let modes = || vec![("average".to_string(), Label::from(tl!("Average"))), ("userDefined".to_string(), Label::from(tl!("User Defined")))];
    let methods = vec![
        ("userDefined".to_string(), Label::from(tl!("User Defined"))),
        ("average".to_string(), Label::from(tl!("Average"))),
        ("random".to_string(), Label::from(tl!("Random"))),
    ];
    let not_sprayer = tools_but(&["symbolSprayer"]);
    let range = |key: &str, label: &'static str| Param::new(key, label, ParamKind::range(1.0, 10.0, 5.0, SliderScale::Linear));
    let mut rows = vec![
        Param::new("tool", tl!("Tool:"), ParamKind::choice(tools, None)),
        Param::number("diameter", tl!("Diameter:"), 80.0, 1.0, 0).length(lengths),
        range("intensity", tl!("Intensity:")).enabled_when_in("tool", not_sprayer.clone()),
        Param::new("usePressure", tl!("Use Pressure Pen"), ParamKind::bool(false)).enabled_when_in("tool", not_sprayer),
        range("density", tl!("Symbol Set Density:")),
        Param::new("method", tl!("Method:"), ParamKind::choice(methods, Some("userDefined".into())))
            .enabled_when_in("tool", tools_but(&["symbolSprayer", "symbolShifter"])),
        Param::section(tl!("Symbol Sprayer Options")).visible_when("tool", "symbolSprayer"),
    ];
    let labels = [tl!("Scrunch:"), tl!("Size:"), tl!("Spin:"), tl!("Screen:"), tl!("Stain:"), tl!("Style:")];
    rows.extend(
        SPRAY
            .iter()
            .zip(labels)
            .map(|(key, label)| Param::new(*key, label, ParamKind::choice(modes(), Some("average".into()))).visible_when("tool", "symbolSprayer")),
    );
    rows.extend([
        Param::section(tl!("Symbol Sizer Options")).visible_when("tool", "symbolSizer"),
        Param::new("proportional", tl!("Proportional Resizing"), ParamKind::bool(true)).visible_when("tool", "symbolSizer"),
        Param::new("affectsDensity", tl!("Resizing Affects Density"), ParamKind::bool(false)).visible_when("tool", "symbolSizer"),
        Param::new("showBrush", tl!("Show Brush Size and Intensity"), ParamKind::bool(true)),
    ]);
    rows
}

/// Open the options of Symbolism tool `tool` (one of `vectorcraft_tools::settings::SYMBOLISM`)
/// with their current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Value {
    let params = params(crate::canvas::general_lengths(app)); // PLAN: assumed path of cv.B5b
    let o = app.session.tool_options_of(tool);
    let mut values: serde_json::Map<String, Value> =
        NUMBERS.iter().chain(&SWITCHES).chain(&SPRAY).chain(&["method"]).filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    values.insert("tool".into(), json!(tool));
    let density_was = values.get("density").and_then(Value::as_f64).map(|d| d.clamp(1.0, 10.0));
    let fields = form::fields_of(&params, &values);
    let shown = params.clone();
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| {
        let mut values = form::json_of(&params, &result.fields);
        // A tool or a value `tool.setOption` refuses changes no density either: the values go through
        // both of its checks, the kinds and then each option's range, before the density changes.
        let tool = result.fields.text("tool").unwrap_or_default().to_string();
        values.remove("tool");
        if !SYMBOLISM.contains(&tool.as_str()) {
            return Err(format!("unknown tool `{tool}`"));
        }
        vectorcraft_tools::symbolism::check_options(&tool, &values)?;
        vectorcraft_tools::option_check::check_values(&vectorcraft_tools::symbolism::SymbolismTool::new(&tool), &values)?;
        if let Some(from) = density_was {
            let to = result.fields.number("density").unwrap_or(from).clamp(1.0, 10.0);
            let sets = selected_sets(app);
            if from != to && !sets.is_empty() {
                app.run("symbol.setDensity", json!({ "from": from, "to": to, "ids": sets }))?;
            }
        }
        tool_options::set_options(app, &tool, values)
    };
    let opened = app.ui_ctx.clone().and_then(|ctx| {
        let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |f| {
            ParamDialog::new(KIND, tl!("Symbolism Tools Options"), shown.clone()).values(Bind::bound(f))
        });
        host::open_held(app, dialog, ok)
    });
    if opened.is_none() {
        app.status(tl!("No window to show the dialog in.").to_string());
    }
    json!({ "dialog": KIND })
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

#[cfg(test)]
mod tests {
    use super::super::tests_rectangular_grid::{set_of, try_confirm};
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    }

    /// A symbol set of eight instances in a row, and its geometric width.
    fn sprayed(app: &mut VectorcraftApp) -> u64 {
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        app.run("symbol.new", json!({"name": "Dot"})).unwrap();
        app.run("select.set", json!({"ids": []})).unwrap();
        let points: Vec<Value> = (0..8).map(|i| json!([40.0 + 40.0 * f64::from(i), 150.0])).collect();
        app.run("symbol.spray", json!({"name": "Dot", "points": points, "radius": 4})).unwrap()["id"].as_u64().unwrap()
    }

    fn width(app: &VectorcraftApp, set: u64) -> f64 {
        let st = app.session.active().unwrap();
        st.doc.node(vectorcraft_doc::NodeId(set)).and_then(|n| n.geometric_bounds()).map(|b| b.width()).unwrap()
    }

    fn undo(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// A new Symbol Set Density spaces the instances of the selected symbol set, as one undo step;
    /// the same density adds none, and a value `tool.setOption` refuses changes no density.
    #[test]
    fn ok_gives_a_new_density_to_the_selected_symbol_set() {
        let mut app = app();
        let set = sprayed(&mut app);
        let (was, steps) = (width(&app, set), undo(&app));
        // The same density: the set stays as it is.
        open(&mut app, "symbolSprayer");
        try_confirm(&mut app, KIND).unwrap();
        assert_eq!((width(&app, set), undo(&app)), (was, steps));
        // A value the options refuse keeps the dialog open and changes neither the set nor the options.
        open(&mut app, "symbolSprayer");
        set_of(&mut app, KIND, "density", json!(9)).unwrap();
        set_of(&mut app, KIND, "diameter", json!(1)).unwrap();
        assert!(try_confirm(&mut app, KIND).is_err());
        assert_eq!((width(&app, set), undo(&app)), (was, steps));
        assert_eq!(app.session.tool_options_of("symbolSizer")["density"].as_f64(), Some(5.0));
        // A row refuses a value of the wrong kind at the row.
        assert!(set_of(&mut app, KIND, "method", json!("smooth")).is_err());
        // A higher density packs the set closer, as one undo step.
        set_of(&mut app, KIND, "diameter", json!(80)).unwrap();
        try_confirm(&mut app, KIND).unwrap();
        assert!(width(&app, set) < was * 0.8, "{} of {was}", width(&app, set));
        assert_eq!(undo(&app), steps + 1);
        assert_eq!(app.session.tool_options_of("symbolSizer")["density"].as_f64(), Some(9.0));
        // Without a set selected only the option changes.
        app.run("select.set", json!({"ids": []})).unwrap();
        let (was, steps) = (width(&app, set), undo(&app));
        open(&mut app, "symbolSprayer");
        set_of(&mut app, KIND, "density", json!(2)).unwrap();
        try_confirm(&mut app, KIND).unwrap();
        assert_eq!((width(&app, set), undo(&app)), (was, steps));
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
        let steps = undo(&app);
        open(&mut app, "symbolSprayer");
        set_of(&mut app, KIND, "density", json!(9)).unwrap();
        set_of(&mut app, KIND, "diameter", json!(30)).unwrap();
        try_confirm(&mut app, KIND).unwrap();
        assert_eq!(undo(&app), steps);
        let o = app.session.tool_options_of("symbolSizer");
        assert_eq!((o["density"].as_f64(), o["diameter"].as_f64()), (Some(9.0), Some(30.0)));
    }
}
