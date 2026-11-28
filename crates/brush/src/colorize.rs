//! Brush colorization (None / Tints / Tints and Shades / Hue Shift) and symbol staining.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};

use crate::Colorization;

fn luminance(c: &Color) -> f32 {
    let [r, g, b] = c.to_rgb();
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// How light `c` is for colorization, 0 black to 1 white. Black ink alone is its own tint: its
/// amount is the tint, not how light it displays (100% K displays lighter than black).
fn lightness(c: &Color) -> f32 {
    match *c {
        Color::Cmyk { c, m, y, k } if c == 0.0 && m == 0.0 && y == 0.0 => 1.0 - k,
        _ => luminance(c),
    }
}

fn map_color(c: &Color, mode: Colorization, key: &Color) -> Color {
    let light = lightness(c);
    // A process or gray stroke color's tints and shades stay in its model.
    let (white, black) = match key {
        Color::Cmyk { .. } => (Color::cmyk(0.0, 0.0, 0.0, 0.0), Color::cmyk(0.0, 0.0, 0.0, 1.0)),
        Color::Gray { .. } => (Color::gray(0.0), Color::gray(1.0)),
        _ => (Color::WHITE, Color::BLACK),
    };
    match mode {
        Colorization::None => *c,
        Colorization::Tints => key.lerp_keep_model(&white, light),
        Colorization::TintsAndShades => {
            if light < 0.5 {
                key.lerp_keep_model(&black, 1.0 - light * 2.0)
            } else {
                key.lerp_keep_model(&white, (light - 0.5) * 2.0)
            }
        }
        Colorization::HueShift { key: art_key } => {
            let [kh, _, _] = art_key.to_hsb();
            let [th, _, _] = key.to_hsb();
            let [h, s, v] = c.to_hsb();
            if (c.to_rgb().iter().zip(art_key.to_rgb()).map(|(a, b)| (a - b).abs()).sum::<f32>()) < 0.02 {
                return *key;
            }
            Color::from_hsb((h + th - kh).rem_euclid(360.0), s, v)
        }
    }
}

/// The swatch a stroke paint links to and its tint, for colorization: Tints of a linked color are
/// tints of its swatch (a spot color's tints print on its own plate).
type Link<'a> = Option<(&'a str, f32)>;

fn map_paint(p: &mut Paint, mode: Colorization, key: &Color, link: Link<'_>) {
    if let Paint::Solid { color, .. } = p {
        *p = match (mode, link) {
            (Colorization::Tints, Some((swatch, tint))) => {
                let t = (1.0 - lightness(color)).clamp(0.0, 1.0);
                Paint::Solid { color: key.tinted(t), swatch: Some(swatch.to_string()), tint: tint * t }
            }
            _ => Paint::solid(map_color(color, mode, key)),
        };
    }
}

/// Recolour `n` (recursively) for a stroke painted `stroke`, and strip brushes from its strokes so
/// brush output never recurses. Non-solid stroke paints leave the art's colours alone. Under Tints,
/// a stroke color linked to a swatch gives tints of that swatch, linked to it, as a spot color's
/// tints are (the art's darkest parts the stroke's own tint, white none of it). Type is
/// recolored like paths: its characters' solid fills and strokes as well as its own.
pub fn colorize(n: &mut Node, mode: Colorization, stroke: &Paint) {
    let key = match (mode, stroke.color()) {
        (Colorization::None, _) | (_, None) => None,
        (_, Some(c)) => Some(c),
    };
    let link = match stroke {
        Paint::Solid { swatch: Some(s), tint, .. } => Some((s.as_str(), *tint)),
        _ => None,
    };
    fn walk(n: &mut Node, mode: Colorization, key: Option<Color>, link: Link<'_>) {
        for it in &mut n.appearance.items {
            match it {
                AppearanceItem::Fill(f) => {
                    if let Some(k) = &key {
                        map_paint(&mut f.paint, mode, k, link);
                    }
                }
                AppearanceItem::Stroke(s) => {
                    s.brush = None;
                    if let Some(k) = &key {
                        map_paint(&mut s.paint, mode, k, link);
                    }
                }
            }
        }
        if let (Some(k), NodeKind::Text(t)) = (&key, &mut n.kind) {
            for r in &mut t.runs {
                map_paint(&mut r.style.fill, mode, k, link);
                map_paint(&mut r.style.stroke, mode, k, link);
            }
        }
        if let Some(ch) = n.children_mut() {
            for c in ch.iter_mut() {
                walk(Arc::make_mut(c), mode, key, link);
            }
        }
        // A mask's art is drawn too: a brush on its strokes would run this brush again. Its colours
        // are the mask's values, so they stay.
        if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
            strip_brushes(Arc::make_mut(&mut m.art));
        }
    }
    walk(n, mode, key, link);
}

/// Mixed into a stroke's seed for its hue jitter, so the amounts come from a sequence of their own
/// and a copy's size, spacing, scatter and rotation stay as they were without hue jitter.
const HUE_JITTER_SALT: u64 = 0x6875_655f_6a69_7474;

/// Mixed into a stroke's seed for its saturation jitter, so its amounts do not follow the hue
/// jitter's and the hue jitter's amounts stay as they were without it.
const SATURATION_JITTER_SALT: u64 = 0x7361_745f_6a69_7474;

/// Mixed into a stroke's seed for its brightness jitter, so its amounts follow neither the hue
/// jitter's nor the saturation jitter's, and theirs stay as they were without it.
const BRIGHTNESS_JITTER_SALT: u64 = 0x6272_695f_6a69_7474;

/// Recolor each of `copies` (a scatter brush's copies, in order along the path) with `adjust`,
/// given that copy's own amount between −`amount` and `amount`, drawn from `seed` (the stroke's
/// [`crate::track::seed_of`]) mixed with `salt`. `adjust` gives `None` for a color it leaves as it
/// is.
fn jitter_copies(copies: &mut [Node], amount: f64, seed: u64, salt: u64, adjust: fn(&Color, f32) -> Option<Color>) {
    let mut rng = crate::track::Rng::new(seed ^ salt);
    for n in copies {
        let by = rng.range(-amount, amount) as f32;
        recolor_solids(n, &|c| adjust(c, by));
    }
}

/// Turn the hue of each of `copies` by its own amount between −`amount` and `amount` degrees (see
/// [`jitter_copies`]).
pub(crate) fn jitter_hues(copies: &mut [Node], amount: f64, seed: u64) {
    jitter_copies(copies, amount, seed, HUE_JITTER_SALT, turned);
}

/// Move the saturation of each of `copies` by its own amount between −`amount` and `amount`
/// percentage points (see [`jitter_copies`]).
pub(crate) fn jitter_saturations(copies: &mut [Node], amount: f64, seed: u64) {
    jitter_copies(copies, amount, seed, SATURATION_JITTER_SALT, saturated);
}

/// Move the brightness of each of `copies` by its own amount between −`amount` and `amount`
/// percentage points (see [`jitter_copies`]).
pub(crate) fn jitter_brightnesses(copies: &mut [Node], amount: f64, seed: u64) {
    jitter_copies(copies, amount, seed, BRIGHTNESS_JITTER_SALT, brightened);
}

/// `c` with its brightness moved by `points` percentage points, worked out on its own components
/// so its hue and saturation stay: RGB in HSB of its values, Gray by taking that much ink off or
/// adding it, CMYK by changing its K over the brightness its C, M and Y leave (C, M and Y change
/// only to go brighter than they show with no K), Lab by moving its lightness by `points` units
/// with a and b kept. The result stays between black and full brightness. White, black and grays
/// move too. `None` when the brightness stays as it is.
fn brightened(c: &Color, points: f32) -> Option<Color> {
    if !points.is_finite() {
        return None;
    }
    let by = points / 100.0;
    match *c {
        Color::Rgb { r, g, b } => {
            let [h, s, v] = vectorcraft_color::rgb_to_hsb([r, g, b]);
            let to = (v + by).clamp(0.0, 1.0);
            (to != v).then(|| {
                let [r, g, b] = vectorcraft_color::hsb_to_rgb([h, s, to]);
                Color::rgb(r, g, b)
            })
        }
        Color::Gray { k } => {
            let to = (k - by).clamp(0.0, 1.0);
            (to != k).then_some(Color::gray(to))
        }
        Color::Cmyk { c, m, y, k } => {
            // `v` is the brightness C, M and Y leave; with its K the color shows `v` × (1 − K).
            let [h, s, v] = vectorcraft_color::rgb_to_hsb([1.0 - c, 1.0 - m, 1.0 - y]);
            let shown = v * (1.0 - k);
            let to = (shown + by).clamp(0.0, 1.0);
            if to == shown {
                None
            } else if to <= v {
                // v > 0 here: to < shown would need shown > 0, and to > shown leaves to > 0.
                Some(Color::cmyk(c, m, y, (1.0 - to / v).clamp(0.0, 1.0)))
            } else {
                let [r, g, b] = vectorcraft_color::hsb_to_rgb([h, s, to]);
                Some(Color::cmyk(1.0 - r, 1.0 - g, 1.0 - b, 0.0))
            }
        }
        Color::Lab { l, a, b } => {
            let to = (l + points).clamp(0.0, 100.0);
            (to != l).then_some(Color::lab(to, a, b))
        }
    }
}

/// The largest a or b a Lab color takes.
const LAB_AB_MAX: f64 = 127.0;

/// `c` with its saturation moved by `points` percentage points, worked out on its own components
/// so its hue and brightness stay: RGB in HSB of its values, CMYK over C, M and Y with K kept, Lab
/// by moving its chroma (the length of a and b) by `points` with its lightness and hue angle kept.
/// The result stays between no saturation and full. `None` for a color without a hue (white,
/// black and grays, which have no saturation to move) and for one whose saturation stays as it is
/// (already at the end it would move past), so such a color keeps its swatch link.
fn saturated(c: &Color, points: f32) -> Option<Color> {
    if !points.is_finite() || vectorcraft_color::recolor::Neutral::of(c).is_some() {
        return None;
    }
    let by = points / 100.0;
    let move_by = |v: [f32; 3]| {
        let [h, s, b] = vectorcraft_color::rgb_to_hsb(v);
        let to = (s + by).clamp(0.0, 1.0);
        (s > 0.0 && to != s).then(|| vectorcraft_color::hsb_to_rgb([h, to, b]))
    };
    match *c {
        Color::Rgb { r, g, b } => move_by([r, g, b]).map(|[r, g, b]| Color::rgb(r, g, b)),
        Color::Cmyk { c, m, y, k } => move_by([1.0 - c, 1.0 - m, 1.0 - y]).map(|[r, g, b]| Color::cmyk(1.0 - r, 1.0 - g, 1.0 - b, k)),
        Color::Gray { .. } => None,
        Color::Lab { l, a, b } => {
            let (a, b) = (f64::from(a), f64::from(b));
            let chroma = a.hypot(b);
            if chroma <= 0.0 {
                return None;
            }
            // A longer chroma is cut back along the same hue angle until a and b fit; one that
            // already reaches past that does not grow.
            let fit = (LAB_AB_MAX / a.abs().max(b.abs())).max(1.0);
            let scale = ((chroma + f64::from(points)).max(0.0) / chroma).min(fit);
            (scale != 1.0).then(|| Color::lab(l, (a * scale) as f32, (b * scale) as f32))
        }
    }
}

/// `c` with its hue turned by `degrees`, worked out on its own components so nothing else about it
/// changes: RGB in HSB of its values, CMYK over C, M and Y with K kept (as
/// [`Color::complement_keep_model`] does), Lab by turning a and b. `None` for a color without a hue:
/// white, black and grays, in their own model (Gray, CMYK with no C, M or Y ink).
fn turned(c: &Color, degrees: f32) -> Option<Color> {
    if !degrees.is_finite() || vectorcraft_color::recolor::Neutral::of(c).is_some() {
        return None;
    }
    let turn = |v: [f32; 3]| {
        let [h, s, b] = vectorcraft_color::rgb_to_hsb(v);
        (s > 0.0).then(|| vectorcraft_color::hsb_to_rgb([(h + degrees).rem_euclid(360.0), s, b]))
    };
    match *c {
        Color::Rgb { r, g, b } => turn([r, g, b]).map(|[r, g, b]| Color::rgb(r, g, b)),
        Color::Cmyk { c, m, y, k } => turn([1.0 - c, 1.0 - m, 1.0 - y]).map(|[r, g, b]| Color::cmyk(1.0 - r, 1.0 - g, 1.0 - b, k)),
        Color::Gray { .. } => None,
        Color::Lab { l, a, b } => {
            let (sin, cos) = f64::from(degrees).to_radians().sin_cos();
            let (a, b) = (f64::from(a), f64::from(b));
            Some(Color::lab(l, (a * cos - b * sin) as f32, (a * sin + b * cos) as f32))
        }
    }
}

/// Replace a solid paint's color with what `adjust` gives for it, dropping its swatch link.
fn recolor_paint(p: &mut Paint, adjust: &dyn Fn(&Color) -> Option<Color>) {
    if let Paint::Solid { color, .. } = p
        && let Some(c) = adjust(color)
    {
        *p = Paint::solid(c);
    }
}

/// Recolor the solid fills and strokes under `n` with `adjust`, its type's characters included. A
/// mask's colors are its values, so they stay.
fn recolor_solids(n: &mut Node, adjust: &dyn Fn(&Color) -> Option<Color>) {
    for it in &mut n.appearance.items {
        match it {
            AppearanceItem::Fill(f) => recolor_paint(&mut f.paint, adjust),
            AppearanceItem::Stroke(s) => recolor_paint(&mut s.paint, adjust),
        }
    }
    if let NodeKind::Text(t) = &mut n.kind {
        for r in &mut t.runs {
            recolor_paint(&mut r.style.fill, adjust);
            recolor_paint(&mut r.style.stroke, adjust);
        }
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            recolor_solids(Arc::make_mut(c), adjust);
        }
    }
}

/// Strip the brushes from every stroke under `n`, its masks' art included.
fn strip_brushes(n: &mut Node) {
    for it in &mut n.appearance.items {
        if let AppearanceItem::Stroke(s) = it {
            s.brush = None;
        }
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            strip_brushes(Arc::make_mut(c));
        }
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        strip_brushes(Arc::make_mut(&mut m.art));
    }
}

/// The Symbol Stainer's mix, an instance's stain and its stained art, from
/// [`vectorcraft_doc::symbol_overrides`], which the effects that reshape an instance also read.
pub use vectorcraft_doc::symbol_overrides::{instance_art, stain, tint_node};

/// The solid colours of the fills and strokes under `nodes`, most used first (then in the order
/// they first appear): the colours Brush Options offers as Hue Shift's key colour, the first being
/// its default.
/// Type's characters count as well: each run's fill, and its stroke when the stroke is wider
/// than 0.
pub fn art_colors<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> Vec<Color> {
    fn walk(n: &Node, seen: &mut std::collections::HashMap<String, (usize, usize, Color)>) {
        for it in &n.appearance.items {
            let (visible, paint) = match it {
                AppearanceItem::Fill(f) => (f.visible, &f.paint),
                AppearanceItem::Stroke(s) => (s.visible, &s.paint),
            };
            if let (true, Some(c)) = (visible, paint.color()) {
                let order = seen.len();
                seen.entry(c.to_hex()).or_insert((0, order, c)).0 += 1;
            }
        }
        if let NodeKind::Text(t) = &n.kind {
            for r in &t.runs {
                let stroke = (r.style.stroke_width > 0.0).then_some(&r.style.stroke);
                for c in [Some(&r.style.fill), stroke].into_iter().flatten().filter_map(Paint::color) {
                    let order = seen.len();
                    seen.entry(c.to_hex()).or_insert((0, order, c)).0 += 1;
                }
            }
        }
        for c in n.children().into_iter().flatten() {
            walk(c, seen);
        }
    }
    let mut seen = std::collections::HashMap::new();
    for n in nodes {
        walk(n, &mut seen);
    }
    let mut v: Vec<(usize, usize, Color)> = seen.into_values().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().map(|(.., c)| c).collect()
}
