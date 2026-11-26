//! The Script Console panel's commands: Clear works during a run, Copy Selected and Copy All through
//! `ui.scriptConsole.copySelected` and `ui.copyScriptConsole`.

use craft::behavior::Selection;
use craft::keymap::CommandRef;
use craft::ui::runtime::commands::CommandStates;
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::script::console::{Ending, Entry};

use super::script_console::{menu_states, show, shown};
use crate::panel_docking::PANEL_MIN_WIDTH;
use crate::tests::panel_harness::{Panel, Place, ROOM, floated};
use crate::{Services, VectorcraftApp, script_runs};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({})).unwrap();
    app
}

/// Runs `source` to its end, a frame at a time, as the app does.
fn run(app: &mut VectorcraftApp, source: &str) {
    app.session.script_frames = true;
    app.run("script.run", json!({"source": source, "name": "c.js"})).unwrap();
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return;
        }
        script_runs::poll(app);
    }
    panic!("the script did not end");
}

/// Whether `menu_states` enables the console command `id`.
fn enabled(app: &VectorcraftApp, id: &str) -> bool {
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    states.get(&CommandRef::new(id)).is_some_and(|s| s.is_enabled())
}

#[test]
fn the_console_footer_stays_inside_the_panel_at_minimum_width() {
    let names = ["Clear", "Copy Selected", "Copy All"];
    for lang in ["en", "fr", "de"] {
        let mut app = app();
        run(&mut app, "$.writeln('one')");
        let panel = floated(app, "scriptConsole", lang, Place::Floating(PANEL_MIN_WIDTH), &names);
        for name in names {
            let bounds = panel.rects(name);
            assert!(
                bounds.iter().all(|rect| panel.body.expand(0.5).contains_rect(*rect)),
                "{lang}: {name} at {bounds:?} is outside {:?}",
                panel.body
            );
        }
    }

    let wide = floated(app(), "scriptConsole", "en", Place::Floating(380.0), &names);
    assert!(wide.middles(&names).windows(2).all(|pair| (pair[0] - pair[1]).abs() < 1.0), "the wide footer uses one row");
}

#[test]
fn the_console_footer_buttons_run_their_commands_at_minimum_width() {
    let mut app = app();
    run(&mut app, "$.writeln('one'); $.writeln('two')");
    let mut panel = Panel::new(app, show);
    panel.room(egui::vec2(PANEL_MIN_WIDTH, ROOM.y)).frame();
    assert!(!panel.enabled("Copy Selected"));
    panel.click("#1 c.js: one");
    panel.click("Copy Selected");
    assert_eq!(panel.app().clipboard_out.take().as_deref(), Some("#1 c.js: one"));
    panel.click("Copy All");
    assert!(panel.app().clipboard_out.take().is_some_and(|text| text.contains("#1 c.js: two")));
    panel.click("Clear");
    assert!(panel.app().session.script_console().lines().is_empty());
    assert!(!panel.enabled("Copy All"));
}

#[test]
fn clear_works_during_a_run_and_the_run_goes_on_writing() {
    let mut app = app();
    app.session.script_frames = true;
    app.run("script.run", json!({"source": "$.writeln('before'); while (true) {}", "name": "c.js"})).unwrap();
    script_runs::poll(&mut app);
    assert!(app.session.script_running());
    assert_eq!(app.run("script.console.clear", json!({})).unwrap()["cleared"], 2);
    app.run("script.stop", json!({})).unwrap();
    script_runs::poll(&mut app);
    assert!(!app.session.script_running());
    let lines = app.session.script_console().lines();
    assert!(lines.iter().all(|l| !shown(l).contains("before")), "the cleared lines stay gone");
    assert!(lines.iter().any(|l| matches!(l.entry, Entry::End(Ending::Stopped))), "the run wrote its end after the clear");
}

#[test]
fn copy_all_and_copy_selected_put_the_lines_on_the_clipboard() {
    let mut app = app();
    run(&mut app, "$.writeln('one'); $.writeln('two'); $.writeln('three')");
    assert_eq!(app.run("ui.copyScriptConsole", json!({})).unwrap()["lines"], 6);
    let all = app.clipboard_out.take().unwrap();
    assert!(all.starts_with("#1 c.js: ▶\n#1 c.js: one\n#1 c.js: two\n"), "{all}");
    assert_eq!(app.run("ui.copyScriptConsole", json!({"first": 2, "last": 3})).unwrap()["lines"], 2);
    assert_eq!(app.clipboard_out.take().as_deref(), Some("#1 c.js: two\n#1 c.js: three"));
    for bad in [json!({"first": 3, "last": 2}), json!({"first": 1}), json!({"first": 0, "last": 99}), json!({"first": "1", "last": 2})] {
        assert!(app.run("ui.copyScriptConsole", bad.clone()).is_err(), "{bad}");
    }
    assert!(app.clipboard_out.is_none(), "a refused copy copies nothing");
    run(&mut app, "for (var i = 0; i < 2100; i++) $.writeln(i)");
    app.run("ui.copyScriptConsole", json!({})).unwrap();
    let all = app.clipboard_out.take().unwrap();
    assert!(all.lines().next().is_some_and(|l| l.contains("lines dropped")), "Copy All starts with the notice: {}", &all[..80]);
}

#[test]
fn copy_selected_copies_from_the_first_to_the_last_selected_line() {
    let mut app = app();
    run(&mut app, "for (var i = 0; i < 12; i++) $.writeln('l' + i)");
    assert!(!enabled(&app, "ui.scriptConsole.copySelected"), "nothing is selected");
    assert!(app.run("ui.scriptConsole.copySelected", json!({})).is_err());
    // Several rows in any order: lines 2 to 9 are copied.
    app.ui.script_console_selection = Selection::new(vec![5, 2, 9], Some(9), Some(5));
    assert!(enabled(&app, "ui.scriptConsole.copySelected"));
    assert_eq!(app.run("ui.scriptConsole.copySelected", json!({})).unwrap()["lines"], 8);
    let want = (1..=8).map(|i| format!("#1 c.js: l{i}")).collect::<Vec<_>>().join("\n");
    assert_eq!(app.clipboard_out.take(), Some(want));
    app.run("script.console.clear", json!({})).unwrap();
    assert!(!enabled(&app, "ui.scriptConsole.copySelected"), "the selected lines are gone");
    assert!(!enabled(&app, "script.console.clear") && !enabled(&app, "ui.copyScriptConsole"));
    assert!(app.run("ui.scriptConsole.copySelected", json!({})).is_err());
}
