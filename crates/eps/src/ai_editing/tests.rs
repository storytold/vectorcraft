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
            // The matrix is written to ten decimals.
            NodeKind::SymbolInstance { symbol, xf } => {
                let xf: Vec<String> = xf.as_coeffs().iter().map(|v| format!("{:.6}", v + 0.0)).collect();
                out.push(format!("{head} instance {symbol:?} [{}] {} {:?}", xf.join(" "), r3(f64::from(n.opacity)), n.blend));
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

/// The CMYK equivalent written beside RGB or Lab values is the one of the values as written (to four
/// decimals, Lab clamped to the range a reader reads), so the reopened file is written with the same
/// numbers: a new document's swatches, and colors of many decimals in fills, gradient stops, global
/// process swatches (RGB and Lab), an RGB spot swatch (in a fill and in a stop), Lab spot swatches
/// and a Lab spot swatch out of range.
#[test]
fn cmyk_equivalents_are_written_again_unchanged() {
    let mut doc = Document::new(300.0, 200.0);
    let v = |i: u16, s: f32| (f32::from(i) * s + 0.123_456_7).fract();
    let rgb = |i: u16| Color::rgb(v(i, 0.137_911_3), v(i, 0.271_828_2), v(i, 0.314_159_3));
    let lab = |i: u16| Color::lab(v(i, 0.137_911_3) * 100.0, v(i, 0.271_828_2) * 200.0 - 100.0, v(i, 0.314_159_3) * 200.0 - 100.0);
    let glaze = rgb(102);
    let swatch = |name: String, c: Color, spot: bool| Swatch { name, paint: solid(c), global: true, spot };
    let mut named: Vec<(String, Color, bool)> =
        vec![("Glaze".into(), glaze, true), ("Out".into(), Color::lab(104.567_89, -140.123_46, 131.987_65), true)];
    for i in 0..8u16 {
        named.push((format!("Leaf {i}"), rgb(i + 110), false));
        named.push((format!("Fern {i}"), lab(i + 120), false));
        named.push((format!("Moss {i}"), lab(i + 130), true));
    }
    let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
    swatches.extend(named.iter().map(|(name, c, spot)| swatch(name.clone(), *c, *spot)));
    doc.swatches = swatches.into();
    let layer = doc.layers[0].id;
    for i in 0..24u16 {
        let x = f64::from(i % 8) * 30.0 + 10.0;
        let y = f64::from(i / 8) * 30.0 + 10.0;
        let n = square(&mut doc, Rect::new(x, y, x + 20.0, y + 20.0), rgb(i));
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let mut stops: Vec<GradientStop> = (0..5u16).map(|i| GradientStop::new(f32::from(i) / 5.0, rgb(i + 50))).collect();
    let out = named[1].1;
    stops.push(GradientStop { swatch: Some("Out".into()), tint: 0.5, ..GradientStop::new(0.9, out.tinted(0.5)) });
    stops.push(GradientStop { swatch: Some("Glaze".into()), tint: 0.7, ..GradientStop::new(1.0, glaze.tinted(0.7)) });
    let mut fills = vec![gradient_paint(Gradient::new(GradientKind::Linear, stops), None, None)];
    fills.extend(named.iter().map(|(name, c, spot)| linked(name, *c, if *spot { 1.0 } else { 0.6 })));
    for (i, fill) in fills.into_iter().enumerate() {
        let x = (i % 9) as f64 * 30.0 + 10.0;
        let y = (i / 9) as f64 * 30.0 + 110.0;
        let n = painted(&mut doc, shapes::rectangle(Rect::new(x, y, x + 20.0, y + 20.0)), vec![AppearanceItem::Fill(FillLayer::new(fill))]);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let first = program(&doc);
    let again = String::from_utf8(export_ai_editing(&read_back(&doc), &opts()).unwrap().program).unwrap().replace("\r\n", "\n");
    same_lines(&again.lines().map(String::from).collect::<Vec<_>>(), &first.lines().map(String::from).collect::<Vec<_>>());
}

/// With a working RGB space other than sRGB, an RGB fill, an RGB gradient stop and an RGB spot
/// color in a stop are written with their working RGB values, the values the reader reads back, so
/// the reopened file is written with the same RGB values and CMYK equivalents.
#[test]
fn rgb_values_are_written_again_unchanged_in_a_wide_gamut_working_space() {
    const CHILD: &str = "VECTORCRAFT_TEST_AI_EDITING_WIDE_GAMUT";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: exercise this profile in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ai_editing::tests::rgb_values_are_written_again_unchanged_in_a_wide_gamut_working_space", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated color profile test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    use vectorcraft_color::cms;
    cms::set_active(&cms::ColorSettings { rgb: cms::WIDE_GAMUT_RGB.into(), ..Default::default() }).unwrap();
    assert!(!cms::active().rgb_is_srgb());
    let mut doc = Document::new(300.0, 200.0);
    let glaze = Color::rgb(0.812_345_6, 0.234_567_8, 0.456_789_1);
    let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
    swatches.push(Swatch { name: "Glaze".into(), paint: solid(glaze), global: true, spot: true });
    doc.swatches = swatches.into();
    let layer = doc.layers[0].id;
    let n = square(&mut doc, Rect::new(10.0, 10.0, 30.0, 30.0), Color::rgb(0.8, 0.4, 0.2));
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let stops = vec![
        GradientStop::new(0.0, Color::rgb(0.1, 0.7, 0.3)),
        GradientStop { swatch: Some("Glaze".into()), tint: 0.7, ..GradientStop::new(1.0, glaze.tinted(0.7)) },
    ];
    let fill = gradient_paint(Gradient::new(GradientKind::Linear, stops), None, None);
    let n = painted(&mut doc, shapes::rectangle(Rect::new(40.0, 10.0, 60.0, 30.0)), vec![AppearanceItem::Fill(FillLayer::new(fill))]);
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let first = program(&doc);
    assert!(first.contains("0.8 0.4 0.2 Xa"), "the fill's working RGB values are written");
    let again = String::from_utf8(export_ai_editing(&read_back(&doc), &opts()).unwrap().program).unwrap().replace("\r\n", "\n");
    same_lines(&again.lines().map(String::from).collect::<Vec<_>>(), &first.lines().map(String::from).collect::<Vec<_>>());
}

/// A gradient stop of a Lab spot color writes the RGB values of the Lab values as written (to four
/// decimals, clamped to the range a reader reads), the same Lab values its CMYK equivalent and the
/// palette's definition come from: for Lab values of many decimals and for Lab values out of range.
#[test]
fn a_lab_spot_stop_writes_rgb_values_of_its_lab_values_as_written() {
    let mut doc = Document::new(300.0, 200.0);
    let v = |i: u16, s: f32| (f32::from(i) * s + 0.123_456_7).fract();
    let mut spots: Vec<(String, Color)> = vec![("Out".into(), Color::lab(104.567_89, -140.123_46, 131.987_65))];
    spots.push(("Rim".into(), Color::lab(55.555_55, 131.234_56, -12.345_67)));
    for i in 0..8u16 {
        spots
            .push((format!("Moss {i}"), Color::lab(v(i, 0.137_911_3) * 100.0, v(i, 0.271_828_2) * 200.0 - 100.0, v(i, 0.314_159_3) * 200.0 - 100.0)));
    }
    let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
    swatches.extend(spots.iter().map(|(name, c)| Swatch { name: name.clone(), paint: solid(*c), global: true, spot: true }));
    doc.swatches = swatches.into();
    let layer = doc.layers[0].id;
    for (i, (name, c)) in spots.iter().enumerate() {
        let stops = vec![
            GradientStop::new(0.0, Color::rgb(0.1, 0.2, 0.3)),
            GradientStop { swatch: Some(name.clone()), tint: 0.5, ..GradientStop::new(1.0, c.tinted(0.5)) },
        ];
        let fill = gradient_paint(Gradient::new(GradientKind::Linear, stops), None, None);
        let x = i as f64 * 25.0 + 10.0;
        let n = painted(&mut doc, shapes::rectangle(Rect::new(x, 10.0, x + 20.0, 30.0)), vec![AppearanceItem::Fill(FillLayer::new(fill))]);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let program = program(&doc);
    for (name, c) in &spots {
        let mut want = String::new();
        crate::ps::push_nums(&mut want, &crate::ai_ops::as_written(*c).to_rgb().map(|v| f64::from(crate::ai_ops::unit(v))));
        want.push_str(&format!("({name}) 0.5 1 4 "));
        assert!(program.lines().any(|l| l.contains(&want)), "no stop of {name} writes {want:?}");
    }
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
    let (alone, notes) = crate::ai_alone_on_page(&p.data, (20_000.0, 20_000.0), 1.389, crate::NameEncoding::default()).unwrap();
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

/// A document with one top-level layer for each of the 27 layer colour presets, named after its
/// position.
fn every_layer_preset() -> Document {
    let mut doc = Document::new(100.0, 100.0);
    doc.layers.clear();
    for i in 0..vectorcraft_doc::LAYER_COLORS.len() {
        let id = doc.alloc_id();
        doc.layers.push(Arc::new(Node::layer(id, &format!("P{i}"), LayerColor::Preset(u8::try_from(i).unwrap()))));
    }
    doc
}

/// #6502: Yellow, Magenta and Cyan (presets 4, 5 and 6) are written as the file's indexes 6, 4
/// and 5, with their own RGB beside them; every other preset keeps its position.
#[test]
fn layer_presets_are_written_with_the_files_colour_index() {
    let text = program(&every_layer_preset());
    for (i, (_, [r, g, b])) in vectorcraft_doc::LAYER_COLORS.iter().enumerate() {
        let index = match i {
            4 => 6,
            5 => 4,
            6 => 5,
            i => i,
        };
        let want = format!(" {index} {r} {g} {b} 0 50 0 Lb\n(P{i}) Ln\n");
        assert!(text.contains(&want), "missing {want:?}");
    }
}

/// #6502: every preset comes back as itself after a save and an open.
#[test]
fn layer_presets_survive_a_round_trip() {
    let doc = read_back(&every_layer_preset());
    let colors: Vec<LayerColor> = doc
        .layers
        .iter()
        .map(|l| match l.kind {
            NodeKind::Layer { color, .. } => color,
            _ => panic!("{:?}", l.kind),
        })
        .collect();
    let want: Vec<LayerColor> = (0..27u8).map(LayerColor::Preset).collect();
    assert_eq!(colors, want);
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

/// A 200 × 100 document of a gradient fill, a compound path, an image and a symbol instance, and
/// what the data holds as plain paths: type.
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
    assert_eq!(out.warnings, [writer::TYPE]);
    let back = read_back(&doc);
    let layer = back.layers[0].children().unwrap();
    let kinds: Vec<&str> = layer
        .iter()
        .map(|n| match n.kind {
            NodeKind::Path { .. } => "path",
            NodeKind::Group { .. } => "group",
            NodeKind::Compound { .. } => "compound",
            NodeKind::Image(_) => "image",
            NodeKind::SymbolInstance { .. } => "instance",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["path", "compound", "image", "group", "instance"]);
    let mut all = vec![];
    paths(&layer[3..4], &mut all);
    assert!(all.iter().all(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    // The gradient as a gradient, drawing as it did.
    let bounds = Rect::new(10.0, 10.0, 30.0, 30.0);
    let original = doc.layers[0].children().unwrap()[0].appearance.items[0].paint();
    assert_eq!(drawn(&back, layer[0].appearance.items[0].paint(), bounds), drawn(&doc, original, bounds));
    // The image as an image.
    assert_eq!(layer[2].visual_bounds().map(|b| (r3(b.x0), r3(b.y0), r3(b.x1), r3(b.y1))), Some((r3(100.0), r3(10.0), r3(120.0), r3(20.0))));
}

/// A brushed stroke the reader reads as its drawn look (a Scatter brush's) is written as its brush's art.
#[test]
fn brushed_strokes_are_written_as_their_brush_art() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut line = Node::path(
        doc.alloc_id(),
        shapes::line((20.0, 50.0).into(), (180.0, 50.0).into()),
        Appearance::basic(Paint::None, solid(Color::rgb(1.0, 0.0, 0.0)), 4.0),
    );
    line.appearance.stroke_mut().unwrap().brush = Some("Dots".into());
    doc.insert(Some(layer), usize::MAX, line).unwrap();
    let pieces = vectorcraft_brush::stroke_pieces(
        &doc,
        &vectorcraft_brush::find(&doc, "Dots").unwrap(),
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

/// Nested symbols whose instances multiply, written as their art (on a large canvas), stop at
/// [`vectorcraft_doc::MAX_SYMBOL_NODES`] objects of symbol art in all, with a warning.
#[test]
fn nested_symbol_instances_stop_at_the_symbol_art_limit() {
    let mut doc = Document::new(20000.0, 100.0);
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
    // On the layers; the symbols' definitions come before them, in the setup.
    let layers = text.split_once("%%EndSetup").unwrap().1;
    assert_eq!(layers.matches("\r\nf\r\n").count(), 3198, "the squares of the leaves written");
}

/// The documents [`the_writer_counts_objects_as_the_reader_reads_them`] writes: groups, compound
/// paths, guides, sublayers, clipping groups and layers, painted clipping paths, opacity masks,
/// gradients, pattern definitions and tiles, images, type, symbol definitions and instances,
/// objects with art styles and objects bound to variables.
fn counted() -> Vec<(&'static str, Document)> {
    vec![
        ("sample", sample()),
        ("unsupported", unsupported()),
        ("transparent", transparent()),
        ("gradients and patterns", gradients_and_patterns()),
        ("images", images_doc()),
        ("typed", typed()),
        ("variables", variables_doc()),
        ("symbols", symbols_document()),
        ("styles", styles_document()),
        ("painted clips", painted_clips()),
        ("blend", blend_doc()),
        ("brushes", brushes_document()),
    ]
}

/// A blend of two squares 20 pt apart with a step every 10 pt, written as its steps.
fn blend_doc() -> Document {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let keys = vec![
        Arc::new(square(&mut doc, Rect::new(10.0, 10.0, 12.0, 12.0), Color::gray(0.5))),
        Arc::new(square(&mut doc, Rect::new(30.0, 10.0, 32.0, 12.0), Color::gray(0.5))),
    ];
    let spec = vectorcraft_doc::BlendSpec { spacing: vectorcraft_doc::BlendSpacing::Distance(10.0), ..Default::default() };
    let blend = Node::new(doc.alloc_id(), NodeKind::Blend { children: keys, spec: Box::new(spec) });
    doc.insert(Some(layer), usize::MAX, blend).unwrap();
    doc
}

/// Clipping groups whose clipping paths are filled: a square with a color, and a ring (a path of
/// two subpaths) with a color, with a gradient, and with a color and a gradient stroke.
fn painted_clips() -> Document {
    let mut doc = Document::new(260.0, 100.0);
    let layer = doc.layers[0].id;
    let ring = |x: f64| {
        let mut p = shapes::rectangle(Rect::new(x, 10.0, x + 20.0, 30.0));
        p.subpaths.extend(shapes::rectangle(Rect::new(x + 5.0, 15.0, x + 15.0, 25.0)).subpaths);
        p
    };
    let g = Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, Color::rgb(1.0, 1.0, 1.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 0.0))],
    );
    let shade = || Paint::Gradient(Box::new(GradientPaint::new(g.clone())));
    let clips = [
        (10.0, shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None),
        (70.0, ring(70.0), solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None),
        (130.0, ring(130.0), shade(), Paint::None),
        (190.0, ring(190.0), solid(Color::rgb(0.0, 1.0, 0.0)), shade()),
    ];
    for (x, path, fill, stroke) in clips {
        let mut clip = clipping(&mut doc, path);
        let width = if matches!(stroke, Paint::None) { 0.0 } else { 2.0 };
        clip.appearance = Appearance::basic(fill, stroke, width).into();
        let inside = square(&mut doc, Rect::new(x - 5.0, 5.0, x + 10.0, 20.0), Color::gray(0.5));
        let g = group(&mut doc, vec![clip, inside], true);
        doc.insert(Some(layer), usize::MAX, g).unwrap();
    }
    doc
}

/// The writer counts the objects it writes as the reader counts the objects it reads: with room
/// for as many objects as the reader reads, nothing is left out; with one less, art is left out,
/// with a warning. With room for fewer, the reader never reads more objects than there is room for.
#[test]
fn the_writer_counts_objects_as_the_reader_reads_them() {
    let objects = |out: &AiEditingOutput| crate::import::objects_read(&out.program).unwrap();
    let limited = |out: &AiEditingOutput| out.warnings.iter().any(|w| w == writer::OBJECT_LIMIT);
    for (name, doc) in counted() {
        let all = export_ai_editing(&doc, &opts()).unwrap();
        let read = objects(&all);
        assert!(read > 0 && !limited(&all), "{name}: {read} objects, {:?}", all.warnings);
        let room = export_within(&doc, &opts(), read).unwrap();
        assert!(!limited(&room), "{name}: nothing is left out with room for {read} objects");
        assert_eq!(room.program, all.program, "{name}");
        for max in 0..read {
            let out = export_within(&doc, &opts(), max).unwrap();
            assert!(limited(&out), "{name}: art is left out with room for {max} of {read} objects");
            assert!(objects(&out) <= max, "{name}: {} objects read with room for {max}", objects(&out));
        }
    }
}

/// An opacity mask whose art doesn't all fit within the object limit is left out with its art,
/// and the object it masks is written without it; the art left out leaves room for the objects
/// after it.
#[test]
fn a_mask_whose_art_passes_the_object_limit_is_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut masked = square(&mut doc, Rect::new(10.0, 10.0, 50.0, 50.0), Color::gray(0.5));
    let a = square(&mut doc, Rect::new(0.0, 0.0, 30.0, 30.0), Color::rgb(1.0, 1.0, 1.0));
    let b = square(&mut doc, Rect::new(30.0, 30.0, 60.0, 60.0), Color::rgb(1.0, 1.0, 1.0));
    let art = group(&mut doc, vec![a, b], false);
    masked.extras_mut().mask = Some(Box::new(OpacityMask::new(art, false)));
    doc.insert(Some(layer), usize::MAX, masked).unwrap();
    let after = square(&mut doc, Rect::new(100.0, 10.0, 140.0, 50.0), Color::gray(0.5));
    doc.insert(Some(layer), usize::MAX, after).unwrap();
    let text = |max: usize| {
        let out = export_within(&doc, &opts(), max).unwrap();
        let limited = out.warnings.iter().any(|w| w == writer::OBJECT_LIMIT);
        (String::from_utf8_lossy(&out.program).replace("\r\n", "\n"), limited)
    };
    let read = crate::import::objects_read(&export_ai_editing(&doc, &opts()).unwrap().program).unwrap();
    let (all, limited) = text(read);
    assert!(!limited && all.contains("(AI9_Mask)"), "{all}");
    let has_mask = |max: usize| text(max).0.contains("(AI9_Mask)");
    let Some(fit) = (0..=read).find(|&max| has_mask(max)) else { panic!("the mask fits with room for {read} objects") };
    assert!(fit > 2, "the mask needs room beyond the two squares: {fit}");
    for max in fit..=read {
        assert!(has_mask(max), "room for {max} of {read}, the mask fits in {fit}");
    }
    let (short, limited) = text(fit - 1);
    assert!(limited);
    assert!(!short.contains("(AI9_Mask)") && !short.contains("%_/ArtDictionary :\n%_/Mask"), "{short}");
    assert_eq!(short.matches("\nf\n").count(), 2, "both squares are written: {short}");
}

/// A symbol's definition is written only when there is room for its instance too.
#[test]
fn a_symbol_definition_is_written_only_with_room_for_its_instance() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let a = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), Color::gray(0.5));
    let b = square(&mut doc, Rect::new(10.0, 10.0, 20.0, 20.0), Color::gray(0.5));
    let art = group(&mut doc, vec![a, b], false);
    doc.symbols.push(Symbol { dynamic: false, name: "Pair".into(), art: Arc::new(art) });
    let placed = instance(&mut doc, "Pair", Affine::translate((50.0, 10.0)));
    doc.insert(Some(layer), usize::MAX, placed).unwrap();
    add_rect(&mut doc, Rect::new(100.0, 10.0, 110.0, 20.0));
    let program = export_ai_editing(&doc, &opts()).unwrap().program;
    let all = String::from_utf8_lossy(&program).replace("\r\n", "\n");
    assert!(all.contains("%AI14_BeginSymbol") && all.contains("/SymbolInstance :"), "{all}");
    let read = crate::import::objects_read(&program).unwrap();
    for max in 0..read {
        let out = export_within(&doc, &opts(), max).unwrap();
        let text = String::from_utf8_lossy(&out.program).replace("\r\n", "\n");
        let defined = text.contains("%AI14_BeginSymbol");
        assert_eq!(defined, text.contains("/SymbolInstance :"), "room for {max} of {read}: {text}");
        assert!(out.warnings.iter().any(|w| w == writer::OBJECT_LIMIT), "art is left out with room for {max} of {read} objects");
        let objects = crate::import::objects_read(&out.program).unwrap();
        assert!(objects <= max, "{objects} objects read with room for {max}");
    }
}

/// The writer's limit is the reader's, which its warning names.
#[test]
fn the_object_limit_is_the_readers() {
    assert_eq!(crate::import::MAX_EDITING_OBJECTS, 4_000_000);
    assert!(writer::OBJECT_LIMIT.contains("4,000,000"));
}

/// Paths past the object limit are left out of a layer of many paths, with a warning: the paths
/// that fit are written.
#[test]
fn paths_past_the_object_limit_are_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    for i in 0..10 {
        add_rect(&mut doc, Rect::new(f64::from(i) * 10.0, 0.0, f64::from(i) * 10.0 + 5.0, 5.0));
    }
    let out = export_within(&doc, &opts(), 6).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::OBJECT_LIMIT), "{:?}", out.warnings);
    assert_eq!(crate::import::objects_read(&out.program).unwrap(), 6);
    let text = String::from_utf8_lossy(&out.program).replace("\r\n", "\n");
    assert_eq!(text.matches("\nf\n").count(), 6, "the first six paths are written");
}

/// On a canvas of the usual size, nested symbols whose instances multiply, written as their art in
/// art cut to a clipping object, stop at [`vectorcraft_doc::MAX_SYMBOL_NODES`] objects of symbol art
/// in all, with a warning.
#[test]
fn clipped_nested_symbol_instances_stop_at_the_symbol_art_limit() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    // Empty paths: counted, but written as nothing, so the test stays small; and one filled square.
    let empty = Arc::new(Node::path(doc.alloc_id(), PathData::new(vec![]), Appearance::default()));
    let mut members = vec![empty; 4999];
    let square_art = shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0));
    members.push(Arc::new(Node::path(doc.alloc_id(), square_art, Appearance::basic(solid(Color::gray(0.5)), Paint::None, 0.0))));
    let leaves = Node::group(doc.alloc_id(), members);
    doc.symbols.push(Symbol { dynamic: false, name: "Leaf".into(), art: Arc::new(leaves) });
    let leaf = Arc::new(Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Leaf".into(), xf: Affine::IDENTITY }));
    let trees = Node::group(doc.alloc_id(), vec![leaf; 5000]);
    doc.symbols.push(Symbol { dynamic: false, name: "Tree".into(), art: Arc::new(trees) });
    let tree = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "Tree".into(), xf: Affine::IDENTITY });
    // A group as the clipping object cuts the art instead of clipping it with a path, so the
    // instances in it are written as their art. Its square is unpainted, so the only filled squares
    // are the leaves'.
    let outline = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default());
    let clip = group(&mut doc, vec![outline], false);
    let clipped = group(&mut doc, vec![clip, tree], true);
    doc.insert(Some(layer), usize::MAX, clipped).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::SYMBOL_LIMIT), "{:?}", out.warnings);
    assert!(out.warnings.iter().any(|w| w == writer::SYMBOL_ART), "{:?}", out.warnings);
    // 5,001 objects for the tree and each leaf: 3,198 leaves fit in 16,000,000.
    let text = String::from_utf8(out.program).unwrap();
    let layers = text.split_once("%%EndSetup").unwrap().1;
    assert_eq!(layers.matches("\r\nf\r\n").count(), 3198, "the squares of the leaves written");
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
    let b = back.bounds_of_with(&[back.layers[0].children().unwrap()[0].id], true, &back.symbol_boxes()).unwrap();
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
    let art: Vec<Arc<Node>> = back.symbols.iter().map(|s| s.art.clone()).collect();
    pictures(&back, &art, "", &mut found);
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
            NodeKind::SymbolInstance { symbol, .. } => format!("instance {symbol:?}"),
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

/// A clipping path filled and stroked is written with its fill and stroke, and reads back with
/// both on the clipping path.
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
            (true, Some(paint(&solid(Color::rgb(1.0, 1.0, 0.0)))), Some(paint(&solid(Color::BLACK)))),
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

/// A new document's gradient swatches read back with the paint they were written with, not linked
/// to a swatch, and art filled with one of them reads back linked to it.
#[test]
fn gradient_swatches_read_back_unlinked() {
    let mut doc = Document::new(200.0, 100.0);
    let Some(Paint::Gradient(sunset)) = doc.swatch("Sunset").map(|s| s.paint.clone()) else { panic!("no Sunset swatch") };
    let layer = doc.layers[0].id;
    let linked = gradient_paint(sunset.gradient.clone(), None, Some("Sunset"));
    let rect = painted(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 50.0)), vec![AppearanceItem::Fill(FillLayer::new(linked))]);
    doc.insert(Some(layer), usize::MAX, rect).unwrap();
    let back = read_back(&doc);
    let gradients = |d: &Document| -> Vec<String> {
        d.swatches_iter()
            .filter_map(|s| match &s.paint {
                Paint::Gradient(g) => {
                    Some(format!("{} {:?} stops={} geom={:?} swatch={:?}", s.name, g.gradient.kind, g.gradient.stops.len(), g.geom, g.swatch))
                }
                _ => None,
            })
            .collect()
    };
    let want = gradients(&doc);
    assert_eq!(want.len(), 4, "{want:?}");
    assert_eq!(gradients(&back), want);
    let link = back.layers[0].children().and_then(|c| c.first()).map(|n| match n.appearance.items.first().map(AppearanceItem::paint) {
        Some(Paint::Gradient(g)) => g.swatch.clone(),
        other => panic!("{other:?}"),
    });
    assert_eq!(link, Some(Some("Sunset".to_string())));
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

/// A clipping path filled with a gradient keeps it, and reads back with it.
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
    assert_eq!(children.len(), 2, "the clipping path and the art");
    let fill = &children[0];
    assert!(matches!(fill.kind, NodeKind::Path { clipping: true, .. }), "{:?}", fill.kind);
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
        stroke_cap: vectorcraft_doc::LineCap::Square,
        stroke_join: vectorcraft_doc::LineJoin::Bevel,
        stroke_miter_limit: 2.5,
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
                "  run {:?} {} {} {} {:?} {} {} {} {} fill {} stroke {} {} {:?} {:?} {} caps={} small={:?} tcy={} roman={}",
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
                s.stroke_cap,
                s.stroke_join,
                r3(s.stroke_miter_limit),
                s.all_caps,
                s.small_caps.map(r3),
                s.tate_chu_yoko,
                s.standard_vertical_roman,
            ));
        }
    }
    out
}

// ---------- symbols ----------

const LEAF: Color = Color::Cmyk { c: 0.6, m: 0.0, y: 0.9, k: 0.1 };

fn instance(doc: &mut Document, symbol: &str, xf: Affine) -> Node {
    Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: symbol.into(), xf })
}

/// A `w` × `h` document whose swatches are one CMYK color (the default swatches' CMYK equivalents
/// change at the fourth decimal when read back, so a second write would differ there).
fn one_swatch_document(w: f64, h: f64) -> Document {
    let mut doc = Document::new(w, h);
    doc.swatches = vec![Swatch { name: "Leaf".into(), paint: solid(LEAF), global: false, spot: false }].into();
    doc.swatch_groups = vec![].into();
    doc
}

/// A 300 × 200 document of symbols. Its symbols, in the Symbols panel's order: `Spare`, a path no
/// instance uses; `Pair`, a group of a red square and an instance of `Leaf` at half its size; `Leaf`,
/// a group of a square in `LEAF` and an ellipse, its registration point the top left; `Shade`, a
/// group of a rectangle filled with a gradient, a clipping group and an image, its registration
/// point the bottom right; `Solo`, a group of one square, which no instance uses. Layer `Art`
/// holds, bottom first: `Leaf` moved; `Leaf` turned 30° and scaled 1.5 by 0.5, named `Turned`; a
/// hidden one; a locked one; one at 50% in Multiply; `Pair`; a group holding `Leaf`; a clipping
/// group holding `Leaf`; `Leaf` with an opacity mask whose art holds an instance of `Leaf`; `Shade`;
/// and sublayer `Inner` (60%, Screen), which holds `Pair` mirrored.
fn symbols_document() -> Document {
    use vectorcraft_color::gradient::GradientGeom;
    let mut doc = Document::new(300.0, 200.0);
    doc.swatches = vec![Swatch { name: "Leaf".into(), paint: solid(LEAF), global: false, spot: false }].into();
    doc.swatch_groups = vec![].into();
    doc.layers.clear();
    let art_layer = doc.add_layer(Some("Art"));
    // The symbols.
    let spare = square(&mut doc, Rect::new(-5.0, -5.0, 5.0, 5.0), Color::gray(0.4));
    let leaf_square = square(&mut doc, Rect::new(0.0, 0.0, 20.0, 20.0), LEAF);
    let ellipse =
        Node::path(doc.alloc_id(), shapes::ellipse(Rect::new(5.0, 10.0, 25.0, 30.0)), Appearance::basic(Paint::None, solid(Color::BLACK), 1.5));
    let leaf = group(&mut doc, vec![leaf_square, ellipse], false);
    let red = square(&mut doc, Rect::new(-10.0, -10.0, 10.0, 10.0), Color::rgb(1.0, 0.0, 0.0));
    let small = instance(&mut doc, "Leaf", Affine::translate((15.0, -5.0)) * Affine::scale(0.5));
    let pair = group(&mut doc, vec![red, small], false);
    let geom = GradientGeom { start: (0.0, 0.0).into(), end: (40.0, 10.0).into(), aspect: 1.0, focal: None };
    let shaded = painted(
        &mut doc,
        shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 20.0)),
        vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(sunset(), Some(geom), None)))],
    );
    let edge = clipping(&mut doc, shapes::ellipse(Rect::new(0.0, 20.0, 20.0, 40.0)));
    let inside = square(&mut doc, Rect::new(0.0, 25.0, 40.0, 35.0), Color::rgb(0.0, 0.0, 1.0));
    let clipped = group(&mut doc, vec![edge, inside], true);
    let pixels = png_of(image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 128, 255, 255]).unwrap()));
    let picture = image_node(&mut doc, ImageBlob::new("image/png", pixels), (2, 1), Affine::new([10.0, 0.0, 0.0, 10.0, 25.0, 25.0]), None);
    let shade = group(&mut doc, vec![shaded, clipped, picture], false);
    let lone = square(&mut doc, Rect::new(0.0, 0.0, 8.0, 8.0), Color::gray(0.2));
    let solo = group(&mut doc, vec![lone], false);
    for (name, art) in [("Spare", spare), ("Pair", pair), ("Leaf", leaf), ("Shade", shade), ("Solo", solo)] {
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
    }
    doc.set_symbol_registration("Leaf", vectorcraft_doc::SymbolRegistration(0));
    doc.set_symbol_registration("Shade", vectorcraft_doc::SymbolRegistration(8));
    // The instances.
    let mut art = vec![instance(&mut doc, "Leaf", Affine::translate((10.0, 10.0)))];
    let turned = Affine::translate((80.0, 20.0)) * Affine::rotate(30f64.to_radians()) * Affine::scale_non_uniform(1.5, 0.5);
    art.push(named(instance(&mut doc, "Leaf", turned), "Turned"));
    let mut hidden = instance(&mut doc, "Leaf", Affine::translate((130.0, 10.0)));
    hidden.visible = false;
    art.push(hidden);
    let mut locked = instance(&mut doc, "Leaf", Affine::translate((160.0, 10.0)));
    locked.locked = true;
    art.push(locked);
    art.push(see_through(instance(&mut doc, "Leaf", Affine::translate((190.0, 10.0))), 0.5, BlendMode::Multiply));
    art.push(instance(&mut doc, "Pair", Affine::translate((240.0, 30.0))));
    let grouped = instance(&mut doc, "Leaf", Affine::translate((10.0, 60.0)));
    art.push(group(&mut doc, vec![grouped], false));
    let window = clipping(&mut doc, shapes::rectangle(Rect::new(50.0, 60.0, 65.0, 75.0)));
    let clipped = instance(&mut doc, "Leaf", Affine::translate((50.0, 60.0)));
    art.push(group(&mut doc, vec![window, clipped], true));
    let mask_leaf = instance(&mut doc, "Leaf", Affine::translate((90.0, 60.0)));
    let mask_art = group(&mut doc, vec![mask_leaf], false);
    let mut masked = instance(&mut doc, "Leaf", Affine::translate((90.0, 60.0)));
    masked.extras_mut().mask = Some(Box::new(OpacityMask { art: Arc::new(mask_art), clip: true, invert: false, disabled: false, linked: true }));
    art.push(masked);
    art.push(instance(&mut doc, "Shade", Affine::translate((130.0, 60.0))));
    for n in art {
        doc.insert(Some(art_layer), usize::MAX, n).unwrap();
    }
    let inner = doc.add_layer(Some("Inner"));
    let inner_node = doc.layers.iter().position(|l| l.id == inner).map(|i| doc.layers.remove(i)).unwrap();
    doc.insert(Some(art_layer), usize::MAX, (*inner_node).clone()).unwrap();
    let mirrored = instance(&mut doc, "Pair", Affine::new([-1.0, 0.0, 0.0, 1.0, 260.0, 120.0]));
    doc.insert(Some(inner), usize::MAX, mirrored).unwrap();
    // The sublayer's own transparency, after its last object, an instance.
    let sub = doc.node_mut(inner).unwrap();
    (sub.opacity, sub.blend) = (0.6, BlendMode::Screen);
    doc
}

/// Each symbol of `doc` in its order: its name and registration point, then what its art draws, its
/// objects as [`view`] shows them (points, handles, paint, strokes and instances), its images, and
/// its objects' names, transparency and clipping.
fn symbol_looks(doc: &Document) -> Vec<String> {
    let mut out = vec![];
    for s in doc.symbols.iter() {
        out.push(format!("symbol {} {:?}", s.name, doc.symbol_registration(&s.name)));
        let art = std::slice::from_ref(&s.art);
        paints_of(doc, art, 1, &mut out);
        let mut objects = vec![];
        view(art, 1, &mut objects);
        // Images are compared by `pictures`, gradients by `paints_of`.
        out.extend(objects.into_iter().filter(|l| !l.contains(" Image(")).map(|l| match l.find(" fill(Gradient(") {
            Some(at) => format!("{} fill(gradient)", &l[..at]),
            None => l,
        }));
        pictures(doc, art, &s.name, &mut out);
        looks(art, 1, &mut out);
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

/// A run without a stroke whose stroke's cap, join and miter limit aren't the defaults loses
/// nothing that draws: the save reports no attributes left out.
#[test]
fn an_unstroked_run_with_a_stroke_style_loses_nothing() {
    use vectorcraft_doc::{CharStyle, LineCap, LineJoin, TextObject};
    for miter in [3.0, f64::NAN] {
        let st = CharStyle { stroke_cap: LineCap::Round, stroke_join: LineJoin::Bevel, stroke_miter_limit: miter, ..style(14.0, Color::BLACK) };
        let mut doc = Document::new(200.0, 200.0);
        let layer = doc.layers[0].id;
        let n = text_node(&mut doc, TextObject::point(vectorcraft_geom::Point::new(20.0, 40.0), "Plain", st));
        doc.insert(Some(layer), usize::MAX, n).unwrap();
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert!(out.warnings.is_empty(), "{miter}: {:?}", out.warnings);
    }
}

/// The text document has no Korean Line Break Anywhere: a paragraph with it on and Hangul in it
/// is reported as left out, and comes back off; with it off, or with no Hangul, nothing is
/// reported.
#[test]
fn korean_line_break_anywhere_is_reported_only_for_korean_paragraphs() {
    use vectorcraft_doc::TextObject;
    for (text, on, reported) in [("가나다 라마", true, true), ("가나다 라마", false, false), ("Plain", true, false)] {
        let mut t = TextObject::point(vectorcraft_geom::Point::new(20.0, 40.0), text, style(14.0, Color::BLACK));
        t.para.korean_line_break_anywhere = on;
        let mut doc = Document::new(200.0, 200.0);
        let layer = doc.layers[0].id;
        let n = text_node(&mut doc, t);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert_eq!(!out.warnings.is_empty(), reported, "{text} {on}: {:?}", out.warnings);
        let mut all = vec![];
        paths(&read_back(&doc).layers, &mut all);
        let paras: Vec<bool> = all
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Text(t) => Some(t.para_at(0).korean_line_break_anywhere),
                _ => None,
            })
            .collect();
        assert_eq!(paras, [false], "{text} {on}");
    }
}

/// The text document has no Bunri-Kinshi: a paragraph with it on and an inseparable pair in it is
/// reported as left out, and comes back off; with it off, or with no pair, nothing is reported.
#[test]
fn bunri_kinshi_is_reported_only_for_paragraphs_with_a_pair() {
    use vectorcraft_doc::TextObject;
    for (text, on, reported) in [
        ("Wait——now", true, true),
        ("Wait——now", false, false),
        ("Wait—now", true, false),
        ("Wait……now", true, true),
        ("Wait‥‥now", true, true),
        ("あ〳〵い", true, true),
        ("あ〴〵い", true, true),
        ("あ〵〳い", true, false),
    ] {
        let mut t = TextObject::point(vectorcraft_geom::Point::new(20.0, 40.0), text, style(14.0, Color::BLACK));
        t.para.bunri_kinshi = on;
        let mut doc = Document::new(200.0, 200.0);
        let layer = doc.layers[0].id;
        let n = text_node(&mut doc, t);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert_eq!(!out.warnings.is_empty(), reported, "{text} {on}: {:?}", out.warnings);
        let mut all = vec![];
        paths(&read_back(&doc).layers, &mut all);
        let paras: Vec<bool> = all
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Text(t) => Some(t.para_at(0).bunri_kinshi),
                _ => None,
            })
            .collect();
        assert_eq!(paras, [false], "{text} {on}");
    }
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

/// Standard Vertical Roman Alignment comes back on or off, in vertical type and in horizontal
/// type, and Tate-chu-yoko stays as it was: each run's key 35 is 1 (Standard) when it's on, 2
/// (rotated) when it's off and 3 for Tate-chu-yoko, and the document's defaults and the normal
/// character style carry 1. Each object lays out as it did. A Tate-chu-yoko run with the
/// setting on comes back without it, and that is reported.
#[test]
fn standard_vertical_roman_alignment_comes_back() {
    use vectorcraft_doc::{CharStyle, TextObject, TextRun};
    let plain = style(14.0, Color::BLACK);
    let roman = CharStyle { standard_vertical_roman: true, ..plain.clone() };
    let tcy = CharStyle { tate_chu_yoko: true, ..plain.clone() };
    let mut doc = Document::new(200.0, 200.0);
    let layer = doc.layers[0].id;
    let mut down = TextObject::point(vectorcraft_geom::Point::new(150.0, 20.0), "", plain.clone());
    down.runs = vec![TextRun::new("Up AB ", roman.clone()), TextRun::new("turned CD ", plain.clone()), TextRun::new("12", tcy.clone())];
    down.vertical = true;
    let mut across = TextObject::point(vectorcraft_geom::Point::new(20.0, 40.0), "", plain.clone());
    across.runs = vec![TextRun::new("Marked ", roman.clone()), TextRun::new("plain", plain.clone())];
    for t in [down, across] {
        let n = text_node(&mut doc, t);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_lines(&type_lines(&back), &type_lines(&doc));
    let texts = |d: &Document| -> Vec<TextObject> {
        d.layers[0].children().unwrap().iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some((**t).clone()) } else { None }).collect()
    };
    let flags = |t: &TextObject| t.runs.iter().map(|r| (r.style.standard_vertical_roman, r.style.tate_chu_yoko)).collect::<Vec<_>>();
    let (written, read) = (texts(&doc), texts(&back));
    assert_eq!(read.iter().map(flags).collect::<Vec<_>>(), [vec![(true, false), (false, false), (false, true)], vec![(true, false), (false, false)]]);
    let origins = |t: &TextObject| {
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).glyphs.iter().map(|g| (r3(g.origin.x), r3(g.origin.y))).collect::<Vec<_>>()
    };
    for (w, r) in written.iter().zip(&read) {
        assert_eq!(origins(r), origins(w), "vertical={}", w.vertical);
    }
    let decoded = text_document(&program(&doc));
    // Two runs on, two off and one Tate-chu-yoko; the defaults and the normal style add a 1 each.
    for (key, count) in [("/35 1 ", 4), ("/35 2 ", 2), ("/35 3 ", 1)] {
        assert_eq!(decoded.matches(key).count(), count, "{key}: {decoded}");
    }
    let normal = decoded.find("[Normal Character Style]").unwrap();
    let sheet = &decoded[normal..decoded.find("[Normal Paragraph Style]").unwrap()];
    assert!(sheet.contains("/35 1 "), "{sheet}");
    // Key 35 holds one value: Tate-chu-yoko wins, and the setting it leaves out is reported.
    let mut both = doc.clone();
    let id = both.layers[0].children().unwrap()[0].id;
    if let Some(NodeKind::Text(t)) = both.node_mut(id).map(|n| &mut n.kind) {
        t.runs[2].style.standard_vertical_roman = true;
    }
    assert_eq!(export_ai_editing(&both, &opts()).unwrap().warnings, [writer::TYPE_ATTRIBUTES]);
    assert_eq!(flags(&texts(&read_back(&both))[0])[2], (false, true));
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
    // left-to-right direction set on it, Optical Margin Alignment, a font that isn't
    // installed whose name doesn't come back (a hyphen in its family), a Lab color,
    // small capitals at another size, right-to-left letters aligned to the start of a paragraph
    // without a direction of its own, tracking that isn't a whole number.
    for edit in [
        (|t: &mut vectorcraft_doc::TextObject| t.runs[0].style.no_break = true) as fn(&mut vectorcraft_doc::TextObject),
        |t| t.runs[0].style.tracking = 12.5,
        |t| t.para.direction = Some(vectorcraft_doc::ParaDirection::LeftToRight),
        |t| t.optical_margin = true,
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
        // A version that isn't installed draws in the face the font resolves to, and comes back in it.
        let mut uninstalled = t.clone();
        uninstalled.runs[0].style.font_version = Some("2.0".into());
        assert!(with(uninstalled).is_empty());
        edit(&mut t);
        assert_eq!(with(t), [writer::TYPE_ATTRIBUTES]);
    }
}

/// Installs the copies of each font `copies` lists, the first of each loaded first.
fn install_copies(copies: &[(bool, vectorcraft_text::test_fonts::CopyNames<'_>)]) {
    for (inter, names) in copies {
        let font = vectorcraft_text::test_fonts::named_copy(*inter, *names).unwrap();
        assert_eq!(vectorcraft_text::FontDb::global().add_font(font), 1, "{names:?}");
    }
}

/// A run's text, font family and font version, as [`fonts_saved`] reads them back.
type SavedRun = (String, String, Option<String>);

/// A 200 × 100 document of point type whose runs are `runs` (text, family, version).
fn runs_doc(runs: &[(&str, &str, Option<&str>)]) -> Document {
    use vectorcraft_doc::{CharStyle, TextObject, TextRun};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut t = TextObject::point(vectorcraft_geom::Point::new(10.0, 70.0), "", style(18.0, Color::BLACK));
    t.runs = runs
        .iter()
        .map(|(text, family, version)| {
            let st = CharStyle { font_family: (*family).into(), font_version: version.map(Into::into), ..style(18.0, Color::BLACK) };
            TextRun::new(*text, st)
        })
        .collect();
    let n = text_node(&mut doc, t);
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    doc
}

/// [`runs_doc`] saved and read back: the save's warnings, and the runs read back.
fn fonts_saved(runs: &[(&str, &str, Option<&str>)]) -> (Vec<String>, Vec<SavedRun>) {
    let doc = runs_doc(runs);
    let warnings = export_ai_editing(&doc, &opts()).unwrap().warnings;
    let back = read_back(&doc);
    let NodeKind::Text(t) = &back.layers[0].children().unwrap()[0].kind else { panic!("type comes back as type") };
    (warnings, t.runs.iter().map(|r| (r.text.clone(), r.style.font_family.clone(), r.style.font_version.clone())).collect())
}

/// How many fonts the font set of [`runs_doc`] names, the hidden characters' font aside.
fn fonts_written(runs: &[(&str, &str, Option<&str>)]) -> usize {
    text_document(&program(&runs_doc(runs))).matches("/CoolTypeFont").count() - 1
}

/// Type in a font installed beside a family spelled alike but for case and punctuation
/// (`Sample/Ink`, PostScript name `SampleInk`, beside `SAMPLE INK`), or beside a copy whose
/// PostScript name differs only in case (the type naming its version), is written under its
/// PostScript name and read back in its own family and version, so nothing about its font is
/// reported lost.
#[test]
fn type_in_a_font_beside_a_family_spelled_alike_keeps_its_font() {
    install_copies(&[
        (true, ("SAVETEST INK", "Regular", "Version 1.000", "SAVETESTINK-Regular")),
        (false, ("Savetest/Ink", "Regular", "Version 1.000", "SavetestInk")),
        (true, ("CASESAVE INK", "Regular", "Version 1.000", "CASESAVEINK")),
        (false, ("Casesave Ink", "Regular", "Version 2.000", "CasesaveInk")),
    ]);
    let (warnings, back) = fonts_saved(&[("Kept", "Savetest/Ink", None)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back, [("Kept".to_string(), "Savetest/Ink".to_string(), None)]);
    let (warnings, back) = fonts_saved(&[("Kept", "Casesave Ink", Some("Version 2.000"))]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back, [("Kept".to_string(), "Casesave Ink".to_string(), Some("Version 2.000".to_string()))]);
}

/// Type in a static font's version that the family and style don't resolve to (two installed
/// copies of one PostScript name, family and style) is written with that version (`/5`) and comes
/// back in it, without a note; two runs in two versions are two fonts of the set.
#[test]
fn type_in_another_version_of_a_static_font_keeps_it() {
    install_copies(&[(true, ("Rttest Ink", "Regular", "1.0", "RttestInk-Regular")), (false, ("Rttest Ink", "Regular", "1.1", "RttestInk-Regular"))]);
    let (warnings, back) = fonts_saved(&[("Kept", "Rttest Ink", Some("1.1"))]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back, [("Kept".to_string(), "Rttest Ink".to_string(), Some("1.1".to_string()))]);
    let (warnings, back) = fonts_saved(&[("Old", "Rttest Ink", None), ("New", "Rttest Ink", Some("1.1"))]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back, [("Old".to_string(), "Rttest Ink".to_string(), None), ("New".to_string(), "Rttest Ink".to_string(), Some("1.1".to_string()))]);
    // A run naming the version its family and style resolve to anyway, or one that isn't
    // installed, is in the same face as a run naming none: one font of the set, and no note.
    let (warnings, back) = fonts_saved(&[("Same", "Rttest Ink", Some("1.0")), ("Gone", "Rttest Ink", Some("9.9")), ("Plain", "Rttest Ink", None)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(back.iter().all(|(_, family, version)| family == "Rttest Ink" && version.is_none()), "{back:?}");
    assert_eq!(fonts_written(&[("Same", "Rttest Ink", Some("1.0")), ("Gone", "Rttest Ink", Some("9.9")), ("Plain", "Rttest Ink", None)]), 1);
}

/// Type in the copy of a font that its family names, beside a copy of the same PostScript name in
/// another family that the name alone gives first, is written with its version (`/5`), so it comes
/// back in its own copy and family, without a note.
#[test]
fn type_in_a_copy_its_postscript_name_alone_doesnt_give_keeps_it() {
    install_copies(&[
        (true, ("RvdupsaveInk", "Regular", "2.0", "RvdupsaveInk-Regular")),
        (false, ("Rvdupsave Ink", "Regular", "1.0", "RvdupsaveInk-Regular")),
    ]);
    let (warnings, back) = fonts_saved(&[("Kept", "Rvdupsave Ink", None)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["Rvdupsave Ink"]);
    let (warnings, back) = fonts_saved(&[("Kept", "RvdupsaveInk", None)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["RvdupsaveInk"]);
}

/// A version string longer than the reader takes (256 bytes) isn't one it can pick a copy by: type
/// in such a copy, which its name alone doesn't give back, is reported as losing its font.
#[test]
fn a_version_longer_than_the_reader_takes_is_reported() {
    let long = "9".repeat(300);
    install_copies(&[
        (true, ("Longtest Ink", "Regular", "1.0", "LongtestInk-Regular")),
        (false, ("Longtest Ink", "Regular", &long, "LongtestInk-Regular")),
    ]);
    let (warnings, back) = fonts_saved(&[("Long", "Longtest Ink", Some(&long))]);
    assert_eq!(warnings, [writer::TYPE_ATTRIBUTES]);
    assert_eq!(back, [("Long".to_string(), "Longtest Ink".to_string(), None)]);
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

/// Type on a path with Dynamic Text is written as type at the sizes it is drawn at, with a warning
/// that the setting is not written.
#[test]
fn dynamic_text_is_written_at_its_drawn_sizes() {
    use vectorcraft_doc::{TextKind, TextObject};
    let mut doc = Document::new(600.0, 300.0);
    let layer = doc.layers[0].id;
    let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "Grow", style(12.0, Color::BLACK));
    t.kind = TextKind::OnPath {
        path: shapes::line(vectorcraft_geom::Point::new(50.0, 200.0), vectorcraft_geom::Point::new(550.0, 200.0)),
        start: 0.0,
        end: None,
    };
    t.xf = Affine::IDENTITY;
    t.dynamic_text = true;
    let drawn = 12.0 * vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).fit_scale;
    assert!(drawn > 50.0, "{drawn}");
    let node = text_node(&mut doc, t);
    doc.insert(Some(layer), usize::MAX, node).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::DYNAMIC_TEXT), "{:?}", out.warnings);
    let back = read_back(&doc);
    let texts: Vec<&TextObject> =
        back.layers[0].children().unwrap().iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.as_ref()) } else { None }).collect();
    assert_eq!(texts.len(), 1);
    assert!(!texts[0].dynamic_text);
    let k = texts[0].xf.as_coeffs()[0];
    assert_eq!(r3(texts[0].runs[0].style.size * k), r3(drawn));
    // Type written as outlines (here on an open path whose ends meet) loses the setting too, and
    // says so.
    let mut doc = Document::new(600.0, 300.0);
    let layer = doc.layers[0].id;
    let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "Grow", style(12.0, Color::BLACK));
    let ends_meet = [(50.0, 200.0).into(), (550.0, 200.0).into(), (300.0, 50.0).into(), (50.0, 200.0).into()];
    t.kind = TextKind::OnPath { path: PathData::new(vec![vectorcraft_geom::SubPath::polyline(&ends_meet, false)]), start: 0.0, end: None };
    t.xf = Affine::IDENTITY;
    t.dynamic_text = true;
    let node = text_node(&mut doc, t);
    doc.insert(Some(layer), usize::MAX, node).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::TYPE) && out.warnings.iter().any(|w| w == writer::DYNAMIC_TEXT), "{:?}", out.warnings);
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

#[test]
fn symbols_and_their_instances_are_read_back() {
    let doc = symbols_document();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_lines(&lines(&back), &lines(&doc));
    same_lines(&all_looks(&back), &all_looks(&doc));
    same_lines(&symbol_looks(&back), &symbol_looks(&doc));
    // Each instance draws where it did: the symbols' art and the instances' places agree.
    let boxes = |d: &Document| -> Vec<String> {
        let b = d.symbol_boxes();
        let mut ids = vec![];
        d.walk(|n| {
            if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
                ids.push(n.id);
            }
        });
        ids.iter().map(|id| format!("{:?}", d.bounds_of_with(&[*id], true, &b).map(|r| [r.x0, r.y0, r.x1, r.y1].map(r3)))).collect()
    };
    same_lines(&boxes(&back), &boxes(&doc));
}

#[test]
fn symbols_write_the_same_bytes_again() {
    let doc = symbols_document();
    let first = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(first.program, export_ai_editing(&doc, &opts()).unwrap().program);
    assert_eq!(private_data(&doc, &opts()).unwrap().0, private_data(&doc, &opts()).unwrap().0);
    let again = export_ai_editing(&read_back(&doc), &opts()).unwrap().program;
    same_lines(&shown(&again).lines().map(String::from).collect::<Vec<_>>(), &shown(&first.program).lines().map(String::from).collect::<Vec<_>>());
    assert_eq!(again, first.program);
}

#[test]
fn symbols_are_written_in_their_forms() {
    let p = program_shown(&symbols_document());
    // The definitions in a non-printing block after the patterns, in the order the art first uses
    // them (each after those its art uses), the others after them; and the list after the palette
    // in the panel's order.
    let defs = ["(Leaf)", "(Pair)", "(Shade)", "(Spare)", "(Solo)"].map(|n| p.find(&format!("%AI14_BeginSymbol\n{n}\n0 A\n0 Xw\n0 Ae\n")).unwrap());
    assert!(defs.windows(2).all(|w| w[0] < w[1]), "{p}");
    assert!(p.contains("%AI5_Begin_NonPrinting\nNp\n%AI14_BeginSymbol\n(Leaf)\n"), "{p}");
    let list =
        "%AI5_Begin_NonPrinting\nNp\n%AI24_BeginSymbolList\n(Spare)\n(Pair)\n(Leaf)\n(Shade)\n(Solo)\n%AI24_EndSymbolList\n%AI5_End_NonPrinting--\n";
    let at = p.find(list).unwrap();
    assert!(p.find("%AI5_EndPalette").unwrap() < at && at < p.find("%AI9_BeginDocumentData").unwrap(), "{p}");
    // A group's registration point in its dictionary, before its state block; a path is written in
    // a group of its own.
    assert!(p.contains("U\n%_/ArtDictionary :\n%_1 /Int (AI13PatternRegistrationType) ,\n%_;\n%_\n9 () XW\n%AI10_EndSymbol\n"), "{p}");
    assert!(p.contains("%_/ArtDictionary :\n%_9 /Int (AI13PatternRegistrationType) ,\n"), "{p}");
    assert!(p.contains("%AI14_BeginSymbol\n(Spare)\n0 A\n0 Xw\n0 Ae\nu\n"), "{p}");
    // A plain group of one object in a group of its own.
    assert!(p.contains("%AI14_BeginSymbol\n(Solo)\n0 A\n0 Xw\n0 Ae\nu\nu\n"), "{p}");
    assert!(p.contains("f\nU\n%_/ArtDictionary :\n%_5 /Int (AI13PatternRegistrationType) ,\n%_;\n%_\n9 () XW\n%AI10_EndSymbol\n"), "{p}");
    // An instance on the page: its matrix takes the art to art space (y up from the artboard's top
    // left), then its state block.
    assert!(p.contains("/SymbolInstance :\n(Leaf) /SymbolRef ,\n1 0 0 -1 10 -10 /RTransformMatrix\n;\n9 () XW\n"), "{p}");
    // Its name between the two, and its transparency as a group's.
    assert!(p.contains("/RTransformMatrix\n;\n%_/ArtDictionary :\n%_/XMLUID : (Turned) ; (AI10_ArtUID) ,\n%_;\n%_\n9 () XW\n"), "{p}");
    assert!(p.contains("1 0 0 -1 190 -10 /RTransformMatrix\n;\n1 0.5 0 2 0 Xy\n0 0 Xd\n6 () XW\n"), "{p}");
    // In a symbol's art, about the canvas point of art space's origin (the template box's center
    // is the canvas's).
    let t: Vec<f64> = p.lines().find_map(|l| l.strip_prefix("%AI3_TemplateBox: ")).unwrap().split(' ').map(|v| v.parse().unwrap()).collect();
    let (ox, oy) = (f64::from(CANVAS_SIZE) / 2.0 - t[0], f64::from(CANVAS_SIZE) / 2.0 + t[1]);
    let nested = format!("(Leaf) /SymbolRef ,\n0.5 0 0 -0.5 {} {} /RTransformMatrix\n", ps::num(15.0 - ox), ps::num(oy + 5.0));
    assert!(p.contains(&nested), "{nested}\n{p}");
}

/// Instances that the data can't hold as instances are written as their art, with a warning: a
/// stained one, one of a symbol with 9-slice guides, one scaled almost flat, every one on the
/// layers of a large canvas (whose symbols are still defined), one in a pattern's tile, one with a
/// dynamic symbol's appearance changes and one in art cut to a clipping object that isn't a path.
/// A symbol whose art holds an instance of itself is defined once, that instance as art.
#[test]
fn instances_the_data_cant_hold_are_written_as_their_art() {
    let leaf = |doc: &mut Document| {
        let art = square(doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
        doc.symbols.push(Symbol { dynamic: false, name: "Leaf".into(), art: Arc::new(art) });
    };
    let kinds = |d: &Document| -> Vec<&'static str> {
        d.layers[0].children().unwrap().iter().map(|n| if matches!(n.kind, NodeKind::SymbolInstance { .. }) { "instance" } else { "art" }).collect()
    };
    // Stained.
    let mut doc = Document::new(200.0, 100.0);
    leaf(&mut doc);
    let mut stained = instance(&mut doc, "Leaf", Affine::translate((50.0, 10.0)));
    stained.appearance = Appearance::basic(solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0).into();
    let plain = instance(&mut doc, "Leaf", Affine::translate((80.0, 10.0)));
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, stained).unwrap();
    doc.insert(Some(layer), usize::MAX, plain).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_ART]);
    assert_eq!(kinds(&read_back(&doc)), ["art", "instance"]);
    // 9-slice guides.
    doc.set_symbol_nine_slice("Leaf", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::SYMBOL_ART, writer::SYMBOL_OPTIONS]);
    let back = read_back(&doc);
    assert_eq!((kinds(&back), back.symbols.len()), (vec!["art", "art"], 1));
    // Scaled so flat that its matrix to ten decimals has no area.
    let mut doc = Document::new(200.0, 100.0);
    leaf(&mut doc);
    let flat = instance(&mut doc, "Leaf", Affine::new([1e-11, 0.0, 0.0, 1.0, 50.0, 10.0]));
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, flat).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_ART]);
    assert_eq!(kinds(&read_back(&doc)), ["art"]);
    // A large canvas.
    let mut doc = Document::new(20000.0, 100.0);
    leaf(&mut doc);
    let one = instance(&mut doc, "Leaf", Affine::translate((50.0, 10.0)));
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, one).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_ART]);
    // The symbol is still defined, at its own size: the canvas's scale doesn't touch its art.
    let back = read_back(&doc);
    assert_eq!(kinds(&back), ["art"]);
    assert_eq!(symbol_looks(&back), symbol_looks(&doc));
    // A pattern a symbol's art and the layers both paint with: the layers' fill keeps it, the
    // symbol's art has its tiles.
    let dot = Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 4.0, 4.0), Color::BLACK));
    let mut dots = vectorcraft_doc::PatternDef::new("Dots", vec![dot]);
    dots.tile = Rect::new(0.0, 0.0, 8.0, 8.0);
    doc.patterns.push(dots);
    let fill = || Appearance::basic(Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }, Paint::None, 0.0);
    let patch = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 16.0, 16.0)), fill());
    doc.symbols.push(Symbol { dynamic: false, name: "Patch".into(), art: Arc::new(patch) });
    let filled = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(100.0, 10.0, 132.0, 42.0)), fill());
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, filled).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::SYMBOL_ART, writer::PATTERNS]);
    let back = read_back(&doc);
    let on_layer = back.layers[0].children().unwrap()[1].appearance.items[0].paint().clone();
    assert!(matches!(on_layer, Paint::Pattern { .. }), "{on_layer:?}");
    let patch = &back.symbols.iter().find(|s| s.name == "Patch").unwrap().art;
    assert!(matches!(patch.kind, NodeKind::Group { .. }), "the symbol's art is its tiles: {:?}", patch.kind);
    // A pattern's tile: the tile holds the symbol's art.
    let mut doc = Document::new(200.0, 100.0);
    leaf(&mut doc);
    let tiled = instance(&mut doc, "Leaf", Affine::IDENTITY);
    doc.patterns.push(vectorcraft_doc::PatternDef::new("Tiles", vec![Arc::new(tiled)]));
    let fill = Appearance::basic(Paint::Pattern { pattern: "Tiles".into(), xf: Affine::IDENTITY }, Paint::None, 0.0);
    let filled = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), fill);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, filled).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_ART]);
    let back = read_back(&doc);
    let tile = &back.patterns.iter().find(|p| p.name == "Tiles").unwrap().art;
    assert_eq!(tile.len(), 1);
    assert_eq!(instances(&tile[0]), 0, "{:?}", tile[0].kind);
    let NodeKind::Group { children, .. } = &tile[0].kind else { panic!("{:?}", tile[0].kind) };
    assert_eq!(paints_of_nodes(&back, children), paints_of_nodes(&back, &[Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF))]));
    // A symbol holding an instance of itself: defined once, that instance as its art.
    let mut doc = Document::new(200.0, 100.0);
    let looped_art = {
        let me = instance(&mut doc, "Loop", Affine::translate((20.0, 0.0)));
        let dot = square(&mut doc, Rect::new(0.0, 0.0, 5.0, 5.0), LEAF);
        group(&mut doc, vec![dot, me], false)
    };
    doc.symbols.push(Symbol { dynamic: false, name: "Loop".into(), art: Arc::new(looped_art) });
    let looped = instance(&mut doc, "Loop", Affine::translate((100.0, 10.0)));
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, looped).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_ART]);
    let p = program(&doc);
    assert_eq!(p.matches("%AI14_BeginSymbol\n(Loop)\n").count(), 1, "{p}");
    assert_eq!(kinds(&read_back(&doc)), ["instance"]);
    // Appearance changes of a dynamic symbol's instance.
    let mut doc = Document::new(200.0, 100.0);
    leaf(&mut doc);
    doc.symbols[0].dynamic = true;
    let mut changed = instance(&mut doc, "Leaf", Affine::translate((50.0, 10.0)));
    changed.extras_mut().symbol_overrides =
        vec![vectorcraft_doc::SymbolOverride { fill: Some(solid(Color::rgb(0.0, 1.0, 0.0))), ..Default::default() }];
    let plain = instance(&mut doc, "Leaf", Affine::translate((80.0, 10.0)));
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, changed).unwrap();
    doc.insert(Some(layer), usize::MAX, plain).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_ART, writer::SYMBOL_OPTIONS]);
    let back = read_back(&doc);
    assert_eq!(kinds(&back), ["art", "instance"]);
    let green = back.layers[0].children().unwrap()[0].clone();
    let drawn = paints_of_nodes(&back, &[green]);
    assert!(drawn.iter().any(|l| l.contains("rgb 0.000 1.000 0.000")), "the change is drawn: {drawn:?}");
    // Art cut to a clipping object that isn't a path.
    let mut doc = Document::new(200.0, 100.0);
    leaf(&mut doc);
    let pixels = png_of(image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255]))));
    let frame = image_node(&mut doc, ImageBlob::new("image/png", pixels), (2, 2), Affine::new([20.0, 0.0, 0.0, 20.0, 40.0, 0.0]), None);
    let cut = instance(&mut doc, "Leaf", Affine::translate((45.0, 5.0)));
    let clipped = group(&mut doc, vec![frame, cut], true);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, clipped).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::SYMBOL_ART), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.layers.iter().map(|l| instances(l)).sum::<usize>(), 0);
    assert_eq!(back.symbols.len(), 1);
}

/// Symbols whose art paints with a pattern, directly, through a nested symbol, in an opacity mask
/// and through an inline graphic: their definitions hold the pattern fills as tiles cut to the shape (the reader keeps no
/// symbol whose art paints with a pattern), and their instances are written as their art, whose
/// pattern fills keep the document's placement as the canvas draws them, each with a warning. An
/// instance of a symbol without pattern paint stays an instance.
#[test]
fn symbols_with_pattern_fills_are_defined_with_tiles_and_placed_as_art() {
    let mut doc = Document::new(200.0, 100.0);
    // Tiles that cover the shape, so its tiles cut to it draw its box.
    let dot = Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 8.0, 8.0), Color::BLACK));
    let mut dots = vectorcraft_doc::PatternDef::new("Dots", vec![dot]);
    dots.tile = Rect::new(0.0, 0.0, 8.0, 8.0);
    doc.patterns.push(dots);
    let fill = || Appearance::basic(Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }, Paint::None, 0.0);
    let patch = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 16.0, 16.0)), fill());
    doc.symbols.push(Symbol { dynamic: false, name: "Patch".into(), art: Arc::new(patch) });
    let inner = instance(&mut doc, "Patch", Affine::translate((20.0, 0.0)));
    let edge = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
    let outer = group(&mut doc, vec![edge, inner], false);
    doc.symbols.push(Symbol { dynamic: false, name: "Outer".into(), art: Arc::new(outer) });
    let mask_art = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 12.0, 12.0)), fill());
    let mut masked = square(&mut doc, Rect::new(0.0, 0.0, 12.0, 12.0), LEAF);
    masked.extras_mut().mask = Some(Box::new(OpacityMask { art: Arc::new(mask_art), clip: true, invert: false, disabled: false, linked: true }));
    doc.symbols.push(Symbol { dynamic: false, name: "Masked".into(), art: Arc::new(masked) });
    let plain = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
    doc.symbols.push(Symbol { dynamic: false, name: "Plain".into(), art: Arc::new(plain) });
    // Type holding `Patch` as an inline graphic.
    let style = vectorcraft_doc::CharStyle { size: 18.0, fill: solid(Color::BLACK), ..vectorcraft_doc::CharStyle::default() };
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(0.0, 20.0), "A", style.clone());
    t.runs.push(vectorcraft_doc::TextRun::inline(vectorcraft_doc::InlineArt::new("Patch"), style));
    let typed = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    doc.symbols.push(Symbol { dynamic: false, name: "Typed".into(), art: Arc::new(typed) });
    let layer = doc.layers[0].id;
    // Two instances of `Patch` a part of a tile apart.
    for (name, x) in [("Patch", 10.0), ("Patch", 33.0), ("Outer", 60.0), ("Masked", 120.0), ("Plain", 140.0), ("Typed", 10.0)] {
        let one = instance(&mut doc, name, Affine::translate((x, 10.0)));
        doc.insert(Some(layer), usize::MAX, one).unwrap();
    }
    let filled = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(150.0, 10.0, 182.0, 42.0)), fill());
    doc.insert(Some(layer), usize::MAX, filled).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::SYMBOL_ART) && out.warnings.iter().any(|w| w == writer::PATTERNS), "{:?}", out.warnings);
    let back = read_back(&doc);
    let names: Vec<&str> = back.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Patch", "Outer", "Masked", "Plain", "Typed"]);
    let patch = &back.symbols[0].art;
    assert!(matches!(patch.kind, NodeKind::Group { .. }), "the symbol's art is its tiles: {:?}", patch.kind);
    assert_eq!(instances(&back.symbols[1].art), 0, "the nested instance is its art");
    let kids = back.layers[0].children().unwrap();
    let kinds: Vec<bool> = kids.iter().map(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })).collect();
    assert_eq!(kinds, [false, false, false, false, true, false, false]);
    // The pattern fills of both `Patch` instances and of the layers' path keep one placement.
    let placements = |n: &Node| {
        let mut out = vec![];
        vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| {
            for it in m.appearance.items.iter() {
                if let Paint::Pattern { xf, .. } = it.paint() {
                    out.push(xf.as_coeffs().map(r3));
                }
            }
        });
        out
    };
    let layer_fill = placements(&kids[6]);
    assert_eq!(layer_fill.len(), 1);
    assert_eq!(placements(&kids[0]), layer_fill);
    assert_eq!(placements(&kids[1]), layer_fill);
    // Each instance draws where it did.
    let boxes = |d: &Document| -> Vec<String> {
        let b = d.symbol_boxes();
        let ids: Vec<_> = d.layers[0].children().unwrap().iter().take(5).map(|n| n.id).collect();
        ids.iter().map(|id| format!("{:?}", d.bounds_of_with(&[*id], true, &b).map(|r| [r.x0, r.y0, r.x1, r.y1].map(r3)))).collect()
    };
    same_lines(&boxes(&back), &boxes(&doc));
}

/// A symbol whose art paints with a pattern only through a brushed stroke, whose brush's art is
/// filled with a pattern: the definition holds the tiles, and its instance is written as its art.
#[test]
fn symbols_painting_patterns_through_brush_art_are_placed_as_art() {
    let mut doc = Document::new(200.0, 100.0);
    let dot = Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 8.0, 8.0), Color::BLACK));
    let mut dots = vectorcraft_doc::PatternDef::new("Dots", vec![dot]);
    dots.tile = Rect::new(0.0, 0.0, 8.0, 8.0);
    doc.patterns.push(dots);
    let fill = Appearance::basic(Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY }, Paint::None, 0.0);
    let art = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 8.0)), fill);
    let brush = vectorcraft_brush::Brush {
        name: "Dotted".into(),
        kind: vectorcraft_brush::BrushKind::Art(vectorcraft_brush::ArtBrush {
            art,
            colorization: vectorcraft_brush::Colorization::None,
            ..Default::default()
        }),
    };
    vectorcraft_brush::store(&mut doc, &[brush]);
    let mut line =
        Node::path(doc.alloc_id(), shapes::line((0.0, 0.0).into(), (60.0, 0.0).into()), Appearance::basic(Paint::None, solid(Color::BLACK), 4.0));
    line.appearance.stroke_mut().unwrap().brush = Some("Dotted".into());
    doc.symbols.push(Symbol { dynamic: false, name: "Brushed".into(), art: Arc::new(line) });
    let layer = doc.layers[0].id;
    let one = instance(&mut doc, "Brushed", Affine::translate((10.0, 40.0)));
    doc.insert(Some(layer), usize::MAX, one).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::SYMBOL_ART), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Brushed"]);
    assert_eq!(instances(&back.layers[0]), 0, "the instance is its art");
}

/// A symbol first defined from the drawn look of a group with a style, through a member's scatter
/// brush, still warns that its art's own effects are left out, and so does a later instance of it.
#[test]
fn a_symbol_defined_inside_a_drawn_look_warns_of_its_own_effects() {
    let mut doc = Document::new(200.0, 100.0);
    let mut shadowed = square(&mut doc, Rect::new(0.0, 0.0, 6.0, 6.0), Color::BLACK);
    shadowed.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::basic(black_fill(), Paint::None, 0.0) }.into();
    doc.symbols.push(Symbol { dynamic: false, name: "Shadowed".into(), art: Arc::new(shadowed) });
    let dot = square(&mut doc, Rect::new(0.0, 0.0, 6.0, 6.0), Color::BLACK);
    let placed = instance(&mut doc, "Shadowed", Affine::translate((8.0, 0.0)));
    let art = group(&mut doc, vec![dot, placed], false);
    let brush = vectorcraft_brush::Brush {
        name: "Scattered".into(),
        kind: vectorcraft_brush::BrushKind::Scatter(vectorcraft_brush::Scatter {
            art,
            size: (100.0, 100.0),
            spacing: (100.0, 100.0),
            scatter: (0.0, 0.0),
            rotation: (0.0, 0.0),
            colorization: vectorcraft_brush::Colorization::None,
            ..Default::default()
        }),
    };
    vectorcraft_brush::store(&mut doc, &[brush]);
    let mut line =
        Node::path(doc.alloc_id(), shapes::line((10.0, 50.0).into(), (90.0, 50.0).into()), Appearance::basic(Paint::None, solid(Color::BLACK), 1.0));
    line.appearance.stroke_mut().unwrap().brush = Some("Scattered".into());
    let edge = square(&mut doc, Rect::new(10.0, 60.0, 30.0, 80.0), LEAF);
    let mut styled = group(&mut doc, vec![line, edge], false);
    styled.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::default() }.into();
    put(&mut doc, named(styled, "Styled"));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    let p = shown(&out.program);
    assert!(p.contains("%AI14_BeginSymbol") && p.contains("1 (Anon 1) XW"), "{p}");
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);

    let later = instance(&mut doc, "Shadowed", Affine::translate((150.0, 20.0)));
    put(&mut doc, later);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
}

/// A symbol whose stroke is written as its outline (an outside stroke): a scaled instance, a scaled
/// instance of a symbol holding it and a turned and mirrored instance are written as their art,
/// which keeps the stroke the canvas draws; a moved instance stays an instance.
#[test]
fn scaled_instances_of_symbols_with_outlined_strokes_are_written_as_their_art() {
    let mut doc = Document::new(200.0, 100.0);
    let mut look = Appearance::basic(Paint::None, solid(Color::BLACK), 4.0);
    look.stroke_mut().unwrap().align = vectorcraft_doc::StrokeAlign::Outside;
    let framed = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), look);
    doc.symbols.push(Symbol { dynamic: false, name: "Framed".into(), art: Arc::new(framed) });
    let inner = instance(&mut doc, "Framed", Affine::IDENTITY);
    let edge = square(&mut doc, Rect::new(20.0, 0.0, 30.0, 10.0), LEAF);
    let outer = group(&mut doc, vec![inner, edge], false);
    doc.symbols.push(Symbol { dynamic: false, name: "Outer".into(), art: Arc::new(outer) });
    let layer = doc.layers[0].id;
    let turned = Affine::translate((150.0, 20.0)) * Affine::rotate(0.5) * Affine::new([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    for (name, xf) in [
        ("Framed", Affine::translate((40.0, 40.0)) * Affine::scale(2.0)),
        ("Outer", Affine::translate((80.0, 10.0)) * Affine::scale(2.0)),
        ("Framed", turned),
        ("Framed", Affine::translate((170.0, 60.0))),
    ] {
        let one = instance(&mut doc, name, xf);
        doc.insert(Some(layer), usize::MAX, one).unwrap();
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::SYMBOL_ART), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Framed", "Outer"]);
    let kids = back.layers[0].children().unwrap();
    let kinds: Vec<bool> = kids.iter().map(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })).collect();
    assert_eq!(kinds, [false, false, false, true]);
    // Each draws where it did, with the stroke the canvas draws: 4 pt wide whatever the scale.
    let boxes = |d: &Document| -> Vec<String> {
        let b = d.symbol_boxes();
        let ids: Vec<_> = d.layers[0].children().unwrap().iter().map(|n| n.id).collect();
        ids.iter().map(|id| format!("{:?}", d.bounds_of_with(&[*id], true, &b).map(|r| [r.x0, r.y0, r.x1, r.y1].map(r3)))).collect()
    };
    let (got, want) = (boxes(&back), boxes(&doc));
    same_lines(&[got[0].clone(), got[2].clone(), got[3].clone()], &[want[0].clone(), want[2].clone(), want[3].clone()]);
    // `Outer` at twice its size from (80, 10): the frame's square is 20 pt from 80 with its 4 pt
    // stroke outside, and the plain square is 40 to 60 pt in from it.
    assert_eq!(got[1], format!("{:?}", Some([76.0, 6.0, 140.0, 34.0].map(r3))));
}

/// Scaled instances of a symbol holding a rounded rectangle with live corners, of one clipped by
/// it, of one holding type, of one holding a blend and of one holding an instance written as its
/// art (of a symbol with 9-slice guides), and instances that scale their symbol more than 100
/// times (tiny, thin and stroked tiny objects, and a 10 pt square), are written as their art;
/// instances that scale a 10 pt square a thousandth, scale an ellipse unevenly or only move stay
/// instances.
#[test]
fn scaled_instances_of_symbols_drawn_only_at_their_own_size_are_written_as_their_art() {
    let mut doc = Document::new(200.0, 100.0);
    let live = vectorcraft_doc::LiveShape::Rectangle {
        w: 100.0,
        h: 100.0,
        radii: [10.0; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: Affine::IDENTITY,
    };
    let mut rounded = Node::new(
        doc.alloc_id(),
        NodeKind::Path { path: live.to_path(), rule: FillRule::NonZero, live: Some(Box::new(live)), clipping: false, guide: false },
    );
    rounded.appearance = Appearance::basic(solid(LEAF), Paint::None, 0.0).into();
    // The same rounded rectangle clipping a square.
    let mut frame = rounded.clone();
    frame.id = doc.alloc_id();
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 100.0, 100.0), LEAF);
    let clipped = group(&mut doc, vec![frame, inside], true);
    doc.symbols.push(Symbol { dynamic: false, name: "Rounded".into(), art: Arc::new(rounded) });
    doc.symbols.push(Symbol { dynamic: false, name: "Clipped".into(), art: Arc::new(clipped) });
    // A tiny square with a 1 pt stroke.
    let mut dot = square(&mut doc, Rect::new(0.0, 0.0, 0.00001, 0.00001), LEAF);
    dot.appearance = Appearance::basic(solid(LEAF), solid(Color::BLACK), 1.0).into();
    doc.symbols.push(Symbol { dynamic: false, name: "Dot".into(), art: Arc::new(dot) });
    // A blend of two squares 20 pt apart with a step every 10 pt.
    let keys =
        vec![Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 2.0, 2.0), LEAF)), Arc::new(square(&mut doc, Rect::new(20.0, 0.0, 22.0, 2.0), LEAF))];
    let spec = vectorcraft_doc::BlendSpec { spacing: vectorcraft_doc::BlendSpacing::Distance(10.0), ..Default::default() };
    let blend = Node::new(doc.alloc_id(), NodeKind::Blend { children: keys, spec: Box::new(spec) });
    doc.symbols.push(Symbol { dynamic: false, name: "Steps".into(), art: Arc::new(blend) });
    // A symbol holding an instance of one with 9-slice guides.
    let frame = Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 100.0, 100.0), LEAF));
    let corner = Arc::new(square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK));
    let sliced = Node::group(doc.alloc_id(), vec![frame, corner]);
    doc.symbols.push(Symbol { dynamic: false, name: "Sliced".into(), art: Arc::new(sliced) });
    doc.set_symbol_nine_slice("Sliced", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let inner = instance(&mut doc, "Sliced", Affine::IDENTITY);
    let holder = group(&mut doc, vec![inner], false);
    doc.symbols.push(Symbol { dynamic: false, name: "Holder".into(), art: Arc::new(holder) });
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(0.0, 10.0), "Hi", style(10.0, Color::BLACK));
    let typed = text_node(&mut doc, t);
    doc.symbols.push(Symbol { dynamic: false, name: "Typed".into(), art: Arc::new(typed) });
    let tiny = square(&mut doc, Rect::new(0.0, 0.0, 0.00001, 0.00001), LEAF);
    doc.symbols.push(Symbol { dynamic: false, name: "Tiny".into(), art: Arc::new(tiny) });
    let thin = square(&mut doc, Rect::new(0.0, 0.0, 0.00001, 1.0), LEAF);
    doc.symbols.push(Symbol { dynamic: false, name: "Thin".into(), art: Arc::new(thin) });
    let plain = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
    doc.symbols.push(Symbol { dynamic: false, name: "Plain".into(), art: Arc::new(plain) });
    let oval = vectorcraft_doc::LiveShape::Ellipse { w: 10.0, h: 6.0, pie: (0.0, 360.0), xf: Affine::IDENTITY };
    let mut ellipse = Node::new(
        doc.alloc_id(),
        NodeKind::Path { path: oval.to_path(), rule: FillRule::NonZero, live: Some(Box::new(oval)), clipping: false, guide: false },
    );
    ellipse.appearance = Appearance::basic(solid(LEAF), Paint::None, 0.0).into();
    doc.symbols.push(Symbol { dynamic: false, name: "Oval".into(), art: Arc::new(ellipse) });
    let layer = doc.layers[0].id;
    for (name, xf) in [
        ("Rounded", Affine::new([0.3, 0.0, 0.0, 0.1, 10.0, 10.0])),
        ("Typed", Affine::translate((50.0, 50.0)) * Affine::scale(3.0)),
        ("Tiny", Affine::translate((100.0, 50.0)) * Affine::scale(1e6)),
        ("Plain", Affine::translate((150.0, 50.0)) * Affine::scale(1e-3)),
        ("Plain", Affine::translate((150.0, 60.0)) * Affine::scale(1e3)),
        ("Oval", Affine::translate((170.0, 20.0)) * Affine::new([3.0, 0.0, 0.0, 1.0, 0.0, 0.0])),
        ("Thin", Affine::translate((100.0, 70.0)) * Affine::new([1e6, 0.0, 0.0, 10.0, 0.0, 0.0])),
        ("Clipped", Affine::new([0.3, 0.0, 0.0, 0.1, 10.0, 30.0])),
        ("Dot", Affine::translate((120.0, 70.0)) * Affine::scale(1e6)),
        ("Steps", Affine::translate((10.0, 80.0)) * Affine::scale(5.0)),
        ("Holder", Affine::scale(2.0)),
        ("Holder", Affine::translate((5.0, 5.0))),
    ] {
        let one = instance(&mut doc, name, xf);
        doc.insert(Some(layer), usize::MAX, one).unwrap();
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::SYMBOL_ART), "{:?}", out.warnings);
    let back = read_back(&doc);
    let kids = back.layers[0].children().unwrap();
    let kinds: Vec<bool> = kids.iter().map(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })).collect();
    assert_eq!(kinds, [false, false, false, true, false, true, false, false, false, false, false, true]);
    let b = kids[6].visual_bounds().map(|r| [r.x0, r.y0, r.x1, r.y1].map(r3));
    assert_eq!(b, Some([100.0, 70.0, 110.0, 80.0].map(r3)));
    // The tiny square comes back 10 pt across, where the instance put it.
    let b = kids[2].visual_bounds().map(|r| [r.x0, r.y0, r.x1, r.y1].map(r3));
    assert_eq!(b, Some([100.0, 50.0, 110.0, 60.0].map(r3)));
}

/// A symbol whose art is a hidden path with two fills: the canvas draws a symbol's art even when
/// its root is hidden, so the definition's root is written shown and the instance still draws.
#[test]
fn a_symbol_whose_root_is_hidden_still_draws() {
    let mut doc = Document::new(200.0, 100.0);
    let mut look = Appearance::basic(solid(LEAF), Paint::None, 0.0);
    look.items.push(AppearanceItem::Fill(FillLayer::new(solid(Color::BLACK))));
    let mut art = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), look);
    art.visible = false;
    doc.symbols.push(Symbol { dynamic: false, name: "Hidden".into(), art: Arc::new(art) });
    let layer = doc.layers[0].id;
    let one = instance(&mut doc, "Hidden", Affine::translate((20.0, 20.0)));
    doc.insert(Some(layer), usize::MAX, one).unwrap();
    let back = read_back(&doc);
    let kids = back.layers[0].children().unwrap();
    assert!(matches!(kids[0].kind, NodeKind::SymbolInstance { .. }));
    let mut shown = 0;
    vectorcraft_doc::swatches::walk_with_masks(&back.symbols[0].art, &mut |m| {
        shown += usize::from(m.visible && matches!(m.kind, NodeKind::Path { .. }))
    });
    assert!(back.symbols[0].art.visible && shown > 0, "{:?}", back.symbols[0].art);
}

/// The number of symbol instances in `n` and its subtree.
fn instances(n: &Node) -> usize {
    let mut count = 0;
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| count += usize::from(matches!(m.kind, NodeKind::SymbolInstance { .. })));
    count
}

/// What `nodes` draw ([`paints_of`]).
fn paints_of_nodes(doc: &Document, nodes: &[Arc<Node>]) -> Vec<String> {
    let mut out = vec![];
    paints_of(doc, nodes, 1, &mut out);
    out
}

/// An instance with isolation, knockout or knockout shape of its own is written as its art, which
/// keeps them as a group's; a plain one beside it stays an instance.
#[test]
fn instances_with_a_groups_transparency_are_written_as_their_art() {
    let mut doc = Document::new(200.0, 100.0);
    let art = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
    doc.symbols.push(Symbol { dynamic: false, name: "Leaf".into(), art: Arc::new(art) });
    let mut shape = instance(&mut doc, "Leaf", Affine::translate((10.0, 10.0)));
    shape.knockout_shape = true;
    let mut isolated = instance(&mut doc, "Leaf", Affine::translate((30.0, 10.0)));
    isolated.isolate = true;
    let mut knocks = instance(&mut doc, "Leaf", Affine::translate((50.0, 10.0)));
    knocks.knockout = Knockout::On;
    let plain = instance(&mut doc, "Leaf", Affine::translate((70.0, 10.0)));
    let mut outer = group(&mut doc, vec![shape, isolated, knocks, plain], false);
    outer.knockout = Knockout::On;
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, outer).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_ART]);
    let back = read_back(&doc);
    let kids = back.layers[0].children().unwrap()[0].children().unwrap().clone();
    let states: Vec<_> = kids.iter().map(|n| (matches!(n.kind, NodeKind::SymbolInstance { .. }), n.knockout_shape, n.isolate, n.knockout)).collect();
    assert_eq!(
        states,
        [
            (false, true, false, Knockout::Neutral),
            (false, false, true, Knockout::Neutral),
            (false, false, false, Knockout::On),
            (true, false, false, Knockout::Neutral)
        ]
    );
    // Each group holds the symbol's art where the instance placed it.
    for (i, k) in kids[..3].iter().enumerate() {
        let b = k.geometric_bounds().unwrap();
        assert_eq!([b.x0, b.y0, b.x1, b.y1].map(r3), [10.0 + 20.0 * i as f64, 10.0, 20.0 + 20.0 * i as f64, 20.0].map(r3));
    }
}

/// A symbol's definition is the same wherever its first instance lies: one first met deep in
/// opacity masks keeps its own masks and instances.
#[test]
fn a_symbol_first_used_deep_in_masks_is_defined_whole() {
    let mut doc = Document::new(200.0, 100.0);
    let leaf = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
    doc.symbols.push(Symbol { dynamic: false, name: "Leaf".into(), art: Arc::new(leaf) });
    // `Deep`: a square with an opacity mask, and an instance of `Leaf`.
    let mask_art = square(&mut doc, Rect::new(0.0, 0.0, 5.0, 5.0), Color::WHITE);
    let mut veiled = square(&mut doc, Rect::new(0.0, 0.0, 20.0, 20.0), Color::BLACK);
    veiled.extras_mut().mask = Some(Box::new(OpacityMask { art: Arc::new(mask_art), clip: true, invert: false, disabled: false, linked: true }));
    let inner = instance(&mut doc, "Leaf", Affine::translate((5.0, 5.0)));
    let deep = group(&mut doc, vec![veiled, inner], false);
    doc.symbols.push(Symbol { dynamic: false, name: "Deep".into(), art: Arc::new(deep) });
    // Its only instance, in the art of an opacity mask seven masks deep.
    let mut art = instance(&mut doc, "Deep", Affine::translate((50.0, 20.0)));
    for _ in 0..7 {
        let mut holder = square(&mut doc, Rect::new(0.0, 0.0, 100.0, 100.0), Color::gray(0.5));
        holder.extras_mut().mask = Some(Box::new(OpacityMask { art: Arc::new(art), clip: false, invert: false, disabled: false, linked: true }));
        art = holder;
    }
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, art).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::DEEP_MASKS || w == writer::SYMBOL_ART), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_lines(&symbol_looks(&back), &symbol_looks(&doc));
    let deep = &back.symbols.iter().find(|s| s.name == "Deep").unwrap().art;
    assert_eq!(instances(deep), 1);
    let mut masks = 0;
    deep.walk(&mut |n| masks += usize::from(n.extras().mask.is_some()));
    assert_eq!(masks, 1);
}

/// Symbols past the 4,096 the reader keeps are left out, their instances written as their art.
#[test]
fn symbols_past_the_readers_count_are_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    for i in 0..=crate::import::MAX_SYMBOLS {
        let art = square(&mut doc, Rect::new(0.0, 0.0, 4.0, 4.0), Color::gray(0.5));
        doc.symbols.push(Symbol { dynamic: false, name: format!("S{i}"), art: Arc::new(art) });
    }
    // An instance of each, in the panel's order: the last one's symbol is past the count.
    let layer = doc.layers[0].id;
    for i in 0..=crate::import::MAX_SYMBOLS {
        let one = instance(&mut doc, &format!("S{i}"), Affine::translate((10.0, 10.0)));
        doc.insert(Some(layer), usize::MAX, one).unwrap();
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::SYMBOL_COUNT, writer::SYMBOL_ART]);
    let back = read_back(&doc);
    assert_eq!(back.symbols.len(), crate::import::MAX_SYMBOLS);
    assert!(back.symbols.iter().all(|s| s.name != format!("S{}", crate::import::MAX_SYMBOLS)));
    let live = back.layers[0].children().unwrap().iter().map(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })).collect::<Vec<_>>();
    assert_eq!(live.len(), crate::import::MAX_SYMBOLS + 1);
    assert!(live[..crate::import::MAX_SYMBOLS].iter().all(|l| *l) && !live[crate::import::MAX_SYMBOLS]);
}

/// With one place left under the reader's count, a symbol whose type holds an inline graphic with
/// an instance of another symbol (found only as its art is written) takes the last place, and that
/// instance is written as its art: no more definitions are written than the reader keeps.
#[test]
fn a_symbol_found_while_its_holder_is_written_keeps_within_the_readers_count() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    for i in 0..crate::import::MAX_SYMBOLS - 1 {
        let art = square(&mut doc, Rect::new(0.0, 0.0, 4.0, 4.0), Color::gray(0.5));
        doc.symbols.push(Symbol { dynamic: false, name: format!("S{i}"), art: Arc::new(art) });
        let one = instance(&mut doc, &format!("S{i}"), Affine::translate((10.0, 10.0)));
        doc.insert(Some(layer), usize::MAX, one).unwrap();
    }
    let style = vectorcraft_doc::CharStyle { size: 18.0, fill: solid(Color::BLACK), ..vectorcraft_doc::CharStyle::default() };
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(0.0, 20.0), "A", style.clone());
    t.runs.push(vectorcraft_doc::TextRun::inline(vectorcraft_doc::InlineArt::new("Inner"), style));
    let typed = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    doc.symbols.push(Symbol { dynamic: false, name: "Typed".into(), art: Arc::new(typed) });
    // The inline graphic's art holds an instance of a symbol of its own.
    let deeper = square(&mut doc, Rect::new(0.0, 0.0, 4.0, 4.0), LEAF);
    doc.symbols.push(Symbol { dynamic: false, name: "Deeper".into(), art: Arc::new(deeper) });
    let nested = instance(&mut doc, "Deeper", Affine::translate((1.0, 1.0)));
    let edge = square(&mut doc, Rect::new(0.0, 0.0, 2.0, 2.0), LEAF);
    let inner = group(&mut doc, vec![edge, nested], false);
    doc.symbols.push(Symbol { dynamic: false, name: "Inner".into(), art: Arc::new(inner) });
    let one = instance(&mut doc, "Typed", Affine::translate((50.0, 50.0)));
    doc.insert(Some(layer), usize::MAX, one).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    let program = String::from_utf8_lossy(&out.program);
    assert_eq!(program.matches("%AI14_BeginSymbol").count(), crate::import::MAX_SYMBOLS);
    let back = read_back(&doc);
    assert_eq!(back.symbols.len(), crate::import::MAX_SYMBOLS);
    assert!(back.symbols.iter().any(|s| s.name == "Typed"));
    assert!(matches!(back.layers[0].children().unwrap().last().unwrap().kind, NodeKind::SymbolInstance { .. }));
}

/// A linked image in a symbol's art is embedded; a dynamic symbol and a symbol's export type are
/// reported left out.
#[test]
fn symbol_options_and_linked_images_in_symbols_are_reported() {
    let mut doc = Document::new(200.0, 100.0);
    let pixels = png_of(image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255]))));
    let link = vectorcraft_doc::LinkInfo { path: "/tmp/blue.png".into(), ..Default::default() };
    let picture = image_node(&mut doc, ImageBlob::new("image/png", pixels), (2, 2), Affine::scale(5.0), Some(link));
    doc.symbols.push(Symbol { dynamic: true, name: "Picture".into(), art: Arc::new(picture) });
    let one = instance(&mut doc, "Picture", Affine::translate((50.0, 10.0)));
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, one).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(out.warnings, [writer::SYMBOL_OPTIONS, writer::LINKS_EMBEDDED]);
    let p = program_shown(&doc);
    assert!(p.contains("%AI5_BeginRaster") && !p.contains("%AI5_BeginPlace"), "{p}");
    let back = read_back(&doc);
    let NodeKind::Image(im) = &back.symbols[0].art.kind else { panic!("the symbol's image") };
    assert!(im.link.is_none());
    doc.symbols[0].dynamic = false;
    doc.set_symbol_export_type("Picture", vectorcraft_doc::SymbolExportType::Graphic);
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_OPTIONS, writer::LINKS_EMBEDDED]);
}

/// Each of the nine registration points is written 1 to 9 by its place on the grid, row by row
/// from the top left, and read back.
#[test]
fn every_registration_point_is_written_by_its_place() {
    for (point, written, read) in (0..9).map(|r| (r, r + 1, r)) {
        let mut doc = Document::new(200.0, 100.0);
        let art = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
        doc.symbols.push(Symbol { dynamic: false, name: "Mark".into(), art: Arc::new(art) });
        doc.set_symbol_registration("Mark", vectorcraft_doc::SymbolRegistration(point));
        let p = program_shown(&doc);
        assert!(p.contains(&format!("%_{written} /Int (AI13PatternRegistrationType) ,\n")), "{point}: {p}");
        assert_eq!(read_back(&doc).symbol_registration("Mark"), vectorcraft_doc::SymbolRegistration(read), "{point}");
    }
}

/// Symbols whose art is a group with a state of its own (a name, transparency, an opacity mask, a
/// clipping group) come back with it; the registration point shares the group's dictionary.
#[test]
fn symbols_whose_art_is_a_group_with_a_state_come_back() {
    let mut doc = one_swatch_document(200.0, 100.0);
    let pair = |doc: &mut Document| {
        let a = square(doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
        let b = square(doc, Rect::new(5.0, 5.0, 15.0, 15.0), Color::rgb(1.0, 0.0, 0.0));
        vec![a, b]
    };
    let kids = pair(&mut doc);
    let labeled = named(group(&mut doc, kids, false), "Label");
    let kids = pair(&mut doc);
    let faded = see_through(group(&mut doc, kids, false), 0.5, BlendMode::Multiply);
    let kids = pair(&mut doc);
    let mut masked = group(&mut doc, kids, false);
    let mask_art = square(&mut doc, Rect::new(0.0, 0.0, 8.0, 8.0), Color::WHITE);
    masked.extras_mut().mask = Some(Box::new(OpacityMask { art: Arc::new(mask_art), clip: true, invert: true, disabled: false, linked: true }));
    let edge = clipping(&mut doc, shapes::ellipse(Rect::new(0.0, 0.0, 12.0, 12.0)));
    let inside = square(&mut doc, Rect::new(0.0, 0.0, 20.0, 20.0), Color::rgb(0.0, 0.0, 1.0));
    let window = group(&mut doc, vec![edge, inside], true);
    let lone = square(&mut doc, Rect::new(0.0, 0.0, 6.0, 6.0), Color::gray(0.3));
    let lone_named = named(group(&mut doc, vec![lone], false), "Lone");
    for (name, art) in [("Labeled", labeled), ("Faded", faded), ("Masked", masked), ("Window", window), ("Lone", lone_named)] {
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
        let one = instance(&mut doc, name, Affine::translate((20.0, 20.0)));
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), usize::MAX, one).unwrap();
    }
    doc.set_symbol_registration("Labeled", vectorcraft_doc::SymbolRegistration(2));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_lines(&symbol_looks(&back), &symbol_looks(&doc));
    same_lines(&all_looks(&back), &all_looks(&doc));
    assert_eq!(export_ai_editing(&back, &opts()).unwrap().program, out.program);
    let p = program_shown(&doc);
    // The name and the registration point in one dictionary, the group not wrapped again.
    assert!(p.contains("%AI14_BeginSymbol\n(Labeled)\n0 A\n0 Xw\n0 Ae\nu\n0.6 0 0.9 0.1 k\n"), "{p}");
    assert!(p.contains("%_/ArtDictionary :\n%_3 /Int (AI13PatternRegistrationType) ,\n%_/XMLUID : (Label) ; (AI10_ArtUID) ,\n%_;\n"), "{p}");
}

/// The names of symbols in every form a string takes: parentheses, a backslash, a slash, text
/// outside ASCII, an empty name (which the reader names `Symbol`); two symbols of one name keep the
/// first; an instance of a symbol the document doesn't have is an empty group, with a warning that
/// says so.
#[test]
fn symbol_names_and_missing_symbols() {
    let mut doc = Document::new(200.0, 100.0);
    let names = ["Paren (a)", "Back\\slash", "Path/To", "Hoja ñ 葉", "", "Twice"];
    for (i, name) in names.iter().chain(["Twice"].iter()).enumerate() {
        let art = square(&mut doc, Rect::new(0.0, 0.0, 4.0 + i as f64, 4.0), Color::gray(0.5));
        doc.symbols.push(Symbol { dynamic: false, name: (*name).into(), art: Arc::new(art) });
    }
    let layer = doc.layers[0].id;
    for name in names {
        let one = instance(&mut doc, name, Affine::translate((10.0, 10.0)));
        doc.insert(Some(layer), usize::MAX, one).unwrap();
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    let names = names.map(|n| if n.is_empty() { "Symbol" } else { n });
    assert_eq!(back.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), names);
    let refs: Vec<String> = back.layers[0]
        .children()
        .unwrap()
        .iter()
        .map(|n| match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } => symbol.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(refs, names);
    // The first symbol of a name is the one defined.
    let twice = &back.symbols.iter().find(|s| s.name == "Twice").unwrap().art;
    assert_eq!(paints_of_nodes(&back, std::slice::from_ref(twice)), paints_of_nodes(&doc, std::slice::from_ref(&doc.symbols[5].art)));
    // A missing symbol.
    let lost = instance(&mut doc, "Gone", Affine::translate((40.0, 10.0)));
    doc.insert(Some(layer), usize::MAX, lost).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::MISSING_SYMBOL]);
    let back = read_back(&doc);
    let last = back.layers[0].children().unwrap().last().unwrap().clone();
    assert!(matches!(&last.kind, NodeKind::Group { children, clip: false } if children.is_empty()), "{:?}", last.kind);
}

/// On a large canvas, a symbol's art holding a gradient and a nested instance is defined at its own
/// size and comes back as it was.
#[test]
fn a_symbol_with_a_gradient_and_an_instance_on_a_large_canvas_comes_back() {
    use vectorcraft_color::gradient::GradientGeom;
    let mut doc = Document::new(20000.0, 100.0);
    let leaf = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
    doc.symbols.push(Symbol { dynamic: false, name: "Leaf".into(), art: Arc::new(leaf) });
    let geom = GradientGeom { start: (0.0, 0.0).into(), end: (40.0, 10.0).into(), aspect: 1.0, focal: None };
    let shaded = painted(
        &mut doc,
        shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 20.0)),
        vec![AppearanceItem::Fill(FillLayer::new(gradient_paint(sunset(), Some(geom), None)))],
    );
    let nested = instance(&mut doc, "Leaf", Affine::translate((25.0, 5.0)) * Affine::rotate(0.4));
    let both = group(&mut doc, vec![shaded, nested], false);
    doc.symbols.push(Symbol { dynamic: false, name: "Both".into(), art: Arc::new(both) });
    let one = instance(&mut doc, "Both", Affine::translate((100.0, 10.0)));
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, one).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::SYMBOL_ART]);
    let back = read_back(&doc);
    same_lines(&symbol_looks(&back), &symbol_looks(&doc));
}

/// Instance matrices of every kind (turned, sheared, mirrored, scaled near the smallest area the
/// data keeps), nested in one another, are read back as written, and written again as the same
/// bytes. The matrices come from a fixed seed.
#[test]
fn generated_instance_matrices_come_back() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for round in 0..40 {
        let mut doc = one_swatch_document(300.0, 300.0);
        let base = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
        doc.symbols.push(Symbol { dynamic: false, name: "S0".into(), art: Arc::new(base) });
        let matrix = |next: &mut dyn FnMut() -> f64| {
            // Four nested matrices together scale by at most 64, within the 100 an instance may.
            let scale = 10f64.powf(next() * 3.0 - 3.0);
            Affine::new([
                (next() * 4.0 - 2.0) * scale,
                (next() * 4.0 - 2.0) * scale,
                (next() * 4.0 - 2.0) * scale,
                (next() * 4.0 - 2.0) * scale,
                next() * 300.0,
                next() * 300.0,
            ])
        };
        // A chain of symbols, each holding an instance of the one before.
        for level in 1..4 {
            let xf = matrix(&mut next);
            let inner = instance(&mut doc, &format!("S{}", level - 1), xf);
            let dot = square(&mut doc, Rect::new(0.0, 0.0, 3.0, 3.0), Color::gray(0.4));
            let art = group(&mut doc, vec![dot, inner], false);
            doc.symbols.push(Symbol { dynamic: false, name: format!("S{level}"), art: Arc::new(art) });
        }
        let layer = doc.layers[0].id;
        for _ in 0..6 {
            let pick = format!("S{}", (next() * 4.0) as usize % 4);
            let xf = matrix(&mut next);
            let one = instance(&mut doc, &pick, xf);
            doc.insert(Some(layer), usize::MAX, one).unwrap();
        }
        let out = export_ai_editing(&doc, &opts()).unwrap();
        let back = read_back(&doc);
        // Each instance written as one comes back with its matrix to ten decimals.
        let written = |d: &Document| {
            let mut all = vec![];
            for n in d.layers[0].children().unwrap().iter().chain(d.symbols.iter().map(|s| &s.art)) {
                n.walk(&mut |m| {
                    if let NodeKind::SymbolInstance { symbol, xf } = &m.kind {
                        all.push((symbol.clone(), *xf));
                    }
                });
            }
            all
        };
        assert!(out.warnings.is_empty(), "round {round}: {:?}", out.warnings);
        let (got, want) = (written(&back), written(&doc));
        assert_eq!(got.len(), want.len(), "round {round}");
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g.0, w.0, "round {round}");
            for (a, b) in g.1.as_coeffs().iter().zip(w.1.as_coeffs()) {
                assert!((a - b).abs() <= 1e-9 * b.abs().max(1.0), "round {round}: {:?} {:?}", g.1, w.1);
            }
        }
        let again = export_ai_editing(&back, &opts()).unwrap().program;
        same_lines(&shown(&again).lines().map(String::from).collect::<Vec<_>>(), &shown(&out.program).lines().map(String::from).collect::<Vec<_>>());
        assert_eq!(again, out.program, "round {round}");
    }
    // About the smallest area the data keeps: an instance whose matrix keeps it stays one, one
    // whose matrix (to ten decimals) loses it is written as its art.
    for (xf, live) in [
        (Affine::new([1e-6, 0.0, 0.0, 1.1e-6, 50.0, 10.0]), true),
        (Affine::new([1e-6, 0.0, 0.0, 0.9e-6, 50.0, 10.0]), false),
        (Affine::new([1.0, 0.4, 2.5, 1.0000000001, 50.0, 10.0]), true),
        (Affine::new([4e-11, 0.0, 0.0, 1e6, 50.0, 10.0]), false),
    ] {
        let mut doc = Document::new(200.0, 100.0);
        let base = square(&mut doc, Rect::new(0.0, 0.0, 10.0, 10.0), LEAF);
        doc.symbols.push(Symbol { dynamic: false, name: "S0".into(), art: Arc::new(base) });
        let one = instance(&mut doc, "S0", xf);
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), usize::MAX, one).unwrap();
        let back = read_back(&doc);
        assert_eq!(matches!(back.layers[0].children().unwrap()[0].kind, NodeKind::SymbolInstance { .. }), live, "{xf:?}");
    }
}

/// Type in a symbol's art is written as its outlines (the reader reads no type there), so the
/// symbol and its instance come back; type on the layers stays type.
#[test]
fn type_in_a_symbols_art_is_written_as_its_outlines() {
    let mut doc = Document::new(200.0, 100.0);
    let label = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(0.0, 20.0), "Tag", style(18.0, Color::BLACK));
    let label = text_node(&mut doc, label);
    doc.symbols.push(Symbol { dynamic: false, name: "Tag".into(), art: Arc::new(label) });
    let one = instance(&mut doc, "Tag", Affine::translate((20.0, 20.0)));
    let plain = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 90.0), "Plain", style(18.0, Color::BLACK));
    let plain = text_node(&mut doc, plain);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, one).unwrap();
    doc.insert(Some(layer), usize::MAX, plain).unwrap();
    assert_eq!(export_ai_editing(&doc, &opts()).unwrap().warnings, [writer::TYPE]);
    assert_eq!(program(&doc).matches("/AI11Text :").count(), 1);
    let back = read_back(&doc);
    assert_eq!(back.symbols.len(), 1);
    assert!(matches!(back.layers[0].children().unwrap()[0].kind, NodeKind::SymbolInstance { .. }));
}

// ---------- art styles ----------

/// Effect `id` with its defaults and `params` over them.
fn fx(id: &str, params: serde_json::Value) -> vectorcraft_doc::Effect {
    vectorcraft_effects::new_effect(id, &params).unwrap()
}

/// A path `name` over `r` filled with `fill` and stroked 2 pt in `stroke` (none when `Paint::None`),
/// with `effects`.
fn styled_rect(doc: &mut Document, name: &str, r: Rect, fill: Paint, stroke: Paint, effects: Vec<vectorcraft_doc::Effect>) -> Node {
    let mut ap = Appearance::basic(fill, stroke, 2.0);
    ap.effects = effects;
    named(Node::path(doc.alloc_id(), shapes::rectangle(r), ap), name)
}

/// A 400 × 300 document whose objects keep their live effects as art styles: a drop shadow in a
/// color and one by darkness, an inner and an outer glow in that order, Feather, Round Corners,
/// Scribble, Convert to Shape, Offset Path, Outline Stroke on an open path, a drop shadow on a
/// gradient fill and on a group with transparency, a hidden effect, Pathfinder › Add on a group, a
/// hidden locked object, an opacity mask, an object linked to a graphic style; and a graphic style
/// of its own with two fills, a stroke, item transparency and effects.
fn styles_document() -> Document {
    let mut doc = Document::new(400.0, 300.0);
    let layer = doc.layers[0].id;
    let add = |doc: &mut Document, n: Node| {
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    };
    let red = solid(Color::rgb(0.9, 0.2, 0.1));
    let blue = solid(Color::rgb(0.1, 0.3, 0.8));
    let shadow = fx("stylize.dropShadow", serde_json::json!({"x": 6.0, "y": 4.0, "blur": 8.0, "opacity": 60.0, "color": "#203040"}));
    let n = styled_rect(&mut doc, "Shadowed", Rect::new(10.0, 10.0, 60.0, 50.0), red.clone(), blue.clone(), vec![shadow.clone()]);
    add(&mut doc, n);
    let dark = fx("stylize.dropShadow", serde_json::json!({"shade": "darkness", "darkness": 40.0, "mode": "darken"}));
    let n = styled_rect(&mut doc, "Darkened", Rect::new(70.0, 10.0, 120.0, 50.0), blue.clone(), Paint::None, vec![dark]);
    add(&mut doc, n);
    let glows = vec![
        fx("stylize.innerGlow", serde_json::json!({"source": "center", "blur": 3.0})),
        fx("stylize.outerGlow", serde_json::json!({"blur": 9.0, "opacity": 50.0})),
    ];
    let n = styled_rect(&mut doc, "Glowing", Rect::new(130.0, 10.0, 180.0, 50.0), red.clone(), Paint::None, glows);
    add(&mut doc, n);
    let n = styled_rect(
        &mut doc,
        "Soft",
        Rect::new(190.0, 10.0, 240.0, 50.0),
        blue.clone(),
        Paint::None,
        vec![fx("stylize.feather", serde_json::json!({"radius": 7.0}))],
    );
    add(&mut doc, n);
    let round = fx("stylize.roundCorners", serde_json::json!({"radius": 9.0}));
    let n = styled_rect(&mut doc, "Rounded", Rect::new(250.0, 10.0, 300.0, 50.0), red.clone(), blue.clone(), vec![round]);
    add(&mut doc, n);
    let n = styled_rect(
        &mut doc,
        "Scribbled",
        Rect::new(310.0, 10.0, 360.0, 50.0),
        blue.clone(),
        Paint::None,
        vec![fx("stylize.scribble", serde_json::json!({}))],
    );
    add(&mut doc, n);
    let shape = fx("convertToShape.roundedRectangle", serde_json::json!({"relative": true, "extraW": 12.0, "extraH": 8.0, "radius": 5.0}));
    let n = styled_rect(&mut doc, "Shaped", Rect::new(10.0, 70.0, 60.0, 110.0), red.clone(), Paint::None, vec![shape]);
    add(&mut doc, n);
    let offset = fx("path.offsetPath", serde_json::json!({"offset": 4.0, "joins": "miter", "miterLimit": 6.0}));
    let n = styled_rect(&mut doc, "Offset", Rect::new(70.0, 70.0, 120.0, 110.0), blue.clone(), red.clone(), vec![offset]);
    add(&mut doc, n);
    let mut open = PathData::from_bezpath(&kurbo::BezPath::from_vec(vec![
        kurbo::PathEl::MoveTo((130.0, 110.0).into()),
        kurbo::PathEl::LineTo((180.0, 70.0).into()),
    ]));
    open.subpaths.iter_mut().for_each(|sp| sp.closed = false);
    let mut ap = Appearance::basic(Paint::None, red.clone(), 6.0);
    ap.effects = vec![fx("path.outlineStroke", serde_json::json!({}))];
    let n = named(Node::path(doc.alloc_id(), open, ap), "Outlined");
    add(&mut doc, n);
    let ramp = Gradient {
        kind: GradientKind::Linear,
        stops: vec![GradientStop::new(0.0, Color::rgb(1.0, 1.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.6, 0.2))],
        ..Gradient::default()
    };
    let graded = Paint::Gradient(Box::new(GradientPaint::new(ramp)));
    let n = styled_rect(&mut doc, "Graded", Rect::new(190.0, 70.0, 240.0, 110.0), graded, Paint::None, vec![shadow.clone()]);
    add(&mut doc, n);
    let mut hidden_fx = fx("stylize.outerGlow", serde_json::json!({}));
    hidden_fx.visible = false;
    let mut quiet = styled_rect(&mut doc, "Quiet", Rect::new(250.0, 70.0, 300.0, 110.0), blue.clone(), Paint::None, vec![hidden_fx]);
    quiet.visible = false;
    quiet.locked = true;
    add(&mut doc, quiet);
    // A group with a shadow, transparency and a mask; a group merged by Pathfinder › Add.
    let a = square(&mut doc, Rect::new(10.0, 130.0, 50.0, 170.0), Color::rgb(0.2, 0.6, 0.3));
    let b = square(&mut doc, Rect::new(30.0, 150.0, 70.0, 190.0), Color::rgb(0.6, 0.2, 0.5));
    let mut shaded = see_through(group(&mut doc, vec![a, b], false), 0.5, BlendMode::Multiply);
    shaded.appearance = Appearance { effects: vec![shadow.clone()], ..Appearance::default() }.into();
    let mask_art = square(&mut doc, Rect::new(10.0, 130.0, 70.0, 160.0), Color::gray(0.0));
    shaded.extras_mut().mask = Some(OpacityMask { art: Arc::new(mask_art), clip: true, invert: false, disabled: false, linked: true }.into());
    add(&mut doc, named(shaded, "Shaded group"));
    let a = square(&mut doc, Rect::new(90.0, 130.0, 130.0, 170.0), Color::rgb(0.9, 0.6, 0.1));
    let b = square(&mut doc, Rect::new(110.0, 150.0, 150.0, 190.0), Color::rgb(0.9, 0.6, 0.1));
    let mut merged = group(&mut doc, vec![a, b], false);
    merged.appearance = Appearance { effects: vec![fx("pathfinder.add", serde_json::json!({}))], ..Appearance::default() }.into();
    add(&mut doc, named(merged, "Merged"));
    // A graphic style of its own, and an object linked to one.
    let mut fill_top = FillLayer::new(solid(Color::rgb(0.2, 0.2, 0.9)));
    fill_top.opacity = 0.5;
    fill_top.blend = BlendMode::Screen;
    fill_top.effects = vec![fx("distort.twist", serde_json::json!({"angle": 30.0}))];
    let style_ap = Appearance {
        items: vec![
            AppearanceItem::Fill(FillLayer::new(solid(Color::rgb(0.9, 0.9, 0.1)))),
            AppearanceItem::Fill(fill_top),
            AppearanceItem::Stroke(StrokeLayer::new(solid(Color::BLACK), 3.0)),
        ],
        effects: vec![fx("warp.arc", serde_json::json!({"bend": 30.0})), shadow],
        // The contents over every fill and stroke.
        contents_index: Some(3),
    };
    let mut layered = vectorcraft_doc::GraphicStyle::new("Layered", style_ap);
    layered.id = 9;
    layered.opacity = 0.8;
    doc.graphic_styles.push(layered);
    let sunshine = doc.graphic_style("Sunshine").unwrap().clone();
    let mut linked_obj = Node::path(
        doc.alloc_id(),
        shapes::rectangle(Rect::new(170.0, 130.0, 220.0, 170.0)),
        sunshine.appearance_on(&Node::group(vectorcraft_doc::NodeId(0), vec![])).into_owned(),
    );
    linked_obj.extras_mut().graphic_style = Some(sunshine.id);
    add(&mut doc, named(linked_obj, "Sunny"));
    doc
}

/// Each object's name, kind, transparency, state, mask and appearance, and whether it is linked to a
/// graphic style (by name), compared with [`writer::same_appearance`] apart.
fn style_view(doc: &Document) -> Vec<(String, Appearance)> {
    let mut out = vec![];
    for n in doc.layers.iter().flat_map(|l| l.children().cloned().unwrap_or_default()) {
        let kind = match &n.kind {
            NodeKind::Group { .. } => "group",
            NodeKind::Path { .. } => "path",
            NodeKind::Compound { .. } => "compound",
            _ => "other",
        };
        let link = n.extras().graphic_style.and_then(|id| doc.graphic_style_by_id(id)).map(|g| g.name.clone());
        let head = format!(
            "{:?} {kind} {} {:?} isolate={} {:?} shown={} locked={} mask={} link={link:?} members={}",
            n.name,
            r3(f64::from(n.opacity)),
            n.blend,
            n.isolate,
            n.knockout,
            n.visible,
            n.locked,
            n.extras().mask.is_some(),
            n.children().map_or(0, Vec::len)
        );
        out.push((head, (*n.appearance).clone()));
    }
    out
}

/// `ap` with its gradients' stops and kind only: their placement is the slice of gradients' (a
/// gradient comes back placed on the object, named after its definition).
fn unplaced(ap: &Appearance) -> Appearance {
    let mut ap = ap.clone();
    for item in ap.items.iter_mut() {
        let paint = match item {
            AppearanceItem::Fill(f) => &mut f.paint,
            AppearanceItem::Stroke(s) => &mut s.paint,
        };
        if let Paint::Gradient(g) = paint {
            (g.geom, g.swatch, g.angle) = (None, None, 0.0);
        }
    }
    ap
}

fn same_styles(got: &Document, want: &Document) {
    let (g, w) = (style_view(got), style_view(want));
    same_lines(&g.iter().map(|(h, _)| h.clone()).collect::<Vec<_>>(), &w.iter().map(|(h, _)| h.clone()).collect::<Vec<_>>());
    for ((h, a), (_, b)) in g.iter().zip(&w) {
        assert!(writer::same_appearance(&unplaced(a), &unplaced(b)), "{h}:\n  got  {a:?}\n  want {b:?}");
    }
    same_lines(&outlines(got), &outlines(want));
}

/// The points of `doc`'s objects and of their opacity masks, each path's fill rule and each
/// object's nesting, to the written precision.
fn outlines(doc: &Document) -> Vec<String> {
    fn walk(n: &Node, depth: usize, out: &mut Vec<String>) {
        let mut points = |path: &PathData, rule: Option<FillRule>| {
            let mut s = format!("{depth} {:?} {rule:?}", n.name);
            for sp in &path.subpaths {
                s += &format!(" [closed={}", sp.closed);
                for a in &sp.anchors {
                    s += &format!(" {:.2},{:.2}", a.p.x + 0.0, a.p.y + 0.0);
                }
                s += "]";
            }
            out.push(s);
        };
        match &n.kind {
            NodeKind::Path { path, rule, .. } => points(path, Some(*rule)),
            NodeKind::Compound { rule, children } => {
                for c in children {
                    points(c.path_data().unwrap(), Some(*rule));
                }
            }
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => {
                out.push(format!("{depth} {:?} container", n.name));
                for c in children {
                    walk(c, depth + 1, out);
                }
            }
            other => out.push(format!("{depth} {other:?}")),
        }
        if let Some(mask) = &n.extras().mask {
            out.push(format!("{depth} mask clip={} invert={} disabled={} linked={}", mask.clip, mask.invert, mask.disabled, mask.linked));
            // The reader holds a mask's art in a group.
            match &mask.art.kind {
                NodeKind::Group { children, .. } if children.len() == 1 => walk(&children[0], depth + 1, out),
                _ => walk(&mask.art, depth + 1, out),
            }
        }
    }
    let mut out = vec![];
    for l in doc.layers.iter() {
        walk(l, 0, &mut out);
    }
    out
}

#[test]
fn objects_with_live_effects_and_graphic_styles_are_read_back() {
    let doc = styles_document();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::EFFECTS || w == writer::PANEL_STYLES), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_styles(&back, &doc);
    // The panel's styles, in its order, with their appearance and transparency.
    let names = |d: &Document| d.graphic_styles.iter().map(|g| g.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&back), names(&doc));
    for (g, w) in back.graphic_styles.iter().zip(doc.graphic_styles.iter()) {
        assert!(writer::same_appearance(&g.appearance, &w.appearance), "{}:\n  got  {:?}\n  want {:?}", w.name, g.appearance, w.appearance);
        assert_eq!(
            (r3(f64::from(g.opacity)), g.blend, g.isolate, g.knockout),
            (r3(f64::from(w.opacity)), w.blend, w.isolate, w.knockout),
            "{}",
            w.name
        );
    }
    // A group's members come back as they were.
    let members = |d: &Document, name: &str| {
        let mut out = vec![];
        let g = d.layers[0].children().unwrap().iter().find(|n| n.name.as_deref() == Some(name)).unwrap().clone();
        view(g.children().unwrap(), 0, &mut out);
        out
    };
    same_lines(&members(&back, "Shaded group"), &members(&doc, "Shaded group"));
    same_lines(&members(&back, "Merged"), &members(&doc, "Merged"));
}

/// In a document with only a new document's graphic styles, an object linked to one of them keeps
/// its link: the panel's styles are written.
#[test]
fn a_link_to_a_built_in_graphic_style_is_read_back() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let sunshine = doc.graphic_style("Sunshine").unwrap().clone();
    let ap = sunshine.appearance_on(&Node::group(vectorcraft_doc::NodeId(0), vec![])).into_owned();
    // Without a link, a new document's styles are not written.
    let mut plain = doc.clone();
    let unlinked = Node::path(plain.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 50.0)), ap.clone());
    plain.insert(Some(layer), usize::MAX, unlinked).unwrap();
    assert!(!program(&plain).contains("%AI9_BeginArtStyleList"));
    let mut sunny = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 50.0)), ap);
    sunny.extras_mut().graphic_style = Some(sunshine.id);
    doc.insert(Some(layer), usize::MAX, named(sunny, "Sunny")).unwrap();
    assert_eq!(doc.graphic_styles, Document::new(1.0, 1.0).graphic_styles);
    let back = read_back(&doc);
    same_styles(&back, &doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert_eq!(n.extras().graphic_style.and_then(|id| back.graphic_style_by_id(id)).map(|g| g.name.as_str()), Some("Sunshine"));
}

#[test]
fn art_styles_are_written_in_their_forms() {
    let p = program_shown(&styles_document());
    // The styles in a non-printing block after the symbols' and before the palette; the panel's in
    // its order first, then the objects' own.
    let styles = p.find("%AI5_Begin_NonPrinting\nNp\n%AI9_BeginArtStyles\n/KnownStyle :\n(Default Graphic Style) /Name ,\n/ActiveStyle :\n").unwrap();
    assert!(styles < p.find("%AI5_BeginPalette").unwrap(), "{p}");
    let list = "%AI5_Begin_NonPrinting\nNp\n%AI9_BeginArtStyleList\n(Default Graphic Style)\n(Black Outline)\n(Heavy Ink)\n(Sunshine)\n(Layered)\n%AI9_EndArtStyleList\n%AI5_End_NonPrinting--\n";
    let at = p.find(list).unwrap();
    assert!(p.find("%AI5_EndPalette").unwrap() < at && at < p.find("%AI9_BeginDocumentData").unwrap(), "{p}");
    // A drop shadow on the object, after its stack and before its transparency.
    assert!(
        p.contains(
            "/BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n1 /Int (blnd) ,\n0.6 /Real (opac) ,\n\
             4 /Real (vert) ,\n6 /Real (horz) ,\n4 /Real (blur) ,\n/FillStyle : 0 O\n"
        ),
        "{p}"
    );
    // The drawn look, its state block with the object's transparency, the object on `%_` lines
    // painted with nothing, its name, and its style.
    assert!(p.contains("U\n0 1 0 2 0 Xy\n0 0 Xd\n6 () XW\n%_0 XR\n%_0 1 0 0 0 Xy\n%_10 -10 m\n"), "{p}");
    assert!(p.contains("%_n\n%_/ArtDictionary :\n%_/XMLUID : (Shadowed) ; (AI10_ArtUID) ,\n%_;\n%_\n1 (Anon 1) XW\n"), "{p}");
    assert!(p.contains("%_n\n%_/ArtDictionary :\n%_/XMLUID : (Offset) ; (AI10_ArtUID) ,\n%_;\n%_\n1 (Anon "), "{p}");
    // The object linked to a graphic style names it.
    assert!(p.contains("%_/XMLUID : (Sunny) ; (AI10_ArtUID) ,\n%_;\n%_\n1 (Sunshine) XW\n"), "{p}");
    // A group's members on `%_` lines, then its name and style.
    assert!(p.contains("6 () XW\n%_u\n"), "{p}");
    assert!(p.contains("%_U\n%_/ArtDictionary :\n%_/XMLUID : (Merged) ; (AI10_ArtUID) ,\n%_;\n%_\n1 (Anon "), "{p}");
    // Objects of the same appearance share their style.
    let two = {
        let mut doc = Document::new(200.0, 100.0);
        let shadow = fx("stylize.dropShadow", serde_json::json!({}));
        for x in [10.0, 60.0] {
            let n = styled_rect(&mut doc, "Twin", Rect::new(x, 10.0, x + 30.0, 40.0), solid(Color::BLACK), Paint::None, vec![shadow.clone()]);
            let layer = doc.layers[0].id;
            doc.insert(Some(layer), usize::MAX, n).unwrap();
        }
        program(&doc)
    };
    assert_eq!((two.matches("/KnownStyle :").count(), two.matches("1 (Anon 1) XW\n").count()), (1, 2), "{two}");
    assert!(!two.contains("%AI9_BeginArtStyleList"), "{two}");
}

/// Objects whose appearance the reader wouldn't read with their style are written as their
/// drawn look, with the warnings they were written with before; graphic styles it wouldn't read
/// are left out with a warning; and a document without effects or styles of its own has no art
/// styles.
#[test]
fn appearances_the_reader_wouldnt_read_are_their_drawn_look() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let shadow = fx("stylize.dropShadow", serde_json::json!({}));
    // Two fills and an effect: the reader keeps such an object's drawn look.
    let mut two = styled_rect(&mut doc, "Two fills", Rect::new(10.0, 10.0, 40.0, 40.0), solid(Color::BLACK), Paint::None, vec![shadow.clone()]);
    let mut ap = (*two.appearance).clone();
    ap.items.push(AppearanceItem::Fill(FillLayer { opacity: 0.5, ..FillLayer::new(solid(Color::WHITE)) }));
    two.appearance = ap.into();
    doc.insert(Some(layer), usize::MAX, two).unwrap();
    // A warp on a path: the reader keeps a path's drawn look for it.
    let warped = styled_rect(
        &mut doc,
        "Warped",
        Rect::new(50.0, 10.0, 80.0, 40.0),
        solid(Color::BLACK),
        Paint::None,
        vec![fx("warp.arc", serde_json::json!({}))],
    );
    doc.insert(Some(layer), usize::MAX, warped).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    let p = String::from_utf8(out.program).unwrap();
    assert!(!p.contains("%AI9_BeginArtStyles") && !p.lines().any(|l| l.starts_with("1 (")), "{p}");
    let back = read_back(&doc);
    assert!(back.layers[0].children().unwrap().iter().all(|n| n.appearance.effects.is_empty()));
    // A graphic style with a gradient: left out, the others listed.
    let ramp = Gradient { stops: vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)], ..Gradient::default() };
    let graded = Appearance::basic(Paint::Gradient(Box::new(GradientPaint::new(ramp))), Paint::None, 0.0);
    doc.graphic_styles.push(vectorcraft_doc::GraphicStyle::new("Graded", graded));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PANEL_STYLES), "{:?}", out.warnings);
    let p = String::from_utf8(out.program).unwrap().replace("\r\n", "\n");
    assert!(p.contains("(Sunshine)\n%AI9_EndArtStyleList") && !p.contains("(Graded)"), "{p}");
    // Nothing styled: no art styles.
    assert!(!program(&sample()).contains("ArtStyle"));
}

/// A graphic style's opacity is read back within the written number's precision: the style and an
/// object's link to it stay.
#[test]
fn a_graphic_styles_opacity_is_compared_at_the_written_precision() {
    let mut doc = Document::new(200.0, 100.0);
    let ap = Appearance::basic(solid(Color::BLACK), Paint::None, 1.0);
    let mut g = vectorcraft_doc::GraphicStyle::new("Fine", ap.clone());
    g.id = 1;
    g.opacity = 0.123456;
    *doc.graphic_styles = vec![g];
    let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 50.0)), ap);
    n.opacity = 0.123456;
    n.extras_mut().graphic_style = Some(1);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::PANEL_STYLES), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.graphic_styles.len(), 1);
    assert!((back.graphic_styles[0].opacity - 0.123456).abs() < 1e-3);
    assert_eq!(back.layers[0].children().unwrap()[0].extras().graphic_style, Some(back.graphic_styles[0].id));
}

/// A compound shape draws as one area, which an art style's object wouldn't keep: it and a group
/// holding one are written as their drawn look, with a warning.
#[test]
fn compound_shapes_are_not_written_with_an_art_style() {
    let shadow = || fx("stylize.dropShadow", serde_json::json!({}));
    let shape = |doc: &mut Document| {
        let a = square(doc, Rect::new(10.0, 10.0, 50.0, 50.0), Color::BLACK);
        let b = square(doc, Rect::new(30.0, 30.0, 70.0, 70.0), Color::BLACK);
        let mut g = group(doc, vec![a, b], false);
        g.extras_mut().compound_shape = true;
        g
    };
    let mut own = Document::new(200.0, 100.0);
    let mut n = shape(&mut own);
    n.appearance = Appearance { effects: vec![shadow()], ..Appearance::default() }.into();
    let layer = own.layers[0].id;
    own.insert(Some(layer), usize::MAX, n).unwrap();
    let mut nested = Document::new(200.0, 100.0);
    let member = shape(&mut nested);
    let mut n = group(&mut nested, vec![member], false);
    n.appearance = Appearance { effects: vec![shadow()], ..Appearance::default() }.into();
    let layer = nested.layers[0].id;
    nested.insert(Some(layer), usize::MAX, n).unwrap();
    for doc in [own, nested] {
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
        assert!(!shown(&out.program).contains("1 (Anon 1) XW"));
    }
}

/// Graphic styles the reader's style list wouldn't hold (a name over its limit, styles past its
/// count) are left out with a warning, and an object linked to one has no link.
#[test]
fn graphic_styles_past_the_lists_limits_are_left_out() {
    let plain = || Appearance::basic(solid(Color::BLACK), Paint::None, 1.0);
    let styled = |name: String, id: u32| {
        let mut g = vectorcraft_doc::GraphicStyle::new(name, plain());
        g.id = id;
        g
    };
    // A name one byte over the limit, among others.
    let mut doc = Document::new(200.0, 100.0);
    let long = "n".repeat(1025);
    *doc.graphic_styles = vec![styled("Before".into(), 1), styled(long.clone(), 2), styled("After".into(), 3)];
    let mut linked = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 50.0)), plain());
    linked.extras_mut().graphic_style = Some(2);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, linked).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PANEL_STYLES), "{:?}", out.warnings);
    assert!(!shown(&out.program).contains(&long));
    let back = read_back(&doc);
    let names: Vec<&str> = back.graphic_styles.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["Before", "After"]);
    assert_eq!(back.layers[0].children().unwrap()[0].extras().graphic_style, None);
    // A name at the limit is kept.
    let at_limit = "n".repeat(1024);
    doc.graphic_styles[1].name = at_limit.clone();
    assert!(read_back(&doc).graphic_styles.iter().any(|g| g.name == at_limit));
    // Styles past the count.
    let mut many = Document::new(200.0, 100.0);
    *many.graphic_styles = (1..=4097).map(|i| styled(format!("Style {i}"), i)).collect();
    let out = export_ai_editing(&many, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PANEL_STYLES), "{:?}", out.warnings);
    let back = read_back(&many);
    assert_eq!(back.graphic_styles.len(), 4096);
    assert_eq!(back.graphic_styles.last().map(|g| g.name.as_str()), Some("Style 4096"));
}

/// A member of a group whose Transform effect moves it is written where baking put it, not with
/// its own effects where it was.
#[test]
fn a_styled_member_of_a_moved_group_stays_where_it_draws() {
    fn left_of_paths(n: &Node) -> f64 {
        let own = n.path_data().map_or(f64::INFINITY, |p| kurbo::Shape::bounding_box(&p.to_bezpath()).x0);
        n.children().map_or(own, |c| c.iter().map(|m| left_of_paths(m)).fold(own, f64::min))
    }
    let mut doc = Document::new(300.0, 100.0);
    let layer = doc.layers[0].id;
    let shadow = fx("stylize.dropShadow", serde_json::json!({}));
    let member = styled_rect(&mut doc, "Member", Rect::new(10.0, 10.0, 40.0, 40.0), solid(Color::BLACK), Paint::None, vec![shadow]);
    let mut moved = group(&mut doc, vec![member], false);
    moved.appearance = Appearance { effects: vec![fx("distort.transform", serde_json::json!({"moveH": 70.0}))], ..Appearance::default() }.into();
    doc.insert(Some(layer), usize::MAX, moved).unwrap();
    let back = read_back(&doc);
    let left = left_of_paths(&back.layers[0]);
    assert!((left - 80.0).abs() < 1e-3, "the art starts at x = {left}");
}

/// A hidden, locked styled group keeps its state when its members are shown and unlocked.
#[test]
fn a_styled_group_keeps_its_state_after_members_in_another() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let member = square(&mut doc, Rect::new(10.0, 10.0, 40.0, 40.0), Color::BLACK);
    let mut shaded = group(&mut doc, vec![member], false);
    shaded.appearance = Appearance { effects: vec![fx("stylize.dropShadow", serde_json::json!({}))], ..Appearance::default() }.into();
    shaded.visible = false;
    shaded.locked = true;
    doc.insert(Some(layer), usize::MAX, shaded).unwrap();
    let back = read_back(&doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert!(matches!(n.kind, NodeKind::Group { .. }));
    assert_eq!(n.appearance.effects.len(), 1);
    assert!(!n.visible && n.locked, "visible {} locked {}", n.visible, n.locked);
}

/// Effects in a styled object's opacity mask are not its style's: leaving them out warns.
#[test]
fn effects_in_a_styled_objects_mask_are_warned_about() {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let shadow = fx("stylize.dropShadow", serde_json::json!({}));
    let mut n = styled_rect(&mut doc, "Masked", Rect::new(10.0, 10.0, 40.0, 40.0), solid(Color::BLACK), Paint::None, vec![shadow]);
    let blurred = styled_rect(
        &mut doc,
        "Mask art",
        Rect::new(10.0, 10.0, 40.0, 25.0),
        solid(Color::gray(0.0)),
        Paint::None,
        vec![fx("blur.gaussian", serde_json::json!({}))],
    );
    n.extras_mut().mask = Some(OpacityMask { art: Arc::new(blurred), clip: true, invert: false, disabled: false, linked: true }.into());
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    assert!(shown(&out.program).contains("\n1 (Anon 1) XW\n"));
}

/// On a large canvas no art style is written (the reader scales the styles of a large canvas):
/// objects with effects are written as their vector art with their geometry effects applied, with a
/// warning.
#[test]
fn a_large_canvas_writes_no_art_styles() {
    let mut doc = Document::new(20000.0, 100.0);
    let n = styled_rect(
        &mut doc,
        "Far",
        Rect::new(10.0, 10.0, 40.0, 40.0),
        solid(Color::BLACK),
        Paint::None,
        vec![fx("stylize.feather", serde_json::json!({}))],
    );
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::LARGE_CANVAS_STYLES), "{:?}", out.warnings);
    assert!(!String::from_utf8(out.program).unwrap().contains("ArtStyle"));
}

#[test]
fn art_styles_write_the_same_bytes_again() {
    let doc = styles_document();
    let first = export_ai_editing(&doc, &opts()).unwrap();
    assert!(shown(&first.program).contains("\n1 (Anon 1) XW\n"));
    assert_eq!(first.program, export_ai_editing(&doc, &opts()).unwrap().program);
    assert_eq!(private_data(&doc, &opts()).unwrap().0, private_data(&doc, &opts()).unwrap().0);
    // What is read back is written as it was.
    let again = export_ai_editing(&read_back(&doc), &opts()).unwrap().program;
    same_lines(&shown(&again).lines().map(String::from).collect::<Vec<_>>(), &shown(&first.program).lines().map(String::from).collect::<Vec<_>>());
}

/// `n` on the first layer of `doc`.
fn put(doc: &mut Document, n: Node) {
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();
}

fn drop_shadow() -> vectorcraft_doc::Effect {
    fx("stylize.dropShadow", serde_json::json!({}))
}

fn black_fill() -> Paint {
    solid(Color::BLACK)
}

/// An even-odd compound path of a square inside a square, with `ap`.
fn ring(doc: &mut Document, ap: Appearance) -> Node {
    let outer = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(70.0, 10.0, 110.0, 50.0)), Appearance::default());
    let inner = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(80.0, 20.0, 100.0, 40.0)), Appearance::default());
    let mut n = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(inner)], rule: FillRule::EvenOdd });
    n.appearance = ap.into();
    n
}

/// A path filled black with no stroke and `effects`.
fn plain_rect(doc: &mut Document, name: &str, effects: Vec<vectorcraft_doc::Effect>) -> Node {
    styled_rect(doc, name, Rect::new(10.0, 10.0, 50.0, 40.0), black_fill(), Paint::None, effects)
}

/// A 200 × 100 document's first object as it reads back from a document of only `n`.
fn read_one(make: impl FnOnce(&mut Document) -> Node) -> (Arc<Node>, Vec<String>, String) {
    let mut doc = Document::new(200.0, 100.0);
    let n = make(&mut doc);
    put(&mut doc, n);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    let back = read_back(&doc);
    (back.layers[0].children().unwrap()[0].clone(), out.warnings, shown(&out.program))
}

/// A filled path or compound path with live effects that fills by the even-odd rule is written as its
/// drawn look with a warning, since a style's fill fills by the non-zero rule. A stroke-only object
/// keeps its style and its own fill rule. A filled even-odd object linked to a graphic style isn't
/// written with that style, and its link is left out with a warning.
#[test]
fn a_filled_even_odd_object_is_written_as_its_drawn_look() {
    let mut doc = Document::new(300.0, 100.0);
    let mut odd = plain_rect(&mut doc, "Odd", vec![drop_shadow()]);
    if let NodeKind::Path { rule, .. } = &mut odd.kind {
        *rule = FillRule::EvenOdd;
    }
    put(&mut doc, odd);
    let ap = Appearance { effects: vec![drop_shadow()], ..Appearance::basic(black_fill(), Paint::None, 0.0) };
    let n = named(ring(&mut doc, ap), "Ring");
    put(&mut doc, n);
    let n = plain_rect(&mut doc, "Even", vec![drop_shadow()]);
    put(&mut doc, n);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    let p = shown(&out.program);
    assert_eq!((p.matches("/KnownStyle :").count(), p.matches("\n1 (Anon 1) XW\n").count()), (1, 1), "{p}");
    assert!(!p.contains("1 /Bool (UseEvenOdd)"), "{p}");

    let mut doc = Document::new(300.0, 100.0);
    let ap = Appearance { effects: vec![drop_shadow()], ..Appearance::basic(Paint::None, solid(Color::BLACK), 2.0) };
    let n = named(ring(&mut doc, ap), "Stroked");
    put(&mut doc, n);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_styles(&back, &doc);
    assert!(matches!(back.layers[0].children().unwrap()[0].kind, NodeKind::Compound { rule: FillRule::EvenOdd, .. }));

    let mut doc = Document::new(200.0, 100.0);
    let ap = Appearance::basic(black_fill(), Paint::None, 0.0);
    let mut g = vectorcraft_doc::GraphicStyle::new("Solid", ap.clone());
    g.id = 7;
    doc.graphic_styles.push(g);
    let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap);
    n.extras_mut().graphic_style = Some(7);
    if let NodeKind::Path { rule, .. } = &mut n.kind {
        *rule = FillRule::EvenOdd;
    }
    put(&mut doc, n);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::STYLE_LINKS), "{:?}", out.warnings);
    let back = read_back(&doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert!(matches!(n.kind, NodeKind::Path { rule: FillRule::EvenOdd, .. }), "{n:?}");
    assert_eq!(n.extras().graphic_style, None);
}

/// A compound path filled by the non-zero rule keeps its effects, its gradient fill and its link to
/// a graphic style.
#[test]
fn a_compound_path_keeps_its_style() {
    let mut doc = Document::new(300.0, 100.0);
    let non_zero = |mut n: Node| {
        if let NodeKind::Compound { rule, .. } = &mut n.kind {
            *rule = FillRule::NonZero;
        }
        n
    };
    let ap = Appearance { effects: vec![drop_shadow()], ..Appearance::basic(black_fill(), solid(Color::WHITE), 2.0) };
    let n = named(non_zero(ring(&mut doc, ap)), "Shadowed");
    put(&mut doc, n);
    let stops = vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)];
    let graded = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let ap = Appearance { effects: vec![drop_shadow()], ..Appearance::basic(graded, Paint::None, 0.0) };
    let n = named(non_zero(ring(&mut doc, ap)), "Graded");
    put(&mut doc, n);
    let sunshine = doc.graphic_style("Sunshine").unwrap().clone();
    let ap = sunshine.appearance_on(&Node::group(vectorcraft_doc::NodeId(0), vec![])).into_owned();
    let mut linked_ring = named(ring(&mut doc, ap), "Linked");
    if let NodeKind::Compound { rule, .. } = &mut linked_ring.kind {
        *rule = FillRule::NonZero;
    }
    linked_ring.extras_mut().graphic_style = Some(sunshine.id);
    put(&mut doc, linked_ring);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::EFFECTS || w == writer::STYLE_LINKS), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_styles(&back, &doc);
    assert!(back.layers[0].children().unwrap().iter().all(|n| matches!(n.kind, NodeKind::Compound { .. })));
}

/// A group holding an image is written as its drawn look: the image stays a placed file, once.
#[test]
fn a_group_holding_an_image_is_written_as_its_drawn_look() {
    let mut doc = Document::new(200.0, 100.0);
    let link = Some(vectorcraft_doc::LinkInfo::new("/tmp/ramp.png"));
    let im = image_node(&mut doc, ImageBlob::png(rgb_png()), (3, 2), Affine::scale(10.0), link);
    let mut g = group(&mut doc, vec![im], false);
    g.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::default() }.into();
    put(&mut doc, g);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    assert!(!out.warnings.iter().any(|w| w == writer::LINKS_EMBEDDED), "{:?}", out.warnings);
    assert!(!shown(&out.program).contains("(Anon 1)"));
    let back = read_back(&doc);
    let g = &back.layers[0].children().unwrap()[0];
    let member = g.children().map_or(g, |c| &c[0]);
    let NodeKind::Image(im) = &member.kind else { panic!("{member:?}") };
    assert!(im.link.is_some());
}

/// A link to a graphic style that isn't written warns; one that is written doesn't.
#[test]
fn a_link_left_out_warns() {
    let warns = |make: &dyn Fn(&mut Document, u32) -> Node| {
        let mut doc = Document::new(200.0, 100.0);
        let ap = Appearance {
            items: vec![AppearanceItem::Fill(FillLayer::new(black_fill())), AppearanceItem::Fill(FillLayer::new(solid(Color::WHITE)))],
            ..Appearance::default()
        };
        let mut layered = vectorcraft_doc::GraphicStyle::new("Layered", ap);
        layered.id = 7;
        doc.graphic_styles.push(layered);
        let mut plain = vectorcraft_doc::GraphicStyle::new("Plain", Appearance::basic(black_fill(), Paint::None, 0.0));
        plain.id = 8;
        doc.graphic_styles.push(plain);
        let n = make(&mut doc, 7);
        put(&mut doc, n);
        let warned = export_ai_editing(&doc, &opts()).unwrap().warnings.iter().any(|w| w == writer::STYLE_LINKS);
        (warned, read_back(&doc))
    };
    let linked = |mut n: Node, id: u32| {
        n.extras_mut().graphic_style = Some(id);
        n
    };
    // A style of two fills: a path isn't read with it.
    let (warned, back) = warns(&|doc, id| {
        let ap = doc.graphic_style_by_id(id).unwrap().appearance.clone();
        linked(Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap), id)
    });
    assert!(warned);
    assert_eq!(back.layers[0].children().unwrap()[0].extras().graphic_style, None);
    assert!(back.graphic_style("Layered").is_some());
    // An image, a symbol instance and a knockout shape are never written with a style.
    let (warned, _) = warns(&|doc, _| {
        let art = square(doc, Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK);
        doc.symbols.push(Symbol { dynamic: false, name: "S".into(), art: Arc::new(art) });
        linked(Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((50.0, 20.0)) }), 8)
    });
    assert!(warned);
    let (warned, _) = warns(&|doc, _| {
        let mut n = plain_rect(doc, "Out", vec![]);
        n.knockout_shape = true;
        linked(n, 8)
    });
    assert!(warned);
    // A group with a member the style can't hold.
    let (warned, _) = warns(&|doc, _| {
        let member = plain_rect(doc, "Member", vec![drop_shadow()]);
        linked(group(doc, vec![member], false), 8)
    });
    assert!(warned);
    // A group with an effect and a member linked to a style: the group keeps its effect, and the
    // member's link is left out.
    let (warned, back) = warns(&|doc, _| {
        let member = linked(plain_rect(doc, "Member", vec![]), 8);
        let mut g = group(doc, vec![member], false);
        g.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::default() }.into();
        g
    });
    assert!(warned);
    let g = &back.layers[0].children().unwrap()[0];
    assert_eq!(g.appearance.effects.len(), 1, "{g:?}");
    assert_eq!(g.children().unwrap()[0].extras().graphic_style, None);
    // A group whose link isn't written: its member keeps its own.
    let (warned, back) = warns(&|doc, id| {
        let plain = doc.graphic_style_by_id(id + 1).unwrap().appearance.clone();
        let member = linked(Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), plain), id + 1);
        linked(group(doc, vec![member], false), id)
    });
    assert!(warned);
    let g = &back.layers[0].children().unwrap()[0];
    assert_eq!(g.extras().graphic_style, None);
    let member = g.children().unwrap()[0].extras().graphic_style.unwrap();
    assert_eq!(back.graphic_style_by_id(member).map(|s| s.name.as_str()), Some("Plain"));
    // A link in a symbol's art.
    let (warned, _) = warns(&|doc, _| {
        let art = linked(square(doc, Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK), 8);
        doc.symbols.push(Symbol { dynamic: false, name: "S".into(), art: Arc::new(art) });
        Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((50.0, 20.0)) })
    });
    assert!(warned);
    // A link that is kept.
    let (warned, back) = warns(&|doc, id| {
        let plain = doc.graphic_style_by_id(id + 1).unwrap().appearance.clone();
        linked(Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), plain), id + 1)
    });
    assert!(!warned);
    let id = back.layers[0].children().unwrap()[0].extras().graphic_style.unwrap();
    assert_eq!(back.graphic_style_by_id(id).map(|g| g.name.as_str()), Some("Plain"));
}

/// A panel with no style written warns that a new document's styles come back.
#[test]
fn an_empty_graphic_styles_panel_warns() {
    let mut doc = Document::new(200.0, 100.0);
    doc.graphic_styles.clear();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PANEL_EMPTY), "{:?}", out.warnings);
    assert!(!shown(&out.program).contains("ArtStyle"));
    assert_eq!(read_back(&doc).graphic_styles, Document::new(1.0, 1.0).graphic_styles);
    // Every style left out.
    let stops = vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)];
    let graded = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    doc.graphic_styles.push(vectorcraft_doc::GraphicStyle::new("Graded", Appearance::basic(graded, Paint::None, 0.0)));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PANEL_EMPTY) && out.warnings.iter().any(|w| w == writer::PANEL_STYLES), "{:?}", out.warnings);
    // A new document's styles aren't written and don't warn.
    let out = export_ai_editing(&Document::new(200.0, 100.0), &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::PANEL_EMPTY), "{:?}", out.warnings);
}

/// A second style of one name is left out with a warning, and an object linked to it has no link.
#[test]
fn a_graphic_style_of_a_name_another_has_is_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let style = |id: u32, c: Color| {
        let mut g = vectorcraft_doc::GraphicStyle::new("Twin", Appearance::basic(solid(c), Paint::None, 0.0));
        g.id = id;
        g
    };
    *doc.graphic_styles = vec![style(1, Color::BLACK), style(2, Color::WHITE)];
    let mut n = styled_rect(&mut doc, "Second", Rect::new(10.0, 10.0, 50.0, 40.0), solid(Color::WHITE), Paint::None, vec![]);
    n.extras_mut().graphic_style = Some(2);
    put(&mut doc, n);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::PANEL_STYLES) && out.warnings.iter().any(|w| w == writer::STYLE_LINKS), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.graphic_styles.len(), 1);
    assert_eq!(back.graphic_styles[0].appearance.fill().and_then(|f| f.paint.color()), Some(Color::BLACK));
    assert_eq!(back.layers[0].children().unwrap()[0].extras().graphic_style, None);
}

/// A path or compound path with no points writes nothing, with or without a style; a group with no
/// members keeps its effects.
#[test]
fn objects_with_nothing_to_write_are_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let mut empty = plain_rect(&mut doc, "Empty", vec![drop_shadow()]);
    if let NodeKind::Path { path, .. } = &mut empty.kind {
        path.subpaths.clear();
    }
    put(&mut doc, empty);
    let mut hollow = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![], rule: FillRule::NonZero });
    hollow.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::basic(black_fill(), Paint::None, 0.0) }.into();
    put(&mut doc, named(hollow, "Hollow"));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    let p = shown(&out.program);
    assert!(!p.contains("(Anon 1)") && !p.contains("(Empty)") && !p.contains("6 () XW"), "{p}");
    assert_eq!(read_back(&doc).layers[0].children().unwrap().len(), 0);

    let (n, ..) = read_one(|doc| {
        let mut g = group(doc, vec![], false);
        g.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::default() }.into();
        named(g, "Bare")
    });
    assert_eq!((n.name.as_deref(), n.appearance.effects.len(), n.children().map(Vec::len)), (Some("Bare"), 1, Some(0)));
}

/// Outline Object, every Convert to Shape form and size, Offset Path's joins, Inner Glow's sources
/// and a stroke's width profile, each on an object, read back as they are. Drop Shadow and the glows
/// keep their colors, in CMYK too.
#[test]
fn every_effect_setting_written_on_an_object_is_read_back() {
    use serde_json::json;
    let cases = [
        ("path.outlineObject", json!({})),
        ("path.outlineStroke", json!({})),
        ("convertToShape.rectangle", json!({"relative": true, "extraW": 6.0, "extraH": 4.0})),
        ("convertToShape.rectangle", json!({"relative": false, "width": 30.0, "height": 20.0})),
        ("convertToShape.roundedRectangle", json!({"relative": false, "width": 30.0, "height": 20.0, "radius": 3.0})),
        ("convertToShape.roundedRectangle", json!({"relative": true, "extraW": 6.0, "extraH": 4.0, "radius": 3.0})),
        ("convertToShape.ellipse", json!({"relative": false, "width": 30.0, "height": 20.0})),
        ("path.offsetPath", json!({"offset": 3.0, "joins": "round", "miterLimit": 4.0})),
        ("path.offsetPath", json!({"offset": -3.0, "joins": "bevel", "miterLimit": 4.0})),
        ("path.offsetPath", json!({"offset": 3.0, "joins": "miter", "miterLimit": 9.0})),
        ("stylize.innerGlow", json!({"source": "edge", "blur": 2.5, "opacity": 40.0, "mode": "screen", "color": "#102030"})),
        ("stylize.innerGlow", json!({"source": "center", "blur": 2.5})),
        ("stylize.outerGlow", json!({"blur": 7.5, "opacity": 33.333, "mode": "normal", "color": "#a0b0c0"})),
        ("stylize.dropShadow", json!({"x": 1.25, "y": -2.5, "blur": 3.5, "opacity": 12.5, "mode": "overlay", "color": "#405060"})),
        ("stylize.dropShadow", json!({"shade": "darkness", "darkness": 62.5, "x": 0.0, "y": 0.0, "blur": 0.0})),
        ("stylize.feather", json!({"radius": 11.5})),
        ("stylize.roundCorners", json!({"radius": 0.5})),
        (
            "stylize.scribble",
            json!({"angle": 15.0, "overlap": 2.0, "overlapVariation": 1.0, "strokeWidth": 1.5, "curviness": 12.5, "curvinessVariation": 7.5, "spacing": 4.0, "spacingVariation": 1.5}),
        ),
    ];
    for (id, params) in cases {
        let effect = fx(id, params.clone());
        let (n, warnings, p) = read_one(|doc| plain_rect(doc, "One", vec![effect.clone()]));
        assert!(!warnings.iter().any(|w| w == writer::EFFECTS), "{id} {params}: {warnings:?}\n{p}");
        assert_eq!(n.appearance.effects.len(), 1, "{id} {params}");
        let (got, want) = (&n.appearance.effects[0], &effect);
        assert_eq!(got.id, want.id);
        let (g, w) = (vectorcraft_effects::merged_params(&got.id, &got.params), vectorcraft_effects::merged_params(&want.id, &want.params));
        for (key, value) in w.as_object().unwrap() {
            let read = g.get(key).unwrap_or(&serde_json::Value::Null);
            match (key.as_str(), value.as_f64()) {
                // A case the file doesn't keep: `seed` and the like, which the reader fills in as their defaults.
                ("mode" | "shade" | "source" | "joins", _) => {
                    let lower = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_ascii_lowercase();
                    assert_eq!(lower(read), lower(value), "{id} {key}");
                }
                ("color", _) => {
                    let color = |p: &serde_json::Value| vectorcraft_effects::color_param(p, Color::BLACK).to_hex();
                    assert_eq!(color(&g), color(&w), "{id}");
                }
                (_, Some(v)) => assert!((read.as_f64().unwrap() - v).abs() < 6e-3, "{id} {key}: {read} for {v}"),
                _ => assert_eq!(read, value, "{id} {key}"),
            }
        }
    }
    // A relative Ellipse: the reader keeps the drawn look, which is the effect's result.
    let effect = fx("convertToShape.ellipse", json!({"relative": true, "extraW": 6.0, "extraH": 4.0}));
    let (n, _, p) = read_one(|doc| plain_rect(doc, "One", vec![effect.clone()]));
    assert!(!p.contains("%AI9_BeginArtStyles"), "{p}");
    assert!(n.appearance.effects.is_empty());
    // A stroke's width profile.
    let profile = vectorcraft_doc::WidthProfile { points: vec![(0.0, 1.0, 0.5), (0.5, 0.25, 0.75), (1.0, 0.0, 1.0)] };
    let (n, warnings, p) = read_one(|doc| {
        let mut stroke = StrokeLayer::new(black_fill(), 4.0);
        stroke.profile = Some(profile.clone());
        let ap = Appearance { items: vec![AppearanceItem::Stroke(stroke)], effects: vec![drop_shadow()], contents_index: None };
        Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap)
    });
    assert!(!warnings.iter().any(|w| w == writer::EFFECTS), "{warnings:?}");
    assert!(p.contains("(BSVariableWidthTValuesArray)") && p.contains("(BSLeftWidthsArray)") && p.contains("(BSRightWidthsArray)"), "{p}");
    assert_eq!(n.appearance.stroke().and_then(|s| s.profile.clone()), Some(profile));
    // A CMYK document: a CMYK and a spot fill, a CMYK stroke, and an effect's CMYK color stay as they are.
    let mut doc = Document::new(200.0, 100.0);
    doc.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    let mut swatches = (*doc.swatches).clone();
    swatches.push(Swatch { name: "Ink".into(), paint: solid(INK), global: true, spot: true });
    swatches.push(Swatch { name: "Brand".into(), paint: solid(BRAND), global: true, spot: false });
    doc.swatches = swatches.into();
    let shadow = fx("stylize.dropShadow", json!({"color": serde_json::to_value(GOLD).unwrap()}));
    let n = styled_rect(&mut doc, "Process", Rect::new(10.0, 10.0, 40.0, 40.0), solid(GOLD), solid(BRAND), vec![shadow.clone()]);
    put(&mut doc, n);
    let n = styled_rect(&mut doc, "Spot", Rect::new(50.0, 10.0, 80.0, 40.0), linked("Ink", INK, 0.5), Paint::None, vec![shadow.clone()]);
    put(&mut doc, n);
    let n = styled_rect(&mut doc, "Global", Rect::new(90.0, 10.0, 120.0, 40.0), linked("Brand", BRAND, 1.0), Paint::None, vec![shadow]);
    put(&mut doc, n);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_styles(&back, &doc);
    for n in back.layers[0].children().unwrap() {
        assert!(matches!(vectorcraft_effects::color_param(&n.appearance.effects[0].params, Color::BLACK), Color::Cmyk { .. }), "{n:?}");
    }
    let spot = &back.layers[0].children().unwrap()[1];
    assert!(matches!(&spot.appearance.fill().unwrap().paint, Paint::Solid { swatch: Some(s), .. } if s == "Ink"), "{spot:?}");
}

/// A drawn look's strokes warn only where its art style leaves them out: a path with a drop shadow and
/// a stroke with a width profile is written with a style that holds the profile, and doesn't warn;
/// a group with a drop shadow whose member has a stroke inside the path still warns.
#[test]
fn a_drawn_looks_strokes_warn_only_where_its_style_leaves_them_out() {
    let profile = vectorcraft_doc::WidthProfile { points: vec![(0.0, 1.0, 0.5), (0.5, 0.25, 0.75), (1.0, 0.0, 1.0)] };
    let (n, warnings, p) = read_one(|doc| {
        let mut stroke = StrokeLayer::new(black_fill(), 4.0);
        stroke.profile = Some(profile.clone());
        let ap = Appearance { items: vec![AppearanceItem::Stroke(stroke)], effects: vec![drop_shadow()], contents_index: None };
        Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap)
    });
    assert!(p.contains("%AI9_BeginArtStyles"), "{p}");
    assert!(!warnings.iter().any(|w| w == writer::STROKES), "{warnings:?}");
    assert_eq!(n.appearance.stroke().and_then(|s| s.profile.clone()), Some(profile));
    let (_, warnings, p) = read_one(|doc| {
        let mut n =
            Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), Appearance::basic(Paint::None, black_fill(), 2.0));
        n.appearance.stroke_mut().unwrap().align = vectorcraft_doc::StrokeAlign::Inside;
        let mut g = group(doc, vec![n], false);
        g.appearance.effects = vec![drop_shadow()];
        g
    });
    assert!(p.contains("%AI9_BeginArtStyles"), "{p}");
    assert!(warnings.iter().any(|w| w == writer::STROKES), "{warnings:?}");
}

/// A path linked to a graphic style with a drop shadow and a stroke with a width profile is written
/// with that style, which holds the profile, and doesn't warn; it reads back linked, with its
/// profile. A linked path whose fill is half opaque in Screen mode isn't written with its style, which
/// the reader reads on no object, and warns that the fill's transparency is left out.
#[test]
fn a_linked_paths_drawn_look_warns_only_for_what_its_style_leaves_out() {
    let profile = vectorcraft_doc::WidthProfile { points: vec![(0.0, 1.0, 0.5), (0.5, 0.25, 0.75), (1.0, 0.0, 1.0)] };
    let linked = |doc: &mut Document, fill: FillLayer| {
        let mut stroke = StrokeLayer::new(black_fill(), 4.0);
        stroke.profile = Some(profile.clone());
        let ap = Appearance {
            items: vec![AppearanceItem::Fill(fill), AppearanceItem::Stroke(stroke)],
            effects: vec![drop_shadow()],
            contents_index: None,
        };
        let mut g = vectorcraft_doc::GraphicStyle::new("Linked", ap.clone());
        g.id = 7;
        doc.graphic_styles.push(g);
        let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap);
        n.extras_mut().graphic_style = Some(7);
        n
    };
    let (n, warnings, p) = read_one(|doc| linked(doc, FillLayer::new(solid(Color::rgb(0.2, 0.2, 0.9)))));
    assert!(p.contains("1 (Linked) XW"), "{warnings:?}");
    assert!(!warnings.iter().any(|w| w == writer::STROKES), "{warnings:?}");
    assert!(n.extras().graphic_style.is_some(), "{n:?}");
    assert_eq!(n.appearance.stroke().and_then(|s| s.profile.clone()), Some(profile.clone()));
    let (_, warnings, p) = read_one(|doc| {
        let mut fill = FillLayer::new(solid(Color::rgb(0.2, 0.2, 0.9)));
        fill.opacity = 0.5;
        fill.blend = BlendMode::Screen;
        linked(doc, fill)
    });
    assert!(!p.contains("1 (Linked) XW"), "{p}");
    assert!(warnings.iter().any(|w| w == writer::ITEM_TRANSPARENCY), "{warnings:?}");
}

/// Each effect a graphic style can hold, in a style of its own, reads back with its settings, whatever
/// the writer's conversions to the file's units.
#[test]
fn every_effect_a_graphic_style_holds_is_read_back() {
    use serde_json::json;
    let mut cases = vec![
        (
            "distort.transform",
            json!({"scaleH": 50.0, "scaleV": 150.0, "moveH": 12.5, "moveV": -7.5, "rotate": 30.0, "copies": 2, "reference": 8, "reflectX": true, "objects": false, "strokes": true, "patterns": true}),
        ),
        ("distort.freeDistort", json!({"corners": [[0.1, 0.05], [0.9, 0.1], [1.0, 1.0], [0.0, 0.8]]})),
        ("distort.puckerBloat", json!({"amount": -40.0})),
        ("distort.puckerBloat", json!({"amount": 60.0})),
        ("distort.twist", json!({"angle": -45.0})),
        ("distort.zigZag", json!({"size": 7.5, "relative": false, "ridges": 6, "points": "corner"})),
        ("distort.zigZag", json!({"size": 25.0, "relative": true, "ridges": 3, "points": "smooth"})),
        ("blur.gaussian", json!({"radius": 2.5})),
    ];
    let warps = [
        "arc",
        "arcLower",
        "arcUpper",
        "arch",
        "bulge",
        "shellLower",
        "shellUpper",
        "flag",
        "wave",
        "fish",
        "rise",
        "fisheye",
        "inflate",
        "squeeze",
        "twist",
    ];
    let ids: Vec<String> = warps.iter().map(|w| format!("warp.{w}")).collect();
    for id in &ids {
        cases.push((id, json!({"bend": 30.0, "horizontal": -10.0, "vertical": 20.0, "orientation": "vertical"})));
        cases.push((id, json!({"bend": -30.0, "horizontal": 0.0, "vertical": 0.0, "orientation": "horizontal"})));
        cases.push((id, json!({"bend": 45.0, "horizontal": 15.0, "vertical": -5.0, "orientation": "horizontal", "version": 1})));
    }
    for (id, params) in cases {
        let effect = fx(id, params.clone());
        let mut doc = Document::new(200.0, 100.0);
        let mut g = vectorcraft_doc::GraphicStyle::new(
            "Held",
            Appearance { effects: vec![effect.clone()], ..Appearance::basic(black_fill(), Paint::None, 0.0) },
        );
        g.id = 3;
        *doc.graphic_styles = vec![g];
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert!(!out.warnings.iter().any(|w| w == writer::PANEL_STYLES), "{id} {params}: {:?}", out.warnings);
        let back = read_back(&doc);
        let got = &back.graphic_styles[0].appearance.effects;
        assert_eq!(got.len(), 1, "{id} {params}");
        assert_eq!(got[0].id, effect.id);
        // A warp read from a file bends as its style's Bézier net.
        if id.starts_with("warp.") {
            assert_eq!(got[0].params["version"], json!(vectorcraft_doc::live::WARP_NET), "{id} {params}");
        }
        let (g, w) = (vectorcraft_effects::merged_params(&got[0].id, &got[0].params), vectorcraft_effects::merged_params(&effect.id, &effect.params));
        for (key, value) in w.as_object().unwrap() {
            // Random isn't written, and a style with it isn't read; its seed is the default.
            let read = g.get(key).unwrap_or(&serde_json::Value::Null);
            match value {
                serde_json::Value::Array(rows) => {
                    let got = read.as_array().unwrap();
                    assert_eq!(got.len(), rows.len(), "{id}");
                    for (r, w) in got.iter().zip(rows) {
                        for (r, w) in r.as_array().unwrap().iter().zip(w.as_array().unwrap()) {
                            assert!((r.as_f64().unwrap() - w.as_f64().unwrap()).abs() < 6e-3, "{id} {key}: {read} for {value}");
                        }
                    }
                }
                serde_json::Value::Number(v) => assert!((read.as_f64().unwrap() - v.as_f64().unwrap()).abs() < 6e-3, "{id} {key}: {read} for {v}"),
                serde_json::Value::String(v) => assert_eq!(read.as_str().map(str::to_ascii_lowercase), Some(v.to_ascii_lowercase()), "{id} {key}"),
                _ => assert_eq!(read, value, "{id} {key}"),
            }
        }
    }
    // A warp saved before the map version existed is written with its settings too, and reads back on
    // the nets' map.
    let old = vectorcraft_doc::Effect { id: "warp.flag".into(), params: json!({"bend": 40.0, "horizontal": 10.0}), visible: true };
    let mut doc = Document::new(200.0, 100.0);
    let mut g = vectorcraft_doc::GraphicStyle::new("Old", Appearance { effects: vec![old], ..Appearance::basic(black_fill(), Paint::None, 0.0) });
    g.id = 3;
    *doc.graphic_styles = vec![g];
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::PANEL_STYLES), "{:?}", out.warnings);
    let back = read_back(&doc);
    let [warp] = back.graphic_styles[0].appearance.effects.as_slice() else { panic!("{:?}", back.graphic_styles) };
    assert_eq!((warp.id.as_str(), &warp.params["bend"], &warp.params["horizontal"]), ("warp.flag", &json!(40.0), &json!(10.0)));
    assert_eq!(warp.params["version"], json!(vectorcraft_doc::live::WARP_NET));
    // A Zig Zag with its 0 to 1 settings written under the size's own key, and a turn in radians.
    let mut doc = Document::new(200.0, 100.0);
    let effects = vec![
        fx("distort.zigZag", json!({"size": 7.5, "relative": false, "ridges": 6, "points": "corner"})),
        fx("distort.transform", json!({"moveV": -7.5, "rotate": 30.0})),
    ];
    doc.graphic_styles.push(vectorcraft_doc::GraphicStyle::new("Forms", Appearance { effects, ..Appearance::default() }));
    let p = program_shown(&doc);
    assert!(p.contains("7.5 /Real (amount) ,") && p.contains("0 /Real (roundness) ,"), "{p}");
    assert!(p.contains("7.5 /Real (moveV_Pts) ,") && p.contains("0.5236 /Real (rotate_Radians) ,"), "{p}");
}

/// Every kind of appearance a style isn't written for is written as its drawn look, with a warning.
#[test]
fn appearances_written_as_their_drawn_look() {
    type Make = Box<dyn Fn(&mut Document) -> Node>;
    let cases: Vec<(&str, Make)> = vec![
        (
            "a pattern fill",
            Box::new(|doc| {
                let tile = square(doc, Rect::new(0.0, 0.0, 5.0, 5.0), Color::BLACK);
                let mut dots = vectorcraft_doc::PatternDef::new("Dots", vec![Arc::new(tile)]);
                dots.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
                doc.patterns.push(dots);
                let fill = Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY };
                styled_rect(doc, "P", Rect::new(10.0, 10.0, 50.0, 40.0), fill, Paint::None, vec![drop_shadow()])
            }),
        ),
        (
            "an overprinting fill",
            Box::new(|doc| {
                let ap = Appearance {
                    items: vec![AppearanceItem::Fill(FillLayer { overprint: true, ..FillLayer::new(black_fill()) })],
                    effects: vec![drop_shadow()],
                    contents_index: None,
                };
                Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap)
            }),
        ),
        (
            "an overprinting stroke",
            Box::new(|doc| {
                let ap = Appearance {
                    items: vec![AppearanceItem::Stroke(StrokeLayer { overprint: true, ..StrokeLayer::new(black_fill(), 2.0) })],
                    effects: vec![drop_shadow()],
                    contents_index: None,
                };
                Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap)
            }),
        ),
        (
            "a gradient stroke",
            Box::new(|doc| {
                let stops = vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)];
                let graded = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
                styled_rect(doc, "G", Rect::new(10.0, 10.0, 50.0, 40.0), Paint::None, graded, vec![drop_shadow()])
            }),
        ),
        (
            "a fill with an opacity of its own",
            Box::new(|doc| {
                let ap = Appearance {
                    items: vec![AppearanceItem::Fill(FillLayer { opacity: 0.5, ..FillLayer::new(black_fill()) })],
                    effects: vec![drop_shadow()],
                    contents_index: None,
                };
                Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap)
            }),
        ),
        (
            "a fill with an effect of its own",
            Box::new(|doc| {
                let fill = FillLayer { effects: vec![fx("stylize.feather", serde_json::json!({}))], ..FillLayer::new(black_fill()) };
                let ap = Appearance { items: vec![AppearanceItem::Fill(fill)], ..Appearance::default() };
                Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap)
            }),
        ),
        ("Pathfinder on a path", Box::new(|doc| plain_rect(doc, "P", vec![fx("pathfinder.add", serde_json::json!({}))]))),
        (
            "a group with a fill of its own",
            Box::new(|doc| {
                let member = square(doc, Rect::new(10.0, 10.0, 40.0, 40.0), Color::BLACK);
                let mut g = group(doc, vec![member], false);
                g.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::basic(solid(Color::WHITE), Paint::None, 0.0) }.into();
                g
            }),
        ),
        ("an effect the style doesn't hold", Box::new(|doc| plain_rect(doc, "B", vec![fx("blur.gaussian", serde_json::json!({}))]))),
        (
            "a group whose member's stroke is written as its outline",
            Box::new(|doc| {
                let mut stroke = StrokeLayer::new(solid(Color::BLACK), 4.0);
                stroke.profile = Some(vectorcraft_doc::WidthProfile { points: vec![(0.0, 1.0, 0.5), (0.5, 0.25, 0.75), (1.0, 0.0, 1.0)] });
                let ap = Appearance { items: vec![AppearanceItem::Stroke(stroke)], ..Appearance::default() };
                let member = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), ap);
                let mut g = group(doc, vec![member], false);
                let effects = vec![fx("stylize.roundCorners", serde_json::json!({})), drop_shadow()];
                g.appearance = Appearance { effects, ..Appearance::default() }.into();
                g
            }),
        ),
        (
            "a group whose effects reshape a member filled with pattern tiles",
            Box::new(|doc| {
                let big = square(doc, Rect::new(-2.0, -2.0, 6.0, 6.0), Color::rgb(1.0, 0.0, 0.0));
                let mut over = vectorcraft_doc::PatternDef::new("Over", vec![Arc::new(big)]);
                over.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
                doc.patterns.push(over);
                let tiles = Paint::Pattern { pattern: "Over".into(), xf: Affine::IDENTITY };
                let member =
                    Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)), Appearance::basic(tiles, Paint::None, 0.0));
                let mut g = group(doc, vec![member], false);
                let offset = fx("path.offsetPath", serde_json::json!({"offset": 3.0}));
                g.appearance = Appearance { effects: vec![offset, drop_shadow()], ..Appearance::default() }.into();
                g
            }),
        ),
        (
            "a group whose effects reshape a member with a gradient that fits it",
            Box::new(|doc| {
                let stops = vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)];
                let graded = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
                let member =
                    Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(30.0, 10.0, 100.0, 40.0)), Appearance::basic(graded, Paint::None, 0.0));
                let mut g = group(doc, vec![member], false);
                let offset = fx("path.offsetPath", serde_json::json!({"offset": 15.0}));
                g.appearance = Appearance { effects: vec![offset, drop_shadow()], ..Appearance::default() }.into();
                g
            }),
        ),
        (
            "Define Knockout Shape",
            Box::new(|doc| {
                let mut n = plain_rect(doc, "K", vec![drop_shadow()]);
                n.knockout_shape = true;
                n
            }),
        ),
    ];
    for (what, make) in cases {
        let mut doc = Document::new(200.0, 100.0);
        let n = make(&mut doc);
        put(&mut doc, n);
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{what}: {:?}", out.warnings);
        let p = shown(&out.program);
        assert!(!p.contains("%AI9_BeginArtStyles") && !p.contains("(Anon 1) XW"), "{what}:\n{p}");
        let back = read_back(&doc);
        let n = &back.layers[0].children().unwrap()[0];
        assert!(n.appearance.effects.is_empty(), "{what}: {n:?}");
    }
}

/// A link to a graphic style whose Transform moves the outline is kept on a path the reader reads
/// that style on, and left out with a warning on a path with an opacity mask, which keeps its mask.
#[test]
fn a_link_to_a_style_that_moves_the_outline_is_kept_where_the_reader_reads_it() {
    let make = |masked: bool| {
        let mut doc = Document::new(200.0, 100.0);
        let moved = fx("distort.transform", serde_json::json!({"copies": 0, "scaleH": 50.0}));
        let ap = Appearance { effects: vec![moved], ..Appearance::basic(black_fill(), Paint::None, 0.0) };
        let mut g = vectorcraft_doc::GraphicStyle::new("Moved", ap.clone());
        g.id = 9;
        doc.graphic_styles.push(g);
        let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap);
        n.extras_mut().graphic_style = Some(9);
        if masked {
            let art = square(&mut doc, Rect::new(10.0, 10.0, 50.0, 25.0), Color::gray(0.5));
            n.extras_mut().mask = Some(OpacityMask { art: Arc::new(art), clip: true, invert: false, disabled: false, linked: true }.into());
        }
        put(&mut doc, n);
        doc
    };
    let doc = make(false);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::STYLE_LINKS), "{:?}", out.warnings);
    let back = read_back(&doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert_eq!(n.extras().graphic_style.and_then(|id| back.graphic_style_by_id(id)).map(|g| g.name.as_str()), Some("Moved"));

    let doc = make(true);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::STYLE_LINKS), "{:?}", out.warnings);
    let back = read_back(&doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert_eq!(n.extras().graphic_style, None);
    assert!(n.extras().mask.is_some(), "{n:?}");
}

/// An object linked to a graphic style that doesn't fit within the object limit is left out with
/// the limit's warning and the warning about links left out.
#[test]
fn a_linked_object_past_the_object_limit_warns_that_its_link_is_left_out() {
    let mut doc = Document::new(200.0, 100.0);
    let ap = Appearance { effects: vec![drop_shadow()], ..Appearance::basic(black_fill(), Paint::None, 0.0) };
    let mut g = vectorcraft_doc::GraphicStyle::new("Shadowed", ap.clone());
    g.id = 9;
    doc.graphic_styles.push(g);
    // Room for everything but the object.
    let room = crate::import::objects_read(&export_ai_editing(&doc, &opts()).unwrap().program).unwrap();
    let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap);
    n.extras_mut().graphic_style = Some(9);
    put(&mut doc, n);
    let all = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!all.warnings.iter().any(|w| w == writer::OBJECT_LIMIT || w == writer::STYLE_LINKS), "{:?}", all.warnings);
    let out = export_within(&doc, &opts(), room).unwrap();
    for w in [writer::OBJECT_LIMIT, writer::STYLE_LINKS] {
        assert!(out.warnings.iter().any(|x| x == w), "{w}: {:?}", out.warnings);
    }
}

/// A link to a graphic style with Twist is kept where the reader reads Twist: on a curve, a curved
/// compound path or a group with nothing stroked up to a quarter turn, and on a path with straight
/// segments at any angle. A stroked curve, a group with a stroked member or a guide, or a curve,
/// compound path or group twisted further, is written without the link and warns.
#[test]
fn a_link_to_a_twist_style_is_kept_where_the_reader_reads_it() {
    // A document with the object `make` builds from its appearance, `ap` with Twist at `angle`,
    // linked to a graphic style of that appearance.
    fn twisted(angle: f64, ap: Appearance, make: impl FnOnce(&mut Document, Appearance) -> Node) -> Document {
        let mut doc = Document::new(200.0, 100.0);
        let ap = Appearance { effects: vec![fx("distort.twist", serde_json::json!({"angle": angle}))], ..ap };
        let mut g = vectorcraft_doc::GraphicStyle::new("Twisted", ap.clone());
        g.id = 9;
        doc.graphic_styles.push(g);
        let mut n = make(&mut doc, ap);
        n.extras_mut().graphic_style = Some(9);
        put(&mut doc, n);
        doc
    }
    let oval = || shapes::ellipse(Rect::new(10.0, 10.0, 90.0, 60.0));
    let square = || shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 60.0));
    let red = solid(Color::rgb(1.0, 0.0, 0.0));
    let filled = || Appearance::basic(black_fill(), Paint::None, 0.0);
    let path = |p: PathData| move |doc: &mut Document, ap: Appearance| Node::path(doc.alloc_id(), p, ap);
    let ring = |doc: &mut Document, ap: Appearance| {
        let outer = Node::path(doc.alloc_id(), oval(), Appearance::default());
        let inner = Node::path(doc.alloc_id(), shapes::ellipse(Rect::new(30.0, 20.0, 70.0, 50.0)), Appearance::default());
        let mut n = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(inner)], rule: FillRule::NonZero });
        n.appearance = ap.into();
        n
    };
    // A group of a filled square and `second`.
    let group = |second: fn(&mut Document) -> Node| {
        move |doc: &mut Document, ap: Appearance| {
            let first = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), filled());
            let second = second(doc);
            let mut n = Node::group(doc.alloc_id(), vec![Arc::new(first), Arc::new(second)]);
            n.appearance = ap.into();
            n
        }
    };
    let curve = |doc: &mut Document| {
        Node::path(doc.alloc_id(), shapes::ellipse(Rect::new(50.0, 10.0, 90.0, 60.0)), Appearance::basic(black_fill(), Paint::None, 0.0))
    };
    let stroked_curve = |doc: &mut Document| {
        let ap = Appearance::basic(black_fill(), solid(Color::rgb(1.0, 0.0, 0.0)), 2.0);
        Node::path(doc.alloc_id(), shapes::ellipse(Rect::new(50.0, 10.0, 90.0, 60.0)), ap)
    };
    let guide = |doc: &mut Document| {
        let path = shapes::rectangle(Rect::new(50.0, 10.0, 90.0, 60.0));
        Node::new(doc.alloc_id(), NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: true })
    };
    for (what, doc, kept) in [
        ("a twisted curve", twisted(45.0, filled(), path(oval())), true),
        ("a curve twisted a quarter turn back", twisted(-90.0, filled(), path(oval())), true),
        ("a square twisted three quarters of a turn", twisted(270.0, filled(), path(square())), true),
        ("a curve twisted past a quarter turn", twisted(120.0, filled(), path(oval())), false),
        ("a stroked curve", twisted(45.0, Appearance::basic(black_fill(), red.clone(), 2.0), path(oval())), false),
        ("a curved compound path", twisted(-60.0, filled(), ring), true),
        ("a curved compound path twisted past a quarter turn", twisted(120.0, filled(), ring), false),
        ("a twisted group", twisted(90.0, Appearance::default(), group(curve)), true),
        ("a group twisted past a quarter turn", twisted(135.0, Appearance::default(), group(curve)), false),
        ("a group with a stroked member", twisted(45.0, Appearance::default(), group(stroked_curve)), false),
        ("a group with a guide", twisted(45.0, Appearance::default(), group(guide)), false),
    ] {
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert_eq!(out.warnings.iter().any(|w| w == writer::STYLE_LINKS), !kept, "{what}: {:?}", out.warnings);
        let back = read_back(&doc);
        let n = &back.layers[0].children().unwrap()[0];
        let style = n.extras().graphic_style.and_then(|id| back.graphic_style_by_id(id)).map(|g| g.name.as_str());
        assert_eq!(style, kept.then_some("Twisted"), "{what}: {n:?}");
        assert_eq!(n.appearance.effects.iter().any(|e| e.id == "distort.twist"), kept, "{what}: {n:?}");
    }
}

/// Twist and Transform on an object that isn't linked to a graphic style are not written as a style
/// of its own: the object reads back without live effects.
#[test]
fn distort_effects_on_an_unlinked_object_write_no_style() {
    let cases = [fx("distort.twist", serde_json::json!({"angle": 30.0})), fx("distort.transform", serde_json::json!({"copies": 0, "moveH": 5.0}))];
    for e in cases {
        let what = e.id.clone();
        let mut doc = Document::new(200.0, 100.0);
        let n = plain_rect(&mut doc, "D", vec![e]);
        put(&mut doc, n);
        let p = shown(&export_ai_editing(&doc, &opts()).unwrap().program);
        assert!(!p.contains("%AI9_BeginArtStyles") && !p.contains("(Anon 1) XW"), "{what}:\n{p}");
        let back = read_back(&doc);
        let n = &back.layers[0].children().unwrap()[0];
        assert!(n.appearance.effects.is_empty(), "{what}: {n:?}");
    }
}

/// A gradient fill that fits its object is placed on the outline its geometry effects draw: an
/// object with Offset Path reads back with the gradient a plain object of the drawn size has.
#[test]
fn a_styled_objects_gradient_fits_its_drawn_outline() {
    let stops = || vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)];
    let graded = || Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops()))));
    let first = |doc: &Document| doc.layers[0].children().unwrap()[0].clone();
    let mut doc = Document::new(200.0, 100.0);
    let offset = fx("path.offsetPath", serde_json::json!({"offset": 20.0, "joins": "miter"}));
    let n = styled_rect(&mut doc, "G", Rect::new(50.0, 30.0, 100.0, 60.0), graded(), Paint::None, vec![offset]);
    put(&mut doc, n);
    let styled = first(&read_back(&doc));
    assert_eq!(styled.appearance.effects.len(), 1);
    let mut doc = Document::new(200.0, 100.0);
    let n = styled_rect(&mut doc, "P", Rect::new(30.0, 10.0, 120.0, 80.0), graded(), Paint::None, vec![]);
    put(&mut doc, n);
    let plain = first(&read_back(&doc));
    let geom = |n: &Node| match &n.appearance.fill().unwrap().paint {
        Paint::Gradient(g) => {
            let g = g.geom.unwrap();
            format!("{:.2} {:.2} {:.2} {:.2} {:.2}", g.start.x, g.start.y, g.end.x, g.end.y, g.aspect)
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(geom(&styled), geom(&plain));
}

/// The effects of a pattern's tile art are its own: a styled group's drawn look that first uses the
/// pattern still warns that the tile's Gaussian Blur is left out.
#[test]
fn effects_in_a_pattern_a_styled_look_uses_are_warned_about() {
    let mut doc = Document::new(200.0, 100.0);
    let dot = styled_rect(
        &mut doc,
        "Dot",
        Rect::new(2.0, 2.0, 8.0, 8.0),
        solid(Color::BLACK),
        Paint::None,
        vec![fx("blur.gaussian", serde_json::json!({}))],
    );
    let mut grid = vectorcraft_doc::PatternDef::new("Grid", vec![Arc::new(dot)]);
    grid.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
    doc.patterns.push(grid);
    let tiles = Paint::Pattern { pattern: "Grid".into(), xf: Affine::IDENTITY };
    let member = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)), Appearance::basic(tiles, Paint::None, 0.0));
    let mut g = group(&mut doc, vec![member], false);
    g.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::default() }.into();
    put(&mut doc, g);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    let p = shown(&out.program);
    assert!(p.contains("%AI3_BeginPattern") && p.contains("\n1 (Anon 1) XW\n"), "{p}");
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
}

/// A style that isn't written defines no gradient: a hidden gradient fill that only the style
/// would have held leaves no gradient the art uses.
#[test]
fn a_style_that_isnt_written_defines_no_gradient() {
    let mut doc = Document::new(200.0, 100.0);
    let stops = vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)];
    let graded = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let ap = Appearance {
        items: vec![
            AppearanceItem::Fill(FillLayer { visible: false, ..FillLayer::new(graded) }),
            AppearanceItem::Stroke(StrokeLayer::new(solid(Color::BLACK), 2.0)),
        ],
        effects: vec![fx("blur.gaussian", serde_json::json!({}))],
        contents_index: None,
    };
    let n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0)), ap);
    put(&mut doc, n);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    let p = shown(&out.program);
    assert!(!p.contains("(Anon 1) XW"), "{p}");
    assert!(p.contains("%%BeginSetup\n0 Bn\n"), "{p}");
}

/// An object in a plain group, a sublayer or a clipping group by a path keeps its style there. One
/// in a symbol's art, in an opacity mask or in art cut to a clipping object other than a path is
/// written as its drawn look, with a warning.
#[test]
fn where_a_styled_object_keeps_its_style() {
    let inside = |doc: &mut Document| plain_rect(doc, "Inside", vec![drop_shadow()]);
    let found = |doc: &Document| {
        fn find(nodes: &[Arc<Node>]) -> Option<&Arc<Node>> {
            nodes.iter().find_map(|n| if n.name.as_deref() == Some("Inside") { Some(n) } else { n.children().and_then(|c| find(c)) })
        }
        find(&doc.layers).cloned()
    };
    // In a plain group.
    let mut doc = Document::new(200.0, 100.0);
    let n = inside(&mut doc);
    let g = group(&mut doc, vec![n], false);
    put(&mut doc, g);
    assert_eq!(found(&read_back(&doc)).unwrap().appearance.effects.len(), 1);
    same_styles(&read_back(&doc), &doc);
    // In a sublayer.
    let mut doc = Document::new(200.0, 100.0);
    let n = inside(&mut doc);
    let mut sub = Node::layer(doc.alloc_id(), "Sub", LayerColor::Preset(1));
    sub.children_mut().unwrap().push(Arc::new(n));
    put(&mut doc, sub);
    assert_eq!(found(&read_back(&doc)).unwrap().appearance.effects.len(), 1);
    // In a clipping group.
    let mut doc = Document::new(200.0, 100.0);
    let clip = Node::new(
        doc.alloc_id(),
        NodeKind::Path {
            path: shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 60.0)),
            rule: FillRule::NonZero,
            live: None,
            clipping: true,
            guide: false,
        },
    );
    let n = inside(&mut doc);
    let g = group(&mut doc, vec![clip, n], true);
    put(&mut doc, g);
    assert_eq!(found(&read_back(&doc)).unwrap().appearance.effects.len(), 1);
    // In a symbol's art: the instance keeps the symbol, the effects are left out.
    let mut doc = Document::new(200.0, 100.0);
    let n = inside(&mut doc);
    doc.symbols.push(Symbol { dynamic: false, name: "S".into(), art: Arc::new(n) });
    let instance = Node::new(doc.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((50.0, 20.0)) });
    put(&mut doc, instance);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    assert!(!shown(&out.program).contains("(Anon 1) XW"));
    // In an opacity mask.
    let mut doc = Document::new(200.0, 100.0);
    let art = inside(&mut doc);
    let mut n = square(&mut doc, Rect::new(10.0, 10.0, 60.0, 60.0), Color::BLACK);
    n.extras_mut().mask = Some(OpacityMask { art: Arc::new(art), clip: true, invert: false, disabled: false, linked: true }.into());
    put(&mut doc, n);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    assert!(!shown(&out.program).contains("(Anon 1) XW"));
    // In art cut to a clipping object other than a path: type.
    let mut doc = Document::new(200.0, 100.0);
    let t = vectorcraft_doc::TextObject::point(
        vectorcraft_geom::Point::new(10.0, 70.0),
        "Type",
        vectorcraft_doc::CharStyle { size: 18.0, fill: solid(Color::BLACK), ..vectorcraft_doc::CharStyle::default() },
    );
    let t = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    let n = inside(&mut doc);
    let g = group(&mut doc, vec![t, n], true);
    put(&mut doc, g);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    assert!(!shown(&out.program).contains("(Anon 1) XW"));
}

/// A styled group's members of every kind allowed come back as they were: a compound path, a named
/// path, a hidden and a locked path, a nested group and a clipping group.
#[test]
fn a_styled_groups_members_of_each_kind_are_read_back() {
    let mut doc = Document::new(200.0, 100.0);
    let solid_ap = Appearance::basic(solid(Color::rgb(0.2, 0.4, 0.6)), Paint::None, 0.0);
    let compound = named(ring(&mut doc, solid_ap.clone()), "Compound");
    let plain = named(square(&mut doc, Rect::new(10.0, 10.0, 30.0, 30.0), Color::rgb(0.8, 0.1, 0.1)), "Named");
    let mut quiet = named(square(&mut doc, Rect::new(30.0, 10.0, 50.0, 30.0), Color::rgb(0.1, 0.8, 0.1)), "Hidden");
    quiet.visible = false;
    let mut fixed = named(square(&mut doc, Rect::new(50.0, 10.0, 70.0, 30.0), Color::rgb(0.1, 0.1, 0.8)), "Locked");
    fixed.locked = true;
    let a = square(&mut doc, Rect::new(10.0, 40.0, 30.0, 60.0), Color::BLACK);
    let nested = named(group(&mut doc, vec![a], false), "Nested");
    let clip = Node::new(
        doc.alloc_id(),
        NodeKind::Path {
            path: shapes::rectangle(Rect::new(40.0, 40.0, 60.0, 60.0)),
            rule: FillRule::NonZero,
            live: None,
            clipping: true,
            guide: false,
        },
    );
    let b = square(&mut doc, Rect::new(40.0, 40.0, 80.0, 80.0), Color::WHITE);
    let cut = named(group(&mut doc, vec![clip, b], true), "Cut");
    let mut g = named(group(&mut doc, vec![compound, plain, quiet, fixed, nested, cut], false), "Holder");
    g.appearance = Appearance { effects: vec![drop_shadow()], ..Appearance::default() }.into();
    put(&mut doc, g);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::EFFECTS), "{:?}", out.warnings);
    let back = read_back(&doc);
    same_styles(&back, &doc);
    let members = |d: &Document| {
        let mut out = vec![];
        view(d.layers[0].children().unwrap()[0].children().unwrap(), 0, &mut out);
        out
    };
    same_lines(&members(&back), &members(&doc));
}

/// What compares two appearances for the writer's checks and for these tests tells them apart.
#[test]
fn same_appearance_tells_appearances_apart() {
    use serde_json::json;
    let base = || Appearance {
        items: vec![
            AppearanceItem::Fill(FillLayer::new(solid(Color::rgb(0.5, 0.25, 0.75)))),
            AppearanceItem::Stroke(StrokeLayer::new(black_fill(), 2.0)),
        ],
        effects: vec![fx("stylize.dropShadow", json!({"x": 10.0, "mode": "Multiply", "color": "#102030"}))],
        contents_index: None,
    };
    let same = writer::same_appearance;
    assert!(same(&base(), &base()));
    // Within the written precision, a mode in another case and a color in another form.
    let mut other = base();
    other.effects[0].params["x"] = json!(10.005);
    other.effects[0].params["mode"] = json!("multiply");
    other.effects[0].params["color"] = serde_json::to_value(Color::from_hex("#102030").unwrap()).unwrap();
    assert!(same(&other, &base()));
    // A fill or stroke that doesn't paint.
    let mut other = base();
    other.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None)));
    assert!(same(&other, &base()));
    type Change = Box<dyn Fn(&mut Appearance)>;
    let change: Vec<Change> = vec![
        Box::new(|a| a.effects[0].params["x"] = json!(10.01)),
        Box::new(|a| a.effects[0].params["color"] = json!("#102031")),
        Box::new(|a| a.effects[0].params["mode"] = json!("screen")),
        Box::new(|a| a.effects[0].visible = false),
        Box::new(|a| a.effects[0].id = "stylize.outerGlow".into()),
        Box::new(|a| a.effects.clear()),
        Box::new(|a| a.items.truncate(1)),
        Box::new(|a| a.items.swap(0, 1)),
        Box::new(|a| a.contents_index = Some(2)),
        Box::new(|a| {
            if let AppearanceItem::Fill(f) = &mut a.items[0] {
                f.paint = solid(Color::rgb(0.5, 0.25, 0.76));
            }
        }),
        Box::new(|a| {
            if let AppearanceItem::Fill(f) = &mut a.items[0] {
                f.opacity = 0.5;
            }
        }),
        Box::new(|a| {
            if let AppearanceItem::Stroke(s) = &mut a.items[1] {
                s.width = 2.5;
            }
        }),
    ];
    for (i, f) in change.iter().enumerate() {
        let mut other = base();
        f(&mut other);
        assert!(!same(&other, &base()), "change {i}");
    }
}

/// A gradient fill in a style is placed as on an object without a style: a turned linear gradient
/// and a radial one with a focal point, and on a first artboard away from the origin among several.
#[test]
fn a_styled_objects_gradient_is_placed_as_a_plain_objects_is() {
    use vectorcraft_color::gradient::GradientGeom;
    let stops = || vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::rgb(0.1, 0.2, 0.8))];
    let turned = GradientGeom { start: (120.0, 60.0).into(), end: (170.0, 90.0).into(), aspect: 1.0, focal: None };
    let mut focal = GradientGeom { start: (140.0, 70.0).into(), end: (170.0, 70.0).into(), aspect: 0.5, focal: None };
    focal.set_focal(Some((150.0, 65.0).into()));
    let boards = [Rect::new(0.0, 0.0, 300.0, 200.0), Rect::new(100.0, 50.0, 400.0, 250.0)];
    for (kind, geom) in [(GradientKind::Linear, turned), (GradientKind::Radial, focal)] {
        for board in boards {
            let paint = gradient_paint(Gradient::new(kind, stops()), Some(geom), None);
            let make = |effects: Vec<vectorcraft_doc::Effect>| {
                let mut doc = Document::new(300.0, 200.0);
                doc.artboards[0].rect = board;
                let second = doc.artboards[0].clone();
                doc.artboards.push(Artboard { rect: board + vectorcraft_geom::Vec2::new(400.0, 0.0), ..second });
                let n = styled_rect(&mut doc, "G", Rect::new(120.0, 60.0, 180.0, 100.0), paint.clone(), Paint::None, effects);
                put(&mut doc, n);
                doc
            };
            let (with, without) = (read_back(&make(vec![drop_shadow()])), read_back(&make(vec![])));
            let (a, b) = (&with.layers[0].children().unwrap()[0], &without.layers[0].children().unwrap()[0]);
            assert_eq!(a.appearance.effects.len(), 1);
            let geom = |n: &Node| match &n.appearance.fill().unwrap().paint {
                Paint::Gradient(g) => {
                    let g = g.geom.unwrap();
                    format!(
                        "{:.2} {:.2} {:.2} {:.2} {:.2} {:?}",
                        g.start.x,
                        g.start.y,
                        g.end.x,
                        g.end.y,
                        g.aspect,
                        g.focal.map(|f| (r3(f.x), r3(f.y)))
                    )
                }
                other => panic!("{other:?}"),
            };
            assert_eq!(geom(a), geom(b));
            same_lines(&outlines(&with), &outlines(&without));
        }
    }
}

/// On a large canvas a document with only graphic styles, an object linked to one and an object with
/// an effect all warn, and the object comes back as its drawn look at its place.
#[test]
fn a_large_canvas_writes_every_object_as_its_look() {
    let warns = |doc: &Document| export_ai_editing(doc, &opts()).unwrap().warnings.iter().any(|w| w == writer::LARGE_CANVAS_STYLES);
    let mut styles_only = Document::new(20000.0, 100.0);
    styles_only.graphic_styles.push(vectorcraft_doc::GraphicStyle::new("Mine", Appearance::basic(black_fill(), Paint::None, 0.0)));
    assert!(warns(&styles_only));
    assert_eq!(read_back(&styles_only).graphic_style("Mine"), None);
    let mut plain = Document::new(20000.0, 100.0);
    let n = plain_rect(&mut plain, "None", vec![]);
    put(&mut plain, n);
    assert!(!warns(&plain));
    let mut linked_doc = Document::new(20000.0, 100.0);
    let id = linked_doc.graphic_style("Sunshine").unwrap().id;
    let mut n = plain_rect(&mut linked_doc, "Linked", vec![]);
    n.extras_mut().graphic_style = Some(id);
    put(&mut linked_doc, n);
    assert!(warns(&linked_doc));
    assert_eq!(read_back(&linked_doc).layers[0].children().unwrap()[0].extras().graphic_style, None);
    let mut shaded = Document::new(20000.0, 100.0);
    let n = plain_rect(&mut shaded, "Shaded", vec![drop_shadow()]);
    put(&mut shaded, n);
    assert!(warns(&shaded));
    let back = read_back(&shaded);
    let n = &back.layers[0].children().unwrap()[0];
    assert!(n.appearance.effects.is_empty());
    let p = n.path_data().unwrap().subpaths[0].anchors[0].p;
    assert!((p.x - 10.0).abs() < 1e-2 && (p.y - 10.0).abs() < 1e-2, "{p:?}");
}

// ---------- brushes ----------

/// Brush art: a wide block with a narrow one below it (y down), filled black.
fn brush_art_node() -> Node {
    let block =
        |id: u64, r: Rect| Arc::new(Node::path(vectorcraft_doc::NodeId(id), shapes::rectangle(r), Appearance::basic(black_fill(), Paint::None, 0.0)));
    Node::group(vectorcraft_doc::NodeId(1), vec![block(2, Rect::new(0.0, 0.0, 40.0, 4.0)), block(3, Rect::new(0.0, 4.0, 10.0, 8.0))])
}

/// A Calligraphic brush varying with the barrel's rotation and the pen's pressure, a Scatter brush in
/// Tints, an Art brush in Tints and one that keeps its art's colors.
fn brush_library() -> Vec<vectorcraft_brush::Brush> {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind, Calligraphic, Colorization, Scatter, Variation};
    let art = |colorization| BrushKind::Art(ArtBrush { art: brush_art_node(), colorization, ..ArtBrush::default() });
    vec![
        Brush {
            name: "Nib".into(),
            kind: BrushKind::Calligraphic(Calligraphic {
                angle: 30.0,
                roundness: 40.0,
                size: 6.0,
                variation: [20.0, 10.0, 2.0],
                modes: Some([Variation::Rotation, Variation::Fixed, Variation::Pressure]),
            }),
        },
        Brush {
            name: "Spray".into(),
            kind: BrushKind::Scatter(Scatter {
                art: brush_art_node(),
                size: (50.0, 150.0),
                spacing: (120.0, 200.0),
                scatter: (-25.0, 25.0),
                rotation: (-45.0, 30.0),
                rotation_relative_to_path: true,
                colorization: Colorization::Tints,
                modes: Some([Variation::Random, Variation::Fixed, Variation::Pressure, Variation::Random]),
            }),
        },
        Brush { name: "Ribbon".into(), kind: art(Colorization::Tints) },
        Brush { name: "Plain ribbon".into(), kind: art(Colorization::None) },
    ]
}

/// A path stroked red at `width` with the brush `brush`, on the first layer of `doc`.
fn brushed(doc: &mut Document, brush: &str, from: (f64, f64), to: (f64, f64), width: f64) {
    let mut line =
        Node::path(doc.alloc_id(), shapes::line(from.into(), to.into()), Appearance::basic(Paint::None, solid(Color::rgb(1.0, 0.0, 0.0)), width));
    line.appearance.stroke_mut().unwrap().brush = Some(brush.into());
    put(doc, line);
}

/// [`brush_library`] and a path with the Calligraphic brush and one with the Art brush in Tints.
fn brushes_document() -> Document {
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &brush_library());
    brushed(&mut doc, "Nib", (20.0, 50.0), (280.0, 60.0), 2.0);
    brushed(&mut doc, "Ribbon", (20.0, 120.0), (280.0, 150.0), 1.5);
    doc
}

/// Brush art `n` as a tree to compare with [`same_tree`]: each object's kind, its anchors and handles,
/// the fills and strokes that paint (a stroke with its width), and its members in order.
fn art_tree(n: &Node) -> serde_json::Value {
    let kind = match &n.kind {
        NodeKind::Path { .. } => "path",
        NodeKind::Compound { .. } => "compound",
        NodeKind::Group { clip: false, .. } => "group",
        _ => "other",
    };
    let anchors: Vec<[f64; 6]> = match &n.kind {
        NodeKind::Path { path, .. } => {
            path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| [a.h_in.x, a.h_in.y, a.p.x, a.p.y, a.h_out.x, a.h_out.y])).collect()
        }
        _ => vec![],
    };
    let paints: Vec<serde_json::Value> = n
        .appearance
        .items
        .iter()
        .filter(|i| !i.paint().is_none())
        .map(|i| match i {
            AppearanceItem::Fill(f) => serde_json::json!(["fill", f.paint]),
            AppearanceItem::Stroke(s) => serde_json::json!(["stroke", s.paint, s.width]),
        })
        .collect();
    let members: Vec<serde_json::Value> = n.children().map(|c| c.iter().map(|m| art_tree(m)).collect()).unwrap_or_default();
    serde_json::json!({"kind": kind, "anchors": anchors, "paints": paints, "members": members})
}

/// Are trees `a` and `b` ([`art_tree`]) the same but for numbers within the written numbers'
/// precision (1/1000)?
fn same_tree(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() <= 1e-3,
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_tree(p, q)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same_tree(v, w))),
        _ => a == b,
    }
}

/// The document's brushes come back in the Brushes panel's order with their settings and art: the
/// Calligraphic brush's channels, the Scatter brush's ranges, modes and Tints, the Art brushes'
/// colorizations.
#[test]
fn brushes_are_read_back_with_their_settings() {
    use vectorcraft_brush::BrushKind;
    let doc = brushes_document();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let text = program(&doc);
    assert_eq!(text.matches("%AI8_BeginPluginObject").count(), 5, "the manager's order and four brushes");
    assert_eq!(text.matches("%AI8_BeginBrushPattern").count(), 3);
    let back = vectorcraft_brush::library(&read_back(&doc));
    let lib = brush_library();
    assert_eq!(back.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Nib", "Spray", "Ribbon", "Plain ribbon"]);
    assert_eq!(back[0], lib[0]);
    let (BrushKind::Scatter(got), BrushKind::Scatter(want)) = (&back[1].kind, &lib[1].kind) else { panic!("scatter") };
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
    assert!(
        near(got.size, want.size) && near(got.spacing, want.spacing) && near(got.scatter, want.scatter) && near(got.rotation, want.rotation),
        "{got:?}"
    );
    assert_eq!((got.modes, got.colorization, got.rotation_relative_to_path), (want.modes, want.colorization, true));
    let (g, w) = (art_tree(&got.art), art_tree(&want.art));
    assert!(same_tree(&g, &w), "the art without its frame, with its paint and members in order: {g:#} for {w:#}");
    for (i, colorization) in [(2, vectorcraft_brush::Colorization::Tints), (3, vectorcraft_brush::Colorization::None)] {
        let BrushKind::Art(a) = &back[i].kind else { panic!("art") };
        assert_eq!(a.colorization, colorization);
        let (g, w) = (art_tree(&a.art), art_tree(&brush_art_node()));
        assert!(same_tree(&g, &w), "the narrow block stays below the wide one, and no frame stays: {g:#} for {w:#}");
        let BrushKind::Art(want) = &lib[i].kind else { panic!("art") };
        assert_eq!(ArtBrushOptions::of(a), ArtBrushOptions::of(want));
    }
}

/// An Art brush's options, without its art.
#[derive(Debug, PartialEq)]
struct ArtBrushOptions(serde_json::Value);

impl ArtBrushOptions {
    fn of(a: &vectorcraft_brush::ArtBrush) -> Self {
        let mut v = serde_json::to_value(a).unwrap();
        v.as_object_mut().unwrap().remove("art");
        Self(v)
    }
}

/// A path with the Calligraphic brush, and one with an Art brush in Tints whose art bends along the
/// path, come back as those paths with the brush on their stroke at its weight, not as their drawn
/// look; a path with the Art brush that keeps its art's colors comes back as its drawn look.
#[test]
fn brushed_paths_are_read_back_brushed() {
    let mut doc = brushes_document();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::STROKES), "{:?}", out.warnings);
    let back = read_back(&doc);
    let art = back.layers[0].children().unwrap();
    assert_eq!(art.len(), 2);
    for (n, (brush, width)) in art.iter().zip([("Nib", 2.0), ("Ribbon", 1.5)]) {
        assert!(matches!(n.kind, NodeKind::Path { .. }), "{brush}: the path, not its look: {n:#?}");
        let st = n.appearance.stroke().unwrap();
        assert_eq!((st.brush.as_deref(), st.width, st.paint.color()), (Some(brush), width, Some(Color::rgb(1.0, 0.0, 0.0))));
        assert!(n.appearance.fill().is_none_or(|f| f.paint.is_none()));
    }
    let ours = doc.layers[0].children().unwrap();
    assert_eq!(art[0].path_data(), ours[0].path_data());
    assert_eq!(art[1].path_data(), ours[1].path_data());
    brushed(&mut doc, "Plain ribbon", (20.0, 180.0), (280.0, 180.0), 1.0);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::STROKES), "{:?}", out.warnings);
    assert!(matches!(read_back(&doc).layers[0].children().unwrap()[2].kind, NodeKind::Group { .. }));
}

/// A compound path with the Calligraphic brush comes back as that compound path with the brush on its
/// stroke; a filled ellipse with it comes back with its fill and the brush.
#[test]
fn brushed_compound_and_filled_paths_are_read_back_brushed() {
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &brush_library());
    let mut stroke = StrokeLayer::new(solid(Color::rgb(1.0, 0.0, 0.0)), 2.0);
    stroke.brush = Some("Nib".into());
    let ring = ring(&mut doc, Appearance { items: vec![AppearanceItem::Stroke(stroke.clone())], effects: vec![], contents_index: None });
    put(&mut doc, ring);
    let fill = vectorcraft_doc::FillLayer::new(solid(Color::rgb(0.2, 0.6, 0.3)));
    let ap = Appearance { items: vec![AppearanceItem::Fill(fill), AppearanceItem::Stroke(stroke)], effects: vec![], contents_index: None };
    let ellipse = Node::path(doc.alloc_id(), shapes::ellipse(Rect::new(20.0, 120.0, 140.0, 180.0)), ap);
    put(&mut doc, ellipse);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::STROKES || w == writer::BRUSHES), "{:?}", out.warnings);
    let back = read_back(&doc);
    let (art, ours) = (back.layers[0].children().unwrap(), doc.layers[0].children().unwrap());
    assert!(matches!(art[0].kind, NodeKind::Compound { .. }), "the compound path, not its look: {:#?}", art[0]);
    assert!(same_tree(&art_tree(&art[0])["members"], &art_tree(&ours[0])["members"]), "{:#?}", art[0]);
    assert!(matches!(art[1].kind, NodeKind::Path { .. }), "the ellipse, not its look: {:#?}", art[1]);
    assert!(same_tree(&art_tree(&art[1])["anchors"], &art_tree(&ours[1])["anchors"]), "{:#?}", art[1]);
    assert_eq!(art[1].appearance.fill().and_then(|f| f.paint.color()), Some(Color::rgb(0.2, 0.6, 0.3)));
    for n in art {
        let st = n.appearance.stroke().unwrap();
        assert_eq!((st.brush.as_deref(), st.width), (Some("Nib"), 2.0));
    }
}

/// A path with an Art brush in Tints whose art doesn't bend along the path (its art has a painted
/// stroke) comes back as its drawn look, with the stroke warning; the brush itself comes back.
#[test]
fn an_art_brush_whose_art_doesnt_bend_keeps_its_paths_look() {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind, Colorization};
    let mut art = brush_art_node();
    if let Some(c) = art.children_mut() {
        for m in c.iter_mut() {
            Arc::make_mut(m).appearance = Appearance::basic(black_fill(), black_fill(), 1.0).into();
        }
    }
    let brush =
        Brush { name: "Stroked ribbon".into(), kind: BrushKind::Art(ArtBrush { art, colorization: Colorization::Tints, ..ArtBrush::default() }) };
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, std::slice::from_ref(&brush));
    brushed(&mut doc, "Stroked ribbon", (20.0, 120.0), (280.0, 150.0), 1.0);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::STROKES), "{:?}", out.warnings);
    assert!(!out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert!(matches!(back.layers[0].children().unwrap()[0].kind, NodeKind::Group { .. }));
    let lib = vectorcraft_brush::library(&back);
    let (BrushKind::Art(got), BrushKind::Art(want)) = (&lib[0].kind, &brush.kind) else { panic!("art") };
    assert!(same_tree(&art_tree(&got.art), &art_tree(&want.art)), "{:#?}", got.art);
}

/// Settings the other tests don't use come back: a Calligraphic brush varying with tilt and bearing,
/// a Scatter brush keeping its art's colors whose rotation doesn't follow the path, and brushes whose
/// modes aren't given (the reader gives them, the same ones).
#[test]
fn more_brush_settings_are_read_back() {
    use vectorcraft_brush::{Brush, BrushKind, Calligraphic, Colorization, Scatter, Variation};
    let tilted = Calligraphic {
        angle: -40.0,
        roundness: 75.0,
        size: 9.0,
        variation: [10.0, 5.0, 3.0],
        modes: Some([Variation::Tilt, Variation::Bearing, Variation::Random]),
    };
    let scatter = Scatter {
        art: brush_art_node(),
        size: (80.0, 80.0),
        spacing: (100.0, 100.0),
        scatter: (0.0, 0.0),
        rotation: (15.0, 15.0),
        rotation_relative_to_path: false,
        colorization: Colorization::None,
        modes: Some([Variation::Tilt, Variation::Bearing, Variation::Fixed, Variation::Rotation]),
    };
    let lib = vec![
        Brush { name: "Tilted".into(), kind: BrushKind::Calligraphic(tilted.clone()) },
        Brush { name: "Still".into(), kind: BrushKind::Scatter(scatter.clone()) },
        Brush { name: "Unset nib".into(), kind: BrushKind::Calligraphic(Calligraphic { modes: None, ..tilted }) },
        Brush { name: "Unset spray".into(), kind: BrushKind::Scatter(Scatter { modes: None, ..scatter }) },
    ];
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &lib);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let back = vectorcraft_brush::library(&read_back(&doc));
    assert_eq!(back.len(), lib.len());
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
    for (got, want) in back.iter().zip(&lib) {
        assert_eq!(got.name, want.name);
        match (&got.kind, &want.kind) {
            (BrushKind::Calligraphic(g), BrushKind::Calligraphic(w)) => {
                assert_eq!(g.modes(), w.modes(), "{}", want.name);
                assert_eq!(Calligraphic { modes: None, ..g.clone() }, Calligraphic { modes: None, ..w.clone() }, "{}", want.name);
            }
            (BrushKind::Scatter(g), BrushKind::Scatter(w)) => {
                assert_eq!(g.modes(), w.modes(), "{}", want.name);
                assert!(near(g.size, w.size) && near(g.spacing, w.spacing) && near(g.scatter, w.scatter) && near(g.rotation, w.rotation), "{g:?}");
                assert_eq!((g.colorization, g.rotation_relative_to_path), (Colorization::None, false), "{}", want.name);
                assert!(same_tree(&art_tree(&g.art), &art_tree(&w.art)), "{}: {:#?}", want.name, g.art);
            }
            _ => panic!("{}: {got:?}", want.name),
        }
    }
}

/// Brush art without width or height, a straight stroked line, comes back as that line, without a
/// frame or a group around it: a Scatter brush's horizontal line and an Art brush's vertical one.
#[test]
fn brush_art_of_a_straight_line_comes_back_as_the_line() {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind, Scatter};
    let line = |id: u64, to: (f64, f64)| {
        Node::path(vectorcraft_doc::NodeId(id), shapes::line((0.0, 0.0).into(), to.into()), Appearance::basic(Paint::None, black_fill(), 2.0))
    };
    let lib = vec![
        Brush { name: "Dash".into(), kind: BrushKind::Scatter(Scatter { art: line(1, (40.0, 0.0)), ..Scatter::default() }) },
        Brush { name: "Post".into(), kind: BrushKind::Art(ArtBrush { art: line(2, (0.0, 40.0)), ..ArtBrush::default() }) },
    ];
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &lib);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let back = vectorcraft_brush::library(&read_back(&doc));
    assert_eq!(back.len(), 2);
    for (got, want) in back.iter().zip(&lib) {
        let art = |b: &Brush| match &b.kind {
            BrushKind::Scatter(s) => s.art.clone(),
            BrushKind::Art(a) => a.art.clone(),
            _ => panic!("{}", b.name),
        };
        let (g, w) = (art_tree(&art(got)), art_tree(&art(want)));
        assert!(same_tree(&g, &w), "{}: {g:#} for {w:#}", want.name);
    }
}

/// Brush art of a path with several subpaths is written as a compound path, which the reader reads as
/// a compound path of one path for each subpath, and is weighed as the reader weighs that against the
/// art it copies into one file's brushes: of two Art brushes whose art is a path of 30,000 squares,
/// the second is left out with a warning, so that what is written is what comes back.
#[test]
fn brush_art_of_a_path_with_subpaths_is_weighed_as_it_is_read() {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind};
    let squares = |n: u32| {
        let mut path = PathData::default();
        for i in 0..n {
            let x = f64::from(i % 200) * 2.0;
            let y = f64::from(i / 200) * 2.0;
            path.subpaths.extend(shapes::rectangle(Rect::new(x, y, x + 1.0, y + 1.0)).subpaths);
        }
        Node::path(vectorcraft_doc::NodeId(1), path, Appearance::basic(black_fill(), Paint::None, 0.0))
    };
    let brush = |name: &str, n: u32| Brush { name: name.into(), kind: BrushKind::Art(ArtBrush { art: squares(n), ..ArtBrush::default() }) };
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &[brush("Two", 2)]);
    let back = vectorcraft_brush::library(&read_back(&doc));
    let BrushKind::Art(a) = &back[0].kind else { panic!("art") };
    let tree = art_tree(&a.art);
    assert_eq!(tree["kind"], "compound", "{tree:#}");
    assert_eq!(tree["members"].as_array().map(Vec::len), Some(2), "{tree:#}");
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &[brush("Many 1", 30_000), brush("Many 2", 30_000)]);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let names: Vec<String> = vectorcraft_brush::library(&read_back(&doc)).into_iter().map(|b| b.name).collect();
    assert_eq!(names, ["Many 1"]);
    assert_eq!(program(&doc).matches("%AI8_BeginBrushPattern").count(), 1);
}

/// Brush art that is a compound path is written as one path of all its members' subpaths, and is
/// weighed as the reader reads that back: a compound path of 24,000 members of two squares each is
/// read as 48,000 paths, within what the reader copies into one file's brushes, so the brush is
/// written without a warning; a compound path with one square comes back as one path.
#[test]
fn brush_art_of_a_compound_path_is_weighed_as_it_is_read() {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind};
    let square = |i: u32| {
        let x = f64::from(i % 400) * 2.0;
        let y = f64::from(i / 400) * 2.0;
        shapes::rectangle(Rect::new(x, y, x + 1.0, y + 1.0)).subpaths
    };
    let compound = |members: u32, per: u32| {
        let children = (0..members)
            .map(|m| {
                let path = PathData::new((0..per).flat_map(|k| square(m * per + k)).collect());
                Arc::new(Node::path(vectorcraft_doc::NodeId(2), path, Appearance::basic(black_fill(), Paint::None, 0.0)))
            })
            .collect();
        let mut n = Node::path(vectorcraft_doc::NodeId(1), PathData::default(), Appearance::basic(black_fill(), Paint::None, 0.0));
        n.kind = NodeKind::Compound { children, rule: FillRule::NonZero };
        n
    };
    let brush = |name: &str, art: Node| Brush { name: name.into(), kind: BrushKind::Art(ArtBrush { art, ..ArtBrush::default() }) };
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &[brush("One", compound(1, 1))]);
    let back = vectorcraft_brush::library(&read_back(&doc));
    let BrushKind::Art(a) = &back[0].kind else { panic!("art") };
    assert_eq!(art_tree(&a.art)["kind"], "path", "{:#}", art_tree(&a.art));
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &[brush("Many", compound(24_000, 2))]);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let back = vectorcraft_brush::library(&read_back(&doc));
    assert_eq!(back.len(), 1);
    let BrushKind::Art(a) = &back[0].kind else { panic!("art") };
    assert_eq!(a.art.children().map(Vec::len), Some(48_000));
}

/// A path with a drop shadow and the Calligraphic brush keeps the brush and the effect, and its drawn
/// look doesn't warn about the brushed stroke; a group with a drop shadow whose member has the Scatter
/// brush still warns, as the member is written as its brush's art.
#[test]
fn a_brushed_path_with_effects_keeps_its_brush_without_the_stroke_warning() {
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &brush_library());
    let mut line = Node::path(
        doc.alloc_id(),
        shapes::line((20.0, 50.0).into(), (280.0, 60.0).into()),
        Appearance::basic(Paint::None, solid(Color::rgb(1.0, 0.0, 0.0)), 2.0),
    );
    line.appearance.stroke_mut().unwrap().brush = Some("Nib".into());
    line.appearance.effects = vec![drop_shadow()];
    put(&mut doc, line);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::STROKES), "{:?}", out.warnings);
    let back = read_back(&doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert!(matches!(n.kind, NodeKind::Path { .. }), "the path, not its look: {n:#?}");
    assert_eq!(n.appearance.stroke().and_then(|s| s.brush.as_deref()), Some("Nib"));
    assert_eq!(n.appearance.effects.len(), 1);
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &brush_library());
    let mut member = Node::path(
        doc.alloc_id(),
        shapes::line((20.0, 50.0).into(), (280.0, 60.0).into()),
        Appearance::basic(Paint::None, solid(Color::rgb(1.0, 0.0, 0.0)), 2.0),
    );
    member.appearance.stroke_mut().unwrap().brush = Some("Spray".into());
    let mut g = group(&mut doc, vec![member], false);
    g.appearance.effects = vec![drop_shadow()];
    put(&mut doc, g);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::STROKES), "{:?}", out.warnings);
}

/// A brushed stroke that paints nothing isn't kept by an art style: a path with only such a stroke is
/// written without a style, and a document whose brushes are a new document's writes no brushes.
#[test]
fn a_brushed_stroke_that_paints_nothing_writes_no_brushes() {
    let mut doc = Document::new(300.0, 200.0);
    let mut line =
        Node::path(doc.alloc_id(), shapes::line((20.0, 50.0).into(), (280.0, 60.0).into()), Appearance::basic(Paint::None, Paint::None, 1.0));
    line.appearance.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    put(&mut doc, line);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let text = program(&doc);
    assert!(!text.contains("%AI8_BeginPluginObject"), "no brushes");
    assert!(!text.contains("/KnownStyle"), "no art style");
}

/// The lines of `program` that hold brushes and what is drawn with them: the brush art, the plug-in
/// objects, the art styles and the layers.
fn brush_lines(program: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut inside = false;
    for line in program.lines() {
        if ["%AI8_BeginBrushPattern", "%AI8_BeginPluginObject", "%AI9_BeginArtStyles", "%%EndSetup"].iter().any(|m| line.starts_with(m)) {
            inside = true;
        }
        if inside {
            out.push(line);
        }
        if ["%AI8_EndBrushPattern", "%AI8_EndPluginObject", "%AI9_EndArtStyles"].iter().any(|m| line.starts_with(m)) {
            inside = false;
        }
    }
    out
}

/// A document with brushes and brushed paths is written as the same bytes again, and what the reader
/// reads back is written with the same brushes, art styles and layers.
#[test]
fn brushes_write_the_same_bytes_again() {
    let doc = brushes_document();
    let first = export_ai_editing(&doc, &opts()).unwrap();
    assert_eq!(first.program, export_ai_editing(&doc, &opts()).unwrap().program);
    let again = String::from_utf8(export_ai_editing(&read_back(&doc), &opts()).unwrap().program).unwrap();
    let first = String::from_utf8(first.program).unwrap();
    let (again, first) = (brush_lines(&again), brush_lines(&first));
    let differ = again.iter().zip(&first).enumerate().find(|(_, (a, b))| a != b);
    assert_eq!(differ, None, "the first line that differs");
    assert_eq!(again.len(), first.len());
}

/// Brushes the reader wouldn't read back as they are are left out with a warning, each on its own
/// beside a brush that comes back, and each changed from a brush that comes back by the one setting
/// that leaves it out: an Art brush whose art is a group with a fill of its own, a Scatter brush whose
/// art is a path without points, an Art brush whose art is a group without members, Pattern and Bristle
/// brushes, Art brushes flipped along or across, overlapping, at another width, with a width that
/// varies, in another direction or scale, Art and Scatter brushes in Tints and Shades or Hue Shift, a
/// brush whose name holds a slash and a second brush of a name. A brush left out doesn't take its
/// name from a later one.
#[test]
fn brushes_the_reader_cant_read_are_left_out() {
    use vectorcraft_brush::{ArtBrush, ArtScale, Bristle, Brush, BrushKind, Colorization, Direction, PatternBrush, Variation};
    let lib = brush_library();
    let keep = lib[0].clone();
    let (BrushKind::Scatter(scatter), BrushKind::Art(art)) = (&lib[1].kind, &lib[2].kind) else { panic!("library") };
    let hue = Colorization::HueShift { key: Color::rgb(1.0, 0.0, 0.0) };
    let art_with = |f: &dyn Fn(&mut ArtBrush)| {
        let mut a = art.clone();
        f(&mut a);
        BrushKind::Art(a)
    };
    let mut painted_group = brush_art_node();
    painted_group.appearance = Appearance::basic(black_fill(), Paint::None, 0.0).into();
    let empty_path = Node::path(vectorcraft_doc::NodeId(1), PathData::new(vec![]), Appearance::basic(black_fill(), Paint::None, 0.0));
    let cases: Vec<(&str, BrushKind)> = vec![
        ("painted group", art_with(&|a| a.art = painted_group.clone())),
        ("empty path", BrushKind::Scatter(vectorcraft_brush::Scatter { art: empty_path.clone(), ..scatter.clone() })),
        ("empty group", art_with(&|a| a.art = Node::group(vectorcraft_doc::NodeId(1), vec![]))),
        ("pattern", BrushKind::Pattern(PatternBrush::default())),
        ("bristle", BrushKind::Bristle(Bristle::default())),
        ("flipped along", art_with(&|a| a.flip_along = true)),
        ("flipped across", art_with(&|a| a.flip_across = true)),
        ("overlap", art_with(&|a| a.overlap = true)),
        ("width", art_with(&|a| a.width = 50.0)),
        ("width varies", art_with(&|a| a.width_mode = Some(Variation::Pressure))),
        ("direction", art_with(&|a| a.direction = Direction::RightToLeft)),
        ("proportional", art_with(&|a| a.scale = ArtScale::Proportional)),
        ("art tints and shades", art_with(&|a| a.colorization = Colorization::TintsAndShades)),
        ("art hue shift", art_with(&|a| a.colorization = hue)),
        (
            "scatter tints and shades",
            BrushKind::Scatter(vectorcraft_brush::Scatter { colorization: Colorization::TintsAndShades, ..scatter.clone() }),
        ),
        ("scatter hue shift", BrushKind::Scatter(vectorcraft_brush::Scatter { colorization: hue, ..scatter.clone() })),
    ];
    let left_out = |extra: Vec<Brush>, why: &str| {
        let mut doc = Document::new(300.0, 200.0);
        let mut lib = vec![keep.clone()];
        lib.extend(extra);
        vectorcraft_brush::store(&mut doc, &lib);
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert!(out.warnings.iter().any(|w| w == writer::BRUSHES), "{why}: {:?}", out.warnings);
        assert_eq!(vectorcraft_brush::library(&read_back(&doc)), std::slice::from_ref(&keep), "{why}");
    };
    for (why, kind) in cases {
        left_out(vec![Brush { name: "Other".into(), kind }], why);
    }
    left_out(vec![Brush { name: "A/B".into(), ..keep.clone() }], "slash");
    left_out(vec![Brush { kind: BrushKind::Bristle(Bristle::default()), ..keep.clone() }], "a second brush of the name");
    // A Pattern brush named "X" is left out, so a Calligraphic brush named "X" after it comes back. A
    // path with "X" is drawn with the Pattern brush, so it comes back as its drawn look.
    let mut doc = Document::new(300.0, 200.0);
    let x = Brush { name: "X".into(), ..keep.clone() };
    vectorcraft_brush::store(&mut doc, &[Brush { name: "X".into(), kind: BrushKind::Pattern(PatternBrush::default()) }, x.clone()]);
    brushed(&mut doc, "X", (20.0, 50.0), (280.0, 60.0), 2.0);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(vectorcraft_brush::library(&back), [x]);
    let n = &back.layers[0].children().unwrap()[0];
    assert!(n.appearance.stroke().is_none_or(|s| s.brush.is_none()), "not with the other brush of the name: {n:#?}");
}

/// Brushes past the most the reader reads from one file are left out with a warning: of 4,097
/// Calligraphic brushes, the first 4,096 come back.
#[test]
fn brushes_past_the_readers_count_are_left_out() {
    let nib = brush_library().remove(0);
    let lib: Vec<vectorcraft_brush::Brush> = (0..4097).map(|i| vectorcraft_brush::Brush { name: format!("Nib {i}"), ..nib.clone() }).collect();
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &lib);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let back = vectorcraft_brush::library(&read_back(&doc));
    assert_eq!(back.len(), 4096);
    assert_eq!(back.last().map(|b| b.name.as_str()), Some("Nib 4095"));
}

/// A brush whose art the editing data would change (a stroke inside the path, written as its
/// outline) is left out, also after an object on the layers was already written with that change.
#[test]
fn a_brush_whose_art_would_change_is_left_out() {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind};
    let inside = |doc: &mut Document| {
        let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 8.0)), Appearance::basic(Paint::None, black_fill(), 2.0));
        n.appearance.stroke_mut().unwrap().align = vectorcraft_doc::StrokeAlign::Inside;
        n
    };
    let mut doc = Document::new(300.0, 200.0);
    let on_layer = inside(&mut doc);
    put(&mut doc, on_layer);
    let keep = brush_library().remove(0);
    let art = inside(&mut doc);
    vectorcraft_brush::store(
        &mut doc,
        &[keep.clone(), Brush { name: "Outlined".into(), kind: BrushKind::Art(ArtBrush { art, ..ArtBrush::default() }) }],
    );
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::STROKES), "{:?}", out.warnings);
    assert!(out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    assert_eq!(vectorcraft_brush::library(&read_back(&doc)), [keep]);
}

/// Brushes past the art the reader copies into one file's brushes are left out with a warning, so
/// that what is written is what comes back: three Art brushes of 100,000 anchors each, of which the
/// reader keeps two.
#[test]
fn brushes_past_the_readers_art_limit_are_left_out() {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind};
    let heavy = |id: u64| {
        let points: Vec<vectorcraft_geom::Point> =
            (0..100_000).map(|i| vectorcraft_geom::Point::new(f64::from(i) / 1000.0, if i % 2 == 0 { 0.0 } else { 4.0 })).collect();
        let path = PathData::single(vectorcraft_geom::SubPath::polyline(&points, true));
        Node::path(vectorcraft_doc::NodeId(id), path, Appearance::basic(black_fill(), Paint::None, 0.0))
    };
    let lib: Vec<Brush> =
        (1..=3).map(|i| Brush { name: format!("Heavy {i}"), kind: BrushKind::Art(ArtBrush { art: heavy(i), ..ArtBrush::default() }) }).collect();
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &lib);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let back: Vec<String> = vectorcraft_brush::library(&read_back(&doc)).into_iter().map(|b| b.name).collect();
    assert_eq!(back, ["Heavy 1", "Heavy 2"]);
    assert_eq!(program(&doc).matches("%AI8_BeginBrushPattern").count(), 2);
    // The art of the brush left out isn't counted: there is room for it all with room for what is read.
    let read = crate::import::objects_read(&out.program).unwrap();
    let room = export_within(&doc, &opts(), read).unwrap();
    assert!(!room.warnings.iter().any(|w| w == writer::OBJECT_LIMIT), "{read}: {:?}", room.warnings);
}

/// A brush whose art doesn't fit within the object limit is left out with the limit's warning, and
/// what its art took is given back: a later brush whose art fits comes back.
#[test]
fn a_brush_whose_art_passes_the_object_limit_leaves_room_for_the_next() {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind};
    let squares = |n: u64| {
        let block = |i: u64| {
            let x = i as f64 * 10.0;
            Arc::new(Node::path(
                vectorcraft_doc::NodeId(i + 1),
                shapes::rectangle(Rect::new(x, 0.0, x + 5.0, 5.0)),
                Appearance::basic(black_fill(), Paint::None, 0.0),
            ))
        };
        Node::group(vectorcraft_doc::NodeId(100), (0..n).map(block).collect())
    };
    let art = |name: &str, n: u64| Brush { name: name.into(), kind: BrushKind::Art(ArtBrush { art: squares(n), ..ArtBrush::default() }) };
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, &[art("Big", 6), art("Small", 2)]);
    let all = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!all.warnings.iter().any(|w| w == writer::OBJECT_LIMIT), "{:?}", all.warnings);
    let mut small = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut small, &[art("Small", 2)]);
    let room = crate::import::objects_read(&export_ai_editing(&small, &opts()).unwrap().program).unwrap();
    let out = export_within(&doc, &opts(), room).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::OBJECT_LIMIT), "{:?}", out.warnings);
    assert!(crate::import::objects_read(&out.program).unwrap() <= room);
    let mut data = ZSTD_MARKER.to_vec();
    data.extend(ruzstd::encoding::compress_to_vec(out.program.as_slice(), ruzstd::encoding::CompressionLevel::Fastest));
    let back = crate::ai_alone(&data).unwrap().0;
    let names: Vec<String> = vectorcraft_brush::library(&back).into_iter().map(|b| b.name).collect();
    assert_eq!(names, ["Small"]);
}

/// A graphic style with a brushed stroke, which is left out, takes no room from the layers' objects:
/// a document with a new document's brushes and no brushed path writes no brush art, so with room for
/// what its layers write, every object comes back.
#[test]
fn a_graphic_style_with_a_brush_takes_no_room_from_the_layers() {
    let mut doc = Document::new(300.0, 200.0);
    for i in 0..3 {
        let x = 20.0 + f64::from(i) * 40.0;
        let square =
            Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(x, 20.0, x + 30.0, 50.0)), Appearance::basic(black_fill(), Paint::None, 0.0));
        put(&mut doc, square);
    }
    let room = crate::import::objects_read(&export_ai_editing(&doc, &opts()).unwrap().program).unwrap();
    let mut stroke = StrokeLayer::new(solid(Color::rgb(1.0, 0.0, 0.0)), 1.0);
    stroke.brush = Some("3 pt. Round".into());
    let ap = Appearance { items: vec![AppearanceItem::Stroke(stroke)], effects: vec![], contents_index: None };
    doc.graphic_styles.push(vectorcraft_doc::GraphicStyle::new("Brushed", ap));
    let out = export_within(&doc, &opts(), room).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::OBJECT_LIMIT), "{room}: {:?}", out.warnings);
    let program = String::from_utf8(out.program).unwrap();
    assert!(!program.contains("%AI8_BeginBrushPattern"), "no brush art");
}

/// A graphic style whose brushed stroke paints nothing comes back in the Graphic Styles panel.
#[test]
fn a_graphic_style_with_an_unpainted_brushed_stroke_comes_back() {
    let mut doc = Document::new(300.0, 200.0);
    let mut ap = Appearance::basic(black_fill(), Paint::None, 1.0);
    ap.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    doc.graphic_styles.push(vectorcraft_doc::GraphicStyle::new("Unpainted brush", ap));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::PANEL_STYLES), "{:?}", out.warnings);
    assert!(read_back(&doc).graphic_style("Unpainted brush").is_some());
}

/// A brush whose art doesn't fit within the object limit after a path was written with it on its
/// stroke is left out with both warnings, and the path comes back as its drawn look.
#[test]
fn a_brushed_path_whose_brush_art_doesnt_fit_comes_back_as_its_look() {
    let ribbon = brush_library().remove(2);
    let mut doc = Document::new(300.0, 200.0);
    vectorcraft_brush::store(&mut doc, std::slice::from_ref(&ribbon));
    brushed(&mut doc, "Ribbon", (20.0, 120.0), (280.0, 150.0), 1.5);
    let all = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!all.warnings.iter().any(|w| w == writer::OBJECT_LIMIT || w == writer::BRUSHES), "{:?}", all.warnings);
    let room = crate::import::objects_read(&all.program).unwrap() - 1;
    let out = export_within(&doc, &opts(), room).unwrap();
    for w in [writer::OBJECT_LIMIT, writer::BRUSHES] {
        assert!(out.warnings.iter().any(|x| x == w), "{w}: {:?}", out.warnings);
    }
    assert!(crate::import::objects_read(&out.program).unwrap() <= room);
    let mut data = ZSTD_MARKER.to_vec();
    data.extend(ruzstd::encoding::compress_to_vec(out.program.as_slice(), ruzstd::encoding::CompressionLevel::Fastest));
    let back = crate::ai_alone(&data).unwrap().0;
    assert!(vectorcraft_brush::library(&back).is_empty());
    let n = &back.layers[0].children().unwrap()[0];
    assert!(matches!(n.kind, NodeKind::Group { .. }), "the path's drawn look: {n:#?}");
}

/// An empty Brushes panel comes back empty, and a document with a new document's brushes and no
/// brushed path writes no brushes (the reader gives such a file a new document's).
#[test]
fn an_empty_brushes_panel_comes_back_empty() {
    let mut doc = Document::new(300.0, 200.0);
    assert!(!program(&doc).contains("%AI8_BeginPluginObject"));
    assert_eq!(vectorcraft_brush::library(&read_back(&doc)), vectorcraft_brush::library(&doc));
    vectorcraft_brush::store(&mut doc, &[]);
    assert!(vectorcraft_brush::library(&read_back(&doc)).is_empty());
}

/// A path with one of a new document's brushes writes the brushes the reader reads, and keeps its
/// brush. The new document's Art brush with other options, its Pattern brushes and its Bristle
/// brushes are left out with a warning, so the file opens without them.
#[test]
fn a_new_documents_brush_on_a_path_writes_the_brushes() {
    let mut doc = Document::new(300.0, 200.0);
    brushed(&mut doc, "3 pt. Round", (20.0, 50.0), (280.0, 60.0), 1.0);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    let back = read_back(&doc);
    let n = &back.layers[0].children().unwrap()[0];
    assert_eq!(n.appearance.stroke().and_then(|s| s.brush.as_deref()), Some("3 pt. Round"));
    let left_out = ["Arrow", "Chain", "Stitches", "Diamonds", "Bristle Round", "Bristle Flat Fan"];
    let want: Vec<&str> = vectorcraft_brush::defaults().iter().map(|b| b.name.as_str()).filter(|n| !left_out.contains(n)).collect();
    let lib = vectorcraft_brush::library(&back);
    assert_eq!(lib.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), want);
    assert_eq!(vectorcraft_brush::defaults().len(), want.len() + left_out.len());
}

/// A path with one of a new document's brushes that doesn't fit within the object limit is left
/// out, and no brushes are written: the file opens with a new document's brushes, all of them.
#[test]
fn a_new_documents_brush_on_a_path_past_the_object_limit_writes_no_brushes() {
    let empty = Document::new(300.0, 200.0);
    let room = crate::import::objects_read(&export_ai_editing(&empty, &opts()).unwrap().program).unwrap();
    let mut doc = Document::new(300.0, 200.0);
    brushed(&mut doc, "3 pt. Round", (20.0, 50.0), (280.0, 60.0), 1.0);
    let out = export_within(&doc, &opts(), room).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::OBJECT_LIMIT), "{:?}", out.warnings);
    assert!(!out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    assert!(!String::from_utf8(out.program).unwrap().contains("%AI8_BeginPluginObject"));
}

/// On a large canvas the brushes are left out with a warning, and brushed paths come back as their
/// drawn look.
#[test]
fn a_large_canvas_leaves_the_brushes_out() {
    let mut doc = Document::new(20000.0, 100.0);
    vectorcraft_brush::store(&mut doc, &brush_library());
    brushed(&mut doc, "Nib", (20.0, 50.0), (280.0, 60.0), 2.0);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    assert!(!program(&doc).contains("%AI8_BeginPluginObject"));
    assert!(matches!(read_back(&doc).layers[0].children().unwrap()[0].kind, NodeKind::Group { .. }));
    // An empty panel is kept, without a warning.
    let mut doc = Document::new(20000.0, 100.0);
    vectorcraft_brush::store(&mut doc, &[]);
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(!out.warnings.iter().any(|w| w == writer::BRUSHES), "{:?}", out.warnings);
    assert!(vectorcraft_brush::library(&read_back(&doc)).is_empty());
}

// ---------- variables ----------

/// A 200 × 100 document with variables: a text variable `name` bound to point type, a visibility
/// variable `show` bound to an unnamed square and to a rectangle `Badge`, an image variable
/// `picture` bound to a linked image, a graph variable `chart` and a variable of unknown kind
/// `other`; the data sets `First` (two lines of text, hidden, a linked file in a folder with a space
/// and a `%`) and `Second` (empty text, shown, a linked file on a drive), `Second` shown; the
/// variables locked.
fn variables_doc() -> Document {
    use vectorcraft_doc::{DataSet, DataValue, Variable, VariableKind};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let square = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0)), Appearance::basic(solid(GOLD), Paint::None, 0.0));
    let mut badge =
        Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(40.0, 10.0, 90.0, 30.0)), Appearance::basic(solid(BRAND), Paint::None, 0.0));
    badge.name = Some("Badge".into());
    let pic = image_node(
        &mut doc,
        ImageBlob::png(rgb_png()),
        (3, 2),
        Affine::new([10.0, 0.0, 0.0, 10.0, 100.0, 10.0]),
        Some(vectorcraft_doc::LinkInfo::new("/tmp/a b/one.png")),
    );
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 70.0), "Ann", style(18.0, Color::BLACK));
    let t = text_node(&mut doc, t);
    let ids = [square.id, badge.id, pic.id, t.id];
    for n in [square, badge, pic, t] {
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let vars = &mut doc.variables;
    for (name, kind) in [
        ("name", VariableKind::Text),
        ("show", VariableKind::Visibility),
        ("picture", VariableKind::Image),
        ("chart", VariableKind::Graph),
        ("other", VariableKind::Unknown),
    ] {
        vars.variables.push(Variable { name: name.into(), kind });
    }
    vars.bind(ids[0], "show");
    vars.bind(ids[1], "show");
    vars.bind(ids[2], "picture");
    vars.bind(ids[3], "name");
    let row = |name: &str, text: &str, show: bool, file: &str| DataSet {
        name: name.into(),
        values: [
            ("name".to_string(), DataValue::Text(text.into())),
            ("show".to_string(), DataValue::Visible(show)),
            ("picture".to_string(), DataValue::Image(file.into())),
        ]
        .into(),
    };
    vars.datasets = vec![row("First", "Ann (one)\nLee & é", false, "/Users/test/Job 100%/pic.png"), row("Second", "", true, "C:\\Art\\two.png")];
    vars.active_dataset = Some("Second".into());
    vars.locked = true;
    doc
}

/// The objects of `doc` bound to each variable, by kind and name, in document order.
fn bound_objects(doc: &Document) -> Vec<String> {
    let mut out = vec![];
    for v in &doc.variables.variables {
        for id in doc.variables.objects_of(&v.name) {
            let n = doc.node(id).unwrap();
            let kind = match &n.kind {
                NodeKind::Image(im) => format!("image {:?}", im.link.as_ref().map(|l| l.path.clone())),
                NodeKind::Path { .. } => "path".to_string(),
                NodeKind::Text(t) => format!("type {:?}", t.runs.iter().map(|r| r.text.as_str()).collect::<String>()),
                _ => "other".to_string(),
            };
            out.push(format!("{} → {kind} {:?}", v.name, n.name));
        }
    }
    out
}

/// The warnings the reader gives about the variable library of `doc`'s editing data.
fn library_notes(doc: &Document) -> Vec<String> {
    let (_, warnings) = crate::ai_alone(&private_data(doc, &opts()).unwrap().0.data).unwrap();
    warnings.into_iter().filter(|w| w.contains("variable")).collect()
}

#[test]
fn variables_data_sets_and_bindings_are_read_back() {
    let doc = variables_doc();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.variables.variables, doc.variables.variables);
    assert_eq!(back.variables.datasets, doc.variables.datasets);
    assert_eq!(back.variables.active_dataset.as_deref(), Some("Second"));
    assert!(back.variables.locked);
    assert_eq!(bound_objects(&back), bound_objects(&doc));
    assert_eq!(
        bound_objects(&back),
        ["name → type \"Ann\" None", "show → path None", "show → path Some(\"Badge\")", "picture → image Some(\"/tmp/a b/one.png\") None"]
    );
    assert_eq!(library_notes(&doc), Vec::<String>::new());
    // The document read back is written the same way again.
    let library = |p: String| p[p.find("%_/XMLNode :").unwrap()..p.find("(#document)").unwrap()].to_string();
    assert_eq!(library(program_shown(&back)), library(program_shown(&doc)));
}

#[test]
fn variables_and_data_sets_past_the_readers_caps_are_left_out() {
    use vectorcraft_doc::{DataSet, Variable, VariableKind};
    let mut doc = variables_doc();
    for i in 0..300 {
        doc.variables.variables.push(Variable { name: format!("extra{i}"), kind: VariableKind::Text });
    }
    for i in 0..1_000 {
        doc.variables.datasets.push(DataSet { name: format!("Row {i}"), values: Default::default() });
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::LIMITS), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.variables.variables[..], doc.variables.variables[..256]);
    assert_eq!(back.variables.datasets[..], doc.variables.datasets[..1_000]);
    assert_eq!(bound_objects(&back).len(), 4);
    assert_eq!(library_notes(&doc), Vec::<String>::new());
}

/// Bindings that fill the reader's byte budget: the bindings past it are left out, and every
/// variable and every binding written reads back.
#[test]
fn bindings_that_fill_the_readers_budget_keep_every_variable() {
    use vectorcraft_doc::{Variable, VariableKind};
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let mut ids = vec![];
    for i in 0..170 {
        let mut n = Node::path(doc.alloc_id(), shapes::rectangle(Rect::new(1.0, 1.0, 2.0, 2.0)), Appearance::basic(solid(GOLD), Paint::None, 0.0));
        n.name = Some(format!("o{i:0>249}"));
        ids.push(n.id);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    for v in 0..120 {
        let name = format!("v{v:0>254}");
        doc.variables.variables.push(Variable { name: name.clone(), kind: VariableKind::Visibility });
        for id in &ids {
            doc.variables.bind(*id, &name);
        }
    }
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::LIMITS), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.variables.variables, doc.variables.variables);
    let bound: usize = back.variables.bindings.values().map(|names| names.len()).sum();
    assert!(bound > 0 && bound < 120 * 170, "{bound}");
    assert_eq!(library_notes(&doc), Vec::<String>::new());
}

#[test]
fn long_xml_ids_and_text_of_many_lines_are_left_out() {
    use vectorcraft_doc::DataValue;
    let mut doc = variables_doc();
    let badge = doc.variables.objects_of("show")[1];
    doc.node_mut(badge).unwrap().name = Some("_".repeat(60));
    let lines = "\n".repeat(150_000);
    doc.variables.datasets[0].values.insert("name".into(), DataValue::Text(lines));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::LIMITS), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(bound_objects(&back), ["name → type \"Ann\" None", "show → path None", "picture → image Some(\"/tmp/a b/one.png\") None"]);
    assert_eq!(back.variables.datasets[0].values.get("name"), None);
    assert_eq!(back.variables.datasets[1].values.get("name"), Some(&DataValue::Text(String::new())));
    // Nothing is written that the reader leaves out.
    assert_eq!(library_notes(&doc), Vec::<String>::new());
}

#[test]
fn a_layer_bound_to_a_visibility_variable_is_reported_left_out() {
    let mut doc = variables_doc();
    assert!(export_ai_editing(&doc, &opts()).unwrap().warnings.iter().all(|w| w != writer::BINDINGS));
    let layer = doc.layers[0].id;
    doc.variables.bind(layer, "show");
    assert!(export_ai_editing(&doc, &opts()).unwrap().warnings.iter().any(|w| w == writer::BINDINGS));
    assert_eq!(bound_objects(&read_back(&doc)).len(), 4);
}

#[test]
fn a_binding_to_an_object_no_longer_in_the_document_is_not_reported() {
    let mut doc = variables_doc();
    let gone = doc.alloc_id();
    doc.variables.bind(gone, "show");
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(bound_objects(&read_back(&doc)), bound_objects(&variables_doc()));
}

#[test]
fn a_data_set_shown_that_is_not_written_is_reported_left_out() {
    for active in [" Second", "Nope"] {
        let mut doc = variables_doc();
        doc.variables.active_dataset = Some(active.into());
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert_eq!(out.warnings, [writer::ACTIVE], "{active:?}");
        assert_eq!(read_back(&doc).variables.active_dataset, None, "{active:?}");
    }
}

#[test]
fn a_locked_library_without_variables_stays_locked() {
    let mut doc = Document::new(200.0, 100.0);
    doc.variables.locked = true;
    assert!(read_back(&doc).variables.locked);
}

#[test]
fn names_the_reader_does_not_read_as_they_are_are_left_out() {
    use vectorcraft_doc::{DataSet, Variable, VariableKind};
    let mut doc = variables_doc();
    for name in ["x".repeat(256), " padded".into(), String::new(), "show".into()] {
        doc.variables.variables.push(Variable { name: name.clone(), kind: VariableKind::Text });
        doc.variables.datasets.push(DataSet { name, values: Default::default() });
    }
    doc.variables.datasets.push(DataSet { name: "First".into(), values: Default::default() });
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::LIMITS), "{:?}", out.warnings);
    let back = read_back(&doc);
    let expected = variables_doc();
    assert_eq!(back.variables.variables, expected.variables.variables);
    let mut datasets = expected.variables.datasets.clone();
    datasets.push(DataSet { name: "show".into(), values: Default::default() });
    assert_eq!(back.variables.datasets, datasets);
    assert_eq!(library_notes(&doc), Vec::<String>::new());
}

#[test]
fn a_document_without_variables_has_no_library() {
    let p = program(&sample());
    assert!(!p.contains("XMLNode"));
}

#[test]
fn an_unlocked_library_without_a_data_set_is_read_back() {
    let mut doc = variables_doc();
    doc.variables.datasets.clear();
    doc.variables.active_dataset = None;
    doc.variables.locked = false;
    let back = read_back(&doc);
    let expected = vectorcraft_doc::Variables { bindings: back.variables.bindings.clone(), ..(*doc.variables).clone() };
    assert_eq!(*back.variables, expected);
    assert_eq!(bound_objects(&back), bound_objects(&doc));
}

#[test]
fn bindings_and_values_the_reader_cant_take_are_left_out() {
    use vectorcraft_doc::{DataValue, VariableKind};
    let mut doc = variables_doc();
    let layer = doc.layers[0].id;
    // Type of only white space is written as its outlines.
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 90.0), "   ", style(18.0, Color::BLACK));
    let t = text_node(&mut doc, t);
    let text_id = t.id;
    doc.insert(Some(layer), usize::MAX, t).unwrap();
    doc.variables.bind(text_id, "name");
    doc.variables.variables.push(vectorcraft_doc::Variable { name: "a:b".into(), kind: VariableKind::Text });
    let first = &mut doc.variables.datasets[0].values;
    first.insert("chart".into(), DataValue::Graph("a,b\n1,2".into()));
    first.insert("a:b".into(), DataValue::Text("lost".into()));
    first.insert("picture".into(), DataValue::Image("/a:b/pic.png".into()));
    let second = &mut doc.variables.datasets[1].values;
    second.insert("picture".into(), DataValue::Image("relative/pic.png".into()));
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w == writer::BINDINGS), "{:?}", out.warnings);
    assert!(out.warnings.iter().any(|w| w == writer::VALUES), "{:?}", out.warnings);
    let back = read_back(&doc);
    assert_eq!(back.variables.objects_of("name").len(), 1);
    assert_eq!(back.variables.datasets[0].values.get("chart"), None);
    assert_eq!(back.variables.datasets[0].values.get("a:b"), None);
    assert_eq!(back.variables.datasets[0].values.get("picture"), None);
    assert_eq!(back.variables.datasets[1].values.get("picture"), None);
    assert_eq!(bound_objects(&back).len(), 4);
}

/// Type in a variable font's named instance names the instance's place in the font's design space
/// (`/4`, each axis as 16.16 fixed) and the font's version (`/5`) in its font record, so the app that opens the file doesn't take
/// the font's default instance (#6388); a run's own Variable Font setting is a font of its own, and
/// a static font's record has no design vector. Each comes back as the style it was set in.
#[test]
fn variable_font_instances_name_their_design_space() {
    use vectorcraft_doc::{CharStyle, TextObject};
    use vectorcraft_text::test_fonts::{VARIABLE_FAMILY, variable_font};
    vectorcraft_text::FontDb::global().add_font(variable_font().unwrap());
    let set = |doc: &mut Document, y: f64, style: &str, axes: &[(&str, f64)]| {
        let st = CharStyle {
            font_family: VARIABLE_FAMILY.into(),
            font_style: style.into(),
            font_axes: axes.iter().map(|(t, v)| (t.to_string(), *v)).collect(),
            ..style_of(12.0)
        };
        let n = text_node(doc, TextObject::point(vectorcraft_geom::Point::new(10.0, y), style, st));
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    };
    let mut doc = Document::new(300.0, 200.0);
    set(&mut doc, 20.0, "Regular", &[]);
    set(&mut doc, 40.0, "Bold", &[]);
    set(&mut doc, 60.0, "Bold", &[("wght", 650.0)]);
    let static_text = TextObject::point(vectorcraft_geom::Point::new(10.0, 80.0), "Static", style_of(12.0));
    let n = text_node(&mut doc, static_text);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();

    let td = text_document(&program(&doc));
    // 400 × 65536, 700 × 65536 and 650 × 65536.
    assert!(td.contains("VaritestSans-Regular) /2 1 /4 [ 26214400 ]"), "{td}");
    assert!(td.contains("VaritestSans-Bold) /2 1 /4 [ 45875200 ]"), "{td}");
    assert!(td.contains("VaritestSans-Bold) /2 1 /4 [ 42598400 ]"), "a setting of its own is a font of its own: {td}");
    let static_name = vectorcraft_text::FontDb::global().face(&style_of(12.0).font_family, "Regular").unwrap().postscript_name();
    assert!(td.contains(&format!("{static_name}) /2 1 >>")), "a static font has no design vector: {td}");
    // Each variable font's record names the font's version (name ID 5) after its design vector.
    // (The string is UTF-16 after a byte order mark, so the match starts after it.)
    assert_eq!(td.matches("] /5 (").count(), 3, "{td}");
    assert_eq!(td.matches("Version 1.000) >>").count(), 3, "{td}");
    assert_eq!(td.matches("/CoolTypeFont").count(), 5, "four fonts and the hidden characters' font");

    let back = read_back(&doc);
    let styles: Vec<(String, String)> = back.layers[0]
        .children()
        .unwrap()
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some((t.runs[0].style.font_family.clone(), t.runs[0].style.font_style.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        styles[..3],
        [(VARIABLE_FAMILY.into(), "Regular".into()), (VARIABLE_FAMILY.into(), "Bold".into()), (VARIABLE_FAMILY.into(), "Bold".into())]
    );
}

fn style_of(size: f64) -> vectorcraft_doc::CharStyle {
    style(size, Color::BLACK)
}

/// The data has no conic gradient: a conic fill is a linear gradient along its vector, with the
/// same stops, and so is a conic gradient swatch, with a warning.
#[test]
fn conic_gradients_are_written_as_linear_gradients_with_their_stops() {
    let stops =
        vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(0.6, SKY), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Conic, stops.clone()));
    let (start, end) = (vectorcraft_geom::Point::new(100.0, 50.0), vectorcraft_geom::Point::new(150.0, 50.0));
    g.geom = Some(vectorcraft_color::GradientGeom { start, end, aspect: -0.5, focal: None });
    let mut doc = Document::new(200.0, 100.0);
    let id = doc.alloc_id();
    let l = doc.layers[0].id;
    let fill = Paint::Gradient(Box::new(g.clone()));
    doc.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(20.0, 10.0, 180.0, 90.0)), Appearance::basic(fill.clone(), Paint::None, 0.0)))
        .unwrap();
    let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
    swatches.push(Swatch { name: "Sweep".into(), paint: Paint::Gradient(Box::new(GradientPaint { geom: None, ..g })), global: false, spot: false });
    doc.swatches = swatches.into();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("conic gradients are written as linear gradients")), "{:?}", out.warnings);
    let back = read_back(&doc);
    let mut read = None;
    back.walk(|n| {
        if let Paint::Gradient(g) = n.appearance.fill_paint() {
            read = Some(g.clone());
        }
    });
    let read = read.expect("a gradient fill");
    assert_eq!(read.gradient.kind, GradientKind::Linear);
    assert_eq!(
        read.gradient.stops.iter().map(|s| (s.offset, s.color.to_hex())).collect::<Vec<_>>(),
        stops.iter().map(|s| (s.offset, s.color.to_hex())).collect::<Vec<_>>()
    );
    let geom = read.geom.unwrap();
    assert!(geom.start.distance(start) < 0.01 && geom.end.distance(end) < 0.01, "{geom:?}");
    let Some(Paint::Gradient(swatch)) = back.swatch("Sweep").map(|s| &s.paint) else { panic!("the swatch is lost") };
    assert_eq!(swatch.gradient.kind, GradientKind::Linear);
}

/// The data has no diamond gradient: a diamond fill is the radial gradient through its square's
/// corners (centered on it, its vector and the aspect across it), with the same stops, and so is
/// a diamond gradient swatch, with a warning.
#[test]
fn diamond_gradients_are_written_as_radial_gradients_through_their_corners() {
    let stops =
        vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(0.6, SKY), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Diamond, stops.clone()));
    let (start, end) = (vectorcraft_geom::Point::new(100.0, 50.0), vectorcraft_geom::Point::new(150.0, 50.0));
    // A focal point (a plug-in's or a file's) draws nothing on a diamond and isn't written.
    let focal = Some(vectorcraft_geom::Point::new(110.0, 55.0));
    g.geom = Some(vectorcraft_color::GradientGeom { start, end, aspect: 0.5, focal });
    let mut doc = Document::new(200.0, 100.0);
    let id = doc.alloc_id();
    let l = doc.layers[0].id;
    let fill = Paint::Gradient(Box::new(g.clone()));
    doc.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(20.0, 10.0, 180.0, 90.0)), Appearance::basic(fill, Paint::None, 0.0))).unwrap();
    let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
    swatches.push(Swatch { name: "Gem".into(), paint: Paint::Gradient(Box::new(GradientPaint { geom: None, ..g })), global: false, spot: false });
    doc.swatches = swatches.into();
    let out = export_ai_editing(&doc, &opts()).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("diamond gradients are written as radial gradients")), "{:?}", out.warnings);
    let back = read_back(&doc);
    let mut read = None;
    back.walk(|n| {
        if let Paint::Gradient(g) = n.appearance.fill_paint() {
            read = Some(g.clone());
        }
    });
    let read = read.expect("a gradient fill");
    assert_eq!(read.gradient.kind, GradientKind::Radial);
    assert_eq!(
        read.gradient.stops.iter().map(|s| (s.offset, s.color.to_hex())).collect::<Vec<_>>(),
        stops.iter().map(|s| (s.offset, s.color.to_hex())).collect::<Vec<_>>()
    );
    let geom = read.geom.unwrap();
    assert!(geom.start.distance(start) < 0.01 && geom.end.distance(end) < 0.01 && (geom.aspect - 0.5).abs() < 0.01, "{geom:?}");
    assert!(geom.focal.is_none_or(|f| f.distance(start) < 0.01), "{geom:?}");
    let Some(Paint::Gradient(swatch)) = back.swatch("Gem").map(|s| &s.paint) else { panic!("the swatch is lost") };
    assert_eq!(swatch.gradient.kind, GradientKind::Radial);
}

/// The data has no spread: a fill that repeats or reflects holds its stops repeated across the
/// shape and reads back looking as the canvas paints it; a swatch holds its stops repeated along
/// its vector; both with a warning.
#[test]
fn spread_gradients_are_written_with_their_stops_repeated() {
    use vectorcraft_color::GradientSpread;
    for kind in [GradientKind::Linear, GradientKind::Radial] {
        let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
        let mut g = GradientPaint::new(Gradient { spread: GradientSpread::Reflect, repeats: 2, ..Gradient::new(kind, stops) });
        let (start, end) = (vectorcraft_geom::Point::new(60.0, 50.0), vectorcraft_geom::Point::new(120.0, 50.0));
        g.geom = Some(vectorcraft_color::GradientGeom { start, end, aspect: 1.0, focal: None });
        let mut doc = Document::new(200.0, 100.0);
        let id = doc.alloc_id();
        let l = doc.layers[0].id;
        let fill = Paint::Gradient(Box::new(g.clone()));
        doc.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(20.0, 10.0, 180.0, 90.0)), Appearance::basic(fill, Paint::None, 0.0)))
            .unwrap();
        let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
        swatches.push(Swatch {
            name: "Bands".into(),
            paint: Paint::Gradient(Box::new(GradientPaint { geom: None, ..g })),
            global: false,
            spot: false,
        });
        doc.swatches = swatches.into();
        let out = export_ai_editing(&doc, &opts()).unwrap();
        assert!(
            out.warnings.iter().any(|w| w.contains("gradients that repeat or reflect are written with their stops repeated")),
            "{:?}",
            out.warnings
        );
        let back = read_back(&doc);
        let st = vectorcraft_testkit::raster::perceptual_diff(
            &vectorcraft_testkit::raster::render_artboard(&doc),
            &vectorcraft_testkit::raster::render_artboard(&back),
            8.0,
        );
        assert!((st.bad_pixels as f64) < 0.02 * st.pixels as f64, "{kind:?}: {} of {} pixels differ", st.bad_pixels, st.pixels);
        let Some(Paint::Gradient(swatch)) = back.swatch("Bands").map(|s| &s.paint) else { panic!("the swatch is lost") };
        let colors: Vec<String> = swatch.gradient.stops.iter().map(|s| s.color.to_hex()).collect();
        assert_eq!(colors, ["#ff0000", "#0000ff", "#0000ff", "#ff0000"], "{kind:?}");
    }
}

/// A stroke's gradient that repeats is written with its stops repeated as far as the stroke's
/// miter joins reach, past the box the gradient is placed on, so a sharp join's tip shows the next
/// run rather than the color the gradient pads with; a straight line, which has no joins, keeps
/// every run within the stops the reader reads.
#[test]
fn a_stroke_spread_gradient_runs_on_into_its_miter_tips() {
    use vectorcraft_color::GradientSpread;
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut g = GradientPaint::new(Gradient { spread: GradientSpread::Repeat, repeats: 3, ..Gradient::new(GradientKind::Linear, stops) });
    // From the top of the box the gradient is placed on (the path's box grown by half the width).
    let (start, end) = (vectorcraft_geom::Point::new(100.0, 15.0), vectorcraft_geom::Point::new(100.0, 85.0));
    g.geom = Some(vectorcraft_color::GradientGeom { start, end, aspect: 1.0, focal: None });
    let mut app = Appearance::basic(Paint::None, Paint::Gradient(Box::new(g)), 10.0);
    for item in &mut app.items {
        if let AppearanceItem::Stroke(s) = item {
            (s.join, s.miter_limit) = (LineJoin::Miter, 10.0);
        }
    }
    // A V whose apex's miter reaches about 10 pt above it, to y = 9.9.
    let pts = [vectorcraft_geom::Point::new(60.0, 90.0), vectorcraft_geom::Point::new(100.0, 20.0), vectorcraft_geom::Point::new(140.0, 90.0)];
    let mut doc = Document::new(200.0, 100.0);
    let id = doc.alloc_id();
    let l = doc.layers[0].id;
    doc.insert(Some(l), 0, Node::path(id, PathData::new(vec![vectorcraft_geom::SubPath::polyline(&pts, false)]), app)).unwrap();
    let back = read_back(&doc);
    let mut geom = None;
    back.walk(|n| {
        if let Paint::Gradient(g) = n.appearance.stroke_paint() {
            geom = g.geom;
        }
    });
    let geom = geom.expect("the stroke keeps a gradient");
    assert!(geom.start.y.min(geom.end.y) < 9.9, "the runs start at {:?}", geom);
    // A straight line has no joins: its 100 runs, 200 stops, fit the 256 the reader reads.
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut g = GradientPaint::new(Gradient { spread: GradientSpread::Repeat, repeats: 100, ..Gradient::new(GradientKind::Linear, stops) });
    let (start, end) = (vectorcraft_geom::Point::new(50.0, 50.0), vectorcraft_geom::Point::new(150.0, 50.0));
    g.geom = Some(vectorcraft_color::GradientGeom { start, end, aspect: 1.0, focal: None });
    let line = PathData::new(vec![vectorcraft_geom::SubPath::polyline(&[start, end], false)]);
    let mut doc = Document::new(200.0, 100.0);
    let id = doc.alloc_id();
    let l = doc.layers[0].id;
    doc.insert(Some(l), 0, Node::path(id, line, Appearance::basic(Paint::None, Paint::Gradient(Box::new(g)), 10.0))).unwrap();
    let mut stops = None;
    read_back(&doc).walk(|n| {
        if let Paint::Gradient(g) = n.appearance.stroke_paint() {
            stops = Some(g.gradient.stops.len());
        }
    });
    assert_eq!(stops, Some(200));
}

/// A conic fill (written linear, its stops repeated around its turn), a diamond fill (written
/// radial) and a swatch whose stops repeat more times than the 256 stops the reader reads stay
/// gradients: each holds its stops repeated as far as 256 stops go.
#[test]
fn spread_gradients_past_the_stops_the_reader_reads_stay_gradients() {
    use vectorcraft_color::GradientSpread;
    let three = || {
        vec![
            GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)),
            GradientStop::new(0.5, Color::rgb(0.0, 1.0, 0.0)),
            GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0)),
        ]
    };
    for kind in [GradientKind::Conic, GradientKind::Diamond] {
        let mut g = GradientPaint::new(Gradient { spread: GradientSpread::Repeat, repeats: 100, ..Gradient::new(kind, three()) });
        let c = vectorcraft_geom::Point::new(100.0, 50.0);
        g.geom = Some(vectorcraft_color::GradientGeom { start: c, end: c + vectorcraft_geom::Vec2::new(40.0, 0.0), aspect: 1.0, focal: None });
        let mut doc = Document::new(200.0, 100.0);
        let id = doc.alloc_id();
        let l = doc.layers[0].id;
        let fill = Paint::Gradient(Box::new(g));
        doc.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(20.0, 10.0, 180.0, 90.0)), Appearance::basic(fill, Paint::None, 0.0)))
            .unwrap();
        let back = read_back(&doc);
        let mut stops = None;
        back.walk(|n| {
            if let Paint::Gradient(g) = n.appearance.fill_paint() {
                stops = Some(g.gradient.stops.len());
            }
        });
        let stops = stops.unwrap_or_else(|| panic!("{kind:?}: the gradient is written as a solid color"));
        assert!((200..=crate::import::MAX_GRADIENT_STOPS).contains(&stops), "{kind:?}: {stops} stops");
    }
    // Stops at 0.1 and 0.9 run from 0 to 1 as four: 70 runs are 280 stops.
    let stops = vec![GradientStop::new(0.1, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(0.9, Color::rgb(0.0, 0.0, 1.0))];
    let g = GradientPaint::new(Gradient { spread: GradientSpread::Reflect, repeats: 70, ..Gradient::new(GradientKind::Linear, stops) });
    let mut doc = Document::new(200.0, 100.0);
    let mut swatches: Vec<Swatch> = doc.swatches.iter().cloned().collect();
    swatches.push(Swatch { name: "Bands".into(), paint: Paint::Gradient(Box::new(g)), global: false, spot: false });
    doc.swatches = swatches.into();
    let back = read_back(&doc);
    let Some(Paint::Gradient(swatch)) = back.swatch("Bands").map(|s| &s.paint) else { panic!("the swatch is lost") };
    assert!((200..=crate::import::MAX_GRADIENT_STOPS).contains(&swatch.gradient.stops.len()), "{} stops", swatch.gradient.stops.len());
}
