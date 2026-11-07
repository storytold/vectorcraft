//! The colour visitor that recolouring shares: Edit Colors, Recolor Artwork and the live colour
//! adjustment effects walk an object's colours through [`recolor_node`] and decide what each
//! colour, mesh point and embedded image becomes through a [`ColorVisitor`].

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};

use crate::{AppearanceItem, Node, NodeKind};

/// What a recolouring pass does to the colours it meets.
pub trait ColorVisitor {
    /// A fill's or stroke's paint (an object's, or a text run's): true when it changed.
    fn paint(&mut self, p: &mut Paint) -> bool;
    /// One gradient-mesh point's colour.
    fn color(&mut self, c: Color) -> Color;
    /// The key of embedded image `key` recoloured, `None` when it stays as it is.
    fn image(&mut self, key: &str) -> Option<String>;
}

/// What a recolouring pass reaches: fills (including colorized image fills and gradient-mesh
/// points), strokes and the pixels of embedded images without a colorize paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reach {
    pub fill: bool,
    pub stroke: bool,
    pub images: bool,
}

impl Reach {
    /// Every colour.
    pub const ALL: Reach = Reach { fill: true, stroke: true, images: true };
}

/// Recolour `n` and everything inside it through `v`: the paints of its fills and strokes and of
/// its text runs, its gradient-mesh points and an embedded image's pixels (linked and colorized
/// images keep their pixels). Members that change are replaced by copies; the others stay shared.
/// Returns the number of paints, meshes and images that changed.
pub fn recolor_node(n: &mut Node, reach: Reach, v: &mut dyn ColorVisitor) -> usize {
    let mut changed = 0;
    let colorized = matches!(&n.kind, NodeKind::Image(_)) && n.extras().colorize.is_some();
    if colorized
        && reach.fill
        && let Some(paint) = n.extras_mut().colorize.as_deref_mut()
    {
        changed += v.paint(paint) as usize;
    }
    for it in &mut n.appearance.items {
        match it {
            AppearanceItem::Fill(l) if reach.fill => changed += v.paint(&mut l.paint) as usize,
            AppearanceItem::Stroke(l) if reach.stroke => changed += v.paint(&mut l.paint) as usize,
            _ => {}
        }
    }
    match &mut n.kind {
        NodeKind::Text(t) => {
            for r in &mut t.runs {
                if reach.fill {
                    changed += v.paint(&mut r.style.fill) as usize;
                }
                if reach.stroke {
                    changed += v.paint(&mut r.style.stroke) as usize;
                }
            }
        }
        NodeKind::Mesh(m) if reach.fill => {
            let mut ch = false;
            for p in &mut m.points {
                let c = v.color(p.color);
                ch |= c != p.color;
                p.color = c;
            }
            changed += ch as usize;
        }
        // An image holding its own pixels is a glyph's (emoji in outlined type): it keeps its
        // colours, as the glyph does in type.
        NodeKind::Image(im) if reach.images && !colorized && im.link.is_none() && im.pixels.is_none() => {
            if let Some(k) = v.image(&im.key) {
                im.key = k;
                changed += 1;
            }
        }
        _ => {
            for c in n.children_mut().into_iter().flatten() {
                let mut m = (**c).clone();
                let k = recolor_node(&mut m, reach, v);
                if k > 0 {
                    *c = Arc::new(m);
                    changed += k;
                }
            }
        }
    }
    changed
}

/// Apply `f` to the solid colour or gradient stops of a paint, with their swatch link and tint
/// ([`Paint::map_links`]); a gradient that changes is no longer its gradient swatch's. Returns
/// whether anything changed.
pub fn map_links(p: &mut Paint, f: &dyn Fn(&mut Color, &mut Option<String>, &mut f32) -> bool) -> bool {
    let changed = p.map_links(&mut |c, l, t| f(c, l, t));
    if changed && let Paint::Gradient(g) = p {
        g.swatch = None;
    }
    changed
}

/// Apply `f` to a paint's colours; a colour that changes loses its swatch link. Returns whether
/// anything changed.
pub fn map_paint(p: &mut Paint, f: &dyn Fn(Color) -> Color) -> bool {
    map_links(p, &|c, link, _| {
        let n = f(*c);
        if n == *c {
            return false;
        }
        *c = n;
        *link = None;
        true
    })
}
