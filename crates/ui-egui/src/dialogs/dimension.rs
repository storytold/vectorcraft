//! Dimension Tool Options, the tool's control strip, and the Properties panel's Dimension section.
//! Dialog values are drafts until Save, which sets the style of new dimensions.
use super::DialogSpec;
use crate::{VectorcraftApp, state::Dialog, theme::Tokens, widgets};
use egui::{RichText, Ui};
use serde_json::{Value, json};
use vectorcraft_doc::dimension::DimensionOptions;

pub const KIND: &str = "dimensionOptions";
pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Dimension Tool Options").into(),
    body,
    confirm,
    ok: Some("Save"),
    min_width: 520.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

/// (value, label) of each choice.
type Choices = &'static [(&'static str, &'static str)];
const UNITS: Choices = &[
    ("in", "Inches"),
    ("mm", "Millimeters"),
    ("pt", "Points"),
    ("pc", "Picas"),
    ("px", "Pixels"),
    ("cm", "Centimeters"),
    ("m", "Meters"),
    ("ft", "Feet"),
    ("yd", "Yards"),
];
const LINE_TYPES: Choices = &[("solid", "Solid"), ("dashed", "Dashed"), ("dotted", "Dotted")];
const ARROWS: Choices = &[("filled", "Filled"), ("open", "Open"), ("tick", "Tick"), ("none", "None")];
const POSITIONS: Choices = &[("away", "Away from the Line"), ("between", "Between Line and Object")];
const ALIGNMENTS: Choices = &[("left", "Left"), ("center", "Center"), ("right", "Right")];

pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.tool_id() == "dimension" && app.session.tool_busy() {
        app.run("tool.select", json!({"tool":"selection"}))?;
        app.run("tool.select", json!({"tool":"dimension"}))?;
    }
    let style = app.session.tool_options_of("dimension")["style"].clone();
    let o: DimensionOptions = serde_json::from_value(style).unwrap_or_default();
    let fields = serde_json::to_value(o).map_err(|e| e.to_string())?.as_object().cloned().unwrap_or_default();
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({"dialog":KIND}))
}

fn read(fields: &serde_json::Map<String, Value>) -> Result<DimensionOptions, String> {
    let o: DimensionOptions = serde_json::from_value(Value::Object(fields.clone())).map_err(|e| e.to_string())?;
    o.validate()?;
    Ok(o)
}

/// A dropdown row: `fields[key]` is one of `choices`' values.
fn choose(ui: &mut Ui, fields: &mut serde_json::Map<String, Value>, key: &str, label: &str, choices: Choices) -> bool {
    let mut changed = false;
    widgets::label_row(ui, tl!(label), 96.0, |ui| {
        let current = fields.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let text = choices.iter().find(|(v, _)| *v == current).map_or(current.as_str(), |(_, label)| *label);
        if let Some(index) =
            widgets::dropdown_names(ui, ("dimension", key), tl!(text), &choices.iter().map(|(_, s)| tl!(*s)).collect::<Vec<_>>(), 150.0)
            && let Some((value, _)) = choices.get(index)
        {
            fields.insert(key.into(), json!(value));
            changed = true;
        }
    });
    changed
}
fn number(ui: &mut Ui, fields: &mut serde_json::Map<String, Value>, key: &str, label: &str, range: (f64, f64), suffix: &str) -> bool {
    let mut changed = false;
    widgets::label_row(ui, tl!(label), 96.0, |ui| {
        let value = fields.get(key).and_then(Value::as_f64).unwrap_or_default();
        if let Some(v) = widgets::range_field(ui, ("dimension", key), value, range.0..=range.1, suffix, 2, 90.0) {
            fields.insert(key.into(), json!(v));
            changed = true;
        }
    });
    changed
}
fn check(ui: &mut Ui, fields: &mut serde_json::Map<String, Value>, key: &str, label: &str) -> bool {
    let mut value = fields.get(key).and_then(Value::as_bool).unwrap_or_default();
    let changed = ui.checkbox(&mut value, tl!(label)).changed();
    if changed {
        fields.insert(key.into(), json!(value));
    }
    changed
}
fn color(ui: &mut Ui, fields: &mut serde_json::Map<String, Value>, key: &str) -> bool {
    let mut value: [u8; 3] = serde_json::from_value(fields.get(key).cloned().unwrap_or(Value::Null)).unwrap_or([0, 0, 0]);
    let changed = ui.push_id(key, |ui| ui.color_edit_button_srgb(&mut value).changed()).inner;
    if changed {
        fields.insert(key.into(), json!(value));
    }
    changed
}
fn precision(ui: &mut Ui, fields: &mut serde_json::Map<String, Value>, id: &str) -> bool {
    let before = fields.get("precision").and_then(Value::as_u64).unwrap_or(2) as u32;
    let mut value = before;
    ui.label(tl!("Precision:"));
    ui.name_inputs(tl!("Precision"), |ui| {
        craft_ui::ComboBox::from_id_salt(id).selected_text(value.to_string()).width(55.0).show_ui(ui, |ui| {
            for n in 0..=6 {
                ui.selectable_value(&mut value, n, n.to_string());
            }
        });
    });
    fields.insert("precision".into(), json!(value));
    value != before
}
fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.group(|ui| {
        ui.set_min_width(486.0);
        ui.label(RichText::new(tl!(title)).color(t.text_dim).strong());
        ui.add_space(5.0);
        body(ui);
    });
    ui.add_space(7.0);
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let f = &mut d.fields;
    section(ui, "Units & Scale", |ui| {
        ui.horizontal(|ui| {
            choose(ui, f, "units", "Units:", UNITS);
            precision(ui, f, "dimension-precision");
        });
        ui.horizontal(|ui| {
            widgets::label_row(ui, tl!("Scale:"), 96.0, |ui| {
                let mut scale = f.get("scale").and_then(Value::as_str).unwrap_or_default().to_owned();
                let changed = ui.name_inputs(tl!("Scale"), |ui| ui.add(egui::TextEdit::singleline(&mut scale).desired_width(72.0))).changed();
                if changed {
                    f.insert("scale".into(), json!(scale));
                }
                ui.name_inputs(tl!("Scale Presets"), |ui| {
                    craft_ui::ComboBox::from_id_salt("dimension-scales").selected_text(tl!("Presets")).width(76.0).show_ui(ui, |ui| {
                        for scale in ["1:1", "1:2", "1:5", "1:10", "1:20", "1:50", "1:100", "1:200", "2:1", "5:1", "10:1"] {
                            if ui.selectable_label(false, scale).clicked() {
                                f.insert("scale".into(), json!(scale));
                            }
                        }
                    });
                });
            });
            check(ui, f, "hideUnits", "Hide Units");
        });
        ui.label(RichText::new(tl!("Drawing : real size — 1:10 labels 1 unit on the page as 10.")).small());
    });
    section(ui, "Dimension Lines", |ui| {
        ui.horizontal(|ui| {
            number(ui, f, "lineWeight", "Line Weight:", (0.1, 100.0), "pt");
            choose(ui, f, "lineType", "Line Type:", LINE_TYPES);
            color(ui, f, "lineColor");
        });
        ui.horizontal(|ui| {
            choose(ui, f, "arrowStyle", "Arrow Style:", ARROWS);
            number(ui, f, "arrowScale", "Arrow Scale:", (1.0, 1000.0), "%");
        });
    });
    section(ui, "Extension Lines", |ui| {
        ui.horizontal(|ui| {
            number(ui, f, "extensionWeight", "Line Weight:", (0.1, 100.0), "pt");
            choose(ui, f, "extensionType", "Line Type:", LINE_TYPES);
            color(ui, f, "extensionColor");
        });
        ui.horizontal(|ui| {
            number(ui, f, "extensionOffset", "Offset:", (0.0, 10000.0), "pt");
            number(ui, f, "extensionLength", "Extension:", (0.0, 10000.0), "pt");
        });
        check(ui, f, "hideExtensions", "Hide Extension Lines");
    });
    section(ui, "Dimension Text", |ui| {
        widgets::label_row(ui, tl!("Font Family:"), 96.0, |ui| {
            let family = f.get("fontFamily").and_then(Value::as_str).unwrap_or_default().to_owned();
            if let Some(crate::font_menu::FontPick::Chosen(family, style)) =
                crate::font_menu::font_menu(ui, "dimension-font", &family, 280.0, Some("123.45"), crate::font_menu::MenuLook::of(app))
            {
                f.insert("fontFamily".into(), json!(family));
                f.insert("fontStyle".into(), json!(style.unwrap_or_else(|| "Regular".into())));
            }
        });
        widgets::label_row(ui, tl!("Font Style:"), 96.0, |ui| {
            let family = f.get("fontFamily").and_then(Value::as_str).unwrap_or_default().to_owned();
            let current = f.get("fontStyle").and_then(Value::as_str).unwrap_or_default().to_owned();
            let styles = vectorcraft_text::FontDb::global().styles(&family);
            let names: Vec<&str> = styles.iter().map(|s| s.as_str()).collect();
            if let Some(i) = widgets::dropdown_names(ui, "dimension-font-style", &current, &names, 180.0)
                && let Some(style) = styles.get(i)
            {
                f.insert("fontStyle".into(), json!(style));
            }
        });
        ui.horizontal(|ui| {
            number(ui, f, "fontSize", "Font Size:", (1.0, 1000.0), "pt");
            color(ui, f, "textColor");
        });
        ui.horizontal(|ui| {
            choose(ui, f, "textPosition", "Position:", POSITIONS);
            number(ui, f, "textOffset", "Offset:", (0.0, 10000.0), "pt");
        });
        choose(ui, f, "textAlignment", "Alignment:", ALIGNMENTS);
    });
    match read(f) {
        Ok(o) => {
            let sample = o.label(72.0, false).unwrap_or_default();
            let t = Tokens::get(ui.ctx());
            ui.label(RichText::new(format!("{}: {sample}", tl!("Preview (1 inch on the page)"))).color(t.text));
        }
        Err(e) => {
            ui.colored_label(Tokens::get(ui.ctx()).text, e);
        }
    }
    if ui.button(tl!("Reset")).clicked()
        && let Ok(Value::Object(fields)) = serde_json::to_value(DimensionOptions::default())
    {
        d.fields = fields;
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let o = read(&d.fields)?;
    let r = app.run("tool.setOption", json!({"tool":"dimension","values":{"style":o}}))?;
    app.ui.dialog = None;
    Ok(r)
}

/// The Dimension tool's strip: the kind of dimension, and Tool Options.
pub fn controls(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mode = app.session.tool_options()["mode"].as_str().unwrap_or("linear").to_owned();
    for (id, label) in [("linear", "Linear"), ("angular", "Angular"), ("radial", "Radial")] {
        if ui.selectable_label(mode == id, tl!(label)).clicked() {
            // Switching away cancels previews and unfinished picks.
            if let Err(e) = app
                .run("tool.select", json!({"tool":"selection"}))
                .and_then(|_| app.run("tool.select", json!({"tool":"dimension"})))
                .and_then(|_| app.run("tool.setOption", json!({"tool":"dimension","values":{"mode":id}})))
            {
                app.ui.status = e;
            }
        }
    }
    if ui.button(tl!("Tool Options…")).clicked()
        && let Err(e) = open(app)
    {
        app.ui.status = e;
    }
}

/// Always available while measuring, including workspaces with the main Control bar hidden.
pub fn floating(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.session.tool_id() != "dimension" || app.ui.dialog.is_some() {
        return;
    }
    egui::Window::new(tl!("Dimension Tool"))
        .id(egui::Id::unique("dimension-strip"))
        .default_pos(egui::pos2(90.0, 120.0))
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| controls(app, ui));
            let mode = app.session.tool_options()["mode"].as_str().unwrap_or("linear").to_owned();
            let hint = match mode.as_str() {
                "angular" => "Drag out from a corner, or click two straight lines, then click to place.",
                "radial" | "radius" | "diameter" => "Drag out from a circle or arc for its radius, inwards for its diameter.",
                _ => "Drag out from a segment, or click two points, then click to place.",
            };
            ui.label(RichText::new(tl!(hint)).small().color(Tokens::get(ctx).text_dim));
        });
}

/// The selected dimension, if one is selected alone: its id and style.
pub fn selected(app: &VectorcraftApp) -> Option<(u64, DimensionOptions)> {
    let st = app.session.active()?;
    let [id] = st.selection.objects.as_slice() else { return None };
    let d = st.doc.node(*id)?.extras().dimension.as_deref()?;
    Some((id.0, d.options.clone()))
}

/// The Properties panel's Dimension section: the selected dimension's style, Apply to All and Set
/// as Default. Its scale is set in the Tool Options.
pub fn properties_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((id, o)) = selected(app) else { return };
    let Ok(Value::Object(mut f)) = serde_json::to_value(&o) else { return };
    widgets::section_header(ui, tl!("Dimension"));
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= choose(ui, &mut f, "units", "Units:", UNITS);
    });
    ui.horizontal(|ui| changed |= precision(ui, &mut f, "props-dimension-precision"));
    ui.horizontal(|ui| {
        changed |= number(ui, &mut f, "lineWeight", "Line Weight:", (0.1, 100.0), "pt");
        changed |= color(ui, &mut f, "lineColor");
    });
    ui.horizontal(|ui| changed |= choose(ui, &mut f, "arrowStyle", "Arrow Style:", ARROWS));
    ui.horizontal(|ui| {
        changed |= number(ui, &mut f, "fontSize", "Font Size:", (1.0, 1000.0), "pt");
        changed |= color(ui, &mut f, "textColor");
    });
    ui.horizontal(|ui| changed |= choose(ui, &mut f, "textPosition", "Position:", POSITIONS));
    ui.horizontal(|ui| changed |= choose(ui, &mut f, "textAlignment", "Alignment:", ALIGNMENTS));
    if changed {
        match read(&f) {
            Ok(o) => {
                if let Err(e) = app.run("dimension.update", json!({"ids":[id],"options":o})) {
                    app.ui.status = e;
                }
            }
            Err(e) => app.ui.status = e,
        }
    }
    let w = (ui.available_width() - 6.0) / 2.0;
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("Apply to All"), w).clicked()
            && let Err(e) = app.run("dimension.applyToAll", json!({"id":id}))
        {
            app.ui.status = e;
        }
        if widgets::flat_button(ui, tl!("Set as Default"), w).clicked()
            && let Err(e) = app.run("tool.setOption", json!({"tool":"dimension","values":{"style":o}}))
        {
            app.ui.status = e;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dimension_dialog_fields_save_and_cancel() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("tool.options", json!({"tool":"dimension"})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        for label in ["Dimension Tool Options", "Units & Scale", "Dimension Lines", "Extension Lines", "Dimension Text", "Save"] {
            assert!(text.contains(label), "{label}: {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("scale".into(), json!("1:10"));
        d.fields.insert("units".into(), json!("pc"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.tool_options_of("dimension")["style"]["scale"], "1:10");
        assert_eq!(app.session.tool_options_of("dimension")["style"]["units"], "pc");
        open(&mut app).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("scale".into(), json!("1:0"));
        assert!(super::super::confirm(&mut app).is_err());
        app.ui.dialog = None;
        assert_eq!(app.session.tool_options_of("dimension")["style"]["scale"], "1:10");
    }

    /// A selected dimension shows its style in Properties, with Apply to All and Set as Default.
    #[test]
    fn properties_show_the_selected_dimension() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("dimension.create", json!({"a":[10,10],"b":[110,10]})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, properties_section);
        for label in ["Dimension", "Units:", "Apply to All", "Set as Default"] {
            assert!(text.contains(label), "{label}: {text}");
        }
    }
}
