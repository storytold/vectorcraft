//! The Asset Export panel driven by control name: the shared export settings rows, the panel menu's
//! `ui.assetExport.*` commands, presets, Create Sub-folders and Open Location, and Export.

use std::cell::RefCell;
use std::rc::Rc;

use craft::keymap::CommandRef;
use craft::ui::dialog::StackDialog;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::CommandStates;
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::asset_export::{menu_rows, menu_states};
use crate::dialogs::ExportForScreensDialog;
use crate::tests::panel_harness::Panel;
use crate::{Services, VectorcraftApp};

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

const SUBFOLDERS: &str = "ui.assetExport.subfolders";
const OPEN_LOCATION: &str = "ui.assetExport.openLocation";
const APPLY_PRESET: &str = "ui.assetExport.applyPreset";

/// An app with two unstroked squares; `web` gives it a download service (recorded in the log).
fn app(web: bool) -> (VectorcraftApp, Log, [u64; 2]) {
    let log = Log::default();
    let l = log.clone();
    let mut services = Services::default();
    if web {
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
            l.borrow_mut().push((name.to_string(), bytes.to_vec()));
            Ok(())
        }));
    }
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.session.paint.stroke = vectorcraft_color::Paint::None;
    let mut rect = |x: f64| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let ids = [rect(10.0), rect(100.0)];
    (app, log, ids)
}

/// The panel over `app`, drawn once so its document state exists.
fn panel(app: VectorcraftApp) -> Panel {
    let mut p = Panel::new(app, super::asset_export::show);
    p.frame();
    p
}

fn run(p: &Panel, id: &str, params: Value) -> Value {
    p.app().run(id, params).unwrap()
}

fn assets(app: &mut VectorcraftApp) -> Vec<Value> {
    app.run("assets.list", json!({})).unwrap()["assets"].as_array().unwrap().clone()
}

/// Draws a row carrying the selected artwork's payload above the Asset Export panel.
fn art_and_assets(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    use craft::ui::contract::{ListDrag, ShowCollection as _};
    use craft::ui::list::{List, ListRow};

    let st = app.session.active().unwrap();
    let payload = crate::panel_drag::PanelDrag::Art { document: st.uid, reverts: st.reverts, ids: st.selection.objects.clone() };
    ui.allocate_ui(egui::vec2(ui.available_width(), 80.0), |ui| {
        let keys = craft::behavior::keyed(&[0_usize], |key: &usize| *key);
        List::<usize>::new("Artwork")
            .drag_payload(move |_: &ListDrag<usize>| Some(payload.clone()))
            .show_items(ui, &keys, &mut |_, _| ListRow::new("Dragged artwork"));
    });
    super::asset_export::show(app, ui);
}

#[test]
fn asset_export_accepts_artwork_only_from_its_document() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let mut rectangle = |x| {
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap()
    };
    let first = rectangle(10);
    let second = rectangle(70);
    assert_eq!(first, second, "the two documents have different artwork with the same local ID");
    assert!(app.session.set_active(0));
    let source = app.session.active().unwrap().uid;

    let mut p = Panel::new(app, art_and_assets);
    let from = p.rect("Dragged artwork").center();
    let to = p.rect("No assets").center();
    p.drag_hold_role("Dragged artwork", craft::ui::contract::Role::LIST_ITEM, to - from);
    let drag = craft::ui::runtime::drag::active::<crate::panel_drag::PanelDrag>(p.ctx());
    assert!(matches!(drag.as_deref(), Some(crate::panel_drag::PanelDrag::Art { document, .. }) if *document == source));
    assert!(craft::ui::runtime::drag::mark(p.ctx()).is_some(), "Asset Export accepts the artwork before switching documents");
    assert!(p.app().session.set_active(1));
    p.frame();
    let drag = craft::ui::runtime::drag::active::<crate::panel_drag::PanelDrag>(p.ctx());
    assert!(matches!(drag.as_deref(), Some(crate::panel_drag::PanelDrag::Art { document, .. }) if *document == source));
    assert!(craft::ui::runtime::drag::mark(p.ctx()).is_none(), "Asset Export withdraws the drop indicator when the artwork is foreign");
    p.release();
    assert!(assets(&mut p.app()).is_empty(), "the foreign drag must never collect the active document's same-ID artwork");

    p.drag("Dragged artwork", "No assets", egui::Modifiers::NONE);
    let list = assets(&mut p.app());
    assert_eq!(list.len(), 1, "dropping artwork from the active document still collects it");
    assert_eq!(list[0]["nodes"], json!([second]));
}

/// The document's export settings, which the panel and Export for Screens share.
fn settings(p: &Panel) -> Value {
    run(p, "document.exportSettings", json!({}))["settings"].clone()
}

/// The settings of the saved export preset `name`.
fn saved_preset(p: &Panel, name: &str) -> Value {
    let app = p.app();
    let preset = app.session.prefs.screen_export_presets.iter().find(|x| x.name == name).expect("saved");
    serde_json::to_value(&preset.settings).unwrap()
}

/// The check of the panel-menu row `id` with `params`; `None` for a row without a check or without a state.
fn check(p: &Panel, id: &str, params: Value) -> Option<bool> {
    let mut states = CommandStates::new();
    menu_states(&p.app(), &mut states);
    // States match their rows by id and params, and a row without params holds `Null`, not `{}`.
    let command = if params == json!({}) { CommandRef::new(id) } else { CommandRef::with_params(id, params) };
    states.get(&command).and_then(|s| s.is_checked())
}

/// Whether the panel menu lists the row `id` with `params`.
fn has_row(p: &Panel, id: &str, params: Value) -> bool {
    fn walk(rows: &[PanelRow], key: &str) -> bool {
        rows.iter().any(|row| match row {
            PanelRow::Command(c) | PanelRow::Alternated(c, _) => c.key() == key,
            PanelRow::Submenu(_, rows) => walk(rows, key),
            _ => false,
        })
    }
    walk(&menu_rows(&p.app()), &CommandRef::with_params(id, params).key())
}

fn apply_preset(p: &Panel, name: &str) {
    run(p, APPLY_PRESET, json!({"name": name}));
}

fn preset_checked(p: &Panel, name: &str) -> bool {
    check(p, APPLY_PRESET, json!({"name": name})) == Some(true)
}

fn save_changes(p: &Panel) {
    run(p, "ui.assetExport.saveChanges", json!({}));
}

/// Save As Preset…: the panel's name row, typed into.
fn save_preset_as(p: &mut Panel, name: &str) {
    run(p, "ui.assetExport.savePresetAs", json!({}));
    p.frame();
    assert!(p.shows("Name:"));
    p.type_into("Name:", name);
    if p.shows("Save") {
        p.click("Save");
    }
}

/// The values of the open Export for Screens dialog.
fn screens(app: &VectorcraftApp) -> Option<Value> {
    app.dialogs()?.with(|host| host.get::<ExportForScreensDialog>().and_then(|d| d.json_fields()))
}

/// Closes Export for Screens with OK (and runs its result) or Cancel; whether it was open.
fn close_screens(p: &Panel, ok: bool) -> bool {
    let ctx = p.ctx().clone();
    let host = p.app().dialogs().expect("a window");
    let closed = host.with(|h| if ok { h.confirm::<ExportForScreensDialog>(&ctx) } else { h.cancel::<ExportForScreensDialog>(&ctx) });
    if !ok {
        return closed;
    }
    crate::dialogs::host::pump(&mut p.app(), &ctx);
    // The dialog holds its OK: the pump accepts the filed answer, and OK then closes it with it.
    if closed || !host.with(|h| h.confirm::<ExportForScreensDialog>(&ctx)) {
        return closed;
    }
    crate::dialogs::host::pump(&mut p.app(), &ctx);
    true
}

/// Delivers the scripted pick's answer to the panel's pending export.
#[cfg(not(target_arch = "wasm32"))]
fn finish_pick(p: &Panel) {
    let ctx = p.ctx().clone();
    crate::tests::picks::poll(&mut p.app(), &ctx);
}

/// Picks `to` in the format select named `select` (the first row's "Format", or "3D Format:").
fn pick_format(p: &mut Panel, select: &str, to: &str) {
    p.click(select);
    p.click(to);
}

fn decoded(format: craft_image_codecs::Format, bytes: &[u8]) -> image::DynamicImage {
    craft_image_codecs::decode_as(format, bytes)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
}

#[test]
fn the_rows_are_the_shared_export_settings() {
    let (mut app, log, [a, b]) = app(true);
    app.run("assets.add", json!({"ids": [a, b]})).unwrap();
    let mut p = panel(app);
    // + Add Scale adds a 2x row to the document's export settings (Export for Screens' too).
    p.click("+ Add Scale");
    let saved = settings(&p);
    assert_eq!(saved["formats"].as_array().unwrap().len(), 2);
    assert_eq!(saved["formats"][1]["scale"], "2x");
    run(&p, "file.exportForScreens", json!({}));
    assert_eq!(screens(&p.app()).unwrap()["formats"][1]["scale"], "2x");
    assert!(close_screens(&p, false));
    // A preset set by an agent shows as its rows.
    run(&p, "assets.settings.set", json!({"preset": "mobile"}));
    p.frame();
    // "@3x" is the third row's Suffix: a field's text, not a name.
    assert!(p.shows("iOS") && p.shows_value("@3x"));
    // Export (nothing selected: every asset) downloads one zip on the web.
    p.click("Export");
    let log = log.borrow();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, "Untitled-1.zip");
}

/// The panel menu applies and saves presets, and lists presets imported while it is open.
#[test]
fn the_preset_menu_applies_saves_and_lists_presets() {
    let (mut app, _, [a, _]) = app(true);
    app.run("assets.add", json!({"ids": [a]})).unwrap();
    let mut p = panel(app);
    assert!(!p.shows("Export Preset:"));
    apply_preset(&p, "Logo Package");
    let applied = settings(&p);
    assert_eq!(applied["formats"].as_array().map(Vec::len), Some(5), "{applied}");
    assert!(preset_checked(&p, "Logo Package"), "the menu checks the applied preset");
    // Save: the shared settings under a name of their own.
    save_preset_as(&mut p, "Mine");
    let mine = saved_preset(&p, "Mine");
    assert_eq!(mine["formats"], applied["formats"]);
    // Create Sub-folders, off after a preset that sorts by format, is saved with it.
    assert_eq!((mine.get("folders"), mine.get("subfolders")), (Some(&json!("format")), Some(&json!(false))), "{mine:?}");
    assert!(preset_checked(&p, "Mine"), "the menu checks the saved preset");
    // A preset file opened while the panel shows (the web's Import) is in its menu.
    let data = run(&p, "document.exportPresets.export", json!({"names": ["Mine"]}))["data"].as_str().unwrap().to_string();
    crate::io::open_bytes(&mut p.app(), "pack.vcexportpresets", data.as_bytes(), None).unwrap();
    assert!(has_row(&p, APPLY_PRESET, json!({"name": "Mine 2"})));
    // A mobile preset with no sub-folders is saved again with no sub-folders.
    run(&p, "document.exportPresets.save", json!({"name": "Flat", "settings": {"preset": "mobile", "folders": "none"}}));
    apply_preset(&p, "Flat");
    save_preset_as(&mut p, "Flat Again");
    let again = saved_preset(&p, "Flat Again");
    assert_eq!(
        (again.get("preset"), again.get("folders"), again.get("subfolders")),
        (Some(&json!("mobile")), Some(&json!("none")), Some(&json!(false))),
        "{again:?}"
    );
}

#[test]
fn the_panel_saves_mobile_preset_without_subfolders() {
    let (mut app, _, [a, _]) = app(true);
    app.run("assets.add", json!({"ids": [a]})).unwrap();
    let asset = app.run("assets.list", json!({})).unwrap()["assets"][0]["id"].as_u64().unwrap();
    app.run("assets.settings.set", json!({"preset": "mobile", "subfolders": false})).unwrap();
    let mut p = panel(app);
    save_preset_as(&mut p, "Flat Mobile");

    let saved = saved_preset(&p, "Flat Mobile");
    assert_eq!(saved.get("folders"), Some(&json!("none")));
    assert_eq!(saved.get("subfolders"), Some(&json!(false)));

    let out = run(&p, "document.exportForScreens", json!({"exportPreset": "Flat Mobile", "assets": [asset]}));
    let files = out["files"].as_array().unwrap();
    assert_eq!(files.len(), 3, "{out}");
    assert!(files.iter().all(|f| !f["name"].as_str().unwrap().contains('/')), "{out}");

    run(&p, "assets.settings.set", json!({"exportPreset": "Flat Mobile"}));
    let shared = settings(&p);
    assert_eq!(shared["folders"], "none");
    assert_eq!(shared["subfolders"], false);
}

#[test]
fn asset_export_preserves_applied_preset_naming_and_folders_when_saving() {
    let (mut app, _, [a, _]) = app(true);
    app.run("assets.add", json!({"ids": [a]})).unwrap();
    let asset = app.run("assets.list", json!({})).unwrap()["assets"][0]["id"].as_u64().unwrap();
    let original = json!({
        "formats": [{"format": "svg"}],
        "name": "<name>-###",
        "folders": "format",
        "counterStart": 7,
        "counterPer": "file",
        "includeBleed": true,
    });
    app.run("document.exportPresets.save", json!({"name": "Named Assets", "settings": original})).unwrap();
    let before = app.run("document.exportForScreens", json!({"exportPreset": "Named Assets", "assets": [asset]})).unwrap();

    let p = panel(app);
    apply_preset(&p, "Named Assets");
    assert!(preset_checked(&p, "Named Assets"));
    save_changes(&p);

    let saved = saved_preset(&p, "Named Assets");
    for key in ["name", "folders", "counterStart", "counterPer", "includeBleed"] {
        assert_eq!(saved.get(key), original.get(key), "{key}: {saved:?}");
    }
    let file = run(&p, "document.exportPresets.export", json!({"names": ["Named Assets"]}));
    let data: Value = serde_json::from_str(file["data"].as_str().unwrap()).unwrap();
    for key in ["name", "folders", "counterStart", "counterPer", "includeBleed"] {
        assert_eq!(data["presets"][0]["settings"].get(key), original.get(key), "{key}: {data}");
    }
    let after = run(&p, "document.exportForScreens", json!({"exportPreset": "Named Assets", "assets": [asset]}));
    assert_eq!(after["files"][0]["name"], before["files"][0]["name"]);
}

#[test]
fn asset_export_replaces_applied_format_choice_when_saving() {
    let (mut app, _, _) = app(true);
    app.run(
        "document.exportPresets.save",
        json!({
            "name": "Named Rows",
            "settings": {"formats": [{"format": "svg"}], "name": "<name>-###"}
        }),
    )
    .unwrap();
    let p = panel(app);
    apply_preset(&p, "Named Rows");
    run(&p, "assets.settings.set", json!({"preset": "mobile"}));
    save_changes(&p);

    let saved = saved_preset(&p, "Named Rows");
    assert_eq!(saved.get("preset"), Some(&json!("mobile")));
    assert!(saved.get("formats").is_none(), "{saved:?}");
    assert_eq!(saved.get("name"), Some(&json!("<name>-###")));

    run(&p, "assets.settings.set", json!({"exportPreset": "Named Rows"}));
    run(&p, "file.exportForScreens", json!({}));
    assert_eq!(screens(&p.app()).unwrap()["formats"], json!(vectorcraft_engine::cmd::fileio::screen_preset_rows("mobile").unwrap()));
    assert!(close_screens(&p, false));

    run(&p, "document.exportPresets.save", json!({"name": "Mobile First", "settings": {"preset": "mobile", "name": "<name>-###"}}));
    apply_preset(&p, "Mobile First");
    run(&p, "assets.settings.set", json!({"formats": [{"format": "svg"}]}));
    save_changes(&p);
    let saved = saved_preset(&p, "Mobile First");
    assert_eq!(saved.get("formats"), Some(&json!([{"format": "svg"}])));
    assert!(saved.get("preset").is_none(), "{saved:?}");
    assert_eq!(saved.get("name"), Some(&json!("<name>-###")));
}

#[test]
fn asset_export_saves_changed_rows_with_applied_preset_fields() {
    let (mut app, _, _) = app(true);
    let original = json!({
        "formats": [{"format": "svg"}],
        "name": "<name>-###",
        "counterStart": 7,
        "counterPer": "file",
        "folder": "exports",
        "includeBleed": true,
    });
    app.run("document.exportPresets.save", json!({"name": "Named Rows", "settings": original})).unwrap();
    let p = panel(app);
    apply_preset(&p, "Named Rows");
    run(&p, "assets.settings.set", json!({"formats": [{"format": "png", "scale": "2x"}]}));
    save_changes(&p);

    let saved = saved_preset(&p, "Named Rows");
    assert_eq!(saved["formats"], json!([{"format": "png", "scale": "2x"}]));
    for key in ["name", "counterStart", "counterPer", "folder", "includeBleed"] {
        assert_eq!(saved.get(key), original.get(key), "{key}: {saved:?}");
    }
}

#[test]
fn asset_export_saves_changed_subfolders_from_applied_preset() {
    let (mut app, _, _) = app(true);
    app.run("document.exportPresets.save", json!({"name": "Grouped Mobile", "settings": {"preset": "mobile", "folders": "format"}})).unwrap();
    let p = panel(app);
    apply_preset(&p, "Grouped Mobile");
    run(&p, "assets.settings.set", json!({"subfolders": true}));
    assert_eq!(settings(&p)["folders"], "rows");
    save_changes(&p);

    let saved = saved_preset(&p, "Grouped Mobile");
    assert_eq!(saved.get("folders"), Some(&json!("rows")));
    assert_eq!(saved.get("subfolders"), Some(&json!(true)));
    run(&p, "assets.settings.set", json!({"exportPreset": "Grouped Mobile"}));
    assert_eq!(settings(&p)["folders"], "rows");
}

#[test]
fn asset_export_preserves_applied_mobile_preset_folder_choice() {
    let (mut app, _, _) = app(true);
    app.run("document.exportPresets.save", json!({"name": "Mobile by Format", "settings": {"preset": "mobile", "folders": "format"}})).unwrap();
    let p = panel(app);
    apply_preset(&p, "Mobile by Format");
    assert!(preset_checked(&p, "Mobile by Format"));
    save_changes(&p);
    let saved = saved_preset(&p, "Mobile by Format");
    assert_eq!(saved.get("folders"), Some(&json!("format")));
    assert_eq!(saved.get("subfolders"), Some(&json!(false)));
}

#[test]
fn the_preset_menu_saves_changes_and_opens_import_and_export() {
    let (mut app, log, _) = app(true);
    app.run("document.exportPresets.save", json!({"name": "Pack", "settings": {"formats": [{"format": "svg"}]}})).unwrap();
    let picker = crate::tests::picks::scripted(&mut app);
    let p = panel(app);
    apply_preset(&p, "Pack");
    assert!(preset_checked(&p, "Pack"));
    run(&p, "assets.settings.set", json!({"formats": [{"format": "png", "scale": "2x"}]}));
    save_changes(&p);
    assert_eq!(saved_preset(&p, "Pack")["formats"][0]["format"], "png");
    run(&p, "ui.assetExport.exportPreset", json!({}));
    assert_eq!(log.borrow()[0].0, "Pack.vcexportpresets");
    run(&p, "ui.assetExport.importPreset", json!({}));
    assert_eq!(picker.requests().len(), 1, "Import Preset… asks for a file");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_writes_into_the_picked_folder() {
    let dir = vectorcraft_testkit::TempDir::new("asset-panel");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let (mut app, _, [a, _]) = app(false);
    let picker = crate::tests::picks::scripted(&mut app);
    app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
    let mut p = panel(app);
    p.click("Export");
    picker.answer(&[&folder]);
    finish_pick(&p);
    // Create Sub-folders is on by default: the 1x row's folder.
    assert!(dir.join("1x").join("Badge.png").is_file(), "{:?}", std::fs::read_dir(&dir).map(|d| d.count()));
}

/// The panel menu's Create Sub-folders and Open Location after Export are check rows, on by
/// default, that change the settings the panel shares with Export for Screens, and the panel's
/// Export follows them.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_menu_sets_create_subfolders_and_open_location_and_export_follows_them() {
    let dir = vectorcraft_testkit::TempDir::new("asset-panel-menu");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let (mut app, _, [a, _]) = app(false);
    let picker = crate::tests::picks::scripted(&mut app);
    let shown = Rc::new(RefCell::new(Vec::<String>::new()));
    let s = shown.clone();
    app.services.reveal = Some(Box::new(move |path: &str| {
        s.borrow_mut().push(path.to_string());
        Ok(())
    }));
    app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
    let mut p = panel(app);
    assert_eq!((check(&p, SUBFOLDERS, json!({})), check(&p, OPEN_LOCATION, json!({}))), (Some(true), Some(true)));
    assert!(has_row(&p, "ui.assetExport.formatSettings", json!({})));
    let export = |p: &mut Panel| {
        p.click("Export");
        picker.answer(&[&folder]);
        finish_pick(p);
    };
    // Both on: the 1x row's folder, and the file manager shows the file.
    export(&mut p);
    assert!(dir.join("1x").join("Badge.png").is_file());
    assert_eq!(shown.borrow().len(), 1);
    // Both turned off.
    run(&p, SUBFOLDERS, json!({}));
    run(&p, OPEN_LOCATION, json!({}));
    let shared = settings(&p);
    assert_eq!((&shared["subfolders"], &shared["openLocation"]), (&json!(false), &json!(false)), "{shared}");
    assert_eq!((check(&p, SUBFOLDERS, json!({})), check(&p, OPEN_LOCATION, json!({}))), (Some(false), Some(false)));
    // Export for Screens opens on the same choices.
    run(&p, "file.exportForScreens", json!({}));
    let d = screens(&p.app()).unwrap();
    assert_eq!((&d["subfolders"], &d["openLocation"]), (&json!(false), &json!(false)));
    assert!(close_screens(&p, false));
    let _ = std::fs::remove_dir_all(&dir);
    export(&mut p);
    assert!(dir.join("Badge.png").is_file() && !dir.join("1x").exists());
    assert_eq!(shown.borrow().len(), 1, "no file manager after export");
    // Create Sub-folders on again: a folder per row.
    run(&p, SUBFOLDERS, json!({}));
    let shared = settings(&p);
    assert_eq!((&shared["subfolders"], &shared["folders"]), (&json!(true), &json!("rows")), "{shared}");
    // Only true and false are taken.
    for bad in [json!({"openLocation": "yes"}), json!({"subfolders": 1})] {
        assert!(p.app().run("assets.settings.set", bad.clone()).is_err(), "{bad}");
    }
}

/// A folder per format, remembered by Export for Screens or a preset, is used by the panel's Export
/// too.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_uses_a_remembered_folder_per_format() {
    let dir = vectorcraft_testkit::TempDir::new("asset-panel-format");
    let folder = dir.to_string_lossy().replace('\\', "/");
    let (mut app, _, [a, _]) = app(false);
    let picker = crate::tests::picks::scripted(&mut app);
    let asset = app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap()["assets"][0].clone();
    // An export as Export for Screens sends it with Create Sub-folders by Format, which the
    // document remembers.
    let formats = json!([{"format": "png"}, {"format": "svg"}]);
    app.run("document.exportForScreens", json!({"assets": [asset], "formats": formats, "folders": "format", "subfolders": false})).unwrap();
    let mut p = panel(app);
    assert_eq!(check(&p, SUBFOLDERS, json!({})), Some(true));
    p.click("Export");
    picker.answer(&[&folder]);
    finish_pick(&p);
    assert!(dir.join("png").join("Badge.png").is_file() && dir.join("svg").join("Badge.svg").is_file());
}

/// Remembered empty folders are none, as an export reads them: Create Sub-folders is off in the
/// panel menu and in Export for Screens.
#[test]
fn empty_remembered_folders_are_create_subfolders_off() {
    let (mut app, _, [a, _]) = app(true);
    let asset = app.run("assets.add", json!({"ids": [a]})).unwrap()["assets"][0].clone();
    app.run("document.exportForScreens", json!({"assets": [asset], "formats": [{"format": "svg"}], "folders": "", "subfolders": true})).unwrap();
    let p = panel(app);
    assert_eq!(check(&p, SUBFOLDERS, json!({})), Some(false));
    run(&p, "file.exportForScreens", json!({}));
    assert_eq!(screens(&p.app()).unwrap()["subfolders"], false);
}

/// Open Location after Export turned off in a document without settings leaves Create Sub-folders
/// on, in the panel menu and in Export for Screens.
#[test]
fn open_location_off_leaves_create_subfolders_on() {
    let (app, _, _) = app(false);
    let p = panel(app);
    run(&p, OPEN_LOCATION, json!({}));
    assert_eq!((check(&p, SUBFOLDERS, json!({})), check(&p, OPEN_LOCATION, json!({}))), (Some(true), Some(false)));
    run(&p, "file.exportForScreens", json!({}));
    let d = screens(&p.app()).unwrap();
    assert_eq!((&d["subfolders"], &d["openLocation"]), (&json!(true), &json!(false)));
}

/// Editing the rows of a document without a Create Sub-folders choice keeps it on, in Export for
/// Screens too; a preset that turns it off shows it off, and the menu turns it on again.
#[test]
fn row_edits_keep_create_subfolders_on_and_a_preset_can_turn_it_off() {
    let (mut app, _, [a, _]) = app(true);
    app.run("assets.add", json!({"ids": [a]})).unwrap();
    let mut p = panel(app);
    p.click("+ Add Scale");
    assert_eq!(check(&p, SUBFOLDERS, json!({})), Some(true));
    run(&p, "file.exportForScreens", json!({}));
    assert_eq!(screens(&p.app()).unwrap()["subfolders"], true);
    assert!(close_screens(&p, false));
    // Logo Package sorts by format: checked, as in Export for Screens. Turned off, the sorting
    // ends; turned on again, a folder per row.
    run(&p, "assets.settings.set", json!({"exportPreset": "Logo Package"}));
    assert_eq!(check(&p, SUBFOLDERS, json!({})), Some(true));
    run(&p, SUBFOLDERS, json!({}));
    let shared = settings(&p);
    assert_eq!((&shared["subfolders"], &shared["folders"]), (&json!(false), &json!("none")), "{shared}");
    run(&p, "file.exportForScreens", json!({}));
    assert_eq!(screens(&p.app()).unwrap()["subfolders"], false);
    assert!(close_screens(&p, false));
    run(&p, SUBFOLDERS, json!({}));
    let shared = settings(&p);
    assert_eq!((&shared["subfolders"], &shared["folders"]), (&json!(true), &json!("rows")), "{shared}");
    assert_eq!(check(&p, SUBFOLDERS, json!({})), Some(true));
    // The web downloads the files: the menu has no Open Location after Export.
    assert!(!has_row(&p, OPEN_LOCATION, json!({})));
}

/// Format Settings… in the panel menu opens Export for Screens on its Assets tab, showing the
/// first row's format settings.
#[test]
fn format_settings_opens_the_first_rows_format_settings() {
    let (mut app, _, [a, b]) = app(true);
    app.run("assets.add", json!({"ids": [a, b], "multiple": true})).unwrap();
    app.run("assets.settings.set", json!({"formats": [{"format": "jpeg"}, {"format": "svg"}]})).unwrap();
    let p = panel(app);
    run(&p, "ui.assetExport.formatSettings", json!({}));
    let d = screens(&p.app()).expect("Export for Screens");
    assert_eq!((&d["tab"], &d["__settings"]), (&json!("assets"), &json!("jpg")));
    assert_eq!(d["assets"].as_array().map(Vec::len), Some(2));
}

#[test]
fn tiff_dropdown_exports_only_the_selected_asset_and_shares_settings() {
    let (mut app, log, [a, b]) = app(true);
    app.run("paint.setFill", json!({"ids": [a], "color": "#c03020"})).unwrap();
    app.run("paint.setFill", json!({"ids": [b], "color": "#2040c0"})).unwrap();
    app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
    app.run("assets.add", json!({"ids": [b], "name": "Other"})).unwrap();
    app.run(
        "assets.settings.set",
        json!({
            "formats": [{"format": "png", "scale": "2x", "suffix": "@2x"}],
            "settings": {"tiff": {"byteOrder": "big", "lzw": false, "embedIcc": false}}
        }),
    )
    .unwrap();
    let mut p = panel(app);
    pick_format(&mut p, "Format", "TIFF");
    let saved = settings(&p);
    assert_eq!(saved["formats"], json!([{"format": "tiff", "scale": "2x", "suffix": "@2x"}]));
    p.click("Badge");
    p.click("Export 1 Selected");
    {
        let log = log.borrow();
        let [(name, bytes)] = log.as_slice() else { panic!("expected only the selected asset") };
        assert_eq!(name, "Badge@2x.tif");
        assert_eq!(&bytes[..4], b"MM\0*", "the saved TIFF byte order reaches asset export");
        let img = decoded(craft_image_codecs::Format::Tiff, bytes).to_rgba8();
        assert_eq!(img.dimensions(), (60, 40), "the selected 30x20 asset at 2x");
        let pixel = img.get_pixel(30, 20).0;
        assert!(pixel[0] > pixel[1] && pixel[0] > pixel[2] && pixel[3] == 255, "the red Badge, not the blue Other asset: {pixel:?}");
    }
    run(&p, "file.exportForScreens", json!({}));
    let d = screens(&p.app()).unwrap();
    assert_eq!(d["formats"], saved["formats"]);
    assert_eq!(d["settings"]["tiff"]["byteOrder"], "big");
    assert_eq!(d["settings"]["tiff"]["lzw"], false);
    assert_eq!(d["settings"]["tiff"]["embedIcc"], false);
}

/// TIFF tag 262 identifies RGB (2) or separated CMYK (5), independently of a decoder's conversion.
fn tiff_photometric(bytes: &[u8]) -> u16 {
    let big = match &bytes[..4] {
        b"II*\0" => false,
        b"MM\0*" => true,
        magic => panic!("not TIFF: {magic:?}"),
    };
    let short = |at: usize| {
        let b = bytes[at..at + 2].try_into().unwrap();
        if big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    let long = |at: usize| {
        let b = bytes[at..at + 4].try_into().unwrap();
        if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
    };
    let dir = long(4) as usize;
    let entry = (0..usize::from(short(dir))).map(|i| dir + 2 + 12 * i).find(|&at| short(at) == 262).expect("PhotometricInterpretation");
    assert_eq!((short(entry + 2), long(entry + 4)), (3, 1), "one SHORT");
    short(entry + 8)
}

/// Remembered format settings that hold keys the export no longer reads, and settings for a format
/// it doesn't write, are left out of the panel's export, so it is not refused.
#[test]
fn export_leaves_out_remembered_keys_no_longer_read() {
    let (mut app, log, [a, _]) = app(true);
    app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
    let serde_json::Value::Object(saved) = json!({
        "formats": [{"format": "tiff", "scale": "1x", "noSuchOption": 1}],
        "settings": {"png": {"noSuchSetting": true}, "dxf": {}, "tiff": {"lzw": false, "noSuchSetting": 1}}
    }) else {
        panic!("an object")
    };
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).export_settings = saved;
    let mut p = panel(app);
    p.click("Export");
    let log = log.borrow();
    let [(name, _)] = log.as_slice() else { panic!("expected one TIFF: {}", p.app().ui.status) };
    assert_eq!(name, "Badge.tif");
}

#[test]
fn tiff_cmyk_defaults_match_screens_and_preserve_explicit_rgb() {
    for rgb_override in [false, true] {
        let mut models = vec![];
        for screens in [false, true] {
            let (mut app, log, [a, _]) = app(true);
            app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
            app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
            assert!(app.session.active().unwrap().doc.export_settings.get("settings").is_none(), "fresh format settings");
            if rgb_override {
                app.run("assets.settings.set", json!({"settings": {"tiff": {"colorModel": "rgb"}}})).unwrap();
            }
            let mut p = panel(app);
            pick_format(&mut p, "Format", "TIFF");
            if !rgb_override {
                assert!(p.app().session.active().unwrap().doc.export_settings.get("settings").and_then(|s| s.get("tiff")).is_none());
            }
            if screens {
                // The row's Settings edit opens Format Settings on the Assets tab.
                p.click("Format Settings");
                let d = super::tests_asset_export::screens(&p.app()).expect("the panel gear opens Format Settings");
                assert_eq!((&d["tab"], &d["__settings"]), (&json!("assets"), &json!("tiff")));
                assert_eq!(d["settings"]["tiff"]["colorModel"], if rgb_override { "rgb" } else { "cmyk" });
                assert!(close_screens(&p, true));
                assert!(super::tests_asset_export::screens(&p.app()).is_none());
            } else {
                p.click("Export");
            }
            let log = log.borrow();
            let [(name, bytes)] = log.as_slice() else { panic!("expected one TIFF: screens={screens}, rgb_override={rgb_override}") };
            assert_eq!(name, "Badge.tif");
            models.push(tiff_photometric(bytes));
        }
        assert_eq!(models, vec![if rgb_override { 2 } else { 5 }; 2], "direct panel and screens; explicit RGB={rgb_override}");
    }
}

#[test]
fn tiff_panel_preserves_shared_antialias_and_explicit_overrides() {
    for (format_aa, row_aa, effective_aa) in [(None, None, "none"), (Some("art"), None, "art"), (Some("art"), Some("none"), "none")] {
        let (mut app, log, _) = app(true);
        // The fractional ellipse reuses the raster-export hard/smooth edge oracle.
        let id = app.run("shape.ellipse", json!({"x": 5.3, "y": 4.7, "width": 25.8, "height": 21.5})).unwrap()["id"].clone();
        app.run("assets.add", json!({"ids": [id], "name": "Edge"})).unwrap();
        let mut row = json!({"format": "png", "scale": "1x", "suffix": ""});
        if let Some(aa) = row_aa {
            row["antiAlias"] = json!(aa);
        }
        let other_settings = json!({"png": {"antiAlias": "type", "interlaced": true}, "jpg": {"quality": 80}});
        let mut shared_settings = other_settings.clone();
        if let Some(aa) = format_aa {
            shared_settings["tiff"] = json!({"antiAlias": aa});
        }
        app.run("assets.settings.set", json!({"formats": [row], "antiAlias": "none", "settings": shared_settings})).unwrap();
        let mut p = panel(app);
        pick_format(&mut p, "Format", "TIFF");
        p.click("Export");
        {
            let log = log.borrow();
            let [(name, bytes)] = log.as_slice() else { panic!("expected one TIFF: format={format_aa:?}, row={row_aa:?}") };
            assert_eq!(name, "Edge.tif");
            let img = decoded(craft_image_codecs::Format::Tiff, bytes).to_rgba8();
            assert!(img.pixels().any(|p| p[3] == 255), "the ellipse is present");
            assert!(img.pixels().any(|p| p[3] == 0), "transparent outside the ellipse");
            let hard = img.pixels().all(|p| p[3] == 0 || p[3] == 255);
            assert_eq!(hard, effective_aa == "none", "effective anti-aliasing: format={format_aa:?}, row={row_aa:?}");
        }
        let saved = settings(&p);
        assert_eq!(saved["antiAlias"], "none", "the shared choice stays saved");
        assert_eq!(saved["settings"], shared_settings, "direct export preserves saved format options, including an absent TIFF map");
        assert_eq!(saved["formats"][0]["format"], "tiff");
        assert_eq!(saved["formats"][0].get("antiAlias").and_then(Value::as_str), row_aa, "row overrides stay on the row");
        for format in ["png", "jpg"] {
            assert_eq!(saved["settings"][format], other_settings[format], "other format choices survive");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn tiff_saved_aliases_use_document_defaults_and_explicit_rgb() {
    let dir = vectorcraft_testkit::TempDir::new("asset-tiff-aliases");
    // The first case is the historical control: saved tif without an explicit model must be CMYK.
    // Native picked folders let the mixed-row case inspect each file without introducing a ZIP decoder.
    for (case, (alias, rgb_override, mixed)) in
        [("tif", false, false), ("TIFF", false, false), (".tif", false, true), ("tif", true, false)].into_iter().enumerate()
    {
        let folder = dir.join(case.to_string());
        let picked = folder.to_string_lossy().replace('\\', "/");
        let (mut app, _, [a, _]) = app(false);
        let picker = crate::tests::picks::scripted(&mut app);
        app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
        let mut rows = vec![json!({"format": alias, "scale": "2x", "suffix": "@2x"})];
        if mixed {
            rows.push(json!({"format": "png", "scale": "1x", "suffix": ""}));
        }
        let mut shared = json!({"png": {"interlaced": true}});
        if rgb_override {
            shared["tiff"] = json!({"colorModel": "rgb"});
        }
        let saved = json!({"formats": rows, "settings": shared});
        app.run("assets.settings.set", saved.clone()).unwrap();
        let mut p = panel(app);
        // Keep the valid saved spelling; choosing the TIFF dropdown would hide the alias bug.
        p.click("Export");
        picker.answer(&[&picked]);
        finish_pick(&p);
        let bytes = std::fs::read(folder.join("Badge@2x.tif")).unwrap();
        assert_eq!(tiff_photometric(&bytes), if rgb_override { 2 } else { 5 }, "saved alias {alias}, explicit RGB={rgb_override}");
        if rgb_override {
            let img = decoded(craft_image_codecs::Format::Tiff, &bytes);
            assert_eq!((img.width(), img.height()), (60, 40), "the saved TIFF row is still 2x");
        }
        if mixed {
            let png = std::fs::read(folder.join("Badge.png")).unwrap();
            let img = decoded(craft_image_codecs::Format::Png, &png);
            assert_eq!((img.width(), img.height()), (30, 20), "the PNG row keeps its own 1x scale");
            assert_eq!(png.get(28), Some(&1), "the saved PNG interlace option survives the TIFF defaults");
        }
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), if mixed { 2 } else { 1 });
        assert_eq!(settings(&p), saved, "direct export preserves saved aliases and explicit options");
    }
}

/// Export 3D Object in the panel menu collects the selected 3D object; its tile is marked 3D, the
/// Format row sets its mesh format, and Export writes it as a mesh file beside the pictures.
#[test]
fn a_3d_asset_takes_its_format_from_the_row_and_exports_as_a_mesh() {
    let (mut app, log, [a, b]) = app(true);
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "params": {"depth": 20}})).unwrap();
    app.run("layer.setProps", json!({"id": a, "name": "Box"})).unwrap();
    app.run("assets.add", json!({"ids": [b], "name": "Flat"})).unwrap();
    let mut p = panel(app);
    // Without a 3D asset there is no 3D Format row.
    assert!(!p.shows("3D Format:"));
    run(&p, "ui.assetExport.add3D", json!({}));
    p.frame();
    let list = assets(&mut p.app());
    assert_eq!((list[1]["name"].clone(), list[1]["format3D"].clone()), (json!("Box"), json!("usda")));
    assert!(p.shows("3D Format:"));
    pick_format(&mut p, "3D Format:", "OBJ");
    assert_eq!(assets(&mut p.app())[1]["format3D"], "obj");
    // The menu left the new 3D asset selected: Export writes its OBJ and MTL alone, as one zip.
    p.click("Export 1 Selected");
    {
        let log = log.borrow();
        let [(name, zip)] = log.as_slice() else { panic!("{:?} {}", log.iter().map(|f| &f.0).collect::<Vec<_>>(), p.app().ui.status) };
        assert!(name.ends_with(".zip"), "{name}");
        let has_name = |n: &str| zip.windows(n.len()).any(|w| w == n.as_bytes());
        assert!(has_name("Box.obj") && has_name("Box.mtl"));
    }
    // Every asset: the 3D asset's files first, since they are written at once, then the picture
    // asset as before.
    log.borrow_mut().clear();
    run(&p, "ui.assetExport.selectAll", json!({}));
    p.frame();
    p.click("Export");
    let log = log.borrow();
    assert_eq!(log.iter().map(|f| f.0.ends_with(".zip")).collect::<Vec<_>>(), [true, false], "{:?}", log.iter().map(|f| &f.0).collect::<Vec<_>>());
    assert!(log[1].0.starts_with("Flat"), "{}", log[1].0);
}

/// When the 3D assets fail to export, Export leaves the picture assets unexported and the status
/// bar keeps the 3D error.
#[test]
fn a_failed_3d_export_stops_the_picture_export() {
    let (mut app, log, [a, b]) = app(true);
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "params": {"depth": 20}})).unwrap();
    app.run("layer.setProps", json!({"id": a, "name": "Box"})).unwrap();
    app.run("assets.add3D", json!({"ids": [a]})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.run("effect.remove", json!({"index": 0})).unwrap();
    app.run("assets.add", json!({"ids": [b], "name": "Flat"})).unwrap();
    let mut p = panel(app);
    run(&p, "ui.assetExport.selectAll", json!({}));
    p.frame();
    p.click("Export");
    assert!(log.borrow().is_empty(), "{:?}", log.borrow().iter().map(|f| &f.0).collect::<Vec<_>>());
    assert!(p.app().ui.status.contains("Box"), "{}", p.app().ui.status);
}

/// The panel over `app`'s assets, with the tiles `picked` picked by clicks (Cmd adding after the first).
fn picking(app: VectorcraftApp, picked: &[&str]) -> Panel {
    let mut p = panel(app);
    for (i, name) in picked.iter().enumerate() {
        p.click_with(name, if i == 0 { egui::Modifiers::NONE } else { egui::Modifiers::COMMAND });
    }
    p
}

/// The assets picked in the panel, in id order.
fn picked(p: &Panel) -> Vec<u64> {
    let mut ids = super::asset_export::selected(&p.app());
    ids.sort_unstable();
    ids
}

fn asset_ids(app: &mut VectorcraftApp) -> Vec<u64> {
    assets(app).iter().map(|v| v["id"].as_u64().unwrap()).collect()
}

/// A right-click on a tile outside the picked assets picks it alone; on a picked tile it keeps every picked asset,
/// which the menu's Export and Remove then name. Opening the menu leaves the canvas selection alone.
#[test]
fn right_click_selects_an_unselected_tile_and_preserves_a_multi_selection() {
    for selected_tile in [false, true] {
        let (mut app, _, [a, b]) = app(true);
        app.run("assets.add", json!({"ids": [a, b]})).unwrap();
        let ids = asset_ids(&mut app);
        let mut p = picking(app, if selected_tile { &["Asset 1", "Asset 2"][..] } else { &["Asset 2"][..] });
        let canvas = p.app().session.active().unwrap().selection.objects.clone();
        p.secondary_click("Asset 1");
        assert_eq!(picked(&p), if selected_tile { ids.clone() } else { vec![ids[0]] });
        assert!(p.shows(if selected_tile { "Export 2 Assets…" } else { "Export Asset…" }));
        assert!(p.shows(if selected_tile { "Remove 2 Assets" } else { "Remove Asset" }));
        for name in ["Rename", "Select Artwork on Canvas", "Add Selected Artwork as One Asset", "Add Selected Artwork as Multiple Assets"] {
            assert!(p.shows(name), "the menu has {name}");
        }
        assert_eq!(p.app().session.active().unwrap().selection.objects, canvas, "opening a menu leaves canvas art alone");
    }
}

/// The menu's Remove removes the tile's asset, or every picked one, in one undo step, and keeps the artwork.
#[test]
fn tile_menu_remove_uses_one_undo_step_for_one_or_multiple_assets() {
    for multiple in [false, true] {
        let (mut app, _, [a, b]) = app(true);
        app.run("assets.add", json!({"ids": [a, b]})).unwrap();
        let before = assets(&mut app);
        let mut p = picking(app, if multiple { &["Asset 1", "Asset 2"][..] } else { &[] });
        p.secondary_click("Asset 1");
        p.click(if multiple { "Remove 2 Assets" } else { "Remove Asset" });
        let remaining = assets(&mut p.app());
        assert_eq!(remaining.len(), if multiple { 0 } else { 1 });
        if !multiple {
            assert_eq!(remaining[0]["name"], "Asset 2");
        }
        assert!(!p.shows("Asset 1"));
        assert!(picked(&p).is_empty());
        assert!(p.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(a)).is_some(), "removing an asset keeps the artwork");
        run(&p, "edit.undo", json!({}));
        assert_eq!(assets(&mut p.app()), before, "one undo restores every removed asset");
        run(&p, "edit.redo", json!({}));
        assert_eq!(assets(&mut p.app()), remaining);
    }
}

/// Select Artwork on Canvas selects the tile's asset's objects only, and keeps the picked assets.
#[test]
fn tile_menu_select_artwork_selects_only_the_clicked_assets_nodes() {
    let (mut app, _, [a, b]) = app(true);
    let c = app.run("shape.rectangle", json!({"x": 50, "y": 60, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("assets.add", json!({"ids": [a, b], "multiple": false, "name": "Pair"})).unwrap();
    app.run("assets.add", json!({"ids": [c], "name": "Other"})).unwrap();
    app.run("select.set", json!({"ids": [c]})).unwrap();
    let ids = asset_ids(&mut app);
    let mut p = picking(app, &["Pair", "Other"]);
    p.secondary_click("Pair");
    p.click("Select Artwork on Canvas");
    let mut selection: Vec<u64> = p.app().session.active().unwrap().selection.objects.iter().map(|id| id.0).collect();
    selection.sort_unstable();
    assert_eq!(selection, vec![a, b]);
    assert_eq!(picked(&p), ids, "the canvas selection leaves the picked assets alone");
}

/// The menu's Export writes the tile's asset, or every picked one, and nothing else.
#[test]
fn tile_menu_exports_only_its_targets() {
    for multiple in [false, true] {
        let (mut app, log, [a, b]) = app(true);
        app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
        app.run("assets.add", json!({"ids": [b], "name": "Other"})).unwrap();
        let mut p = picking(app, if multiple { &["Badge", "Other"][..] } else { &[] });
        p.secondary_click("Badge");
        p.click(if multiple { "Export 2 Assets…" } else { "Export Asset…" });
        let log = log.borrow();
        let [(name, bytes)] = log.as_slice() else { panic!("expected one download: {}", p.app().ui.status) };
        if multiple {
            assert!(name.ends_with(".zip"));
            assert!(bytes.windows(9).any(|w| w == b"Badge.png"));
            assert!(bytes.windows(9).any(|w| w == b"Other.png"));
        } else {
            assert_eq!(name, "Badge.png");
        }
    }
}

/// The menu's Rename opens the tile's name field.
#[test]
fn tile_menu_rename_opens_the_clicked_assets_inline_field() {
    let (mut app, _, [a, _]) = app(true);
    app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
    let mut p = panel(app);
    p.secondary_click("Badge");
    p.click("Rename");
    p.key(egui::Key::A, egui::Modifiers::COMMAND);
    p.type_text("Renamed");
    p.key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(assets(&mut p.app())[0]["name"], "Renamed");
}

/// In a panel narrower than the scale row's fields at their full length, the row's Scale, Suffix, Format and Remove
/// share the panel's width, and the Save As Preset row under them stays inside it too.
#[test]
fn the_scale_row_fits_the_panel_width() {
    use craft::ui::contract::Role;
    for width in [300.0, 440.0] {
        let (mut app, _, [a, b]) = app(true);
        app.run("assets.add", json!({"ids": [a, b]})).unwrap();
        let mut p = panel(app);
        p.room(egui::vec2(width, 1600.0));
        p.frame();
        run(&p, "ui.assetExport.savePresetAs", json!({}));
        p.frame();
        let controls = [
            ("Scale", p.rect_role("Scale", Role::COMBO_BOX)),
            ("Suffix", p.rect("Suffix")),
            ("Format", p.rect_role("Format", Role::COMBO_BOX)),
            ("Remove", p.rect("Remove")),
            ("Name:", p.rect_role("Name:", Role::TEXT_INPUT)),
            ("Cancel", p.rect("Cancel")),
        ];
        // The four controls of the scale row are on one line.
        assert_eq!(controls[0].1.center().y, controls[3].1.center().y, "one row at {width}");
        for (name, rect) in controls {
            assert!(rect.left() >= -0.5 && rect.right() <= width + 0.5, "{name} at {rect:?} is outside a {width} pt panel");
        }
    }
}

/// The menu adds the selected artwork as one asset or one per object (an object that is an asset already keeps its
/// asset), and is off with no artwork selected.
#[test]
fn tile_menu_collects_canvas_art_as_one_or_multiple_assets_and_disables_empty_selection() {
    for (selected, single) in [(true, true), (true, false), (false, true), (false, false)] {
        let (mut app, _, [a, b]) = app(true);
        app.run("assets.add", json!({"ids": [a], "name": "Existing"})).unwrap();
        app.run("select.set", json!({"ids": if selected { vec![a, b] } else { vec![] }})).unwrap();
        let mut p = panel(app);
        p.secondary_click("Existing");
        let label = if single { "Add Selected Artwork as One Asset" } else { "Add Selected Artwork as Multiple Assets" };
        assert_eq!(p.enabled(label), selected, "{label}");
        if !selected {
            continue;
        }
        p.click(label);
        let list = assets(&mut p.app());
        assert_eq!(list.len(), 2);
        assert_eq!(list[1]["nodes"], if single { json!([a, b]) } else { json!([b]) });
        assert_eq!(picked(&p).len(), if single { 1 } else { 2 });
    }
}
