//! `script.run` and `script.stop`: one transaction per run, the rollback after a throw, a limit, a
//! stop or a panic, the mutation lease between slices, ids, growth and what a run reports.

use std::cell::Cell;

use serde_json::{Value, json};

use crate::script::{PANIC_IN_SLICE, TEST_GROWTH, TEST_RESULT_BYTES};
use crate::*;

fn rect(x: i64) -> Value {
    json!({"x": x, "y": 0, "width": 10, "height": 10})
}

/// Two open documents, each with one rectangle, the second one active.
fn two_docs() -> Session {
    let mut s = Session::new();
    for x in [0, 50] {
        s.execute("file.new", &json!({})).unwrap();
        s.execute("shape.rectangle", &rect(x)).unwrap();
    }
    s
}

/// What a rollback has to put back.
fn state(s: &Session) -> Value {
    let docs: Vec<Value> = s
        .documents()
        .iter()
        .map(|d| {
            json!({
                "uid": d.uid,
                "art": format!("{:?}", d.doc.layers),
                "selection": d.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>(),
                "undo": d.history.undo.iter().map(|e| e.label.clone()).collect::<Vec<_>>(),
                "redo": d.history.redo.len(),
                "path": d.path,
                "dirty": d.is_dirty(),
            })
        })
        .collect();
    json!({"docs": docs, "active": s.active().map(|d| d.uid), "journal": s.journal.len(), "paint": format!("{:?}", s.paint)})
}

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

fn objects(s: &Session, doc: usize) -> usize {
    s.documents()[doc].doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

/// A framed run of `source`, one slice in.
fn paused(s: &mut Session, source: &str) {
    s.script_frames = true;
    assert_eq!(run(s, source).unwrap()["running"], true);
    assert!(s.script_pump().is_none());
    assert!(s.script_running());
}

fn two_rect_script() -> String {
    "vc.run('shape.rectangle', {x:100,y:0,width:5,height:5});\
     vc.run('shape.rectangle', {x:110,y:0,width:5,height:5});\
     vc.run('document.activate', {index:0});\
     vc.run('shape.rectangle', {x:120,y:0,width:5,height:5});\
     vc.run('document.activate', {index:1});"
        .to_string()
}

#[test]
fn a_run_is_one_undo_step_in_each_document_it_changed() {
    let mut s = two_docs();
    let before = state(&s);
    let r = run(&mut s, &two_rect_script()).unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!((objects(&s, 0), objects(&s, 1)), (2, 3));
    assert_eq!(s.documents()[0].history.undo.last().unwrap().label, "Run Script");
    assert_eq!(s.documents()[1].history.undo.last().unwrap().label, "Run Script");
    assert_eq!(s.documents()[1].history.undo.len(), before["docs"][1]["undo"].as_array().unwrap().len() + 1);
    let after = format!("{:?}", s.documents()[1].doc.layers);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(objects(&s, 1), 1);
    assert_eq!(format!("{:?}", s.documents()[1].doc.layers), before["docs"][1]["art"].as_str().unwrap());
    assert_eq!(s.documents()[1].selection.objects.len(), before["docs"][1]["selection"].as_array().unwrap().len());
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(format!("{:?}", s.documents()[1].doc.layers), after);
}

#[test]
fn a_run_that_changes_nothing_leaves_no_step_no_mark_and_no_journal_entry() {
    for source in ["", "var a = 1 + 2;", "vc.log(vc.document); vc.commands.length;"] {
        let mut s = two_docs();
        s.execute("document.save", &json!({"path": std::env::temp_dir().join(format!("vc-noop-{}.svg", std::process::id())).to_string_lossy()}))
            .unwrap();
        let before = state(&s);
        let revisions = |s: &Session| s.documents().iter().map(|d| d.revision).collect::<Vec<_>>();
        let revisions_before = revisions(&s);
        let r = run(&mut s, source).unwrap();
        assert_eq!(r["changed"], json!([]), "{source}");
        assert_eq!(state(&s), before, "{source}");
        assert_eq!(revisions(&s), revisions_before, "{source}");
    }
}

#[test]
fn a_caught_command_failure_leaves_nothing_behind_and_the_script_goes_on() {
    let mut s = two_docs();
    let source = "var m = '';\
        try { vc.run('command.batch', {commands: [\
            {command:'shape.rectangle', params:{x:1,y:1,width:1,height:1}},\
            {command:'no.such', params:{}}]}); } catch (e) { m = e.message; }\
        try { vc.run('shape.rectangle', {x:'bad'}); } catch (e) { m += '|' + e.name; }\
        vc.log(m); vc.run('shape.rectangle', {x:200,y:0,width:5,height:5});";
    let r = run(&mut s, source).unwrap();
    assert_eq!(objects(&s, 1), 2, "{r}");
    assert!(r["log"][0].as_str().unwrap().contains("|Error"), "{r}");
}

#[test]
fn a_failure_puts_back_every_document_and_the_session() {
    enum How {
        Throw,
        Fuel,
        Stop,
        Panic,
    }
    for how in [How::Throw, How::Fuel, How::Stop, How::Panic] {
        let mut s = two_docs();
        let before = state(&s);
        let edits = two_rect_script();
        let r = match how {
            How::Throw => run(&mut s, &format!("{edits} throw new Error('boom');")),
            How::Fuel => s.execute("script.run", &json!({"source": format!("{edits} while (true) {{}}"), "maxFuel": 2_000_000})),
            How::Stop => {
                paused(&mut s, &format!("{edits} while (true) {{}}"));
                assert_eq!(s.execute("script.stop", &json!({})).unwrap()["stopped"], true);
                assert_eq!(s.execute("script.stop", &json!({})).unwrap()["stopped"], true);
                s.script_pump().unwrap()
            }
            How::Panic => {
                paused(&mut s, &format!("{edits} while (true) {{}}"));
                PANIC_IN_SLICE.with(|p| p.set(true));
                let r = s.script_pump().unwrap();
                PANIC_IN_SLICE.with(|p| p.set(false));
                r
            }
        };
        assert!(r.is_err());
        assert_eq!(state(&s), before);
        assert!(!s.script_running());
        s.script_frames = false;
        assert_eq!(run(&mut s, "1").unwrap()["ok"], true, "a later run starts");
    }
}

#[test]
fn a_stop_with_no_run_reports_it() {
    assert_eq!(Session::new().execute("script.stop", &json!({})).unwrap()["stopped"], false);
}

#[test]
fn an_error_names_the_script_its_place_and_kind() {
    let mut s = two_docs();
    let e = run(&mut s, "var a;\n  throw new TypeError('no');").unwrap_err().to_string();
    assert_eq!(e, "t.js: line 2, column 3: TypeError: no (thrown)");
    let e = run(&mut s, "var = 1;").unwrap_err().to_string();
    assert!(e.starts_with("t.js: line 1,") && e.ends_with("(syntax)"), "{e}");
    assert!(run(&mut s, "app.activeDocument;").unwrap_err().to_string().contains("`vc`"));
    assert!(run(&mut s, "#target x\n1").unwrap_err().to_string().contains("#target"));
    assert_eq!(s.script_report().unwrap()["line"], 1);
}

#[test]
fn bad_parameters_are_refused_before_anything_runs() {
    let mut s = two_docs();
    let before = state(&s);
    let mut deep = json!(1);
    for _ in 0..100 {
        deep = json!([deep]);
    }
    for p in [
        json!({}),
        json!({"source": "1", "args": {"a": deep}}),
        json!({"path": "a", "source": "b"}),
        json!({"path": ""}),
        json!({"path": 1}),
        json!({"source": "1", "nope": 1}),
        json!({"source": "1", "args": []}),
        json!({"source": "1", "args": {"a": "x".repeat(1 << 20)}}),
        json!({"source": "1", "answers": {"a": 1}}),
        json!({"source": "1", "answers": 1}),
        json!({"source": "1", "maxFuel": 0}),
        json!({"source": "1", "maxFuel": -1}),
        json!({"source": "1", "maxFuel": 1.5}),
        json!({"source": "1", "maxFuel": "9"}),
        json!({"source": "1", "maxFuel": 100_000_000_001u64}),
    ] {
        assert!(matches!(s.execute("script.run", &p), Err(EngineError::BadParams { .. })), "{p}");
    }
    assert_eq!(state(&s), before);
}

#[test]
fn files_are_checked_before_they_are_read() {
    let dir = std::env::temp_dir().join(format!("vc-script-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    };
    let mut s = two_docs();
    let path = |p: &str| json!({"path": p});
    let ok = write("Bézier (1) & co.js", b"\xef\xbb\xbfvc.log(1);\r\nvc.log(2);\r\n");
    let r = s.execute("script.run", &path(&ok)).unwrap();
    assert_eq!((r["log"].clone(), r["name"].as_str()), (json!([1, 2]), Some("Bézier (1) & co.js")));
    let invalid = write("a.js", b"var a = '\xff';");
    assert!(s.execute("script.run", &path(&invalid)).unwrap_err().to_string().contains("UTF-8"));
    let huge = write("huge.js", &vec![b' '; script::MAX_SOURCE_BYTES + 1]);
    assert!(s.execute("script.run", &path(&huge)).unwrap_err().to_string().contains("too large"));
    let compiled = write("b.jsxbin", b"@JSXBIN@ES@2.0@");
    assert!(s.execute("script.run", &path(&compiled)).unwrap_err().to_string().contains("compiled"));
    let renamed = write("c.js", b"@JSXBIN@ES@2.0@");
    assert!(s.execute("script.run", &path(&renamed)).unwrap_err().to_string().contains("compiled"));
    assert!(s.execute("script.run", &path(&dir.to_string_lossy())).is_err());
    let huge_source = " ".repeat(script::MAX_SOURCE_BYTES + 1);
    assert!(run(&mut s, &huge_source).unwrap_err().to_string().contains("too large"));
    let other = write("notes.txt", b"vc.log(1);");
    assert!(matches!(s.execute("script.run", &path(&other)), Err(EngineError::BadParams { .. })), "a text file that is not a script");
    let upper = write("UP.JSX", b"vc.log(3);");
    assert_eq!(s.execute("script.run", &path(&upper)).unwrap()["log"], json!([3]));
    std::fs::create_dir_all(dir.join("folder.js")).unwrap();
    assert!(s.execute("script.run", &path(&dir.join("folder.js").to_string_lossy())).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_batch_puts_back_the_session_and_names_a_file_it_wrote() {
    let path = std::env::temp_dir().join(format!("vc-script-batch-{}.svg", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();
    let mut s = two_docs();
    let source = format!(
        "try {{ vc.run('command.batch', {{commands: [\
            {{command: 'view.guides.lock', params: {{locked: true}}}},\
            {{command: 'document.export', params: {{path: {p:?}, format: 'svg'}}}},\
            {{command: 'no.such', params: {{}}}}]}}); }} catch (e) {{ vc.log(e.name); }}"
    );
    let r = run(&mut s, &source).unwrap();
    assert_eq!(r["log"], json!(["Error"]), "{r}");
    assert!(!s.guides_locked(), "the failed batch's Lock Guides does not stay");
    assert!(path.exists());
    assert_eq!(r["written"], json!([p]), "{r}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_export_that_fails_names_no_file() {
    let path = std::env::temp_dir().join(format!("vc-script-no-folder-{}", std::process::id())).join("out.svg");
    let p = path.to_string_lossy().into_owned();
    let mut s = two_docs();
    let source = format!("try {{ vc.run('document.export', {{path: {p:?}, format: 'svg'}}); }} catch (e) {{ vc.log(e.name); }}");
    let r = run(&mut s, &source).unwrap();
    assert_eq!(r["log"], json!(["Error"]), "{r}");
    assert!(!path.exists());
    assert_eq!(r["written"], json!([]), "{r}");
}

#[test]
fn a_failed_run_puts_back_the_choices_its_commands_made() {
    let mut s = two_docs();
    let before = s.global_edit_options.clone();
    let source = "vc.run('select.globalEditOptions', {appearance: !vc.run('select.globalEditOptions', {}).appearance}); throw new Error('x');";
    assert!(run(&mut s, source).is_err());
    assert_eq!(s.global_edit_options, before);
    let source = "try { vc.run('command.batch', {commands: [\
        {command: 'select.globalEditOptions', params: {size: true}},\
        {command: 'no.such', params: {}}]}); } catch (e) {}";
    run(&mut s, source).unwrap();
    assert_eq!(s.global_edit_options, before, "the failed command's choice went back");
}

#[test]
fn a_failed_run_puts_back_the_preferences_its_commands_changed() {
    let mut s = two_docs();
    let basic = s.prefs.new_art_basic;
    assert!(run(&mut s, &format!("vc.run('appearance.setNewArtBasic', {{on: {}}}); throw new Error('x');", !basic)).is_err());
    assert_eq!(s.prefs.new_art_basic, basic);
}

#[test]
fn a_finished_run_keeps_its_preferences_and_the_ones_the_app_changed_meanwhile() {
    let mut s = two_docs();
    let (basic, increment) = (s.prefs.new_art_basic, s.prefs.keyboard_increment);
    paused(&mut s, &format!("vc.run('appearance.setNewArtBasic', {{on: {}}}); for (var i = 0; i < 2000000; i++) {{}}", !basic));
    assert_eq!(s.prefs.new_art_basic, basic, "between slices the app sees the preferences before the run");
    s.prefs.keyboard_increment = increment + 1.0;
    while s.script_pump().is_none() {}
    assert_eq!((s.prefs.new_art_basic, s.prefs.keyboard_increment), (!basic, increment + 1.0));
}

#[test]
fn a_script_cannot_load_the_place_cursor() {
    let mut s = two_docs();
    let e = run(&mut s, "vc.run('file.place.queue', {paths: []});").unwrap_err().to_string();
    assert!(e.contains("a script cannot run `file.place.queue`"), "{e}");
    let listed = run(&mut s, "vc.log(vc.commands.some(function (c) { return c.id === 'file.place.queue'; }));").unwrap();
    assert_eq!(listed["log"], json!([false]), "{listed}");
}

#[test]
fn a_command_whose_result_cannot_cross_is_put_back() {
    TEST_RESULT_BYTES.with(|b| b.set(Some(1)));
    let mut s = two_docs();
    let r = run(&mut s, "try { vc.run('shape.rectangle', {x:1,y:1,width:1,height:1}); } catch (e) { vc.log(e.name); }");
    TEST_RESULT_BYTES.with(|b| b.set(None));
    let r = r.unwrap();
    assert_eq!(r["log"], json!(["RangeError"]), "{r}");
    assert_eq!(r["changed"], json!([]), "{r}");
    assert_eq!((objects(&s, 0), objects(&s, 1)), (1, 1));
}

#[test]
fn an_abandoned_run_reports_the_files_it_wrote() {
    let path = std::env::temp_dir().join(format!("vc-script-abandon-{}.svg", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();
    let mut s = two_docs();
    paused(&mut s, &format!("vc.run('document.export', {{path: {p:?}, format: 'svg'}}); while (true) {{}}"));
    assert!(path.exists(), "the first slice exported");
    assert!(s.script_abandon());
    let report = s.script_report().unwrap();
    assert_eq!((report["kind"].as_str(), report["written"].clone()), (Some("stopped"), json!([p])), "{report}");
    assert!(s.script_failure_text().unwrap().contains(&p));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_run_that_only_closes_a_document_keeps_its_journal_entry() {
    let mut s = two_docs();
    let journal = s.journal.len();
    let r = run(&mut s, "vc.run('file.close', {index: 1});").unwrap();
    assert_eq!(r["changed"], json!([]), "{r}");
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.iter().last().unwrap().0, "file.close");
}

#[test]
fn a_command_that_can_fail_after_copying_has_the_clipboard_put_back() {
    let mut s = two_docs();
    let keep_clipboard = |s: &mut Session, id: &str| {
        let save = script::Savepoint::take(s, id);
        s.clipboard.source_artboard = Some(Default::default());
        save.restore(s);
    };
    keep_clipboard(&mut s, "edit.cut");
    assert_eq!(s.clipboard.source_artboard, None, "put back");
    s.clipboard.source_artboard = Some(Default::default());
    keep_clipboard(&mut s, "shape.rectangle");
    assert!(s.clipboard.source_artboard.is_some(), "only those commands pay for a copy of the clipboard");
}

#[test]
fn a_snapshot_too_large_to_cross_throws_instead_of_reading_as_no_document() {
    let mut s = two_docs();
    s.docs[1].path = Some("x".repeat(17 << 20));
    let r = run(&mut s, "var m = ''; try { vc.document; m = 'read'; } catch (e) { m = e.name + ': ' + e.message; } vc.log(m);").unwrap();
    let m = r["log"][0].as_str().unwrap();
    assert!(m.starts_with("RangeError: vc.document.") && m.ends_with("is too large for a script"), "{m}");
}

#[test]
fn recovery_copies_made_between_slices_hold_the_art_before_the_run() {
    let store = std::sync::Arc::new(cmd::recovery::MemoryStore::default());
    let mut s = two_docs();
    s.recovery.set_store(store.clone());
    paused(&mut s, &format!("{} while (true) {{}}", two_rect_script()));
    cmd::recovery::save_copies(&mut s).unwrap();
    s.script_abandon();
    drop(s);
    let mut s = Session::new();
    s.recovery.set_store(store);
    let r = s.execute("file.recovery.restore", &json!({})).unwrap();
    assert_eq!(r["restored"].as_array().map(Vec::len), Some(2), "{r}");
    assert_eq!((objects(&s, 0), objects(&s, 1)), (1, 1), "the copies hold the art from before the run");
}

#[test]
fn values_that_cannot_cross_are_named_and_shared_ones_can() {
    let mut s = two_docs();
    let cases = [
        ("var a = {}; a.a = a; vc.run('shape.rectangle', a);", "RangeError|TypeError"),
        ("var a = {}, c = a; for (var i = 0; i < 100; i++) { c.n = {}; c = c.n; } vc.run('shape.rectangle', a);", "RangeError|TypeError"),
        ("vc.run('shape.rectangle', {x: 1/0});", "RangeError|TypeError"),
        ("vc.run('shape.rectangle', {x: function () {}});", "RangeError|TypeError"),
        ("vc.run('shape.rectangle', 7);", "TypeError"),
        ("vc.run(7);", "TypeError"),
    ];
    for (source, kinds) in cases {
        let e = run(&mut s, source).unwrap_err().to_string();
        assert!(kinds.split('|').any(|k| e.contains(k)), "{source}: {e}");
    }
    let r = run(&mut s, "var p = {w: 5}; vc.log({a: p, b: p}); var l = ''; for (var i = 0; i < 20; i++) l += 'xxxxxxxxxx'; vc.log(l);").unwrap();
    assert_eq!(r["log"][0], json!({"a": {"w": 5}, "b": {"w": 5}}));
    assert_eq!(r["log"][1].as_str().unwrap().len(), 200);
}

#[test]
fn commands_outside_the_documents_are_refused_in_a_batch_too() {
    let mut s = two_docs();
    for source in [
        "vc.run('prefs.set', {key: 'a', value: 1});",
        "vc.run('script.run', {source: '1'});",
        "vc.run('script.stop');",
        "vc.run('edit.undoGroup.begin', {});",
        "vc.run('view.proofColors', {on: true});",
        "vc.run('view.proofSetup', {});",
        "vc.run('view.overprintPreview', {on: true});",
        "vc.run('view.separationsPreview', {});",
        "vc.run('command.batch', {commands: [{command: 'prefs.reset', params: {}}]});",
    ] {
        let e = run(&mut s, source).unwrap_err().to_string();
        assert!(e.contains("cannot run"), "{source}: {e}");
    }
    let r = run(&mut s, "var c = vc.commands; var n = 0; for (var i = 0; i < c.length; i++) if (c[i].id.indexOf('prefs.') === 0) n++; vc.log(n); vc.log(c.length > 100);").unwrap();
    assert_eq!(r["log"], json!([0, true]));
}

#[test]
fn the_vc_object_reads_the_active_document_and_cannot_be_changed() {
    let mut s = two_docs();
    let r = s
        .execute(
            "script.run",
            &json!({"name": "x", "args": {"n": 2}, "source": "vc.args.n = 9; vc.api.version = 'x'; vc.log(vc.args.n); vc.log(vc.api.version); vc.log(vc.api.has('run')); vc.log(vc.api.has('nope')); vc.log(vc.document.artboards); vc.log(typeof vc.document.path);"}),
        )
        .unwrap();
    assert_eq!(r["log"], json!([2, "1.0", true, false, 1, "object"]));
    let mut empty = Session::new();
    assert_eq!(run(&mut empty, "vc.log(vc.document === null);").unwrap()["log"], json!([true]));
    assert!(run(&mut empty, "vc.run('shape.rectangle', {x:0,y:0,width:1,height:1});").unwrap_err().to_string().contains("Error"));
}

#[test]
fn the_log_is_bounded_and_a_script_can_catch_that() {
    let mut s = two_docs();
    let r = run(
        &mut s,
        "var n = 0; try { while (true) { vc.log(n); n++; } } catch (e) { vc.run('document.inspect', {}); throw new Error(e.name + ' at ' + n); }",
    );
    let e = r.unwrap_err().to_string();
    assert!(e.contains(&format!("RangeError at {}", script::MAX_LOG_LINES)), "{e}");
}

#[test]
fn between_slices_every_change_is_refused_and_readers_see_the_committed_art() {
    let mut s = two_docs();
    let before = state(&s);
    paused(&mut s, &format!("{} while (true) {{}}", two_rect_script()));
    assert_eq!(state(&s), before);
    for (id, p) in [
        ("shape.rectangle", rect(1)),
        ("clipboard.importSvg", json!({})),
        ("clipboard.importImage", json!({})),
        ("clipboard.importText", json!({})),
        ("clipboard.importPdf", json!({})),
        ("clipboard.importEmf", json!({})),
        ("clipboard.importEps", json!({})),
        ("edit.undo", json!({})),
        ("file.new", json!({})),
        ("file.close", json!({})),
        ("file.revert", json!({})),
        ("file.recovery.restore", json!({})),
        ("action.batch", json!({})),
        ("command.batch", json!({"commands": []})),
        ("effect.apply", json!({})),
        ("script.run", json!({"source": "1"})),
    ] {
        assert!(matches!(s.execute(id, &p), Err(EngineError::Busy)), "{id}");
    }
    assert!(matches!(s.begin_interaction("x"), Err(EngineError::Busy)));
    assert!(matches!(s.edit("x", |_, _| Ok(())), Err(EngineError::Busy)));
    assert!(matches!(s.preview("shape.rectangle", &rect(1)), Err(EngineError::Busy)));
    let press = tools::PointerEvent {
        kind: tools::PointerKind::Down,
        ..serde_json::from_value(json!({"kind": "move", "pos": {"x": 0.0, "y": 0.0}})).unwrap()
    };
    assert!(matches!(s.pointer(&press, ViewInfo::default()), Err(EngineError::Busy)));
    let inspected = s.execute("document.inspect", &json!({})).unwrap();
    assert_eq!(inspected["layers"][0]["children"].as_array().map(Vec::len), Some(1), "{inspected}");
    assert_eq!(state(&s), before, "the refused commands changed nothing");
    s.script_abandon();
    assert_eq!(state(&s), before);
    assert!(s.execute("shape.rectangle", &rect(1)).unwrap()["id"].is_u64());
}

#[test]
fn a_document_closed_or_opened_in_a_run_follows_the_run() {
    let closing = "vc.run('file.close', {index: 0}); vc.run('file.new', {}); vc.run('shape.rectangle', {x:1,y:1,width:1,height:1});";
    let mut s = two_docs();
    let before = state(&s);
    assert!(run(&mut s, &format!("{closing} throw new Error('x');")).is_err());
    assert_eq!(state(&s), before);
    let first = s.documents()[0].uid;
    run(&mut s, closing).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert!(s.documents().iter().all(|d| d.uid != first));
    assert_eq!(s.documents()[1].history.undo.len(), 0, "a document the script made starts with no undo steps");
}

#[test]
fn undo_in_a_run_reaches_only_the_runs_own_steps_and_old_ids_stay_dead() {
    let mut s = two_docs();
    let history = s.documents()[1].history.undo.len();
    let source = "var p = vc.run('shape.rectangle', {x:1,y:1,width:1,height:1}).id;\
        vc.run('edit.undo');\
        var q = vc.run('shape.rectangle', {x:2,y:2,width:1,height:1}).id;\
        var why = '';\
        try { vc.run('edit.clear', {ids: [p]}); } catch (e) { why = e.message; }\
        vc.run('edit.undo');\
        try { vc.run('edit.undo'); why += '|undid a step from before the run'; } catch (e) { why += '|stopped'; }\
        vc.log({same: p === q, why: why});\
        vc.run('shape.rectangle', {x:3,y:3,width:1,height:1});";
    let r = run(&mut s, source).unwrap();
    assert_eq!(r["log"][0]["same"], false, "{r}");
    let why = r["log"][0]["why"].as_str().unwrap();
    assert!(why.ends_with("|stopped") && why.len() > "|stopped".len(), "{why}");
    assert_eq!(objects(&s, 1), 2);
    assert_eq!(s.documents()[1].history.undo.len(), history + 1);
}

#[test]
fn a_string_over_the_crossing_limit_does_not_cross() {
    let limits = vectorcraft_js::DataLimits { bytes: 20, ..vectorcraft_js::DataLimits::default() };
    assert!(script::to_data(&json!("four"), &limits, 0, &mut (0, 0)).is_ok());
    assert_eq!(script::to_data(&json!("fives"), &limits, 0, &mut (0, 0)), Err(String::new()));
}

#[test]
fn a_result_too_large_to_cross_names_its_place() {
    let limits = vectorcraft_js::DataLimits { bytes: 90, ..vectorcraft_js::DataLimits::default() };
    let v = json!({"a": [1, {"name": "x".repeat(40)}]});
    assert_eq!(script::to_data(&v, &limits, 0, &mut (0, 0)), Err(".a[1].name".to_string()));
}

#[test]
fn ids_given_out_in_an_abandoned_run_are_not_given_out_again() {
    let mut s = two_docs();
    let before = s.active().unwrap().doc.peek_next_id();
    paused(&mut s, &format!("{} while (true) {{}}", two_rect_script()));
    assert!(s.script_abandon());
    assert!(s.active().unwrap().doc.peek_next_id() > before);
}

#[test]
fn ids_given_out_in_a_failed_run_are_not_given_out_again() {
    let mut s = two_docs();
    let e =
        run(&mut s, "var p = vc.run('shape.rectangle', {x:1,y:1,width:1,height:1}).id; throw new Error('id=' + p + ';');").unwrap_err().to_string();
    let used: u64 = e.split("id=").nth(1).unwrap().split(';').next().unwrap().parse().unwrap();
    let dirty = s.documents()[1].is_dirty();
    let id = s.execute("shape.rectangle", &rect(3)).unwrap()["id"].as_u64().unwrap();
    assert!(id > used, "{id} after {used}");
    assert!(dirty);
}

#[test]
fn a_rollback_keeps_the_history_after_an_undo_in_the_run() {
    let mut s = two_docs();
    let before = state(&s);
    let source = "vc.run('shape.rectangle', {x:1,y:1,width:1,height:1}); vc.run('edit.undo'); throw new Error('x');";
    assert!(run(&mut s, source).is_err());
    assert_eq!(state(&s), before);
}

#[test]
fn a_batch_joins_the_runs_one_step() {
    let mut s = two_docs();
    let undo = s.documents()[1].history.undo.len();
    let source = "vc.run('command.batch', {commands: [\
        {command:'shape.rectangle', params:{x:1,y:1,width:1,height:1}},\
        {command:'shape.rectangle', params:{x:2,y:2,width:1,height:1}}]});";
    run(&mut s, source).unwrap();
    assert_eq!(s.documents()[1].history.undo.len(), undo + 1);
    assert_eq!(objects(&s, 1), 3);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(objects(&s, 1), 1);
}

#[test]
fn a_run_with_nobody_to_ask_ends_at_its_growth_allowance() {
    TEST_GROWTH.with(|g| g.set(Some(20)));
    let mut s = two_docs();
    let before = state(&s);
    let source = "try { while (true) vc.run('shape.rectangle', {x:1,y:1,width:1,height:1}); } catch (e) { vc.run('shape.rectangle', {x:2,y:1,width:1,height:1}); }";
    let e = run(&mut s, source).unwrap_err().to_string();
    TEST_GROWTH.with(|g| g.set(None));
    assert!(e.contains("objects and data"), "{e}");
    assert_eq!(state(&s), before);
}

#[test]
fn a_document_a_run_opens_counts_toward_its_growth_allowance() {
    TEST_GROWTH.with(|g| g.set(Some(20)));
    let rects: String = (0..30).map(|i| format!(r#"<rect x="{i}" y="0" width="1" height="1"/>"#)).collect();
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{rects}</svg>"#);
    let data = vectorcraft_format::base64_encode(svg.as_bytes());
    let mut s = two_docs();
    let before = state(&s);
    let source = format!("try {{ vc.run('document.open', {{name: 'a.svg', dataBase64: '{data}'}}); }} catch (e) {{}}");
    let e = run(&mut s, &source).unwrap_err().to_string();
    assert!(e.contains("objects and data"), "{e}");
    assert_eq!(state(&s), before);

    // A document whose art is all in a symbol counts it too.
    let mut maker = Session::new();
    maker.execute("file.new", &json!({})).unwrap();
    for x in 0..30 {
        maker.execute("shape.rectangle", &rect(x)).unwrap();
    }
    maker.execute("select.all", &json!({})).unwrap();
    let instance = maker.execute("symbol.new", &json!({"name": "Many"})).unwrap()["id"].as_u64().unwrap();
    maker.execute("edit.clear", &json!({"ids": [instance]})).unwrap();
    assert_eq!(objects(&maker, 0), 0);
    let data = maker.execute("document.serialize", &json!({})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    let source = format!("vc.run('document.open', {{name: 'b.vectorcraft', dataBase64: '{data}'}});");
    let e = run(&mut s, &source).unwrap_err().to_string();
    TEST_GROWTH.with(|g| g.set(None));
    assert!(e.contains("objects and data"), "{e}");
    assert_eq!(state(&s), before);
}

#[test]
fn a_person_can_let_a_growing_run_go_on_or_stop_it() {
    TEST_GROWTH.with(|g| g.set(Some(20)));
    let run_to_the_question = |s: &mut Session| {
        s.script_frames = true;
        run(s, "for (var i = 0; i < 25; i++) vc.run('shape.rectangle', {x:i,y:1,width:1,height:1});").unwrap();
        let slices = Cell::new(0);
        while !s.script_waiting() {
            assert!(s.script_pump().is_none());
            slices.set(slices.get() + 1);
            assert!(slices.get() < 1_000);
        }
        // Waiting takes no fuel, however often the frame comes round.
        assert!(s.script_pump().is_none());
    };
    let mut s = two_docs();
    run_to_the_question(&mut s);
    s.script_continue();
    assert!(!s.script_waiting());
    let done = loop {
        if let Some(r) = s.script_pump() {
            break r;
        }
    };
    assert_eq!(done.unwrap()["ok"], true);
    assert_eq!(objects(&s, 1), 26);

    let mut s = two_docs();
    let before = state(&s);
    run_to_the_question(&mut s);
    assert_eq!(s.execute("script.stop", &json!({})).unwrap()["stopped"], true);
    assert!(!s.script_waiting());
    assert!(s.script_pump().unwrap().is_err());
    TEST_GROWTH.with(|g| g.set(None));
    assert_eq!(state(&s), before);
}

#[test]
fn a_file_saved_before_a_failure_stays_written_and_is_named() {
    let path = std::env::temp_dir().join(format!("vc-script-save-{}.svg", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();
    let mut s = two_docs();
    let before = state(&s);
    let source =
        format!("vc.run('file.saveAs', {{path: {p:?}}}); vc.run('shape.rectangle', {{x:1,y:1,width:1,height:1}}); throw new Error('later');");
    let e = run(&mut s, &source).unwrap_err().to_string();
    assert!(path.exists());
    assert!(e.contains(&p), "{e}");
    assert_eq!(s.script_report().unwrap()["written"], json!([p]));
    assert_eq!(state(&s), before, "the document does not keep the new path");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_committed_run_exports_what_it_drew() {
    let path = std::env::temp_dir().join(format!("vc-script-export-{}.svg", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let p = path.to_string_lossy().into_owned();
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let source = "for (var i = 0; i < 7; i++) vc.run('shape.rectangle', {x:i*20,y:5,width:10,height:10});";
    run(&mut s, source).unwrap();
    s.execute("document.save", &json!({"path": p})).unwrap();
    let svg = std::fs::read_to_string(&path).unwrap();
    assert_eq!(svg.matches("<rect").count() + svg.matches("<path").count(), 7, "{svg}");
    let inspected = s.execute("document.inspect", &json!({})).unwrap();
    assert_eq!(inspected["layers"][0]["children"].as_array().map(Vec::len), Some(7));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_run_needing_a_recovery_copy_it_cannot_write_does_not_start() {
    let mut s = two_docs();
    s.script_copy_first = true;
    let before = state(&s);
    let e = run(&mut s, "vc.run('shape.rectangle', {x:1,y:1,width:1,height:1});").unwrap_err().to_string();
    assert!(e.contains("recovery copy"), "{e}");
    assert_eq!(state(&s), before);
    assert!(!s.script_running());
}

#[test]
fn a_run_does_not_start_while_a_complex_document_goes_without_a_copy() {
    let store = std::sync::Arc::new(cmd::recovery::MemoryStore::default());
    let mut s = Session::new();
    s.recovery.set_store(store);
    s.script_copy_first = true;
    let mut d = vectorcraft_doc::Document::new(1000.0, 1000.0);
    let layer = d.layers[0].id;
    for i in 0..cmd::recovery::COMPLEX_OBJECTS {
        let id = d.alloc_id();
        let r = vectorcraft_geom::Rect::new((i % 200) as f64, (i / 200) as f64, (i % 200) as f64 + 1.0, (i / 200) as f64 + 1.0);
        let n = vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(r), vectorcraft_doc::Appearance::default_art());
        d.insert(Some(layer), i, n).unwrap();
    }
    s.add_document(d, None);
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    s.prefs.recovery_off_for_complex = true;
    let e = run(&mut s, "1").unwrap_err().to_string();
    assert!(e.contains("recovery copy"), "{e}");
    assert!(!s.script_running());
    s.prefs.recovery_off_for_complex = false;
    assert_eq!(run(&mut s, "1").unwrap()["ok"], true);
}

#[test]
fn an_open_undo_group_stops_a_run_from_starting() {
    let mut s = two_docs();
    s.execute("edit.undoGroup.begin", &json!({})).unwrap();
    assert!(run(&mut s, "1").unwrap_err().to_string().contains("undo group"));
    s.execute("edit.undoGroup.end", &json!({})).unwrap();
    assert_eq!(run(&mut s, "1").unwrap()["ok"], true);
}

#[test]
fn a_cancel_from_the_caller_stops_a_headless_run() {
    let mut s = two_docs();
    let before = state(&s);
    let stop = s.script_interrupt();
    stop.set();
    assert!(s.execute("script.run", &json!({"source": "1", "maxFuel": 10_000})).is_ok(), "a run clears a flag left from before");
    assert!(!stop.is_set());
    assert_eq!(state(&s), before);
}
