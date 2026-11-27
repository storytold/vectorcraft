//! Arrowheads a file defines: the art of a symbol that a stroke's arrowhead entry names
//! ([`super::style::StrokeArrows`]), read into a head ([`head`]).
//!
//! Not publicly documented. As the files have it, an arrowhead symbol's art (y down here, as the
//! document's symbols are) points right along a straight, level sample of the line it ends: an
//! open stroked path with no fill whose anchors and handles all lie on that line, 1 pt wide. The
//! sample ends where the line stops under the head. The rest of the art that paints is the head;
//! its tip is its rightmost point, and a symbol unit is one point on a 1 pt stroke at 100 %, so a
//! unit is the head's weight as it is for VectorCraft's own heads. Art that paints nothing (the
//! frame of the symbol's box, hidden or fully transparent art) is left out.

use vectorcraft_doc::{AppearanceItem, CustomArrowhead, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, FillRule, PathData, Point};

/// The most anchors an arrowhead symbol's art is read with, outlined strokes included, before its
/// shapes are united.
const MAX_ART_ANCHORS: usize = 65_536;
/// How deep the groups of an arrowhead symbol's art are read.
const MAX_DEPTH: usize = 16;

/// The head that symbol art `art` (in the document's symbol space) draws, named `name`. `None` when
/// the art doesn't have exactly one sample of the line, its sample isn't 1 pt wide, it holds
/// anything but paths, compound paths and groups (a clip group, say), has an effect, a brush, an
/// opacity mask, partial opacity or a group with fills or strokes of its own, paints no head, or is
/// too detailed for one; or when the
/// line would stop past the head's tip or farther back than a head can reach
/// ([`CustomArrowhead::MAX_UNITS`]).
pub(super) fn head(name: &str, art: &Node) -> Option<CustomArrowhead> {
    let mut parts = Parts::default();
    match &art.kind {
        // The members of the symbol's group are its art's top level, where the sample is.
        NodeKind::Group { children, clip: false } if art.visible && plain(art) && !paints(art) => {
            for c in children {
                parts.member(c, true, 1)?;
            }
        }
        _ => parts.member(art, true, 0)?,
    }
    let [(axis, line_end)] = parts.lines.as_slice() else { return None };
    if parts.shapes.is_empty() {
        return None;
    }
    let refs: Vec<(&PathData, FillRule)> = parts.shapes.iter().map(|(p, r)| (p, *r)).collect();
    let area = vectorcraft_pathops::try_unite_all(&refs).ok()?;
    let b = area.bounds().filter(|b| b.is_finite() && b.height() > 1e-9)?;
    let tip = b.x1;
    // Head units: x back from the tip, y across to the right of the way the head points (down).
    let h = CustomArrowhead::new(name.to_string(), area.transformed(Affine::new([-1.0, 0.0, 0.0, 1.0, tip, -axis])));
    let h = h.with_stop(tip - line_end);
    (!h.path.subpaths.is_empty() && h.stop.is_some()).then_some(h)
}

/// What an arrowhead symbol's art is made of.
#[derive(Default)]
struct Parts {
    /// Each sample of the line: the line's y, and the x where the sample ends.
    lines: Vec<(f64, f64)>,
    /// The areas the head paints.
    shapes: Vec<(PathData, FillRule)>,
    anchors: usize,
}

impl Parts {
    /// Read `n`, a member of the art (`top`: of its top level) `depth` groups down. `None` when it
    /// is something [`head`] doesn't read.
    fn member(&mut self, n: &Node, top: bool, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        // Hidden or fully transparent art paints nothing.
        if !n.visible || n.opacity <= 0.0 {
            return Some(());
        }
        if !plain(n) {
            return None;
        }
        match &n.kind {
            NodeKind::Group { children, clip: false } => {
                if paints(n) {
                    return None;
                }
                children.iter().try_for_each(|c| self.member(c, false, depth + 1))
            }
            NodeKind::Path { guide: true, .. } => Some(()),
            NodeKind::Path { path, rule, .. } => self.painted(n, path, *rule, top),
            NodeKind::Compound { children, rule } => {
                let mut subpaths = vec![];
                for c in children {
                    subpaths.extend(c.path_data()?.subpaths.iter().cloned());
                }
                self.painted(n, &PathData::new(subpaths), *rule, false)
            }
            _ => None,
        }
    }

    /// Read the fills and strokes of path or compound path `n`, its outline `path` filled under
    /// `rule` (`top`: a member of the art's top level, which may be the sample of the line).
    fn painted(&mut self, n: &Node, path: &PathData, rule: FillRule, top: bool) -> Option<()> {
        self.room(path.anchor_count())?;
        let mut filled = false;
        let mut strokes: Vec<&StrokeLayer> = vec![];
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(f) if f.visible && f.opacity > 0.0 && !f.paint.is_none() => {
                    if !f.effects.is_empty() || f.opacity < 1.0 {
                        return None;
                    }
                    filled = true;
                }
                AppearanceItem::Stroke(s) if s.visible && s.opacity > 0.0 && !s.paint.is_none() && s.width > 0.0 => {
                    if !s.effects.is_empty() || s.brush.is_some() || s.opacity < 1.0 {
                        return None;
                    }
                    strokes.push(s);
                }
                _ => {}
            }
        }
        if top
            && !filled
            && let [line] = strokes.as_slice()
            && let Some(sample) = sample_line(path)
        {
            // The sample is 1 pt, the weight of a head at one unit; another weight (art stored
            // at another scale, as a Large Canvas file's) gives the units no known size.
            if (line.width - 1.0).abs() > 1e-6 {
                return None;
            }
            self.lines.push(sample);
            return Some(());
        }
        if filled {
            self.shapes.push((path.clone(), rule));
        }
        for s in strokes {
            let outline = vectorcraft_effects::stroke::outline_region(path, rule, s);
            self.room(outline.anchor_count())?;
            if !outline.subpaths.is_empty() {
                self.shapes.push((outline, FillRule::NonZero));
            }
        }
        Some(())
    }

    /// Count `anchors` more; `None` past [`MAX_ART_ANCHORS`].
    fn room(&mut self, anchors: usize) -> Option<()> {
        self.anchors = self.anchors.saturating_add(anchors);
        (self.anchors <= MAX_ART_ANCHORS).then_some(())
    }
}

/// Does `n` draw as its shapes alone: fully opaque, with no effects and no opacity mask applied?
/// A head is one opaque shape, so partly transparent or masked art isn't read as one.
fn plain(n: &Node) -> bool {
    n.opacity >= 1.0 && n.appearance.effects.is_empty() && n.extras().mask.as_ref().is_none_or(|m| m.disabled)
}

/// Does `n` (a group) have fills or strokes of its own that paint?
fn paints(n: &Node) -> bool {
    n.appearance.items.iter().any(|i| match i {
        AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
        AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none(),
    })
}

/// `path` as a sample of the line, when it is one open subpath of two anchors or more whose anchors
/// and handles all lie level with its first anchor: that y, and the x where it ends (its
/// rightmost anchor).
fn sample_line(path: &PathData) -> Option<(f64, f64)> {
    let [sub] = path.subpaths.as_slice() else { return None };
    if sub.closed || sub.anchors.len() < 2 {
        return None;
    }
    let y = sub.anchors.first()?.p.y;
    let level = |q: Point| q.x.is_finite() && (q.y - y).abs() <= 1e-6 * (1.0 + y.abs());
    if !y.is_finite() || !sub.anchors.iter().all(|a| level(a.p) && level(a.h_in) && level(a.h_out)) {
        return None;
    }
    Some((y, sub.anchors.iter().map(|a| a.p.x).fold(f64::NEG_INFINITY, f64::max)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Node, NodeId, OpacityMask};
    use vectorcraft_geom::{PathData, Point, SubPath};

    use super::head;

    fn path(id: u64, points: &[(f64, f64)], closed: bool, appearance: Appearance) -> Arc<Node> {
        let points: Vec<Point> = points.iter().map(|(x, y)| Point::new(*x, *y)).collect();
        Arc::new(Node::path(NodeId(id), PathData::single(SubPath::polyline(&points, closed)), appearance))
    }

    /// A 1 pt sample of the line from x = −9 to 4 on y = 0.
    fn sample() -> Arc<Node> {
        path(1, &[(-9.0, 0.0), (4.0, 0.0)], false, Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0))
    }

    /// A filled wedge from x = 2 back to its tip at x = 8, 3 either side.
    fn wedge() -> Arc<Node> {
        path(2, &[(2.0, -3.0), (8.0, 0.0), (2.0, 3.0)], true, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 1.0))
    }

    fn art(members: Vec<Arc<Node>>) -> Node {
        Node::group(NodeId(10), members)
    }

    #[test]
    fn a_wedge_and_its_sample_make_a_head() {
        let h = head("W", &art(vec![sample(), wedge()])).expect("a head");
        assert_eq!(h.stop(), 4.0);
    }

    #[test]
    fn partly_transparent_or_masked_art_makes_no_head() {
        // The head would come in opaque and whole, with the bounds of the art.
        let half = |n: Arc<Node>| Arc::new(Node { opacity: 0.5, ..(*n).clone() });
        let mut faint_fill = (*wedge()).clone();
        let mut ap = (*faint_fill.appearance).clone();
        if let Some(f) = ap.fill_mut() {
            f.opacity = 0.5;
        }
        faint_fill.appearance = ap.into();
        let mut masked = (*wedge()).clone();
        masked.extras_mut().mask = Some(Box::new(OpacityMask { art: sample(), clip: true, invert: false, disabled: false, linked: true }));
        let cases = [
            ("a head at half opacity", art(vec![sample(), half(wedge())])),
            ("a fill at half opacity", art(vec![sample(), Arc::new(faint_fill)])),
            ("a group at half opacity", art(vec![sample(), half(Arc::new(art(vec![wedge()])))])),
            ("the art at half opacity", Node { opacity: 0.5, ..art(vec![sample(), wedge()]) }),
            ("a head with an opacity mask", art(vec![sample(), Arc::new(masked.clone())])),
        ];
        for (why, a) in cases {
            assert!(head("W", &a).is_none(), "{why}");
        }
        // A disabled mask isn't applied.
        masked.extras_mut().mask.as_mut().unwrap().disabled = true;
        assert!(head("W", &art(vec![sample(), Arc::new(masked)])).is_some());
    }
}
