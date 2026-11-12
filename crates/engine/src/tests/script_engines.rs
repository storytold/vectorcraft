//! `#targetengine`: named engines whose globals last between the runs that name them, their names
//! and places, their limits, `script.engines` and `script.engine.drop`.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::script::{PANIC_IN_SLICE, TEST_ENGINE_HEAP, TEST_KEPT_MAP_BYTES, engines_on_this_thread, releases_kept_for};
use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

fn log(s: &mut Session, source: &str) -> Value {
    run(s, source).unwrap()["log"].clone()
}

/// A session with one document holding one rectangle.
fn one_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s
}

fn objects(s: &Session) -> usize {
    s.documents()[0].doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

fn names(s: &mut Session) -> Vec<String> {
    let r = s.execute("script.engines", &json!({})).unwrap();
    r["engines"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
}

const RECT: &str = "{x: 100, y: 0, width: 5, height: 5}";

#[test]
fn a_named_engine_keeps_its_globals_and_functions_between_runs() {
    let mut s = Session::new();
    assert_eq!(log(&mut s, "#targetengine \"session\"\nvar n = 41;\nfunction f() { return n + 1; }\nvc.log($.engineName);"), json!(["session"]));
    assert_eq!(log(&mut s, "//@targetengine session\nvc.log(f()); n = 1;"), json!([42]));
    assert_eq!(log(&mut s, "#targetengine 'session'\nvc.log(n);"), json!([1]), "single quotes name the same engine");
    assert_eq!(log(&mut s, "#targetengine Session\nvc.log(typeof n);"), json!(["undefined"]), "names are matched as written");
    assert_eq!(log(&mut s, "#targetengine other\nvc.log(typeof f);"), json!(["undefined"]));
    assert_eq!(log(&mut s, "vc.log(typeof n, $.engineName);\nvc.log($.engineName);"), json!(["undefined", "VectorCraft"]));
    // `main` is new for each run, as a script without the line is.
    log(&mut s, "#targetengine main\nvar m = 1;");
    assert_eq!(log(&mut s, "#targetengine main\nvc.log(typeof m, $.engineName);\nvc.log($.engineName);"), json!(["undefined", "main"]));
    assert_eq!(names(&mut s), ["session", "Session", "other"]);
    assert_eq!(log(&mut s, "#targetengine \"Ünï cödé 1\"\nvc.log($.engineName);"), json!(["Ünï cödé 1"]));
    let r = s.execute("script.run", &json!({"source": "#targetengine session\n$.engineName = 'x'; vc.log($.engineName);"})).unwrap();
    assert_eq!(r["log"], json!(["session"]), "$.engineName is read-only");
}

#[test]
fn engine_names_and_places_are_checked_before_anything_runs() {
    let mut s = Session::new();
    let long = "a".repeat(65);
    let control = "#targetengine \"a\u{1}b\"";
    let cases: Vec<(String, &str, u64)> = vec![
        ("#targetengine".into(), "#targetengine needs a name", 1),
        ("#targetengine \"\"".into(), "#targetengine needs a name", 1),
        ("#targetengine a b".into(), "#targetengine needs a name of letters, digits and _", 1),
        ("#targetengine 9a".into(), "#targetengine needs a name of letters, digits and _", 1),
        ("#targetengine \"a\" \"b\"".into(), "#targetengine takes one name", 1),
        (format!("#targetengine \"{long}\""), "#targetengine needs a name of at most 64 bytes", 1),
        (format!("#targetengine {long}"), "#targetengine needs a name of at most 64 bytes", 1),
        (control.into(), "#targetengine needs a name without control characters", 1),
        ("#targetengine a\n//@targetengine a".into(), "the script has more than one //@targetengine line", 2),
        ("var x;\n#targetengine a".into(), "#targetengine must come before the script's code and #include lines", 2),
        ("/* a */ 1;\n#targetengine a".into(), "#targetengine must come before the script's code and #include lines", 2),
    ];
    for (source, message, line) in &cases {
        let e = run(&mut s, &format!("{source}\nvc.log(1);")).unwrap_err().to_string();
        assert!(e.contains(message), "{source}: {e}");
        assert_eq!(s.script_report().unwrap()["line"], *line, "{source}");
        assert_eq!(s.script_report().unwrap()["log"], json!([]), "{source}");
    }
    assert!(names(&mut s).is_empty(), "no failed line made an engine");
    let ok = format!(
        "// a comment\n/* and\n another */\n#target vectorcraft\n#script \"Named\"\n  #targetengine \"{}\"\nvc.log($.engineName.length);",
        "b".repeat(64)
    );
    assert_eq!(log(&mut s, &ok), json!([64]));
    // A script that does not start: an engine it would make is not kept.
    assert!(run(&mut s, "#targetengine broken\nvar = 1;").is_err());
    assert_eq!(names(&mut s), ["b".repeat(64)]);
}

#[test]
fn only_the_script_itself_names_its_engine() {
    let base: PathBuf = std::env::temp_dir().join(format!("vc-script-engines-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("lib.jsxinc"), "#targetengine lib\nvar a = 1;").unwrap();
    let main = base.join("main.jsx");
    let mut s = Session::new();
    for text in ["#include \"lib.jsxinc\"\n", "$.evalFile('lib.jsxinc');\n"] {
        std::fs::write(&main, text).unwrap();
        let e = s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap_err().to_string();
        assert!(e.contains("#targetengine may only be in the script itself, not in a file it runs"), "{text}: {e}");
        assert!(s.script_report().unwrap()["file"].as_str().unwrap().ends_with("lib.jsxinc"), "{text}");
    }
    std::fs::write(base.join("ok.jsxinc"), "var a = 1;").unwrap();
    std::fs::write(&main, "#include \"ok.jsxinc\"\n#targetengine e\n").unwrap();
    let e = s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("#targetengine must come before the script's code and #include lines"), "{e}");
    assert_eq!(s.script_report().unwrap()["line"], 2);
    // An error in a file $.evalFile runs is at that file's place in a kept engine's later runs too.
    std::fs::write(base.join("bad.jsxinc"), "var b;\nthrow new Error('here');").unwrap();
    std::fs::write(&main, "#targetengine e\n$.evalFile('ok.jsxinc');").unwrap();
    s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap();
    std::fs::write(&main, "#targetengine e\n$.evalFile('bad.jsxinc');").unwrap();
    assert!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).is_err());
    let report = s.script_report().unwrap().clone();
    assert!(report["file"].as_str().is_some_and(|f| f.ends_with("bad.jsxinc")), "{report}");
    assert_eq!(report["line"], 2, "{report}");
    assert_eq!(names(&mut s), ["e"]);
    let _ = std::fs::remove_dir_all(base);
    // An error in a function an earlier run defined is at that run's line; a script given as text
    // is named by its run's name.
    run(&mut s, "#targetengine e\n\n\n\nfunction bad() { throw new Error('kept'); }").unwrap();
    let e = run(&mut s, "#targetengine e\nbad();").unwrap_err().to_string();
    let report = s.script_report().unwrap().clone();
    assert_eq!((report["name"].as_str(), report["file"].as_str(), report["line"].as_u64()), (Some("t.js"), Some("t.js"), Some(5)), "{report}");
    let column = report["column"].as_u64().unwrap();
    assert_eq!(error_lines(&report), [format!("t.js: t.js:5:{column}: Error: kept")]);
    assert!(e.contains(&format!("line 5, column {column} of t.js")), "{e}");
}

/// The console error lines of a run's report.
fn error_lines(report: &Value) -> Vec<String> {
    let lines = report["console"]["lines"].as_array().unwrap();
    lines.iter().filter_map(Value::as_str).filter(|l| l.contains("Error: ")).map(str::to_string).collect()
}

/// The file, line and column of the last run's error.
fn place(s: &Session) -> (Option<String>, u64, u64) {
    let r = s.script_report().unwrap();
    (r["file"].as_str().map(str::to_string), r["line"].as_u64().unwrap(), r["column"].as_u64().unwrap())
}

/// An error in code an earlier run of a named engine compiled is at the place of that code: in that
/// run's script, a file it included or a file `$.evalFile` gave, while the report and the console
/// line keep the calling run's name.
#[test]
fn an_error_in_code_an_earlier_run_kept_is_at_that_runs_file_and_line() {
    let base: PathBuf = std::env::temp_dir().join(format!("vc-script-engines-places-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("inc.jsxinc"), "\nfunction fromInclude() { throw new Error('inc'); }").unwrap();
    std::fs::write(base.join("ev.jsxinc"), "\n\n\nfunction fromEval() { throw new Error('ev'); }").unwrap();
    let def = base.join("def.jsx");
    let text = "#targetengine e\n#include \"inc.jsxinc\"\n$.evalFile('ev.jsxinc');\nfunction fromMain() {\n  throw new Error('main');\n}\n\
                function spin() {\n  while (true) {}\n}\nfunction badEval() {\n  eval('var = 1');\n}";
    std::fs::write(&def, text).unwrap();
    let mut s = Session::new();
    s.execute("script.run", &json!({"path": def.to_string_lossy()})).unwrap();
    let call = |s: &mut Session, code: &str| {
        let e = s.execute("script.run", &json!({"source": format!("#targetengine e\n\n{code}"), "name": "caller.js"}));
        assert!(e.is_err(), "{code}");
    };
    let shown = def.to_string_lossy().to_string();
    let ends = |p: &(Option<String>, u64, u64), file: &str, line: u64| p.0.as_deref().is_some_and(|f| f.ends_with(file)) && p.1 == line && p.2 > 0;
    call(&mut s, "fromMain();");
    let p = place(&s);
    assert!(ends(&p, "def.jsx", 5), "{p:?}");
    let report = s.script_report().unwrap().clone();
    assert_eq!(report["name"], "caller.js");
    assert_eq!(error_lines(&report), [format!("caller.js: {}:5:{}: Error: main", p.0.as_deref().unwrap(), p.2)]);
    call(&mut s, "fromInclude();");
    assert!(ends(&place(&s), "inc.jsxinc", 2), "{:?}", place(&s));
    call(&mut s, "fromEval();");
    assert!(ends(&place(&s), "ev.jsxinc", 4), "{:?}", place(&s));
    call(&mut s, "badEval();");
    assert!(ends(&place(&s), "def.jsx", 11), "a syntax error eval raises is at its call: {:?}", place(&s));
    let e = s.execute("script.run", &json!({"source": "#targetengine e\nspin();", "name": "caller.js", "maxFuel": 100_000}));
    assert!(e.is_err());
    assert_eq!(s.script_report().unwrap()["kind"], "limit");
    assert!(ends(&place(&s), "def.jsx", 8), "the fuel ran out in spin: {:?}", place(&s));
    // A caught error thrown again is where that throw is, in the calling run.
    call(&mut s, "try { fromMain(); } catch (x) {\n  throw x;\n}");
    assert_eq!(place(&s), (None, 4, 3));
    // A run that fails before it starts compiles nothing, and the kept places stay.
    assert!(run(&mut s, "#targetengine e\n#include \"missing.jsxinc\"").is_err());
    call(&mut s, "fromMain();");
    assert!(ends(&place(&s), "def.jsx", 5), "{:?}", place(&s));
    assert!(place(&s).0.is_some_and(|f| f == shown));
    // A dropped engine takes its places with it: the new one has its own.
    s.execute("script.engine.drop", &json!({"name": "e"})).unwrap();
    run(&mut s, "#targetengine e\nfunction fromMain() { throw new Error('new'); }").unwrap();
    call(&mut s, "fromMain();");
    assert_eq!(place(&s).0.as_deref(), Some("t.js"));
    assert_eq!(place(&s).1, 2);
    // The name stands for the earlier script as that run's report gave it, not as console lines
    // show it.
    s.execute("script.run", &json!({"source": "#targetengine e\nfunction named() { throw 1; }", "name": "a\u{1}b.js"})).unwrap();
    call(&mut s, "named();");
    assert_eq!(place(&s).0.as_deref(), Some("a\u{1}b.js"));
    let _ = std::fs::remove_dir_all(base);
}

/// A named engine keeps the places of its earlier runs' code within its budget, the oldest going
/// first, and the code still runs once its place is gone.
#[test]
fn an_engine_keeps_the_newest_places_within_its_budget() {
    let mut s = Session::new();
    run(&mut s, "#targetengine e\nfunction a() { throw new Error('a'); }").unwrap();
    TEST_KEPT_MAP_BYTES.with(|b| b.set(Some(300)));
    run(&mut s, "#targetengine e\n\nfunction b() { throw new Error('b'); }").unwrap();
    for _ in 0..20 {
        run(&mut s, "#targetengine e\nvar x = 1;").unwrap();
    }
    run(&mut s, "#targetengine e\n\n\nfunction c() { throw new Error('c'); }").unwrap();
    // Each run's own map takes the room of the one before, so c is called first.
    assert!(run(&mut s, "#targetengine e\nc();").is_err());
    let c_place = place(&s);
    let a = run(&mut s, "#targetengine e\na();").map(drop);
    let a_place = place(&s);
    TEST_KEPT_MAP_BYTES.with(|b| b.set(Some(1)));
    run(&mut s, "#targetengine e\n\n\n\nfunction d() { throw new Error('d'); }").unwrap();
    assert!(run(&mut s, "#targetengine e\nd();").is_err());
    let d_place = place(&s);
    TEST_KEPT_MAP_BYTES.with(|b| b.set(None));
    assert!(a.is_err());
    assert_eq!((a_place.0, a_place.1, a_place.2), (None, 0, 0), "a's place went to make room");
    assert_eq!((c_place.0.as_deref(), c_place.1), (Some("t.js"), 4));
    assert_eq!((d_place.0, d_place.1, d_place.2), (None, 0, 0), "a map larger than the budget is not kept");
}

/// A run of a named engine whose script does not compile defines no code, so it keeps no place
/// and pushes out none of the places kept before. An included file without a preprocessor line is
/// first parsed with the script it joins, so its syntax error ends the run as it starts.
#[test]
fn a_run_that_does_not_compile_keeps_no_place() {
    let base: PathBuf = std::env::temp_dir().join(format!("vc-script-engines-uncompiled-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    // A kept place counts the names of the files the run read: four places of the failed run's size
    // would not fit in the budget.
    let mut text = String::from("#targetengine e\n");
    for k in 0..10 {
        std::fs::write(base.join(format!("ok{k}.jsxinc")), "var ok = 1;").unwrap();
        text.push_str(&format!("#include \"ok{k}.jsxinc\"\n"));
    }
    std::fs::write(base.join("bad.jsxinc"), "var = ;").unwrap();
    text.push_str("#include \"bad.jsxinc\"");
    let bad = base.join("bad.jsx");
    std::fs::write(&bad, text).unwrap();
    let mut s = Session::new();
    run(&mut s, "#targetengine e\nvar x = 1;").unwrap();
    TEST_KEPT_MAP_BYTES.with(|b| b.set(Some(3000)));
    run(&mut s, "#targetengine e\n\n\nfunction c() { throw new Error('c'); }").unwrap();
    let mut kinds = vec![];
    for _ in 0..5 {
        assert!(s.execute("script.run", &json!({"path": bad.to_string_lossy()})).is_err());
        kinds.push(s.script_report().unwrap()["kind"].clone());
    }
    assert!(run(&mut s, "#targetengine e\nc();").is_err());
    let c_place = place(&s);
    TEST_KEPT_MAP_BYTES.with(|b| b.set(None));
    let _ = std::fs::remove_dir_all(base);
    assert!(kinds.iter().all(|k| k == "syntax"), "{kinds:?}");
    assert_eq!((c_place.0.as_deref(), c_place.1), (Some("t.js"), 4));
}

/// A `$.evalFile` an earlier run kept reads files the way the run that calls it does: relative to
/// that run's script, and as `$.evalFile` does in a run given as text.
#[test]
fn a_kept_eval_file_reads_in_the_calling_run_scope() {
    let base: PathBuf = std::env::temp_dir().join(format!("vc-script-engines-scope-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for dir in ["a", "b"] {
        std::fs::create_dir_all(base.join(dir)).unwrap();
        std::fs::write(base.join(dir).join("lib.jsxinc"), format!("'{dir}'")).unwrap();
    }
    let mut s = Session::new();
    let a = base.join("a").join("main.jsx");
    std::fs::write(&a, "#targetengine e\nvar ev = $.evalFile;\nvc.log(ev('lib.jsxinc'));").unwrap();
    assert_eq!(s.execute("script.run", &json!({"path": a.to_string_lossy()})).unwrap()["log"], json!(["a"]));
    let b = base.join("b").join("main.jsx");
    std::fs::write(&b, "#targetengine e\nvc.log(ev('lib.jsxinc'));").unwrap();
    assert_eq!(s.execute("script.run", &json!({"path": b.to_string_lossy()})).unwrap()["log"], json!(["b"]), "the kept function reads beside b");
    let kept = run(&mut s, "#targetengine e\nvc.log(ev('lib.jsxinc'));").map_err(|e| e.to_string());
    let direct = run(&mut s, "vc.log($.evalFile('lib.jsxinc'));").map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(base);
    assert!(direct.is_err(), "{direct:?}");
    assert_eq!(kept.unwrap_err().split(": ").last(), direct.unwrap_err().split(": ").last(), "a run given as text reads no file beside a");
}

/// `File` and `Folder` objects an earlier run kept read in the folder of the run that calls them,
/// `Folder.current` starts at each run's script, and a file an earlier run left open is not open:
/// it takes the encoding it is given, as a closed file does.
#[test]
fn kept_file_objects_read_in_the_calling_run_folder() {
    let base: PathBuf = std::env::temp_dir().join(format!("vc-script-engines-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for dir in ["a", "b"] {
        std::fs::create_dir_all(base.join(dir)).unwrap();
        std::fs::write(base.join(dir).join("data.txt"), dir.to_uppercase()).unwrap();
    }
    let mut s = Session::new();
    let a = base.join("a").join("main.jsx");
    std::fs::write(&a, "#targetengine e\nvar keep = new File('data.txt');\nkeep.open('r');\nvc.log([Folder.current.name, keep.read(1)]);").unwrap();
    assert_eq!(s.execute("script.run", &json!({"path": a.to_string_lossy()})).unwrap()["log"], json!([["a", "A"]]));
    let b = base.join("b").join("main.jsx");
    let text = "#targetengine e\nvar g = new File('data.txt');\ng.open('r');\nvc.log([Folder.current.name, g.read(), keep.read(), keep.error, keep.exists]);\n\
                var before = keep.encoding;\nkeep.encoding = 'BINARY';\nkeep.open('r');\nvc.log([before, keep.encoding]);";
    std::fs::write(&b, text).unwrap();
    let log = s.execute("script.run", &json!({"path": b.to_string_lossy()})).unwrap()["log"].clone();
    let _ = std::fs::remove_dir_all(base);
    assert_eq!(log, json!([["b", "B", "", "Script file is not open", false], ["UTF-8", "BINARY"]]));
}

#[test]
fn a_failed_or_stopped_run_keeps_the_globals_it_set_and_puts_the_documents_back() {
    let mut s = one_doc();
    let undo = s.documents()[0].history.undo.len();
    let e = run(&mut s, &format!("#targetengine e\nvar n = 1;\nvc.run('shape.rectangle', {RECT});\nthrow new Error('boom');"));
    assert!(e.is_err());
    assert_eq!((objects(&s), s.documents()[0].history.undo.len()), (1, undo), "the documents went back");
    assert_eq!(log(&mut s, "#targetengine e\nvc.log(n);"), json!([1]));
    // A stop.
    s.script_frames = true;
    assert_eq!(run(&mut s, &format!("#targetengine e\nn = 10;\nvc.run('shape.rectangle', {RECT});\nwhile (true) {{}}")).unwrap()["running"], true);
    assert!(s.script_pump().is_none());
    assert_eq!(s.execute("script.stop", &json!({})).unwrap()["stopped"], true);
    assert!(s.script_pump().unwrap().is_err());
    s.script_frames = false;
    assert_eq!(objects(&s), 1);
    // An error in another engine leaves this one as it was.
    assert!(run(&mut s, "#targetengine f\nvar n = 99;\nthrow 1;").is_err());
    assert_eq!(log(&mut s, "#targetengine e\nvc.log(n);"), json!([10]));
    // A run that ends well is one undo step.
    run(&mut s, &format!("#targetengine e\nvc.run('shape.rectangle', {RECT});")).unwrap();
    assert_eq!((objects(&s), s.documents()[0].history.undo.len()), (2, undo + 1));
    assert_eq!(s.documents()[0].history.undo.last().unwrap().label, "Run Script");
}

/// A run stopped at a dialog, or left paused, ends where it was: the engine keeps its globals and
/// the next run starts afresh.
#[test]
fn a_run_stopped_at_a_dialog_or_left_paused_ends_where_it_was() {
    let mut s = Session::new();
    s.script_frames = true;
    run(&mut s, "#targetengine e\nvar n = 20;\nfunction f() { var big = new Array(4000001).join('x'); alert('x'); n = big.length; }\nf();").unwrap();
    for _ in 0..1_000 {
        if s.script_dialog().is_some() {
            break;
        }
        assert!(s.script_pump().is_none());
    }
    assert!(s.script_dialog().is_some());
    s.execute("script.stop", &json!({})).unwrap();
    assert!(s.script_pump().unwrap().is_err());
    let heap = s.execute("script.engines", &json!({})).unwrap()["engines"][0]["heapBytes"].as_u64().unwrap();
    assert!(heap < 4 << 20, "the stopped run's frames, and the string they held, are gone: {heap} bytes");
    run(&mut s, "#targetengine e\nn += 1;\nwhile (true) {}").unwrap();
    assert!(s.script_pump().is_none());
    assert!(s.script_abandon());
    s.script_frames = false;
    for _ in 0..3 {
        assert_eq!(log(&mut s, "#targetengine e\nvc.log(n);"), json!([21]));
    }
}

#[test]
fn a_kept_host_function_acts_for_the_run_that_calls_it() {
    let mut s = one_doc();
    run(&mut s, "#targetengine e\nvar oldRun = vc.run, oldVc = vc, oldWrite = $.writeln, oldAlert = alert;").unwrap();
    // The functions an earlier run kept work in this run's transaction, which goes back.
    let e = run(&mut s, &format!("#targetengine e\noldRun('shape.rectangle', {RECT});\noldVc.log('kept');\nthrow new Error('x');"));
    assert!(e.is_err());
    assert_eq!(objects(&s), 1, "the kept vc.run's change went back with the run");
    assert_eq!(s.script_report().unwrap()["log"], json!(["kept"]));
    let r = s
        .execute(
            "script.run",
            &json!({"source": format!("#targetengine e\noldRun('shape.rectangle', {RECT});\noldWrite('out');\noldAlert('hi');"), "answers": [{"kind": "alert", "value": "ok"}]}),
        )
        .unwrap();
    assert_eq!((objects(&s), r["dialogCount"].as_u64()), (2, Some(1)));
    assert!(r["console"]["lines"].as_array().unwrap().iter().any(|l| l == "out"), "{r}");
    assert_eq!(s.documents()[0].history.undo.last().unwrap().label, "Run Script");
    let r = s
        .execute(
            "script.run",
            &json!({"source": "#targetengine e\nvc.log([oldVc.args.n, vc === oldVc, Object.isFrozen(vc.args)]);", "args": {"n": 2}}),
        )
        .unwrap();
    assert_eq!(r["log"], json!([[2, true, true]]), "a kept vc gives the calling run's args, frozen");
}

/// The values a run sets on `$` in a named engine end with that run, as in a run without one, and
/// a `$` an earlier run kept gives the values of the run that reads it.
#[test]
fn the_values_of_the_dollar_object_belong_to_each_run_of_a_named_engine() {
    let mut s = Session::new();
    let read = "vc.log([$.getenv('OS'), $.getenv('LANG'), $.localize, $.flags, $.locale]);";
    let fresh = log(&mut s, read);
    let set = "$.setenv('OS', 'set_here');\n$.setenv('LANG', 'set_here');\n$.localize = true;\n$.locale = 'fr_FR';";
    run(&mut s, &format!("#targetengine e\nvar d = $;\n{set}\n{read}")).unwrap();
    assert_eq!(log(&mut s, &format!("#targetengine e\n{read}")), fresh);
    let os = fresh[0][0].clone();
    assert_eq!(log(&mut s, "#targetengine e\nvc.log([d === $, d.engineName, d.getenv('OS'), d.localize]);"), json!([[true, "e", os, false]]));
    assert_eq!(log(&mut s, "#targetengine e\nd.setenv('OS', 'again');\nvc.log(d.getenv('OS'));"), json!(["again"]));
    assert_eq!(log(&mut s, &format!("#targetengine e\n{read}")), fresh);
}

#[test]
fn an_engine_does_not_keep_a_closed_document() {
    let mut s = one_doc();
    let name = s.documents()[0].title();
    run(&mut s, "#targetengine e\nvar snap = vc.document;\nvar read = function () { return vc.document; };").unwrap();
    let weak = Arc::downgrade(&s.documents()[0].doc);
    s.execute("file.close", &json!({})).unwrap();
    assert!(weak.upgrade().is_none(), "nothing holds the closed document");
    assert_eq!(log(&mut s, "#targetengine e\nvc.log(read());\nvc.log(snap.name);"), json!([null, name]));
}

#[test]
fn a_session_keeps_eight_engines_and_drops_them_on_request() {
    let mut s = Session::new();
    for i in 0..8 {
        run(&mut s, &format!("#targetengine e{i}\nvar v = {i};")).unwrap();
    }
    let e = run(&mut s, "#targetengine e8\nvc.log(1);").unwrap_err().to_string();
    assert!(e.contains("a session keeps at most 8 script engines"), "{e}");
    assert_eq!((s.script_report().unwrap()["kind"].as_str(), s.script_report().unwrap()["log"].clone()), (Some("limit"), json!([])));
    assert_eq!(log(&mut s, "#targetengine e3\nvc.log(v);"), json!([3]), "the eight still run");
    let list = s.execute("script.engines", &json!({})).unwrap();
    assert_eq!((list["limit"].as_u64(), list["heapLimit"].as_u64()), (Some(8), Some(64 << 20)));
    for (i, e) in list["engines"].as_array().unwrap().iter().enumerate() {
        assert_eq!(e["name"], format!("e{i}"));
        assert_eq!(e["running"], false);
        assert!(e["heapBytes"].as_u64().unwrap() > 0, "{e}");
    }
    assert_eq!(s.execute("script.engine.drop", &json!({"name": "e2"})).unwrap(), json!({"dropped": true, "name": "e2"}));
    assert_eq!(names(&mut s), ["e0", "e1", "e3", "e4", "e5", "e6", "e7"]);
    run(&mut s, "#targetengine e8\nvar v = 8;").unwrap();
    assert!(run(&mut s, "#targetengine e2\n1").unwrap_err().to_string().contains("at most 8"));
    s.execute("script.engine.drop", &json!({"name": "e8"})).unwrap();
    assert_eq!(log(&mut s, "#targetengine e2\nvc.log(typeof v);"), json!(["undefined"]), "a dropped engine comes back new");
    for (params, message) in [
        (json!({"name": "main"}), "the main script engine is new for each run"),
        (json!({"name": "nope"}), "there is no script engine named \"nope\""),
        (json!({}), "`name` must be the name of a script engine"),
        (json!({"name": 3}), "`name` must be the name of a script engine"),
    ] {
        let e = s.execute("script.engine.drop", &params).unwrap_err().to_string();
        assert!(e.contains(message), "{params}: {e}");
    }
    // The running script's engine cannot be dropped; another one can.
    s.script_frames = true;
    assert_eq!(run(&mut s, "#targetengine e0\nwhile (true) {}").unwrap()["running"], true);
    assert!(s.script_pump().is_none());
    assert!(matches!(s.execute("script.engine.drop", &json!({"name": "e0"})), Err(EngineError::Busy)));
    let list = s.execute("script.engines", &json!({})).unwrap();
    assert_eq!(list["engines"][0]["running"], true);
    s.execute("script.engine.drop", &json!({"name": "e1"})).unwrap();
    s.execute("script.stop", &json!({})).unwrap();
    assert!(s.script_pump().unwrap().is_err());
    assert_eq!(names(&mut s), ["e0", "e3", "e4", "e5", "e6", "e7", "e2"]);
    // A script cannot drop or list engines itself.
    s.script_frames = false;
    let e = run(&mut s, "vc.run('script.engine.drop', {name: 'e0'});").unwrap_err().to_string();
    assert!(e.contains("a script cannot run `script.engine.drop`"), "{e}");
}

#[test]
fn a_named_engine_has_its_own_heap_limit_and_stays_usable() {
    TEST_ENGINE_HEAP.with(|h| h.set(Some(8 << 20)));
    let mut s = Session::new();
    let big = run(&mut s, "#targetengine big\nvar marker = 'set', keep = [];\nfor (var i = 0; i < 10000000; i++) keep.push('item ' + i);");
    // The next run starts on the nearly full heap even with large `args`, and can free memory.
    let big_args = json!({"big": "x".repeat(1 << 19)});
    let free = json!({"source": "#targetengine big\nkeep = null;\nvc.log(marker);", "name": "t.js", "args": big_args});
    let kept = s.execute("script.run", &free).unwrap()["log"].clone();
    let after = s.execute("script.engines", &json!({})).unwrap();
    TEST_ENGINE_HEAP.with(|h| h.set(None));
    assert!(big.unwrap_err().to_string().contains("Heap"));
    assert_eq!(kept, json!(["set"]), "the globals set before the limit stay, and the engine still runs");
    assert!(after["engines"][0]["heapBytes"].as_u64().unwrap() < 8 << 20, "{after}");
    // A run without #targetengine keeps the larger heap of one run.
    assert_eq!(log(&mut s, "var keep = [];\nfor (var i = 0; i < 300000; i++) keep.push('item ' + i);\nvc.log(keep.length);"), json!([300000]));
}

/// The engines stay on the thread that made them: a session used on another thread starts there
/// with none, and the engines it left, or that a session dropped elsewhere held, are dropped on
/// their own thread when it next uses its engines.
#[test]
fn a_session_moved_to_another_thread_starts_with_no_engines_and_lets_the_old_ones_go() {
    let mut s = Session::new();
    run(&mut s, "#targetengine e\nvar kept = 1;").unwrap();
    assert_eq!(engines_on_this_thread(), 1);
    let (mut s, there, listed, thread) = std::thread::spawn(move || {
        let there = log(&mut s, "#targetengine e\nvc.log(typeof kept);");
        let listed = names(&mut s);
        (s, there, listed, std::thread::current().id())
    })
    .join()
    .unwrap();
    assert_eq!((there, listed), (json!(["undefined"]), vec!["e".to_string()]));
    assert!(!releases_kept_for(thread), "a thread that ended keeps no list of engines to let go");
    // Back here, the first engine has been let go: the run gets a new one.
    assert_eq!(log(&mut s, "#targetengine e\nvc.log(typeof kept);"), json!(["undefined"]));
    assert_eq!(engines_on_this_thread(), 1);
    assert!(!releases_kept_for(thread), "letting go of the ended thread's engines adds nothing for it");
    std::thread::spawn(move || drop(s)).join().unwrap();
    let mut other = Session::new();
    assert!(names(&mut other).is_empty());
    assert_eq!(engines_on_this_thread(), 0, "the engine of the session dropped on another thread is dropped here");
}

#[test]
fn an_internal_error_drops_the_engine() {
    let mut s = Session::new();
    assert_eq!(log(&mut s, "#targetengine p\nvar a = 1;\nvc.log(a);"), json!([1]));
    assert_eq!(log(&mut s, "#targetengine p\nvc.log(a);"), json!([1]));
    s.script_frames = true;
    run(&mut s, "#targetengine p\nwhile (true) {}").unwrap();
    PANIC_IN_SLICE.with(|p| p.set(true));
    let r = s.script_pump().unwrap();
    PANIC_IN_SLICE.with(|p| p.set(false));
    assert!(r.is_err());
    assert!(names(&mut s).is_empty());
    s.script_frames = false;
    assert_eq!(log(&mut s, "#targetengine p\nvc.log(typeof a);"), json!(["undefined"]));
}

/// A run whose job is gone took its engine's realm with it: the engine is dropped, so the next run
/// that names it starts it new and other engines stay.
#[test]
fn a_run_whose_job_is_gone_drops_its_engine() {
    for abandon in [true, false] {
        let mut s = Session::new();
        run(&mut s, "#targetengine kept\nvar k = 1;").unwrap();
        run(&mut s, "#targetengine lost\nvar a = 1;").unwrap();
        s.script_frames = true;
        run(&mut s, "#targetengine lost\nwhile (true) {}").unwrap();
        assert!(s.script_pump().is_none());
        assert!(crate::script::forget_job(&s));
        if abandon {
            assert!(s.script_abandon());
        } else {
            assert!(s.script_pump().unwrap().is_err());
        }
        s.script_frames = false;
        assert_eq!(names(&mut s), ["kept"], "abandon: {abandon}");
        assert_eq!(log(&mut s, "#targetengine lost\nvc.log(typeof a);"), json!(["undefined"]), "abandon: {abandon}");
        assert_eq!(log(&mut s, "#targetengine kept\nvc.log(k);"), json!([1]), "abandon: {abandon}");
    }
}
