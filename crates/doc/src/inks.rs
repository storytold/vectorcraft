//! Printing inks: the plates of a document (four process inks and its spot inks), separating a
//! colour into them ([`inks`]), and visiting every colour of the art with its swatch link (the
//! Separations Preview recolours the art with these; print separates it into plates).

use std::sync::Arc;

use vectorcraft_color::cms::{Cms, Intent, PROCESS_PLATES};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Paint};

use crate::{AppearanceItem, Document, Node, NodeKind};

/// Ink coverage of one paint colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Inks {
    /// Process inks C, M, Y, K (0..1).
    pub cmyk: [f32; 4],
    /// Spot ink (swatch name, tint 0..1).
    pub spot: Option<(String, f32)>,
    /// Registration: the tint (every process ink's) prints on every spot plate too.
    pub registration: bool,
}

impl Inks {
    /// The tint this colour prints on spot plate `name`.
    pub fn spot_tint(&self, name: &str) -> f32 {
        match &self.spot {
            _ if self.registration => self.cmyk[0],
            Some((n, t)) if n == name => *t,
            _ => 0.0,
        }
    }

    /// The ink this colour prints on `plate` (a process plate name or a spot swatch name).
    pub fn on_plate(&self, plate: &str) -> f32 {
        match PROCESS_PLATES.iter().position(|p| *p == plate) {
            Some(i) => self.cmyk[i],
            None => self.spot_tint(plate),
        }
    }
}

/// A printing plate.
#[derive(Clone, Debug, PartialEq)]
pub struct Plate {
    pub name: String,
    pub spot: bool,
    /// Display colour of the ink (for the panel swatch).
    pub rgb: [f32; 3],
}

/// The plates of `doc`: the four process plates plus one per spot swatch.
pub fn plates(doc: &Document) -> Vec<Plate> {
    let c = vectorcraft_color::cms::active();
    let mut v: Vec<Plate> = PROCESS_PLATES
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut ink = [0.0; 4];
            ink[i] = 1.0;
            Plate { name: (*n).into(), spot: false, rgb: c.cmyk_to_srgb(ink, false) }
        })
        .collect();
    for sw in doc.swatches_iter() {
        if let (true, Paint::Solid { color, .. }) = (sw.spot, &sw.paint)
            && !v.iter().any(|p| p.name == sw.name)
        {
            v.push(Plate { name: sw.name.clone(), spot: true, rgb: c.display_rgb(&doc.linked_color(*color, true)) });
        }
    }
    v
}

/// The ink colour of spot swatch `name` as it shows ([`Document::linked_color`]).
pub fn spot_color(doc: &Document, name: &str) -> Option<Color> {
    let s = doc.swatch(name).filter(|s| s.spot)?;
    s.paint.color().map(|c| doc.linked_color(c, true))
}

/// A colour's swatch link and tint (`(swatch name, tint 0..1)`), as the colour visitors pass it.
pub type Link<'a> = Option<(&'a str, f32)>;

/// Separate one colour into inks. A colour linked to a spot swatch prints on that plate only, at
/// its tint; one linked to the Registration swatch prints its tint on every plate.
pub fn inks(doc: &Document, c: &Cms, color: &Color, link: Link, intent: Intent) -> Inks {
    if let Some((REGISTRATION, tint)) = link {
        return Inks { cmyk: [tint.clamp(0.0, 1.0); 4], spot: None, registration: true };
    }
    if let Some((name, tint)) = link
        && doc.swatch(name).is_some_and(|s| s.spot && s.paint.color().is_some())
    {
        return Inks { cmyk: [0.0; 4], spot: Some((name.to_string(), tint.clamp(0.0, 1.0))), registration: false };
    }
    Inks { cmyk: c.to_cmyk(color, intent), spot: None, registration: false }
}

/// Process plates a colour prints together with its spot tint (C, M, Y, K, 0..1): a gradient
/// stop's [`vectorcraft_color::GradientStop::plates`]; `None` for every other colour.
pub type StopPlates = Option<[f32; 4]>;

/// [`inks`] of a colour that may print process plates `plates` with its spot tint. The plates
/// print only beside a spot ink: a colour without a valid spot link, or linked to Registration,
/// prints as [`inks`] separates it. A plate amount that is not a finite number prints no ink.
pub fn stop_inks(doc: &Document, c: &Cms, color: &Color, link: Link, plates: StopPlates, intent: Intent) -> Inks {
    let mut ink = inks(doc, c, color, link, intent);
    if let (Some(p), Some(_)) = (plates, &ink.spot) {
        ink.cmyk = p.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    }
    ink
}

/// Visits one colour: the colour, its swatch link, its tint and its process plates
/// ([`StopPlates`]), all of which it may change.
pub type ColorVisitor<'a> = dyn FnMut(&mut Color, &mut Option<String>, &mut f32, &mut StopPlates) + 'a;

fn visit_paint(p: &mut Paint, f: &mut ColorVisitor) {
    match p {
        Paint::Solid { color, swatch, tint } => f(color, swatch, tint, &mut None),
        Paint::Gradient(g) => {
            for s in &mut g.gradient.stops {
                f(&mut s.color, &mut s.swatch, &mut s.tint, &mut s.plates);
            }
            for p in g.freeform.iter_mut().flat_map(|ff| ff.points.iter_mut()) {
                f(&mut p.color, &mut None, &mut 1.0, &mut None);
            }
        }
        _ => {}
    }
}

/// Visit every colour of a node tree (fills, strokes, gradient stops, a freeform gradient's
/// points, text runs, mesh points, an image's colorize color, the fills and strokes a dynamic
/// symbol instance keeps for objects of its art, not opacity masks) with its swatch link, tint and
/// process plates. Freeform and mesh points have no link or plates: a link or plates `f` gives
/// them are dropped.
pub fn visit_node_colors(n: &mut Node, f: &mut ColorVisitor) {
    for it in &mut n.appearance.items {
        match it {
            AppearanceItem::Fill(l) => visit_paint(&mut l.paint, f),
            AppearanceItem::Stroke(l) => visit_paint(&mut l.paint, f),
        }
    }
    if let Some(e) = n.extras.0.as_deref_mut() {
        if let Some(p) = e.colorize.as_deref_mut() {
            visit_paint(p, f);
        }
        for p in e.symbol_overrides.iter_mut().flat_map(|o| [&mut o.fill, &mut o.stroke]).flatten() {
            visit_paint(p, f);
        }
    }
    match &mut n.kind {
        NodeKind::Text(t) => {
            for r in &mut t.runs {
                visit_paint(&mut r.style.fill, f);
                visit_paint(&mut r.style.stroke, f);
            }
        }
        NodeKind::Mesh(m) => {
            for p in &mut m.points {
                f(&mut p.color, &mut None, &mut 1.0, &mut None);
            }
        }
        _ => {}
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            visit_node_colors(Arc::make_mut(c), f);
        }
    }
}

/// Apply `f` to every colour of a node tree (fills, strokes, gradient stops, a freeform
/// gradient's points, text runs, mesh points, a dynamic symbol instance's own fills and strokes)
/// given its swatch link and tint, keeping the links and process plates.
pub fn map_node_colors(n: &mut Node, f: &mut dyn FnMut(&Color, Link) -> Color) {
    map_node_inks(n, &mut |c, link, _| f(c, link));
}

/// [`map_node_colors`], with each colour's process plates ([`StopPlates`]) passed to `f` too.
pub fn map_node_inks(n: &mut Node, f: &mut dyn FnMut(&Color, Link, StopPlates) -> Color) {
    visit_node_colors(n, &mut |c, swatch, tint, plates| *c = f(c, swatch.as_deref().map(|s| (s, *tint)), *plates));
}

/// Apply `f` to every colour in the document's art and symbol definitions.
pub fn map_document_colors(doc: &mut Document, f: &mut dyn FnMut(&Color, Link) -> Color) {
    for l in &mut doc.layers {
        map_node_colors(Arc::make_mut(l), f);
    }
    for s in &mut doc.symbols {
        map_node_colors(Arc::make_mut(&mut s.art), f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, SymbolOverride};
    use vectorcraft_color::{Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint, GradientStop, Swatch};
    use vectorcraft_geom::Affine;

    /// A document with spot swatch `Gold` and process swatch `Teal`.
    fn gold_doc() -> Document {
        let mut d = Document::new(100.0, 100.0);
        d.swatches.push(Swatch { name: "Gold".into(), paint: Paint::solid(Color::cmyk(0.0, 0.2, 0.8, 0.1)), global: true, spot: true });
        d.swatches.push(Swatch { name: "Teal".into(), paint: Paint::solid(Color::cmyk(0.8, 0.0, 0.3, 0.0)), global: true, spot: false });
        d
    }

    #[test]
    fn a_spot_stops_process_plates_print_with_its_tint() {
        let d = gold_doc();
        let c = vectorcraft_color::cms::active();
        let look = Color::cmyk(0.25, 0.1, 0.8, 0.05);
        let ink = |link: Link, plates: StopPlates| stop_inks(&d, &c, &look, link, plates, Intent::RelativeColorimetric);
        let mixed = ink(Some(("Gold", 0.5)), Some([0.25, 0.0, 0.4, 0.0]));
        assert_eq!(mixed, Inks { cmyk: [0.25, 0.0, 0.4, 0.0], spot: Some(("Gold".into(), 0.5)), registration: false });
        assert_eq!((mixed.on_plate("Cyan"), mixed.on_plate("Gold")), (0.25, 0.5));
        // Amounts outside 0..1 are clamped; one that is not a number prints no ink.
        let odd = ink(Some(("Gold", 1.0)), Some([f32::NAN, 2.0, -1.0, f32::INFINITY]));
        assert_eq!(odd.cmyk, [0.0, 1.0, 0.0, 0.0]);
        // Without plates the spot ink prints alone, as `inks` separates it.
        assert_eq!(ink(Some(("Gold", 0.5)), None), inks(&d, &c, &look, Some(("Gold", 0.5)), Intent::RelativeColorimetric));
        // Plates beside no spot ink print nothing of their own: a process swatch, an unknown
        // swatch, no link and Registration separate as `inks` does.
        for link in [Some(("Teal", 1.0)), Some(("Missing", 1.0)), None, Some((REGISTRATION, 0.5))] {
            assert_eq!(ink(link, Some([0.25, 0.0, 0.4, 0.0])), inks(&d, &c, &look, link, Intent::RelativeColorimetric), "{link:?}");
        }
    }

    #[test]
    fn gradient_stops_pass_their_plates_and_freeform_points_are_visited() {
        let mut stop = GradientStop::new(0.0, Color::cmyk(0.25, 0.1, 0.8, 0.05));
        (stop.swatch, stop.tint, stop.plates) = (Some("Gold".into()), 0.5, Some([0.25, 0.0, 0.4, 0.0]));
        let mut g = GradientPaint::new(Gradient::new(GradientKind::Linear, vec![stop, GradientStop::new(1.0, Color::rgb(1.0, 1.0, 1.0))]));
        let point = FreeformPoint::new(kurbo::Point::new(10.0, 10.0), Color::rgb(0.0, 1.0, 0.0));
        g.freeform = Some(Freeform { points: vec![point], ..Default::default() });
        let mut n = Node::path(
            NodeId(1),
            vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0)),
            crate::Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 0.0),
        );
        let mut seen = vec![];
        let gray = Color::Gray { k: 0.5 };
        map_node_inks(&mut n, &mut |c, link, plates| {
            seen.push((*c, link.map(|(s, t)| (s.to_string(), t)), plates));
            gray
        });
        assert_eq!(
            seen,
            vec![
                (Color::cmyk(0.25, 0.1, 0.8, 0.05), Some(("Gold".to_string(), 0.5)), Some([0.25, 0.0, 0.4, 0.0])),
                (Color::rgb(1.0, 1.0, 1.0), None, None),
                (Color::rgb(0.0, 1.0, 0.0), None, None),
            ]
        );
        let Some(Paint::Gradient(g)) = n.appearance.fill().map(|f| f.paint.clone()) else { panic!("no gradient fill") };
        assert_eq!(g.gradient.stops[0].plates, Some([0.25, 0.0, 0.4, 0.0]), "mapping a colour keeps its plates");
        assert!(g.gradient.stops.iter().all(|s| s.color == gray));
        assert_eq!(g.freeform.map(|f| f.points[0].color), Some(gray), "the freeform point is mapped too");
    }

    #[test]
    fn a_dynamic_instances_own_fills_and_strokes_are_visited() {
        let red = Color::rgb(1.0, 0.0, 0.0);
        let blue = Color::rgb(0.0, 0.0, 1.0);
        let spot = Paint::Solid { color: red, swatch: Some("Spot".into()), tint: 0.5 };
        let mut inst = Node::new(NodeId(1), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::IDENTITY });
        inst.extras_mut().symbol_overrides = vec![
            SymbolOverride { path: vec![0], fill: Some(spot), stroke: Some(Paint::solid(red)), ..Default::default() },
            SymbolOverride { path: vec![1], opacity: Some(0.5), ..Default::default() },
        ];
        let mut links = vec![];
        map_node_colors(&mut inst, &mut |_, link| {
            links.push(link.map(|(s, t)| (s.to_string(), t)));
            blue
        });
        assert_eq!(links, vec![Some(("Spot".to_string(), 0.5)), None], "each override paint is visited once, with its link");
        let o = &inst.extras().symbol_overrides;
        assert_eq!(o[0].fill, Some(Paint::Solid { color: blue, swatch: Some("Spot".into()), tint: 0.5 }), "the link is kept");
        assert_eq!(o[0].stroke, Some(Paint::solid(blue)));
        assert_eq!((o[1].fill.as_ref(), o[1].stroke.as_ref()), (None, None), "an override without paint gets none");
    }
}
