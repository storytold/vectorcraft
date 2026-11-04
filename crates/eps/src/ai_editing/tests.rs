//! The editing data written, then read back by VectorCraft's own reader ([`crate::ai_alone`]).

use std::sync::Arc;

use vectorcraft_color::swatch::{Swatch, SwatchGroup};
use vectorcraft_color::{BlendMode, Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, Dash, Document, FillLayer, ImageBlob, ImageObject, Knockout, LayerColor, LineCap, LineJoin, Node, NodeKind,
    OpacityMask, StrokeLayer, Symbol,
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
            // The compound path's own paint and its members' points (the paint is the compound path's).
            NodeKind::Compound { rule, .. } => {
                let mut s = format!("{head} compound {rule:?}");
                for c in n.children().map(Vec::as_slice).unwrap_or_default() {
                    for sp in c.path_data().map(|p| p.subpaths.as_slice()).unwrap_or_default() {
                        let r2 = |v: f64| format!("{:.2}", v + 0.0);
                        s += &format!(" [closed={}", sp.closed);
                        for a in &sp.anchors {
                            s += &format!(" {},{}", r2(a.p.x), r2(a.p.y));
                        }
                        s += "]";
                    }
                }
                for it in n.appearance.items.iter().filter(|i| !i.paint().is_none()) {
                    s += &format!(" {}", paint(it.paint()));
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
    // no version number (the version is written in `%%AI8_CreatorVersion`).
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

/// An artboard larger than the canvas (20,000 points square): the data is written on a large
/// canvas, at 1/10 of the size, so the artboard lies on the canvas; read beside a page of the
/// artboard's size in points, it comes back at its size.
#[test]
fn an_artboard_larger_than_the_canvas_is_written_on_a_large_canvas() {
    let mut doc = Document::new(20_000.0, 20_000.0);
    add_rect(&mut doc, Rect::new(1000.0, 1000.0, 3000.0, 2000.0));
    let text = program(&doc);
    for line in [
        "%AI24_LargeCanvasScale: 10\n",
        "%AI3_Cropmarks: 0 -2000 2000 0\n",
        "\n100 -100 m\n",
        "%_0 0 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_2000 -2000 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_7191 7191 /RealPoint\n%_ (RulerOrigin) ,\n",
        RECORDED,
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    // A page of the artboard's size in points, as a PDF part in larger units gives it.
    let mut page = Document::new(20_000.0, 20_000.0);
    add_rect(&mut page, Rect::new(1000.0, 1000.0, 3000.0, 2000.0));
    let (p, _) = private_data(&doc, &opts()).unwrap();
    let (read, notes, layers) = crate::layered_ai(&p.data, page, vec![], false, true, 1.389);
    assert!(layers, "{notes:?}");
    assert_eq!(read.artboards.iter().map(|a| a.rect).collect::<Vec<_>>(), [Rect::new(0.0, 0.0, 20_000.0, 20_000.0)], "{notes:?}");
    let bounds = read.layers.iter().filter_map(|l| l.visual_bounds()).reduce(|a, b| a.union(b)).unwrap();
    assert!((bounds.x0 - 1000.0).abs() < 1e-6 && (bounds.y1 - 2000.0).abs() < 1e-6 && (bounds.width() - 2000.0).abs() < 1e-6, "{bounds:?}");
    // The same without a readable PDF part, on its first page's size.
    let (alone, notes) = crate::ai_alone_on_page(&p.data, (20_000.0, 20_000.0), 1.389).unwrap();
    assert_eq!(alone.artboards.iter().map(|a| a.rect).collect::<Vec<_>>(), [Rect::new(0.0, 0.0, 20_000.0, 20_000.0)], "{notes:?}");
    // And from the editing data alone, as a file saved without its PDF part opens.
    let alone = read_back(&doc);
    assert_eq!(alone.artboards.iter().map(|a| a.rect).collect::<Vec<_>>(), [Rect::new(0.0, 0.0, 20_000.0, 20_000.0)]);
    let bounds = alone.layers.iter().filter_map(|l| l.visual_bounds()).reduce(|a, b| a.union(b)).unwrap();
    assert!((bounds.x0 - 1000.0).abs() < 1e-6 && (bounds.width() - 2000.0).abs() < 1e-6, "{bounds:?}");

    // A symbol's 30-point stroke is written at 1/10 too, as the layers' strokes are.
    let mut doc = Document::new(20_000.0, 20_000.0);
    let line = shapes::rectangle(Rect::new(0.0, 0.0, 500.0, 500.0));
    let art = Node::path(doc.alloc_id(), line, Appearance::basic(Paint::None, solid(Color::gray(0.5)), 30.0));
    doc.symbols.push(Symbol { dynamic: false, name: "Box".into(), art: Arc::new(art) });
    let instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Box".into(), xf: Affine::translate((1000.0, 1000.0)) });
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, instance).unwrap();
    let (p, _) = private_data(&doc, &opts()).unwrap();
    let (read, notes, layers) = crate::layered_ai(&p.data, Document::new(20_000.0, 20_000.0), vec![], false, true, 1.389);
    assert!(layers, "{notes:?}");
    let mut all = vec![];
    paths(read.layers[0].children().unwrap(), &mut all);
    let widths: Vec<f64> = all.iter().filter_map(|n| n.appearance.stroke().map(|s| s.width)).collect();
    assert!(!widths.is_empty() && widths.iter().all(|w| (w - 30.0).abs() < 1e-6), "{widths:?}");

    // Artboards that fit stay on the canvas at their size.
    assert!(program(&Document::new(16_000.0, 16_000.0)).contains("%AI24_LargeCanvasScale: 1\n"));
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

/// A 200 × 100 document of what the data holds as plain paths: a gradient fill, a compound path,
/// an image, type and a symbol instance.
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
    doc.symbols.push(Symbol { dynamic: false, name: "Dot".into(), art: Arc::new(art) });
    let instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((150.0, 10.0)) });
    doc.insert(Some(layer), usize::MAX, instance).unwrap();
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
    for w in [writer::GRADIENTS, writer::IMAGES, writer::TYPE, writer::LIVE] {
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
    assert_eq!(kinds, ["path", "compound", "path", "group", "group"]);
    let mut all = vec![];
    paths(layer, &mut all);
    assert!(all.iter().all(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    // The gradient as its average color.
    assert_eq!(paint(back.layers[0].children().unwrap()[0].appearance.items[0].paint()), paint(&solid(Color::rgb(0.5, 0.5, 0.5))));
    // The image's outline filled with its pixels' average.
    assert_eq!(paint(layer[2].appearance.items[0].paint()), paint(&solid(Color::rgb(0.5, 0.0, 0.5))));
    assert_eq!(layer[2].visual_bounds().map(|b| (r3(b.x0), r3(b.y0), r3(b.x1), r3(b.y1))), Some((r3(100.0), r3(10.0), r3(120.0), r3(20.0))));
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
    doc.symbols.push(Symbol { dynamic: false, name: "Mark".into(), art: Arc::new(mark) });
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

/// A scaled instance of a symbol with 9-slice guides is written as the canvas draws it: the corner
/// keeps its size while the rest stretches.
#[test]
fn a_scaled_nine_slice_symbol_instance_keeps_its_corner_size() {
    let mut doc = Document::new(400.0, 400.0);
    let layer = doc.layers[0].id;
    let square = |doc: &mut Document, r: Rect| {
        Node::path(doc.alloc_id(), shapes::rectangle(r), Appearance::basic(solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0))
    };
    let frame = Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 100.0, 100.0)));
    let corner = Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0)));
    let art = Node::group(doc.alloc_id(), vec![frame, corner]);
    doc.symbols.push(Symbol { dynamic: false, name: "S".into(), art: Arc::new(art) });
    doc.set_symbol_nine_slice("S", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let xf = Affine::scale_non_uniform(2.0, 1.5);
    let instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf });
    doc.insert(Some(layer), usize::MAX, instance).unwrap();
    let back = read_back(&doc);
    let mut all = vec![];
    paths(back.layers[0].children().unwrap(), &mut all);
    let sizes: Vec<(i64, i64)> =
        all.iter().filter_map(|p| p.geometric_bounds()).map(|b| (b.width().round() as i64, b.height().round() as i64)).collect();
    assert!(sizes.contains(&(10, 10)), "the corner keeps its size: {sizes:?}");
    assert!(sizes.contains(&(200, 150)), "the frame is scaled: {sizes:?}");
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
    doc.symbols.push(Symbol { dynamic: false, name: "Leaf".into(), art: Arc::new(leaves) });
    let leaf = Arc::new(Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Leaf".into(), xf: Affine::IDENTITY }));
    let trees = Node::group(doc.alloc_id(), vec![leaf; 5000]);
    doc.symbols.push(Symbol { dynamic: false, name: "Tree".into(), art: Arc::new(trees) });
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
    doc.symbols.push(Symbol { dynamic: false, name: "Dot".into(), art: Arc::new(art) });
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
        // A clipping layer's art is a clipping group, outside which the marks lie.
        let expected =
            if clipping { matches!(members[0].kind, NodeKind::Group { clip: true, .. }) } else { matches!(members[0].kind, NodeKind::Path { .. }) };
        assert!(expected && matches!(back.layers[0].kind, NodeKind::Layer { clip: false, .. }), "clipping {clipping}");
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
    doc.symbols.push(Symbol { dynamic: false, name: "Dot".into(), art: Arc::new(art) });
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
    doc.symbols.push(Symbol { dynamic: false, name: "Picture".into(), art: Arc::new(im) });
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

/// A pattern fill that would place more tiles than a fill places is written as the shape filled
/// gray, as a pattern that isn't found is, instead of as nothing.
#[test]
fn a_pattern_fill_of_too_many_tiles_is_written_gray() {
    let mut doc = Document::new(1200.0, 1200.0);
    let layer = doc.layers[0].id;
    let dot = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 0.5, 0.5)), Appearance::basic(solid(Color::BLACK), Paint::None, 0.0));
    let tiny = vectorcraft_doc::PatternDef { tile: Rect::new(0.0, 0.0, 1.0, 1.0), ..vectorcraft_doc::PatternDef::new("Tiny", vec![Arc::new(dot)]) };
    doc.patterns.push(tiny);
    let filled = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(100.0, 100.0, 1100.0, 1100.0)),
        Appearance::basic(Paint::Pattern { pattern: "Tiny".into(), xf: Affine::IDENTITY }, Paint::None, 0.0),
    );
    doc.insert(Some(layer), usize::MAX, filled).unwrap();
    let back = read_back(&doc);
    let mut all = vec![];
    paths(back.layers[0].children().unwrap(), &mut all);
    let gray = paint(&solid(Color::gray(0.5)));
    assert!(all.iter().any(|p| p.appearance.items.iter().any(|i| paint(i.paint()) == gray)), "{}", all.len());
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PATTERN_TILES), "{:?}", out.warnings);
}

/// An artboard's center mark, cross hairs and video safe areas are written as the bits of its
/// `DisplayMark` and read back.
#[test]
fn artboard_display_marks_are_written() {
    let mut doc = Document::new(200.0, 100.0);
    (doc.artboards[0].show_center_mark, doc.artboards[0].show_video_safe_areas) = (true, true);
    assert!(program(&doc).contains("%_5 /Int (DisplayMark) ,\n"));
    let back = read_back(&doc);
    let ab = &back.artboards[0];
    assert!(ab.show_center_mark && !ab.show_cross_hairs && ab.show_video_safe_areas);
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

// ---------- transparency, compound paths, clipping, opacity masks ----------

/// A rectangle filled with `c`.
fn square(doc: &mut Document, r: Rect, c: Color) -> Node {
    Node::path(doc.alloc_id(), shapes::rectangle(r), Appearance::basic(solid(c), Paint::None, 0.0))
}

/// A clipping path of `path`, unpainted.
fn clipping(doc: &mut Document, path: PathData) -> Node {
    Node::new(doc.alloc_id(), NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: true, guide: false })
}

fn named(mut n: Node, name: &str) -> Node {
    n.name = Some(name.into());
    n
}

fn see_through(mut n: Node, opacity: f32, blend: BlendMode) -> Node {
    (n.opacity, n.blend) = (opacity, blend);
    n
}

fn group(doc: &mut Document, children: Vec<Node>, clip: bool) -> Node {
    Node::new(doc.alloc_id(), NodeKind::Group { children: children.into_iter().map(Arc::new).collect(), clip })
}

/// A 200 × 100 document of what the transparency, compound path and clipping forms hold. Layer
/// `Clear` (60%, Multiply, isolated) holds, bottom first: a path at 50% in Screen whose opacity
/// defines the knockout shape; a group at 40%, isolated, knockout on, whose opacity defines the
/// knockout shape, of a square and a square at 75% in Overlay; a group with knockout off; a group of
/// a square in each blending mode; a clipping group at 50%; a clipping group whose clipping path
/// is an even-odd compound path; a clipping group holding a clipping group; a named path with an
/// inverted opacity mask that doesn't clip; a group with a disabled opacity mask; and a compound
/// path at 70% in Screen whose opacity defines the knockout shape last. The layer's opacity
/// defines the knockout shape too. Layer `Cut` clips by its first path. Layer `Plain` holds an
/// opaque path.
fn transparent() -> Document {
    let mut doc = Document::new(200.0, 100.0);
    doc.swatches = vec![Swatch { name: "Blue".into(), paint: solid(Color::rgb(0.0, 0.0, 1.0)), global: false, spot: false }].into();
    doc.swatch_groups = vec![].into();
    doc.layers.clear();
    let clear = doc.add_layer(Some("Clear"));
    let cut = doc.add_layer(Some("Cut"));
    let plain = doc.add_layer(Some("Plain"));
    let red = Color::rgb(1.0, 0.0, 0.0);
    let blue = Color::rgb(0.0, 0.0, 1.0);
    let mut art = vec![];

    let mut shine = see_through(square(&mut doc, Rect::new(5.0, 5.0, 25.0, 25.0), red), 0.5, BlendMode::Screen);
    shine.knockout_shape = true;
    art.push(named(shine, "Shine"));
    let a = square(&mut doc, Rect::new(30.0, 5.0, 50.0, 25.0), red);
    let b = see_through(square(&mut doc, Rect::new(35.0, 10.0, 55.0, 30.0), blue), 0.75, BlendMode::Overlay);
    let mut knock = see_through(group(&mut doc, vec![a, b], false), 0.4, BlendMode::Normal);
    (knock.isolate, knock.knockout, knock.knockout_shape) = (true, Knockout::On, true);
    art.push(named(knock, "Knock"));
    let a = square(&mut doc, Rect::new(60.0, 5.0, 70.0, 15.0), red);
    let b = square(&mut doc, Rect::new(65.0, 10.0, 75.0, 20.0), blue);
    let mut off = group(&mut doc, vec![a, b], false);
    off.knockout = Knockout::Off;
    art.push(off);
    let modes: Vec<Node> = BlendMode::ALL
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let x = 5.0 + 11.0 * i as f64;
            see_through(square(&mut doc, Rect::new(x, 40.0, x + 10.0, 50.0), red), 0.5, *m)
        })
        .collect();
    art.push(group(&mut doc, modes, false));
    let clip = clipping(&mut doc, shapes::rectangle(Rect::new(90.0, 5.0, 100.0, 25.0)));
    let inside = square(&mut doc, Rect::new(85.0, 10.0, 110.0, 20.0), blue);
    art.push(named(see_through(group(&mut doc, vec![named(clip, "Edge"), inside], true), 0.5, BlendMode::Normal), "Window"));
    // The subpaths of a compound clipping path are clipping paths, as the reader marks them.
    let outer = clipping(&mut doc, shapes::rectangle(Rect::new(115.0, 5.0, 135.0, 25.0)));
    let inner = clipping(&mut doc, shapes::rectangle(Rect::new(120.0, 10.0, 130.0, 20.0)));
    let compound_clip = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(inner)], rule: FillRule::EvenOdd });
    let green = square(&mut doc, Rect::new(110.0, 0.0, 140.0, 30.0), Color::rgb(0.0, 1.0, 0.0));
    art.push(group(&mut doc, vec![compound_clip, green], true));
    let deep_clip = clipping(&mut doc, shapes::rectangle(Rect::new(145.0, 5.0, 160.0, 25.0)));
    let deep = square(&mut doc, Rect::new(140.0, 0.0, 170.0, 30.0), red);
    let deep = group(&mut doc, vec![deep_clip, deep], true);
    let outer_clip = clipping(&mut doc, shapes::rectangle(Rect::new(150.0, 0.0, 165.0, 20.0)));
    art.push(group(&mut doc, vec![outer_clip, deep], true));
    let gray = square(&mut doc, Rect::new(5.0, 60.0, 15.0, 70.0), Color::gray(0.5));
    let white = square(&mut doc, Rect::new(15.0, 60.0, 25.0, 70.0), Color::rgb(1.0, 1.0, 1.0));
    let mask_art = group(&mut doc, vec![gray, white], false);
    let mut masked = named(square(&mut doc, Rect::new(0.0, 55.0, 30.0, 75.0), blue), "Masked");
    masked.extras_mut().mask = Some(Box::new(OpacityMask { art: Arc::new(mask_art), clip: false, invert: true, disabled: false, linked: true }));
    art.push(masked);
    let off_art = square(&mut doc, Rect::new(40.0, 60.0, 50.0, 70.0), Color::gray(0.25));
    let in_faded = square(&mut doc, Rect::new(35.0, 55.0, 60.0, 75.0), red);
    let mut faded = group(&mut doc, vec![in_faded], false);
    faded.extras_mut().mask = Some(Box::new(OpacityMask { art: Arc::new(off_art), clip: true, invert: false, disabled: true, linked: false }));
    art.push(faded);
    let o = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(70.0, 55.0, 90.0, 75.0)), Appearance::default());
    let i = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(75.0, 60.0, 85.0, 70.0)), Appearance::default());
    let mut ring = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(o), Arc::new(i)], rule: FillRule::EvenOdd });
    ring.appearance = Appearance::basic(solid(red), Paint::None, 0.0).into();
    ring.knockout_shape = true;
    art.push(named(see_through(ring, 0.7, BlendMode::Screen), "Ring"));
    for n in art {
        doc.insert(Some(clear), usize::MAX, n).unwrap();
    }

    let edge = clipping(&mut doc, shapes::rectangle(Rect::new(100.0, 40.0, 150.0, 90.0)));
    doc.insert(Some(cut), usize::MAX, named(edge, "Edge")).unwrap();
    for (r, c) in [(Rect::new(90.0, 35.0, 130.0, 70.0), red), (Rect::new(120.0, 60.0, 160.0, 95.0), blue)] {
        let n = square(&mut doc, r, c);
        doc.insert(Some(cut), usize::MAX, n).unwrap();
    }
    let opaque = square(&mut doc, Rect::new(170.0, 60.0, 190.0, 80.0), blue);
    doc.insert(Some(plain), usize::MAX, opaque).unwrap();
    for (i, l) in doc.layers.iter_mut().enumerate() {
        let l = Arc::make_mut(l);
        match i {
            0 => {
                (l.opacity, l.blend, l.isolate, l.knockout_shape) = (0.6, BlendMode::Multiply, true, true);
            }
            1 => {
                if let NodeKind::Layer { clip, .. } = &mut l.kind {
                    *clip = true;
                }
            }
            _ => {}
        }
    }
    doc
}

/// The transparency, clipping and opacity masks of every object, one line each, the masks' art
/// after their objects.
fn looks(nodes: &[Arc<Node>], depth: usize, out: &mut Vec<String>) {
    for n in nodes {
        let kind = match &n.kind {
            NodeKind::Layer { clip, .. } => format!("layer clip={clip}"),
            NodeKind::Group { clip, .. } => format!("group clip={clip}"),
            NodeKind::Path { clipping, .. } => format!("path clipping={clipping}"),
            NodeKind::Compound { .. } => "compound".into(),
            _ => "other".into(),
        };
        let mut s = format!(
            "{depth} {:?} {kind} {} {:?} isolate={} {:?} shape={}",
            n.name,
            r3(f64::from(n.opacity)),
            n.blend,
            n.isolate,
            n.knockout,
            n.knockout_shape
        );
        let mask = n.extras().mask.clone();
        if let Some(m) = &mask {
            s += &format!(" mask(clip={} invert={} disabled={} linked={})", m.clip, m.invert, m.disabled, m.linked);
        }
        out.push(s);
        if let Some(m) = mask {
            view(std::slice::from_ref(&m.art), depth + 1, out);
        }
        looks(n.children().map(Vec::as_slice).unwrap_or_default(), depth + 1, out);
    }
}

fn all_looks(doc: &Document) -> Vec<String> {
    let mut out = vec![];
    looks(&doc.layers, 0, &mut out);
    out
}

/// Fails with the lines where `got` and `want` differ.
fn same_lines(got: &[String], want: &[String]) {
    let differ: Vec<String> = (0..got.len().max(want.len()))
        .filter(|&i| got.get(i) != want.get(i))
        .map(|i| format!("line {i}:\n  got  {:?}\n  want {:?}", got.get(i), want.get(i)))
        .collect();
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

/// `nodes` as the reader reads them back: an opacity mask's art that isn't a group comes back as
/// a group holding it.
fn mask_art_grouped(nodes: &mut [Arc<Node>]) {
    for n in nodes {
        let n = Arc::make_mut(n);
        if let Some(m) = n.extras_mut().mask.as_mut()
            && !matches!(m.art.kind, NodeKind::Group { .. })
        {
            let art = m.art.clone();
            m.art = Arc::new(Node::new(art.id, NodeKind::Group { children: vec![art], clip: false }));
        }
        if let Some(c) = n.children_mut() {
            mask_art_grouped(c);
        }
    }
}

#[test]
fn transparency_clipping_and_masks_are_read_back() {
    let mut doc = transparent();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_lines(&lines(&back), &lines(&doc));
    // The disabled mask's art is one path: it reads back as a group holding the path.
    let mask_arts = |d: &Document| {
        let mut out = vec![];
        d.walk(|n| {
            if let Some(m) = &n.extras().mask {
                out.push(match &m.art.kind {
                    NodeKind::Group { children, .. } => format!("group of {}", children.len()),
                    NodeKind::Path { .. } => "path".into(),
                    _ => "other".into(),
                });
            }
        });
        out
    };
    assert_eq!(
        (mask_arts(&doc), mask_arts(&back)),
        (vec!["group of 2".to_string(), "path".into()], vec!["group of 2".to_string(), "group of 1".into()])
    );
    mask_art_grouped(&mut doc.layers);
    same_lines(&all_looks(&back), &all_looks(&doc));
}

#[test]
fn transparency_clipping_and_masks_write_the_same_bytes_again() {
    let doc = transparent();
    let first = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(first.program, export_ai_editing(&doc, &opts()).unwrap().program);
    assert_eq!(private_data(&doc, &opts()).unwrap().0, private_data(&doc, &opts()).unwrap().0);
    // What the reader reads back is written as the same program.
    let again = String::from_utf8(export_ai_editing(&read_back(&doc), &opts()).unwrap().program).unwrap();
    let first = String::from_utf8(first.program).unwrap();
    same_lines(&again.lines().map(String::from).collect::<Vec<_>>(), &first.lines().map(String::from).collect::<Vec<_>>());
}

#[test]
fn transparency_is_written_in_its_forms() {
    let p = program(&transparent());
    // A path's in its paint, after its colors; Screen is 2.
    assert!(p.contains(" Xa\n0 O\n2 0.5 0 0 1 Xy\n0 XR\n"), "{p}");
    // A compound path's in its paint before `*u`, its knockout shape kept.
    let (ring, _) = p.rsplit_once("*u\n").unwrap();
    assert!(ring.ends_with(" O\n2 0.7 0 0 1 Xy\n1 XR\n"), "{ring}");
    // A group's after its end and its name, then `Xd` and the object's own state marker.
    let knock = "U\n%_/ArtDictionary :\n%_/XMLUID : (Knock) ; (AI10_ArtUID) ,\n%_;\n%_\n0 0.4 1 1 1 Xy\n0 0 Xd\n6 () XW\n";
    assert!(p.contains(knock), "{p}");
    // A group without transparency of its own.
    assert!(p.contains("U\n9 () XW\n"), "{p}");
    // A clipping group: the clipping path last, then `W`.
    let window = "h\nW\nn\n%_/ArtDictionary :\n%_/XMLUID : (Edge) ; (AI10_ArtUID) ,\n%_;\n%_\nQ\n\
                  %_/ArtDictionary :\n%_/XMLUID : (Window) ; (AI10_ArtUID) ,\n%_;\n%_\n0 0.5 0 2 0 Xy\n0 0 Xd\n6 () XW\n";
    assert!(p.contains(window), "{p}");
    // A layer's after its art, with the layer's state marker; its knockout neutral (2).
    assert!(p.contains("1 0.6 1 2 1 Xy\n0 0 Xd\n7 () XW\nLB\n"), "{p}");
    // A clipping layer: its last `Lb` flag, its clipping path last.
    assert!(p.contains(" 0 50 1 Lb\n(Cut) Ln\n") && p.contains(" 0 50 0 Lb\n(Plain) Ln\n"), "{p}");
    let (_, cut) = p.split_once("(Cut) Ln").unwrap();
    // The name `Edge` is taken by the clipping group's path: this one is numbered.
    let edge = "h\nW\nn\n%_/ArtDictionary :\n%_/XMLUID : (Edge_00000000000000000001_) ; (AI10_ArtUID) ,\n%_;\n%_\n";
    assert!(cut.split_once("LB\n").unwrap().0.ends_with(edge), "{cut}");
    // The next layer's first path sets the state back.
    let (_, cut) = p.split_once("(Cut) Ln").unwrap();
    assert!(cut.split_once("\nf\n").unwrap().0.contains(" Xa\n0 O\n0 1 0 0 0 Xy\n0 XR\n"), "{cut}");
    // The mask in the object's dictionary, after its name.
    assert!(p.contains("%_/XMLUID : (Masked) ; (AI10_ArtUID) ,\n%_/Mask :\n%_X=\n"), "{p}");
    assert!(
        p.contains(
            "%_X+\n%_(Art) ,\n%_0 /Bool (Clipping) ,\n%_1 /Bool (Inverted) ,\n%_0 /Bool (Disabled) ,\n%_1 /Bool (Linked) ,\n%_; (AI9_Mask) ,\n"
        ),
        "{p}"
    );
}

/// A clipping path filled and stroked keeps its stroke; the reader reads its fill as a path of
/// its own under the clipped art, as the file's PDF page draws it.
#[test]
fn a_painted_clipping_path_is_written_with_its_paint() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut clip = clipping(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)));
    clip.appearance = Appearance::basic(solid(Color::rgb(1.0, 1.0, 0.0)), solid(Color::BLACK), 2.0).into();
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 40.0, 20.0), Color::rgb(0.0, 0.0, 1.0));
    let g = group(&mut doc, vec![clip, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let p = program(&doc);
    assert!(p.contains("h\nW\nb\nQ\n"), "{p}");
    let back = read_back(&doc);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    let painted: Vec<(bool, Option<String>, Option<String>)> = children
        .iter()
        .map(|c| {
            let clipping = matches!(c.kind, NodeKind::Path { clipping: true, .. });
            (clipping, c.appearance.fill().map(|f| paint(&f.paint)), c.appearance.stroke().map(|s| paint(&s.paint)))
        })
        .collect();
    assert_eq!(
        painted,
        [
            (true, None, Some(paint(&solid(Color::BLACK)))),
            (false, Some(paint(&solid(Color::rgb(1.0, 1.0, 0.0)))), None),
            (false, Some(paint(&solid(Color::rgb(0.0, 0.0, 1.0)))), None)
        ]
    );
}

/// A clipping path without points hides what it clips.
#[test]
fn an_empty_clipping_path_hides_the_art() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let clip = clipping(&mut doc, PathData::new(vec![]));
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 40.0, 20.0), Color::rgb(0.0, 0.0, 1.0));
    let g = group(&mut doc, vec![clip, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    // The art is hidden, as it is drawn: the group comes back empty.
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::CLIPPING), "{:?}", out.warnings);
    let back = read_back(&doc);
    let art = back.layers[0].children().unwrap();
    assert_eq!(art.len(), 1, "{art:?}");
    assert!(matches!(&art[0].kind, NodeKind::Group { children, .. } if children.is_empty()), "{art:?}");
}

/// A clipping group without members is written as an empty group, and reads back as one.
#[test]
fn an_empty_clipping_group_is_an_empty_group() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let g = see_through(group(&mut doc, vec![], true), 0.5, BlendMode::Normal);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let p = program(&doc);
    assert!(p.contains("\nu\nU\n0 0.5 0 2 0 Xy\n0 0 Xd\n6 () XW\n") && !p.contains("\nq\n"), "{p}");
    let back = read_back(&doc);
    let art = back.layers[0].children().unwrap();
    assert_eq!(art.len(), 1, "{art:?}");
    assert!(matches!(&art[0].kind, NodeKind::Group { children, clip: false } if children.is_empty()), "{art:?}");
    assert_eq!(art[0].opacity, 0.5);
}

/// A clipping path's paint that the path can't carry itself keeps the path's opacity.
#[test]
fn a_clipping_paths_other_paint_keeps_its_opacity() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut clip = clipping(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)));
    let g = Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, Color::rgb(1.0, 1.0, 1.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 0.0))],
    );
    clip.appearance = Appearance::basic(Paint::Gradient(Box::new(GradientPaint::new(g))), Paint::None, 0.0).into();
    clip.opacity = 0.5;
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 40.0, 20.0), Color::rgb(0.0, 0.0, 1.0));
    let g = group(&mut doc, vec![clip, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::CLIP_PAINT), "{:?}", out.warnings);
    assert!(!out.warnings.iter().any(|w| w == writer::CLIP_PAINT_MASKS), "{:?}", out.warnings);
    // With an opacity mask, which stays on the clipping path: a warning says so.
    let mut masked = doc.clone();
    let art = square(&mut masked, Rect::new(10.0, 10.0, 20.0, 20.0), Color::gray(0.5));
    if let Some(c) = Arc::make_mut(&mut masked.layers[0]).children_mut()
        && let Some(NodeKind::Group { children, .. }) = c.first_mut().map(|g| &mut Arc::make_mut(g).kind)
    {
        Arc::make_mut(&mut children[0]).extras_mut().mask = Some(Box::new(OpacityMask::new(art, true)));
    }
    let out = export_ai_editing(&masked, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::CLIP_PAINT_MASKS), "{:?}", out.warnings);
    // Two fills at the clipping path's 50% are a group at 50% of the two at 100%, as the canvas
    // draws them together.
    let mut two = doc.clone();
    if let Some(c) = Arc::make_mut(&mut two.layers[0]).children_mut()
        && let Some(NodeKind::Group { children, .. }) = c.first_mut().map(|g| &mut Arc::make_mut(g).kind)
    {
        let clip = Arc::make_mut(&mut children[0]);
        let mut items = clip.appearance.items.clone();
        items.push(AppearanceItem::Fill(FillLayer::new(solid(Color::rgb(1.0, 0.0, 0.0)))));
        clip.appearance = Appearance { items, ..Appearance::default() }.into();
    }
    let back = read_back(&two);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    let fills = &children[1];
    assert!(matches!(fills.kind, NodeKind::Group { clip: false, .. }) && fills.opacity == 0.5, "{fills:?}");
    assert_eq!(fills.children().unwrap().iter().map(|c| c.opacity).collect::<Vec<_>>(), [1.0, 1.0]);
    // A locked clipping path's fill is locked too, the art it clips not.
    let mut locked = doc.clone();
    if let Some(c) = Arc::make_mut(&mut locked.layers[0]).children_mut()
        && let Some(NodeKind::Group { children, .. }) = c.first_mut().map(|g| &mut Arc::make_mut(g).kind)
    {
        Arc::make_mut(&mut children[0]).locked = true;
    }
    let back = read_back(&locked);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    assert_eq!(children.iter().map(|c| c.locked).collect::<Vec<_>>(), [true, true, false]);
    let back = read_back(&doc);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    // The clipping path, its fill as a path under the art, the art.
    let fill = &children[1];
    assert!(matches!(fill.kind, NodeKind::Path { clipping: false, .. }));
    assert_eq!((fill.opacity, paint(fill.appearance.items[0].paint())), (0.5, paint(&solid(Color::rgb(0.5, 0.5, 0.5)))));
}

/// A clipping path's stroke that a plain line can't draw goes over the clipped art, its fill under it.
#[test]
fn a_clipping_paths_other_stroke_goes_over_the_art() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut clip = clipping(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)));
    let g = Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, Color::rgb(1.0, 1.0, 1.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 0.0))],
    );
    let mut stroke = StrokeLayer::new(solid(Color::rgb(1.0, 0.0, 0.0)), 4.0);
    stroke.dash = Some(Dash { pattern: vec![2.0, 2.0], offset: 0.0, align_corners: true });
    clip.appearance = Appearance {
        items: vec![AppearanceItem::Fill(FillLayer::new(Paint::Gradient(Box::new(GradientPaint::new(g))))), AppearanceItem::Stroke(stroke)],
        ..Appearance::default()
    }
    .into();
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 40.0, 20.0), Color::rgb(0.0, 0.0, 1.0));
    let g = group(&mut doc, vec![clip, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let back = read_back(&doc);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    let colors: Vec<String> = children.iter().skip(1).map(|c| paint(c.appearance.items.first().map_or(&Paint::None, |i| i.paint()))).collect();
    // The fill (its average) under the blue art, the red stroke's dots over it.
    assert_eq!(colors.first(), Some(&paint(&solid(Color::rgb(0.5, 0.5, 0.5)))), "{colors:?}");
    assert_eq!(colors.get(1), Some(&paint(&solid(Color::rgb(0.0, 0.0, 1.0)))), "{colors:?}");
    assert!(colors.len() > 2 && colors.iter().skip(2).all(|c| *c == paint(&solid(Color::rgb(1.0, 0.0, 0.0)))), "{colors:?}");
}

/// An unpainted path with transparency of its own after a compound path: the compound path keeps its own.
#[test]
fn an_unpainted_paths_transparency_after_a_compound_path_is_its_own() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let o = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), Appearance::default());
    let i = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(15.0, 15.0, 25.0, 25.0)), Appearance::default());
    let mut ring = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(o), Arc::new(i)], rule: FillRule::EvenOdd });
    ring.appearance = Appearance::basic(solid(Color::BLACK), Paint::None, 0.0).into();
    doc.insert(Some(layer), usize::MAX, ring).unwrap();
    let bare = see_through(
        Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(50.0, 10.0, 70.0, 30.0)), Appearance::default()),
        0.5,
        BlendMode::Multiply,
    );
    doc.insert(Some(layer), usize::MAX, bare).unwrap();
    let back = read_back(&doc);
    let art = back.layers[0].children().unwrap();
    assert_eq!((art[0].opacity, art[0].blend, art[1].opacity, art[1].blend), (1.0, BlendMode::Normal, 0.5, BlendMode::Multiply));
}

/// Opacity masks in mask art nested past the limit are left out, not written empty.
#[test]
fn opacity_masks_nested_too_deep_are_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut n = square(&mut doc, Rect::new(10.0, 10.0, 20.0, 20.0), Color::gray(0.5));
    for _ in 0..12 {
        let mut owner = square(&mut doc, Rect::new(10.0, 10.0, 20.0, 20.0), Color::gray(0.5));
        owner.extras_mut().mask = Some(Box::new(OpacityMask::new(n, true)));
        n = owner;
    }
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::DEEP_MASKS), "{:?}", out.warnings);
    let p = String::from_utf8(out.program).unwrap().replace("\r\n", "\n");
    assert!(!p.contains("%_X=\n%_X+\n"), "no empty mask");
    // The masks within the limit are read back, one in the art of the other.
    let back = read_back(&doc);
    let mut n = back.layers[0].children().unwrap()[0].clone();
    let mut depth = 0;
    while let Some(m) = n.extras().mask.clone() {
        depth += 1;
        n = match &m.art.kind {
            NodeKind::Group { children, .. } if children.len() == 1 => children[0].clone(),
            _ => m.art.clone(),
        };
    }
    assert_eq!(depth, 8);
}

#[test]
fn a_clipping_object_other_than_a_path_cuts_the_art() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let t = vectorcraft_doc::TextObject::point(
        vectorcraft_geom::Point::new(10.0, 70.0),
        "Type",
        vectorcraft_doc::CharStyle { size: 18.0, fill: solid(Color::BLACK), ..vectorcraft_doc::CharStyle::default() },
    );
    let t = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(0.0, 0.0, 1.0));
    let g = group(&mut doc, vec![t, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::CLIPPING), "{:?}", out.warnings);
    assert!(!String::from_utf8(out.program).unwrap().contains("\nq\n"));
    // The type's black fill under the blue square, both cut to the type: within its line,
    // narrower than the square.
    assert_eq!(cut_to_type(&read_back(&doc).layers[0]), ["black", "blue"]);
}

/// The colors of the paths `layer` holds, in paint order, each kind once, after checking that they
/// all lie within the line of type `Type` set at (10, 70) in 18 points.
fn cut_to_type(layer: &Node) -> Vec<&'static str> {
    let mut all = vec![];
    paths(layer.children().unwrap(), &mut all);
    let b = all.iter().filter_map(|n| n.geometric_bounds()).reduce(|a, b| a.union(b)).unwrap();
    assert!(b.x0 >= 9.0 && b.x1 < 100.0 && b.y0 > 40.0 && b.y1 < 80.0, "{b:?}");
    let mut colors = vec![];
    for n in &all {
        let c = match n.appearance.fill().map(|f| paint(&f.paint)) {
            Some(p) if p == paint(&solid(Color::BLACK)) => "black",
            Some(p) if p == paint(&solid(Color::rgb(0.0, 0.0, 1.0))) => "blue",
            _ => "other",
        };
        if colors.last() != Some(&c) {
            colors.push(c);
        }
    }
    colors
}

/// A clipping layer whose clipping object isn't a path is written as a layer that doesn't clip,
/// holding its art cut to that object.
#[test]
fn a_clipping_layer_clipped_by_type_is_not_written_as_a_clipping_layer() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let t = vectorcraft_doc::TextObject::point(
        vectorcraft_geom::Point::new(10.0, 70.0),
        "Type",
        vectorcraft_doc::CharStyle { size: 18.0, fill: solid(Color::BLACK), ..vectorcraft_doc::CharStyle::default() },
    );
    let t = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    doc.insert(Some(layer), usize::MAX, t).unwrap();
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(0.0, 0.0, 1.0));
    doc.insert(Some(layer), usize::MAX, inside).unwrap();
    if let NodeKind::Layer { clip, .. } = &mut Arc::make_mut(&mut doc.layers[0]).kind {
        *clip = true;
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::CLIPPING), "{:?}", out.warnings);
    let p = String::from_utf8(out.program).unwrap().replace("\r\n", "\n");
    assert!(p.contains(" 0 50 0 Lb\n") && !p.contains(" 0 50 1 Lb\n") && !p.contains("\nW\n"), "{p}");
    let back = read_back(&doc);
    assert!(matches!(back.layers[0].kind, NodeKind::Layer { clip: false, .. }));
    assert_eq!(cut_to_type(&back.layers[0]), ["black", "blue"]);
}

/// An empty clipping layer is written as an empty layer.
#[test]
fn an_empty_clipping_layer_is_an_empty_layer() {
    let mut doc = Document::new(200.0, 100.0);
    if let NodeKind::Layer { clip, .. } = &mut Arc::make_mut(&mut doc.layers[0]).kind {
        *clip = true;
    }
    let p = program(&doc);
    assert!(p.contains(" 0 50 0 Lb\n(Layer 1) Ln\n") && !p.contains("\nu\n"), "{p}");
    let back = read_back(&doc);
    assert!(back.layers[0].children().is_none_or(Vec::is_empty), "{:?}", back.layers[0]);
}

/// A guide takes its own transparency, not the one of the object before it.
#[test]
fn guides_keep_their_own_transparency() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let bright = see_through(square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1.0, 0.0, 0.0)), 0.5, BlendMode::Screen);
    doc.insert(Some(layer), usize::MAX, bright).unwrap();
    for (r, opacity, blend) in
        [(Rect::new(20.0, 20.0, 40.0, 40.0), 1.0, BlendMode::Normal), (Rect::new(50.0, 20.0, 70.0, 40.0), 0.4, BlendMode::Multiply)]
    {
        let mut g = clipping(&mut doc, shapes::rectangle(r));
        g.kind = NodeKind::Path { path: shapes::rectangle(r), rule: FillRule::NonZero, live: None, clipping: false, guide: true };
        doc.insert(Some(layer), usize::MAX, see_through(g, opacity, blend)).unwrap();
    }
    let back = read_back(&doc);
    let got: Vec<(bool, f32, BlendMode)> =
        back.layers[0].children().unwrap().iter().map(|n| (matches!(n.kind, NodeKind::Path { guide: true, .. }), n.opacity, n.blend)).collect();
    assert_eq!(got, [(false, 0.5, BlendMode::Screen), (true, 1.0, BlendMode::Normal), (true, 0.4, BlendMode::Multiply)]);
}

/// The transparency a group's last member or a group's state block sets carries over to the object
/// after the group, which then writes no `Xy` of its own when it has the same.
#[test]
fn objects_after_a_group_keep_their_transparency() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let red = Color::rgb(1.0, 0.0, 0.0);
    let screen = |doc: &mut Document, x: f64| see_through(square(doc, Rect::new(x, 0.0, x + 10.0, 10.0), red), 0.5, BlendMode::Screen);
    // A plain group whose last member is at 50% in Screen, then a path the same.
    let a = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), red);
    let b = screen(&mut doc, 10.0);
    let plain = group(&mut doc, vec![a, b], false);
    let after_plain = screen(&mut doc, 30.0);
    // A group at 40% with knockout off, then a path at 40%.
    let c = square(&mut doc, Rect::new(50.0, 0.0, 60.0, 10.0), red);
    let mut faint = see_through(group(&mut doc, vec![c], false), 0.4, BlendMode::Normal);
    faint.knockout = Knockout::Off;
    let after_faint = see_through(square(&mut doc, Rect::new(70.0, 0.0, 80.0, 10.0), red), 0.4, BlendMode::Normal);
    for n in [plain, after_plain, faint, after_faint] {
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let p = program(&doc);
    // No `Xy` between the groups' state blocks and the paths after them: the second square's, the
    // third's (opaque, after the second) and the second group's.
    let after = |block: &str| p.split_once(block).unwrap().1.split_once("\nf\n").unwrap().0.to_string();
    assert!(!after("U\n9 () XW\n").contains("Xy"), "{p}");
    assert!(!after("U\n0 0.4 0 0 0 Xy\n0 0 Xd\n6 () XW\n").contains("Xy"), "{p}");
    assert_eq!(p.matches(" Xy\n").count(), 3, "{p}");
    same_lines(&all_looks(&read_back(&doc)), &all_looks(&doc));
}

#[test]
fn transparency_of_single_fills_and_layer_masks_is_reported_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut fill = FillLayer::new(solid(Color::BLACK));
    fill.opacity = 0.5;
    let n = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)),
        Appearance { items: vec![AppearanceItem::Fill(fill)], ..Appearance::default() },
    );
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let art = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), Color::gray(0.5));
    Arc::make_mut(&mut doc.layers[0]).extras_mut().mask = Some(Box::new(OpacityMask::new(art, true)));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    for w in [writer::ITEM_TRANSPARENCY, writer::LAYER_MASKS] {
        assert!(out.warnings.iter().any(|x| x == w), "{w}: {:?}", out.warnings);
    }
    // A disabled one too.
    if let Some(m) = Arc::make_mut(&mut doc.layers[0]).extras_mut().mask.as_mut() {
        m.disabled = true;
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|x| x == writer::LAYER_MASKS), "{:?}", out.warnings);
    // A clipping path's fill of its own opacity.
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut clip = clipping(&mut doc, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)));
    let mut fill = FillLayer::new(solid(Color::BLACK));
    fill.opacity = 0.5;
    clip.appearance = Appearance { items: vec![AppearanceItem::Fill(fill)], ..Appearance::default() }.into();
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 20.0, 20.0), Color::gray(0.5));
    let g = group(&mut doc, vec![clip, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|x| x == writer::ITEM_TRANSPARENCY), "{:?}", out.warnings);
}
