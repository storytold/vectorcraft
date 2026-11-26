//! `.mjs` scripts: modules, the modules they import from the script's folder, the refusals of
//! specifiers and paths, the run's limits, and errors at the file they are in.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::*;

/// A fresh folder for test `name`, with a `main` folder inside it for the script.
fn folder(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("vc-script-modules-{name}-{}", std::process::id()));
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

/// Runs `main/<file>` in `base`, with the text `text`.
fn run_file(base: &Path, file: &str, text: &str) -> (Result<Value>, Session) {
    let main = base.join("main").join(file);
    write(&main, text);
    let mut s = Session::new();
    let r = s.execute("script.run", &json!({"path": main.to_string_lossy()}));
    (r, s)
}

fn run(base: &Path, text: &str) -> Result<Value> {
    run_file(base, "main.mjs", text).0
}

/// The report of the run of `main/main.mjs` that failed.
fn failed(base: &Path, text: &str) -> Value {
    let (r, s) = run_file(base, "main.mjs", text);
    assert!(r.is_err(), "the run failed: {r:?}");
    s.script_report().unwrap().clone()
}

fn message(report: &Value) -> &str {
    report["message"].as_str().unwrap_or_default()
}

#[test]
fn script_module_imports_from_its_folder_and_below() {
    let base = folder("imports");
    write(&base.join("main/lib/count.mjs"), "export let n = 0;\nexport function inc() { n++; }\nexport default 'lib';");
    write(
        &base.join("main/lib/use.mjs"),
        "import { inc } from './count.mjs';\nimport name from '../lib/count.mjs';\nexport const used = name;\ninc();",
    );
    let text = "import { n, inc } from './lib/count.mjs';\nimport { used } from './lib/use.mjs';\ninc();\nvc.log([n, used, this === undefined, /main\\.mjs$/.test($.fileName)]);";
    let r = run(&base, text).unwrap();
    // count.mjs ran once: use.mjs and the script share its binding.
    assert_eq!(r["log"], json!([[2, "lib", true, true]]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_two_modules_import_each_other() {
    let base = folder("cycle");
    write(&base.join("main/a.mjs"), "import { b } from './b.mjs';\nexport function a() { return 'a' + b(); }");
    write(&base.join("main/b.mjs"), "import { a } from './a.mjs';\nexport function b() { return 'b'; }\nexport const viaA = () => a();");
    let r = run(&base, "import { a } from './a.mjs';\nimport { viaA } from './b.mjs';\nvc.log([a(), viaA()]);").unwrap();
    assert_eq!(r["log"], json!([["ab", "ab"]]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_path_above_the_script_folder_is_refused() {
    let base = folder("above");
    write(&base.join("outside.mjs"), "export var leaked = 1;");
    let report = failed(&base, "vc.log(1);\nimport { leaked } from '../outside.mjs';");
    assert!(message(&report).contains("module path leads out of the script's folder: '../outside.mjs'"), "{report}");
    assert_eq!((report["line"].as_u64(), report["log"].clone()), (Some(2), json!([])), "{report}");
    // `..` names that stay inside are read by name.
    write(&base.join("main/sub/x.mjs"), "export var x = 'x';");
    let r = run(&base, "import { x } from './sub/../sub/./x.mjs';\nvc.log(x);").unwrap();
    assert_eq!(r["log"], json!(["x"]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_missing_file_is_named() {
    let base = folder("missing");
    write(&base.join("main/a.mjs"), "import './nope.mjs';");
    let report = failed(&base, "import './a.mjs';");
    assert!(message(&report).contains("module file not found: ./nope.mjs"), "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("a.mjs"), "{report}");
    assert_eq!(report["line"], 1);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_specifiers_other_than_relative_paths_are_syntax_errors() {
    let base = folder("specifiers");
    // The specifier as the script writes it, as the error names it, and why it is refused.
    for (written, shown, why) in [
        ("lib", "lib", "must start with './' or '../'"),
        ("lib/x.mjs", "lib/x.mjs", "must start with './' or '../'"),
        ("https://example.com/x.mjs", "https://example.com/x.mjs", "must start with './' or '../'"),
        ("/abs/x.mjs", "/abs/x.mjs", "must start with './' or '../'"),
        ("./C:x.mjs", "./C:x.mjs", "must not contain"),
        ("./a\\u005cb.mjs", "./a\\b.mjs", "must not contain"),
        ("./x.txt", "./x.txt", "must name a .mjs or .js file"),
        ("./x", "./x", "must name a .mjs or .js file"),
    ] {
        let report = failed(&base, &format!("import '{written}';"));
        let expected = format!("SyntaxError: module specifier '{shown}' {why}");
        assert!(message(&report).starts_with(&expected), "{written}: {report}");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_errors_are_at_the_file_they_are_in() {
    let base = folder("errors");
    write(&base.join("main/bad.mjs"), "export var a = 1;\nexport var = 2;");
    let report = failed(&base, "import { a } from './bad.mjs';");
    assert_eq!(report["kind"], "syntax", "{report}");
    assert!(report["file"].as_str().unwrap().ends_with("bad.mjs"), "{report}");
    assert_eq!(report["line"], 2);
    write(&base.join("main/thrower.mjs"), "export var a = 1;\n\nthrow new Error('from thrower');");
    let report = failed(&base, "import { a } from './thrower.mjs';\nvc.log(a);");
    assert!(report["file"].as_str().unwrap().ends_with("thrower.mjs"), "{report}");
    assert_eq!((report["line"].as_u64(), report["log"].clone()), (Some(3), json!([])), "{report}");
    let report = failed(&base, "import { b } from './thrower.mjs';");
    assert!(message(&report).contains("the module './thrower.mjs' has no export named 'b'"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_preprocessor_lines_and_scripts_differ_by_extension() {
    let base = folder("kinds");
    write(&base.join("main/lib.jsxinc"), "var included = 1;");
    // A module has no preprocessor lines.
    let report = failed(&base, "#include \"lib.jsxinc\"\n");
    assert_eq!(report["kind"], "syntax", "{report}");
    // A .js or .jsx script does not read import or export.
    for file in ["main.js", "main.jsx"] {
        let (r, s) = run_file(&base, file, "import { a } from './a.mjs';");
        assert!(r.is_err(), "{file}");
        assert_eq!(s.script_report().unwrap()["kind"], "syntax", "{file}");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_files_count_against_the_include_limits() {
    let base = folder("limits");
    for i in 0..70 {
        write(&base.join(format!("main/m{i}.mjs")), &format!("import './m{}.mjs';", i + 1));
    }
    let report = failed(&base, "import './m0.mjs';");
    assert!(message(&report).contains("limit"), "{report}");
    assert_eq!(report["log"], json!([]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_paths_of_one_file_give_one_module() {
    let base = folder("same");
    write(&base.join("main/once.mjs"), "vc.log('ran');\nexport var v = 1;");
    let mut text = "import { v } from './once.mjs';\nimport { v as w } from './sub/../once.mjs';\n".to_string();
    // Windows file names ignore letter case.
    if cfg!(windows) {
        text.push_str("import { v as u } from './ONCE.mjs';\n");
    }
    let r = run(&base, &text).unwrap();
    assert_eq!(r["log"], json!(["ran"]));
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
fn script_module_link_out_of_the_folder_is_refused() {
    let base = folder("link");
    write(&base.join("outside/x.mjs"), "export var leaked = 1;");
    if !link_folder(&base.join("outside"), &base.join("main").join("away")) {
        eprintln!("skipped: a folder link can't be made here");
        let _ = std::fs::remove_dir_all(base);
        return;
    }
    let report = failed(&base, "import { leaked } from './away/x.mjs';\nvc.log(leaked);");
    assert!(message(&report).contains("script file read refused: ./away/x.mjs"), "{report}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_is_listed_and_taken_by_path() {
    assert!(cmd::scripts::runs("tool.MJS"));
    let base = folder("text");
    let root = base.join("Scripts");
    write(&root.join("tool.mjs"), "export var a = 1;");
    let mut s = Session::new();
    s.attach_scripts_dir(root.clone());
    let listed = s.execute("script.listInstalled", &json!({})).unwrap();
    assert_eq!(listed["scripts"], json!([{"path": root.join("tool.mjs").to_string_lossy(), "label": "tool", "folders": []}]));
    // A script given as text is not a module, whatever its name.
    let mut s = Session::new();
    let r = s.execute("script.run", &json!({"source": "export var a = 1;", "name": "t.mjs"}));
    assert!(r.is_err());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_is_chosen_by_the_module_param() {
    let mut s = Session::new();
    let r = s.execute("script.run", &json!({"source": "export var a = 1;\nvc.log([a, this === undefined]);", "module": true})).unwrap();
    assert_eq!(r["log"], json!([[1, true]]));
    // A source run as a module imports no files.
    let r = s.execute("script.run", &json!({"source": "import { b } from './b.mjs';", "module": true}));
    assert!(r.is_err());
    assert!(message(s.script_report().unwrap()).contains("./b.mjs"), "{:?}", s.script_report());
    let r = s.execute("script.run", &json!({"source": "export var a = 1;", "module": "yes"}));
    assert!(matches!(r, Err(EngineError::BadParams { .. })), "{r:?}");
    // `module: false` runs a .mjs file as a script, where `export` is a syntax error.
    let base = folder("param");
    let main = base.join("main").join("main.mjs");
    write(&main, "export var a = 1;");
    let r = s.execute("script.run", &json!({"path": main.to_string_lossy(), "module": false}));
    assert!(r.is_err());
    assert_eq!(s.script_report().unwrap()["kind"], "syntax");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_read_again_through_another_path_is_not_counted_again() {
    let base = folder("budget");
    // 7 MiB, under the 8 MiB a file may have; five reads of it would pass the 32 MiB of a run.
    let big = format!("export var v = 1;\n//{}", "x".repeat(7 * 1024 * 1024));
    write(&base.join("main/lib/big.mjs"), &big);
    let mut text = String::new();
    for i in 0..5 {
        if !link_folder(&base.join("main/lib"), &base.join(format!("main/l{i}"))) {
            eprintln!("skipped: a folder link can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        text.push_str(&format!("import {{ v as v{i} }} from './l{i}/big.mjs';\n"));
    }
    text.push_str("vc.log(v0 + v4);");
    let r = run(&base, &text);
    assert_eq!(r.map(|r| r["log"].clone()).ok(), Some(json!([2])));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_imports_are_read_from_the_folder_the_file_is_in() {
    let base = folder("realdir");
    write(&base.join("main/real/user.mjs"), "import { d } from './dep.mjs';\nexport const got = d;");
    write(&base.join("main/real/dep.mjs"), "export const d = 'real';");
    write(&base.join("main/other/dep.mjs"), "export const d = 'other';");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(base.join("main/real/user.mjs"), base.join("main/other/user.mjs")).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(base.join("main/real/user.mjs"), base.join("main/other/user.mjs")).is_ok();
    #[cfg(not(any(unix, windows)))]
    let linked = false;
    if !linked {
        eprintln!("skipped: a file link can't be made here");
        let _ = std::fs::remove_dir_all(base);
        return;
    }
    let r = run(&base, "import { got } from './other/user.mjs';\nvc.log(got);").unwrap();
    assert_eq!(r["log"], json!(["real"]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_eval_file_reads_beside_the_module_that_calls_it() {
    let base = folder("evalfile");
    write(&base.join("main/helper.jsx"), "'top'");
    write(&base.join("main/sub/helper.jsx"), "'sub'");
    write(&base.join("main/sub/m.mjs"), "export const got = $.evalFile('helper.jsx');");
    let r = run(&base, "import { got } from './sub/m.mjs';\nvc.log([got, $.evalFile('helper.jsx')]);").unwrap();
    assert_eq!(r["log"], json!([["sub", "top"]]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn script_module_function_of_the_script_reads_beside_the_script_when_another_file_calls_it() {
    let base = folder("evalfile-entry");
    write(&base.join("main/helper.jsx"), "'top'");
    write(&base.join("main/sub/helper.jsx"), "'sub'");
    // The file in `sub` calls the script's function while it is the file being run.
    write(&base.join("main/sub/caller.jsx"), "$.global.load()");
    let text = "$.global.load = function () { return $.evalFile('helper.jsx'); };\nvc.log($.evalFile('sub/caller.jsx'));";
    let r = run(&base, text).unwrap();
    assert_eq!(r["log"], json!(["top"]));
    let _ = std::fs::remove_dir_all(base);
}
