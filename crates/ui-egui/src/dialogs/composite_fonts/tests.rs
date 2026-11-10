use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::*;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app
}

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn dialog(app: &mut VectorcraftApp) -> &mut Dialog {
    app.ui.dialog.as_mut().expect("Composite Fonts open")
}

fn names(app: &VectorcraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.composite_fonts.iter().map(|f| f.name.clone()).collect()
}

fn steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// Do `act` with the dialog as it is, as its button does.
fn press(app: &mut VectorcraftApp, act: Action) {
    let mut d = app.ui.dialog.clone().unwrap();
    run(app, &mut d, act).unwrap();
    app.ui.dialog = Some(d);
}

/// Type › Composite Fonts… opens on the document's fonts; New adds one, its parts edited in the
/// table; nothing changes until OK, which sets them as one undo step; Cancel leaves them.
#[test]
fn composite_fonts_change_on_ok_as_one_undo_step() {
    let mut app = app();
    crate::menus::invoke(&mut app, COMMAND, json!({}));
    assert_eq!(dialog(&mut app).kind, KIND);
    frame(&mut app);
    press(&mut app, Action::New);
    frame(&mut app);
    let d = dialog(&mut app);
    assert_eq!(d.str("name"), "Composite Font 1");
    let mut list = fonts(d);
    list[0].parts.roman.font = "JetBrains Mono".into();
    list[0].parts.roman.size = 80.0;
    list[0].parts.kana.scale_at_center = true;
    put_fonts(d, &list);
    d.fields.insert("name".into(), json!("UI Composite A"));
    frame(&mut app);
    assert!(names(&app).is_empty());
    let before = steps(&app);
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(names(&app), ["UI Composite A"]);
    assert_eq!(steps(&app), before + 1);
    let f = &app.session.active().unwrap().doc.composite_fonts[0];
    assert_eq!((f.parts.roman.font.as_str(), f.parts.roman.size, f.parts.kana.scale_at_center), ("JetBrains Mono", 80.0, true));
    // OK with nothing changed adds no step; Cancel leaves the fonts.
    open(&mut app).unwrap();
    super::super::confirm(&mut app).unwrap();
    assert_eq!(steps(&app), before + 1);
    open(&mut app).unwrap();
    press(&mut app, Action::Delete);
    super::super::cancel(&mut app);
    assert_eq!(names(&app), ["UI Composite A"]);
    // Delete Font and OK takes it out.
    open(&mut app).unwrap();
    press(&mut app, Action::Delete);
    assert_eq!(dialog(&mut app).str("name"), "");
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert!(names(&app).is_empty());
}

/// New is based on the font picked and takes the next free name; a name two fonts share, or one
/// left empty, keeps the dialog open and changes nothing.
#[test]
fn shared_or_empty_names_keep_the_dialog_open() {
    let mut app = app();
    open(&mut app).unwrap();
    press(&mut app, Action::New);
    let d = dialog(&mut app);
    let mut list = fonts(d);
    list[0].parts.numerals.font = "Inter".into();
    put_fonts(d, &list);
    press(&mut app, Action::New);
    let list = fonts(dialog(&mut app));
    assert_eq!(list.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Composite Font 1", "Composite Font 2"]);
    assert_eq!(list[1].parts.numerals.font, "Inter");
    for bad in ["composite font 1", "  "] {
        dialog(&mut app).fields.insert("name".into(), json!(bad));
        frame(&mut app);
        assert!(super::super::confirm(&mut app).is_err(), "{bad:?}");
        assert!(app.ui.dialog.is_some());
        assert!(names(&app).is_empty());
    }
    dialog(&mut app).fields.insert("name".into(), json!("UI Composite B"));
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!(names(&app), ["Composite Font 1", "UI Composite B"]);
}

/// The menu item shows only with Show East Asian Options on, and needs a document.
#[test]
fn the_menu_item_shows_with_east_asian_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.show_east_asian_options = true;
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})).map(|s| s.0), Some(false));
    app.run("file.new", json!({})).unwrap();
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})).map(|s| s.0), Some(true));
    app.session.prefs.show_east_asian_options = false;
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})), None);
}

/// Export… writes the font picked to a file; Import… adds the fonts of a file to the dialog's,
/// replacing one of the same name, and opening the file adds them to the document.
#[test]
fn composite_fonts_go_through_files() {
    let dir = std::env::temp_dir().join(format!("vc-composite-fonts-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fonts.vccomposite").to_string_lossy().to_string();
    let (saved, picked) = (file.clone(), file.clone());
    let services = crate::Services {
        pick_save: Some(Box::new(move |_: &FilePick| Some(saved.clone()))),
        pick_open: Some(Box::new(move |_: &FilePick| Some(picked.clone()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({})).unwrap();
    open(&mut app).unwrap();
    press(&mut app, Action::New);
    let d = dialog(&mut app);
    let mut list = fonts(d);
    list[0].name = "UI Composite IO".into();
    list[0].parts.roman.baseline = -12.0;
    put_fonts(d, &list);
    press(&mut app, Action::Export);
    assert!(std::fs::read_to_string(&file).unwrap().contains("UI Composite IO"));
    // Import into the dialog: the same name is replaced, not added.
    let d = dialog(&mut app);
    let mut list = fonts(d);
    list[0].parts.roman.baseline = 0.0;
    put_fonts(d, &list);
    press(&mut app, Action::Import);
    let list = fonts(dialog(&mut app));
    assert_eq!((list.len(), list[0].parts.roman.baseline), (1, -12.0));
    super::super::cancel(&mut app);
    assert!(names(&app).is_empty());
    // Opening the file adds its fonts to the document.
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    assert_eq!(names(&app), ["UI Composite IO"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The dialog lists the fonts and draws its table without a document font picked too.
#[test]
fn the_dialog_draws_with_and_without_fonts() {
    let mut app = app();
    open(&mut app).unwrap();
    frame(&mut app);
    assert_eq!(dialog(&mut app).fields.get("fonts"), Some(&Value::Array(vec![])));
    press(&mut app, Action::New);
    frame(&mut app);
    assert_eq!(fonts(dialog(&mut app)).len(), 1);
}
