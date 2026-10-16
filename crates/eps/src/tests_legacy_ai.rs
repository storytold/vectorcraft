//! Legacy `.ai` files (version 7) written, then read back as File › Open reads them.

use std::sync::Arc;

use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, AnchorKind, FillRule, PathData, Rect, shapes};

use super::*;

/// A node of `doc` filled `fill` (and stroked `stroke` 2 pt wide), path `path`.
fn shape(doc: &mut Document, path: PathData, fill: Paint, stroke: Paint) -> Node {
    Node::path(doc.alloc_id(), path, Appearance::basic(fill, stroke, 2.0))
}

fn red() -> Paint {
    Paint::solid(Color::rgb(1.0, 0.0, 0.0))
}

/// A 200 × 100 document: a layer `Back` holding a group of a red square (stroked blue) and a
/// locked one, a compound ring, a clipping group and a gradient square; a hidden layer `Front`
/// holding an ellipse and a 2 × 1 image.
fn sample() -> Document {
    let mut doc = Document::new(200.0, 100.0);
    doc.layers.clear();
    let back = doc.add_layer(Some("Back"));
    let front = doc.add_layer(Some("Front"));
    let a = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), red(), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    let mut b = shape(&mut doc, shapes::rectangle(Rect::new(50.0, 10.0, 80.0, 40.0)), Paint::solid(Color::gray(0.5)), Paint::None);
    b.locked = true;
    let group = Node::group(doc.alloc_id(), vec![Arc::new(a), Arc::new(b)]);
    doc.insert(Some(back), usize::MAX, group).unwrap();
    let outer = shape(&mut doc, shapes::rectangle(Rect::new(100.0, 10.0, 140.0, 50.0)), Paint::None, Paint::None);
    let inner = shape(&mut doc, shapes::rectangle(Rect::new(110.0, 20.0, 130.0, 40.0)), Paint::None, Paint::None);
    let mut ring = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(inner)], rule: FillRule::EvenOdd });
    ring.appearance = Appearance::basic(Paint::solid(Color::cmyk(0.0, 1.0, 1.0, 0.0)), Paint::None, 0.0).into();
    doc.insert(Some(back), usize::MAX, ring).unwrap();
    let clip = Node::new(
        doc.alloc_id(),
        NodeKind::Path {
            path: shapes::rectangle(Rect::new(160.0, 20.0, 180.0, 40.0)),
            rule: FillRule::NonZero,
            live: None,
            clipping: true,
            guide: false,
        },
    );
    let clipped = shape(&mut doc, shapes::rectangle(Rect::new(150.0, 10.0, 190.0, 50.0)), Paint::solid(Color::gray(0.25)), Paint::None);
    let window = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(clipped)], clip: true });
    doc.insert(Some(back), usize::MAX, window).unwrap();
    let g = Gradient {
        kind: GradientKind::Linear,
        stops: vec![GradientStop::new(0.0, Color::rgb(1.0, 1.0, 1.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 0.0))],
    };
    let shaded = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 60.0, 50.0, 90.0)), Paint::Gradient(Box::new(GradientPaint::new(g))), Paint::None);
    doc.insert(Some(back), usize::MAX, shaded).unwrap();
    let ellipse = shape(&mut doc, shapes::ellipse(Rect::new(100.0, 60.0, 160.0, 90.0)), red(), Paint::None);
    doc.insert(Some(front), usize::MAX, ellipse).unwrap();
    let mut png = vec![];
    let img = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let blob = ImageBlob::new("image/png", png);
    let key = blob.content_key();
    doc.images.insert(key.clone(), blob);
    let im =
        ImageObject { key, width: 2, height: 1, xf: Affine::new([10.0, 0.0, 0.0, 10.0, 170.0, 70.0]), link: None, placement: Default::default() };
    let im = Node::new(doc.alloc_id(), NodeKind::Image(Box::new(im)));
    doc.insert(Some(front), usize::MAX, im).unwrap();
    if let Some(l) = doc.layers.get_mut(1) {
        Arc::make_mut(l).visible = false;
    }
    doc
}

fn write(doc: &Document) -> LegacyAiOutput {
    write_with(doc, LegacyAiOptions::default())
}

/// `doc` written with options `o`. Every file the tests write is also run as a PostScript program
/// ([`prints_as`]) and must draw what `doc` draws.
fn write_with(doc: &Document, o: LegacyAiOptions) -> LegacyAiOutput {
    let o = LegacyAiOptions { title: "Sample".into(), created: Some(0), ..o };
    let out = export_legacy_ai(doc, &o).unwrap();
    prints_as(&out.bytes, doc, &o);
    out
}

/// `doc` as the prolog prints the file written from it with options `o`: without its layers that
/// don't print, its type as its glyphs' outlines where the writer writes them (all type with
/// [`LegacyAiOptions::outline_text`], else the type 8-bit text can't hold) and left out where it
/// writes text objects, its
/// hidden objects shown when `o` writes them visible, and its gradients filled with their first
/// stop's colour.
fn as_printed(doc: &Document, o: &LegacyAiOptions) -> Document {
    // A run's glyphs, laid out, as one path with the run's fill (the renderer draws no type).
    fn outlined(t: &vectorcraft_doc::TextObject) -> Node {
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let runs = t.runs.iter().enumerate().map(|(i, run)| {
            let mut bp = kurbo::BezPath::new();
            for g in layout.glyphs.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
            }
            let path = PathData::from_bezpath(&bp).transformed(t.xf);
            Arc::new(Node::path(vectorcraft_doc::NodeId(0), path, Appearance::basic(run.style.fill.clone(), Paint::None, 0.0)))
        });
        Node::group(vectorcraft_doc::NodeId(0), runs.collect())
    }
    fn plain(nodes: &mut Vec<Arc<Node>>, o: &LegacyAiOptions) {
        nodes.retain(|n| match &n.kind {
            NodeKind::Text(t) => o.outline_text || !writable(t),
            NodeKind::Layer { printable, .. } => *printable,
            _ => true,
        });
        for n in nodes {
            if let NodeKind::Text(t) = &n.kind {
                *n = Arc::new(outlined(t));
            }
            let n = Arc::make_mut(n);
            if o.hidden == HiddenObjects::Visible && !matches!(n.kind, NodeKind::Layer { .. }) {
                n.visible = true;
            }
            for item in &mut n.appearance.items {
                if let AppearanceItem::Fill(f) = item
                    && let Paint::Gradient(g) = &f.paint
                {
                    f.paint = g.gradient.stops.first().map_or(Paint::None, |s| Paint::solid(s.color));
                }
            }
            if let Some(c) = n.children_mut() {
                plain(c, o);
            }
        }
    }
    let mut d = doc.clone();
    plain(&mut d.layers, o);
    d
}

/// The legacy `.ai` `bytes` run as a PostScript program (VectorCraft's PostScript reader, with the
/// file's prolog) draws as `doc`, written with options `o` (its first artboard the file's), prints
/// ([`as_printed`]): the same art in the same places, in the same colours.
fn prints_as(bytes: &[u8], doc: &Document, o: &LegacyAiOptions) {
    let text = String::from_utf8_lossy(bytes);
    let nums = |key: &str| -> Vec<f64> {
        let line = text.lines().find_map(|l| l.strip_prefix(key)).unwrap_or_else(|| panic!("no {key}"));
        line.split_whitespace().map(|v| v.parse().unwrap()).collect()
    };
    // The art's box (the page the program draws on) and the artboard's top, in art space (y up).
    let [llx, lly, urx, ury] = nums("%%HiResBoundingBox:")[..] else { panic!() };
    let top = nums("%AI3_TileBox:")[3];
    let board = doc.artboards[0].rect;
    let there = Rect::new(board.x0 + llx, board.y0 + top - ury, board.x0 + urx, board.y0 + top - lly);
    let scale = 300.0 / (urx - llx).max(ury - lly).max(1.0);
    let want = vectorcraft_render::Renderer::new().render_region(&as_printed(doc, o), there, scale, true);
    let drew = match crate::import_with(bytes, false) {
        Ok(page) => {
            assert!(!page.preview, "read as its program");
            vectorcraft_render::Renderer::new().render_region(&page.document, Rect::new(0.0, 0.0, urx - llx, ury - lly), scale, true)
        }
        // A page that marks nothing (type alone): blank.
        Err(e) if e.contains("draws nothing") => {
            vectorcraft_render::Renderer::new().render_region(&Document::new(1.0, 1.0), Rect::new(0.0, 0.0, urx - llx, ury - lly), scale, true)
        }
        Err(e) => panic!("the file doesn't run as PostScript: {e}"),
    };
    assert_eq!((drew.width, drew.height), (want.width, want.height));
    let differ = drew
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(want.pixels.as_chunks::<4>().0)
        .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 48))
        .count();
    let share = differ as f64 / f64::from(want.width * want.height).max(1.0);
    assert!(share < 0.002, "the program draws {:.2}% of the page differently", share * 100.0);
}

fn children(n: &Node) -> &[Arc<Node>] {
    n.children().map_or(&[][..], Vec::as_slice)
}

#[test]
fn writes_the_documented_operators() {
    let out = write(&sample());
    let text = String::from_utf8(out.bytes).unwrap();
    for needle in [
        "%!PS-Adobe-3.0\n",
        "%AI5_FileFormat 3\n",
        "%%CreationDate: (1/1/1970) (00:00)\n",
        "%AI5_ArtSize: 200 100",
        "%AI3_TemplateBox: 100 50 100 50",
        "%AI5_NumLayers: 2",
        "(Back) Ln",
        "(Front) Ln",
        "%AI5_BeginGradient: (Gradient)",
        "\nu\n",
        "\n*u\n",
        "\nq\n",
        "\n1 A\n",
        "XI\n",
        "%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\nend\n%%EOF\n",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in\n{text}");
    }
    // The prolog is the writer's own procedure set, named in the header, in the dictionary the
    // setup begins; it defines the page trailer's `annotatepage` (§3).
    let prolog = text.split("%%BeginProlog\n").nth(1).and_then(|t| t.split("%%EndProlog\n").next()).unwrap();
    assert!(prolog.starts_with(&format!("%%BeginResource: procset {PROCSET}\n/VCLegacy ")), "{prolog}");
    assert!(prolog.ends_with("end\n%%EndResource\n") && prolog.contains("\n/annotatepage {} bd\n"), "{prolog}");
    assert!(text.contains(&format!("%%DocumentSuppliedResources: procset {PROCSET}\n")), "{text}");
    // The hidden layer is written hidden: visible 0.
    assert!(text.contains("%AI5_BeginLayer\n0 1 1 1 0 0 1 "), "{text}");
    // Art space is y up from the artboard's bottom-left: the red square (10..40 down from the top)
    // is 60..90 up. Its corners are corner points (upper case, AI7 spec §5.3.2); its fill RGB
    // (`r g b Xa`, §5.4.1).
    assert!(text.contains("1 0 0 Xa\n"), "{text}");
    assert!(text.contains("10 90 m\n40 90 L\n40 60 L\n10 60 L\n10 90 L\nb\n"), "{text}");
    // The ellipse's points are smooth (lower case).
    assert!(text.contains(" c\n") && !text.contains(" C\n"), "{text}");
    // The clipping path: closed by `h`, then `W n` (§5.6.2).
    assert!(text.contains(" L\nh\nW\nn\n"), "{text}");
    // Gradients: counted (`Bn`), each stop's operands before `%_Bs` (§3.5).
    assert!(text.contains("%%BeginSetup\nVCLegacy begin\n1 Bn\n%AI5_BeginGradient: (Gradient)\n(Gradient) 0 2 Bd\n[\n"), "{text}");
    assert!(text.contains(" 2 50 100 %_Bs\n") && !text.contains("Np\n"), "{text}");
    assert!(text.contains("1 0 0 1 0 0 Bg\nf\n0 BB\n"), "{text}");
    // The image: operands and `XI` on a line, its samples on comment lines (§8).
    assert!(text.contains("] 0 0 2 1 2 1 8 3 0 0 0 0 XI\n%FF00000000FF\n%AI5_EndRaster\n"), "{text}");
}

/// An image's matrix (§8) is an art-space matrix applied to its pixels with their rows running
/// downwards, `(u, -v)`: a rotated image is written with the matrix the reader and other apps
/// place it by, prints where it is, and reads back where it was (#2269).
#[test]
fn a_rotated_image_keeps_its_place() {
    let mut doc = Document::new(200.0, 100.0);
    let mut png = vec![];
    image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255])
        .unwrap()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let blob = ImageBlob::new("image/png", png);
    let key = blob.content_key();
    doc.images.insert(key.clone(), blob);
    // Turned 90° clockwise on the page: the pixel rows run down, the next row is to the left.
    let xf = Affine::new([0.0, 10.0, -10.0, 0.0, 100.0, 40.0]);
    let im =
        Node::new(doc.alloc_id(), NodeKind::Image(Box::new(ImageObject { key, width: 2, height: 1, xf, link: None, placement: Default::default() })));
    doc.layers.clear();
    let layer = doc.add_layer(Some("Art"));
    doc.insert(Some(layer), usize::MAX, im).unwrap();
    let out = write(&doc);
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    // In art space (y up) the rows run down, (0, -10) a pixel, and the next row is to the left,
    // (-10, 0): [a b] = (0, -10) and [c d] = (10, 0), as the matrix takes (u, -v).
    let line = text.lines().find(|l| l.ends_with(" XI")).unwrap();
    let m: Vec<f64> = line.trim_start_matches('[').split(']').next().unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect();
    assert_eq!(m[..4], [0.0, -10.0, 10.0, 0.0], "{line}");
    let read = crate::import(&out.bytes).unwrap().document;
    let mut found = None;
    read.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            found = Some(im.xf);
        }
    });
    let back = found.unwrap();
    assert!(back.as_coeffs().iter().zip(xf.as_coeffs()).all(|(a, b)| (a - b).abs() < 1e-6), "{back:?}");
}

#[test]
fn a_gradient_fill_and_its_stroke_are_one_object() {
    let mut doc = Document::new(100.0, 100.0);
    let g = Gradient {
        kind: GradientKind::Radial,
        stops: vec![GradientStop::new(0.0, Color::rgb(1.0, 1.0, 1.0)), GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 0.0, 1.0))],
    };
    let mut paint = GradientPaint::new(g);
    paint.geom = Some(vectorcraft_color::GradientGeom {
        start: vectorcraft_geom::Point::new(30.0, 30.0),
        end: vectorcraft_geom::Point::new(50.0, 30.0),
        aspect: 1.0,
        focal: Some(vectorcraft_geom::Point::new(35.0, 30.0)),
    });
    let n = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), Paint::Gradient(Box::new(paint)), red());
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let text = String::from_utf8(write(&doc).bytes).unwrap();
    // `2 BB` strokes the filled path (§3.5.3); the highlight is a quarter of the radius out (§3.5.5).
    assert!(text.contains("Bb\n5 0 0 0.25 Bh\n1 (Gradient) 30 70 0 20 1 0 0 1 0 0 Bg\nf\n2 BB\n"), "{text}");
    let doc = crate::import(text.as_bytes()).unwrap().document;
    let n = &children(&doc.layers[0])[0];
    let Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("{:?}", n.appearance) };
    assert_eq!(g.gradient.kind, GradientKind::Radial);
    assert_eq!(n.appearance.stroke_paint().color(), Some(Color::rgb(1.0, 0.0, 0.0)));
}

#[test]
fn reads_back_with_layers_groups_and_paths() {
    let out = write(&sample());
    let imported = crate::import(&out.bytes).unwrap();
    let doc = imported.document.clone();
    let names: Vec<_> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Back", "Front"], "{:?}", imported.warnings);
    assert!(doc.layers[0].visible);
    assert!(!doc.layers[1].visible);
    let back = children(&doc.layers[0]);
    assert_eq!(back.len(), 4, "{back:#?}");
    // The group of two squares, the second locked.
    let NodeKind::Group { children: pair, clip: false } = &back[0].kind else { panic!("{:?}", back[0].kind) };
    assert_eq!(pair.len(), 2);
    assert!(!pair[0].locked && pair[1].locked);
    let a = &pair[0];
    assert_eq!(a.appearance.fill_paint().color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(a.appearance.stroke_paint().color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    // Four corners: the start point written again to close it isn't a fifth.
    let sq = a.path_data().unwrap();
    assert!(sq.subpaths[0].closed && sq.subpaths[0].anchors.len() == 4, "{sq:?}");
    assert!(sq.subpaths[0].anchors.iter().all(|p| p.kind == AnchorKind::Corner), "{sq:?}");
    let bounds = a.visual_bounds().unwrap();
    assert!((bounds.x0 - 9.0).abs() < 0.01 && (bounds.y0 - 9.0).abs() < 0.01, "{bounds:?}");
    // The ring: a compound path, even-odd.
    assert!(matches!(&back[1].kind, NodeKind::Compound { children, rule: FillRule::EvenOdd } if children.len() == 2), "{:?}", back[1].kind);
    // The clipping group: its clipping path first.
    let NodeKind::Group { children: window, clip: true } = &back[2].kind else { panic!("{:?}", back[2].kind) };
    assert!(matches!(window[0].kind, NodeKind::Path { clipping: true, .. }));
    // The gradient square.
    let Paint::Gradient(g) = back[3].appearance.fill_paint() else { panic!("{:?}", back[3].appearance) };
    assert_eq!(g.gradient.kind, GradientKind::Linear);
    assert_eq!(g.gradient.stops.len(), 2);
    assert_eq!(g.gradient.stops[0].color, Color::rgb(1.0, 1.0, 1.0));
    // The hidden layer: the ellipse with smooth points, and the image.
    let front = children(&doc.layers[1]);
    assert_eq!(front.len(), 2, "{:?} {:?}", front.iter().map(|n| &n.kind).collect::<Vec<_>>(), imported.warnings);
    let path = front[0].path_data().unwrap();
    assert_eq!(path.subpaths.len(), 1);
    assert!(path.subpaths[0].closed);
    assert!(path.subpaths[0].anchors.iter().all(|a| a.kind == AnchorKind::Smooth), "{path:?}");
    assert_eq!(path.subpaths[0].anchors.len(), 4, "{path:?}");
    let NodeKind::Image(im) = &front[1].kind else { panic!("{:?}", front[1].kind) };
    assert_eq!((im.width, im.height), (2, 1));
    let b = front[1].visual_bounds().unwrap();
    assert!((b.x0 - 170.0).abs() < 0.01 && (b.y0 - 70.0).abs() < 0.01 && (b.x1 - 190.0).abs() < 0.01, "{b:?}");
}

#[test]
fn spot_colours_are_custom_colours() {
    let mut doc = Document::new(100.0, 100.0);
    doc.swatches.push(vectorcraft_color::Swatch {
        name: "Spot Test".into(),
        paint: Paint::solid(Color::cmyk(0.0, 0.5, 1.0, 0.0)),
        global: true,
        spot: true,
    });
    let fill = Paint::Solid { color: Color::cmyk(0.0, 0.25, 0.5, 0.0), swatch: Some("Spot Test".into()), tint: 0.5 };
    let n = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), fill, Paint::None);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = write(&doc);
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    assert!(text.contains("%%DocumentCustomColors: (Spot Test)"), "{text}");
    assert!(text.contains("0 0.5 1 0 (Spot Test) 0.5 x"), "{text}");
    let doc = crate::import(&out.bytes).unwrap().document;
    let n = &children(&doc.layers[0])[0];
    let Paint::Solid { swatch, tint, .. } = n.appearance.fill_paint() else { panic!() };
    assert_eq!(swatch.as_deref(), Some("Spot Test"));
    assert!((tint - 0.5).abs() < 1e-6);
}

#[test]
fn losses_are_warned() {
    let mut doc = sample();
    // A sublayer, a hidden object, a named object, type and half opacity.
    let back = doc.layers[0].id;
    let sub = Node::layer(doc.alloc_id(), "Sub", vectorcraft_doc::LayerColor::Preset(2));
    let sub = doc.insert(Some(back), usize::MAX, sub).unwrap();
    let mut hidden = shape(&mut doc, shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), red(), Paint::None);
    hidden.visible = false;
    doc.insert(Some(sub), usize::MAX, hidden).unwrap();
    let mut named = shape(&mut doc, shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), red(), Paint::None);
    named.name = Some("Named".into());
    named.opacity = 0.5;
    doc.insert(Some(sub), usize::MAX, named).unwrap();
    let w = write(&doc).warnings;
    for expected in [SUBLAYERS, HIDDEN_LAYER, NAMES, TRANSPARENCY] {
        assert!(w.iter().any(|x| x == expected), "{expected} not in {w:?}");
    }
    // The sublayer is a group in its layer.
    let doc = crate::import(&write(&doc).bytes).unwrap().document;
    let last = children(&doc.layers[0]).last().unwrap();
    assert!(matches!(&last.kind, NodeKind::Group { children, .. } if children.len() == 1), "{:?}", last.kind);
}

#[test]
fn several_fills_and_odd_strokes_become_objects() {
    let mut doc = Document::new(100.0, 100.0);
    let mut n = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), red(), Paint::solid(Color::gray(1.0)));
    let mut a = (*n.appearance).clone();
    if let Some(AppearanceItem::Stroke(s)) = a.items.iter_mut().find(|i| !i.is_fill()) {
        s.align = vectorcraft_doc::StrokeAlign::Inside;
    }
    n.appearance = a.into();
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = write(&doc);
    assert!(out.warnings.iter().any(|w| w.contains("filled outlines")), "{:?}", out.warnings);
    let doc = crate::import(&out.bytes).unwrap().document;
    let n = &children(&doc.layers[0])[0];
    // The fill, then the stroke's outline filled.
    assert!(matches!(&n.kind, NodeKind::Group { children, .. } if children.len() == 2), "{:?}", n.kind);
}

#[test]
fn art_stays_put_on_an_artboard_a_fraction_of_a_point_tall() {
    let mut doc = Document::new(100.0, 364.252);
    let n = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 20.0, 30.0, 40.0)), red(), Paint::None);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let text = String::from_utf8(write(&doc).bytes).unwrap();
    // The artboard's top is on a whole point (365); the template box's centre is the artboard's.
    assert!(text.contains("%AI3_TemplateBox: 50 182.874 50 182.874\n"), "{text}");
    assert!(text.contains("10 345 m\n"), "{text}");
    let doc = crate::import(text.as_bytes()).unwrap().document;
    let b = children(&doc.layers[0])[0].visual_bounds().unwrap();
    assert!((b.x0 - 10.0).abs() < 1e-3 && (b.y0 - 20.0).abs() < 1e-3 && (b.y1 - 40.0).abs() < 1e-3, "{b:?}");
}

/// The file prints as the document with every layer shown: run as a PostScript program
/// ([`write_with`] checks it), its paths, fills, strokes, compound path, clipping group and image
/// draw where and as the document has them (its gradient filled with its first stop's colour, as
/// the prolog fills gradients). Read for its layers, it is read from its program, which has the
/// gradient.
#[test]
fn prints_as_the_document() {
    let mut doc = sample();
    if let Some(l) = doc.layers.get_mut(1) {
        Arc::make_mut(l).visible = true;
    }
    let out = write(&doc);
    let read = crate::import(&out.bytes).unwrap().document;
    let mut gradients = 0;
    read.walk(|n| {
        gradients += n.appearance.items.iter().filter(|i| matches!(i, AppearanceItem::Fill(f) if matches!(f.paint, Paint::Gradient(_)))).count()
    });
    assert_eq!((read.layers.len(), gradients), (2, 1));
}

#[test]
fn needs_an_artboard() {
    let mut doc = Document::new(100.0, 100.0);
    doc.artboards.clear();
    assert!(export_legacy_ai(&doc, &LegacyAiOptions::default()).is_err());
}

/// Every text object of `doc`'s layers, in paint order.
fn texts(doc: &Document) -> Vec<(Node, vectorcraft_doc::TextObject)> {
    let mut out = vec![];
    doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            out.push((n.clone(), (**t).clone()));
        }
    });
    out
}

fn style(size: f64, fill: Paint) -> vectorcraft_doc::CharStyle {
    vectorcraft_doc::CharStyle { size, fill, ..vectorcraft_doc::CharStyle::default() }
}

/// A document of point type (two paragraphs, the second centred, accented letters and a euro
/// sign), area type in a 100 × 50 frame and type on a path starting a quarter of the way along.
fn typed() -> Document {
    use vectorcraft_doc::{Justify, ParaStyle, TextKind, TextObject, TextRun};
    let mut doc = Document::new(300.0, 200.0);
    let layer = doc.layers[0].id;
    let mut point = TextObject::point(vectorcraft_geom::Point::new(20.0, 40.0), "Héllo €5\nWorld", style(18.0, red()));
    point.runs.push(TextRun::new(" again", style(12.0, Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)))));
    point.set_paragraph_styles(vec![ParaStyle::default(), ParaStyle { justify: Justify::Center, ..ParaStyle::default() }]);
    let mut area = TextObject::point(vectorcraft_geom::Point::ZERO, "Area type flows in its frame", style(10.0, Paint::solid(Color::BLACK)));
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)) };
    area.xf = Affine::translate((20.0, 80.0));
    let mut on_path = TextObject::point(vectorcraft_geom::Point::ZERO, "Along a path", style(12.0, Paint::solid(Color::gray(0.5))));
    let mut line = PathData::new(vec![]);
    line.subpaths.push(vectorcraft_geom::SubPath::polyline(&[(150.0, 150.0).into(), (250.0, 150.0).into(), (250.0, 190.0).into()], false));
    on_path.kind = TextKind::OnPath { path: line, start: 0.25, end: None };
    on_path.xf = Affine::IDENTITY;
    for t in [point, area, on_path] {
        let n = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    doc
}

#[test]
fn type_is_written_as_text_objects() {
    let out = write(&typed());
    let text = String::from_utf8(out.bytes).unwrap();
    for needle in [
        "%%DocumentFonts: ",
        "%%IncludeFont: ",
        "\nTE\n",
        "%AI3_BeginEncoding: _",
        "0 To\n1 0 0 1 20 160 0 Tp\nTP\n",
        "1 To\n",
        "2 To\n",
        " Tf\n",
        "TO\n",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in\n{text}");
    }
    // Windows-1252: é is 351 (octal), the euro sign 200, the paragraph ends with `\r`.
    assert!(text.contains("(H\\351llo \\2005\\r) Tx\n"), "{text}");
    // The second paragraph centred.
    assert!(text.contains("1 Ta\n"), "{text}");
    // The area's frame in art space, painted with nothing.
    assert!(text.contains("20 120 m\n"), "{text}");
    assert!(!out.warnings.iter().any(|w| w == TYPE), "{:?}", out.warnings);
}

#[test]
fn type_reads_back_as_text() {
    use vectorcraft_doc::{Justify, TextKind};
    let imported = crate::import(&write(&typed()).bytes).unwrap();
    let found = texts(&imported.document);
    assert_eq!(found.len(), 3, "{:?}", imported.warnings);
    let squeeze = |s: &str| s.replace(' ', "");
    // Point type: the characters, paragraphs, sizes, colours and place.
    let (_, point) = &found[0];
    assert!(matches!(point.kind, TextKind::Point));
    assert_eq!(point.plain_text(), "Héllo €5\nWorld again");
    assert_eq!(point.runs.len(), 2, "{:?}", point.runs);
    let first = &point.runs[0].style;
    assert_eq!(squeeze(&first.font_family), "SourceSans3", "{first:?}");
    assert!((first.size - 18.0).abs() < 1e-6 && first.leading.is_none(), "{first:?}");
    assert_eq!(first.fill.color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(point.runs[1].style.fill.color(), Some(Color::cmyk(1.0, 0.0, 0.0, 0.0)));
    assert!((point.runs[1].style.size - 12.0).abs() < 1e-6);
    assert_eq!(point.para_at(0).justify, Justify::Left);
    assert_eq!(point.para_at(1).justify, Justify::Center);
    let origin = point.xf * vectorcraft_geom::Point::ZERO;
    assert!((origin.x - 20.0).abs() < 1e-3 && (origin.y - 40.0).abs() < 1e-3, "{:?}", point.xf);
    // Area type: its frame where it was.
    let (_, area) = &found[1];
    let TextKind::Area { frame } = &area.kind else { panic!("{:?}", area.kind) };
    let b = frame.transformed(area.xf).to_bezpath().bounding_box();
    assert!((b.x0 - 20.0).abs() < 1e-3 && (b.y0 - 80.0).abs() < 1e-3 && (b.x1 - 120.0).abs() < 1e-3 && (b.y1 - 130.0).abs() < 1e-3, "{b:?}");
    assert_eq!(area.plain_text(), "Area type flows in its frame");
    // Type on a path: its path and where along it the type starts.
    let (_, along) = &found[2];
    let TextKind::OnPath { path, start, .. } = &along.kind else { panic!("{:?}", along.kind) };
    assert!((start - 0.25).abs() < 1e-2, "{start}");
    let b = path.transformed(along.xf).to_bezpath().bounding_box();
    assert!((b.x0 - 150.0).abs() < 1e-3 && (b.y1 - 190.0).abs() < 1e-3, "{b:?}");
}

#[test]
fn type_8_bit_text_cant_hold_and_type_asked_for_as_outlines_are_outlines() {
    use vectorcraft_doc::TextObject;
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    let t = TextObject::point(vectorcraft_geom::Point::new(10.0, 50.0), "Ωmega 日本", style(12.0, red()));
    let n = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = write(&doc);
    assert!(out.warnings.iter().any(|w| w == TYPE), "{:?}", out.warnings);
    assert!(!String::from_utf8_lossy(&out.bytes).contains(" To\n"));
    // Type that could be text, asked for as outlines.
    let out = write_with(&typed(), LegacyAiOptions { outline_text: true, ..LegacyAiOptions::default() });
    let text = String::from_utf8(out.bytes).unwrap();
    assert!(!text.contains(" To\n") && !text.contains("%%DocumentFonts"), "{text}");
    assert!(!out.warnings.iter().any(|w| w == TYPE), "{:?}", out.warnings);
}

/// A layer `Art` holding a group of a shown square and a hidden one; a hidden layer `Off` holding
/// a hidden square.
fn with_hidden() -> Document {
    let mut doc = Document::new(200.0, 100.0);
    doc.layers.clear();
    let art = doc.add_layer(Some("Art"));
    let off = doc.add_layer(Some("Off"));
    let shown = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), red(), Paint::None);
    let mut hidden = shape(&mut doc, shapes::rectangle(Rect::new(50.0, 10.0, 80.0, 40.0)), red(), Paint::None);
    hidden.visible = false;
    hidden.locked = true;
    let group = Node::group(doc.alloc_id(), vec![Arc::new(shown), Arc::new(hidden)]);
    doc.insert(Some(art), usize::MAX, group).unwrap();
    let mut also = shape(&mut doc, shapes::rectangle(Rect::new(100.0, 10.0, 140.0, 40.0)), red(), Paint::None);
    also.visible = false;
    doc.insert(Some(off), usize::MAX, also).unwrap();
    if let Some(l) = doc.layers.get_mut(1) {
        Arc::make_mut(l).visible = false;
    }
    doc
}

#[test]
fn hidden_objects_go_to_a_hidden_layer_above_their_own() {
    let out = write(&with_hidden());
    assert!(out.warnings.iter().any(|w| w == HIDDEN_LAYER), "{:?}", out.warnings);
    let doc = crate::import(&out.bytes).unwrap().document;
    let names: Vec<_> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Art", "Art (hidden)", "Off"]);
    assert!(doc.layers[0].visible && !doc.layers[1].visible && !doc.layers[2].visible);
    // The group keeps the shown square; the hidden one, still locked, is in the hidden layer.
    let group = &children(&doc.layers[0])[0];
    assert_eq!(children(group).len(), 1);
    let moved = children(&doc.layers[1]);
    assert_eq!(moved.len(), 1);
    assert!(moved[0].locked);
    let b = moved[0].visual_bounds().unwrap();
    assert!((b.x0 - 50.0).abs() < 0.01, "{b:?}");
    // In a hidden layer, hidden objects stay where they are.
    assert_eq!(children(&doc.layers[2]).len(), 1);
}

#[test]
fn hidden_objects_can_be_left_out_or_shown() {
    let out = write_with(&with_hidden(), LegacyAiOptions { hidden: HiddenObjects::Omit, ..LegacyAiOptions::default() });
    assert!(out.warnings.iter().any(|w| w == HIDDEN), "{:?}", out.warnings);
    let doc = crate::import(&out.bytes).unwrap().document;
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(children(&children(&doc.layers[0])[0]).len(), 1);
    let out = write_with(&with_hidden(), LegacyAiOptions { hidden: HiddenObjects::Visible, ..LegacyAiOptions::default() });
    assert!(out.warnings.iter().any(|w| w == HIDDEN_SHOWN), "{:?}", out.warnings);
    let doc = crate::import(&out.bytes).unwrap().document;
    assert_eq!(doc.layers.len(), 2);
    let pair = children(&children(&doc.layers[0])[0]);
    assert_eq!(pair.len(), 2);
    assert!(pair.iter().all(|n| n.visible));
}
