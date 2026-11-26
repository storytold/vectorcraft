use std::path::{Path, PathBuf};
use std::time::Instant;

use vectorcraft_engine::cmd::findfiles::Rules;

use super::super::missing_links::{MissingLinksDialog, drive};
use super::*;

/// A fresh temporary folder (canonical, as a search reports it).
fn folder(tag: &str) -> PathBuf {
    let dir = vectorcraft_testkit::temp_dir(&format!("find-links-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    vectorcraft_engine::cmd::findfiles::plain(std::fs::canonicalize(&dir).unwrap())
}

/// An app that reads files, with a window dialog host, and whose searches may go anywhere in `root`,
/// on two threads.
fn app_for(root: &Path) -> VectorcraftApp {
    let mut app = drive::app();
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
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
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(40, 30, image::Rgba([200, n.wrapping_mul(40), 0, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| out.extend_from_slice(&__b))
    }
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
    // Reopened: an open file would only come to the front (#3455).
    app.run("file.close", json!({})).unwrap();
    crate::io::open_path(app, &doc.to_string_lossy()).unwrap();
    files
}

/// How long a test waits for the dialog's search to end. The wait only ends a test whose search
/// never ends. A machine busy with other builds can hold a walker for seconds. It is shorter than a
/// search's default `maxSeconds` of 60, which would end a search that does not end by itself.
const WAIT: Duration = Duration::from_secs(30);

/// The fields of the open Find Linked Files dialog.
fn found(app: &VectorcraftApp) -> Value {
    drive::fields::<FindLinksDialog>(app)
}

/// Pump frames until the dialog's search ends ([`WAIT`] at most).
fn searched(app: &mut VectorcraftApp) {
    let until = Instant::now() + WAIT;
    while found(app)["state"] == "searching" && Instant::now() < until {
        drive::pump(app);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The pick of the row named `name` in the dialog's `fields`.
fn pick(fields: &Value, name: &str) -> Value {
    let i = fields["missing"].as_array().unwrap().iter().position(|r| r["name"] == name).unwrap();
    fields["picks"][i].clone()
}

/// The folder the open dialog copies into, when it offers Copy.
fn copy_folder(app: &VectorcraftApp) -> Option<String> {
    app.dialogs().and_then(|d| d.with(|h| h.get::<FindLinksDialog>().map(|d| d.copy_folder.clone()))).expect("the dialog is open")
}

/// The Find Linked Files state `v` as the dialog keeps it.
fn state(v: Value) -> Fields {
    match v {
        Value::Object(fields) => fields,
        other => panic!("fields are an object, got {other}"),
    }
}

/// Find in Folder… in Linked File Not Found: each missing file's same file is chosen, and Relink
/// relinks them.
#[test]
fn find_in_folder_relinks_the_missing_files_to_the_files_found() {
    let root = folder("relink");
    let mut app = app_for(&root);
    let bytes = opened_without(&mut app, &root, &["a.png", "b.png"]);
    assert!(drive::is_open::<MissingLinksDialog>(&app));
    let (a, b) = (at(&root, "found/sub/a.png"), at(&root, "found/b.png"));
    write(&a, &bytes[0]);
    write(&b, &bytes[1]);
    write(&at(&root, "found/other/b.png"), b"another file");
    // What Find in Folder… does with the folder picked.
    drive::answer::<MissingLinksDialog>(&mut app, &[("searchFolder", json!(root.join("found")))], true);
    assert!(drive::is_open::<FindLinksDialog>(&app) && !drive::is_open::<MissingLinksDialog>(&app));
    assert!(matches!(found(&app)["state"].as_str(), Some("searching" | "done")), "{}", found(&app));
    searched(&mut app);
    let f = found(&app);
    assert_eq!((&f["state"], pick(&f, "a.png"), pick(&f, "b.png")), (&json!("done"), json!(a), json!(b)));
    drive::answer::<FindLinksDialog>(&mut app, &[], true);
    assert!(!drive::is_open::<FindLinksDialog>(&app));
    assert_eq!(app.ui.status, "Relinked 2 images");
    assert_eq!(app.run("links.check", json!({})).unwrap()["missing"], 0);
}

/// The files left without a file, or whose file fails to relink, go back to Linked File Not
/// Found, with the open's notes; the failure is named in the status line, and the other files
/// still relink.
#[test]
fn files_not_relinked_go_back_to_linked_file_not_found() {
    let root = folder("back");
    let mut app = app_for(&root);
    let bytes = opened_without(&mut app, &root, &["a.png", "b.png", "c.png"]);
    write(&at(&root, "found/a.png"), &bytes[0]);
    write(&at(&root, "found/b.png"), b"not an image");
    // The open's notes are the ones the questions keep.
    app.link_asks.as_mut().unwrap().notes = vec!["a note".to_string()];
    drive::answer::<MissingLinksDialog>(&mut app, &[("searchFolder", json!(root.join("found")))], true);
    searched(&mut app);
    let f = found(&app);
    assert_eq!(f["notes"], json!(["a note"]));
    // b.png's only file is chosen, c.png has none.
    assert_eq!((pick(&f, "b.png"), pick(&f, "c.png")), (json!(at(&root, "found/b.png")), Value::Null));
    drive::answer::<FindLinksDialog>(&mut app, &[], true);
    assert!(app.ui.status.starts_with("Relinked 1 image; b.png couldn't be relinked: "), "{}", app.ui.status);
    let m = drive::fields::<MissingLinksDialog>(&app);
    let names: Vec<&str> = m["missing"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"b.png") && names.contains(&"c.png"), "{names:?}");
    assert_eq!(m["notes"], json!(["a note"]));
    drive::answer::<MissingLinksDialog>(&mut app, &[], false);
    assert_eq!(app.run("links.check", json!({})).unwrap()["missing"], 2);
    let opened = crate::io::opened_with_notes(1, "a note");
    assert_eq!((&app.ui.status, &app.ui.status_notes), (&opened, &Some((opened.clone(), vec!["a note".to_string()]))));
}

/// Relink, and Cancel, in the Find Linked Files dialog that Linked File Not Found's Find in Folder…
/// opened: the status bar shows the open's notes again and keeps them for its hover. Cancel of the
/// Links panel's Find in Folder… leaves the message as it is.
#[test]
fn the_last_find_in_folder_answer_keeps_the_open_notes() {
    let root = folder("notes");
    let mut app = app_for(&root);
    let bytes = opened_without(&mut app, &root, &["a.png"]);
    let place = at(&root, "found/a.png");
    write(&place, &bytes[0]);
    let missing = drive::fields::<MissingLinksDialog>(&app)["missing"].as_array().cloned().unwrap();
    let notes = vec!["a note".to_string(), "another note".to_string()];
    let opened = crate::io::opened_with_notes(2, "a note");
    drive::answer::<MissingLinksDialog>(&mut app, &[], false);
    let search = root.join("found").to_string_lossy().into_owned();

    // From the Links panel: Cancel leaves the message.
    app.run("ui.findLinksInFolder", json!({ "folder": search })).unwrap();
    app.status("Something else");
    drive::answer::<FindLinksDialog>(&mut app, &[], false);
    assert_eq!(app.ui.status, "Something else");

    // From Linked File Not Found.
    let from_missing = |app: &mut VectorcraftApp| {
        from_missing_links(app, missing.clone(), vec![], notes.clone(), &search).unwrap();
        app.status("Something else");
    };
    from_missing(&mut app);
    drive::answer::<FindLinksDialog>(&mut app, &[], false);
    assert!(!drive::is_open::<FindLinksDialog>(&app));
    assert_eq!((&app.ui.status, &app.ui.status_notes), (&opened, &Some((opened.clone(), notes.clone()))), "Cancel");
    from_missing(&mut app);
    drive::answer::<FindLinksDialog>(&mut app, &[("picks", json!([place]))], true);
    assert!(!drive::is_open::<FindLinksDialog>(&app) && !drive::is_open::<MissingLinksDialog>(&app));
    assert_eq!(app.run("links.check", json!({})).unwrap()["missing"], 0);
    assert_eq!((&app.ui.status, &app.ui.status_notes), (&opened, &Some((opened.clone(), notes.clone()))), "Relink");
}

/// A file is chosen when the choice is clear: the only same file, else the only file. A row the
/// user chose for keeps its choice as the search goes on.
#[test]
fn a_file_is_chosen_only_when_the_choice_is_clear() {
    let row = |p: &str| json!({"name": "a.png", "path": p, "ids": [1]});
    let mut d = state(json!({"missing": [row("/x/a.png"), row("/y/a.png"), row("/z/a.png")], "picks": [], "touched": []}));
    let file = |p: &str, m: &str| json!({"path": p, "match": m});
    let answer = |y: Vec<Value>| {
        json!({"state": "searching", "links": [
            {"path": "/x/a.png", "ids": [1], "files": [file("/f/a.png", "same"), file("/g/a.png", "name")]},
            {"path": "/y/a.png", "ids": [1], "files": y},
            {"path": "/z/a.png", "ids": [1], "files": [file("/f/a.png", "size"), file("/g/a.png", "name")]},
        ]})
    };
    update(&mut d, &answer(vec![file("/f/a.png", "name")]));
    assert_eq!(d["picks"], json!(["/f/a.png", "/f/a.png", null]));
    // Don't Relink chosen for the first row: the next answer leaves it, and the second row has
    // two files now.
    d.insert("picks".into(), json!([null, "/f/a.png", null]));
    d.insert("touched".into(), json!([0]));
    update(&mut d, &answer(vec![file("/f/a.png", "name"), file("/g/a.png", "name")]));
    assert_eq!(d["picks"], json!([null, null, null]));
}

/// A search of another folder starts the choices over: a file chosen from the last folder's
/// results isn't kept.
#[test]
fn another_folder_starts_the_choices_over() {
    let root = folder("again");
    let mut app = app_for(&root);
    opened_without(&mut app, &root, &["a.png"]);
    drive::answer::<MissingLinksDialog>(&mut app, &[], false);
    // Two files of the name and neither the same file: the search reads both folders.
    write(&at(&root, "one/a.png"), b"one file");
    write(&at(&root, "one/x/a.png"), b"another file");
    std::fs::create_dir_all(root.join("two")).unwrap();
    app.run("ui.findLinksInFolder", json!({"folder": root.join("one")})).unwrap();
    searched(&mut app);
    assert_eq!(found(&app)["found"][0]["files"].as_array().map(Vec::len), Some(2));
    // The second file chosen, then another folder searched, which has no file of the name.
    let fields = [("picks", json!([at(&root, "one/x/a.png")])), ("searchFolder", json!(root.join("two")))];
    drive::answer::<FindLinksDialog>(&mut app, &fields, true);
    searched(&mut app);
    let f = found(&app);
    assert_eq!((pick(&f, "a.png"), &f["touched"]), (Value::Null, &json!([])));
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
    drive::answer::<MissingLinksDialog>(&mut app, &[], false);
    app.run("ui.findLinksInFolder", json!({"folder": root.join("tree")})).unwrap();
    let id = app.ui.links_search.expect("the dialog's search");
    assert_eq!((drive::is_open::<FindLinksDialog>(&app), &found(&app)["back"]), (true, &json!(false)));
    drive::answer::<FindLinksDialog>(&mut app, &[], false);
    assert!(!drive::is_open::<FindLinksDialog>(&app) && app.ui.links_search.is_none());
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
    drive::answer::<MissingLinksDialog>(&mut app, &[], false);
    write(&at(&root, "found/a.png"), &bytes[0]);
    app.run("ui.findLinksInFolder", json!({"folder": root.join("found")})).unwrap();
    searched(&mut app);
    assert_eq!(copy_folder(&app), Some(at(&root, "doc").to_string_lossy().into_owned()));
    drive::answer::<FindLinksDialog>(&mut app, &[("copy", json!(true))], true);
    assert!(!drive::is_open::<FindLinksDialog>(&app));
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
    assert_eq!(copy_folder(&app), None);
}

/// Copy into the document's folder copies the file of an image in a symbol's art too, picked in the
/// Links panel by its id.
#[test]
fn copy_takes_an_image_in_a_symbol_into_the_documents_folder() {
    let root = folder("copy-symbol");
    let mut app = app_for(&root);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let orig = at(&root, "orig/a.png");
    write(&orig, &png(3));
    let placed = app.run("file.place", json!({"path": orig})).unwrap()["ids"][0].clone();
    app.run("symbol.new", json!({"name": "Mark", "ids": [placed]})).unwrap();
    let doc = at(&root, "doc/doc.vectorcraft");
    std::fs::create_dir_all(doc.parent().unwrap()).unwrap();
    app.run("document.save", json!({"path": doc})).unwrap();
    write(&at(&root, "found/a.png"), &png(3));
    let list = app.run("links.list", json!({})).unwrap();
    let row = list["links"].as_array().unwrap().iter().find(|r| r["owner"]["kind"] == "symbol").cloned().expect("the symbol's image");
    app.run("ui.findLinksInFolder", json!({"folder": root.join("found"), "ids": [row["id"]]})).unwrap();
    searched(&mut app);
    drive::answer::<FindLinksDialog>(&mut app, &[("copy", json!(true))], true);
    assert!(!drive::is_open::<FindLinksDialog>(&app));
    let copy = at(&root, "doc/a.png");
    assert_eq!(std::fs::read(&copy).unwrap(), png(3));
    let list = app.run("links.list", json!({})).unwrap();
    let row = list["links"].as_array().unwrap().iter().find(|r| r["owner"]["kind"] == "symbol").cloned().unwrap();
    assert_eq!(row["path"], json!(copy), "{list}");
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
    let place = at(&root, "found/badge.vectorcraft");
    write(&place, &std::fs::read(&badge).unwrap());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("file.place", json!({"path": badge, "link": true})).unwrap();
    let doc = at(&root, "doc/doc.vectorcraft");
    std::fs::create_dir_all(doc.parent().unwrap()).unwrap();
    app.run("document.save", json!({"path": doc})).unwrap();
    std::fs::remove_dir_all(root.join("orig")).unwrap();
    app.run("file.close", json!({})).unwrap();
    crate::io::open_path(&mut app, &doc.to_string_lossy()).unwrap();
    drive::answer::<MissingLinksDialog>(&mut app, &[], false);
    app.run("ui.findLinksInFolder", json!({"folder": root.join("found")})).unwrap();
    searched(&mut app);
    drive::answer::<FindLinksDialog>(&mut app, &[("copy", json!(true))], true);
    assert!(!drive::is_open::<FindLinksDialog>(&app));
    assert!(!at(&root, "doc/badge.vectorcraft").exists());
    assert_eq!(app.run("links.check", json!({})).unwrap()["links"][0]["path"], json!(place));
}

/// The Links panel's Find in Folder… (`ui.findLinksInFolder`): the selected linked files, else
/// the missing ones, in a folder.
#[test]
fn the_links_panel_looks_for_the_selected_or_the_missing_files() {
    let root = folder("panel");
    let mut app = app_for(&root);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let picked = root.join("found").to_string_lossy().into_owned();
    // Nothing to look for: no folder is asked for.
    assert_eq!(app.run("ui.findLinksInFolder", json!({"folder": picked})).err().as_deref(), Some("No linked files to look for"));
    assert!(!drive::is_open::<FindLinksDialog>(&app));
    let here = at(&root, "found/here.png");
    write(&here, &png(1));
    let id = app.run("file.place", json!({"path": here})).unwrap()["ids"][0].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("ui.findLinksInFolder", json!({"folder": picked})).unwrap();
    let f = found(&app);
    assert_eq!((f["missing"][0]["name"].as_str(), f["folder"].as_str()), (Some("here.png"), Some(picked.as_str())));
    searched(&mut app);
    assert_eq!(pick(&found(&app), "here.png"), json!(here));
}

/// A row whose path `links.check` left out (its file is outside the read roots) still shows the
/// files the search found for its objects.
#[test]
fn a_row_without_its_path_shows_the_files_found_for_its_objects() {
    let found = [json!({"name": "a.png", "path": "/elsewhere/a.png", "ids": [3], "files": [{"path": "/root/a.png"}]})];
    assert_eq!(files_for(&found, &json!({"name": "a.png", "ids": [3]})), [json!({"path": "/root/a.png"})]);
    assert!(files_for(&found, &json!({"name": "a.png", "ids": [4]})).is_empty());
}
