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

/// How long a test waits for the dialog's search to end. The wait only ends a test whose search
/// never ends. A machine busy with other builds can hold a walker for seconds. It is shorter than a
/// search's default `maxSeconds` of 60, which would end a search that does not end by itself.
const WAIT: Duration = Duration::from_secs(30);

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
    for label in
        ["Missing Fonts", "No Such Font Family Regular", "These fonts aren't available. Their text shows in another font.", "Find Fonts", "Close"]
    {
        assert!(text.contains(label), "{label} in {text}");
    }
    assert!(!text.contains("Search Fonts Folder"), "no Fonts folder next to the document");
    assert!(!text.contains("Add Fonts") && !text.contains("Copy only fonts"), "no file found yet: {text}");
    assert_eq!(super::super::confirm(&mut app).err().as_deref(), Some("choose the font files to add"));
    assert_eq!(kind(&app), Some(KIND), "nothing to add: the dialog stays open");
    super::super::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
}

#[test]
fn the_documents_fonts_folder_is_offered_and_searched_off_the_ui_thread() {
    let root = folder("package");
    let font = root.join("pkg").join("Fonts").join("sub").join("Findme.otf");
    std::fs::create_dir_all(font.parent().unwrap()).unwrap();
    std::fs::write(&font, vectorcraft_testkit::fonts::renamed("Findme Sans 3")).unwrap();
    let mut app = app_for(&root);
    let doc = root.join("pkg/doc.vectorcraft");
    save_doc(&mut app, &doc, "Findme Sans 3", false);
    open(&mut app, &doc);
    let next = app.ui.dialog.as_ref().filter(|d| d.kind == KIND).expect("Missing Fonts at once: no links missing").str("fontsNextToDocument");
    assert_eq!(next, root.join("pkg").join("Fonts").to_string_lossy());
    assert!(frame(&mut app).contains("Search Fonts Folder"));
    app.ui.dialog.as_mut().unwrap().fields.insert("searchFolder".into(), json!(next));
    let r = super::super::confirm(&mut app).unwrap();
    assert!(matches!(r["state"].as_str(), Some("searching" | "done")), "{r}");
    assert_eq!(kind(&app), Some(KIND), "the dialog stays open while it searches");
    let until = Instant::now() + WAIT;
    while app.ui.dialog.as_ref().is_some_and(|d| d.str("state") == "searching") && Instant::now() < until {
        frame(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("state"), &d.fields["found"][0]["files"]), ("done".to_string(), &json!([font])));
    assert_eq!(d.fields["chosen"], json!([font]), "the font's file is chosen");
    let text = frame(&mut app);
    for label in ["Findme.otf", "Copy only fonts you own or are licensed to install.", "Add Fonts"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    // Adding needs VectorCraft's Fonts folder, which only the apps set.
    let e = super::super::confirm(&mut app).unwrap_err();
    assert!(e.contains("isn't set here"), "{e}");
    assert_eq!(kind(&app), Some(KIND));
}

/// Find Fonts opens Type › Find Font on the first missing font, to replace it, and the Missing
/// Fonts dialog doesn't come back for that open.
#[test]
fn find_fonts_opens_find_font_on_the_missing_font() {
    let root = folder("find-fonts");
    let mut app = app_for(&root);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Here", "font": "Source Sans 3"})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 80, "text": "Gone", "font": "Zz Missing Family"})).unwrap();
    let doc = root.join("doc.vectorcraft");
    app.run("document.save", json!({"path": doc.to_string_lossy()})).unwrap();
    open(&mut app, &doc);
    assert_eq!(kind(&app), Some(KIND));
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    super::super::confirm(&mut app).unwrap();
    let d = app.ui.dialog.as_ref().expect("Find Font");
    assert_eq!(d.kind, "findFont");
    let fonts = app.session.execute("text.fonts", &json!({})).unwrap();
    let selected = d.fields["selected"].as_u64().unwrap() as usize;
    assert!(selected > 0 && fonts[selected]["family"] == "Zz Missing Family", "{selected} in {fonts}");
    // Find Font closed: no Missing Fonts dialog follows.
    app.ui.dialog = None;
    frame(&mut app);
    assert!(app.ui.dialog.is_none() && app.ui.pending_fonts.is_empty());
}

/// The dialog of a document in another tab: Find Fonts makes that document the active one, so
/// Find Font lists and replaces its fonts.
#[test]
fn find_fonts_works_on_the_dialog_s_own_document() {
    let root = folder("find-fonts-tab");
    let mut app = app_for(&root);
    let (a, b) = (root.join("a.vectorcraft"), root.join("b.vectorcraft"));
    save_doc(&mut app, &a, "Alpha Missing Family", false);
    save_doc(&mut app, &b, "Beta Missing Family", true);
    open(&mut app, &a);
    open(&mut app, &b);
    // B's linked files are asked about first; then A's fonts, with B the active tab.
    super::super::cancel(&mut app);
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields["fonts"][0]["family"], "Alpha Missing Family");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(&*b.to_string_lossy()));
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(&*a.to_string_lossy()));
    let fonts = app.session.execute("text.fonts", &json!({})).unwrap();
    let selected = app.ui.dialog.as_ref().unwrap().fields["selected"].as_u64().unwrap() as usize;
    assert_eq!(fonts[selected]["family"], "Alpha Missing Family", "{fonts}");
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
    let files = vec![opened("a.vectorcraft", Some(doc.to_string_lossy().into_owned()), std::fs::read(&doc).unwrap()), opened("b.pdf", None, pdf())];
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
