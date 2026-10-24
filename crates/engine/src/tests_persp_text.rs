//! Type and symbols in perspective (projected, still type and symbol instances, Edit Text),
//! attachment bookkeeping (copy, paste, duplicate, Selection-tool moves, release) and drawing in
//! perspective (flare, spiral, polar grid, click-to-size dialogs).

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::Mods;
use vectorcraft_tools::distort::perspective::{self as persp, Plane};

use super::tests_persp_select::{attached, drag, grid, persp_session, undo_len};
use super::*;

/// Point type "HELLO" (48 pt) at (430, 400), attached to the right plane.
fn persp_text(s: &mut Session) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 430, "y": 400, "text": "HELLO", "size": 48})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [id.0], "plane": "right"})).unwrap();
    id
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

/// The art the renderer and exporters draw for `id` (its outlines, projected).
fn drawn_bounds(s: &Session, id: NodeId) -> Rect {
    let n = node(s, id);
    vectorcraft_render::effects::reshape(None, &n, None).unwrap().geometric_bounds().unwrap()
}

#[test]
fn type_in_perspective_is_projected_and_stays_type() {
    let (mut s, _) = persp_session();
    let id = persp_text(&mut s);
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Text(_)), "still type");
    assert_eq!(attached(&s, id), Some((Plane::Right, 0.0)));
    let h = n.projection().expect("drawn through a projection");
    // Really foreshortened: the right plane recedes to the right, so the letters' far end is
    // shorter than their near end.
    let out = vectorcraft_render::effects::reshape(None, &n, None).unwrap();
    let mut pts = vec![];
    out.walk(&mut |c| {
        if let Some(p) = c.path_data() {
            pts.extend(p.anchors().map(|(_, _, a)| a.p));
        }
    });
    let b = drawn_bounds(&s, id);
    let near: Vec<&Point> = pts.iter().filter(|p| p.x < b.x0 + 0.2 * b.width()).collect();
    let far: Vec<&Point> = pts.iter().filter(|p| p.x > b.x1 - 0.2 * b.width()).collect();
    let span = |v: &[&Point]| v.iter().map(|p| p.y).fold(f64::MIN, f64::max) - v.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    assert!(span(&far) < span(&near) * 0.97, "far {} near {}", span(&far), span(&near));
    // Bounds, hit testing and the selection box follow the drawn type.
    let gb = n.geometric_bounds().unwrap();
    assert!((gb.x0 - b.x0).abs() < 12.0 && (gb.x1 - b.x1).abs() < 12.0, "{gb:?} {b:?}");
    let hit = vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, b.center(), Default::default()).unwrap();
    assert_eq!(hit.leaf, id);
    let _ = h;
}

#[test]
fn type_in_perspective_exports_projected_outlines() {
    let (mut s, _) = persp_session();
    let id = persp_text(&mut s);
    let b = drawn_bounds(&s, id);
    let doc = s.doc().unwrap().doc.clone();
    let svg = vectorcraft_svg::export(&doc, &Default::default());
    assert!(!svg.contains("HELLO"), "no flat text in the SVG");
    let pdf = vectorcraft_render::effects::bake_document(&doc).unwrap();
    let baked = pdf.node(id).unwrap();
    assert!(!matches!(baked.kind, NodeKind::Text(_)));
    let bb = baked.geometric_bounds().unwrap();
    assert!((bb.x0 - b.x0).abs() < 1e-6 && (bb.y1 - b.y1).abs() < 1e-6, "{bb:?} {b:?}");
}

#[test]
fn type_in_perspective_in_a_group_with_a_geometry_effect_exports_projected() {
    let (mut s, rect) = persp_session();
    let id = persp_text(&mut s);
    let b = drawn_bounds(&s, id);
    s.execute("select.set", &json!({"ids": [rect.0, id.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    // Offset Path 0 on the group leaves every path as it is.
    s.execute("effect.apply", &json!({"effect": "path.offsetPath", "params": {"offset": 0}})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let baked = vectorcraft_render::effects::bake_document(&doc).unwrap();
    let bb = baked.node(id).unwrap().geometric_bounds().unwrap();
    assert!((bb.x0 - b.x0).abs() < 1e-6 && (bb.y1 - b.y1).abs() < 1e-6, "{bb:?} {b:?}");
}

#[test]
fn moving_type_in_perspective_moves_its_projection() {
    let (mut s, rect) = persp_session();
    let id = persp_text(&mut s);
    let NodeKind::Text(t) = &node(&s, id).kind else { panic!("type") };
    let flat = t.xf;
    s.execute("perspective.transform", &json!({"ids": [id.0], "matrix": [1, 0, 0, 1, 40, 0]})).unwrap();
    let n = node(&s, id);
    let NodeKind::Text(t) = &n.kind else { panic!() };
    assert_eq!(t.xf, flat, "the type itself is untouched");
    // Its drawn corner moved 40 pt along the plane.
    let g = grid(&s);
    let hi = g.homography(Plane::Right).unwrap().inverse().unwrap();
    let b0 = {
        s.execute("edit.undo", &json!({})).unwrap();
        drawn_bounds(&s, id)
    };
    s.execute("edit.redo", &json!({})).unwrap();
    let b1 = drawn_bounds(&s, id);
    let d = hi.apply(Point::new(b1.x0, b1.y1)).unwrap() - hi.apply(Point::new(b0.x0, b0.y1)).unwrap();
    assert!((d.x - 40.0).abs() < 1.0 && d.y.abs() < 1.0, "{d:?}");
    let _ = rect;
}

#[test]
fn edit_text_shows_it_flat_in_isolation_then_projects_it_again() {
    let (mut s, _) = persp_session();
    let id = persp_text(&mut s);
    let drawn = node(&s, id).geometric_bounds().unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("perspective.editText", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.isolation, Some(id));
    let n = node(&s, id);
    assert!(n.projection().is_none(), "flat while editing");
    // Shown where it is drawn in perspective.
    assert!(n.geometric_bounds().unwrap().center().distance(drawn.center()) < 1e-6);
    s.execute("text.setText", &json!({"id": id.0, "text": "HELLO WORLD"})).unwrap();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    let n = node(&s, id);
    assert!(n.projection().is_some(), "projected again");
    // Nothing left flat anywhere in the history.
    for e in &s.doc().unwrap().history.undo {
        if let Some(p) = e.doc.node(id).and_then(|n| n.extras().perspective.as_deref()) {
            assert!(!p.editing);
        }
    }
    // Not type in perspective: refused.
    let (mut s2, rect) = persp_session();
    assert!(s2.execute("perspective.editText", &json!({"id": rect.0})).is_err());
}

/// Edit Text ending while a background save writes the file leaves the document saved once the
/// file is written: the flat view is never saved. Saved while editing and edited after: undoing
/// the edit once Edit Text has ended is back at what the file holds.
#[test]
fn ending_edit_text_during_a_save_leaves_the_document_saved() {
    use crate::cmd::fileio::{SaveMode, save_job, save_plan, save_with};
    let (mut s, _) = persp_session();
    let id = persp_text(&mut s);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("perspective.editText", &json!({})).unwrap();
    let uid = s.doc().unwrap().uid;
    let plan = save_plan(&s, SaveMode::SaveAs, &json!({"path": "/x/type.vectorcraft", "modified": 777})).unwrap();
    let job = save_job(&mut s, plan).unwrap();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    job.write(|_, _| Ok(())).unwrap();
    job.complete(&mut s, uid);
    assert!(node(&s, id).projection().is_some(), "projected again");
    let st = s.doc().unwrap();
    assert_eq!(st.doc.metadata.modified, Some(777));
    assert!(!st.is_dirty());
    s.execute("perspective.editText", &json!({})).unwrap();
    let plan = save_plan(&s, SaveMode::Save, &json!({"modified": 778})).unwrap();
    save_with(&mut s, plan, |_, _| Ok(())).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("text.setText", &json!({"id": id.0, "text": "HELLO WORLD"})).unwrap();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert!(s.doc().unwrap().is_dirty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
}

#[test]
fn symbols_in_perspective_stay_instances() {
    let (mut s, rect) = persp_session();
    s.execute("perspective.release", &json!({"ids": [rect.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [rect.0]})).unwrap();
    let sym = s.execute("symbol.new", &json!({"name": "Box"})).unwrap();
    let _ = sym;
    let r = s.execute("symbol.place", &json!({"name": "Box", "x": 470, "y": 380})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [id.0], "plane": "right"})).unwrap();
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::SymbolInstance { .. }));
    assert!(n.projection().is_some());
    let baked = vectorcraft_render::effects::bake_document(&s.doc().unwrap().doc).unwrap();
    assert!(!matches!(baked.node(id).unwrap().kind, NodeKind::SymbolInstance { .. }), "exports draw the projected art");
}

#[test]
fn copies_pastes_and_duplicates_stay_attached() {
    let (mut s, id) = persp_session();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let pasted = *s.doc().unwrap().selection.objects.first().unwrap();
    assert_ne!(pasted, id);
    assert_eq!(attached(&s, pasted), Some((Plane::Right, 0.0)));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.transform", &json!({"matrix": [1, 0, 0, 1, 30, 0], "copy": true})).unwrap();
    let copy = *s.doc().unwrap().selection.objects.first().unwrap();
    assert_eq!(attached(&s, copy), Some((Plane::Right, 0.0)));
    // Deleting leaves nothing stale behind.
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(grid(&s).attached_plane(copy).is_none());
}

#[test]
fn a_selection_tool_move_keeps_the_attachment_and_type_follows() {
    let (mut s, rect) = persp_session();
    let id = persp_text(&mut s);
    let before = drawn_bounds(&s, id);
    s.execute("select.set", &json!({"ids": [id.0, rect.0]})).unwrap();
    let c = node(&s, rect).geometric_bounds().unwrap().center();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    drag(&mut s, &[c, Point::new(c.x + 25.0, c.y + 10.0), Point::new(c.x + 25.0, c.y + 10.0)], Mods::default());
    assert_eq!(attached(&s, rect), Some((Plane::Right, 0.0)));
    assert_eq!(attached(&s, id), Some((Plane::Right, 0.0)));
    let after = drawn_bounds(&s, id);
    assert!((after.x0 - before.x0 - 25.0).abs() < 1e-6 && (after.y0 - before.y0 - 10.0).abs() < 1e-6, "{before:?} {after:?}");
}

#[test]
fn release_keeps_the_look_of_type_in_perspective() {
    let (mut s, _) = persp_session();
    let id = persp_text(&mut s);
    let b = drawn_bounds(&s, id);
    s.execute("perspective.release", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(attached(&s, id), None);
    assert_eq!(drawn_bounds(&s, id), b);
    assert!(persp::attachment(&node(&s, id)).is_none());
}

#[test]
fn flare_spiral_and_polar_grid_draw_in_perspective() {
    let (mut s, _) = persp_session();
    s.execute("perspective.grid.show", &json!({"visible": true})).unwrap();
    s.execute("perspective.plane.set", &json!({"plane": "left"})).unwrap();
    for (tool, pts) in [("spiral", [(300.0, 380.0), (320.0, 390.0), (330.0, 400.0)]), ("polarGrid", [(250.0, 350.0), (280.0, 380.0), (300.0, 410.0)])]
    {
        let n = undo_len(&s);
        s.select_tool(tool, ViewInfo::default()).unwrap();
        drag(&mut s, &pts.map(|(x, y)| Point::new(x, y)), Mods::default());
        assert_eq!(undo_len(&s), n + 1, "{tool}");
        assert_eq!(s.journal.last().unwrap().0, "perspective.draw", "{tool}");
        let id = *s.doc().unwrap().selection.objects.first().unwrap();
        assert_eq!(attached(&s, id).map(|a| a.0), Some(Plane::Left), "{tool}");
    }
    let r = s.execute("perspective.draw", &json!({"command": "shape.flare", "params": {"cx": 300, "cy": 380, "diameter": 40}})).unwrap();
    assert_eq!(attached(&s, NodeId(r["id"].as_u64().unwrap())).map(|a| a.0), Some(Plane::Left));
}

#[test]
fn click_to_size_dialogs_draw_on_the_active_plane_in_plane_units() {
    let (mut s, _) = persp_session();
    s.execute("perspective.grid.show", &json!({"visible": true})).unwrap();
    s.execute("perspective.plane.set", &json!({"plane": "right"})).unwrap();
    let at = Point::new(470.0, 300.0);
    let (cmd, p) = crate::perspective_click(&s, "shape.rectangle", &json!({"x": at.x, "y": at.y, "width": 50, "height": 30}), at).unwrap();
    assert_eq!(cmd, "perspective.draw");
    let r = s.execute(&cmd, &p).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let g = grid(&s);
    let hi = g.homography(Plane::Right).unwrap().inverse().unwrap();
    let q: Vec<Point> = node(&s, id).path_data().unwrap().anchors().map(|(_, _, a)| hi.apply(a.p).unwrap()).collect();
    let b = q.iter().skip(1).fold(Rect::from_points(q[0], q[0]), |r, p| r.union_pt(*p));
    assert!((b.width() - 50.0).abs() < 1e-6 && (b.height() - 30.0).abs() < 1e-6, "{b:?}");
    assert!(q.iter().any(|p| (g.homography(Plane::Right).unwrap().apply(*p).unwrap() - at).hypot() < 1e-6), "the click stays a corner");
    // Without a shown grid (or with no active plane) nothing changes.
    s.execute("perspective.plane.set", &json!({"plane": "none"})).unwrap();
    assert!(crate::perspective_click(&s, "shape.rectangle", &json!({}), at).is_none());
}

#[test]
fn the_canvas_draws_type_in_perspective_foreshortened() {
    let (mut s, rect) = persp_session();
    s.execute("edit.clear", &json!({"ids": [rect.0]})).ok();
    let id = persp_text(&mut s);
    let b = drawn_bounds(&s, id);
    let img = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, b, 2.0, true);
    // The inked height of a strip of columns near each end of the line.
    let height = |x0: f64, x1: f64| {
        let (mut lo, mut hi) = (u32::MAX, 0);
        for x in (x0 * img.width as f64) as u32..(x1 * img.width as f64) as u32 {
            for y in 0..img.height {
                if img.pixel(x, y)[0] < 128 {
                    lo = lo.min(y);
                    hi = hi.max(y);
                }
            }
        }
        hi.saturating_sub(lo)
    };
    let (near, far) = (height(0.0, 0.25), height(0.75, 1.0));
    assert!(near > 10 && far * 100 < near * 97, "near {near} far {far}");
}

#[test]
fn double_clicking_type_in_perspective_edits_it_and_projections_round_trip() {
    let (mut s, _) = persp_session();
    let id = persp_text(&mut s);
    // Saved and opened again: still type, still projected.
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(doc.node(id).unwrap().projection(), node(&s, id).projection());
    let v = ViewInfo::default();
    s.select_tool("perspectiveSelection", v).unwrap();
    let c = node(&s, id).geometric_bounds().unwrap().center();
    let r = s.pointer(&vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::DoubleClick, c.x, c.y), v).unwrap();
    assert_eq!(r, vec![crate::UiRequest::SwitchTool("type".into())]);
    assert_eq!(s.doc().unwrap().isolation, Some(id));
    // Saved while editing: the file has it projected.
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert!(doc.node(id).unwrap().projection().is_some());
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert!(node(&s, id).projection().is_some());
}
