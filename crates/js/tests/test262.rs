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
//! and uses only the built-ins the interpreter has (those that name a missing global or method,
//! in their own source or an included file, are counted as skipped, as are `async` tests). A test
//! passes when it completes, or, for `negative: phase: runtime`, when it throws an object whose
//! `constructor` is the global its `type:` names. Its counts are printed apart from the parse
//! counts, and its failures go to `run-failures.txt`.
//!
//! A third test runs the tests of the ES5 built-ins the same way: those under the folders
//! [`ES5_BUILT_INS`] names that have an `es5id:` (or, under `annexB/`, any edition's id) and no
//! `features:` tag. Its failures go to `built-ins-failures.txt`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use vectorcraft_js::{JsValue, Limits, Options, Realm, RunErrorKind, TokenKind, parse, tokenize};

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

/// The stand-in for `String` that the harness's failure messages call.
const HARNESS_STRING: &str = "function String(value) { return '' + value; }";

/// The source names a missing built-in, as an identifier, a property name or a word inside a
/// string literal (code passed to `eval` or `Function`), or does not parse as ES5 (the tests that
/// use `let` and `const`).
fn needs_missing_builtin(source: &str, missing: &HashSet<&str>) -> bool {
    let Ok(tokens) = tokenize(source, &Options::default()) else { return true };
    tokens.iter().any(|t| match &t.kind {
        TokenKind::Ident(name) => missing.contains(name.as_str()),
        TokenKind::String(units) => {
            String::from_utf16_lossy(units).split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')).any(|word| missing.contains(word))
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
    let roots = vec![(dir.join("test").join("language"), true)];
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_subset(&dir, &roots, "language", "run-failures.txt")).unwrap();
    handle.join().unwrap();
}

#[test]
fn es5_built_ins_subset_runs() {
    let Some(dir) = suite_dir() else {
        println!("test262 built-ins: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    // The Annex B tests name the edition they were written for with `esid:`, so they run without
    // an `es5id:`.
    let roots = ES5_BUILT_INS.iter().map(|d| (dir.join("test").join(d), !d.starts_with("annexB"))).collect::<Vec<_>>();
    let handle =
        std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_subset(&dir, &roots, "built-ins", "built-ins-failures.txt")).unwrap();
    handle.join().unwrap();
}

/// Runs the tests under each root (with `true`, only those that name an ES5 section), prints the
/// counts and writes the failures to `report` beside the suite.
fn run_subset(dir: &Path, roots: &[(PathBuf, bool)], label: &str, report: &str) {
    let harness_dir = dir.join("harness");
    // A file that cannot be read gives an empty source.
    let read = |p: &Path| String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned();
    // `assert.js` formats a failed assertion's message with the global `String`, which the
    // interpreter does not provide yet; the stand-in converts with `+` so a failure reports the
    // assertion's message instead of a ReferenceError.
    let prelude = format!("{HARNESS_STRING}\n{}\n{}\n", read(&harness_dir.join("assert.js")), read(&harness_dir.join("sta.js")));
    let missing: HashSet<&str> = MISSING_BUILTINS.split_whitespace().collect();
    let verbose = std::env::var_os("VECTORCRAFT_TEST262_VERBOSE").is_some();
    let (mut tests, mut skipped, mut passed, mut failed, mut panicked) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut failures = String::new();
    let files = roots.iter().flat_map(|(root, es5_only)| test_files(root).into_iter().map(move |p| (p, *es5_only)));
    for (path, es5_only) in files {
        let source = read(&path);
        let meta = meta(&source);
        let negative_parse = matches!(meta.negative_phase.as_deref(), Some("parse" | "resolution"));
        if (es5_only && !meta.es5) || !meta.features.is_empty() || negative_parse || meta.flags.iter().any(|f| f == "module") {
            continue;
        }
        let includes: String = meta.includes.iter().map(|i| read(&harness_dir.join(i)) + "\n").collect();
        if meta.flags.iter().any(|f| f == "async") || needs_missing_builtin(&source, &missing) || needs_missing_builtin(&includes, &missing) {
            skipped += 1;
            continue;
        }
        tests += 1;
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
                Ok(Ok(())) => passed += 1,
                Ok(Err(why)) => {
                    failed += 1;
                    failures.push_str(&format!("{rel} ({mode}): {why}\n"));
                }
                Err(_) => {
                    failed += 1;
                    panicked += 1;
                    failures.push_str(&format!("{rel} ({mode}): PANIC\n"));
                }
            }
        }
    }
    let report = dir.join(report);
    if let Err(e) = std::fs::write(&report, &failures) {
        println!("could not write {}: {e}", report.display());
    }
    println!(
        "test262 ES5 {label} subset through the interpreter: {tests} tests that need only the built-ins present, {passed} runs passed, \
         {failed} failed ({panicked} panicked). Skipped {skipped} tests that need other built-ins, use syntax after ES5 or are asynchronous. Failures: {}",
        report.display()
    );
    assert_eq!(panicked, 0, "the interpreter panicked; see {}", report.display());
}
