//! File › Scripts lists the Scripts folder: its scripts and subfolders above Check Script… and
//! Other Script…, names shown as they are, a listed script run without the warning, a script that
//! is gone, the notes and the reads of the folder.

use std::sync::Arc;
use std::time::Duration;

use craft::host::{AppInfo, Host, ThreadJobs};
use craft::menu::Node;
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::i18n::LangExt as _;
use crate::menus;
use crate::{VectorcraftApp, script_runs, scripts_menu};

/// This thread's context, with a host whose jobs run on threads. Tests run on threads of their
/// own, each with its own listing.
fn ctx() -> egui::Context {
    thread_local!(static CTX: egui::Context = {
        let ctx = crate::testing::ui_context();
        Host::builder(AppInfo { id: "vectorcraft", name: "VectorCraft", version: "0" }).jobs(Arc::new(ThreadJobs)).install(&ctx);
        ctx
    });
    CTX.with(Clone::clone)
}

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
    app.ui_ctx = Some(ctx());
    app.run("file.new", json!({})).unwrap();
    app
}

/// Reads the folder again and waits for the listing.
fn read(app: &VectorcraftApp) {
    scripts_menu::refresh(app, &ctx());
    scripts_menu::finish();
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
fn scripts() -> Vec<Node> {
    scripts_menu::items()
}

/// The File › Scripts submenu's entries as the menu bar shows them: the first submenu that lists a
/// script itself (its folders come after it).
fn shown_scripts(nodes: &[Node]) -> Option<&[Node]> {
    nodes.iter().find_map(|node| match node {
        Node::Submenu { children, .. } if children.iter().any(|c| matches!(c, Node::Item(i) if i.command.id == scripts_menu::COMMAND)) => {
            Some(children.as_slice())
        }
        Node::Submenu { children, .. } => shown_scripts(children),
        _ => None,
    })
}

/// The label of an item or a submenu.
fn label(node: &Node) -> Option<String> {
    match node {
        Node::Item(item) => Some(item.label.clone()),
        Node::Submenu { label, .. } => Some(label.clone()),
        _ => None,
    }
}

/// Menu items as text: a command's label, `label/[…]` for a submenu, `(text)` for a note and `-`
/// for a separator.
fn shape(nodes: &[Node]) -> String {
    let one = |node: &Node| match node {
        Node::Item(item) => item.label.clone(),
        Node::Submenu { label, children, .. } => format!("{label}/[{}]", shape(children)),
        Node::Heading(text) => format!("({text})"),
        Node::Separator => "-".to_string(),
        _ => "?".to_string(),
    };
    nodes.iter().map(one).collect::<Vec<_>>().join(", ")
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

/// The items File › Scripts always ends with.
const FIXED: &str = "Check Script…, Other Script…";

#[test]
fn the_folders_scripts_are_listed_above_check_script_and_other_script() {
    let dir = folder("listed");
    let app = app(&dir);
    assert_eq!(shape(&scripts()), FIXED, "before the first read");
    read(&app);
    assert_eq!(shape(&scripts()), FIXED, "an empty folder");
    let draw = write(&dir, "Draw Two.jsx", DRAW);
    write(&dir, "Export/All Artboards.js", "");
    write(&dir, "notes.txt", "");
    read(&app);
    let items = scripts();
    assert_eq!(shape(&items), format!("Draw Two, Export/[All Artboards], -, {FIXED}"));
    assert!(
        matches!(&items[0], Node::Item(i) if i.command.id == scripts_menu::COMMAND && i.command.params == json!({ "path": draw })),
        "{:?}",
        items[0]
    );
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
    let Node::Item(item) = &items[0] else { panic!("{:?}", items[0]) };
    let (label, id, p) = (item.label.as_str(), item.command.id.as_str(), &item.command.params);
    let undo = app.session.doc().unwrap().history.undo.len();
    assert!(app.session.prefs.show_external_script_warning);
    let (id, p) = menus::click_target(label, id, p);
    menus::invoke(&mut app, &id, p);
    assert!(!app.any_dialog_open(), "no warning for a script of the folder");
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
    let Node::Item(item) = &items[1] else { panic!("{:?}", items[1]) };
    let (label, id, p) = (item.label.as_str(), item.command.id.as_str(), &item.command.params);
    assert_eq!(menus::click_target(label, id, p), (scripts_menu::COMMAND.to_string(), json!({ "path": dots })));
    menus::run_ui_command(&mut app, id, p).unwrap().unwrap();
    finish(&mut app);
    assert_eq!(objects(&app), 2, "the file the item names ran, not its neighbor");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_listed_script_reads_its_included_files_and_runs_files_beside_it() {
    let dir = folder("include");
    let mut app = app(&dir);
    write(&dir, "lib/two.jsxinc", DRAW);
    write(&dir, "lib/one.js", "vc.run('shape.rectangle', {x:40,y:0,width:10,height:10});");
    let main = write(&dir, "Main.jsx", "#include \"lib/two.jsxinc\"\n$.evalFile('lib/one.js');\n");
    read(&app);
    menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({ "path": main })).unwrap().unwrap();
    finish(&mut app);
    assert_eq!(objects(&app), 3, "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn script_and_folder_names_are_shown_as_they_are_in_every_language() {
    let dir = folder("language");
    let mut app = app(&dir);
    write(&dir, "Layers.js", "");
    write(&dir, "Export/Layers.jsx", "");
    read(&app);
    let de = crate::i18n::Lang::from_code("de").unwrap();
    assert!(crate::i18n::tr(de, "Layers") != "Layers" && crate::i18n::tr(de, "Export") != "Export", "German translates both words");
    crate::i18n::set_current(de);
    menus::model::refresh(&mut app, &ctx());
    // The bar keeps an item's own label; the row shows its state's title.
    let shown = |node: &Node| match node {
        Node::Item(item) => Some(crate::menus::shown_label(&app, &item.command.id, &item.command.params, &item.label)),
        other => label(other),
    };
    let labels: Vec<String> =
        app.menus.bar.menus.iter().find_map(|m| shown_scripts(&m.children)).expect("File › Scripts").iter().filter_map(shown).collect();
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
    assert!(!crate::palette::items().iter().any(|(_, id)| id == scripts_menu::COMMAND), "a script runs from its menu item");
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
    scripts_menu::finish();
    assert_eq!(shape(&scripts()), format!("kept, -, {FIXED}"));
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
    let (tx, frames) = std::sync::mpsc::channel();
    let test = std::thread::current().id();
    // The observer sees every request; the repaint callback drops one no sooner than the first read's, still pending.
    ctx().set_repaint_observer(move |_| {
        // The read's lane asks from a thread of its own.
        if std::thread::current().id() != test {
            let _ = tx.send(());
        }
    });
    assert!(menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({ "path": gone })).unwrap().is_err());
    // With the app's copy of the context dropped, only the queue's wake can still ask for a frame.
    app.ui_ctx = None;
    let asked = frames.recv_timeout(Duration::from_secs(10));
    ctx().set_repaint_observer(|_| {});
    assert_eq!(asked, Ok(()), "the finished read asks for a frame so an idle window takes it in");
    scripts_menu::finish();
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
    assert_eq!(shape(&items[items.len() - 4..]), format!("({}), -, {FIXED}", scripts_menu::TRUNCATED));
    // A file where the folder should be.
    std::fs::remove_dir_all(dir.join("Scripts")).unwrap();
    std::fs::write(dir.join("Scripts"), "").unwrap();
    read(&app);
    assert_eq!(shape(&scripts()), format!("({}), -, {FIXED}", scripts_menu::UNREADABLE));
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
fn the_macos_menu_bar_lists_the_same_scripts_and_follows_a_new_listing() {
    let dir = folder("native");
    let app = app(&dir);
    let path = write(&dir, "Export/All Artboards.js", "");
    let before = scripts_menu::revision();
    scripts_menu::refresh(&app, &ctx());
    let noting = scripts_menu::revision();
    assert_ne!(noting, before, "the loading note changes what the menus show");
    scripts_menu::finish();
    let listed = scripts_menu::revision();
    assert_ne!(listed, noting, "the listing takes the note's place");
    let items = scripts_menu::items();
    let [Node::Submenu { label, children, .. }, Node::Separator, Node::Item(check), Node::Item(other)] = items.as_slice() else {
        panic!("{items:?}")
    };
    assert_eq!((label.as_str(), check.command.id.as_str(), other.command.id.as_str()), ("Export", "ui.checkScript", "ui.runScript"));
    let [Node::Item(script)] = children.as_slice() else { panic!("{children:?}") };
    assert_eq!(
        (script.label.as_str(), script.command.id.as_str(), &script.command.params),
        ("All Artboards", scripts_menu::COMMAND, &json!({ "path": path }))
    );
    assert!(script.enabled && script.shortcut.is_none());
    // A read that finds the same scripts changes nothing the menu bar would rebuild for.
    read(&app);
    assert_eq!(scripts_menu::revision(), listed);
    write(&dir, "More.jsx", "");
    read(&app);
    assert_ne!(scripts_menu::revision(), listed);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_folder_is_read_at_the_first_frame_and_when_the_window_becomes_active() {
    let dir = folder("focus");
    let app = app(&dir);
    write(&dir, "first.js", "");
    let ctx = ctx();
    let frame = |events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| scripts_menu::poll(&app, ui.ctx()));
        out.textures_delta.clear();
        scripts_menu::finish();
    };
    let listed = || shape(&scripts());
    frame(vec![]);
    assert!(listed().starts_with("first, -"), "{}", listed());
    // A pass without the window becoming active leaves the folder alone, focused or not.
    write(&dir, "second.js", "");
    frame(vec![]);
    frame(vec![egui::Event::WindowFocused(false)]);
    assert!(listed().starts_with("first, -"), "{}", listed());
    frame(vec![egui::Event::WindowFocused(true)]);
    assert!(listed().starts_with("first, second, -"), "{}", listed());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_app_without_a_scripts_folder_keeps_the_two_fixed_items() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    read(&app);
    assert_eq!(shape(&scripts()), FIXED, "no loading note: there is no folder to read");
    let e = menus::run_ui_command(&mut app, scripts_menu::COMMAND, &json!({ "path": "draw.js" })).unwrap().unwrap_err();
    assert_eq!(e, "the script is no longer in the Scripts folder");
    assert_eq!(menus::UI_COMMANDS.iter().filter(|c| c.0 == scripts_menu::COMMAND).count(), 1);
    let _: Value = app.run("script.listInstalled", json!({})).unwrap();
}

#[test]
fn the_first_read_shows_a_loading_note_until_its_listing_comes() {
    let dir = folder("loading");
    let app = app(&dir);
    write(&dir, "first.js", "");
    scripts_menu::refresh(&app, &ctx());
    // The job runs, or has ended with its result not taken yet.
    assert_eq!(shape(&scripts()), format!("({}), -, {FIXED}", scripts_menu::LOADING));
    scripts_menu::finish();
    assert_eq!(shape(&scripts()), format!("first, -, {FIXED}"));
    let listed = scripts_menu::revision();
    // A later read keeps the listing, with no note, while it is under way.
    write(&dir, "second.js", "");
    scripts_menu::refresh(&app, &ctx());
    assert_eq!(shape(&scripts()), format!("first, -, {FIXED}"));
    assert_eq!(scripts_menu::revision(), listed);
    scripts_menu::finish();
    assert_eq!(shape(&scripts()), format!("first, second, -, {FIXED}"));
    assert_ne!(scripts_menu::revision(), listed);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_empty_or_unreadable_first_listing_takes_the_place_of_the_loading_note() {
    let dir = folder("loading-empty");
    let app = app(&dir);
    scripts_menu::refresh(&app, &ctx());
    let noting = scripts_menu::revision();
    assert_eq!(shape(&scripts()), format!("({}), -, {FIXED}", scripts_menu::LOADING));
    scripts_menu::finish();
    assert_eq!(shape(&scripts()), FIXED);
    assert_ne!(scripts_menu::revision(), noting, "the note went, though the listing is the empty one it started from");
    let _ = std::fs::remove_dir_all(dir);
    // A thread of its own has a menu that has not listed yet.
    let dir = folder("loading-unreadable");
    std::fs::remove_dir_all(dir.join("Scripts")).unwrap();
    std::fs::write(dir.join("Scripts"), "").unwrap();
    let there = dir.clone();
    std::thread::spawn(move || {
        let app = self::app(&there);
        read(&app);
        assert_eq!(shape(&scripts()), format!("({}), -, {FIXED}", scripts_menu::UNREADABLE));
    })
    .join()
    .unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_slow_first_read_says_so_until_its_listing_comes() {
    let dir = folder("slow");
    let app = app(&dir);
    write(&dir, "first.js", "");
    let asked = craft::platform::clock::monotonic_ms() / 1000.0;
    scripts_menu::refresh(&app, &ctx());
    let started = craft::platform::clock::monotonic_ms() / 1000.0;
    // The read started between `asked` and `started`, and its result is not taken in yet.
    let before = scripts_menu::revision();
    assert_eq!(shape(&scripts_menu::items_at(asked + scripts_menu::SLOW_SECS / 2.0)), format!("({}), -, {FIXED}", scripts_menu::LOADING));
    assert_eq!(shape(&scripts_menu::items_at(started + 1.5 * scripts_menu::SLOW_SECS)), format!("({}), -, {FIXED}", scripts_menu::SLOW));
    assert_eq!(scripts_menu::revision(), before, "the note follows the clock, not a flag");
    scripts_menu::finish();
    assert_eq!(shape(&scripts_menu::items_at(started + 10.0 * scripts_menu::SLOW_SECS)), format!("first, -, {FIXED}"));
    let _ = std::fs::remove_dir_all(dir);
}
