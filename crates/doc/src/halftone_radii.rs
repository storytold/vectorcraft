//! Color Halftone's Max Radius was a distance in points until documents of version 2; since then
//! it is in pixels of the Document Raster Effects resolution, as public documentation gives it
//! (#3260). [`Document::migrate_halftone_radii`] converts the radii of older documents.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::limits::FORMAT_VERSION;
use crate::{Appearance, Document, Effect, Node};

/// The first document version whose Color Halftone radii are in pixels.
const PIXEL_RADII_SINCE: u32 = 2;
const HALFTONE: &str = "pixelate.colorHalftone";
const RADIUS: &str = "maxRadius";
/// The radius (points) an older document's halftone without one drew with.
const OLD_DEFAULT: f64 = 8.0;
/// The range the radius was kept within in points, and is kept within in pixels.
const RANGE: std::ops::RangeInclusive<f64> = 4.0..=127.0;

impl Document {
    /// Saved before version 2: each Color Halftone's Max Radius in points becomes the pixels of
    /// the document's raster effects resolution that draw the same dots, within the 4 to 127 px
    /// the effect takes (a radius over 127 px, from a large radius at a high resolution, draws
    /// smaller dots than before). In the art, symbols, patterns, graph designs, opacity masks and
    /// graphic styles.
    pub fn migrate_halftone_radii(&mut self) {
        if self.version >= PIXEL_RADII_SINCE {
            return;
        }
        self.version = FORMAT_VERSION;
        let k = self.raster_ppi() / 72.0;
        let fix = &mut |e: &mut Effect| to_pixels(e, k);
        for l in &mut self.layers {
            node(l, fix);
        }
        for s in &mut self.symbols {
            node(&mut s.art, fix);
        }
        for p in &mut self.patterns {
            for a in &mut p.art {
                node(a, fix);
            }
        }
        for g in &mut self.graph_designs {
            node(&mut g.art, fix);
        }
        if self.graphic_styles.iter().any(|s| has_halftone(&s.appearance)) {
            for s in self.graphic_styles.iter_mut() {
                appearance(&mut s.appearance, fix);
            }
        }
    }
}

/// `e`'s radius in points as pixels, `k` pixels a point, when it is a Color Halftone.
fn to_pixels(e: &mut Effect, k: f64) {
    if e.id != HALFTONE {
        return;
    }
    let pt = e.params.get(RADIUS).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(OLD_DEFAULT);
    let px = pt.clamp(*RANGE.start(), *RANGE.end()) * k;
    if !e.params.is_object() {
        e.params = Value::Object(Default::default());
    }
    e.params[RADIUS] = json!(px.clamp(*RANGE.start(), *RANGE.end()));
}

fn has_halftone(a: &Appearance) -> bool {
    a.effects.iter().chain(a.items.iter().flat_map(|i| i.effects())).any(|e| e.id == HALFTONE)
}

fn appearance(a: &mut Appearance, fix: &mut dyn FnMut(&mut Effect)) {
    for e in a.effects.iter_mut().chain(a.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut())) {
        fix(e);
    }
}

/// Fix the effects of `n`'s subtree and its opacity masks (copying only what changes).
fn node(n: &mut Arc<Node>, fix: &mut dyn FnMut(&mut Effect)) {
    let n = Arc::make_mut(n);
    if has_halftone(&n.appearance) {
        appearance(&mut n.appearance, fix);
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        node(&mut m.art, fix);
    }
    if let Some(ch) = n.children_mut() {
        for c in ch {
            node(c, fix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn halftone(radius: Option<f64>) -> Effect {
        let params = radius.map_or_else(|| json!({"channel1": 108.0}), |r| json!({"maxRadius": r, "channel1": 108.0}));
        Effect { id: HALFTONE.into(), params, visible: true }
    }

    fn radius(e: &Effect) -> f64 {
        e.params[RADIUS].as_f64().unwrap()
    }

    #[test]
    fn older_radii_become_the_pixels_that_draw_the_same_dots() {
        for (ppi, pt, px) in [(72.0, 8.0, 8.0), (300.0, 8.0, 33.333333333333336), (300.0, 4.0, 16.666666666666668), (150.0, 127.0, 127.0)] {
            let mut e = halftone(Some(pt));
            to_pixels(&mut e, ppi / 72.0);
            assert!((radius(&e) - px).abs() < 1e-9, "{pt} pt at {ppi} ppi: {}", radius(&e));
            assert_eq!(e.params["channel1"], json!(108.0), "the angles stay");
        }
        // A radius left out drew at the old default, 8 pt.
        let mut e = halftone(None);
        to_pixels(&mut e, 300.0 / 72.0);
        assert!((radius(&e) - 33.333333333333336).abs() < 1e-9);
        // Other effects keep their parameters.
        let mut blur = Effect { id: "blur.gaussian".into(), params: json!({"radius": 8.0}), visible: true };
        to_pixels(&mut blur, 300.0 / 72.0);
        assert_eq!(blur.params, json!({"radius": 8.0}));
    }

    #[test]
    fn only_documents_before_version_2_are_converted_once() {
        let mut d = Document::new(100.0, 100.0);
        d.raster_effects_ppi = 300.0;
        let mut style = d.graphic_styles.first().cloned().unwrap();
        style.appearance.effects.push(halftone(Some(8.0)));
        d.graphic_styles.push(style);
        let fresh = d.clone();
        d.migrate_halftone_radii();
        assert_eq!(d, fresh, "a new document's radii are already pixels");
        d.version = 1;
        d.migrate_halftone_radii();
        assert_eq!(d.version, FORMAT_VERSION);
        let r = d.graphic_styles.last().map(|s| radius(&s.appearance.effects[0])).unwrap();
        assert!((r - 33.333333333333336).abs() < 1e-9, "{r}");
        let once = d.clone();
        d.migrate_halftone_radii();
        assert_eq!(d, once, "converted documents are left alone");
    }
}
