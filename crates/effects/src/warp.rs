//! Warp effects: 15 envelope styles plus horizontal/vertical perspective distortion.
//!
//! Each style is a map on normalised box coordinates (x, y) ∈ [-1, 1]² (y down). The path is
//! split into short pieces whose control points are mapped (see `map_nonlinear`).

use serde_json::Value;
use vectorcraft_geom::{PathData, Point, Rect};

use crate::util::{map_nonlinear_within, num, text};

pub use vectorcraft_doc::live::{WarpStyle, warp_point};

/// Apply a warp to `path` using box `b`, making at most `max` anchors ([`map_nonlinear_within`]).
pub fn warp(path: &PathData, b: Rect, style: WarpStyle, p: &Value, max: usize) -> PathData {
    let bend = num(p, "bend", 50.0).clamp(-100.0, 100.0) / 100.0;
    let dh = num(p, "horizontal", 0.0).clamp(-100.0, 100.0) / 100.0;
    let dv = num(p, "vertical", 0.0).clamp(-100.0, 100.0) / 100.0;
    let vertical = text(p, "orientation", "horizontal").eq_ignore_ascii_case("vertical");
    let c = b.center();
    let hw = (b.width() / 2.0).max(1e-9);
    let hh = (b.height() / 2.0).max(1e-9);
    let f = |q: Point| {
        let (x, y) = ((q.x - c.x) / hw, (q.y - c.y) / hh);
        let (x2, y2) = if vertical {
            let (a, b2) = warp_point(style, bend, dh, dv, hw / hh, y, x);
            (b2, a)
        } else {
            warp_point(style, bend, dh, dv, hh / hw, x, y)
        };
        Point::new(c.x + x2 * hw, c.y + y2 * hh)
    };
    let piece = b.width().hypot(b.height()) / 24.0;
    map_nonlinear_within(path, piece, max, f)
}

/// Does warp effect `id` with params `p` move any point? Every style leaves the box as it is with
/// no bend and no distortion, and Arc (`warp.arc`) takes a bend under 0.0001 % as none.
pub(crate) fn bends(id: &str, p: &Value) -> bool {
    let share = |key: &str, default: f64| num(p, key, default).clamp(-100.0, 100.0) / 100.0;
    let bend = share("bend", 50.0);
    let bent = if id == "warp.arc" { bend.abs() >= 1e-6 } else { bend != 0.0 };
    bent || share("horizontal", 0.0) != 0.0 || share("vertical", 0.0) != 0.0
}
