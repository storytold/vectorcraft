//! Distortion tools: Width (Shift+W), the Liquify family (Warp Shift+R, Twirl, Pucker, Bloat,
//! Scallop, Crystallize, Wrinkle), Puppet Warp, Perspective Grid (Shift+P) and Perspective
//! Selection (Shift+V).
//!
//! Like every tool they only emit actions. The geometry kernels the engine commands run live here
//! too (so tools, engine and tests share one implementation):
//! - [`liquify`]: deterministic brush deformation of anchors and handles with adaptive subdivision;
//! - [`arap`]: As-Rigid-As-Possible mesh deformation (Igarashi, Moscovich & Hughes 2005) with a
//!   small banded Cholesky solver;
//! - [`perspective`]: the perspective grid model (one/two/three-point) and plane homographies;
//! - [`pathutil`]: arc-length helpers for width points.

pub mod arap;
pub mod liquify;
pub mod pathutil;
pub mod perspective;
mod puppet;
mod width;

use std::sync::Arc;

use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Homography, Point};

use crate::Tool;

pub use liquify::LiquifyTool;
pub use perspective::{PerspectiveGridTool, PerspectiveSelectionTool};
pub use puppet::{PinSet, PuppetWarpTool, mesh_for, mesh_input, rest_and_pins, warp_from_rest};
pub use width::{WidthTool, stroke_owner};

/// Create a distortion tool by id (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "width" => Box::new(WidthTool::default()),
        "warp" | "twirl" | "pucker" | "bloat" | "scallop" | "crystallize" | "wrinkle" => Box::new(LiquifyTool::new(id)),
        "puppetWarp" => Box::new(PuppetWarpTool::default()),
        "perspectiveGrid" => Box::new(PerspectiveGridTool::default()),
        "perspectiveSelection" => Box::new(PerspectiveSelectionTool::default()),
        _ => return None,
    })
}

/// Apply a point map to every anchor and handle of `n` and its descendants. Gradients (pinned
/// first if unplaced) follow the map's affine approximation at their centre; objects without
/// editable points (text, images, symbols, meshes, live objects) get the approximation at their
/// centre. Live shapes become plain paths.
pub fn warp_node_with(n: &mut Node, f: &dyn Fn(Point) -> Point) {
    warp_node(n, f, None);
}

/// [`warp_node_with`] for the projective map `h` (perspective): type and symbol instances keep
/// their flat art and are drawn through `h` (composed with the projection they had). Returns
/// `false`, and leaves `n` as it is, when the projection of a type or symbol instance in `n` would
/// have an entry that is not finite.
pub fn project_node(n: &mut Node, h: &Homography) -> bool {
    if !projects_finitely(n, h) {
        return false;
    }
    warp_node(n, &|p| h.apply(p).unwrap_or(p), Some(h));
    true
}

/// Whether every type and symbol instance that [`project_node`] reaches in `n` keeps a projection
/// whose entries are all finite.
fn projects_finitely(n: &Node, h: &Homography) -> bool {
    match &n.kind {
        NodeKind::Text(_) | NodeKind::SymbolInstance { .. } => {
            let had = n.extras().perspective.as_deref().and_then(|p| p.homography()).unwrap_or(Homography::IDENTITY);
            h.checked_then_after(&had).is_some()
        }
        NodeKind::Layer { children, .. }
        | NodeKind::Group { children, .. }
        | NodeKind::Compound { children, .. }
        | NodeKind::Blend { children, .. } => children.iter().all(|c| projects_finitely(c, h)),
        _ => true,
    }
}

fn warp_node(n: &mut Node, f: &dyn Fn(Point) -> Point, h: Option<&Homography>) {
    if let Some(h) = h
        && matches!(n.kind, NodeKind::Text(_) | NodeKind::SymbolInstance { .. })
    {
        let mut extras = n.extras_mut();
        let rec = extras.perspective.get_or_insert_with(Default::default);
        let had = rec.homography().unwrap_or(Homography::IDENTITY);
        if let Some(c) = h.checked_then_after(&had) {
            rec.projection = Some(c.to_array());
        }
        return;
    }
    let bounds = n.geometric_bounds();
    // Finite-difference step: 5% of the object (at least half a point).
    let eps = bounds.map_or(0.5, |b| (b.width().max(b.height()) * 0.05).max(0.5));
    n.pin_gradients();
    match &mut n.kind {
        NodeKind::Path { path, live, .. } => {
            *live = None;
            for sp in &mut path.subpaths {
                for a in &mut sp.anchors {
                    a.p = f(a.p);
                    a.h_in = f(a.h_in);
                    a.h_out = f(a.h_out);
                }
            }
        }
        NodeKind::Layer { children, .. }
        | NodeKind::Group { children, .. }
        | NodeKind::Compound { children, .. }
        | NodeKind::Blend { children, .. } => {
            for c in children.iter_mut() {
                warp_node(Arc::make_mut(c), f, h);
            }
        }
        _ => {
            if let Some(b) = bounds {
                n.transform(affine_near(f, b.center(), eps), false);
            }
            return;
        }
    }
    n.appearance.warp_gradients(&|p| affine_near(f, p, eps));
}

pub use vectorcraft_doc::live::affine_near;

/// Every anchor and handle position in `n` (for meshes and bounds).
pub fn collect_points(n: &Node, out: &mut Vec<Point>) {
    n.walk(&mut |c| match &c.kind {
        NodeKind::Path { path, .. } => {
            for sp in &path.subpaths {
                for a in &sp.anchors {
                    out.extend([a.p, a.h_in, a.h_out]);
                }
            }
        }
        NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Compound { .. } | NodeKind::Blend { .. } => {}
        _ => {
            if let Some(b) = c.geometric_bounds() {
                out.extend([Point::new(b.x0, b.y0), Point::new(b.x1, b.y1)]);
            }
        }
    });
}

/// Selection blue used by the distortion tool overlays.
pub(crate) const BLUE: [u8; 3] = [0x4a, 0x7c, 0xff];

/// A small diamond polygon at `p` (half-diagonal `r`).
pub(crate) fn diamond(p: Point, r: f64) -> vectorcraft_geom::BezPath {
    crate::xform::polygon(&[Point::new(p.x, p.y - r), Point::new(p.x + r, p.y), Point::new(p.x, p.y + r), Point::new(p.x - r, p.y)], true)
}

/// A circle / ellipse outline for brush feedback.
pub(crate) fn ellipse_path(c: Point, rx: f64, ry: f64, angle_deg: f64) -> vectorcraft_geom::BezPath {
    use vectorcraft_geom::Shape;
    let e = vectorcraft_geom::kurbo::Ellipse::new(c, (rx.max(0.01), ry.max(0.01)), angle_deg.to_radians());
    e.to_path(0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::{Gradient, GradientPaint, Paint};
    use vectorcraft_doc::Appearance;
    use vectorcraft_geom::{Affine, PathData, Rect, shapes};

    #[test]
    fn create_covers_all_distort_tools() {
        for id in [
            "width",
            "warp",
            "twirl",
            "pucker",
            "bloat",
            "scallop",
            "crystallize",
            "wrinkle",
            "puppetWarp",
            "perspectiveGrid",
            "perspectiveSelection",
        ] {
            assert_eq!(create(id).map(|t| t.id()), Some(id));
        }
        assert!(create("pen").is_none());
    }

    #[test]
    fn warp_node_maps_all_points_and_drops_live_shape() {
        let mut n = Node::path(vectorcraft_doc::NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        warp_node_with(&mut n, &|p| Point::new(p.x * 2.0, p.y + 1.0));
        let b = n.geometric_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 1.0, 20.0, 11.0));
        if let NodeKind::Path { live, .. } = &n.kind {
            assert!(live.is_none());
        }
        let _ = PathData::default();
    }

    #[test]
    fn warp_pins_and_maps_gradients_once() {
        let grad = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let mut n =
            Node::path(vectorcraft_doc::NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(grad, Paint::None, 0.0));
        warp_node_with(&mut n, &|p| Point::new(p.x * 2.0, p.y + 1.0));
        let Paint::Gradient(g) = n.appearance.fill_paint() else { panic!() };
        let geom = g.geom.expect("pinned before the warp");
        assert!(geom.start.distance(Point::new(0.0, 6.0)) < 1e-9 && geom.end.distance(Point::new(20.0, 6.0)) < 1e-9, "{geom:?}");
    }

    #[test]
    fn affine_near_recovers_affine_maps() {
        let a = Affine::new([2.0, 0.5, -0.3, 1.5, 10.0, -4.0]);
        let got = affine_near(&|p| a * p, Point::new(3.0, 7.0), 0.5);
        for (x, y) in a.as_coeffs().iter().zip(got.as_coeffs()) {
            assert!((x - y).abs() < 1e-9);
        }
    }
}
