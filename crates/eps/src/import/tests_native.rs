//! The layers of an Illustrator EPS and `.ai`: read from the editing copy of the art the file
//! carries (see `native`). The files here are small ones written by hand for the tests, with the
//! operators seen in files their users own.

use vectorcraft_doc::{Document, LayerColor, Node, NodeKind};

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
        // A clipping group: its objects, then its clip path.
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
    // The page has what shows: the clip group's square is outside its clip path.
    let page = [10, 20, 30, 80].map(|x| page_square(x, 10)).join("\n");
    let r = import(&eps(&page, &art)).unwrap();
    let outer = &r.document.layers[0];
    assert_eq!(kinds(outer), ["group[1]", "compound[2]", "clip[2]", "group[1]", "other"]);
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

#[test]
fn an_ai_file_gets_its_layers_and_the_art_outside_its_artboard() {
    // A square on the artboard, and one above it (the artboard is y 0 to 100 here).
    let art = native(&(layer("Jig", true, &square(10, 10)) + &layer("Parts", true, &square(30, 110)) + &layer("Old", false, &square(60, 60))));
    let (d, warnings) = crate::layered_ai(&private(&art), pdf_part(10.0, 86.0), vec!["a note".into()], false);
    assert_eq!(names(&d), ["Jig", "Parts", "Old"], "{warnings:?}");
    assert_eq!(warnings, ["a note"], "nothing else to say");
    let above = d.layers[1].children().unwrap()[0].visual_bounds().unwrap();
    assert!(above.y1 <= 0.0 + 1e-6, "the art above the artboard is kept, outside it: {above:?}");
    assert!(!d.layers[2].visible);
}

#[test]
fn an_ai_file_that_cant_be_read_through_its_editing_data_keeps_its_pdf_part() {
    let pdf = pdf_part(10.0, 86.0);
    // No editing data this reads.
    let (d, w) = crate::layered_ai(b"nothing", pdf.clone(), vec![], false);
    assert_eq!(d.layers.len(), pdf.layers.len());
    assert!(w.is_empty());
    // Editing data that isn't for this page.
    let other = native(&layer("Jig", true, &square(10, 10))).replace("%AI3_Cropmarks: 0 0 100 100", "%AI3_Cropmarks: 0 0 50 50");
    let (d, w) = crate::layered_ai(&private(&other), pdf.clone(), vec![], false);
    assert_eq!(names(&d), names(&pdf));
    assert!(w.iter().any(|w| w.contains("different page")), "{w:?}");
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

#[test]
fn an_object_the_page_doesnt_draw_makes_the_file_come_in_as_its_page() {
    // A 20-point square that the page doesn't draw: 4% of the page, and an object all the same.
    let extra = "0 0 1 0 k\n50 50 m\n70 50 L\n70 70 L\n50 70 L\nf";
    let art = native(&layer("Back", true, &(square(10, 10) + "\n" + extra)));
    let r = import(&eps(&page_square(10, 10), &art)).unwrap();
    assert_eq!(names(&r.document), ["Layer 1"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("layers weren't read") && w.contains("differs from the page's")), "{:?}", r.warnings);
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
    let (d, w) = crate::layered_ai(&private(&shown), pdf_part(10.0, 86.0), vec![], true);
    assert_eq!(names(&d), ["Layer 1"], "{w:?}");
    assert!(w.iter().any(|w| w.contains("layers weren't read") && w.contains("outlines")), "{w:?}");
    // Type only on a hidden layer: the layers come in.
    let hidden = native_with_text(&(layer("Art", true, &square(10, 10)) + &layer("Notes", false, &text_object(0))), &doc);
    let (d, w) = crate::layered_ai(&private(&hidden), pdf_part(10.0, 86.0), vec![], true);
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
