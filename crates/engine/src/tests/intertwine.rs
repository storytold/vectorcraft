//! Object › Intertwine (#1763), as a tutorial does it: two thick rings that overlap twice are
//! made into an Intertwine group, then one overlap is clicked so the rings pass over and under
//! each other. Driven through `Session::execute`; the art is checked on screen, in the saved file
//! and in the SVG and PDF output.

use serde_json::{Value, json};
use vectorcraft_render::Rendered;

use crate::tests_clip::{hit, id_of, render, select, session};
use crate::*;

/// Ring A (red, centre 35,50) and ring B (blue, centre 65,50), radius 25, 8 pt strokes, no fill:
/// B is in front. The rings cross at (50, 30) and (50, 70).
fn rings(s: &mut Session) -> (NodeId, NodeId) {
    let ring = |s: &mut Session, cx: f64, color: &str| {
        let id = id_of(&s.execute("shape.ellipse", &json!({"x": cx - 25.0, "y": 25, "width": 50, "height": 50})).unwrap());
        s.execute("paint.setFill", &json!({"ids": [id.0], "none": true})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [id.0], "color": color})).unwrap();
        s.execute("stroke.set", &json!({"ids": [id.0], "weight": 8})).unwrap();
        id
    };
    let a = ring(s, 35.0, "#ff0000");
    let b = ring(s, 65.0, "#0000ff");
    (a, b)
}

/// The rings made into an Intertwine group: (group, a, b).
fn intertwined(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let (a, b) = rings(s);
    select(s, &[a, b]);
    let g = id_of(&s.execute("object.intertwine.make", &json!({})).unwrap());
    (g, a, b)
}

fn red(r: &Rendered, x: u32, y: u32) -> bool {
    let p = r.pixel(x, y);
    p[3] > 200 && p[0] > 200 && p[2] < 60
}

fn blue(r: &Rendered, x: u32, y: u32) -> bool {
    let p = r.pixel(x, y);
    p[3] > 200 && p[2] > 200 && p[0] < 60
}

fn arrange(s: &mut Session, p: Value) -> Value {
    s.execute("object.intertwine.arrange", &p).unwrap()
}

fn area(s: &Session, id: NodeId) -> Option<vectorcraft_geom::PathData> {
    s.doc().unwrap().doc.node(id).unwrap().extras().intertwine.as_deref().cloned()
}

#[test]
fn make_puts_the_objects_in_an_intertwine_group_drawn_as_before() {
    let mut s = session();
    let (g, a, b) = intertwined(&mut s);
    let d = &s.doc().unwrap().doc;
    let n = d.node(g).unwrap();
    assert!(matches!(n.kind, NodeKind::Intertwine { .. }));
    assert_eq!(n.kind_label(), "Intertwine");
    assert_eq!(n.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [a, b], "the stacking order is kept");
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [g]);
    let r = render(&s);
    assert!(blue(&r, 50, 30) && blue(&r, 50, 70), "B is in front at both crossings until an area is arranged");
}

#[test]
fn clicking_an_overlap_brings_the_object_below_to_the_front_there_only() {
    let mut s = session();
    let (_, a, b) = intertwined(&mut s);
    let v = arrange(&mut s, json!({"at": [50, 30]}));
    assert_eq!(v["count"], 1);
    assert_eq!(v["top"], json!([a.0]));
    let r = render(&s);
    assert!(red(&r, 50, 30), "A passes over B at the top crossing");
    assert!(blue(&r, 50, 70), "and under it at the bottom one");
    // Elsewhere each ring looks as it did.
    assert!(red(&r, 10, 50) && blue(&r, 90, 50));
    assert!(area(&s, a).is_some() && area(&s, b).is_none());
    // A click there picks the ring drawn on top.
    assert_eq!(hit(&s, 50.0, 30.0), Some(a));
    assert_eq!(hit(&s, 50.0, 70.0), Some(b));
}

#[test]
fn clicking_the_same_overlap_again_swaps_back() {
    let mut s = session();
    let (_, a, b) = intertwined(&mut s);
    arrange(&mut s, json!({"at": [50, 30]}));
    let v = arrange(&mut s, json!({"at": [50, 30]}));
    assert_eq!(v["top"], json!([b.0]));
    assert!(area(&s, a).is_none() && area(&s, b).is_none(), "no area is left over");
    assert!(blue(&render(&s), 50, 30));
}

#[test]
fn a_path_drawn_around_overlaps_arranges_every_overlap_inside_it() {
    let mut s = session();
    let (_, a, _) = intertwined(&mut s);
    // Around the bottom crossing only.
    let v = arrange(&mut s, json!({"area": [[40, 60], [60, 60], [60, 80], [40, 80]]}));
    assert_eq!(v["count"], 1);
    let r = render(&s);
    assert!(blue(&r, 50, 30) && red(&r, 50, 70));
    // Around both: each one swaps.
    let v = arrange(&mut s, json!({"area": [[40, 20], [60, 20], [60, 80], [40, 80]]}));
    assert_eq!(v["count"], 2);
    let r = render(&s);
    assert!(red(&r, 50, 30) && blue(&r, 50, 70));
    assert!(area(&s, a).is_some());
}

#[test]
fn front_and_back_choose_the_member_where_three_objects_overlap() {
    let mut s = session();
    let (a, b) = rings(&mut s);
    // A green bar across the top crossing, in front of both rings.
    let c = id_of(&s.execute("shape.rectangle", &json!({"x": 46, "y": 20, "width": 8, "height": 20})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [c.0], "color": "#00ff00"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [c.0], "none": true})).unwrap();
    select(&mut s, &[a, b, c]);
    s.execute("object.intertwine.make", &json!({})).unwrap();
    let green = |r: &Rendered| {
        let p = r.pixel(50, 30);
        p[1] > 200 && p[0] < 60 && p[2] < 60
    };
    assert!(green(&render(&s)));
    arrange(&mut s, json!({"at": [50, 30], "order": "front", "member": a.0}));
    assert!(red(&render(&s), 50, 30), "A is brought to the front of all three");
    arrange(&mut s, json!({"at": [50, 30], "order": "back", "member": a.0}));
    assert!(green(&render(&s)), "sent to the back, the others are over it again in their own order");
    arrange(&mut s, json!({"at": [50, 30], "order": "back"}));
    assert!(blue(&render(&s), 50, 30), "back without a member sends the top one (C) back: B is on top");
    let e = s.execute("object.intertwine.arrange", &json!({"at": [50, 30], "order": "front", "member": 999})).unwrap_err();
    assert!(e.to_string().contains("999"), "{e}");
}

#[test]
fn release_gives_the_objects_back_as_they_were() {
    let mut s = session();
    let (g, a, b) = intertwined(&mut s);
    arrange(&mut s, json!({"at": [50, 30]}));
    let v = s.execute("object.intertwine.release", &json!({})).unwrap();
    assert_eq!(v["ids"], json!([a.0, b.0]));
    let d = &s.doc().unwrap().doc;
    assert!(d.node(g).is_none());
    assert!(d.node(a).unwrap().extras().intertwine.is_none());
    assert!(blue(&render(&s), 50, 30));
    // Undo brings the group back, arranged.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(red(&render(&s), 50, 30));
}

/// #2540: Object › Expand turns the group into a plain group of what it draws (the rings, then the
/// part of A drawn over B as a clip group) in one undo step, looking the same; Ungroup then takes
/// it apart.
#[test]
fn expand_turns_the_group_into_plain_art_that_ungroups() {
    let mut s = session();
    let (g, a, b) = intertwined(&mut s);
    arrange(&mut s, json!({"at": [50, 30]}));
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(true));
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let n = s.doc().unwrap().doc.node(g).unwrap().clone();
    let NodeKind::Group { children, clip: false } = &n.kind else { panic!("a group: {}", n.kind_label()) };
    assert_eq!(children.len(), 3, "the rings, then the part of A drawn over B");
    assert_eq!((children[0].id, children[1].id), (a, b), "the rings keep their ids and order");
    assert!(matches!(children[2].kind, NodeKind::Group { clip: true, .. }));
    assert!(children.iter().all(|c| c.extras().intertwine.is_none()), "no area is left over");
    let mut ids = vec![];
    n.walk(&mut |c| ids.push(c.id));
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert!(unique.len() == ids.len() && !ids.contains(&NodeId(0)), "the generated art has ids of its own: {ids:?}");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [g]);
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(false), "nothing live is left");
    let r = render(&s);
    assert!(red(&r, 50, 30) && blue(&r, 50, 70), "it looks as it did");
    // Plain art: Ungroup takes it apart.
    s.execute("object.ungroup", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(g).is_none());
    assert_eq!(s.doc().unwrap().selection.objects.len(), 3);
    let r = render(&s);
    assert!(red(&r, 50, 30) && blue(&r, 50, 70));
    // Undo brings the Intertwine group back.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(s.doc().unwrap().doc.node(g).unwrap().kind, NodeKind::Intertwine { .. }));
}

/// Expand reaches an Intertwine group inside a selected group, and with every option on also
/// outlines the strokes of the rings and of the part drawn over the other one.
#[test]
fn expand_reaches_intertwine_groups_in_groups_and_outlines_their_strokes() {
    let mut s = session();
    let (g, ..) = intertwined(&mut s);
    arrange(&mut s, json!({"at": [50, 30]}));
    let c = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap());
    select(&mut s, &[g, c]);
    let outer = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.expand", &json!({})).unwrap();
    let (mut intertwines, mut strokes) = (0, 0);
    s.doc().unwrap().doc.node(outer).unwrap().walk(&mut |n| {
        intertwines += usize::from(matches!(n.kind, NodeKind::Intertwine { .. }));
        strokes += usize::from(!n.appearance.stroke_paint().is_none());
    });
    assert_eq!((intertwines, strokes), (0, 0));
    let r = render(&s);
    assert!(red(&r, 50, 30) && blue(&r, 50, 70), "it looks as it did");
    let info = s.execute("object.expand.info", &json!({})).unwrap();
    assert_eq!((&info["object"], &info["stroke"]), (&json!(false), &json!(false)), "nothing is left to expand");
}

#[test]
fn the_arranged_areas_move_with_the_group() {
    let mut s = session();
    let (g, ..) = intertwined(&mut s);
    arrange(&mut s, json!({"at": [50, 30]}));
    select(&mut s, &[g]);
    s.execute("object.move", &json!({"dx": 0, "dy": 10})).unwrap();
    let r = render(&s);
    assert!(red(&r, 50, 40) && blue(&r, 50, 80));
}

#[test]
fn the_woven_rings_survive_save_and_export() {
    let mut s = session();
    intertwined(&mut s);
    arrange(&mut s, json!({"at": [50, 30]}));
    for format in ["vectorcraft", "svg", "pdf"] {
        let data = s.execute("document.export", &json!({"format": format})).unwrap()["dataBase64"].clone();
        let mut o = Session::new();
        o.execute("document.open", &json!({"name": format!("rings.{format}"), "dataBase64": data})).unwrap();
        let r = render(&o);
        assert!(red(&r, 50, 30) && blue(&r, 50, 70), "{format}");
    }
}

#[test]
fn what_cannot_be_intertwined_is_refused_with_a_reason() {
    let mut s = session();
    let (a, b) = rings(&mut s);
    select(&mut s, &[a]);
    let e = s.execute("object.intertwine.make", &json!({})).unwrap_err();
    assert!(e.to_string().contains("two or more"), "{e}");
    select(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.intertwine.make", &json!({})).unwrap());
    let c = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap());
    let e = s.execute("object.intertwine.make", &json!({"ids": [g.0, c.0]})).unwrap_err();
    assert!(e.to_string().contains("an Intertwine group can't") && !e.to_string().contains("already"), "{e}");
    let e = s.execute("object.intertwine.make", &json!({"ids": [a.0, c.0]})).unwrap_err();
    assert!(e.to_string().contains("already in an Intertwine group"), "{e}");
    select(&mut s, &[g]);
    let e = s.execute("object.intertwine.arrange", &json!({"at": [5, 50]})).unwrap_err();
    assert!(e.to_string().contains("no overlap"), "{e}");
    let e = s.execute("object.intertwine.arrange", &json!({"at": [50, 30], "area": [[0, 0], [1, 0], [1, 1]]})).unwrap_err();
    assert!(e.to_string().contains("either"), "{e}");
    let e = s.execute("object.intertwine.arrange", &json!({"at": [50, 30], "order": "sideways"})).unwrap_err();
    assert!(e.to_string().contains("sideways"), "{e}");
    let e = s.execute("object.intertwine.arrange", &json!({"area": [[0, 0], [1, 0]]})).unwrap_err();
    assert!(e.to_string().contains("polygon"), "{e}");
    let e = s.execute("object.intertwine.arrange", &json!({"id": c.0, "at": [1, 1]})).unwrap_err();
    assert!(e.to_string().contains("not an Intertwine group"), "{e}");
}

// ---------- on the canvas ----------

fn press(s: &mut Session, pts: &[(f64, f64)], shift: bool) -> Vec<UiRequest> {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind};
    let view = ViewInfo::default();
    let ev = |kind, (x, y): (f64, f64)| {
        let mut e = PointerEvent::new(kind, x, y);
        e.mods = Mods { shift, ..Mods::default() };
        e
    };
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    s.pointer(&ev(PointerKind::Move, first), view).unwrap();
    s.pointer(&ev(PointerKind::Down, first), view).unwrap();
    for p in &pts[1..] {
        s.pointer(&ev(PointerKind::Drag, *p), view).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, last), view).unwrap()
}

#[test]
fn make_chooses_the_intertwine_tool_and_a_click_on_an_overlap_arranges_it() {
    let mut s = session();
    let view = ViewInfo::default();
    s.select_tool("ellipse", view).unwrap();
    intertwined(&mut s);
    assert_eq!(s.tool_id(), "intertwine");
    // Hovering the overlap highlights it.
    s.pointer(&vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Move, 50.0, 30.0), view).unwrap();
    assert_eq!(s.overlays(view).len(), 1);
    press(&mut s, &[(50.0, 30.0)], false);
    let r = render(&s);
    assert!(red(&r, 50, 30) && blue(&r, 50, 70));
    // A click off any overlap changes nothing (and reports no error).
    press(&mut s, &[(5.0, 5.0)], false);
    assert!(red(&render(&s), 50, 30));
    // Esc goes back to the tool chosen before Make.
    let back = s.tool_key(vectorcraft_tools::ToolKey::Escape, Default::default(), view).unwrap();
    assert_eq!(back, [UiRequest::SwitchTool("ellipse".into())]);
}

#[test]
fn a_drag_around_overlaps_arranges_them_and_shift_drags_a_rectangle() {
    let mut s = session();
    intertwined(&mut s);
    // A loop around the bottom crossing.
    press(&mut s, &[(40.0, 60.0), (60.0, 60.0), (60.0, 80.0), (40.0, 80.0), (40.0, 61.0)], false);
    let r = render(&s);
    assert!(blue(&r, 50, 30) && red(&r, 50, 70));
    // A rectangle around both.
    press(&mut s, &[(40.0, 20.0), (50.0, 50.0), (60.0, 80.0)], true);
    let r = render(&s);
    assert!(red(&r, 50, 30) && blue(&r, 50, 70));
}

#[test]
fn edit_chooses_the_tool_for_the_group_of_a_selected_member() {
    let mut s = session();
    let (g, a, _) = intertwined(&mut s);
    s.select_tool("selection", ViewInfo::default()).unwrap();
    select(&mut s, &[a]);
    let v = s.execute("object.intertwine.edit", &json!({})).unwrap();
    assert_eq!(v["id"], g.0);
    assert_eq!(s.tool_id(), "intertwine");
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [g]);
}
