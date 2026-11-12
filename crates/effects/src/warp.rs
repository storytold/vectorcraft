//! Warp effects: 15 envelope styles plus horizontal/vertical perspective distortion.
//!
//! A warp with `version` [`WARP_NET`] is its style's Bézier patch over the box ([`WarpNet`]);
//! one without (saved before the version existed) is a map on normalized box coordinates
//! (x, y) ∈ [-1, 1]² (y down), [`warp_point`]. The path is split into short pieces whose control
//! points are mapped (see `map_nonlinear`).

use serde_json::Value;
use vectorcraft_geom::{PathData, Point, Rect};

use crate::util::{map_nonlinear_within, num, text};

pub use vectorcraft_doc::live::{WARP_NET, WarpNet, WarpStyle, uses_net, warp_point, warp_version};

/// The map version warp params `p` ask for: their `version`, 0 (the first map) when absent.
pub fn map_version(p: &Value) -> u32 {
    let v = num(p, "version", 0.0);
    if v.is_finite() && v >= 0.0 { v.min(f64::from(u32::MAX)) as u32 } else { 0 }
}

/// Apply a warp to `path` using box `b`, making at most `max` anchors ([`map_nonlinear_within`]).
pub fn warp(path: &PathData, b: Rect, style: WarpStyle, p: &Value, max: usize) -> PathData {
    let piece = b.width().hypot(b.height()) / 24.0;
    map_nonlinear_within(path, piece, max, warp_map(b, style, p))
}

/// The point map a warp of `style` with params `p` applies to box `b`: what [`warp`] moves each
/// point of a path to.
pub(crate) fn warp_map(b: Rect, style: WarpStyle, p: &Value) -> Box<dyn Fn(Point) -> Point> {
    let bend = num(p, "bend", 50.0).clamp(-100.0, 100.0) / 100.0;
    let dh = num(p, "horizontal", 0.0).clamp(-100.0, 100.0) / 100.0;
    let dv = num(p, "vertical", 0.0).clamp(-100.0, 100.0) / 100.0;
    let vertical = text(p, "orientation", "horizontal").eq_ignore_ascii_case("vertical");
    if uses_net(map_version(p)) {
        let net = WarpNet::new(style, bend, dh, dv, b.width(), b.height(), vertical);
        let (w, h) = (b.width(), b.height());
        // A box with no width or no height takes the middle of the patch along that side.
        return Box::new(move |q: Point| {
            let (u, v) = (if w > 0.0 { (q.x - b.x0) / w } else { 0.5 }, if h > 0.0 { (q.y - b.y0) / h } else { 0.5 });
            b.origin() + net.at(u, v).to_vec2()
        });
    }
    let c = b.center();
    let hw = (b.width() / 2.0).max(1e-9);
    let hh = (b.height() / 2.0).max(1e-9);
    Box::new(move |q: Point| {
        let (x, y) = ((q.x - c.x) / hw, (q.y - c.y) / hh);
        let (x2, y2) = if vertical {
            let (a, b2) = warp_point(style, bend, dh, dv, hw / hh, y, x);
            (b2, a)
        } else {
            warp_point(style, bend, dh, dv, hh / hw, x, y)
        };
        Point::new(c.x + x2 * hw, c.y + y2 * hh)
    })
}

/// Does warp effect `id` with params `p` move any point? Every style leaves the box as it is with
/// no bend and no distortion. With the first map, Arc (`warp.arc`) takes a bend under 0.0001 % as
/// none: the threshold must match the one in [`warp_point`]'s Arc case
/// (`warp_bends_when_the_warp_moves_the_outline` tests both sides of it). With [`WarpNet`] any
/// bend moves the outline.
pub(crate) fn bends(id: &str, p: &Value) -> bool {
    let share = |key: &str, default: f64| num(p, key, default).clamp(-100.0, 100.0) / 100.0;
    let bend = share("bend", 50.0);
    let bent = if id == "warp.arc" && !uses_net(map_version(p)) { bend.abs() >= 1e-6 } else { bend != 0.0 };
    bent || share("horizontal", 0.0) != 0.0 || share("vertical", 0.0) != 0.0
}
