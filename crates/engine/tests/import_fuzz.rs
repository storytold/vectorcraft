//! Untrusted files never crash the app: garbage, truncated, mutated and hostile SVG, PDF and DXF input,
//! mutated raster images (Photoshop documents among them) placed with File → Place, and swatch (`.vcswatches`, `.gpl`, `.ase`), graphic style (`.vcstyles`) and flattener preset
//! (`.vcflattener`) libraries, and native files (compressed, damaged, saved for older versions),
//! must load as an error or as a document that then renders and exports, without a panic; nor may
//! bitmaps, PDF and text pasted from other apps, nor EMF and WMF pictures (damaged files, records
//! of every kind with random contents) opened, placed or pasted, nor EPS and PostScript files
//! (damaged ones, hostile programs) read by the PostScript interpreter, nor the editing data of
//! Illustrator EPS and `.ai` files (the layers they carry, damaged or hostile), nor Affinity documents
//! (mutated object streams, archives and indexed PNG previews, hostile image dimensions), nor the
//! font files a folder search reads (damaged fonts, collections whose headers give any count of
//! faces).
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
            for cmd in ["file.place", "file.place.info"] { assert!(s.execute(cmd, &p).is_err()); }
            assert!(s.execute("file.place.queue", &json!({"files":[p]})).is_err());
        });
        prop_assert!(r.is_ok(), "Affinity placement panicked: {:?}", r.err());
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
    ]);
    let value = prop_oneof![
        arb_num(),
        prop::collection::vec(arb_num(), 0..7).prop_map(|v| v.join(" ")),
        prop::collection::vec(arb_num(), 0..7).prop_map(|v| format!("matrix({})", v.join(" "))),
        prop::collection::vec(arb_num(), 0..3).prop_map(|v| format!("rotate({}) scale({})", v.join(" "), v.join(","))),
        prop::collection::vec((prop::sample::select(vec!["M", "L", "C", "Q", "A", "Z", "H", "V", "S", "T", "m", "a"]), arb_num()), 0..12)
            .prop_map(|v| v.iter().map(|(c, n)| format!("{c}{n} {n}")).collect::<Vec<_>>().join(" ")),
        prop::collection::vec(arb_num(), 0..2).prop_map(|v| format!("{}%", v.join(""))),
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
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox {media} /Contents 4 0 R /Resources {resources} >>"),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
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

    /// Every page box, rotation, page pick, password and Crop To, through `document.open` and
    /// `document.pdfInfo` (with a thumbnail).
    #[test]
    fn pdf_import_options_never_panic(
        boxes in prop::collection::vec(arb_box(), 5),
        rotate in prop::sample::select(vec![0, 90, 180, 270, 45, -90, 1_000_000_000]),
        encrypt in any::<bool>(),
        password in prop::sample::select(vec!["", "pw", "wrong", "\u{0}\u{ff}"]),
        pages in prop::sample::select(vec!["1", "2", "1-2", "2-1", "0", "-", "1-", "all", "1,1,2", "99999999999999999999"]),
        crop in prop::sample::select(vec!["bounding", "art", "crop", "trim", "bleed", "media", "nope"]),
    ) {
        use vectorcraft_testkit::pdf::{PdfPage, pdf};
        let page = PdfPage {
            media: boxes[0].unwrap_or([0.0, 0.0, 100.0, 100.0]),
            crop: boxes[1],
            bleed: boxes[2],
            trim: boxes[3],
            art: boxes[4],
            rotate,
            ..PdfPage::new(0.0, 0.0, "0 0 1 rg 20 20 50 50 re f")
        };
        let bytes = pdf(&[page.clone(), page], encrypt.then_some("pw"));
        let b64 = vectorcraft_format::base64_encode(&bytes);
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            let p = json!({"name": "x.pdf", "dataBase64": b64, "pages": pages, "cropTo": crop, "password": password, "thumbnail": 1, "thumbnailSize": 64});
            let _ = s.execute("document.pdfInfo", &p);
            if s.execute("document.open", &p).is_ok() {
                let d = (*s.doc().unwrap().doc).clone();
                survive("pdf options", || Some(d)).unwrap();
            }
        });
        r.map_err(|msg| TestCaseError::fail(format!("{pages} {crop} {rotate}: panicked: {msg}")))?;
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
        "/Properties << /MC0 {a} 0 R /MC1 {b} 0 R >> /XObject << /X0 {form} 0 R /X1 {hidden} 0 R >> /Pattern << /P0 {pat} 0 R >> /Shading << /Sh0 {mesh} 0 R /Sh1 << /ShadingType 3 /ColorSpace /DeviceGray /Coords [50 50 0 50 50 40] /Extend [false false] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >> >> /ExtGState << /GS0 << /SMask << /Type /Mask /S /Luminosity /G {form} 0 R /BC [1] /TR << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> >> >> /GS1 << /SMask << /Type /Mask /S /Alpha /G {form} 0 R >> >> >> /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+Odd-Name,Bold >> >> /ColorSpace << /CS0 [/Pattern /DeviceRGB] >>"
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
            "/CS0 cs 1 0 0 /P0 scn",
            "/Pattern cs /P0 scn",
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
    fn library_garbage_never_panics(s in ".{0,300}", head in prop::sample::select(vec!["", "GIMP Palette\n","{\"format\": \"vcswatches\", ", "{\"format\": \"vcstyles\", ", "{\"format\": \"vcflattener\", "])) {
        let data = format!("{head}{s}");
        swatches("swatch library garbage", &data)?;
        styles("style library garbage", &data)?;
        flattener_presets("flattener preset garbage", &data)?;
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
    fn mutated_images_place_without_panics(which in 0usize..6, cut in 0usize..4000, edits in prop::collection::vec((0usize..120, any::<u8>()), 0..12)) {
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
            let _ = s.execute("file.place.info", &p);
            let _ = s.execute("file.place", &p);
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
        if s.execute(cmd, &p).is_ok() {
            // Paste may refuse what it was given, e.g. a damaged PDF whose art spans more than the
            // canvas ("result would exceed the canvas"): an error is a fine outcome, a panic isn't.
            let _ = s.execute("edit.paste", &json!({"center": [50, 50]}));
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

    /// Hostile PostScript programs: operators in any order with any operands, opened and placed;
    /// as files in the legacy Illustrator format too (the creator line is what turns on its
    /// documented `u` … `U` groups), the groups unbalanced, deep and among clips.
    #[test]
    fn eps_hostile_programs_never_panic(tokens in prop::collection::vec(arb_ps_token(), 0..60), illustrator in any::<bool>()) {
        let head = if illustrator { "%%Creator: Adobe Illustrator(R) 8.0\n%%EndComments\n/u {} def /U {} def" } else { "%%EndComments" };
        let ps = format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 200\n{head}\n{}\nshowpage\n", tokens.join(" "));
        survive("hostile PostScript", || vectorcraft_engine::cmd::fileio::load("x.eps", ps.as_bytes()).ok().map(|l| l.doc))?;
        let r = catch_quiet(|| {
            let mut s = rich_session();
            let _ = s.execute("file.place", &json!({"name": "x.eps", "dataBase64": vectorcraft_format::base64_encode(ps.as_bytes())}));
        });
        r.map_err(|msg| TestCaseError::fail(format!("placing hostile PostScript panicked: {msg}")))?;
    }
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
            "<< /Predictor 12 /Columns 2 /Colors 1 >> /FlateDecode", "<< /FormType 1 /BBox [0 0 9 9] /PaintProc { pop 0 0 5 5 rectfill } >>",
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
            let _ = s.execute("document.pdfInfo", &p);
            if s.execute("document.open", &p).is_ok() {
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
        s.execute("file.new", &json!({"width": 300, "height": 200})).ok()?;
        let p = json!({"name": "x.dxf", "dataBase64": vectorcraft_format::base64_encode(bytes), "dxf": {"fit": fit}, "thumbnail": 8});
        let _ = s.execute("file.place.info", &p);
        let _ = s.execute("file.place", &p);
        let _ = s.execute("document.dxfInfo", &p);
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
        let _ = s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}));
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
/// each one imported.
fn print_presets(what: &str, data: &str) -> Result<(), TestCaseError> {
    survive_library(what, "print.presets.import", data, |s, r| {
        let list = no_panic(s.execute("print.presets.list", &json!({}))).unwrap_or_default();
        for name in r["imported"].as_array().into_iter().flatten() {
            let Some(p) = list["presets"].as_array().into_iter().flatten().find(|p| &p["name"] == name) else { continue };
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
        "U".into(),
        "0 Xw".into(),
        "1 0 0 0 1 0 Bg".into(),
        "0 1 w 2 J 0 j 4 M [3 2]0 d 1 D".into(),
    ]
    .join("\n");
    let layer = |name: &str, visible: u8, body: &str| {
        format!("%AI5_BeginLayer\n{visible} 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n({name}) Ln\n{body}\nLB\n%AI5_EndLayer--\n")
    };
    format!(
        "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 100 100\n%%HiResBoundingBox: 0 0 100 100\n%AI3_Cropmarks: 0 0 100 100\n{}{}%%Trailer\n",
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
        survive("mutated .ai editing data", || Some(vectorcraft_eps::layered_ai(&private, Document::new(100.0, 100.0), vec![], false).0))?;
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
            "/SymbolInstance :",
            "/Binary : /ASCII85Decode ,",
            "~>",
            "p",
            "To",
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
            "\n%%PageTrailer\n",
        ])
        .prop_map(str::to_string),
    ]
}

/// The testkit's sample editing data, compressed as older `.ai` files compress it.
fn ai_sample_compressed() -> Vec<u8> {
    use std::io::Write as _;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&vectorcraft_testkit::ai::sample_data()).unwrap();
    [&b"%AI12_CompressedData"[..], &e.finish().unwrap()].concat()
}

proptest! {
    #![proptest_config(config())]

    /// Hostile editing data in Illustrator EPS and `.ai` files: operators in any order with any
    /// operands, sections and dictionaries left open, images of any size.
    #[test]
    fn ai_hostile_editing_data_never_panics(tokens in prop::collection::vec(arb_ai_token(), 0..80)) {
        use vectorcraft_testkit::ai;
        let data = ai::editing_data(200.0, 100.0, &format!("%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\n{}\nLB\n", tokens.join(" ")));
        survive("hostile editing data", || Some(vectorcraft_eps::layered_ai(data.as_bytes(), Document::new(200.0, 100.0), vec![], false).0))?;
        survive("hostile editing data, type as outlines", || Some(vectorcraft_eps::layered_ai(data.as_bytes(), Document::new(200.0, 100.0), vec![], true).0))?;
        let eps = ai::eps(data.as_bytes(), ai::page_ps());
        survive("hostile editing data in an EPS", || vectorcraft_engine::cmd::fileio::load("x.eps", &eps).ok().map(|l| l.doc))?;
        // An EPS whose page is its art's box, not its artboard: what prints outside it is compared too.
        let boxed = ai::eps(data.replace("%AI3_Cropmarks: 0 0 200 100", "%AI3_Cropmarks: -50 -50 300 200").as_bytes(), ai::page_ps());
        survive("hostile editing data in an EPS of its art's box", || vectorcraft_engine::cmd::fileio::load("x.eps", &boxed).ok().map(|l| l.doc))?;
    }

    /// The testkit's sample damaged, as it is and compressed, in EPS and `.ai` files.
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
