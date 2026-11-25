//! The runner of `cargo xtask scripts --pass-rate` (`xtask/src/script_corpus.rs`): the ignored test
//! `row` runs one script of the public script corpus and prints its result on one line, and xtask
//! starts it once for each script, in a process of its own that it stops after its time limit.
//!
//! A row gets a new session with one new document and runs `script.run` with the script's path,
//! 5,000,000 instructions and no dialog answers. Before the session starts, the process confines
//! itself to the automation roots the run's folders give: reads only in the script's repository
//! folder and the run's own folder, and writes only in the run's own folder, which holds the data
//! folder (`Folder.userData`) and, through the temporary-folder variables xtask sets, the run's
//! temporary folder (`Folder.temp`). The other tests check the result line on scripts given as
//! text.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::file_access::{AutomationRoots, confine_process};

/// The instructions each script may run.
const FUEL: u64 = 5_000_000;
/// The most characters of a message a result keeps.
const MAX_MESSAGE_CHARS: usize = 300;

/// A session with one new document.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s
}

/// Runs `script.run` with `params` and gives the result line's JSON: whether the run started (a
/// script `script.run` refuses before it runs has no report), whether it ended well, and the
/// failure's kind, message (at most [`MAX_MESSAGE_CHARS`] characters), line and column, the fuel
/// and host calls it used, its `File.execute` calls, and whether a dialog found no answer.
fn result(s: &mut Session, params: &Value) -> Value {
    let ran = s.execute("script.run", params);
    let report = s.script_report().cloned().unwrap_or(Value::Null);
    let message = report["message"].as_str().map(str::to_owned).or_else(|| ran.as_ref().err().map(ToString::to_string)).unwrap_or_default();
    json!({
        "started": !report.is_null(),
        "ok": ran.is_ok(),
        "kind": report["kind"],
        "message": message.chars().take(MAX_MESSAGE_CHARS).collect::<String>(),
        "line": report["line"],
        "column": report["column"],
        "fuelUsed": report["fuelUsed"],
        "hostCalls": report["hostCalls"],
        "fileExecutes": report["hostCallCounts"]["fileExecutes"],
        "missingAnswer": report["dialogs"].as_array().is_some_and(|d| d.iter().any(|x| x["answer"] == "missing")),
    })
}

fn source(text: &str) -> Value {
    json!({"source": text, "name": "corpus.js", "maxFuel": FUEL, "answers": []})
}

#[test]
#[ignore = "cargo xtask scripts --pass-rate runs it once for each script"]
fn row() {
    let (Some(script), Some(folder), Some(work)) = (
        std::env::var_os("VECTORCRAFT_SCRIPT_CORPUS_ROW"),
        std::env::var_os("VECTORCRAFT_SCRIPT_CORPUS_SOURCE"),
        std::env::var_os("VECTORCRAFT_SCRIPT_CORPUS_WORK"),
    ) else {
        println!("skipped: run by cargo xtask scripts --pass-rate");
        return;
    };
    let (folder, work) = (PathBuf::from(folder), PathBuf::from(work));
    let roots = AutomationRoots::new(&[folder.as_path(), work.as_path()], &[work.as_path()]).unwrap().unwrap();
    assert!(confine_process(roots));
    let mut s = session();
    s.attach_script_data(work.join("data"));
    let script = script.to_string_lossy().into_owned();
    let line = result(&mut s, &json!({"path": script, "maxFuel": FUEL, "answers": []}));
    println!("script-corpus-row {line}");
}

#[test]
fn a_script_that_ends_well_passes() {
    let r = result(&mut session(), &source("var a = [1, 2].join('-');"));
    assert_eq!(r["started"], true);
    assert_eq!(r["ok"], true);
    assert!(r["fuelUsed"].as_u64().unwrap() > 0);
}

#[test]
fn a_failure_gives_its_kind_and_place() {
    let r = result(&mut session(), &source("var a = 1;\nvar = ;"));
    assert_eq!((r["ok"].clone(), r["kind"].clone(), r["line"].clone()), (json!(false), json!("syntax"), json!(2)));
    let r = result(&mut session(), &source("var d = app.activeDocument;"));
    assert_eq!(r["kind"], "thrown");
    assert!(r["message"].as_str().unwrap().starts_with("ReferenceError: app"), "{r}");
    let r = result(&mut session(), &source("while (true) {}"));
    assert_eq!(r["kind"], "limit");
    assert!(r["fuelUsed"].as_u64().unwrap() <= FUEL);
}

#[test]
fn a_dialog_without_an_answer_is_marked() {
    let r = result(&mut session(), &source("alert('hello');"));
    assert_eq!(r["ok"], false);
    assert_eq!(r["missingAnswer"], true);
    let r = result(&mut session(), &source("var n = 1;"));
    assert_eq!(r["missingAnswer"], false);
}

#[test]
fn file_execute_is_counted_and_refused() {
    let r = result(&mut session(), &source("var ok = new File('x.txt').execute(); if (ok) throw new Error('ran');"));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["fileExecutes"], 1);
}

#[test]
fn a_script_refused_before_it_runs_has_not_started() {
    let r = result(&mut session(), &json!({"path": "notes.txt", "maxFuel": FUEL, "answers": []}));
    assert_eq!(r["started"], false);
    assert_eq!(r["ok"], false);
    assert!(r["message"].as_str().unwrap().contains(".js or .jsx"), "{r}");
}

#[test]
fn a_long_message_is_cut() {
    let r = result(&mut session(), &source(&format!("throw new Error('{}');", "x".repeat(1000))));
    assert_eq!(r["message"].as_str().unwrap().chars().count(), MAX_MESSAGE_CHARS);
}
