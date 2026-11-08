//! Untrusted plug-ins never crash the app: mutated and garbage modules handed to `plugin.install`,
//! and plug-ins returning hostile output (random JSON in the object model's shape: huge or
//! malformed coordinates, unknown ids, broken paints and gradients, junk of every type) as object
//! filters (`plugin.run`) or live effects (`effect.apply`). Each run must end as an error that
//! leaves the document as it was, or as a document that then renders and exports, without a
//! panic.
//!
//! `PROPTEST_CASES=2000 cargo test -p vectorcraft-engine --test plugin_fuzz` runs a deeper search.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use serde_json::{Map, Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;
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

fn install(s: &mut Session, wasm: &[u8]) -> Result<Value, String> {
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(wasm)})).map_err(|e| e.to_string())
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

    /// A live effect that declares `paintOutput` returning hostile pieces, on objects or on a fill:
    /// drawing, export and Expand Appearance never crash.
    #[test]
    fn hostile_painted_effect_output_never_crashes(
        objects in prop::collection::vec(arb_object(), 0..5),
        junk in prop::option::weighted(0.1, arb_junk()),
        on_fill in any::<bool>(),
    ) {
        let mut s = rich_session();
        s.execute("select.all", &json!({})).unwrap();
        let out = output(&objects, &[], junk);
        let manifest = r#"{"id":"fuzz.painted","name":"Fuzz","version":"1","kind":"effect","api":2,"paintOutput":true,"params":{}}"#;
        install(&mut s, &wat::parse_str(tpl::returning(manifest, &out)).unwrap()).unwrap();
        let item = if on_fill { json!(0) } else { Value::Null };
        let r = catch_quiet(|| s.execute("effect.apply", &json!({"effect": "plugin.fuzz.painted", "item": item})).map(|_| ()));
        if let Err(msg) = r {
            return Err(TestCaseError::fail(format!("panicked on {out}: {msg}")));
        }
        renders(&s, &out)?;
        let r = catch_quiet(|| s.execute("effect.expandAppearance", &json!({})).map(|_| ()));
        if let Err(msg) = r {
            return Err(TestCaseError::fail(format!("expanding panicked on {out}: {msg}")));
        }
        renders(&s, &out)?;
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
}
