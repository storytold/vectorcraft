//! Overprinting: each fill and stroke (and each character's fill and stroke) can print its inks
//! over the inks below instead of knocking them out ([`crate::FillLayer::overprint`]).
//! Edit → Edit Colors → Overprint Black sets it on black fills and strokes ([`OverprintBlack`]).

use vectorcraft_color::{Color, Paint};

use crate::{Document, Node, NodeId, NodeKind};

/// `Document.unknown` key of files from before per-fill/stroke overprint: the ids of objects
/// marked Overprint Black ([`Document::migrate_overprint_black`]).
pub const LEGACY_OVERPRINT_KEY: &str = "overprintBlack";

/// Black ink of at least `min_k` (0..1): CMYK with that much K and, unless `rich`, no cyan,
/// magenta or yellow; or a grey that dark. RGB colours are never black ink.
pub fn is_black_ink(c: &Color, min_k: f32, rich: bool) -> bool {
    const E: f32 = 0.005;
    match *c {
        Color::Cmyk { c, m, y, k } => k >= min_k - E && (rich || (c <= E && m <= E && y <= E)),
        Color::Gray { k } => k >= min_k - E,
        Color::Rgb { .. } => false,
    }
}

/// Edit → Edit Colors → Overprint Black: which fills and strokes count as black, and whether
/// they start or stop overprinting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverprintBlack {
    /// Remove overprinting instead of adding it.
    pub remove: bool,
    /// Minimum black ink, 0..1.
    pub min_k: f32,
    pub fill: bool,
    pub stroke: bool,
    /// Include Blacks with CMY: rich blacks count too.
    pub rich: bool,
    /// Include Spot Blacks: colours linked to a spot swatch count too.
    pub spot: bool,
}

impl Default for OverprintBlack {
    fn default() -> Self {
        Self { remove: false, min_k: 1.0, fill: true, stroke: true, rich: false, spot: false }
    }
}

impl OverprintBlack {
    /// Whether `p` is black: a solid colour (linked to one of the spot swatches `spots` only with
    /// [`Self::spot`]), or a gradient whose every stop is.
    fn black(&self, p: &Paint, spots: &[String]) -> bool {
        match p {
            Paint::Solid { color, swatch } => {
                (self.spot || !swatch.as_ref().is_some_and(|s| spots.contains(s))) && is_black_ink(color, self.min_k, self.rich)
            }
            Paint::Gradient(g) => !g.gradient.stops.is_empty() && g.gradient.stops.iter().all(|s| is_black_ink(&s.color, self.min_k, self.rich)),
            _ => false,
        }
    }

    /// Set (or clear) the overprint of `n`'s own black fills and strokes and its characters'.
    /// `spots`: the document's spot swatch names ([`Document::spot_names`]). Returns whether
    /// anything changed.
    pub fn apply(&self, n: &mut Node, spots: &[String]) -> bool {
        let on = !self.remove;
        let mut changed = false;
        let mut set = |flag: &mut bool| {
            changed |= *flag != on;
            *flag = on;
        };
        for it in &mut n.appearance.items {
            if (if it.is_fill() { self.fill } else { self.stroke }) && self.black(it.paint(), spots) {
                set(it.overprint_mut());
            }
        }
        if let NodeKind::Text(t) = &mut n.kind {
            for r in &mut t.runs {
                if self.fill && self.black(&r.style.fill, spots) {
                    set(&mut r.style.overprint_fill);
                }
                if self.stroke && self.black(&r.style.stroke, spots) {
                    set(&mut r.style.overprint_stroke);
                }
            }
        }
        changed
    }
}

impl Node {
    /// Whether any fill or stroke of this subtree overprints (characters' aside).
    pub fn has_overprint(&self) -> bool {
        self.appearance.items.iter().any(|i| i.overprint()) || self.children().is_some_and(|c| c.iter().any(|c| c.has_overprint()))
    }
}

impl Document {
    /// The names of the spot swatches.
    pub fn spot_names(&self) -> Vec<String> {
        self.swatches_iter().filter(|s| s.spot).map(|s| s.name.clone()).collect()
    }

    /// Files from before per-fill/stroke overprint listed the objects marked Overprint Black
    /// under [`LEGACY_OVERPRINT_KEY`]: their 100% black fills and strokes overprint now.
    pub fn migrate_overprint_black(&mut self) {
        let Some(list) = self.unknown.remove(LEGACY_OVERPRINT_KEY) else { return };
        let ids: Vec<NodeId> = list.as_array().into_iter().flatten().filter_map(|v| v.as_u64()).map(NodeId).collect();
        // The old list marked spot-linked blacks too (so the spot names don't matter).
        let op = OverprintBlack { spot: true, ..Default::default() };
        for id in ids {
            if let Some(n) = self.node_mut(id) {
                op.apply(n, &[]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, CharStyle, TextObject};
    use vectorcraft_geom::Point;

    fn cmyk(c: f32, m: f32, y: f32, k: f32) -> Paint {
        Paint::solid(Color::cmyk(c, m, y, k))
    }

    #[test]
    fn black_ink_rules() {
        assert!(is_black_ink(&Color::cmyk(0.0, 0.0, 0.0, 1.0), 1.0, false));
        assert!(!is_black_ink(&Color::cmyk(0.6, 0.4, 0.4, 1.0), 1.0, false), "rich black needs `rich`");
        assert!(is_black_ink(&Color::cmyk(0.6, 0.4, 0.4, 1.0), 1.0, true));
        assert!(is_black_ink(&Color::cmyk(0.0, 0.0, 0.0, 0.8), 0.75, false) && !is_black_ink(&Color::cmyk(0.0, 0.0, 0.0, 0.8), 0.9, false));
        assert!(is_black_ink(&Color::gray(1.0), 1.0, false));
        assert!(!is_black_ink(&Color::rgb(0.0, 0.0, 0.0), 1.0, true), "RGB black is not ink");
    }

    #[test]
    fn apply_covers_items_runs_gradients_and_spots() {
        let k = cmyk(0.0, 0.0, 0.0, 1.0);
        let mut n = Node::path(NodeId(1), Default::default(), Appearance::basic(k.clone(), k.clone(), 1.0));
        let op = OverprintBlack { fill: false, ..Default::default() };
        assert!(op.apply(&mut n, &[]));
        assert!(!n.appearance.fill().unwrap().overprint && n.appearance.stroke().unwrap().overprint, "fill: false marks strokes only");
        assert!(!op.apply(&mut n, &[]), "nothing left to change");
        assert!(OverprintBlack { remove: true, ..Default::default() }.apply(&mut n, &[]));
        assert!(!n.has_overprint());

        // Gradients count when every stop is black.
        let mut g = vectorcraft_color::GradientPaint::new(Default::default());
        for s in &mut g.gradient.stops {
            s.color = Color::cmyk(0.0, 0.0, 0.0, 1.0);
        }
        n.appearance.set_fill(Paint::Gradient(Box::new(g.clone())));
        assert!(OverprintBlack::default().apply(&mut n, &[]) && n.appearance.fill().unwrap().overprint);
        g.gradient.stops[0].color = Color::cmyk(0.0, 0.0, 0.0, 0.0);
        let fill = n.appearance.fill_mut().unwrap();
        fill.paint = Paint::Gradient(Box::new(g));
        fill.overprint = false;
        assert!(!OverprintBlack::default().apply(&mut n, &[]), "the stroke was marked already");
        assert!(!n.appearance.fill().unwrap().overprint, "a gradient with a white stop is not black");

        // Spot-linked blacks only with `spot`; type runs too.
        let spot = Paint::Solid { color: Color::cmyk(0.0, 0.0, 0.0, 1.0), swatch: Some("Ink".into()) };
        let style = CharStyle { fill: spot, stroke: k, stroke_width: 1.0, ..CharStyle::default() };
        let mut t = Node::new(NodeId(2), NodeKind::Text(Box::new(TextObject::point(Point::ZERO, "Hi", style))));
        OverprintBlack::default().apply(&mut t, &["Ink".to_string()]);
        let NodeKind::Text(tx) = &t.kind else { panic!("not text") };
        assert!(!tx.runs[0].style.overprint_fill && tx.runs[0].style.overprint_stroke);
        OverprintBlack { spot: true, ..Default::default() }.apply(&mut t, &["Ink".to_string()]);
        let NodeKind::Text(tx) = &t.kind else { panic!("not text") };
        assert!(tx.runs[0].style.overprint_fill);
    }

    #[test]
    fn legacy_overprint_black_list_migrates() {
        let mut d = Document::new(100.0, 100.0);
        let layer = d.default_layer().unwrap();
        let k = cmyk(0.0, 0.0, 0.0, 1.0);
        let a = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::path(a, Default::default(), Appearance::basic(k.clone(), cmyk(1.0, 0.0, 0.0, 0.0), 1.0))).unwrap();
        let b = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::path(b, Default::default(), Appearance::basic(k, Paint::None, 1.0))).unwrap();
        d.unknown.insert(LEGACY_OVERPRINT_KEY.into(), serde_json::json!([a.0]));
        d.migrate_overprint_black();
        assert!(!d.unknown.contains_key(LEGACY_OVERPRINT_KEY));
        let na = d.node(a).unwrap();
        assert!(na.appearance.fill().unwrap().overprint && !na.appearance.stroke().unwrap().overprint, "the black fill, not the cyan stroke");
        assert!(!d.node(b).unwrap().has_overprint(), "objects not listed keep knocking out");
    }

    #[test]
    fn overprint_round_trips_and_old_layers_load() {
        let mut f = crate::FillLayer::new(Paint::None);
        assert!(!serde_json::to_string(&f).unwrap().contains("overprint"), "off is not written");
        f.overprint = true;
        let back: crate::FillLayer = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert!(back.overprint);
        let old: crate::StrokeLayer = serde_json::from_str(r#"{"paint":{"type":"none"},"width":2.0}"#).unwrap();
        assert!(!old.overprint);
        let st = CharStyle { overprint_stroke: true, ..CharStyle::default() };
        let back: CharStyle = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert!(back.overprint_stroke && !back.overprint_fill);
    }
}
