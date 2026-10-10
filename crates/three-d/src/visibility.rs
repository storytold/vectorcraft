//! Vector visibility in the evaluator's paint order. Convex projected faces are cut by the
//! nearer faces covering them, irrespective of their winding or which side faces the camera.
//! This preserves interiors visible through open ends and partial sweeps.

use vectorcraft_geom::{PathData, Point, Rect, SubPath};

use crate::{Face, MAX_FACES};

const GRID: usize = 32;
const MAX_GRID_REFS: usize = 2_000_000;
const MAX_WORK: usize = 16_000_000;
const MAX_FRAGMENTS: usize = 256;
const MAX_OUTPUT_POINTS: usize = 262_144;

/// The visible portions of one source face, with its original shading and paint order.
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleFace {
    pub path: PathData,
    pub brightness: f64,
    pub depth: f64,
}

struct Polygon {
    points: Vec<Point>,
    bounds: Rect,
}

fn bounds(points: &[Point]) -> Rect {
    let Some(first) = points.first() else { return Rect::ZERO };
    points.iter().fold(Rect::from_points(*first, *first), |b, p| b.union_pt(*p))
}

fn area(points: &[Point]) -> f64 {
    let Some(origin) = points.first() else { return 0.0 };
    points.iter().zip(points.iter().cycle().skip(1)).take(points.len()).map(|(a, b)| (*a - *origin).cross(*b - *origin)).sum::<f64>() / 2.0
}

fn clean(mut points: Vec<Point>, epsilon: f64) -> Vec<Point> {
    points.dedup_by(|a, b| a.distance(*b) <= epsilon);
    if points.len() > 1 && points.first().zip(points.last()).is_some_and(|(a, b)| a.distance(*b) <= epsilon) {
        points.pop();
    }
    if points.len() < 3 || area(&points).abs() <= epsilon * epsilon {
        return vec![];
    }
    if area(&points) < 0.0 {
        points.reverse();
    }
    points
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// Split a convex polygon at an occluder's edge. Boundary vertices belong to both halves,
/// so independently clipped neighbours share precisely the same intersection positions.
fn split(points: &[Point], a: Point, b: Point, epsilon: f64) -> (Vec<Point>, Vec<Point>) {
    let edge = b - a;
    let mut inside = vec![];
    let mut outside = vec![];
    for (p, q) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len()) {
        let dp = edge.cross(*p - a);
        let dq = edge.cross(*q - a);
        if dp >= 0.0 {
            inside.push(*p);
        }
        if dp <= 0.0 {
            outside.push(*p);
        }
        if (dp > 0.0 && dq < 0.0) || (dp < 0.0 && dq > 0.0) {
            let intersection = p.lerp(*q, dp / (dp - dq));
            inside.push(intersection);
            outside.push(intersection);
        }
    }
    (clean(inside, epsilon), clean(outside, epsilon))
}

fn subtract(points: Vec<Point>, cover: &Polygon, epsilon: f64, work: &mut usize) -> Result<Vec<Vec<Point>>, String> {
    if !overlaps(bounds(&points), cover.bounds) {
        return Ok(vec![points]);
    }
    let mut remaining = points;
    let mut visible = vec![];
    for (a, b) in cover.points.iter().zip(cover.points.iter().cycle().skip(1)).take(cover.points.len()) {
        *work += remaining.len();
        if *work > MAX_WORK {
            return Err("Revolve visibility exceeds its geometry work limit".into());
        }
        let (inside, outside) = split(&remaining, *a, *b, epsilon);
        if !outside.is_empty() {
            visible.push(outside);
        }
        remaining = inside;
        if remaining.is_empty() {
            break;
        }
    }
    Ok(visible)
}

struct Index {
    cells: Vec<Vec<usize>>,
    bounds: Rect,
    refs: usize,
}

impl Index {
    fn tiles(&self, b: Rect) -> impl Iterator<Item = usize> {
        let cell = |v: f64, lo: f64, size: f64| (((v - lo) / size * GRID as f64).floor() as usize).min(GRID - 1);
        let x0 = cell(b.x0, self.bounds.x0, self.bounds.width());
        let x1 = cell(b.x1, self.bounds.x0, self.bounds.width());
        let y0 = cell(b.y0, self.bounds.y0, self.bounds.height());
        let y1 = cell(b.y1, self.bounds.y0, self.bounds.height());
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| y * GRID + x))
    }

    fn insert(&mut self, b: Rect, face: usize) -> Result<(), String> {
        let tiles: Vec<_> = self.tiles(b).collect();
        self.refs += tiles.len();
        if self.refs > MAX_GRID_REFS {
            return Err("Revolve visibility exceeds its spatial index limit".into());
        }
        for tile in tiles {
            if let Some(cell) = self.cells.get_mut(tile) {
                cell.push(face);
            }
        }
        Ok(())
    }
}

/// Remove covered portions of opaque faces in back-to-front paint order. This follows the
/// live evaluator's painter visibility, including its approximation for intersecting surfaces.
/// Resource limits return an error; callers can retain the original complete surface safely.
pub fn visible_faces(faces: &[Face]) -> Result<Vec<VisibleFace>, String> {
    if faces.len() > MAX_FACES {
        return Err("Too many Revolve faces for visibility evaluation".into());
    }
    if faces.iter().any(|f| {
        f.points.len() > 4
            || !f.brightness.is_finite()
            || !f.depth.is_finite()
            || f.points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs().max(p.y.abs()) > 1e8)
    }) {
        return Err("Revolve visibility requires finite projected convex faces".into());
    }
    let all = faces.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>();
    let total = bounds(&all);
    if total.width() <= 0.0 || total.height() <= 0.0 {
        return Ok(vec![]);
    }
    let epsilon = (total.width().max(total.height()) * 1e-10).clamp(1e-8, 1e-3);
    let polygons: Vec<_> = faces
        .iter()
        .map(|f| {
            let points = clean(f.points.clone(), epsilon);
            Polygon { bounds: bounds(&points), points }
        })
        .collect();
    // Generated quads are convex. Validate the public helper rather than accepting concave
    // callers and silently deleting geometry they can see.
    for p in &polygons {
        if p.points.iter().enumerate().any(|(i, a)| {
            let Some(b) = p.points.get((i + 1) % p.points.len()) else { return false };
            let Some(c) = p.points.get((i + 2) % p.points.len()) else { return false };
            (*b - *a).cross(*c - *b) < -epsilon * b.distance(*a).max(1.0)
        }) {
            return Err("Revolve visibility requires convex projected faces".into());
        }
    }
    let mut index = Index { cells: vec![vec![]; GRID * GRID], bounds: total, refs: 0 };
    let mut seen = vec![usize::MAX; faces.len()];
    let (mut out, mut work, mut output_points) = (vec![], 0, 0);
    for (source, (face, polygon)) in faces.iter().zip(&polygons).enumerate().rev() {
        if polygon.points.is_empty() {
            continue;
        }
        let mut fragments = vec![polygon.points.clone()];
        'covers: for tile in index.tiles(polygon.bounds) {
            for &candidate in index.cells.get(tile).into_iter().flatten() {
                work += 1;
                if work > MAX_WORK {
                    return Err("Revolve visibility exceeds its geometry work limit".into());
                }
                if let Some(mark) = seen.get_mut(candidate)
                    && *mark != source
                {
                    *mark = source;
                    if let Some(cover) = polygons.get(candidate)
                        && overlaps(polygon.bounds, cover.bounds)
                    {
                        let mut remaining = vec![];
                        for fragment in fragments {
                            remaining.extend(subtract(fragment, cover, epsilon, &mut work)?);
                            if remaining.len() > MAX_FRAGMENTS {
                                return Err("Revolve visibility creates too many fragments".into());
                            }
                        }
                        fragments = remaining;
                        if fragments.is_empty() {
                            break 'covers;
                        }
                    }
                }
            }
        }
        if fragments.is_empty() {
            continue;
        }
        output_points += fragments.iter().map(Vec::len).sum::<usize>();
        if output_points > MAX_OUTPUT_POINTS {
            return Err("Revolve visibility creates too many vector anchors".into());
        }
        let path = PathData::new(fragments.into_iter().map(|p| SubPath::polyline(&p, true)).collect());
        out.push(VisibleFace { path, brightness: face.brightness, depth: face.depth });
        // A completely covered face need not occupy the grid. Its coverage already belongs
        // to nearer faces; this keeps repeated overlapping surfaces bounded in practice.
        index.insert(polygon.bounds, source)?;
    }
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, Revolve, revolve};
    use vectorcraft_geom::{BezPath, Shape};

    fn face(rect: Rect, brightness: f64) -> Face {
        Face {
            points: vec![Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)],
            depth: brightness,
            brightness,
        }
    }

    #[test]
    fn covered_faces_disappear_and_partial_faces_are_trimmed_in_paint_order() {
        let faces =
            vec![face(Rect::new(2.0, 2.0, 8.0, 8.0), 0.1), face(Rect::new(0.0, 0.0, 10.0, 10.0), 0.2), face(Rect::new(5.0, 0.0, 15.0, 10.0), 0.8)];
        let visible = visible_faces(&faces).unwrap();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].brightness, 0.2);
        assert_eq!(visible[0].path.bounds(), Some(Rect::new(0.0, 0.0, 5.0, 10.0)));
        assert!((visible[0].path.to_bezpath().area() - 50.0).abs() < 1e-8);
        assert!((visible[1].path.to_bezpath().area() - 100.0).abs() < 1e-8);
        // Winding and equal depth do not determine visibility; actual paint order does.
        let mut flipped = faces.clone();
        for f in &mut flipped {
            f.points.reverse();
            f.depth = 0.0;
        }
        let other = visible_faces(&flipped).unwrap();
        assert_eq!(visible.iter().map(|f| f.brightness).collect::<Vec<_>>(), other.iter().map(|f| f.brightness).collect::<Vec<_>>());
        assert_eq!(visible[0].path.bounds(), other[0].path.bounds());
    }

    #[test]
    fn an_interior_surface_remains_visible_through_an_opening() {
        let faces = vec![
            face(Rect::new(0.0, 0.0, 10.0, 10.0), 0.1),
            face(Rect::new(0.0, 0.0, 10.0, 3.0), 0.2),
            face(Rect::new(0.0, 7.0, 10.0, 10.0), 0.3),
            face(Rect::new(0.0, 3.0, 3.0, 7.0), 0.4),
            face(Rect::new(7.0, 3.0, 10.0, 7.0), 0.5),
        ];
        let visible = visible_faces(&faces).unwrap();
        assert_eq!(visible[0].brightness, 0.1);
        assert_eq!(visible[0].path.bounds(), Some(Rect::new(3.0, 3.0, 7.0, 7.0)));
        assert_eq!(visible[0].path.to_bezpath().winding(Point::new(5.0, 5.0)), 1);
        assert_eq!(visible[0].path.to_bezpath().winding(Point::new(1.0, 1.0)), 0);
    }

    #[test]
    fn revolved_open_profiles_match_visible_paint_for_both_axis_sides_and_perspective() {
        let mut bp = BezPath::new();
        bp.move_to((40.0, 10.0));
        bp.curve_to((75.0, 20.0), (75.0, 80.0), (40.0, 90.0));
        let path = PathData::from_bezpath(&bp);
        for edge in [Edge::Left, Edge::Right] {
            for angle in [155.0, 360.0] {
                let options = Revolve {
                    edge,
                    angle,
                    offset: 8.0,
                    segments: 32,
                    rotation_x: -32.0,
                    rotation_y: 24.0,
                    rotation_z: 19.0,
                    perspective: 0.7,
                    ..Default::default()
                };
                let faces = revolve(&path, path.bounds().unwrap(), options).unwrap();
                let visible = visible_faces(&faces).unwrap();
                assert!(visible.len() <= faces.len());
                let all: Vec<_> = faces.iter().map(|f| (f.path().to_bezpath(), f.brightness)).collect();
                let cut: Vec<_> = visible.iter().map(|f| (f.path.to_bezpath(), f.brightness)).collect();
                let b = bounds(&faces.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>());
                for x in 0..48 {
                    for y in 0..48 {
                        let p = Point::new(b.x0 + b.width() * (x as f64 + 0.337) / 48.0, b.y0 + b.height() * (y as f64 + 0.413) / 48.0);
                        let expected = all.iter().rev().find(|(bp, _)| bp.winding(p) != 0).map(|(_, value)| value);
                        let actual = cut.iter().rev().find(|(bp, _)| bp.winding(p) != 0).map(|(_, value)| value);
                        assert_eq!(actual, expected, "{edge:?}, {angle}, {p:?}");
                        assert!(cut.iter().filter(|(bp, _)| bp.winding(p) != 0).count() <= 1, "expanded surfaces do not cover hidden regions");
                    }
                }
            }
        }
    }

    #[test]
    fn visibility_is_bounded_for_repeated_faces_and_rejects_invalid_input() {
        let faces = vec![face(Rect::new(0.0, 0.0, 10.0, 10.0), 0.5); MAX_FACES];
        assert_eq!(visible_faces(&faces).unwrap().len(), 1);
        assert!(visible_faces(&vec![faces[0].clone(); MAX_FACES + 1]).is_err());
        let mut bad = faces[0].clone();
        bad.points[0].x = f64::NAN;
        assert!(visible_faces(&[bad]).is_err());
        assert!(visible_faces(&[]).unwrap().is_empty());
    }
}
