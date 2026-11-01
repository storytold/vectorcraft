//! New Character Style and New Paragraph Style: the new style's name. OK makes the style from the
//! text that was selected when the dialog opened (`charStyle.new`, `paraStyle.new`).
//!
//! Fields: `name`, `__para` (true for a paragraph style) and `__target` (the `id`, `start` and
//! `end` of the Type tool's selection, empty for the selected objects, and the `attrs` that text had
//! when the dialog opened).

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of New Character Style and New Paragraph Style.
pub const KIND: &str = "newTextStyle";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 320.0, ..DialogSpec::FORM };

fn para(d: &Dialog) -> bool {
    d.fields.get("__para").and_then(Value::as_bool).unwrap_or(false)
}

fn heading(d: &Dialog) -> String {
    if para(d) { tl!("New Paragraph Style") } else { tl!("New Character Style") }.into()
}

/// Open New Character Style (`para`: New Paragraph Style) with the name `charStyle.new` would give
/// the style, to make it from the text `target` names.
pub fn open(app: &mut VectorcraftApp, para: bool, target: Value) -> Result<Value, String> {
    let d = &app.session.active().ok_or("no document open")?.doc;
    let name = vectorcraft_engine::cmd::textstyles::new_style_name(d, para);
    let mut target = target;
    let selected_attrs = vectorcraft_engine::cmd::textstyles::target_style_attrs(&app.session, &target, para);
    if let Some(attrs) = selected_attrs.or_else(|| (!para).then(|| vectorcraft_engine::cmd::textstyles::default_new_style_attrs(&app.session, false)))
    {
        target["attrs"] = Value::Object(attrs);
    }
    let mut fields = Dialog::new(KIND, json!({ "name": name, "__para": para, "__target": target })).fields;
    if !para {
        fields.insert("section".into(), json!("general"));
        fields.insert("__smallCapsSize".into(), json!(d.setup.small_caps_size));
        fields.insert("__superscript".into(), json!(d.setup.superscript));
        fields.insert("__subscript".into(), json!(d.setup.subscript));
        let attrs = target.get("attrs").and_then(Value::as_object).cloned().unwrap_or_default();
        fields.extend(super::char_style_options::fields_from_attrs(&attrs));
        fields.insert("__attrs".into(), Value::Object(attrs));
    }
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({ "dialog": KIND }))
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    if !para(d) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                for (id, title) in [("general", tl!("General")), ("basic", tl!("Basic Character Formats"))] {
                    if ui.selectable_label(d.str("section") == id, title).clicked() {
                        d.fields.insert("section".into(), json!(id));
                    }
                }
            });
            ui.separator();
            if d.str("section") == "basic" {
                super::char_style_options::basic_body(ui, d);
            } else {
                ui.vertical(|ui| {
                    grid(ui, |ui| {
                        label(ui, tl!("Style Name:"));
                        form::text(ui, d, "name", 190.0);
                        ui.end_row();
                    });
                    if let Ok(attrs) = super::char_style_options::edited_attrs(d) {
                        for (key, value) in attrs {
                            ui.label(format!("{}: {}", super::humanize(&key), value));
                        }
                    }
                });
            }
        });
        ui.add_enabled_ui(d.str("section") == "basic", |ui| {
            if ui.button(tl!("Reset Panel")).clicked() {
                super::char_style_options::reset_basic(d);
            }
        });
        return false;
    }
    grid(ui, |ui| {
        label(ui, tl!("Style Name:"));
        form::text(ui, d, "name", 190.0);
        ui.end_row();
    });
    false
}

/// OK: make the style with the name. A name another style has, or an empty one, keeps the dialog
/// open. The panel highlights the new style and scrolls it into view when it next draws.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = d.fields.get("__target").filter(|t| t.is_object()).cloned().unwrap_or_else(|| json!({}));
    params["name"] = json!(d.str("name"));
    if !para(d) {
        params["attrs"] = Value::Object(super::char_style_options::edited_attrs(d)?);
    }
    let r = app.run(if para(d) { "paraStyle.new" } else { "charStyle.new" }, params);
    if let Ok(v) = &r {
        app.ui.dialog = None;
        if let Some((uid, name)) = app.session.active().and_then(|doc| v["name"].as_str().map(|name| (doc.uid, name))) {
            let para = para(d);
            app.ui.made_text_style.retain(|(pending_uid, pending_para, _)| *pending_uid != uid || *pending_para != para);
            app.ui.made_text_style.push((uid, para, name.to_string()));
        }
    }
    r
}
