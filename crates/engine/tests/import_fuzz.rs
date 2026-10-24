//! Untrusted files never crash the app: garbage, truncated, mutated and hostile SVG, PDF and DXF input,
//! mutated raster images (Photoshop documents among them) placed with File → Place, and swatch (`.vcswatches`, `.gpl`, `.ase`), graphic style (`.vcstyles`) and flattener preset
//! (`.vcflattener`) libraries, shortcut sets other apps save as text (`.kys`), and native files (compressed, damaged, saved for older versions),
//! must load as an error or as a document that then renders and exports, without a panic; nor may
//! bitmaps, PDF and text pasted from other apps, nor EMF and WMF pictures (damaged files, records
//! of every kind with random contents) opened, placed or pasted, nor EPS and PostScript files
//! (damaged ones, hostile programs) read by the PostScript interpreter, nor the editing data of
//! Illustrator EPS and `.ai` files (the layers they carry, damaged or hostile), nor Affinity documents
//! (mutated object streams, archives and indexed PNG previews, hostile image dimensions), nor the
//! font files a folder search reads (damaged fonts, collections whose headers give any count of
//! faces), nor scripts written for the reference app checked without running them (garbage,
//! generated script text and mutated scripts).
//!
//! `PROPTEST_CASES=20000 cargo test -p vectorcraft-engine --test import_fuzz` runs a deeper search.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_testkit::catch_quiet;
use vectorcraft_testkit::fixtures::rich_session;

/// 64 cases each, unless `PROPTEST_CASES` asks for more.
fn config() -> ProptestConfig {
    let mut c = ProptestConfig { failure_persistence: None, ..ProptestConfig::default() };
    if std::env::var_os("PROPTEST_CASES").is_none() {
        c.cases = 64;
    }
    c
}

// ---------- Affinity previews ----------

/// Original pixels in a synthetic container envelope, not an Affinity document writer.
fn affinity_preview_sample() -> Vec<u8> {
    let im = image::RgbaImage::from_pixel(2, 1, image::Rgba([220, 40, 60, 255]));
    let mut png = Vec::new();
    im.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut b = vec![0; 72];
    b[..4].copy_from_slice(vectorcraft_affinity::MAGIC);
    b[4..6].copy_from_slice(&12u16.to_le_bytes());
    b[8..12].copy_from_slice(b"nsrP");
    b[12..16].copy_from_slice(b"#Inf");
    b[24..32].copy_from_slice(&72u64.to_le_bytes());
    b[64..68].copy_from_slice(b"Prot");
    b.extend(b"\xff\xff\xff\xffThmb");
    b.extend(1u32.to_le_bytes());
    b.extend((png.len() as u32 + 13).to_le_bytes());
    b.extend(29u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend((png.len() as u32).to_le_bytes());
    b.push(1);
    b.extend(png);
    b
}

/// A synthetic native document exercising the reader's paths: curves (with live corners), every
/// parametric shape it knows, a compound, gradients, a stroke, text, a mask and an artboard. Not
/// an Affinity writer: the layout is the one the reader accepts.
fn affinity_native_stream() -> Vec<u8> {
    use vectorcraft_affinity::synth::{self, F, tag};
    let f32s = |v: &[f32]| F::Struct(v.iter().flat_map(|x| x.to_le_bytes()).collect());
    let rec = |x: f64, y: f64, role: u8| {
        let mut r = x.to_le_bytes().to_vec();
        r.extend(y.to_le_bytes());
        r.extend([1, role]);
        r
    };
    let mut id = 100;
    let mut next = || {
        id += 1;
        id
    };
    let colour = |id: u32| F::Def(id, vec![tag(b"RGBA")], vec![(tag(b"_col"), f32s(&[0.2, 0.4, 0.6, 0.8]))]);
    let gradient = |a: u32, b: u32, c: u32, kind: u16| {
        F::Def(
            a,
            vec![tag(b"FDsc")],
            vec![
                (
                    tag(b"FDeF"),
                    F::Def(
                        b,
                        vec![tag(b"FilG")],
                        vec![
                            (tag(b"Type"), F::Enum(kind, 0)),
                            (tag(b"Grad"), F::Obj(tag(b"Grad"), vec![(tag(b"Cols"), F::Shared(vec![colour(c), F::Ref(c)]))])),
                        ],
                    ),
                ),
                (tag(b"FDeX"), F::F64s(vec![10.0, 0.0, 5.0, 0.0, 20.0, 5.0])),
            ],
        )
    };
    let curve = F::Obj(
        tag(b"PCvD"),
        vec![(
            tag(b"Data"),
            F::Pos(vec![
                F::U8(0),
                F::U32(1),
                F::Bool(true),
                F::Records(18, vec![rec(0.0, 0.0, 0), rec(5.0, 0.0, 1), rec(10.0, 5.0, 2), rec(10.0, 10.0, 0), rec(0.0, 10.0, 0), rec(0.0, 0.0, 0)]),
            ]),
        )],
    );
    let mut kids = vec![F::Def(
        next(),
        vec![tag(b"PCrv")],
        vec![(tag(b"Crvs"), curve.clone()), (tag(b"BFFl"), F::Shared(vec![gradient(next(), next(), next(), 0)]))],
    )];
    for (i, class) in [b"ShNR", b"ShpE", b"ShPy", b"ShSt", b"ShSS", b"ShPi", b"ShpT", b"ShTz", b"ShCl"].into_iter().enumerate() {
        let fields = vec![
            (tag(b"Side"), F::U32(6)),
            (tag(b"Smth"), F::Bool(i % 2 == 0)),
            (tag(b"Curv"), F::F32(0.5)),
            (tag(b"CTyp"), F::Raw(vec![0xaa, b'p', b'y', b'T', b'C', 4, 0, 0, 0, 0, 0, 0, 0, 4, 0, 4, 0, 4, 0])),
            (tag(b"ShCR"), f32s(&[0.25, 0.0, 0.0, 0.0])),
            (tag(b"AngE"), F::F32(1.0)),
        ];
        kids.push(F::Def(
            next(),
            vec![tag(b"ShpN")],
            vec![
                (tag(b"Shpe"), F::Def(next(), vec![tag(class)], fields)),
                (tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 40.0 + i as f64, 30.0])),
                (tag(b"Xfrm"), F::F64s(vec![1.0, 0.2, i as f64 * 10.0, 0.0, 1.0, 5.0])),
                (tag(b"BFFl"), F::Shared(vec![gradient(next(), next(), next(), (i % 4) as u16)])),
                (tag(b"AdCh"), F::Shared(vec![F::Def(next(), vec![tag(b"PCrv")], vec![(tag(b"Crvs"), curve.clone())])])),
            ],
        ));
    }
    kids.push(F::Def(
        next(),
        vec![tag(b"Comp")],
        vec![(tag(b"Chld"), F::Shared(vec![F::Def(next(), vec![tag(b"PCrv")], vec![(tag(b"Crvs"), curve.clone()), (tag(b"ComO"), F::Enum(2, 0))])]))],
    ));
    let run = F::Def(
        next(),
        vec![tag(b"GAtt")],
        vec![(tag(b"Doub"), F::Raw(vec![0x8a, b'b', b'u', b'o', b'D', 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x28, 0x40]))],
    );
    let block = F::Def(
        next(),
        vec![tag(b"StBl")],
        vec![
            (tag(b"Glyp"), F::Obj(tag(b"GStr"), vec![(tag(b"Utf8"), F::Str("Hi\u{2029}there\0".into()))])),
            (
                tag(b"GAtt"),
                F::Obj(tag(b"GlAS"), vec![(tag(b"Runs"), F::Objs(tag(b"GlAR"), vec![vec![(tag(b"Indx"), F::I32(9)), (tag(b"Item"), run)]]))]),
            ),
        ],
    );
    kids.push(F::Def(
        next(),
        vec![tag(b"TxtA")],
        vec![
            (tag(b"StSt"), F::Def(next(), vec![tag(b"Stry")], vec![(tag(b"Blok"), F::Shared(vec![block]))])),
            (
                tag(b"TxtH"),
                F::Def(next(), vec![tag(b"ArFr")], vec![(tag(b"FrmB"), F::F64s(vec![0.0, -10.0, 30.0, 0.0])), (tag(b"ArtV"), F::F64(10.0))]),
            ),
        ],
    ));
    let board = F::Def(
        next(),
        vec![tag(b"ShpN")],
        vec![
            (tag(b"ABEn"), F::Bool(true)),
            (tag(b"Shpe"), F::Def(next(), vec![tag(b"ShNR")], vec![])),
            (tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 200.0, 100.0])),
            (tag(b"Chld"), F::Shared(kids)),
        ],
    );
    synth::stream(&[
        (tag(b"UVCn"), F::Obj(tag(b"UVCn"), vec![(tag(b"UPPI"), F::F64(96.0))])),
        (
            tag(b"DocR"),
            F::Def(
                1,
                vec![tag(b"DocN")],
                vec![(tag(b"Chld"), F::Shared(vec![F::Def(2, vec![tag(b"Sprd")], vec![(tag(b"Chld"), F::Shared(vec![board]))])]))],
            ),
        ),
    ])
}

fn affinity_native(stream: &[u8], method: vectorcraft_affinity::synth::Method) -> Vec<u8> {
    vectorcraft_affinity::synth::container(&[("doc.dat", stream, method)], None)
}

#[test]
fn the_native_affinity_sample_reaches_every_reader_path() {
    let bytes = affinity_native(&affinity_native_stream(), vectorcraft_affinity::synth::Method::Zstd);
    let l = vectorcraft_engine::cmd::fileio::load("x.af", &bytes).unwrap();
    assert!(!l.preview_only, "{:?}", l.warnings);
    let kinds = l.doc.layers.iter().flat_map(|l| l.children().into_iter().flatten()).count();
    assert!(kinds > 0);
    let all = format!("{:?}", l.doc.layers);
    for want in ["Path", "Text", "Compound"].iter().take(2) {
        assert!(all.contains(want), "{want} missing");
    }
    assert_eq!(l.doc.artboards.len(), 1);
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn affinity_native_documents_with_mutated_streams_render_and_export(
        cut in 0usize..4096,
        edits in prop::collection::vec((0usize..4096, any::<u8>()), 0..24),
    ) {
        let mut stream = affinity_native_stream();
        let n = stream.len();
        for (at, b) in edits { stream[at % n] = b; }
        stream.truncate(cut.max(16));
        let bytes = affinity_native(&stream, vectorcraft_affinity::synth::Method::Stored);
        survive("Affinity native stream", || vectorcraft_engine::cmd::fileio::load("x.af", &bytes).ok().map(|l| l.doc))?;
    }

    #[test]
    fn affinity_native_containers_with_mutated_bytes_render_and_export(
        edits in prop::collection::vec((0usize..8192, any::<u8>()), 1..16),
        method in 0u8..3,
    ) {
        use vectorcraft_affinity::synth::Method;
        let m = [Method::Stored, Method::Zlib, Method::Zstd][usize::from(method)];
        let mut bytes = affinity_native(&affinity_native_stream(), m);
        let n = bytes.len();
        for (at, b) in edits { bytes[at % n] = b; }
        survive("Affinity native container", || vectorcraft_engine::cmd::fileio::load("x.af", &bytes).ok().map(|l| l.doc))?;
    }

    #[test]
    fn affinity_garbage_never_panics(tail in prop::collection::vec(any::<u8>(), 0..2048)) {
        let mut bytes = vectorcraft_affinity::MAGIC.to_vec();
        bytes.extend(tail);
        survive("Affinity garbage", || vectorcraft_engine::cmd::fileio::load("x.af", &bytes).ok().map(|l| l.doc))?;
    }

    #[test]
    fn affinity_mutated_previews_render_and_export_without_panicking(
        cut in 0usize..400,
        edits in prop::collection::vec((0usize..400, any::<u8>()), 0..16),
    ) {
        let mut bytes = affinity_preview_sample();
        for (at, b) in edits {
            if let Some(byte) = bytes.get_mut(at) { *byte = b; }
        }
        bytes.truncate(cut);
        survive("Affinity preview", || vectorcraft_engine::cmd::fileio::load("x.af", &bytes).ok().map(|l| l.doc))?;
    }

    #[test]
    fn affinity_placement_is_rejected_without_panicking(tail in prop::collection::vec(any::<u8>(), 0..2048)) {
        let mut bytes = vectorcraft_affinity::MAGIC.to_vec();
        bytes.extend(tail);
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
            let p = json!({"name":"renamed.png", "dataBase64":vectorcraft_format::base64_encode(&bytes)});
            for cmd in ["file.place", "file.place.info"] { assert!(no_panic(s.execute(cmd, &p)).is_err()); }
            assert!(no_panic(s.execute("file.place.queue", &json!({"files":[p]}))).is_err());
        });
        prop_assert!(r.is_ok(), "Affinity placement panicked: {:?}", r.err());
    }
}

// ---------- Affinity documents with every feature the reader maps ----------

#[path = "support/affinity_docs.rs"]
mod affinity_docs;
#[path = "../../affinity/tests/support/text_fixture.rs"]
mod text_fixture;

/// One sample document: its object stream and, for embedded content, the embedded document's.
struct AffinitySample {
    name: &'static str,
    stream: Vec<u8>,
    embedded: Option<Vec<u8>>,
}

/// Original synthetic documents (not an Affinity writer) reaching what the reader maps beyond
/// [`affinity_native_stream`]: layer effects and colour overlays (also over tables), stroke
/// styles, dashes and brushes, paragraph alignment, frame text, underline, strikethrough and
/// line breaks inside a paragraph, tables with merged and two-line cells, master pages, CMYK
/// pixel layers with and without the document's CMYK profile, and embedded documents.
fn affinity_feature_samples() -> Vec<AffinitySample> {
    use affinity_docs::*;
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let fx = vec![shadow(), effect(b"OutG", 6.0), effect(b"InnS", 3.0), effect(b"BevE", 4.0), outline(0, 3.0), outline(2, 4.0), colour_overlay(0.5)];
    let canvas = canvas_stream(vec![
        with_effects(stroked(10, 1, &[]), fx),
        stroked(11, 0, &[]),
        stroked(12, 2, &[4.0, 2.0]),
        stroked(13, 3, &[]),
        mixed_alignment(1, 2),
        two_line_frame(150.0),
        decorated_text(70),
        with_effects(table_with(30, identity, ["A", "B\u{2028}two", "C", "D", "E"]), vec![colour_overlay(1.0)]),
    ]);
    let icc = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::GENERIC_CMYK).unwrap();
    let inner = canvas_stream(vec![with_effects(rectangle(10, (0.0, 0.0), (50.0, 50.0)), vec![colour_overlay(1.0)]), artistic_text(20, "Hi", 40.0)]);
    vec![
        AffinitySample { name: "effects, strokes, text and tables", stream: canvas, embedded: None },
        AffinitySample { name: "master pages", stream: master_stream(3), embedded: None },
        AffinitySample { name: "CMYK pixels with a profile", stream: cmyk_stream(false, Some(&icc)), embedded: None },
        AffinitySample { name: "CMYK pixels with transparency", stream: cmyk_stream(true, None), embedded: None },
        AffinitySample { name: "embedded document", stream: canvas_stream(vec![embedded_node(20)]), embedded: Some(inner) },
    ]
}

/// A sample's file: its streams stored uncompressed, so mutations reach the object graph
/// instead of failing a decompressor's checks.
fn affinity_sample_file(stream: &[u8], embedded: Option<&[u8]>) -> Vec<u8> {
    use vectorcraft_affinity::synth::Method;
    let inner = embedded.map(|e| affinity_docs::packed(e, None, Method::Stored));
    affinity_docs::packed(stream, inner.as_deref(), Method::Stored)
}

#[test]
fn the_affinity_feature_samples_reach_what_they_are_for() {
    use vectorcraft_doc::NodeKind;
    let samples = affinity_feature_samples();
    let open = |i: usize| {
        let s: &AffinitySample = &samples[i];
        let l = vectorcraft_engine::cmd::fileio::load("x.af", &affinity_sample_file(&s.stream, s.embedded.as_deref())).unwrap();
        assert!(!l.preview_only, "{}: {:?}", s.name, l.warnings);
        l
    };
    fn all(d: &Document) -> Vec<&vectorcraft_doc::Node> {
        fn walk<'a>(nodes: &'a [std::sync::Arc<vectorcraft_doc::Node>], out: &mut Vec<&'a vectorcraft_doc::Node>) {
            for n in nodes {
                out.push(n);
                if let Some(c) = n.children() {
                    walk(c, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&d.layers, &mut out);
        out
    }
    let l = open(0);
    let nodes = all(&l.doc);
    let runs: Vec<&vectorcraft_doc::text::TextRun> =
        nodes.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.runs.iter()) } else { None }).flatten().collect();
    assert!(runs.iter().any(|r| r.style.underline) && runs.iter().any(|r| r.style.strikethrough));
    assert!(runs.iter().any(|r| r.text.contains("two")), "the table's two-line cell");
    assert!(nodes.iter().any(|n| n.appearance.effects.iter().any(|e| e.id == "stylize.dropShadow")));
    assert!(l.warnings.iter().any(|w| w.contains("bevel")), "{:?}", l.warnings);
    // Line styles: solid, None (no stroke), dashed and brush.
    use vectorcraft_doc::appearance::{AppearanceItem, StrokeLayer};
    let strokes = |n: &vectorcraft_doc::Node| -> Vec<StrokeLayer> {
        n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.clone()) } else { None }).collect()
    };
    let boxes: Vec<_> = nodes.iter().filter(|n| n.name.as_deref() == Some("Magenta box")).collect();
    assert_eq!(boxes.len(), 4);
    assert_eq!(boxes.iter().filter(|b| strokes(b).is_empty()).count(), 1, "line style None");
    assert!(boxes.iter().any(|b| strokes(b).iter().any(|s| s.dash.is_some())), "dashed");
    let l = open(1);
    assert_eq!(l.doc.artboards.len(), 3);
    assert!(l.doc.layers.iter().any(|n| n.name.as_deref() == Some("Master A")));
    for i in [2, 3] {
        assert!(all(&open(i).doc).iter().any(|n| matches!(n.kind, NodeKind::Image(_))), "{}", samples[i].name);
    }
    assert!(all(&open(4).doc).iter().any(|n| n.name.as_deref() == Some("Magenta box")));
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn affinity_feature_documents_with_mutated_streams_render_and_export(
        sample in 0usize..5,
        inner in any::<bool>(),
        cut in 0usize..8192,
        edits in prop::collection::vec((0usize..8192, any::<u8>()), 0..24),
    ) {
        let samples = affinity_feature_samples();
        let s = &samples[sample % samples.len()];
        let (mut stream, mut embedded) = (s.stream.clone(), s.embedded.clone());
        // Mutate the embedded document's stream instead, where there is one.
        let target = match embedded.as_mut() {
            Some(e) if inner => e,
            _ => &mut stream,
        };
        let n = target.len();
        for (at, b) in edits { target[at % n] = b; }
        target.truncate(cut.max(16));
        let bytes = affinity_sample_file(&stream, embedded.as_deref());
        survive(s.name, || vectorcraft_engine::cmd::fileio::load("x.af", &bytes).ok().map(|l| l.doc))?;
    }
}

/// Import must not panic; whatever comes back must render and export without panicking either.
fn survive(what: &str, import: impl FnOnce() -> Option<Document>) -> Result<(), TestCaseError> {
    let r = catch_quiet(|| {
        if let Some(d) = import() {
            let mut r = vectorcraft_render::Renderer::new();
            if let Some(ab) = d.artboards.first() {
                let scale = (256.0 / ab.rect.width().max(ab.rect.height()).max(1.0)).min(1.0);
                if vectorcraft_render::raster_size(ab.rect, scale).is_ok() {
                    let _ = r.render_region(&d, ab.rect, scale, true).to_png();
                }
            }
            let _ = vectorcraft_svg::export(&d, &vectorcraft_svg::ExportOptions::default());
            let _ = vectorcraft_engine::export_pdf(&d, &vectorcraft_pdf::PdfOptions::default());
            let _ = vectorcraft_format::save_file(&d);
        }
    });
    r.map_err(|msg| TestCaseError::fail(format!("{what}: panicked: {msg}")))
}

fn rich_doc() -> Document {
    (*rich_session().doc().unwrap().doc).clone()
}

fn rich_svg() -> String {
    vectorcraft_svg::export(&rich_doc(), &vectorcraft_svg::ExportOptions::default())
}

fn rich_pdf() -> Vec<u8> {
    vectorcraft_engine::export_pdf(&rich_doc(), &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap()
}

/// Numbers that tend to break arithmetic: zero, negatives, huge, tiny, exponents, junk.
fn arb_num() -> impl Strategy<Value = String> {
    prop_oneof![
        (-1000.0f64..1000.0).prop_map(|v| format!("{v:.2}")),
        Just("0".to_string()),
        Just("-0".to_string()),
        Just("1e308".to_string()),
        Just("-1e308".to_string()),
        Just("1e-308".to_string()),
        Just("1e15".to_string()),
        Just("4294967296".to_string()),
        Just("-2147483649".to_string()),
        Just("NaN".to_string()),
        Just("inf".to_string()),
        Just("".to_string()),
        Just("1e".to_string()),
        Just("--1".to_string()),
    ]
}

// ---------- SVG ----------

fn arb_svg_attr() -> impl Strategy<Value = String> {
    let names = prop::sample::select(vec![
        "x",
        "y",
        "width",
        "height",
        "rx",
        "ry",
        "r",
        "cx",
        "cy",
        "x1",
        "y1",
        "x2",
        "y2",
        "dx",
        "dy",
        "font-size",
        "stroke-width",
        "stroke-dasharray",
        "stroke-dashoffset",
        "stroke-miterlimit",
        "opacity",
        "fill-opacity",
        "offset",
        "startOffset",
        "letter-spacing",
        "rotate",
        "textLength",
        "fx",
        "fy",
        "fr",
        "points",
        "viewBox",
        "transform",
        "d",
        "fill",
        "stroke",
        "stop-color",
        "mask-type",
        "data-vectorcraft-mask",
        "href",
        "baseline-shift",
        "word-spacing",
        "kerning",
        "writing-mode",
        "font-weight",
        "font-variation-settings",
        "text-anchor",
        "side",
        "path",
        // Filters, spreads, hidden objects and linked images.
        "spreadMethod",
        "display",
        "filter",
        "clip-path",
        "stdDeviation",
        "flood-color",
        "flood-opacity",
        "in",
        "in2",
        "result",
        "operator",
        "values",
        "tableValues",
        "type",
        "preserveAspectRatio",
        "overflow",
        "data-vc-blend",
        // SVG Interactivity: event attributes, kept as text.
        "onclick",
        "onload",
    ]);
    let axes = prop::sample::select(vec!["wght", "wdth", "opsz", "wd", "slnt!", ""]);
    let value = prop_oneof![
        arb_num(),
        prop::collection::vec(arb_num(), 0..7).prop_map(|v| v.join(" ")),
        prop::collection::vec(arb_num(), 0..7).prop_map(|v| format!("matrix({})", v.join(" "))),
        prop::collection::vec(arb_num(), 0..3).prop_map(|v| format!("rotate({}) scale({})", v.join(" "), v.join(","))),
        prop::collection::vec((prop::sample::select(vec!["M", "L", "C", "Q", "A", "Z", "H", "V", "S", "T", "m", "a"]), arb_num()), 0..12)
            .prop_map(|v| v.iter().map(|(c, n)| format!("{c}{n} {n}")).collect::<Vec<_>>().join(" ")),
        prop::collection::vec(arb_num(), 0..2).prop_map(|v| format!("{}%", v.join(""))),
        // Variable font axis settings, well formed or not.
        prop::collection::vec((axes, arb_num()), 0..5).prop_map(|v| v.iter().map(|(t, n)| format!("'{t}' {n}")).collect::<Vec<_>>().join(", ")),
        prop::sample::select(vec!["normal", "initial", "unset"]).prop_map(str::to_string),
        // Colours (Lab too), mask options and links.
        prop::collection::vec(arb_num(), 0..4).prop_map(|v| format!("lab({})", v.join(" "))),
        prop::sample::select(vec!["noclip invert", "invert", "noclip", "alpha", "luminance", "url(#e)", "#e", "#zz", "none", "currentColor"])
            .prop_map(str::to_string),
        prop::sample::select(vec![
            "reflect",
            "repeat",
            "SourceAlpha",
            "SourceGraphic",
            "e",
            "over",
            "in",
            "table",
            "1 0",
            "multiply",
            "img/x%2.png",
            "file:///C:/nope.png",
            "https://x/y.png",
            "data:image/svg+xml;base64,PHN2Zy8+",
        ])
        .prop_map(str::to_string),
    ];
    (names, value).prop_map(|(n, v)| format!(" {n}=\"{v}\""))
}

fn arb_svg_element(depth: u32) -> BoxedStrategy<String> {
    let tag = prop::sample::select(vec![
        "rect",
        "circle",
        "ellipse",
        "line",
        "polyline",
        "polygon",
        "path",
        "text",
        "tspan",
        "textPath",
        "g",
        "use",
        "image",
        "linearGradient",
        "radialGradient",
        "stop",
        "pattern",
        "clipPath",
        "mask",
        "symbol",
        "defs",
        "svg",
        "marker",
        "a",
        "filter",
        "feGaussianBlur",
        "feOffset",
        "feFlood",
        "feComposite",
        "feMerge",
        "feMergeNode",
        "feColorMatrix",
        "feDropShadow",
        "feComponentTransfer",
        "feFuncA",
        "script",
    ]);
    let attrs = || prop::collection::vec(arb_svg_attr(), 0..6).prop_map(|v| v.concat());
    let leaf = (tag.clone(), attrs(), "[a-z ]{0,6}").prop_map(|(t, a, txt)| format!("<{t} id=\"e\"{a} href=\"#e\">{txt}</{t}>"));
    if depth == 0 {
        return leaf.boxed();
    }
    prop_oneof![
        leaf,
        (tag, attrs(), prop::collection::vec(arb_svg_element(depth - 1), 0..4)).prop_map(|(t, a, ch)| format!("<{t}{a}>{}</{t}>", ch.concat()))
    ]
    .boxed()
}

/// A character-level edit that keeps the string valid UTF-8 (the importer takes `&str`).
fn mutate_text(src: &str, cut: usize, edits: &[(usize, char)]) -> String {
    let mut chars: Vec<char> = src.chars().collect();
    for &(i, c) in edits {
        if !chars.is_empty() {
            let n = chars.len();
            chars[i % n] = c;
        }
    }
    chars.truncate(cut.min(chars.len()));
    chars.into_iter().collect()
}

/// Type shifted two billion points down exports to PDF: an input of
/// `svg_hostile_markup_never_panics`, cut down to its type.
#[test]
fn svg_type_shifted_two_billion_points_exports_to_pdf() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text baseline-shift="-2147483649">f</text></svg>"#;
    let d = vectorcraft_svg::import(svg).unwrap();
    assert!(vectorcraft_engine::export_pdf(&d, &vectorcraft_pdf::PdfOptions::default()).is_ok());
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn svg_garbage_never_panics(s in ".{0,300}") {
        survive("svg garbage", || vectorcraft_svg::import(&s).ok())?;
    }

    #[test]
    fn svg_hostile_markup_never_panics(
        root in prop::collection::vec(arb_svg_attr(), 0..4),
        body in prop::collection::vec(arb_svg_element(2), 0..6),
    ) {
        let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"{}>{}</svg>", root.concat(), body.concat());
        survive(&svg, || vectorcraft_svg::import(&svg).ok())?;
    }

    #[test]
    fn svg_hostile_css_never_panics(
        sheet in r#"[a-z#.>*\[\]=:;{}@ !"'/,-]{0,160}"#,
        decls in r#"[a-z0-9.%: ;!-]{0,60}"#,
        body in prop::collection::vec(arb_svg_element(1), 0..3),
    ) {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>{sheet}</style><g class=\"a b\"><text id=\"t\" class=\"a\" x=\"1 2 3\" y=\"9\" rotate=\"5\" style=\"{decls}\">ab<tspan dy=\"3\">c</tspan></text></g>{}</svg>",
            body.concat()
        );
        survive(&svg, || vectorcraft_svg::import(&svg).ok())?;
    }

    #[test]
    fn svg_mutated_export_never_panics(
        cut in 0usize..20_000,
        edits in prop::collection::vec((0usize..20_000, prop::sample::select(vec!['<', '>', '"', '/', '-', '9', 'e', '.', ' ', '#', '%', '&', ';', 'x'])), 0..10),
    ) {
        let svg = mutate_text(&rich_svg(), cut, &edits);
        survive("mutated svg", || vectorcraft_svg::import(&svg).ok())?;
    }
}

// ---------- PDF ----------

fn handmade_pdf(content: &str, resources: &str, media: &str) -> Vec<u8> {
    handmade_pdf_with(content, resources, media, &[])
}

/// The same, followed by the objects `more` (numbered from 5).
fn handmade_pdf_with(content: &str, resources: &str, media: &str, more: &[Vec<u8>]) -> Vec<u8> {
    let page = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox {media} /Contents 4 0 R /Resources {resources} >>"),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
    ];
    pdf_of(&page.into_iter().map(String::into_bytes).chain(more.iter().cloned()).collect::<Vec<_>>())
}

/// The PDF of the objects `objs` (numbered from 1, the catalog first).
fn pdf_of(objs: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// A file filling thousands of paths with function-based shadings, each a long PostScript
/// calculator function of fine stripes, opens in bounded time: each shading is read once, and all
/// of them within one budget of function evaluations per import (#3621 review).
#[test]
fn pdf_many_paths_filled_with_long_function_shadings_open_in_bounded_time() {
    let shadings = 20;
    // Five hundred no-op steps, then stripes: red, then white, along x.
    let program = format!("{{ pop {}dup truncate sub 0.5 le {{ 1 0 0 }} {{ 1 1 1 }} ifelse }}", "1 mul ".repeat(500));
    let patterns: String = (0..shadings)
        .map(|i| format!("/P{i} << /PatternType 2 /Shading << /ShadingType 1 /ColorSpace /DeviceRGB /Domain [0 {} 0 1] /Matrix [1 0 0 100 0 0] /Function {} 0 R >> >> ", 200 + i, 5 + i))
        .collect();
    let functions: Vec<Vec<u8>> = (0..shadings)
        .map(|i| {
            format!("<< /FunctionType 4 /Domain [0 {} 0 1] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n{program}\nendstream", 200 + i, program.len())
                .into_bytes()
        })
        .collect();
    let content: String = (0..5000).map(|i| format!("/Pattern cs /P{} scn {} {} 2 2 re f\n", i % shadings, i % 100 * 2, i / 100 * 2)).collect();
    let pdf = handmade_pdf_with(&content, &format!("<< /Pattern << {patterns}>> >>"), "[0 0 200 100]", &functions);
    let start = std::time::Instant::now();
    let d = vectorcraft_pdf::import(&pdf).unwrap();
    let took = start.elapsed();
    assert!(!d.layers.is_empty());
    assert!(took < std::time::Duration::from_secs(30), "took {took:?}");
}

/// One content-stream operation with junk operands.
fn arb_pdf_op() -> impl Strategy<Value = String> {
    let op = prop::sample::select(vec![
        "m", "l", "c", "v", "y", "h", "re", "f", "f*", "F", "S", "s", "B", "B*", "b", "n", "W", "W*", "q", "Q", "cm", "w", "J", "j", "M", "d", "ri",
        "i", "gs", "g", "G", "rg", "RG", "k", "K", "cs", "CS", "sc", "SC", "scn", "SCN", "sh", "Do", "BT", "ET", "Tf", "Td", "TD", "Tm", "T*", "Tc",
        "Tw", "Tz", "TL", "Tr", "Ts", "Tj", "TJ", "'", "\"", "BMC", "BDC", "EMC", "d0", "d1",
    ]);
    let operand = prop_oneof![
        arb_num(),
        prop::collection::vec(arb_num(), 0..5).prop_map(|v| format!("[{}]", v.join(" "))),
        Just("/F1".to_string()),
        Just("/Sh0".to_string()),
        Just("/Im0".to_string()),
        Just("/GS0".to_string()),
        Just("/DeviceRGB".to_string()),
        Just("/CS0".to_string()),
        Just("/CS1".to_string()),
        Just("/P0".to_string()),
        Just("/DeviceCMYK".to_string()),
        Just("/Pattern".to_string()),
        Just("(Hi)".to_string()),
        Just("<00ff>".to_string()),
        Just("[(a) -250 (b)]".to_string()),
    ];
    (prop::collection::vec(operand, 0..7), op).prop_map(|(args, op)| format!("{} {op}", args.join(" ")))
}

fn arb_resources() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "<< >>".to_string(),
        "<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>".to_string(),
        "<< /ExtGState << /GS0 << /CA 0.5 /ca -3 /BM /Multiply /SMask /None >> >> >>".to_string(),
        "<< /Shading << /Sh0 << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 1e308 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >> >>".to_string(),
        "<< /Shading << /Sh0 << /ShadingType 3 /ColorSpace /DeviceRGB /Coords [0 0 0 0 0 -5] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [] /N -1 >> >> >> >>".to_string(),
        "<< /XObject << /Im0 << /Type /XObject /Subtype /Image /Width 4294967295 /Height 0 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Length 3 >> >> >>".to_string(),
        "<< /Pattern << /P0 << /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 0 0] /XStep 0 /YStep -0 /Resources << >> >> >> >>".to_string(),
        // Ink colour spaces with junk names, alternates and tint transforms (spot swatch import).
        "<< /ColorSpace << /CS0 [/Separation /All /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [1 1 1 1] /N 1 >>] >> >>".to_string(),
        "<< /ColorSpace << /CS0 [/Separation /#00 /Lab << /FunctionType 2 /Domain [0 1] /C0 [1e308] /C1 [NaN 5] /N -1 >>] >> >>".to_string(),
        "<< /ColorSpace << /CS0 [/DeviceN [] /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>] >> >>".to_string(),
        "<< /ColorSpace << /CS0 [/DeviceN [/A /None /Cyan /All] [/ICCBased << /N 4 >>] << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>] >> >>".to_string(),
        "<< /Shading << /Sh0 << /ShadingType 2 /ColorSpace [/Separation /Ink /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 0 0] /N 1 >>] /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >> >> >>".to_string(),
        // Inks with the same values and other names, as color spaces, a shading pattern and a
        // shading (the page's walk tells them apart).
        "<< /ColorSpace << /CS0 [/Separation /A /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>] /CS1 [/Separation /B /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>] >> /Pattern << /P0 << /PatternType 2 /Shading << /ShadingType 2 /ColorSpace [/Separation /B /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>] /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >> >> >> /Shading << /Sh0 << /ShadingType 2 /ColorSpace [/Separation /A /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>] /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >> >> >>".to_string(),
    ])
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn pdf_garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..600)) {
        let mut b = b"%PDF-1.7\n".to_vec();
        b.extend(bytes);
        survive("pdf garbage", || vectorcraft_pdf::import(&b).ok())?;
    }

    #[test]
    fn pdf_hostile_content_never_panics(
        ops in prop::collection::vec(arb_pdf_op(), 0..40),
        resources in arb_resources(),
        media in prop::sample::select(vec!["[0 0 100 100]", "[0 0 0 0]", "[0 0 1e308 1e308]", "[100 100 0 0]", "[-5 -5 -1 -1]", "[0 0 14400 14400]", "[]", "[0 0 NaN 5]"]),
    ) {
        let pdf = handmade_pdf(&ops.join("\n"), &resources, media);
        survive(&ops.join(" "), || vectorcraft_pdf::import(&pdf).ok())?;
    }

    #[test]
    fn pdf_mutated_export_never_panics(
        cut in 0usize..40_000,
        flips in prop::collection::vec((0usize..40_000, prop::sample::select(vec![b'0', b'9', b'-', b'.', b' ', b'[', b']', b'<', b'>', b'/', b'(', b'e', 0u8, 0xff])), 0..10),
    ) {
        let mut b = rich_pdf();
        for (i, c) in flips {
            let n = b.len();
            b[i % n] = c;
        }
        b.truncate(cut.min(b.len()).max(9));
        survive("mutated pdf", || vectorcraft_pdf::import(&b).ok())?;
    }
}

/// A dictionary value: one the reader reads, or one it skips over but can't parse (a name with a
/// broken `#` escape, a number with a stray `-`).
fn arb_dict_value() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["/Fine", "/#zz", "12-.5"])
}

/// A page's resources: `space` in the color space dictionary before a Separation space, `font`
/// in the font dictionary before a font, and a calculator function (`function`: plain, with
/// decode parameters the reader can't apply, or decoding past the cap) written as the shading's
/// function, or inside an array or a dictionary under `/Properties` (`held`). Returns the
/// resources and whether a file with them is refused.
fn unreadable_resources(space: &str, font: &str, function: &str, held: &str) -> (String, bool) {
    let program = deflated(b"{ dup 1 exch sub 0 }");
    let (function, refused) = match function {
        "Columns 0" => (calculator(&program, "/DecodeParms [null << /Predictor 2 /Columns 0 >>]"), true),
        "a huge Columns" => (calculator(&program, "/DecodeParms [null << /Predictor 12 /Colors 4 /Columns 99999999999 >>]"), true),
        "spaces past the cap" => (spaces_past_the_cap_once().to_string(), true),
        _ => (calculator(&program, ""), false),
    };
    let ramp = "<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>";
    let (shaded, other) = match held {
        "in an array" => (ramp, format!("/Properties << /P0 [{function}] >>")),
        "in a dictionary" => (ramp, format!("/Properties << /P0 << /K {function} >> >>")),
        _ => (function.as_str(), String::new()),
    };
    let tint = "<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0 1 0] /N 1 >>";
    let resources = format!(
        "<< /ColorSpace << /A {space} /CS0 [/Separation /Gold /DeviceCMYK {tint}] >> \
         /Font << /A {font} /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> \
         /Shading << /Sh0 << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function {shaded} >> >> {other} >>"
    );
    (resources, refused)
}

proptest! {
    #![proptest_config(config())]

    /// A page that paints with each of [`unreadable_resources`] is imported without a panic. The
    /// file opens with its art, or is refused when its calculator function decodes past the cap or
    /// has decode parameters the reader can't apply.
    #[test]
    fn pdf_resources_the_reader_cannot_parse_never_panic(
        space in arb_dict_value(),
        font in arb_dict_value(),
        function in prop::sample::select(vec!["plain", "Columns 0", "a huge Columns", "spaces past the cap"]),
        held in prop::sample::select(vec!["as the shading's function", "in an array", "in a dictionary"]),
    ) {
        let (resources, refused) = unreadable_resources(space, font, function, held);
        let pdf = handmade_pdf("/CS0 cs 1 scn 0 0 50 50 re f BT /F1 12 Tf 10 60 Td (Hi) Tj ET /Sh0 sh", &resources, "[0 0 100 100]");
        let what = format!("{space} and {font}, a function ({function}) {held}");
        let mut read = None;
        survive(&what, || {
            let r = vectorcraft_pdf::import(&pdf);
            read = Some(r.as_ref().map(|d| d.layers.iter().any(|l| l.children().is_some_and(|c| !c.is_empty()))).map_err(ToString::to_string));
            r.ok()
        })?;
        match read.expect("the import ran") {
            Ok(art) => prop_assert!(!refused && art, "{what}: opened (its art read: {art})"),
            Err(e) => prop_assert!(refused && e.contains("would decode to more than 256 MB"), "{what}: {e}"),
        }
    }
}

/// A page box with junk in it: inverted, empty, huge, NaN or outside the media box.
fn arb_box() -> impl Strategy<Value = Option<[f64; 4]>> {
    prop_oneof![
        Just(None),
        Just(Some([10.0, 10.0, 90.0, 90.0])),
        Just(Some([90.0, 90.0, 10.0, 10.0])),
        Just(Some([0.0, 0.0, 0.0, 0.0])),
        Just(Some([-1e308, -1e308, 1e308, 1e308])),
        Just(Some([f64::NAN, 0.0, 50.0, f64::INFINITY])),
        Just(Some([200.0, 200.0, 300.0, 300.0])),
    ]
}

proptest! {
    #![proptest_config(config())]

    /// Every page box, rotation, `UserUnit`, page pick, password and Crop To, through
    /// `document.open` and `document.pdfInfo` (with a thumbnail).
    #[test]
    fn pdf_import_options_never_panic(
        boxes in prop::collection::vec(arb_box(), 5),
        rotate in prop::sample::select(vec![0, 90, 180, 270, 45, -90, 1_000_000_000]),
        unit in prop::sample::select(vec!["", "10", "2.5", "1", "0", "-10", "0.001", "99999999999999999999", "/Ten", "(10)", "99 0 R", "huge"]),
        encrypt in any::<bool>(),
        password in prop::sample::select(vec!["", "pw", "wrong", "\u{0}\u{ff}"]),
        pages in prop::sample::select(vec!["1", "2", "1-2", "2-1", "0", "-", "1-", "all", "1,1,2", "99999999999999999999"]),
        crop in prop::sample::select(vec!["bounding", "art", "crop", "trim", "bleed", "media", "nope"]),
    ) {
        use vectorcraft_testkit::pdf::{PdfPage, pdf};
        // A `UserUnit` of 1 followed by 308 zeros (a PDF number has no exponent).
        let unit = if unit == "huge" { format!("1{}", "0".repeat(308)) } else { unit.to_string() };
        let page = PdfPage {
            media: boxes[0].unwrap_or([0.0, 0.0, 100.0, 100.0]),
            crop: boxes[1],
            bleed: boxes[2],
            trim: boxes[3],
            art: boxes[4],
            rotate,
            entries: if unit.is_empty() { String::new() } else { format!("/UserUnit {unit}") },
            ..PdfPage::new(0.0, 0.0, "0 0 1 rg 20 20 50 50 re f")
        };
        let bytes = pdf(&[page.clone(), page], encrypt.then_some("pw"));
        let b64 = vectorcraft_format::base64_encode(&bytes);
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            let p = json!({"name": "x.pdf", "dataBase64": b64, "pages": pages, "cropTo": crop, "password": password, "thumbnail": 1, "thumbnailSize": 64});
            let _ = no_panic(s.execute("document.pdfInfo", &p));
            if no_panic(s.execute("document.open", &p)).is_ok() {
                let d = (*s.doc().unwrap().doc).clone();
                survive("pdf options", || Some(d)).unwrap();
            }
        });
        r.map_err(|msg| TestCaseError::fail(format!("{pages} {crop} {rotate} {unit}: panicked: {msg}")))?;
    }
}

/// An XMP packet with every property File Info reads: arrays, language alternatives, attributes,
/// a qualified value, a URI value and dates.
const RICH_XMP: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
    <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" \
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
    xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" \
    xmlns:q=\"urn:example:qualifiers\" xmpRights:Marked=\"True\" xmp:ModifyDate=\"2026-10-06T09:00:00.5+05:30\">\
    <dc:creator><rdf:Seq><rdf:li rdf:parseType=\"Resource\"><rdf:value>Ada</rdf:value><q:role>Author</q:role></rdf:li><rdf:li>Grace</rdf:li></rdf:Seq></dc:creator>\
    <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">A poster</rdf:li><rdf:li xml:lang=\"de\">Ein Plakat</rdf:li></rdf:Alt></dc:description>\
    <dc:subject><rdf:Bag><rdf:li>fair</rdf:li><rdf:li>print</rdf:li></rdf:Bag></dc:subject><pdf:Keywords>a, b; c</pdf:Keywords>\
    <dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">(c) 2026 Ada</rdf:li></rdf:Alt></dc:rights>\
    <xmpRights:WebStatement rdf:resource=\"https://example.com/rights\"/><xmp:CreateDate>2026-10-05T14:03:00Z</xmp:CreateDate>\
    </rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>";

/// A PDF whose trailer names an information dictionary with the entries `info` and whose catalog's
/// metadata stream holds `xmp`.
fn file_info_pdf(info: &str, xmp: &str) -> Vec<u8> {
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with_catalog};
    let x = first_extra(1);
    let stream = format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{xmp}\nendstream", xmp.len());
    let dict = format!("<< {info} >>");
    let pdf = pdf_with_catalog(&[PdfPage::new(100.0, 100.0, "0 0 1 rg 10 10 50 50 re f")], &[&stream, &dict], &format!("/Metadata {x} 0 R "), None);
    // The trailer follows the cross-reference table, so a longer trailer moves no object.
    String::from_utf8(pdf).unwrap().replacen("/Root 1 0 R", &format!("/Root 1 0 R /Info {} 0 R", x + 1), 1).into_bytes()
}

/// The sample `pdf_file_info_never_panics` mutates opens with the File Info of its packet.
#[test]
fn the_file_info_sample_reaches_the_reader() {
    let pdf = file_info_pdf("/Author (Info author) /Subject (Info subject)", RICH_XMP);
    let mut s = vectorcraft_engine::Session::new();
    s.execute("document.open", &json!({"name": "x.ai", "dataBase64": vectorcraft_format::base64_encode(&pdf)})).unwrap();
    let info = s.execute("file.info", &json!({})).unwrap();
    assert_eq!((info["author"].as_str(), info["description"].as_str()), (Some("Ada, Grace"), Some("A poster")));
    assert_eq!((info["copyrightStatus"].as_str(), info["keywords"].clone()), (Some("copyrighted"), json!(["fair", "print"])));
    assert_eq!(info["copyrightUrl"].as_str(), Some("https://example.com/rights"));
}

proptest! {
    #![proptest_config(config())]

    /// File Info read on open: mutated XMP packets, some with elements nested deeply, and junk
    /// information dictionaries, opened as a PDF and as a `.ai`.
    #[test]
    fn pdf_file_info_never_panics(
        cut in 0usize..2_000,
        edits in prop::collection::vec((0usize..2_000, prop::sample::select(vec!['<', '>', '"', '/', ':', '-', '+', 'T', 'Z', '9', '0', '.', ' ', '&', ';', '=', '#'])), 0..10),
        info in prop::sample::select(vec![
            "",
            "/Author (Ada) /Subject (A poster) /Keywords (a;b,,c ;) /CreationDate (D:2026) /ModDate (D:20261005140300-08'30')",
            "/CreationDate (D:99999999999999+99'99') /ModDate (D:20261301) /Author 5 /Keywords [(a)]",
            "/Subject <FEFF00> /Author <FEFFD800> /Keywords <EFBBBF> /Title (\\377\\376)",
            "/CreationDate (D:00000101000000Z) /ModDate (D:99991231235959+23'59')",
        ]),
        nest in prop::sample::select(vec![0usize, 0, 0, 251, 100_000]),
    ) {
        // `nest` elements nested in one another around the packet.
        let xmp = format!("{}{}{}", "<a>".repeat(nest), mutate_text(RICH_XMP, cut, &edits), "</a>".repeat(nest));
        let pdf = file_info_pdf(info, &xmp);
        let b64 = vectorcraft_format::base64_encode(&pdf);
        let r = catch_quiet(|| {
            let _ = vectorcraft_pdf::file_info(&pdf, None);
            let mut s = vectorcraft_engine::Session::new();
            for name in ["x.pdf", "x.ai"] {
                let _ = no_panic(s.execute("document.open", &json!({"name": name, "dataBase64": b64})));
                let _ = no_panic(s.execute("file.info", &json!({})));
            }
        });
        r.map_err(|msg| TestCaseError::fail(format!("{info}: panicked: {msg}")))?;
    }
}

/// Hostile content over layers (optional content, one group off), soft masks, tiling patterns,
/// a patch mesh, an unextended shading and fonts: the import's own readings of the file.
fn rich_resources_pdf(content: &str, config: &str) -> Vec<u8> {
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with_catalog};
    let x = first_extra(1);
    let stream = |dict: &str, data: &str| format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len());
    let extra = [
        "<< /Type /OCG /Name (A) >>".to_string(),
        "<< /Type /OCG /Name <FEFF> /Usage << /Print << /PrintState /OFF >> >> >>".to_string(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceRGB /I true /K true >>",
            "1 g 0 0 50 50 re f /OC /MC0 BDC 0 g 0 0 5 5 re f EMC",
        ),
        stream(
            "/PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 1e-9 10] /XStep 0.001 /YStep 10 /Resources << >>",
            "0 0 5 5 re f /P0 scn 0 0 1 1 re f",
        ),
        stream(
            "/ShadingType 6 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 255 0 255 0 1 0 1 0 1] /Filter /ASCIIHexDecode",
            "000A0A0A250A3F0A5A255A3F5A5A5A5A3F5A255A0A3F0A250AFF000000FF000000FFFFFFFF>",
        ),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /OC 6 0 R", "/X0 Do"),
    ];
    let (a, b, form, pat, mesh, hidden) = (x, x + 1, x + 2, x + 3, x + 4, x + 5);
    let resources = format!(
        "/Properties << /MC0 {a} 0 R /MC1 {b} 0 R >> /XObject << /X0 {form} 0 R /X1 {hidden} 0 R >> /Pattern << /P0 {pat} 0 R >> /Shading << /Sh0 {mesh} 0 R /Sh1 << /ShadingType 3 /ColorSpace /DeviceGray /Coords [50 50 0 50 50 40] /Extend [false false] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >> >> /ExtGState << /GS0 << /OP true /OPM 1 /SMask << /Type /Mask /S /Luminosity /G {form} 0 R /BC [1] /TR << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> >> >> /GS1 << /OP true /op false /SMask << /Type /Mask /S /Alpha /G {form} 0 R >> >> /GS2 << /OP 7 /op (x) /OPM -3 >> /GS3 << /op true >> >> /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+Odd-Name,Bold >> >> /ColorSpace << /CS0 [/Pattern /DeviceRGB] /CS1 [/Separation /A /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>] /CS2 [/Separation /B /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >>] >>"
    );
    let catalog = format!("/OCProperties << /OCGs [{a} 0 R {b} 0 R 99 0 R] /D << {} >> >> ", config.replace("{B}", &format!("{b} 0 R")));
    let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
    pdf_with_catalog(&[PdfPage { resources, ..PdfPage::new(100.0, 100.0, content) }], &extra, &catalog, None)
}

fn arb_rich_op() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_pdf_op(),
        prop::sample::select(vec![
            "/OC /MC0 BDC",
            "/OC /MC1 BDC",
            "/OC /Nope BDC",
            "/Span << /MCID 3 >> BDC",
            "/X BMC",
            "EMC",
            "/X0 Do",
            "/X1 Do",
            "/GS0 gs",
            "/GS1 gs",
            // Overprint (#1388): set, malformed, and for fills alone.
            "/GS2 gs",
            "/GS3 gs",
            "/CS0 cs 1 0 0 /P0 scn",
            "/Pattern cs /P0 scn",
            // Inks with the same values and other names: set, and a pattern set in one.
            "/CS1 cs 1 scn",
            "/CS2 cs 0.5 scn",
            "/CS2 CS 1 SCN",
            "/CS1 cs /P0 scn",
            "/Sh0 sh",
            "/Sh1 sh",
            "BT /F1 12 Tf 10 10 Td (Hi there) Tj [(a) -900 (b)] TJ ET",
            // Lines wrapped in a frame, digits set apart by gaps, glyph outlines stroked (#508).
            "BT /F1 10 Tf 10 80 Td (Wrapped line ) Tj 0 -12 Td (and its next ) Tj 0 -12 Td (end.) Tj ET",
            "BT /F1 10 Tf 10 40 Td (3) Tj 20 0 Td (5331) Tj 40 0 Td (0106) Tj ET",
            "0.5 w 10 10 m 14 18 l 18 10 l h S",
            "7 Tr",
            "q 10 10 50 50 re W n",
            "Q",
            "0 0 100 100 re f",
            "10 10 20 20 re B",
        ])
        .prop_map(String::from),
    ]
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn pdf_layers_masks_patterns_and_text_never_panic(
        ops in prop::collection::vec(arb_rich_op(), 0..40),
        config in prop::sample::select(vec!["/OFF [{B}]", "/BaseState /OFF", "/BaseState /Unchanged /ON [{B}] /Locked [{B}]", "", "/OFF {B}"]),
        layers in any::<bool>(),
        outlines in any::<bool>(),
    ) {
        let bytes = rich_resources_pdf(&ops.join("\n"), config);
        let opts = vectorcraft_pdf::ImportOptions {
            layers,
            text_as: if outlines { vectorcraft_pdf::TextAs::Outlines } else { vectorcraft_pdf::TextAs::Text },
            ..Default::default()
        };
        survive(&ops.join(" "), || vectorcraft_pdf::import_with_report(&bytes, &opts).ok().map(|r| r.document))?;
    }
}

// ---------- library files ----------

/// The `data` of a library file `cmd` writes for the rich document.
fn saved(cmd: &str, params: Value) -> String {
    rich_session().execute(cmd, &params).unwrap()["data"].as_str().unwrap().to_string()
}

/// `r`, after failing the case if the command panicked. The engine's guard reports a panic as
/// `EngineError::Internal`.
fn no_panic(r: Result<Value, vectorcraft_engine::EngineError>) -> Result<Value, vectorcraft_engine::EngineError> {
    if let Err(vectorcraft_engine::EngineError::Internal { cmd, msg }) = &r {
        panic!("{cmd} panicked: {msg}");
    }
    r
}

/// Library file `data` loaded with `load`; what it holds is then used on the rich document
/// (`then`, given the load's result), which renders and exports.
fn survive_library(what: &str, load: &str, data: &str, then: impl FnOnce(&mut vectorcraft_engine::Session, &Value)) -> Result<(), TestCaseError> {
    survive(what, || {
        let mut s = rich_session();
        let r = no_panic(s.execute(load, &json!({"data": data, "name": "fuzz"}))).ok()?;
        then(&mut s, &r);
        Some((*s.doc().ok()?.doc).clone())
    })
}

fn swatches(what: &str, data: &str) -> Result<(), TestCaseError> {
    survive_library(what, "swatch.library.load", data, |s, r| {
        let _ = no_panic(s.execute("swatch.library.add", &json!({"library": r["library"], "apply": "fill"})));
    })
}

fn styles(what: &str, data: &str) -> Result<(), TestCaseError> {
    survive_library(what, "graphicStyle.loadLibrary", data, |s, r| {
        let _ = no_panic(s.execute("select.all", &json!({})));
        let _ = no_panic(s.execute("graphicStyle.addFromLibrary", &json!({"library": r["library"], "apply": true})));
    })
}

/// Libraries panel library file `data` imported, then each kind's first item used on the rich
/// document, moved into its first group and next to its kind's last item, the group moved and
/// deleted and the library exported (#926).
fn libraries(what: &str, data: &str) -> Result<(), TestCaseError> {
    survive_library(what, "library.import", data, |s, r| {
        let lib = r["id"].clone();
        let Ok(got) = no_panic(s.execute("library.get", &json!({"library": lib}))) else { return };
        let _ = no_panic(s.execute("select.all", &json!({})));
        for (list, kind, key) in
            [("graphics", "graphic", "id"), ("colors", "fillColor", "name"), ("charStyles", "charStyle", "name"), ("paraStyles", "paraStyle", "name")]
        {
            if let Some(item) = got[list].get(0) {
                let _ = no_panic(s.execute("library.use", &json!({"library": lib, "kind": kind, "item": item[key]})));
                let to = json!({"library": lib, "kind": kind, "item": item[key], "group": got["groups"][0]["name"]});
                let _ = no_panic(s.execute("library.moveItem", &to));
                // Next to the last item of its kind, whatever group that is in.
                let last = got[list].as_array().and_then(|a| a.last()).map(|v| v[key].clone()).unwrap_or_default();
                for side in ["before", "after"] {
                    let _ = no_panic(s.execute("library.moveItem", &json!({"library": lib, "kind": kind, "item": item[key], side: last})));
                }
            }
        }
        if let Some(g) = got["groups"].get(0) {
            for to in [0, 1, 1_000_000] {
                let _ = no_panic(s.execute("library.moveGroup", &json!({"library": lib, "group": g["name"], "to": to})));
            }
            let _ = no_panic(s.execute("library.deleteGroup", &json!({"library": lib, "group": g["name"]})));
        }
        let _ = no_panic(s.execute("library.export", &json!({"library": lib})));
    })
}

/// A Libraries panel library of the rich document's first object (a graphic), a colour, its
/// type's character and paragraph styles and a group, exported, made once.
fn saved_library() -> String {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let mut s = rich_session();
        s.execute("library.create", &json!({"name": "Fuzz"})).unwrap();
        let first = s.doc().unwrap().doc.layers[0].children().unwrap()[0].id.0;
        let g = s.execute("library.add", &json!({"kind": "graphic", "ids": [first]})).unwrap()["id"].clone();
        s.execute("library.add", &json!({"kind": "fillColor", "color": "#336699"})).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        for kind in ["charStyle", "paraStyle"] {
            s.execute("library.add", &json!({"kind": kind})).unwrap();
        }
        s.execute(
            "library.createGroup",
            &json!({"name": "Logos", "items": [{"kind": "graphic", "item": g}, {"kind": "fillColor", "item": "#336699"}]}),
        )
        .unwrap();
        s.execute("library.export", &json!({})).unwrap()["data"].as_str().unwrap().to_string()
    })
    .clone()
}

fn flattener_presets(what: &str, data: &str) -> Result<(), TestCaseError> {
    survive_library(what, "flattener.presets.import", data, |s, r| {
        let _ = no_panic(s.execute("select.all", &json!({})));
        if let Some(name) = r["imported"].get(0) {
            let _ = no_panic(s.execute("flattener.preview", &json!({"preset": name, "highlight": "allAffected"})));
            let _ = no_panic(s.execute("object.flattenTransparency", &json!({"preset": name, "lineArtPpi": 36, "gradientPpi": 36})));
        }
    })
}

/// Swatch exchange (`.ase`) bytes loaded as a swatch library, then added to the rich document.
fn swatch_exchange(what: &str, bytes: &[u8]) -> Result<(), TestCaseError> {
    survive(what, || {
        let mut s = rich_session();
        let load = json!({"dataBase64": vectorcraft_format::base64_encode(bytes), "name": "fuzz.ase"});
        let r = no_panic(s.execute("swatch.library.load", &load)).ok()?;
        let _ = no_panic(s.execute("swatch.library.add", &json!({"library": r["library"], "apply": "fill"})));
        Some((*s.doc().ok()?.doc).clone())
    })
}

/// The rich document's swatches with a CMYK spot color and a global Lab color added, saved as a
/// swatch exchange file, made once.
fn saved_ase() -> Vec<u8> {
    static ASE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    ASE.get_or_init(|| {
        let mut s = rich_session();
        s.execute("swatch.new", &json!({"name": "Ink", "color": {"c": 1, "m": 0.5, "y": 0, "k": 0.2}, "spot": true})).unwrap();
        s.execute("swatch.new", &json!({"name": "Clay", "color": {"l": 50, "a": 20, "b": -30}, "global": true})).unwrap();
        let r = s.execute("swatch.library.save", &json!({"format": "ase"})).unwrap();
        vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
    })
    .clone()
}

/// A random swatch exchange block (type, body). The body holds a name, a color model when the block
/// is a color, and random bytes for the rest.
fn arb_ase_block() -> impl Strategy<Value = (u16, Vec<u8>)> {
    let kind = prop::sample::select(vec![0xC001u16, 0xC002, 0x0001, 0x0042]);
    let model = prop::sample::select(vec![*b"RGB ", *b"CMYK", *b"LAB ", *b"Gray", *b"HSB "]);
    (kind, "[ -~]{0,8}", model, prop::collection::vec(any::<u8>(), 0..24)).prop_map(|(kind, name, model, rest)| {
        let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut body = (units.len() as u16).to_be_bytes().to_vec();
        body.extend(units.iter().flat_map(|u| u.to_be_bytes()));
        if kind == 0x0001 {
            body.extend(model);
        }
        body.extend(rest);
        (kind, body)
    })
}

/// Characters that break JSON and GPL palettes.
fn arb_edit() -> impl Strategy<Value = (usize, char)> {
    (0usize..20_000, prop::sample::select(vec!['{', '}', '[', ']', '"', ':', ',', '-', '9', 'e', '.', ' ', '\n', '#', 'n', 'x', '\t']))
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn library_garbage_never_panics(s in ".{0,300}", head in prop::sample::select(vec!["", "GIMP Palette\n","{\"format\": \"vcswatches\", ", "{\"format\": \"vcstyles\", ", "{\"format\": \"vcflattener\", ", "{\"name\": \"L\", "])) {
        let data = format!("{head}{s}");
        swatches("swatch library garbage", &data)?;
        styles("style library garbage", &data)?;
        flattener_presets("flattener preset garbage", &data)?;
        libraries("library garbage", &data)?;
    }

    #[test]
    fn mutated_swatch_libraries_never_panic(cut in 0usize..20_000, edits in prop::collection::vec(arb_edit(), 0..10), gpl in any::<bool>()) {
        let text = saved("swatch.library.save", json!({"format": if gpl { "gpl" } else { "vcswatches" }}));
        swatches("mutated swatch library", &mutate_text(&text, cut, &edits))?;
    }

    #[test]
    fn mutated_style_libraries_never_panic(cut in 0usize..20_000, edits in prop::collection::vec(arb_edit(), 0..10)) {
        let text = saved("graphicStyle.saveLibrary", json!({}));
        styles("mutated style library", &mutate_text(&text, cut, &edits))?;
    }

    #[test]
    fn mutated_flattener_presets_never_panic(cut in 0usize..5_000, edits in prop::collection::vec(arb_edit(), 0..10)) {
        let text = saved("flattener.presets.export", json!({"names": ["high", "medium", "low"]}));
        flattener_presets("mutated flattener presets", &mutate_text(&text, cut, &edits))?;
    }

    #[test]
    fn mutated_libraries_never_panic(keep in 0.0f64..1.2, edits in prop::collection::vec(arb_edit(), 0..10)) {
        // Cut anywhere in the file (whole most of the time), the edits anywhere in it.
        let text = saved_library();
        let cut = (text.chars().count() as f64 * keep) as usize;
        libraries("mutated library", &mutate_text(&text, cut, &edits))?;
    }

    #[test]
    fn swatch_exchange_garbage_never_panics(blocks in prop::collection::vec(arb_ase_block(), 0..12), tail in prop::collection::vec(any::<u8>(), 0..16)) {
        let mut bytes = b"ASEF\0\x01\0\0".to_vec();
        bytes.extend((blocks.len() as u32).to_be_bytes());
        for (kind, body) in &blocks {
            bytes.extend(kind.to_be_bytes());
            bytes.extend((body.len() as u32).to_be_bytes());
            bytes.extend(body);
        }
        bytes.extend(tail);
        swatch_exchange("swatch exchange garbage", &bytes)?;
    }

    #[test]
    fn mutated_swatch_exchange_files_never_panic(cut in prop::option::of(0usize..200), edits in prop::collection::vec((0usize..200, any::<u8>()), 0..12)) {
        let mut bytes = vectorcraft_testkit::ase::sample();
        for (at, b) in edits {
            let n = bytes.len();
            bytes[at % n] = b;
        }
        if let Some(cut) = cut {
            bytes.truncate(cut);
        }
        swatch_exchange("mutated swatch exchange file", &bytes)?;
    }

    #[test]
    fn mutated_saved_swatch_exchange_files_never_panic(cut in prop::option::of(0usize..4_000), edits in prop::collection::vec((0usize..4_000, any::<u8>()), 0..12)) {
        let mut bytes = saved_ase();
        for (at, b) in edits {
            let n = bytes.len();
            bytes[at % n] = b;
        }
        if let Some(cut) = cut {
            bytes.truncate(cut);
        }
        swatch_exchange("mutated saved swatch exchange file", &bytes)?;
    }
}

// ---------- data set CSV files ----------

/// A document with a text, a visibility and a graph variable, each bound to an object, and two
/// data sets.
fn merge_session() -> vectorcraft_engine::Session {
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = |r: Value| r["id"].as_u64().unwrap();
    let t = id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": "One"})).unwrap());
    let r = id(s.execute("shape.rectangle", &json!({"x": 10, "y": 40, "width": 50, "height": 40})).unwrap());
    let graph = json!({"type": "column", "x": 80, "y": 10, "width": 100, "height": 80, "csv": ",Sales\nA,1\nB,2"});
    let g = id(s.execute("graph.create", &graph).unwrap());
    for (name, kind, object) in [("Title", "text", t), ("Show", "visibility", r), ("Chart", "graph", g)] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
        s.execute("variable.bind", &json!({"variable": name, "ids": [object]})).unwrap();
    }
    s.execute("dataset.new", &json!({"name": "A", "values": {"Title": "Two, \"quoted\"\nlines", "Show": true, "Chart": ",Q\nX,3"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Title": " padded ", "Show": false}})).unwrap();
    s
}

/// [`merge_session`]'s data sets exported as CSV, made once.
fn saved_datasets() -> String {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| merge_session().execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string()).clone()
}

/// `dataset.import` with `params` on [`merge_session`]'s document, the first three data sets it
/// read selected in turn and every data set exported again; the document then renders and exports.
fn datasets(what: &str, params: Value) -> Result<(), TestCaseError> {
    survive(what, || {
        let mut s = merge_session();
        if let Ok(r) = no_panic(s.execute("dataset.import", &params)) {
            for name in r["names"].as_array().into_iter().flatten().take(3) {
                let _ = no_panic(s.execute("dataset.select", &json!({"name": name})));
            }
        }
        let _ = no_panic(s.execute("dataset.export", &json!({})));
        Some((*s.doc().ok()?.doc).clone())
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn dataset_csv_garbage_never_panics(s in ".{0,300}", head in prop::sample::select(vec!["", "name,Title,Show,Chart\n", "Title\tShow\tChart\n", "name\tTitle\n", "Show,Chart\n", "name,#Show,@Photo,New\n", "#Shown\t@Title\tChart\n", "name,%Chart,#\n"])) {
        datasets("data set CSV garbage", json!({"csv": format!("{head}{s}")}))?;
    }

    #[test]
    fn prefixed_dataset_headers_never_panic(
        cells in prop::collection::vec((prop::sample::select(vec!["", "#", "@", "%", "#@", " # "]), "[A-Za-z #@%\"]{0,8}"), 1..8),
        tabs in any::<bool>(),
        body in ".{0,200}",
    ) {
        let sep = if tabs { "\t" } else { "," };
        let header: Vec<String> = cells.iter().map(|(prefix, name)| format!("{prefix}{name}")).collect();
        datasets("prefixed data set CSV header", json!({"csv": format!("{}\n{body}", header.join(sep))}))?;
    }

    #[test]
    fn dataset_csv_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        datasets("data set CSV bytes", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes)}))?;
    }

    #[test]
    fn mutated_dataset_csvs_never_panic(cut in 0usize..200, edits in prop::collection::vec(arb_edit(), 0..10), tabs in any::<bool>()) {
        let text = saved_datasets();
        let text = if tabs { text.replace(',', "\t") } else { text };
        datasets("mutated data set CSV", json!({"csv": mutate_text(&text, cut, &edits)}))?;
    }
}

// ---------- variable library XML files ----------

/// A variable library written from its public grammar: two text variables (`Title` is a
/// variable of [`merge_session`]'s document), a variable of another kind and two data sets.
const VARIABLE_LIBRARY: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<!DOCTYPE svg [\n<!ENTITY ns_vars \"http://ns.adobe.com/Variables/1.0/\">\n<!ENTITY ns_custom \"http://ns.adobe.com/GenericCustomNamespace/1.0/\">\n]>\n\
<svg>\n<variableSets xmlns=\"&ns_vars;\">\n<variableSet varSetName=\"binding1\" locked=\"none\">\n<variables>\n\
<variable varName=\"Title\" trait=\"textcontent\"></variable>\n<variable varName=\"Note\" trait=\"textcontent\"></variable>\n\
<variable varName=\"Show\" trait=\"other\"></variable>\n</variables>\n\
<v:sampleDataSets xmlns:v=\"&ns_vars;\" xmlns=\"&ns_custom;\">\n\
<v:sampleDataSet dataSetName=\"A\">\n<Title><p>One</p><p>Two &amp; three</p></Title>\n<Note><p>x</p></Note>\n<Show><p>true</p></Show>\n</v:sampleDataSet>\n\
<v:sampleDataSet dataSetName=\"C\">\n<Title><p>Four</p></Title>\n<Note></Note>\n</v:sampleDataSet>\n\
</v:sampleDataSets>\n</variableSet>\n</variableSets>\n</svg>\n";

/// `variable.loadLibrary` with `params` on [`merge_session`]'s document, the first three data sets
/// it read selected in turn and every data set exported; the document then renders and exports.
fn variable_library(what: &str, params: Value) -> Result<(), TestCaseError> {
    survive(what, || {
        let mut s = merge_session();
        if let Ok(r) = no_panic(s.execute("variable.loadLibrary", &params)) {
            for name in r["names"].as_array().into_iter().flatten().take(3) {
                let _ = no_panic(s.execute("dataset.select", &json!({"name": name})));
            }
        }
        let _ = no_panic(s.execute("dataset.export", &json!({})));
        Some((*s.doc().ok()?.doc).clone())
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn variable_library_garbage_never_panics(s in ".{0,300}", head in prop::sample::select(vec![
        "",
        "<variableSets>",
        "<variableSets><variableSet><variables><variable varName=\"Title\" trait=\"textcontent\"/></variables><sampleDataSets><sampleDataSet dataSetName=\"A\">",
        "<variableSets><variableSet><variables><variable varName=\"Title\" trait=\"textcontent\"/></variables><sampleDataSets><sampleDataSet dataSetName=\"A\"><Title><p>",
        "<!DOCTYPE svg [<!ENTITY a \"",
    ])) {
        variable_library("variable library garbage", json!({"xml": format!("{head}{s}")}))?;
    }

    #[test]
    fn variable_library_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        variable_library("variable library bytes", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes)}))?;
    }

    /// The library cut short and with characters replaced, then with `nest` elements nested in one
    /// another inside its `svg` element.
    #[test]
    fn mutated_variable_libraries_never_panic(
        cut in 0usize..1_200,
        edits in prop::collection::vec((0usize..1_200, prop::sample::select(vec!['<', '>', '"', '/', '=', '&', ';', '!', '[', ']', 'p', 'x', ' '])), 0..10),
        nest in prop::sample::select(vec![0usize, 0, 0, 240, 100_000]),
    ) {
        let xml = mutate_text(VARIABLE_LIBRARY, cut, &edits);
        let xml = xml.replacen("<svg>", &format!("<svg>{}", "<a>".repeat(nest)), 1).replacen("</svg>", &format!("{}</svg>", "</a>".repeat(nest)), 1);
        variable_library("mutated variable library", json!({"xml": xml}))?;
    }
}

// ---------- shortcut sets ----------

/// Shortcut set `bytes` read with `shortcutSet.read`, with import matches saved to annotate it: an
/// error, or a set within the caps.
fn shortcut_set(bytes: &[u8]) -> Result<(), TestCaseError> {
    let mut s = vectorcraft_engine::Session::new();
    let matches = json!([
        {"kind": "name", "name": "A", "target": "edit.undo"},
        {"kind": "key", "name": "1", "target": "K"},
        {"kind": "modifiers", "name": "-1", "target": "Cmd"},
    ]);
    s.execute("importMatches.set", &json!({ "matches": matches })).unwrap();
    let read = json!({"dataBase64": vectorcraft_format::base64_encode(bytes), "name": "fuzz.kys"});
    if let Ok(r) = no_panic(s.execute("shortcutSet.read", &read)) {
        prop_assert!(r["entries"].as_array().map_or(0, Vec::len) <= 20_000);
        prop_assert!(r["unread"].as_array().map_or(0, Vec::len) <= 1000);
    }
    Ok(())
}

/// A token of a text shortcut set: a name (escaped spaces), a field, an integer of any size and sign,
/// a brace, a lone backslash, a line end, NUL or bytes that aren't UTF-8.
fn arb_kys_token() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        "[A-Za-z ]{0,12}".prop_map(|n| format!("/{}", n.replace(' ', "\\ ")).into_bytes()),
        prop::sample::select(vec!["/Key", "/Modifiers", "/Context", "/Represent"]).prop_map(|f| f.as_bytes().to_vec()),
        any::<i64>().prop_map(|n| n.to_string().into_bytes()),
        Just(b"99999999999999999999999".to_vec()),
        prop::sample::select(vec!["{", "}", "\\", "\r\n", "\r", "\n"]).prop_map(|t| t.as_bytes().to_vec()),
        prop::sample::select(vec![vec![0u8], vec![0xFF, 0xFE], vec![0xC3]]),
    ]
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn shortcut_set_garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..600), head in any::<bool>()) {
        let mut data = if head { b"/Menus {".to_vec() } else { vec![] };
        data.extend(bytes);
        shortcut_set(&data)?;
    }

    #[test]
    fn hostile_shortcut_sets_never_panic(tokens in prop::collection::vec(arb_kys_token(), 0..200), sep in prop::sample::select(vec![" ", "", "\t"])) {
        let mut data = b"/Menus {".to_vec();
        for t in tokens {
            data.extend(sep.as_bytes());
            data.extend(t);
        }
        shortcut_set(&data)?;
    }
}

/// A shortcut set past any cap (nesting, entries, name length, file size) is read up to it and
/// marked truncated.
#[test]
fn shortcut_sets_past_the_caps_are_truncated() {
    let read = |text: String| vectorcraft_engine::Session::new().execute("shortcutSet.read", &json!({ "data": text })).unwrap();
    let deep = format!("/Menus {{ {} /K {{ /Key 1 }} }}", "/A { ".repeat(40));
    let many = format!("/Menus {{ {} }}", "/E { /Key 1 } ".repeat(20_001));
    let long = format!("/Menus {{ /{} {{ /Key 1 }} }}", "n".repeat(2000));
    let big = format!("/Menus {{ /E {{ /Key 1 }} {} }}", " ".repeat(4 << 20));
    for text in [deep, many, long, big] {
        assert_eq!(read(text)["truncated"], true);
    }
}

/// A small image of each raster format Place reads, with resolution metadata where it has some.
fn raster_samples() -> Vec<(&'static str, Vec<u8>)> {
    let img = image::RgbImage::from_pixel(5, 3, image::Rgb([200, 40, 10]));
    let mut out = vec![];
    for (name, f) in [
        ("a.png", image::ImageFormat::Png),
        ("a.jpg", image::ImageFormat::Jpeg),
        ("a.tif", image::ImageFormat::Tiff),
        ("a.bmp", image::ImageFormat::Bmp),
    ] {
        let mut b = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut b), f).unwrap();
        out.push((name, b));
    }
    out[0].1 = vectorcraft_engine::cmd::fileio::ppi::with_png_resolution(&out[0].1, (300.0, 150.0));
    // TIFFs that store one plane per channel: CMYK with a channel after the inks, and gray with
    // alpha, 16 bits a sample.
    let plane = |v: u16| vec![v; 15];
    let cmyk = [plane(200), plane(40), plane(10), plane(0), plane(90)];
    let cmyk =
        vectorcraft_testkit::tiff::Tiff { width: 5, height: 3, photometric: 5, bits: 8, extra: &[0], planar: true, ppi: Some(300), channels: &cmyk };
    out.push(("planar-cmyk.tif", vectorcraft_testkit::tiff::tiff(&cmyk)));
    let gray = [plane(30_000), plane(65_535)];
    let gray =
        vectorcraft_testkit::tiff::Tiff { width: 5, height: 3, photometric: 1, bits: 16, extra: &[2], planar: true, ppi: None, channels: &gray };
    out.push(("planar-gray.tif", vectorcraft_testkit::tiff::tiff(&gray)));
    // A layered PSD with transparency (as VectorCraft exports it), and a PackBits PSB by hand.
    let mut s = rich_session();
    let r = s.execute("document.export", &json!({"format": "psd", "ppi": 2})).unwrap();
    out.push(("a.psd", vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()));
    let mut psb = b"8BPS\0\x02\0\0\0\0\0\0\0\x04\0\0\0\x02\0\0\0\x03\0\x08\0\x03".to_vec();
    psb.extend([0; 8]);
    psb.extend(10u64.to_be_bytes());
    psb.extend(2u64.to_be_bytes());
    psb.extend((-1i16).to_be_bytes());
    psb.extend([0, 1]);
    psb.extend((0..8).flat_map(|_| 2u32.to_be_bytes()));
    psb.extend((0..8u8).flat_map(|v| [0xfe, v * 30]));
    out.push(("a.psb", psb));
    out
}

proptest! {
    #![proptest_config(config())]

    /// Mutated image headers (resolution metadata, chunk and segment lengths) never crash reading
    /// their resolution or placing them.
    #[test]
    fn mutated_images_place_without_panics(which in 0usize..8, cut in 0usize..4000, edits in prop::collection::vec((0usize..120, any::<u8>()), 0..12)) {
        let (name, mut bytes) = raster_samples().swap_remove(which);
        for (at, b) in edits {
            if let Some(x) = bytes.get_mut(at) {
                *x = b;
            }
        }
        bytes.truncate(cut.max(8));
        let r = catch_quiet(|| {
            let _ = vectorcraft_engine::cmd::fileio::ppi::resolution(&bytes);
            let _ = vectorcraft_engine::cmd::fileio::ppi::with_png_resolution(&bytes, (72.0, 72.0));
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
            let p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes), "thumbnail": 8});
            let _ = no_panic(s.execute("file.place.info", &p));
            let _ = no_panic(s.execute("file.place", &p));
        });
        prop_assert!(r.is_ok(), "{name}: panicked: {:?}", r.err());
    }
}

/// 1-bit TIFFs compressed with CCITT Group 3: modified Huffman (Compression 2) in three strips, T.4
/// (Compression 3) with two-dimensional rows, fill bits, six EOLs at the end and each byte's lowest
/// bit first, and T.4 two-dimensional rows in 16 × 16 tiles.
fn group3_samples() -> Vec<Vec<u8>> {
    let rows: Vec<Vec<bool>> = (0..6).map(|y| (0..40).map(|x| (x * (y + 1) / 3 + y) % 4 < 2).collect()).collect();
    let tiff = |compression, t4_options, fill_order, rows_per_strip, tile| {
        let rtc = compression == 3;
        let g = vectorcraft_testkit::tiff::Group3 { rows: &rows, compression, t4_options, photometric: 0, fill_order, rows_per_strip, tile, rtc };
        vectorcraft_testkit::tiff::group3(&g)
    };
    vec![tiff(2, 0, 1, 2, None), tiff(3, 5, 2, 6, None), tiff(3, 1, 1, 6, Some((16, 16)))]
}

/// [`group3_samples`] place as images.
#[test]
fn the_group3_samples_place() {
    for bytes in group3_samples() {
        let mut s = vectorcraft_engine::Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        s.execute("file.place", &json!({"name": "fax.tif", "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    }
}

proptest! {
    #![proptest_config(config())]

    /// Mutated Group 3 TIFFs (directory values, strip offsets and counts, the coded rows) never
    /// crash reading or placing them.
    #[test]
    fn mutated_group3_tiffs_place_without_panics(which in 0usize..3, cut in 0usize..400, edits in prop::collection::vec((0usize..400, any::<u8>()), 0..12)) {
        let mut bytes = group3_samples().swap_remove(which);
        for (at, b) in edits {
            if let Some(x) = bytes.get_mut(at) {
                *x = b;
            }
        }
        bytes.truncate(cut.max(8));
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
            let p = json!({"name": "fax.tif", "dataBase64": vectorcraft_format::base64_encode(&bytes), "thumbnail": 8});
            let _ = no_panic(s.execute("file.place.info", &p));
            let _ = no_panic(s.execute("file.place", &p));
        });
        prop_assert!(r.is_ok(), "panicked: {:?}", r.err());
    }
}

/// Targa files, which start with no signature: run-length encoded 24 bits (as the `image` crate
/// writes them), a 16-bit export with a one-bit alpha, and by hand a 16-bit run-length encoded file
/// stored right to left and top down, a run-length encoded color-mapped one with 16-bit entries,
/// and a 32-bit one with an extension area.
fn targa_samples() -> Vec<(&'static str, Vec<u8>)> {
    let mut rle = vec![];
    image::RgbImage::from_pixel(5, 3, image::Rgb([200, 40, 10])).write_to(&mut std::io::Cursor::new(&mut rle), image::ImageFormat::Tga).unwrap();
    let mut s = rich_session();
    let r = s.execute("document.export", &json!({"format": "tga", "depth": 16, "ppi": 2})).unwrap();
    let export = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    let mut alpha = vec![0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 2, 0, 16, 0x31, 0x02, 0x00, 0xfc, 0x1f, 0x80, 0x00, 0x00];
    alpha.extend([0x82, 0xe0, 0x83]);
    let mut mapped = vec![0, 1, 9, 5, 0, 2, 0, 16, 0, 0, 0, 0, 3, 0, 2, 0, 8, 0x21, 0x00, 0x00, 0x00, 0xfc];
    mapped.extend([0x80, 6, 0x00, 5, 0x83, 6]);
    let mut extended = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 0x28, 50, 50, 50, 128];
    let mut area = vec![0; 495];
    area[..2].copy_from_slice(&495u16.to_le_bytes());
    area[494] = 4;
    extended.extend(area);
    extended.extend(22u32.to_le_bytes());
    extended.extend([0; 4]);
    extended.extend(b"TRUEVISION-XFILE.\0");
    vec![("a.tga", rle), ("b.tga", export), ("c.tga", alpha), ("d.vda", mapped), ("e.vst", extended)]
}

proptest! {
    #![proptest_config(config())]

    /// Mutated Targa files (headers, run-length packets, color maps, the extension area and the
    /// footer) never crash placing them.
    #[test]
    fn mutated_targa_files_place_without_panics(which in 0usize..5, cut in 0usize..600, edits in prop::collection::vec((0usize..560, any::<u8>()), 0..12)) {
        let (name, mut bytes) = targa_samples().swap_remove(which);
        for (at, b) in edits {
            if let Some(x) = bytes.get_mut(at) {
                *x = b;
            }
        }
        bytes.truncate(cut.max(8));
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
            let p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes), "thumbnail": 8});
            let _ = no_panic(s.execute("file.place.info", &p));
            let _ = no_panic(s.execute("file.place", &p));
            let _ = no_panic(s.execute("clipboard.importImage", &json!({"dataBase64": vectorcraft_format::base64_encode(&bytes)})));
        });
        prop_assert!(r.is_ok(), "{name}: panicked: {:?}", r.err());
    }
}

// ---------- SVG saved with hidden layers and editing data ----------

/// The rich document with a hidden layer, saved as SVG with its editing data.
fn saved_svg() -> String {
    let mut d = rich_doc();
    if let Some(l) = d.layers.first_mut() {
        std::sync::Arc::make_mut(l).visible = false;
    }
    let opts = vectorcraft_svg::ExportOptions { hidden_layers: true, preserve_editing: true, ..Default::default() };
    vectorcraft_svg::export_full(&d, &opts, Some(&vectorcraft_format::save(&d, false))).svg
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn svg_mutated_save_never_panics(
        cut in 0usize..40_000,
        edits in prop::collection::vec((0usize..40_000, prop::sample::select(vec!['<', '>', '"', '/', '-', '9', 'e', '.', ' ', '#', '%', '&', ';', 'x', ']', 'A'])), 0..10),
    ) {
        let svg = mutate_text(&saved_svg(), cut, &edits);
        survive("mutated saved svg", || vectorcraft_engine::cmd::fileio::load("saved.svg", svg.as_bytes()).ok().map(|l| l.doc))?;
    }
}

// ---------- PDF saved with editing data ----------

/// The rich document saved as a PDF-compatible .ai (the native document embedded, uncompressed).
fn saved_ai() -> Vec<u8> {
    static AI: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    AI.get_or_init(|| {
        let r = rich_session().execute("document.save", &json!({"format": "ai", "compression": {"compressText": false}})).unwrap();
        vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
    })
    .clone()
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn ai_mutated_save_never_panics(
        cut in 0usize..200_000,
        flips in prop::collection::vec((0usize..200_000, prop::sample::select(vec![b'0', b'9', b'-', b'.', b' ', b'[', b']', b'<', b'>', b'/', b'(', b'{', b'"', 0u8, 0xff])), 0..10),
    ) {
        let mut b = saved_ai();
        for (i, c) in flips {
            let n = b.len();
            b[i % n] = c;
        }
        b.truncate(cut.min(b.len()).max(9));
        survive("mutated ai", || vectorcraft_engine::cmd::fileio::load("saved.ai", &b).ok().map(|l| l.doc))?;
    }
}

// ---------- PDF presets files ----------

/// Import PDF presets from `data`, then export with each one imported (and save the .ai).
fn pdf_presets(what: &str, data: &str) -> Result<(), TestCaseError> {
    survive_library(what, "pdf.preset.import", data, |s, r| {
        for name in r["imported"].as_array().into_iter().flatten() {
            let _ = no_panic(s.execute("document.exportPdf", &json!({"preset": name})));
            let _ = no_panic(s.execute("document.save", &json!({"format": "ai", "preset": name})));
        }
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn pdf_presets_garbage_never_panics(s in ".{0,300}", head in prop::sample::select(vec!["", "{\"format\": \"vcpdfpresets\", ", "{\"format\": \"vcpdfpresets\", \"presets\": [{\"name\": \"x\", \"settings\": "])) {
        pdf_presets("pdf presets garbage", &format!("{head}{s}"))?;
    }

    #[test]
    fn mutated_pdf_presets_never_panic(cut in 0usize..20_000, edits in prop::collection::vec(arb_edit(), 0..10)) {
        let text = saved("pdf.preset.export", json!({"names": ["VectorCraft Default", "Smallest File Size", "PDF/X-4:2010"]}));
        pdf_presets("mutated pdf presets", &mutate_text(&text, cut, &edits))?;
    }
}

// ---------- what Paste takes from other apps ----------

/// Load `p` into the clipboard with `cmd`, then paste it into a new document: neither may panic.
fn paste_from_elsewhere(cmd: &str, p: Value) -> Result<(), TestCaseError> {
    let r = catch_quiet(|| {
        let mut s = vectorcraft_engine::Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        if no_panic(s.execute(cmd, &p)).is_ok() {
            // Paste may refuse what it was given, e.g. a damaged PDF whose art spans more than the
            // canvas ("result would exceed the canvas"): an error is a fine outcome, a panic isn't.
            let _ = no_panic(s.execute("edit.paste", &json!({"center": [50, 50]})));
        }
    });
    prop_assert!(r.is_ok(), "{cmd}: panicked: {:?}", r.err());
    Ok(())
}

/// [`rich_pdf`], made once.
fn rich_pdf_once() -> Vec<u8> {
    static PDF: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PDF.get_or_init(rich_pdf).clone()
}

/// `bytes` with `edits` (positions wrap around) cut to `cut` bytes, as base64.
fn mutated_b64(mut bytes: Vec<u8>, cut: usize, edits: &[(usize, u8)]) -> String {
    let n = bytes.len().max(1);
    for (at, b) in edits {
        if let Some(x) = bytes.get_mut(at % n) {
            *x = *b;
        }
    }
    bytes.truncate(cut.max(8));
    vectorcraft_format::base64_encode(&bytes)
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn pasted_bitmaps_never_panic(which in 0usize..4, cut in 0usize..400, edits in prop::collection::vec((0usize..120, any::<u8>()), 0..12)) {
        let (_, image) = raster_samples().swap_remove(which);
        paste_from_elsewhere("clipboard.importImage", json!({"dataBase64": mutated_b64(image, cut, &edits), "mime": "image/png"}))?;
    }

    #[test]
    fn pasted_pdf_never_panics(cut in 0usize..40_000, edits in prop::collection::vec((0usize..40_000, any::<u8>()), 0..12)) {
        paste_from_elsewhere("clipboard.importPdf", json!({"dataBase64": mutated_b64(rich_pdf_once(), cut, &edits)}))?;
    }

    // ASCII with control characters and line breaks, a mark and multi-byte characters (characters
    // no font has would send every layout to the slow font fallback).
    #[test]
    fn pasted_text_never_panics(text in r"[\x00-\x7f\u{300}\u{feff}\u{fffd}é€]{0,200}") {
        paste_from_elsewhere("clipboard.importText", json!({ "text": text }))?;
    }
}

// ---------- native files: compressed, and saved for older versions ----------

/// The rich document saved compressed, as v1 and as v2 (pretty).
fn native_samples() -> &'static [Vec<u8>] {
    use vectorcraft_format::{SaveOptions, save_with};
    static SAMPLES: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    SAMPLES.get_or_init(|| {
        let d = rich_doc();
        [
            SaveOptions { compress: true, ..SaveOptions::default() },
            SaveOptions { version: 1, ..SaveOptions::default() },
            SaveOptions { version: 2, pretty: true, ..SaveOptions::default() },
        ]
        .iter()
        .map(|o| save_with(&d, o).unwrap())
        .collect()
    })
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

proptest! {
    #![proptest_config(config())]

    /// Damaged native files of every kind: flipped bytes and cuts in the compressed stream, in v1
    /// and v2 text, and in the text inside an intact compressed stream.
    #[test]
    fn native_mutated_files_never_panic(which in 0usize..4, cut in 0usize..80_000, edits in prop::collection::vec((0usize..80_000, any::<u8>()), 0..10)) {
        let samples = native_samples();
        let mut bytes = samples.get(which).cloned().unwrap_or_else(|| {
            let mut f = vectorcraft_format::save(&rich_doc(), false);
            f.truncate(cut.max(16));
            f
        });
        for &(at, b) in &edits {
            let n = bytes.len();
            bytes[at % n] = b;
        }
        if which < samples.len() {
            bytes.truncate(cut.max(2));
        } else {
            bytes = gzip(&bytes);
        }
        let _ = vectorcraft_format::sniff(&bytes);
        let _ = vectorcraft_format::preview(&bytes);
        survive("mutated native file", || vectorcraft_engine::cmd::fileio::load("x.vectorcraft", &bytes).ok().map(|l| l.doc))?;
    }
}

/// A native file whose area type has 2^60 rows, 2^60 columns, or 2^32 of each (their product
/// overflows) opens with 100 in place of each count over 100, and renders and exports.
#[test]
fn native_area_type_with_2_to_the_60_rows_or_columns_opens_renders_and_exports() {
    let mut s = vectorcraft_engine::Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let made = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "word ".repeat(40), "area": {"width": 300, "height": 120}})).unwrap();
    let id = vectorcraft_doc::NodeId(made["id"].as_u64().unwrap());
    let mut d = (*s.doc().unwrap().doc).clone();
    for (rows, columns) in [(1 << 60, 1), (1, 1 << 60), (1 << 32, 1 << 32)] {
        let Some(vectorcraft_doc::NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("area type") };
        t.area.rows = rows;
        t.area.columns = columns;
        let file = vectorcraft_format::base64_encode(&vectorcraft_format::save(&d, false));
        s.execute("document.open", &json!({"name": "x.vectorcraft", "dataBase64": file})).unwrap();
        let opened = (*s.doc().unwrap().doc).clone();
        let Some(vectorcraft_doc::NodeKind::Text(t)) = opened.node(id).map(|n| &n.kind) else { panic!("area type") };
        assert_eq!((t.area.rows, t.area.columns), (rows.min(100), columns.min(100)));
        survive("area type in 2^60 rows or columns", || Some(opened)).unwrap();
    }
}

/// The rich document as EPS (with a preview, a thumbnail and the document it carries), and the
/// document read back from it.
fn eps_sample() -> Vec<u8> {
    let mut s = rich_session();
    let r = s.execute("document.export", &json!({"format": "eps", "useArtboards": false})).unwrap();
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
}

proptest! {
    #![proptest_config(config())]

    /// Damaged EPS files: their sections, embedded document and thumbnail read as nothing, or as
    /// a document that renders and exports.
    #[test]
    fn eps_mutated_files_never_panic(cut in 0usize..400_000, edits in prop::collection::vec((0usize..400_000, any::<u8>()), 0..12)) {
        static SAMPLE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        let mut bytes = SAMPLE.get_or_init(eps_sample).clone();
        for &(at, b) in &edits {
            let n = bytes.len();
            bytes[at % n] = b;
        }
        bytes.truncate(cut.max(4));
        let _ = vectorcraft_eps::sections(&bytes);
        let _ = vectorcraft_eps::thumbnail(&bytes);
        survive("mutated EPS file", || vectorcraft_eps::native(&bytes).and_then(|n| vectorcraft_format::load(&n).ok()))?;
    }

    /// Damaged EPS files without the document: the PostScript interpreter (or the preview
    /// fallback) reads them as an error or a document.
    #[test]
    fn eps_mutated_postscript_never_panics(cut in 0usize..400_000, edits in prop::collection::vec((0usize..400_000, any::<u8>()), 0..12)) {
        static SAMPLE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        let mut bytes = SAMPLE.get_or_init(foreign_eps_sample).clone();
        for &(at, b) in &edits {
            let n = bytes.len();
            bytes[at % n] = b;
        }
        bytes.truncate(cut.max(4));
        survive("mutated EPS PostScript", || vectorcraft_eps::import(&bytes).ok().map(|r| r.document))?;
    }

    /// A damaged EPS another app copied: Paste reads it as an error or as objects.
    #[test]
    fn pasted_eps_never_panics(cut in 0usize..400_000, edits in prop::collection::vec((0usize..400_000, any::<u8>()), 0..12)) {
        static SAMPLE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        paste_from_elsewhere("clipboard.importEps", json!({"dataBase64": mutated_b64(SAMPLE.get_or_init(foreign_eps_sample).clone(), cut, &edits)}))?;
    }

    /// Hostile PostScript programs: operators in any order with any operands, opened and placed;
    /// as files in the legacy Illustrator format too (the creator line is what turns on its
    /// documented `u` … `U` groups), the groups unbalanced, deep and among clips; in a layer, which
    /// the editing data reader reads from the program too.
    #[test]
    fn eps_hostile_programs_never_panic(tokens in prop::collection::vec(arb_ps_token(), 0..60), illustrator in any::<bool>(), layers in any::<bool>()) {
        let head = if illustrator { "%%Creator: Adobe Illustrator(R) 8.0\n%%EndComments\n/u {} def /U {} def" } else { "%%EndComments" };
        let (open, close) = if layers {
            ("/Lb {10 {pop} repeat} def /Ln {pop} def /LB {} def\n%AI5_BeginLayer\n1 1 1 1 0 0 -1 0 0 0 Lb\n(L) Ln\n", "\nLB\n%AI5_EndLayer--")
        } else {
            ("", "")
        };
        let ps = format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 200\n{head}\n{open}{}{close}\nshowpage\n", tokens.join(" "));
        survive("hostile PostScript", || vectorcraft_engine::cmd::fileio::load("x.eps", ps.as_bytes()).ok().map(|l| l.doc))?;
        let r = catch_quiet(|| {
            let mut s = rich_session();
            let _ = no_panic(s.execute("file.place", &json!({"name": "x.eps", "dataBase64": vectorcraft_format::base64_encode(ps.as_bytes())})));
        });
        r.map_err(|msg| TestCaseError::fail(format!("placing hostile PostScript panicked: {msg}")))?;
    }

    /// Hostile PostScript programs in `.ps` files whose header is only `%!`: opened, placed, and
    /// pasted as a copied file is.
    #[test]
    fn ps_hostile_programs_never_panic(tokens in prop::collection::vec(arb_ps_token(), 0..60)) {
        let ps = format!("%!\n{}\nshowpage\n", tokens.join(" "));
        survive("hostile .ps", || vectorcraft_engine::cmd::fileio::load("x.ps", ps.as_bytes()).ok().map(|l| l.doc))?;
        let r = catch_quiet(|| {
            let mut s = rich_session();
            let _ = no_panic(s.execute("file.place", &json!({"name": "x.ps", "dataBase64": vectorcraft_format::base64_encode(ps.as_bytes())})));
        });
        r.map_err(|msg| TestCaseError::fail(format!("placing a hostile .ps panicked: {msg}")))?;
        paste_from_elsewhere("clipboard.importEps", json!({"dataBase64": vectorcraft_format::base64_encode(ps.as_bytes())}))?;
    }

    /// Damaged or hostile CMYK images (a CMYK JPEG, an image with a key color): read as images,
    /// or as an error.
    #[test]
    fn eps_cmyk_images_never_panic(cut in 0usize..2_500, edits in prop::collection::vec((0usize..2_500, prop::sample::select(vec!['0', '9', 'f', 'a', ' ', '\n', '<', '>', '/', '[', ']', '(', ')', '-'])), 0..12)) {
        let text = mutate_text(&cmyk_images_eps(), cut, &edits);
        survive("mutated EPS CMYK images", || vectorcraft_eps::import(text.as_bytes()).ok().map(|r| r.document))?;
    }

    /// Damaged or hostile CCITT fax images (Group 4 and Group 3 data, their parameters): read as
    /// images, or as an error.
    #[test]
    fn eps_ccitt_images_never_panic(cut in 0usize..800, edits in prop::collection::vec((0usize..800, prop::sample::select(vec!['0', '1', '9', 'f', '8', ' ', '\n', '<', '>', '/', '-', 't', 'e'])), 0..12)) {
        let text = mutate_text(&ccitt_images_eps(), cut, &edits);
        survive("mutated EPS CCITT images", || vectorcraft_eps::import(text.as_bytes()).ok().map(|r| r.document))?;
    }

    /// TIFF-predicted Flate data (Predictor 2) at every depth the filter accepts, with any bytes, 1 to 5
    /// colors and 1 to 39 columns, read by images of up to 23 × 7 pixels at 1, 2, 4, 8, 12 or 16 bits:
    /// read as an image, or as an error.
    #[test]
    fn eps_tiff_predicted_images_never_panic(
        data in prop::collection::vec(any::<u8>(), 0..300),
        bpc in prop::sample::select(vec![1u8, 2, 4, 8, 16]),
        colors in 1u8..6,
        columns in 1u16..40,
        image_bpc in prop::sample::select(vec![1u8, 2, 4, 8, 12, 16]),
        (width, height) in (1u8..24, 1u8..8),
    ) {
        let hex: String = deflated(&data).iter().map(|b| format!("{b:02x}")).collect();
        let text = format!(
            "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n40 30 scale {width} {height} {image_bpc} [{width} 0 0 -{height} 0 {height}] \
             currentfile /ASCIIHexDecode filter << /Predictor 2 /Colors {colors} /BitsPerComponent {bpc} /Columns {columns} >> /FlateDecode filter image\n\
             {hex}>\nshowpage\n%%EOF\n"
        );
        survive("TIFF-predicted EPS images", || vectorcraft_eps::import(text.as_bytes()).ok().map(|r| r.document))?;
    }
}

/// An EPS with two 8 × 3 CCITT fax images that the PostScript interpreter reads, coded by hand from
/// ITU-T T.4 and T.6: two rows of white 2, black 4, white 2 over a black row. The first is Group 4
/// (horizontal, vertical 0 and vertical ±2 modes, then EOFB), the second Group 3 one-dimensional
/// with an EOL before each row and six at the end, drawn as an image mask.
fn ccitt_images_eps() -> String {
    "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\ngsave 40 30 scale \
     8 3 1 [8 0 0 -3 0 3] currentfile /ASCIIHexDecode filter << /K -1 /Columns 8 /Rows 3 >> /CCITTFaxDecode filter image\n\
     2efc20c0040040>\ngrestore 50 50 translate 40 30 scale \
     8 3 false [8 0 0 -3 0 3] currentfile /ASCIIHexDecode filter << /Columns 8 /EndOfLine true /BlackIs1 false >> /CCITTFaxDecode filter imagemask\n\
     00176e002edc004d45001001001001001001>\nshowpage\n%%EOF\n"
        .to_string()
}

/// [`ccitt_images_eps`] as it is: both images draw their rows (`B` black, `w` white, `.` left
/// clear by the mask), without a warning.
#[test]
fn the_ccitt_images_fixture_reads_both_images() {
    let r = vectorcraft_eps::import(ccitt_images_eps().as_bytes()).unwrap();
    assert!(r.warnings.is_empty() && !r.preview, "{:?}", r.warnings);
    let d = &r.document;
    let shade = |p: &image::Rgba<u8>| match p.0 {
        [_, _, _, 0] => '.',
        [r, ..] if r < 128 => 'B',
        _ => 'w',
    };
    let mut found = vec![];
    d.walk(|n| {
        if let vectorcraft_doc::NodeKind::Image(im) = &n.kind {
            let px = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
            found.push(px.pixels().map(shade).collect::<String>());
        }
    });
    assert_eq!(found, ["wwBBBBwwwwBBBBwwBBBBBBBB", "..BBBB....BBBB..BBBBBBBB"]);
}

/// An EPS with CMYK images that the PostScript interpreter reads: a CMYK JPEG through `DCTDecode`
/// (Adobe's inverted samples), and an image whose key color hides a pixel.
fn cmyk_images_eps() -> String {
    use vectorcraft_render::encode::jpeg::{ColorModel, JpegOptions, encode};
    let o = JpegOptions { color_model: ColorModel::Cmyk, embed_icc: false, ..JpegOptions::default() };
    let jpeg = encode(&[0, 102, 255, 0].repeat(16), 4, 4, 90, None, &o).unwrap();
    let hex: String = jpeg.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n/DeviceCMYK setcolorspace gsave 40 40 scale \
         << /ImageType 1 /Width 4 /Height 4 /BitsPerComponent 8 /Decode [1 0 1 0 1 0 1 0] /ImageMatrix [4 0 0 -4 0 4] \
         /DataSource currentfile /ASCIIHexDecode filter /DCTDecode filter >> image\n{hex}>\ngrestore 50 50 translate 40 40 scale \
         << /ImageType 4 /Width 2 /Height 1 /BitsPerComponent 8 /Decode [0 1 0 1 0 1 0 1] /ImageMatrix [2 0 0 -1 0 1] \
         /MaskColor [10 20 30 40] /DataSource <0a141e28 00667f08> >> image\nshowpage\n%%EOF\n"
    )
}

/// [`cmyk_images_eps`] as it is: both images keep their ink amounts, the keyed one with its mask.
#[test]
fn the_cmyk_images_fixture_reads_its_inks_and_mask() {
    let r = vectorcraft_eps::import(cmyk_images_eps().as_bytes()).unwrap();
    assert!(r.warnings.is_empty() && !r.preview, "{:?}", r.warnings);
    let d = &r.document;
    let mut found = vec![];
    d.walk(|n| {
        if let vectorcraft_doc::NodeKind::Image(im) = &n.kind {
            found.push((d.images[&im.key].cmyk().map(|i| i.data.len()), n.extras().mask.is_some()));
        }
    });
    assert_eq!(found, [(Some(64), false), (Some(8), true)]);
}

/// [`eps_sample`] as another app would write it: without the document it carries, so its
/// PostScript is read.
fn foreign_eps_sample() -> Vec<u8> {
    let mut bytes = eps_sample();
    let marker = b"%VectorCraft_BeginData: native";
    let at = bytes.windows(marker.len()).position(|w| w == marker).unwrap();
    bytes[at + marker.len() - 1] = b'x';
    bytes
}

/// The operators and operands of a hostile PostScript program.
fn arb_ps_token() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_num(),
        prop::sample::select(vec![
            "moveto", "lineto", "curveto", "rlineto", "rmoveto", "closepath", "fill", "eofill", "stroke", "clip", "eoclip", "newpath",
            "gsave", "grestore", "save", "restore", "concat", "translate", "scale", "rotate", "setlinewidth", "setdash", "setgray",
            "setrgbcolor", "setcmykcolor", "sethsbcolor", "setcolorspace", "setcolor", "setpattern", "makepattern", "shfill", "arc", "arcn",
            "arct", "arcto", "rectfill", "rectclip", "rectstroke", "image", "imagemask", "colorimage", "currentfile", "filter",
            "readhexstring", "readstring", "show", "stringwidth", "charpath", "findfont", "scalefont", "makefont", "setfont", "selectfont",
            "def", "bind", "load", "dup", "exch", "pop", "roll", "index", "copy", "for", "repeat", "loop", "exit", "if", "ifelse", "exec",
            "stopped", "begin", "end", "dict", "array", "string", "get", "put", "getinterval", "putinterval", "aload", "astore", "forall",
            "[", "]", "<<", ">>", "{", "}", "matrix", "currentmatrix", "setmatrix", "transform", "itransform", "invertmatrix", "pathbbox",
            "clippath", "initclip", "currentpoint", "eexec", "cleartomark", "mark", "counttomark", "showpage", "gstate", "setgstate",
            "clipsave", "cliprestore", "languagelevel", "cvx", "findresource", "defineresource", "undefineresource", "resourcestatus",
            "resourceforall", "/Category", "/Generic", "/ProcSet", "(*)", "(G?n*\\\\*?)", "{pop}", "8 string",
            "(*) {pop} 8 string /Category resourceforall", "/Generic /Category findresource dup length dict copy /Category defineresource",
            "dictstack", "8 array", "1183615869 internaldict", "cshow", "currentsystemparams", "currentuserparams", "flushfile", "xcheck",
            "systemdict", "/userdict", "SharedFontDirectory", "(1 2 add) 0 () /SubFileDecode filter cvx exec", "currentfile cvx exec",
            "/p { (p) 0 () /SubFileDecode filter cvx exec } def p", "currentfile 0 () /SubFileDecode filter cvx exec", "cvlit", "{ (a) cshow }",
            "/ASCII85Decode", "/ASCIIHexDecode", "/FlateDecode", "/RunLengthDecode", "/LZWDecode", "/DCTDecode", "/SubFileDecode",
            "/CCITTFaxDecode", "<< /K -1 /Columns 8 /Rows 3 >> /CCITTFaxDecode", "<< /K 2 /Columns 99999 /EndOfLine true /EncodedByteAlign true /BlackIs1 true >>",
            "/DeviceRGB", "/DeviceCMYK", "/DeviceGray", "/Pattern", "/x", "x", "/Helvetica", "(abc)", "<ff00>", "<~87cURD]i,\"Ebo80~>",
            "true", "false", "null", "[/Separation (S) /DeviceCMYK {dup dup dup}]", "[/Indexed /DeviceRGB 1 <ff000000ff00>]",
            "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 1 1] /Function << /FunctionType 2 /C0 [0 0 0] /C1 [1 1 1] /N 1 >> >>",
            "<< /PatternType 2 /Shading << /ShadingType 3 /ColorSpace /DeviceGray /Coords [0 0 0 9 9 9] /Function << /FunctionType 3 /Functions [] /Bounds [] /Encode [] >> >> >>",
            "<< /ImageType 1 /Width 2 /Height 2 /BitsPerComponent 8 /ImageMatrix [2 0 0 2 0 0] /DataSource (abcdefghijkl) >>",
            "u", "U", "u u u", "U U", "{ u } 300 repeat", "1 1 300 { pop u 0 0 9 9 rectclip } for",
            // Probing, files, forms and devices.
            "status", "token", "bytesavailable", "resetfile", "pdfmark", "execform", "nulldevice", "strokepath", "pathforall",
            "currenthsbcolor", "currentcolorrendering", "gcheck", "rootfont", "setcachedevice2", "writestring", "(%stdout) (w) file",
            "//x", "{ //dup }", "/ReusableStreamDecode", "currentfile /ASCII85Decode filter /ReusableStreamDecode filter",
            "<< /Predictor 12 /Columns 2 /Colors 1 >> /FlateDecode", "<< /Predictor 2 /Colors 3 /BitsPerComponent 4 /Columns 3 >> /LZWDecode",
            "<< /Predictor 2 /BitsPerComponent 16 /Columns 5 >> /FlateDecode", "<< /FormType 1 /BBox [0 0 9 9] /PaintProc { pop 0 0 5 5 rectfill } >>",
            // Tiling patterns, uncoloured ones and patterns drawing patterns.
            "<< /PatternType 1 /PaintType 1 /XStep 5 /YStep 5 /BBox [0 0 5 5] /PaintProc { pop 0 0 3 3 rectfill } >> matrix makepattern",
            "<< /PatternType 1 /PaintType 2 /XStep 5 /YStep 5 /BBox [0 0 5 5] /PaintProc { pop dup setpattern 0 0 3 3 rectfill } >>",
            "[/Pattern /DeviceRGB] setcolorspace", "1 0 0", "setpattern",
            // Mesh and function shadings, sampled functions.
            "<< /ShadingType 4 /ColorSpace /DeviceGray /DataSource [0 0 0 0 0 9 0 1 0 0 9 1 1 9 9 0] >>",
            "<< /ShadingType 5 /ColorSpace /DeviceGray /VerticesPerRow 2 /DataSource [0 0 0 9 0 1 0 9 1 9 9 0] >>",
            "<< /ShadingType 6 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 9 0 9 0 1] /DataSource <00ff10ff20ff30> >>",
            "<< /ShadingType 7 /ColorSpace /DeviceRGB /BitsPerCoordinate 32 /BitsPerComponent 16 /BitsPerFlag 8 /Decode [0 9 0 9 0 1 0 1 0 1] /DataSource (abc) >>",
            "<< /ShadingType 1 /ColorSpace /DeviceGray /Function << /FunctionType 0 /Domain [0 1 0 1] /Range [0 1] /Size [2 2] /BitsPerSample 8 /DataSource <00ff00ff> >> >>",
            "<< /FunctionType 0 /Domain [0 1] /Range [0 1 0 1 0 1] /Size [99999999] /BitsPerSample 32 /DataSource () >>",
            // Masked images.
            "<< /ImageType 3 /InterleaveType 1 /DataDict << /ImageType 1 /Width 2 /Height 2 /BitsPerComponent 8 /ImageMatrix [2 0 0 2 0 0] /DataSource (abcdefghijklmnop) >> /MaskDict << /ImageType 1 /Width 2 /Height 2 /BitsPerComponent 1 /ImageMatrix [2 0 0 2 0 0] >> >>",
            "<< /ImageType 3 /InterleaveType 2 /DataDict << /ImageType 1 /Width 3 /Height 1 /BitsPerComponent 8 /ImageMatrix [3 0 0 1 0 0] /DataSource (abcdefghijklmnop) >> /MaskDict << /ImageType 1 /Width 3 /Height 5 /BitsPerComponent 1 /ImageMatrix [3 0 0 5 0 0] >> >>",
            "<< /ImageType 4 /Width 2 /Height 2 /BitsPerComponent 8 /Decode [0 1 0 1 0 1 0 1] /ImageMatrix [2 0 0 2 0 0] /MaskColor [97 98 99 100] /DataSource (abcdefghijklmnop) >>",
            "2 2 8 [2 0 0 2 0 0] (abcdefghijklmnop) false 4 colorimage",
            // Type 3 fonts and the show operators that space their glyphs.
            "/T3 << /FontType 3 /FontMatrix [0.1 0 0 0.1 0 0] /FontBBox [0 0 9 9] /Encoding [/a /b] /BuildChar { pop pop 9 0 setcharwidth 0 0 5 5 rectfill } >> definefont setfont",
            "/T4 << /FontType 3 /FontMatrix [1 0 0 1 0 0] /BuildGlyph { pop pop (x) show } /Encoding [/a] >> definefont setfont",
            "glyphshow", "xshow", "xyshow", "awidthshow", "kshow", "/a", "[1 2 3]",
            // Several data sources reading one file in turn, and sources of uneven lengths.
            "/f currentfile /ASCIIHexDecode filter def", "{f 3 string readstring pop}", "{f 0 string readstring pop}", "true 3 colorimage",
            "3 2 8 [3 0 0 2 0 0] {f 5 string readstring pop} {(ab)} {f 1 string readstring pop} true 3 colorimage",
            "<< /ImageType 1 /Width 3 /Height 2 /BitsPerComponent 4 /Decode [0 1 0 1 0 1 0 1] /ImageMatrix [3 0 0 2 0 0] /MultipleDataSources true /DataSource [(a) (bc) {f 2 string readstring pop} (d)] >>",
            // Local VM restored: dictionary changes undone, saves restored twice.
            "/a 1 def save /a 2 def", "restore", "save dup restore", "1 dict save exch /k 1 put", "cachestatus", "/n 0 def save /n n 1 add store",
            // Executable strings and keys other than names (PLRM 3rd ed., `cvx`, `load`).
            "(>>) cvx", "(1 2 add) cvx exec", "(x) cvx dup exec", "0 load", "1 { } def", "(mark) cvx cvlit"
        ])
        .prop_map(str::to_string),
    ]
}

proptest! {
    #![proptest_config(config())]

    /// Damaged password-protected PDFs, opened with the open password, the permissions password
    /// (which reads the encryption dictionary to find the open password) or a wrong one.
    #[test]
    fn mutated_encrypted_pdfs_never_panic(
        cut in 0usize..2_000,
        edits in prop::collection::vec((0usize..2_000, any::<u8>()), 0..12),
        password in prop::sample::select(vec!["pw", "pw-owner", "wrong", ""]),
    ) {
        use vectorcraft_testkit::pdf::{PdfPage, pdf};
        let bytes = pdf(&[PdfPage::new(100.0, 100.0, "0 0 1 rg 20 20 50 50 re f")], Some("pw"));
        let b64 = mutated_b64(bytes, cut, &edits);
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            let p = json!({"name": "x.pdf", "dataBase64": b64, "password": password});
            let _ = no_panic(s.execute("document.pdfInfo", &p));
            if no_panic(s.execute("document.open", &p)).is_ok() {
                let d = (*s.doc().unwrap().doc).clone();
                survive("encrypted pdf", || Some(d)).unwrap();
            }
        });
        r.map_err(|msg| TestCaseError::fail(format!("{password}: panicked: {msg}")))?;
    }
}

// ---------- DXF ----------

/// The rich document exported as DXF (2018: hatches, splines, text, images; R12: polylines).
fn rich_dxf(version: &str) -> Vec<u8> {
    vectorcraft_engine::cmd::fileio::encode(&rich_doc(), "dxf", &json!({"version": version, "preserve": "editability"})).unwrap()
}

/// Group-code pairs as DXF text.
fn dxf_text(pairs: &[(i32, String)]) -> String {
    pairs.iter().map(|(c, v)| format!("{c:>3}\n{v}\n")).collect()
}

/// One entity: a type and a run of groups with hostile numbers and names.
fn arb_dxf_entity() -> impl Strategy<Value = Vec<(i32, String)>> {
    let kind = prop::sample::select(vec![
        "LINE",
        "CIRCLE",
        "ARC",
        "ELLIPSE",
        "LWPOLYLINE",
        "POLYLINE",
        "VERTEX",
        "SEQEND",
        "SPLINE",
        "SOLID",
        "3DFACE",
        "HATCH",
        "TEXT",
        "MTEXT",
        "INSERT",
        "ATTRIB",
        "DIMENSION",
        "VIEWPORT",
    ]);
    let code = prop::sample::select(vec![
        2, 6, 7, 8, 10, 11, 12, 13, 20, 21, 22, 23, 40, 41, 42, 43, 44, 45, 48, 50, 51, 60, 62, 66, 67, 70, 71, 72, 73, 74, 75, 76, 90, 91, 92, 93,
        94, 95, 96, 97, 210, 220, 230, 370, 420, 440, 450,
    ]);
    let value = prop_oneof![arb_num(), Just("B".to_string()), Just("*U1".to_string()), Just(r"{\fA;x}\P%%c^".to_string())];
    (kind, prop::collection::vec((code, value), 0..24)).prop_map(|(k, g)| std::iter::once((0, k.to_string())).chain(g).collect())
}

/// A drawing whose block "B" holds `block` (it may insert itself) and whose entities are `body`.
fn hostile_dxf(block: &[Vec<(i32, String)>], body: &[Vec<(i32, String)>]) -> Vec<u8> {
    let head: Vec<(i32, String)> =
        [(0, "SECTION"), (2, "BLOCKS"), (0, "BLOCK"), (2, "B"), (10, "0"), (20, "0")].iter().map(|(c, v)| (*c, v.to_string())).collect();
    let mid: Vec<(i32, String)> = [(0, "ENDBLK"), (0, "ENDSEC"), (0, "SECTION"), (2, "ENTITIES")].iter().map(|(c, v)| (*c, v.to_string())).collect();
    let tail = vec![(0, "ENDSEC".to_string()), (0, "EOF".to_string())];
    let all: Vec<(i32, String)> = head.into_iter().chain(block.concat()).chain(mid).chain(body.concat()).chain(tail).collect();
    dxf_text(&all).into_bytes()
}

/// Import with every option at once, and place it; whatever comes back must render and export.
fn survive_dxf(what: &str, bytes: &[u8], fit: bool) -> Result<(), TestCaseError> {
    let o = vectorcraft_cad::ImportOptions { fit, center: !fit, merge_layers: fit, ..Default::default() };
    survive(what, || vectorcraft_cad::import(bytes, &o).ok().map(|r| r.document))?;
    let _ = vectorcraft_cad::info(bytes);
    let _ = vectorcraft_cad::is_dxf(bytes);
    survive(what, || {
        let mut s = vectorcraft_engine::Session::new();
        no_panic(s.execute("file.new", &json!({"width": 300, "height": 200}))).ok()?;
        let p = json!({"name": "x.dxf", "dataBase64": vectorcraft_format::base64_encode(bytes), "dxf": {"fit": fit}, "thumbnail": 8});
        let _ = no_panic(s.execute("file.place.info", &p));
        let _ = no_panic(s.execute("file.place", &p));
        let _ = no_panic(s.execute("document.dxfInfo", &p));
        Some((*s.doc().ok()?.doc).clone())
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn dxf_garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..400), section in any::<bool>()) {
        let mut b = if section { b"  0\nSECTION\n  2\nENTITIES\n".to_vec() } else { vec![] };
        b.extend(bytes);
        survive_dxf("dxf garbage", &b, section)?;
    }

    #[test]
    fn dxf_hostile_entities_never_panic(
        block in prop::collection::vec(arb_dxf_entity(), 0..4),
        body in prop::collection::vec(arb_dxf_entity(), 0..8),
        fit in any::<bool>(),
    ) {
        let bytes = hostile_dxf(&block, &body);
        survive_dxf("hostile dxf", &bytes, fit)?;
    }

    #[test]
    fn dxf_mutated_export_never_panics(
        r12 in any::<bool>(),
        cut in 0usize..60_000,
        edits in prop::collection::vec((0usize..60_000, prop::sample::select(vec![b'0', b'1', b'9', b'-', b'.', b'e', b'\n', b' ', b'A', b'^', b'\\', 0xC3])), 0..12),
    ) {
        static SAMPLES: std::sync::OnceLock<[Vec<u8>; 2]> = std::sync::OnceLock::new();
        let samples = SAMPLES.get_or_init(|| [rich_dxf("2018"), rich_dxf("R12")]);
        let mut bytes = samples[usize::from(r12)].clone();
        for &(at, b) in &edits {
            let n = bytes.len();
            bytes[at % n] = b;
        }
        bytes.truncate(cut.max(8));
        survive_dxf("mutated dxf", &bytes, cut % 2 == 0)?;
    }
}

// ---------- Windows metafiles: opened, placed and pasted ----------

/// The rich document as EMF and as WMF.
fn metafile_samples() -> &'static [Vec<u8>] {
    static SAMPLES: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    SAMPLES.get_or_init(|| {
        let mut s = rich_session();
        ["emf", "wmf"]
            .iter()
            .map(|f| {
                let r = s.execute("document.export", &json!({ "format": f })).unwrap();
                vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
            })
            .collect()
    })
}

/// An EMF of a 100 × 100 mm frame on a device of 10 pixels a millimetre holding `records`.
fn emf_of(records: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut b: Vec<u8> = vec![];
    let mut head = vec![0u8; 108];
    head[..4].copy_from_slice(&1u32.to_le_bytes());
    head[4..8].copy_from_slice(&108u32.to_le_bytes());
    for (at, v) in [(32, 10_000i32), (36, 10_000), (72, 1000), (76, 1000), (80, 100), (84, 100), (100, 100_000), (104, 100_000)] {
        head[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    head[40..44].copy_from_slice(b" EMF");
    b.extend(head);
    for (kind, body) in records {
        let mut body = body.clone();
        body.resize(body.len().div_ceil(4) * 4, 0);
        b.extend(kind.to_le_bytes());
        b.extend((8 + body.len() as u32).to_le_bytes());
        b.extend(body);
    }
    b.extend([14, 0, 0, 0, 20, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0, 0, 20, 0, 0, 0]);
    b
}

/// A placeable WMF of a 1 × 1 inch box holding `records`.
fn wmf_of(records: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut b: Vec<u8> = vec![0xD7, 0xCD, 0xC6, 0x9A, 0, 0, 0, 0, 0, 0, 0xA0, 0x05, 0xA0, 0x05, 0xA0, 0x05, 0, 0, 0, 0];
    let sum = b.as_chunks::<2>().0.iter().fold(0u16, |a, w| a ^ u16::from_le_bytes(*w));
    b.extend(sum.to_le_bytes());
    b.extend([1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for (function, body) in records {
        let mut body = body.clone();
        body.resize(body.len().div_ceil(2) * 2, 0);
        b.extend((3 + body.len() as u32 / 2).to_le_bytes());
        b.extend(function.to_le_bytes());
        b.extend(body);
    }
    b.extend([3, 0, 0, 0, 0, 0]);
    b
}

/// Open the bytes as `name`, and place them: neither may panic.
fn survive_metafile(what: &str, name: &str, bytes: &[u8]) -> Result<(), TestCaseError> {
    survive(what, || vectorcraft_engine::cmd::fileio::load(name, bytes).ok().map(|l| l.doc))?;
    let r = catch_quiet(|| {
        let mut s = vectorcraft_engine::Session::new();
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let _ = no_panic(s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})));
    });
    prop_assert!(r.is_ok(), "{what}: placing panicked: {:?}", r.err());
    Ok(())
}

/// A record body: random bytes, or small numbers (coordinates, counts and offsets that land).
fn arb_body() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..96),
        prop::collection::vec(prop_oneof![Just(0i32), Just(1), Just(-1), 0i32..200, Just(i32::MAX), Just(i32::MIN)], 0..24)
            .prop_map(|v| v.iter().flat_map(|x| x.to_le_bytes()).collect()),
    ]
}

/// WMF record functions VectorCraft reads, and one it doesn't.
const WMF_FUNCTIONS: [u16; 33] = [
    0x0324, 0x0325, 0x0538, 0x041B, 0x0418, 0x061C, 0x0817, 0x081A, 0x0830, 0x0416, 0x0415, 0x02FA, 0x02FC, 0x02FB, 0x012D, 0x01F0, 0x0521, 0x0A32,
    0x0F43, 0x0B41, 0x0940, 0x061D, 0x0213, 0x0214, 0x001E, 0x0127, 0x0106, 0x0209, 0x012E, 0x00F7, 0x01F9, 0x0142, 0x0999,
];

proptest! {
    #![proptest_config(config())]

    #[test]
    fn mutated_metafiles_never_panic(which in 0usize..2, cut in 0usize..200_000, edits in prop::collection::vec((0usize..200_000, any::<u8>()), 0..16)) {
        let mut bytes = metafile_samples()[which].clone();
        let n = bytes.len();
        for &(at, b) in &edits {
            bytes[at % n] = b;
        }
        bytes.truncate(cut.max(8));
        survive_metafile("mutated metafile", ["x.emf", "x.wmf"][which], &bytes)?;
    }

    #[test]
    fn random_emf_records_never_panic(records in prop::collection::vec((0u32..124, arb_body()), 0..32)) {
        survive_metafile("EMF records", "x.emf", &emf_of(&records))?;
    }

    #[test]
    fn random_wmf_records_never_panic(records in prop::collection::vec((prop::sample::select(WMF_FUNCTIONS.to_vec()), arb_body()), 0..32)) {
        survive_metafile("WMF records", "x.wmf", &wmf_of(&records))?;
    }

    #[test]
    fn pasted_metafiles_never_panic(which in 0usize..2, cut in 0usize..200_000, edits in prop::collection::vec((0usize..200_000, any::<u8>()), 0..12)) {
        paste_from_elsewhere("clipboard.importEmf", json!({"dataBase64": mutated_b64(metafile_samples()[which].clone(), cut, &edits)}))?;
    }
}

// ---------- print presets files ----------

/// Import print presets from `data`, then lay out (and print, when short) the rich document with
/// each one imported, by its settings and by its name.
fn print_presets(what: &str, data: &str) -> Result<(), TestCaseError> {
    survive_library(what, "print.presets.import", data, |s, r| {
        let list = no_panic(s.execute("print.presets.list", &json!({}))).unwrap_or_default();
        for name in r["imported"].as_array().into_iter().flatten() {
            let Some(p) = list["presets"].as_array().into_iter().flatten().find(|p| &p["name"] == name) else { continue };
            let _ = no_panic(s.execute("print.preview", &json!({ "preset": name })));
            let settings = json!({ "settings": p["settings"] });
            let pages = no_panic(s.execute("print.preview", &settings)).ok().and_then(|v| v["pages"].as_u64());
            let _ = no_panic(s.execute("print.setup", &settings));
            if pages.is_some_and(|n| n <= 8) {
                let _ = no_panic(s.execute("file.print", &settings));
            }
        }
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn print_presets_garbage_never_panics(s in ".{0,300}", head in prop::sample::select(vec!["", "{\"format\": \"vcprintpresets\", ", "{\"format\": \"vcprintpresets\", \"presets\": [{\"name\": \"x\", \"settings\": "])) {
        print_presets("print presets garbage", &format!("{head}{s}"))?;
    }

    #[test]
    fn mutated_print_presets_never_panic(cut in 0usize..20_000, edits in prop::collection::vec(arb_edit(), 0..10)) {
        let mut s = rich_session();
        s.execute("print.presets.save", &json!({"name": "Tiles", "settings": {"scaling": "tileFull", "scale": {"width": 300, "height": 300}, "marks": {"trim": true}}})).unwrap();
        s.execute("print.presets.save", &json!({"name": "Seps", "settings": {"output": {"mode": "separations", "inks": [{"name": "Cyan", "print": false}]}}})).unwrap();
        let text = s.execute("print.presets.export", &json!({"names": ["Tiles", "Seps", "[Default]"]})).unwrap()["data"].as_str().unwrap().to_string();
        print_presets("mutated print presets", &mutate_text(&text, cut, &edits))?;
    }
}

// ---------- perspective grid presets files ----------

/// Import perspective grid presets from `data`, then apply each one and draw on its grid.
fn perspective_presets(what: &str, data: &str) -> Result<(), TestCaseError> {
    survive_library(what, "perspective.presets.import", data, |s, r| {
        for name in r["imported"].as_array().into_iter().flatten() {
            let _ = no_panic(s.execute("perspective.grid.preset", &json!({"name": name})));
            let _ = no_panic(s.execute("perspective.grid.define", &json!({"name": name, "gridline": 3})));
            let _ = no_panic(
                s.execute("perspective.draw", &json!({"command": "shape.rectangle", "params": {"x": 300, "y": 400, "width": 40, "height": 30}})),
            );
        }
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn perspective_presets_garbage_never_panics(s in ".{0,300}", head in prop::sample::select(vec!["", "{\"format\": \"vcperspective\", ", "{\"format\": \"vcperspective\", \"presets\": [{\"name\": \"x\", \"kind\": 3, \"scale\": "])) {
        perspective_presets("perspective presets garbage", &format!("{head}{s}"))?;
    }

    #[test]
    fn mutated_perspective_presets_never_panic(cut in 0usize..20_000, edits in prop::collection::vec(arb_edit(), 0..10)) {
        let mut s = rich_session();
        s.execute("perspective.presets.save", &json!({"name": "Tall", "kind": 3, "units": "inches", "scale": [1, 4], "angle": 25, "thirdVp": [1, 30]})).unwrap();
        s.execute("perspective.presets.save", &json!({"name": "Flat", "preset": "[1P-Low View]", "gridline": 4, "groundColor": "#00ff00"})).unwrap();
        let text = s.execute("perspective.presets.export", &json!({"names": ["Tall", "Flat", "[2P-High View]"]})).unwrap()["data"].as_str().unwrap().to_string();
        perspective_presets("mutated perspective presets", &mutate_text(&text, cut, &edits))?;
    }
}

// ---------- the editing data of Illustrator files ----------

/// `data` as ASCII85, up to the `~>`.
fn ascii85(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(4) {
        let mut x = chunk.iter().enumerate().fold(0u32, |v, (i, b)| v | u32::from(*b) << (24 - 8 * i));
        let mut d = [0u8; 5];
        for c in d.iter_mut().rev() {
            *c = (x % 85) as u8 + b'!';
            x /= 85;
        }
        out.extend(d.iter().take(chunk.len() + 1).map(|c| char::from(*c)));
    }
    out.push_str("~>");
    out
}

/// A pattern's definition in editing data: its name and tile, its art on `%_` lines, its end.
const PATTERN: &str = "%AI3_BeginPattern: (Dots)\n(Dots) 0 0 6 6\n%_0 A\n%_0 Ae\n%_u\n%_0 0 m\n%_0 6 L\n%_6 6 L\n%_6 0 L\n%_n\n%_0 0 0 1 K\n%_1 J 1 w\n%_3 3 m\n%_3 3 L\n%_S\n%_U\n%_9 () XW\nE\n%AI3_EndPattern\n";

/// The swatches in editing data: process, spot (one the art doesn't use), global, a gradient that
/// isn't defined and a pattern, a colour group, and versioned content read once.
const PALETTE: &str = "%AI5_BeginPalette\n0 0 Pb\n%AI17_Begin_Content_if_version_gt:24 15\n1 1 1 1 ([Registration]) 0 Xs\n([Registration])\nPc\n\
     0 0 0 1 k\n(Black)\nPc\n0.1 0.9 0.8 0 (Unused ink) 0 x\n(Unused ink)\nPc\n0 0 0 1 0 0 0 (Black global) 0 1 Xk\n(Black global)\nPc\n\
     Bb\n2 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n(G)\nPc\n(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n(Dots)\nPc\n\
     %AI17_Alternate_Content\n0 0 0 1 k\n(Black)\nPc\n%AI17_End_Versioned_Content\n1 (Greys) 1 Pg\n0 0 0 0.5 k\n(K50)\nPc\nPB\n%AI5_EndPalette\n";

/// The editing data of a file with every sort of object the layers reader knows, and some it doesn't.
fn editing_text() -> String {
    let square = |x: u32| format!("0 0 1 0 k\n{x} 10 m\n{} 10 L\n{} 14 L\n{x} 14 L\nf", x + 4, x + 4);
    let body = [
        "u".to_string(),
        square(10),
        "U".into(),
        "*u".into(),
        square(20),
        square(30),
        "*U".into(),
        "q".into(),
        square(40),
        "50 10 m 60 10 L 60 20 L 50 20 L h W f".into(),
        "Q".into(),
        "1 Xw".into(),
        "u".into(),
        "/AI11Text :\n0 /FreeUndo ,\n;".into(),
        "/AI11Text :\n1 /FreeUndo ,\n;\n1 (Style) XW".into(),
        "U".into(),
        "0 Xw".into(),
        "1 0 0 0 1 0 Bg".into(),
        "Bb\n1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n1 0 0 1 0 0 Xm\n0 0 0 0 0 0 Bm\n1 0 0 1 5 5 Bm\n0 BB".into(),
        "0 1 w 2 J 0 j 4 M [3 2]0 d 1 D".into(),
        "0 0 0 1 (Black global) 0.4 0 Xk".into(),
        "0 0 0 0 0 0 0 ([Registration]) 0.1 1 Xz".into(),
        "(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p".into(),
        square(60),
        // A 2 × 1 CMYK image with an alpha channel, its samples in hex.
        "%AI5_File:\n%AI5_BeginRaster\n() 1 XG\n/DeviceCMYK XN\n[ 4 0 0 4 10 30 ] 2 1 0 Xh\n[ 4 0 0 4 10 30 ] 0 0 2 1 2 1 8 4 1 0 0 0 4 4 0 0\n\
         %%BeginData: 18\rXI\nff000000000000ff80ff\n%%EndData\nXH\n%AI5_EndRaster\nN"
            .into(),
    ]
    .join("\n");
    let layer = |name: &str, visible: u8, body: &str| {
        format!("%AI5_BeginLayer\n{visible} 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n({name}) Ln\n{body}\nLB\n%AI5_EndLayer--\n")
    };
    format!(
        "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 100 100\n%%HiResBoundingBox: 0 0 100 100\n%AI3_Cropmarks: 0 0 100 100\n{PATTERN}{PALETTE}{}{}%%Trailer\n",
        layer("One", 1, &format!("{body}\n{}", layer("Sub", 1, &square(70)))),
        layer("Two", 0, &square(80))
    )
}

fn zstd(text: &str) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(text.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest)
}

/// An EPS drawing a square whose private data holds `editing`.
fn editing_eps(editing: &str) -> Vec<u8> {
    let lines: Vec<String> = ascii85(&zstd(editing)).as_bytes().chunks(60).map(|c| format!("%{}", String::from_utf8_lossy(c))).collect();
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n0 0 1 0 setcmykcolor 10 10 moveto 14 10 lineto 14 14 lineto closepath fill\nshowpage\n%%EOF\n%AI9_PrivateDataBegin\n%AI24_DataStream\n{}\n%AI9_PrivateDataEnd\n",
        lines.join("\n")
    )
    .into_bytes()
}

/// The fuzzed editing data, as it is, opens with its palette's swatches in its order and groups,
/// the spot colour the art doesn't use included (the gradient isn't defined: left out).
#[test]
fn the_editing_fixture_reads_its_palette() {
    let private = [b"%AI24_ZStandard_Data".as_slice(), &zstd(&editing_text())].concat();
    let (doc, warnings, layers) = vectorcraft_eps::layered_ai(&private, Document::new(100.0, 100.0), vec![], false, true, 1.0);
    assert!(layers, "{warnings:?}");
    let swatches: Vec<&str> = doc.swatches.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(swatches, ["[None]", "Black", "Unused ink", "Black global", "Dots"]);
    assert!(doc.swatch("Unused ink").is_some_and(|s| s.spot && s.global));
    let groups: Vec<(&str, usize)> = doc.swatch_groups.iter().map(|g| (g.name.as_str(), g.swatches.len())).collect();
    assert_eq!(groups, [("Greys", 1)]);
}

/// The fuzzed editing data, as it is, reads its CMYK image with its ink amounts and its alpha
/// channel as an opacity mask.
#[test]
fn the_editing_fixture_reads_its_cmyk_image_with_its_mask() {
    let private = [b"%AI24_ZStandard_Data".as_slice(), &zstd(&editing_text())].concat();
    let (doc, warnings, layers) = vectorcraft_eps::layered_ai(&private, Document::new(100.0, 100.0), vec![], false, true, 1.0);
    let mut found = vec![];
    doc.visit_images(|n, im| found.push((doc.images[&im.key].cmyk().map(|i| i.data), n.extras().mask.is_some())));
    assert!(layers && found.contains(&(Some(vec![255, 0, 0, 0, 0, 0, 0, 255]), true)), "{found:?} {warnings:?}");
}

/// The text document of a story of centred type with two styles, as an editing copy keeps it.
fn text_document_text() -> String {
    "/0 << /1 << /0 [ << /0 << /0 << /0 (Helvetica) >> >> >> ] >> /8 << /0 [ << /0 << /0 [ 0 0 ] /2 << /2 [ 1 0 0 1 3 4 ] >> >> >> ] >> >>\n\
     /1 << /1 [ << /0 << /0 (Hi\\rthere\\r) /5 << /0 [ << /0 << /0 << /0 () /5 << /0 2 >> /6 0 >> >> /1 9 >> ] >> \
     /6 << /0 [ << /0 << /0 << /0 () /5 0 /6 << /0 0 /1 14.0 /53 << /99 /CAITextPaint /0 << /0 2 /1 [ 1.0 0.0 1.0 0.0 0.0 ] >> >> >> >> >> /1 9 >> ] >> >> \
     /1 << /0 [ << /0 0 >> ] /2 [ << /99 /PC /6 [ << /99 /F /0 << /0 [ 8200.0 8180.0 ] >> /6 [ << /99 /R /6 [ << /99 /R /6 [ << /99 /L /6 [ \
     << /99 /S /0 << /0 [ -5.0 0.0 ] >> /15 << /0 3 >> >> ] >> << /99 /L /0 << /0 [ 0.0 16.8 ] >> /6 [ << /99 /S /0 << /0 [ -9.0 0.0 ] >> /15 << /0 6 >> >> ] >> ] >> ] >> ] >> ] >> ] >> >> ] /2 << /1 12.0 >> >>\n"
        .to_string()
}

/// The text document of area type threaded through two frames and of type on a path, as an
/// editing copy keeps it (frames on the canvas, y down).
fn frames_document_text() -> String {
    let area = |x0: f64, x1: f64| {
        let (y0, y1) = (8201.5, 8221.5);
        let corners = [(x0, y0), (x0, y1), (x1, y1), (x1, y0), (x0, y0)];
        let segments: Vec<String> = corners.windows(2).map(|w| format!("{0} {1} {0} {1} {2} {3} {2} {3}", w[0].0, w[0].1, w[1].0, w[1].1)).collect();
        format!("<< /0 << /0 [ 0 0 ] /1 << /0 [ {} ] >> /2 << /0 1 /7 18 >> >> >>", segments.join(" "))
    };
    let path = "<< /0 << /0 [ 0 0 ] /1 << /0 [ 8151.5 8171.5 8151.5 8171.5 8191.5 8161.5 8191.5 8171.5 8191.5 8171.5 8211.5 8181.5 8231.5 8171.5 8231.5 8171.5 ] >> /2 << /0 2 /6 [ 0.5 2.0 ] >> >> >>";
    let story = |text: &str, frames: &str, style: &str| {
        let n = text.chars().count() - text.matches('\\').count();
        format!(
            "<< /0 << /0 ({text}) /5 << /0 [ << /0 << /0 << /0 () /5 << /0 2 /1 4 /2 6 >> /6 0 >> >> /1 {n} >> ] >> \
             /6 << /0 [ << /0 << /0 << /0 () /5 0 /6 << {style} >> >> >> /1 {n} >> ] >> >> /1 << /0 [ {frames} ] >> >>"
        )
    };
    format!(
        "/0 << /1 << /0 [ << /0 << /0 << /0 (Helvetica) >> >> >> ] >> /8 << /0 [ {} {} {path} ] >> >>\n/1 << /1 [ {} {} ] /2 << /1 12.0 >> >>\n",
        area(8151.5, 8201.5),
        area(8211.5, 8241.5),
        story(
            "A story in two frames\\rand more\\r",
            "<< /0 0 >> << /0 1 >>",
            "/1 9.0 /8 50 /53 << /99 /CAITextPaint /0 << /0 1 /1 [ 1.0 1.0 0.0 0.0 ] >> >>"
        ),
        story("On a path\\r", "<< /0 2 >>", "/1 8.0"),
    )
}

/// The text objects of [`frames_document_text`]: story 0 in its two frames, story 1 in its one (a
/// text object names its frame among its story's).
const FRAME_OBJECTS: &str = "/AI11Text :\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\n/AI11Text :\n1 /FrameIndex ,\n0 /StoryIndex ,\n;\n/AI11Text :\n0 /FrameIndex ,\n1 /StoryIndex ,\n;\n";

/// The fuzzed fixture, as it is, reads as area type in two threaded frames and type on a path.
#[test]
fn the_frames_fixture_reads_as_threaded_area_type_and_type_on_a_path() {
    let l = vectorcraft_engine::cmd::fileio::load("x.eps", &text_eps_of(&frames_document_text(), FRAME_OBJECTS)).unwrap();
    let mut kinds = vec![];
    l.doc.walk(|n| {
        if let vectorcraft_doc::NodeKind::Text(t) = &n.kind {
            kinds.push(match t.kind {
                vectorcraft_doc::TextKind::Area { .. } => "area",
                vectorcraft_doc::TextKind::OnPath { .. } => "path",
                _ => "point",
            });
        }
    });
    assert_eq!(kinds, ["area", "area", "path"], "{:?}", l.warnings);
    assert_eq!(l.doc.text_threads.len(), 1);
}

/// The smallest failing input `eps_text_frames_never_panic` found: the first frame's bottom edge
/// has a control point at x = 8e115 (`8e0115`). That frame holds no text, and the story flows on
/// into the second frame.
#[test]
fn a_text_frame_with_a_control_point_at_8e115_opens_with_its_story_in_the_next_frame() {
    let text = mutate_text(&frames_document_text(), 1397, &[(191, '1'), (1586, 'e')]);
    assert!(text.contains(" 8e0115 "));
    let l = vectorcraft_engine::cmd::fileio::load("x.eps", &text_eps_of(&text, FRAME_OBJECTS)).unwrap();
    let mut texts = vec![];
    l.doc.walk(|n| {
        if let vectorcraft_doc::NodeKind::Text(t) = &n.kind {
            texts.push(t.plain_text());
        }
    });
    assert_eq!(texts, ["", "A story in two frames\nand more", "On a path"], "{:?}", l.warnings);
    survive("a frame with a control point at 8e115 pt", || Some(l.doc)).unwrap();
}

/// An EPS with a hidden text object whose story is in `document`.
fn text_eps(document: &str) -> Vec<u8> {
    text_eps_of(document, "/AI11Text :\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\n")
}

/// An EPS with the text `objects` on a hidden layer, their stories in `document`.
fn text_eps_of(document: &str, objects: &str) -> Vec<u8> {
    let lines: Vec<String> = ascii85(document.as_bytes()).as_bytes().chunks(60).map(|c| format!("%{}", String::from_utf8_lossy(c))).collect();
    let editing = format!(
        "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 100 100\n%%HiResBoundingBox: 0 0 100 100\n%AI3_Cropmarks: 0 0 100 100\n%AI3_TemplateBox: 50 50 50 50\n\
         %AI5_BeginLayer\n0 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(Spare) Ln\n{objects}LB\n%AI5_EndLayer--\n\
         %AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n{}\n%AI11_EndTextDocument\n%%Trailer\n",
        lines.join("\n")
    );
    editing_eps(&editing)
}

proptest! {
    #![proptest_config(config())]

    /// An Illustrator EPS whose text document is damaged or hostile: read as its type, or without it.
    #[test]
    fn eps_text_document_never_panics(cut in 0usize..1_500, edits in prop::collection::vec((0usize..1_500, prop::sample::select(vec!['0', '9', '-', '.', ' ', '\n', '(', ')', '/', '[', ']', '<', '>', '\\', 'e', '1'])), 0..12)) {
        let text = mutate_text(&text_document_text(), cut, &edits);
        let bytes = text_eps(&text);
        survive("mutated EPS text document", || vectorcraft_eps::import(&bytes).ok().map(|r| r.document))?;
    }

    /// The same for area type threaded through frames and type on a path, opened as a document
    /// (its threads flow).
    #[test]
    fn eps_text_frames_never_panic(cut in 0usize..2_500, edits in prop::collection::vec((0usize..2_500, prop::sample::select(vec!['0', '9', '-', '.', ' ', '\n', '(', ')', '/', '[', ']', '<', '>', '\\', 'e', '1', '2'])), 0..12)) {
        let text = mutate_text(&frames_document_text(), cut, &edits);
        let bytes = text_eps_of(&text, FRAME_OBJECTS);
        survive("mutated EPS text frames", || vectorcraft_engine::cmd::fileio::load("x.eps", &bytes).ok().map(|l| l.doc))?;
    }

    /// An Illustrator EPS whose editing data is damaged or hostile: read as its layers, or as its page.
    #[test]
    fn eps_editing_data_never_panics(cut in 0usize..3_000, edits in prop::collection::vec((0usize..3_000, prop::sample::select(vec!['0', '9', '-', '.', ' ', '\n', '(', ')', '/', ':', ';', '[', ']', '%', 'q', 'Q', 'W', 'u', 'U', 'L', 'k', 'x'])), 0..12)) {
        let text = mutate_text(&editing_text(), cut, &edits);
        let bytes = editing_eps(&text);
        survive("mutated EPS editing data", || vectorcraft_eps::import(&bytes).ok().map(|r| r.document))?;
    }

    /// A `.ai` whose editing data is damaged or hostile.
    #[test]
    fn ai_editing_data_never_panics(cut in 0usize..3_000, edits in prop::collection::vec((0usize..3_000, prop::sample::select(vec!['0', '9', '-', '.', ' ', '\n', '(', ')', '/', ':', ';', '[', ']', '%', 'q', 'Q', 'W', 'u', 'U', 'L', 'k', 'x'])), 0..12), damage in prop::collection::vec((0usize..4_000, any::<u8>()), 0..4)) {
        let text = mutate_text(&editing_text(), cut, &edits);
        let mut private = [b"%AI24_ZStandard_Data".as_slice(), &zstd(&text)].concat();
        for (at, b) in damage {
            let n = private.len();
            if let Some(x) = private.get_mut(at % n) {
                *x = b;
            }
        }
        survive("mutated .ai editing data", || Some(vectorcraft_eps::layered_ai(&private, Document::new(100.0, 100.0), vec![], false, false, 1.0).0))?;
    }

    /// A `.ai` whose variable library is damaged or hostile: the library is counted for the warning
    /// whether the layers are read or not (an inserted `Z` can make an operator the reader doesn't
    /// handle, and then the layers aren't read).
    #[test]
    fn ai_variable_library_never_panics(cut in 0usize..12_000, edits in prop::collection::vec((0usize..12_000, prop::sample::select(vec!['0', '9', ' ', '\n', '(', ')', '/', ':', ';', ',', '%', '_', 'Z'])), 0..12)) {
        use vectorcraft_testkit::ai;
        let library = ai::variable_library(&["Name", "Number"], &["One", "Two"]);
        let data = ai::editing_data(100.0, 100.0, &format!("{library}{}", ai::layer("L", "0 g\n10 10 m\n20 20 L\nS\n")));
        let text = mutate_text(&data, cut, &edits);
        let private = [b"%AI24_ZStandard_Data".as_slice(), &zstd(&text)].concat();
        survive("mutated .ai variable library", || Some(vectorcraft_eps::layered_ai(&private, Document::new(100.0, 100.0), vec![], false, false, 1.0).0))?;
    }
}

/// A `.ai` in PostScript form as other apps write it: the procsets that define its operators named,
/// not included, so its program can't be run and its layers are read from it.
const NAMED_PROLOG_AI: &str = "%!PS-Adobe-3.0\n%%BoundingBox: 0 0 200 100\n%%DocumentNeededResources: procset Adobe_packedarray 2.0 0\n%AI3_TemplateBox: 288 384 288 384\n%%EndComments\n\
    %%BeginProlog\n%%IncludeResource: procset Adobe_packedarray 2.0 0\nAdobe_packedarray /initialize get exec\n%%EndProlog\n%%BeginSetup\n%AI5_Begin_NonPrinting\nNp\n%AI5_End_NonPrinting--\n%%EndSetup\n\
    %AI5_BeginLayer\n1 1 1 1 0 0 -1 191 63 255 Lb\n(A::B) Ln\n0 A\n0 R\n0 0 0 1 K\n0 i 1 J 1 j 0.8 w 4 M [2 1]0 d\n0 D\n10 10 m\n90 10 L\n90 90 L\n10 90 L\n10 10 L\nS\n\
    u\n0.5 g\n20 20 m\n40 20 L\n40 40 L\nf\nU\n%AI5_BeginLayer\n0 1 1 1 0 0 3 0 0 0 Lb\n(Sub) Ln\n110 10 m\n190 90 L\nS\nLB\n%AI5_EndLayer--\nLB\n%AI5_EndLayer--\n\
    %%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n";

/// The fixture, as it is, opens with its layer and sublayer.
#[test]
fn the_named_prolog_fixture_opens_with_its_layers() {
    let l = vectorcraft_engine::cmd::fileio::load("x.ai", NAMED_PROLOG_AI.as_bytes()).unwrap();
    let names: Vec<_> = l.doc.layers.iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, [Some("A::B".to_string())], "{:?}", l.warnings);
    assert_eq!(l.doc.layers[0].children().unwrap().len(), 3);
}

proptest! {
    #![proptest_config(config())]

    /// A `.ai` in PostScript form whose prolog is named, not included, damaged: read from its
    /// layers, or refused.
    #[test]
    fn ps_form_ai_never_panics(cut in 0usize..1_200, edits in prop::collection::vec((0usize..1_200, prop::sample::select(vec!['0', '9', '-', '.', ' ', '\n', '(', ')', '/', '[', ']', '%', 'u', 'U', 'L', 'b', 'B', 'i', 'A', 'K'])), 0..12)) {
        let text = mutate_text(NAMED_PROLOG_AI, cut, &edits);
        survive("mutated PostScript-form .ai", || vectorcraft_engine::cmd::fileio::load("x.ai", text.as_bytes()).ok().map(|l| l.doc))?;
    }
}

/// The operators, operands and section comments of hostile editing data.
fn arb_ai_token() -> impl Strategy<Value = String> {
    prop_oneof![
        arb_num(),
        prop::sample::select(vec![
            "m",
            "l",
            "L",
            "c",
            "C",
            "v",
            "V",
            "y",
            "Y",
            "h",
            "H",
            "N",
            "n",
            "F",
            "f",
            "S",
            "s",
            "B",
            "b",
            "W",
            "(b) *",
            "u",
            "U",
            "*u",
            "*U",
            "q",
            "Q",
            "LB",
            "Lb",
            "Ln",
            "(name) Ln",
            "g",
            "G",
            "k",
            "K",
            "x",
            "X",
            "Xa",
            "XA",
            "Xx",
            "XX",
            "(Ink) 0.5 x",
            "O",
            "R",
            "XR",
            "w",
            "J",
            "j",
            "M",
            "d",
            "[6 3] 0 d",
            "[]0 d",
            "Xy",
            "2 0.5 1 1 1 Xy",
            "99 -4 Xy",
            "Xw",
            "1 Xw",
            "A",
            "1 A",
            "Ae",
            "XW",
            "1 (style) XW",
            "9 () XW",
            "Bd",
            "(G) 1 3 Bd",
            "Bs",
            "0 0 0 0 1 0 0 2 1 6 50 0 Bs",
            "BD",
            "Bb",
            "1 Bb",
            "BB",
            "2 BB",
            "1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg",
            "Bg",
            "Bm",
            "1e308 0 0 1e-308 0 0 Bm",
            "Bh",
            "XN",
            "/DeviceCMYK XN",
            "/DeviceGray XN",
            "[ 1 0 0 1 0 0 ] 0 0 2 2 2 2 8 3 1 0 1 0",
            "[ 1 0 0 1 0 0 ] 0 0 3 2 3 2 1 4 1 0 1 0",
            "[ 1 0 0 1 0 0 ] 0 0 99999 99999 99999 99999 1 1 0 0 0 0",
            "[",
            "]",
            "/Name",
            "(text)",
            "<ff00>",
            ":",
            ";",
            ",",
            "/ArtDictionary :",
            "/XMLUID : (A_x41_) ; (AI10_ArtUID) ,",
            "(n) /String (AIArtName) ,",
            "/Document :",
            "/Array :",
            "/Dictionary :",
            "0 0 /RealPoint (PositionPoint1) ,",
            "/AI11Text :",
            "0 /StoryIndex ,",
            "1 /FreeUndo ,",
            "6 () XW",
            "\n%_/AI11Text :\n%_0 /StoryIndex ,\n%_;\n",
            "/SymbolInstance :",
            "/Binary : /ASCII85Decode ,",
            "~>",
            "p",
            "P",
            "(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p",
            "(Dots) 0 0 1 1 0 0 0 0 0 [0 0 0 0 1e308 0] P",
            "Xk",
            "XK",
            "0 0 0 1 (Global) 0.4 0 Xk",
            "1 0.5 0 (Global) 0 1 XK",
            "E",
            "\n%AI3_BeginPattern: (Dots)\n(Dots) 0 0 6 6\n",
            "\n%AI3_EndPattern\n",
            "To",
            // Legacy type (version 7 §15): text objects, their paths, attributes and strings.
            "0 To",
            "1 To",
            "2 To",
            "1e308 To",
            "TO",
            "1 0 0 1 10 10 0 Tp",
            "0 0 0 0 1e308 -1e308 1e308 Tp",
            "-1 0 0 -1 5 5 99.5 Tp",
            "TP",
            "/_Helvetica 12 9 3 Tf",
            "/_ 1e308 Tf",
            "-5 Tf",
            "(H\\351llo\\r) Tx",
            "(\\r) TX",
            "() Tj",
            "(x) Tx",
            "7 Tr",
            "-1e308 Ts",
            "1e308 0 Tz",
            "1e308 Tt",
            "0 TA",
            "1e308 -1e308 Tl",
            "9 Ta",
            "1e308 -1e308 1e308 Ti",
            "0 1 Tk",
            "[39/quotesingle 128/Euro/eacute/nosuch 300/x TE",
            "[1e308/a -1/b TE",
            "\n%AI3_BeginEncoding: _Helvetica Helvetica\n[/_Helvetica/Helvetica 0 0 1 TZ\n%AI3_EndEncoding AdobeType\n",
            "\n%AI3_BeginEncoding:\n",
            "800 Ar",
            "frobnicate",
            "\n%AI5_BeginLayer\n",
            "\n%AI5_EndLayer--\n",
            "\n%_",
            "\n%AI5_BeginRaster\n",
            "\n%AI5_EndRaster\n",
            "\n%%BeginData: 12\rXI\n",
            "\n%%EndData\n",
            "\n%AI5_BeginGradient: (G)\n",
            "\n%AI14_BeginSymbol\n",
            "\n%AI10_EndSymbol\n",
            "\n%AI17_Begin_Content_if_version_gt:24 4\n",
            "\n%AI17_Alternate_Content\n",
            "\n%AI17_End_Versioned_Content\n",
            "\n%AI3_Cropmarks: 0 0 1e308 -1e308\n",
            "\n%AI5_ArtSize: 0 0\n",
            "\n%AI9_ColorModel: 2\n",
            "\n%AI5_BeginPlace\n",
            "\n%AI26_BeginPlacedObjectPreview\n",
            "\n%AI26_BeginPlacedObjectPreview\n2 1 8 32 17 abcdefgh",
            "\n%AI26_BeginPlacedObjectPreview\n4096 4096 1e9 32 17 ",
            "\n%AI26_EndPlacedObjectPreview\n",
            "[1 0 0 -1 0 0] 0 0 2 1 4 4 0 (a.png) `",
            "[1e308 0 0 1e308 0 0] 0 0 -1e308 1e308 4 4 0 (a.png) `",
            "`",
            "~",
            "\n%AI5_EndPlace\n",
            "Xm",
            "50 -50 -100 -100 5000 -2000 Xm",
            "0 0 0 0 1e308 -1e308 Xm",
            "Xd",
            "0 1 Xd",
            "1 0 Xd",
            "XD",
            "(00800080) Xt 0 XD",
            "X!",
            "/Mesh X!",
            "/End X!",
            "X#",
            "(DeviceRGB) /CS X#",
            "[0 0] /P X#",
            "[1 0 0 1 0 0 10 10 12 10 1 8 10 1] /N X#",
            "[1e308 -1e308 0] /N X#",
            "0 /E X#",
            "\n%AI9_BeginArtStyles\n",
            "\n%AI9_EndArtStyles\n",
            "/KnownStyle :",
            "(style) /Name ,",
            "/ActiveStyle :",
            "/CompoundFilter :",
            "/BasicFilter :",
            "(Stroke Style Filter) 0 0 /Filter ,",
            "(Fill Style Filter) 0 0 /Filter ,",
            "(Adobe Outer Glow) 1 0 /Filter ,",
            "(Adobe Drop Shadow) 1 0 /Filter ,",
            "1 /Visible ,",
            "2 /FillOrStroke ,",
            "1 /Bool (BSDashAdj) ,",
            "0.75 /Real (opac) ,",
            "1e308 /Real (blur) ,",
            "1 /Int (csrc) ,",
            "-1e308 /Real (dark) ,",
            "0 /Real ,",
            "; (BSVariableWidthTValuesArray) ,",
            "; (BSCenteredWidthsArray) ,",
            "/StrokeStyle : 0 R",
            "/FillStyle : 0 O",
            "/Def ;",
            "/Execution ;",
            "[5 5] 1e308 d",
            "-0.5 /Real ,",
            "1.5 /Real ,",
            "-2 /Real ,",
            "; (BSLeftWidthsArray) ,",
            "; (BSRightWidthsArray) ,",
            "[0.5 1e308 0 0 0 0 1 0 0 1] /N X#",
            "(DeviceGray) /CS X#",
            "\n%\n",
            "\n%AI5_EndPlace\n",
            "\n%%PageTrailer\n",
        ])
        .prop_map(str::to_string),
    ]
}

/// The testkit's sample editing data, compressed as older `.ai` files compress it.
fn ai_sample_compressed() -> Vec<u8> {
    [&b"%AI12_CompressedData"[..], &deflated(&vectorcraft_testkit::ai::sample_data())].concat()
}

/// `data` compressed (zlib).
fn deflated(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

proptest! {
    #![proptest_config(config())]

    /// Hostile editing data in Illustrator EPS and `.ai` files: operators in any order with any
    /// operands, sections and dictionaries left open, images of any size.
    #[test]
    fn ai_hostile_editing_data_never_panics(tokens in prop::collection::vec(arb_ai_token(), 0..80)) {
        use vectorcraft_testkit::ai;
        let data = ai::editing_data(200.0, 100.0, &format!("%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\n{}\nLB\n", tokens.join(" ")));
        survive("hostile editing data", || Some(vectorcraft_eps::layered_ai(data.as_bytes(), Document::new(200.0, 100.0), vec![], false, false, 1.0).0))?;
        survive("hostile editing data, type as outlines", || Some(vectorcraft_eps::layered_ai(data.as_bytes(), Document::new(200.0, 100.0), vec![], true, false, 1.0).0))?;
        survive("hostile editing data, trusted", || Some(vectorcraft_eps::layered_ai(data.as_bytes(), Document::new(200.0, 100.0), vec![], false, true, 1.0).0))?;
        // A PDF part read from a user space of 10 points a unit: the layers are read in its units and scaled.
        survive("hostile editing data in units of 10 points", || {
            Some(vectorcraft_eps::layered_ai(data.as_bytes(), Document::new(2000.0, 1000.0), vec![], false, false, 10.0).0)
        })?;
        let eps = ai::eps(data.as_bytes(), ai::page_ps());
        survive("hostile editing data in an EPS", || vectorcraft_engine::cmd::fileio::load("x.eps", &eps).ok().map(|l| l.doc))?;
        // An EPS whose page is its art's box, not its artboard: what prints outside it is compared too.
        let boxed = ai::eps(data.replace("%AI3_Cropmarks: 0 0 200 100", "%AI3_Cropmarks: -50 -50 300 200").as_bytes(), ai::page_ps());
        survive("hostile editing data in an EPS of its art's box", || vectorcraft_engine::cmd::fileio::load("x.eps", &boxed).ok().map(|l| l.doc))?;
    }

    /// The testkit's sample damaged, as it is and compressed, in EPS, `.ai` and PDF files.
    #[test]
    fn ai_mutated_sample_never_panics(cut in 0usize..4_000, edits in prop::collection::vec((0usize..4_000, any::<u8>()), 0..12)) {
        use vectorcraft_testkit::ai;
        static PLAIN: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        static PACKED: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        for (what, sample) in [("editing data", PLAIN.get_or_init(ai::sample_data)), ("compressed editing data", PACKED.get_or_init(ai_sample_compressed))] {
            let mut bytes = sample.clone();
            for &(at, b) in &edits {
                let n = bytes.len();
                bytes[at % n] = b;
            }
            bytes.truncate(cut.max(4));
            let file = ai::ai(&bytes, ai::page_pdf());
            survive(what, || vectorcraft_engine::cmd::fileio::load("x.ai", &file).ok().map(|l| l.doc))?;
            let file = ai::pdf(&bytes, ai::page_pdf());
            survive(what, || vectorcraft_engine::cmd::fileio::load("x.pdf", &file).ok().map(|l| l.doc))?;
            let file = ai::eps(&bytes, ai::page_ps());
            survive(what, || vectorcraft_engine::cmd::fileio::load("x.eps", &file).ok().map(|l| l.doc))?;
        }
    }
}

// ---------- font files a folder search reads ----------

/// A small font file holding what a folder search reads: an outline table's tag, the `name` and
/// `OS/2` tables of the bundled Source Sans 3 renamed "Findme Sans 3", and an `fvar` table with a
/// weight axis and three named instances (named by name ids 2, 1 and 4).
fn search_font() -> Vec<u8> {
    let font = vectorcraft_testkit::fonts::renamed("Findme Sans 3");
    let be32 = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().unwrap()) as usize;
    let table = |tag: &[u8; 4]| {
        let n = u16::from_be_bytes([font[4], font[5]]) as usize;
        let r = (0..n).map(|i| 12 + 16 * i).find(|&r| &font[r..r + 4] == tag).unwrap();
        font[be32(r + 8)..be32(r + 8) + be32(r + 12)].to_vec()
    };
    // fvar 1.0: the axis array at 16, one axis of 20 bytes, three instances of 8.
    let mut fvar = vec![];
    for v in [1u16, 0, 16, 2, 1, 20, 3, 8] {
        fvar.extend(v.to_be_bytes());
    }
    fvar.extend(b"wght");
    for v in [100i32, 400, 900] {
        fvar.extend((v << 16).to_be_bytes());
    }
    fvar.extend([0, 0, 1, 0]);
    for (name, weight) in [(2u16, 300i32), (1, 600), (4, 900)] {
        fvar.extend(name.to_be_bytes());
        fvar.extend(0u16.to_be_bytes());
        fvar.extend((weight << 16).to_be_bytes());
    }
    let tables: [(&[u8; 4], Vec<u8>); 4] = [(b"OS/2", table(b"OS/2")), (b"fvar", fvar), (b"glyf", vec![0; 4]), (b"name", table(b"name"))];
    let mut out = 0x0001_0000_u32.to_be_bytes().to_vec();
    for v in [tables.len() as u16, 0, 0, 0] {
        out.extend(v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend(*tag);
        for v in [0, offset as u32, data.len() as u32] {
            out.extend(v.to_be_bytes());
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// `font` as a collection of `faces % 5` faces that all read it, whose header gives `faces` as the
/// count.
fn search_collection(font: &[u8], faces: u32) -> Vec<u8> {
    let k = (faces % 5) as usize;
    let base = 12 + 4 * k;
    let mut out = b"ttcf".to_vec();
    out.extend(0x0001_0000_u32.to_be_bytes());
    out.extend(faces.to_be_bytes());
    for _ in 0..k {
        out.extend((base as u32).to_be_bytes());
    }
    // The tables' offsets count from the start of the collection.
    let mut f = font.to_vec();
    let n = u16::from_be_bytes([f[4], f[5]]) as usize;
    for i in 0..n {
        let at = 12 + 16 * i + 8;
        let offset = u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) + base as u32;
        f[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    }
    out.extend(f);
    out
}

/// The fonts a search for "Findme Sans 3" looks for: the face, and a style of the family as if the
/// family were installed.
fn findme_wanted() -> vectorcraft_text::WantedFonts {
    use vectorcraft_text::WantedFont;
    vectorcraft_text::WantedFonts::new(&[
        WantedFont { family: "Findme Sans 3".into(), style: "Regular".into(), installed: None },
        WantedFont { family: "FINDME SANS 3".into(), style: "Black".into(), installed: Some("Findme Sans 3".into()) },
        WantedFont { family: "SourceSans3-Regular".into(), style: "Bold".into(), installed: None },
    ])
}

/// Whether reading `bytes` as a font file a search reads panics.
fn search_reads(what: &str, bytes: &[u8]) -> Result<Vec<usize>, TestCaseError> {
    let path = vectorcraft_testkit::temp_dir("font-search-fuzz").join(format!("{:?}.ttc", std::thread::current().id()));
    std::fs::write(&path, bytes).unwrap();
    catch_quiet(|| findme_wanted().provided_by(&path)).map_err(|e| TestCaseError::fail(format!("{what}: a search reading the file panicked: {e}")))
}

/// The samples the fuzz properties mutate reach the matching: each provides the face it names.
#[test]
fn the_search_font_samples_are_read_whole() {
    let font = search_font();
    assert_eq!(search_reads("font", &font).unwrap(), [0, 2]);
    assert_eq!(search_reads("collection", &search_collection(&font, 3)).unwrap(), [0, 2]);
    // Two faces, and a header that gives seven.
    assert_eq!(search_reads("collection claiming more faces", &search_collection(&font, 7)).unwrap(), [0, 2]);
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn font_files_a_search_reads_never_panic(
        faces in prop::option::of(any::<u32>()),
        cut in prop::option::of(0usize..12_000),
        edits in prop::collection::vec((0usize..12_000, any::<u8>()), 0..12),
    ) {
        let font = search_font();
        let mut bytes = match faces {
            Some(n) => search_collection(&font, n),
            None => font,
        };
        for (at, b) in edits {
            let n = bytes.len();
            bytes[at % n] = b;
        }
        if let Some(cut) = cut {
            bytes.truncate(cut);
        }
        search_reads("mutated font file", &bytes)?;
    }
}

// ---------- scripts checked without running them ----------

/// A script written for these tests, with invented names: directives in both forms, member chains,
/// files, a dialog, menu commands, E4X, an operator and a message to another app.
const SCRIPT_SAMPLE: &str = concat!(
    "#target acme_2-12-en_US\n",
    "//@include \"lib/util.jsxinc\"\n",
    "#includepath \"lib;shared\"\n",
    "#targetengine session\n",
    "var doc = app.activeDocument;\n",
    "var w = new Window(\"dialog { text: 'Tidy' }\");\n",
    "var f = File.saveDialog('Save'); f.open('w'); f.write(doc.acmeName); f.close();\n",
    "app.executeMenuCommand('acmecopyall');\n",
    "var x = <root><item id=\"1\">{doc.acmeName}</item></root>;\n",
    "for each (var i in x..item) { $.writeln(i.@id); }\n",
    "Vec.prototype['+'] = function (o) { return o; };\n",
    "var bt = new BridgeTalk(); bt.target = 'acme'; bt.send();\n",
    "if (item.typename == 'AcmeShape') { alert(/a[/]b/g.test(s) ? 'yes' : \"no\"); }\n",
);

/// The pieces generated scripts are made of.
const SCRIPT_PIECES: &[&str] = &[
    "#target acme",
    "//@include 'a.jsxinc'",
    "#includepath 'x;..'",
    "#pragma",
    "app",
    "activeDocument",
    "selection",
    "var doc = ",
    ".",
    ".acmeItems",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    ";",
    "=",
    "'str'",
    "\"open",
    "/re[/]x/g",
    "/",
    "<a b=\"{v}\">",
    "</a>",
    "<!--",
    ".@",
    "..",
    "`t",
    "=>",
    "\n",
    "\r",
    "\u{2028}",
    "@JSXBIN@",
    "executeMenuCommand",
    "new File(p)",
    "Window(",
    "import",
    "/*",
    "\\",
    "0x1F",
    " ",
];

/// `script.check` with `p`, which mustn't panic, and whose lists keep to their limit.
fn checked(what: &str, p: Value) -> Result<(), TestCaseError> {
    let r = catch_quiet(|| no_panic(vectorcraft_engine::Session::new().execute("script.check", &p)))
        .map_err(|e| TestCaseError::fail(format!("{what}: script.check panicked: {e}")))?;
    if let Ok(r) = r {
        for list in [&r["targets"], &r["directives"], &r["includes"], &r["problems"], &r["dom"]["members"], &r["menuCommands"]["strings"]] {
            prop_assert!(list.as_array().is_some_and(|l| l.len() <= 1000), "{}: {}", what, list);
        }
    }
    Ok(())
}

/// The sample the properties mutate reaches every part of the check.
#[test]
fn the_script_sample_is_checked_whole() {
    let r = vectorcraft_engine::Session::new().execute("script.check", &json!({"data": SCRIPT_SAMPLE})).unwrap();
    assert_eq!(r["kind"], "script");
    assert_eq!(r["needs"], json!(["language", "dom", "files", "menuCommands", "dialogs", "hostOnly"]));
    assert_eq!(r["problems"], json!([]));
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn script_garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2_000), compiled in any::<bool>()) {
        let name = if compiled { "garbage.jsxbin" } else { "garbage.jsx" };
        checked("script garbage", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": name}))?;
    }

    #[test]
    fn generated_scripts_never_panic(pieces in prop::collection::vec(prop::sample::select(SCRIPT_PIECES.to_vec()), 0..120)) {
        checked("generated script", json!({"data": pieces.concat()}))?;
    }

    #[test]
    fn mutated_scripts_never_panic(cut in 0usize..1_200, edits in prop::collection::vec((0usize..1_200, any::<u8>()), 0..12)) {
        let mut bytes = SCRIPT_SAMPLE.as_bytes().to_vec();
        for (at, b) in edits {
            let n = bytes.len();
            bytes[at % n] = b;
        }
        bytes.truncate(cut);
        checked("mutated script", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "sample.jsx"}))?;
    }
}

// ---------- deep nesting ----------
//
// The tree is walked by recursion, so a file nested deeply enough overflowed the stack: an abort,
// which nothing catches. Every reader caps what it reads, and no document is deeper than
// `MAX_DEPTH`. These run on a thread with the stack of a Windows main thread (1 MB), where an
// overflow ends the test run.

use vectorcraft_doc::MAX_DEPTH;

/// `f` on a thread with a 1 MB stack.
fn on_a_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().stack_size(1 << 20).spawn(f).unwrap().join().unwrap();
}

/// Open `bytes` as the file `name`: an error, or a document within the cap that renders and
/// exports. Returns the document and the import's warnings.
fn open_deep(name: &str, bytes: &[u8]) -> Option<(Document, Vec<String>)> {
    let loaded = vectorcraft_engine::cmd::fileio::load(name, bytes).ok()?;
    assert!(!loaded.doc.too_deep(None), "{name}: deeper than the cap");
    let doc = loaded.doc.clone();
    survive(name, move || Some(doc)).unwrap();
    Some((loaded.doc, loaded.warnings))
}

fn merged(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains("nested more than"))
}

fn paths_in(doc: &Document) -> usize {
    let mut paths = 0;
    doc.walk(|n| paths += usize::from(n.path_data().is_some()));
    paths
}

const SVG_NS: &str = "xmlns=\"http://www.w3.org/2000/svg\"";

/// `n` groups in one another, each named (so it is kept) and with a rectangle beside the next.
fn nested_svg(n: usize) -> String {
    let open: String = (0..n).map(|i| format!("<g id=\"g{i}\"><rect width=\"5\" height=\"5\"/>")).collect();
    format!("<svg {SVG_NS} width=\"100\" height=\"100\">{open}<rect width=\"10\" height=\"10\"/>{}</svg>", "</g>".repeat(n))
}

#[test]
fn deeply_nested_svg_is_refused_or_merged() {
    on_a_small_stack(|| {
        // Within what the reader takes: opened, the groups past the cap merged into the last.
        let (doc, warnings) = open_deep("deep.svg", nested_svg(40).as_bytes()).expect("40 groups open");
        assert!(merged(&warnings), "{warnings:?}");
        assert_eq!(paths_in(&doc), 41, "no art is lost");
        let (_, warnings) = open_deep("shallow.svg", nested_svg(20).as_bytes()).expect("20 groups open");
        assert!(!merged(&warnings), "{warnings:?}");
        // As deep as the reader goes (on a stack of its own): opened and merged the same.
        let most = vectorcraft_svg::MAX_ELEMENT_DEPTH;
        let (doc, warnings) = open_deep("deepest.svg", nested_svg(most - 2).as_bytes()).expect("opens at the reader's cap");
        assert!(merged(&warnings), "{warnings:?}");
        assert_eq!(paths_in(&doc), most - 1, "no art is lost");
        // Past it: an error that says why, whatever the depth.
        for n in [most, 2 * most, 200_000] {
            let e = vectorcraft_engine::cmd::fileio::load("deep.svg", nested_svg(n).as_bytes()).err().expect("refused").to_string();
            assert!(e.contains(&format!("nested more than {most} deep")), "{n}: {e}");
        }
        // Unnamed groups of one object aren't kept, but usvg walks them all the same.
        let plain = format!(
            "<svg {SVG_NS} width=\"9\" height=\"9\">{}<rect width=\"1\" height=\"1\"/>{}</svg>",
            "<g>".repeat(2 * most),
            "</g>".repeat(2 * most)
        );
        assert!(open_deep("plain.svg", plain.as_bytes()).is_none());
        // A chain of `<use>`: shallow XML, a deep tree.
        let links: String = (0..3000).map(|i| format!("<g id=\"u{i}\"><use href=\"#u{}\"/></g>", i + 1)).collect();
        let chain = format!(
            "<svg {SVG_NS} width=\"9\" height=\"9\"><defs>{links}<rect id=\"u3000\" width=\"1\" height=\"1\"/></defs><use href=\"#u0\"/></svg>"
        );
        assert!(open_deep("chain.svg", chain.as_bytes()).is_none());
        // SVG images in SVG images, read on the same stack: each too deep, of a type that is read
        // as SVG too (`text/plain`), or each within the cap but not all together (#3562). What is
        // past the cap is left out.
        let images = [nested_images(3, 1000), nested_images(12, most - 4)];
        for (i, file) in images.iter().enumerate() {
            let (_, warnings) = open_deep("images.svg", file.as_bytes()).expect("opens");
            assert!(warnings.iter().any(|w| w.contains("SVG images were left out")), "{i}: {warnings:?}");
        }
        // Placed and pasted, as opened.
        let mut s = vectorcraft_engine::Session::new();
        s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        for svg in [nested_svg(40), nested_svg(2 * most)].iter().chain(&images) {
            let p = json!({"name": "deep.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())});
            let _ = no_panic(s.execute("file.place", &p));
            let _ = no_panic(s.execute("clipboard.importSvg", &json!({"svg": svg})));
            let _ = no_panic(s.execute("edit.paste", &json!({})));
            assert!(!s.doc().unwrap().doc.too_deep(None));
        }
    });
}

/// An SVG showing a chain of `levels` SVG images from `text/plain` data URLs, each `n` groups
/// deep around the next.
fn nested_images(levels: usize, n: usize) -> String {
    let image = |svg: &str| {
        format!("<image href=\"data:text/plain;base64,{}\" width=\"9\" height=\"9\"/>", vectorcraft_format::base64_encode(svg.as_bytes()))
    };
    let mut svg = nested_svg(0);
    for _ in 0..levels {
        svg = format!("<svg {SVG_NS} width=\"9\" height=\"9\">{}{}{}</svg>", "<g>".repeat(n), image(&svg), "</g>".repeat(n));
    }
    format!("<svg {SVG_NS} width=\"9\" height=\"9\">{}</svg>", image(&svg))
}

/// A page whose content is `content`, with `resources`.
fn deep_pdf(content: &str, resources: &str) -> Vec<u8> {
    handmade_pdf(content, resources, "[0 0 100 100]")
}

/// A stream object with the entries `dict`, its `data` compressed.
fn packed(dict: &str, data: &str) -> Vec<u8> {
    packed_as_is(dict, &deflated(data.as_bytes()))
}

/// A stream object with the entries `dict`, its data `z` compressed already.
fn packed_as_is(dict: &str, z: &[u8]) -> Vec<u8> {
    // (A length the test gives stands.)
    let length = if dict.contains("/Length") { String::new() } else { format!(" /Length {}", z.len()) };
    [format!("<< {dict} /Filter /FlateDecode{length} >>\nstream\n").as_bytes(), z, b"\nendstream"].concat()
}

/// A page whose content is `content`, with `resources` and the objects `more` (numbered from 5).
fn pdf_with(content: &str, resources: &str, more: &[Vec<u8>]) -> Vec<u8> {
    handmade_pdf_with(content, resources, "[0 0 100 100]", more)
}

/// Arrays in arrays, and dictionaries in dictionaries, far deeper than any reader goes: as the
/// operands of content.
fn nested_operands() -> [String; 2] {
    let (n, d) = (100_000, 50_000);
    [format!("BT {}{} TJ ET 0 0 5 5 re f", "[".repeat(n), "]".repeat(n)), format!("/P {}{} DP 0 0 5 5 re f", "<< /A ".repeat(d), ">> ".repeat(d))]
}

/// A page drawing through the compressed stream that `container` makes of each of
/// [`nested_operands`] (object 5), one file each.
fn nested_in(content: &str, resources: &str, container: impl Fn(&str) -> Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    nested_operands().iter().map(|deep| pdf_with(content, resources, &container(deep))).collect()
}

const FORM: &str = "/Type /XObject /Subtype /Form /BBox [0 0 100 100]";

/// A Type 3 font (object 5) whose glyph `a` is object 6.
fn type3_font(resources: &str) -> Vec<u8> {
    format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 10 10] /FontMatrix [0.1 0 0 0.1 0 0] /CharProcs << /a 6 0 R >> \
         /Encoding << /Type /Encoding /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [10] /Resources {resources} >>"
    )
    .into_bytes()
}

/// A page whose box is object 6, kept with object 7 (a number) in a compressed object stream
/// (object 5, with the entries `stream`): a file with a cross-reference stream, as it must be.
fn pdf_with_object_stream(boxed: &str, stream: &str) -> Vec<u8> {
    let content = "0 0 5 5 re f";
    let inside = format!("{boxed} 77");
    let index = format!("6 0 7 {} ", boxed.len() + 1);
    let objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox 6 0 R /Contents 4 0 R /Resources << >> >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1).into_bytes(),
        packed(&format!("/Type /ObjStm /N 2 /First {} {stream}", index.len()), &format!("{index}{inside}")),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    // One entry an object: its kind, then where it is (an offset, or the stream it is in and its
    // place there).
    let mut entries = vec![[0u8, 0, 0, 0, 0, 255, 255]];
    let at = |kind: u8, a: usize, b: u16| {
        let (a, b) = ((a as u32).to_be_bytes(), b.to_be_bytes());
        [kind, a[0], a[1], a[2], a[3], b[0], b[1]]
    };
    for (i, o) in objs.iter().enumerate() {
        entries.push(at(1, out.len(), 0));
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    entries.extend([at(2, 5, 0), at(2, 5, 1), at(1, out.len(), 0)]);
    let xref = out.len();
    out.extend_from_slice(format!("8 0 obj\n<< /Type /XRef /Size 9 /Root 1 0 R /W [1 4 2] /Length {} >>\nstream\n", entries.len() * 7).as_bytes());
    out.extend_from_slice(&entries.concat());
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

/// `n` objects from object 5 on, each made by `link` of the number of the next, then `last`.
fn chain(n: usize, link: impl Fn(usize) -> String, last: &str) -> Vec<Vec<u8>> {
    (0..n).map(|i| link(i + 6).into_bytes()).chain([last.as_bytes().to_vec()]).collect()
}

/// A page painting with the shading whose function is object 5.
fn pdf_with_function(more: &[Vec<u8>]) -> Vec<u8> {
    let shading = "/Shading << /S << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function 5 0 R >> >>";
    pdf_with("/S sh", &format!("<< {shading} >>"), more)
}

/// A page filling in the colour space that is object 5.
fn pdf_with_color_space(more: &[Vec<u8>]) -> Vec<u8> {
    pdf_with("/C cs 0 sc 0 0 5 5 re f /C CS 0 SC 0 0 5 5 re S", "<< /ColorSpace << /C 5 0 R >> >>", more)
}

/// Every hostile PDF here opens as an error or as a document within the cap, on a small stack.
fn open_each(name: &str, files: impl IntoIterator<Item = Vec<u8>> + Send + 'static) {
    let name = name.to_string();
    on_a_small_stack(move || {
        for (i, bytes) in files.into_iter().enumerate() {
            open_deep(&format!("{name}-{i}.pdf"), &bytes);
            // As `.ai` (the PDF part of one) and placed, too.
            open_deep(&format!("{name}-{i}.ai"), &bytes);
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
            let _ = no_panic(s.execute("file.place", &json!({"name": "deep.pdf", "dataBase64": vectorcraft_format::base64_encode(&bytes)})));
            assert!(!s.doc().unwrap().doc.too_deep(None));
        }
    });
}

// Nesting where only the reader of the decoded data sees it: inside each kind of compressed
// stream a page draws through.

#[test]
fn pdf_nesting_inside_a_compressed_form() {
    open_each("form", nested_in("/F Do", "<< /XObject << /F 5 0 R >> >>", |deep| vec![packed(FORM, deep)]));
}

#[test]
fn pdf_nesting_inside_a_compressed_pattern() {
    let tile = "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>";
    open_each("pattern", nested_in("/Pattern cs /P scn 0 0 50 50 re f", "<< /Pattern << /P 5 0 R >> >>", |deep| vec![packed(tile, deep)]));
}

#[test]
fn pdf_nesting_inside_a_compressed_glyph() {
    let glyph = |deep: &str| vec![type3_font("<< >>"), packed("", &format!("10 0 d0 {deep}"))];
    open_each("glyph", nested_in("BT /F1 12 Tf 10 10 Td (aa) Tj ET", "<< /Font << /F1 5 0 R >> >>", glyph));
}

#[test]
fn pdf_nesting_inside_a_compressed_soft_mask() {
    let group = format!("{FORM} /Group << /S /Transparency /CS /DeviceGray >>");
    let state = "<< /ExtGState << /G << /Type /ExtGState /SMask << /S /Luminosity /G 5 0 R >> >> >> >>";
    open_each("mask", nested_in("/G gs 0 0 50 50 re f", state, move |deep| vec![packed(&group, deep)]));
}

#[test]
fn pdf_nesting_inside_a_compressed_annotation() {
    let annots = "<< >> /Annots [<< /Type /Annot /Subtype /Square /Rect [0 0 50 50] /AP << /N 5 0 R >> >>]";
    open_each("annotation", nested_in("0 0 5 5 re f", annots, |deep| vec![packed(FORM, deep)]));
}

#[test]
fn pdf_nesting_inside_compressed_page_content() {
    let files = nested_operands().map(|deep| {
        let mut objs: Vec<Vec<u8>> = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"].map(Vec::from).into();
        objs.push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents [4 0 R 4 0 R] /Resources << >> >>".to_vec());
        objs.push(packed("", &deep));
        pdf_of(&objs)
    });
    open_each("content", files);
}

#[test]
fn pdf_nesting_inside_an_object_stream() {
    let (n, d) = (100_000, 50_000);
    open_each(
        "objects",
        [
            // A plain one first: the file is read as it should be.
            pdf_with_object_stream("[0 0 100 100]", ""),
            pdf_with_object_stream(&format!("{}0 0 100 100{}", "[".repeat(n), "]".repeat(n)), ""),
            pdf_with_object_stream(&format!("{}{}", "<< /A ".repeat(d), ">> ".repeat(d)), ""),
            // Its length is an object kept in the stream itself.
            pdf_with_object_stream("[0 0 100 100]", "/Length 7 0 R"),
            // It extends itself.
            pdf_with_object_stream("[0 0 100 100]", "/Extends 5 0 R"),
        ],
    );
    let plain = vectorcraft_engine::cmd::fileio::load("plain.pdf", &pdf_with_object_stream("[0 0 120 80]", "")).expect("an object stream is read");
    assert_eq!(plain.doc.artboards[0].rect.width(), 120.0);
}

// What refers to itself, or to the next of its kind without end.

#[test]
fn pdf_page_trees_that_hold_themselves() {
    let page = |parent: usize| format!("<< /Type /Page /Parent {parent} 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << >> >>");
    let content = "<< /Length 13 >>\nstream\n0 0 5 5 re f\nendstream";
    let own = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [2 0 R 3 0 R 2 0 R] /Count 1 >>".to_string(),
        page(2),
        content.to_string(),
    ];
    // Thousands of nodes, each holding the next; the page is the last.
    let n = 30_000;
    let mut deep = vec!["<< /Type /Catalog /Pages 5 0 R >>".to_string(), "null".to_string(), "null".to_string(), content.to_string()];
    deep.extend((0..n).map(|i| format!("<< /Type /Pages /Kids [{} 0 R] /Count 1 >>", i + 6)));
    deep.push(page(n + 4));
    // Two nodes holding each other.
    let pair = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [5 0 R] /Count 1 >>".to_string(),
        page(5),
        content.to_string(),
        "<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 1 >>".to_string(),
    ];
    let bytes = |objs: &[String]| pdf_of(&objs.iter().map(|o| o.clone().into_bytes()).collect::<Vec<_>>());
    open_each("pages", [bytes(&own), bytes(&deep), bytes(&pair)]);
}

#[test]
fn pdf_functions_that_hold_themselves() {
    let stitch = |next: usize| format!("<< /FunctionType 3 /Domain [0 1] /Functions [{next} 0 R] /Bounds [] /Encode [0 1] >>");
    let ramp = "<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>";
    let calculator = "/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]";
    let n = 100_000;
    open_each(
        "functions",
        [
            pdf_with_function(&[stitch(5).into_bytes()]),
            pdf_with_function(&[stitch(6).into_bytes(), stitch(5).into_bytes()]),
            pdf_with_function(&chain(20_000, stitch, ramp)),
            // A calculator function of procedures in procedures.
            pdf_with_function(&[packed(calculator, &format!("{}{}", "{".repeat(n), "}".repeat(n)))]),
            pdf_with_function(&[packed(calculator, &format!("{{ {} 0 0 0 {} }}", "true {".repeat(n), "} if ".repeat(n)))]),
        ],
    );
}

#[test]
fn pdf_color_spaces_that_hold_themselves() {
    let indexed = |next: usize| format!("[/Indexed {next} 0 R 1 <000000ffffff>]");
    let tint = "<< /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 0] /N 1 >>";
    let profile = |alternate: &str| format!("<< /N 3 /Alternate {alternate} /Length 4 >>\nstream\njunk\nendstream").into_bytes();
    open_each(
        "spaces",
        [
            pdf_with_color_space(&[indexed(5).into_bytes()]),
            pdf_with_color_space(&[indexed(6).into_bytes(), indexed(5).into_bytes()]),
            pdf_with_color_space(&chain(20_000, indexed, "/DeviceRGB")),
            pdf_with_color_space(&[b"[/Separation /Ink 5 0 R 6 0 R]".to_vec(), tint.as_bytes().to_vec()]),
            pdf_with_color_space(&[b"[/DeviceN [/Ink] 5 0 R 6 0 R]".to_vec(), tint.as_bytes().to_vec()]),
            pdf_with_color_space(&[b"[/Pattern 5 0 R]".to_vec()]),
            pdf_with_color_space(&[b"[/ICCBased 6 0 R]".to_vec(), profile("5 0 R")]),
            pdf_with_color_space(&[b"[/ICCBased 6 0 R]".to_vec(), profile("[/ICCBased 6 0 R]")]),
            // As the space of a shading and of an image.
            pdf_with(
                "/S sh",
                "<< /Shading << /S << /ShadingType 2 /ColorSpace 5 0 R /Coords [0 0 9 0] /Function 6 0 R >> >> >>",
                &[indexed(5).into_bytes(), tint.as_bytes().to_vec()],
            ),
            pdf_with(
                "q 50 0 0 50 0 0 cm /I Do Q",
                "<< /XObject << /I 6 0 R >> >>",
                &[
                    indexed(5).into_bytes(),
                    b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace 5 0 R /Length 1 >>\nstream\n0\nendstream"
                        .to_vec(),
                ],
            ),
        ],
    );
}

/// Spaces past what a stream may decode to (256 MB), compressed (about 260 kB).
fn spaces_past_the_cap() -> Vec<u8> {
    use std::io::Write as _;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    for _ in 0..257 {
        e.write_all(&[b' '; 1 << 20]).unwrap();
    }
    e.finish().unwrap()
}

/// [`calculator`] of [`spaces_past_the_cap`], made once.
fn spaces_past_the_cap_once() -> &'static str {
    static FUNCTION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FUNCTION.get_or_init(|| calculator(&spaces_past_the_cap(), ""))
}

/// A calculator function with the deflated program `z` and the decode parameters `dp`, to write
/// inside another object. Its data is in hexadecimal digits with spaces between them, so the
/// object around it still reads.
fn calculator(z: &[u8], dp: &str) -> String {
    let data: String = z.iter().map(|b| format!("{:X} {:X} ", b >> 4, b & 15)).collect();
    let dict = format!("/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1] /Filter [/ASCIIHexDecode /FlateDecode] {dp}");
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

// Streams the reader would fail on (#1700, #1701): it applies a predictor before checking its
// parameters (`Columns 0` divides by zero, a huge `Columns` allocates a row that large), and
// inflates without a ceiling.

#[test]
fn pdf_streams_the_reader_would_fail_on_are_refused() {
    let zero = "/DecodeParms << /Predictor 2 /Columns 0 >>";
    let huge = "/DecodeParms << /Predictor 12 /Colors 4 /Columns 99999999999 >>";
    // An inline image whose data is deflated, written in hexadecimal so the content stays text.
    let inline = |dp: &str| {
        let data: String = deflated(&[0; 8]).iter().map(|b| format!("{b:02x}")).collect();
        format!("q 10 0 0 10 0 0 cm BI /W 2 /H 2 /CS /G /BPC 8 /F [/AHx /Fl] /DP [null {dp}] ID {data}> EI Q")
    };
    let form = |dict: &str, content: &str| pdf_with("/F Do", "<< /XObject << /F 5 0 R >> >>", &[packed(&format!("{FORM} {dict}"), content)]);
    let held = |object: String| pdf_with("0 0 5 5 re f", "<< >>", &[object.into_bytes()]);
    let shaded = |function: &str| {
        let shading = format!("<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function {function} >>");
        pdf_with("/S sh", "<< /Shading << /S 5 0 R >> >>", &[shading.into_bytes()])
    };
    let spaces = calculator(&spaces_past_the_cap(), "");
    let program = deflated(b"{ dup 1 exch sub 0 }");
    let files = [
        ("a form with Columns 0", form(zero, "0 0 5 5 re f")),
        ("a form with a huge Columns", form(huge, "0 0 5 5 re f")),
        ("an inline image with Columns 0", deep_pdf(&inline("<< /Predictor 2 /Columns 0 >>"), "<< >>")),
        ("an inline image with a huge Columns", deep_pdf(&inline("<< /Predictor 15 /Columns 99999999999 >>"), "<< >>")),
        ("an inline image in a compressed form", form("", &inline("<< /Predictor 2 /Columns 0 >>"))),
        ("an object stream with Columns 0", pdf_with_object_stream("[0 0 100 100]", zero)),
        ("a form of spaces past the cap", pdf_with("/F Do", "<< /XObject << /F 5 0 R >> >>", &[packed_as_is(FORM, &spaces_past_the_cap())])),
        ("a function of spaces past the cap in an array", held(format!("[{spaces}]"))),
        ("a function of spaces past the cap in a dictionary", held(format!("<< /K {spaces} >>"))),
        ("a function of spaces past the cap in a shading", shaded(&spaces)),
        ("a function with Columns 0 in an array", held(format!("[{}]", calculator(&program, "/DecodeParms [null << /Predictor 2 /Columns 0 >>]")))),
    ];
    on_a_small_stack(move || {
        for (what, bytes) in &files {
            for name in ["hostile.pdf", "hostile.ai"] {
                let e = vectorcraft_engine::cmd::fileio::load(name, bytes).err().map(|e| e.to_string());
                assert!(e.as_deref().is_some_and(|e| e.contains("would decode to more than 256 MB")), "{what} as {name}: {e:?}");
            }
        }
    });
    // Within range, the same files open.
    let fine = "/DecodeParms << /Predictor 1 /Columns 0 >>";
    let function = calculator(&program, "");
    for bytes in [form(fine, "0 0 5 5 re f"), deep_pdf(&inline("<< /Predictor 1 >>"), "<< >>"), held(format!("[{function}]")), shaded(&function)] {
        vectorcraft_engine::cmd::fileio::load("fine.pdf", &bytes).expect("opens");
    }
}

#[test]
fn pdf_art_that_draws_itself() {
    let own = "<< /XObject << /F 5 0 R >> /Pattern << /P 5 0 R >> /Font << /F1 5 0 R >> /ExtGState << /G << /SMask << /S /Alpha /G 5 0 R >> >> >> >>";
    let tile = format!("/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources {own}");
    let group = format!("{FORM} /Group << /S /Transparency >> /Resources {own}");
    open_each(
        "itself",
        [
            // A form drawing itself twice, a pattern painting its own tile, a glyph showing
            // itself, and a mask masked by itself.
            pdf_with("/F Do", own, &[packed(&format!("{FORM} /Resources {own}"), "0 0 5 5 re f /F Do 1 0 0 1 3 3 cm /F Do")]),
            pdf_with("/Pattern cs /P scn 0 0 50 50 re f", own, &[packed(&tile, "/Pattern cs /P scn 0 0 5 5 re f")]),
            pdf_with("BT /F1 12 Tf (a) Tj ET", own, &[type3_font(own), packed("", "10 0 d0 0 0 5 5 re f BT /F1 1 Tf (a) Tj ET")]),
            pdf_with("/G gs 0 0 50 50 re f", own, &[packed(&group, "/G gs 0 0 5 5 re f")]),
            // An image that is its own mask, and objects that are only a reference to each other.
            pdf_with("q 50 0 0 50 0 0 cm /F Do Q", own, &[
                b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /SMask 5 0 R /Length 1 >>\nstream\n0\nendstream".to_vec(),
            ]),
            handmade_pdf_with("0 0 5 5 re f", "5 0 R", "6 0 R", &[b"6 0 R".to_vec(), b"5 0 R".to_vec()]),
            handmade_pdf_with("0 0 5 5 re f", "<< >>", "[0 0 9 9]", &[b"<< /Length 5 0 R >>\nstream\nx\nendstream".to_vec()]),
        ],
    );
}

/// An EPS whose program is `body`.
fn deep_eps(body: &str) -> Vec<u8> {
    format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{body}\nshowpage\n%%EOF\n").into_bytes()
}

/// A test for each file: it opens as an error or as a document within the cap, on a small stack.
macro_rules! deep_files {
    ($($test:ident: $name:literal, $bytes:expr;)*) => {$(
        #[test]
        fn $test() {
            on_a_small_stack(|| {
                open_deep($name, &$bytes);
            });
        }
    )*};
}

deep_files! {
    // PDF: clips inside clips (saved states never restored); arrays in arrays as an operand and as
    // an object; dictionaries in dictionaries as a resource and inline; marked content never ended.
    pdf_clips_in_clips: "clips.pdf", deep_pdf(&format!("{} 0 0 5 5 re f", "q 0 0 50 50 re W n ".repeat(20_000)), "<< >>");
    pdf_arrays_in_an_operand: "arrays.pdf", deep_pdf(&format!("BT {}{} TJ ET 0 0 5 5 re f", "[".repeat(100_000), "]".repeat(100_000)), "<< >>");
    pdf_arrays_in_an_object: "boxes.pdf", handmade_pdf("0 0 5 5 re f", "<< >>", &format!("{}0 0 100 100{}", "[".repeat(100_000), "]".repeat(100_000)));
    pdf_dictionaries_in_resources: "dicts.pdf", deep_pdf("0 0 5 5 re f", &format!("{}{}", "<< /A ".repeat(50_000), ">> ".repeat(50_000)));
    pdf_dictionaries_in_content: "inline.pdf", deep_pdf(&format!("/P {}{} DP 0 0 5 5 re f", "<< /A ".repeat(50_000), ">> ".repeat(50_000)), "<< >>");
    pdf_marked_content_never_ended: "marked.pdf", deep_pdf(&format!("{} 0 0 5 5 re f", "/Span << /MCID 0 >> BDC ".repeat(50_000)), "<< >>");
    // PostScript: clips inside clips; procedures in procedures; arrays in arrays; a procedure that
    // calls itself; dictionaries never ended.
    eps_clips_in_clips: "clips.eps", deep_eps(&format!("{} 0 0 5 5 rectfill", "gsave 0 0 50 50 rectclip ".repeat(20_000)));
    eps_procedures_in_procedures: "procs.eps", deep_eps(&format!("{}{} 0 0 5 5 rectfill", "{".repeat(100_000), "}".repeat(100_000)));
    eps_arrays_in_arrays: "arrays.eps", deep_eps(&format!("{}{} pop 0 0 5 5 rectfill", "[".repeat(100_000), "]".repeat(100_000)));
    eps_a_procedure_calling_itself: "runaway.eps", deep_eps("/f { 0 0 5 5 rectfill f } def f");
    eps_dictionaries_never_ended: "dicts.eps", deep_eps(&format!("{} 0 0 5 5 rectfill", "1 dict begin ".repeat(100_000)));
}

#[test]
fn deeply_nested_dxf_blocks_are_read_within_the_cap() {
    on_a_small_stack(|| {
        // Block `B0` inserts `B1`, which inserts `B2`, and so on; the last one holds a line.
        let pair = |c: i32, v: &str| (c, v.to_string());
        let mut pairs = vec![pair(0, "SECTION"), pair(2, "BLOCKS")];
        for i in 0..2_000 {
            pairs.extend([pair(0, "BLOCK"), pair(2, &format!("B{i}")), pair(10, "0"), pair(20, "0")]);
            if i + 1 < 2_000 {
                pairs.extend([pair(0, "INSERT"), pair(2, &format!("B{}", i + 1)), pair(10, "1"), pair(20, "1")]);
            } else {
                pairs.extend([pair(0, "LINE"), pair(10, "0"), pair(20, "0"), pair(11, "5"), pair(21, "5")]);
            }
            pairs.push(pair(0, "ENDBLK"));
        }
        pairs.extend([pair(0, "ENDSEC"), pair(0, "SECTION"), pair(2, "ENTITIES"), pair(0, "INSERT"), pair(2, "B0"), pair(10, "0"), pair(20, "0")]);
        pairs.extend([pair(0, "LINE"), pair(10, "0"), pair(20, "0"), pair(11, "9"), pair(21, "9"), pair(0, "ENDSEC"), pair(0, "EOF")]);
        open_deep("blocks.dxf", dxf_text(&pairs).as_bytes()).expect("opens");
    });
}

#[test]
fn dxf_blocks_inserting_themselves_are_left_out() {
    on_a_small_stack(|| {
        let pair = |c: i32, v: &str| (c, v.to_string());
        // A line, then an insert of `inserts`.
        let block = |name: &str, inserts: &str| {
            let line = [pair(0, "LINE"), pair(10, "0"), pair(20, "0"), pair(11, "5"), pair(21, "5")];
            let insert = [pair(0, "INSERT"), pair(2, inserts), pair(10, "1"), pair(20, "1"), pair(0, "ENDBLK")];
            [pair(0, "BLOCK"), pair(2, name), pair(10, "0"), pair(20, "0")].into_iter().chain(line).chain(insert).collect::<Vec<_>>()
        };
        let mut pairs = vec![pair(0, "SECTION"), pair(2, "BLOCKS")];
        pairs.extend([block("SELF", "SELF"), block("A", "B"), block("B", "A")].concat());
        pairs.extend([pair(0, "ENDSEC"), pair(0, "SECTION"), pair(2, "ENTITIES")]);
        for name in ["SELF", "A"] {
            pairs.extend([pair(0, "INSERT"), pair(2, name), pair(10, "0"), pair(20, "0")]);
        }
        pairs.extend([pair(0, "ENDSEC"), pair(0, "EOF")]);
        let (_, warnings) = open_deep("self.dxf", dxf_text(&pairs).as_bytes()).expect("opens");
        assert!(warnings.iter().any(|w| w.contains("inside themselves")), "{warnings:?}");
    });
}

#[test]
fn deeply_nested_affinity_groups_are_read_within_the_cap() {
    use vectorcraft_affinity::synth::{self, F, Method, tag};
    // Groups in groups: within what the reader takes, past its cap on layers (128), past its cap
    // on the stream's own nesting (384), and far past both.
    let files: Vec<Vec<u8>> = [20u32, 120, 180, 370, 2000]
        .into_iter()
        .map(|levels| {
            let group = |id: u32, fields| F::Def(id, vec![tag(b"Grup")], fields);
            let inner = (0..levels).fold(group(10, vec![]), |inner, i| group(11 + i, vec![(tag(b"Chld"), F::Shared(vec![inner]))]));
            let spread = F::Def(2, vec![tag(b"Sprd")], vec![(tag(b"Chld"), F::Shared(vec![inner]))]);
            let stream = synth::stream(&[(tag(b"DocR"), F::Def(1, vec![tag(b"DocN")], vec![(tag(b"Chld"), F::Shared(vec![spread]))]))]);
            affinity_native(&stream, Method::Stored)
        })
        .collect();
    on_a_small_stack(move || {
        for bytes in &files {
            open_deep("deep.afdesign", bytes);
        }
    });
}

#[test]
fn metafiles_saving_states_without_end_are_read_within_the_cap() {
    on_a_small_stack(|| {
        // A saved state, a clip and a rectangle, over and over (nothing is ever restored).
        let longs = |v: [i32; 4]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let emf: Vec<(u32, Vec<u8>)> = (0..50_000).flat_map(|_| [(33, vec![]), (30, longs([0, 0, 500, 500])), (43, longs([0, 0, 50, 50]))]).collect();
        open_deep("deep.emf", &emf_of(&emf));
        let shorts = |v: [i16; 4]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let wmf: Vec<(u16, Vec<u8>)> =
            (0..20_000).flat_map(|_| [(0x001E, vec![]), (0x0416, shorts([500, 500, 0, 0])), (0x041B, shorts([50, 50, 0, 0]))]).collect();
        open_deep("deep.wmf", &wmf_of(&wmf));
    });
}

#[test]
fn deeply_nested_plugin_manifests_and_output_are_errors() {
    on_a_small_stack(|| {
        // (A manifest may be 64 kB.)
        let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(vectorcraft_plugins::Manifest::parse(deep(30_000).as_bytes()).is_err());
        assert!(vectorcraft_plugins::Manifest::parse(format!("{{\"params\": {}}}", deep(30_000)).as_bytes()).is_err());
        let deep = deep(100_000);
        for output in [deep.clone(), format!("{{\"objects\": {deep}}}"), format!("{{\"objects\": [{{\"kind\": \"path\", \"path\": {deep}}}]}}")] {
            assert!(vectorcraft_plugins::objects::decode(output.as_bytes()).is_err());
        }
    });
}

/// A document whose symbol `s0` and pattern `p0` are each a line stroked with the brush `loop`,
/// with an instance of the symbol and a square painted with the pattern on the layer. The brush
/// is the `kind`th of those that hold art (art, scatter, pattern), and its art is the `art`th of:
/// an instance of the symbol; that instance inside as many groups as a file holds; a square
/// painted with the pattern. So what the brush draws is drawn with the brush.
fn brushed_document(kind: usize, art: usize) -> Document {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind, PatternBrush, Scatter};
    use vectorcraft_color::Paint;
    use vectorcraft_doc::{Appearance, AppearanceItem, Node, NodeKind, PatternDef, Symbol};
    let mut doc = nested_document(1);
    let square = || vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 60.0, 20.0));
    let brushed = |doc: &mut Document| {
        let mut look = Appearance::basic(Paint::None, Paint::solid(vectorcraft_color::Color::rgb8(0, 0, 0)), 4.0);
        for item in &mut look.items {
            if let AppearanceItem::Stroke(s) = item {
                s.brush = Some("loop".into());
            }
        }
        Node::path(doc.alloc_id(), square(), look)
    };
    let instance =
        |doc: &mut Document| Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "s0".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let painted = |doc: &mut Document| {
        let fill = Paint::Pattern { pattern: "p0".into(), xf: vectorcraft_geom::Affine::IDENTITY };
        Node::path(doc.alloc_id(), square(), Appearance::basic(fill, Paint::None, 0.0))
    };
    let (in_symbol, in_pattern) = (brushed(&mut doc), brushed(&mut doc));
    doc.symbols.push(Symbol { name: "s0".into(), art: std::sync::Arc::new(in_symbol) });
    doc.patterns.push(PatternDef::new("p0", vec![std::sync::Arc::new(in_pattern)]));
    let art = match art {
        0 => instance(&mut doc),
        1 => (0..MAX_DEPTH + 4).fold(instance(&mut doc), |inner, _| Node::group(doc.alloc_id(), vec![std::sync::Arc::new(inner)])),
        _ => painted(&mut doc),
    };
    let kind = match kind {
        0 => BrushKind::Art(ArtBrush { art, ..Default::default() }),
        1 => BrushKind::Scatter(Scatter { art, ..Default::default() }),
        _ => BrushKind::Pattern(PatternBrush { side: art.clone(), start: Some(art), ..Default::default() }),
    };
    vectorcraft_brush::store(&mut doc, &[Brush { name: "loop".into(), kind }]);
    let layer = doc.layers[0].id;
    for on_layer in [instance(&mut doc), painted(&mut doc), brushed(&mut doc)] {
        doc.insert(Some(layer), usize::MAX, on_layer).unwrap();
    }
    doc
}

/// `doc` saved, opened again, drawn, exported and edited: nothing overflows.
fn art_by_name_survives(what: &str, doc: Document) {
    let (back, _) = open_deep("named.vectorcraft", &vectorcraft_format::save_file(&doc)).expect("opens");
    let mut s = vectorcraft_engine::Session::new();
    s.add_document(back, None);
    for (cmd, p) in [
        ("select.all", json!({})),
        ("document.inspect", json!({})),
        ("brush.expand", json!({})),
        ("edit.undo", json!({})),
        ("object.expand", json!({})),
        ("edit.undo", json!({})),
        ("object.outlineStroke", json!({})),
        ("edit.undo", json!({})),
        ("graph.design", json!({"paste": "deep"})),
        ("graph.create", json!({"type": "line", "x": 10, "y": 10, "width": 80, "height": 60})),
        ("graph.marker", json!({"design": "deep"})),
        ("graph.create", json!({"type": "column", "x": 10, "y": 10, "width": 80, "height": 60})),
        ("graph.column", json!({"design": "deep", "columnType": "repeating", "each": 1})),
        ("graph.design", json!({"new": "again"})),
        ("edit.copy", json!({})),
        ("edit.paste", json!({})),
    ] {
        let _ = no_panic(s.execute(cmd, &p));
        assert!(!s.doc().unwrap().doc.too_deep(None), "{what}: {cmd}");
    }
    for format in ["svg", "pdf", "eps", "png", "dxf", "emf"] {
        let _ = no_panic(s.execute("document.export", &json!({"format": format})));
    }
}

/// A test for each brush whose art is drawn with the brush itself ([`brushed_document`]).
macro_rules! brushes_drawn_with_themselves {
    ($($test:ident: $kind:literal, $art:literal;)*) => {$(
        #[test]
        fn $test() {
            on_a_small_stack(|| art_by_name_survives(stringify!($test), brushed_document($kind, $art)));
        }
    )*};
}

brushes_drawn_with_themselves! {
    an_art_brush_of_its_own_symbol: 0, 0;
    an_art_brush_of_its_own_symbol_in_deep_art: 0, 1;
    an_art_brush_painted_with_its_own_pattern: 0, 2;
    a_scatter_brush_of_its_own_symbol: 1, 0;
    a_scatter_brush_of_its_own_symbol_in_deep_art: 1, 1;
    a_scatter_brush_painted_with_its_own_pattern: 1, 2;
    a_pattern_brush_of_its_own_symbol: 2, 0;
    a_pattern_brush_of_its_own_symbol_in_deep_art: 2, 1;
    a_pattern_brush_painted_with_its_own_pattern: 2, 2;
}

#[test]
fn a_graph_design_of_deep_art_and_of_a_symbol_holding_itself_is_drawn() {
    use vectorcraft_doc::{GraphDesign, Node, NodeKind};
    on_a_small_stack(|| {
        let mut doc = symbols_document(1, 1, "s0");
        let instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "s0".into(), xf: vectorcraft_geom::Affine::IDENTITY });
        let art = (0..MAX_DEPTH + 4).fold(instance, |inner, _| Node::group(doc.alloc_id(), vec![std::sync::Arc::new(inner)]));
        doc.graph_designs.push(GraphDesign { name: "deep".into(), art: std::sync::Arc::new(art) });
        art_by_name_survives("a graph design of deep art", doc);
    });
}

/// A layer holding `levels` groups in one another around a rectangle, each with a second
/// rectangle beside the next: `levels + 2` deep.
fn nested_document(levels: usize) -> Document {
    use vectorcraft_doc::Node;
    use vectorcraft_testkit::fixtures::DocBuilder;
    let mut b = DocBuilder::new(100.0, 100.0);
    let rect =
        |b: &mut DocBuilder, x: f64| b.rect_node(vectorcraft_geom::Rect::new(x, 0.0, x + 10.0, 10.0), vectorcraft_color::Color::rgb8(200, 0, 0));
    let mut inner = rect(&mut b, 0.0);
    for _ in 0..levels {
        let beside = rect(&mut b, 20.0);
        inner = Node::group(b.alloc(), vec![std::sync::Arc::new(inner), std::sync::Arc::new(beside)]);
        inner.opacity = 0.9;
    }
    b.add(inner);
    b.build()
}

#[test]
fn a_document_at_the_cap_saves_opens_and_exports() {
    on_a_small_stack(|| {
        let doc = nested_document(MAX_DEPTH - 2);
        assert!(!doc.too_deep(None));
        let back = vectorcraft_format::load_file(&vectorcraft_format::save_file(&doc)).expect("what can be made opens again");
        assert!(back.depth_notes.is_empty());
        assert_eq!(back.doc.layers, doc.layers);
        survive("at the cap", || Some(doc.clone())).unwrap();
        // Every format that keeps the nesting reads its own file back within the cap.
        let mut s = vectorcraft_engine::Session::new();
        s.add_document(doc, None);
        for format in ["svg", "pdf", "eps", "ai"] {
            let r = s.execute("document.export", &json!({"format": format})).unwrap();
            let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
            open_deep(&format!("cap.{format}"), &bytes).unwrap_or_else(|| panic!("{format} opens"));
        }
        for cmd in ["select.all", "document.inspect", "document.info", "edit.copy", "edit.paste", "object.ungroup", "edit.undo"] {
            no_panic(s.execute(cmd, &json!({}))).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        }
    });
}

#[test]
fn a_native_file_nested_too_deep_is_merged_or_refused() {
    on_a_small_stack(|| {
        // Saved before the cap by a version that could still open it: opened, the deepest merged.
        let old = vectorcraft_format::save_file(&nested_document(MAX_DEPTH + 2));
        let (doc, warnings) = open_deep("old.vectorcraft", &old).expect("opens");
        assert!(merged(&warnings), "{warnings:?}");
        assert_eq!(paths_in(&doc), MAX_DEPTH + 3, "no art is lost");
        // Deeper than any version could open, and a hostile file: an error that says why. Written
        // by hand: a tree this deep can't be built and saved.
        let text = String::from_utf8(vectorcraft_format::save(&nested_document(0), false)).unwrap();
        let (open, close) = ("{\"id\":900000,\"kind\":{\"type\":\"group\",\"children\":[", "]}}");
        let at = text.find("\"children\":[").expect("the layer's children") + "\"children\":[".len();
        for levels in [100, 50_000] {
            let deep = format!("{}{}{},{}", &text[..at], open.repeat(levels), close.repeat(levels), &text[at..]);
            let e = vectorcraft_engine::cmd::fileio::load("deep.vectorcraft", deep.as_bytes()).err().expect("refused").to_string();
            assert!(e.contains("nested too deep"), "{levels}: {e}");
        }
    });
}

#[test]
fn commands_cannot_nest_deeper_than_the_cap() {
    on_a_small_stack(|| {
        let mut s = vectorcraft_engine::Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        let mut grouped = 0;
        for _ in 0..10_000 {
            match no_panic(s.execute("object.group", &json!({}))) {
                Ok(_) => grouped += 1,
                Err(e) => assert!(e.to_string().contains(vectorcraft_engine::TOO_DEEP), "after {grouped} groups: {e}"),
            }
        }
        // The layer, the groups and the rectangle.
        assert_eq!(grouped, MAX_DEPTH - 2);
        assert!(!s.doc().unwrap().doc.too_deep(None));
        // Other ways down: a clipping mask, a copy pasted, a symbol made of it.
        for (cmd, p) in [
            ("object.clippingMask.make", json!({})),
            ("edit.copy", json!({})),
            ("edit.pasteInFront", json!({})),
            ("symbol.new", json!({"name": "deep"})),
        ] {
            let _ = no_panic(s.execute(cmd, &p));
            assert!(!s.doc().unwrap().doc.too_deep(None), "{cmd}");
        }
        survive("grouped to the cap", || Some((*s.doc().unwrap().doc).clone())).unwrap();
    });
}

/// A document whose one object is an instance of the first of `count` symbols: each symbol's art
/// is `levels` groups around an instance of the next symbol, the last one of `last`.
fn symbols_document(count: usize, levels: usize, last: &str) -> Document {
    use vectorcraft_doc::{Node, NodeKind, Symbol};
    let instance =
        |doc: &mut Document, of: String| Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: of, xf: vectorcraft_geom::Affine::IDENTITY });
    let mut doc = nested_document(1);
    for i in 0..count {
        let mut art = instance(&mut doc, if i + 1 < count { format!("s{}", i + 1) } else { last.to_string() });
        for _ in 0..levels {
            art = Node::group(doc.alloc_id(), vec![std::sync::Arc::new(art)]);
        }
        doc.symbols.push(Symbol { name: format!("s{i}"), art: std::sync::Arc::new(art) });
    }
    let first = instance(&mut doc, "s0".into());
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc
}

/// `doc` saved (as an older version, or a hostile file, could have) and opened again: within the
/// caps (`kept`: as it was), drawn, exported and edited.
fn symbols_survive(what: &str, doc: Document, kept: bool) {
    let (back, warnings) = open_deep("symbols.vectorcraft", &vectorcraft_format::save_file(&doc)).expect("opens");
    assert_eq!(!warnings.iter().any(|w| w.contains("symbol instances")), kept, "{what}: {warnings:?}");
    let mut s = vectorcraft_engine::Session::new();
    s.add_document(back, None);
    for cmd in [
        "select.all",
        "document.inspect",
        "document.info",
        "object.expand",
        "symbol.breakLink",
        "object.flattenTransparency",
        "edit.copy",
        "edit.paste",
    ] {
        let _ = no_panic(s.execute(cmd, &json!({})));
        assert!(!s.doc().unwrap().doc.too_deep(None), "{what}: {cmd}");
    }
    for format in ["svg", "pdf", "eps", "png"] {
        let _ = no_panic(s.execute("document.export", &json!({"format": format})));
    }
}

#[test]
fn a_symbol_holding_itself_opens_without_the_instance() {
    on_a_small_stack(|| {
        symbols_survive("a symbol holding itself", symbols_document(1, 1, "s0"), false);
        symbols_survive("symbols holding one another", symbols_document(3, 2, "s0"), false);
    });
}

#[test]
fn symbols_in_symbols_are_drawn_only_so_deep() {
    on_a_small_stack(|| {
        // As deep as symbols draw: each one's art as deep as art goes.
        symbols_survive("symbols at the cap", symbols_document(3, MAX_DEPTH - 2, "none"), true);
        symbols_survive("symbols in symbols", symbols_document(40, MAX_DEPTH - 1, "none"), false);
        // A symbol can't be made to hold itself, or one that holds it, by editing either.
        let mut s = vectorcraft_engine::Session::new();
        s.add_document(symbols_document(2, 1, "none"), None);
        s.execute("select.all", &json!({})).unwrap();
        for (cmd, p) in [
            ("symbol.new", json!({"name": "s1"})),
            ("symbol.update", json!({"name": "s1"})),
            ("symbol.update", json!({"name": "s0"})),
            ("symbol.replace", json!({"name": "s1"})),
        ] {
            let _ = no_panic(s.execute(cmd, &p));
            assert!(!s.doc().unwrap().doc.too_deep(None), "{cmd} {p}");
        }
    });
}

#[test]
fn a_pattern_painting_its_own_tile_opens_without_the_paint() {
    on_a_small_stack(|| {
        use vectorcraft_color::Paint;
        use vectorcraft_doc::{Appearance, PatternDef};
        let painted = |doc: &mut Document, with: &str| {
            let fill = Paint::Pattern { pattern: with.into(), xf: vectorcraft_geom::Affine::IDENTITY };
            let square = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 20.0, 20.0));
            vectorcraft_doc::Node::path(doc.alloc_id(), square, Appearance::basic(fill, Paint::None, 0.0))
        };
        // One pattern painting its own tile, and two painting each other's.
        for names in [["p", "p"], ["p", "q"]] {
            let mut doc = nested_document(1);
            let (first, second) = (painted(&mut doc, names[1]), painted(&mut doc, names[0]));
            doc.patterns.push(PatternDef::new(names[0], vec![std::sync::Arc::new(first)]));
            if names[0] != names[1] {
                doc.patterns.push(PatternDef::new(names[1], vec![std::sync::Arc::new(second)]));
            }
            let on_layer = painted(&mut doc, "p");
            let layer = doc.layers[0].id;
            doc.insert(Some(layer), usize::MAX, on_layer).unwrap();
            symbols_survive("a pattern painting its own tile", doc, false);
        }
    });
}

/// A .ai cut short inside an array or dictionary of its PDF part (found by
/// `ai_mutated_save_never_panics`): a value can't be read whole, and hayro's iterators over the
/// array's items or the dictionary's entries would panic on it.
#[test]
fn ai_cut_inside_an_array_or_dictionary_opens_without_panicking() {
    for flip in [None, Some((12262, b'/'))] {
        let mut b = saved_ai();
        if let Some((at, c)) = flip {
            b[at] = c;
        }
        b.truncate(44902.min(b.len()));
        survive("ai cut inside an array or dictionary", || vectorcraft_engine::cmd::fileio::load("saved.ai", &b).ok().map(|l| l.doc)).unwrap();
    }
}

/// Found by the fuzzer above: a shape 3.3 million points long and 50 high, with a drop shadow,
/// made the PDF export rasterise its shadow wider than the renderer can address (within the
/// effect image's area limit, past any side), which panicked.
#[test]
fn a_long_thin_shape_with_a_shadow_renders_and_exports() {
    use affinity_docs::*;
    let stream = canvas_stream(vec![with_effects(rectangle(10, (0.0, 0.0), (3_276_800.0, 50.0)), vec![shadow()])]);
    let bytes = affinity_sample_file(&stream, None);
    let doc = vectorcraft_engine::cmd::fileio::load("long.af", &bytes).unwrap().doc;
    survive("long shape", || Some(doc)).unwrap();
}
