//! Liquify tools: Warp (Shift+R), Twirl, Pucker, Bloat, Scallop, Crystallize, Wrinkle.
//!
//! A drag previews `object.liquify {tool, points, width, height, angle, intensity, detail,
//! simplify, rate?, complexity?, horizontal?, vertical?, affect…?, usePressure?, ids?}` with every
//! pointer sample so far (and its pen pressure while Use Pressure Pen is on), so the result is a
//! pure function of the parameters (replay reproduces it exactly). Holding the brush still with
//! Twirl, Pucker or Bloat repeats the last sample every [`HOLD_EVERY`] seconds of [`Tool::tick`]
//! time: one more dab there each time.
//!
//! The kernel resamples the stroke into dabs spaced a fraction of the brush apart ([`Dabber`]:
//! more samples only add dabs, so a growing stroke can be applied as it grows). For each dab it
//! first subdivides the segments the brush touches (Detail: more anchors where the brush passes,
//! lines become curves so the result stays smooth), then moves anchors and handles by the tool's
//! displacement field weighted by a smooth falloff `(1 − r²)²` inside the (elliptical, rotated)
//! brush. Noise for Scallop/Crystallize/Wrinkle is a hash of the dab and anchor indices.

use serde_json::{Value, json};
use vectorcraft_geom::kurbo::ParamCurveArclen;
use vectorcraft_geom::{Anchor, AnchorKind, PathData, Point, Rect, SubPath, Vec2};

use super::ellipse_path;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquifyKind {
    Warp,
    Twirl,
    Pucker,
    Bloat,
    Scallop,
    Crystallize,
    Wrinkle,
}

impl LiquifyKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "warp" => Self::Warp,
            "twirl" => Self::Twirl,
            "pucker" => Self::Pucker,
            "bloat" => Self::Bloat,
            "scallop" => Self::Scallop,
            "crystallize" => Self::Crystallize,
            "wrinkle" => Self::Wrinkle,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Warp => "warp",
            Self::Twirl => "twirl",
            Self::Pucker => "pucker",
            Self::Bloat => "bloat",
            Self::Scallop => "scallop",
            Self::Crystallize => "crystallize",
            Self::Wrinkle => "wrinkle",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Warp => "Warp",
            Self::Twirl => "Twirl",
            Self::Pucker => "Pucker",
            Self::Bloat => "Bloat",
            Self::Scallop => "Scallop",
            Self::Crystallize => "Crystallize",
            Self::Wrinkle => "Wrinkle",
        }
    }
    /// Holding the brush still keeps applying the tool (a repeated stroke sample is one more dab).
    pub fn holds(self) -> bool {
        matches!(self, Self::Twirl | Self::Pucker | Self::Bloat)
    }
    /// Has the Simplify option (Scallop, Crystallize and Wrinkle have Brush Affects instead).
    pub fn simplifies(self) -> bool {
        matches!(self, Self::Warp | Self::Twirl | Self::Pucker | Self::Bloat)
    }
    /// Has the Brush Affects options (anchor points, in and out tangent handles).
    pub fn has_affects(self) -> bool {
        !self.simplifies()
    }
}

/// Brush and tool options (Warp Tool Options and friends).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiquifyParams {
    pub kind: LiquifyKind,
    /// Brush width / height in points and rotation in degrees.
    pub width: f64,
    pub height: f64,
    pub angle: f64,
    /// 0..1.
    pub intensity: f64,
    /// Use Pressure Pen: each stroke sample's pen pressure is the intensity there.
    pub use_pressure: bool,
    /// 1..10: anchor density added where the brush passes.
    pub detail: f64,
    /// 0..100: removal of redundant flat anchors afterwards (Warp, Twirl, Pucker, Bloat).
    pub simplify: f64,
    /// The Simplify checkbox.
    pub simplify_on: bool,
    /// Twirl rate, degrees (−180..180).
    pub rate: f64,
    /// Scallop/Crystallize/Wrinkle complexity (0..15).
    pub complexity: f64,
    /// Wrinkle horizontal / vertical amount (0..1).
    pub horizontal: f64,
    pub vertical: f64,
    pub affect_anchors: bool,
    pub affect_in: bool,
    pub affect_out: bool,
}

impl LiquifyParams {
    pub fn new(kind: LiquifyKind) -> Self {
        Self {
            kind,
            width: 100.0,
            height: 100.0,
            angle: 0.0,
            intensity: 0.5,
            use_pressure: false,
            detail: 2.0,
            simplify: 50.0,
            simplify_on: true,
            rate: 40.0,
            complexity: 1.0,
            horizontal: 0.0,
            vertical: 1.0,
            affect_anchors: true,
            affect_in: true,
            affect_out: true,
        }
    }

    /// Parse command parameters (percentages above 1 are accepted for intensity/horizontal/vertical).
    pub fn from_json(p: &Value) -> Option<Self> {
        let kind = LiquifyKind::parse(p.get("tool")?.as_str()?)?;
        let mut s = Self::new(kind);
        let f = |k: &str| p.get(k).and_then(Value::as_f64);
        let pct = |v: f64| if v > 1.0 { v / 100.0 } else { v };
        if let Some(d) = f("diameter") {
            s.width = d;
            s.height = d;
        }
        s.width = f("width").unwrap_or(s.width).clamp(0.5, 10_000.0);
        s.height = f("height").unwrap_or(s.height).clamp(0.5, 10_000.0);
        s.angle = f("angle").unwrap_or(0.0);
        s.intensity = pct(f("intensity").unwrap_or(0.5)).clamp(0.0, 1.0);
        s.detail = f("detail").unwrap_or(2.0).clamp(1.0, 10.0);
        s.simplify = f("simplify").unwrap_or(50.0).clamp(0.0, 100.0);
        s.rate = f("rate").unwrap_or(40.0).clamp(-180.0, 180.0);
        s.complexity = f("complexity").unwrap_or(1.0).clamp(0.0, 15.0);
        s.horizontal = pct(f("horizontal").unwrap_or(0.0)).clamp(0.0, 1.0);
        s.vertical = pct(f("vertical").unwrap_or(1.0)).clamp(0.0, 1.0);
        let b = |k: &str, d: bool| p.get(k).and_then(Value::as_bool).unwrap_or(d);
        s.affect_anchors = b("affectAnchors", true);
        s.affect_in = b("affectIn", true);
        s.affect_out = b("affectOut", true);
        s.use_pressure = b("usePressure", false);
        s.simplify_on = b("simplifyOn", true);
        Some(s)
    }

    /// The command parameters (those the kind uses).
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "tool": self.kind.id(), "width": self.width, "height": self.height, "angle": self.angle,
            "intensity": self.intensity, "detail": self.detail,
        });
        if self.use_pressure {
            v["usePressure"] = json!(true);
        }
        if self.kind.simplifies() {
            v["simplify"] = json!(self.simplify);
            v["simplifyOn"] = json!(self.simplify_on);
        } else {
            v["complexity"] = json!(self.complexity);
            v["affectAnchors"] = json!(self.affect_anchors);
            v["affectIn"] = json!(self.affect_in);
            v["affectOut"] = json!(self.affect_out);
        }
        match self.kind {
            LiquifyKind::Twirl => v["rate"] = json!(self.rate),
            LiquifyKind::Wrinkle => {
                v["horizontal"] = json!(self.horizontal);
                v["vertical"] = json!(self.vertical);
            }
            _ => {}
        }
        v
    }

    /// Every option, whichever tool these are (the tool's options: [`Self::to_json`] gives only
    /// the parameters its kind uses).
    pub fn options_json(&self) -> Value {
        let mut v = self.to_json();
        for (k, x) in [
            ("simplify", self.simplify),
            ("rate", self.rate),
            ("complexity", self.complexity),
            ("horizontal", self.horizontal),
            ("vertical", self.vertical),
        ] {
            v[k] = json!(x);
        }
        for (k, x) in [
            ("usePressure", self.use_pressure),
            ("simplifyOn", self.simplify_on),
            ("affectAnchors", self.affect_anchors),
            ("affectIn", self.affect_in),
            ("affectOut", self.affect_out),
        ] {
            v[k] = json!(x);
        }
        v
    }

    /// How far off the chord Simplify removes anchors (0: Simplify is off or not an option).
    fn simplify_tolerance(&self) -> f64 {
        if self.kind.simplifies() && self.simplify_on { self.simplify / 100.0 * self.detail_spacing() * 0.02 } else { 0.0 }
    }

    fn radii(&self) -> (f64, f64) {
        (self.width / 2.0, self.height / 2.0)
    }

    /// Spacing between dabs.
    pub fn dab_spacing(&self) -> f64 {
        let (rx, ry) = self.radii();
        (rx.min(ry) * 0.2).max(0.5)
    }

    /// Target segment length inside the brush.
    fn detail_spacing(&self) -> f64 {
        let (rx, ry) = self.radii();
        let extra = match self.kind {
            LiquifyKind::Scallop | LiquifyKind::Crystallize | LiquifyKind::Wrinkle => 1.0 + self.complexity * 0.5,
            _ => 1.0,
        };
        (rx.min(ry) * 2.0 / (self.detail * 2.0 + 1.0) / extra).max(0.5)
    }

    /// Bounding box of the brush at `c`.
    pub fn brush_bounds(&self, c: Point) -> Rect {
        let r = self.width.max(self.height) / 2.0;
        Rect::new(c.x - r, c.y - r, c.x + r, c.y + r)
    }

    /// Falloff weight of `q` for a brush centred at `c` (0 outside).
    pub fn falloff(&self, c: Point, q: Point) -> f64 {
        let (rx, ry) = self.radii();
        let (s, co) = (-self.angle.to_radians()).sin_cos();
        let d = q - c;
        let u = Vec2::new(d.x * co - d.y * s, d.x * s + d.y * co);
        let r2 = (u.x / rx).powi(2) + (u.y / ry).powi(2);
        if r2 >= 1.0 { 0.0 } else { (1.0 - r2).powi(2) }
    }
}

/// Resample a stroke polyline into dabs spaced `spacing` apart (the first point is always a dab).
pub fn dabs(points: &[Point], spacing: f64) -> Vec<Point> {
    let mut d = Dabber::with(spacing, Some(1.0), false);
    for p in points {
        d.push((*p, 1.0));
    }
    d.dabs.iter().chain(d.tail().as_ref()).map(|d| d.c).collect()
}

/// One brush dab: its centre and intensity (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub c: Point,
    pub intensity: f64,
}

/// A stroke sample: the pointer position and its pen pressure (0..1).
pub type Sample = (Point, f64);

/// Most dabs a stroke makes (later samples add none).
const MAX_DABS: usize = 20_000;

/// Resamples a growing stroke into dabs `spacing` apart along it, the first sample always a dab.
/// More samples only add dabs ([`Self::dabs`] is a prefix of what it becomes), so a stroke can be
/// applied as it grows; the stroke's end, when it is off the spacing, is the [`Self::tail`].
#[derive(Clone, Debug, Default)]
pub struct Dabber {
    spacing: f64,
    /// Every dab's intensity; None: each sample's pressure.
    intensity: Option<f64>,
    /// A repeated sample is one more dab there (the brush held still).
    holds: bool,
    samples: Vec<Sample>,
    /// Distance from the last dab to the last sample.
    carry: f64,
    /// The dabs on the spacing so far.
    pub dabs: Vec<Dab>,
}

impl Dabber {
    /// The dabs of a stroke with brush `prm`.
    pub fn new(prm: &LiquifyParams) -> Self {
        Self::with(prm.dab_spacing(), (!prm.use_pressure).then_some(prm.intensity), prm.kind.holds())
    }

    fn with(spacing: f64, intensity: Option<f64>, holds: bool) -> Self {
        Self { spacing, intensity, holds, ..Self::default() }
    }

    /// The samples so far.
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    fn dab(&self, c: Point, pressure: f64) -> Dab {
        Dab { c, intensity: self.intensity.unwrap_or(pressure.clamp(0.0, 1.0)) }
    }

    /// Add a sample, and the dabs up to it.
    pub fn push(&mut self, (b, pb): Sample) {
        let prev = self.samples.last().copied();
        self.samples.push((b, pb));
        let Some((a, pa)) = prev else {
            self.dabs.push(self.dab(b, pb));
            return;
        };
        if self.dabs.len() >= MAX_DABS {
            return;
        }
        let len = a.distance(b);
        if len < 1e-12 {
            if self.holds {
                self.dabs.push(self.dab(b, pb));
                self.carry = 0.0;
            }
            return;
        }
        let mut s = self.spacing - self.carry;
        while s <= len {
            if self.dabs.len() >= MAX_DABS {
                return;
            }
            let t = s / len;
            self.dabs.push(self.dab(a.lerp(b, t), pa + (pb - pa) * t));
            s += self.spacing;
        }
        self.carry = len - (s - self.spacing);
    }

    /// The dab at the last sample when it is off the spacing (the stroke ends there).
    pub fn tail(&self) -> Option<Dab> {
        let &(end, p) = self.samples.last()?;
        let last = self.dabs.last()?.c;
        (end.distance(last) > self.spacing * 0.25).then(|| self.dab(end, p))
    }
}

fn hash(mut x: u64) -> u64 {
    // splitmix64
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Deterministic noise in [-1, 1].
fn noise(a: u64, b: u64, c: u64) -> f64 {
    let h = hash(a.wrapping_mul(0x1000_0000_01b3) ^ hash(b.wrapping_mul(31) ^ hash(c)));
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

const MAX_ANCHORS: usize = 20_000;

/// Split the segments of `sp` that the brush at `c` touches until they are at most `spacing` long.
/// Straight segments become curves first so the deformation stays smooth.
fn subdivide(sp: &mut SubPath, prm: &LiquifyParams, c: Point, spacing: f64) {
    let bb = prm.brush_bounds(c);
    let mut seg = 0;
    while seg < sp.segment_count() {
        if sp.anchors.len() >= MAX_ANCHORS {
            return;
        }
        let cub = sp.segment(seg);
        let sb = vectorcraft_geom::kurbo::ParamCurveExtrema::bounding_box(&cub);
        let touched = sb.intersect(bb).area() > 0.0 || (sb.width() == 0.0 || sb.height() == 0.0) && sb.inflate(1e-6, 1e-6).intersect(bb).area() > 0.0;
        if !touched || !(0..=8).any(|i| prm.falloff(c, vectorcraft_geom::kurbo::ParamCurve::eval(&cub, i as f64 / 8.0)) > 0.0) {
            seg += 1;
            continue;
        }
        let len = cub.arclen(1e-3);
        // (No more pieces than the anchors left under the ceiling.)
        let k = ((len / spacing).ceil() as usize).min(MAX_ANCHORS.saturating_sub(sp.anchors.len()) + 1);
        if k <= 1 {
            seg += 1;
            continue;
        }
        let n = sp.anchors.len();
        if sp.segment_is_line(seg) {
            let (i0, i1) = (seg % n, (seg + 1) % n);
            let (a, b) = (sp.anchors[i0].p, sp.anchors[i1].p);
            sp.anchors[i0].h_out = a.lerp(b, 1.0 / 3.0);
            sp.anchors[i1].h_in = a.lerp(b, 2.0 / 3.0);
        }
        // Split into k equal-parameter pieces: split at 1/k, then 1/(k-1) of the rest, ...
        let mut cur = seg;
        for j in 0..k - 1 {
            let t = 1.0 / (k - j) as f64;
            cur = sp.insert_anchor(cur, t);
        }
        seg = cur + 1;
    }
}

/// Displace one point (`which`: 0 anchor, 1 in-handle, 2 out-handle).
#[allow(clippy::too_many_arguments)]
fn displace(prm: &LiquifyParams, c: Point, prev: Point, dab: u64, key: u64, q: Point, anchor_new: Option<Point>, anchor_old: Point) -> Point {
    let f = prm.falloff(c, q);
    let i = prm.intensity;
    let (rx, ry) = prm.radii();
    let r = rx.max(ry);
    let d = q - c;
    let len = d.hypot();
    let dir = if len > 1e-9 { d / len } else { Vec2::ZERO };
    match prm.kind {
        LiquifyKind::Warp => q + (c - prev) * (i * f),
        LiquifyKind::Twirl => {
            let a = prm.rate.to_radians() * i * f * 0.25;
            let (s, co) = a.sin_cos();
            c + Vec2::new(d.x * co - d.y * s, d.x * s + d.y * co)
        }
        LiquifyKind::Pucker => q - d * (i * f * 0.2),
        LiquifyKind::Bloat => q + dir * (i * f * 0.1 * r),
        LiquifyKind::Scallop => match anchor_new {
            None => q - d * (i * f * 0.12),
            Some(a) => {
                // Handles swing sideways (curls) and stretch: arc-like details on the outline.
                let h = q - anchor_old;
                let phi = noise(dab, key, 7) * std::f64::consts::FRAC_PI_2 * i * f * (1.0 + prm.complexity * 0.2);
                let (s, co) = phi.sin_cos();
                a + Vec2::new(h.x * co - h.y * s, h.x * s + h.y * co) * (1.0 + 0.6 * i * f)
            }
        },
        LiquifyKind::Crystallize => match anchor_new {
            None => q + dir * (i * f * 0.1 * r * (0.5 + 0.5 * noise(dab, key, 3).abs())),
            // Handles are pulled in: spikes.
            Some(a) => a + (q - anchor_old) * (1.0 - 0.6 * i * f),
        },
        LiquifyKind::Wrinkle => {
            let amp = i * f * 0.06 * r * (1.0 + prm.complexity * 0.1);
            q + Vec2::new(noise(dab, key, 11) * prm.horizontal * amp, noise(dab, key, 13) * prm.vertical * amp)
        }
    }
}

/// Apply one dab to a subpath (after subdivision).
fn apply_dab(sp: &mut SubPath, prm: &LiquifyParams, c: Point, prev: Point, dab: u64, salt: u64) -> bool {
    let bb = prm.brush_bounds(c);
    let mut changed = false;
    for (ai, a) in sp.anchors.iter_mut().enumerate() {
        if !bb.contains(a.p) && !bb.contains(a.h_in) && !bb.contains(a.h_out) {
            continue;
        }
        let key = hash(salt ^ (ai as u64).wrapping_mul(0x9e37_79b9));
        let old = *a;
        let np = if prm.affect_anchors { displace(prm, c, prev, dab, key, old.p, None, old.p) } else { old.p };
        let moved = np - old.p;
        let handle = |h: Point, affect: bool, k: u64| -> Point {
            if h.distance(old.p) < 1e-12 {
                return np;
            }
            if !affect {
                return h + moved;
            }
            match prm.kind {
                LiquifyKind::Scallop | LiquifyKind::Crystallize => displace(prm, c, prev, dab, key ^ k, h, Some(np), old.p),
                _ => displace(prm, c, prev, dab, key ^ k, h, None, old.p),
            }
        };
        let hi = handle(old.h_in, prm.affect_in, 1);
        let ho = handle(old.h_out, prm.affect_out, 2);
        if np != old.p || hi != old.h_in || ho != old.h_out {
            changed = true;
            *a = Anchor {
                p: np,
                h_in: hi,
                h_out: ho,
                kind: if old.kind == AnchorKind::Smooth && prm.kind == LiquifyKind::Crystallize { AnchorKind::Corner } else { old.kind },
            };
        }
    }
    changed
}

/// Distance from `p` to segment `ab`.
fn dist_seg(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let l2 = ab.hypot2();
    if l2 < 1e-18 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Remove anchors lying flat between their neighbours (all control points within `tol` of the chord).
fn simplify(sp: &mut SubPath, tol: f64, region: Rect) {
    if tol <= 0.0 {
        return;
    }
    let mut i = 1;
    loop {
        let n = sp.anchors.len();
        let min = if sp.closed { 4 } else { 3 };
        if n < min {
            return;
        }
        let last = if sp.closed { n } else { n - 1 };
        if i >= last {
            return;
        }
        let (pa, k, pb) = (sp.anchors[i - 1], sp.anchors[i], sp.anchors[(i + 1) % n]);
        let flat = region.contains(k.p)
            && [k.p, k.h_in, k.h_out, pa.h_out, pb.h_in].iter().all(|q| dist_seg(*q, pa.p, pb.p) <= tol)
            && (k.p - pa.p).dot(pb.p - k.p) > 0.0;
        if flat {
            let lab = pa.p.distance(pb.p);
            let (lak, lkb) = (pa.p.distance(k.p).max(1e-9), k.p.distance(pb.p).max(1e-9));
            sp.anchors[i - 1].h_out = pa.p + (pa.h_out - pa.p) * (lab / lak);
            sp.anchors[(i + 1) % n].h_in = pb.p + (pb.h_in - pb.p) * (lab / lkb);
            sp.anchors.remove(i);
        } else {
            i += 1;
        }
    }
}

/// Apply a liquify stroke (its dabs, at the brush's intensity) to `path`. `salt` decorrelates
/// noise between paths. Returns whether anything moved.
pub fn apply_stroke(path: &mut PathData, dab_pts: &[Point], prm: &LiquifyParams, salt: u64) -> bool {
    let dabs: Vec<Dab> = dab_pts.iter().map(|c| Dab { c: *c, intensity: prm.intensity }).collect();
    let mut ps = PathStroke::new(std::mem::take(path), salt);
    ps.advance(&dabs, prm);
    match ps.finish(&dabs, None, prm) {
        Some(done) => {
            *path = done;
            true
        }
        None => {
            *path = ps.path;
            false
        }
    }
}

/// Does the brush box `bb` reach subpath `sp`?
fn touches(sp: &SubPath, bb: Rect) -> bool {
    sp_bounds(sp).is_some_and(|spb| spb.intersect(bb).area() > 0.0 || spb.width() == 0.0 || spb.height() == 0.0)
}

/// Apply dab `d` (number `di`, after the dab at `prev`) to every subpath of `path` it reaches.
fn dab_on(path: &mut PathData, di: usize, d: Dab, prev: Point, prm: &LiquifyParams, salt: u64) -> bool {
    let prm = LiquifyParams { intensity: d.intensity, ..*prm };
    let (bb, spacing) = (prm.brush_bounds(d.c), prm.detail_spacing());
    let mut changed = false;
    for (si, sp) in path.subpaths.iter_mut().enumerate() {
        if !touches(sp, bb) {
            continue;
        }
        subdivide(sp, &prm, d.c, spacing);
        changed |= apply_dab(sp, &prm, d.c, prev, di as u64, hash(salt ^ (si as u64 + 1).wrapping_mul(0x1234_5678_9abc_def1)));
    }
    changed
}

/// A path under a growing stroke: the dabs applied so far (Simplify comes at [`Self::finish`]).
/// Applying a stroke's dabs in any number of steps gives the same path as applying them at once.
#[derive(Clone, Debug)]
pub struct PathStroke {
    path: PathData,
    /// Decorrelates noise between paths.
    salt: u64,
    /// Dabs applied.
    applied: usize,
    /// Where dabs moved something (Simplify works there), None while nothing moved.
    region: Option<Rect>,
}

impl PathStroke {
    pub fn new(path: PathData, salt: u64) -> Self {
        Self { path, salt, applied: 0, region: None }
    }

    /// Apply the stroke's dabs not applied yet.
    pub fn advance(&mut self, dabs: &[Dab], prm: &LiquifyParams) {
        for di in self.applied..dabs.len() {
            let Some(&d) = dabs.get(di) else { break };
            let prev = di.checked_sub(1).and_then(|i| dabs.get(i)).map_or(d.c, |p| p.c);
            if dab_on(&mut self.path, di, d, prev, prm, self.salt) {
                let bb = prm.brush_bounds(d.c);
                self.region = Some(self.region.map_or(bb, |r| r.union(bb)));
            }
        }
        self.applied = self.applied.max(dabs.len());
    }

    /// The path after the stroke (`dabs`, all applied, then `tail`), simplified; None when
    /// nothing moved.
    pub fn finish(&self, dabs: &[Dab], tail: Option<Dab>, prm: &LiquifyParams) -> Option<PathData> {
        let mut region = self.region;
        let mut out = None;
        if let Some(t) = tail.filter(|t| self.path.subpaths.iter().any(|sp| touches(sp, prm.brush_bounds(t.c)))) {
            let mut p = self.path.clone();
            if dab_on(&mut p, dabs.len(), t, dabs.last().map_or(t.c, |d| d.c), prm, self.salt) {
                let bb = prm.brush_bounds(t.c);
                region = Some(region.map_or(bb, |r| r.union(bb)));
            }
            out = Some(p);
        }
        let region = region?;
        let mut path = out.unwrap_or_else(|| self.path.clone());
        let tol = prm.simplify_tolerance();
        if tol > 0.0 {
            for sp in &mut path.subpaths {
                simplify(sp, tol, region);
            }
        }
        Some(path)
    }
}

/// The box around every anchor and handle of `path`: no dab outside it changes the path.
pub fn reach_bounds(path: &PathData) -> Option<Rect> {
    path.subpaths.iter().filter_map(sp_bounds).reduce(|a, b| a.union(b))
}

fn sp_bounds(sp: &SubPath) -> Option<Rect> {
    let mut it = sp.anchors.iter().flat_map(|a| [a.p, a.h_in, a.h_out]);
    let f = it.next()?;
    Some(it.fold(Rect::from_points(f, f), |r, p| r.union_pt(p)))
}

// ---------- the tool ----------

/// Holding the brush still repeats the stroke's last sample this often (seconds): Twirl, Pucker
/// and Bloat keep applying, scaled by the time held.
pub const HOLD_EVERY: f64 = 0.1;

/// Most repeats one tick adds (a minute held).
const MAX_HOLDS_PER_TICK: u32 = 600;

/// An Alt-drag sizing the brush: where it started and the brush size then.
#[derive(Clone, Copy, Debug)]
struct Sizing {
    start: Point,
    width: f64,
    height: f64,
}

pub struct LiquifyTool {
    pub params: LiquifyParams,
    /// Show Brush Size: the brush outline follows the pointer.
    pub show_brush: bool,
    /// The stroke's samples (position, pen pressure).
    points: Vec<Sample>,
    active: bool,
    sizing: Option<Sizing>,
    hover: Option<Point>,
    /// Time held still since the last sample or repeat (seconds).
    held: f64,
    /// How many samples the last preview of the stroke had.
    previewed: usize,
}

impl LiquifyTool {
    pub fn new(id: &str) -> Self {
        let kind = LiquifyKind::parse(id).unwrap_or(LiquifyKind::Warp);
        Self { params: LiquifyParams::new(kind), show_brush: true, points: vec![], active: false, sizing: None, hover: None, held: 0.0, previewed: 0 }
    }

    /// The command + params for the stroke so far (with each sample's pressure while Use Pressure
    /// Pen is on).
    pub fn command(&self, cx: &ToolContext) -> (String, Value) {
        let mut v = self.params.to_json();
        let pressure = self.params.use_pressure;
        v["points"] = Value::Array(self.points.iter().map(|(p, f)| if pressure { json!([p.x, p.y, f]) } else { json!([p.x, p.y]) }).collect());
        if !cx.selection.is_empty() {
            v["ids"] = crate::json_ids(&cx.selection.objects);
        }
        ("object.liquify".into(), v)
    }

    /// The preview of the stroke so far, when [`crate::previews_at`] its length.
    fn preview(&mut self, cx: &ToolContext) -> Vec<Action> {
        if !crate::previews_at(self.points.len()) {
            return vec![];
        }
        self.previewed = self.points.len();
        let (c, v) = self.command(cx);
        vec![Action::Preview(c, v)]
    }

    /// The preview of the whole stroke, when samples came after the last one shown: what is
    /// committed is the last preview.
    fn stale_preview(&mut self, cx: &ToolContext) -> Option<Action> {
        (self.previewed != self.points.len() && !self.points.is_empty()).then(|| {
            self.previewed = self.points.len();
            let (c, v) = self.command(cx);
            Action::Preview(c, v)
        })
    }

    /// Alt-drag from `s` to `p`: the brush grows or shrinks from its size at the press (its edge
    /// follows the pointer); with Shift it keeps its proportions.
    fn resize(&mut self, s: Sizing, p: Point, proportional: bool) {
        let (w, h) = (s.width + 2.0 * (p.x - s.start.x), s.height + 2.0 * (p.y - s.start.y));
        let (w, h) = if !proportional {
            (w, h)
        } else if (p.x - s.start.x).abs() >= (p.y - s.start.y).abs() {
            (w, s.height * w / s.width)
        } else {
            (s.width * h / s.height, h)
        };
        self.params.width = w.clamp(1.0, 10_000.0);
        self.params.height = h.clamp(1.0, 10_000.0);
    }
}

impl Tool for LiquifyTool {
    fn id(&self) -> &'static str {
        self.params.kind.id()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        self.hover = Some(p);
        let sample = (p, f64::from(ev.pressure).clamp(0.0, 1.0));
        match ev.kind {
            PointerKind::Down if ev.mods.alt => {
                self.sizing = Some(Sizing { start: p, width: self.params.width, height: self.params.height });
                vec![]
            }
            PointerKind::Down => {
                self.points = vec![sample];
                self.active = true;
                self.held = 0.0;
                self.previewed = self.points.len();
                let (c, v) = self.command(cx);
                vec![Action::Begin(self.params.kind.label().into()), Action::Preview(c, v)]
            }
            PointerKind::Drag => {
                if let Some(s) = self.sizing {
                    self.resize(s, p, ev.mods.shift);
                    self.hover = Some(s.start);
                    return vec![];
                }
                if !self.active
                    || self.points.last().is_some_and(|(l, _)| l.distance(p) < cx.tol(2.0))
                    || self.points.len() >= crate::MAX_STROKE_SAMPLES
                {
                    return vec![];
                }
                self.points.push(sample);
                self.held = 0.0;
                self.preview(cx)
            }
            PointerKind::Up => {
                if self.sizing.take().is_some() {
                    return vec![];
                }
                if !self.active {
                    return vec![];
                }
                self.active = false;
                let out = self.stale_preview(cx).into_iter().chain([Action::Commit]).collect();
                self.points.clear();
                out
            }
            _ => vec![],
        }
    }
    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        let Some(c) = self.hover.filter(|_| self.show_brush || self.sizing.is_some()) else { return vec![] };
        vec![Overlay::Path {
            path: ellipse_path(c, self.params.width / 2.0, self.params.height / 2.0, self.params.angle),
            color: [0x80, 0x80, 0x80],
            width: 1.0,
            dashed: false,
        }]
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }
    /// Every option ([`LiquifyParams::options_json`]) and `showBrush`.
    fn options(&self) -> Value {
        let mut v = self.params.options_json();
        v["showBrush"] = json!(self.show_brush);
        v
    }
    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            // The kind is the tool's: another tool is another tool.
            "tool" => {}
            "showBrush" => self.show_brush = value.as_bool().unwrap_or(self.show_brush),
            _ => {
                let mut v = self.params.options_json();
                v[key] = value.clone();
                if key == "diameter" {
                    v["width"] = value.clone();
                    v["height"] = value.clone();
                }
                if let Some(p) = LiquifyParams::from_json(&v) {
                    self.params = p;
                }
            }
        }
    }
    fn busy(&self) -> bool {
        self.active
    }
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.hover = None;
        self.sizing = None;
        if !std::mem::take(&mut self.active) {
            return vec![];
        }
        let out = self.stale_preview(cx).into_iter().chain([Action::Commit]).collect();
        self.points.clear();
        out
    }
    /// Held still: every [`HOLD_EVERY`] the last sample repeats (one more dab there).
    fn tick(&mut self, cx: &ToolContext, dt: f64) -> Vec<Action> {
        if !self.wants_ticks() || !dt.is_finite() || dt <= 0.0 {
            return vec![];
        }
        self.held += dt;
        let mut n = 0;
        // A hair under the period, so a sum of ticks that is one in exact arithmetic counts.
        while self.held >= HOLD_EVERY - 1e-9 && n < MAX_HOLDS_PER_TICK {
            self.held -= HOLD_EVERY;
            n += 1;
        }
        self.held = self.held.clamp(0.0, HOLD_EVERY);
        let Some(&last) = self.points.last().filter(|_| n > 0) else { return vec![] };
        let room = crate::MAX_STROKE_SAMPLES.saturating_sub(self.points.len());
        // A full stroke takes no sample: the preview it already has stands.
        if room == 0 {
            return vec![];
        }
        self.points.extend(std::iter::repeat_n(last, (n as usize).min(room)));
        self.preview(cx)
    }
    fn wants_ticks(&self) -> bool {
        self.active && self.params.kind.holds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::shapes;

    fn square() -> PathData {
        shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0))
    }

    fn prm(kind: LiquifyKind) -> LiquifyParams {
        LiquifyParams { intensity: 1.0, ..LiquifyParams::new(kind) }
    }

    #[test]
    fn dabs_are_evenly_spaced() {
        let d = dabs(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], 10.0);
        assert_eq!(d.len(), 11);
        assert!(d.windows(2).all(|w| (w[0].distance(w[1]) - 10.0).abs() < 1e-9));
        assert_eq!(dabs(&[Point::new(5.0, 5.0)], 10.0), vec![Point::new(5.0, 5.0)]);
    }

    #[test]
    fn one_long_segment_stays_within_the_dab_cap() {
        let mut h = Dabber::with(1.0, Some(1.0), true);
        for _ in 0..MAX_DABS + 100 {
            h.push((Point::new(3.0, 3.0), 1.0));
        }
        assert_eq!(h.dabs.len(), MAX_DABS);
        let d = dabs(&[Point::new(0.0, 0.0), Point::new(1.0e9, 0.0)], 1.0);
        assert!(d.len() <= MAX_DABS + 1, "{} dabs", d.len());
        let d = dabs(&[Point::new(0.0, 0.0), Point::new(f64::MAX, f64::MAX)], 1.0);
        assert!(d.len() <= MAX_DABS + 1, "{} dabs", d.len());
    }

    #[test]
    fn warp_is_deterministic_and_localized() {
        let pts = [Point::new(200.0, 100.0), Point::new(240.0, 100.0)];
        let p = prm(LiquifyKind::Warp);
        let d = dabs(&pts, p.dab_spacing());
        let mut a = square();
        let mut b = square();
        assert!(apply_stroke(&mut a, &d, &p, 7));
        apply_stroke(&mut b, &d, &p, 7);
        assert_eq!(a, b, "same input, same output");
        // The right edge bulged outward near y = 100; the left edge (x = 0) did not move.
        let bb = a.bounds().unwrap();
        assert!(bb.x1 > 210.0, "{bb:?}");
        assert!(a.anchors().all(|(_, _, an)| an.p.x > 150.0 || (an.p.x - 0.0).abs() < 1e-9));
        assert!(a.anchor_count() > 4, "detail added anchors");
        // Anchors outside the brush are untouched.
        assert!(a.anchors().any(|(_, _, an)| an.p == Point::new(0.0, 0.0)));
    }

    #[test]
    fn pucker_and_bloat_move_toward_and_away_from_centre() {
        let c = Point::new(200.0, 100.0);
        let p = prm(LiquifyKind::Pucker);
        let mut a = square();
        apply_stroke(&mut a, &[c], &p, 1);
        let near = |pd: &PathData| pd.anchors().filter(|(_, _, an)| an.p.distance(c) < 50.0).map(|(_, _, an)| an.p.x).fold(f64::MIN, f64::max);
        assert!(near(&a) <= 200.0 + 1e-9);
        let mut b = square();
        // A brush just inside the right edge bulges it outward; the far edges stay put.
        apply_stroke(&mut b, &[Point::new(180.0, 100.0); 3], &prm(LiquifyKind::Bloat), 1);
        let bb = b.bounds().unwrap();
        assert!(bb.x1 > 205.0, "{bb:?}");
        assert_eq!((bb.x0, bb.y0, bb.y1), (0.0, 0.0, 200.0));
    }

    #[test]
    fn twirl_rotates_about_the_brush_centre() {
        let c = Point::new(100.0, 100.0);
        let mut pd = PathData::single(SubPath::polyline(&[Point::new(80.0, 100.0), Point::new(120.0, 100.0)], false));
        let p = LiquifyParams { detail: 1.0, simplify: 0.0, ..prm(LiquifyKind::Twirl) };
        apply_stroke(&mut pd, &[c], &p, 0);
        // Rotation preserves every anchor's distance to the centre (ends at 20, the midpoint at 0).
        for (_, _, a) in pd.anchors() {
            let d = a.p.distance(c);
            assert!((d - 20.0).abs() < 1e-9 || d < 1e-9, "{d}");
        }
        let first = pd.subpaths[0].anchors[0].p;
        assert!(first.y != 100.0 && (first.distance(c) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn noisy_tools_are_reproducible() {
        for kind in [LiquifyKind::Scallop, LiquifyKind::Crystallize, LiquifyKind::Wrinkle] {
            let p = LiquifyParams { horizontal: 1.0, ..prm(kind) };
            let d = dabs(&[Point::new(200.0, 50.0), Point::new(200.0, 150.0)], p.dab_spacing());
            let (mut a, mut b) = (square(), square());
            assert!(apply_stroke(&mut a, &d, &p, 3), "{kind:?}");
            apply_stroke(&mut b, &d, &p, 3);
            assert_eq!(a, b, "{kind:?}");
            assert_ne!(a, square());
        }
    }

    #[test]
    fn params_round_trip_through_json() {
        let mut p = LiquifyParams::new(LiquifyKind::Wrinkle);
        p.horizontal = 0.3;
        p.width = 60.0;
        let q = LiquifyParams::from_json(&p.to_json()).unwrap();
        assert_eq!(p, q);
        let r = LiquifyParams::from_json(&json!({"tool": "warp", "diameter": 40, "intensity": 80})).unwrap();
        assert_eq!((r.width, r.height, r.intensity), (40.0, 40.0, 0.8));
    }
}
