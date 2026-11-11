//! The ES5 parse subset of test262, the ECMAScript conformance suite, when `cargo xtask test262`
//! has fetched it (it sets `VECTORCRAFT_TEST262_DIR`; without it the test looks in
//! `corpus/test262/` and skips when the suite is absent).
//!
//! The subset is every test under `test/language/` that names the ES5 section it was written for
//! (`es5id:`) and has no `features:` tag (the tags name syntax and built-ins later than ES5) and no
//! `module` flag. The other tests without a feature tag (written for later editions, many of them
//! using later syntax) run too, and their counts are printed apart. A test whose metadata says
//! `negative: phase: parse` must give a `SyntaxError`; every other test must parse. Each test runs
//! as sloppy and as strict mode code (with `"use strict";` before it, as the suite's
//! `INTERPRETING.md` says), unless its `onlyStrict`, `noStrict` or `raw` flag says otherwise. The
//! test prints the counts and writes each failure to `failures.txt` beside the suite; it fails only
//! when the suite cannot be read.
//!
//! A second test runs the same ES5 tests through the interpreter, each in a new realm after the
//! suite's `assert.js`, `sta.js` and the files its `includes:` names: every test that must parse
//! and uses only the built-ins the interpreter has (those whose code, or an included file's, uses a
//! missing global or method are counted as skipped, as are `async` tests). A test
//! passes when it completes, or, for `negative: phase: runtime`, when it throws an object whose
//! `constructor` is the global its `type:` names. Its counts are printed apart from the parse
//! counts, and its failures go to `run-failures.txt`.
//!
//! A third test runs the tests of the ES5 built-ins the same way: those under the folders
//! [`ES5_BUILT_INS`] names that have an `es5id:` (or, under `annexB/`, any edition's id) and no
//! `features:` tag. Its failures go to `built-ins-failures.txt`. A fourth test runs the tests under
//! `test/built-ins/String`, `Number`, `RegExp` and `Date` the same way, counting the tests that name
//! an ES5 section apart from the other tests without a feature tag; its failures go to
//! `builtins-failures.txt`. The interpreter tests fail when the interpreter panics, when a harness
//! or test file cannot be read, or when no ES5 test runs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use vectorcraft_js::{JsValue, Limits, Options, Punct, Realm, RunErrorKind, Token, TokenKind, parse, tokenize};

/// A test file's metadata, from its `/*--- … ---*/` block.
#[derive(Default)]
struct Meta {
    features: Vec<String>,
    flags: Vec<String>,
    includes: Vec<String>,
    negative_phase: Option<String>,
    /// The error constructor a negative test expects (`negative: type:`).
    negative_type: Option<String>,
    /// The test names the ES5 section it was written for (`es5id:`).
    es5: bool,
}

fn suite_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VECTORCRAFT_TEST262_DIR") {
        return Some(PathBuf::from(dir));
    }
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/test262");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(corpus).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.pop()
}

/// The `.js` files under `dir`, sorted, with the `_FIXTURE` helper files left out.
fn test_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "js") && !path.to_string_lossy().contains("_FIXTURE") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// A YAML flow list (`[a, b]`) or the block list (`- a` lines) after a key.
fn list(value: &str, lines: &mut std::iter::Peekable<std::str::Lines<'_>>) -> Vec<String> {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        return inner.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    let mut out = Vec::new();
    while let Some(item) = lines.peek().and_then(|l| l.trim().strip_prefix("- ")) {
        out.push(item.trim().to_string());
        lines.next();
    }
    out
}

fn meta(source: &str) -> Meta {
    let mut meta = Meta::default();
    let Some(block) = source.split_once("/*---").and_then(|(_, rest)| rest.split_once("---*/")).map(|(block, _)| block) else {
        return meta;
    };
    let mut lines = block.lines().peekable();
    let mut in_negative = false;
    while let Some(line) = lines.next() {
        let indented = line.starts_with(' ');
        let trimmed = line.trim();
        if !indented {
            in_negative = false;
        }
        if let Some(v) = trimmed.strip_prefix("features:").filter(|_| !indented) {
            meta.features = list(v, &mut lines);
        } else if let Some(v) = trimmed.strip_prefix("flags:").filter(|_| !indented) {
            meta.flags = list(v, &mut lines);
        } else if let Some(v) = trimmed.strip_prefix("includes:").filter(|_| !indented) {
            meta.includes = list(v, &mut lines);
        } else if let Some(v) = trimmed.strip_prefix("type:").filter(|_| in_negative) {
            meta.negative_type = Some(v.trim().to_string());
        } else if trimmed.starts_with("es5id:") && !indented {
            meta.es5 = true;
        } else if trimmed == "negative:" && !indented {
            in_negative = true;
        } else if let Some(v) = trimmed.strip_prefix("phase:").filter(|_| in_negative) {
            meta.negative_phase = Some(v.trim().to_string());
        }
    }
    meta
}

#[test]
fn es5_parse_subset() {
    let Some(dir) = suite_dir() else {
        println!("test262: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let language = dir.join("test").join("language");
    assert!(language.is_dir(), "{} is not a test262 checkout", dir.display());
    let files = test_files(&language);
    assert!(!files.is_empty(), "no tests under {}", language.display());
    // VECTORCRAFT_TEST262_VERBOSE=1 prints each test before it is parsed.
    let verbose = std::env::var_os("VECTORCRAFT_TEST262_VERBOSE").is_some();

    let (mut positive, mut negative, mut with_features, mut modules) = (0usize, 0usize, 0usize, 0usize);
    let (mut passed, mut failed) = (0usize, 0usize);
    // The tests without a feature tag that name no ES5 section: files, runs passed and failed.
    let (mut later, mut later_passed, mut later_failed) = (0usize, 0usize, 0usize);
    let mut failures = String::new();
    let mut later_failures = String::new();
    for path in &files {
        let source = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
        let meta = meta(&source);
        if !meta.features.is_empty() {
            with_features += 1;
            continue;
        }
        if meta.flags.iter().any(|f| f == "module") || meta.negative_phase.as_deref() == Some("resolution") {
            modules += 1;
            continue;
        }
        let expect_error = meta.negative_phase.as_deref() == Some("parse");
        if !meta.es5 {
            later += 1;
        } else if expect_error {
            negative += 1;
        } else {
            positive += 1;
        }
        let has = |flag: &str| meta.flags.iter().any(|f| f == flag);
        let modes: &[bool] = if has("raw") || has("noStrict") {
            &[false]
        } else if has("onlyStrict") {
            &[true]
        } else {
            &[false, true]
        };
        for &strict in modes {
            if verbose {
                eprintln!("{} ({})", path.display(), if strict { "strict" } else { "sloppy" });
            }
            let text = if strict { format!("\"use strict\";\n{source}") } else { source.clone() };
            let result = parse(&text, &Options::default());
            let (pass, fail, list) =
                if meta.es5 { (&mut passed, &mut failed, &mut failures) } else { (&mut later_passed, &mut later_failed, &mut later_failures) };
            if result.is_err() == expect_error {
                *pass += 1;
            } else {
                *fail += 1;
                let rel = path.strip_prefix(&dir).unwrap_or(path).display().to_string().replace('\\', "/");
                let mode = if strict { "strict" } else { "sloppy" };
                let outcome = match &result {
                    Ok(_) => "parsed; a SyntaxError was expected".to_string(),
                    Err(e) => e.to_string(),
                };
                list.push_str(&format!("{rel} ({mode}): {outcome}\n"));
            }
        }
    }
    let report = dir.join("failures.txt");
    std::fs::write(&report, format!("# ES5 subset\n{failures}\n# Other tests without a feature tag\n{later_failures}")).unwrap();
    println!(
        "test262 ES5 parse subset ({}): {} tests ({positive} that must parse, {negative} that must give a SyntaxError), \
         {passed} runs passed, {failed} failed. Other tests without a feature tag: {later} tests, {later_passed} runs passed, \
         {later_failed} failed. Skipped {with_features} tests with feature tags and {modules} module tests. Failures: {}",
        dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        positive + negative,
        report.display()
    );
}

/// Globals and method names the interpreter does not provide yet: a test that names one, in its
/// own source or in a harness file it includes, is skipped.
const MISSING_BUILTINS: &str = "String Number Boolean Date RegExp print $262 $DONE Symbol Proxy Reflect Promise Map Set WeakMap \
    WeakSet ArrayBuffer DataView Int8Array Uint8Array Float64Array charAt charCodeAt fromCharCode substring substr split replace \
    match search exec test toUpperCase toLowerCase trim toFixed toPrecision toExponential now";

/// The directories under `test/built-ins/` and `test/annexB/built-ins/` that hold tests of the
/// ES5 built-ins (ECMA-262 5.1 sections 15.1 to 15.12 and Annex B.2).
const ES5_BUILT_INS: &[&str] = &[
    "built-ins/Object",
    "built-ins/Function",
    "built-ins/Array",
    "built-ins/Error",
    "built-ins/NativeErrors",
    "built-ins/Math",
    "built-ins/JSON",
    "built-ins/global",
    "built-ins/Infinity",
    "built-ins/NaN",
    "built-ins/undefined",
    "built-ins/parseInt",
    "built-ins/parseFloat",
    "built-ins/isNaN",
    "built-ins/isFinite",
    "built-ins/encodeURI",
    "built-ins/encodeURIComponent",
    "built-ins/decodeURI",
    "built-ins/decodeURIComponent",
    "annexB/built-ins/escape",
    "annexB/built-ins/unescape",
];

/// The names in [`MISSING_BUILTINS`] that `String`, `Number`, `RegExp` and `Date` provide.
const TEXT_BUILTINS: &str = "String Number Date RegExp charAt charCodeAt fromCharCode substring substr split replace match search exec test \
    toUpperCase toLowerCase trim toFixed toPrecision toExponential now";

/// String methods whose names `Array.prototype` also uses: provided for the tests under
/// `built-ins/String/`, and still missing elsewhere, where the name usually means the array method.
const STRING_METHODS_SHARED_WITH_ARRAY: &str = "indexOf lastIndexOf concat slice";

/// The names in [`MISSING_BUILTINS`] the interpreter still lacks, without those in each of
/// `provided`.
fn missing_builtins<'a>(provided: &[&str]) -> HashSet<&'a str> {
    let provided: HashSet<&str> = provided.iter().flat_map(|p| p.split_whitespace()).collect();
    MISSING_BUILTINS.split_whitespace().filter(|name| !provided.contains(name)).collect()
}

/// The code of `sources` (a test and the harness files it includes) uses a missing built-in, or
/// one of them does not parse as ES5 (the tests that use `let` and `const`). A name counts as an
/// identifier or a property name, as a string literal that is the name alone (a property key, as
/// in `this["Map"]`), and as a word in any string literal when one of the sources can run a string
/// as code. Comments, and strings that only mention the name in a message, do not count.
fn needs_missing_builtin(sources: &[&str], missing: &HashSet<&str>) -> bool {
    let mut lists = Vec::with_capacity(sources.len());
    for source in sources {
        let Ok(tokens) = tokenize(source, &Options::default()) else { return true };
        lists.push(tokens);
    }
    let strings_are_code = lists.iter().any(|tokens| runs_strings_as_code(tokens));
    lists.iter().flatten().any(|t| match &t.kind {
        TokenKind::Ident(name) => missing.contains(name.as_str()),
        TokenKind::String(units) => {
            let text = String::from_utf16_lossy(units);
            missing.contains(text.as_str())
                || (strings_are_code && text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')).any(|word| missing.contains(word)))
        }
        _ => false,
    })
}

/// The tokens can run a string as code: they name `eval`, `Function` or a `constructor` property,
/// as an identifier or as a string key (`this["eval"]`), called directly or through an alias.
/// `Function.prototype` alone does not count.
fn runs_strings_as_code(tokens: &[Token]) -> bool {
    const RUNNERS: [&str; 3] = ["eval", "Function", "constructor"];
    tokens.iter().enumerate().any(|(i, t)| match &t.kind {
        TokenKind::String(units) => RUNNERS.iter().any(|r| r.encode_utf16().eq(units.iter().copied())),
        TokenKind::Ident(name) if name == "eval" || name == "constructor" => true,
        TokenKind::Ident(name) if name == "Function" => {
            let prototype = tokens.get(i + 1).is_some_and(|d| d.is_punct(Punct::Dot))
                && tokens.get(i + 2).is_some_and(|p| matches!(&p.kind, TokenKind::Ident(p) if p == "prototype"));
            !prototype
        }
        _ => false,
    })
}

/// Runs one test in a new realm; `Err` describes a failure.
fn run_one(script: &str, negative_type: Option<&str>) -> Result<(), String> {
    let mut realm = Realm::with_options(Options::default(), Limits { fuel: 20_000_000, heap_bytes: 256 << 20, ..Limits::default() });
    match (realm.run(script), negative_type) {
        (Ok(_), None) => Ok(()),
        (Ok(_), Some(t)) => Err(format!("completed; a {t} was expected")),
        (Err(e), Some(t)) if e.kind == RunErrorKind::Thrown => {
            realm.bind_last_thrown("__thrown");
            match realm.run(&format!("typeof __thrown === 'object' && __thrown !== null && __thrown.constructor === {t}")) {
                Ok(JsValue::Bool(true)) => Ok(()),
                _ => Err(format!("threw {} (line {}); a {t} was expected", e.message, e.line)),
            }
        }
        (Err(e), _) => Err(format!("{:?}: {} (line {}, column {})", e.kind, e.message, e.line, e.column)),
    }
}

#[test]
fn es5_language_subset_runs() {
    let Some(dir) = suite_dir() else {
        println!("test262 interpreter: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    // Each test runs on a thread with a stack large enough for the nested runs the re-entry limit
    // allows in an unoptimized build.
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_language_subset(&dir)).unwrap();
    handle.join().unwrap();
}

/// The counts of one run of a set of tests through the interpreter.
#[derive(Default)]
struct RunCounts {
    tests: usize,
    skipped: usize,
    passed: usize,
    failed: usize,
    panicked: usize,
    failures: String,
}

impl RunCounts {
    fn summary(&self) -> String {
        format!(
            "{} tests that need only the built-ins present, {} runs passed, {} failed ({} panicked), {} skipped",
            self.tests, self.passed, self.failed, self.panicked, self.skipped
        )
    }
}

fn run_language_subset(dir: &Path) {
    let language = dir.join("test").join("language");
    assert!(language.is_dir(), "{} is not a test262 checkout", dir.display());
    let missing = missing_builtins(&[TEXT_BUILTINS]);
    let (es5, _) = run_tests(dir, &language, &missing, false);
    assert!(es5.tests > 0, "no ES5 tests ran under {}", language.display());
    let report = dir.join("run-failures.txt");
    if let Err(e) = std::fs::write(&report, &es5.failures) {
        println!("could not write {}: {e}", report.display());
    }
    println!(
        "test262 ES5 language subset through the interpreter: {} tests that need only the built-ins present, {} runs passed, \
         {} failed ({} panicked). Skipped {} tests that need other built-ins, use syntax after ES5 or are asynchronous. Failures: {}",
        es5.tests,
        es5.passed,
        es5.failed,
        es5.panicked,
        es5.skipped,
        report.display()
    );
    assert_eq!(es5.panicked, 0, "the interpreter panicked; see {}", report.display());
}

#[test]
fn es5_built_ins_subset_runs() {
    let Some(dir) = suite_dir() else {
        println!("test262 built-ins: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_built_ins(&dir)).unwrap();
    handle.join().unwrap();
}

/// Runs the tests under the folders [`ES5_BUILT_INS`] names, prints the counts and writes the
/// failures to `built-ins-failures.txt` beside the suite. The Annex B tests name the edition they
/// were written for with `esid:`, so they run without an `es5id:`.
fn run_built_ins(dir: &Path) {
    let missing = missing_builtins(&[TEXT_BUILTINS]);
    let mut all = RunCounts::default();
    for folder in ES5_BUILT_INS {
        let root = dir.join("test").join(folder);
        assert!(root.is_dir(), "{} is not a test262 checkout", dir.display());
        let (es5, later) = run_tests(dir, &root, &missing, folder.starts_with("annexB"));
        for counts in [es5, later] {
            all.tests += counts.tests;
            all.skipped += counts.skipped;
            all.passed += counts.passed;
            all.failed += counts.failed;
            all.panicked += counts.panicked;
            all.failures.push_str(&counts.failures);
        }
    }
    assert!(all.tests > 0, "no ES5 built-ins tests ran under {}", dir.display());
    let report = dir.join("built-ins-failures.txt");
    if let Err(e) = std::fs::write(&report, &all.failures) {
        println!("could not write {}: {e}", report.display());
    }
    println!("test262 ES5 built-ins subset through the interpreter: {}. Failures: {}", all.summary(), report.display());
    assert_eq!(all.panicked, 0, "the interpreter panicked; see {}", report.display());
}

#[test]
fn es5_text_builtins_run() {
    let Some(dir) = suite_dir() else {
        println!("test262 text built-ins: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_text_builtins(&dir)).unwrap();
    handle.join().unwrap();
}

/// Runs the tests under `test/built-ins/` for `String`, `Number`, `RegExp` and `Date`: those that
/// name an ES5 section and the other tests without a feature tag, counted apart.
fn run_text_builtins(dir: &Path) {
    let mut report = String::new();
    let mut panicked = 0;
    for name in ["String", "Number", "RegExp", "Date"] {
        let root = dir.join("test").join("built-ins").join(name);
        assert!(root.is_dir(), "{} is not a test262 checkout", dir.display());
        let missing =
            if name == "String" { missing_builtins(&[TEXT_BUILTINS, STRING_METHODS_SHARED_WITH_ARRAY]) } else { missing_builtins(&[TEXT_BUILTINS]) };
        let (es5, later) = run_tests(dir, &root, &missing, true);
        assert!(es5.tests > 0, "no ES5 tests ran under {}", root.display());
        println!("test262 built-ins/{name}, ES5 tests: {}. Other tests without a feature tag: {}.", es5.summary(), later.summary());
        report.push_str(&format!("# built-ins/{name}, ES5 tests\n{}\n# built-ins/{name}, other tests\n{}\n", es5.failures, later.failures));
        panicked += es5.panicked + later.panicked;
    }
    let path = dir.join("builtins-failures.txt");
    if let Err(e) = std::fs::write(&path, &report) {
        println!("could not write {}: {e}", path.display());
    }
    println!("test262 built-ins failures: {}", path.display());
    assert_eq!(panicked, 0, "the interpreter panicked; see {}", path.display());
}

/// Runs the tests under `root` that have no feature tag and are not module, asynchronous or
/// negative parse tests, each in a new realm after the harness. Returns the counts for the tests
/// that name an ES5 section and, when `later` is set, for the others (otherwise those are left out).
fn run_tests(dir: &Path, root: &Path, missing: &HashSet<&str>, later: bool) -> (RunCounts, RunCounts) {
    let harness_dir = dir.join("harness");
    let read = |p: &Path| {
        let bytes = std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display()));
        assert!(bytes.is_ok(), "{}", bytes.as_ref().err().map_or("", String::as_str));
        String::from_utf8_lossy(&bytes.unwrap_or_default()).into_owned()
    };
    let prelude = format!("{}\n{}\n", read(&harness_dir.join("assert.js")), read(&harness_dir.join("sta.js")));
    let verbose = std::env::var_os("VECTORCRAFT_TEST262_VERBOSE").is_some();
    let (mut es5_counts, mut later_counts) = (RunCounts::default(), RunCounts::default());
    for path in test_files(root) {
        let source = read(&path);
        let meta = meta(&source);
        let negative_parse = matches!(meta.negative_phase.as_deref(), Some("parse" | "resolution"));
        if (!meta.es5 && !later) || !meta.features.is_empty() || negative_parse || meta.flags.iter().any(|f| f == "module") {
            continue;
        }
        let counts = if meta.es5 { &mut es5_counts } else { &mut later_counts };
        let includes: String = meta.includes.iter().map(|i| read(&harness_dir.join(i)) + "\n").collect();
        if meta.flags.iter().any(|f| f == "async") || needs_missing_builtin(&[&source, &includes], missing) {
            counts.skipped += 1;
            continue;
        }
        counts.tests += 1;
        let has = |flag: &str| meta.flags.iter().any(|f| f == flag);
        let modes: &[bool] = if has("raw") || has("noStrict") {
            &[false]
        } else if has("onlyStrict") {
            &[true]
        } else {
            &[false, true]
        };
        let rel = path.strip_prefix(dir).unwrap_or(&path).display().to_string().replace('\\', "/");
        for &strict in modes {
            let mode = if strict { "strict" } else { "sloppy" };
            if verbose {
                eprintln!("{rel} ({mode})");
            }
            let body = if has("raw") { source.clone() } else { format!("{prelude}{includes}{source}") };
            let script = if strict { format!("\"use strict\";\n{body}") } else { body };
            let negative_type = meta.negative_type.clone().filter(|_| meta.negative_phase.as_deref() == Some("runtime"));
            let outcome = std::panic::catch_unwind(|| run_one(&script, negative_type.as_deref()));
            match outcome {
                Ok(Ok(())) => counts.passed += 1,
                Ok(Err(why)) => {
                    counts.failed += 1;
                    counts.failures.push_str(&format!("{rel} ({mode}): {why}\n"));
                }
                Err(_) => {
                    counts.failed += 1;
                    counts.panicked += 1;
                    counts.failures.push_str(&format!("{rel} ({mode}): PANIC\n"));
                }
            }
        }
    }
    (es5_counts, later_counts)
}

/// `needs_missing_builtin` with `Map`, `Symbol` and `Set` missing.
fn skips(sources: &[&str]) -> bool {
    let missing: HashSet<&str> = ["Map", "Symbol", "Set"].into_iter().collect();
    needs_missing_builtin(sources, &missing)
}

#[test]
fn a_missing_name_in_a_message_string_does_not_skip() {
    assert!(!skips(&[r##"if (x !== 1) { $ERROR("#1: new Map() should not be needed"); }"##]));
    assert!(!skips(&[r#"assert.sameValue(f(), 1, 'Set the value: ' + "a Symbol");"#]));
    // Quotes escaped inside the string, and a regular expression literal holding quotes.
    assert!(!skips(&[r#"var r = /"Map'/; var s = "say \"Map\" and \'Set\'";"#]));
    // A string in an included harness file is a message too.
    assert!(!skips(&["var x = 1;", r#"function check(v) { if (!v) throw new Test262Error("no Symbol"); }"#]));
}

#[test]
fn a_missing_name_in_code_skips() {
    assert!(skips(&["var m = new Map();"]));
    assert!(skips(&["var s = this.Symbol;"]));
    assert!(skips(&["", "function helper() { return typeof Set; }"]));
    // A name both in a message and in code.
    assert!(skips(&[r#"var m = new Map(); assert(m, "Map");"#]));
}

#[test]
fn a_missing_name_in_a_string_that_can_run_as_code_skips() {
    assert!(skips(&[r#"eval("new Map()");"#]));
    assert!(skips(&[r#"var e = eval; var s = "Symbol()"; e(s);"#]));
    assert!(skips(&[r#"var f = Function("return Set");"#]));
    assert!(skips(&[r#"var f = new Function("a", "return new Map(a)");"#]));
    assert!(skips(&[r#"var f = Function.apply(null, ["return Symbol"]);"#]));
    assert!(skips(&[r#"var f = (function () {}).constructor("return Set");"#]));
    assert!(skips(&[r#"var F = Function; F("return new Map()")();"#]));
    assert!(skips(&[r#"var C = Function.prototype.constructor; C("return new Map()")();"#]));
    assert!(skips(&[r#"this["eval"]("new Map()");"#]));
    assert!(skips(&[r#"var F = this['Function']; F("return new Map()")();"#]));
    // The string is in the test and the eval in an included file.
    assert!(skips(&[r#"run("new Map()");"#, "function run(code) { return eval(code); }"]));
}

#[test]
fn a_string_that_is_only_a_missing_name_skips() {
    // A property key reads the global by name, with or without `eval` in the source.
    assert!(skips(&[r#"var M = this["Map"];"#]));
    assert!(skips(&[r#"for (var k in this) { if (k === 'Set') { throw 1; } }"#]));
    assert!(skips(&[r#"var names = ["Object", "Symbol"];"#]));
}

#[test]
fn comments_and_longer_names_do_not_skip() {
    assert!(!skips(&["// new Map()\n/* Symbol() */ var x = 1;"]));
    assert!(!skips(&["var Mapping = 1, Settings = 2; Symbolic = 3;"]));
    assert!(!skips(&[r#"eval("var Mapping = 1, Settings = 2;");"#]));
    // `Function.prototype` alone does not make strings code.
    assert!(!skips(&[r#"assert(Function.prototype, "a Map");"#]));
}

#[test]
fn a_source_that_does_not_parse_as_es5_skips() {
    assert!(skips(&["let x = 1;"]));
    assert!(skips(&["var x = 1;", "const y = 2;"]));
    assert!(skips(&["var s = `${x}`;"]));
}
