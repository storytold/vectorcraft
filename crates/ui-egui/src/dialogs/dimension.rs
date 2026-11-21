//! Dimension Tool Options, the tool's control strip, and the Properties panel's Dimension section.
//! Dialog values are drafts until Save, which sets the style of new dimensions.
use super::DialogSpec;
use crate::{VectorcraftApp, state::Dialog, theme::Tokens, widgets};
use egui::{RichText, Ui};
use serde_json::{Value, json};
use vectorcraft_doc::Arrowhead;
use vectorcraft_doc::dimension::{ArrowStyle, DimensionOptions};

pub const KIND: &str = "dimensionOptions";
pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Dimension Tool Options").into(),
    body,
    confirm,
    ok: Some("Save"),
    min_width: 520.0,
    max_width: Some(560.0),
    // The saved options leave `arrowHead` out when it is null.
    inputs: |_| &["arrowHead"],
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
/// The size of an Arrow Style preview: a line with the head at its right end. Line Arrow, the widest
/// head, spreads 20 pt across at the preview's head weight.
const ARROW_PREVIEW: egui::Vec2 = egui::vec2(40.0, 22.0);
/// One Arrow Style entry: the `arrowStyle` and `arrowHead` values it sets, and its name.
struct ArrowChoice {
    style: ArrowStyle,
    head: Option<Arrowhead>,
    name: &'static str,
}
impl ArrowChoice {
    /// The arrowhead the dimension line draws with this entry.
    fn drawn(&self) -> Option<Arrowhead> {
        self.head.clone().or_else(|| self.style.arrowhead())
    }
}
/// The Arrow Style entries: Filled, Open, Tick and None, then the other built-in stroke arrowheads
/// in the Stroke panel's order. A stroke arrowhead keeps `filled` as its `arrowStyle`, the head that
/// versions which do not read `arrowHead` draw when they draw the dimension again.
fn arrow_choices() -> Vec<ArrowChoice> {
    let named = [(ArrowStyle::Filled, "Filled"), (ArrowStyle::Open, "Open"), (ArrowStyle::Tick, "Tick"), (ArrowStyle::None, "None")];
    let heads = Arrowhead::ALL.into_iter().filter(|h| !named.iter().any(|(s, _)| s.arrowhead().as_ref() == Some(h)));
    named
        .into_iter()
        .map(|(style, name)| ArrowChoice { style, head: None, name })
        .chain(heads.map(|h| ArrowChoice { style: ArrowStyle::Filled, name: h.label(), head: Some(h) }))
        .collect()
}
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

/// For `ui.dialog.set`, as the Arrow Style menu sets them: an `arrowStyle` set alone draws that
/// style's own head, and a built-in head set as `arrowHead` sets both values of its Arrow Style
/// entry (`tick` with no `arrowHead` for `Slash`, `filled` with `Barbed` for `Barbed`). Returns
/// whether it set the field itself.
pub(super) fn note_set(d: &mut Dialog, field: &str, value: &Value) -> bool {
    match field {
        "arrowStyle" => {
            d.fields.insert("arrowHead".into(), Value::Null);
            false
        }
        "arrowHead" => {
            let head = built_in_head(value);
            if let Some(ch) = arrow_choices().into_iter().find(|ch| head.is_some() && ch.drawn() == head)
                && let (Ok(style), Ok(head)) = (serde_json::to_value(ch.style), serde_json::to_value(&ch.head))
            {
                d.fields.insert("arrowStyle".into(), style);
                d.fields.insert("arrowHead".into(), head);
                return true;
            }
            false
        }
        _ => false,
    }
}

/// The built-in stroke arrowhead `value` names. Only a short name is read: a draft may hold anything.
fn built_in_head(value: &Value) -> Option<Arrowhead> {
    let name = value.as_str().filter(|name| name.len() <= 64)?;
    serde_json::from_value::<Arrowhead>(Value::String(name.into())).ok().filter(|h| !matches!(h, Arrowhead::Custom(_)))
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
/// The Arrow Style dropdown: Filled, Open, Tick and None, then the other built-in stroke
/// arrowheads. Each style shows a drawn preview of its arrowhead before its name, in the open list
/// and in the closed field.
fn arrow_style(ui: &mut Ui, fields: &mut serde_json::Map<String, Value>, id: &str) -> bool {
    const STYLE: &str = "arrowStyle";
    const HEAD: &str = "arrowHead";
    let mut changed = false;
    widgets::label_row(ui, tl!("Arrow Style:"), 96.0, |ui| {
        let choices = arrow_choices();
        let style = fields.get(STYLE).cloned().and_then(|v| serde_json::from_value::<ArrowStyle>(v).ok());
        let head = fields.get(HEAD).and_then(built_in_head);
        let current = style.map(|s| head.or_else(|| s.arrowhead()));
        let shown = current.and_then(|c| choices.iter().position(|ch| ch.drawn() == c));
        let names: Vec<&str> = choices.iter().map(|ch| crate::panels::stroke::builtin_arrow_name(ch.name)).collect();
        let icon = |ui: &Ui, rect: egui::Rect, i: usize, ink: egui::Color32, _: bool| {
            // Only the rows in view render their previews.
            if ui.is_rect_visible(rect)
                && let Some(ch) = choices.get(i)
            {
                crate::panels::stroke::paint_arrow(ui, rect, ch.drawn().as_ref(), false, ink);
            }
        };
        if let Some(ch) =
            widgets::dropdown_icon_names_sized(ui, ("dimension", id), shown, &names, 150.0, ARROW_PREVIEW, icon).and_then(|i| choices.get(i))
            && let (Ok(style), Ok(head)) = (serde_json::to_value(ch.style), serde_json::to_value(&ch.head))
        {
            fields.insert(STYLE.into(), style);
            fields.insert(HEAD.into(), head);
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
                let changed = ui
                    .name_inputs(tl!("Scale"), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut scale).align(egui::Align2::LEFT_CENTER).desired_width(72.0))
                    })
                    .changed();
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
            arrow_style(ui, f, "arrowStyle");
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
            ui.label(RichText::new(format!("{} {sample}", crate::i18n::colon(tl!("Preview (1 inch on the page)")))).color(t.text));
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

/// The Dimension tool's strip: the kind of dimension, and Tool Options. In the Control bar
/// (`reserve` given), each button shows only while the row has room for it and `reserve` after it,
/// and the buttons after one left out are left out too; the floating strip (`None`) shows them all.
pub fn controls(app: &mut VectorcraftApp, ui: &mut Ui, reserve: Option<f32>) {
    let mut fit = reserve.map(crate::chrome::Fit::new);
    let mut room = |ui: &Ui, text: &str| fit.as_mut().is_none_or(|f| f.room(ui, &[button_width(ui, text)]));
    let mode = app.session.tool_options()["mode"].as_str().unwrap_or("linear").to_owned();
    for (id, label) in [("linear", "Linear"), ("angular", "Angular"), ("radial", "Radial")] {
        if room(ui, tl!(label)) && ui.selectable_label(mode == id, tl!(label)).clicked() {
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
    if room(ui, tl!("Tool Options…"))
        && ui.button(tl!("Tool Options…")).clicked()
        && let Err(e) = open(app)
    {
        app.ui.status = e;
    }
}

/// The width `ui.button` and `ui.selectable_label` give `text`: the text and the button padding on
/// both sides.
fn button_width(ui: &Ui, text: &str) -> f32 {
    widgets::text_width(ui, RichText::new(text).text_style(egui::TextStyle::Button)) + 2.0 * ui.spacing().button_padding.x
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
            ui.horizontal(|ui| controls(app, ui, None));
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
    ui.horizontal(|ui| changed |= arrow_style(ui, &mut f, "props-arrowStyle"));
    widgets::section_header(ui, tl!("Extension Lines"));
    ui.horizontal(|ui| {
        changed |= number(ui, &mut f, "extensionWeight", "Line Weight:", (0.1, 100.0), "pt");
        changed |= color(ui, &mut f, "extensionColor");
    });
    widgets::section_header(ui, tl!("Dimension Text"));
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
        // The default style has no `arrowHead`, and the dialog still takes one.
        assert!(!d.fields.contains_key("arrowHead") && super::super::accepts_field(d, "arrowHead"));
        d.fields.insert("scale".into(), json!("1:10"));
        d.fields.insert("units".into(), json!("pc"));
        d.fields.insert("arrowHead".into(), json!("Barbed"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.tool_options_of("dimension")["style"]["arrowHead"], "Barbed");
        assert_eq!(app.session.tool_options_of("dimension")["style"]["scale"], "1:10");
        assert_eq!(app.session.tool_options_of("dimension")["style"]["units"], "pc");
        open(&mut app).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("scale".into(), json!("1:0"));
        assert!(super::super::confirm(&mut app).is_err());
        app.ui.dialog = None;
        assert_eq!(app.session.tool_options_of("dimension")["style"]["scale"], "1:10");
        // An arrow style set by `ui.dialog.set` replaces the Barbed head, as the menu does.
        open(&mut app).unwrap();
        super::super::set_field(app.ui.dialog.as_mut().unwrap(), "arrowStyle".into(), json!("none"));
        super::super::confirm(&mut app).unwrap();
        let style = serde_json::from_value::<DimensionOptions>(app.session.tool_options_of("dimension")["style"].clone()).unwrap();
        assert_eq!((style.arrow_style, style.arrowhead()), (ArrowStyle::None, None));
        // A stroke head set by `ui.dialog.set` keeps Filled as the arrow style, as the menu does; an
        // unknown name leaves the arrow style as it was.
        open(&mut app).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        super::super::set_field(d, "arrowStyle".into(), json!("tick"));
        super::super::set_field(d, "arrowHead".into(), json!("Nope"));
        assert_eq!(d.fields["arrowStyle"], "tick");
        super::super::set_field(d, "arrowHead".into(), json!("Barbed"));
        super::super::confirm(&mut app).unwrap();
        let style = serde_json::from_value::<DimensionOptions>(app.session.tool_options_of("dimension")["style"].clone()).unwrap();
        assert_eq!((style.arrow_style, style.arrowhead()), (ArrowStyle::Filled, Some(Arrowhead::Barbed)));
        // The head of an older style sets that style and no `arrowHead`, as its menu entry does.
        open(&mut app).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        super::super::set_field(d, "arrowHead".into(), json!("Slash"));
        assert_eq!((&d.fields["arrowStyle"], &d.fields["arrowHead"]), (&json!("tick"), &Value::Null));
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

    /// Whether a frame drew a picture (a textured mesh) just before the text `name` on its line.
    fn picture_before(out: &egui::FullOutput, name: &str) -> bool {
        fn collect(s: &egui::Shape, texts: &mut Vec<(String, egui::Rect)>, pictures: &mut Vec<egui::Rect>) {
            match s {
                egui::Shape::Text(t) => texts.push((t.galley.text().to_owned(), t.visual_bounding_rect())),
                egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => pictures.push(m.calc_bounds()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, texts, pictures)),
                _ => {}
            }
        }
        let (mut texts, mut pictures) = (vec![], vec![]);
        for c in &out.shapes {
            collect(&c.shape, &mut texts, &mut pictures);
        }
        texts
            .iter()
            .filter(|(t, _)| t == name)
            .any(|(_, r)| pictures.iter().any(|p| p.max.x <= r.min.x + 1.0 && r.min.x - p.max.x < 16.0 && p.min.y < r.max.y && p.max.y > r.min.y))
    }

    /// The frame `f` draws, once its previews are loaded.
    fn frame(app: &mut VectorcraftApp, mut f: impl FnMut(&mut VectorcraftApp, &mut Ui)) -> egui::FullOutput {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.run_ui(egui::RawInput::default(), |ui| f(app, ui)).textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(app, ui));
        // Previews of rows that come into view in this frame load now.
        out.textures_delta.clear();
        out
    }

    /// The Arrow Style field of the Tool Options and of Properties shows the style's arrowhead,
    /// drawn, before its name (#8049).
    #[test]
    fn arrow_style_shows_a_drawn_arrowhead() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("tool.options", json!({"tool":"dimension"})).unwrap();
        let out = frame(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        assert!(picture_before(&out, "Filled"), "Tool Options");
        app.ui.dialog = None;
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("dimension.create", json!({"a":[10,10],"b":[110,10],"options":{"arrowStyle":"tick"}})).unwrap();
        let out = frame(&mut app, properties_section);
        assert!(picture_before(&out, "Tick"), "Properties");
    }

    /// A dimension drawn with a stroke arrowhead shows that head's name and preview in its Arrow
    /// Style field (#3222).
    #[test]
    fn arrow_style_shows_a_stroke_arrowhead() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("dimension.create", json!({"a":[10,10],"b":[110,10],"options":{"arrowHead":"HalfArrowLeft"}})).unwrap();
        let out = frame(&mut app, properties_section);
        assert!(picture_before(&out, "Half Arrow (left)"), "Properties");
    }

    /// The Arrow Style list names each arrowhead once, with the four older styles first, and each
    /// stroke arrowhead keeps Filled as its `arrowStyle`.
    #[test]
    fn arrow_style_list_names_each_style_once() {
        let choices = arrow_choices();
        let names: Vec<&str> = choices.iter().map(|c| c.name).collect();
        assert_eq!(names[..4], ["Filled", "Open", "Tick", "None"]);
        for name in ["Triangle", "Triangle (open)", "Slash"] {
            assert!(!names.contains(&name), "{name} is listed as Filled, Open or Tick");
        }
        assert_eq!(names.len(), Arrowhead::ALL.len() + 1, "None and each built-in head: {names:?}");
        let drawn: Vec<_> = choices.iter().map(|c| format!("{:?}", c.drawn())).collect();
        assert_eq!(drawn.iter().collect::<std::collections::HashSet<_>>().len(), drawn.len(), "{drawn:?}");
        assert!(choices[4..].iter().all(|c| c.style == ArrowStyle::Filled && c.head.is_some()));
    }
}
