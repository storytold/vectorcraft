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

/// The end of the document data: the artboards, the active artboard (the first) and the canvas
/// size in the `/Recorded` part of the `/Document` dictionary, then an empty `/NotRecorded` part.
const RECORDED: &str = "%_; (ArtboardArray) ,\n%_0 /Int (CropAreaActive) ,\n%_16383 /Int (AIDocumentCanvasSize) ,\n%_; /Recorded ,\n%_/Dictionary : /NotRecorded ,\n%_; /NotRecorded ,\n%_;\n%AI9_EndDocumentData\n";

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

/// A 200 × 100 document of a gradient fill, a compound path and an image, and what the data holds
/// as plain paths: type and a symbol instance.
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
    // Type the text document can't hold (a gradient fill).
    let shade = Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))]);
    let t = vectorcraft_doc::TextObject::point(
        vectorcraft_geom::Point::new(10.0, 70.0),
        "Type",
        vectorcraft_doc::CharStyle {
            size: 18.0,
            fill: Paint::Gradient(Box::new(GradientPaint::new(shade))),
            ..vectorcraft_doc::CharStyle::default()
        },
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
    for w in [writer::TYPE, writer::LIVE] {
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
            NodeKind::Image(_) => "image",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["path", "compound", "image", "group", "group"]);
    let mut all = vec![];
    paths(&layer[3..], &mut all);
    assert!(all.iter().all(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    // The gradient as a gradient, drawing as it did.
    let bounds = Rect::new(10.0, 10.0, 30.0, 30.0);
    let original = doc.layers[0].children().unwrap()[0].appearance.items[0].paint();
    assert_eq!(drawn(&back, layer[0].appearance.items[0].paint(), bounds), drawn(&doc, original, bounds));
    // The image as an image.
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
    // A pattern fill: type the text document can't hold.
    let pattern = Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY };
    let style = vectorcraft_doc::CharStyle { size: 18.0, fill: pattern, underline: true, ..vectorcraft_doc::CharStyle::default() };
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
    let mut found = vec![];
    pictures(&back, &back.layers, "", &mut found);
    assert_eq!(found.len(), 1, "the adjusted image is found");
    assert!(!found[0].ends_with("rgba [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]"), "the image is adjusted: {found:?}");
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
    paths(&back.pattern("Dots").unwrap().art, &mut all);
    let red = paint(&solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(all.iter().any(|p| p.appearance.items.iter().any(|i| paint(i.paint()) == red)), "the tile keeps its group's fill");
}

/// A pattern fill written as its tiles (its tiles aren't a grid) that would place more tiles than
/// a fill places is written as the shape filled gray, as a pattern that isn't found is, instead of
/// as nothing.
#[test]
fn a_pattern_fill_of_too_many_tiles_is_written_gray() {
    let mut doc = Document::new(1200.0, 1200.0);
    let layer = doc.layers[0].id;
    let dot = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 0.5, 0.5)), Appearance::basic(solid(Color::BLACK), Paint::None, 0.0));
    let tiny = vectorcraft_doc::PatternDef {
        tile: Rect::new(0.0, 0.0, 1.0, 1.0),
        tile_type: vectorcraft_doc::TileType::BrickByRow { offset: 0.5 },
        ..vectorcraft_doc::PatternDef::new("Tiny", vec![Arc::new(dot)])
    };
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

/// The active artboard, the one `opened_artboard` names (else the first), is the selected one and
/// its index follows the artboards in the recorded part; the editing data reads back with that
/// artboard active.
#[test]
fn the_active_artboard_follows_the_artboards() {
    let mut doc = Document::new(200.0, 100.0);
    for (id, x0) in [(2, 250.0), (3, 500.0)] {
        doc.artboards.push(Artboard { id, rect: Rect::new(x0, 0.0, x0 + 200.0, 100.0), ..doc.artboards[0].clone() });
    }
    let first = doc.artboards[0].id;
    // None, the first, the third, and an artboard the document doesn't have (the first).
    for (opened, index) in [(None, 0), (Some(first), 0), (Some(3), 2), (Some(9), 0)] {
        doc.opened_artboard = opened;
        let p = program(&doc);
        let entry = format!("%_; (ArtboardArray) ,\n%_{index} /Int (CropAreaActive) ,\n%_16383 /Int (AIDocumentCanvasSize) ,\n%_; /Recorded ,\n");
        assert!(p.contains(&entry), "{opened:?}: {p}");
        assert_eq!(p.matches("%_1 /Bool (IsArtboardSelected) ,\n").count(), 1, "{opened:?}");
        let back = read_back(&doc);
        let active = back.opened_artboard.and_then(|id| back.artboards.iter().position(|a| a.id == id)).unwrap_or(0);
        assert_eq!((back.artboards.len(), active), (3, index), "{opened:?}");
    }
}

/// The copy's active artboard read on its own, for a document restored from the file beside the
/// copy: `None` for the first, and when the copy names an artboard that document doesn't have or
/// can't be decoded.
#[test]
fn a_restored_document_opens_on_the_artboard_the_copy_names() {
    let mut doc = Document::new(200.0, 100.0);
    for (id, x0) in [(2, 250.0), (3, 500.0)] {
        doc.artboards.push(Artboard { id, rect: Rect::new(x0, 0.0, x0 + 200.0, 100.0), ..doc.artboards[0].clone() });
    }
    let mut one_artboard = doc.clone();
    one_artboard.artboards.truncate(1);
    for (opened, read) in [(Some(3), Some(3)), (Some(2), Some(2)), (Some(1), None), (None, None), (Some(9), None)] {
        doc.opened_artboard = opened;
        let data = private_data(&doc, &opts()).unwrap().0.data;
        assert_eq!(crate::ai_opened_artboard(&data, &doc), read, "{opened:?}");
        assert_eq!(crate::ai_opened_artboard(&data, &one_artboard), None, "{opened:?}");
    }
    assert_eq!(crate::ai_opened_artboard(b"%AI24_ZStandard_Data not zstd", &doc), None);
}

/// A guide is its path followed by `(n) *` (closed) or `(N) *` (open), with no `(N) *` before
/// it, and reads back as the same guide.
#[test]
fn guides_are_their_path_then_the_guide_operator() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let line = shapes::line(vectorcraft_geom::Point::new(10.0, 80.0), vectorcraft_geom::Point::new(150.0, 80.0));
    for path in [shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), line] {
        let g = Node::new(doc.alloc_id(), NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: true });
        doc.insert(Some(layer), usize::MAX, g).unwrap();
    }
    let p = program(&doc);
    let (closed, open) = (p.matches("\n(n) *\n").count(), p.matches("\n(N) *\n").count());
    assert_eq!((closed, open), (1, 1), "{p}");
    assert!(p.contains("0 Xw\n10 -10 m\n") && p.contains("10 -10 L\n(n) *\n10 -80 m\n150 -80 L\n(N) *\nLB\n"), "{p}");
    assert_eq!(lines(&read_back(&doc)), lines(&doc));
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
fn images_of_more_pixels_than_the_reader_reads_are_mid_gray() {
    let mut png = vec![];
    image::GrayImage::new(6000, 6000).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let im = image_node(&mut doc, ImageBlob::png(png), (10, 10), Affine::IDENTITY, None);
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::IMAGES]);
    assert_eq!(paint(read_back(&doc).layers[0].children().unwrap()[0].appearance.items[0].paint()), paint(&solid(Color::gray(0.5))));
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

#[test]
fn transparency_clipping_and_masks_are_read_back() {
    let doc = transparent();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_lines(&lines(&back), &lines(&doc));
    same_lines(&all_looks(&back), &all_looks(&doc));
}

/// An opacity mask's art reads back in its own shape: one object, a group of one object, a group
/// of two, a group of one group, a see-through group of two, and a group around a hidden object or
/// a guide.
#[test]
fn opacity_mask_art_keeps_its_shape_when_read_back() {
    let mut doc = Document::new(200.0, 100.0);
    doc.swatches = vec![Swatch { name: "Blue".into(), paint: solid(Color::rgb(0.0, 0.0, 1.0)), global: false, spot: false }].into();
    doc.swatch_groups = vec![].into();
    let layer = doc.layers[0].id;
    let art = |doc: &mut Document| square(doc, Rect::new(5.0, 5.0, 10.0, 10.0), Color::gray(0.5));
    let path = art(&mut doc);
    let one = art(&mut doc);
    let of_one = group(&mut doc, vec![one], false);
    let (a, b) = (art(&mut doc), art(&mut doc));
    let of_two = group(&mut doc, vec![a, b], false);
    let (a, b) = (art(&mut doc), art(&mut doc));
    let inner = group(&mut doc, vec![a, b], false);
    let of_group = group(&mut doc, vec![inner], false);
    let (a, b) = (art(&mut doc), art(&mut doc));
    let faded = see_through(group(&mut doc, vec![a, b], false), 0.5, BlendMode::Normal);
    let mut off = art(&mut doc);
    off.visible = false;
    let of_hidden = group(&mut doc, vec![off], false);
    let line = Node::new(
        doc.alloc_id(),
        NodeKind::Path {
            path: shapes::rectangle(Rect::new(5.0, 5.0, 10.0, 10.0)),
            rule: FillRule::NonZero,
            live: None,
            clipping: false,
            guide: true,
        },
    );
    let of_guide = group(&mut doc, vec![line], false);
    for mask_art in [path, of_one, of_two, of_group, faded, of_hidden, of_guide] {
        let mut owner = square(&mut doc, Rect::new(0.0, 0.0, 30.0, 30.0), Color::rgb(1.0, 0.0, 0.0));
        owner.extras_mut().mask = Some(Box::new(OpacityMask::new(mask_art, true)));
        doc.insert(Some(layer), usize::MAX, owner).unwrap();
    }
    let back = read_back(&doc);
    same_lines(&all_looks(&back), &all_looks(&doc));
    let first = String::from_utf8(export_ai_editing(&doc, &opts()).unwrap().program).unwrap();
    let again = String::from_utf8(export_ai_editing(&back, &opts()).unwrap().program).unwrap();
    same_lines(&again.lines().map(String::from).collect::<Vec<_>>(), &first.lines().map(String::from).collect::<Vec<_>>());
}

/// Hidden art of a mask is written shown: the canvas draws the art's root either way, and the reader
/// puts a hidden object in a group that doesn't draw it.
#[test]
fn a_hidden_opacity_mask_art_root_reads_back_shown() {
    let mut doc = Document::new(200.0, 100.0);
    doc.swatches = vec![Swatch { name: "Blue".into(), paint: solid(Color::rgb(0.0, 0.0, 1.0)), global: false, spot: false }].into();
    doc.swatch_groups = vec![].into();
    let layer = doc.layers[0].id;
    let art = |doc: &mut Document| square(doc, Rect::new(5.0, 5.0, 10.0, 10.0), Color::gray(0.5));
    let mut path = art(&mut doc);
    path.visible = false;
    let (a, b) = (art(&mut doc), art(&mut doc));
    let mut of_two = group(&mut doc, vec![a, b], false);
    of_two.visible = false;
    for mask_art in [path, of_two] {
        let mut owner = square(&mut doc, Rect::new(0.0, 0.0, 30.0, 30.0), Color::rgb(1.0, 0.0, 0.0));
        owner.extras_mut().mask = Some(Box::new(OpacityMask::new(mask_art, true)));
        doc.insert(Some(layer), usize::MAX, owner).unwrap();
    }
    let back = read_back(&doc);
    let roots: Vec<_> = back.layers[0].children().unwrap().iter().map(|o| o.extras().mask.as_ref().unwrap().art.clone()).collect();
    assert!(matches!(roots.as_slice(), [p, g] if p.visible && g.visible
        && matches!(p.kind, NodeKind::Path { .. })
        && matches!(&g.kind, NodeKind::Group { children, clip: false } if children.len() == 2 && children.iter().all(|c| c.visible))));
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
    // Two fills: a path paints one.
    let fills = [Color::rgb(0.5, 0.5, 0.5), Color::rgb(1.0, 0.0, 0.0)].map(|c| AppearanceItem::Fill(FillLayer::new(solid(c))));
    clip.appearance = Appearance { items: fills.to_vec(), ..Appearance::default() }.into();
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
    // draws them together, under the art.
    let back = read_back(&doc);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    let fills = &children[1];
    assert!(matches!(fills.kind, NodeKind::Group { clip: false, .. }) && fills.opacity == 0.5, "{fills:?}");
    assert_eq!(fills.children().unwrap().iter().map(|c| c.opacity).collect::<Vec<_>>(), [1.0, 1.0]);
    let colors: Vec<String> = fills.children().unwrap().iter().map(|c| paint(c.appearance.items[0].paint())).collect();
    assert_eq!(colors, [paint(&solid(Color::rgb(0.5, 0.5, 0.5))), paint(&solid(Color::rgb(1.0, 0.0, 0.0)))]);
    assert!(matches!(children[2].kind, NodeKind::Path { .. }) && children.len() == 3, "the art over the fills");
    // One fill a path can't paint (a brick pattern, written as its tiles) is written under the art
    // at the clipping path's opacity.
    let mut brick = doc.clone();
    let dot = square(&mut brick, Rect::new(0.0, 0.0, 2.0, 2.0), Color::rgb(1.0, 0.0, 0.0));
    brick.patterns.push(vectorcraft_doc::PatternDef {
        tile: Rect::new(0.0, 0.0, 4.0, 4.0),
        tile_type: vectorcraft_doc::TileType::BrickByRow { offset: 0.5 },
        ..vectorcraft_doc::PatternDef::new("Brick", vec![Arc::new(dot)])
    });
    if let Some(c) = Arc::make_mut(&mut brick.layers[0]).children_mut()
        && let Some(NodeKind::Group { children, .. }) = c.first_mut().map(|g| &mut Arc::make_mut(g).kind)
    {
        let fill = Paint::Pattern { pattern: "Brick".into(), xf: Affine::IDENTITY };
        Arc::make_mut(&mut children[0]).appearance = Appearance::basic(fill, Paint::None, 0.0).into();
    }
    let out = export_ai_editing(&brick, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PATTERNS) && out.warnings.iter().any(|w| w == writer::CLIP_PAINT), "{:?}", out.warnings);
    let back = read_back(&brick);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    assert_eq!(children.len(), 3, "the clipping path, its fill, the art");
    assert_eq!(children[1].opacity, 0.5);
    let mut tiles = vec![];
    paths(std::slice::from_ref(&children[1]), &mut tiles);
    let painted: Vec<&Node> = tiles.iter().filter(|t| !t.appearance.items.is_empty()).collect();
    let red = paint(&solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(!painted.is_empty() && painted.iter().all(|t| t.opacity == 1.0 && paint(t.appearance.items[0].paint()) == red), "{painted:?}");
    assert_eq!(paint(children[2].appearance.items[0].paint()), paint(&solid(Color::rgb(0.0, 0.0, 1.0))), "the art over the fill");
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
}

/// A clipping path's stroke that a plain line can't draw goes over the clipped art, its fill under it.
#[test]
fn a_clipping_paths_other_stroke_goes_over_the_art() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut clip = clipping(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)));
    let mut stroke = StrokeLayer::new(solid(Color::rgb(1.0, 0.0, 0.0)), 4.0);
    stroke.dash = Some(Dash { pattern: vec![2.0, 2.0], offset: 0.0, align_corners: true });
    clip.appearance = Appearance {
        items: vec![AppearanceItem::Fill(FillLayer::new(solid(Color::rgb(0.5, 0.5, 0.5)))), AppearanceItem::Stroke(stroke)],
        ..Appearance::default()
    }
    .into();
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 40.0, 20.0), Color::rgb(0.0, 0.0, 1.0));
    let g = group(&mut doc, vec![clip, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let back = read_back(&doc);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    let colors: Vec<String> = children.iter().skip(1).map(|c| paint(c.appearance.items.first().map_or(&Paint::None, |i| i.paint()))).collect();
    // The fill under the blue art, the red stroke's dots over it.
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

// ---------- gradients and patterns ----------

const INK: Color = Color::Cmyk { c: 0.0, m: 0.6, y: 1.0, k: 0.0 };

/// A gradient of `kind` through `stops` (offset, color, opacity, midpoint).
fn ramp(kind: GradientKind, stops: &[(f32, Color, f32, f32)]) -> Gradient {
    let stops =
        stops.iter().map(|&(offset, color, opacity, midpoint)| GradientStop { opacity, midpoint, ..GradientStop::new(offset, color) }).collect();
    Gradient::new(kind, stops)
}

/// The three-stop linear gradient of the `Sunset` swatch: its middle stop half opaque, its first
/// midpoint at 30%.
fn sunset() -> Gradient {
    ramp(
        GradientKind::Linear,
        &[(0.0, Color::rgb(1.0, 0.0, 0.0), 1.0, 0.3), (0.4, Color::rgb(0.0, 1.0, 0.0), 0.5, 0.5), (1.0, Color::rgb(0.0, 0.0, 1.0), 1.0, 0.5)],
    )
}

fn gradient_paint(g: Gradient, geom: Option<vectorcraft_color::gradient::GradientGeom>, swatch: Option<&str>) -> Paint {
    Paint::Gradient(Box::new(GradientPaint { geom, swatch: swatch.map(Into::into), ..GradientPaint::new(g) }))
}

fn painted(doc: &mut Document, path: PathData, items: Vec<AppearanceItem>) -> Node {
    Node::path(doc.alloc_id(), path, Appearance { items, ..Appearance::default() })
}

/// A 300 × 200 document of gradients and patterns. Swatches: the linear gradient `Sunset`, the radial
/// gradient `Glow` that nothing uses, the spot color `Ink`, the pattern `Dots` (a 10-point grid tile of
/// a red square and a circle filled with a gradient) and `Checks`, which nothing uses. Art, bottom
/// first: a rectangle filled with `Sunset`
/// placed at 30° over part of it and stroked black; an ellipse filled with a radial gradient half as
/// high as wide whose focal point is off its center; a rectangle filled yellow and stroked with a
/// gradient; one filled and stroked with gradients; an even-odd compound path filled with a
/// gradient; an open path filled with a gradient; a rectangle filled with a gradient of a gray, a
/// CMYK color and `Ink` at 70%; a rectangle filled with `Dots` turned, scaled and moved; and a
/// rectangle stroked 8 points with `Dots`.
fn gradients_and_patterns() -> Document {
    use vectorcraft_color::gradient::GradientGeom;
    let mut doc = Document::new(300.0, 200.0);
    let glow = ramp(GradientKind::Radial, &[(0.0, Color::rgb(1.0, 1.0, 1.0), 1.0, 0.5), (1.0, Color::rgb(0.1, 0.2, 0.8), 1.0, 0.5)]);
    let two = ramp(GradientKind::Linear, &[(0.0, Color::rgb(1.0, 1.0, 0.0), 1.0, 0.5), (1.0, Color::rgb(0.0, 0.0, 0.0), 1.0, 0.5)]);
    // The pattern art.
    let red = square(&mut doc, Rect::new(0.0, 0.0, 4.0, 4.0), Color::rgb(1.0, 0.0, 0.0));
    let ball = painted(
        &mut doc,
        shapes::ellipse(Rect::new(5.0, 5.0, 9.0, 9.0)),
        vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(two.clone(), None, None)))],
    );
    let mut dots = vectorcraft_doc::PatternDef::new("Dots", vec![Arc::new(red), Arc::new(ball)]);
    dots.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
    let check = square(&mut doc, Rect::new(0.0, 0.0, 5.0, 5.0), Color::BLACK);
    let mut checks = vectorcraft_doc::PatternDef::new("Checks", vec![Arc::new(check)]);
    checks.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
    doc.patterns.extend([dots, checks]);
    let swatch = |name: &str, paint: Paint, spot: bool| Swatch { name: name.into(), paint, global: spot, spot };
    let mut swatches: Vec<Swatch> = doc.swatches.iter().filter(|s| matches!(s.paint, Paint::None)).cloned().collect();
    swatches.extend([
        swatch("Sunset", gradient_paint(sunset(), None, None), false),
        swatch("Glow", gradient_paint(glow.clone(), None, None), false),
        swatch("Ink", solid(INK), true),
        swatch("Dots", Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }, false),
        swatch("Checks", Paint::Pattern { pattern: "Checks".into(), xf: Affine::IDENTITY }, false),
    ]);
    doc.swatches = swatches.into();
    doc.swatch_groups = Default::default();
    let layer = doc.layers[0].id;
    let fill = |p: Paint| AppearanceItem::Fill(FillLayer::new(p));
    let stroke = |p: Paint, w: f64| AppearanceItem::Stroke(StrokeLayer::new(p, w));
    let placed = GradientGeom { start: (20.0, 40.0).into(), end: (60.0, 17.0).into(), aspect: 1.0, focal: None };
    let mut oval = GradientGeom { start: (140.0, 30.0).into(), end: (180.0, 30.0).into(), aspect: 0.5, focal: None };
    oval.set_focal(Some((150.0, 25.0).into()));
    let mut nodes = vec![
        painted(
            &mut doc,
            shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 50.0)),
            vec![fill(gradient_paint(sunset(), Some(placed), Some("Sunset"))), stroke(solid(Color::BLACK), 2.0)],
        ),
        painted(&mut doc, shapes::ellipse(Rect::new(100.0, 10.0, 180.0, 50.0)), vec![fill(gradient_paint(glow.clone(), Some(oval), None))]),
        painted(
            &mut doc,
            shapes::rectangle(Rect::new(190.0, 10.0, 280.0, 50.0)),
            vec![fill(solid(Color::rgb(1.0, 1.0, 0.0))), stroke(gradient_paint(two.clone(), None, None), 6.0)],
        ),
        painted(
            &mut doc,
            shapes::rectangle(Rect::new(10.0, 70.0, 90.0, 110.0)),
            vec![fill(gradient_paint(sunset(), None, Some("Sunset"))), stroke(gradient_paint(two.clone(), None, None), 6.0)],
        ),
    ];
    let outer = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(100.0, 70.0, 180.0, 110.0)), Appearance::default());
    let hole = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(120.0, 80.0, 160.0, 100.0)), Appearance::default());
    let mut ring = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(hole)], rule: FillRule::EvenOdd });
    ring.appearance = Appearance { items: vec![fill(gradient_paint(two.clone(), None, None))], ..Appearance::default() }.into();
    nodes.push(ring);
    let mut open = shapes::line((190.0, 110.0).into(), (280.0, 70.0).into());
    if let Some(sp) = open.subpaths.first_mut() {
        sp.anchors.push(vectorcraft_geom::Anchor::corner((200.0, 70.0).into()));
        sp.closed = false;
    }
    nodes.push(painted(&mut doc, open, vec![fill(gradient_paint(two, None, None))]));
    let mixed = ramp(
        GradientKind::Linear,
        &[(0.0, Color::gray(0.2), 1.0, 0.5), (0.5, Color::cmyk(1.0, 0.0, 0.0, 0.0), 1.0, 0.5), (1.0, INK.tinted(0.7), 1.0, 0.5)],
    );
    let mut mixed_paint = gradient_paint(mixed, None, None);
    if let Paint::Gradient(g) = &mut mixed_paint
        && let Some(s) = g.gradient.stops.last_mut()
    {
        s.set_color(INK.tinted(0.7), Some(("Ink".into(), 0.7)));
    }
    nodes.push(painted(&mut doc, shapes::rectangle(Rect::new(10.0, 130.0, 90.0, 170.0)), vec![fill(mixed_paint)]));
    let turned = Affine::translate((103.0, 127.0)) * Affine::rotate(30f64.to_radians()) * Affine::scale(0.5);
    nodes.push(painted(
        &mut doc,
        shapes::rectangle(Rect::new(100.0, 130.0, 180.0, 170.0)),
        vec![fill(Paint::Pattern { pattern: "Dots".into(), xf: turned })],
    ));
    nodes.push(painted(
        &mut doc,
        shapes::rectangle(Rect::new(195.0, 135.0, 235.0, 165.0)),
        vec![stroke(Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }, 8.0)],
    ));
    for n in nodes {
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    doc
}

/// A gradient paint's swatch link in `doc`: the gradient swatch it names, `-` for none, and
/// `not a gradient swatch: name` when it names a swatch of other paint. A link that names no swatch
/// of `doc` at all is `-` too: the reader links every gradient instance to its definition's name,
/// and a definition of a gradient that isn't a swatch has a name no swatch has.
fn link(doc: &Document, swatch: &Option<String>) -> String {
    match swatch.as_deref().map(|s| (s, doc.swatch(s))) {
        Some((s, Some(w))) if matches!(w.paint, Paint::Gradient(_)) => s.to_string(),
        Some((s, Some(_))) => format!("not a gradient swatch: {s}"),
        Some((_, None)) | None => "-".into(),
    }
}

/// Paint `p` as it draws on `bounds`: a gradient by its kind, stops (place, color, opacity,
/// midpoint, link) and its parameter at the corners, edge midpoints and center of `bounds`; a
/// pattern by its name and matrix.
fn drawn(doc: &Document, p: &Paint, bounds: Rect) -> String {
    match p {
        Paint::Gradient(g) => {
            let geom = g.resolve(bounds);
            let stops: Vec<String> = g
                .gradient
                .stops
                .iter()
                .map(|s| {
                    format!(
                        "{} {} {} {} {:?}",
                        r3(f64::from(s.offset)),
                        color(&s.color),
                        r3(f64::from(s.opacity)),
                        r3(f64::from(s.midpoint)),
                        s.swatch
                    )
                })
                .collect();
            let (x, y) = ([bounds.x0, bounds.center().x, bounds.x1], [bounds.y0, bounds.center().y, bounds.y1]);
            let at: Vec<String> =
                x.iter().flat_map(|x| y.iter().map(move |y| (*x, *y))).map(|(x, y)| r3(geom.param_at(g.gradient.kind, (x, y).into()))).collect();
            format!("{:?} [{}] link={} at {}", g.gradient.kind, stops.join(", "), link(doc, &g.swatch), at.join(" "))
        }
        Paint::Pattern { pattern, xf } => format!("pattern {pattern} {:?}", xf.as_coeffs().map(r3)),
        other => paint(other),
    }
}

/// What each object of `nodes` draws: its kind, bounds and paint.
fn paints_of(doc: &Document, nodes: &[Arc<Node>], depth: usize, out: &mut Vec<String>) {
    for n in nodes {
        let bounds = n.geometric_bounds().unwrap_or(Rect::ZERO);
        let mut s = format!("{depth} {} {:?}", n.kind_label(), [bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(r3));
        // Items that paint nothing aren't written.
        for it in n.appearance.items.iter().filter(|i| !i.paint().is_none()) {
            s += &match it {
                AppearanceItem::Fill(f) => format!(" fill({})", drawn(doc, &f.paint, bounds)),
                AppearanceItem::Stroke(st) => format!(" stroke({} w={})", drawn(doc, &st.paint, st.paint_bounds(bounds)), r3(st.width)),
            };
        }
        out.push(s);
        // A compound path's paint is its own, not its members'.
        if !matches!(n.kind, NodeKind::Compound { .. }) {
            paints_of(doc, n.children().map(Vec::as_slice).unwrap_or_default(), depth + 1, out);
        }
    }
}

/// What `doc`'s art, patterns and gradient and pattern swatches draw.
fn gradient_looks(doc: &Document) -> Vec<String> {
    let mut out = vec![];
    paints_of(doc, &doc.layers, 0, &mut out);
    for p in doc.patterns.iter() {
        out.push(format!("pattern {} tile {:?}", p.name, [p.tile.x0, p.tile.y0, p.tile.x1, p.tile.y1].map(r3)));
        paints_of(doc, &p.art, 1, &mut out);
    }
    for s in doc.swatches_iter() {
        match &s.paint {
            Paint::Gradient(g) => {
                out.push(format!("swatch {} {}", s.name, drawn(doc, &Paint::Gradient(Box::new(GradientPaint::new(g.gradient.clone()))), Rect::ZERO)))
            }
            Paint::Pattern { pattern, .. } => out.push(format!("swatch {} pattern {pattern}", s.name)),
            _ => {}
        }
    }
    out
}

#[test]
fn gradients_and_patterns_are_read_back() {
    let doc = gradients_and_patterns();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    same_lines(&gradient_looks(&read_back(&doc)), &gradient_looks(&doc));
}

#[test]
fn gradients_and_patterns_write_the_same_bytes_again() {
    let doc = gradients_and_patterns();
    let first = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(first.program, export_ai_editing(&doc, &opts()).unwrap().program);
    let again = export_ai_editing(&read_back(&doc), &opts()).unwrap();
    same_lines(
        &String::from_utf8(again.program).unwrap().lines().map(str::to_string).collect::<Vec<_>>(),
        &String::from_utf8(first.program).unwrap().lines().map(str::to_string).collect::<Vec<_>>(),
    );
}

#[test]
fn gradients_and_patterns_are_written_in_their_forms() {
    let text = program(&gradients_and_patterns());
    // The gradients the art uses, then the one only the palette uses; then the patterns.
    let setup = text.split("%%BeginSetup\n").nth(1).and_then(|s| s.split("%AI5_BeginPalette").next()).unwrap();
    let markers: Vec<&str> = setup.lines().filter(|l| l.starts_with("%AI") || l.ends_with(" Bn")).collect();
    assert_eq!(
        markers,
        [
            "4 Bn",
            "%AI5_BeginGradient: (Sunset)",
            "%AI5_EndGradient",
            "%AI5_BeginGradient: (Gradient)",
            "%AI5_EndGradient",
            "%AI5_BeginGradient: (Gradient 2)",
            "%AI5_EndGradient",
            "%AI5_BeginGradient: (Gradient 3)",
            "%AI5_EndGradient",
            "%AI5_Begin_NonPrinting",
            "1 Bn",
            "%AI5_BeginGradient: (Glow)",
            "%AI5_EndGradient",
            "%AI5_End_NonPrinting--",
            "%AI3_BeginPattern: (Dots)",
            "%AI3_EndPattern",
            "%AI5_Begin_NonPrinting",
            "%AI3_BeginPattern: (Checks)",
            "%AI3_EndPattern",
            "%AI5_End_NonPrinting--",
        ]
    );
    // A linear gradient's stops from the last; RGB with its CMYK equivalent, style 2, opacity, 6,
    // midpoint, place on the ramp.
    assert!(text.contains("(Sunset) 0 3 Bd\n[\n%_"), "{setup}");
    assert!(
        setup.contains(" 0 0 1 2 1 6 50 100 Bs\n") && setup.contains(" 0 1 0 2 0.5 6 50 40 Bs\n") && setup.contains(" 1 0 0 2 1 6 30 0 Bs\n"),
        "{setup}"
    );
    // Gray, CMYK, and a spot color at 70%.
    assert!(
        setup.contains("%_0.8 0 1 6 50 0 Bs\n%_1 0 0 0 1 1 6 50 50 Bs\n")
            || setup.contains("(Ink) 0.3 3 1 6 50 100 Bs\n%_1 0 0 0 1 1 6 50 50 Bs\n%_0.8 0 1 6 50 0 Bs\n"),
        "{setup}"
    );
    // The tile in the pattern's space, y up, the art on `%_` lines.
    assert!(setup.contains("%AI3_BeginPattern: (Dots)\n(Dots) 0 -10 10 0\n%_"), "{setup}");
    assert!(!setup.contains("%_E\n") && setup.contains("\nE\n%AI3_EndPattern\n"), "{setup}");
    // The swatches.
    assert!(text.contains("Bb\n2 (Sunset) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n(Sunset) Pc\n"), "{text}");
    assert!(text.contains("Bb\n0 0 0 0 Bh\n2 (Glow) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n(Glow) Pc\n"), "{text}");
    assert!(text.contains("(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n(Dots) Pc\n"), "{text}");
    // The instances: the stroked rectangle's fill placed at 29.9° from 0.15 of the rectangle's extent
    // that way, 0.52 of it long, with `Xm`, caps and a `Bm` for each pair of stops, `b` and `2 BB`; the
    // stroke's `1 Bb`; the radial's focal point and `Bm`.
    let art = text.split("%AI5_BeginLayer").nth(1).unwrap();
    assert!(art.contains("Bb\n1 (Sunset) 0.1529 0 29.8989 0.5167 1 0 0 1 0 0 1 Bg\n") && art.contains(" Xm\n"), "{art}");
    let sunset_instance = art.split("Bb\n1 (Sunset)").nth(1).unwrap();
    let ops: Vec<&str> = sunset_instance.lines().take(7).map(|l| l.rsplit(' ').next().unwrap_or(l)).collect();
    assert_eq!(ops, ["Bg", "Xm", "Bc", "Bm", "Bm", "Bc", "b"], "{sunset_instance}");
    assert!(sunset_instance.contains("\nb\n2 BB\n"), "{sunset_instance}");
    assert!(art.contains("1 Bb\n1 (Gradient 2) "), "{art}");
    assert!(art.contains("2 BB\n1 Bb\n"), "fill and stroke: {art}");
    // The radial (40 wide, 20 high) as the unit circle through its matrix: the focal point 10 right of
    // the center and 5 up, at 45° and 0.35 of the radius on that circle.
    assert!(art.contains("Bb\n10 5 45 0.3536 Bh\n1 (Gradient) 0 0 0 1 40 0 0 20 0 0 1 Bg\n40 0 0 -20 140 -30 Bm\nf\n0 BB\n"), "{art}");
    // The tile's box first in a pattern's definition.
    assert!(setup.contains("(Dots) 0 -10 10 0\n%_0 A\n%_0 Xw\n%_0 XR\n%_0 0 m\n"), "{setup}");
    assert!(setup.contains("%_n\n%_/ArtDictionary :\n%_1 /Bool (AIPattern_Editor_Backing_Tile_Rect) ,\n%_;\n%_\n"), "{setup}");
    assert!(art.contains("F\n0 BB\n"), "the open path: {art}");
    assert!(art.contains(" p\n") && art.contains(" P\n"), "{art}");
}

/// A clipping path filled with a gradient keeps it: the reader reads it as the clipping path and
/// its fill as a path of its own under the art.
#[test]
fn a_clipping_path_keeps_its_gradient() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut clip = clipping(&mut doc, shapes::ellipse(Rect::new(10.0, 10.0, 90.0, 60.0)));
    clip.appearance = Appearance::basic(gradient_paint(sunset(), None, None), Paint::None, 0.0).into();
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 50.0, 40.0), Color::rgb(0.0, 0.0, 1.0));
    let g = group(&mut doc, vec![clip, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::CLIP_PAINT), "{:?}", out.warnings);
    let text = program(&doc);
    assert!(text.contains("h\nW\nBb\n0 (Gradient) "), "{text}");
    let back = read_back(&doc);
    let NodeKind::Group { children, clip: true } = &back.layers[0].children().unwrap()[0].kind else { panic!("a clipping group") };
    let fill = &children[1];
    let bounds = Rect::new(10.0, 10.0, 90.0, 60.0);
    let want = drawn(&doc, &gradient_paint(sunset(), None, None), bounds);
    assert_eq!(drawn(&back, fill.appearance.items[0].paint(), bounds), want);
}

#[test]
fn freeform_gradients_are_written_as_their_average_color() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let g = ramp(GradientKind::Freeform, &[(0.0, Color::rgb(1.0, 1.0, 1.0), 1.0, 0.5), (1.0, Color::rgb(0.0, 0.0, 0.0), 1.0, 0.5)]);
    let n = painted(
        &mut doc,
        shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)),
        vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(g, None, None)))],
    );
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::GRADIENTS), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(paint(back.layers[0].children().unwrap()[0].appearance.items[0].paint()), paint(&solid(Color::rgb(0.5, 0.5, 0.5))));
}

/// A gradient that dithers or mixes perceptually is written as a classic one; a stop linked to a
/// global process color is written as its color; each with a warning.
#[test]
fn gradient_options_and_links_the_data_cant_hold_are_reported() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut g = sunset();
    g.dither = true;
    let n = painted(
        &mut doc,
        shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)),
        vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(g, None, None)))],
    );
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::GRADIENT_MIX]);
    // A swatch of such a gradient that nothing uses.
    let mut swatch_only = Document::new(200.0, 100.0);
    let mut dithered = sunset();
    dithered.dither = true;
    let mut swatches: Vec<Swatch> = swatch_only.swatches.iter().cloned().collect();
    swatches.push(Swatch { name: "Dithered".into(), paint: gradient_paint(dithered, None, None), global: false, spot: false });
    swatch_only.swatches = swatches.into();
    let out = export_ai_editing(&swatch_only, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::GRADIENT_MIX), "{:?}", out.warnings);
    // A gradient along a stroke: the stroke's outline in its average color.
    let mut along = Document::new(200.0, 100.0);
    let layer = along.layers[0].id;
    let mut st = StrokeLayer::new(gradient_paint(sunset(), None, None), 4.0);
    st.gradient_mode = vectorcraft_doc::StrokeGradientMode::Along;
    let n = painted(&mut along, shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), vec![AppearanceItem::Stroke(st)]);
    along.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&along, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::GRADIENTS), "{:?}", out.warnings);
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
    swatches.push(Swatch { name: "Brand".into(), paint: solid(BRAND), global: true, spot: false });
    doc.swatches = swatches.into();
    let mut g = sunset();
    g.method = vectorcraft_color::gradient::GradientMethod::Perceptual;
    if let Some(s) = g.stops.first_mut() {
        s.set_color(BRAND, Some(("Brand".into(), 1.0)));
    }
    let n = painted(
        &mut doc,
        shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)),
        vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(g, None, None)))],
    );
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::GRADIENT_MIX, writer::STOP_LINKS]);
    let back = read_back(&doc);
    let Paint::Gradient(g) = back.layers[0].children().unwrap()[0].appearance.items[0].paint() else { panic!("a gradient") };
    assert_eq!((color(&g.gradient.stops[0].color), g.gradient.stops[0].swatch.as_deref()), (color(&BRAND), None));
}

/// A pattern whose art reaches past its tile is written as its tiles cut to the shape: a definition
/// would cut its art off at the tile's edge.
#[test]
fn a_pattern_whose_art_reaches_past_its_tile_is_written_as_its_tiles() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let big = square(&mut doc, Rect::new(-2.0, -2.0, 6.0, 6.0), Color::rgb(1.0, 0.0, 0.0));
    let mut over = vectorcraft_doc::PatternDef::new("Over", vec![Arc::new(big)]);
    over.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
    doc.patterns.push(over);
    let n = painted(
        &mut doc,
        shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)),
        vec![AppearanceItem::Fill(FillLayer::new(Paint::Pattern { pattern: "Over".into(), xf: Affine::IDENTITY }))],
    );
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PATTERNS), "{:?}", out.warnings);
    assert!(!program(&doc).contains("%AI3_BeginPattern"));
}

/// A pattern fill in another pattern's tile is written there as its tiles cut to the shape; the
/// outer pattern is defined and fills the shape.
#[test]
fn a_pattern_fill_in_a_pattern_tile_is_written_as_its_tiles() {
    let mut doc = gradients_and_patterns();
    let layer = doc.layers[0].id;
    let inner = painted(
        &mut doc,
        shapes::rectangle(Rect::new(2.0, 2.0, 18.0, 18.0)),
        vec![AppearanceItem::Fill(FillLayer::new(Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }))],
    );
    let mut nest = vectorcraft_doc::PatternDef::new("Nest", vec![Arc::new(inner)]);
    nest.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    doc.patterns.push(nest);
    let n = painted(
        &mut doc,
        shapes::rectangle(Rect::new(250.0, 130.0, 290.0, 170.0)),
        vec![AppearanceItem::Fill(FillLayer::new(Paint::Pattern { pattern: "Nest".into(), xf: Affine::IDENTITY }))],
    );
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PATTERNS), "{:?}", out.warnings);
    let text = program(&doc);
    assert!(text.contains("%AI3_BeginPattern: (Nest)") && text.contains("%AI3_BeginPattern: (Dots)"));
    let back = read_back(&doc);
    let filled = back.layers[0].children().unwrap().last().unwrap();
    assert!(matches!(filled.appearance.items[0].paint(), Paint::Pattern { pattern, .. } if pattern == "Nest"));
    // Nest's tile: the dots' tiles (a red square, a gradient circle) cut to its square.
    let mut all = vec![];
    paths(&back.pattern("Nest").unwrap().art, &mut all);
    assert!(all.len() > 2, "{}", all.len());
    assert!(all.iter().all(|p| !matches!(p.appearance.items[0].paint(), Paint::Pattern { .. })));
    assert!(all.iter().any(|p| paint(p.appearance.items[0].paint()) == paint(&solid(Color::rgb(1.0, 0.0, 0.0)))));
}

/// A 200 × 100 document with one object of `items` on `path`, and only the None swatch.
fn one_object(path: PathData, items: Vec<AppearanceItem>) -> Document {
    let mut doc = Document::new(200.0, 100.0);
    doc.swatches = doc.swatches.iter().filter(|s| matches!(s.paint, Paint::None)).cloned().collect::<Vec<_>>().into();
    doc.swatch_groups = Default::default();
    let layer = doc.layers[0].id;
    let n = painted(&mut doc, path, items);
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    doc
}

/// What `doc`'s art draws (a palette of only the None swatch reads back as the default swatches).
fn art_looks(doc: &Document) -> Vec<String> {
    let mut out = vec![];
    paints_of(doc, &doc.layers, 0, &mut out);
    out
}

fn swatches_plus(doc: &mut Document, more: Vec<Swatch>) {
    let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
    swatches.extend(more);
    doc.swatches = swatches.into();
}

/// `Bg` places each gradient on its path: a radial one 30 right of and 10 above the middle of a
/// 120 × 60 rectangle, 20 in radius, by its unit circle's matrix, its center's place as fractions of
/// the bounds and its radius as a multiple of √((1.5² + 2²) / 2) (the center's distances to the
/// right side and to the bottom in that circle's units); and a linear one at −45° on a triangle,
/// from a quarter of the triangle's extent that way, half of it long. In a pattern's tile the same
/// two are placed mirrored top to bottom: the radial's center 10 below the middle, its radius a
/// multiple of √((1.5² + 1²) / 2), and the linear one at 45°.
#[test]
fn bg_places_gradients_on_their_paths() {
    use vectorcraft_color::gradient::GradientGeom;
    let radial = ramp(GradientKind::Radial, &[(0.0, Color::rgb(1.0, 1.0, 1.0), 1.0, 0.5), (1.0, Color::rgb(0.0, 0.0, 1.0), 1.0, 0.5)]);
    let off_center = GradientGeom { start: (90.0, 20.0).into(), end: (110.0, 20.0).into(), aspect: 1.0, focal: None };
    let fill = |p: Paint| AppearanceItem::Fill(FillLayer::new(p));
    let mut doc = one_object(shapes::rectangle(Rect::new(0.0, 0.0, 120.0, 60.0)), vec![fill(gradient_paint(radial.clone(), Some(off_center), None))]);
    let layer = doc.layers[0].id;
    let mut triangle = shapes::line((130.0, 10.0).into(), (190.0, 10.0).into());
    if let Some(sp) = triangle.subpaths.first_mut() {
        sp.anchors.push(vectorcraft_geom::Anchor::corner((130.0, 40.0).into()));
        sp.closed = true;
    }
    let turned = GradientGeom { start: (145.0, 10.0).into(), end: (160.0, 25.0).into(), aspect: 1.0, focal: None };
    let n = painted(&mut doc, triangle.clone(), vec![fill(gradient_paint(sunset(), Some(turned), None))]);
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let in_tile = painted(&mut doc, shapes::rectangle(Rect::new(0.0, 0.0, 120.0, 60.0)), vec![fill(gradient_paint(radial, Some(off_center), None))]);
    let slant = painted(&mut doc, triangle, vec![fill(gradient_paint(sunset(), Some(turned), None))]);
    let mut tile = vectorcraft_doc::PatternDef::new("Tile", vec![Arc::new(in_tile), Arc::new(slant)]);
    tile.tile = Rect::new(0.0, 0.0, 200.0, 60.0);
    doc.patterns.push(tile);
    swatches_plus(
        &mut doc,
        vec![Swatch { name: "Tile".into(), paint: Paint::Pattern { pattern: "Tile".into(), xf: Affine::IDENTITY }, global: false, spot: false }],
    );
    let n = painted(
        &mut doc,
        shapes::rectangle(Rect::new(130.0, 50.0, 190.0, 90.0)),
        vec![fill(Paint::Pattern { pattern: "Tile".into(), xf: Affine::IDENTITY })],
    );
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let text = program(&doc);
    let art = text.split("%AI5_BeginLayer").nth(1).unwrap();
    assert!(art.contains("Bb\n0 0 0 0 Bh\n1 (Gradient) 0.25 -0.1667 0 0.5657 20 0 0 20 0 0 1 Bg\n"), "{art}");
    assert!(art.contains("Bb\n1 (Gradient 2) 0.25 0 -45 0.5 1 0 0 1 0 0 1 Bg\n"), "{art}");
    let setup = text.split("%AI3_BeginPattern: (Tile)").nth(1).unwrap();
    assert!(setup.contains("%_Bb\n%_0 0 0 0 Bh\n%_1 (Gradient) 0.25 0.1667 0 0.7845 20 0 0 20 0 0 1 Bg\n"), "{setup}");
    assert!(setup.contains("%_Bb\n%_1 (Gradient 2) 0.25 0 45 0.5 1 0 0 1 0 0 1 Bg\n"), "{setup}");
    same_lines(&gradient_looks(&read_back(&doc)), &gradient_looks(&doc));
}

/// A definition of a gradient that isn't a swatch takes a name that no swatch and no pattern has,
/// so that its instances don't read back linked to another swatch.
#[test]
fn gradient_definitions_skip_the_names_of_swatches_and_patterns() {
    let mut doc = gradients_and_patterns();
    let check = square(&mut doc, Rect::new(0.0, 0.0, 5.0, 5.0), Color::BLACK);
    let mut taken = vectorcraft_doc::PatternDef::new("Gradient 2", vec![Arc::new(check)]);
    taken.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
    doc.patterns.push(taken);
    swatches_plus(
        &mut doc,
        vec![
            Swatch { name: "Gradient".into(), paint: solid(Color::rgb(0.0, 0.5, 0.0)), global: false, spot: false },
            Swatch {
                name: "Gradient 2".into(),
                paint: Paint::Pattern { pattern: "Gradient 2".into(), xf: Affine::IDENTITY },
                global: false,
                spot: false,
            },
        ],
    );
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let text = program(&doc);
    let names: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("%AI5_BeginGradient: ")).collect();
    assert_eq!(names, ["(Sunset)", "(Gradient 3)", "(Gradient 4)", "(Gradient 5)", "(Glow)"]);
    same_lines(&gradient_looks(&read_back(&doc)), &gradient_looks(&doc));
}

/// Each search for a new gradient name starts after the last name given, so naming many gradients
/// tries each name once.
#[test]
fn gradient_names_are_searched_from_the_last_one_given() {
    let mut taken: std::collections::HashSet<String> = ["Gradient 2", "Gradient 5"].map(String::from).into();
    let (mut next, mut tries) = (1, 0);
    let mut names = vec![];
    for _ in 0..1000 {
        let name = writer::free_gradient_name(&mut next, |name| {
            tries += 1;
            taken.contains(name)
        });
        taken.insert(name.clone());
        names.push(name);
    }
    assert_eq!(names[..4], ["Gradient", "Gradient 3", "Gradient 4", "Gradient 6"]);
    assert_eq!(names.last().map(String::as_str), Some("Gradient 1002"));
    assert_eq!(tries, 1002);
    assert_eq!(next, 1003);
}

/// Many gradients that no swatch names each get their own name, in order, skipping the names of
/// swatches and patterns, and the same names when the document is written again.
#[test]
fn many_gradient_definitions_take_distinct_names_in_order() {
    let mut doc = one_object(shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), vec![]);
    let check = square(&mut doc, Rect::new(0.0, 0.0, 5.0, 5.0), Color::BLACK);
    let mut taken = vectorcraft_doc::PatternDef::new("Gradient 7", vec![Arc::new(check)]);
    taken.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
    doc.patterns.push(taken);
    swatches_plus(&mut doc, vec![Swatch { name: "Gradient 3".into(), paint: solid(Color::rgb(0.0, 0.5, 0.0)), global: false, spot: false }]);
    let layer = doc.layers[0].id;
    for i in 0..300u16 {
        let shade = f32::from(i) / 300.0;
        let g = ramp(GradientKind::Linear, &[(0.0, Color::rgb(shade, 0.0, 0.0), 1.0, 0.5), (1.0, Color::rgb(0.0, 0.0, 1.0), 1.0, 0.5)]);
        let x = f64::from(i % 20) * 10.0;
        let y = f64::from(i / 20) * 6.0;
        let n = painted(
            &mut doc,
            shapes::rectangle(Rect::new(x, y, x + 8.0, y + 5.0)),
            vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(g, None, None)))],
        );
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let text = program(&doc);
    let names: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("%AI5_BeginGradient: ")).collect();
    let expected: Vec<String> =
        std::iter::once("(Gradient)".to_string()).chain((2..=302).filter(|&i| i != 3 && i != 7).map(|i| format!("(Gradient {i})"))).collect();
    assert_eq!(names, expected);
    assert_eq!(text, program(&doc));
}

/// A hard edge: a linear gradient whose stops all lie at one place is written from its last stop,
/// and reads back with each stop on its side.
#[test]
fn a_linear_gradient_with_every_stop_at_one_place_keeps_its_sides() {
    let edge = ramp(GradientKind::Linear, &[(0.3, Color::rgb(1.0, 0.0, 0.0), 1.0, 0.4), (0.3, Color::rgb(0.0, 0.0, 1.0), 1.0, 0.7)]);
    let doc = one_object(
        shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 50.0)),
        vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(edge, None, None)))],
    );
    let text = program(&doc);
    let (blue, red) = (text.find(" 0 0 1 2 1 6 70 30 Bs\n").unwrap(), text.find(" 1 0 0 2 1 6 40 30 Bs\n").unwrap());
    assert!(blue < red, "{text}");
    same_lines(&art_looks(&read_back(&doc)), &art_looks(&doc));
}

/// Gradients without stops or with more than 256, without a ramp's length or flattened to a line
/// are written as their average color with a warning that says so, not the one for freeform
/// gradients.
#[test]
fn gradients_that_cant_be_placed_are_reported_as_such() {
    use vectorcraft_color::gradient::GradientGeom;
    let point = GradientGeom { start: (20.0, 20.0).into(), end: (20.0, 20.0).into(), aspect: 1.0, focal: None };
    let flat = GradientGeom { start: (20.0, 20.0).into(), end: (40.0, 20.0).into(), aspect: 0.0, focal: None };
    let radial = ramp(GradientKind::Radial, &[(0.0, Color::rgb(1.0, 1.0, 1.0), 1.0, 0.5), (1.0, Color::rgb(0.0, 0.0, 0.0), 1.0, 0.5)]);
    // More stops than the `.ai` reader reads.
    let many: Vec<(f32, Color, f32, f32)> = (0..257).map(|i| (i as f32 / 256.0, Color::gray(i as f32 / 256.0), 1.0, 0.5)).collect();
    for p in [
        gradient_paint(sunset(), Some(point), None),
        gradient_paint(radial, Some(flat), None),
        gradient_paint(Gradient::new(GradientKind::Linear, vec![]), None, None),
        gradient_paint(ramp(GradientKind::Linear, &many), None, None),
    ] {
        let doc = one_object(shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), vec![AppearanceItem::Fill(FillLayer::new(p.clone()))]);
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert_eq!(out.warnings, [writer::GRADIENT_SHAPES], "{p:?}");
        let back = read_back(&doc);
        assert!(matches!(back.layers[0].children().unwrap()[0].appearance.items[0].paint(), Paint::Solid { .. }), "{p:?}");
    }
}

/// A gradient across a stroke is written as the stroke's outline in its average color, with the
/// warning.
#[test]
fn a_gradient_across_a_stroke_is_its_average_color() {
    let mut st = StrokeLayer::new(gradient_paint(sunset(), None, None), 4.0);
    st.gradient_mode = vectorcraft_doc::StrokeGradientMode::Across;
    let doc = one_object(shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), vec![AppearanceItem::Stroke(st)]);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::GRADIENTS), "{:?}", out.warnings);
    assert!(!program(&doc).contains("Bb\n"));
    let mut all = vec![];
    paths(read_back(&doc).layers[0].children().unwrap(), &mut all);
    assert!(!all.is_empty() && all.iter().all(|p| p.appearance.items.iter().all(|i| matches!(i.paint(), Paint::Solid { .. }))), "{all:?}");
}

/// Patterns whose tiles are hexagons or bricks are written as their tiles cut to the shape, with the
/// warning, and aren't defined.
#[test]
fn hex_and_brick_patterns_are_written_as_their_tiles() {
    for tile_type in [
        vectorcraft_doc::TileType::HexByRow,
        vectorcraft_doc::TileType::HexByColumn,
        vectorcraft_doc::TileType::BrickByRow { offset: 0.5 },
        vectorcraft_doc::TileType::BrickByColumn { offset: 0.5 },
    ] {
        let mut doc = one_object(
            shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)),
            vec![AppearanceItem::Fill(FillLayer::new(Paint::Pattern { pattern: "Cells".into(), xf: Affine::IDENTITY }))],
        );
        let dot = square(&mut doc, Rect::new(0.0, 0.0, 4.0, 4.0), Color::rgb(1.0, 0.0, 0.0));
        doc.patterns.push(vectorcraft_doc::PatternDef {
            tile: Rect::new(0.0, 0.0, 10.0, 10.0),
            tile_type,
            ..vectorcraft_doc::PatternDef::new("Cells", vec![Arc::new(dot)])
        });
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert!(out.warnings.iter().any(|w| w == writer::PATTERNS), "{tile_type:?}: {:?}", out.warnings);
        assert!(!program(&doc).contains("%AI3_BeginPattern"), "{tile_type:?}");
        let mut all = vec![];
        paths(read_back(&doc).layers[0].children().unwrap(), &mut all);
        let red = paint(&solid(Color::rgb(1.0, 0.0, 0.0)));
        assert!(all.iter().any(|p| p.appearance.items.iter().any(|i| paint(i.paint()) == red)), "{tile_type:?}");
    }
}

/// An open path filled and stroked with gradients: the fill's instance ends `1 BB`, the stroke's
/// `1 Bb` follows, and the open path's `B` ends `1 BB`.
#[test]
fn an_open_path_keeps_its_gradient_fill_and_stroke() {
    let two = ramp(GradientKind::Linear, &[(0.0, Color::rgb(1.0, 1.0, 0.0), 1.0, 0.5), (1.0, Color::rgb(0.0, 0.0, 0.0), 1.0, 0.5)]);
    let mut open = shapes::line((20.0, 80.0).into(), (100.0, 20.0).into());
    if let Some(sp) = open.subpaths.first_mut() {
        sp.anchors.push(vectorcraft_geom::Anchor::corner((150.0, 70.0).into()));
        sp.closed = false;
    }
    let doc = one_object(
        open,
        vec![
            AppearanceItem::Fill(FillLayer::new(gradient_paint(sunset(), None, None))),
            AppearanceItem::Stroke(StrokeLayer::new(gradient_paint(two, None, None), 3.0)),
        ],
    );
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let text = program(&doc);
    assert!(text.contains("\n1 BB\n1 Bb\n1 (Gradient 2) ") && text.contains("\nB\n1 BB\n"), "{text}");
    same_lines(&art_looks(&read_back(&doc)), &art_looks(&doc));
}

/// A clipping path stroked with a gradient keeps it.
#[test]
fn a_clipping_path_keeps_its_gradient_stroke() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut clip = clipping(&mut doc, shapes::ellipse(Rect::new(10.0, 10.0, 90.0, 60.0)));
    clip.appearance = Appearance::basic(Paint::None, gradient_paint(sunset(), None, None), 3.0).into();
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 50.0, 40.0), Color::rgb(0.0, 0.0, 1.0));
    let g = group(&mut doc, vec![clip, inside], true);
    doc.insert(Some(layer), usize::MAX, g).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let text = program(&doc);
    assert!(text.contains("h\nW\n1 Bb\n0 (Gradient) "), "{text}");
    let back = read_back(&doc);
    let mut want = vec![];
    paints_of(&doc, doc.layers[0].children().unwrap(), 0, &mut want);
    let mut got = vec![];
    paints_of(&back, back.layers[0].children().unwrap(), 0, &mut got);
    let strokes = |v: &[String]| {
        v.iter().filter(|l| l.contains(" stroke(Linear")).map(|l| l.split(" stroke(").nth(1).unwrap_or_default().to_string()).collect::<Vec<_>>()
    };
    assert_eq!(strokes(&got), strokes(&want), "{got:#?}");
}

/// In a CMYK document the gradient's CMYK stops are written as CMYK and read back the same.
#[test]
fn a_cmyk_documents_gradients_keep_their_stops() {
    let cmyk = ramp(GradientKind::Linear, &[(0.0, Color::cmyk(0.0, 0.6, 1.0, 0.0), 1.0, 0.5), (1.0, Color::cmyk(1.0, 0.0, 0.0, 0.2), 0.5, 0.25)]);
    let mut doc = one_object(
        shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 50.0)),
        vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(cmyk, None, None)))],
    );
    doc.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    let text = program(&doc);
    assert!(text.contains("%AI9_ColorModel: 2\n"), "{text}");
    assert!(text.contains("%_1 0 0 0.2 1 0.5 6 25 100 Bs\n%_0 0.6 1 0 1 1 6 50 0 Bs\n"), "{text}");
    same_lines(&art_looks(&read_back(&doc)), &art_looks(&doc));
}

/// A compound path's gradient instance is on its last subpath only.
#[test]
fn a_compound_paths_gradient_is_on_its_last_subpath() {
    let text = program(&gradients_and_patterns());
    let compound = text.split("*u\n").nth(1).and_then(|s| s.split("*U\n").next()).unwrap();
    assert_eq!(compound.matches("Bb\n").count(), 1, "{compound}");
    let last_subpath = compound.rfind(" m\n").unwrap();
    assert!(compound.find("Bb\n").unwrap() > last_subpath, "{compound}");
    assert_eq!(compound.matches(" m\n").count(), 2, "{compound}");
}

/// Freeform gradient swatches and swatches of patterns that can't be defined are left out with the
/// warning; a color group counts only the swatches written.
#[test]
fn swatches_the_data_cant_hold_are_left_out() {
    let mut doc = gradients_and_patterns();
    let dot = square(&mut doc, Rect::new(0.0, 0.0, 4.0, 4.0), Color::rgb(1.0, 0.0, 0.0));
    doc.patterns.push(vectorcraft_doc::PatternDef {
        tile: Rect::new(0.0, 0.0, 10.0, 10.0),
        tile_type: vectorcraft_doc::TileType::BrickByRow { offset: 0.5 },
        ..vectorcraft_doc::PatternDef::new("Brick", vec![Arc::new(dot)])
    });
    let free = ramp(GradientKind::Freeform, &[(0.0, Color::rgb(1.0, 1.0, 1.0), 1.0, 0.5), (1.0, Color::rgb(0.0, 0.0, 0.0), 1.0, 0.5)]);
    let swatch = |name: &str, paint: Paint| Swatch { name: name.into(), paint, global: false, spot: false };
    doc.swatch_groups = vec![SwatchGroup {
        name: "Mixed".into(),
        swatches: vec![
            swatch("Free", gradient_paint(free.clone(), None, None)),
            swatch("Dawn", gradient_paint(sunset(), None, None)),
            swatch("Brick", Paint::Pattern { pattern: "Brick".into(), xf: Affine::IDENTITY }),
            swatch("Dots 2", Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }),
        ],
    }]
    .into();
    swatches_plus(&mut doc, vec![swatch("Free 2", gradient_paint(free, None, None))]);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::SWATCHES], "{:?}", out.warnings);
    let text = program(&doc);
    assert!(text.contains("0 (Mixed) 2 Pg\n"), "{text}");
    for name in ["(Free) Pc", "(Free 2) Pc", "(Brick) Pc", "%AI3_BeginPattern: (Brick)"] {
        assert!(!text.contains(name), "{name}");
    }
    let back = read_back(&doc);
    let group = back.swatch_groups.iter().find(|g| g.name == "Mixed").unwrap();
    assert_eq!(group.swatches.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Dawn", "Dots 2"]);
}

/// A pattern swatch that moves, turns or scales its pattern is written with its pattern, and the
/// placement is reported left out.
#[test]
fn a_pattern_swatchs_own_placement_is_reported_left_out() {
    let mut doc = gradients_and_patterns();
    let moved = Paint::Pattern { pattern: "Dots".into(), xf: Affine::scale(2.0) };
    swatches_plus(&mut doc, vec![Swatch { name: "Big Dots".into(), paint: moved, global: false, spot: false }]);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::SWATCH_PLACEMENT]);
    assert!(program(&doc).contains("(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n(Big Dots) Pc\n"));
}

// ---------- images and placed files ----------

/// `img` as PNG bytes.
fn png_of(img: image::DynamicImage) -> Vec<u8> {
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

/// An image object of `blob`'s pixels (`w` × `h`) placed by `xf`, its blob kept in `doc`.
fn image_node(doc: &mut Document, blob: ImageBlob, (w, h): (u32, u32), xf: Affine, link: Option<vectorcraft_doc::LinkInfo>) -> Node {
    let key = blob.content_key();
    doc.images.insert(key.clone(), blob);
    let im = ImageObject { key, width: w, height: h, xf, link, placement: Default::default(), pixels: None };
    Node::new(doc.alloc_id(), NodeKind::Image(Box::new(im)))
}

/// A 3 × 2 RGB image, every pixel a different color.
fn rgb_png() -> Vec<u8> {
    let px = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [0, 255, 255], [12, 34, 56]];
    png_of(image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(3, 2, |x, y| image::Rgb(px[(y * 3 + x) as usize]))))
}

/// A 200 × 100 document of images: on a layer `Pictures`, an RGB image `Photo` turned, scaled and
/// moved, at half opacity in Multiply; a locked RGBA image whose pixels are see-through in part; a
/// hidden gray image; a CMYK image; a clipping group of the RGB image cut by a rectangle; a square
/// whose opacity mask is a gray image. On a layer `Links`, a linked image in a folder next to the
/// document's.
fn images_doc() -> Document {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let turned = Affine::translate((40.0, 30.0)) * Affine::rotate(0.5) * Affine::scale_non_uniform(8.0, 6.0);
    let mut photo = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), turned, None);
    photo.name = Some("Photo".into());
    photo.opacity = 0.5;
    photo.blend = BlendMode::Multiply;
    doc.insert(Some(layer), usize::MAX, photo).unwrap();
    let alpha = [0, 128, 255, 200];
    let rgba = image::RgbaImage::from_fn(2, 2, |x, y| image::Rgba([200, 100, (x * 50) as u8, alpha[(y * 2 + x) as usize]]));
    let mut see = image_node(&mut doc, ImageBlob::png(png_of(rgba.into())), (2, 2), Affine::new([10.0, 0.0, 0.0, 10.0, 60.0, 10.0]), None);
    see.locked = true;
    doc.insert(Some(layer), usize::MAX, see).unwrap();
    let gray = image::GrayImage::from_fn(4, 1, |x, _| image::Luma([(x * 60) as u8]));
    let mut gray = image_node(&mut doc, ImageBlob::png(png_of(gray.into())), (4, 1), Affine::new([5.0, 0.0, 0.0, 5.0, 90.0, 60.0]), None);
    gray.visible = false;
    doc.insert(Some(layer), usize::MAX, gray).unwrap();
    let inks = vectorcraft_doc::cmyk::Inks::new(2, 1, vec![255, 0, 0, 0, 10, 20, 30, 40]).unwrap();
    let cmyk = image_node(&mut doc, ImageBlob::cmyk_tiff(&inks).unwrap(), (2, 1), Affine::new([10.0, 0.0, 0.0, 10.0, 120.0, 10.0]), None);
    doc.insert(Some(layer), usize::MAX, cmyk).unwrap();
    let clip = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(150.0, 10.0, 170.0, 25.0)), Appearance::default());
    let inside = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::new([10.0, 0.0, 0.0, 10.0, 145.0, 5.0]), None);
    let group = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(inside)], clip: true });
    doc.insert(Some(layer), usize::MAX, group).unwrap();
    let ramp = image::GrayImage::from_fn(3, 3, |x, y| image::Luma([(x * 100 + y * 20) as u8]));
    let mask_art = image_node(&mut doc, ImageBlob::png(png_of(ramp.into())), (3, 3), Affine::new([10.0, 0.0, 0.0, 10.0, 10.0, 60.0]), None);
    let mut square = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(10.0, 60.0, 40.0, 90.0)),
        Appearance::basic(solid(Color::rgb(0.0, 0.5, 1.0)), Paint::None, 0.0),
    );
    square.extras_mut().mask = Some(Box::new(OpacityMask::new(mask_art, false)));
    doc.insert(Some(layer), usize::MAX, square).unwrap();
    let links = Node::layer(doc.alloc_id(), "Links", LayerColor::Preset(1));
    let links_id = links.id;
    doc.layers.push(Arc::new(links));
    let pic = image::RgbaImage::from_fn(3, 3, |x, y| image::Rgba([(x * 80) as u8, (y * 80) as u8, 30, 255]));
    let mut link = vectorcraft_doc::LinkInfo::new("/Users/someone/Job (1)/art/pic.png");
    link.relative = Some("art/pic.png".into());
    let placed = image_node(&mut doc, ImageBlob::png(png_of(pic.into())), (3, 3), Affine::new([4.0, 1.0, -1.0, 4.0, 60.0, 50.0]), Some(link));
    doc.insert(Some(links_id), usize::MAX, placed).unwrap();
    doc
}

/// Each image in `nodes` (opacity masks' art included), in order: where it is, its state, size,
/// placement, link and pixels (RGBA, or a CMYK image's ink amounts).
fn pictures(doc: &Document, nodes: &[Arc<Node>], path: &str, out: &mut Vec<String>) {
    for (i, n) in nodes.iter().enumerate() {
        let at = format!("{path}/{i}");
        if let NodeKind::Image(im) = &n.kind {
            let blob = doc.image_blob(im).unwrap();
            let pixels = match blob.cmyk() {
                Some(inks) if blob.is_cmyk() => format!("cmyk {:?}", inks.data),
                _ => format!("rgba {:?}", image::load_from_memory(&blob.bytes).unwrap().to_rgba8().into_raw()),
            };
            let xf: Vec<String> = im.xf.as_coeffs().iter().map(|v| r3(*v)).collect();
            let link = im.link.as_ref().map(|l| (l.path.clone(), l.relative.clone()));
            out.push(format!(
                "{at} {:?} shown={} locked={} opacity={} {:?} {}x{} [{}] {link:?} {pixels}",
                n.name,
                n.visible,
                n.locked,
                r3(f64::from(n.opacity)),
                n.blend,
                im.width,
                im.height,
                xf.join(" ")
            ));
        }
        if let Some(m) = &n.extras().mask {
            pictures(doc, std::slice::from_ref(&m.art), &format!("{at}/mask"), out);
        }
        if let Some(c) = n.children() {
            pictures(doc, c, &at, out);
        }
    }
}

fn images_of(doc: &Document) -> Vec<String> {
    let mut out = vec![];
    pictures(doc, &doc.layers, "", &mut out);
    out
}

#[test]
fn images_come_back_with_their_pixels_and_places() {
    let doc = images_doc();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    let want = images_of(&doc);
    let got = images_of(&back);
    assert_eq!(want.len(), 7);
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g, w);
    }
    assert_eq!(got.len(), want.len(), "{got:?}");
    // The clipping group and the masked square are there as themselves.
    assert_eq!(lines(&back).len(), lines(&doc).len(), "{:?}", lines(&back));
}

/// An image whose opacity defines the knockout shape comes back with it, embedded or linked. The
/// image after it and their layer do not take it.
#[test]
fn an_images_knockout_shape_comes_back() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let at = |x: f64| Affine::new([10.0, 0.0, 0.0, 10.0, x, 10.0]);
    let mut shaped = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), at(10.0), None);
    (shaped.opacity, shaped.knockout_shape) = (0.5, true);
    doc.insert(Some(layer), usize::MAX, shaped).unwrap();
    let plain = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), at(50.0), None);
    doc.insert(Some(layer), usize::MAX, plain).unwrap();
    let mut link = vectorcraft_doc::LinkInfo::new("/Users/someone/Job/art/pic.png");
    link.relative = Some("art/pic.png".into());
    let mut linked = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), at(90.0), Some(link));
    linked.knockout_shape = true;
    doc.insert(Some(layer), usize::MAX, linked).unwrap();
    let back = read_back(&doc);
    let art = back.layers[0].children().unwrap();
    let got: Vec<_> = art.iter().map(|n| (matches!(n.kind, NodeKind::Image(_)), n.opacity, n.knockout_shape)).collect();
    assert_eq!(got, [(true, 0.5, true), (true, 1.0, false), (true, 1.0, true)]);
    assert_eq!((back.layers[0].opacity, back.layers[0].knockout_shape), (1.0, false));
}

#[test]
fn images_are_written_again_as_the_same_bytes() {
    let mut doc = images_doc();
    // The default swatches' RGB colors come back a ten-thousandth off in their CMYK equivalents.
    doc.swatches.retain(|s| matches!(s.paint, Paint::None));
    doc.swatches.push(Swatch { name: "Gold".into(), paint: solid(GOLD), global: false, spot: false });
    doc.swatch_groups = Default::default();
    let first = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(first.program, export_ai_editing(&doc, &opts()).unwrap().program);
    let again = export_ai_editing(&read_back(&doc), &opts()).unwrap().program;
    for (a, b) in shown(&again).lines().zip(shown(&first.program).lines()) {
        assert_eq!(a, b);
    }
    assert_eq!(again, first.program);
}

/// The program of `doc` with LF line ends, and each image's samples (from after `XI` and its line
/// end to `%%EndData`, a `%_` before it kept) and each preview's rows (from after `32 17 ` to the
/// line end before `%AI26_EndPlacedObjectPreview`) shown as `<n bytes>`.
fn program_shown(doc: &Document) -> String {
    shown(&export_ai_editing(doc, &opts()).unwrap().program)
}

/// Program `p` as [`program_shown`] shows it.
fn shown(p: &[u8]) -> String {
    let find = |from: usize, what: &[u8]| p.get(from..).and_then(|r| r.windows(what.len()).position(|w| w == what)).map(|i| i + from);
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        let xi = find(at, b"\r\nXI\r\n").map(|i| (i + 6, b"%%EndData".as_slice()));
        let pv = find(at, b" 32 17 ").map(|i| (i + 7, b"\r\n%AI26_EndPlacedObjectPreview".as_slice()));
        let Some((start, end)) = [xi, pv].into_iter().flatten().min_by_key(|(s, _)| *s) else { break };
        let mut stop = find(start, end).unwrap();
        if end.starts_with(b"%%") && p.get(stop - 2..stop) == Some(b"%_") {
            stop -= 2;
        }
        out.extend_from_slice(&p[at..start]);
        out.extend_from_slice(format!("<{} bytes>", stop - start).as_bytes());
        at = stop;
    }
    out.extend_from_slice(&p[at..]);
    String::from_utf8(out).unwrap().replace("\r\n", "\n")
}

#[test]
fn an_image_is_a_raster_with_binary_samples() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let im = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::new([10.0, 0.0, 0.0, 10.0, 20.0, 30.0]), None);
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    let p = export_ai_editing(&doc, &opts()).unwrap().program;
    // `XI`, CR LF and the 18 samples, which are counted, follow the count's line.
    let at = p.windows(17).position(|w| w == b"%%BeginData: 22\r\n").unwrap();
    let samples: Vec<u8> = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 255, 255, 12, 34, 56];
    let mut want = b"XI\r\n".to_vec();
    want.extend_from_slice(&samples);
    want.extend_from_slice(b"%%EndData\r\nXH\r\n%AI5_EndRaster\r\nN\r\n");
    assert_eq!(p.get(at + 17..at + 17 + want.len()), Some(want.as_slice()));
    let text = program_shown(&doc);
    // Art space is y up from the artboard's top-left corner, the pixels' rows run downwards.
    assert!(
        text.contains(
            "%AI5_File:\n%AI5_BeginRaster\n() 1 XG\n/DeviceRGB XN\n[10 0 0 10 20 -30] 3 2 0 Xh\n[10 0 0 10 20 -30] 0 0 3 2 3 2 8 3 0 1 1 0 4 4 0 0\n"
        ),
        "{text}"
    );
}

#[test]
fn see_through_gray_and_cmyk_images_have_their_flags() {
    let doc = images_doc();
    let text = program_shown(&doc);
    // RGBA: the alpha channel follows the color samples, which the count doesn't take in.
    assert!(text.contains(" 0 0 2 2 2 2 8 3 1 17 1 0 4 4 0 0\n%%BeginData: 16\nXI\n<16 bytes>%%EndData\n"), "{text}");
    assert!(text.contains("/DeviceGray XN\n"), "{text}");
    assert!(text.contains(" 0 0 4 1 4 1 8 1 0 0 1 0 4 4 0 0\n"), "{text}");
    assert!(text.contains("/DeviceCMYK XN\n"), "{text}");
    assert!(text.contains(" 0 0 2 1 2 1 8 4 0 2 1 0 4 4 0 0\n%%BeginData: 12\nXI\n<8 bytes>%%EndData\n"), "{text}");
    // In an opacity mask: on `%_` lines but for `XI` and its samples.
    assert!(text.contains("%_%%BeginData: 13\nXI\n<9 bytes>%_%%EndData\n%_XH\n%_%AI5_EndRaster\n%_N\n"), "{text}");
}

#[test]
fn a_linked_image_is_a_placed_file_with_a_preview() {
    let doc = images_doc();
    let text = program_shown(&doc);
    let begin = text.find("%AI5_BeginPlace\n").unwrap();
    let place = &text[begin..text[begin..].find("%AI5_EndPlace\n").unwrap() + begin];
    let lines: Vec<&str> = place.lines().collect();
    assert_eq!(lines[1], "%%FileType: 1");
    assert!(lines[2].ends_with("] 8091 8141 8094 8144 4 4 0"), "{place}");
    assert_eq!(lines[3], "(/Users/someone/Job \\(1\\)/art/pic.png)`");
    assert_eq!(lines[4], "%%IncludeFile:/Users/someone/Job (1)/art/pic.png");
    assert_eq!(
        &lines[5..10],
        [
            "%AI17_Begin_Content_if_version_gt:24 12",
            "%AI28_BeginPlacedRelativePath",
            "(art/)",
            "%AI28_EndPlacedRelativePath",
            "%AI17_Alternate_Content"
        ]
    );
    assert_eq!(&lines[11..13], ["%AI17_Begin_Content_if_version_gt:24 5", "%AI26_BeginPlacedObjectPreview"]);
    // 3 × 3 pixels of premultiplied ARGB.
    assert_eq!(lines[13], "3 3 12 32 17 <36 bytes>");
    assert_eq!(&lines[14..], ["%AI26_EndPlacedObjectPreview", "%AI17_End_Versioned_Content", "~"]);
}

/// The relative folder goes with the path's file name, so a relative path to a file of another
/// name isn't written.
#[test]
fn a_relative_path_to_a_file_of_another_name_is_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut link = vectorcraft_doc::LinkInfo::new("/old/original.png");
    link.relative = Some("assets/current.png".into());
    let im = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::scale(10.0), Some(link));
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    assert!(!program_shown(&doc).contains("%AI28_BeginPlacedRelativePath"));
    let back = read_back(&doc);
    let NodeKind::Image(im) = &back.layers[0].children().unwrap()[0].kind else { panic!("not an image") };
    let link = im.link.as_ref().unwrap();
    assert_eq!((link.path.as_str(), link.relative.as_deref()), ("/old/original.png", None));
    // Names that differ only in letter case keep the folder where file names ignore case.
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut link = vectorcraft_doc::LinkInfo::new("C:\\Job\\art\\PHOTO.PNG");
    link.relative = Some("art/photo.png".into());
    let im = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::scale(10.0), Some(link));
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    let kept = program_shown(&doc).contains("%AI28_BeginPlacedRelativePath\n(art/)\n");
    assert_eq!(kept, cfg!(any(windows, target_os = "macos")));
    // A relative path with `\` separators keeps its folder.
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut link = vectorcraft_doc::LinkInfo::new("C:\\Job\\art\\photo.png");
    link.relative = Some("art\\photo.png".into());
    let im = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::scale(10.0), Some(link));
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    assert!(program_shown(&doc).contains("%AI28_BeginPlacedRelativePath\n(art/)\n"));
}

#[test]
fn a_large_linked_image_has_a_smaller_preview_over_its_pixels() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let big = image::RgbaImage::from_fn(1024, 512, |x, y| image::Rgba([(x / 4) as u8, (y / 2) as u8, 0, 255]));
    let xf = Affine::new([0.1, 0.0, 0.0, 0.1, 20.0, 30.0]);
    let im = image_node(&mut doc, ImageBlob::png(png_of(big.into())), (1024, 512), xf, Some(vectorcraft_doc::LinkInfo::new("/tmp/big.png")));
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    assert!(program_shown(&doc).contains("\n256 128 1024 32 17 <131072 bytes>\n"));
    let back = read_back(&doc);
    let NodeKind::Image(im) = &back.layers[0].children().unwrap()[0].kind else { panic!("not an image") };
    assert_eq!((im.width, im.height), (256, 128));
    let bounds = im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, 256.0, 128.0));
    assert_eq!((r3(bounds.x0), r3(bounds.y0), r3(bounds.x1), r3(bounds.y1)), (r3(20.0), r3(30.0), r3(122.4), r3(81.2)));
    assert_eq!(im.link.as_ref().map(|l| l.path.as_str()), Some("/tmp/big.png"));
}

/// Samples that hold the bytes of `%%EndData` without a line end after them are read whole.
#[test]
fn samples_holding_the_end_marker_are_read_whole() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    // The color samples end `%%En` and the alpha channel starts `dData`.
    let rgb = [[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, b'%'], *b"%En"];
    let alpha = *b"dData";
    let img = image::RgbaImage::from_fn(5, 1, |x, _| {
        let [r, g, b] = rgb[x as usize];
        image::Rgba([r, g, b, alpha[x as usize]])
    });
    let im = image_node(&mut doc, ImageBlob::png(png_of(img.into())), (5, 1), Affine::scale(10.0), None);
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    // The alpha channel holds the marker, a line end and the operator that follows the samples.
    let alpha = *b"%%EndData\r\nXH\r\nZZ";
    let img = image::RgbaImage::from_fn(17, 1, |x, _| image::Rgba([x as u8, 9, 9, alpha[x as usize]]));
    let marked = ImageBlob::png(png_of(img.into()));
    let im = image_node(&mut doc, marked.clone(), (17, 1), Affine::translate((0.0, 20.0)), None);
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    // The same image as an opacity mask's art, on `%_` lines.
    let art = image_node(&mut doc, marked, (17, 1), Affine::translate((0.0, 40.0)), None);
    let mut square =
        Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 40.0, 17.0, 41.0)), Appearance::basic(solid(Color::BLACK), Paint::None, 0.0));
    square.extras_mut().mask = Some(Box::new(OpacityMask::new(Node::group(doc.alloc_id(), vec![Arc::new(art)]), false)));
    doc.insert(Some(layer), usize::MAX, square).unwrap();
    let back = read_back(&doc);
    assert_eq!(images_of(&back), images_of(&doc));
}

/// A preview scaled down mixes no color from pixels that don't show.
#[test]
fn a_smaller_preview_takes_no_color_from_see_through_pixels() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let half = image::RgbaImage::from_fn(511, 3, |x, _| if x % 2 == 0 { image::Rgba([255, 0, 0, 0]) } else { image::Rgba([0, 0, 255, 255]) });
    let im = image_node(
        &mut doc,
        ImageBlob::png(png_of(half.into())),
        (511, 3),
        Affine::scale(0.2),
        Some(vectorcraft_doc::LinkInfo::new("/tmp/half.png")),
    );
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    let back = read_back(&doc);
    let NodeKind::Image(im) = &back.layers[0].children().unwrap()[0].kind else { panic!("not an image") };
    assert_eq!(im.width, 256);
    let px = image::load_from_memory(&back.image_blob(im).unwrap().bytes).unwrap().to_rgba8();
    assert!(px.pixels().all(|p| p.0[0] == 0), "{:?}", px.pixels().take(4).collect::<Vec<_>>());
    assert!(px.pixels().filter(|p| p.0[3] > 0).count() > 400);
}

#[test]
fn linked_images_in_masks_and_tiles_are_embedded() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let link = Some(vectorcraft_doc::LinkInfo::new("/tmp/ramp.png"));
    let art = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::scale(10.0), link);
    let mut square =
        Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 30.0, 20.0)), Appearance::basic(solid(Color::BLACK), Paint::None, 0.0));
    square.extras_mut().mask = Some(Box::new(OpacityMask::new(art, false)));
    doc.insert(Some(layer), usize::MAX, square).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::LINKS_EMBEDDED]);
    let text = program_shown(&doc);
    assert!(!text.contains("BeginPlace") && text.contains("%_%AI5_BeginRaster"), "{text}");
}

/// An image in an opacity mask's art inside a pattern's tile is hidden once: its samples end
/// `%_%%EndData`.
#[test]
fn an_image_in_a_mask_in_a_pattern_tile_ends_its_samples_once() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let art = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::scale(10.0), None);
    let mut square =
        Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 30.0, 20.0)), Appearance::basic(solid(Color::BLACK), Paint::None, 0.0));
    square.extras_mut().mask = Some(Box::new(OpacityMask::new(Node::group(doc.alloc_id(), vec![Arc::new(art)]), false)));
    doc.patterns.push(vectorcraft_doc::PatternDef::new("Masked", vec![Arc::new(square)]));
    let filled = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(40.0, 40.0, 100.0, 80.0)),
        Appearance::basic(Paint::Pattern { pattern: "Masked".into(), xf: Affine::IDENTITY }, Paint::None, 0.0),
    );
    doc.insert(Some(layer), usize::MAX, filled).unwrap();
    let text = program_shown(&doc);
    assert!(text.contains("XI\n<18 bytes>%_%%EndData\n%_XH\n"), "{text}");
    assert!(!text.contains("%_%_%%EndData"), "{text}");
}

#[test]
fn an_image_in_art_cut_to_a_clipping_object_is_clipped_by_that_region() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    // Type clips: its art is cut to the glyphs' region, which can't cut an image.
    let ring = Node::new(
        doc.alloc_id(),
        NodeKind::Compound {
            children: vec![
                Arc::new(Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 40.0)), Appearance::default())),
                Arc::new(Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), Appearance::default())),
            ],
            rule: FillRule::EvenOdd,
        },
    );
    let ring = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(ring)], clip: false });
    let im = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::scale(20.0), None);
    let group = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(ring), Arc::new(im)], clip: true });
    doc.insert(Some(layer), usize::MAX, group).unwrap();
    let back = read_back(&doc);
    let mut found = vec![];
    pictures(&back, &back.layers, "", &mut found);
    assert_eq!(found.len(), 1, "{:?}", lines(&back));
    let text = program_shown(&doc);
    let at = text.find("%AI5_EndRaster\nN\n").unwrap();
    assert!(text[at..].contains("h\nW\nn\n") && text[..at].contains("q\n"), "{text}");
    // The ring's two subpaths clip as one compound path, not as one cut by the other.
    let tail = &text[at..];
    assert!(tail.contains("0 XR\n*u\n") && tail.contains("h\nW\nn\n*U\n"), "{text}");
    fn clipping(nodes: &[Arc<Node>], out: &mut Vec<usize>) {
        for n in nodes {
            if let NodeKind::Group { children, clip: true } = &n.kind {
                out.extend(children.first().map(|c| match &c.kind {
                    NodeKind::Path { path, .. } => path.subpaths.len(),
                    NodeKind::Compound { children, .. } => children.len(),
                    _ => 0,
                }));
            }
            clipping(n.children().map(Vec::as_slice).unwrap_or_default(), out);
        }
    }
    let mut subpaths = vec![];
    clipping(&back.layers, &mut subpaths);
    assert_eq!(subpaths, [2], "{:?}", lines(&back));
}

/// Art cut to a region with nothing left in it (two clipping objects that don't overlap) leaves the
/// image out.
#[test]
fn an_image_cut_to_nothing_is_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let boxed = |doc: &mut Document, r: Rect| {
        let p = Node::path(doc.alloc_id(), shapes::rectangle(r), Appearance::default());
        Arc::new(Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(p)], clip: false }))
    };
    let im = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::scale(20.0), None);
    let far = boxed(&mut doc, Rect::new(50.0, 0.0, 60.0, 10.0));
    let inner = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![far, Arc::new(im)], clip: true });
    let near = boxed(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0));
    let outer = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![near, Arc::new(inner)], clip: true });
    doc.insert(Some(layer), usize::MAX, outer).unwrap();
    assert!(!program_shown(&doc).contains("%AI5_BeginRaster"));
}

#[test]
fn images_that_cant_be_decoded_are_mid_gray_boxes() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let im = image_node(&mut doc, ImageBlob::png(b"not an image".to_vec()), (2, 1), Affine::scale(10.0), None);
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::IMAGES]);
    let back = read_back(&doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert_eq!(paint(n.appearance.items[0].paint()), paint(&solid(Color::gray(0.5))));
}

#[test]
fn sixteen_bit_images_are_written_at_eight_bits() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let deep = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(2, 1, |x, _| image::Rgb([65535 * x as u16, 257 * 10, 0]));
    let im = image_node(&mut doc, ImageBlob::png(png_of(image::DynamicImage::ImageRgb16(deep))), (2, 1), Affine::scale(10.0), None);
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::IMAGE_DEPTH]);
    let back = read_back(&doc);
    let mut found = vec![];
    pictures(&back, &back.layers, "", &mut found);
    assert!(found[0].ends_with("rgba [0, 10, 0, 255, 255, 10, 0, 255]"), "{found:?}");
}

/// A little-endian, uncompressed CMYK TIFF of 16 bits a sample, `w` × 1 pixels, each pixel's inks
/// `inks[x]`.
fn cmyk16_tiff(inks: &[[u16; 4]]) -> Vec<u8> {
    let w = inks.len() as u32;
    let data: Vec<u8> = inks.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    // Tags: width, height, bits (an offset), compression, photometric, strip offset, samples, rows,
    // strip bytes, planar configuration.
    let bits_at = 8 + 2 + 12 * 10 + 4;
    let data_at = bits_at + 8;
    let entries: [(u16, u16, u32, u32); 10] = [
        (256, 4, 1, w),
        (257, 4, 1, 1),
        (258, 3, 4, bits_at),
        (259, 3, 1, 1),
        (262, 3, 1, 5),
        (273, 4, 1, data_at),
        (277, 3, 1, 4),
        (278, 4, 1, 1),
        (279, 4, 1, data.len() as u32),
        (284, 3, 1, 1),
    ];
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&10u16.to_le_bytes());
    for (tag, kind, count, value) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&16u16.to_le_bytes());
    }
    out.extend_from_slice(&data);
    out
}

#[test]
fn sixteen_bit_cmyk_images_are_written_at_eight_bits() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let blob = ImageBlob::new("image/tiff", cmyk16_tiff(&[[65535, 0, 0, 0], [0, 0x8000, 0, 0x1234]]));
    assert!(blob.is_cmyk() && blob.is_cmyk16());
    let im = image_node(&mut doc, blob, (2, 1), Affine::scale(10.0), None);
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::IMAGE_DEPTH]);
    assert!(program_shown(&doc).contains("/DeviceCMYK XN\n"));
}

// ---------- type ----------

fn style(size: f64, fill: Color) -> vectorcraft_doc::CharStyle {
    vectorcraft_doc::CharStyle { size, fill: solid(fill), ..vectorcraft_doc::CharStyle::default() }
}

fn text_node(doc: &mut Document, t: vectorcraft_doc::TextObject) -> Node {
    Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)))
}

/// A 300 × 200 document of the type the slice writes as text, on a layer `Type`: two paragraphs
/// of point type (the second centered, with space before, a fixed leading) named `Greeting`; point
/// type turned 30° in CMYK; area type in two columns with a gutter and a first baseline at its
/// leading, its runs plain, then bold, tracked, shifted, scaled, stroked and in capitals, then a
/// right-to-left paragraph; type on an open curve from a tenth of its length; vertical point type;
/// hidden, locked point type at half opacity multiplied; and area type threaded through two frames.
fn typed() -> Document {
    use vectorcraft_doc::{CharStyle, FirstBaseline, Justify, ParaDirection, ParaStyle, TextKind, TextObject, TextRun};
    use vectorcraft_geom::Point;
    let mut doc = Document::new(300.0, 200.0);
    // One CMYK swatch, not the default ones: their CMYK equivalents round differently once read back.
    doc.swatches = vec![
        Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false },
        Swatch { name: "Ink".into(), paint: solid(BRAND), global: false, spot: false },
    ]
    .into();
    doc.swatch_groups = vec![].into();
    doc.layers.clear();
    let layer = doc.add_layer(Some("Type"));

    let mut greet = TextObject::point(Point::new(20.0, 40.0), "Hello\nWorld", CharStyle { leading: Some(30.0), ..style(24.0, SKY) });
    let centered = ParaStyle { justify: Justify::Center, space_before: 6.0, ..ParaStyle::default() };
    greet.set_paragraph_styles(vec![ParaStyle { first_line_indent: 10.0, ..ParaStyle::default() }, centered]);
    let greet = named(text_node(&mut doc, greet), "Greeting");
    doc.insert(Some(layer), usize::MAX, greet).unwrap();

    let mut turned = TextObject::point(Point::ZERO, "Turned", style(18.0, BRAND));
    turned.xf = Affine::translate((150.0, 100.0)) * Affine::rotate(-std::f64::consts::FRAC_PI_6);
    let turned = text_node(&mut doc, turned);
    doc.insert(Some(layer), usize::MAX, turned).unwrap();

    let regular = style(10.0, Color::BLACK);
    let bold = CharStyle {
        font_style: "Bold".into(),
        tracking: 50.0,
        baseline_shift: 3.0,
        h_scale: 120.0,
        v_scale: 90.0,
        stroke: solid(Color::rgb(0.0, 0.5, 0.0)),
        stroke_width: 0.5,
        all_caps: true,
        ..regular.clone()
    };
    let mut area = TextObject::point(Point::ZERO, "", regular.clone());
    area.runs = vec![
        TextRun::new("Area type flows in two columns ", regular.clone()),
        TextRun::new("and wraps", bold),
        TextRun::new(" onto the next line.\nRight to left", regular.clone()),
    ];
    area.set_paragraph_styles(vec![
        ParaStyle { left_indent: 4.0, right_indent: 2.0, space_after: 3.0, hyphenate: false, ..ParaStyle::default() },
        ParaStyle { direction: Some(ParaDirection::RightToLeft), justify: Justify::JustifyAll, ..ParaStyle::default() },
    ]);
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(20.0, 80.0, 140.0, 160.0)) };
    area.xf = Affine::IDENTITY;
    area.area.columns = 2;
    area.area.gutter = 12.0;
    area.area.first_baseline = FirstBaseline::Leading;
    area.area.first_baseline_min = 4.0;
    let area = text_node(&mut doc, area);
    doc.insert(Some(layer), usize::MAX, area).unwrap();

    let mut on_path = TextObject::point(Point::ZERO, "On a curve", style(12.0, Color::gray(0.6)));
    let mut curve = PathData::from_bezpath(&kurbo::BezPath::from_vec(vec![
        kurbo::PathEl::MoveTo((160.0, 150.0).into()),
        kurbo::PathEl::CurveTo((190.0, 110.0).into(), (250.0, 110.0).into(), (280.0, 150.0).into()),
    ]));
    curve.subpaths.iter_mut().for_each(|sp| sp.closed = false);
    on_path.kind = TextKind::OnPath { path: curve, start: 0.1, end: None };
    on_path.xf = Affine::IDENTITY;
    let on_path = text_node(&mut doc, on_path);
    doc.insert(Some(layer), usize::MAX, on_path).unwrap();

    let mut down = TextObject::point(Point::new(280.0, 20.0), "Down", style(14.0, Color::BLACK));
    down.vertical = true;
    let down = text_node(&mut doc, down);
    doc.insert(Some(layer), usize::MAX, down).unwrap();

    let quiet = TextObject::point(Point::new(20.0, 190.0), "Quiet", style(9.0, Color::BLACK));
    let mut quiet = see_through(text_node(&mut doc, quiet), 0.5, BlendMode::Multiply);
    quiet.visible = false;
    quiet.locked = true;
    doc.insert(Some(layer), usize::MAX, quiet).unwrap();

    let mut first = TextObject::point(Point::ZERO, "A story that runs from one frame into the next one.", regular.clone());
    first.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(160.0, 20.0, 220.0, 40.0)) };
    first.xf = Affine::IDENTITY;
    let mut second = first.clone();
    second.runs = vec![TextRun::new("", regular)];
    second.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(160.0, 50.0, 220.0, 90.0)) };
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, second).unwrap();
    doc
}

/// What the reader's view of type is made of, one line per text object: its name, state,
/// transparency, kind and frame, matrix, orientation, area options and path span; each paragraph's
/// attributes; each run's characters and attributes. Lengths to three decimals.
fn type_lines(doc: &Document) -> Vec<String> {
    use vectorcraft_doc::TextKind;
    let mut out = vec![];
    let mut all = vec![];
    paths(&doc.layers, &mut all);
    for n in all {
        let NodeKind::Text(t) = &n.kind else {
            out.push(format!("not type: {:?}", n.name));
            continue;
        };
        let pts = |p: &PathData| {
            p.subpaths
                .iter()
                .map(|sp| {
                    format!("[{} {}]", sp.closed, sp.anchors.iter().map(|a| format!("{},{}", r3(a.p.x), r3(a.p.y))).collect::<Vec<_>>().join(" "))
                })
                .collect::<String>()
        };
        let kind = match &t.kind {
            TextKind::Point => "point".to_string(),
            TextKind::Area { frame } => format!("area {}", pts(frame)),
            TextKind::OnPath { path, start, end } => format!("path {} {} {:?}", pts(path), r3(*start), end.map(r3)),
        };
        let a = &t.area;
        out.push(format!(
            "{:?} shown={} locked={} {} {:?} {kind} xf={:?} vertical={} area {} {} {} {:?} {:?} {}",
            n.name,
            n.visible,
            n.locked,
            r3(f64::from(n.opacity)),
            n.blend,
            t.xf.as_coeffs().map(r3),
            t.vertical,
            a.rows,
            a.columns,
            r3(a.gutter),
            a.row_gutter.map(r3),
            a.first_baseline,
            r3(a.first_baseline_min),
        ));
        for p in t.paragraph_styles() {
            out.push(format!(
                "  para {:?} {} {} {} {} {} {} {:?}",
                p.justify,
                r3(p.first_line_indent),
                r3(p.left_indent),
                r3(p.right_indent),
                r3(p.space_before),
                r3(p.space_after),
                p.hyphenate,
                p.direction
            ));
        }
        for r in &t.runs {
            let s = &r.style;
            out.push(format!(
                "  run {:?} {} {} {} {:?} {} {} {} {} fill {} stroke {} {} caps={} small={:?} tcy={}",
                r.text,
                s.font_family,
                s.font_style,
                r3(s.size),
                s.leading.map(r3),
                r3(s.tracking),
                r3(s.baseline_shift),
                r3(s.h_scale),
                r3(s.v_scale),
                paint(&s.fill),
                paint(&s.stroke),
                r3(s.stroke_width),
                s.all_caps,
                s.small_caps.map(r3),
                s.tate_chu_yoko,
            ));
        }
    }
    out
}

#[test]
fn type_comes_back_as_type() {
    let doc = typed();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_lines(&type_lines(&back), &type_lines(&doc));
    assert_eq!(back.text_threads.len(), 1, "the thread comes back");
}

#[test]
fn type_is_written_again_as_the_same_bytes() {
    let doc = typed();
    let first = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(first.program, export_ai_editing(&doc, &opts()).unwrap().program);
    let again = export_ai_editing(&read_back(&doc), &opts()).unwrap();
    let text = |p: &[u8]| text_document(&String::from_utf8_lossy(p).replace("\r\n", "\n"));
    assert_eq!(text(&again.program), text(&first.program));
    assert_eq!(String::from_utf8_lossy(&again.program), String::from_utf8_lossy(&first.program));
}

/// The text document of program `text` (LF line ends), decoded, with its UTF-16 strings' zero
/// bytes left out.
fn text_document(text: &str) -> String {
    let Some(start) = text.find("%AI11_BeginTextDocument\n") else { return String::new() };
    let end = text.find("%AI11_EndTextDocument").unwrap();
    let packed: String = text[start..end].lines().skip(2).filter(|l| *l != ";").map(|l| l.strip_prefix('%').unwrap_or(l)).collect();
    String::from_utf8_lossy(&crate::ps::ascii85_decode(&packed).unwrap()).replace('\0', "")
}

/// The text document: its section after the document data, ASCII85 on comment lines none of which
/// is a `%_` line; a stub for each text object, area type and type on a path with their paths.
#[test]
fn type_is_a_text_document_and_stubs() {
    let text = program(&typed());
    let doc_data = text.find("%AI9_EndDocumentData").unwrap();
    let start = text.find("%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n").unwrap();
    let end = text.find(";\n%AI11_EndTextDocument\n%%EndSetup\n").unwrap();
    assert!(doc_data < start && start < end);
    let body = &text[start..end];
    for l in body.lines().skip(2) {
        assert!(l.starts_with('%') && !l.starts_with("%_"), "{l}");
    }
    assert_eq!(text.matches("/AI11Text :\n0 /FreeUndo ,\n").count(), 8);
    assert_eq!(text.matches("; /ConfiningPath ,\n").count(), 4);
    assert!(text.contains("0 /FrameIndex ,\n6 /StoryIndex ,") && text.contains("1 /FrameIndex ,\n6 /StoryIndex ,"), "{text}");
    // The decoded document: the version, a font of each face and the hidden characters' font, the
    // stories, the normal sheets and the document's text settings.
    let decoded = text_document(&text);
    assert!(decoded.starts_with("/98 << /0 14 >> /0 <<"), "{decoded}");
    for want in
        ["SourceSans3-Regular", "SourceSans3-Bold", "AdobeInvisFont", "[Normal Character Style]", "[Normal Paragraph Style]", "Hello\rWorld\r"]
    {
        assert!(decoded.contains(want), "{want}: {decoded}");
    }
    // Tracking is a whole number, and colors are simple paints.
    assert!(decoded.contains("/8 50 ") && !decoded.contains("/8 50."), "{decoded}");
    assert!(decoded.contains("/53 << /99 /SimplePaint") && !decoded.contains("CAITextPaint"), "{decoded}");
}

/// Point type scaled 150% comes back at its size: its scale is in its sizes, its matrix only turns
/// it. Type that ends in an empty paragraph keeps it.
#[test]
fn scaled_point_type_and_a_last_empty_paragraph_come_back() {
    use vectorcraft_doc::{CharStyle, ParaStyle, TextObject};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut big = TextObject::point(vectorcraft_geom::Point::ZERO, "Big", CharStyle { leading: Some(20.0), ..style(12.0, Color::BLACK) });
    big.xf = Affine::translate((20.0, 40.0)) * Affine::rotate(0.3) * Affine::scale(1.5);
    big.set_all_paras(ParaStyle { left_indent: 4.0, ..ParaStyle::default() });
    let ends = TextObject::point(vectorcraft_geom::Point::new(20.0, 80.0), "Ends\n", style(12.0, Color::BLACK));
    let (big, ends) = (text_node(&mut doc, big), text_node(&mut doc, ends));
    doc.insert(Some(layer), usize::MAX, big).unwrap();
    doc.insert(Some(layer), usize::MAX, ends).unwrap();
    let back = read_back(&doc);
    let texts: Vec<&vectorcraft_doc::TextObject> =
        back.layers[0].children().unwrap().iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.as_ref()) } else { None }).collect();
    let [big, ends] = texts.as_slice() else { panic!("two text objects: {texts:?}") };
    let s = big.xf.determinant().sqrt();
    let st = &big.runs[0].style;
    assert_eq!((r3(s), r3(st.size), st.leading.map(r3), r3(big.para.left_indent)), (r3(1.0), r3(18.0), Some(r3(30.0)), r3(6.0)));
    assert_eq!(big.xf.as_coeffs().map(r3), (Affine::translate((20.0, 40.0)) * Affine::rotate(0.3)).as_coeffs().map(r3));
    assert_eq!(ends.plain_text(), "Ends\n");
    // A tiny size, and a stroke of no width that keeps its paint.
    let mut d = Document::new(200.0, 100.0);
    let layer = d.layers[0].id;
    let st = CharStyle { stroke: solid(Color::rgb(1.0, 0.0, 0.0)), stroke_width: 0.0, ..style(0.00001, Color::BLACK) };
    let n = text_node(&mut d, TextObject::point(vectorcraft_geom::Point::new(20.0, 40.0), "Tiny", st));
    d.insert(Some(layer), usize::MAX, n).unwrap();
    let back = read_back(&d);
    let NodeKind::Text(t) = &back.layers[0].children().unwrap()[0].kind else { panic!("type") };
    let s = &t.runs[0].style;
    assert_eq!(
        (format!("{:.6}", s.size), paint(&s.stroke), r3(s.stroke_width)),
        ("0.000010".into(), paint(&solid(Color::rgb(1.0, 0.0, 0.0))), r3(0.0))
    );
}

/// A threaded story whose frames each hold their slice of it, a paragraph split between them,
/// comes back as one story in its first frame with each paragraph's attributes once.
#[test]
fn a_story_split_between_frames_comes_back_whole() {
    use vectorcraft_doc::{Justify, ParaStyle, TextKind, TextObject, TextRun};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let st = style(10.0, Color::BLACK);
    let (p1, p2, p3) = (
        ParaStyle { justify: Justify::Center, ..ParaStyle::default() },
        ParaStyle { left_indent: 5.0, ..ParaStyle::default() },
        ParaStyle { space_before: 4.0, ..ParaStyle::default() },
    );
    let mut first = TextObject::point(vectorcraft_geom::Point::ZERO, "", st.clone());
    first.runs = vec![TextRun::new("One two\nthree ", st.clone())];
    first.set_paragraph_styles(vec![p1.clone(), p2.clone()]);
    first.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 40.0)) };
    first.xf = Affine::IDENTITY;
    let p4 = ParaStyle { right_indent: 7.0, ..ParaStyle::default() };
    let mut second = first.clone();
    second.runs = vec![TextRun::new("four\nFive\n", st)];
    second.set_paragraph_styles(vec![p2.clone(), p3.clone(), p4.clone()]);
    second.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(110.0, 10.0, 190.0, 40.0)) };
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, second).unwrap();
    let back = read_back(&doc);
    let texts: Vec<&TextObject> =
        back.layers[0].children().unwrap().iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.as_ref()) } else { None }).collect();
    let [a, b] = texts.as_slice() else { panic!("two frames: {texts:?}") };
    assert_eq!((a.plain_text().as_str(), b.plain_text().as_str()), ("One two\nthree four\nFive\n", ""));
    let attrs = |p: &ParaStyle| (p.justify, r3(p.left_indent), r3(p.right_indent), r3(p.space_before));
    let got: Vec<_> = a.paragraph_styles().iter().map(attrs).collect();
    assert_eq!(got, [&p1, &p2, &p3, &p4].into_iter().map(attrs).collect::<Vec<_>>(), "the last, empty paragraph keeps its own");
    assert_eq!(back.text_threads.len(), 1);
    assert!(export_ai_editing(&doc, &opts()).unwrap().warnings.is_empty());
    // An attribute the text document doesn't hold in a later frame is reported.
    let second_id = doc.text_threads[0][1];
    let mut lossy = doc.clone();
    if let NodeKind::Text(t) = &mut lossy.node_mut(second_id).unwrap().kind {
        t.runs[0].style.underline = true;
    }
    assert_eq!(export_ai_editing(&lossy, &opts()).unwrap().warnings, [writer::TYPE_ATTRIBUTES]);
    // So is a paragraph split between the frames whose second part has other attributes.
    let mut split = doc.clone();
    if let NodeKind::Text(t) = &mut split.node_mut(second_id).unwrap().kind {
        t.set_paragraph_styles(vec![ParaStyle { left_indent: 9.0, ..ParaStyle::default() }, p3, p4]);
    }
    assert_eq!(export_ai_editing(&split, &opts()).unwrap().warnings, [writer::TYPE_ATTRIBUTES]);
}

/// A story longer than the reader reads is written as its outlines.
#[test]
fn a_story_longer_than_the_reader_reads_is_written_as_its_outlines() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let long = format!("A{}", " ".repeat(crate::import::MAX_STORY));
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 50.0), &long, style(12.0, Color::BLACK));
    let short = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 90.0), "Short", style(12.0, Color::BLACK));
    let (t, short) = (text_node(&mut doc, t), text_node(&mut doc, short));
    doc.insert(Some(layer), usize::MAX, t).unwrap();
    doc.insert(Some(layer), usize::MAX, short).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::TYPE]);
    assert_eq!(program(&doc).matches("/AI11Text :").count(), 1, "the short story is text");
}

/// Stories that together make a text document larger than the reader reads: the ones past it are
/// written as their outlines, and what is written reads back.
#[test]
fn stories_past_the_text_document_the_reader_reads_are_written_as_their_outlines() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let long = format!("A{}", " ".repeat(900_000));
    for i in 0..9 {
        // A font of its own, which no other story names.
        let st = vectorcraft_doc::CharStyle { font_family: format!("Probe Family {i}"), ..style(8.0, Color::BLACK) };
        let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 10.0 + f64::from(i) * 10.0), &long, st);
        let n = text_node(&mut doc, t);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::TYPE]);
    let text = String::from_utf8_lossy(&out.program).replace("\r\n", "\n");
    let stubs = text.matches("/AI11Text :").count();
    assert!((1..9).contains(&stubs), "{stubs}");
    // The font set holds the fonts of the stories written, and the hidden characters' font.
    assert_eq!(text_document(&text).matches("/CoolTypeFont").count(), stubs + 1);
    let back = read_back(&doc);
    let texts = back.layers[0].children().unwrap().iter().filter(|n| matches!(n.kind, NodeKind::Text(_))).count();
    assert_eq!(texts, stubs, "every story written comes back");
}

/// A thread whose frames differ in orientation is written as its outlines: a story's frames share
/// one.
#[test]
fn a_thread_of_horizontal_and_vertical_frames_is_written_as_its_outlines() {
    use vectorcraft_doc::{TextKind, TextObject};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut first = TextObject::point(vectorcraft_geom::Point::ZERO, "Across, then down", style(10.0, Color::BLACK));
    first.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 40.0)) };
    first.xf = Affine::IDENTITY;
    let mut second = first.clone();
    second.runs[0].text.clear();
    second.vertical = true;
    second.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(110.0, 10.0, 190.0, 90.0)) };
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, second).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::TYPE]);
    assert!(!program(&doc).contains("/AI11Text"));
}

/// Area type in an open frame whose ends meet is written as its outlines, alone or as a frame of a
/// thread: as text it would be read back in a closed frame.
#[test]
fn an_open_frame_whose_ends_meet_is_written_as_its_outlines() {
    use vectorcraft_doc::{TextKind, TextObject};
    let open = |r: Rect| {
        let mut frame = shapes::rectangle(r);
        frame.subpaths[0].closed = false;
        let start = frame.subpaths[0].anchors[0];
        frame.subpaths[0].anchors.push(start);
        frame
    };
    let mut alone = TextObject::point(vectorcraft_geom::Point::ZERO, "Open", style(10.0, Color::BLACK));
    alone.kind = TextKind::Area { frame: open(Rect::new(10.0, 10.0, 90.0, 40.0)) };
    alone.xf = Affine::IDENTITY;
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let n = text_node(&mut doc, alone.clone());
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::TYPE]);
    assert!(!program(&doc).contains("/AI11Text"));

    let mut first = alone;
    first.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 40.0)) };
    let mut second = first.clone();
    second.runs[0].text.clear();
    second.kind = TextKind::Area { frame: open(Rect::new(110.0, 10.0, 190.0, 90.0)) };
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, second).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::TYPE]);
    assert!(!program(&doc).contains("/AI11Text"));
}

/// A thread with a frame in a clipping group whose clipping object is a path stays type, every frame
/// of it, and comes back as one thread: the art of such a group isn't cut.
#[test]
fn a_thread_with_a_frame_in_a_clipping_group_stays_type() {
    use vectorcraft_doc::{TextKind, TextObject};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut first = TextObject::point(vectorcraft_geom::Point::ZERO, "A story in two frames", style(10.0, Color::BLACK));
    first.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 40.0)) };
    first.xf = Affine::IDENTITY;
    let mut second = first.clone();
    second.runs[0].text.clear();
    second.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(110.0, 10.0, 190.0, 40.0)) };
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    let clipper = clipping(&mut doc, shapes::rectangle(Rect::new(100.0, 0.0, 200.0, 50.0)));
    let clipped = group(&mut doc, vec![clipper, second], true);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, clipped).unwrap();
    assert!(export_ai_editing(&doc, &opts()).unwrap().warnings.is_empty());
    assert_eq!(program(&doc).matches("/AI11Text :").count(), 2);
    assert_eq!(read_back(&doc).text_threads.len(), 1);
}

/// A thread with a frame in art cut to a clipping object that isn't a path is written as its
/// outlines, every frame of it; so is type whose size is past what the reader reads.
#[test]
fn a_thread_with_a_frame_in_cut_art_and_type_past_the_readers_sizes_are_outlines() {
    use vectorcraft_doc::{TextKind, TextObject};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut first = TextObject::point(vectorcraft_geom::Point::ZERO, "A story in two frames", style(10.0, Color::BLACK));
    first.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 40.0)) };
    first.xf = Affine::IDENTITY;
    let mut second = first.clone();
    second.runs[0].text.clear();
    second.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(110.0, 10.0, 190.0, 40.0)) };
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    // The second frame is clipped by type: its art is cut to the type's outlines.
    let clipper = text_node(&mut doc, TextObject::point(vectorcraft_geom::Point::new(110.0, 30.0), "CUT", style(30.0, Color::BLACK)));
    let cut = group(&mut doc, vec![clipper, second], true);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, cut).unwrap();
    let huge = TextObject::point(vectorcraft_geom::Point::new(10.0, 90.0), "Huge", style(12.0, Color::BLACK));
    let huge = TextObject { xf: Affine::translate((10.0, 90.0)) * Affine::scale(1e5), ..huge };
    let huge = text_node(&mut doc, huge);
    doc.insert(Some(layer), usize::MAX, huge).unwrap();
    let text = program(&doc);
    assert!(!text.contains("/AI11Text"), "{text}");
    // Tracking that rounds to the reader's limit, 100,000, is written as outlines.
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut wide = TextObject::point(vectorcraft_geom::Point::new(10.0, 50.0), "Wide", style(12.0, Color::BLACK));
    wide.runs[0].style.tracking = 99_999.6;
    let wide = text_node(&mut doc, wide);
    doc.insert(Some(layer), usize::MAX, wide).unwrap();
    let text = program(&doc);
    assert!(!text.contains("/AI11Text"), "{text}");
    // A frame that is the clipping object of a clipping group is written as the clipping outline.
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut first = TextObject::point(vectorcraft_geom::Point::ZERO, "A story in two frames", style(10.0, Color::BLACK));
    first.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 40.0)) };
    first.xf = Affine::IDENTITY;
    let mut second = first.clone();
    second.runs[0].text.clear();
    second.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(110.0, 10.0, 190.0, 40.0)) };
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    let backing = square(&mut doc, Rect::new(100.0, 0.0, 200.0, 50.0), Color::rgb(1.0, 0.0, 0.0));
    let clipped = group(&mut doc, vec![second, backing], true);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, clipped).unwrap();
    let text = program(&doc);
    assert!(!text.contains("/AI11Text"), "{text}");
}

#[test]
fn type_the_data_cant_hold_is_written_as_its_outlines() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let g = Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::rgb(1.0, 0.0, 0.0))]);
    let fill = Paint::Gradient(Box::new(GradientPaint::new(g)));
    let t = vectorcraft_doc::TextObject::point(
        vectorcraft_geom::Point::new(10.0, 70.0),
        "Shaded",
        vectorcraft_doc::CharStyle { size: 18.0, fill, ..vectorcraft_doc::CharStyle::default() },
    );
    let mirrored = vectorcraft_doc::TextObject {
        xf: Affine::new([-1.0, 0.0, 0.0, 1.0, 150.0, 70.0]),
        ..vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "Mirror", style(18.0, Color::BLACK))
    };
    // Area type in a frame of two subpaths: a frame of the text document is one path.
    let mut holed = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "Two rings", style(10.0, Color::BLACK));
    let mut rings = shapes::rectangle(Rect::new(100.0, 10.0, 190.0, 90.0));
    rings.subpaths.extend(shapes::rectangle(Rect::new(120.0, 30.0, 170.0, 70.0)).subpaths);
    holed.kind = vectorcraft_doc::TextKind::Area { frame: rings };
    holed.xf = Affine::IDENTITY;
    // A carriage return in the characters: a story ends its paragraphs with it. Area type that
    // flows around a wrap object: the wrap object's settings aren't written.
    let returned = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 90.0), "A\rB", style(10.0, Color::BLACK));
    let mut wrapped = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "Wrapped round", style(10.0, Color::BLACK));
    wrapped.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 60.0)) };
    wrapped.xf = Affine::IDENTITY;
    wrapped.wrap = vec![vectorcraft_doc::WrapShape { path: shapes::rectangle(Rect::new(30.0, 20.0, 50.0, 40.0)), wrap: Default::default() }];
    let wrapped = text_node(&mut doc, wrapped);
    doc.insert(Some(layer), usize::MAX, wrapped).unwrap();
    // Type on an open path whose ends meet: it would be read back on a closed path.
    let mut ring = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "Round", style(10.0, Color::BLACK));
    let mut open_ring = shapes::ellipse(Rect::new(120.0, 10.0, 180.0, 70.0));
    if let Some(first) = open_ring.subpaths[0].anchors.first().copied() {
        open_ring.subpaths[0].anchors.push(first);
    }
    open_ring.subpaths[0].closed = false;
    ring.kind = vectorcraft_doc::TextKind::OnPath { path: open_ring, start: 0.0, end: None };
    ring.xf = Affine::IDENTITY;
    let ring = text_node(&mut doc, ring);
    doc.insert(Some(layer), usize::MAX, ring).unwrap();
    let (t, mirrored, holed, returned) =
        (text_node(&mut doc, t), text_node(&mut doc, mirrored), text_node(&mut doc, holed), text_node(&mut doc, returned));
    doc.insert(Some(layer), usize::MAX, t).unwrap();
    doc.insert(Some(layer), usize::MAX, mirrored).unwrap();
    doc.insert(Some(layer), usize::MAX, holed).unwrap();
    doc.insert(Some(layer), usize::MAX, returned).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::TYPE]);
    assert!(!program(&doc).contains("%AI11_BeginTextDocument"));
    let back = read_back(&doc);
    let kinds: Vec<bool> = back.layers[0].children().unwrap().iter().map(|n| matches!(n.kind, NodeKind::Group { .. })).collect();
    assert_eq!(kinds, [true; 6]);
}

#[test]
fn type_attributes_the_data_cant_hold_are_reported() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let st = vectorcraft_doc::CharStyle { underline: true, ..style(18.0, Color::BLACK) };
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 70.0), "Under", st);
    let t = text_node(&mut doc, t);
    doc.insert(Some(layer), usize::MAX, t).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::TYPE_ATTRIBUTES]);
    let back = read_back(&doc);
    assert!(matches!(back.layers[0].children().unwrap()[0].kind, NodeKind::Text(_)));
    // Any other attribute the text document doesn't hold is reported too: No Break, a paragraph's
    // left-to-right direction set on it, Optical Margin Alignment, a font version, a font that isn't
    // installed whose name doesn't come back (a hyphen in its family), a Lab color,
    // small capitals at another size, right-to-left letters aligned to the start of a paragraph
    // without a direction of its own, tracking that isn't a whole number.
    for edit in [
        (|t: &mut vectorcraft_doc::TextObject| t.runs[0].style.no_break = true) as fn(&mut vectorcraft_doc::TextObject),
        |t| t.runs[0].style.tracking = 12.5,
        |t| t.para.direction = Some(vectorcraft_doc::ParaDirection::LeftToRight),
        |t| t.optical_margin = true,
        |t| t.runs[0].style.font_version = Some("2.0".into()),
        |t| t.runs[0].style.font_family = "my-missing font".into(),
        |t| t.runs[0].style.fill = Paint::solid(MINT),
        |t| t.runs[0].style.small_caps = Some(80.0),
        |t| {
            t.runs[0].text = "\u{645}\u{631}\u{62d}\u{628}\u{627}".into();
            t.para.justify = vectorcraft_doc::Justify::Auto;
        },
    ] {
        let with = |t: vectorcraft_doc::TextObject| {
            let mut d = Document::new(200.0, 100.0);
            let layer = d.layers[0].id;
            let n = text_node(&mut d, t);
            d.insert(Some(layer), usize::MAX, n).unwrap();
            export_ai_editing(&d, &opts()).unwrap().warnings
        };
        let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 70.0), "Kept", style(18.0, Color::BLACK));
        assert!(with(t.clone()).is_empty());
        // A font that isn't installed keeps its family and style as its name.
        let mut missing = t.clone();
        missing.runs[0].style.font_family = "Missing Sans Probe".into();
        missing.runs[0].style.font_style = "Bold Italic".into();
        assert!(with(missing).is_empty());
        edit(&mut t);
        assert_eq!(with(t), [writer::TYPE_ATTRIBUTES]);
    }
}

/// On a large canvas, area type, threaded frames and type on a path stay type: the frames and
/// lengths are written at the canvas's scale. Frames of a thread at two scales, and area type
/// scaled by two factors, are written as outlines.
#[test]
fn scaled_area_type_and_type_on_a_path_stay_type() {
    use vectorcraft_doc::{TextKind, TextObject};
    let mut doc = Document::new(20_000.0, 10_000.0);
    let layer = doc.layers[0].id;
    let framed = |text: &str, r: Rect| {
        let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, text, style(100.0, Color::BLACK));
        t.kind = TextKind::Area { frame: shapes::rectangle(r) };
        t.xf = Affine::IDENTITY;
        t
    };
    let mut area = framed("Area", Rect::new(1000.0, 1000.0, 5000.0, 3000.0));
    area.area.columns = 2;
    area.area.gutter = 200.0;
    let mut on_path = TextObject::point(vectorcraft_geom::Point::ZERO, "Along", style(100.0, Color::BLACK));
    let mut curve = PathData::from_bezpath(&kurbo::BezPath::from_vec(vec![
        kurbo::PathEl::MoveTo((1000.0, 5000.0).into()),
        kurbo::PathEl::CurveTo((3000.0, 4000.0).into(), (7000.0, 4000.0).into(), (9000.0, 5000.0).into()),
    ]));
    curve.subpaths.iter_mut().for_each(|sp| sp.closed = false);
    on_path.kind = TextKind::OnPath { path: curve, start: 0.0, end: None };
    on_path.xf = Affine::IDENTITY;
    let first = text_node(&mut doc, framed("A story in two frames", Rect::new(1000.0, 6000.0, 5000.0, 8000.0)));
    let second = text_node(&mut doc, framed("", Rect::new(6000.0, 6000.0, 10_000.0, 8000.0)));
    doc.text_threads.push(vec![first.id, second.id]);
    let (area, on_path) = (text_node(&mut doc, area), text_node(&mut doc, on_path));
    for n in [area, on_path, first, second] {
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    let art = back.layers[0].children().unwrap();
    let texts: Vec<&TextObject> = art.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.as_ref()) } else { None }).collect();
    assert_eq!(texts.len(), 4);
    assert_eq!(back.text_threads.len(), 1);
    let TextKind::Area { frame } = &texts[0].kind else { panic!("area type") };
    let b = frame.transformed(texts[0].xf).bounds().unwrap();
    assert_eq!([b.x0, b.y0, b.x1, b.y1].map(r3), ["1000.000", "1000.000", "5000.000", "3000.000"].map(String::from));
    // The reader scales the document back: its matrix holds the canvas's scale, and its lengths
    // through that matrix are the ones written.
    let k = |t: &TextObject| t.xf.as_coeffs()[0];
    assert_eq!(
        (r3(texts[0].runs[0].style.size * k(texts[0])), texts[0].area.columns, r3(texts[0].area.gutter * k(texts[0]))),
        ("100.000".to_string(), 2, "200.000".to_string())
    );
    assert!(matches!(texts[1].kind, TextKind::OnPath { .. }));
    assert_eq!(r3(texts[1].runs[0].style.size * k(texts[1])), "100.000");

    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut uneven = framed("Uneven", Rect::new(10.0, 10.0, 50.0, 30.0));
    uneven.xf = Affine::new([2.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let uneven = text_node(&mut doc, uneven);
    doc.insert(Some(layer), usize::MAX, uneven).unwrap();
    assert!(!program(&doc).contains("/AI11Text"));

    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let first = framed("A story in two frames", Rect::new(10.0, 10.0, 50.0, 30.0));
    let mut second = framed("", Rect::new(30.0, 20.0, 50.0, 30.0));
    second.xf = Affine::scale(2.0);
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, second).unwrap();
    assert!(!program(&doc).contains("/AI11Text"));
}

/// See-through type with the knockout shape flag comes back with the flag and writes no warning.
#[test]
fn the_knockout_shape_of_type_comes_back() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut text = text_node(&mut doc, vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 40.0), "Hi", style(10.0, Color::BLACK)));
    (text.opacity, text.knockout_shape) = (0.5, true);
    doc.insert(Some(layer), usize::MAX, text).unwrap();
    assert!(export_ai_editing(&doc, &opts()).unwrap().warnings.is_empty());
    let back = read_back(&doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert!(matches!(n.kind, NodeKind::Text(_)));
    assert_eq!((n.opacity, n.knockout_shape), (0.5, true));
}

/// The kinds of object that write no paint before their `Xy` or no state block after them, and a
/// group, which writes a state block.
#[derive(Clone, Copy, Debug)]
enum Beside {
    Type,
    Compound,
    Image,
    Guide,
    Group,
}

/// An object of `kind` at `x`, with opacity, blending mode and knockout shape flag `state`.
fn beside(doc: &mut Document, kind: Beside, x: f64, state: (f32, BlendMode, bool)) -> Node {
    let r = Rect::new(x, 10.0, x + 20.0, 30.0);
    let mut n = match kind {
        Beside::Type => {
            let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(x, 40.0), "Hi", style(10.0, Color::BLACK));
            text_node(doc, t)
        }
        Beside::Compound => {
            let outer = Node::path(doc.alloc_id(), shapes::rectangle(r), Appearance::default());
            let inner = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(x + 5.0, 15.0, x + 15.0, 25.0)), Appearance::default());
            let mut ring =
                Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(inner)], rule: FillRule::EvenOdd });
            ring.appearance = Appearance::basic(solid(BRAND), Paint::None, 0.0).into();
            ring
        }
        Beside::Image => {
            let mut png = vec![];
            let img = image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255]).unwrap();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            let blob = ImageBlob::new("image/png", png);
            let key = blob.content_key();
            doc.images.insert(key.clone(), blob);
            let xf = Affine::new([10.0, 0.0, 0.0, 10.0, x, 10.0]);
            let im = ImageObject { key, width: 1, height: 1, xf, link: None, placement: Default::default(), pixels: None };
            Node::new(doc.alloc_id(), NodeKind::Image(Box::new(im)))
        }
        Beside::Guide => Node::new(
            doc.alloc_id(),
            NodeKind::Path { path: shapes::rectangle(r), rule: FillRule::NonZero, live: None, clipping: false, guide: true },
        ),
        Beside::Group => {
            let member = square(doc, r, BRAND);
            group(doc, vec![member], false)
        }
    };
    (n.opacity, n.blend, n.knockout_shape) = state;
    n
}

/// Two objects next to each other come back with their own opacity, blending mode and knockout
/// shape flag, whichever is first and whichever is opaque: an `Xy` right after a text object or a
/// compound path is theirs when no other operator comes between. The layer's stays its own.
#[test]
fn objects_next_to_each_other_keep_their_own_transparency() {
    use Beside::{Compound, Group, Guide, Image, Type};
    let (clear, screen, plain) = ((0.5, BlendMode::Multiply, true), (0.25, BlendMode::Screen, false), (1.0, BlendMode::Normal, false));
    let pairs = [
        (Type, Type),
        (Compound, Type),
        (Group, Type),
        (Type, Group),
        (Type, Compound),
        (Type, Image),
        (Type, Guide),
        (Compound, Image),
        (Compound, Guide),
    ];
    for (a, b) in pairs {
        for (first, second) in [(clear, plain), (plain, clear), (clear, screen), (clear, clear)] {
            let mut doc = Document::new(200.0, 100.0);
            let layer = doc.layers[0].id;
            let (x, y) = (beside(&mut doc, a, 10.0, first), beside(&mut doc, b, 60.0, second));
            doc.insert(Some(layer), usize::MAX, x).unwrap();
            doc.insert(Some(layer), usize::MAX, y).unwrap();
            let back = read_back(&doc);
            let at = format!("{a:?} {first:?} then {b:?} {second:?}");
            let kids = back.layers[0].children().unwrap();
            assert_eq!(kids.len(), 2, "{at}");
            for (n, want) in kids.iter().zip([first, second]) {
                assert_eq!((n.opacity, n.blend, n.knockout_shape), want, "{at}");
            }
            let l = &back.layers[0];
            assert_eq!((l.opacity, l.blend, l.knockout_shape), plain, "the layer's own: {at}");
        }
    }
}

/// A layer whose last object is type keeps its own transparency, and the type keeps its.
#[test]
fn type_last_in_a_transparent_layer_keeps_its_own_transparency() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let text = beside(&mut doc, Beside::Type, 10.0, (0.5, BlendMode::Multiply, true));
    doc.insert(Some(layer), usize::MAX, text).unwrap();
    let l = Arc::make_mut(&mut doc.layers[0]);
    (l.opacity, l.blend) = (0.75, BlendMode::Screen);
    let back = read_back(&doc);
    let (l, n) = (&back.layers[0], &back.layers[0].children().unwrap()[0]);
    assert_eq!((l.opacity, l.blend), (0.75, BlendMode::Screen));
    assert_eq!((n.opacity, n.blend, n.knockout_shape), (0.5, BlendMode::Multiply, true));
}

/// Isolate Blending and the Knockout Group state of type come back, named or not and whatever
/// follows it, as outlines kept them on their group, and are written again as the same bytes.
#[test]
fn the_isolation_and_knockout_of_type_come_back() {
    for name in [None, Some("Title")] {
        let cases = [(true, Knockout::On), (false, Knockout::On), (false, Knockout::Off), (true, Knockout::Neutral), (false, Knockout::Neutral)];
        let mut doc = Document::new(200.0, 100.0);
        let layer = doc.layers[0].id;
        for (i, (isolate, knockout)) in cases.into_iter().enumerate() {
            let mut n = beside(&mut doc, Beside::Type, 10.0 + 30.0 * i as f64, (0.5, BlendMode::Multiply, false));
            (n.isolate, n.knockout) = (isolate, knockout);
            n.name = name.map(|s| format!("{s} {i}"));
            doc.insert(Some(layer), usize::MAX, n).unwrap();
        }
        let after = beside(&mut doc, Beside::Compound, 170.0, (1.0, BlendMode::Normal, false));
        doc.insert(Some(layer), usize::MAX, after).unwrap();
        let back = read_back(&doc);
        let kids = back.layers[0].children().unwrap();
        assert_eq!(kids.len(), cases.len() + 1);
        for (i, (n, (isolate, knockout))) in kids.iter().zip(cases).enumerate() {
            assert!(matches!(n.kind, NodeKind::Text(_)));
            assert_eq!((n.isolate, n.knockout, n.opacity, n.blend), (isolate, knockout, 0.5, BlendMode::Multiply), "{i}");
            assert_eq!(n.name, name.map(|s| format!("{s} {i}")));
        }
        assert_eq!(kids[cases.len()].opacity, 1.0);
        let art = |d: &Document| program(d).split_once("%%EndSetup").map(|(_, art)| art.to_string()).unwrap();
        assert_eq!(art(&back), art(&doc));
    }
}

/// The end marker of the text document stays on one line wherever the ASCII85 ends: the reader
/// looks for `~>` in the lines as they are, before it joins them.
#[test]
fn the_end_marker_of_the_text_document_stays_on_one_line() {
    let mut at_the_boundary = 0;
    // A character is two bytes: sizes of more digits reach the lengths that characters don't.
    for size in [10.0, 10.25, 10.125, 10.0625] {
        for len in 1..=60 {
            let mut doc = Document::new(200.0, 100.0);
            let layer = doc.layers[0].id;
            let text = "x".repeat(len);
            let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 40.0), &text, style(size, Color::BLACK));
            let t = text_node(&mut doc, t);
            doc.insert(Some(layer), usize::MAX, t).unwrap();
            let p = program(&doc);
            assert!(!p.contains("~\n%>"), "{size} {len}");
            // A full line (`%` and 100 characters) that the `>` goes past.
            at_the_boundary += usize::from(p.lines().any(|l| l.len() == 102 && l.ends_with("~>")));
            let back = read_back(&doc);
            let NodeKind::Text(read) = &back.layers[0].children().unwrap()[0].kind else { panic!("{size} {len}: not type") };
            assert_eq!(read.plain_text(), text);
        }
    }
    assert!(at_the_boundary > 0, "no length put the `~` at the end of a line");
}

/// An empty frame after a slice that ends with a paragraph break holds the story's last paragraph,
/// and that paragraph comes back with the empty frame's style.
#[test]
fn an_empty_last_frame_keeps_the_style_of_the_last_paragraph() {
    use vectorcraft_doc::{Justify, ParaStyle, TextKind, TextObject};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut first = TextObject::point(vectorcraft_geom::Point::ZERO, "Ends\n", style(10.0, Color::BLACK));
    first.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 40.0)) };
    first.xf = Affine::IDENTITY;
    let mut second = TextObject::point(vectorcraft_geom::Point::ZERO, "", style(10.0, Color::BLACK));
    second.set_all_paras(ParaStyle { justify: Justify::Right, ..ParaStyle::default() });
    second.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(110.0, 10.0, 190.0, 40.0)) };
    second.xf = Affine::IDENTITY;
    let (first, second) = (text_node(&mut doc, first), text_node(&mut doc, second));
    doc.text_threads.push(vec![first.id, second.id]);
    doc.insert(Some(layer), usize::MAX, first).unwrap();
    doc.insert(Some(layer), usize::MAX, second).unwrap();
    let back = read_back(&doc);
    let NodeKind::Text(t) = &back.layers[0].children().unwrap()[0].kind else { panic!("type") };
    assert_eq!(t.plain_text(), "Ends\n");
    let justify: Vec<Justify> = t.paragraph_styles().iter().map(|p| p.justify).collect();
    assert_eq!(justify, [Justify::Left, Justify::Right]);
}

/// A minimum first baseline with the Ascent first baseline is reported, and kept with the others.
/// An end bracket of an open path that the reader would read as none is reported, and one further
/// from the path's end comes back.
#[test]
fn a_first_baseline_minimum_and_a_path_end_the_reader_cant_read_are_reported() {
    use vectorcraft_doc::{FirstBaseline, TextKind, TextObject};
    let with = |t: TextObject| {
        let mut d = Document::new(300.0, 200.0);
        let layer = d.layers[0].id;
        let n = text_node(&mut d, t);
        d.insert(Some(layer), usize::MAX, n).unwrap();
        let warnings = export_ai_editing(&d, &opts()).unwrap().warnings;
        (warnings, read_back(&d))
    };
    let mut area = TextObject::point(vectorcraft_geom::Point::ZERO, "Area", style(12.0, Color::BLACK));
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(20.0, 20.0, 140.0, 160.0)) };
    area.xf = Affine::IDENTITY;
    area.area.first_baseline_min = 40.0;
    let (warnings, _) = with(area.clone());
    assert_eq!(warnings, [writer::TYPE_ATTRIBUTES]);
    area.area.first_baseline = FirstBaseline::Fixed;
    let (warnings, back) = with(area);
    assert!(warnings.is_empty(), "{warnings:?}");
    let NodeKind::Text(t) = &back.layers[0].children().unwrap()[0].kind else { panic!("type") };
    assert_eq!((t.area.first_baseline, t.area.first_baseline_min), (FirstBaseline::Fixed, 40.0));

    let on_path = |end: Option<f64>| {
        let mut curve = PathData::from_bezpath(&kurbo::BezPath::from_vec(vec![
            kurbo::PathEl::MoveTo((20.0, 150.0).into()),
            kurbo::PathEl::CurveTo((50.0, 110.0).into(), (110.0, 110.0).into(), (140.0, 150.0).into()),
        ]));
        curve.subpaths.iter_mut().for_each(|sp| sp.closed = false);
        let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "On", style(12.0, Color::BLACK));
        t.kind = TextKind::OnPath { path: curve, start: 0.0, end };
        t.xf = Affine::IDENTITY;
        t
    };
    assert_eq!(with(on_path(Some(0.999))).0, [writer::TYPE_ATTRIBUTES]);
    assert!(with(on_path(None)).0.is_empty());
    let (warnings, back) = with(on_path(Some(0.5)));
    assert!(warnings.is_empty(), "{warnings:?}");
    let NodeKind::Text(t) = &back.layers[0].children().unwrap()[0].kind else { panic!("type") };
    let TextKind::OnPath { end: Some(end), .. } = t.kind else { panic!("an end bracket") };
    assert!((end - 0.5).abs() < 1e-3, "{end}");
}

/// A right-to-left paragraph aligned to the start of its direction comes back aligned right, as
/// it draws; Latin type aligned to the start comes back aligned left.
#[test]
fn type_aligned_to_the_start_of_its_direction_keeps_its_side() {
    use vectorcraft_doc::{Justify, ParaDirection, ParaStyle, TextObject};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut rtl = TextObject::point(vectorcraft_geom::Point::new(150.0, 40.0), "\u{5e9}\u{5dc}\u{5d5}\u{5dd}", style(12.0, Color::BLACK));
    rtl.set_all_paras(ParaStyle { justify: Justify::Auto, direction: Some(ParaDirection::RightToLeft), ..ParaStyle::default() });
    let mut ltr = TextObject::point(vectorcraft_geom::Point::new(20.0, 80.0), "Start", style(12.0, Color::BLACK));
    ltr.set_all_paras(ParaStyle { justify: Justify::Auto, ..ParaStyle::default() });
    let (rtl, ltr) = (text_node(&mut doc, rtl), text_node(&mut doc, ltr));
    doc.insert(Some(layer), usize::MAX, rtl).unwrap();
    doc.insert(Some(layer), usize::MAX, ltr).unwrap();
    assert!(export_ai_editing(&doc, &opts()).unwrap().warnings.is_empty());
    let back = read_back(&doc);
    let sides: Vec<Justify> =
        back.layers[0].children().unwrap().iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.para.justify) } else { None }).collect();
    assert_eq!(sides, [Justify::Right, Justify::Left]);
}

/// Type in an opacity mask is written as its outlines; the type beside it as text.
#[test]
fn type_in_opacity_masks_is_written_as_its_outlines() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let label = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 40.0), "Mask", style(18.0, Color::BLACK));
    let label = text_node(&mut doc, label);
    let mut square = square(&mut doc, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(1.0, 0.0, 0.0));
    square.extras_mut().mask = Some(Box::new(OpacityMask::new(label, false)));
    doc.insert(Some(layer), usize::MAX, square).unwrap();
    let plain = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 90.0), "Plain", style(18.0, Color::BLACK));
    let plain = text_node(&mut doc, plain);
    doc.insert(Some(layer), usize::MAX, plain).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::TYPE]);
    let text = program(&doc);
    assert_eq!(text.matches("/AI11Text :").count(), 1);
}

/// Type with an opacity mask of its own is written as its outlines, which keep the mask.
#[test]
fn type_with_an_opacity_mask_is_written_as_its_outlines() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 70.0), "Masked", style(18.0, Color::BLACK));
    let mut t = text_node(&mut doc, t);
    let art = square(&mut doc, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(1.0, 1.0, 1.0));
    t.extras_mut().mask = Some(Box::new(OpacityMask::new(art, false)));
    doc.insert(Some(layer), usize::MAX, t).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::TYPE]);
    assert_eq!(program(&doc).matches("/AI11Text :").count(), 0);
    let back = read_back(&doc);
    let top = &back.layers[0].children().unwrap()[0];
    assert!(!matches!(top.kind, NodeKind::Text(_)));
    assert!(top.extras().mask.is_some());
}
