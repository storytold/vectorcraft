//! The plug-in ABI end to end: the example Desaturate plug-in, and hostile modules (infinite
//! loops, memory bombs, out-of-bounds access, bad manifests and outputs…), which must all fail
//! with an error, within their instruction budget.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_plugins::manifest::{MANIFEST_SECTION, MAX_MANIFEST_BYTES};
use vectorcraft_plugins::storage::{Meter, Storage, StorageError};
use vectorcraft_plugins::{
    Error, FUEL_SLICE, HEAVY_FUEL, HEAVY_WALL_TIME_MS, Kind, LARGE_FUEL, Limits, Plugin, Report, heavy_terms, registry, wat as tpl,
};

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

/// Runs `bytes` on an empty input → the error.
fn run_err(bytes: &[u8], limits: Limits) -> Error {
    let r = Plugin::load(bytes, limits).and_then(|p| p.run(INPUT.as_bytes(), b"{}"));
    r.expect_err("hostile module must fail")
}

/// Two slices of instructions and nothing per input byte: a run that is not stopped between the
/// first and the second slice ends with the instruction budget.
fn two_slices() -> Limits {
    Limits { fuel_base: 2 * FUEL_SLICE, fuel_per_byte: 0, ..Limits::default() }
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

/// A Desaturate run changes colours only: a live shape with fractional coordinates keeps its path
/// exactly and stays live.
#[test]
fn example_desaturate_keeps_the_geometry_exactly() {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Document, LiveShape, Node, NodeKind};
    use vectorcraft_geom::{Affine, PathData, Point, SubPath};
    use vectorcraft_plugins::objects;
    let p = Plugin::load(DESATURATE, Limits::default()).unwrap();
    let mut d = Document::new(2000.0, 2000.0);
    let layer = d.layers[0].id;
    let points: Vec<Point> = (1..1000).map(|k| f64::from(k) * 1.234_567_890_123_456_7 / 7.0 + 0.1).map(|x| Point::new(x, x / 3.0)).collect();
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 1.0);
    let mut n = Node::path(d.alloc_id(), PathData::single(SubPath::polyline(&points, true)), red.clone());
    if let NodeKind::Path { live, .. } = &mut n.kind {
        *live = Some(Box::new(LiveShape::Rectangle {
            w: 10.0 / 3.0,
            h: 20.0 / 7.0,
            radii: [0.0; 4],
            radii_relative: false,
            kinds: Default::default(),
            xf: Affine::translate((0.1, 1.0 / 3.0)),
        }));
    }
    let id = d.insert(Some(layer), usize::MAX, n).unwrap();
    let before = d.node(id).unwrap().kind.clone();
    let out = p.run(&objects::encode_input(&d, &[id]), b"{}").unwrap();
    objects::apply_output(&mut d, &[id], objects::decode(&out).unwrap(), None, &|fx, _| Ok(fx.to_vec())).unwrap();
    let after = d.node(id).unwrap();
    assert!(after.kind == before, "the path or its live shape changed");
    assert!(after.appearance != red, "the fill was not desaturated");
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

/// Each hostile module fails with the error for what it did. The ones that never end stop at
/// their instruction budget, not at the time budget, so how long they run depends on the
/// instructions they are given and not on the machine's speed.
#[test]
fn hostile_modules_fail_within_their_budget() {
    let budget = "plug-in exceeded its instruction budget";
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("infinite loop", ok_module("(loop $l (br $l)) (i64.const 0)"), budget),
        (
            "memory.grow bomb, then trap",
            ok_module("(loop $l (br_if $l (i32.ne (memory.grow (i32.const 512)) (i32.const -1)))) unreachable"),
            "plug-in trapped: wasm `unreachable`",
        ),
        ("memory.grow forever", ok_module("(loop $l (drop (memory.grow (i32.const 1))) (br $l)) (i64.const 0)"), budget),
        ("out-of-bounds write", ok_module("(i32.store (i32.const -4) (i32.const 0)) (i64.const 0)"), "plug-in trapped: out of bounds memory access"),
        ("trap", ok_module("unreachable"), "plug-in trapped: wasm `unreachable`"),
        ("division by zero", ok_module("(drop (i32.div_u (i32.const 1) (i32.const 0))) (i64.const 0)"), "plug-in trapped: integer divide by zero"),
        ("deep recursion", module(MANIFEST, "(call $r) (i64.const 0)", "(func $r (call $r))"), "plug-in trapped: call stack exhausted"),
        ("error code", ok_module("(i64.const -7)"), "plug-in failed: vc_run returned error code -7"),
        ("output outside memory", ok_module("(i64.const 0x00000100fffffff0)"), "plug-in ABI: the output points outside memory"),
        ("huge output", ok_module("(i64.const 0x7fffffff00000000)"), "plug-in exceeded its output size"),
        (
            "wrong ABI version",
            build(&tpl::module(MANIFEST, "(i64.const 0)", "", 1).replace("(result i32) (i32.const 1))", "(result i32) (i32.const 2))")),
            "plug-in ABI: vc_abi_version returned 2",
        ),
        ("bad manifest", module("not json", "(i64.const 0)", ""), "plug-in manifest: "),
        ("manifest with a bad id", module(r#"{"id":"a b","name":"x","kind":"filter"}"#, "(i64.const 0)", ""), "plug-in manifest: invalid id \"a b\""),
        (
            "manifest of the wrong kind",
            module(r#"{"id":"a","name":"x","kind":"panel"}"#, "(i64.const 0)", ""),
            "plug-in manifest: manifest: unknown variant `panel`",
        ),
        ("start function loops", module(MANIFEST, "(i64.const 0)", "(func $spin (loop $l (br $l))) (start $spin)"), budget),
        (
            "WASI import",
            module(MANIFEST, "(i64.const 0)", r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#),
            "plug-in ABI: the module imports `wasi_snapshot_preview1.fd_write`",
        ),
    ];
    for (name, bytes, want) in cases {
        let e = run_err(&bytes, Limits::default()).to_string();
        assert!(e.starts_with(want), "{name}: {e}");
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
    let e = run_err(&bogus_alloc, Limits::default());
    assert!(matches!(e, Error::Abi(_)), "{e}");
}

/// The WebAssembly proposals `docs/plugins.md` lets a plug-in use, each with module fields that
/// use it.
const ACCEPTED_PROPOSALS: &[(&str, &str)] = &[
    ("mutable globals", r#"(global (export "counter") (mut i32) (i32.const 0))"#),
    ("sign extension", "(func (param i32) (result i32) (i32.extend8_s (local.get 0)))"),
    ("saturating conversions", "(func (param f32) (result i32) (i32.trunc_sat_f32_s (local.get 0)))"),
    ("multi-value", "(func (result i32 i32) (i32.const 1) (i32.const 2))"),
    ("bulk memory", "(func (memory.fill (i32.const 0) (i32.const 0) (i32.const 0)))"),
    ("reference types", "(func (result externref) (ref.null extern))"),
    ("tail calls", "(func $again (param i32) (return_call $again (local.get 0)))"),
    ("extended constant expressions", "(global i32 (i32.add (i32.const 1) (i32.const 2)))"),
];

/// Modules that use a WebAssembly proposal plug-ins may not use, each with the words its error
/// names it by.
const REFUSED_PROPOSALS: &[(&str, &str)] = &[
    ("(module (func (result v128) (v128.const i32x4 0 0 0 0)))", "SIMD"),
    ("(module (func (result v128) (i32x4.relaxed_trunc_f32x4_s (v128.const i32x4 0 0 0 0))))", "SIMD"),
    ("(module (memory i64 1))", "memory64"),
    ("(module (memory 1) (memory 1))", "multiple memories"),
    ("(module (memory 1 (pagesize 1)))", "custom page sizes"),
    (
        "(module (func (param i64 i64 i64 i64) (result i64 i64) (i64.add128 (local.get 0) (local.get 1) (local.get 2) (local.get 3))))",
        "wide arithmetic",
    ),
    ("(module (memory 1 1 shared))", "threads"),
    ("(module (tag))", "exceptions"),
    ("(module (func try catch_all end))", "legacy exceptions"),
    ("(module (type $t (func)) (func (param (ref $t))))", "function references"),
    ("(module (type (struct)))", "gc feature"),
];

#[test]
fn a_plugin_may_use_the_listed_webassembly_proposals_and_no_others() {
    for (name, fields) in ACCEPTED_PROPOSALS {
        let bytes = module(MANIFEST, tpl::ECHO, fields);
        let out = Plugin::load(&bytes, Limits::default()).and_then(|p| p.run(INPUT.as_bytes(), b"{}"));
        assert!(out.is_ok(), "{name}: {out:?}");
    }
    for (src, name) in REFUSED_PROPOSALS {
        match Plugin::load(&build(src), Limits::default()) {
            Err(Error::Module(m)) => assert!(m.to_lowercase().contains(&name.to_lowercase()), "{name}: the error doesn't name it: {m}"),
            Err(e) => panic!("{name}: refused with another kind of error: {e}"),
            Ok(_) => panic!("{name}: a module that uses it must be refused"),
        }
    }
}

/// A multi-value `br_table` whose targets all take the same values, with another operand under
/// those values, gives every target the values it was branched with: the run traps unless each
/// branch index returns both of them.
#[test]
fn a_multi_value_branch_table_gives_every_target_its_values() {
    let fields = "(func $pick (param $i i32) (result i64 i64)
        (block (result i64 i64) (i32.const 7) (i64.const 0x1111) (i64.const 0x2222) (br_table 0 1 0 (local.get $i))))
      (func $check (param $i i32) (local $a i64) (local $b i64)
        (call $pick (local.get $i)) (local.set $b) (local.set $a)
        (if (i64.ne (local.get $a) (i64.const 0x1111)) (then unreachable))
        (if (i64.ne (local.get $b) (i64.const 0x2222)) (then unreachable)))";
    let run = format!("(call $check (i32.const 0)) (call $check (i32.const 1)) (call $check (i32.const 2)) {}", tpl::ECHO);
    let p = Plugin::load(&module(MANIFEST, &run, fields), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
}

/// A function type with more than 32 parameters or results is refused before any function body is
/// validated: the module's extra function has an invalid body, and the error names the limit. A
/// type with 32 of each loads.
#[test]
fn a_function_type_past_32_values_is_refused_before_any_function_body_is_validated() {
    let i32s = |n: usize| vec!["i32"; n].join(" ");
    for (word, field) in [("parameters", "param"), ("results", "result")] {
        let fits = module(MANIFEST, tpl::ECHO, &format!("(type (func ({field} {})))", i32s(32)));
        assert!(Plugin::load(&fits, Limits::default()).is_ok(), "32 {word}");
        let past = module(MANIFEST, tpl::ECHO, &format!("(type (func ({field} {}))) (func (i32.add))", i32s(33)));
        match Plugin::load(&past, Limits::default()) {
            Err(Error::Module(m)) => assert_eq!(m, format!("a function type exceeds the limit of 32 {word}")),
            other => panic!("33 {word}: {:?}", other.err()),
        }
    }
}

#[test]
fn budgets_stop_runaway_plugins() {
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    let e = run_err(&spin, Limits { fuel_base: 1_000_000, fuel_per_byte: 10, ..Limits::default() });
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

/// The time budget stops a runaway plug-in where the first slice of its instruction budget ends:
/// with a time budget of 0 the deadline has passed on the monotonic clock before the run starts,
/// and an endless loop given two slices ends with the time budget. With the default time budget
/// the same loop runs through both slices and ends with the instruction budget.
#[test]
fn wall_clock_budget_stops_runaway_plugins() {
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    assert_eq!(run_err(&spin, Limits { wall_time_ms: 0, ..two_slices() }), Error::Limit("time budget".into()));
    assert_eq!(run_err(&spin, two_slices()), Error::Limit("instruction budget".into()));
}

/// Cancelling stops a run where one slice of its budget ends: an endless loop given two slices of
/// instructions comes back `Cancelled` at the end of the first, where outside a cancelled scope the
/// same module runs through both and ends with the instruction budget.
#[test]
fn cancelling_stops_a_run_between_two_slices_of_its_budget() {
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    let p = Plugin::load(&spin, two_slices()).unwrap();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || p.run(INPUT.as_bytes(), b"{}"));
    assert_eq!(r, Err(Error::Cancelled));
    // Outside a cancelled scope the same module still runs to its budget.
    assert_eq!(p.run(INPUT.as_bytes(), b"{}"), Err(Error::Limit("instruction budget".into())));
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
    assert_eq!(run_err(&light, Limits::default()), Error::Limit("instruction budget".into()));
    let p = Plugin::load(&heavy, Limits::default()).unwrap();
    assert_eq!((p.limits().fuel_base, p.limits().wall_time_ms), (HEAVY_FUEL, HEAVY_WALL_TIME_MS));
    assert_eq!(heavy_terms(), (5, 20));
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
}

/// The Large instruction budget gives a light filter 5 times the instructions besides those for
/// each byte of input, so it finishes 20 million steps of a loop that its own budget can't, in the
/// same time budget. A heavy filter keeps its larger budget, and every other limit stays.
#[test]
fn the_large_budget_finishes_what_a_light_one_cannot_and_keeps_a_heavy_one() {
    let light = Plugin::load(&build(&tpl::spinning(MANIFEST, 20_000_000, INPUT)), Limits::default()).unwrap();
    assert_eq!(light.run(INPUT.as_bytes(), b"{}"), Err(Error::Limit("instruction budget".into())));
    let large = light.large_limits();
    assert_eq!(large, Limits { fuel_base: LARGE_FUEL, ..Limits::default() });
    assert_eq!(LARGE_FUEL, 5 * Limits::default().fuel_base);
    assert_eq!(light.run_limited(INPUT.as_bytes(), b"{}", &large).unwrap(), INPUT.as_bytes());
    let heavy = r#"{"id":"test.heavy","name":"Heavy","version":"1","kind":"filter","budget":"heavy"}"#;
    let heavy = Plugin::load(&build(&tpl::spinning(heavy, 1, INPUT)), Limits::default()).unwrap();
    assert_eq!(heavy.large_limits(), *heavy.limits());
    assert_eq!((heavy.large_limits().fuel_base, heavy.large_limits().wall_time_ms), (HEAVY_FUEL, HEAVY_WALL_TIME_MS));
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

/// An importer runs on an import's budget, which replaces its own instruction and time budgets and
/// keeps its other limits: a run that needs more than `fuel_base` finishes within `export_fuel`,
/// and a run that needs more than `export_fuel` ends there, however large `fuel_base` is.
#[test]
fn an_importer_runs_on_an_import_s_budget() {
    use vectorcraft_plugins::export;
    use vectorcraft_plugins::import::{self, Source};
    let run = format!(
        "(local $n i32) (local.set $n (i32.const 100000)) \
         (loop $work (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $work (local.get $n))) {}",
        tpl::ECHO
    );
    let wasm = build(&tpl::module(&tpl::importer("test.import.budget", r#"["tst"]"#, "{}"), &run, "", 1));
    let file = br#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,0]},{"p":[1,0]}]}]}}]}"#;
    let source = Source { extension: "tst", name: "a" };

    let limits = import::limits(&Plugin::load(&wasm, Limits::default()).unwrap());
    assert_eq!((limits.fuel_base, limits.wall_time_ms), (export::FUEL, export::WALL_TIME_MS));
    let own = Limits {
        fuel_base: 9_000,
        fuel_per_byte: 0,
        wall_time_ms: 8_000,
        export_fuel: 10_000_000,
        export_wall_time_ms: 7_000,
        max_output_bytes: 6_000,
        ..Limits::default()
    };
    let p = Plugin::load(&wasm, own.clone()).unwrap();
    assert_eq!(import::limits(&p), Limits { fuel_base: 10_000_000, wall_time_ms: 7_000, ..own });
    assert_eq!(import::run(&p, file, &Value::Null, source).map(|i| i.objects.len()), Ok(1));

    let p = Plugin::load(&wasm, Limits { fuel_base: 10_000_000, fuel_per_byte: 0, export_fuel: 9_000, ..Limits::default() }).unwrap();
    assert_eq!(import::run(&p, file, &Value::Null, source), Err(Error::Limit("instruction budget".into())));
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

/// A module may import only the host's functions, with the host's types, and the progress ones only
/// when its manifest gives `api` 2 and lists `progress`; the error names the import it refuses.
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

/// A start function that traps: a load that instantiates the module fails with it.
const TRAPPING_START: &str = "(func $trap unreachable) (start $trap)";

/// [`MANIFEST`] with the ABI version, as a `vc_manifest` section must give it.
const SECTION: &str = r#"{"id":"test.plugin","name":"Test","version":"1","kind":"filter","abi":1}"#;

/// `module` with a `vc_manifest` custom section holding `data`.
fn with_section(module: &str, data: &[u8]) -> String {
    tpl::with_field(module, &tpl::custom_section(MANIFEST_SECTION, data)).expect("module text")
}

/// Asserts that each case's module is refused with an error that contains its text, and lists every
/// case that is not.
fn assert_refused(cases: &[(&str, String, &str)]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(name, src, says)| match Plugin::load(&build(src), Limits::default()) {
            Err(e) if e.to_string().contains(says) => None,
            other => Some(format!("{name}: {other:?}")),
        })
        .collect();
    assert!(wrong.is_empty(), "not refused as expected:\n{}", wrong.join("\n"));
}

/// A module that carries its manifest in a `vc_manifest` custom section loads without any of its
/// code running: not its start function, not `vc_abi_version` and not `vc_manifest`. Its runs
/// instantiate it as before.
#[test]
fn a_manifest_section_is_read_without_running_the_module() {
    let exports = r#"(func (export "vc_abi_version") (result i32) unreachable)
  (func (export "vc_manifest") (result i64) (loop $l (br $l)) (i64.const 0))"#;
    let p = Plugin::load(&build(&tpl::sectioned(SECTION, tpl::ECHO, &format!("{exports} {TRAPPING_START}"), 1)), Limits::default()).unwrap();
    assert_eq!(p.id(), "test.plugin");
    assert!(matches!(p.run(INPUT.as_bytes(), b"{}"), Err(Error::Trap(_))), "a run still runs the start function");
    let p = Plugin::load(&build(&tpl::sectioned(SECTION, tpl::ECHO, exports, 1)), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
}

/// A module with the section needs neither `vc_abi_version` nor `vc_manifest`, and when it also
/// has a `vc_manifest` export that says otherwise, the section is the manifest.
#[test]
fn the_manifest_section_needs_no_manifest_export_and_wins_over_one() {
    let p = Plugin::load(&build(&tpl::sectioned(SECTION, tpl::ECHO, "", 1)), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
    let exported = r#"{"id":"test.exported","name":"Exported","version":"1","kind":"effect"}"#;
    let p = Plugin::load(&build(&with_section(&tpl::module(exported, tpl::ECHO, "", 1), SECTION.as_bytes())), Limits::default()).unwrap();
    assert_eq!((p.id(), p.manifest().kind), ("test.plugin", Kind::Filter));
}

/// A test plug-in asked for no initial memory still gets one page, so its first `vc_alloc` block is
/// not at 0; a field added to a named module goes after the module's name.
#[test]
fn test_modules_with_no_pages_or_a_name_still_build() {
    for src in [tpl::sectioned(SECTION, tpl::ECHO, "", 0), tpl::module(MANIFEST, tpl::ECHO, "", 0)] {
        let p = Plugin::load(&build(&src), Limits::default()).unwrap();
        assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
    }
    let named = tpl::module(MANIFEST, tpl::ECHO, "", 1).replacen("(module", "(module $named", 1);
    let p = Plugin::load(&build(&with_section(&named, SECTION.as_bytes())), Limits::default()).unwrap();
    assert_eq!(p.id(), "test.plugin");
    assert_eq!(tpl::with_field("(module)", "(memory 1)").as_deref(), Some("(module\n  (memory 1))"));
}

/// The section's manifest gives the ABI version as `abi`, which the host checks in place of the
/// `vc_abi_version` export: left out or another version, the module is refused, whatever its
/// export returns.
#[test]
fn a_manifest_section_must_give_the_abi_version() {
    let says = "must give `abi` as 1";
    let version_1 = tpl::module(MANIFEST, tpl::ECHO, "", 1);
    let version_2 = version_1.replace("(result i32) (i32.const 1)", "(result i32) (i32.const 2)");
    assert!(load_err(&build(&version_2)).contains("vc_abi_version returned 2"));
    assert_refused(&[
        ("left out", with_section(&version_1, MANIFEST.as_bytes()), says),
        ("2", with_section(&version_1, SECTION.replace(r#""abi":1"#, r#""abi":2"#).as_bytes()), says),
        ("a string", with_section(&version_1, SECTION.replace(r#""abi":1"#, r#""abi":"1""#).as_bytes()), says),
        ("a fraction", with_section(&version_1, SECTION.replace(r#""abi":1"#, r#""abi":1.5"#).as_bytes()), says),
    ]);
    // The section's version is the one the host checks.
    let p = Plugin::load(&build(&with_section(&version_2, SECTION.as_bytes())), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
}

/// A `vc_manifest` section that is repeated, too long, not UTF-8, not JSON or not a valid manifest
/// is refused, even when the module's `vc_manifest` export returns a valid one.
#[test]
fn hostile_manifest_sections_are_refused() {
    let base = tpl::module(MANIFEST, tpl::ECHO, "", 1);
    let with = |data: &[u8]| with_section(&base, data);
    let long = format!(r#"{{"id":"test.long","name":"Long","kind":"filter","abi":1,"description":"{}"}}"#, " ".repeat(MAX_MANIFEST_BYTES));
    for (name, src, says) in [
        ("repeated", with_section(&with(SECTION.as_bytes()), SECTION.as_bytes()), "more than one"),
        ("too long", with(long.as_bytes()), "(limit "),
        ("not UTF-8", with(b"{\"id\":\"\xff\xfe\"}"), "not UTF-8"),
        ("not JSON", with(br#"{"id":"test.plugin","#), "manifest"),
        ("empty", with(b""), "manifest"),
        ("a bad id", with(br#"{"id":"a b","name":"x","kind":"filter","abi":1}"#), "invalid id"),
        ("an unknown kind", with(br#"{"id":"a","name":"x","kind":"panel","abi":1}"#), "manifest"),
    ] {
        match Plugin::load(&build(&src), Limits::default()) {
            Err(Error::Manifest(e)) => assert!(e.contains(says), "{name}: {e}"),
            other => panic!("{name}: {other:?}"),
        }
    }
}

/// A module with the section has its imports checked against its manifest, its exports by type,
/// and its initial memory and tables against the limits its runs have, before any of its code runs
/// (its start function would trap).
#[test]
fn a_sectioned_module_s_imports_and_exports_are_checked_before_it_runs() {
    let call = r#"(import "env" "vc_call" (func (param i32 i32 i32 i32) (result i32)))"#;
    let level2 = LEVEL2.replace(r#""api":2"#, r#""api":2,"abi":1"#);
    let e = load_err(&build(&tpl::sectioned(&level2, tpl::ECHO, &format!("{call} {TRAPPING_START}"), 1)));
    assert!(e.contains("imports `env.vc_call`") && e.contains("the permission `commands`"), "{e}");
    let sectioned = tpl::sectioned(SECTION, tpl::ECHO, TRAPPING_START, 1);
    let tables = |t: &str| tpl::sectioned(SECTION, tpl::ECHO, &format!("{t} {TRAPPING_START}"), 1);
    assert_refused(&[
        ("no memory", sectioned.replace(r#"(memory (export "memory") 1)"#, "(memory 1)"), "export its linear memory as `memory`"),
        (
            "initial memory past the cap",
            tpl::sectioned(SECTION, tpl::ECHO, TRAPPING_START, 60_000),
            "exceeded its memory budget (the module's initial memory",
        ),
        ("no vc_alloc", sectioned.replace(r#"(export "vc_alloc")"#, ""), "must export `vc_alloc` with the type (param i32) (result i32)"),
        (
            "vc_run of another type",
            tpl::sectioned(SECTION, "(i32.const 0)", TRAPPING_START, 1).replace("(param $pl i32) (result i64)", "(param $pl i32) (result i32)"),
            "must export `vc_run` with the type (param i32 i32 i32 i32) (result i64)",
        ),
        ("five tables", tables(&"(table 0 funcref) ".repeat(5)), "exceeded its table budget (the module defines 5 tables"),
        ("a table too large", tables("(table 1 funcref) (table 100001 200000 externref)"), "the largest with 100001 elements"),
    ]);
    // At the limits, it loads and runs.
    let at_limits = tpl::sectioned(SECTION, tpl::ECHO, "(table 100000 funcref) (table 0 funcref) (table 0 externref) (table 2 3 funcref)", 1);
    let p = Plugin::load(&build(&at_limits), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
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

/// Settings and a cache kept in memory, for the storage imports. Each call charges its meter the
/// bytes of the value it reads or writes.
#[derive(Default)]
struct Kept {
    settings: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    cache: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

/// The value `key` has in `kept`, once `meter` is charged its bytes.
fn charged(
    kept: &std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    key: &str,
    meter: &mut Meter,
) -> Result<Option<Vec<u8>>, StorageError> {
    let kept = kept.lock().unwrap();
    meter.take(kept.get(key).map_or(0, |v| v.len() as u64))?;
    Ok(kept.get(key).cloned())
}

impl Storage for Kept {
    fn setting(&self, key: &str, meter: &mut Meter) -> Result<Option<Vec<u8>>, StorageError> {
        charged(&self.settings, key, meter)
    }
    fn set_setting(&self, key: &str, value: &[u8], meter: &mut Meter) -> Result<(), StorageError> {
        meter.take(value.len() as u64)?;
        self.settings.lock().unwrap().insert(key.into(), value.to_vec());
        Ok(())
    }
    fn delete_setting(&self, key: &str, _: &mut Meter) -> Result<bool, StorageError> {
        Ok(self.settings.lock().unwrap().remove(key).is_some())
    }
    fn cache_entry(&self, name: &str, meter: &mut Meter) -> Result<Option<Vec<u8>>, StorageError> {
        charged(&self.cache, name, meter)
    }
    fn set_cache_entry(&self, name: &str, value: &[u8], meter: &mut Meter) -> Result<(), StorageError> {
        meter.take(value.len() as u64)?;
        self.cache.lock().unwrap().insert(name.into(), value.to_vec());
        Ok(())
    }
    fn remove_cache_entry(&self, name: &str) -> Result<bool, StorageError> {
        Ok(self.cache.lock().unwrap().remove(name).is_some())
    }
    fn clear_cache(&self) -> Result<u64, StorageError> {
        let mut cache = self.cache.lock().unwrap();
        let freed = cache.values().map(|v| v.len() as u64).sum();
        cache.clear();
        Ok(freed)
    }
    fn cache_bytes(&self) -> u64 {
        self.cache.lock().unwrap().values().map(|v| v.len() as u64).sum()
    }
}

/// A filter `id` of API level 2 that lists `settings` and `cache`.
fn storing(id: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Storing","version":"1","kind":"filter","api":2,"permissions":["settings","cache"]}}"#)
}

/// Instructions that trap unless `call` returns `want`.
fn expect(call: &str, want: i32) -> String {
    format!("(if (i32.ne {call} (i32.const {want})) (then unreachable))")
}

/// The module fields of a storage test: the storage imports, keys at 0x2000 ("zoom", "..", "a/b",
/// "CON", a tab, 65 "x"), the value "hello" at 0x2100; 0x3000 is free for an output buffer.
fn storage_data() -> String {
    format!(r#"{} (data (i32.const 0x2000) "zoom..a/bCON\09{}") (data (i32.const 0x2100) "hello")"#, tpl::STORAGE_IMPORTS, "x".repeat(65))
}

/// Each storage import needs its own permission: a module that imports one its manifest doesn't
/// list is refused, and the error names the import and the permission.
#[test]
fn storage_imports_need_their_own_permission() {
    let with = |manifest: &str, import: &str| build(&tpl::with_imports(manifest, import, "", INPUT));
    let get = r#"(import "env" "vc_settings_get" (func (param i32 i32 i32 i32) (result i32)))"#;
    let clear = r#"(import "env" "vc_cache_clear" (func (param i32 i32) (result i32)))"#;
    let e = load_err(&with(LEVEL2, get));
    assert!(
        e.contains("imports `env.vc_settings_get`, which a plug-in may use only at plug-in API level 2 (`api` 2) and with the permission `settings`"),
        "{e}"
    );
    let settings_only = r#"{"id":"test.settings","name":"S","kind":"filter","api":2,"permissions":["settings"]}"#;
    assert!(Plugin::load(&with(settings_only, get), Limits::default()).is_ok());
    let e = load_err(&with(settings_only, clear));
    assert!(
        e.contains("imports `env.vc_cache_clear`, which a plug-in may use only at plug-in API level 2 (`api` 2) and with the permission `cache`"),
        "{e}"
    );
    let wrong = load_err(&with(&storing("test.wrong"), r#"(import "env" "vc_cache_clear" (func (param i32) (result i32)))"#));
    assert!(wrong.contains("imports `env.vc_cache_clear` with another type than the host's: it is (param i32 i32) (result i32)"), "{wrong}");
    assert!(Plugin::load(&build(&tpl::with_imports(&storing("test.all"), tpl::STORAGE_IMPORTS, "", INPUT)), Limits::default()).is_ok());
}

/// A plug-in sets, reads and removes its settings and cache entries through the storage imports,
/// by key; a value too large for its buffer returns its length and copies nothing; a key or buffer
/// outside its memory, a key that isn't a plain name and a value over its cap are errors it sees.
#[test]
fn storage_imports_read_and_write_the_plugins_own_storage() {
    use vectorcraft_plugins::host::{STORAGE_BAD_NAME, STORAGE_DONE, STORAGE_FULL, STORAGE_NOT_FOUND, STORAGE_OUTSIDE_MEMORY};
    let id = "test.storage.own";
    let kept = std::sync::Arc::new(Kept::default());
    registry::set_storage(id, Some(kept.clone()));
    let zoom = "(i32.const 0x2000) (i32.const 4)";
    let first = [
        expect(&format!("(call $vc_settings_get {zoom} (i32.const 0x3000) (i32.const 16))"), STORAGE_NOT_FOUND),
        expect(&format!("(call $vc_settings_set {zoom} (i32.const 0x2100) (i32.const 5))"), STORAGE_DONE),
        // Too small a buffer: the length, and nothing copied.
        expect(&format!("(call $vc_settings_get {zoom} (i32.const 0x3000) (i32.const 4))"), 5),
        expect("(i32.load8_u (i32.const 0x3000))", 0),
        expect(&format!("(call $vc_settings_get {zoom} (i32.const 0x3000) (i32.const 16))"), 5),
        expect("(i32.load (i32.const 0x3000))", i32::from_le_bytes(*b"hell")),
        expect(&format!("(call $vc_cache_write {zoom} (i32.const 0x2100) (i32.const 0))"), STORAGE_DONE),
        expect(&format!("(call $vc_cache_read {zoom} (i32.const 0x3000) (i32.const 0))"), 0),
        expect(&format!("(call $vc_cache_write {zoom} (i32.const 0x2100) (i32.const 3))"), STORAGE_DONE),
        // Hostile keys: `..`, `a/b`, `CON`, a tab, 65 bytes, no bytes.
        expect("(call $vc_settings_set (i32.const 0x2004) (i32.const 2) (i32.const 0x2100) (i32.const 5))", STORAGE_BAD_NAME),
        expect("(call $vc_settings_set (i32.const 0x2006) (i32.const 3) (i32.const 0x2100) (i32.const 5))", STORAGE_BAD_NAME),
        expect("(call $vc_cache_write (i32.const 0x2009) (i32.const 3) (i32.const 0x2100) (i32.const 5))", STORAGE_BAD_NAME),
        expect("(call $vc_cache_read (i32.const 0x200c) (i32.const 1) (i32.const 0x3000) (i32.const 16))", STORAGE_BAD_NAME),
        expect("(call $vc_settings_get (i32.const 0x200d) (i32.const 65) (i32.const 0x3000) (i32.const 16))", STORAGE_BAD_NAME),
        expect("(call $vc_settings_delete (i32.const 0x2000) (i32.const 0))", STORAGE_BAD_NAME),
        // Outside memory: the key, the value, the buffer.
        expect("(call $vc_settings_get (i32.const -1) (i32.const 4) (i32.const 0x3000) (i32.const 16))", STORAGE_OUTSIDE_MEMORY),
        expect(&format!("(call $vc_settings_set {zoom} (i32.const 0x2100) (i32.const -1))"), STORAGE_OUTSIDE_MEMORY),
        expect(&format!("(call $vc_cache_read {zoom} (i32.mul (memory.size) (i32.const 65536)) (i32.const 1))"), STORAGE_OUTSIDE_MEMORY),
        // A setting over its cap changes nothing.
        expect(
            &format!("(call $vc_settings_set {zoom} (i32.const 0) (i32.const {}))", vectorcraft_plugins::storage::MAX_SETTING_BYTES + 1),
            STORAGE_FULL,
        ),
        expect(&format!("(call $vc_settings_get {zoom} (i32.const 0x3000) (i32.const 16))"), 5),
    ]
    .concat();
    let p = Plugin::load(&build(&tpl::with_imports(&storing(id), &storage_data(), &first, INPUT)), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes(), "every call returned what it should");
    assert_eq!(kept.settings.lock().unwrap().get("zoom"), Some(&b"hello".to_vec()));
    assert_eq!(kept.cache.lock().unwrap().get("zoom"), Some(&b"hel".to_vec()));
    // Removing: one setting, then a missing one, then the whole cache, then a missing entry.
    let removing = [
        expect("(call $vc_settings_delete (i32.const 0x2000) (i32.const 4))", STORAGE_DONE),
        expect("(call $vc_settings_delete (i32.const 0x2000) (i32.const 4))", STORAGE_NOT_FOUND),
        expect("(call $vc_cache_clear (i32.const 0) (i32.const 0))", STORAGE_DONE),
        expect("(call $vc_cache_clear (i32.const 0x2000) (i32.const 4))", STORAGE_NOT_FOUND),
    ]
    .concat();
    let p = Plugin::load(&build(&tpl::with_imports(&storing(id), &storage_data(), &removing, INPUT)), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes(), "every call returned what it should");
    assert!(kept.settings.lock().unwrap().is_empty() && kept.cache.lock().unwrap().is_empty());
    registry::set_storage(id, None);
}

/// A read counts the value's bytes toward the run's storage bytes even when its buffer is too small
/// and nothing is copied, so reading a large entry into no buffer again and again ends in
/// `STORAGE_BUDGET`.
#[test]
fn a_read_into_too_small_a_buffer_still_counts_its_bytes() {
    use vectorcraft_plugins::host::{MAX_STORAGE_BYTES, STORAGE_BUDGET};
    use vectorcraft_plugins::storage::MAX_CACHE_ENTRY;
    let id = "test.storage.budget";
    let kept = std::sync::Arc::new(Kept::default());
    kept.cache.lock().unwrap().insert("zoom".into(), vec![0; MAX_CACHE_ENTRY]);
    registry::set_storage(id, Some(kept.clone()));
    let read = "(call $vc_cache_read (i32.const 0x2000) (i32.const 4) (i32.const 0x3000) (i32.const 0))";
    let fits = MAX_STORAGE_BYTES / MAX_CACHE_ENTRY;
    let reading = [vec![expect(read, MAX_CACHE_ENTRY as i32); fits], vec![expect(read, STORAGE_BUDGET)]].concat().concat();
    let p = Plugin::load(&build(&tpl::with_imports(&storing(id), &storage_data(), &reading, INPUT)), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes(), "every call returned what it should");
    registry::set_storage(id, None);
}

/// Storage calls cost the run one instruction for every 64 bytes they read or write: reading a
/// 64 MiB cache entry, or writing 1 MiB to the cache 64 times, ends a run of 1 M instructions with
/// its instruction budget and finishes within 2 M. A call the run can't pay for reads and writes
/// nothing, a call pays from the budget not yet in the store, a call past the 1 GiB still returns
/// `STORAGE_BUDGET`, and a small read finishes on a small budget.
#[test]
fn storage_calls_pay_instructions_for_the_bytes_they_move() {
    use vectorcraft_plugins::host::{MAX_STORAGE_BYTES, STORAGE_BUDGET, STORAGE_DONE};
    use vectorcraft_plugins::storage::MAX_CACHE_ENTRY;
    let id = "test.storage.fuel";
    let kept = std::sync::Arc::new(Kept::default());
    kept.cache.lock().unwrap().insert("zoom".into(), vec![0; MAX_CACHE_ENTRY]);
    kept.settings.lock().unwrap().insert("zoom".into(), b"hello".to_vec());
    registry::set_storage(id, Some(kept.clone()));
    let run = |first: &str, fuel_base: u64| {
        let limits = Limits { fuel_base, fuel_per_byte: 0, ..Limits::default() };
        Plugin::load(&build(&tpl::with_imports(&storing(id), &storage_data(), first, INPUT)), limits).unwrap().run(INPUT.as_bytes(), b"{}")
    };
    let zoom = "(i32.const 0x2000) (i32.const 4)";
    let reading = expect(&format!("(call $vc_cache_read {zoom} (i32.const 0x3000) (i32.const 0))"), MAX_CACHE_ENTRY as i32);
    // 17 more pages of memory, then 1 MiB of them written to the cache 64 times.
    let write = expect(&format!("(call $vc_cache_write {zoom} (i32.const 0x20000) (i32.const 1048576))"), STORAGE_DONE);
    let writing = format!(
        "(local $n i32) (drop (memory.grow (i32.const 17))) (local.set $n (i32.const 64)) \
         (loop $work {write} (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $work (local.get $n)))"
    );
    let budget = Err(Error::Limit("instruction budget".into()));
    let done = Ok(INPUT.as_bytes().to_vec());
    let small = expect(&format!("(call $vc_settings_get {zoom} (i32.const 0x3000) (i32.const 16))"), 5);
    assert_eq!(run(&small, 10_000), done, "a small read on 10,000 instructions");
    // Growing the memory by 17 pages costs about 17,400 instructions and writing 1 MiB 16,384: a
    // write the run can't pay for writes nothing.
    let once = format!("(drop (memory.grow (i32.const 17))) {write}");
    assert_eq!(run(&once, 30_000), budget, "1 MiB written on 30,000 instructions");
    assert_eq!(kept.cache_bytes(), MAX_CACHE_ENTRY as u64, "the entry is unchanged");
    assert_eq!(run(&once, 60_000), done, "1 MiB written on 60,000 instructions");
    assert_eq!(kept.cache_bytes(), 1 << 20, "the entry is written");
    kept.cache.lock().unwrap().insert("zoom".into(), vec![0; MAX_CACHE_ENTRY]);
    assert_eq!(run(&reading, 1_000_000), budget, "a 64 MiB read on 1 M instructions");
    assert_eq!(run(&reading, 2_000_000), done, "a 64 MiB read on 2 M instructions");
    assert_eq!(run(&writing, 1_000_000), budget, "64 MiB written on 1 M instructions");
    assert_eq!(run(&writing, 2_000_000), done, "64 MiB written on 2 M instructions");
    kept.cache.lock().unwrap().insert("zoom".into(), vec![0; MAX_CACHE_ENTRY]);
    // About 19.7 M instructions of fills leave less of the first 20 M slice than a 64 MiB read
    // costs; the read pays from the rest of the budget.
    let burning = format!(
        "(local $n i32) (drop (memory.grow (i32.const 130))) (local.set $n (i32.const 149)) \
         (loop $burn (memory.fill (i32.const 0x20000) (i32.const 7) (i32.const 8388608)) \
         (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $burn (local.get $n))) {reading}"
    );
    assert_eq!(run(&burning, 21_000_000), done, "a read across two slices within the budget");
    assert_eq!(run(&burning, 20_500_000), budget, "a read across two slices over the budget");
    // What the read took from the rest of the budget is gone: six more 8 MiB fills (about 786,000
    // instructions) don't fit the 21 M.
    let more = format!(
        "{burning} (local.set $n (i32.const 6)) (loop $more (memory.fill (i32.const 0x20000) (i32.const 7) (i32.const 8388608)) \
         (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $more (local.get $n)))"
    );
    assert_eq!(run(&more, 21_000_000), budget, "fills after the read over the budget");
    // With 8 MiB of the run's 1 GiB of storage bytes left and instructions for less, a 64 MiB read
    // returns `STORAGE_BUDGET`, since the bytes alone refuse it, and the run goes on.
    let rest = MAX_STORAGE_BYTES - 15 * MAX_CACHE_ENTRY - (8 << 20);
    kept.cache.lock().unwrap().insert("x".into(), vec![0; rest]);
    let x = expect("(call $vc_cache_read (i32.const 0x200d) (i32.const 1) (i32.const 0x3000) (i32.const 0))", rest as i32);
    let over = expect(&format!("(call $vc_cache_read {zoom} (i32.const 0x3000) (i32.const 0))"), STORAGE_BUDGET);
    let near = [vec![reading.clone(); 15], vec![x, over]].concat().concat();
    assert_eq!(run(&near, 16_700_000), done, "a read past the 1 GiB");
    // Bytes are counted over the run: 9,000 writes of 63 bytes, each under 64, cost about 8,860
    // instructions more than 9,000 writes of none, not 0 or 9,000 as rounding each write down or up
    // would. Each run's fewest instructions are found by bisection.
    let looped = |len: u32| {
        let write = expect(&format!("(call $vc_cache_write {zoom} (i32.const 0x2000) (i32.const {len}))"), STORAGE_DONE);
        format!(
            "(local $n i32) (local.set $n (i32.const 9000)) \
             (loop $work {write} (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $work (local.get $n)))"
        )
    };
    let fewest = |first: &str| {
        let (mut lo, mut hi) = (10_000, 50_000_000);
        assert_eq!(run(first, hi), done, "the writes finish");
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            match run(first, mid) {
                Ok(_) => hi = mid,
                Err(Error::Limit(why)) if why == "instruction budget" => lo = mid,
                Err(e) => panic!("the writes failed on {mid} instructions: {e:?}"),
            }
        }
        hi
    };
    let (none, small) = (fewest(&looped(0)), fewest(&looped(63)));
    assert!((none + 8_800..=none + 8_900).contains(&small), "63-byte writes take {small} instructions, empty ones {none}");
    registry::set_storage(id, None);
}

/// A storage call that needs more of the budget than the store holds takes it only before Cancel
/// and the deadline: after either, a 32 MiB write near the end of the first 20 M slice writes
/// nothing and the run ends as canceled or with its time budget.
#[test]
fn after_cancel_or_the_deadline_a_storage_call_takes_no_more_of_the_budget() {
    use vectorcraft_plugins::host::STORAGE_DONE;
    use vectorcraft_plugins::storage::MAX_CACHE_ENTRY;
    let id = "test.storage.cancel";
    let kept = std::sync::Arc::new(Kept::default());
    kept.cache.lock().unwrap().insert("zoom".into(), vec![0; MAX_CACHE_ENTRY]);
    registry::set_storage(id, Some(kept.clone()));
    // Growing by 512 pages and 148 fills of 8 MiB take about 19.9 M instructions; the write costs
    // 524,288.
    let write = expect("(call $vc_cache_write (i32.const 0x2000) (i32.const 4) (i32.const 0x20000) (i32.const 33554432))", STORAGE_DONE);
    let first = format!(
        "(local $n i32) (drop (memory.grow (i32.const 512))) (local.set $n (i32.const 148)) \
         (loop $burn (memory.fill (i32.const 0x20000) (i32.const 7) (i32.const 8388608)) \
         (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $burn (local.get $n))) {write}"
    );
    let limits = Limits { fuel_base: 21_000_000, fuel_per_byte: 0, ..Limits::default() };
    let p = Plugin::load(&build(&tpl::with_imports(&storing(id), &storage_data(), &first, INPUT)), limits.clone()).unwrap();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || p.run(INPUT.as_bytes(), b"{}"));
    assert_eq!(r, Err(Error::Cancelled));
    assert_eq!(kept.cache_bytes(), MAX_CACHE_ENTRY as u64, "the entry is unchanged");
    let timed = Limits { wall_time_ms: 0, ..limits };
    assert_eq!(p.run_limited(INPUT.as_bytes(), b"{}", &timed), Err(Error::Limit("time budget".into())));
    assert_eq!(kept.cache_bytes(), MAX_CACHE_ENTRY as u64, "the entry is unchanged after the deadline");
    // With both, the time budget, as between two slices.
    let r = vectorcraft_doc::cancel::scope(&token, None, || p.run_limited(INPUT.as_bytes(), b"{}", &timed));
    assert_eq!(r, Err(Error::Limit("time budget".into())));
    assert_eq!(p.run(INPUT.as_bytes(), b"{}"), Ok(INPUT.as_bytes().to_vec()), "the same run without Cancel");
    assert_eq!(kept.cache_bytes(), 32 << 20, "the entry is written");
    registry::set_storage(id, None);
}

/// The host checks the deadline and Cancel after every storage call, also one the store's
/// instructions pay for: after either, a 1 MiB cache write is the run's last storage call, and the
/// write after it doesn't happen.
#[test]
fn a_run_past_its_deadline_or_canceled_stops_after_a_storage_call() {
    use vectorcraft_plugins::host::STORAGE_DONE;
    let id = "test.storage.after";
    let kept = std::sync::Arc::new(Kept::default());
    registry::set_storage(id, Some(kept.clone()));
    let first = format!(
        "(drop (memory.grow (i32.const 17))) {} {}",
        expect("(call $vc_cache_write (i32.const 0x2000) (i32.const 4) (i32.const 0x20000) (i32.const 1048576))", STORAGE_DONE),
        expect("(call $vc_cache_write (i32.const 0x200d) (i32.const 1) (i32.const 0x20000) (i32.const 1048576))", STORAGE_DONE),
    );
    let p = Plugin::load(&build(&tpl::with_imports(&storing(id), &storage_data(), &first, INPUT)), Limits::default()).unwrap();
    let written = || {
        let cache = kept.cache.lock().unwrap();
        let names = (cache.contains_key("zoom"), cache.contains_key("x"));
        drop(cache);
        kept.cache.lock().unwrap().clear();
        names
    };
    let timed = Limits { wall_time_ms: 0, ..p.limits().clone() };
    assert_eq!(p.run_limited(INPUT.as_bytes(), b"{}", &timed), Err(Error::Limit("time budget".into())));
    assert_eq!(written(), (true, false), "after the deadline");
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    assert_eq!(vectorcraft_doc::cancel::scope(&token, None, || p.run(INPUT.as_bytes(), b"{}")), Err(Error::Cancelled));
    assert_eq!(written(), (true, false), "after Cancel");
    assert_eq!(p.run(INPUT.as_bytes(), b"{}"), Ok(INPUT.as_bytes().to_vec()), "the same run before both");
    assert_eq!(written(), (true, true), "before both");
    registry::set_storage(id, None);
}

/// A store that answers every setting with the same value without counting its bytes, as the app's
/// store answers a get from the settings it keeps in memory.
struct Remembered(Vec<u8>);

impl Storage for Remembered {
    fn setting(&self, _: &str, _: &mut Meter) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(Some(self.0.clone()))
    }
    fn set_setting(&self, _: &str, _: &[u8], _: &mut Meter) -> Result<(), StorageError> {
        Ok(())
    }
    fn delete_setting(&self, _: &str, _: &mut Meter) -> Result<bool, StorageError> {
        Ok(false)
    }
    fn cache_entry(&self, _: &str, _: &mut Meter) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(None)
    }
    fn set_cache_entry(&self, _: &str, _: &[u8], _: &mut Meter) -> Result<(), StorageError> {
        Ok(())
    }
    fn remove_cache_entry(&self, _: &str) -> Result<bool, StorageError> {
        Ok(false)
    }
    fn clear_cache(&self) -> Result<u64, StorageError> {
        Ok(0)
    }
    fn cache_bytes(&self) -> u64 {
        0
    }
}

/// A settings get the store answers without counting bytes still costs the bytes it copies into
/// the module's memory: 2,000 gets of a 64 KiB value (128,000 KiB copied) end a run of 1 M
/// instructions with its instruction budget and finish within 4 M.
#[test]
fn settings_gets_answered_from_memory_pay_for_the_bytes_they_copy() {
    let id = "test.storage.copied";
    registry::set_storage(id, Some(std::sync::Arc::new(Remembered(vec![7; 65536]))));
    let get = expect("(call $vc_settings_get (i32.const 0x2000) (i32.const 4) (i32.const 0x20000) (i32.const 65536))", 65536);
    let first = format!(
        "(local $n i32) (drop (memory.grow (i32.const 2))) (local.set $n (i32.const 2000)) \
         (loop $work {get} (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $work (local.get $n)))"
    );
    let run = |fuel_base: u64| {
        let limits = Limits { fuel_base, fuel_per_byte: 0, ..Limits::default() };
        Plugin::load(&build(&tpl::with_imports(&storing(id), &storage_data(), &first, INPUT)), limits).unwrap().run(INPUT.as_bytes(), b"{}")
    };
    assert_eq!(run(1_000_000), Err(Error::Limit("instruction budget".into())), "on 1 M instructions");
    assert_eq!(run(4_000_000), Ok(INPUT.as_bytes().to_vec()), "on 4 M instructions");
    registry::set_storage(id, None);
}

/// A plug-in reaches only the storage of its own id, and a plug-in with none (not installed with a
/// plug-in store, as on the web) gets `STORAGE_UNAVAILABLE`.
#[test]
fn a_plugin_reaches_only_its_own_storage() {
    use vectorcraft_plugins::host::{STORAGE_DONE, STORAGE_NOT_FOUND, STORAGE_UNAVAILABLE};
    let (mine, theirs) = (std::sync::Arc::new(Kept::default()), std::sync::Arc::new(Kept::default()));
    registry::set_storage("test.storage.mine", Some(mine.clone()));
    registry::set_storage("test.storage.theirs", Some(theirs.clone()));
    theirs.settings.lock().unwrap().insert("zoom".into(), b"theirs".to_vec());
    theirs.cache.lock().unwrap().insert("zoom".into(), b"theirs".to_vec());
    let zoom = "(i32.const 0x2000) (i32.const 4)";
    let looking = [
        expect(&format!("(call $vc_settings_get {zoom} (i32.const 0x3000) (i32.const 16))"), STORAGE_NOT_FOUND),
        expect(&format!("(call $vc_cache_read {zoom} (i32.const 0x3000) (i32.const 16))"), STORAGE_NOT_FOUND),
        expect("(call $vc_cache_clear (i32.const 0) (i32.const 0))", STORAGE_DONE),
        expect(&format!("(call $vc_settings_set {zoom} (i32.const 0x2100) (i32.const 5))"), STORAGE_DONE),
    ]
    .concat();
    let p = Plugin::load(&build(&tpl::with_imports(&storing("test.storage.mine"), &storage_data(), &looking, INPUT)), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
    assert_eq!(theirs.settings.lock().unwrap().get("zoom"), Some(&b"theirs".to_vec()));
    assert_eq!(theirs.cache_bytes(), 6);
    assert_eq!(mine.settings.lock().unwrap().get("zoom"), Some(&b"hello".to_vec()));
    let nowhere = expect(&format!("(call $vc_settings_set {zoom} (i32.const 0x2100) (i32.const 5))"), STORAGE_UNAVAILABLE);
    let p = Plugin::load(&build(&tpl::with_imports(&storing("test.storage.nowhere"), &storage_data(), &nowhere, INPUT)), Limits::default()).unwrap();
    assert_eq!(p.run(INPUT.as_bytes(), b"{}").unwrap(), INPUT.as_bytes());
    registry::set_storage("test.storage.mine", None);
    registry::set_storage("test.storage.theirs", None);
}

/// The manifest of an event plug-in `id` that writes log lines and reacts to `events` (JSON list).
fn reacting_manifest(id: &str, events: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["progress","events"],"events":{events}}}"#)
}

/// A module that lists `events` needs a `vc_event` export of the right type; one that lists none
/// may leave it out.
#[test]
fn an_event_plugin_exports_vc_event() {
    let manifest = reacting_manifest("test.events.export", r#"["selection.changed"]"#);
    let e = Plugin::load(&build(&tpl::reporting(&manifest, "", INPUT)), Limits::default()).unwrap_err();
    assert!(e.to_string().contains("vc_event"), "{e}");
    let wrong = format!(r#"{} (func (export "vc_event") (param i32) (result i32) (i32.const 0))"#, tpl::HOST_IMPORTS);
    let e = Plugin::load(&build(&tpl::module(&manifest, tpl::ECHO, &wrong, 1)), Limits::default()).unwrap_err();
    assert!(e.to_string().contains("vc_event"), "{e}");
    assert!(Plugin::load(&build(&tpl::reacting(&manifest, tpl::HOST_IMPORTS, tpl::LOG_EVENT)), Limits::default()).is_ok());
    assert!(Plugin::load(&ok_module(tpl::ECHO), Limits::default()).is_ok(), "no events, no vc_event");
}

/// `vc_event` is given the event JSON in its own memory and returns 0; anything else, a trap and a
/// handler past its instruction budget fail it. A module without events runs no handler.
#[test]
fn vc_event_is_given_the_event_and_its_code_is_checked() {
    let manifest = reacting_manifest("test.events.run", r#"["layer.changed"]"#);
    let p = Plugin::load(&build(&tpl::reacting(&manifest, tpl::HOST_IMPORTS, tpl::LOG_EVENT)), Limits::default()).unwrap();
    let event = br#"{"type":"layer.changed","document":7}"#;
    let (r, report) = p.run_event(event, None);
    r.unwrap();
    assert_eq!(report.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), [std::str::from_utf8(event).unwrap()]);
    for (handler, says) in [
        ("(i32.const 3)", "vc_event returned 3"),
        ("unreachable", "trapped"),
        // The instruction budget, or the time budget on a loaded machine.
        ("(loop $l (br $l)) (i32.const 0)", "budget"),
    ] {
        let p = Plugin::load(&build(&tpl::reacting(&manifest, tpl::HOST_IMPORTS, handler)), Limits::default()).unwrap();
        let e = p.run_event(event, None).0.unwrap_err().to_string();
        assert!(e.contains(says), "{handler}: {e}");
    }
    let e = p.run_event(&vec![b' '; vectorcraft_plugins::events::MAX_EVENT_BYTES + 1], None).0.unwrap_err();
    assert!(e.to_string().contains("event size"), "{e}");
    let plain = Plugin::load(&ok_module(tpl::ECHO), Limits::default()).unwrap();
    assert!(plain.run_event(event, None).0.is_err());
    let limits = p.event_limits();
    assert_eq!((limits.fuel_base, limits.wall_time_ms), (vectorcraft_plugins::events::EVENT_FUEL, vectorcraft_plugins::events::EVENT_WALL_TIME_MS));
}

/// The start function, `vc_alloc` and `vc_event` share one event budget: work that fits it in the
/// handler alone fits it no longer when the start function has done as much first. The budget is
/// 200,000 instructions, which run in far less than the 200 ms an event is given, so the run that
/// fails is stopped by its instruction budget.
#[test]
fn a_handler_and_its_start_function_share_the_event_budget() {
    let manifest = reacting_manifest("test.events.shared", r#"["layer.changed"]"#);
    let burn = r#"(func $burn (param $k i32) (block $b (loop $l
      (br_if $b (i32.eqz (local.get $k)))
      (local.set $k (i32.sub (local.get $k) (i32.const 1)))
      (br $l))))"#;
    let plugin = |start: u32, handler: u32| {
        let imports = format!("{} {burn} (func $s (call $burn (i32.const {start}))) (start $s)", tpl::HOST_IMPORTS);
        let body = format!("(call $burn (i32.const {handler})) (i32.const 0)");
        Plugin::load(&build(&tpl::reacting(&manifest, &imports, &body)), Limits { fuel_base: 200_000, ..Limits::default() }).unwrap()
    };
    let event = br#"{"type":"layer.changed","document":1}"#;
    // The most loop turns, doubling, that fit the budget in the handler alone: twice as many run out
    // of instructions.
    let mut k = 1u32 << 8;
    let e = loop {
        match plugin(0, k * 2).run_event(event, None).0 {
            Ok(()) => k *= 2,
            Err(e) => break e.to_string(),
        }
        assert!(k < 1 << 30, "a handler that never runs out of its budget");
    };
    assert!(e.contains("instruction budget"), "{e}");
    assert!(plugin(0, k).run_event(event, None).0.is_ok());
    let e = plugin(k, k).run_event(event, None).0.unwrap_err().to_string();
    assert!(e.contains("instruction budget"), "{e}");
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
