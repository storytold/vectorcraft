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
//!
//! A fifth test reads the ECMAScript 2015 subset that `Options::es2015` covers (`let`, `const`,
//! arrow functions, templates, default and rest parameters, classes, `super` and `new.target`,
//! and from ECMAScript 2020 optional chains and `??`) without and with that option, and
//! prints both counts; its failures with the option go to `es2015-failures.txt`.
//!
//! A sixth test runs the tests under the folders [`ES2015_COLLECTION_BUILT_INS`] names (`Map`, `Set`,
//! `WeakMap`, `WeakSet`, `ArrayBuffer`, `DataView`, `TypedArray` and `TypedArrayConstructors`) in
//! realms with [`Options::es2015`], each test whose feature tags are all in
//! [`ES2015_COLLECTION_FEATURES`]; its failures go to `es2015-builtins-failures.txt`. It fails when
//! the interpreter panics.
//!
//! Two more tests read the Annex B trees, where tests name any edition's section: one parses every
//! test under `test/annexB/language/` without a feature tag the same way as the ES5 parse subset
//! (failures in `annexb-parse-failures.txt`), and one runs those tests and the tests under the
//! folders [`ANNEX_B_BUILT_INS`] names through the interpreter (failures in
//! `annexb-language-failures.txt` and `annexb-builtins-failures.txt`). A suite without
//! `test/annexB/` skips them; a folder there with no test to parse or run fails them.
//!
//! Another test runs the tests under `test/built-ins/Promise` in realms with `Options::es2015`:
//! those whose feature tags name only `Promise`, `Promise.allSettled` and
//! `Promise.prototype.finally`, whose code parses and uses only the built-ins present. `async` tests
//! run with the suite's `doneprintHandle.js` and pass when they print the completion line. Its
//! failures go to `promise-failures.txt`.
//!
//! Another test runs the tests of `Symbol`, `Object.getOwnPropertySymbols` and the iterators of
//! arrays and strings (the folders [`SYMBOL_BUILT_INS`] names) in a realm with
//! [`Options::es2015`]: tests of any edition whose feature tags are all in [`SYMBOL_FEATURES`] and
//! that parse with that option. Its failures go to `symbol-failures.txt`.
//!
//! Another test reads the tests of `for-of` and destructuring (the folders
//! [`FOR_OF_AND_PATTERN_FOLDERS`] names) with [`Options::es2015`]: tests of any edition whose
//! feature tags are all in [`FOR_OF_AND_PATTERN_FEATURES`]. Negative parse tests must give a
//! `SyntaxError`; the others run as the Symbol tests run. Its failures go to `for-of-failures.txt`.
//!
//! The last test reads and runs the tests of generator functions, `yield` and the generator objects
//! (the folders [`GENERATOR_FOLDERS`] names) the same way, with the `generators` tag and the tags of
//! the two tests before it. Its failures go to `generators-failures.txt`.
//!
//! The runner reads only regular files inside the suite: it does not follow symbolic links, refuses
//! an `includes:` name that leaves the `harness/` folder, and refuses a file over
//! [`MAX_FILE_BYTES`] before reading it. A folder or file it cannot read fails the test.

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

/// The largest test or harness file read (the largest file in the pinned suite is 3,234,014 bytes).
const MAX_FILE_BYTES: u64 = 8 << 20;

/// The value of `result`; an error fails the test with its message.
fn or_fail<T: Default>(result: Result<T, String>) -> T {
    assert!(result.is_ok(), "{}", result.as_ref().err().map_or("", String::as_str));
    result.unwrap_or_default()
}

/// `dir` joined with the `/`-separated path `rel`. An empty, `.` or `..` component, one holding a
/// `\` or `:`, and a component that is a symbolic link give an error, so the path stays inside
/// `dir`. A component that does not exist is not checked further.
fn inside(dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let mut path = dir.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':']) {
            return Err(format!("{rel} is not a path inside {}", dir.display()));
        }
        path.push(part);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("{} is a symbolic link; the runner reads only files inside the suite", path.display()));
        }
    }
    Ok(path)
}

/// `dir` joined with `rel` as [`inside`] gives it, or `None` when nothing is at that path. A path
/// whose metadata cannot be read, such as one under a folder that cannot be searched, gives an error.
fn present(dir: &Path, rel: &str) -> Result<Option<PathBuf>, String> {
    let path = inside(dir, rel)?;
    match path.try_exists() {
        Ok(true) => Ok(Some(path)),
        Ok(false) => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// Reads a test or harness file: a regular file (not a symbolic link) of at most
/// [`MAX_FILE_BYTES`], with invalid UTF-8 replaced.
fn read_source(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} is {} bytes, over the {MAX_FILE_BYTES}-byte limit", path.display(), meta.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The `.js` files under `dir`, sorted, with the `_FIXTURE` helper files and symbolic links left
/// out. A folder or entry that cannot be read gives an error.
fn test_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("cannot read {}: {e}", d.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", d.display()))?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|e| e == "js") && !path.to_string_lossy().contains("_FIXTURE") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The modes a test runs in, `true` for strict mode code: both, unless its `onlyStrict`,
/// `noStrict` or `raw` flag says otherwise.
fn modes(meta: &Meta) -> &'static [bool] {
    let has = |flag: &str| meta.flags.iter().any(|f| f == flag);
    if has("raw") || has("noStrict") {
        &[false]
    } else if has("onlyStrict") {
        &[true]
    } else {
        &[false, true]
    }
}

/// The test is a script without a feature tag and, unless `any_edition` is set, names an ES5
/// section. The Annex B tests name the edition they were written for with `esid:`, so they are
/// read with `any_edition`.
fn eligible(meta: &Meta, any_edition: bool) -> bool {
    (meta.es5 || any_edition)
        && meta.features.is_empty()
        && !meta.flags.iter().any(|f| f == "module")
        && meta.negative_phase.as_deref() != Some("resolution")
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
    let language = or_fail(inside(&dir, "test/language"));
    assert!(language.is_dir(), "{} is not a test262 checkout", dir.display());
    let files = or_fail(test_files(&language));
    assert!(!files.is_empty(), "no tests under {}", language.display());
    let ParseRun { main: es5, later: other, with_features, modules } = or_fail(parse_files(&dir, &files, false));
    let (positive, negative, passed, failed) = (es5.positive, es5.negative, es5.passed, es5.failed);
    let (later, later_passed, later_failed) = (other.positive + other.negative, other.passed, other.failed);
    let report = dir.join("failures.txt");
    std::fs::write(&report, format!("# ES5 subset\n{}\n# Other tests without a feature tag\n{}", es5.failures, other.failures)).unwrap();
    println!(
        "test262 ES5 parse subset ({}): {} tests ({positive} that must parse, {negative} that must give a SyntaxError), \
         {passed} runs passed, {failed} failed. Other tests without a feature tag: {later} tests, {later_passed} runs passed, \
         {later_failed} failed. Skipped {with_features} tests with feature tags and {modules} module tests. Failures: {}",
        dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        positive + negative,
        report.display()
    );
}

#[test]
fn annex_b_parse_subset() {
    let Some(dir) = suite_dir() else {
        println!("test262 Annex B parse: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    parse_annex_b(&dir);
}

/// Parses the tests without a feature tag under `test/annexB/language`, of any edition, prints the
/// counts and writes the failures to `annexb-parse-failures.txt` beside the suite.
fn parse_annex_b(dir: &Path) {
    if or_fail(present(dir, "test/annexB")).is_none() {
        println!("test262 Annex B parse: SKIPPED ({} has no test/annexB)", dir.display());
        return;
    }
    // A suite with `test/annexB` and no `test/annexB/language` fails in the walk.
    let language = or_fail(inside(dir, "test/annexB/language"));
    let files = or_fail(test_files(&language));
    let ParseRun { main, with_features, modules, .. } = or_fail(parse_files(dir, &files, true));
    assert!(main.positive + main.negative > 0, "no Annex B tests to parse under {}", language.display());
    let report = dir.join("annexb-parse-failures.txt");
    if let Err(e) = std::fs::write(&report, &main.failures) {
        println!("could not write {}: {e}", report.display());
    }
    println!(
        "test262 Annex B language parse: {} tests ({} that must parse, {} that must give a SyntaxError), {} runs passed, {} failed. \
         Skipped {with_features} tests with feature tags and {modules} module tests. Failures: {}",
        main.positive + main.negative,
        main.positive,
        main.negative,
        main.passed,
        main.failed,
        report.display()
    );
}

/// The parse counts of one group of tests.
#[derive(Default)]
struct ParseCounts {
    /// Tests that must parse.
    positive: usize,
    /// Tests that must give a `SyntaxError`.
    negative: usize,
    passed: usize,
    failed: usize,
    failures: String,
}

/// The counts of [`parse_files`].
#[derive(Default)]
struct ParseRun {
    /// The tests that name an ES5 section, or every eligible test when any edition is read.
    main: ParseCounts,
    /// The other tests without a feature tag.
    later: ParseCounts,
    with_features: usize,
    modules: usize,
}

/// Parses each of `files` in the modes its flags give. With `any_edition`, every test without a
/// feature tag is counted in [`ParseRun::main`]; otherwise only those that name an ES5 section.
fn parse_files(dir: &Path, files: &[PathBuf], any_edition: bool) -> Result<ParseRun, String> {
    // VECTORCRAFT_TEST262_VERBOSE=1 prints each test before it is parsed.
    let verbose = std::env::var_os("VECTORCRAFT_TEST262_VERBOSE").is_some();
    let mut run = ParseRun::default();
    for path in files {
        let source = read_source(path)?;
        let meta = meta(&source);
        if !meta.features.is_empty() {
            run.with_features += 1;
            continue;
        }
        if !eligible(&meta, true) {
            run.modules += 1;
            continue;
        }
        let counts = if eligible(&meta, any_edition) { &mut run.main } else { &mut run.later };
        if meta.negative_phase.as_deref() == Some("parse") {
            counts.negative += 1;
        } else {
            counts.positive += 1;
        }
        let rel = path.strip_prefix(dir).unwrap_or(path).display().to_string().replace('\\', "/");
        for &strict in modes(&meta) {
            let mode = if strict { "strict" } else { "sloppy" };
            if verbose {
                eprintln!("{} ({mode})", path.display());
            }
            match parse_check(&source, &meta, strict) {
                Ok(()) => counts.passed += 1,
                Err(outcome) => {
                    counts.failed += 1;
                    counts.failures.push_str(&format!("{rel} ({mode}): {outcome}\n"));
                }
            }
        }
    }
    Ok(run)
}

/// Parses `source` as sloppy or strict mode code (with `"use strict";` before it); `Err` describes
/// a result other than the one the metadata expects (a `SyntaxError` for `negative: phase: parse`).
fn parse_check(source: &str, meta: &Meta, strict: bool) -> Result<(), String> {
    let expect_error = meta.negative_phase.as_deref() == Some("parse");
    let text = if strict { format!("\"use strict\";\n{source}") } else { source.to_string() };
    match parse(&text, &Options::default()) {
        Ok(_) if expect_error => Err("parsed; a SyntaxError was expected".to_string()),
        Err(e) if !expect_error => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Globals and method names the interpreter does not provide yet: a test that names one, in its
/// own source or in a harness file it includes, is skipped.
const MISSING_BUILTINS: &str = "String Number Date RegExp print $262 $DONE Symbol Proxy Reflect Promise Map Set WeakMap \
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

/// The tests of the Annex B syntax and semantics (ECMA-262 Annex B.1 and B.3).
const ANNEX_B_LANGUAGE: &str = "annexB/language";

/// The folders under `test/annexB/built-ins/` with tests without a feature tag, other than the
/// `escape` and `unescape` folders [`ES5_BUILT_INS`] runs: `substr`, `getYear`, `setYear`,
/// `toGMTString`, the legacy `RegExp` features and the `Function` tests.
const ANNEX_B_BUILT_INS: &[&str] = &["annexB/built-ins/Date", "annexB/built-ins/Function", "annexB/built-ins/RegExp", "annexB/built-ins/String"];

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
    needs_missing_builtin_in(sources, missing, &Options::default())
}

/// [`needs_missing_builtin`] for sources read with `options`. With [`Options::es2015`], a source
/// that does not tokenize is left out of the check rather than skipping the test, so the run
/// counts the tokenizer's error as a failure.
fn needs_missing_builtin_in(sources: &[&str], missing: &HashSet<&str>, options: &Options) -> bool {
    let mut lists = Vec::with_capacity(sources.len());
    for source in sources {
        match tokenize(source, options) {
            Ok(tokens) => lists.push(tokens),
            Err(_) if options.es2015 => {}
            Err(_) => return true,
        }
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

/// Runs one test in a new realm made with `options`; `Err` describes a failure.
fn run_one_with(options: &Options, script: &str, negative_type: Option<&str>) -> Result<(), String> {
    run_one(script, negative_type, options)
}

/// Runs one test in a new realm made with `options`; `Err` describes a failure.
fn run_one(script: &str, negative_type: Option<&str>, options: &Options) -> Result<(), String> {
    let mut realm = Realm::with_options(options.clone(), Limits { fuel: 20_000_000, heap_bytes: 256 << 20, ..Limits::default() });
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

    /// Adds the counts and failures of `other`.
    fn add(&mut self, other: RunCounts) {
        self.tests += other.tests;
        self.skipped += other.skipped;
        self.passed += other.passed;
        self.failed += other.failed;
        self.panicked += other.panicked;
        self.failures.push_str(&other.failures);
    }
}

fn run_language_subset(dir: &Path) {
    let language = or_fail(inside(dir, "test/language"));
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
        let root = or_fail(inside(dir, &format!("test/{folder}")));
        assert!(root.is_dir(), "{} is not a test262 checkout", dir.display());
        let (es5, later) = run_tests(dir, &root, &missing, folder.starts_with("annexB"));
        all.add(es5);
        all.add(later);
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
        let root = or_fail(inside(dir, &format!("test/built-ins/{name}")));
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

/// The directories under `test/built-ins/` that hold the tests of the ECMAScript 2015 collections
/// and binary data, which run in a realm with [`Options::es2015`].
const ES2015_COLLECTION_BUILT_INS: &[&str] = &["Map", "Set", "WeakMap", "WeakSet", "ArrayBuffer", "DataView", "TypedArray", "TypedArrayConstructors"];

/// The names in [`MISSING_BUILTINS`] that a realm with [`Options::es2015`] provides.
const ES2015_BUILTINS: &str = "Map Set WeakMap WeakSet ArrayBuffer DataView Int8Array Uint8Array Float64Array";

/// The feature tags a test under [`ES2015_COLLECTION_BUILT_INS`] may have and still run: the built-ins
/// themselves and the stable typed array sort. A tag starting `DataView.prototype.` names one of
/// the `DataView` methods. Tests with any other tag (symbols, `Reflect`, classes, resizable or
/// detached buffers, `BigInt` and later additions) are counted as skipped.
const ES2015_COLLECTION_FEATURES: &[&str] = &[
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "ArrayBuffer",
    "DataView",
    "TypedArray",
    "Int8Array",
    "Uint8Array",
    "Float64Array",
    "stable-typedarray-sort",
];

#[test]
fn es2015_collections_and_binary_data_run() {
    let Some(dir) = suite_dir() else {
        println!("test262 ECMAScript 2015 built-ins: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_es2015_built_ins(&dir)).unwrap();
    handle.join().unwrap();
}

/// Runs the tests under the folders [`ES2015_COLLECTION_BUILT_INS`] names in realms with
/// [`Options::es2015`], prints each folder's counts and writes the failures to
/// `es2015-builtins-failures.txt` beside the suite.
fn run_es2015_built_ins(dir: &Path) {
    let missing = missing_builtins(&[TEXT_BUILTINS, ES2015_BUILTINS]);
    let mut report = String::new();
    let mut panicked = 0;
    for name in ES2015_COLLECTION_BUILT_INS {
        let root = or_fail(inside(dir, &format!("test/built-ins/{name}")));
        assert!(root.is_dir(), "{} is not a test262 checkout", dir.display());
        let counts = run_es2015_tests(dir, &root, &missing);
        println!("test262 built-ins/{name} in an ECMAScript 2015 realm: {}.", counts.summary());
        report.push_str(&format!("# built-ins/{name}\n{}\n", counts.failures));
        panicked += counts.panicked;
    }
    let path = dir.join("es2015-builtins-failures.txt");
    if let Err(e) = std::fs::write(&path, &report) {
        println!("could not write {}: {e}", path.display());
    }
    println!("test262 ECMAScript 2015 built-ins failures: {}", path.display());
    assert_eq!(panicked, 0, "the interpreter panicked; see {}", path.display());
}

/// Runs the tests under `root` whose feature tags are all in [`ES2015_COLLECTION_FEATURES`], as
/// [`run_tests`] runs the ES5 tests, each in a new realm with [`Options::es2015`].
fn run_es2015_tests(dir: &Path, root: &Path, missing: &HashSet<&str>) -> RunCounts {
    // A harness file is named by the test's `includes:`, so it is read only inside `harness/`.
    let harness = |name: &str| or_fail(inside(dir, &format!("harness/{name}")).and_then(|p| read_source(&p)));
    let prelude = format!("{}\n{}\n", harness("assert.js"), harness("sta.js"));
    let mut counts = RunCounts::default();
    for path in or_fail(test_files(root)) {
        let source = or_fail(read_source(&path));
        let meta = meta(&source);
        if matches!(meta.negative_phase.as_deref(), Some("parse" | "resolution")) || meta.flags.iter().any(|f| f == "module") {
            continue;
        }
        let includes: String = meta.includes.iter().map(|i| harness(i) + "\n").collect();
        let known = meta.features.iter().all(|f| ES2015_COLLECTION_FEATURES.contains(&f.as_str()) || f.starts_with("DataView.prototype."));
        if !known || meta.flags.iter().any(|f| f == "async") || needs_missing_builtin(&[&source, &includes], missing) {
            counts.skipped += 1;
            continue;
        }
        counts.tests += 1;
        let has = |flag: &str| meta.flags.iter().any(|f| f == flag);
        let rel = path.strip_prefix(dir).unwrap_or(&path).display().to_string().replace('\\', "/");
        for &strict in modes(&meta) {
            let mode = if strict { "strict" } else { "sloppy" };
            let body = if has("raw") { source.clone() } else { format!("{prelude}{includes}{source}") };
            let script = if strict { format!("\"use strict\";\n{body}") } else { body };
            let negative_type = meta.negative_type.clone().filter(|_| meta.negative_phase.as_deref() == Some("runtime"));
            match std::panic::catch_unwind(|| run_one(&script, negative_type.as_deref(), &Options { es2015: true, ..Options::default() })) {
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
    counts
}

/// Runs the tests under `root` that have no feature tag and are not module, asynchronous or
/// negative parse tests, each in a new realm after the harness. Returns the counts for the tests
/// that name an ES5 section and, when `later` is set, for the others (otherwise those are left out).
fn run_tests(dir: &Path, root: &Path, missing: &HashSet<&str>, later: bool) -> (RunCounts, RunCounts) {
    // A harness file is named by the test's `includes:`, so it is read only inside `harness/`.
    let harness = |name: &str| or_fail(inside(dir, &format!("harness/{name}")).and_then(|p| read_source(&p)));
    let prelude = format!("{}\n{}\n", harness("assert.js"), harness("sta.js"));
    let verbose = std::env::var_os("VECTORCRAFT_TEST262_VERBOSE").is_some();
    let (mut es5_counts, mut later_counts) = (RunCounts::default(), RunCounts::default());
    for path in or_fail(test_files(root)) {
        let source = or_fail(read_source(&path));
        let meta = meta(&source);
        if !eligible(&meta, later) || meta.negative_phase.as_deref() == Some("parse") {
            continue;
        }
        let counts = if meta.es5 { &mut es5_counts } else { &mut later_counts };
        let includes: String = meta.includes.iter().map(|i| harness(i) + "\n").collect();
        if meta.flags.iter().any(|f| f == "async") || needs_missing_builtin(&[&source, &includes], missing) {
            counts.skipped += 1;
            continue;
        }
        counts.tests += 1;
        let has = |flag: &str| meta.flags.iter().any(|f| f == flag);
        let rel = path.strip_prefix(dir).unwrap_or(&path).display().to_string().replace('\\', "/");
        for &strict in modes(&meta) {
            let mode = if strict { "strict" } else { "sloppy" };
            if verbose {
                eprintln!("{rel} ({mode})");
            }
            let body = if has("raw") { source.clone() } else { format!("{prelude}{includes}{source}") };
            let script = if strict { format!("\"use strict\";\n{body}") } else { body };
            let negative_type = meta.negative_type.clone().filter(|_| meta.negative_phase.as_deref() == Some("runtime"));
            let outcome = std::panic::catch_unwind(|| run_one(&script, negative_type.as_deref(), &Options::default()));
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

/// The folders under `test/language/` that hold tests of the ECMAScript 2015 forms
/// [`Options::es2015`] reads.
const ES2015_FOLDERS: &[&str] = &[
    "block-scope",
    "statements/let",
    "statements/const",
    "expressions/arrow-function",
    "expressions/template-literal",
    "expressions/tagged-template",
    "rest-parameters",
    "statements/class",
    "expressions/class",
    "expressions/super",
    "expressions/new.target",
    "expressions/object",
    "computed-property-names",
    "expressions/optional-chaining",
    "expressions/coalesce",
];

/// The folders under `test/built-ins/` that [`run_es2015`] also reads: the `Function` tests hold
/// those of the `name` and `length` properties of functions.
const ES2015_BUILT_INS: &[&str] = &["Function"];

/// The test files under `test/language/` that [`run_es2015`] also reads wherever they are: those
/// of the `name` a function gets from its declaration or from the binding or property it is first
/// assigned to.
fn es2015_name_test(rel: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    file.starts_with("fn-name-") || (file == "name.js" && (rel.contains("/function/") || rel.contains("/arrow-function/")))
}

/// The feature tags of the forms [`Options::es2015`] reads.
const ES2015_FEATURES: &[&str] = &[
    "let",
    "const",
    "arrow-function",
    "template",
    "default-parameters",
    "rest-parameters",
    "class",
    "super",
    "new.target",
    "computed-property-names",
    "__proto__",
    "optional-chaining",
    "coalesce-expression",
];

#[test]
fn es2015_subset_runs() {
    let Some(dir) = suite_dir() else {
        println!("test262 ES2015 subset: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_es2015(&dir)).unwrap();
    handle.join().unwrap();
}

/// The counts of the ES2015 subset read with one setting of [`Options::es2015`].
#[derive(Default)]
struct Es2015Counts {
    parse_passed: usize,
    parse_failed: usize,
    run_passed: usize,
    run_failed: usize,
    panicked: usize,
    failures: String,
}

/// Runs the ES2015 subset: the tests under `test/language/` in [`ES2015_FOLDERS`], under
/// `test/built-ins/` in [`ES2015_BUILT_INS`] and those [`es2015_name_test`] picks, whose feature
/// tags are all in [`ES2015_FEATURES`], and the tests elsewhere under `test/language/` with feature
/// tags that are all in it. Negative parse tests must give a `SyntaxError`; the other tests run in a
/// new realm after the harness, as [`run_tests`] runs them, skipping module and asynchronous tests
/// and those that need a missing built-in. Each test is read twice, without and with
/// [`Options::es2015`], and the counts of both are printed; the failures with it go to
/// `es2015-failures.txt`.
fn run_es2015(dir: &Path) {
    let language = or_fail(inside(dir, "test/language"));
    assert!(language.is_dir(), "{} is not a test262 checkout", dir.display());
    // A harness file is named by the test's `includes:`, so it is read only inside `harness/`.
    let harness = |name: &str| or_fail(inside(dir, &format!("harness/{name}")).and_then(|p| read_source(&p)));
    let prelude = format!("{}\n{}\n", harness("assert.js"), harness("sta.js"));
    let missing = missing_builtins(&[TEXT_BUILTINS]);
    let modern = Options { es2015: true, ..Options::default() };
    let editions = [Options::default(), modern.clone()];
    let mut counts = [Es2015Counts::default(), Es2015Counts::default()];
    let (mut selected, mut skipped, mut other_features) = (0usize, 0usize, 0usize);
    let mut paths = or_fail(test_files(&language));
    for folder in ES2015_BUILT_INS {
        paths.extend(or_fail(inside(dir, &format!("test/built-ins/{folder}")).and_then(|d| test_files(&d))));
    }
    for path in paths {
        let rel = path.strip_prefix(dir).unwrap_or(&path).display().to_string().replace('\\', "/");
        let in_folder = ES2015_FOLDERS.iter().any(|f| rel.starts_with(&format!("test/language/{f}/")))
            || ES2015_BUILT_INS.iter().any(|f| rel.starts_with(&format!("test/built-ins/{f}/")))
            || es2015_name_test(&rel);
        let source = or_fail(read_source(&path));
        let meta = meta(&source);
        let tags_in_set = meta.features.iter().all(|f| ES2015_FEATURES.contains(&f.as_str()));
        if !(in_folder || !meta.features.is_empty()) {
            continue;
        }
        if !tags_in_set {
            if in_folder {
                other_features += 1;
            }
            continue;
        }
        let has = |flag: &str| meta.flags.iter().any(|f| f == flag);
        if has("module") || has("async") || meta.negative_phase.as_deref() == Some("resolution") {
            skipped += 1;
            continue;
        }
        let expect_error = meta.negative_phase.as_deref() == Some("parse");
        let includes: String = meta.includes.iter().map(|i| harness(i) + "\n").collect();
        if !expect_error && needs_missing_builtin_in(&[&source, &includes], &missing, &modern) {
            skipped += 1;
            continue;
        }
        selected += 1;
        for (options, counts) in editions.iter().zip(counts.iter_mut()) {
            for &strict in modes(&meta) {
                let mode = if strict { "strict" } else { "sloppy" };
                let outcome = if expect_error {
                    let text = if strict { format!("\"use strict\";\n{source}") } else { source.clone() };
                    let result = parse(&text, options);
                    if result.is_err() {
                        counts.parse_passed += 1;
                        continue;
                    }
                    counts.parse_failed += 1;
                    "parsed; a SyntaxError was expected".to_string()
                } else {
                    let body = if has("raw") { source.clone() } else { format!("{prelude}{includes}{source}") };
                    let script = if strict { format!("\"use strict\";\n{body}") } else { body };
                    let negative_type = meta.negative_type.clone().filter(|_| meta.negative_phase.as_deref() == Some("runtime"));
                    match std::panic::catch_unwind(|| run_one(&script, negative_type.as_deref(), options)) {
                        Ok(Ok(())) => {
                            counts.run_passed += 1;
                            continue;
                        }
                        Ok(Err(why)) => {
                            counts.run_failed += 1;
                            why
                        }
                        Err(_) => {
                            counts.run_failed += 1;
                            counts.panicked += 1;
                            "PANIC".to_string()
                        }
                    }
                };
                counts.failures.push_str(&format!("{rel} ({mode}): {outcome}\n"));
            }
        }
    }
    assert!(selected > 0, "no ES2015 tests under {}", language.display());
    let [before, after] = counts;
    let report = dir.join("es2015-failures.txt");
    if let Err(e) = std::fs::write(&report, &after.failures) {
        println!("could not write {}: {e}", report.display());
    }
    let line = |c: &Es2015Counts| {
        format!(
            "negative parse runs {} passed, {} failed; other runs {} passed, {} failed ({} panicked)",
            c.parse_passed, c.parse_failed, c.run_passed, c.run_failed, c.panicked
        )
    };
    println!(
        "test262 ES2015 subset: {selected} tests. Without Options::es2015: {}. With it: {}. Skipped {skipped} module, asynchronous \
         or missing-built-in tests and {other_features} tests in the folders with other feature tags. Failures: {}",
        line(&before),
        line(&after),
        report.display()
    );
    assert_eq!(before.panicked + after.panicked, 0, "the interpreter panicked; see {}", report.display());
}

#[test]
fn annex_b_runs() {
    let Some(dir) = suite_dir() else {
        println!("test262 Annex B interpreter: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_annex_b(&dir)).unwrap();
    handle.join().unwrap();
}

/// Runs the tests without a feature tag under [`ANNEX_B_LANGUAGE`] and under each folder
/// [`ANNEX_B_BUILT_INS`] names, of any edition, prints the counts of each and writes the failures
/// to `annexb-language-failures.txt` and `annexb-builtins-failures.txt` beside the suite.
fn run_annex_b(dir: &Path) {
    if or_fail(present(dir, "test/annexB")).is_none() {
        println!("test262 Annex B interpreter: SKIPPED ({} has no test/annexB)", dir.display());
        return;
    }
    let mut panicked = 0;
    let mut built_ins_report = String::new();
    for folder in std::iter::once(&ANNEX_B_LANGUAGE).chain(ANNEX_B_BUILT_INS) {
        let Some(root) = or_fail(present(dir, &format!("test/{folder}"))) else {
            assert!(*folder != ANNEX_B_LANGUAGE, "{} has test/annexB but no test/{folder}", dir.display());
            println!("test262 {folder}: SKIPPED (not in this suite)");
            continue;
        };
        let missing = if folder.ends_with("/String") {
            missing_builtins(&[TEXT_BUILTINS, STRING_METHODS_SHARED_WITH_ARRAY])
        } else {
            missing_builtins(&[TEXT_BUILTINS])
        };
        let (es5, later) = run_tests(dir, &root, &missing, true);
        let mut counts = RunCounts::default();
        counts.add(es5);
        counts.add(later);
        assert!(counts.tests > 0, "no Annex B tests ran under {}", root.display());
        panicked += counts.panicked;
        println!("test262 {folder} through the interpreter: {}.", counts.summary());
        if *folder == ANNEX_B_LANGUAGE {
            let path = dir.join("annexb-language-failures.txt");
            if let Err(e) = std::fs::write(&path, &counts.failures) {
                println!("could not write {}: {e}", path.display());
            }
            println!("test262 {folder} failures: {}", path.display());
        } else {
            built_ins_report.push_str(&format!("# {folder}\n{}\n", counts.failures));
        }
    }
    let path = dir.join("annexb-builtins-failures.txt");
    if let Err(e) = std::fs::write(&path, &built_ins_report) {
        println!("could not write {}: {e}", path.display());
    }
    println!("test262 Annex B built-ins failures: {}", path.display());
    assert_eq!(panicked, 0, "the interpreter panicked; see the Annex B failure files in {}", dir.display());
}

/// The feature tags a `test/built-ins/Promise` test may carry and still run: `Promise`,
/// `Promise.allSettled` and `Promise.prototype.finally`.
const PROMISE_FEATURES: &[&str] = &["Promise", "Promise.allSettled", "Promise.prototype.finally"];

#[test]
fn promise_builtins_run() {
    let Some(dir) = suite_dir() else {
        println!("test262 Promise: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_promise(&dir)).unwrap();
    handle.join().unwrap();
}

#[test]
fn promise_async_completion_requires_an_exact_print_argument() {
    assert!(run_promise_one("print('not Test262:AsyncTestComplete');", None, true).is_err());
    assert!(run_promise_one("print('Test262:AsyncTestComplete suffix');", None, true).is_err());
    assert!(run_promise_one("print('message', 'Test262:AsyncTestComplete');", None, true).is_ok());
    assert!(run_promise_one("print('Test262:AsyncTestComplete');", None, true).is_ok());
}

#[test]
fn promise_async_failure_checks_each_print_argument() {
    assert!(run_promise_one("print('Test262:AsyncTestComplete', 'Test262:AsyncTestFailure: failed');", None, true).is_err());
    assert!(run_promise_one("print('not Test262:AsyncTestFailure'); print('Test262:AsyncTestComplete');", None, true).is_ok());
}

/// Runs one Promise test with a global `print` that collects every argument. An `async` test
/// needs an exact `Test262:AsyncTestComplete` argument and no `Test262:AsyncTestFailure` prefix.
fn run_promise_one(script: &str, negative_type: Option<&str>, asynchronous: bool) -> Result<(), String> {
    let options = Options { es2015: true, ..Options::default() };
    let mut realm = Realm::with_options(options, Limits { fuel: 20_000_000, heap_bytes: 256 << 20, ..Limits::default() });
    let printed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let out = printed.clone();
    realm.define_function("print", move |args| {
        let mut printed = out.borrow_mut();
        for arg in args {
            printed.push(match arg {
                JsValue::String(s) => s.clone(),
                other => format!("{other:?}"),
            });
        }
        Ok(JsValue::Undefined)
    });
    let result = realm.run(script);
    if asynchronous && result.is_ok() && negative_type.is_none() {
        let printed = printed.borrow();
        return if printed.iter().any(|arg| arg == "Test262:AsyncTestComplete")
            && !printed.iter().any(|arg| arg.starts_with("Test262:AsyncTestFailure"))
        {
            Ok(())
        } else {
            Err(format!("printed {printed:?}"))
        };
    }
    match (result, negative_type) {
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

/// Runs the tests under `test/built-ins/Promise` whose feature tags are all in
/// [`PROMISE_FEATURES`], whose code parses as ES5 and uses only the built-ins present, as sloppy
/// and strict code as [`run_tests`] does, `async` tests included. Prints the counts and writes the
/// failures to `promise-failures.txt` beside the suite.
fn run_promise(dir: &Path) {
    let root = or_fail(inside(dir, "test/built-ins/Promise"));
    assert!(root.is_dir(), "{} is not a test262 checkout", dir.display());
    // A harness file is named by the test's `includes:`, so it is read only inside `harness/`.
    let harness = |name: &str| or_fail(inside(dir, &format!("harness/{name}")).and_then(|p| read_source(&p)));
    let prelude = format!("{}\n{}\n", harness("assert.js"), harness("sta.js"));
    let done = harness("doneprintHandle.js");
    let missing: HashSet<&str> = missing_builtins(&[TEXT_BUILTINS, "Promise $DONE print"]);
    let mut groups: std::collections::BTreeMap<String, RunCounts> = std::collections::BTreeMap::new();
    let (mut files, mut other_features) = (0usize, 0usize);
    for path in or_fail(test_files(&root)) {
        files += 1;
        let source = or_fail(read_source(&path));
        let meta = meta(&source);
        if meta.features.iter().any(|f| !PROMISE_FEATURES.contains(&f.as_str())) {
            other_features += 1;
            continue;
        }
        let asynchronous = meta.flags.iter().any(|f| f == "async");
        let tags = if meta.features.is_empty() { "no feature tag".to_string() } else { meta.features.join(", ") };
        let counts = groups.entry(format!("{tags}, {}", if asynchronous { "async" } else { "sync" })).or_default();
        let negative_parse = matches!(meta.negative_phase.as_deref(), Some("parse" | "resolution"));
        if negative_parse || meta.flags.iter().any(|f| f == "module") {
            counts.skipped += 1;
            continue;
        }
        let mut includes: String = meta.includes.iter().map(|i| harness(i) + "\n").collect();
        if asynchronous {
            includes.push_str(&done);
            includes.push('\n');
        }
        if parse(&source, &Options::default()).is_err() || needs_missing_builtin(&[&source, &includes], &missing) {
            counts.skipped += 1;
            continue;
        }
        counts.tests += 1;
        let has = |flag: &str| meta.flags.iter().any(|f| f == flag);
        let rel = path.strip_prefix(dir).unwrap_or(&path).display().to_string().replace('\\', "/");
        for &strict in modes(&meta) {
            let mode = if strict { "strict" } else { "sloppy" };
            let body = if has("raw") { source.clone() } else { format!("{prelude}{includes}{source}") };
            let script = if strict { format!("\"use strict\";\n{body}") } else { body };
            let negative_type = meta.negative_type.clone().filter(|_| meta.negative_phase.as_deref() == Some("runtime"));
            match std::panic::catch_unwind(|| run_promise_one(&script, negative_type.as_deref(), asynchronous)) {
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
    let mut all = RunCounts::default();
    for (group, counts) in &groups {
        println!("test262 built-ins/Promise, {group}: {}.", counts.summary());
        all.tests += counts.tests;
        all.skipped += counts.skipped;
        all.passed += counts.passed;
        all.failed += counts.failed;
        all.panicked += counts.panicked;
        all.failures.push_str(&counts.failures);
    }
    let report = dir.join("promise-failures.txt");
    if let Err(e) = std::fs::write(&report, &all.failures) {
        println!("could not write {}: {e}", report.display());
    }
    println!(
        "test262 built-ins/Promise through the interpreter: {files} files, {other_features} with other feature tags left out; {}. Failures: {}",
        all.summary(),
        report.display()
    );
    assert!(all.tests > 0, "no Promise tests ran under {}", root.display());
    assert_eq!(all.panicked, 0, "the interpreter panicked; see {}", report.display());
}

/// The folders of the tests of `Symbol`, Symbol-keyed properties and the iterators of arrays and
/// strings, which run in a realm with [`Options::es2015`].
const SYMBOL_BUILT_INS: &[&str] = &[
    "built-ins/Symbol",
    "built-ins/ArrayIteratorPrototype",
    "built-ins/StringIteratorPrototype",
    "built-ins/Object/getOwnPropertySymbols",
    "built-ins/Array/prototype/values",
    "built-ins/Array/prototype/keys",
    "built-ins/Array/prototype/entries",
    "built-ins/Array/prototype/Symbol.iterator",
    "built-ins/String/prototype/Symbol.iterator",
];

/// The feature tags of the tests under [`SYMBOL_BUILT_INS`] that run; a test with any other tag is
/// skipped.
const SYMBOL_FEATURES: &[&str] = &[
    "Symbol",
    "Symbol.iterator",
    "Symbol.toStringTag",
    "Symbol.hasInstance",
    "Symbol.toPrimitive",
    "Symbol.asyncIterator",
    "Symbol.prototype.description",
];

#[test]
fn es2015_symbol_and_iterators_run() {
    let Some(dir) = suite_dir() else {
        println!("test262 Symbol and iterators: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_symbol_built_ins(&dir)).unwrap();
    handle.join().unwrap();
}

/// Runs the tests under each folder [`SYMBOL_BUILT_INS`] names in a realm with
/// [`Options::es2015`], prints the counts of each and writes the failures to
/// `symbol-failures.txt` beside the suite.
fn run_symbol_built_ins(dir: &Path) {
    let options = Options { es2015: true, ..Options::default() };
    let missing: HashSet<&str> = missing_builtins(&[TEXT_BUILTINS, "Symbol"]);
    let mut all = RunCounts::default();
    let mut report = String::new();
    for folder in SYMBOL_BUILT_INS {
        let root = or_fail(inside(dir, &format!("test/{folder}")));
        assert!(root.is_dir(), "{} is not a test262 checkout", dir.display());
        let counts = run_tagged_tests(dir, &root, &missing, &options, SYMBOL_FEATURES);
        println!("test262 {folder} with Options::es2015: {}.", counts.summary());
        report.push_str(&format!("# {folder}\n{}\n", counts.failures));
        all.add(counts);
    }
    assert!(all.tests > 0, "no Symbol or iterator tests ran under {}", dir.display());
    let path = dir.join("symbol-failures.txt");
    if let Err(e) = std::fs::write(&path, &report) {
        println!("could not write {}: {e}", path.display());
    }
    println!("test262 Symbol and iterators with Options::es2015: {}. Failures: {}", all.summary(), path.display());
    assert_eq!(all.panicked, 0, "the interpreter panicked; see {}", path.display());
}

/// The folders of the tests of `for-of` and destructuring, which are read in a realm with
/// [`Options::es2015`].
const FOR_OF_AND_PATTERN_FOLDERS: &[&str] = &[
    "language/statements/for-of",
    "language/expressions/assignment/destructuring",
    "language/expressions/assignment/dstr",
    "language/statements/variable/dstr",
    "language/statements/let/dstr",
    "language/statements/const/dstr",
    "language/statements/function/dstr",
    "language/expressions/function/dstr",
    "language/expressions/arrow-function/dstr",
    "language/statements/try/dstr",
    "language/statements/for/dstr",
    "language/statements/for-in/dstr",
];

/// The feature tags of the tests under [`FOR_OF_AND_PATTERN_FOLDERS`] that are read; a test with
/// any other tag is skipped.
const FOR_OF_AND_PATTERN_FEATURES: &[&str] = &[
    "let",
    "const",
    "arrow-function",
    "template",
    "default-parameters",
    "rest-parameters",
    "destructuring-binding",
    "destructuring-assignment",
    "computed-property-names",
    "Symbol",
    "Symbol.iterator",
];

#[test]
fn es2015_for_of_and_destructuring_run() {
    let Some(dir) = suite_dir() else {
        println!("test262 for-of and destructuring: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_for_of_and_patterns(&dir)).unwrap();
    handle.join().unwrap();
}

/// Reads the tests under each folder [`FOR_OF_AND_PATTERN_FOLDERS`] names whose feature tags are
/// all in [`FOR_OF_AND_PATTERN_FEATURES`] with [`Options::es2015`]: a negative parse test must give
/// a `SyntaxError`, and the others run as [`run_tagged_tests`] runs them. Prints the counts of each
/// folder and writes the failures to `for-of-failures.txt` beside the suite.
fn run_for_of_and_patterns(dir: &Path) {
    let options = Options { es2015: true, ..Options::default() };
    let missing: HashSet<&str> = missing_builtins(&[TEXT_BUILTINS, "Symbol"]);
    let mut all = RunCounts::default();
    let mut report = String::new();
    for folder in FOR_OF_AND_PATTERN_FOLDERS {
        let Some(root) = or_fail(present(dir, &format!("test/{folder}"))) else {
            println!("test262 {folder}: SKIPPED (not in this suite)");
            continue;
        };
        let mut counts = run_parse_negatives(dir, &root, &options, FOR_OF_AND_PATTERN_FEATURES);
        counts.add(run_tagged_tests(dir, &root, &missing, &options, FOR_OF_AND_PATTERN_FEATURES));
        println!("test262 {folder} with Options::es2015: {}.", counts.summary());
        report.push_str(&format!("# {folder}\n{}\n", counts.failures));
        all.add(counts);
    }
    assert!(all.tests > 0, "no for-of or destructuring tests ran under {}", dir.display());
    let path = dir.join("for-of-failures.txt");
    if let Err(e) = std::fs::write(&path, &report) {
        println!("could not write {}: {e}", path.display());
    }
    println!("test262 for-of and destructuring with Options::es2015: {}. Failures: {}", all.summary(), path.display());
    assert_eq!(all.panicked, 0, "the interpreter panicked; see {}", path.display());
}

/// The folders of the tests of generator functions, `yield` and the generator objects, which run
/// in a realm with [`Options::es2015`].
const GENERATOR_FOLDERS: &[&str] = &[
    "language/statements/generators",
    "language/expressions/generators",
    "language/expressions/yield",
    "built-ins/GeneratorPrototype",
    "built-ins/GeneratorFunction",
];

#[test]
fn es2015_generators_run() {
    let Some(dir) = suite_dir() else {
        println!("test262 generators: SKIPPED (no suite; run cargo xtask test262)");
        return;
    };
    let handle = std::thread::Builder::new().stack_size(64 << 20).spawn(move || run_generators(&dir)).unwrap();
    handle.join().unwrap();
}

/// Reads the tests under each folder [`GENERATOR_FOLDERS`] names whose feature tags are all
/// `generators` or in [`FOR_OF_AND_PATTERN_FEATURES`] or [`SYMBOL_FEATURES`] with
/// [`Options::es2015`], as [`run_for_of_and_patterns`] reads its tests. Prints the counts of each
/// folder and writes the failures to `generators-failures.txt` beside the suite.
fn run_generators(dir: &Path) {
    let options = Options { es2015: true, ..Options::default() };
    let missing: HashSet<&str> = missing_builtins(&[TEXT_BUILTINS, "Symbol"]);
    let mut features: Vec<&str> = vec!["generators"];
    features.extend(FOR_OF_AND_PATTERN_FEATURES);
    features.extend(SYMBOL_FEATURES);
    let mut all = RunCounts::default();
    let mut report = String::new();
    for folder in GENERATOR_FOLDERS {
        let Some(root) = or_fail(present(dir, &format!("test/{folder}"))) else {
            println!("test262 {folder}: SKIPPED (not in this suite)");
            continue;
        };
        let mut counts = run_parse_negatives(dir, &root, &options, &features);
        counts.add(run_tagged_tests(dir, &root, &missing, &options, &features));
        println!("test262 {folder} with Options::es2015: {}.", counts.summary());
        report.push_str(&format!("# {folder}\n{}\n", counts.failures));
        all.add(counts);
    }
    assert!(all.tests > 0, "no generator tests ran under {}", dir.display());
    let path = dir.join("generators-failures.txt");
    if let Err(e) = std::fs::write(&path, &report) {
        println!("could not write {}: {e}", path.display());
    }
    println!("test262 generators with Options::es2015: {}. Failures: {}", all.summary(), path.display());
    assert_eq!(all.panicked, 0, "the interpreter panicked; see {}", path.display());
}

/// Parses the negative parse tests under `root` whose feature tags are all in `features`, in
/// each mode the test names, with `options`; each must give a `SyntaxError`.
fn run_parse_negatives(dir: &Path, root: &Path, options: &Options, features: &[&str]) -> RunCounts {
    let mut counts = RunCounts::default();
    for path in or_fail(test_files(root)) {
        let source = or_fail(read_source(&path));
        let meta = meta(&source);
        if meta.flags.iter().any(|f| f == "module") || meta.negative_phase.as_deref() != Some("parse") {
            continue;
        }
        if meta.features.iter().any(|f| !features.contains(&f.as_str())) {
            counts.skipped += 1;
            continue;
        }
        counts.tests += 1;
        let rel = path.strip_prefix(dir).unwrap_or(&path).display().to_string().replace('\\', "/");
        for &strict in modes(&meta) {
            let text = if strict { format!("\"use strict\";\n{source}") } else { source.clone() };
            match std::panic::catch_unwind(|| parse(&text, options).is_err()) {
                Ok(true) => counts.passed += 1,
                Ok(false) => {
                    counts.failed += 1;
                    let mode = if strict { "strict" } else { "sloppy" };
                    counts.failures.push_str(&format!("{rel} ({mode}): parsed; a SyntaxError was expected\n"));
                }
                Err(_) => {
                    counts.failed += 1;
                    counts.panicked += 1;
                    counts.failures.push_str(&format!("{rel}: PANIC\n"));
                }
            }
        }
    }
    counts
}

/// Runs the script tests under `root` of any edition whose feature tags are all in `features`,
/// each in a new realm made with `options` after the harness, the way [`run_tests`] runs the
/// others. A test with another tag is counted as skipped.
fn run_tagged_tests(dir: &Path, root: &Path, missing: &HashSet<&str>, options: &Options, features: &[&str]) -> RunCounts {
    let harness = |name: &str| or_fail(inside(dir, &format!("harness/{name}")).and_then(|p| read_source(&p)));
    let prelude = format!("{}\n{}\n", harness("assert.js"), harness("sta.js"));
    let mut counts = RunCounts::default();
    for path in or_fail(test_files(root)) {
        let source = or_fail(read_source(&path));
        let meta = meta(&source);
        if meta.flags.iter().any(|f| f == "module") || matches!(meta.negative_phase.as_deref(), Some("parse" | "resolution")) {
            continue;
        }
        let tagged_elsewhere = meta.features.iter().any(|f| !features.contains(&f.as_str()));
        let includes: String = meta.includes.iter().map(|i| harness(i) + "\n").collect();
        if tagged_elsewhere || meta.flags.iter().any(|f| f == "async") || needs_missing_builtin_in(&[&source, &includes], missing, options) {
            counts.skipped += 1;
            continue;
        }
        counts.tests += 1;
        let rel = path.strip_prefix(dir).unwrap_or(&path).display().to_string().replace('\\', "/");
        for &strict in modes(&meta) {
            let mode = if strict { "strict" } else { "sloppy" };
            let body = if meta.flags.iter().any(|f| f == "raw") { source.clone() } else { format!("{prelude}{includes}{source}") };
            let script = if strict { format!("\"use strict\";\n{body}") } else { body };
            let negative_type = meta.negative_type.clone().filter(|_| meta.negative_phase.as_deref() == Some("runtime"));
            match std::panic::catch_unwind(|| run_one_with(options, &script, negative_type.as_deref())) {
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
    counts
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
fn a_source_that_does_not_tokenize_as_es2015_is_run_and_not_skipped() {
    // The ES5 runs skip a test whose ES2015 syntax they cannot read. In the ES2015 subset, a
    // source that does not tokenize is run, so the tokenizer's error is counted as a failure.
    let missing: HashSet<&str> = ["Map"].into_iter().collect();
    let modern = Options { es2015: true, ..Options::default() };
    let unreadable = "var s = `never closed";
    assert!(needs_missing_builtin(&[unreadable], &missing));
    assert!(!needs_missing_builtin_in(&[unreadable], &missing, &modern));
    // A missing built-in in another source still skips.
    assert!(needs_missing_builtin_in(&[unreadable, "var m = new Map();"], &missing, &modern));
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

#[test]
fn annex_b_runs_read_the_language_tree_and_the_built_ins_folders_not_run_elsewhere() {
    assert_eq!(ANNEX_B_LANGUAGE, "annexB/language");
    for folder in ["annexB/built-ins/String", "annexB/built-ins/Date", "annexB/built-ins/RegExp", "annexB/built-ins/Function"] {
        assert!(ANNEX_B_BUILT_INS.contains(&folder), "{folder} is not read");
    }
    // `escape` and `unescape` run with the ES5 built-ins, so they are not run twice.
    for folder in ANNEX_B_BUILT_INS {
        assert!(!ES5_BUILT_INS.contains(folder), "{folder} runs twice");
        assert!(folder.starts_with("annexB/built-ins/"), "{folder} is not an Annex B built-ins folder");
    }
}

/// A test written for a later edition, naming its section with `esid:` only.
const ESID_ONLY: &str = "/*---\nesid: sec-example\ndescription: a test of a later edition\n---*/\nvar a = 1;\n";

#[test]
fn tests_that_name_only_a_later_section_are_read_with_any_edition() {
    let later = meta(ESID_ONLY);
    assert!(!later.es5);
    assert!(eligible(&later, true));
    assert!(!eligible(&later, false));
    let es5 = meta("/*---\nes5id: 12.6.4\n---*/\nvar a = 1;\n");
    assert!(eligible(&es5, false));
    let tagged = meta("/*---\nesid: sec-example\nfeatures: [let]\n---*/\nlet a = 1;\n");
    assert!(!eligible(&tagged, true));
    let module = meta("/*---\nesid: sec-example\nflags: [module]\n---*/\nvar a = 1;\n");
    assert!(!eligible(&module, true));
}

/// A strict-only test that expects a `SyntaxError` for an initializer on a for-in variable.
const STRICT_FOR_IN_INITIALIZER: &str =
    "/*---\nesid: sec-example\nnegative:\n  phase: parse\n  type: SyntaxError\nflags: [onlyStrict]\n---*/\nfor (var a = 0 in {}) {}\n";

/// A sloppy-only test with an initializer on a for-in variable, which must parse.
const SLOPPY_FOR_IN_INITIALIZER: &str = "/*---\nesid: sec-example\nflags: [noStrict]\n---*/\nvar n = 0;\nfor (var a = 1 in {}) { n += 1; }\n";

#[test]
fn a_for_in_initializer_is_a_syntax_error_in_strict_code_only() {
    let strict = meta(STRICT_FOR_IN_INITIALIZER);
    assert_eq!(strict.negative_phase.as_deref(), Some("parse"));
    assert_eq!(modes(&strict), &[true]);
    assert_eq!(parse_check(STRICT_FOR_IN_INITIALIZER, &strict, true), Ok(()));
    assert!(parse_check(STRICT_FOR_IN_INITIALIZER, &strict, false).is_err(), "the initializer parses in sloppy code");

    let sloppy = meta(SLOPPY_FOR_IN_INITIALIZER);
    assert_eq!(sloppy.negative_phase, None);
    assert_eq!(modes(&sloppy), &[false]);
    assert_eq!(parse_check(SLOPPY_FOR_IN_INITIALIZER, &sloppy, false), Ok(()));
    assert!(parse_check(SLOPPY_FOR_IN_INITIALIZER, &sloppy, true).is_err(), "the initializer parses in strict code");

    assert_eq!(modes(&meta(ESID_ONLY)), &[false, true]);
    assert_eq!(modes(&meta("/*---\nflags: [raw]\n---*/\n")), &[false]);
}

/// A new, empty folder under the system's temporary folder for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-test262-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    or_fail(std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display())));
    dir
}

#[test]
fn the_walk_reads_only_test_files_inside_the_folder() {
    let dir = scratch("walk");
    std::fs::create_dir_all(dir.join("suite/sub")).unwrap();
    std::fs::write(dir.join("suite/b.js"), "").unwrap();
    std::fs::write(dir.join("suite/sub/a.js"), "").unwrap();
    std::fs::write(dir.join("suite/c_FIXTURE.js"), "").unwrap();
    std::fs::write(dir.join("suite/notes.txt"), "").unwrap();
    assert_eq!(test_files(&dir.join("suite")), Ok(vec![dir.join("suite/b.js"), dir.join("suite/sub/a.js")]));
    assert!(test_files(&dir.join("absent")).is_err(), "a missing folder is skipped silently");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn the_walk_does_not_follow_symbolic_links() {
    let dir = scratch("links");
    std::fs::create_dir_all(dir.join("suite")).unwrap();
    std::fs::create_dir_all(dir.join("outside")).unwrap();
    std::fs::write(dir.join("outside/x.js"), "").unwrap();
    std::fs::write(dir.join("suite/a.js"), "").unwrap();
    std::os::unix::fs::symlink(dir.join("outside"), dir.join("suite/linked")).unwrap();
    std::os::unix::fs::symlink(dir.join("outside/x.js"), dir.join("suite/x.js")).unwrap();
    assert_eq!(test_files(&dir.join("suite")), Ok(vec![dir.join("suite/a.js")]));
    assert!(read_source(&dir.join("suite/x.js")).is_err(), "a linked file is read");
    assert!(inside(&dir.join("suite"), "linked/x.js").is_err(), "a path through a linked folder is allowed");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn harness_names_stay_inside_the_suite() {
    let dir = Path::new("suite");
    assert_eq!(inside(dir, "harness/compareArray.js"), Ok(dir.join("harness").join("compareArray.js")));
    for name in ["harness/../../secret.js", "harness/./a.js", "/etc/passwd", "harness//a.js", "harness/a\\..\\b.js", "harness/C:a.js", ""] {
        assert!(inside(dir, name).is_err(), "{name:?} is allowed");
    }
}

#[test]
fn an_unreadable_annex_b_folder_fails_and_only_an_absent_one_skips() {
    let dir = scratch("present");
    std::fs::create_dir_all(dir.join("test/annexB")).unwrap();
    assert_eq!(present(&dir, "test/annexB"), Ok(Some(dir.join("test").join("annexB"))));
    assert_eq!(present(&dir, "test/annexB/language"), Ok(None));
    // A NUL byte in the path makes its metadata unreadable on every platform.
    assert!(present(Path::new("suite\0dir"), "test/annexB").is_err(), "a path whose metadata cannot be read is skipped as absent");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_annex_b_folder_whose_tests_are_all_skipped_fails() {
    let dir = scratch("all-skipped");
    std::fs::create_dir_all(dir.join("harness")).unwrap();
    std::fs::create_dir_all(dir.join("test/annexB/language")).unwrap();
    std::fs::write(dir.join("harness/assert.js"), "").unwrap();
    std::fs::write(dir.join("harness/sta.js"), "").unwrap();
    std::fs::write(dir.join("test/annexB/language/a.js"), "/*---\nesid: sec-x\nflags: [async]\n---*/\n").unwrap();
    let outcome = std::panic::catch_unwind(|| run_annex_b(&dir));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(outcome.is_err(), "a folder where every test is skipped passes");
}

#[test]
fn an_annex_b_tree_without_its_language_folder_fails() {
    let dir = scratch("no-language");
    std::fs::create_dir_all(dir.join("harness")).unwrap();
    std::fs::create_dir_all(dir.join("test/annexB/built-ins")).unwrap();
    std::fs::write(dir.join("harness/assert.js"), "").unwrap();
    std::fs::write(dir.join("harness/sta.js"), "").unwrap();
    let parsed = std::panic::catch_unwind(|| parse_annex_b(&dir));
    let ran = std::panic::catch_unwind(|| run_annex_b(&dir));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(parsed.is_err(), "the parse test skips a suite without test/annexB/language");
    assert!(ran.is_err(), "the interpreter test skips a suite without test/annexB/language");
}

#[test]
fn a_file_over_the_limit_is_refused_before_it_is_read() {
    let dir = scratch("size");
    let path = dir.join("big.js");
    std::fs::File::create(&path).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
    assert!(read_source(&path).is_err_and(|e| e.contains("over the")), "an oversized file is read");
    std::fs::File::create(&path).unwrap().set_len(4).unwrap();
    assert_eq!(read_source(&path), Ok("\0\0\0\0".to_string()));
    std::fs::remove_dir_all(&dir).unwrap();
}
