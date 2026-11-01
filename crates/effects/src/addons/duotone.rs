//! Color Effects' Duotone (Effect › Color Effects › Duotone…), a colour adjustment: the
//! grey-equivalent tone of each colour an object paints is printed with one to four inks (a
//! monotone, duotone, tritone or quadtone). Each ink, a tone, is a colour, typically a spot swatch,
//! with its own curve from the tone's tint to the ink's. The inks overprint, so their tints
//! multiply.
//!
//! With one tone of a swatch, a colour becomes a tint of that swatch, linked to it, so a spot tone
//! separates onto its plate. With several tones it becomes the CMYK mix of their tints: a colour
//! links to one swatch at one tint, and a mix of several spot inks has no swatch to link to.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_doc::color::{Color, cms::Model};

use crate::adjust::{LUT, curve_at, curve_points_on, lookup, sample};
use crate::raster::color_param;
use crate::util::flag;

/// Duotone (Effect › Color Effects › Duotone…).
pub const DUOTONE: &str = "addon.color.duotone";

/// Most tones Duotone reads (a quadtone): any past them are left out.
pub const MAX_TONES: usize = 4;

/// Most points a tone's curve reads: any past them are left out.
pub const MAX_CURVE_POINTS: usize = 64;

/// The parameters' description (`effect.apply`).
pub(crate) const PARAMS: &str = "{tones: [{color: \"#rrggbb\" or a colour object, swatch?: a global or spot swatch's name (with the swatch's colour as `color`), curve?: \"tint,ink tint,ink …\" or [[tint, ink], …] (0..100, at most 64 points; \"0,0 100,100\"), active?: true}] (up to four inks, overprinting; any past four are left out), skipSpot?: false (colours linked to a spot swatch keep their colour)} prints the grey-equivalent tone of each colour the object paints with the inks, each at the tint its curve gives for the tone's: with one tone of a swatch, as a tint of that swatch (a spot tone prints on its own plate); with several, as the CMYK mix of their tints";

/// The dialog's defaults: one black tone, its curve linear.
pub(crate) fn defaults() -> Value {
    json!({"tones": [{"color": "#000000", "curve": "0,0 100,100", "active": true}], "skipSpot": false})
}

/// Whether swatch `name` of the document is a spot colour: `Some(false)` for a global process
/// swatch, `None` when the document has no global swatch of that name.
pub type SwatchKinds<'a> = dyn Fn(&str) -> Option<bool> + 'a;

/// [`SwatchKinds`] of `d`'s swatch `name`.
pub fn swatch_kind(d: &Document, name: &str) -> Option<bool> {
    d.swatches_iter().find(|w| w.global && w.name == name).map(|w| w.spot)
}

/// [`swatch_kind`] of each of `d`'s global swatches.
pub fn swatch_kinds(d: &Document) -> HashMap<String, bool> {
    d.swatches_iter().filter(|w| w.global).map(|w| (w.name.clone(), w.spot)).collect()
}

/// A tone's curve points (0..1 each) from its `curve` (0..100).
pub fn tone_curve_points(v: Option<&Value>) -> Vec<(f32, f32)> {
    curve_points_on(v, 100.0, MAX_CURVE_POINTS)
}

/// One ink.
#[derive(Clone, Debug, PartialEq)]
struct Tone {
    color: Color,
    /// `color` as CMYK inks, which several tones mix in.
    cmyk: [f32; 4],
    /// The swatch the colour is of, if any.
    swatch: Option<String>,
    /// Its curve, sampled: the tone's tint → the ink's.
    curve: Arc<[f32]>,
}

/// Duotone's tones and options, its parameters read.
#[derive(Clone, Debug, PartialEq)]
pub struct Duotone {
    tones: Vec<Tone>,
    skip_spot: bool,
    /// The display colour at each sampled tint, for pixels.
    rgb: Arc<[[f32; 3]]>,
}

impl Duotone {
    /// `None` without an active tone (the effect then leaves the colours as they are).
    pub(crate) fn parse(p: &Value) -> Option<Self> {
        let tones: Vec<Tone> = p
            .get("tones")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_TONES)
            .filter(|t| t.is_object() && flag(t, "active", true))
            .map(|t| {
                let color = finite(color_param(t, Color::BLACK));
                let cmyk = match color.in_model(Model::Cmyk) {
                    Color::Cmyk { c, m, y, k } => [c, m, y, k].map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }),
                    _ => [0.0, 0.0, 0.0, 1.0],
                };
                let pts = tone_curve_points(t.get("curve"));
                let swatch = t.get("swatch").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
                Tone { color, cmyk, swatch, curve: sample(|x| curve_at(&pts, x)) }
            })
            .collect();
        if tones.is_empty() {
            return None;
        }
        let mut d = Duotone { tones, skip_spot: flag(p, "skipSpot", false), rgb: Arc::new([]) };
        d.rgb = (0..LUT).map(|i| d.mix(i as f32 / (LUT - 1) as f32).to_rgb()).collect();
        Some(d)
    }

    /// The inks' colour for tone `t` (0 = white, 1 = black): one tone's colour at its tint, or
    /// several tones' CMYK tints overprinted (each ink lets through what the others leave).
    fn mix(&self, t: f32) -> Color {
        if let [one] = self.tones.as_slice() {
            return one.color.tinted(lookup(&one.curve, t));
        }
        let mut paper = [1.0f32; 4];
        for tone in &self.tones {
            let k = lookup(&tone.curve, t);
            for (p, ink) in paper.iter_mut().zip(tone.cmyk) {
                *p *= 1.0 - ink * k;
            }
        }
        let [c, m, y, k] = paper.map(|p| 1.0 - p);
        Color::cmyk(c, m, y, k)
    }

    /// Display components 0..1 through the effect.
    pub(crate) fn apply_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        let x = tone(rgb) * (LUT - 1) as f32;
        let i = (x.floor() as usize).min(LUT - 2);
        let f = x - i as f32;
        match (self.rgb.get(i), self.rgb.get(i + 1)) {
            (Some(a), Some(b)) => std::array::from_fn(|k| a[k] + (b[k] - a[k]) * f),
            _ => rgb,
        }
    }

    /// A colour with its swatch link and tint through the effect ([`crate::ColorMap::apply_link`]):
    /// true when it changed. With `skipSpot`, a colour linked to a spot swatch stays.
    pub(crate) fn map_link(&self, c: &mut Color, link: &mut Option<String>, tint: &mut f32, swatches: &SwatchKinds) -> bool {
        if self.skip_spot && link.as_deref().and_then(swatches) == Some(true) {
            return false;
        }
        let t = tone(c.to_rgb());
        let new = self.mix(t);
        let (new_link, new_tint) = match self.tones.as_slice() {
            [one] => match one.swatch.as_ref().filter(|n| swatches(n).is_some()) {
                Some(name) => (Some(name.clone()), lookup(&one.curve, t)),
                None => (None, 1.0),
            },
            _ => (None, 1.0),
        };
        if new == *c && new_link == *link && new_tint == *tint {
            return false;
        }
        (*c, *link, *tint) = (new, new_link, new_tint);
        true
    }
}

/// The grey-equivalent tone of display colour `rgb`: 0 for white, 1 for black (its luma, as
/// Convert to Grayscale reads it, so a grey's tone is its ink).
fn tone([r, g, b]: [f32; 3]) -> f32 {
    let t = 1.0 - (0.299 * r + 0.587 * g + 0.114 * b);
    if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
}

/// `c` with its components finite and in range.
fn finite(c: Color) -> Color {
    let v = |x: f32, lo: f32, hi: f32| if x.is_finite() { x.clamp(lo, hi) } else { lo };
    match c {
        Color::Rgb { r, g, b } => Color::rgb(v(r, 0.0, 1.0), v(g, 0.0, 1.0), v(b, 0.0, 1.0)),
        Color::Cmyk { c, m, y, k } => Color::cmyk(v(c, 0.0, 1.0), v(m, 0.0, 1.0), v(y, 0.0, 1.0), v(k, 0.0, 1.0)),
        Color::Gray { k } => Color::gray(v(k, 0.0, 1.0)),
        Color::Lab { l, a, b } => Color::lab(v(l, 0.0, 100.0), v(a, -128.0, 127.0), v(b, -128.0, 127.0)),
    }
}

#[cfg(test)]
mod tests;
