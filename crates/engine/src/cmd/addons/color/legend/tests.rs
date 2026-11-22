use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_doc::{Node, NodeId, NodeKind};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn group(s: &Session, id: u64) -> Node {
    s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone()
}

/// The text of each label in group `g`, in order.
fn labels(g: &Node) -> Vec<(String, Color)> {
    g.children()
        .unwrap()
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some((t.plain_text(), t.runs[0].style.fill.color().unwrap())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_legend_draws_a_labelled_chip_per_colour_swatch_in_one_undo_step() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#101010"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Paper", "color": "#f8f4e8"})).unwrap();
    let colours = s.doc().unwrap().doc.swatches_iter().filter(|w| w.paint.color().is_some()).count();
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("addon.color.swatchLegend", &json!({"columns": 3, "size": 60, "gap": 10})).unwrap();
    assert_eq!(r["count"].as_u64().unwrap() as usize, colours);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    assert_eq!(s.doc().unwrap().selection.objects, [NodeId(r["id"].as_u64().unwrap())], "the legend is selected");
    let g = group(&s, r["id"].as_u64().unwrap());
    assert_eq!(g.name.as_deref(), Some("Swatch Legend"));
    let kids = g.children().unwrap();
    assert_eq!(kids.len(), 2 * colours, "a chip and a label each");
    // Chips fill with their swatch, three to a row, 70 pt apart.
    let chips: Vec<&Node> = kids.iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).map(|n| &**n).collect();
    let doc = &s.doc().unwrap().doc;
    for (chip, w) in chips.iter().zip(doc.swatches_iter().filter(|w| w.paint.color().is_some())) {
        assert_eq!(chip.appearance.fill_paint().color(), w.paint.color(), "{}", w.name);
        assert_eq!(chip.name.as_deref(), Some(w.name.as_str()));
    }
    let b = |i: usize| chips[i].geometric_bounds().unwrap();
    assert!((b(1).x0 - b(0).x0 - 70.0).abs() < 1e-6 && (b(3).y0 - b(0).y0 - 70.0).abs() < 1e-6 && (b(3).x0 - b(0).x0).abs() < 1e-6);
    assert!((b(0).width() - 60.0).abs() < 1e-6);
    // Labels: the name, then the swatch's own values; white on the dark chip, black on the light.
    let l = labels(&g);
    let ink = l.iter().find(|(t, _)| t.starts_with("Ink")).unwrap();
    let paper = l.iter().find(|(t, _)| t.starts_with("Paper")).unwrap();
    assert!(ink.0.contains("R=16 G=16 B=16"), "{}", ink.0);
    assert_eq!((ink.1, paper.1), (Color::WHITE, Color::BLACK));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(g.id).is_none());
}

#[test]
fn a_legend_takes_names_values_and_a_position() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Signal", "color": "cmyk 0 100 100 0", "spot": true})).unwrap();
    let r =
        s.execute("addon.color.swatchLegend", &json!({"names": ["Signal"], "values": ["hex", "cmyk", "lab", "gray"], "x": 100, "y": 50})).unwrap();
    assert_eq!(r["count"], 1);
    let g = group(&s, r["id"].as_u64().unwrap());
    let (text, _) = labels(&g).remove(0);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "Signal");
    assert_eq!(lines[1], "Spot", "a spot swatch says so");
    assert!(lines[2].starts_with('#') && lines[3] == "C=0 M=100 Y=100 K=0" && lines[4].starts_with("L=") && lines[5].starts_with("K="), "{lines:?}");
    let chip = g.children().unwrap().iter().find(|n| matches!(n.kind, NodeKind::Path { .. })).unwrap().geometric_bounds().unwrap();
    assert!((chip.x0 - 100.0).abs() < 1e-6 && (chip.y0 - 50.0).abs() < 1e-6);
    // Refused, with nothing drawn.
    let before = s.doc().unwrap().history.undo.len();
    for bad in [
        json!({"names": ["Nope"]}),
        json!({"names": "Signal"}),
        json!({"values": ["xyz"]}),
        json!({"columns": 0}),
        json!({"columns": 2.5}),
        json!({"size": 2}),
        json!({"gap": -1}),
        json!({"x": f64::MAX}),
        json!({"names": []}),
        json!({"values": ["rgb", "rgb"]}),
    ] {
        assert!(s.execute("addon.color.swatchLegend", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), before);
}

/// A chip of a global swatch is linked to it: editing the swatch recolours the chip. Each label is
/// area type inside its chip that shrinks a long name to fit (#4919 review).
#[test]
fn chips_stay_linked_and_labels_stay_on_their_chips() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "A Rather Long Brand Colour Name", "color": "#2050a0", "global": true})).unwrap();
    let r = s
        .execute("addon.color.swatchLegend", &json!({"names": ["A Rather Long Brand Colour Name"], "size": 40, "values": ["hex", "rgb", "cmyk"]}))
        .unwrap();
    let id = r["id"].as_u64().unwrap();
    s.execute("swatch.edit", &json!({"name": "A Rather Long Brand Colour Name", "color": "#a02050"})).unwrap();
    let g = group(&s, id);
    let kids = g.children().unwrap();
    let chip = kids.iter().find(|n| matches!(n.kind, NodeKind::Path { .. })).unwrap();
    assert_eq!(chip.appearance.fill_paint().color().map(|c| c.to_hex()), Some("#a02050".into()), "the edit reached the chip");
    let label = kids.iter().find(|n| matches!(n.kind, NodeKind::Text(_))).unwrap();
    let NodeKind::Text(t) = &label.kind else { panic!("not type") };
    assert!(matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }) && matches!(t.area.fit, vectorcraft_doc::AreaFit::ShrinkText { .. }));
    let (c, l) = (chip.geometric_bounds().unwrap(), label.geometric_bounds().unwrap());
    assert!(l.x0 >= c.x0 - 1e-6 && l.y0 >= c.y0 - 1e-6 && l.x1 <= c.x1 + 1e-6 && l.y1 <= c.y1 + 1e-6, "label {l:?} inside chip {c:?}");
}
