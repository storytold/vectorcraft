//! Safe Mode in the app: the choice after a launch that stopped while a plug-in was loading
//! (Disable and Continue, Start Normally, Start in Safe Mode), Help › Safe Mode with Enable on
//! Re-launch, and Safe Mode in the window title. Safe Mode and installed plug-ins are
//! process-wide, so these tests run in their own test binary, one at a time.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::safe_mode::{self, Item};
use vectorcraft_plugins::{registry, wat as tpl};
use vectorcraft_ui_egui::dialogs::safe_mode as ui;
use vectorcraft_ui_egui::panels::appearance;
use vectorcraft_ui_egui::{Services, VectorcraftApp, chrome, dialogs, theme};

static LOCK: Mutex<()> = Mutex::new(());

/// Safe Mode off, no launch marker, nothing pending, disabled or installed.
fn reset() {
    ui::end_launch();
    safe_mode::set_active(None);
    safe_mode::forget_previous();
    safe_mode::set_disabled(vec![]);
    for p in registry::list() {
        registry::remove(p.id());
    }
}

/// Holds the lock for one test and resets the process-wide state when the test starts and when it
/// ends, so a test that fails leaves the next one as it would find a new process.
struct Fresh {
    _guard: std::sync::MutexGuard<'static, ()>,
}

fn fresh() -> Fresh {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    Fresh { _guard: guard }
}

impl Drop for Fresh {
    fn drop(&mut self) {
        reset();
    }
}

fn filter(id: &str) -> Vec<u8> {
    wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-ui-safe-mode-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A plug-in folder holding x and y, as `plugin.install` keeps them.
fn store_with_two(tag: &str) -> PathBuf {
    let dir = tmp_dir(tag);
    let mut s = Session::new();
    s.attach_plugin_store(dir.clone());
    for id in ["test.ui.x", "test.ui.y"] {
        s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&filter(id))})).unwrap();
    }
    dir
}

/// A launch of the app after one that stopped while `stopped` was loading (none: Safe Mode from
/// `--safe-mode` when `flag`), with the plug-in folder `dir` and the launch marker folder `launch`.
fn launch(dir: &Path, launch: &Path, flag: bool) -> VectorcraftApp {
    for p in registry::list() {
        registry::remove(p.id());
    }
    let stopped = ui::begin_launch(Some(launch), flag);
    let mut session = Session::new();
    session.attach_plugin_store(dir.to_path_buf());
    let mut app = VectorcraftApp::new(session, Services::default());
    ui::after_start(&mut app, stopped);
    app
}

/// A marker left by a launch that stopped while the stored plug-in x was loading.
fn stopped_while_loading_x(launch: &Path, dir: &Path) {
    let x = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).find(|p| p.to_string_lossy().contains("test_ui_x")).unwrap();
    let item = json!({"format": "vectorcraft-launch", "version": 1, "item": {"file": x.display().to_string()}});
    std::fs::create_dir_all(launch).unwrap();
    std::fs::write(launch.join("launch-1-dead.json"), item.to_string()).unwrap();
}

/// One headless frame of the dialog layer.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn done(launch: &Path) {
    ui::end_launch();
    safe_mode::set_active(None);
    safe_mode::forget_previous();
    safe_mode::set_disabled(vec![]);
    let _ = std::fs::remove_dir_all(launch);
}

fn kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

#[test]
fn disable_and_continue_loads_every_plug_in_but_the_one_that_was_loading() {
    let _f = fresh();
    let dir = store_with_two("disable");
    let launch_dir = dir.join("Launch");
    stopped_while_loading_x(&launch_dir, &dir);
    let mut app = launch(&dir, &launch_dir, false);
    assert_eq!(kind(&app), Some(ui::STOPPED));
    assert!(safe_mode::active() && registry::list().is_empty(), "nothing loads until the user chooses");
    assert!(chrome::window_title(&app).ends_with("(Safe Mode)"));
    frame(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(!safe_mode::active() && kind(&app).is_none());
    assert!(registry::get("test.ui.x").is_none() && registry::get("test.ui.y").is_some(), "only the one that was loading is off");
    assert_eq!(chrome::window_title(&app), "VectorCraft");
    assert!(dir.join("disabled.json").is_file(), "kept off at later launches");
    ui::end_launch();

    // The next launch, which quit normally before: nothing is asked, and x stays off.
    let app = launch(&dir, &launch_dir, false);
    assert!(kind(&app).is_none() && !safe_mode::active());
    assert!(registry::get("test.ui.x").is_none() && registry::get("test.ui.y").is_some());
    done(&launch_dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn start_normally_loads_everything_and_start_in_safe_mode_loads_nothing() {
    let _f = fresh();
    let dir = store_with_two("normal");
    let launch_dir = dir.join("Launch");
    stopped_while_loading_x(&launch_dir, &dir);
    let mut app = launch(&dir, &launch_dir, false);
    app.ui.dialog.as_mut().unwrap().fields.insert("normal".into(), json!(true));
    dialogs::confirm(&mut app).unwrap();
    assert!(!ui::choosing(&app) && !ui::staying_on(&app), "Start Normally ends Safe Mode");
    assert!(registry::get("test.ui.x").is_some() && registry::get("test.ui.y").is_some());
    assert!(safe_mode::disabled().is_empty() && !dir.join("disabled.json").exists(), "Start Normally changes no setting");
    ui::end_launch();

    stopped_while_loading_x(&launch_dir, &dir);
    let mut app = launch(&dir, &launch_dir, false);
    assert!(ui::choosing(&app) && !ui::staying_on(&app), "Safe Mode until the user chooses");
    dialogs::cancel(&mut app);
    assert!(!ui::choosing(&app) && ui::staying_on(&app), "Safe Mode for the rest of the launch");
    assert!(safe_mode::active() && registry::list().is_empty(), "Start in Safe Mode");
    assert_eq!(safe_mode::source(), Some(safe_mode::Source::Startup));
    assert!(safe_mode::disabled().is_empty(), "nothing turned off");
    ui::end_launch();
    safe_mode::set_active(None);
    let app = launch(&dir, &launch_dir, false);
    assert_eq!(kind(&app), Some(ui::STOPPED), "Start in Safe Mode leaves the choice to the next launch");
    done(&launch_dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_disabled_list_that_cannot_be_saved_says_so() {
    let _f = fresh();
    let dir = store_with_two("unsaved");
    let launch_dir = dir.join("Launch");
    stopped_while_loading_x(&launch_dir, &dir);
    // A later version's list, which a save keeps as it is.
    let later = r#"{"format": "vectorcraft-disabled-plugins", "version": 2, "items": []}"#;
    std::fs::write(dir.join("disabled.json"), later).unwrap();
    let unsaved = "Couldn't save the disabled plug-ins for the next launch";
    let mut app = launch(&dir, &launch_dir, false);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.ui.status, unsaved, "Disable and Continue");
    assert!(safe_mode::previous().is_some(), "the next launch asks again");
    assert!(registry::get("test.ui.x").is_none(), "this launch still skips it");
    app.ui.status.clear();
    app.run("help.safeMode", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields["items"][0]["enable"] = json!(true);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.ui.status, unsaved, "Enable on Re-launch");
    assert_eq!(std::fs::read_to_string(dir.join("disabled.json")).unwrap(), later);
    done(&launch_dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_safe_mode_label_leaves_the_menus_their_room() {
    let _f = fresh();
    safe_mode::set_active(Some(safe_mode::Source::Flag));
    let menu_names = ["File", "Edit", "Object", "Type", "Select", "Effect", "View", "Window", "Help"];
    for width in [560.0, 720.0, 1280.0] {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 400.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| chrome::app_bar(&mut app, ui));
        out.textures_delta.clear();
        let texts: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some((t.galley.job.text.clone(), t.visual_bounding_rect())),
                _ => None,
            })
            .collect();
        let menus: Vec<egui::Rect> = texts.iter().filter(|(t, _)| menu_names.contains(&t.as_str())).map(|(_, r)| *r).collect();
        assert!(!menus.is_empty(), "{width}: the menus are drawn: {texts:?}");
        let (start, end) = menus.iter().fold((f32::MAX, f32::MIN), |(a, b), r| (a.min(r.left()), b.max(r.right())));
        assert!(texts.iter().any(|(t, _)| t == "Safe Mode"), "{width}: the label is drawn");
        for (t, r) in texts.iter().filter(|(t, _)| !menu_names.contains(&t.as_str())) {
            assert!(r.left() >= end || r.right() <= start, "{width}: {t:?} at {r:?} overlaps the menus from {start} to {end}");
        }
    }
}

#[test]
fn a_dialog_that_replaces_the_choice_leaves_it_waiting() {
    let _f = fresh();
    let dir = store_with_two("replaced");
    let launch_dir = dir.join("Launch");
    stopped_while_loading_x(&launch_dir, &dir);
    let mut app = launch(&dir, &launch_dir, false);
    // Help › Safe Mode (or any other dialog) takes the choice's place before it is answered.
    app.run("help.safeMode", json!({})).unwrap();
    assert_eq!(kind(&app), Some(ui::KIND));
    assert!(ui::choosing(&app) && !ui::staying_on(&app), "still waiting for the choice");
    dialogs::cancel(&mut app);
    assert_eq!(kind(&app), Some(ui::STOPPED), "the choice opens again once no dialog is open");
    assert!(ui::choosing(&app) && registry::list().is_empty());
    dialogs::confirm(&mut app).unwrap();
    assert!(!ui::choosing(&app) && !safe_mode::active());
    assert!(registry::get("test.ui.x").is_none() && registry::get("test.ui.y").is_some());
    done(&launch_dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_safe_mode_flag_asks_nothing_and_loads_nothing() {
    let _f = fresh();
    let dir = store_with_two("flag");
    let launch_dir = dir.join("Launch");
    let app = launch(&dir, &launch_dir, true);
    assert!(kind(&app).is_none() && ui::staying_on(&app));
    assert_eq!((safe_mode::source(), registry::list().len()), (Some(safe_mode::Source::Flag), 0));
    assert_eq!(app.ui.status, vectorcraft_plugins::Error::SafeMode.to_string());
    assert!(chrome::window_title(&app).ends_with("(Safe Mode)"));
    // An object's plug-in effect stays in its Appearance list, marked as skipped.
    assert!(appearance::effect_row_label("plugin.org.example.glow").ends_with("(Safe Mode)"));
    assert!(!appearance::effect_row_label("stylize.dropShadow").contains("Safe Mode"), "built-in effects still draw");
    // Two plug-ins' effects with the same row name: the tooltip names each one's plug-in.
    let (a, b) = (appearance::skipped_effect_tip("plugin.com.a.blur").unwrap(), appearance::skipped_effect_tip("plugin.com.b.blur").unwrap());
    assert!(a.ends_with("\ncom.a.blur") && b.ends_with("\ncom.b.blur"), "{a:?} {b:?}");
    assert_eq!(appearance::skipped_effect_tip("stylize.dropShadow"), None);
    safe_mode::set_active(None);
    assert!(!appearance::effect_row_label("plugin.org.example.glow").contains("Safe Mode"));
    assert_eq!(appearance::skipped_effect_tip("plugin.com.a.blur"), None);
    done(&launch_dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_safe_mode_flag_launch_leaves_the_stopped_plug_in_to_the_next_launch() {
    let _f = fresh();
    let dir = store_with_two("flag-pending");
    let launch_dir = dir.join("Launch");
    stopped_while_loading_x(&launch_dir, &dir);
    let app = launch(&dir, &launch_dir, true);
    assert!(kind(&app).is_none() && registry::list().is_empty(), "--safe-mode asks nothing");
    assert!(safe_mode::previous().is_some(), "the plug-in is still pending");
    ui::end_launch();
    safe_mode::set_active(None);

    // The next launch, without the flag, asks about it before anything loads.
    let mut app = launch(&dir, &launch_dir, false);
    assert_eq!(kind(&app), Some(ui::STOPPED));
    assert!(registry::list().is_empty());
    dialogs::confirm(&mut app).unwrap();
    assert!(registry::get("test.ui.x").is_none() && registry::get("test.ui.y").is_some());
    ui::end_launch();

    // A launch after the choice asks nothing.
    let app = launch(&dir, &launch_dir, false);
    assert!(kind(&app).is_none() && !safe_mode::active());
    done(&launch_dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_safe_mode_enables_a_plug_in_for_the_next_launch_only() {
    let _f = fresh();
    let dir = store_with_two("help");
    let launch_dir = dir.join("Launch");
    let x = Item { id: Some("test.ui.x".into()), name: Some("X\u{202E}gnp".into()), file: None };
    let mut s = Session::new();
    s.attach_plugin_store(dir.clone());
    assert!(safe_mode::set_enabled(&x, false));
    assert!(s.store_disabled_plugins());
    let mut app = launch(&dir, &launch_dir, false);
    assert!(registry::get("test.ui.x").is_none());
    app.run("help.safeMode", json!({})).unwrap();
    assert_eq!(kind(&app), Some(ui::KIND));
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields["items"][0]["label"], "Xgnp", "a name can't reorder the dialog's text");
    frame(&mut app);
    // Close keeps everything as it was.
    dialogs::cancel(&mut app);
    assert_eq!(safe_mode::disabled(), std::slice::from_ref(&x));
    app.run("help.safeMode", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields["items"][0]["enable"] = json!(true);
    dialogs::confirm(&mut app).unwrap();
    assert!(safe_mode::disabled().is_empty());
    assert!(registry::get("test.ui.x").is_none(), "not loaded during this launch");
    ui::end_launch();
    launch(&dir, &launch_dir, false);
    assert!(registry::get("test.ui.x").is_some(), "loaded at the next launch");
    done(&launch_dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_safe_mode_lists_a_plug_in_found_under_several_files_once() {
    let _f = fresh();
    let dir = store_with_two("help-files");
    let launch_dir = dir.join("Launch");
    let mut app = launch(&dir, &launch_dir, false);
    let x = |file: &str| Item { id: Some("test.ui.x".into()), file: Some(file.into()), name: Some("X".into()) };
    let y = Item { id: Some("test.ui.y".into()), file: None, name: Some("Y".into()) };
    let z = Item { id: None, file: Some("z.wasm".into()), name: None };
    // x is turned off by its id, then found under two files, as loading a folder finds it.
    let x_by_id = Item { file: None, ..x("") };
    for item in [&x_by_id, &y, &z] {
        assert!(safe_mode::set_enabled(item, false));
    }
    assert!(safe_mode::disable_file_of(&x("a.wasm")));
    assert!(safe_mode::disable_file_of(&x("b.wasm")));
    assert!(app.session.store_disabled_plugins());
    assert_eq!(safe_mode::disabled().len(), 4, "an entry for each of x's files");
    app.run("help.safeMode", json!({})).unwrap();
    let items = app.ui.dialog.as_ref().unwrap().fields["items"].as_array().unwrap().clone();
    let labels: Vec<&str> = items.iter().map(|v| v["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["X", "Y", "z.wasm"], "one row for each plug-in");
    frame(&mut app);
    app.ui.dialog.as_mut().unwrap().fields["items"][0]["enable"] = json!(true);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(safe_mode::disabled(), [y, z], "every entry with x's id is turned on");
    done(&launch_dir);
    let _ = std::fs::remove_dir_all(&dir);
}
