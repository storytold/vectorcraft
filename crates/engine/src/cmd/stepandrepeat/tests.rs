use serde_json::json;

use super::*;

const A4: (f64, f64) = (210.0, 297.0);

#[test]
fn fills_a4_with_50_by_30_items_and_3_gap() {
    assert_eq!(fill_counts(A4.0, A4.1, 0.0, 50.0, 30.0, 3.0, 3.0), (9, 4));
    assert_eq!(fill_counts(A4.0, A4.1, 5.0, 50.0, 30.0, 3.0, 3.0), (8, 3));
    assert_eq!(fill_counts(40.0, 297.0, 0.0, 50.0, 30.0, 3.0, 3.0), (9, 0));
}

#[test]
fn layout_places_cells_at_the_pitch_and_alternates_rotation() {
    let c = layout(50.0, 30.0, 3.0, 3.0, 9, 4, Alternate::Columns);
    assert_eq!(c.len(), 36);
    let at = |r: usize, k: usize| c[r * 4 + k];
    assert_eq!((at(0, 0).dx, at(0, 0).dy), (0.0, 0.0));
    assert_eq!((at(2, 3).dx, at(2, 3).dy), (159.0, 66.0));
    assert_eq!(c.iter().filter(|c| c.turned).count(), 18);
    assert!(at(4, 1).turned && at(4, 3).turned && !at(4, 0).turned && !at(4, 2).turned);
    let rows = layout(50.0, 30.0, 3.0, 3.0, 3, 2, Alternate::Rows);
    assert_eq!(rows.iter().map(|c| c.turned).collect::<Vec<_>>(), [false, false, true, true, false, false]);
    assert!(layout(50.0, 30.0, 3.0, 3.0, 3, 2, Alternate::None).iter().all(|c| !c.turned));
}

fn setup() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 210, "height": 297})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 30})).unwrap()["id"].as_u64().unwrap();
    (s, id)
}

fn bounds(s: &Session, id: u64) -> (f64, f64, f64, f64) {
    let b = s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
    (b.x0, b.y0, b.x1, b.y1)
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn creates_the_grid_in_one_undo_step_and_undoes_cleanly() {
    let (mut s, _) = setup();
    let (before, steps) = (s.doc().unwrap().doc.clone(), undo_steps(&s));
    let r = s.execute(CMD, &json!({"rows": 2, "columns": 3, "gap": 3})).unwrap();
    assert_eq!((r["count"].clone(), r["rows"].clone(), r["columns"].clone()), (json!(6), json!(2), json!(3)));
    assert_eq!(undo_steps(&s), steps + 1);
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|i| i.as_u64().unwrap()).collect();
    assert_eq!(bounds(&s, ids[0]), (0.0, 0.0, 50.0, 30.0));
    assert_eq!(bounds(&s, ids[4]), (53.0, 33.0, 103.0, 63.0));
    assert_eq!(bounds(&s, ids[5]), (106.0, 33.0, 156.0, 63.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(undo_steps(&s), steps);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards.len(), before.artboards.len());
    assert!(d.node(NodeId(ids[0])).is_some());
    assert!(ids[1..].iter().all(|i| d.node(NodeId(*i)).is_none()));
}

#[test]
fn fill_sheet_with_margin_moves_the_first_copy_to_the_corner() {
    let (mut s, _) = setup();
    let r = s.execute(CMD, &json!({"fill": true, "gap": 3, "margin": 0})).unwrap();
    assert_eq!((r["rows"].clone(), r["columns"].clone(), r["count"].clone()), (json!(9), json!(4), json!(36)));
    let (mut s, _) = setup();
    let r = s.execute(CMD, &json!({"fill": true, "gap": 3, "margin": 5})).unwrap();
    assert_eq!(r["count"], json!(24));
    let first = r["ids"][0].as_u64().unwrap();
    assert_eq!(bounds(&s, first), (5.0, 5.0, 55.0, 35.0));
}

#[test]
fn fill_places_every_cell_from_the_original_when_it_starts_away_from_the_margin() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 210, "height": 297})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 40, "width": 50, "height": 30})).unwrap();
    let r = s.execute(CMD, &json!({"fill": true, "gap": 3, "margin": 5, "rotate": "columns"})).unwrap();
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|i| i.as_u64().unwrap()).collect();
    assert_eq!((r["rows"].clone(), r["columns"].clone()), (json!(8), json!(3)));
    assert_eq!(bounds(&s, ids[0]), (5.0, 5.0, 55.0, 35.0));
    assert_eq!(bounds(&s, ids[1]), (58.0, 5.0, 108.0, 35.0));
    assert_eq!(bounds(&s, ids[2]), (111.0, 5.0, 161.0, 35.0));
    assert_eq!(bounds(&s, ids[3]), (5.0, 38.0, 55.0, 68.0));
}

#[test]
fn alternate_columns_turn_by_180_degrees_about_the_cell_center() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 210, "height": 297})).unwrap();
    let id = s.execute("shape.polygon", &json!({"cx": 25, "cy": 15, "radius": 10, "sides": 3})).unwrap()["id"].as_u64().unwrap();
    let b0 = bounds(&s, id);
    let r = s.execute(CMD, &json!({"rows": 1, "columns": 2, "gap": 4, "rotate": "columns"})).unwrap();
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|i| i.as_u64().unwrap()).collect();
    let (w, b1) = (b0.2 - b0.0, bounds(&s, ids[1]));
    assert!((b1.0 - (b0.0 + w + 4.0)).abs() < 1e-6 && ((b1.2 - b1.0) - w).abs() < 1e-6);
    assert!((b1.1 - b0.1).abs() < 1e-6 && (b1.3 - b0.3).abs() < 1e-6);
    let tri = |s: &Session, id: u64| format!("{:?}", s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind);
    assert_ne!(tri(&s, ids[0]), tri(&s, ids[1]));
}

#[test]
fn a_cut_contour_stays_grouped_with_its_artwork_in_every_copy() {
    let (mut s, art) = setup();
    let cut = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [art, cut]})).unwrap();
    let r = s.execute(CMD, &json!({"rows": 1, "columns": 3, "gap": 2, "cutId": cut})).unwrap();
    assert_eq!(r["count"], json!(3));
    let d = &s.doc().unwrap().doc;
    for id in r["ids"].as_array().unwrap() {
        let n = d.node(NodeId(id.as_u64().unwrap())).unwrap();
        assert!(matches!(n.kind, vectorcraft_doc::NodeKind::Group { .. }));
        let mut paths = 0;
        n.walk(&mut |c| paths += usize::from(matches!(c.kind, vectorcraft_doc::NodeKind::Path { .. })));
        assert_eq!(paths, 2);
    }
}

#[test]
fn rejects_bad_params_and_a_selection_that_does_not_fit() {
    let (mut s, _) = setup();
    assert!(s.execute(CMD, &json!({"rows": 0})).is_err());
    assert!(s.execute(CMD, &json!({"rows": 1000, "columns": 1000})).is_err());
    assert!(s.execute(CMD, &json!({"rotate": "diagonal"})).is_err());
    assert!(s.execute(CMD, &json!({"cutId": 999999})).is_err());
    assert!(s.execute(CMD, &json!({"fill": true, "sheet": {"x": 0, "y": 0, "width": 40, "height": 40}})).is_err());
    let mut e = Session::new();
    e.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    assert!(e.execute(CMD, &json!({})).is_err());
}
