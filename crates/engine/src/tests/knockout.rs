//! Knockout groups, the knockout shape and the page group (M3.46–M3.48): commands, undo, files
//! (old bools load) and the PDF and SVG output.

use serde_json::{Value, json};
use vectorcraft_doc::Knockout;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn node(s: &Session, id: u64) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone()
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// A group of two overlapping 50% rectangles, selected. Returns the group id.
fn knockout_group(s: &mut Session) -> u64 {
    let a = run(s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 100}))["id"].clone();
    let b = run(s, "shape.rectangle", json!({"x": 60, "y": 60, "width": 100, "height": 100}))["id"].clone();
    run(s, "transparency.set", json!({"ids": [a, b], "opacity": 50}));
    run(s, "select.set", json!({"ids": [a, b]}));
    run(s, "object.group", json!({}))["id"].as_u64().unwrap()
}

#[test]
fn knockout_takes_three_states_names_and_bools() {
    let mut s = session();
    let g = knockout_group(&mut s);
    assert_eq!(node(&s, g).knockout, Knockout::Neutral, "new groups are neutral");
    for (param, want) in [
        (json!("on"), Knockout::On),
        (json!("Off"), Knockout::Off),
        (json!("neutral"), Knockout::Neutral),
        (json!(true), Knockout::On),
        (json!(false), Knockout::Neutral),
    ] {
        let before = steps(&s);
        run(&mut s, "transparency.set", json!({"knockout": param}));
        assert_eq!((node(&s, g).knockout, steps(&s)), (want, before + 1), "{param}");
    }
    run(&mut s, "transparency.set", json!({"knockout": "off"}));
    let info = run(&mut s, "transparency.info", json!({}));
    assert_eq!((info["knockout"].clone(), info["knockoutShape"].clone()), (json!("off"), json!(false)));
    // A bad state changes nothing, also next to an opacity (which would go to an appearance item).
    let before = steps(&s);
    assert!(s.execute("transparency.set", &json!({"knockout": "maybe"})).is_err());
    assert!(s.execute("transparency.set", &json!({"knockout": 3, "opacity": 20})).is_err());
    assert!(s.execute("object.setProps", &json!({"knockout": "maybe"})).is_err());
    assert_eq!((node(&s, g).knockout, node(&s, g).opacity, steps(&s)), (Knockout::Off, 1.0, before));
    // Undo is one step per change.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, g).knockout, Knockout::Neutral);
    // The panel's click order.
    assert_eq!([Knockout::On, Knockout::Neutral, Knockout::Off].map(Knockout::cycle), [Knockout::Neutral, Knockout::Off, Knockout::On]);
}

#[test]
fn knockout_shape_is_an_object_option() {
    let mut s = session();
    let g = knockout_group(&mut s);
    let child = node(&s, g).children().unwrap()[1].id.0;
    run(&mut s, "transparency.set", json!({"ids": [child], "knockoutShape": true}));
    assert!(node(&s, child).knockout_shape && !node(&s, child).has_default_transparency());
    let info = run(&mut s, "transparency.info", json!({"id": child}));
    assert_eq!(info["knockoutShape"], true);
    let mixed = run(&mut s, "transparency.info", json!({"ids": [child, g]}));
    assert_eq!(mixed["knockoutShape"], Value::Null);
    run(&mut s, "edit.undo", json!({}));
    assert!(!node(&s, child).knockout_shape);
}

#[test]
fn page_group_toggles_are_undoable_document_settings() {
    let mut s = session();
    let doc = |s: &Session| (s.doc().unwrap().doc.page_isolate, s.doc().unwrap().doc.page_knockout);
    let before = steps(&s);
    assert_eq!(run(&mut s, "transparency.togglePageIsolatedBlending", json!({})), json!({"value": true}));
    assert_eq!(run(&mut s, "transparency.togglePageKnockoutGroup", json!({"value": true})), json!({"value": true}));
    assert_eq!((doc(&s), steps(&s)), ((true, true), before + 2));
    // Setting the current value is not a change.
    run(&mut s, "transparency.togglePageKnockoutGroup", json!({"value": true}));
    assert_eq!(steps(&s), before + 2);
    let info = run(&mut s, "transparency.info", json!({}));
    assert_eq!((info["pageIsolatedBlending"].clone(), info["pageKnockoutGroup"].clone()), (json!(true), json!(true)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(doc(&s), (true, false));
    run(&mut s, "transparency.togglePageIsolatedBlending", json!({}));
    assert_eq!(doc(&s), (false, false));
}

#[test]
fn knockout_settings_round_trip_and_old_bools_load() {
    let mut s = session();
    let g = knockout_group(&mut s);
    let child = node(&s, g).children().unwrap()[0].id.0;
    run(&mut s, "transparency.set", json!({"ids": [g], "knockout": "off"}));
    run(&mut s, "transparency.set", json!({"ids": [child], "knockout": "on", "knockoutShape": true}));
    run(&mut s, "transparency.togglePageIsolatedBlending", json!({}));
    run(&mut s, "transparency.togglePageKnockoutGroup", json!({}));
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    let (bg, bc) = (back.node(NodeId(g)).unwrap(), back.node(NodeId(child)).unwrap());
    assert_eq!((bg.knockout, bc.knockout, bc.knockout_shape, back.page_isolate, back.page_knockout), (Knockout::Off, Knockout::On, true, true, true));
    // Defaults are not written.
    let text = String::from_utf8(vectorcraft_format::save(&session().doc().unwrap().doc, false)).unwrap();
    assert!(!text.contains("knockout") && !text.contains("page_isolate"), "{text}");
    // Files from before the three states stored a bool: true is On, false is Neutral.
    let mut v: Value = serde_json::from_slice(&bytes).unwrap();
    let mut patched = 0;
    fn patch(v: &mut Value, n: &mut usize) {
        match v {
            Value::Object(m) => {
                if let Some(k) = m.get_mut("knockout") {
                    *k = json!(*k == json!("on"));
                    *n += 1;
                }
                m.values_mut().for_each(|c| patch(c, n));
            }
            Value::Array(a) => a.iter_mut().for_each(|c| patch(c, n)),
            _ => {}
        }
    }
    patch(&mut v, &mut patched);
    assert_eq!(patched, 2);
    let old = vectorcraft_format::load(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!((old.node(NodeId(g)).unwrap().knockout, old.node(NodeId(child)).unwrap().knockout), (Knockout::Neutral, Knockout::On));
}

#[test]
fn pdf_and_svg_write_knockout_groups_as_masks() {
    let mut s = session();
    let g = knockout_group(&mut s);
    let plain = s.doc().unwrap().doc.clone();
    run(&mut s, "transparency.set", json!({"ids": [g], "knockout": "on"}));
    let ko = s.doc().unwrap().doc.clone();

    let pdf = |d: &Document| vectorcraft_pdf::export_with_report(d, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    let r = pdf(&plain);
    assert!(!String::from_utf8_lossy(&r.bytes).contains("/SMask") && r.warnings.is_empty());
    let r = pdf(&ko);
    let text = String::from_utf8_lossy(&r.bytes);
    // The lower rectangle is drawn through a luminosity mask made of the upper one's alpha.
    assert!(text.contains("/Luminosity") && text.contains("/Alpha"), "soft masks");
    assert!(r.warnings.iter().any(|w| w.contains("knockout")), "{:?}", r.warnings);

    let svg = |d: &Document| vectorcraft_svg::export(d, &Default::default());
    assert!(!svg(&plain).contains("knockout"));
    let out = svg(&ko);
    assert_eq!(out.matches("mask=\"url(#knockout-").count(), 1, "{out}");
    assert!(out.contains("<feColorMatrix") && out.contains("isolation"), "{out}");

    // A neutral group passes its children through: still one mask per element below the top one.
    run(&mut s, "select.set", json!({"ids": [g]}));
    let outer = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "transparency.set", json!({"ids": [g], "knockout": "neutral"}));
    run(&mut s, "transparency.set", json!({"ids": [outer], "knockout": "on"}));
    assert_eq!(svg(&s.doc().unwrap().doc).matches("mask=\"url(#knockout-").count(), 1);

    // The page group: an isolated group around the page content.
    let mut page = (*plain).clone();
    page.page_isolate = true;
    assert!(svg(&page).contains("isolation"));
    page.page_knockout = true;
    assert_eq!(svg(&page).matches("mask=\"url(#knockout-").count(), 1, "the layer is neutral: its objects knock each other out");
    assert!(String::from_utf8_lossy(&pdf(&page).bytes).contains("/Luminosity"));
}
