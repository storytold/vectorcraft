use super::kind::NodeKind;
use super::{BezPath, FillRule, Node, Outliner, Rect, shapes};

/// Unites filled regions (each under its own fill rule) into one path filled non-zero. Booleans
/// live above this crate (`vectorcraft-pathops`), so callers supply it.
pub type Uniter<'a> = &'a dyn Fn(&[(BezPath, FillRule)]) -> BezPath;

impl Node {
    /// The filled regions this object clips to as the clipping path of a clip group, in document
    /// space, each with its fill rule: a path, a compound path (with its holes), an image's frame,
    /// text outlined by `text` (glyph outlines need the font engine, above this crate), the visible
    /// members of a group (their union) and the clipping path of a nested clip group. Live objects
    /// are evaluated first. Guides and symbol instances add nothing.
    pub fn clip_shapes(&self, text: Outliner) -> Vec<(BezPath, FillRule)> {
        let mut out = vec![];
        self.push_clip_shapes(text, &mut out);
        out
    }

    fn push_clip_shapes(&self, text: Outliner, out: &mut Vec<(BezPath, FillRule)>) {
        match &self.kind {
            NodeKind::Path { guide: true, .. } | NodeKind::RulerGuide { .. } | NodeKind::SymbolInstance { .. } => {}
            NodeKind::Path { path, rule, .. } => out.push((path.to_bezpath(), *rule)),
            NodeKind::Compound { children, rule } => {
                let mut bp = BezPath::new();
                for p in children.iter().filter_map(|c| c.path_data()) {
                    bp.extend(p.to_bezpath());
                }
                out.push((bp, *rule));
            }
            NodeKind::Group { children, clip: true } => {
                if let Some(c) = children.first() {
                    c.push_clip_shapes(text, out);
                }
            }
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
                for c in children.iter().skip(usize::from(self.extras().shaper.is_some())).filter(|c| c.visible) {
                    c.push_clip_shapes(text, out);
                }
            }
            NodeKind::Text(_) => {
                if let Some(o) = text.and_then(|f| f(self)) {
                    o.push_clip_shapes(None, out);
                }
            }
            NodeKind::Image(im) => {
                // A distorted image's frame joins its corners.
                let frame = match self.image_frame().filter(|_| self.is_distorted_image()) {
                    Some(bp) => bp,
                    None => shapes::rectangle(Rect::new(0.0, 0.0, im.width as f64, im.height as f64)).transformed(im.xf).to_bezpath(),
                };
                out.push((frame, FillRule::NonZero));
            }
            NodeKind::PlacedDocument(p) => {
                let frame = shapes::rectangle(p.natural()).transformed(p.xf);
                out.push((frame.to_bezpath(), FillRule::NonZero));
            }
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::Intertwine { .. } => {
                crate::live::expand_deep(self, text).push_clip_shapes(text, out);
            }
        }
    }

    /// The region this object clips to as one path and fill rule ([`Self::clip_shapes`]), shared
    /// by the renderer and the SVG and PDF writers so every output clips alike. One shape keeps its
    /// own rule; several are united by `unite` (filled non-zero). `None` when there is nothing to
    /// clip by: the clipped art is then hidden.
    pub fn clip_outline(&self, text: Outliner, unite: Uniter) -> Option<(BezPath, FillRule)> {
        let mut shapes = self.clip_shapes(text);
        match shapes.len() {
            0 => None,
            1 => shapes.pop(),
            _ => Some((unite(&shapes), FillRule::NonZero)),
        }
    }

    /// What this object paints as the clipping path of a clip group, shared by the renderer and
    /// the SVG and PDF writers: its fills, painted behind the clipped art, and its strokes, painted
    /// over it (unclipped). Each part is this object keeping only those appearance items (type:
    /// also only its characters' fills or strokes); `None` where it paints nothing, as right after
    /// Make Clipping Mask. Only paths, compound paths and type paint as clipping paths, and so does
    /// a group of the pieces a clipping path's fills and strokes draw ([`crate::NodeExtras::clip_pieces`]),
    /// even after effects around it split or copy those pieces into groups: each part is then that
    /// group keeping only the pieces of those fills or strokes.
    pub fn clip_paint(&self) -> ClipPaint {
        use crate::appearance::AppearanceItem;
        let item = |fill: bool, i: &AppearanceItem| match i {
            AppearanceItem::Fill(f) => fill && f.visible && !f.paint.is_none(),
            AppearanceItem::Stroke(s) => !fill && s.visible && !s.paint.is_none() && s.width > 0.0,
        };
        if matches!(self.kind, NodeKind::Group { clip: false, .. }) && self.extras().clip_pieces {
            let part = |fill: bool| {
                let mut paints = false;
                let mut n = pieces_of(self, fill, &|i| item(fill, i), &mut paints)?;
                n.extras_mut().clip_pieces = false;
                paints.then_some(n)
            };
            return ClipPaint { fill: part(true), stroke: part(false) };
        }
        let runs = match &self.kind {
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => None,
            NodeKind::Text(t) => Some(&t.runs),
            _ => return ClipPaint::default(),
        };
        let paints = |fill: bool| {
            let item = |i: &AppearanceItem| item(fill, i);
            let run = |r: &crate::text::TextRun| if fill { !r.style.fill.is_none() } else { !r.style.stroke.is_none() && r.style.stroke_width > 0.0 };
            self.appearance.items.iter().any(item) || runs.is_some_and(|rs| rs.iter().any(run))
        };
        let part = |fill: bool| {
            paints(fill).then(|| {
                let mut n = self.clone();
                for i in (0..n.appearance.items.len()).rev() {
                    if matches!(n.appearance.items[i], AppearanceItem::Fill(_)) != fill {
                        n.appearance.remove_item(i);
                    }
                }
                if let NodeKind::Text(t) = &mut n.kind {
                    for r in &mut t.runs {
                        *(if fill { &mut r.style.stroke } else { &mut r.style.fill }) = vectorcraft_color::Paint::None;
                    }
                }
                n
            })
        };
        ClipPaint { fill: part(true), stroke: part(false) }
    }
}

/// `n`, a group of a clipping path's pieces or one of them, keeping only the pieces of its fills
/// (`fill`) or of its strokes, at every depth of the groups an effect put them in; `None` when it
/// holds none. `paints` is set when one of the kept fills or strokes paints.
fn pieces_of(n: &Node, fill: bool, paints_item: &dyn Fn(&crate::appearance::AppearanceItem) -> bool, paints: &mut bool) -> Option<Node> {
    use crate::appearance::AppearanceItem;
    if let NodeKind::Group { children, clip: false } = &n.kind {
        let kept: Vec<_> = children.iter().filter_map(|c| pieces_of(c, fill, paints_item, paints).map(std::sync::Arc::new)).collect();
        if kept.is_empty() {
            return None;
        }
        let mut m = n.clone();
        if let Some(ch) = m.children_mut() {
            *ch = kept;
        }
        return Some(m);
    }
    let items = &n.appearance.items;
    if !items.iter().any(|i| matches!(i, AppearanceItem::Fill(_)) == fill) {
        return None;
    }
    *paints |= items.iter().any(paints_item);
    Some(n.clone())
}

/// What a clipping path paints ([`Node::clip_paint`]).
#[derive(Clone, Debug, Default)]
pub struct ClipPaint {
    /// The clipping path with only its fills: painted behind the clipped art (inside the clip).
    pub fill: Option<Node>,
    /// The clipping path with only its strokes: painted over the clipped art, not clipped.
    pub stroke: Option<Node>,
}
