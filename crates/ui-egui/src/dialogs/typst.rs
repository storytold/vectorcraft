//! Type › Typst…: Typst markup typeset into the document (`type.typst.insert`), or, with a Typst
//! object selected, that object's source to change and typeset again (`type.typst.edit`). OK
//! typesets; when Typst reports an error the dialog stays open and shows it under the source.
//!
//! Nothing is typeset until Enable Typst (Preferences › Type) is on; the dialog has the same
//! switch, and says that Typst downloads the packages markup imports.
//!
//! Fields: `source`, `preamble`, `fontSize` (pt), `__id` (the Typst object edited; null to insert)
//! and `__error` (Typst's last message).

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Type › Typst….
pub const KIND: &str = "typst";

/// Width of the label column.
const LABEL_W: f32 = 82.0;
/// Width of the source and preamble boxes.
const BOX_W: f32 = 440.0;
/// What the source box starts with when inserting.
const EXAMPLE: &str = "$ sum_(k=1)^n k = (n(n+1))/2 $";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.fields.get("__id").is_some_and(|v| !v.is_null()) { tl!("Edit Typst") } else { tl!("Insert Typst") }.into(),
    body,
    confirm,
    min_width: 480.0,
    ..DialogSpec::FORM
};

/// Open Type › Typst…: on the selected Typst object's source, else to insert.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let selected = vectorcraft_engine::cmd::typst::selected(&app.session).unwrap_or_default();
    let fields = match selected[..] {
        [] => json!({
            "source": EXAMPLE,
            "preamble": "",
            "fontSize": vectorcraft_engine::cmd::typst::DEFAULT_FONT_SIZE,
            "__id": null,
        }),
        [id] => {
            let i = app.session.execute("type.typst.info", &json!({ "id": id.0 })).map_err(|e| e.to_string())?;
            json!({ "source": i["source"], "preamble": i["preamble"], "fontSize": i["fontSize"], "__id": i["id"] })
        }
        // Not a new object either: which one to edit is unclear.
        _ => return Err("select one Typst object".into()),
    };
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

/// A monospaced box `rows` lines tall bound to `d.fields[key]`. Enter adds a line.
fn code(ui: &mut egui::Ui, d: &mut Dialog, key: &str, rows: usize) {
    let t = Tokens::get(ui.ctx());
    let mut s = d.str(key);
    let id = ui.id().with(("typst-code", key));
    // The source box (the first) takes the keyboard focus when the dialog opens.
    widgets::take_dialog_focus(ui, id, &s);
    let r = egui::Frame::NONE
        .fill(t.input)
        .stroke(egui::Stroke::new(1.0, t.input_border))
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            crate::theme::strong_selected_text(ui);
            let edit = egui::TextEdit::multiline(&mut s)
                .id(id)
                .frame(egui::Frame::NONE)
                .desired_width(BOX_W)
                .desired_rows(rows)
                .font(egui::TextStyle::Monospace);
            ui.add(edit)
        });
    if r.inner.changed() {
        d.fields.insert(key.into(), Value::String(s));
    }
    if r.inner.has_focus() {
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    widgets::label_row(ui, "Text:", LABEL_W, |_| {});
    code(ui, d, "source", 8);
    if let Some(e) = d.fields.get("__error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(e).monospace().size(11.0).color(t.error));
    }
    ui.add_space(6.0);
    widgets::label_row(ui, "Preamble:", LABEL_W, |_| {});
    code(ui, d, "preamble", 3);
    ui.add_space(6.0);
    widgets::label_row(ui, "Size:", LABEL_W, |ui| {
        if let Some(v) = widgets::plain_field(ui, "typst-size", d.f64("fontSize", 11.0), "pt", 1, 64.0) {
            d.fields.insert("fontSize".into(), json!(v.clamp(1.0, 1296.0)));
        }
    });
    ui.add_space(8.0);
    let on = app.session.prefs.enable_typst;
    if widgets::check(ui, "Enable Typst", on, true) {
        // A refusal (automation confined to its roots) shows in the status bar.
        let _ = app.run("prefs.set", json!({ "key": "enableTypst", "value": !on }));
    }
    ui.label(
        egui::RichText::new(tl!("Runs the typst program, which downloads the @preview packages the markup imports.")).size(11.0).color(t.text_dim),
    );
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = json!({ "source": d.str("source"), "preamble": d.str("preamble"), "fontSize": d.f64("fontSize", 11.0) });
    let id = d.fields.get("__id").filter(|v| !v.is_null()).cloned();
    let cmd = match id {
        Some(id) => {
            p["id"] = id;
            "type.typst.edit"
        }
        None => "type.typst.insert",
    };
    match app.run(cmd, p) {
        Ok(v) => {
            // The message of a failed try no longer applies.
            if d.fields.get("__error").and_then(Value::as_str).is_some_and(|e| !e.is_empty()) {
                app.status(String::new());
            }
            app.ui.dialog = None;
            Ok(v)
        }
        // The dialog stays open with the message, to fix the source.
        Err(e) => {
            if let Some(open) = app.ui.dialog.as_mut().filter(|o| o.kind == KIND) {
                open.fields.insert("__error".into(), json!(e));
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_typst() -> bool {
        std::process::Command::new("typst").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    #[test]
    fn inserts_then_reopens_on_the_selection_to_edit_and_keeps_errors_open() {
        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.session.prefs.enable_typst = true;
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        open(&mut app).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.kind.as_str(), d.str("source").as_str()), (KIND, EXAMPLE));
        assert!(d.fields["__id"].is_null());
        assert_eq!((SPEC.heading)(&d), "Insert Typst");
        if !have_typst() {
            eprintln!("typst isn't installed: the rest is skipped");
            return;
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        // Open again on the new object: Edit, with its source.
        open(&mut app).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert!(d.fields["__id"].is_u64());
        assert_eq!((SPEC.heading)(&d), "Edit Typst");
        // An error keeps the dialog open, showing Typst's message.
        app.ui.dialog.as_mut().unwrap().fields.insert("source".into(), json!("#nope"));
        assert!(super::super::confirm(&mut app).is_err());
        let d = app.ui.dialog.clone().unwrap();
        assert!(d.str("__error").contains("line 1:"), "{:?}", d.fields);
    }

    #[test]
    fn two_typst_objects_selected_open_nothing() {
        if !have_typst() {
            return;
        }
        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.session.prefs.enable_typst = true;
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("type.typst.insert", &json!({"source": "a", "at": [100, 100]})).unwrap();
        app.session.execute("type.typst.insert", &json!({"source": "b", "at": [300, 100]})).unwrap();
        app.session.execute("select.all", &json!({})).unwrap();
        assert_eq!(open(&mut app).unwrap_err(), "select one Typst object");
        assert!(app.ui.dialog.is_none());
    }
}
