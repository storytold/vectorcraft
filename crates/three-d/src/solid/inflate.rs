use super::build::{Builder, Camera, Order, Turn, check_bounds, cuts, fit, newell, outward};
use super::membrane::{Edges, MAX_MATERIAL_SEGMENTS, Membrane, cell_triangles, clip, filled, material};
use super::rings::{MAX_SOLID_POINTS, rings, signed_area};
use super::*;

/// A shape to inflate: its rings and the membrane under them on the grid of the bounds.
struct Puff {
    rings: Vec<Vec<Point>>,
    /// The index of the rings' edges.
    edges: Edges,
    cell: f64,
    nx: usize,
    ny: usize,
    m: Membrane,
    /// How high the front rises at its peak.
    height: f64,
}

/// The rings of `path` and its membrane on the grid of `bounds`; `None` for a shape with no area.
fn puff(path: &PathData, rule: FillRule, bounds: Rect, o: &Inflate) -> Result<Option<Puff>, String> {
    let rings = rings(path, rule, bounds.width().max(bounds.height()), MAX_SOLID_POINTS)?;
    if rings.is_empty() {
        return Ok(None);
    }
    let size = bounds.width().max(bounds.height());
    if size <= 0.0 {
        return Ok(None);
    }
    const CELLS: usize = 48;
    let cell = size / CELLS as f64;
    let (nx, ny) = (((bounds.width() / cell).ceil() as usize).max(1), ((bounds.height() / cell).ceil() as usize).max(1));
    let edges = Edges::new(&rings);
    let m = Membrane::solve(&rings, &edges, bounds.origin(), cell, nx + 1, ny + 1);
    // A circle of radius r puffs into a half sphere of radius r at 50 % (its membrane peaks at r²/4).
    let height = 2.0 * m.peak.sqrt() * o.volume / 50.0;
    Ok(Some(Puff { rings, edges, cell, nx, ny, m, height }))
}

/// Inflate: the shape puffed out like a cushion, its edge kept where the art's outline is.
pub fn inflate(path: &PathData, rule: FillRule, bounds: Rect, options: Inflate, view: View) -> Result<Vec<Facet>, String> {
    inflate_together(&[(path, rule)], bounds, options, view).pop().unwrap_or(Ok(vec![]))
}

/// Inflate for several shapes of one object (a fill and the outlines of its strokes): each puffs
/// on its own over the grid of `bounds`, and one eye sees them all, as far away as the highest
/// puff needs with Perspective. One result for each shape, in order.
pub fn inflate_together(shapes: &[(&PathData, FillRule)], bounds: Rect, options: Inflate, view: View) -> Vec<Result<Vec<Facet>, String>> {
    let o = options.sanitized();
    let puffs: Vec<Result<Option<Puff>, String>> =
        shapes.iter().map(|(path, rule)| check_bounds(bounds).and_then(|()| puff(path, *rule, bounds, &o))).collect();
    let reach = puffs.iter().flatten().flatten().map(|p| p.height).fold(0.0, f64::max);
    puffs
        .into_iter()
        .map(|p| match p {
            Ok(Some(p)) => inflated(p, bounds, o, view, reach),
            Ok(None) => Ok(vec![]),
            Err(e) => Err(e),
        })
        .collect()
}

/// The facets of one inflated shape, for a camera that allows for a puff `reach` high (or the
/// shape's own, if higher): `reach` is the highest puff of the shapes drawn together.
fn inflated(p: Puff, bounds: Rect, o: Inflate, view: View, reach: f64) -> Result<Vec<Facet>, String> {
    let Puff { rings, edges, cell, nx, ny, m, height } = p;
    let outline: Vec<(Rect, vectorcraft_geom::SubPath)> = rings
        .iter()
        .filter_map(|r| {
            let b = r.iter().skip(1).fold(Rect::from_points(*r.first()?, *r.first()?), |b, p| b.union_pt(*p));
            Some((b, vectorcraft_geom::SubPath::polyline(r, true)))
        })
        .collect();
    // When no shape puffs (no volume, or shapes too narrow for the grid), every front (and a back
    // that puffs too) lies flat in one plane: its cells are drawn last, as caps are. If one shape
    // puffs, a flat front can be behind its puff, so the cells keep their depths.
    let flat = reach <= 0.0;
    let lift = |p: Point| {
        if m.peak <= 0.0 || edges.near(p, 1e-9) {
            return 0.0;
        }
        height * (m.at(p).max(0.0) / m.peak).sqrt()
    };
    let normal = |p: Point| {
        let (dx, dy) = m.gradient(p);
        let (_, inward, distance) = edges.closest(p);
        let t = (distance / (2.0 * cell)).clamp(0.0, 1.0);
        let weight = (1.0 - t).powi(2) * (1.0 + 2.0 * t);
        let gradient = Vec2::new(dx, dy);
        let slope = gradient.dot(inward).max(0.0);
        // The zero boundary condition also fixes the tangential derivative at the outline.
        // Extrapolated outside solver nodes cannot supply it reliably. Blend to that known
        // direction and its limiting phi near the rim, without showing the grid in highlights.
        let gradient = gradient * (1.0 - weight) + inward * (slope * weight);
        let phi = m.at(p).max(0.0) * (1.0 - weight) + slope * distance * weight;
        // Multiplying the height-function normal by 2√(peak φ) avoids division by
        // zero at the rim, where the limiting normal is horizontal.
        let n = V3(-height * gradient.x, -height * gradient.y, 2.0 * (m.peak * phi).sqrt()).unit();
        if n.dot(n) > 0.5 { n } else { V3(0.0, 0.0, 1.0) }
    };
    let smooth = view.shade && view.has_colored_lights();
    let camera = Camera::new(view, bounds, o.depth + 2.0 * height.max(reach));
    // Without its back cap the object is a shell open behind, its inside showing.
    let mut b = Builder { camera: &camera, closed: o.cap, facets: vec![] };
    let half = o.depth / 2.0;
    // The back: turned and scaled by the twist and taper, its puff too.
    let back_turn = Turn::new(bounds, o.twist, o.taper, 1.0);
    // The grid's cells, those the surface rises steeply across (near the outline) cut into
    // halves or quarters each way, so the steep rim is drawn in facets as fine as its curve.
    // Every corner sits on one fine lattice (a quarter cell), computed the same way wherever it
    // is used, and a cell's border takes its finer neighbour's lattice points too: where cells
    // of different fineness meet, their edges follow the surface through the same points, with
    // no cracks.
    const FINE: usize = 4;
    let unit = cell / FINE as f64;
    let at = |qx: usize, qy: usize| Point::new(bounds.x0 + qx as f64 * unit, bounds.y0 + qy as f64 * unit);
    let base = |i: usize, j: usize| Rect::from_points(at(i * FINE, j * FINE), at((i + 1) * FINE, (j + 1) * FINE));
    // Segments the booleans of mixed cells may take in all ([`MAX_MATERIAL_SEGMENTS`]): each one
    // builds the topology of every outline ring near its cell, so a self-crossing outline cost its
    // crossings again in every cell it spans. Past it, cells take their clipped pieces.
    let material_left = std::cell::Cell::new(MAX_MATERIAL_SEGMENTS);
    // The shape's part of a rectangle, as rings.
    let cut = |r: Rect| -> Vec<Vec<Point>> {
        // A cell no edge crosses is wholly inside the shape or wholly outside it.
        if !edges.touch(r) {
            let corners = vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
            return if filled(&rings, r.center()) { vec![corners] } else { vec![] };
        }
        let pieces: Vec<Vec<Point>> = rings.iter().map(|ring| clip(ring, r)).filter(|c| !c.is_empty()).collect();
        if pieces.iter().map(|p| signed_area(p)).sum::<f64>() <= 1e-9 * r.area() {
            return vec![];
        }
        // Where a hole's piece cancels part of an outline's, that only holds flat: lifted onto
        // the surface the two no longer meet. Such a cell takes the shape's own part of it.
        let mixed = pieces.iter().any(|p| signed_area(p) < 0.0) && pieces.iter().any(|p| signed_area(p) > 0.0);
        if !mixed {
            return pieces;
        }
        let near: usize =
            outline.iter().filter(|(b, _)| b.x0 <= r.x1 && r.x0 <= b.x1 && b.y0 <= r.y1 && r.y0 <= b.y1).map(|(_, sp)| sp.anchors.len()).sum();
        match material_left.get().checked_sub(near) {
            Some(left) => {
                material_left.set(left);
                material(&outline, r).unwrap_or(pieces)
            }
            None => pieces,
        }
    };
    // How many times each cell is halved each way (0, 1 or 2), from how steeply it rises.
    let levels: Vec<u32> = (0..nx * ny)
        .map(|k| {
            let pieces = cut(base(k % nx, k / nx));
            let (lo, hi) = pieces.iter().flatten().map(|q| lift(*q)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), h| (lo.min(h), hi.max(h)));
            let rise = if pieces.is_empty() { 0.0 } else { (hi - lo) / cell };
            if rise > 6.0 {
                2
            } else if rise > 2.0 {
                1
            } else {
                0
            }
        })
        .collect();
    let level =
        |i: isize, j: isize| -> u32 { if i < 0 || j < 0 || i as usize >= nx || j as usize >= ny { 0 } else { levels[j as usize * nx + i as usize] } };
    for j in 0..ny {
        for i in 0..nx {
            let l = level(i as isize, j as isize);
            let step = FINE >> l;
            // The lattice spacing along each side of this cell: its own or its neighbour's.
            let (ii, jj) = (i as isize, j as isize);
            let side_step = |dl: u32| FINE >> l.max(dl);
            let (left, right, top, bottom) =
                (side_step(level(ii - 1, jj)), side_step(level(ii + 1, jj)), side_step(level(ii, jj - 1)), side_step(level(ii, jj + 1)));
            let (bx0, by0, bx1, by1) = (i * FINE, j * FINE, (i + 1) * FINE, (j + 1) * FINE);
            for sy in (by0..by1).step_by(step) {
                for sx in (bx0..bx1).step_by(step) {
                    let r = Rect::from_points(at(sx, sy), at(sx + step, sy + step));
                    let pieces = cut(r);
                    if pieces.is_empty() {
                        continue;
                    }
                    // Lattice points along the cell's own border, between each edge's ends.
                    let stitch = |a: Point, b: Point| -> Vec<Point> {
                        let tol = 1e-7 * unit;
                        let along = |fixed_x: Option<usize>, fixed_y: Option<usize>, every: usize| -> Vec<Point> {
                            let mut out = vec![];
                            match (fixed_x, fixed_y) {
                                (Some(qx), None) => {
                                    let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
                                    for qy in (by0..=by1).step_by(every) {
                                        let p = at(qx, qy);
                                        if p.y > lo + tol && p.y < hi - tol {
                                            out.push(p);
                                        }
                                    }
                                }
                                (None, Some(qy)) => {
                                    let (lo, hi) = (a.x.min(b.x), a.x.max(b.x));
                                    for qx in (bx0..=bx1).step_by(every) {
                                        let p = at(qx, qy);
                                        if p.x > lo + tol && p.x < hi - tol {
                                            out.push(p);
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if (a.x > b.x) || (a.y > b.y) {
                                out.reverse();
                            }
                            out
                        };
                        let on = |v: f64, w: f64| (v - w).abs() <= tol;
                        let (x0, x1, y0, y1) = (at(bx0, by0).x, at(bx1, by1).x, at(bx0, by0).y, at(bx1, by1).y);
                        if on(a.x, x0) && on(b.x, x0) {
                            along(Some(bx0), None, left)
                        } else if on(a.x, x1) && on(b.x, x1) {
                            along(Some(bx1), None, right)
                        } else if on(a.y, y0) && on(b.y, y0) {
                            along(None, Some(by0), top)
                        } else if on(a.y, y1) && on(b.y, y1) {
                            along(None, Some(by1), bottom)
                        } else {
                            vec![]
                        }
                    };
                    let pieces: Vec<Vec<Point>> = pieces
                        .iter()
                        .map(|p| {
                            let mut out = Vec::with_capacity(p.len());
                            for (a, b) in p.iter().zip(p.iter().cycle().skip(1)).take(p.len()) {
                                out.push(*a);
                                out.extend(stitch(*a, *b));
                            }
                            out
                        })
                        .collect();
                    if height > 0.0
                        && smooth
                        && let Some(triangles) = cell_triangles(&pieces)
                    {
                        for points in triangles {
                            let front = points.map(|q| camera.place(q, half + lift(q)));
                            b.smooth_triangle(front, points.map(|q| camera.rotate(normal(q))), false)?;
                            if o.both_sides && o.cap {
                                let back = points.map(|q| camera.place(back_turn.apply(q), -half - lift(q) * back_turn.scale));
                                let normals = points.map(|q| {
                                    let n = normal(q);
                                    // A uniform taper scales all three coordinates equally;
                                    // twist turns the back normal about the shape's z-axis.
                                    camera.rotate(V3(back_turn.cos * n.0 - back_turn.sin * n.1, back_turn.sin * n.0 + back_turn.cos * n.1, -n.2))
                                });
                                b.smooth_triangle(back, normals, true)?;
                            }
                        }
                        continue;
                    }
                    let lifts: Vec<Vec<f64>> = pieces.iter().map(|p| p.iter().map(|q| lift(*q)).collect()).collect();
                    // Each piece lifted onto the surface. Its normal is the lifted piece's own average
                    // (Newell's), not the slope at one point: near the edge the surface turns
                    // vertical, and a point sample there misjudges which way a whole piece faces.
                    let front: Vec<Vec<V3>> =
                        pieces.iter().zip(&lifts).map(|(p, h)| p.iter().zip(h).map(|(q, h)| camera.place(*q, half + h)).collect()).collect();
                    let n = newell(&front);
                    let order = if flat { Order::Last } else { Order::Depth };
                    b.face(front, n, order)?;
                    if o.both_sides && o.cap {
                        // The back winds the same way round as the front, so its outward normal is
                        // reversed.
                        let back: Vec<Vec<V3>> = pieces
                            .iter()
                            .zip(&lifts)
                            .map(|(p, h)| p.iter().zip(h).map(|(q, h)| camera.place(back_turn.apply(*q), -half - h * back_turn.scale)).collect())
                            .collect();
                        let n = newell(&back);
                        b.face(back, V3(-n.0, -n.1, -n.2), order)?;
                    }
                }
            }
        }
    }
    if !o.both_sides && o.cap {
        let back: Vec<Vec<V3>> = rings.iter().map(|r| r.iter().map(|p| camera.place(back_turn.apply(*p), -half)).collect()).collect();
        // The back turned toward the viewer covers the walls it overlaps, even those nearer than
        // its center: twisted walls, and the walls of a fill inside the solid of its stroke.
        b.face(back, camera.rotate(V3(0.0, 0.0, -1.0)), Order::Last)?;
    }
    if o.depth > 0.0 {
        // The sides, cut into slices that follow a twist or taper.
        let slices = if o.twist != 0.0 || o.taper != 1.0 { ((o.twist.abs() / 6.0).ceil() as usize).clamp(8, 64) } else { 1 };
        let at = |ring: &[Point], k: usize| -> Vec<V3> {
            let t = k as f64 / slices as f64;
            let turn = Turn::new(bounds, o.twist, o.taper, t);
            ring.iter().map(|p| camera.place(turn.apply(*p), half - o.depth * t)).collect()
        };
        let mut pieces: Vec<Vec<usize>> = rings.iter().map(|r| if o.twist != 0.0 { cuts(&at(r, 0), &at(r, 1)) } else { vec![] }).collect();
        fit(&mut pieces, slices, MAX_FACES.saturating_sub(b.facets.len()));
        for (ring, pieces) in rings.iter().zip(&pieces) {
            for k in 0..slices {
                b.band(&at(ring, k), &at(ring, k + 1), pieces, |i| outward(ring, i, 0.0))?;
            }
        }
    }
    Ok(b.finish())
}
