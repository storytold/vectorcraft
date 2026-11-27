//! The shared part of Save Swatch, Graphic Style, Symbol and Brush Library: a Name row, the caller's own
//! rows, Save To (the user library folder or a file) and Selected Only. OK runs the library's save command.
//!
//! Fields: `name`, `saveTo` (`user` or `file`), `selectedOnly`, plus `names` (the items selected in the
//! panel) and `hasNames` (there are some), which have no row.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// What differs between the four Save … Library dialogs.
pub(super) struct Library {
    /// The dialog's key.
    pub key: &'static str,
    /// The dialog's title.
    pub title: String,
    /// The save command OK runs.
    pub cmd: &'static str,
    /// The Selected Only row's label, before the count.
    pub only: String,
    /// The extension of the saved file, from the dialog's values.
    pub ext: fn(&DialogFields) -> String,
    /// Whether the library kind has a user library folder.
    pub has_user_dir: bool,
}

/// The active document's name without its extension: the library name the save commands use when
/// none is given.
fn document_name(app: &VectorcraftApp) -> Option<String> {
    let title = app.session.active()?.title();
    Some(title.rsplit_once('.').map_or(title.as_str(), |(s, _)| s).to_string())
}

/// Open the dialog for `library`: `extra` rows sit under the Name row. `names` are the items selected in the
/// panel.
pub(super) fn open(app: &mut VectorcraftApp, library: Library, names: Vec<String>, extra: Vec<Param>) -> Result<Value, String> {
    let state = app.session.active().ok_or("no document open")?;
    let document = (state.uid, state.reverts);
    let name = document_name(app).ok_or("no document open")?;
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let Library { key, title, cmd, only, ext, has_user_dir } = library;
    let count = names.len();
    let mut fields = DialogFields::new();
    fields.set("name", name);
    fields.set("hasNames", count > 0);
    fields.set("names", FieldValue::List(names.into_iter().map(FieldValue::Text).collect()));

    let options = |ids: [(&str, &'static str); 2]| ids.map(|(id, label)| (id.to_string(), label.into())).to_vec();
    let mut save_to = Param::new(
        "saveTo",
        tl!("Save To:"),
        ParamKind::radio(
            options([("user", tl!("User Defined Libraries")), ("file", tl!("A File…"))]),
            Some(if has_user_dir { "user" } else { "file" }.into()),
        ),
    );
    if !has_user_dir {
        save_to = save_to.disabled_options(["user"]);
    }
    let mut params = vec![Param::new("name", tl!("Name:"), ParamKind::text(""))];
    params.extend(extra);
    params.push(save_to);
    params.push(Param::new("selectedOnly", format!("{only} ({count})"), ParamKind::bool(false)).enabled_when("hasNames", true));

    let dialog =
        HostedParamDialog::new(&ctx, key, fields, move |fields| ParamDialog::new(key, title.clone(), params.clone()).values(Bind::bound(fields)));
    let owner = egui::Id::unique((key, "opening-drawing"));
    ctx.data_mut(|data| data.insert_temp(owner, document));
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| {
        let document = ctx.data(|data| data.get_temp::<(u64, u64)>(owner));
        let Some(state) = app.session.active().filter(|st| document.is_some_and(|(uid, _)| st.uid == uid)) else {
            return Err(tl!("Return to the original drawing to save its library").into());
        };
        if document.is_some_and(|(_, reverts)| state.reverts != reverts) {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").into());
        }
        save(app, cmd, ext, result)
    };
    super::host::open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

/// The save command's parameters from the answer: `name`, `user`, the dialog's `format` when it has one
/// and, with Selected Only, the selected `names`.
fn params(result: &ParamResult) -> Value {
    let fields = &result.fields;
    let mut p = json!({"name": fields.text("name").unwrap_or_default(), "user": fields.text("saveTo") == Some("user")});
    if let Some(format) = fields.text("format") {
        p["format"] = json!(format);
    }
    if fields.bool("selectedOnly") == Some(true) {
        let names = fields.list("names").map(|l| l.iter().filter_map(|v| if let FieldValue::Text(s) = v { Some(json!(s)) } else { None }).collect());
        p["names"] = Value::Array(names.unwrap_or_default());
    }
    p
}

/// Run save command `cmd` for the answer: into the user library folder, else to a file the user picks.
/// The save panel suggests the file name that a save into the user library folder uses: the name (the
/// document's when it is empty) with the extension `ext` gives. The web downloads the file under that name.
fn save(app: &mut VectorcraftApp, cmd: &str, ext: fn(&DialogFields) -> String, result: &ParamResult) -> Result<(), String> {
    let p = params(result);
    let name = result.fields.text("name").unwrap_or_default().trim().to_string();
    if p["user"] == json!(true) {
        app.run(cmd, p)?;
        app.status(format!("Saved to User Defined: {}", result.fields.text("name").unwrap_or_default()));
        return Ok(());
    }
    let name = Some(name).filter(|n| !n.is_empty()).or_else(|| document_name(app)).unwrap_or_default();
    let file = vectorcraft_engine::cmd::swatchlib::library_file_name(&name, &ext(&result.fields));
    // PLAN: assumed `save_command_output_named(app, cmd, name, params, then)` (fl.A21)
    match crate::io::save_command_output_named(app, cmd, &file, p, |_, path| Ok(json!({ "path": path }))) {
        // A cancelled pick leaves nothing open.
        Err(e) if e == "cancelled" => Ok(()),
        Err(e) => Err(e),
        Ok(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{save_symbol_library, tests_export};
    use serde_json::json;

    #[test]
    fn save_library_waits_for_the_drawing_that_opened_it() {
        let (mut app, written) = tests_export::app(1);
        app.run("symbol.new", json!({"name": "Source Mark"})).unwrap();
        save_symbol_library::open(&mut app, vec!["Source Mark".into()]).unwrap();
        tests_export::set(&mut app, "saveTo", json!("file"));
        tests_export::set(&mut app, "name", json!("Marks"));
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("shape.ellipse", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        app.run("symbol.new", json!({"name": "Other Mark"})).unwrap();
        let error = tests_export::try_confirm(&mut app).expect_err("Save Library must refuse the other drawing");
        assert!(error.contains("original drawing"), "{error}");
        assert!(written.borrow().is_empty());
        assert_eq!(tests_export::fields(&app)["name"], "Marks");
        app.run("document.activate", json!({"index": 0})).unwrap();
        tests_export::try_confirm(&mut app).unwrap();
        assert_eq!(written.borrow().len(), 1);
    }

    #[test]
    fn reopening_save_library_uses_the_new_drawings_draft() {
        let (mut app, written) = tests_export::app(1);
        app.run("symbol.new", json!({"name": "Source Mark"})).unwrap();
        save_symbol_library::open(&mut app, vec!["Source Mark".into()]).unwrap();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("shape.ellipse", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        app.run("symbol.new", json!({"name": "Other Mark"})).unwrap();
        save_symbol_library::open(&mut app, vec!["Other Mark".into()]).unwrap();
        tests_export::set(&mut app, "saveTo", json!("file"));
        tests_export::set(&mut app, "name", json!("Other Marks"));
        tests_export::try_confirm(&mut app).unwrap();
        let output = written.borrow();
        assert_eq!(output.len(), 1);
        assert!(output[0].0.contains("Other Marks"));
    }

    #[test]
    fn an_old_save_library_approval_cannot_export_definitions_restored_by_revert() {
        let (mut app, written) = tests_export::app(1);
        app.services.read = Some(Box::new(|path: &str| std::fs::read(path).map_err(|error| error.to_string())));
        let output = written.clone();
        app.services.write = Some(Box::new(move |path: &str, bytes: &[u8]| {
            if path.ends_with(".vectorcraft") {
                std::fs::write(path, bytes).map_err(|error| error.to_string())
            } else {
                output.borrow_mut().push((path.to_string(), bytes.to_vec()));
                Ok(())
            }
        }));
        let id = app.session.active().unwrap().selection.objects[0];
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        app.run("graphicStyle.new", json!({"name": "Mark", "id": id.0})).unwrap();
        let saved_fill = app.session.active().unwrap().doc.graphic_style("Mark").unwrap().appearance.fill_paint();
        let temp = vectorcraft_testkit::TempDir::new("save-library-revert");
        let file = temp.join("library-source.vectorcraft");
        app.run("file.saveAs", json!({"path": file})).unwrap();
        assert!(file.is_file());
        let state = app.session.active().unwrap();
        let (uid, reverts) = (state.uid, state.reverts);
        app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
        app.run("graphicStyle.redefine", json!({"name": "Mark", "id": id.0})).unwrap();
        assert_ne!(app.session.active().unwrap().doc.graphic_style("Mark").unwrap().appearance.fill_paint(), saved_fill);
        super::super::save_style_library::open(&mut app, vec!["Mark".into()]).unwrap();
        tests_export::set(&mut app, "saveTo", json!("file"));
        tests_export::set(&mut app, "selectedOnly", json!(true));
        tests_export::set(&mut app, "name", json!("Draft Marks"));

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let state = app.session.active().unwrap();
        assert_eq!((state.uid, state.reverts), (uid, reverts + 1));
        assert_eq!(state.doc.graphic_style("Mark").unwrap().appearance.fill_paint(), saved_fill);
        let undo = state.history.undo.len();
        assert!(tests_export::try_confirm(&mut app).is_err(), "old approval must not export the restored library");
        assert!(written.borrow().is_empty(), "no output is written for the retired draft");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert_eq!(tests_export::fields(&app)["name"], "Draft Marks", "refusal preserves the user's draft");

        tests_export::cancel(&mut app);
        super::super::save_style_library::open(&mut app, vec!["Mark".into()]).unwrap();
        tests_export::set(&mut app, "saveTo", json!("file"));
        tests_export::set(&mut app, "selectedOnly", json!(true));
        tests_export::set(&mut app, "name", json!("Restored Marks"));
        tests_export::try_confirm(&mut app).unwrap();
        let output = written.borrow();
        assert_eq!(output.len(), 1);
        let text = std::str::from_utf8(&output[0].1).unwrap();
        let library = vectorcraft_doc::style_libs::read(text, "Restored Marks").unwrap();
        assert_eq!(library.style("Mark").unwrap().appearance.fill_paint(), saved_fill, "fresh approval exports the restored definition");
    }
}
