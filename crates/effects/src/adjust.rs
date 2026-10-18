//! Colour adjustment effects (Effect → Color Adjustments): Brightness/Contrast, Curves, Levels,
//! Hue/Saturation, Shift to Color and Temperature/Tint.
//!
//! They recolour what an object paints with, non-destructively: its fills and strokes (solid
//! colours and gradient stops), type, gradient meshes, embedded images and, on a group or layer,
//! every member — through the colour visitor the Edit Colors commands use
//! ([`vectorcraft_doc::recolor`]). Each colour keeps its colour model. An effect on one fill or
//! stroke recolours that paint only. [`adjust`] evaluates them, innermost first and each list in
//! stack order; the renderer draws the result (caching recoloured copies of embedded images) and
//! [`crate::bake_document`] writes it for export, recoloured images as new embedded images.
//! Pattern tiles, and the shadows and glows of raster effects, keep their colours.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{Color, Paint, keep_model};
use vectorcraft_doc::recolor::{ColorVisitor, Reach, map_paint, recolor_node};
use vectorcraft_doc::{Document, Effect, Node, NodeKind};

use crate::merged_params;
use crate::raster::color_param;
use crate::util::{flag, num, text};

/// The colour adjustment effects, in menu order.
pub const ADJUSTMENTS: [&str; 6] =
    ["adjust.brightnessContrast", "adjust.curves", "adjust.hueSaturation", "adjust.levels", "adjust.shiftToColor", "adjust.temperatureTint"];

/// Is `id` a colour adjustment effect?
pub fn is_adjustment(id: &str) -> bool {
    ADJUSTMENTS.contains(&id)
}

fn visible_adjustment(e: &Effect) -> bool {
    e.visible && is_adjustment(&e.id)
}

/// Does `n` carry a visible colour adjustment, on the object or on one of its fills or strokes?
pub fn has_adjustment(n: &Node) -> bool {
    n.appearance.effects.iter().any(visible_adjustment) || n.appearance.items.iter().any(|i| i.effects().iter().any(visible_adjustment))
}

/// The channels a tone curve (Curves, Levels) applies to.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Channel {
    All,
    One(usize),
}

impl Channel {
    fn of(p: &Value) -> Self {
        Self::parse(text(p, "channel", "rgb")).unwrap_or(Channel::All)
    }

    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "rgb" => Some(Channel::All),
            "red" | "r" => Some(Channel::One(0)),
            "green" | "g" => Some(Channel::One(1)),
            "blue" | "b" => Some(Channel::One(2)),
            _ => None,
        }
    }
}

/// The channels Curves and Levels apply to (`channel`), as their parameters list them.
pub const CHANNELS: [&str; 4] = ["rgb", "red", "green", "blue"];

/// Do Curves and Levels read `s` as their `channel`: one of [`CHANNELS`], or r, g or b, in any
/// case? They draw any other value on all three channels, as RGB.
pub fn is_channel(s: &str) -> bool {
    Channel::parse(s).is_some()
}

/// Entries of a sampled tone curve.
const LUT: usize = 256;

/// One adjustment, its parameters normalised.
#[derive(Clone, Debug, PartialEq)]
enum Adjust {
    /// Brightness and contrast, -1..1 each.
    BrightnessContrast { brightness: f32, contrast: f32 },
    /// A tone curve (Curves, Levels) sampled at [`LUT`] inputs, on `channel`.
    Tone { channel: Channel, lut: Arc<[f32]> },
    /// Hue shift in degrees, saturation and lightness -1..1; `colorize` sets the hue instead.
    HueSaturation { hue: f32, saturation: f32, lightness: f32, colorize: bool },
    /// Move colours `amount` (0..1) of the way to `target` (sRGB), keeping their lightness when
    /// `preserve`.
    ShiftToColor { target: [f32; 3], amount: f32, preserve: bool },
    /// Warm/cool and magenta/green shifts, -1..1.
    TemperatureTint { temperature: f32, tint: f32 },
}

/// The colour adjustments of an effect list, composed in order.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorMap(Vec<Adjust>);

impl ColorMap {
    /// `self`, then `next`.
    pub fn then(&self, next: &ColorMap) -> ColorMap {
        ColorMap(self.0.iter().chain(&next.0).cloned().collect())
    }

    /// Adjust display (sRGB) components 0..1.
    pub fn apply_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.0.iter().fold(rgb, |c, a| a.apply(c).map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }))
    }

    /// Adjust a colour, keeping its colour model (unchanged when the adjustments leave it as is).
    pub fn apply(&self, c: Color) -> Color {
        let rgb = c.to_rgb();
        let out = self.apply_rgb(rgb);
        if out.iter().zip(rgb).all(|(a, b)| (a - b).abs() < 1e-6) {
            return c;
        }
        keep_model(c, Color::rgb(out[0], out[1], out[2]))
    }

    /// Adjust an 8-bit sRGB pixel.
    pub fn apply_rgb8(&self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        let out = self.apply_rgb([r, g, b].map(|v| v as f32 / 255.0));
        out.map(|v| (v * 255.0).round() as u8)
    }
}

/// The visible colour adjustments of `effects` as one map, `None` when there are none.
pub fn color_map(effects: &[Effect]) -> Option<ColorMap> {
    let v: Vec<Adjust> =
        effects.iter().filter(|e| visible_adjustment(e)).filter_map(|e| Adjust::parse(&e.id, &merged_params(&e.id, &e.params))).collect();
    (!v.is_empty()).then_some(ColorMap(v))
}

/// A parameter in -100..100 as -1..1.
fn amount(p: &Value, key: &str) -> f32 {
    (num(p, key, 0.0).clamp(-100.0, 100.0) / 100.0) as f32
}

/// A level 0..255 as 0..1.
fn level(p: &Value, key: &str, default: f64) -> f32 {
    (num(p, key, default).clamp(0.0, 255.0) / 255.0) as f32
}

impl Adjust {
    fn parse(id: &str, p: &Value) -> Option<Self> {
        Some(match id {
            "adjust.brightnessContrast" => Adjust::BrightnessContrast { brightness: amount(p, "brightness"), contrast: amount(p, "contrast") },
            "adjust.levels" => {
                let (ib, iw) = (level(p, "inputBlack", 0.0), level(p, "inputWhite", 255.0));
                let (ob, ow) = (level(p, "outputBlack", 0.0), level(p, "outputWhite", 255.0));
                let gamma = num(p, "gamma", 1.0).clamp(0.1, 10.0) as f32;
                let f = |x: f32| {
                    let t = if iw > ib {
                        ((x - ib) / (iw - ib)).clamp(0.0, 1.0)
                    } else if x >= ib {
                        1.0
                    } else {
                        0.0
                    };
                    ob + t.powf(1.0 / gamma) * (ow - ob)
                };
                Adjust::Tone { channel: Channel::of(p), lut: sample(f) }
            }
            "adjust.curves" => {
                let pts = curve_points(p.get("points"));
                Adjust::Tone { channel: Channel::of(p), lut: sample(|x| curve_at(&pts, x)) }
            }
            "adjust.hueSaturation" => Adjust::HueSaturation {
                hue: num(p, "hue", 0.0).clamp(-360.0, 360.0) as f32,
                saturation: amount(p, "saturation"),
                lightness: amount(p, "lightness"),
                colorize: flag(p, "colorize", false),
            },
            "adjust.shiftToColor" => Adjust::ShiftToColor {
                target: color_param(p, Color::rgb(1.0, 0.5, 0.0)).to_rgb(),
                amount: (num(p, "amount", 50.0).clamp(0.0, 100.0) / 100.0) as f32,
                preserve: flag(p, "preserveLightness", true),
            },
            "adjust.temperatureTint" => Adjust::TemperatureTint { temperature: amount(p, "temperature"), tint: amount(p, "tint") },
            _ => return None,
        })
    }

    fn apply(&self, [r, g, b]: [f32; 3]) -> [f32; 3] {
        match self {
            Adjust::BrightnessContrast { brightness, contrast } => {
                // Brightness bends the tones (black and white stay), contrast spreads them from the middle.
                let gamma = 2f32.powf(-brightness);
                let k = if *contrast >= 0.0 { 1.0 / (1.0 - contrast * 0.99) } else { 1.0 + contrast };
                [r, g, b].map(|v| (v.clamp(0.0, 1.0).powf(gamma) - 0.5) * k + 0.5)
            }
            Adjust::Tone { channel, lut } => {
                let f = |v: f32| lookup(lut, v);
                match channel {
                    Channel::All => [f(r), f(g), f(b)],
                    Channel::One(i) => {
                        let mut c = [r, g, b];
                        if let Some(v) = c.get_mut(*i) {
                            *v = f(*v);
                        }
                        c
                    }
                }
            }
            Adjust::HueSaturation { hue, saturation, lightness, colorize } => {
                let [h, s, l] = to_hsl([r, g, b]);
                let (h, s) = if *colorize {
                    (hue.rem_euclid(360.0), (saturation + 1.0) / 2.0)
                } else {
                    let s = if *saturation >= 0.0 { s / (1.0 - saturation * 0.99) } else { s * (1.0 + saturation) };
                    ((h + hue).rem_euclid(360.0), s)
                };
                let l = if *lightness >= 0.0 { l + (1.0 - l) * lightness } else { l * (1.0 + lightness) };
                from_hsl([h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0)])
            }
            Adjust::ShiftToColor { target, amount, preserve } => {
                let to = if *preserve {
                    let [th, ts, _] = to_hsl(*target);
                    from_hsl([th, ts, to_hsl([r, g, b])[2]])
                } else {
                    *target
                };
                let mix = |a: f32, b: f32| a + (b - a) * amount;
                [mix(r, to[0]), mix(g, to[1]), mix(b, to[2])]
            }
            Adjust::TemperatureTint { temperature, tint } => {
                let (t, m) = (temperature * 0.3, tint * 0.3);
                [r * (1.0 + t), g * (1.0 - m), b * (1.0 - t)]
            }
        }
    }
}

/// `f` sampled at [`LUT`] inputs 0..1.
fn sample(f: impl Fn(f32) -> f32) -> Arc<[f32]> {
    (0..LUT).map(|i| f(i as f32 / (LUT - 1) as f32).clamp(0.0, 1.0)).collect()
}

/// The tone curve `lut` at `v` (linear between samples).
fn lookup(lut: &[f32], v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0) * (LUT - 1) as f32;
    let i = (x.floor() as usize).min(LUT - 2);
    let (a, b) = (lut.get(i).copied().unwrap_or(v), lut.get(i + 1).copied().unwrap_or(v));
    a + (b - a) * (x - i as f32)
}

/// Curves' points (0..1 each, sorted by input, one per input) from its `points` parameter:
/// `"x,y x,y …"` or `[[x, y], …]` in 0..255. Missing or unreadable points give the identity line.
pub fn curve_points(v: Option<&Value>) -> Vec<(f32, f32)> {
    let mut pts: Vec<(f64, f64)> = match v {
        Some(Value::String(s)) => s
            .split(|c: char| c.is_whitespace() || c == ';')
            .filter_map(|pair| {
                let (x, y) = pair.split_once(',')?;
                Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
            })
            .collect(),
        Some(Value::Array(a)) => a.iter().filter_map(|xy| Some((xy.get(0)?.as_f64()?, xy.get(1)?.as_f64()?))).collect(),
        _ => vec![],
    };
    pts.retain(|(x, y)| x.is_finite() && y.is_finite());
    // At most a point per level is meaningful.
    pts.truncate(256);
    let mut pts: Vec<(f32, f32)> =
        pts.into_iter().map(|(x, y)| ((x.clamp(0.0, 255.0) / 255.0) as f32, (y.clamp(0.0, 255.0) / 255.0) as f32)).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
    if pts.len() < 2 {
        return vec![(0.0, 0.0), (1.0, 1.0)];
    }
    pts
}

/// Curves' curve through `pts` ([`curve_points`]) at `x` (0..1): a monotone cubic, flat beyond
/// the end points.
pub fn curve_at(pts: &[(f32, f32)], x: f32) -> f32 {
    let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { return x };
    if x <= first.0 {
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    let n = pts.len();
    let slope = |i: usize| -> f32 {
        match (pts.get(i), pts.get(i + 1)) {
            (Some(a), Some(b)) if b.0 > a.0 => (b.1 - a.1) / (b.0 - a.0),
            _ => 0.0,
        }
    };
    // Tangents: the harmonic mean of the neighbouring slopes, zero at extrema, so the curve never
    // overshoots its points.
    let tangent = |i: usize| -> f32 {
        if i == 0 {
            return slope(0);
        }
        if i + 1 >= n {
            return slope(n - 2);
        }
        let (a, b) = (slope(i - 1), slope(i));
        if a * b <= 0.0 { 0.0 } else { 2.0 * a * b / (a + b) }
    };
    let k = pts.windows(2).position(|w| x >= w[0].0 && x <= w[1].0).unwrap_or(0);
    let (Some(&(x0, y0)), Some(&(x1, y1))) = (pts.get(k), pts.get(k + 1)) else { return x };
    let h = x1 - x0;
    if h <= 0.0 {
        return y0;
    }
    let t = (x - x0) / h;
    let (m0, m1) = (tangent(k) * h, tangent(k + 1) * h);
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * m1
}

/// HSL with hue in degrees, saturation and lightness 0..1.
fn to_hsl([r, g, b]: [f32; 3]) -> [f32; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= 1e-6 {
        return [0.0, 0.0, l];
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h * 60.0, s, l]
}

fn from_hsl([h, s, l]: [f32; 3]) -> [f32; 3] {
    if s <= 0.0 {
        return [l; 3];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let h = h / 360.0;
    [hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0)]
}

/// Gives the key of an embedded image recoloured by a map (`None`: leave it as it is).
pub type ImageHook<'a> = dyn FnMut(&str, &ColorMap) -> Option<String> + 'a;

/// The live visitor: every colour through `map`, embedded images through the hook.
struct Live<'a, 'b> {
    map: &'a ColorMap,
    images: &'a mut ImageHook<'b>,
}

impl ColorVisitor for Live<'_, '_> {
    fn paint(&mut self, p: &mut Paint) -> bool {
        map_paint(p, &|c| self.map.apply(c))
    }
    fn color(&mut self, c: Color) -> Color {
        self.map.apply(c)
    }
    fn image(&mut self, key: &str) -> Option<String> {
        (self.images)(key, self.map)
    }
}

/// `n` with the colour adjustments in it applied, `None` when it has none: its members' first,
/// then those of its fills and strokes on their paints, then its own followed by `outer` (the
/// adjustments of an object `n` is the art of) on everything it paints. The adjustment effects
/// are gone from the result; embedded images take the keys `images` gives.
pub fn adjust(n: &Node, outer: Option<&ColorMap>, images: &mut ImageHook<'_>) -> Option<Node> {
    let mut m: Option<Node> = None;
    if let Some(ch) = n.children() {
        for (i, c) in ch.iter().enumerate() {
            if let Some(new) = adjust(c, None, images)
                && let Some(slot) = m.get_or_insert_with(|| n.clone()).children_mut().and_then(|ch| ch.get_mut(i))
            {
                *slot = Arc::new(new);
            }
        }
    }
    for (i, item) in n.appearance.items.iter().enumerate() {
        if let Some(map) = color_map(item.effects())
            && let Some(it) = m.get_or_insert_with(|| n.clone()).appearance.items.get_mut(i)
        {
            map_paint(it.paint_mut(), &|c| map.apply(c));
            it.effects_mut().retain(|e| !is_adjustment(&e.id));
        }
    }
    let map = match (color_map(&n.appearance.effects), outer) {
        (Some(own), Some(outer)) => Some(own.then(outer)),
        (own, None) => own,
        (None, Some(outer)) => Some(outer.clone()),
    };
    if let Some(map) = map {
        let mm = m.get_or_insert_with(|| n.clone());
        mm.appearance.effects.retain(|e| !is_adjustment(&e.id));
        recolor_node(mm, Reach::ALL, &mut Live { map: &map, images });
    }
    m
}

/// [`adjust`] for export and Expand Appearance: recoloured embedded images become new images in
/// `d` (named by their content), and a symbol instance stands for its symbol's art.
pub fn adjust_in_document(d: &mut Document, n: &Node) -> Option<Node> {
    let art;
    let src = match &n.kind {
        NodeKind::SymbolInstance { symbol, .. } if has_adjustment(n) => {
            let symbol = d.symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone());
            art = crate::reshape::as_art(n, symbol.as_deref())?;
            &art
        }
        _ => n,
    };
    let mut hook = |key: &str, map: &ColorMap| -> Option<String> {
        let blob = d.images.get(key)?.map_rgb(|rgb| map.apply_rgb8(rgb))?;
        let k = blob.content_key();
        d.images.entry(k.clone()).or_insert(blob);
        Some(k)
    };
    adjust(src, None, &mut hook)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_doc::{Appearance, NodeId};
    use vectorcraft_geom::{Rect, shapes};

    fn map(id: &str, params: Value) -> ColorMap {
        color_map(&[Effect { id: id.into(), params, visible: true }]).unwrap()
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
    }

    #[test]
    fn neutral_settings_change_nothing() {
        for id in ADJUSTMENTS {
            let m = color_map(&[Effect { id: id.into(), params: json!({"amount": 0}), visible: true }]).unwrap();
            for c in [[0.2, 0.4, 0.6], [1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] {
                assert!(close(m.apply_rgb(c), c), "{id} {c:?} → {:?}", m.apply_rgb(c));
            }
        }
        let cmyk = Color::cmyk(0.1, 0.2, 0.3, 0.4);
        assert_eq!(map("adjust.levels", json!({})).apply(cmyk), cmyk, "an untouched colour keeps its exact values");
    }

    #[test]
    fn adjustments_move_colours_the_expected_way() {
        let grey = [0.5, 0.5, 0.5];
        assert!(map("adjust.brightnessContrast", json!({"brightness": 50})).apply_rgb(grey)[0] > 0.6);
        assert!(map("adjust.brightnessContrast", json!({"contrast": 50})).apply_rgb([0.7; 3])[0] > 0.8);
        assert!(close(map("adjust.levels", json!({"inputBlack": 64, "inputWhite": 192})).apply_rgb([0.25, 0.5, 0.75]), [0.0, 0.5, 1.0]));
        assert!(close(map("adjust.levels", json!({"outputWhite": 128, "channel": "red"})).apply_rgb([1.0; 3]), [0.502, 1.0, 1.0]));
        // An S curve darkens the shadows and lightens the highlights, through its points exactly.
        let s = map("adjust.curves", json!({"points": "0,0 64,32 192,224 255,255"}));
        assert!(close(s.apply_rgb([64.0 / 255.0; 3]), [32.0 / 255.0; 3]));
        assert!(s.apply_rgb([0.85; 3])[0] > 0.9);
        let arr = map("adjust.curves", json!({"points": [[0, 255], [255, 0]]}));
        assert!(close(arr.apply_rgb([0.0, 1.0, 0.25]), [1.0, 0.0, 0.75]), "an inverting curve");
        assert!(close(map("adjust.hueSaturation", json!({"hue": 120})).apply_rgb([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
        assert!(close(map("adjust.hueSaturation", json!({"saturation": -100})).apply_rgb([1.0, 0.0, 0.0]), [0.5; 3]));
        assert!(close(map("adjust.hueSaturation", json!({"saturation": 100})).apply_rgb(grey), grey), "greys stay grey");
        assert!(close(map("adjust.hueSaturation", json!({"colorize": true, "hue": 240, "saturation": 100})).apply_rgb(grey), [0.0, 0.0, 1.0]));
        let shift = map("adjust.shiftToColor", json!({"color": "#0000ff", "amount": 100, "preserveLightness": false}));
        assert!(close(shift.apply_rgb([1.0, 1.0, 0.0]), [0.0, 0.0, 1.0]));
        let warm = map("adjust.temperatureTint", json!({"temperature": 100})).apply_rgb(grey);
        assert!(warm[0] > grey[0] && warm[2] < grey[2]);
        // Effects compose in stack order.
        let both = map("adjust.levels", json!({"outputWhite": 128})).then(&map("adjust.brightnessContrast", json!({"brightness": 100})));
        assert!(close(both.apply_rgb([1.0; 3]), [(128.0f32 / 255.0).sqrt(); 3]));
    }

    #[test]
    fn adjust_recolours_paints_members_and_strips_the_effects() {
        let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
        let leaf = |id| Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), red.clone()));
        let mut g = Node::group(NodeId(1), vec![leaf(2), leaf(3)]);
        g.appearance.effects.push(Effect { id: "adjust.hueSaturation".into(), params: json!({"hue": 120}), visible: true });
        assert!(has_adjustment(&g));
        let out = adjust(&g, None, &mut |_, _| None).unwrap();
        assert!(out.appearance.effects.is_empty());
        for c in out.children().unwrap() {
            assert_eq!(c.appearance.fill_paint().color().unwrap().to_hex(), "#00ff00");
        }
        // A hidden effect does nothing; a stroke's own effect recolours just that stroke.
        g.appearance.effects[0].visible = false;
        assert!(!has_adjustment(&g) && adjust(&g, None, &mut |_, _| None).is_none());
        let mut p = Node::path(NodeId(5), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        p.appearance.items[1].effects_mut().push(Effect { id: "adjust.levels".into(), params: json!({"outputBlack": 255}), visible: true });
        let out = adjust(&p, None, &mut |_, _| None).unwrap();
        assert_eq!(out.appearance.items[1].paint().color().unwrap().to_hex(), "#ffffff");
        assert_eq!(out.appearance.items[0].paint(), p.appearance.items[0].paint());
        assert!(out.appearance.items[1].effects().is_empty());
    }

    #[test]
    fn adjust_recolours_the_points_of_a_freeform_gradient() {
        use vectorcraft_doc::color::{Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint};
        let mut gp = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Gradient::default() });
        let pt = |x, c| FreeformPoint::new(kurbo::Point::new(x, 5.0), c);
        gp.set_freeform(Freeform { points: vec![pt(2.0, Color::rgb(1.0, 0.0, 0.0)), pt(8.0, Color::rgb(0.0, 0.0, 1.0))], ..Default::default() });
        let fill = Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0);
        let mut p = Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), fill);
        p.appearance.effects.push(Effect { id: "adjust.hueSaturation".into(), params: json!({"hue": 120}), visible: true });
        let out = adjust(&p, None, &mut |_, _| None).unwrap();
        let Paint::Gradient(g) = out.appearance.fill_paint() else { panic!("a gradient fill") };
        // The points are what a freeform gradient draws from, not its stops.
        let hexes: Vec<String> = g.freeform.as_ref().unwrap().points.iter().map(|p| p.color.to_hex()).collect();
        assert_eq!(hexes, ["#00ff00", "#ff0000"]);
    }
}
