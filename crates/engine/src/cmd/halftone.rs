//! Object → Vector Halftone: the selected art (vectors, type, images) as vector dots, lines or
//! shapes sized by its tone.
//!
//! The art is rendered offscreen and sampled once per halftone cell on a grid turned by the screen
//! angle; each cell gets a shape whose area follows the tone there. Mono screens follow lightness
//! in one colour; CMYK screens follow the art's ink amounts (as CMYK output separates it), one
//! screen per ink at the classic angles, multiplied over each other. The dots can be clipped to the
//! art's outline as real geometry.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, shapes};
use vectorcraft_pathops as po;
use vectorcraft_pathops::BoolOp;

use super::edit::selected_roots;
use super::menucmds::isolated_doc;
use super::{CommandSpec, EngineError, Result, Session, bad, bool_or, cmd, color_value, f64_or, has_selection, str_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "object.vectorHalftone",
        "Vector Halftone…",
        ["Object"],
        None,
        "{shape?: \"circle\"|\"ellipse\"|\"square\"|\"diamond\"|\"line\" (circle), frequency?: lines per inch 1–300 (20), angle?: deg (45; CMYK: the black screen's, cyan −30°, magenta +30°, yellow −45° from it), mode?: \"mono\"|\"cmyk\" (mono), color?: mono dot colour (\"#000000\"), invert?: bool (dots for light tones), clip?: true (clip the dots to the art's outline), keepOriginal?: false} the selection as vector halftone dots sized by its tone, a group of one path per screen (CMYK screens multiply), as one undo step → {id, dots, warning? (an image in the art couldn't be decoded, as object.rasterize says)}",
        has_selection,
        halftone
    )]
}

/// Most halftone cells per screen.
const MAX_CELLS: f64 = 100_000.0;
/// Most pixels the tone is sampled from.
const MAX_SAMPLE_PIXELS: f64 = 16.0e6;
/// Cells lighter than this get no dot.
const MIN_TONE: f64 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq)]
enum DotShape {
    Circle,
    Ellipse,
    Square,
    Diamond,
    Line,
}

impl DotShape {
    fn parse(s: &str) -> Option<Self> {
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
    /// Index into the sampled pixel (RGBA lightness for mono, CMYK inks for cmyk).
    channel: usize,
}

/// The sampled art: `w`×`h` pixels of 4 bytes over `region` at `scale` px/pt.
struct Samples {
    px: Vec<u8>,
    w: usize,
    h: usize,
    region: Rect,
    scale: f64,
    /// Mono: tones are 1 − lightness of RGBA pixels; CMYK: ink amounts.
    mono: bool,
    invert: bool,
}

impl Samples {
    /// The tone (0..1) of `channel` at document point `p`.
    fn tone(&self, p: Point, channel: usize) -> f64 {
        let x = ((p.x - self.region.x0) * self.scale).floor();
        let y = ((p.y - self.region.y0) * self.scale).floor();
        if !(x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h) {
            return if self.invert { 1.0 } else { 0.0 };
        }
        let i = (y as usize * self.w + x as usize) * 4;
        let byte = |k: usize| self.px.get(i + k).copied().unwrap_or(0) as f64 / 255.0;
        let t = if self.mono { 1.0 - (0.2126 * byte(0) + 0.7152 * byte(1) + 0.0722 * byte(2)) } else { byte(channel) };
        if self.invert { 1.0 - t } else { t }
    }

    /// The mean tone over a cell of side `s` at `c` turned by `rot` (3 × 3 samples).
    fn cell_tone(&self, c: Point, s: f64, rot: Affine, channel: usize) -> f64 {
        let mut sum = 0.0;
        for i in 0..3 {
            for j in 0..3 {
                let off = Point::new((i as f64 - 1.0) * s / 3.0, (j as f64 - 1.0) * s / 3.0);
                sum += self.tone(c + (rot * off).to_vec2(), channel);
            }
        }
        sum / 9.0
    }
}

fn halftone(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.vectorHalftone";
    let shape = match str_param(p, "shape") {
        Some(v) => DotShape::parse(v).ok_or_else(|| bad(C, format!("unknown shape `{v}` (circle|ellipse|square|diamond|line)")))?,
        None => DotShape::Circle,
    };
    let frequency = f64_or(p, "frequency", 20.0);
    if !(1.0..=300.0).contains(&frequency) {
        return Err(bad(C, "frequency must be between 1 and 300 lines per inch"));
    }
    let angle = f64_or(p, "angle", 45.0);
    if !angle.is_finite() {
        return Err(bad(C, "invalid angle"));
    }
    let mono = match str_param(p, "mode").unwrap_or("mono") {
        "mono" => true,
        "cmyk" => false,
        other => return Err(bad(C, format!("unknown mode `{other}` (mono|cmyk)"))),
    };
    let color = match p.get("color") {
        Some(v) => color_value(v).ok_or_else(|| bad(C, "invalid color"))?,
        None => Color::BLACK,
    };
    let (invert, clip, keep) = (bool_or(p, "invert", false), bool_or(p, "clip", true), bool_or(p, "keepOriginal", false));
    let cell = 72.0 / frequency;

    let roots = selected_roots(s)?;
    let st = s.doc()?;
    let b = st.doc.bounds_of(&roots, true).ok_or_else(|| bad(C, "the selection has no bounds"))?;
    if b.width() <= 0.0 || b.height() <= 0.0 || !b.area().is_finite() {
        return Err(bad(C, "the selection has no area"));
    }
    // The grid turned by the screen angle covers at most (w + h)² / 2.
    if (b.width() + b.height()).powi(2) / 2.0 / (cell * cell) > MAX_CELLS {
        return Err(bad(C, "too many dots for this size: lower the frequency"));
    }
    let nodes: Vec<Node> = roots
        .iter()
        .filter_map(|id| st.doc.node(*id).cloned())
        .map(|mut n| {
            n.visible = true;
            n
        })
        .collect();
    let outline = if clip {
        vectorcraft_render::effects::clip_outline(&Node::group(NodeId(u64::MAX), nodes.iter().cloned().map(Arc::new).collect()))
    } else {
        None
    };
    // Sample the art at about four pixels per cell.
    let scale = (4.0 / cell).min((MAX_SAMPLE_PIXELS / b.area()).sqrt()).clamp(0.01, 16.0);
    let region = b.inflate(cell, cell);
    let tmp = isolated_doc(&st.doc, nodes);
    let mut r = vectorcraft_render::Renderer::new();
    let (w, h) = vectorcraft_render::region_pixels(region, scale);
    let (w, h) = (w as usize, h as usize);
    let failed = vectorcraft_render::decode_failures();
    let px = if mono {
        r.render_region(&tmp, region, scale, true).pixels
    } else {
        r.render_region_inks(&tmp, region, scale, &vectorcraft_render::RenderOptions { skip_templates: true, ..Default::default() })
    };
    let warning = super::rasterfx::undecoded_warning(failed);
    if px.len() < w * h * 4 {
        return Err(EngineError::Other("the art could not be sampled".into()));
    }
    let samples = Samples { px, w, h, region, scale, mono, invert };
    let screens: Vec<Screen> = if mono {
        vec![Screen { paint: color, angle, channel: 0 }]
    } else {
        // Bottom to top: yellow, magenta, cyan, black.
        vec![
            Screen { paint: Color::cmyk(0.0, 0.0, 1.0, 0.0), angle: angle - 45.0, channel: 2 },
            Screen { paint: Color::cmyk(0.0, 1.0, 0.0, 0.0), angle: angle + 30.0, channel: 1 },
            Screen { paint: Color::cmyk(1.0, 0.0, 0.0, 0.0), angle: angle - 30.0, channel: 0 },
            Screen { paint: Color::cmyk(0.0, 0.0, 0.0, 1.0), angle, channel: 3 },
        ]
    };
    let mut layers = vec![];
    let mut dots = 0;
    for sc in &screens {
        let (path, n) = screen_dots(&samples, b, cell, sc, shape, outline.as_ref());
        dots += n;
        if !path.is_empty() {
            layers.push((path, sc.paint));
        }
    }
    if layers.is_empty() {
        return Err(EngineError::Other("the art is too light for any dots".into()));
    }
    let top = *roots.last().ok_or_else(|| bad(C, "nothing selected"))?;
    let id = s.edit("Vector Halftone", |d, sel| {
        let children = layers
            .into_iter()
            .map(|(path, ink)| {
                let mut n = Node::path(d.alloc_id(), path, Appearance::basic(Paint::solid(ink), Paint::None, 0.0));
                if !mono {
                    n.blend = BlendMode::Multiply;
                }
                Arc::new(n)
            })
            .collect();
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = Some("Vector Halftone".into());
        if keep {
            let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
            d.insert(par, idx + 1, g)?;
        } else {
            d.wrap(&roots, |_| g)?;
        }
        sel.set([gid]);
        Ok(gid)
    })?;
    let mut out = json!({ "id": id.0, "dots": dots });
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}

/// The dots of one screen over `bounds` as one path, with their count.
fn screen_dots(samples: &Samples, bounds: Rect, cell: f64, sc: &Screen, shape: DotShape, outline: Option<&(BezPath, FillRule)>) -> (PathData, usize) {
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
    let (mut inside, mut edge) = (vec![], vec![]);
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
            let dot = shape.path(t, cell).transformed(place);
            // Dots wholly inside the outline need no clipping.
            let whole = match (outline, dot.control_bounds()) {
                (Some((bp, rule)), Some(db)) => [db.origin(), Point::new(db.x1, db.y0), Point::new(db.x1, db.y1), Point::new(db.x0, db.y1)]
                    .iter()
                    .all(|q| fill_contains(bp, *rule, *q)),
                _ => true,
            };
            if whole { inside.push(dot) } else { edge.push(dot) }
        }
    }
    let count = inside.len() + edge.len();
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
    (PathData::new(subs), count)
}
