//! Bounding-box handle geometry shared by the Selection tool and the UI.

use vectorcraft_geom::{Affine, Point, Rect, Vec2};

/// The 8 handles, clockwise from top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    pub const ALL: [Handle; 8] =
        [Handle::TopLeft, Handle::Top, Handle::TopRight, Handle::Right, Handle::BottomRight, Handle::Bottom, Handle::BottomLeft, Handle::Left];

    pub fn pos(self, r: Rect) -> Point {
        let c = r.center();
        match self {
            Handle::TopLeft => Point::new(r.x0, r.y0),
            Handle::Top => Point::new(c.x, r.y0),
            Handle::TopRight => Point::new(r.x1, r.y0),
            Handle::Right => Point::new(r.x1, c.y),
            Handle::BottomRight => Point::new(r.x1, r.y1),
            Handle::Bottom => Point::new(c.x, r.y1),
            Handle::BottomLeft => Point::new(r.x0, r.y1),
            Handle::Left => Point::new(r.x0, c.y),
        }
    }
    pub fn opposite(self) -> Handle {
        Handle::ALL[(self as usize + 4) % 8]
    }
    pub fn is_corner(self) -> bool {
        (self as usize).is_multiple_of(2)
    }
    /// Which axes this handle scales (x, y).
    pub fn axes(self) -> (bool, bool) {
        match self {
            Handle::Top | Handle::Bottom => (false, true),
            Handle::Left | Handle::Right => (true, false),
            _ => (true, true),
        }
    }
    /// The handle's point in the 9-point reference grid (row-major from the top-left, 4 the center)
    /// that `object.setBounds` takes as `reference`.
    pub fn reference(self) -> usize {
        match self {
            Handle::TopLeft => 0,
            Handle::Top => 1,
            Handle::TopRight => 2,
            Handle::Left => 3,
            Handle::Right => 5,
            Handle::BottomLeft => 6,
            Handle::Bottom => 7,
            Handle::BottomRight => 8,
        }
    }
}

/// Handle under `p` (within `tol` document units).
pub fn hit_handle(r: Rect, p: Point, tol: f64) -> Option<Handle> {
    Handle::ALL.into_iter().find(|h| {
        let q = h.pos(r);
        (q.x - p.x).abs() <= tol && (q.y - p.y).abs() <= tol
    })
}

/// Is `p` in the rotate zone just outside a corner (Illustrator shows the curved rotate cursor there)?
pub fn in_rotate_zone(r: Rect, p: Point, inner: f64, outer: f64) -> Option<Handle> {
    [Handle::TopLeft, Handle::TopRight, Handle::BottomRight, Handle::BottomLeft].into_iter().find(|h| {
        let q = h.pos(r);
        let d = p.distance(q);
        d > inner && d <= outer && !r.contains(p)
    })
}

/// Scale transform for dragging `handle` of `r` to `p`.
/// `proportional` (Shift) keeps the aspect ratio; `from_center` (Alt) scales about the centre.
pub fn scale_for_drag(r: Rect, handle: Handle, p: Point, proportional: bool, from_center: bool) -> Affine {
    let origin = if from_center { r.center() } else { handle.opposite().pos(r) };
    let start = handle.pos(r);
    let (ax, ay) = handle.axes();
    let d0 = start - origin;
    let d1 = p - origin;
    let mut sx = if ax && d0.x.abs() > 1e-9 { d1.x / d0.x } else { 1.0 };
    let mut sy = if ay && d0.y.abs() > 1e-9 { d1.y / d0.y } else { 1.0 };
    if proportional {
        if handle.is_corner() {
            let s = if sx.abs() > sy.abs() { sx } else { sy };
            sx = s.abs() * sx.signum();
            sy = s.abs() * sy.signum();
        } else if ax {
            sy = sx.abs();
        } else {
            sx = sy.abs();
        }
    }
    // Avoid degenerate zero scales (Illustrator never collapses an object completely).
    let clamp = |s: f64| if s.abs() < 1e-4 { 1e-4 * if s < 0.0 { -1.0 } else { 1.0 } } else { s };
    Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(clamp(sx), clamp(sy)) * Affine::translate(-origin.to_vec2())
}

/// Rotation about `center` taking `from` to `to`; `snap` constrains to 45° steps.
pub fn rotate_for_drag(center: Point, from: Point, to: Point, snap: bool) -> (Affine, f64) {
    let a0 = (from - center).atan2();
    let a1 = (to - center).atan2();
    let mut a = a1 - a0;
    if snap {
        let step = std::f64::consts::FRAC_PI_4;
        a = (a / step).round() * step;
    }
    (Affine::translate(center.to_vec2()) * Affine::rotate(a) * Affine::translate(-center.to_vec2()), -a.to_degrees())
}

/// Move constrained to 45° multiples when `shift`.
pub fn move_delta(from: Point, to: Point, shift: bool) -> Vec2 {
    let d = to - from;
    if shift { vectorcraft_geom::constrain_angle(d, 45.0) } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect::new(0.0, 0.0, 100.0, 50.0);

    #[test]
    fn handle_positions() {
        assert_eq!(Handle::Right.pos(R), Point::new(100.0, 25.0));
        assert_eq!(Handle::TopLeft.opposite(), Handle::BottomRight);
        assert_eq!(Handle::Top.opposite(), Handle::Bottom);
        assert_eq!(hit_handle(R, Point::new(99.0, 24.0), 3.0), Some(Handle::Right));
        assert_eq!(hit_handle(R, Point::new(50.0, 25.0), 3.0), None);
        for h in Handle::ALL {
            assert_eq!(vectorcraft_geom::reference_point(R, h.reference()), h.pos(R), "{h:?}");
        }
    }

    #[test]
    fn scale_corner_free_and_proportional() {
        let a = scale_for_drag(R, Handle::BottomRight, Point::new(200.0, 50.0), false, false);
        assert_eq!(a.transform_rect_bbox(R), Rect::new(0.0, 0.0, 200.0, 50.0));
        let a = scale_for_drag(R, Handle::BottomRight, Point::new(200.0, 50.0), true, false);
        assert_eq!(a.transform_rect_bbox(R), Rect::new(0.0, 0.0, 200.0, 100.0));
    }

    #[test]
    fn scale_side_and_center() {
        let a = scale_for_drag(R, Handle::Right, Point::new(150.0, 999.0), false, false);
        assert_eq!(a.transform_rect_bbox(R), Rect::new(0.0, 0.0, 150.0, 50.0));
        let a = scale_for_drag(R, Handle::Right, Point::new(150.0, 25.0), false, true);
        assert_eq!(a.transform_rect_bbox(R), Rect::new(-50.0, 0.0, 150.0, 50.0));
    }

    #[test]
    fn rotate_snaps() {
        let (_, deg) = rotate_for_drag(Point::ZERO, Point::new(10.0, 0.0), Point::new(10.0, 9.0), true);
        assert!((deg + 45.0).abs() < 1e-9);
        assert!(in_rotate_zone(R, Point::new(-8.0, -8.0), 4.0, 20.0).is_some());
        assert!(in_rotate_zone(R, Point::new(5.0, 5.0), 4.0, 20.0).is_none());
    }
}
