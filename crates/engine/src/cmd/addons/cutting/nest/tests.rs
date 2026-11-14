//! Tests for `addon.cutting.nest`: packing the selected objects onto a sheet or a roll.

use serde_json::{Value, json};
use vectorcraft_geom::Rect;

use super::{Skyline, pack};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 - 1e-9 && b.x0 < a.x1 - 1e-9 && a.y0 < b.y1 - 1e-9 && b.y0 < a.y1 - 1e-9
}

#[test]
fn boxes_pack_tallest_first_from_the_top_left() {
    // 100 wide: the 60 × 50 box goes first at the left and the 40 × 30 one beside it. The 50 × 20
    // one is too wide for the 40 pt beside the first box, so it goes below the first box.
    let at = pack(&fixed(&[(40.0, 30.0), (60.0, 50.0), (50.0, 20.0)]), 100.0, f64::INFINITY);
    assert_eq!(at, vec![Some((60.0, 0.0, false)), Some((0.0, 0.0, false)), Some((0.0, 50.0, false))]);
}

#[test]
fn a_box_that_does_not_fit_is_left_out() {
    let at = pack(&fixed(&[(80.0, 80.0), (80.0, 80.0), (120.0, 10.0)]), 100.0, 100.0);
    assert_eq!(at, vec![Some((0.0, 0.0, false)), None, None]);
}

/// Boxes that keep their orientation.
fn fixed(b: &[(f64, f64)]) -> Vec<super::Sizes> {
    b.iter().map(|&s| (s, None)).collect()
}

/// Boxes that may turn a quarter.
fn turning(b: &[(f64, f64)]) -> Vec<super::Sizes> {
    b.iter().map(|&(w, h)| ((w, h), Some((h, w)))).collect()
}

#[test]
fn a_box_turns_where_its_bottom_edge_ends_higher_or_only_turned_it_fits() {
    // A 20 × 90 box lies down as 90 × 20 at the top; a 120 × 10 box only fits the 100 pt strip
    // turned, as 10 × 120.
    let at = pack(&turning(&[(20.0, 90.0), (120.0, 10.0)]), 100.0, f64::INFINITY);
    assert_eq!(at, vec![Some((10.0, 0.0, true)), Some((0.0, 0.0, true))]);
    // Without turning, the 120 pt wide box is left out and the other stands up.
    let at = pack(&fixed(&[(20.0, 90.0), (120.0, 10.0)]), 100.0, f64::INFINITY);
    assert_eq!(at, vec![Some((0.0, 0.0, false)), None]);
    // A square box is never turned: turned, it is no better.
    let at = pack(&turning(&[(30.0, 30.0)]), 100.0, 100.0);
    assert_eq!(at, vec![Some((0.0, 0.0, false))]);
}

#[test]
fn the_skyline_fills_and_merges_spans() {
    let mut sky = Skyline::new(100.0, f64::INFINITY);
    sky.place(0.0, 0.0, 30.0, 10.0);
    sky.place(30.0, 0.0, 30.0, 10.0);
    assert_eq!(sky.spans.len(), 2, "two spans at the same depth are one: {:?}", sky.spans);
    assert_eq!(sky.find(50.0, 5.0), Some((0.0, 10.0)), "50 wide does not fit in the 40 left at the top");
    assert_eq!(sky.find(40.0, 5.0), Some((60.0, 0.0)));
}

#[test]
fn selected_objects_are_nested_on_the_artboard_as_one_undo_step() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..6).map(|i| rect(&mut s, 900.0 + 300.0 * i as f64, 700.0, 150.0, 100.0 + 10.0 * i as f64)).collect();
    select(&mut s, &ids);
    let n = steps(&s);
    let r = s.execute("addon.cutting.nest", &json!({"margin": 10, "spacing": 5, "usePreviewBounds": false})).unwrap();
    assert_eq!(steps(&s), n + 1, "one undo step");
    assert_eq!(r["placed"], json!(6));
    assert_eq!(r["unplaced"], json!([]));
    let boxes: Vec<Rect> = ids.iter().map(|id| bounds(&s, *id)).collect();
    for (i, a) in boxes.iter().enumerate() {
        assert!(a.x0 >= 10.0 - 1e-9 && a.y0 >= 10.0 - 1e-9 && a.x1 <= 790.0 + 1e-9 && a.y1 <= 590.0 + 1e-9, "{a:?} inside the margins");
        assert!((a.width(), a.height()) == (150.0, 100.0 + 10.0 * i as f64), "moved, not resized");
        for b in &boxes[i + 1..] {
            assert!(!overlap(a.inflate(2.5, 2.5), b.inflate(2.5, 2.5)), "{a:?} and {b:?} are 5 pt apart");
        }
    }
    // The tallest (150 pt) is at the top-left corner inside the margin, and five 150 pt wide
    // boxes with 5 pt between them fill the 780 pt inside the margins. The shortest goes 5 pt below
    // the 110 pt tall box at the right, where its bottom edge is highest.
    assert_eq!((boxes[5].x0, boxes[5].y0), (10.0, 10.0));
    assert_eq!(boxes.iter().filter(|b| b.y0 == 10.0).count(), 5);
    assert_eq!((boxes[0].x0, boxes[0].y0), (630.0, 125.0));
    assert_eq!(r["length"], json!(10.0 + 115.0 + 100.0 + 10.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(bounds(&s, ids[0]), Rect::new(900.0, 700.0, 1050.0, 800.0));
}

#[test]
fn objects_that_do_not_fit_go_beside_the_sheet() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 500.0, 400.0);
    let b = rect(&mut s, 1500.0, 0.0, 500.0, 400.0);
    let big = rect(&mut s, 2100.0, 0.0, 900.0, 100.0);
    select(&mut s, &[a, b, big]);
    let r = s.execute("addon.cutting.nest", &json!({"margin": 0, "spacing": 5, "usePreviewBounds": false})).unwrap();
    assert_eq!(r["placed"], json!(1));
    assert_eq!(r["unplaced"], json!([b.0, big.0]));
    assert_eq!(bounds(&s, a), Rect::new(0.0, 0.0, 500.0, 400.0));
    // A column 5 pt to the right of the 800 pt sheet, in stacking order, 5 pt apart.
    assert_eq!(bounds(&s, b), Rect::new(805.0, 0.0, 1305.0, 400.0));
    assert_eq!(bounds(&s, big), Rect::new(805.0, 405.0, 1705.0, 505.0));
}

#[test]
fn the_column_beside_the_sheet_is_in_stacking_order() {
    let mut s = session();
    let back = rect(&mut s, 900.0, 0.0, 900.0, 100.0);
    let front = rect(&mut s, 900.0, 200.0, 900.0, 50.0);
    let small = rect(&mut s, 900.0, 400.0, 50.0, 50.0);
    // Selected front first: the column still starts with the object at the back.
    select(&mut s, &[front, small, back]);
    let r = s.execute("addon.cutting.nest", &json!({"margin": 0, "spacing": 5, "usePreviewBounds": false})).unwrap();
    assert_eq!(r["unplaced"], json!([back.0, front.0]));
    assert_eq!(bounds(&s, back), Rect::new(805.0, 0.0, 1705.0, 100.0));
    assert_eq!(bounds(&s, front), Rect::new(805.0, 105.0, 1705.0, 155.0));
}

#[test]
fn objects_without_bounds_are_skipped_and_listed() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 50.0, 50.0);
    let layer = s.doc().unwrap().doc.parent_of(a);
    let empty = s
        .edit("empty group", |d, _| {
            let g = vectorcraft_doc::Node::new(d.alloc_id(), vectorcraft_doc::NodeKind::Group { children: vec![], clip: false });
            Ok(d.insert(layer, usize::MAX, g).unwrap())
        })
        .unwrap();
    select(&mut s, &[a, empty]);
    let r = s.execute("addon.cutting.nest", &json!({"margin": 0, "usePreviewBounds": false})).unwrap();
    assert_eq!((r["placed"].clone(), r["unplaced"].clone(), r["skipped"].clone()), (json!(1), json!([]), json!([empty.0])));
    assert_eq!(bounds(&s, a), Rect::new(0.0, 0.0, 50.0, 50.0));
}

#[test]
fn the_sheet_is_the_artboard_under_the_center_of_the_selection() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 300, "height": 300})).unwrap();
    let a = rect(&mut s, 1100.0, 100.0, 50.0, 50.0);
    let b = rect(&mut s, 1200.0, 200.0, 40.0, 40.0);
    select(&mut s, &[a, b]);
    s.execute("addon.cutting.nest", &json!({"margin": 0, "spacing": 0, "usePreviewBounds": false})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(1000.0, 0.0, 1050.0, 50.0), "on the second artboard, not the first");
    assert_eq!(bounds(&s, b), Rect::new(1050.0, 0.0, 1090.0, 40.0));
}

#[test]
fn rotate_turns_clockwise() {
    let mut s = session();
    // A 400 × 100 group with a 20 pt mark at its top-left corner. Turned clockwise on the page, the
    // top-left corner goes to the top-right.
    let long = rect(&mut s, 900.0, 0.0, 400.0, 100.0);
    let mark = rect(&mut s, 900.0, 0.0, 20.0, 20.0);
    select(&mut s, &[long, mark]);
    s.execute("object.group", &json!({})).unwrap();
    let r = s
        .execute("addon.cutting.nest", &json!({"width": 300, "height": 500, "margin": 0, "spacing": 0, "rotate": true, "usePreviewBounds": false}))
        .unwrap();
    assert_eq!(r["rotated"].as_array().map(Vec::len), Some(1));
    assert_eq!(bounds(&s, long), Rect::new(0.0, 0.0, 100.0, 400.0));
    assert_eq!(bounds(&s, mark), Rect::new(80.0, 0.0, 100.0, 20.0), "the mark is at the top-right");
}

#[test]
fn a_path_of_a_pixel_aligned_compound_path_is_not_turned() {
    let mut s = session();
    let outer = rect(&mut s, 900.0, 100.0, 500.0, 200.0);
    let hole = rect(&mut s, 950.0, 150.0, 400.0, 100.0);
    select(&mut s, &[outer, hole]);
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    select(&mut s, &[hole]);
    // The 400 pt wide path fits the 300 pt sheet only turned, and it keeps its orientation.
    let r =
        s.execute("addon.cutting.nest", &json!({"width": 300, "height": 500, "margin": 0, "spacing": 0, "rotate": true, "usePreviewBounds": false}));
    let e = r.unwrap_err().to_string();
    assert!(e.contains("no selected object fits"), "{e}");
}

/// [`Skyline::find`] as a search of every span and every span under the box.
fn find_by_search(sky: &Skyline, w: f64, h: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for (i, s) in sky.spans.iter().enumerate() {
        if s.x + w > sky.width + 1e-9 {
            break;
        }
        let y = sky.spans[i..].iter().take_while(|t| t.x == s.x || t.x < s.x + w - 1e-9).fold(0.0_f64, |m, t| m.max(t.y));
        if y + h > sky.height + 1e-9 {
            continue;
        }
        if best.is_none_or(|(bx, by)| y < by - 1e-9 || ((y - by).abs() <= 1e-9 && s.x < bx)) {
            best = Some((s.x, y));
        }
    }
    best
}

#[test]
fn find_matches_a_search_of_every_span() {
    // A fixed pseudo-random sequence of boxes, each placed where `find` puts it.
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n) as f64 + 1.0
    };
    let mut sky = Skyline::new(200.0, 400.0);
    for _ in 0..400 {
        let (w, h) = (next(60), next(30));
        let got = sky.find(w, h);
        assert_eq!(got, find_by_search(&sky, w, h), "{w} × {h} on {:?}", sky.spans);
        if let Some((x, y)) = got {
            sky.place(x, y, w, h);
        }
    }
}

#[test]
fn an_object_that_does_not_fit_is_not_covered_by_the_others() {
    let mut s = session();
    // The wide object sits on the sheet and does not fit; the small one must not land on it.
    let wide = rect(&mut s, 0.0, 0.0, 900.0, 100.0);
    let small = rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    select(&mut s, &[wide, small]);
    s.execute("addon.cutting.nest", &json!({"margin": 0, "spacing": 0, "usePreviewBounds": false})).unwrap();
    assert!(!overlap(bounds(&s, wide), bounds(&s, small)), "{:?} {:?}", bounds(&s, wide), bounds(&s, small));
}

#[test]
fn objects_on_the_pixel_grid_keep_the_margin_and_spacing() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 1100.0, 0.0, 100.0, 90.0);
    select(&mut s, &[a, b]);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    let on_grid = |r: Rect| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| (v * 2.0).fract() == 0.0);
    assert!(on_grid(bounds(&s, a)) && on_grid(bounds(&s, b)));
    s.execute("addon.cutting.nest", &json!({"margin": 0.6, "spacing": 0.6, "usePreviewBounds": false})).unwrap();
    let (ba, bb) = (bounds(&s, a), bounds(&s, b));
    assert!(on_grid(ba) && on_grid(bb), "still on the grid: {ba:?} {bb:?}");
    assert!(ba.x0 >= 0.6 && ba.y0 >= 0.6 && bb.y0 >= 0.6, "inside the margin: {ba:?} {bb:?}");
    assert!(bb.x0 >= ba.x1 + 0.6, "0.6 pt apart: {ba:?} {bb:?}");
    assert!(ba.x1.max(bb.x1) <= 800.0 - 0.6, "inside the margin: {ba:?} {bb:?}");
}

#[test]
fn preview_bounds_take_in_a_drop_shadow() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 1100.0, 0.0, 100.0, 100.0);
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [a.0], "params": {"x": 20, "y": 0, "blur": 0}})).unwrap();
    select(&mut s, &[a, b]);
    s.execute("addon.cutting.nest", &json!({"margin": 0, "spacing": 0})).unwrap();
    let (ba, bb) = (bounds(&s, a), bounds(&s, b));
    let shadow = vectorcraft_render::drawn_bounds(&s.doc().unwrap().doc, s.doc().unwrap().doc.node(a).unwrap()).unwrap();
    assert!(shadow.x1 >= ba.x1 + 20.0 - 1e-9, "the shadow reaches 20 pt right: {shadow:?}");
    let shadow_b = vectorcraft_render::drawn_bounds(&s.doc().unwrap().doc, s.doc().unwrap().doc.node(b).unwrap()).unwrap_or(bb);
    assert!(!overlap(shadow, shadow_b.union(bb)), "the shadow {shadow:?} does not reach {bb:?}");
}

#[test]
fn a_roll_has_no_end() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..5).map(|i| rect(&mut s, 900.0, 300.0 * i as f64, 500.0, 400.0)).collect();
    select(&mut s, &ids);
    let r = s.execute("addon.cutting.nest", &json!({"roll": true, "margin": 0, "spacing": 10, "usePreviewBounds": false})).unwrap();
    assert_eq!((r["placed"].clone(), r["unplaced"].clone()), (json!(5), json!([])));
    let mut tops: Vec<f64> = ids.iter().map(|id| bounds(&s, *id).y0).collect();
    tops.sort_by(f64::total_cmp);
    assert_eq!(tops, vec![0.0, 410.0, 820.0, 1230.0, 1640.0], "one 500 pt wide shape per row on the 800 pt roll");
    assert_eq!(r["length"], json!(2040.0));
}

#[test]
fn width_and_height_set_the_sheet_from_the_artboard_corner() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..3).map(|i| rect(&mut s, 50.0 * i as f64, 300.0, 40.0, 40.0)).collect();
    select(&mut s, &ids);
    let r = s.execute("addon.cutting.nest", &json!({"width": "1 in", "height": 200, "margin": 0, "spacing": 0, "usePreviewBounds": false})).unwrap();
    assert_eq!(r["placed"], json!(3));
    for id in &ids {
        let b = bounds(&s, *id);
        assert!(b.x0 == 0.0 && b.x1 <= 72.0, "one 40 pt shape per row on a 72 pt sheet: {b:?}");
    }
}

#[test]
fn preview_bounds_keep_strokes_apart() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 1100.0, 0.0, 100.0, 100.0);
    select(&mut s, &[a, b]);
    s.execute("stroke.set", &json!({"weight": 10})).unwrap();
    s.execute("addon.cutting.nest", &json!({"margin": 0, "spacing": 0})).unwrap();
    let (ba, bb) = (bounds(&s, a), bounds(&s, b));
    assert_eq!((ba.x0, ba.y0), (5.0, 5.0), "the stroke's outer half is inside the sheet");
    assert_eq!((bb.x0 - ba.x1), 10.0, "the strokes touch and do not overlap");
}

#[test]
fn a_selection_already_nested_adds_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 18.0, 18.0, 100.0, 100.0);
    select(&mut s, &[a]);
    let n = steps(&s);
    let r = s.execute("addon.cutting.nest", &json!({"usePreviewBounds": false})).unwrap();
    assert_eq!(r["placed"], json!(1));
    assert_eq!(steps(&s), n, "nothing moved");
}

#[test]
fn bad_params_are_refused_and_change_nothing() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 100.0, 100.0);
    select(&mut s, &[a]);
    let n = steps(&s);
    let bad: [(Value, &str); 10] = [
        (json!({"rotate": 1}), "`rotate` must be true or false"),
        (json!({"width": -1}), "`width` must be from 0"),
        (json!({"height": 2e6}), "`height` must be from 0"),
        (json!({"margin": "x"}), "`margin` must be a number"),
        (json!({"spacing": 1e6}), "`spacing` must be from 0"),
        (json!({"roll": "yes"}), "`roll` must be true or false"),
        (json!({"usePreviewBounds": 1}), "`usePreviewBounds` must be true or false"),
        (json!({"artboard": 3}), "there is no artboard 3"),
        (json!({"artboard": -1}), "`artboard` must be"),
        (json!({"margin": 400}), "the margin leaves no room"),
    ];
    for (p, want) in bad {
        let e = s.execute("addon.cutting.nest", &p).expect_err(&p.to_string()).to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
    let e = s.execute("addon.cutting.nest", &json!({"width": 50, "margin": 0})).unwrap_err().to_string();
    assert!(e.contains("no selected object fits"), "{e}");
    let e = s.execute("addon.cutting.nest", &json!({"patterns": "no"})).unwrap_err().to_string();
    assert!(e.contains("`patterns` must be true or false"), "{e}");
    assert_eq!(steps(&s), n, "a refused run changes nothing");
    assert_eq!(bounds(&s, a), Rect::new(900.0, 0.0, 1000.0, 100.0));
}

#[test]
fn the_named_artboard_is_the_sheet() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 300, "height": 300})).unwrap();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    select(&mut s, &[a]);
    s.execute("addon.cutting.nest", &json!({"artboard": 1, "margin": 0, "usePreviewBounds": false})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(1000.0, 0.0, 1050.0, 50.0));
}

#[test]
fn rotate_turns_objects_a_quarter_to_fit_the_sheet() {
    let mut s = session();
    // On a 300 × 500 sheet with no margin, a 400 × 100 object only fits standing up.
    let long = rect(&mut s, 900.0, 0.0, 400.0, 100.0);
    let small = rect(&mut s, 900.0, 300.0, 50.0, 40.0);
    select(&mut s, &[long, small]);
    let p = json!({"width": 300, "height": 500, "margin": 0, "spacing": 0, "usePreviewBounds": false});
    let r = s.execute("addon.cutting.nest", &p).unwrap();
    assert_eq!((r["unplaced"].clone(), r["rotated"].clone()), (json!([long.0]), json!([])), "without rotate it does not fit");
    s.execute("edit.undo", &json!({})).unwrap();
    let mut p = p;
    p["rotate"] = json!(true);
    let n = steps(&s);
    let r = s.execute("addon.cutting.nest", &p).unwrap();
    assert_eq!(steps(&s), n + 1, "one undo step");
    assert_eq!((r["placed"].clone(), r["unplaced"].clone(), r["rotated"].clone()), (json!(2), json!([]), json!([long.0])));
    assert_eq!(bounds(&s, long), Rect::new(0.0, 0.0, 100.0, 400.0), "turned and at the corner");
    // The 50 × 40 object stays as it is: turned, its bottom edge would be lower.
    assert_eq!(bounds(&s, small), Rect::new(100.0, 0.0, 150.0, 40.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(bounds(&s, long), Rect::new(900.0, 0.0, 1300.0, 100.0), "undo turns it back");
}

#[test]
fn rotate_measures_the_turned_preview_bounds() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 400.0, 100.0);
    select(&mut s, &[a]);
    s.execute("stroke.set", &json!({"weight": 10})).unwrap();
    let r = s.execute("addon.cutting.nest", &json!({"width": 300, "height": 500, "margin": 0, "spacing": 0, "rotate": true})).unwrap();
    assert_eq!(r["rotated"], json!([a.0]));
    assert_eq!(bounds(&s, a), Rect::new(5.0, 5.0, 105.0, 405.0), "the turned stroke's outer half is inside the sheet");
}

#[test]
fn a_compound_paths_path_selected_alone_moves_alone() {
    let mut s = session();
    let outer = rect(&mut s, 900.0, 100.0, 40.0, 40.0);
    let hole = rect(&mut s, 910.0, 110.0, 10.0, 10.0);
    select(&mut s, &[outer, hole]);
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    select(&mut s, &[hole]);
    s.execute("addon.cutting.nest", &json!({"margin": 0, "usePreviewBounds": false})).unwrap();
    assert_eq!(bounds(&s, hole), Rect::new(0.0, 0.0, 10.0, 10.0), "the path moved");
    assert_eq!(bounds(&s, outer), Rect::new(900.0, 100.0, 940.0, 140.0), "the rest of the compound path stayed");
}

#[test]
fn a_group_is_nested_as_one_object() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 50.0, 50.0);
    let b = rect(&mut s, 1000.0, 100.0, 50.0, 50.0);
    select(&mut s, &[a, b]);
    s.execute("object.group", &json!({})).unwrap();
    s.execute("addon.cutting.nest", &json!({"margin": 0, "usePreviewBounds": false})).unwrap();
    assert_eq!((bounds(&s, a), bounds(&s, b)), (Rect::new(0.0, 0.0, 50.0, 50.0), Rect::new(100.0, 100.0, 150.0, 150.0)));
}
