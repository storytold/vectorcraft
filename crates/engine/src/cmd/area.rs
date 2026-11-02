//! The area of the selection, shown in the Info panel: the bounding box's area (the sheet a print
//! needs: width × height) and the area the selected closed shapes enclose (what is printed or cut,
//! with holes left out and overlaps counted once).
//!
//! **Fill rule.** Each path and compound path is first filled under its own fill rule (non-zero or
//! even-odd), so a hole stays a hole; the filled regions are then united, so overlaps count once.
//!
//! **Precision.** Curves are not flattened: the area of a Bézier outline is integrated exactly from
//! its control points (Green's theorem). The only approximation is the union, which refits the
//! curve pieces it splits at intersections to [`vectorcraft_pathops::DEFAULT_PRECISION`]
//! (0.01 pt, ≈ 0.0035 mm) and drops slivers thinner than that. The error is therefore at most about
//! 0.01 pt × the outline's length: under 0.15 cm² for a 1 m × 1 m sign, far below the third
//! decimal of the m² the Info panel shows. A single shape is measured without the union, exactly.
//! An ellipse is measured as the four Bézier arcs the document stores (0.03 % above π r²).
//!
//! **Cost.** Uniting is O(n log n) in the segments; the measurement stops at [`MAX_SHAPES`] shapes
//! or [`MAX_ANCHORS`] anchors and then reports only the bounding box. The Info panel measures once
//! per change of the document or the selection.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{FillRule, PathData};

use super::{CommandSpec, Result, Session, checked_ids_or_selection, cmd, has_selection};

/// Square metres in one square point (1 pt = 1/72 in = 0.0254/72 m).
pub const SQUARE_METRES_PER_SQUARE_POINT: f64 = (0.0254 / 72.0) * (0.0254 / 72.0);

/// Paths are measured up to this many; a larger selection reports only its bounding box, so a
/// selection of a whole map can't stall the panel.
pub const MAX_SHAPES: usize = 20_000;

/// Anchors measured at most, across all shapes: one path with millions of points is as costly as
/// many shapes.
pub const MAX_ANCHORS: usize = 200_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "selection.area",
        "Area",
        [],
        None,
        "{ids?} → {boxArea, shapeArea, shapes, width, height, unit: \"pt²\", squareMetres: {box, shape}}: boxArea is the selection's bounding box (width × height, strokes left out); shapeArea the area its closed paths and compound paths enclose, united (holes left out, overlaps counted once). Images, placed files, symbols and text count their bounds; open paths, guides and clipping paths count nothing, and a clip group counts its clipping path. shapeArea is null past 20,000 shapes or 200,000 anchors, or when the geometry can't be united. Curves are integrated exactly; the union refits at 0.01 pt",
        has_selection,
        area
    )]
}

/// Area of `ids` in `doc`, in square points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Area {
    /// Bounding box width and height.
    pub width: f64,
    pub height: f64,
    /// `width × height`.
    pub bounds: f64,
    /// Enclosed area of the closed shapes, united; `None` past [`MAX_SHAPES`] or [`MAX_ANCHORS`], or
    /// when the union refuses the geometry (non-finite or too complex), never a silent zero.
    pub shape: Option<f64>,
    /// The shapes measured.
    pub shapes: usize,
}

/// Measure `ids` (see [`Area`]); `None` when they have no extent.
pub fn measure(doc: &Document, ids: &[NodeId]) -> Option<Area> {
    let b = doc.bounds_of(ids, false)?;
    let (width, height) = (b.width(), b.height());
    if !(width.is_finite() && height.is_finite()) {
        return None;
    }
    let mut found = Shapes::default();
    for n in ids.iter().filter_map(|id| doc.node(*id)) {
        collect(n, &mut found);
        if found.over {
            break;
        }
    }
    let shape = if found.over { None } else { enclosed(&found.list) };
    Some(Area { width, height, bounds: width * height, shape, shapes: found.list.len() })
}

/// Area the shapes enclose, each under its own fill rule, united; `None` when the geometry is refused.
fn enclosed(shapes: &[(PathData, FillRule)]) -> Option<f64> {
    let refs: Vec<(&PathData, FillRule)> = shapes.iter().map(|(p, r)| (p, *r)).collect();
    let a = match refs.as_slice() {
        [] => Ok(0.0),
        [(p, r)] => vectorcraft_pathops::try_area(p, *r),
        _ => vectorcraft_pathops::try_unite_all(&refs).and_then(|u| vectorcraft_pathops::try_area(&u, FillRule::NonZero)),
    };
    a.ok().filter(|a| a.is_finite())
}

/// The shapes collected so far and whether a limit was passed.
#[derive(Default)]
struct Shapes {
    list: Vec<(PathData, FillRule)>,
    anchors: usize,
    over: bool,
}

/// Only the closed subpaths of `p`: an open path encloses nothing.
fn closed_only(p: &PathData) -> Option<PathData> {
    let subpaths: Vec<_> = p.subpaths.iter().filter(|sp| sp.closed && sp.anchors.len() > 2).cloned().collect();
    (!subpaths.is_empty()).then(|| PathData::new(subpaths))
}

fn push(out: &mut Shapes, shape: (PathData, FillRule)) {
    let anchors = out.anchors.saturating_add(shape.0.subpaths.iter().map(|sp| sp.anchors.len()).sum());
    if out.list.len() >= MAX_SHAPES || anchors > MAX_ANCHORS {
        out.over = true;
    } else {
        out.anchors = anchors;
        out.list.push(shape);
    }
}

fn collect(n: &Node, out: &mut Shapes) {
    if out.over || !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Path { path, rule, clipping, guide, .. } => {
            if !clipping
                && !guide
                && let Some(p) = closed_only(path)
            {
                push(out, (p, *rule));
            }
        }
        NodeKind::Compound { children, rule } => {
            // The compound's subpaths fill together, so its holes stay holes.
            let subpaths: Vec<_> = children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect();
            if let Some(p) = closed_only(&PathData::new(subpaths)) {
                push(out, (p, *rule));
            }
        }
        NodeKind::Group { children, clip: true } => {
            // What shows of a clip group lies inside its clipping path.
            if let Some(p) = children.first().and_then(|c| c.path_data()).and_then(closed_only) {
                push(out, (p, FillRule::NonZero));
            }
        }
        NodeKind::Layer { .. } | NodeKind::Group { .. } => {
            for c in n.children().into_iter().flatten() {
                collect(c, out);
            }
        }
        _ => {
            if let Some(b) = n.geometric_bounds()
                && b.width() > 0.0
                && b.height() > 0.0
            {
                push(out, (vectorcraft_geom::shapes::rectangle(b), FillRule::NonZero));
            }
        }
    }
}

fn area(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "selection.area";
    let ids = checked_ids_or_selection(s, p, C)?;
    let st = s.doc()?;
    // Objects without extent (an empty group) measure nothing.
    let a = measure(&st.doc, &ids).unwrap_or_default();
    Ok(json!({
        "width": a.width,
        "height": a.height,
        "boxArea": a.bounds,
        "shapeArea": a.shape,
        "shapes": a.shapes,
        "unit": "pt²",
        "squareMetres": {
            "box": a.bounds * SQUARE_METRES_PER_SQUARE_POINT,
            "shape": a.shape.map(|v| v * SQUARE_METRES_PER_SQUARE_POINT),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 5000, "height": 5000})).unwrap();
        s
    }

    fn id(v: &Value) -> u64 {
        v["id"].as_u64().unwrap()
    }

    #[test]
    fn a_rectangle_measures_its_box_and_its_shape() {
        let mut s = session();
        let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 100})).unwrap();
        let a = s.execute("selection.area", &json!({"ids": [id(&r)]})).unwrap();
        assert!((a["boxArea"].as_f64().unwrap() - 20_000.0).abs() < 1e-6, "{a}");
        assert!((a["shapeArea"].as_f64().unwrap() - 20_000.0).abs() < 1.0, "{a}");
        assert_eq!(a["unit"], "pt²");
    }

    #[test]
    fn an_ellipse_encloses_less_than_its_box() {
        let mut s = session();
        let e = s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
        let a = s.execute("selection.area", &json!({"ids": [id(&e)]})).unwrap();
        let shape = a["shapeArea"].as_f64().unwrap();
        // π r² with r = 100, within the Bézier circle's error.
        assert!((shape - std::f64::consts::PI * 10_000.0).abs() < 50.0, "{shape}");
        assert!((a["boxArea"].as_f64().unwrap() - 40_000.0).abs() < 1e-6);
    }

    #[test]
    fn overlapping_shapes_count_once_and_square_metres_follow() {
        let mut s = session();
        // Two 1 m × 1 m squares (2834.6 pt), overlapping by half: 1.5 m² of shape, a 1.5 m × 1 m box.
        let m = 72.0 / 0.0254;
        let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": m, "height": m})).unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": m / 2.0, "y": 0, "width": m, "height": m})).unwrap();
        let r = s.execute("selection.area", &json!({"ids": [id(&a), id(&b)]})).unwrap();
        assert!((r["squareMetres"]["shape"].as_f64().unwrap() - 1.5).abs() < 1e-3, "{r}");
        assert!((r["squareMetres"]["box"].as_f64().unwrap() - 1.5).abs() < 1e-6, "{r}");
        assert_eq!(r["shapes"], 2);
    }

    #[test]
    fn open_paths_enclose_nothing() {
        let mut s = session();
        let l = s.execute("shape.line", &json!({"x1": 0, "y1": 0, "x2": 100, "y2": 100})).unwrap();
        let r = s.execute("selection.area", &json!({"ids": [id(&l)]})).unwrap();
        assert_eq!(r["shapeArea"].as_f64().unwrap(), 0.0, "{r}");
        assert_eq!(r["shapes"], 0);
    }

    /// Two nested squares drawn the same way round: non-zero fills the hole, even-odd leaves it.
    #[test]
    fn the_fill_rule_decides_whether_an_inner_contour_is_a_hole() {
        let mut s = session();
        let d = "M0 0 L300 0 L300 300 L0 300 Z M100 100 L200 100 L200 200 L100 200 Z";
        let p = s.execute("path.create", &json!({"d": d})).unwrap();
        let nonzero = s.execute("selection.area", &json!({"ids": [id(&p)]})).unwrap();
        assert!((nonzero["shapeArea"].as_f64().unwrap() - 90_000.0).abs() < 1e-6, "{nonzero}");
        s.execute("path.setFillRule", &json!({"ids": [id(&p)], "rule": "evenOdd"})).unwrap();
        let evenodd = s.execute("selection.area", &json!({"ids": [id(&p)]})).unwrap();
        assert!((evenodd["shapeArea"].as_f64().unwrap() - 80_000.0).abs() < 1e-6, "{evenodd}");
        // The same ring next to a square that covers its hole: the union counts the hole as covered.
        let q = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let both = s.execute("selection.area", &json!({"ids": [id(&p), id(&q)]})).unwrap();
        assert!((both["shapeArea"].as_f64().unwrap() - 90_000.0).abs() < 1.0, "{both}");
    }

    /// Overlapping circles: the union refits the arcs it splits, and stays within the precision
    /// the module documents (0.01 pt × outline length).
    #[test]
    fn a_union_of_curves_stays_within_the_documented_precision() {
        let mut s = session();
        let a = s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
        let b = s.execute("shape.ellipse", &json!({"x": 100, "y": 0, "width": 200, "height": 200})).unwrap();
        let one = s.execute("selection.area", &json!({"ids": [id(&a)]})).unwrap()["shapeArea"].as_f64().unwrap();
        let both = s.execute("selection.area", &json!({"ids": [id(&a), id(&b)]})).unwrap()["shapeArea"].as_f64().unwrap();
        // Two equal circles with centres one radius apart overlap by r²(2π/3 − √3/2).
        let lens = 10_000.0 * (2.0 * std::f64::consts::PI / 3.0 - 3f64.sqrt() / 2.0);
        let outline = 2.0 * 2.0 * std::f64::consts::PI * 100.0;
        assert!((both - (2.0 * one - lens)).abs() < 0.01 * outline + 0.003 * lens, "{both} vs {}", 2.0 * one - lens);
    }

    #[test]
    fn a_selection_past_the_limits_reports_only_its_box() {
        let mut shapes = Shapes::default();
        let square = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 1.0, 1.0));
        for _ in 0..=MAX_SHAPES {
            push(&mut shapes, (square.clone(), FillRule::NonZero));
        }
        assert!(shapes.over && shapes.list.len() == MAX_SHAPES);
        let mut many = Shapes::default();
        let anchors = (0..=MAX_ANCHORS).map(|i| vectorcraft_geom::Anchor::corner(vectorcraft_geom::Point::new(i as f64, (i % 2) as f64))).collect();
        push(&mut many, (PathData::new(vec![vectorcraft_geom::SubPath::new(anchors, true)]), FillRule::NonZero));
        assert!(many.over && many.list.is_empty());
    }

    #[test]
    fn refused_geometry_reports_no_shape_area_instead_of_zero() {
        use vectorcraft_geom::{Anchor, Point, SubPath};
        let corner = |x: f64, y: f64| Anchor::corner(Point::new(x, y));
        let bad = PathData::new(vec![SubPath::new(vec![corner(0.0, 0.0), corner(f64::NAN, 0.0), corner(1.0, 1.0)], true)]);
        let good = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 1.0, 1.0));
        assert_eq!(enclosed(&[(bad.clone(), FillRule::NonZero)]), None);
        assert_eq!(enclosed(&[(good, FillRule::NonZero), (bad, FillRule::NonZero)]), None);
    }
}
