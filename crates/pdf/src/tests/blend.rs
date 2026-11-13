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

/// PDF/A-2 allows transparency: a knockout group is written as one there too.
#[test]
fn a_pdf_a_2b_file_keeps_a_knockout_group() {
    let mut o = PdfOptions::default();
    (o.settings.standard, o.settings.compatibility) = (Standard::PdfA2b, Standard::PdfA2b.version());
    let bytes = export(&knockout_squares(3, false, false), &o).unwrap();
    assert_eq!(knockout_groups(&import(&bytes).unwrap()), [3]);
}
