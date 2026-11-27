//! The Links panel, its menu rows, Link Info, Edit Original / Show in Folder, the Placement Options
//! dialog and the Properties panel's image section, drawn headless.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::i18n::LangExt as _;
use crate::tests::panel_harness::Panel;
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

type Rig = Harness<'static, VectorcraftApp>;

/// The Links panel of `app` in a headless window, after two settled frames.
fn rig(app: VectorcraftApp) -> Rig {
    let mut h = Harness::builder()
        .with_size(vec2(280.0, 500.0))
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut VectorcraftApp| super::links::show(app, ui), app);
    h.run_steps(2);
    h
}

/// Whether some node of the panel is named `label`.
fn shown(h: &Rig, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

/// How many nodes of the panel are named `label`.
fn count(h: &Rig, label: &str) -> usize {
    h.query_all_by_label(label).count()
}

/// Runs the panel command `id` with `params` on the app in `h`, then a frame.
fn command(h: &mut Rig, id: &str, params: serde_json::Value) {
    super::links::run(h.state_mut(), id, &params).expect("a panel command").unwrap();
    h.run_steps(2);
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
    let (app, linked, _) = app(&dir);
    let mut h = rig(app);
    assert!(shown(&h, "linked.png") && shown(&h, "embedded.png"));
    // The selection is the embedded image.
    assert!(shown(&h, "Embedded") && shown(&h, "Effective PPI"));
    h.state_mut().run("select.set", json!({"ids": [linked]})).unwrap();
    h.run_steps(2);
    assert!(shown(&h, "Linked") && shown(&h, "300 × 200 px") && shown(&h, "Location"));
}

/// Opening the panel on the selected image (the Control bar's Linked File or Embedded label) opens
/// Link Info when it was closed, and sets a Show filter that leaves out the image's row back to
/// Show All; a filter that lists the row stays.
#[test]
fn opening_the_panel_on_the_selected_image_shows_its_row_and_link_info() {
    let dir = Folder::new("show-selected");
    let (app, linked, embedded) = app(&dir);
    let mut h = rig(app);
    h.state_mut().run("select.set", json!({"ids": [linked]})).unwrap();
    command(&mut h, "ui.links.linkInfo", json!({}));
    command(&mut h, "ui.links.show", json!({"show": "embedded"}));
    assert!(!shown(&h, "linked.png") && !shown(&h, "Location"), "the filter hides the row, Link Info is closed");
    h.state_mut().run("window.panel.close", json!({"panel": super::links::ID})).ok();
    let ctx = h.ctx.clone();
    super::links::show_selected_image(h.state_mut(), &ctx);
    h.run_steps(2);
    assert!(crate::panel_docking::visible(h.state(), super::links::ID), "status: {}", h.state().ui.status);
    assert!(shown(&h, "linked.png") && shown(&h, "embedded.png"), "Show All lists every row");
    assert!(shown(&h, "Location") && shown(&h, "300 × 200 px"), "Link Info shows the linked image");
    // An embedded image under Show Embedded keeps the filter.
    h.state_mut().run("select.set", json!({"ids": [embedded]})).unwrap();
    command(&mut h, "ui.links.linkInfo", json!({}));
    command(&mut h, "ui.links.show", json!({"show": "embedded"}));
    super::links::show_selected_image(h.state_mut(), &ctx);
    h.run_steps(2);
    assert!(crate::panel_docking::visible(h.state(), super::links::ID), "an open panel stays open");
    assert!(shown(&h, "embedded.png") && !shown(&h, "linked.png"), "Show Embedded stays");
    assert!(shown(&h, "Effective PPI"), "Link Info shows the embedded image");
}

/// Opening the panel on the selected image draws a list read after the request, so a row that a
/// file change brought into the Show filter since the list was last read is there to scroll to.
#[test]
fn opening_the_panel_on_the_selected_image_reads_the_list_again() {
    let dir = Folder::new("show-selected-fresh");
    let (app, linked, _) = app(&dir);
    let mut h = rig(app);
    // How many nodes name the linked image: its row, and Link Info.
    h.state_mut().run("select.set", json!({"ids": [linked]})).unwrap();
    command(&mut h, "ui.links.show", json!({"show": "missing"}));
    assert_eq!(count(&h, "linked.png"), 1, "Link Info only: nothing is missing yet");
    std::fs::remove_file(dir.0.join("linked.png")).unwrap();
    // Within the time the panel keeps its list before it looks at the files again.
    let ctx = h.ctx.clone();
    super::links::show_selected_image(h.state_mut(), &ctx);
    h.run_steps(2);
    assert_eq!(count(&h, "linked.png"), 2, "the missing image's row and its Link Info are drawn");
}

/// A request to show the selected image applies only in the document it was made in: drawn for
/// another document, the panel keeps its Show filter and drops the request.
#[test]
fn opening_the_panel_on_the_selected_image_leaves_another_document_alone() {
    let dir = Folder::new("show-selected-other-doc");
    let (app, linked, _) = app(&dir);
    let mut h = rig(app);
    h.state_mut().run("select.set", json!({"ids": [linked]})).unwrap();
    command(&mut h, "ui.links.show", json!({"show": "missing"}));
    let ctx = h.ctx.clone();
    super::links::show_selected_image(h.state_mut(), &ctx);
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).unwrap();
    h.run_steps(2);
    assert!(shown(&h, "Nothing matches the Show filter."), "the other document keeps Show Missing");
    assert_eq!(h.state().ui.links.show, None, "the request is dropped");
}

/// A frame that fetches nothing keeps the stored rows and Link Info as they are, without a copy; the
/// two-second recheck, another first selected image and an edit each store a new cache.
#[test]
fn frames_that_fetch_nothing_reuse_the_cached_rows() {
    let dir = Folder::new("cache");
    let (app, linked, _) = app(&dir);
    let mut h = rig(app);
    let cache = |h: &Rig| std::sync::Arc::clone(&h.state().ui.links.cache);
    let now = |h: &Rig| h.ctx.input(|i| i.time);
    assert!(shown(&h, "linked.png") && shown(&h, "40 × 20 px"));
    let first = cache(&h);
    h.step();
    assert!(std::sync::Arc::ptr_eq(&first, &cache(&h)), "a frame that fetches nothing stores no new cache");
    assert!(shown(&h, "linked.png") && shown(&h, "40 × 20 px"));
    let start = now(&h);
    while now(&h) - start <= 2.1 {
        h.step();
    }
    let rechecked = cache(&h);
    assert!(!std::sync::Arc::ptr_eq(&first, &rechecked), "the files are looked at again after two seconds");
    h.state_mut().run("select.set", json!({"ids": [linked]})).unwrap();
    h.step();
    let picked = cache(&h);
    assert!(!std::sync::Arc::ptr_eq(&rechecked, &picked), "Link Info follows the first selected image");
    assert!(shown(&h, "300 × 200 px"));
    h.state_mut().run("edit.clear", json!({"ids": [linked]})).unwrap();
    h.step();
    assert!(!std::sync::Arc::ptr_eq(&picked, &cache(&h)), "an edit lists the rows again");
    assert!(!shown(&h, "linked.png") && shown(&h, "embedded.png"));
}

/// Copy Info copies the file names of the selected links, or of `ids` in the order given, one per
/// line; it is off with nothing selected, and a bad `ids` leaves the clipboard as it was.
#[test]
fn copy_info_copies_the_file_names_of_the_links() {
    let dir = Folder::new("copy-info");
    let (mut app, linked, embedded) = app(&dir);
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
    let mut h = Panel::new(app, super::links::show);
    assert!(h.shows("20 × 20 px"), "Link Info of the selected image in the layers");
    h.click_with("inside.png", egui::Modifiers::NONE);
    assert!(selected(&h.app()).is_empty(), "the canvas can't select it");
    assert!(h.shows("30 × 20 px"), "Link Info of the symbol's image");
    // Copy Info without ids copies the row picked in the panel.
    assert!(crate::menus::enabled(&h.app(), "links.copyInfo"));
    assert_eq!(h.app().run("links.copyInfo", json!({})).unwrap()["text"], "inside.png");
    assert_eq!(h.app().clipboard_out.take().as_deref(), Some("inside.png"));
    h.click_with("layer.png", egui::Modifiers::COMMAND);
    assert_eq!(selected(&h.app()), [layer]);
    // Both rows are selected until the canvas selection changes: then the panel's pick goes too. The second click
    // comes after the double-click time, so it is a click of its own.
    h.advance(1.0);
    h.click_with("layer.png", egui::Modifiers::COMMAND);
    assert!(selected(&h.app()).is_empty());
    assert!(h.shows("30 × 20 px"), "the symbol's image is still picked");
    h.app().run("select.set", json!({"ids": [layer]})).unwrap();
    h.frame();
    assert!(h.shows("20 × 20 px"));
    h.app().run("select.set", json!({"ids": []})).unwrap();
    h.frame();
    assert!(!h.shows("30 × 20 px") && !h.shows("20 × 20 px"), "nothing is picked");
    // Go To clears the selection and says the link has no place on the canvas.
    h.app().run("select.set", json!({"ids": [layer]})).unwrap();
    super::links::go_to(&mut h.app(), row["id"].as_u64().unwrap());
    assert_eq!(h.app().ui.status, "The link is inside a symbol, graph design, pattern or opacity mask and has no place on the canvas");
    assert!(selected(&h.app()).is_empty(), "the image in the layers is no longer selected");
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
    let h = rig(app);
    let found = h.state().ui.links.cache.nested_node(owner.0).map(|n| super::layers::shown_name(h.state(), n, crate::i18n::current()));
    assert_eq!(found.as_deref(), Some(name.as_str()), "the panel collected the masked object with the list");
    let tip = super::links::status_tip(&row, found);
    assert!(tip.contains(&format!("Opacity Mask: {name}")), "{tip}");
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

#[test]
fn a_long_link_location_fits_the_minimum_width_float() {
    use crate::panel_docking::PANEL_MIN_WIDTH;
    use crate::tests::panel_harness::{Place, floated};

    let folder = Folder::new("location-width");
    let subfolder = folder.0.join("Überlagerung".repeat(5));
    std::fs::create_dir_all(&subfolder).unwrap();
    let path = subfolder.join("Éléments.png");
    std::fs::write(&path, png(30, 20)).unwrap();
    for locale in ["en", "fr", "de"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let placed = app.run("file.place", json!({"path": path, "at": [100, 100]})).unwrap();
        let info = app.run("links.info", json!({"id": placed["ids"][0]})).unwrap();
        let location = info["location"].as_str().unwrap();
        let names = ["Location", location, "Show in Folder"];
        app.services.reveal = Some(Box::new(|_| Ok(())));
        let panel = floated(app, "links", locale, Place::Floating(PANEL_MIN_WIDTH), &names);
        let past = panel.past_named(&names);
        assert!(past.is_empty(), "{locale} location row outside {:?}: {past:?}", panel.body);
    }
}

/// The Location row of Link Info has a Show in Folder button that shows the file of the image Link
/// Info shows. An embedded image has no Location row; the button is off while the file is missing
/// and without a file manager (the web).
#[test]
fn link_infos_location_row_shows_the_file_in_its_folder() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let dir = Folder::new("location-reveal");
    let (mut app, linked, embedded) = app(&dir);
    let other = dir.file("other.png", &png(30, 20));
    let other = app.run("file.place", json!({"path": other, "at": [50, 50]})).unwrap()["ids"][0].as_u64().unwrap();
    app.run("select.set", json!({"ids": [embedded]})).unwrap();
    let revealed: Rc<RefCell<Vec<String>>> = Rc::default();
    let r = revealed.clone();
    app.services.reveal = Some(Box::new(move |p: &str| {
        r.borrow_mut().push(p.to_string());
        Ok(())
    }));
    let path = |app: &mut VectorcraftApp, id: u64| app.run("links.info", json!({"id": id})).unwrap()["path"].as_str().unwrap().to_string();
    let (linked_path, other_path) = (path(&mut app, linked), path(&mut app, other));
    let mut h = Panel::new(app, super::links::show);
    let select = |h: &mut Panel, ids: &[u64]| {
        h.app().run("select.set", json!({ "ids": ids })).unwrap();
        h.frame();
    };
    // The button is pressed as a keyboard or screen-reader user presses it: focus, then Enter.
    let press = |h: &mut Panel| {
        h.focus("Show in Folder");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
    };
    // An embedded image has no file to show.
    assert!(!h.shows("Show in Folder"));
    select(&mut h, &[linked]);
    assert!(h.enabled("Show in Folder"));
    press(&mut h);
    select(&mut h, &[other]);
    assert!(h.enabled("Show in Folder"));
    press(&mut h);
    assert_eq!(*revealed.borrow(), [linked_path.clone(), other_path.clone()]);
    // Both images selected: Link Info shows the first selected, `linked`, while the list has `other`
    // (on top) first. Only the file of the image Link Info shows is handed over.
    select(&mut h, &[linked, other]);
    press(&mut h);
    assert_eq!(*revealed.borrow(), [linked_path.clone(), other_path.clone(), linked_path]);
    // A missing file: the row stays, the button is off once the panel looks at the files again.
    select(&mut h, &[other]);
    std::fs::remove_file(&other_path).unwrap();
    h.advance(3.0);
    h.frame();
    assert!(h.shows("Location") && !h.enabled("Show in Folder"));
    // Without a file manager (the web): off.
    select(&mut h, &[linked]);
    h.app().services.reveal = None;
    h.frame();
    assert!(!h.enabled("Show in Folder"));
    assert_eq!(revealed.borrow().len(), 3, "nothing more was shown");
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
    // A pass that carries injected input acts for automation too.
    let ctx = crate::testing::ui_context();
    app.adopt_context(&ctx);
    craft::ui::runtime::windows::WindowHost::of(&ctx).inject_key(egui::ViewportId::ROOT, craft::keymap::Key::A, Default::default(), false);
    ctx.run_ui(egui::RawInput::default(), |_| {
        assert!(app.run("links.editOriginal", json!({"id": linked})).unwrap_err().contains("only the person"));
    })
    .textures_delta
    .clear();
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
    use crate::tests::picks::{answer_dialog, open_dialogs};
    let dir = Folder::new("dialog");
    let (app, linked, _) = app(&dir);
    let mut app = crate::testing::windowed(app);
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    assert!(crate::menus::enabled(&app, "ui.placementOptionsDialog"));
    app.run("ui.placementOptionsDialog", json!({})).unwrap();
    let open = open_dialogs(&app);
    assert_eq!(open.len(), 1, "the dialog is open");
    // PLAN: assumed `placement_options::ALIGN` names the reference points in the order of the
    // dialog's `align` index (files-w22).
    let align = crate::dialogs::placement_options::ALIGN.iter().position(|a| *a == "bottomRight").unwrap();
    answer_dialog(&mut app, open[0].id, &[("preserve", json!("fit")), ("align", json!(align))], true);
    assert!(open_dialogs(&app).is_empty(), "OK closes the dialog");
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
    use crate::tests::picks::{poll, scripted};
    assert!(super::links::can_unembed(&app, 1));
    assert!(!super::links::can_unembed(&app, 0));
    let out = dir.0.join("out");
    std::fs::create_dir_all(&out).unwrap();
    // PLAN: assumed `scripted` and `poll` of fl.A20 live in `crate::tests::picks`.
    let picker = scripted(&mut app);
    let ctx = crate::testing::ui_context();
    assert!(super::links::can_unembed(&app, 2));
    // No folder picked: nothing written or linked.
    super::links::unembed_several(&mut app, vec![embedded, other]);
    picker.cancel();
    poll(&mut app, &ctx);
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    assert_eq!(app.run("links.info", json!({"id": embedded})).unwrap()["status"], "embedded");
    let picked = out.to_string_lossy().into_owned();
    super::links::unembed_several(&mut app, vec![embedded, other]);
    picker.answer(&[picked.as_str()]);
    poll(&mut app, &ctx);
    for (id, name) in [(embedded, "embedded.png"), (other, "other.png")] {
        let info = app.run("links.info", json!({"id": id})).unwrap();
        assert_eq!(info["path"], out.join(name).to_string_lossy().as_ref(), "{info}");
    }
    assert_eq!(app.ui.status, format!("Unembedded 2 images to {picked}"));
}

#[test]
fn switching_documents_dismisses_a_links_row_menu() {
    let dir = Folder::new("context-switched-document");
    let (mut app, first, _) = app(&dir);
    let source = app.session.active().unwrap().uid;
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let path = dir.file("other.png", &png(40, 20));
    let second = app.run("file.place", json!({"path": path, "at": [100, 100]})).unwrap()["ids"][0].as_u64().unwrap();
    let destination = app.session.active().unwrap().uid;
    assert_eq!(first, second, "the two documents have different images at the same local ID");
    app.run("window.previousDocument", json!({})).unwrap();

    let mut panel = Panel::new(app, super::links::show);
    let activate = |panel: &mut Panel, index, document| {
        panel.app().session.execute("document.activate", &json!({"index": index})).unwrap();
        panel.frame();
        panel.frame();
        assert_eq!(panel.app().session.active().unwrap().uid, document);
    };
    panel.secondary_click("linked.png");
    assert!(panel.menu_item_enabled("Embed Image"));
    activate(&mut panel, 1, destination);
    assert!(!panel.shows("Embed Image"), "the other document must not inherit the row menu");
    assert_eq!(panel.app().run("links.info", json!({"id": second})).unwrap()["linked"], true);

    activate(&mut panel, 0, source);
    assert!(!panel.shows("Embed Image"), "switching back must not revive the menu");
    assert_eq!(panel.app().run("links.info", json!({"id": first})).unwrap()["linked"], true);

    activate(&mut panel, 1, destination);
    panel.secondary_click("other.png");
    panel.click_menu_item("Embed Image");
    assert_eq!(panel.app().run("links.info", json!({"id": second})).unwrap()["status"], "embedded");
}

#[test]
fn right_click_context_menu_in_links_panel() {
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

    let mut h = Panel::new(app, super::links::show);

    // 1. Right-click unselected "linked.png": selects it and opens context menu
    h.secondary_click("linked.png");
    assert_eq!(selected(&h.app()), vec![linked], "right-clicking unselected row selects that row first");

    // "Relink…" is enabled because the desktop can ask for a file
    assert!(h.menu_item_enabled("Relink…"));
    // "Go To Link" is enabled because linked node is in canvas document
    assert!(h.menu_item_enabled("Go To Link"));
    // "Update Link" is disabled because linked file is not modified
    assert!(!h.menu_item_enabled("Update Link"));
    // "Edit Original" and "Show in Folder" are enabled because linked file exists
    assert!(h.menu_item_enabled("Edit Original"));
    assert!(h.menu_item_enabled("Show in Folder"));
    // "Embed Image" is enabled because it is a linked image
    assert!(h.menu_item_enabled("Embed Image"));

    // 2. Click "Show in Folder" dispatches links.reveal
    h.click_menu_item("Show in Folder");
    assert_eq!(revealed.borrow().len(), 1);
    assert!(revealed.borrow()[0].ends_with("linked.png"));

    // 3. Multi-selection: select both linked and embedded
    h.app().run("select.set", json!({"ids": [linked, embedded]})).unwrap();
    assert_eq!(selected(&h.app()), vec![linked, embedded]);

    // Right-click "linked.png" while already selected in multi-selection: preserves selection
    h.secondary_click("linked.png");
    assert_eq!(selected(&h.app()), vec![linked, embedded], "right-clicking selected row preserves multi-selection");

    // 4. Click "Go To Link" focuses the art on canvas via go_to
    assert!(h.menu_item_enabled("Go To Link"));
    h.click_menu_item("Go To Link");
    assert_eq!(selected(&h.app()), vec![linked]);
    assert!(h.app().view().unwrap().fitted);

    // 5. Right-click embedded image: displays "Unembed…" while "Edit Original" and "Show in Folder" are disabled
    h.secondary_click("embedded.png");
    assert_eq!(selected(&h.app()), vec![embedded]);
    assert!(h.menu_item_enabled("Unembed…"));
    assert!(!h.menu_item_enabled("Edit Original"));
    assert!(!h.menu_item_enabled("Show in Folder"));
}
