//! HP-GL export: the instructions written, pens, origins, what is cut and left out, limits, and
//! plots read back by the HP-GL reader.

use std::sync::Arc;

use kurbo::{PathEl, Shape as _};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, CharStyle, Document, LayerColor, Node, NodeKind, TextObject};
use vectorcraft_geom::{PathData, Point, Rect, shapes};

use super::*;

/// A 200 × 100 pt document whose first layer holds what `nodes` makes.
fn doc_with(nodes: impl FnOnce(&mut Document) -> Vec<Node>) -> Document {
    let mut d = Document::new(200.0, 100.0);
    let nodes = nodes(&mut d);
    let layer = d.layers[0].id;
    for n in nodes {
        d.insert(Some(layer), usize::MAX, n).unwrap();
    }
    d
}

fn stroked(d: &mut Document, p: PathData, c: Color) -> Node {
    Node::path(d.alloc_id(), p, Appearance::basic(Paint::None, Paint::solid(c), 1.0))
}

fn filled(d: &mut Document, p: PathData, c: Color) -> Node {
    Node::path(d.alloc_id(), p, Appearance::basic(Paint::solid(c), Paint::None, 1.0))
}

fn line(a: (f64, f64), b: (f64, f64)) -> PathData {
    let mut bp = BezPath::new();
    bp.move_to(a);
    bp.line_to(b);
    PathData::from_bezpath(&bp)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> PathData {
    shapes::rectangle(Rect::new(x0, y0, x1, y1))
}

/// The options with the first artboard as the region.
fn opts(d: &Document) -> HpglOptions {
    HpglOptions { region: d.artboards[0].rect, ..HpglOptions::default() }
}

fn plot(d: &Document, o: &HpglOptions) -> HpglOutput {
    match export(d, o) {
        Ok(out) => out,
        Err(e) => panic!("{e}"),
    }
}

fn text(out: &HpglOutput) -> String {
    String::from_utf8(out.bytes.clone()).expect("ASCII")
}

/// A black square outline, a red line and a blue ellipse filled without a stroke.
fn three() -> Document {
    doc_with(|d| {
        vec![
            stroked(d, rect(10.0, 10.0, 60.0, 40.0), Color::BLACK),
            stroked(d, line((70.0, 20.0), (150.0, 80.0)), Color::rgb8(255, 0, 0)),
            filled(d, shapes::ellipse(Rect::new(70.0, 30.0, 130.0, 70.0)), Color::rgb8(0, 0, 255)),
        ]
    })
}

/// Every instruction in the HP-GL reference guide's form: a two-letter mnemonic, integer
/// parameters separated by commas, and a semicolon (https://support.hp.com/sg-en/document/c01121263).
#[test]
fn a_plot_is_pen_moves_in_plotter_units_from_the_bottom_left() {
    let d = doc_with(|d| vec![stroked(d, rect(10.0, 10.0, 60.0, 40.0), Color::BLACK)]);
    let out = plot(&d, &opts(&d));
    // 1016 units to the inch, y up from the artboard's bottom edge; the square closes.
    assert_eq!(text(&out), "IN;PU;SP1;PU141,1270;PD847,1270,847,847,141,847,141,1270;PU;SP0;");
    for i in text(&out).split_terminator(';') {
        let (op, params) = i.split_at(2);
        assert!(op.bytes().all(|c| c.is_ascii_uppercase()), "{i}");
        assert!(params.is_empty() || params.split(',').all(|v| v.parse::<i64>().is_ok()), "{i}");
    }
    assert_eq!(out.pens, [[0, 0, 0]]);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn the_origin_can_be_the_center() {
    let d = doc_with(|d| vec![stroked(d, rect(10.0, 10.0, 60.0, 40.0), Color::BLACK)]);
    let out = plot(&d, &HpglOptions { origin: Origin::Center, ..opts(&d) });
    // The artboard's center is (100, 50): x from -1270, y from 564 (40 pt up).
    assert_eq!(text(&out), "IN;PU;SP1;PU-1270,564;PD-564,564,-564,141,-1270,141,-1270,564;PU;SP0;");
}

#[test]
fn each_new_color_is_the_next_pen_and_pens_draw_in_turn() {
    let d = doc_with(|d| {
        vec![
            stroked(d, line((10.0, 10.0), (20.0, 10.0)), Color::rgb8(255, 0, 0)),
            stroked(d, line((10.0, 20.0), (20.0, 20.0)), Color::BLACK),
            stroked(d, line((10.0, 30.0), (20.0, 30.0)), Color::rgb8(255, 0, 0)),
        ]
    });
    let out = plot(&d, &opts(&d));
    assert_eq!(out.pens, [[255, 0, 0], [0, 0, 0]]);
    assert_eq!(text(&out), "IN;PU;SP1;PU141,1270;PD282,1270;PU141,988;PD282,988;PU;SP2;PU141,1129;PD282,1129;PU;SP0;");
    // A pen table: each color takes the nearest pen's (red is as near black as blue, and black
    // comes first).
    let table = plot(&d, &HpglOptions { pens: vec![[0, 0, 0], [0, 0, 255]], ..opts(&d) });
    assert_eq!(table.pens, [[0, 0, 0], [0, 0, 255]]);
    assert!(text(&table).starts_with("IN;PU;SP1;PU141,1270;"), "{}", text(&table));
    assert!(!text(&table).contains("SP2"), "{}", text(&table));
}

#[test]
fn a_filled_path_is_its_closed_outline_and_a_stroke_its_line() {
    let d = doc_with(|d| {
        let mut open = BezPath::new();
        open.move_to((10.0, 10.0));
        open.line_to((20.0, 10.0));
        open.line_to((20.0, 20.0));
        let open = PathData::from_bezpath(&open);
        let both = Node::path(d.alloc_id(), open.clone(), Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::rgb8(255, 0, 0)), 1.0));
        vec![filled(d, open.clone(), Color::BLACK), stroked(d, open, Color::BLACK), both]
    });
    let out = plot(&d, &opts(&d));
    let t = text(&out);
    assert!(t.contains("PU141,1270;PD282,1270,282,1129,141,1270;"), "the fill closes: {t}");
    assert!(t.contains("PU141,1270;PD282,1270,282,1129;PU;"), "the stroke doesn't: {t}");
    // A path with a fill and a stroke is closed too, in its stroke's color.
    assert_eq!(out.pens, [[0, 0, 0], [255, 0, 0]]);
    assert!(t.ends_with("SP2;PU141,1270;PD282,1270,282,1129,141,1270;PU;SP0;"), "{t}");
}

/// A gradient mesh is its outline, once, in the average of its points' colors.
#[test]
fn a_gradient_mesh_is_its_outline() {
    let d = doc_with(|d| {
        let mesh = vectorcraft_doc::live::GradientMesh::from_surface(4, 4, &|u, v| Point::new(10.0 + u * 50.0, 10.0 + v * 50.0), &|u, _| {
            if u < 0.5 { Color::rgb8(255, 0, 0) } else { Color::rgb8(0, 0, 255) }
        });
        vec![Node::new(d.alloc_id(), NodeKind::Mesh(mesh))]
    });
    let out = plot(&d, &opts(&d));
    let t = text(&out);
    assert_eq!(t.matches("PD").count(), 1, "{t}");
    assert!(t.starts_with("IN;PU;SP1;PU"), "{t}");
    // The square's four corners are on the line it draws.
    for corner in ["141,1270", "847,1270", "847,564", "141,564"] {
        assert!(t.contains(corner), "{corner} in {t}");
    }
    let [r, g, b] = out.pens[0];
    assert!(r > 0 && b > 0 && g == 0, "{:?}", out.pens);
}

/// A square gradient mesh, 30 pt a side, at `x`.
fn mesh_at(x: f64) -> vectorcraft_doc::live::GradientMesh {
    vectorcraft_doc::live::GradientMesh::from_surface(4, 4, &|u, v| Point::new(x + u * 30.0, 10.0 + v * 30.0), &|_, _| Color::BLACK)
}

fn blend_of(d: &mut Document, keys: Vec<Node>, steps: u32) -> Node {
    let spec = vectorcraft_doc::BlendSpec { spacing: vectorcraft_doc::BlendSpacing::Steps(steps), ..Default::default() };
    Node::new(d.alloc_id(), NodeKind::Blend { children: keys.into_iter().map(Arc::new).collect(), spec: Box::new(spec) })
}

/// A mesh inside a blend is its outline too: a blend of two meshes with one step between them
/// draws three outlines. A mesh whose every point is at 0% opacity draws nothing.
#[test]
fn meshes_in_blends_are_outlines_and_clear_meshes_draw_nothing() {
    let d = doc_with(|d| {
        let keys = vec![Node::new(d.alloc_id(), NodeKind::Mesh(mesh_at(10.0))), Node::new(d.alloc_id(), NodeKind::Mesh(mesh_at(110.0)))];
        let mut clear = mesh_at(60.0);
        clear.points.iter_mut().for_each(|p| p.opacity = 0.0);
        vec![blend_of(d, keys, 1), Node::new(d.alloc_id(), NodeKind::Mesh(clear))]
    });
    let t = text(&plot(&d, &opts(&d)));
    assert_eq!(t.matches("PD").count(), 3, "{t}");
}

/// Live art that makes more objects than the plot's limit (1,000 in tests) is left out with a
/// warning.
#[test]
fn live_art_past_its_limit_is_left_out() {
    let d = doc_with(|d| {
        let keys = vec![stroked(d, line((10.0, 10.0), (10.0, 20.0)), Color::BLACK), stroked(d, line((190.0, 10.0), (190.0, 20.0)), Color::BLACK)];
        vec![blend_of(d, keys, 1_200)]
    });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == LIVE_LEFT_OUT), "{:?}", out.warnings);
    assert_eq!(text(&out), "IN;PU;SP0;");
}

/// A line shorter than a plotter unit draws nothing and takes no pen.
#[test]
fn a_line_that_draws_nothing_takes_no_pen() {
    let d = doc_with(|d| {
        vec![stroked(d, line((10.0, 10.0), (10.01, 10.0)), Color::rgb8(255, 0, 0)), stroked(d, rect(10.0, 10.0, 60.0, 40.0), Color::BLACK)]
    });
    let out = plot(&d, &opts(&d));
    assert_eq!(out.pens, [[0, 0, 0]]);
    assert!(text(&out).starts_with("IN;PU;SP1;PU141,1270;"), "{}", text(&out));
    let two = plot(&d, &HpglOptions { version: Version::Hpgl2, ..opts(&d) });
    assert!(two.bytes.starts_with(b"IN;NP2;PC1,0,0,0;PU;SP1;PE"), "{:?}", String::from_utf8_lossy(&two.bytes));
}

/// The plots of one export share the pens: a color is the same pen in each, numbered in the
/// order the plots first draw in it.
#[test]
fn the_plots_of_one_export_share_their_pens() {
    let d = doc_with(|d| {
        vec![
            stroked(d, line((10.0, 10.0), (20.0, 10.0)), Color::BLACK),
            stroked(d, line((110.0, 10.0), (120.0, 10.0)), Color::rgb8(255, 0, 0)),
            stroked(d, line((110.0, 20.0), (120.0, 20.0)), Color::BLACK),
        ]
    });
    let regions = [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(100.0, 0.0, 200.0, 100.0)];
    let plots = export_regions(&d, &HpglOptions::default(), &regions).unwrap();
    assert_eq!(plots.len(), 2);
    for p in &plots {
        assert_eq!(p.pens, [[0, 0, 0], [255, 0, 0]]);
    }
    assert_eq!(text(&plots[0]), "IN;PU;SP1;PU141,1270;PD282,1270;PU;SP0;");
    assert_eq!(text(&plots[1]), "IN;PU;SP1;PU141,1129;PD282,1129;PU;SP2;PU141,1270;PD282,1270;PU;SP0;");
}

#[test]
fn hidden_and_non_printing_art_is_left_out() {
    let mut d = doc_with(|d| {
        let mut hidden = stroked(d, rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
        hidden.visible = false;
        let none = Node::path(d.alloc_id(), rect(30.0, 10.0, 40.0, 20.0), Appearance::basic(Paint::None, Paint::None, 1.0));
        vec![hidden, none]
    });
    let mut layer = Node::layer(d.alloc_id(), "Notes", LayerColor::Preset(1));
    if let NodeKind::Layer { printable, .. } = &mut layer.kind {
        *printable = false;
    }
    let art = stroked(&mut d, rect(50.0, 10.0, 60.0, 20.0), Color::BLACK);
    layer.children_mut().unwrap().push(Arc::new(art));
    d.layers.push(Arc::new(layer));
    assert_eq!(text(&plot(&d, &opts(&d))), "IN;PU;SP0;");
}

/// Hidden and template layers, guides of either kind and placed images draw nothing; an image
/// comes back as a warning.
#[test]
fn layers_guides_and_images_that_draw_nothing_are_left_out() {
    let mut d = doc_with(|d| {
        let mut guide = stroked(d, rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
        if let NodeKind::Path { guide, .. } = &mut guide.kind {
            *guide = true;
        }
        let ruler = Node::new(d.alloc_id(), NodeKind::RulerGuide { vertical: true, pos: 50.0, artboard: None });
        let image = vectorcraft_doc::ImageObject {
            key: "raster-1".into(),
            width: 4,
            height: 2,
            xf: Affine::translate((10.0, 20.0)) * Affine::scale(5.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        vec![ruler, guide, Node::new(d.alloc_id(), NodeKind::Image(Box::new(image)))]
    });
    let hidden = d.add_layer(Some("Notes"));
    let template = d.add_layer(Some("Tracing"));
    for l in [hidden, template] {
        let n = stroked(&mut d, rect(30.0, 10.0, 40.0, 20.0), Color::BLACK);
        d.insert(Some(l), 0, n).unwrap();
    }
    if let Some(n) = d.node_mut(hidden) {
        n.visible = false;
    }
    if let Some(NodeKind::Layer { template, .. }) = d.node_mut(template).map(|n| &mut n.kind) {
        *template = true;
    }
    let out = plot(&d, &opts(&d));
    assert_eq!(text(&out), "IN;PU;SP0;");
    assert_eq!(out.warnings, [IMAGES]);
}

/// A non-printing layer's Crop Marks are left out with the layer, and so is a template layer's.
#[test]
fn the_crop_marks_of_a_layer_left_out_are_left_out() {
    for template in [false, true] {
        let mut d = doc_with(|d| vec![stroked(d, rect(10.0, 10.0, 60.0, 40.0), Color::BLACK)]);
        let layer = d.layers[0].id;
        if let Some(n) = d.node_mut(layer) {
            n.appearance.effects.push(vectorcraft_doc::Effect { id: "cropMarks".into(), params: serde_json::json!({}), visible: true });
            if let NodeKind::Layer { printable, template: t, .. } = &mut n.kind {
                (*printable, *t) = (template, template);
            }
        }
        assert_eq!(text(&plot(&d, &opts(&d))), "IN;PU;SP0;", "template: {template}");
    }
}

/// A guide's Crop Marks are left out with the guide, in the layers and in a symbol's art.
#[test]
fn the_crop_marks_of_a_guide_are_left_out() {
    let guide = |d: &mut Document| {
        let mut g = stroked(d, rect(10.0, 10.0, 60.0, 40.0), Color::BLACK);
        g.appearance.effects.push(vectorcraft_doc::Effect { id: "cropMarks".into(), params: serde_json::json!({}), visible: true });
        if let NodeKind::Path { guide, .. } = &mut g.kind {
            *guide = true;
        }
        g
    };
    let d = doc_with(|d| {
        let g = guide(d);
        vec![Node::group(d.alloc_id(), vec![Arc::new(g)])]
    });
    assert_eq!(text(&plot(&d, &opts(&d))), "IN;PU;SP0;");
    let mut d = Document::new(200.0, 100.0);
    let g = guide(&mut d);
    let art = Node::group(d.alloc_id(), vec![Arc::new(g)]);
    let d = symbol_doc(art, |_| {});
    assert_eq!(text(&plot(&d, &opts(&d))), "IN;PU;SP0;");
}

/// Inline graphics in type are left out with a warning.
#[test]
fn inline_graphics_in_type_are_left_out_with_a_warning() {
    let d = doc_with(|d| {
        let style = CharStyle { size: 30.0, fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
        let mut t = TextObject::point(Point::new(10.0, 60.0), "O", style.clone());
        t.runs.push(vectorcraft_doc::TextRun::inline(vectorcraft_doc::InlineArt::new("mark"), style));
        vec![Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)))]
    });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == INLINE_GRAPHICS), "{:?}", out.warnings);
}

/// A gradient's pen is the average of its stops' colors.
#[test]
fn a_gradient_takes_the_average_of_its_stops() {
    let mut g = vectorcraft_color::GradientPaint::new(Default::default());
    g.gradient.stops[0].color = Color::rgb8(255, 0, 0);
    g.gradient.stops[1].color = Color::rgb8(0, 0, 255);
    let d =
        doc_with(|d| vec![Node::path(d.alloc_id(), rect(10.0, 10.0, 60.0, 40.0), Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 1.0))]);
    assert_eq!(plot(&d, &opts(&d)).pens, [[128, 0, 128]]);
}

/// An object or group at 0% opacity shows nothing, and a fill or stroke at 0% opacity gives a
/// path no color: here only the blue fill's outline is drawn.
#[test]
fn art_at_no_opacity_is_left_out() {
    let (red, green, blue) = (Color::rgb8(255, 0, 0), Color::rgb8(0, 255, 0), Color::rgb8(0, 0, 255));
    let d = doc_with(|d| {
        let mut faded = stroked(d, rect(10.0, 10.0, 20.0, 20.0), red);
        faded.opacity = 0.0;
        let inner = stroked(d, line((30.0, 10.0), (40.0, 10.0)), green);
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(inner)]);
        group.opacity = 0.0;
        let mut clear_fill = filled(d, rect(10.0, 30.0, 20.0, 40.0), green);
        let mut fill_and_clear_stroke =
            Node::path(d.alloc_id(), rect(50.0, 10.0, 60.0, 20.0), Appearance::basic(Paint::solid(blue), Paint::solid(red), 1.0));
        for item in clear_fill.appearance.items.iter_mut().chain(fill_and_clear_stroke.appearance.items.iter_mut()) {
            match item {
                AppearanceItem::Fill(f) if f.paint == Paint::solid(green) => f.opacity = 0.0,
                AppearanceItem::Stroke(s) => s.opacity = 0.0,
                _ => {}
            }
        }
        vec![faded, group, clear_fill, fill_and_clear_stroke]
    });
    let out = plot(&d, &opts(&d));
    assert_eq!(out.pens, [[0, 0, 255]]);
    assert_eq!(text(&out).matches("PD").count(), 1, "{}", text(&out));
}

/// A pattern's pen is the color of its first visible solid fill: a hidden tile object, one at 0%
/// opacity and a fill at 0% opacity are skipped.
#[test]
fn a_pattern_takes_the_color_of_its_first_visible_fill() {
    let mut d = doc_with(|_| vec![]);
    let mut hidden = filled(&mut d, rect(0.0, 0.0, 5.0, 5.0), Color::rgb8(255, 0, 0));
    hidden.visible = false;
    let mut faded = filled(&mut d, rect(0.0, 5.0, 5.0, 10.0), Color::rgb8(0, 255, 0));
    faded.opacity = 0.0;
    let mut clear_fill = filled(&mut d, rect(5.0, 5.0, 10.0, 10.0), Color::rgb8(255, 255, 0));
    for item in &mut clear_fill.appearance.items {
        if let AppearanceItem::Fill(f) = item {
            f.opacity = 0.0;
        }
    }
    let shown = filled(&mut d, rect(5.0, 0.0, 10.0, 5.0), Color::rgb8(0, 0, 255));
    let art = [hidden, faded, clear_fill, shown].map(Arc::new).to_vec();
    d.patterns.push(vectorcraft_doc::PatternDef::new("tiles", art));
    let path = Node::path(
        d.alloc_id(),
        rect(10.0, 10.0, 50.0, 50.0),
        Appearance::basic(Paint::Pattern { pattern: "tiles".into(), xf: Default::default() }, Paint::None, 1.0),
    );
    let layer = d.layers[0].id;
    d.insert(Some(layer), usize::MAX, path).unwrap();
    assert_eq!(plot(&d, &opts(&d)).pens, [[0, 0, 255]]);
}

#[test]
fn art_is_cut_at_the_artboard_and_by_clipping_masks() {
    let d = doc_with(|d| {
        let across = stroked(d, line((-50.0, 50.0), (250.0, 50.0)), Color::BLACK);
        let clip = Node::path(d.alloc_id(), rect(20.0, 20.0, 40.0, 40.0), Appearance::default());
        let clipped = stroked(d, line((0.0, 30.0), (200.0, 30.0)), Color::BLACK);
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(clip), Arc::new(clipped)]);
        if let NodeKind::Group { clip, .. } = &mut group.kind {
            *clip = true;
        }
        vec![across, group]
    });
    let t = text(&plot(&d, &opts(&d)));
    assert_eq!(t, "IN;PU;SP1;PU0,706;PD2822,706;PU282,988;PD564,988;PU;SP0;");
}

#[test]
fn type_is_drawn_as_its_outlines() {
    let d = doc_with(|d| {
        let style = CharStyle { size: 30.0, fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
        vec![Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 60.0), "Ol", style))))]
    });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == TEXT_OUTLINES), "{:?}", out.warnings);
    // A pen-up move to each contour (the O's two and the l's one), and the two around the pen.
    let t = text(&out);
    assert!(t.matches("PU").count() >= 5, "{t}");
}

#[test]
fn an_empty_or_zero_length_plot_has_no_moves() {
    let empty = Document::new(200.0, 100.0);
    assert_eq!(text(&plot(&empty, &opts(&empty))), "IN;PU;SP0;");
    let d =
        doc_with(|d| vec![stroked(d, line((10.0, 10.0), (10.0, 10.0)), Color::BLACK), stroked(d, line((10.0, 10.0), (10.01, 10.0)), Color::BLACK)]);
    assert_eq!(text(&plot(&d, &opts(&d))), "IN;PU;SP0;");
    let bad = HpglOptions { region: Rect::new(0.0, 0.0, 0.0, 10.0), ..HpglOptions::default() };
    assert_eq!(export(&d, &bad).err().as_deref(), Some("the export region is empty"));
    let nan = HpglOptions { region: Rect::new(0.0, 0.0, f64::NAN, 10.0), ..HpglOptions::default() };
    assert!(export(&d, &nan).is_err());
}

/// A tolerance outside its range and a pen table of more than 255 pens are errors, not changed.
#[test]
fn bad_options_are_errors() {
    let d = three();
    for tolerance in [0.0, 0.009, 10.01, f64::NAN, f64::INFINITY] {
        let o = HpglOptions { tolerance, ..opts(&d) };
        assert_eq!(export(&d, &o).err().as_deref(), Some("the curve tolerance must be from 0.01 to 10 points"), "{tolerance}");
    }
    for tolerance in [0.01, 10.0] {
        assert!(export(&d, &HpglOptions { tolerance, ..opts(&d) }).is_ok(), "{tolerance}");
    }
    let o = HpglOptions { pens: vec![[0, 0, 0]; MAX_PENS + 1], ..opts(&d) };
    assert_eq!(export(&d, &o).err().as_deref(), Some("the pen table holds more than 255 pens"));
    assert!(export(&d, &HpglOptions { pens: vec![[0, 0, 0]; MAX_PENS], ..opts(&d) }).is_ok());
}

#[test]
fn art_past_the_plotter_range_is_cut_off() {
    let d = doc_with(|d| vec![stroked(d, line((0.0, 50.0), (1e9, 50.0)), Color::BLACK)]);
    let out = plot(&d, &HpglOptions { region: Rect::new(0.0, 0.0, 1e9, 100.0), ..HpglOptions::default() });
    assert!(out.warnings.iter().any(|w| w == OUT_OF_RANGE), "{:?}", out.warnings);
    assert_eq!(text(&out), format!("IN;PU;SP1;PU0,706;PD{MAX_COORD},706;PU;SP0;"));
    // Art past the range but off the region asked for would be cut off anyway: no warning.
    let d = doc_with(|d| vec![stroked(d, line((-1e9, 50.0), (10.0, 50.0)), Color::BLACK)]);
    let out = plot(&d, &HpglOptions { region: Rect::new(0.0, 0.0, 1e9, 100.0), ..HpglOptions::default() });
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(text(&out), "IN;PU;SP1;PU0,706;PD141,706;PU;SP0;");
}

/// HP-GL past 16-bit coordinates is written as it is, with a warning; HP-GL/2 gets none.
#[test]
fn hpgl_past_16_bit_coordinates_warns() {
    let region = Rect::new(-2400.0, 0.0, 2400.0, 100.0);
    // Past 32,767 from the bottom-left corner, within the range from the center.
    let d = doc_with(|d| vec![stroked(d, line((-2000.0, 50.0), (2000.0, 50.0)), Color::BLACK)]);
    let out = plot(&d, &HpglOptions { region, ..HpglOptions::default() });
    assert_eq!(out.warnings, [PAST_CLASSIC_RANGE]);
    assert_eq!(text(&out), "IN;PU;SP1;PU5644,706;PD62089,706;PU;SP0;");
    let out = plot(&d, &HpglOptions { region, origin: Origin::Center, ..HpglOptions::default() });
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(text(&out), "IN;PU;SP1;PU-28222,0;PD28222,0;PU;SP0;");
    // Past −32,768 and 32,767 from the center.
    let d = doc_with(|d| vec![stroked(d, line((-2400.0, 50.0), (2400.0, 50.0)), Color::BLACK)]);
    let out = plot(&d, &HpglOptions { region, origin: Origin::Center, ..HpglOptions::default() });
    assert_eq!(out.warnings, [PAST_CLASSIC_RANGE]);
    assert_eq!(text(&out), "IN;PU;SP1;PU-33867,0;PD33867,0;PU;SP0;");
    let out = plot(&d, &HpglOptions { region, origin: Origin::Center, version: Version::Hpgl2, ..HpglOptions::default() });
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn a_plot_stops_at_its_point_budget() {
    let d = doc_with(|d| (0..MAX_POINTS / 4).map(|i| stroked(d, rect(1.0, 1.0, 2.0 + (i % 50) as f64, 3.0), Color::BLACK)).collect());
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == TOO_MANY_POINTS), "{:?}", out.warnings);
    assert!(text(&out).matches("PU").count() <= MAX_POINTS / 5 + 3);
}

/// The points a plain plot moves through.
fn moves(t: &str) -> usize {
    t.split_terminator(';')
        .filter(|i| i.starts_with("PU") || i.starts_with("PD"))
        .map(|i| i[2..].split(',').filter(|v| !v.is_empty()).count() / 2)
        .sum()
}

#[test]
fn one_path_longer_than_the_budget_is_cut_at_it() {
    let d = doc_with(|d| {
        let mut bp = BezPath::new();
        bp.move_to((1.0, 1.0));
        for i in 0..MAX_POINTS + 10 {
            bp.line_to((if i % 2 == 0 { 2.0 } else { 1.0 }, 1.0 + (i % 50) as f64));
        }
        vec![stroked(d, PathData::from_bezpath(&bp), Color::BLACK)]
    });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == TOO_MANY_POINTS), "{:?}", out.warnings);
    let n = moves(&text(&out));
    assert!(n > 2 && n <= MAX_POINTS, "{n}");
}

/// A path whose flattening stops at the budget while its moves so far lie off the artboard: the
/// rest, over the artboard, is left out with a warning.
#[test]
fn a_path_cut_short_by_the_budget_warns() {
    let d = doc_with(|d| {
        let mut bp = BezPath::new();
        bp.move_to((-50.0, 10.0));
        for i in 0..MAX_POINTS * 4 + 10 {
            bp.line_to((if i % 2 == 0 { -40.0 } else { -50.0 }, 10.0));
        }
        bp.line_to((50.0, 50.0));
        bp.line_to((60.0, 50.0));
        vec![stroked(d, PathData::from_bezpath(&bp), Color::BLACK)]
    });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == TOO_MANY_POINTS), "{:?}", out.warnings);
}

/// A filled path cut short by the budget is not closed: the move from where it stopped back to its
/// start is not on its outline. Only its first move, along y = 50 off the artboard's left edge,
/// is drawn.
#[test]
fn a_filled_path_cut_short_stays_open() {
    let d = doc_with(|d| {
        let mut bp = BezPath::new();
        bp.move_to((10.0, 50.0));
        bp.line_to((-40.0, 50.0));
        for i in 0..MAX_POINTS * 4 + 10 {
            bp.line_to((if i % 2 == 0 { -40.0 } else { -50.0 }, 10.0));
        }
        vec![filled(d, PathData::from_bezpath(&bp), Color::BLACK)]
    });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == TOO_MANY_POINTS), "{:?}", out.warnings);
    assert_eq!(text(&out), "IN;PU;SP1;PU141,706;PD0,706;PU;SP0;");
}

/// A blend's steps that its bent spine turns onto the region plotted are drawn, although its keys
/// and spine lie off it.
#[test]
fn blend_steps_a_spine_turns_onto_the_region_are_drawn() {
    let d = doc_with(|d| {
        let keys =
            vec![stroked(d, line((-400.0, -200.0), (200.0, -200.0)), Color::BLACK), stroked(d, line((-560.0, -60.0), (40.0, -60.0)), Color::BLACK)];
        let mut spine = BezPath::new();
        spine.move_to((-100.0, -200.0));
        spine.quad_to((-100.0, -60.0), (-260.0, -60.0));
        let spec = vectorcraft_doc::BlendSpec {
            spacing: vectorcraft_doc::BlendSpacing::Steps(20),
            spine: Some(PathData::from_bezpath(&spine)),
            orientation: vectorcraft_doc::BlendOrientation::AlignToPath,
            ..Default::default()
        };
        vec![Node::new(d.alloc_id(), NodeKind::Blend { children: keys.into_iter().map(Arc::new).collect(), spec: Box::new(spec) })]
    });
    // The keys and spine lie above y = -55; the steps turned along the spine cross this area.
    let region = Rect::new(-300.0, 0.0, -200.0, 50.0);
    let t = text(&plot(&d, &HpglOptions { region, ..HpglOptions::default() }));
    assert!(t.contains("PD"), "{t}");
}

/// A closed path that starts on the artboard, leaves it and comes back is one line, with no
/// pen lift where it started.
#[test]
fn a_closed_path_cut_at_the_artboard_is_one_line() {
    let d = doc_with(|d| {
        let mut bp = BezPath::new();
        bp.move_to((150.0, 10.0));
        bp.line_to((250.0, 10.0));
        bp.line_to((250.0, 50.0));
        bp.line_to((150.0, 50.0));
        bp.close_path();
        vec![stroked(d, PathData::from_bezpath(&bp), Color::BLACK)]
    });
    let t = text(&plot(&d, &opts(&d)));
    assert_eq!(t, "IN;PU;SP1;PU2822,706;PD2117,706,2117,1270,2822,1270;PU;SP0;");
}

/// A clipping path that paints is drawn once: where its fill paints (before the art it clips),
/// or where its stroke paints (after it).
#[test]
fn a_clipping_path_is_drawn_where_it_paints() {
    let clip_doc = |fill: Paint, stroke: Paint| {
        doc_with(|d| {
            let clip = Node::path(d.alloc_id(), rect(20.0, 20.0, 40.0, 40.0), Appearance::basic(fill, stroke, 1.0));
            let clipped = stroked(d, line((0.0, 30.0), (200.0, 30.0)), Color::BLACK);
            let mut group = Node::group(d.alloc_id(), vec![Arc::new(clip), Arc::new(clipped)]);
            if let NodeKind::Group { clip, .. } = &mut group.kind {
                *clip = true;
            }
            vec![group]
        })
    };
    let red = Paint::solid(Color::rgb8(255, 0, 0));
    let filled = clip_doc(red.clone(), Paint::None);
    assert_eq!(plot(&filled, &opts(&filled)).pens, [[255, 0, 0], [0, 0, 0]]);
    let stroked = clip_doc(Paint::None, red.clone());
    assert_eq!(plot(&stroked, &opts(&stroked)).pens, [[0, 0, 0], [255, 0, 0]]);
    let both = clip_doc(Paint::solid(Color::rgb8(0, 0, 255)), red);
    let out = plot(&both, &opts(&both));
    assert_eq!(out.pens, [[0, 0, 0], [255, 0, 0]], "drawn once, in its stroke's color");
    // Two pen changes, a move to each of the two paths, and the last pen up.
    assert_eq!(text(&out).matches("PU").count(), 5, "{}", text(&out));
    // A stroke at 0% opacity isn't drawn: the clipping path is drawn in its fill, below.
    let faded = doc_with(|d| {
        let mut clip = Node::path(
            d.alloc_id(),
            rect(20.0, 20.0, 40.0, 40.0),
            Appearance::basic(Paint::solid(Color::rgb8(0, 0, 255)), Paint::solid(Color::rgb8(255, 0, 0)), 1.0),
        );
        for item in &mut clip.appearance.items {
            if let AppearanceItem::Stroke(s) = item {
                s.opacity = 0.0;
            }
        }
        let clipped = Node::path(d.alloc_id(), line((0.0, 30.0), (200.0, 30.0)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0));
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(clip), Arc::new(clipped)]);
        if let NodeKind::Group { clip, .. } = &mut group.kind {
            *clip = true;
        }
        vec![group]
    });
    assert_eq!(plot(&faded, &opts(&faded)).pens, [[0, 0, 255], [0, 0, 0]]);
    // An open clipping path with a fill and a stroke is drawn closed, as other filled paths are.
    let mut open = BezPath::new();
    open.move_to((20.0, 20.0));
    open.line_to((40.0, 20.0));
    open.line_to((40.0, 40.0));
    let open = PathData::from_bezpath(&open);
    let open_doc = doc_with(|d| {
        let clip = Node::path(d.alloc_id(), open, Appearance::basic(Paint::solid(Color::rgb8(0, 0, 255)), Paint::solid(Color::rgb8(255, 0, 0)), 1.0));
        let clipped = Node::path(d.alloc_id(), line((0.0, 30.0), (200.0, 30.0)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0));
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(clip), Arc::new(clipped)]);
        if let NodeKind::Group { clip, .. } = &mut group.kind {
            *clip = true;
        }
        vec![group]
    });
    let t = text(&plot(&open_doc, &opts(&open_doc)));
    assert!(t.ends_with("SP2;PU282,1129;PD564,1129,564,847,282,1129;PU;SP0;"), "{t}");
}

/// A symbol `art` and an instance of it at its place.
fn symbol_doc(art: Node, instance: impl FnOnce(&mut Node)) -> Document {
    let mut d = doc_with(|d| {
        let mut n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::IDENTITY });
        instance(&mut n);
        vec![n]
    });
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "s".into(), art: Arc::new(art) });
    d
}

/// Symbols nested more than 8 deep are left out with a warning.
#[test]
fn symbols_nested_too_deep_are_left_out_with_a_warning() {
    let mut d = doc_with(|d| vec![Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "s9".into(), xf: Affine::IDENTITY })]);
    let art = stroked(&mut d, rect(10.0, 10.0, 60.0, 40.0), Color::BLACK);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "s0".into(), art: Arc::new(art) });
    for i in 1..10 {
        let inner = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: format!("s{}", i - 1), xf: Affine::IDENTITY });
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: format!("s{i}"), art: Arc::new(inner) });
    }
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == TOO_DEEP), "{:?}", out.warnings);
    assert_eq!(text(&out), "IN;PU;SP0;");
}

/// A symbol instance stained by its fill draws in the stained color.
#[test]
fn a_stained_symbol_instance_takes_its_stain() {
    let mut d = Document::new(200.0, 100.0);
    let art = stroked(&mut d, rect(10.0, 10.0, 60.0, 40.0), Color::BLACK);
    let d = symbol_doc(art, |n| n.appearance = Appearance::basic(Paint::solid(Color::rgb8(255, 0, 0)), Paint::None, 1.0).into());
    assert_eq!(plot(&d, &opts(&d)).pens, [[255, 0, 0]]);
}

/// The geometry effects in a symbol's art apply: a line moved 30 pt right by a Transform effect.
#[test]
fn a_symbol_draws_its_art_with_its_effects() {
    let mut d = Document::new(200.0, 100.0);
    let mut art = stroked(&mut d, line((10.0, 10.0), (20.0, 10.0)), Color::BLACK);
    art.appearance.effects.push(vectorcraft_doc::Effect {
        id: "distort.transform".into(),
        params: serde_json::json!({"moveH": 30.0}),
        visible: true,
    });
    let d = symbol_doc(art, |_| {});
    assert_eq!(text(&plot(&d, &opts(&d))), "IN;PU;SP1;PU564,1270;PD706,1270;PU;SP0;");
}

/// Art an effect moves onto the artboard from off it is drawn, for an instance on its own and
/// one in a group: the bounds that skip art off the artboard don't measure symbol effects.
#[test]
fn symbol_art_an_effect_moves_onto_the_artboard_is_drawn() {
    let mut d = Document::new(200.0, 100.0);
    let mut art = stroked(&mut d, line((-30.0, 10.0), (-20.0, 10.0)), Color::BLACK);
    art.appearance.effects.push(vectorcraft_doc::Effect {
        id: "distort.transform".into(),
        params: serde_json::json!({"moveH": 40.0}),
        visible: true,
    });
    let alone = symbol_doc(art.clone(), |_| {});
    assert_eq!(text(&plot(&alone, &opts(&alone))), "IN;PU;SP1;PU141,1270;PD282,1270;PU;SP0;");
    let mut grouped = doc_with(|d| {
        let n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::IDENTITY });
        vec![Node::group(d.alloc_id(), vec![Arc::new(n)])]
    });
    grouped.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "s".into(), art: Arc::new(art.clone()) });
    assert_eq!(text(&plot(&grouped, &opts(&grouped))), "IN;PU;SP1;PU141,1270;PD282,1270;PU;SP0;");
    // Other art off the artboard is still skipped: a blend there doesn't spend the live art's
    // limit (1,000 objects in tests) before the blend on the artboard.
    let mut d = doc_with(|d| {
        let off =
            vec![stroked(d, line((-500.0, 10.0), (-500.0, 20.0)), Color::BLACK), stroked(d, line((-400.0, 10.0), (-400.0, 20.0)), Color::BLACK)];
        let on = vec![stroked(d, line((10.0, 10.0), (10.0, 20.0)), Color::BLACK), stroked(d, line((30.0, 10.0), (30.0, 20.0)), Color::BLACK)];
        vec![blend_of(d, off, 1_200), blend_of(d, on, 1)]
    });
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "unused".into(), art: Arc::new(art) });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(text(&out).matches("PD").count(), 3, "{}", text(&out));
}

/// A rectangle with `per_side` straight moves along each side.
fn fine_rect(x0: f64, y0: f64, x1: f64, y1: f64, per_side: u32) -> PathData {
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut bp = BezPath::new();
    bp.move_to(corners[0]);
    for (k, a) in corners.iter().enumerate() {
        let b = corners[(k + 1) % 4];
        for i in 1..=per_side {
            let t = f64::from(i) / f64::from(per_side);
            bp.line_to((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    bp.close_path();
    PathData::from_bezpath(&bp)
}

/// Nested clipping masks of 6,000 points each pass the 10,000 points masks may hold together
/// in tests: the inner one is left out with a warning, and the line is cut by the outer one only.
#[test]
fn masks_past_their_point_budget_are_left_out() {
    let d = doc_with(|d| {
        let clip_group = |d: &mut Document, mask: PathData, art: Node| {
            let mask = Node::path(d.alloc_id(), mask, Appearance::basic(Paint::None, Paint::None, 1.0));
            let mut group = Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(art)]);
            if let NodeKind::Group { clip, .. } = &mut group.kind {
                *clip = true;
            }
            group
        };
        let across = Node::path(d.alloc_id(), line((0.0, 50.0), (200.0, 50.0)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0));
        let inner = clip_group(d, fine_rect(50.0, 30.0, 150.0, 70.0, 1500), across);
        vec![clip_group(d, fine_rect(10.0, 10.0, 190.0, 90.0, 1500), inner)]
    });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == MASK_POINTS), "{:?}", out.warnings);
    assert_eq!(text(&out), "IN;PU;SP1;PU141,706;PD2681,706;PU;SP0;");
}

/// A line through the 500 teeth of a comb-shaped mask crosses its edges about 1,000 times, and
/// each piece between crossings is tested against all 2,002 edges: that spends the clipping
/// budget, so the line is drawn whole with a warning.
#[test]
fn a_mask_crossed_many_times_spends_the_clipping_budget() {
    let d = doc_with(|d| {
        let mut pts = vec![];
        for i in 0..500 {
            let x = f64::from(i) * 0.4;
            pts.extend([(x, 20.0), (x + 0.2, 20.0), (x + 0.2, 50.0), (x + 0.4, 50.0)]);
        }
        pts.extend([(200.0, 80.0), (0.0, 80.0)]);
        let mut bp = BezPath::new();
        bp.move_to(pts[0]);
        pts[1..].iter().for_each(|&p| bp.line_to(p));
        bp.close_path();
        let comb = Node::path(d.alloc_id(), PathData::from_bezpath(&bp), Appearance::basic(Paint::None, Paint::None, 1.0));
        let clipped = stroked(d, line((0.5, 35.0), (199.5, 35.0)), Color::BLACK);
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(comb), Arc::new(clipped)]);
        if let NodeKind::Group { clip, .. } = &mut group.kind {
            *clip = true;
        }
        vec![group]
    });
    let out = plot(&d, &opts(&d));
    assert!(out.warnings.iter().any(|w| w == CLIP_WORK), "{:?}", out.warnings);
    assert_eq!(text(&out).matches("PD").count(), 1, "{}", text(&out));
}

#[test]
fn more_colors_than_pens_share_the_nearest_pen() {
    let d = doc_with(|d| (0..300).map(|i| stroked(d, rect(1.0, 1.0, 5.0, 5.0), Color::rgb8((i % 256) as u8, (i / 256) as u8, 7))).collect());
    let out = plot(&d, &opts(&d));
    assert_eq!(out.pens.len(), MAX_PENS);
    assert!(out.warnings.iter().any(|w| w == MANY_COLORS), "{:?}", out.warnings);
}

#[test]
fn sorting_draws_the_nearest_path_next() {
    let d = doc_with(|d| {
        vec![
            stroked(d, line((150.0, 90.0), (150.0, 80.0)), Color::BLACK),
            stroked(d, line((10.0, 80.0), (10.0, 90.0)), Color::BLACK),
            stroked(d, line((80.0, 90.0), (80.0, 80.0)), Color::BLACK),
        ]
    });
    let unsorted = text(&plot(&d, &opts(&d)));
    assert_eq!(unsorted, "IN;PU;SP1;PU2117,141;PD2117,282;PU141,282;PD141,141;PU1129,141;PD1129,282;PU;SP0;");
    // From the origin: the left line drawn backwards from its lower end, then the middle and the
    // right one from the end the pen is nearest.
    let sorted = text(&plot(&d, &HpglOptions { sort: true, ..opts(&d) }));
    assert_eq!(sorted, "IN;PU;SP1;PU141,141;PD141,282;PU1129,282;PD1129,141;PU2117,141;PD2117,282;PU;SP0;");
}

/// A closed path is as near as its nearest point, and starts there.
#[test]
fn sorting_measures_a_closed_path_by_its_nearest_point() {
    let d = doc_with(|d| vec![stroked(d, rect(40.0, 60.0, 50.0, 70.0), Color::BLACK), stroked(d, rect(10.0, 10.0, 190.0, 90.0), Color::BLACK)]);
    let t = text(&plot(&d, &HpglOptions { sort: true, ..opts(&d) }));
    // The large square's bottom-left corner is nearer the origin than any point of the small one.
    assert_eq!(t, "IN;PU;SP1;PU141,141;PD141,1270,2681,1270,2681,141,141,141;PU564,423;PD564,564,706,564,706,423,564,423;PU;SP0;");
}

#[test]
fn sorting_stops_at_its_budget_and_keeps_every_path() {
    let d = doc_with(|d| (0..600).map(|i| stroked(d, line((1.0 + (i % 100) as f64, 10.0), (1.0 + (i % 100) as f64, 20.0)), Color::BLACK)).collect());
    let out = plot(&d, &HpglOptions { sort: true, ..opts(&d) });
    assert_eq!(out.setting_notes, [UNSORTED]);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(text(&out).matches("PD").count(), 600);
}

/// `x` points in plotter units.
fn plu(x: f64) -> i64 {
    (x * 1016.0 / 72.0).round() as i64
}

/// The left edge of each path a plain HP-GL plot draws, in drawing order (plotter units).
fn lefts(out: &HpglOutput) -> Vec<i64> {
    let mut lefts: Vec<i64> = vec![];
    for i in text(out).split_terminator(';') {
        let xs = i.get(2..).unwrap_or_default().split(',').step_by(2).filter_map(|x| x.parse::<i64>().ok());
        match i.get(..2) {
            Some("PU") => lefts.extend(xs.min()),
            Some("PD") => {
                if let (Some(l), Some(x)) = (lefts.last_mut(), xs.min()) {
                    *l = (*l).min(x);
                }
            }
            _ => {}
        }
    }
    lefts
}

/// A letter O made as one path: its outline, then its counter.
#[test]
fn inside_first_cuts_a_letters_counter_before_its_outline() {
    let d = doc_with(|d| {
        let mut bp = rect(10.0, 10.0, 60.0, 90.0).to_bezpath();
        bp.extend(shapes::ellipse(Rect::new(25.0, 30.0, 45.0, 70.0)).to_bezpath());
        vec![filled(d, PathData::from_bezpath(&bp), Color::BLACK)]
    });
    assert_eq!(lefts(&plot(&d, &opts(&d))), [plu(10.0), plu(25.0)]);
    assert_eq!(lefts(&plot(&d, &HpglOptions { inside_first: true, ..opts(&d) })), [plu(25.0), plu(10.0)]);
}

/// A part drawn before its two holes: the holes are cut first, nearest first with `sort`.
#[test]
fn a_part_with_two_holes_is_cut_holes_first() {
    let d = doc_with(|d| {
        vec![
            stroked(d, rect(10.0, 10.0, 190.0, 90.0), Color::BLACK),
            stroked(d, shapes::ellipse(Rect::new(160.0, 40.0, 180.0, 60.0)), Color::BLACK),
            stroked(d, shapes::ellipse(Rect::new(20.0, 40.0, 40.0, 60.0)), Color::BLACK),
        ]
    });
    let (part, right, left) = (plu(10.0), plu(160.0), plu(20.0));
    assert_eq!(lefts(&plot(&d, &opts(&d))), [part, right, left]);
    assert_eq!(lefts(&plot(&d, &HpglOptions { inside_first: true, ..opts(&d) })), [right, left, part]);
    // Sorting alone starts at the part's corner, nearest the origin; inside first, the nearest
    // hole comes first, then the other, then the part.
    assert_eq!(lefts(&plot(&d, &HpglOptions { sort: true, ..opts(&d) })), [part, left, right]);
    let both = plot(&d, &HpglOptions { sort: true, inside_first: true, ..opts(&d) });
    assert_eq!(lefts(&both), [left, right, part]);
    assert!(both.warnings.is_empty(), "{:?}", both.warnings);
}

/// Rings inside rings are cut innermost first, and a ring with nothing inside it keeps its place
/// in paint order.
#[test]
fn nested_rings_are_cut_innermost_first() {
    let d = doc_with(|d| {
        vec![
            stroked(d, rect(120.0, 10.0, 190.0, 90.0), Color::BLACK),
            stroked(d, rect(10.0, 10.0, 100.0, 90.0), Color::BLACK),
            stroked(d, rect(20.0, 20.0, 90.0, 80.0), Color::BLACK),
            stroked(d, rect(30.0, 30.0, 80.0, 70.0), Color::BLACK),
        ]
    });
    let out = plot(&d, &HpglOptions { inside_first: true, ..opts(&d) });
    assert_eq!(lefts(&out), [plu(120.0), plu(30.0), plu(20.0), plu(10.0)]);
}

/// Each pen's paths are ordered among themselves: a red square inside a black part is drawn with
/// the red pen, after the black part.
#[test]
fn inside_first_orders_each_pens_paths() {
    let d = doc_with(|d| {
        vec![
            stroked(d, rect(10.0, 10.0, 190.0, 90.0), Color::BLACK),
            stroked(d, rect(20.0, 20.0, 40.0, 40.0), Color::rgb8(255, 0, 0)),
            stroked(d, rect(50.0, 20.0, 70.0, 40.0), Color::BLACK),
        ]
    });
    let out = plot(&d, &HpglOptions { inside_first: true, ..opts(&d) });
    assert_eq!(lefts(&out), [plu(50.0), plu(10.0), plu(20.0)]);
}

/// Finding which of 100 nested rings lie inside which is past the budget: they are drawn in
/// paint order, with a note on the option.
#[test]
fn inside_first_past_its_budget_draws_in_paint_order() {
    let d = doc_with(|d| (0..100).map(|i| stroked(d, rect(1.0 + 0.4 * f64::from(i), 10.0, 199.0, 90.0), Color::BLACK)).collect());
    let out = plot(&d, &HpglOptions { inside_first: true, ..opts(&d) });
    assert_eq!(out.setting_notes, [UNNESTED]);
    let l = lefts(&out);
    assert_eq!(l.len(), 100);
    assert!(l.windows(2).all(|w| w[0] <= w[1]), "{l:?}");
    // With `sort` too, the budget is spent before sorting starts: paint order, with both notes.
    let out = plot(&d, &HpglOptions { sort: true, inside_first: true, ..opts(&d) });
    assert_eq!(out.setting_notes, [UNNESTED, UNSORTED]);
    assert_eq!(lefts(&out), l);
}

/// Sorting 600 lines inside two rings runs out of work: every path is drawn, the inner ring and
/// then the outer one last.
#[test]
fn inside_first_past_its_sorting_budget_keeps_the_rings_last() {
    let d = doc_with(|d| {
        let mut v = vec![stroked(d, rect(0.5, 5.0, 150.0, 30.0), Color::BLACK), stroked(d, rect(0.7, 7.0, 140.0, 25.0), Color::BLACK)];
        v.extend((0..600).map(|i| stroked(d, line((1.0 + (i % 100) as f64, 10.0), (1.0 + (i % 100) as f64, 20.0)), Color::BLACK)));
        v
    });
    let out = plot(&d, &HpglOptions { sort: true, inside_first: true, ..opts(&d) });
    assert_eq!(out.setting_notes, [UNSORTED_INSIDE]);
    let l = lefts(&out);
    assert_eq!(l.len(), 602);
    assert_eq!(l.get(600..), Some(&[plu(0.7), plu(0.5)][..]));
}

/// Without `sort`, 1,000 lines inside a ring are drawn in paint order and the ring after them,
/// within the budget.
#[test]
fn inside_first_alone_keeps_paint_order_for_many_paths() {
    let d = doc_with(|d| {
        let mut v = vec![stroked(d, rect(0.5, 5.0, 199.0, 30.0), Color::BLACK)];
        v.extend((0..1000).map(|i| stroked(d, line((1.0 + 0.19 * f64::from(i), 10.0), (1.0 + 0.19 * f64::from(i), 20.0)), Color::BLACK)));
        v
    });
    let out = plot(&d, &HpglOptions { inside_first: true, ..opts(&d) });
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let l = lefts(&out);
    assert_eq!(l.len(), 1001);
    assert_eq!(l.last(), Some(&plu(0.5)));
    assert!(l.get(..1000).is_some_and(|l| l.windows(2).all(|w| w[0] < w[1])), "{l:?}");
}

#[test]
fn art_wholly_past_the_plotter_range_is_cut_off_with_a_warning() {
    let d = doc_with(|d| vec![stroked(d, line((7e8, 50.0), (7e8 + 10.0, 50.0)), Color::BLACK)]);
    let out = plot(&d, &HpglOptions { region: Rect::new(0.0, 0.0, 1e9, 100.0), ..HpglOptions::default() });
    assert!(out.warnings.iter().any(|w| w == OUT_OF_RANGE), "{:?}", out.warnings);
    assert_eq!(text(&out), "IN;PU;SP0;");
}

/// The reference guide's example of an encoded number: 10,525 is written as the bytes 121, 71
/// and 196 (https://support.hp.com/sg-en/document/c01121263).
#[test]
fn encoded_numbers_follow_the_reference_guide() {
    let mut out = vec![];
    pe_number(&mut out, 10_525);
    assert_eq!(out, [121, 71, 196]);
    out.clear();
    pe_number(&mut out, -1);
    pe_number(&mut out, 0);
    // The sign in the low bit: -1 is 3.
    assert_eq!(out, [194, 191]);
}

#[test]
fn hpgl2_writes_the_palette_and_encoded_polylines() {
    let out = plot(&three(), &HpglOptions { version: Version::Hpgl2, ..opts(&three()) });
    let b = &out.bytes;
    assert!(b.starts_with(b"IN;NP4;PC1,0,0,0;PC2,255,0,0;PC3,0,0,255;PU;SP1;PE<="), "{:?}", String::from_utf8_lossy(b));
    assert!(b.ends_with(b";PU;SP0;"));
    assert_eq!(b.windows(4).filter(|w| w == b";PE<").count(), 3, "one encoded polyline per pen");
    // Encoded moves take fewer bytes than the same moves written as text.
    let plain = plot(&three(), &opts(&three()));
    assert!(b.len() < plain.bytes.len(), "{} against {}", b.len(), plain.bytes.len());
}

/// A move longer than 8,388,607 units is written absolute (`=`), so every number stays in the
/// range: here from about −5,000,000 to +5,000,000 units around a centered origin.
#[test]
fn hpgl2_writes_a_move_past_the_range_absolute() {
    let d = doc_with(|d| {
        vec![
            stroked(d, line((-354_000.0, 50.0), (-353_990.0, 50.0)), Color::BLACK),
            stroked(d, line((353_990.0, 50.0), (354_000.0, 50.0)), Color::BLACK),
        ]
    });
    let o = HpglOptions {
        version: Version::Hpgl2,
        origin: Origin::Center,
        region: Rect::new(-400_000.0, 0.0, 400_000.0, 100.0),
        ..HpglOptions::default()
    };
    let out = plot(&d, &o);
    assert_eq!(out.bytes.iter().filter(|b| **b == b'=').count(), 2, "{:?}", String::from_utf8_lossy(&out.bytes));
    let back = super::super::import(&out.bytes).unwrap();
    let mut xs = vec![];
    back.document.walk(|n| {
        if let Some(p) = n.path_data() {
            xs.extend(p.to_bezpath().elements().iter().filter_map(|e| if let PathEl::MoveTo(p) = e { Some(p.x) } else { None }));
        }
    });
    assert_eq!(xs.len(), 2);
    assert!((xs[1] - xs[0] - 707_990.0).abs() < 0.2, "{xs:?}");
}

/// The points of every path `doc` holds, in paint order.
fn polylines(doc: &Document) -> Vec<(Vec<Point>, Option<Paint>)> {
    let mut out = vec![];
    doc.walk(|n| {
        if let NodeKind::Path { path, .. } = &n.kind {
            let bp = path.to_bezpath();
            let pts = bp.elements().iter().filter_map(|e| match e {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
                _ => None,
            });
            let stroke = n.appearance.items.iter().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.paint.clone()) } else { None });
            out.push((pts.collect(), stroke));
        }
    });
    out
}

/// Each version's plot read back: the same lines, within the tolerance and half a plotter unit,
/// and with HP-GL/2 the same colors.
#[test]
fn plots_read_back_as_the_same_lines() {
    let d = three();
    for version in Version::ALL {
        let out = plot(&d, &HpglOptions { version, ..opts(&d) });
        let back = super::super::import(&out.bytes).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        let lines = polylines(&back.document);
        assert_eq!(lines.len(), 3, "{version:?}");
        // The reader's artboard is round what the art paints: line the two up by the square.
        let shift = lines[0].0[0] - Point::new(10.0, 10.0);
        // Two roundings to whole plotter units (1/1016 in) apart at most.
        let near = |p: Point, q: Point| (p - shift).distance(q) <= 0.11;
        let square = [(10.0, 10.0), (60.0, 10.0), (60.0, 40.0), (10.0, 40.0), (10.0, 10.0)].map(Point::from);
        assert!(lines[0].0.iter().zip(square).all(|(p, q)| near(*p, q)) && lines[0].0.len() == 5, "{:?}", lines[0].0);
        assert!(near(lines[1].0[0], Point::new(70.0, 20.0)) && near(lines[1].0[1], Point::new(150.0, 80.0)), "{:?}", lines[1].0);
        // The ellipse's every point lies on it, within the tolerance and the rounding.
        let ellipse = kurbo::Ellipse::new((100.0, 50.0), (30.0, 20.0), 0.0);
        for p in &lines[2].0 {
            let q = p.to_vec2() - shift - kurbo::Vec2::new(100.0, 50.0);
            let r = (q.x / 30.0).hypot(q.y / 20.0);
            assert!((r - 1.0).abs() * 20.0 <= DEFAULT_TOLERANCE + 0.1, "{p:?} off the ellipse");
        }
        assert!(lines[2].0.len() > 16 && ellipse.area() > 0.0);
        if version == Version::Hpgl2 {
            let colors: Vec<Option<Paint>> = lines.iter().map(|l| l.1.clone()).collect();
            assert_eq!(
                colors,
                [Paint::solid(Color::rgb8(0, 0, 0)), Paint::solid(Color::rgb8(255, 0, 0)), Paint::solid(Color::rgb8(0, 0, 255))].map(Some)
            );
        }
    }
}
