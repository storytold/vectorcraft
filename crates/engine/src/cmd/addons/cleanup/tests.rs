//! Artwork Cleanup's commands: Simplify Anchors (Keep Shape), the health check's kinds found and
//! fixed one by one, Fix as one undo step, scopes, Show, Deep Ungroup and Rejoin Path Pieces.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind};
use vectorcraft_geom::{FillRule, Rect};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

/// A path through `pts` (anchors as `{x, y, out?}` objects).
fn path(s: &mut Session, anchors: Value, closed: bool) -> NodeId {
    id_of(&s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap())
}

fn line(s: &mut Session, pts: &[(f64, f64)]) -> NodeId {
    path(s, pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect(), false)
}

fn node(s: &Session, id: NodeId) -> Option<Node> {
    s.doc().unwrap().doc.node(id).cloned()
}

fn change(s: &mut Session, id: NodeId, f: impl FnOnce(&mut Node)) {
    s.edit("t", |d, _| {
        f(d.node_mut(id).unwrap());
        Ok(())
    })
    .unwrap();
}

/// Put `id` inside a new container that `kind` makes of its members.
fn wrap(s: &mut Session, id: NodeId, kind: fn(Vec<std::sync::Arc<Node>>) -> NodeKind) -> NodeId {
    s.edit("t", |d, _| {
        let w = d.alloc_id();
        Ok(d.wrap(&[id], |c| Node::new(w, kind(c)))?)
    })
    .unwrap()
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.select(|_, sel| sel.set(ids.iter().copied())).unwrap();
}

fn anchors(s: &Session, id: NodeId) -> usize {
    node(s, id).unwrap().path_data().unwrap().anchor_count()
}

/// How many issues of `kind` a check of `scope` finds.
fn found(s: &mut Session, kind: &str, scope: &str) -> usize {
    let r = s.execute("addon.cleanup.check", &json!({"kinds": [kind], "scope": scope})).unwrap();
    r["findings"].as_array().unwrap().iter().find(|f| f["kind"] == kind).map_or(0, |f| f["count"].as_u64().unwrap() as usize)
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn simplify_keeps_the_shape_with_fewer_anchors_in_one_step() {
    let mut s = session();
    let e = id_of(&s.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap());
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    assert_eq!(anchors(&s, e), 16);
    let bounds = node(&s, e).unwrap().geometric_bounds().unwrap();
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.simplify", &json!({})).unwrap();
    assert_eq!((r["before"].as_u64(), r["after"].as_u64()), (Some(16), Some(anchors(&s, e) as u64)));
    assert!(anchors(&s, e) <= 8, "{r}");
    let now = node(&s, e).unwrap().geometric_bounds().unwrap();
    for (a, b) in [(bounds.x0, now.x0), (bounds.y0, now.y0), (bounds.x1, now.x1), (bounds.y1, now.y1)] {
        assert!((a - b).abs() < 0.4, "{bounds:?} → {now:?}");
    }
    assert_eq!(undo_steps(&s), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(anchors(&s, e), 16);
    // A rectangle's corners stay; the anchors on its sides go.
    let r = rect(&mut s, 200.0, 50.0, 100.0, 60.0);
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    assert_eq!(anchors(&s, r), 8);
    s.execute("addon.cleanup.simplify", &json!({"protectCorners": true})).unwrap();
    assert_eq!(anchors(&s, r), 4);
    // Nothing left to remove: no undo step.
    let steps = undo_steps(&s);
    s.execute("addon.cleanup.simplify", &json!({})).unwrap();
    assert_eq!(undo_steps(&s), steps);
    for bad in [json!({"tolerance": 0}), json!({"tolerance": 101}), json!({"tolerance": "20"}), json!({"protectCorners": 1})] {
        assert!(s.execute("addon.cleanup.simplify", &bad).is_err(), "{bad}");
    }
}

/// Each kind: what makes the issue in a fresh document (beside a clean rectangle).
fn make_issue(s: &mut Session, kind: &str) {
    match kind {
        "emptyText" => {
            s.execute("text.create", &json!({"x": 20, "y": 20, "text": "  "})).unwrap();
        }
        "strayPoints" => {
            path(s, json!([{"x": 30, "y": 30}]), false);
        }
        "unpainted" => {
            let id = rect(s, 20.0, 150.0, 30.0, 30.0);
            change(s, id, |n| n.appearance = Appearance::default().into());
        }
        "zeroOpacity" => {
            let id = rect(s, 20.0, 150.0, 30.0, 30.0);
            change(s, id, |n| n.opacity = 0.0);
        }
        "duplicateObjects" => {
            let id = rect(s, 20.0, 150.0, 30.0, 30.0);
            s.edit("t", |d, _| {
                let mut copy = d.node(id).unwrap().clone();
                copy.id = d.alloc_id();
                let (parent, i, _) = d.position(id).unwrap();
                Ok(d.insert(parent, i + 1, copy)?)
            })
            .unwrap();
        }
        "offArtboard" => {
            rect(s, -500.0, -500.0, 30.0, 30.0);
        }
        "duplicateAnchors" => {
            line(s, &[(10.0, 150.0), (50.0, 150.0), (50.005, 150.0), (50.0, 190.0)]);
        }
        "tinyHandles" => {
            path(s, json!([{"x": 10, "y": 150, "out": [10.004, 150]}, {"x": 60, "y": 190}]), false);
        }
        "collinearAnchors" => {
            line(s, &[(10.0, 150.0), (30.0, 150.0), (50.0, 150.0), (50.0, 190.0)]);
        }
        "nearPoints" => {
            line(s, &[(10.0, 150.0), (50.0, 150.0)]);
            line(s, &[(50.005, 150.0), (90.0, 190.0)]);
        }
        "nearlyClosed" => {
            line(s, &[(10.0, 150.0), (50.0, 150.0), (50.0, 190.0), (10.005, 150.0)]);
        }
        "filledOpenPaths" => {
            let id = line(s, &[(10.0, 150.0), (50.0, 150.0), (50.0, 190.0)]);
            change(s, id, |n| n.appearance = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 1.0).into());
        }
        "needlessClipGroups" => {
            let inner = rect(s, 30.0, 160.0, 20.0, 20.0);
            let clip = rect(s, 20.0, 150.0, 50.0, 50.0);
            select(s, &[inner, clip]);
            s.execute("object.clippingMask.make", &json!({})).unwrap();
        }
        "needlessCompounds" => {
            // A compound path holds the paint of its paths.
            let id = rect(s, 20.0, 150.0, 30.0, 30.0);
            let look = node(s, id).unwrap().appearance;
            let compound = wrap(s, id, |children| NodeKind::Compound { children, rule: FillRule::NonZero });
            change(s, compound, |n| n.appearance = look);
        }
        "needlessGroups" => {
            let id = rect(s, 20.0, 150.0, 30.0, 30.0);
            wrap(s, id, |children| NodeKind::Group { children, clip: false });
        }
        "emptyArtboards" => {
            s.edit("t", |d, _| {
                let mut a = d.artboards[0].clone();
                a.id = d.next_artboard_id();
                a.rect = Rect::new(1000.0, 1000.0, 1100.0, 1100.0);
                d.artboards.push(a);
                Ok(())
            })
            .unwrap();
        }
        _ => panic!("no setup for {kind}"),
    }
}

const KINDS: [&str; 16] = [
    "emptyText",
    "strayPoints",
    "unpainted",
    "zeroOpacity",
    "duplicateObjects",
    "offArtboard",
    "duplicateAnchors",
    "tinyHandles",
    "collinearAnchors",
    "nearPoints",
    "nearlyClosed",
    "filledOpenPaths",
    "needlessClipGroups",
    "needlessCompounds",
    "needlessGroups",
    "emptyArtboards",
];

#[test]
fn each_check_finds_and_fixes_its_issue_and_leaves_clean_art_alone() {
    let mut clean = session();
    let r = rect(&mut clean, 200.0, 20.0, 80.0, 60.0);
    let before = node(&clean, r).unwrap();
    assert_eq!(clean.execute("addon.cleanup.check", &json!({})).unwrap()["total"], 0, "clean art has no issues");
    assert_eq!(clean.execute("addon.cleanup.fix", &json!({"scope": "document"})).unwrap()["total"], 0);
    assert_eq!(node(&clean, r).unwrap(), before);
    for kind in KINDS {
        let mut s = session();
        let r = rect(&mut s, 200.0, 20.0, 80.0, 60.0);
        make_issue(&mut s, kind);
        select(&mut s, &[]);
        assert!(found(&mut s, kind, "document") > 0, "{kind} not found");
        let fixed = s.execute("addon.cleanup.fix", &json!({"kinds": [kind], "scope": "document"})).unwrap();
        assert!(fixed["fixed"][kind].as_u64().unwrap() > 0, "{kind}: {fixed}");
        assert_eq!(found(&mut s, kind, "document"), 0, "{kind} still found after Fix");
        assert_eq!(s.execute("addon.cleanup.check", &json!({})).unwrap()["total"], 0, "{kind} left other issues");
        assert_eq!(node(&s, r).unwrap(), before, "{kind} changed the clean rectangle");
    }
}

#[test]
fn fix_all_is_one_undo_step() {
    let mut s = session();
    for kind in KINDS {
        make_issue(&mut s, kind);
    }
    select(&mut s, &[]);
    let total = s.execute("addon.cleanup.check", &json!({})).unwrap()["total"].as_u64().unwrap();
    assert!(total >= 16, "{total}");
    let (steps, doc) = (undo_steps(&s), s.doc().unwrap().doc.clone());
    let r = s.execute("addon.cleanup.fix", &json!({})).unwrap();
    assert_eq!(r["scope"], "document");
    assert_eq!(undo_steps(&s), steps + 1);
    assert_eq!(s.execute("addon.cleanup.check", &json!({})).unwrap()["total"], 0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers, doc.layers);
    assert_eq!(s.doc().unwrap().doc.artboards.len(), doc.artboards.len());
}

#[test]
fn the_scope_is_the_selection_or_the_document() {
    let mut s = session();
    let a = path(&mut s, json!([{"x": 30, "y": 30}]), false);
    let b = path(&mut s, json!([{"x": 60, "y": 30}]), false);
    select(&mut s, &[a]);
    let r = s.execute("addon.cleanup.check", &json!({"kinds": ["strayPoints"]})).unwrap();
    assert_eq!((r["scope"].as_str(), r["total"].as_u64()), (Some("selection"), Some(1)));
    assert_eq!(found(&mut s, "strayPoints", "document"), 2);
    s.execute("addon.cleanup.fix", &json!({"kinds": ["strayPoints"]})).unwrap();
    assert!(node(&s, a).is_none() && node(&s, b).is_some(), "only the selection is fixed");
    // Nothing selected: the document.
    let r = s.execute("addon.cleanup.check", &json!({})).unwrap();
    assert_eq!((r["scope"].as_str(), r["total"].as_u64()), (Some("document"), Some(1)));
    assert!(s.execute("addon.cleanup.check", &json!({"scope": "selection"})).is_err(), "nothing selected");
    for bad in [json!({"scope": "page"}), json!({"kinds": ["nope"]}), json!({"kinds": "strayPoints"}), json!({"kinds": [1]})] {
        assert!(s.execute("addon.cleanup.check", &bad).is_err(), "{bad}");
        assert!(s.execute("addon.cleanup.fix", &bad).is_err(), "{bad}");
    }
}

#[test]
fn show_selects_the_objects_with_the_issue() {
    let mut s = session();
    rect(&mut s, 200.0, 20.0, 80.0, 60.0);
    let stray = path(&mut s, json!([{"x": 30, "y": 30}]), false);
    select(&mut s, &[]);
    let r = s.execute("addon.cleanup.show", &json!({"kind": "strayPoints"})).unwrap();
    assert_eq!(r["count"], 1);
    assert_eq!(s.doc().unwrap().selection.objects, [stray]);
    assert!(s.execute("addon.cleanup.show", &json!({"kind": "emptyText", "scope": "document"})).is_err(), "no such issue");
    assert!(s.execute("addon.cleanup.show", &json!({})).is_err() && s.execute("addon.cleanup.show", &json!({"kind": "x"})).is_err());
}

#[test]
fn deep_ungroup_takes_apart_every_level_but_clipping_groups() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let b = rect(&mut s, 40.0, 10.0, 20.0, 20.0);
    let c = rect(&mut s, 70.0, 10.0, 20.0, 20.0);
    select(&mut s, &[b, c]);
    let inner = id_of(&s.execute("object.group", &json!({})).unwrap());
    change(&mut s, inner, |n| n.opacity = 0.5);
    let clipped = rect(&mut s, 100.0, 10.0, 10.0, 10.0);
    let clip = rect(&mut s, 95.0, 5.0, 30.0, 30.0);
    select(&mut s, &[clipped, clip]);
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let clip_group = s.doc().unwrap().selection.objects[0];
    select(&mut s, &[a, inner, clip_group]);
    let outer = id_of(&s.execute("object.group", &json!({})).unwrap());
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.deepUngroup", &json!({})).unwrap();
    assert_eq!(r["ungrouped"], 2);
    assert_eq!(undo_steps(&s), steps + 1);
    let d = &s.doc().unwrap().doc;
    let layer: Vec<NodeId> = d.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(layer, [a, b, c, clip_group]);
    assert!(d.node(outer).is_none() && d.node(inner).is_none());
    assert_eq!(d.node(b).unwrap().opacity, 0.5, "members keep their groups' opacity");
    assert_eq!(s.doc().unwrap().selection.objects.len(), 4);
    // Asked to, clipping groups come apart too, their clipping paths kept as plain paths.
    select(&mut s, &[clip_group]);
    s.execute("addon.cleanup.deepUngroup", &json!({"includeClipGroups": true})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(clip_group).is_none());
    assert!(matches!(d.node(clip).unwrap().kind, NodeKind::Path { clipping: false, .. }));
}

#[test]
fn rejoin_joins_pieces_within_the_gap_only() {
    let mut s = session();
    let a = line(&mut s, &[(10.0, 10.0), (50.0, 10.0)]);
    let b = line(&mut s, &[(50.05, 10.0), (90.0, 50.0)]);
    let far = line(&mut s, &[(92.0, 50.0), (120.0, 50.0)]);
    select(&mut s, &[a, b, far]);
    let r = s.execute("addon.cleanup.rejoin", &json!({})).unwrap();
    assert_eq!((r["joined"].as_u64(), r["removed"].as_u64()), (Some(1), Some(1)));
    // The topmost piece takes the joined path.
    assert!(node(&s, a).is_none());
    assert_eq!(anchors(&s, b), 3);
    assert_eq!(anchors(&s, far), 2, "beyond the gap");
    select(&mut s, &[b, far]);
    let r = s.execute("addon.cleanup.rejoin", &json!({"gap": 3})).unwrap();
    assert_eq!(r["joined"], 1);
    assert_eq!(anchors(&s, far), 4);
    for bad in [json!({"gap": -1}), json!({"gap": 7}), json!({"gap": "1"})] {
        assert!(s.execute("addon.cleanup.rejoin", &bad).is_err(), "{bad}");
    }
}
