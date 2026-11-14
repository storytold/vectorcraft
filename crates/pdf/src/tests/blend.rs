//! Blend modes and transparency groups in PDF export.

use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

fn rect(r: Rect, rgb: (f32, f32, f32)) -> Node {
    Node::path(NodeId(0), shapes::rectangle(r), Appearance::basic(Paint::solid(Color::rgb(rgb.0, rgb.1, rgb.2)), Paint::None, 0.0))
}

fn add(d: &mut Document, mut n: Node) {
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

/// The file's text with spaces removed (content streams uncompressed).
fn text(d: &Document) -> String {
    let bytes = export(d, &PdfOptions::uncompressed()).unwrap();
    String::from_utf8_lossy(&bytes).replace(' ', "")
}

#[test]
fn every_blend_mode_is_written_under_its_pdf_name_and_read_back() {
    for mode in BlendMode::ALL.into_iter().filter(|m| *m != BlendMode::Normal) {
        let mut d = Document::new(100.0, 100.0);
        add(&mut d, rect(Rect::new(0.0, 0.0, 100.0, 100.0), (0.2, 0.5, 0.9)));
        let mut top = rect(Rect::new(20.0, 20.0, 80.0, 80.0), (0.6, 0.25, 0.5));
        top.blend = mode;
        add(&mut d, top);
        let name = mode.label().replace(' ', "");
        assert!(text(&d).contains(&format!("/BM/{name}")), "{mode:?}");
        let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
        let mut found = BlendMode::Normal;
        back.walk(|n| {
            if n.blend != BlendMode::Normal {
                found = n.blend;
            }
        });
        assert_eq!(found, mode);
    }
}

/// The backdrop and a 50% group (isolated or not) holding a Multiply square.
fn half_group(isolate: bool) -> Document {
    let mut d = Document::new(100.0, 100.0);
    add(&mut d, rect(Rect::new(0.0, 0.0, 100.0, 100.0), (0.8, 0.6, 0.2)));
    let mut m = rect(Rect::new(20.0, 20.0, 80.0, 80.0), (0.5, 0.5, 1.0));
    (m.id, m.blend) = (d.alloc_id(), BlendMode::Multiply);
    let mut g = Node::group(NodeId(0), vec![std::sync::Arc::new(m)]);
    (g.opacity, g.isolate) = (0.5, isolate);
    add(&mut d, g);
    d
}

#[test]
fn a_group_is_written_non_isolated_unless_it_isolates_blending() {
    let open = text(&half_group(false));
    // A transparency group without /I, its 50% carried by a constant alpha soft mask.
    assert!(!open.contains("/Itrue"), "{open}");
    assert!(open.contains("/SMask") && open.contains("/Alpha") && open.contains("/BM/Multiply"), "{open}");
    let isolated = text(&half_group(true));
    assert!(isolated.contains("/Itrue") && isolated.contains("/ca0.5"), "{isolated}");
    assert!(!isolated.contains("/Alpha"), "{isolated}");
    // Either way the child's blend mode reads back.
    for d in [half_group(false), half_group(true)] {
        let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
        let mut bm = BlendMode::Normal;
        back.walk(|n| {
            if n.blend != BlendMode::Normal {
                bm = n.blend;
            }
        });
        assert_eq!(bm, BlendMode::Multiply);
    }
}

#[test]
fn groups_without_blending_inside_stay_isolated_groups() {
    let mut d = half_group(false);
    let g = d.layers[0].children().unwrap()[1].id;
    let m = d.node(g).unwrap().children().unwrap()[0].id;
    d.node_mut(m).unwrap().blend = BlendMode::Normal;
    let t = text(&d);
    assert!(t.contains("/Itrue") && t.contains("/ca0.5") && !t.contains("/SMask"), "{t}");
}

#[test]
fn a_knockout_group_with_blending_elements_is_non_isolated_unless_isolated() {
    for isolate in [false, true] {
        let mut d = half_group(isolate);
        let g = d.layers[0].children().unwrap()[1].id;
        let n = d.node_mut(g).unwrap();
        (n.opacity, n.knockout) = (1.0, vectorcraft_doc::Knockout::On);
        assert_eq!(text(&d).contains("/Itrue"), isolate, "isolate {isolate}");
    }
}

/// A knockout group of `n` red, green and blue squares at 50% opacity that all overlap, over an
/// opaque backdrop. `knockout_shape`: their opacity defines their knockout shape.
fn knockout_squares(n: usize, knockout_shape: bool, isolate: bool) -> Document {
    let mut d = Document::new(200.0, 200.0);
    add(&mut d, rect(Rect::new(0.0, 0.0, 200.0, 200.0), (0.9, 0.8, 0.2)));
    let colors = [(1.0, 0.0, 0.0), (0.0, 0.8, 0.0), (0.0, 0.0, 1.0)];
    let squares = (0..n)
        .map(|i| {
            let x = (i % 20) as f64 * 2.0;
            let mut s = rect(Rect::new(20.0 + x, 50.0, 120.0 + x, 150.0), colors[i % 3]);
            (s.id, s.opacity, s.knockout_shape) = (d.alloc_id(), 0.5, knockout_shape);
            std::sync::Arc::new(s)
        })
        .collect();
    let mut g = Node::group(NodeId(0), squares);
    (g.knockout, g.isolate) = (vectorcraft_doc::Knockout::On, isolate);
    add(&mut d, g);
    d
}

/// How many objects each knockout group of `d` holds.
fn knockout_groups(d: &Document) -> Vec<usize> {
    let mut out = vec![];
    d.walk(|n| {
        if n.knockout == vectorcraft_doc::Knockout::On {
            out.push(n.children().map_or(0, |c| c.len()));
        }
    });
    out
}

/// A knockout group whose elements knock out the area they paint is written as one PDF knockout
/// group (`/K true`), whatever the number of elements, and reads back as a knockout group that
/// draws as the original does.
#[test]
fn a_knockout_group_is_written_as_one_pdf_knockout_group() {
    for n in [0, 1, 2, 49, 50, 200] {
        let d = knockout_squares(n, false, false);
        let t = text(&d);
        assert_eq!(t.matches("/Ktrue").count(), usize::from(n > 1), "{n}");
        let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap_or_else(|e| panic!("{n}: {e}"));
        assert_eq!(knockout_groups(&back), if n > 1 { vec![n] } else { vec![] }, "{n}");
        if n == 200 {
            vectorcraft_testkit::raster::assert_similar(
                &vectorcraft_testkit::raster::render_artboard(&back),
                &vectorcraft_testkit::raster::render_artboard(&d),
                4.0,
                0.01,
            );
        }
    }
}

/// The knockout group of an isolated group is isolated too (`/I true` beside `/K true`), and the
/// group reads back as isolated and knocking out. (Each translucent element is a group of its own.)
#[test]
fn an_isolated_knockout_group_is_one_isolated_pdf_knockout_group() {
    for isolate in [false, true] {
        let mut d = knockout_squares(3, false, isolate);
        // Blending inside, which isolation keeps from the backdrop.
        let g = d.layers[0].children().unwrap()[1].id;
        let top = d.node(g).unwrap().children().unwrap()[2].id;
        d.node_mut(top).unwrap().blend = BlendMode::Multiply;
        let t = text(&d);
        assert_eq!(t.matches("/Ktrue").count(), 1, "isolate {isolate}");
        assert_eq!(t.contains("/Itrue/Ktrue"), isolate, "isolate {isolate}");
        let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
        let mut found = vec![];
        back.walk(|n| {
            if n.knockout == vectorcraft_doc::Knockout::On {
                found.push(n.isolate);
            }
        });
        assert_eq!(found, [isolate]);
    }
}

/// Elements whose opacity defines their knockout shape knock out less than a PDF knockout group
/// knocks out: each is drawn through one soft mask of the elements above it, so that 200 of them
/// still read back, every one of them.
#[test]
fn elements_whose_opacity_defines_their_knockout_shape_are_drawn_through_one_mask_each() {
    let d = knockout_squares(200, true, false);
    let r = export_with_report(&d, &PdfOptions::default()).unwrap();
    assert!(!r.warnings.iter().any(|w| w.contains("knockout")), "{:?}", r.warnings);
    assert!(!text(&d).contains("/Ktrue"));
    let back = import(&r.bytes).unwrap();
    let mut paths = 0;
    back.walk(|n| paths += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. })));
    // The backdrop and the 200 squares (the masks' copies of them are opacity-mask art).
    assert_eq!(paths, 201);
}

/// When the masks would draw more than [`crate::export::MAX_KNOCKOUT_MASK_OBJECTS`] objects, the
/// group is written as a PDF knockout group, with a warning, and reads back as a knockout group.
#[test]
fn a_knockout_group_too_large_to_mask_is_a_pdf_knockout_group_with_a_warning() {
    // 400 elements that all overlap: 79,800 objects in their masks.
    const { assert!(400 * 399 / 2 > crate::export::MAX_KNOCKOUT_MASK_OBJECTS) };
    let d = knockout_squares(400, true, false);
    let r = export_with_report(&d, &PdfOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w == crate::export::KNOCKOUT_SHAPES), "{:?}", r.warnings);
    assert_eq!(text(&d).matches("/Ktrue").count(), 1);
    assert_eq!(knockout_groups(&import(&r.bytes).unwrap()), [400]);
}

/// In a PDF knockout group each painting operation knocks out the ones before it: an element with
/// two fills, the second in Multiply, is drawn as one group (through a full alpha mask), so that
/// its second fill blends with its first. An element with one fill needs no group.
#[test]
fn an_element_with_several_paints_is_one_group_in_a_knockout_group() {
    for (fills, groups) in [(1, 0), (2, 1)] {
        let mut d = Document::new(200.0, 200.0);
        let below = rect(Rect::new(0.0, 0.0, 100.0, 100.0), (0.0, 0.0, 1.0));
        let mut top = rect(Rect::new(50.0, 50.0, 150.0, 150.0), (1.0, 0.0, 0.0));
        if fills == 2 {
            let mut multiply = vectorcraft_doc::FillLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
            multiply.blend = BlendMode::Multiply;
            top.appearance.items.push(vectorcraft_doc::AppearanceItem::Fill(multiply));
        }
        top.id = d.alloc_id();
        let below = Node { id: d.alloc_id(), ..below };
        let mut g = Node::group(NodeId(0), vec![std::sync::Arc::new(below), std::sync::Arc::new(top)]);
        g.knockout = vectorcraft_doc::Knockout::On;
        add(&mut d, g);
        let t = text(&d);
        assert_eq!(t.matches("/Ktrue").count(), 1, "{fills}");
        let Some(start) = t.find("/Ktrue") else { panic!("no knockout group") };
        let group = &t[start..];
        let Some(end) = group.find("endstream") else { panic!("no knockout stream") };
        assert_eq!(group[..end].matches("Do").count(), groups, "{fills}: {t}");
    }
}

/// Elements that don't overlap need no masks of each other.
#[test]
fn knockout_masks_hold_only_the_elements_that_overlap() {
    let mut d = Document::new(200.0, 100.0);
    let squares = (0..3)
        .map(|i| {
            let x = f64::from(i) * 60.0;
            let mut s = rect(Rect::new(x, 0.0, x + 40.0, 40.0), (0.0, 0.0, 1.0));
            (s.id, s.opacity, s.knockout_shape) = (d.alloc_id(), 0.5, true);
            std::sync::Arc::new(s)
        })
        .collect();
    let mut g = Node::group(NodeId(0), squares);
    g.knockout = vectorcraft_doc::Knockout::On;
    add(&mut d, g);
    let t = text(&d);
    assert!(!t.contains("/Luminosity") && !t.contains("/Ktrue"), "{t}");
}

/// The copies of symbol instances drawn in knockout masks don't count toward the objects the
/// visible symbol art of an export may hold (`MAX_SYMBOL_NODES`): of two overlapping 50% instances
/// whose art placed through 9-slice guides holds about 6,510,000 objects each (a gradient square
/// in all nine cells, each copy with a disabled opacity mask of 723,351 objects), both are written
/// in their nine cells, although the lower one's mask draws a copy of the upper one first.
#[test]
fn knockout_mask_copies_of_symbol_instances_leave_the_visible_art_its_object_limit() {
    let d = panel_instances(850, &[20.0, 40.0]);
    // Nine squares of each instance (the mask's copy of the upper one is opacity-mask art).
    assert_eq!(painted_paths(&export(&d, &PdfOptions::default()).unwrap()), 18);
}

/// A knockout group of overlapping 50% instances, at `xs`, of a symbol whose art placed through
/// 9-slice guides holds 9 × (`mask_objects`² + `mask_objects` + 3) objects (a gradient square in
/// all nine cells, each copy with a disabled opacity mask of `mask_objects`² + `mask_objects` + 1
/// objects).
fn panel_instances(mask_objects: usize, xs: &[f64]) -> Document {
    use vectorcraft_color::{Gradient, GradientPaint};
    use vectorcraft_geom::Affine;
    let mut d = Document::new(300.0, 200.0);
    let leaf = std::sync::Arc::new(rect(Rect::new(0.0, 0.0, 1.0, 1.0), (0.0, 0.0, 0.0)));
    let inner = std::sync::Arc::new(Node::group(NodeId(0), vec![leaf; mask_objects]));
    let mut mask = vectorcraft_doc::OpacityMask::new(Node::group(NodeId(0), vec![inner; mask_objects]), true);
    (mask.linked, mask.disabled) = (false, true);
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let mut square = Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(gradient, Paint::None, 0.0));
    square.extras_mut().mask = Some(Box::new(mask));
    let art = Node::group(NodeId(0), vec![std::sync::Arc::new(square)]);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Panel".into(), art: std::sync::Arc::new(art) });
    d.set_symbol_nine_slice("Panel", Some(vectorcraft_doc::SymbolNineSlice::default()));
    let instances = xs
        .iter()
        .map(|&x| {
            let xf = Affine::translate((x, 20.0)) * Affine::scale_non_uniform(2.0, 1.5);
            let mut n = Node::new(NodeId(0), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Panel".into(), xf });
            (n.id, n.opacity, n.knockout_shape) = (d.alloc_id(), 0.5, true);
            std::sync::Arc::new(n)
        })
        .collect();
    let mut g = Node::group(NodeId(0), instances);
    g.knockout = vectorcraft_doc::Knockout::On;
    add(&mut d, g);
    d
}

/// The paths of `pdf` read back, the art of opacity masks left out.
fn painted_paths(pdf: &[u8]) -> usize {
    let mut paths = 0;
    import(pdf).unwrap().walk(|n| paths += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. })));
    paths
}

/// When the copies the knockout masks draw would hold more symbol objects than they may
/// (`MAX_SYMBOL_NODES` of their own), the group is written as a PDF knockout group, with a
/// warning, instead of masks that leave out art their elements cover: four overlapping instances
/// of about 3,870,000 objects each fit the visible art's limit, and the six copies in their masks
/// do not.
#[test]
fn knockout_masks_whose_symbol_copies_run_out_give_way_to_a_pdf_knockout_group() {
    let d = panel_instances(655, &[20.0, 40.0, 60.0, 80.0]);
    let r = export_with_report(&d, &PdfOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w == crate::export::KNOCKOUT_SHAPES), "{:?}", r.warnings);
    assert_eq!(painted_paths(&r.bytes), 36);
}

/// A brushed stroke draws brush art that the objects counted for the knockout masks leave out:
/// a group in which a mask would draw an element with a brushed stroke is written as a PDF
/// knockout group, with a warning. A brushed element that no mask draws (the lowest one) leaves
/// the masks as they are.
#[test]
fn a_knockout_group_whose_masks_would_draw_a_brushed_stroke_is_a_pdf_knockout_group() {
    for (brushed, expected) in [(1, true), (0, false)] {
        let mut d = Document::new(200.0, 200.0);
        let squares = (0..2)
            .map(|i| {
                let x = f64::from(i) * 40.0;
                let mut s = rect(Rect::new(20.0 + x, 50.0, 120.0 + x, 150.0), (0.0, 0.0, 1.0));
                (s.id, s.opacity, s.knockout_shape) = (d.alloc_id(), 0.5, true);
                if i == brushed {
                    let mut st = vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 3.0);
                    st.brush = Some("3 pt. Round".into());
                    s.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(st));
                }
                std::sync::Arc::new(s)
            })
            .collect();
        let mut g = Node::group(NodeId(0), squares);
        g.knockout = vectorcraft_doc::Knockout::On;
        add(&mut d, g);
        let r = export_with_report(&d, &PdfOptions::default()).unwrap();
        assert_eq!(r.warnings.iter().any(|w| w == crate::export::KNOCKOUT_BRUSHES), expected, "{brushed}: {:?}", r.warnings);
        assert!(!r.warnings.iter().any(|w| w == crate::export::KNOCKOUT_SHAPES), "{brushed}: {:?}", r.warnings);
        assert_eq!(text(&d).matches("/Ktrue").count(), usize::from(expected), "{brushed}");
    }
}

/// The knockout group of `upper` at 50%, whose opacity defines its knockout shape when
/// `knockout_shape`, between two 50% squares whose opacity defines theirs (so that the group is
/// drawn through masks): whether the export gives the brushed-stroke warning.
fn warns_of_brushes(mut d: Document, mut upper: Node, knockout_shape: bool) -> bool {
    (upper.id, upper.opacity, upper.knockout_shape) = (d.alloc_id(), 0.5, knockout_shape);
    let mut lower = rect(Rect::new(20.0, 50.0, 120.0, 150.0), (1.0, 0.0, 0.0));
    (lower.id, lower.opacity, lower.knockout_shape) = (d.alloc_id(), 0.5, true);
    let mut top = rect(Rect::new(100.0, 50.0, 180.0, 150.0), (0.0, 0.8, 0.0));
    (top.id, top.opacity, top.knockout_shape) = (d.alloc_id(), 0.5, true);
    let mut g = Node::group(NodeId(0), [lower, upper, top].map(std::sync::Arc::new).to_vec());
    g.knockout = vectorcraft_doc::Knockout::On;
    add(&mut d, g);
    let r = export_with_report(&d, &PdfOptions::default()).unwrap();
    r.warnings.iter().any(|w| w == crate::export::KNOCKOUT_BRUSHES)
}

/// A square at (60, 50) with a 3 pt brushed stroke.
fn brushed_square() -> Node {
    let mut line = rect(Rect::new(60.0, 50.0, 160.0, 150.0), (1.0, 1.0, 1.0));
    let mut st = vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 3.0);
    st.brush = Some("3 pt. Round".into());
    line.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(st));
    line
}

/// Only brushed strokes a mask draws count: a hidden member's is not drawn, nor is one without
/// paint or width, nor the one in the opacity mask of an element whose mask does not define its
/// knockout shape (its copy in a mask leaves the opacity mask out).
#[test]
fn brushed_strokes_that_a_mask_does_not_draw_leave_the_masks_as_they_are() {
    let d = Document::new(200.0, 200.0);
    let mut hidden = brushed_square();
    hidden.visible = false;
    let plain = rect(Rect::new(60.0, 50.0, 160.0, 150.0), (0.0, 0.0, 1.0));
    let group = Node::group(NodeId(0), vec![std::sync::Arc::new(plain.clone()), std::sync::Arc::new(hidden)]);
    assert!(!warns_of_brushes(d.clone(), group, true));
    // A brushed stroke without paint or width draws nothing.
    for (paint, width) in [(Paint::None, 3.0), (Paint::solid(Color::BLACK), 0.0)] {
        let mut idle = brushed_square();
        if let Some(vectorcraft_doc::AppearanceItem::Stroke(st)) = idle.appearance.items.last_mut() {
            (st.paint, st.width) = (paint, width);
        }
        assert!(!warns_of_brushes(d.clone(), idle, true), "width {width}");
    }
    assert!(warns_of_brushes(d.clone(), brushed_square(), true));
    // A group's own brushed stroke is drawn too: the export bakes it into a path among the
    // group's members.
    let mut stroked_group = Node::group(NodeId(0), vec![std::sync::Arc::new(plain.clone())]);
    stroked_group.appearance = brushed_square().appearance;
    assert!(warns_of_brushes(d.clone(), stroked_group, true));
    for (knockout_shape, expected) in [(false, false), (true, true)] {
        let mut masked = plain.clone();
        let mut mask = vectorcraft_doc::OpacityMask::new(brushed_square(), true);
        mask.linked = false;
        masked.extras_mut().mask = Some(Box::new(mask));
        assert_eq!(warns_of_brushes(d.clone(), masked, knockout_shape), expected, "knockout shape {knockout_shape}");
    }
}

/// A compound path's members are drawn with its own paint, so a brushed stroke on a member draws
/// nothing; one on the compound path does.
#[test]
fn a_brushed_stroke_counts_on_a_compound_path_and_not_on_its_members() {
    let d = Document::new(200.0, 200.0);
    let compound = |members: Vec<Node>| {
        let children = members.into_iter().map(std::sync::Arc::new).collect();
        let mut c = Node::new(NodeId(0), vectorcraft_doc::NodeKind::Compound { children, rule: Default::default() });
        c.appearance = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0).into();
        c
    };
    assert!(!warns_of_brushes(d.clone(), compound(vec![brushed_square()]), true));
    let mut stroked = compound(vec![rect(Rect::new(60.0, 50.0, 160.0, 150.0), (1.0, 1.0, 1.0))]);
    stroked.appearance = brushed_square().appearance;
    assert!(warns_of_brushes(d, stroked, true));
}

/// A pattern's tiles draw the brushed strokes in its art: an element filled with such a pattern
/// counts, and one filled with a pattern without them does not.
#[test]
fn a_brushed_stroke_in_a_pattern_counts() {
    for (brushed_tile, expected) in [(true, true), (false, false)] {
        let mut d = Document::new(200.0, 200.0);
        let tile = if brushed_tile { brushed_square() } else { rect(Rect::new(60.0, 50.0, 160.0, 150.0), (1.0, 1.0, 1.0)) };
        d.patterns.push(vectorcraft_doc::pattern::PatternDef::new("Tile", vec![std::sync::Arc::new(tile)]));
        let pattern = Paint::Pattern { pattern: "Tile".into(), xf: vectorcraft_geom::Affine::IDENTITY };
        let filled = Node::path(NodeId(0), shapes::rectangle(Rect::new(60.0, 50.0, 160.0, 150.0)), Appearance::basic(pattern, Paint::None, 0.0));
        assert_eq!(warns_of_brushes(d, filled, true), expected, "brushed tile {brushed_tile}");
    }
}

/// A stroke whose brush the export does not know is drawn as a plain stroke, and a stroke without
/// width draws nothing, not even its pattern's brushed strokes.
#[test]
fn strokes_that_draw_no_brush_art_leave_the_masks_as_they_are() {
    let mut unknown = brushed_square();
    if let Some(vectorcraft_doc::AppearanceItem::Stroke(st)) = unknown.appearance.items.last_mut() {
        st.brush = Some("No Such Brush".into());
    }
    assert!(!warns_of_brushes(Document::new(200.0, 200.0), unknown, true));
    let mut d = Document::new(200.0, 200.0);
    d.patterns.push(vectorcraft_doc::pattern::PatternDef::new("Tile", vec![std::sync::Arc::new(brushed_square())]));
    let pattern = Paint::Pattern { pattern: "Tile".into(), xf: vectorcraft_geom::Affine::IDENTITY };
    let mut thin = rect(Rect::new(60.0, 50.0, 160.0, 150.0), (0.0, 0.0, 1.0));
    thin.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(pattern, 0.0)));
    assert!(!warns_of_brushes(d, thin, true));
}

/// Type draws the brushed strokes in the patterns its characters are filled with and in the art
/// of its inline graphics.
#[test]
fn brushed_strokes_in_type_patterns_and_inline_graphics_count() {
    let text = |d: &mut Document, fill: Paint, inline: bool| {
        let style = vectorcraft_doc::CharStyle { size: 60.0, fill, ..Default::default() };
        let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(60.0, 120.0), "Type", style.clone());
        if inline {
            d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Brushed".into(), art: std::sync::Arc::new(brushed_square()) });
            let mut run = vectorcraft_doc::TextRun::new("\u{FFFC}", style);
            run.inline = Some(vectorcraft_doc::InlineArt::new("Brushed"));
            t.runs.push(run);
        }
        Node::new(NodeId(0), vectorcraft_doc::NodeKind::Text(Box::new(t)))
    };
    let black = Paint::solid(Color::BLACK);
    let mut d = Document::new(200.0, 200.0);
    let plain = text(&mut d, black.clone(), false);
    assert!(!warns_of_brushes(d.clone(), plain, true));
    d.patterns.push(vectorcraft_doc::pattern::PatternDef::new("Tile", vec![std::sync::Arc::new(brushed_square())]));
    let patterned = text(&mut d, Paint::Pattern { pattern: "Tile".into(), xf: vectorcraft_geom::Affine::IDENTITY }, false);
    assert!(warns_of_brushes(d, patterned, true));
    let mut d = Document::new(200.0, 200.0);
    let inline = text(&mut d, black, true);
    assert!(warns_of_brushes(d, inline, true));
}

/// Type draws the brushed strokes of its appearance as brush art too.
#[test]
fn a_brushed_stroke_on_type_counts() {
    let style = vectorcraft_doc::CharStyle { size: 60.0, ..Default::default() };
    let text = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(60.0, 120.0), "Type", style);
    let mut n = Node::new(NodeId(0), vectorcraft_doc::NodeKind::Text(Box::new(text)));
    n.appearance = brushed_square().appearance;
    assert!(warns_of_brushes(Document::new(200.0, 200.0), n, true));
}

/// The brushed strokes a mask would draw are found in the opacity-mask art of a symbol's art too,
/// and a disabled opacity mask, which is not drawn, holds none.
#[test]
fn brushed_strokes_in_mask_art_count_only_where_the_mask_is_on() {
    for (in_symbol, disabled, expected) in [(true, false, true), (false, true, false), (false, false, true)] {
        let mut d = Document::new(200.0, 200.0);
        let mut line = rect(Rect::new(0.0, 0.0, 100.0, 100.0), (1.0, 1.0, 1.0));
        let mut st = vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 3.0);
        st.brush = Some("3 pt. Round".into());
        line.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(st));
        let mut mask = vectorcraft_doc::OpacityMask::new(line, true);
        (mask.linked, mask.disabled) = (false, disabled);
        let mut masked = rect(Rect::new(0.0, 0.0, 100.0, 100.0), (0.0, 0.0, 1.0));
        masked.extras_mut().mask = Some(Box::new(mask));
        let mut upper = if in_symbol {
            let art = Node::group(NodeId(0), vec![std::sync::Arc::new(masked)]);
            d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Masked".into(), art: std::sync::Arc::new(art) });
            let xf = vectorcraft_geom::Affine::translate((60.0, 50.0));
            Node::new(NodeId(0), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Masked".into(), xf })
        } else {
            masked.transform(vectorcraft_geom::Affine::translate((60.0, 50.0)), false);
            masked
        };
        (upper.id, upper.opacity, upper.knockout_shape) = (d.alloc_id(), 0.5, true);
        let mut lower = rect(Rect::new(20.0, 50.0, 120.0, 150.0), (1.0, 0.0, 0.0));
        (lower.id, lower.opacity, lower.knockout_shape) = (d.alloc_id(), 0.5, true);
        let mut g = Node::group(NodeId(0), vec![std::sync::Arc::new(lower), std::sync::Arc::new(upper)]);
        g.knockout = vectorcraft_doc::Knockout::On;
        add(&mut d, g);
        let r = export_with_report(&d, &PdfOptions::default()).unwrap();
        let warned = r.warnings.iter().any(|w| w == crate::export::KNOCKOUT_BRUSHES);
        assert_eq!(warned, expected, "symbol {in_symbol}, disabled {disabled}: {:?}", r.warnings);
    }
}

/// PDF/A-2 allows transparency: a knockout group is written as one there too.
#[test]
fn a_pdf_a_2b_file_keeps_a_knockout_group() {
    let mut o = PdfOptions::default();
    (o.settings.standard, o.settings.compatibility) = (Standard::PdfA2b, Standard::PdfA2b.version());
    let bytes = export(&knockout_squares(3, false, false), &o).unwrap();
    assert_eq!(knockout_groups(&import(&bytes).unwrap()), [3]);
}
