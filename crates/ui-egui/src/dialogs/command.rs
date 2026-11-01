//! The generic parameter dialog (`ui.paramDialog`): edits a command's parameters (`__command`,
//! headed `__label`) and runs it on OK, with the fixed ones in `__params` (not shown) added.

use std::sync::LazyLock;

use serde_json::{Value, json};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| tl!(&d.str("__label")).to_string(),
    body: |app, ui, d| {
        let command = d.str("__command");
        let (lengths, order) = (lengths(&command), order(&command));
        let rank = |k: &str| order.iter().position(|o| *o == k).unwrap_or(order.len());
        // Area Type Options › Text Flow is a pair of radio buttons after the other options.
        let flow = if command == "text.areaOptions" { d.fields.remove("flow") } else { None };
        form::param_fields(ui, d, &|k| lengths.contains(&k), &|k| choices(&command, k), &rank, app.session.general_unit());
        if let Some(v) = flow {
            let mut rows = v.as_str() == Some("rows");
            ui.add_space(4.0);
            egui::Grid::new("text-flow").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                let row = ui.label(egui::RichText::new(format!("{}:", tl!("Text Flow"))).color(Tokens::get(ui.ctx()).text));
                ui.label_inputs_by(row.id, |ui| {
                    ui.horizontal(|ui| {
                        if widgets::radio(ui, tl!("By Rows"), rows, true) {
                            rows = true;
                        }
                        if widgets::radio(ui, tl!("By Columns"), !rows, true) {
                            rows = false;
                        }
                    });
                });
                ui.end_row();
            });
            d.fields.insert("flow".into(), json!(if rows { "rows" } else { "columns" }));
        }
        if previews(d) {
            let (cmd, params) = call(d);
            form::preview(app, ui, d, &d.str("__label"), &cmd, params);
        }
        false
    },
    confirm,
    passes: |d, key| takes(d, key) || (key == "preview" && previews(d)),
    ..DialogSpec::FORM
};

/// Does this dialog show a Preview checkbox? Only Grid Repeat Options does: the public page for
/// the grid options lists one, and the radial and mirror pages do not. (Cancel rolls a preview
/// back whichever dialog started it.)
fn previews(d: &Dialog) -> bool {
    d.str("__command") == "object.repeat.options" && d.fields.contains_key("gridType")
}

/// The command the dialog runs and its parameters: the fields shown and the fixed ones.
fn call(d: &Dialog) -> (String, Value) {
    let mut params = form::params(d);
    if let (Some(o), Some(Value::Object(fixed))) = (params.as_object_mut(), d.fields.get("__params")) {
        o.extend(fixed.clone());
    }
    (d.str("__command"), params)
}

/// Does the dialog's command take parameter `key` (its params description names it)?
fn takes(d: &Dialog, key: &str) -> bool {
    let id = d.str("__command");
    vectorcraft_engine::find_command(&id).is_some_and(|c| vectorcraft_engine::check_keys(&id, c.params, &serde_json::json!({ key: null })).is_ok())
}

/// The parameters of the commands this dialog edits that are distances.
fn lengths(command: &str) -> &'static [&'static str] {
    match command {
        "graph.create" => &["width", "height"],
        "shape.flare" => &["diameter", "pathLength"],
        "perspective.grid.set" => &["cell", "distance"],
        "object.repeat.options" => &["radius", "hSpacing", "vSpacing"],
        "text.areaOptions" => &["width", "height", "gutter", "inset", "firstBaselineMin"],
        "type.pathOptions" => &["spacing"],
        "object.pathfinder.trap" => &["thickness"],
        _ => &[],
    }
}

/// The order the fields of the commands this dialog edits come in, where the order of their
/// names isn't it (the rest follow, by name).
fn order(command: &str) -> &'static [&'static str] {
    match command {
        // The public Trap options' order.
        "object.pathfinder.trap" => &["thickness", "heightWidth", "tintReduction", "trapsWithProcessColor", "reverseTraps"],
        "addon.color.printFixes" => &["inkLimit", "excludePureBlack", "removeWhiteOverprint", "overprintBlack", "textOnly", "fill", "stroke"],
        _ => &[],
    }
}

/// The parameters of the commands this dialog edits that pick one of some values.
fn choices(command: &str, key: &str) -> Option<form::Choices> {
    match (command, key) {
        ("type.pathOptions", "effect") => Some(crate::menus::PATH_EFFECTS),
        ("type.pathOptions", "alignToPath") => Some(PATH_ALIGN),
        ("text.areaOptions", "fit") => Some(AREA_FIT),
        ("text.areaOptions", "firstBaseline") => Some(FIRST_BASELINE),
        ("text.areaOptions", "verticalAlign") => Some(VERTICAL_ALIGN),
        ("graph.setType", "type") => Some(GRAPH_KINDS.as_slice()),
        ("graph.setType", "valueAxis") => Some(GRAPH_VALUE_AXIS),
        ("graph.setType", "tickLength" | "rightTickLength" | "categoryTickLength") => Some(GRAPH_TICK_LENGTH),
        ("graph.setType", "pieLegend") => Some(PIE_LEGEND),
        ("graph.setType", "piePosition") => Some(PIE_POSITION),
        ("graph.setType", "pieSort") => Some(PIE_SORT),
        _ => None,
    }
}

/// Type on a Path Options › Align to Path.
const PATH_ALIGN: form::Choices = &[("Ascender", "ascender"), ("Descender", "descender"), ("Center", "center"), ("Baseline", "baseline")];

/// Area Type Options › Fit.
const AREA_FIT: form::Choices = &[("None", "none"), ("Auto Size", "autoHeight"), ("Shrink Text to Fit", "shrinkText")];

/// Area Type Options › First Baseline.
const FIRST_BASELINE: form::Choices =
    &[("Ascent", "ascent"), ("Cap Height", "capHeight"), ("x Height", "xHeight"), ("Leading", "leading"), ("Fixed", "fixed")];

/// Area Type Options › Align (vertical alignment of the lines in each row/column).
const VERTICAL_ALIGN: form::Choices = &[("Top", "top"), ("Center", "center"), ("Bottom", "bottom"), ("Justify", "justify")];

/// Graph Type › Type: the nine graph kinds, in the Graph tool group's order.
static GRAPH_KINDS: LazyLock<Vec<(&'static str, &'static str)>> =
    LazyLock::new(|| vectorcraft_doc::GraphKind::ALL.iter().map(|k| (k.label(), k.id())).collect());

/// Graph Type › Value Axis (series picked on both axes show none, and OK leaves each where it is).
const GRAPH_VALUE_AXIS: form::Choices = &[("On Left Side", "left"), ("On Right Side", "right"), ("On Both Sides", "both")];

/// Graph Type › Legend, Position and Sort for pie graphs.
const PIE_LEGEND: form::Choices = &[("No Legend", "none"), ("Standard Legend", "standard"), ("Legends in Wedges", "wedges")];
const PIE_POSITION: form::Choices = &[("Ratio", "ratio"), ("Even", "even"), ("Stacked", "stacked")];
const PIE_SORT: form::Choices = &[("All", "all"), ("First", "first"), ("None", "none")];

/// Graph Type › Tick Marks › Length.
const GRAPH_TICK_LENGTH: form::Choices = &[("None", "none"), ("Short", "short"), ("Full Width", "full")];

/// Closes before running, so a dialog the command opens stays open. A previewed one keeps its
/// preview as the one undo step.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (cmd, params) = call(d);
    if previews(d) {
        return form::commit_preview(app, &cmd, params);
    }
    app.ui.dialog = None;
    // A tool's click-to-size shape (Flare) goes on the active perspective plane while the grid shows.
    let at = |x: &str, y: &str| Some(vectorcraft_geom::Point::new(params.get(x)?.as_f64()?, params.get(y)?.as_f64()?));
    if let Some((c, p)) =
        at("cx", "cy").or_else(|| at("x", "y")).and_then(|pt| vectorcraft_engine::perspective_click(&app.session, &cmd, &params, pt))
    {
        return app.run(&c, p);
    }
    app.run(&cmd, params)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::{NodeKind, VerticalAlign};
    use vectorcraft_engine::Session;

    use super::*;

    #[test]
    fn area_type_options_choices_match_the_params_doc() {
        let doc = vectorcraft_engine::find_command("text.areaOptions").unwrap().params;
        for key in ["firstBaseline", "verticalAlign"] {
            let values: Vec<&str> = choices("text.areaOptions", key).unwrap().iter().map(|(_, v)| *v).collect();
            assert!(doc.contains(&format!("{key}?: {}", values.join("|"))), "{key}: {values:?}");
        }
        assert!(choices("text.areaOptions", "firstBaselineMin").is_none(), "a distance, not a choice");
        assert!(choices("text.areaOptions", "inset").is_none());
    }

    #[test]
    fn area_type_options_dialog_sets_the_vertical_alignment() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 20, "text": "Hello", "area": {"width": 200, "height": 200}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        crate::menus::invoke(&mut app, "text.areaOptions", json!({}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("verticalAlign"), "top");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        app.ui.dialog.as_mut().unwrap().fields.insert("verticalAlign".into(), json!("center"));
        crate::dialogs::confirm(&mut app).unwrap();
        let NodeKind::Text(t) = &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
        assert_eq!(t.area.vertical_align, VerticalAlign::Center);
    }

    /// Area Type Options › Text Flow (#4978): two radio buttons, By Columns as new area type has
    /// it, and OK fills the cells by rows when By Rows is picked.
    #[test]
    fn area_type_options_dialog_sets_the_text_flow() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 20, "text": "Hello", "area": {"width": 200, "height": 200}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        crate::menus::invoke(&mut app, "text.areaOptions", json!({}));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("flow"), "columns", "drawing the radio buttons keeps the value");
        app.ui.dialog.as_mut().unwrap().fields.insert("flow".into(), json!("rows"));
        crate::dialogs::confirm(&mut app).unwrap();
        let NodeKind::Text(t) = &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
        assert_eq!(t.area.flow, vectorcraft_doc::TextFlow::Rows);
    }

    /// Graph Type › Value Axis is a dropdown of the values `graph.setType` takes, and OK puts the
    /// value axis on the side picked.
    #[test]
    fn graph_type_value_axis_is_a_dropdown_of_its_sides() {
        let doc = vectorcraft_engine::find_command("graph.setType").unwrap().params;
        let values: Vec<&str> = choices("graph.setType", "valueAxis").unwrap().iter().map(|(_, v)| *v).collect();
        assert!(doc.contains(&format!("valueAxis?: {}", values.join("|"))), "{values:?}");
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
        crate::menus::graph_dialog(&mut app, "graph.setType").unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("valueAxis"), "left");
        app.ui.dialog.as_mut().unwrap().fields.insert("valueAxis".into(), json!("both"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.session.execute("graph.setType", &json!({})).unwrap()["valueAxis"], "both");
    }

    /// Graph Type › Type is a dropdown of the nine graph kinds (#2175), and OK changes the graph's
    /// type and keeps its data.
    #[test]
    fn graph_type_is_a_dropdown_of_the_graph_kinds() {
        let kinds: Vec<(&str, &str)> = vectorcraft_doc::GraphKind::ALL.iter().map(|k| (k.label(), k.id())).collect();
        assert_eq!(choices("graph.setType", "type").unwrap().to_vec(), kinds);
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150, "csv": ",a,b\nQ1,1,2\nQ2,3,4"})).unwrap();
        let data = app.session.execute("graph.setData", &json!({})).unwrap();
        assert_eq!(data["rows"], json!([[1.0, 2.0], [3.0, 4.0]]));
        crate::menus::graph_dialog(&mut app, "graph.setType").unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("type"), "column");
        app.ui.dialog.as_mut().unwrap().fields.insert("type".into(), json!("stackedBar"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.session.execute("graph.setType", &json!({})).unwrap()["type"], "stackedBar");
        assert_eq!(app.session.execute("graph.setData", &json!({})).unwrap(), data);
    }

    /// A document with a rectangle made a 2 x 2 Grid Repeat and selected, its Repeat Options
    /// dialog open.
    fn grid_options_app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("object.repeat.grid", json!({"ids": [id], "rows": 2, "cols": 2})).unwrap();
        crate::menus::invoke(&mut app, "object.repeat.options", json!({}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, "command");
        app
    }

    /// One headless frame of the open dialog.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn cols(app: &VectorcraftApp) -> u64 {
        crate::panels::pattern_options::repeat_fields(app).unwrap()["cols"].as_u64().unwrap()
    }

    /// #2651: the grid options' public page lists a Preview checkbox: with it on, the repeat on
    /// the canvas follows the fields while the dialog is open; Cancel puts it back without a
    /// step of history; OK keeps what was previewed as one step.
    #[test]
    fn grid_repeat_options_preview_on_the_canvas() {
        let mut app = grid_options_app();
        let steps = app.session.doc().unwrap().history.undo.len();
        // Off: the fields change nothing until OK.
        app.ui.dialog.as_mut().unwrap().fields.insert("cols".into(), json!(3));
        frame(&mut app);
        assert_eq!(cols(&app), 2, "no preview without the checkbox");
        // On, the repeat follows the fields.
        crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
        frame(&mut app);
        assert_eq!(cols(&app), 3, "the canvas shows 3 columns while the dialog is open");
        app.ui.dialog.as_mut().unwrap().fields.insert("cols".into(), json!(4));
        frame(&mut app);
        assert_eq!(cols(&app), 4, "and follows the next edit");
        // Cancel puts the original back and leaves no history.
        crate::dialogs::cancel(&mut app);
        assert_eq!(cols(&app), 2, "Cancel restores the original repeat");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps, "and adds no undo step");
        assert!(!app.session.in_interaction());
        // OK keeps the previewed values as one undo step.
        crate::menus::invoke(&mut app, "object.repeat.options", json!({}));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("cols".into(), json!(5));
        crate::dialogs::set_field(d, "preview".into(), json!(true));
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(cols(&app), 5);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(cols(&app), 2, "one Undo takes the whole change back");
    }

    /// Radial and Mirror Repeat Options have no Preview (their public pages list none): their
    /// fields apply on OK only, and the dialog takes no `preview`.
    #[test]
    fn radial_repeat_options_have_no_preview() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("object.repeat.radial", json!({"ids": [id], "instances": 4})).unwrap();
        crate::menus::invoke(&mut app, "object.repeat.options", json!({}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(!crate::dialogs::accepts_field(d, "preview"));
        app.ui.dialog.as_mut().unwrap().fields.insert("instances".into(), json!(7));
        frame(&mut app);
        assert_eq!(crate::panels::pattern_options::repeat_fields(&app).unwrap()["instances"], 4);
        assert!(!app.session.in_interaction());
    }
}
