//! The fonts a document opened without (`text.missingFonts`), asked about once the missing linked
//! file questions (dialog `missingLinks`) are answered, one document at a time ([`super::settle`]).
//! Search Fonts Folder (the
//! `Fonts` folder next to the document, as File › Package writes it) and Find in Folder… look for
//! their files (`text.findFontFiles`, on separate threads), and Add Fonts copies the files the
//! user chooses into VectorCraft's own Fonts folder (`text.addFontFiles`). Type › Find Font's
//! Find in Folder… opens it too (`ui.findFontsInFolder`), and so does `ui.missingFontsDialog`.
//!
//! Fields: `document` (the document's uid), `fonts` (`text.missingFonts`'s rows),
//! `fontsNextToDocument` (or empty), `state` (empty, `searching`, `done`, `stopped` or `failed`),
//! `search` (the search's `id`), `folder`, `found` (`text.findFontFiles`'s `fonts`, with the files
//! found), `searched`, `skipped`, `unreadable`, `stopped`, `error`, `chosen` (the files to add; a
//! font's first file is chosen when it is found) and `searchFolder` (set it and confirm to search
//! that folder).

use std::time::Duration;

use serde_json::{Value, json};

use super::DialogSpec;
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the fonts a document opened without.
pub const KIND: &str = "missingFonts";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Missing Fonts").into(),
    body,
    confirm,
    ok: Some("Add Fonts"),
    min_width: 440.0,
    max_width: Some(540.0),
    ..DialogSpec::FORM
};

/// After a document opened (`document.open`'s result `r`): its Missing Fonts dialog waits for the
/// dialog slot. On the web the status line shows how many fonts are missing instead, unless it
/// shows the open's notes.
pub fn after_open(app: &mut VectorcraftApp, r: &Value) {
    after_open_for(app, r, cfg!(target_arch = "wasm32"));
}

fn after_open_for(app: &mut VectorcraftApp, r: &Value, web: bool) {
    let Some(st) = app.session.active() else { return };
    let uid = st.uid;
    if web {
        let n = vectorcraft_engine::cmd::fontfiles::missing_fonts(&st.doc).len();
        if n > 0 && r["warnings"].as_array().is_none_or(Vec::is_empty) {
            app.status(format!("{n} font(s) aren't available here: their text shows in another font"));
        }
        return;
    }
    if !app.ui.pending_fonts.contains(&uid) {
        app.ui.pending_fonts.push(uid);
    }
}

/// The Missing Fonts dialog of the open document `uid`, when fonts it uses aren't available.
fn dialog_for(app: &VectorcraftApp, uid: u64) -> Option<Dialog> {
    let st = app.session.documents().iter().find(|d| d.uid == uid)?;
    let r = vectorcraft_engine::cmd::fontfiles::missing_fonts_of(st, app.session.search_rules.as_ref());
    let fonts = r["fonts"].as_array().filter(|f| !f.is_empty())?;
    let next = r["fontsNextToDocument"].as_str().unwrap_or_default();
    Some(Dialog::new(KIND, json!({ "document": uid, "fonts": fonts, "fontsNextToDocument": next, "state": "", "chosen": [] })))
}

/// With no dialog open: the dialog of the first queued document that is still open and still
/// misses fonts.
pub(super) fn open_next(app: &mut VectorcraftApp) {
    while app.ui.dialog.is_none() && !app.ui.pending_fonts.is_empty() {
        let uid = app.ui.pending_fonts.remove(0);
        app.ui.dialog = dialog_for(app, uid);
        if app.ui.dialog.is_some() {
            app.ui.fonts_dialog = Some(uid);
        }
    }
}

/// Put the document `uid` at the front of the queue.
fn queue_first(app: &mut VectorcraftApp, uid: u64) {
    app.ui.pending_fonts.retain(|u| *u != uid);
    app.ui.pending_fonts.insert(0, uid);
}

/// Once the Missing Fonts dialog on show is gone: when another dialog took its place (Import PDF
/// for a file opened after the document, for example), its document goes back to the front of the
/// queue, and the dialog opens again once no dialog is open. A dialog closed by Cancel, Add Fonts
/// or Escape stays closed.
pub(super) fn requeue_replaced(app: &mut VectorcraftApp) {
    let Some(uid) = app.ui.fonts_dialog else { return };
    let Some(d) = &app.ui.dialog else {
        app.ui.fonts_dialog = None;
        return;
    };
    if d.kind != KIND || d.fields.get("document").and_then(Value::as_u64) != Some(uid) {
        app.ui.fonts_dialog = None;
        queue_first(app, uid);
    }
}

/// Stop the search the dialog started once the dialog is gone (closed, or another in its place).
pub(super) fn stop_when_closed(app: &mut VectorcraftApp) {
    let Some(id) = app.ui.dialog_search else { return };
    let open = app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND && d.fields.get("search").and_then(Value::as_u64) == Some(id));
    if !open {
        app.ui.dialog_search = None;
        vectorcraft_engine::cmd::fontfiles::stop(&mut app.session, id);
    }
}

/// `ui.missingFontsDialog`: the dialog for the active document; with `folder`, searching it.
pub(crate) fn open_command(app: &mut VectorcraftApp, folder: Option<String>) -> Result<Value, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    let mut d = dialog_for(app, uid).ok_or("No fonts are missing in this document")?;
    if let Some(folder) = folder {
        start_search(app, &mut d, &folder)?;
    }
    show_for(app, uid, d);
    Ok(Value::Null)
}

/// Show `d`, the dialog of the document `uid`, in place of the open dialog (another document's
/// Missing Fonts dialog asks again later).
fn show_for(app: &mut VectorcraftApp, uid: u64, d: Dialog) {
    requeue_replaced(app);
    if let Some(shown) = app.ui.fonts_dialog.filter(|shown| *shown != uid) {
        queue_first(app, shown);
    }
    app.ui.pending_fonts.retain(|u| *u != uid);
    app.ui.fonts_dialog = Some(uid);
    app.ui.dialog = Some(d);
}

/// `ui.findFontsInFolder` (Type › Find Font's Find in Folder…): ask for a folder (or take
/// `folder`), then open the dialog for the active document, searching it.
pub(crate) fn find_in_folder_command(app: &mut VectorcraftApp, folder: Option<String>) -> Result<Value, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    // Nothing to look for: no folder is asked for.
    let mut d = dialog_for(app, uid).ok_or("No fonts are missing in this document")?;
    let folder = match folder {
        Some(f) => f,
        None => crate::picks::folder(app).ok_or("cancelled")?,
    };
    start_search(app, &mut d, &folder)?;
    show_for(app, uid, d);
    Ok(Value::Null)
}

/// Start looking in `folder` for the files of the dialog's fonts; what an earlier search found is
/// dropped.
fn start_search(app: &mut VectorcraftApp, d: &mut Dialog, folder: &str) -> Result<(), String> {
    let fonts: Vec<Value> = d
        .fields
        .get("fonts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| json!({ "family": f["family"], "style": f["style"] }))
        .collect();
    let r = app.run("text.findFontFiles", json!({ "folder": folder, "fonts": fonts }))?;
    for k in ["found", "searched", "skipped", "unreadable", "stopped", "error"] {
        d.fields.remove(k);
    }
    d.fields.insert("chosen".into(), json!([]));
    d.fields.insert("search".into(), r["id"].clone());
    d.fields.insert("folder".into(), json!(folder));
    update(d, &r);
    app.ui.dialog_search = r["id"].as_u64();
    Ok(())
}

/// The files a search found for the font `f` (`{family, style}`): the answer's `fonts` row with
/// the same family and style.
fn files_for<'a>(found: &'a [Value], f: &Value) -> &'a [Value] {
    found.iter().find(|r| r["family"] == f["family"] && r["style"] == f["style"]).and_then(|r| r["files"].as_array()).map_or(&[], Vec::as_slice)
}

/// A search's answer `r` into the dialog. A font that gets its first file has that file chosen; a
/// file the user unchecked stays unchecked.
fn update(d: &mut Dialog, r: &Value) {
    let before = d.fields.get("found").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut chosen = d.fields.get("chosen").and_then(Value::as_array).cloned().unwrap_or_default();
    for font in r["fonts"].as_array().into_iter().flatten() {
        let had = !files_for(&before, font).is_empty();
        if let Some(first) = font["files"].as_array().and_then(|f| f.first()).filter(|f| !had && !chosen.contains(f)) {
            chosen.push(first.clone());
        }
    }
    d.fields.insert("chosen".into(), json!(chosen));
    d.fields.insert("found".into(), r["fonts"].clone());
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
    let Ok(r) = app.session.execute("text.findFontFiles", &json!({})) else { return };
    if d.fields.get("search") != Some(&r["id"]) {
        d.fields.insert("state".into(), json!("stopped"));
        return;
    }
    update(d, &r);
    if let Some(e) = r["error"].as_str() {
        app.status(e);
    }
}

/// A button of the dialog's.
enum Act {
    Search(String),
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
    widgets::dim_label(ui, tl!("These fonts aren't available. Their text shows in another font."));
    ui.add_space(8.0);
    let fonts = d.fields.get("fonts").and_then(Value::as_array).cloned().unwrap_or_default();
    let found = d.fields.get("found").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut chosen = d.fields.get("chosen").and_then(Value::as_array).cloned().unwrap_or_default();
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for f in &fonts {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.add_space(3.0);
                        let (note, color) = crate::find_font::font_note(f);
                        let name = format!("{} {}{note}", f["family"].as_str().unwrap_or_default(), f["style"].as_str().unwrap_or_default());
                        ui.label(egui::RichText::new(name).font(theme::semibold(12.0)).color(color.unwrap_or(t.text_strong)));
                        let files = files_for(&found, f);
                        for file in files {
                            let path = file.as_str().unwrap_or_default();
                            let name = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
                            let on = chosen.contains(file);
                            if widgets::check(ui, &name, on, true) {
                                if on {
                                    chosen.retain(|c| c != file);
                                } else {
                                    chosen.push(file.clone());
                                }
                            }
                            ui.add(egui::Label::new(egui::RichText::new(path).size(11.0).color(t.text_dim)).truncate()).on_hover_text(path);
                        }
                        if files.is_empty() && !state.is_empty() {
                            widgets::dim_label(ui, if searching { tl!("Searching…") } else { tl!("Not found") });
                        }
                        ui.add_space(3.0);
                    });
                });
            }
        });
    });
    d.fields.insert("chosen".into(), json!(chosen));
    ui.add_space(10.0);
    let mut act = None;
    let can_pick = crate::picks::can(app, &PickRequest::Folder);
    ui.horizontal(|ui| {
        let next = d.str("fontsNextToDocument");
        if !next.is_empty() && ui.add_enabled(!searching, egui::Button::new(tl!("Search Fonts Folder"))).on_hover_text(&next).clicked() {
            act = Some(Act::Search(next));
        }
        if can_pick && ui.add_enabled(!searching, egui::Button::new(tl!("Find in Folder…"))).clicked() {
            act = Some(Act::Pick);
        }
        if searching {
            if ui.button(tl!("Stop")).clicked() {
                act = Some(Act::Stop);
            }
            ui.add(egui::Spinner::new().size(12.0).color(t.accent));
        }
    });
    if searching {
        widgets::dim_name(ui, &d.str("folder"));
    }
    if let Some(s) = d.fields.get("searched") {
        let (folders, files) = (s["folders"].to_string(), s["files"].to_string());
        widgets::dim_name(
            ui,
            &crate::i18n::fmt(tl!("{folders} folder(s) and {files} file(s) searched"), &[("folders", &folders), ("files", &files)]),
        );
    }
    let count = |k: &str| d.fields.get(k).and_then(Value::as_u64).filter(|n| *n > 0).map(|n| n.to_string());
    if let Some(n) = count("skipped") {
        let text = tl!("{count} folder(s) skipped, such as other apps' and the system's folders and hidden ones");
        widgets::dim_name(ui, &crate::i18n::fmt(text, &[("count", &n)]));
    }
    if let Some(n) = count("unreadable") {
        widgets::dim_name(ui, &crate::i18n::fmt(tl!("{count} folder(s) or file(s) couldn't be read"), &[("count", &n)]));
    }
    if matches!(d.fields.get("stopped").and_then(Value::as_str), Some("time" | "limit")) {
        widgets::dim_label(ui, tl!("The search stopped at its limit: search a smaller folder."));
    }
    ui.add_space(8.0);
    widgets::dim_label(ui, tl!("Copy only fonts you own or are licensed to install."));
    if let Some(dir) = vectorcraft_text::app_font_dir() {
        widgets::dim_name(ui, &crate::i18n::fmt(tl!("VectorCraft copies the fonts to {folder}."), &[("folder", &dir.to_string_lossy())]));
    }
    let r = match act {
        Some(Act::Search(folder)) => start_search(app, d, &folder),
        Some(Act::Pick) => crate::picks::in_dialog(app, d, |app, d| match crate::picks::folder(app) {
            Some(folder) => start_search(app, d, &folder),
            None => Ok(()),
        }),
        Some(Act::Stop) => app.run("text.findFontFiles", json!({ "stop": true })).map(|_| follow(app, d)),
        None => Ok(()),
    };
    if let Err(e) = r {
        app.status(e);
    }
    false
}

/// OK (Add Fonts): copy the chosen files into VectorCraft's Fonts folder and close. With
/// `searchFolder` (agents): search that folder instead, and stay open.
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
    let chosen = d.fields.get("chosen").and_then(Value::as_array).cloned().unwrap_or_default();
    if chosen.is_empty() {
        return Err("choose the font files to add".into());
    }
    let r = app.run("text.addFontFiles", json!({ "files": chosen }))?;
    app.ui.dialog = None;
    let len = |k: &str| r[k].as_array().map_or(0, Vec::len);
    let n = len("copied") + len("kept");
    let folder = r["folder"].as_str().unwrap_or_default();
    match r["skipped"].as_array().and_then(|s| s.first()) {
        None => app.status(format!("{n} font file(s) are now in {folder}")),
        Some(first) => {
            let failed = len("skipped");
            let file =
                first["file"].as_str().map(std::path::Path::new).and_then(std::path::Path::file_name).map(|f| f.to_string_lossy().into_owned());
            let reason = format!("{}: {}", file.unwrap_or_default(), first["reason"].as_str().unwrap_or_default());
            app.status(format!("{n} font file(s) are now in {folder}; {failed} couldn't be added: {reason}"));
        }
    }
    Ok(r)
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
        let dir = vectorcraft_testkit::temp_dir(&format!("missing-fonts-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        vectorcraft_engine::cmd::findfiles::plain(std::fs::canonicalize(&dir).unwrap())
    }

    /// An app that reads files and whose searches may go anywhere in `root`, on two threads.
    fn app_for(root: &Path) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.session.search_rules = Some(Rules { user_content: vec![root.to_path_buf()], ..Rules::default() });
        app.session.search_threads = Some(2);
        app
    }

    fn png() -> Vec<u8> {
        let mut out = vec![];
        image::RgbaImage::from_pixel(40, 30, image::Rgba([200, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// Save a new document with type in `font` at `path`, with a linked image that is then deleted
    /// when `link`.
    fn save_doc(app: &mut VectorcraftApp, path: &Path, font: &str, link: bool) {
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let pic = path.with_extension("png");
        if link {
            std::fs::write(&pic, png()).unwrap();
            app.run("file.place", json!({"path": pic.to_string_lossy()})).unwrap();
        }
        app.run("text.create", json!({"x": 10, "y": 40, "text": "Type", "font": font})).unwrap();
        app.run("document.save", json!({"path": path.to_string_lossy()})).unwrap();
        if link {
            std::fs::remove_file(&pic).unwrap();
        }
    }

    fn open(app: &mut VectorcraftApp, path: &Path) {
        crate::io::open_path(app, &path.to_string_lossy()).unwrap();
    }

    fn kind(app: &VectorcraftApp) -> Option<&str> {
        app.ui.dialog.as_ref().map(|d| d.kind.as_str())
    }

    #[test]
    fn missing_fonts_are_asked_about_after_the_missing_links() {
        let root = folder("after-links");
        let mut app = app_for(&root);
        let doc = root.join("doc.vectorcraft");
        save_doc(&mut app, &doc, "No Such Font Family", true);
        open(&mut app, &doc);
        assert_eq!(kind(&app), Some(super::super::missing_links::KIND));
        super::super::cancel(&mut app);
        let d = app.ui.dialog.as_ref().expect("Missing Fonts after the linked files");
        assert_eq!((d.kind.as_str(), d.fields["fonts"][0]["family"].as_str()), (KIND, Some("No Such Font Family")));
        let text = frame(&mut app);
        for label in [
            "Missing Fonts",
            "No Such Font Family Regular",
            "These fonts aren't available. Their text shows in another font.",
            "Copy only fonts you own or are licensed to install.",
            "Add Fonts",
            "Cancel",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        assert!(!text.contains("Search Fonts Folder"), "no Fonts folder next to the document");
        assert_eq!(super::super::confirm(&mut app).err().as_deref(), Some("choose the font files to add"));
        assert_eq!(kind(&app), Some(KIND), "nothing to add: the dialog stays open");
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn the_documents_fonts_folder_is_offered_and_searched_off_the_ui_thread() {
        let root = folder("package");
        let font = root.join("pkg/Fonts/sub/Findme.otf");
        std::fs::create_dir_all(font.parent().unwrap()).unwrap();
        std::fs::write(&font, vectorcraft_testkit::fonts::renamed("Findme Sans 3")).unwrap();
        let mut app = app_for(&root);
        let doc = root.join("pkg/doc.vectorcraft");
        save_doc(&mut app, &doc, "Findme Sans 3", false);
        open(&mut app, &doc);
        let next = app.ui.dialog.as_ref().filter(|d| d.kind == KIND).expect("Missing Fonts at once: no links missing").str("fontsNextToDocument");
        assert_eq!(next, root.join("pkg/Fonts").to_string_lossy());
        assert!(frame(&mut app).contains("Search Fonts Folder"));
        app.ui.dialog.as_mut().unwrap().fields.insert("searchFolder".into(), json!(next));
        let r = super::super::confirm(&mut app).unwrap();
        assert!(matches!(r["state"].as_str(), Some("searching" | "done")), "{r}");
        assert_eq!(kind(&app), Some(KIND), "the dialog stays open while it searches");
        let until = Instant::now() + Duration::from_secs(10);
        while app.ui.dialog.as_ref().is_some_and(|d| d.str("state") == "searching") && Instant::now() < until {
            frame(&mut app);
            std::thread::sleep(Duration::from_millis(5));
        }
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("state"), &d.fields["found"][0]["files"]), ("done".to_string(), &json!([font])));
        assert_eq!(d.fields["chosen"], json!([font]), "the font's file is chosen");
        assert!(frame(&mut app).contains("Findme.otf"));
        // Adding needs VectorCraft's Fonts folder, which only the apps set.
        let e = super::super::confirm(&mut app).unwrap_err();
        assert!(e.contains("isn't set here"), "{e}");
        assert_eq!(kind(&app), Some(KIND));
    }

    #[test]
    fn closing_the_dialog_stops_its_search() {
        let root = folder("stop");
        // Enough folders that the search usually still runs when the dialog closes.
        for a in 0..40 {
            for b in 0..40 {
                std::fs::create_dir_all(root.join(format!("f{a}/g{b}"))).unwrap();
            }
        }
        let mut app = app_for(&root);
        app.session.search_threads = Some(1);
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("text.create", json!({"x": 10, "y": 40, "text": "Gone", "font": "No Such Font Family"})).unwrap();
        app.run("ui.missingFontsDialog", json!({"folder": root})).unwrap();
        let id = app.ui.dialog_search.expect("the dialog's search");
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none() && app.ui.dialog_search.is_none());
        // Stopped by the close, unless the walker finished the folders first.
        let r = app.session.execute("text.findFontFiles", &json!({})).unwrap();
        assert_eq!(r["id"].as_u64(), Some(id), "{r}");
        match r["state"].as_str() {
            Some("stopped") => assert_eq!(r["stopped"], "stop", "{r}"),
            state => assert_eq!(state, Some("done"), "{r}"),
        }
        // Without missing fonts there is nothing to open.
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        assert_eq!(app.run("ui.missingFontsDialog", json!({})).err().as_deref(), Some("No fonts are missing in this document"));
    }

    #[test]
    fn one_dialog_at_a_time_document_by_document() {
        let root = folder("queue");
        let mut app = app_for(&root);
        let (a, b) = (root.join("a.vectorcraft"), root.join("b.vectorcraft"));
        save_doc(&mut app, &a, "Alpha Missing Family", false);
        save_doc(&mut app, &b, "Beta Missing Family", true);
        open(&mut app, &a);
        let family = |app: &VectorcraftApp| app.ui.dialog.as_ref().and_then(|d| d.fields["fonts"][0]["family"].as_str().map(str::to_string));
        assert_eq!(family(&app).as_deref(), Some("Alpha Missing Family"));
        // The second document's linked files are asked about first; then each document's fonts.
        open(&mut app, &b);
        assert_eq!(kind(&app), Some(super::super::missing_links::KIND));
        super::super::cancel(&mut app);
        assert_eq!(family(&app).as_deref(), Some("Alpha Missing Family"));
        super::super::cancel(&mut app);
        assert_eq!(family(&app).as_deref(), Some("Beta Missing Family"));
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        // A document closed before its turn is passed over.
        open(&mut app, &a);
        open(&mut app, &b);
        let index = app.session.documents().iter().rposition(|d| d.path.as_deref() == Some(&*a.to_string_lossy())).unwrap();
        app.session.execute("file.close", &json!({"index": index})).unwrap();
        super::super::cancel(&mut app);
        assert_eq!(family(&app).as_deref(), Some("Beta Missing Family"));
    }

    /// Another dialog that takes the Missing Fonts dialog's place (Import PDF for a PDF dropped
    /// with the document) hands the question back once it closes.
    #[test]
    fn the_question_comes_back_after_a_dialog_that_took_its_place() {
        let root = folder("replaced");
        let mut app = app_for(&root);
        let doc = root.join("a.vectorcraft");
        save_doc(&mut app, &doc, "Alpha Missing Family", false);
        let pdf = || {
            let page = || vectorcraft_testkit::pdf::PdfPage::new(100.0, 100.0, "0 g 0 0 10 10 re f");
            vectorcraft_testkit::pdf::pdf(&[page(), page()], None)
        };
        let opened = |name: &str, path: Option<String>, bytes: Vec<u8>| (crate::place::DropTarget::Open, (name.to_string(), path, bytes));
        let files =
            vec![opened("a.vectorcraft", Some(doc.to_string_lossy().into_owned()), std::fs::read(&doc).unwrap()), opened("b.pdf", None, pdf())];
        crate::place::drop_files(&mut app, files);
        assert_eq!(kind(&app), Some(super::super::import_pdf::KIND));
        super::super::cancel(&mut app);
        let family = |app: &VectorcraftApp| app.ui.dialog.as_ref().filter(|d| d.kind == KIND).map(|d| d.fields["fonts"][0]["family"].clone());
        assert_eq!(family(&app), Some(json!("Alpha Missing Family")));
        // The same when a frame drew the other dialog before it closed.
        crate::io::open_bytes(&mut app, "c.pdf", &pdf(), None).unwrap();
        assert_eq!(kind(&app), Some(super::super::import_pdf::KIND));
        frame(&mut app);
        super::super::cancel(&mut app);
        assert_eq!(family(&app), Some(json!("Alpha Missing Family")));
        // Closed by Cancel, or by Escape (which empties the dialog slot), it stays closed.
        super::super::cancel(&mut app);
        frame(&mut app);
        assert!(app.ui.dialog.is_none() && app.ui.pending_fonts.is_empty());
        app.run("ui.missingFontsDialog", json!({})).unwrap();
        app.ui.dialog = None;
        frame(&mut app);
        assert!(app.ui.dialog.is_none() && app.ui.pending_fonts.is_empty());
    }

    /// Type in a CSS generic family (`sans-serif`, the default font of SVG files from some editors)
    /// opens without the question.
    #[test]
    fn generic_families_are_not_asked_about() {
        let mut app = app_for(&folder("generic"));
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><text x="10" y="40" font-family="sans-serif">A</text></svg>"#;
        crate::io::open_bytes(&mut app, "generic.svg", svg.as_bytes(), None).unwrap();
        let listed = app.session.execute("text.fonts", &json!({})).unwrap();
        assert!(listed.as_array().unwrap().iter().any(|f| f["family"] == "sans-serif"), "{listed}");
        frame(&mut app);
        assert!(app.ui.dialog.is_none() && app.ui.pending_fonts.is_empty());
    }

    /// A `searchFolder` whose search doesn't start is dropped from the dialog: OK doesn't try it
    /// again.
    #[test]
    fn a_search_folder_that_fails_is_not_tried_again() {
        let root = folder("retry");
        let mut app = app_for(&root);
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("text.create", json!({"x": 10, "y": 40, "text": "Gone", "font": "No Such Font Family"})).unwrap();
        app.run("ui.missingFontsDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("searchFolder".into(), json!("relative/folder"));
        let e = super::super::confirm(&mut app).unwrap_err();
        assert!(e.contains("absolute"), "{e}");
        assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("searchFolder"));
        assert_eq!(super::super::confirm(&mut app).err().as_deref(), Some("choose the font files to add"));
    }

    /// A placed document that Links › Edit Original opens asks about its fonts, as File › Open does.
    #[test]
    fn edit_original_asks_about_the_placed_documents_fonts() {
        let root = folder("edit-original");
        let mut app = app_for(&root);
        let badge = root.join("badge.vectorcraft");
        save_doc(&mut app, &badge, "Badge Missing Family", false);
        app.session.execute("file.close", &json!({})).unwrap();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("file.place", json!({"path": badge.to_string_lossy(), "link": true})).unwrap();
        let tabs = app.session.documents().len();
        app.run("links.editOriginal", json!({})).unwrap();
        assert_eq!(app.session.documents().len(), tabs + 1);
        let d = app.ui.dialog.as_ref().expect("Missing Fonts for the document opened");
        assert_eq!((d.kind.as_str(), d.fields["fonts"][0]["family"].as_str()), (KIND, Some("Badge Missing Family")));
    }

    /// A search leaves out the fonts that became available: the files it found go with their fonts
    /// by name, and a file the user unchecked stays unchecked as the search goes on.
    #[test]
    fn files_go_with_their_fonts_by_name() {
        let (a, b) = (json!({"family": "A Sans", "style": "Regular"}), json!({"family": "B Sans", "style": "Bold"}));
        let mut d = Dialog::new(KIND, json!({"fonts": [a, b], "chosen": []}));
        update(&mut d, &json!({"state": "searching", "fonts": [{"family": "B Sans", "style": "Bold", "files": ["/f/b.otf"]}]}));
        let found = d.fields["found"].as_array().unwrap().clone();
        assert!(files_for(&found, &a).is_empty());
        assert_eq!(files_for(&found, &b), [json!("/f/b.otf")]);
        assert_eq!(d.fields["chosen"], json!(["/f/b.otf"]));
        d.fields.insert("chosen".into(), json!([]));
        update(&mut d, &json!({"state": "done", "fonts": [{"family": "B Sans", "style": "Bold", "files": ["/f/b.otf", "/g/b.otf"]}]}));
        assert_eq!((d.str("state"), &d.fields["chosen"]), ("done".to_string(), &json!([])));
    }

    /// The status lines are translated with the reasons in them, and so are the reasons a search
    /// failed and a file wasn't copied, which the code holds as constants.
    #[test]
    fn the_status_lines_read_in_the_interface_language() {
        let fr = crate::i18n::Lang::from_code("fr").unwrap();
        let shown = crate::i18n::message(fr, "2 font file(s) are now in /Fonts; 1 couldn't be added: a.otf: larger than 256 MB");
        assert!(shown.contains("se trouvent maintenant dans /Fonts") && shown.contains("a.otf") && shown.contains("plus de 256 Mo"), "{shown}");
        for code in ["es", "fr", "it", "ru"] {
            let lang = crate::i18n::Lang::from_code(code).unwrap();
            for s in [
                "every search thread stopped on an internal error (please report this bug)",
                "not a file",
                "the files come to more than 1 GB: add fewer at a time",
                "every numbered name for it is taken in the Fonts folder",
            ] {
                assert_ne!(crate::i18n::message(lang, s), s, "{code}");
            }
        }
    }

    #[test]
    fn the_web_says_how_many_fonts_are_missing() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("text.create", json!({"x": 10, "y": 40, "text": "Gone", "font": "No Such Font Family"})).unwrap();
        after_open_for(&mut app, &json!({}), true);
        assert_eq!(app.ui.status, "1 font(s) aren't available here: their text shows in another font");
        assert!(app.ui.pending_fonts.is_empty());
        // The open's notes stay on show.
        app.ui.status = "Opened with 1 note(s): fonts".into();
        after_open_for(&mut app, &json!({"warnings": ["fonts"]}), true);
        assert_eq!(app.ui.status, "Opened with 1 note(s): fonts");
    }
}
