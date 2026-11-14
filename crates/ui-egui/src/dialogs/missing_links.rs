//! The linked files a document opened without, one at a time: Replace picks another file
//! (`links.relink`), and with Apply to All the other missing files are looked for by name in its
//! folder; Ignore keeps the images showing their saved preview, and with Apply to All ignores the
//! rest. Find in Folder… looks for every missing file in a folder and its subfolders (dialog
//! `findLinks`, [`super::find_links`]). Afterwards, with Preferences › Update Links: Ask When
//! Modified, the modified links are offered for update (`links.update`). Files another app changes
//! while the document is open are offered the same way ([`ask_update_changed`], from
//! [`vectorcraft_engine::link_watch`]).
//!
//! Fields: `missing` (`[{name, path, ids}]`, the one asked about first), `modified` (the ids of
//! images whose file was modified), `notes` (the open's notes, shown with the line about the
//! modified files), `applyToAll`, `path` (the replacement file; picked when empty), `discard`
//! (Ignore) and `searchFolder` (set it and confirm for Find in Folder… on that folder).

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of a missing linked file.
pub const KIND: &str = "missingLinks";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Linked File Not Found").into(),
    body,
    confirm,
    ok: Some("Replace…"),
    discard: Some("Ignore"),
    min_width: 380.0,
    max_width: Some(460.0),
    inputs: |_| &["searchFolder"],
    ..DialogSpec::FORM
};

/// The ids in `rows` (`[{ids}]`).
fn ids(rows: &[Value]) -> Vec<Value> {
    rows.iter().flat_map(|r| r["ids"].as_array().cloned().unwrap_or_default()).collect()
}

/// After a document opened (`document.open`'s result `r`): ask about its missing linked files,
/// then about its modified ones, then about the fonts it opened without
/// ([`super::missing_fonts`]). The web reads no linked files, so missing ones just show their
/// previews there. The status bar shows what it says about the linked files followed by the open's
/// `notes` ([`crate::io::show_opened`]). On the web, with both missing and modified files and
/// Update Links not asking, the line about the missing files comes first and the line about the
/// modified ones is the first of the notes.
pub fn after_open(app: &mut VectorcraftApp, r: &Value, notes: Vec<String>) {
    after_open_for(app, r, notes, cfg!(target_arch = "wasm32"));
}

fn after_open_for(app: &mut VectorcraftApp, r: &Value, mut notes: Vec<String>, web: bool) {
    // The fonts' dialog waits for these questions (one dialog at a time).
    super::missing_fonts::after_open(app, r);
    let rows = |k: &str| r[k].as_array().cloned().unwrap_or_default();
    let (missing, modified) = (rows("missingLinks"), ids(&rows("modifiedLinks")));
    let notice = if missing.is_empty() || web {
        let unreadable = (!missing.is_empty()).then(|| unreadable_status(missing.len()));
        match (ask_update(app, modified), unreadable) {
            (Some(changed), Some(unreadable)) => {
                notes.insert(0, changed);
                Some(unreadable)
            }
            (changed, unreadable) => changed.or(unreadable),
        }
    } else {
        open(app, missing, modified, &notes);
        None
    };
    crate::io::show_opened(app, notice, notes);
    super::settle(app);
}

/// The status bar's line for `n` linked files the web can't read.
fn unreadable_status(n: usize) -> String {
    crate::i18n::msg_n(
        n as u64,
        "{n} linked file can't be read here: its preview shows",
        "{n} linked files can't be read here: their previews show",
        &[],
    )
}

/// What the linked files of a document that opened (`document.open`'s result `r`) need, without
/// asking yet → the status bar's lines about them, the missing files to ask about and the ids of
/// the modified images to offer for update ([`ask_next`]). The web reads no linked files, so the
/// missing ones get a line there; without Update Links: Ask When Modified the modified ones get a
/// line instead of the offer.
pub(crate) fn questions(app: &VectorcraftApp, r: &Value) -> (Vec<String>, Vec<Value>, Vec<Value>) {
    questions_for(app, r, cfg!(target_arch = "wasm32"))
}

fn questions_for(app: &VectorcraftApp, r: &Value, web: bool) -> (Vec<String>, Vec<Value>, Vec<Value>) {
    let rows = |k: &str| r[k].as_array().cloned().unwrap_or_default();
    let (mut missing, mut modified) = (rows("missingLinks"), ids(&rows("modifiedLinks")));
    let mut lines = vec![];
    if web && !missing.is_empty() {
        lines.push(unreadable_status(missing.len()));
        missing.clear();
    }
    if !modified.is_empty() && app.session.prefs.update_links != "askWhenModified" {
        lines.push(changed_on_disk_status(modified.len()));
        modified.clear();
    }
    (lines, missing, modified)
}

fn open(app: &mut VectorcraftApp, missing: Vec<Value>, modified: Vec<Value>, notes: &[String]) {
    let fields = json!({ "missing": missing, "modified": modified, "notes": notes, "applyToAll": false, "path": "" });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

/// The open's notes a dialog `d` of the missing linked files keeps (field `notes`).
pub(super) fn notes(d: &Dialog) -> Vec<String> {
    let notes = d.fields.get("notes").and_then(Value::as_array).into_iter().flatten();
    notes.filter_map(Value::as_str).map(str::to_string).collect()
}

/// Ask about the missing files `rest`, one at a time, then offer the modified ones `modified` for
/// update. A line about the modified files is followed by the open's `notes`
/// ([`crate::io::show_opened`]).
pub(crate) fn ask_next(app: &mut VectorcraftApp, rest: Vec<Value>, modified: Vec<Value>, notes: Vec<String>) {
    if rest.is_empty() {
        app.ui.dialog = None;
        if let Some(notice) = ask_update(app, modified) {
            crate::io::show_opened(app, Some(notice), notes);
        }
    } else {
        open(app, rest, modified, &notes);
    }
}

/// With Update Links: Ask When Modified, offer to read the modified linked files `ids` again;
/// otherwise → the status bar's line for them ([`changed_on_disk_status`]).
fn ask_update(app: &mut VectorcraftApp, ids: Vec<Value>) -> Option<String> {
    ask(
        app,
        ids,
        "{n} linked image changed since this document was saved. Show the new version?",
        "{n} linked images changed since this document was saved. Show the new versions?",
    )
}

/// The same offer for linked files another app changed while the document is open
/// ([`vectorcraft_engine::link_watch`]).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn ask_update_changed(app: &mut VectorcraftApp, ids: Vec<Value>) {
    let status =
        ask(app, ids, "{n} linked image changed on disk. Show the new version?", "{n} linked images changed on disk. Show the new versions?");
    if let Some(status) = status {
        app.status(status);
    }
}

/// Ask about the modified linked files `ids`; `one` and `other` are the English forms of the
/// question. Without Update Links: Ask When Modified → the status bar's line for them instead.
fn ask(app: &mut VectorcraftApp, ids: Vec<Value>, one: &str, other: &str) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    if app.session.prefs.update_links != "askWhenModified" {
        return Some(changed_on_disk_status(ids.len()));
    }
    let detail = crate::i18n::tn(ids.len() as u64, one, other);
    super::confirm::ask(app, tl!("Update Modified Links"), &detail, "links.update", json!({ "ids": ids }));
    None
}

/// The status bar's line for `n` linked images that changed on disk while Update Links doesn't ask.
pub(crate) fn changed_on_disk_status(n: usize) -> String {
    crate::i18n::msg_n(
        n as u64,
        "{n} linked image changed on disk: Update Links shows the new version",
        "{n} linked images changed on disk: Update Links shows the new versions",
        &[],
    )
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let missing = d.fields.get("missing").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let Some(current) = missing.first() else { return true };
    let rest = missing.len() - 1;
    let name = current["name"].as_str().unwrap_or_default();
    widgets::list_box(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.add_space(3.0);
                ui.label(egui::RichText::new(name).font(theme::semibold(12.0)).color(t.text_strong));
                let path = current["path"].as_str().unwrap_or_default();
                ui.add(egui::Label::new(egui::RichText::new(path).size(11.5).color(t.text_dim)).truncate()).on_hover_text(path);
                ui.add_space(3.0);
            });
        });
    });
    ui.add_space(8.0);
    widgets::dim_label(ui, tl!("Replace it with another file, or ignore it: its images keep showing the preview saved with the document."));
    if crate::picks::can(app, &crate::picks::PickRequest::Folder) {
        ui.add_space(8.0);
        if ui.button(tl!("Find in Folder…")).clicked() {
            let r = crate::picks::in_dialog(app, d, |app, d| match crate::picks::folder(app) {
                Some(folder) => super::find_links::from_missing_links(app, d, &folder),
                None => Ok(()),
            });
            if let Err(e) = r {
                app.status(e);
            }
        }
    }
    if rest > 0 {
        ui.add_space(10.0);
        let all = d.bool("applyToAll");
        let label = crate::i18n::tn(rest as u64, "Apply to All ({n} more missing)", "Apply to All ({n} more missing)");
        if widgets::check(ui, &label, all, true) {
            d.fields.insert("applyToAll".into(), json!(!all));
        }
    }
    false
}

/// Replace (with `path`, else a picked file) or Ignore (`discard`) the first missing file, then
/// ask about the next. With `searchFolder`: Find in Folder… on that folder.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let folder = d.str("searchFolder");
    if !folder.is_empty() {
        let mut d = d.clone();
        // The folder is searched once: a search that doesn't start leaves the dialog without it.
        if let Some(open) = app.ui.dialog.as_mut().filter(|open| open.kind == KIND) {
            open.fields.remove("searchFolder");
        }
        super::find_links::from_missing_links(app, &mut d, &folder)?;
        let state = d.fields.get("state").cloned().unwrap_or(Value::Null);
        app.ui.dialog = Some(d);
        return Ok(json!({ "state": state }));
    }
    let missing = d.fields.get("missing").and_then(Value::as_array).cloned().unwrap_or_default();
    let modified = d.fields.get("modified").and_then(Value::as_array).cloned().unwrap_or_default();
    let all = d.bool("applyToAll");
    let Some((current, rest)) = missing.split_first() else {
        app.ui.dialog = None;
        return Ok(Value::Null);
    };
    let mut rest = rest.to_vec();
    let mut out = Value::Null;
    if d.bool("discard") {
        if all {
            rest.clear();
        }
    } else {
        let path = match d.str("path") {
            p if !p.is_empty() => p,
            _ => {
                let pick =
                    crate::FilePick { filters: crate::FilePick::filters(vectorcraft_engine::cmd::fileio::place_filters()), ..Default::default() };
                crate::picks::open(app, &pick).ok_or("cancelled")?
            }
        };
        out = app.run("links.relink", json!({ "ids": current["ids"], "path": path }))?;
        // Apply to All: the others by name in the same folder; any not there are asked about.
        let folder = std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned());
        if let (true, false, Some(folder)) = (all, rest.is_empty(), folder) {
            let r = app.run("links.relink", json!({ "ids": ids(&rest), "folder": folder }))?;
            let done = r["relinked"].as_array().cloned().unwrap_or_default();
            rest.retain(|row| row["ids"].as_array().is_none_or(|ids| ids.iter().any(|id| !done.contains(id))));
        }
    }
    ask_next(app, rest, modified, notes(d));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn frame(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    fn png(rgb: [u8; 3]) -> Vec<u8> {
        let mut out = vec![];
        let [r, g, b] = rgb;
        image::RgbaImage::from_pixel(400, 300, image::Rgba([r, g, b, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A document with two placed linked files, saved to a fresh folder; the files are then
    /// deleted → (app, folder, document path).
    fn opened_without_links(name: &str) -> (VectorcraftApp, std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("vectorcraft-missing-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        for n in ["a.png", "b.png"] {
            let p = dir.join(n);
            std::fs::write(&p, png([200, 0, 0])).unwrap();
            app.run("file.place", json!({"path": p.to_string_lossy()})).unwrap();
        }
        let doc = dir.join("doc.vectorcraft").to_string_lossy().into_owned();
        app.run("document.save", json!({"path": doc})).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::remove_file(dir.join(n)).unwrap();
        }
        // Reopened: an open file would only come to the front (#3455).
        app.run("file.close", json!({})).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        (app, dir, doc)
    }

    #[test]
    fn opening_without_linked_files_asks_to_replace_or_ignore_each() {
        let (mut app, dir, _) = opened_without_links("each");
        let d = app.ui.dialog.as_ref().expect("the dialog");
        assert_eq!((d.kind.as_str(), d.fields["missing"].as_array().unwrap().len()), (KIND, 2));
        let text = frame(&mut app);
        for label in ["Linked File Not Found", "a.png", "Apply to All (1 more missing)", "Replace…", "Ignore", "Cancel"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        // Replace the first with a file given by path (the file picker otherwise).
        let blue = dir.join("new/blue.png");
        std::fs::create_dir_all(blue.parent().unwrap()).unwrap();
        std::fs::write(&blue, png([0, 0, 200])).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("path".into(), json!(blue.to_string_lossy()));
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!(r["relinked"].as_array().unwrap().len(), 1);
        let d = app.ui.dialog.as_ref().expect("asks about the next");
        assert_eq!(d.fields["missing"][0]["name"], "b.png");
        assert!(!d.fields.contains_key("discard"));
        assert!(frame(&mut app).contains("b.png"));
        // Ignore: done, the image keeps its preview.
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let c = app.run("links.check", json!({})).unwrap();
        let status: Vec<&str> = c["links"].as_array().unwrap().iter().map(|l| l["status"].as_str().unwrap()).collect();
        assert_eq!((status, c["missing"].as_u64()), (vec!["ok", "missing"], Some(1)));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A restored document's linked files: the web gets a line about the missing ones, and
    /// without Update Links: Ask When Modified the modified ones get a line; the rest are asked
    /// about.
    #[test]
    fn the_questions_about_linked_files_wait_or_become_lines() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let r = json!({ "missingLinks": [{"name": "a.png", "path": "/x/a.png", "ids": [1]}], "modifiedLinks": [{"ids": [2]}, {"ids": [3]}] });
        app.session.prefs.update_links = "askWhenModified".into();
        assert_eq!(questions_for(&app, &r, false), (vec![], r["missingLinks"].as_array().unwrap().clone(), vec![json!(2), json!(3)]));
        let changed = "2 linked images changed on disk: Update Links shows the new versions".to_string();
        app.session.prefs.update_links = "manually".into();
        assert_eq!(questions_for(&app, &r, false), (vec![changed.clone()], r["missingLinks"].as_array().unwrap().clone(), vec![]));
        let unreadable = "1 linked file can't be read here: its preview shows".to_string();
        assert_eq!(questions_for(&app, &r, true), (vec![unreadable, changed], vec![], vec![]));
        assert!(app.ui.dialog.is_none(), "nothing is asked yet");
    }

    /// The question about modified linked files uses the form the language's plural rule picks for
    /// the count.
    #[test]
    fn the_modified_links_question_uses_each_count_form() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let mut question = |code: &str, n: u64| {
            crate::i18n::set_current(crate::i18n::Lang::from_code(code).unwrap());
            ask_update(&mut app, (0..n).map(|id| json!(id)).collect());
            let detail = app.ui.dialog.take().map(|d| d.str("detail"));
            crate::i18n::set_current(crate::i18n::Lang::EN);
            detail.unwrap_or_default()
        };
        assert_eq!(question("en", 1), "1 linked image changed since this document was saved. Show the new version?");
        assert_eq!(question("cs", 1), "Od uložení dokumentu se změnil 1 propojený obrázek. Zobrazit novou verzi?");
        assert_eq!(question("cs", 2), "Od uložení dokumentu se změnily 2 propojené obrázky. Zobrazit nové verze?");
        assert_eq!(question("cs", 5), "Od uložení dokumentu se změnilo 5 propojených obrázků. Zobrazit nové verze?");
    }

    /// On the web, an open with two notes and two linked files it can't read: the status bar says
    /// so about the files, then counts the notes, and keeps the notes for its hover. Without notes
    /// the line about the files is shown alone.
    #[test]
    fn an_open_with_unreadable_links_keeps_import_notes_reachable() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let missing = json!([{ "name": "a.png", "path": "/gone/a.png", "ids": [1] }, { "name": "b.png", "path": "/gone/b.png", "ids": [2] }]);
        let notes = vec!["a note".to_string(), "another note".to_string()];
        let r = json!({ "missingLinks": missing, "warnings": notes });
        after_open_for(&mut app, &r, notes.clone(), true);
        let notice = "2 linked files can't be read here: their previews show";
        assert_eq!(app.ui.status, crate::io::opened_after_notice(notice, 2, "a note"));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes)));
        assert!(app.ui.dialog.is_none());
        after_open_for(&mut app, &json!({ "missingLinks": missing }), vec![], true);
        assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), (notice, &None));
    }

    /// On the web, an open with linked files it can't read and modified ones, with Update Links
    /// not asking: the status bar shows the line about the unreadable files and counts the line
    /// about the modified ones as the first note, ahead of the open's notes, and keeps them all
    /// for its hover. With Ask When Modified the update question opens and the line about the
    /// unreadable files stays.
    #[test]
    fn a_web_open_with_unreadable_and_modified_links_keeps_both_lines() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.session.prefs.update_links = "manually".into();
        let missing = json!([{ "name": "a.png", "path": "/gone/a.png", "ids": [1] }, { "name": "b.png", "path": "/gone/b.png", "ids": [2] }]);
        let r = json!({ "missingLinks": missing, "modifiedLinks": [{ "ids": [3] }] });
        let unreadable = "2 linked files can't be read here: their previews show";
        let changed = changed_on_disk_status(1);

        let notes = vec!["a note".to_string(), "another note".to_string()];
        after_open_for(&mut app, &r, notes.clone(), true);
        let kept = [vec![changed.clone()], notes].concat();
        assert_eq!(app.ui.status, crate::io::opened_after_notice(unreadable, 3, &changed));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), kept)));
        assert!(app.ui.dialog.is_none());

        after_open_for(&mut app, &r, vec![], true);
        assert_eq!(app.ui.status, crate::io::opened_after_notice(unreadable, 1, &changed));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), vec![changed.clone()])));

        // One kind of line alone shows as before.
        after_open_for(&mut app, &json!({ "modifiedLinks": [{ "ids": [3] }] }), vec![], true);
        assert_eq!((app.ui.status.clone(), &app.ui.status_notes), (changed, &None));

        app.session.prefs.update_links = "askWhenModified".into();
        after_open_for(&mut app, &r, vec![], true);
        let d = app.ui.dialog.as_ref().expect("asks");
        assert_eq!((d.kind.as_str(), d.str("__command")), (super::super::confirm::KIND, "links.update".to_string()));
        assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), (unreadable, &None));
    }

    /// The line about modified linked files that follows the Linked File Not Found dialog keeps the
    /// notes of that open, also when another message was shown meanwhile. An open without notes
    /// shows the line alone, even while the status bar shows an earlier open's notes, and a later
    /// line about files changed on disk while the document is open doesn't take them.
    #[test]
    fn modified_links_after_the_missing_ones_keep_the_open_notes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.session.prefs.update_links = "manually".into();
        let notes = vec!["a note".to_string(), "another note".to_string()];
        let links = json!({
            "missingLinks": [{ "name": "a.png", "path": "/gone/a.png", "ids": [1] }],
            "modifiedLinks": [{ "ids": [2] }],
        });
        let ignore = |app: &mut VectorcraftApp| {
            app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
            super::super::confirm(app).unwrap();
            assert!(app.ui.dialog.is_none());
        };
        after_open_for(&mut app, &links, notes.clone(), false);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        assert_eq!(app.ui.status, crate::io::opened_with_notes(2, "a note"));
        app.status("Relinked 1 image");
        ignore(&mut app);
        assert_eq!(app.ui.status, crate::io::opened_after_notice(&changed_on_disk_status(1), 2, "a note"));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes.clone())));

        crate::io::show_opened(&mut app, None, notes);
        after_open_for(&mut app, &links, vec![], false);
        ignore(&mut app);
        assert_eq!((app.ui.status.clone(), &app.ui.status_notes), (changed_on_disk_status(1), &None));

        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::io::show_opened(&mut app, None, vec!["a note".to_string(), "another note".to_string()]);
            ask_update_changed(&mut app, vec![json!(2)]);
            assert_eq!(app.ui.status, changed_on_disk_status(1));
            assert_ne!(app.ui.status_notes.as_ref().map(|(status, _)| status), Some(&app.ui.status));
        }
    }

    #[test]
    fn apply_to_all_replaces_the_others_from_the_same_folder() {
        let (mut app, dir, _) = opened_without_links("all");
        let found = dir.join("found");
        std::fs::create_dir_all(&found).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::write(found.join(n), png([0, 0, 200])).unwrap();
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("applyToAll".into(), json!(true));
        d.fields.insert("path".into(), json!(found.join("a.png").to_string_lossy()));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "both relinked");
        let c = app.run("links.check", json!({})).unwrap();
        assert_eq!(c["missing"], 0, "{c}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn modified_links_are_offered_for_update_when_the_preference_asks() {
        let dir = std::env::temp_dir().join(format!("vectorcraft-modified-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let pic = dir.join("a.png");
        std::fs::write(&pic, png([200, 0, 0])).unwrap();
        app.run("file.place", json!({"path": pic.to_string_lossy()})).unwrap();
        let doc = dir.join("doc.vectorcraft").to_string_lossy().into_owned();
        app.run("document.save", json!({"path": doc})).unwrap();
        let original_stamp = std::fs::metadata(&pic).unwrap().modified().unwrap();
        let mark_modified = |seconds| {
            // Link stamps use milliseconds. Equal-size writes can finish within one tick;
            // advance the fixture's mtime explicitly rather than relying on a sleep or load.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&pic)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(original_stamp + std::time::Duration::from_secs(seconds)))
                .unwrap();
        };
        std::fs::write(&pic, png([0, 0, 200])).unwrap();
        mark_modified(2);
        // Reopened: an open file would only come to the front (#3455).
        app.run("file.close", json!({})).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        let d = app.ui.dialog.as_ref().expect("asks");
        assert_eq!((d.kind.as_str(), d.str("__command")), (super::super::confirm::KIND, "links.update".to_string()));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.run("links.check", json!({})).unwrap()["modified"], 0);
        // Manually: no question.
        app.run("prefs.set", json!({"key": "updateLinks", "value": "manually"})).unwrap();
        std::fs::write(&pic, png([0, 200, 0])).unwrap();
        mark_modified(4);
        app.session.execute("file.close", &json!({})).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.run("links.check", json!({})).unwrap()["modified"], 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
