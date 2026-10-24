//! Object IDs: names survive the round trip (Layer Names), Minimal writes only referenced ids,
//! Unique ids never collide, and other apps' label attributes become object names.

use std::collections::HashSet;

use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, ObjectIds, Styling, export, import};

/// A document with a named rectangle, an unnamed gradient-filled one and a second layer.
fn doc(title: &str, shift: f64) -> Document {
    let mut d = Document::new(200.0, 200.0);
    d.title = title.into();
    let l = d.layers[0].id;
    let mut a = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0 + shift, 10.0, 60.0, 60.0)), Appearance::default_art());
    a.name = Some("Logo mark (v2)".into());
    d.insert(Some(l), usize::MAX, a).unwrap();
    let g = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let b = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(80.0, 10.0, 120.0, 60.0)), Appearance::basic(g, Paint::None, 0.0));
    d.insert(Some(l), usize::MAX, b).unwrap();
    d.add_layer(Some("Top & Front"));
    let top = d.layers[1].id;
    let c = Node::path(
        d.alloc_id(),
        shapes::ellipse(Rect::new(0.0, 100.0, 50.0, 150.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    d.insert(Some(top), usize::MAX, c).unwrap();
    d
}

/// Every `id="…"` value.
fn ids(svg: &str) -> Vec<String> {
    svg.split(" id=\"").skip(1).map(|s| s[..s.find('"').unwrap()].to_string()).collect()
}

fn names(d: &Document) -> Vec<String> {
    let mut v = vec![];
    d.walk(|n| v.extend(n.name.clone()));
    v
}

#[test]
fn layer_names_round_trip_exactly() {
    let d = doc("a", 0.0);
    let s = export(&d, &ExportOptions::default());
    assert!(s.contains("id=\"Layer_1\" data-name=\"Layer 1\""), "{s}");
    assert!(s.contains("data-name=\"Top &amp; Front\""), "{s}");
    let back = import(&s).unwrap();
    assert_eq!(back.layers.iter().map(|l| l.name.clone().unwrap()).collect::<Vec<_>>(), ["Layer 1", "Top & Front"]);
    assert!(names(&back).contains(&"Logo mark (v2)".to_string()), "{:?}", names(&back));
    // An id that is the name already gets no data-name.
    let mut d = Document::new(10.0, 10.0);
    std::sync::Arc::make_mut(&mut d.layers[0]).name = Some("Layer_1".into());
    let s = export(&d, &ExportOptions::default());
    assert!(s.contains("<g id=\"Layer_1\">"), "{s}");
}

#[test]
fn minimal_writes_only_referenced_ids() {
    let s = export(&doc("a", 0.0), &ExportOptions { object_ids: ObjectIds::Minimal, ..Default::default() });
    let ids = ids(&s);
    assert!(!ids.is_empty(), "the gradient keeps its id");
    for id in &ids {
        assert!(s.contains(&format!("#{id})")) || s.contains(&format!("\"#{id}\"")), "`{id}` is never referenced:\n{s}");
    }
    assert!(!s.contains("data-name"));
}

#[test]
fn unique_ids_never_collide() {
    let opts = ExportOptions { object_ids: ObjectIds::Unique, styling: Styling::InternalCss, ..Default::default() };
    let a = export(&doc("a", 0.0), &opts);
    let b = export(&doc("b", 5.0), &opts);
    let (ia, ib) = (ids(&a), ids(&b));
    assert_eq!(ia.len(), ia.iter().collect::<HashSet<_>>().len(), "unique within a document");
    assert!(ia.iter().all(|i| !ib.contains(i)), "{ia:?} vs {ib:?}");
    // CSS class names are page-global too.
    let classes = |s: &str| s.split("class=\"").skip(1).map(|s| s[..s.find('"').unwrap()].to_string()).collect::<HashSet<_>>();
    assert!(classes(&a).is_disjoint(&classes(&b)));
    // The same document gets the same ids, and the names still come back.
    assert_eq!(a, export(&doc("a", 0.0), &opts));
    let back = import(&a).unwrap();
    assert_eq!(back.layers[0].name.as_deref(), Some("Layer 1"));
    assert!(names(&back).contains(&"Logo mark (v2)".to_string()));
}

#[test]
fn label_attributes_name_objects() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns:serif="http://www.serif.com/" width="100" height="100">
        <g id="layer1" inkscape:label="Background">
            <rect id="r1" data-name="Sky box" x="0" y="0" width="10" height="10"/>
            <rect id="r2" serif:id="Sun disc" x="20" y="0" width="10" height="10"/>
            <rect id="r3" aria-label="Badge" x="40" y="0" width="10" height="10"/>
            <rect id="plain" x="60" y="0" width="10" height="10"/>
        </g>
    </svg>"##;
    let d = import(svg).unwrap();
    assert_eq!(d.layers[0].name.as_deref(), Some("Background"));
    assert_eq!(names(&d), ["Background", "Sky box", "Sun disc", "Badge", "plain"]);
}
