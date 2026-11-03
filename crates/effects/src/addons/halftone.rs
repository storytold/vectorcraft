//! Vector Halftone (Color Effects): the art as vector dots, lines or shapes sized by its tone, as a
//! live effect (Effect › Color Effects › Vector Halftone…, [`HALFTONE`]) and for Object › Vector
//! Halftone, which builds its paths with the same screens ([`Halftone::layers`]).
//!
//! The art is sampled from pixels ([`Samples`], drawn by the renderer) once per halftone cell on a
//! grid turned by the screen angle; each cell gets a shape whose area follows the tone there. Mono
//! screens follow lightness in one colour (or, with `fromArt`, each dot takes the art's colour under
//! it); CMYK screens follow the art's ink amounts (as CMYK output separates it), one screen per ink at
//! the classic angles, multiplied over each other. The dots can be clipped to the art's outline as
//! real geometry.
//!
//! The renderer, the exporters ([`crate::bake_document`]) and Expand Appearance draw an object with
//! the effect as [`halftone_art`]: the renderer samples the art, so it registers how ([`set_art`]).

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};
use vectorcraft_doc::color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, shapes};
use vectorcraft_pathops::{self as po, BoolOp};

use crate::catalog::{EffectInfo, Lengths};
use crate::util::{flag, num, text};

/// Vector Halftone, the live effect.
pub const HALFTONE: &str = "addon.color.halftone";

/// Most halftone cells per screen.
pub const MAX_CELLS: f64 = 100_000.0;
/// Most pixels the tone is sampled from.
pub const MAX_SAMPLE_PIXELS: f64 = 16.0e6;
/// Cells lighter than this get no dot.
const MIN_TONE: f64 = 0.01;
/// Levels per channel the art's colours are rounded to with `fromArt` (one path per colour).
const COLOR_LEVELS: f64 = 15.0;

/// The effect's catalogue entry.
pub(crate) fn catalog() -> Vec<EffectInfo> {
    vec![EffectInfo {
        id: HALFTONE,
        label: "Vector Halftone…",
        menu: &["Effect", "Color Effects"],
        params: "{shape: \"circle\"|\"ellipse\"|\"square\"|\"diamond\"|\"line\" (circle), frequency: lines per inch 1..300 (20), angle: deg (45; CMYK: the black screen's, cyan −30°, magenta +30°, yellow −45° from it), mode: \"mono\"|\"cmyk\" (mono), color: mono dot colour (\"#000000\"), fromArt: bool (false; mono: each dot takes the art's colour under it), invert: bool (false; dots for light tones), clip: bool (true; the dots clipped to the art's outline)} draws the object as vector halftone dots sized by its tone, as object.vectorHalftone makes them (CMYK screens multiply); the object is sampled as drawn with its other effects; at most 100000 cells per screen, past that the object draws as it is",
        defaults: json!({"shape": "circle", "frequency": 20.0, "angle": 45.0, "mode": "mono", "color": "#000000", "fromArt": false, "invert": false, "clip": true}),
        raster: false,
        lengths: Lengths::default(),
    }]
}

/// Does `n` carry a visible Vector Halftone effect?
pub fn has_halftone(n: &Node) -> bool {
    n.appearance.effects.iter().any(|e| e.visible && e.id == HALFTONE)
}

/// The shape of a halftone dot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DotShape {
    Circle,
    Ellipse,
    Square,
    Diamond,
    Line,
}

impl DotShape {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "circle" | "round" | "dot" => DotShape::Circle,
            "ellipse" => DotShape::Ellipse,
            "square" => DotShape::Square,
            "diamond" => DotShape::Diamond,
            "line" | "lines" => DotShape::Line,
            _ => return None,
        })
    }

    /// The shape for tone `t` (0..1) in a cell of side `s` centred on the origin, the screen
    /// running along x.
    fn path(self, t: f64, s: f64) -> PathData {
        let square = |side: f64| shapes::rectangle(Rect::from_center_size(Point::ZERO, (side, side)));
        match self {
            DotShape::Circle => {
                let r = s * (t / std::f64::consts::PI).sqrt();
                shapes::ellipse(Rect::from_center_size(Point::ZERO, (2.0 * r, 2.0 * r)))
            }
            DotShape::Ellipse => {
                let r = s * (t / std::f64::consts::PI).sqrt();
                shapes::ellipse(Rect::from_center_size(Point::ZERO, (2.6 * r, 2.0 * r / 1.3)))
            }
            DotShape::Square => square(s * t.sqrt()),
            DotShape::Diamond => square(s * t.sqrt()).transformed(Affine::rotate(std::f64::consts::FRAC_PI_4)),
            // Cells along a row join into a line as thick as the tone.
            DotShape::Line => shapes::rectangle(Rect::from_center_size(Point::ZERO, (s * 1.001, s * t))),
        }
    }
}

/// One screen: its ink and angle, and how to read its tone from a sampled pixel.
struct Screen {
    paint: Color,
    angle: f64,
    /// Index into the sampled pixel (unused for mono, CMYK inks for cmyk).
    channel: usize,
}

/// The sampled art: `w`×`h` pixels of 4 bytes over `region` at `scale` px/pt.
pub struct Samples {
    px: Vec<u8>,
    w: usize,
    h: usize,
    region: Rect,
    scale: f64,
    /// Mono: tones are 1 − lightness of RGBA pixels drawn on white; CMYK: ink amounts.
    mono: bool,
    invert: bool,
}

impl Samples {
    /// Pixels `px` (`w`×`h`, 4 bytes each) over `region` at `scale` px/pt; `None` when there are
    /// fewer bytes than that.
    pub fn new(px: Vec<u8>, (w, h): (usize, usize), region: Rect, scale: f64, mono: bool, invert: bool) -> Option<Self> {
        (px.len() >= w.checked_mul(h)?.checked_mul(4)?).then_some(Self { px, w, h, region, scale, mono, invert })
    }

    /// The index of the pixel at document point `p`, `None` outside the samples.
    fn index(&self, p: Point) -> Option<usize> {
        let x = ((p.x - self.region.x0) * self.scale).floor();
        let y = ((p.y - self.region.y0) * self.scale).floor();
        (x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h).then(|| (y as usize * self.w + x as usize) * 4)
    }

    fn byte(&self, i: usize, k: usize) -> f64 {
        self.px.get(i + k).copied().unwrap_or(0) as f64 / 255.0
    }

    /// The tone (0..1) of `channel` at document point `p`.
    fn tone(&self, p: Point, channel: usize) -> f64 {
        let Some(i) = self.index(p) else { return if self.invert { 1.0 } else { 0.0 } };
        let t =
            if self.mono { 1.0 - (0.2126 * self.byte(i, 0) + 0.7152 * self.byte(i, 1) + 0.0722 * self.byte(i, 2)) } else { self.byte(i, channel) };
        if self.invert { 1.0 - t } else { t }
    }

    /// The 3 × 3 sample points of a cell of side `s` at `c` turned by `rot`.
    fn cell_points(c: Point, s: f64, rot: Affine) -> impl Iterator<Item = Point> {
        (0..9).map(move |k| c + (rot * Point::new((f64::from(k / 3) - 1.0) * s / 3.0, (f64::from(k % 3) - 1.0) * s / 3.0)).to_vec2())
    }

    /// The mean tone over a cell of side `s` at `c` turned by `rot` (3 × 3 samples).
    fn cell_tone(&self, c: Point, s: f64, rot: Affine, channel: usize) -> f64 {
        Self::cell_points(c, s, rot).map(|p| self.tone(p, channel)).sum::<f64>() / 9.0
    }

    /// The mean colour over a cell (as [`Self::cell_tone`]; white outside the samples), each channel
    /// rounded to [`COLOR_LEVELS`] steps.
    fn cell_color(&self, c: Point, s: f64, rot: Affine) -> [u8; 3] {
        let mut sum = [0.0; 3];
        for p in Self::cell_points(c, s, rot) {
            for (k, v) in sum.iter_mut().enumerate() {
                *v += self.index(p).map_or(1.0, |i| self.byte(i, k));
            }
        }
        sum.map(|v| (v / 9.0 * COLOR_LEVELS).round().clamp(0.0, COLOR_LEVELS) as u8)
    }
}

/// A halftone's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Halftone {
    pub shape: DotShape,
    /// Lines per inch.
    pub frequency: f64,
    /// Degrees.
    pub angle: f64,
    /// One screen in `color` (else four CMYK screens).
    pub mono: bool,
    pub color: Color,
    /// Mono: each dot in the art's colour under it instead of `color`.
    pub from_art: bool,
    /// Dots for light tones.
    pub invert: bool,
    /// The dots clipped to the art's outline.
    pub clip: bool,
}

impl Halftone {
    /// The settings of `n`'s last visible Vector Halftone effect, its values brought into range.
    pub fn of(n: &Node) -> Option<Self> {
        let e = n.appearance.effects.iter().rev().find(|e| e.visible && e.id == HALFTONE)?;
        Some(Self::from_params(&e.params))
    }

    fn from_params(p: &Value) -> Self {
        Self {
            shape: DotShape::parse(text(p, "shape", "circle")).unwrap_or(DotShape::Circle),
            frequency: num(p, "frequency", 20.0).clamp(1.0, 300.0),
            angle: num(p, "angle", 45.0),
            mono: !text(p, "mode", "mono").eq_ignore_ascii_case("cmyk"),
            color: crate::raster::color_param(p, Color::BLACK),
            from_art: flag(p, "fromArt", false),
            invert: flag(p, "invert", false),
            clip: flag(p, "clip", true),
        }
    }

    /// The side of a cell in points.
    pub fn cell(&self) -> f64 {
        72.0 / self.frequency
    }

    /// Where the art over `bounds` is sampled and at how many pixels per point: about four pixels per
    /// cell, at most [`MAX_SAMPLE_PIXELS`]. `None` when `bounds` have no area or the grid turned by
    /// the screen angle would have more than [`MAX_CELLS`] cells.
    pub fn plan(&self, bounds: Rect) -> Option<(Rect, f64)> {
        let cell = self.cell();
        if !(bounds.width() > 0.0 && bounds.height() > 0.0 && bounds.area().is_finite()) {
            return None;
        }
        // The grid turned by the screen angle covers at most (w + h)² / 2.
        if (bounds.width() + bounds.height()).powi(2) / 2.0 / (cell * cell) > MAX_CELLS {
            return None;
        }
        let scale = (4.0 / cell).min((MAX_SAMPLE_PIXELS / bounds.area()).sqrt()).clamp(0.01, 16.0);
        Some((bounds.inflate(cell, cell), scale))
    }

    /// The screens, bottom to top.
    fn screens(&self) -> Vec<Screen> {
        let angle = self.angle;
        if self.mono {
            return vec![Screen { paint: self.color, angle, channel: 0 }];
        }
        // Bottom to top: yellow, magenta, cyan, black.
        vec![
            Screen { paint: Color::cmyk(0.0, 0.0, 1.0, 0.0), angle: angle - 45.0, channel: 2 },
            Screen { paint: Color::cmyk(0.0, 1.0, 0.0, 0.0), angle: angle + 30.0, channel: 1 },
            Screen { paint: Color::cmyk(1.0, 0.0, 0.0, 0.0), angle: angle - 30.0, channel: 0 },
            Screen { paint: Color::cmyk(0.0, 0.0, 0.0, 1.0), angle, channel: 3 },
        ]
    }

    /// The dots of the art `samples` show over `bounds`, one path per ink (per colour with
    /// `fromArt`) in paint order, each clipped to `outline` when given, and how many dots there are.
    pub fn layers(&self, samples: &Samples, bounds: Rect, outline: Option<&(BezPath, FillRule)>) -> (Vec<(PathData, Color)>, usize) {
        let cell = self.cell();
        let mut layers = vec![];
        let mut count = 0;
        for sc in self.screens() {
            let dots = screen_dots(samples, bounds, cell, &sc, self.shape);
            count += dots.len();
            if self.mono && self.from_art {
                let rot = Affine::rotate(sc.angle.to_radians());
                let mut by_color: BTreeMap<[u8; 3], Vec<PathData>> = BTreeMap::new();
                for (dot, c) in dots {
                    by_color.entry(samples.cell_color(c, cell, rot)).or_default().push(dot);
                }
                for (rgb, dots) in by_color {
                    let [r, g, b] = rgb.map(|v| (f64::from(v) / COLOR_LEVELS) as f32);
                    layers.push((merged(dots, outline), Color::rgb(r, g, b)));
                }
            } else {
                layers.push((merged(dots.into_iter().map(|d| d.0).collect(), outline), sc.paint));
            }
        }
        layers.retain(|(path, _)| !path.is_empty());
        (layers, count)
    }
}

/// The dots of one screen over `bounds`, each with its cell's centre.
fn screen_dots(samples: &Samples, bounds: Rect, cell: f64, sc: &Screen, shape: DotShape) -> Vec<(PathData, Point)> {
    let rot = Affine::rotate(sc.angle.to_radians());
    let inv = rot.inverse();
    // The grid covers the bounds in screen space.
    let corners =
        [bounds.origin(), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)].map(|q| inv * q);
    let (mut lo, mut hi) = (corners[0], corners[0]);
    for q in &corners[1..] {
        lo = Point::new(lo.x.min(q.x), lo.y.min(q.y));
        hi = Point::new(hi.x.max(q.x), hi.y.max(q.y));
    }
    let (i0, i1) = ((lo.x / cell).floor() as i64 - 1, (hi.x / cell).ceil() as i64 + 1);
    let (j0, j1) = ((lo.y / cell).floor() as i64 - 1, (hi.y / cell).ceil() as i64 + 1);
    let near = bounds.inflate(cell, cell);
    let mut dots = vec![];
    for j in j0..=j1 {
        for i in i0..=i1 {
            let c = rot * Point::new((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell);
            if !near.contains(c) {
                continue;
            }
            let t = samples.cell_tone(c, cell, rot, sc.channel).clamp(0.0, 1.0);
            if t < MIN_TONE {
                continue;
            }
            let place = Affine::translate(c.to_vec2()) * rot;
            dots.push((shape.path(t, cell).transformed(place), c));
        }
    }
    dots
}

/// The outline Clip to Art clips the dots of `nodes` to: their clip outline taken together.
pub fn art_outline(nodes: &[Node]) -> Option<(BezPath, FillRule)> {
    crate::clip_outline(&Node::group(NodeId(u64::MAX), nodes.iter().cloned().map(Arc::new).collect()))
}

/// `dots` as one path, clipped to `outline` when given (dots wholly inside it need no clipping).
fn merged(dots: Vec<PathData>, outline: Option<&(BezPath, FillRule)>) -> PathData {
    let (mut inside, mut edge) = (vec![], vec![]);
    for dot in dots {
        let whole = match (outline, dot.control_bounds()) {
            (Some((bp, rule)), Some(db)) => [db.origin(), Point::new(db.x1, db.y0), Point::new(db.x1, db.y1), Point::new(db.x0, db.y1)]
                .iter()
                .all(|q| fill_contains(bp, *rule, *q)),
            _ => true,
        };
        if whole { inside.push(dot) } else { edge.push(dot) }
    }
    let mut subs: Vec<_> = inside.into_iter().flat_map(|p| p.subpaths).collect();
    if let Some((bp, rule)) = outline
        && !edge.is_empty()
    {
        let edge = PathData::new(edge.into_iter().flat_map(|p| p.subpaths).collect());
        let clipped = po::boolean(&edge, FillRule::NonZero, &PathData::from_bezpath(bp), *rule, BoolOp::Intersect);
        subs.extend(clipped.subpaths);
    } else {
        subs.extend(edge.into_iter().flat_map(|p| p.subpaths));
    }
    PathData::new(subs)
}

/// The paths of `layers` ([`Halftone::layers`]), each filled with its ink and ids from `id`; CMYK
/// screens (`mono` false) multiply over each other.
pub fn dot_nodes(layers: Vec<(PathData, Color)>, mono: bool, id: &mut dyn FnMut() -> NodeId) -> Vec<Arc<Node>> {
    layers
        .into_iter()
        .map(|(path, ink)| {
            let mut n = Node::path(id(), path, Appearance::basic(Paint::solid(ink), Paint::None, 0.0));
            if !mono {
                n.blend = BlendMode::Multiply;
            }
            Arc::new(n)
        })
        .collect()
}

/// `n` split for its halftone: the object the effect samples (`n` visible, without its Vector
/// Halftone effects, transparency, mask and link) and an empty group that takes over its identity,
/// transparency, mask and link, for the dots.
pub fn split(n: &Node) -> (Node, Node) {
    let mut object = n.clone();
    object.appearance.effects.retain(|e| e.id != HALFTONE);
    object.visible = true;
    let mut g = Node::group(n.id, vec![]);
    (g.visible, g.locked) = (n.visible, n.locked);
    g.name = object.name.take();
    g.opacity = std::mem::replace(&mut object.opacity, 1.0);
    g.blend = std::mem::take(&mut object.blend);
    g.isolate = std::mem::take(&mut object.isolate);
    g.extras_mut().mask = object.extras_mut().mask.take();
    g.extras_mut().attrs = object.extras_mut().attrs.take();
    (object, g)
}

/// How far the dots of `n`'s halftone reach past the bounds of the object it samples: nothing when
/// they are clipped to its outline, else up to two cells (a dot's centre lies within a cell of the
/// bounds, and the dot within another).
pub fn reach(n: &Node) -> f64 {
    Halftone::of(n).filter(|h| !h.clip).map_or(0.0, |h| 2.0 * h.cell())
}

/// Makes the art of an object with a visible Vector Halftone effect ([`halftone_art`]).
pub type ArtHook = fn(&Document, &Node) -> Option<Node>;

static ART: OnceLock<ArtHook> = OnceLock::new();

/// Register how the art of an object with Vector Halftone is made (the renderer samples it, above
/// this crate; the first registration stays).
pub fn set_art(f: ArtHook) {
    // Already set: the same renderer, registered by another session.
    let _ = ART.set(f);
}

/// The art of `n` with a visible Vector Halftone effect in `d`: the group [`split`] makes, holding
/// the dot paths ([`dot_nodes`], ids `n`'s). `None` without the effect, while nothing is registered
/// ([`set_art`]), and when the art has no area or would take too many cells ([`Halftone::plan`]).
pub fn halftone_art(d: &Document, n: &Node) -> Option<Node> {
    if !has_halftone(n) {
        return None;
    }
    ART.get()?(d, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::Effect;

    /// A black square's samples: `side` points of black from (10, 10), white around it.
    fn black_square(side: f64, h: &Halftone) -> (Samples, Rect) {
        let b = Rect::new(10.0, 10.0, 10.0 + side, 10.0 + side);
        let (region, scale) = h.plan(b).unwrap();
        let (w, hh) = ((region.width() * scale).round() as usize, (region.height() * scale).round() as usize);
        let mut px = vec![255u8; w * hh * 4];
        for y in 0..hh {
            for x in 0..w {
                let p = Point::new(region.x0 + (x as f64 + 0.5) / scale, region.y0 + (y as f64 + 0.5) / scale);
                if b.contains(p) {
                    px[(y * w + x) * 4..][..3].copy_from_slice(&[200, 0, 0]);
                }
            }
        }
        (Samples::new(px, (w, hh), region, scale, true, h.invert).unwrap(), b)
    }

    fn effect(params: Value) -> Node {
        let mut n = Node::path(NodeId(7), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default());
        n.appearance.effects.push(Effect { id: HALFTONE.into(), params, visible: true });
        n
    }

    #[test]
    fn the_effect_is_in_color_effects_with_its_defaults() {
        let info = crate::effect_info(HALFTONE).unwrap();
        assert_eq!(info.menu, ["Effect", "Color Effects"]);
        assert!(!crate::is_geometry(HALFTONE) && !crate::is_raster(HALFTONE) && !crate::is_adjustment(HALFTONE));
        let h = Halftone::of(&effect(info.defaults)).unwrap();
        assert_eq!((h.shape, h.frequency, h.angle, h.mono, h.from_art, h.invert, h.clip), (DotShape::Circle, 20.0, 45.0, true, false, false, true));
        assert_eq!(h.color, Color::BLACK);
    }

    #[test]
    fn values_out_of_range_are_brought_in() {
        let h = Halftone::of(&effect(json!({"frequency": 1e308, "angle": "NaN", "shape": 7, "mode": "CMYK", "color": "nope"}))).unwrap();
        assert_eq!((h.frequency, h.angle, h.shape, h.mono, h.color), (300.0, 45.0, DotShape::Circle, false, Color::BLACK));
        assert_eq!(Halftone::of(&effect(json!({"frequency": -5}))).unwrap().frequency, 1.0);
        let mut hidden = effect(json!({}));
        hidden.appearance.effects[0].visible = false;
        assert!(Halftone::of(&hidden).is_none() && !has_halftone(&hidden));
    }

    #[test]
    fn dots_follow_the_tone_and_the_settings() {
        let h = Halftone::of(&effect(json!({"frequency": 18}))).unwrap();
        let (s, b) = black_square(60.0, &h);
        let (layers, dots) = h.layers(&s, b, None);
        assert_eq!(layers.len(), 1);
        assert!((150..=320).contains(&dots), "{dots}");
        // Inverted, the dark square gets small dots and the white around it full ones.
        let inv = Halftone { invert: true, ..h.clone() };
        let (s2, _) = black_square(60.0, &inv);
        assert!(inv.layers(&s2, b, None).1 > dots);
        // The colour of the art under each dot.
        let art = Halftone { from_art: true, ..h.clone() };
        let (layers, _) = art.layers(&s, b, None);
        assert!(layers.iter().any(|(_, c)| c.to_hex() == "#cc0000"), "{:?}", layers.iter().map(|l| l.1.to_hex()).collect::<Vec<_>>());
        // CMYK screens: one layer per ink with dots (red: magenta and yellow).
        let cmyk = Halftone { mono: false, ..h };
        let one = Samples::new([0u8, 255, 255, 0].repeat(400), (20, 20), Rect::new(0.0, 0.0, 20.0, 20.0), 1.0, false, false).unwrap();
        assert_eq!(cmyk.layers(&one, Rect::new(2.0, 2.0, 18.0, 18.0), None).0.len(), 2);
    }

    #[test]
    fn the_caps_refuse_too_many_cells() {
        let h = Halftone::of(&effect(json!({"frequency": 300}))).unwrap();
        assert!(h.plan(Rect::new(0.0, 0.0, 10_000.0, 10_000.0)).is_none());
        assert!(h.plan(Rect::new(0.0, 0.0, f64::INFINITY, 10.0)).is_none());
        assert!(h.plan(Rect::new(0.0, 0.0, 0.0, 10.0)).is_none());
        assert!(h.plan(Rect::new(0.0, 0.0, 40.0, 40.0)).is_some());
        assert!(Samples::new(vec![0; 3], (1, 1), Rect::new(0.0, 0.0, 1.0, 1.0), 1.0, true, false).is_none());
        assert!(Samples::new(vec![], (usize::MAX, 2), Rect::new(0.0, 0.0, 1.0, 1.0), 1.0, true, false).is_none());
    }

    #[test]
    fn the_group_takes_the_objects_identity() {
        let mut n = effect(json!({"clip": false, "frequency": 36}));
        n.opacity = 0.5;
        n.name = Some("art".into());
        let (object, g) = split(&n);
        assert!(!has_halftone(&object));
        assert_eq!((object.opacity, g.opacity, g.id, g.name.as_deref()), (1.0, 0.5, n.id, Some("art")));
        assert_eq!(reach(&n), 4.0);
        assert_eq!(reach(&effect(json!({}))), 0.0);
    }
}
