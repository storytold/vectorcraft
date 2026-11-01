//! File → Place of a text file (Text Import Options), an RTF file or a Word document: the text set
//! as area type.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, TextKind};
use vectorcraft_geom::Rect;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn place_text(s: &mut Session, text: &[u8], extra: Value) -> Value {
    let mut p = json!({"name": "notes.txt", "dataBase64": vectorcraft_format::base64_encode(text)});
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    s.execute("file.place", &p).unwrap()
}

/// The placed type's text and frame bounds.
fn placed(s: &Session, r: &Value) -> (String, Rect) {
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type: {:?}", n.kind) };
    assert!(matches!(t.kind, TextKind::Area { .. }), "area type");
    (t.plain_text(), n.geometric_bounds().unwrap())
}

fn close(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 1e-6)
}

#[test]
fn crlf_with_returns_removed_places_one_paragraph_per_block_as_area_type() {
    let mut s = session();
    let text = b"Dear reader,\r\nthis is the first\r\nparagraph.\r\n\r\nAnd   here\r\nthe second.\r\n";
    let opts = json!({"text": {"removeLineReturns": true, "removeParagraphReturns": true, "replaceSpaces": 3}});
    let r = place_text(&mut s, text, opts);
    assert_eq!((r["format"].as_str(), r["name"].as_str(), r["linked"].as_bool()), (Some("txt"), Some("notes.txt"), Some(false)));
    let (body, frame) = placed(&s, &r);
    assert_eq!(body, "Dear reader, this is the first paragraph.\nAnd\there the second.");
    assert!(close(frame, Rect::new(36.0, 36.0, 364.0, 264.0)), "the artboard less a 36 pt margin: {frame:?}");
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.selection.objects.len()), (1, 1), "one undo step, selected");
    // Without options each line is a paragraph; `rect` sets the frame.
    let r = place_text(&mut s, text, json!({"rect": [10, 20, 100, 50]}));
    let (body, frame) = placed(&s, &r);
    assert_eq!(body, "Dear reader,\nthis is the first\nparagraph.\n\nAnd   here\nthe second.");
    assert!(close(frame, Rect::new(10.0, 20.0, 110.0, 70.0)), "{frame:?}");
}

/// Placed at a point on an artboard's right or bottom edge, the frame takes that artboard's size less
/// a 36 pt margin on each side; on the edge two artboards share, away from its ends, it takes the
/// size of the artboard to the right of it.
#[test]
fn type_placed_on_an_artboard_edge_takes_that_artboards_frame() {
    let mut s = session();
    let mut wide = s.doc().unwrap().doc.artboards[0].clone();
    wide.rect = Rect::new(400.0, 0.0, 600.0, 200.0);
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).artboards.push(wide);
    for at in [[600.0, 100.0], [500.0, 200.0], [400.0, 100.0]] {
        let r = place_text(&mut s, b"Edge", json!({ "at": at }));
        let frame = placed(&s, &r).1;
        assert!(close(frame, Rect::from_center_size((at[0], at[1]), (128.0, 128.0))), "{at:?}: {frame:?}");
    }
}

#[test]
fn text_files_report_and_queue_with_their_options() {
    let mut s = session();
    let file = json!({"name": "a.txt", "dataBase64": vectorcraft_format::base64_encode(b"caf\xe9")});
    let info = s.execute("file.place.info", &file).unwrap();
    assert_eq!((info["format"].as_str(), info["width"].as_f64(), info["height"].as_f64()), (Some("txt"), Some(468.0), Some(648.0)));
    // Not UTF-8: read as Windows-1252, or Mac Roman for a Mac file.
    let r = place_text(&mut s, b"caf\x8e", json!({"text": {"characterSet": "ansi", "platform": "mac"}}));
    assert_eq!(placed(&s, &r).0, "café");
    let r = s.execute("file.place.queue", &json!({"files": [file], "text": {"characterSet": "ansi"}})).unwrap();
    assert_eq!((r["count"].as_u64(), r["files"][0]["format"].as_str()), (Some(1), Some("txt")));
    assert_eq!(s.tool_options()["name"], "a.txt");
    let bad = json!({"name": "a.txt", "dataBase64": "YQ==", "text": {"platform": "beos"}});
    assert!(s.execute("file.place", &bad).is_err());
    let blank = json!({"name": "blank.txt", "dataBase64": vectorcraft_format::base64_encode(b" \r\n\t\r\n")});
    assert!(s.execute("file.place", &blank).is_err(), "no text to place");
    // Replace: the frame takes the replaced object's bounds.
    s.execute("shape.rectangle", &json!({"x": 50, "y": 60, "width": 80, "height": 40})).unwrap();
    let r = place_text(&mut s, b"Swap", json!({"replace": true}));
    let (body, frame) = placed(&s, &r);
    assert_eq!(body, "Swap");
    assert!(close(frame, Rect::new(50.0, 60.0, 130.0, 100.0)), "{frame:?}");
}

/// An RTF file with a font table, a color table, a bold run, an italic run, a superscript and a
/// centered, indented second paragraph.
const LETTER_RTF: &str = concat!(
    r"{\rtf1\ansi\deff0{\fonttbl{\f0\froman Times New Roman;}{\f1\fswiss Arial;}}",
    r"{\colortbl;\red255\green0\blue0;}{\*\generator Some Writer;}{\info{\title Notes}}",
    "\r\n",
    r"\pard\plain\fs24 Plain and {\b bold} words{\super 2}.\par",
    "\r\n",
    r"\pard\qc\li720\sa240\f1\fs36\cf1 Centered {\i caf\'e9}\par",
    "\r\n}"
);

#[test]
fn rtf_places_as_area_type_with_its_character_and_paragraph_formatting() {
    let mut s = session();
    s.execute("document.setup", &json!({"superscript": {"size": 50, "position": 40}})).unwrap();
    let r = s.execute("file.place", &json!({"name": "letter.rtf", "dataBase64": vectorcraft_format::base64_encode(LETTER_RTF.as_bytes())})).unwrap();
    assert_eq!((r["format"].as_str(), r["name"].as_str()), (Some("rtf"), Some("letter.rtf")));
    let (body, frame) = placed(&s, &r);
    assert_eq!(body, "Plain and bold words2.\nCentered café");
    assert!(close(frame, Rect::new(36.0, 36.0, 364.0, 264.0)), "the artboard less a 36 pt margin: {frame:?}");
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type") };
    let runs: Vec<_> = t.runs.iter().map(|r| (r.text.as_str(), r.style.font_family.as_str(), r.style.font_style.as_str(), r.style.size)).collect();
    assert_eq!(
        runs,
        [
            ("Plain and ", "Times New Roman", "Regular", 12.0),
            ("bold", "Times New Roman", "Bold", 12.0),
            (" words", "Times New Roman", "Regular", 12.0),
            ("2", "Times New Roman", "Regular", 12.0),
            (".\n", "Times New Roman", "Regular", 12.0),
            ("Centered ", "Arial", "Regular", 18.0),
            ("café", "Arial", "Italic", 18.0),
        ]
    );
    let superscript = vectorcraft_doc::ScriptMetrics { size: 50.0, position: 40.0 };
    assert_eq!(t.runs[3].style.position, vectorcraft_doc::CharPosition::Superscript(superscript), "in Document Setup's proportions");
    let red = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(255, 0, 0));
    assert_eq!((t.runs[0].style.fill == red, t.runs[5].style.fill == red, t.runs[6].style.fill == red), (false, true, true));
    let paras = t.paragraph_styles();
    let summary: Vec<_> = paras.iter().map(|p| (p.justify, p.left_indent, p.space_after)).collect();
    assert_eq!(summary, [(vectorcraft_doc::Justify::Left, 0.0, 0.0), (vectorcraft_doc::Justify::Center, 36.0, 12.0)]);
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.selection.objects.len()), (2, 1), "Document Setup and Place, each one undo step");
    // Info and the place cursor report it as RTF.
    let file = json!({"name": "letter.rtf", "dataBase64": vectorcraft_format::base64_encode(LETTER_RTF.as_bytes())});
    let info = s.execute("file.place.info", &file).unwrap();
    assert_eq!((info["format"].as_str(), info["width"].as_f64()), (Some("rtf"), Some(468.0)));
    let q = s.execute("file.place.queue", &json!({"files": [file]})).unwrap();
    assert_eq!((q["count"].as_u64(), q["files"][0]["format"].as_str()), (Some(1), Some("rtf")));
}

#[test]
fn malformed_rtf_fails_cleanly() {
    let mut s = session();
    let place = |s: &mut Session, name: &str, bytes: &[u8]| {
        s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}))
    };
    assert!(place(&mut s, "a.rtf", b"Plain words, not RTF").is_err(), "an .rtf file that isn't RTF");
    assert!(place(&mut s, "a.rtf", br"{\rtf1{\fonttbl{\f0 Arial;}}\par\par }").is_err(), "no text to place");
    assert!(place(&mut s, "a.rtf", br"{\rtf1 {\pict\bin999999 ").is_err(), "binary data past the end of the file");
    // Text, a stray brace and a cut-off escape after the closing brace are ignored.
    let r = place(&mut s, "a.rtf", br"{\rtf1 Cut {\b off}} after}\'9").unwrap();
    assert_eq!(placed(&s, &r).0, "Cut off");
    // A .txt file is plain text even when it holds RTF.
    let r = place(&mut s, "a.txt", br"{\rtf1 x}").unwrap();
    assert_eq!((r["format"].as_str(), placed(&s, &r).0.as_str()), (Some("txt"), r"{\rtf1 x}"));
    assert_eq!(s.doc().unwrap().history.undo.len(), 2, "failed places leave no undo step");
}

/// A ZIP archive of `files`, each deflated, its headers declaring each file's size, or `declared`
/// for every file.
fn deflated_zip(files: &[(&str, &[u8])], declared: Option<u32>) -> Vec<u8> {
    let (mut out, mut central) = (vec![], vec![]);
    for (name, data) in files {
        let mut e = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
        std::io::Write::write_all(&mut e, data).unwrap();
        let packed = e.finish().unwrap();
        // Flags, method (8: deflated), time, date, CRC, sizes and the name's length.
        let mut shared = vec![];
        for v in [0u16, 8, 0, 0] {
            shared.extend(v.to_le_bytes());
        }
        for v in [crc32fast::hash(data), packed.len() as u32, declared.unwrap_or(data.len() as u32)] {
            shared.extend(v.to_le_bytes());
        }
        shared.extend((name.len() as u16).to_le_bytes());
        let offset = out.len() as u32;
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(20u16.to_le_bytes());
        out.extend(&shared);
        out.extend(0u16.to_le_bytes());
        out.extend(name.as_bytes());
        out.extend(&packed);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20u16, 20].map(u16::to_le_bytes).concat());
        central.extend(&shared);
        central.extend([0u8; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend([files.len() as u16; 2].map(u16::to_le_bytes).concat());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// A Word document's package: its main part `body` (the inside of `w:body`), with a styles part
/// and a theme part named through the main part's relationships.
pub(super) fn docx(body: &str, styles: &str) -> Vec<u8> {
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let rels = "http://schemas.openxmlformats.org/package/2006/relationships";
    let types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/></Types>"#;
    let root =
        format!(r#"<Relationships xmlns="{rels}"><Relationship Id="rId1" Type="{rel}/officeDocument" Target="word/document.xml"/></Relationships>"#);
    let doc_rels = format!(
        r#"<Relationships xmlns="{rels}"><Relationship Id="rId1" Type="{rel}/styles" Target="styles.xml"/><Relationship Id="rId2" Type="{rel}/theme" Target="theme/theme1.xml"/><Relationship Id="rId3" Type="{rel}/hyperlink" Target="https://example.com/" TargetMode="External"/></Relationships>"#
    );
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W_NS}"><w:body>{body}<w:sectPr/></w:body></w:document>"#
    );
    let styles = format!(r#"<w:styles xmlns:w="{W_NS}">{styles}</w:styles>"#);
    let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Test"><a:themeElements><a:fontScheme name="Test"><a:majorFont><a:latin typeface="Arial"/></a:majorFont><a:minorFont><a:latin typeface="Georgia"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;
    let files: [(&str, &[u8]); 6] = [
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
        ("word/theme/theme1.xml", theme.as_bytes()),
    ];
    deflated_zip(&files, None)
}

/// Document defaults in the theme's minor font at 11 pt with 8 pt after each paragraph, a default
/// Normal style, a centered, bold Heading 1 in the theme's major font at 16 pt and in a color, a
/// bold character style and a default character style in all caps.
pub(super) const STYLES: &str = concat!(
    r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi"/><w:sz w:val="22"/></w:rPr></w:rPrDefault>"#,
    r#"<w:pPrDefault><w:pPr><w:spacing w:after="160"/></w:pPr></w:pPrDefault></w:docDefaults>"#,
    r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>"#,
    r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:basedOn w:val="Normal"/><w:pPr><w:jc w:val="center"/><w:spacing w:before="240" w:after="0"/></w:pPr>"#,
    r#"<w:rPr><w:rFonts w:asciiTheme="majorHAnsi"/><w:b/><w:sz w:val="32"/><w:color w:val="1F3864"/></w:rPr></w:style>"#,
    r#"<w:style w:type="character" w:styleId="Strong"><w:rPr><w:b/></w:rPr></w:style>"#,
    r#"<w:style w:type="character" w:default="1" w:styleId="Plain"><w:rPr><w:caps/></w:rPr></w:style>"#,
);

#[test]
fn docx_places_as_area_type_with_its_styles_and_formatting() {
    let body = concat!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t xml:space="preserve">Report </w:t></w:r>"#,
        r#"<w:r><w:rPr><w:rStyle w:val="Strong"/></w:rPr><w:t>notes</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:ind w:left="720" w:hanging="360"/><w:jc w:val="both"/></w:pPr><w:r><w:t xml:space="preserve">Plain and </w:t></w:r>"#,
        r#"<w:r><w:rPr><w:rFonts w:ascii="Courier New"/><w:i/><w:u w:val="single"/></w:rPr><w:t>styled</w:t></w:r>"#,
        r#"<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:t>2</w:t></w:r><w:r><w:rPr><w:vanish/></w:rPr><w:t>hidden</w:t></w:r>"#,
        r#"<w:del w:id="1" w:author="A"><w:r><w:delText>gone</w:delText></w:r></w:del><w:ins w:id="2" w:author="A"><w:r><w:t xml:space="preserve"> page </w:t></w:r></w:ins>"#,
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>"#,
        r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>7</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
    );
    let mut s = session();
    s.execute("document.setup", &json!({"superscript": {"size": 50, "position": 40}})).unwrap();
    let file = json!({"name": "report.docx", "dataBase64": vectorcraft_format::base64_encode(&docx(body, STYLES))});
    let r = s.execute("file.place", &file).unwrap();
    assert_eq!((r["format"].as_str(), r["name"].as_str()), (Some("docx"), Some("report.docx")));
    let (text, frame) = placed(&s, &r);
    assert_eq!(text, "Report notes\nPlain and styled2 page 7", "hidden and deleted text and the field's instructions are left out");
    assert!(close(frame, Rect::new(36.0, 36.0, 364.0, 264.0)), "the artboard less a 36 pt margin: {frame:?}");
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type") };
    let runs: Vec<_> = t.runs.iter().map(|r| (r.text.as_str(), r.style.font_family.as_str(), r.style.font_style.as_str(), r.style.size)).collect();
    assert_eq!(
        runs,
        [
            ("Report ", "Arial", "Bold", 16.0),
            // Bold in the character style reverses the paragraph style's bold.
            ("notes", "Arial", "Regular", 16.0),
            ("\n", "Arial", "Bold", 16.0),
            ("Plain and ", "Georgia", "Regular", 11.0),
            ("styled", "Courier New", "Italic", 11.0),
            ("2", "Georgia", "Regular", 11.0),
            (" page 7", "Georgia", "Regular", 11.0),
        ]
    );
    let navy = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(0x1F, 0x38, 0x64));
    assert_eq!((t.runs[0].style.fill == navy, t.runs[1].style.fill == navy, t.runs[3].style.fill == navy), (true, true, false));
    assert_eq!((t.runs[3].style.underline, t.runs[4].style.underline), (false, true));
    let caps: Vec<_> = t.runs.iter().map(|r| r.style.all_caps).collect();
    assert_eq!(caps, [true, false, false, true, true, true, true], "runs without a character style take the default one");
    let superscript = vectorcraft_doc::ScriptMetrics { size: 50.0, position: 40.0 };
    assert_eq!(t.runs[5].style.position, vectorcraft_doc::CharPosition::Superscript(superscript), "in Document Setup's proportions");
    let paras = t.paragraph_styles();
    let summary: Vec<_> = paras.iter().map(|p| (p.justify, p.left_indent, p.first_line_indent, p.space_before, p.space_after)).collect();
    assert_eq!(summary, [(vectorcraft_doc::Justify::Center, 0.0, 0.0, 12.0, 0.0), (vectorcraft_doc::Justify::JustifyLeft, 36.0, -18.0, 0.0, 8.0)]);
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.selection.objects.len()), (2, 1), "Document Setup and Place, each one undo step");
    // Info and the place cursor report it as a Word document.
    let info = s.execute("file.place.info", &file).unwrap();
    assert_eq!((info["format"].as_str(), info["width"].as_f64()), (Some("docx"), Some(468.0)));
    let q = s.execute("file.place.queue", &json!({"files": [file]})).unwrap();
    assert_eq!((q["count"].as_u64(), q["files"][0]["format"].as_str()), (Some(1), Some("docx")));
}

#[test]
fn docx_double_strikethrough_is_not_a_toggle_property() {
    // A paragraph style and a character style that both set double strikethrough, and a run that
    // turns it off directly.
    let styles = concat!(
        r#"<w:style w:type="paragraph" w:styleId="Struck"><w:rPr><w:dstrike/></w:rPr></w:style>"#,
        r#"<w:style w:type="character" w:styleId="AlsoStruck"><w:rPr><w:dstrike/></w:rPr></w:style>"#,
    );
    let body = concat!(
        r#"<w:p><w:pPr><w:pStyle w:val="Struck"/></w:pPr><w:r><w:t xml:space="preserve">one </w:t></w:r>"#,
        r#"<w:r><w:rPr><w:rStyle w:val="AlsoStruck"/></w:rPr><w:t xml:space="preserve">two </w:t></w:r>"#,
        r#"<w:r><w:rPr><w:rStyle w:val="AlsoStruck"/><w:dstrike w:val="0"/></w:rPr><w:t>three</w:t></w:r></w:p>"#,
    );
    let mut s = session();
    let r = s.execute("file.place", &json!({"name": "struck.docx", "dataBase64": vectorcraft_format::base64_encode(&docx(body, styles))})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type") };
    let struck: Vec<_> = t.runs.iter().map(|r| (r.text.as_str(), r.style.strikethrough)).collect();
    assert_eq!(struck, [("one two ", true), ("three", false)], "the character style keeps the paragraph style's double strikethrough");
}

#[test]
fn docx_tables_breaks_and_fields_place_as_paragraphs_and_tabs() {
    // The Strict namespace, a package without relationships or styles, stored without compression
    // and named without `.docx`.
    let document = concat!(
        r#"<w:document xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main"><w:body>"#,
        r#"<w:p><w:r><w:t>Line one</w:t><w:br/><w:t>line two</w:t><w:br/><w:br w:type="page"/></w:r></w:p>"#,
        r#"<w:tbl><w:tblPr/><w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr>"#,
        r#"<w:tr><w:tc><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc>"#,
        r#"<w:sdt><w:sdtContent><w:tc><w:p><w:hyperlink><w:r><w:t>d</w:t></w:r></w:hyperlink></w:p></w:tc></w:sdtContent></w:sdt></w:tr>"#,
        r#"<w:tr><w:tc><w:p><w:r><w:t>e</w:t></w:r></w:p></w:tc><w:tc><w:p/><w:p><w:r><w:t>f</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        r#"<w:p><w:fldSimple w:instr=" DATE "><w:r><w:t>today</w:t></w:r></w:fldSimple><w:r><w:tab/><w:t>end</w:t></w:r>"#,
        r#"<w:r><w:drawing><w:t>not text</w:t></w:drawing></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:bidi/><w:jc w:val="both"/></w:pPr><w:r><w:t><!-- a comment -->justi<?pi x?>fied</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:bidi/><w:jc w:val="end"/><w:ind w:start="400"/></w:pPr><w:r><w:t>rtl</w:t></w:r></w:p><w:p/><w:p/>"#,
        "</w:body></w:document>"
    );
    let mut zip = cmd::fileio::zip::store(&[("word/document.xml", document)]).unwrap();
    // An archive comment holding the end record's signature.
    let comment = b"note PK\x05\x06 and 22 more bytes after it, at least";
    let length = zip.len() - 2;
    zip[length..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    zip.extend(comment);
    let mut s = session();
    let r = s.execute("file.place", &json!({"name": "notes.bin", "dataBase64": vectorcraft_format::base64_encode(&zip)})).unwrap();
    assert_eq!(r["format"].as_str(), Some("docx"));
    assert_eq!(
        placed(&s, &r).0,
        "Line one\nline two\n\na\tb\nc\t\td\ne\t\nf\ntoday\tend\njustified\nrtl",
        "blank paragraphs at the end are left out"
    );
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type") };
    let paras = t.paragraph_styles();
    assert_eq!(paras[paras.len() - 2].justify, vectorcraft_doc::Justify::JustifyRight, "the last line of right-to-left justified text at the right");
    let last = paras.last().cloned().unwrap();
    // The start indent of a right-to-left paragraph is its right indent.
    assert_eq!(
        (last.justify, last.left_indent, last.right_indent, last.direction),
        (vectorcraft_doc::Justify::Left, 0.0, 20.0, Some(vectorcraft_doc::ParaDirection::RightToLeft))
    );
}

#[test]
fn docx_kashida_justification_places_with_insert_kashidas() {
    use vectorcraft_doc::Justify;
    use vectorcraft_doc::text::Kashida;
    // The document holds right-to-left paragraphs justified with each kashida length and with `both`, a paragraph style
    // that sets `mediumKashida`, that style under direct formatting, and a left-to-right paragraph.
    let styles = r#"<w:style w:type="paragraph" w:styleId="Kashidas"><w:pPr><w:bidi/><w:jc w:val="mediumKashida"/></w:pPr></w:style>"#;
    let para = |ppr: &str| format!(r#"<w:p><w:pPr>{ppr}</w:pPr><w:r><w:t>كتاب جميل</w:t></w:r></w:p>"#);
    let body: String = [
        r#"<w:bidi/><w:jc w:val="lowKashida"/>"#,
        r#"<w:bidi/><w:jc w:val="mediumKashida"/>"#,
        r#"<w:bidi/><w:jc w:val="highKashida"/>"#,
        r#"<w:bidi/><w:jc w:val="both"/>"#,
        r#"<w:pStyle w:val="Kashidas"/>"#,
        r#"<w:pStyle w:val="Kashidas"/><w:jc w:val="both"/>"#,
        r#"<w:jc w:val="highKashida"/>"#,
        r#"<w:pStyle w:val="Kashidas"/><w:jc w:val="center"/>"#,
    ]
    .map(para)
    .concat();
    let mut s = session();
    let r = s.execute("file.place", &json!({"name": "kashida.docx", "dataBase64": vectorcraft_format::base64_encode(&docx(&body, styles))})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type") };
    let paras: Vec<_> = t.paragraph_styles().iter().map(|p| (p.justify, p.kashida)).collect();
    assert_eq!(
        paras,
        [
            (Justify::JustifyRight, Kashida::Short),
            (Justify::JustifyRight, Kashida::Medium),
            (Justify::JustifyRight, Kashida::Long),
            (Justify::JustifyRight, Kashida::None),
            (Justify::JustifyRight, Kashida::Medium),
            (Justify::JustifyRight, Kashida::None),
            (Justify::JustifyLeft, Kashida::Long),
            (Justify::Center, Kashida::None),
        ],
        "low, medium and high kashida lengths are Short, Medium and Long; other alignments have none"
    );
}

#[test]
fn malformed_docx_fails_cleanly() {
    let mut s = session();
    let place = |s: &mut Session, name: &str, bytes: &[u8]| {
        s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).map_err(|e| e.to_string())
    };
    let store = |files: &[(&str, &[u8])]| cmd::fileio::zip::store(files).unwrap();
    let document = |body: &str| format!(r#"<w:document xmlns:w="{W_NS}"><w:body>{body}</w:body></w:document>"#);
    assert!(place(&mut s, "a.docx", b"Plain words, not a package").is_err(), "a .docx file that isn't a ZIP archive");
    let e = place(&mut s, "a.docx", &store(&[("readme.txt", b"x")])).unwrap_err();
    assert!(e.contains("no main document part"), "{e}");
    let e = place(&mut s, "a.zip", &store(&[("readme.txt", b"x")])).unwrap_err();
    assert!(e.contains("not a format VectorCraft reads"), "a ZIP archive of something else isn't read as a Word document: {e}");
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#;
    let e = place(&mut s, "a.docx", &store(&[("word/document.xml", sheet.as_bytes())])).unwrap_err();
    assert!(e.contains("not a WordprocessingML document"), "{e}");
    let e = place(&mut s, "a.docx", &store(&[("word/document.xml", b"<w:document><w:body>")])).unwrap_err();
    assert!(e.contains("not well-formed XML"), "{e}");
    let e = place(&mut s, "a.docx", &store(&[("word/document.xml", document("<w:p/>").as_bytes())])).unwrap_err();
    assert!(e.contains("no text to place"), "{e}");
    // A file whose headers understate what it unpacks to, and one whose headers say it is too large.
    let bomb = vec![b' '; (16 << 20) + 1];
    let e = place(&mut s, "a.docx", &deflated_zip(&[("word/document.xml", &bomb)], Some(100))).unwrap_err();
    assert!(e.contains("too large to place"), "{e}");
    let e = place(&mut s, "a.docx", &deflated_zip(&[("word/document.xml", b"<w:document/>")], Some(17 << 20))).unwrap_err();
    assert!(e.contains("too large to place"), "{e}");
    let many: Vec<(String, &[u8])> = (0..10_001).map(|i| (format!("f{i}"), &b""[..])).collect();
    let e = place(&mut s, "a.docx", &store(&many.iter().map(|(n, b)| (n.as_str(), *b)).collect::<Vec<_>>())).unwrap_err();
    assert!(e.contains("more than 10000 files"), "{e}");
    // A changed byte fails the file's CRC.
    let mut changed = store(&[("word/document.xml", document("<w:p><w:r><w:t>words</w:t></w:r></w:p>").as_bytes())]);
    let at = changed.windows(5).position(|w| w == b"words").unwrap();
    changed[at] = b'W';
    let e = place(&mut s, "a.docx", &changed).unwrap_err();
    assert!(e.contains("damaged"), "{e}");
    // A stored file whose directory record gives more packed bytes than unpacked ones.
    let mut longer = store(&[("word/document.xml", document("<w:p><w:r><w:t>words</w:t></w:r></w:p>").as_bytes()), ("pad", &[0; 64])]);
    let central = longer.windows(4).position(|w| w == 0x0201_4b50u32.to_le_bytes()).unwrap();
    let packed = u32::from_le_bytes(longer[central + 20..central + 24].try_into().unwrap()) + 64;
    longer[central + 20..central + 24].copy_from_slice(&packed.to_le_bytes());
    let e = place(&mut s, "a.docx", &longer).unwrap_err();
    assert!(e.contains("damaged"), "{e}");
    // Containers nested past the depth read are skipped, and a part nested deeper still is refused.
    let nested = |n: usize| {
        let deep = format!("{}<w:p><w:r><w:t>deep</w:t></w:r></w:p>{}", "<w:sdt><w:sdtContent>".repeat(n), "</w:sdtContent></w:sdt>".repeat(n));
        store(&[("word/document.xml", document(&format!("{deep}<w:p><w:r><w:t>shallow</w:t></w:r></w:p>")).as_bytes())])
    };
    let r = place(&mut s, "a.docx", &nested(100)).unwrap();
    assert_eq!(placed(&s, &r).0, "shallow");
    let e = place(&mut s, "a.docx", &nested(5000)).unwrap_err();
    assert!(e.contains("nests elements more than 256 deep"), "{e}");
    // A font name is cut to 256 bytes.
    let long = format!(r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="{}"/></w:rPr><w:t>font</w:t></w:r></w:p>"#, "é".repeat(1000));
    let r = place(&mut s, "a.docx", &store(&[("word/document.xml", document(&long).as_bytes())])).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type") };
    assert_eq!(t.runs[0].style.font_family, "é".repeat(128));
    assert_eq!(s.doc().unwrap().history.undo.len(), 2, "failed places leave no undo step");
}

/// The bytes of the Prls `prls`: each sprm and its operand.
fn prls(prls: &[(u16, &[u8])]) -> Vec<u8> {
    prls.iter().flat_map(|(sprm, op)| sprm.to_le_bytes().into_iter().chain(op.iter().copied())).collect()
}

/// A style of a [`Word97`] document: its kind (1 paragraph, 2 character), the style it is based
/// on, and its paragraph and character properties.
type Word97Style = (u16, u16, Vec<u8>, Vec<u8>);

/// A Word 97-2003 document, built as \[MS-DOC\] lays one out: a file information block, the main
/// text's pieces stored last first, one property page per run of up to 16 character or paragraph
/// property runs, the piece table, style sheet and font table in the table stream, all in a
/// compound file.
#[derive(Default)]
struct Word97 {
    /// The main text's pieces: their text and whether it is stored compressed.
    pieces: Vec<(&'static str, bool)>,
    /// Character properties of characters `start..end`.
    chpx: Vec<(u32, u32, Vec<u8>)>,
    /// The paragraph style and properties of the paragraph whose mark is character `cp`; a
    /// character's paragraph is the first one whose mark is at or after it.
    papx: Vec<(u32, u16, Vec<u8>)>,
    /// The styles by istd; None: an empty slot.
    styles: Vec<Option<Word97Style>>,
    fonts: Vec<&'static str>,
    /// FibBase's flags and `nFib` (0: 0x00C1).
    flags: u16,
    nfib: u16,
    /// The table stream is `0Table` (else `1Table`).
    zero_table: bool,
    /// `ccpFtn`, `ccpHdd`, `ccpAtn`, `ccpEdn`, `ccpTxbx` and `ccpHdrTxbx`.
    stories: [u32; 6],
    /// Characters `ccpText` gives beyond the pieces.
    ccp_extra: u32,
    /// A Clx to write instead of the pieces'.
    clx: Option<Vec<u8>>,
}

/// One 512-byte property page of `runs` (file positions and properties), written to `wd`; its
/// page number.
fn property_page(runs: &[(u32, u32, Vec<u8>)], papx: bool, wd: &mut Vec<u8>) -> u32 {
    let mut page = [0u8; 512];
    let n = runs.len();
    for (i, r) in runs.iter().enumerate() {
        page[4 * i..4 * i + 4].copy_from_slice(&r.0.to_le_bytes());
    }
    page[4 * n..4 * n + 4].copy_from_slice(&runs[n - 1].1.to_le_bytes());
    let mut top = 511;
    for (i, (_, _, props)) in runs.iter().enumerate() {
        let blob = match (papx, props.len() % 2) {
            (false, _) if props.is_empty() => continue,
            (false, _) => [vec![props.len() as u8], props.clone()].concat(),
            // PapxInFkp: cb with 2 × cb − 1 bytes, or 0 and cb′ with 2 × cb′ bytes.
            (true, 1) => [vec![props.len().div_ceil(2) as u8], props.clone()].concat(),
            (true, _) => [vec![0, (props.len() / 2) as u8], props.clone()].concat(),
        };
        top = (top - blob.len()) & !1;
        page[top..top + blob.len()].copy_from_slice(&blob);
        page[4 * (n + 1) + if papx { 13 * i } else { i }] = (top / 2) as u8;
    }
    page[511] = n as u8;
    wd.resize(wd.len().div_ceil(512) * 512, 0);
    let pn = (wd.len() / 512) as u32;
    wd.extend(page);
    pn
}

impl Word97 {
    fn build(&self) -> Vec<u8> {
        let mut wd = vec![0u8; 900];
        let mut stored = vec![0u32; self.pieces.len()];
        for (i, (text, compressed)) in self.pieces.iter().enumerate().rev() {
            stored[i] = wd.len() as u32;
            if *compressed {
                wd.extend(text.chars().map(|c| match c {
                    '“' => 0x93,
                    '”' => 0x94,
                    c => u8::try_from(u32::from(c)).unwrap(),
                }));
            } else {
                // U+FFFF and U+FFFE stand for the high surrogate 0xD800 and the low surrogate 0xDE00
                // on their own.
                let unit = |u| match u {
                    0xFFFF => 0xD800,
                    0xFFFE => 0xDE00,
                    u => u,
                };
                wd.extend(text.encode_utf16().map(unit).flat_map(u16::to_le_bytes));
            }
        }
        // Each character's file position and size, in character order.
        let mut chars: Vec<(u32, u32)> = vec![];
        for ((text, compressed), &at) in self.pieces.iter().zip(&stored) {
            let w = if *compressed { 1 } else { 2 };
            chars.extend((0..text.encode_utf16().count() as u32).map(|k| (at + k * w, w)));
        }
        let chpx_of = |cp: u32| (0u32, self.chpx.iter().rev().find(|(s, e, _)| (*s..*e).contains(&cp)).map(|r| r.2.clone()).unwrap_or_default());
        let papx_of = |cp: u32| {
            let p = self.papx.iter().filter(|(mark, _, _)| *mark >= cp).min_by_key(|(mark, _, _)| *mark);
            p.map_or((u32::MAX, vec![0, 0]), |(mark, istd, g)| (*mark, [istd.to_le_bytes().to_vec(), g.clone()].concat()))
        };
        let mut bins = vec![];
        for (papx, of) in [(false, &chpx_of as &dyn Fn(u32) -> (u32, Vec<u8>)), (true, &papx_of)] {
            let mut runs: Vec<(u32, u32, u32, Vec<u8>)> = vec![];
            for (cp, &(fc, w)) in chars.iter().enumerate() {
                let (key, props) = of(cp as u32);
                match runs.last_mut() {
                    Some((_, end, k, p)) if *end == fc && *k == key && *p == props => *end = fc + w,
                    _ => runs.push((fc, fc + w, key, props)),
                }
            }
            // Pages of contiguous runs, 16 at most.
            let mut groups: Vec<Vec<(u32, u32, Vec<u8>)>> = vec![];
            for (start, end, _, props) in runs {
                match groups.last_mut() {
                    Some(g) if g.len() < 16 && g.last().unwrap().1 == start => g.push((start, end, props)),
                    _ => groups.push(vec![(start, end, props)]),
                }
            }
            let mut fcs: Vec<u32> = groups.iter().map(|g| g[0].0).collect();
            fcs.extend(groups.last().map(|g| g.last().unwrap().1));
            let pns: Vec<u32> = groups.iter().map(|g| property_page(g, papx, &mut wd)).collect();
            bins.push([fcs, pns].concat().into_iter().flat_map(u32::to_le_bytes).collect::<Vec<u8>>());
        }
        let mut cps = vec![0u32];
        for (text, _) in &self.pieces {
            cps.push(cps.last().unwrap() + text.encode_utf16().count() as u32);
        }
        let pcds = self.pieces.iter().zip(&stored).flat_map(|((_, compressed), &at)| {
            let fc = if *compressed { (at * 2) | 0x4000_0000 } else { at };
            [&[0u8, 0][..], &fc.to_le_bytes(), &[0, 0]].concat()
        });
        let plc: Vec<u8> = cps.iter().flat_map(|c| c.to_le_bytes()).chain(pcds).collect();
        let clx = self.clx.clone().unwrap_or_else(|| [vec![0x02], (plc.len() as u32).to_le_bytes().to_vec(), plc].concat());
        let words = |v: &[u16]| v.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
        let mut stsh = words(&[18, self.styles.len() as u16, 10, 1, 0, 15, 0, 0, 0, 0]);
        for (istd, s) in self.styles.iter().enumerate() {
            let Some((stk, base, papx, chpx)) = s else {
                stsh.extend([0, 0]);
                continue;
            };
            let upx = |bytes: Vec<u8>| [words(&[bytes.len() as u16]), bytes.clone(), vec![0; bytes.len() % 2]].concat();
            let grlpupx = match stk {
                1 => [upx([(istd as u16).to_le_bytes().to_vec(), papx.clone()].concat()), upx(chpx.clone())].concat(),
                _ => upx(chpx.clone()),
            };
            let cupx = if *stk == 1 { 2 } else { 1 };
            let mut std = words(&[0x0FFE, stk | (base << 4), cupx | (0x0FFF << 4), 0, 0, 1, u16::from(b'S'), 0]);
            std.extend(grlpupx);
            let len = std.len() as u16;
            std[6..8].copy_from_slice(&len.to_le_bytes());
            stsh.extend(len.to_le_bytes());
            stsh.extend(std);
        }
        let mut ffn = words(&[self.fonts.len() as u16, 0]);
        for name in &self.fonts {
            let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
            ffn.push((39 + 2 * units.len()) as u8);
            ffn.extend([0u8; 39]);
            ffn.extend(words(&units));
        }
        let mut table = vec![];
        let mut pairs = vec![(0u32, 0u32); 0x5D];
        for (index, part) in [(1, stsh), (12, bins[0].clone()), (13, bins[1].clone()), (15, ffn), (33, clx)] {
            pairs[index] = (table.len() as u32, part.len() as u32);
            table.extend(part);
        }
        let mut put = |at: usize, bytes: &[u8]| wd[at..at + bytes.len()].copy_from_slice(bytes);
        let flags = self.flags | if self.zero_table { 0 } else { 0x0200 };
        put(0, &words(&[0xA5EC, if self.nfib == 0 { 0x00C1 } else { self.nfib }]));
        put(10, &words(&[flags, 0x00BF]));
        put(32, &words(&[14]));
        put(62, &words(&[22]));
        put(76, &(chars.len() as u32 + self.ccp_extra).to_le_bytes());
        for (i, n) in [4, 5, 7, 8, 9, 10].into_iter().zip(self.stories) {
            put(64 + 4 * i, &n.to_le_bytes());
        }
        put(152, &words(&[0x5D]));
        for (i, (fc, lcb)) in pairs.iter().enumerate() {
            put(154 + 8 * i, &[fc.to_le_bytes(), lcb.to_le_bytes()].concat());
        }
        compound_file(&[("WordDocument", &wd), (if self.zero_table { "0Table" } else { "1Table" }, &table)])
    }
}

/// A compound file holding `streams`.
fn compound_file(streams: &[(&str, &[u8])]) -> Vec<u8> {
    let mut cf = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
    for (name, bytes) in streams {
        std::io::Write::write_all(&mut cf.create_stream(name).unwrap(), bytes).unwrap();
    }
    cf.flush().unwrap();
    cf.into_inner().into_inner()
}

#[test]
fn doc_places_as_area_type_with_its_styles_and_formatting() {
    let navy: &[u8] = &[0x1F, 0x38, 0x64, 0];
    // Normal: 11 pt with 8 pt after; Heading (based on Normal): centered, 12 pt before and none
    // after, in Arial, bold, 16 pt and navy; Emphasis, a character style: italic.
    let mut styles = vec![
        Some((1, 0x0FFF, prls(&[(0xA414, &[160, 0])]), prls(&[(0x4A43, &[22, 0])]))),
        Some((
            1,
            0,
            prls(&[(0x2403, &[1]), (0xA413, &[240, 0]), (0xA414, &[0, 0])]),
            prls(&[(0x4A4F, &[1, 0]), (0x0835, &[1]), (0x4A43, &[32, 0]), (0x6870, navy)]),
        )),
    ];
    styles.resize_with(10, || None);
    styles.push(Some((2, 0x0FFF, vec![], prls(&[(0x0836, &[1])]))));
    // The first piece holds characters 0 to 24 ("notes" at 7, the first paragraph mark at 12), the
    // second "styled" at 25, "2" at 31, "hidden" at 32, "gone" at 38, " café ✓ " at 42, the field
    // at 50 (its result "7" at 58) and the paragraph mark at 60.
    let doc = Word97 {
        pieces: vec![("Report notes\r“Plain” and ", true), ("styled2hiddengone café ✓ \u{13} PAGE \u{14}7\u{15}\r", false)],
        chpx: vec![
            (7, 12, prls(&[(0x4A30, &[10, 0])])),
            (25, 31, prls(&[(0x4A4F, &[2, 0]), (0x0836, &[1]), (0x2A3E, &[1])])),
            (31, 32, prls(&[(0x2A48, &[1])])),
            (32, 38, prls(&[(0x083C, &[1])])),
            (38, 42, prls(&[(0x0800, &[1])])),
            (42, 50, prls(&[(0x0801, &[1])])),
        ],
        papx: vec![(12, 1, vec![]), (60, 0, prls(&[(0x840F, &[0xD0, 0x02]), (0x8411, &(-360i16).to_le_bytes()), (0x2403, &[3])]))],
        styles,
        fonts: vec!["Times New Roman", "Arial", "Courier New"],
        ..Word97::default()
    };
    let mut s = session();
    s.execute("document.setup", &json!({"superscript": {"size": 50, "position": 40}})).unwrap();
    let file = json!({"name": "report.doc", "dataBase64": vectorcraft_format::base64_encode(&doc.build())});
    let r = s.execute("file.place", &file).unwrap();
    assert_eq!((r["format"].as_str(), r["name"].as_str()), (Some("doc"), Some("report.doc")));
    let (text, frame) = placed(&s, &r);
    assert_eq!(text, "Report notes\n“Plain” and styled2 café ✓ 7", "hidden and deleted text and the field's instructions are left out");
    assert!(close(frame, Rect::new(36.0, 36.0, 364.0, 264.0)), "the artboard less a 36 pt margin: {frame:?}");
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type") };
    let runs: Vec<_> = t.runs.iter().map(|r| (r.text.as_str(), r.style.font_family.as_str(), r.style.font_style.as_str(), r.style.size)).collect();
    assert_eq!(
        runs,
        [
            ("Report ", "Arial", "Bold", 16.0),
            ("notes", "Arial", "Bold Italic", 16.0),
            ("\n", "Arial", "Bold", 16.0),
            ("“Plain” and ", "Times New Roman", "Regular", 11.0),
            ("styled", "Courier New", "Italic", 11.0),
            ("2", "Times New Roman", "Regular", 11.0),
            (" café ✓ 7", "Times New Roman", "Regular", 11.0),
        ]
    );
    let navy = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(0x1F, 0x38, 0x64));
    assert_eq!((t.runs[0].style.fill == navy, t.runs[1].style.fill == navy, t.runs[3].style.fill == navy), (true, true, false));
    assert_eq!((t.runs[3].style.underline, t.runs[4].style.underline), (false, true));
    let superscript = vectorcraft_doc::ScriptMetrics { size: 50.0, position: 40.0 };
    assert_eq!(t.runs[5].style.position, vectorcraft_doc::CharPosition::Superscript(superscript), "in Document Setup's proportions");
    let paras = t.paragraph_styles();
    let summary: Vec<_> = paras.iter().map(|p| (p.justify, p.left_indent, p.first_line_indent, p.space_before, p.space_after)).collect();
    assert_eq!(summary, [(vectorcraft_doc::Justify::Center, 0.0, 0.0, 12.0, 0.0), (vectorcraft_doc::Justify::JustifyLeft, 36.0, -18.0, 0.0, 8.0)]);
    let fields = "the Word document's fields were placed as the text of their results";
    let revisions = "the Word document's tracked changes were accepted: deleted text was left out and inserted text kept";
    assert_eq!(r["warnings"], json!([fields, revisions]));
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.selection.objects.len()), (2, 1), "Document Setup and Place, each one undo step");
    // Info and the place cursor report it as a Word 97-2003 document, with the same warnings.
    let info = s.execute("file.place.info", &file).unwrap();
    assert_eq!((info["format"].as_str(), info["width"].as_f64(), &info["warnings"]), (Some("doc"), Some(468.0), &json!([fields, revisions])));
    let q = s.execute("file.place.queue", &json!({"files": [file]})).unwrap();
    assert_eq!((q["count"].as_u64(), q["files"][0]["format"].as_str()), (Some(1), Some("doc")));
}

#[test]
fn doc_tables_breaks_and_other_stories_place_as_paragraphs_and_tabs_with_warnings() {
    let in_table = prls(&[(0x2416, &[1])]);
    // A line break; a table row of two cells; a page break inside a paragraph and a section break
    // that ends one; a picture, a tab and a nonbreaking hyphen; right-to-left justified text.
    let text = "Line one\u{B}line two\ra\u{7}b\u{7}\u{7}x\u{C}y\rsec\u{C}next\rpic\u{1}ture\tend\u{1E}ok\rrtl\r";
    let doc = Word97 {
        pieces: vec![(text, true)],
        // Marks: the first paragraph's at 17, the cells' at 19 and 21, the row's at 22, the page
        // break at 24 inside the paragraph that ends at 26, the section break at 30, and the
        // paragraph marks at 35, 51 and 55.
        papx: vec![
            (17, 0, vec![]),
            (19, 0, in_table.clone()),
            (21, 0, in_table),
            (22, 0, prls(&[(0x2416, &[1]), (0x2417, &[1])])),
            (26, 0, vec![]),
            (30, 0, vec![]),
            (35, 0, vec![]),
            (51, 0, vec![]),
            (55, 0, prls(&[(0x2441, &[1]), (0x2461, &[3])])),
        ],
        zero_table: true,
        stories: [5, 0, 2, 0, 0, 0],
        ..Word97::default()
    };
    let mut s = session();
    // Recognized by its WordDocument stream whatever its name says.
    let r = s.execute("file.place", &json!({"name": "notes.bin", "dataBase64": vectorcraft_format::base64_encode(&doc.build())})).unwrap();
    assert_eq!(r["format"].as_str(), Some("doc"));
    assert_eq!(placed(&s, &r).0, "Line one\nline two\na\tb\nxy\nsec\nnext\npicture\tend\u{2011}ok\nrtl");
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type") };
    let last = t.paragraph_styles().last().cloned().unwrap();
    assert_eq!((last.justify, last.direction), (vectorcraft_doc::Justify::JustifyRight, Some(vectorcraft_doc::ParaDirection::RightToLeft)));
    assert_eq!(
        r["warnings"],
        json!([
            "the Word document's pictures and embedded objects were left out",
            "the Word document's tables were placed as paragraphs with their cells separated by tabs",
            "the Word document's footnotes and comments were left out",
        ])
    );
}

#[test]
fn malformed_doc_fails_cleanly() {
    let mut s = session();
    let place = |s: &mut Session, name: &str, bytes: &[u8]| {
        s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).map_err(|e| e.to_string())
    };
    let words = || Word97 { pieces: vec![("words\r", true)], ..Word97::default() };
    let e = place(&mut s, "a.doc", b"Plain words, not a compound file").unwrap_err();
    assert!(e.contains("not a format VectorCraft reads"), "{e}");
    // An RTF file named `.doc` is still placed as RTF.
    let r = place(&mut s, "a.doc", br"{\rtf1 Rich}").unwrap();
    assert_eq!((r["format"].as_str(), placed(&s, &r).0.as_str()), (Some("rtf"), "Rich"));
    let e = place(&mut s, "a.doc", &compound_file(&[("Other", b"x")])).unwrap_err();
    assert!(e.contains("no WordDocument stream"), "{e}");
    let e = place(&mut s, "a.bin", &compound_file(&[("Other", b"x")])).unwrap_err();
    assert!(e.contains("not a format VectorCraft reads"), "a compound file of something else isn't read as a Word document: {e}");
    let e = place(&mut s, "a.doc", &[&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1][..], &[0; 600]].concat()).unwrap_err();
    assert!(e.contains("compound file structure is damaged"), "{e}");
    let e = place(&mut s, "a.doc", &compound_file(&[("WordDocument", &[0; 900])])).unwrap_err();
    assert!(e.contains("doesn't start with a file information block"), "{e}");
    let e = place(&mut s, "a.doc", &Word97 { nfib: 0x0065, ..words() }.build()).unwrap_err();
    assert!(e.contains("Word 6.0 or Word 95"), "{e}");
    let e = place(&mut s, "a.doc", &Word97 { flags: 0x0100, ..words() }.build()).unwrap_err();
    assert!(e.contains("encrypted"), "{e}");
    let e = place(&mut s, "a.doc", &Word97 { flags: 0x0004, ..words() }.build()).unwrap_err();
    assert!(e.contains("fast save"), "{e}");
    let e = place(&mut s, "a.doc", &Word97 { ccp_extra: 3, ..words() }.build()).unwrap_err();
    assert!(e.contains("doesn't cover its text"), "{e}");
    // A piece stored past the end of the WordDocument stream.
    let far = [&0u32.to_le_bytes()[..], &6u32.to_le_bytes(), &[0, 0], &0x00FF_0000u32.to_le_bytes(), &[0, 0]].concat();
    let clx = [&[0x02][..], &(far.len() as u32).to_le_bytes(), &far].concat();
    let e = place(&mut s, "a.doc", &Word97 { clx: Some(clx), ..words() }.build()).unwrap_err();
    assert!(e.contains("lies outside its WordDocument stream"), "{e}");
    let e = place(&mut s, "a.doc", &Word97 { clx: Some(vec![0x02, 0xFF, 0xFF, 0, 0]), ..words() }.build()).unwrap_err();
    assert!(e.contains("piece table is damaged"), "{e}");
    let e = place(&mut s, "a.doc", &Word97 { pieces: vec![("\r\r", true)], ..Word97::default() }.build()).unwrap_err();
    assert!(e.contains("no text to place"), "{e}");
    // 16-bit text with half of a surrogate pair is refused rather than placed with a stand-in.
    let e = place(&mut s, "a.doc", &Word97 { pieces: vec![("bad \u{FFFF} text\r", false)], ..Word97::default() }.build()).unwrap_err();
    assert!(e.contains("half of a UTF-16 surrogate pair"), "{e}");
    // A surrogate pair split between two pieces is one character.
    let r = place(&mut s, "a.doc", &Word97 { pieces: vec![("x\u{FFFF}", false), ("\u{FFFE}y\r", false)], ..Word97::default() }.build()).unwrap();
    assert_eq!(placed(&s, &r).0, "x\u{10200}y");
    // A field that ends without a result is left out, with its own warning.
    let r = place(&mut s, "a.doc", &Word97 { pieces: vec![("a\u{13} PAGE \u{15}b\r", true)], ..Word97::default() }.build()).unwrap();
    assert_eq!(placed(&s, &r).0, "ab");
    assert_eq!(r["warnings"], json!(["the Word document's fields without a result that could be read were left out"]));
    // So is a field that never ends, with the text after it.
    let r = place(&mut s, "a.doc", &Word97 { pieces: vec![("a\rb\u{13} PAGE c\r", true)], ..Word97::default() }.build()).unwrap();
    assert_eq!(placed(&s, &r).0, "a\nb");
    assert_eq!(r["warnings"], json!(["the Word document's fields without a result that could be read were left out"]));
    let r = place(&mut s, "a.doc", &words().build()).unwrap();
    assert_eq!(placed(&s, &r).0, "words");
    assert_eq!(s.doc().unwrap().history.undo.len(), 5, "failed places leave no undo step");
}

#[test]
fn place_reads_what_open_reads_and_text_files() {
    use cmd::fileio::{DOC_EXTS, DOCX_EXTS, OPEN_EXTS, PLACE_EXTS, RTF_EXTS, TEXT_EXTS};
    assert_eq!(PLACE_EXTS, [OPEN_EXTS, TEXT_EXTS, RTF_EXTS, DOCX_EXTS, DOC_EXTS].concat());
    let filters: Vec<_> = cmd::fileio::place_filters().collect();
    let word: (&str, &[&str]) = ("Word", &["docx", "doc"]);
    assert_eq!((filters.first(), filters.last()), (Some(&("All placeable files", PLACE_EXTS)), Some(&word)));
    assert_eq!(filters.get(filters.len() - 3..), Some(&[("Text", TEXT_EXTS), ("RTF", RTF_EXTS), word][..]));
    assert!(
        !cmd::fileio::open_filters().any(|(_, exts)| ["txt", "rtf", "docx", "doc"].iter().any(|e| exts.contains(e))),
        "Open doesn't read text, RTF or Word files"
    );
}
