use super::*;

/// Deepest node tree mapped (the reader bounds trees well below this).
pub(super) const MAX_DEPTH: usize = 160;

pub(super) fn rect_of(r: af::Rect) -> Rect {
    Rect::new(r.x0, r.y0, r.x1, r.y1)
}

pub(super) fn affine(a: af::Affine) -> Affine {
    Affine::new(a.0)
}

impl Builder {
    pub(super) fn layer_with(&mut self, name: Option<&str>, children: Vec<Arc<Node>>) -> Arc<Node> {
        let id = self.doc.alloc_id();
        self.layer = self.layer.wrapping_add(1);
        let label = name.map(str::to_string).unwrap_or_else(|| format!("Layer {}", self.layer));
        let mut n = Node::layer(id, &label, LayerColor::Preset(self.layer % 9));
        if let Some(c) = n.children_mut() {
            *c = children;
        }
        Arc::new(n)
    }

    /// An artboard at the top level becomes a layer of its name holding its clip group.
    pub(super) fn layer_of(&mut self, name: &str, art: Arc<Node>, visible: bool, locked: bool) -> Arc<Node> {
        let mut l = (*self.layer_with(Some(if name.is_empty() { "Artboard" } else { name }), vec![art])).clone();
        l.visible = visible;
        l.locked = locked;
        Arc::new(l)
    }

    pub(super) fn base(&mut self, n: &af::Node, kind: NodeKind) -> Node {
        let mut out = Node::new(self.doc.alloc_id(), kind);
        out.name = (!n.name.is_empty()).then(|| n.name.clone());
        out.visible = n.visible;
        out.locked = n.locked;
        out.opacity = n.opacity as f32;
        out.blend = self.blend(n.blend);
        out
    }

    fn blend(&mut self, b: Blend) -> BlendMode {
        match b {
            Blend::Normal | Blend::PassThrough => BlendMode::Normal,
            Blend::Darken => BlendMode::Darken,
            Blend::Multiply => BlendMode::Multiply,
            Blend::ColorBurn => BlendMode::ColorBurn,
            Blend::Lighten => BlendMode::Lighten,
            Blend::Screen => BlendMode::Screen,
            Blend::ColorDodge => BlendMode::ColorDodge,
            Blend::Overlay => BlendMode::Overlay,
            Blend::SoftLight => BlendMode::SoftLight,
            Blend::HardLight => BlendMode::HardLight,
            Blend::Difference => BlendMode::Difference,
            Blend::Exclusion => BlendMode::Exclusion,
            Blend::Hue => BlendMode::Hue,
            Blend::Saturation => BlendMode::Saturation,
            Blend::Color => BlendMode::Color,
            Blend::Luminosity => BlendMode::Luminosity,
            _ => {
                self.warn("blend modes VectorCraft doesn't have (Add, Linear Light, Vivid Light, Pin Light, Hard Mix, Subtract, Darker/Lighter Colour, Average, Negation, Reflect, Glow, Erase) import as Normal");
                BlendMode::Normal
            }
        }
    }

    /// `n` with its colour and gradient overlays baked into its paint and pixels (VectorCraft has
    /// no overlay effect).
    fn overlaid<'n>(&mut self, n: &'n af::Node) -> std::borrow::Cow<'n, af::Node> {
        // Each kind of message once, however often it is heard.
        let mut warnings: Vec<String> = Vec::new();
        let mut work = self.overlay_work;
        let out = vectorcraft_affinity::overlay::apply(n, &mut work, &mut |w| {
            if !warnings.iter().any(|x| x == w) {
                warnings.push(w.to_string());
            }
        });
        self.overlay_work = work;
        for w in warnings {
            self.warn(&w);
        }
        out
    }

    /// One Affinity node (and its subtree) as VectorCraft art; `to_doc` maps document pixels to
    /// points. `layer_level`: the node is directly in a spread or a layer, so a layer stays a layer.
    pub(super) fn node(
        &mut self,
        n: &af::Node,
        to_doc: Affine,
        boards: &mut Vec<(String, Rect)>,
        layer_level: bool,
        depth: usize,
    ) -> Option<Arc<Node>> {
        if depth > MAX_DEPTH {
            self.warn("objects nested too deeply (left out)");
            return None;
        }
        // VectorCraft documents stay within `MAX_COORD` of the origin (renderers and exports rely
        // on it): objects reaching beyond it are left out.
        if let Some(b) = af::kind_bounds(&n.kind) {
            let r = to_doc.transform_rect_bbox(rect_of(b));
            if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= crate::MAX_COORD) {
                self.warn("objects beyond the document range VectorCraft supports (left out)");
                return None;
            }
        }
        let n = &*self.overlaid(n);
        // Register before descending so nested artboards keep their document order. Inside an
        // embedded document an artboard is just its content's frame.
        if let Kind::Artboard { rect, .. } = &n.kind
            && self.embedded == 0
        {
            boards.push((
                if n.name.is_empty() { format!("Artboard {}", boards.len() + 1) } else { n.name.clone() },
                to_doc.transform_rect_bbox(rect_of(*rect)),
            ));
        }
        let child_level = matches!(n.kind, Kind::Layer) && layer_level;
        let embeds = n.class == vectorcraft_affinity::stream::Tag::of(b"EmbN");
        self.embedded += usize::from(embeds);
        let children: Vec<Arc<Node>> = n.children.iter().filter_map(|c| self.node(c, to_doc, boards, child_level, depth + 1)).collect();
        self.embedded -= usize::from(embeds);
        let mut out = match &n.kind {
            Kind::Layer if layer_level => {
                let mut l = (*self.layer_with(Some(if n.name.is_empty() { "Layer" } else { &n.name }), children)).clone();
                l.visible = n.visible;
                l.locked = n.locked;
                l.opacity = n.opacity as f32;
                l.blend = self.blend(n.blend);
                l
            }
            Kind::Layer | Kind::Group => {
                let mut g = self.base(n, NodeKind::Group { children, clip: false });
                // Affinity groups isolate their blending unless they pass through.
                g.isolate = n.blend != Blend::PassThrough;
                g
            }
            Kind::Shape { path, fills, strokes, even_odd } => {
                let data = self.path(path, to_doc);
                let rule = if *even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
                let mut appearance = self.appearance(fills, strokes, to_doc);
                // An outline effect on a shape is one more stroke.
                self.outline_stroke(&n.effects, &mut appearance);
                if children.is_empty() {
                    let mut p = self.base(n, NodeKind::Path { path: data, rule, live: None, clipping: false, guide: false });
                    p.appearance = appearance.into();
                    p
                } else {
                    // A shape clips its children: VectorCraft's clipping path keeps its paint, its
                    // fill behind the clipped art and its stroke over it, as Affinity draws it.
                    let mut clip = Node::new(self.doc.alloc_id(), NodeKind::Path { path: data, rule, live: None, clipping: true, guide: false });
                    clip.appearance = appearance.into();
                    let mut kids = vec![Arc::new(clip)];
                    kids.extend(children);
                    self.base(n, NodeKind::Group { children: kids, clip: true })
                }
            }
            Kind::Artboard { path, even_odd, background, strokes, .. } => {
                let mut clip = Node::new(
                    self.doc.alloc_id(),
                    NodeKind::Path {
                        path: self.path(path, to_doc),
                        rule: if *even_odd { FillRule::EvenOdd } else { FillRule::NonZero },
                        live: None,
                        clipping: true,
                        guide: false,
                    },
                );
                clip.appearance = self.appearance(background, strokes, to_doc).into();
                let mut kids = vec![Arc::new(clip)];
                kids.extend(children);
                let mut g = self.base(n, NodeKind::Group { children: kids, clip: true });
                g.name = Some(if n.name.is_empty() { "Artboard".into() } else { n.name.clone() });
                g
            }
            Kind::Text(t) => {
                let Some((mut text, alpha)) = self.text(t, to_doc) else {
                    // The text is left out (warned); what it holds still comes in.
                    if children.is_empty() {
                        return None;
                    }
                    let g = self.base(n, NodeKind::Group { children, clip: false });
                    return Some(Arc::new(g));
                };
                self.outline_text(&n.effects, &mut text);
                let mut node = self.base(n, NodeKind::Text(Box::new(text)));
                node.opacity *= alpha;
                if !children.is_empty() {
                    let mut kids = vec![Arc::new(node)];
                    kids.extend(children);
                    node = Node::group(self.doc.alloc_id(), kids);
                }
                node
            }
            Kind::Table(t) => {
                let mut kids = self.table(t, to_doc);
                if kids.is_empty() && children.is_empty() {
                    return None;
                }
                kids.extend(children);
                let mut g = self.base(n, NodeKind::Group { children: kids, clip: false });
                g.name.get_or_insert_with(|| "Table".into());
                g
            }
            Kind::Image(img) => match self.image(img, to_doc) {
                Some(kind) => self.base(n, kind),
                None if children.is_empty() => return None,
                None => self.base(n, NodeKind::Group { children, clip: false }),
            },
            // Shown on the pages through the master's layer (see `master_pages`).
            Kind::MasterInstance { .. } => return None,
            Kind::Unsupported if children.is_empty() => return None,
            Kind::Unsupported => self.base(n, NodeKind::Group { children, clip: false }),
        };
        if !matches!(n.kind, Kind::Shape { .. } | Kind::Text(_)) && n.effects.iter().any(|e| matches!(e, af::Effect::Outline { .. })) {
            self.warn("outline effects on groups, images and tables (left out)");
        }
        if let Some(img) = &n.pixel_mask {
            // A pixel mask: its grey levels show (white) or hide (black) the node, and nothing
            // shows outside it, as an opacity mask that clips.
            if let Some(kind) = self.image(img, to_doc) {
                let art = Node::new(self.doc.alloc_id(), kind);
                out.extras_mut().mask = Some(Box::new(OpacityMask::new(art, true)));
            }
        }
        if let Some(mask) = &n.mask {
            // A vector mask: the node shows only inside it.
            let clip = Node::new(
                self.doc.alloc_id(),
                NodeKind::Path { path: self.path(mask, to_doc), rule: FillRule::NonZero, live: None, clipping: true, guide: false },
            );
            let (opacity, blend, visible, name) = (out.opacity, out.blend, out.visible, out.name.clone());
            out.opacity = 1.0;
            out.blend = BlendMode::Normal;
            let mut g = Node::new(self.doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(out)], clip: true });
            (g.opacity, g.blend, g.visible, g.name) = (opacity, blend, visible, name);
            out = g;
        }
        // Shadows and glows apply to what the node shows, mask and all.
        let fx = self.effects(&n.effects);
        if !fx.is_empty() {
            out.appearance.effects.extend(fx);
        }
        Some(Arc::new(out))
    }
}
