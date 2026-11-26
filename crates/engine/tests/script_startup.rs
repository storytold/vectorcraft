//! Startup scripts (`script.startup.run`): off by default, the folder's `.jsx` files in order at
//! launch and before a chosen script, Safe Mode and disabled scripts, failures and limits, and the
//! launch marker naming the startup script that runs.
//!
//! A test binary of its own: Safe Mode, the disabled list and the launch marker are process-wide,
//! so these tests run one at a time and no other test sees them.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::safe_mode;
use vectorcraft_testkit::TempDir;

static LOCK: Mutex<()> = Mutex::new(());

/// Safe Mode off, nothing disabled, no launch marker.
fn reset() {
    safe_mode::end_launch();
    safe_mode::set_active(None);
    safe_mode::forget_previous();
    safe_mode::set_disabled(vec![]);
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

/// A new, empty folder for test `tag`, removed with everything in it when the test ends.
fn tmp_dir(tag: &str) -> TempDir {
    TempDir::new(&format!("script-startup-{tag}"))
}

/// Writes `text` to `name` under `dir`, folders included.
fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

/// A session with the Startup Scripts folder `dir` and Run Startup Scripts on.
fn session(dir: &Path) -> Session {
    let mut s = Session::new();
    s.attach_startup_scripts_dir(dir.to_path_buf());
    s.execute("prefs.set", &json!({"key": "runStartupScripts", "value": true})).unwrap();
    s
}

/// The names of the runs the Script Console started, in order.
fn started(s: &Session) -> Vec<String> {
    let lines = s.script_console().to_json();
    lines["lines"].as_array().unwrap().iter().filter(|l| l["kind"] == "start").map(|l| l["name"].as_str().unwrap().to_string()).collect()
}

/// The texts of the Script Console's lines of `kind`.
fn console(s: &Session, kind: &str) -> Vec<String> {
    let lines = s.script_console().to_json();
    lines["lines"].as_array().unwrap().iter().filter(|l| l["kind"] == kind).map(|l| l["text"].as_str().unwrap().to_string()).collect()
}

fn names(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect()
}

#[test]
fn startup_scripts_are_off_by_default_and_run_nothing() {
    let _f = fresh();
    let dir = tmp_dir("off");
    write(&dir, "a.jsx", "vc.log('ran');");
    let mut s = Session::new();
    assert!(!s.prefs.run_startup_scripts, "off by default");
    s.attach_startup_scripts_dir(dir.to_path_buf());
    assert_eq!(s.startup_skip(), Some("off"));
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    assert_eq!(r, json!({"reason": "launch", "skipped": true, "why": "off"}));
    let r = s.execute("script.startup.run", &json!({"reason": "menu", "then": {"source": "vc.log(1);", "name": "chosen"}})).unwrap();
    assert_eq!(r["skipped"], true);
    assert_eq!(r["then"]["ok"], true, "the chosen script still runs");
    assert_eq!(started(&s), ["chosen"], "no startup script ran");
}

#[test]
fn the_folders_jsx_files_run_in_name_order_at_launch_and_before_a_chosen_script() {
    let _f = fresh();
    let dir = tmp_dir("order");
    for name in ["b.jsx", "C.JSX", "a.jsx", ".hidden.jsx", "x.js", "lib.jsxinc", "sub/d.jsx", "notes.txt"] {
        write(&dir, name, "vc.log(1);");
    }
    let mut s = session(&dir);
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    assert_eq!(r["skipped"], false);
    assert_eq!(names(&r["ran"]), ["a.jsx", "b.jsx", "C.JSX"], "by name without regard to case");
    assert!(r["failed"].as_array().unwrap().is_empty());
    assert_eq!(started(&s), ["a.jsx", "b.jsx", "C.JSX"]);
    let r = s.execute("script.startup.run", &json!({"reason": "menu", "then": {"source": "vc.log(2);", "name": "chosen"}})).unwrap();
    assert_eq!(r["then"]["ok"], true);
    assert_eq!(started(&s), ["a.jsx", "b.jsx", "C.JSX", "a.jsx", "b.jsx", "C.JSX", "chosen"], "the startup scripts run first");
}

#[test]
fn a_missing_folder_runs_nothing_and_a_session_without_one_skips() {
    let _f = fresh();
    let dir = tmp_dir("missing");
    let mut s = session(&dir.join("Startup Scripts"));
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    assert_eq!(r["skipped"], false);
    assert!(r["ran"].as_array().unwrap().is_empty() && r["failed"].as_array().unwrap().is_empty());
    assert_eq!(r["unreadable"], false);
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"key": "runStartupScripts", "value": true})).unwrap();
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    assert_eq!(r["why"], "noFolder");
}

#[test]
fn safe_mode_skips_the_startup_scripts_and_still_runs_the_chosen_one() {
    let _f = fresh();
    let dir = tmp_dir("safe");
    write(&dir, "a.jsx", "vc.log(1);");
    let mut s = session(&dir);
    safe_mode::set_active(Some(safe_mode::Source::Flag));
    assert_eq!(s.startup_skip(), Some("safeMode"));
    let r = s.execute("script.startup.run", &json!({"reason": "menu", "then": {"source": "vc.log(2);", "name": "chosen"}})).unwrap();
    assert_eq!(r["why"], "safeMode");
    assert_eq!(started(&s), ["chosen"]);
}

#[test]
fn a_startup_script_the_safe_mode_choice_disabled_does_not_run() {
    let _f = fresh();
    let dir = tmp_dir("disabled");
    let a = write(&dir, "a.jsx", "vc.log(1);");
    write(&dir, "b.jsx", "vc.log(1);");
    let mut s = session(&dir);
    safe_mode::disable(safe_mode::Item::file(a.to_string_lossy()));
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    assert_eq!(names(&r["disabled"]), ["a.jsx"]);
    assert_eq!(names(&r["ran"]), ["b.jsx"]);
}

#[test]
fn a_failing_or_endless_startup_script_is_reported_and_the_next_and_the_chosen_script_run() {
    let _f = fresh();
    let dir = tmp_dir("failing");
    write(&dir, "1 throws.jsx", "throw new Error('broken');");
    write(&dir, "2 endless.jsx", "for (;;) {}");
    write(&dir, "3 syntax.jsx", "var = ;");
    write(&dir, "4 fine.jsx", "vc.log(1);");
    let mut s = session(&dir);
    let then = json!({"source": "vc.log('chosen');", "name": "chosen"});
    let r = s.execute("script.startup.run", &json!({"reason": "menu", "then": then, "maxFuel": 100000})).unwrap();
    let failed = r["failed"].as_array().unwrap();
    let failed_names: Vec<&str> = failed.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(failed_names, ["1 throws.jsx", "2 endless.jsx", "3 syntax.jsx"]);
    assert!(failed[0]["error"].as_str().unwrap().contains("broken"), "{failed:?}");
    assert_eq!(names(&r["ran"]), ["4 fine.jsx"]);
    assert_eq!(r["then"]["ok"], true, "the chosen script runs after them");
    assert!(console(&s, "end").contains(&"ended: fuel".to_string()), "the endless one ran out of instructions");
}

#[test]
fn a_startup_script_larger_than_a_script_may_be_is_refused_unread() {
    let _f = fresh();
    let dir = tmp_dir("large");
    let mut big = "//".to_string();
    big.push_str(&"x".repeat(vectorcraft_engine::script::MAX_SOURCE_BYTES));
    write(&dir, "big.jsx", &big);
    let mut s = session(&dir);
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    assert_eq!(r["failed"], json!([{"name": "big.jsx", "error": vectorcraft_engine::cmd::scripts::TOO_LARGE}]));
    assert!(console(&s, "error").iter().any(|l| l.contains(vectorcraft_engine::cmd::scripts::TOO_LARGE)), "the console names it");
}

#[test]
fn a_startup_script_cannot_start_another_startup_run() {
    let _f = fresh();
    let dir = tmp_dir("nested");
    let text =
        "try { vc.run('script.startup.run', {reason: 'menu', then: {source: 'vc.log(1);'}}); vc.log('nested'); } catch (e) { vc.log('refused'); }";
    write(&dir, "a.jsx", text);
    let mut s = session(&dir);
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    assert_eq!(names(&r["ran"]), ["a.jsx"]);
    assert_eq!(console(&s, "log"), ["refused"]);
    assert_eq!(started(&s), ["a.jsx"]);
}

#[test]
fn bad_parameters_are_refused() {
    let _f = fresh();
    let dir = tmp_dir("params");
    let mut s = session(&dir);
    for p in [
        json!({}),
        json!({"reason": "boot"}),
        json!({"reason": "menu", "then": "x.jsx"}),
        json!({"reason": "launch", "maxFuel": 0}),
        json!({"reason": "launch", "maxFuel": 1.5}),
    ] {
        assert!(s.execute("script.startup.run", &p).is_err(), "{p}");
    }
    // `then` is checked as `script.run` checks its parameters, before any startup script runs.
    write(&dir, "a.jsx", "vc.log(1);");
    let then = json!({"source": "vc.log(2);", "bogus": true});
    let alone = s.execute("script.run", &then).unwrap_err().to_string();
    let e = s.execute("script.startup.run", &json!({"reason": "menu", "then": then})).unwrap_err().to_string();
    assert_eq!(e, alone);
    assert!(started(&s).is_empty(), "no startup script ran");
}

#[test]
fn in_the_app_the_runs_are_queued_and_the_launch_marker_names_the_running_startup_script() {
    let _f = fresh();
    let dir = tmp_dir("app");
    let launch = tmp_dir("app-launch");
    write(&dir, "a.jsx", "vc.log(1);");
    write(&dir, "b.jsx", "throw new Error('broken');");
    let mut big = "//".to_string();
    big.push_str(&"x".repeat(vectorcraft_engine::script::MAX_SOURCE_BYTES));
    write(&dir, "bb.jsx", &big);
    write(&dir, "c.jsx", "vc.log(3);");
    assert!(safe_mode::begin_launch(&launch).is_empty());
    let mut s = session(&dir);
    s.script_frames = true;
    let r = s.execute("script.startup.run", &json!({"reason": "menu", "then": {"source": "vc.log(4);", "name": "chosen"}})).unwrap();
    assert_eq!((r["running"].clone(), r["queued"].clone(), r["failed"].clone()), (json!(true), json!(5), json!([])));
    let marker = || {
        let file = std::fs::read_dir(&launch)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "json"))
            .unwrap();
        safe_mode::parse_marker(&std::fs::read(file).unwrap())
    };
    let names = |items: Vec<safe_mode::Item>| items.into_iter().map(|i| i.name).collect::<Vec<_>>();
    assert_eq!(names(marker()), [Some("a.jsx".to_string())], "the marker names the running startup script");
    let mut results = vec![];
    for _ in 0..10_000 {
        if !s.script_running() {
            break;
        }
        if let Some(r) = s.script_pump() {
            results.push(r.is_ok());
        }
    }
    assert!(!s.script_running());
    assert_eq!(results, [true, false, true, true], "each run reports as it ends");
    assert_eq!(started(&s), ["a.jsx", "b.jsx", "bb.jsx", "c.jsx", "chosen"]);
    let too_large = vectorcraft_engine::cmd::scripts::TOO_LARGE.to_string();
    assert_eq!(s.take_startup_failures(), [("bb.jsx".to_string(), too_large)], "the one that could not start is kept for the app");
    assert!(s.take_startup_failures().is_empty());
    assert!(marker().is_empty(), "the marker names nothing once they have run");
}

#[test]
fn a_chosen_script_that_cannot_start_is_refused_before_any_startup_script_runs() {
    let _f = fresh();
    let dir = tmp_dir("app-then");
    write(&dir, "a.jsx", "vc.log(1);");
    let missing = dir.join("gone").join("missing.jsx");
    let then = json!({"path": missing.to_string_lossy()});
    for frames in [false, true] {
        let mut s = session(&dir);
        s.script_frames = frames;
        let alone = s.execute("script.run", &then).unwrap_err().to_string();
        let e = s.execute("script.startup.run", &json!({"reason": "menu", "then": then})).unwrap_err();
        assert_eq!(e.to_string(), alone, "the error script.run gives, frames {frames}");
        assert!(!s.script_running());
        assert!(started(&s).is_empty(), "no startup script ran, frames {frames}");
        assert!(s.take_startup_failures().is_empty());
        let e = s.execute("script.startup.run", &json!({"reason": "menu", "then": {"name": "neither"}})).unwrap_err();
        assert!(e.to_string().contains("exactly one of"), "{e}");
        assert!(started(&s).is_empty());
    }
}

#[test]
fn a_chosen_script_with_invalid_syntax_is_refused_before_startup_scripts_run() {
    let _f = fresh();
    let dir = tmp_dir("invalid-chosen-syntax");
    write(&dir, "a.jsx", "vc.log('startup ran');");
    for frames in [false, true] {
        let mut s = session(&dir);
        s.script_frames = frames;
        let result = s.execute("script.startup.run", &json!({"reason": "menu", "then": {"source": "var = ;"}}));
        assert!(result.is_err(), "frames={frames}: {result:?}");
        assert!(!s.script_running());
        assert!(started(&s).is_empty(), "no startup script ran, frames={frames}");
        assert!(s.take_startup_failures().is_empty());
    }
}

#[test]
fn an_installed_chosen_script_runs_the_text_read_before_startup_scripts() {
    let _f = fresh();
    for frames in [false, true] {
        let dir = tmp_dir(if frames { "chosen-snapshot-sliced" } else { "chosen-snapshot-sync" });
        let data = dir.join("data");
        let startup = dir.join("startup");
        let entry = write(&startup, "a.jsx", "");
        let real = std::fs::canonicalize(&entry).unwrap();
        let key = vectorcraft_plugins::permissions::sha256_hex(real.to_string_lossy().as_bytes());
        let installed = data.join(&key[..16]);
        let chosen = write(&installed, "chosen.jsx", "vc.log('chosen');");
        let quoted = serde_json::to_string(&chosen.to_string_lossy()).unwrap();
        write(&startup, "a.jsx", &format!("Folder.userData; if (!new File({quoted}).remove()) throw 'remove failed';"));
        let mut s = session(&startup);
        s.attach_script_data(data);
        s.attach_scripts_dir(installed);
        s.script_frames = frames;
        s.execute("script.startup.run", &json!({"reason": "menu", "then": {"path": chosen, "installed": true}})).unwrap();
        for _ in 0..10_000 {
            if !s.script_running() {
                break;
            }
            if let Some(result) = s.script_pump() {
                result.unwrap();
            }
        }
        assert!(!s.script_running());
        assert!(s.take_startup_failures().is_empty());
        assert_eq!(started(&s), ["a.jsx", "chosen.jsx"]);
        assert!(!chosen.exists());
    }
}

#[test]
fn in_the_app_a_stopped_startup_script_keeps_its_report_until_the_next_pump_starts_the_queued_run() {
    let _f = fresh();
    let dir = tmp_dir("app-stop");
    write(&dir, "a.jsx", "for (;;) {}");
    let mut s = session(&dir);
    s.script_frames = true;
    s.execute("script.startup.run", &json!({"reason": "menu", "then": {"source": "vc.log(2);", "name": "chosen"}})).unwrap();
    assert!(s.script_pump().is_none(), "the endless one runs");
    assert!(s.script_request_stop());
    let ended = (0..10_000).find_map(|_| s.script_pump()).unwrap();
    assert!(ended.is_err());
    assert_eq!(s.script_report().unwrap()["kind"], "stopped", "the ended run's report is still there");
    assert!(s.script_running(), "the chosen script waits for the next pump");
    // Nothing else starts ahead of the queued run.
    let e = s.execute("script.run", &json!({"source": "vc.log(9);", "name": "direct"})).unwrap_err();
    assert!(matches!(e, vectorcraft_engine::EngineError::Busy), "{e}");
    assert_eq!(started(&s), ["a.jsx"]);
    let r = (0..10_000).find_map(|_| s.script_pump()).unwrap();
    assert_eq!(r.unwrap()["ok"], true);
    assert_eq!(started(&s), ["a.jsx", "chosen"]);
    assert!(!s.script_running());
}

#[test]
fn in_the_app_a_stop_between_two_runs_stops_the_run_that_starts_next() {
    let _f = fresh();
    let dir = tmp_dir("app-stop-between");
    write(&dir, "a.jsx", "vc.log(1);");
    write(&dir, "b.jsx", "for (;;) {}");
    for by_interrupt in [false, true] {
        let mut s = session(&dir);
        s.script_frames = true;
        let then = json!({"source": "vc.log(3);", "name": "chosen"});
        s.execute("script.startup.run", &json!({"reason": "menu", "then": then, "maxFuel": 1_000_000})).unwrap();
        assert_eq!((0..10_000).find_map(|_| s.script_pump()).unwrap().unwrap()["ok"], true, "a.jsx ends");
        assert!(s.script_running(), "b.jsx waits for the next pump");
        if by_interrupt {
            // The status bar's Stop button.
            s.script_interrupt().set();
        } else {
            assert!(s.script_request_stop(), "script.stop has a run to stop");
        }
        let ended = (0..10_000).find_map(|_| s.script_pump()).unwrap();
        assert!(ended.is_err());
        assert_eq!(s.script_report().unwrap()["kind"], "stopped");
        assert_eq!(s.script_report().unwrap()["name"], "b.jsx");
        assert_eq!((0..10_000).find_map(|_| s.script_pump()).unwrap().unwrap()["ok"], true, "the chosen script still runs");
        assert_eq!(started(&s), ["a.jsx", "b.jsx", "chosen"]);
    }
}

#[test]
fn a_startup_script_refused_before_it_begins_is_in_the_script_console() {
    let _f = fresh();
    let dir = tmp_dir("refused");
    write(&dir, "a.jsx", "vc.log(1);");
    let mut s = session(&dir);
    s.execute("file.new", &json!({})).unwrap();
    s.execute("edit.undoGroup.begin", &json!({})).unwrap();
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    let error = r["failed"][0]["error"].as_str().unwrap().to_string();
    assert!(error.contains("undo group"), "{r}");
    assert_eq!(started(&s), ["a.jsx"]);
    assert_eq!(console(&s, "error"), [format!("a.jsx: {error}")]);
}

#[test]
fn in_the_app_abandoning_between_two_runs_drops_the_queued_runs() {
    let _f = fresh();
    let dir = tmp_dir("app-abandon-between");
    write(&dir, "a.jsx", "vc.log(1);");
    write(&dir, "b.jsx", "vc.log(2);");
    let mut s = session(&dir);
    s.script_frames = true;
    let then = json!({"source": "vc.log(3);", "name": "chosen"});
    s.execute("script.startup.run", &json!({"reason": "menu", "then": then})).unwrap();
    assert_eq!((0..10_000).find_map(|_| s.script_pump()).unwrap().unwrap()["ok"], true, "a.jsx ends");
    assert!(s.script_running() && s.script_name().is_none(), "b.jsx waits for the next pump");
    assert!(s.script_abandon(), "the queued runs are stopped");
    assert!(!s.script_running());
    assert!(s.script_pump().is_none());
    assert_eq!(started(&s), ["a.jsx"]);
}

#[test]
fn in_the_app_startup_scripts_that_cannot_start_are_in_the_reply() {
    let _f = fresh();
    let dir = tmp_dir("app-unread");
    let mut big = "//".to_string();
    big.push_str(&"x".repeat(vectorcraft_engine::script::MAX_SOURCE_BYTES));
    write(&dir, "big.jsx", &big);
    let mut s = session(&dir);
    s.script_frames = true;
    let r = s.execute("script.startup.run", &json!({"reason": "launch"})).unwrap();
    assert_eq!(r["running"], json!(false));
    assert_eq!(r["failed"], json!([{"name": "big.jsx", "error": vectorcraft_engine::cmd::scripts::TOO_LARGE}]));
    assert!(s.take_startup_failures().is_empty(), "the reply names them, so the status bar does not again");
}

#[test]
fn in_the_app_every_queued_run_keeps_the_automation_roots_of_the_request_that_queued_it() {
    let _f = fresh();
    let dir = tmp_dir("app-roots");
    // Writes a file in the run's temporary folder, which is outside the write roots below.
    let source = "var ok; try { var t = new File(Folder.temp.fsName + '/t.txt'); ok = t.open('w'); t.close(); } catch (e) { ok = false; }\n\
                  vc.log(ok ? 'wrote' : 'refused');";
    write(&dir, "a.jsx", source);
    write(&dir, "b.jsx", source);
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[dir.as_path()], &[]).unwrap().unwrap();
    for confined in [false, true] {
        let mut s = session(&dir);
        s.script_frames = true;
        let params = json!({"reason": "menu", "then": {"source": source, "name": "chosen"}});
        let mut request = || s.execute("script.startup.run", &params).unwrap();
        let r = if confined { vectorcraft_engine::file_access::confine(Some(&roots), request) } else { request() };
        assert_eq!(r["queued"], json!(3));
        // The frames that start the later runs are outside the request.
        while s.script_running() {
            s.script_pump();
        }
        assert_eq!(started(&s), ["a.jsx", "b.jsx", "chosen"]);
        let want = if confined { "refused" } else { "wrote" };
        assert_eq!(console(&s, "log"), [want, want, want], "confined {confined}");
    }
}
