//! File → Place in the app: the Place dialog, files dropped on the window, the loaded place
//! cursor and the Control bar's image details.

use std::sync::Arc;

use egui::{Pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_testkit::pdf::{PdfPage, pdf};

use super::picks::{answer_dialog, open_dialogs, poll, scripted};
use crate::VectorcraftApp;
use crate::place::{DropAt, DropSpot, DropTarget};
use crate::tests_folder::Folder;

/// The keys of the import dialogs the Place flow opens.
const TEXT_IMPORT: &str = "textImport";
const DXF_IMPORT: &str = "dxfImport";
const PDF_IMPORT: &str = "importPdf";

/// A `w`×`h` red PNG declaring `ppi`.
fn png(w: u32, h: u32, ppi: f64) -> Vec<u8> {
    let mut out = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]))).into();
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
    vectorcraft_engine::cmd::fileio::ppi::with_png_resolution(&out, (ppi, ppi))
}

/// An app reading files from disk, as the desktop host's does, in a window with no document.
fn reading() -> VectorcraftApp {
    let services = crate::Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
    crate::testing::windowed(VectorcraftApp::new(Session::new(), services))
}

/// [`reading`] with a 400×300 document.
fn app() -> VectorcraftApp {
    let mut app = reading();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// The context of `app`'s window, which every frame of a test runs on.
fn window(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the app has a window")
}

/// File → Place… with `paths` chosen in the file picker.
fn place_picked_all(app: &mut VectorcraftApp, ctx: &egui::Context, paths: &[&str]) {
    let picker = scripted(app);
    app.run("file.place", json!({})).unwrap();
    picker.answer(paths);
    poll(app, ctx);
}

/// File → Place… with `path` chosen in the file picker.
fn place_picked(app: &mut VectorcraftApp, ctx: &egui::Context, path: &str) {
    place_picked_all(app, ctx, &[path]);
}

/// The top open dialog's key, if a dialog is open.
fn dialog_kind(app: &VectorcraftApp) -> Option<String> {
    open_dialogs(app).pop().and_then(|d| d.key)
}

/// The values of the top open dialog.
fn fields(app: &VectorcraftApp) -> Value {
    open_dialogs(app).pop().and_then(|d| d.fields).expect("a dialog with values is open")
}

/// Whether boolean `key` of the top dialog is on.
fn flag(app: &VectorcraftApp, key: &str) -> bool {
    fields(app)[key].as_bool().unwrap_or(false)
}

/// Sets `values` on the top dialog and presses OK.
fn answer(app: &mut VectorcraftApp, values: &[(&str, Value)]) {
    let id = open_dialogs(app).pop().expect("a dialog is open").id;
    answer_dialog(app, id, values, true);
}

/// Cancels every open dialog.
fn dismiss(app: &mut VectorcraftApp) {
    // PLAN: assumed `ui_ctx: Option<egui::Context>` (lib.rs:203) and `cancel_by_id` of the host
    let (Some(ctx), Some(dialogs)) = (app.ui_ctx.clone(), app.dialogs()) else { return };
    for d in open_dialogs(app).into_iter().rev() {
        dialogs.with(|host| host.cancel_by_id(&ctx, d.id));
    }
}

#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
struct MockDrop {
    path: std::path::PathBuf,
    bytes: Result<Vec<u8>, String>,
}

impl egui::DroppedFile for MockDrop {
    fn path(&self) -> &std::path::Path {
        &self.path
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        self.bytes.clone()
    }
}

/// The pointer over the middle of the canvas `rect`, as the events of a frame.
fn over(rect: egui::Rect) -> Vec<egui::Event> {
    vec![egui::Event::PointerMoved(rect.center())]
}

/// No pointer over the window, as the events of a frame: a drop then goes where no canvas is.
fn off() -> Vec<egui::Event> {
    vec![egui::Event::PointerGone]
}

/// One headless frame of the whole window (800×600) with `events` and `dropped` files. A drop is
/// three passes: the files hover with the pointer where `events` put it, they drop, and the
/// target under the pointer takes them in the pass the drop asks for.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, dropped: &[&str], shift: bool) {
    let files = dropped.iter().map(|p| Arc::new(Dropped(p.into())) as egui::DroppedFileHandle).collect();
    frame_files(app, ctx, events, files, shift);
}

fn frame_files(app: &mut VectorcraftApp, ctx: &egui::Context, mut events: Vec<egui::Event>, dropped: Vec<egui::DroppedFileHandle>, shift: bool) {
    let mods = egui::Event::ModifiersChanged(egui::Modifiers { shift, ..Default::default() });
    events.push(mods.clone());
    if dropped.is_empty() {
        pass(app, ctx, raw_input(events, vec![], dropped));
        return;
    }
    let pointer = events.iter().filter(|e| matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerGone)).cloned().collect();
    let hovered = dropped
        .iter()
        .map(|f| egui::HoveredFile { path: Some(f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()), ..Default::default() })
        .collect();
    pass(app, ctx, raw_input(pointer, hovered, vec![]));
    pass(app, ctx, raw_input(events, vec![], dropped));
    pass(app, ctx, raw_input(vec![mods], vec![], vec![]));
}

fn raw_input(events: Vec<egui::Event>, hovered_files: Vec<egui::HoveredFile>, dropped_files: Vec<egui::DroppedFileHandle>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events,
        hovered_files,
        dropped_files,
        ..Default::default()
    }
}

fn pass(app: &mut VectorcraftApp, ctx: &egui::Context, raw: egui::RawInput) {
    crate::testing::app_frame(app, ctx, raw);
}

fn selected_image(app: &VectorcraftApp) -> vectorcraft_doc::Node {
    let st = app.session.active().unwrap();
    let n = st.doc.node(st.selection.objects[0]).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Image(_)), "{:?}", n.kind);
    n
}

/// A tiny SVG document.
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="40"><path d="M5 5 L45 5 L30 30 Z"/></svg>"#;

/// Two frames: fonts, then the canvas lays out → the canvas.
fn laid_out(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::Rect {
    frame(app, ctx, vec![], &[], false);
    frame(app, ctx, vec![], &[], false);
    // PLAN: assumed `CanvasView::rect` is an `egui::Rect` (`ScreenRect`)
    app.canvas_view().map(|v| v.rect).expect("the canvas is laid out")
}

/// Where the platform tells where files were dropped, documents, pictures and text go as in the
/// reference app: placed at the pointer on the canvas, opened off it (the tab bar).
#[test]
fn a_drop_with_a_position_places_on_the_canvas_and_opens_off_it() {
    let mut app = app();
    let ctx = window(&app);
    let rect = laid_out(&mut app, &ctx);
    let pos = rect.center() + vec2(60.0, -40.0);
    let want = app.canvas_view().unwrap().to_doc(pos);
    let uid = app.session.active().unwrap().uid;
    for name in ["a.png", "a.svg", "a.ai", "a.vectorcraft"] {
        assert_eq!(
            app.drop_target(name, DropSpot::Canvas(want), true),
            DropTarget::Place(DropAt { doc: uid, reverts: 0, at: want, embed: true }),
            "{name}"
        );
        assert_eq!(app.drop_target(name, DropSpot::Elsewhere, false), DropTarget::Open, "{name} on the tab bar");
    }
}

/// A drop that carries no position (a floating window's): a dropped document opens as a tab of
/// its own, a picture or text is placed in the middle of the view.
#[test]
fn a_drop_without_a_position_opens_documents_and_places_pictures() {
    let mut app = app();
    let ctx = window(&app);
    laid_out(&mut app, &ctx);
    let center = app.view().unwrap().center;
    let uid = app.session.active().unwrap().uid;
    for name in ["a.svg", "a.svgz", "a.pdf", "a.ai", "a.eps", "a.vectorcraft", "a.drawcraft", "a.vctemplate", "a.dxf", "a.emf"] {
        assert_eq!(app.drop_target(name, DropSpot::Unknown, false), DropTarget::Open, "{name}");
    }
    for name in ["a.png", "a.jpg", "a.tiff", "a.webp", "a.txt"] {
        assert_eq!(
            app.drop_target(name, DropSpot::Unknown, false),
            DropTarget::Place(DropAt { doc: uid, reverts: 0, at: center, embed: false }),
            "{name}"
        );
    }
}

/// A dropped file of each kind File → Open opens in a place of its own (a library, a script, a
/// shortcut set) opens there wherever it is dropped: on the canvas, off it, and where the platform
/// doesn't say.
#[test]
fn a_dropped_file_of_each_library_kind_opens_in_its_destination() {
    let set = "/Menus {\n /Acme\\ Undo { /Modifiers 11 /Key 902 }\n}\n";
    let library = json!({"name": "Dropped", "colors": [{"name": "Red", "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}}]}).to_string();
    type Opened = fn(&VectorcraftApp, &str) -> bool;
    let kinds: [(&str, Vec<u8>, Opened); 4] = [
        ("dropped.gpl", b"GIMP Palette\nName: Dropped\n#\n255   0   0\tRed\n".to_vec(), |app, path| {
            crate::panels::library_panel::last_open(&app.ui).is_some_and(|o| o.id == format!("loaded/{path}"))
        }),
        ("dropped.jsx", b"#target acme\napp.executeMenuCommand('acmecopyall');\n".to_vec(), |app, _| {
            app.dialogs().is_some_and(|d| d.with(|h| h.get::<craft::ui::patterns::form_dialogs::HostedReport>().is_some()))
        }),
        ("dropped.vclibrary", library.into_bytes(), |app, _| app.ui.status == "Imported library Dropped"),
        // PLAN: `DialogState::snapshot(..).review` (shortcuts-w8) is not asserted; the dialog open is
        ("dropped.kys", set.as_bytes().to_vec(), |app, _| app.shortcuts_dialog.is_some()),
    ];
    let dir = Folder::new("ui-place-opens");
    for (name, bytes, opened) in kinds {
        let path = dir.file(name, &bytes);
        for route in 0..4 {
            let mut app = app();
            let ctx = window(&app);
            let rect = laid_out(&mut app, &ctx);
            match route {
                0 => frame(&mut app, &ctx, over(rect), &[&path], false),
                1 => frame(&mut app, &ctx, off(), &[&path], false),
                _ => {
                    let spot = if route == 2 { DropSpot::Unknown } else { DropSpot::Canvas(app.canvas_view().unwrap().to_doc(rect.center())) };
                    let target = app.drop_target(name, spot, false);
                    assert_eq!(target, DropTarget::Open, "{name}");
                    crate::place::drop_files(&mut app, vec![(target, (name.into(), Some(path.clone()), vec![]))]);
                }
            }
            assert!(opened(&app, &path), "{name}, route {route}: {}", app.ui.status);
            assert_eq!((app.session.documents().len(), images(&app, 0)), (1, 0), "{name}: neither opened as a document nor placed");
        }
    }
    let app = app();
    for name in [
        "a.vcswatches",
        "a.gpl",
        "a.ase",
        "a.acb",
        "a.vcstyles",
        "a.vclibrary",
        "a.vcflattener",
        "a.vcpdfpresets",
        "a.vcprintpresets",
        "a.vcperspective",
        "a.vcexportpresets",
        "a.wasm",
        "a.jsx",
        "a.jsxinc",
        "a.jsxbin",
    ] {
        for spot in [DropSpot::Unknown, DropSpot::Elsewhere, DropSpot::Canvas(vectorcraft_geom::Point::new(1.0, 1.0))] {
            assert_eq!(app.drop_target(name, spot, true), DropTarget::Open, "{name}");
        }
    }
}

#[test]
fn a_dropped_png_is_placed_where_it_was_dropped() {
    let mut app = app();
    let ctx = window(&app);
    let rect = laid_out(&mut app, &ctx);
    let dir = Folder::new("ui-place-png");
    let path = dir.file("drop.png", &png(20, 10, 72.0));
    let pos = rect.center() + vec2(60.0, -40.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos)], &[&path], false);
    let n = selected_image(&app);
    let want = app.canvas_view().unwrap().to_doc(pos);
    let c = n.geometric_bounds().unwrap().center();
    assert!((c.x - want.x).abs() < 1e-6 && (c.y - want.y).abs() < 1e-6, "{c:?} vs {want:?}");
    assert!(matches!(&n.kind, NodeKind::Image(im) if im.link.as_ref().map(|l| l.path.as_str()) == Some(path.as_str())), "linked by default");
    assert_eq!(app.session.documents().len(), 1, "placed, not opened");
    // Shift embeds.
    frame(&mut app, &ctx, vec![], &[&path], true);
    assert!(matches!(&selected_image(&app).kind, NodeKind::Image(im) if im.link.is_none()));
}

/// A document dropped where the platform gives no position opens as a new tab, not into the open
/// document; the pictures dropped with it are placed in the document that was open, before it
/// opens.
#[test]
fn a_dropped_document_opens_as_a_tab_of_its_own() {
    let mut app = app();
    let ctx = window(&app);
    laid_out(&mut app, &ctx);
    let dir = Folder::new("ui-place-tab");
    let svg = dir.file("dropped.svg", SVG.as_bytes());
    let pic = dir.file("with-it.png", &png(8, 8, 72.0));
    let files = [("dropped.svg", &svg), ("with-it.png", &pic)]
        .map(|(name, path)| (app.drop_target(name, DropSpot::Unknown, false), (name.to_string(), Some(path.clone()), vec![])));
    crate::place::drop_files(&mut app, files.to_vec());
    assert_eq!(app.session.documents().len(), 2, "opened, not placed");
    assert_eq!(app.session.active_index(), Some(1), "the document opened is shown");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(svg.as_str()));
    assert_eq!((images(&app, 0), images(&app, 1)), (1, 0), "the picture is placed in the document that was open");
    assert_eq!(app.ui.recent_files.first(), Some(&svg));
}

#[test]
fn a_native_svg_drop_uses_file_open_even_with_a_path_only_handle() {
    let dir = Folder::new("ui-place-path-only");
    let path = dir.file("path-only.svg", SVG.as_bytes());
    let mut dropped = app();
    let ctx = window(&dropped);
    laid_out(&mut dropped, &ctx);
    let handle = Arc::new(MockDrop { path: path.clone().into(), bytes: Err("only a path".into()) }) as egui::DroppedFileHandle;
    frame_files(&mut dropped, &ctx, vec![], vec![handle], false);

    let mut opened = app();
    opened.run("file.open", json!({ "path": path })).unwrap();
    let anchors = |app: &VectorcraftApp| {
        let mut count = 0;
        app.session.active().unwrap().doc.walk(|n| {
            if let NodeKind::Path { path, .. } = &n.kind {
                count += path.anchors().count();
            }
        });
        count
    };
    assert_eq!(anchors(&dropped), anchors(&opened), "same editable SVG anchors as File > Open");
    assert!(anchors(&dropped) >= 3, "the imported path has editable anchors");
    assert_eq!(dropped.session.active().unwrap().path, opened.session.active().unwrap().path);
    assert_eq!(dropped.session.active().unwrap().selection.objects.len(), opened.session.active().unwrap().selection.objects.len());
    assert_eq!(dropped.ui.recent_files, opened.ui.recent_files);
}

#[test]
fn an_svg_drop_without_a_path_is_detected_from_its_bytes() {
    let mut app = app();
    let ctx = window(&app);
    let rect = laid_out(&mut app, &ctx);
    let svg = Arc::new(MockDrop { path: Default::default(), bytes: Ok(SVG.as_bytes().to_vec()) }) as egui::DroppedFileHandle;
    frame_files(&mut app, &ctx, vec![], vec![svg], false);
    assert_eq!(app.session.documents().len(), 2, "SVG bytes open as a document, not placed art");
    assert!(app.session.active().unwrap().path.is_none(), "no filesystem path was supplied");

    frame(&mut app, &ctx, vec![], &[], false);
    let image = Arc::new(MockDrop { path: Default::default(), bytes: Ok(png(6, 6, 72.0)) }) as egui::DroppedFileHandle;
    frame_files(&mut app, &ctx, over(rect), vec![image], false);
    assert_eq!(app.session.documents().len(), 2, "an image is placed, not opened");
    assert_eq!(images(&app, 1), 1);
}

#[test]
fn multiple_svg_drops_open_individually() {
    let mut app = app();
    let ctx = window(&app);
    laid_out(&mut app, &ctx);
    let dir = Folder::new("ui-place-several-svg");
    let a = dir.file("first.svg", SVG.as_bytes());
    let b = dir.file("second.svg", SVG.as_bytes());
    frame(&mut app, &ctx, vec![], &[&a, &b], false);
    assert_eq!(app.session.documents().len(), 3);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(b.as_str()));
    assert_eq!(&app.ui.recent_files[..2], &[b, a]);
}

#[test]
fn unsupported_and_unreadable_drops_leave_the_document_open() {
    let mut app = app();
    let ctx = window(&app);
    let rect = laid_out(&mut app, &ctx);
    let dir = Folder::new("ui-place-unreadable");
    // Over the canvas the first is a place; the SVGs are dropped off it, so they open.
    let invalid = dir.file("not-art.xyz", b"not an image or document");
    frame(&mut app, &ctx, over(rect), &[&invalid], false);
    assert!(app.ui.status.contains("Couldn't place not-art.xyz"), "{}", app.ui.status);

    let invalid_svg = dir.file("broken.svg", b"<svg");
    frame(&mut app, &ctx, off(), &[&invalid_svg], false);
    assert!(app.ui.status.contains("Couldn't open broken.svg"), "{}", app.ui.status);

    let missing = dir.file("unreadable.svg", SVG.as_bytes());
    std::fs::remove_file(&missing).unwrap();
    frame(&mut app, &ctx, vec![], &[&missing], false);
    assert!(app.ui.status.contains("Couldn't open unreadable.svg"), "{}", app.ui.status);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.ui.recent_files.is_empty(), "failed drops don't enter Open Recent");
}

#[test]
fn files_dropped_with_no_document_open_open_and_are_recent() {
    let mut app = reading();
    let ctx = window(&app);
    let dir = Folder::new("ui-place-no-document");
    let path = dir.file("open-me.png", &png(8, 8, 72.0));
    frame(&mut app, &ctx, vec![], &[], false);
    frame(&mut app, &ctx, vec![], &[&path], false);
    assert_eq!(app.session.documents().len(), 1, "no document: the drop opens");
    assert_eq!(app.ui.recent_files.first(), Some(&path));
}

/// The images in open document `i`.
fn images(app: &VectorcraftApp, i: usize) -> usize {
    let mut n = 0;
    app.session.documents()[i].doc.walk(|node| n += usize::from(matches!(node.kind, NodeKind::Image(_))));
    n
}

/// The web reads dropped files asynchronously (#359): a file that arrives after another document
/// became active still lands where it was dropped, and is dropped itself when that document closed.
#[test]
fn a_file_read_after_another_document_became_active_lands_where_it_was_dropped() {
    let mut app = app();
    let inbox = crate::place::PlaceInbox::default();
    app.services.place_inbox = Some(inbox.clone());
    let ctx = window(&app);
    let rect = laid_out(&mut app, &ctx);
    let want = app.canvas_view().unwrap().to_doc(rect.center() + vec2(60.0, -40.0));
    let drop_on_active = |app: &VectorcraftApp| match app.drop_target("late.png", DropSpot::Canvas(want), false) {
        DropTarget::Place(d) => d,
        t => panic!("{t:?}"),
    };
    let arrive = |name: &str, drop| inbox.lock().unwrap().push(crate::place::PlaceArrival { name: name.into(), bytes: png(20, 10, 72.0), drop });
    // Dropped on A; another document opens while the file is read.
    let on_a = drop_on_active(&app);
    app.run("file.new", json!({"width": 600, "height": 500})).unwrap();
    assert_eq!(app.session.active_index(), Some(1));
    arrive("late.png", on_a);
    frame(&mut app, &ctx, vec![], &[], false);
    assert_eq!(app.session.active_index(), Some(0), "the document it was dropped on is active again");
    assert_eq!((images(&app, 0), images(&app, 1)), (1, 0));
    let c = selected_image(&app).geometric_bounds().unwrap().center();
    assert!((c.x - want.x).abs() < 1e-6 && (c.y - want.y).abs() < 1e-6, "where it was dropped: {c:?} vs {want:?}");
    // Dropped on B, which closes before the file arrives: nothing is placed.
    app.run("document.activate", json!({"index": 1})).unwrap();
    let on_b = drop_on_active(&app);
    app.run("file.close", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    arrive("orphan.png", on_b);
    frame(&mut app, &ctx, vec![], &[], false);
    assert_eq!(images(&app, 0), 1, "not placed in the document left open");
    assert!(app.ui.status.contains("orphan.png") && app.ui.status.contains("closed"), "{}", app.ui.status);
}

#[test]
fn the_place_dialog_lists_the_files_and_loads_the_cursor_with_several() {
    let mut app = app();
    let ctx = window(&app);
    let dir = Folder::new("ui-place-dialog");
    let a = dir.file("a.png", &png(300, 150, 300.0));
    let b = dir.file("b.png", &png(10, 10, 72.0));
    app.run("file.place", json!({"paths": [a, b]})).unwrap();
    assert!(flag(&app, "link") && !flag(&app, "__replace"), "Link on; Replace needs one file and one selected object");
    assert_eq!(fields(&app)["__info"][0], "300 × 150 px, 300 ppi, RGB (72 pt × 36 pt)");
    answer(&mut app, &[]);
    assert!(dialog_kind(&app).is_none());
    assert_eq!(app.session.tool_id(), "place");
    assert_eq!(app.session.tool_options()["count"], 2);
    // The cursor carries the thumbnails of the files.
    frame(&mut app, &ctx, vec![], &[], false);
    frame(&mut app, &ctx, vec![], &[], false);
    let thumbs = app.place.thumbnails();
    assert!(thumbs.contains(&"a.png") && thumbs.contains(&"b.png"), "{thumbs:?}");
}

#[test]
fn one_file_places_centred_in_the_view_and_replace_swaps_the_selection() {
    let mut app = app();
    let ctx = window(&app);
    app.view_mut().unwrap().center = vectorcraft_geom::Point::new(120.0, 80.0);
    let dir = Folder::new("ui-place-replace");
    place_picked(&mut app, &ctx, &dir.file("one.png", &png(300, 150, 300.0)));
    assert!(flag(&app, "link") && !flag(&app, "__replace"), "Link on; Replace needs one selected object");
    assert_eq!(fields(&app)["__info"][0], "300 × 150 px, 300 ppi, RGB (72 pt × 36 pt)");
    answer(&mut app, &[]);
    assert!(dialog_kind(&app).is_none());
    let first = selected_image(&app);
    assert_eq!(first.geometric_bounds().unwrap().center(), vectorcraft_geom::Point::new(120.0, 80.0), "centred in the view");
    // With one object selected, Replace applies.
    place_picked(&mut app, &ctx, &dir.file("two.png", &png(40, 40, 144.0)));
    assert!(flag(&app, "__replace"));
    answer(&mut app, &[("replace", json!(true)), ("link", json!(false))]);
    let second = selected_image(&app);
    assert_eq!(second.name.as_deref(), Some("two.png"));
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 1, "replaced");
    assert!(!app.ui.place_link, "Link is remembered");
}

#[test]
fn the_control_bar_shows_the_image_file_link_colour_mode_and_ppi() {
    use crate::tests_removeanchors::{control_frame, has};
    let mut app = app();
    let ctx = window(&app);
    let dir = Folder::new("ui-place-control-bar");
    let path = dir.file("photo.png", &png(30, 30, 300.0));
    app.run("file.place", json!({"path": path})).unwrap();
    let info = crate::place::selected_image_info(&mut app).cloned().expect("one image is selected");
    assert!(info["link"].as_str().is_some(), "linked");
    let (name, summary) = crate::place::image_summary(&info);
    assert_eq!(name, "photo.png");
    assert!(summary.ends_with("300"), "{summary}");
    assert!(has(&control_frame(&mut app, &ctx, vec![]), "photo.png"), "the bar names the file");
    let v: Value = app.run("file.place", json!({"path": path, "link": false})).unwrap();
    assert_eq!(v["linked"], false);
    let info = crate::place::selected_image_info(&mut app).cloned().expect("one image is selected");
    assert!(info["link"].as_str().is_none(), "embedded");
}

/// The Control bar's label is read from the first selected object each frame, so it follows an
/// edit, Undo, a switch to another document and back, and a selection change.
#[test]
fn the_control_bar_label_follows_edits_undo_documents_and_selection() {
    let mut app = app();
    let dir = Folder::new("ui-place-label");
    let path = dir.file("label.png", &png(30, 30, 72.0));
    app.run("file.place", json!({"path": path})).unwrap();
    let text = |app: &mut VectorcraftApp| crate::tests_labels::painted_text(app, crate::chrome::control_bar);
    assert!(text(&mut app).contains("Linked File"));
    app.run("links.embed", json!({})).unwrap();
    let embedded = text(&mut app);
    assert!(embedded.contains("Embedded") && !embedded.contains("Linked File"), "{embedded}");
    app.run("edit.undo", json!({})).unwrap();
    assert!(text(&mut app).contains("Linked File"));
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    assert!(!text(&mut app).contains("Linked File"), "the new document has nothing selected");
    app.session.set_active(0);
    assert!(text(&mut app).contains("Linked File"));
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    let rect = text(&mut app);
    assert!(!rect.contains("Linked File") && !rect.contains("Embedded"), "{rect}");
}

/// Where the Control bar draws `label`; menu items are drawn after the space a check mark takes.
fn label_at(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) -> egui::Pos2 {
    let texts = crate::tests_removeanchors::control_frame(app, ctx, vec![]);
    texts.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
}

/// Click the Control bar's `label`.
fn click_label(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let p = label_at(app, ctx, label);
    crate::tests_removeanchors::click_control(app, ctx, p);
}

/// The Control bar's file name opens the link menu: Replace… puts the picked file in the image's
/// place (an embedded image becomes linked) with the new file's own aspect ratio, fitted inside the
/// old bounds; Placement Options… opens its dialog.
#[test]
fn the_control_bars_file_name_relinks_the_image() {
    use crate::tests_removeanchors::click_control;
    let item = label_at;
    let mut app = app();
    let data = vectorcraft_format::base64_encode(&png(30, 30, 72.0));
    app.run("file.place", json!({"name": "red.png", "dataBase64": data, "link": false})).unwrap();
    let before = selected_image(&app).geometric_bounds().unwrap();
    let ctx = window(&app);
    click_label(&mut app, &ctx, "red.png");
    for s in ["Replace…", "Go To Link", "Edit Original", "Update Link", "Placement Options…"] {
        item(&mut app, &ctx, s);
    }
    let dir = Folder::new("ui-place-relink");
    let other = dir.file("blue.png", &png(60, 30, 72.0));
    let picker = scripted(&mut app);
    let p = item(&mut app, &ctx, "Replace…");
    click_control(&mut app, &ctx, p);
    picker.answer(&[&other]);
    poll(&mut app, &ctx);
    let image = selected_image(&app);
    let NodeKind::Image(im) = &image.kind else { panic!("an image") };
    assert!(im.link.as_ref().is_some_and(|l| l.name() == "blue.png"), "relinked to the picked file");
    let after = image.geometric_bounds().unwrap();
    assert!((after.width() - before.width()).abs() < 1e-6 && (after.height() - before.height() / 2.0).abs() < 1e-6, "2:1, fitted: {after:?}");
    assert!((after.center() - before.center()).hypot() < 1e-6, "centred where the old image was");
    click_label(&mut app, &ctx, "blue.png");
    let p = item(&mut app, &ctx, "Placement Options…");
    click_control(&mut app, &ctx, p);
    poll(&mut app, &ctx);
    assert!(dialog_kind(&app).is_some(), "Placement Options opens");
}

/// The centre of the Control bar's control whose accessible name starts with `name`, read from the accessibility tree
/// of one Control bar frame.
fn control_named(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) -> Pos2 {
    ctx.enable_accesskit();
    let screen = egui::Rect::from_min_size(Pos2::ZERO, crate::tests_removeanchors::CONTROL_ROOM);
    let raw = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
    let out = crate::testing::part_frame(app, ctx, raw, crate::chrome::control_bar);
    out.platform_output
        .accesskit_update
        .iter()
        .flat_map(|u| u.nodes.iter())
        .find_map(|(_, n)| {
            let b = n.bounds().filter(|_| n.label().is_some_and(|l| l.starts_with(name)))?;
            Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
        })
        .unwrap_or_else(|| panic!("no control named {name}"))
}

#[test]
fn the_control_bar_and_properties_trace_a_selected_image() {
    use crate::panels::image_trace::selected_trace;
    let selected_preset = |app: &VectorcraftApp| selected_trace(app).map(|t| t.0);
    use crate::tests_removeanchors::{at, click_control, control_frame, has, properties_frame};
    let mut app = app();
    let data = vectorcraft_format::base64_encode(&png(30, 30, 72.0));
    app.run("file.place", json!({"name": "red.png", "dataBase64": data, "link": false})).unwrap();
    let ctx = window(&app);
    let bar = control_frame(&mut app, &ctx, vec![]);
    for s in ["Embedded", "Unembed…", "Image Trace", "Mask", "Crop Image", "Opacity:"] {
        assert!(has(&bar, s), "{s}");
    }
    assert!(!has(&bar, "Stroke:"), "an image has no Fill and Stroke in the Control bar");
    let props = properties_frame(&mut app, 260.0);
    for s in ["Unembed…", "Image Trace", "Mask", "Crop Image"] {
        assert!(has(&props, s), "Properties: {s}");
    }
    // The button's arrow lists the presets; choosing one traces the image with it.
    let arrow = control_named(&mut app, &ctx, "Tracing Presets");
    click_control(&mut app, &ctx, arrow);
    click_label(&mut app, &ctx, "6 Colors");
    assert_eq!(selected_preset(&app).as_deref(), Some("6 Colors"));
    // The button itself traces with the Default preset.
    app.run("edit.undo", json!({})).unwrap();
    click_label(&mut app, &ctx, "Image Trace");
    assert_eq!(selected_preset(&app).as_deref(), Some("Default"));
    // An Image Trace object: its preset (another traces it again), view, the panel and Expand.
    let bar = control_frame(&mut app, &ctx, vec![]);
    // A field's colon is drawn as a text run of its own, after its label.
    for s in ["Image Tracing", "Preset", "Default", "View", "Tracing Result", "Expand"] {
        assert!(has(&bar, s), "{s}");
    }
    let props = properties_frame(&mut app, 260.0);
    assert!(has(&props, "Release") && has(&props, "View"));
    // Another view redraws it without tracing again.
    let traced = app.session.active().unwrap().selection.objects.clone();
    click_control(&mut app, &ctx, at(&bar, "Tracing Result"));
    click_label(&mut app, &ctx, "Outlines with Source Image");
    assert_eq!(selected_trace(&app).map(|t| t.1), Some(vectorcraft_doc::TraceView::OutlinesWithSource));
    assert_eq!(app.session.active().unwrap().selection.objects, traced, "the same object");
    let bar = control_frame(&mut app, &ctx, vec![]);
    click_control(&mut app, &ctx, at(&bar, "Default"));
    click_label(&mut app, &ctx, "3 Colors");
    assert_eq!(selected_preset(&app).as_deref(), Some("3 Colors"));
    click_label(&mut app, &ctx, "Expand");
    let st = app.session.active().unwrap();
    let g = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(selected_trace(&app).is_none() && g.children().unwrap().iter().all(|c| !matches!(c.kind, NodeKind::Image(_))), "expanded");
}

#[test]
fn the_control_bars_mask_clips_the_image_and_selects_the_clipping_path() {
    let mut app = app();
    let data = vectorcraft_format::base64_encode(&png(30, 30, 72.0));
    let img = app.run("file.place", json!({"name": "red.png", "dataBase64": data, "link": false})).unwrap()["ids"][0].as_u64().unwrap();
    let ctx = window(&app);
    click_label(&mut app, &ctx, "Mask");
    let st = app.session.active().unwrap();
    let group = &st.doc.layers[0].children().unwrap()[0];
    assert!(matches!(group.kind, NodeKind::Group { clip: true, .. }));
    let ids: Vec<u64> = group.children().unwrap().iter().map(|c| c.id.0).collect();
    assert_eq!((ids[1], st.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>()), (img, vec![ids[0]]));
}

#[test]
fn a_vectorcraft_document_places_linked_and_edit_original_opens_it() {
    let mut app = app();
    let ctx = window(&app);
    // A one-artboard document with a rectangle.
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    let bytes = vectorcraft_format::save_file(&src.active().unwrap().doc);
    let dir = Folder::new("ui-place-document");
    let path = dir.file("badge.vectorcraft", &bytes);
    place_picked(&mut app, &ctx, &path);
    assert!(flag(&app, "link") && flag(&app, "__documents") && !flag(&app, "__others"), "Link on, for a document");
    answer(&mut app, &[]);
    let st = app.session.active().unwrap();
    let id = st.selection.objects[0];
    let vectorcraft_doc::NodeKind::PlacedDocument(p) = &st.doc.node(id).unwrap().kind else { panic!("not a placed document") };
    assert_eq!(p.link.path, path);
    // Edit Original opens the document in a new tab.
    let tabs = app.session.documents().len();
    app.run("links.editOriginal", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), tabs + 1);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path.as_str()));
    // A PNG is an image only; a PNG and a document, both.
    let pic = dir.file("pic.png", &png(10, 10, 72.0));
    place_picked(&mut app, &ctx, &pic);
    assert!(!flag(&app, "__documents") && flag(&app, "__others"));
    dismiss(&mut app);
    place_picked_all(&mut app, &ctx, &[&pic, &path]);
    assert!(flag(&app, "__documents") && flag(&app, "__others"));
}

#[test]
fn the_place_dialog_links_a_pdf_page_and_embeds_it_with_link_off() {
    let mut app = app();
    let ctx = window(&app);
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    let pdf = src.execute("document.serialize", &json!({"format": "pdf", "preserveEditing": false})).unwrap();
    let dir = Folder::new("ui-place-pdf");
    let path = dir.file("flyer.pdf", &vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap());
    // Link on (the default): one locked object, linked to the file.
    place_picked(&mut app, &ctx, &path);
    assert!(flag(&app, "link") && flag(&app, "__documents") && !flag(&app, "__others"), "Link places it as one locked object");
    answer(&mut app, &[]);
    let st = app.session.active().unwrap();
    let vectorcraft_doc::NodeKind::PlacedDocument(p) = &st.doc.node(st.selection.objects[0]).unwrap().kind else { panic!("not a placed document") };
    assert_eq!(p.link.path, path);
    // Link off: the page's art is embedded.
    place_picked(&mut app, &ctx, &path);
    answer(&mut app, &[("link", json!(false))]);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.node(st.selection.objects[0]).unwrap().kind_label(), "Clip Group");
}

#[test]
fn the_place_dialog_offers_a_postscript_ai_and_an_eps_file_as_a_locked_object() {
    let mut app = app();
    let ctx = window(&app);
    let ps = b"%!PS-Adobe-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n1 0 0 setrgbcolor 10 10 50 50 rectfill\nshowpage\n%%EOF\n";
    let dir = Folder::new("ui-place-postscript");
    for name in ["legacy.ai", "legacy.eps", "legacy.ps"] {
        let path = dir.file(name, ps);
        // With Link, file.place places a .ai saved without PDF compatibility, an EPS file and a
        // PostScript file as one locked placed document.
        let r = app.session.execute("file.place", &json!({"path": path, "link": true})).unwrap();
        assert_eq!(r["linked"], json!(true), "{name}: {r}");
        let st = app.session.active().unwrap();
        assert!(matches!(st.doc.node(st.selection.objects[0]).unwrap().kind, NodeKind::PlacedDocument(_)), "{name}");
        // So the dialog counts it as a document.
        place_picked(&mut app, &ctx, &path);
        assert!(flag(&app, "__documents") && !flag(&app, "__others"), "{name}: {:?}", fields(&app));
        dismiss(&mut app);
    }
    // Where the platform reads part of a file, only the file's start is read to tell.
    let asked = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = asked.clone();
    app.services.read_head = Some(Box::new(move |p: &str, max: usize| {
        seen.set(max);
        std::fs::read(p).map(|b| b[..b.len().min(max)].to_vec()).map_err(|e| e.to_string())
    }));
    place_picked(&mut app, &ctx, &dir.file("headed.eps", ps));
    assert!(flag(&app, "__documents"));
    assert_eq!(asked.get(), vectorcraft_engine::cmd::place::DCS_HEAD_BYTES + 1);
    dismiss(&mut app);
    // A DCS file is placed as a linked image, from its start alone too: past the start the
    // PostScript goes on longer than what is read.
    let mut dcs = b"%!PS-Adobe-2.0 EPSF-1.2\n%%BoundingBox: 0 0 20 10\n%%CyanPlate: pic.C\n%%EndComments\n".to_vec();
    dcs.resize(vectorcraft_engine::cmd::place::DCS_HEAD_BYTES + 100, b'\n');
    // The same PostScript behind a binary EPS header (no preview).
    let mut binary = vec![0xC5, 0xD0, 0xD3, 0xC6];
    binary.extend(30u32.to_le_bytes());
    binary.extend((dcs.len() as u32).to_le_bytes());
    binary.resize(30, 0);
    binary.extend(&dcs);
    let files = [("plates.eps", &dcs, true), ("plates.ai", &dcs, true), ("binary.eps", &binary, true), ("binary.eps", &binary, false)];
    for (name, bytes, head) in files {
        if !head {
            app.services.read_head = None;
        }
        place_picked(&mut app, &ctx, &dir.file(name, bytes));
        assert!(!flag(&app, "__documents") && flag(&app, "__others"), "{name}, head {head}: {:?}", fields(&app));
        dismiss(&mut app);
    }
    // A file named .eps that isn't PostScript counts as another file.
    let path = dir.file("notes.eps", b"not PostScript");
    place_picked(&mut app, &ctx, &path);
    assert!(!flag(&app, "__documents") && flag(&app, "__others"), "{:?}", fields(&app));
}

/// Tool key `key` as the Control bar's Apply (Enter) and Cancel (Escape) send it.
fn tool_key(app: &mut VectorcraftApp, key: vectorcraft_tools::ToolKey) {
    let view = app.view_info();
    let r = app.session.tool_key(key, vectorcraft_tools::Mods::default(), view);
    crate::canvas::apply_requests(app, r);
}

/// A press at (10, 10) of the place cursor, dragged and released at `to` (document points), as
/// the canvas sends it to the tool; what the tool asks for is acted on as the canvas acts on it.
fn cursor_gesture(app: &mut VectorcraftApp, mods: vectorcraft_tools::Mods, to: (f64, f64)) {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let view = app.view_info();
    for (kind, (x, y)) in [(PointerKind::Down, (10.0, 10.0)), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        crate::canvas::dispatch(app, &PointerEvent::new(kind, x, y).with_mods(mods), view);
    }
}

/// A click of the place cursor at (10, 10).
fn click(app: &mut VectorcraftApp) {
    cursor_gesture(app, vectorcraft_tools::Mods::default(), (10.0, 10.0));
}

/// Holding Escape with the place cursor loaded discards one file for the press. The repeats discard
/// no more and run no command bound to Escape.
#[test]
fn holding_escape_with_the_place_cursor_loaded_discards_one_file() {
    let mut app = app();
    let ctx = window(&app);
    frame(&mut app, &ctx, vec![], &[], false);
    app.run("shortcuts.set", json!({"id": "select.none", "shortcut": "Escape", "force": true})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let dir = Folder::new("ui-place-held-escape");
    let paths: Vec<String> = ["held-a.png", "held-b.png", "held-c.png"].iter().map(|name| dir.file(name, &png(10, 10, 72.0))).collect();
    app.run("file.place", json!({"paths": paths})).unwrap();
    answer(&mut app, &[]);
    assert_eq!(app.session.tool_options()["count"], 3);
    let selected = app.session.active().unwrap().selection.objects.clone();
    assert!(!selected.is_empty());
    // The host marks the presses after the first as the held key's repeats.
    let key = |pressed, repeat| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![], &[], false);
    for repeat in 0..4 {
        frame(&mut app, &ctx, vec![key(true, repeat > 0)], &[], false);
        assert_eq!(app.session.tool_options()["count"], 2, "repeat {repeat}: one file is discarded");
        assert_eq!(app.session.active().unwrap().selection.objects, selected, "repeat {repeat}: no command ran");
    }
    frame(&mut app, &ctx, vec![key(false, false)], &[], false);
    frame(&mut app, &ctx, vec![key(true, false)], &[], false);
    assert_eq!(app.session.tool_options()["count"], 1, "a new press discards the next file");
}

/// A command bound to Shift+Escape keeps repeating with the place cursor loaded, and no file is
/// discarded.
#[test]
fn holding_a_chord_bound_to_a_command_repeats_it_with_the_place_cursor_loaded() {
    let mut app = app();
    let ctx = window(&app);
    frame(&mut app, &ctx, vec![], &[], false);
    app.run("shortcuts.set", json!({"id": "view.zoomIn", "shortcut": "Shift+Escape", "force": true})).unwrap();
    let dir = Folder::new("ui-place-held-chord");
    let paths: Vec<String> = ["chord-a.png", "chord-b.png"].iter().map(|name| dir.file(name, &png(10, 10, 72.0))).collect();
    app.run("file.place", json!({"paths": paths})).unwrap();
    answer(&mut app, &[]);
    let modifiers = egui::Modifiers::SHIFT;
    let press = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers };
    frame(&mut app, &ctx, vec![], &[], false);
    let mut zoom = app.view().unwrap().zoom;
    for repeat in 0..3 {
        frame(&mut app, &ctx, vec![press.clone()], &[], true);
        assert!(app.view().unwrap().zoom > zoom, "repeat {repeat}: the command runs");
        assert_eq!(app.session.tool_options()["count"], 2, "repeat {repeat}: no file is discarded");
        zoom = app.view().unwrap().zoom;
    }
}

/// #734: Crop Image shows a crop box on the image instead of cropping to the artboard at once (an
/// image inside the artboard had nothing to cut); Apply crops to the box in one undo step, Cancel
/// leaves the image whole, and both go back to the Selection tool.
#[test]
fn crop_image_shows_a_box_that_apply_crops_to() {
    use vectorcraft_tools::ToolKey;
    let mut app = app();
    let dir = Folder::new("ui-place-crop");
    app.run("file.place", json!({"path": dir.file("crop.png", &png(100, 50, 72.0)), "link": false})).unwrap();
    let whole = selected_image(&app).geometric_bounds().unwrap();
    assert!(crate::menus::enabled(&app, "ui.cropImage"));
    app.run("ui.cropImage", json!({})).unwrap();
    assert_eq!(app.session.tool_id(), "cropImage");
    tool_key(&mut app, ToolKey::Escape);
    assert_eq!(app.session.tool_id(), "selection");
    assert_eq!(selected_image(&app).geometric_bounds(), Some(whole), "Cancel leaves it whole");
    // The box set to the image's left half, as the Control bar's fields set it, then Apply.
    app.run("ui.cropImage", json!({})).unwrap();
    app.run("tool.setOption", json!({"key": "rect", "value": [whole.x0, whole.y0, whole.width() / 2.0, whole.height()]})).unwrap();
    tool_key(&mut app, ToolKey::Enter);
    assert_eq!(app.session.tool_id(), "selection");
    let b = selected_image(&app).geometric_bounds().unwrap();
    assert!(
        (b.x0 - whole.x0).abs() < 1e-6 && (b.width() - whole.width() / 2.0).abs() < 1e-6 && (b.height() - whole.height()).abs() < 1e-6,
        "{b:?} of {whole:?}"
    );
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(selected_image(&app).geometric_bounds(), Some(whole), "one undo step");
    // Without an image selected there's nothing to crop.
    app.run("select.none", json!({})).unwrap();
    assert!(!crate::menus::enabled(&app, "ui.cropImage") && app.run("ui.cropImage", json!({})).is_err());
}

/// With Crop Image in use and room to spare, the Control bar shows the cropping options: the
/// resolution, the box's position and size, and Apply and Cancel.
#[test]
fn the_control_bar_shows_crop_image_s_options_with_apply_and_cancel() {
    use crate::tests_removeanchors::{control_frame, has};
    let mut app = app();
    let dir = Folder::new("ui-place-crop-bar");
    app.run("file.place", json!({"path": dir.file("crop.png", &png(100, 50, 72.0)), "link": false})).unwrap();
    app.run("ui.cropImage", json!({})).unwrap();
    let ctx = window(&app);
    let bar = control_frame(&mut app, &ctx, vec![]);
    for s in ["PPI", "X", "Y", "W", "H", "Apply", "Cancel"] {
        assert!(has(&bar, s), "Crop Image: {s} missing from {:?}", bar.iter().map(|(t, _)| t).collect::<Vec<_>>());
    }
}

/// Crop Image's Apply and Object › Rasterize say in the status bar when the image couldn't be
/// decoded, as raster exports do.
#[test]
fn crop_and_rasterize_say_in_the_status_bar_when_an_image_could_not_be_decoded() {
    const WARNING: &str = "an image couldn't be decoded and is missing from the picture";
    // A PNG whose header reads but whose pixel data is cut short.
    let mut app = app();
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png(100, 50, 72.0), json!({}));
    let whole = selected_image(&app).geometric_bounds().unwrap();
    app.run("ui.cropImage", json!({})).unwrap();
    app.run("tool.setOption", json!({"key": "rect", "value": [whole.x0, whole.y0, whole.width() / 2.0, whole.height()]})).unwrap();
    app.status("");
    tool_key(&mut app, vectorcraft_tools::ToolKey::Enter);
    assert_eq!(app.ui.status, WARNING, "Crop Image's Apply");
    // Undo puts the image back as placed; then Rasterize runs on it.
    app.run("edit.undo", json!({})).unwrap();
    app.status("");
    app.run("object.rasterize", json!({"ppi": 72})).unwrap();
    assert_eq!(app.ui.status, WARNING, "Rasterize");
}

/// One headless frame of the whole window at `time` → how long the app asked to wait before the
/// next one.
fn frame_at(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> std::time::Duration {
    // The app's background work (the link-watch look) runs on host time; drive it with this test's
    // simulated seconds rather than the real monotonic clock, as the window loop would.
    app.now_override = Some(time);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(time), ..Default::default() };
    let out = crate::testing::app_frame(app, ctx, raw);
    out.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay)
}

/// The window sits idle for `secs` from `*time` (a frame now, then only the frames the app asks
/// for) or until `done` → whether it was.
fn idle(app: &mut VectorcraftApp, ctx: &egui::Context, time: &mut f64, secs: f64, done: &dyn Fn(&VectorcraftApp) -> bool) -> bool {
    let end = *time + secs;
    loop {
        let wait = frame_at(app, ctx, *time);
        if done(app) {
            return true;
        }
        let next = *time + wait.as_secs_f64().max(1.0 / 60.0);
        if next > end {
            *time = end;
            return false;
        }
        // The look runs on a worker thread, in real time: the clock waits for it to finish, else a
        // busy machine's slow look outlasts the simulated seconds and the test fails at random.
        if app.session.link_scan.is_some() {
            app.session.wait_link_scan();
            continue;
        }
        *time = next;
    }
}

/// The text of the one selected type object.
fn selected_text(app: &VectorcraftApp) -> String {
    let st = app.session.active().unwrap();
    let NodeKind::Text(t) = &st.doc.node(st.selection.objects[0]).unwrap().kind else { panic!("not type") };
    t.plain_text()
}

/// The undo steps of the active document.
fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// The first selected object's width.
fn selected_width(app: &VectorcraftApp) -> f64 {
    let st = app.session.active().unwrap();
    st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap().width()
}

const NOTES: &[u8] = b"one\r\ntwo\r\n\r\nthree\r\n";

/// A DXF drawing of one 100 × 50 line.
const LINE_DXF: &[u8] = b"0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n100\n21\n50\n0\nENDSEC\n0\nEOF\n";

/// Two pages: 100 × 100 and 300 × 100.
fn two_pages() -> [PdfPage; 2] {
    [PdfPage::new(100.0, 100.0, "0 g 0 0 10 10 re f"), PdfPage::new(300.0, 100.0, "1 0 0 rg 0 0 30 30 re f")]
}

#[test]
fn show_import_options_is_a_place_option_saved_with_the_preferences() {
    let mut app = app();
    let dir = Folder::new("ui-place-show-import-options");
    let notes = dir.file("notes.txt", NOTES);
    app.run("file.place", json!({"paths": [notes]})).unwrap();
    assert!(flag(&app, "showImportOptions"), "on by default");
    // On: the text file asks for its import options first.
    answer(&mut app, &[]);
    assert_eq!(dialog_kind(&app).as_deref(), Some(TEXT_IMPORT));
    dismiss(&mut app);
    // Off: it is placed at once, and the next Place dialog starts off.
    app.run("file.place", json!({"paths": [notes]})).unwrap();
    answer(&mut app, &[("showImportOptions", json!(false))]);
    assert_eq!(dialog_kind(&app), None);
    assert!(selected_text(&app).contains("three"));
    assert!(!app.ui.place_import_options);
    app.run("file.place", json!({"paths": [notes]})).unwrap();
    assert!(!flag(&app, "showImportOptions"), "remembered");
    // `showImportOptions` given with `paths` sets the dialog's checkbox.
    app.run("file.place", json!({"paths": [notes], "showImportOptions": true})).unwrap();
    assert!(flag(&app, "showImportOptions"));
    // Saved with the preferences; older preferences without it start on.
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(saved["place_import_options"], false);
    let old: crate::UiState = serde_json::from_value(json!({"place_link": false})).unwrap();
    assert!(old.place_import_options && old.text_import.is_null());
}

#[test]
fn with_show_import_options_off_a_file_takes_the_options_confirmed_last() {
    let mut app = app();
    let b64 = vectorcraft_format::base64_encode;
    // Nothing confirmed yet: file.place's defaults.
    app.run("file.place", json!({"name": "a.txt", "dataBase64": b64(NOTES), "showImportOptions": false})).unwrap();
    assert_eq!(dialog_kind(&app), None);
    let plain = selected_text(&app);
    // Text Import Options remembers what it confirmed, and the next text file starts from it.
    app.run("file.place", json!({"name": "b.txt", "dataBase64": b64(NOTES)})).unwrap();
    assert_eq!(dialog_kind(&app).as_deref(), Some(TEXT_IMPORT));
    answer(
        &mut app,
        &[("removeLineReturns", json!(true)), ("removeParagraphReturns", json!(true)), ("replaceSpaces", json!(true)), ("spaces", json!(4))],
    );
    assert_eq!(selected_text(&app), "one two\nthree");
    assert_ne!(plain, "one two\nthree");
    app.run("file.place", json!({"name": "c.txt", "dataBase64": b64(NOTES)})).unwrap();
    assert!(flag(&app, "removeLineReturns") && flag(&app, "replaceSpaces"));
    assert_eq!(fields(&app)["spaces"].as_f64(), Some(4.0));
    dismiss(&mut app);
    app.run("file.place", json!({"name": "d.txt", "dataBase64": b64(NOTES), "showImportOptions": false})).unwrap();
    assert_eq!((dialog_kind(&app), selected_text(&app).as_str()), (None, "one two\nthree"));
    // A DXF drawing: placed without asking, with the fit chosen last.
    app.run("file.place", json!({"name": "line.dxf", "dataBase64": b64(LINE_DXF), "showImportOptions": false})).unwrap();
    assert_eq!(dialog_kind(&app), None);
    let unfitted = selected_width(&app);
    app.ui.dxf_import = json!({"fit": true});
    app.run("file.place", json!({"name": "line.dxf", "dataBase64": b64(LINE_DXF), "showImportOptions": false})).unwrap();
    assert_eq!(dialog_kind(&app), None);
    assert!(selected_width(&app) > unfitted + 1.0, "fitted: {} vs {unfitted}", selected_width(&app));
    app.run("file.place", json!({"name": "line.dxf", "dataBase64": b64(LINE_DXF)})).unwrap();
    assert_eq!(dialog_kind(&app).as_deref(), Some(DXF_IMPORT), "on: it asks");
    dismiss(&mut app);
    // A PDF with several pages: its first page, without asking.
    let pages = two_pages();
    let two = pdf(&pages, None);
    app.run("file.place", json!({"name": "two.pdf", "dataBase64": b64(&two), "showImportOptions": false})).unwrap();
    assert_eq!(dialog_kind(&app), None);
    assert!((selected_width(&app) - 100.0).abs() < 0.01, "page 1: {}", selected_width(&app));
    // An encrypted PDF still asks for its password, and then places its first page.
    let locked = pdf(&pages, Some("pw"));
    app.run("file.place", json!({"name": "locked.pdf", "dataBase64": b64(&locked), "showImportOptions": false})).unwrap();
    assert!(dialog_kind(&app).is_some(), "the password dialog");
    answer(&mut app, &[("password", json!("pw"))]);
    assert_eq!(dialog_kind(&app), None, "no page to pick");
    assert!((selected_width(&app) - 100.0).abs() < 0.01, "page 1: {}", selected_width(&app));
    // Given `page`, it asks for the password and places that page.
    app.run("file.place", json!({"name": "locked.pdf", "dataBase64": b64(&locked), "page": 2, "showImportOptions": false})).unwrap();
    answer(&mut app, &[("password", json!("pw"))]);
    assert!((selected_width(&app) - 300.0).abs() < 0.01, "page 2: {}", selected_width(&app));
    // A page past the last one is refused once the file is unlocked, not placed as another page.
    let before = undo_steps(&app);
    app.run("file.place", json!({"name": "locked.pdf", "dataBase64": b64(&locked), "page": 9, "showImportOptions": false})).unwrap();
    answer(&mut app, &[("password", json!("pw"))]);
    assert_eq!(undo_steps(&app), before, "page 9 is not placed");
    dismiss(&mut app);
    // A value that is not true or false is refused, and nothing is placed.
    for bad in [json!("no"), json!(0)] {
        assert!(app.run("file.place", json!({"name": "e.txt", "dataBase64": b64(NOTES), "showImportOptions": bad})).is_err());
    }
    assert_eq!((dialog_kind(&app), undo_steps(&app)), (None, before));
}

#[test]
fn the_place_cursors_files_ask_for_their_import_options_when_show_import_options_is_on() {
    let mut app = app();
    let dir = Folder::new("ui-place-cursor-import-options");
    let a = dir.file("a.png", &png(10, 10, 72.0));
    let notes = dir.file("notes.txt", NOTES);
    // On: a drag over the text file asks for its import options, and OK places it.
    app.run("file.place", json!({"paths": [notes, a]})).unwrap();
    answer(&mut app, &[]);
    assert_eq!(app.session.tool_options()["count"], 2);
    cursor_gesture(&mut app, vectorcraft_tools::Mods::default(), (210.0, 110.0));
    assert_eq!(dialog_kind(&app).as_deref(), Some(TEXT_IMPORT));
    answer(&mut app, &[]);
    assert!(selected_text(&app).contains("three"));
    // A click on a PDF places the page chosen at its own size, centered where the click put the
    // first page; a drag keeps its box.
    let two = dir.file("two.pdf", &pdf(&two_pages(), None));
    app.run("file.place", json!({"paths": [two, a, two]})).unwrap();
    answer(&mut app, &[("link", json!(false))]);
    click(&mut app);
    assert_eq!(dialog_kind(&app).as_deref(), Some(PDF_IMPORT));
    answer(&mut app, &[("page", json!(2))]);
    let st = app.session.active().unwrap();
    let b = st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.width().round(), b.center()), (300.0, vectorcraft_geom::Point::new(60.0, 60.0)));
    app.run("file.place", json!({"paths": [two, a]})).unwrap();
    answer(&mut app, &[("link", json!(false))]);
    cursor_gesture(&mut app, vectorcraft_tools::Mods::default(), (60.0, 60.0));
    assert_eq!(dialog_kind(&app).as_deref(), Some(PDF_IMPORT));
    dismiss(&mut app);
    // Off: the cursor's files are placed at once, with the options confirmed last.
    app.run("file.place", json!({"paths": [notes, a]})).unwrap();
    answer(&mut app, &[("showImportOptions", json!(false))]);
    assert_eq!(app.session.tool_options()["count"], 2);
    click(&mut app);
    assert_eq!(dialog_kind(&app), None);
    assert!(selected_text(&app).contains("three"));
    // A cursor loaded by file.place.queue places at once, as before.
    app.run("file.place", json!({"paths": [a, notes]})).unwrap();
    answer(&mut app, &[]);
    app.run("file.place.queue", json!({"paths": [notes, a]})).unwrap();
    click(&mut app);
    assert_eq!(dialog_kind(&app), None);
}

/// A Shift-click of the place cursor asks for the current file's import options however the
/// cursor was loaded; a file without import options, and a Shift-drag, place as before.
#[test]
fn a_shift_click_of_the_place_cursor_asks_for_the_files_import_options() {
    let mut app = app();
    let dir = Folder::new("ui-place-cursor-shift-click");
    let a = dir.file("a.png", &png(10, 10, 72.0));
    let notes = dir.file("notes.txt", NOTES);
    let line = dir.file("line.dxf", LINE_DXF);
    let shift = vectorcraft_tools::Mods { shift: true, ..Default::default() };
    let count = |app: &VectorcraftApp| app.session.tool_options()["count"].as_u64();
    let load = |app: &mut VectorcraftApp, paths: Value, show: bool| {
        app.run("file.place", json!({ "paths": paths })).unwrap();
        answer(app, &[("showImportOptions", json!(show))]);
    };
    // Show Import Options off: a Shift-click on a text file opens Text Import Options, whose OK
    // places it.
    load(&mut app, json!([notes, line, a]), false);
    assert_eq!(count(&app), Some(3));
    cursor_gesture(&mut app, shift, (10.0, 10.0));
    assert_eq!(dialog_kind(&app).as_deref(), Some(TEXT_IMPORT));
    answer(&mut app, &[]);
    assert_eq!(dialog_kind(&app), None);
    assert!(selected_text(&app).contains("three"));
    assert_eq!(count(&app), Some(2), "the cursor goes on to the next file");
    // A DXF drawing opens DXF Import Options.
    cursor_gesture(&mut app, shift, (10.0, 10.0));
    assert_eq!(dialog_kind(&app).as_deref(), Some(DXF_IMPORT));
    dismiss(&mut app);
    // A PNG image has no import options: it is placed.
    let placed = images(&app, 0);
    cursor_gesture(&mut app, shift, (10.0, 10.0));
    assert_eq!((dialog_kind(&app), images(&app, 0)), (None, placed + 1));
    // Show Import Options on: one dialog, not two.
    load(&mut app, json!([notes, a]), true);
    cursor_gesture(&mut app, shift, (10.0, 10.0));
    assert_eq!(dialog_kind(&app).as_deref(), Some(TEXT_IMPORT));
    answer(&mut app, &[]);
    assert_eq!(dialog_kind(&app), None);
    // Off, a Shift-drag places at once, as a drag does.
    load(&mut app, json!([notes, a]), false);
    cursor_gesture(&mut app, shift, (200.0, 10.0));
    assert_eq!(dialog_kind(&app), None);
    assert!(selected_text(&app).contains("three"));
    // A plain click places at once.
    let placed = images(&app, 0);
    click(&mut app);
    assert_eq!((dialog_kind(&app), images(&app, 0)), (None, placed + 1));
    // A cursor loaded by file.place.queue with a `page`: the Shift-click still asks for the page.
    let pages = two_pages();
    let two = dir.file("two.pdf", &pdf(&pages, None));
    app.run("file.place.queue", json!({"paths": [two], "link": false, "page": 2})).unwrap();
    cursor_gesture(&mut app, shift, (10.0, 10.0));
    assert_eq!(dialog_kind(&app).as_deref(), Some(PDF_IMPORT));
    dismiss(&mut app);
    // A one-page PDF opens no dialog, and is placed with the options the cursor was loaded with.
    let one = dir.file("one.pdf", &pdf(&pages[..1], None));
    app.run("file.place.queue", json!({"paths": [one], "link": false, "crop": "bounding"})).unwrap();
    cursor_gesture(&mut app, shift, (10.0, 10.0));
    assert_eq!(dialog_kind(&app), None);
    assert!((selected_width(&app) - 10.0).abs() < 0.01, "the bounding box: {}", selected_width(&app));
}

/// An encrypted PDF in the place cursor asks for its password when clicked, with Show Import
/// Options on or off, and the password's OK places it where the click put it.
#[test]
fn an_encrypted_pdf_in_the_place_cursor_asks_for_its_password_when_clicked() {
    let mut app = app();
    let dir = Folder::new("ui-place-cursor-encrypted-pdf");
    let a = dir.file("a.png", &png(10, 10, 72.0));
    let pages = two_pages();
    let locked = dir.file("locked.pdf", &pdf(&pages, Some("pw")));
    // The click gives the box of the size the cursor holds it at, a letter page.
    let center = vectorcraft_geom::Point::new(316.0, 406.0);
    let placed = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let b = st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap();
        (b.width().round(), b.center())
    };
    for show in [false, true] {
        app.run("file.place", json!({"paths": [locked, a]})).unwrap();
        answer(&mut app, &[("showImportOptions", json!(show)), ("link", json!(false))]);
        assert_eq!(app.session.tool_options()["count"], 2, "kept in the cursor");
        click(&mut app);
        assert!(dialog_kind(&app).is_some(), "the password dialog");
        answer(&mut app, &[("password", json!("pw"))]);
        if show {
            // Unlocked, it asks for the page.
            assert_eq!(dialog_kind(&app).as_deref(), Some(PDF_IMPORT));
            answer(&mut app, &[]);
        }
        assert_eq!((dialog_kind(&app), placed(&app)), (None, (100.0, center)), "Show Import Options {show}");
    }
    // A cursor loaded by file.place.queue asks too, also with a `page`, which it then places.
    app.run("file.place.queue", json!({"paths": [locked, a], "link": false, "page": 2})).unwrap();
    click(&mut app);
    assert!(dialog_kind(&app).is_some(), "the password dialog");
    answer(&mut app, &[("password", json!("pw"))]);
    assert_eq!((dialog_kind(&app), placed(&app).0), (None, 300.0), "page 2");
    // File › Place of it with a `page` and Show Import Options on asks for the password too.
    crate::place::run_for_user(&mut app, &json!({"path": locked, "link": false, "page": 1})).unwrap();
    assert!(dialog_kind(&app).is_some());
    dismiss(&mut app);
    // An encrypted PDF whose name has no PDF extension is still left out of the cursor.
    let misnamed = dir.file("locked.dat", &pdf(&pages, Some("pw")));
    let r = app.run("file.place.queue", json!({"paths": [a, misnamed]})).unwrap();
    assert_eq!((&r["count"], &r["skipped"][0]["name"]), (&json!(1), &json!("locked.dat")), "{r}");
}

/// A cursor loaded again after the Place dialog loaded it with Show Import Options on (a queue set
/// on the Place tool) places its files at once.
#[test]
fn a_place_cursor_loaded_again_no_longer_asks_for_import_options() {
    let mut app = app();
    let dir = Folder::new("ui-place-cursor-reloaded");
    let a = dir.file("a.png", &png(10, 10, 72.0));
    let notes = dir.file("notes.txt", NOTES);
    app.run("file.place", json!({"paths": [notes, a]})).unwrap();
    answer(&mut app, &[]);
    let entries: Vec<Value> = [(&notes, "notes.txt"), (&a, "a.png")]
        .iter()
        .map(|(path, name)| json!({"params": {"path": path}, "name": name, "width": 190, "height": 90}))
        .collect();
    assert_eq!(app.session.tool_id(), "place");
    app.run("tool.setOption", json!({"values": {"queue": {"entries": entries}}})).unwrap();
    click(&mut app);
    assert_eq!(dialog_kind(&app), None);
    assert!(selected_text(&app).contains("three"));
}

/// The import options a click of a cursor loaded within automation roots asks for place within
/// those roots when OK is pressed, by the person at the keyboard too: a file outside them is not
/// read. One inside them is placed.
#[test]
fn the_import_options_a_place_cursor_asks_for_place_within_its_roots() {
    use vectorcraft_engine::file_access::{AutomationRoots, confine, unconfined};
    let mut app = app();
    let dir = Folder::new("ui-place-cursor-ask-roots");
    std::fs::create_dir_all(dir.join("in")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let a = dir.file("in/a.png", &png(10, 10, 72.0));
    let notes = dir.file("in/notes.txt", NOTES);
    let two = dir.file("in/two.pdf", &pdf(&two_pages(), None));
    let outside_pdf = dir.file("outside.pdf", &pdf(&two_pages(), None));
    let roots = AutomationRoots::new(&[&dir.join("in")], &[]).unwrap();
    let narrower = AutomationRoots::new(&[&dir.join("b")], &[]).unwrap();
    // The cursor loaded within the roots with `file` first, and the first file clicked.
    let ask = |app: &mut VectorcraftApp, file: &str| {
        confine(roots.as_ref(), || {
            app.run("file.place", json!({"paths": [file, a]})).unwrap();
            answer(app, &[("link", json!(false))]);
        });
        assert_eq!(app.session.tool_options()["count"], 2);
        unconfined(|| click(app));
    };
    let before = undo_steps(&app);
    // A caller with roots of its own confirms within those: the file in the cursor's roots but
    // outside the caller's is not placed.
    ask(&mut app, &notes);
    assert_eq!(dialog_kind(&app).as_deref(), Some(TEXT_IMPORT));
    confine(narrower.as_ref(), || answer(&mut app, &[]));
    assert_eq!(undo_steps(&app), before, "outside the caller's roots");
    dismiss(&mut app);
    // The PDF's OK, its file outside the roots: nothing is placed.
    ask(&mut app, &two);
    assert_eq!(dialog_kind(&app).as_deref(), Some(PDF_IMPORT));
    unconfined(|| answer(&mut app, &[("path", json!(outside_pdf))]));
    assert_eq!(undo_steps(&app), before, "the PDF outside the roots");
    dismiss(&mut app);
    // The text file inside the roots is placed.
    ask(&mut app, &notes);
    unconfined(|| answer(&mut app, &[]));
    assert!(selected_text(&app).contains("three"));
    // Once the cursor is loaded again, the earlier click's import options place nothing.
    let after = undo_steps(&app);
    ask(&mut app, &notes);
    assert_eq!(dialog_kind(&app).as_deref(), Some(TEXT_IMPORT));
    app.run("file.place.queue", json!({"paths": [a]})).unwrap();
    unconfined(|| answer(&mut app, &[]));
    assert_eq!(undo_steps(&app), after);
    dismiss(&mut app);
    // So do those of a click before a queue set on the Place tool, by either caller.
    for caller in [None, Some(roots.as_ref())] {
        ask(&mut app, &notes);
        assert_eq!(dialog_kind(&app).as_deref(), Some(TEXT_IMPORT));
        let entry = json!({"params": {"path": a}, "name": "a.png", "width": 10, "height": 10});
        app.run("tool.setOption", json!({"values": {"queue": {"entries": [entry]}}})).unwrap();
        match caller {
            None => unconfined(|| answer(&mut app, &[])),
            Some(roots) => confine(roots, || answer(&mut app, &[])),
        }
        assert_eq!(undo_steps(&app), after);
        dismiss(&mut app);
    }
}

#[test]
fn a_file_placed_while_idle_follows_its_changes() {
    let mut app = app();
    // Every frame renders on this thread, however long it takes: a frame finished by the render
    // worker during the idle frames asks for a repaint.
    app.canvas.worker_started = true;
    app.run("prefs.set", json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let ctx = window(&app);
    let mut time = 0.0;
    idle(&mut app, &ctx, &mut time, 3.0, &|_| false);
    // A look finds the new document without links: no wakes of its own for it.
    assert_eq!(frame_at(&mut app, &ctx, time), std::time::Duration::MAX, "{:?}", ctx.repaint_causes());
    // Half a second later, before the next look is due, a file is placed. The place itself is a
    // frame (as any command); then nothing happens for 3 s.
    time += 0.5;
    let dir = Folder::new("ui-place-idle-watch");
    let path = dir.file("idle-watch.png", &png(40, 30, 72.0));
    app.run("file.place", json!({"path": path})).unwrap();
    let id = app.session.active().unwrap().selection.objects[0];
    idle(&mut app, &ctx, &mut time, 3.0, &|_| false);
    let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    let changed = png(60, 30, 72.0);
    std::fs::write(&path, &changed).unwrap();
    // The rewritten file is dated 2 s later, however fast it was written.
    let later = std::fs::FileTimes::new().set_modified(stamp + std::time::Duration::from_secs(2));
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_times(later).unwrap();
    let hash = vectorcraft_doc::links::hash_bytes(&changed);
    let updated = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        matches!(&st.doc.node(id).unwrap().kind, NodeKind::Image(im) if im.link.as_ref().and_then(|l| l.hash.as_deref()) == Some(hash.as_str()))
    };
    assert!(idle(&mut app, &ctx, &mut time, 10.0, &updated), "the changed file was never read again");
}

/// A look whose worker ended without stamps is cleared: the app stops asking for a frame every
/// 100 ms, and the next look, on schedule, reads a changed file again.
#[test]
fn a_look_whose_worker_stopped_does_not_stop_later_looks() {
    let mut app = app();
    // Every frame renders on this thread: a frame finished by the render worker asks for a repaint.
    app.canvas.worker_started = true;
    app.run("prefs.set", json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let dir = Folder::new("ui-place-stopped-watch");
    let path = dir.file("stopped-watch.png", &png(40, 30, 72.0));
    app.run("file.place", json!({"path": path})).unwrap();
    let id = app.session.active().unwrap().selection.objects[0];
    let ctx = window(&app);
    let mut time = 0.0;
    idle(&mut app, &ctx, &mut time, 3.0, &|_| false);
    // A look that is due starts in this frame and is applied in the next one, at the same time.
    time += 3.0;
    frame_at(&mut app, &ctx, time);
    app.session.wait_link_scan();
    frame_at(&mut app, &ctx, time);
    assert!(app.session.link_scan.is_none());
    let scan = vectorcraft_engine::link_watch::LinkScan::default();
    scan.2.store(true, std::sync::atomic::Ordering::SeqCst);
    app.session.link_scan = Some(scan);
    // Half a second later, before the next look is due.
    time += 0.5;
    let wait = frame_at(&mut app, &ctx, time);
    assert!(app.session.link_scan.is_none(), "the look ended");
    assert!(wait > std::time::Duration::from_millis(100), "{wait:?}: {:?}", ctx.repaint_causes());
    let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    let changed = png(60, 30, 72.0);
    std::fs::write(&path, &changed).unwrap();
    // The rewritten file is dated 2 s later, however fast it was written.
    let later = std::fs::FileTimes::new().set_modified(stamp + std::time::Duration::from_secs(2));
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_times(later).unwrap();
    let hash = vectorcraft_doc::links::hash_bytes(&changed);
    let updated = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        matches!(&st.doc.node(id).unwrap().kind, NodeKind::Image(im) if im.link.as_ref().and_then(|l| l.hash.as_deref()) == Some(hash.as_str()))
    };
    assert!(idle(&mut app, &ctx, &mut time, 10.0, &updated), "the changed file was never read again");
}
