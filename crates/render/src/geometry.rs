use super::{BezPath, Rect, kurbo};

/// Most dashes a dotted guide is drawn with (1.5 on, 2.5 off on screen); a longer one is solid.
pub(crate) const MAX_GUIDE_DASHES: f64 = 100_000.0;

/// The length of `p`'s control polygon: at least its own length (infinite when a point is).
pub(crate) fn polygon_length(p: &BezPath) -> f64 {
    let (mut last, mut len): (Option<kurbo::Point>, f64) = (None, 0.0);
    for el in p.elements() {
        let points = match *el {
            kurbo::PathEl::MoveTo(q) => {
                last = Some(q);
                continue;
            }
            kurbo::PathEl::LineTo(q) => [Some(q), None, None],
            kurbo::PathEl::QuadTo(a, b) => [Some(a), Some(b), None],
            kurbo::PathEl::CurveTo(a, b, c) => [Some(a), Some(b), Some(c)],
            kurbo::PathEl::ClosePath => continue,
        };
        for q in points.into_iter().flatten() {
            if let Some(l) = last {
                len += l.distance(q);
            }
            last = Some(q);
        }
    }
    if len.is_nan() { f64::INFINITY } else { len }
}

/// The pixel size of `region` rendered at `scale` pixels per point (at least 1×1).
pub fn region_pixels(region: Rect, scale: f64) -> (u32, u32) {
    ((region.width() * scale).round().max(1.0) as u32, (region.height() * scale).round().max(1.0) as u32)
}

pub(crate) fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}
