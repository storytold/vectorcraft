//! A key of a gradient for the gradient definitions the legacy `.ai` writer
//! ([`crate::legacy_ai`]) and the editing data writer ([`crate::ai_editing`]) look up: the writers
//! keep each definition's place under its gradient's key and compare only the gradients that
//! share it.

use std::hash::{Hash, Hasher};

use vectorcraft_color::{Color, Gradient};

/// A key of `g`: the same gradients have the same key (each number by its bits, with -0 as 0);
/// different gradients may share one.
pub(crate) fn gradient_key(g: &Gradient) -> u64 {
    let bits = |x: f32| if x == 0.0 { 0 } else { x.to_bits() };
    let mut h = std::hash::DefaultHasher::new();
    std::mem::discriminant(&g.kind).hash(&mut h);
    std::mem::discriminant(&g.method).hash(&mut h);
    (g.dither, g.spread, g.repeats, g.stops.len()).hash(&mut h);
    for s in &g.stops {
        let (model, c) = match s.color {
            Color::Rgb { r, g, b } => (0u8, [r, g, b, 0.0]),
            Color::Cmyk { c, m, y, k } => (1, [c, m, y, k]),
            Color::Gray { k } => (2, [k, 0.0, 0.0, 0.0]),
            Color::Lab { l, a, b } => (3, [l, a, b, 0.0]),
        };
        (model, c.map(bits), [s.offset, s.opacity, s.midpoint, s.tint].map(bits)).hash(&mut h);
        (&s.swatch, s.plates.map(|p| p.map(bits))).hash(&mut h);
    }
    h.finish()
}
