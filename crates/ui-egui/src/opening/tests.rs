use vectorcraft_engine::Session;

use super::*;
use crate::Services;
use crate::tests::picks::{cancel_dialog, close_shown, draw_dialogs, open_dialogs, show_question};

/// #9286: both real open consumers apply the same preference/explicit-option precedence and bind
/// the carried program to rendered text, rather than merely storing the requested option.
#[test]
fn synchronous_and_background_pdf_open_honor_embedded_font_preferences_and_overrides() {
    use std::fmt::Write as _;
    use vectorcraft_doc::{FILE_FONT_MIME, NodeKind};
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};
    use vectorcraft_text::{FileFontInfo, FileGlyph, build_file_font};

    let mut glyph = FileGlyph { ch: 'x', outline: Default::default(), advance: 600.0 };
    glyph.outline.move_to((0.0, 0.0));
    glyph.outline.line_to((500.0, 0.0));
    glyph.outline.line_to((500.0, 700.0));
    glyph.outline.line_to((0.0, 700.0));
    glyph.outline.close_path();
    let info = FileFontInfo { family: "VectorCraftUIOpenPreference".into(), style: "Regular".into(), ..Default::default() };
    let font_bytes = build_file_font(&info, &[glyph]).unwrap();
    let mut hex = String::with_capacity(font_bytes.len() * 2);
    for b in font_bytes {
        write!(hex, "{b:02x}").unwrap();
    }
    let n = first_extra(1);
    let page = PdfPage { resources: format!("/Font << /F1 {n} 0 R >>"), ..PdfPage::new(100.0, 100.0, "BT /F1 24 Tf 10 40 Td (x) Tj ET") };
    let font = format!(
        "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+VectorCraftUIOpenPreference /FirstChar 120 /LastChar 120 /Widths [600] /Encoding /WinAnsiEncoding /FontDescriptor {} 0 R >>",
        n + 1
    );
    let descriptor = format!(
        "<< /Type /FontDescriptor /FontName /ABCDEF+VectorCraftUIOpenPreference /Flags 34 /FontBBox [0 -200 500 900] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 {} 0 R >>",
        n + 2
    );
    let program = format!("<< /Length {} /Filter /ASCIIHexDecode >>\nstream\n{hex}>\nendstream", hex.len() + 1);
    let bytes = pdf_with(&[page], &[&font, &descriptor, &program], None);
    for background in [false, true] {
        for (preference, explicit, expected) in [(false, None, false), (true, None, true), (false, Some(true), true), (true, Some(false), false)] {
            let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()));
            app.session.prefs.pdf_import_embedded_fonts = preference;
            let mut params = json!({"textAs": "text"});
            if let Some(value) = explicit {
                params["embeddedFonts"] = json!(value);
            }
            if background {
                start_with(&mut app, "embedded-preference.pdf", bytes.clone(), params);
                open_ready(&mut app);
            } else {
                crate::io::open_document(&mut app, "embedded-preference.pdf", &bytes, None, &params).unwrap();
            }
            let doc = &app.session.doc().unwrap().doc;
            let db = vectorcraft_text::FontDb::global();
            let mut text_count = 0;
            let mut carries_font = false;
            doc.walk(|node| {
                if let NodeKind::Text(text) = &node.kind {
                    assert_eq!(text.plain_text(), "x", "imported text content");
                    text_count += 1;
                    carries_font |= vectorcraft_text::layout(db, text).glyphs.iter().any(|glyph| db.is_file_font(glyph.font_id));
                }
            });
            assert_eq!(text_count, 1, "text survives: background={background}, preference={preference}, explicit={explicit:?}");
            assert_eq!(carries_font, expected, "rendered font: background={background}, preference={preference}, explicit={explicit:?}");
            assert_eq!(
                doc.images.values().any(|blob| blob.mime == FILE_FONT_MIME),
                expected,
                "owned font program: background={background}, preference={preference}, explicit={explicit:?}"
            );
        }
    }
}

#[test]
fn long_operation_busy_while_a_file_opens_or_a_background_save_runs() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(!long_operation_busy(&app));
    start(&mut app, "/art/Wait.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(10.0, 10.0), false));
    assert!(long_operation_busy(&app));
    open_ready(&mut app);
    assert!(!long_operation_busy(&app));
    let (release, held) = std::sync::mpsc::channel::<()>();
    let services = Services { write_shared: Some(std::sync::Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    app = VectorcraftApp::new(Session::new(), services);
    crate::background::run(
        &mut app,
        true,
        "Saving test.vectorcraft".into(),
        None,
        move |_| {
            let _ = held.recv();
            Ok(json!({}))
        },
        |_, r| r,
    )
    .unwrap();
    assert!(long_operation_busy(&app));
    let _ = release.send(());
    crate::background::wait_all(&mut app);
    assert!(!long_operation_busy(&app));
}

/// Wait until every file being opened has been read, keeping each result for [`poll`].
fn wait_read(app: &mut VectorcraftApp) {
    for p in &mut app.opening.pending {
        let (tx, done) = std::sync::mpsc::channel();
        // A reader that stopped without a result leaves the new channel closed too.
        if let Ok(r) = p.done.recv() {
            let _ = tx.send(r);
        }
        p.done = done;
    }
}

/// Wait until every file being opened has been read, then open the files that may open now.
/// Whether the missing-fonts question is open once a frame has drawn the dialogs.
fn asks_fonts(app: &mut VectorcraftApp) -> bool {
    let ctx = app.ui_ctx.clone().expect("a window context");
    draw_dialogs(app, &ctx);
    open_dialogs(app).iter().any(|d| d.key.as_deref() == Some(crate::dialogs::missing_fonts::KIND))
}

fn open_ready(app: &mut VectorcraftApp) {
    wait_read(app);
    poll(app);
}

fn install_importer(app: &mut VectorcraftApp, src: &str) {
    let wasm = wat::parse_str(src).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
}

/// A file for the importers below, in a folder of its own.
fn importer_file(tag: &str) -> (crate::tests_folder::Folder, String) {
    let dir = crate::tests_folder::Folder::new(&format!("ui-opening-{tag}"));
    let file = dir.file("a.tob", b"not json");
    (dir, file)
}

/// Open With off the UI thread: nothing opens until the importer has read the file with the
/// parameters it was given; then the document opens and joins Open Recent, and a failure is
/// reported in a dialog.
#[test]
fn open_with_runs_the_importer_off_the_ui_thread() {
    use vectorcraft_plugins::wat::{self as tpl, Given};
    let rect = r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":true,"anchors":[{"p":[10,20]},{"p":[110,20]},{"p":[110,70]},{"p":[10,70]}]}]}}]}"#;
    let (_dir, file) = importer_file("with");
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()));
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    install_importer(&mut app, &tpl::when_given(&tpl::importer("test.opening.with", r#"["tob"]"#, params), Given::Params, r#""density":7"#, rect));

    let open = |density: i64| json!({"importer": "plugin.test.opening.with", "path": file, "params": {"density": density}});
    let r = crate::io::open_with_on(&mut app, &open(7), true).unwrap();
    assert_eq!(r["background"], true, "{r}");
    assert!(app.session.documents().is_empty(), "nothing opens before the importer has read the file");
    assert!(app.ui.recent_files.is_empty());
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.ui.recent_files, std::slice::from_ref(&file));

    // Density 4 gives the file back, which isn't JSON.
    app.ui.recent_files.clear();
    crate::io::open_with_on(&mut app, &open(4), true).unwrap();
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert!(!app.ui.status.is_empty(), "the failure reaches the status bar");
    assert!(crate::dialogs::confirm::open_message(&app).is_some(), "the failure is reported in a dialog");
    assert!(app.ui.recent_files.is_empty());
    app.run("plugin.remove", json!({"id": "test.opening.with"})).unwrap();
}

/// Cancel stops an importer that never returns, on the thread reading the file.
#[test]
fn cancel_stops_a_running_importer() {
    use vectorcraft_plugins::wat as tpl;
    let (_dir, file) = importer_file("cancel");
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    // The importer never returns: it must not run on the calling thread.
    crate::tests_background::threaded(&mut app);
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    install_importer(&mut app, &tpl::module(&tpl::importer("test.opening.spins", r#"["tob"]"#, "{}"), "(loop $l (br $l)) (i64.const 0)", "", 1));
    crate::io::open_with_on(&mut app, &json!({"importer": "plugin.test.opening.spins", "path": file}), true).unwrap();
    let done = std::mem::replace(&mut app.opening.pending[0].done, std::sync::mpsc::channel().1);
    assert_eq!(cancel_all(&mut app)["cancelled"], 1);
    let r = done.recv().unwrap();
    assert!(r.as_ref().err().is_some_and(|e| e.contains("cancelled")), "{:?}", r.err());
    assert!(app.session.documents().is_empty());
    app.run("plugin.remove", json!({"id": "test.opening.spins"})).unwrap();
}

/// #1694: a file that finishes opening while a dialog is up waits for it to close, so the
/// dialog's document stays the active one.
#[test]
fn a_file_ready_while_a_dialog_is_open_waits_for_it_to_close() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    let (first, before) = (app.session.doc().unwrap().uid, app.session.documents().len());
    let question = show_question(&mut app);
    start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false));
    open_ready(&mut app);
    assert_eq!((app.session.documents().len(), app.session.doc().unwrap().uid), (before, first), "still the dialog's document");
    assert_eq!(app.opening.pending.len(), 1);
    close_shown(&mut app, question);
    poll(&mut app);
    assert!(app.opening.pending.is_empty());
    assert_eq!(app.session.documents().len(), before + 1);
    assert_eq!(app.session.doc().unwrap().path.as_deref(), Some("/art/Second.vectorcraft"));
}

/// A file that finishes opening while an object is being dragged waits for the drag to end, so
/// the drag is not canceled.
#[test]
fn a_file_ready_during_a_drag_waits_for_it_to_end() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let square = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    let view = vectorcraft_engine::ViewInfo::default();
    for (kind, x) in [(PointerKind::Down, 20.0), (PointerKind::Drag, 40.0), (PointerKind::Drag, 60.0)] {
        app.session.pointer(&PointerEvent::new(kind, x, 20.0), view).unwrap();
    }
    start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false));
    open_ready(&mut app);
    assert!(app.session.documents().len() == 1 && app.session.in_interaction(), "the file waits for the drag");
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 60.0, 20.0), view).unwrap();
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 2);
    let moved = app.session.documents()[0].doc.node(vectorcraft_doc::NodeId(square)).and_then(|n| n.geometric_bounds());
    assert_eq!(moved.map(|b| (b.x0, b.y0)), Some((50.0, 10.0)), "the drag was not canceled");
}

/// A file that finishes opening while the place cursor is being dragged, a drag the tool keeps
/// to itself, waits for the drag to end: the drag still places its file.
#[test]
fn a_file_ready_during_a_place_cursor_drag_waits_for_it_to_end() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbImage::new(8, 8)).into();
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
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    app.run("file.place.queue", json!({"files": [{"name": "a.png", "dataBase64": vectorcraft_format::base64_encode(&png)}]})).unwrap();
    let view = vectorcraft_engine::ViewInfo::default();
    for (kind, x) in [(PointerKind::Down, 20.0), (PointerKind::Drag, 60.0)] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, x, 20.0), view);
    }
    start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false));
    open_ready(&mut app);
    assert!(app.session.documents().len() == 1 && app.session.tool_busy(), "the file waits for the drag");
    crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Up, 60.0, 20.0), view);
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 2);
    let first = &app.session.documents()[0].doc;
    let names: Vec<_> = first.layers.iter().flat_map(|l| l.children().into_iter().flatten()).filter_map(|n| n.name.clone()).collect();
    assert_eq!(names, ["a.png"], "the drag placed its file");
}

/// A file that finishes opening while a preview runs waits for the preview to end, so the
/// preview is not canceled: a slider's preview with the tool idle, a slider's preview while the
/// Dimension tool waits after its first point, and the Dimension tool's line following the
/// pointer after its second point.
#[test]
fn a_file_ready_during_a_preview_waits_for_it_to_end() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let view = vectorcraft_engine::ViewInfo::default();
    let click = |app: &mut VectorcraftApp, x: f64| {
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(app, &PointerEvent::new(kind, x, 20.0), view);
        }
    };
    for case in ["slider", "slider between clicks", "line"] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        if case != "slider" {
            app.select_tool("dimension");
            click(&mut app, 20.0);
        }
        if case == "line" {
            click(&mut app, 80.0);
            crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Move, 50.0, 60.0), view);
        } else {
            app.session.begin_interaction("Preview").unwrap();
        }
        assert_eq!(app.session.tool_busy(), case != "slider", "{case}");
        start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false));
        open_ready(&mut app);
        assert!(app.session.documents().len() == 1 && app.session.in_interaction(), "the file waits for the preview: {case}");
        if case == "line" {
            click(&mut app, 50.0);
        } else {
            app.session.cancel_interaction().unwrap();
        }
        poll(&mut app);
        assert_eq!(app.session.documents().len(), 2, "{case}");
    }
}

/// A file that finishes opening while a tool waits between its clicks with no preview open (the
/// Dimension tool after its first point, then after its second before the pointer moves) opens
/// without waiting for the next click, and the tool starts afresh in it, as when the user
/// switches documents.
#[test]
fn a_file_ready_between_a_tools_clicks_opens() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("dimension");
    let view = vectorcraft_engine::ViewInfo::default();
    let click = |app: &mut VectorcraftApp, x: f64| {
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(app, &PointerEvent::new(kind, x, 20.0), view);
        }
    };
    click(&mut app, 20.0);
    assert!(app.session.tool_busy() && !app.session.in_interaction(), "the first point waits for the second");
    start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(400.0, 300.0), false));
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), 2, "the file opens after one click");
    assert!(!app.session.tool_busy(), "the tool starts afresh");
    click(&mut app, 20.0);
    click(&mut app, 80.0);
    assert!(app.session.tool_busy() && !app.session.in_interaction(), "the second point waits for the line's place");
    start(&mut app, "/art/Third.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false));
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), 3, "the file opens after two clicks");
    assert!(!app.session.tool_busy() && !app.session.in_interaction(), "the tool starts afresh");
    let second = &app.session.documents()[1].doc;
    assert!(second.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())), "the unfinished dimension is not made");
}

#[test]
fn a_file_read_off_the_ui_thread_opens_as_one_read_at_once() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let mut d = vectorcraft_doc::Document::new(120.0, 80.0);
    d.title = "Poster".into();
    let bytes = vectorcraft_format::save(&d, false);
    let before = app.session.documents().len();
    let r = start(&mut app, "/art/Poster.vectorcraft", bytes);
    assert_eq!(r["background"], true);
    assert_eq!(app.ui.status, "Opening Poster.vectorcraft…");
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), before + 1);
    assert!(!app.ui.status.starts_with("Opening"), "{}", app.ui.status);
    let st = app.session.doc().unwrap();
    assert_eq!((st.path.as_deref(), st.doc.artboards[0].rect.width()), (Some("/art/Poster.vectorcraft"), 120.0));
    assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("/art/Poster.vectorcraft"));
    // A file that can't be read: reported, nothing opened, not a recent file.
    start(&mut app, "/art/Broken.vectorcraft", b"{\"format\":\"vectorcraft\"".to_vec());
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), before + 1);
    assert!(app.ui.status.starts_with("Couldn't open Broken.vectorcraft"), "{}", app.ui.status);
    assert_ne!(app.ui.recent_files.first().map(String::as_str), Some("/art/Broken.vectorcraft"));
}

/// A file read off the UI thread opens as the document read at once. Its linked TIFF, BMP and JPEG
/// files are found beside it after its folder moved, a missing one shows its preview, a changed
/// one follows Update Links, and its Auto Size area type in two columns is fitted. Neither open
/// leaves a step to undo, and both ask the same question.
#[test]
fn a_file_read_off_the_ui_thread_has_its_links_and_type_as_one_read_at_once() {
    use craft_image_codecs::Format::{Bmp, Jpeg, Png, Tiff};
    let dir = crate::tests_folder::Folder::new("opening-links");
    let (before, after) = (dir.join("before"), dir.join("after"));
    std::fs::create_dir_all(&before).unwrap();
    let file = |folder: &std::path::Path, name: &str| folder.join(name).to_string_lossy().into_owned();
    let picture = |format: craft_image_codecs::Format, rgb: [u8; 3]| {
        let mut out = vec![];
        {
            let __dyn: image::DynamicImage = (image::RgbImage::from_pixel(60, 40, image::Rgb(rgb))).into();
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
            craft_image_codecs::encode(&__cimg, format, &craft_image_codecs::EncodeOptions::default()).map(|__b| out.extend_from_slice(&__b))
        }
        .unwrap();
        out
    };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()));
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    for (name, format) in [("a.tif", Tiff), ("b.bmp", Bmp), ("c.jpg", Jpeg), ("gone.png", Png), ("changed.png", Png)] {
        std::fs::write(file(&before, name), picture(format, [200, 40, 40])).unwrap();
        app.session.execute("file.place", &json!({"path": file(&before, name), "at": [200, 150]})).unwrap();
    }
    let area = json!({"width": 150, "height": 20, "fit": "autoHeight"});
    let t = app.session.execute("text.create", &json!({"x": 20, "y": 40, "text": "Type in two columns. ".repeat(12), "area": area})).unwrap();
    app.session.execute("text.areaOptions", &json!({"ids": [t["id"]], "columns": 2})).unwrap();
    app.session.execute("document.save", &json!({"path": file(&before, "poster.vectorcraft")})).unwrap();
    std::fs::remove_file(file(&before, "gone.png")).unwrap();
    let changed = file(&before, "changed.png");
    let stamp = std::fs::metadata(&changed).unwrap().modified().unwrap();
    std::fs::write(&changed, picture(Png, [40, 40, 200])).unwrap();
    // Link stamps use milliseconds: the new file is dated 2 s later.
    let later = std::fs::FileTimes::new().set_modified(stamp + std::time::Duration::from_secs(2));
    std::fs::OpenOptions::new().write(true).open(&changed).unwrap().set_times(later).unwrap();
    // The folder moves: the linked files are found beside the document.
    std::fs::rename(&before, &after).unwrap();
    let path = file(&after, "poster.vectorcraft");
    let bytes = std::fs::read(&path).unwrap();
    let asked = |app: &VectorcraftApp| open_dialogs(app).pop().map(|d| (d.key, d.fields));
    for update in ["askWhenModified", "automatically"] {
        app.session.prefs.update_links = update.into();
        let r = crate::io::open_bytes(&mut app, &path, &bytes, Some(path.clone())).unwrap();
        let rows = if update == "automatically" { &r["updatedLinks"] } else { &r["modifiedLinks"] };
        assert_eq!(rows[0]["name"], "changed.png", "{update}: {r}");
        let (at_once, question) = (app.session.doc().unwrap().doc.clone(), asked(&app));
        assert!(question.as_ref().and_then(|(_, f)| f.as_ref()).is_some_and(|f| f["missing"][0]["name"] == "gone.png"), "{update}: {question:?}");
        let open = open_dialogs(&app).pop().map(|d| d.id).expect("the links question is open");
        cancel_dialog(&mut app, open);
        start(&mut app, &path, bytes.clone());
        open_ready(&mut app);
        let st = app.session.doc().unwrap();
        assert!(*st.doc == *at_once, "{update}: the same document");
        assert!(st.history.undo.is_empty() && !st.is_dirty(), "{update}");
        assert_eq!(asked(&app), question, "{update}");
    }
}

/// A file opened under automation roots is installed under them too: the open keeps the scope it
/// started in, not the desktop's access of the frame it finishes in.
#[test]
fn a_file_opened_under_automation_roots_finishes_under_them() {
    use vectorcraft_engine::file_access;
    let dir = crate::tests_folder::Folder::new("opening-scope");
    let roots = file_access::AutomationRoots::new(&[dir.as_path()], &[]).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    file_access::confine(roots.as_ref(), || {
        start(&mut app, "/art/Scoped.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(10.0, 10.0), false))
    });
    let scope = app.opening.pending[0].scope.clone();
    assert!(scope.run(file_access::current).is_some(), "the roots go with the open");
}

/// A file asked for again while it is still being read is not read a second time (#3455).
#[test]
fn a_file_asked_for_again_while_it_is_read_opens_once() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let bytes = vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false);
    start(&mut app, "/art/Poster.vectorcraft", bytes);
    assert_eq!(crate::io::open_file(&mut app, "/art/Poster.vectorcraft", true).unwrap(), Value::Null);
    assert_eq!(app.opening.pending.len(), 1);
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), 1);
}

/// A document asked for again while a dialog was open, closed meanwhile, opens again once the
/// dialog closes and asks about its fonts; a file read meanwhile waits for that question too, so
/// the dialog acts on its own document (#5369).
#[test]
fn a_queued_reopen_that_asks_holds_back_files_ready_to_open() {
    let dir = crate::tests_folder::Folder::new("opening-queued");
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let (doc, other) = (dir.join("doc.vectorcraft"), dir.join("other.vectorcraft"));
    let (doc, other) = (doc.to_string_lossy().into_owned(), other.to_string_lossy().into_owned());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Type", "font": "No Such Font Family"})).unwrap();
    app.run("document.save", json!({"path": doc})).unwrap();
    app.run("file.new", json!({})).unwrap();
    app.run("document.save", json!({"path": other})).unwrap();
    app.run("file.new", json!({})).unwrap();
    let question = show_question(&mut app);
    crate::io::open_reporting(&mut app, &doc).unwrap();
    crate::io::open_reporting(&mut app, &other).unwrap();
    assert_eq!(app.opening.activate.len(), 2);
    app.session.execute("file.close", &json!({"index": 1})).unwrap();
    app.session.execute("file.close", &json!({"index": 0})).unwrap();
    start(&mut app, "/art/Other.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false));
    // The other file is read; it waits for the dialog meanwhile.
    open_ready(&mut app);
    assert_eq!(app.opening.pending.len(), 1);
    close_shown(&mut app, question);
    poll(&mut app);
    assert!(asks_fonts(&mut app));
    assert_eq!(app.session.doc().unwrap().path.as_deref(), Some(&*doc), "the dialog's document stays active");
    assert_eq!(app.opening.activate.first().map(|(p, _)| p), Some(&other), "the later activation waits");
    assert_eq!(app.opening.pending.len(), 1, "the other file waits");
}

/// Of two files read off the UI thread and ready in the same frame, the first that opens and asks
/// about its fonts holds back the second, so the question acts on its own document (#5369).
#[test]
fn a_file_that_opens_and_asks_holds_back_the_next_one_ready() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Type", "font": "No Such Font Family"})).unwrap();
    let asks = vectorcraft_format::save(&app.session.doc().unwrap().doc, false);
    app.session.execute("file.close", &json!({})).unwrap();
    let question = show_question(&mut app);
    start(&mut app, "/art/Asks.vectorcraft", asks);
    start(&mut app, "/art/Plain.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false));
    // Both files are read; they wait for the dialog meanwhile.
    open_ready(&mut app);
    assert_eq!(app.opening.pending.len(), 2);
    close_shown(&mut app, question);
    poll(&mut app);
    assert!(asks_fonts(&mut app));
    assert_eq!(app.session.doc().unwrap().path.as_deref(), Some("/art/Asks.vectorcraft"), "the dialog's document stays active");
    assert_eq!(app.opening.pending.len(), 1, "the next file waits");
}

/// Makes every file being read old enough for its window.
fn age(app: &mut VectorcraftApp) {
    for p in &mut app.opening.pending {
        p.started -= 2.0 * crate::export_progress::DELAY;
    }
}

/// Draws the progress window → the texts it drew.
fn progress(app: &mut VectorcraftApp, ctx: &egui::Context) -> String {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::export_progress::show(app, ui));
    out.textures_delta.clear();
    let mut texts = String::new();
    for shape in out.shapes {
        if let egui::epaint::Shape::Text(t) = shape.shape {
            texts.push_str(t.galley.text());
            texts.push('\n');
        }
    }
    texts
}

/// A document's bytes, `title` naming it.
fn bytes(title: &str) -> Vec<u8> {
    let mut d = vectorcraft_doc::Document::new(120.0, 80.0);
    d.title = title.into();
    vectorcraft_format::save(&d, false)
}

/// #5930: a file that takes a while to open shows a progress window with its name and a moving
/// bar, after [`crate::export_progress::DELAY`] so a quick open doesn't flash it; a second file
/// says it waits.
#[test]
fn a_file_that_takes_a_while_to_open_shows_a_progress_window() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    // The app's clock stands still: a read is only old enough for its window after `age`.
    app.now_override = Some(0.0);
    let ctx = crate::testing::ui_context();
    assert_eq!(progress(&mut app, &ctx), "", "nothing opening");
    start(&mut app, "/art/Big.vectorcraft", bytes("Big"));
    assert_eq!(progress(&mut app, &ctx), "", "not at once");
    age(&mut app);
    progress(&mut app, &ctx); // a new window measures itself unseen first
    let texts = progress(&mut app, &ctx);
    for want in ["Opening Big.vectorcraft…", "Reading the file…", "Cancel", "Continue in Background"] {
        assert!(texts.contains(want), "{want} in {texts}");
    }
    assert!(!texts.contains("waiting"), "{texts}");
    start(&mut app, "/art/Next.vectorcraft", bytes("Next"));
    let texts = progress(&mut app, &ctx);
    assert!(texts.contains("Opening Big.vectorcraft…") && texts.contains("waiting"), "{texts}");
    // Continue in Background: the window closes, the files still open.
    let before = app.session.documents().len();
    assert!(crate::menus::enabled(&app, "file.openInBackground"));
    assert_eq!(app.run("file.openInBackground", json!({})).unwrap()["hidden"], 2);
    assert_eq!(progress(&mut app, &ctx), "");
    assert!(!crate::menus::enabled(&app, "file.openInBackground"));
    assert!(long_operation_busy(&app), "the status bar still shows them");
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), before + 2);
}

/// #5930: the progress window's Cancel (`file.cancelOpen`): the file doesn't open, even once it
/// has been read, and the status bar no longer says it is opening.
#[test]
fn cancelling_an_open_opens_nothing() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(!crate::menus::enabled(&app, "file.cancelOpen"));
    let before = app.session.documents().len();
    start(&mut app, "/art/Big.vectorcraft", bytes("Big"));
    assert!(crate::menus::enabled(&app, "file.cancelOpen"));
    assert_eq!(app.run("file.cancelOpen", json!({})).unwrap()["cancelled"], 1);
    assert!(app.opening.pending.is_empty() && !long_operation_busy(&app));
    assert!(!app.ui.status.starts_with("Opening"), "{}", app.ui.status);
    std::thread::sleep(std::time::Duration::from_millis(50));
    poll(&mut app);
    assert_eq!(app.session.documents().len(), before);
    assert_ne!(app.ui.recent_files.first().map(String::as_str), Some("/art/Big.vectorcraft"));
}
