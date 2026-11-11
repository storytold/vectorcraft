//! The Script Console: `$.write` and `$.writeln`, `vc.log` lines, the bounded store of a session
//! and of a run, errors with their place, the end and report lines, and the diagnostics a run's
//! report holds.

use serde_json::{Value, json};

use crate::script::console::{CUT_MARK, MAX_CONSOLE_BYTES, MAX_CONSOLE_LINES, MAX_LINE_BYTES, MAX_NAME_BYTES};
use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

/// The session console's `(run, kind, text)` lines.
fn lines(s: &mut Session) -> Vec<(u64, String, String)> {
    let c = s.execute("script.console", &json!({})).unwrap();
    c["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["run"].as_u64().unwrap(), l["kind"].as_str().unwrap().to_string(), l["text"].as_str().unwrap().to_string()))
        .collect()
}

fn texts(s: &mut Session, kind: &str) -> Vec<String> {
    lines(s).into_iter().filter(|(_, k, _)| k == kind).map(|(_, _, t)| t).collect()
}

#[test]
fn write_and_writeln_join_their_values_and_break_lines() {
    let mut s = Session::new();
    run(&mut s, "$.write('a', 1, null, undefined, 1/0); $.write('b\\r\\nc\\rd\\n'); $.writeln(); $.write('x\\r'); $.write('\\ny'); $.writeln([1, 2], {}); $.write('p\\r'); $.write(''); $.write('\\nq'); $.writeln(); $.write('\\0\\t\\u2028\\x7f')").unwrap();
    let want = ["a1nullundefinedInfinityb", "c", "d", "", "x", "y1,2[object Object]", "p", "q", "\u{2400}\u{2409}\u{2424}\u{2421}"];
    assert_eq!(texts(&mut s, "output"), want);
    let report = s.script_report().unwrap();
    assert_eq!(report["console"]["lines"], json!(want), "the run's own console holds the same output");
    assert_eq!(report["hostCallCounts"]["consoleWrites"], 11);
}

#[test]
fn an_endless_writeln_stays_within_the_line_limit_and_counts_what_it_dropped() {
    let mut s = Session::new();
    let e = s.execute("script.run", &json!({"source": "while (1) $.writeln('x')", "name": "t.js", "maxFuel": 2_000_000})).unwrap_err();
    assert!(e.to_string().ends_with("(limit)"), "{e}");
    let c = s.execute("script.console", &json!({})).unwrap();
    let kept = c["lines"].as_array().unwrap().len();
    assert_eq!(kept, MAX_CONSOLE_LINES);
    let report = s.script_report().unwrap().clone();
    let writes = report["hostCallCounts"]["consoleWrites"].as_u64().unwrap();
    assert!(writes > MAX_CONSOLE_LINES as u64 * 2, "{writes} writes");
    // The session console holds a start line, the output, an error, an end and a report line.
    assert_eq!(c["droppedLines"].as_u64().unwrap(), writes + 4 - MAX_CONSOLE_LINES as u64);
    assert_eq!(report["console"]["lines"].as_array().unwrap().len(), MAX_CONSOLE_LINES);
    assert_eq!(report["console"]["droppedLines"].as_u64().unwrap(), writes + 1 - MAX_CONSOLE_LINES as u64);
    assert!(report["fuelUsed"].as_u64().unwrap() > 1_900_000);
    assert_eq!(report["hostCalls"].as_u64().unwrap(), writes);
    let tail = lines(&mut s);
    let ends: Vec<&str> = tail.iter().rev().take(3).map(|(_, k, t)| if k == "error" { "error" } else { t.as_str() }).collect();
    assert_eq!(ends[1], "ended: fuel");
    assert_eq!(ends[2], "error", "{tail:?}");
}

#[test]
fn long_lines_stay_within_the_byte_limit() {
    let mut s = Session::new();
    let source = "var l = new Array(4000).join('y'); for (var i = 0; i < 400; i++) $.writeln(l)";
    run(&mut s, source).unwrap();
    let c = s.execute("script.console", &json!({})).unwrap();
    let bytes: usize = c["lines"].as_array().unwrap().iter().map(|l| l["text"].as_str().unwrap().len()).sum();
    assert!(bytes <= MAX_CONSOLE_BYTES, "{bytes}");
    let kept = c["lines"].as_array().unwrap().len();
    assert!(kept < 400 && kept > 200, "{kept} lines");
    assert!(c["droppedLines"].as_u64().unwrap() > 100);
}

#[test]
fn a_huge_line_is_cut_at_a_character_boundary_and_counted() {
    let mut s = Session::new();
    run(&mut s, "$.writeln(new Array(5000).join('é')); $.write(new Array(3000).join('a')); $.write(new Array(3000).join('b')); $.writeln('c'); $.writeln('ok')")
        .unwrap();
    let out = texts(&mut s, "output");
    assert_eq!(out.len(), 3);
    for line in &out[..2] {
        assert!(line.len() <= MAX_LINE_BYTES && line.len() > MAX_LINE_BYTES - 8, "{}", line.len());
        assert!(line.ends_with(CUT_MARK));
    }
    assert!(!out[1].contains('c'), "the rest of a cut line is left out until it ends");
    assert_eq!(out[2], "ok");
    let c = s.execute("script.console", &json!({})).unwrap();
    assert_eq!(c["truncatedLines"], 2);
    assert_eq!(s.script_report().unwrap()["console"]["truncatedLines"], 2);
}

#[test]
fn a_long_script_name_is_cut_in_every_line_and_shows_its_control_characters() {
    let mut s = Session::new();
    let name = format!("a\nb{}", "é".repeat(100_000));
    s.execute("script.run", &json!({"source": "$.writeln('x'); $.writeln('y')", "name": name})).unwrap();
    let c = s.execute("script.console", &json!({})).unwrap();
    let names: Vec<&str> = c["lines"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 5);
    for n in names {
        assert!(n.len() <= MAX_NAME_BYTES && n.len() > MAX_NAME_BYTES - 8, "{}", n.len());
        assert!(n.starts_with("a\u{240a}b") && n.ends_with(CUT_MARK), "{n}");
    }
    assert_eq!(s.script_report().unwrap()["name"].as_str().map(str::len), Some(name.len()), "the report keeps the name given");
}

#[test]
fn an_uncaught_error_shows_the_script_its_line_and_column() {
    let mut s = Session::new();
    run(&mut s, "$.write('half');\n\n  throw new Error('boom\\nsecond');").unwrap_err();
    let all = lines(&mut s);
    let kinds: Vec<&str> = all.iter().map(|(_, k, _)| k.as_str()).collect();
    assert_eq!(kinds, ["start", "output", "error", "error", "end", "report"]);
    assert_eq!(all[1].2, "half", "a line left unended still shows");
    assert_eq!(all[2].2, "t.js:3:3: Error: boom");
    assert_eq!(all[3].2, "second");
    assert_eq!(all[4].2, "ended: error");
    let report = s.script_report().unwrap();
    assert_eq!(report["console"]["lines"], json!(["half", "t.js:3:3: Error: boom", "second"]));
    let mut s = Session::new();
    run(&mut s, "var = 1").unwrap_err();
    assert!(texts(&mut s, "error")[0].starts_with("t.js:1:"), "a syntax error has its place too");
    let mut s = Session::new();
    s.script_frames = true;
    run(&mut s, "while (true) {}").unwrap();
    s.script_abandon();
    assert_eq!(texts(&mut s, "error"), ["t.js: location unavailable: limit reached: Interrupted"]);
}

#[test]
fn the_result_and_the_report_agree_and_count_each_kind_of_host_call() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let source = "vc.run('shape.rectangle', {x:0,y:0,width:5,height:5}); vc.run('shape.rectangle', {x:9,y:0,width:5,height:5}); \
                  try { vc.run('no.such', {}); } catch (e) {} var d = vc.document; var c = vc.commands; vc.log({a: 1}); vc.log('two\\nlines'); $.writeln('w');";
    let r = run(&mut s, source).unwrap();
    assert_eq!(&r, s.script_report().unwrap());
    assert_eq!(r["hostCallCounts"], json!({"vcRun": 3, "documentReads": 1, "commandListReads": 1, "vcLog": 2, "consoleWrites": 1}));
    assert_eq!(r["timingAvailable"], true);
    assert!(r["runTimeMs"].is_u64());
    let slow = r["slowestCommands"].as_array().unwrap();
    assert_eq!(slow.len(), 1, "an unknown command is not timed: {slow:?}");
    assert_eq!((slow[0]["id"].as_str(), slow[0]["calls"].as_u64()), (Some("shape.rectangle"), Some(2)));
    assert!(slow[0]["maxMs"].as_u64() <= slow[0]["totalMs"].as_u64());
    assert_eq!(r["console"]["lines"], json!(["{\"a\":1}", "two", "lines", "w"]));
    assert_eq!(texts(&mut s, "log"), ["{\"a\":1}", "two", "lines"]);
    assert_eq!(texts(&mut s, "end"), ["ended: finished"]);
    assert!(texts(&mut s, "report")[0].contains("host calls; slowest: shape.rectangle ×2"), "{:?}", texts(&mut s, "report"));
    assert_eq!(r["log"], json!([{"a": 1}, "two\nlines"]), "the log keeps its values");
}

#[test]
fn a_stopped_run_reports_its_diagnostics_and_its_unended_line() {
    let mut s = Session::new();
    s.script_frames = true;
    run(&mut s, "$.write('so far'); vc.log(1); while (true) {}").unwrap();
    assert!(s.script_pump().is_none());
    assert_eq!(texts(&mut s, "log"), ["1"], "a line shows while the run goes on");
    s.execute("script.stop", &json!({})).unwrap();
    assert!(s.script_pump().unwrap().is_err());
    let report = s.script_report().unwrap();
    assert_eq!(report["kind"], "stopped");
    for key in ["runTimeMs", "hostCallCounts", "slowestCommands", "console", "fuelUsed", "hostCalls", "timingAvailable"] {
        assert!(!report[key].is_null(), "{key}: {report}");
    }
    assert_eq!(report["hostCalls"], 2);
    assert_eq!(texts(&mut s, "output"), ["so far"]);
    assert_eq!(texts(&mut s, "end"), ["ended: stopped"]);
}

/// A report line counts one host call in the singular, and gives a run without a clock as "time
/// unavailable".
#[test]
fn a_report_line_counts_one_host_call_in_the_singular_and_says_when_time_is_unavailable() {
    use crate::script::console::{Entry, Slow};
    let report = |ms: Option<u64>, host_calls: u64| Entry::Report { ms, host_calls, slowest: vec![] }.text();
    assert_eq!(report(Some(3), 0), "3 ms, 0 host calls");
    assert_eq!(report(Some(3), 1), "3 ms, 1 host call");
    assert_eq!(report(Some(0), 2), "0 ms, 2 host calls");
    assert_eq!(report(None, 0), "time unavailable, 0 host calls");
    assert_eq!(report(None, 1), "time unavailable, 1 host call");
    assert_eq!(report(None, 2), "time unavailable, 2 host calls");
    let slowest = vec![Slow { id: "a.b".into(), calls: 1, total_ms: 4, max_ms: 4 }];
    assert_eq!(Entry::Report { ms: None, host_calls: 1, slowest }.text(), "time unavailable, 1 host call; slowest: a.b ×1 4 ms");
}

/// A run whose job is gone still ends its console entry, once, with its own number and name and no
/// report line: as stopped when it is abandoned, as an internal fault when a slice finds it gone.
/// The session can start the next run.
#[test]
fn a_run_whose_job_is_gone_still_ends_its_console_entry() {
    for abandon in [true, false] {
        let mut s = Session::new();
        s.script_frames = true;
        run(&mut s, "$.writeln('one'); while (true) {}").unwrap();
        assert!(s.script_pump().is_none());
        assert!(script::forget_job(&s));
        if abandon {
            assert!(s.script_abandon());
        } else {
            let e = s.script_pump().unwrap().unwrap_err();
            assert_eq!(e.to_string(), "the script's run is no longer available");
        }
        assert!(!s.script_running());
        let all = lines(&mut s);
        let kinds: Vec<&str> = all.iter().map(|(_, k, _)| k.as_str()).collect();
        assert_eq!(kinds, ["start", "output", "end"], "{all:?}");
        assert_eq!(all[2].2, if abandon { "ended: stopped" } else { "ended: internal" });
        let c = s.execute("script.console", &json!({})).unwrap();
        assert_eq!((c["lines"][2]["run"].as_u64(), c["lines"][2]["name"].as_str()), (Some(1), Some("t.js")));
        assert!(!s.script_abandon() && s.script_pump().is_none(), "nothing is left to end");
        assert_eq!(lines(&mut s).len(), 3, "the entry ends once");
        s.script_frames = false;
        run(&mut s, "$.writeln('two')").unwrap();
        assert_eq!(texts(&mut s, "end").last().map(String::as_str), Some("ended: finished"), "the next run starts");
        assert_eq!(lines(&mut s).last().map(|l| l.0), Some(2));
    }
}

#[test]
fn runs_are_numbered_a_busy_run_adds_nothing_and_clear_keeps_the_report() {
    let mut s = Session::new();
    for _ in 0..3 {
        run(&mut s, "for (var i = 0; i < 1000; i++) $.writeln(i)").unwrap();
    }
    let all = lines(&mut s);
    assert_eq!(all.len(), MAX_CONSOLE_LINES);
    assert_eq!(all.last().map(|l| l.0), Some(3));
    s.script_frames = true;
    run(&mut s, "$.writeln('four'); while (true) {}").unwrap();
    assert!(s.script_pump().is_none());
    let before = lines(&mut s);
    assert!(matches!(run(&mut s, "$.writeln('five')"), Err(EngineError::Busy)));
    assert_eq!(lines(&mut s), before, "a refused run adds no line");
    assert_eq!(s.execute("script.console.clear", &json!({})).unwrap()["cleared"], MAX_CONSOLE_LINES);
    let c = s.execute("script.console", &json!({})).unwrap();
    assert_eq!((c["lines"].as_array().map(Vec::len), c["droppedLines"].as_u64()), (Some(0), Some(0)));
    s.execute("script.stop", &json!({})).unwrap();
    assert!(s.script_pump().unwrap().is_err());
    let report = s.script_report().unwrap();
    assert_eq!(report["console"]["lines"][0], "four", "a clear leaves the running script's own console");
    assert_eq!(texts(&mut s, "end"), ["ended: stopped"], "the run goes on writing after a clear");
    assert_eq!(lines(&mut s)[0].0, 4);
    let mut other = Session::new();
    assert!(lines(&mut other).is_empty(), "each session has its own console");
}

#[test]
fn the_slowest_commands_are_the_longest_in_all_then_by_id() {
    let times = std::collections::HashMap::from([
        ("a.one", (1, 5_000_000, 5_000_000)),
        ("b.two", (3, 9_000_000, 4_000_000)),
        ("c.three", (1, 5_000_000, 5_000_000)),
        ("d", (1, 1, 1)),
        ("e", (1, 2, 2)),
        ("f", (1, 3, 3)),
    ]);
    let slow = script::slowest(&times);
    let ids: Vec<&str> = slow.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["b.two", "a.one", "c.three", "f", "e"]);
    assert_eq!((slow[0].calls, slow[0].total_ms, slow[0].max_ms), (3, 9, 4));
}
