//! The Script Console panel drawn headless: a full console lays out only the rows in view, output
//! goes on while the panel is hidden, Clear works during a run, and Copy Selected and Copy All
//! through `ui.copyScriptConsole`.

use serde_json::json;
use vectorcraft_engine::Session;

use super::pstate;
use super::script_console::{DRAWN, ID, show, shown};
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

/// A report line reads in English as `script.console` gives it: "time unavailable" for a run
/// without a clock, one host call in the singular, and the slowest commands.
#[test]
fn a_report_line_reads_as_script_console_gives_it() {
    use vectorcraft_engine::script::console::{ConsoleLine, Entry, Slow};
    let slow = vec![Slow { id: "a.b".into(), calls: 2, total_ms: 7, max_ms: 4 }, Slow { id: "c".into(), calls: 1, total_ms: 1, max_ms: 1 }];
    for ms in [None, Some(0), Some(12)] {
        for host_calls in [0, 1, 2] {
            for slowest in [vec![], slow.clone()] {
                let entry = Entry::Report { ms, host_calls, slowest };
                let want = format!("#3 c.js: {}", entry.text());
                assert_eq!(shown(&ConsoleLine { run: 3, name: "c.js".into(), entry }), want);
            }
        }
    }
    let line = |ms, host_calls| shown(&ConsoleLine { run: 1, name: "c.js".into(), entry: Entry::Report { ms, host_calls, slowest: vec![] } });
    assert_eq!(line(None, 1), "#1 c.js: time unavailable, 1 host call");
    assert_eq!(line(Some(0), 2), "#1 c.js: 0 ms, 2 host calls");
}

/// The notice counts the lines dropped and the lines cut each in its own form, a count of one in
/// the singular; Copy All starts with it.
#[test]
fn the_notice_counts_one_line_in_the_singular() {
    for (dropped, cut, want) in [
        (1, 0, "1 earlier line dropped, 0 lines cut"),
        (0, 1, "0 earlier lines dropped, 1 line cut"),
        (1, 2, "1 earlier line dropped, 2 lines cut"),
        (2, 1, "2 earlier lines dropped, 1 line cut"),
        (2, 2, "2 earlier lines dropped, 2 lines cut"),
    ] {
        let mut app = app();
        // A run adds a start, an end and a report line to its output lines.
        let lines = if dropped > 0 { vectorcraft_engine::script::console::MAX_CONSOLE_LINES - 3 + dropped } else { cut + 1 };
        let source = format!("var big = 'x'; while (big.length < 5000) big += big; for (var i = 0; i < {lines}; i++) $.writeln(i < {cut} ? big : i)");
        run(&mut app, &source);
        let c = app.session.script_console();
        assert_eq!((c.dropped_lines(), c.truncated_lines()), (dropped as u64, cut as u64));
        app.run("ui.copyScriptConsole", json!({})).unwrap();
        let all = app.clipboard_out.take().unwrap();
        assert_eq!(all.lines().next(), Some(want));
        assert!(painted_text(&mut app, show).contains(want));
    }
}
