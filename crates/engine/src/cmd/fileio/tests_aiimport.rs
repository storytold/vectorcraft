//! Illustrator EPS, `.ai` and PDF files open from the editing data they carry: layers, groups,
//! names, hidden objects, artboards, colours and images as they were; what the reader doesn't read
//! yet opens as before, with a warning saying why.

use std::io::Write as _;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_testkit::ai;
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};

use super::*;

fn open(s: &mut Session, name: &str, bytes: &[u8], params: Value) -> Result<Value> {
    let mut p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), params.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("document.open", &p)
}

/// The `.ai` private data of `data`, compressed as older `.ai` files compress it.
fn compressed(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    [&b"%AI12_CompressedData"[..], &e.finish().unwrap()].concat()
}

fn names(nodes: &[Arc<Node>]) -> Vec<String> {
    nodes.iter().map(|n| n.name.clone().unwrap_or_default()).collect()
}

/// The sample's structure, as [`ai::sample_data`] builds it.
fn check_sample(s: &Session) {
    let doc = &s.doc().unwrap().doc;
    assert_eq!(names(&doc.layers), ["Art", "Hidden"]);
    assert_eq!(doc.artboards.len(), 1);
    assert_eq!(doc.artboards[0].rect, vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 100.0));
    let art = doc.layers[0].children().unwrap();
    assert_eq!(names(art), ["Pair", "Ring", "Window"]);
    let pair = art[0].children().unwrap();
    assert_eq!(names(pair), ["Red", ""]);
    assert!(pair[0].visible && !pair[1].visible, "the hidden member stays hidden");
    assert!(matches!(art[1].kind, NodeKind::Compound { .. }));
    assert!(matches!(&art[2].kind, NodeKind::Group { clip: true, children } if matches!(children[0].kind, NodeKind::Path { clipping: true, .. })));
    let hidden = &doc.layers[1];
    assert!(!hidden.visible);
    let kids = hidden.children().unwrap();
    assert_eq!(names(kids), ["Shaded", "", "Sub", ""]);
    assert!(matches!(kids[0].appearance.fill().map(|f| &f.paint), Some(vectorcraft_color::Paint::Gradient(_))));
    assert_eq!((kids[1].opacity, kids[1].blend), (0.5, vectorcraft_color::BlendMode::Screen));
    assert!(kids[2].locked && matches!(kids[2].kind, NodeKind::Layer { .. }));
    let NodeKind::Image(im) = &kids[3].kind else { panic!("an image") };
    assert_eq!((im.width, im.height), (2, 1));
}

#[test]
fn an_illustrator_eps_opens_with_its_layers() {
    let mut s = Session::new();
    let r = open(&mut s, "art.eps", &ai::eps(&ai::sample_data(), ai::page_ps()), json!({})).unwrap();
    assert_eq!((&r["format"], &r["warnings"]), (&json!("eps"), &json!([])), "{r}");
    check_sample(&s);
    // Saved as EPS and opened again, it comes back the same.
    let eps = s.execute("document.exportEps", &json!({})).unwrap();
    let bytes = vectorcraft_format::base64_decode(eps["dataBase64"].as_str().unwrap()).unwrap();
    let r = open(&mut s, "again.eps", &bytes, json!({})).unwrap();
    assert_eq!(r["restored"], json!(true), "{r}");
    check_sample(&s);
}

#[test]
fn an_illustrator_ai_file_opens_from_its_editing_data() {
    let mut s = Session::new();
    let file = ai::ai(&compressed(&ai::sample_data()), ai::page_pdf());
    let r = open(&mut s, "art.ai", &file, json!({})).unwrap();
    assert_eq!(r["format"], json!("ai"), "{r}");
    assert!(r["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap().contains("weren't read")), "{r}");
    check_sample(&s);
    // Pages picked: the PDF's.
    open(&mut s, "art.ai", &file, json!({"pages": "1"})).unwrap();
    assert_ne!(names(&s.doc().unwrap().doc.layers), ["Art", "Hidden"]);
}

/// A `.ai` file opens with the swatches its palette holds, spot colours the art doesn't use and colour
/// groups included, in place of the default swatches; art painted with Registration (`XZ`) still
/// opens from the editing data.
#[test]
fn an_ai_file_opens_with_the_swatches_of_its_palette() {
    let palette = "%AI5_BeginPalette\n0 0 Pb\n0 0 0 0 1 1 1 Xa\n(White)\nPc\n0.04 0.3 0 0 81.96 21 -8 (Pink ink) 0 2 Xx\n(Pink ink)\nPc\n\
                   0.06 0.98 1 0 (Red ink) 0 x\n(Red ink)\nPc\n1 (Inks) 1 Pg\n0.02 0.1 1 0 (Yellow ink) 0 x\n(Yellow ink)\nPc\nPB\n%AI5_EndPalette\n";
    // The square the page draws, in the red ink, after a stroke colour of Registration.
    let art = format!("0 0 0 0 0 0 0 ([Registration]) 0 1 XZ\n0.06 0.98 1 0 (Red ink) 0 x\n{}f\n", ai::rect(10.0, 50.0, 40.0, 90.0));
    let data = ai::editing_data(200.0, 100.0, &format!("{palette}{}", ai::layer("Art", &art)));
    let mut s = Session::new();
    let r = open(&mut s, "inks.ai", &ai::ai(&compressed(data.as_bytes()), "1 0 0 rg 10 50 30 40 re f"), json!({})).unwrap();
    assert_eq!(r["format"], json!("ai"), "{r}");
    let doc = &s.doc().unwrap().doc;
    assert_eq!(names(&doc.layers), ["Art"], "opened from the editing data: {r}");
    let swatches: Vec<&str> = doc.swatches.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(swatches, ["[None]", "White", "Pink ink", "Red ink"]);
    let pink = doc.swatch("Pink ink").unwrap();
    assert!(pink.spot && pink.global);
    assert_eq!(pink.paint.color(), Some(vectorcraft_color::Color::lab(81.96, 21.0, -8.0)), "a Lab ink keeps its Lab values");
    let groups: Vec<&str> = doc.swatch_groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(groups, ["Inks"]);
    assert!(doc.swatch("Yellow ink").is_some_and(|s| s.spot), "an ink the art doesn't use");
    let red = &doc.layers[0].children().unwrap()[0];
    assert!(
        matches!(red.appearance.fill().map(|f| &f.paint), Some(vectorcraft_color::Paint::Solid { swatch: Some(n), .. }) if n == "Red ink"),
        "{:?}",
        red.appearance.fill()
    );
}

/// A 1-bit RGB image opens from the editing data in the colors its PDF part shows.
#[test]
fn a_one_bit_rgb_image_opens_from_the_editing_data_in_its_colors() {
    // 3 × 1: red, green, blue (100 010 001), as hex samples in both parts.
    let raster = "%AI5_File:\n%AI5_BeginRaster\n() 1 XG\n/DeviceRGB XN\n[ 60 0 0 60 10 90 ] 3 1 0 Xh\n\
         [ 60 0 0 60 10 90 ] 0 0 3 1 3 1 1 3 0 0 0 0 4 4 0 0\n%%BeginData: 6\rXI\n8880\n%%EndData\r\nXH\r\n%AI5_EndRaster\r\nN\r\n";
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", raster));
    let page = "q 180 0 0 60 10 30 cm BI /W 3 /H 1 /CS /RGB /BPC 1 /F /AHx ID 8880> EI Q";
    let file = ai::ai(&compressed(data.as_bytes()), page);
    let mut s = Session::new();
    let r = open(&mut s, "bits.ai", &file, json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    assert_eq!(names(&doc.layers), ["Art"], "{r}");
    let mut found = vec![];
    doc.visit_images(|_, im| {
        let img = image::load_from_memory(&doc.images[&im.key].bytes).unwrap().to_rgb8();
        found.push(img.pixels().map(|p| p.0).collect::<Vec<_>>());
    });
    assert_eq!(found, [[[255, 0, 0], [0, 255, 0], [0, 0, 255]]], "{r}");
}

/// A page of a square and a line of type, as a `.ai`'s PDF part draws them.
const SQUARE_AND_TYPE: &str = "0 g 10 10 40 40 re f BT /F1 12 Tf 100 50 Td (Hi) Tj ET";

/// `file` loaded with `clipboard.importPdf` (`editingData` as given) and pasted → the command's
/// reply and the objects the paste selected, in stacking order.
fn paste(s: &mut Session, file: &[u8], editing_data: bool) -> (Value, Vec<Node>) {
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(file), "editingData": editing_data});
    let r = s.execute("clipboard.importPdf", &p).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let st = s.doc().unwrap();
    (r, st.selection.in_paint_order(&st.doc).into_iter().filter_map(|id| st.doc.node(id).cloned()).collect())
}

/// Is any of `nodes` (or an object inside one) type?
fn has_type(nodes: &[Node]) -> bool {
    let mut found = false;
    nodes.iter().for_each(|n| n.walk(&mut |c| found |= matches!(c.kind, NodeKind::Text(_))));
    found
}

/// A `.ai` copied in a file manager pastes the objects of its editing data, as `document.open`
/// reads it: both layers' objects in their stacking order, into the active layer, with their names
/// and hidden members, and its type as type. Without `editingData`, as for a PDF, its page pastes:
/// the shown layer's art without names.
#[test]
fn a_copied_ai_file_pastes_from_its_editing_data() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let file = ai::ai(&compressed(&ai::sample_data()), ai::page_pdf());
    let (r, pasted) = paste(&mut s, &file, true);
    assert_eq!(r["count"], 7, "{r}");
    let pasted_names: Vec<String> = pasted.iter().map(|n| n.name.clone().unwrap_or_default()).collect();
    assert_eq!(pasted_names, ["Pair", "Ring", "Window", "Shaded", "", "Sub", ""]);
    let pair = pasted[0].children().unwrap();
    assert!(pair[0].visible && !pair[1].visible, "the hidden member stays hidden");
    assert!(matches!(pasted[5].kind, NodeKind::Layer { .. }) && pasted[5].locked, "the sublayer comes as it is");
    assert_eq!(pasted.iter().map(|n| n.visible).collect::<Vec<_>>(), [true, true, true, false, false, false, false], "the hidden layer's objects");
    let layers = &s.doc().unwrap().doc.layers;
    assert_eq!(layers.len(), 1, "no layer is added");
    assert!(pasted.iter().all(|n| layers[0].children().unwrap().iter().any(|c| c.id == n.id)), "into the active layer");
    // The same editing data in an EPS: the same objects, the hidden layer's objects hidden.
    let eps = ai::eps(&ai::sample_data(), ai::page_ps());
    s.execute("clipboard.importEps", &json!({"dataBase64": vectorcraft_format::base64_encode(&eps)})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let st = s.doc().unwrap();
    let shown = |nodes: Vec<&Node>| nodes.iter().map(|n| (n.name.clone().unwrap_or_default(), n.visible)).collect::<Vec<_>>();
    assert_eq!(shown(st.selection.in_paint_order(&st.doc).into_iter().filter_map(|id| st.doc.node(id)).collect()), shown(pasted.iter().collect()));
    let (r, page) = paste(&mut s, &file, false);
    assert!(page.iter().all(|n| n.name.is_none()), "{r}");
    // A page of text only, or only a placeholder (saved without PDF compatibility): the editing
    // data's objects all the same.
    for (page, boxed) in [(TEXT_ART, "/ArtBox [0 0 200 100]"), (PLACEHOLDER, "")] {
        let (r, pasted) = paste(&mut s, &ai::ai_with(&compressed(&ai::sample_data()), page, boxed), true);
        assert_eq!(pasted.first().and_then(|n| n.name.as_deref()), Some("Pair"), "{page}: {r}");
        assert_eq!(r["count"], 7, "{page}: {r}");
    }
    // Type: a text object from the editing data, outlines from the page.
    let data = ai::editing_data(
        200.0,
        100.0,
        &ai::layer("Art", &format!("0 g\n{}f\n/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\n", ai::rect(10.0, 10.0, 50.0, 50.0))),
    );
    let typed = ai::ai(&compressed(data.as_bytes()), SQUARE_AND_TYPE);
    // (The page's type comes in as type too now that PDF import keeps it, so only this side is told apart.)
    let (r, pasted) = paste(&mut s, &typed, true);
    assert!(has_type(&pasted), "{r}: {pasted:?}");
    // A `.ai` VectorCraft wrote: the document it carries, its hidden square too.
    let mut other = Session::new();
    other.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    for x in [10, 100] {
        other.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap();
    }
    other.execute("object.hide", &json!({})).unwrap();
    let saved = other.execute("document.export", &json!({"format": "ai"})).unwrap();
    let saved = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    let (r, pasted) = paste(&mut s, &saved, true);
    assert_eq!(pasted.iter().map(|n| n.visible).collect::<Vec<_>>(), [true, false], "{r}");
    let (r, page) = paste(&mut s, &saved, false);
    assert!(page.iter().all(|n| n.visible), "{r}");
}

/// A PDF saved with the editing data opens from it, as a `.ai` does (#868).
#[test]
fn a_pdf_saved_with_the_editing_data_opens_from_it() {
    let mut s = Session::new();
    let file = ai::pdf(&compressed(&ai::sample_data()), ai::page_pdf());
    let r = open(&mut s, "art.pdf", &file, json!({})).unwrap();
    assert_eq!((&r["format"], &r["warnings"]), (&json!("pdf"), &json!([])), "{r}");
    check_sample(&s);
    // Without it: the PDF part.
    let r = open(&mut s, "art.pdf", &file, json!({"editingData": false})).unwrap();
    let layers = &s.doc().unwrap().doc.layers;
    assert_eq!(layers.len(), 1, "{r}");
    assert_ne!(names(layers), ["Art"]);
}

/// Art crossing the page's edge opens whole from a PDF's editing data, without the note that the
/// PDF's art was clipped to the page; without the editing data it is clipped, with the note.
#[test]
fn a_pdfs_art_past_the_page_opens_whole_from_its_editing_data() {
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &format!("0 g\n{}f\n", ai::rect(150.0, 20.0, 250.0, 80.0))));
    let file = ai::pdf(data.as_bytes(), "0 g 150 20 100 60 re f");
    let mut s = Session::new();
    let r = open(&mut s, "past.pdf", &file, json!({})).unwrap();
    assert_eq!(r["warnings"], json!([]), "{r}");
    let doc = &s.doc().unwrap().doc;
    assert_eq!(names(&doc.layers), ["Art"]);
    assert!(doc.art_bounds().is_some_and(|b| b.x1 > 200.5), "{:?}", doc.art_bounds());
    let r = open(&mut s, "past.pdf", &file, json!({"editingData": false})).unwrap();
    assert_eq!(r["warnings"], json!([vectorcraft_pdf::PAST_PAGE_NOTE]), "{r}");
    assert!(s.doc().unwrap().doc.art_bounds().is_some_and(|b| b.x1 < 200.5));
}

/// A `.ai` or a PDF whose editing data can't be decompressed opens from its PDF part with a note
/// that its layers weren't read, and an export to its own path is refused unless it is given
/// `acknowledgeLoss`.
#[test]
fn a_file_whose_editing_data_cant_be_decompressed_is_not_written_over_unasked() {
    let d = std::env::temp_dir().join(format!("vc-aiundecoded-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let damaged = b"%AI12_CompressedDataxx";
    for (name, file) in [("art.ai", ai::ai(damaged, ai::page_pdf())), ("art.pdf", ai::pdf(damaged, ai::page_pdf()))] {
        let path = d.join(name);
        std::fs::write(&path, &file).unwrap();
        let path = path.to_string_lossy().to_string();
        let mut s = Session::new();
        let r = s.execute("document.open", &json!({"path": path})).unwrap();
        assert!(r["warnings"].to_string().contains("layers weren't read from its editing data (its zlib compression is damaged)"), "{name}: {r}");
        let e = s.execute("document.export", &json!({"path": path})).unwrap_err().to_string();
        assert!(e.contains("left things out") && e.contains("layers weren't read"), "{name}: {e}");
        assert_eq!(std::fs::read(&path).unwrap(), file, "{name}");
        s.execute("document.export", &json!({"path": path, "acknowledgeLoss": true})).unwrap();
        assert_ne!(std::fs::read(&path).unwrap(), file, "{name}");
    }
    std::fs::remove_dir_all(d).ok();
}

/// Editing data that can't be read leaves the file as its PDF part reads, with that part's notes
/// and then a note saying why its layers weren't read: a `.ai`'s art outside its artboards is
/// missing, and a PDF's art past the page is clipped.
#[test]
fn editing_data_that_cant_be_read_keeps_the_pdf_parts_notes() {
    let damaged = b"%AI12_CompressedData not zlib";
    let unread = "the file's layers weren't read from its editing data (its zlib compression is damaged): they come from its PDF part";
    let mut s = Session::new();
    let r = open(&mut s, "damaged.ai", &ai::ai(damaged, ai::page_pdf()), json!({})).unwrap();
    assert_eq!(r["warnings"], json!([vectorcraft_pdf::OFF_ARTBOARD_NOTE, unread]), "{r}");
    let r = open(&mut s, "damaged.pdf", &ai::pdf(damaged, "0 g 150 20 100 60 re f"), json!({})).unwrap();
    assert_eq!(r["warnings"], json!([vectorcraft_pdf::PAST_PAGE_NOTE, unread]), "{r}");
}

/// A copied `.ai` whose editing data can't be read, or isn't there, pastes its page as a PDF
/// does: the same objects as without `editingData`.
#[test]
fn a_copied_ai_file_whose_editing_data_cant_be_read_pastes_its_page() {
    let symbols = ai::editing_data(200.0, 100.0, &ai::layer("Art", "/SymbolInstance :\n(Dot) /SymbolRef ,\n;\n"));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    for (what, private) in
        [("damaged", b"%AI12_CompressedData not zlib".to_vec()), ("unread layers", compressed(symbols.as_bytes())), ("none", vec![])]
    {
        let file = ai::ai(&private, SQUARE_AND_TYPE);
        let (r, pasted) = paste(&mut s, &file, true);
        let (want, page) = paste(&mut s, &file, false);
        assert_eq!((&r["count"], &r["warnings"]), (&want["count"], &want["warnings"]), "{what}");
        let kinds = |nodes: &[Node]| nodes.iter().map(|n| std::mem::discriminant(&n.kind)).collect::<Vec<_>>();
        assert_eq!(kinds(&pasted), kinds(&page), "{what}");
        assert!(!pasted.is_empty(), "{what}: {r}");
    }
    // Editing data that reads and draws as the page does: its objects, not the page's.
    let (r, _) = paste(&mut s, &ai::ai(&compressed(&ai::sample_data()), ai::page_pdf()), true);
    assert_eq!(r["count"], 7, "{r}");
}

#[test]
fn what_the_reader_doesnt_read_opens_as_before_with_a_warning() {
    let symbols = ai::editing_data(200.0, 100.0, &ai::layer("Art", "/SymbolInstance :\n(Dot) /SymbolRef ,\n;\n"));
    let mut s = Session::new();
    let r = open(&mut s, "symbols.eps", &ai::eps(symbols.as_bytes(), ai::page_ps()), json!({})).unwrap();
    assert!(r["warnings"].to_string().contains("layers weren't read") && r["warnings"].to_string().contains("symbols"), "{r}");
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Layer 1"]);
    let r = open(&mut s, "symbols.ai", &ai::ai(&compressed(symbols.as_bytes()), ai::page_pdf()), json!({})).unwrap();
    assert!(r["warnings"].to_string().contains("symbols") && r["warnings"].to_string().contains("PDF part"), "{r}");
}

/// A `.ai` whose document data has a variable library opens with a note that its variables and
/// data sets weren't read, whether its layers are read or it comes in as its PDF part.
#[test]
fn an_ai_file_with_a_variable_library_says_its_variables_werent_read() {
    const NOTE: &str = "the file's variables and data sets weren't read (variables: 2, data sets: 3): the Variables panel doesn't list them";
    let library = ai::variable_library(&["Name", "Number"], &["One", "Two", "Three"]);
    let square = format!("0 1 1 0 1 0 0 Xa\n{}f\n", ai::rect(10.0, 50.0, 40.0, 90.0));
    let notes =
        |r: &Value| r["warnings"].as_array().unwrap().iter().filter(|w| w.as_str().unwrap().contains("variables")).cloned().collect::<Vec<_>>();
    let mut s = Session::new();
    let layered = ai::editing_data(200.0, 100.0, &format!("{library}{}", ai::layer("Art", &square)));
    let file = ai::ai(&compressed(layered.as_bytes()), "1 0 0 rg 10 50 30 40 re f");
    let r = open(&mut s, "variables.ai", &file, json!({})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
    assert_eq!(notes(&r), [json!(NOTE)], "{r}");
    let r = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(r["variables"], json!([]), "{r}");
    // Its layers can't be read (a symbol): it comes in as its PDF part, with the note.
    let symbols = ai::editing_data(200.0, 100.0, &format!("{library}{}", ai::layer("Art", "/SymbolInstance :\n(Dot) /SymbolRef ,\n;\n")));
    let r = open(&mut s, "symbols.ai", &ai::ai(&compressed(symbols.as_bytes()), ai::page_pdf()), json!({})).unwrap();
    assert!(r["warnings"].to_string().contains("PDF part"), "{r}");
    assert_eq!(notes(&r), [json!(NOTE)], "{r}");
    // Without a library, no note.
    let plain = ai::editing_data(200.0, 100.0, &ai::layer("Art", &square));
    let r = open(&mut s, "plain.ai", &ai::ai(&compressed(plain.as_bytes()), "1 0 0 rg 10 50 30 40 re f"), json!({})).unwrap();
    assert!(notes(&r).is_empty(), "{r}");
}

/// A page showing only a placeholder line, as a `.ai` saved without PDF compatibility has.
const PLACEHOLDER: &str = "BT /F1 12 Tf 10 50 Td (Saved without its PDF part) Tj ET";

#[test]
fn an_ai_file_saved_without_its_pdf_part_opens_from_its_editing_data() {
    let mut s = Session::new();
    let r = open(&mut s, "art.ai", &ai::ai(&compressed(&ai::sample_data()), PLACEHOLDER), json!({})).unwrap();
    assert_eq!(r["format"], json!("ai"), "{r}");
    check_sample(&s);
    // Its editing data damaged: it says so (the placeholder isn't the art).
    let e = open(&mut s, "damaged.ai", &ai::ai(b"%AI12_CompressedData not zlib", PLACEHOLDER), json!({})).unwrap_err().to_string();
    assert!(e.contains("without PDF compatibility") && e.contains("zlib"), "{e}");
    // Without editing data, the placeholder still isn't opened as the art.
    let e = open(&mut s, "plain.ai", &ai::ai(b"", PLACEHOLDER), json!({})).unwrap_err().to_string();
    assert!(e.contains("placeholder"), "{e}");
}

#[test]
fn an_ai_file_saved_without_its_pdf_part_opens_with_pages_naming_every_page() {
    let file = ai::ai(&compressed(&ai::sample_data()), PLACEHOLDER);
    for pages in [json!("1"), json!(1), json!("1-")] {
        let mut s = Session::new();
        open(&mut s, "art.ai", &file, json!({"pages": pages})).unwrap();
        check_sample(&s);
    }
    // Without editing data, the placeholder still isn't opened as the art.
    let e = open(&mut Session::new(), "plain.ai", &ai::ai(b"", PLACEHOLDER), json!({"pages": "1"})).unwrap_err().to_string();
    assert!(e.contains("placeholder"), "{e}");
}

/// [`PLACEHOLDER`]'s `.ai` files as `dataBase64`: the sample's editing data, damaged editing data
/// and none.
fn placeholder_files() -> [String; 3] {
    [compressed(&ai::sample_data()), b"%AI12_CompressedData not zlib".to_vec(), vec![]]
        .map(|private| vectorcraft_format::base64_encode(&ai::ai(&private, PLACEHOLDER)))
}

#[test]
fn an_ai_file_saved_without_its_pdf_part_pastes_from_its_editing_data() {
    let [sample, damaged, plain] = placeholder_files();
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    // clipboard.importPdf (the PDF flavor's command) takes every object of both layers, as
    // document.open reads them.
    let r = s.execute("clipboard.importPdf", &json!({"dataBase64": sample})).unwrap();
    assert_eq!(r["count"], json!(7), "{r}");
    s.execute("edit.paste", &json!({})).unwrap();
    let art = s.doc().unwrap().doc.layers[0].children().unwrap().clone();
    assert_eq!(names(&art), ["Pair", "Ring", "Window", "Shaded", "", "Sub", ""]);
    assert_eq!(art.iter().map(|n| n.visible).collect::<Vec<_>>(), [true, true, true, false, false, false, false], "the hidden layer's objects");
    let e = s.execute("clipboard.importPdf", &json!({"dataBase64": damaged})).unwrap_err().to_string();
    assert!(e.contains("without PDF compatibility") && e.contains("zlib"), "{e}");
    let e = s.execute("clipboard.importPdf", &json!({"dataBase64": plain})).unwrap_err().to_string();
    assert!(e.contains("placeholder"), "{e}");
}

#[test]
fn an_ai_file_saved_without_its_pdf_part_places_from_its_editing_data() {
    let [sample, damaged, plain] = placeholder_files();
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    // The place takes the art of the layers that show on the artboard and clips it to the artboard.
    let r = s.execute("file.place", &json!({"name": "art.ai", "dataBase64": sample, "link": false})).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(200.0), Some(100.0)), "{r}");
    let placed = s.doc().unwrap().doc.layers[0].children().unwrap()[0].clone();
    let kids = placed.children().unwrap();
    assert_eq!(names(&kids[1..]), ["Pair", "Ring", "Window"], "after the clipping path");
    // With `crop: "bounding"` the size is the bounds of the art that shows. They run from the red
    // square's left edge to the window's right one; the pair's hidden square, stroked 2 pt wide,
    // is left out.
    let r = s.execute("file.place.info", &json!({"name": "art.ai", "dataBase64": sample, "crop": "bounding"})).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(170.0), Some(40.0)), "{r}");
    // The place takes the art of both layers that show.
    let square = |x: f64| format!("0 g\n{}f\n", ai::rect(x, 10.0, x + 40.0, 50.0));
    let two = ai::editing_data(200.0, 100.0, &[ai::layer("Back", &square(10.0)), ai::layer("Front", &square(110.0))].concat());
    let two = vectorcraft_format::base64_encode(&ai::ai(&compressed(two.as_bytes()), PLACEHOLDER));
    let r = s.execute("file.place.info", &json!({"name": "two.ai", "dataBase64": two, "crop": "bounding"})).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(140.0), Some(40.0)), "{r}");
    // The file has two 100-point artboards and one placeholder page. The second artboard lies 50
    // below the first. A square of 40 lies on the first and an oblong of 60 × 40 on the second.
    let boards = "%_/Document :\n%_/Dictionary :\n%_/Array :\n\
        %_/Dictionary :\n%_0 100 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_100 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n\
        %_/Dictionary :\n%_0 -50 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_100 -150 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n\
        %_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n";
    let art = format!("0 g\n{}f\n{}f\n", ai::rect(10.0, 10.0, 50.0, 50.0), ai::rect(10.0, -140.0, 70.0, -100.0));
    let paged = ai::editing_data(100.0, 100.0, &(boards.to_string() + &ai::layer("Art", &art)));
    let paged = vectorcraft_format::base64_encode(&ai::ai(&compressed(paged.as_bytes()), PLACEHOLDER));
    let mut size = |page: u64| {
        let r = s.execute("file.place.info", &json!({"name": "boards.ai", "dataBase64": paged, "page": page, "crop": "bounding"}));
        r.map(|r| (r["width"].as_f64(), r["height"].as_f64())).map_err(|e| e.to_string())
    };
    assert_eq!(size(1), Ok((Some(40.0), Some(40.0))));
    assert_eq!(size(2), Ok((Some(60.0), Some(40.0))));
    let third = size(3);
    assert!(third.as_ref().is_err_and(|e| e.contains("has 2 page(s)")), "{third:?}");
    let e = s.execute("file.place", &json!({"name": "damaged.ai", "dataBase64": damaged})).unwrap_err().to_string();
    assert!(e.contains("without PDF compatibility") && e.contains("zlib"), "{e}");
    let e = s.execute("file.place", &json!({"name": "plain.ai", "dataBase64": plain})).unwrap_err().to_string();
    assert!(e.contains("placeholder"), "{e}");
}

/// A page of text alone, as text-only art has in its PDF part: no path is painted.
const TEXT_ART: &str = "BT /F1 36 Tf 10 40 Td (Text only) Tj ET";

/// The kinds of what the first layer holds: type (or its outlines, without the font) and others.
fn text_and_others(s: &Session) -> (usize, usize) {
    let art = s.doc().unwrap().doc.layers[0].children().unwrap();
    let text = art.iter().filter(|n| matches!(n.kind, NodeKind::Text(_)) || n.name.as_deref() == Some("<Text Outlines>")).count();
    (text, art.len() - text)
}

#[test]
fn an_ai_file_whose_pdf_part_is_only_text_opens_from_it_when_its_editing_data_cant_be_read() {
    // #1387. The page states where its art is (an art, trim or bleed box), which the notice
    // page of a file saved without its PDF part has no art to do.
    let unreadable = b"%AI24_ZStandard_Data\0\0\0\0\0\0\0\0";
    for boxed in ["/ArtBox [0 0 200 100]", "/TrimBox [0 0 200 100]", "/BleedBox [0 0 200 100]"] {
        let mut s = Session::new();
        let r = open(&mut s, "text.ai", &ai::ai_with(unreadable, TEXT_ART, boxed), json!({})).unwrap();
        // Then a note on the font, where the machine lacks it.
        assert_eq!(r["warnings"][0], json!(vectorcraft_pdf::OFF_ARTBOARD_NOTE), "{boxed}: {r}");
        assert_eq!(text_and_others(&s), (1, 0), "{boxed}");
        // The PDF part alone, as asked.
        open(&mut s, "text.ai", &ai::ai_with(unreadable, TEXT_ART, boxed), json!({"editingData": false})).unwrap();
        assert_eq!(text_and_others(&s), (1, 0), "{boxed}");
    }
    // The same page without a box is the notice: the file still says why it can't open.
    let mut s = Session::new();
    let e = open(&mut s, "text.ai", &ai::ai(unreadable, TEXT_ART), json!({})).unwrap_err().to_string();
    assert!(e.contains("without PDF compatibility") && e.contains("can't be read"), "{e}");
    let e = open(&mut s, "text.ai", &ai::ai(unreadable, TEXT_ART), json!({"editingData": false})).unwrap_err().to_string();
    assert!(e.contains("placeholder"), "{e}");
    // Placed, it is read as its page too.
    let r = s.execute("file.new", &json!({})).and_then(|_| {
        let data = vectorcraft_format::base64_encode(&ai::ai_with(unreadable, TEXT_ART, "/ArtBox [0 0 200 100]"));
        s.execute("file.place", &json!({"name": "text.ai", "dataBase64": data, "link": false}))
    });
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn an_ai_file_whose_pdf_part_is_only_text_still_opens_from_editing_data_that_reads() {
    let mut s = Session::new();
    open(&mut s, "art.ai", &ai::ai_with(&compressed(&ai::sample_data()), TEXT_ART, "/ArtBox [0 0 200 100]"), json!({})).unwrap();
    check_sample(&s);
}

#[test]
fn editing_data_false_opens_only_what_the_page_prints() {
    let mut s = Session::new();
    for (name, file) in [("art.eps", ai::eps(&ai::sample_data(), ai::page_ps())), ("art.ai", ai::ai(&compressed(&ai::sample_data()), ai::page_pdf()))]
    {
        let r = open(&mut s, name, &file, json!({"editingData": false})).unwrap();
        let layers = &s.doc().unwrap().doc.layers;
        assert_eq!(layers.len(), 1, "{name}: {r}");
        assert_ne!(names(layers), ["Art"], "{name}: the page's layer");
    }
    let e = open(&mut s, "art.eps", &ai::eps(&ai::sample_data(), ai::page_ps()), json!({"editingData": "no"})).unwrap_err().to_string();
    assert!(e.contains("editingData must be true, false or \"always\""), "{e}");
    // `always`: the editing data, as by default (whether it draws as the page does is the reader's).
    open(&mut s, "art.ai", &ai::ai(&compressed(&ai::sample_data()), ai::page_pdf()), json!({"editingData": "always"})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art", "Hidden"]);
    // A file that is only its editing data can't open without it.
    let e = open(&mut s, "art.ai", &ai::ai(&compressed(&ai::sample_data()), PLACEHOLDER), json!({"editingData": false})).unwrap_err().to_string();
    assert!(e.contains("placeholder"), "{e}");
}

/// A CMYK image opens with the same ink amounts from a `.ai` file's editing data as from its PDF
/// part.
#[test]
fn a_cmyk_image_keeps_its_inks_from_the_editing_data_as_from_the_pdf_part() {
    // 3 × 1: cyan, 100% black, then a mix, as hex samples in both parts.
    let inks = [255u8, 0, 0, 0, 0, 0, 0, 255, 12, 34, 56, 78];
    let hex: String = inks.iter().map(|b| format!("{b:02x}")).collect();
    let raster = format!(
        "%AI5_File:\n%AI5_BeginRaster\n() 1 XG\n/DeviceCMYK XN\n[ 10 0 0 10 10 60 ] 3 1 0 Xh\n\
         [ 10 0 0 10 10 60 ] 0 0 3 1 3 1 8 4 0 0 0 0 4 4 0 0\n%%BeginData: {}\rXI\n{hex}\n%%EndData\r\nXH\r\n%AI5_EndRaster\r\nN\r\n",
        hex.len() + 2
    );
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &raster));
    let page = format!("q 30 0 0 10 10 50 cm BI /W 3 /H 1 /CS /CMYK /BPC 8 /F /AHx ID {hex}> EI Q");
    let file = ai::ai(&compressed(data.as_bytes()), &page);
    let opened = |s: &Session| {
        let doc = &s.doc().unwrap().doc;
        let mut found = vec![];
        doc.visit_images(|_, im| found.push(doc.images[&im.key].cmyk().map(|i| i.data)));
        found
    };
    let mut s = Session::new();
    let r = open(&mut s, "cmyk.ai", &file, json!({})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
    assert_eq!(opened(&s), [Some(inks.to_vec())], "from the editing data");
    let r = open(&mut s, "cmyk.ai", &file, json!({"editingData": false})).unwrap();
    assert_ne!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
    assert_eq!(opened(&s), [Some(inks.to_vec())], "from the PDF part");
}

#[test]
fn fit_to_artwork_bounds_leaves_out_the_hidden_art_and_guides_the_editing_data_has() {
    // A shown square; a hidden one and a long guide far from it, which the page doesn't draw.
    let art =
        format!("0 g\n{}f\n1 Xw\n{}f\n0 Xw\n-500 20 m\n700 20 L\n(N) *\n", ai::rect(10.0, 10.0, 50.0, 50.0), ai::rect(150.0, 60.0, 190.0, 95.0));
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &art));
    let mut s = Session::new();
    let r = open(&mut s, "fit.eps", &ai::eps(data.as_bytes(), "0 setgray 10 10 40 40 rectfill\n"), json!({})).unwrap();
    let layer = &s.doc().unwrap().doc.layers[0];
    let kids = layer.children().unwrap();
    assert_eq!(kids.len(), 3, "{r}");
    assert!(!kids[1].visible && matches!(kids[2].kind, NodeKind::Path { guide: true, .. }));
    s.execute("artboard.fitToArt", &json!({})).unwrap();
    // The square, 50 to 90 down the 100-point page.
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, vectorcraft_geom::Rect::new(10.0, 50.0, 50.0, 90.0));
}

#[test]
fn type_asked_for_as_outlines_opens_a_file_whose_type_shows_from_its_pdf_part() {
    let text = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\n";
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &format!("0 g\n{}f\n{text}", ai::rect(10.0, 10.0, 50.0, 50.0))));
    let file = ai::ai(&compressed(data.as_bytes()), "0 g 10 10 40 40 re f BT /F1 12 Tf 100 50 Td (Hi) Tj ET");
    let mut s = Session::new();
    // As type, the page's type goes into the text object of the layers.
    let r = open(&mut s, "type.ai", &file, json!({})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
    // As outlines, only the PDF part has them.
    let r = open(&mut s, "type.ai", &file, json!({"textAs": "outlines"})).unwrap();
    assert!(r["warnings"].to_string().contains("outlines"), "{r}");
    assert_ne!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
}

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

/// Page content drawing one glyph and then `rest` in 10 pt on the baseline at `y`, from x = 14, in
/// rendering mode `mode` (0 fills the type, 1 strokes it). The glyph is code 0x41 of the font `F2`,
/// standing for the "fi" of the story, and `F2` isn't installed and its `ToUnicode` map gives that
/// glyph U+FFFD, so that the PDF part keeps it as outlines; `rest` is in Helvetica (`F1`).
fn half_outlined_line(y: f64, rest: &str, mode: u8) -> String {
    format!("{mode} Tr BT /F2 10 Tf 14 {y} Td (A) Tj ET BT /F1 10 Tf 19 {y} Td ({rest}) Tj ET ")
}

/// A `.ai` whose page draws a gray square and `type_content` (see [`half_outlined_line`]), and whose
/// layer `Art` has the square and text objects: the first of story 0, which `story` gives in the
/// text document (a line starting at (14, 65)), then `more` of a story the text document doesn't
/// have. Without `story` the file has no text document.
fn half_outlined_ai(story: Option<&str>, more: usize, type_content: &str) -> Vec<u8> {
    let text = |story: u32| format!("/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n{story} /StoryIndex ,\n;\n");
    let texts: String = std::iter::once(text(0)).chain((0..more).map(|_| text(99))).collect();
    let mut body =
        format!("%AI3_TemplateBox: 100 50 100 50\n{}", ai::layer("Art", &format!("0.5 g\n{}f\n{texts}", ai::rect(120.0, 20.0, 160.0, 60.0))));
    if let Some(story) = story {
        // The story's one line starts at the canvas center (8191.5, the template box's center) moved
        // to (14, 65): its y runs down the page.
        let n = story.chars().count();
        let document = format!(
            "/0 << /1 << /0 [ << /0 << /0 << /0 (Helvetica) >> >> >> ] >> /8 << /0 [ << /0 << /0 [ 0 0 ] /2 << /2 [ 1 0 0 1 0 0 ] >> >> >> ] >> >>\n\
             /1 << /1 [ << /0 << /0 ({story}\r) /5 << /0 [ << /0 << /0 << /0 () /5 << /0 0 >> /6 0 >> >> /1 {} >> ] >> \
             /6 << /0 [ << /0 << /0 << /0 () /5 0 /6 << /0 0 /1 10.0 /53 << /99 /CAITextPaint /0 << /0 2 /1 [ 1.0 0.0 0.0 0.0 1.0 ] >> >> >> >> >> /1 {} >> ] >> >> \
             /1 << /0 [ << /0 0 >> ] /2 [ << /99 /PC /6 [ << /99 /F /0 << /0 [ 8105.5 8176.5 ] >> /6 [ << /99 /R /6 [ << /99 /R /6 [ << /99 /L /6 [ \
             << /99 /S /0 << /0 [ 0.0 0.0 ] >> /15 << /0 {} >> >> ] >> ] >> ] >> ] >> ] >> ] >> >> ] /2 << /1 12.0 >> >>\n",
            n + 1,
            n + 1,
            n + 1
        );
        let lines: Vec<String> = ascii85(document.as_bytes()).as_bytes().chunks(60).map(|c| format!("%{}", String::from_utf8_lossy(c))).collect();
        body.push_str(&format!("%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n{}\n%AI11_EndTextDocument\n", lines.join("\n")));
    }
    let data = ai::editing_data(200.0, 100.0, &body);
    let content = format!("0.5 g 120 20 40 40 re f 0.2 g 0.2 G {type_content}");
    let page = PdfPage {
        resources: format!(
            "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> \
             /F2 << /Type /Font /Subtype /Type1 /BaseFont /VectorCraftTestUnnamed /ToUnicode {} 0 R >> >>",
            first_extra(1) + 1
        ),
        entries: format!("/PieceInfo << /Illustrator << /Private << /AIPrivateData1 {} 0 R /NumBlock 1 >> >> >>", first_extra(1)),
        ..PdfPage::new(200.0, 100.0, &content)
    };
    let cmap = "begincmap\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n1 beginbfchar\n<41> <FFFD>\nendbfchar\nendcmap\n";
    pdf_with(
        &[page],
        &[&format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len()), &format!("<< /Length {} >>\nstream\n{cmap}endstream", cmap.len())],
        None,
    )
}

fn outlines_notes(r: &Value) -> Vec<String> {
    let notes = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str);
    notes.filter(|w| vectorcraft_pdf::is_outlines_note(w)).map(String::from).collect()
}

/// The notes the PDF part of [`half_outlined_ai`] reads with: its glyph of U+FFFD, kept as outlines.
fn pdf_part_outlines_notes() -> [String; 2] {
    [format!("text in Vector Craft Test Unnamed {}", vectorcraft_pdf::UNNAMED_CHARACTERS_NOTE), vectorcraft_pdf::OUTLINED_TEXT_NOTE.into()]
}

/// The note on text objects that hold the page's type without some of its characters, for `n` of
/// them.
fn without_outlines_note(n: usize) -> Value {
    json!(format!("{n} text objects hold their type as the page draws it, without the characters the page has only as outlines"))
}

/// The texts in the first layer.
fn texts(s: &Session) -> Vec<String> {
    let art = s.doc().unwrap().doc.layers[0].children().unwrap();
    art.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.plain_text()) } else { None }).collect()
}

/// The note that glyphs were kept as outlines is dropped when the type comes from the editing data's
/// text document, and stays with the PDF part alone.
#[test]
fn type_from_the_text_document_opens_without_the_pdf_parts_note_on_outlines() {
    let file = half_outlined_ai(Some("first draft"), 0, &half_outlined_line(65.0, "rst draft", 0));
    let mut s = Session::new();
    let r = open(&mut s, "draft.ai", &file, json!({})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
    assert_eq!(texts(&s), ["first draft"], "{r}");
    assert_eq!(outlines_notes(&r), Vec::<String>::new(), "{r}");
    assert!(!r["warnings"].as_array().unwrap().contains(&without_outlines_note(1)), "{r}");
    let r = open(&mut s, "draft.ai", &file, json!({"editingData": false})).unwrap();
    assert_eq!(outlines_notes(&r), pdf_part_outlines_notes(), "{r}");
    // A second text object that takes the page's type, away from the glyph: no note on it.
    let page = half_outlined_line(65.0, "rst draft", 0) + "BT /F1 10 Tf 14 30 Td (notes) Tj ET";
    let r = open(&mut s, "notes.ai", &half_outlined_ai(Some("first draft"), 1, &page), json!({})).unwrap();
    assert_eq!(texts(&s), ["first draft", "notes"], "{r}");
    assert_eq!(outlines_notes(&r), Vec::<String>::new(), "{r}");
    assert!(!r["warnings"].as_array().unwrap().contains(&without_outlines_note(1)), "{r}");
}

/// A text object that takes the page's type lacks the glyphs next to it that the page kept as
/// outlines: a note counts such objects, in place of the PDF part's notes. Stroked glyphs that
/// come in with the stroked type keep the PDF part's notes.
#[test]
fn type_from_the_page_opens_with_a_note_on_the_characters_it_has_only_as_outlines() {
    let mut s = Session::new();
    let r = open(&mut s, "draft.ai", &half_outlined_ai(None, 0, &half_outlined_line(65.0, "rst draft", 0)), json!({})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
    assert_eq!(texts(&s), ["rst draft"], "{r}");
    assert!(!vectorcraft_pdf::holds_outlines(&s.doc().unwrap().doc), "{r}");
    assert_eq!(outlines_notes(&r), Vec::<String>::new(), "{r}");
    assert!(r["warnings"].as_array().unwrap().contains(&without_outlines_note(1)), "{r}");
    // Stroked: the glyph comes in with the type.
    let r = open(&mut s, "draft.ai", &half_outlined_ai(None, 0, &half_outlined_line(65.0, "rst draft", 1)), json!({})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
    assert!(vectorcraft_pdf::holds_outlines(&s.doc().unwrap().doc), "{r}");
    assert_eq!(outlines_notes(&r), pdf_part_outlines_notes(), "{r}");
    assert!(!r["warnings"].to_string().contains("without the characters"), "{r}");
    // A stroked line whose glyph comes in, and a filled one whose glyph doesn't.
    let page = half_outlined_line(65.0, "rst draft", 1) + &half_outlined_line(30.0, "nd more", 0);
    let r = open(&mut s, "both.ai", &half_outlined_ai(None, 1, &page), json!({})).unwrap();
    assert_eq!(texts(&s), ["rst draft", "nd more"], "{r}");
    assert_eq!(outlines_notes(&r), pdf_part_outlines_notes(), "{r}");
    assert!(r["warnings"].as_array().unwrap().contains(&without_outlines_note(1)), "{r}");
}

/// A `.ai` saved in units of 10 points: its PDF part's page is 200 × 100 with `/UserUnit 10`, and
/// its editing data has the artboard and a red rectangle in the same units. It opens at 2000 × 1000
/// pt, from its editing data and from its PDF part alone, with the rectangle ten times as large,
/// and `document.pdfInfo` gives the page in points (#1482).
#[test]
fn an_ai_file_with_a_user_unit_opens_at_its_size_in_points() {
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &format!("0 1 1 0 1 0 0 Xa\n{}f\n", ai::rect(20.0, 20.0, 60.0, 50.0))));
    let file = ai::ai_with(&compressed(data.as_bytes()), "1 0 0 rg 20 20 40 30 re f", "/UserUnit 10");
    let near = |r: vectorcraft_geom::Rect, want: [f64; 4]| [r.x0, r.y0, r.x1, r.y1].iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01);
    let mut s = Session::new();
    for (params, layers) in [(json!({}), ["Art"]), (json!({"editingData": false}), ["Page 1"])] {
        let r = open(&mut s, "large.ai", &file, params.clone()).unwrap();
        let doc = &s.doc().unwrap().doc;
        assert_eq!(names(&doc.layers), layers, "{params}: {r}");
        assert!(near(doc.artboards[0].rect, [0.0, 0.0, 2000.0, 1000.0]), "{params}: {:?}", doc.artboards[0].rect);
        let art = doc.art_bounds().unwrap();
        assert!(near(art, [200.0, 500.0, 600.0, 800.0]), "{params}: {art:?}");
    }
    let v = s.execute("document.pdfInfo", &json!({"dataBase64": vectorcraft_format::base64_encode(&file)})).unwrap();
    let page = &v["pageInfo"][0];
    assert_eq!((page["width"].as_f64(), page["height"].as_f64()), (Some(2000.0), Some(1000.0)), "{v}");
    assert_eq!(page["boxes"]["media"], json!([0.0, 0.0, 2000.0, 1000.0]), "{v}");
}

/// A `.ai` of a 200 × 100 pt artboard with a 10 mm bleed: its PDF part's page is the artboard
/// (its trim box) grown by the bleed, and a red square reaches 20 pt into the bleed, on the page
/// as in the editing data. `key`: `AIPrivateData`, or `AIPDFPrivateData` for a PDF saved with its
/// editing data.
fn bled_ai(bleed: f64, key: &str) -> Vec<u8> {
    let art = format!("0 1 1 0 1 0 0 Xa\n{}f\n", ai::rect(-20.0, -20.0, 60.0, 50.0));
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &art));
    let (w, h) = (200.0 + 2.0 * bleed, 100.0 + 2.0 * bleed);
    let page = PdfPage {
        bleed: Some([0.0, 0.0, w, h]),
        trim: Some([bleed, bleed, 200.0 + bleed, 100.0 + bleed]),
        entries: format!("/PieceInfo << /Illustrator << /Private << /{key}1 {} 0 R /NumBlock 1 >> >> >>", first_extra(1)),
        ..PdfPage::new(w, h, &format!("1 0 0 rg {0} {0} 80 70 re f", bleed - 20.0))
    };
    pdf_with(&[page], &[&format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())], None)
}

#[test]
fn a_file_with_a_bleed_opens_on_its_artboard_with_the_bleed_round_it() {
    // A `.ai`, and a PDF saved with its editing data (#1345), which has it under other keys.
    for (name, key) in [("bled.ai", "AIPrivateData"), ("bled.pdf", "AIPDFPrivateData")] {
        bled_file_opens_on_its_artboard(name, key);
    }
}

fn bled_file_opens_on_its_artboard(name: &str, key: &str) {
    let bleed = 10.0 * 72.0 / 25.4;
    let file = bled_ai(bleed, key);
    let mut s = Session::new();
    // From the editing data, and from the PDF part alone.
    for params in [json!({}), json!({"editingData": false})] {
        let r = open(&mut s, name, &file, params.clone()).unwrap();
        let doc = &s.doc().unwrap().doc;
        assert_eq!(doc.artboards.len(), 1, "{name} {params}: {r}");
        let ab = doc.artboards[0].rect;
        assert!((ab.width() - 200.0).abs() < 0.01 && (ab.height() - 100.0).abs() < 0.01, "{name} {params}: {ab:?} {r}");
        assert!(doc.setup.bleed.iter().all(|b| (b - bleed).abs() < 0.01), "{name} {params}: {:?}", doc.setup.bleed);
        // The square reaches into the bleed, whole.
        let art = doc.art_bounds().unwrap();
        let want = [ab.x0 - 20.0, ab.y0 + 50.0, ab.x0 + 60.0, ab.y0 + 120.0];
        assert!([art.x0, art.y0, art.x1, art.y1].iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01), "{name} {params}: {art:?}");
        assert!(doc.setup.bleed_rect(ab).inflate(0.01, 0.01).contains_rect(art), "{name} {params}: {art:?}");
    }
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Page 1"], "{name}: editingData: false reads the PDF part");
    open(&mut s, name, &file, json!({})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art"], "{name}: the editing data's layers are read");
}

/// The rows that open in the Layers panel (layers, sublayers and groups), and which of them are
/// open.
fn open_rows(s: &Session) -> (Vec<String>, Vec<String>) {
    let st = s.doc().unwrap();
    let (mut rows, mut open) = (vec![], vec![]);
    st.doc.walk(|n| {
        if n.children().is_some() {
            rows.push(n.name.clone().unwrap_or_default());
            if st.layers_open.contains(n.id) {
                open.push(n.name.clone().unwrap_or_default());
            }
        }
    });
    (rows, open)
}

/// #1475: a `.ai` another app wrote keeps no open rows VectorCraft reads, so it opens with every
/// row closed, its top-level layers as its groups; a `.ai` VectorCraft wrote, and other formats,
/// open as before (with their top-level layers open unless the file says otherwise).
#[test]
fn an_illustrator_ai_file_opens_with_its_layers_closed() {
    let mut s = Session::new();
    for file in [ai::ai(&compressed(&ai::sample_data()), ai::page_pdf()), ai::eps(&ai::sample_data(), ai::page_ps())] {
        let r = open(&mut s, "art.ai", &file, json!({})).unwrap();
        assert_eq!((&r["format"], &r["restored"]), (&json!("ai"), &json!(false)), "{r}");
        let (rows, open) = open_rows(&s);
        assert!(rows.contains(&"Art".into()) && rows.contains(&"Pair".into()) && rows.contains(&"Sub".into()), "{rows:?}");
        assert_eq!(open, Vec::<String>::new(), "{r}");
    }
    // The same art as an EPS: its top-level layers open.
    open(&mut s, "art.eps", &ai::eps(&ai::sample_data(), ai::page_ps()), json!({})).unwrap();
    assert_eq!(open_rows(&s).1, ["Art", "Hidden"]);
    // Saved as .ai by VectorCraft, it opens with the rows it was saved with.
    let saved = s.execute("document.save", &json!({"format": "ai"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    let r = open(&mut s, "again.ai", &bytes, json!({})).unwrap();
    assert_eq!(r["restored"], json!(true), "{r}");
    assert_eq!(open_rows(&s).1, ["Art", "Hidden"]);
}
