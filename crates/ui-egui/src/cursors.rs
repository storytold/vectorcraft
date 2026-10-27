//! Tool cursors drawn as vector glyphs (egui only offers system cursors). Black shapes with a white
//! halo, hotspot at `p`, Illustrator's visual grammar: solid arrow (Selection), hollow arrow (Direct
//! Selection), pen nib with state badges, crosshair for drawing tools, curved arrows for rotate.
//!
//! The desktop app shows them as OS cursors ([`Images`]): the system moves those itself, at once,
//! while a cursor painted into the window follows the pointer a few frames late (#444).

use egui::epaint::{Mesh, TessellationOptions, Tessellator, Vertex};
use egui::{Color32, CustomCursorImage, Painter, Pos2, Shape, Stroke, Vec2, pos2, vec2};
use vectorcraft_tools::Cursor;

const INK: Color32 = Color32::BLACK;
const HALO: Color32 = Color32::WHITE;

/// The shapes of one cursor glyph.
#[derive(Default)]
struct Ink(Vec<Shape>);

impl Ink {
    fn add(&mut self, s: Shape) {
        self.0.push(s);
    }
    fn line_segment(&mut self, pts: [Pos2; 2], stroke: Stroke) {
        self.add(Shape::line_segment(pts, stroke));
    }
    fn circle_stroke(&mut self, c: Pos2, r: f32, stroke: Stroke) {
        self.add(Shape::circle_stroke(c, r, stroke));
    }
    fn circle_filled(&mut self, c: Pos2, r: f32, fill: Color32) {
        self.add(Shape::circle_filled(c, r, fill));
    }
}

fn poly(p: &mut Ink, pts: Vec<Pos2>, fill: Color32, stroke: Color32) {
    // Halo first (thicker white outline), then the glyph.
    p.add(Shape::closed_line(pts.clone(), Stroke::new(3.0, HALO)));
    p.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
    p.add(Shape::closed_line(pts, Stroke::new(1.0, stroke)));
}

fn line(p: &mut Ink, a: Pos2, b: Pos2) {
    p.line_segment([a, b], Stroke::new(3.0, HALO));
    p.line_segment([a, b], Stroke::new(1.2, INK));
}

fn arrow_points(o: Pos2) -> Vec<Pos2> {
    // Classic pointer, tip at o.
    [(0.0, 0.0), (0.0, 15.0), (3.8, 11.4), (6.4, 17.0), (8.6, 16.0), (6.1, 10.6), (11.0, 10.6)].iter().map(|(x, y)| o + vec2(*x, *y)).collect()
}

fn arrow(p: &mut Ink, o: Pos2, hollow: bool) {
    let pts = arrow_points(o);
    // The arrow is concave: draw as a filled mesh of two convex parts, then outline.
    p.add(Shape::closed_line(pts.clone(), Stroke::new(3.0, HALO)));
    let fill = if hollow { HALO } else { INK };
    p.add(Shape::convex_polygon(vec![pts[0], pts[1], pts[2], pts[5], pts[6]], fill, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![pts[2], pts[3], pts[4], pts[5]], fill, Stroke::NONE));
    p.add(Shape::closed_line(pts, Stroke::new(1.0, INK)));
}

fn crosshair(p: &mut Ink, o: Pos2) {
    for (a, b) in
        [(vec2(-9.0, 0.0), vec2(-2.0, 0.0)), (vec2(2.0, 0.0), vec2(9.0, 0.0)), (vec2(0.0, -9.0), vec2(0.0, -2.0)), (vec2(0.0, 2.0), vec2(0.0, 9.0))]
    {
        line(p, o + a, o + b);
    }
}

/// A pipette pointing down-left: the sampling tip is the hotspot, with the tube and bulb above
/// right. Drawn here so bitmap and painted cursors share the same original glyph (#1269).
fn eyedropper(p: &mut Ink, o: Pos2) {
    let tube = [(0.0, 0.0), (1.0, -5.0), (14.0, -18.0), (18.0, -14.0), (5.0, -1.0)];
    poly(p, tube.iter().map(|(x, y)| o + vec2(*x, *y)).collect(), HALO, INK);
    let bulb = o + vec2(20.0, -20.0);
    p.circle_filled(bulb, 6.5, HALO);
    p.circle_filled(bulb, 5.0, INK);
    line(p, o + vec2(11.0, -19.0), o + vec2(19.0, -11.0));
}

/// The selection arrow's axis: the midline between its two edges at the tip, a unit vector.
/// The pen nib lies along it, so swapping between the Pen and the selection tools (Cmd/Ctrl)
/// doesn't rotate the cursor.
fn arrow_axis() -> Vec2 {
    let a = arrow_points(Pos2::ZERO);
    let (e1, e2) = (a[1] - a[0], a[6] - a[0]);
    (e1.normalized() + e2.normalized()).normalized()
}

/// The Pen family's nib: the pen-tool icon's, swung round onto the selection arrow's axis
/// (`arrow_axis`, down-right) so swapping to the selection tools doesn't rotate the cursor, its
/// point on the hotspot `o`. In nib coordinates `u` runs along the axis and `v` across it: the
/// silhouette is symmetric about the axis — the point, shoulders flaring to their widest
/// two-thirds back, then a narrower flat heel. The slit runs back along the axis from the point,
/// widening into the round breather hole; both are cut to the halo's white. Every Pen cursor is
/// this nib plus a badge below right of it saying what a click does.
fn pen(p: &mut Ink, o: Pos2, badge: &str) {
    let d = arrow_axis();
    let n = vec2(-d.y, d.x);
    let at = |u: f32, v: f32| o + u * d + v * n;
    poly(p, vec![at(0.0, 0.0), at(10.5, -6.3), at(15.5, -3.0), at(15.5, 3.0), at(10.5, 6.3)], INK, INK);
    // The slit is a wedge opening at the point, widening into the round breather hole.
    p.add(Shape::convex_polygon(vec![at(2.0, 0.0), at(10.4, -0.85), at(10.4, 0.85)], HALO, Stroke::NONE));
    p.circle_filled(at(11.8, 0.0), 1.7, HALO);
    let b = o + vec2(13.0, 12.0);
    match badge {
        "o" => {
            p.circle_stroke(b + vec2(3.0, 3.0), 3.0, Stroke::new(3.0, HALO));
            p.circle_stroke(b + vec2(3.0, 3.0), 3.0, Stroke::new(1.2, INK));
        }
        "+" => strokes(p, &[vec![b + vec2(0.0, 3.0), b + vec2(6.0, 3.0)], vec![b + vec2(3.0, 0.0), b + vec2(3.0, 6.0)]]),
        "-" => line(p, b + vec2(0.0, 3.0), b + vec2(6.0, 3.0)),
        "/" => line(p, b + vec2(0.0, 6.0), b + vec2(5.0, 0.0)),
        "~" => {
            // A quarter circle bulging up-left, from the badge's bottom-left to its top-right.
            let c = b + vec2(7.0, 7.0);
            let arc: Vec<Pos2> = (0..=8).map(|i| c + egui::Vec2::angled(std::f32::consts::PI * (1.0 + i as f32 / 16.0)) * 6.0).collect();
            p.add(Shape::line(arc.clone(), Stroke::new(3.0, HALO)));
            p.add(Shape::line(arc, Stroke::new(1.2, INK)));
        }
        // Join: a segment running into a square end point.
        "j" => {
            line(p, b + vec2(0.0, 6.0), b + vec2(3.5, 2.5));
            let sq = [(3.0, -1.0), (7.0, -1.0), (7.0, 3.0), (3.0, 3.0)].map(|(x, y)| b + vec2(x, y));
            poly(p, sq.to_vec(), INK, INK);
        }
        "^" => strokes(p, &[vec![b + vec2(0.0, 6.0), b + vec2(3.0, 0.0)], vec![b + vec2(3.0, 0.0), b + vec2(6.0, 6.0)]]),
        "*" => {
            // The new-path star: eight spokes on the badge's centre.
            strokes(
                p,
                &[
                    vec![b + vec2(0.6, 0.6), b + vec2(5.4, 5.4)],
                    vec![b + vec2(5.4, 0.6), b + vec2(0.6, 5.4)],
                    vec![b + vec2(3.0, 0.0), b + vec2(3.0, 6.0)],
                    vec![b + vec2(0.0, 3.0), b + vec2(6.0, 3.0)],
                ],
            );
        }
        _ => {}
    }
}

/// The Anchor Point tool's caret (as its toolbar icon): a small hollow square on the anchor at
/// the hotspot, legs spreading down from its lower corners. Every halo goes under every stroke, so
/// the legs join the square.
fn anchor_point(p: &mut Ink, o: Pos2) {
    let s = 2.5;
    let square = vec![o + vec2(-s, -s), o + vec2(s, -s), o + vec2(s, s), o + vec2(-s, s)];
    let legs = [[o + vec2(-s, s), o + vec2(-7.5, 13.0)], [o + vec2(s, s), o + vec2(7.5, 13.0)]];
    for leg in legs {
        p.line_segment(leg, Stroke::new(3.0, HALO));
    }
    p.add(Shape::closed_line(square.clone(), Stroke::new(3.0, HALO)));
    p.add(Shape::convex_polygon(square.clone(), HALO, Stroke::NONE));
    for leg in legs {
        p.line_segment(leg, Stroke::new(1.2, INK));
    }
    p.add(Shape::closed_line(square, Stroke::new(1.0, INK)));
}

/// Halos under every stroke first, then the ink, so crossing strokes don't cut each other.
fn strokes(p: &mut Ink, parts: &[Vec<Pos2>]) {
    for s in parts {
        p.add(Shape::line(s.clone(), Stroke::new(3.0, HALO)));
    }
    for s in parts {
        p.add(Shape::line(s.clone(), Stroke::new(1.2, INK)));
    }
}

/// A stemless arrowhead, tip at `o`.
fn arrowhead(o: Pos2) -> Vec<Pos2> {
    vec![o, o + vec2(0.0, 14.0), o + vec2(4.2, 10.4), o + vec2(10.0, 10.0)]
}

/// Direct Selection over an anchor: the white arrow with a small square at its lower right,
/// filled when the anchor's path is selected.
fn arrow_square(p: &mut Ink, o: Pos2, filled: bool) {
    arrow(p, o, true);
    let b = o + vec2(11.0, 13.0);
    let square = [(0.0, 0.0), (5.0, 0.0), (5.0, 5.0), (0.0, 5.0)].map(|(x, y)| b + vec2(x, y)).to_vec();
    poly(p, square, if filled { INK } else { HALO }, INK);
}

/// The Reshape Segment cursor: a black arrowhead with a small curve between two anchor marks at
/// its lower right.
fn reshape_segment(p: &mut Ink, o: Pos2) {
    poly(p, arrowhead(o), INK, INK);
    let (a, c) = (o + vec2(10.0, 19.0), o + vec2(19.0, 19.0));
    let curve: Vec<Pos2> = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            a + (c - a) * t - vec2(0.0, 4.5 * (std::f32::consts::PI * t).sin())
        })
        .collect();
    strokes(p, &[curve]);
    for m in [a, c] {
        let mark = [(-1.5, -1.5), (1.5, -1.5), (1.5, 1.5), (-1.5, 1.5)].map(|(x, y)| m + vec2(x, y)).to_vec();
        poly(p, mark, INK, INK);
    }
}

/// Group Selection: the white arrow with a plus at its lower right.
fn arrow_plus(p: &mut Ink, o: Pos2) {
    arrow(p, o, true);
    let c = o + vec2(14.0, 15.0);
    strokes(p, &[vec![c - vec2(3.5, 0.0), c + vec2(3.5, 0.0)], vec![c - vec2(0.0, 3.5), c + vec2(0.0, 3.5)]]);
}

/// An Alt-drag copy: a black arrowhead in front of a white one set down and to the right.
fn double_arrowhead(p: &mut Ink, o: Pos2) {
    poly(p, arrowhead(o + vec2(4.0, 4.0)), HALO, INK);
    poly(p, arrowhead(o), INK, INK);
}

fn double_arrow(p: &mut Ink, o: Pos2, dir: egui::Vec2) {
    let d = dir.normalized() * 8.0;
    let n = vec2(-d.y, d.x) * 0.45;
    line(p, o - d, o + d);
    for (tip, back) in [(o + d, o + d * 0.45), (o - d, o - d * 0.45)] {
        poly(p, vec![tip, back + n, back - n], INK, INK);
    }
}

/// Two single arrows side by side across the hotspot, pointing opposite ways along `dir`: a side
/// sliding past its opposite one, as a shear does.
fn shear(p: &mut Ink, o: Pos2, dir: egui::Vec2) {
    let d = dir.normalized() * 7.0;
    let n = vec2(-d.y, d.x) / 7.0;
    for (side, way) in [(-3.5, 1.0), (3.5, -1.0)] {
        let (c, d) = (o + n * side, d * way);
        line(p, c - d, c + d * 0.55);
        let back = c + d * 0.4;
        poly(p, vec![c + d, back + n * 2.6, back - n * 2.6], INK, INK);
    }
}

/// A curved double arrow over the hotspot: an arc with an arrowhead at each end, pointing on along
/// the arc (#701). The arc bows up, or toward `octant` (0 up, then clockwise in 45° steps): beside a
/// bounding box it wraps the corner the pointer is at.
fn rotate(p: &mut Ink, o: Pos2, octant: u8) {
    const R: f32 = 8.0;
    let (a0, a1) = (std::f32::consts::PI * 0.12, std::f32::consts::PI * 0.88);
    // Clockwise on screen (y down) by whole octants.
    let (s, c) = (f32::from(octant % 8) * std::f32::consts::FRAC_PI_4).sin_cos();
    let turn = |v: Vec2| vec2(v.x * c - v.y * s, v.x * s + v.y * c);
    let at = |a: f32| o + turn(vec2(a.cos() * R, -a.sin() * R));
    let pts: Vec<Pos2> = (0..=12).map(|i| at(a0 + (a1 - a0) * i as f32 / 12.0)).collect();
    p.add(Shape::line(pts.clone(), Stroke::new(3.2, HALO)));
    p.add(Shape::line(pts, Stroke::new(1.3, INK)));
    // Each head points away from the arc, along its tangent there.
    for (a, away) in [(a0, -1.0f32), (a1, 1.0)] {
        let t = turn(vec2(-a.sin(), -a.cos()) * away);
        let n = vec2(-t.y, t.x);
        let (end, tip) = (at(a) - t * 1.5, at(a) + t * 4.5);
        poly(p, vec![tip, end + n * 1.8, end - n * 1.8], INK, INK);
    }
}

/// Live Corners: the hollow arrow with a rounded-corner badge.
fn corner_radius(p: &mut Ink, o: Pos2) {
    arrow(p, o, true);
    let b = o + vec2(12.0, 13.0);
    let mut pts = vec![b + vec2(0.0, 10.0)];
    pts.extend((0..=8).map(|i| {
        let a = std::f32::consts::PI * (1.0 + 0.5 * i as f32 / 8.0);
        b + vec2(4.0 + 4.0 * a.cos(), 4.0 + 4.0 * a.sin())
    }));
    pts.push(b + vec2(10.0, 0.0));
    p.add(Shape::line(pts.clone(), Stroke::new(3.0, HALO)));
    p.add(Shape::line(pts, Stroke::new(1.2, INK)));
}

/// Over a bracket of type on a path: the arrow with a bracket (a stem standing on a baseline,
/// with a foot) below right.
fn path_bracket(p: &mut Ink, o: Pos2) {
    arrow(p, o, false);
    let b = o + vec2(13.0, 13.0);
    let stem = [b + vec2(3.0, 0.0), b + vec2(3.0, 10.0)];
    let foot = [b + vec2(3.0, 0.0), b + vec2(7.0, 0.0)];
    let base = [b + vec2(0.0, 8.0), b + vec2(10.0, 8.0)];
    for (w, c) in [(3.0, HALO), (1.2, INK)] {
        for l in [stem, foot, base] {
            p.line_segment(l, Stroke::new(w, c));
        }
    }
}

/// Over the type widget: the arrow with a type badge (a T) below right.
fn type_widget(p: &mut Ink, o: Pos2) {
    arrow(p, o, false);
    let b = o + vec2(12.0, 13.0);
    line(p, b, b + vec2(8.0, 0.0));
    line(p, b + vec2(4.0, 0.0), b + vec2(4.0, 9.0));
}

/// Overflow text the Selection tool carries after a click on the red out port: a small block of
/// lines at the hotspot (where a new frame's top-left would go).
fn loaded_text(p: &mut Ink, o: Pos2) {
    let tl = o;
    let br = o + vec2(14.0, 11.0);
    let frame = vec![tl, pos2(br.x, tl.y), br, pos2(tl.x, br.y)];
    poly(p, frame, HALO, INK);
    for y in [3.0, 6.5, 10.0] {
        line(p, tl + vec2(2.0, y), tl + vec2(12.0, y));
    }
}

/// The gradient annotator's stop cursors: the arrow with a plus (add a stop) or minus (delete it)
/// badge.
fn stop_badge(p: &mut Ink, o: Pos2, add: bool) {
    arrow(p, o, false);
    let b = o + vec2(13.0, 13.0);
    line(p, b, b + vec2(6.0, 0.0));
    if add {
        line(p, b + vec2(3.0, -3.0), b + vec2(3.0, 3.0));
    }
}

/// A slice badge: a small rectangle cut by a line, at `b` (its top left).
fn slice_badge(p: &mut Ink, b: Pos2) {
    let pts = vec![b, b + vec2(8.0, 0.0), b + vec2(8.0, 6.0), b + vec2(0.0, 6.0)];
    poly(p, pts, HALO, INK);
    line(p, b + vec2(4.0, 0.0), b + vec2(4.0, 6.0));
}

/// The Slice tool: a crosshair with a blade below right of the hotspot.
fn slice(p: &mut Ink, o: Pos2) {
    crosshair(p, o);
    let b = o + vec2(7.0, 7.0);
    poly(p, vec![b, b + vec2(9.0, 4.0), b + vec2(10.0, 7.0), b + vec2(3.0, 6.0)], HALO, INK);
    line(p, b + vec2(8.0, 6.0), b + vec2(12.0, 12.0));
}

/// The Width tool: the hollow arrow with a stroke that swells in the middle (a width point),
/// plus a badge: `+` over a stroke (a drag adds a point), a bar across the swell over a width point
/// (a drag moves or widens it).
fn width(p: &mut Ink, o: Pos2, badge: &str) {
    arrow(p, o, true);
    let b = o + vec2(11.0, 18.0);
    let top: Vec<Pos2> = (0..=8)
        .map(|i| {
            let t = i as f32 / 8.0;
            b + vec2(12.0 * t, -3.5 * (std::f32::consts::PI * t).sin())
        })
        .collect();
    let mut lens = top.clone();
    lens.extend(top.iter().rev().map(|q| pos2(q.x, 2.0 * b.y - q.y)));
    p.add(Shape::closed_line(lens.clone(), Stroke::new(3.0, HALO)));
    p.add(Shape::closed_line(lens, Stroke::new(1.2, INK)));
    match badge {
        "+" => {
            let c = b + vec2(16.0, -6.0);
            line(p, c - vec2(3.0, 0.0), c + vec2(3.0, 0.0));
            line(p, c - vec2(0.0, 3.0), c + vec2(0.0, 3.0));
        }
        "point" => line(p, b + vec2(6.0, -6.0), b + vec2(6.0, 6.0)),
        _ => {}
    }
}

fn ibeam(p: &mut Ink, o: Pos2) {
    line(p, o + vec2(0.0, -8.0), o + vec2(0.0, 8.0));
    line(p, o + vec2(-3.0, -8.0), o + vec2(3.0, -8.0));
    line(p, o + vec2(-3.0, 8.0), o + vec2(3.0, 8.0));
    line(p, o + vec2(-2.0, 3.0), o + vec2(2.0, 3.0));
}

/// The Blend tool: a crosshair with a square below right of the hotspot, hollow away from art,
/// filled over an object; over an anchor point a ringed dot (the blend starts there).
fn blend(p: &mut Ink, o: Pos2, badge: Cursor) {
    crosshair(p, o);
    let b = o + vec2(9.0, 9.0);
    if badge == Cursor::BlendAnchor {
        p.circle_stroke(b + vec2(3.5, 3.5), 3.5, Stroke::new(3.0, HALO));
        p.circle_stroke(b + vec2(3.5, 3.5), 3.5, Stroke::new(1.2, INK));
        p.circle_filled(b + vec2(3.5, 3.5), 1.4, INK);
        return;
    }
    let fill = if badge == Cursor::BlendObject { INK } else { HALO };
    poly(p, vec![b, b + vec2(7.0, 0.0), b + vec2(7.0, 7.0), b + vec2(0.0, 7.0)], fill, INK);
}

/// The Shape Builder: a crosshair with a plus badge below right of the hotspot (merge mode), or a
/// minus (erase mode).
fn shape_builder(p: &mut Ink, o: Pos2, erase: bool) {
    crosshair(p, o);
    let c = o + vec2(12.0, 12.0);
    line(p, c - vec2(3.5, 0.0), c + vec2(3.5, 0.0));
    if !erase {
        line(p, c - vec2(0.0, 3.5), c + vec2(0.0, 3.5));
    }
}

/// The Hand tool and Space: an open hand (palm and four fingers, thumb to the left), or the same
/// hand closed while it drags the view. Windows has no system hand that isn't a pointing finger
/// (winit maps egui's Grab and Grabbing to the four-headed move cursor), so it is drawn like the
/// other tool cursors. Hotspot in the palm.
fn hand(p: &mut Ink, o: Pos2, closed: bool) {
    // Thumb out to the left, four fingers up from the palm (open) or folded over it (closed),
    // then the palm, a rounded block below the hotspot, over their bases.
    let mut parts = vec![vec![o + vec2(-7.0, 0.0), o + vec2(-13.0, -4.0), o + vec2(-14.0, -1.0), o + vec2(-9.0, 5.0)]];
    for (i, x) in [-5.5f32, -1.75, 2.0, 5.75].into_iter().enumerate() {
        let top = if closed {
            -5.0
        } else if i == 0 || i == 3 {
            -11.0
        } else {
            -14.0
        };
        parts.push(vec![o + vec2(x - 1.5, 0.0), o + vec2(x - 1.5, top), o + vec2(x + 1.5, top), o + vec2(x + 1.5, 0.0)]);
    }
    parts.push(vec![o + vec2(-7.0, -2.0), o + vec2(7.25, -2.0), o + vec2(8.0, 6.0), o + vec2(4.0, 11.0), o + vec2(-5.0, 11.0), o + vec2(-9.0, 5.0)]);
    // Every halo first, so no part's halo covers another's outline.
    for part in &parts {
        p.add(Shape::closed_line(part.clone(), Stroke::new(3.0, HALO)));
    }
    for part in parts {
        p.add(Shape::convex_polygon(part.clone(), HALO, Stroke::NONE));
        p.add(Shape::closed_line(part, Stroke::new(1.0, INK)));
    }
}

/// The Zoom tool: a magnifier with a plus in the lens (Alt: a minus), the lens centred on the
/// hotspot, the handle down to the right.
fn magnifier(p: &mut Ink, o: Pos2, out: bool) {
    const R: f32 = 7.0;
    p.circle_stroke(o, R, Stroke::new(3.0, HALO));
    let d = vec2(R * 0.72, R * 0.72);
    line(p, o + d, o + d + vec2(7.0, 7.0));
    p.circle_filled(o, R, HALO);
    p.circle_stroke(o, R, Stroke::new(1.2, INK));
    // Inside the white lens, without halos: a halo would cut the plus's other bar.
    let bar = Stroke::new(1.2, INK);
    p.line_segment([o - vec2(3.5, 0.0), o + vec2(3.5, 0.0)], bar);
    if !out {
        p.line_segment([o - vec2(0.0, 3.5), o + vec2(0.0, 3.5)], bar);
    }
}

/// Every cursor with a glyph: its own, drawn here, or the one it borrows.
#[cfg(test)]
const GLYPHS: [Cursor; 81] = [
    Cursor::Hand,
    Cursor::HandGrab,
    Cursor::ZoomIn,
    Cursor::ZoomOut,
    Cursor::Arrow,
    Cursor::ArrowHollow,
    Cursor::DirectAnchor,
    Cursor::DirectAnchorSelected,
    Cursor::ReshapeSegment,
    Cursor::GroupSelection,
    Cursor::SelectionCopy,
    Cursor::Move,
    Cursor::Crosshair,
    Cursor::ResizeH,
    Cursor::ResizeV,
    Cursor::ResizeNwSe,
    Cursor::ResizeNeSw,
    Cursor::ShearH,
    Cursor::ShearV,
    Cursor::Rotate,
    Cursor::RotateToward(1),
    Cursor::CornerRadius,
    Cursor::Pen,
    Cursor::PenDrawing,
    Cursor::PenAdd,
    Cursor::PenDelete,
    Cursor::PenClose,
    Cursor::PenContinue,
    Cursor::PenJoin,
    Cursor::PenConvert,
    Cursor::AnchorPoint,
    Cursor::Curvature,
    Cursor::Text,
    Cursor::Eyedropper,
    Cursor::AddStop,
    Cursor::RemoveStop,
    Cursor::Slice,
    Cursor::SliceSelect,
    Cursor::Width,
    Cursor::WidthAdd,
    Cursor::WidthPoint,
    Cursor::Blend,
    Cursor::BlendObject,
    Cursor::BlendAnchor,
    Cursor::ObjectsOnPath,
    Cursor::PathBracket,
    Cursor::TypeWidget,
    Cursor::LoadedText,
    Cursor::ShapeBuilder,
    Cursor::ShapeBuilderErase,
    Cursor::Lasso,
    Cursor::MagicWand,
    Cursor::Pencil,
    Cursor::PencilNew,
    Cursor::PencilClose,
    Cursor::PencilStraight,
    Cursor::PencilContinue,
    Cursor::Paintbrush,
    Cursor::PaintbrushClose,
    Cursor::BlobBrush,
    Cursor::Eraser,
    Cursor::Smooth,
    Cursor::PathEraser,
    Cursor::Join,
    Cursor::Knife,
    Cursor::Shaper,
    Cursor::LivePaintBucket,
    Cursor::Measure,
    Cursor::PrintTiling,
    Cursor::SymbolSprayer,
    Cursor::SymbolShifter,
    Cursor::SymbolScruncher,
    Cursor::SymbolSizer,
    Cursor::SymbolSpinner,
    Cursor::SymbolStainer,
    Cursor::SymbolScreener,
    Cursor::SymbolStyler,
    Cursor::TypeVertical,
    Cursor::Scissors,
    Cursor::MeshAdd,
    Cursor::PuppetPin,
];

/// The shapes of cursor `c` with its hotspot at `p`; `None` for cursors that stay system cursors
/// (the busy states): the one drawn here, else the glyph of the cursor it borrows
/// ([`Cursor::stand_in`]).
fn glyph(c: Cursor, p: Pos2) -> Option<Vec<Shape>> {
    let ink = &mut Ink::default();
    // A cursor not drawn yet: the one it borrows.
    if let Some(stand_in) = c.stand_in() {
        return glyph(stand_in, p);
    }
    match c {
        Cursor::Arrow => arrow(ink, p, false),
        Cursor::ArrowHollow => arrow(ink, p, true),
        Cursor::DirectAnchor => arrow_square(ink, p, false),
        Cursor::DirectAnchorSelected => arrow_square(ink, p, true),
        Cursor::ReshapeSegment => reshape_segment(ink, p),
        Cursor::GroupSelection => arrow_plus(ink, p),
        Cursor::SelectionCopy => double_arrowhead(ink, p),
        Cursor::Move => {
            arrow(ink, p, false);
            double_arrow(ink, p + vec2(15.0, 18.0), vec2(1.0, 0.0));
        }
        Cursor::Crosshair => crosshair(ink, p),
        // The Pencil's crosshair with the Pen's star where the closing ring goes.
        Cursor::PencilNew => {
            crosshair(ink, p);
            let b = p + vec2(5.0, 5.0);
            strokes(ink, &[vec![b, b + vec2(6.0, 6.0)], vec![b + vec2(6.0, 0.0), b + vec2(0.0, 6.0)], vec![b + vec2(3.0, -1.0), b + vec2(3.0, 7.0)]]);
        }
        // The crosshair the Pencil and the Paintbrush borrow, with the PenClose ring below right.
        Cursor::PencilClose | Cursor::PaintbrushClose => {
            crosshair(ink, p);
            ink.circle_stroke(p + vec2(8.0, 8.0), 3.0, Stroke::new(3.0, HALO));
            ink.circle_stroke(p + vec2(8.0, 8.0), 3.0, Stroke::new(1.2, INK));
        }
        // The Pencil's crosshair with a short straight line where the closing ring goes.
        Cursor::PencilStraight => {
            crosshair(ink, p);
            line(ink, p + vec2(5.0, 12.0), p + vec2(12.0, 5.0));
        }
        // The straight-line cursor's line leaving a filled square: the end point a line continues from.
        Cursor::PencilContinue => {
            crosshair(ink, p);
            // The square first, so its halo does not cover the line.
            let end = p + vec2(5.0, 12.0);
            poly(ink, [(-2.0, -2.0), (2.0, -2.0), (2.0, 2.0), (-2.0, 2.0)].map(|(x, y)| end + vec2(x, y)).to_vec(), INK, INK);
            line(ink, end, p + vec2(12.0, 5.0));
        }
        Cursor::Eyedropper => eyedropper(ink, p),
        Cursor::ResizeH => double_arrow(ink, p, vec2(1.0, 0.0)),
        Cursor::ResizeV => double_arrow(ink, p, vec2(0.0, 1.0)),
        Cursor::ResizeNwSe => double_arrow(ink, p, vec2(1.0, 1.0)),
        Cursor::ResizeNeSw => double_arrow(ink, p, vec2(1.0, -1.0)),
        Cursor::ShearH => shear(ink, p, vec2(1.0, 0.0)),
        Cursor::ShearV => shear(ink, p, vec2(0.0, -1.0)),
        Cursor::Rotate => rotate(ink, p, 0),
        Cursor::RotateToward(octant) => rotate(ink, p, octant),
        Cursor::CornerRadius => corner_radius(ink, p),
        Cursor::Pen => pen(ink, p, "*"),
        Cursor::PenDrawing => pen(ink, p, ""),
        Cursor::PenAdd => pen(ink, p, "+"),
        Cursor::PenDelete => pen(ink, p, "-"),
        Cursor::PenClose => pen(ink, p, "o"),
        Cursor::PenContinue => pen(ink, p, "/"),
        Cursor::AnchorPoint => anchor_point(ink, p),
        Cursor::Curvature => pen(ink, p, "~"),
        Cursor::PenJoin => pen(ink, p, "j"),
        Cursor::PenConvert => pen(ink, p, "^"),
        Cursor::Text => ibeam(ink, p),
        Cursor::AddStop => stop_badge(ink, p, true),
        Cursor::RemoveStop => stop_badge(ink, p, false),
        Cursor::Slice => slice(ink, p),
        Cursor::SliceSelect => {
            arrow(ink, p, false);
            slice_badge(ink, p + vec2(11.0, 14.0));
        }
        Cursor::Width => width(ink, p, ""),
        Cursor::WidthAdd => width(ink, p, "+"),
        Cursor::WidthPoint => width(ink, p, "point"),
        Cursor::Blend | Cursor::BlendObject | Cursor::BlendAnchor => blend(ink, p, c),
        Cursor::PathBracket => path_bracket(ink, p),
        Cursor::TypeWidget => type_widget(ink, p),
        Cursor::LoadedText => loaded_text(ink, p),
        Cursor::ShapeBuilder => shape_builder(ink, p, false),
        Cursor::ShapeBuilderErase => shape_builder(ink, p, true),
        Cursor::Hand => hand(ink, p, false),
        Cursor::HandGrab => hand(ink, p, true),
        Cursor::ZoomIn => magnifier(ink, p, false),
        Cursor::ZoomOut => magnifier(ink, p, true),
        _ => return None,
    }
    Some(std::mem::take(&mut ink.0))
}

/// Paint cursor `c` at `p`, `scale` times its size, on the given (foreground) painter, where the
/// window can't show it as an OS cursor ([`OS_CURSORS`]). Returns false for cursors that stay
/// system cursors.
pub fn paint(painter: &Painter, c: Cursor, p: Pos2, scale: f32) -> bool {
    let place = egui::emath::TSTransform::new(p.to_vec2(), scale);
    glyph(c, Pos2::ZERO)
        .map(|shapes| {
            painter.extend(shapes.into_iter().map(|mut s| {
                s.transform(place);
                s
            }))
        })
        .is_some()
}

/// Does the window show the glyphs as OS cursors ([`Images`])? Not on macOS, which sizes a cursor
/// bitmap in points (a Retina one would show twice as big, or blurred), nor on the web (eframe has no
/// bitmap cursors there): those paint them into the window ([`paint`]).
pub const OS_CURSORS: bool = !cfg!(any(target_os = "macos", target_arch = "wasm32"));

/// Does this window show the glyphs as OS cursors, on a build that can ([`OS_CURSORS`])? Not on
/// Wayland, where the compositor decides how big a cursor bitmap shows: Hyprland shows each pixel
/// as a point, twice too big on a 2× display (#1645). The window paints them there, as PhotoCraft
/// and PdfCraft do.
pub fn os_cursors(build: bool, wayland: bool) -> bool {
    build && !wayland
}

/// Does the canvas hide the system pointer to show a painted tool glyph alone? Not when there is no
/// cursor theme on Wayland ([`crate::Services::no_cursor_theme`]): winit sets no pointer then, and a
/// hidden one would not come back over the panels (#2450). The system pointer stays beside the glyph.
pub fn hide_system_pointer(no_cursor_theme: bool) -> bool {
    !no_cursor_theme
}

/// Does the window hide the pointer only while it is the key window? macOS does: winit hides it with
/// a cursor rect holding a transparent cursor, and macOS applies the rect only in the key window.
/// Over a window behind another app or under a sheet, the system pointer shows (#2020).
pub const HIDES_ONLY_WHEN_KEY: bool = cfg!(target_os = "macos");

/// Does the window paint the glyphs over a hidden pointer, `focused` or not? Not where the system
/// shows its pointer over a window without the focus ([`HIDES_ONLY_WHEN_KEY`]): the system pointer
/// shows alone there.
pub fn paint_glyphs(hides_only_when_key: bool, focused: bool) -> bool {
    focused || !hides_only_when_key
}

/// The cursor bitmaps handed to the OS, by (cursor, pixels per point): each glyph is rasterized
/// once, and handing the same `Arc` every frame keeps the window's OS cursor instead of making a
/// new one.
#[derive(Default)]
pub struct Images(Vec<(Cursor, f32, CustomCursorImage)>);

impl Images {
    /// Cursor `c` for a display of `ppp` physical pixels per point; `None` for cursors that stay
    /// system cursors.
    pub fn get(&mut self, c: Cursor, ppp: f32) -> Option<CustomCursorImage> {
        if let Some((.., image)) = self.0.iter().find(|(k, s, _)| *k == c && *s == ppp) {
            return Some(image.clone());
        }
        let image = rasterize(c, ppp)?;
        // A few dozen glyphs at the scales of the displays the window visited.
        if self.0.len() >= 128 {
            self.0.clear();
        }
        self.0.push((c, ppp, image.clone()));
        Some(image)
    }
}

/// The largest side of a cursor bitmap, in pixels (the glyphs span under 50 points).
const MAX_SIDE: f32 = 256.0;

/// Cursor `c` as a straight-alpha RGBA bitmap at `ppp` pixels per point, tessellated (anti-aliased)
/// as egui would paint it, the hotspot on a pixel corner.
fn rasterize(c: Cursor, ppp: f32) -> Option<CustomCursorImage> {
    if !(ppp.is_finite() && ppp > 0.0) {
        return None;
    }
    let shapes = glyph(c, Pos2::ZERO)?;
    let mut tessellator = Tessellator::new(ppp, TessellationOptions::default(), [1, 1], vec![]);
    let mut mesh = Mesh::default();
    for shape in shapes {
        tessellator.tessellate_shape(shape, &mut mesh);
    }
    let bounds = mesh.calc_bounds();
    // The hotspot must lie inside the bitmap.
    let min = (bounds.min.to_vec2() * ppp).floor().min(Vec2::ZERO);
    let max = (bounds.max.to_vec2() * ppp).ceil().max(Vec2::splat(1.0));
    let size = max - min;
    if !(size.x <= MAX_SIDE && size.y <= MAX_SIDE) {
        return None;
    }
    let (w, h) = (size.x as usize, size.y as usize);
    let mut px = vec![[0.0; 4]; w * h];
    let corner = |v: &Vertex| (v.pos.to_vec2() * ppp - min, v.color);
    for &[i, j, k] in mesh.indices.as_chunks::<3>().0 {
        if let (Some(a), Some(b), Some(c)) = (mesh.vertices.get(i as usize), mesh.vertices.get(j as usize), mesh.vertices.get(k as usize)) {
            fill_triangle(&mut px, w, [corner(a), corner(b), corner(c)]);
        }
    }
    let rgba: Vec<u8> = px
        .iter()
        .flat_map(|&[r, g, b, a]| {
            let k = if a > 0.0 { 1.0 / a } else { 0.0 };
            [r * k, g * k, b * k, a].map(|x| (x * 255.0).round().clamp(0.0, 255.0) as u8)
        })
        .collect();
    Some(CustomCursorImage { rgba: rgba.into(), size: [w as u16, h as u16], hotspot: [-min.x as u16, -min.y as u16] })
}

/// Blend triangle `t` (pixel positions and premultiplied colours, interpolated across it) over `px`
/// (premultiplied RGBA, `w` pixels a row) at the pixel centres it covers, as the GPU blends egui's
/// meshes. A centre on an edge goes to one of the two triangles sharing it, so seams aren't doubled.
fn fill_triangle(px: &mut [[f32; 4]], w: usize, t: [(Vec2, Color32); 3]) {
    let edge = |a: Vec2, b: Vec2, p: Vec2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let [a, mut b, mut c] = t;
    let mut area = edge(a.0, b.0, c.0);
    if area < 0.0 {
        std::mem::swap(&mut b, &mut c);
        area = -area;
    }
    if area.is_nan() || area <= 1e-6 || w == 0 {
        return;
    }
    // Of the two triangles sharing an edge (walking it in opposite directions), one owns it.
    let owns = |u: Vec2, v: Vec2| v.y > u.y || (v.y == u.y && v.x > u.x);
    let h = px.len() / w;
    let lo = a.0.min(b.0).min(c.0).max(Vec2::ZERO);
    let hi = a.0.max(b.0).max(c.0).min(vec2(w as f32, h as f32));
    for y in lo.y.floor() as usize..hi.y.ceil() as usize {
        for x in lo.x.floor() as usize..hi.x.ceil() as usize {
            let p = vec2(x as f32 + 0.5, y as f32 + 0.5);
            let weights = [(b.0, c.0), (c.0, a.0), (a.0, b.0)].map(|(u, v)| (edge(u, v, p), owns(u, v)));
            if !weights.iter().all(|&(e, own)| e > 0.0 || (e == 0.0 && own)) {
                continue;
            }
            let Some(dst) = px.get_mut(y * w + x) else { continue };
            let [wa, wb, wc] = weights.map(|(e, _)| e / area);
            let src: [f32; 4] = std::array::from_fn(|i| {
                let channel = |col: Color32| f32::from(col.to_array()[i]);
                (wa * channel(a.1) + wb * channel(b.1) + wc * channel(c.1)) / 255.0
            });
            let keep = 1.0 - src[3];
            *dst = std::array::from_fn(|i| src[i] + dst[i] * keep);
        }
    }
}

#[cfg(test)]
mod tests;
