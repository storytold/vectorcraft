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
    assert!(image::load_from_memory(&png).is_ok(), "still a valid PNG");
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
