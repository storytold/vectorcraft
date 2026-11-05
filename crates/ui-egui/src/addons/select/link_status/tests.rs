use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::{COMMAND, KIND, choose_shown, message, refresh, shown, toggle};
use crate::tests_labels::painted_text;
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

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 40, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A document with `river.png` linked, `harbour.png` linked and then deleted (missing), and
/// `stamp.png` embedded → (app, river, harbour, stamp).
fn app(dir: &Folder) -> (VectorcraftApp, u64, u64, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
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
    let ids = |v: Vec<&Value>| v.iter().map(|r| r["id"].as_u64().unwrap()).collect::<Vec<_>>();
    assert_eq!(ids(shown(&rows, "", "all")), vec![1, 2, 3, 4]);
    assert_eq!(ids(shown(&rows, " RIVER ", "all")), vec![1, 4]);
    assert_eq!(ids(shown(&rows, "river", "linked")), vec![1], "ok is Linked");
    assert_eq!(ids(shown(&rows, "", "missing")), vec![2]);
    assert_eq!(ids(shown(&rows, "zzz", "all")), Vec::<u64>::new());
    // Row 3 is chosen but hidden: neither All nor None touches it.
    let mut c = vec![3];
    choose_shown(&mut c, &[1, 4], true);
    assert_eq!(c, vec![3, 1, 4]);
    choose_shown(&mut c, &[1, 4], false);
    assert_eq!(c, vec![3]);
    toggle(&mut c, 2);
    toggle(&mut c, 3);
    assert_eq!(c, vec![2]);
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
    let run = |app: &mut VectorcraftApp, label: &str, shift: bool| {
        let e = item(label);
        let target = menus::click_target(&e.label, e.command.as_deref().unwrap(), &e.params);
        let held = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
        let (cmd, p) = menus::with_held_keys(target, held);
        menus::invoke(app, &cmd, p);
    };
    run(&mut app, "Select Missing", false);
    assert_eq!(selected(&app), vec![harbour]);
    assert_eq!(app.ui.status, "Selected 1 missing image");
    run(&mut app, "Select Embedded", true);
    assert_eq!(selected(&app), vec![harbour, stamp], "Shift adds");
    run(&mut app, "Select Linked", false);
    assert_eq!(selected(&app), vec![river], "without Shift it replaces");
    // A hidden image is left out, and the status bar says so.
    app.run("object.setProps", json!({"ids": [river], "visible": false})).unwrap();
    run(&mut app, "Select Linked", false);
    assert_eq!(selected(&app), Vec::<u64>::new());
    assert_eq!(app.ui.status, "Selected 0 linked images; 1 more is hidden or locked");
    // The document changes: the items follow on the next refresh.
    app.run("select.set", json!({"ids": [harbour]})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    refresh(&mut app, 1.0);
    let entries = menus::menu_entries(&app);
    let on = |label: &str| entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Selection Tools"] && e.label == label).unwrap().enabled;
    assert!(!on("Select Missing"));
    assert!(on("Select Embedded"));
}

#[test]
fn the_links_panel_menu_lists_the_select_items() {
    let dir = Folder::new("flyout");
    let (mut app, ..) = app(&dir);
    refresh(&mut app, 0.0);
    let shown = painted_text(&mut app, crate::panels::links::menu);
    for s in ["Select Linked", "Select Embedded", "Select Missing", "Select Modified"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
}

#[test]
fn the_dialog_lists_filters_and_selects_with_zoom() {
    let dir = Folder::new("dialog");
    let (mut app, river, harbour, stamp) = app(&dir);
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 460.0)));
    let zoom_before = app.view().unwrap().zoom;
    menus::invoke(&mut app, COMMAND, Value::Null);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), &d.fields["zoom"], &d.fields["show"]), (KIND, &json!(true), &json!("all")));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in [
        "Select by Link Status",
        "river.png",
        "harbour.png",
        "stamp.png",
        "Linked",
        "Missing",
        "Embedded",
        "PNG",
        "Zoom to Selection",
        "Chosen: 0 of 3",
        "All",
        "None",
        "Select",
    ] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    assert!(!(super::SPEC.ok_enabled.unwrap())(&app, app.ui.dialog.as_ref().unwrap()), "Select is off with nothing chosen");
    // Show: Missing lists only the missing image.
    app.ui.dialog.as_mut().unwrap().fields.insert("show".into(), json!("missing"));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(shown.contains("harbour.png") && !shown.contains("river.png") && !shown.contains("stamp.png"), "{shown}");
    // Choose two and Select: they are selected and the view zooms to them.
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("ids".into(), json!([river, stamp]));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(selected(&app), vec![river, stamp]);
    assert_eq!(app.ui.status, "Selected 2 images");
    assert_ne!(app.view().unwrap().zoom, zoom_before, "zoomed to the selection");
    // Without Zoom to Selection the view stays.
    menus::invoke(&mut app, COMMAND, Value::Null);
    let zoom = app.view().unwrap().zoom;
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("ids".into(), json!([harbour]));
    d.fields.insert("zoom".into(), json!(false));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(selected(&app), vec![harbour]);
    assert_eq!(app.view().unwrap().zoom, zoom);
}
