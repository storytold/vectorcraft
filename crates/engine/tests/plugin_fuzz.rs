//! Untrusted plug-ins never crash the app: mutated and garbage modules handed to `plugin.install`,
//! and plug-ins returning hostile output (random JSON in the object model's shape: huge or
//! malformed coordinates, unknown ids, broken paints and gradients, junk of every type) as object
//! filters (`plugin.run`) or live effects (`effect.apply`), and plug-ins running bulk memory
//! instructions (`memory.fill`, `memory.copy`, `memory.init`, `data.drop`) on ranges in, at and past
//! the ends of memory and the data segment, under small and default budgets. Each run must end as
//! an error that leaves the document as it was, or as a document that then renders and exports,
//! without a panic. A bulk memory run traps exactly when a range leaves its region.
//!
//! `PROPTEST_CASES=2000 cargo test -p vectorcraft-engine --test plugin_fuzz` runs a deeper search.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use serde_json::{Map, Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;
use vectorcraft_plugins::{Error, Limits, Plugin};
use vectorcraft_testkit::catch_quiet;
use vectorcraft_testkit::fixtures::rich_session;

const DESATURATE: &[u8] = include_bytes!("../../plugins/tests/fixtures/desaturate.wasm");

/// 64 cases each, unless `PROPTEST_CASES` asks for more.
fn config() -> ProptestConfig {
    let mut c = ProptestConfig { failure_persistence: None, ..ProptestConfig::default() };
    if std::env::var_os("PROPTEST_CASES").is_none() {
        c.cases = 64;
    }
    c
}

/// Installs `wasm`, in place of any module installed with its id before (`replace: true`).
fn install(s: &mut Session, wasm: &[u8]) -> Result<Value, String> {
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "replace": true})).map_err(|e| e.to_string())
}

/// The document renders and exports without panicking.
fn renders(s: &Session, what: &str) -> Result<(), TestCaseError> {
    let d = &s.doc().unwrap().doc;
    let r = catch_quiet(|| {
        let mut r = vectorcraft_render::Renderer::new();
        if let Some(ab) = d.artboards.first() {
            let _ = r.render_region(d, ab.rect, 0.25, true).to_png();
        }
        let _ = vectorcraft_svg::export(d, &vectorcraft_svg::ExportOptions::default());
        let _ = vectorcraft_engine::export_pdf(d, &vectorcraft_pdf::PdfOptions::default());
        let _ = vectorcraft_format::save_file(d);
    });
    r.map_err(|msg| TestCaseError::fail(format!("{what}: panicked: {msg}")))
}

/// Junk of every JSON type.
fn arb_junk() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        (-1e3f64..1e3).prop_map(Value::from),
        "[a-zA-Z#]{0,6}".prop_map(Value::from),
    ];
    leaf.prop_recursive(2, 12, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Value::Array),
            prop::collection::btree_map("[a-z]{1,5}", inner, 0..3).prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

/// Numbers that tend to break arithmetic.
fn arb_num() -> impl Strategy<Value = Value> {
    prop_oneof![
        6 => (-600.0f64..600.0).prop_map(Value::from),
        1 => Just(json!(0)),
        1 => Just(json!(-0.0)),
        1 => Just(json!(1e308)),
        1 => Just(json!(-1e308)),
        1 => Just(json!(4.0e6)),
        1 => Just(json!(4.000001e6)),
        1 => Just(json!(1e-308)),
        1 => Just(json!(u64::MAX)),
        1 => Just(json!(i64::MIN)),
        1 => arb_junk(),
    ]
}

fn arb_point() -> impl Strategy<Value = Value> {
    prop_oneof![
        8 => (arb_num(), arb_num()).prop_map(|(x, y)| json!([x, y])),
        1 => (arb_num(), arb_num()).prop_map(|(x, y)| json!({"x": x, "y": y})),
        1 => arb_junk(),
    ]
}

fn arb_path() -> impl Strategy<Value = Value> {
    let anchor = (
        arb_point(),
        prop::option::of(arb_point()),
        prop::option::of(arb_point()),
        prop::option::of(prop_oneof![Just("Smooth"), Just("Corner"), Just("Weird")]),
    )
        .prop_map(|(p, i, o, k)| {
            let mut a = json!({"p": p});
            if let Some(i) = i {
                a["in"] = i;
            }
            if let Some(o) = o {
                a["out"] = o;
            }
            if let Some(k) = k {
                a["kind"] = json!(k);
            }
            a
        });
    let sub = (prop::collection::vec(anchor, 0..6), any::<bool>()).prop_map(|(anchors, closed)| json!({"anchors": anchors, "closed": closed}));
    prop_oneof![9 => prop::collection::vec(sub, 0..3).prop_map(|s| json!({"subpaths": s})), 1 => arb_junk()]
}

fn arb_color() -> impl Strategy<Value = Value> {
    prop_oneof![
        (arb_num(), arb_num(), arb_num()).prop_map(|(r, g, b)| json!({"model": "rgb", "r": r, "g": g, "b": b})),
        (arb_num(), arb_num(), arb_num(), arb_num()).prop_map(|(c, m, y, k)| json!({"model": "cmyk", "c": c, "m": m, "y": y, "k": k})),
        arb_num().prop_map(|k| json!({"model": "gray", "k": k})),
        (arb_num(), arb_num(), arb_num()).prop_map(|(l, a, b)| json!({"model": "lab", "l": l, "a": a, "b": b})),
        arb_junk(),
    ]
}

fn arb_paint() -> impl Strategy<Value = Value> {
    let stop = (arb_num(), arb_color(), arb_num(), arb_num()).prop_map(|(o, c, a, m)| json!({"offset": o, "color": c, "opacity": a, "midpoint": m}));
    let freeform = (
        prop::collection::vec((arb_point(), arb_color(), arb_num()).prop_map(|(at, c, s)| json!({"at": at, "color": c, "spread": s})), 0..3),
        arb_junk(),
    )
        .prop_map(|(points, lines)| json!({"points": points, "lines": if lines.is_array() { lines } else { json!([[0, 1, 9]]) }}));
    let gradient = (
        prop_oneof![Just("Linear"), Just("Radial"), Just("Freeform"), Just("Conic")],
        prop::collection::vec(stop, 0..4),
        prop::option::of((arb_point(), arb_point(), arb_num(), prop::option::of(arb_point()))),
        arb_num(),
        prop::option::of(freeform),
    )
        .prop_map(|(kind, stops, geom, angle, ff)| {
            let mut g = json!({"type": "gradient", "gradient": {"kind": kind, "stops": stops}, "angle": angle});
            if let Some((start, end, aspect, focal)) = geom {
                g["geom"] = json!({"start": start, "end": end, "aspect": aspect, "focal": focal});
            }
            if let Some(ff) = ff {
                g["freeform"] = ff;
            }
            g
        });
    prop_oneof![
        2 => Just(Value::Null),
        3 => (arb_color(), prop::option::of(prop_oneof![Just("White"), Just("Nope")]), arb_num())
            .prop_map(|(c, sw, t)| json!({"type": "solid", "color": c, "swatch": sw, "tint": t})),
        3 => gradient,
        1 => (prop_oneof![Just("Nope"), Just("")], prop::collection::vec(arb_num(), 6)).prop_map(|(p, xf)| json!({"type": "pattern", "pattern": p, "xf": xf})),
        1 => Just(json!({"type": "none"})),
        1 => arb_junk(),
    ]
}

/// Bytes in the passive data segment that bulk memory cases initialize memory from.
const SEGMENT_LEN: u32 = 4096;

/// Fuel the host hands a run per slice (`FUEL_SLICE` in `vectorcraft_plugins`' runtime).
const FUEL_SLICE: u64 = 20_000_000;

/// Where a bulk memory plug-in keeps its output (the manifest sits at 16).
const BULK_OUTPUT_AT: u32 = 0x8000;

/// The output a bulk memory plug-in returns.
const BULK_OUTPUT: &str = r#"{"objects":[]}"#;

/// The input and parameters a bulk memory plug-in is run on directly: each takes one page, so the
/// memory then ends two pages after the module's initial pages.
const BULK_INPUT: &[u8] = BULK_OUTPUT.as_bytes();
const BULK_PARAMS: &[u8] = b"{}";

/// The end of memory in WAT, read when the instruction runs.
const MEMORY_END: &str = "(i32.shl (memory.size) (i32.const 16))";

/// A bulk memory instruction a generated plug-in runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BulkOp {
    /// `memory.fill` over the destination range.
    Fill,
    /// `memory.copy` from the source range to the destination range, which may overlap.
    Copy,
    /// `memory.init` from the data segment, then `data.drop`.
    Init,
    /// `data.drop`, then `memory.init` from the emptied segment.
    Drop,
}

impl BulkOp {
    const ALL: [BulkOp; 4] = [BulkOp::Fill, BulkOp::Copy, BulkOp::Init, BulkOp::Drop];

    /// The instruction a module running this operation contains.
    fn opcode(self) -> &'static str {
        match self {
            BulkOp::Fill => "memory.fill",
            BulkOp::Copy => "memory.copy",
            BulkOp::Init => "memory.init",
            BulkOp::Drop => "data.drop",
        }
    }
}

/// What an offset is measured from: the start, the end, or the end less the length (a range that
/// ends exactly at the end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    Start,
    End,
    EndLessLen,
}

/// An offset into memory or the data segment: `base` plus `delta`, in the wrapping 32-bit
/// arithmetic the module computes it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Offset {
    base: Base,
    delta: i32,
}

impl Offset {
    const fn start(delta: i32) -> Self {
        Offset { base: Base::Start, delta }
    }

    /// The offset in a region that ends at `end`, for a range of `len` bytes.
    fn value(self, end: u32, len: u32) -> u32 {
        let base = match self.base {
            Base::Start => 0,
            Base::End => end,
            Base::EndLessLen => end.wrapping_sub(len),
        };
        base.wrapping_add_signed(self.delta)
    }

    /// The WAT computing the offset, with `end` the WAT of the region's end.
    fn wat(self, end: &str, len: u32) -> String {
        let base = match self.base {
            Base::Start => "(i32.const 0)".to_string(),
            Base::End => end.to_string(),
            Base::EndLessLen => format!("(i32.sub {end} (i32.const {len}))"),
        };
        format!("(i32.add {base} (i32.const {}))", self.delta)
    }
}

/// A plug-in whose run executes one bulk memory instruction.
#[derive(Clone, Copy, Debug)]
struct BulkCase {
    op: BulkOp,
    /// The destination in memory.
    dst: Offset,
    /// The source: in memory for `Copy`, in the data segment for `Init` and `Drop`.
    src: Offset,
    len: u32,
    /// The byte `Fill` writes.
    value: u8,
    /// How many times `Fill` and `Copy` run; `Init` and `Drop` run once.
    repeat: u32,
    /// Initial memory pages.
    pages: u32,
}

impl BulkCase {
    fn runs(self) -> u32 {
        match self.op {
            BulkOp::Fill | BulkOp::Copy => self.repeat.max(1),
            BulkOp::Init | BulkOp::Drop => 1,
        }
    }

    /// The memory end when the plug-in is run on [`BULK_INPUT`] and [`BULK_PARAMS`].
    fn end(self) -> u32 {
        (self.pages + 2) << 16
    }

    /// Whether every range the instruction touches lies inside memory ending at `end` and inside
    /// the data segment (empty once dropped). A range may end exactly at the end; an offset past
    /// the end traps even for zero bytes.
    fn in_bounds(self, end: u32) -> bool {
        let fits = |off: u32, end: u32| u64::from(off) + u64::from(self.len) <= u64::from(end);
        let dst = fits(self.dst.value(end, self.len), end);
        let segment = self.src.value(SEGMENT_LEN, self.len);
        match self.op {
            BulkOp::Fill => dst,
            BulkOp::Copy => dst && fits(self.src.value(end, self.len), end),
            BulkOp::Init => dst && fits(segment, SEGMENT_LEN),
            BulkOp::Drop => dst && fits(segment, 0),
        }
    }

    /// A generous estimate of the fuel the instructions use: a unit per 64 bytes moved, and a few
    /// per pass.
    fn fuel(self) -> u64 {
        u64::from(self.runs()) * (u64::from(self.len) / 64 + 16)
    }

    /// The plug-in module in WAT, with `manifest`.
    fn wat(self, manifest: &str) -> String {
        let (dst, len) = (self.dst.wat(MEMORY_END, self.len), self.len);
        let segment = self.src.wat(&format!("(i32.const {SEGMENT_LEN})"), len);
        let op = match self.op {
            BulkOp::Fill => format!("(memory.fill {dst} (i32.const {}) (i32.const {len}))", self.value),
            BulkOp::Copy => format!("(memory.copy {dst} {} (i32.const {len}))", self.src.wat(MEMORY_END, len)),
            BulkOp::Init => format!("(memory.init $seg {dst} {segment} (i32.const {len})) (data.drop $seg)"),
            BulkOp::Drop => format!("(data.drop $seg) (memory.init $seg {dst} {segment} (i32.const {len}))"),
        };
        let run = format!(
            "(local $n i32) (local.set $n (i32.const {runs}))
    (block $done (loop $again
      (br_if $done (i32.eqz (local.get $n)))
      (local.set $n (i32.sub (local.get $n) (i32.const 1)))
      {op}
      (br $again)))
    (i64.or (i64.shl (i64.const {out_len}) (i64.const 32)) (i64.const {BULK_OUTPUT_AT}))",
            runs = self.runs(),
            out_len = BULK_OUTPUT.len(),
        );
        let bytes: String = (0..SEGMENT_LEN).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        let extra = format!(r#"(data $seg "{}") (data (i32.const {BULK_OUTPUT_AT}) "{}")"#, bytes, tpl::string(BULK_OUTPUT));
        tpl::module(manifest, &run, &extra, self.pages)
    }
}

/// Lengths: zero, a byte, around the 64 bytes a unit of fuel pays for, the data segment's length,
/// a page, a mebibyte, or anything up to four pages.
fn arb_bulk_len() -> impl Strategy<Value = u32> {
    prop_oneof![
        3 => prop::sample::select(vec![0u32, 1, 63, 64, 65, SEGMENT_LEN - 1, SEGMENT_LEN, SEGMENT_LEN + 1, 0xffff, 0x1_0000, 0x1_0001, 1 << 20]),
        1 => 0u32..0x4_0000,
    ]
}

/// Offsets: near the start, ending at, before or past the end, at the end, or one past it.
fn arb_bulk_offset() -> impl Strategy<Value = Offset> {
    prop_oneof![
        3 => prop::sample::select(vec![0i32, 1, 100, BULK_OUTPUT_AT as i32]).prop_map(Offset::start),
        3 => (-2i32..=2).prop_map(|delta| Offset { base: Base::EndLessLen, delta }),
        2 => (-1i32..=1).prop_map(|delta| Offset { base: Base::End, delta }),
        1 => any::<i32>().prop_map(Offset::start),
    ]
}

/// How many times `Fill` and `Copy` run.
#[derive(Clone, Copy, Debug)]
enum Repeat {
    Times(u32),
    /// As many passes as a slice of fuel pays for the bytes, plus this (at most 2,000 passes).
    AroundSlice(i64),
}

/// Bulk memory cases in initial memories of 1, 2 or 17 pages.
fn arb_bulk_case() -> impl Strategy<Value = BulkCase> {
    (
        prop::sample::select(BulkOp::ALL.to_vec()),
        arb_bulk_offset(),
        arb_bulk_offset(),
        arb_bulk_len(),
        any::<u8>(),
        prop_oneof![8 => Just(Repeat::Times(1)), 2 => (2u32..=4).prop_map(Repeat::Times), 1 => (-1i64..=1).prop_map(Repeat::AroundSlice)],
        prop::sample::select(vec![1u32, 2, 17]),
    )
        .prop_map(|(op, dst, src, len, value, repeat, pages)| {
            let repeat = match repeat {
                Repeat::Times(n) => n,
                Repeat::AroundSlice(k) => {
                    let slice = (FUEL_SLICE * 64 / u64::from(len.max(1))) as i64;
                    (slice + k).clamp(1, 2_000) as u32
                }
            };
            BulkCase { op, dst, src, len, value, repeat, pages }
        })
}

/// The fuel a direct run gets: the default budget, or a budget of its own for each call with none
/// added per input byte: a small fixed one, or [`BulkCase::fuel`] plus or minus a little.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fuel {
    Default,
    Fixed(u64),
    AroundCost(i64),
}

fn arb_fuel() -> impl Strategy<Value = Fuel> {
    prop_oneof![
        4 => Just(Fuel::Default),
        1 => prop::sample::select(vec![1u64, 100, 1_000, 10_000]).prop_map(Fuel::Fixed),
        2 => (-64i64..64).prop_map(Fuel::AroundCost),
    ]
}

/// Runs `case` directly under `fuel`, without the engine.
fn run_bulk(case: BulkCase, fuel: Fuel) -> Result<Vec<u8>, Error> {
    let wasm = wat::parse_str(case.wat(&tpl::manifest("fuzz.bulk", "filter", "{}"))).unwrap();
    let plugin = Plugin::load(&wasm, Limits::default()).unwrap();
    let fuel_base = match fuel {
        Fuel::Default => return plugin.run(BULK_INPUT, BULK_PARAMS),
        Fuel::Fixed(f) => f,
        Fuel::AroundCost(k) => case.fuel().saturating_add_signed(k).max(1),
    };
    plugin.run_limited(BULK_INPUT, BULK_PARAMS, &Limits { fuel_base, fuel_per_byte: 0, ..Limits::default() })
}

#[test]
fn bulk_memory_cases_reach_their_instruction() {
    for op in BulkOp::ALL {
        let base = BulkCase { op, dst: Offset::start(0x1_0000), src: Offset::start(0), len: 1, value: 7, repeat: 1, pages: 2 };
        let (inside, outside) = match op {
            BulkOp::Fill | BulkOp::Copy => (base, BulkCase { dst: Offset { base: Base::End, delta: 0 }, ..base }),
            BulkOp::Init => (base, BulkCase { src: Offset { base: Base::End, delta: 0 }, ..base }),
            BulkOp::Drop => (BulkCase { len: 0, ..base }, base),
        };
        let src = inside.wat(&tpl::manifest("fuzz.bulk", "filter", "{}"));
        assert!(src.contains(op.opcode()), "{op:?} emits no {}", op.opcode());
        wat::parse_str(&src).unwrap();
        assert!(inside.in_bounds(inside.end()) && !outside.in_bounds(outside.end()), "{op:?}");
        assert!(run_bulk(inside, Fuel::Default).is_ok(), "{op:?} failed in bounds");
        let r = run_bulk(outside, Fuel::Default);
        assert!(matches!(r, Err(Error::Trap(_))), "{op:?} out of bounds gave {r:?}");
    }
}

/// An output object: an input object's id (by index into the inputs) or a random one, and any of
/// the object keys.
fn arb_object() -> impl Strategy<Value = (Option<usize>, Value)> {
    let stroke = (arb_paint(), prop::option::of(arb_num())).prop_map(|(p, w)| json!({"paint": p, "width": w}));
    (
        prop::option::of(0usize..8),
        prop::option::of(prop_oneof![Just(json!("path")), Just(json!("compound")), arb_junk()]),
        prop::option::of(arb_path()),
        prop::option::of(prop_oneof![Just(json!("evenodd")), Just(json!("nonzero")), arb_junk()]),
        prop::option::of(prop::collection::vec(arb_paint(), 0..3)),
        prop::option::of(prop::collection::vec(stroke, 0..3)),
        prop::option::of(arb_num()),
        prop::option::of(prop_oneof![Just(json!("name")), arb_junk()]),
    )
        .prop_map(|(id, t, path, rule, fills, strokes, opacity, name)| {
            let mut o = Map::new();
            for (k, v) in [
                ("type", t),
                ("path", path),
                ("fillRule", rule),
                ("fills", fills.map(Value::from)),
                ("strokes", strokes.map(Value::from)),
                ("opacity", opacity),
                ("name", name),
            ] {
                if let Some(v) = v {
                    o.insert(k.into(), v);
                }
            }
            (id, Value::Object(o))
        })
}

/// The output document: objects (ids resolved against `inputs`; out-of-range indices become
/// unknown ids), or junk.
fn output(objects: &[(Option<usize>, Value)], inputs: &[u64], junk: Option<Value>) -> String {
    if let Some(j) = junk {
        return j.to_string();
    }
    let list: Vec<Value> = objects
        .iter()
        .map(|(id, o)| {
            let mut o = o.clone();
            if let Some(i) = id {
                o["id"] = json!(inputs.get(*i).copied().unwrap_or(9_999_999 + *i as u64));
            }
            o
        })
        .collect();
    json!({ "objects": list }).to_string()
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn hostile_filter_and_effect_output_never_crashes(
        objects in prop::collection::vec(arb_object(), 0..5),
        junk in prop::option::weighted(0.1, arb_junk()),
        effect in any::<bool>(),
    ) {
        let mut s = rich_session();
        s.execute("select.all", &json!({})).unwrap();
        let st = s.doc().unwrap();
        let inputs: Vec<u64> = vectorcraft_plugins::objects::filter_targets(&st.doc, &st.selection.objects).iter().map(|i| i.0).collect();
        let out = output(&objects, &inputs, junk);
        let id = if effect { "fuzz.effect" } else { "fuzz.filter" };
        let src = tpl::returning(&tpl::manifest(id, if effect { "effect" } else { "filter" }, "{}"), &out);
        install(&mut s, &wat::parse_str(src).unwrap()).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let r = catch_quiet(|| {
            if effect {
                s.execute("effect.apply", &json!({"effect": format!("plugin.{id}")})).map(|_| ())
            } else {
                s.execute("plugin.run", &json!({"id": id})).map(|_| ())
            }
        });
        match r {
            Err(msg) => return Err(TestCaseError::fail(format!("panicked on {out}: {msg}"))),
            Ok(Err(_)) => prop_assert!(s.doc().unwrap().doc == before, "a failed run changed the document: {}", out),
            Ok(Ok(())) => {}
        }
        renders(&s, &out)?;
    }

    #[test]
    fn bulk_memory_plugins_never_crash(case in arb_bulk_case(), fuel in arb_fuel()) {
        let r = catch_quiet(|| run_bulk(case, fuel)).map_err(|msg| TestCaseError::fail(format!("panicked on {case:?}: {msg}")))?;
        let in_bounds = case.in_bounds(case.end());
        let ample = fuel == Fuel::Default && case.fuel() <= FUEL_SLICE / 4;
        match &r {
            Ok(_) => prop_assert!(in_bounds, "{:?} ran out of bounds without a trap", case),
            Err(Error::Trap(e)) => prop_assert!(!in_bounds && !e.contains("panicked"), "{:?} trapped: {}", case, e),
            Err(Error::Limit(e)) => prop_assert!(fuel != Fuel::Default || (in_bounds && !ample), "{:?} hit a limit: {}", case, e),
            Err(e) => return Err(TestCaseError::fail(format!("{case:?} failed: {e}"))),
        }
        if ample {
            prop_assert!(r.is_ok() == in_bounds, "{:?} gave {:?}", case, r.err());
        }

        let mut s = rich_session();
        s.execute("select.all", &json!({})).unwrap();
        install(&mut s, &wat::parse_str(case.wat(&tpl::manifest("fuzz.bulk", "filter", "{}"))).unwrap()).unwrap();
        let before = s.doc().unwrap().doc.clone();
        match catch_quiet(|| s.execute("plugin.run", &json!({"id": "fuzz.bulk"})).map(|_| ())) {
            Err(msg) => return Err(TestCaseError::fail(format!("panicked on {case:?}: {msg}"))),
            Ok(Err(_)) => prop_assert!(s.doc().unwrap().doc == before, "a failed run changed the document: {:?}", case),
            Ok(Ok(())) => {}
        }
        renders(&s, &format!("{case:?}"))?;
    }

    #[test]
    fn mutated_and_garbage_modules_never_crash(
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..8),
        cut in prop::option::of(any::<prop::sample::Index>()),
        garbage in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let mut bytes = DESATURATE.to_vec();
        for (i, b) in &flips {
            let i = i.index(bytes.len());
            bytes[i] = *b;
        }
        if let Some(c) = cut {
            bytes.truncate(c.index(bytes.len()));
        }
        let mut s = rich_session();
        s.execute("select.all", &json!({})).unwrap();
        for module in [bytes, [b"\0asm\x01\0\0\0".as_slice(), &garbage].concat(), garbage.clone()] {
            let r = catch_quiet(|| {
                if let Ok(info) = install(&mut s, &module) {
                    let _ = s.execute("plugin.run", &json!({"id": info["id"]}));
                }
            });
            prop_assert!(r.is_ok(), "panicked: {:?}", r.err());
        }
        renders(&s, "after mutated modules")?;
    }

    /// Manifests with hostile `permissions`, `api` and names (unknown, reserved and repeated
    /// permissions, junk of every type, control and bidirectional override characters) parse to
    /// an error or to a manifest whose permissions fit its kind, and installing one with a junk
    /// `grant` ends as an error or an install, never a panic. A prompt never shows a hidden
    /// character from the name.
    #[test]
    fn hostile_manifest_permissions_never_crash(
        kind in prop::sample::select(vec!["filter", "effect", "exporter"]),
        names in prop::collection::vec(
            prop_oneof![
                prop::sample::select(vec!["doc.read", "doc.write", "state", "effect.render", "export.write", "progress", "ui.panel", ""]),
                Just("doc.read\u{202E}"),
                Just("\u{0007}"),
            ],
            0..6,
        ),
        permissions in prop::option::weighted(0.8, prop_oneof![Just(Value::Null), arb_junk()]),
        api in prop_oneof![2 => Just(json!(1)), 1 => Just(json!(2)), 1 => arb_junk()],
        name in prop_oneof![3 => "[a-z\u{202E}\u{2066}\u{200F} ]{1,64}", 1 => "[a-z\u{202E}\u{2066}\u{0007}\u{200F} ]{1,80}"],
        grant in prop_oneof![Just(Value::Null), arb_junk(), Just(json!(["doc.read", "doc.write", "state", "effect.render", "export.write", "progress"]))],
    ) {
        let mut m = json!({"id": "fuzz.permissions", "name": name, "kind": kind, "api": api});
        if kind == "exporter" {
            m["extensions"] = json!(["fz"]);
        }
        m["permissions"] = match permissions {
            Some(Value::Null) | None => json!(names),
            Some(junk) => junk,
        };
        let text = m.to_string();
        let parsed = catch_quiet(|| vectorcraft_plugins::Manifest::parse(text.as_bytes()));
        let parsed = parsed.map_err(|msg| TestCaseError::fail(format!("panicked on {text}: {msg}")))?;
        if let Ok(man) = &parsed {
            prop_assert!(man.permissions().iter().all(|p| p.applies(man.kind)), "{}", text);
            let shown = vectorcraft_plugins::permissions::display_text(&man.name, vectorcraft_plugins::permissions::MAX_SHOWN_NAME);
            prop_assert!(!shown.chars().any(|c| c.is_control() || matches!(c, '\u{202E}' | '\u{2066}' | '\u{200F}')), "{}", shown);
            prop_assert!(shown.chars().count() <= vectorcraft_plugins::permissions::MAX_SHOWN_NAME);
        }
        let mut s = rich_session();
        let module = wat::parse_str(tpl::returning(&text, r#"{"objects":[]}"#)).unwrap();
        let data = vectorcraft_format::base64_encode(&module);
        let r = catch_quiet(|| {
            let _ = s.execute("plugin.inspect", &json!({"dataBase64": data}));
            s.execute("plugin.install", &json!({"dataBase64": data, "grant": grant})).is_ok()
        });
        let installed = r.map_err(|msg| TestCaseError::fail(format!("panicked installing {text}: {msg}")))?;
        prop_assert!(!installed || parsed.is_ok(), "a manifest that doesn't parse installed: {}", text);
        vectorcraft_plugins::registry::remove("fuzz.permissions");
        vectorcraft_plugins::registry::revoke("fuzz.permissions");
    }

    /// A plug-in of API level 2 that calls the host imports with hostile arguments (any progress
    /// counts, any log level, pointers and lengths in and out of its memory, text that isn't
    /// UTF-8) runs to an error or an output, never a panic, and its report stays within the caps.
    #[test]
    fn hostile_host_import_arguments_never_crash(
        calls in prop::collection::vec(
            prop_oneof![
                (any::<i64>(), any::<i64>()).prop_map(|(d, t)| format!("(call $vc_progress (i64.const {d}) (i64.const {t}))")),
                Just("(drop (call $vc_cancelled))".to_string()),
                (prop_oneof![any::<i32>(), -1..5i32], prop_oneof![any::<i32>(), 0x1ff00..0x20100i32], prop_oneof![any::<i32>(), 0..5000i32])
                    .prop_map(|(l, p, n)| format!("(drop (call $vc_log (i32.const {l}) (i32.const {p}) (i32.const {n})))")),
            ],
            0..24,
        ),
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        effect in any::<bool>(),
    ) {
        use vectorcraft_plugins::host::{LOG_TRUNCATED, MAX_LOG_BYTES, MAX_LOG_LINE, MAX_LOG_LINES};
        let manifest = if effect {
            r#"{"id":"fuzz.host","name":"F","kind":"effect","api":2,"permissions":["doc.read","effect.render","progress"]}"#
        } else {
            r#"{"id":"fuzz.host","name":"F","kind":"filter","api":2,"permissions":["doc.read","doc.write","progress"]}"#
        };
        let text: String = bytes.iter().map(|b| format!("\\{b:02x}")).collect();
        let imports = format!(r#"{} (data (i32.const 0x2000) "{text}")"#, tpl::HOST_IMPORTS);
        let wasm = wat::parse_str(tpl::with_imports(manifest, &imports, &calls.concat(), r#"{"objects":[]}"#)).unwrap();
        let r = catch_quiet(|| {
            let p = vectorcraft_plugins::Plugin::load(&wasm, vectorcraft_plugins::Limits::default())?;
            let token = vectorcraft_doc::cancel::CancelToken::new();
            let sink: vectorcraft_doc::cancel::ProgressSink = std::rc::Rc::new(|_, _, _| {});
            Ok::<_, vectorcraft_plugins::Error>(vectorcraft_doc::cancel::scope(&token, Some(sink), || p.run_reporting(b"{\"objects\":[]}", b"{}", p.limits())))
        });
        let r = r.map_err(|msg| TestCaseError::fail(format!("panicked on {calls:?}: {msg}")))?;
        let (out, report) = r.map_err(|e| TestCaseError::fail(format!("refused at load: {e}")))?;
        prop_assert!(out.is_ok(), "{:?}", out);
        prop_assert!(report.lines.len() <= MAX_LOG_LINES + 1);
        prop_assert!(report.lines.iter().all(|l| l.text.chars().count() <= MAX_LOG_LINE));
        prop_assert!(report.lines.iter().filter(|l| l.text != LOG_TRUNCATED).map(|l| l.text.len()).sum::<usize>() <= MAX_LOG_BYTES);
        if let Some((done, total)) = report.progress {
            prop_assert!(total == 0 || done <= total);
        }
    }

    /// Manifests with hostile `inputs` (unknown, repeated and oddly cased kinds, long lists, junk of
    /// every type) with any `api`, kind and permissions parse to an error or to a filter at `api` 2
    /// or later asking for each kind once, whose `permissions` list `doc.read` so that installing
    /// it asks for approval. One that installs and echoes its input on a document
    /// with type, images and symbols leaves the document as it was, and it still renders.
    #[test]
    fn hostile_manifest_inputs_never_crash(
        kind in prop::sample::select(vec!["filter", "effect", "exporter"]),
        names in prop::collection::vec(prop::sample::select(vec!["type", "image", "symbolInstance", "Type", "text", "path", ""]), 0..6),
        long in prop::option::weighted(0.1, 4usize..300),
        inputs in prop::option::weighted(0.8, prop_oneof![Just(Value::Null), arb_junk()]),
        api in prop_oneof![2 => Just(json!(2)), 1 => Just(json!(1)), 1 => Just(json!(3)), 1 => arb_junk()],
        permissions in prop_oneof![Just(Value::Null), Just(json!([])), Just(json!(["doc.read", "doc.write"])), Just(json!(["doc.write"]))],
    ) {
        let mut m = json!({"id": "fuzz.inputs", "name": "Inputs", "kind": kind, "api": api});
        if kind == "exporter" {
            m["extensions"] = json!(["fz"]);
        }
        if !permissions.is_null() {
            m["permissions"] = permissions;
        }
        m["inputs"] = match (inputs, long) {
            (_, Some(n)) => json!(vec!["image"; n]),
            (Some(Value::Null) | None, None) => json!(names),
            (Some(junk), None) => junk,
        };
        let text = m.to_string();
        let parsed = catch_quiet(|| vectorcraft_plugins::Manifest::parse(text.as_bytes()));
        let parsed = parsed.map_err(|msg| TestCaseError::fail(format!("panicked on {text}: {msg}")))?;
        if let Ok(man) = &parsed {
            let kinds = man.input_kinds();
            prop_assert_eq!(kinds.len(), man.inputs.len(), "{}", text);
            prop_assert!(kinds.iter().enumerate().all(|(i, k)| !kinds[..i].contains(k)), "{}", text);
            prop_assert!(kinds.is_empty() || (man.kind == vectorcraft_plugins::Kind::Filter && man.api >= 2), "{}", text);
            let reads = man.permissions.as_ref().is_some_and(|p| p.iter().any(|n| n == "doc.read"));
            prop_assert!(kinds.is_empty() || (man.asks_permissions() && reads), "{}", text);
        }
        let mut s = rich_session();
        let module = wat::parse_str(tpl::module(&text, tpl::ECHO, "", 1)).unwrap();
        let data = vectorcraft_format::base64_encode(&module);
        let grant = json!(["doc.read", "doc.write", "state", "effect.render", "export.write"]);
        let before = s.doc().unwrap().doc.clone();
        let r = catch_quiet(|| {
            let installed = s.execute("plugin.install", &json!({"dataBase64": data, "grant": grant})).is_ok();
            let _ = s.execute("select.all", &json!({}));
            let ran = installed && s.execute("plugin.run", &json!({"id": "fuzz.inputs"})).is_ok();
            (installed, ran)
        });
        let (installed, ran) = r.map_err(|msg| TestCaseError::fail(format!("panicked running {text}: {msg}")))?;
        prop_assert!(!installed || parsed.is_ok(), "a manifest that doesn't parse installed: {}", text);
        if ran {
            prop_assert!(s.doc().unwrap().doc.layers == before.layers, "an echo changed the document: {}", text);
        }
        renders(&s, &text)?;
        vectorcraft_plugins::registry::remove("fuzz.inputs");
        vectorcraft_plugins::registry::revoke("fuzz.inputs");
    }

    /// A plug-in that calls the storage imports with hostile keys (`..`, separators, device names,
    /// control characters, too long), values, lengths and pointers in and out of its memory runs to
    /// an output, never a panic; every call returns a length or a storage code, and the settings and
    /// cache of another plug-in in the same store stay as they were.
    #[test]
    fn hostile_storage_arguments_never_crash(
        calls in prop::collection::vec(
            (
                0..6usize,
                prop_oneof![any::<i32>(), 0x1ff00..0x20100i32, Just(0x2000)],
                prop_oneof![any::<i32>(), 0..80i32],
                prop_oneof![any::<i32>(), 0x1ff00..0x20100i32, Just(0x3000)],
                prop_oneof![any::<i32>(), 0..5000i32],
            ),
            0..16,
        ),
        key in prop::collection::vec(prop_oneof![Just(b'a'), Just(b'.'), Just(b'/'), Just(b'\\'), Just(b'_'), any::<u8>()], 0..80),
    ) {
        use vectorcraft_plugins::host::STORAGE_BUDGET;
        use vectorcraft_plugins::storage::Meter;
        const FUNCS: [&str; 6] = ["vc_settings_get", "vc_settings_set", "vc_cache_read", "vc_cache_write", "vc_settings_delete", "vc_cache_clear"];
        let folder = std::env::temp_dir().join(format!("vectorcraft-plugin-fuzz-storage-{}", std::process::id()));
        let mut s = Session::new();
        s.attach_plugin_store(folder.clone());
        let storing = |id: &str| format!(r#"{{"id":"{id}","name":"F","kind":"filter","api":2,"permissions":["settings","cache"]}}"#);
        let grant = json!(["settings", "cache"]);
        let other = wat::parse_str(tpl::returning(&storing("fuzz.storage.other"), r#"{"objects":[]}"#)).unwrap();
        s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&other), "grant": grant})).unwrap();
        let theirs = vectorcraft_plugins::registry::storage("fuzz.storage.other").unwrap();
        let m = &mut Meter::unlimited();
        theirs.set_setting("mine", b"kept", m).unwrap();
        theirs.set_cache_entry("mine", b"kept", m).unwrap();
        // Each call's result is checked: a length (0 or more) or one of the storage codes.
        let check = |call: String| format!("(local.set $r {call}) (if (i32.lt_s (local.get $r) (i32.const {STORAGE_BUDGET})) (then unreachable))");
        let body: String = calls
            .iter()
            .map(|&(f, kp, kl, vp, vl)| match f {
                0..4 => check(format!("(call ${} (i32.const {kp}) (i32.const {kl}) (i32.const {vp}) (i32.const {vl}))", FUNCS[f])),
                _ => check(format!("(call ${} (i32.const {kp}) (i32.const {kl}))", FUNCS[f])),
            })
            .collect();
        let text: String = key.iter().map(|b| format!("\\{b:02x}")).collect();
        let imports = format!(r#"{} (data (i32.const 0x2000) "{text}")"#, tpl::STORAGE_IMPORTS);
        let wasm = wat::parse_str(tpl::with_imports(&storing("fuzz.storage"), &imports, &format!("(local $r i32) {body}"), r#"{"objects":[]}"#)).unwrap();
        let r = catch_quiet(|| {
            s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant})).map_err(|e| e.to_string())?;
            let p = vectorcraft_plugins::registry::get("fuzz.storage").ok_or("not installed")?;
            p.run(b"{\"objects\":[]}", b"{}").map_err(|e| e.to_string())
        });
        let out = r.map_err(|msg| TestCaseError::fail(format!("panicked on {calls:?}: {msg}")))?;
        prop_assert!(out.is_ok(), "{:?}", out);
        prop_assert_eq!(theirs.setting("mine", m).unwrap(), Some(b"kept".to_vec()));
        prop_assert_eq!(theirs.cache_entry("mine", m).unwrap(), Some(b"kept".to_vec()));
        prop_assert_eq!(theirs.cache_bytes(), 4);
        // The host names every file: the store holds the two modules, their grants and their folders.
        let names = |dir: &std::path::Path| std::fs::read_dir(dir).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect::<Vec<_>>();
        prop_assert!(names(&folder).len() <= 4, "{:?}", names(&folder));
        prop_assert!(names(&folder.join("storage")).len() <= 2, "{:?}", names(&folder.join("storage")));
        let _ = s.execute("plugin.remove", &json!({"id": "fuzz.storage"}));
        let _ = s.execute("plugin.remove", &json!({"id": "fuzz.storage.other"}));
    }
}
