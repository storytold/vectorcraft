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
        // Area Type Options: a Row Gutter still as it opened, on rows without a gutter of their own,
        // follows the Gutter as it is changed.
        let linked = d.fields.get("__rowGutterLinked").is_some() && d.fields.get("__rowGutterLinked") == d.fields.get("rowGutter");
        let cmyk = app.session.active().is_some_and(|s| s.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk);
        let choices = |k: &str| rasterize_color_models(&command, k, cmyk).or_else(|| choices(&command, k));
        form::param_fields(ui, d, &|k| lengths.contains(&k), &choices, &rank, app.session.general_unit());
        if linked
            && d.fields.get("rowGutter") == d.fields.get("__rowGutterLinked")
            && let Some(g) = d.fields.get("gutter").cloned()
        {
            d.fields.insert("rowGutter".into(), g.clone());
            d.fields.insert("__rowGutterLinked".into(), g);
        }
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

/// Does this dialog show a Preview checkbox? Grid Repeat Options does: the public page for the
/// grid options lists one, and the radial and mirror pages do not. Type on a Path Options does
/// too (#5360), as public tutorials of the dialog show, and Type Tools › Release Type on a Path
/// works the same way (#5532), and so do Image Tools › Remove Background and Color Mask and
/// Cutting Tools › Weed Box and Lines and Banner Hems and Grommets. (Cancel rolls a preview back
/// whichever dialog started it.)
fn previews(d: &Dialog) -> bool {
    match d.str("__command").as_str() {
        "object.repeat.options" => d.fields.contains_key("gridType"),
        "type.pathOptions"
        | crate::addons::typetools::RELEASE
        | crate::addons::image::REMOVE_BACKGROUND
        | crate::addons::image::COLOR_MASK
        | crate::addons::cutting::WEED
        | crate::addons::cutting::BANNER => true,
        _ => false,
    }
}

/// The command the dialog runs and its parameters: the fields shown and the fixed ones.
fn call(d: &Dialog) -> (String, Value) {
    let mut params = form::params(d);
    if let (Some(o), Some(Value::Object(fixed))) = (params.as_object_mut(), d.fields.get("__params")) {
        o.extend(fixed.clone());
    }
    // Area Type Options: a Row Gutter left as it opened, on rows without a gutter of their own,
    // isn't sent, so the rows take the Gutter.
    if let (Some(o), Some(linked)) = (params.as_object_mut(), d.fields.get("__rowGutterLinked"))
        && d.fields.get("rowGutter") == Some(linked)
    {
        o.remove("rowGutter");
    }
    // Weed Box and Lines: a Spacing of 0, or none, leaves the Count in use; any other Spacing is
    // sent instead of the Count, so the command refuses one below 0.
    if d.str("__command") == crate::addons::cutting::WEED
        && let Some(o) = params.as_object_mut()
    {
        let zero = |s: &Value| match s {
            Value::Null => true,
            Value::String(t) => vectorcraft_doc::Unit::Points.parse(t).or_else(|| t.trim().parse().ok()) == Some(0.0),
            v => v.as_f64() == Some(0.0),
        };
        let by_count = o.get("spacing").is_none_or(zero);
        o.remove(if by_count { "spacing" } else { "count" });
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
        crate::addons::cutting::WEED => &["margin", "spacing", "gap", "weight"],
        crate::addons::cutting::BANNER => &["hem", "spacing", "grommetInset", "diameter", "weight"],
        "object.repeat.options" => &["radius", "hSpacing", "vSpacing"],
        "text.areaOptions" => &["width", "height", "gutter", "rowGutter", "inset", "firstBaselineMin", "lineGap"],
        "type.pathOptions" => &["spacing"],
        "object.pathfinder.trap" => &["thickness"],
        crate::addons::cutting::BRIDGES => &["width"],
        "object.textWrap.options" => &["offset"],
        _ => &[],
    }
}

/// The range (points) of a distance parameter of the commands this dialog edits, where the
/// command has one: the field refuses a distance typed outside it.
pub(super) fn length_range(command: &str, key: &str) -> Option<std::ops::RangeInclusive<f64>> {
    match (command, key) {
        ("object.textWrap.options", "offset") => Some(vectorcraft_engine::TEXT_WRAP_OFFSET_RANGE),
        _ => None,
    }
}

/// The order the fields of the commands this dialog edits come in, where the order of their
/// names isn't it (the rest follow, by name).
fn order(command: &str) -> &'static [&'static str] {
    match command {
        crate::addons::cutting::WEED => &["margin", "box", "direction", "count", "spacing", "gap", "weight", "swatch"],
        crate::addons::cutting::BANNER => {
            &["hem", "top", "right", "bottom", "left", "grommets", "spacing", "grommetInset", "diameter", "weight", "swatch"]
        }
        // The public Trap options' order.
        "object.pathfinder.trap" => &["thickness", "heightWidth", "tintReduction", "trapsWithProcessColor", "reverseTraps"],
        "addon.color.printFixes" => &["inkLimit", "excludePureBlack", "removeWhiteOverprint", "overprintBlack", "textOnly", "fill", "stroke"],
        crate::addons::typetools::RELEASE => &["mode", "keepOriginal"],
        crate::addons::image::REMOVE_BACKGROUND | crate::addons::image::COLOR_MASK => {
            &["background", "colors", "contiguous", "match", "tolerance", "softness"]
        }
        crate::addons::cutting::BRIDGES => &["width", "count", "direction", "keepOriginal"],
        _ => &[],
    }
}

/// The parameters of the commands this dialog edits that pick one of some values.
fn choices(command: &str, key: &str) -> Option<form::Choices> {
    match (command, key) {
        ("type.pathOptions", "effect") => Some(crate::menus::PATH_EFFECTS),
        ("type.pathOptions", "alignToPath") => Some(PATH_ALIGN),
        (crate::addons::typetools::RELEASE, "mode") => Some(crate::addons::typetools::RELEASE_MODES),
        (crate::addons::cutting::BRIDGES, "direction") => Some(crate::addons::cutting::BRIDGE_DIRECTIONS),
        ("text.areaOptions", "fit") => Some(AREA_FIT),
        ("text.areaOptions", "firstBaseline") => Some(FIRST_BASELINE),
        ("text.areaOptions", "verticalAlign") => Some(VERTICAL_ALIGN),
        (crate::addons::cutting::WEED, "direction") => Some(crate::addons::cutting::WEED_DIRECTIONS),
        ("graph.setType", "type") => Some(GRAPH_KINDS.as_slice()),
        ("graph.setType", "valueAxis") => Some(GRAPH_VALUE_AXIS),
        ("graph.setType", "tickLength" | "rightTickLength" | "categoryTickLength") => Some(GRAPH_TICK_LENGTH),
        ("graph.setType", "pieLegend") => Some(PIE_LEGEND),
        ("graph.setType", "piePosition") => Some(PIE_POSITION),
        ("graph.setType", "pieSort") => Some(PIE_SORT),
        (crate::addons::image::REMOVE_BACKGROUND | crate::addons::image::COLOR_MASK, "background") => Some(crate::addons::image::BACKGROUNDS),
        (crate::addons::image::REMOVE_BACKGROUND | crate::addons::image::COLOR_MASK, "match") => Some(crate::addons::image::MATCHES),
        _ => None,
    }
}

/// Object › Rasterize › Color Model: the values `object.rasterize` takes, the document's own
/// colour model first (#5660).
fn rasterize_color_models(command: &str, key: &str, cmyk: bool) -> Option<form::Choices> {
    match (command, key) {
        ("object.rasterize", "colorModel") => Some(if cmyk { RASTERIZE_CMYK } else { RASTERIZE_RGB }),
        _ => None,
    }
}

/// Object › Rasterize › Color Model in an RGB and in a CMYK document.
const RASTERIZE_RGB: form::Choices = &[("RGB", "rgb"), ("Grayscale", "grayscale"), ("Bitmap", "bitmap")];
const RASTERIZE_CMYK: form::Choices = &[("CMYK", "cmyk"), ("Grayscale", "grayscale"), ("Bitmap", "bitmap")];

/// Type on a Path Options › Align to Path.
const PATH_ALIGN: form::Choices = &[("Ascender", "ascender"), ("Descender", "descender"), ("Center", "center"), ("Baseline", "baseline")];

/// Area Type Options › Fit (Block is Dynamic Text's, as `type.dynamicText` sets it).
const AREA_FIT: form::Choices = &[("None", "none"), ("Auto Size", "autoHeight"), ("Shrink Text to Fit", "shrinkText"), ("Block", "block")];

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

    /// Area Type Options shows the gutter between rows beside the gutter, and OK sets a gutter
    /// between rows of its own.
    #[test]
    fn area_type_options_dialog_sets_the_gutter_between_rows() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 20, "text": "Hello", "area": {"width": 200, "height": 200}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        crate::menus::invoke(&mut app, "text.areaOptions", json!({}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("rowGutter"), Some(&json!(18.0)));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        app.ui.dialog.as_mut().unwrap().fields.insert("rowGutter".into(), json!(30.0));
        crate::dialogs::confirm(&mut app).unwrap();
        let area = |app: &VectorcraftApp| {
            let NodeKind::Text(t) = &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
            (t.area.gutter, t.area.row_gutter)
        };
        assert_eq!(area(&app), (18.0, Some(30.0)));
        // Rows with a gutter of their own keep it when only the Gutter changes.
        crate::menus::invoke(&mut app, "text.areaOptions", json!({}));
        app.ui.dialog.as_mut().unwrap().fields.insert("gutter".into(), json!(12.0));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(area(&app), (12.0, Some(30.0)));
        // Without one, a changed Gutter is the rows' too.
        app.run("text.areaOptions", json!({"gutter": 18})).unwrap();
        crate::menus::invoke(&mut app, "text.areaOptions", json!({}));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        app.ui.dialog.as_mut().unwrap().fields.insert("gutter".into(), json!(10.0));
        // The Row Gutter shown follows it.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("rowGutter"), Some(&json!(10.0)));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(area(&app), (10.0, None));
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

    /// Object › Rasterize › Color Model is a dropdown of the models `object.rasterize` takes, the
    /// document's own first (#5660): every value runs, and picking Grayscale rasterizes in grays.
    #[test]
    fn rasterize_color_model_is_a_dropdown_of_the_models_the_command_takes() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        for (mode, own) in [("rgb", "RGB"), ("cmyk", "CMYK")] {
            let models = rasterize_color_models("object.rasterize", "colorModel", mode == "cmyk").unwrap();
            assert_eq!(models.first(), Some(&(own, mode)), "the document's own model first");
            for (_, value) in models {
                let mut s = Session::new();
                s.execute("file.new", &json!({"width": 100, "height": 100, "colorMode": mode})).unwrap();
                s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
                s.execute("object.rasterize", &json!({"ppi": 72, "colorModel": value})).unwrap_or_else(|e| panic!("{mode} {value}: {e}"));
            }
        }
        assert!(rasterize_color_models("object.rasterize", "background", false).is_none());
        assert!(rasterize_color_models("document.rasterEffectsSettings", "colorModel", false).is_none());
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"color": "#ff0000", "ids": [id]})).unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        crate::menus::invoke(&mut app, "object.rasterize", json!({}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("colorModel"), "rgb");
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                crate::dialogs::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(5);
        assert!(h.query_by_role_and_label(Role::TextInput, "Color Model:").is_none(), "not a text field");
        h.get_by_role_and_label(Role::ComboBox, "Color Model:").click();
        h.run_steps(3);
        h.get_by_label("Grayscale").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.dialog.as_ref().unwrap().str("colorModel"), "grayscale");
        let app = h.state_mut();
        crate::dialogs::confirm(app).unwrap();
        let st = app.session.active().unwrap();
        let key = st.selection.objects.iter().find_map(|id| match st.doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::Image(i)) => Some(i.key.clone()),
            _ => None,
        });
        let img = image::load_from_memory(&st.doc.images[&key.expect("the rectangle became an image")].bytes).unwrap().to_rgba8();
        let p = img.get_pixel(img.width() / 2, img.height() / 2).0;
        assert!(p[0] == p[1] && p[1] == p[2] && p[0] < 128, "red in grayscale: {p:?}");
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

    /// Text Wrap Options › Offset is a distance, and its range is the one `object.textWrap.options`
    /// accepts: the command takes the range's ends and refuses an Offset outside them.
    #[test]
    fn text_wrap_offset_is_a_distance_within_the_commands_range() {
        assert!(lengths("object.textWrap.options").contains(&"offset"));
        assert!(!lengths("object.textWrap.options").contains(&"invert"));
        let range = length_range("object.textWrap.options", "offset").expect("a range");
        assert_eq!(range, vectorcraft_engine::TEXT_WRAP_OFFSET_RANGE);
        assert_eq!(length_range("object.textWrap.options", "invert"), None);
        assert_eq!(length_range("text.areaOptions", "inset"), None);
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        for end in [*range.end(), *range.start()] {
            assert_eq!(s.execute("object.textWrap.options", &json!({ "offset": end })).unwrap()["offset"], json!(end));
        }
        for sent in [5000.0, -5000.0] {
            assert!(s.execute("object.textWrap.options", &json!({ "offset": sent })).is_err(), "{sent}");
        }
    }

    /// Replaces the text of the dialog's Offset field with `text`, leaving the field focused.
    fn type_offset(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, text: &str) {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let all = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        h.get_by_role_and_label(Role::TextInput, "Offset:").focus();
        h.run_steps(2);
        h.key_press_modifiers(all, egui::Key::A);
        h.get_by_role_and_label(Role::TextInput, "Offset:").type_text(text);
        h.run_steps(2);
    }

    /// Replaces the text of the dialog's Offset field with `text` and presses Enter.
    fn enter_offset(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, text: &str) {
        type_offset(h, text);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    }

    /// Text Wrap Options › Offset reads a bare number in the document's units and a typed unit as
    /// typed, and OK sends points. A distance typed outside the command's range, or text that
    /// doesn't read, is refused: the dialog stays open and the Offset isn't changed.
    #[test]
    fn text_wrap_offset_field_takes_document_units_and_refuses_values_outside_its_range() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("document.setUnits", json!({"units": "millimeters"})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("object.textWrap.make", json!({})).unwrap();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                crate::dialogs::show(app, ui.ctx());
            },
            app,
        );
        let offset = |h: &mut egui_kittest::Harness<'_, VectorcraftApp>| {
            h.state_mut().session.execute("object.textWrap.options", &json!({})).unwrap()["offset"].as_f64().unwrap()
        };
        // The ends as the field shows them in millimeters (352.7778 mm is 1000.00006 pt) read
        // back as the ends.
        let ends = [("-1000 pt", -1000.0), ("352.7778", 1000.0), ("-352.7778", -1000.0), ("1000 pt", 1000.0)];
        for (text, want) in [("10", 10.0 * 72.0 / 25.4), ("-12 pt", -12.0)].into_iter().chain(ends) {
            crate::menus::invoke(h.state_mut(), "object.textWrap.options", json!({}));
            h.run_steps(3);
            enter_offset(&mut h, text);
            assert!(h.state().ui.dialog.is_none(), "{text}: OK applied it");
            assert!((offset(&mut h) - want).abs() < 1e-9, "{text}: {} pt", offset(&mut h));
        }
        for text in ["1000.5 pt", "-2000", "abc"] {
            crate::menus::invoke(h.state_mut(), "object.textWrap.options", json!({}));
            h.run_steps(3);
            enter_offset(&mut h, text);
            let d = h.state().ui.dialog.as_ref().unwrap_or_else(|| panic!("{text}: the dialog stays open"));
            assert_eq!(d.fields.get("offset"), Some(&json!(1000.0)), "{text}: the field keeps its value");
            crate::dialogs::cancel(h.state_mut());
            assert_eq!(offset(&mut h), 1000.0, "{text}: the Offset is unchanged");
        }
        // A click on OK with a value outside the range still in the field is refused too.
        crate::menus::invoke(h.state_mut(), "object.textWrap.options", json!({}));
        h.run_steps(3);
        type_offset(&mut h, "2000 pt");
        click_ok(&mut h);
        assert!(h.state().ui.dialog.is_some(), "OK leaves the dialog open");
        assert_eq!(offset(&mut h), 1000.0, "the Offset is unchanged");
        // After it, a value inside the range committed with Tab and OK pressed from the keyboard
        // apply.
        type_offset(&mut h, "-7 pt");
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label("OK").focus();
        }
        h.run_steps(2);
        h.key_press(egui::Key::Space);
        h.run_steps(3);
        assert!(h.state().ui.dialog.is_none(), "OK from the keyboard applied it");
        assert_eq!(offset(&mut h), -7.0);
        // One inside it applies.
        crate::menus::invoke(h.state_mut(), "object.textWrap.options", json!({}));
        h.run_steps(3);
        type_offset(&mut h, "-5 pt");
        click_ok(&mut h);
        assert!(h.state().ui.dialog.is_none(), "OK applied it");
        assert_eq!(offset(&mut h), -5.0);
    }

    /// In a document in picas, Text Wrap Options › Offset reads a bare number as the other length
    /// fields in picas do: in points with Preferences › Units › Numbers Without Units Are Points on
    /// (the default), in picas with it off. A typed unit is read as typed either way.
    #[test]
    fn text_wrap_offset_in_picas_follows_numbers_without_units_are_points() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("document.setUnits", json!({"units": "picas"})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("object.textWrap.make", json!({})).unwrap();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                crate::dialogs::show(app, ui.ctx());
            },
            app,
        );
        let apply = |h: &mut egui_kittest::Harness<'_, VectorcraftApp>, text: &str| {
            crate::menus::invoke(h.state_mut(), "object.textWrap.options", json!({}));
            h.run_steps(3);
            enter_offset(h, text);
            assert!(h.state().ui.dialog.is_none(), "{text}: OK applied it");
            h.state_mut().session.execute("object.textWrap.options", &json!({})).unwrap()["offset"].as_f64().unwrap()
        };
        assert_eq!(apply(&mut h, "10"), 10.0, "on by default: a bare number is in points");
        assert_eq!(apply(&mut h, "2p6"), 30.0, "picas and points as typed");
        assert_eq!(apply(&mut h, "-1in"), -72.0, "a typed unit wins");
        crate::widgets::set_bare_numbers_are_points(&h.ctx, false);
        assert_eq!(apply(&mut h, "10"), 120.0, "off: a bare number is in picas");
        assert_eq!(apply(&mut h, "-12 pt"), -12.0, "a typed unit wins");
    }

    /// Clicks the dialog's OK button with the pointer, the press and the release in frames of
    /// their own.
    fn click_ok(h: &mut egui_kittest::Harness<'_, VectorcraftApp>) {
        use egui_kittest::kittest::Queryable;
        let ok = h.get_by_label("OK").rect().center();
        let (button, modifiers) = (egui::PointerButton::Primary, egui::Modifiers::default());
        h.event(egui::Event::PointerMoved(ok));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: ok, button, pressed: true, modifiers });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: ok, button, pressed: false, modifiers });
        h.run_steps(3);
    }
}
