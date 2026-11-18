//! Artwork Cleanup's commands: Simplify Anchors (Keep Shape), the health check's kinds found and
//! fixed one by one, Fix as one undo step, scopes, Show, Deep Ungroup and Rejoin Path Pieces.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, LayerColor, Node, NodeId, NodeKind};
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

#[test]
fn simplify_leaves_locked_and_hidden_paths_alone() {
    let mut s = session();
    let mut ids = vec![];
    for x in [20.0, 140.0, 260.0] {
        ids.push(id_of(&s.execute("shape.ellipse", &json!({"x": x, "y": 50, "width": 100, "height": 100})).unwrap()));
        s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
        s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    }
    select(&mut s, &ids);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    change(&mut s, ids[1], |n| n.locked = true);
    change(&mut s, ids[2], |n| n.visible = false);
    select(&mut s, &[g]);
    let r = s.execute("addon.cleanup.simplify", &json!({})).unwrap();
    assert_eq!(r["ids"], json!([ids[0].0]));
    assert_eq!(r["before"], 16);
    assert!(anchors(&s, ids[0]) < 16);
    assert_eq!((anchors(&s, ids[1]), anchors(&s, ids[2])), (16, 16));
    // In a selected locked group, a path stays too, with no undo step.
    change(&mut s, ids[1], |n| n.locked = false);
    change(&mut s, g, |n| n.locked = true);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.simplify", &json!({})).unwrap();
    assert_eq!((r["before"].as_u64(), r["after"].as_u64()), (Some(0), Some(0)));
    assert_eq!(anchors(&s, ids[1]), 16);
    assert_eq!(undo_steps(&s), steps);
    // Named by `ids`, the locked group's paths, its hidden one included, are simplified.
    let r = s.execute("addon.cleanup.simplify", &json!({"ids": [g.0]})).unwrap();
    assert_eq!(r["ids"], json!([ids[0].0, ids[1].0, ids[2].0]));
    assert!(anchors(&s, ids[1]) < 16 && anchors(&s, ids[2]) < 16);
    assert_eq!(undo_steps(&s), steps + 1);
}

/// A path of 16 anchors at (`x`, `y`): an ellipse with anchors added twice.
fn ellipse16(s: &mut Session, x: f64, y: f64) -> NodeId {
    let e = id_of(&s.execute("shape.ellipse", &json!({"x": x, "y": y, "width": 50, "height": 50})).unwrap());
    select(s, &[e]);
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    e
}

/// Simplify Anchors takes the locked and hidden paths `ids` names, those in the locked or hidden
/// groups and layers it names (their locked and hidden members too) and a hidden compound path's
/// member, as one undo step, without selecting them; from the selection it leaves them alone, and a
/// path on a locked artboard stays even when named.
#[test]
fn simplify_named_locked_and_hidden_paths() {
    let mut s = session();
    let locked = ellipse16(&mut s, 10.0, 10.0);
    let hidden = ellipse16(&mut s, 70.0, 10.0);
    change(&mut s, locked, |n| n.locked = true);
    change(&mut s, hidden, |n| n.visible = false);
    // A locked group with a hidden member, and a hidden group with a locked member.
    let in_locked = ellipse16(&mut s, 130.0, 10.0);
    let locked_group = wrap(&mut s, in_locked, |c| NodeKind::Group { children: c, clip: false });
    change(&mut s, locked_group, |n| n.locked = true);
    change(&mut s, in_locked, |n| n.visible = false);
    let in_hidden = ellipse16(&mut s, 190.0, 10.0);
    let hidden_group = wrap(&mut s, in_hidden, |c| NodeKind::Group { children: c, clip: false });
    change(&mut s, hidden_group, |n| n.visible = false);
    change(&mut s, in_hidden, |n| n.locked = true);
    // A member of a hidden compound path.
    let member = ellipse16(&mut s, 250.0, 10.0);
    let compound = wrap(&mut s, member, |c| NodeKind::Compound { children: c, rule: FillRule::NonZero });
    change(&mut s, compound, |n| n.visible = false);
    // A path in a locked layer, named itself.
    let layer = id_of(&s.execute("layer.new", &json!({})).unwrap());
    let in_layer = ellipse16(&mut s, 10.0, 100.0);
    let on_board = ellipse16(&mut s, 10.0, 200.0);
    change(&mut s, layer, |n| n.locked = true);
    let all = [locked, hidden, in_locked, in_hidden, member, in_layer];
    select(&mut s, &[locked, hidden, locked_group, hidden_group, compound]);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.simplify", &json!({})).unwrap();
    assert_eq!(r["ids"], json!([]), "from the selection, none is taken");
    assert_eq!(undo_steps(&s), steps);
    assert!(all.iter().all(|id| anchors(&s, *id) == 16));
    select(&mut s, &[]);
    s.edit("Select Anchors", |_, sel| {
        sel.anchors.insert(locked, [(0, 1)].into());
        Ok(())
    })
    .unwrap();
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.simplify", &json!({"ids": [locked.0, hidden.0, locked_group.0, hidden_group.0, member.0, in_layer.0]})).unwrap();
    assert_eq!(r["ids"], json!(all.iter().map(|i| i.0).collect::<Vec<_>>()));
    assert_eq!(r["before"], 16 * 6);
    assert_eq!(r["after"].as_u64(), Some(all.iter().map(|id| anchors(&s, *id) as u64).sum::<u64>()));
    assert!(all.iter().all(|id| anchors(&s, *id) < 16), "{r}");
    assert_eq!(undo_steps(&s), steps + 1);
    let sel = &s.doc().unwrap().selection;
    assert!(sel.objects.is_empty() && sel.anchors.is_empty(), "nothing is selected, and the changed path's anchors are not");
    let n = node(&s, in_hidden).unwrap();
    assert!(n.locked && n.visible && !node(&s, hidden_group).unwrap().visible);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(all.iter().all(|id| anchors(&s, *id) == 16));
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(all.iter().all(|id| anchors(&s, *id) < 16));
    // Nothing left to remove: no undo step.
    let steps = undo_steps(&s);
    s.execute("addon.cleanup.simplify", &json!({"ids": [locked.0]})).unwrap();
    assert_eq!(undo_steps(&s), steps);
    // On a locked artboard, a named path stays as it is.
    s.edit("Lock Artboard", |d, _| {
        d.artboards[0].locked = true;
        Ok(())
    })
    .unwrap();
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.simplify", &json!({"ids": [on_board.0]})).unwrap();
    assert_eq!((r["ids"].clone(), anchors(&s, on_board), undo_steps(&s)), (json!([]), 16, steps));
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

/// The pixel at (100, 100) of the document drawn on white.
fn pixel(s: &Session) -> [u8; 4] {
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 400, 300, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(100, 100)
}

/// A clipping path hides the art outside it: empty type that clips a group, and type off every
/// artboard that clips a layer, stay, and the art stays hidden.
#[test]
fn clipping_type_stays_with_the_art_it_hides() {
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    let mut s = session();
    let art = rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let t = id_of(&s.execute("text.create", &json!({"x": 60, "y": 80, "text": "  ", "size": 40})).unwrap());
    select(&mut s, &[art, t]);
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    select(&mut s, &[]);
    assert_eq!(pixel(&s), WHITE);
    assert_eq!(found(&mut s, "emptyText", "document"), 0);
    assert_eq!(s.execute("addon.cleanup.fix", &json!({"scope": "document"})).unwrap()["total"], 0);
    select(&mut s, &[t]);
    assert_eq!(found(&mut s, "emptyText", "selection"), 0, "the clipping path selected");
    assert_eq!(s.execute("addon.cleanup.fix", &json!({"scope": "selection"})).unwrap()["total"], 0);
    assert!(node(&s, t).is_some());
    assert_eq!(pixel(&s), WHITE);

    let mut s = session();
    rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let t = id_of(&s.execute("text.create", &json!({"x": -300, "y": -300, "text": "Clip", "size": 40})).unwrap());
    s.execute("layer.clippingMask.toggle", &json!({})).unwrap();
    select(&mut s, &[]);
    assert_eq!(pixel(&s), WHITE);
    assert_eq!(found(&mut s, "offArtboard", "document"), 0);
    assert_eq!(s.execute("addon.cleanup.fix", &json!({"scope": "document"})).unwrap()["total"], 0);
    select(&mut s, &[t]);
    assert_eq!(found(&mut s, "offArtboard", "selection"), 0, "the clipping path selected");
    assert_eq!(s.execute("addon.cleanup.fix", &json!({"scope": "selection"})).unwrap()["total"], 0);
    assert!(node(&s, t).is_some());
    assert_eq!(pixel(&s), WHITE);
}

/// Type set twice in the same place (from a PDF, or for a fake bold) is a duplicate within a point:
/// Show selects the upper copy and Fix removes it. Other text, or the same text further off, stays.
#[test]
fn doubled_type_within_a_point_is_a_duplicate() {
    let mut s = session();
    let text = |s: &mut Session, x: f64, text: &str| id_of(&s.execute("text.create", &json!({"x": x, "y": 100, "text": text})).unwrap());
    let base = text(&mut s, 20.0, "Total");
    let doubled = text(&mut s, 20.4, "Total");
    let other = text(&mut s, 20.0, "Subtotal");
    let far = text(&mut s, 23.0, "Subtotal");
    assert_eq!(found(&mut s, "duplicateObjects", "document"), 1);
    let shown = s.execute("addon.cleanup.show", &json!({"kind": "duplicateObjects", "scope": "document"})).unwrap();
    assert_eq!(shown["ids"], json!([doubled.0]));
    s.execute("addon.cleanup.fix", &json!({"kinds": ["duplicateObjects"], "scope": "document"})).unwrap();
    assert!(node(&s, doubled).is_none());
    for id in [base, other, far] {
        assert!(node(&s, id).is_some(), "{id:?} stays");
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
fn a_long_straight_run_loses_its_inner_anchors_in_one_step() {
    let mut s = session();
    let n: u32 = 5_000;
    let mut pts: Vec<(f64, f64)> = (0..n).map(|i| (10.0 + f64::from(i) * 0.05, 150.0)).collect();
    pts.push((260.0, 190.0));
    let id = line(&mut s, &pts);
    let other = rect(&mut s, 300.0, 20.0, 40.0, 40.0);
    select(&mut s, &[other]);
    let check = s.execute("addon.cleanup.check", &json!({"kinds": ["collinearAnchors"], "scope": "document"})).unwrap();
    assert_eq!(check["total"], n - 2, "{check}");
    assert_eq!(check["findings"][0]["ids"], json!([id.0]));
    let shown = s.execute("addon.cleanup.show", &json!({"kind": "collinearAnchors", "scope": "document"})).unwrap();
    assert_eq!(shown["count"], n - 2);
    assert_eq!(s.doc().unwrap().selection.objects, [id], "Show selects the path");
    select(&mut s, &[other]);
    let (steps, before) = (undo_steps(&s), node(&s, id).unwrap());
    let r = s.execute("addon.cleanup.fix", &json!({"kinds": ["collinearAnchors"], "scope": "document"})).unwrap();
    assert_eq!((r["total"].as_u64(), r["fixed"]["collinearAnchors"].as_u64()), (Some(u64::from(n) - 2), Some(u64::from(n) - 2)));
    assert_eq!(undo_steps(&s), steps + 1);
    assert_eq!(s.doc().unwrap().selection.objects, [other], "Fix keeps the selection");
    let ends: Vec<(f64, f64)> = node(&s, id).unwrap().path_data().unwrap().subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect();
    assert_eq!(ends, [(10.0, 150.0), (10.0 + f64::from(n - 1) * 0.05, 150.0), (260.0, 190.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id).unwrap(), before);
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

/// The ids of the first layer's objects, bottom to top.
fn layer_ids(s: &Session) -> Vec<NodeId> {
    match &s.doc().unwrap().doc.layers[0].kind {
        NodeKind::Layer { children, .. } => children.iter().map(|c| c.id).collect(),
        other => panic!("not a layer: {other:?}"),
    }
}

/// A plain group of the one object `id`.
fn group(s: &mut Session, id: NodeId) -> NodeId {
    wrap(s, id, |children| NodeKind::Group { children, clip: false })
}

#[test]
fn unwrapping_selected_groups_keeps_their_places_and_order() {
    let mut s = session();
    let c1 = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let a = rect(&mut s, 40.0, 10.0, 20.0, 20.0);
    let g1 = group(&mut s, a);
    let c2 = rect(&mut s, 70.0, 10.0, 20.0, 20.0);
    let empty = s
        .edit("t", |d, _| {
            let (parent, i, _) = d.position(c2).unwrap();
            let g = d.alloc_id();
            Ok(d.insert(parent, i + 1, Node::new(g, NodeKind::Group { children: vec![], clip: false }))?)
        })
        .unwrap();
    let c3 = rect(&mut s, 100.0, 10.0, 20.0, 20.0);
    let b = rect(&mut s, 130.0, 10.0, 20.0, 20.0);
    let inner = group(&mut s, b);
    let outer = group(&mut s, inner);
    let c4 = rect(&mut s, 160.0, 10.0, 20.0, 20.0);
    let x = rect(&mut s, 190.0, 10.0, 20.0, 20.0);
    let unselected = group(&mut s, x);
    assert_eq!(layer_ids(&s), [c1, g1, c2, empty, c3, outer, c4, unselected]);
    select(&mut s, &[c1, g1, c2, empty, c3, outer, c4]);
    let check = s.execute("addon.cleanup.check", &json!({"kinds": ["needlessGroups"], "scope": "selection"})).unwrap();
    let r = s.execute("addon.cleanup.fix", &json!({"kinds": ["needlessGroups"], "scope": "selection"})).unwrap();
    // The inner group is found only once its outer group is gone: on a later pass, as a root.
    assert_eq!(r["fixed"]["needlessGroups"], 4, "{r}");
    assert_eq!(check["total"], 4, "the check finds what Fix fixes: {check}");
    let mut listed: Vec<u64> = check["findings"][0]["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    listed.sort_unstable();
    let mut gone: Vec<u64> = [g1, empty, outer, inner].iter().map(|i| i.0).collect();
    gone.sort_unstable();
    assert_eq!(listed, gone);
    assert_eq!(layer_ids(&s), [c1, a, c2, c3, b, c4, unselected], "each member takes its group's place");
    for id in [g1, empty, outer, inner] {
        assert!(node(&s, id).is_none(), "{id:?} unwrapped");
    }
}

#[test]
fn fix_all_reaches_the_members_of_unwrapped_selected_groups() {
    let mut s = session();
    rect(&mut s, 200.0, 20.0, 80.0, 60.0);
    let below = rect(&mut s, 20.0, 150.0, 30.0, 30.0);
    let copy = s
        .edit("t", |d, _| {
            let mut copy = d.node(below).unwrap().clone();
            copy.id = d.alloc_id();
            let (parent, i, _) = d.position(below).unwrap();
            Ok(d.insert(parent, i + 1, copy)?)
        })
        .unwrap();
    let g = group(&mut s, copy);
    select(&mut s, &[below, g]);
    // The copy sits apart from the object it doubles until its group is unwrapped.
    assert_eq!(found(&mut s, "duplicateObjects", "selection"), 0);
    assert_eq!(found(&mut s, "needlessGroups", "selection"), 1);
    let r = s.execute("addon.cleanup.fix", &json!({"scope": "selection"})).unwrap();
    assert_eq!((r["fixed"]["needlessGroups"].as_u64(), r["fixed"]["duplicateObjects"].as_u64()), (Some(1), Some(1)), "{r}");
    assert!(node(&s, g).is_none() && node(&s, copy).is_none());
    assert!(node(&s, below).is_some());
}

#[test]
fn replacing_roots_takes_each_found_object_at_its_first_place() {
    use super::checks::replace_roots;
    let n = |id: u64| Node::new(NodeId(id), NodeKind::Group { children: vec![], clip: false });
    let ids = |v: &[u64]| v.iter().map(|i| NodeId(*i)).collect::<Vec<_>>();
    let roots = ids(&[1, 2, 1, 3]);
    assert_eq!(replace_roots(&roots, &[(NodeId(1), vec![n(10), n(11)])]), ids(&[10, 11, 2, 1, 3]));
    assert_eq!(replace_roots(&roots, &[(NodeId(1), vec![]), (NodeId(1), vec![])]), ids(&[2, 3]), "listed twice: both places");
    assert_eq!(replace_roots(&roots, &[(NodeId(3), vec![]), (NodeId(2), vec![n(20)])]), ids(&[1, 20, 1]));
    // A found object inside a root leaves the roots as they are.
    assert_eq!(replace_roots(&roots, &[(NodeId(7), vec![n(70)])]), roots);
    assert_eq!(replace_roots(&[], &[(NodeId(7), vec![])]), ids(&[]));
}

/// How many scope roots fixing `n` selected stray points above `n` selected clean objects looks at.
fn roots_seen_removing(n: usize) -> usize {
    use super::checks::ROOTS_SEEN;
    let mut s = session();
    let mut ids: Vec<NodeId> = (0..n).map(|i| rect(&mut s, (i % 20) as f64 * 15.0, (i / 20) as f64 * 15.0, 10.0, 10.0)).collect();
    ids.extend((0..n).map(|i| path(&mut s, json!([{"x": i % 300, "y": i / 300}]), false)));
    select(&mut s, &ids);
    ROOTS_SEEN.set(0);
    let r = s.execute("addon.cleanup.fix", &json!({"kinds": ["strayPoints"], "scope": "selection"})).unwrap();
    assert_eq!(r["total"].as_u64(), Some(n as u64));
    assert_eq!(layer_ids(&s), ids[..n]);
    ROOTS_SEEN.get()
}

#[test]
fn removing_many_selected_objects_looks_at_each_root_once_a_pass() {
    let (small, large) = (roots_seen_removing(150), roots_seen_removing(300));
    assert_eq!(small, 300, "one pass over the 300 roots");
    assert!(large <= 2 * small, "{small} roots looked at for 150 removals, {large} for 300");
}

#[test]
fn deleting_many_empty_artboards_walks_the_artboards_and_guides_once() {
    use crate::cmd::layer::ARTBOARD_PASSES;
    let mut s = session();
    rect(&mut s, 200.0, 20.0, 80.0, 60.0);
    let (keep, gone) = s
        .edit("t", |d, _| {
            let first = d.artboards[0].id;
            let keep = d.add_ruler_guide(&vectorcraft_doc::Guide { artboard: Some(first), ..vectorcraft_doc::Guide::new(true, 100.0) });
            let mut gone = vec![];
            for i in 0..40 {
                let mut a = d.artboards[0].clone();
                a.id = d.next_artboard_id();
                let x = 1000.0 + i as f64 * 200.0;
                a.rect = Rect::new(x, 1000.0, x + 100.0, 1100.0);
                gone.push(d.add_ruler_guide(&vectorcraft_doc::Guide { artboard: Some(a.id), ..vectorcraft_doc::Guide::new(true, x + 50.0) }));
                d.artboards.push(a);
            }
            Ok((keep, gone))
        })
        .unwrap();
    let first = s.doc().unwrap().doc.artboards[0].clone();
    ARTBOARD_PASSES.set(0);
    let r = s.execute("addon.cleanup.fix", &json!({"kinds": ["emptyArtboards"], "scope": "document"})).unwrap();
    assert_eq!(r["total"].as_u64(), Some(40), "{r}");
    assert_eq!(ARTBOARD_PASSES.get(), 2, "one walk over the artboards and one over the guides");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards, vec![first]);
    let left: Vec<Option<NodeId>> = d.ruler_guides().into_iter().map(|g| g.id).collect();
    assert_eq!(left, vec![Some(keep)]);
    assert!(gone.iter().all(|id| !left.contains(&Some(*id))));
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
fn deep_ungroup_keeps_hidden_art_hidden_and_locked_art_locked() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let b = rect(&mut s, 40.0, 10.0, 20.0, 20.0);
    let c = rect(&mut s, 70.0, 10.0, 20.0, 20.0);
    let e = rect(&mut s, 100.0, 10.0, 20.0, 20.0);
    select(&mut s, &[b]);
    let hidden = id_of(&s.execute("object.group", &json!({})).unwrap());
    select(&mut s, &[c]);
    let locked = id_of(&s.execute("object.group", &json!({})).unwrap());
    change(&mut s, hidden, |n| n.visible = false);
    change(&mut s, locked, |n| n.locked = true);
    change(&mut s, e, |n| n.locked = true);
    s.edit("t", |d, _| {
        let w = d.alloc_id();
        Ok(d.wrap(&[a, hidden, locked, e], |ch| Node::group(w, ch))?)
    })
    .unwrap();
    let outer = s.doc().unwrap().doc.layers[0].children().unwrap()[0].id;
    select(&mut s, &[outer]);
    let r = s.execute("addon.cleanup.deepUngroup", &json!({})).unwrap();
    assert_eq!(r["ungrouped"], 1);
    assert_eq!(r["ids"], json!([a.0]));
    let d = &s.doc().unwrap().doc;
    let layer: Vec<NodeId> = d.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(layer, [a, hidden, locked, e], "the hidden and locked groups stay whole");
    assert!(!d.node(hidden).unwrap().visible && d.node(b).unwrap().visible);
    assert!(d.node(locked).unwrap().locked && d.node(e).unwrap().locked);
    assert_eq!(s.doc().unwrap().selection.objects, [a]);
    // Named by `ids`, a locked group is taken apart as one undo step; its member becomes locked
    // and is not selected.
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.deepUngroup", &json!({"ids": [locked.0]})).unwrap();
    assert_eq!(r["ungrouped"], 1);
    assert!(node(&s, locked).is_none() && node(&s, c).unwrap().locked);
    assert_eq!(undo_steps(&s), steps + 1);
    assert_eq!(r["ids"], json!([]));
    // With nothing ungrouped, the selection stays and the reply lists it.
    let f = rect(&mut s, 130.0, 10.0, 20.0, 20.0);
    select(&mut s, &[a]);
    let r = s.execute("addon.cleanup.deepUngroup", &json!({"ids": [f.0]})).unwrap();
    assert_eq!((r["ids"].clone(), r["ungrouped"].clone()), (json!([a.0]), json!(0)));
    assert_eq!(s.doc().unwrap().selection.objects, [a]);
}

/// Put `ids` inside a new container that `kind` makes of its members.
fn wrap_all(s: &mut Session, ids: &[NodeId], kind: fn(Vec<std::sync::Arc<Node>>) -> NodeKind) -> NodeId {
    s.edit("t", |d, _| {
        let w = d.alloc_id();
        Ok(d.wrap(ids, |c| Node::new(w, kind(c)))?)
    })
    .unwrap()
}

fn plain_group(c: Vec<std::sync::Arc<Node>>) -> NodeKind {
    NodeKind::Group { children: c, clip: false }
}

fn sublayer(c: Vec<std::sync::Arc<Node>>) -> NodeKind {
    NodeKind::Layer { color: LayerColor::Preset(0), template: false, printable: true, children: c, clip: false, preview: true, dim_images: None }
}

/// Named by `ids`, Deep Ungroup takes apart locked and hidden groups, those nested in a group it
/// names and those in a layer it names (its sublayers included, the layer staying), as one undo
/// step; their members become locked or hidden as the groups were, keep the groups' opacity and are
/// not selected. From the selection those groups stay whole, as do a clipping group and a group on a
/// locked artboard even when named.
#[test]
fn deep_ungroup_named_locked_and_hidden_groups() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let b = rect(&mut s, 40.0, 10.0, 20.0, 20.0);
    let locked = wrap_all(&mut s, &[a], plain_group);
    let hidden = wrap_all(&mut s, &[b], plain_group);
    change(&mut s, locked, |n| n.locked = true);
    change(&mut s, hidden, |n| {
        n.visible = false;
        n.opacity = 0.5;
    });
    select(&mut s, &[locked, hidden]);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.deepUngroup", &json!({})).unwrap();
    assert_eq!((r["ungrouped"].clone(), undo_steps(&s)), (json!(0), steps), "from the selection, they stay whole");
    assert!(node(&s, locked).is_some() && node(&s, hidden).is_some());
    let r = s.execute("addon.cleanup.deepUngroup", &json!({"ids": [locked.0, hidden.0]})).unwrap();
    assert_eq!((r["ungrouped"].clone(), r["ids"].clone()), (json!(2), json!([])));
    assert_eq!(undo_steps(&s), steps + 1);
    assert!(node(&s, locked).is_none() && node(&s, hidden).is_none());
    let (na, nb) = (node(&s, a).unwrap(), node(&s, b).unwrap());
    assert!(na.locked && na.visible && !nb.visible && !nb.locked);
    assert_eq!(nb.opacity, 0.5);
    assert!(s.doc().unwrap().selection.objects.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, locked).is_some() && !node(&s, a).unwrap().locked && node(&s, b).unwrap().visible);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(node(&s, locked).is_none() && node(&s, a).unwrap().locked);
    // Locked and hidden groups nested in a named group that can be edited.
    let c = rect(&mut s, 70.0, 10.0, 20.0, 20.0);
    let e = rect(&mut s, 100.0, 10.0, 20.0, 20.0);
    let f = rect(&mut s, 130.0, 10.0, 20.0, 20.0);
    let inner_locked = wrap_all(&mut s, &[e], plain_group);
    let inner_hidden = wrap_all(&mut s, &[f], plain_group);
    change(&mut s, inner_locked, |n| n.locked = true);
    change(&mut s, inner_hidden, |n| n.visible = false);
    let outer = wrap_all(&mut s, &[c, inner_locked, inner_hidden], plain_group);
    let r = s.execute("addon.cleanup.deepUngroup", &json!({"ids": [outer.0]})).unwrap();
    assert_eq!((r["ungrouped"].clone(), r["ids"].clone()), (json!(3), json!([c.0])));
    assert!(node(&s, e).unwrap().locked && !node(&s, f).unwrap().visible);
    assert_eq!(s.doc().unwrap().selection.objects, [c]);
    // In a named layer and its sublayer; the layers and a clipping group stay.
    let layer = id_of(&s.execute("layer.new", &json!({})).unwrap());
    let g = rect(&mut s, 10.0, 100.0, 20.0, 20.0);
    let h = rect(&mut s, 40.0, 100.0, 20.0, 20.0);
    let layer_locked = wrap_all(&mut s, &[g], plain_group);
    change(&mut s, layer_locked, |n| n.locked = true);
    let layer_hidden = wrap_all(&mut s, &[h], plain_group);
    change(&mut s, layer_hidden, |n| n.visible = false);
    let sub = wrap_all(&mut s, &[layer_hidden], sublayer);
    let clipped = rect(&mut s, 100.0, 100.0, 10.0, 10.0);
    let clip = rect(&mut s, 95.0, 95.0, 30.0, 30.0);
    select(&mut s, &[clipped, clip]);
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let clip_group = s.doc().unwrap().selection.objects[0];
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.deepUngroup", &json!({"ids": [layer.0]})).unwrap();
    assert_eq!((r["ungrouped"].clone(), undo_steps(&s)), (json!(2), steps + 1));
    assert!(node(&s, layer_locked).is_none() && node(&s, layer_hidden).is_none());
    assert!(node(&s, layer).is_some() && node(&s, clip_group).is_some());
    assert!(node(&s, g).unwrap().locked && !node(&s, h).unwrap().visible);
    assert_eq!(s.doc().unwrap().doc.parent_of(h), Some(sub));
    // On a locked artboard, a named group stays whole, with the selection as it was.
    let k = rect(&mut s, 10.0, 200.0, 20.0, 20.0);
    let on_board = wrap_all(&mut s, &[k], plain_group);
    let off_board = rect(&mut s, 500.0, 10.0, 20.0, 20.0);
    s.edit("Lock Artboard", |d, _| {
        d.artboards[0].locked = true;
        Ok(())
    })
    .unwrap();
    select(&mut s, &[off_board]);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.deepUngroup", &json!({"ids": [on_board.0]})).unwrap();
    assert_eq!((r["ungrouped"].clone(), r["ids"].clone(), undo_steps(&s)), (json!(0), json!([off_board.0]), steps));
    assert!(node(&s, on_board).is_some());
    assert_eq!(s.doc().unwrap().selection.objects, [off_board]);
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

/// Named by `ids`, Rejoin Path Pieces takes locked and hidden pieces (in a locked group or layer
/// too) and joins only pieces alike in being hidden and in being locked, whichever is topmost, as one
/// undo step; a locked or hidden path it makes is not selected. From the selection it takes only
/// pieces that can be edited, and compound path members and pieces on a locked artboard stay out.
#[test]
fn rejoin_named_locked_and_hidden_pieces() {
    let mut s = session();
    let a = line(&mut s, &[(10.0, 10.0), (50.0, 10.0)]);
    let b = line(&mut s, &[(50.05, 10.0), (90.0, 10.0)]);
    let g = wrap_all(&mut s, &[a, b], plain_group);
    change(&mut s, g, |n| n.locked = true);
    select(&mut s, &[g]);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.rejoin", &json!({})).unwrap();
    assert_eq!((r["joined"].clone(), undo_steps(&s)), (json!(0), steps), "from the selection, none is taken");
    let r = s.execute("addon.cleanup.rejoin", &json!({"ids": [g.0]})).unwrap();
    assert_eq!((r["joined"].clone(), r["removed"].clone(), r["ids"].clone()), (json!(1), json!(1), json!([b.0])));
    assert_eq!(undo_steps(&s), steps + 1);
    assert!(node(&s, a).is_none() && anchors(&s, b) == 3);
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the locked path is not selected");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, a).is_some() && anchors(&s, b) == 2);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(node(&s, a).is_none() && anchors(&s, b) == 3);
    // A shown piece and two hidden ones, the hidden on top: only the hidden pair joins.
    let c = line(&mut s, &[(10.0, 40.0), (50.0, 40.0)]);
    let e = line(&mut s, &[(50.05, 40.0), (90.0, 40.0)]);
    let h = line(&mut s, &[(90.05, 40.0), (130.0, 40.0)]);
    change(&mut s, e, |n| n.visible = false);
    change(&mut s, h, |n| n.visible = false);
    let r = s.execute("addon.cleanup.rejoin", &json!({"ids": [c.0, e.0, h.0]})).unwrap();
    assert_eq!((r["joined"].clone(), r["removed"].clone(), r["ids"].clone()), (json!(1), json!(1), json!([h.0])));
    assert!(node(&s, e).is_none() && anchors(&s, c) == 2 && anchors(&s, h) == 3);
    assert!(!node(&s, h).unwrap().visible && node(&s, c).unwrap().visible);
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the hidden path is not selected");
    // An unlocked piece under a locked one, and a hidden piece under a shown one: none joins.
    let k = line(&mut s, &[(10.0, 70.0), (50.0, 70.0)]);
    let m = line(&mut s, &[(50.05, 70.0), (90.0, 70.0)]);
    let n = line(&mut s, &[(10.0, 100.0), (50.0, 100.0)]);
    let o = line(&mut s, &[(50.05, 100.0), (90.0, 100.0)]);
    change(&mut s, m, |x| x.locked = true);
    change(&mut s, n, |x| x.visible = false);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.rejoin", &json!({"ids": [k.0, m.0, n.0, o.0]})).unwrap();
    assert_eq!((r["joined"].clone(), r["removed"].clone(), undo_steps(&s)), (json!(0), json!(0), steps));
    assert!([k, m, n, o].iter().all(|id| anchors(&s, *id) == 2));
    // Named pieces in a locked layer join; members of a compound path stay out.
    let layer = id_of(&s.execute("layer.new", &json!({})).unwrap());
    let p = line(&mut s, &[(10.0, 130.0), (50.0, 130.0)]);
    let q = line(&mut s, &[(50.05, 130.0), (90.0, 130.0)]);
    let u = line(&mut s, &[(10.0, 160.0), (50.0, 160.0)]);
    let v = line(&mut s, &[(50.05, 160.0), (90.0, 160.0)]);
    wrap_all(&mut s, &[u, v], |c| NodeKind::Compound { children: c, rule: FillRule::NonZero });
    let w = line(&mut s, &[(10.0, 200.0), (50.0, 200.0)]);
    let x = line(&mut s, &[(50.05, 200.0), (90.0, 200.0)]);
    change(&mut s, layer, |l| l.locked = true);
    let r = s.execute("addon.cleanup.rejoin", &json!({"ids": [p.0, q.0, u.0, v.0]})).unwrap();
    assert_eq!((r["joined"].clone(), r["ids"].clone()), (json!(1), json!([q.0])));
    assert!(node(&s, p).is_none() && anchors(&s, u) == 2 && anchors(&s, v) == 2);
    // On a locked artboard, named pieces stay as they are.
    s.edit("Lock Artboard", |d, _| {
        d.artboards[0].locked = true;
        Ok(())
    })
    .unwrap();
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.rejoin", &json!({"ids": [w.0, x.0]})).unwrap();
    assert_eq!((r["joined"].clone(), undo_steps(&s)), (json!(0), steps));
    assert!(node(&s, w).is_some() && node(&s, x).is_some());
}
