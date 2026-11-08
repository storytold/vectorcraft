//! The plug-in ABI end to end: the example Desaturate plug-in, and hostile modules (infinite
//! loops, memory bombs, out-of-bounds access, bad manifests and outputs…), which must all fail
//! with an error, quickly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_plugins::{Error, HEAVY_FUEL, HEAVY_WALL_TIME_MS, Kind, Limits, Plugin, Report, heavy_terms, registry, wat as tpl};

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

/// A filter that runs `op`, an 8 MiB bulk memory instruction costing about 131,000 instructions,
/// 160 times (about 21 million instructions in all), or forever when `endless`.
fn bulk_module(op: &str, endless: bool) -> Vec<u8> {
    let bulk = match op {
        "fill" => "(memory.fill (i32.const 1024) (i32.const 7) (i32.const 8388608))",
        "copy" => "(memory.copy (i32.const 1024) (i32.const 2048) (i32.const 8387584))",
        _ => panic!("no bulk operation `{op}`"),
    };
    let run = if endless {
        format!("(loop $work {bulk} (br $work)) {}", tpl::ECHO)
    } else {
        format!(
            "(local $n i32) (local.set $n (i32.const 160)) \
             (loop $work {bulk} (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $work (local.get $n))) {}",
            tpl::ECHO
        )
    };
    build(&tpl::module(MANIFEST, &run, "", 130))
}

fn bulk_run(op: &str, endless: bool, fuel_base: u64) -> vectorcraft_plugins::Result<Vec<u8>> {
    let p = Plugin::load(&bulk_module(op, endless), Limits { fuel_base, fuel_per_byte: 0, ..Limits::default() })?;
    p.run(INPUT.as_bytes(), b"{}")
}

/// A bulk memory instruction that the fuel left in a slice of the budget can't pay for pauses the
/// run without using that fuel, and the run keeps it: 160 fills or copies that cross the first
/// slice finish within 21 million instructions, though the pause left about 75,000 of them in the
/// slice. With fewer instructions than the work takes, or with a loop that never ends, the run
/// still ends with the instruction budget.
#[test]
fn a_bulk_memory_instruction_at_the_end_of_a_slice_keeps_the_run_s_fuel() {
    for op in ["fill", "copy"] {
        assert_eq!(bulk_run(op, false, 21_000_000), Ok(INPUT.as_bytes().to_vec()), "{op} within its budget");
        assert_eq!(bulk_run(op, false, 15_000_000), Err(Error::Limit("instruction budget".into())), "{op} over its budget");
        assert_eq!(bulk_run(op, true, 21_000_000), Err(Error::Limit("instruction budget".into())), "endless {op}");
    }
}

#[test]
fn wall_clock_budget_stops_runaway_plugins() {
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    // Wall-clock budget, with an effectively unlimited instruction budget.
    let (e, t) = run_err(&spin, Limits { fuel_base: u64::MAX / 4, wall_time_ms: 300, ..Limits::default() });
    assert_eq!(e, Error::Limit("time budget".into()));
    assert!(t < Duration::from_secs(3), "{t:?}");
}

/// Cancelling stops a run where one slice of its budget ends: an endless loop given all but
/// unlimited instructions comes back `Cancelled` within seconds, where outside a cancelled scope the
/// same module runs until its budget is spent.
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

/// Compiling a plug-in's code takes none of a run's instructions, so a first run takes as many as the
/// next, though wasmi compiles each function only when it is first called. When the compiling was the
/// run's to pay for, and the slice of the budget the run was in ran out while a function compiled,
/// wasmi ended the run with an error the host can't resume: it failed with most of its budget left.
/// Here `vc_run` calls a function about 100 KB long, a `br_table` of 100,000 labels.
#[test]
fn a_first_run_takes_no_more_instructions_than_the_next() {
    let big = format!("(func $big (param $i i32) (block (br_table {}0 (local.get $i))))", "0 ".repeat(100_000));
    let calling = module(MANIFEST, &format!("(call $big (i32.const 0)) {}", tpl::ECHO), &big);
    let with = |fuel_base| Limits { fuel_base, fuel_per_byte: 0, ..Limits::default() };
    let warm = Plugin::load(&calling, Limits::default()).unwrap();
    assert_eq!(warm.run(INPUT.as_bytes(), b"{}"), Ok(INPUT.as_bytes().to_vec()));
    // The fewest instructions a run takes once the functions are compiled.
    let (mut lo, mut hi) = (0, 1 << 20);
    assert!(warm.run_limited(INPUT.as_bytes(), b"{}", &with(hi)).is_ok());
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if warm.run_limited(INPUT.as_bytes(), b"{}", &with(mid)).is_ok() { hi = mid } else { lo = mid }
    }
    let first = Plugin::load(&calling, Limits::default()).unwrap();
    assert_eq!(first.run_limited(INPUT.as_bytes(), b"{}", &with(hi)), Ok(INPUT.as_bytes().to_vec()), "a first run within {hi} instructions");
}

/// A filter that asks for a heavy budget gets 20 times the instructions and five times the time of
/// a light one, so it finishes work a light one can't: here 100 million steps of a loop. The
/// budget is its own: what it returns can't add to it, since the host learns that only at the end.
#[test]
fn a_heavy_filter_finishes_what_a_light_one_cannot() {
    let heavy = r#"{"id":"test.heavy","name":"Heavy","version":"1","kind":"filter","budget":"heavy"}"#;
    let (light, heavy) = (build(&tpl::spinning(MANIFEST, 100_000_000, INPUT)), build(&tpl::spinning(heavy, 100_000_000, INPUT)));
    assert_eq!(run_err(&light, Limits::default()).0, Error::Limit("instruction budget".into()));
    let p = Plugin::load(&heavy, Limits::default()).unwrap();
    assert_eq!((p.limits().fuel_base, p.limits().wall_time_ms), (HEAVY_FUEL, HEAVY_WALL_TIME_MS));
    assert_eq!(heavy_terms(), (5, 20));
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
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
    let e = export::run(&filter, b"{}", &Value::Null, target).unwrap_err();
    assert!(matches!(e, Error::Params(_)) && e.to_string().contains("is a plug-in of kind `filter`, not an exporter"), "{e}");
    let importer = Plugin::load(&build(&tpl::returning(&tpl::importer("test.reads", r#"["tst"]"#, "{}"), "{}")), Limits::default()).unwrap();
    let e = export::run(&importer, b"{}", &Value::Null, target).unwrap_err();
    assert!(e.to_string().contains("is a plug-in of kind `importer`, not an exporter"), "{e}");
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

/// A filter of plug-in API level 2 that may use the host imports.
const LEVEL2: &str =
    r#"{"id":"test.level2","name":"Level 2","version":"1","kind":"filter","api":2,"permissions":["doc.read","doc.write","progress"]}"#;

/// The error of loading `bytes`, as text.
fn load_err(bytes: &[u8]) -> String {
    Plugin::load(bytes, Limits::default()).expect_err("the module must be refused").to_string()
}

/// A module may import only the host's three functions, with the host's types, and only when its
/// manifest gives `api` 2 and lists `progress`; the error names the import it refuses.
#[test]
fn host_imports_are_checked_when_a_module_is_loaded() {
    let with = |manifest: &str, import: &str| build(&tpl::with_imports(manifest, import, "", INPUT));
    let unknown = with(LEVEL2, r#"(import "env" "vc_clock" (func (result i64)))"#);
    assert!(load_err(&unknown).contains("imports `env.vc_clock`, which the host doesn't offer"), "{}", load_err(&unknown));
    let elsewhere = with(LEVEL2, r#"(import "vc" "vc_log" (func (param i32 i32 i32) (result i32)))"#);
    assert!(load_err(&elsewhere).contains("imports `vc.vc_log`, which the host doesn't offer"), "{}", load_err(&elsewhere));
    let wrong_type = with(LEVEL2, r#"(import "env" "vc_progress" (func (param i32 i32)))"#);
    assert!(load_err(&wrong_type).contains("imports `env.vc_progress` with another type than the host's: it is (param i64 i64)"));
    let memory = with(LEVEL2, r#"(import "env" "vc_cancelled" (global i32))"#);
    assert!(load_err(&memory).contains("imports `env.vc_cancelled` with another type"), "{}", load_err(&memory));
    let progress = r#"(import "env" "vc_progress" (func (param i64 i64)))"#;
    let ungranted = r#"{"id":"test.ungranted","name":"U","kind":"filter","api":2,"permissions":["doc.read","doc.write"]}"#;
    let says = "imports `env.vc_progress`, which a plug-in may use only at plug-in API level 2 (`api` 2) and with the permission `progress`";
    for manifest in [ungranted, r#"{"id":"test.implied","name":"U","kind":"filter","api":2}"#, MANIFEST] {
        let e = load_err(&with(manifest, progress));
        assert!(e.contains(says), "{manifest}: {e}");
    }
    let p = Plugin::load(&with(LEVEL2, progress), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
    // A level-2 plug-in that imports nothing runs as a level-1 one does.
    let plain = Plugin::load(&build(&tpl::returning(LEVEL2, INPUT)), Limits::default()).unwrap();
    assert_eq!(plain.run_reporting(INPUT.as_bytes(), b"{}", plain.limits()), (Ok(INPUT.as_bytes().to_vec()), Report::default()));
}

/// Progress is clamped and passed on to the caller's sink as a share; the run's last report is kept.
#[test]
fn progress_reports_are_clamped_and_reach_the_caller() {
    use std::{cell::RefCell, rc::Rc};
    let first = "(call $vc_progress (i64.const 50) (i64.const 10)) (call $vc_progress (i64.const -3) (i64.const 8)) (call $vc_progress (i64.const 3) (i64.const 8))";
    let p = Plugin::load(&build(&tpl::reporting(LEVEL2, first, INPUT)), Limits::default()).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink: vectorcraft_doc::cancel::ProgressSink = {
        let seen = seen.clone();
        Rc::new(move |d, t, w| seen.borrow_mut().push((d, t, w.to_string())))
    };
    let token = vectorcraft_doc::cancel::CancelToken::new();
    let (out, report) = vectorcraft_doc::cancel::scope(&token, Some(sink), || p.run_reporting(INPUT.as_bytes(), b"{}", p.limits()));
    assert_eq!(out.unwrap(), INPUT.as_bytes());
    assert_eq!(report.progress, Some((3, 8)));
    let step = vectorcraft_plugins::host::PROGRESS_STEP.to_string();
    assert_eq!(*seen.borrow(), [(10, 10, step.clone()), (0, 8, step.clone()), (3, 8, step)]);
    // A live effect's progress stays its own: it is drawn as part of something else.
    let effect = r#"{"id":"test.level2.effect","name":"E","kind":"effect","api":2,"permissions":["doc.read","effect.render","progress"]}"#;
    let e = Plugin::load(&build(&tpl::reporting(effect, first, INPUT)), Limits::default()).unwrap();
    let quiet = Rc::new(RefCell::new(0));
    let sink: vectorcraft_doc::cancel::ProgressSink = {
        let quiet = quiet.clone();
        Rc::new(move |_, _, _| *quiet.borrow_mut() += 1)
    };
    let (_, report) = vectorcraft_doc::cancel::scope(&token, Some(sink), || e.run_reporting(INPUT.as_bytes(), b"{}", e.limits()));
    assert_eq!((report.progress, *quiet.borrow()), (Some((3, 8)), 0));
}

/// Cancel makes `vc_cancelled` return 1, and a plug-in that checks it stops at once; what it returns
/// then is discarded, so the run is `Cancelled`; past the time budget the same holds. One that
/// ignores it still ends at its instruction budget. Outside a cancelled scope it returns 0.
#[test]
fn cancel_reaches_a_cooperating_plugin_and_a_budget_ends_one_that_ignores_it() {
    let p =
        Plugin::load(&build(&tpl::cooperating(LEVEL2, 1 << 40, r#"{"stopped":1}"#, INPUT)), Limits { fuel_base: u64::MAX / 4, ..Limits::default() })
            .unwrap();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let (out, report) = vectorcraft_doc::cancel::scope(&token, None, || p.run_reporting(INPUT.as_bytes(), b"{}", p.limits()));
    assert_eq!(out, Err(Error::Cancelled), "what it returned after the Cancel is discarded");
    assert_eq!(report.progress, Some((0, 1 << 40)), "it saw the Cancel on its first step");
    // Not cancelled: it runs all its steps.
    let short = Plugin::load(&build(&tpl::cooperating(LEVEL2, 1000, r#"{"stopped":1}"#, INPUT)), Limits::default()).unwrap();
    let (out, report) = short.run_reporting(INPUT.as_bytes(), b"{}", short.limits());
    assert_eq!((out.unwrap(), report.progress), (INPUT.as_bytes().to_vec(), Some((999, 1000))));
    // Past its time budget `vc_cancelled` returns 1 too, and what it returns then is discarded.
    let late =
        Plugin::load(&build(&tpl::cooperating(LEVEL2, 1000, r#"{"stopped":1}"#, INPUT)), Limits { wall_time_ms: 0, ..Limits::default() }).unwrap();
    let (out, report) = late.run_reporting(INPUT.as_bytes(), b"{}", late.limits());
    assert_eq!((out, report.progress), (Err(Error::Limit("time budget".into())), Some((0, 1000))));
    // One that never checks ends at its budget, cancelled or not.
    let ignoring = build(&tpl::reporting(LEVEL2, "(loop $l (br $l))", INPUT));
    let p = Plugin::load(&ignoring, Limits { fuel_base: 1_000_000, ..Limits::default() }).unwrap();
    let r = vectorcraft_doc::cancel::scope(&token, None, || p.run(INPUT.as_bytes(), b"{}"));
    assert_eq!(r, Err(Error::Limit("instruction budget".into())));
}

/// A log line longer than the cap is cut; lines past the run's budget are dropped, with one line
/// that says so; text outside memory, text that isn't UTF-8 and an unknown level are errors the
/// plug-in sees, never a panic.
#[test]
fn log_lines_are_capped_and_bad_text_is_an_error_the_plugin_sees() {
    use vectorcraft_plugins::host::{self, LOG_BAD_LEVEL, LOG_NOT_UTF8, LOG_OUTSIDE_MEMORY, MAX_LOG_LINE, MAX_LOG_LINES};
    // `$text` at 0x2000 holds "hi", then 0xff, then 2000 bytes of "x".
    let expect = |level: i32, ptr: &str, len: &str, want: i32| {
        format!("(if (i32.ne (call $vc_log (i32.const {level}) {ptr} {len}) (i32.const {want})) (then unreachable))")
    };
    let first = [
        expect(2, "(i32.const 0x2000)", "(i32.const 2)", 0),
        expect(1, "(i32.const 0x2003)", "(i32.const 2000)", 0),
        expect(2, "(i32.const -1)", "(i32.const 5)", LOG_OUTSIDE_MEMORY),
        expect(2, "(i32.const 0x2000)", "(i32.const -1)", LOG_OUTSIDE_MEMORY),
        expect(2, "(i32.mul (memory.size) (i32.const 65536))", "(i32.const 1)", LOG_OUTSIDE_MEMORY),
        expect(2, "(i32.const 0x2000)", "(i32.const 3)", LOG_NOT_UTF8),
        expect(9, "(i32.const 0x2000)", "(i32.const 2)", LOG_BAD_LEVEL),
    ]
    .concat();
    let data = format!(r#"(data (i32.const 0x2000) "hi\ff{}")"#, "x".repeat(2000));
    let imports = format!("{} {data}", tpl::HOST_IMPORTS);
    let p = Plugin::load(&build(&tpl::with_imports(LEVEL2, &imports, &first, INPUT)), Limits::default()).unwrap();
    let (out, report) = p.run_reporting(INPUT.as_bytes(), b"{}", p.limits());
    assert_eq!(out.unwrap(), INPUT.as_bytes(), "every call returned what it should");
    assert_eq!(report.lines.len(), 2);
    assert_eq!(report.lines[0], vectorcraft_plugins::LogLine { level: vectorcraft_plugins::LogLevel::Info, text: "hi".into() });
    assert_eq!(report.lines[1].text, format!("{}…", "x".repeat(MAX_LOG_LINE - 2)), "cut at the cap, ellipsis included");
    // Past the budget: dropped, with one line that says so.
    let flood = format!(
        "(local $i i32) (loop $l (drop (call $vc_log (i32.const 2) (i32.const 0x2000) (i32.const 2))) (local.set $i (i32.add (local.get $i) (i32.const 1))) (br_if $l (i32.lt_u (local.get $i) (i32.const {}))))",
        MAX_LOG_LINES * 3
    );
    let p = Plugin::load(&build(&tpl::with_imports(LEVEL2, &imports, &flood, INPUT)), Limits::default()).unwrap();
    let (out, report) = p.run_reporting(INPUT.as_bytes(), b"{}", p.limits());
    assert!(out.is_ok() && report.truncated);
    assert_eq!(report.lines.len(), MAX_LOG_LINES + 1);
    assert_eq!(report.lines.iter().filter(|l| l.text == host::LOG_TRUNCATED).count(), 1);
}

/// What a module's start function reports through the host imports before it fails is kept in the
/// run's report. This start function logs, reports progress and traps only when the run was
/// cancelled, so the module loads.
#[test]
fn what_a_start_function_reports_before_it_fails_is_kept() {
    let start = r#"(data (i32.const 0x2000) "hi")
  (func $start (if (call $vc_cancelled) (then
    (drop (call $vc_log (i32.const 2) (i32.const 0x2000) (i32.const 2)))
    (call $vc_progress (i64.const 1) (i64.const 4))
    unreachable)))
  (start $start)"#;
    let imports = format!("{} {start}", tpl::HOST_IMPORTS);
    let p = Plugin::load(&build(&tpl::with_imports(LEVEL2, &imports, "", INPUT)), Limits::default()).unwrap();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let (out, report) = vectorcraft_doc::cancel::scope(&token, None, || p.run_reporting(INPUT.as_bytes(), b"{}", p.limits()));
    assert!(out.is_err(), "{out:?}");
    assert_eq!(report.lines, [vectorcraft_plugins::LogLine { level: vectorcraft_plugins::LogLevel::Info, text: "hi".into() }]);
    assert_eq!(report.progress, Some((1, 4)));
}

/// A heavy filter installs once the user agrees, which loading a folder doesn't ask: it is skipped
/// and reported.
#[test]
fn folder_loading_skips_heavy_filters() {
    let dir = std::env::temp_dir().join(format!("vectorcraft-plugins-heavy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = r#"{"id":"test.folder.heavy","name":"Heavy","kind":"filter","budget":"heavy"}"#;
    std::fs::write(dir.join("heavy.wasm"), build(&tpl::spinning(manifest, 1, INPUT))).unwrap();
    let r = registry::load_folder(&dir).unwrap();
    assert!(r.loaded.is_empty());
    assert!(r.failed.len() == 1 && r.failed[0].1.contains("heavy budget"), "{:?}", r.failed);
    assert!(registry::get("test.folder.heavy").is_none());
    std::fs::remove_dir_all(&dir).ok();
}
