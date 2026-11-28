//! View → Perspective Grid options on the grid: Snap to Grid (plane coordinates land on
//! gridlines), the gridline colours and opacity, and Show Rulers (a ruler up the vertical line
//! where the planes meet, in Define Grid's real-world units).

use vectorcraft_doc::Unit;
use vectorcraft_geom::{Point, Rect, Vec2};

use super::{PerspectiveGrid, Plane};
use crate::Overlay;

/// Snap to Grid reaches this fraction of a cell from a gridline.
const SNAP: f64 = 0.25;

/// Ruler ticks are this many screen pixels long, and labels at least this far apart.
const TICK_PX: f64 = 6.0;
const LABEL_PX: f64 = 36.0;

impl PerspectiveGrid {
    /// The gridline (a multiple of the cell) within a quarter cell of `v`, if any.
    fn gridline_near(&self, v: f64) -> Option<f64> {
        let line = (v / self.cell).round() * self.cell;
        ((line - v).abs() <= SNAP * self.cell).then_some(line)
    }

    /// Plane coordinates `q` on the nearest gridlines within a quarter cell (each axis).
    pub fn snap_plane(&self, q: Point) -> Point {
        Point::new(self.gridline_near(q.x).unwrap_or(q.x), self.gridline_near(q.y).unwrap_or(q.y))
    }

    /// A plane-space move `d` of art spanning `b` (plane coordinates) adjusted so the edge nearer
    /// a gridline on each axis lands on it, within a quarter cell.
    pub fn snap_offset(&self, b: Rect, d: Vec2) -> Vec2 {
        let axis = |lo: f64, hi: f64, d: f64| {
            let pulls = [lo + d, hi + d].map(|e| self.gridline_near(e).map(|l| l - e));
            pulls.into_iter().flatten().min_by(|a, b| a.abs().total_cmp(&b.abs())).map_or(d, |c| d + c)
        };
        Vec2::new(axis(b.x0, b.x1, d.x), axis(b.y0, b.y1, d.y))
    }

    /// The map that slides art lying on `plane` by `d` in plane coordinates.
    pub fn move_map_by(&self, plane: Plane, d: Vec2) -> Option<impl Fn(Point) -> Option<Point> + use<>> {
        let h = self.homography(plane)?;
        let hi = h.inverse()?;
        Some(move |p: Point| h.apply(hi.apply(p)? + d))
    }

    /// Lock Station Point: the left (`left`) or right vanishing point dragged to `x` turns the
    /// view around the station point, which stays put: the other vanishing point moves so the
    /// station still sees them at a right angle. A drag past the centre of vision is ignored.
    pub fn swing(&mut self, left: bool, x: f64) {
        let st = self.station();
        let off = if left { st.x - x } else { x - st.x };
        if off <= 1e-6 || st.distance <= 0.0 {
            return;
        }
        let a = off.atan2(st.distance).to_degrees();
        self.set_station(st, if left { 90.0 - a } else { a });
    }

    /// A plane's gridline colour (Define Grid).
    pub fn plane_color(&self, plane: Plane) -> [u8; 3] {
        match plane {
            Plane::Left => self.left_color.0,
            Plane::Right => self.right_color.0,
            Plane::Ground => self.ground_color.0,
            Plane::None => plane.color(),
        }
    }

    /// A plane's gridline colour with the grid's opacity.
    pub fn line_color(&self, plane: Plane) -> [u8; 4] {
        let [r, g, b] = self.plane_color(plane);
        [r, g, b, (self.opacity.clamp(0.0, 100.0) * 2.55).round() as u8]
    }

    /// Show Rulers: ticks at the gridlines up the vertical line where the planes meet (the true
    /// height line), labelled in Define Grid's real-world units. `tol` = document units per
    /// screen pixel.
    pub fn ruler_overlays(&self, tol: f64) -> Vec<Overlay> {
        let Some(h) = self.homography(Plane::Left) else { return vec![] };
        let unit = Unit::named(&self.units).unwrap_or_default();
        let ratio = unit.points() * self.scale[0] / self.scale[1];
        let color = [0x40, 0x40, 0x40];
        let mut out = vec![];
        let (Some(o), Some(top)) = (h.apply(Point::ZERO), h.apply(Point::new(0.0, self.height))) else { return out };
        out.push(Overlay::Line { a: o, b: top, color, dashed: false });
        // Labels every k cells, k the least that keeps them LABEL_PX apart at the origin.
        let cell_px = self.cell / tol.max(1e-9);
        let every = (LABEL_PX / cell_px.max(1e-9)).ceil().clamp(1.0, 1.0e6) as u64;
        let tick = TICK_PX * tol;
        let count = (self.height / self.cell).floor().min(2000.0) as u64;
        for i in 0..=count {
            let v = i as f64 * self.cell;
            let Some(p) = h.apply(Point::new(0.0, v)) else { continue };
            let long = i % every == 0;
            out.push(Overlay::Line { a: p, b: p + Vec2::new(if long { -2.0 * tick } else { -tick }, 0.0), color, dashed: false });
            if long {
                let text = format!("{} {}", unit.number(unit.to_pt(v / ratio)), unit.suffix());
                out.push(Overlay::Label { p: p + Vec2::new(-2.0 * tick, 0.0), text, color });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> PerspectiveGrid {
        PerspectiveGrid { cell: 20.0, ..PerspectiveGrid::normal(2, Rect::new(0.0, 0.0, 800.0, 600.0)) }
    }

    #[test]
    fn snapping_lands_within_a_quarter_cell_on_gridlines() {
        let g = grid();
        assert_eq!(g.snap_plane(Point::new(43.0, 53.0)), Point::new(40.0, 53.0));
        assert_eq!(g.snap_plane(Point::new(-4.9, 61.0)), Point::new(0.0, 60.0));
        // The nearer edge lands on a line: the right one (1 from 60) rather than the left (3 from 40).
        let b = Rect::new(10.0, 10.0, 34.0, 20.0);
        assert_eq!(g.snap_offset(b, Vec2::new(27.0, 30.0)), Vec2::new(26.0, 30.0));
        assert_eq!(g.snap_offset(b, Vec2::new(30.0, 0.0)), Vec2::new(30.0, 0.0), "an edge on a line stays");
        assert_eq!(g.snap_offset(b, Vec2::new(18.0, 0.0)), Vec2::new(18.0, 0.0), "nothing within reach");
    }

    #[test]
    fn a_tiny_stored_grid_draws_a_bounded_number_of_lines() {
        // A stored grid can carry extreme values: the loop bound must not depend on a fixed
        // tolerance swallowing the whole domain.
        let g = PerspectiveGrid { cell: 1e-18, extent: 1e-18, height: 1e-18, ..PerspectiveGrid::normal(2, Rect::new(0.0, 0.0, 800.0, 600.0)) };
        for pl in Plane::ALL {
            assert!(g.lines(pl).len() <= 121 + 121, "at most 121 lines a way on {pl:?}");
        }
    }

    #[test]
    fn colours_and_rulers_come_from_the_definition() {
        let mut g = grid();
        g.opacity = 40.0;
        g.right_color = super::super::Rgb([1, 2, 3]);
        assert_eq!(g.line_color(Plane::Right), [1, 2, 3, 102]);
        g.units = "inches".into();
        g.cell = 72.0;
        let labels: Vec<String> =
            g.ruler_overlays(1.0).into_iter().filter_map(|o| if let Overlay::Label { text, .. } = o { Some(text) } else { None }).collect();
        assert_eq!(labels.first().map(String::as_str), Some("0 in"));
        assert_eq!(labels.get(1).map(String::as_str), Some("1 in"));
        g.scale = [1.0, 4.0];
        let second = g.ruler_overlays(1.0).into_iter().filter_map(|o| if let Overlay::Label { text, .. } = o { Some(text) } else { None }).nth(1);
        assert_eq!(second.as_deref(), Some("4 in"), "real-world lengths at 1:4");
    }
}
