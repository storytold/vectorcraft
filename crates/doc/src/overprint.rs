//! Overprinting: each fill and stroke (and each character's fill and stroke) can print its inks
//! over the inks below instead of knocking them out ([`crate::FillLayer::overprint`]).
//! Edit → Edit Colors → Overprint Black sets it on black fills and strokes ([`OverprintBlack`]).

use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color, GradientKind, Paint};

use crate::{Document, Node, NodeId, NodeKind};

/// `Document.unknown` key of files from before per-fill/stroke overprint: the ids of objects
/// marked Overprint Black ([`Document::migrate_overprint_black`]).
pub const LEGACY_OVERPRINT_KEY: &str = "overprintBlack";

/// Black ink of at least `min_k` (0..1): CMYK with that much K and, unless `rich`, no cyan,
/// magenta or yellow; or a grey that dark. RGB and Lab colours are never black ink.
pub fn is_black_ink(c: &Color, min_k: f32, rich: bool) -> bool {
    const E: f32 = 0.005;
    match *c {
        Color::Cmyk { c, m, y, k } => k >= min_k - E && (rich || (c <= E && m <= E && y <= E)),
        Color::Gray { k } => k >= min_k - E,
        Color::Rgb { .. } | Color::Lab { .. } => false,
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
    /// Whether `p` is black: a solid color (linked to one of the spot swatches `spots` only with
    /// [`Self::spot`]), a gradient whose every stop is, or a freeform gradient with color points
    /// whose every point is (it draws from its points, not its stops).
    fn black(&self, p: &Paint, spots: &[String]) -> bool {
        let ink = |color: &Color, swatch: &Option<String>| {
            (self.spot || !swatch.as_ref().is_some_and(|s| spots.contains(s))) && is_black_ink(color, self.min_k, self.rich)
        };
        match p {
            Paint::Solid { color, swatch, .. } => ink(color, swatch),
            // Only a freeform gradient draws its points; another kind draws its stops whatever it holds.
            Paint::Gradient(g) => match g.freeform.as_ref().filter(|f| g.gradient.kind == GradientKind::Freeform && !f.points.is_empty()) {
                Some(f) => f.points.iter().all(|pt| is_black_ink(&pt.color, self.min_k, self.rich)),
                // Without points of its own a freeform gradient places them from its stops.
                None => !g.gradient.stops.is_empty() && g.gradient.stops.iter().all(|s| ink(&s.color, &s.swatch)),
            },
            _ => false,
        }
    }

    /// Set (or clear) the overprint of `n`'s own black fills and strokes, its characters' and,
    /// for a colorized image, its color's.
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
        if self.colorize_black(n, spots) {
            set(&mut n.extras_mut().colorize_overprint);
        }
        changed
    }

    /// Whether `n` is an image colorized with a black color that [`Self::fill`] covers: the color
    /// of a gray or 1-bit image is its fill ([`crate::NodeExtras::colorize`]).
    fn colorize_black(&self, n: &Node, spots: &[String]) -> bool {
        self.fill && matches!(n.kind, NodeKind::Image(_)) && n.extras().colorize.as_deref().is_some_and(|p| self.black(p, spots))
    }

    /// [`Self::apply`] to every node of `a`'s subtree, copying only the nodes on the way to those
    /// that change. Returns whether anything changed.
    pub fn apply_deep(&self, a: &mut Arc<Node>, spots: &[String]) -> bool {
        match self.marked(a, spots) {
            Some(n) => {
                *a = n;
                true
            }
            None => false,
        }
    }

    /// `n`'s subtree with [`Self::apply`] done to each node, when that changes anything (`None`
    /// otherwise). Each node is visited once, and only the nodes on the way to a change are copied.
    fn marked(&self, n: &Arc<Node>, spots: &[String]) -> Option<Arc<Node>> {
        let members: Vec<(usize, Arc<Node>)> =
            n.children().into_iter().flatten().enumerate().filter_map(|(i, c)| Some((i, self.marked(c, spots)?))).collect();
        if members.is_empty() && !self.changes(n, spots) {
            return None;
        }
        let mut copy = Arc::clone(n);
        let m = Arc::make_mut(&mut copy);
        self.apply(m, spots);
        if let Some(children) = m.children_mut() {
            for (i, c) in members {
                if let Some(slot) = children.get_mut(i) {
                    *slot = c;
                }
            }
        }
        Some(copy)
    }

    /// Whether [`Self::apply`] changes a fill, stroke or character of `n` itself.
    fn changes(&self, n: &Node, spots: &[String]) -> bool {
        let on = !self.remove;
        let item = |it: &crate::AppearanceItem| {
            it.overprint() != on && (if it.is_fill() { self.fill } else { self.stroke }) && self.black(it.paint(), spots)
        };
        let run = |r: &crate::TextRun| {
            (self.fill && r.style.overprint_fill != on && self.black(&r.style.fill, spots))
                || (self.stroke && r.style.overprint_stroke != on && self.black(&r.style.stroke, spots))
        };
        n.appearance.items.iter().any(item)
            || matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(run))
            || (n.extras().colorize_overprint != on && self.colorize_black(n, spots))
    }
}

impl Node {
    /// Whether any fill or stroke of this subtree overprints (characters' aside).
    pub fn has_overprint(&self) -> bool {
        self.appearance.items.iter().any(|i| i.overprint())
            || self.colorize_overprints()
            || self.children().is_some_and(|c| c.iter().any(|c| c.has_overprint()))
    }

    /// Whether this is an image colorized with a color ([`crate::NodeExtras::colorize`]) that
    /// overprints ([`crate::NodeExtras::colorize_overprint`]).
    pub fn colorize_overprints(&self) -> bool {
        let e = self.extras();
        matches!(self.kind, NodeKind::Image(_)) && e.colorize_overprint && e.colorize.as_deref().is_some_and(|p| !p.is_none())
    }
}

/// Document Setup → Discard White Overprint as it applies to paints: off, or on with the names of
/// the document's spot swatches. Paper white knocks out where it overprints; the color of a spot
/// swatch is an ink of its own, so a white spot ink still overprints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiscardWhite(Option<Vec<String>>);

impl DiscardWhite {
    /// The setting of document `doc`.
    pub fn of(doc: &Document) -> Self {
        Self(doc.setup.discard_white_overprint.then(|| doc.spot_names()))
    }

    /// Whether `p` knocks out instead of overprinting.
    pub fn knocks_out(&self, p: &Paint) -> bool {
        self.0.as_ref().is_some_and(|spots| is_paper_white(p, |name| spots.iter().any(|s| s == name)))
    }
}

/// Whether `p` is paper white ([`is_white`]) and not linked to a swatch that `spot` says is a
/// spot ink.
fn is_paper_white(p: &Paint, spot: impl Fn(&str) -> bool) -> bool {
    is_white(p) && !matches!(p, Paint::Solid { swatch: Some(name), .. } if spot(name))
}

impl Document {
    /// Whether paint `p` is paper white: white ([`is_white`]) and not linked to a spot swatch,
    /// whose white is an ink.
    pub fn is_paper_white(&self, p: &Paint) -> bool {
        is_paper_white(p, |name| self.swatch(name).is_some_and(|w| w.spot))
    }

    /// Whether paint `p` knocks out where it overprints under Document Setup → Discard White
    /// Overprint ([`DiscardWhite`]).
    pub fn discards_white(&self, p: &Paint) -> bool {
        self.setup.discard_white_overprint && self.is_paper_white(p)
    }
}

/// Whether fill or stroke row `i` of an image draws: it is visible, has a paint and a visible
/// effect, which gives a row on an image a shape of its own.
pub fn image_row_draws(i: &crate::AppearanceItem) -> bool {
    i.visible() && !i.paint().is_none() && i.effects().iter().any(|e| e.visible)
}

/// Whether paint `paint` of blend mode `blend` darkens when `overprint` asks it to: with the
/// Normal blend, and not knocked out by `discard_white`.
fn darkens(overprint: bool, blend: BlendMode, paint: &Paint, discard_white: &DiscardWhite) -> bool {
    overprint && blend == BlendMode::Normal && !discard_white.knocks_out(paint)
}

/// Whether colorized image `n` is drawn overprinting as a whole: its color overprints
/// ([`Node::colorize_overprints`]) with the Normal blend and is not knocked out by
/// `discard_white`, and each row that draws ([`image_row_draws`]) overprints that way too. A
/// colorized image's color is not a fill row, so the whole node overprints or none of it does.
pub fn image_overprints_whole(n: &Node, discard_white: &DiscardWhite) -> bool {
    let row = |i: &crate::AppearanceItem| match i {
        crate::AppearanceItem::Fill(l) => darkens(l.overprint, l.blend, &l.paint, discard_white),
        crate::AppearanceItem::Stroke(l) => darkens(l.overprint, l.blend, &l.paint, discard_white),
    };
    n.colorize_overprints()
        && n.extras().colorize.as_deref().is_some_and(|p| darkens(true, n.blend, p, discard_white))
        && n.appearance.items.iter().filter(|i| image_row_draws(i)).all(row)
}

/// Draw the overprinting fills and strokes of `a`'s subtree with Darken (those with a blend
/// mode of their own keep it), copying only the nodes on the way to them: on screen and on a
/// printed plate this approximates their inks printing over the inks below. Darken keeps the
/// stronger ink of each plate (and each additive channel), so a colour overprinting itself stays
/// that colour, where Multiply would darken it. Paints that `discard_white` knocks out
/// (Document Setup → Discard White Overprint) are left as they are. A colorized image darkens as
/// a whole ([`image_overprints_whole`]).
pub fn darken_overprints(a: &mut Arc<Node>, discard_white: &DiscardWhite) {
    if !a.has_overprint() {
        return;
    }
    let n = Arc::make_mut(a);
    let whole = image_overprints_whole(n, discard_white);
    for it in &mut n.appearance.items {
        match it {
            crate::AppearanceItem::Fill(l) if darkens(l.overprint, l.blend, &l.paint, discard_white) => l.blend = BlendMode::Darken,
            crate::AppearanceItem::Stroke(l) if darkens(l.overprint, l.blend, &l.paint, discard_white) => l.blend = BlendMode::Darken,
            _ => {}
        }
    }
    if whole {
        n.blend = BlendMode::Darken;
    }
    for c in n.children_mut().into_iter().flatten() {
        darken_overprints(c, discard_white);
    }
}

/// A solid paint of paper white (no ink in any colour model).
pub fn is_white(p: &Paint) -> bool {
    let Paint::Solid { color, .. } = p else { return false };
    match *color {
        Color::Rgb { r, g, b } => r.min(g).min(b) >= 1.0,
        Color::Cmyk { c, m, y, k } => c.max(m).max(y).max(k) <= 0.0,
        Color::Gray { k } => k <= 0.0,
        Color::Lab { l, a, b } => l >= 100.0 && a == 0.0 && b == 0.0,
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
        // One walk of the tree finds them all (a missing id costs a walk each, and a list can repeat it).
        let paths = self.index_paths(ids);
        for path in paths.values() {
            if let Some(n) = self.node_mut_at(path) {
                op.apply(n, &[]);
            }
        }
    }
}

/// Whether any fill or stroke of `n`'s subtree overprints, characters' too.
fn overprints_anywhere(n: &Node) -> bool {
    n.appearance.items.iter().any(|i| i.overprint())
        || n.colorize_overprints()
        || matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| r.style.overprint_fill || r.style.overprint_stroke))
        || n.children().is_some_and(|c| c.iter().any(|c| overprints_anywhere(c)))
}

/// Stop every fill and stroke of `a`'s subtree overprinting (characters' too), copying only the
/// nodes on the way to them.
pub fn clear_overprints(a: &mut Arc<Node>) {
    if !overprints_anywhere(a) {
        return;
    }
    let n = Arc::make_mut(a);
    for it in &mut n.appearance.items {
        *it.overprint_mut() = false;
    }
    if let NodeKind::Text(t) = &mut n.kind {
        for r in &mut t.runs {
            (r.style.overprint_fill, r.style.overprint_stroke) = (false, false);
        }
    }
    if n.colorize_overprints() {
        n.extras_mut().colorize_overprint = false;
    }
    for c in n.children_mut().into_iter().flatten() {
        clear_overprints(c);
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
        let spot = Paint::Solid { color: Color::cmyk(0.0, 0.0, 0.0, 1.0), swatch: Some("Ink".into()), tint: 1.0 };
        let style = CharStyle { fill: spot, stroke: k, stroke_width: 1.0, ..CharStyle::default() };
        let mut t = Node::new(NodeId(2), NodeKind::Text(Box::new(TextObject::point(Point::ZERO, "Hi", style))));
        OverprintBlack::default().apply(&mut t, &["Ink".to_string()]);
        let NodeKind::Text(tx) = &t.kind else { panic!("type") };
        assert!(!tx.runs[0].style.overprint_fill && tx.runs[0].style.overprint_stroke);
        OverprintBlack { spot: true, ..Default::default() }.apply(&mut t, &["Ink".to_string()]);
        let NodeKind::Text(tx) = &t.kind else { panic!("type") };
        assert!(tx.runs[0].style.overprint_fill);
    }

    #[test]
    fn a_freeform_gradient_is_black_when_every_point_is() {
        use vectorcraft_color::freeform::{Freeform, FreeformPoint};
        let k = Color::cmyk(0.0, 0.0, 0.0, 1.0);
        let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient { kind: GradientKind::Freeform, ..Default::default() });
        for s in &mut g.gradient.stops {
            s.color = k;
        }
        let points = vec![FreeformPoint::new(Point::ZERO, k), FreeformPoint::new(Point::new(10.0, 0.0), Color::cmyk(1.0, 0.0, 0.0, 0.0))];
        g.freeform = Some(Freeform { points, ..Default::default() });
        let fill = |g: &vectorcraft_color::GradientPaint| Appearance::basic(Paint::Gradient(Box::new(g.clone())), Paint::None, 0.0);
        let mut n = Node::path(NodeId(1), Default::default(), fill(&g));
        assert!(!OverprintBlack::default().apply(&mut n, &[]), "a cyan point is drawn, whatever the stops");
        // Another kind of gradient draws its black stops, not the points it still holds.
        let mut linear = g.clone();
        linear.gradient.kind = GradientKind::Linear;
        let mut n = Node::path(NodeId(1), Default::default(), fill(&linear));
        assert!(OverprintBlack::default().apply(&mut n, &[]), "a linear gradient is judged by its stops");
        if let Some(f) = &mut g.freeform {
            f.points[1].color = Color::gray(1.0);
        }
        let mut n = Node::path(NodeId(1), Default::default(), fill(&g));
        assert!(OverprintBlack::default().apply(&mut n, &[]), "every point black");
        g.freeform = Some(Freeform::default());
        let mut n = Node::path(NodeId(1), Default::default(), fill(&g));
        assert!(OverprintBlack::default().apply(&mut n, &[]), "no points: placed from the black stops");
    }

    #[test]
    fn a_gradient_with_a_spot_linked_stop_is_black_only_with_spot() {
        let mut g = vectorcraft_color::GradientPaint::new(Default::default());
        for s in &mut g.gradient.stops {
            s.color = Color::cmyk(0.0, 0.0, 0.0, 1.0);
        }
        g.gradient.stops[0].swatch = Some("Ink".into());
        let k = Paint::Gradient(Box::new(g));
        let mut n = Node::path(NodeId(1), Default::default(), Appearance::basic(k, Paint::None, 0.0));
        let spots = ["Ink".to_string()];
        assert!(!OverprintBlack::default().apply(&mut n, &spots), "a stop linked to a spot swatch is not process black");
        assert!(OverprintBlack { spot: true, ..Default::default() }.apply(&mut n, &spots), "Include Spot Blacks takes it");
    }

    #[test]
    fn apply_deep_copies_only_the_nodes_that_change() {
        let k = cmyk(0.0, 0.0, 0.0, 1.0);
        let black = Arc::new(Node::path(NodeId(2), Default::default(), Appearance::basic(k, Paint::None, 0.0)));
        let cyan = Arc::new(Node::path(NodeId(3), Default::default(), Appearance::basic(cmyk(1.0, 0.0, 0.0, 0.0), Paint::None, 0.0)));
        let mut group = Arc::new(Node::group(NodeId(1), vec![black.clone(), cyan.clone()]));
        let before = group.clone();
        assert!(OverprintBlack::default().apply_deep(&mut group, &[]));
        let members = group.children().unwrap();
        assert!(members[0].appearance.fill().unwrap().overprint && !black.has_overprint(), "the copy is marked, not the original");
        assert!(Arc::ptr_eq(&members[1], &cyan), "the cyan member is shared, not copied");
        assert!(!before.has_overprint());
        let marked = group.clone();
        assert!(!OverprintBlack::default().apply_deep(&mut group, &[]), "nothing left to change");
        assert!(Arc::ptr_eq(&group, &marked), "and nothing is copied");
    }

    /// A gray image colorized with `color`, its color overprinting, with the fill row `fill`.
    fn colorized(color: Paint, fill: Paint) -> Node {
        let im = crate::ImageObject {
            key: "pic".into(),
            width: 1,
            height: 1,
            xf: vectorcraft_geom::Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        let mut n = Node::new(NodeId(1), NodeKind::Image(Box::new(im)));
        n.appearance = Appearance::basic(fill, Paint::None, 0.0).into();
        n.extras_mut().colorize = Some(Box::new(color));
        n.extras_mut().colorize_overprint = true;
        n
    }

    #[test]
    fn a_white_spot_ink_overprints_under_discard_white() {
        let mut d = Document::new(100.0, 100.0);
        let white = Color::cmyk(0.0, 0.0, 0.0, 0.0);
        d.swatches.push(vectorcraft_color::Swatch { name: "Opaque White".into(), paint: Paint::solid(white), global: true, spot: true });
        let spot = Paint::Solid { color: white, swatch: Some("Opaque White".into()), tint: 1.0 };
        let paper = Paint::solid(white);
        d.setup.discard_white_overprint = true;
        let discard = DiscardWhite::of(&d);
        assert!(discard.knocks_out(&paper) && d.discards_white(&paper), "paper white knocks out");
        assert!(!discard.knocks_out(&spot) && !d.discards_white(&spot), "a white spot ink is an ink");
        d.setup.discard_white_overprint = false;
        assert!(!DiscardWhite::of(&d).knocks_out(&paper) && !d.discards_white(&paper), "off: white overprints too");

        // The previews darken the spot ink and leave paper white knocking out.
        let blend = |fill: Paint| {
            let mut n = Node::path(NodeId(1), Default::default(), Appearance::basic(fill, Paint::None, 0.0));
            if let Some(f) = n.appearance.fill_mut() {
                f.overprint = true;
            }
            let mut a = Arc::new(n);
            darken_overprints(&mut a, &discard);
            a.appearance.fill().map(|f| f.blend)
        };
        assert_eq!(blend(spot.clone()), Some(BlendMode::Darken));
        assert_eq!(blend(paper.clone()), Some(BlendMode::Normal));
        let image = |color: Paint| {
            let mut a = Arc::new(colorized(color, Paint::None));
            darken_overprints(&mut a, &discard);
            a.blend
        };
        assert_eq!((image(spot), image(paper)), (BlendMode::Darken, BlendMode::Normal), "a colorized image's color too");
    }

    #[test]
    fn a_colorized_image_darkens_unless_its_rows_draw_an_effect() {
        let blend = |effect: Option<bool>, overprint: bool| {
            let mut n = colorized(cmyk(0.0, 0.0, 0.0, 1.0), cmyk(1.0, 0.0, 0.0, 0.0));
            if let Some(f) = n.appearance.fill_mut() {
                f.overprint = overprint;
                if let Some(visible) = effect {
                    f.effects.push(crate::Effect { id: "path.offsetPath".into(), params: serde_json::json!({}), visible });
                }
            }
            let mut a = Arc::new(n);
            darken_overprints(&mut a, &DiscardWhite::default());
            a.blend
        };
        assert_eq!(blend(None, false), BlendMode::Darken, "a row with no effect paints nothing on an image");
        assert_eq!(blend(Some(false), false), BlendMode::Darken, "nor one whose effect is hidden");
        assert_eq!(blend(Some(true), false), BlendMode::Normal, "a knocking-out row drawn by its effect keeps knocking out");
        assert_eq!(blend(Some(true), true), BlendMode::Darken, "with an overprinting row the whole image overprints");
    }

    #[test]
    fn overprint_black_takes_a_black_colorized_image() {
        let off = |mut n: Node| {
            n.extras_mut().colorize_overprint = false;
            Arc::new(n)
        };
        let mut a = off(colorized(cmyk(0.0, 0.0, 0.0, 1.0), Paint::None));
        assert!(!OverprintBlack { fill: false, ..Default::default() }.apply_deep(&mut a, &[]), "the color is the image's fill");
        assert!(OverprintBlack::default().apply_deep(&mut a, &[]) && a.colorize_overprints());
        assert!(!OverprintBlack::default().apply_deep(&mut a, &[]), "nothing left to change");
        assert!(OverprintBlack { remove: true, ..Default::default() }.apply_deep(&mut a, &[]) && !a.colorize_overprints());
        let mut cyan = off(colorized(cmyk(1.0, 0.0, 0.0, 0.0), Paint::None));
        assert!(!OverprintBlack::default().apply_deep(&mut cyan, &[]), "a cyan image is not black");
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
