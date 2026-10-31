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

/// Visits one colour: the colour, its swatch link and its tint, all of which it may change.
pub type ColorVisitor<'a> = dyn FnMut(&mut Color, &mut Option<String>, &mut f32) + 'a;

fn visit_paint(p: &mut Paint, f: &mut ColorVisitor) {
    match p {
        Paint::Solid { color, swatch, tint } => f(color, swatch, tint),
        Paint::Gradient(g) => {
            for s in &mut g.gradient.stops {
                f(&mut s.color, &mut s.swatch, &mut s.tint);
            }
        }
        _ => {}
    }
}

/// Visit every colour of a node tree (fills, strokes, gradient stops, text runs, mesh points, an
/// image's colorize color, not opacity masks) with its swatch link and tint. Mesh points have no
/// link: changes to it are dropped.
pub fn visit_node_colors(n: &mut Node, f: &mut ColorVisitor) {
    for it in &mut n.appearance.items {
        match it {
            AppearanceItem::Fill(l) => visit_paint(&mut l.paint, f),
            AppearanceItem::Stroke(l) => visit_paint(&mut l.paint, f),
        }
    }
    if let Some(p) = n.extras.0.as_deref_mut().and_then(|e| e.colorize.as_deref_mut()) {
        visit_paint(p, f);
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
                f(&mut p.color, &mut None, &mut 1.0);
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

/// Apply `f` to every colour of a node tree (fills, strokes, gradient stops, text runs, mesh
/// points) given its swatch link and tint, keeping the links.
pub fn map_node_colors(n: &mut Node, f: &mut dyn FnMut(&Color, Link) -> Color) {
    visit_node_colors(n, &mut |c, swatch, tint| *c = f(c, swatch.as_deref().map(|s| (s, *tint))));
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
