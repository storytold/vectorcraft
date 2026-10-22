//! Find Linked Files: the files of a document's missing linked images and placed documents (from
//! the Links panel, of the selected ones whatever their status, when any are selected), looked for
//! in a folder the user picks and its subfolders (`links.findFiles`, on separate threads). Each
//! file it looks for lists the files found with its name, best first, with Don't Relink: a file is
//! chosen at the start only when it is the only file found, or the only one that is the same file
//! (the size and hash saved with the link). Relink relinks each choice (`links.relink`, one undo
//! step each); with Copy into the document's folder (a saved document), an image's file is copied
//! there first, never replacing a file, and a placed document is relinked where its file is. Opened
//! from Find in Folder… in Linked File Not Found (dialog `missingLinks`), the files left without a
//! choice and those that fail to relink go back to that dialog; opened from the Links panel's Find
//! in Folder… (`ui.findLinksInFolder`), they stay as they are.
//!
//! Fields: `document` (the document's uid), `missing` (`[{name, path, ids}]`), `modified` (the ids
//! of the images whose file was modified, offered for update afterwards), `back` (the files left
//! without a choice and those that fail to relink go back to dialog `missingLinks`), `state`
//! (empty, `searching`, `done`, `stopped` or `failed`), `search` (the search's `id`), `folder`,
//! `found` (`links.findFiles`'s `links`, with the files found), `searched`, `skipped`,
//! `unreadable`, `stopped`, `error`, `picks` (the file chosen for each row of `missing`, or null),
//! `touched` (the rows whose file was chosen in the dialog: new search results leave their pick as
//! it is), `copy` and `searchFolder` (set it and confirm to search that folder).

use std::time::Duration;

use serde_json::{Value, json};

use super::DialogSpec;
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Find Linked Files.
pub const KIND: &str = "findLinks";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Find Linked Files").into(),
    body,
    confirm,
    ok: Some("Relink"),
    min_width: 440.0,
    max_width: Some(560.0),
    inputs: |_| &["searchFolder"],
    ..DialogSpec::FORM
};

/// The rows of field `key` of `d` (`missing`, `modified`, `picks`…).
fn rows(d: &Dialog, key: &str) -> Vec<Value> {
    d.fields.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The ids in `rows` (`[{ids}]`).
fn ids(rows: &[Value]) -> Vec<Value> {
    rows.iter().flat_map(|r| r["ids"].as_array().cloned().unwrap_or_default()).collect()
}

/// A Find Linked Files dialog for the `missing` files of the active document.
fn dialog(app: &VectorcraftApp, missing: Vec<Value>, modified: Vec<Value>, back: bool) -> Result<Dialog, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    let picks = vec![Value::Null; missing.len()];
    let fields =
        json!({ "document": uid, "missing": missing, "modified": modified, "back": back, "state": "", "picks": picks, "touched": [], "copy": false });
    Ok(Dialog::new(KIND, fields))
}

/// Linked File Not Found's Find in Folder… (`d`, the `missingLinks` dialog): `d` becomes Find
/// Linked Files, searching `folder` for every file `d` still asks about.
pub(super) fn from_missing_links(app: &mut VectorcraftApp, d: &mut Dialog, folder: &str) -> Result<(), String> {
    let mut find = dialog(app, rows(d, "missing"), rows(d, "modified"), true)?;
    start_search(app, &mut find, folder)?;
    *d = find;
    Ok(())
}

/// The selected linked images and placed documents of the active document.
fn selected_linked(app: &VectorcraftApp) -> Vec<Value> {
    let Some(st) = app.session.active() else { return vec![] };
    let linked = |id: &vectorcraft_doc::NodeId| {
        st.doc.node(*id).is_some_and(|n| match &n.kind {
            vectorcraft_doc::NodeKind::Image(im) => im.link.is_some(),
            vectorcraft_doc::NodeKind::PlacedDocument(_) => true,
            _ => false,
        })
    };
    st.selection.objects.iter().filter(|id| linked(id)).map(|id| json!(id.0)).collect()
}

/// `ui.findLinksInFolder` (the Links panel's Find in Folder…): ask for a folder (or take `folder`),
/// then open Find Linked Files searching it for the files of linked objects `ids` (default: the
/// selected linked images and placed documents, else the files that are missing).
pub(crate) fn find_in_folder_command(app: &mut VectorcraftApp, folder: Option<String>, ids: Option<Vec<Value>>) -> Result<Value, String> {
    let ids = ids.filter(|ids| !ids.is_empty()).or_else(|| Some(selected_linked(app)).filter(|ids| !ids.is_empty()));
    let all = ids.is_some();
    let r = app.session.execute("links.check", &json!({ "ids": ids })).map_err(|e| e.to_string())?;
    let missing: Vec<Value> = r["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| all || l["status"] == "missing")
        .map(|l| json!({ "name": l["name"], "path": l["path"], "ids": l["ids"] }))
        .collect();
    // Nothing to look for: no folder is asked for.
    if missing.is_empty() {
        return Err("No linked files to look for".into());
    }
    let folder = match folder {
        Some(f) => f,
        None => crate::picks::folder(app).ok_or("cancelled")?,
    };
    let mut d = dialog(app, missing, vec![], false)?;
    start_search(app, &mut d, &folder)?;
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

/// Whether object `id` of the active document is an image (and not a placed document).
fn is_image(app: &VectorcraftApp, id: &Value) -> bool {
    let Some(id) = id.as_u64() else { return false };
    let st = app.session.active();
    st.and_then(|st| st.doc.node(vectorcraft_doc::NodeId(id))).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)))
}

/// Make the dialog's document the active one: it may be another tab's.
fn activate(app: &mut VectorcraftApp, d: &Dialog) -> Result<(), String> {
    let uid = d.fields.get("document").and_then(Value::as_u64);
    if app.session.active().map(|st| st.uid) == uid {
        return Ok(());
    }
    let index = app.session.documents().iter().position(|doc| Some(doc.uid) == uid).ok_or("that document is no longer open")?;
    app.run("document.activate", json!({ "index": index })).map(|_| ())
}

/// Start looking in `folder` for the files of the dialog's rows; what an earlier search found, and
/// the files chosen from it, are dropped.
fn start_search(app: &mut VectorcraftApp, d: &mut Dialog, folder: &str) -> Result<(), String> {
    activate(app, d)?;
    let r = app.run("links.findFiles", json!({ "folder": folder, "ids": ids(&rows(d, "missing")) }))?;
    for k in ["found", "searched", "skipped", "unreadable", "stopped", "error"] {
        d.fields.remove(k);
    }
    d.fields.insert("picks".into(), json!(vec![Value::Null; rows(d, "missing").len()]));
    d.fields.insert("touched".into(), json!([]));
    d.fields.insert("search".into(), r["id"].clone());
    d.fields.insert("folder".into(), json!(folder));
    update(d, &r);
    app.ui.links_search = r["id"].as_u64();
    Ok(())
}

/// The files a search found for `row` (`{path, ids}`), best first: the answer's `links` row with
/// the same path and ids.
fn files_for<'a>(found: &'a [Value], row: &Value) -> &'a [Value] {
    found.iter().find(|r| r["path"] == row["path"] && r["ids"] == row["ids"]).and_then(|r| r["files"].as_array()).map_or(&[], Vec::as_slice)
}

/// The file chosen at the start among `files`: the only one that is the same file, else the only
/// file found.
fn first_pick(files: &[Value]) -> Option<&Value> {
    let mut same = files.iter().filter(|f| f["match"] == "same");
    match (same.next(), same.next(), files) {
        (Some(f), None, _) | (None, _, [f]) => Some(&f["path"]),
        _ => None,
    }
}

/// A search's answer `r` into the dialog. A row whose file wasn't chosen in the dialog takes the
/// file chosen at the start ([`first_pick`]).
fn update(d: &mut Dialog, r: &Value) {
    let missing = rows(d, "missing");
    let touched = rows(d, "touched");
    let mut picks = rows(d, "picks");
    picks.resize(missing.len(), Value::Null);
    let found = r["links"].as_array().map_or(&[][..], Vec::as_slice);
    for (i, (row, pick)) in missing.iter().zip(picks.iter_mut()).enumerate() {
        if !touched.contains(&json!(i)) {
            *pick = first_pick(files_for(found, row)).cloned().unwrap_or(Value::Null);
        }
    }
    d.fields.insert("picks".into(), json!(picks));
    d.fields.insert("found".into(), r["links"].clone());
    for k in ["state", "searched", "skipped", "unreadable", "stopped", "error"] {
        match r.get(k) {
            Some(v) => d.fields.insert(k.into(), v.clone()),
            None => d.fields.remove(k),
        };
    }
}

/// While searching: the search's progress into the dialog. A search started elsewhere since
/// stopped this one.
fn follow(app: &mut VectorcraftApp, d: &mut Dialog) {
    // Polled through `session.execute`.
    let Ok(r) = app.session.execute("links.findFiles", &json!({})) else { return };
    if d.fields.get("search") != Some(&r["id"]) {
        d.fields.insert("state".into(), json!("stopped"));
        return;
    }
    update(d, &r);
    if let Some(e) = r["error"].as_str() {
        app.status(e);
    }
}

/// Stop the search the dialog started once the dialog is gone (closed, or another in its place).
pub(super) fn stop_when_closed(app: &mut VectorcraftApp) {
    let Some(id) = app.ui.links_search else { return };
    let open = app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND && d.fields.get("search").and_then(Value::as_u64) == Some(id));
    if !open {
        app.ui.links_search = None;
        vectorcraft_engine::cmd::linkfiles::stop(&mut app.session, id);
    }
}

/// The folder of the dialog's document, when it was saved.
fn document_folder(app: &VectorcraftApp, d: &Dialog) -> Option<String> {
    let uid = d.fields.get("document").and_then(Value::as_u64)?;
    let path = app.session.documents().iter().find(|doc| doc.uid == uid)?.path.clone()?;
    std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned())
}

/// A button of the dialog's.
enum Act {
    Pick,
    Stop,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    if d.str("state") == "searching" {
        follow(app, d);
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let state = d.str("state");
    let searching = state == "searching";
    widgets::dim_label(ui, tl!("Choose the file to relink each linked file to. Files with the size and contents saved with the link come first."));
    ui.add_space(8.0);
    let missing = rows(d, "missing");
    let found = rows(d, "found");
    let mut picks = rows(d, "picks");
    picks.resize(missing.len(), Value::Null);
    let mut touched = rows(d, "touched");
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, (row, pick)) in missing.iter().zip(picks.iter_mut()).enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.add_space(3.0);
                        ui.label(egui::RichText::new(row["name"].as_str().unwrap_or_default()).font(theme::semibold(12.0)).color(t.text_strong));
                        let path = row["path"].as_str().unwrap_or_default();
                        ui.add(egui::Label::new(egui::RichText::new(path).size(11.0).color(t.text_dim)).truncate()).on_hover_text(path);
                        let files = files_for(&found, row);
                        let mut chose = None;
                        for f in files {
                            let file = f["path"].as_str().unwrap_or_default();
                            let p = std::path::Path::new(file);
                            ui.horizontal(|ui| {
                                let name = p.file_name().map_or_else(|| file.to_string(), |n| n.to_string_lossy().into_owned());
                                if widgets::radio(ui, &name, *pick == f["path"], true) {
                                    chose = Some(f["path"].clone());
                                }
                                let tag = match f["match"].as_str() {
                                    Some("same") => tl!("Same file"),
                                    Some("size") => tl!("Same size"),
                                    _ => "",
                                };
                                if !tag.is_empty() {
                                    ui.label(egui::RichText::new(tag).size(11.0).color(t.accent));
                                }
                            });
                            let folder = p.parent().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
                            ui.add(egui::Label::new(egui::RichText::new(folder).size(11.0).color(t.text_dim)).truncate()).on_hover_text(file);
                        }
                        if !files.is_empty() && widgets::radio(ui, tl!("Don't Relink"), pick.is_null(), true) {
                            chose = Some(Value::Null);
                        }
                        if let Some(c) = chose {
                            *pick = c;
                            if !touched.contains(&json!(i)) {
                                touched.push(json!(i));
                            }
                        }
                        // A failed search (a folder it refuses) didn't look: `progress` says why.
                        if files.is_empty() && !state.is_empty() && state != "failed" {
                            widgets::dim_label(ui, if searching { tl!("Searching…") } else { tl!("Not found") });
                        }
                        ui.add_space(3.0);
                    });
                });
            }
        });
    });
    d.fields.insert("picks".into(), json!(picks));
    d.fields.insert("touched".into(), json!(touched));
    ui.add_space(10.0);
    let mut act = None;
    ui.horizontal(|ui| {
        if crate::picks::can(app, &PickRequest::Folder) && ui.add_enabled(!searching, egui::Button::new(tl!("Find in Folder…"))).clicked() {
            act = Some(Act::Pick);
        }
        if searching {
            if ui.button(tl!("Stop")).clicked() {
                act = Some(Act::Stop);
            }
            ui.add(egui::Spinner::new().size(12.0).color(t.accent));
        }
    });
    super::missing_fonts::progress(ui, d);
    if let Some(folder) = document_folder(app, d) {
        ui.add_space(8.0);
        let copy = d.bool("copy");
        if widgets::check_tip(ui, tl!("Copy into the document's folder"), copy, true, &folder) {
            d.fields.insert("copy".into(), json!(!copy));
        }
    }
    let r = match act {
        Some(Act::Pick) => crate::picks::in_dialog(app, d, |app, d| match crate::picks::folder(app) {
            Some(folder) => start_search(app, d, &folder),
            None => Ok(()),
        }),
        Some(Act::Stop) => app.run("links.findFiles", json!({ "stop": true })).map(|_| follow(app, d)),
        None => Ok(()),
    };
    if let Err(e) = r {
        app.status(e);
    }
    false
}

/// With `searchFolder` (agents): search that folder, and stay open. Otherwise Relink: each row's
/// file, one `links.relink` each (copied into the document's folder with `copy`). The status line
/// names the first file that fails to relink, and the other files still relink. From Linked File
/// Not Found, the rows left without a file and the rows that failed to relink go back to it, then
/// the modified files are offered for update.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let folder = d.str("searchFolder");
    if !folder.is_empty() {
        let mut d = d.clone();
        d.fields.remove("searchFolder");
        // The folder is searched once: a search that doesn't start leaves the dialog without it.
        if let Some(open) = app.ui.dialog.as_mut().filter(|open| open.kind == KIND) {
            open.fields.remove("searchFolder");
        }
        start_search(app, &mut d, &folder)?;
        let state = d.fields.get("state").cloned().unwrap_or(Value::Null);
        app.ui.dialog = Some(d);
        return Ok(json!({ "state": state }));
    }
    activate(app, d)?;
    let picks = rows(d, "picks");
    let (mut relinked, mut failed, mut rest) = (vec![], vec![], vec![]);
    for (i, row) in rows(d, "missing").into_iter().enumerate() {
        let Some(path) = picks.get(i).and_then(Value::as_str) else {
            rest.push(row);
            continue;
        };
        let mut params = json!({ "ids": row["ids"], "path": path });
        // A placed document is relinked where its file is.
        if d.bool("copy") && row["ids"].as_array().is_some_and(|ids| ids.iter().all(|id| is_image(app, id))) {
            params["copy"] = json!(true);
        }
        match app.run("links.relink", params) {
            Ok(r) => relinked.extend(r["relinked"].as_array().cloned().unwrap_or_default()),
            Err(e) => {
                failed.push(json!({ "name": row["name"], "error": e }));
                rest.push(row);
            }
        }
    }
    app.ui.dialog = None;
    let n = relinked.len();
    match failed.first() {
        Some(f) => {
            let (name, reason) = (f["name"].as_str().unwrap_or_default(), f["error"].as_str().unwrap_or_default());
            app.status(format!("Relinked {n} image(s); {name} couldn't be relinked: {reason}"));
        }
        None if n > 0 => app.status(format!("Relinked {n} image(s)")),
        None => {}
    }
    if d.bool("back") {
        super::missing_links::ask_next(app, rest, rows(d, "modified"));
    }
    Ok(json!({ "relinked": relinked, "failed": failed }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use vectorcraft_engine::Session;
    use vectorcraft_engine::cmd::findfiles::Rules;

    use super::*;

    fn frame(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    /// A fresh temporary folder (canonical, as a search reports it).
    fn folder(tag: &str) -> PathBuf {
        let dir = vectorcraft_testkit::temp_dir(&format!("find-links-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        vectorcraft_engine::cmd::findfiles::plain(std::fs::canonicalize(&dir).unwrap())
    }

    /// An app that reads files, shows a folder picker and whose searches may go anywhere in `root`,
    /// on two threads.
    fn app_for(root: &Path) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.services.pick_folder = Some(Box::new(|| None));
        app.session.search_rules = Some(Rules { user_content: vec![root.to_path_buf()], ..Rules::default() });
        app.session.search_threads = Some(2);
        app
    }

    /// `rel` (`/`-separated) inside `root`, with the platform's separators, as the app reports paths.
    fn at(root: &Path, rel: &str) -> PathBuf {
        rel.split('/').fold(root.to_path_buf(), |p, c| p.join(c))
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// A PNG of one color, a different one for each `n`.
    fn png(n: u8) -> Vec<u8> {
        let mut out = vec![];
        image::RgbaImage::from_pixel(40, 30, image::Rgba([200, n.wrapping_mul(40), 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A document linking to files `names` in `root/orig`, saved as `root/doc/doc.vectorcraft`; the
    /// files are then deleted and the document opened again → each file's bytes.
    fn opened_without(app: &mut VectorcraftApp, root: &Path, names: &[&str]) -> Vec<Vec<u8>> {
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let mut files = vec![];
        for (n, name) in (0..).zip(names) {
            let path = root.join("orig").join(name);
            write(&path, &png(n));
            app.run("file.place", json!({"path": path})).unwrap();
            files.push(png(n));
        }
        let doc = at(root, "doc/doc.vectorcraft");
        std::fs::create_dir_all(doc.parent().unwrap()).unwrap();
        app.run("document.save", json!({"path": doc})).unwrap();
        std::fs::remove_dir_all(root.join("orig")).unwrap();
        crate::io::open_path(app, &doc.to_string_lossy()).unwrap();
        files
    }

    /// Draw the dialog until its search ends (10 seconds at most).
    fn searched(app: &mut VectorcraftApp) {
        let until = Instant::now() + Duration::from_secs(10);
        while app.ui.dialog.as_ref().is_some_and(|d| d.str("state") == "searching") && Instant::now() < until {
            frame(app);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn kind(app: &VectorcraftApp) -> Option<&str> {
        app.ui.dialog.as_ref().map(|d| d.kind.as_str())
    }

    /// The pick of the row named `name`.
    fn pick<'a>(d: &'a Dialog, name: &str) -> &'a Value {
        let i = d.fields["missing"].as_array().unwrap().iter().position(|r| r["name"] == name).unwrap();
        &d.fields["picks"][i]
    }

    /// Find in Folder… in Linked File Not Found: each missing file's same file is chosen, and Relink
    /// relinks them.
    #[test]
    fn find_in_folder_relinks_the_missing_files_to_the_files_found() {
        let root = folder("relink");
        let mut app = app_for(&root);
        let bytes = opened_without(&mut app, &root, &["a.png", "b.png"]);
        assert_eq!(kind(&app), Some(super::super::missing_links::KIND));
        assert!(frame(&mut app).contains("Find in Folder…"));
        let (a, b) = (at(&root, "found/sub/a.png"), at(&root, "found/b.png"));
        write(&a, &bytes[0]);
        write(&b, &bytes[1]);
        write(&at(&root, "found/other/b.png"), b"another file");
        // What Find in Folder… does with the folder picked.
        app.ui.dialog.as_mut().unwrap().fields.insert("searchFolder".into(), json!(root.join("found")));
        let r = super::super::confirm(&mut app).unwrap();
        assert!(matches!(r["state"].as_str(), Some("searching" | "done")), "{r}");
        assert_eq!(kind(&app), Some(KIND));
        searched(&mut app);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("state"), pick(d, "a.png"), pick(d, "b.png")), ("done".to_string(), &json!(a), &json!(b)));
        let text = frame(&mut app);
        for label in
            ["Find Linked Files", "a.png", "b.png", "Same file", "Don't Relink", "Find in Folder…", "Copy into the document's folder", "Relink"]
        {
            assert!(text.contains(label), "{label} in {text}");
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.ui.status, "Relinked 2 image(s)");
        assert_eq!(app.run("links.check", json!({})).unwrap()["missing"], 0);
    }

    /// The files left without a file, or whose file fails to relink, go back to Linked File Not
    /// Found; the failure is named in the status line, and the other files still relink.
    #[test]
    fn files_not_relinked_go_back_to_linked_file_not_found() {
        let root = folder("back");
        let mut app = app_for(&root);
        let bytes = opened_without(&mut app, &root, &["a.png", "b.png", "c.png"]);
        write(&at(&root, "found/a.png"), &bytes[0]);
        write(&at(&root, "found/b.png"), b"not an image");
        app.ui.dialog.as_mut().unwrap().fields.insert("searchFolder".into(), json!(root.join("found")));
        super::super::confirm(&mut app).unwrap();
        searched(&mut app);
        let d = app.ui.dialog.as_ref().unwrap();
        // b.png's only file is chosen, c.png has none.
        assert_eq!((pick(d, "b.png"), pick(d, "c.png")), (&json!(at(&root, "found/b.png")), &Value::Null));
        assert!(frame(&mut app).contains("Not found"));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.status.starts_with("Relinked 1 image(s); b.png couldn't be relinked: "), "{}", app.ui.status);
        let d = app.ui.dialog.as_ref().expect("Linked File Not Found again");
        let names: Vec<&str> = d.fields["missing"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!((d.kind.as_str(), names.len()), (super::super::missing_links::KIND, 2));
        assert!(names.contains(&"b.png") && names.contains(&"c.png"), "{names:?}");
        super::super::cancel(&mut app);
        assert_eq!(app.run("links.check", json!({})).unwrap()["missing"], 2);
    }

    /// A file is chosen when the choice is clear: the only same file, else the only file. A row the
    /// user chose for keeps its choice as the search goes on.
    #[test]
    fn a_file_is_chosen_only_when_the_choice_is_clear() {
        let row = |p: &str| json!({"name": "a.png", "path": p, "ids": [1]});
        let mut d = Dialog::new(KIND, json!({"missing": [row("/x/a.png"), row("/y/a.png"), row("/z/a.png")], "picks": [], "touched": []}));
        let file = |p: &str, m: &str| json!({"path": p, "match": m});
        let answer = |y: Vec<Value>| {
            json!({"state": "searching", "links": [
                {"path": "/x/a.png", "ids": [1], "files": [file("/f/a.png", "same"), file("/g/a.png", "name")]},
                {"path": "/y/a.png", "ids": [1], "files": y},
                {"path": "/z/a.png", "ids": [1], "files": [file("/f/a.png", "size"), file("/g/a.png", "name")]},
            ]})
        };
        update(&mut d, &answer(vec![file("/f/a.png", "name")]));
        assert_eq!(d.fields["picks"], json!(["/f/a.png", "/f/a.png", null]));
        // Don't Relink chosen for the first row: the next answer leaves it, and the second row has
        // two files now.
        d.fields.insert("picks".into(), json!([null, "/f/a.png", null]));
        d.fields.insert("touched".into(), json!([0]));
        update(&mut d, &answer(vec![file("/f/a.png", "name"), file("/g/a.png", "name")]));
        assert_eq!(d.fields["picks"], json!([null, null, null]));
    }

    /// A search of another folder starts the choices over: a file chosen from the last folder's
    /// results isn't kept.
    #[test]
    fn another_folder_starts_the_choices_over() {
        let root = folder("again");
        let mut app = app_for(&root);
        opened_without(&mut app, &root, &["a.png"]);
        super::super::cancel(&mut app);
        // Two files of the name and neither the same file: the search reads both folders.
        write(&at(&root, "one/a.png"), b"one file");
        write(&at(&root, "one/x/a.png"), b"another file");
        std::fs::create_dir_all(root.join("two")).unwrap();
        app.run("ui.findLinksInFolder", json!({"folder": root.join("one")})).unwrap();
        searched(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["found"][0]["files"].as_array().map(Vec::len), Some(2));
        // The second file chosen, then another folder searched, which has no file of the name.
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("picks".into(), json!([at(&root, "one/x/a.png")]));
        d.fields.insert("touched".into(), json!([0]));
        d.fields.insert("searchFolder".into(), json!(root.join("two")));
        super::super::confirm(&mut app).unwrap();
        searched(&mut app);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((pick(d, "a.png"), &d.fields["touched"]), (&Value::Null, &json!([])));
    }

    #[test]
    fn cancel_closes_the_dialog_and_stops_its_search() {
        let root = folder("cancel");
        // Enough folders that the search usually still runs when the dialog closes.
        for a in 0..40 {
            for b in 0..40 {
                std::fs::create_dir_all(root.join(format!("tree/f{a}/g{b}"))).unwrap();
            }
        }
        let mut app = app_for(&root);
        app.session.search_threads = Some(1);
        opened_without(&mut app, &root, &["a.png"]);
        super::super::cancel(&mut app);
        app.run("ui.findLinksInFolder", json!({"folder": root.join("tree")})).unwrap();
        let id = app.ui.links_search.expect("the dialog's search");
        assert_eq!((kind(&app), app.ui.dialog.as_ref().unwrap().bool("back")), (Some(KIND), false));
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none() && app.ui.links_search.is_none());
        // Stopped by the close, unless the walker finished the folders first.
        let r = app.session.execute("links.findFiles", &json!({})).unwrap();
        assert_eq!(r["id"].as_u64(), Some(id), "{r}");
        match r["state"].as_str() {
            Some("stopped") => assert_eq!(r["stopped"], "stop", "{r}"),
            state => assert_eq!(state, Some("done"), "{r}"),
        }
    }

    /// Copy into the document's folder, for a saved document: the file is copied there first.
    #[test]
    fn copy_into_the_documents_folder_copies_before_relinking() {
        let root = folder("copy");
        let mut app = app_for(&root);
        let bytes = opened_without(&mut app, &root, &["a.png"]);
        super::super::cancel(&mut app);
        write(&at(&root, "found/a.png"), &bytes[0]);
        app.run("ui.findLinksInFolder", json!({"folder": root.join("found")})).unwrap();
        searched(&mut app);
        assert!(frame(&mut app).contains("Copy into the document's folder"));
        app.ui.dialog.as_mut().unwrap().fields.insert("copy".into(), json!(true));
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!((r["relinked"].as_array().unwrap().len(), app.ui.dialog.is_none()), (1, true), "{r}");
        let copy = at(&root, "doc/a.png");
        assert_eq!(std::fs::read(&copy).unwrap(), bytes[0]);
        let c = app.run("links.check", json!({})).unwrap();
        assert_eq!((c["missing"].as_u64(), c["links"][0]["path"].as_str()), (Some(0), Some(&*copy.to_string_lossy())), "{c}");
        // An unsaved document has no folder to copy into.
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let id = app.run("file.place", json!({"path": at(&root, "found/a.png")})).unwrap()["ids"][0].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("ui.findLinksInFolder", json!({"folder": root.join("found")})).unwrap();
        searched(&mut app);
        assert!(!frame(&mut app).contains("Copy into the document's folder"));
    }

    /// With Copy into the document's folder, a placed document is relinked where its file is.
    #[test]
    fn copy_leaves_placed_documents_where_they_are() {
        let root = folder("placed");
        let mut app = app_for(&root);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        let badge = at(&root, "orig/badge.vectorcraft");
        std::fs::create_dir_all(badge.parent().unwrap()).unwrap();
        app.run("document.save", json!({"path": badge})).unwrap();
        let found = at(&root, "found/badge.vectorcraft");
        write(&found, &std::fs::read(&badge).unwrap());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("file.place", json!({"path": badge, "link": true})).unwrap();
        let doc = at(&root, "doc/doc.vectorcraft");
        std::fs::create_dir_all(doc.parent().unwrap()).unwrap();
        app.run("document.save", json!({"path": doc})).unwrap();
        std::fs::remove_dir_all(root.join("orig")).unwrap();
        crate::io::open_path(&mut app, &doc.to_string_lossy()).unwrap();
        super::super::cancel(&mut app);
        app.run("ui.findLinksInFolder", json!({"folder": root.join("found")})).unwrap();
        searched(&mut app);
        app.ui.dialog.as_mut().unwrap().fields.insert("copy".into(), json!(true));
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!((r["relinked"].as_array().unwrap().len(), &r["failed"]), (1, &json!([])), "{r}");
        assert!(!at(&root, "doc/badge.vectorcraft").exists());
        assert_eq!(app.run("links.check", json!({})).unwrap()["links"][0]["path"], json!(found));
    }

    /// The Links panel's Find in Folder… (`ui.findLinksInFolder`): the selected linked files, else
    /// the missing ones, in a folder the user picks.
    #[test]
    fn the_links_panel_looks_for_the_selected_or_the_missing_files() {
        let root = folder("panel");
        let mut app = app_for(&root);
        app.services.pick_folder = None;
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        assert!(!crate::menus::enabled(&app, "ui.findLinksInFolder"), "no folder picker (the web)");
        let picked = root.join("found").to_string_lossy().into_owned();
        let answer = picked.clone();
        app.services.pick_folder = Some(Box::new(move || Some(answer.clone())));
        assert!(crate::menus::enabled(&app, "ui.findLinksInFolder"));
        // Nothing to look for: no folder is asked for.
        assert_eq!(app.run("ui.findLinksInFolder", json!({})).err().as_deref(), Some("No linked files to look for"));
        let here = at(&root, "found/here.png");
        write(&here, &png(1));
        let id = app.run("file.place", json!({"path": here})).unwrap()["ids"][0].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let menu = crate::tests_labels::painted_text(&mut app, crate::panels::links::menu);
        assert!(menu.contains("Find in Folder…"), "{menu}");
        app.run("ui.findLinksInFolder", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.fields["missing"][0]["name"].as_str(), d.str("folder")), (KIND, Some("here.png"), picked));
        // here.png isn't missing: the dialog asks about linked files.
        assert!(frame(&mut app).contains("Choose the file to relink each linked file to."));
        searched(&mut app);
        assert_eq!(pick(app.ui.dialog.as_ref().unwrap(), "here.png"), &json!(here));
    }

    /// The Links panel's Find in Folder… asks for the folder off the UI thread (#867): Find Linked
    /// Files opens on the folder when the folder picker answers.
    #[test]
    fn find_in_folder_asks_for_the_folder_off_the_ui_thread() {
        let root = folder("later");
        let mut app = app_for(&root);
        let (shown, _) = crate::tests_picks::off_the_ui_thread(&mut app.services);
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let here = at(&root, "found/here.png");
        write(&here, &png(1));
        let id = app.run("file.place", json!({"path": here})).unwrap()["ids"][0].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        // Nothing opens until the picker answers.
        assert!(app.run("ui.findLinksInFolder", json!({})).is_err());
        assert_eq!((shown.requests(), kind(&app)), (vec![PickRequest::Folder], None));
        let found = root.join("found").to_string_lossy().into_owned();
        shown.answer(&[found.as_str()]);
        crate::picks::poll(&mut app, &egui::Context::default());
        let d = app.ui.dialog.as_ref().expect("Find Linked Files");
        assert_eq!((d.kind.as_str(), d.str("folder")), (KIND, found));
        searched(&mut app);
        assert_eq!(pick(app.ui.dialog.as_ref().unwrap(), "here.png"), &json!(here));
    }
}
