//! The editing data written, then read back by VectorCraft's own reader ([`crate::ai_alone`]).

use std::sync::Arc;

use vectorcraft_color::swatch::{Swatch, SwatchGroup};
use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, Dash, Document, FillLayer, ImageBlob, ImageObject, LayerColor, LineCap, LineJoin, Node, NodeKind,
    StrokeLayer, Symbol,
};
use vectorcraft_geom::{Affine, FillRule, PathData, Rect, shapes};

use super::*;

fn opts() -> AiEditingOptions {
    AiEditingOptions { title: "Sample".into(), created: Some(0) }
}

/// The program written from `doc`, as text with its CR LF line ends read as LF.
fn program(doc: &Document) -> String {
    String::from_utf8(export_ai_editing(doc, &opts()).unwrap().program).unwrap().replace("\r\n", "\n")
}

/// `doc` written, then read back from the private data alone.
fn read_back(doc: &Document) -> Document {
    let (p, _) = private_data(doc, &opts()).unwrap();
    crate::ai_alone(&p.data).unwrap().0
}

fn solid(c: Color) -> Paint {
    Paint::solid(c)
}

/// A swatch's color at `tint`, linked to it.
fn linked(name: &str, full: Color, tint: f32) -> Paint {
    Paint::Solid { color: full.tinted(tint), swatch: Some(name.into()), tint }
}

const BRAND: Color = Color::Cmyk { c: 0.1, m: 0.7, y: 0.2, k: 0.05 };
const GOLD: Color = Color::Cmyk { c: 0.0, m: 0.25, y: 0.9, k: 0.1 };
const SKY: Color = Color::Rgb { r: 0.25, g: 0.5, b: 1.0 };
const MINT: Color = Color::Lab { l: 80.0, a: -40.0, b: 10.0 };

/// A 200 × 100 document with a second artboard: swatches (process, global process, CMYK, RGB and
/// Lab spot colors, a color group); a layer `Back` (preset color 2) holding a named group `Pair`
/// of a named rectangle `Red` (stroked 2 pt, round caps, bevel joins, miter limit 4, dashed, over
/// a gray fill that overprints) and a hidden, locked one, an even-odd ellipse in a global color at
/// half tint, an open path stroked in a spot color, a guide, and a hidden locked sublayer `Sub`
/// (a custom color, not printing, dimmed) holding a path filled with an RGB and stroked with a Lab
/// spot color; a second layer `Front`, locked, with a CMYK square named like the rectangle.
fn sample() -> Document {
    let mut doc = Document::new(200.0, 100.0);
    let mut second = doc.artboards[0].clone();
    second.id = 2;
    second.name = "Back side".into();
    second.rect = Rect::new(250.0, 0.0, 450.0, 100.0);
    doc.artboards.push(second);
    let swatch = |name: &str, c: Color, global: bool, spot: bool| Swatch { name: name.into(), paint: solid(c), global, spot };
    doc.swatches = vec![
        Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false },
        swatch("Teal", Color::rgb(0.0, 0.5, 0.5), false, false),
        swatch("Brand", BRAND, true, false),
        swatch("Gold", GOLD, true, true),
        swatch("Sky", SKY, true, true),
        swatch("Mint", MINT, true, true),
    ]
    .into();
    doc.swatch_groups = vec![SwatchGroup { name: "Grays".into(), swatches: vec![swatch("Mid", Color::gray(0.5), false, false)] }].into();
    doc.layers.clear();
    let back = doc.add_layer(Some("Back"));
    let front = doc.add_layer(Some("Front"));

    let mut stroke = StrokeLayer::new(solid(Color::rgb(0.0, 0.0, 1.0)), 2.0);
    stroke.cap = LineCap::Round;
    stroke.join = LineJoin::Bevel;
    stroke.miter_limit = 4.0;
    stroke.dash = Some(Dash { pattern: vec![3.0, 2.0], offset: 1.0, align_corners: false });
    let fill = FillLayer { overprint: true, ..FillLayer::new(solid(Color::gray(0.25))) };
    let mut red = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)),
        Appearance { items: vec![AppearanceItem::Fill(fill), AppearanceItem::Stroke(stroke)], ..Appearance::default() },
    );
    red.name = Some("Red".into());
    let mut hidden = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(50.0, 10.0, 80.0, 40.0)),
        Appearance::basic(solid(Color::cmyk(0.0, 1.0, 1.0, 0.0)), Paint::None, 0.0),
    );
    hidden.visible = false;
    hidden.locked = true;
    let mut pair = Node::group(doc.alloc_id(), vec![Arc::new(red), Arc::new(hidden)]);
    pair.name = Some("Pair".into());
    doc.insert(Some(back), usize::MAX, pair).unwrap();

    let ellipse = Node::new(
        doc.alloc_id(),
        NodeKind::Path {
            path: shapes::ellipse(Rect::new(100.0, 10.0, 160.0, 50.0)),
            rule: FillRule::EvenOdd,
            live: None,
            clipping: false,
            guide: false,
        },
    );
    let ellipse = Node { appearance: Appearance::basic(linked("Brand", BRAND, 0.5), Paint::None, 0.0).into(), ..ellipse };
    doc.insert(Some(back), usize::MAX, ellipse).unwrap();
    let mut open = PathData::from_bezpath(&kurbo::BezPath::from_vec(vec![
        kurbo::PathEl::MoveTo((10.0, 60.0).into()),
        kurbo::PathEl::LineTo((60.0, 90.0).into()),
    ]));
    open.subpaths.iter_mut().for_each(|sp| sp.closed = false);
    let line = Node::path(doc.alloc_id(), open, Appearance::basic(Paint::None, linked("Gold", GOLD, 1.0), 3.0));
    doc.insert(Some(back), usize::MAX, line).unwrap();
    let guide = Node::new(
        doc.alloc_id(),
        NodeKind::Path {
            path: shapes::rectangle(Rect::new(0.0, 50.0, 200.0, 50.0)),
            rule: FillRule::NonZero,
            live: None,
            clipping: false,
            guide: true,
        },
    );
    doc.insert(Some(back), usize::MAX, guide).unwrap();

    let mut sub = Node::layer(doc.alloc_id(), "Sub", LayerColor::Custom([10, 200, 30]));
    if let NodeKind::Layer { printable, dim_images, .. } = &mut sub.kind {
        *printable = false;
        *dim_images = Some(70);
    }
    sub.visible = false;
    sub.locked = true;
    let sub = doc.insert(Some(back), usize::MAX, sub).unwrap();
    let skyfill = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(120.0, 60.0, 190.0, 95.0)),
        Appearance::basic(linked("Sky", SKY, 0.8), linked("Mint", MINT, 1.0), 1.0),
    );
    doc.insert(Some(sub), usize::MAX, skyfill).unwrap();

    let mut square = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(300.0, 20.0, 340.0, 60.0)),
        Appearance::basic(solid(Color::cmyk(0.5, 0.0, 1.0, 0.0)), Paint::None, 0.0),
    );
    square.name = Some("Red".into());
    doc.insert(Some(front), usize::MAX, square).unwrap();
    if let Some(l) = doc.layers.get_mut(1) {
        let l = Arc::make_mut(l);
        l.locked = true;
        if let NodeKind::Layer { color, .. } = &mut l.kind {
            *color = LayerColor::Preset(2);
        }
    }
    doc
}

/// `v` to three decimals.
fn r3(v: f64) -> String {
    format!("{:.3}", (v * 1000.0).round() / 1000.0 + 0.0)
}

fn color(c: &Color) -> String {
    let f = |v: f32| r3(f64::from(v));
    match *c {
        Color::Gray { k } => format!("gray {}", f(k)),
        Color::Cmyk { c, m, y, k } => format!("cmyk {} {} {} {}", f(c), f(m), f(y), f(k)),
        Color::Rgb { r, g, b } => format!("rgb {} {} {}", f(r), f(g), f(b)),
        Color::Lab { l, a, b } => format!("lab {} {} {}", f(l), f(a), f(b)),
    }
}

fn paint(p: &Paint) -> String {
    match p {
        Paint::Solid { color: c, swatch, tint } => format!("{} {swatch:?} {}", color(c), r3(f64::from(*tint))),
        Paint::None => "none".into(),
        other => format!("{other:?}"),
    }
}

/// What the reader's view of the slice's constructs is made of, one line per object.
fn view(nodes: &[Arc<Node>], depth: usize, out: &mut Vec<String>) {
    for n in nodes {
        let head = format!("{depth} {:?} shown={} locked={}", n.name, n.visible, n.locked);
        match &n.kind {
            NodeKind::Layer { color, printable, preview, dim_images, clip, children, .. } => {
                out.push(format!("{head} layer {color:?} printable={printable} preview={preview} dim={dim_images:?} clip={clip}"));
                view(children, depth + 1, out);
            }
            NodeKind::Group { children, clip } => {
                out.push(format!("{head} group clip={clip}"));
                view(children, depth + 1, out);
            }
            NodeKind::Path { path, rule, guide, .. } => {
                let mut s = format!("{head} path {rule:?} guide={guide}");
                for sp in &path.subpaths {
                    s += &format!(" [closed={}", sp.closed);
                    // Points are written to four decimals.
                    let r2 = |v: f64| format!("{:.2}", v + 0.0);
                    for a in &sp.anchors {
                        s += &format!(" {},{} {},{} {},{}", r2(a.h_in.x), r2(a.h_in.y), r2(a.p.x), r2(a.p.y), r2(a.h_out.x), r2(a.h_out.y));
                    }
                    s += "]";
                }
                if !guide {
                    // Items that paint nothing aren't written.
                    for it in n.appearance.items.iter().filter(|i| !i.paint().is_none()) {
                        s += &match it {
                            AppearanceItem::Fill(f) => format!(" fill({} op={})", paint(&f.paint), f.overprint),
                            AppearanceItem::Stroke(st) => format!(
                                " stroke({} w={} {:?} {:?} m={} {:?} op={})",
                                paint(&st.paint),
                                r3(st.width),
                                st.cap,
                                st.join,
                                r3(st.miter_limit),
                                st.dash.as_ref().map(|d| (d.pattern.clone(), d.offset)),
                                st.overprint
                            ),
                        };
                    }
                }
                out.push(s);
            }
            other => out.push(format!("{head} {other:?}")),
        }
    }
}

fn lines(doc: &Document) -> Vec<String> {
    let mut out = vec![];
    view(&doc.layers, 0, &mut out);
    out
}

fn swatches(doc: &Document) -> Vec<String> {
    let one = |s: &Swatch| format!("{} global={} spot={} {}", s.name, s.global, s.spot, paint(&s.paint));
    let mut out: Vec<String> = doc.swatches.iter().filter(|s| !matches!(s.paint, Paint::None)).map(one).collect();
    for g in doc.swatch_groups.iter() {
        out.push(format!("group {}", g.name));
        out.extend(g.swatches.iter().map(one));
    }
    out
}

#[test]
fn the_reader_reads_back_every_construct_of_the_slice() {
    let doc = sample();
    let back = read_back(&doc);
    assert_eq!(lines(&back), lines(&doc));
    assert_eq!(swatches(&back), swatches(&doc));
    let boards: Vec<(String, Rect)> = back.artboards.iter().map(|a| (a.name.clone(), a.rect)).collect();
    assert_eq!(
        boards,
        vec![("Artboard 1".to_string(), Rect::new(0.0, 0.0, 200.0, 100.0)), ("Back side".to_string(), Rect::new(250.0, 0.0, 450.0, 100.0))]
    );
    assert_eq!(back.color_mode, doc.color_mode);
    assert_eq!(back.units, doc.units);
}

#[test]
fn writing_again_gives_the_same_bytes() {
    let doc = sample();
    let first = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(first.program, export_ai_editing(&doc, &opts()).unwrap().program);
    assert_eq!(private_data(&doc, &opts()).unwrap().0, private_data(&doc, &opts()).unwrap().0);
    // What the reader reads back is written as the same program.
    let read = export_ai_editing(&read_back(&doc), &opts()).unwrap();
    assert_eq!(read.program, first.program);
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
}

#[test]
fn the_data_is_zstandard_compressed_with_the_header_as_metadata() {
    let doc = sample();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    let (p, _) = private_data(&doc, &opts()).unwrap();
    assert!(p.data.starts_with(ZSTD_MARKER));
    let mut body = p.data.get(ZSTD_MARKER.len()..).unwrap();
    let mut decoded = vec![];
    std::io::Read::read_to_end(&mut ruzstd::decoding::StreamingDecoder::new(&mut body).unwrap(), &mut decoded).unwrap();
    assert_eq!(decoded, out.program);
    assert!(out.program.starts_with(&out.header));
    assert_eq!(p.metadata, out.header);
    let header = String::from_utf8(out.header).unwrap();
    assert!(header.ends_with("%%EndComments\r\n"));
    let header = header.replace("\r\n", "\n");
    for line in [
        "%%AI8_CreatorVersion: 30.7.0\n%%For: () ()\n%%Title: (Sample)\n",
        "%%Canvassize: 16383\n",
        "%AI5_FileFormat 14.0\n%AI3_ColorUsage: Color\n%AI7_ImageSettings: 0\n%%DocumentCustomColors: (Gold)\n%%+ (Sky)\n%%+ (Mint)\n",
        "%AI3_Cropmarks: 0 -100 200 0\n%AI3_TemplateBox: 225.5 -50.5 225.5 -50.5\n%AI3_TileBox: 0 -100 200 0\n",
        "%AI5_ArtSize: 14400 14400\n",
        "%AI9_ColorModel: 1\n%AI5_ArtFlags: 0 0 0 1 0 0 1 0 0\n%AI5_TargetResolution: 800\n%AI5_NumLayers: 2\n",
        "%AI5_OpenViewLayers: 77\n%AI7_GridSettings: 72 8 72 8 1 0 0.7843 0.7843 0.7843 0.7843 0.7843 0.7843\n%%EndComments\n",
    ] {
        assert!(header.contains(line), "{line}\n{header}");
    }
    assert_eq!((p.container_version, p.creator_version, p.roundtrip_version, p.roundtrip_stream_type), (12, 30, 24, 2));
}

/// Art space is ruler space: y up from the first artboard's top-left corner. The artboards
/// together sit centered on the canvas: each one's top-left corner on the canvas (y down) is its
/// `RulerOrigin`, and the template box holds the canvas's center in art space.
#[test]
fn art_lies_on_its_artboard_in_ruler_space() {
    let mut doc = Document::new(200.0, 100.0);
    let mut second = doc.artboards[0].clone();
    (second.id, second.name, second.rect) = (2, "Back".into(), Rect::new(250.0, 0.0, 450.0, 100.0));
    doc.artboards.push(second);
    let rect = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(10.0, 20.0, 30.0, 40.0)),
        Appearance::basic(solid(Color::gray(0.5)), Paint::None, 0.0),
    );
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, rect).unwrap();
    let text = program(&doc);
    for line in [
        "%%BoundingBox: 10 -40 30 -20\n",
        "%AI3_Cropmarks: 0 -100 200 0\n",
        "%AI3_TemplateBox: 225.5 -50.5 225.5 -50.5\n",
        "\n10 -20 m\n",
        "%_0 0 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_200 -100 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_7966 8141 /RealPoint\n%_ (RulerOrigin) ,\n%_1 /Real (PAR) ,\n%_0 /Int (DisplayMark) ,\n%_1 /Bool (IsArtboardSelected) ,\n",
        "%_250 0 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_450 -100 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_8216 8141 /RealPoint\n%_ (RulerOrigin) ,\n%_1 /Real (PAR) ,\n%_0 /Int (DisplayMark) ,\n%_0 /Bool (IsArtboardSelected) ,\n",
        "%_16383 /Int (AIDocumentCanvasSize) ,\n",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    // The creator names the format's app, before the canvas size and the template box, and holds
    // no version number another app could read as the format's version.
    let creator = text.lines().find_map(|l| l.strip_prefix("%%Creator: ")).unwrap();
    assert!(creator.contains("Illustrator"), "{creator}");
    assert!(!creator.contains("88") && !creator.chars().any(|c| c.is_ascii_digit()), "{creator}");
    assert!(text.find("%%Creator: ").unwrap() < text.find("%%Canvassize: 16383").unwrap());
    assert!(text.find("%%Canvassize: 16383").unwrap() < text.find("%AI3_TemplateBox").unwrap());
    // The reader puts the art back where it was, on its artboard.
    let read = read_back(&doc);
    assert_eq!(read.artboards.iter().map(|a| a.rect).collect::<Vec<_>>(), doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>());
    let bounds = read.layers.iter().filter_map(|l| l.visual_bounds()).reduce(|a, b| a.union(b)).unwrap();
    assert_eq!(bounds, Rect::new(10.0, 20.0, 30.0, 40.0));
}

/// A gray rectangle at `r` (document space) in `doc`'s first layer.
fn add_rect(doc: &mut Document, r: Rect) {
    let rect = Node::path(doc.alloc_id(), shapes::rectangle(r), Appearance::basic(solid(Color::gray(0.5)), Paint::None, 0.0));
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, rect).unwrap();
}

/// The end of the document data: the artboards and the canvas size in the `/Recorded` part of the
/// `/Document` dictionary, then an empty `/NotRecorded` part.
const RECORDED: &str = "%_; (ArtboardArray) ,\n%_16383 /Int (AIDocumentCanvasSize) ,\n%_; /Recorded ,\n%_/Dictionary : /NotRecorded ,\n%_; /NotRecorded ,\n%_;\n%AI9_EndDocumentData\n";

/// One letter-size artboard: the art 36 points in from its top-left corner, the artboard's record
/// and the boxes, with the artboards in the `/Recorded` part of the `/Document` dictionary.
#[test]
fn one_artboard_is_recorded_with_its_art_in_place() {
    let mut doc = Document::new(612.0, 792.0);
    doc.artboards[0].name = "Page".into();
    add_rect(&mut doc, Rect::new(36.0, 36.0, 576.0, 756.0));
    let text = program(&doc);
    for line in [
        "%%BoundingBox: 36 -756 576 -36\n",
        "%AI3_Cropmarks: 0 -792 612 0\n",
        "%AI3_TemplateBox: 306.5 -396.5 306.5 -396.5\n",
        "\n36 -36 m\n",
        "%_0 0 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_612 -792 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_7885 7795 /RealPoint\n%_ (RulerOrigin) ,\n",
        "%_(Page) /UnicodeString (Name) ,\n%_; ,\n",
        RECORDED,
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    let read = read_back(&doc);
    assert_eq!(read.artboards.iter().map(|a| (a.name.as_str(), a.rect)).collect::<Vec<_>>(), [("Page", Rect::new(0.0, 0.0, 612.0, 792.0))]);
    let bounds = read.layers.iter().filter_map(|l| l.visual_bounds()).reduce(|a, b| a.union(b)).unwrap();
    assert_eq!(bounds, Rect::new(36.0, 36.0, 576.0, 756.0));
}

/// Two named artboards of different sizes, tops aligned: each record holds its corners in art
/// space and its top-left corner on the canvas, where the two together are centered.
#[test]
fn artboards_of_different_sizes_keep_their_names_and_places() {
    let mut doc = Document::new(400.0, 300.0);
    doc.artboards[0].name = "Front".into();
    let mut back = doc.artboards[0].clone();
    (back.id, back.name, back.rect) = (2, "Back".into(), Rect::new(440.0, 0.0, 740.0, 500.0));
    doc.artboards.push(back);
    add_rect(&mut doc, Rect::new(50.0, 50.0, 150.0, 100.0));
    add_rect(&mut doc, Rect::new(490.0, 50.0, 590.0, 100.0));
    let text = program(&doc);
    for line in [
        "%AI3_Cropmarks: 0 -300 400 0\n",
        "%AI3_TemplateBox: 370.5 -250.5 370.5 -250.5\n",
        "\n50 -50 m\n",
        "\n490 -50 m\n",
        "%_0 0 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_400 -300 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_7821 7941 /RealPoint\n%_ (RulerOrigin) ,\n",
        "%_1 /Bool (IsArtboardSelected) ,\n%_(Front) /UnicodeString (Name) ,\n",
        "%_440 0 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_740 -500 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_8261 7941 /RealPoint\n%_ (RulerOrigin) ,\n",
        "%_0 /Bool (IsArtboardSelected) ,\n%_(Back) /UnicodeString (Name) ,\n",
        RECORDED,
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    let read = read_back(&doc);
    assert_eq!(
        read.artboards.iter().map(|a| (a.name.as_str(), a.rect)).collect::<Vec<_>>(),
        [("Front", Rect::new(0.0, 0.0, 400.0, 300.0)), ("Back", Rect::new(440.0, 0.0, 740.0, 500.0))]
    );
    let bounds = read.layers.iter().filter_map(|l| l.visual_bounds()).reduce(|a, b| a.union(b)).unwrap();
    assert_eq!(bounds, Rect::new(50.0, 50.0, 590.0, 100.0));
}

#[test]
fn a_pdf_carries_the_data_on_its_first_page() {
    let doc = sample();
    let (p, _) = private_data(&doc, &opts()).unwrap();
    let pdf =
        vectorcraft_pdf::export(&doc, &vectorcraft_pdf::PdfOptions { private_data: Some(p.clone()), ..vectorcraft_pdf::PdfOptions::uncompressed() })
            .unwrap();
    assert_eq!(vectorcraft_pdf::illustrator_data(&pdf, None).as_deref(), Some(&p.data[..]));
    let (read, _) = crate::ai_alone(&vectorcraft_pdf::illustrator_data(&pdf, None).unwrap()).unwrap();
    assert_eq!(lines(&read), lines(&doc));
}

#[test]
fn names_are_xml_ids_numbered_when_taken() {
    let text = program(&sample());
    assert!(text.contains("%_/XMLUID : (Red) ; (AI10_ArtUID) ,"));
    assert!(text.contains("%_/XMLUID : (Red_00000000000000000001_) ; (AI10_ArtUID) ,"));
    assert!(text.contains("%_/XMLUID : (Pair) ; (AI10_ArtUID) ,"));
}

#[test]
fn many_objects_of_one_name_are_numbered_in_turn() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    for _ in 0..20_000 {
        let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default());
        n.name = Some("Dot".into());
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let text = program(&doc);
    let ids: std::collections::HashSet<&str> = text.lines().filter(|l| l.starts_with("%_/XMLUID : (Dot")).collect();
    assert_eq!(ids.len(), 20_000);
    assert!(text.contains("(Dot_00000000000000019999_)"));
}

#[test]
fn layers_have_fourteen_operands() {
    let text = program(&sample());
    assert!(text.contains("%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(Back) Ln\n"), "{text}");
    assert!(text.contains("%AI5_BeginLayer\n0 1 0 0 1 0 1 -1 10 200 30 0 70 0 Lb\n(Sub) Ln\n"), "{text}");
}

#[test]
fn layers_are_named_as_xml_ids_after_their_names() {
    let mut doc = sample();
    let layer = doc.layers[0].id;
    let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default());
    n.name = Some("Back".into());
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let text = program(&doc);
    assert!(text.contains("(Back) Ln\n%_/ArtDictionary :\n%_/XMLUID : (Back) ; (AI10_ArtUID) ,\n%_;\n%_\n"), "{text}");
    assert!(text.contains("(Sub) Ln\n%_/ArtDictionary :\n%_/XMLUID : (Sub) ; (AI10_ArtUID) ,\n"), "{text}");
    // An object named as a layer is numbered.
    assert!(text.contains("%_/XMLUID : (Back_00000000000000000001_) ; (AI10_ArtUID) ,"), "{text}");
    // The reader keeps the names where they were.
    let read = read_back(&doc);
    assert_eq!(read.layers[0].name.as_deref(), Some("Back"));
    let names: Vec<Option<&str>> = read.layers[0].children().unwrap().iter().map(|c| c.name.as_deref()).collect();
    assert_eq!(names.last(), Some(&Some("Back")));
}

#[test]
fn lines_end_in_cr_lf() {
    let mut doc = sample();
    doc.metadata.author = "A\nB".into();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    let text = String::from_utf8(out.program).unwrap();
    assert_eq!(text.matches('\n').count(), text.matches("\r\n").count());
    assert_eq!(text.matches('\r').count(), text.matches("\r\n").count());
    // A line break in a string stays in the string, escaped.
    assert!(text.contains("%%For: (A\\nB) ()\r\n"), "{text}");
}

#[test]
fn the_palette_starts_with_registration() {
    let mut doc = sample();
    let rgb = program(&doc);
    assert!(rgb.contains("0 0 Pb\n"), "{rgb}");
    let cell = rgb.split("0 0 Pb\n").nth(1).unwrap().lines().take(2).collect::<Vec<_>>();
    assert!(cell[0].ends_with(" 0 0 0 ([Registration]) 0 1 Xz") && cell[0].split(' ').count() == 11, "{cell:?}");
    assert_eq!(cell[1], "([Registration]) Pc");
    doc.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    assert!(program(&doc).contains("0 0 Pb\n1 1 1 1 ([Registration]) 0 Xs\n([Registration]) Pc\n"));
    // It's the built-in swatch: reading back adds no swatch.
    assert_eq!(swatches(&read_back(&doc)), swatches(&doc));
}

#[test]
fn dotted_strokes_are_written_as_their_dots_with_a_warning() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut stroke = StrokeLayer::new(solid(Color::rgb(0.0, 0.0, 0.0)), 4.0);
    stroke.cap = LineCap::Round;
    stroke.dash = Some(Dash { pattern: vec![0.0, 10.0], offset: 0.0, align_corners: false });
    let line = PathData::from_bezpath(&kurbo::BezPath::from_vec(vec![
        kurbo::PathEl::MoveTo((10.0, 50.0).into()),
        kurbo::PathEl::LineTo((60.0, 50.0).into()),
    ]));
    let n = Node::path(doc.alloc_id(), line, Appearance { items: vec![AppearanceItem::Stroke(stroke)], ..Appearance::default() });
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("dots")), "{:?}", out.warnings);
    let text = program(&doc);
    assert!(!text.contains("[0 10] 0 d"), "{text}");
    assert!(text.matches("\nf\n").count() >= 5, "a filled dot for each dash: {text}");
}

#[test]
fn hidden_layers_open_hidden() {
    let mut doc = sample();
    let mut hidden = Node::layer(doc.alloc_id(), "Off", LayerColor::Preset(1));
    hidden.visible = false;
    doc.layers.push(Arc::new(hidden));
    let text = program(&doc);
    assert!(text.contains("%AI5_NumLayers: 3\n%AI5_OpenViewLayers: 776\n"), "{text}");
}

/// A 200 × 100 document of what slice 1 writes as plain paths: a gradient fill, a clipping group,
/// a compound path, an image, type, a symbol instance and a half-transparent square.
fn unsupported() -> Document {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let fill = |p: Paint| Appearance::basic(p, Paint::None, 0.0);
    let g = Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, Color::rgb(1.0, 1.0, 1.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 0.0))],
    );
    let shaded =
        Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), fill(Paint::Gradient(Box::new(GradientPaint::new(g)))));
    doc.insert(Some(layer), usize::MAX, shaded).unwrap();
    let clip = Node::new(
        doc.alloc_id(),
        NodeKind::Path {
            path: shapes::rectangle(Rect::new(40.0, 10.0, 50.0, 30.0)),
            rule: FillRule::NonZero,
            live: None,
            clipping: true,
            guide: false,
        },
    );
    let clipped = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(35.0, 15.0, 60.0, 25.0)),
        Appearance::basic(solid(Color::gray(0.5)), solid(Color::BLACK), 1.0),
    );
    let mut window = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(clipped)], clip: true });
    window.name = Some("Window".into());
    doc.insert(Some(layer), usize::MAX, window).unwrap();
    let outer = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(70.0, 10.0, 90.0, 30.0)), Appearance::default());
    let inner = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(75.0, 15.0, 85.0, 25.0)), Appearance::default());
    let mut ring = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(inner)], rule: FillRule::EvenOdd });
    ring.appearance = fill(solid(Color::cmyk(0.0, 1.0, 1.0, 0.0))).into();
    doc.insert(Some(layer), usize::MAX, ring).unwrap();
    let mut png = vec![];
    let img = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let blob = ImageBlob::new("image/png", png);
    let key = blob.content_key();
    doc.images.insert(key.clone(), blob);
    let im = ImageObject {
        key,
        width: 2,
        height: 1,
        xf: Affine::new([10.0, 0.0, 0.0, 10.0, 100.0, 10.0]),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let im = Node::new(doc.alloc_id(), NodeKind::Image(Box::new(im)));
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    let t = vectorcraft_doc::TextObject::point(
        vectorcraft_geom::Point::new(10.0, 70.0),
        "Type",
        vectorcraft_doc::CharStyle { size: 18.0, fill: solid(Color::BLACK), ..vectorcraft_doc::CharStyle::default() },
    );
    let t = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    doc.insert(Some(layer), usize::MAX, t).unwrap();
    let art = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), fill(solid(Color::rgb(0.0, 1.0, 0.0))));
    doc.symbols.push(Symbol { name: "Dot".into(), art: Arc::new(art) });
    let instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((150.0, 10.0)) });
    doc.insert(Some(layer), usize::MAX, instance).unwrap();
    let mut faded = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(170.0, 10.0, 190.0, 30.0)), fill(solid(Color::BLACK)));
    faded.opacity = 0.5;
    doc.insert(Some(layer), usize::MAX, faded).unwrap();
    doc
}

/// Every path in `nodes` and their descendants.
fn paths(nodes: &[Arc<Node>], out: &mut Vec<Node>) {
    for n in nodes {
        match n.children() {
            Some(c) => paths(c, out),
            None => out.push((**n).clone()),
        }
    }
}

#[test]
fn art_the_slice_does_not_cover_is_written_as_plain_paths() {
    let doc = unsupported();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    for w in [writer::GRADIENTS, writer::CLIPPING, writer::IMAGES, writer::TYPE, writer::LIVE, writer::TRANSPARENCY] {
        assert!(out.warnings.iter().any(|x| x == w), "{w}: {:?}", out.warnings);
    }
    let back = read_back(&doc);
    let layer = back.layers[0].children().unwrap();
    let kinds: Vec<&str> = layer
        .iter()
        .map(|n| match n.kind {
            NodeKind::Path { .. } => "path",
            NodeKind::Group { .. } => "group",
            NodeKind::Compound { .. } => "compound",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["path", "group", "compound", "path", "group", "group", "path"]);
    let mut all = vec![];
    paths(layer, &mut all);
    assert!(all.iter().all(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    // The gradient as its average color.
    assert_eq!(paint(back.layers[0].children().unwrap()[0].appearance.items[0].paint()), paint(&solid(Color::rgb(0.5, 0.5, 0.5))));
    // The clipping group: the gray fill and the black stroke's outline, cut to the 10 × 20 clip.
    let window = &layer[1];
    assert_eq!(window.name.as_deref(), Some("Window"));
    let b = window.visual_bounds().unwrap();
    assert!((b.x0 - 40.0).abs() < 0.01 && (b.x1 - 50.0).abs() < 0.01, "{b:?}");
    // The image's outline filled with its pixels' average.
    assert_eq!(paint(layer[3].appearance.items[0].paint()), paint(&solid(Color::rgb(0.5, 0.0, 0.5))));
    assert_eq!(layer[3].visual_bounds().map(|b| (r3(b.x0), r3(b.y0), r3(b.x1), r3(b.y1))), Some((r3(100.0), r3(10.0), r3(120.0), r3(20.0))));
    // The half-transparent square, opaque.
    assert_eq!(layer[6].opacity, 1.0);
}

#[test]
fn brushed_strokes_are_written_as_their_brush_art() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut line = Node::path(
        doc.alloc_id(),
        shapes::line((20.0, 50.0).into(), (180.0, 50.0).into()),
        Appearance::basic(Paint::None, solid(Color::rgb(1.0, 0.0, 0.0)), 4.0),
    );
    line.appearance.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    doc.insert(Some(layer), usize::MAX, line).unwrap();
    let pieces = vectorcraft_brush::stroke_pieces(
        &doc,
        &vectorcraft_brush::find(&doc, "3 pt. Round").unwrap(),
        &shapes::line((20.0, 50.0).into(), (180.0, 50.0).into()).to_bezpath(),
        doc.layers[0].children().unwrap()[0].appearance.stroke().unwrap(),
    );
    let back = read_back(&doc);
    let NodeKind::Group { children, .. } = &back.layers[0].children().unwrap()[0].kind else { panic!("a group of the brush's art") };
    assert_eq!(children.len(), pieces.len());
    let bounds = |n: &Node| n.visual_bounds().map(|b| [b.x0, b.y0, b.x1, b.y1].map(|v| (v * 10.0).round()));
    assert_eq!(children.iter().map(|c| bounds(c)).collect::<Vec<_>>(), pieces.iter().map(bounds).collect::<Vec<_>>());
}

#[test]
fn type_is_written_as_create_outlines_makes_it() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let style = vectorcraft_doc::CharStyle { size: 18.0, fill: solid(Color::BLACK), underline: true, ..vectorcraft_doc::CharStyle::default() };
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 70.0), "Under", style);
    let mut t = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    t.name = Some("Title".into());
    doc.insert(Some(layer), usize::MAX, t.clone()).unwrap();
    let outlined = vectorcraft_effects::outline_text(&t).unwrap();
    let back = read_back(&doc);
    let group = &back.layers[0].children().unwrap()[0];
    assert_eq!(group.name.as_deref(), Some("Title"));
    let rounded = |n: &Node| n.visual_bounds().map(|b| [b.x0, b.y0, b.x1, b.y1].map(|v| (v * 100.0).round()));
    // The underline reaches below the glyphs: the bounds match only with it.
    assert_eq!(rounded(group), rounded(&outlined));
}

#[test]
fn inline_graphics_in_type_are_written_as_their_art() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mark = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)),
        Appearance::basic(solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0),
    );
    doc.symbols.push(Symbol { name: "Mark".into(), art: Arc::new(mark) });
    let style = vectorcraft_doc::CharStyle { size: 18.0, fill: solid(Color::BLACK), ..vectorcraft_doc::CharStyle::default() };
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 70.0), "A", style.clone());
    t.runs.push(vectorcraft_doc::TextRun::inline(vectorcraft_doc::InlineArt::new("Mark"), style));
    let mut text = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    // A stroke of the object's own above the Characters row: drawn over the inline graphic.
    let red = solid(Color::rgb(1.0, 0.0, 0.0));
    text.appearance =
        Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(red.clone(), 3.0))], contents_index: Some(0), ..Appearance::default() }
            .into();
    doc.insert(Some(layer), usize::MAX, text).unwrap();
    let back = read_back(&doc);
    let mut all = vec![];
    paths(back.layers[0].children().unwrap(), &mut all);
    let has = |p: &Node, c: &Paint| p.appearance.items.iter().any(|i| paint(i.paint()) == paint(c));
    let blue = solid(Color::rgb(0.0, 0.0, 1.0));
    let art: Vec<&Node> = all.iter().filter(|p| has(p, &blue)).collect();
    assert_eq!(art.len(), 1, "the inline graphic is written");
    // To the right of the letter, 18 pt high as the run's size makes it.
    let b = art[0].visual_bounds().unwrap();
    assert!(b.x0 > 10.0 && (b.height() - 18.0).abs() < 0.01, "{b:?}");
    let at = |c: &Paint| all.iter().position(|p| has(p, c)).unwrap();
    assert!(at(&blue) < at(&red), "the stroke above the characters is over the inline graphic");
}

/// Nested symbols whose instances multiply stop at [`vectorcraft_doc::MAX_SYMBOL_NODES`] objects of
/// symbol art in all, with a warning.
#[test]
fn nested_symbol_instances_stop_at_the_symbol_art_limit() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    // Empty paths: counted, but written as nothing, so the test stays small; and one filled square.
    let empty = Arc::new(Node::path(doc.alloc_id(), PathData::new(vec![]), Appearance::default()));
    let mut members = vec![empty; 4999];
    let square = shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0));
    members.push(Arc::new(Node::path(doc.alloc_id(), square, Appearance::basic(solid(Color::gray(0.5)), Paint::None, 0.0))));
    let leaves = Node::group(doc.alloc_id(), members);
    doc.symbols.push(Symbol { name: "Leaf".into(), art: Arc::new(leaves) });
    let leaf = Arc::new(Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Leaf".into(), xf: Affine::IDENTITY }));
    let trees = Node::group(doc.alloc_id(), vec![leaf; 5000]);
    doc.symbols.push(Symbol { name: "Tree".into(), art: Arc::new(trees) });
    let tree = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Tree".into(), xf: Affine::IDENTITY });
    doc.insert(Some(layer), usize::MAX, tree).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::SYMBOL_LIMIT), "{:?}", out.warnings);
    // 5,001 objects for the tree and each leaf: 3,198 leaves fit in 16,000,000.
    let text = String::from_utf8(out.program).unwrap();
    assert_eq!(text.matches("\r\nf\r\n").count(), 3198, "the squares of the leaves written");
}

#[test]
fn a_stained_symbol_instance_is_written_in_its_stained_colors() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let art = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)),
        Appearance::basic(solid(Color::rgb(0.0, 1.0, 0.0)), Paint::None, 0.0),
    );
    doc.symbols.push(Symbol { name: "Dot".into(), art: Arc::new(art) });
    let mut instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((50.0, 10.0)) });
    instance.appearance = Appearance::basic(solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0).into();
    let stained = vectorcraft_brush::instance_art(&doc.symbols[0].art, &instance);
    doc.insert(Some(layer), usize::MAX, instance).unwrap();
    let back = read_back(&doc);
    let NodeKind::Group { children, .. } = &back.layers[0].children().unwrap()[0].kind else { panic!("the instance's group") };
    assert_eq!(paint(children[0].appearance.items[0].paint()), paint(stained.appearance.items[0].paint()));
    assert_ne!(paint(stained.appearance.items[0].paint()), paint(&solid(Color::rgb(0.0, 1.0, 0.0))), "the instance stains its art");
}

#[test]
fn a_layer_with_crop_marks_keeps_its_art_and_gets_the_marks() {
    for clipping in [false, true] {
        let mut doc = Document::new(200.0, 100.0);
        let layer = doc.layers[0].id;
        let at = Rect::new(50.0, 30.0, 90.0, 60.0);
        if clipping {
            let clip = Node::new(
                doc.alloc_id(),
                NodeKind::Path { path: shapes::rectangle(at), rule: FillRule::NonZero, live: None, clipping: true, guide: false },
            );
            doc.insert(Some(layer), usize::MAX, clip).unwrap();
        }
        let square = Node::path(doc.alloc_id(), shapes::rectangle(at), Appearance::basic(solid(Color::BLACK), Paint::None, 0.0));
        doc.insert(Some(layer), usize::MAX, square).unwrap();
        if let Some(l) = doc.layers.get_mut(0) {
            let fx = vectorcraft_doc::Effect { id: vectorcraft_effects::CROP_MARKS.into(), params: serde_json::json!({}), visible: true };
            let l = Arc::make_mut(l);
            let mut ap = (*l.appearance).clone();
            ap.effects.push(fx);
            l.appearance = ap.into();
            if let NodeKind::Layer { clip, .. } = &mut l.kind {
                *clip = clipping;
            }
        }
        let name = doc.layers[0].name.clone();
        let back = read_back(&doc);
        assert_eq!(back.layers.len(), 1);
        assert_eq!(back.layers[0].name, name);
        let members = back.layers[0].children().unwrap();
        assert_eq!(members.len(), 2, "clipping {clipping}: the square and the marks");
        assert!(matches!(members[0].kind, NodeKind::Path { .. }));
        assert!(matches!(members[1].kind, NodeKind::Group { .. }));
        // The marks lie around the square, outside a clipping path.
        let marks = members[1].visual_bounds().unwrap();
        assert!(marks.x0 < at.x0 - 1.0 && marks.x1 > at.x1 + 1.0, "clipping {clipping}: {marks:?}");
    }
}

#[test]
fn geometry_effects_in_a_symbol_are_written_as_their_result() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut ap = Appearance::basic(solid(Color::BLACK), Paint::None, 0.0);
    ap.effects.push(vectorcraft_doc::Effect { id: "path.offsetPath".into(), params: serde_json::json!({"offset": 5}), visible: true });
    let art = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), ap);
    doc.symbols.push(Symbol { name: "Dot".into(), art: Arc::new(art) });
    let instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((50.0, 20.0)) });
    doc.insert(Some(layer), usize::MAX, instance).unwrap();
    let back = read_back(&doc);
    let b = back.layers[0].children().unwrap()[0].visual_bounds().unwrap();
    // The 10 pt square offset by 5 pt, at (50, 20).
    assert!((b.x0 - 45.0).abs() < 0.1 && (b.x1 - 65.0).abs() < 0.1 && (b.y0 - 15.0).abs() < 0.1 && (b.y1 - 35.0).abs() < 0.1, "{b:?}");
}

#[test]
fn an_adjusted_image_in_a_symbol_is_written_in_its_adjusted_color() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut png = vec![];
    image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let blob = ImageBlob::new("image/png", png);
    let key = blob.content_key();
    doc.images.insert(key.clone(), blob);
    let im = ImageObject { key, width: 2, height: 2, xf: Affine::scale(5.0), link: None, placement: Default::default(), pixels: None };
    let mut im = Node::new(doc.alloc_id(), NodeKind::Image(Box::new(im)));
    let shift = serde_json::json!({"amount": 100, "preserveLightness": false});
    let mut ap = (*im.appearance).clone();
    ap.effects.push(vectorcraft_doc::Effect { id: "adjust.shiftToColor".into(), params: shift, visible: true });
    im.appearance = ap.into();
    doc.symbols.push(Symbol { name: "Picture".into(), art: Arc::new(im) });
    let instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Picture".into(), xf: Affine::translate((50.0, 20.0)) });
    doc.insert(Some(layer), usize::MAX, instance).unwrap();
    let back = read_back(&doc);
    let mut all = vec![];
    paths(back.layers[0].children().unwrap(), &mut all);
    let fill = paint(all[0].appearance.items[0].paint());
    assert_ne!(fill, paint(&solid(Color::gray(0.5))), "the adjusted image is found");
    assert_ne!(fill, paint(&solid(Color::rgb(0.0, 0.0, 0.0))), "the image is adjusted");
}

#[test]
fn a_group_fill_in_a_pattern_tile_is_written() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let dot = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default());
    let mut group = Node::group(doc.alloc_id(), vec![Arc::new(dot)]);
    group.appearance = Appearance::basic(solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0).into();
    doc.patterns.push(vectorcraft_doc::PatternDef::new("Dots", vec![Arc::new(group)]));
    let filled = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(20.0, 20.0, 40.0, 40.0)),
        Appearance::basic(Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }, Paint::None, 0.0),
    );
    doc.insert(Some(layer), usize::MAX, filled).unwrap();
    let back = read_back(&doc);
    let mut all = vec![];
    paths(back.layers[0].children().unwrap(), &mut all);
    let red = paint(&solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(all.iter().any(|p| p.appearance.items.iter().any(|i| paint(i.paint()) == red)), "the tiles keep their group's fill");
}

#[test]
fn effects_on_a_fill_are_reported_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut fill = FillLayer::new(solid(Color::BLACK));
    fill.effects.push(vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: serde_json::json!({}), visible: true });
    let ap = Appearance { items: vec![AppearanceItem::Fill(fill)], ..Appearance::default() };
    let square = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), ap);
    doc.insert(Some(layer), usize::MAX, square).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
}

#[test]
fn images_too_large_to_average_are_mid_gray() {
    let mut png = vec![];
    image::GrayImage::new(5000, 4000).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    assert_eq!(image_average_of(&png), paint(&solid(Color::gray(0.5))));
    let mut small = vec![];
    image::GrayImage::new(50, 40).write_to(&mut std::io::Cursor::new(&mut small), image::ImageFormat::Png).unwrap();
    assert_eq!(image_average_of(&small), paint(&solid(Color::rgb(0.0, 0.0, 0.0))));
}

/// The fill an image of `png` is written with, as the reader reads it.
fn image_average_of(png: &[u8]) -> String {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let blob = ImageBlob::new("image/png", png.to_vec());
    let key = blob.content_key();
    doc.images.insert(key.clone(), blob);
    let im = ImageObject { key, width: 10, height: 10, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    let im = Node::new(doc.alloc_id(), NodeKind::Image(Box::new(im)));
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    paint(read_back(&doc).layers[0].children().unwrap()[0].appearance.items[0].paint())
}

#[test]
fn a_document_without_an_artboard_is_an_error() {
    let mut doc = Document::new(200.0, 100.0);
    doc.artboards.clear();
    assert!(export_ai_editing(&doc, &opts()).is_err());
    doc.artboards.push(Artboard { rect: Rect::new(0.0, 0.0, 0.0, 10.0), ..Document::new(1.0, 1.0).artboards[0].clone() });
    assert!(export_ai_editing(&doc, &opts()).is_err());
}
