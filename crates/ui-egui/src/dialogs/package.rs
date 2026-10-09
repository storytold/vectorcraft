//! File → Package…: where the package folder goes, its name, and what it collects (the linked
//! files, in a folder of their own or not, relinked or not; the fonts; the report). A document
//! never saved is saved first (Save As); unsaved changes are saved before packaging. The web
//! downloads the package as a zip.
//!
//! Fields: `folder` (the location), `name` (the package folder), `copyLinks`, `linksFolder`,
//! `relink`, `copyFonts`, `report`. OK runs `file.package` with them, then offers to show the
//! package folder (`file.showPackage`).

use serde_json::{Value, json};

use super::confirm::ask;
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "package";
const LABEL: f32 = 90.0;
/// The options: (field, label).
const OPTIONS: [(&str, &str); 5] = [
    ("copyLinks", "Copy Links"),
    ("linksFolder", "Collect Links in a Separate Folder"),
    ("relink", "Relink Linked Files to the Document"),
    ("copyFonts", "Copy Fonts Used in the Document"),
    ("report", "Create Report"),
];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Package").into(),
    body,
    confirm,
    ok: Some("Package"),
    min_width: 440.0,
    max_width: Some(460.0),
    ..DialogSpec::FORM
};

/// Is there no file system to write a folder to (the web downloads a zip)?
fn downloads(app: &VectorcraftApp) -> bool {
    app.services.download.is_some()
}

/// Open the dialog for the active document; one never saved is offered Save As first.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let Some(path) = st.path.clone() else {
        ask(
            app,
            tl!("Save the document first?"),
            tl!("Package collects a saved document with its linked files and fonts."),
            "file.saveAs",
            json!({}),
        );
        return Ok(Value::Null);
    };
    let p = std::path::Path::new(&path);
    let stem = p.file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    let folder = p.parent().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let mut fields = json!({ "folder": folder, "name": format!("{stem} Folder") });
    for (k, _) in OPTIONS {
        fields[k] = json!(true);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    if !downloads(app) {
        widgets::label_row(ui, tl!("Location:"), LABEL, |ui| {
            form::text(ui, d, "folder", 230.0);
            if app.services.pick_folder.is_some() && ui.button(tl!("Choose…")).clicked() {
                crate::picks::folder_field(app, d, "folder");
            }
        });
        ui.add_space(4.0);
    }
    widgets::label_row(ui, tl!("Folder Name:"), LABEL, |ui| {
        form::text(ui, d, "name", 230.0);
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Options"));
    for (k, label) in OPTIONS {
        // The link options apply only when the links are copied.
        let enabled = !matches!(k, "linksFolder" | "relink") || d.bool("copyLinks");
        let on = d.bool(k);
        if widgets::check(ui, label, on, enabled) {
            d.fields.insert(k.into(), json!(!on));
        }
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Fonts whose licence doesn't allow embedding are not copied.")).size(11.5).color(t.text_dim));
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = Value::Object(d.fields.clone());
    let web = downloads(app);
    if web {
        params["folder"] = Value::Null;
    } else if d.str("folder").trim().is_empty() {
        return Err("choose a location for the package".into());
    }
    // The package holds the document as saved.
    if app.session.active().is_some_and(|st| st.is_dirty()) {
        let r = io::save(app, vectorcraft_engine::cmd::fileio::SaveMode::Save, &json!({}), false)?;
        // The save asks first (it would replace a file that loses what opening it left out).
        if r.get("pending").is_some() {
            return Ok(r);
        }
    }
    let r = app.run("file.package", params)?;
    app.ui.dialog = None;
    let n = r["files"].as_array().map_or(0, Vec::len);
    let missing = r["missingLinks"].as_array().map_or(0, Vec::len);
    // The fonts left out: their license doesn't allow it, or they aren't on this computer.
    let fonts = r["skippedFonts"].as_array().into_iter().flatten().filter_map(|f| f["font"].as_str()).collect::<Vec<_>>().join(", ");
    let mut note = if missing > 0 { format!(" ({missing} linked file(s) not found)") } else { String::new() };
    if !fonts.is_empty() {
        note.push_str(&format!(" (fonts not copied: {fonts})"));
    }
    if web {
        let (name, data) = (r["name"].as_str().unwrap_or("package.zip"), r["dataBase64"].as_str().unwrap_or_default());
        let bytes = vectorcraft_format::base64_decode(data).ok_or("the package came back damaged")?;
        io::write_named(app, None, name, &bytes)?;
        app.status(format!("Packaged {n} file(s){note}"));
        return Ok(r);
    }
    let folder = r["folder"].as_str().unwrap_or_default().to_string();
    app.status(format!("Packaged {n} file(s) in {folder}{note}"));
    let mut shown_note = if missing > 0 {
        format!(" ({})", crate::i18n::fmt(tl!("{count} linked file(s) not found"), &[("count", &missing.to_string())]))
    } else {
        String::new()
    };
    if !fonts.is_empty() {
        shown_note.push_str(&format!(" ({})", crate::i18n::fmt(tl!("fonts not copied: {fonts}"), &[("fonts", &fonts)])));
    }
    let detail = crate::i18n::fmt(
        tl!("{count} file(s) in {folder}{note}. Show the package folder?"),
        &[("count", &n.to_string()), ("folder", &folder), ("note", &shown_note)],
    );
    ask(app, tl!("Package created"), &detail, "file.showPackage", json!({ "folder": folder }));
    Ok(r)
}

/// `file.showPackage {folder}`: show a package folder in the file manager.
pub fn show_package(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let folder = p.get("folder").and_then(Value::as_str).ok_or("missing `folder`")?.to_string();
    io::open_in_app(app, &folder)?;
    Ok(json!({ "folder": folder }))
}
