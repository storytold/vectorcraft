//! Untrusted text files placed with File → Place never crash: any bytes, with any Text Import
//! Options, place as area type or fail with an error, and so do hostile RTF files, Word documents
//! and Word 97-2003 documents.
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

    #[test]
    fn any_doc_places_without_panics(
        text in prop::collection::vec(prop::sample::select(DOC_CHARS.chars().collect::<Vec<_>>()), 1..60),
        compressed in any::<bool>(),
        chpx in arb_grpprl(),
        papx in arb_grpprl(),
        istd in 0u16..4,
        stsh in prop_oneof![1 => prop::collection::vec(any::<u8>(), 0..120), 3 => arb_stsh()],
        ffn in prop::collection::vec(any::<u8>(), 0..80),
        changes in prop_oneof![
            3 => Just(vec![]),
            1 => prop::collection::vec((any::<bool>(), any::<prop::sample::Index>(), any::<u8>()), 1..4),
        ],
        name in prop::sample::select(vec!["fuzz.doc", "fuzz.bin"]),
    ) {
        vectorcraft_text::FontDb::global().set_system_fallback(false);
        let text: String = text.into_iter().collect();
        let (mut wd, mut table) = word97(&text, compressed, &chpx, istd, &papx, &stsh, &ffn);
        // A changed byte anywhere in either stream: in the FIB, the text, a property page or a
        // table.
        for (in_table, at, byte) in changes {
            let stream = if in_table { &mut table } else { &mut wd };
            let i = at.index(stream.len());
            stream[i] = byte;
        }
        let mut cf = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
        std::io::Write::write_all(&mut cf.create_stream("WordDocument").unwrap(), &wd).unwrap();
        std::io::Write::write_all(&mut cf.create_stream("1Table").unwrap(), &table).unwrap();
        cf.flush().unwrap();
        let data = cf.into_inner().into_inner();
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

/// Characters of a Word document's text: letters, marks of paragraphs, cells and breaks, fields,
/// pictures, hyphens and surrogate-free Unicode.
const DOC_CHARS: &str = "ab \r\u{7}\u{B}\u{C}\u{13}\u{14}\u{15}\u{1}\u{8}\t\u{1E}\u{1F}(é“✓";

/// Character and paragraph properties: Prls the Word 97-2003 reader knows, with operands in and out
/// of range, and raw bytes.
fn arb_grpprl() -> impl Strategy<Value = Vec<u8>> {
    let prl = prop_oneof![
        prop::sample::select(vec![
            &[0x35, 0x08, 0x81][..],
            &[0x35, 0x08, 0x80],
            &[0x3C, 0x08, 0x01],
            &[0x00, 0x08, 0x01],
            &[0x01, 0x08, 0x01],
            &[0x55, 0x08, 0x01],
            &[0x30, 0x4A, 0x0A, 0x00],
            &[0x30, 0x4A, 0xFF, 0xFF],
            &[0x43, 0x4A, 0xFF, 0xFF],
            &[0x4F, 0x4A, 0x07, 0x00],
            &[0x42, 0x2A, 0x63],
            &[0x70, 0x68, 1, 2, 3, 0xFF],
            &[0x45, 0x48, 0x00, 0x80],
            &[0x03, 0x24, 0x09],
            &[0x61, 0x24, 0x08],
            &[0x17, 0x24, 0x01],
            &[0x4B, 0x24, 0x01],
            &[0x41, 0x24, 0x01],
            &[0x0F, 0x84, 0x00, 0x80],
            &[0x13, 0xA4, 0xFF, 0xFF],
            &[0x08, 0xD6, 0xFF, 0xFF],
            &[0x15, 0xC6, 0xFF, 0x7F],
            &[0x2A, 0xC6, 0xFF],
        ])
        .prop_map(<[u8]>::to_vec),
        prop::collection::vec(any::<u8>(), 0..4),
    ];
    prop::collection::vec(prl, 0..6).prop_map(|p| p.concat())
}

/// A style sheet of up to four styles (paragraph or character, based on another or none) with
/// properties of [`arb_grpprl`], or empty slots.
fn arb_stsh() -> impl Strategy<Value = Vec<u8>> {
    let style = prop::option::of((1u16..3, prop_oneof![Just(0x0FFFu16), 0u16..5], arb_grpprl(), arb_grpprl()));
    prop::collection::vec(style, 0..5).prop_map(|styles| {
        let words = |v: &[u16]| v.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
        let mut stsh = words(&[18, styles.len() as u16, 10, 1, 0, 15, 0, 0, 0, 0]);
        for s in styles {
            let Some((stk, base, papx, chpx)) = s else {
                stsh.extend([0, 0]);
                continue;
            };
            let upx = |b: Vec<u8>| [words(&[b.len() as u16]), b.clone(), vec![0; b.len() % 2]].concat();
            let (cupx, grlpupx) = if stk == 1 { (2, [upx([vec![0, 0], papx].concat()), upx(chpx)].concat()) } else { (1, upx(chpx)) };
            let mut std = words(&[0x0FFE, stk | (base << 4), cupx | (0x0FFF << 4), 0, 0, 1, u16::from(b'S'), 0]);
            std.extend(grlpupx);
            stsh.extend((std.len() as u16).to_le_bytes());
            stsh.extend(std);
        }
        stsh
    })
}

/// The WordDocument and table streams of a Word 97-2003 document whose main text is `text` in one
/// piece, with one page of character properties `chpx` and one of paragraph properties `papx` of
/// style `istd` covering it, and the style sheet `stsh` and font table `ffn`.
fn word97(text: &str, compressed: bool, chpx: &[u8], istd: u16, papx: &[u8], stsh: &[u8], ffn: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut wd = vec![0u8; 1024];
    let at = wd.len() as u32;
    if compressed {
        wd.extend(text.chars().map(|c| if c == '“' { 0x93 } else { u32::from(c).min(0xFF) as u8 }));
    } else {
        wd.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    }
    let ccp = text.encode_utf16().count() as u32;
    let end = wd.len() as u32;
    // One page each: a run of every character, its properties near the page's end.
    let page = |props: Vec<u8>, papx: bool| {
        let mut p = vec![0u8; 512];
        p[0..4].copy_from_slice(&at.to_le_bytes());
        p[4..8].copy_from_slice(&end.to_le_bytes());
        let blob = if papx { [vec![0, (props.len() / 2) as u8], props].concat() } else { [vec![props.len() as u8], props].concat() };
        let top = (511 - blob.len().min(400)) & !1;
        let n = blob.len().min(511 - top);
        p[top..top + n].copy_from_slice(&blob[..n]);
        p[8] = (top / 2) as u8;
        p[511] = 1;
        p
    };
    let papx_props = [istd.to_le_bytes().to_vec(), papx.to_vec(), vec![0; papx.len() % 2]].concat();
    wd.resize(wd.len().div_ceil(512) * 512, 0);
    let chpx_page = (wd.len() / 512) as u32;
    wd.extend(page(chpx.iter().copied().take(200).collect(), false));
    wd.extend(page(papx_props.into_iter().take(400).collect(), true));
    let bte = |pn: u32| [at, end, pn].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<u8>>();
    let fc = if compressed { (at * 2) | 0x4000_0000 } else { at };
    let plc = [&0u32.to_le_bytes()[..], &ccp.to_le_bytes(), &[0, 0], &fc.to_le_bytes(), &[0, 0]].concat();
    let clx = [&[0x02][..], &(plc.len() as u32).to_le_bytes(), &plc].concat();
    let mut table = vec![];
    let mut pairs = vec![(0u32, 0u32); 0x5D];
    for (index, part) in [(1, stsh.to_vec()), (12, bte(chpx_page)), (13, bte(chpx_page + 1)), (15, ffn.to_vec()), (33, clx)] {
        pairs[index] = (table.len() as u32, part.len() as u32);
        table.extend(part);
    }
    let words = |v: &[u16]| v.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
    let mut put = |at: usize, bytes: &[u8]| wd[at..at + bytes.len()].copy_from_slice(bytes);
    put(0, &words(&[0xA5EC, 0x00C1]));
    put(10, &words(&[0x0200, 0x00BF]));
    put(32, &words(&[14]));
    put(62, &words(&[22]));
    put(76, &ccp.to_le_bytes());
    put(80, &1u32.to_le_bytes());
    put(152, &words(&[0x5D]));
    for (i, (fc, lcb)) in pairs.iter().enumerate() {
        put(154 + 8 * i, &[fc.to_le_bytes(), lcb.to_le_bytes()].concat());
    }
    (wd, table)
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
