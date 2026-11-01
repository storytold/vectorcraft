//! Define Perspective Grid: the dialog's view of the grid ([`GridDefinition`]: type, units, scale,
//! gridline spacing, viewing angle and distance, horizon height, third vanishing point, gridline
//! colours and opacity), the camera behind it (the [`Station`] point) and the built-in presets.
//!
//! The station point is the viewer: `D` points in front of the picture plane, level with the
//! horizon at the centre of vision `cv`. In two- and three-point perspective the viewing angle θ
//! is the angle between the left plane and the picture plane, so the vanishing points sit at
//! `cv − D·cot θ` and `cv + D·tan θ` (the station point lies on the circle over them). A receding
//! axis foreshortens by the distance from the viewer to its vanishing point, which keeps the grid
//! cells square in the scene whatever the angle.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vectorcraft_doc::{Document, Unit};
use vectorcraft_geom::Rect;

use super::{PerspectiveGrid, Plane};

/// A Letter page: where presets are fitted without a document.
pub const LETTER: Rect = Rect::new(0.0, 0.0, 612.0, 792.0);

/// The viewing angle of grids made before it was stored, and of the presets.
pub const DEFAULT_ANGLE: f64 = 45.0;

/// The largest length (points) a grid takes, as [`PerspectiveGrid::validate`] allows.
const MAX_LEN: f64 = 4.0e6;

/// The built-in presets: (name, kind, horizon height as a fraction of the artboard height from
/// the top). The ground line sits at 78 % of the height in all of them.
pub const BUILTINS: [(&str, u8, f64); 8] = [
    ("[1P-Normal View]", 1, 0.42),
    ("[1P-Low View]", 1, 0.66),
    ("[1P-High View]", 1, 0.12),
    ("[2P-Normal View]", 2, 0.42),
    ("[2P-Low View]", 2, 0.66),
    ("[2P-High View]", 2, 0.12),
    ("[3P-Normal View]", 3, 0.42),
    ("[3P-Low View]", 3, 0.66),
];

/// Is `name` a built-in preset's (any case)?
pub fn is_builtin(name: &str) -> bool {
    BUILTINS.iter().any(|b| b.0.eq_ignore_ascii_case(name.trim()))
}

/// The first artboard of `doc` (a Letter page without one).
pub fn first_artboard(doc: &Document) -> Rect {
    doc.artboards.first().map_or(LETTER, |a| a.rect)
}

// Serde defaults of the grid's definition fields.
pub(super) fn points() -> String {
    Unit::Points.key().into()
}
pub(super) fn is_points(u: &String) -> bool {
    u == Unit::Points.key()
}
pub(super) fn one_to_one() -> [f64; 2] {
    [1.0, 1.0]
}
pub(super) fn is_one_to_one(s: &[f64; 2]) -> bool {
    *s == one_to_one()
}
pub(super) fn left_rgb() -> Rgb {
    Rgb(Plane::Left.color())
}
pub(super) fn right_rgb() -> Rgb {
    Rgb(Plane::Right.color())
}
pub(super) fn ground_rgb() -> Rgb {
    Rgb(Plane::Ground.color())
}
pub(super) fn half() -> f64 {
    50.0
}

/// A `#rrggbb` colour (gridline colours).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// `#rrggbb` or `rrggbb`.
    pub fn parse(s: &str) -> Option<Self> {
        let h = s.trim();
        let h = h.strip_prefix('#').unwrap_or(h);
        if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
        Some(Self([byte(0)?, byte(2)?, byte(4)?]))
    }
    /// `#rrggbb`.
    pub fn hex(self) -> String {
        let [r, g, b] = self.0;
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

impl TryFrom<String> for Rgb {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::parse(&s).ok_or_else(|| format!("`{s}` isn't a #rrggbb colour"))
    }
}

impl From<Rgb> for String {
    fn from(c: Rgb) -> String {
        c.hex()
    }
}

/// The viewer: the centre of vision `x` on the horizon and the distance to the picture plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Station {
    pub x: f64,
    pub distance: f64,
}

/// Define Perspective Grid's fields: the grid as the dialog shows it. Lengths are real-world
/// lengths in `units`, drawn at `scale` (`[artboard, real world]`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GridDefinition {
    /// The preset's name ("" = custom).
    pub name: String,
    /// 1, 2 or 3-point perspective.
    pub kind: u8,
    /// A unit name (`points`, `pixels`, `inches`, `centimeters`…).
    pub units: String,
    /// `[artboard, real world]`: 1:4 draws 4 real-world units as 1.
    pub scale: [f64; 2],
    /// Gridline every (real-world units).
    pub gridline: f64,
    /// Viewing angle (degrees, between 0 and 90; two/three-point).
    pub angle: f64,
    /// Viewing distance: the station point's distance from the picture plane.
    pub distance: f64,
    /// Horizon height above the ground level.
    pub horizon_height: f64,
    /// The third vanishing point (three-point) from the centre of vision: x to the right, y up.
    pub third_vp: [f64; 2],
    pub left_color: Rgb,
    pub right_color: Rgb,
    pub ground_color: Rgb,
    /// Gridline opacity, 0–100 %.
    pub opacity: f64,
}

impl Default for GridDefinition {
    /// [2P-Normal View] on a Letter page.
    fn default() -> Self {
        PerspectiveGrid::normal(2, LETTER).definition()
    }
}

impl GridDefinition {
    /// The unit the lengths are in.
    pub fn unit(&self) -> Unit {
        Unit::named(&self.units).unwrap_or_default()
    }

    /// Artboard points per real-world unit.
    pub fn ratio(&self) -> f64 {
        self.unit().points() * self.scale[0] / self.scale[1]
    }

    /// Merge `patch` (any subset of the fields; other keys are ignored) into this definition.
    pub fn merged(&self, patch: &Value) -> Result<Self, String> {
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if let (Some(o), Some(p)) = (v.as_object_mut(), patch.as_object()) {
            for (k, val) in p {
                if o.contains_key(k) {
                    o.insert(k.clone(), val.clone());
                }
            }
        }
        let mut d: Self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        d.units = Unit::named(&d.units).ok_or_else(|| format!("unknown unit `{}`", d.units))?.key().into();
        d.validate()?;
        Ok(d)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3).contains(&self.kind) {
            return Err("kind must be 1, 2 or 3".into());
        }
        if Unit::named(&self.units).is_none() {
            return Err(format!("unknown unit `{}`", self.units));
        }
        if !self.scale.iter().all(|s| s.is_finite() && *s > 0.0 && *s <= 1.0e6) {
            return Err("scale must be two positive numbers [artboard, real world]".into());
        }
        let r = self.ratio();
        let lens = [self.gridline, self.distance, self.horizon_height, self.third_vp[0], self.third_vp[1]];
        if lens.iter().any(|v| !(v * r).is_finite() || (v * r).abs() > MAX_LEN) {
            return Err("grid lengths must be finite".into());
        }
        if self.gridline * r <= 0.0 || self.distance * r <= 0.0 {
            return Err("gridline spacing and viewing distance must be positive".into());
        }
        if (self.horizon_height * r).abs() < 1e-6 {
            return Err("the horizon height can't be 0".into());
        }
        if !(self.angle > 0.0 && self.angle < 90.0) {
            return Err("the viewing angle must be between 0 and 90°".into());
        }
        if !(0.0..=100.0).contains(&self.opacity) {
            return Err("opacity must be 0–100".into());
        }
        Ok(())
    }

    /// The same grid (names aside, lengths compared in points)?
    pub fn same(&self, o: &Self) -> bool {
        let (r, s) = (self.ratio(), o.ratio());
        let near = |a: f64, b: f64| (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0);
        self.kind == o.kind
            && near(self.gridline * r, o.gridline * s)
            && near(self.distance * r, o.distance * s)
            && near(self.horizon_height * r, o.horizon_height * s)
            && (self.kind != 3 || near(self.third_vp[0] * r, o.third_vp[0] * s) && near(self.third_vp[1] * r, o.third_vp[1] * s))
            && (self.kind == 1 || near(self.angle, o.angle))
            && (self.left_color, self.right_color, self.ground_color) == (o.left_color, o.right_color, o.ground_color)
            && near(self.opacity, o.opacity)
    }
}

impl PerspectiveGrid {
    /// The built-in preset `name` (any case) fitted to `ab`.
    pub fn builtin(name: &str, ab: Rect) -> Option<Self> {
        let &(name, kind, horizon) = BUILTINS.iter().find(|b| b.0.eq_ignore_ascii_case(name.trim()))?;
        let mut g = Self::preset(kind, ab);
        let ground = g.origin[1];
        g.horizon = ab.y0 + horizon * ab.height().max(1.0);
        g.vp_vertical = [g.vp_vertical[0], g.horizon - 3.0 * (ground - g.horizon)];
        g.name = name.into();
        g.set_station(g.station(), DEFAULT_ANGLE);
        Some(g)
    }

    /// The normal view preset of `kind` (1–3) fitted to `ab`.
    pub fn normal(kind: u8, ab: Rect) -> Self {
        let name = BUILTINS.iter().find(|b| b.1 == kind.clamp(1, 3)).map_or("", |b| b.0);
        Self::builtin(name, ab).unwrap_or_else(|| Self::preset(kind, ab))
    }

    /// The viewing angle in degrees.
    pub fn viewing_angle(&self) -> f64 {
        self.angle.unwrap_or(DEFAULT_ANGLE)
    }

    /// The station point: one-point grids look straight at their vanishing point from
    /// `distance`; two/three-point ones stand where the viewing angle puts them (grids without
    /// one: halfway between the vanishing points).
    pub fn station(&self) -> Station {
        let w = self.vp_right - self.vp_left;
        match self.angle {
            _ if self.kind == 1 => Station { x: self.vp_left, distance: self.distance },
            Some(a) if w > 0.0 => {
                let (s, c) = a.to_radians().sin_cos();
                Station { x: self.vp_left + w * c * c, distance: w * s * c }
            }
            _ => Station { x: (self.vp_left + self.vp_right) / 2.0, distance: if w.abs() > 1e-9 { w.abs() / 2.0 } else { self.distance } },
        }
    }

    /// Place the vanishing points for a viewer at `st` looking at `angle` degrees (one-point: the
    /// vanishing point at the centre of vision).
    pub fn set_station(&mut self, st: Station, angle: f64) {
        self.distance = st.distance;
        self.angle = Some(angle);
        if self.kind == 1 {
            self.vp_left = st.x;
        } else {
            let t = angle.to_radians().tan();
            self.vp_left = st.x - st.distance / t;
            self.vp_right = st.x + st.distance * t;
        }
    }

    /// 1 / the measuring distance of the left, right and vertical axes: the viewer's distance to
    /// their vanishing points (grids without a viewing angle: `distance` for all).
    pub(super) fn foreshortening(&self) -> (f64, f64, f64) {
        let uniform = 1.0 / self.distance.max(1e-9);
        if self.kind == 1 || self.angle.is_none() || self.vp_right <= self.vp_left {
            return (uniform, uniform, uniform);
        }
        let st = self.station();
        let to = |x: f64, y: f64| 1.0 / (st.distance.powi(2) + (x - st.x).powi(2) + (y - self.horizon).powi(2)).sqrt().max(1e-9);
        (to(self.vp_left, self.horizon), to(self.vp_right, self.horizon), to(self.vp_vertical[0], self.vp_vertical[1]))
    }

    /// After merging `patch` over `old`: a new viewing angle, viewing distance or type moves the
    /// vanishing points around the old station point (unless the patch places them), and
    /// `distance` reads the station's.
    pub(super) fn reconcile(&mut self, old: &Self, patch: &Value) {
        let has = |k: &str| patch.get(k).is_some();
        let camera = has("angle") || has("distance") && self.kind != 1;
        if !(has("vpLeft") || has("vpRight")) && (camera || self.kind != old.kind) {
            let st = old.station();
            let st = Station { distance: if has("distance") { self.distance } else { st.distance }, ..st };
            self.set_station(st, self.viewing_angle());
        }
        if self.kind != 1 && self.angle.is_some() && self.vp_right > self.vp_left {
            self.distance = self.station().distance;
        }
    }

    /// The definition fields' part of [`PerspectiveGrid::validate`].
    pub(super) fn validate_definition(&self) -> Result<(), String> {
        if self.angle.is_some_and(|a| !(a > 0.0 && a < 90.0)) {
            return Err("the viewing angle must be between 0 and 90°".into());
        }
        if self.kind != 1 && self.angle.is_some() && self.vp_right <= self.vp_left {
            return Err("the right vanishing point must be right of the left one".into());
        }
        if Unit::named(&self.units).is_none() {
            return Err(format!("unknown unit `{}`", self.units));
        }
        if !self.scale.iter().all(|s| s.is_finite() && *s > 0.0 && *s <= 1.0e6) {
            return Err("scale must be two positive numbers [artboard, real world]".into());
        }
        if !(0.0..=100.0).contains(&self.opacity) {
            return Err("opacity must be 0–100".into());
        }
        if self.extent_right.is_some_and(|e| !(e > 0.0 && e <= MAX_LEN)) {
            return Err("extentRight must be positive".into());
        }
        Ok(())
    }

    /// The grid as Define Grid shows it.
    pub fn definition(&self) -> GridDefinition {
        let mut d = GridDefinition {
            name: self.name.clone(),
            kind: self.kind,
            units: self.units.clone(),
            scale: self.scale,
            gridline: 0.0,
            angle: self.viewing_angle(),
            distance: 0.0,
            horizon_height: 0.0,
            third_vp: [0.0; 2],
            left_color: self.left_color,
            right_color: self.right_color,
            ground_color: self.ground_color,
            opacity: self.opacity,
        };
        let r = d.ratio();
        let st = self.station();
        d.gridline = self.cell / r;
        d.distance = st.distance / r;
        d.horizon_height = (self.origin[1] - self.horizon) / r;
        d.third_vp = [(self.vp_vertical[0] - st.x) / r, (self.horizon - self.vp_vertical[1]) / r];
        d
    }

    /// This grid redefined by `def`: only what differs from [`Self::definition`] changes, so the
    /// unchanged fields leave the grid exactly as it was. A new type, viewing angle or distance
    /// moves the vanishing points around the station point.
    pub fn with_definition(&self, def: &GridDefinition) -> Result<Self, String> {
        def.validate()?;
        let cur = self.definition();
        let r = def.ratio();
        let near = |a: f64, b: f64| (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0);
        let cr = cur.ratio();
        let mut g = self.clone();
        g.name = def.name.clone();
        g.units = def.unit().key().into();
        g.scale = def.scale;
        (g.left_color, g.right_color, g.ground_color, g.opacity) = (def.left_color, def.right_color, def.ground_color, def.opacity);
        g.kind = def.kind;
        if !near(def.gridline * r, cur.gridline * cr) {
            g.cell = def.gridline * r;
        }
        if !near(def.horizon_height * r, cur.horizon_height * cr) {
            g.horizon = g.origin[1] - def.horizon_height * r;
        }
        if def.kind != self.kind || !near(def.distance * r, cur.distance * cr) || def.kind != 1 && !near(def.angle, cur.angle) {
            let st = Station { distance: def.distance * r, ..self.station() };
            g.set_station(st, def.angle);
        }
        if !(near(def.third_vp[0] * r, cur.third_vp[0] * cr) && near(def.third_vp[1] * r, cur.third_vp[1] * cr)) || g.horizon != self.horizon {
            let x = g.station().x;
            g.vp_vertical = [x + def.third_vp[0] * r, g.horizon - def.third_vp[1] * r];
        }
        g.validate()?;
        Ok(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_geom::Point;

    const AB: Rect = Rect::new(0.0, 0.0, 800.0, 600.0);

    #[test]
    fn rgb_rejects_repeated_hash() {
        assert!(Rgb::parse("##ff0000").is_none());
        assert!(Rgb::parse("####00ff00").is_none());
        assert!(Rgb::try_from(String::from("##ff0000")).is_err());
        assert_eq!(Rgb::parse("#ff0000"), Some(Rgb([255, 0, 0])));
        assert_eq!(Rgb::parse("ff0000"), Some(Rgb([255, 0, 0])));
    }

    #[test]
    fn the_station_point_places_the_vanishing_points() {
        let mut g = PerspectiveGrid::normal(2, AB);
        let st = g.station();
        assert!((st.x - g.origin[0]).abs() < 1e-9, "the normal view looks at the corner");
        // At 30° the left plane turns toward the picture plane: its vanishing point recedes.
        g.set_station(st, 30.0);
        assert!((st.x - g.vp_left - st.distance * 3f64.sqrt()).abs() < 1e-9);
        assert!((g.vp_right - st.x - st.distance / 3f64.sqrt()).abs() < 1e-9);
        let back = g.station();
        assert!((back.x - st.x).abs() < 1e-9 && (back.distance - st.distance).abs() < 1e-9);
        // The station point sees the vanishing points at a right angle.
        let (a, b) = (Point::new(g.vp_left - st.x, st.distance), Point::new(g.vp_right - st.x, st.distance));
        assert!((a.x * b.x + a.y * b.y).abs() < 1e-6 * st.distance * st.distance);
    }

    #[test]
    fn receding_axes_keep_true_lengths_at_the_picture_plane() {
        // A true camera: at the picture plane a step along a receding axis spans its true width
        // (cos θ of it on the left, sin θ on the right), whatever the viewing angle.
        for angle in [20.0, 45.0, 70.0] {
            let mut g = PerspectiveGrid::normal(2, AB);
            g.set_station(g.station(), angle);
            g.origin[0] = g.station().x;
            for pl in [Plane::Left, Plane::Right] {
                let h = g.homography(pl).unwrap();
                let (o, e) = (h.apply(Point::new(0.0, 0.0)).unwrap(), h.apply(Point::new(1e-3, 0.0)).unwrap());
                let s = angle.to_radians();
                let dx = if pl == Plane::Left { s.cos() } else { s.sin() };
                assert!(((e.x - o.x).abs() / 1e-3 - dx).abs() < 1e-3, "{angle} {pl:?}");
            }
        }
    }

    #[test]
    fn definition_round_trips_and_unchanged_fields_change_nothing() {
        for kind in [1u8, 2, 3] {
            let g = PerspectiveGrid::normal(kind, AB);
            let d = g.definition();
            assert_eq!(g.with_definition(&d).unwrap(), g);
            // In inches at 1:4 the same grid reads differently but is the same.
            let inches = GridDefinition { units: "inches".into(), scale: [1.0, 4.0], ..d.clone() };
            let r = inches.ratio() / d.ratio();
            let inches = GridDefinition {
                gridline: d.gridline / r,
                distance: d.distance / r,
                horizon_height: d.horizon_height / r,
                third_vp: d.third_vp.map(|v| v / r),
                ..inches
            };
            let g2 = g.with_definition(&inches).unwrap();
            assert_eq!((g2.origin, g2.horizon, g2.vp_left, g2.vp_right, g2.cell), (g.origin, g.horizon, g.vp_left, g.vp_right, g.cell));
            assert!(g2.definition().same(&d));
        }
        // A grid from before the viewing angle keeps its foreshortening through an unchanged OK.
        let legacy = PerspectiveGrid::preset(2, AB);
        let d = legacy.definition();
        assert_eq!(legacy.with_definition(&GridDefinition { opacity: 80.0, ..d.clone() }).unwrap().angle, None);
        assert!(legacy.with_definition(&GridDefinition { angle: 30.0, ..d }).unwrap().angle.is_some());
    }

    #[test]
    fn definition_fields_map_to_the_grid() {
        let g = PerspectiveGrid::normal(3, AB);
        let d = GridDefinition {
            gridline: 2.0,
            angle: 30.0,
            distance: 5.0,
            horizon_height: 3.0,
            third_vp: [1.0, 8.0],
            units: "inches".into(),
            ..g.definition()
        };
        let n = g.with_definition(&d).unwrap();
        let st = n.station();
        assert!((n.cell - 144.0).abs() < 1e-9);
        assert!((st.distance - 360.0).abs() < 1e-6 && (st.x - g.station().x).abs() < 1e-6, "the viewer stays put");
        assert!((n.viewing_angle() - 30.0).abs() < 1e-9);
        assert!((n.origin[1] - n.horizon - 216.0).abs() < 1e-9);
        assert!((n.vp_vertical[0] - st.x - 72.0).abs() < 1e-6 && (n.horizon - n.vp_vertical[1] - 576.0).abs() < 1e-6);
        assert_eq!(n.units, "inches");
        // Bad values are refused.
        for bad in [
            json!({"angle": 90}),
            json!({"distance": 0}),
            json!({"units": "furlongs"}),
            json!({"scale": [1, 0]}),
            json!({"opacity": 120}),
            json!({"kind": 4}),
        ] {
            assert!(d.merged(&bad).is_err(), "{bad}");
        }
        assert_eq!(d.merged(&json!({"units": "mm", "leftColor": "#00ff00"})).unwrap().units, "millimeters");
        assert!(d.merged(&json!({"leftColor": "green"})).is_err());
    }

    #[test]
    fn set_moves_the_vanishing_points_for_a_new_angle_or_distance() {
        let g = PerspectiveGrid::normal(2, AB);
        let st = g.station();
        let n = g.merged(&json!({"angle": 60})).unwrap();
        assert!((n.station().x - st.x).abs() < 1e-6 && (n.station().distance - st.distance).abs() < 1e-6);
        let n = g.merged(&json!({"distance": 100})).unwrap();
        assert!((n.vp_right - n.vp_left - 200.0).abs() < 1e-6);
        // Moving a vanishing point keeps the angle; `distance` follows.
        let n = g.merged(&json!({"vpLeft": g.vp_left - 100.0})).unwrap();
        assert_eq!(n.angle, Some(45.0));
        assert!((n.distance - (n.vp_right - n.vp_left) / 2.0).abs() < 1e-6);
        assert!(g.merged(&json!({"vpRight": g.vp_left - 10.0})).is_err());
    }

    #[test]
    fn rgb_parses_hex() {
        assert_eq!(Rgb::parse("#3366ff"), Some(Rgb([0x33, 0x66, 0xff])));
        assert_eq!(Rgb::parse("ff8C1A").map(Rgb::hex).as_deref(), Some("#ff8c1a"));
        for bad in ["", "#12345", "#gg0000", "é12345", "#1234567", "+f+f+f"] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
    }
}
