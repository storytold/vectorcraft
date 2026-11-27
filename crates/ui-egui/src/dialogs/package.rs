//! File → Package…: where the package folder goes, its name, and what it collects (the linked
//! files, in a folder of their own or not, relinked or not; the fonts; the report). A document
//! never saved is saved first (Save As); unsaved changes are saved before packaging, and when that
//! save fails no package is written. The web downloads the package as a zip.
//!
//! Fields: `folder` (the location), `name` (the package folder), `copyLinks`, `linksFolder`,
//! `relink`, `copyFonts`, `report`. OK runs `file.package` with them, then offers to show the
//! package folder (`file.showPackage`). On the desktop with Background Export on, the package is
//! collected and written on the background thread ([`crate::background`]) after the dialog closes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft::host::PickRequest;
use craft::ui::contract::Bind;
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::PathKind;
use serde_json::{Value, json};
use vectorcraft_engine::EngineError;

use super::confirm::{ask, ask_in_document};
use crate::background::{self, Writer};
use crate::{VectorcraftApp, file_asks, io};

/// The key of the Package dialog on the host.
const KEY: &str = "package";
/// What the host remembers the folder of a Location pick under.
const PURPOSE: &str = "package";
/// The options: (field, label).
const OPTIONS: [(&str, &str); 5] = [
    ("copyLinks", "Copy Links"),
    ("linksFolder", "Collect Links in a Separate Folder"),
    ("relink", "Relink Linked Files to the Document"),
    ("copyFonts", "Copy Fonts Used in the Document"),
    ("report", "Create Report"),
];

/// Is there no file system to write a folder to (the web downloads a zip)?
fn downloads(app: &VectorcraftApp) -> bool {
    app.services.download.is_some()
}

/// Open the dialog for the active document; one never saved is offered Save As first.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let Some(path) = st.path.clone() else {
        ask_in_document(
            app,
            tl!("Save the document first?"),
            tl!("Package collects a saved document with its linked files and fonts."),
            "file.saveAs",
            json!({}),
        );
        return Ok(Value::Null);
    };
    let document = st.uid;
    let p = std::path::Path::new(&path);
    let stem = p.file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    let folder = p.parent().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let mut fields = DialogFields::new();
    fields.set("folder", folder);
    fields.set("name", format!("{stem} Folder"));
    for (k, _) in OPTIONS {
        fields.set(k, true);
    }
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let params = params(downloads(app));
    let dialog = HostedParamDialog::new(&ctx, KEY, fields, move |fields| {
        ParamDialog::new(KEY, tl!("Package"), params.clone())
            .values(Bind::bound(fields))
            .ok_label(Label::from(tl!("Package")))
            .footer_note(Label::from(tl!("Fonts whose licence doesn't allow embedding are not copied.")))
    });
    crate::dialogs::host::open_held(app, dialog, |app, answer| confirm(app, &answer.fields).map(|_| ())).ok_or("no window")?;
    app.package_document = Some(document);
    Ok(Value::Null)
}

/// The rows of the dialog: Location (the web downloads a zip, so it has none), Folder Name and the options.
fn params(web: bool) -> Vec<Param> {
    let mut rows = Vec::new();
    if !web {
        rows.push(Param::new("folder", tl!("Location"), ParamKind::text("")).browse(PathKind::Folder));
    }
    rows.push(Param::new("name", tl!("Folder Name"), ParamKind::text("")));
    rows.push(Param::section(tl!("Options")));
    for (key, label) in OPTIONS {
        let row = Param::new(key, tl!(label), ParamKind::bool(true));
        // The link options apply only when the links are copied.
        rows.push(if matches!(key, "linksFolder" | "relink") { row.enabled_when("copyLinks", true) } else { row });
    }
    rows
}

/// Answers the Location row's Browse: a folder pick through the file asks, whose answer goes back into the dialog.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KEY);
    let Some(request) = dialogs.with(|host| host.get_mut_by_id::<HostedParamDialog>(id).map(HostedParamDialog::pick_path)) else {
        app.package_document = None;
        return;
    };
    if let Some(request) = request {
        file_asks::browse(app, id, request, PickRequest::folder(PURPOSE), file_asks::answer_path);
    }
}

fn require_package_document(app: &VectorcraftApp, document: u64) -> Result<(), String> {
    if app.session.active().is_none_or(|st| st.uid != document) {
        return Err(tl!("Package can only save and collect the document where it was opened.").to_owned());
    }
    Ok(())
}

/// What OK does: save a dirty document, then write the package (on the worker with Background Export on) or
/// download it. `Err` refuses the OK and keeps the dialog open; the reply says whether the package is in the background.
pub(super) fn confirm(app: &mut VectorcraftApp, fields: &DialogFields) -> Result<Value, String> {
    let document = app.package_document.ok_or("no Package dialog open")?;
    require_package_document(app, document)?;
    let text = |key: &str| fields.text(key).unwrap_or_default();
    let mut params = json!({ "folder": text("folder"), "name": text("name") });
    for (key, _) in OPTIONS {
        params[key] = json!(fields.bool(key).unwrap_or(true));
    }
    let web = downloads(app);
    if web {
        params["folder"] = Value::Null;
    } else if text("folder").trim().is_empty() {
        return Err("choose a location for the package".into());
    }
    // The package holds the document as saved: when the save fails, or asks a question and writes
    // nothing yet, no package is written.
    let mut saving = None;
    if app.session.active().is_some_and(|st| st.is_dirty()) {
        let r = io::save(app, vectorcraft_engine::cmd::fileio::SaveMode::Save, &json!({}), false)?;
        // The save asks first (it would replace a file that loses what opening it left out).
        if r.get("pending").is_some() {
            return Ok(r);
        }
        // Background Save: the save is written on the worker, and its result decides the package.
        if r["background"] == true {
            saving = app.background.jobs.last().map(background::Job::id);
        }
    }
    // Background Export (Preferences › File Handling): the linked files and fonts are collected
    // and the package is written from the document as it is now, while editing goes on. The jobs
    // started before it (an earlier package to the same folder) are written first: the worker
    // runs its jobs in order, and a package made at once waits for them.
    let enabled = app.session.prefs.background_export;
    if !enabled {
        let outcome: Rc<RefCell<Option<Result<Value, String>>>> = Rc::default();
        if let Some(id) = saving {
            let applied = outcome.clone();
            background::after(app, id, move |_, r| *applied.borrow_mut() = Some(r.clone()));
        }
        background::wait_all(app);
        if let Some(r) = outcome.take().filter(|r| !saved(r)) {
            return r;
        }
    }
    // A completed background job may have changed the active document while Package waited.
    require_package_document(app, document)?;
    let package = app.session.prepare_package(&params).map_err(|e| e.to_string()).inspect_err(|e| app.ui.status = e.clone())?;
    let label = format!("Packaging {}", text("name"));
    let later = Rc::new(Cell::new(false));
    let in_background = later.clone();
    let work = move |write: Writer| {
        // A writer error that already names the file (an automation root's refusal) is kept as it is.
        let mut write =
            |path: &str, bytes: &[u8]| write(path, bytes).map_err(|e| EngineError::Other(if e.contains(path) { e } else { format!("{path}: {e}") }));
        package.write(&mut write).map_err(|e| e.to_string())
    };
    let start =
        move |app: &mut VectorcraftApp| background::run(app, enabled, label, None, work, move |app, r| packaged(app, r, web, in_background.get()));
    // Both in the background: the package, collected from the document as it is now, starts once
    // the save is written, with the automation roots in force at OK.
    if let Some(id) = saving.filter(|_| enabled) {
        later.set(true);
        let scope = vectorcraft_engine::file_access::ScopeHandle::capture();
        background::after(app, id, move |app, r| {
            if saved(r)
                && let Err(e) = scope.run(|| start(app))
            {
                app.status(e);
            }
        });
        return Ok(json!({ "background": true }));
    }
    let r = start(app)?;
    if r["background"] == true {
        later.set(true);
    }
    Ok(r)
}

/// Did a save whose applied result is `r` write the document (no error, no question asked before
/// writing, and not canceled by Cancel Save)?
fn saved(r: &Result<Value, String>) -> bool {
    r.as_ref().is_ok_and(|r| r.get("pending").is_none() && r.get("cancelled").and_then(Value::as_bool) != Some(true))
}

/// Report the package `r` (`file.package`'s reply): the status bar, the web's download, and the
/// offer to show the package folder. `later`: it was written in the background, after the dialog
/// closed, and a dialog open now is another one, which stays (the status bar names the folder).
fn packaged(app: &mut VectorcraftApp, r: Result<Value, String>, web: bool, later: bool) -> Result<Value, String> {
    let r = r.inspect_err(|e| app.ui.status = e.clone())?;
    let n = r["files"].as_array().map_or(0, Vec::len);
    let missing = r["missingLinks"].as_array().map_or(0, Vec::len);
    // The fonts left out: their license doesn't allow it, or they aren't on this computer.
    let fonts = r["skippedFonts"].as_array().into_iter().flatten().filter_map(|f| f["font"].as_str()).collect::<Vec<_>>().join(", ");
    let n = n as u64;
    if web {
        let name = r["name"].as_str().unwrap_or("package.zip").to_string();
        let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap_or_default()).ok_or("the package came back damaged")?;
        // The web shows no dialog, so the status line names every font left out. The download runs the continuation at once.
        let status = package_status(n, None, &package_status_note(missing, &fonts));
        return io::write_named(app, None, &name, bytes, move |app, _path| {
            app.status(status);
            Ok(r)
        });
    }
    let folder = r["folder"].as_str().unwrap_or_default().to_string();
    // A package written at once asks, over the dialog that is still closing. A dialog open now while one written
    // in the background ends (another one) stays, and no dialog names the fonts left out, so the status line
    // names every one of them. Otherwise the dialog names them all, and the status line may cut the list short.
    let asks = !later || !app.any_dialog_open();
    let listed = if asks { status_fonts(&fonts) } else { fonts.clone() };
    app.status(package_status(n, Some(&folder), &package_status_note(missing, &listed)));
    let shown_note = package_dialog_note(missing, &fonts);
    let detail = if shown_note.is_empty() {
        crate::i18n::fmt(
            &crate::i18n::tn(n, "{n} file in {folder}. Show the package folder?", "{n} files in {folder}. Show the package folder?"),
            &[("folder", &folder)],
        )
    } else {
        crate::i18n::fmt(
            &crate::i18n::tn(n, "{n} file in {folder} {note}. Show the package folder?", "{n} files in {folder} {note}. Show the package folder?"),
            &[("folder", &folder), ("note", &shown_note)],
        )
    };
    if asks {
        ask(app, tl!("Package created"), &detail, "file.showPackage", json!({ "folder": folder }));
    }
    Ok(r)
}

/// The status bar's line for a package of `n` files written to `folder` (`None`: the web's
/// download), with `note` from [`package_status_note`] when it isn't empty. With a note and a
/// folder, the note comes before the folder and the folder ends the line. The line is English
/// (the status bar translates it when it draws it): each catalog has `@msgplural` rows for it, and
/// the folder takes the rest of the line after `) in `, so a folder name with spaces or parentheses
/// stays whole.
pub(super) fn package_status(n: u64, folder: Option<&str>, note: &str) -> String {
    let (one, other) = match (folder.is_some(), note.is_empty()) {
        (false, true) => ("Packaged {n} file", "Packaged {n} files"),
        (false, false) => ("Packaged {n} file {note}", "Packaged {n} files {note}"),
        (true, true) => ("Packaged {n} file in {folder}", "Packaged {n} files in {folder}"),
        (true, false) => ("Packaged {n} file {note} in {folder}", "Packaged {n} files {note} in {folder}"),
    };
    crate::i18n::msg_n(n, one, other, &[("folder", folder.unwrap_or_default()), ("note", note)])
}

/// Parenthetical notes in a package status (`(2 linked files not found)` …), in English,
/// joined with spaces and without a leading space (#3494). The catalogs' `@msg` and `@msgplural`
/// rows for each note, and for one or two notes in parentheses, translate them where the status
/// bar draws the line.
pub(super) fn package_status_note(missing: usize, fonts: &str) -> String {
    let missing = (missing > 0).then(|| crate::i18n::msg_n(missing as u64, "{n} linked file not found", "{n} linked files not found", &[]));
    join_notes(missing, (!fonts.is_empty()).then(|| format!("fonts not copied: {fonts}")))
}

/// The most bytes of font names a package status lists next to the dialog that names them all.
pub(super) const STATUS_FONTS: usize = 500;

/// `fonts` (names joined with `, `) as the desktop's status line lists them while the Package
/// created dialog names them all: whole, or the names that fit in [`STATUS_FONTS`] bytes followed
/// by `, …`, so the line stays short enough for the status bar to translate it.
pub(super) fn status_fonts(fonts: &str) -> String {
    if fonts.len() <= STATUS_FONTS {
        return fonts.to_string();
    }
    let mut kept = String::new();
    for name in fonts.split(", ") {
        if kept.len() + name.len() + 2 > STATUS_FONTS {
            break;
        }
        if !kept.is_empty() {
            kept.push_str(", ");
        }
        kept.push_str(name);
    }
    if kept.is_empty() { "…".to_string() } else { format!("{kept}, …") }
}

/// [`package_status_note`] in the interface language, for the dialog shown when the package is made.
pub(super) fn package_dialog_note(missing: usize, fonts: &str) -> String {
    let missing = (missing > 0).then(|| crate::i18n::tn(missing as u64, "{n} linked file not found", "{n} linked files not found"));
    join_notes(missing, (!fonts.is_empty()).then(|| crate::i18n::fmt(tl!("fonts not copied: {fonts}"), &[("fonts", fonts)])))
}

/// Each note in parentheses, joined with spaces.
fn join_notes(missing: Option<String>, fonts: Option<String>) -> String {
    [missing, fonts].into_iter().flatten().map(|note| format!("({note})")).collect::<Vec<_>>().join(" ")
}

/// `file.showPackage {folder}`: show a package folder in the file manager.
pub fn show_package(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    app.refuse_automation("file.showPackage")?;
    let folder = p.get("folder").and_then(Value::as_str).ok_or("missing `folder`")?.to_string();
    // A package is a folder: an executable, shortcut or app bundle would run, not be shown.
    if !std::path::Path::new(&folder).is_dir() || folder.to_ascii_lowercase().ends_with(".app") {
        return Err(format!("{folder} is not a package folder"));
    }
    io::open_in_app(app, &folder)?;
    Ok(json!({ "folder": folder }))
}
