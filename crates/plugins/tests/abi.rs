//! The plug-in ABI end to end: the example Desaturate plug-in, and hostile modules (infinite
//! loops, memory bombs, out-of-bounds access, bad manifests and outputs…), which must all fail
//! with an error, quickly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_plugins::{Error, Kind, Limits, Plugin, registry, wat as tpl};

/// The example Desaturate plug-in, built from `crates/plugins/example`.
const DESATURATE: &[u8] = include_bytes!("fixtures/desaturate.wasm");

const MANIFEST: &str = r#"{"id":"test.plugin","name":"Test","version":"1","kind":"filter"}"#;

fn build(src: &str) -> Vec<u8> {
    wat::parse_str(src).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{src}"))
}

/// A filter whose `vc_run` body is `run`, with `extra` module fields.
fn module(manifest: &str, run: &str, extra: &str) -> Vec<u8> {
    build(&tpl::module(manifest, run, extra, 1))
}

fn ok_module(run: &str) -> Vec<u8> {
    module(MANIFEST, run, "")
}

const INPUT: &str = r#"{"objects":[]}"#;

/// Runs `bytes` on an empty input; returns the error and how long it took.
fn run_err(bytes: &[u8], limits: Limits) -> (Error, Duration) {
    let t = Instant::now();
    let r = Plugin::load(bytes, limits).and_then(|p| p.run(INPUT.as_bytes(), b"{}"));
    (r.expect_err("hostile module must fail"), t.elapsed())
}

#[test]
fn example_desaturate_loads_and_reports_its_manifest() {
    let p = Plugin::load(DESATURATE, Limits::default()).unwrap();
    let m = p.manifest();
    assert_eq!(m.id, "org.vectorcraft.example.desaturate");
    assert_eq!(m.name, "Desaturate");
    assert_eq!(m.kind, Kind::Filter);
    assert_eq!(m.params_doc(), "{amount: 0..100 (100)}");
}

#[test]
fn example_desaturate_moves_every_colour_model_to_grey() {
    let p = Plugin::load(DESATURATE, Limits::default()).unwrap();
    let input = json!({"objects": [{"id": 3, "type": "path", "fills": [
        {"type": "solid", "color": {"model": "rgb", "r": 1.0, "g": 0.0, "b": 0.0}},
        {"type": "gradient", "gradient": {"kind": "Linear", "stops": [
            {"offset": 0.0, "color": {"model": "lab", "l": 50.0, "a": 40.0, "b": -20.0}},
            {"offset": 1.0, "color": {"model": "cmyk", "c": 0.9, "m": 0.0, "y": 0.3, "k": 0.1}}]}}]}]});
    let out = p.run(input.to_string().as_bytes(), br#"{"amount": 100}"#).unwrap();
    let out: Value = serde_json::from_slice(&out).unwrap();
    let fills = &out["objects"][0]["fills"];
    let rgb = &fills[0]["color"];
    assert!((rgb["r"].as_f64().unwrap() - 0.2126).abs() < 1e-9 && rgb["r"] == rgb["g"] && rgb["g"] == rgb["b"], "{rgb}");
    let stops = &fills[1]["gradient"]["stops"];
    assert_eq!((stops[0]["color"]["a"].as_f64(), stops[0]["color"]["b"].as_f64()), (Some(0.0), Some(0.0)));
    let cmyk = &stops[1]["color"];
    assert!((cmyk["c"].as_f64().unwrap() - 0.4).abs() < 1e-9 && cmyk["k"] == 0.1, "{cmyk}");
    assert_eq!(out["objects"][0]["id"], 3, "ids come back, so the objects are updated in place");
    // Half way.
    let half: Value = serde_json::from_slice(&p.run(input.to_string().as_bytes(), br#"{"amount": 50}"#).unwrap()).unwrap();
    assert!((half["objects"][0]["fills"][0]["color"]["r"].as_f64().unwrap() - 0.6063).abs() < 1e-9);
    // Nothing to work on: the plug-in's own message.
    let none = p.run(INPUT.as_bytes(), b"{}").unwrap();
    assert!(matches!(vectorcraft_plugins::objects::decode(&none), Err(Error::Failed(m)) if m.contains("Select some paths")));
}

#[test]
fn input_and_params_reach_the_plugin_and_output_comes_back() {
    let p = Plugin::load(&build(&tpl::module(MANIFEST, tpl::ECHO, "", 1)), Limits::default()).unwrap();
    let input = br#"{"objects":[{"id":1}]}"#;
    assert_eq!(p.run(input, b"{}").unwrap(), input);
    // Returns the parameters instead.
    let params = module(MANIFEST, "(i64.or (i64.shl (i64.extend_i32_u (local.get $pl)) (i64.const 32)) (i64.extend_i32_u (local.get $pp)))", "");
    let p = Plugin::load(&params, Limits::default()).unwrap();
    assert_eq!(p.run(input, br#"{"amount":5}"#).unwrap(), br#"{"amount":5}"#);
    let fixed = Plugin::load(&build(&tpl::returning(MANIFEST, r#"{"objects":[]}"#)), Limits::default()).unwrap();
    assert_eq!(fixed.run(input, b"{}").unwrap(), br#"{"objects":[]}"#);
}

#[test]
fn hostile_modules_fail_quickly() {
    let quick = Duration::from_secs(5);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("infinite loop", ok_module("(loop $l (br $l)) (i64.const 0)")),
        ("memory.grow bomb, then trap", ok_module("(loop $l (br_if $l (i32.ne (memory.grow (i32.const 512)) (i32.const -1)))) unreachable")),
        ("memory.grow forever", ok_module("(loop $l (drop (memory.grow (i32.const 1))) (br $l)) (i64.const 0)")),
        ("out-of-bounds write", ok_module("(i32.store (i32.const -4) (i32.const 0)) (i64.const 0)")),
        ("trap", ok_module("unreachable")),
        ("division by zero", ok_module("(drop (i32.div_u (i32.const 1) (i32.const 0))) (i64.const 0)")),
        ("deep recursion", module(MANIFEST, "(call $r) (i64.const 0)", "(func $r (call $r))")),
        ("error code", ok_module("(i64.const -7)")),
        ("output outside memory", ok_module("(i64.const 0x00000100fffffff0)")),
        ("huge output", ok_module("(i64.const 0x7fffffff00000000)")),
        (
            "wrong ABI version",
            build(&tpl::module(MANIFEST, "(i64.const 0)", "", 1).replace("(result i32) (i32.const 1))", "(result i32) (i32.const 2))")),
        ),
        ("bad manifest", module("not json", "(i64.const 0)", "")),
        ("manifest with a bad id", module(r#"{"id":"a b","name":"x","kind":"filter"}"#, "(i64.const 0)", "")),
        ("manifest of the wrong kind", module(r#"{"id":"a","name":"x","kind":"panel"}"#, "(i64.const 0)", "")),
        ("start function loops", module(MANIFEST, "(i64.const 0)", "(func $spin (loop $l (br $l))) (start $spin)")),
        (
            "WASI import",
            module(MANIFEST, "(i64.const 0)", r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#),
        ),
    ];
    for (name, bytes) in cases {
        let (e, t) = run_err(&bytes, Limits::default());
        assert!(t < quick, "{name} took {t:?}");
        eprintln!("{name}: {e} ({t:?})");
    }
}

#[test]
fn malformed_and_oversized_modules_are_refused() {
    let half = &DESATURATE[..DESATURATE.len() / 2];
    for (name, bytes) in [("empty", &b""[..]), ("garbage", &b"\0asm\x01\0\0\0\xff\xff\xff"[..]), ("not wasm", &b"MZ\x90\0"[..]), ("truncated", half)]
    {
        assert!(matches!(Plugin::load(bytes, Limits::default()), Err(Error::Module(_))), "{name}");
    }
    let tiny = Limits { max_module_bytes: 100, ..Limits::default() };
    assert!(matches!(Plugin::load(DESATURATE, tiny), Err(Error::Module(_))));
    // Huge initial memory, manifest outside memory, missing exports, bogus allocations.
    let huge = build(r#"(module (memory (export "memory") 60000))"#);
    assert!(Plugin::load(&huge, Limits::default()).is_err());
    let no_run = build(
        r#"(module (memory (export "memory") 1) (func (export "vc_abi_version") (result i32) (i32.const 1))
           (func (export "vc_manifest") (result i64) (i64.const 0)))"#,
    );
    assert!(Plugin::load(&no_run, Limits::default()).is_err());
    let src = r#"(module (memory (export "memory") 1) (func (export "vc_abi_version") (result i32) (i32.const 1))
           (func (export "vc_manifest") (result i64) (i64.const 0x00000064ffffff00))
           (func (export "vc_alloc") (param i32) (result i32) (i32.const 16))
           (func (export "vc_run") (param i32 i32 i32 i32) (result i64) (i64.const 0)))"#;
    assert!(matches!(Plugin::load(&build(src), Limits::default()), Err(Error::Abi(_))));
    let bogus_alloc = build(&tpl::module(MANIFEST, "(i64.const 0)", "", 1).replace(
        "(func (export \"vc_alloc\") (param $n i32) (result i32) (local $old i32)",
        "(func (export \"vc_alloc\") (param $n i32) (result i32) (local $old i32) (return (i32.const -16))",
    ));
    let (e, _) = run_err(&bogus_alloc, Limits::default());
    assert!(matches!(e, Error::Abi(_)), "{e}");
}

#[test]
fn budgets_stop_runaway_plugins() {
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    let (e, _) = run_err(&spin, Limits { fuel_base: 1_000_000, fuel_per_byte: 10, ..Limits::default() });
    assert_eq!(e, Error::Limit("instruction budget".into()));
}

#[test]
fn wall_clock_budget_stops_runaway_plugins() {
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    // Wall-clock budget, with an effectively unlimited instruction budget.
    let (e, t) = run_err(&spin, Limits { fuel_base: u64::MAX / 4, wall_time_ms: 300, ..Limits::default() });
    assert_eq!(e, Error::Limit("time budget".into()));
    assert!(t < Duration::from_secs(3), "{t:?}");
}

#[test]
fn cancelling_stops_a_run_between_two_slices_of_its_budget() {
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    let p = Plugin::load(&spin, Limits { fuel_base: u64::MAX / 4, ..Limits::default() }).unwrap();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let t = Instant::now();
    let r = vectorcraft_doc::cancel::scope(&token, None, || p.run(INPUT.as_bytes(), b"{}"));
    assert_eq!(r, Err(Error::Cancelled));
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    // Outside a cancelled scope the same module still runs to its budget.
    let (e, _) = run_err(&spin, Limits { fuel_base: 1_000_000, ..Limits::default() });
    assert_eq!(e, Error::Limit("instruction budget".into()));
}

#[test]
fn an_exporter_returns_its_file_with_an_export_s_budget() {
    use vectorcraft_plugins::export::{self, Target};
    let target = Target { extension: "tst", name: "Poster" };
    let manifest = tpl::exporter("test.exporter", r#"["tst"]"#, r#"{"level":{"type":"int","min":1,"max":3,"default":2}}"#);
    let p = Plugin::load(&build(&tpl::returning(&manifest, r#"{"dataBase64":"SGk=","warnings":["note"]}"#)), Limits::default()).unwrap();
    assert_eq!((p.manifest().kind, p.manifest().extensions.clone()), (Kind::Exporter, vec!["tst".to_string()]));
    let out = export::run(&p, br#"{"format":"vectorcraft"}"#, &json!({"level": 9}), target).unwrap();
    assert_eq!((out.data_base64.as_str(), out.warnings), ("SGk=", vec!["note".to_string()]));
    let limits = export::limits(&p);
    assert_eq!((limits.fuel_base, limits.wall_time_ms), (export::FUEL, export::WALL_TIME_MS));
    // Its parameters are checked, and a filter is not an exporter.
    assert!(matches!(export::run(&p, b"{}", &json!({"level": "high"}), target), Err(Error::Params(_))));
    let filter = Plugin::load(&ok_module("(i64.const 0)"), Limits::default()).unwrap();
    assert!(matches!(export::run(&filter, b"{}", &Value::Null, target), Err(Error::Params(_))));
}

/// The echo exporter the engine and UI tests read back what they hand a plug-in with: its file is
/// its input, or its parameters, byte for byte, whatever their length modulo three.
#[test]
fn the_echo_exporter_writes_what_it_is_given() {
    use vectorcraft_plugins::export::{self, Target};
    let manifest = tpl::exporter("test.echo", r#"["txt"]"#, "{}");
    let input = Plugin::load(&build(&tpl::echo_exporter(&manifest, tpl::Echo::Input)), Limits::default()).unwrap();
    let params = Plugin::load(&build(&tpl::echo_exporter(&manifest, tpl::Echo::Params)), Limits::default()).unwrap();
    let target = Target { extension: "txt", name: "a" };
    let decoded = |out: export::Exported| String::from_utf8(base64_decode(&out.data_base64)).unwrap();
    for doc in ["{}", r#"{"a":1}"#, r#"{"ab":12}"#, r#"{"format":"vectorcraft","é":"\u0000"}"#] {
        assert_eq!(decoded(export::run(&input, doc.as_bytes(), &Value::Null, target).unwrap()), doc);
    }
    let sent: Value = serde_json::from_str(&decoded(export::run(&params, b"{}", &Value::Null, target).unwrap())).unwrap();
    assert_eq!(sent, json!({"_context": {"mode": "export", "extension": "txt", "name": "a"}}));
}

/// Standard base64 (RFC 4648, padded), decoded here so the test needs no other crate.
fn base64_decode(s: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let sextets: Vec<u32> = s.bytes().filter(|b| *b != b'=').map(|b| ALPHABET.iter().position(|a| *a == b).unwrap() as u32).collect();
    let mut out = Vec::new();
    for group in sextets.chunks(4) {
        let bits = group.iter().enumerate().fold(0u32, |acc, (i, v)| acc | v << (18 - 6 * i));
        out.extend(bits.to_be_bytes()[1..].iter().take(group.len() - 1));
    }
    out
}

#[test]
fn memory_cap_limits_input_and_growth() {
    let echo = build(&tpl::module(MANIFEST, tpl::ECHO, "", 1));
    let p = Plugin::load(&echo, Limits { max_memory_bytes: 1 << 20, ..Limits::default() }).unwrap();
    assert!(p.run(&vec![b' '; 300 << 10], b"{}").is_ok(), "fits in half the memory");
    assert!(matches!(p.run(&vec![b' '; 600 << 10], b"{}"), Err(Error::Limit(_))), "doesn't");
    // Growing past the cap fails inside the module (memory.grow returns -1): vc_alloc gives 0.
    let p = Plugin::load(&echo, Limits { max_memory_bytes: 64 << 10, ..Limits::default() }).unwrap();
    assert!(matches!(p.run(b"{}", b"{}"), Err(Error::Failed(m)) if m.contains("vc_alloc")));
}

#[test]
fn registry_install_list_remove() {
    // Its own id: the registry is process-wide and tests run in parallel.
    let id = "test.registry";
    let p = registry::install_bytes(&module(r#"{"id":"test.registry","name":"R","kind":"filter"}"#, "(i64.const 0)", "")).unwrap();
    assert_eq!(p.id(), id);
    let rev = registry::revision();
    assert!(registry::list().iter().any(|p| p.id() == id));
    assert!(registry::get(id).is_some());
    assert!(registry::remove(id));
    assert!(!registry::remove(id));
    assert!(registry::get(id).is_none());
    assert!(registry::revision() > rev);
    assert!(std::ptr::eq(registry::intern("Abc"), registry::intern("Abc")), "interned once");
}

#[test]
fn folder_loading_skips_bad_files() {
    let dir = std::env::temp_dir().join(format!("vectorcraft-plugins-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a-desaturate.wasm"), DESATURATE).unwrap();
    std::fs::write(dir.join("b-broken.wasm"), b"\0asm junk").unwrap();
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
    let r = registry::load_folder(&dir).unwrap();
    assert_eq!(r.loaded, ["org.vectorcraft.example.desaturate"]);
    assert_eq!(r.failed.len(), 1);
    assert!(r.failed[0].0.ends_with("b-broken.wasm"));
    assert!(registry::load_folder(&dir.join("missing")).is_err());
    assert!(registry::load_file(&dir, Limits::default()).is_err(), "a directory is not a module");
    std::fs::remove_dir_all(&dir).ok();
}
