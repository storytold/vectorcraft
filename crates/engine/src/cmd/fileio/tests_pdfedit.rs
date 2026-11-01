//! PDF and .ai files that reopen as they were: Preserve Editing embeds the native document, which
//! `document.open` restores unless the pages changed elsewhere; `.ai` saves always carry it and
//! `.ait` templates open from it untitled.

use serde_json::{Value, json};

use super::tests_svgedit::{comparable, rich};
use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap_or_else(|| panic!("no dataBase64 in {v}"))).unwrap()
}

fn open(s: &mut Session, name: &str, bytes: &[u8]) -> Value {
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap()
}

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("vc-pdfedit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().to_string()
}

/// `pdf` with `from` replaced by `to` (the same length).
fn patched(pdf: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let at = pdf.windows(from.len()).position(|w| w == from).unwrap_or_else(|| panic!("no {:?}", String::from_utf8_lossy(from)));
    let mut out = pdf.to_vec();
    out[at..at + to.len()].copy_from_slice(to);
    out
}

#[test]
fn a_pdf_saved_with_preserve_editing_reopens_as_the_same_document() {
    let mut s = rich();
    let before = comparable(&s.doc().unwrap().doc);
    // On in the default preset.
    let v = s.execute("document.exportPdf", &json!({})).unwrap();
    assert_eq!(v["warnings"].as_array().map(|w| w.iter().any(|w| w.as_str().unwrap().contains("Preserve"))), Some(false), "{v}");
    let pdf = b64(&v);
    let r = open(&mut s, "rich.pdf", &pdf);
    assert_eq!((&r["warnings"], &r["restored"]), (&json!([]), &json!(true)), "{r}");
    assert_eq!(comparable(&s.doc().unwrap().doc), before, "the document came back exactly");
    assert_eq!(s.doc().unwrap().path, None, "a PDF is not saved over");
    // Picking pages imports them as artwork.
    let r = s.execute("document.open", &json!({"name": "rich.pdf", "dataBase64": vectorcraft_format::base64_encode(&pdf), "pages": "1"})).unwrap();
    assert_eq!(r["restored"], false, "{r}");
    // Off: the pages import as artwork.
    let plain = b64(&s.execute("document.exportPdf", &json!({"preserveEditing": false})).unwrap());
    assert!(vectorcraft_pdf::editing(&plain).is_none());
    assert_eq!(open(&mut s, "plain.pdf", &plain)["restored"], false);
    // document.export and serialize take the same option.
    let v = s.execute("document.serialize", &json!({"format": "pdf"})).unwrap();
    assert!(vectorcraft_pdf::editing(&b64(&v)).is_some_and(|e| e.intact));
}

#[test]
fn pages_changed_elsewhere_open_as_artwork_with_a_warning() {
    let mut s = rich();
    let pdf = b64(&s.execute("document.exportPdf", &json!({"compression": {"compressText": false}})).unwrap());
    // Another app changes the artboard's height (300 × 200 pt).
    let edited = patched(&pdf, b"/MediaBox[0 0 300 200]", b"/MediaBox[0 0 300 201]");
    let r = open(&mut s, "edited.pdf", &edited);
    assert_eq!((&r["warnings"][0], &r["restored"]), (&json!(load::EDITING_STALE), &json!(false)), "{r}");
    assert!(s.doc().unwrap().doc.symbols.is_empty(), "plain import: no symbols");
    // Editing data that isn't a native document can't be read.
    let damaged = patched(&pdf, b"{\"format\":\"vectorcraft\"", b"{\"format\":\"vectorcrafX\"");
    let r = open(&mut s, "damaged.pdf", &damaged);
    assert_eq!(r["warnings"][0], load::EDITING_DAMAGED, "{r}");
}

/// `pdf` with its editing file's description rewritten by another app in UTF-16BE with a byte
/// order mark: "hash " and the hash it held, padded with spaces to its length.
fn utf16_described(pdf: &[u8]) -> Vec<u8> {
    let from = b"(VectorCraft editing data; page hash ";
    let at = pdf.windows(from.len()).position(|w| w == from).unwrap_or_else(|| panic!("no description"));
    let (hash, len) = (&pdf[at + from.len()..at + from.len() + 16], from.len() + 17);
    let mut desc = b"(\xFE\xFF".to_vec();
    desc.extend(b"hash ".iter().chain(hash).flat_map(|&c| [0, c]));
    desc.push(b')');
    desc.resize(len, b' ');
    patched(pdf, &pdf[at..at + len], &desc)
}

#[test]
fn pages_changed_elsewhere_are_noticed_with_a_utf16_description() {
    let mut s = rich();
    let pdf = b64(&s.execute("document.exportPdf", &json!({"compression": {"compressText": false}})).unwrap());
    let ai = b64(&s.execute("document.export", &json!({"format": "ai", "compress": false})).unwrap());
    for (name, file) in [("edited.pdf", pdf), ("edited.ai", ai)] {
        let r = open(&mut s, name, &utf16_described(&file));
        assert_eq!((&r["warnings"], &r["restored"]), (&json!([]), &json!(true)), "{name}: the same pages: {r}");
        let edited = patched(&file, b"/MediaBox[0 0 300 200]", b"/MediaBox[0 0 300 201]");
        let r = open(&mut s, name, &utf16_described(&edited));
        assert_eq!((&r["warnings"][0], &r["restored"]), (&json!(load::EDITING_STALE), &json!(false)), "{name}: {r}");
    }
}

#[test]
fn the_legacy_payload_name_is_accepted() {
    let mut s = rich();
    let before = comparable(&s.doc().unwrap().doc);
    let pdf = b64(&s.execute("document.exportPdf", &json!({"compression": {"compressText": false}})).unwrap());
    let (name, legacy) = (format!("({})", vectorcraft_pdf::EDITING_FILE), format!("({})", vectorcraft_pdf::LEGACY_EDITING_FILE));
    let legacy = format!("{legacy}{}", " ".repeat(name.len() - legacy.len()));
    let mut old = pdf.clone();
    while let Some(at) = old.windows(name.len()).position(|w| w == name.as_bytes()) {
        old[at..at + name.len()].copy_from_slice(legacy.as_bytes());
    }
    assert!(!String::from_utf8_lossy(&old).contains(vectorcraft_pdf::EDITING_FILE));
    assert_eq!(open(&mut s, "old.pdf", &old)["restored"], true);
    assert_eq!(comparable(&s.doc().unwrap().doc), before);
}

#[test]
fn ai_files_are_pdfs_that_save_and_reopen_editable() {
    let mut s = rich();
    let path = tmp("art.ai");
    let r = s.execute("document.save", &json!({"path": path, "preserveEditing": false, "range": "1"})).unwrap();
    assert_eq!(r["path"], path.as_str());
    // As saved (Save stamps the File Info dates).
    let before = comparable(&s.doc().unwrap().doc);
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF"), ".ai is PDF-compatible");
    assert!(vectorcraft_pdf::editing(&bytes).is_some_and(|e| e.intact), "always carries the document");
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(path.as_str()), "the document takes the path");
    // Reopening restores it and keeps the path: Save writes .ai again.
    let r = s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!((&r["format"], &r["restored"]), (&json!("ai"), &json!(true)), "{r}");
    assert_eq!(comparable(&s.doc().unwrap().doc), before);
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(path.as_str()));
    s.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
    s.execute("document.save", &json!({})).unwrap();
    let again = vectorcraft_pdf::editing(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(vectorcraft_format::load(&again.data).unwrap().layers, s.doc().unwrap().doc.layers);
    // Save As also writes a plain PDF (the document then saves back as PDF); PNG stays an export.
    assert_eq!(s.execute("document.save", &json!({"path": tmp("x.pdf")})).unwrap()["format"], "pdf");
    assert!(s.execute("document.save", &json!({"path": tmp("x.png")})).is_err(), "PNG is written with document.export");
    // A plain PDF named .ai opens as artwork, without a path to save over.
    let plain = b64(&s.execute("document.exportPdf", &json!({"preserveEditing": false})).unwrap());
    let plain_path = tmp("plain.ai");
    std::fs::write(&plain_path, plain).unwrap();
    let r = s.execute("document.open", &json!({"path": plain_path})).unwrap();
    assert_eq!(r["restored"], false);
    assert_eq!(s.doc().unwrap().path, None);
}

/// #526: the exports (`document.export`, the CLI's `--export` and `convert`) write `.ai` as Save
/// As does, leaving the document's own path alone.
#[test]
fn exports_write_ai_as_save_as_does() {
    let mut s = rich();
    let before = comparable(&s.doc().unwrap().doc);
    let path = tmp("exported.ai");
    let r = s.execute("document.export", &json!({"path": path})).unwrap();
    assert_eq!(r["format"], "ai", "{r}");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF") && vectorcraft_pdf::editing(&bytes).is_some_and(|e| e.intact), "PDF-compatible, carrying the document");
    assert_eq!(s.doc().unwrap().path, None, "an export doesn't give the document a path");
    let r = s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!((&r["format"], &r["restored"]), (&json!("ai"), &json!(true)), "{r}");
    assert_eq!(comparable(&s.doc().unwrap().doc), before);
    // Its options as Save As takes them; no path gives the bytes.
    let r = s.execute("document.export", &json!({"format": "ai", "pdfCompatible": false})).unwrap();
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|w| w.contains("without PDF content"))), "{r}");
    assert!(b64(&r).starts_with(b"%PDF"));
}

/// A `.ai` with an artboard larger than 14,400 pt has its PDF page written with a `UserUnit`,
/// and reopens at its size from its editing data and from its PDF part.
#[test]
fn an_ai_with_an_artboard_past_14400_pt_reopens_at_its_size() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 20000, "height": 10000})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 1000, "y": 1000, "width": 5000, "height": 3000})).unwrap();
    let ai = b64(&s.execute("document.export", &json!({"format": "ai", "compress": false})).unwrap());
    assert!(String::from_utf8_lossy(&ai).contains("/UserUnit 1.389"), "20,000 / 14,400 rounded up to the next thousandth");
    let size = |s: &Session| {
        let r = s.doc().unwrap().doc.artboards[0].rect;
        (r.width().round(), r.height().round())
    };
    assert_eq!(open(&mut s, "big.ai", &ai)["restored"], true);
    assert_eq!(size(&s), (20_000.0, 10_000.0));
    // Picking its page opens the PDF part.
    let r = s.execute("document.open", &json!({"name": "big.ai", "dataBase64": vectorcraft_format::base64_encode(&ai), "pages": "1"})).unwrap();
    assert_eq!(r["restored"], false, "{r}");
    assert_eq!(size(&s), (20_000.0, 10_000.0));
}

#[test]
fn ait_templates_open_their_document_untitled() {
    let mut s = rich();
    let before = comparable(&s.doc().unwrap().doc);
    // A never-saved document's .ai comes back as bytes.
    let ai = b64(&s.execute("document.save", &json!({"format": "ai"})).unwrap());
    assert!(ai.starts_with(b"%PDF"));
    let r = open(&mut s, "brochure.ait", &ai);
    assert_eq!((&r["format"], &r["restored"]), (&json!("ait"), &json!(true)), "{r}");
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    assert_eq!(comparable(&s.doc().unwrap().doc), before);
}

/// Save as Template writes a PDF-compatible `.ait` for a `.ait` path or `format: ait`, and New
/// from Template opens it as an untitled copy of the document, layers and art included.
#[test]
fn save_as_template_writes_an_ait_that_new_from_template_opens_untitled() {
    let mut s = rich();
    s.execute("layer.new", &json!({"name": "Notes"})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 20, "y": 120, "width": 40, "height": 30})).unwrap();
    let before = comparable(&s.doc().unwrap().doc);
    assert_eq!(before.layers.len(), 2);
    let path = tmp("brochure template.ait");
    let r = s.execute("file.saveAsTemplate", &json!({"path": path})).unwrap();
    assert_eq!(r["format"], "ait", "{r}");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF") && vectorcraft_pdf::editing(&bytes).is_some_and(|e| e.intact), "PDF-compatible, carrying the document");
    assert_eq!((s.doc().unwrap().path.as_deref(), s.doc().unwrap().is_dirty()), (None, true), "the document is unchanged");
    let r = s.execute("file.newFromTemplate", &json!({"path": path})).unwrap();
    assert_eq!(r["format"], "ait", "{r}");
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    assert_eq!(comparable(&s.doc().unwrap().doc), before);
    // Without a path: the bytes and a name for the Templates folder.
    let r = s.execute("file.saveAsTemplate", &json!({"format": "ait"})).unwrap();
    assert_eq!(r["format"], "ait", "{r}");
    assert!(r["name"].as_str().is_some_and(|n| n.ends_with(" template.ait")), "{r}");
    assert!(vectorcraft_pdf::editing(&b64(&r)).is_some_and(|e| e.intact));
}

/// Save as Template writes a file named with its template format's extension: another format's
/// extension is replaced and a name without one gets it; Save and Save As name the command that
/// writes a `.ait`.
#[test]
fn save_as_template_names_its_file_after_the_template_format() {
    let mut s = rich();
    for (name, format, saved) in [
        ("card.png", None, "card.vctemplate"),
        ("card.vectorcraft", None, "card.vctemplate"),
        ("card.ait", Some("template"), "card.vctemplate"),
        ("card.pdf", Some("ait"), "card.ait"),
        ("card v1.2", None, "card v1.2.vctemplate"),
    ] {
        let (typed, want) = (tmp(&format!("named/{name}")), tmp(&format!("named/{saved}")));
        let _ = std::fs::create_dir_all(std::path::Path::new(&typed).parent().unwrap());
        let _ = std::fs::remove_file(&typed);
        let _ = std::fs::remove_file(&want);
        let r = s.execute("file.saveAsTemplate", &json!({"path": typed, "format": format})).unwrap();
        assert_eq!(r["path"], want.as_str(), "{name}: {r}");
        let bytes = std::fs::read(&want).unwrap_or_else(|e| panic!("{name}: {saved} not written: {e}"));
        let ait = saved.ends_with(".ait");
        assert_eq!(bytes.starts_with(b"%PDF"), ait, "{name}");
        assert!(ait || vectorcraft_format::load(&bytes).is_ok_and(|d| d.template), "{name}: a native template");
        assert!(!std::path::Path::new(&typed).exists(), "{name}: nothing written under the typed name");
    }
    for (cmd, p) in [("file.saveAs", json!({"format": "ait"})), ("document.save", json!({"path": tmp("card.ait")}))] {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains("file.saveAsTemplate") && !e.contains("document.export"), "{cmd}: {e}");
    }
}

/// Exports don't write a `.ait`: each export asked for one, by its path or `format`, answers an
/// error that names `file.saveAsTemplate`, which writes it, and writes nothing. A format that
/// nothing writes keeps its own error.
#[test]
fn exports_of_an_ait_name_save_as_template() {
    let mut s = rich();
    let path = tmp("exported.ait");
    let _ = std::fs::remove_file(&path);
    for (cmd, p) in [
        ("document.export", json!({"path": path})),
        ("document.export", json!({"format": "ait"})),
        ("document.serialize", json!({"format": "ait"})),
        ("document.exportSelection", json!({"path": path})),
        ("document.exportForScreens", json!({"formats": [{"format": "ait"}]})),
    ] {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains("PDF-compatible .ait template files are written by file.saveAsTemplate"), "{cmd} {p}: {e}");
    }
    assert!(!std::path::Path::new(&path).exists(), "nothing written");
    assert_eq!(writable_format(Some("ps"), None).unwrap_err(), "PostScript files can be opened but not written (see document.formats)");
}

/// Save as Template refuses `aiVersion` 7 for a `.ait`, given in `options` or flat, and writes no
/// file; `aiVersion` "pdf" writes the PDF-compatible template.
#[test]
fn save_as_template_refuses_a_legacy_ait() {
    let mut s = rich();
    let path = tmp("legacy-ait/card.ait");
    let dir = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for p in [
        json!({"path": path, "options": {"aiVersion": "7"}}),
        json!({"path": path, "format": "ait", "aiVersion": 7}),
        json!({"format": "ait", "options": {"aiVersion": "7"}}),
    ] {
        let e = s.execute("file.saveAsTemplate", &p).unwrap_err().to_string();
        assert!(e.contains("aiVersion 7 writes legacy .ai files"), "{p}: {e}");
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing written");
    let r = s.execute("file.saveAsTemplate", &json!({"path": path, "options": {"aiVersion": "pdf"}})).unwrap();
    assert_eq!(r["format"], "ait", "{r}");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF") && vectorcraft_pdf::editing(&bytes).is_some_and(|e| e.intact), "PDF-compatible, carrying the document");
}

#[test]
fn standards_and_screens_leave_the_editing_data_out() {
    let mut s = rich();
    // Choosing PDF/A turns Preserve Editing off unless it is asked for, which PDF/A refuses.
    let pdf_a = b64(&s.execute("document.exportPdf", &json!({"standard": "pdfA2b"})).unwrap());
    assert!(vectorcraft_pdf::editing(&pdf_a).is_none());
    let e = s.execute("document.exportPdf", &json!({"standard": "pdfA2b", "preserveEditing": true})).unwrap_err();
    assert!(matches!(e, crate::EngineError::BadParams { .. }), "{e}");
    // Export for Screens writes a PDF per artboard for viewing.
    let r = s.execute("document.exportForScreens", &json!({"formats": [{"format": "pdf"}]})).unwrap();
    let file = vectorcraft_format::base64_decode(r["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    assert!(vectorcraft_pdf::editing(&file).is_none());
}

#[test]
fn save_dialogs_offer_every_format_save_writes() {
    let labels = |name: &str| save_filters(name).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
    assert_eq!(labels("a.ai")[0], "PDF-compatible .ai");
    assert_eq!(labels("a.vectorcraft").len(), SAVE_FORMATS.len());
    assert_eq!(labels("a.svg")[0], "SVG");
    assert!(labels("a.png").is_empty() && labels("noext").is_empty(), "exports pick their own name");
    assert_eq!(save_format(Some("ai"), None).unwrap().id, "ai");
    assert_eq!(save_format(None, Some("x.AI")).unwrap().id, "ai");
    assert!(save_format(Some("png"), None).is_err());
}

#[test]
fn a_pdf_of_some_artboards_carries_no_editing_data() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "artboards": 3})).unwrap();
    for (range, kept) in [("2", false), ("1,3", false), ("3,2,1", false), ("1-3", true), ("all", true)] {
        let v = s.execute("document.exportPdf", &json!({ "range": range })).unwrap();
        assert_eq!(vectorcraft_pdf::editing(&b64(&v)).is_some(), kept, "{range}");
        let warned = v["warnings"].as_array().unwrap().iter().any(|w| w == super::pdf::EDITING_NEEDS_EVERY_ARTBOARD);
        assert_eq!(warned, !kept, "{range}: {v}");
    }
}

#[test]
fn ai_export_of_some_artboards_keeps_only_those_pages() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "artboards": 3})).unwrap();
    let r = s.execute("document.export", &json!({"format": "ai", "range": "2"})).unwrap();
    let bytes = b64(&r);
    assert!(vectorcraft_pdf::editing(&bytes).is_none());
    assert!(r["warnings"].as_array().unwrap().contains(&json!(super::pdf::EDITING_NEEDS_EVERY_ARTBOARD)));
    open(&mut s, "one.ai", &bytes);
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 1);
    let mut s = Session::new();
    s.execute("file.new", &json!({"artboards": 3})).unwrap();
    assert!(s.execute("document.export", &json!({"format": "ai", "range": "2", "pdfCompatible": false})).is_err());
}

#[test]
fn ai_export_validates_options_even_when_editing_data_is_omitted() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"artboards": 3})).unwrap();
    let before = comparable(&s.doc().unwrap().doc);
    for command in ["document.export", "document.serialize"] {
        for range in ["all", "2"] {
            for option in ["includeLinked", "embedProfiles", "compress", "pdfCompatible"] {
                let mut params = json!({"format": "ai", "range": range});
                params[option] = json!("no");
                assert!(s.execute(command, &params).is_err(), "{command}: {params}");
                assert_eq!(comparable(&s.doc().unwrap().doc), before);
            }
            assert!(
                s.execute(
                    command,
                    &json!({"format": "ai", "range": range, "includeLinked": false, "embedProfiles": false, "compress": false, "pdfCompatible": true})
                )
                .is_ok()
            );
        }
    }
}

#[test]
fn ai_export_without_artboards_keeps_art_outside_the_original_pages() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "artboards": 2})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 180, "y": 100, "width": 30, "height": 20})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 1000, "y": 1000, "width": 50, "height": 40})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    let before = comparable(&s.doc().unwrap().doc);
    let r = s.execute("document.export", &json!({"format": "ai", "useArtboards": false, "range": "2"})).unwrap();
    let bytes = b64(&r);
    let pages = vectorcraft_pdf::import(&bytes).unwrap();
    assert_eq!(pages.artboards.len(), 1);
    assert_eq!((pages.artboards[0].rect.width(), pages.artboards[0].rect.height()), (30.0, 20.0));
    let editing = vectorcraft_pdf::editing(&bytes).unwrap();
    let exported = vectorcraft_format::load(&editing.data).unwrap();
    assert_eq!(exported.artboards.len(), 1);
    assert_eq!(exported.artboards[0].rect, vectorcraft_geom::Rect::new(180.0, 100.0, 210.0, 120.0));
    assert_eq!(comparable(&s.doc().unwrap().doc), before);
    assert!(s.execute("document.export", &json!({"format": "ai", "useArtboards": "yes"})).is_err());
}
