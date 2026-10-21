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
/// hidden objects shown when `o` writes them visible, and its gradient fills and strokes painted
/// in their first stop's color.
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
                let paint = match item {
                    AppearanceItem::Fill(f) => &mut f.paint,
                    AppearanceItem::Stroke(s) => &mut s.paint,
                };
                if let Paint::Gradient(g) = paint {
                    *paint = g.gradient.stops.first().map_or(Paint::None, |s| Paint::solid(s.color));
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

/// A 16 × 16 RGB image of `doc` filling `r`.
fn picture(doc: &mut Document, r: Rect) -> Node {
    let mut png = vec![];
    image::RgbaImage::from_fn(16, 16, |u, v| image::Rgba([(u * 16) as u8, (v * 16) as u8, 128, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let blob = ImageBlob::new("image/png", png);
    let key = blob.content_key();
    doc.images.insert(key.clone(), blob);
    let xf = Affine::new([r.width() / 16.0, 0.0, 0.0, r.height() / 16.0, r.x0, r.y0]);
    Node::new(doc.alloc_id(), NodeKind::Image(Box::new(ImageObject { key, width: 16, height: 16, xf, link: None, placement: Default::default() })))
}

/// A layer `Below` holding a clipping group (a circle clipping an image, as in #2692), a group
/// holding an image and an image; a layer `Above` holding an orange square. Without `images`,
/// `Below` is empty.
fn images_below_a_square(images: bool) -> Document {
    let mut doc = Document::new(400.0, 200.0);
    doc.layers.clear();
    let below = doc.add_layer(Some("Below"));
    let above = doc.add_layer(Some("Above"));
    if images {
        let clip = Node::new(
            doc.alloc_id(),
            NodeKind::Path {
                path: shapes::ellipse(Rect::new(30.0, 30.0, 170.0, 170.0)),
                rule: FillRule::NonZero,
                live: None,
                clipping: true,
                guide: false,
            },
        );
        let clipped = picture(&mut doc, Rect::new(20.0, 20.0, 180.0, 180.0));
        let window = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(clipped)], clip: true });
        doc.insert(Some(below), usize::MAX, window).unwrap();
        let grouped = picture(&mut doc, Rect::new(200.0, 20.0, 240.0, 60.0));
        let group = Node::group(doc.alloc_id(), vec![Arc::new(grouped)]);
        doc.insert(Some(below), usize::MAX, group).unwrap();
        let loose = picture(&mut doc, Rect::new(250.0, 20.0, 290.0, 60.0));
        doc.insert(Some(below), usize::MAX, loose).unwrap();
    }
    let orange = Paint::solid(Color::rgb(247.0 / 255.0, 148.0 / 255.0, 29.0 / 255.0));
    let square = shape(&mut doc, shapes::rectangle(Rect::new(220.0, 100.0, 380.0, 180.0)), orange, Paint::None);
    doc.insert(Some(above), usize::MAX, square).unwrap();
    doc
}

/// Each image the file holds (§8.4): the lines from `%AI5_File:` to the line after
/// `%AI5_EndRaster`.
fn rasters(text: &str) -> Vec<Vec<&str>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = vec![];
    for (i, l) in lines.iter().enumerate() {
        if *l == "%AI5_File:" {
            let end = lines[i..].iter().position(|l| *l == "%AI5_EndRaster").map(|n| i + n).unwrap();
            out.push(lines[i..=(end + 1).min(lines.len() - 1)].to_vec());
        }
    }
    out
}

/// Every image, in a clipping group, a group or neither, is written as §8.1's syntax and §8.4's
/// examples write one: `%AI5_File:`, `%AI5_BeginRaster`, the matrix with a space inside each
/// bracket and the operands on the `XI` line, the samples in hex on comment lines of 60 digits,
/// `%AI5_EndRaster`. A line `F`, which the examples do not show, follows each image. The art
/// after the images is written as it is without them, and the file reads back with its layers,
/// clipping group, images and square (#2692, #2830).
#[test]
fn images_are_written_in_the_form_the_specification_shows() {
    let out = write(&images_below_a_square(true));
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    let images = rasters(&text);
    assert_eq!(images.len(), 3, "{text}");
    for lines in &images {
        let [file, begin, xi, data @ .., end, paint] = lines.as_slice() else { panic!("{lines:?}") };
        assert_eq!((*file, *begin, *end, *paint), ("%AI5_File:", "%AI5_BeginRaster", "%AI5_EndRaster", "F"));
        let matrix = xi.strip_prefix("[ ").and_then(|l| l.strip_suffix(" ] 0 0 16 16 16 16 8 3 0 0 0 0 XI")).unwrap_or_else(|| panic!("{xi}"));
        assert_eq!(matrix.split(' ').filter(|v| v.parse::<f64>().is_ok()).count(), 6, "{xi}");
        let digits: Vec<&str> = data.iter().map(|l| l.strip_prefix('%').unwrap_or_else(|| panic!("{l}"))).collect();
        let (last, full) = digits.split_last().unwrap();
        assert!(full.iter().all(|d| d.len() == 60) && (1..=60).contains(&last.len()), "{digits:?}");
        assert!(digits.iter().all(|d| d.bytes().all(|b| b.is_ascii_hexdigit())), "{digits:?}");
        assert_eq!(digits.concat().len(), 16 * 16 * 3 * 2);
    }
    // The clipping group: `q`, the clipping path painted `h W n` (§5.6.2), the image, `Q`; the
    // group: `u`, the image, `U`; the loose image, then the layer's end.
    assert!(text.contains("\nh\nW\nn\n%AI5_File:\n") && text.contains("\n%AI5_EndRaster\nF\nQ\n"), "{text}");
    assert!(text.contains("\nu\n%AI5_File:\n") && text.contains("\n%AI5_EndRaster\nF\nU\n"), "{text}");
    assert!(text.contains("\n%AI5_EndRaster\nF\nLB\n"), "{text}");
    let layer = |t: &str| -> String {
        let at = t.find("(Above) Ln\n").unwrap();
        t[at..].split("%AI5_EndLayer--").next().unwrap().to_string()
    };
    let alone = String::from_utf8(write(&images_below_a_square(false)).bytes).unwrap();
    assert_eq!(layer(&text), layer(&alone));
    let doc = crate::import(&out.bytes).unwrap().document;
    let names: Vec<_> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Below", "Above"]);
    let below = children(&doc.layers[0]);
    assert_eq!(below.len(), 3, "{below:#?}");
    let NodeKind::Group { children: window, clip: true } = &below[0].kind else { panic!("{:?}", below[0].kind) };
    assert_eq!(window.len(), 2, "{window:#?}");
    assert!(matches!(window[0].kind, NodeKind::Path { clipping: true, .. }) && matches!(window[1].kind, NodeKind::Image(_)), "{window:#?}");
    assert!(
        matches!(&below[1].kind, NodeKind::Group { children, clip: false } if children.len() == 1 && matches!(children[0].kind, NodeKind::Image(_)))
    );
    assert!(matches!(below[2].kind, NodeKind::Image(_)));
    let above = children(&doc.layers[1]);
    assert_eq!(above.len(), 1);
    let rgb = above[0].appearance.fill_paint().color().map(|c| c.to_rgb()).unwrap();
    assert!(rgb.iter().zip([247.0, 148.0, 29.0]).all(|(c, want)| (c * 255.0 - want).abs() < 0.1), "{rgb:?}");
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

/// A gradient from white to #1b2a6b, the colors of #2687's circle, placed at `geom` (None: fitted
/// to the object).
fn white_to_navy(kind: GradientKind, geom: Option<vectorcraft_color::GradientGeom>) -> Paint {
    let navy = Color::rgb(27.0 / 255.0, 42.0 / 255.0, 107.0 / 255.0);
    let g = Gradient { kind, stops: vec![GradientStop::new(0.0, Color::rgb(1.0, 1.0, 1.0)), GradientStop::new(1.0, navy)] };
    Paint::Gradient(Box::new(GradientPaint { geom, ..GradientPaint::new(g) }))
}

/// A 400 × 400 document: #2687's radial circle (center 100, 100, radius 80) and linear square;
/// below them a radial square in a group, a radial compound ring, a clipping group whose clipping
/// path and member are radial, a square stroked radial, and a radial square whose elliptical
/// gradient has its focal point 30 pt above the center, past its 20 pt radius.
fn radials() -> Document {
    let mut doc = Document::new(400.0, 400.0);
    let layer = doc.layers[0].id;
    let radial = || white_to_navy(GradientKind::Radial, None);
    let circle = shape(&mut doc, shapes::ellipse(Rect::new(20.0, 20.0, 180.0, 180.0)), radial(), Paint::None);
    let square = shape(&mut doc, shapes::rectangle(Rect::new(220.0, 20.0, 380.0, 180.0)), white_to_navy(GradientKind::Linear, None), Paint::None);
    let member = shape(&mut doc, shapes::rectangle(Rect::new(20.0, 220.0, 60.0, 260.0)), radial(), Paint::None);
    let group = Node::group(doc.alloc_id(), vec![Arc::new(member)]);
    let outer = shape(&mut doc, shapes::rectangle(Rect::new(80.0, 220.0, 120.0, 260.0)), Paint::None, Paint::None);
    let inner = shape(&mut doc, shapes::rectangle(Rect::new(90.0, 230.0, 110.0, 250.0)), Paint::None, Paint::None);
    let mut ring = Node::new(doc.alloc_id(), NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(inner)], rule: FillRule::EvenOdd });
    ring.appearance = Appearance::basic(radial(), Paint::None, 0.0).into();
    let mut clip = Node::new(
        doc.alloc_id(),
        NodeKind::Path {
            path: shapes::rectangle(Rect::new(140.0, 220.0, 180.0, 260.0)),
            rule: FillRule::NonZero,
            live: None,
            clipping: true,
            guide: false,
        },
    );
    clip.appearance = Appearance::basic(radial(), Paint::None, 0.0).into();
    let clipped = shape(&mut doc, shapes::rectangle(Rect::new(130.0, 210.0, 190.0, 270.0)), radial(), Paint::None);
    let window = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(clipped)], clip: true });
    let stroked = shape(&mut doc, shapes::rectangle(Rect::new(200.0, 220.0, 240.0, 260.0)), Paint::None, radial());
    let geom = vectorcraft_color::GradientGeom {
        start: vectorcraft_geom::Point::new(300.0, 240.0),
        end: vectorcraft_geom::Point::new(320.0, 240.0),
        aspect: 2.0,
        focal: Some(vectorcraft_geom::Point::new(300.0, 210.0)),
    };
    let off_center =
        shape(&mut doc, shapes::rectangle(Rect::new(260.0, 220.0, 340.0, 260.0)), white_to_navy(GradientKind::Radial, Some(geom)), Paint::None);
    for n in [circle, square, group, ring, window, stroked, off_center] {
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    doc
}

/// Every radial gradient instance has its highlight `Bh` (§3.5.5): `0 0 0 0 Bh` when it is at the
/// center (§3.5.6), on fills, strokes, in groups, compound paths and clipping groups, and an
/// off-center one within the gradient's circle. Linear instances have none. VectorCraft reads
/// back one linear and at least seven radial gradients, each with its stops, and the circle, the
/// square and the off-center square in their place.
#[test]
fn radial_gradients_have_their_highlight() {
    let out = write(&radials());
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    // One definition for the radial gradient, one for the linear one (§3.5.2).
    assert!(text.contains("2 Bn\n%AI5_BeginGradient: (Gradient)\n(Gradient) 1 2 Bd\n[\n"), "{text}");
    assert!(text.contains("%AI5_BeginGradient: (Gradient 2)\n(Gradient 2) 0 2 Bd\n[\n"), "{text}");
    // Each instance, `Bb` to `BB`.
    let instances: Vec<&str> = text.split("Bb\n").skip(1).filter_map(|t| t.split_inclusive("BB\n").next()).collect();
    let radial: Vec<&str> = instances.iter().copied().filter(|i| i.contains(" (Gradient) ")).collect();
    let linear: Vec<&str> = instances.iter().copied().filter(|i| i.contains(" (Gradient 2) ")).collect();
    assert_eq!((radial.len() + linear.len(), linear.len()), (instances.len(), 1), "{instances:#?}");
    assert!(radial.len() >= 8, "{radial:#?}");
    for i in &radial {
        let bh = i.lines().next().unwrap_or_default();
        assert!(bh.ends_with(" Bh") && bh.split_whitespace().count() == 5, "{i}");
    }
    assert!(!linear[0].contains("Bh"), "{}", linear[0]);
    for needle in [
        // #2687's circle and square.
        "\nBb\n0 0 0 0 Bh\n1 (Gradient) 100 300 0 80 1 0 0 1 0 0 Bg\nf\n0 BB\n",
        "\nBb\n1 (Gradient 2) 220 300 0 160 1 0 0 1 0 0 Bg\nf\n0 BB\n",
        // In a group.
        "\nBb\n0 0 0 0 Bh\n1 (Gradient) 40 160 0 20 1 0 0 1 0 0 Bg\nf\n0 BB\nU\n",
        // Both subpaths of the compound ring (§5.3.5).
        "\nBb\n0 0 0 0 Bh\n1 (Gradient) 100 160 0 20 1 0 0 1 0 0 Bg\nf\n0 BB\n90 170 m\n",
        "\nBb\n0 0 0 0 Bh\n1 (Gradient) 100 160 0 20 1 0 0 1 0 0 Bg\nf\n0 BB\n*U\n",
        // The clipping path's fill and the clipped square (§5.6.2).
        "\nBb\n0 0 0 0 Bh\n1 (Gradient) 160 160 0 20 1 0 0 1 0 0 Bg\nf\n0 BB\n",
        "\nBb\n0 0 0 0 Bh\n1 (Gradient) 160 160 0 30 1 0 0 1 0 0 Bg\nf\n0 BB\nQ\n",
        // The stroke, written as its outline filled.
        "\nBb\n0 0 0 0 Bh\n1 (Gradient) 220 160 0 21 1 0 0 1 0 0 Bg\nf\n0 BB\n",
        // The focal point 30 pt up, 1.5 radii out: written on the circle, straight up.
        "\nBb\n0 20 90 1 Bh\n1 (Gradient) 300 160 0 20 1 0 0 1 0 0 Bg\nf\n0 BB\n",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in\n{text}");
    }
    for expected in [STROKES, ELLIPTICAL] {
        assert!(out.warnings.iter().any(|w| w == expected), "{expected} not in {:?}", out.warnings);
    }
    // Read back: every gradient keeps its stops, the document holds the linear square and at
    // least seven radial gradients, and the circle, the square and the off-center square keep
    // their place.
    let doc = crate::import(&out.bytes).unwrap().document;
    let mut found = vec![];
    doc.walk(|n| {
        for i in &n.appearance.items {
            if let AppearanceItem::Fill(f) = i
                && let Paint::Gradient(g) = &f.paint
            {
                found.push((**g).clone());
            }
        }
    });
    let kinds = |k: GradientKind| found.iter().filter(|g| g.gradient.kind == k).count();
    // Seven radial objects (a compound path's members carry its fill too) and the linear square.
    assert!(kinds(GradientKind::Radial) >= 7 && kinds(GradientKind::Linear) == 1, "{found:#?}");
    let near = |a: vectorcraft_geom::Point, x: f64, y: f64| (a.x - x).abs() < 1e-3 && (a.y - y).abs() < 1e-3;
    for g in &found {
        let [white, navy] = g.gradient.stops.as_slice() else { panic!("{g:?}") };
        assert_eq!((white.offset, navy.offset, white.color), (0.0, 1.0, Color::rgb(1.0, 1.0, 1.0)), "{g:?}");
        let rgb = navy.color.to_rgb();
        assert!([27.0, 42.0, 107.0].iter().zip(rgb).all(|(want, got)| (want / 255.0 - got).abs() < 1e-3), "{rgb:?}");
    }
    let placed = |x: f64, y: f64| found.iter().filter_map(|g| g.geom.map(|m| (g.gradient.kind, m))).find(|(_, m)| near(m.start, x, y));
    let Some((GradientKind::Radial, circle)) = placed(100.0, 100.0) else { panic!("{found:#?}") };
    assert!(near(circle.end, 180.0, 100.0) && circle.focal.is_none(), "{circle:?}");
    let Some((GradientKind::Linear, square)) = placed(220.0, 100.0) else { panic!("{found:#?}") };
    assert!(near(square.end, 380.0, 100.0), "{square:?}");
    let Some((GradientKind::Radial, off)) = placed(300.0, 240.0) else { panic!("{found:#?}") };
    assert!(near(off.end, 320.0, 240.0) && off.focal.is_some_and(|f| near(f, 300.0, 220.0)), "{off:?}");
}

/// A radial gradient's stops are listed from the first to the last, as §3.5.2's radial definition
/// lists them (10, then 100), and a linear gradient's stops are listed from the last to the first
/// (§1.4.3). Each stop keeps its color and its midpoint to the next stop. The reference app read a
/// radial gradient's stops listed from the last as one color (two stops) or inverted (three
/// stops), reported on #2807. Both gradients read back with their stops.
#[test]
fn radial_gradient_stops_are_listed_from_the_first() {
    let white = Color::rgb(1.0, 1.0, 1.0);
    let red = Color::rgb(1.0, 0.0, 0.0);
    let navy = Color::rgb(27.0 / 255.0, 42.0 / 255.0, 107.0 / 255.0);
    let stop = |offset: f32, color: Color, midpoint: f32| GradientStop { midpoint, ..GradientStop::new(offset, color) };
    let stops = vec![stop(0.0, white, 0.3), stop(0.3, red, 0.7), stop(1.0, navy, 0.5)];
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    for (kind, x) in [(GradientKind::Radial, 10.0), (GradientKind::Linear, 110.0)] {
        let paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient { kind, stops: stops.clone() })));
        let n = shape(&mut doc, shapes::rectangle(Rect::new(x, 10.0, x + 80.0, 90.0)), paint, Paint::None);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let out = write(&doc);
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    // Each `%_Bs` line of a definition: its RGB color (`c m y k r g b 2`), midpoint and ramp point.
    let listed = |head: &str| -> Vec<([f64; 3], f64, f64)> {
        let def = text.split(head).nth(1).and_then(|t| t.split("BD\n").next()).unwrap_or_else(|| panic!("no {head:?} in\n{text}"));
        def.lines()
            .map(|l| {
                let n: Vec<f64> = l.trim_end_matches(" %_Bs").split_whitespace().map(|v| v.parse().unwrap()).collect();
                let [_, _, _, _, r, g, b, style, mid, ramp] = n.as_slice() else { panic!("{l}") };
                assert_eq!(*style, 2.0, "{l}");
                ([*r, *g, *b], *mid, *ramp)
            })
            .collect()
    };
    let rgb = |c: Color| c.to_rgb().map(f64::from);
    let first_first = [(rgb(white), 30.0, 0.0), (rgb(red), 70.0, 30.0), (rgb(navy), 50.0, 100.0)];
    let mut last_first = first_first;
    last_first.reverse();
    let same = |got: &[([f64; 3], f64, f64)], want: &[([f64; 3], f64, f64)]| {
        got.len() == want.len()
            && got.iter().zip(want).all(|((c, m, r), (wc, wm, wr))| {
                c.iter().zip(wc).all(|(a, b)| (a - b).abs() < 1e-3) && (m - wm).abs() < 1e-3 && (r - wr).abs() < 1e-3
            })
    };
    let radial = listed("(Gradient) 1 3 Bd\n[\n");
    assert!(same(&radial, &first_first), "{radial:?}\n{text}");
    let linear = listed("(Gradient 2) 0 3 Bd\n[\n");
    assert!(same(&linear, &last_first), "{linear:?}\n{text}");
    // Read back: both gradients, each stop at its offset with its color and midpoint.
    let doc = crate::import(&out.bytes).unwrap().document;
    let read: Vec<_> = children(&doc.layers[0])
        .iter()
        .filter_map(|n| if let Paint::Gradient(g) = n.appearance.fill_paint() { Some(g.gradient.clone()) } else { None })
        .collect();
    assert_eq!(read.iter().map(|g| g.kind).collect::<Vec<_>>(), [GradientKind::Radial, GradientKind::Linear], "{read:#?}");
    for g in &read {
        let got: Vec<_> = g.stops.iter().map(|s| (rgb(s.color), f64::from(s.midpoint) * 100.0, f64::from(s.offset) * 100.0)).collect();
        assert!(same(&got, &first_first), "{g:#?}");
    }
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

/// A document with a spot color swatch for each of `spots`, a square filled with each at half
/// tint, and a square whose linear gradient fill (the first spot color to white) names the
/// gradient swatch `gradient` (the document holds no such swatch; the writer reads only the
/// fill's name).
fn spots_and_gradient(spots: &[&str], gradient: &str) -> Document {
    let mut doc = Document::new(200.0, 100.0);
    let layer = doc.layers[0].id;
    for (i, name) in spots.iter().enumerate() {
        let c = i as f32 * 0.5;
        doc.swatches.push(vectorcraft_color::Swatch {
            name: (*name).into(),
            paint: Paint::solid(Color::cmyk(c, 0.5, 1.0, 0.0)),
            global: true,
            spot: true,
        });
        let fill = Paint::Solid { color: Color::cmyk(c / 2.0, 0.25, 0.5, 0.0), swatch: Some((*name).into()), tint: 0.5 };
        let x = 10.0 + 40.0 * i as f64;
        let n = shape(&mut doc, shapes::rectangle(Rect::new(x, 10.0, x + 30.0, 40.0)), fill, Paint::None);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }
    let mut first = GradientStop::new(0.0, Color::cmyk(0.0, 0.5, 1.0, 0.0));
    first.swatch = Some(spots[0].into());
    let g = Gradient { kind: GradientKind::Linear, stops: vec![first, GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 0.0, 0.0))] };
    let mut paint = GradientPaint::new(g);
    paint.swatch = Some(gradient.into());
    let n = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 50.0, 50.0, 90.0)), Paint::Gradient(Box::new(paint)), Paint::None);
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    doc
}

/// `doc` written with the title `title`, and run as a PostScript program ([`prints_as`]).
fn write_titled(doc: &Document, title: &str) -> LegacyAiOutput {
    let o = LegacyAiOptions { title: title.into(), created: Some(0), ..LegacyAiOptions::default() };
    let out = export_legacy_ai(doc, &o).unwrap();
    prints_as(&out.bytes, doc, &o);
    out
}

/// The spot colors and gradients of the objects of `doc`'s first layer, in paint order: each
/// solid fill's swatch, and each gradient fill's swatch with its first stop's.
fn inks(doc: &Document) -> Vec<Option<String>> {
    let mut out = vec![];
    for n in children(&doc.layers[0]) {
        match n.appearance.fill_paint() {
            Paint::Solid { swatch, .. } => out.push(swatch.clone()),
            Paint::Gradient(g) => {
                out.push(g.swatch.clone());
                out.push(g.gradient.stops.first().and_then(|s| s.swatch.clone()));
            }
            _ => {}
        }
    }
    out
}

/// The title, spot color names and gradient names are written in Windows-1252, as layer names
/// are: é is 351 (octal) in the header's custom colors, each `x`, the gradient's definition, its
/// stops and `Bg`. The spot color and gradient names read back as they were.
#[test]
fn spot_gradient_and_title_names_are_written_in_windows_1252() {
    let doc = spots_and_gradient(&["Caf\u{e9}"], "D\u{e9}grad\u{e9}");
    let out = write_titled(&doc, "R\u{e9}sum\u{e9}");
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    for needle in [
        "%%Title: (R\\351sum\\351)\n",
        "%%DocumentCustomColors: (Caf\\351)\n",
        "%%CMYKCustomColor: 0 0.5 1 0 (Caf\\351)\n",
        "0 0.5 1 0 (Caf\\351) 0.5 x\n",
        "%AI5_BeginGradient: (D\\351grad\\351)\n(D\\351grad\\351) 0 2 Bd\n",
        "0 0.5 1 0 (Caf\\351) 0 3 50 0 %_Bs\n",
        "1 (D\\351grad\\351) ",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in\n{text}");
    }
    assert!(!text.contains("\\303"), "{text}");
    assert!(!out.warnings.iter().any(|w| w == TITLE_AND_COLOR_NAMES), "{:?}", out.warnings);
    let read = crate::import(&out.bytes).unwrap().document;
    let names = inks(&read);
    let want = ["Caf\u{e9}", "D\u{e9}grad\u{e9}", "Caf\u{e9}"].map(|n| Some(n.to_string()));
    assert_eq!(names, want);
}

/// A character Windows-1252 can't hold is written as `?`, with one warning for all of these
/// names. Names that become the same are numbered, so the header, each `x` and each `Bg` still
/// tell the colors and gradients apart.
#[test]
fn names_outside_windows_1252_are_replaced_and_kept_apart() {
    // 赤 and 青 (spot colors), 渐变 and グラ (gradients), 日本 (the title).
    let mut doc = spots_and_gradient(&["\u{8d64}", "\u{9752}"], "\u{6e10}\u{53d8}");
    let g = Gradient {
        kind: GradientKind::Linear,
        stops: vec![GradientStop::new(0.0, Color::cmyk(1.0, 0.0, 0.0, 0.0)), GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 0.0, 0.0))],
    };
    let mut paint = GradientPaint::new(g);
    paint.swatch = Some("\u{30b0}\u{30e9}".into());
    let n = shape(&mut doc, shapes::rectangle(Rect::new(60.0, 50.0, 100.0, 90.0)), Paint::Gradient(Box::new(paint)), Paint::None);
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let out = write_titled(&doc, "\u{65e5}\u{672c}");
    let text = String::from_utf8(out.bytes.clone()).unwrap();
    for needle in [
        "%%Title: (??)\n",
        "%%DocumentCustomColors: (?)\n%%+ (? 2)\n",
        "%%CMYKCustomColor: 0 0.5 1 0 (?)\n%%+ 0.5 0.5 1 0 (? 2)\n",
        "0 0.5 1 0 (?) 0.5 x\n",
        "0.5 0.5 1 0 (? 2) 0.5 x\n",
        "(??) 0 2 Bd\n",
        "(?? 2) 0 2 Bd\n",
        "0 0.5 1 0 (?) 0 3 50 0 %_Bs\n",
        "1 (??) ",
        "1 (?? 2) ",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in\n{text}");
    }
    assert_eq!(out.warnings.iter().filter(|w| *w == TITLE_AND_COLOR_NAMES).count(), 1, "{:?}", out.warnings);
    assert!(!out.warnings.iter().any(|w| w == LAYER_NAME), "{:?}", out.warnings);
    let read = crate::import(&out.bytes).unwrap().document;
    let want = [Some("?"), Some("? 2"), Some("??"), Some("?"), Some("?? 2"), None].map(|n| n.map(String::from));
    assert_eq!(inks(&read), want);
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

/// A legacy `.ai` text object names a font, not a setting of its variation axes: type with
/// Variable Font settings is written in the font's style, with the warning that they are left out.
#[test]
fn variable_font_settings_are_warned_as_left_out() {
    let mut doc = typed();
    assert!(!write(&doc).warnings.iter().any(|w| w == TYPE_ATTRIBUTES));
    let id = texts(&doc)[0].0.id;
    if let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) {
        t.runs[0].style.font_axes.insert("wght".into(), 650.0);
    }
    let out = write(&doc);
    assert!(out.warnings.iter().any(|w| w == TYPE_ATTRIBUTES), "{:?}", out.warnings);
    assert!(String::from_utf8(out.bytes).unwrap().contains("(H\\351llo \\2005\\r) Tx\n"), "still written as text");
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

/// Area type that Shrink Text to Fit scales down is written at the sizes, leading and baseline
/// shifts it draws at, so it reads back fitting its frame.
#[test]
fn shrunk_area_type_is_written_at_the_size_it_draws_at() {
    use vectorcraft_doc::{TextKind, TextObject, TextRun};
    let mut doc = Document::new(300.0, 200.0);
    let layer = doc.layers[0].id;
    let mut area = TextObject::point(vectorcraft_geom::Point::ZERO, "Area type that is too long ", style(14.0, Paint::solid(Color::BLACK)));
    let raised = vectorcraft_doc::CharStyle { baseline_shift: 3.0, leading: Some(20.0), ..style(14.0, red()) };
    area.runs.push(TextRun::new("for its frame shrinks", raised));
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 40.0)) };
    area.area.fit = vectorcraft_doc::AreaFit::ShrinkText { min_percent: 10.0 };
    area.xf = Affine::translate((20.0, 80.0));
    let fit = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &area).fit_scale;
    assert!(fit < 0.9, "the text is shrunk to fit: {fit}");
    let n = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(area)));
    doc.insert(Some(layer), usize::MAX, n).unwrap();
    let imported = crate::import(&write(&doc).bytes).unwrap();
    let found = texts(&imported.document);
    let (_, back) = found.first().expect("the area type reads back");
    let sizes: Vec<(f64, f64, Option<f64>)> = back.runs.iter().map(|r| (r.style.size, r.style.baseline_shift, r.style.leading)).collect();
    assert_eq!(sizes.len(), 2, "{sizes:?}");
    assert!((sizes[0].0 - 14.0 * fit).abs() < 1e-3 && (sizes[1].0 - 14.0 * fit).abs() < 1e-3, "{sizes:?} at {fit}");
    assert!((sizes[1].1 - 3.0 * fit).abs() < 1e-3, "{sizes:?} at {fit}");
    assert!(sizes[1].2.is_some_and(|l| (l - 20.0 * fit).abs() < 1e-3), "{sizes:?} at {fit}");
    assert!(!vectorcraft_text::layout(vectorcraft_text::FontDb::global(), back).overflow, "it fits its frame as written");
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

#[test]
fn layer_names_outside_latin_1_are_replaced_and_warned() {
    let layered = |name: &str| {
        let mut doc = Document::new(100.0, 100.0);
        doc.layers.clear();
        let layer = doc.add_layer(Some(name));
        let n = shape(&mut doc, shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), red(), Paint::None);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
        // A hidden object, so the layer also gets a hidden layer named after it.
        let mut hidden = shape(&mut doc, shapes::rectangle(Rect::new(50.0, 10.0, 80.0, 40.0)), red(), Paint::None);
        hidden.visible = false;
        doc.insert(Some(layer), usize::MAX, hidden).unwrap();
        write(&doc)
    };
    let out = layered("\u{b808}\u{c774}\u{c5b4} 1 Caf\u{e9}");
    let text = String::from_utf8_lossy(&out.bytes);
    assert!(text.contains("(??? 1 Caf\\351) Ln"), "{text}");
    assert!(text.contains("(??? 1 Caf\\351 \\(hidden\\)) Ln"), "{text}");
    assert!(out.warnings.iter().any(|w| w == LAYER_NAME), "{:?}", out.warnings);
    // A name Latin-1 can hold is written as it is, with no warning.
    assert!(!layered("Caf\u{e9}").warnings.iter().any(|w| w == LAYER_NAME));
}
