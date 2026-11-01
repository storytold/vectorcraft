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
