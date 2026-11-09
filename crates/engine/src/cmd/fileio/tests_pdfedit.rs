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

/// Color Halftone's Max Radius is in pixels of the raster effects resolution since document
/// version 3 (#3260). A PDF saved with Preserve Editing carries the native document, so one from
/// before that opens with the radius converted, drawing the same dots; one saved since opens as
/// it was.
#[test]
fn a_preserve_editing_pdf_opens_with_its_halftone_radius_in_pixels() {
    let halftone = |old: bool| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        s.execute("document.rasterEffectsSettings", &json!({"resolution": 300})).unwrap();
        let r = super::tests_svgedit::rect(&mut s, 10.0, 10.0);
        s.execute("effect.apply", &json!({"effect": "pixelate.colorHalftone", "params": {"maxRadius": 8}, "ids": [r]})).unwrap();
        if old {
            // As VectorCraft 0.18 wrote it: the document model's version 1, the radius in points.
            std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).version = 1;
        }
        let pdf = b64(&s.execute("document.exportPdf", &json!({})).unwrap());
        let opened = open(&mut s, "halftone.pdf", &pdf);
        assert_eq!(opened["restored"], json!(true), "{opened}");
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.version, vectorcraft_doc::FORMAT_VERSION);
        let n = d.node(vectorcraft_doc::NodeId(r)).unwrap();
        n.appearance.effects.iter().find(|e| e.id == "pixelate.colorHalftone").and_then(|e| e.params["maxRadius"].as_f64()).unwrap()
    };
    // 8 pt at 300 ppi are 33⅓ px.
    assert!((halftone(true) - 8.0 * 300.0 / 72.0).abs() < 1e-9, "{}", halftone(true));
    assert_eq!(halftone(false), 8.0);
}

/// The layers of `doc` as their names and the kinds and bounds of what they hold, rounded.
fn outline(doc: &vectorcraft_doc::Document) -> Vec<String> {
    fn walk(nodes: &[std::sync::Arc<vectorcraft_doc::Node>], depth: usize, out: &mut Vec<String>) {
        for n in nodes {
            let b = n.visual_bounds().map(|b| [b.x0, b.y0, b.x1, b.y1].map(|v| v.round() as i64));
            let kind = match &n.kind {
                vectorcraft_doc::NodeKind::Layer { .. } => "layer",
                vectorcraft_doc::NodeKind::Group { .. } => "group",
                vectorcraft_doc::NodeKind::Path { .. } => "path",
                _ => "other",
            };
            out.push(format!("{depth} {kind} {:?} {} {b:?}", n.name, n.visible));
            walk(n.children().map(Vec::as_slice).unwrap_or_default(), depth + 1, out);
        }
    }
    let mut out = vec![];
    walk(&doc.layers, 0, &mut out);
    out
}

/// A `.ai` also carries the editing copy of its art that other apps read: opened without the
/// VectorCraft document it carries, it gives back the same layers, groups and paths, its page
/// matching them. `editingData: false` leaves the copy out.
#[test]
fn ai_files_carry_an_editing_copy_for_other_apps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 60, "y": 10, "width": 50, "height": 30})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("layer.new", &json!({"name": "Top"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 120, "y": 50, "width": 60, "height": 40})).unwrap();
    let before = outline(&s.doc().unwrap().doc);
    let bytes = b64(&s.execute("document.export", &json!({"format": "ai"})).unwrap());
    let private = vectorcraft_pdf::illustrator_data(&bytes, None).expect("the editing copy for other apps");
    assert!(private.starts_with(b"%AI24_ZStandard_Data"));
    let opts = LoadOptions { native: false, ..Default::default() };
    let l = load::load_with("art.ai", &bytes, &opts).unwrap();
    assert!(l.editing_data && !l.restored);
    assert!(!l.warnings.iter().any(|w| w.contains("editing data")), "{:?}", l.warnings);
    assert_eq!(outline(&l.doc), before);
    let r = s.execute("document.export", &json!({"format": "ai", "editingData": false})).unwrap();
    assert_eq!(vectorcraft_pdf::illustrator_data(&b64(&r), None), None);
    assert!(s.execute("document.export", &json!({"format": "ai", "editingData": "no"})).is_err());
}

/// A `.ai` whose editing copy for other apps differs from the VectorCraft document it carries (the
/// copy leaves the live effect out) opens in VectorCraft from the VectorCraft document.
#[test]
fn an_ai_file_opens_from_its_vectorcraft_document_when_the_editing_copy_differs() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("effect.apply", &json!({"effect": "blur.gaussian", "ids": [id]})).unwrap();
    let r = s.execute("document.export", &json!({"format": "ai"})).unwrap();
    assert!(r["warnings"].to_string().contains("live effects"), "{r}");
    let bytes = b64(&r);
    let effects = |doc: &vectorcraft_doc::Document| {
        let mut out = vec![];
        doc.walk(|n| {
            if matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. }) {
                out.push(n.appearance.effects.len());
            }
        });
        out
    };
    let copy = load::load_with("art.ai", &bytes, &LoadOptions { native: false, trust_editing_data: true, ..Default::default() }).unwrap();
    assert!(copy.editing_data && !copy.restored);
    assert_eq!(effects(&copy.doc), [0]);
    let opened = load::load_with("art.ai", &bytes, &LoadOptions::default()).unwrap();
    assert!(opened.restored);
    assert_eq!(effects(&opened.doc), [1]);
}

/// The layers, groups, compound paths and paths of `doc` with their names, visibility, geometric
/// bounds (rounded), transparency, clipping and opacity masks, one line each; a mask's art after
/// its object.
fn see_through_outline(doc: &vectorcraft_doc::Document) -> Vec<String> {
    use vectorcraft_doc::NodeKind;
    fn walk(nodes: &[std::sync::Arc<vectorcraft_doc::Node>], depth: usize, out: &mut Vec<String>) {
        for n in nodes {
            let kind = match &n.kind {
                NodeKind::Layer { clip, .. } => format!("layer clip={clip}"),
                NodeKind::Group { clip, .. } => format!("group clip={clip}"),
                NodeKind::Compound { .. } => "compound".into(),
                NodeKind::Path { clipping, .. } => format!("path clipping={clipping}"),
                _ => "other".into(),
            };
            let b = n.geometric_bounds().map(|b| [b.x0, b.y0, b.x1, b.y1].map(|v| v.round() as i64));
            let mask = n.extras().mask.clone();
            out.push(format!(
                "{depth} {kind} {:?} {} {b:?} {} {:?} isolate={} {:?} shape={} mask={:?}",
                n.name,
                n.visible,
                (n.opacity * 100.0).round(),
                n.blend,
                n.isolate,
                n.knockout,
                n.knockout_shape,
                mask.as_ref().map(|m| (m.clip, m.invert, m.disabled, m.linked))
            ));
            if let Some(m) = mask {
                walk(std::slice::from_ref(&m.art), depth + 1, out);
            }
            walk(n.children().map(Vec::as_slice).unwrap_or_default(), depth + 1, out);
        }
    }
    let mut out = vec![];
    walk(&doc.layers, 0, &mut out);
    out
}

/// A document of two artboards (the art on the first) with a compound path at 70% in Screen, a
/// clipping group at 50% in Multiply, a named group (isolated, knockout on, knockout shape) and a
/// square with an opacity mask that doesn't clip on layer `Layer 1`, and layer `Cut` (40%,
/// clipping) holding two squares.
fn see_through_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let rect = |s: &mut Session, x: f64, y: f64, w: f64, h: f64| {
        id(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
    };
    // A compound path at 70% in Screen.
    let (a, b) = (rect(&mut s, 10.0, 10.0, 40.0, 40.0), rect(&mut s, 20.0, 20.0, 20.0, 20.0));
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 70, "blend": "Screen"})).unwrap();
    // A clipping group at 50% in Multiply.
    let (art, clip) = (rect(&mut s, 60.0, 10.0, 60.0, 40.0), rect(&mut s, 70.0, 20.0, 30.0, 20.0));
    s.execute("select.set", &json!({"ids": [art, clip]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50, "blend": "Multiply"})).unwrap();
    // A named group, isolated, knockout on, whose opacity defines the knockout shape.
    let (a, b) = (rect(&mut s, 60.0, 60.0, 30.0, 30.0), rect(&mut s, 75.0, 75.0, 30.0, 30.0));
    let g = id(s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
    s.execute("object.setProps", &json!({"ids": [g], "name": "Knock", "opacity": 80, "isolate": true, "knockout": "on", "knockoutShape": true}))
        .unwrap();
    // A square with an opacity mask that doesn't clip.
    let masked = rect(&mut s, 130.0, 10.0, 40.0, 40.0);
    s.execute("paint.setFill", &json!({"color": "#808080"})).unwrap();
    let mask = rect(&mut s, 140.0, 20.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [masked, mask]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({"clip": false})).unwrap();
    // A clipping layer at 40%.
    let cut = id(s.execute("layer.new", &json!({"name": "Cut"})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    rect(&mut s, 10.0, 100.0, 80.0, 60.0);
    rect(&mut s, 30.0, 110.0, 40.0, 40.0);
    s.execute("layer.clippingMask.toggle", &json!({})).unwrap();
    s.execute("object.setProps", &json!({"ids": [cut], "opacity": 40})).unwrap();
    s
}

/// The editing copy of `bytes` read alone, checked against its page.
fn editing_copy(bytes: &[u8]) -> vectorcraft_doc::Document {
    let l = load::load_with("art.ai", bytes, &LoadOptions { native: false, ..Default::default() }).unwrap();
    assert!(l.editing_data && !l.restored, "{:?}", l.warnings);
    assert!(!l.warnings.iter().any(|w| w.contains("editing data")), "{:?}", l.warnings);
    l.doc
}

/// Transparency, a compound path, a clipping group, an opacity mask and a clipping layer come back
/// from the editing copy for other apps alone, its page matching it, with every save option that
/// writes the copy; the version 7 `.ai` keeps the compound path and the clipping group.
#[test]
fn transparency_clipping_and_opacity_masks_come_back_from_the_editing_copy() {
    let mut s = see_through_session();
    let before = (*s.doc().unwrap().doc).clone();
    // The mask's art is one path, and it reads back as that path.
    let mut arts = vec![];
    before.walk(|n| arts.extend(n.extras().mask.as_ref().map(|m| matches!(m.art.kind, vectorcraft_doc::NodeKind::Path { .. }))));
    assert_eq!(arts, [true]);
    let want = see_through_outline(&before);
    for opts in [json!({"format": "ai"}), json!({"format": "ai", "compress": false}), json!({"format": "ai", "pdfCompatible": true})] {
        let bytes = b64(&s.execute("document.export", &opts).unwrap());
        assert_eq!(see_through_outline(&editing_copy(&bytes)), want, "{opts}");
    }
    // Each artboard in a file of its own: the first holds all the art.
    let path = tmp("see-through.ai");
    let r = s.execute("document.save", &json!({"path": path, "separateArtboards": true})).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    assert_eq!(files.len(), 3, "{r}");
    for f in &files {
        let copy = editing_copy(&std::fs::read(f).unwrap());
        if f.ends_with("-2.ai") {
            assert!(copy.layers.iter().all(|l| l.children().is_none_or(Vec::is_empty)), "{f} {:#?}", see_through_outline(&copy));
        } else {
            assert_eq!(see_through_outline(&copy), want, "{f}");
        }
    }
    // Version 7 has no transparency; it keeps the compound path and the clipping group.
    let r = s.execute("document.export", &json!({"format": "ai", "aiVersion": 7})).unwrap();
    let legacy = load::load_with("art.ai", &b64(&r), &LoadOptions::default()).unwrap();
    let kinds = see_through_outline(&legacy.doc);
    assert!(kinds.iter().any(|l| l.contains(" compound ")) && kinds.iter().any(|l| l.contains("group clip=true")), "{kinds:#?}");
}

/// The paths and compound paths of `doc`'s art and of its patterns' tiles with their geometric
/// bounds (rounded) and fills and strokes, one line each, then its gradient and pattern swatches: a
/// gradient by its kind, its swatch link (`-` when it names no gradient swatch of `doc`), its stops
/// (place, color, opacity, midpoint, swatch link and tint) and the gradient's place at the corners and
/// center of the paint's bounds (rounded to hundredths); a pattern by its name and placement.
fn paint_outline(doc: &vectorcraft_doc::Document) -> Vec<String> {
    use std::sync::Arc;
    use vectorcraft_color::Paint;
    use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
    let link = |name: &Option<String>| match name.as_deref().and_then(|n| doc.swatch(n)) {
        Some(sw) if matches!(sw.paint, Paint::Gradient(_)) => sw.name.clone(),
        _ => "-".into(),
    };
    let drawn = |p: &Paint, b: vectorcraft_geom::Rect| match p {
        Paint::Gradient(g) => {
            let geom = g.resolve(b);
            let at: Vec<f64> = [(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1), (b.center().x, b.center().y)]
                .map(|(x, y)| (geom.param_at(g.gradient.kind, (x, y).into()) * 100.0).round() / 100.0)
                .to_vec();
            let stops: Vec<String> = g
                .gradient
                .stops
                .iter()
                .map(|s| {
                    let rgb = s.color.to_rgb().map(|v| (v * 100.0).round());
                    format!("{:.3} {rgb:?} {:.2} mid {:.2} {:?} {:.2}", s.offset, s.opacity, s.midpoint, s.swatch, s.tint)
                })
                .collect();
            format!("{:?} link {} {stops:?} {at:?}", g.gradient.kind, link(&g.swatch))
        }
        Paint::Pattern { pattern, xf } => format!("pattern {pattern} {:?}", xf.as_coeffs().map(|v| (v * 1000.0).round() / 1000.0)),
        Paint::Solid { color, .. } => format!("{:?}", color.to_rgb().map(|v| (v * 100.0).round())),
        Paint::None => "none".into(),
    };
    // A compound path's paint is its own: its members' paint isn't drawn (Release gives them the
    // compound path's), so a compound path's line lists only its members' bounds.
    fn walk(nodes: &[Arc<Node>], f: &mut dyn FnMut(&Node)) {
        for n in nodes {
            f(n);
            if !matches!(n.kind, NodeKind::Compound { .. }) {
                walk(n.children().map(Vec::as_slice).unwrap_or_default(), f);
            }
        }
    }
    let bounds = |n: &Node| n.geometric_bounds().map(|b| [b.x0, b.y0, b.x1, b.y1].map(|v| v.round() as i64));
    let lines = |nodes: &[Arc<Node>]| {
        let mut lines = vec![];
        walk(nodes, &mut |n| {
            if !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
                return;
            }
            let Some(b) = n.geometric_bounds() else { return };
            let kind = match n.children() {
                Some(members) => format!("compound of {:?}", members.iter().map(|m| bounds(m)).collect::<Vec<_>>()),
                None => "path".into(),
            };
            let mut line = format!("{kind} {:?}", [b.x0, b.y0, b.x1, b.y1].map(|v| v.round() as i64));
            for it in n.appearance.items.iter().filter(|i| !i.paint().is_none()) {
                line += &match it {
                    AppearanceItem::Fill(f) => format!(" fill {}", drawn(&f.paint, b)),
                    AppearanceItem::Stroke(st) => format!(" stroke {} {}", st.width, drawn(&st.paint, st.paint_bounds(b))),
                };
            }
            lines.push(line);
        });
        lines
    };
    let mut out = lines(&doc.layers);
    for p in doc.patterns.iter() {
        out.push(format!("pattern {} tile {:?}", p.name, [p.tile.x0, p.tile.y0, p.tile.x1, p.tile.y1]));
        out.extend(lines(&p.art));
    }
    for sw in doc.swatches_iter() {
        match &sw.paint {
            // A gradient swatch reads back linked to itself.
            Paint::Gradient(g) => {
                let own = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(g.gradient.clone())));
                out.push(format!("swatch {} {}", sw.name, drawn(&own, vectorcraft_geom::Rect::ZERO)));
            }
            Paint::Pattern { pattern, .. } => out.push(format!("swatch {} pattern {pattern}", sw.name)),
            _ => {}
        }
    }
    out
}

/// The images of `doc`'s art, one line each: its pixel size, where its corners land (rounded to
/// hundredths), its link's path and relative path, and its pixels (RGBA).
fn image_outline(doc: &vectorcraft_doc::Document) -> Vec<String> {
    use vectorcraft_doc::NodeKind;
    let mut out = vec![];
    doc.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            let r = vectorcraft_geom::Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height));
            let corners = [(r.x0, r.y0), (r.x1, r.y0), (r.x0, r.y1)].map(|(x, y)| {
                let p = im.xf * vectorcraft_geom::Point::new(x, y);
                ((p.x * 100.0).round() / 100.0, (p.y * 100.0).round() / 100.0)
            });
            let link = im.link.as_ref().map(|l| (l.path.clone(), l.relative.clone()));
            let pixels = doc.image_blob(im).and_then(|b| image::load_from_memory(&b.bytes).ok()).map(|i| i.to_rgba8().into_raw());
            out.push(format!("{}x{} {corners:?} {link:?} {pixels:?}", im.width, im.height));
        }
    });
    out
}

/// An embedded image (RGBA, part of it see-through, turned) and a linked one come back from the
/// editing copy for other apps alone, pixel for pixel, with every save option that writes the copy;
/// the linked image keeps its path and its path relative to the document's folder.
#[test]
fn images_come_back_from_the_editing_copy() {
    let dir = tmp("ai-images");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(format!("{dir}/art")).unwrap();
    let png = |w: u32, h: u32, f: &dyn Fn(u32, u32) -> [u8; 4]| {
        let mut out = vec![];
        image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y))).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let linked = format!("{dir}/art/linked.png");
    std::fs::write(&linked, png(5, 4, &|x, y| [(x * 50) as u8, (y * 60) as u8, 90, 255])).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let data = vectorcraft_format::base64_encode(&png(6, 3, &|x, y| [200, (x * 40) as u8, (y * 80) as u8, if x < 2 { 100 } else { 255 }]));
    s.execute("file.place", &json!({"name": "embedded.png", "dataBase64": data})).unwrap();
    // The placed image is selected.
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    s.execute("file.place", &json!({"path": linked, "link": true})).unwrap();
    let path = format!("{dir}/images.ai");
    s.execute("document.save", &json!({"path": path})).unwrap();
    let before = image_outline(&s.doc().unwrap().doc);
    assert_eq!(before.len(), 2, "{before:#?}");
    assert!(before[1].contains("art/linked.png"), "{before:#?}");
    for opts in [
        json!({"format": "ai"}),
        json!({"format": "ai", "compress": false}),
        json!({"format": "ai", "embedProfiles": false}),
        json!({"format": "ai", "includeLinked": true}),
        json!({"format": "ai", "range": "all"}),
        json!({"format": "ai", "pdfCompatible": false}),
    ] {
        let r = s.execute("document.export", &opts).unwrap();
        assert!(!r["warnings"].to_string().contains("image"), "{r}");
        let copy = load::load_with(&path, &b64(&r), &LoadOptions { native: false, ..Default::default() }).unwrap();
        assert!(copy.editing_data && !copy.restored, "{opts} {:?}", copy.warnings);
        assert!(!copy.warnings.iter().any(|w| w.contains("editing data")), "{opts} {:?}", copy.warnings);
        assert_eq!(image_outline(&copy.doc), before, "{opts}");
    }
}

/// The type of `doc`'s art, one line each: its kind, point type's matrix or area type's frame on the
/// document (rounded to hundredths), its characters, and each run's font, size and fill.
fn type_outline(doc: &vectorcraft_doc::Document) -> Vec<String> {
    use vectorcraft_doc::{NodeKind, TextKind};
    let mut out = vec![];
    let round = |v: f64| (v * 100.0).round() / 100.0 + 0.0;
    doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            let (kind, xf) = match &t.kind {
                TextKind::Point => ("point", t.xf.as_coeffs().map(round).to_vec()),
                TextKind::Area { frame } => {
                    let b = frame.transformed(t.xf).bounds().unwrap();
                    ("area", vec![round(b.x0), round(b.y0), round(b.x1), round(b.y1)])
                }
                TextKind::OnPath { .. } => ("path", vec![]),
            };
            let runs: Vec<String> = t
                .runs
                .iter()
                .map(|r| format!("{:?} {} {} {} {:?}", r.text, r.style.font_family, r.style.font_style, round(r.style.size), r.style.fill))
                .collect();
            out.push(format!("{kind} {xf:?} {runs:?}"));
        }
    });
    out
}

/// Point type of two paragraphs, turned point type and area type come back from the editing copy
/// for other apps alone as type, with every save option that writes the copy.
#[test]
fn type_comes_back_from_the_editing_copy() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("text.create", &json!({"x": 20, "y": 40, "text": "Hello\nWorld", "size": 24})).unwrap();
    s.execute("text.create", &json!({"x": 150, "y": 100, "text": "Turned"})).unwrap();
    // The new type is selected.
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    s.execute("text.create", &json!({"x": 20, "y": 120, "text": "Area type in a frame", "area": {"width": 120, "height": 60}})).unwrap();
    let path = tmp("type.ai");
    s.execute("document.save", &json!({"path": path})).unwrap();
    let before = type_outline(&s.doc().unwrap().doc);
    assert_eq!(before.len(), 3, "{before:#?}");
    for opts in [
        json!({"format": "ai"}),
        json!({"format": "ai", "compress": false}),
        json!({"format": "ai", "embedProfiles": false}),
        json!({"format": "ai", "includeLinked": true}),
        json!({"format": "ai", "range": "all"}),
        json!({"format": "ai", "pdfCompatible": false}),
    ] {
        let r = s.execute("document.export", &opts).unwrap();
        assert!(!r["warnings"].to_string().contains("type"), "{opts} {}", r["warnings"]);
        let copy = load::load_with(&path, &b64(&r), &LoadOptions { native: false, ..Default::default() }).unwrap();
        assert!(copy.editing_data && !copy.restored, "{opts} {:?}", copy.warnings);
        assert!(!copy.warnings.iter().any(|w| w.contains("editing data")), "{opts} {:?}", copy.warnings);
        assert_eq!(type_outline(&copy.doc), before, "{opts}");
    }
}

/// A document of two artboards (the art on the first): a rectangle filled with a linear gradient of
/// three stops placed across part of it, an ellipse filled with an elliptical radial gradient whose
/// focal point is off its center, a rectangle stroked with a gradient, rectangles filled and
/// stroked with the pattern `Dots`, one of them turned, an even-odd compound path filled with a
/// gradient of a gray, a CMYK color and the spot color `Ink` at 70%, and the gradient swatch `Dawn`.
fn gradient_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let rect = |s: &mut Session, x: f64, y: f64, w: f64, h: f64| {
        id(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
    };
    let three = json!({"kind": "linear", "stops": [
        {"offset": 0, "color": "#ff0000", "midpoint": 0.3},
        {"offset": 0.4, "color": "#00ff00", "opacity": 0.5},
        {"offset": 1, "color": "#0000ff"}
    ], "start": [20, 40], "end": [60, 17]});
    let a = rect(&mut s, 10.0, 10.0, 80.0, 40.0);
    s.execute("paint.setFill", &json!({"ids": [a], "gradient": three})).unwrap();
    let e = id(s.execute("shape.ellipse", &json!({"x": 100, "y": 10, "width": 80, "height": 40})).unwrap());
    let glow = json!({"kind": "radial", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#1a33cc"}],
        "start": [140, 30], "end": [180, 30], "aspect": 50, "focal": [150, 25]});
    s.execute("paint.setFill", &json!({"ids": [e], "gradient": glow})).unwrap();
    let c = rect(&mut s, 190.0, 10.0, 90.0, 40.0);
    let two = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 1, "color": "#000000"}]});
    s.execute("paint.setStroke", &json!({"ids": [c], "gradient": two, "width": 6})).unwrap();
    let tile = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [tile], "color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [tile], "none": true})).unwrap();
    s.execute("object.pattern.make", &json!({"ids": [tile], "name": "Dots", "edit": false})).unwrap();
    let p = rect(&mut s, 10.0, 70.0, 80.0, 40.0);
    s.execute("paint.setFill", &json!({"ids": [p], "swatch": "Dots"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [p], "swatch": "Dots", "width": 4})).unwrap();
    let turned = rect(&mut s, 100.0, 70.0, 80.0, 40.0);
    s.execute("paint.setFill", &json!({"ids": [turned], "swatch": "Dots"})).unwrap();
    // Turned 30° and scaled to half about (140, 90).
    let (c, k) = (30f64.to_radians().cos() * 0.5, 30f64.to_radians().sin() * 0.5);
    let m = [c, k, -k, c, 140.0 - 140.0 * c + 90.0 * k, 90.0 - 140.0 * k - 90.0 * c];
    s.execute("pattern.transform", &json!({"ids": [turned], "matrix": m})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#ff9900", "spot": true})).unwrap();
    let outer = rect(&mut s, 190.0, 70.0, 90.0, 40.0);
    let hole = rect(&mut s, 210.0, 80.0, 40.0, 20.0);
    let ring = id(s.execute("object.compoundPath.make", &json!({"ids": [outer, hole]})).unwrap());
    let mixed = json!({"kind": "linear", "stops": [
        {"offset": 0, "color": {"gray": 0.8}},
        {"offset": 0.5, "color": {"c": 1, "m": 0, "y": 0, "k": 0}, "midpoint": 0.7},
        {"offset": 1, "swatch": "Ink", "tint": 70}
    ]});
    s.execute("paint.setFill", &json!({"ids": [ring], "gradient": mixed})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [ring], "none": true})).unwrap();
    let dawn = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ffcc00"}, {"offset": 1, "color": "#cc3300"}]});
    s.execute("swatch.new", &json!({"name": "Dawn", "gradient": dawn})).unwrap();
    s
}

/// Gradients and patterns come back from the editing copy for other apps alone (the native
/// document left out), with every save option that writes the copy; where the file has PDF pages,
/// the reader takes the copy only when it draws as its page does. The version 7 `.ai` keeps each
/// gradient.
#[test]
fn gradients_and_patterns_come_back_from_the_editing_copy() {
    let mut s = gradient_session();
    let before = (*s.doc().unwrap().doc).clone();
    let want = paint_outline(&before);
    assert!(want.iter().filter(|l| l.starts_with("path") && l.contains("pattern Dots")).count() == 2, "{want:#?}");
    assert!(want.iter().any(|l| l.starts_with("compound") && l.contains("Ink") && l.contains(") 0.70")), "{want:#?}");
    assert!(want.iter().any(|l| l.contains("mid 0.30")) && want.iter().any(|l| l.starts_with("swatch Dawn")), "{want:#?}");
    for opts in [
        json!({"format": "ai"}),
        json!({"format": "ai", "compress": false}),
        json!({"format": "ai", "embedProfiles": false}),
        json!({"format": "ai", "includeLinked": true}),
        json!({"format": "ai", "range": "all"}),
        json!({"format": "ai", "pdfCompatible": false}),
    ] {
        let r = s.execute("document.export", &opts).unwrap();
        assert!(!r["warnings"].to_string().contains("gradient") && !r["warnings"].to_string().contains("pattern"), "{r}");
        let copy = editing_copy(&b64(&r));
        assert_eq!(paint_outline(&copy), want, "{opts}");
    }
    let path = tmp("gradients.ai");
    let r = s.execute("document.save", &json!({"path": path, "separateArtboards": true})).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    assert_eq!(files.len(), 3, "{r}");
    for f in files.iter().filter(|f| !f.ends_with("-2.ai")) {
        assert_eq!(paint_outline(&editing_copy(&std::fs::read(f).unwrap())), want, "{f}");
    }
    let r = s.execute("document.export", &json!({"format": "ai", "aiVersion": 7})).unwrap();
    let legacy = load::load_with("art.ai", &b64(&r), &LoadOptions::default()).unwrap();
    // Each gradient the art paints with, by its kind and its stops' places, and its stops' colors
    // within 5% (version 7 writes a gradient stroke as its filled outline, so the objects differ),
    // except a spot color's tint, which version 7 writes through its CMYK equivalent: 70% of that
    // equivalent displays about 7% off the tint of the spot color.
    let gradients = |doc: &vectorcraft_doc::Document| {
        let mut all = std::collections::BTreeMap::new();
        doc.walk(|node| {
            for it in node.appearance.items.iter() {
                if let vectorcraft_color::Paint::Gradient(g) = it.paint() {
                    let places: Vec<String> = g.gradient.stops.iter().map(|s| format!("{:.3}", s.offset)).collect();
                    let colors: Vec<([f32; 3], bool)> =
                        g.gradient.stops.iter().map(|s| (s.color.to_rgb(), s.swatch.is_some() && s.tint < 1.0)).collect();
                    all.insert(format!("{:?} {places:?}", g.gradient.kind), colors);
                }
            }
        });
        all
    };
    let (got, want) = (gradients(&legacy.doc), gradients(&before));
    assert_eq!(got.keys().collect::<Vec<_>>(), want.keys().collect::<Vec<_>>());
    for (key, stops) in &want {
        for (i, (rgb, spot)) in stops.iter().enumerate() {
            let back = got[key][i].0;
            let most = if *spot { 0.1 } else { 0.05 };
            assert!((0..3).all(|c| (back[c] - rgb[c]).abs() < most), "{key} stop {i}: {back:?} for {rgb:?}");
        }
    }
}
