use craft::i18n::Folder as Language;
use craft::ui::dialog::dialog_id;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use serde_json::{Value, json};

use super::{COMMAND, KIND, LinkStatusDialog, STATUSES, choose_shown, message, refresh, shown, states, with_shift};
use crate::addons::select::tests::{confirm, host, new_app, set};
use crate::{VectorcraftApp, menus};

/// A fresh folder for one test (removed when dropped).
struct Folder(std::path::PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vectorcraft-ui-select-links-{name}-{}", std::process::id()));
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

/// A `w` × `h` red PNG.
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

/// A document with `river.png` linked, `harbour.png` linked and then deleted (missing), and
/// `stamp.png` embedded → (app, river, harbour, stamp).
fn app(dir: &Folder) -> (VectorcraftApp, u64, u64, u64) {
    let mut app = new_app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let place = |app: &mut VectorcraftApp, path: &str, at: [u32; 2], link: bool| {
        app.run("file.place", json!({"path": path, "at": at, "link": link})).unwrap()["ids"][0].as_u64().unwrap()
    };
    let (a, b, c) = (dir.file("river.png", &png(40, 30)), dir.file("harbour.png", &png(30, 20)), dir.file("stamp.png", &png(20, 20)));
    let river = place(&mut app, &a, [100, 100], true);
    let harbour = place(&mut app, &b, [300, 100], true);
    let stamp = place(&mut app, &c, [200, 220], false);
    std::fs::remove_file(&b).unwrap();
    app.run("select.none", json!({})).unwrap();
    (app, river, harbour, stamp)
}

/// The selected objects' ids, sorted.
fn selected(app: &VectorcraftApp) -> Vec<u64> {
    let mut ids: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect();
    ids.sort_unstable();
    ids
}

#[test]
fn the_message_says_what_was_selected_and_what_was_left_out() {
    assert_eq!(message(&["missing"], 9, &[]), "Selected 9 missing images");
    assert_eq!(message(&["linked"], 1, &[]), "Selected 1 linked image");
    assert_eq!(message(&["embedded", "modified"], 2, &[]), "Selected 2 images");
    assert_eq!(message(&["missing"], 9, &["hidden", "locked", "hidden"]), "Selected 9 missing images; 3 more are hidden or locked");
    assert_eq!(message(&["modified"], 0, &["template"]), "Selected 0 modified images; 1 more is on a template layer");
    assert_eq!(message(&["embedded"], 2, &["isolation", "isolation"]), "Selected 2 embedded images; 2 more are outside the isolated group");
    assert_eq!(message(&["linked"], 1, &["isolation", "locked"]), "Selected 1 linked image; 2 more can't be selected now");
}

#[test]
fn the_list_filters_by_name_and_status_and_all_and_none_act_on_the_rows_shown() {
    let rows = vec![
        json!({"id": 1, "name": "River.png", "status": "ok"}),
        json!({"id": 2, "name": "harbour.png", "status": "missing"}),
        json!({"id": 3, "name": "Stamp.png", "status": "embedded"}),
        json!({"id": 4, "name": "river-2.tif", "status": "modified"}),
    ];
    let folder = Language::new("en");
    let ids = |v: Vec<&Value>| v.iter().map(|r| r["id"].as_u64().unwrap()).collect::<Vec<_>>();
    assert_eq!(ids(shown(&rows, "", "all", &folder)), vec![1, 2, 3, 4]);
    assert_eq!(ids(shown(&rows, " RIVER ", "all", &folder)), vec![1, 4]);
    assert_eq!(ids(shown(&rows, "river", "linked", &folder)), vec![1], "ok is Linked");
    assert_eq!(ids(shown(&rows, "", "missing", &folder)), vec![2]);
    assert_eq!(ids(shown(&rows, "zzz", "all", &folder)), Vec::<u64>::new());
    // Row 3 is chosen but hidden: neither All nor None touches it.
    let mut c = vec![3];
    choose_shown(&mut c, &[1, 4], true);
    assert_eq!(c, vec![3, 1, 4]);
    choose_shown(&mut c, &[1, 4], false);
    assert_eq!(c, vec![3]);
}

#[test]
fn menu_items_are_on_for_the_statuses_present_and_shift_adds() {
    let dir = Folder::new("menu");
    let (mut app, river, harbour, stamp) = app(&dir);
    refresh(&mut app, 0.0);
    let entries = menus::menu_entries(&app);
    let item = |label: &str| entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Selection Tools"] && e.label == label).unwrap();
    for (label, on) in
        [("Select Linked", true), ("Select Embedded", true), ("Select Missing", true), ("Select Modified", false), ("Select by Link Status…", true)]
    {
        assert_eq!(item(label).enabled, on, "{label}");
    }
    // The Links panel's rows follow the statuses present, per status.
    let enabled_of = |app: &VectorcraftApp| {
        let mut all = CommandStates::new();
        states(app, &mut all);
        STATUSES.map(|(status, ..)| all.get(&CommandRef::with_params(COMMAND, json!({ "status": status }))).cloned())
    };
    let on = Some(CommandState::new().enabled(true));
    assert_eq!(enabled_of(&app), [on.clone(), on.clone(), on.clone(), Some(CommandState::new().enabled(false))]);
    // The item's params run as the menu runs them: Shift's alternate is `with_shift` of them.
    let run = |app: &mut VectorcraftApp, status: &str, shift: bool| {
        let p = json!({ "status": status });
        menus::invoke(app, COMMAND, if shift { with_shift(p) } else { p });
    };
    run(&mut app, "missing", false);
    assert_eq!(selected(&app), vec![harbour]);
    assert_eq!(app.ui.status, "Selected 1 missing image");
    run(&mut app, "embedded", true);
    assert_eq!(selected(&app), vec![harbour, stamp], "Shift adds");
    run(&mut app, "linked", false);
    assert_eq!(selected(&app), vec![river], "without Shift it replaces");
    // A hidden image is left out, and the status bar says so.
    app.run("object.setProps", json!({"ids": [river], "visible": false})).unwrap();
    run(&mut app, "linked", false);
    assert_eq!(selected(&app), Vec::<u64>::new());
    assert_eq!(app.ui.status, "Selected 0 linked images; 1 more is hidden or locked");
    // The document changes: the states follow on the next refresh.
    app.run("select.set", json!({"ids": [harbour]})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    refresh(&mut app, 1.0);
    let after = enabled_of(&app);
    assert_eq!(after[2], Some(CommandState::new().enabled(false)), "no missing image is left");
    assert_eq!(after[1], on, "the embedded one stays");
}

/// A missing link kept only in a symbol's art can't be selected: Select Missing stays off and
/// the dialog does not list it.
#[test]
fn links_inside_a_symbol_are_left_out_of_selecting_by_status() {
    let dir = Folder::new("symbol");
    let mut app = new_app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let path = dir.file("inside.png", &png(30, 20));
    let id = app.run("file.place", json!({"path": path, "at": [100, 100]})).unwrap()["ids"][0].as_u64().unwrap();
    app.run("symbol.new", json!({"name": "Mark", "ids": [id]})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    std::fs::remove_file(&path).unwrap();
    let list = app.run("links.list", json!({})).unwrap();
    assert_eq!(list["missing"], 1, "{list}");
    refresh(&mut app, 0.0);
    let entries = menus::menu_entries(&app);
    let missing = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Selection Tools"] && e.label == "Select Missing").unwrap();
    assert!(!missing.enabled);
    menus::invoke(&mut app, COMMAND, Value::Null);
    assert_eq!(host(&app, |h| h.get::<LinkStatusDialog>().map(|d| d.rows.len())), Some(0), "the dialog lists no row of the symbol");
}

#[test]
fn the_dialog_draws_its_images_and_buttons() {
    let dir = Folder::new("drawn");
    let (app, ..) = app(&dir);
    let mut p = crate::tests::panel_harness::Panel::new(app, crate::tests::picks::dialogs);
    menus::invoke(&mut p.app(), COMMAND, Value::Null);
    p.frame();
    for name in ["river.png", "harbour.png", "Select", "Cancel"] {
        assert!(p.shows(name), "{name} is drawn");
    }
}

#[test]
fn the_dialog_lists_filters_and_selects_with_zoom() {
    let dir = Folder::new("dialog");
    let (mut app, river, harbour, stamp) = app(&dir);
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 460.0));
    let zoom_before = app.view().unwrap().zoom;
    menus::invoke(&mut app, COMMAND, Value::Null);
    let id = dialog_id(KIND);
    let (fields, names) = host(&app, |h| {
        let d = h.get::<LinkStatusDialog>().unwrap();
        let language = Language::new("en");
        let names: Vec<String> = d.rows.iter().map(|r| r["name"].as_str().unwrap().to_owned()).collect();
        let missing: Vec<String> = shown(&d.rows, "", "missing", &language).iter().map(|r| r["name"].as_str().unwrap().to_owned()).collect();
        assert_eq!(missing, ["harbour.png"], "Show: Missing lists only the missing image");
        (d.fields.clone(), names)
    });
    assert_eq!((fields.zoom, fields.show.as_str(), fields.ids.is_empty()), (true, "all", true), "Zoom on, Show All, nothing chosen");
    assert_eq!(names.len(), 3);
    for name in ["river.png", "harbour.png", "stamp.png"] {
        assert!(names.iter().any(|n| n == name), "{name} in {names:?}");
    }
    assert!(!confirm(&mut app, id), "Select is refused with nothing chosen");
    // Choose two and Select: they are selected and the view zooms to them.
    set(&app, id, "ids", json!([river, stamp])).unwrap();
    assert!(confirm(&mut app, id));
    assert!(host(&app, |h| !h.is_open::<LinkStatusDialog>()));
    assert_eq!(selected(&app), vec![river, stamp]);
    assert_eq!(app.ui.status, "Selected 2 images");
    assert_ne!(app.view().unwrap().zoom, zoom_before, "zoomed to the selection");
    // Without Zoom to Selection the view stays.
    menus::invoke(&mut app, COMMAND, Value::Null);
    let zoom = app.view().unwrap().zoom;
    set(&app, id, "ids", json!([harbour])).unwrap();
    set(&app, id, "zoom", json!(false)).unwrap();
    assert!(confirm(&mut app, id));
    assert_eq!(selected(&app), vec![harbour]);
    assert_eq!(app.view().unwrap().zoom, zoom);
}
