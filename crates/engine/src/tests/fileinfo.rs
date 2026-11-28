//! File → File Info (`file.info`): editing as one undo step, the native round trip, and the PDF,
//! PNG and SVG metadata it writes.

use serde_json::{Value, json};
use vectorcraft_doc::{CopyrightStatus, DocMetadata};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "title": "Poster"})).unwrap();
    s
}

fn info() -> Value {
    json!({
        "author": "Ada Lovelace",
        "authorTitle": "Designer",
        "description": "A poster\nfor the fair",
        "keywords": "fair, Poster; print, poster",
        "rating": 4,
        "copyrightStatus": "copyrighted",
        "copyrightNotice": "(c) 2026 Ada",
        "copyrightUrl": "https://example.com/rights",
    })
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

#[test]
fn file_info_is_one_undo_step_and_reports_everything() {
    let mut s = session();
    let r = s.execute("file.info", &info()).unwrap();
    assert_eq!(r["keywords"], json!(["fair", "Poster", "print"]), "keywords split, trimmed, each once ignoring case");
    assert_eq!((r["rating"].as_u64(), r["copyrightStatus"].as_str()), (Some(4), Some("copyrighted")));
    assert_eq!(r["description"], "A poster\nfor the fair");
    assert!(r["created"].as_str().is_some_and(|c| c.ends_with('Z')), "a new document has its creation date: {}", r["created"]);
    assert!(r["modified"].is_null(), "never saved");
    assert_eq!(r["title"], "Poster");
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), 1);
    assert_eq!(st.doc.metadata.author, "Ada Lovelace");
    // The report fed back changes nothing (read-only values are ignored).
    let again = s.execute("file.info", &r).unwrap();
    assert_eq!(again, r);
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    let m = &s.doc().unwrap().doc.metadata;
    assert_eq!((m.author.as_str(), m.keywords.len()), ("", 0));
    // Clearing a field with null.
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("file.info", &json!({"author": null, "keywords": []})).unwrap();
    let m = &s.doc().unwrap().doc.metadata;
    assert_eq!((m.author.as_str(), m.keywords.len(), m.rating), ("", 0, 4));
}

#[test]
fn junk_is_rejected() {
    let mut s = session();
    for bad in [
        json!({"rating": 9}),
        json!({"rating": -1}),
        json!({"rating": "five"}),
        json!({"keywords": 5}),
        json!({"keywords": ["ok", 3]}),
        json!({"copyrightStatus": "maybe"}),
        json!({"copyrightUrl": "not a url"}),
        json!({"author": "two\nlines"}),
        json!({"description": "bell\u{7}"}),
        json!({"title": "  "}),
        json!({"author": 3}),
        json!({"favouriteColour": "blue"}),
        json!([1, 2]),
        json!({"keywords": "x".repeat(300)}),
    ] {
        assert!(s.execute("file.info", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), 0);
    assert_eq!(s.doc().unwrap().doc.metadata, DocMetadata { created: s.doc().unwrap().doc.metadata.created, ..Default::default() });
}

/// A control character inside a value is refused with the field's rule: Description and Copyright
/// Notice take line breaks and tabs, the other text fields and each keyword in a list take none.
#[test]
fn control_characters_are_refused_with_the_field_rule() {
    let mut s = session();
    for (params, msg) in [
        (json!({"description": "a\u{7}b"}), "description has control characters"),
        (json!({"copyrightNotice": "a\u{0}b"}), "copyrightNotice has control characters"),
        (json!({"title": "a\nb"}), "title has control characters or line breaks"),
        (json!({"author": "a\rb"}), "author has control characters or line breaks"),
        (json!({"authorTitle": "a\tb"}), "authorTitle has control characters or line breaks"),
        (json!({"copyrightUrl": "a\u{7}b"}), "copyrightUrl has control characters or line breaks"),
        (json!({"keywords": ["ok", "a\nb"]}), "keywords has control characters or line breaks"),
    ] {
        match s.execute("file.info", &params) {
            Err(EngineError::BadParams { cmd, msg: got }) => assert_eq!((cmd.as_str(), got.as_str()), ("file.info", msg), "{params}"),
            other => panic!("{params}: {other:?}"),
        }
    }
    s.execute("file.info", &json!({"description": "two\nlines\tand a tab", "copyrightNotice": "a\r\nb"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.metadata.description, "two\nlines\tand a tab");
}

#[test]
fn file_info_round_trips_and_old_files_load() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(d)).unwrap();
    assert_eq!(back.metadata, d.metadata);
    assert_eq!(back.metadata.copyright_status, CopyrightStatus::Copyrighted);
    // A document saved before File Info existed has none.
    let mut old = vectorcraft_doc::Document::new(10.0, 10.0);
    old.metadata = DocMetadata::default();
    let bytes = vectorcraft_format::save_file(&old);
    assert!(!String::from_utf8_lossy(&bytes).contains("\"metadata\""), "default File Info isn't written");
    assert_eq!(vectorcraft_format::load(&bytes).unwrap().metadata, DocMetadata::default());
}

#[test]
fn saving_to_a_file_stamps_the_modified_date() {
    let mut s = session();
    let created = s.doc().unwrap().doc.metadata.created;
    let path = std::env::temp_dir().join(format!("vc-fileinfo-{}.vectorcraft", std::process::id()));
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.metadata.modified.is_some() && st.doc.metadata.created == created);
    assert!(!st.is_dirty(), "the stamped document is the saved one");
    let back = vectorcraft_format::load(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(back.metadata.modified, st.doc.metadata.modified);
    let _ = std::fs::remove_file(&path);
    assert!(s.execute("file.info", &json!({})).unwrap()["modified"].is_string());
}

#[test]
fn inspect_and_open_report_the_file_info_title_beside_the_file_name() {
    let dir = std::env::temp_dir().join(format!("vc-fileinfotitle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("poster.vectorcraft");
    let mut s = session();
    // Untitled, the two are the same.
    let r = s.execute("document.inspect", &json!({"depth": 0})).unwrap();
    assert_eq!((r["title"].as_str(), r["fileInfoTitle"].as_str()), (Some("Poster"), Some("Poster")));
    s.execute("file.info", &json!({"title": "Poster: final"})).unwrap();
    s.execute("document.save", &json!({"path": path.to_str().unwrap()})).unwrap();
    let r = s.execute("document.inspect", &json!({"depth": 0})).unwrap();
    assert_eq!((r["title"].as_str(), r["fileInfoTitle"].as_str()), (Some("poster.vectorcraft"), Some("Poster: final")));
    // Opened from the file, in another session.
    let mut s = Session::new();
    let opened = s.execute("document.open", &json!({"path": path.to_str().unwrap()})).unwrap();
    assert_eq!((opened["title"].as_str(), opened["fileInfoTitle"].as_str()), (Some("poster.vectorcraft"), Some("Poster: final")));
    let r = s.execute("document.inspect", &json!({"depth": 0})).unwrap();
    assert_eq!((r["title"].as_str(), r["fileInfoTitle"].as_str()), (Some("poster.vectorcraft"), Some("Poster: final")));
    assert_eq!(r["fileInfoTitle"], s.execute("file.info", &json!({})).unwrap()["title"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pdf_info_carries_author_subject_and_keywords() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let pdf = crate::export_pdf(&s.doc().unwrap().doc, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    // (The description has a line break, so it is written as UTF-16.)
    for want in ["/Author(Ada Lovelace)", "/Keywords(fair, Poster, print)", "/Title(Poster)", "/Subject<FEFF0041"] {
        assert!(text.contains(want), "PDF Info has {want}");
    }
    // XMP too.
    assert!(text.contains("<dc:creator>") && text.contains("Ada Lovelace"));
}

#[test]
fn an_exported_pdf_opens_with_its_file_info() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let exported = s.execute("document.export", &json!({"format": "pdf", "preserveEditing": false})).unwrap();
    let r = s.execute("document.open", &json!({"name": "poster.pdf", "dataBase64": exported["dataBase64"]})).unwrap();
    assert_eq!(r["restored"], false, "opened from its pages");
    let back = s.execute("file.info", &json!({})).unwrap();
    assert_eq!(back["author"], "Ada Lovelace");
    assert_eq!(back["description"], "A poster\nfor the fair");
    assert_eq!(back["keywords"], json!(["fair", "Poster", "print"]));
    assert!(back["created"].as_str().is_some_and(|c| c.ends_with('Z')), "the file's creation date: {}", back["created"]);
    assert_eq!(back["title"], "Poster", "File Info shows the title the PDF carries");
    assert_eq!(s.doc().unwrap().title(), "poster.pdf", "the document stays named after the file");
    assert_eq!(back["copyrightStatus"], "copyrighted");
    assert_eq!(back["copyrightNotice"], "(c) 2026 Ada");
    assert_eq!(back["copyrightUrl"], "https://example.com/rights");
}

#[test]
fn a_pdf_compatible_ai_carries_the_copyright() {
    let mut s = session();
    let rights = json!({"copyrightStatus": "publicDomain", "copyrightNotice": "No rights reserved", "copyrightUrl": "https://example.com/cc0"});
    s.execute("file.info", &rights).unwrap();
    let path = std::env::temp_dir().join(format!("vc-fileinfo-rights-{}.ai", std::process::id()));
    let saved = s.execute("document.save", &json!({"path": path.to_string_lossy()}));
    let ai = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    saved.unwrap();
    let f = vectorcraft_pdf::file_info(&ai.unwrap(), None).expect("a PDF-compatible .ai");
    assert_eq!(f.copyrighted, Some(false));
    assert_eq!(f.copyright_notice.as_deref(), Some("No rights reserved"));
    assert_eq!(f.copyright_url.as_deref(), Some("https://example.com/cc0"));
}

/// Unix seconds of 2020-01-01 00:00:00 UTC.
const CREATED: i64 = 1_577_836_800;

/// A session whose document was created at `created` (Unix seconds, or `null` for none).
fn created_at(created: Value) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "created": created})).unwrap();
    s
}

/// The PDF date the writer gives Unix seconds `t` (UTC).
fn pdf_date(t: i64) -> String {
    let [y, mo, d, h, mi, s] = vectorcraft_doc::metadata::civil(t);
    format!("(D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z)")
}

/// Check that a PDF or PDF-compatible `.ai` was created at a time in `created` and modified at a
/// time in `modified`: in the XMP packet (`vectorcraft_pdf::file_info`) and in the document
/// information dictionary.
fn assert_dated(bytes: &[u8], created: std::ops::RangeInclusive<i64>, modified: std::ops::RangeInclusive<i64>) {
    let xmp = vectorcraft_pdf::file_info(bytes, None).expect("a PDF");
    assert!(xmp.created.is_some_and(|t| created.contains(&t)), "xmp:CreateDate {:?} in {created:?}", xmp.created);
    assert!(xmp.modified.is_some_and(|t| modified.contains(&t)), "xmp:ModifyDate {:?} in {modified:?}", xmp.modified);
    let text = String::from_utf8_lossy(bytes);
    let trailer = &text[text.rfind("trailer").expect("a trailer")..];
    let n = trailer.split("/Info ").nth(1).and_then(|r| r.split(' ').next()).expect("an /Info entry");
    let info = &text[text.find(&format!("\n{n} 0 obj")).expect("the document information")..];
    let info = &info[..info.find("endobj").expect("its end")];
    assert!(created.clone().any(|t| info.contains(&format!("/CreationDate{}", pdf_date(t)))), "/CreationDate in {created:?}: {info}");
    assert!(modified.clone().any(|t| info.contains(&format!("/ModDate{}", pdf_date(t)))), "/ModDate in {modified:?}: {info}");
}

/// Run `f` → (its result, the time of writing: the Unix seconds before and after it).
fn written<T>(f: impl FnOnce() -> T) -> (T, std::ops::RangeInclusive<i64>) {
    let before = vectorcraft_doc::metadata::output_time().unwrap();
    let out = f();
    (out, before..=vectorcraft_doc::metadata::output_time().unwrap())
}

#[test]
fn an_exported_pdf_is_created_with_the_document_and_modified_when_it_is_written() {
    let mut s = created_at(json!(CREATED));
    for p in [json!({}), json!({"preset": "PDF/X-4:2010"})] {
        let (pdf, now) = written(|| b64(&s.execute("document.exportPdf", &p).unwrap()));
        assert_dated(&pdf, CREATED..=CREATED, now);
    }
    let (pdf, now) = written(|| b64(&s.execute("document.export", &json!({"format": "pdf", "preserveEditing": false})).unwrap()));
    assert_dated(&pdf, CREATED..=CREATED, now);
}

#[test]
fn a_pdf_compatible_ai_is_created_with_the_document_and_modified_when_it_is_written() {
    let mut s = created_at(json!(CREATED));
    let path = std::env::temp_dir().join(format!("vc-fileinfo-created-{}.ai", std::process::id()));
    let (saved, now) = written(|| s.execute("document.save", &json!({"path": path.to_string_lossy()})));
    saved.unwrap();
    let ai = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_dated(&ai, CREATED..=CREATED, now);
    assert_eq!(s.doc().unwrap().doc.metadata.created, Some(CREATED), "the save keeps the creation date");
}

#[test]
fn a_pdf_of_a_document_without_a_creation_date_is_dated_when_it_is_written() {
    let mut s = created_at(Value::Null);
    assert_eq!(s.doc().unwrap().doc.metadata.created, None);
    let (pdf, now) = written(|| b64(&s.execute("document.exportPdf", &json!({})).unwrap()));
    assert_dated(&pdf, now.clone(), now);
    let xmp = vectorcraft_pdf::file_info(&pdf, None).unwrap();
    assert_eq!(xmp.created, xmp.modified, "created and modified at the same time of writing");
}

#[test]
fn a_pdf_compatible_ai_opens_with_the_file_info_of_its_xmp_packet() {
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with_catalog};
    let xmp = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
               <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\" xmpRights:Marked=\"False\">\
               <dc:creator><rdf:Seq><rdf:li>Grace Hopper</rdf:li></rdf:Seq></dc:creator>\
               <dc:subject><rdf:Bag><rdf:li>navy</rdf:li><rdf:li>cobol</rdf:li></rdf:Bag></dc:subject>\
               <dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">No rights reserved</rdf:li></rdf:Alt></dc:rights>\
               <xmpRights:WebStatement>https://example.com/a page</xmpRights:WebStatement>\
               </rdf:Description></rdf:RDF></x:xmpmeta>";
    let stream = format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{xmp}\nendstream", xmp.len());
    let catalog = format!("/Metadata {} 0 R ", first_extra(1));
    let bytes = pdf_with_catalog(&[PdfPage::new(100.0, 100.0, "0 0 1 rg 10 10 50 50 re f")], &[&stream], &catalog, None);
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "notes.ai", "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    let m = &s.doc().unwrap().doc.metadata;
    assert_eq!((m.author.as_str(), m.keywords.clone()), ("Grace Hopper", vec!["navy".to_string(), "cobol".into()]));
    assert_eq!((m.copyright_status, m.copyright_notice.as_str()), (CopyrightStatus::PublicDomain, "No rights reserved"));
    // A URL with a space is one `file.info` refuses: it is left out.
    assert_eq!(m.copyright_url, "");
}

/// A one-page PDF whose trailer names an information dictionary with the entries `info`, and whose
/// catalog has a metadata stream holding an XMP `dc:title` of `xmp_title`.
fn pdf_titled(info: Option<&str>, xmp_title: Option<&str>) -> Vec<u8> {
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with_catalog};
    let first = first_extra(1);
    let mut objects = vec![];
    let mut catalog = String::new();
    if let Some(t) = xmp_title {
        let xmp = format!(
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
             <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\
             <dc:title><rdf:Alt><rdf:li xml:lang=\"de\">Zugangswege</rdf:li><rdf:li xml:lang=\"x-default\">{t}</rdf:li></rdf:Alt></dc:title>\
             </rdf:Description></rdf:RDF></x:xmpmeta>"
        );
        objects.push(format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{xmp}\nendstream", xmp.len()));
        catalog = format!("/Metadata {first} 0 R ");
    }
    let info_at = first + objects.len();
    if let Some(i) = info {
        objects.push(format!("<< {i} >>"));
    }
    let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
    let bytes = pdf_with_catalog(&[PdfPage::new(100.0, 100.0, "0 0 1 rg 10 10 50 50 re f")], &refs, &catalog, None);
    if info.is_none() {
        return bytes;
    }
    // The trailer follows the cross-reference table, so a longer trailer moves no object.
    String::from_utf8(bytes).unwrap().replacen("/Root 1 0 R", &format!("/Root 1 0 R /Info {info_at} 0 R"), 1).into_bytes()
}

/// A document called `name` with content `bytes` opened (as `document.open` with no path: the tab
/// shows the document's name), `s` holding it.
fn open_named(s: &mut Session, name: &str, bytes: &[u8]) -> Value {
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap()
}

/// The document's name (its tab) and its File Info title.
fn titles(s: &mut Session) -> [String; 2] {
    let info = s.execute("file.info", &json!({})).unwrap();
    [s.doc().unwrap().title(), info["title"].as_str().unwrap().to_string()]
}

/// What opening `bytes` as a file called `name` gives: the document's name and its File Info title.
fn opened(name: &str, bytes: &[u8]) -> [String; 2] {
    let mut s = Session::new();
    let r = open_named(&mut s, name, bytes);
    let t = titles(&mut s);
    assert_eq!((r["title"].as_str(), r["fileInfoTitle"].as_str()), (Some(t[0].as_str()), Some(t[1].as_str())), "the open reply");
    assert_eq!(s.doc().unwrap().doc.title, t[0], "the tab is the document's title");
    t
}

fn svg_titled(inner: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\">{inner}<rect width=\"5\" height=\"5\"><title>Tooltip</title></rect></svg>"
    )
}

#[test]
fn import_puts_the_files_own_title_in_file_info_only() {
    // SVG: the root's `<title>`, its white space made single spaces (not a shape's tooltip), is the
    // File Info title; the document keeps the file's name, with its extension.
    assert_eq!(opened("box.svg", svg_titled("<title>\n  Access\n  paths </title>").as_bytes()), ["box.svg", "Access paths"]);
    assert_eq!(
        opened("/work/box.svg", svg_titled("<title>Zugangswege Rechenzentrum – Prüfdiagramm</title>").as_bytes()),
        ["box.svg", "Zugangswege Rechenzentrum – Prüfdiagramm"]
    );
    // PDF: the XMP `dc:title` (its `x-default` item) beats the Info `Title`; the Info `Title` stands alone.
    assert_eq!(opened("plan.pdf", &pdf_titled(Some("/Title (Info title)"), Some("XMP title"))), ["plan.pdf", "XMP title"]);
    assert_eq!(opened("plan.pdf", &pdf_titled(None, Some("XMP title"))), ["plan.pdf", "XMP title"]);
    assert_eq!(opened("plan.pdf", &pdf_titled(Some("/Title (Info title)"), None)), ["plan.pdf", "Info title"]);
    assert_eq!(opened("plan.ai", &pdf_titled(Some("/Title (Info title)"), None)), ["plan.ai", "Info title"]);
    // Compressed SVG too.
    let svgz = |inner: &str| vectorcraft_svg::compress(&svg_titled(inner));
    assert_eq!(opened("box.svgz", &svgz("<title>Access paths</title>")), ["box.svgz", "Access paths"]);
    // A file that gives none (or a blank one) gives File Info the document's name.
    assert_eq!(opened("box.svg", svg_titled("").as_bytes()), ["box.svg", "box.svg"]);
    assert_eq!(opened("box.svg", svg_titled("<title>  </title>").as_bytes()), ["box.svg", "box.svg"]);
    assert_eq!(opened("plan.pdf", &pdf_titled(None, None)), ["plan.pdf", "plan.pdf"]);
    assert_eq!(opened("plan.pdf", &pdf_titled(Some("/Author (Ada)"), None)), ["plan.pdf", "plan.pdf"]);
    // The Untitled-N name New gives is no title (a document that never had one writes it).
    assert_eq!(opened("plan.pdf", &pdf_titled(Some("/Title (Untitled-2)"), None)), ["plan.pdf", "plan.pdf"]);
    // A template opens as a new untitled document, whatever title its file gave.
    let [name, info] = opened("plan.ait", &pdf_titled(Some("/Title (Info title)"), None));
    assert!(name.starts_with("Untitled-") && info == name, "{name} / {info}");
}

#[test]
fn imported_svg_titles_are_filtered_and_bounded() {
    for (raw, expected) in [
        ("Untitled-2".to_string(), None),
        ("\u{7f}".to_string(), None),
        ("Re\u{7f}lease  Plan".to_string(), Some("Release Plan".to_string())),
        ("λ".repeat(65_537), Some("λ".repeat(65_536))),
    ] {
        let svg = svg_titled(&format!("<title>{raw}</title>"));
        for format in ["svg", "svgz"] {
            let name = format!("box.{format}");
            let bytes = if format == "svgz" { vectorcraft_svg::compress(&svg) } else { svg.as_bytes().to_vec() };
            assert_eq!(opened(&name, &bytes), [name.clone(), expected.clone().unwrap_or(name)], "{format}");
        }
    }
}

#[test]
fn a_file_with_a_title_opened_from_a_path_is_named_after_the_file() {
    let dir = std::env::temp_dir().join(format!("vc-infotitle-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("box.svg");
    std::fs::write(&path, svg_titled("<title>Access paths</title>")).unwrap();
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"path": path.to_str().unwrap()})).unwrap();
    assert_eq!((r["title"].as_str(), r["fileInfoTitle"].as_str()), (Some("box.svg"), Some("Access paths")));
    let r = s.execute("document.inspect", &json!({"depth": 0})).unwrap();
    assert_eq!((r["title"].as_str(), r["fileInfoTitle"].as_str()), (Some("box.svg"), Some("Access paths")));
    // The default names of what the document exports are made from the file's name, not its title.
    assert_eq!(s.doc().unwrap().doc.title, "box.svg");
    assert_eq!(crate::cmd::fileio::default_name(&s.doc().unwrap().doc, "png"), "box.png");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn file_info_edits_a_files_own_title_and_the_document_follows() {
    let mut s = Session::new();
    open_named(&mut s, "box.svg", svg_titled("<title>Access paths</title>").as_bytes());
    // The dialog sends back the title it was given: nothing changes (no undo step).
    let before = s.doc().unwrap().revision;
    let shown = s.execute("file.info", &json!({})).unwrap();
    s.execute("file.info", &shown).unwrap();
    assert_eq!(s.doc().unwrap().revision, before, "the title it gave is no edit");
    assert_eq!(titles(&mut s), ["box.svg", "Access paths"]);
    // Another title is the document's own, as ever, and the file's is gone.
    let r = s.execute("file.info", &json!({"title": "Plan B"})).unwrap();
    assert_eq!(r["title"], "Plan B");
    assert_eq!(titles(&mut s), ["Plan B", "Plan B"]);
    assert_eq!(s.doc().unwrap().doc.metadata.title, None);
    // Undo brings the file's title back, and the name.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(titles(&mut s), ["box.svg", "Access paths"]);
}

#[test]
fn restored_file_info_title_keeps_authored_spaces() {
    let mut s = session();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).metadata.title = Some("Release  Plan".into());
    for format in ["vectorcraft", "svg", "pdf"] {
        let bytes = b64(&s.execute("document.export", &json!({"format": format, "preserveEditing": true})).unwrap());
        let mut reopened = Session::new();
        let reply = open_named(&mut reopened, &format!("again.{format}"), &bytes);
        assert_eq!(reply["restored"], format != "vectorcraft", "{format}");
        assert_eq!(reopened.doc().unwrap().doc.metadata.title.as_deref(), Some("Release  Plan"), "{format}");
        assert_eq!(titles(&mut reopened)[1], "Release  Plan", "{format}");
    }
}

#[test]
fn the_files_own_title_is_kept_by_saves_and_written_by_exports() {
    let mut s = Session::new();
    open_named(&mut s, "box.svg", svg_titled("<title>Access paths</title>").as_bytes());
    // The native file: its own title is the document's name, the File Info title survives.
    let native = b64(&s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap());
    let mut t = Session::new();
    open_named(&mut t, "other.vectorcraft", &native);
    assert_eq!(titles(&mut t), ["box.svg", "Access paths"]);
    // An SVG with the editing data it carries, reopened under another name.
    let svg = s.execute("document.serialize", &json!({"format": "svg", "preserveEditing": true, "metadata": true})).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(svg.contains("<title>Access paths</title>") && svg.contains("<dc:title>Access paths</dc:title>"), "{svg}");
    let mut u = Session::new();
    let r = open_named(&mut u, "again.svg", svg.as_bytes());
    assert_eq!(r["restored"], true);
    assert_eq!(titles(&mut u), ["again.svg", "Access paths"]);
    // PDF: the Info `Title` and the XMP `dc:title`, shown by the viewer.
    for pdf in pdf_texts(&mut s) {
        assert!(pdf.contains("/Title(Access paths)") && xmp_title(&pdf).contains("Access paths"), "{}", xmp_title(&pdf));
        assert!(pdf.contains("/ViewerPreferences<</DisplayDocTitle true>>"), "{pdf}");
    }
    // PNG text.
    let png = b64(&s.execute("document.export", &json!({"format": "png"})).unwrap());
    assert!(crate::cmd::fileio::pngtext::text(&png).iter().any(|(k, v)| k == "Title" && v == "Access paths"));
    // The default file names are made from the document's name.
    assert_eq!(crate::cmd::fileio::default_name(&s.doc().unwrap().doc, "pdf"), "box.pdf");
}

#[test]
fn png_exports_carry_text_chunks() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let png = b64(&s.execute("document.export", &json!({"format": "png"})).unwrap());
    let text = crate::cmd::fileio::pngtext::text(&png);
    let get = |k: &str| text.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    assert_eq!(get("Title"), Some("Poster"));
    assert_eq!(get("Author"), Some("Ada Lovelace"));
    assert_eq!(get("Keywords"), Some("fair, Poster, print"));
    assert_eq!(get("Copyright"), Some("(c) 2026 Ada"));
    assert!(get("Creation Time").is_some_and(|t| t.ends_with('Z')));
    assert!(
        craft_image_codecs::decode(&png)
            .map(|__cimg| {
                let (__w, __h) = __cimg.dimensions();
                match (__cimg.layout(), __cimg.sample_type()) {
                    (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
                }
            })
            .is_ok(),
        "still a valid PNG"
    );
}

#[test]
fn svg_metadata_carries_file_info_as_dublin_core() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let svg = String::from_utf8(b64(&s.execute("document.export", &json!({"format": "svg", "metadata": true})).unwrap())).unwrap();
    for want in [
        "<dc:title>Poster</dc:title>",
        "<dc:creator>Ada Lovelace</dc:creator>",
        "<dc:subject>fair</dc:subject>",
        "<dc:subject>print</dc:subject>",
        "<dc:rights>(c) 2026 Ada</dc:rights>",
        "<dc:rights>https://example.com/rights</dc:rights>",
        "<desc>A poster\nfor the fair</desc>",
    ] {
        assert!(svg.contains(want), "SVG has {want}");
    }
    // Without the option: no <metadata>, but the title and description stay.
    let plain = String::from_utf8(b64(&s.execute("document.export", &json!({"format": "svg"})).unwrap())).unwrap();
    assert!(!plain.contains("<metadata>") && plain.contains("<desc>"));
}

/// The PDF `document.exportPdf` and `document.export` write of the active document (text
/// uncompressed, so its metadata reads).
fn pdf_texts(s: &mut Session) -> [String; 2] {
    let p = json!({"compression": {"compressText": false}, "preserveEditing": false});
    let a = b64(&s.execute("document.exportPdf", &p).unwrap());
    let b = b64(&s.execute("document.export", &json!({"format": "pdf", "compression": {"compressText": false}, "preserveEditing": false})).unwrap());
    [a, b].map(|pdf| String::from_utf8_lossy(&pdf).into_owned())
}

/// The XMP `dc:title` of a PDF's text.
fn xmp_title(pdf: &str) -> &str {
    let from = pdf.find("<dc:title>").expect("XMP title");
    &pdf[from..from + pdf[from..].find("</dc:title>").unwrap()]
}

#[test]
fn an_untitled_document_saved_to_a_file_exports_titled_after_it() {
    let dir = std::env::temp_dir().join(format!("vc-pdftitle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Document C.vectorcraft");
    // Unsaved and untitled: viewers keep showing the file name, not "Untitled-1".
    let mut untitled = Session::new();
    untitled.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
    for pdf in pdf_texts(&mut untitled) {
        assert!(!pdf.contains("/DisplayDocTitle true"), "{pdf}");
    }
    // An explicitly empty document title is also not a title to display.
    std::sync::Arc::make_mut(&mut untitled.doc_mut().unwrap().doc).title.clear();
    for pdf in pdf_texts(&mut untitled) {
        assert!(!pdf.contains("/DisplayDocTitle true"), "{pdf}");
    }

    // Saved as a file, then exported: Untitled-1 was only the name New gave it.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
    s.execute("document.save", &json!({"path": path.to_str().unwrap()})).unwrap();
    for pdf in pdf_texts(&mut s) {
        assert!(pdf.contains("/Title(Document C)") && xmp_title(&pdf).contains("Document C"), "{}", xmp_title(&pdf));
    }
    // Non-object parameters still preserve that the file-stem title was not user supplied.
    let pdf = String::from_utf8_lossy(&b64(&s.execute("document.exportPdf", &Value::Null).unwrap())).into_owned();
    assert!(!pdf.contains("/DisplayDocTitle true"), "{pdf}");
    // Opened from that file, in another session.
    let mut s = Session::new();
    s.execute("document.open", &json!({"path": path.to_str().unwrap()})).unwrap();
    for pdf in pdf_texts(&mut s) {
        assert!(pdf.contains("/Title(Document C)") && !pdf.contains("Untitled"), "{}", xmp_title(&pdf));
    }
    // The document's own title is left as it was.
    assert_eq!(s.doc().unwrap().doc.title, "Untitled-1");
    // A title of its own (File Info, New Document's Name) stays.
    let mut s = session();
    s.execute("document.save", &json!({"path": dir.join("Document D.vectorcraft").to_str().unwrap()})).unwrap();
    for pdf in pdf_texts(&mut s) {
        assert!(pdf.contains("/Title(Poster)") && xmp_title(&pdf).contains("Poster"));
        assert!(pdf.contains("/ViewerPreferences<</DisplayDocTitle true>>"), "{pdf}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A session whose metadata templates folder is `dir/Metadata Templates`, with a document titled
/// Poster.
fn template_session(dir: &std::path::Path) -> Session {
    let mut s = session();
    s.attach_metadata_templates_dir(dir.join("Metadata Templates"));
    s
}

#[test]
fn a_saved_metadata_template_is_listed_and_applies_to_another_document() {
    let dir = vectorcraft_testkit::TempDir::new("fileinfo-template");
    let mut s = template_session(&dir);
    assert_eq!(s.execute("file.info.templates", &json!({})).unwrap()["templates"], json!([]), "no folder yet");
    s.execute("file.info", &info()).unwrap();
    let r = s.execute("file.info.saveTemplate", &json!({"name": " Studio "})).unwrap();
    assert_eq!(r["name"], "Studio");
    let path = dir.join("Metadata Templates").join("Studio.xmp");
    assert!(path.is_file(), "saved into the folder: {r}");
    let listed = s.execute("file.info.templates", &json!({})).unwrap();
    assert_eq!(listed["templates"][0]["name"], "Studio");
    assert_eq!(listed["templates"].as_array().map(Vec::len), Some(1));
    // A name that isn't a usable file name as it is is refused, so `a/b` and `a?b` never share a file.
    for name in ["a/b", "a?b", "CON", "Studio.", "x\u{1}"] {
        assert!(s.execute("file.info.saveTemplate", &json!({"name": name})).is_err(), "{name}");
    }
    assert_eq!(s.execute("file.info.templates", &json!({})).unwrap()["templates"].as_array().map(Vec::len), Some(1));
    // Saving is not an edit of the document.
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    // Into a new document, by name, in one undo step.
    s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
    let r = s.execute("file.info.applyTemplate", &json!({"name": "Studio", "mode": "keep"})).unwrap();
    assert_eq!(r["changed"], true);
    let m = &s.doc().unwrap().doc.metadata;
    assert_eq!((m.author.as_str(), m.author_title.as_str(), m.rating), ("Ada Lovelace", "Designer", 4));
    assert_eq!(m.keywords, ["fair", "Poster", "print"]);
    assert_eq!((m.copyright_status, m.copyright_notice.as_str()), (CopyrightStatus::Copyrighted, "(c) 2026 Ada"));
    assert_eq!(m.description, "A poster\nfor the fair");
    assert_eq!(r["title"], "Poster", "the template carries the title of its own");
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    // Applied again, nothing changes and nothing is recorded.
    let r = s.execute("file.info.applyTemplate", &json!({"path": path.to_str().unwrap(), "mode": "keep"})).unwrap();
    assert_eq!(r["changed"], false);
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.metadata.author, "");
    // The same name replaces the template.
    s.execute("file.info.saveTemplate", &json!({"name": "Studio", "fields": {"author": "Grace"}})).unwrap();
    let back = vectorcraft_pdf::read_template(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(back.author.as_deref(), Some("Grace"));
    assert_eq!(back.title, None, "an Untitled-N title is not written");
}

#[test]
fn a_metadata_template_merges_in_three_modes() {
    let mut s = session();
    s.execute("file.info", &json!({"author": "Ada", "keywords": ["fair"], "rating": 2, "copyrightUrl": "https://example.com/a"})).unwrap();
    let template = vectorcraft_pdf::metadata_template(&vectorcraft_pdf::FileInfo {
        author: Some("Grace".into()),
        description: Some("From the template".into()),
        keywords: Some(vec!["print".into(), "Fair".into()]),
        rating: Some(5),
        copyrighted: Some(false),
        ..Default::default()
    });
    let merged =
        |s: &mut Session, mode: &str| s.execute("file.info.applyTemplate", &json!({"data": template, "mode": mode, "apply": false})).unwrap();
    // Clear and replace: the fields the template lacks are cleared; the title stays.
    let r = merged(&mut s, "replace");
    assert_eq!((r["author"].as_str(), r["description"].as_str(), r["rating"].as_u64()), (Some("Grace"), Some("From the template"), Some(5)));
    assert_eq!(
        (r["keywords"].clone(), r["copyrightUrl"].as_str(), r["copyrightStatus"].as_str()),
        (json!(["print", "Fair"]), Some(""), Some("publicDomain"))
    );
    assert_eq!(r["title"], "Poster");
    // Keep and replace: the template's fields replace those it has; the others stay.
    let r = merged(&mut s, "keep");
    assert_eq!((r["author"].as_str(), r["rating"].as_u64(), r["copyrightUrl"].as_str()), (Some("Grace"), Some(5), Some("https://example.com/a")));
    assert_eq!(r["keywords"], json!(["print", "Fair"]));
    // Keep and append: keywords are added (each once), and only empty fields are filled.
    let r = merged(&mut s, "append");
    assert_eq!((r["author"].as_str(), r["rating"].as_u64(), r["description"].as_str()), (Some("Ada"), Some(2), Some("From the template")));
    assert_eq!((r["keywords"].clone(), r["copyrightStatus"].as_str()), (json!(["fair", "print"]), Some("publicDomain")));
    // `apply: false` changed nothing; `base` merges into the given fields instead.
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    assert_eq!(s.doc().unwrap().doc.metadata.author, "Ada");
    let r = s.execute("file.info.applyTemplate", &json!({"data": template, "mode": "append", "apply": false, "base": {"author": ""}})).unwrap();
    assert_eq!(r["author"], "Grace");
    // Applied: one undo step.
    s.execute("file.info.applyTemplate", &json!({"data": template, "mode": "replace"})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), 2);
    assert_eq!(s.doc().unwrap().doc.metadata.copyright_url, "");
}

#[test]
fn metadata_template_errors() {
    let dir = vectorcraft_testkit::TempDir::new("fileinfo-template-errors");
    let mut s = template_session(&dir);
    let xmp = vectorcraft_pdf::metadata_template(&vectorcraft_pdf::FileInfo { author: Some("Ada".into()), ..Default::default() });
    let not_xmp = dir.join("note.xmp");
    std::fs::write(&not_xmp, "<note>not metadata</note>").unwrap();
    let big = dir.join("big.xmp");
    std::fs::write(&big, vec![b' '; vectorcraft_pdf::MAX_TEMPLATE_BYTES + 1]).unwrap();
    let e = s.execute("file.info.applyTemplate", &json!({"path": big.to_str().unwrap(), "mode": "keep"})).unwrap_err();
    assert!(e.to_string().contains("larger than 8 MB"), "{e}");
    let e = s.execute("file.info.applyTemplate", &json!({"data": " ".repeat(vectorcraft_pdf::MAX_TEMPLATE_BYTES + 1), "mode": "keep"})).unwrap_err();
    assert!(e.to_string().contains("larger than 8 MB"), "{e}");
    let encoded = "A".repeat(vectorcraft_pdf::MAX_TEMPLATE_BYTES.div_ceil(3) * 4 + 1);
    let e = s.execute("file.info.applyTemplate", &json!({"dataBase64": encoded, "mode": "keep"})).unwrap_err();
    assert!(e.to_string().contains("larger than 8 MB"), "{e}");
    for bad in [
        json!({"data": xmp, "mode": "merge"}),
        json!({"data": xmp}),
        json!({"data": xmp, "mode": 3}),
        json!({"mode": "keep"}),
        json!({"data": xmp, "path": not_xmp.to_str().unwrap(), "mode": "keep"}),
        json!({"data": xmp, "dataBase64": "SGVsbG8=", "mode": "keep"}),
        json!({"dataBase64": "!", "mode": "keep"}),
        json!({"path": dir.join("missing.xmp").to_str().unwrap(), "mode": "keep"}),
        json!({"path": not_xmp.to_str().unwrap(), "mode": "keep"}),
        json!({"path": big.to_str().unwrap(), "mode": "keep"}),
        json!({"name": "missing", "mode": "keep"}),
        json!({"data": "Ada Lovelace", "mode": "keep"}),
        json!({"data": xmp, "mode": "keep", "apply": "yes"}),
        json!({"data": xmp, "mode": "keep", "base": [1]}),
        json!({"data": xmp, "mode": "keep", "base": {"rating": 9}}),
    ] {
        assert!(s.execute("file.info.applyTemplate", &bad).is_err(), "{bad}");
    }
    for bad in [json!({"name": "a", "path": "b.xmp"}), json!({"name": "  "}), json!({"fields": {"rating": 9}}), json!({"fields": "x"})] {
        assert!(s.execute("file.info.saveTemplate", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), 0);
    assert!(!dir.join("Metadata Templates").exists(), "no failed save made the folder");
    // A link in the folder is not followed, by its name or its file name.
    #[cfg(unix)]
    {
        let outside = dir.join("outside.xmp");
        std::fs::write(&outside, &xmp).unwrap();
        std::fs::create_dir_all(dir.join("Metadata Templates")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("Metadata Templates").join("Linked.xmp")).unwrap();
        for name in ["Linked", "Linked.xmp"] {
            assert!(s.execute("file.info.applyTemplate", &json!({"name": name, "mode": "keep"})).is_err(), "{name}");
        }
        let e = s.execute("file.info.applyTemplate", &json!({"name": "Linked", "mode": "keep"})).unwrap_err();
        assert!(e.to_string().contains("not a file"), "{e}");
        assert_eq!(s.execute("file.info.templates", &json!({})).unwrap()["templates"], json!([]));
        // A save by that name leaves the linked file as it was.
        let e = s.execute("file.info.saveTemplate", &json!({"name": "Linked", "fields": {"author": "Grace"}})).unwrap_err();
        assert!(e.to_string().contains("not a file"), "{e}");
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), xmp);
        std::fs::remove_dir_all(dir.join("Metadata Templates")).unwrap();
    }
    // Without a folder (runs without preferences), a name is an error, the data comes back, and
    // the list is empty.
    let mut s = session();
    assert!(s.execute("file.info.saveTemplate", &json!({"name": "Studio"})).is_err());
    let data = s.execute("file.info.saveTemplate", &json!({"fields": {"author": "Ada"}})).unwrap()["data"].as_str().unwrap().to_string();
    assert_eq!(vectorcraft_pdf::read_template(data.as_bytes()).unwrap().author.as_deref(), Some("Ada"));
    assert_eq!(s.execute("file.info.templates", &json!({"create": true})).unwrap(), json!({"templates": []}));
}

#[test]
fn the_templates_folder_lists_only_xmp_files_and_can_be_made() {
    let dir = vectorcraft_testkit::TempDir::new("fileinfo-template-list");
    let mut s = template_session(&dir);
    let r = s.execute("file.info.templates", &json!({"create": true})).unwrap();
    let folder = dir.join("Metadata Templates");
    assert!(folder.is_dir());
    assert_eq!(r["folder"], folder.to_str().unwrap());
    for f in ["b.XMP", "a.xmp", "notes.txt", ".xmp"] {
        std::fs::write(folder.join(f), "x").unwrap();
    }
    std::fs::create_dir(folder.join("c.xmp")).unwrap();
    // Other files, however many, leave the templates listed.
    for i in 0..1000 {
        std::fs::write(folder.join(format!("{i:04}.txt")), "").unwrap();
    }
    let names: Vec<String> = s.execute("file.info.templates", &json!({})).unwrap()["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["a", "b"]);
    // A listed name opens whatever the case of its extension.
    let xmp = vectorcraft_pdf::metadata_template(&vectorcraft_pdf::FileInfo { author: Some("Grace".into()), ..Default::default() });
    std::fs::write(folder.join("b.XMP"), xmp).unwrap();
    let r = s.execute("file.info.applyTemplate", &json!({"name": "b", "mode": "keep"})).unwrap();
    assert_eq!(r["author"], "Grace");
    // Its listed file name opens it too.
    let listed = s.execute("file.info.templates", &json!({})).unwrap();
    assert_eq!(listed["templates"][1]["file"], "b.XMP");
    let r = s.execute("file.info.applyTemplate", &json!({"name": "b.XMP", "mode": "replace", "apply": false})).unwrap();
    assert_eq!(r["author"], "Grace");
    // A listed file name is taken before the name with `.xmp` added.
    let ada = vectorcraft_pdf::metadata_template(&vectorcraft_pdf::FileInfo { author: Some("Ada".into()), ..Default::default() });
    std::fs::write(folder.join("b.XMP.xmp"), ada).unwrap();
    let r = s.execute("file.info.applyTemplate", &json!({"name": "b.XMP", "mode": "replace", "apply": false})).unwrap();
    assert_eq!(r["author"], "Grace");
    let r = s.execute("file.info.applyTemplate", &json!({"name": "b.XMP.xmp", "mode": "replace", "apply": false})).unwrap();
    assert_eq!(r["author"], "Ada");
    // A name that is not a file name never opens the template its safe form names.
    let grace = vectorcraft_pdf::metadata_template(&vectorcraft_pdf::FileInfo { author: Some("Grace".into()), ..Default::default() });
    std::fs::write(folder.join("a-b.xmp"), grace).unwrap();
    let r = s.execute("file.info.applyTemplate", &json!({"name": "a-b", "mode": "replace", "apply": false})).unwrap();
    assert_eq!(r["author"], "Grace");
    for name in ["a/b", "a\\b", "a:b"] {
        let e = s.execute("file.info.applyTemplate", &json!({"name": name, "mode": "replace", "apply": false})).unwrap_err();
        assert!(e.to_string().contains("isn't usable as a file name"), "{name}: {e}");
    }
}

#[test]
fn the_largest_file_info_saves_a_template_that_imports() {
    let mut s = session();
    let text = "\"&".repeat(32_768);
    let keywords: Vec<String> = (0..1000).map(|i| format!("{i:04}{}", "\"".repeat(252))).collect();
    let fields = json!({
        "author": text, "authorTitle": text, "description": text, "copyrightNotice": text,
        "copyrightUrl": format!("https://example.com/{}", "&".repeat(2000)), "keywords": keywords, "rating": 5, "copyrightStatus": "copyrighted",
    });
    let data = s.execute("file.info.saveTemplate", &json!({"fields": fields})).unwrap()["data"].as_str().unwrap().to_string();
    assert!(data.len() > 1 << 20, "{} bytes", data.len());
    let r = s.execute("file.info.applyTemplate", &json!({"data": data, "mode": "replace"})).unwrap();
    assert_eq!((r["author"].as_str(), r["keywords"].as_array().map(Vec::len)), (Some(text.as_str()), Some(1000)));
}

#[test]
fn a_pdf_opens_with_the_rating_and_author_title_its_packet_gives() {
    let mut m = DocMetadata::default();
    let f = vectorcraft_pdf::FileInfo { author_title: Some("Editor".into()), rating: Some(3), ..Default::default() };
    crate::cmd::fileinfo::read_pdf(&mut m, f);
    assert_eq!((m.author_title.as_str(), m.rating), ("Editor", 3));
}
