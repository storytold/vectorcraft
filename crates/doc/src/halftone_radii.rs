//! Color Halftone's Max Radius was a distance in points until documents of version 3; since then
//! it is in pixels of the Document Raster Effects resolution, as public documentation gives it
//! (#3260). [`Document::migrate_halftone_radii`] converts the radii of older documents, and
//! [`Document::halftone_radii_to_points`] converts them back for a file saved for an older app.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::limits::FORMAT_VERSION;
use crate::{Appearance, Document, Effect, Node};

/// The first document version whose Color Halftone radii are in pixels.
const PIXEL_RADII_SINCE: u32 = 3;
const HALFTONE: &str = "pixelate.colorHalftone";
const RADIUS: &str = "maxRadius";
/// The key of `Document::unknown` that holds the brush definitions (`vectorcraft-brush`'s
/// `DOC_KEY`), whose art can carry effects.
const BRUSHES: &str = "brushes";
/// The radius (points) an older document's halftone without one drew with.
const OLD_DEFAULT: f64 = 8.0;
/// The range the radius was kept within in points, and is kept within in pixels.
const RANGE: std::ops::RangeInclusive<f64> = 4.0..=127.0;

impl Document {
    /// Saved before version 3: each Color Halftone's Max Radius in points becomes the pixels of
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
        self.each_halftone(&mut |e| rescale(e, k));
    }

    /// For a file saved for an app before version 3 (every native file version before today's):
    /// each Color Halftone's Max Radius in pixels becomes the points that draw the same dots,
    /// within the 4 to 127 pt that app takes (a radius under 4 pt, from a fine screen at a high
    /// resolution, draws bigger dots there), and the document becomes version 2, so reopening the
    /// file here converts them back.
    pub fn halftone_radii_to_points(&mut self) {
        if self.version < PIXEL_RADII_SINCE {
            return;
        }
        self.version = PIXEL_RADII_SINCE - 1;
        let k = 72.0 / self.raster_ppi();
        self.each_halftone(&mut |e| rescale(e, k));
    }

    /// `fix` each effect in the art, symbols, patterns (and the one being edited as it was), graph
    /// designs, opacity masks, Shaper edits, graphic styles and brush definitions that use a Color
    /// Halftone.
    fn each_halftone(&mut self, fix: &mut dyn FnMut(&mut Effect)) {
        for l in &mut self.layers {
            node(l, fix);
        }
        for s in &mut self.symbols {
            node(&mut s.art, fix);
        }
        // Pattern editing is saved with the pattern as it was, which Cancel puts back.
        for p in self.patterns.iter_mut().chain(self.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut())) {
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
        if let Some(b) = self.unknown.get_mut(BRUSHES) {
            brushes(b, fix);
        }
    }
}

/// `fix` each Color Halftone in JSON `v` (brush definitions): every object whose `id` is
/// [`HALFTONE`] is a serialized [`Effect`] (object ids are numbers).
fn brushes(v: &mut Value, fix: &mut dyn FnMut(&mut Effect)) {
    match v {
        Value::Object(m) if m.get("id").and_then(Value::as_str) == Some(HALFTONE) => {
            let params = m.get("params").cloned().unwrap_or(Value::Null);
            let mut e = Effect { id: HALFTONE.into(), params, visible: true };
            fix(&mut e);
            m.insert("params".into(), e.params);
        }
        Value::Object(m) => m.values_mut().for_each(|v| brushes(v, fix)),
        Value::Array(a) => a.iter_mut().for_each(|v| brushes(v, fix)),
        _ => {}
    }
}

/// `e`'s radius times `k` (pixels a point, or points a pixel), within the effect's range, when it
/// is a Color Halftone. Its default is 8 in either unit.
fn rescale(e: &mut Effect, k: f64) {
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

/// Fix the effects of `n`'s subtree, its opacity masks and its Shaper edits (copying only what
/// changes).
fn node(n: &mut Arc<Node>, fix: &mut dyn FnMut(&mut Effect)) {
    let n = Arc::make_mut(n);
    if has_halftone(&n.appearance) {
        appearance(&mut n.appearance, fix);
    }
    if let Some(x) = n.extras_if_any() {
        if let Some(m) = x.mask.as_mut() {
            node(&mut m.art, fix);
        }
        // A Shaper Group's face and edge edits paint with appearances of their own.
        if let Some(s) = x.shaper.as_mut() {
            let faces = s.faces.iter_mut().filter_map(|f| f.paint.as_mut());
            for p in faces.chain(s.edges.iter_mut().filter_map(|e| e.paint.as_mut())) {
                appearance(p, fix);
            }
        }
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
            rescale(&mut e, ppi / 72.0);
            assert!((radius(&e) - px).abs() < 1e-9, "{pt} pt at {ppi} ppi: {}", radius(&e));
            assert_eq!(e.params["channel1"], json!(108.0), "the angles stay");
        }
        // A radius left out drew at the old default, 8 pt.
        let mut e = halftone(None);
        rescale(&mut e, 300.0 / 72.0);
        assert!((radius(&e) - 33.333333333333336).abs() < 1e-9);
        // Other effects keep their parameters.
        let mut blur = Effect { id: "blur.gaussian".into(), params: json!({"radius": 8.0}), visible: true };
        rescale(&mut blur, 300.0 / 72.0);
        assert_eq!(blur.params, json!({"radius": 8.0}));
    }

    #[test]
    fn only_documents_before_version_3_are_converted_once() {
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

    #[test]
    fn radii_go_back_to_points_for_an_older_app_and_return_unchanged() {
        let mut d = Document::new(100.0, 100.0);
        d.raster_effects_ppi = 300.0;
        let mut style = d.graphic_styles.first().cloned().unwrap();
        style.appearance.effects.push(halftone(Some(100.0)));
        style.appearance.effects.push(halftone(Some(4.0)));
        d.graphic_styles.push(style);
        let r = |d: &Document, i: usize| d.graphic_styles.last().map(|s| radius(&s.appearance.effects[i])).unwrap();
        d.halftone_radii_to_points();
        assert_eq!(d.version, 2, "reopened here, it converts back");
        assert!((r(&d, 0) - 24.0).abs() < 1e-9, "100 px at 300 ppi is 24 pt: {}", r(&d, 0));
        assert_eq!(r(&d, 1), 4.0, "0.96 pt is under the 4 pt the older app takes");
        let points = d.clone();
        d.halftone_radii_to_points();
        assert_eq!(d, points, "already in points");
        d.migrate_halftone_radii();
        assert!((r(&d, 0) - 100.0).abs() < 1e-9, "{}", r(&d, 0));
    }

    /// Shaper face and edge edits paint with appearances of their own, and brush definitions keep
    /// their art as JSON: their radii convert both ways too.
    #[test]
    fn shaper_edits_and_brushes_are_converted_both_ways() {
        use crate::shaper::{FaceEdit, FaceSelector, ShaperSpec};
        let mut d = Document::new(100.0, 100.0);
        d.raster_effects_ppi = 300.0;
        d.version = 1;
        let mut look = Appearance::default_art();
        look.effects = vec![halftone(Some(8.0))];
        let face = FaceEdit {
            selector: FaceSelector { sources: vec![0], anchor: vectorcraft_geom::Point::ZERO },
            erase: false,
            erase_edges: false,
            paint_source: None,
            merge: None,
            paint: Some(look),
        };
        let mut shaper = Node::group(d.alloc_id(), vec![]);
        shaper.extras_mut().shaper = Some(Box::new(ShaperSpec { faces: vec![face], edges: vec![] }));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, shaper).unwrap();
        let effect = serde_json::to_value(halftone(Some(8.0))).unwrap();
        d.unknown.insert(BRUSHES.into(), json!([{"name": "B", "art": {"id": 99, "appearance": {"effects": [effect]}}}]));
        let radii = |d: &Document| {
            let shaper = d.layers[0].children().and_then(|c| c.first()).cloned().unwrap();
            let paint = shaper.extras().shaper.as_ref().and_then(|s| s.faces[0].paint.clone()).unwrap();
            let brush = d.unknown[BRUSHES][0]["art"]["appearance"]["effects"][0]["params"][RADIUS].as_f64().unwrap();
            (radius(&paint.effects[0]), brush)
        };
        d.migrate_halftone_radii();
        let (face, brush) = radii(&d);
        assert!((face - 33.333333333333336).abs() < 1e-9, "the face edit: {face}");
        assert!((brush - 33.333333333333336).abs() < 1e-9, "the brush: {brush}");
        assert_eq!(d.unknown[BRUSHES][0]["art"]["appearance"]["effects"][0]["params"]["channel1"], json!(108.0));
        d.halftone_radii_to_points();
        let (face, brush) = radii(&d);
        assert!((face - 8.0).abs() < 1e-9 && (brush - 8.0).abs() < 1e-9, "back in points: {face}, {brush}");
    }

    #[test]
    fn the_pattern_being_edited_is_converted_as_it_was() {
        let mut d = Document::new(100.0, 100.0);
        d.raster_effects_ppi = 300.0;
        let id = d.alloc_id();
        let mut look = Appearance::default_art();
        look.effects = vec![halftone(Some(8.0))];
        let square = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
        let tile = Arc::new(Node::path(id, square, look));
        let original = Some(crate::PatternDef::new("Tile", vec![tile]));
        d.pattern_edit = Some(crate::PatternEdit { pattern: "Tile".into(), layer: crate::NodeId(0), original });
        d.version = 1;
        d.migrate_halftone_radii();
        let art = &d.pattern_edit.as_ref().unwrap().original.as_ref().unwrap().art[0];
        assert!((radius(&art.appearance.effects[0]) - 33.333333333333336).abs() < 1e-9, "the tile Cancel puts back");
    }
}
