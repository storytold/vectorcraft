//! The Script Console panel drawn headless: a full console lays out only the rows in view, output
//! goes on while the panel is hidden, Clear works during a run, and Copy Selected and Copy All
//! through `ui.copyScriptConsole`.

use serde_json::json;
use vectorcraft_engine::Session;

use super::pstate;
use super::script_console::{DRAWN, ID, show};
use crate::tests_labels::painted_text;
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

#[test]
fn a_full_console_lays_out_only_the_rows_in_view() {
    let mut app = app();
    run(&mut app, "for (var i = 0; i < 3000; i++) $.writeln('line ' + i)");
    assert_eq!(app.session.script_console().lines().len(), vectorcraft_engine::script::console::MAX_CONSOLE_LINES);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut drawn = 0;
    for _ in 0..2 {
        ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui);
            drawn = pstate::<usize>(ui.ctx(), DRAWN);
        })
        .textures_delta
        .clear();
    }
    assert!(drawn > 0 && drawn < 40, "{drawn} rows laid out");
    let text = painted_text(&mut app, show);
    assert!(text.contains("#1 c.js: line 2999"), "the newest line is in view: {text}");
    assert!(text.contains("lines dropped"), "the panel says lines were dropped: {text}");
}

#[test]
fn output_goes_on_while_the_panel_is_hidden_and_shows_when_it_opens() {
    let mut app = app();
    assert!(crate::state::rail_panel(ID).is_some());
    run(&mut app, "$.writeln('hidden'); vc.log({n: 1}); throw new Error('late')");
    app.run("window.panel", json!({"panel": ID})).unwrap();
    let text = painted_text(&mut app, show);
    for want in ["#1 c.js: ▶", "#1 c.js: hidden", "#1 c.js: {\"n\":1}", "#1 c.js: c.js:1:", "#1 c.js: ■ Ended with an error", "host calls"] {
        assert!(text.contains(want), "{want} in {text}");
    }
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
    let text = painted_text(&mut app, show);
    assert!(!text.contains("before") && text.contains("■ Stopped"), "{text}");
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
