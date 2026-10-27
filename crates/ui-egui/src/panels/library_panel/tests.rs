use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use serde_json::Value;
use vectorcraft_color::Paint;
use vectorcraft_engine::Session;

use super::*;
use crate::panels::swatches::SwatchLibraries;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// One frame of the library panel with `events` and `modifiers` held.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, modifiers: Modifiers, time: f64) {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| show_window(app, ui.ctx()));
    out.textures_delta.clear();
}

/// The panel opened last.
fn shown(app: &VectorcraftApp) -> String {
    last_open(&app.ui).expect("a library panel is open").panel()
}

/// Click the tile or row of `name` in the panel opened last with `modifiers` (frames from
/// `time` to `time + 0.5`; a new window takes a sizing frame first).
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, modifiers: Modifiers, time: f64) {
    frame(app, ctx, vec![], Modifiers::NONE, time);
    frame(app, ctx, vec![], Modifiers::NONE, time + 0.25);
    let at = ctx.read_response(tile_id(&shown(app), name)).unwrap_or_else(|| panic!("no tile for {name}")).rect.center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    frame(app, ctx, vec![Event::PointerMoved(at), button(true), button(false)], modifiers, time + 0.5);
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

#[test]
fn clicking_a_library_swatch_adds_and_applies_it_in_one_step() {
    let mut app = app();
    let ctx = context();
    let r = app.run("window.swatchLibrary", json!({"library": "Pastels"})).unwrap();
    assert_eq!((r["open"].as_str(), r["count"].as_u64(), r["panel"].as_str()), (Some("pastels"), Some(24), Some("library:swatches:pastels")));
    assert_eq!(last_open(&app.ui).map(|p| (p.kind.as_str(), p.id.as_str())), Some(("swatches", "pastels")));
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Pastel Teal", Modifiers::NONE, 0.0);
    let d = &app.session.doc().unwrap().doc;
    let added = d.swatch("Pastel Teal").expect("added to the document");
    assert_eq!(app.session.paint.fill, Paint::solid(added.paint.color().unwrap()), "applied to the fill");
    assert_eq!(undo_len(&app), undo + 1);
    // Alt-click paints the stroke (the inactive proxy).
    click(&mut app, &ctx, "Soft Blue", Modifiers::ALT, 2.0);
    let blue = app.session.doc().unwrap().doc.swatch("Soft Blue").and_then(|w| w.paint.color()).unwrap();
    assert_eq!(app.session.paint.stroke.color(), Some(blue));
    assert!(app.session.fill_active, "Alt keeps the active proxy");
    // A library that doesn't exist is an error; null closes the panel.
    assert!(app.run("window.swatchLibrary", json!({"library": "nope"})).is_err());
    app.run("window.swatchLibrary", json!({"library": null})).unwrap();
    assert!(last_open(&app.ui).is_none());
}

#[test]
fn modifier_clicks_select_and_add_to_swatches_adds_the_selection() {
    let mut app = app();
    let ctx = context();
    app.run("window.swatchLibrary", json!({"library": "earth-tones"})).unwrap();
    click(&mut app, &ctx, "Clay 1", Modifiers::COMMAND, 0.0);
    click(&mut app, &ctx, "Ochre", Modifiers::NONE, 2.0);
    assert_eq!(selection(&ctx, "library:swatches:earth-tones"), ["Ochre"], "a folder click selects its group");
    click(&mut app, &ctx, "Clay 3", Modifiers::COMMAND, 4.0);
    assert!(app.session.doc().unwrap().doc.swatch("Clay 3").is_none(), "modifier clicks only select");
    let sel = selection(&ctx, "library:swatches:earth-tones");
    SwatchLibraries::add(&mut app, "earth-tones", sel);
    let d = &app.session.doc().unwrap().doc;
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Ochre").map(|g| g.swatches.len()), Some(6));
    assert!(d.swatches.iter().any(|w| w.name == "Clay 3"));
    // The selection belongs to its library's panel.
    app.run("window.swatchLibrary", json!({"library": "brights"})).unwrap();
    assert!(selection(&ctx, "library:swatches:brights").is_empty());
}

#[test]
fn a_plain_click_sets_where_a_shift_click_range_starts() {
    let mut app = app();
    let ctx = context();
    app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap();
    let (_, lib) = SwatchLibraries::get(&app, "pastels").unwrap();
    let order: Vec<String> = SwatchLibraries::rows(&lib, "").iter().map(|r| r.name.to_string()).collect();
    click(&mut app, &ctx, &order[1], Modifiers::NONE, 0.0);
    click(&mut app, &ctx, &order[4], Modifiers::SHIFT, 2.0);
    assert_eq!(selection(&ctx, "library:swatches:pastels"), order[1..=4], "the range starts at the plain click");
    // A plain click after a Cmd/Ctrl-click moves the anchor from the Cmd/Ctrl-clicked item.
    click(&mut app, &ctx, &order[8], Modifiers::COMMAND, 4.0);
    click(&mut app, &ctx, &order[2], Modifiers::NONE, 6.0);
    click(&mut app, &ctx, &order[3], Modifiers::SHIFT, 8.0);
    assert_eq!(selection(&ctx, "library:swatches:pastels"), order[2..=3]);
    let d = &app.session.doc().unwrap().doc;
    assert!(d.swatch(&order[2]).is_some(), "a plain click still adds the swatch");
    assert!(d.swatch(&order[3]).is_none(), "a Shift-click only selects");
}

#[test]
fn find_filters_list_view_and_arrows_step_through_the_libraries() {
    let mut app = app();
    let ctx = context();
    app.run("window.swatchLibrary", json!({"library": "web-safe-216"})).unwrap();
    let panel = "library:swatches:web-safe-216";
    app.run("ui.libraryPanel", json!({"panel": panel, "view": "smallList"})).unwrap();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(key(panel, "find")), "#FF00".to_string()));
    frame(&mut app, &ctx, vec![], Modifiers::NONE, 0.0);
    frame(&mut app, &ctx, vec![], Modifiers::NONE, 0.5);
    assert!(ctx.read_response(tile_id(panel, "#FF0033")).is_some());
    assert!(ctx.read_response(tile_id(panel, "#00FF00")).is_none());
    let first = SWATCH_IDS[0];
    let last = SwatchLibraries::list(&app).pop().unwrap().id;
    assert_eq!(last, "radial-glows", "gradient libraries come after the swatch libraries");
    assert_eq!(neighbour::<SwatchLibraries>(&app, first, -1), Some(last), "wraps around");
    assert_eq!(neighbour::<SwatchLibraries>(&app, "web-safe-216", 1).as_deref(), Some(SWATCH_IDS[1]));
}

const SWATCH_IDS: [&str; 9] = [
    "web-safe-216",
    "grays-neutrals",
    "earth-tones",
    "skin-tone-ramps",
    "pastels",
    "brights",
    "metallic-gradients",
    "perceptual-scales",
    "harmony-sets",
];

#[test]
fn the_window_submenu_lists_every_library() {
    let app = app();
    let ids: Vec<Value> = crate::menus::menu_entries(&app)
        .into_iter()
        .filter(|e| e.path == ["Window", "Swatch Libraries"] && e.command.as_deref() == Some("window.swatchLibrary"))
        .map(|e| e.params["library"].clone())
        .collect();
    let all = vectorcraft_engine::cmd::swatchlib::libraries(&app.session);
    let listed = |category: &str| all.iter().filter(|l| l.category == category).map(|l| json!(l.id)).collect::<Vec<_>>();
    assert_eq!(ids, listed("builtIn"));
    assert_eq!(ids, SWATCH_IDS.map(|i| json!(i)));
    let gradients: Vec<Value> = crate::menus::menu_entries(&app)
        .into_iter()
        .filter(|e| e.path == ["Window", "Swatch Libraries", "Gradients"])
        .map(|e| e.params["library"].clone())
        .collect();
    assert!(gradients.len() >= 5);
    assert_eq!(gradients, listed("gradients"));
    let defaults = crate::menus::menu_entries(&app).into_iter().any(|e| e.command.as_deref() == Some("swatch.resetDefaults"));
    assert!(defaults, "Default Swatches is listed");
}

#[test]
fn save_dialog_writes_user_libraries_that_the_window_menu_lists() {
    let dir = std::env::temp_dir().join(format!("vc-library-panel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = app();
    app.session.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let user_items = |app: &VectorcraftApp| -> Vec<(String, String)> {
        crate::menus::menu_entries(app)
            .into_iter()
            .filter(|e| e.path == ["Window", "Swatch Libraries", "User Defined"])
            .map(|e| (e.label, e.command.unwrap_or_default()))
            .collect()
    };
    assert!(user_items(&app).is_empty(), "empty slots are hidden");
    app.run("ui.saveSwatchLibrary", json!({"names": ["Brights"]})).unwrap();
    let d = app.ui.dialog.as_mut().expect("Save Swatch Library opens");
    assert_eq!((d.kind.as_str(), d.bool("user")), (crate::dialogs::save_swatch_library::KIND, true));
    d.fields.insert("name".into(), json!("Loud"));
    d.fields.insert("selectedOnly".into(), json!(true));
    let ctx = context();
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))), ..Default::default() };
    ctx.run_ui(input, |ui| crate::dialogs::show(&mut app, ui.ctx())).textures_delta.clear();
    assert!(app.ui.dialog.is_some(), "the dialog draws and stays open");
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(user_items(&app), [("Loud".to_string(), "window.userSwatchLibrary1".to_string())]);
    app.run("window.userSwatchLibrary1", json!({})).unwrap();
    assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some("user/Loud.vcswatches"));
    let (_, lib) = SwatchLibraries::get(&app, "user/Loud.vcswatches").unwrap();
    assert_eq!((lib.groups.len(), lib.len()), (1, 5), "only the selected group");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saving_to_a_file_and_opening_a_palette_file() {
    let written = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let w = written.clone();
    let services = crate::Services {
        pick_save: Some(Box::new(|p: &crate::FilePick| Some(format!("/tmp/{}", p.name)))),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("ui.saveSwatchLibrary", json!({})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    assert!(!d.bool("user") && !d.bool("__user"), "no user folder: a file");
    d.fields.insert("format".into(), json!("gpl"));
    crate::dialogs::confirm(&mut app).unwrap();
    let (path, bytes) = written.borrow()[0].clone();
    assert!(path.ends_with(".gpl"), "{path}");
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.starts_with(
            "GIMP Palette
"
        ) && text.contains(
            "	Red
"
        ),
        "{text}"
    );
    // Opening a palette file opens it in the library panel.
    crate::io::open_bytes(&mut app, "Saved.gpl", text.as_bytes(), None).unwrap();
    let open = last_open(&app.ui).cloned().unwrap();
    assert!(open.id.starts_with("loaded/"), "{open:?}");
    assert_eq!(app.session.documents().len(), 1, "not opened as a document");
    let other = SwatchLibraries::list(&app).into_iter().find(|l| l.id == open.id).unwrap();
    assert_eq!(other.submenu, Some("Other Libraries"));
    // Built-in libraries' names are interface labels; a file's is shown as it is.
    for l in SwatchLibraries::list(&app) {
        assert_eq!(builtin_library(&l.id), !matches!(l.submenu, Some("User Defined" | "Other Libraries")), "{}", l.id);
    }
    assert!(!builtin_library(&open.id) && builtin_library(vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES));
}

#[test]
fn the_save_dialog_writes_swatch_exchange_files() {
    let written = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let w = written.clone();
    let services = crate::Services {
        pick_save: Some(Box::new(|p: &crate::FilePick| Some(format!("/tmp/{}", p.name)))),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("ui.saveSwatchLibrary", json!({})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("format".into(), json!("ase"));
    d.fields.insert("name".into(), json!("Brand Kit"));
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(text.contains("Swatch Exchange (.ase)") && text.contains("Swatch exchange files keep solid colors only."), "{text}");
    crate::dialogs::confirm(&mut app).unwrap();
    let (path, bytes) = written.borrow()[0].clone();
    assert_eq!(path, "/tmp/Brand Kit.ase", "the save panel suggests the name");
    let lib = vectorcraft_color::palette_io::read_bytes(&bytes, "x").unwrap();
    let solid = app.session.doc().unwrap().doc.swatches_iter().filter(|w| w.paint.color().is_some()).count();
    assert_eq!(lib.len(), solid, "every solid color of the document");
    // The web downloads the file under that name.
    let downloads = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let dl = downloads.clone();
    let services = crate::Services {
        download: Some(Box::new(move |name: &str, _: &[u8]| {
            dl.borrow_mut().push(name.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut web = VectorcraftApp::new(Session::new(), services);
    web.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    web.run("ui.saveSwatchLibrary", json!({})).unwrap();
    let d = web.ui.dialog.as_mut().unwrap();
    d.fields.insert("format".into(), json!("ase"));
    d.fields.insert("name".into(), json!(" Brand: Kit. "));
    crate::dialogs::confirm(&mut web).unwrap();
    assert_eq!(*downloads.borrow(), ["Brand- Kit.ase"], "the name made safe as a file name, as for User Defined");
}

#[test]
fn swatch_exchange_files_without_a_path_open_in_the_library_panel() {
    // A file opened on the web has no path; open_bytes passes its bytes as dataBase64.
    let mut app = app();
    crate::io::open_bytes(&mut app, "Brand.ase", &vectorcraft_testkit::ase::sample(), None).unwrap();
    let open = last_open(&app.ui).cloned().unwrap();
    assert_eq!(open.id, "loaded/Brand.ase");
    let (_, lib) = SwatchLibraries::get(&app, &open.id).unwrap();
    assert_eq!((lib.len(), lib.groups.len()), (4, 1));
    assert_eq!(app.session.documents().len(), 1, "not opened as a document");
}

fn docking(app: &VectorcraftApp) -> &craft_ui::docking::Layout<String> {
    app.ui.docking.as_ref().expect("library panels live in the shared layout")
}

fn panel_of(reply: Value) -> String {
    reply["panel"].as_str().expect("the reply names the panel").to_string()
}

fn groups(app: &VectorcraftApp) -> Vec<Vec<String>> {
    docking(app).floating.iter().map(crate::panel_docking::window_panels).collect()
}

#[test]
fn each_library_opens_in_a_panel_of_its_own() {
    let mut app = app();
    // Without the shared layout yet, `window.panel` floats a library panel too.
    app.run("window.panel", json!({"panel": "library:swatches:earth-tones"})).unwrap();
    assert!(is_open(&app.ui, "library:swatches:earth-tones"));
    app.run("window.panel.close", json!({"panel": "library:swatches:earth-tones"})).unwrap();
    let pastels = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
    let brights = panel_of(app.run("window.swatchLibrary", json!({"library": "Brights"})).unwrap());
    let styles = panel_of(app.run("window.graphicStyleLibrary", json!({"library": "hand-drawn"})).unwrap());
    assert_eq!([&pastels, &brights, &styles], ["library:swatches:pastels", "library:swatches:brights", "library:graphicStyles:hand-drawn"]);
    assert_eq!(groups(&app), [vec![pastels.clone()], vec![brights.clone()], vec![styles.clone()]], "each floats on its own");
    assert_ne!(docking(&app).floating[0].rect, docking(&app).floating[1].rect, "one beside another");
    assert_eq!(tab_label(&app, &brights).as_deref(), Some("Brights"));
    // Opening an open library brings its panel forward.
    app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap();
    assert_eq!(groups(&app).len(), 3);
    assert_eq!(last_open(&app.ui).map(LibraryPanel::panel), Some(pastels.clone()));
    // They group, dock and close as any panel, and `window.panel` shows one again.
    app.run("window.panel.move", json!({"panel": brights, "anchor": pastels})).unwrap();
    assert!(groups(&app).contains(&vec![pastels.clone(), brights.clone()]), "{:?}", groups(&app));
    app.run("window.panel.dock", json!({"panel": styles})).unwrap();
    assert!(craft_ui::docking::Layout { root: docking(&app).root.clone(), floating: vec![] }.contains(&styles));
    app.run("window.panel.close", json!({"panel": brights})).unwrap();
    assert!(!is_open(&app.ui, &brights) && is_open(&app.ui, &pastels));
    app.run("window.panel", json!({"panel": brights})).unwrap();
    assert!(is_open(&app.ui, &brights));
    assert!(crate::panel_docking::valid(docking(&app)));
    // `library: null` closes the panels of its kind.
    app.run("window.swatchLibrary", json!({"library": null})).unwrap();
    assert!(!any_open::<SwatchLibraries>(&app.ui) && is_open(&app.ui, &styles));
    // What isn't a library panel's id.
    for bad in ["library:nope:x", "library:swatches:", "library:swatches", "Library:swatches:pastels"] {
        assert!(crate::menus::normalize_panel(bad).is_none(), "{bad}");
    }
    assert!(app.run("ui.libraryPanel", json!({"panel": "layers"})).is_err());
    assert!(app.run("ui.libraryPanel", json!({"panel": pastels, "persistent": true})).is_err(), "closed and not persistent");
    assert!(app.run("ui.libraryPanel", json!({"panel": styles, "view": "huge"})).is_err());
}

/// A context whose frames report their accessibility tree.
fn accessible() -> egui::Context {
    let ctx = context();
    ctx.enable_accesskit();
    ctx
}

/// One frame of `draw` with `events`, after the frames `ctx` ran.
fn draw_frame(app: &mut VectorcraftApp, ctx: &egui::Context, draw: &dyn Fn(&mut VectorcraftApp, &mut Ui), events: Vec<Event>) -> egui::FullOutput {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
    let time = Some(ctx.input(|i| i.time) + 0.1);
    let mut out = ctx.run_ui(egui::RawInput { events, time, screen_rect, ..Default::default() }, |ui| draw(app, ui));
    out.textures_delta.clear();
    out
}

/// Whether the control `draw` shows that screen readers name `label` is enabled, and its center.
fn control(app: &mut VectorcraftApp, ctx: &egui::Context, draw: &dyn Fn(&mut VectorcraftApp, &mut Ui), label: &str) -> (bool, Pos2) {
    draw_frame(app, ctx, draw, vec![]);
    let out = draw_frame(app, ctx, draw, vec![]);
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    let (_, node) = nodes.iter().find(|(_, n)| n.label().map(str::trim) == Some(label)).unwrap_or_else(|| panic!("no control named {label:?}"));
    let b = node.bounds().expect("bounds");
    (!node.is_disabled(), Pos2::new((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5))
}

/// A click at `at` in a frame of `draw`.
fn click_at(app: &mut VectorcraftApp, ctx: &egui::Context, draw: &dyn Fn(&mut VectorcraftApp, &mut Ui), at: Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    draw_frame(app, ctx, draw, vec![Event::PointerMoved(at)]);
    draw_frame(app, ctx, draw, vec![button(true), button(false)]);
}

/// The app restarted with the preferences `app` saves.
fn restarted(app: &VectorcraftApp) -> VectorcraftApp {
    let saved: crate::UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
    let mut next = VectorcraftApp::new(Session::new(), Default::default());
    next.ui = saved.sanitized();
    restore(&mut next);
    next
}

#[test]
fn persistent_panels_reopen_at_startup_with_their_settings() {
    let dir = std::env::temp_dir().join(format!("vc-persistent-libraries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let gpl = dir.join("Brand.gpl");
    std::fs::write(&gpl, "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n").unwrap();
    let mut app = app();
    let pastels = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
    let brights = panel_of(app.run("window.swatchLibrary", json!({"library": "brights"})).unwrap());
    let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
    let r = app.run("ui.libraryPanel", json!({"panel": pastels, "persistent": true, "view": "largeList", "showFind": false})).unwrap();
    assert_eq!((r["persistent"].as_bool(), r["view"].as_str(), r["showFind"].as_bool()), (Some(true), Some("largeList"), Some(false)));
    app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true})).unwrap();
    app.run("window.panel.float", json!({"panel": pastels, "x": 40, "y": 90})).unwrap();
    // A persistent panel reopens even when it was closed.
    app.run("window.panel.close", json!({"panel": brand})).unwrap();
    let next = restarted(&app);
    assert!(is_open(&next.ui, &pastels) && is_open(&next.ui, &brand), "{}", next.ui.status);
    assert!(!is_open(&next.ui, &brights), "a panel that isn't persistent closed with the app");
    let group = docking(&next).floating.iter().find(|g| g.contains(&pastels)).unwrap();
    assert_eq!(group.rect[..2], [40.0, 90.0], "where it was");
    let s = settings(&next.ui, "swatches", "pastels").unwrap();
    assert_eq!((s.view, s.hide_find, s.persistent), (Some(View::LargeList), true, true));
    let (_, id) = parse(&brand).unwrap();
    assert!(SwatchLibraries::get(&next, id).is_some_and(|(_, lib)| lib.swatch("Red").is_some()), "read again from its file");
    assert!(next.ui.status.is_empty(), "{}", next.ui.status);
    assert!(next.ui.docking_hidden.keys().all(|p| p != &brights));
    // Unchecked, a panel no longer reopens; one whose file is gone reopens as missing, with a note.
    let mut next = next;
    next.run("ui.libraryPanel", json!({"panel": pastels, "persistent": false})).unwrap();
    std::fs::remove_file(&gpl).unwrap();
    let last = restarted(&next);
    assert!(!is_open(&last.ui, &pastels) && is_open(&last.ui, &brand));
    assert!(last.ui.status.contains("Brand.gpl"), "{}", last.ui.status);
    assert_eq!(last.ui.library_panels.iter().map(LibraryPanel::panel).collect::<Vec<_>>(), [brand]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_persistent_panel_whose_file_is_gone_is_listed_as_missing_until_located() {
    let dir = std::env::temp_dir().join(format!("vc-missing-library-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("moved")).unwrap();
    let palette = "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n";
    let gpl = dir.join("Brand.gpl");
    std::fs::write(&gpl, palette).unwrap();
    let mut app = app();
    let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
    app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true, "view": "smallList"})).unwrap();
    std::fs::remove_file(&gpl).unwrap();
    // The panel reopens showing its library as missing, and the preferences keep it.
    let mut next = restarted(&app);
    tidy(&mut next);
    assert!(is_open(&next.ui, &brand), "{}", next.ui.status);
    assert!(next.ui.status.contains("Brand.gpl"), "{}", next.ui.status);
    let (_, gone) = parse(&brand).unwrap();
    let s = settings(&next.ui, "swatches", gone).cloned().unwrap();
    assert_eq!((s.path, s.persistent, s.view), (Some(gpl.to_string_lossy().into_owned()), true, Some(View::SmallList)));
    assert_eq!(tab_label(&next, &brand).as_deref(), Some("Brand"));
    // Closed, it opens again by its library's id.
    next.run("window.panel.close", json!({"panel": brand})).unwrap();
    let r = next.run("window.swatchLibrary", json!({"library": gone})).unwrap();
    assert_eq!((r["panel"].as_str(), r["name"].as_str(), r["count"].as_u64()), (Some(brand.as_str()), Some("Brand"), Some(0)));
    assert!(is_open(&next.ui, &brand));
    let text = crate::tests_labels::painted_text(&mut next, |app, ui| show(app, ui, &brand));
    let shown = format!("Missing: {}", gpl.to_string_lossy());
    assert!(text.contains(&shown), "{text}");
    let ctx = accessible();
    let body = |app: &mut VectorcraftApp, ui: &mut Ui| show(app, ui, &brand);
    assert!(!control(&mut next, &ctx, &body, "Locate Library…").0, "dimmed without an open dialog");
    let panel_menu = |app: &mut VectorcraftApp, ui: &mut Ui| menu(app, ui, &brand);
    assert!(!control(&mut next, &ctx, &panel_menu, "Next Library").0, "no library to step from");
    let mut next = restarted(&next);
    assert!(is_open(&next.ui, &brand), "still listed after another restart");
    // Locate Library… reads the file where it is now and shows it in the same panel.
    assert!(next.run("ui.libraryPanel.locate", json!({"panel": "layers"})).is_err());
    let moved = dir.join("moved").join("Brand.gpl");
    std::fs::write(&moved, palette).unwrap();
    let picked = moved.to_string_lossy().into_owned();
    next.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(picked.clone())));
    let (enabled, at) = control(&mut next, &ctx, &body, "Locate Library…");
    assert!(enabled);
    click_at(&mut next, &ctx, &body, at);
    let located = panel_id("swatches", &format!("loaded/{}", moved.to_string_lossy()));
    assert!(!is_open(&next.ui, &brand) && is_open(&next.ui, &located), "{}", next.ui.status);
    let (_, id) = parse(&located).unwrap();
    assert!(SwatchLibraries::get(&next, id).is_some_and(|(_, lib)| lib.swatch("Red").is_some()));
    let s = settings(&next.ui, "swatches", id).cloned().unwrap();
    assert_eq!((s.path, s.persistent, s.view), (Some(moved.to_string_lossy().into_owned()), true, Some(View::SmallList)));
    assert!(settings(&next.ui, "swatches", gone).is_none());
    let located_menu = |app: &mut VectorcraftApp, ui: &mut Ui| menu(app, ui, &located);
    assert!(control(&mut next, &ctx, &located_menu, "Next Library").0);
    let last = restarted(&next);
    assert!(is_open(&last.ui, &located) && last.ui.status.is_empty(), "{}", last.ui.status);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn locating_a_library_open_in_its_own_panel_brings_that_panel_forward() {
    let dir = std::env::temp_dir().join(format!("vc-locate-open-library-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let palette = "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n";
    let gpl = dir.join("Brand.gpl");
    std::fs::write(&gpl, palette).unwrap();
    let mut app = app();
    let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
    app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true})).unwrap();
    std::fs::remove_file(&gpl).unwrap();
    let mut next = restarted(&app);
    let copy = dir.join("Copy.gpl");
    std::fs::write(&copy, palette).unwrap();
    let other = panel_of(next.run("window.swatchLibrary.other", json!({"path": copy.to_string_lossy()})).unwrap());
    next.run("window.panel.close", json!({"panel": brand})).unwrap();
    assert!(next.run("ui.libraryPanel.locate", json!({"panel": brand, "path": copy.to_string_lossy()})).is_err(), "a closed panel");
    next.run("window.panel", json!({"panel": brand})).unwrap();
    let r = next.run("ui.libraryPanel.locate", json!({"panel": brand, "path": copy.to_string_lossy()})).unwrap();
    assert_eq!((r["panel"].as_str(), r["persistent"].as_bool()), (Some(other.as_str()), Some(true)));
    assert!(!is_open(&next.ui, &brand) && is_open(&next.ui, &other));
    assert!(next.ui.library_panels.iter().all(|p| p.panel() != brand), "the missing panel is forgotten");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn locate_library_leaves_a_panel_whose_library_is_there_alone() {
    let dir = std::env::temp_dir().join(format!("vc-locate-found-library-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let gpl = dir.join("Brand.gpl");
    std::fs::write(&gpl, "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n").unwrap();
    let mut app = app();
    let pastels = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
    for persistent in [false, true] {
        app.run("ui.libraryPanel", json!({"panel": pastels, "persistent": persistent})).unwrap();
        let e = app.run("ui.libraryPanel.locate", json!({"panel": pastels, "path": gpl.to_string_lossy()})).unwrap_err();
        assert!(e.contains("isn't missing its library"), "{e}");
        assert!(is_open(&app.ui, &pastels), "persistent: {persistent}");
        assert_eq!(settings(&app.ui, "swatches", "pastels").map(|s| s.persistent), Some(persistent));
    }
    assert!(SwatchLibraries::get(&app, &format!("loaded/{}", gpl.to_string_lossy())).is_none(), "the file isn't loaded");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn locate_library_reads_a_missing_library_whose_panel_is_no_longer_persistent() {
    let dir = std::env::temp_dir().join(format!("vc-locate-not-persistent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("moved")).unwrap();
    let palette = "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n";
    let gpl = dir.join("Brand.gpl");
    std::fs::write(&gpl, palette).unwrap();
    let mut app = app();
    let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
    app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true})).unwrap();
    std::fs::remove_file(&gpl).unwrap();
    let mut next = restarted(&app);
    // After a command turns Persistent off, the panel stays open on its missing library until the next frame's tidy.
    next.run("ui.libraryPanel", json!({"panel": brand, "persistent": false})).unwrap();
    assert!(is_open(&next.ui, &brand));
    let moved = dir.join("moved").join("Brand.gpl");
    std::fs::write(&moved, palette).unwrap();
    let r = next.run("ui.libraryPanel.locate", json!({"panel": brand, "path": moved.to_string_lossy()})).unwrap();
    let located = panel_id("swatches", &format!("loaded/{}", moved.to_string_lossy()));
    assert_eq!((r["panel"].as_str(), r["persistent"].as_bool()), (Some(located.as_str()), Some(false)));
    assert!(!is_open(&next.ui, &brand) && is_open(&next.ui, &located), "{}", next.ui.status);
    let (_, id) = parse(&located).unwrap();
    assert!(SwatchLibraries::get(&next, id).is_some_and(|(_, lib)| lib.swatch("Red").is_some()));
    tidy(&mut next);
    assert!(is_open(&next.ui, &located), "its library is there now");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_open_panel_whose_library_is_missing_is_brought_forward_by_its_id_when_not_persistent() {
    let dir = std::env::temp_dir().join(format!("vc-open-not-persistent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let gpl = dir.join("Brand.gpl");
    std::fs::write(&gpl, "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n").unwrap();
    let mut app = app();
    let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
    app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true})).unwrap();
    std::fs::remove_file(&gpl).unwrap();
    let mut next = restarted(&app);
    // After a command turns Persistent off, the panel stays open on its missing library until the next frame's tidy.
    next.run("ui.libraryPanel", json!({"panel": brand, "persistent": false})).unwrap();
    let (_, gone) = parse(&brand).unwrap();
    let r = next.run("window.swatchLibrary", json!({"library": gone})).unwrap();
    assert_eq!((r["panel"].as_str(), r["name"].as_str(), r["count"].as_u64()), (Some(brand.as_str()), Some("Brand"), Some(0)));
    assert!(is_open(&next.ui, &brand));
    tidy(&mut next);
    assert!(!is_open(&next.ui, &brand), "a panel that isn't persistent still closes");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn previous_and_next_show_the_neighbouring_library_in_the_same_panel() {
    let mut app = app();
    let first = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
    app.run("window.panel.float", json!({"panel": first, "x": 50, "y": 60})).unwrap();
    app.run("ui.libraryPanel", json!({"panel": first, "persistent": true, "view": "smallList"})).unwrap();
    step::<SwatchLibraries>(&mut app, "pastels", 1);
    let next = panel_id("swatches", SWATCH_IDS[5]);
    assert!(!is_open(&app.ui, &first) && is_open(&app.ui, &next), "{}", app.ui.status);
    let group = docking(&app).floating.iter().find(|g| g.contains(&next)).unwrap();
    assert_eq!((group.panels().count(), group.rect[0], group.rect[1]), (1, 50.0, 60.0), "in its place");
    let s = settings(&app.ui, "swatches", SWATCH_IDS[5]).unwrap();
    assert_eq!((s.view, s.persistent), (Some(View::SmallList), true), "the panel keeps its settings");
    assert!(settings(&app.ui, "swatches", "pastels").is_none());
    assert!(crate::panel_docking::valid(docking(&app)));
    // A library open in a panel of its own is brought forward instead.
    app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap();
    let r = app.run("ui.libraryPanel", json!({"panel": next, "library": "Pastels"})).unwrap();
    assert_eq!(r["panel"], json!(first));
    assert!(is_open(&app.ui, &next) && is_open(&app.ui, &first));
}

#[test]
fn a_panel_closes_when_its_library_is_gone() {
    let dir = std::env::temp_dir().join(format!("vc-gone-library-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Gone.gpl"), "GIMP Palette\nName: Gone\n#\n0 0 0\tInk\n").unwrap();
    std::fs::write(dir.join("Kept.gpl"), "GIMP Palette\nName: Kept\n#\n0 0 0\tInk\n").unwrap();
    let mut app = app();
    app.session.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let panel = panel_of(app.run("window.swatchLibrary", json!({"library": "user/Gone.gpl"})).unwrap());
    let kept = panel_of(app.run("window.swatchLibrary", json!({"library": "user/Kept.gpl"})).unwrap());
    app.run("ui.libraryPanel", json!({"panel": kept, "persistent": true})).unwrap();
    tidy(&mut app);
    assert!(is_open(&app.ui, &panel));
    std::fs::remove_file(dir.join("Gone.gpl")).unwrap();
    std::fs::remove_file(dir.join("Kept.gpl")).unwrap();
    app.session.swatch_libraries.rescan();
    tidy(&mut app);
    assert!(!is_open(&app.ui, &panel));
    // A persistent panel stays open and shows its library as missing.
    assert!(is_open(&app.ui, &kept));
    assert_eq!(tab_label(&app, &kept).as_deref(), Some("Kept"));
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui, &kept));
    assert!(text.contains("Missing: user/Kept.gpl"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_panel_menu_has_persistent() {
    let mut app = app();
    let panel = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| menu(app, ui, &panel));
    for item in ["Add to Swatches", "Small List View", "Show Find Field", "Previous Library", "Persistent", "Close Library"] {
        assert!(text.contains(item), "{item}: {text}");
    }
}
