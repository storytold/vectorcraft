use std::path::Path;
use std::time::Instant;

use craft::ui::dialog::DialogHost;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::findfiles::Rules;

use super::*;
use crate::tests_folder::Folder;

fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
    app.dialogs().expect("a window host").with(f)
}

/// One frame's pump of the dialogs.
fn pump(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    crate::dialogs::host::pump(app, &ctx);
}

fn is_open<D: StackDialog>(app: &VectorcraftApp) -> bool {
    host(app, |h| h.is_open::<D>())
}

/// The Missing Fonts dialog's fields, while it is open.
fn fields(app: &VectorcraftApp) -> Option<Value> {
    host(app, |h| h.open_dialogs().into_iter().find(|d| d.id == dialog_id(KIND)).and_then(|d| d.fields))
}

/// The family of the first missing font the open dialog lists.
fn family(app: &VectorcraftApp) -> Option<String> {
    fields(app).and_then(|f| f["fonts"][0]["family"].as_str().map(str::to_string))
}

fn set_field(app: &VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().expect("a window context");
    host(app, |h| h.set_json_field(&ctx, dialog_id(KIND), field, value))
}

/// OK on the dialog (Add Fonts, or what its fields ask), then the frame's pump.
fn answer_ok(app: &mut VectorcraftApp) -> bool {
    let ctx = app.ui_ctx.clone().expect("a window context");
    let done = host(app, |h| h.confirm_by_id(&ctx, dialog_id(KIND)));
    pump(app);
    done
}

/// Close on the dialog, then the frame's pump.
fn answer_cancel(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    host(app, |h| h.cancel_by_id(&ctx, dialog_id(KIND)));
    pump(app);
}

/// Cancel the questions other than Missing Fonts (the linked files' question comes first).
fn cancel_others(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    host(app, |h| {
        for d in h.open_dialogs().into_iter().filter(|d| d.id != dialog_id(KIND)) {
            h.cancel_by_id(&ctx, d.id);
        }
    });
}

/// A fresh temporary folder (canonical, as a search reports it), removed when dropped.
fn folder(tag: &str) -> Folder {
    let dir = vectorcraft_testkit::temp_dir(&format!("missing-fonts-{tag}"));
    Folder::adopt(vectorcraft_engine::cmd::findfiles::plain(std::fs::canonicalize(&dir).unwrap()))
}

/// An app that reads files and whose searches may go anywhere in `root`, on two threads.
fn app_for(root: &Path) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.adopt_context(&crate::testing::ui_context());
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    app.session.search_rules = Some(Rules { user_content: vec![root.to_path_buf()], ..Rules::default() });
    app.session.search_threads = Some(2);
    app
}

fn png() -> Vec<u8> {
    let mut out = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(40, 30, image::Rgba([200, 0, 0, 255]))).into();
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
    // Opened afresh by `open`: an open file would only come to the front (#3455).
    app.run("file.close", json!({})).unwrap();
}

fn open(app: &mut VectorcraftApp, path: &Path) {
    crate::io::open_path(app, &path.to_string_lossy()).unwrap();
}

/// How long a test waits for the dialog's search to end. The wait only ends a test whose search
/// never ends. A machine busy with other builds can hold a walker for seconds. It is shorter than a
/// search's default `maxSeconds` of 60, which would end a search that does not end by itself.
const WAIT: Duration = Duration::from_secs(30);

#[test]
fn missing_fonts_are_asked_about_after_the_missing_links() {
    let root = folder("after-links");
    let mut app = app_for(&root);
    let doc = root.join("doc.vectorcraft");
    save_doc(&mut app, &doc, "No Such Font Family", true);
    open(&mut app, &doc);
    assert!(!is_open::<MissingFontsDialog>(&app), "the linked files are asked about first");
    cancel_others(&mut app);
    pump(&mut app);
    assert_eq!(family(&app).as_deref(), Some("No Such Font Family"), "Missing Fonts after the linked files");
    assert_eq!(fields(&app).unwrap()["fontsNextToDocument"], "", "no Fonts folder next to the document");
    assert!(!answer_ok(&mut app), "nothing to add");
    assert!(is_open::<MissingFontsDialog>(&app), "nothing to add: the dialog stays open");
    answer_cancel(&mut app);
    assert!(!is_open::<MissingFontsDialog>(&app));
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
    pump(&mut app);
    let next = fields(&app).expect("Missing Fonts at once: no links missing")["fontsNextToDocument"].as_str().unwrap().to_string();
    assert_eq!(next, root.join("pkg").join("Fonts").to_string_lossy());
    set_field(&app, "searchFolder", json!(next)).unwrap();
    assert!(answer_ok(&mut app));
    let f = fields(&app).expect("the dialog stays open while it searches");
    assert!(matches!(f["state"].as_str(), Some("searching" | "done")), "{f}");
    assert!(f.get("searchFolder").is_none(), "the folder is searched once");
    let until = Instant::now() + WAIT;
    while fields(&app).is_some_and(|f| f["state"] == "searching") && Instant::now() < until {
        pump(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    let f = fields(&app).unwrap();
    assert_eq!((f["state"].clone(), &f["found"][0]["files"]), (json!("done"), &json!([font])));
    assert_eq!(f["chosen"], json!([font]), "the font's file is chosen");
    // Adding needs VectorCraft's Fonts folder, which only the apps set.
    assert!(answer_ok(&mut app));
    assert!(app.ui.status.contains("isn't set here"), "{}", app.ui.status);
    assert!(is_open::<MissingFontsDialog>(&app), "a failed add keeps the dialog");
}

/// Find in Folder… closes the dialog and asks for the folder as the app does for Type › Find Font's
/// Find in Folder…; the dialog comes back searching the folder picked.
#[test]
fn find_in_folder_asks_for_the_folder_and_the_dialog_comes_back_searching_it() {
    let root = folder("pick");
    std::fs::create_dir_all(root.join("somewhere")).unwrap();
    let mut app = app_for(&root);
    let picks = crate::tests::picks::scripted(&mut app);
    let doc = root.join("doc.vectorcraft");
    save_doc(&mut app, &doc, "No Such Font Family", false);
    open(&mut app, &doc);
    pump(&mut app);
    let ctx = app.ui_ctx.clone().expect("a window context");
    let state = host(&app, |h| h.get::<MissingFontsDialog>().map(|d| d.state.clone())).expect("the dialog is open");
    answer_cancel(&mut app);
    assert!(!is_open::<MissingFontsDialog>(&app));
    settle(&mut app, Answer { act: Act::Pick, state });
    assert_eq!(picks.requests().last().map(|r| r.kind), Some(PickKind::Folder), "the app asks for a folder");
    assert!(!is_open::<MissingFontsDialog>(&app), "the dialog waits for the answer");
    picks.answer(&[&root.join("somewhere").to_string_lossy()]);
    crate::tests::picks::poll(&mut app, &ctx);
    pump(&mut app);
    let f = fields(&app).expect("the dialog comes back");
    assert_eq!(f["folder"], json!(root.join("somewhere").to_string_lossy()), "searching the folder picked: {f}");
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
    // Opened afresh: an open file would only come to the front (#5369).
    app.run("file.close", json!({})).unwrap();
    open(&mut app, &doc);
    pump(&mut app);
    assert!(is_open::<MissingFontsDialog>(&app));
    set_field(&app, "discard", json!(true)).unwrap();
    assert!(answer_ok(&mut app));
    assert!(is_open::<crate::find_font::FindFontDialog>(&app), "Find Font");
    let fonts = app.session.execute("text.fonts", &json!({})).unwrap();
    let selected =
        host(&app, |h| h.get::<crate::find_font::FindFontDialog>().and_then(|d| d.json_fields())).unwrap()["selected"].as_u64().unwrap() as usize;
    assert!(selected > 0 && fonts[selected]["family"] == "Zz Missing Family", "{selected} in {fonts}");
    // Find Font closed: no Missing Fonts dialog follows.
    let ctx = app.ui_ctx.clone().unwrap();
    host(&app, |h| h.cancel::<crate::find_font::FindFontDialog>(&ctx));
    pump(&mut app);
    assert!(!is_open::<MissingFontsDialog>(&app) && app.ui.pending_fonts.is_empty());
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
    cancel_others(&mut app);
    pump(&mut app);
    assert_eq!(family(&app).as_deref(), Some("Alpha Missing Family"));
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(&*b.to_string_lossy()));
    set_field(&app, "discard", json!(true)).unwrap();
    answer_ok(&mut app);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(&*a.to_string_lossy()));
    let fonts = app.session.execute("text.fonts", &json!({})).unwrap();
    let selected =
        host(&app, |h| h.get::<crate::find_font::FindFontDialog>().and_then(|d| d.json_fields())).unwrap()["selected"].as_u64().unwrap() as usize;
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
    app.run("ui.missingFontsDialog", json!({"folder": root.as_path()})).unwrap();
    let id = app.ui.dialog_search.expect("the dialog's search");
    answer_cancel(&mut app);
    assert!(!is_open::<MissingFontsDialog>(&app) && app.ui.dialog_search.is_none());
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
    pump(&mut app);
    assert_eq!(family(&app).as_deref(), Some("Alpha Missing Family"));
    // The second document's linked files are asked about first; then each document's fonts.
    open(&mut app, &b);
    cancel_others(&mut app);
    pump(&mut app);
    assert_eq!(family(&app).as_deref(), Some("Alpha Missing Family"), "the second waits its turn");
    answer_cancel(&mut app);
    assert_eq!(family(&app).as_deref(), Some("Beta Missing Family"));
    answer_cancel(&mut app);
    assert!(!is_open::<MissingFontsDialog>(&app) && app.ui.pending_fonts.is_empty());
    // A document closed before its turn is passed over.
    app.run("file.closeAll", json!({})).unwrap();
    open(&mut app, &a);
    open(&mut app, &b);
    let index = app.session.documents().iter().rposition(|d| d.path.as_deref() == Some(&*a.to_string_lossy())).unwrap();
    app.session.execute("file.close", &json!({"index": index})).unwrap();
    cancel_others(&mut app);
    pump(&mut app);
    assert_eq!(family(&app).as_deref(), Some("Beta Missing Family"));
}

/// Another dialog opened over the Missing Fonts dialog (Import PDF for a PDF opened after the
/// document) leaves the question on the stack under it, and it is the open one again once that
/// closes.
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
    open(&mut app, &doc);
    pump(&mut app);
    assert_eq!(family(&app).as_deref(), Some("Alpha Missing Family"));
    crate::io::open_bytes(&mut app, "c.pdf", &pdf(), None).unwrap();
    let open_keys = |app: &VectorcraftApp| host(app, |h| h.open_dialogs().into_iter().map(|d| (d.id, d.key)).collect::<Vec<_>>());
    let stack = open_keys(&app);
    assert_eq!(stack.len(), 2, "{stack:?}");
    assert_eq!(stack[0].1.as_deref(), Some(KIND), "Missing Fonts stays under Import PDF");
    // Import PDF closes: the question is the one open.
    let ctx = app.ui_ctx.clone().unwrap();
    host(&app, |h| h.cancel_by_id(&ctx, stack[1].0));
    pump(&mut app);
    assert_eq!(family(&app).as_deref(), Some("Alpha Missing Family"));
    // Closed (Cancel or Escape), it stays closed.
    answer_cancel(&mut app);
    assert!(!is_open::<MissingFontsDialog>(&app) && app.ui.pending_fonts.is_empty());
    app.run("ui.missingFontsDialog", json!({})).unwrap();
    answer_cancel(&mut app);
    assert!(!is_open::<MissingFontsDialog>(&app) && app.ui.pending_fonts.is_empty());
}

/// Type in a CSS generic family (`sans-serif`, the default font of SVG files from some editors)
/// opens without the question.
#[test]
fn generic_families_are_not_asked_about() {
    let root = folder("generic");
    let mut app = app_for(&root);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><text x="10" y="40" font-family="sans-serif">A</text></svg>"#;
    crate::io::open_bytes(&mut app, "generic.svg", svg.as_bytes(), None).unwrap();
    let listed = app.session.execute("text.fonts", &json!({})).unwrap();
    assert!(listed.as_array().unwrap().iter().any(|f| f["family"] == "sans-serif"), "{listed}");
    pump(&mut app);
    assert!(!app.any_dialog_open() && !is_open::<MissingFontsDialog>(&app) && app.ui.pending_fonts.is_empty());
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
    set_field(&app, "searchFolder", json!("relative/folder")).unwrap();
    assert!(answer_ok(&mut app));
    assert!(app.ui.status.contains("absolute"), "{}", app.ui.status);
    assert!(fields(&app).expect("the dialog is open again").get("searchFolder").is_none());
    assert!(!answer_ok(&mut app), "nothing chosen: OK refuses");
}

/// Opening a file that is already open only brings its tab to the front: Missing Fonts, closed
/// once, is not asked again (#5369).
#[test]
fn reopening_an_open_document_does_not_ask_about_its_fonts_again() {
    let root = folder("reopen");
    let mut app = app_for(&root);
    let doc = root.join("doc.vectorcraft");
    save_doc(&mut app, &doc, "No Such Font Family", false);
    open(&mut app, &doc);
    pump(&mut app);
    assert!(is_open::<MissingFontsDialog>(&app));
    answer_cancel(&mut app);
    app.run("file.new", json!({})).unwrap();
    open(&mut app, &doc);
    pump(&mut app);
    assert_eq!((app.session.documents().len(), app.session.active_index()), (2, Some(0)));
    assert!(!is_open::<MissingFontsDialog>(&app));
}

/// A placed document that Links › Edit Original opens asks about its fonts, as File › Open does.
#[test]
fn edit_original_asks_about_the_placed_documents_fonts() {
    let root = folder("edit-original");
    let mut app = app_for(&root);
    let badge = root.join("badge.vectorcraft");
    save_doc(&mut app, &badge, "Badge Missing Family", false);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("file.place", json!({"path": badge.to_string_lossy(), "link": true})).unwrap();
    let tabs = app.session.documents().len();
    app.run("links.editOriginal", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), tabs + 1);
    pump(&mut app);
    assert_eq!(family(&app).as_deref(), Some("Badge Missing Family"), "Missing Fonts for the document opened");
}

/// A search leaves out the fonts that became available: the files it found go with their fonts
/// by name, and a file the user unchecked stays unchecked as the search goes on.
#[test]
fn files_go_with_their_fonts_by_name() {
    let (a, b) = (json!({"family": "A Sans", "style": "Regular"}), json!({"family": "B Sans", "style": "Bold"}));
    let mut state = State::new(1, vec![a.clone(), b.clone()], String::new(), false);
    update(&mut state, &json!({"state": "searching", "fonts": [{"family": "B Sans", "style": "Bold", "files": ["/f/b.otf"]}]}));
    assert!(files_for(&state.found, &a).is_empty());
    assert_eq!(files_for(&state.found, &b), [json!("/f/b.otf")]);
    assert_eq!(state.chosen, [json!("/f/b.otf")]);
    state.chosen.clear();
    update(&mut state, &json!({"state": "done", "fonts": [{"family": "B Sans", "style": "Bold", "files": ["/f/b.otf", "/g/b.otf"]}]}));
    assert_eq!((state.name(), state.chosen.len()), ("done", 0), "an unchecked file stays unchecked");
}

#[test]
fn the_web_says_how_many_fonts_are_missing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Gone", "font": "No Such Font Family"})).unwrap();
    app.ui.status.clear();
    after_open_for(&mut app, &json!({}), true);
    assert!(!app.ui.status.is_empty(), "the status line says fonts are missing");
    assert!(app.ui.pending_fonts.is_empty());
    // The open's notes stay on show.
    app.ui.status = "Opened with 1 note: fonts".into();
    after_open_for(&mut app, &json!({"warnings": ["fonts"]}), true);
    assert_eq!(app.ui.status, "Opened with 1 note: fonts");
}
