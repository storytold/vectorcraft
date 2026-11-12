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
        ("#targetengine \"session\"", "#targetengine must come before the script's code and #include lines"),
        ("#engine \"main\"", "unsupported script directive: #engine"),
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

/// The script's `ro`, with a read-only `x`.
const READ_ONLY: &str = "var ro = {};\nObject.defineProperty(ro, 'x', {value: 1});\n";

#[test]
fn strict_on_runs_and_read_only_write_throws() {
    let mut s = Session::new();
    for head in ["#strict on\n", "//@strict on\n", "#strict ON\n", "#strict off\n#strict on\n"] {
        let source = format!("{head}{READ_ONLY}vc.log(1);\n  ro.x = 2;\nvc.log(2);");
        let e = s.execute("script.run", &json!({"source": source})).unwrap_err().to_string();
        assert!(e.contains("TypeError: cannot assign to property 'x': it is read-only"), "{head}: {e}");
        let report = s.script_report().unwrap();
        assert_eq!(report["kind"], "thrown", "{head}");
        assert_eq!(report["log"], json!([1]), "{head}: the script ran up to the write");
        let line = head.lines().count() as u64 + 4;
        assert_eq!((report["line"].as_u64(), report["column"].as_u64()), (Some(line), Some(3)), "{head}: {report}");
    }
    // The error can be caught, and every kind of assignment is checked.
    let source = format!(
        "#strict on\n{READ_ONLY}var out = [];\nfunction t(f) {{ try {{ f(); out.push('wrote'); }} catch (e) {{ out.push(e.name); }} }}\nt(function () {{ ro.x = 2; }});\nt(function () {{ ro['x'] += 1; }});\nt(function () {{ ro.x++; }});\nt(function () {{ undefined = 1; }});\nt(function () {{ [].concat.length = 0; }});\nvc.log(out);\nvc.log(ro.x);"
    );
    let r = s.execute("script.run", &json!({"source": source})).unwrap();
    assert_eq!(r["log"], json!([["TypeError", "TypeError", "TypeError", "TypeError", "TypeError"], 1]));
    // Without the line, or with `#strict off` last, the write is ignored as before.
    for head in ["", "#strict off\n", "#strict on\n#strict off\n"] {
        let r = s.execute("script.run", &json!({"source": format!("{head}{READ_ONLY}ro.x = 2;\nvc.log(ro.x);")})).unwrap();
        assert_eq!(r["log"], json!([1]), "{head}");
    }
}

#[test]
fn strict_on_rejects_getter_only_and_nonextensible_additions() {
    let mut s = Session::new();
    let source = "#strict on\nvar out = [];\nfunction t(f) { try { f(); out.push('wrote'); } catch (e) { out.push(String(e)); } }\n\
        var g = {get v() { return 1; }}, set = {set v(x) { this.got = x; }};\n\
        t(function () { g.v = 2; });\n\
        t(function () { Object.preventExtensions({}).n = 1; });\n\
        t(function () { Object.seal({a: 1}).n = 1; });\n\
        t(function () { Object.freeze({a: 1}).a = 2; });\n\
        t(function () { vc.extra = 1; });\n\
        var open = {};\nt(function () { open.n = 1; });\nt(function () { set.v = 3; });\nt(function () { Object.seal({a: 1}).a = 2; });\n\
        vc.log(out);\nvc.log(open.n);\nvc.log(set.got);";
    let r = s.execute("script.run", &json!({"source": source})).unwrap();
    assert_eq!(
        r["log"],
        json!([
            [
                "TypeError: cannot assign to property 'v': it has only a getter",
                "TypeError: cannot assign to property 'n': the object is not extensible",
                "TypeError: cannot assign to property 'n': the object is not extensible",
                "TypeError: cannot assign to property 'a': it is read-only",
                "TypeError: cannot assign to property 'extra': the object is not extensible",
                "wrote",
                "wrote",
                "wrote"
            ],
            1,
            3
        ])
    );
    let r = s.execute("script.run", &json!({"source": source.replacen("#strict on", "#strict off", 1)})).unwrap();
    assert_eq!(r["log"][0], json!(["wrote", "wrote", "wrote", "wrote", "wrote", "wrote", "wrote", "wrote"]), "off, nothing is refused");
}

#[test]
fn strict_on_does_not_enable_es5_early_errors() {
    let mut s = Session::new();
    let source = "#strict on\nwith ({w: 1}) { vc.log(w); }\nfunction f(a, a) { return a; }\nvc.log(f(1, 2));\nvc.log(010);\nundeclared = 4;\nvc.log(undeclared);\n\
        vc.log((function () { return typeof this; })());\nvc.log(delete Object.prototype);\nvar eval2 = 5;\nvc.log((function () { arguments = 6; return arguments; })());";
    let r = s.execute("script.run", &json!({"source": source})).unwrap();
    assert_eq!(r["log"], json!([1, 2, 8, 4, "object", false, 6]));
    // 'use strict' still makes the code strict mode code.
    let e = s.execute("script.run", &json!({"source": "#strict on\n'use strict';\nundeclared2 = 1;"})).unwrap_err().to_string();
    assert!(e.contains("ReferenceError"), "{e}");
    let e = s.execute("script.run", &json!({"source": "#strict on\n'use strict';\nwith ({}) {}"})).unwrap_err().to_string();
    assert_eq!(s.script_report().unwrap()["kind"], "syntax", "{e}");
}

#[test]
fn strict_on_in_include_controls_expanded_run() {
    let base = folder("strict");
    write(&base.join("main/on.jsxinc"), "#strict on\nvar inc = 1;");
    write(&base.join("main/off.jsxinc"), "//@strict off\n");
    write(&base.join("main/writes.jsxinc"), &format!("{READ_ONLY}\n  ro.x = 2;\n"));
    // An included file's line sets the checks for the script's own code too.
    let report = failed(&base, &format!("#include \"on.jsxinc\"\n{READ_ONLY}ro.x = 2;"));
    assert!(report["message"].as_str().unwrap().contains("it is read-only"), "{report}");
    assert_eq!(report["line"], 4, "{report}");
    assert!(report.get("file").is_none(), "{report}");
    // A write in an included file is reported at its own line.
    let report = failed(&base, "#strict on\n#include \"writes.jsxinc\"");
    assert!(report["message"].as_str().unwrap().contains("it is read-only"), "{report}");
    assert_eq!((report["line"].as_u64(), report["column"].as_u64()), (Some(4), Some(3)), "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("writes.jsxinc"), "{report}");
    // The last line of the expanded run wins, wherever it is.
    let r = run(&base, &format!("#strict on\n#include \"off.jsxinc\"\n{READ_ONLY}ro.x = 2;\nvc.log(ro.x);")).unwrap();
    assert_eq!(r["log"], json!([1]));
    let report = failed(&base, &format!("#include \"off.jsxinc\"\n#strict on\n{READ_ONLY}ro.x = 2;"));
    assert!(report["message"].as_str().unwrap().contains("it is read-only"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn strict_mode_in_eval_file_is_scoped() {
    let base = folder("strict-eval");
    write(
        &base.join("main/on.jsx"),
        &format!(
            "#strict on\n{READ_ONLY}function w(o) {{ o.x = 2; }}\nvar inner;\ntry {{ ro.x = 2; inner = 'wrote'; }} catch (e) {{ inner = e.name; }}\n"
        ),
    );
    write(&base.join("main/off.jsx"), &format!("#strict off\n{READ_ONLY}ro.x = 2;\n'off ran';"));
    write(&base.join("main/plain.jsx"), &format!("{READ_ONLY}ro.x = 2;\n'plain ran';"));
    // A file's own `#strict on` holds for its code, its functions included, and not the caller's.
    let r = run(
        &base,
        "$.evalFile('on.jsx');\nvar after;\ntry { ro.x = 3; after = 'ignored'; } catch (e) { after = e.name; }\nvar called;\ntry { w(ro); called = 'wrote'; } catch (e) { called = e.name; }\nvc.log([inner, after, called]);",
    )
    .unwrap();
    assert_eq!(r["log"], json!([["TypeError", "ignored", "TypeError"]]));
    // A file without the line takes the caller's checks, and its own `#strict off` turns them off.
    let r = run(&base, "#strict on\nvc.log($.evalFile('off.jsx'));\nvar e1;\ntry { ro.x = 2; } catch (e) { e1 = e.name; }\nvc.log(e1);").unwrap();
    assert_eq!(r["log"], json!(["off ran", "TypeError"]), "the caller keeps its own checks");
    let report = failed(&base, "#strict on\n$.evalFile('plain.jsx');");
    assert!(report["message"].as_str().unwrap().contains("it is read-only"), "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("plain.jsx"), "{report}");
    assert_eq!(report["line"], 3, "{report}");
    assert_eq!(run(&base, "vc.log($.evalFile('plain.jsx'));").unwrap()["log"], json!(["plain ran"]));
    let _ = std::fs::remove_dir_all(base);
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

/// The line, column and file name of a failed run's report, and its console error line (the only
/// line of a run without output that starts with the script's name).
fn place(report: &Value) -> (u64, u64, Option<&str>, &str) {
    let file = report.get("file").map(|f| f.as_str().unwrap().rsplit(['/', '\\']).next().unwrap());
    let lines = report["console"]["lines"].as_array().unwrap();
    let error = lines.iter().filter_map(Value::as_str).find(|l| l.starts_with("main.jsx")).unwrap();
    (report["line"].as_u64().unwrap(), report["column"].as_u64().unwrap(), file, error)
}

#[test]
fn script_eval_file_error_caught_and_thrown_again_is_at_the_new_throw() {
    let base = folder("rethrow");
    write(&base.join("main/sub/broken.jsx"), "var a = 1;\n\nvar = 2;");
    write(&base.join("main/sub/directive.jsx"), "\n#frobnicate 1");
    write(&base.join("main/sub/rethrow.jsx"), "var r = 1;\n  throw saved;");
    write(&base.join("main/sub/outer.jsx"), "var q = 1;\n$.evalFile('broken.jsx');");
    write(&base.join("main/sub/outer2.jsx"), "try { $.evalFile('broken.jsx'); } catch (e) {\n\n    throw e;\n}");
    let caught = "var saved;\ntry { $.evalFile('sub/broken.jsx'); } catch (e) { saved = e; }\n";
    let cases = [
        // Thrown again in the script, or in another file $.evalFile runs.
        (format!("{caught}var b = 2;\n  throw saved;"), (4, 3, None)),
        (format!("{caught}$.evalFile('sub/rethrow.jsx');"), (2, 3, Some("rethrow.jsx"))),
        // A new error whose message holds the failure's text, or is the same text.
        (format!("{caught}throw new Error(saved.message);"), (3, 1, None)),
        (format!("{caught}throw new SyntaxError(saved.message);"), (3, 1, None)),
        // A second failure, caught, does not move the first one.
        (format!("{caught}try {{ $.evalFile('sub/directive.jsx'); }} catch (e) {{}}\n throw saved;"), (4, 2, None)),
        // A second failure, uncaught, is at its own file's place.
        (format!("{caught}$.evalFile('sub/directive.jsx');"), (2, 1, Some("directive.jsx"))),
        // An uncaught failure is at the file's place, through a finally block and from a nested file.
        ("var a = 1;\ntry { $.evalFile('sub/broken.jsx'); } finally { a = 2; }".to_string(), (3, 5, Some("broken.jsx"))),
        ("$.evalFile('sub/outer.jsx');".to_string(), (3, 5, Some("broken.jsx"))),
        // Caught in a nested file and thrown again there.
        ("$.evalFile('sub/outer2.jsx');".to_string(), (3, 5, Some("outer2.jsx"))),
        // A failure caught in a run after one that failed with it, and a different error.
        ("try { $.evalFile('sub/broken.jsx'); } catch (e) {}\nthrow new Error('other');".to_string(), (2, 1, None)),
    ];
    let mut s = Session::new();
    let main = base.join("main/main.jsx");
    for (text, (line, column, file)) in cases {
        write(&main, &text);
        assert!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).is_err(), "{text}");
        let report = s.script_report().unwrap().clone();
        let (l, c, f, error) = place(&report);
        assert_eq!((report["kind"].as_str(), l, c, f), (Some("thrown"), line, column, file), "{text}\n{report}");
        let at = match file {
            Some(f) => format!("{f}:{line}:{column}: "),
            None => format!("main.jsx:{line}:{column}: "),
        };
        assert!(error.starts_with("main.jsx") && error.contains(&at), "{error}");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_error_that_passes_through_a_finally_block_is_at_its_throw() {
    let base = folder("finally");
    let main = base.join("main/main.jsx");
    let mut s = Session::new();
    for text in ["try {\n  throw new Error('x');\n} finally {\n  var z=1;\n}", "try {\n  null();\n} finally {\n  var z=1;\n}"] {
        write(&main, text);
        assert!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).is_err(), "{text}");
        let report = s.script_report().unwrap().clone();
        let (l, c, f, error) = place(&report);
        assert_eq!((report["kind"].as_str(), l, c, f), (Some("thrown"), 2, 3, None), "{text}\n{report}");
        assert!(error.contains("main.jsx:2:3: "), "{error}");
    }
    let _ = std::fs::remove_dir_all(base);
}
