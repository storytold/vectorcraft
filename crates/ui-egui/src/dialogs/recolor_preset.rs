//! Edit → Edit Colors → Recolor with Preset: before Recolor Artwork opens on a preset (an n-color
//! job or Color Library), this dialog asks whether to limit the new colors to a swatch library
//! (`ui.recolorDialog {preset: true, colors?, library?}`, without `group`). OK opens Recolor Artwork with the
//! preset's colors and the library chosen (None: no limit); Cancel opens nothing.
//!
//! Fields: `limitTo` (a library id, `"document"` for the document's swatches, `""` for None), as
//! Recolor Artwork's.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use super::recolor::{cancel_open, library_choices, library_param};
use super::swatch_options::{NO_WINDOW, field_label, hosted};
use crate::VectorcraftApp;

/// The dialog kind of Recolor with Preset.
pub const KIND: &str = "recolorPreset";
const OPENING: &str = "__opening";

/// Open the dialog for the preset of `p` (`colors`: a count; `library`: the library it starts on,
/// "" the first library) on the selected art.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if st.selection.is_empty() {
        return Err("select artwork to recolor".into());
    }
    let (document, reverts) = (st.uid, st.reverts);
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
    fields.set(OPENING, FieldValue::Json(json!({"uid": document, "reverts": reverts, "colors": colors})));
    let rows = vec![Param::new("limitTo", field_label(tl!("Limit to Library:")), ParamKind::choice(options, Some(limit)))];
    let dialog = hosted(&ctx, KIND, tl!("Recolor with Preset"), fields, rows, false);
    // Recolor Artwork takes the dialog's place; when it cannot open, this dialog stays.
    let ok = |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        let Some(FieldValue::Json(opening)) = answer.fields.get(OPENING) else { return Err("the Recolor with Preset target is missing".into()) };
        let Some(active) = app.session.active().filter(|active| Some(active.uid) == opening["uid"].as_u64()) else {
            return Err(tl!("Open the original drawing to continue with Recolor with Preset.").to_owned());
        };
        if Some(active.reverts) != opening["reverts"].as_u64() {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").to_owned());
        }
        let mut p = serde_json::Map::new();
        if let Some(n) = opening["colors"].as_u64() {
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

    #[test]
    fn an_old_recolor_preset_does_not_open_on_restored_artwork() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let shape = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        let dir = vectorcraft_testkit::TempDir::new("recolor-preset-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        app.run("paint.setFill", json!({"ids": [shape], "color": "#ff0000"})).unwrap();
        app.run("select.set", json!({"ids": [shape]})).unwrap();
        app.run("ui.recolorDialog", json!({"preset": true, "colors": 1})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        let host = app.dialogs().unwrap();
        let id = dialog_id(KIND);
        assert!(!host.with(|h| h.confirm_by_id(&ctx, id)));
        assert_eq!(host.with(|h| h.held_state(id)), Some(HeldOk::Waiting));

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        app.run("select.set", json!({"ids": [shape]})).unwrap();
        super::super::host::pump(&mut app, &ctx);
        assert!(app.color_dialogs.recolor.is_none());
        assert!(matches!(host.with(|h| h.held_state(id)), Some(HeldOk::Refused(_))));

        host.with(|h| h.cancel_by_id(&ctx, id));
        app.run("ui.recolorDialog", json!({"preset": true, "colors": 1})).unwrap();
        assert!(super::super::tests::confirm(&mut app));
        assert!(app.color_dialogs.recolor.is_some());
    }
}
