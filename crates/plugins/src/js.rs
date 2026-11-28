//! JavaScript plug-ins: an object filter whose manifest gives `"runtime": "js"`, with its code in a
//! JavaScript file instead of a WebAssembly module, run on `vectorcraft-js`.
//!
//! On disk the plug-in is a manifest file (`.vcplugin`, JSON) with `"runtime": "js"` and `script`,
//! the name of its `.js` file in the same folder ([`crate::registry::read_js_package`]). The host
//! loads it as one JSON object, the manifest with the script's text in `source`
//! ([`read_bundle`]): that object is what `plugin.install {dataBase64}` takes, what the plug-in
//! store keeps, and what the SHA-256 a grant records covers, so a change to the manifest or to the
//! script needs a new approval. Loading parses the script and runs none of it.
//!
//! Each run gets a fresh realm with the language's built-ins and a `vc` object: `vc.input`
//! and `vc.params` hold the input and parameter JSON a WebAssembly filter's `vc_run` is given, and
//! with the `progress` permission `vc.progress(done, total)`, `vc.cancelled()` and
//! `vc.log(level, text)` do what `vc_progress`, `vc_cancelled` and `vc_log` do ([`crate::host`]).
//! The realm has no `$`, `File`, `Folder`, `app` or other global of the script runner, no clock
//! (`Date.now()` is `NaN`) and no host function besides these. The script's top level runs, then
//! `run(input, params)` is called with the parsed JSON, and the value it returns, written as JSON,
//! is the run's output, which the host checks as it checks a WebAssembly filter's.
//!
//! A run is held to the filter's instruction budget (one budget for the top level and the call,
//! checked in slices between which the deadline and Cancel are checked), a heap of at most the
//! plug-in's memory cap, and the interpreter's frame and re-entry limits. A thrown error, a limit
//! or a malformed result fails the run with an [`Error`] that names the plug-in, its script and,
//! when known, the line and column.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Map, Value};
use vectorcraft_js::{Data, HostError, HostItem, JsValue, LimitKind, Progress, Realm, RunError, RunErrorKind};

use crate::host::{Deadline, LOG_BAD_LEVEL, LOG_DROPPED, LOG_WRITTEN, Report, Run};
use crate::manifest::{MAX_MANIFEST_BYTES, Manifest, Runtime};
use crate::permissions::{self, Permission};
use crate::runtime::SELECTION_TOO_LARGE;
use crate::{Error, Limits, Result};

/// The extension of a JavaScript plug-in's manifest file.
pub const PACKAGE_EXT: &str = "vcplugin";

/// Largest script, in bytes.
pub const MAX_SCRIPT_BYTES: usize = 8 << 20;

/// Longest script file name, in bytes.
pub const MAX_SCRIPT_NAME: usize = 128;

/// The script's name when the plug-in's JSON gives none.
pub const DEFAULT_SCRIPT: &str = "plugin.js";

/// Instructions a run executes between two checks of its deadline and Cancel.
pub const FUEL_SLICE: u64 = 1_000_000;

/// Longest error message a run reports, in characters.
const MAX_ERROR_TEXT: usize = 512;

/// What `script` may hold.
const SCRIPT_NAME_RULE: &str =
    "`script` must be the name of a .js file in the manifest's folder: 1-128 bytes, no folder, no `:`, not starting with `.`";

/// Where the script's text may be given.
pub const SOURCE_RULE: &str =
    "`source` is the script's text, given only in the JSON that plugin.install's dataBase64 takes: a .vcplugin file names its script in `script`";

/// The error of a plug-in's JSON without `"runtime": "js"`.
pub(crate) const NOT_JS: &str = "a JavaScript plug-in's manifest gives `runtime` \"js\"";

/// What the arguments of `vc.progress` and `vc.log` may be.
const PROGRESS_ARGS: &str = "vc.progress(done, total) takes two numbers";
const LOG_ARGS: &str = "vc.log(level, text) takes a level, 0 (error), 1 (warning), 2 (information) or 3 (debug), and a string";

/// What runs the plug-in: its script's top level is done, `run` is called with the input and the
/// parameters, and what it returns is written as JSON.
const ENTRY: &str = "(function () {\n\
    if (typeof run !== 'function') throw new TypeError('the script defines no function run(input, params)');\n\
    return JSON.stringify(run(JSON.parse(vc.input), JSON.parse(vc.params)));\n\
})()";

/// A JavaScript plug-in's code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Script {
    /// Its file name, which errors give.
    pub(crate) name: String,
    pub(crate) source: String,
}

/// Is `name` a script file name `script` may give: a `.js` file directly in the manifest's folder?
pub fn valid_script_name(name: &str) -> bool {
    (1..=MAX_SCRIPT_NAME).contains(&name.len())
        && !name.starts_with('.')
        && !name.chars().any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
        && name.len() > 3
        && name.get(name.len() - 3..).is_some_and(|ext| ext.eq_ignore_ascii_case(".js"))
}

/// The `script` a manifest's JSON gives, checked.
pub(crate) fn script_name(v: &Value) -> Result<String> {
    match v.as_str() {
        Some(name) if valid_script_name(name) => Ok(name.to_string()),
        _ => Err(Error::Manifest(SCRIPT_NAME_RULE.into())),
    }
}

/// The error of a manifest with `"runtime": "js"` and no `script`.
pub(crate) fn no_script() -> Error {
    Error::Manifest("`runtime` \"js\" needs `script`: the name of the plug-in's .js file, next to the manifest".into())
}

/// Does `map`, a plug-in's JSON, give `"runtime": "js"`?
pub(crate) fn is_js(map: &Map<String, Value>) -> bool {
    map.get("runtime").and_then(Value::as_str) == Some(Runtime::Js.name())
}

/// How the script is read.
fn options() -> vectorcraft_js::Options {
    vectorcraft_js::Options { max_source_units: MAX_SCRIPT_BYTES, ..vectorcraft_js::Options::default() }
}

/// Reads a JavaScript plug-in given as one JSON object: its manifest, with the script's text in
/// `source` and its file name in `script` (left out: [`DEFAULT_SCRIPT`]). The manifest is checked as
/// a module's is, and the script is parsed, not run.
pub(crate) fn read_bundle(bytes: &[u8]) -> Result<(Manifest, Script)> {
    let mut map: Map<String, Value> = serde_json::from_slice(bytes).map_err(|e| Error::Manifest(e.to_string()))?;
    if !is_js(&map) {
        return Err(Error::Manifest(NOT_JS.into()));
    }
    let named = map.remove("script");
    let source = match map.remove("source") {
        Some(Value::String(s)) => s,
        Some(_) => return Err(Error::Manifest(SOURCE_RULE.into())),
        None if named.is_some() => return Err(Error::Manifest(SOURCE_RULE.into())),
        None => return Err(no_script()),
    };
    let name = named.as_ref().map_or_else(|| Ok(DEFAULT_SCRIPT.to_string()), script_name)?;
    if source.len() > MAX_SCRIPT_BYTES {
        return Err(Error::Module(format!("{} is {} bytes (limit {})", name, source.len(), MAX_SCRIPT_BYTES)));
    }
    // The manifest is read from `bytes`, so `params` keep their order; its size is counted
    // without `script` and `source`.
    let size = serde_json::to_vec(&map).map_err(|e| Error::Manifest(e.to_string()))?.len();
    if size > MAX_MANIFEST_BYTES {
        return Err(Error::Manifest(format!("manifest is {size} bytes (limit {MAX_MANIFEST_BYTES})")));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Manifest("manifest is not UTF-8".into()))?;
    let manifest = Manifest::parse_text(text)?;
    if let Err(e) = vectorcraft_js::parse(&source, &options()) {
        let named = Named { id: &manifest.id, script: &name };
        return Err(Error::Module(located(&permissions::display_text(&e.message, MAX_ERROR_TEXT), &named, Some((e.line, e.column)))));
    }
    Ok((manifest, Script { name, source }))
}

/// `reason` followed by where it happened: the plug-in's id, its script's name and, when `at` gives
/// them, the line and column.
fn located(reason: &str, named: &Named<'_>, at: Option<(u32, u32)>) -> String {
    let (id, script) = (named.id, named.script);
    match at {
        Some((line, column)) if line > 0 => format!("{reason} ({id}, {script} line {line}, column {column})"),
        _ => format!("{reason} ({id}, {script})"),
    }
}

/// One run's input and budgets.
pub(crate) struct Call<'a> {
    pub(crate) input: &'a [u8],
    pub(crate) params: &'a [u8],
    pub(crate) limits: &'a Limits,
    /// The plug-in's memory cap, which the run's heap stays within.
    pub(crate) max_memory: usize,
    pub(crate) deadline: Deadline,
}

/// The heap a run may use with memory cap `max_memory`.
fn heap_bytes(max_memory: usize) -> usize {
    vectorcraft_js::Limits::default().heap_bytes.min(max_memory)
}

/// Runs `script`, the code of the plug-in `m` describes, on `call`'s input in a fresh realm → the
/// output JSON. What it reported with `vc.progress` and `vc.log` goes to `report`.
pub(crate) fn run(script: &Script, m: &Manifest, call: &Call<'_>, state: Run, report: &mut Report) -> Result<Vec<u8>> {
    let heap = heap_bytes(call.max_memory);
    // Each UTF-8 byte may become a UTF-16 code unit of two bytes, and the parsed JSON takes more.
    if call.input.len().saturating_add(call.params.len()) > heap / 8 {
        return Err(Error::Limit(SELECTION_TOO_LARGE.into()));
    }
    // The host writes the input and the parameters with serde_json, so they are UTF-8.
    let (input, params) = (String::from_utf8_lossy(call.input), String::from_utf8_lossy(call.params));
    let state = Rc::new(RefCell::new(state));
    let out = run_realm(script, m, (&*input, &*params), call, heap, &state);
    *report = std::mem::take(&mut state.borrow_mut().report);
    out
}

/// What errors name: the plug-in's id and its script's file name.
struct Named<'a> {
    id: &'a str,
    script: &'a str,
}

fn run_realm(
    script: &Script,
    m: &Manifest,
    (input, params): (&str, &str),
    call: &Call<'_>,
    heap: usize,
    state: &Rc<RefCell<Run>>,
) -> Result<Vec<u8>> {
    let defaults = vectorcraft_js::Limits::default();
    let fuel = call.limits.fuel_base.saturating_add((call.input.len() as u64).saturating_mul(call.limits.fuel_per_byte));
    let limits = vectorcraft_js::Limits { fuel, heap_bytes: heap, ..defaults };
    let mut realm = Realm::with_options(options(), limits);
    realm.set_clock(|| f64::NAN);
    let named = Named { id: &m.id, script: &script.name };
    realm.define_host_object("vc", vc_items(input, params, m, state)).map_err(|e| match e.kind {
        RunErrorKind::Limit(LimitKind::Heap) => Error::Limit(SELECTION_TOO_LARGE.into()),
        _ => js_error(&realm, e, vectorcraft_js::UNKNOWN_SOURCE, &named),
    })?;
    let (_, tag) = drive(&mut realm, &script.source, &call.deadline, &named, None)?;
    // The call takes what the top level left of the budget.
    let left = realm.fuel_left();
    realm.set_limits(vectorcraft_js::Limits { fuel: left, ..realm.limits().clone() });
    let (value, _) = drive(&mut realm, ENTRY, &call.deadline, &named, Some(tag))?;
    vectorcraft_doc::cancel::check().map_err(|_| Error::Cancelled)?;
    call.deadline.check()?;
    match value {
        JsValue::String(s) if s.len() > call.limits.max_output_bytes => {
            let (what, len, max) = ("the output", s.len(), call.limits.max_output_bytes);
            Err(Error::Limit(format!("output size ({what} is {len} bytes, limit {max})")))
        }
        JsValue::String(s) => Ok(s.into_bytes()),
        _ => Err(Error::Output(located(NO_OUTPUT, &named, None))),
    }
}

/// The error of a `run` that returned nothing JSON can write.
const NO_OUTPUT: &str = "run(input, params) returned no output object";

/// The limit a run that nested its calls too deeply exceeded.
const CALL_DEPTH: &str = "call depth";

/// Runs `source` in `realm` to its end, in slices of [`FUEL_SLICE`] with the deadline and Cancel
/// checked between them → its completion value, and the source tag of the plug-in's script:
/// `script_tag`, or with `None` the tag of `source`, which is then the script.
fn drive(realm: &mut Realm, source: &str, deadline: &Deadline, named: &Named<'_>, script_tag: Option<u32>) -> Result<(JsValue, u32)> {
    let started = realm.start(source);
    let tag = script_tag.unwrap_or_else(|| realm.script_tag());
    started.map_err(|e| js_error(realm, e, tag, named))?;
    loop {
        match realm.advance(FUEL_SLICE) {
            Ok(Progress::Done(v)) => return Ok((v, tag)),
            Ok(Progress::Paused) => {}
            Err(e) => return Err(js_error(realm, e, tag, named)),
        }
        if vectorcraft_doc::cancel::check().is_err() {
            realm.end();
            return Err(Error::Cancelled);
        }
        if let Err(e) = deadline.check() {
            realm.end();
            return Err(e);
        }
    }
}

/// The plug-in error for `e`, which a run of `realm` ended with: it names the plug-in and the script,
/// and the line and column when the error is in the script (source tag `tag`).
fn js_error(realm: &Realm, e: RunError, tag: u32, named: &Named<'_>) -> Error {
    let at = (realm.error_tag() == tag).then_some((e.line, e.column));
    let message = permissions::display_text(&e.message, MAX_ERROR_TEXT);
    match e.kind {
        RunErrorKind::Syntax => Error::Module(located(&message, named, at)),
        RunErrorKind::Thrown | RunErrorKind::Limit(LimitKind::Host) => Error::Failed(located(&message, named, at)),
        RunErrorKind::Limit(LimitKind::Jobs) => Error::Limit(located(&message, named, at)),
        RunErrorKind::Limit(LimitKind::Fuel | LimitKind::Unbroken) => Error::Limit(located("instruction budget", named, at)),
        RunErrorKind::Limit(LimitKind::Heap) => Error::Limit(located("memory budget", named, at)),
        RunErrorKind::Limit(LimitKind::Frames | LimitKind::Reentry) => Error::Limit(located(CALL_DEPTH, named, at)),
        RunErrorKind::Limit(LimitKind::Interrupted) => Error::Cancelled,
        RunErrorKind::Internal => Error::Trap(located(&message, named, at)),
    }
}

/// The members of the realm's `vc` object.
fn vc_items(input: &str, params: &str, m: &Manifest, state: &Rc<RefCell<Run>>) -> Vec<(String, HostItem)> {
    let mut items = vec![
        ("input".to_string(), HostItem::Value(Data::String(input.into()))),
        ("params".to_string(), HostItem::Value(Data::String(params.into()))),
    ];
    if !m.permissions().contains(&Permission::Progress) {
        return items;
    }
    let number = |args: &[Data], i: usize, rule: &str| match args.get(i) {
        Some(Data::Number(n)) => Ok(*n),
        _ => Err(HostError::Type(rule.into())),
    };
    let run = state.clone();
    items.push((
        "progress".into(),
        HostItem::Function(Rc::new(move |args| {
            let (done, total) = (number(&args, 0, PROGRESS_ARGS)?, number(&args, 1, PROGRESS_ARGS)?);
            // `as` turns NaN into 0 and clamps the rest, as `vc_progress` clamps negative values.
            run.borrow_mut().progress(done as i64, total as i64);
            Ok(Data::Undefined)
        })),
    ));
    let run = state.clone();
    items.push(("cancelled".into(), HostItem::Function(Rc::new(move |_| Ok(Data::Bool(run.borrow().cancelled() == 1))))));
    let run = state.clone();
    items.push((
        "log".into(),
        HostItem::Function(Rc::new(move |args| {
            let level = number(&args, 0, LOG_ARGS)?;
            let Some(Data::String(text)) = args.get(1) else { return Err(HostError::Type(LOG_ARGS.into())) };
            let level = if level.fract() == 0.0 && (0.0..=3.0).contains(&level) { level as i32 } else { LOG_BAD_LEVEL };
            match run.borrow_mut().log(level, text.as_bytes(), 0, text.len()) {
                LOG_WRITTEN => Ok(Data::Bool(true)),
                LOG_DROPPED => Ok(Data::Bool(false)),
                _ => Err(HostError::Range(LOG_ARGS.into())),
            }
        })),
    ));
    items
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::{Error, Limits, Plugin};

    /// A JavaScript filter `id` with `source` as its code, and `extra` manifest keys.
    fn bundle(id: &str, source: &str, extra: Value) -> Vec<u8> {
        let mut m = json!({"id": id, "name": "JS test", "kind": "filter", "runtime": "js", "script": "test.js", "source": source});
        if let (Some(m), Value::Object(extra)) = (m.as_object_mut(), extra) {
            m.extend(extra);
        }
        serde_json::to_vec(&m).unwrap()
    }

    fn load(bytes: &[u8]) -> crate::Result<Plugin> {
        load_with(bytes, Limits::default())
    }

    fn load_with(bytes: &[u8], limits: Limits) -> crate::Result<Plugin> {
        let _lock = crate::safe_mode::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        Plugin::load(bytes, limits)
    }

    const PATH: &str = r#"{"objects":[{"id":7,"type":"path","fills":[{"type":"solid","color":{"model":"rgb","r":1,"g":0,"b":0}}]}]}"#;

    /// The output of `run(input, params)` is the run's output JSON.
    #[test]
    fn a_filter_returns_its_objects_as_json() {
        let src = "function run(input, params) { return {objects: input.objects.map(function (o) { \
                   return {id: o.id, fills: [{type: 'solid', color: {model: 'rgb', r: 0, g: 0, b: params.blue}}]}; })}; }";
        let p = load(&bundle("test.js.blue", src, json!({}))).unwrap();
        assert_eq!((p.runtime(), p.script()), (crate::Runtime::Js, Some("test.js")));
        let out: Value = serde_json::from_slice(&p.run(PATH.as_bytes(), br#"{"blue":1}"#).unwrap()).unwrap();
        assert_eq!(out, json!({"objects": [{"id": 7, "fills": [{"type": "solid", "color": {"model": "rgb", "r": 0, "g": 0, "b": 1}}]}]}));
    }

    /// The realm has none of the script runner's globals, so code that reads them throws a
    /// `ReferenceError` naming the global, the plug-in and where in its script.
    #[test]
    fn file_folder_and_app_are_not_defined() {
        for name in ["File", "Folder", "$", "app", "BridgeTalk"] {
            let src = format!("function run(input) {{\n  return {name}.x;\n}}");
            let e = load(&bundle("test.js.globals", &src, json!({}))).unwrap().run(PATH.as_bytes(), b"{}").unwrap_err();
            let Error::Failed(why) = &e else { panic!("{name}: {e:?}") };
            assert!(why.starts_with("ReferenceError") && why.contains(name), "{name}: {why}");
            assert!(why.ends_with("(test.js.globals, test.js line 2, column 10)"), "{name}: {why}");
        }
        let p = load(&bundle("test.js.clock", "function run() { return {objects: [], n: String(Date.now())}; }", json!({}))).unwrap();
        let out: Value = serde_json::from_slice(&p.run(b"{\"objects\":[]}", b"{}").unwrap()).unwrap();
        assert_eq!(out["n"], "NaN", "no clock");
    }

    /// Each run has a realm of its own: what one run changes in the built-ins, the next run doesn't
    /// see.
    #[test]
    fn a_polluted_prototype_does_not_reach_the_next_run() {
        let src = "function run() { var seen = Object.prototype.polluted; Object.prototype.polluted = 1; \
                   Array.prototype.map = null; return {objects: [], seen: String(seen)}; }";
        let p = load(&bundle("test.js.pollute", src, json!({}))).unwrap();
        for _ in 0..2 {
            let out: Value = serde_json::from_slice(&p.run(b"{\"objects\":[]}", b"{}").unwrap()).unwrap();
            assert_eq!(out["seen"], "undefined");
        }
    }

    /// An endless loop ends with the instruction budget, naming the plug-in and the line.
    #[test]
    fn an_endless_loop_ends_with_the_instruction_budget() {
        let p = load(&bundle("test.js.loop", "function run() {\n  for (;;) {}\n}", json!({}))).unwrap();
        let limits = Limits { fuel_base: 3 * super::FUEL_SLICE, fuel_per_byte: 0, ..Limits::default() };
        let e = p.run_limited(b"{\"objects\":[]}", b"{}", &limits).unwrap_err();
        assert!(matches!(&e, Error::Limit(why) if why.starts_with("instruction budget (test.js.loop, test.js line 2")), "{e:?}");
    }

    /// A plug-in that fills its heap ends with its memory budget, and a realm another caller holds
    /// meanwhile, such as a script's, runs on as before.
    #[test]
    fn heap_exhaustion_leaves_a_held_realm_usable() {
        let mut held = vectorcraft_js::Realm::new();
        held.run("var kept = 41;").unwrap();
        let src = "function run() { var a = []; for (;;) { a.push(new Array(1000).join('x') + a.length); } }";
        let small = load_with(&bundle("test.js.heap", src, json!({})), Limits { max_memory_bytes: 8 << 20, ..Limits::default() }).unwrap();
        let e = small.run(b"{\"objects\":[]}", b"{}").unwrap_err();
        assert!(matches!(&e, Error::Limit(why) if why.starts_with("memory budget (test.js.heap")), "{e:?}");
        assert_eq!(held.run("kept + 1").unwrap(), vectorcraft_js::JsValue::Number(42.0));
    }

    /// An input too large for the heap is refused before it is copied in.
    #[test]
    fn a_selection_too_large_for_the_heap_is_refused() {
        let small =
            load_with(&bundle("test.js.big", "function run(i) { return i; }", json!({})), Limits { max_memory_bytes: 1 << 20, ..Limits::default() })
                .unwrap();
        let input = format!(r#"{{"objects":[],"pad":"{}"}}"#, "x".repeat(200_000));
        assert_eq!(small.run(input.as_bytes(), b"{}"), Err(Error::Limit(crate::runtime::SELECTION_TOO_LARGE.into())));
    }

    /// A result that isn't an output object, and a script without `run`, fail the run.
    #[test]
    fn a_missing_entry_or_result_fails_the_run() {
        let e = load(&bundle("test.js.none", "var x = 1;", json!({}))).unwrap().run(b"{\"objects\":[]}", b"{}").unwrap_err();
        assert!(matches!(&e, Error::Failed(why) if why.contains("defines no function run")), "{e:?}");
        let e = load(&bundle("test.js.undef", "function run() {}", json!({}))).unwrap().run(b"{\"objects\":[]}", b"{}").unwrap_err();
        assert!(matches!(&e, Error::Output(why) if why.contains("test.js.undef")), "{e:?}");
        let e = load(&bundle("test.js.cycle", "function run() { var o = {}; o.o = o; return o; }", json!({})))
            .unwrap()
            .run(b"{\"objects\":[]}", b"{}")
            .unwrap_err();
        assert!(matches!(&e, Error::Failed(why) if why.starts_with("TypeError")), "{e:?}");
    }

    /// With `progress`, `vc.progress` and `vc.log` report as their imports do; without it, `vc`
    /// has neither.
    #[test]
    fn progress_and_log_need_the_progress_permission() {
        let src = "function run() { vc.progress(3, 4); vc.log(2, 'half'); return {objects: [], cancelled: vc.cancelled()}; }";
        let p = load(&bundle("test.js.progress", src, json!({"api": 2, "permissions": ["progress"]}))).unwrap();
        let (out, report) = p.run_reporting(b"{\"objects\":[]}", b"{}", p.limits());
        let out: Value = serde_json::from_slice(&out.unwrap()).unwrap();
        assert_eq!(out["cancelled"], false);
        assert_eq!(report.progress, Some((3, 4)));
        assert_eq!(report.lines, vec![crate::LogLine { level: crate::LogLevel::Info, text: "half".into() }]);
        let e = load(&bundle("test.js.quiet", src, json!({}))).unwrap().run(b"{\"objects\":[]}", b"{}").unwrap_err();
        assert!(matches!(&e, Error::Failed(why) if why.starts_with("TypeError")), "{e:?}");
    }

    /// Loading checks the manifest and the script, and runs none of it.
    #[test]
    fn loading_refuses_a_bad_manifest_or_script() {
        let ok = "function run() { return {objects: []}; }";
        let refused = |bytes: Vec<u8>| load(&bytes).unwrap_err();
        let e = refused(bundle("test.js.syntax", "function run( {", json!({})));
        assert!(matches!(&e, Error::Module(why) if why.contains("(test.js.syntax, test.js line 1, column")), "{e:?}");
        let big = "x".repeat(super::MAX_SCRIPT_BYTES + 1);
        assert!(matches!(refused(bundle("test.js.big", &big, json!({}))), Error::Module(why) if why.starts_with("test.js is")));
        let e = refused(bundle("test.js.effect", ok, json!({"kind": "effect"})));
        assert!(matches!(&e, Error::Manifest(why) if why.contains("`runtime` \"js\" is for an object filter")), "{e:?}");
        let e = refused(bundle("test.js.settings", ok, json!({"api": 2, "permissions": ["settings"]})));
        assert!(matches!(&e, Error::Manifest(why) if why.contains("`settings`")), "{e:?}");
        let e = refused(bundle("test.js.events", ok, json!({"api": 2, "permissions": ["events"], "events": ["selection.changed"]})));
        assert!(matches!(&e, Error::Manifest(why) if why.contains("can't list the permission `events`")), "{e:?}");
        let e = refused(bundle("test.js.name", ok, json!({"script": "../x.js"})));
        assert!(matches!(&e, Error::Manifest(why) if why.starts_with("`script`")), "{e:?}");
        let e = refused(bundle("test.js.py", ok, json!({"runtime": "py"})));
        assert!(matches!(&e, Error::Manifest(why) if why == super::NOT_JS), "{e:?}");
        let mut no_source: Value = serde_json::from_slice(&bundle("test.js.nosource", ok, json!({}))).unwrap();
        no_source.as_object_mut().unwrap().remove("source");
        let e = refused(serde_json::to_vec(&no_source).unwrap());
        assert!(matches!(&e, Error::Manifest(why) if why.contains("`source`")), "{e:?}");
        // Top-level code that would throw does not run at load.
        assert!(load(&bundle("test.js.lazy", "throw new Error('ran');", json!({}))).is_ok());
    }

    /// A WebAssembly module's manifest can't give `"runtime": "js"`.
    #[test]
    fn a_module_manifest_cannot_be_javascript() {
        let manifest = r#"{"id":"test.js.wasm","name":"T","kind":"filter","runtime":"js"}"#;
        let bytes = wat::parse_str(crate::wat::returning(manifest, r#"{"objects":[]}"#)).unwrap();
        assert!(matches!(load(&bytes), Err(Error::Manifest(why)) if why.contains("`runtime`")));
    }

    /// The SHA-256 a grant records is the bundle's, so another script is another module.
    #[test]
    fn the_hash_covers_the_script() {
        let a = load(&bundle("test.js.hash", "function run() { return {objects: []}; }", json!({}))).unwrap();
        let b = load(&bundle("test.js.hash", "function run() { return {objects: [] }; }", json!({}))).unwrap();
        assert_ne!(a.hash(), b.hash());
    }

    /// The parameters, and a command's, keep the order the plug-in's JSON declares them in.
    #[test]
    fn params_keep_their_declaration_order() {
        let num = r#"{"type": "number", "min": 0, "max": 10, "default": 1}"#;
        let text = format!(
            r#"{{"id": "test.js.order", "name": "JS test", "kind": "filter", "runtime": "js", "script": "test.js",
                "params": {{"z": {num}, "a": {num}, "m": {num}}},
                "commands": [{{"id": "go", "label": "Go", "params": {{"y": {num}, "b": {num}}}}}],
                "source": "function run(i) {{ return i; }}"}}"#
        );
        let (manifest, _) = super::read_bundle(text.as_bytes()).unwrap();
        let names = |p: &[(String, crate::manifest::Param)]| p.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
        assert_eq!(names(&manifest.params), ["z", "a", "m"]);
        assert_eq!(names(&manifest.commands[0].params), ["y", "b"]);
    }

    #[test]
    fn script_names_stay_in_the_manifest_s_folder() {
        for ok in ["tint.js", "Tint Tool.JS", "a.b.js"] {
            assert!(super::valid_script_name(ok), "{ok}");
        }
        for bad in ["", ".js", ".hidden.js", "../x.js", "sub/x.js", "sub\\x.js", "c:x.js", "x.jsx", "x.js\n", "x.wasm"] {
            assert!(!super::valid_script_name(bad), "{bad:?}");
        }
        assert!(!super::valid_script_name(&format!("{}.js", "a".repeat(126))));
    }
}
