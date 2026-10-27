//! The Asset Export panel drawn headless: its empty state, art dropped in (one asset per object,
//! or one with Alt), selecting and renaming assets, the shared export settings rows and Export.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::asset_export::{ID, menu, show};
use crate::widgets::PanelDrag;
use crate::{Services, VectorcraftApp};

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

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

/// One frame of the panel, 236 pt wide as in the dock → its texts and where they are.
fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, alt: bool) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut all = vec![egui::Event::ModifiersChanged(egui::Modifiers { alt, ..Default::default() })];
    all.extend(events);
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 700.0))), events: all, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

fn click(at: Pos2) -> Vec<egui::Event> {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    vec![egui::Event::PointerMoved(at), button(true), button(false)]
}

fn release(at: Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
    ]
}

/// Where text `label` was drawn.
fn at(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts
        .iter()
        .find(|(t, _)| t == label)
        .unwrap_or_else(|| panic!("no `{label}` in {:?}", texts.iter().map(|t| &t.0).collect::<Vec<_>>()))
        .1
        .center()
}

fn assets(app: &mut VectorcraftApp) -> Vec<Value> {
    app.run("assets.list", json!({})).unwrap()["assets"].as_array().unwrap().clone()
}

fn has(texts: &[(String, Rect)], label: &str) -> bool {
    texts.iter().any(|(t, _)| t == label)
}

#[test]
fn an_empty_panel_says_how_to_add_assets_and_shows_the_export_settings() {
    let (mut app, _, _) = app(false);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    for label in ["No assets", "Export Settings", "Scale", "Suffix", "Format", "+ Add Scale", "Export"] {
        assert!(has(&texts, label), "{label}");
    }
    // The menu draws.
    let menu = crate::tests_labels::painted_text(&mut app, menu);
    for item in ["Add Selected Artwork as One Asset", "Remove Selected Assets", "Export for Screens…"] {
        assert!(menu.contains(item), "{item}: {menu}");
    }
    assert!(crate::state::ICON_PANELS.iter().any(|(id, label, _)| *id == ID && *label == "Asset Export"));
}

#[test]
fn art_dropped_on_the_panel_becomes_assets() {
    let (mut app, _, [a, b]) = app(false);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![NodeId(a), NodeId(b)]));
    frame(&ctx, &mut app, release(pos2(100.0, 50.0)), false);
    let list = assets(&mut app);
    assert_eq!(list.len(), 2, "one asset per object");
    // With Alt, one asset of them all.
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![NodeId(a), NodeId(b)]));
    frame(&ctx, &mut app, release(pos2(100.0, 50.0)), true);
    let list = assets(&mut app);
    assert_eq!(list.len(), 3);
    assert_eq!(list[2]["nodes"], json!([a, b]));
    let texts = frame(&ctx, &mut app, vec![], false);
    assert!(has(&texts, "Asset 1") && has(&texts, "Asset 3"));
    // The new asset is the panel's selection.
    assert!(has(&texts, "Export 1 Selected"));
}

#[test]
fn the_plus_button_collects_the_selection_and_the_bin_removes_selected_assets() {
    let (mut app, _, [a, b]) = app(false);
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    // The + is the bottom bar's second icon from the right.
    let bar_y = at(&texts, "Export").y + 34.0;
    frame(&ctx, &mut app, click(pos2(236.0 - 40.0, bar_y)), false);
    assert_eq!(assets(&mut app).len(), 2, "+ adds one asset per object");
    let texts = frame(&ctx, &mut app, vec![], false);
    // Click one: it alone is selected; the bin removes it (the art stays).
    frame(&ctx, &mut app, click(at(&texts, "Asset 2")), false);
    let texts = frame(&ctx, &mut app, vec![], false);
    // The bin is the bottom bar's last icon.
    frame(&ctx, &mut app, click(pos2(236.0 - 13.0, at(&texts, "Export 1 Selected").y + 34.0)), false);
    let names: Vec<Value> = assets(&mut app).iter().map(|x| x["name"].clone()).collect();
    assert_eq!(names, [json!("Asset 1")]);
    assert!(app.session.active().unwrap().doc.node(NodeId(b)).is_some());
}

#[test]
fn renaming_an_asset_runs_assets_rename() {
    let (mut app, _, [a, _]) = app(false);
    let id = app.run("assets.add", json!({"ids": [a]})).unwrap()["assets"][0].as_u64().unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    // Double-click starts editing the name (as typed here); Enter commits it.
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("asset-export-rename"), (id, "Logo".to_string())));
    frame(&ctx, &mut app, vec![], false);
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    frame(&ctx, &mut app, vec![enter], false);
    assert_eq!(assets(&mut app)[0]["name"], "Logo");
}

/// Shift+Enter in an asset's name field renames it and keeps the field open, the new name
/// selected; Enter then renames it again and closes the field.
#[test]
fn shift_enter_renames_an_asset_and_keeps_the_field_open() {
    let (mut app, _, [a, _]) = app(false);
    let id = app.run("assets.add", json!({"ids": [a]})).unwrap()["assets"][0].as_u64().unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let rename_id = egui::Id::unique("asset-export-rename");
    let field = rename_id.with("field");
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    frame(&ctx, &mut app, vec![], false);
    // What a double-click on the tile does: its name field opens with the name.
    ctx.data_mut(|d| d.insert_temp(rename_id, (id, "Asset 1".to_string())));
    frame(&ctx, &mut app, vec![], false);
    frame(&ctx, &mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("Logo".into())], false);
    frame(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)], false);
    assert_eq!(assets(&mut app)[0]["name"], "Logo", "Shift+Enter applies the name");
    assert!(ctx.memory(|m| m.has_focus(field)), "the field keeps the focus");
    assert!(ctx.data(|d| d.get_temp::<(u64, String)>(rename_id)).is_some(), "and stays open");
    frame(&ctx, &mut app, vec![], false);
    let selected = egui::TextEdit::load_state(&ctx, field).and_then(|st| st.cursor.char_range()).map(|r| r.as_sorted_char_range());
    assert_eq!(selected.map(|r| (r.start.0, r.end.0)), Some((0, 4)), "the name shown is selected");
    frame(&ctx, &mut app, vec![egui::Event::Text("Mark".into())], false);
    frame(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], false);
    assert_eq!(assets(&mut app)[0]["name"], "Mark", "typing replaced it, and Enter applies that");
    assert!(ctx.data(|d| d.get_temp::<(u64, String)>(rename_id)).is_none(), "and closes the field");
}

#[test]
fn the_rows_are_the_shared_export_settings() {
    let (mut app, log, [a, b]) = app(true);
    app.run("assets.add", json!({"ids": [a, b]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    // + Add Scale adds a 2x row to the document's export settings (Export for Screens' too).
    frame(&ctx, &mut app, click(at(&texts, "+ Add Scale")), false);
    let settings = app.run("document.exportSettings", json!({})).unwrap()["settings"].clone();
    assert_eq!(settings["formats"].as_array().unwrap().len(), 2);
    assert_eq!(settings["formats"][1]["scale"], "2x");
    app.run("file.exportForScreens", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["formats"][1]["scale"], "2x");
    crate::dialogs::cancel(&mut app);
    // A preset set by an agent shows as its rows.
    app.run("assets.settings.set", json!({"preset": "mobile"})).unwrap();
    let texts = frame(&ctx, &mut app, vec![], false);
    assert!(has(&texts, "Mobile 1x/2x/3x") && has(&texts, "@3x"));
    // Export (nothing selected: every asset) downloads one zip on the web.
    let texts = frame(&ctx, &mut app, vec![], false);
    frame(&ctx, &mut app, click(at(&texts, "Export")), false);
    let log = log.borrow();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, "Untitled-1.zip");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_writes_into_the_picked_folder() {
    let dir = std::env::temp_dir().join(format!("vc-asset-panel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().replace('\\', "/");
    let (mut app, _, [a, _]) = app(false);
    let picked = folder.clone();
    app.services.pick_folder = Some(Box::new(move || Some(picked.clone())));
    app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    frame(&ctx, &mut app, click(at(&texts, "Export")), false);
    assert!(dir.join("Badge.png").is_file(), "{:?}", std::fs::read_dir(&dir).map(|d| d.count()));
    let _ = std::fs::remove_dir_all(&dir);
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
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    frame(&ctx, &mut app, click(at(&texts, "PNG")), false);
    let texts = frame(&ctx, &mut app, vec![], false);
    frame(&ctx, &mut app, click(at(&texts, "TIFF")), false);
    let texts = frame(&ctx, &mut app, vec![], false);
    assert!(has(&texts, "TIFF"));
    let saved = app.run("document.exportSettings", json!({})).unwrap()["settings"].clone();
    assert_eq!(saved["formats"], json!([{"format": "tiff", "scale": "2x", "suffix": "@2x"}]));
    frame(&ctx, &mut app, click(at(&texts, "Badge")), false);
    let texts = frame(&ctx, &mut app, vec![], false);
    frame(&ctx, &mut app, click(at(&texts, "Export 1 Selected")), false);
    {
        let log = log.borrow();
        let [(name, bytes)] = log.as_slice() else { panic!("expected only the selected asset") };
        assert_eq!(name, "Badge@2x.tif");
        assert_eq!(&bytes[..4], b"MM\0*", "the saved TIFF byte order reaches asset export");
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Tiff).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (60, 40), "the selected 30x20 asset at 2x");
        let pixel = img.get_pixel(30, 20).0;
        assert!(pixel[0] > pixel[1] && pixel[0] > pixel[2] && pixel[3] == 255, "the red Badge, not the blue Other asset: {pixel:?}");
    }
    app.run("file.exportForScreens", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields["formats"], saved["formats"]);
    assert_eq!(d.fields["settings"]["tiff"]["byteOrder"], "big");
    assert_eq!(d.fields["settings"]["tiff"]["lzw"], false);
    assert_eq!(d.fields["settings"]["tiff"]["embedIcc"], false);
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
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            frame(&ctx, &mut app, vec![], false);
            let texts = frame(&ctx, &mut app, vec![], false);
            frame(&ctx, &mut app, click(at(&texts, "PNG")), false);
            let texts = frame(&ctx, &mut app, vec![], false);
            frame(&ctx, &mut app, click(at(&texts, "TIFF")), false);
            let texts = frame(&ctx, &mut app, vec![], false);
            assert!(has(&texts, "TIFF"));
            if !rgb_override {
                assert!(app.session.active().unwrap().doc.export_settings.get("settings").and_then(|s| s.get("tiff")).is_none());
            }
            if screens {
                // The 24 pt gear is the rightmost control in the Formats row of this 236 pt panel.
                frame(&ctx, &mut app, click(pos2(236.0 - 12.0, at(&texts, "Formats:").y)), false);
                let d = app.ui.dialog.as_ref().expect("the panel gear opens Format Settings");
                assert_eq!((d.kind.as_str(), d.str("tab"), d.str("__settings")), ("exportForScreens", "assets".into(), "tiff".into()));
                assert_eq!(d.fields["settings"]["tiff"]["colorModel"], if rgb_override { "rgb" } else { "cmyk" });
                crate::dialogs::confirm(&mut app).unwrap();
                assert!(app.ui.dialog.is_none());
            } else {
                frame(&ctx, &mut app, click(at(&texts, "Export")), false);
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
        let mut settings = other_settings.clone();
        if let Some(aa) = format_aa {
            settings["tiff"] = json!({"antiAlias": aa});
        }
        app.run("assets.settings.set", json!({"formats": [row], "antiAlias": "none", "settings": settings})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&ctx, &mut app, vec![], false);
        let texts = frame(&ctx, &mut app, vec![], false);
        frame(&ctx, &mut app, click(at(&texts, "PNG")), false);
        let texts = frame(&ctx, &mut app, vec![], false);
        frame(&ctx, &mut app, click(at(&texts, "TIFF")), false);
        let texts = frame(&ctx, &mut app, vec![], false);
        assert!(has(&texts, "TIFF"));
        frame(&ctx, &mut app, click(at(&texts, "Export")), false);
        {
            let log = log.borrow();
            let [(name, bytes)] = log.as_slice() else { panic!("expected one TIFF: format={format_aa:?}, row={row_aa:?}") };
            assert_eq!(name, "Edge.tif");
            let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Tiff).unwrap().to_rgba8();
            assert!(img.pixels().any(|p| p[3] == 255), "the ellipse is present");
            assert!(img.pixels().any(|p| p[3] == 0), "transparent outside the ellipse");
            let hard = img.pixels().all(|p| p[3] == 0 || p[3] == 255);
            assert_eq!(hard, effective_aa == "none", "effective anti-aliasing: format={format_aa:?}, row={row_aa:?}");
        }
        let saved = app.run("document.exportSettings", json!({})).unwrap()["settings"].clone();
        assert_eq!(saved["antiAlias"], "none", "the shared choice stays saved");
        assert_eq!(saved["settings"], settings, "direct export preserves saved format options, including an absent TIFF map");
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
    let dir = std::env::temp_dir().join(format!("vc-asset-tiff-aliases-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // The first case is the historical control: saved tif without an explicit model must be CMYK.
    // Native picked folders let the mixed-row case inspect each file without introducing a ZIP decoder.
    for (case, (alias, rgb_override, mixed)) in
        [("tif", false, false), ("TIFF", false, false), (".tif", false, true), ("tif", true, false)].into_iter().enumerate()
    {
        let folder = dir.join(case.to_string());
        let picked = folder.to_string_lossy().replace('\\', "/");
        let (mut app, _, [a, _]) = app(false);
        app.services.pick_folder = Some(Box::new(move || Some(picked.clone())));
        app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
        let mut rows = vec![json!({"format": alias, "scale": "2x", "suffix": "@2x"})];
        if mixed {
            rows.push(json!({"format": "png", "scale": "1x", "suffix": ""}));
        }
        let mut settings = json!({"png": {"interlaced": true}});
        if rgb_override {
            settings["tiff"] = json!({"colorModel": "rgb"});
        }
        let saved = json!({"formats": rows, "settings": settings});
        app.run("assets.settings.set", saved.clone()).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&ctx, &mut app, vec![], false);
        let texts = frame(&ctx, &mut app, vec![], false);
        // Keep the valid saved spelling; choosing the TIFF dropdown would hide the alias bug.
        frame(&ctx, &mut app, click(at(&texts, "Export")), false);
        let bytes = std::fs::read(folder.join("Badge@2x.tif")).unwrap();
        assert_eq!(tiff_photometric(&bytes), if rgb_override { 2 } else { 5 }, "saved alias {alias}, explicit RGB={rgb_override}");
        if rgb_override {
            let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Tiff).unwrap();
            assert_eq!((img.width(), img.height()), (60, 40), "the saved TIFF row is still 2x");
        }
        if mixed {
            let png = std::fs::read(folder.join("Badge.png")).unwrap();
            let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
            assert_eq!((img.width(), img.height()), (30, 20), "the PNG row keeps its own 1x scale");
            assert_eq!(png.get(28), Some(&1), "the saved PNG interlace option survives the TIFF defaults");
        }
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), if mixed { 2 } else { 1 });
        assert_eq!(
            app.run("document.exportSettings", json!({})).unwrap()["settings"],
            saved,
            "direct export preserves saved aliases and explicit options"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
