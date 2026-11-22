//! The Links panel, its flyout, Link Info, Edit Original / Show in Folder, the Placement Options
//! dialog and the Properties panel's image section, drawn headless.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::tests_labels::painted_text;

/// A fresh folder for one test (removed when dropped).
struct Folder(std::path::PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vectorcraft-ui-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 40, 255]))).into();
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

/// A document with a linked `linked.png` and an embedded `embedded.png` (the selection).
fn app(dir: &Folder) -> (VectorcraftApp, u64, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let a = dir.file("linked.png", &png(300, 200));
    let b = dir.file("embedded.png", &png(40, 20));
    let linked = app.run("file.place", json!({"path": a, "at": [100, 100]})).unwrap()["ids"][0].as_u64().unwrap();
    let embedded = app.run("file.place", json!({"path": b, "at": [300, 200], "link": false})).unwrap()["ids"][0].as_u64().unwrap();
    (app, linked, embedded)
}

fn panel(app: &mut VectorcraftApp) -> String {
    painted_text(app, super::links::show)
}

/// Click the row named `name` in the panel with `mods` held (frames at `time` and just after: a
/// second apart, two clicks are not a double-click).
fn click_row(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, mods: egui::Modifiers, time: f64) {
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>, time: f64| {
        let events = std::iter::once(egui::Event::ModifiersChanged(mods)).chain(events).collect();
        super::tests_appearance::frame_raw(ctx, app, egui::RawInput { events, time: Some(time), ..Default::default() }, super::links::show)
    };
    let at = super::tests_appearance::text_rect(&frame(app, vec![], time), name).center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: mods };
    for (i, e) in [egui::Event::PointerMoved(at), button(true), button(false)].into_iter().enumerate() {
        frame(app, vec![e], time + 0.01 * (i + 1) as f64);
    }
}

/// The selected objects' ids, in ascending order.
fn selected(app: &VectorcraftApp) -> Vec<u64> {
    let mut ids: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect();
    ids.sort_unstable();
    ids
}

#[test]
fn the_panel_lists_the_images_with_link_info_for_the_selected_one() {
    let dir = Folder::new("panel");
    let (mut app, linked, _) = app(&dir);
    let text = panel(&mut app);
    assert!(text.contains("linked.png") && text.contains("embedded.png"), "{text}");
    // The selection is the embedded image.
    assert!(text.contains("Embedded") && text.contains("Effective PPI"), "{text}");
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    let text = panel(&mut app);
    assert!(text.contains("Linked") && text.contains("300 × 200 px") && text.contains("Location"), "{text}");
    // The flyout draws with every item.
    let menu = painted_text(&mut app, super::links::menu);
    for item in [
        "Relink to Folder…",
        "Embed Image(s)",
        "Placement Options…",
        "Show Missing",
        "Sort by Status",
        "Sort by Size",
        "Large Thumbnails",
        "Show in Folder",
    ] {
        assert!(menu.contains(item), "{item}: {menu}");
    }
}

/// Click the flyout item `name`, checked or not (frames at `time` and just after).
fn click_menu_item(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, time: f64) {
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>, time: f64| {
        super::tests_appearance::frame_raw(ctx, app, egui::RawInput { events, time: Some(time), ..Default::default() }, super::links::menu)
    };
    let texts = frame(app, vec![], time);
    let item = texts.iter().find(|(t, _)| t.trim_start_matches('✓').trim() == name);
    let at = item.unwrap_or_else(|| panic!("`{name}` not drawn: {texts:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for (i, e) in [egui::Event::PointerMoved(at), button(true), button(false)].into_iter().enumerate() {
        frame(app, vec![e], time + 0.01 * (i + 1) as f64);
    }
}

/// Opening the panel on the selected image (the Control bar's Linked File or Embedded label) opens
/// Link Info when it was closed, and sets a Show filter that leaves out the image's row back to
/// Show All; a filter that lists the row stays.
#[test]
fn opening_the_panel_on_the_selected_image_shows_its_row_and_link_info() {
    let dir = Folder::new("show-selected");
    let (mut app, linked, embedded) = app(&dir);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let text = |app: &mut VectorcraftApp, time: f64| {
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        let texts = super::tests_appearance::frame_raw(&ctx, app, raw, super::links::show);
        texts.into_iter().map(|(t, _)| t).collect::<Vec<_>>().join("\n")
    };
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    click_menu_item(&mut app, &ctx, "Show Link Info", 1.0);
    click_menu_item(&mut app, &ctx, "Show Embedded", 2.0);
    let before = text(&mut app, 3.0);
    assert!(!before.contains("linked.png") && !before.contains("Location"), "the filter hides the row, Link Info is closed: {before}");
    app.run("window.panel.close", json!({"panel": super::links::ID})).ok();
    super::links::show_selected_image(&mut app, &ctx);
    assert!(crate::panel_docking::visible(&app.ui, super::links::ID), "status: {}", app.ui.status);
    let after = text(&mut app, 4.0);
    assert!(after.contains("linked.png") && after.contains("embedded.png"), "Show All lists every row: {after}");
    assert!(after.contains("Location") && after.contains("300 × 200 px"), "Link Info shows the linked image: {after}");
    // An embedded image under Show Embedded keeps the filter.
    app.run("select.set", json!({"ids": [embedded]})).unwrap();
    click_menu_item(&mut app, &ctx, "Show Link Info", 5.0);
    click_menu_item(&mut app, &ctx, "Show Embedded", 6.0);
    super::links::show_selected_image(&mut app, &ctx);
    assert!(crate::panel_docking::visible(&app.ui, super::links::ID), "an open panel stays open");
    let embedded_only = text(&mut app, 7.0);
    assert!(embedded_only.contains("embedded.png") && !embedded_only.contains("linked.png"), "Show Embedded stays: {embedded_only}");
    assert!(embedded_only.contains("Effective PPI"), "Link Info shows the embedded image: {embedded_only}");
}

/// Opening the panel on the selected image scrolls the list to the image's row when it is out of
/// view.
#[test]
fn opening_the_panel_on_the_selected_image_scrolls_to_its_row() {
    let dir = Folder::new("show-selected-scroll");
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    // The panel lists the image placed first last, below the list's view.
    let mut first = None;
    for i in 0..20 {
        let path = dir.file(&format!("img{i:02}.png"), &png(10, 10));
        let id = app.run("file.place", json!({"path": path, "at": [10 + 15 * i, 100]})).unwrap()["ids"][0].as_u64().unwrap();
        first.get_or_insert(id);
    }
    app.run("select.set", json!({"ids": [first]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // How many times the first image's name is drawn: its row while in view, and Link Info.
    let drawn = |app: &mut VectorcraftApp, time: f64| {
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        let texts = super::tests_appearance::frame_raw(&ctx, app, raw, super::links::show);
        texts.iter().filter(|(t, _)| t == "img00.png").count()
    };
    let before = drawn(&mut app, 1.0);
    assert_eq!(drawn(&mut app, 2.0), before, "the list stays at the top");
    super::links::show_selected_image(&mut app, &ctx);
    // The list scrolls over the frames after the one that asks.
    for time in [3.0, 4.0, 5.0] {
        drawn(&mut app, time);
    }
    assert_eq!(drawn(&mut app, 6.0), before + 1, "the row scrolled into view");
}

/// Opening the panel on the selected image draws a list read after the request, so a row that a
/// file change brought into the Show filter since the list was last read is there to scroll to.
#[test]
fn opening_the_panel_on_the_selected_image_reads_the_list_again() {
    let dir = Folder::new("show-selected-fresh");
    let (mut app, linked, _) = app(&dir);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // How many times the linked image's name is drawn: its row, and Link Info.
    let drawn = |app: &mut VectorcraftApp, time: f64| {
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        let texts = super::tests_appearance::frame_raw(&ctx, app, raw, super::links::show);
        texts.iter().filter(|(t, _)| t == "linked.png").count()
    };
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    click_menu_item(&mut app, &ctx, "Show Missing", 1.0);
    assert_eq!(drawn(&mut app, 2.0), 1, "Link Info only: nothing is missing yet");
    std::fs::remove_file(dir.0.join("linked.png")).unwrap();
    // Within the time the panel keeps its list before it looks at the files again.
    super::links::show_selected_image(&mut app, &ctx);
    assert_eq!(drawn(&mut app, 2.1), 2, "the missing image's row and its Link Info are drawn");
}

/// The Show filter is set back to Show All from the rows the panel draws, not from the files as
/// they were when the panel was asked to open: a missing image restored before the list is drawn
/// still gets its row.
#[test]
fn opening_the_panel_on_the_selected_image_decides_the_filter_from_the_drawn_rows() {
    let dir = Folder::new("show-selected-drawn");
    let (mut app, linked, _) = app(&dir);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let drawn = |app: &mut VectorcraftApp, time: f64| {
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        let texts = super::tests_appearance::frame_raw(&ctx, app, raw, super::links::show);
        texts.iter().filter(|(t, _)| t == "linked.png").count()
    };
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    click_menu_item(&mut app, &ctx, "Show Missing", 1.0);
    let bytes = std::fs::read(dir.0.join("linked.png")).unwrap();
    std::fs::remove_file(dir.0.join("linked.png")).unwrap();
    super::links::show_selected_image(&mut app, &ctx);
    std::fs::write(dir.0.join("linked.png"), bytes).unwrap();
    assert_eq!(drawn(&mut app, 2.0), 2, "the restored image's row and its Link Info are drawn");
    assert_eq!(super::pstate::<Option<(u64, u64)>>(&ctx, "links-scroll-to"), None, "the scroll request is used on the row");
}

/// A request to show the selected image applies only in the document it was made in: drawn for
/// another document, the panel keeps its Show filter and drops the request.
#[test]
fn opening_the_panel_on_the_selected_image_leaves_another_document_alone() {
    let dir = Folder::new("show-selected-other-doc");
    let (mut app, linked, _) = app(&dir);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let text = |app: &mut VectorcraftApp, time: f64| {
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        let texts = super::tests_appearance::frame_raw(&ctx, app, raw, super::links::show);
        texts.into_iter().map(|(t, _)| t).collect::<Vec<_>>().join("\n")
    };
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    click_menu_item(&mut app, &ctx, "Show Missing", 1.0);
    super::links::show_selected_image(&mut app, &ctx);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let other = text(&mut app, 2.0);
    assert!(other.contains("Nothing matches the Show filter."), "the other document keeps Show Missing: {other}");
    assert_eq!(super::pstate::<Option<(u64, u64)>>(&ctx, "links-scroll-to"), None, "the request is dropped");
}

/// A frame that fetches nothing keeps the stored rows and Link Info as they are, without a copy; the
/// two-second recheck, another first selected image and an edit each store a new cache.
#[test]
fn frames_that_fetch_nothing_reuse_the_cached_rows() {
    let dir = Folder::new("cache");
    let (mut app, linked, _) = app(&dir);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, time: f64| {
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        let text: Vec<String> = super::tests_appearance::frame_raw(&ctx, app, raw, super::links::show).into_iter().map(|(t, _)| t).collect();
        (super::pstate::<std::sync::Arc<super::links::Cache>>(&ctx, "links-cache"), text)
    };
    let (first, text) = frame(&mut app, 1.0);
    assert!(text.iter().any(|t| t == "linked.png") && text.iter().any(|t| t == "40 × 20 px"), "{text:?}");
    let (same, text) = frame(&mut app, 1.5);
    assert!(std::sync::Arc::ptr_eq(&first, &same), "a frame that fetches nothing stores no new cache");
    assert!(text.iter().any(|t| t == "linked.png") && text.iter().any(|t| t == "40 × 20 px"), "{text:?}");
    let (rechecked, _) = frame(&mut app, 3.6);
    assert!(!std::sync::Arc::ptr_eq(&same, &rechecked), "the files are looked at again after two seconds");
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    let (picked, text) = frame(&mut app, 3.7);
    assert!(!std::sync::Arc::ptr_eq(&rechecked, &picked), "Link Info follows the first selected image");
    assert!(text.iter().any(|t| t == "300 × 200 px"), "{text:?}");
    app.run("edit.clear", json!({"ids": [linked]})).unwrap();
    let (edited, text) = frame(&mut app, 3.8);
    assert!(!std::sync::Arc::ptr_eq(&picked, &edited), "an edit lists the rows again");
    assert!(!text.iter().any(|t| t == "linked.png") && text.iter().any(|t| t == "embedded.png"), "{text:?}");
}

/// Copy Info copies the file names of the selected links, or of `ids` in the order given, one per
/// line; it is off with nothing selected, and a bad `ids` leaves the clipboard as it was.
#[test]
fn copy_info_copies_the_file_names_of_the_links() {
    let dir = Folder::new("copy-info");
    let (mut app, linked, embedded) = app(&dir);
    let menu = painted_text(&mut app, super::links::menu);
    assert!(menu.contains("Copy Info"), "{menu}");
    // The selection is the embedded image: its row's name.
    assert!(crate::menus::enabled(&app, "links.copyInfo"));
    let r = app.run("links.copyInfo", json!({})).unwrap();
    assert_eq!(r["names"], json!(["embedded.png"]));
    assert_eq!(app.clipboard_out.take().as_deref(), Some("embedded.png"));
    // Both selected: in the order the list gives them (the top image first).
    app.run("select.set", json!({"ids": [linked, embedded]})).unwrap();
    assert_eq!(app.run("links.copyInfo", json!({})).unwrap()["text"], "embedded.png\nlinked.png");
    assert_eq!(app.clipboard_out.take().as_deref(), Some("embedded.png\nlinked.png"));
    // `ids` in the order given, whatever is selected.
    app.run("select.none", json!({})).unwrap();
    let r = app.run("links.copyInfo", json!({"ids": [linked, embedded]})).unwrap();
    assert_eq!(r["names"], json!(["linked.png", "embedded.png"]));
    assert_eq!(app.clipboard_out.take().as_deref(), Some("linked.png\nembedded.png"));
    // Errors copy nothing.
    let rect = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap();
    app.run("select.none", json!({})).unwrap();
    for bad in
        [json!({"ids": [linked, rect]}), json!({"ids": [linked, 999_999]}), json!({"ids": "all"}), json!({"ids": [linked, -1]}), json!({"ids": []})]
    {
        assert!(app.run("links.copyInfo", bad.clone()).is_err(), "{bad}");
        assert!(app.clipboard_out.is_none(), "{bad}");
    }
    assert!(!crate::menus::enabled(&app, "links.copyInfo"));
    assert!(app.run("links.copyInfo", json!({})).is_err());
    assert!(app.clipboard_out.is_none());
}

/// Shift-click selects the images of the rows from the last row clicked without Shift; Ctrl/Cmd-click
/// adds or removes one image, and Ctrl/Cmd-Shift-click adds the rows' images to the selection.
#[test]
fn shift_click_selects_a_range_of_links_and_ctrl_click_toggles_one() {
    let dir = Folder::new("range");
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    for (i, name) in ["a.png", "b.png", "c.png", "d.png"].into_iter().enumerate() {
        let path = dir.file(name, &png(20, 20));
        app.run("file.place", json!({"path": path, "at": [50 + 80 * i, 100]})).unwrap();
    }
    let list = app.run("links.list", json!({})).unwrap();
    let rows: Vec<(String, u64)> =
        list["links"].as_array().unwrap().iter().map(|r| (r["name"].as_str().unwrap().to_string(), r["id"].as_u64().unwrap())).collect();
    assert_eq!(rows.len(), 4, "{list}");
    let id = |rows_at: &[usize]| {
        let mut ids: Vec<u64> = rows_at.iter().map(|&i| rows[i].1).collect();
        ids.sort_unstable();
        ids
    };
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    click_row(&mut app, &ctx, &rows[0].0, egui::Modifiers::NONE, 1.0);
    assert_eq!(selected(&app), id(&[0]));
    click_row(&mut app, &ctx, &rows[2].0, egui::Modifiers::SHIFT, 2.0);
    assert_eq!(selected(&app), id(&[0, 1, 2]), "Shift-click selects the rows from the one clicked before");
    click_row(&mut app, &ctx, &rows[1].0, egui::Modifiers::COMMAND, 3.0);
    assert_eq!(selected(&app), id(&[0, 2]), "Ctrl/Cmd-click removes one image");
    click_row(&mut app, &ctx, &rows[3].0, egui::Modifiers::SHIFT | egui::Modifiers::COMMAND, 4.0);
    assert_eq!(selected(&app), id(&[0, 1, 2, 3]), "Ctrl/Cmd-Shift-click adds the rows from the one clicked before");
    click_row(&mut app, &ctx, &rows[0].0, egui::Modifiers::SHIFT, 5.0);
    assert_eq!(selected(&app), id(&[0, 1]), "Shift-click replaces the selection with the rows");
    click_row(&mut app, &ctx, &rows[3].0, egui::Modifiers::NONE, 6.0);
    assert_eq!(selected(&app), id(&[3]));
}

/// A linked image kept only in a symbol's art has a row of its own: a click picks it in the panel
/// (the canvas selection is cleared), Link Info shows it, Ctrl/Cmd-click adds an image in the
/// layers, a change of the canvas selection drops the pick, and Go To clears the selection and
/// says it has no place on the canvas.
#[test]
fn a_link_inside_a_symbol_is_listed_and_picked_in_the_panel() {
    let dir = Folder::new("nested");
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let inside = dir.file("inside.png", &png(30, 20));
    let id = app.run("file.place", json!({"path": inside, "at": [100, 100]})).unwrap()["ids"][0].as_u64().unwrap();
    app.run("symbol.new", json!({"name": "Mark", "ids": [id]})).unwrap();
    app.run("edit.clear", json!({"ids": selected(&app)})).unwrap();
    let outside = dir.file("layer.png", &png(20, 20));
    let layer = app.run("file.place", json!({"path": outside, "at": [300, 100]})).unwrap()["ids"][0].as_u64().unwrap();
    let list = app.run("links.list", json!({})).unwrap();
    let row = list["links"].as_array().unwrap().iter().find(|r| r["name"] == "inside.png").unwrap_or_else(|| panic!("{list}")).clone();
    assert_eq!(row["owner"]["kind"], "symbol", "{list}");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let shown = |app: &mut VectorcraftApp, time: f64| -> Vec<String> {
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        super::tests_appearance::frame_raw(&ctx, app, raw, super::links::show).into_iter().map(|(t, _)| t).collect()
    };
    assert!(shown(&mut app, 0.5).iter().any(|t| t == "20 × 20 px"), "Link Info of the selected image in the layers");
    click_row(&mut app, &ctx, "inside.png", egui::Modifiers::NONE, 1.0);
    assert!(selected(&app).is_empty(), "the canvas can't select it");
    assert!(shown(&mut app, 1.5).iter().any(|t| t == "30 × 20 px"), "Link Info of the symbol's image");
    // Copy Info without ids copies the row picked in the panel.
    app.ui_ctx = Some(ctx.clone());
    assert!(crate::menus::enabled(&app, "links.copyInfo"));
    assert_eq!(app.run("links.copyInfo", json!({})).unwrap()["text"], "inside.png");
    assert_eq!(app.clipboard_out.take().as_deref(), Some("inside.png"));
    click_row(&mut app, &ctx, "layer.png", egui::Modifiers::COMMAND, 2.0);
    assert_eq!(selected(&app), [layer]);
    // Both rows are selected until the canvas selection changes: then the panel's pick goes too.
    click_row(&mut app, &ctx, "layer.png", egui::Modifiers::COMMAND, 3.0);
    assert!(selected(&app).is_empty());
    assert!(shown(&mut app, 3.5).iter().any(|t| t == "30 × 20 px"), "the symbol's image is still picked");
    app.run("select.set", json!({"ids": [layer]})).unwrap();
    assert!(shown(&mut app, 4.0).iter().any(|t| t == "20 × 20 px"));
    app.run("select.set", json!({"ids": []})).unwrap();
    let text = shown(&mut app, 4.5);
    assert!(!text.iter().any(|t| t == "30 × 20 px" || t == "20 × 20 px"), "nothing is picked: {text:?}");
    // Go To clears the selection and says the link has no place on the canvas.
    app.run("select.set", json!({"ids": [layer]})).unwrap();
    super::links::go_to(&mut app, row["id"].as_u64().unwrap());
    assert_eq!(app.ui.status, "The link is inside a symbol, graph design, pattern or opacity mask and has no place on the canvas");
    assert!(selected(&app).is_empty(), "the image in the layers is no longer selected");
}

/// The tooltip of a row outside the layer tree says where the image is stored, so two rows of the
/// same file in different symbols or masks can be told apart.
#[test]
fn a_nested_rows_tooltip_names_what_holds_it() {
    let tip = |owner: serde_json::Value, mask: Option<&str>| {
        let row = json!({"id": 9, "name": "a.png", "status": "ok", "path": "/x/a.png", "owner": owner});
        super::links::status_tip(&row, mask.map(str::to_string))
    };
    assert_eq!(tip(json!({"kind": "symbol", "name": "Mark"}), None), "/x/a.png\nSymbols: Mark");
    assert_eq!(tip(json!({"kind": "pattern", "name": "Dots"}), None), "/x/a.png\nPattern: Dots");
    assert_eq!(tip(json!({"kind": "graphDesign", "name": "Bar"}), None), "/x/a.png\nGraph Design: Bar");
    assert_eq!(tip(json!({"kind": "brush", "name": "Leaf", "index": 2}), None), "/x/a.png\nBrushes: Leaf");
    assert_eq!(tip(json!({"kind": "layers", "maskOf": 7}), Some("<Ellipse>")), "/x/a.png\nOpacity Mask: <Ellipse>");
    assert_eq!(tip(json!({"kind": "symbol", "name": "Mark", "maskOf": 7}), None), "/x/a.png\nSymbols: Mark\nOpacity Mask: 7");
    // A row in the layers says only its file.
    let row = json!({"id": 9, "name": "a.png", "status": "ok", "path": "/x/a.png"});
    assert_eq!(super::links::status_tip(&row, None), "/x/a.png");
    // The labels take the UI language's colon (#7991).
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    crate::i18n::set_current(fr);
    let french = tip(json!({"kind": "symbol", "name": "Mark", "maskOf": 7}), None);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (symbols, mask) = (crate::i18n::tr(fr, "Symbols"), crate::i18n::tr(fr, "Opacity Mask"));
    assert_eq!(french, format!("/x/a.png\n{symbols}\u{a0}: Mark\n{mask}\u{a0}: 7"));
}

/// The tooltip of an image in the opacity mask of an object inside a symbol names that object,
/// which the panel finds among the objects it collected with the list.
#[test]
fn a_mask_rows_tooltip_names_its_object_inside_a_symbol() {
    let dir = Folder::new("mask-owner");
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let rect = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap();
    let mask = dir.file("mask.png", &png(20, 20));
    let mask = app.run("file.place", json!({"path": mask, "at": [40, 40]})).unwrap()["ids"][0].as_u64().unwrap();
    app.run("transparency.makeOpacityMask", json!({"ids": [rect, mask]})).unwrap();
    app.run("symbol.new", json!({"name": "Mark", "ids": [rect]})).unwrap();
    app.run("edit.clear", json!({"ids": selected(&app)})).unwrap();
    let list = app.run("links.list", json!({})).unwrap();
    let row = list["links"].as_array().unwrap().iter().find(|r| r["name"] == "mask.png").unwrap_or_else(|| panic!("{list}")).clone();
    assert_eq!(row["owner"]["kind"], "symbol", "{list}");
    let owner = vectorcraft_doc::NodeId(row["owner"]["maskOf"].as_u64().unwrap_or_else(|| panic!("{list}")));
    let doc = app.session.active().unwrap().doc.clone();
    assert!(doc.node(owner).is_none(), "the masked object is outside the layer tree");
    let name = super::layers::shown_name(&app, doc.stored_node(owner).unwrap().0, crate::i18n::current());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>, time: f64| {
        super::tests_appearance::frame_raw(&ctx, app, egui::RawInput { events, time: Some(time), ..Default::default() }, super::links::show)
    };
    let at = super::tests_appearance::text_rect(&frame(&mut app, vec![], 1.0), "mask.png").center();
    frame(&mut app, vec![egui::Event::PointerMoved(at)], 2.0);
    frame(&mut app, vec![], 4.0);
    let text: Vec<String> = frame(&mut app, vec![], 4.1).into_iter().map(|(t, _)| t).collect();
    let want = format!("Opacity Mask: {name}");
    assert!(text.iter().any(|t| t.contains(&want)), "{want} in {text:?}");
}

/// The row of an image in a stored brush's art finds its object for the thumbnail, though that art
/// is set on the document only while a command uses it.
#[test]
fn a_row_in_a_stored_brushs_art_finds_its_object() {
    let dir = Folder::new("brush-art");
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let leaf = dir.file("leaf.png", &png(30, 20));
    let id = app.run("file.place", json!({"path": leaf, "at": [100, 100]})).unwrap()["ids"][0].as_u64().unwrap();
    app.run("brush.new", json!({"type": "scatter", "name": "Leaf", "ids": [id]})).unwrap();
    app.run("edit.clear", json!({"ids": [id]})).unwrap();
    let list = app.run("links.list", json!({})).unwrap();
    let row = list["links"].as_array().unwrap().iter().find(|r| r["name"] == "leaf.png").unwrap_or_else(|| panic!("{list}")).clone();
    assert_eq!(row["owner"]["kind"], "brush", "{list}");
    let row = row["id"].as_u64().unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.mounted_art.is_empty(), "the brush's art is not set on the document");
    let found = super::links::stored_objects(doc, [row].into());
    assert_eq!(found.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [row]);
    assert!(matches!(found[0].1.kind, vectorcraft_doc::NodeKind::Image(_)), "{:?}", found[0].1.kind);
}

#[test]
fn an_empty_document_shows_how_to_add_images() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    assert!(panel(&mut app).contains("No images"));
}

#[test]
fn edit_original_and_show_in_folder_hand_the_file_to_the_system() {
    let dir = Folder::new("open");
    let (mut app, linked, embedded) = app(&dir);
    let opened: Rc<RefCell<Vec<(String, bool)>>> = Rc::default();
    let (o, r) = (opened.clone(), opened.clone());
    app.services.open_file = Some(Box::new(move |p: &str| {
        o.borrow_mut().push((p.to_string(), false));
        Ok(())
    }));
    app.services.reveal = Some(Box::new(move |p: &str| {
        r.borrow_mut().push((p.to_string(), true));
        Ok(())
    }));
    assert!(!crate::menus::enabled(&app, "links.editOriginal"), "the embedded image is selected");
    assert!(app.run("links.editOriginal", json!({})).is_err());
    app.run("select.set", json!({"ids": [linked, embedded]})).unwrap();
    assert!(crate::menus::enabled(&app, "links.editOriginal"));
    let r = app.run("links.editOriginal", json!({})).unwrap();
    let path = r["path"].as_str().unwrap().to_string();
    assert!(path.ends_with("linked.png"));
    app.run("links.reveal", json!({"id": linked})).unwrap();
    assert_eq!(*opened.borrow(), [(path.clone(), false), (path, true)]);
    // Without the services (the web): an error, nothing opened.
    (app.services.open_file, app.services.reveal) = (None, None);
    assert!(app.run("links.reveal", json!({"id": linked})).is_err());
    assert!(app.run("links.editOriginal", json!({"id": linked})).is_err());
}

/// #5236: with Preferences › File Handling › Use System Defaults for Edit Original off, Edit
/// Original opens a raster image in the app that made it, PhotoCraft, and falls back to the
/// system's default app when PhotoCraft isn't installed or the file isn't a raster image.
#[test]
fn edit_original_opens_raster_images_in_photocraft_without_system_defaults() {
    /// What each open asked for: (`true` for PhotoCraft, the path).
    type Opened = Rc<RefCell<Vec<(bool, String)>>>;
    let dir = Folder::new("with");
    let (mut app, linked, _) = app(&dir);
    let opened: Opened = Rc::default();
    let installed = Rc::new(Cell::new(true));
    let (o, w, i) = (opened.clone(), opened.clone(), installed.clone());
    app.services.open_file = Some(Box::new(move |p: &str| {
        o.borrow_mut().push((false, p.to_string()));
        Ok(())
    }));
    app.services.open_in_photocraft = Some(Box::new(move |p: &str| {
        if i.get() {
            w.borrow_mut().push((true, p.to_string()));
        }
        Ok(i.get())
    }));
    // On (the default): the system's app.
    let path = app.run("links.editOriginal", json!({"id": linked})).unwrap()["path"].as_str().unwrap().to_string();
    app.run("prefs.set", json!({"key": "editOriginalSystemDefaults", "value": false})).unwrap();
    app.run("links.editOriginal", json!({"id": linked})).unwrap();
    installed.set(false);
    app.run("links.editOriginal", json!({"id": linked})).unwrap();
    installed.set(true);
    // A file that isn't a raster image goes to the system's app.
    crate::io::open_original(&mut app, "/Art/plan.emf").unwrap();
    assert_eq!(*opened.borrow(), [(false, path.clone()), (true, path.clone()), (false, path), (false, "/Art/plan.emf".to_string())]);
    // PhotoCraft failing to open it is said, not hidden behind the system's app.
    app.services.open_in_photocraft = Some(Box::new(|_: &str| Err("PhotoCraft didn't start".to_string())));
    assert!(app.run("links.editOriginal", json!({"id": linked})).is_err());
    assert_eq!(opened.borrow().len(), 4);
}

#[test]
fn edit_original_with_chosen_editor_still_refuses_automation() {
    let dir = Folder::new("chosen-automation");
    let (mut app, linked, _) = app(&dir);
    app.ui.original_editor = serde_json::from_value(json!({"path": "/Apps/Editor.exe"})).unwrap();
    app.session.prefs.edit_original_system_defaults = false;
    app.services.open_in_application = Some(Box::new(|_, _| panic!("automation must not launch an editor")));
    app.ui.answering_control = true;
    assert!(app.run("links.editOriginal", json!({"id": linked})).unwrap_err().contains("only the person"));
    app.ui.answering_control = false;
    app.synthetic_frame = true;
    assert!(app.run("links.editOriginal", json!({"id": linked})).unwrap_err().contains("only the person"));
}

/// Edit Original hands only regular image and document files to the system's default app.
#[test]
fn edit_original_opens_image_and_document_files_only() {
    let dir = Folder::new("kinds");
    for f in [
        "photo.png",
        "Art.SVG",
        "scan.dib",
        "photo.PDD",
        "run.exe",
        "run.bat",
        "photo.png.lnk",
        "page.hta",
        "app.desktop",
        "art.affinity",
        "scan.jpeg2000",
    ] {
        dir.file(f, b"");
    }
    for f in ["Tool.app", "folder.png"] {
        std::fs::create_dir_all(dir.0.join(f)).unwrap();
    }
    let ok = |f: &str| super::links::openable_original(&dir.0.join(f).to_string_lossy()).is_ok();
    assert!(ok("photo.png") && ok("Art.SVG"));
    assert!(ok("scan.dib") && ok("photo.PDD"), "BMP and Photoshop files by their other names");
    // A format's id is not one of its extensions.
    for f in [
        "run.exe",
        "run.bat",
        "photo.png.lnk",
        "page.hta",
        "app.desktop",
        "art.affinity",
        "scan.jpeg2000",
        "Tool.app",
        "folder.png",
        "missing.png",
        "",
    ] {
        assert!(!ok(f), "{f}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.0.join("app.desktop"), dir.0.join("link.png")).unwrap();
        assert!(!ok("link.png"), "an image name linking to another kind of file");
        // The command refuses a linked image that became such a link, without opening anything.
        let (mut app, linked, _) = app(&dir);
        let opened: Rc<RefCell<Vec<String>>> = Rc::default();
        let o = opened.clone();
        app.services.open_file = Some(Box::new(move |p: &str| {
            o.borrow_mut().push(p.to_string());
            Ok(())
        }));
        let png = dir.0.join("linked.png");
        std::fs::remove_file(&png).unwrap();
        std::os::unix::fs::symlink(dir.0.join("app.desktop"), &png).unwrap();
        let e = app.run("links.editOriginal", json!({"id": linked})).unwrap_err();
        assert!(e.contains("image and document files only"), "{e}");
        assert!(opened.borrow().is_empty());
    }
}

#[test]
fn placement_options_dialog_sets_the_options() {
    let dir = Folder::new("dialog");
    let (mut app, linked, _) = app(&dir);
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    assert!(crate::menus::enabled(&app, "ui.placementOptionsDialog"));
    app.run("ui.placementOptionsDialog", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("preserve"), d.str("align")), ("placementOptions", "bounds".into(), "center".into()));
    let text = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(text.contains("Clip to Bounding Box") && text.contains("Preserve"), "{text}");
    let fields = &mut app.ui.dialog.as_mut().unwrap().fields;
    fields.insert("preserve".into(), json!("fit"));
    fields.insert("align".into(), json!("bottomRight"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let r = app.run("links.placementOptions", json!({"ids": [linked]})).unwrap();
    assert_eq!(r["placement"], json!({"preserve": "fit", "align": "bottomRight", "clip": false}));
}

#[test]
fn properties_shows_the_selected_image_and_its_link_actions() {
    let dir = Folder::new("props");
    let (mut app, linked, _) = app(&dir);
    let text = painted_text(&mut app, super::properties::show);
    assert!(text.contains("Embedded Image") && text.contains("embedded.png") && text.contains("Unembed…"), "{text}");
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    let text = painted_text(&mut app, super::properties::show);
    assert!(text.contains("Linked File") && text.contains("Edit Original") && text.contains("Embed"), "{text}");
}

/// Unembed… acts on several selected embedded images: it asks for one folder, writes a file for
/// each image there and links each one to its file; with no folder picked, nothing changes.
#[test]
fn unembed_writes_several_images_to_one_picked_folder() {
    let dir = Folder::new("unembed-several");
    let (mut app, _, embedded) = app(&dir);
    let c = dir.file("other.png", &png(30, 10));
    let other = app.run("file.place", json!({"path": c, "at": [50, 50], "link": false})).unwrap()["ids"][0].as_u64().unwrap();
    assert!(super::links::can_unembed(&app, 1));
    assert!(!super::links::can_unembed(&app, 0));
    assert!(!super::links::can_unembed(&app, 2), "no folder to pick");
    let out = dir.0.join("out");
    std::fs::create_dir_all(&out).unwrap();
    // No folder picked: nothing written or linked.
    app.services.pick_folder = Some(Box::new(|| None));
    assert!(super::links::can_unembed(&app, 2));
    super::links::unembed_several(&mut app, vec![embedded, other]);
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    assert_eq!(app.run("links.info", json!({"id": embedded})).unwrap()["status"], "embedded");
    let picked = out.to_string_lossy().into_owned();
    let answer = picked.clone();
    app.services.pick_folder = Some(Box::new(move || Some(answer.clone())));
    super::links::unembed_several(&mut app, vec![embedded, other]);
    for (id, name) in [(embedded, "embedded.png"), (other, "other.png")] {
        let info = app.run("links.info", json!({"id": id})).unwrap();
        assert_eq!(info["path"], out.join(name).to_string_lossy().as_ref(), "{info}");
    }
    assert_eq!(app.ui.status, format!("Unembedded 2 images to {picked}"));
}

#[test]
fn the_links_menu_items_run_commands() {
    let tree = crate::menus::menu_tree();
    let ids = |label: &str| {
        let (_, items) = tree.iter().find(|(t, _)| *t == label).unwrap();
        format!("{items:?}")
    };
    assert!(ids("Edit").contains("links.editOriginal"));
    assert!(ids("Window").contains(super::links::ID));
}

#[test]
fn right_click_context_menu_in_links_panel() {
    use egui::vec2;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let dir = Folder::new("context-menu");
    let (mut app, linked, embedded) = app(&dir);

    let revealed: Rc<RefCell<Vec<String>>> = Rc::default();
    let rev = revealed.clone();
    app.services.reveal = Some(Box::new(move |p: &str| {
        rev.borrow_mut().push(p.to_string());
        Ok(())
    }));

    // Initially embedded is selected, linked is unselected
    assert_eq!(selected(&app), vec![embedded]);

    let mut h = Harness::builder().with_size(vec2(280.0, 500.0)).with_accessibility_check(false).build_ui_state(
        |ui, app: &mut VectorcraftApp| {
            super::links::show(app, ui);
        },
        app,
    );
    h.run_steps(2);

    let right_click = |h: &mut Harness<'static, VectorcraftApp>, label: &str| {
        let node = h.get_all_by_label(label).find(|n| n.accesskit_node().role() == egui::accesskit::Role::ListItem).unwrap();
        let pos = node.rect().center();
        h.event(egui::Event::PointerMoved(pos));
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);
    };

    // 1. Right-click unselected "linked.png": selects it and opens context menu
    right_click(&mut h, "linked.png");
    assert_eq!(selected(h.state()), vec![linked], "right-clicking unselected row selects that row first");

    let relink = h.get_by_label("   Relink…");
    let go_to = h.get_by_label("   Go To Link");
    let update = h.get_by_label("   Update Link");
    let edit_orig = h.get_by_label("   Edit Original");
    let show_folder = h.get_by_label("   Show in Folder");
    let embed_img = h.get_by_label("   Embed Image");

    // "Relink…" is disabled because app has no pick_open service by default
    assert!(relink.accesskit_node().is_disabled());
    // "Go To Link" is enabled because linked node is in canvas document
    assert!(!go_to.accesskit_node().is_disabled());
    // "Update Link" is disabled because linked file is not modified
    assert!(update.accesskit_node().is_disabled());
    // "Edit Original" and "Show in Folder" are enabled because linked file exists
    assert!(!edit_orig.accesskit_node().is_disabled());
    assert!(!show_folder.accesskit_node().is_disabled());
    // "Embed Image" is enabled because it is a linked image
    assert!(!embed_img.accesskit_node().is_disabled());

    // 2. Click "Show in Folder" dispatches links.reveal
    show_folder.click();
    h.run_steps(2);
    assert_eq!(revealed.borrow().len(), 1);
    assert!(revealed.borrow()[0].ends_with("linked.png"));

    // 3. Multi-selection: select both linked and embedded
    h.state_mut().run("select.set", json!({"ids": [linked, embedded]})).unwrap();
    assert_eq!(selected(h.state()), vec![linked, embedded]);

    // Right-click "linked.png" while already selected in multi-selection: preserves selection
    right_click(&mut h, "linked.png");
    assert_eq!(selected(h.state()), vec![linked, embedded], "right-clicking selected row preserves multi-selection");

    // 4. Click "Go To Link" focuses the art on canvas via go_to
    let go_to_btn = h.get_by_label("   Go To Link");
    assert!(!go_to_btn.accesskit_node().is_disabled());
    go_to_btn.click();
    h.run_steps(2);
    assert_eq!(selected(h.state()), vec![linked]);
    assert!(h.state().view().unwrap().fitted);

    // 5. Right-click embedded image: displays "Unembed…" while "Edit Original" and "Show in Folder" are disabled
    right_click(&mut h, "embedded.png");
    assert_eq!(selected(h.state()), vec![embedded]);
    let unembed_btn = h.get_by_label("   Unembed…");
    assert!(!unembed_btn.accesskit_node().is_disabled());
    assert!(h.get_by_label("   Edit Original").accesskit_node().is_disabled());
    assert!(h.get_by_label("   Show in Folder").accesskit_node().is_disabled());
}
