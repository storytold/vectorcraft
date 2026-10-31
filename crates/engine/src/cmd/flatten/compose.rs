use super::analysis::{has_effects, masked, reach};
use super::{BlendMode, Color, FillRule, Knockout, Node, NodeKind, Paint, PathData, Rect, cmyk_planes, composite, effects, node_path, po, shapes};

// ---------- the compositor ----------

/// What an arrangement shape stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    /// Solid fills and clipping paths: regions inside get a flat colour.
    Vector,
    /// A gradient or mesh: regions inside are rasterized at the gradient resolution.
    Gradient,
    /// Anything else that has no single colour (patterns, images, masks, raster effects).
    Other,
}

/// A solid fill as the compositor uses it.
pub(super) struct Fill {
    /// What it composites: its screen colour (twice), or in a CMYK document its two ink planes
    /// ([`cmyk_planes`]).
    pub(super) planes: [[f32; 3]; 2],
    opacity: f32,
    blend: BlendMode,
    pub(super) paint: Paint,
    pub(super) overprint: bool,
}

/// One object of the art being flattened, with the transparency the renderer gives it.
pub(super) struct Elem {
    opacity: f32,
    blend: BlendMode,
    isolate: bool,
    knockout: Knockout,
    knockout_shape: bool,
    /// Blending inside reaches the backdrop ([`Node::blends_through`]).
    blends: bool,
    /// Inside a knockout group its children are elements of their own ([`Node::passes_knockout_through`]).
    pass_through: bool,
    body: Body,
}

enum Body {
    /// Solid fills (bottom first) of arrangement shape `shape`.
    Fills { shape: usize, fills: Vec<Fill> },
    /// A group's children, clipped to arrangement shape `clip` in a clip group.
    Group { clip: Option<usize>, children: Vec<Elem> },
    /// Art without a flat colour: its regions are rasterized, so it never composites here.
    Complex,
}

/// Builds the compositor's objects and the arrangement shapes they cover.
#[derive(Default)]
pub(super) struct Builder {
    pub(super) shapes: Vec<po::Shape>,
    pub(super) kinds: Vec<Kind>,
    /// Building a CMYK document's art.
    pub(super) cmyk: bool,
}

impl Builder {
    /// What colour `c` composites as ([`Fill::planes`]).
    fn planes(&self, c: &Color) -> [[f32; 3]; 2] {
        if self.cmyk {
            let cms = vectorcraft_color::cms::active();
            cmyk_planes(cms.to_cmyk(c, cms.settings().intent))
        } else {
            [c.to_rgb(); 2]
        }
    }

    fn shape(&mut self, path: PathData, rule: FillRule, kind: Kind) -> Option<usize> {
        path.bounds()?;
        self.shapes.push(po::Shape::new(path, rule, self.shapes.len() as u64));
        self.kinds.push(kind);
        Some(self.shapes.len() - 1)
    }

    pub(super) fn elem(&mut self, n: &Node) -> Option<Elem> {
        if !n.visible {
            return None;
        }
        let body = if masked(n) || has_effects(n) {
            self.shape(shapes::rectangle(reach(n)?), FillRule::NonZero, Kind::Other)?;
            Body::Complex
        } else {
            match &n.kind {
                NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
                    let (clip, rest) = if n.clips() {
                        let (first, rest) = children.split_first()?;
                        // Nothing to clip by hides the clipped art.
                        let (bp, rule) = effects::clip_outline(first)?;
                        (Some(self.shape(PathData::from_bezpath(&bp), rule, Kind::Vector)?), rest)
                    } else {
                        (None, &children[..])
                    };
                    Body::Group { clip, children: rest.iter().filter_map(|c| self.elem(c)).collect() }
                }
                NodeKind::Path { guide: true, .. } => return None,
                NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                    let (path, rule) = node_path(n)?;
                    let mut fills = vec![];
                    for item in n.appearance.items.iter().filter(|i| i.is_fill() && i.visible() && !i.paint().is_none()) {
                        match item.paint() {
                            Paint::Solid { color, .. } => fills.push(Fill {
                                planes: self.planes(color),
                                opacity: item.opacity(),
                                blend: item.blend(),
                                paint: item.paint().clone(),
                                overprint: item.overprint(),
                            }),
                            p => {
                                let kind = if matches!(p, Paint::Gradient(_)) { Kind::Gradient } else { Kind::Other };
                                self.shape(path.clone(), rule, kind);
                            }
                        }
                    }
                    if fills.is_empty() { Body::Complex } else { Body::Fills { shape: self.shape(path, rule, Kind::Vector)?, fills } }
                }
                NodeKind::Image(im) => {
                    let frame = shapes::rectangle(Rect::new(0.0, 0.0, im.width as f64, im.height as f64)).transformed(im.xf);
                    self.shape(frame, FillRule::NonZero, Kind::Other)?;
                    Body::Complex
                }
                NodeKind::Mesh(_) => {
                    self.shape(shapes::rectangle(reach(n)?), FillRule::NonZero, Kind::Gradient)?;
                    Body::Complex
                }
                _ => {
                    self.shape(shapes::rectangle(reach(n)?), FillRule::NonZero, Kind::Other)?;
                    Body::Complex
                }
            }
        };
        Some(Elem {
            opacity: n.opacity,
            blend: n.blend,
            isolate: n.isolate,
            knockout: n.knockout,
            knockout_shape: n.knockout_shape,
            blends: n.blends_through(),
            pass_through: n.passes_knockout_through(),
            body,
        })
    }
}

/// A straight (not premultiplied) RGBA colour in 0..=1.
pub(super) type Px = [f32; 4];

pub(super) const CLEAR: Px = [0.0; 4];

pub(super) const WHITE: Px = [1.0; 4];

fn premul(p: Px) -> Px {
    [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]
}

fn unpremul(q: Px) -> Px {
    if q[3] <= 0.0 { CLEAR } else { [q[0] / q[3], q[1] / q[3], q[2] / q[3], q[3].min(1.0)] }
}

/// Composites the objects covering one atomic region exactly as the renderer composites pixels
/// (its transparency groups, non-isolated groups, knockout groups and clip groups): the same
/// paints cover the whole region, so one colour stands for all of it.
pub(super) struct Composer<'a> {
    /// Arrangement shapes covering the region, ascending.
    pub(super) sources: &'a [usize],
    /// The last solid fill composited.
    pub(super) top: Option<&'a Fill>,
    /// Which of the fills' [`Fill::planes`] composites.
    pub(super) plane: usize,
}

impl<'a> Composer<'a> {
    fn inside(&self, shape: usize) -> bool {
        self.sources.binary_search(&shape).is_ok()
    }

    /// `elems` (paint order) composited over `start`.
    pub(super) fn eval(&mut self, elems: &'a [Elem], start: Px) -> Px {
        elems.iter().fold(start, |acc, e| self.draw(e, acc, false, false, false))
    }

    /// `e` drawn over backdrop `b` inside a group that knocks out when `enclosing`. `as_shape`
    /// draws it as its knockout shape (full object opacity); `nested` is drawing inside a layer,
    /// where a non-isolated group can't copy its backdrop and is drawn isolated.
    fn draw(&mut self, e: &'a Elem, b: Px, enclosing: bool, as_shape: bool, nested: bool) -> Px {
        let opacity = if as_shape { 1.0 } else { e.opacity };
        let ko = matches!(e.body, Body::Group { .. }) && e.knockout.resolve(enclosing);
        if opacity >= 1.0 && e.blend == BlendMode::Normal && !e.isolate && !ko {
            return self.content(e, b, ko, None, nested);
        }
        if e.blends && !e.isolate && !nested {
            // Drawn over a copy of the backdrop, so blending inside reaches it.
            let c = self.content(e, b, ko, Some(b), false);
            if e.blend == BlendMode::Normal {
                let (pb, pc) = (premul(b), premul(c));
                return unpremul([0, 1, 2, 3].map(|i| pb[i] + (pc[i] - pb[i]) * opacity));
            }
            // The group's own colour: the backdrop's share taken out by the group's coverage.
            let (pc, pa, pb) = (premul(c), premul(self.content(e, CLEAR, ko, None, false)), premul(b));
            let keep = 1.0 - pa[3];
            let g = [0, 1, 2].map(|i| (pc[i] - keep * pb[i]).clamp(0.0, pa[3]));
            let g = unpremul([g[0], g[1], g[2], pa[3]]);
            return composite(e.blend, b, [g[0], g[1], g[2], g[3] * opacity]);
        }
        let inner_nested = !(e.blends && (e.isolate || !nested));
        let g = self.content(e, CLEAR, ko, None, inner_nested);
        composite(e.blend, b, [g[0], g[1], g[2], g[3] * opacity])
    }

    /// What `e` draws over `b` inside its group (`ko`: its children knock each other out, against
    /// `backdrop` when the group was drawn over a copy of it).
    fn content(&mut self, e: &'a Elem, b: Px, ko: bool, backdrop: Option<Px>, nested: bool) -> Px {
        match &e.body {
            Body::Fills { shape, fills } if self.inside(*shape) => fills.iter().fold(b, |acc, f| {
                self.top = Some(f);
                let [r, g, b] = f.planes[self.plane];
                composite(f.blend, acc, [r, g, b, f.opacity])
            }),
            Body::Group { clip, children } if clip.is_none_or(|c| self.inside(c)) => {
                if ko {
                    self.knockout(children, b, backdrop)
                } else {
                    children.iter().fold(b, |acc, c| self.draw(c, acc, false, false, nested))
                }
            }
            _ => b,
        }
    }

    /// The children of a knockout group over `b`: each first erases what the ones below it drew
    /// by its knockout shape, then adds itself composited against the group's backdrop.
    fn knockout(&mut self, children: &'a [Elem], b: Px, backdrop: Option<Px>) -> Px {
        fn elements<'e>(children: &'e [Elem], out: &mut Vec<&'e Elem>) {
            for c in children {
                match &c.body {
                    Body::Group { children, .. } if c.pass_through => elements(children, out),
                    _ => out.push(c),
                }
            }
        }
        let mut els = vec![];
        elements(children, &mut els);
        let mut acc = premul(b);
        for c in els {
            let s = self.draw(c, CLEAR, true, !c.knockout_shape, true)[3];
            let own = match backdrop {
                Some(b0) => premul(self.draw(c, b0, true, false, true)).map(|v| v * s),
                None => premul(self.draw(c, CLEAR, true, false, true)),
            };
            acc = [0, 1, 2, 3].map(|i| acc[i] * (1.0 - s) + own[i]);
        }
        unpremul(acc)
    }
}
