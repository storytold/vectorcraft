//! File → Place in the app: the Place dialog, files dropped on the window, the loaded place
//! cursor and the Control bar's image details.

use std::sync::Arc;

use egui::{Pos2, Shape, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::canvas::Xf;
use crate::place::{DropAt, DropTarget};

/// A `w`×`h` red PNG declaring `ppi`.
fn png(w: u32, h: u32, ppi: f64) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    vectorcraft_engine::cmd::fileio::ppi::with_png_resolution(&out, (ppi, ppi))
}

/// `bytes` written to a fresh temporary file named `name` → its path.
fn temp_file(name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("vectorcraft-ui-place-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().to_string()
}

/// An app reading files from disk, with a 400×300 document.
fn app() -> VectorcraftApp {
    let services = crate::Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// File → Place… with `path` chosen in the file picker.
fn place_picked(app: &mut VectorcraftApp, path: &str) {
    let path = path.to_string();
    app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(path.clone())));
    app.run("file.place", json!({})).unwrap();
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

/// One headless frame of the whole window (800×600) with `events` and `dropped` files.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, dropped: &[&str], shift: bool) {
    let files = dropped.iter().map(|p| Arc::new(Dropped(p.into())) as egui::DroppedFileHandle).collect();
    frame_files(app, ctx, events, files, shift);
}

fn frame_files(app: &mut VectorcraftApp, ctx: &egui::Context, mut events: Vec<egui::Event>, dropped: Vec<egui::DroppedFileHandle>, shift: bool) {
    events.push(egui::Event::ModifiersChanged(egui::Modifiers { shift, ..Default::default() }));
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        events,
        dropped_files: dropped,
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
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
    app.canvas_rect.expect("the canvas is laid out")
}

/// Where the platform tells where files were dropped (the web), documents, pictures and text go
/// as in Illustrator: placed at the pointer on the canvas, opened off it (the tab bar).
#[test]
fn a_drop_with_a_position_places_on_the_canvas_and_opens_off_it() {
    let mut app = app();
    let ctx = egui::Context::default();
    let rect = laid_out(&mut app, &ctx);
    let pos = rect.center() + vec2(60.0, -40.0);
    let want = Xf::new(rect, app.view().unwrap()).to_doc(pos);
    let uid = app.session.active().unwrap().uid;
    for name in ["a.png", "a.svg", "a.ai", "a.vectorcraft"] {
        assert_eq!(app.drop_target(name, Some(pos), true), DropTarget::Place(DropAt { doc: uid, at: want, embed: true }), "{name}");
        assert_eq!(app.drop_target(name, Some(Pos2::new(300.0, rect.top() - 10.0)), false), DropTarget::Open, "{name} on the tab bar");
    }
}

/// Desktop drags carry no position (#472): a dropped document opens as a tab of its own, a
/// picture or text is placed in the middle of the view.
#[test]
fn a_drop_without_a_position_opens_documents_and_places_pictures() {
    let mut app = app();
    let ctx = egui::Context::default();
    laid_out(&mut app, &ctx);
    let center = app.view().unwrap().center;
    let uid = app.session.active().unwrap().uid;
    for name in ["a.svg", "a.svgz", "a.pdf", "a.ai", "a.eps", "a.vectorcraft", "a.drawcraft", "a.vctemplate", "a.dxf", "a.emf"] {
        assert_eq!(app.drop_target(name, None, false), DropTarget::Open, "{name}");
    }
    for name in ["a.png", "a.jpg", "a.tiff", "a.webp", "a.txt"] {
        assert_eq!(app.drop_target(name, None, false), DropTarget::Place(DropAt { doc: uid, at: center, embed: false }), "{name}");
    }
}

/// Swatch libraries and color books, graphic style libraries, Libraries panel files, presets and
/// plug-ins open as File → Open opens them, dropped with or without a position, on the canvas too.
#[test]
fn libraries_presets_and_plugins_open_wherever_they_are_dropped() {
    let mut app = app();
    let ctx = egui::Context::default();
    let rect = laid_out(&mut app, &ctx);
    let on_canvas = rect.center() + vec2(60.0, -40.0);
    let libraries = ["a.vcswatches", "a.gpl", "a.ase", "a.acb", "a.vcstyles", "a.vclibrary"];
    let presets_and_plugins = ["a.vcflattener", "a.vcpdfpresets", "a.vcprintpresets", "a.vcperspective", "a.wasm"];
    for name in libraries.into_iter().chain(presets_and_plugins) {
        assert_eq!(app.drop_target(name, None, false), DropTarget::Open, "{name}");
        assert_eq!(app.drop_target(name, Some(on_canvas), true), DropTarget::Open, "{name} on the canvas");
    }
}

/// A swatch library dropped on the window while a document is open loads in the library panel,
/// as File → Open loads it.
#[test]
fn a_dropped_swatch_library_opens_in_the_library_panel() {
    let mut app = app();
    let ctx = egui::Context::default();
    laid_out(&mut app, &ctx);
    let gpl = temp_file("dropped.gpl", b"GIMP Palette\nName: Dropped\n#\n255   0   0\tRed\n  0 128 255\tSky\n");
    frame(&mut app, &ctx, vec![], &[&gpl], false);
    let id = format!("loaded/{gpl}");
    assert_eq!(app.ui.library_panel.as_ref().map(|o| o.id.as_str()), Some(id.as_str()), "{}", app.ui.status);
    let lib = app.session.execute("swatch.library.get", &json!({"library": id})).unwrap();
    let names: Vec<&str> = lib["swatches"].as_array().unwrap().iter().filter_map(|w| w["name"].as_str()).collect();
    assert_eq!((lib["name"].as_str(), names), (Some("Dropped"), vec!["Red", "Sky"]));
    assert_eq!((app.session.documents().len(), images(&app, 0)), (1, 0), "neither opened as a document nor placed");
    assert_eq!(app.ui.recent_files.first(), Some(&gpl), "added to Open Recent Files as File → Open adds it");
}

/// A Libraries panel file dropped on the window while a document is open is added to the
/// Libraries panel as a new library, as File → Open adds it.
#[test]
fn a_dropped_libraries_panel_file_is_added_as_a_new_library() {
    let mut app = app();
    let ctx = egui::Context::default();
    laid_out(&mut app, &ctx);
    let file = json!({"name": "Dropped", "colors": [{"name": "Red", "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}}]});
    let path = temp_file("dropped.vclibrary", file.to_string().as_bytes());
    frame(&mut app, &ctx, vec![], &[&path], false);
    assert_eq!(app.ui.status, "Imported library Dropped");
    let lib = app.session.execute("library.get", &json!({})).unwrap();
    let colors: Vec<&str> = lib["colors"].as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
    assert_eq!((lib["name"].as_str(), colors), (Some("Dropped"), vec!["Red"]));
    assert_eq!((app.session.documents().len(), images(&app, 0)), (1, 0), "neither opened as a document nor placed");
}

#[test]
fn a_dropped_png_is_placed_in_the_middle_of_the_view() {
    let mut app = app();
    let ctx = egui::Context::default();
    laid_out(&mut app, &ctx);
    let path = temp_file("drop.png", &png(20, 10, 72.0));
    // Where egui last saw the pointer is stale during a desktop drag: it doesn't count.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(5.0, 5.0))], &[&path], false);
    let n = selected_image(&app);
    let want = app.view().unwrap().center;
    let c = n.geometric_bounds().unwrap().center();
    assert!((c.x - want.x).abs() < 1e-6 && (c.y - want.y).abs() < 1e-6, "{c:?} vs {want:?}");
    assert!(matches!(&n.kind, NodeKind::Image(im) if im.link.as_ref().map(|l| l.path.as_str()) == Some(path.as_str())), "linked by default");
    assert_eq!(app.session.documents().len(), 1, "placed, not opened");
    // Shift embeds.
    frame(&mut app, &ctx, vec![], &[&path], true);
    assert!(matches!(&selected_image(&app).kind, NodeKind::Image(im) if im.link.is_none()));
}

/// #472: a document dropped on the window opens as a new tab, not into the open document; the
/// pictures dropped with it are placed in the document that was open, before it opens.
#[test]
fn a_dropped_document_opens_as_a_tab_of_its_own() {
    let mut app = app();
    let ctx = egui::Context::default();
    laid_out(&mut app, &ctx);
    let svg = temp_file("dropped.svg", SVG.as_bytes());
    let pic = temp_file("with-it.png", &png(8, 8, 72.0));
    frame(&mut app, &ctx, vec![], &[&svg, &pic], false);
    assert_eq!(app.session.documents().len(), 2, "opened, not placed");
    assert_eq!(app.session.active_index(), Some(1), "the document opened is shown");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(svg.as_str()));
    assert_eq!((images(&app, 0), images(&app, 1)), (1, 0), "the picture is placed in the document that was open");
    assert_eq!(app.ui.recent_files.first(), Some(&svg));
}

#[test]
fn a_native_svg_drop_uses_file_open_even_with_a_path_only_handle() {
    let path = temp_file("path-only.svg", SVG.as_bytes());
    let mut dropped = app();
    let ctx = egui::Context::default();
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
    let ctx = egui::Context::default();
    laid_out(&mut app, &ctx);
    let svg = Arc::new(MockDrop { path: Default::default(), bytes: Ok(SVG.as_bytes().to_vec()) }) as egui::DroppedFileHandle;
    frame_files(&mut app, &ctx, vec![], vec![svg], false);
    assert_eq!(app.session.documents().len(), 2, "SVG bytes open as a document, not placed art");
    assert!(app.session.active().unwrap().path.is_none(), "no filesystem path was supplied");

    frame(&mut app, &ctx, vec![], &[], false);
    let image = Arc::new(MockDrop { path: Default::default(), bytes: Ok(png(6, 6, 72.0)) }) as egui::DroppedFileHandle;
    frame_files(&mut app, &ctx, vec![], vec![image], false);
    assert_eq!(app.session.documents().len(), 2, "an image is placed, not opened");
    assert_eq!(images(&app, 1), 1);
}

#[test]
fn multiple_svg_drops_open_individually() {
    let mut app = app();
    let ctx = egui::Context::default();
    laid_out(&mut app, &ctx);
    let a = temp_file("first.svg", SVG.as_bytes());
    let b = temp_file("second.svg", SVG.as_bytes());
    frame(&mut app, &ctx, vec![], &[&a, &b], false);
    assert_eq!(app.session.documents().len(), 3);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(b.as_str()));
    assert_eq!(&app.ui.recent_files[..2], &[b, a]);
}

#[test]
fn unsupported_and_unreadable_drops_leave_the_document_open() {
    let mut app = app();
    let ctx = egui::Context::default();
    laid_out(&mut app, &ctx);
    let invalid = temp_file("not-art.xyz", b"not an image or document");
    frame(&mut app, &ctx, vec![], &[&invalid], false);
    assert!(app.ui.status.contains("Couldn't place not-art.xyz"), "{}", app.ui.status);

    let invalid_svg = temp_file("broken.svg", b"<svg");
    frame(&mut app, &ctx, vec![], &[&invalid_svg], false);
    assert!(app.ui.status.contains("Couldn't open broken.svg"), "{}", app.ui.status);

    let missing = temp_file("unreadable.svg", SVG.as_bytes());
    std::fs::remove_file(&missing).unwrap();
    frame(&mut app, &ctx, vec![], &[&missing], false);
    assert!(app.ui.status.contains("Couldn't open unreadable.svg"), "{}", app.ui.status);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.ui.recent_files.is_empty(), "failed drops don't enter Open Recent");
}

#[test]
fn files_dropped_with_no_document_open_open_and_are_recent() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let path = temp_file("open-me.png", &png(8, 8, 72.0));
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
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![], &[], false);
    frame(&mut app, &ctx, vec![], &[], false);
    let rect = app.canvas_rect.expect("the canvas is laid out");
    let pos = rect.center() + vec2(60.0, -40.0);
    let drop_on_active = |app: &VectorcraftApp| match app.drop_target("late.png", Some(pos), false) {
        DropTarget::Place(d) => d,
        t => panic!("{t:?}"),
    };
    let arrive =
        |name: &str, drop| inbox.lock().unwrap().push(crate::place::PlaceArrival { name: name.into(), bytes: png(20, 10, 72.0), drop: Some(drop) });
    // Dropped on A; another document opens while the file is read.
    let on_a = drop_on_active(&app);
    let want = Xf::new(rect, app.view().unwrap()).to_doc(pos);
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
    let a = temp_file("a.png", &png(300, 150, 300.0));
    let b = temp_file("b.png", &png(10, 10, 72.0));
    app.run("file.place", json!({"paths": [a, b]})).unwrap();
    let d = app.ui.dialog.as_ref().expect("the Place dialog");
    assert_eq!(d.kind, crate::dialogs::place::KIND);
    assert!(d.bool("link") && !d.bool("__replace"), "Link on; Replace needs one file and one selected object");
    assert_eq!(d.fields["__info"][0], "300 × 150 px, 300 ppi, RGB (72 pt × 36 pt)");
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for label in ["Place", "a.png", "b.png", "Link", "Template", "Replace", "Cancel"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.tool_id(), "place");
    assert_eq!(app.session.tool_options()["count"], 2);
    // The cursor carries the current file's thumbnail and the number of files.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut shapes = vec![];
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::place::paint_cursor(&mut app, ui.ctx(), ui.painter(), Pos2::new(50.0, 50.0)));
        out.textures_delta.clear();
        shapes = out.shapes;
    }
    let mut text = String::new();
    let mut textured = false;
    for s in shapes {
        match s.shape {
            Shape::Text(t) => text.push_str(t.galley.text()),
            Shape::Mesh(m) => textured |= m.texture_id != egui::TextureId::default(),
            _ => {}
        }
    }
    assert!(textured, "the thumbnail");
    assert_eq!(text, "2", "the badge");
}

#[test]
fn one_file_places_centred_in_the_view_and_replace_swaps_the_selection() {
    let mut app = app();
    app.view_mut().unwrap().center = vectorcraft_geom::Point::new(120.0, 80.0);
    place_picked(&mut app, &temp_file("one.png", &png(300, 150, 300.0)));
    let d = app.ui.dialog.as_ref().expect("the Place dialog");
    assert_eq!(d.kind, crate::dialogs::place::KIND);
    assert!(d.bool("link") && !d.bool("__replace"), "Link on; Replace needs one selected object");
    assert_eq!(d.fields["__info"][0], "300 × 150 px, 300 ppi, RGB (72 pt × 36 pt)");
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for label in ["Place", "one.png", "Link", "Template", "Replace", "Cancel"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let first = selected_image(&app);
    assert_eq!(first.geometric_bounds().unwrap().center(), vectorcraft_geom::Point::new(120.0, 80.0), "centred in the view");
    // With one object selected, Replace applies.
    place_picked(&mut app, &temp_file("two.png", &png(40, 40, 144.0)));
    assert!(app.ui.dialog.as_ref().unwrap().bool("__replace"));
    app.ui.dialog.as_mut().unwrap().fields.insert("replace".into(), json!(true));
    app.ui.dialog.as_mut().unwrap().fields.insert("link".into(), json!(false));
    crate::dialogs::confirm(&mut app).unwrap();
    let second = selected_image(&app);
    assert_eq!(second.name.as_deref(), Some("two.png"));
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 1, "replaced");
    assert!(!app.ui.place_link, "Link is remembered");
}

#[test]
fn the_control_bar_shows_the_image_file_link_colour_mode_and_ppi() {
    let mut app = app();
    let path = temp_file("photo.png", &png(30, 30, 300.0));
    app.run("file.place", json!({"path": path})).unwrap();
    let text = crate::tests_labels::painted_text(&mut app, crate::chrome::control_bar);
    for s in ["Linked File", "photo.png", "RGB   PPI: 300"] {
        assert!(text.contains(s), "{s} in {text}");
    }
    let v: Value = app.run("file.place", json!({"path": path, "link": false})).unwrap();
    assert_eq!(v["linked"], false);
    let text = crate::tests_labels::painted_text(&mut app, crate::chrome::control_bar);
    assert!(text.contains("Embedded"), "{text}");
}

/// Click the Control bar's `label`.
fn click_label(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    use crate::tests_removeanchors::{at, click_control, control_frame};
    let p = at(&control_frame(app, ctx, vec![]), label);
    click_control(app, ctx, p);
}

#[test]
fn the_control_bar_and_properties_trace_a_selected_image() {
    use crate::panels::image_trace::{TRACE_BUTTON_W, selected_trace};
    let selected_preset = |app: &VectorcraftApp| selected_trace(app).map(|t| t.0);
    use crate::tests_removeanchors::{at, click_control, control_frame, has, properties_frame};
    let mut app = app();
    let data = vectorcraft_format::base64_encode(&png(30, 30, 72.0));
    app.run("file.place", json!({"name": "red.png", "dataBase64": data, "link": false})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
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
    click_control(&mut app, &ctx, at(&bar, "Image Trace") + vec2(TRACE_BUTTON_W / 2.0, 0.0));
    click_label(&mut app, &ctx, "6 Colors");
    assert_eq!(selected_preset(&app).as_deref(), Some("6 Colors"));
    // The button itself traces with the Default preset.
    app.run("edit.undo", json!({})).unwrap();
    click_label(&mut app, &ctx, "Image Trace");
    assert_eq!(selected_preset(&app).as_deref(), Some("Default"));
    // An Image Trace object: its preset (another traces it again), view, the panel and Expand.
    let bar = control_frame(&mut app, &ctx, vec![]);
    for s in ["Image Tracing", "Preset:", "Default", "View:", "Tracing Result", "Expand"] {
        assert!(has(&bar, s), "{s}");
    }
    let props = properties_frame(&mut app, 260.0);
    assert!(has(&props, "Release") && has(&props, "View:"));
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
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
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
    // A one-artboard document with a rectangle.
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    let bytes = vectorcraft_format::save_file(&src.active().unwrap().doc);
    let path = temp_file("badge.vectorcraft", &bytes);
    place_picked(&mut app, &path);
    let d = app.ui.dialog.as_ref().expect("the Place dialog");
    assert!(d.bool("link") && d.bool("__documents") && !d.bool("__others"), "Link on, for a document");
    // Link's tooltip says what it does for a document.
    assert!(crate::dialogs::place::link_tip(d).contains("editable copy"));
    crate::dialogs::confirm(&mut app).unwrap();
    let st = app.session.active().unwrap();
    let id = st.selection.objects[0];
    let vectorcraft_doc::NodeKind::PlacedDocument(p) = &st.doc.node(id).unwrap().kind else { panic!("not a placed document") };
    assert_eq!(p.link.path, path);
    // Edit Original opens the document in a new tab.
    let tabs = app.session.documents().len();
    app.run("links.editOriginal", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), tabs + 1);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path.as_str()));
    // A PNG keeps the image tooltip; a PNG and a document, both.
    let pic = temp_file("pic.png", &png(10, 10, 72.0));
    place_picked(&mut app, &pic);
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(!d.bool("__documents") && crate::dialogs::place::link_tip(d).contains("image file"));
    app.ui.dialog = None;
    let both = vec![pic.clone(), path.clone()];
    app.services.pick_open_multi = Some(Box::new(move || both.clone()));
    app.run("file.place", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(crate::dialogs::place::link_tip(d).contains("VectorCraft documents"));
}

/// Tool key `key` as the Control bar's Apply (Enter) and Cancel (Escape) send it.
fn tool_key(app: &mut VectorcraftApp, key: vectorcraft_tools::ToolKey) {
    let view = app.view_info();
    let r = app.session.tool_key(key, vectorcraft_tools::Mods::default(), view);
    crate::canvas::apply_requests(app, r);
}

/// #734: Crop Image shows a crop box on the image instead of cropping to the artboard at once (an
/// image inside the artboard had nothing to cut); Apply crops to the box in one undo step, Cancel
/// leaves the image whole, and both go back to the Selection tool.
#[test]
fn crop_image_shows_a_box_that_apply_crops_to() {
    use vectorcraft_tools::ToolKey;
    let mut app = app();
    place_picked(&mut app, &temp_file("crop.png", &png(100, 50, 72.0)));
    app.ui.dialog.as_mut().unwrap().fields.insert("link".into(), json!(false));
    crate::dialogs::confirm(&mut app).unwrap();
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

/// One headless frame of the whole window at `time` → how long the app asked to wait before the
/// next one.
fn frame_at(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> std::time::Duration {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(time), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
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
        if app.session.link_scan.is_some() {
            // The look runs on a worker thread, in real time.
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        *time = next;
    }
}

/// A file placed into a document whose last look found no links, then changed by another app
/// while the window sits idle in the background, follows Update Links: Automatically: the app
/// wakes itself to see the new link before the change, so the change isn't taken for the file as
/// first seen.
#[test]
fn a_file_placed_while_idle_follows_its_changes() {
    let mut app = app();
    app.run("prefs.set", json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    idle(&mut app, &ctx, &mut time, 3.0, &|_| false);
    // A look finds the new document without links: no wakes of its own for it.
    assert_eq!(frame_at(&mut app, &ctx, time), std::time::Duration::MAX, "{:?}", ctx.repaint_causes());
    // Half a second later, before the next look is due, a file is placed. The place itself is a
    // frame (as any command); then nothing happens for 3 s.
    time += 0.5;
    let path = temp_file("idle-watch.png", &png(40, 30, 72.0));
    app.run("file.place", json!({"path": path})).unwrap();
    let id = app.session.active().unwrap().selection.objects[0];
    idle(&mut app, &ctx, &mut time, 3.0, &|_| false);
    std::thread::sleep(std::time::Duration::from_millis(30));
    let changed = png(60, 30, 72.0);
    std::fs::write(&path, &changed).unwrap();
    let hash = vectorcraft_doc::links::hash_bytes(&changed);
    let updated = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        matches!(&st.doc.node(id).unwrap().kind, NodeKind::Image(im) if im.link.as_ref().and_then(|l| l.hash.as_deref()) == Some(hash.as_str()))
    };
    assert!(idle(&mut app, &ctx, &mut time, 10.0, &updated), "the changed file was never read again");
}
