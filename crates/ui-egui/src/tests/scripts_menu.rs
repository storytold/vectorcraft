//! File › Scripts lists the Scripts folder: its scripts and subfolders above Check Script… and
//! Other Script…, names shown as they are, a listed script run without the warning, a script that
//! is gone, the notes, the macOS menu bar and the reads of the folder.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::menus::{self, Entry, Item};
use crate::native_menu::{self, Node};
use crate::{VectorcraftApp, script_runs, scripts_menu};

/// A fresh folder for test `name`, holding an empty `Scripts` folder.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-scripts-menu-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Scripts")).unwrap();
    dir
}

/// An app with a document whose Scripts folder is `dir`'s.
fn app(dir: &std::path::Path) -> VectorcraftApp {
    let mut session = Session::new();
    session.attach_scripts_dir(dir.join("Scripts"));
    let mut app = VectorcraftApp::new(session, crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app
}

/// Reads the folder again and waits for the listing.
fn read(app: &VectorcraftApp) {
    scripts_menu::refresh(app, None);
    scripts_menu::wait();
}

/// Writes the script `name` (`/` between its folders) in `dir`'s Scripts folder; returns its path
/// as a listing gives it.
fn write(dir: &std::path::Path, name: &str, text: &str) -> String {
    let path = name.split('/').fold(dir.join("Scripts"), |path, part| path.join(part));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

/// File › Scripts' items.
fn scripts() -> Vec<Item> {
    let (_, file) = menus::menu_tree().into_iter().find(|(title, _)| *title == "File").unwrap();
    file.into_iter()
        .find_map(|item| match item {
            Item::Sub("Scripts", items) => Some(items),
            _ => None,
        })
        .unwrap()
}

/// Menu items as text: a command's label, `label/[…]` for a submenu, `(text)` for a note and `-`
/// for a separator.
fn shape(items: &[Item]) -> String {
    let one = |item: &Item| match item {
        Item::Cmd(label, ..) | Item::Todo(label, _) => label.to_string(),
        Item::Sub(label, items) => format!("{label}/[{}]", shape(items)),
        Item::Header(text) => format!("({text})"),
        Item::Sep => "-".to_string(),
    };
    items.iter().map(one).collect::<Vec<_>>().join(", ")
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

fn finish(app: &mut VectorcraftApp) {
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return;
        }
        script_runs::poll(app);
    }
    panic!("the script did not end");
}

const DRAW: &str = "vc.run('shape.rectangle', {x:0,y:0,width:10,height:10}); vc.run('shape.rectangle', {x:20,y:0,width:10,height:10});";

#[test]
fn the_folders_scripts_are_listed_above_check_script_and_other_script() {
    let dir = folder("listed");
    let app = app(&dir);
    assert_eq!(shape(&scripts()), "Check Script…, Other Script…", "before the first listing");
    read(&app);
    assert_eq!(shape(&scripts()), "Check Script…, Other Script…", "an empty folder");
    let draw = write(&dir, "Draw Two.jsx", DRAW);
    write(&dir, "Export/All Artboards.js", "");
    write(&dir, "notes.txt", "");
    read(&app);
    let items = scripts();
    assert_eq!(shape(&items), "Draw Two, Export/[All Artboards], -, Check Script…, Other Script…");
    assert!(matches!(&items[0], Item::Cmd(_, scripts_menu::COMMAND, p) if *p == json!({ "path": draw })), "{:?}", items[0]);
    // A script added to the folder is listed by the next read, with no restart.
    write(&dir, "Added.js", "");
    assert!(shape(&scripts()).starts_with("Draw Two"), "the menus keep the last listing");
    read(&app);
    assert!(shape(&scripts()).starts_with("Added, Draw Two"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_listed_script_runs_without_the_warning_as_one_undo_step() {
    let dir = folder("run");
    let mut app = app(&dir);
    write(&dir, "Dessin (été) #1.JSX", DRAW);
    read(&app);
    let items = scripts();
    let Item::Cmd(label, id, p) = &items[0] else { panic!("{:?}", items[0]) };
    let undo = app.session.doc().unwrap().history.undo.len();
    assert!(app.session.prefs.show_external_script_warning);
    let (id, p) = menus::click_target(label, id, p);
    menus::invoke(&mut app, &id, p);
    assert!(app.ui.dialog.is_none(), "no warning for a script of the folder");
    assert!(app.session.script_running());
    finish(&mut app);
    assert_eq!(objects(&app), 2);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(app.ui.status, "Ran Dessin (été) #1.JSX");
    assert!(app.session.prefs.show_external_script_warning, "Other Script… still warns");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_script_runs_its_own_file_whatever_its_name_ends_with() {
    let dir = folder("names");
    let mut app = app(&dir);
    // A label that ends like a menu item that asks for its parameters first.
    let dots = write(&dir, "Draw….js", DRAW);
    write(&dir, "Draw.js", "vc.run('shape.rectangle', {x:0,y:0,width:1,height:1});");
    read(&app);
    let items = scripts();
    assert_eq!(shape(&items[..2]), "Draw, Draw…");
    let Item::Cmd(label, id, p) = &items[1] else { panic!("{:?}", items[1]) };
    assert_eq!(menus::click_target(label, id, p), (scripts_menu::COMMAND.to_string(), json!({ "path": dots })));
    menus::run_ui_command(&mut app, id, p).unwrap().unwrap();
    finish(&mut app);
    assert_eq!(objects(&app), 2, "the file the item names ran, not its neighbor");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn script_and_folder_names_are_shown_as_they_are_in_every_language() {
    let dir = folder("language");
    let app = app(&dir);
    write(&dir, "Layers.js", "");
    write(&dir, "Export/Layers.jsx", "");
    read(&app);
    let de = crate::i18n::Lang::from_code("de").unwrap();
    assert!(crate::i18n::tr(de, "Layers") != "Layers" && crate::i18n::tr(de, "Export") != "Export", "German translates both words");
    crate::i18n::set_current(de);
    let items = scripts();
    let labels: Vec<String> = items
        .iter()
        .filter_map(|item| match menus::entry(&app, item)? {
            Entry::Sub(label, _) => Some(label.to_string()),
            Entry::Item(row) => Some(row.label.to_string()),
            _ => None,
        })
        .collect();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(labels[..2], ["Export", "Layers"]);
    assert_ne!(labels[2], "Check Script…", "the fixed items are translated");
    // The catalogs hold the notes, and no file or folder name.
    let strings = menus::menu_strings();
    assert!(scripts_menu::LABELS.iter().all(|note| strings.contains(*note)));
    assert!(strings.contains("Layers") && !strings.contains("All Artboards"));
    write(&dir, "Zed/All Artboards.js", "");
    read(&app);
    assert!(!menus::menu_strings().contains("All Artboards") && !menus::menu_strings().contains("Zed"));
    assert!(!crate::palette::items().iter().any(|(_, id, _)| id == scripts_menu::COMMAND), "a script runs from its menu item");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_script_that_left_the_folder_is_not_run_and_the_folder_is_read_again() {
    let dir = folder("gone");
    let mut app = app(&dir);
    let gone = write(&dir, "gone.js", DRAW);
    write(&dir, "kept.js", "");
    read(&app);
    std::fs::remove_file(&gone).unwrap();
    let e = menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({ "path": gone })).unwrap().unwrap_err();
    assert_eq!(e, "the script is no longer in the Scripts folder");
    assert!(!app.session.script_running());
    scripts_menu::wait();
    assert_eq!(shape(&scripts()), "kept, -, Check Script…, Other Script…");
    // A file outside the folder is not one of its scripts, and a call needs a path.
    let outside = dir.join("outside.js");
    std::fs::write(&outside, DRAW).unwrap();
    let e = menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({ "path": outside.to_string_lossy() })).unwrap().unwrap_err();
    assert_eq!(e, "the script is no longer in the Scripts folder");
    assert!(menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({})).unwrap().is_err());
    assert!(menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({ "path": 3 })).unwrap().is_err());
    finish(&mut app);
    assert_eq!(objects(&app), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_read_after_a_script_that_left_the_folder_asks_for_a_frame() {
    let dir = folder("gone-frame");
    let mut app = app(&dir);
    let gone = write(&dir, "gone.js", DRAW);
    read(&app);
    std::fs::remove_file(&gone).unwrap();
    let ctx = egui::Context::default();
    let (tx, frames) = std::sync::mpsc::channel();
    ctx.set_request_repaint_callback(move |_| {
        let _ = tx.send(());
    });
    app.ui_ctx = Some(ctx);
    assert!(menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({ "path": gone })).unwrap().is_err());
    // With the app's copy of the context dropped, only the read's thread can still ask for a
    // frame; the channel closes without one when that thread ends without asking.
    app.ui_ctx = None;
    assert_eq!(frames.recv(), Ok(()), "the finished read asks for a frame so an idle window takes it in");
    scripts_menu::wait();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_menu_notes_a_listing_that_stopped_and_a_folder_it_cannot_read() {
    let dir = folder("notes");
    let app = app(&dir);
    for n in 0..vectorcraft_engine::script::folder::MAX_SCRIPTS + 1 {
        write(&dir, &format!("s{n:04}.js"), "");
    }
    read(&app);
    let items = scripts();
    assert_eq!(items.len(), vectorcraft_engine::script::folder::MAX_SCRIPTS + 4);
    assert_eq!(shape(&items[items.len() - 4..]), format!("({}), -, Check Script…, Other Script…", scripts_menu::TRUNCATED));
    // A file where the folder should be.
    std::fs::remove_dir_all(dir.join("Scripts")).unwrap();
    std::fs::write(dir.join("Scripts"), "").unwrap();
    read(&app);
    assert_eq!(shape(&scripts()), format!("({}), -, Check Script…, Other Script…", scripts_menu::UNREADABLE));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_menu_listing_under_a_read_root_leaves_out_scripts_outside_it() {
    use vectorcraft_engine::file_access::{AutomationRoots, confine};
    let dir = folder("read-root");
    let app = app(&dir);
    let path = write(&dir, "Export/All Artboards.js", "");
    read(&app);
    let scripts = || menus::menu_entries(&app).into_iter().filter(|e| e.command.as_deref() == Some(scripts_menu::COMMAND)).collect::<Vec<_>>();
    let listed = scripts();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (&listed[0].path, &listed[0].label, &listed[0].params),
        (&vec!["File".to_string(), "Scripts".into(), "Export".into()], &"All Artboards".to_string(), &json!({ "path": path }))
    );
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let outside = AutomationRoots::new(&[&work], &[]).unwrap().unwrap();
    assert!(confine(Some(&outside), scripts).is_empty());
    let inside = AutomationRoots::new(&[&dir], &[]).unwrap().unwrap();
    assert_eq!(confine(Some(&inside), scripts).len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn names_past_the_session_limit_are_left_out_with_the_note() {
    let dir = folder("names-limit");
    let app = app(&dir);
    write(&dir, "Kept.js", "");
    read(&app);
    assert_eq!(shape(&scripts()), "Kept, -, Check Script…, Other Script…");
    // Names other listings showed fill what one session keeps.
    for n in 0..scripts_menu::MAX_NAMES {
        if scripts_menu::intern(&format!("renamed {n}")).is_none() {
            break;
        }
    }
    write(&dir, "New.js", "");
    write(&dir, "Folder/Inside.js", "");
    read(&app);
    assert_eq!(shape(&scripts()), format!("Kept, ({}), -, Check Script…, Other Script…", scripts_menu::TRUNCATED));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_macos_menu_bar_lists_the_same_scripts_and_follows_a_new_listing() {
    let dir = folder("native");
    let app = app(&dir);
    let path = write(&dir, "Export/All Artboards.js", "");
    let before = scripts_menu::revision();
    read(&app);
    assert_eq!(scripts_menu::revision(), before + 1);
    let bar = native_menu::from_tree(&app, &menus::menu_tree());
    let file = bar.menus.iter().find(|m| m.title == "File").unwrap();
    let scripts = file.children.iter().find_map(|n| match n {
        Node::Submenu { label, children } if label == "Scripts" => Some(children),
        _ => None,
    });
    let Some([Node::Submenu { label, children }, Node::Separator, Node::Item(check), Node::Item(other)]) = scripts.map(Vec::as_slice) else {
        panic!("{scripts:?}")
    };
    assert_eq!((label.as_str(), check.command, other.command), ("Export", Some("ui.checkScript"), Some("ui.runScript")));
    let [Node::Item(script)] = children.as_slice() else { panic!("{children:?}") };
    assert_eq!((script.label.as_str(), script.command, &script.params), ("All Artboards", Some(scripts_menu::COMMAND), &json!({ "path": path })));
    assert!(script.enabled && script.shortcut.is_none());
    // A read that finds the same scripts changes nothing the menu bar would rebuild for.
    read(&app);
    assert_eq!(scripts_menu::revision(), before + 1);
    write(&dir, "More.jsx", "");
    read(&app);
    assert_eq!(scripts_menu::revision(), before + 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_folder_is_read_at_the_first_frame_and_when_the_window_becomes_active() {
    let dir = folder("focus");
    let app = app(&dir);
    write(&dir, "first.js", "");
    let ctx = egui::Context::default();
    let frame = |focused: bool| {
        let mut out = ctx.run_ui(egui::RawInput { focused, ..Default::default() }, |ui| scripts_menu::poll(&app, ui.ctx()));
        out.textures_delta.clear();
        scripts_menu::wait();
    };
    let listed = || shape(&scripts());
    frame(true);
    assert!(listed().starts_with("first, -"), "{}", listed());
    // While the window stays active, or stays in the background, the folder is left alone.
    write(&dir, "second.js", "");
    frame(true);
    frame(false);
    frame(false);
    assert!(listed().starts_with("first, -"), "{}", listed());
    frame(true);
    assert!(listed().starts_with("first, second, -"), "{}", listed());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_read_asked_for_during_another_runs_when_that_one_ends() {
    let dir = folder("pending");
    let app = app(&dir);
    write(&dir, "first.js", "");
    let ctx = egui::Context::default();
    let frame = || {
        let mut out = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| scripts_menu::poll(&app, ui.ctx()));
        out.textures_delta.clear();
    };
    frame();
    scripts_menu::wait();
    // A read that has passed the folder when a script is added, and the File menu opening then.
    scripts_menu::refresh(&app, None);
    scripts_menu::end_untaken();
    write(&dir, "added.js", "");
    scripts_menu::refresh(&app, None);
    frame();
    scripts_menu::wait();
    assert!(shape(&scripts()).starts_with("added, first, -"), "{}", shape(&scripts()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_app_without_a_scripts_folder_keeps_the_two_fixed_items() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    read(&app);
    assert_eq!(shape(&scripts()), "Check Script…, Other Script…");
    let e = menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({ "path": "draw.js" })).unwrap().unwrap_err();
    assert_eq!(e, "the script is no longer in the Scripts folder");
    assert_eq!(menus::UI_COMMANDS.iter().filter(|c| c.0 == scripts_menu::COMMAND).count(), 1);
    let _: Value = app.run("script.listInstalled", json!({})).unwrap();
}
