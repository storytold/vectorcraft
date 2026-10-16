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

fn pen(p: &mut Ink, o: Pos2, badge: &str) {
    // Nib pointing to the top-left hotspot.
    let pts = vec![o, o + vec2(4.0, 12.0), o + vec2(8.0, 16.0), o + vec2(16.0, 8.0), o + vec2(12.0, 4.0)];
    poly(p, pts, INK, INK);
    p.circle_filled(o + vec2(6.0, 6.0), 1.4, HALO);
    line(p, o + vec2(10.0, 14.0), o + vec2(14.0, 18.0));
    let b = o + vec2(16.0, 14.0);
    match badge {
        "o" => {
            p.circle_stroke(b + vec2(3.0, 3.0), 3.0, Stroke::new(3.0, HALO));
            p.circle_stroke(b + vec2(3.0, 3.0), 3.0, Stroke::new(1.2, INK));
        }
        "+" => {
            line(p, b + vec2(0.0, 3.0), b + vec2(6.0, 3.0));
            line(p, b + vec2(3.0, 0.0), b + vec2(3.0, 6.0));
        }
        "-" => line(p, b + vec2(0.0, 3.0), b + vec2(6.0, 3.0)),
        "/" => line(p, b + vec2(0.0, 6.0), b + vec2(5.0, 0.0)),
        // Join: a segment running into a square end point.
        "j" => {
            line(p, b + vec2(0.0, 6.0), b + vec2(3.5, 2.5));
            let sq = [(3.0, -1.0), (7.0, -1.0), (7.0, 3.0), (3.0, 3.0)].map(|(x, y)| b + vec2(x, y));
            poly(p, sq.to_vec(), INK, INK);
        }
        "^" => {
            line(p, b + vec2(0.0, 6.0), b + vec2(3.0, 0.0));
            line(p, b + vec2(3.0, 0.0), b + vec2(6.0, 6.0));
        }
        "*" => {
            line(p, b + vec2(0.0, 0.0), b + vec2(6.0, 6.0));
            line(p, b + vec2(6.0, 0.0), b + vec2(0.0, 6.0));
            line(p, b + vec2(3.0, -1.0), b + vec2(3.0, 7.0));
        }
        _ => {}
    }
}

fn double_arrow(p: &mut Ink, o: Pos2, dir: egui::Vec2) {
    let d = dir.normalized() * 8.0;
    let n = vec2(-d.y, d.x) * 0.45;
    line(p, o - d, o + d);
    for (tip, back) in [(o + d, o + d * 0.45), (o - d, o - d * 0.45)] {
        poly(p, vec![tip, back + n, back - n], INK, INK);
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

/// The shapes of cursor `c` with its hotspot at `p`; `None` for cursors that stay system cursors
/// (the busy states).
fn glyph(c: Cursor, p: Pos2) -> Option<Vec<Shape>> {
    let ink = &mut Ink::default();
    match c {
        Cursor::Arrow => arrow(ink, p, false),
        Cursor::ArrowHollow => arrow(ink, p, true),
        Cursor::Move => {
            arrow(ink, p, false);
            double_arrow(ink, p + vec2(15.0, 18.0), vec2(1.0, 0.0));
        }
        Cursor::Crosshair => crosshair(ink, p),
        Cursor::Eyedropper => eyedropper(ink, p),
        Cursor::ResizeH => double_arrow(ink, p, vec2(1.0, 0.0)),
        Cursor::ResizeV => double_arrow(ink, p, vec2(0.0, 1.0)),
        Cursor::ResizeNwSe => double_arrow(ink, p, vec2(1.0, 1.0)),
        Cursor::ResizeNeSw => double_arrow(ink, p, vec2(1.0, -1.0)),
        Cursor::Rotate => rotate(ink, p, 0),
        Cursor::RotateToward(octant) => rotate(ink, p, octant),
        Cursor::CornerRadius => corner_radius(ink, p),
        Cursor::Pen => pen(ink, p, "*"),
        Cursor::PenDrawing => pen(ink, p, ""),
        Cursor::PenAdd => pen(ink, p, "+"),
        Cursor::PenDelete => pen(ink, p, "-"),
        Cursor::PenClose => pen(ink, p, "o"),
        Cursor::PenContinue => pen(ink, p, "/"),
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
mod tests {
    use std::sync::Arc;

    use super::*;
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    /// Every cursor with a glyph.
    const GLYPHS: [Cursor; 39] = [
        Cursor::Hand,
        Cursor::HandGrab,
        Cursor::ZoomIn,
        Cursor::ZoomOut,
        Cursor::Arrow,
        Cursor::ArrowHollow,
        Cursor::Move,
        Cursor::Crosshair,
        Cursor::ResizeH,
        Cursor::ResizeV,
        Cursor::ResizeNwSe,
        Cursor::ResizeNeSw,
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
        Cursor::PathBracket,
        Cursor::TypeWidget,
        Cursor::ShapeBuilder,
        Cursor::ShapeBuilderErase,
    ];

    /// The straight RGBA of pixel (x, y).
    fn pixel(img: &CustomCursorImage, x: usize, y: usize) -> [u8; 4] {
        let i = (y * img.size[0] as usize + x) * 4;
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
    }

    #[test]
    fn every_glyph_becomes_a_bitmap_with_its_hotspot_inside() {
        for ppp in [1.0, 1.25, 1.5, 2.0, 3.0] {
            for c in GLYPHS {
                let img = rasterize(c, ppp).unwrap_or_else(|| panic!("{c:?} at {ppp}"));
                let [w, h] = img.size.map(usize::from);
                assert_eq!(img.rgba.len(), w * h * 4, "{c:?}");
                assert!(img.hotspot[0] < img.size[0] && img.hotspot[1] < img.size[1], "{c:?} at {ppp}: {img:?}");
                assert!(w as f32 <= 50.0 * ppp && h as f32 <= 50.0 * ppp, "{c:?} at {ppp}: {img:?}");
                assert!(img.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255), "{c:?}: nothing opaque");
            }
            // #1269: the normal Eyedropper is a pipette, with its tip at the hotspot; only
            // Use Precise Cursors turns it into a crosshair.
            let dropper = rasterize(Cursor::Eyedropper, ppp).unwrap();
            let precise = rasterize(Cursor::Eyedropper.precise(), ppp).unwrap();
            assert_ne!(dropper, precise, "Eyedropper must be distinguishable from its precise cursor at {ppp}");
            let [hx, hy] = dropper.hotspot.map(usize::from);
            let halo = (3.0 * ppp).ceil(); // The bitmap bounds round out to whole physical pixels.
            assert!(hx as f32 <= halo && f32::from(dropper.size[1]) - hy as f32 <= halo, "the tip is at the lower-left: {dropper:?}");
            assert!(pixel(&dropper, hx, hy)[3] > 0, "the tip covers the hotspot at {ppp}");
        }
        assert!(rasterize(Cursor::NotAllowed, 1.0).is_none(), "the busy states stay system cursors");
        for ppp in [0.0, -1.0, f32::NAN, f32::INFINITY, 1e9] {
            assert!(rasterize(Cursor::Arrow, ppp).is_none(), "{ppp}");
        }
    }

    #[test]
    fn the_arrow_bitmap_is_black_in_a_white_halo_with_the_tip_on_the_hotspot() {
        let img = rasterize(Cursor::Arrow, 1.0).unwrap();
        let [hx, hy] = img.hotspot.map(usize::from);
        // The tip covers the hotspot's pixel; the body is solid black, the outline's halo white.
        assert!(pixel(&img, hx, hy)[3] > 0, "{:?}", pixel(&img, hx, hy));
        let body = pixel(&img, hx + 2, hy + 8);
        assert!(body[3] == 255 && body[..3].iter().all(|&c| c < 40), "{body:?}");
        assert!(img.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255 && p[..3].iter().all(|&c| c > 215)), "no white halo");
        // Nothing left of or above the tip but the halo.
        assert!((0..img.size[1] as usize).all(|y| pixel(&img, 0, y)[..3].iter().all(|&c| c > 100) || pixel(&img, 0, y)[3] < 128));
        // At twice the pixels per point, twice the pixels.
        let big = rasterize(Cursor::Arrow, 2.0).unwrap();
        for i in 0..2 {
            let ratio = f32::from(big.size[i]) / f32::from(img.size[i]);
            assert!((1.8..=2.2).contains(&ratio), "{:?} vs {:?}", big.size, img.size);
        }
        let [bx, by] = big.hotspot.map(usize::from);
        let body = pixel(&big, bx + 4, by + 16);
        assert!(body[3] == 255 && body[..3].iter().all(|&c| c < 40), "{body:?}");
    }

    /// The Pen drawing a path is the same nib as the new-path Pen, without the asterisk.
    #[test]
    fn the_drawing_pen_has_no_new_path_asterisk() {
        for ppp in [1.0, 2.0] {
            let (new, drawing) = (rasterize(Cursor::Pen, ppp).unwrap(), rasterize(Cursor::PenDrawing, ppp).unwrap());
            assert_eq!(new.hotspot, drawing.hotspot, "at {ppp}");
            // The asterisk's center, 19 × 17 points from the hotspot.
            let at = |img: &CustomCursorImage| {
                let [hx, hy] = img.hotspot.map(usize::from);
                let (x, y) = (hx + (19.0 * ppp) as usize, hy + (17.0 * ppp) as usize);
                if x < img.size[0] as usize && y < img.size[1] as usize { pixel(img, x, y)[3] } else { 0 }
            };
            assert_eq!(at(&new), 255, "the new-path Pen has its asterisk at {ppp}");
            assert_eq!(at(&drawing), 0, "the drawing Pen has no badge at {ppp}");
            // The nib is the same.
            let [hx, hy] = drawing.hotspot.map(usize::from);
            let nib = (hx + (8.0 * ppp) as usize, hy + (8.0 * ppp) as usize);
            assert_eq!(pixel(&new, nib.0, nib.1), pixel(&drawing, nib.0, nib.1), "at {ppp}");
        }
    }

    #[test]
    fn each_bitmap_is_made_once_so_the_os_cursor_is_kept() {
        let mut images = Images::default();
        let a = images.get(Cursor::Pen, 1.5).unwrap();
        let b = images.get(Cursor::Pen, 1.5).unwrap();
        assert!(Arc::ptr_eq(&a.rgba, &b.rgba));
        assert!(!Arc::ptr_eq(&a.rgba, &images.get(Cursor::Pen, 2.0).unwrap().rgba));
        assert!(images.get(Cursor::NotAllowed, 1.5).is_none());
        assert!(images.get(Cursor::Arrow, f32::NAN).is_none());
        assert_eq!(images.0.len(), 2);
    }

    /// One headless 800×600 frame of the whole window, a tenth of a second after the last.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        let time = Some(ctx.input(|i| i.time) + 0.1);
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time, events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        out
    }

    /// One [`frame`] with `focused` set: true for a window that has the focus, false while another app
    /// or a sheet has it.
    fn focus_frame(app: &mut VectorcraftApp, ctx: &egui::Context, focused: bool, events: Vec<egui::Event>) -> egui::FullOutput {
        let time = Some(ctx.input(|i| i.time) + 0.1);
        let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let raw = egui::RawInput { screen_rect, time, focused, events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        out
    }

    #[test]
    fn the_canvas_hands_the_tool_cursor_to_the_os_and_idles_while_the_pointer_rests() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let canvas = app.canvas_rect.unwrap();
        let over = canvas.center();
        let out = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over)]);
        if OS_CURSORS {
            let img = out.platform_output.cursor_image.as_ref().expect("the Selection tool's arrow as an OS cursor");
            assert_eq!(img, &app.canvas.cursors.get(Cursor::Arrow, 1.0).unwrap());
            // Where the OS can't show the bitmap, its system cursor.
            assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::Default);
        } else {
            assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::None);
        }
        // The same bitmap while the pointer moves (the OS keeps its cursor), and an idle window
        // once it rests: no frame is asked for.
        let next = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over + vec2(5.0, 3.0))]);
        if OS_CURSORS {
            assert!(Arc::ptr_eq(&next.platform_output.cursor_image.unwrap().rgba, &out.platform_output.cursor_image.unwrap().rgba));
        }
        let mut idle = frame(&mut app, &ctx, vec![]);
        for _ in 0..3 {
            idle = frame(&mut app, &ctx, vec![]);
        }
        let delay = idle.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay).unwrap();
        assert!(delay > std::time::Duration::from_millis(100), "{delay:?} {:?}", ctx.repaint_causes());
        // Off the canvas, the panels' cursors.
        let off = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(canvas.left() - 20.0, canvas.center().y))]);
        assert!(off.platform_output.cursor_image.is_none());
    }

    /// On Wayland the glyph is painted and the pointer hidden over the canvas, on every build (#1645).
    #[test]
    fn a_wayland_window_paints_the_tool_cursor() {
        let mut app = VectorcraftApp::new(Session::new(), crate::Services { wayland: true, ..Default::default() });
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let over = app.canvas_rect.unwrap().center();
        let out = frame(&mut app, &ctx, vec![egui::Event::PointerMoved(over)]);
        assert!(out.platform_output.cursor_image.is_none());
        assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::None);
    }

    /// On macOS a window without the focus shows the system pointer over the canvas, and the glyph
    /// is not painted next to it (#2020). With the focus back, the canvas sets `CursorIcon::None` again
    /// on the next frame, without a pointer move.
    #[test]
    fn a_window_without_the_focus_shows_the_system_cursor_alone_on_macos() {
        // Wayland: every desktop build paints the glyphs.
        let mut app = VectorcraftApp::new(Session::new(), crate::Services { wayland: true, ..Default::default() });
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let over = app.canvas_rect.unwrap().center();
        let key = focus_frame(&mut app, &ctx, true, vec![egui::Event::PointerMoved(over)]);
        assert_eq!(key.platform_output.cursor_icon, egui::CursorIcon::None, "with the focus: the glyph alone");
        let away = focus_frame(&mut app, &ctx, false, vec![egui::Event::WindowFocused(false), egui::Event::PointerMoved(over + vec2(5.0, 3.0))]);
        if HIDES_ONLY_WHEN_KEY {
            assert_eq!(away.platform_output.cursor_icon, egui::CursorIcon::Crosshair, "macOS without the focus: the system pointer alone");
        } else {
            assert_eq!(away.platform_output.cursor_icon, egui::CursorIcon::None, "other platforms: the glyph without the focus too");
        }
        let back = focus_frame(&mut app, &ctx, true, vec![egui::Event::WindowFocused(true)]);
        assert_eq!(back.platform_output.cursor_icon, egui::CursorIcon::None, "the focus back: the glyph alone");
    }

    /// User Interface › Scale Cursor Proportional to UI (#394): off, the tool cursors keep their
    /// size whatever the UI Scaling; on, they grow with it.
    #[test]
    fn tool_cursors_scale_with_the_ui_only_when_asked() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("prefs.set", json!({"key": "uiScaling", "value": 2.0})).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        assert_eq!(ctx.zoom_factor(), 2.0);
        let over = app.canvas_rect.unwrap().center();
        let shown =
            |app: &mut VectorcraftApp, d: f32| frame(app, &ctx, vec![egui::Event::PointerMoved(over + vec2(d, 0.0))]).platform_output.cursor_image;
        if OS_CURSORS {
            assert_eq!(shown(&mut app, 0.0).unwrap(), app.canvas.cursors.get(Cursor::Arrow, 1.0).unwrap(), "off: its own size");
            app.run("prefs.set", json!({"key": "scaleCursorWithUi", "value": true})).unwrap();
            assert_eq!(shown(&mut app, 4.0).unwrap(), app.canvas.cursors.get(Cursor::Arrow, 2.0).unwrap(), "on: twice as big");
        }
        // Painted into the window (macOS, the web), the glyph is drawn at the scale asked for, its
        // hotspot where the pointer is.
        let painted = |scale: f32| {
            let ctx = egui::Context::default();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("probe"));
                assert!(paint(&ui.ctx().layer_painter(layer), Cursor::Arrow, pos2(100.0, 100.0), scale));
            });
            out.textures_delta.clear();
            out.textures_delta.clear();
            out.shapes.iter().fold(egui::Rect::NOTHING, |r, c| r.union(c.shape.visual_bounding_rect()))
        };
        let (full, half) = (painted(1.0), painted(0.5));
        assert!((half.height() * 2.0 - full.height()).abs() < 1.5 && full.height() > 15.0, "{full:?} {half:?}");
        assert!(full.contains(pos2(100.0, 100.0)) && half.contains(pos2(100.0, 100.0)), "{full:?} {half:?}");
    }

    #[test]
    fn wayland_windows_paint_the_glyphs() {
        assert!(os_cursors(true, false), "Windows, X11");
        assert!(!os_cursors(true, true), "Wayland: the compositor would size the bitmap (#1645)");
        assert!(!os_cursors(false, false) && !os_cursors(false, true), "macOS, the web");
    }

    #[test]
    fn macos_windows_paint_the_glyphs_only_with_the_focus() {
        assert!(paint_glyphs(true, true), "macOS with the focus");
        assert!(!paint_glyphs(true, false), "macOS without the focus: the system pointer shows (#2020)");
        assert!(paint_glyphs(false, true) && paint_glyphs(false, false), "Wayland, the web: painted with the focus or without");
    }

    #[test]
    fn arrow_hotspot_is_tip() {
        let pts = arrow_points(pos2(10.0, 20.0));
        assert_eq!(pts[0], pos2(10.0, 20.0));
        assert!(pts.iter().all(|q| q.x >= 10.0 && q.y >= 20.0));
    }
}
