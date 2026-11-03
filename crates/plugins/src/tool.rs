//! Context handed to sandboxed tool plug-ins.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveShape, Node, NodeKind};
use vectorcraft_geom::{Affine, ParamCurve, Point};

const MAX_PATH_SEGMENTS: usize = 4_096;

#[derive(Clone, Copy, Debug)]
struct Circle {
    center: Point,
    radius: f64,
}

/// Visible, editable circles for a tool plug-in, bottom first. Each is `{id, center, radius}`.
pub fn circles(doc: &Document, limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    if limit == 0 {
        return out;
    }
    let locks = doc.locked_artboards();
    doc.walk(|n| {
        if out.len() >= limit || !doc.is_editable_with(&locks, n.id) {
            return;
        }
        if doc.ancestry(n.id).is_some_and(|a| a.iter().any(|id| doc.node(*id).is_some_and(Node::is_template))) {
            return;
        }
        let NodeKind::Path { guide, clipping, .. } = &n.kind else { return };
        if *guide || *clipping {
            return;
        }
        let Some(circle) = circle_of(n) else { return };
        out.push(json!({
            "id": n.id.0,
            "center": [circle.center.x, circle.center.y],
            "radius": circle.radius,
        }));
    });
    out
}

fn circle_of(node: &Node) -> Option<Circle> {
    if let Some(live) = node.live_shape() {
        let LiveShape::Ellipse { w, h, pie, xf } = live else { return None };
        return live_circle(*w, *h, *pie, *xf);
    }
    path_circle(node.path_data()?)
}

fn live_circle(w: f64, h: f64, pie: (f64, f64), xf: Affine) -> Option<Circle> {
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) || (w - h).abs() > 1e-9 * w.max(h) {
        return None;
    }
    let turn = (pie.1 - pie.0).rem_euclid(360.0);
    if turn > 1e-9 && (360.0 - turn).abs() > 1e-9 {
        return None;
    }
    let [a, b, c, d, ..] = xf.as_coeffs();
    let (sx, sy) = (a.hypot(b), c.hypot(d));
    if !(sx.is_finite() && sy.is_finite() && sx > 1e-12) || (sx - sy).abs() > 1e-9 * sx.max(sy) || (a * c + b * d).abs() > 1e-9 * sx * sy {
        return None;
    }
    let center = xf * Point::new(w / 2.0, h / 2.0);
    let radius = w / 2.0 * sx;
    finite_circle(center, radius)
}

fn path_circle(path: &vectorcraft_geom::PathData) -> Option<Circle> {
    let [sp] = path.subpaths.as_slice() else { return None };
    if !sp.closed || sp.anchors.len() < 4 || sp.segment_count() > MAX_PATH_SEGMENTS {
        return None;
    }
    let b = path.bounds()?;
    let radius = (b.width() + b.height()) / 4.0;
    if !(b.width().is_finite() && b.height().is_finite() && radius > 1e-6) || (b.width() - b.height()).abs() > 1e-9 * radius.max(1.0) {
        return None;
    }
    let center = b.center();
    let tolerance = radius * 1e-3 + 1e-9;
    if sp.anchors.iter().any(|a| (a.p.distance(center) - radius).abs() > tolerance) {
        return None;
    }
    for i in 0..sp.segment_count() {
        let seg = sp.segment(i);
        for t in [0.25, 0.5, 0.75] {
            if (seg.eval(t).distance(center) - radius).abs() > tolerance {
                return None;
            }
        }
    }
    finite_circle(center, radius)
}

fn finite_circle(center: Point, radius: f64) -> Option<Circle> {
    (center.x.is_finite() && center.y.is_finite() && radius.is_finite() && radius > 0.0).then_some(Circle { center, radius })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, NodeId};
    use vectorcraft_geom::{Rect, shapes};

    fn live_circle(d: &mut Document, center: Point, radius: f64) -> NodeId {
        let id = d.alloc_id();
        let live =
            LiveShape::Ellipse { w: radius * 2.0, h: radius * 2.0, pie: (0.0, 360.0), xf: Affine::translate((center.x - radius, center.y - radius)) };
        let mut n = Node::path(id, live.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(Box::new(live));
        }
        d.insert(Some(d.layers[0].id), 0, n).unwrap();
        id
    }

    fn plain_circle(d: &mut Document, center: Point, radius: f64) -> NodeId {
        let id = d.alloc_id();
        let path = shapes::ellipse(Rect::from_center_size(center, (radius * 2.0, radius * 2.0)));
        d.insert(Some(d.layers[0].id), 0, Node::path(id, path, Appearance::default_art())).unwrap();
        id
    }

    #[test]
    fn circle_context_recognizes_live_and_plain_circles_but_not_ellipses() {
        let mut d = Document::new(400.0, 400.0);
        let live = live_circle(&mut d, Point::new(50.0, 60.0), 20.0);
        let plain = plain_circle(&mut d, Point::new(150.0, 60.0), 30.0);
        let oval = d.alloc_id();
        let path = shapes::ellipse(Rect::new(220.0, 30.0, 300.0, 90.0));
        d.insert(Some(d.layers[0].id), 0, Node::path(oval, path, Appearance::default_art())).unwrap();
        let got = circles(&d, 20);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!((&got[0]["id"], &got[0]["center"], &got[0]["radius"]), (&json!(plain.0), &json!([150.0, 60.0]), &json!(30.0)));
        assert_eq!((&got[1]["id"], &got[1]["center"], &got[1]["radius"]), (&json!(live.0), &json!([50.0, 60.0]), &json!(20.0)));
    }

    #[test]
    fn circle_context_skips_hidden_locked_guide_and_clipping_art_and_stops_at_the_limit() {
        let mut d = Document::new(600.0, 400.0);
        let hidden = live_circle(&mut d, Point::new(50.0, 60.0), 10.0);
        let locked = live_circle(&mut d, Point::new(150.0, 60.0), 10.0);
        let guide = live_circle(&mut d, Point::new(250.0, 60.0), 10.0);
        let clipping = live_circle(&mut d, Point::new(350.0, 60.0), 10.0);
        let visible = live_circle(&mut d, Point::new(450.0, 60.0), 10.0);
        d.node_mut(hidden).unwrap().visible = false;
        d.node_mut(locked).unwrap().locked = true;
        for (id, is_guide) in [(guide, true), (clipping, false)] {
            let NodeKind::Path { guide, clipping, .. } = &mut d.node_mut(id).unwrap().kind else { panic!("path") };
            if is_guide {
                *guide = true;
            } else {
                *clipping = true;
            }
        }
        assert_eq!(circles(&d, 30).iter().map(|v| v["id"].clone()).collect::<Vec<_>>(), vec![json!(visible.0)]);
        let none: Vec<Value> = vec![];
        assert_eq!(circles(&d, 0), none);
    }

    #[test]
    fn a_template_layer_gives_no_circle_context() {
        let mut d = Document::new(400.0, 400.0);
        let layer = d.layers[0].id;
        if let NodeKind::Layer { template, .. } = &mut Arc::make_mut(&mut d.layers[0]).kind {
            *template = true;
        }
        live_circle(&mut d, Point::new(50.0, 60.0), 20.0);
        if let NodeKind::Layer { children, .. } = &mut Arc::make_mut(&mut d.layers[0]).kind {
            assert_eq!(children.len(), 1, "fixture is on the template layer");
        }
        assert!(circles(&d, 10).is_empty(), "layer {layer:?}");
    }
}
