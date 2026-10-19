//! The layers of an Illustrator EPS and `.ai`: read from the editing copy of the art the file
//! carries (see `native`). The files here are small ones written by hand for the tests, with the
//! operators seen in files their users own.

use vectorcraft_doc::{Document, LayerColor, Node, NodeKind, Unit};

use crate::import::import;

/// `data` as ASCII85, up to the `~>`.
fn ascii85(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(4) {
        let v = chunk.iter().enumerate().fold(0u32, |v, (i, b)| v | u32::from(*b) << (24 - 8 * i));
        let mut d = [0u8; 5];
        let mut x = v;
        for c in d.iter_mut().rev() {
            *c = (x % 85) as u8 + b'!';
            x /= 85;
        }
        out.extend(d.iter().take(chunk.len() + 1).map(|c| char::from(*c)));
    }
    out.push_str("~>");
    out
}

fn zstd(data: &str) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest)
}

/// The editing copy of a file with `layers` (made by [`layer`]), 100 × 100 points.
fn native(layers: &str) -> String {
    format!(
        "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 100 100\n%%HiResBoundingBox: 0 0 100 100\n%AI3_Cropmarks: 0 0 100 100\n%AI5_NumLayers: 2\n{layers}%%Trailer\n"
    )
}

/// The editing copy of a Large Canvas file (#2031): art and boxes written at 1/`scale`, declared
/// with `%AI24_LargeCanvasScale` as Illustrator writes it. The file's page is `size × scale`
/// points.
fn native_large_canvas(size: f64, scale: f64, layers: &str) -> String {
    format!(
        "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 {size} {size}\n%%HiResBoundingBox: 0 0 {size} {size}\n%AI3_Cropmarks: 0 0 {size} {size}\n%AI24_LargeCanvasScale: {scale}\n%AI5_NumLayers: 1\n{layers}%%Trailer\n"
    )
}

/// A layer as the format writes it.
fn layer(name: &str, visible: bool, body: &str) -> String {
    format!("%AI5_BeginLayer\n{} 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n({name}) Ln\n{body}\nLB\n%AI5_EndLayer--\n", u8::from(visible))
}

/// A yellow square of 4 points at (`x`, `y`) in the format's operators.
fn square(x: u32, y: u32) -> String {
    format!("0 0 1 0 k\n{x} {y} m\n{} {y} L\n{} {} L\n{x} {} L\nf", x + 4, x + 4, y + 4, y + 4)
}

/// An EPS whose page draws `page` and whose private data is `native`.
fn eps(page: &str, native: &str) -> Vec<u8> {
    let stream = ascii85(&zstd(native));
    let lines: Vec<String> = stream.as_bytes().chunks(60).map(|c| format!("%{}", String::from_utf8_lossy(c))).collect();
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{page}\nshowpage\n%%EOF\n%AI9_PrivateDataBegin\n%AI24_DataStream\n{}\n%AI9_PrivateDataEnd\n",
        lines.join("\n")
    )
    .into_bytes()
}

/// The page's drawing of the square at (`x`, `y`).
fn page_square(x: u32, y: u32) -> String {
    format!("0 0 1 0 setcmykcolor {x} {y} moveto {} {y} lineto {} {} lineto {x} {} lineto closepath fill", x + 4, x + 4, y + 4, y + 4)
}

fn names(d: &Document) -> Vec<String> {
    d.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect()
}

#[test]
fn native_import_keeps_ruler_units_from_editing_data() {
    let art = layer("Art", true, &square(10, 10));
    let native = format!(
        "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 100 100\n%%HiResBoundingBox: 0 0 100 100\n%AI3_Cropmarks: 0 0 100 100\n%AI5_RulerUnits: 1\n%AI5_NumLayers: 1\n{art}%%Trailer\n"
    );
    let r = import(&eps(&page_square(10, 10), &native)).unwrap();
    assert_eq!(r.document.units, Unit::Millimeters);
}

fn kinds(l: &Node) -> Vec<&'static str> {
    l.children()
        .into_iter()
        .flatten()
        .map(|n| match &n.kind {
            NodeKind::Group { clip: true, children } => {
                ["clip[0]", "clip[1]", "clip[2]", "clip[3]"].get(children.len()).copied().unwrap_or("clip[many]")
            }
            NodeKind::Group { children, .. } => {
                ["group[0]", "group[1]", "group[2]", "group[3]"].get(children.len()).copied().unwrap_or("group[many]")
            }
            NodeKind::Compound { children, .. } => {
                ["compound[0]", "compound[1]", "compound[2]", "compound[3]"].get(children.len()).copied().unwrap_or("compound[many]")
            }
            NodeKind::Path { clipping: true, .. } => "clippath",
            NodeKind::Path { .. } => "path",
            NodeKind::Text(_) => "text",
            _ => "other",
        })
        .collect()
}

#[test]
fn layers_come_in_with_their_names_colours_and_hidden_state() {
    let art = native(&(layer("Back", true, &square(10, 10)) + &layer("Hidden", false, &square(50, 50))));
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    let d = &r.document;
    assert_eq!(names(d), ["Back", "Hidden"], "{:?}", r.warnings);
    assert_eq!([d.layers[0].visible, d.layers[1].visible], [true, false]);
    // What is on a hidden layer stays: the page doesn't have it.
    assert_eq!(kinds(&d.layers[1]), ["path"]);
    // Colour 0 of the layer colours (light blue), by its place in the list.
    assert!(matches!(&d.layers[0].kind, NodeKind::Layer { color: LayerColor::Preset(0), .. }));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    // The page is where the page's art was.
    let b = d.layers[0].children().unwrap()[0].visual_bounds().unwrap();
    assert!((b.x0 - 10.0).abs() < 1e-6 && (b.y0 - 86.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn sublayers_groups_compounds_clip_groups_and_hidden_objects_come_in() {
    let body = [
        "u".to_string(),
        square(10, 10),
        "U".into(),
        "*u".into(),
        square(20, 10),
        square(30, 10),
        "*U".into(),
        // A clipping group: its objects, then its clip path (filled, so it paints too).
        "q".into(),
        square(40, 10),
        "50 10 m 60 10 L 60 20 L 50 20 L h W f".into(),
        "Q".into(),
        // Hidden until the next `Xw`.
        "1 Xw".into(),
        "u".into(),
        square(70, 10),
        "U".into(),
        "0 Xw".into(),
    ]
    .join("\n");
    let sub = layer("Inner", true, &square(80, 10));
    let art = native(&layer("Outer", true, &format!("{body}\n{sub}")));
    // The page has what shows: the clip group's square is outside its clip path, which paints.
    let clip_fill = "0 0 1 0 setcmykcolor 50 10 moveto 60 10 lineto 60 20 lineto 50 20 lineto closepath fill";
    let page = [10, 20, 30, 80].map(|x| page_square(x, 10)).join("\n") + "\n" + clip_fill;
    let r = import(&eps(&page, &art)).unwrap();
    let outer = &r.document.layers[0];
    assert_eq!(kinds(outer), ["group[1]", "compound[2]", "clip[3]", "group[1]", "other"]);
    let children = outer.children().unwrap();
    assert!(children[0].visible && !children[3].visible, "the hidden object is hidden, not left out");
    // The clip path comes first in its group.
    assert!(matches!(children[2].children().unwrap()[0].kind, NodeKind::Path { clipping: true, .. }));
    // The sublayer is a layer of the layer.
    assert!(matches!(&children[4].kind, NodeKind::Layer { .. }));
    assert_eq!(children[4].name.as_deref(), Some("Inner"));
}

#[test]
fn what_the_layers_cant_say_keeps_the_page_when_it_shows_and_not_when_it_doesnt() {
    // An operator that isn't read (`Zq`) on a layer that shows: the layers would draw without it.
    let shown = native(&layer("Art", true, &format!("{}\n0 0 0 1 0 0 Zq", square(10, 10))));
    let r = import(&eps(&page_square(10, 10), &shown)).unwrap();
    assert_eq!(names(&r.document), ["Layer 1"]);
    assert!(r.warnings.iter().any(|w| w.contains("layers weren't read") && w.contains("`Zq`")), "{:?}", r.warnings);
    // On a hidden layer it only costs that layer's look.
    let hidden = native(&(layer("Art", true, &square(10, 10)) + &layer("Old", false, "0 0 0 1 0 0 Zq")));
    let r = import(&eps(&page_square(10, 10), &hidden)).unwrap();
    assert_eq!(names(&r.document), ["Art", "Old"]);
    assert!(r.warnings.iter().any(|w| w.contains("hidden layers have art this can't read")), "{:?}", r.warnings);
}

#[test]
fn text_and_its_strokes_come_from_the_page_into_the_layer_of_its_text_object() {
    let page = format!(
        "{}\n/Helvetica findfont 12 scalefont setfont 10 50 moveto (Hi) show\n10 50 moveto (Hi) false charpath 2 setlinewidth stroke",
        page_square(10, 10)
    );
    let words = layer("Words", true, "/AI11Text :\n0 /FreeUndo ,\n;\n\n0 0 Xd");
    let art = native(&(layer("Art", true, &square(10, 10)) + &words));
    let r = import(&eps(&page, &art)).unwrap();
    assert_eq!(names(&r.document), ["Art", "Words"], "{:?}", r.warnings);
    assert_eq!(kinds(&r.document.layers[0]), ["path"]);
    // The text and the path of its stroke, where the text object is (no group of their own).
    assert_eq!(kinds(&r.document.layers[1]), ["text", "path"], "{:?}", r.warnings);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn a_damaged_editing_copy_leaves_the_page() {
    // Not ASCII85, ASCII85 of something that isn't Zstandard, Zstandard of something that isn't art.
    let plain = |data: &str| {
        let lines = format!("%{}", data);
        format!(
            "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{}\nshowpage\n%%EOF\n%AI9_PrivateDataBegin\n%AI24_DataStream\n{lines}\n%AI9_PrivateDataEnd\n",
            page_square(10, 10)
        )
        .into_bytes()
    };
    for bytes in [plain("this is not ascii85 {{{~>"), plain(&ascii85(b"not zstandard at all")), eps(&page_square(10, 10), "nothing of the art here")]
    {
        let r = import(&bytes).unwrap();
        assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
    }
    // A file that has the data but not in a form that can be read says so (and a write over it asks).
    let r = import(&plain("this is not ascii85 {{{~>")).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("damaged") && crate::is_loss(w)), "{:?}", r.warnings);
}

/// The `.ai`'s editing data: its marker, then the Zstandard stream.
fn private(native: &str) -> Vec<u8> {
    [b"%AI24_ZStandard_Data".as_slice(), &zstd(native)].concat()
}

/// A page of 100 × 100 with a square at (`x`, `y`) in one layer, as a PDF part would give.
fn pdf_part(x: f64, y: f64) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let id = d.alloc_id();
    let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, y, x + 4.0, y + 4.0));
    let paint = vectorcraft_color::Paint::solid(vectorcraft_color::Color::cmyk(0.0, 0.0, 1.0, 0.0));
    let item = vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(paint));
    let n = Node::path(id, path, vectorcraft_doc::Appearance { items: vec![item], ..Default::default() });
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// The PDF part `d` as read in points from pages of `k` points a unit (their `UserUnit`): its
/// pages and art `k` times as large.
fn in_points(mut d: Document, k: f64) -> Document {
    for layer in &mut d.layers {
        std::sync::Arc::make_mut(layer).transform(vectorcraft_geom::Affine::scale(k), true);
    }
    for a in &mut d.artboards {
        a.rect = a.rect.scale_from_origin(k);
    }
    d
}

#[test]
fn an_ai_file_gets_its_layers_and_the_art_outside_its_artboard() {
    // A square on the artboard, and one above it (the artboard is y 0 to 100 here).
    let art = native(&(layer("Jig", true, &square(10, 10)) + &layer("Parts", true, &square(30, 110)) + &layer("Old", false, &square(60, 60))));
    let (d, warnings, read) = crate::layered_ai(&private(&art), pdf_part(10.0, 86.0), vec!["a note".into()], false, false, 1.0);
    assert_eq!(names(&d), ["Jig", "Parts", "Old"], "{warnings:?}");
    assert!(read);
    assert_eq!(warnings, ["a note"], "nothing else to say");
    let above = d.layers[1].children().unwrap()[0].visual_bounds().unwrap();
    assert!(above.y1 <= 0.0 + 1e-6, "the art above the artboard is kept, outside it: {above:?}");
    assert!(!d.layers[2].visible);
}

/// A `.ai` whose PDF part was read in points from a page of 10 points a unit (its `UserUnit`), and
/// whose editing data has the page in those units: its layers come in at the page's size, ten
/// times as large as the editing data has them, with stroke weights and dashes ten times as large.
/// Editing data with the page in points comes in as it is.
#[test]
fn editing_data_in_the_units_of_a_user_unit_page_comes_in_at_the_pages_size() {
    // A dashed line 2 units wide on a hidden layer, which the page doesn't draw.
    let line = "0 0 1 0 K\n2 w\n[4 2] 1 d\n10 50 m\n50 50 L\nS";
    let art = native(&(layer("Jig", true, &square(10, 10)) + &layer("Parts", true, &square(30, 110)) + &layer("Old", false, line)));
    let page = in_points(pdf_part(10.0, 86.0), 10.0);
    let first = |d: &Document, l: usize| d.layers[l].children().unwrap()[0].visual_bounds().unwrap();
    let near = |r: vectorcraft_geom::Rect, want: [f64; 4]| [r.x0, r.y0, r.x1, r.y1].iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-6);
    let (d, w, read) = crate::layered_ai(&private(&art), page.clone(), vec![], false, false, 10.0);
    assert!(read, "{w:?}");
    assert_eq!(names(&d), ["Jig", "Parts", "Old"]);
    assert_eq!(d.artboards[0].rect, vectorcraft_geom::Rect::new(0.0, 0.0, 1000.0, 1000.0));
    // The square on the artboard and the one above it, ten times as large and as far from its corner.
    assert!(near(first(&d, 0), [100.0, 860.0, 140.0, 900.0]), "{:?}", first(&d, 0));
    assert!(near(first(&d, 1), [300.0, -140.0, 340.0, -100.0]), "{:?}", first(&d, 1));
    let stroke = d.layers[2].children().unwrap()[0].appearance.stroke().cloned().unwrap();
    let dash = stroke.dash.unwrap();
    assert_eq!((stroke.width, dash.pattern, dash.offset), (20.0, vec![40.0, 20.0], 10.0));
    // Read as if its page had no `UserUnit`, the editing data is for another page.
    let (_, w, read) = crate::layered_ai(&private(&art), page.clone(), vec![], false, false, 1.0);
    assert!(!read && w.iter().any(|w| w.contains("for a 100 × 100 pt page, the file's 1000 × 1000 pt page at 1:10")), "{w:?}");
    // Editing data with the page in points.
    let points = native(&layer("Jig", true, "0 0 1 0 k\n100 100 m\n140 100 L\n140 140 L\n100 140 L\nf")).replace(" 0 0 100 100", " 0 0 1000 1000");
    let (d, w, read) = crate::layered_ai(&private(&points), page, vec![], false, false, 10.0);
    assert!(read, "{w:?}");
    assert_eq!(d.artboards[0].rect, vectorcraft_geom::Rect::new(0.0, 0.0, 1000.0, 1000.0));
    assert!(near(first(&d, 0), [100.0, 860.0, 140.0, 900.0]), "{:?}", first(&d, 0));
}

#[test]
fn an_ai_file_that_cant_be_read_through_its_editing_data_keeps_its_pdf_part() {
    let pdf = pdf_part(10.0, 86.0);
    // No editing data this reads.
    let (d, w, read) = crate::layered_ai(b"nothing", pdf.clone(), vec![], false, false, 1.0);
    assert_eq!(d.layers.len(), pdf.layers.len());
    assert!(w.is_empty() && !read);
    // Editing data that isn't for this page.
    let other = native(&layer("Jig", true, &square(10, 10))).replace("%AI3_Cropmarks: 0 0 100 100", "%AI3_Cropmarks: 0 0 50 50");
    let (d, w, _) = crate::layered_ai(&private(&other), pdf.clone(), vec![], false, false, 1.0);
    assert_eq!(names(&d), names(&pdf));
    assert!(w.iter().any(|w| w.contains("for a 50 × 50 pt page, the file's 100 × 100 pt page at 1:2")), "{w:?}");
}

/// #2034: editing data for a page of another size gives both sizes in points, in an EPS's note and
/// in a `.ai`'s.
#[test]
fn editing_data_for_a_page_of_another_size_names_both_sizes() {
    let art = native(&layer("Jig", true, &square(10, 10))).replace(" 0 0 100 100", " 0 0 600 400");
    let file = String::from_utf8(eps(&page_square(10, 10), &art)).unwrap().replace("%%BoundingBox: 0 0 100 100", "%%BoundingBox: 0 0 612 792");
    let r = import(file.as_bytes()).unwrap();
    assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
    let note = "the file's layers weren't read (its editing data is for a 600 × 400 pt page, not the file's 612 × 792 pt page): it comes in as its page, in one layer";
    assert!(r.warnings.iter().any(|w| w == note), "{:?}", r.warnings);
    // A `.ai` whose PDF part has an A4 page.
    let (_, w, read) = crate::layered_ai(&private(&art), Document::new(595.2756, 841.8898), vec![], false, false, 1.0);
    let note = "the file's layers weren't read from its editing data (its editing data is for a 600 × 400 pt page, not the file's 595.276 × 841.89 pt page): they come from its PDF part";
    assert!(!read && w.iter().any(|w| w == note), "{w:?}");
}

/// #2034: editing data whose page is the file's at another scale (a Large Canvas file without its
/// scale, for one) gives that scale, the editing data's page to the file's.
#[test]
fn editing_data_for_the_page_at_another_scale_names_the_scale() {
    let art = native(&layer("Jig", true, &square(10, 10)));
    let note = |file: &str| {
        let r = import(file.as_bytes()).unwrap();
        assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
        r.warnings.into_iter().find(|w| w.contains("its editing data is for")).unwrap_or_default()
    };
    let file = String::from_utf8(eps(&page_square(10, 10), &art)).unwrap();
    let ten = file.replace("%%BoundingBox: 0 0 100 100", "%%BoundingBox: 0 0 1000 1000");
    assert_eq!(
        note(&ten),
        "the file's layers weren't read (its editing data is for a 100 × 100 pt page, the file's 1000 × 1000 pt page at 1:10): it comes in as its page, in one layer"
    );
    let wide = file.replace("%%BoundingBox: 0 0 100 100", "%%BoundingBox: 0 0 250 250");
    assert!(note(&wide).contains("for a 100 × 100 pt page, the file's 250 × 250 pt page at 1:2.5)"));
    let half = file.replace("%%BoundingBox: 0 0 100 100", "%%BoundingBox: 0 0 50 50");
    assert!(note(&half).contains("for a 100 × 100 pt page, the file's 50 × 50 pt page at 2:1)"));
    // A scale that rounds to 1:1 isn't given.
    let large = art.replace(" 0 0 100 100", " 0 0 2000 2000");
    let (_, w, _) = crate::layered_ai(&private(&large), Document::new(2000.8, 2000.8), vec![], false, false, 1.0);
    assert!(w.iter().any(|w| w.contains("for a 2000 × 2000 pt page, not the file's 2000.8 × 2000.8 pt page)")), "{w:?}");
}

/// Editing data that can't be decompressed leaves the `.ai` as its PDF part, after its notes, with a
/// note saying why its layers weren't read, which counts as a loss.
#[test]
fn an_ai_file_whose_editing_data_cant_be_decompressed_says_so() {
    let pdf = pdf_part(10.0, 86.0);
    for (private, why) in [
        (&b"%AI12_CompressedDataxx"[..], "its zlib compression is damaged"),
        (b"%AI24_ZStandard_Data nothing", "its Zstandard compression is damaged"),
    ] {
        let (d, w, read) = crate::layered_ai(private, pdf.clone(), vec!["a note".into()], false, false, 1.0);
        assert!(!read);
        assert_eq!(names(&d), names(&pdf));
        assert_eq!(w, ["a note".to_string(), format!("the file's layers weren't read from its editing data ({why}): they come from its PDF part")]);
        assert!(crate::is_loss(&w[1]), "{w:?}");
    }
}

/// #2031: a Large Canvas editing copy (`%AI24_LargeCanvasScale: 10`) writes its art and boxes at
/// 1/10 scale; the import scales them back, so its layers are read instead of being rejected as
/// another page's — with the art at the page's size. Art that doesn't match the page at that
/// scale is still another page's.
#[test]
fn a_large_canvas_eps_keeps_its_layers_at_the_pages_size() {
    // 10 art units for the page's 100 points: a 0.4-unit square at (5, 5) is the page's 4-point
    // square at (50, 50).
    let art = native_large_canvas(10.0, 10.0, &layer("Art", true, "0 0 1 0 k\n5 5 m\n5.4 5 L\n5.4 5.4 L\n5 5.4 L\nf"));
    let r = import(&eps(&page_square(50, 50), &art)).unwrap();
    let d = &r.document;
    assert_eq!(names(d), ["Art"], "{:?}", r.warnings);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let b = d.layers[0].children().unwrap()[0].visual_bounds().unwrap();
    assert!((b.x0 - 50.0).abs() < 1e-6 && (b.y0 - 46.0).abs() < 1e-6, "at the page's place: {b:?}");
    assert!((b.width() - 4.0).abs() < 1e-6 && (b.height() - 4.0).abs() < 1e-6, "at the page's size: {b:?}");
    // A declared scale the art doesn't fit still makes it another page's.
    let art = native_large_canvas(20.0, 10.0, &layer("Art", true, &square(10, 10)));
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("for a 200 × 200 pt page, the file's 100 × 100 pt page at 2:1")), "{:?}", r.warnings);
    // A `.ai` keeps its art space: its PDF part is read at the size it declares.
    let art = native_large_canvas(100.0, 10.0, &layer("Art", true, &square(10, 10)));
    let (d, w, read) = crate::layered_ai(&private(&art), pdf_part(10.0, 86.0), vec![], false, false, 1.0);
    assert!(read && names(&d) == ["Art"], "{w:?}");
}

/// A `.ai` whose layers can't be read comes in as its PDF part, a page per artboard in a row; its
/// pages still go where its editing data puts its artboards, with their art, and keep their names (#1068).
#[test]
fn an_ai_file_read_as_its_pdf_part_keeps_its_artboard_layout() {
    // Two 100-point artboards, the second below the first (art space, y up).
    let board = |x0: i32, y0: i32| {
        format!(
            "%_/Dictionary :\n%_{x0} {} /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_{} {y0} /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_(at {x0} {y0}) /UnicodeString (Name) ,\n%_; ,\n",
            y0 + 100,
            x0 + 100
        )
    };
    let boards = |list: &[(i32, i32)]| {
        let all: String = list.iter().map(|(x, y)| board(*x, *y)).collect();
        format!("%_/Document :\n%_/Dictionary :\n%_/Array :\n{all}%_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n")
    };
    // An operator this doesn't know, so the layers aren't read.
    let unread = |list: &[(i32, i32)]| native(&(boards(list) + &layer("Art", true, &(square(10, 10) + "\n1 Zz"))));
    // The PDF part: the pages side by side, 20 points apart, a square on each.
    let mut pdf = pdf_part(10.0, 10.0);
    let second = pdf_part(130.0, 10.0).layers[0].children().unwrap()[0].clone();
    let mut second = (*second).clone();
    second.id = pdf.alloc_id();
    let layer_id = pdf.layers[0].id;
    pdf.insert(Some(layer_id), 1, second).unwrap();
    let mut page = pdf.artboards[0].clone();
    (page.id, page.rect) = (2, vectorcraft_geom::Rect::new(120.0, 0.0, 220.0, 100.0));
    pdf.artboards.push(page);
    let squares = |d: &Document| d.layers[0].children().unwrap().iter().map(|n| n.visual_bounds().unwrap().origin()).collect::<Vec<_>>();
    let (d, w, read) = crate::layered_ai(&private(&unread(&[(0, 0), (0, -150)])), pdf.clone(), vec![], false, false, 1.0);
    assert!(!read && w.iter().any(|w| w.contains("layers weren't read")), "{w:?}");
    let rects: Vec<_> = d.artboards.iter().map(|a| a.rect).collect();
    assert_eq!(rects, [vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0), vectorcraft_geom::Rect::new(0.0, 150.0, 100.0, 250.0)]);
    let board_names: Vec<_> = d.artboards.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(board_names, ["at 0 0", "at 0 -150"]);
    let moved = squares(&d);
    assert!((moved[0] - squares(&pdf)[0]).hypot() < 1e-6, "the first page's art stays: {moved:?}");
    assert!((moved[1] - vectorcraft_geom::Point::new(10.0, 160.0)).hypot() < 1e-6, "the second's goes with its page: {moved:?}");
    // Artboards that don't match the pages leave them in a row.
    for list in [&[(0, 0)][..], &[(0, 0), (0, -150), (200, 0)]] {
        let (d, _, _) = crate::layered_ai(&private(&unread(list)), pdf.clone(), vec![], false, false, 1.0);
        assert_eq!(d.artboards[1].rect, pdf.artboards[1].rect, "{list:?}");
        assert_eq!(d.artboards[1].name, pdf.artboards[1].name, "{list:?}");
    }
    // Pages read in points from a user space of 10 points a unit, with artboards in those units:
    // the same layout, ten times as large.
    let (d, _, read) = crate::layered_ai(&private(&unread(&[(0, 0), (0, -150)])), in_points(pdf.clone(), 10.0), vec![], false, false, 10.0);
    assert!(!read);
    let rects: Vec<_> = d.artboards.iter().map(|a| a.rect).collect();
    assert_eq!(rects, [vectorcraft_geom::Rect::new(0.0, 0.0, 1000.0, 1000.0), vectorcraft_geom::Rect::new(0.0, 1500.0, 1000.0, 2500.0)]);
    let moved = squares(&d);
    assert!((moved[1] - vectorcraft_geom::Point::new(100.0, 1600.0)).hypot() < 1e-6, "the second's art goes with its page: {moved:?}");
}

#[test]
fn a_gray_is_a_grayscale_colour() {
    // `g` is a gray: 0.25 is 75 % of the black ink, as the page's own CMYK black says; the colour
    // stays in its grayscale model.
    let art = native(&layer("Art", true, "0.25 g\n10 10 m\n14 10 L\n14 14 L\n10 14 L\nf"));
    let page = "0 0 0 0.75 setcmykcolor 10 10 moveto 14 10 lineto 14 14 lineto 10 14 lineto closepath fill";
    let r = import(&eps(page, &art)).unwrap();
    assert_eq!(names(&r.document), ["Art"], "{:?}", r.warnings);
    let fill = r.document.layers[0].children().unwrap()[0].appearance.fill().and_then(|f| f.paint.color());
    assert_eq!(fill, Some(vectorcraft_color::Color::gray(0.75)));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

/// `native` as a CS-era file keeps it: zlib, under `%AI9_DataStream`.
fn older_eps(page: &str, native: &str) -> Vec<u8> {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::default());
    z.write_all(native.as_bytes()).unwrap();
    let stream = ascii85(&z.finish().unwrap());
    let lines: Vec<String> = stream.as_bytes().chunks(60).map(|c| format!("%{}", String::from_utf8_lossy(c))).collect();
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{page}\nshowpage\n%%EOF\n%AI9_PrivateDataBegin\n%AI9_DataStream\n{}\n%AI9_PrivateDataEnd\n",
        lines.join("\n")
    )
    .into_bytes()
}

#[test]
fn files_from_before_zstandard_give_their_layers_too() {
    let art = native(&(layer("Back", true, &square(10, 10)) + &layer("Hidden", false, &square(50, 50))));
    let r = import(&older_eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Back", "Hidden"], "{:?}", r.warnings);
    assert_eq!([r.document.layers[0].visible, r.document.layers[1].visible], [true, false]);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn custom_colours_open_paths_and_planar_group_marks_are_read() {
    // `x` and `X` are the custom colours of a fill and a stroke (four inks, a name, a tint), `Xx` one
    // with its Lab values; `H` ends an open path, `XP` marks a group the app made from a planar map.
    let body = "0.1 0.2 0.3 0.4 (Spot) 0 x\n0 0 0 1 (Edge) 0 X\n10 10 m\n14 10 L\n14 14 L\n10 14 L\nf\n\
                0.8 0.1 0 0 62.7 -28 -42 (Blue) 0 2 Xx\n(Adobe Planar Group) 1 0 0 XP\n20 20 m\n24 20 L\n24 24 L\nH\nF";
    let r = import(&older_eps(
        "0.1 0.2 0.3 0.4 setcmykcolor 10 10 moveto 14 10 lineto 14 14 lineto 10 14 lineto closepath fill          0.8 0.1 0 0 setcmykcolor 20 20 moveto 24 20 lineto 24 24 lineto closepath fill",
        &native(&layer("Art", true, body)),
    ))
    .unwrap();
    assert_eq!(names(&r.document), ["Art"], "{:?}", r.warnings);
    assert_eq!(kinds(&r.document.layers[0]), ["path", "path"]);
}

/// A text document of `stories` (each as the text, its alignment code, its anchor and the frame's
/// translation, and the start of its first line from the anchor), one font and a 14 point default.
/// A story: its text, alignment code, anchor, frame translation and first line's start.
type Story<'a> = (&'a str, u8, (f64, f64), (f64, f64), f64);

fn text_document(stories: &[Story<'_>]) -> String {
    let story = |(text, align, anchor, _t, start): &Story<'_>| {
        let n = text.chars().count();
        format!(
            "<< /0 << /0 ({text}) /5 << /0 [ << /0 << /0 << /0 () /5 << /0 {align} >> /6 0 >> >> /1 {n} >> ] >> \
             /6 << /0 [ << /0 << /0 << /0 () /5 0 /6 << /0 0 /1 14.0 /53 << /99 /CAITextPaint /0 << /0 2 /1 [ 1.0 0.0 1.0 0.0 0.0 ] >> >> >> >> >> /1 {n} >> ] >> >> \
             /1 << /0 [ << /0 0 >> ] /2 [ << /99 /PC /6 [ << /99 /F /0 << /0 [ {} {} ] >> /6 [ << /99 /R /6 [ << /99 /R /6 [ << /99 /L /6 [ \
             << /99 /S /0 << /0 [ {start} 0.0 ] >> /15 << /0 {n} >> >> ] >> ] >> ] >> ] >> ] >> ] >> >>",
            anchor.0, anchor.1
        )
    };
    let frame =
        |(_, _, _, t, _): &(&str, u8, (f64, f64), (f64, f64), f64)| format!("<< /0 << /0 [ 0 0 ] /2 << /2 [ 1 0 0 1 {} {} ] >> >> >>", t.0, t.1);
    format!(
        "/0 << /1 << /0 [ << /0 << /0 << /0 (Helvetica) >> >> >> ] >> /8 << /0 [ {} ] >> >>\n/1 << /1 [ {} ] /2 << /1 12.0 >> >>\n",
        stories.iter().map(frame).collect::<Vec<_>>().join(" "),
        stories.iter().map(story).collect::<Vec<_>>().join(" ")
    )
}

/// The editing copy of a 100 × 100 file with `layers` and the text `document`; its template box is
/// centred at (50, 50).
fn native_with_text(layers: &str, document: &str) -> String {
    let lines: Vec<String> = ascii85(document.as_bytes()).as_bytes().chunks(60).map(|c| format!("%{}", String::from_utf8_lossy(c))).collect();
    format!(
        "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 100 100\n%%HiResBoundingBox: 0 0 100 100\n%AI3_Cropmarks: 0 0 100 100\n%AI3_TemplateBox: 50 50 50 50\n\
         %AI5_NumLayers: 2\n{layers}%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n{}\n%AI11_EndTextDocument\n%%Trailer\n",
        lines.join("\n")
    )
}

/// A text object of story `story`, as the layers hold it.
fn text_object(story: u32) -> String {
    format!("/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n{story} /StoryIndex ,\n2 /TextAntialiasing ,\n;\n")
}

fn text_of(l: &Node) -> Vec<&vectorcraft_doc::TextObject> {
    l.children().into_iter().flatten().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(&**t) } else { None }).collect()
}

#[test]
fn type_on_a_hidden_layer_comes_from_the_text_document() {
    // Canvas centre 8191.5 is the template box's centre (50, 50): the anchor is 8.5 right of it and
    // 11.5 above it, so at (58.5, 61.5) in the art, which is 38.5 down the page.
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0), ("Mid\r", 2, (8191.5, 8191.5), (0.0, 0.0), -10.0)]);
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, &(text_object(0) + &text_object(1)))), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Back", "Spare"], "{:?}", r.warnings);
    let spare = &r.document.layers[1];
    assert!(!spare.visible);
    let texts = text_of(spare);
    assert_eq!(texts.len(), 2, "{:?}", r.warnings);
    let [a, b] = [texts[0], texts[1]];
    assert_eq!(a.plain_text(), "Hi there");
    let [.., e, f] = a.xf.as_coeffs();
    assert!((e - 58.5).abs() < 1e-6 && (f - 38.5).abs() < 1e-6, "{:?}", a.xf);
    assert_eq!(a.runs[0].style.size, 14.0);
    assert_eq!(a.para.justify, vectorcraft_doc::Justify::Left);
    // Centred type is anchored at its middle.
    assert_eq!(b.para.justify, vectorcraft_doc::Justify::Center);
    let [.., e, f] = b.xf.as_coeffs();
    assert!((e - 50.0).abs() < 1e-6 && (f - 50.0).abs() < 1e-6, "{:?}", b.xf);
    assert!(!r.warnings.iter().any(|w| w.contains("couldn't be read")), "{:?}", r.warnings);
}

#[test]
fn type_with_a_frame_it_cant_read_is_left_out_with_a_note() {
    // A story whose frame is of a kind this doesn't know isn't guessed at.
    let mut doc = text_document(&[("Area\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    doc = doc.replace("/2 << /2 [ 1 0 0 1 0 0 ] >>", "/2 << /0 9 /2 [ 1 0 0 1 0 0 ] >>");
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, &text_object(0))), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert!(text_of(&r.document.layers[1]).is_empty());
    assert!(r.warnings.iter().any(|w| w.contains("1 text objects on hidden layers")), "{:?}", r.warnings);
    assert!(crate::is_loss(r.warnings.iter().find(|w| w.contains("couldn't be read")).unwrap()));
}

#[test]
fn shown_type_keeps_the_pages_and_type_off_the_page_comes_from_the_text_document() {
    // Story 0 is where the page draws "Hi there"; story 1 is off the page, so the page has nothing of it.
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0), ("Away\r", 0, (8300.0, 8180.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &(text_object(0) + &text_object(1))), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (Hi there) show";
    let r = import(&eps(page, &art)).unwrap();
    assert_eq!(names(&r.document), ["Art"], "{:?}", r.warnings);
    let texts = text_of(&r.document.layers[0]);
    // The page's own object, and the other made from the file.
    let all: Vec<String> = {
        let mut v = vec![];
        r.document.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                v.push(t.plain_text());
            }
        });
        v
    };
    assert_eq!(all.len(), 2, "{all:?} {:?}", r.warnings);
    assert!(all.contains(&"Hi there".to_string()) && all.contains(&"Away".to_string()), "{all:?}");
    assert_eq!(texts.len(), 2, "the page's type is in the layer as it is, not in a group");
    assert!(!r.warnings.iter().any(|w| w.contains("order the page paints")), "{:?}", r.warnings);
}

#[test]
fn a_text_object_written_as_comments_is_read_like_any_other() {
    let doc = text_document(&[("Hi\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let commented: String = text_object(0).lines().map(|l| format!("%_{l}\n")).collect();
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, &commented)), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    let texts = text_of(&r.document.layers[1]);
    assert_eq!(texts.len(), 1, "{:?}", r.warnings);
    assert_eq!(texts[0].plain_text(), "Hi");
}

#[test]
fn an_area_type_object_with_its_own_nested_art_doesnt_swallow_the_art_after_it() {
    let area = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n3 /StoryIndex ,\n/Art :\n1 Ap\n0 0 m\n9 0 L\n9 9 L\nn\n; /ConfiningPath ,\n2 /TextAntialiasing ,\n;\n";
    let art = native(&layer("Art", true, &(square(10, 10) + "\n" + area + &square(30, 30))));
    let r = import(&eps(&(page_square(10, 10) + "\n" + &page_square(30, 30)), &art)).unwrap();
    let paths = r.document.layers[0].children().map_or(0, Vec::len);
    assert!(paths >= 2, "{paths} {:?}", r.warnings);
}

#[test]
fn a_line_the_page_draws_in_pieces_goes_to_its_text_object_whole() {
    // The page sets "Hi there" as "Hi" and " there" (as when kerning changes); the story is one line.
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &text_object(0)), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (Hi) show 70 61.5 moveto (there) show";
    let r = import(&eps(page, &art)).unwrap();
    // One text, with the line's space between the pieces.
    let art = &r.document.layers[0];
    assert_eq!(kinds(art), ["text"], "{:?}", r.warnings);
    let texts: Vec<String> = text_of(art).iter().map(|t| t.plain_text()).collect();
    assert_eq!(texts, ["Hi there"]);
    // Its bounds are laid out afresh for the whole text, not those of the first piece.
    let NodeKind::Text(t) = &art.children().unwrap()[0].kind else { panic!("text") };
    assert!(t.cached_bounds.is_none() && t.cached_baselines.is_empty());
}

#[test]
fn type_of_the_page_that_no_story_places_stays_with_the_nearest_text_object() {
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &text_object(0)), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (Hi there) show 58.5 30 moveto (Extra) show";
    let r = import(&eps(page, &art)).unwrap();
    let mut all = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            all.push(t.plain_text());
        }
    });
    assert!(all.contains(&"Extra".to_string()), "{all:?} {:?}", r.warnings);
}

/// Page type of no text object that lies where made type is, but has other letters, is not that type:
/// it stays, with the nearest text object.
#[test]
fn type_of_the_page_near_made_type_with_other_letters_stays() {
    // "Extra" is 11.5 below the line of "Hi there", well inside the box round that line.
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &text_object(0)), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (Hi there) show 58.5 50 moveto (Extra) show";
    let r = import(&eps(page, &art)).unwrap();
    let mut all = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            all.push(t.plain_text());
        }
    });
    assert_eq!(all, ["Hi there", "Extra"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("with another's")), "{:?}", r.warnings);
}

/// Page type of no text object that lies where made type is and has its letters, in any case, is that
/// type as the page draws it: the made type stays, and the page's is left out.
#[test]
fn type_of_the_page_with_the_letters_of_made_type_where_it_lies_is_that_type() {
    // The page draws the line 3 points below where the story starts it, so no line of the story
    // owns it.
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &text_object(0)), &doc);
    for drawn in ["Hi there", "HI THERE", "there"] {
        let page = format!("0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 58.5 moveto ({drawn}) show");
        let r = import(&eps(&page, &art)).unwrap();
        let mut all = vec![];
        r.document.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                all.push((t.plain_text(), t.xf.as_coeffs()[5]));
            }
        });
        assert_eq!(all, [("Hi there".to_string(), 38.5)], "{drawn}: {:?}", r.warnings);
        assert!(!r.warnings.iter().any(|w| w.contains("doesn't draw") || w.contains("with another's")), "{drawn}: {:?}", r.warnings);
    }
}

/// Page type of no text object that lies where the made type of two text objects is, with the
/// letters of one of them in each of its pieces, is the type of both as the page draws it.
#[test]
fn type_of_the_page_with_the_letters_of_two_made_types_where_it_lies_is_their_type() {
    // The two stories start 16.5 apart on one line, and the page draws both 3 points below it: its
    // two pieces are one object, which no line of either story owns.
    let doc = text_document(&[("Hi\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0), ("there\r", 0, (8216.5, 8180.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &(text_object(0) + &text_object(1))), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 58.5 moveto (Hi) show 75 58.5 moveto (there) show";
    let r = import(&eps(page, &art)).unwrap();
    let mut all = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            all.push((t.plain_text(), t.xf.as_coeffs()[5]));
        }
    });
    assert_eq!(all, [("Hi".to_string(), 38.5), ("there".to_string(), 38.5)], "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("doesn't draw") || w.contains("with another's")), "{:?}", r.warnings);
}

/// The file's text document is trusted where the page has type where it lays type out, whatever
/// letters that type has: its type is weighed against the page.
#[test]
fn page_type_where_made_type_is_trusts_the_text_document_whatever_its_letters() {
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &text_object(0)), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 50 moveto (Extra) show";
    let r = import(&eps(page, &art)).unwrap();
    let mut all = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            all.push(t.plain_text());
        }
    });
    assert_eq!(all, ["Extra"], "{:?}", r.warnings);
    // "Hi there" was made from the text document and left out because the page doesn't draw it.
    assert!(r.warnings.iter().any(|w| w.contains("doesn't draw")), "{:?}", r.warnings);
}

#[test]
fn type_the_page_draws_of_a_text_object_on_a_hidden_layer_stays_hidden() {
    // A file's page may draw what its layers have hidden since: story 1's type is on the page at
    // (58.5, 30), and its text object is on a layer that doesn't show.
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0), ("Extra\r", 0, (8200.0, 8211.5), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&(layer("Art", true, &text_object(0)) + &layer("Spare", false, &text_object(1))), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (Hi there) show 58.5 30 moveto (Extra) show";
    let r = import(&eps(page, &art)).unwrap();
    assert_eq!(names(&r.document), ["Art", "Spare"], "{:?}", r.warnings);
    let texts = |l: usize| text_of(&r.document.layers[l]).iter().map(|t| t.plain_text()).collect::<Vec<_>>();
    assert_eq!((texts(0), texts(1)), (vec!["Hi there".to_string()], vec!["Extra".to_string()]), "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("with another's")), "{:?}", r.warnings);
}

/// The opacity and blend mode of a text object (an `Xy` before it, as a path has, or after it, as a
/// group has, or on the look of one with an appearance) are its type's, made from the text document
/// or taken from the page, and are applied once where the type comes with other art.
#[test]
fn type_keeps_the_opacity_and_blend_mode_of_its_text_object() {
    use vectorcraft_color::BlendMode;
    use vectorcraft_doc::Knockout;
    let states = |d: &Document| {
        let mut v = vec![];
        d.walk(|n| {
            if let NodeKind::Text(_) = &n.kind {
                v.push((n.opacity, n.blend));
            }
        });
        v
    };
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let look = "0 Ae\nu\n/AI11Text :\n1 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\nU\n1 0.2 0 1 0 Xy\n0 0 Xd\n6 () XW\n";
    let own: String = text_object(0).lines().map(|l| format!("%_{l}\n")).collect();
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (Hi there) show";
    for body in [
        format!("1 0.2 0 0 0 Xy\n{}", text_object(0)),
        format!("{}1 0.2 0 0 0 Xy\n", text_object(0)),
        format!("{look}%_0 1 0 0 0 Xy\n{own}1 (Style) XW\n"),
    ] {
        let art = native_with_text(&layer("Art", true, &body), &doc);
        // Made from the text document.
        let (d, notes) = crate::ai_alone(&private(&art)).unwrap();
        assert_eq!(states(&d), [(0.2, BlendMode::Multiply)], "{body}{notes:?}");
        // From an EPS page, which draws it without transparency.
        let r = import(&eps(page, &art)).unwrap();
        assert_eq!(names(&r.document), ["Art"], "{body}{:?}", r.warnings);
        assert_eq!(states(&r.document), [(0.2, BlendMode::Multiply)], "{body}{:?}", r.warnings);
    }
    // A look with transparency that draws more than the type: the look and the type are in a group
    // that has the transparency once, whatever state the object is read in.
    let drawing = look.replace("U\n", &format!("{}\nU\n", square(10, 10)));
    for reset in ["%_0 1 0 0 0 Xy\n", ""] {
        let art = native_with_text(&layer("Art", true, &format!("{drawing}{reset}{own}1 (Style) XW\n")), &doc);
        let (d, notes) = crate::ai_alone(&private(&art)).unwrap();
        assert_eq!(kinds(&d.layers[0]), ["group[2]"], "{reset}{notes:?}");
        let group = &d.layers[0].children().unwrap()[0];
        assert_eq!((group.opacity, group.blend, group.knockout), (0.2, BlendMode::Multiply, Knockout::On), "{reset}");
        assert_eq!(kinds(group), ["group[1]", "text"], "{reset}");
        assert_eq!(group.children().unwrap()[0].knockout, Knockout::Neutral, "{reset}");
        assert!(group.children().unwrap().iter().all(|p| (p.opacity, p.blend) == (1.0, BlendMode::Normal)), "{reset}");
    }
    // Such a group isn't the look of a text object after it, so the groups don't nest.
    fn depth(n: &Node) -> usize {
        1 + n.children().map_or(0, |c| c.iter().map(|c| depth(c)).max().unwrap_or(0))
    }
    let art = native_with_text(&layer("Art", true, &(drawing.clone() + &format!("{own}1 (Style) XW\n").repeat(50))), &doc);
    let (d, notes) = crate::ai_alone(&private(&art)).unwrap();
    assert_eq!(depth(&d.layers[0]), 4, "{notes:?}");
    // Made type with the page's type of no text object kept after it: a group of the two has it.
    let art = native_with_text(&layer("Art", true, &format!("1 0.2 0 0 0 Xy\n{}", text_object(0))), &doc);
    let r = import(&eps(&format!("{page} 58.5 30 moveto (Extra) show"), &art)).unwrap();
    assert_eq!(kinds(&r.document.layers[0]), ["group[2]"], "{:?}", r.warnings);
    let group = &r.document.layers[0].children().unwrap()[0];
    assert_eq!((group.opacity, group.blend), (0.2, BlendMode::Multiply));
    let texts: Vec<String> = text_of(group).iter().map(|t| t.plain_text()).collect();
    assert_eq!(texts, ["Hi there", "Extra"]);
    assert!(group.children().unwrap().iter().all(|p| (p.opacity, p.blend) == (1.0, BlendMode::Normal)));
    // A story in a frame of a kind this doesn't know: the type that shows is the page's. An EPS page has
    // no transparency, so its type takes its text object's.
    let doc = doc.replace("/2 << /2 [ 1 0 0 1 0 0 ] >>", "/2 << /0 9 /2 [ 1 0 0 1 0 0 ] >>");
    let art = native_with_text(&layer("Art", true, &format!("{}\n1 0.2 0 0 0 Xy\n{}", square(10, 10), text_object(0))), &doc);
    let r = import(&eps(&(page_square(10, 10) + "\n" + page), &art)).unwrap();
    assert_eq!(names(&r.document), ["Art"], "{:?}", r.warnings);
    assert_eq!(states(&r.document), [(0.2, BlendMode::Multiply)], "{:?}", r.warnings);
    // A PDF part's type at its text object's transparency has it once; transparency of its own that
    // the text object doesn't have (a fill's) stays.
    let pdf = |state: (f32, BlendMode)| {
        let mut pdf = pdf_part(10.0, 86.0);
        let style = vectorcraft_doc::CharStyle { size: 14.0, ..Default::default() };
        let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(58.5, 38.5), "Hi there", style);
        let mut text = Node::new(pdf.alloc_id(), NodeKind::Text(Box::new(text)));
        (text.opacity, text.blend) = state;
        let layer_id = pdf.layers[0].id;
        pdf.insert(Some(layer_id), 1, text).unwrap();
        pdf
    };
    let (d, w, read) = crate::layered_ai(&private(&art), pdf((0.2, BlendMode::Multiply)), vec![], false, false, 1.0);
    assert!(read, "{w:?}");
    assert_eq!(states(&d), [(0.2, BlendMode::Multiply)], "{w:?}");
    let plain = native_with_text(&layer("Art", true, &format!("{}\n{}", square(10, 10), text_object(0))), &doc);
    let (d, w, read) = crate::layered_ai(&private(&plain), pdf((0.7, BlendMode::Normal)), vec![], false, false, 1.0);
    assert!(read, "{w:?}");
    assert_eq!(states(&d), [(0.7, BlendMode::Normal)], "{w:?}");
    // The same type with a look that draws more than it, which the page draws with the look's
    // transparency: the group of the two has it once.
    let art = native_with_text(&layer("Art", true, &format!("{drawing}{own}1 (Style) XW\n")), &doc);
    let (d, w, read) = crate::layered_ai(&private(&art), pdf((0.2, BlendMode::Multiply)), vec![], false, false, 1.0);
    assert!(read, "{w:?}");
    assert_eq!(kinds(&d.layers[0]), ["group[2]"], "{w:?}");
    let group = &d.layers[0].children().unwrap()[0];
    assert_eq!((group.opacity, group.blend), (0.2, BlendMode::Multiply));
    assert_eq!(states(&d), [(1.0, BlendMode::Normal)], "{w:?}");
    // Page type that is a text and the path of its stroke: a group of the two takes the transparency.
    let page = format!(
        "{}\n/Helvetica findfont 12 scalefont setfont 10 50 moveto (Hi) show\n10 50 moveto (Hi) false charpath 2 setlinewidth stroke",
        page_square(10, 10)
    );
    let words = layer("Words", true, "1 0.2 0 0 0 Xy\n/AI11Text :\n0 /FreeUndo ,\n;\n");
    let r = import(&eps(&page, &native(&(layer("Art", true, &square(10, 10)) + &words)))).unwrap();
    let words = &r.document.layers[1];
    assert_eq!(kinds(words), ["group[2]"], "{:?}", r.warnings);
    let group = &words.children().unwrap()[0];
    assert_eq!((group.opacity, group.blend), (0.2, BlendMode::Multiply));
    assert_eq!(kinds(group), ["text", "path"]);
    assert!(group.children().unwrap().iter().all(|p| (p.opacity, p.blend) == (1.0, BlendMode::Normal)));
}

#[test]
fn a_commented_copy_of_a_story_that_has_a_plain_text_object_is_not_another_object() {
    // The file writes some stories twice (plain, then as comments) and others only as comments.
    let doc = text_document(&[("One\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0), ("Two\r", 0, (8300.0, 8180.0), (0.0, 0.0), 0.0)]);
    let commented = |story| -> String { text_object(story).lines().map(|l| format!("%_{l}\n")).collect() };
    let body = text_object(0) + &commented(0) + &commented(1);
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, &body)), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    let texts: Vec<String> = text_of(&r.document.layers[1]).iter().map(|t| t.plain_text()).collect();
    assert_eq!(texts, ["One", "Two"], "{:?}", r.warnings);
}

#[test]
fn a_shown_text_object_on_the_page_that_the_page_doesnt_draw_is_left_out() {
    // Story 1 is at (58.5, 61.5) on the 100 × 100 page, where the page paints no type: the file
    // keeps it but the app that wrote it doesn't draw it (type turned to outlines leaves one).
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0), ("Ghost\r", 0, (8200.0, 8200.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &(text_object(0) + &text_object(1))), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (Hi there) show";
    let r = import(&eps(page, &art)).unwrap();
    let mut all = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            all.push(t.plain_text());
        }
    });
    assert_eq!(all, ["Hi there"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("doesn't draw")), "{:?}", r.warnings);
}

/// A text object on a layer that shows and doesn't print, which the page doesn't draw, isn't taken
/// out: it comes in from the text document, on its layer.
#[test]
fn a_shown_text_object_on_a_layer_that_doesnt_print_stays_on_its_layer() {
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    // The layer's options with its fourth flag, printing, off.
    let tips = layer("Tips", true, &text_object(0)).replace("1 1 1 1 0 0 1 0 79", "1 1 1 0 0 0 1 0 79");
    let art = native_with_text(&(layer("Art", true, &square(10, 10)) + &tips), &doc);
    let (d, w, read) = crate::layered_ai(&private(&art), pdf_part(10.0, 86.0), vec![], false, false, 1.0);
    assert!(read, "{w:?}");
    assert_eq!(names(&d), ["Art", "Tips"], "{w:?}");
    let tips = &d.layers[1];
    assert!(tips.visible && matches!(tips.kind, NodeKind::Layer { printable: false, .. }));
    let texts: Vec<String> = text_of(tips).iter().map(|t| t.plain_text()).collect();
    assert_eq!(texts, ["Hi there"], "{w:?}");
    assert!(!w.iter().any(|w| w.contains("doesn't draw")), "{w:?}");
}

#[test]
fn pieces_of_two_lines_the_page_draws_alternately_go_each_to_its_own_text() {
    // "Hello" is drawn as "He" and "llo", with "abcd" ("ab", "cd") drawn between them.
    let doc = text_document(&[("Hello\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0), ("abcd\r", 0, (8200.0, 8160.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&layer("Art", true, &(text_object(0) + &text_object(1))), &doc);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (He) show 58.5 81.5 moveto (ab) show \
                74.1 81.5 moveto (cd) show 76.4 61.5 moveto (llo) show";
    let r = import(&eps(page, &art)).unwrap();
    let mut all = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            all.push(t.plain_text());
        }
    });
    all.sort();
    assert_eq!(all, ["Hello", "abcd"], "{:?}", r.warnings);
}

#[test]
fn a_run_longer_than_any_text_doesnt_overflow() {
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]).replace("/1 9 >>", "/1 1e30 >>");
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, &text_object(0))), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Back", "Spare"], "{:?}", r.warnings);
}

#[test]
fn more_text_objects_than_are_matched_leave_the_page() {
    let many = text_object(0).repeat(5001);
    let doc = text_document(&[("Hi\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, &many)), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("too many text objects") && crate::is_loss(w)), "{:?}", r.warnings);
}

#[test]
fn a_dictionary_left_open_at_the_end_of_a_layer_is_a_damaged_file() {
    let open = "/AI11Text :\n0 /FreeUndo ,\n";
    let art = native(&(layer("A", true, &(square(10, 10) + open)) + &layer("B", true, &square(20, 20))));
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| crate::is_loss(w)), "{:?}", r.warnings);
}

#[test]
fn an_ai_file_without_its_pdf_part_reads_its_layers_and_makes_its_type() {
    // Point type from the text document, shown or not; a story it can't read is left out, with a
    // note that is a loss.
    let mut doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0), ("Area\r", 0, (8200.0, 8160.0), (0.0, 0.0), 0.0)]);
    // Story 1 in frame 1, a frame of a kind this doesn't know.
    let at = doc.rfind("/1 << /0 [ << /0 0 >> ]").unwrap();
    doc.replace_range(at..at + 23, "/1 << /0 [ << /0 1 >> ]");
    let at = doc.rfind("/2 << /2 [ 1 0 0 1 0 0 ] >>").unwrap();
    doc.replace_range(at..at + 27, "/2 << /0 9 /2 [ 1 0 0 1 0 0 ] >>");
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Words", true, &(text_object(0) + &text_object(1)))), &doc);
    let (d, notes) = crate::ai_alone(&private(&art)).unwrap();
    assert_eq!(names(&d), ["Back", "Words"], "{notes:?}");
    let texts: Vec<String> = text_of(&d.layers[1]).iter().map(|t| t.plain_text()).collect();
    assert_eq!(texts, ["Hi there"], "{notes:?}");
    let left = notes.iter().find(|n| n.contains("left out")).unwrap();
    assert!(left.starts_with("1 ") && crate::is_loss(left), "{notes:?}");
    assert!(!notes.iter().any(|n| n.contains("order the page paints")), "{notes:?}");
    assert!(crate::ai_alone(b"%AI24_ZStandard_Data nothing").is_err());
}

/// A `.ai`'s PDF part has a page for each artboard, in a row: the type on a later page goes to
/// its text object where the layers have that artboard, not by the first page's place.
#[test]
fn type_on_a_later_artboard_of_an_ai_file_goes_to_its_text_object() {
    // Two 100-point artboards, the second 50 below the first; the type is 50 down the second
    // (art space: (58.5, -100); the canvas centre 8191.5 is the template box's centre, (50, 50)).
    let boards = "%_/Document :\n%_/Dictionary :\n%_/Array :\n\
        %_/Dictionary :\n%_0 100 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_100 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n\
        %_/Dictionary :\n%_0 -50 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_100 -150 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n\
        %_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n";
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8341.5), (0.0, 0.0), 0.0)]);
    let art = native_with_text(&(boards.to_string() + &layer("Art", true, &(square(10, 10) + &text_object(0)))), &doc);
    // The PDF part: the second page 20 points right of the first, the type 50 down it.
    let mut pdf = pdf_part(10.0, 86.0);
    let mut page = pdf.artboards[0].clone();
    (page.id, page.rect) = (2, vectorcraft_geom::Rect::new(120.0, 0.0, 220.0, 100.0));
    pdf.artboards.push(page);
    let style = vectorcraft_doc::CharStyle { size: 14.0, ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(178.5, 50.0), "Hi there", style);
    let (id, layer_id) = (pdf.alloc_id(), pdf.layers[0].id);
    pdf.insert(Some(layer_id), 1, Node::new(id, NodeKind::Text(Box::new(text)))).unwrap();
    let (d, w, read) = crate::layered_ai(&private(&art), pdf, vec![], false, false, 1.0);
    assert!(read, "{w:?}");
    assert_eq!(d.artboards[1].rect, vectorcraft_geom::Rect::new(0.0, 150.0, 100.0, 250.0));
    let texts = text_of(&d.layers[0]);
    assert_eq!(texts.len(), 1, "{w:?}");
    let [.., e, f] = texts[0].xf.as_coeffs();
    assert!((e - 58.5).abs() < 1e-6 && (f - 200.0).abs() < 1e-6, "{:?}", texts[0].xf);
    assert!(!w.iter().any(|w| w.contains("with another's") || w.contains("doesn't draw")), "{w:?}");
}

/// A global process colour (`Xk`) or a pattern fill on a sublayer no longer sends a `.ai` to its
/// PDF part, which has only the top-level layers (#1025).
#[test]
fn a_global_colour_or_a_pattern_fill_keeps_the_sublayers_of_an_ai_file() {
    // The PDF part's yellow square, in a global yellow at full tint, on a sublayer; a hidden
    // sublayer with a pattern fill (the page doesn't have it).
    let yellow = "0 0 1 0 (Yellow global) 0 0 Xk\n10 10 m\n14 10 L\n14 14 L\n10 14 L\nf";
    let dots = "%AI3_BeginPattern: (Dots)\n(Dots) 0 0 4 4\n%_0 0 0 1 k\n%_1 1 m\n%_3 1 L\n%_3 3 L\n%_1 3 L\n%_f\nE\n%AI3_EndPattern\n";
    let patterned = "(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n50 50 m\n70 50 L\n70 70 L\n50 70 L\nf";
    let ground = layer("Ground", true, &(layer("Lawn", true, yellow) + &layer("Paving", false, patterned)));
    let art = native(&format!("{dots}{ground}"));
    let (d, warnings, _) = crate::layered_ai(&private(&art), pdf_part(10.0, 86.0), vec![], false, false, 1.0);
    assert_eq!(names(&d), ["Ground"], "{warnings:?}");
    let subs: Vec<_> = d.layers[0].children().unwrap().iter().map(|l| (l.name.clone().unwrap_or_default(), kinds(l))).collect();
    assert_eq!(subs, [("Lawn".to_string(), vec!["path"]), ("Paving".to_string(), vec!["path"])]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(d.swatch("Yellow global").is_some_and(|s| s.global && !s.spot));
    assert!(d.pattern("Dots").is_some() && d.swatch("Dots").is_some());
    // Without a PDF part, the pattern fill shows.
    let shown = native(&format!("{dots}{}", layer("Ground", true, &layer("Lawn", true, patterned))));
    let (d, notes) = crate::ai_alone(&private(&shown)).unwrap();
    let lawn = &d.layers[0].children().unwrap()[0];
    assert_eq!((lawn.name.as_deref(), kinds(lawn)), (Some("Lawn"), vec!["path"]), "{notes:?}");
    let fill = lawn.children().unwrap()[0].appearance.fill().map(|f| f.paint.clone());
    assert!(matches!(&fill, Some(vectorcraft_color::Paint::Pattern { pattern, .. }) if pattern == "Dots"), "{fill:?}");
}

/// A `.ai` whose PDF part is older than its editing copy draws less than its layers: trusted, the
/// layers are used anyway, and the difference is noted.
#[test]
fn a_trusted_editing_copy_is_used_where_it_draws_differently_from_the_pdf_part() {
    let extra = "0 0 1 0 k\n50 50 m\n70 50 L\n70 70 L\n50 70 L\nf";
    let art = native(&layer("Back", true, &(square(10, 10) + "\n" + extra)));
    let (d, w, _) = crate::layered_ai(&private(&art), pdf_part(10.0, 86.0), vec![], false, false, 1.0);
    assert_ne!(names(&d), ["Back"], "untrusted, the PDF part is used: {w:?}");
    let (d, w, _) = crate::layered_ai(&private(&art), pdf_part(10.0, 86.0), vec![], false, true, 1.0);
    assert_eq!(names(&d), ["Back"], "{w:?}");
    assert_eq!(d.layers[0].children().unwrap().len(), 2);
    assert!(w.iter().any(|w| w.contains("differs from the page's") && w.contains("all the same")), "{w:?}");
}

#[test]
fn an_object_the_page_doesnt_draw_makes_the_file_come_in_as_its_page() {
    // A 20-point square that the page doesn't draw: 4% of the page, and an object all the same.
    let extra = "0 0 1 0 k\n50 50 m\n70 50 L\n70 70 L\n50 70 L\nf";
    let art = native(&layer("Back", true, &(square(10, 10) + "\n" + extra)));
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("layers weren't read") && w.contains("differs from the page's")), "{:?}", r.warnings);
}

/// The page sets "Hi there" in pieces, and the layers lay the line out again as one: the page and
/// the layers differ only where the type is.
fn page_in_pieces_and_layers(art: &str) -> Vec<u8> {
    let doc = text_document(&[("Hi there\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    let page = "0 0 0 1 setcmykcolor /Helvetica findfont 14 scalefont setfont 58.5 61.5 moveto (Hi) show 70 61.5 moveto (there) show";
    eps(page, &native_with_text(&layer("Art", true, &format!("{art}{}", text_object(0))), &doc))
}

#[test]
fn type_laid_out_again_is_noted_as_type_and_not_as_art_the_layers_dont_describe() {
    let r = import(&page_in_pieces_and_layers("")).unwrap();
    assert_eq!(names(&r.document), ["Art"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.starts_with("the type on the layers differs from the file's own page")), "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("brush strokes")), "{:?}", r.warnings);
}

#[test]
fn art_the_page_doesnt_draw_is_noted_with_its_own_share_and_not_the_types() {
    // A bar of 0.5 × 80 points that the page doesn't draw, too thin to be refused as an object: 0.4% of
    // the page, away from the type.
    let bar = "0 0 0 1 k\n20 10 m\n20.5 10 L\n20.5 90 L\n20 90 L\nf\n";
    let r = import(&page_in_pieces_and_layers(bar)).unwrap();
    assert_eq!(names(&r.document), ["Art"], "{:?}", r.warnings);
    let notes: Vec<&String> = r.warnings.iter().filter(|w| w.contains("differs from the file's own page")).collect();
    assert_eq!(notes.len(), 1, "{:?}", r.warnings);
    assert!(notes[0].starts_with("the art on the layers differs from the file's own page in about 0.4% of it (brush strokes"), "{notes:?}");
}

/// An EPS whose page is its art's box `[x0 y0 x1 y1]` (not its artboard, 0 0 100 100), drawing
/// `page`, with the editing copy of `layers`.
fn eps_of_art(art_box: [u32; 4], page: &str, layers: &str) -> Vec<u8> {
    let [x0, y0, x1, y1] = art_box;
    let native = native(layers).replace(
        "%%BoundingBox: 0 0 100 100\n%%HiResBoundingBox: 0 0 100 100",
        &format!("%%BoundingBox: {x0} {y0} {x1} {y1}\n%%HiResBoundingBox: {x0} {y0} {x1} {y1}"),
    );
    String::from_utf8(eps(page, &native))
        .unwrap()
        .replacen("%%BoundingBox: 0 0 100 100", &format!("%%BoundingBox: {x0} {y0} {x1} {y1}"), 1)
        .into_bytes()
}

#[test]
fn art_the_layers_print_outside_an_eps_page_of_its_art_makes_it_come_in_as_its_page() {
    // The page is the box of the art it prints, round the square at (10, 10).
    let file = |far: &str| eps_of_art([10, 10, 14, 14], &page_square(10, 10), &(layer("Art", true, &square(10, 10)) + far));
    // A square the layers print far from it: the page would have it.
    let r = import(&file(&layer("Far", true, &square(80, 80)))).unwrap();
    assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("layers weren't read")), "{:?}", r.warnings);
    // Hidden, or on a layer that doesn't print, the page doesn't have it.
    let r = import(&file(&layer("Far", false, &square(80, 80)))).unwrap();
    assert_eq!(names(&r.document), ["Art", "Far"], "{:?}", r.warnings);
    let not_printed = format!("%AI5_BeginLayer\n1 1 1 0 0 0 1 0 79 128 255 0 50 0 Lb\n(Notes) Ln\n{}\nLB\n%AI5_EndLayer--\n", square(80, 80));
    let r = import(&file(&not_printed)).unwrap();
    assert_eq!(names(&r.document), ["Art", "Notes"], "{:?}", r.warnings);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

/// An EPS whose page is its art's box keeps that page when it comes in through its layers, as it
/// does as its page: not the editing data's artboard (#1102).
#[test]
fn an_eps_page_of_its_art_stays_its_page_through_the_layers() {
    let file = eps_of_art([10, 10, 14, 14], &page_square(10, 10), &(layer("Art", true, &square(10, 10)) + &layer("Far", false, &square(80, 80))));
    let r = import(&file).unwrap();
    assert_eq!(names(&r.document), ["Art", "Far"], "{:?}", r.warnings);
    let boards: Vec<_> = r.document.artboards.iter().map(|a| a.rect).collect();
    let art = r.document.layers[0].children().unwrap()[0].visual_bounds().unwrap();
    assert_eq!(boards.len(), 1);
    assert!((boards[0].width() - 4.0).abs() < 1e-6 && (boards[0].height() - 4.0).abs() < 1e-6, "{boards:?}");
    assert!(boards[0].inflate(1e-6, 1e-6).contains_rect(art), "{boards:?} round {art:?}");
}

#[test]
fn an_eps_saved_as_its_artboard_keeps_the_art_outside_it() {
    // The page is the artboard (0 0 100 100): the art beyond it is the pasteboard's.
    let art = native(&(layer("Art", true, &square(10, 10)) + &layer("Scraps", true, &square(120, 10))));
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Art", "Scraps"], "{:?}", r.warnings);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn type_asked_for_as_outlines_leaves_a_file_whose_type_shows_as_its_pdf_part() {
    let doc = text_document(&[("Hi\r", 0, (8200.0, 8180.0), (0.0, 0.0), 0.0)]);
    // Type that shows: the PDF part's outlines are its type, which the text objects can't hold.
    let shown = native_with_text(&layer("Art", true, &(square(10, 10) + "\n" + &text_object(0))), &doc);
    let (d, w, _) = crate::layered_ai(&private(&shown), pdf_part(10.0, 86.0), vec![], true, false, 1.0);
    assert_eq!(names(&d), ["Layer 1"], "{w:?}");
    assert!(w.iter().any(|w| w.contains("layers weren't read") && w.contains("outlines")), "{w:?}");
    // Type only on a hidden layer: the layers come in.
    let hidden = native_with_text(&(layer("Art", true, &square(10, 10)) + &layer("Notes", false, &text_object(0))), &doc);
    let (d, w, _) = crate::layered_ai(&private(&hidden), pdf_part(10.0, 86.0), vec![], true, false, 1.0);
    assert_eq!(names(&d), ["Art", "Notes"], "{w:?}");
}

/// A point of the art (y up) on the canvas of a file whose template box is centred at (50, 50).
fn canvas(x: f64, y: f64) -> (f64, f64) {
    (x + 8141.5, 8241.5 - y)
}

/// The segments (four points each) of the rectangle `x0 y0 x1 y1` of the art, on the canvas.
fn canvas_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
    let corners = [canvas(x0, y1), canvas(x0, y0), canvas(x1, y0), canvas(x1, y1), canvas(x0, y1)];
    corners
        .windows(2)
        .map(|w| format!("{} {} {} {} {} {} {} {}", w[0].0, w[0].1, w[0].0, w[0].1, w[1].0, w[1].1, w[1].0, w[1].1))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A story of `text` in `frames`, one paragraph (`para`: its keys) and one style (`style`).
fn story(text: &str, frames: &[usize], para: &str, style: &str) -> String {
    let n = text.chars().count();
    let frames: String = frames.iter().map(|f| format!("<< /0 {f} >> ")).collect();
    format!(
        "<< /0 << /0 ({text}) /5 << /0 [ << /0 << /0 << /0 () /5 << {para} >> /6 0 >> >> /1 {n} >> ] >> \
         /6 << /0 [ << /0 << /0 << /0 () /5 0 /6 << {style} >> >> >> /1 {n} >> ] >> >> /1 << /0 [ {frames}] >> >>"
    )
}

/// A text document of `frames` and `stories` (written by [`story`]).
fn document_of(frames: &[String], stories: &[String]) -> String {
    format!(
        "/0 << /1 << /0 [ << /0 << /0 << /0 (Helvetica) >> >> >> ] >> /8 << /0 [ {} ] >> >>\n/1 << /1 [ {} ] /2 << /1 12.0 >> >>\n",
        frames.join(" "),
        stories.join(" ")
    )
}

/// A text object of frame `frame` of story `story`.
fn frame_object(story: u32, frame: u32) -> String {
    format!("/AI11Text :\n0 /FreeUndo ,\n{frame} /FrameIndex ,\n{story} /StoryIndex ,\n;\n")
}

#[test]
fn area_type_and_type_on_a_path_come_from_the_text_document() {
    let area = format!("<< /0 << /0 [ 0 0 ] /1 << /0 [ {} ] >> /2 << /0 1 /7 18 /8 18 >> >> >>", canvas_rect(10.0, 20.0, 60.0, 40.0));
    // An open path of two segments, the type from halfway along the first to the end.
    let (a, b, c) = (canvas(10.0, 70.0), canvas(30.0, 70.0), canvas(50.0, 70.0));
    let path = format!(
        "<< /0 << /0 [ 0 0 ] /1 << /0 [ {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} ] >> /2 << /0 2 /6 [ 0.5 2.0 ] >> >> >>",
        a.0, a.1, a.0, a.1, b.0, b.1, b.0, b.1, b.0, b.1, b.0, b.1, c.0, c.1, c.0, c.1
    );
    let red = "/1 9.0 /8 50 /6 1.5 /53 << /99 /CAITextPaint /0 << /0 1 /1 [ 1.0 1.0 0.0 0.0 ] >> >>";
    let doc = document_of(
        &[area, path],
        &[story("Area type\\rSecond\\r", &[0], "/0 2 /1 4 /2 6 /4 3", red), story("On a path\\r", &[1], "/0 0", "/1 8.0")],
    );
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, &(text_object(0) + &text_object(1)))), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    let texts = text_of(&r.document.layers[1]);
    assert_eq!(texts.len(), 2, "{:?}", r.warnings);
    let (area, path) = (texts[0], texts[1]);
    assert_eq!(area.plain_text(), "Area type\nSecond");
    let vectorcraft_doc::TextKind::Area { frame } = &area.kind else { panic!("area type: {:?}", area.kind) };
    // Art y 20..40 is 60..80 down the 100 pt page.
    let b = frame.bounds().unwrap();
    assert!((b.x0 - 10.0).abs() < 1e-6 && (b.y0 - 60.0).abs() < 1e-6 && (b.x1 - 60.0).abs() < 1e-6 && (b.y1 - 80.0).abs() < 1e-6, "{b:?}");
    assert_eq!(area.para.justify, vectorcraft_doc::Justify::Center);
    assert_eq!((area.para.first_line_indent, area.para.left_indent, area.para.space_before), (4.0, 6.0, 3.0));
    let style = &area.runs[0].style;
    assert_eq!((style.size, style.tracking, style.h_scale), (9.0, 50.0, 150.0));
    assert_eq!(style.fill.color(), Some(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)));
    let vectorcraft_doc::TextKind::OnPath { path: p, start, end } = &path.kind else { panic!("type on a path: {:?}", path.kind) };
    assert!((start - 0.25).abs() < 1e-3 && end.is_none(), "{start} {end:?}");
    assert_eq!(p.bounds().unwrap().width(), 40.0);
    assert_eq!(path.runs[0].style.size, 8.0);
    assert!(!r.warnings.iter().any(|w| w.contains("couldn't be read")), "{:?}", r.warnings);
}

#[test]
fn threaded_area_type_flows_through_its_frames() {
    let frames = [
        format!("<< /0 << /0 [ 0 0 ] /1 << /0 [ {} ] >> /2 << /0 1 >> >> >>", canvas_rect(10.0, 60.0, 40.0, 80.0)),
        format!("<< /0 << /0 [ 0 0 ] /1 << /0 [ {} ] >> /2 << /0 1 >> >> >>", canvas_rect(50.0, 60.0, 80.0, 80.0)),
    ];
    let doc = document_of(&frames, &[story("A story in two frames\\r", &[0, 1], "/0 0", "/1 10.0")]);
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, &(frame_object(0, 0) + &frame_object(0, 1)))), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    let texts = text_of(&r.document.layers[1]);
    assert_eq!(texts.len(), 2, "{:?}", r.warnings);
    assert_eq!(texts[0].plain_text(), "A story in two frames");
    assert_eq!(texts[1].plain_text(), "");
    let ids: Vec<_> = r.document.layers[1].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(r.document.text_threads, [ids]);
}

#[test]
fn shown_type_the_page_draws_in_pieces_comes_whole_from_the_text_document() {
    // Type on a path the page draws letter by letter: the letters lie on the path, and only the
    // text document's type object comes in.
    let (a, b) = (canvas(10.0, 50.0), canvas(90.0, 50.0));
    let path = format!(
        "<< /0 << /0 [ 0 0 ] /1 << /0 [ {} {} {} {} {} {} {} {} ] >> /2 << /0 2 /6 [ 0 0.99 ] >> >> >>",
        a.0, a.1, a.0, a.1, b.0, b.1, b.0, b.1
    );
    let doc = document_of(&[path], &[story("Letters\\r", &[0], "/0 0", "/1 12.0")]);
    let art = native_with_text(&layer("Art", true, &text_object(0)), &doc);
    let letters: String = "Letters".chars().enumerate().map(|(i, c)| format!("{} 50 moveto ({c}) show ", 12 + 7 * i)).collect();
    let page = format!("0 0 0 1 setcmykcolor /Helvetica findfont 12 scalefont setfont {letters}");
    let r = import(&eps(&page, &art)).unwrap();
    let mut all = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            all.push(t.plain_text());
        }
    });
    assert_eq!(all, ["Letters"], "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("page's type")), "{:?}", r.warnings);
}

#[test]
fn a_copied_area_type_object_is_framed_where_its_own_art_path_is() {
    // The text document's frame is the original's (art 10..60 × 20..40); this copy's own `/Art`
    // path puts it at 30..80 × 50..70.
    let area = format!("<< /0 << /0 [ 0 0 ] /1 << /0 [ {} ] >> /2 << /0 1 >> >> >>", canvas_rect(10.0, 20.0, 60.0, 40.0));
    let doc = document_of(&[area], &[story("Copied\r", &[0], "/0 0", "/1 8.0")]);
    let copy = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n/Art :\nX=\n30 50 m\n80 50 L\n80 70 L\n30 70 L\nN\n;\n/ConfiningPath ,\n2 /TextAntialiasing ,\n;\n";
    let art = native_with_text(&(layer("Back", true, &square(10, 10)) + &layer("Spare", false, copy)), &doc);
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    let texts = text_of(&r.document.layers[1]);
    assert_eq!(texts.len(), 1, "{:?}", r.warnings);
    let vectorcraft_doc::TextKind::Area { frame } = &texts[0].kind else { panic!("area type: {:?}", texts[0].kind) };
    // Art y 50..70 is 30..50 down the 100 pt page.
    let b = frame.bounds().unwrap();
    assert!((b.x0 - 30.0).abs() < 1e-6 && (b.y0 - 30.0).abs() < 1e-6 && (b.x1 - 80.0).abs() < 1e-6 && (b.y1 - 50.0).abs() < 1e-6, "{b:?}");
}

/// A raster print page can carry a fully vector editing copy. The legacy mesh has no
/// opacity operand and its vector mask hides half of it: ignoring either loses this import.
#[test]
fn raster_print_page_recovers_legacy_mesh_vector_mask_and_editable_curve() {
    let mesh = "/Mesh X!\n%_/DeviceRGB /CS X#\n\
        %_[0 0 1 20 20 20 30 1 30 20 1] /N X#\n\
        %_[0 0 1 20 60 30 60 1 20 50 1] /N X#\n\
        %_[0 0 1 60 60 60 50 1 50 60 1] /N X#\n\
        %_[0 0 1 60 20 50 20 1 60 30 1] /N X#\n%_/E X#\n/End X!\n";
    let mask = "%_/ArtDictionary :\n%_/Mask :\n%_X=\n%_1 g\n%_20 20 m\n%_20 60 L\n%_40 60 L\n%_40 20 L\n%_f\n%_X+\n%_(Art) ,\n%_1 /Bool (Clipping) ,\n%_1 /Bool (Linked) ,\n%_; (AI9_Mask) ,\n%_;\n";
    let curve = "0 1 1 0 1 0 0 Xa\n70 20 m\n70 50 90 50 90 20 c\nf\n";
    let data = native(&layer("Editable", true, &format!("u\n{mesh}{mask}{curve}U\n")));
    let samples = ("0000ff".repeat(20) + &"ffffff".repeat(20)).repeat(40);
    let page = format!(
        "gsave 20 20 translate 40 40 scale /DeviceRGB setcolorspace \
        << /ImageType 1 /Width 40 /Height 40 /BitsPerComponent 8 /Decode [0 1 0 1 0 1] \
        /ImageMatrix [40 0 0 -40 0 40] /DataSource <{samples}> >> image grestore \
        1 0 0 setrgbcolor 70 20 moveto 70 50 90 50 90 20 curveto closepath fill"
    );
    // Use AI9's zlib container, too, rather than only the newer Zstandard test helper.
    use std::io::Write as _;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data.as_bytes()).unwrap();
    let packed = ascii85(&encoder.finish().unwrap());
    let file = format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{page}\nshowpage\n%%EOF\n%AI9_PrivateDataBegin\n%AI9_DataStream\n%{packed}\n%AI9_PrivateDataEnd\n"
    );
    let r = import(file.as_bytes()).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(names(&r.document), ["Editable"]);
    let group = &r.document.layers[0].children().unwrap()[0];
    let a = group.children().unwrap();
    assert_eq!(a.len(), 2);
    assert!(matches!(a[0].kind, NodeKind::Mesh(_)));
    assert!(a[0].extras().mask.is_some());
    let NodeKind::Path { path, .. } = &a[1].kind else { panic!("editable path") };
    assert_eq!(path.anchor_count(), 2);
    assert_ne!(path.subpaths[0].anchors[0].h_out, path.subpaths[0].anchors[0].p);
    assert_ne!(a[0].id, a[1].id);
    // The normal EPS path remains available explicitly.
    let page = crate::import_with(file.as_bytes(), false).unwrap();
    assert!(matches!(page.document.layers[0].children().unwrap()[0].kind, NodeKind::Image(_)));
    // A wrong editing copy must still fail the page comparison.
    let wrong = file.replace("%%BoundingBox: 0 0 100 100", "%%BoundingBox: 0 0 120 100");
    let r = import(wrong.as_bytes()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("for a 100 × 100 pt page, not the file's 120 × 100 pt page")), "{:?}", r.warnings);
}
