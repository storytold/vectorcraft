//! Align and Distribute for artboards.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::Session;
use crate::tooling::ViewInfo;

/// A document with three artboards, each with a 10 pt square 5 pt inside its top-left corner:
/// 0 at (0, 0) 100 × 100, 1 at (150, 40) 60 × 80 and 2 at (400, 10) 100 × 50 → the squares' ids.
fn three_boards() -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("artboard.new", &json!({"x": 150, "y": 40, "width": 60, "height": 80})).unwrap();
    s.execute("artboard.new", &json!({"x": 400, "y": 10, "width": 100, "height": 50})).unwrap();
    let mut sq = |x: u64, y: u64| s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let ids = [sq(5, 5), sq(155, 45), sq(405, 15)];
    s.execute("select.none", &json!({})).unwrap();
    (s, ids)
}

/// The top-left corner of each artboard.
fn corners(s: &Session) -> Vec<(f64, f64)> {
    s.doc().unwrap().doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0)).collect()
}

/// The top-left corner of object `id`.
fn corner(s: &Session, id: u64) -> (f64, f64) {
    let b = s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
    (b.x0, b.y0)
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

const ALL: [u64; 3] = [0, 1, 2];

/// Align Objects lines the artboards up on the bounds of all of them, each with the art inside
/// it, in one undo step.
#[test]
fn align_moves_the_artboards_to_their_common_edge_with_their_art() {
    let (mut s, [a, b, c]) = three_boards();
    let before = corners(&s);
    let steps = undo_steps(&s);
    let out = s.execute("artboard.align", &json!({"indices": ALL, "vertical": "top"})).unwrap();
    assert_eq!(out, json!({"moved": [1, 2]}));
    assert_eq!(corners(&s), [(0.0, 0.0), (150.0, 0.0), (400.0, 0.0)]);
    assert_eq!([corner(&s, a), corner(&s, b), corner(&s, c)], [(5.0, 5.0), (155.0, 5.0), (405.0, 5.0)], "each square stays on its artboard");
    assert_eq!(undo_steps(&s), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((corners(&s), corner(&s, b)), (before, (155.0, 45.0)), "one undo step");
    s.execute("artboard.align", &json!({"indices": ALL, "horizontal": "right", "vertical": "bottom"})).unwrap();
    assert_eq!(corners(&s), [(400.0, 20.0), (440.0, 40.0), (400.0, 70.0)], "the right edges at 500 and the bottom edges at 120");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("artboard.align", &json!({"indices": ALL, "horizontal": "center", "moveArt": false})).unwrap();
    assert_eq!(corners(&s), [(200.0, 0.0), (220.0, 40.0), (200.0, 10.0)], "the centers at 250");
    assert_eq!(corner(&s, a), (5.0, 5.0), "moveArt false leaves the art");
}

/// Align to an artboard: that artboard stays where it is and the others line up on it. It need not
/// be one of the artboards aligned.
#[test]
fn align_to_an_artboard_keeps_that_artboard_in_place() {
    let (mut s, _) = three_boards();
    let out = s.execute("artboard.align", &json!({"indices": ALL, "horizontal": "left", "to": "artboard", "artboard": 2})).unwrap();
    assert_eq!(out, json!({"moved": [0, 1]}));
    assert_eq!(corners(&s), [(400.0, 0.0), (400.0, 40.0), (400.0, 10.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("artboard.align", &json!({"index": 0, "vertical": "center", "to": "artboard", "artboard": 1})).unwrap();
    assert_eq!(corners(&s), [(0.0, 30.0), (150.0, 40.0), (400.0, 10.0)], "artboard 1's center is at y 80");
}

/// With no artboard named, the commands act on the artboards the Artboard tool has selected and
/// align to its active one; the journal records both for a replay, and an action leaves them out.
#[test]
fn the_artboard_tools_selection_is_the_default_and_is_recorded() {
    let (mut s, [_, _, c]) = three_boards();
    s.select_tool("artboard", ViewInfo::default()).unwrap();
    s.set_tool_option("selected", &json!([1, 2]));
    let start = s.journal.len();
    assert_eq!(s.execute("artboard.align", &json!({"horizontal": "left"})).unwrap(), json!({"moved": [2]}));
    assert_eq!(corners(&s), [(0.0, 0.0), (150.0, 40.0), (150.0, 10.0)]);
    assert_eq!(corner(&s, c), (155.0, 15.0), "the tool's Move Artwork with Artboard option is on");
    let recorded = json!({"horizontal": "left", "selectedArtboards": [1, 2], "moveArt": true, "lockedAndHidden": false});
    assert_eq!(s.journal.last().unwrap(), &("artboard.align".to_string(), recorded));
    assert_eq!(
        s.journal_for_action(start),
        [("artboard.align".to_string(), json!({"horizontal": "left", "moveArt": true, "lockedAndHidden": false}))]
    );
    s.execute("artboard.align", &json!({"vertical": "top", "to": "artboard"})).unwrap();
    assert_eq!(corners(&s)[2], (150.0, 40.0), "artboard 2 lines up on the active artboard, 1");
    assert_eq!(s.journal.last().unwrap().1["activeArtboard"], json!(1));
    assert_eq!(s.journal_for_action(start)[1].1.get("activeArtboard"), None, "an action aligns to the artboard active when it plays");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("artboard.align", &json!({"vertical": "top", "to": "artboard", "activeArtboard": 0})).unwrap();
    assert_eq!(corners(&s)[1..], [(150.0, 0.0), (150.0, 0.0)], "a replay aligns to the artboard it recorded, 0");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("artboard.align", &json!({"vertical": "bottom", "to": "artboard", "artboard": 0, "activeArtboard": 2})).unwrap();
    assert_eq!(corners(&s)[1..], [(150.0, 20.0), (150.0, 50.0)], "`artboard` goes ahead of the recorded one");
    let action = s.journal_for_action(start);
    let entry = &action.last().unwrap().1;
    assert_eq!((entry.get("artboard"), entry.get("activeArtboard")), (Some(&json!(0)), None), "an action keeps the artboard a call named");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("artboard.align", &json!({"vertical": "top", "to": "artboard"})).unwrap();
    s.set_tool_option("moveArt", &json!(false));
    s.execute("artboard.align", &json!({"horizontal": "right"})).unwrap();
    assert_eq!((corners(&s)[1], corner(&s, c)), ((190.0, 40.0), (155.0, 45.0)), "with the option off the art stays");
}

/// A command that moves no artboard leaves the document and its undo history as they are.
#[test]
fn nothing_to_move_records_no_undo_step() {
    let (mut s, _) = three_boards();
    s.execute("artboard.align", &json!({"indices": ALL, "vertical": "top"})).unwrap();
    let (steps, before) = (undo_steps(&s), corners(&s));
    for (id, p) in [
        ("artboard.align", json!({"indices": ALL, "vertical": "top"})),
        ("artboard.align", json!({"index": 1, "horizontal": "left", "vertical": "bottom"})),
        ("artboard.align", json!({"index": 1, "horizontal": "center", "to": "artboard", "artboard": 1})),
        ("artboard.distribute", json!({"indices": [0, 2], "horizontal": "center"})),
        ("artboard.distribute", json!({"index": 2, "vertical": "top", "to": "artboard", "artboard": 2, "spacing": 30})),
        ("artboard.distributeSpacing", json!({"indices": [0, 1], "axis": "horizontal"})),
        ("artboard.distributeSpacing", json!({"indices": [1], "axis": "vertical"})),
    ] {
        assert_eq!(s.execute(id, &p).unwrap(), json!({"moved": []}), "{id} {p}");
        assert_eq!((undo_steps(&s), corners(&s)), (steps, before.clone()), "{id} {p}");
    }
}

/// A locked artboard stays where it is and still counts in the bounds and in the order. When every
/// artboard named is locked, the command fails.
#[test]
fn a_locked_artboard_stays_where_it_is() {
    let (mut s, _) = three_boards();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    assert_eq!(s.execute("artboard.align", &json!({"indices": ALL, "vertical": "bottom"})).unwrap(), json!({"moved": [0, 2]}));
    assert_eq!(corners(&s), [(0.0, 20.0), (150.0, 40.0), (400.0, 70.0)], "the bottom edges at 120, the locked artboard's");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("artboard.distributeSpacing", &json!({"indices": ALL})).unwrap(), json!({"moved": []}), "the middle artboard is locked");
    let out =
        s.execute("artboard.distribute", &json!({"indices": ALL, "horizontal": "left", "to": "artboard", "artboard": 0, "spacing": 100})).unwrap();
    assert_eq!((out, corners(&s)), (json!({"moved": [2]}), vec![(0.0, 0.0), (150.0, 40.0), (200.0, 10.0)]), "the third left edge goes to 200");
    s.execute("edit.undo", &json!({})).unwrap();
    let (steps, before) = (undo_steps(&s), corners(&s));
    for (id, p) in [
        ("artboard.align", json!({"index": 1, "horizontal": "left", "to": "artboard", "artboard": 0})),
        ("artboard.distribute", json!({"index": 1, "horizontal": "left"})),
        ("artboard.distributeSpacing", json!({"indices": [1]})),
    ] {
        assert_eq!(s.execute(id, &p).unwrap_err().to_string(), "the artboard is locked", "{id}");
        assert_eq!((undo_steps(&s), corners(&s)), (steps, before.clone()), "{id}");
    }
}

/// Distribute Objects spaces the named edges or centers evenly between the outermost artboards;
/// with a spacing and an artboard to align to, it places them that far apart from that artboard.
#[test]
fn distribute_spaces_the_edges_or_centers() {
    let (mut s, [_, b, _]) = three_boards();
    assert_eq!(s.execute("artboard.distribute", &json!({"indices": ALL, "horizontal": "center"})).unwrap(), json!({"moved": [1]}));
    assert_eq!(corners(&s), [(0.0, 0.0), (220.0, 40.0), (400.0, 10.0)], "the centers at 50, 250 and 450");
    assert_eq!(corner(&s, b), (225.0, 45.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("artboard.distribute", &json!({"indices": ALL, "vertical": "bottom"})).unwrap();
    assert_eq!(corners(&s), [(0.0, -10.0), (150.0, 40.0), (400.0, 10.0)], "the bottom edges at 60, 90 and 120");
    s.execute("edit.undo", &json!({})).unwrap();
    let p = json!({"indices": ALL, "horizontal": "left", "to": "artboard", "artboard": 1, "spacing": 100});
    assert_eq!(s.execute("artboard.distribute", &p).unwrap(), json!({"moved": [0, 2]}));
    assert_eq!(corners(&s), [(50.0, 0.0), (150.0, 40.0), (250.0, 10.0)], "the left edges 100 apart, artboard 1 in place");
    s.execute("edit.undo", &json!({})).unwrap();
    let two = json!({"indices": [0, 2], "horizontal": "right", "to": "artboard", "artboard": 2, "spacing": 150});
    assert_eq!(s.execute("artboard.distribute", &two).unwrap(), json!({"moved": [0]}));
    assert_eq!(corners(&s)[0], (250.0, 0.0), "two artboards are enough with a spacing: the right edges at 350 and 500");
    s.execute("edit.undo", &json!({})).unwrap();
    let held = json!({"indices": ALL, "horizontal": "center", "to": "artboard", "artboard": 1});
    assert_eq!(s.execute("artboard.distribute", &held).unwrap(), json!({"moved": [0, 2]}));
    assert_eq!(corners(&s), [(-70.0, 0.0), (150.0, 40.0), (330.0, 10.0)], "the centers 200 apart, artboard 1 in place");
}

/// Distribute Spacing makes the gaps between the artboards equal; with a spacing and an artboard
/// to align to, the gaps are that wide and that artboard stays.
#[test]
fn distribute_spacing_makes_the_gaps_equal() {
    let (mut s, _) = three_boards();
    assert_eq!(s.execute("artboard.distributeSpacing", &json!({"indices": ALL})).unwrap(), json!({"moved": [1]}));
    assert_eq!(corners(&s), [(0.0, 0.0), (220.0, 40.0), (400.0, 10.0)], "gaps of 120 between 0 and 500");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("artboard.distributeSpacing", &json!({"indices": ALL, "axis": "vertical"})).unwrap();
    assert_eq!(corners(&s), [(0.0, 0.0), (150.0, 40.0), (400.0, 45.0)], "top to bottom: artboards 0, 2 and 1, overlapping by 55");
    s.execute("edit.undo", &json!({})).unwrap();
    let p = json!({"indices": ALL, "axis": "horizontal", "to": "artboard", "artboard": 1, "spacing": 20});
    assert_eq!(s.execute("artboard.distributeSpacing", &p).unwrap(), json!({"moved": [0, 2]}));
    assert_eq!(corners(&s), [(30.0, 0.0), (150.0, 40.0), (230.0, 10.0)], "gaps of 20 on both sides of artboard 1");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("artboard.distributeSpacing", &json!({"indices": [0, 2], "to": "artboard", "artboard": 0, "spacing": -25})).unwrap();
    assert_eq!(corners(&s)[2], (75.0, 10.0), "a negative spacing overlaps them");
    s.execute("edit.undo", &json!({})).unwrap();
    let held = json!({"indices": ALL, "to": "artboard", "artboard": 1});
    assert_eq!(s.execute("artboard.distributeSpacing", &held).unwrap(), json!({"moved": [0, 2]}));
    assert_eq!(corners(&s), [(-70.0, 0.0), (150.0, 40.0), (330.0, 10.0)], "gaps of 120, artboard 1 in place");
}

/// A value outside what a param takes is an error, and nothing changes.
#[test]
fn bad_params_are_errors() {
    let (mut s, _) = three_boards();
    let (steps, before) = (undo_steps(&s), corners(&s));
    let with = |extra: Value| {
        let mut p = json!({"indices": ALL, "horizontal": "left"});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    let shared = [
        json!({"horizontal": "middle"}),
        json!({"horizontal": 1}),
        json!({"to": "key"}),
        json!({"to": "artboard", "artboard": 9}),
        json!({"artboard": "1"}),
        json!({"artboard": -1}),
        json!({"moveArt": "yes"}),
        json!({"patterns": 1}),
        json!({"lockedAndHidden": "no"}),
        json!({"indices": [9]}),
        json!({"indices": []}),
        json!({"indices": "0"}),
    ];
    for id in ["artboard.align", "artboard.distribute", "artboard.distributeSpacing"] {
        let axis = if id == "artboard.distributeSpacing" { json!({"horizontal": null, "axis": "horizontal"}) } else { json!({}) };
        for extra in &shared {
            let mut p = with(axis.clone());
            for (k, v) in extra.as_object().unwrap() {
                p[k] = v.clone();
            }
            if id == "artboard.distributeSpacing" && extra.get("horizontal").is_some() {
                p = json!({"indices": ALL, "axis": extra["horizontal"]});
            }
            assert!(s.execute(id, &p).is_err(), "{id} {p}");
        }
    }
    for (id, p) in [
        ("artboard.align", json!({"indices": ALL})),
        ("artboard.align", json!({"indices": ALL, "vertical": "middle"})),
        ("artboard.distribute", json!({"indices": ALL})),
        ("artboard.distribute", with(json!({"spacing": "10"}))),
        ("artboard.distribute", with(json!({"spacing": true}))),
        ("artboard.distribute", json!({"indices": [0, 1], "horizontal": "left", "to": "artboard", "artboard": 2, "spacing": 10})),
        ("artboard.distributeSpacing", json!({"indices": ALL, "spacing": [10]})),
        ("artboard.distributeSpacing", json!({"indices": [0, 1], "to": "artboard", "artboard": 2, "spacing": 10})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    // Spacings that would put an artboard past the canvas, or overflow to infinity.
    for (id, p) in [
        ("artboard.distribute", with(json!({"to": "artboard", "artboard": 0, "spacing": 1e9}))),
        ("artboard.distribute", with(json!({"to": "artboard", "artboard": 1, "spacing": 1e308}))),
        ("artboard.distributeSpacing", json!({"indices": ALL, "to": "artboard", "artboard": 0, "spacing": 1e9})),
        ("artboard.distributeSpacing", json!({"indices": ALL, "to": "artboard", "artboard": 1, "spacing": 1e308})),
    ] {
        let e = s.execute(id, &p).unwrap_err().to_string();
        assert!(e.contains("exceed the canvas"), "{id} {p}: {e}");
    }
    assert_eq!((undo_steps(&s), corners(&s)), (steps, before));
}

/// Art inside two overlapping artboards moves with the one first in artboard order, as with the
/// other artboard edits, even when Distribute spaces the other one first: artboard 2 at x 140 is
/// left of artboard 1 at x 200, and the square inside both goes with artboard 1.
#[test]
fn art_on_overlapping_artboards_moves_with_the_first_by_index() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("artboard.new", &json!({"x": 200, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("artboard.new", &json!({"x": 140, "y": 0, "width": 100, "height": 100})).unwrap();
    let sq = s.execute("shape.rectangle", &json!({"x": 210, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let p = json!({"indices": ALL, "to": "artboard", "artboard": 0, "spacing": 10});
    assert_eq!(s.execute("artboard.distributeSpacing", &p).unwrap(), json!({"moved": [1, 2]}));
    assert_eq!(corners(&s), [(0.0, 0.0), (220.0, 0.0), (110.0, 0.0)]);
    assert_eq!(corner(&s, sq), (230.0, 10.0), "with artboard 1, 20 pt to the right");
}

/// Art inside an artboard that stays and a later one that moves stays with the first: artboard 2
/// overlaps artboard 0, and aligning it to artboard 0 leaves the square inside both in place.
#[test]
fn art_shared_with_an_artboard_that_stays_stays() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("artboard.new", &json!({"x": 200, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("artboard.new", &json!({"x": 60, "y": 0, "width": 100, "height": 100})).unwrap();
    let sq = s.execute("shape.rectangle", &json!({"x": 70, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let p = json!({"indices": [0, 2], "horizontal": "left", "to": "artboard", "artboard": 0});
    assert_eq!(s.execute("artboard.align", &p).unwrap(), json!({"moved": [2]}));
    assert_eq!(corners(&s)[2], (0.0, 0.0));
    assert_eq!(corner(&s, sq), (70.0, 10.0), "with artboard 0, which stays");
}

/// An artboard's own guides move with it.
#[test]
fn an_artboards_guides_move_with_it() {
    let (mut s, _) = three_boards();
    s.execute("guide.add", &json!({"vertical": true, "pos": 160, "artboard": 1})).unwrap();
    s.execute("artboard.align", &json!({"indices": [0, 1], "horizontal": "left"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let positions: Vec<f64> = d.ruler_guides().iter().map(|g| g.pos).collect();
    assert_eq!(positions, [10.0], "the guide stays 10 pt inside artboard 1");
}
