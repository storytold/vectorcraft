//! The shadow a 3D effect's solid casts: its faces projected along the first light onto a plane
//! behind the object (square to the view) or below it (under the object's own vertical axis, turned
//! with it), as one hard-edged vector shape.
//!
//! Every face is projected whole, so the shape is the union of the projected faces: each contour is
//! turned so that its face winds the same way, and the shape is filled with the nonzero rule. The
//! plane ends at a square about the foot of the object's center, so a light that grazes the plane
//! does not cast a shadow without end.

use std::ops::Range;

use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect};

use crate::solid::{Face3, Keep, extrude_faces, inflate_faces, plane_faces};
use crate::{Extrude, Inflate, MAX_SOLID_FACES, Revolve, V3, View, finite, perspective_scale};

/// Where the shadow falls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowPosition {
    /// On a plane behind the object, square to the view.
    #[default]
    Behind,
    /// On a plane below the object, square to its vertical axis and turned with it.
    Below,
}

impl ShadowPosition {
    pub const ALL: [ShadowPosition; 2] = [Self::Behind, Self::Below];

    pub fn id(self) -> &'static str {
        match self {
            Self::Behind => "behind",
            Self::Below => "below",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == s)
    }
}

/// A shadow's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub position: ShadowPosition,
    /// How far the plane is from the object, as a share (0..1) of the object's largest size.
    pub distance: f64,
    /// How far the plane reaches, as a share (0.1..4): at 1 the plane is a square four times the
    /// object's largest size across.
    pub bounds: f64,
}

impl Default for Shadow {
    fn default() -> Self {
        Self { position: ShadowPosition::Behind, distance: 0.0, bounds: 1.0 }
    }
}

impl Shadow {
    fn sanitized(self) -> Self {
        Self { distance: finite(self.distance, 0.0, 0.0, 1.0), bounds: finite(self.bounds, 1.0, 0.1, 4.0), ..self }
    }
}

/// The rotation and projection a solid is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Projection {
    /// The sanitized view: its rotation, perspective and lights.
    pub(crate) view: Revolve,
    /// Where the view's origin is on the page.
    pub(crate) origin: Point,
    /// The eye's distance in front of the origin; `None` without perspective.
    pub(crate) eye: Option<f64>,
}

impl Projection {
    fn project(&self, v: V3) -> Point {
        let f = perspective_scale(self.eye, v.2);
        Point::new(self.origin.x + v.0 * f, self.origin.y + v.1 * f)
    }

    /// The unit direction toward the first light, in view space; `None` when it lights nothing:
    /// Shading is off or its intensity is 0.
    fn light(&self) -> Option<V3> {
        let first = self.view.lights.and_then(|l| l.iter().next().copied());
        let intensity = first.map_or(self.view.light_intensity, |l| l.intensity);
        if !self.view.shade || intensity <= 0.0 {
            return None;
        }
        let [x, y, z] = first.map_or_else(|| self.view.light_direction(), |l| l.direction());
        Some(V3(x, y, z).unit())
    }
}

/// The light must reach the plane at least this steeply (its cosine to the plane's normal).
const MIN_LIGHT: f64 = 1e-3;

/// Most faces one object's shadow is cast from, its solids together: as many as two of the largest
/// solids.
pub const MAX_SHADOW_FACES: usize = 2 * MAX_SOLID_FACES;

/// The error a solid added past [`MAX_SHADOW_FACES`] returns.
pub const TOO_COMPLEX_SHADOW: &str = "The 3D shadow is too complex; turn Shadows off or simplify the art";

/// The plane a shadow falls on ([`ShadowCaster::shadow_plane`]), placed once for the pieces of one
/// object's shadow ([`ShadowCaster::path_on`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowPlane {
    projection: Projection,
    /// The unit direction toward the light, and its cosine to the plane's normal.
    light: V3,
    reach: f64,
    /// The plane's unit normal toward the object, and two unit axes along it.
    n: V3,
    u: V3,
    w: V3,
    /// Where the plane is along its normal, and the foot of the object's center on it.
    level: f64,
    foot: V3,
    /// Half the side of the square the plane ends at.
    half: f64,
}

/// The faces of one object's solids, gathered to cast one shadow. The solids of one object share
/// their bounds and view, so they share a projection: the first solid's is kept. Each solid added
/// returns the range of its faces, so that the shadow of some of them can be drawn on the plane
/// of others ([`ShadowCaster::shadow_plane`], [`ShadowCaster::path_on`]).
#[derive(Clone, Debug, Default)]
pub struct ShadowCaster {
    faces: Vec<Face3>,
    projection: Option<Projection>,
}

impl ShadowCaster {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many faces have been gathered.
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// Keep `faces`, or an error when they would take the faces gathered past [`MAX_SHADOW_FACES`].
    fn add(&mut self, faces: Vec<Face3>, projection: Option<Projection>) -> Result<Range<usize>, String> {
        let start = self.faces.len();
        if start.saturating_add(faces.len()) > MAX_SHADOW_FACES {
            return Err(TOO_COMPLEX_SHADOW.into());
        }
        if !faces.is_empty() && self.projection.is_none() {
            self.projection = projection;
        }
        self.faces.extend(faces);
        Ok(start..self.faces.len())
    }

    /// Add Extrude & Bevel's solid ([`crate::extrude`]).
    pub fn extrude(&mut self, path: &PathData, rule: FillRule, bounds: Rect, options: Extrude, view: View) -> Result<Range<usize>, String> {
        let (_, faces, projection) = extrude_faces(path, rule, bounds, options, view, Keep::Faces)?;
        self.add(faces?, projection)
    }

    /// Add Rotate's plane ([`crate::plane`]).
    pub fn plane(&mut self, path: &PathData, rule: FillRule, bounds: Rect, view: View) -> Result<Range<usize>, String> {
        let (_, faces, projection) = plane_faces(path, rule, bounds, view, Keep::Faces)?;
        self.add(faces?, projection)
    }

    /// Add Inflate's solids for several shapes of one object ([`crate::inflate_together`]), each
    /// shape whose entry in `casts` is true: the eye is placed for every shape, as for the solids
    /// drawn, but the solids of the others are not built (their range is empty). A casting shape
    /// that does not inflate is left out, as its solid is; the result says which, in order.
    pub fn inflate(
        &mut self,
        shapes: &[(&PathData, FillRule)],
        casts: &[bool],
        bounds: Rect,
        options: Inflate,
        view: View,
    ) -> Vec<Result<Range<usize>, String>> {
        let mut solids = inflate_faces(shapes, bounds, options, view, Keep::Faces);
        (0..shapes.len())
            .map(|i| {
                if !casts.get(i).copied().unwrap_or(true) {
                    solids.pass();
                    return Ok(self.faces.len()..self.faces.len());
                }
                let (_, faces, projection) = solids.next().unwrap_or_else(|| Ok((vec![], Ok(vec![]), None)))?;
                self.add(faces?, projection)
            })
            .collect()
    }

    /// Add Revolve's surface ([`crate::revolve`]).
    pub fn revolve(&mut self, path: &PathData, rule: FillRule, bounds: Rect, options: Revolve) -> Result<Range<usize>, String> {
        let mut faces = vec![];
        crate::revolve_faces(path, rule, bounds, options, Some(&mut faces))?;
        let o = options.sanitized();
        self.add(faces, Some(crate::revolve_projection(bounds, o)))
    }

    /// The shadow of the faces gathered, on the page, filled with the nonzero rule: `None` when
    /// there are none, when the first light lights nothing (Shading is off or its intensity is 0)
    /// or does not reach the plane (it is behind the object for a shadow behind it, or below the
    /// plane for one below), or when nothing of the shadow falls within the plane.
    pub fn path(&self, shadow: Shadow) -> Option<PathData> {
        let all = 0..self.faces.len();
        let all = std::slice::from_ref(&all);
        self.path_on(&self.shadow_plane(shadow, all)?, all)
    }

    /// The faces in the ranges `faces`, from the faces gathered.
    fn within(&self, faces: &[Range<usize>]) -> impl Iterator<Item = &Face3> {
        faces.iter().flat_map(|r| self.faces.get(r.clone()).unwrap_or_default())
    }

    /// The plane `shadow` falls on for the faces in the ranges `faces` (its distance, size and
    /// center follow from them), for [`ShadowCaster::path_on`]: `None` when there are none or when
    /// the first light lights nothing or does not reach the plane.
    pub fn shadow_plane(&self, shadow: Shadow, faces: &[Range<usize>]) -> Option<ShadowPlane> {
        let p = self.projection?;
        let light = p.light()?;
        let s = shadow.sanitized();
        let corners = || self.within(faces).flat_map(|(contours, _)| contours.iter().flatten());
        let first = *corners().next()?;
        let (lo, hi) = corners()
            .fold((first, first), |(lo, hi), v| (V3(lo.0.min(v.0), lo.1.min(v.1), lo.2.min(v.2)), V3(hi.0.max(v.0), hi.1.max(v.1), hi.2.max(v.2))));
        let size = (hi.0 - lo.0).max(hi.1 - lo.1).max(hi.2 - lo.2).max(1e-9);
        // The plane's normal toward the object, and two axes along it.
        let (n, u, w) = match s.position {
            ShadowPosition::Behind => (V3(0.0, 0.0, 1.0), V3(1.0, 0.0, 0.0), V3(0.0, 1.0, 0.0)),
            ShadowPosition::Below => {
                let r = |v: V3| p.view.rotate(v).unit();
                (r(V3(0.0, -1.0, 0.0)), r(V3(1.0, 0.0, 0.0)), r(V3(0.0, 0.0, 1.0)))
            }
        };
        let reach = n.dot(light);
        if !reach.is_finite() || reach < MIN_LIGHT {
            return None;
        }
        // The plane: every corner at least `distance` × size in front of it.
        let level = corners().map(|v| n.dot(*v)).fold(f64::INFINITY, f64::min) - s.distance * size;
        let centre = V3((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0, (lo.2 + hi.2) / 2.0);
        let foot = add(centre, scale(n, level - n.dot(centre)));
        Some(ShadowPlane { projection: p, light, reach, n, u, w, level, foot, half: 2.0 * s.bounds * size })
    }

    /// The shadow the faces in the ranges `faces` cast on `plane`, on the page, filled with the
    /// nonzero rule; `None` when nothing of it falls within the plane.
    pub fn path_on(&self, plane: &ShadowPlane, faces: &[Range<usize>]) -> Option<PathData> {
        let &ShadowPlane { projection: p, light, reach, n, u, w, level, foot, half } = plane;
        // A corner's shadow, in the plane's axes about the foot.
        let cast = |v: V3| {
            let q = add(v, scale(light, -(n.dot(v) - level) / reach));
            let d = q.sub(foot);
            Point::new(u.dot(d), w.dot(d))
        };
        let to_page = |q: Point| p.project(add(foot, add(scale(u, q.x), scale(w, q.y))));
        let mut bp = BezPath::new();
        for (contours, _) in self.within(faces) {
            let flat: Vec<Vec<Point>> = contours.iter().map(|c| c.iter().map(|v| cast(*v)).collect()).collect();
            let turn: f64 = flat.iter().map(|c| signed_area(c)).sum();
            // A face edge-on to the light casts nothing.
            if flat_area(turn, flat.iter().flatten()) {
                continue;
            }
            for c in &flat {
                let mut clipped = clip(c, half);
                if turn < 0.0 {
                    clipped.reverse();
                }
                // A plane seen edge-on draws nothing.
                let page: Vec<Point> = clipped.into_iter().map(to_page).collect();
                if flat_area(signed_area(&page), &page) {
                    continue;
                }
                let Some((start, rest)) = page.split_first() else { continue };
                bp.move_to(*start);
                for q in rest {
                    bp.line_to(*q);
                }
                bp.close_path();
            }
        }
        let path = PathData::from_bezpath(&bp);
        (!path.subpaths.is_empty()).then_some(path)
    }
}

/// Whether `area`, the signed area of a polygon with corners `points`, is within rounding of
/// nothing for its size (the square of its bounds' diagonal), or not a number, so the polygon draws
/// nothing. A long, narrow polygon keeps an area well above that.
fn flat_area<'a>(area: f64, points: impl IntoIterator<Item = &'a Point>) -> bool {
    let mut points = points.into_iter();
    let Some(first) = points.next() else { return true };
    let bounds = points.fold(Rect::from_points(*first, *first), |b, p| b.union_pt(*p));
    let size = bounds.width().powi(2) + bounds.height().powi(2);
    !area.is_finite() || !size.is_finite() || area.abs() <= 1e-14 * size
}

fn add(a: V3, b: V3) -> V3 {
    V3(a.0 + b.0, a.1 + b.1, a.2 + b.2)
}

fn scale(a: V3, k: f64) -> V3 {
    V3(a.0 * k, a.1 * k, a.2 * k)
}

/// Twice the signed area of polygon `c` (positive counterclockwise with y up).
fn signed_area(c: &[Point]) -> f64 {
    c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>() / 2.0
}

/// Polygon `c` cut to the square from -`half` to `half` on both axes, edge by edge (the square is
/// convex, so each contour's winding inside it is kept).
fn clip(c: &[Point], half: f64) -> Vec<Point> {
    let mut out: Vec<Point> = c.to_vec();
    // Each side: the axis it cuts across (x or y) and whether it is the upper one.
    for (across_x, upper) in [(true, false), (true, true), (false, false), (false, true)] {
        let coord = |p: Point| if across_x { p.x } else { p.y };
        let limit = if upper { half } else { -half };
        let inside = |p: Point| if upper { coord(p) <= limit } else { coord(p) >= limit };
        let input = std::mem::take(&mut out);
        let Some(&last) = input.last() else { break };
        let mut prev = last;
        for &cur in &input {
            let (a, b) = (inside(prev), inside(cur));
            if a != b {
                let (pc, cc) = (coord(prev), coord(cur));
                let t = (limit - pc) / (cc - pc);
                out.push(prev.lerp(cur, t));
            }
            if b {
                out.push(cur);
            }
            prev = cur;
        }
    }
    out
}

#[cfg(test)]
mod tests;
