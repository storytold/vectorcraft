//! A script's preprocessor lines: `#include` and `//@include` in place, the folders they are looked
//! for in, the run's limits and scope, the other lines, `$.evalFile`, `$.fileName`, and errors at the
//! line of the file they are in.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::script::TEST_JSINCLUDE;
use crate::*;

/// A fresh folder for test `name`, with a `main` folder inside it for the script.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-script-include-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    base
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// Runs `main/main.jsx` in `base`, with the text `text`.
fn run(base: &Path, text: &str) -> Result<Value> {
    let main = base.join("main/main.jsx");
    write(&main, text);
    Session::new().execute("script.run", &json!({"path": main.to_string_lossy()}))
}

/// The report of the run that failed with `e`.
fn failed(base: &Path, text: &str) -> Value {
    let mut s = Session::new();
    let main = base.join("main/main.jsx");
    write(&main, text);
    let e = s.execute("script.run", &json!({"path": main.to_string_lossy()}));
    assert!(e.is_err(), "the run failed: {e:?}");
    s.script_report().unwrap().clone()
}

#[test]
fn script_include_sibling_runs_in_place() {
    let base = folder("sibling");
    write(&base.join("main/lib.jsxinc"), "var b = a + 1;");
    write(&base.join("main/two.jsxinc"), "b = b * 10;\r\n");
    let r = run(&base, "var a = 1;\n  #include \"lib.jsxinc\"\n//@include 'two.jsxinc'\nvc.log(a + b);").unwrap();
    assert_eq!(r["log"], json!([21]));
    // A file may be included again once its earlier inclusion has ended.
    let r = run(&base, "var n = 0;\n#include \"three.jsxinc\"\n#include \"three.jsxinc\"\nvc.log(n);");
    assert!(r.is_err(), "three.jsxinc is missing");
    write(&base.join("main/three.jsxinc"), "n++;");
    let r = run(&base, "var n = 0;\n#include \"three.jsxinc\"\n#include three.jsxinc\nvc.log(n);").unwrap();
    assert_eq!(r["log"], json!([2]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_nested_include_uses_including_folder() {
    let base = folder("nested");
    write(&base.join("main/sub/one.jsxinc"), "#include \"two.jsxinc\"\nvar one = two + 1;");
    write(&base.join("main/sub/two.jsxinc"), "var two = 2;");
    write(&base.join("main/two.jsxinc"), "var two = 200;");
    let r = run(&base, "#include \"sub/one.jsxinc\"\nvc.log(one);").unwrap();
    assert_eq!(r["log"], json!([3]), "two.jsxinc beside one.jsxinc, not beside the script");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_missing_include_reports_origin() {
    let base = folder("missing");
    let report = failed(&base, "vc.log(1);\n\t#include \"nope.jsxinc\"\n");
    assert_eq!(report["kind"], "thrown");
    assert_eq!((report["line"].as_u64(), report["column"].as_u64()), (Some(2), Some(2)));
    assert!(report["message"].as_str().unwrap().contains("include file not found: nope.jsxinc"), "{report}");
    assert_eq!(report["log"], json!([]), "nothing ran before the include was read");
    assert!(report.get("file").is_none(), "the line is the script's own");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_includepath_searches_in_order() {
    let base = folder("includepath");
    write(&base.join("main/a/x.jsxinc"), "var x = 'a';");
    write(&base.join("main/b/x.jsxinc"), "var x = 'b';");
    write(&base.join("main/b/y.jsxinc"), "var y = 'b';");
    let r = run(&base, "#includepath \"a; b\"\n#include \"x.jsxinc\"\n#include \"y.jsxinc\"\nvc.log(x + y + $.includePath);").unwrap();
    assert_eq!(r["log"], json!(["aba;b"]));
    // A later #includepath replaces the list.
    let report = failed(&base, "#includepath \"b\"\n#includepath \"a\"\n#include \"y.jsxinc\"");
    assert!(report["message"].as_str().unwrap().contains("include file not found: y.jsxinc"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_include_cycle_and_depth_stop() {
    let base = folder("cycle");
    write(&base.join("main/a.jsxinc"), "#include \"b.jsxinc\"");
    write(&base.join("main/b.jsxinc"), "\n#include \"a.jsxinc\"");
    let report = failed(&base, "#include \"a.jsxinc\"");
    assert!(report["message"].as_str().unwrap().contains("include cycle: a.jsxinc"), "{report}");
    assert_eq!(report["line"], 2);
    assert!(report["file"].as_str().unwrap().ends_with("b.jsxinc"), "{report}");
    // A script that includes itself.
    let report = failed(&base, "#include \"main.jsx\"");
    assert!(report["message"].as_str().unwrap().contains("include cycle"), "{report}");
    for i in 0..20 {
        write(&base.join(format!("main/d{i}.jsxinc")), &format!("#include \"d{}.jsxinc\"", i + 1));
    }
    let report = failed(&base, "#include \"d0.jsxinc\"");
    assert_eq!(report["kind"], "limit");
    assert!(report["message"].as_str().unwrap().contains("include depth limit (16) reached"), "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("d15.jsxinc"), "the 17th level is refused: {report}");
    let _ = std::fs::remove_dir_all(base);
}

/// Makes `link` lead to the folder `target`: a junction on Windows, a symbolic link elsewhere.
/// `false` when the system does not allow it here.
fn link_folder(target: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().is_ok_and(|o| o.status.success());
        made && std::fs::canonicalize(link).ok() == std::fs::canonicalize(target).ok()
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        false
    }
}

#[test]
fn script_include_link_out_of_the_folder_is_refused() {
    let base = folder("junction");
    write(&base.join("outside/x.jsxinc"), "var leaked = 1;");
    // `mklink` reads a `/` in its paths as a switch, so each name is joined on its own.
    if !link_folder(&base.join("outside"), &base.join("main").join("away")) {
        eprintln!("skipped: a folder link can't be made here");
        let _ = std::fs::remove_dir_all(base);
        return;
    }
    assert!(std::fs::canonicalize(base.join("main/away/x.jsxinc")).unwrap().starts_with(std::fs::canonicalize(base.join("outside")).unwrap()));
    let report = failed(&base, "#include \"away/x.jsxinc\"\nvc.log(leaked);");
    assert!(report["message"].as_str().unwrap().contains("script file read refused: away/x.jsxinc"), "{report}");
    let report = failed(&base, "$.evalFile('away/x.jsxinc');");
    assert!(report["message"].as_str().unwrap().contains("script file read refused: away/x.jsxinc"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_include_parent_component_refused() {
    let base = folder("parent");
    write(&base.join("outside.jsxinc"), "var leaked = 1;");
    write(&base.join("main/sub/x.jsxinc"), "var x = 1;");
    let report = failed(&base, "#include \"../outside.jsxinc\"");
    assert!(report["message"].as_str().unwrap().contains("script file read refused: ../outside.jsxinc"), "{report}");
    // Refused even where the path would come back inside the folder.
    let report = failed(&base, "#include \"sub/../sub/x.jsxinc\"");
    assert!(report["message"].as_str().unwrap().contains("script file read refused"), "{report}");
    let report = failed(&base, "#includepath \"..\"\n#include \"outside.jsxinc\"");
    assert!(report["message"].as_str().unwrap().contains("script file read refused: outside.jsxinc"), "{report}");
    let absolute = base.join("outside.jsxinc").to_string_lossy().replace('\\', "/");
    let report = failed(&base, &format!("#include \"{absolute}\""));
    assert!(report["message"].as_str().unwrap().contains("script file read refused"), "an absolute path outside: {report}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_jsinclude_outside_grants_ignored() {
    let base = folder("jsinclude");
    write(&base.join("outside/x.jsxinc"), "var x = 'outside';");
    write(&base.join("main/lib/y.jsxinc"), "var y = 'lib';");
    TEST_JSINCLUDE.with(|t| *t.borrow_mut() = Some(vec![base.join("outside"), base.join("main/lib")]));
    let report = failed(&base, "#include \"x.jsxinc\"");
    let r = run(&base, "#include \"y.jsxinc\"\nvc.log(y);");
    TEST_JSINCLUDE.with(|t| *t.borrow_mut() = None);
    assert!(report["message"].as_str().unwrap().contains("include file not found: x.jsxinc"), "passed over, not refused: {report}");
    assert_eq!(r.unwrap()["log"], json!(["lib"]), "a JSINCLUDE folder inside the script's folder is searched");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_target_other_app_does_not_route() {
    let mut s = Session::new();
    let r = s.execute("script.run", &json!({"source": "#target someapp-12\n#target \"other\"\n//@target x\n#strict off\nvc.log(1);"})).unwrap();
    assert_eq!(r["log"], json!([1]));
    for (source, message) in [
        ("#target\nvc.log(1);", "#target needs a value"),
        ("#targetengine \"session\"", "unsupported script directive: #targetengine"),
        ("#engine \"main\"", "unsupported script directive: #engine"),
        ("#strict on", "unsupported strict checking: #strict on"),
        ("#strict maybe", "#strict takes on or off"),
        ("#frobnicate 1", "unknown script directive: #frobnicate"),
        ("#include \"x.jsxinc", "unterminated quote in #include"),
        ("#include \"x.jsxinc\"", "script file read refused: x.jsxinc"),
    ] {
        let e = s.execute("script.run", &json!({"source": format!("vc.log(0);\n{source}")})).unwrap_err().to_string();
        assert!(e.contains(message), "{source}: {e}");
        assert_eq!(s.script_report().unwrap()["line"], 2, "{source}");
        assert_eq!(s.script_report().unwrap()["log"], json!([]), "{source}");
    }
    // An unknown `//@` name stays a comment.
    assert_eq!(s.execute("script.run", &json!({"source": "//@frobnicate 1\nvc.log(2);"})).unwrap()["log"], json!([2]));
}

#[test]
fn script_name_comes_from_the_script_line() {
    let mut s = Session::new();
    let r = s.execute("script.run", &json!({"source": "#script \"First\"\n#script Second\nvc.log($.fileName);"})).unwrap();
    assert_eq!((r["name"].as_str(), r["log"].clone()), (Some("Second"), json!([""])));
    let r = s.execute("script.run", &json!({"source": "#script Second\n", "name": "given.js"})).unwrap();
    assert_eq!(r["name"], "given.js", "a name given to script.run wins");
    let base = folder("name");
    write(&base.join("main/lib.jsxinc"), "#script Inner\n");
    let r = run(&base, "#include \"lib.jsxinc\"").unwrap();
    assert_eq!(r["name"], "main.jsx", "an included file's #script line does not rename the run");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_strict_on_is_refused() {
    let mut s = Session::new();
    let e = s.execute("script.run", &json!({"source": "#strict on\nvc.log(1);"})).unwrap_err().to_string();
    assert!(e.contains("unsupported strict checking: #strict on"), "{e}");
    assert_eq!(s.script_report().unwrap()["kind"], "syntax");
}

#[test]
fn script_eval_file_returns_last_value_and_shares_globals() {
    let base = folder("evalfile");
    write(&base.join("main/sub/e.jsx"), "var inner = $.fileName;\nfunction h(x) { return x * g; }\n#include \"f.jsxinc\"\ng + f;");
    write(&base.join("main/sub/f.jsxinc"), "var f = 40;");
    write(&base.join("main/empty.jsx"), "");
    let r = run(
        &base,
        "var g = 2;\nvar r = $.evalFile('sub/e.jsx', 1000);\nvc.log(r);\nvc.log(h(3));\nvc.log(/e\\.jsx$/.test(inner));\nvc.log(/main\\.jsx$/.test($.fileName));\nvc.log($.evalFile('empty.jsx') === undefined);",
    )
    .unwrap();
    assert_eq!(r["log"], json!([42, 6, true, true, true]));
    // Its errors can be caught; a file is evaluated again after it has returned.
    let r = run(
        &base,
        "var g = 1; var out = [];\ntry { $.evalFile(''); } catch (e) { out.push(e.name); }\ntry { $.evalFile('nope.jsx'); } catch (e) { out.push(String(e)); }\ntry { $.evalFile('sub/e.jsx', -1); } catch (e) { out.push(e.name); }\ntry { $.evalFile('sub/e.jsx', Infinity); } catch (e) { out.push(e.name); }\nout.push($.evalFile('sub/e.jsx') + $.evalFile('sub/e.jsx'));\nvc.log(out);",
    )
    .unwrap();
    assert_eq!(r["log"], json!([["TypeError", "Error: include file not found: nope.jsx", "RangeError", "TypeError", 82]]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_eval_file_outside_scope_refused() {
    let base = folder("evaloutside");
    write(&base.join("outside.jsx"), "1;");
    let absolute = base.join("outside.jsx").to_string_lossy().replace('\\', "/");
    let script = format!(
        "var out = [];\ntry {{ $.evalFile('../outside.jsx'); }} catch (e) {{ out.push(String(e)); }}\ntry {{ $.evalFile('{absolute}'); }} catch (e) {{ out.push(String(e)); }}\nvc.log(out);"
    );
    let r = run(&base, &script).unwrap();
    let out = r["log"][0].as_array().unwrap();
    assert_eq!(out.len(), 2);
    for e in out {
        assert!(e.as_str().unwrap().starts_with("Error: script file read refused"), "{e}");
    }
    // A script given as text has no folder to read from.
    let e = Session::new().execute("script.run", &json!({"source": "$.evalFile('x.jsx');"})).unwrap_err().to_string();
    assert!(e.contains("script file read refused: x.jsx"), "{e}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_directives_in_strings_and_block_comments_are_inert() {
    let mut s = Session::new();
    let source = "#target x\nvar a = '#include \"nope.jsxinc\"';\n/*\n#include \"nope.jsxinc\"\n//@include \"nope.jsxinc\"\n*/\nvar b = 1; // #include \"nope.jsxinc\"\nvar c = 2; #include\nvc.log(a.length + b);";
    let e = s.execute("script.run", &json!({"source": source})).unwrap_err().to_string();
    // `#` after code on a line is not a preprocessor line: the script has a syntax error there.
    assert!(e.contains("line 8"), "{e}");
    let source = source.replace("var c = 2; #include\n", "");
    assert_eq!(s.execute("script.run", &json!({"source": source})).unwrap()["log"], json!([23]));
}

#[test]
fn script_included_error_has_local_file_and_line() {
    let base = folder("errors");
    write(&base.join("main/lib.jsxinc"), "var x = 1;\n\n    throw new Error('boom');\n");
    write(&base.join("main/bad.jsxinc"), "var y = 1;\nvar = 2;\n");
    write(&base.join("main/five.jsxinc"), "1;\n2;\n3;\n4;\n5;");
    write(&base.join("main/sub/thrower.jsx"), "\n\nthrow new Error('deep');");
    let mut s = Session::new();
    let report = failed(&base, "var z = 0;\n#include \"lib.jsxinc\"");
    assert_eq!((report["line"].as_u64(), report["column"].as_u64()), (Some(3), Some(5)), "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("lib.jsxinc"), "{report}");
    let report = failed(&base, "#include \"bad.jsxinc\"");
    assert_eq!((report["kind"].as_str(), report["line"].as_u64()), (Some("syntax"), Some(2)), "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("bad.jsxinc"), "{report}");
    // A line of the script after an include keeps its own number.
    let report = failed(&base, "#include \"five.jsxinc\"\n  throw new Error('after');");
    assert_eq!((report["line"].as_u64(), report["column"].as_u64()), (Some(2), Some(3)), "{report}");
    assert!(report.get("file").is_none(), "{report}");
    let report = failed(&base, "#include \"five.jsxinc\"\nvar = 1;");
    assert_eq!((report["kind"].as_str(), report["line"].as_u64()), (Some("syntax"), Some(2)), "{report}");
    // An included file that ends in a carriage return does not join the script's next line ending.
    write(&base.join("main/cr.jsxinc"), "1;\r2;\r");
    let report = failed(&base, "#include \"cr.jsxinc\"\n\n  throw new Error('after');");
    assert_eq!((report["line"].as_u64(), report["column"].as_u64()), (Some(3), Some(3)), "{report}");
    assert!(report.get("file").is_none(), "{report}");
    let report = failed(&base, "#include \"cr.jsxinc\"\r\nvar = 1;");
    assert_eq!((report["kind"].as_str(), report["line"].as_u64()), (Some("syntax"), Some(2)), "{report}");
    // An uncaught error in a file $.evalFile ran is at that file's line.
    let report = failed(&base, "var a = 1;\n$.evalFile('sub/thrower.jsx');");
    assert_eq!(report["line"], 3, "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("thrower.jsx"), "{report}");
    // So is a file $.evalFile could not run: a syntax error, or a bad preprocessor line.
    write(&base.join("main/sub/broken.jsx"), "var a = 1;\n\nvar = 2;");
    write(&base.join("main/sub/directive.jsx"), "\n#frobnicate 1");
    for (file, line) in [("broken.jsx", 3), ("directive.jsx", 2)] {
        let report = failed(&base, &format!("var a = 1;\n$.evalFile('sub/{file}');"));
        assert_eq!(report["line"], line, "{report}");
        assert!(report["file"].as_str().unwrap().ends_with(file), "{report}");
    }
    // The console's error line names the file.
    let main = base.join("main/main.jsx");
    write(&main, "#include \"lib.jsxinc\"");
    assert!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).is_err());
    let console = s.execute("script.console", &json!({})).unwrap();
    let error = console["lines"].as_array().unwrap().iter().find(|l| l["kind"] == "error").unwrap()["text"].as_str().unwrap().to_string();
    assert!(error.starts_with("main.jsx: ") && error.contains("lib.jsxinc:3:5: Error: boom"), "{error}");
    let _ = std::fs::remove_dir_all(base);
}

/// Pumps the run in the app until it ends or waits at a dialog (`None`).
fn pump(s: &mut Session) -> Option<Result<Value>> {
    for _ in 0..10_000 {
        if s.script_dialog().is_some() {
            return None;
        }
        if let Some(r) = s.script_pump() {
            return Some(r);
        }
    }
    panic!("the run neither ended nor waited");
}

#[test]
fn script_dialogs_in_included_and_evaluated_files() {
    let base = folder("dialogs");
    write(&base.join("main/ask.jsxinc"), "var a = confirm('inc');\n");
    write(&base.join("main/ev.jsx"), "alert('ev'); 7");
    let text = "#include \"ask.jsxinc\"\nvar m = '';\ntry { m = $.evalFile('ev.jsx'); } catch (e) { m = e.message; }\nvc.log([a, m]);";
    let main = base.join("main/main.jsx");
    write(&main, text);
    let path = main.to_string_lossy().to_string();
    // In the app, an included file's dialog waits as the script's own does; one in a file
    // $.evalFile runs cannot wait there and throws an error the script can catch.
    let mut s = Session::new();
    s.script_frames = true;
    assert_eq!(s.execute("script.run", &json!({"path": path})).unwrap()["running"], true);
    assert!(pump(&mut s).is_none());
    assert_eq!(s.script_dialog().unwrap().message, "inc");
    assert!(s.script_answer(crate::script::DialogAnswer::Confirm(true)));
    let r = pump(&mut s).unwrap().unwrap();
    let m = r["log"][0][1].as_str().unwrap_or_default();
    assert_eq!(r["log"][0][0], true, "{r}");
    assert!(m.contains("alert") && m.contains("$.evalFile"), "the error names the call and why: {r}");
    assert_eq!(r["dialogCount"], 1, "{r}");
    // A run with nobody to ask takes both answers in order.
    let mut s = Session::new();
    let answers = json!([{"kind": "confirm", "value": false}, {"kind": "alert", "value": "ok"}]);
    let r = s.execute("script.run", &json!({"path": path, "answers": answers})).unwrap();
    assert_eq!((r["log"].clone(), r["dialogCount"].clone()), (json!([[false, 7]]), json!(2)), "{r}");
    // An error in an included file after its dialog is at that file's line, and the report keeps
    // both the file and the dialog.
    write(&base.join("main/ask.jsxinc"), "var a = confirm('inc');\nif (a) throw new Error('after');\n");
    let mut s = Session::new();
    s.script_frames = true;
    s.execute("script.run", &json!({"path": path})).unwrap();
    assert!(pump(&mut s).is_none());
    assert!(s.script_answer(crate::script::DialogAnswer::Confirm(true)));
    assert!(pump(&mut s).unwrap().is_err());
    let report = s.script_report().unwrap();
    assert_eq!(report["line"], 2, "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("ask.jsxinc"), "{report}");
    assert_eq!(report["dialogs"][0]["answer"], true, "{report}");
    let _ = std::fs::remove_dir_all(base);
}
