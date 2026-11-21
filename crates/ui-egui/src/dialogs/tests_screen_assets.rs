//! Export for Screens' Assets tab and File → Export Selection…, drawn headlessly: collecting the
//! selection, the checked assets, the OK label, desktop folders and web downloads.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::tests_export::{frame, kind, set};
use super::*;
use crate::Services;
use crate::tests_labels::painted_text;

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app with two unstroked squares (selected); `web` gives it a download service (recorded in
/// the log) instead of a file manager.
fn app(web: bool) -> (VectorcraftApp, Log, [u64; 2]) {
    let log = Log::default();
    let l = log.clone();
    let mut services = Services::default();
    if web {
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
            l.borrow_mut().push((name.to_string(), bytes.to_vec()));
            Ok(())
        }));
    } else {
        services.reveal = Some(Box::new(move |path: &str| {
            l.borrow_mut().push((path.to_string(), vec![]));
            Ok(())
        }));
    }
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.session.paint.stroke = vectorcraft_color::Paint::None;
    let mut rect = |x: f64| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let ids = [rect(10.0), rect(100.0)];
    app.run("select.set", json!({"ids": ids})).unwrap();
    (app, log, ids)
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().expect("a dialog is open").fields.get(k).cloned().unwrap_or(Value::Null)
}

fn assets(app: &mut VectorcraftApp) -> Vec<Value> {
    app.run("assets.list", json!({})).unwrap()["assets"].as_array().unwrap().clone()
}

#[test]
fn export_selection_collects_the_selection_and_opens_the_assets_tab() {
    let (mut app, log, _) = app(true);
    assert!(crate::menus::enabled(&app, "file.exportSelection"));
    let r = app.run("file.exportSelection", json!({"multiple": true})).unwrap();
    let list = assets(&mut app);
    assert_eq!(list.len(), 2, "one asset per object");
    let ids: Vec<Value> = list.iter().map(|a| a["id"].clone()).collect();
    assert_eq!(r["assets"], json!(ids));
    assert_eq!(kind(&app), Some(export_for_screens::KIND));
    assert_eq!((field(&app, "tab"), field(&app, "assets")), (json!("assets"), json!(ids)));
    // Collecting the same art again opens on the same assets.
    super::cancel(&mut app);
    app.run("file.exportSelection", json!({"multiple": true})).unwrap();
    assert_eq!(assets(&mut app).len(), 2);
    assert_eq!(field(&app, "assets"), json!(ids));
    let text = painted_text(&mut app, |app, ui| super::show(app, ui.ctx()));
    assert!(text.contains("Asset 1") && text.contains("Asset 2") && text.contains("Clear Selection"), "{text}");
    // The web downloads both as one zip, in the 1x sub-folder Create Sub-folders makes by default.
    let r = confirm(&mut app).unwrap();
    assert_eq!(r["files"], json!(["1x/Asset-1.png", "1x/Asset-2.png"]));
    assert_eq!(log.borrow().len(), 1);
    assert!(app.ui.dialog.is_none());
    // Nothing selected: Export Selection is off; document.exportSelection still exports in one go.
    app.run("select.set", json!({"ids": []})).unwrap();
    assert!(!crate::menus::enabled(&app, "file.exportSelection"));
    app.run("select.set", json!({"ids": [list[0]["nodes"][0]]})).unwrap();
    assert!(app.session.execute("document.exportSelection", &json!({"format": "png"})).unwrap()["dataBase64"].is_string());
}

#[test]
fn export_selection_as_one_asset() {
    // By default the selection, ungrouped, becomes one asset of it all; `multiple: false` says so too.
    for p in [json!({}), json!({"multiple": false})] {
        let (mut app, _, ids) = app(true);
        app.run("file.exportSelection", p.clone()).unwrap();
        let list = assets(&mut app);
        assert_eq!(list.len(), 1, "{p}");
        assert_eq!(list[0]["nodes"], json!(ids), "{p}");
        assert_eq!(field(&app, "assets"), json!([list[0]["id"]]), "{p}");
    }
}

#[test]
fn the_assets_tab_exports_the_checked_assets_into_the_folder() {
    let (mut app, log, _) = app(false);
    app.run("assets.add", json!({})).unwrap();
    let ids: Vec<u64> = assets(&mut app).iter().map(|a| a["id"].as_u64().unwrap()).collect();
    app.run("file.exportForScreens", json!({})).unwrap();
    assert_eq!((field(&app, "tab"), field(&app, "assets")), (json!("artboards"), json!(ids)), "every asset checked");
    assert_eq!((spec(export_for_screens::KIND).ok_label.unwrap())(&app), "Export Artboard");
    set(&mut app, "tab", json!("assets"));
    assert_eq!((spec(export_for_screens::KIND).ok_label.unwrap())(&app), "Export Asset");
    frame(&mut app);
    // Nothing checked keeps the dialog open.
    set(&mut app, "assets", json!([]));
    assert!(confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some(export_for_screens::KIND));
    let dir = std::env::temp_dir().join(format!("vc-efs-assets-{}", std::process::id()));
    let folder = dir.to_string_lossy().replace('\\', "/");
    set(&mut app, "folder", json!(folder));
    set(&mut app, "assets", json!([ids[1], 999]));
    set(&mut app, "formats", json!([{"format": "png", "scale": "2x", "suffix": "@2x"}]));
    let r = confirm(&mut app).unwrap();
    // Create Sub-folders is on by default: a folder per scale.
    assert_eq!(r["files"], json!([format!("{folder}/2x/Asset-2@2x.png")]));
    let png = craft_image_codecs::decode(&std::fs::read(dir.join("2x").join("Asset-2@2x.png")).unwrap())
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
        .unwrap();
    assert_eq!((png.width(), png.height()), (60, 40), "the asset's art at 2x");
    assert_eq!(log.borrow().len(), 1, "Open Location shows the file");
    // The rows are remembered for the dialog and the Asset Export panel alike.
    assert_eq!(app.run("document.exportSettings", json!({})).unwrap()["settings"]["formats"][0]["scale"], "2x");
    let _ = std::fs::remove_dir_all(dir);
}

/// 3D assets export as mesh files from the Asset Export panel: the Assets tab lists and checks the
/// picture assets alone.
#[test]
fn the_assets_tab_leaves_3d_assets_out() {
    let (mut app, _, [a, b]) = app(false);
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "params": {}})).unwrap();
    let three_d = app.run("assets.add3D", json!({"ids": [a]})).unwrap()["assets"][0].as_u64().unwrap();
    let picture = app.run("assets.add", json!({"ids": [b], "name": "Flat"})).unwrap()["assets"][0].as_u64().unwrap();
    app.run("assets.rename", json!({"asset": three_d, "name": "Solid"})).unwrap();
    app.run("file.exportForScreens", json!({})).unwrap();
    assert_eq!(field(&app, "assets"), json!([picture]));
    set(&mut app, "tab", json!("assets"));
    let text = painted_text(&mut app, |app, ui| super::show(app, ui.ctx()));
    assert!(text.contains("Flat") && !text.contains("Solid"), "{text}");
    // Opened from the panel with both checked, the tab still shows the picture asset alone.
    open_export_for_screens_assets(&mut app, Some(&[three_d, picture]));
    assert_eq!(field(&app, "__scrollTo"), json!(picture), "the list scrolls to the first asset it shows");
    let text = painted_text(&mut app, |app, ui| super::show(app, ui.ctx()));
    assert!(text.contains("Flat") && !text.contains("Solid"), "{text}");
}


