//! Edit → Edit Colors → Recolor with Preset: before Recolor Artwork opens on a preset (an n-color
//! job or Color Library), this dialog asks whether to limit the new colors to a swatch library
//! (`ui.recolorDialog {preset: true, colors?, library?}`, without `group`). OK opens Recolor Artwork with the
//! preset's colors and the library chosen (None: no limit); Cancel opens nothing.
//!
//! Fields: `limitTo` (a library id, `"document"` for the document's swatches, `""` for None), as
//! Recolor Artwork's.

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use super::recolor::{cancel_open, library_choices, library_param};
use super::swatch_options::{NO_WINDOW, field_label, hosted};
use crate::VectorcraftApp;

/// The dialog kind of Recolor with Preset.
pub const KIND: &str = "recolorPreset";

/// Open the dialog for the preset of `p` (`colors`: a count; `library`: the library it starts on,
/// "" the first library) on the selected art.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if st.selection.is_empty() {
        return Err("select artwork to recolor".into());
    }
    let document = st.uid;
    if p.get("group").is_some_and(|g| !g.is_null()) {
        return Err("`group` does not apply to a preset".into());
    }
    let colors = count(p.get("colors"))?;
    let limit = library_param(app, p)?;
    let ctx = app.ui_ctx.clone().ok_or(NO_WINDOW)?;
    // A Recolor Artwork this one opens over ends as Cancel ends it, its preview rolled back.
    cancel_open(app);
    let options = library_choices(app).into_iter().map(|(key, name)| (key, Label::from(name))).collect();
    let mut fields = DialogFields::new();
    fields.set("limitTo", limit.clone());
    let rows = vec![Param::new("limitTo", field_label(tl!("Limit to Library:")), ParamKind::choice(options, Some(limit)))];
    let dialog = hosted(&ctx, KIND, tl!("Recolor with Preset"), fields, rows, false);
    // Recolor Artwork takes the dialog's place; when it cannot open, this dialog stays.
    let ok = move |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        if !app.session.active().is_some_and(|active| active.uid == document) {
            return Err(tl!("Open the original drawing to continue with Recolor with Preset.").to_owned());
        }
        let mut p = serde_json::Map::new();
        if let Some(n) = colors {
            p.insert("colors".into(), json!(n));
        }
        if let Some(limit) = answer.fields.text("limitTo").filter(|l| !l.is_empty()) {
            p.insert("library".into(), json!(limit));
        }
        super::recolor::open(app, &Value::Object(p)).map(|_| ())
    };
    super::host::open_held(app, dialog, ok).ok_or(NO_WINDOW)?;
    Ok(Value::Null)
}

/// The preset's count of new colors: none for Color Library, else at least 1.
fn count(v: Option<&Value>) -> Result<Option<u64>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n.as_u64().filter(|n| *n >= 1).map(Some).ok_or_else(|| "`colors` must be a count of at least 1".into()),
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::{HeldOk, dialog_id};
    use vectorcraft_engine::Session;

    use super::*;

    #[test]
    fn recolor_preset_does_not_open_artwork_in_another_drawing() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        for color in ["#ff0000", "#0000ff"] {
            app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
            let shape = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
            app.run("paint.setFill", json!({"ids": [shape], "color": color})).unwrap();
            app.run("select.set", json!({"ids": [shape]})).unwrap();
        }
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("ui.recolorDialog", json!({"preset": true, "colors": 1})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        let host = app.dialogs().unwrap();
        let id = dialog_id(KIND);
        host.with(|h| h.set_json_field(&ctx, id, "limitTo", json!("document"))).unwrap();
        assert!(!host.with(|h| h.confirm_by_id(&ctx, id)), "the preset waits for the app");

        app.run("document.activate", json!({"index": 1})).unwrap();
        let target_undo = app.session.active().unwrap().history.undo.len();
        super::super::host::pump(&mut app, &ctx);
        assert!(app.color_dialogs.recolor.is_none(), "the held answer must not open Recolor Artwork on the other drawing");
        assert_eq!(app.session.active().unwrap().history.undo.len(), target_undo);
        assert!(matches!(host.with(|h| h.held_state(id)), Some(HeldOk::Refused(_))));

        app.run("document.activate", json!({"index": 0})).unwrap();
        assert!(super::super::tests::confirm(&mut app), "the original preset draft can open Recolor Artwork");
        assert!(app.color_dialogs.recolor.is_some());
        let limit = super::super::recolor::limit_to(&host, dialog_id(super::super::recolor::KIND));
        assert_eq!(limit.as_deref(), Some("document"));
    }
}
