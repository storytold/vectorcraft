//! Export As: `useArtboards` writes one file per artboard (all or a range) or, off, the bounds of
//! the visible art.

use serde_json::{Value, json};

use super::*;

fn session(width: f64, height: f64, artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": width, "height": height, "artboards": artboards})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn rgba(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes).unwrap().to_rgba8()
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-exportas-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn use_artboards_writes_one_file_per_artboard_in_the_range() {
    let mut s = session(60.0, 40.0, 5);
    let dir = tmp_dir("range");
    let path = dir.join("Logo.png").to_string_lossy().to_string();
    let r = s.execute("document.export", &json!({"path": path, "useArtboards": true, "range": "1-3,5"})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    let names: Vec<&str> = files.iter().map(|f| f.rsplit(['/', '\\']).next().unwrap()).collect();
    assert_eq!(names, ["Logo-Artboard-1.png", "Logo-Artboard-2.png", "Logo-Artboard-3.png", "Logo-Artboard-5.png"], "{r}");
    assert_eq!(r["path"], files[0]);
    for f in files {
        assert_eq!(rgba(&std::fs::read(f).unwrap()).dimensions(), (60, 40));
    }
    // Without a range: every artboard; without a path the files come back named after the document.
    let r = s.execute("document.export", &json!({"format": "jpg", "useArtboards": true})).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 5);
    assert_eq!(files[0]["name"], "Untitled-1-Artboard-1.jpg");
    assert_eq!(&b64(&files[4])[..2], [0xFF, 0xD8]);
    // One artboard is one file, written where asked; a PDF holds the range as pages of one file.
    let r = s.execute("document.export", &json!({"format": "webp", "useArtboards": true, "artboards": [3]})).unwrap();
    assert!(r.get("files").is_none() && !b64(&r).is_empty(), "{r}");
    let r = s.execute("document.export", &json!({"format": "pdf", "useArtboards": true, "range": "2-4"})).unwrap();
    assert_eq!(vectorcraft_pdf::import(&b64(&r)).unwrap().artboards.len(), 3);
    // Without Use Artboards a raster export holds one artboard.
    assert!(s.execute("document.export", &json!({"format": "png", "range": "1-2"})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn per_artboard_names_keep_the_folder_and_extension() {
    let mut s = session(20.0, 20.0, 3);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Hero / Wide"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "hero---wide"})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let enc = encode_all(&doc, "jpg", &json!({"useArtboards": true})).unwrap();
    let names: Vec<String> = enc.named(&doc, "C:\\out\\shot.jpeg").into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["C:\\out\\shot-Hero---Wide.jpeg", "C:\\out\\shot-hero---wide-2.jpeg", "C:\\out\\shot-Artboard-3.jpeg"]);
}

#[test]
fn bad_artboard_choices_error() {
    let mut s = session(20.0, 20.0, 3);
    for p in [
        json!({"useArtboards": true, "range": ""}),
        json!({"useArtboards": true, "range": "4"}),
        json!({"useArtboards": true, "artboards": []}),
        json!({"useArtboards": "yes"}),
        json!({"format": "pdf", "useArtboards": "yes"}),
    ] {
        assert!(s.execute("document.export", &p).is_err(), "{p}");
    }
}

#[test]
fn without_artboards_the_export_covers_the_visible_art() {
    let mut s = session(200.0, 100.0, 1);
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 10, "width": 30, "height": 20})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 90, "y": 40, "width": 40, "height": 35})).unwrap();
    let hidden = s.execute("shape.rectangle", &json!({"x": 150, "y": 0, "width": 40, "height": 99})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [hidden]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    // The union of the two visible rectangles: 20..130 × 10..75.
    let png = b64(&s.execute("document.export", &json!({"format": "png", "useArtboards": false})).unwrap());
    let img = rgba(&png);
    assert_eq!(img.dimensions(), (110, 65));
    assert_eq!(img.get_pixel(0, 0)[3], 255, "the art starts at the corner");
    assert_eq!(
        rgba(&b64(&s.execute("document.export", &json!({"format": "png", "useArtboards": false, "ppi": 144})).unwrap())).dimensions(),
        (220, 130)
    );
    // A PDF takes the art's bounds too (SVG has its own `useArtboards` option).
    let pdf = vectorcraft_pdf::import(&b64(&s.execute("document.export", &json!({"format": "pdf", "useArtboards": false})).unwrap())).unwrap();
    let page = pdf.artboards[0].rect;
    assert_eq!((page.width().round(), page.height().round()), (110.0, 65.0));
    let mut empty = session(50.0, 50.0, 1);
    let e = empty.execute("document.export", &json!({"format": "png", "useArtboards": false})).unwrap_err().to_string();
    assert!(e.contains("no visible art"), "{e}");
}

#[test]
fn formats_list_use_artboards() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let opts = |id: &str| r["formats"].as_array().unwrap().iter().find(|f| f["id"] == id).unwrap()["options"].clone();
    for id in ["svg", "pdf", "png", "jpg", "webp"] {
        assert!(opts(id).get("useArtboards").is_some(), "{id} lists useArtboards");
        assert!(opts(id).get("range").is_some(), "{id} lists range");
    }
    assert!(opts("vectorcraft").get("useArtboards").is_none());
}

/// An export's reply gives what the file loses before its notes on the options, whatever order
/// the encoder gave them: the WebP encoder notes that lossy WebP isn't applied before the export
/// notes the type it drew in the fallback font. With only notes on the options, they keep their
/// order.
#[test]
fn what_the_file_loses_comes_before_a_note_on_the_options() {
    let lossy = "lossy WebP isn't available yet: the file is lossless (exact, but larger)";
    let warnings = |s: &mut Session, p: Value| -> Vec<String> {
        let r = s.execute("document.export", &p).unwrap();
        r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect()
    };
    let mut s = session(60.0, 40.0, 1);
    assert_eq!(warnings(&mut s, json!({"format": "webp", "lossless": false})), [lossy]);
    s.execute("text.create", &json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let w = warnings(&mut s, json!({"format": "webp", "lossless": false}));
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(w[0].contains("fallback font") && w[0].contains("Nonexistent Sans Probe"), "{w:?}");
    assert_eq!(w[1], lossy);
    // document.serialize gives them in the same order.
    let r = s.execute("document.serialize", &json!({"format": "webp", "lossless": false})).unwrap();
    assert_eq!(r["warnings"], json!(w));
}

/// A session with one artboard and a selected rectangle on it.
fn with_art() -> Session {
    let mut s = session(60.0, 40.0, 1);
    s.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 10})).unwrap();
    s
}

/// The warnings of reply `r` that name an option the file doesn't read.
fn unused(r: &Value) -> Vec<String> {
    r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("was not used")).map(str::to_string).collect()
}

/// An option that only another format reads is accepted, and the reply's warnings name it in the
/// words of a save's note; `document.export`, `document.serialize` and `document.exportSelection`
/// give the same note.
#[test]
fn an_option_only_another_format_reads_gets_a_note() {
    let (mut s, notes) = (with_art(), unused);
    for cmd in ["document.export", "document.serialize", "document.exportSelection"] {
        let r = s.execute(cmd, &json!({"format": "pdf", "quality": 80, "outlineText": true})).unwrap();
        assert_eq!(
            notes(&r),
            ["`outlineText` was not used: PDF files don't read it", "`quality` was not used: PDF files don't read it"],
            "{cmd}: {r}"
        );
    }
    // The format's own options, the keys every export takes and null values get none.
    for p in [
        json!({"format": "jpg", "quality": 80, "scale": 2}),
        json!({"format": "ai", "compress": true, "compatibility": "1.7"}),
        json!({"format": "pdf", "quality": null, "selectedOnly": false, "includeArtboardFill": false, "useArtboards": true}),
        json!({"format": "svg", "outlineText": true, "svg": {"decimals": 2}}),
    ] {
        let r = s.execute("document.export", &p).unwrap();
        assert!(notes(&r).is_empty(), "{p}: {r}");
    }
    let r = s.execute("document.serialize", &json!({"format": "png", "compress": true, "marks": {"trim": true}, "svg": {"minify": true}})).unwrap();
    assert_eq!(
        notes(&r),
        [
            "`compress` was not used: PNG files don't read it",
            "`marks` was not used: PNG files don't read it",
            "`svg` was not used: PNG files don't read it"
        ],
        "{r}"
    );
    // The note names the kind of file written, as a save's does.
    let r = s.execute("document.export", &json!({"format": "ai", "pdfCompatible": false, "quality": 80})).unwrap();
    assert_eq!(notes(&r), ["`quality` was not used: non-PDF-compatible .ai files don't read it"], "{r}");
    let r = s.execute("document.export", &json!({"format": "pdf", "preserveEditing": false, "range": "1"})).unwrap();
    assert!(notes(&r).is_empty(), "{r}");
    let r = s.execute("document.export", &json!({"format": "vectorcraft", "artboard": 0})).unwrap();
    assert_eq!(notes(&r), ["`artboard` was not used: VectorCraft files don't read it"], "{r}");
}

/// A legacy `.ai` file reads its own options and the artboards to write, not the other `.ai`
/// options or the PDF options.
#[test]
fn a_legacy_ai_export_notes_the_options_of_the_other_ai_files() {
    let (mut s, notes) = (with_art(), unused);
    let r = s.execute("document.export", &json!({"format": "ai", "aiVersion": "7", "compress": true, "marks": {"trim": true}})).unwrap();
    assert_eq!(
        notes(&r),
        ["`compress` was not used: legacy .ai (version 7) files don't read it", "`marks` was not used: legacy .ai (version 7) files don't read it"],
        "{r}"
    );
    let r =
        s.execute("document.export", &json!({"format": "ai", "aiVersion": "7", "outlineText": true, "hiddenObjects": "omit", "range": "1"})).unwrap();
    assert!(notes(&r).is_empty(), "{r}");
    let r = s.execute("document.export", &json!({"format": "ai", "aiVersion": "7", "useArtboards": false})).unwrap();
    assert!(notes(&r).is_empty(), "{r}");
}

/// The `.ai` files other than the legacy one don't read the legacy file's own options.
#[test]
fn a_pdf_compatible_ai_export_notes_the_legacy_options() {
    let (mut s, notes) = (with_art(), unused);
    let unread = json!({"format": "ai", "aiVersion": "pdf", "outlineText": true, "hiddenObjects": "omit", "transparentImages": "omit"});
    let r = s.execute("document.export", &unread).unwrap();
    assert_eq!(
        notes(&r),
        [
            "`hiddenObjects` was not used: PDF-compatible .ai files don't read it",
            "`outlineText` was not used: PDF-compatible .ai files don't read it",
            "`transparentImages` was not used: PDF-compatible .ai files don't read it"
        ],
        "{r}"
    );
}

/// `preserveEditing` is always on for a `.ai` file, so an export that gives it gets a note.
#[test]
fn an_ai_export_notes_preserve_editing() {
    let (mut s, notes) = (with_art(), unused);
    let r = s.execute("document.export", &json!({"format": "ai", "preserveEditing": false, "range": "1"})).unwrap();
    assert_eq!(notes(&r), ["`preserveEditing` was not used: PDF-compatible .ai files don't read it"], "{r}");
}

/// An export writes one file: only a save writes each artboard to a file of its own, so an export
/// notes `separateArtboards`, and the `range` that names those files for a native file.
#[test]
fn an_export_notes_the_options_that_only_a_save_reads() {
    let (mut s, notes) = (with_art(), unused);
    let r = s.execute("document.export", &json!({"format": "ai", "separateArtboards": true, "range": "1"})).unwrap();
    assert_eq!(notes(&r), ["`separateArtboards` was not used: PDF-compatible .ai files don't read it"], "{r}");
    for (format, label) in [("vectorcraft", "VectorCraft"), ("template", "VectorCraft Template")] {
        let r = s.execute("document.export", &json!({"format": format, "separateArtboards": true, "range": "1", "compress": true})).unwrap();
        assert_eq!(
            notes(&r),
            [format!("`range` was not used: {label} files don't read it"), format!("`separateArtboards` was not used: {label} files don't read it")],
            "{r}"
        );
    }
}

/// The legacy `.ai` options that the notes go by are the ones the `.ai` option table describes as
/// the legacy file's ("with aiVersion 7"), so an option added to the table is not noted as unread
/// on a legacy export by mistake.
#[test]
fn the_legacy_ai_options_are_the_ones_the_option_table_names() {
    let described: Vec<&str> = native::AI_OPTIONS.iter().filter(|o| o.description.starts_with("with aiVersion 7")).map(|o| o.name).collect();
    assert_eq!(described, legacy_ai::LEGACY_ONLY);
    for key in described.iter().chain(&ARTBOARD_PARAMS).chain(&["aiVersion", "useArtboards"]) {
        assert!(legacy_ai::reads(key), "{key}");
        assert!(native::AI_OPTIONS.iter().any(|o| o.name == *key), "{key} is not a .ai option");
    }
    for key in ["compress", "pdfCompatible", "separateArtboards", "editingData", "marks"] {
        assert!(!legacy_ai::reads(key), "{key}");
    }
}

#[test]
fn note_texts_give_the_losses_first_each_kind_in_order() {
    use super::encode::{Note, note_texts};
    let notes = [Note::setting("s1"), Note::loss("l1"), Note::setting("s2"), Note::loss("l2")];
    assert_eq!(note_texts(&notes), ["l1", "l2", "s1", "s2"]);
    assert_eq!(note_texts(&[Note::setting("s1"), Note::setting("s2")]), ["s1", "s2"]);
    assert!(note_texts(&[]).is_empty());
}

/// A save notes the `.ai` options its file doesn't read as an export does: a PDF-compatible file
/// doesn't read the legacy file's options or `preserveEditing`, and a legacy file reads its own
/// options only. The notes cover the options beside the path and in `options`.
#[test]
fn a_save_notes_the_ai_options_its_file_does_not_read() {
    let (mut s, notes) = (with_art(), unused);
    let r = s
        .execute("file.saveCopy", &json!({"format": "ai", "outlineText": true, "preserveEditing": false, "options": {"hiddenObjects": "omit"}}))
        .unwrap();
    assert_eq!(
        notes(&r),
        [
            "`outlineText` was not used: PDF-compatible .ai files don't read it",
            "`preserveEditing` was not used: PDF-compatible .ai files don't read it",
            "`options.hiddenObjects` was not used: PDF-compatible .ai files don't read it"
        ],
        "{r}"
    );
    let p = json!({"format": "ai", "aiVersion": "7", "compress": true, "marks": {"trim": true}, "options": {"pdfCompatible": false}});
    let r = s.execute("file.saveCopy", &p).unwrap();
    assert_eq!(
        notes(&r),
        [
            "`compress` was not used: legacy .ai (version 7) files don't read it",
            "`marks` was not used: legacy .ai (version 7) files don't read it",
            "`options.pdfCompatible` was not used: legacy .ai (version 7) files don't read it"
        ],
        "{r}"
    );
    // The file's own options get none.
    for p in [
        json!({"format": "ai", "aiVersion": "7", "outlineText": true, "options": {"hiddenObjects": "omit", "range": "1"}}),
        json!({"format": "ai", "compress": true, "marks": {"trim": true}, "options": {"editingData": false}}),
        json!({"format": "pdf", "marks": {"trim": true}, "preserveEditing": false}),
    ] {
        let r = s.execute("file.saveCopy", &p).unwrap();
        assert!(notes(&r).is_empty(), "{p}: {r}");
    }
}

/// A save notes an option beside the path that only another save format takes, as it notes one in
/// `options`.
#[test]
fn a_save_notes_an_option_beside_the_path_that_only_another_save_format_takes() {
    let (mut s, notes) = (with_art(), unused);
    let r = s.execute("file.saveCopy", &json!({"format": "vectorcraft", "marks": {"trim": true}, "svg": {"minify": true}})).unwrap();
    assert_eq!(notes(&r), ["`marks` was not used: VectorCraft files don't read it", "`svg` was not used: VectorCraft files don't read it"], "{r}");
}

/// The `.ai` options a save's file doesn't read keep the values the document's last save in that
/// format gave them, so a save from Save Options, which sends only the options the file reads,
/// leaves the other version's choices as they were.
#[test]
fn a_save_keeps_the_ai_options_its_file_does_not_read_from_the_last_save() {
    let mut s = with_art();
    let dir = tmp_dir("ai-kept");
    let path = dir.join("kept.ai").to_string_lossy().to_string();
    let value = |s: &mut Session, key: &str| s.execute("file.formatOptions", &json!({"format": "ai"})).unwrap()["options"][key]["value"].clone();
    s.execute("file.saveAs", &json!({"path": path, "format": "ai", "options": {"aiVersion": "7", "outlineText": true}})).unwrap();
    s.execute("file.saveAs", &json!({"path": path, "format": "ai", "options": {"aiVersion": "pdf", "editingData": false}})).unwrap();
    assert_eq!(value(&mut s, "outlineText"), json!(true));
    assert_eq!(value(&mut s, "editingData"), json!(false));
    s.execute("file.saveAs", &json!({"path": path, "format": "ai", "options": {"aiVersion": "7", "hiddenObjects": "omit"}})).unwrap();
    assert_eq!(value(&mut s, "editingData"), json!(false));
    // An option the request gives is the one saved.
    s.execute("file.saveAs", &json!({"path": path, "format": "ai", "options": {"aiVersion": "7", "outlineText": false}})).unwrap();
    assert_eq!(value(&mut s, "outlineText"), json!(false));
    // The kept options are neither written nor sent again: a plain save's options (which a retry
    // in another folder sends to Save As) hold only what the legacy file reads.
    for p in [json!({}), json!({"options": {"aiVersion": "7"}})] {
        let plan = save_plan(&s, SaveMode::Save, &p).unwrap();
        assert!(!plan.options.contains_key("editingData") && !plan.options.contains_key("compress"), "{:?}", plan.options);
        assert_eq!(plan.kept.get("editingData"), Some(&json!(false)));
        assert!(plan.notes.is_empty(), "{:?}", plan.notes);
        let retry = json!({"path": path, "format": "ai", "options": plan.options});
        let r = s.execute("file.saveAs", &retry).unwrap();
        assert!(!r["warnings"].to_string().contains("was not used"), "{r}");
        assert_eq!(value(&mut s, "editingData"), json!(false));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The first `.ai` save of a document whose own file is another format remembers only the options
/// it was given: nothing from that format's options is kept.
#[test]
fn a_first_ai_save_keeps_only_the_options_it_is_given() {
    let mut s = with_art();
    let dir = tmp_dir("ai-first");
    let native = dir.join("first.vectorcraft").to_string_lossy().to_string();
    let path = dir.join("first.ai").to_string_lossy().to_string();
    s.execute("file.saveAs", &json!({"path": native, "options": {"compress": false}})).unwrap();
    let plan = save_plan(&s, SaveMode::SaveAs, &json!({"path": path, "format": "ai", "options": {"aiVersion": "7"}})).unwrap();
    assert!(plan.kept.is_empty(), "{:?}", plan.kept);
    s.execute("file.saveAs", &json!({"path": path, "format": "ai", "options": {"aiVersion": "7"}})).unwrap();
    let o = s.execute("file.formatOptions", &json!({"format": "ai"})).unwrap();
    assert_eq!(o["options"]["compress"]["value"], json!(true), "{o}");
    std::fs::remove_dir_all(&dir).unwrap();
}
