//! Bounded convex pieces for continuous caps and curved endpoint faces.

use super::{Face, MAX_WORK, Point, area, clean};

fn convex(points: &[Point], epsilon: f64) -> bool {
    points.iter().enumerate().all(|(i, a)| {
        let Some(b) = points.get((i + 1) % points.len()) else { return true };
        let Some(c) = points.get((i + 2) % points.len()) else { return true };
        (*b - *a).cross(*c - *b) >= -epsilon * b.distance(*a).max(1.0)
    })
}

fn triangulate(mut points: Vec<Point>, epsilon: f64, work: &mut usize, depth: usize) -> Result<Vec<Vec<Point>>, String> {
    if points.is_empty() {
        return Ok(vec![]);
    }
    if convex(&points, epsilon) {
        return Ok(vec![points]);
    }
    // At a grazing view a curved rim can fold across a triangle's diagonal.
    // Split that crossing before ear clipping; its two lobes both have nonzero
    // winding and must remain visible. Bound splitting for public helper inputs.
    for (i, (a, b)) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len()).enumerate() {
        for j in i + 2..points.len() {
            if (j + 1) % points.len() == i {
                continue;
            }
            *work += 1;
            if *work > MAX_WORK {
                return Err("Revolve visibility exceeds its geometry work limit".into());
            }
            let Some(c) = points.get(j) else { continue };
            let Some(d) = points.get((j + 1) % points.len()) else { continue };
            let (ab, cd) = (*b - *a, *d - *c);
            let denominator = ab.cross(cd);
            if denominator.abs() <= epsilon * epsilon {
                continue;
            }
            let (t, u) = ((*c - *a).cross(cd) / denominator, (*c - *a).cross(ab) / denominator);
            if t > 1e-9 && t < 1.0 - 1e-9 && u > 1e-9 && u < 1.0 - 1e-9 {
                if depth >= 8 {
                    return Err("Revolve visibility has too many boundary crossings".into());
                }
                let intersection = a.lerp(*b, t);
                let first = std::iter::once(intersection).chain(points.get(i + 1..=j).unwrap_or_default().iter().copied()).collect();
                let second = std::iter::once(intersection).chain(points.iter().cycle().skip(j + 1).take(points.len() + i - j).copied()).collect();
                let mut out = triangulate(clean(first, epsilon), epsilon, work, depth + 1)?;
                out.extend(triangulate(clean(second, epsilon), epsilon, work, depth + 1)?);
                return Ok(out);
            }
        }
    }
    let mut out = vec![];
    while points.len() > 3 {
        let mut ear = None;
        for (i, b) in points.iter().enumerate() {
            let Some(a) = points.get((i + points.len() - 1) % points.len()) else { continue };
            let Some(c) = points.get((i + 1) % points.len()) else { continue };
            if (*b - *a).cross(*c - *b) <= epsilon * epsilon {
                continue;
            }
            let mut blocked = false;
            for p in &points {
                *work += 1;
                if *work > MAX_WORK {
                    return Err("Revolve visibility exceeds its geometry work limit".into());
                }
                if p != a
                    && p != b
                    && p != c
                    && (*b - *a).cross(*p - *a) >= -epsilon * epsilon
                    && (*c - *b).cross(*p - *b) >= -epsilon * epsilon
                    && (*a - *c).cross(*p - *c) >= -epsilon * epsilon
                {
                    blocked = true;
                    break;
                }
            }
            if !blocked {
                ear = Some((i, vec![*a, *b, *c]));
                break;
            }
        }
        let Some((i, triangle)) = ear else { return Err("Revolve visibility cannot split the projected boundary".into()) };
        out.push(triangle);
        points.remove(i);
    }
    if area(&points).abs() > epsilon * epsilon {
        out.push(points);
    }
    Ok(out)
}

/// Triangles covering a flat face drawn as several straight-edged contours, such as a cut side of a
/// partial Revolve whose profile has holes: outlines one way round and holes the other, the holes
/// left uncovered. The work is checked before the face is cut, at four times its corners squared.
fn with_holes(outline: &vectorcraft_geom::PathData, epsilon: f64, work: &mut usize) -> Result<Vec<Vec<Point>>, String> {
    const LIMIT: &str = "Revolve visibility exceeds its geometry work limit";
    let rings: Vec<Vec<crate::V3>> = outline.subpaths.iter().map(|s| s.anchors.iter().map(|a| crate::V3(a.p.x, a.p.y, 0.0)).collect()).collect();
    let corners: usize = rings.iter().map(Vec::len).sum();
    *work = work.saturating_add(corners.saturating_mul(corners).saturating_mul(4));
    if *work > MAX_WORK {
        return Err(LIMIT.into());
    }
    let mut spent = 0;
    let triangles = crate::mesh::triangulate(&rings, crate::mesh::newell(&rings), &mut spent)?;
    Ok(triangles.into_iter().map(|t| clean(t.iter().map(|v| Point::new(v.0, v.1)).collect(), epsilon)).filter(|t| !t.is_empty()).collect())
}

pub(super) fn pieces(face: &Face, epsilon: f64, work: &mut usize) -> Result<Vec<Vec<Point>>, String> {
    if let Some(strips) = &face.visibility_pieces {
        let mut out = vec![];
        for strip in strips {
            out.extend(triangulate(clean(strip.clone(), epsilon), epsilon, work, 0)?);
        }
        return Ok(out);
    }
    if let Some(outline) = face.outline.as_ref().filter(|o| !face.cap && o.subpaths.len() > 1) {
        return with_holes(outline, epsilon, work);
    }
    if !face.cap {
        return triangulate(clean(face.points.clone(), epsilon), epsilon, work, 0);
    }
    let Some((center, ring)) = face.points.split_first() else { return Ok(vec![]) };
    let mut out = vec![];
    // At least 512 samples per full circle makes 128 edges at most a quarter
    // turn. These sectors are convex even when the complete sweep exceeds 180°.
    for start in (0..ring.len().saturating_sub(1)).step_by(128) {
        let end = (start + 128).min(ring.len().saturating_sub(1));
        let Some(arc) = ring.get(start..=end) else { continue };
        let points = clean(std::iter::once(*center).chain(arc.iter().copied()).collect(), epsilon);
        if !convex(&points, epsilon) {
            return Err("Revolve visibility requires convex cap sectors".into());
        }
        if !points.is_empty() {
            out.push(points);
        }
    }
    Ok(out)
}
