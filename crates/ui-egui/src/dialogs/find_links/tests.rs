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
    for label in ["Find Linked Files", "a.png", "b.png", "Same file", "Don't Relink", "Find in Folder…", "Copy into the document's folder", "Relink"]
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
