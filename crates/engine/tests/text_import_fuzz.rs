//! Untrusted text files placed with File → Place never crash: any bytes, with any Text Import
//! Options, place as area type or fail with an error, and so do hostile RTF files and Word
//! documents.
//!
//! `PROPTEST_CASES=20000 cargo test -p vectorcraft-engine --test text_import_fuzz` runs a deeper search.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use serde_json::json;
use vectorcraft_testkit::catch_quiet;

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn any_bytes_place_as_text_without_panics(
        bytes in prop::collection::vec(prop_oneof![any::<u8>(), Just(b'\r'), Just(b'\n'), Just(b' ')], 0..400),
        bom in prop::sample::select(vec![&b""[..], b"\xEF\xBB\xBF", b"\xFF\xFE", b"\xFE\xFF"]),
        ansi in any::<bool>(),
        mac in any::<bool>(),
        lines in any::<bool>(),
        paras in any::<bool>(),
        spaces in prop::option::of(0u64..8),
    ) {
        // Exercise import/options/thumbnails with bundled fonts, independently of host font collections.
        // Font lookup and fallback have dedicated tests in vectorcraft-text.
        vectorcraft_text::FontDb::global().set_system_fallback(false);
        let data = [bom, &bytes[..]].concat();
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
            let text = json!({
                "characterSet": if ansi { "ansi" } else { "unicode" },
                "platform": if mac { "mac" } else { "windows" },
                "removeLineReturns": lines,
                "removeParagraphReturns": paras,
                "replaceSpaces": spaces,
            });
            let p = json!({"name": "fuzz.txt", "dataBase64": vectorcraft_format::base64_encode(&data), "text": text, "thumbnail": 8});
            let _ = s.execute("file.place.info", &p);
            let _ = s.execute("file.place", &p);
        });
        prop_assert!(r.is_ok(), "panicked: {:?}", r.err());
    }

    #[test]
    fn any_rtf_places_without_panics(
        tokens in prop::collection::vec(arb_rtf_token(), 0..200),
        name in prop::sample::select(vec!["fuzz.rtf", "fuzz.doc"]),
    ) {
        vectorcraft_text::FontDb::global().set_system_fallback(false);
        let data = [&b"{\\rtf1"[..], &tokens.concat()].concat();
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
            let p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&data), "thumbnail": 8});
            let _ = s.execute("file.place.info", &p);
            let _ = s.execute("file.place", &p);
        });
        prop_assert!(r.is_ok(), "panicked: {:?}", r.err());
    }

    #[test]
    fn any_docx_places_without_panics(
        body in prop_oneof![1 => prop::collection::vec(arb_docx_token(), 0..120), 3 => prop::collection::vec(arb_docx_paragraph(), 0..12)],
        styles in prop_oneof![1 => prop::collection::vec(arb_docx_token(), 0..40), 3 => prop::collection::vec(arb_docx_style(), 0..6)],
        changes in prop_oneof![
            3 => Just(vec![]),
            1 => prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..3),
        ],
        name in prop::sample::select(vec!["fuzz.docx", "fuzz.bin"]),
    ) {
        vectorcraft_text::FontDb::global().set_system_fallback(false);
        let w = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";
        let document = format!("<w:document {w}><w:body>{}</w:body></w:document>", body.concat());
        let styles = format!("<w:styles {w}>{}</w:styles>", styles.concat());
        let files = [("word/document.xml", document.as_bytes()), ("word/styles.xml", styles.as_bytes())];
        let mut data = vectorcraft_engine::cmd::fileio::zip::store(&files).unwrap();
        // A changed byte anywhere: in a header, a size, the directory or the XML.
        for (at, byte) in changes {
            let i = at.index(data.len());
            data[i] = byte;
        }
        let r = catch_quiet(|| {
            let mut s = vectorcraft_engine::Session::new();
            s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
            let p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&data), "thumbnail": 8});
            let _ = s.execute("file.place.info", &p);
            let _ = s.execute("file.place", &p);
        });
        prop_assert!(r.is_ok(), "panicked: {:?}", r.err());
    }
}

/// A well-formed paragraph, or a table row of two such paragraphs.
fn arb_docx_paragraph() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => arb_docx_para(),
        1 => (arb_docx_para(), arb_docx_para()).prop_map(|(a, b)| format!("<w:tbl><w:tr><w:tc>{a}</w:tc><w:tc>{b}</w:tc></w:tr></w:tbl>")),
    ]
}

/// A well-formed paragraph: properties and runs of empty elements and text.
fn arb_docx_para() -> impl Strategy<Value = String> {
    let leaves = || prop::collection::vec(arb_docx_leaf(), 0..4);
    (leaves(), prop::collection::vec((leaves(), arb_docx_leaf()), 0..6)).prop_map(|(ppr, runs)| {
        let runs: String = runs.iter().map(|(rpr, t)| format!("<w:r><w:rPr>{}</w:rPr><w:t>{t}</w:t></w:r>", rpr.concat())).collect();
        format!("<w:p><w:pPr>{}</w:pPr>{runs}</w:p>", ppr.concat())
    })
}

/// Elements the Word document reader knows that hold others, opened or closed by
/// [`arb_docx_token`].
const DOCX_TAGS: &str = "p r t pPr rPr tbl tr tc sdt sdtContent hyperlink del ins style docDefaults rPrDefault pPrDefault";

/// Empty elements the Word document reader knows.
const DOCX_EMPTY: &str = "b i caps smallCaps strike dstrike vanish tab ptab br cr noBreakHyphen softHyphen bidi p";

/// Well-formed document defaults, or a paragraph or character style `S` or `C` (based on `S` or
/// not), with properties of empty elements.
fn arb_docx_style() -> impl Strategy<Value = String> {
    let leaves = || prop::collection::vec(arb_docx_leaf(), 0..4).prop_map(|l| l.concat());
    let kind = prop::sample::select(vec![("paragraph", "S"), ("character", "C"), ("paragraph", "Normal\" w:default=\"1")]);
    prop_oneof![
        (leaves(), leaves()).prop_map(|(r, p)| format!(
            "<w:docDefaults><w:rPrDefault><w:rPr>{r}</w:rPr></w:rPrDefault><w:pPrDefault><w:pPr>{p}</w:pPr></w:pPrDefault></w:docDefaults>"
        )),
        (kind, any::<bool>(), leaves(), leaves()).prop_map(|((t, id), based, r, p)| {
            let based = if based { "<w:basedOn w:val=\"S\"/>" } else { "" };
            format!("<w:style w:type=\"{t}\" w:styleId=\"{id}\">{based}<w:rPr>{r}</w:rPr><w:pPr>{p}</w:pPr></w:style>")
        }),
    ]
}

/// Pieces of WordprocessingML the Word document reader knows (paragraphs, runs, properties,
/// tables, fields, styles), a cut-off tag, entities and text.
fn arb_docx_token() -> impl Strategy<Value = String> {
    let tags = prop::sample::select(DOCX_TAGS.split_whitespace().collect::<Vec<_>>());
    let open = (tags, any::<bool>()).prop_map(|(t, end)| format!("<{}w:{t}>", if end { "/" } else { "" }));
    let styles = prop::sample::select(vec!["<w:style w:type=\"paragraph\" w:styleId=\"S\">", "<w:style w:type=\"character\" w:styleId=\"C\">"]);
    prop_oneof![open, styles.prop_map(str::to_string), arb_docx_leaf(), Just("<w:t".to_string())]
}

/// An empty WordprocessingML element the reader knows, with or without a value in or out of
/// range, entities or text.
fn arb_docx_leaf() -> impl Strategy<Value = String> {
    let empty = prop::sample::select(DOCX_EMPTY.split_whitespace().collect::<Vec<_>>());
    let valued = prop::sample::select(vec![
        ("pStyle", "val", "S"),
        ("rStyle", "val", "C"),
        ("basedOn", "val", "S"),
        ("sz", "val", "-7"),
        ("sz", "val", "1e400"),
        ("sz", "val", "12pt"),
        ("position", "val", "99999999999"),
        ("color", "val", "FF00"),
        ("color", "val", "auto"),
        ("u", "val", "none"),
        ("vertAlign", "val", "superscript"),
        ("jc", "val", "distribute"),
        ("jc", "val", "end"),
        ("ind", "hanging", "-360"),
        ("ind", "left", "3in"),
        ("spacing", "after", "x"),
        ("fldChar", "fldCharType", "begin"),
        ("fldChar", "fldCharType", "separate"),
        ("fldChar", "fldCharType", "end"),
        ("rFonts", "asciiTheme", "majorHAnsi"),
        ("rFonts", "ascii", ""),
        ("br", "type", "page"),
        ("b", "val", "0"),
    ]);
    prop_oneof![
        empty.prop_map(|e| format!("<w:{e}/>")),
        valued.prop_map(|(e, a, v)| format!("<w:{e} w:{a}=\"{v}\"/>")),
        Just("&amp;&#x1F600;&lt;".to_string()),
        "[a-z ]{0,6}",
    ]
}

/// Control words the RTF reader knows, and one it doesn't.
const RTF_WORDS: &str = "par line pard plain b i ul ulnone fs f cf u uc qc qj li fi sb sa rtlpar cell row tab fonttbl colortbl red blue \
                         pict bin deff super sub up dn scaps caps ansicpg mac stylesheet upr ud zzunknown";

/// A piece of an RTF file: a brace, a control word the reader knows or doesn't (with a parameter
/// in or out of range, or none), a control symbol, text or raw bytes.
fn arb_rtf_token() -> impl Strategy<Value = Vec<u8>> {
    let words = prop::sample::select(RTF_WORDS.split_whitespace().collect::<Vec<_>>());
    let param = prop::option::of(prop_oneof![-70_000i64..70_000, Just(i64::from(i32::MAX)), Just(-99_999_999_999)]);
    let word = |(w, p, space): (&str, Option<i64>, bool)| {
        let p = p.map(|v| v.to_string()).unwrap_or_default();
        format!("\\{w}{p}{}", if space { " " } else { "" }).into_bytes()
    };
    prop_oneof![
        Just(b"{".to_vec()),
        Just(b"}".to_vec()),
        Just(b"\\*".to_vec()),
        Just(b";".to_vec()),
        (words, param, any::<bool>()).prop_map(word),
        prop::sample::select(vec!["\\'e9", "\\'9", "\\~", "\\-", "\\_", "\\\\", "\\{", "\\}", "\\\r\n", "\\|"]).prop_map(|s| s.as_bytes().to_vec()),
        "[a-z ]{0,6}".prop_map(String::into_bytes),
        prop::collection::vec(any::<u8>(), 0..4),
    ]
}
