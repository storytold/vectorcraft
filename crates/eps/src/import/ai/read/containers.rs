use super::document::slot_of;
use super::ops::{paints_apart, path_rule};
use super::types::{Frame, GState, Kind, MAX_DEPTH, MAX_NODES, PendingEnvelope};
use super::{
    AppearanceItem, Arc, BezPath, BlendMode, FillRule, HashMap, HashSet, LayerColor, Node, NodeId, NodeKind, Obj, Paint, PathEl, Rc, Reader, V, obj,
};
use vectorcraft_doc::TextWrap;

impl<'a> Reader<'a> {
    // ---- frames ----------------------------------------------------------------------------

    pub(super) fn open(&mut self, kind: Kind, hidden: bool) -> Result<(), String> {
        if self.frames.len() >= MAX_DEPTH {
            return Err("its objects are nested too deeply".into());
        }
        if matches!(kind, Kind::Layer(_)) {
            self.ensure_space();
            self.check_symbols();
        }
        let base = if matches!(kind, Kind::Obj(_)) { self.stack.len() } else { self.frames.last().map_or(0, |f| f.base) };
        self.frames.push(Frame {
            kind,
            children: Vec::new(),
            hidden,
            after: self.after_object,
            mesh_ended: self.mesh_ended,
            gs: self.gs.clone(),
            pending_unread_fill: self.pending_unread_fill,
            pending_unread_stroke: self.pending_unread_stroke,
            base,
            clips: Vec::new(),
            compound_members: false,
            unread: Vec::new(),
            state: None,
        });
        self.after_object = false;
        Ok(())
    }

    /// Close the innermost frame, which must be of the kind `is` accepts.
    fn close(&mut self, what: &str, is: impl Fn(&Kind) -> bool) -> Result<Frame, String> {
        match self.frames.last() {
            Some(f) if is(&f.kind) => self.frames.pop().ok_or_else(|| format!("`{what}` closes nothing")),
            _ => Err(format!("`{what}` closes something it didn't open")),
        }
    }

    /// Add a node to the innermost container.
    pub(crate) fn add(&mut self, node: Node, hidden: bool) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!("it has more than {MAX_NODES} objects"));
        }
        if hidden {
            self.hidden_ids.insert(node.id);
        }
        match self.frames.last_mut() {
            Some(f) => f.children.push(node),
            None => return Err("it has art outside its layers".into()),
        }
        self.after_object = true;
        Ok(())
    }

    /// The last object read, while nothing else started.
    pub(super) fn last_object(&mut self) -> Option<&mut Node> {
        if !self.after_object {
            return None;
        }
        self.frames.last_mut()?.children.last_mut()
    }

    pub(crate) fn new_node(&mut self, kind: NodeKind, gs: &GState) -> Node {
        let mut n = Node::new(self.doc.alloc_id(), kind);
        n.visible = !gs.hidden;
        n.locked = gs.locked;
        n
    }

    pub(super) fn end_container(&mut self, what: &str, hidden: bool) -> Result<(), String> {
        self.multi_mask_container_ends()?;
        let f = match what {
            "U" => self.close(what, |k| matches!(k, Kind::Group))?,
            "*U" => self.close(what, |k| matches!(k, Kind::Compound))?,
            "*W" => self.close(what, |k| matches!(k, Kind::Wrap))?,
            _ => self.close(what, |k| matches!(k, Kind::Clip))?,
        };
        let Frame {
            kind,
            children: mut kids,
            gs,
            clips,
            hidden: opened_hidden,
            compound_members,
            unread,
            pending_unread_fill,
            pending_unread_stroke,
            ..
        } = f;
        if matches!(kind, Kind::Clip) {
            // `Q` restores the painting state, not the transparency: the writer sets `Xy` only when
            // the next object's differs from the last one written, wherever that was, so a state set
            // inside a clipping group stays after it (a glow set to Screen, then a clipping path set
            // back to Normal: the type after the group has no `Xy` and is Normal).
            let now = &self.gs;
            let transparency = (now.opacity, now.blend, now.isolate, now.knockout, now.knockout_shape);
            self.gs = gs.clone();
            (self.gs.opacity, self.gs.blend, self.gs.isolate, self.gs.knockout, self.gs.knockout_shape) = transparency;
            self.pending_unread_fill = pending_unread_fill;
            self.pending_unread_stroke = pending_unread_stroke;
        }
        // Wraparound text: every object in it other than type is a wrap object, before the text or
        // after it, with no offset (the format gives none) unless its dictionary gives its options.
        // One before the text is below it, so the text flows around it only once it is moved above.
        if matches!(kind, Kind::Wrap) {
            let graphics = kids.iter_mut().filter(|k| !matches!(k.kind, NodeKind::Text(_)) && slot_of(k.name.as_deref()).is_none());
            for k in graphics.filter(|k| k.extras().wrap.is_none()) {
                k.extras_mut().wrap = Some(TextWrap { offset: 0.0, invert: false });
            }
        }
        // A group holding only clipping paths (`q … u <path> W n U … Q`) doesn't clip on its own: `u`/`U` don't save
        // the graphics state as `q`/`Q` do, so its paths clip the clipping group around it, as the file's PDF part draws it.
        if matches!(kind, Kind::Group)
            && unread.is_empty()
            && !clips.is_empty()
            && clips.len() == kids.len()
            && matches!(self.frames.last(), Some(Frame { kind: Kind::Clip, .. }))
        {
            for kid in kids {
                if let Some(parent) = self.frames.last_mut() {
                    parent.clips.push(parent.children.len());
                }
                self.add(kid, opened_hidden || hidden)?;
            }
            return Ok(());
        }
        // A compound path of clipping paths clips its clipping group.
        let clipping = matches!(kind, Kind::Compound) && !clips.is_empty();
        self.group_ended = !matches!(kind, Kind::Compound);
        let mut painted = None;
        let node = match kind {
            // A compound path holding a group (its drawn look) is that look: its paths paint apart.
            // Some files mark every group so, the plain one holding the subpaths too (#980): one
            // paint for all its paths keeps it one compound path, its holes cut.
            Kind::Compound if kids.iter().any(|k| self.looks.contains(&k.id) && paints_apart(k)) => {
                let (children, clip) = self.clip_first(kids, &clips, true, self.nesting() + 1)?;
                self.new_node(NodeKind::Group { children, clip }, &gs)
            }
            Kind::Compound => {
                // A group in a compound path (`*u u … U *U`) only holds its paths.
                let mut kids = ungrouped(kids);
                // A compound clipping path keeps its subpaths' fill when they all paint alike. When they paint
                // differently, the fills of the filled clipping ones are a compound path of their own, under what it clips.
                let look = |n: &Node| (n.appearance.clone(), n.opacity, n.blend);
                if clipping && kids.windows(2).any(|w| look(&w[0]) != look(&w[1])) {
                    let fills: Vec<Node> = kids
                        .iter_mut()
                        .filter(|k| matches!(k.kind, NodeKind::Path { clipping: true, .. }))
                        .filter_map(|k| self.fill_apart(k))
                        .collect();
                    painted = self.compound(fills, &gs);
                }
                let Some(n) = self.compound(kids, &gs) else {
                    if gs.hidden || !self.shown() {
                        self.hidden_unread.extend(unread);
                    } else {
                        self.unsupported.extend(unread);
                    }
                    return Ok(());
                };
                n
            }
            // A group with a clipping path (`q` … `Q`, or a `W` path in a group) is a clipping group.
            _ => {
                let (children, clip) = self.clip_first(kids, &clips, false, self.nesting() + 1)?;
                self.new_node(NodeKind::Group { children, clip }, &gs)
            }
        };
        if clipping && let Some(f) = self.frames.last_mut() {
            f.clips.push(f.children.len());
        }
        if compound_members {
            self.compound_groups.insert(node.id);
        }
        if !unread.is_empty() {
            if !node.visible || !self.shown() {
                self.hidden_unread.extend(unread);
            } else {
                self.unread.push((node.id, unread));
            }
        }
        self.add(node, opened_hidden || hidden)?;
        if let Some(p) = painted {
            self.clip_fills.insert(p.id);
            self.add(p, opened_hidden || hidden)?;
        }
        Ok(())
    }

    /// The compound path of `kids`, painted and made transparent as its subpaths say.
    fn compound(&mut self, mut kids: Vec<Node>, gs: &GState) -> Option<Node> {
        // The compound path's paint is its last path's (a gradient's instance wraps only
        // that one); the last painted one, should the last have none.
        let shown = kids.iter().rev().find(|k| !k.appearance.items.is_empty()).or(kids.last())?;
        let (appearance, opacity, blend, shape, rule) =
            (shown.appearance.clone(), shown.opacity, shown.blend, shown.knockout_shape, path_rule(shown));
        for k in &mut kids {
            k.opacity = 1.0;
            k.blend = BlendMode::Normal;
            k.knockout_shape = false;
            k.visible = true;
            k.locked = false;
        }
        let mut n = self.new_node(NodeKind::Compound { children: kids.into_iter().map(Arc::new).collect(), rule }, gs);
        n.appearance = appearance;
        n.opacity = opacity;
        n.blend = blend;
        n.knockout_shape = shape;
        Some(n)
    }

    /// How deep in the document the objects the open frames hold are, less one: a dictionary adds no level, and an
    /// opacity mask's art is two under the frames around the object it masks.
    fn nesting(&self) -> usize {
        self.frames
            .iter()
            .map(|f| match f.kind {
                Kind::Obj(_) => 0,
                Kind::Mask => 2,
                _ => 1,
            })
            .sum()
    }

    /// `kids` with the clipping paths among them (indexes `clips`) first → the children and whether they clip.
    ///
    /// Several clipping paths in a clipping group or a layer clip to their intersection, as each `W` reduces the clip
    /// set by the ones before it: each clipping path after the first clips a clipping group nested in the one before
    /// it, and the innermost one holds the art. As with one clipping path, every object in the group is clipped by all
    /// of them, wherever it is written. The fill of a compound clipping path read apart from it (`clip_fills`) stays at
    /// its path's level. In a compound path (`union`), several clipping paths are one compound path. `at`: how deep the
    /// group or layer made of them is in the document.
    fn clip_first(&mut self, kids: Vec<Node>, clips: &[usize], union: bool, at: usize) -> Result<(Vec<Arc<Node>>, bool), String> {
        if union || clips.len() < 2 {
            return Ok(self.clip_union(kids, clips));
        }
        let clips: std::collections::HashSet<usize> = clips.iter().copied().collect();
        let (clip, rest): (Vec<Indexed>, Vec<Indexed>) = kids.into_iter().enumerate().partition(|(i, _)| clips.contains(i));
        let innermost = clip.len() - 1;
        // A fill read apart follows its clipping path; the art goes in the innermost level.
        let level = |i: usize, n: &Node| {
            if self.clip_fills.contains(&n.id) { clip.iter().rposition(|(c, _)| *c < i).unwrap_or(0) } else { innermost }
        };
        // Each nested clipping group adds a level over the nodes placed in it, the clipping paths included. Past the
        // reader's limit, or past the document's when the nodes fit in it without those levels (the document would merge
        // the deepest groups and drop their clipping paths), the editing data isn't read.
        let deepest =
            rest.iter().map(|(i, n)| level(*i, n) + depth(n)).chain(clip.iter().enumerate().map(|(k, (_, n))| k + depth(n))).max().unwrap_or(0);
        let flat = rest.iter().map(|(_, n)| depth(n)).chain(clip.iter().map(|(_, n)| depth(n))).max().unwrap_or(0);
        if at + deepest > MAX_DEPTH || (at + deepest > vectorcraft_doc::MAX_DEPTH && at + flat <= vectorcraft_doc::MAX_DEPTH) {
            return Err("its clipping masks are nested too deeply".into());
        }
        // Each level: its clipping path, the fills read apart from it, which paint under the art they clip, then the art.
        let mut fills_at: Vec<Vec<Node>> = clip.iter().map(|_| Vec::new()).collect();
        let mut art_at: Vec<Vec<Node>> = clip.iter().map(|_| Vec::new()).collect();
        for (i, n) in rest {
            let k = level(i, &n);
            let at = if self.clip_fills.contains(&n.id) { &mut fills_at } else { &mut art_at };
            if let Some(v) = at.get_mut(k) {
                v.push(n);
            }
        }
        let mut children: Vec<Arc<Node>> = Vec::new();
        let mut levels = clip.into_iter().map(|(_, n)| n).zip(fills_at.into_iter().zip(art_at)).rev().peekable();
        while let Some((path, (fills, art))) = levels.next() {
            let mut inner: Vec<Arc<Node>> = Vec::with_capacity(fills.len() + art.len() + children.len() + 1);
            inner.push(Arc::new(path));
            inner.extend(fills.into_iter().chain(art).map(Arc::new));
            inner.append(&mut children);
            if levels.peek().is_none() {
                return Ok((inner, true));
            }
            children = vec![Arc::new(Node::new(self.doc.alloc_id(), NodeKind::Group { children: inner, clip: true }))];
        }
        Ok((children, false))
    }

    /// `kids` with the clipping paths among them (indexes `clips`) first, several as one compound
    /// path → the children and whether they clip.
    fn clip_union(&mut self, kids: Vec<Node>, clips: &[usize]) -> (Vec<Arc<Node>>, bool) {
        let clips: std::collections::HashSet<usize> = clips.iter().copied().collect();
        let (mut clip, rest): (Vec<Indexed>, Vec<Indexed>) = kids.into_iter().enumerate().partition(|(i, _)| clips.contains(i));
        let mut fills = Vec::new();
        let clip_node = match clip.len() {
            0 => None,
            1 => clip.pop().map(|(_, n)| n),
            _ => {
                let rule = clip.first().map_or(FillRule::NonZero, |(_, n)| path_rule(n));
                // Each clipping path's fill is a path of its own, under the art it clips, as each one paints it. The
                // compound path keeps the clipping paths' stroke, opacity and blend mode when they all have the same ones,
                // and the knockout shape flag when they all have it: it has one appearance for all its paths.
                for (_, n) in &mut clip {
                    fills.extend(self.fill_apart(n));
                }
                let shape = clip.iter().all(|(_, n)| n.knockout_shape);
                let look = |n: &Node| (n.appearance.clone(), n.opacity, n.blend);
                let shared =
                    clip.split_first().and_then(|((_, first), others)| others.iter().all(|(_, n)| look(n) == look(first)).then(|| look(first)));
                let children = clip.into_iter().map(|(_, n)| Arc::new(n)).collect();
                let mut n = Node::new(self.doc.alloc_id(), NodeKind::Compound { children, rule });
                if let Some((appearance, opacity, blend)) = shared {
                    n.appearance = appearance;
                    n.opacity = opacity;
                    n.blend = blend;
                }
                n.knockout_shape = shape;
                Some(n)
            }
        };
        let clips = clip_node.is_some();
        // A fill read apart paints under the art it clips.
        let (apart, rest): (Vec<Indexed>, Vec<Indexed>) = rest.into_iter().partition(|(_, n)| self.clip_fills.contains(&n.id));
        let children = clip_node.into_iter().chain(fills).chain(apart.into_iter().chain(rest).map(|(_, n)| n)).map(Arc::new).collect();
        (children, clips)
    }

    /// Clipping path `n`'s fills as a path of their own, taken off `n`; `None` when it has no fill. The path keeps `n`'s
    /// transparency, visibility, lock and opacity mask, and none of its other attributes.
    pub(super) fn fill_apart(&mut self, n: &mut Node) -> Option<Node> {
        let is_fill = |i: &AppearanceItem| matches!(i, AppearanceItem::Fill(_));
        if !n.appearance.items.iter().any(is_fill) {
            return None;
        }
        let mut kind = n.kind.clone();
        if let NodeKind::Path { clipping, .. } = &mut kind {
            *clipping = false;
        }
        let mut fill = Node::new(self.doc.alloc_id(), kind);
        // The copies of a path read on `%_` lines are on those lines too, as `add` records them.
        let hidden = self.hidden_ids.contains(&n.id);
        if hidden {
            self.hidden_ids.insert(fill.id);
        }
        // A compound clipping path's subpaths: copies with ids of their own, linked to the same swatches, that don't clip.
        for c in fill.children_mut().into_iter().flatten() {
            let mut copy = self.doc.reid(c);
            self.reid_masks(&mut copy, 0);
            if let NodeKind::Path { clipping, .. } = &mut copy.kind {
                *clipping = false;
            }
            if hidden {
                self.hidden_ids.insert(copy.id);
            }
            *c = Arc::new(copy);
        }
        (fill.opacity, fill.blend, fill.isolate, fill.knockout, fill.knockout_shape, fill.visible, fill.locked) =
            (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape, n.visible, n.locked);
        // The opacity mask's art is a copy with ids of its own, linked to the same swatches.
        if let Some(mut mask) = n.extras().mask.clone() {
            let mut art = self.doc.reid(&mask.art);
            self.reid_masks(&mut art, 0);
            mask.art = Arc::new(art);
            fill.extras_mut().mask = Some(mask);
        }
        let (mut fills, mut rest) = ((*n.appearance).clone(), (*n.appearance).clone());
        fills.items.retain(is_fill);
        rest.items.retain(|i| !is_fill(i));
        fill.appearance = fills.into();
        n.appearance = rest.into();
        // The new path, its subpaths and their points count toward the objects read, as a multi-layer mask's copies do.
        let mut weight = fill.count_with_masks();
        fill.walk(&mut |p| {
            if let NodeKind::Path { path, .. } = &p.kind {
                weight = weight.saturating_add(path.anchor_count());
            }
        });
        self.nodes = self.nodes.saturating_add(weight);
        Some(fill)
    }

    pub(super) fn end_layer(&mut self) -> Result<(), String> {
        self.pending_unread_fill = false;
        self.pending_unread_stroke = false;
        let mut f = self.close("LB", |k| matches!(k, Kind::Layer(_)))?;
        let state = self.layer_state.take();
        self.container_undo = None;
        // An `Xy` right after the layer's last mesh or blend, then `LB`: the layer's, not theirs.
        if let (Some(_), Some(before)) = (state, self.mesh_undo.take())
            && let Some(mesh) = f.children.last_mut().filter(|n| matches!(n.kind, NodeKind::Mesh(_) | NodeKind::Blend { .. }))
        {
            (mesh.opacity, mesh.blend, mesh.isolate, mesh.knockout, mesh.knockout_shape) = before;
        }
        let Kind::Layer(a) = f.kind else { return Ok(()) };
        let (children, clip) = self.clip_first(f.children, &f.clips, false, self.nesting() + 1)?;
        let color = a.color.unwrap_or(LayerColor::Preset(0));
        let mut layer = Node::new(
            self.doc.alloc_id(),
            NodeKind::Layer { color, template: a.template, printable: a.printable, children, clip, preview: a.preview, dim_images: a.dim },
        );
        layer.name = Some(a.name.clone());
        layer.visible = a.visible;
        layer.locked = a.locked;
        // The layer's own opacity, blend and knockout, written after its art (#2003).
        if let Some((opacity, blend, isolate, knockout, shape)) = state {
            (layer.opacity, layer.blend, layer.isolate, layer.knockout, layer.knockout_shape) = (opacity, blend, isolate, knockout, shape);
        }
        match self.frames.last_mut() {
            Some(parent) => {
                parent.children.push(layer);
                self.after_object = true;
            }
            None => {
                self.doc.layers.push(Arc::new(layer));
                self.after_object = false;
            }
        }
        Ok(())
    }

    /// An operator the reader doesn't know is scoped to its top-level object when possible.
    pub(super) fn note_unknown(&mut self, w: &str, hidden: bool) {
        let op: String = w.chars().take(24).collect();
        // A pattern definition or an owning mask can be on `%_` lines while still
        // determining visible art. Unknown operators there must remain unread.
        if (self.gs.hidden && self.pattern.is_none()) || (hidden && self.pattern.is_none() && !self.in_mask()) || !self.shown() {
            if self.hidden_unread.len() < 16 {
                self.hidden_unread.insert(format!("`{op}`"));
            }
        } else if !self.defer_to_mask(&format!("`{op}`")) && !self.mark_unread(format!("`{op}`")) && self.unknown.len() < 16 {
            self.unknown.insert(op);
        }
    }

    /// `:` after `/Type`: a dictionary opens.
    pub(super) fn begin_obj(&mut self, hidden: bool) -> Result<(), String> {
        let ty = match self.stack.pop() {
            Some(V::Name(n)) => n,
            Some(other) => {
                self.stack.push(other);
                String::new()
            }
            None => String::new(),
        };
        self.open(Kind::Obj(Box::new(Obj { ty, ..Obj::default() })), hidden)?;
        // A dictionary's state starts afresh (styles set theirs inside).
        self.gs = GState::default();
        self.pending_unread_fill = false;
        self.pending_unread_stroke = false;
        Ok(())
    }

    /// Capture mask art in document coordinates, without adding it to the owner's siblings.
    pub(super) fn end_mask(&mut self) -> Result<(), String> {
        self.end_path_quietly();
        let f = self.close("X+", |k| matches!(k, Kind::Mask))?;
        // The mask's art is one level under the object it masks.
        let (children, clip) = self.clip_first(f.children, &f.clips, false, self.nesting() + 2)?;
        // One shown object is the art itself. A hidden object or a guide stays in a group: the canvas draws the art's root
        // whether it is shown or not and whether guides show, and a group doesn't draw a hidden member or a left-out guide.
        let art = match children.as_slice() {
            [only] if !clip && only.visible && !matches!(only.kind, NodeKind::Path { guide: true, .. }) => only.clone(),
            _ => Arc::new(self.new_node(NodeKind::Group { children, clip }, &f.gs)),
        };
        self.gs = f.gs;
        self.pending_unread_fill = f.pending_unread_fill;
        self.pending_unread_stroke = f.pending_unread_stroke;
        self.after_object = f.after;
        self.mesh_ended = f.mesh_ended;
        let Some(Frame { kind: Kind::Obj(o), .. }) = self.frames.last_mut() else {
            return Err("mask art ends outside a mask dictionary".into());
        };
        if o.art.replace(art).is_some() {
            return Err("a mask has more than one art block".into());
        }
        o.unread = f.unread;
        Ok(())
    }

    /// `;`: the innermost dictionary closes.
    pub(super) fn end_obj(&mut self) -> Result<(), String> {
        if !matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(_), .. })) {
            return Ok(());
        }
        let Some(f) = self.frames.pop() else { return Ok(()) };
        let Frame { kind, children, base, hidden, gs, pending_unread_fill, pending_unread_stroke, after, mesh_ended, state, .. } = f;
        let Kind::Obj(mut o) = kind else { return Ok(()) };
        obj::close(&mut o, &mut self.stack, base);
        self.obj_hidden = hidden;
        // Art made inside a dictionary (type's paths, a mask's art) isn't kept, and doesn't end what
        // follows the object the dictionary is about: a mesh's state comes after its mask.
        self.gs = gs;
        self.pending_unread_fill = pending_unread_fill;
        self.pending_unread_stroke = pending_unread_stroke;
        if let Some(state) = state {
            self.keep_state(state);
        }
        self.after_object = after;
        self.mesh_ended = mesh_ended;
        if matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(_), .. })) {
            return self.push(V::Obj(Rc::new(*o)));
        }
        match o.ty.as_str() {
            "ArtDictionary" => {
                let envelope = o.get("AIDeformOptionsKey").is_some();
                self.art_dictionary(&o);
                let before = self.envelope_art.take();
                if envelope {
                    let mut slots = vec![];
                    for child in &children {
                        envelope_text_slots(child, &mut slots);
                    }
                    let typed = !slots.is_empty() || before.as_ref().is_some_and(has_text_slot);
                    for mut slot in slots {
                        // The editing data keeps the source text inside the envelope's options;
                        // its visible result is the outlined art the PDF page carries.
                        slot.visible = false;
                        self.add(slot, false)?;
                    }
                    // An envelope of type keeps its drawn art and its type, hidden, as above, until
                    // the type is filled in after reading: then it opens live where it draws as its
                    // drawn art ([`crate::import::ai::envelope::TypedEnvelope`]). In a symbol's or a
                    // pattern's art it keeps them.
                    let waits = !typed || (self.symbol.is_none() && self.pattern.is_none());
                    if !waits || self.envelopes.len() >= crate::import::ai::envelope::MAX_PENDING {
                        self.warn(crate::import::ai::envelope::drawn_note(typed));
                    } else {
                        let siblings = self.frames.last().map_or(0, |f| f.children.len());
                        let depth = self.frames.len();
                        self.envelopes.push(PendingEnvelope { obj: *o, before, inner: children, depth, siblings, typed });
                    }
                }
            }
            "Document" => self.document(&o),
            "AI11Text" => self.text_slot(&o)?,
            "SymbolInstance" => self.symbol_instance(&o)?,
            "ForeignObject" => self.foreign_art(&o)?,
            _ => {}
        }
        Ok(())
    }

    /// Give `n` and its descendants ids of this document, and unlink their swatches.
    pub(super) fn reid(&mut self, n: &mut Node) {
        n.id = self.doc.alloc_id();
        for it in &mut n.appearance.items {
            let p = match it {
                AppearanceItem::Fill(f) => &mut f.paint,
                AppearanceItem::Stroke(s) => &mut s.paint,
            };
            if let Paint::Gradient(g) = p {
                g.swatch = None;
            }
            p.map_links(&mut |_, link, _| {
                *link = None;
                true
            });
        }
        if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
            self.reid(Arc::make_mut(&mut m.art));
        }
        if let Some(children) = n.children_mut() {
            for c in children {
                self.reid(Arc::make_mut(c));
            }
        }
    }

    /// Note something the data has that isn't read: an error on a layer that shows, left out of a
    /// layer that doesn't.
    pub(super) fn unreadable(&mut self, what: &str, hidden: bool) {
        if (!hidden || self.in_mask()) && !self.gs.hidden && self.shown() {
            if !self.defer_to_mask(what) && !self.mark_unread(what.to_string()) {
                self.unsupported.insert(what.into());
            }
        } else {
            self.hidden_unread.insert(what.into());
        }
    }

    /// Keep what an opacity mask's art doesn't read with the mask, until its dictionary says
    /// whether the mask is enabled. Whether art of a mask is being read. Operators it doesn't know
    /// are kept up to 16, as [`Reader::note_unknown`] keeps them.
    fn defer_to_mask(&mut self, what: &str) -> bool {
        let Some(f) = self.frames.iter_mut().rev().find(|f| matches!(f.kind, Kind::Mask)) else { return false };
        let full = what.starts_with('`') && f.unread.iter().filter(|w| w.starts_with('`')).count() >= 16;
        if !full && !f.unread.iter().any(|w| w == what) {
            f.unread.push(what.into());
        }
        true
    }

    /// Mark the first object container below the innermost layer.
    pub(super) fn mark_unread(&mut self, what: String) -> bool {
        let Some(layer) = self.frames.iter().rposition(|f| matches!(f.kind, Kind::Layer(_))) else { return false };
        let Some(frame) = self.frames.get_mut(layer + 1).filter(|f| matches!(f.kind, Kind::Group | Kind::Wrap | Kind::Compound | Kind::Clip)) else {
            return false;
        };
        if !frame.unread.contains(&what) {
            frame.unread.push(what);
        }
        true
    }

    /// Stand in for one unread object with a box that the page can match.
    pub(super) fn unread_box(&mut self, what: &str, bounds: super::Rect, hidden: bool) -> Result<Option<NodeId>, String> {
        if (hidden && !self.in_mask()) || self.gs.hidden || !self.shown() {
            self.hidden_unread.insert(what.into());
            return Ok(None);
        }
        // A mask's art isn't an object of the layers: its object, or the file, is what isn't read.
        if self.in_mask() {
            self.unreadable(what, false);
            return Ok(None);
        }
        let in_container = self.frames.iter().rposition(|f| matches!(f.kind, Kind::Layer(_))).is_some_and(|layer| {
            self.frames.get(layer + 1).is_some_and(|f| matches!(f.kind, Kind::Group | Kind::Wrap | Kind::Compound | Kind::Clip))
        });
        if !super::art::sane(bounds) || bounds.area() <= 0.0 {
            self.unsupported.insert(what.into());
            return Ok(None);
        }
        let path = vectorcraft_geom::shapes::rectangle(bounds);
        let gs = self.gs.clone();
        let node = self.new_node(NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false }, &gs);
        let id = node.id;
        if in_container {
            self.mark_unread(what.into());
        }
        self.add(node, hidden)?;
        if !in_container {
            self.unread.push((id, vec![what.into()]));
        }
        Ok(Some(id))
    }

    /// A symbol instance (`/SymbolInstance`): the symbol its `SymbolRef` names, placed by its
    /// `RTransformMatrix`, hidden and locked as the state says. In a symbol's art the matrix is
    /// written about the canvas point of art space's origin, and the name is the file's until the
    /// symbols are checked ([`Self::check_symbols`]). An instance of a symbol that wasn't read, one
    /// whose appearance differs from its symbol's, or one placed only by a `Matrix` or `Transform`,
    /// comes in as the page draws it, within the symbol's box placed by the matrix when the box is
    /// known.
    fn symbol_instance(&mut self, o: &Obj) -> Result<(), String> {
        let name = self.names.entry(o, "SymbolRef");
        let affine = |n: &[f64]| match n {
            [a, b, c, d, e, f] if [a, b, c, d, e, f].iter().all(|v| v.is_finite()) => Some(super::Affine::new([*a, *b, *c, *d, *e, *f])),
            _ => None,
        };
        let live = affine(&o.nums("RTransformMatrix"));
        let boxed_only = live.is_none();
        let matrix = live.or_else(|| {
            o.get("Matrix")
                .or_else(|| o.get("Transform"))
                .map(|v| v.iter().find_map(|item| if let V::Arr(a) = item { Some(Self::nums(a)) } else { None }).unwrap_or_else(|| Self::nums(v)))
                .and_then(|n| affine(&n))
        });
        let hidden = self.obj_hidden;
        let place = if self.symbol.is_some() {
            self.canvas_origin().map(|o| super::Affine::new([1.0, 0.0, 0.0, -1.0, o.x, o.y]))
        } else {
            Some(self.space())
        };
        let (Some(name), Some(m), Some(place)) = (name, matrix, place) else {
            self.unreadable("symbols", hidden);
            return Ok(());
        };
        let xf = place * m;
        let usable = xf.determinant().abs() >= 1e-12 && xf.as_coeffs().iter().all(|v| v.is_finite());
        let symbol = if self.symbol.is_some() { Some(name.clone()) } else { self.symbol_names.get(&name).cloned() };
        match symbol.filter(|_| usable && !boxed_only && !overrides(o)) {
            Some(symbol) => {
                // Its transparency is the `Xy` after it, as a group's is.
                let gs = self.gs.clone();
                let node = self.new_node(NodeKind::SymbolInstance { symbol, xf }, &gs);
                self.add(node, hidden)
            }
            None => {
                let bounds = self.symbol_art_bounds(&name).or_else(|| self.symbol_bounds.get(&name).copied());
                match bounds.map(|b| xf.transform_rect_bbox(b)).filter(|_| usable && self.symbol.is_none()) {
                    Some(b) => self.unread_box("symbols", b, hidden).map(|_| ()),
                    None => {
                        self.unreadable("symbols", hidden);
                        Ok(())
                    }
                }
            }
        }
    }

    /// The art box (symbol space) of the symbol read under `name` (its name in the file), measured
    /// once.
    fn symbol_art_bounds(&mut self, name: &str) -> Option<super::Rect> {
        if let Some(b) = self.symbol_art_bounds.get(name) {
            return *b;
        }
        let named = self.symbol_names.get(name)?;
        let boxes = self.doc.symbol_boxes();
        let b = self.doc.symbols.iter().find(|s| &s.name == named).and_then(|s| s.art.visual_bounds_with(&boxes));
        drop(boxes);
        self.symbol_art_bounds.insert(name.to_string(), b);
        b
    }

    /// Check the symbols read for instances in their art of symbols the file doesn't have, before
    /// the first layer: a symbol with one isn't kept, and so on for the symbols holding its
    /// instances. The instances in the art kept then name the symbols as the document does.
    pub(super) fn check_symbols(&mut self) {
        if std::mem::replace(&mut self.symbols_checked, true) {
            return;
        }
        // Each symbol's name in the file (none for one named as an earlier one), and the names its
        // instances use.
        let files: HashMap<&str, &str> = self.symbol_names.iter().map(|(file, named)| (named.as_str(), file.as_str())).collect();
        let defs: Vec<(Option<String>, Vec<String>)> = self
            .doc
            .symbols
            .iter()
            .map(|s| {
                let mut uses = vec![];
                symbol_uses(&s.art, &mut uses, 0);
                (files.get(s.name.as_str()).map(|f| f.to_string()), uses.into_iter().map(str::to_string).collect())
            })
            .collect();
        // A symbol goes when one of its instances names a symbol that isn't kept: first those
        // naming one the file doesn't have, then, one at a time, those using a symbol that went.
        let mut kept: Vec<bool> = vec![true; defs.len()];
        let mut users: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut going: Vec<usize> = vec![];
        for (i, (_, uses)) in defs.iter().enumerate() {
            for u in uses {
                users.entry(u.as_str()).or_default().push(i);
            }
            if uses.iter().any(|u| !self.symbol_names.contains_key(u)) {
                going.push(i);
            }
        }
        while let Some(i) = going.pop() {
            match kept.get_mut(i) {
                Some(k) if *k => *k = false,
                _ => continue,
            }
            if let Some(file) = defs.get(i).and_then(|(file, _)| file.as_deref()) {
                going.extend(users.get(file).into_iter().flatten().copied().filter(|j| kept.get(*j).copied().unwrap_or(false)));
            }
        }
        drop(users);
        if kept.iter().any(|k| !k) {
            let left: Vec<(Option<String>, String)> = defs
                .iter()
                .zip(&self.doc.symbols)
                .zip(&kept)
                .filter(|(_, k)| !**k)
                .map(|(((file, _), s), _)| (file.clone(), s.name.clone()))
                .collect();
            let mut i = 0;
            self.doc.symbols.retain(|_| {
                i += 1;
                kept.get(i - 1).copied().unwrap_or(false)
            });
            // Their art's box isn't known (the instances that made them go have none), so their
            // instances have no box either.
            for (file, named) in left {
                self.doc.set_symbol_registration(&named, vectorcraft_doc::SymbolRegistration::CENTER);
                let Some(file) = file else { continue };
                self.symbol_names.remove(&file);
            }
        }
        let names = self.symbol_names.clone();
        for s in &mut self.doc.symbols {
            rename_uses(Arc::make_mut(&mut s.art), &names, 0);
        }
    }

    /// Drop the object just read when it is an envelope's own: a group on `%_` lines, before the
    /// envelope's dictionary. It holds the mesh that shapes the envelope, or, while the envelope's
    /// contents are being edited, those contents (the mesh then is in the dictionary); either way the
    /// distorted art follows on plain lines. The source text in the contents stays, hidden, as the
    /// dictionary's does. The group is kept in [`Reader::envelope_art`] for the envelope to be made
    /// of. Whether it was dropped.
    pub(super) fn drop_envelope_art(&mut self) -> bool {
        let Some(n) = self.last_object() else { return false };
        let id = n.id;
        if !matches!(n.kind, NodeKind::Group { .. }) || !self.hidden_ids.contains(&id) {
            return false;
        }
        let mut slots = vec![];
        if let Some(f) = self.frames.last_mut()
            && let Some(group) = f.children.pop()
        {
            envelope_text_slots(&group, &mut slots);
            for mut slot in slots {
                slot.visible = false;
                f.children.push(slot);
            }
            self.envelope_art = Some(group);
        }
        self.hidden_ids.remove(&id);
        self.after_object = false;
        true
    }

    /// Do the layers being read show?
    pub(super) fn shown(&self) -> bool {
        self.frames.iter().all(|f| !matches!(&f.kind, Kind::Layer(a) if !a.visible))
    }

    /// Is the art being read an opacity mask's? It is on `%_` lines and still shapes what shows.
    pub(super) fn in_mask(&self) -> bool {
        self.frames.iter().any(|f| matches!(f.kind, Kind::Mask))
    }

    /// A blend's record read: the blend's own art (the group of its key objects and spine, on `%_`
    /// lines) and its drawn steps (the group after it) become one live blend, or, where they can't,
    /// the drawn steps stay, without the blend's own art drawn over them (see [`super::blend`]).
    pub(super) fn end_blend(&mut self) {
        let Some((_, hex)) = self.blend_record.take() else { return };
        let record = crate::import::ai::blend::hex_bytes(&hex).as_deref().and_then(crate::import::ai::blend::record);
        let roles = &self.blend_roles;
        let Some(f) = self.frames.last_mut() else { return };
        let n = f.children.len();
        let (Some(at), Some(drawn)) = (n.checked_sub(2), n.checked_sub(1)) else { return };
        let part_of_blend = |node: &Node| node.children().is_some_and(|c| c.iter().any(|k| roles.contains_key(&k.id)));
        let (Some(edit), Some(steps)) = (f.children.get(at), f.children.get(drawn)) else { return };
        if !part_of_blend(edit) {
            return;
        }
        if !matches!(steps.kind, NodeKind::Group { clip: false, .. }) {
            // The steps aren't a group of drawn steps: the blend's own art goes all the same, so
            // its keys and spine aren't drawn over what the file draws.
            f.children.remove(at);
            self.warn(crate::import::ai::blend::DRAWN);
            return;
        }
        let mut budget = self.blend_budget;
        let live = record.and_then(|record| crate::import::ai::blend::live_blend(edit, steps, &record, roles, &mut budget));
        self.blend_budget = budget;
        let Some(mut steps) = f.children.pop() else { return };
        let Some(_edit) = f.children.pop() else { return };
        match live {
            Some(kind) => {
                // The steps the blend lays out count as the objects they draw, as its drawn steps
                // did (they were counted as read).
                let mut blend = Node::new(self.doc.alloc_id(), kind);
                (blend.name, blend.visible, blend.locked) = (steps.name.take(), steps.visible, steps.locked);
                (blend.opacity, blend.blend, blend.isolate, blend.knockout, blend.knockout_shape) =
                    (steps.opacity, steps.blend, steps.isolate, steps.knockout, steps.knockout_shape);
                // Its note, URL and data (object tags) too.
                blend.extras_mut().attrs = steps.extras_mut().attrs.take();
                f.children.push(blend);
            }
            None => {
                f.children.push(steps);
                self.warn(crate::import::ai::blend::DRAWN);
            }
        }
        // The blend's own state block follows its record, as a group's follows its end.
        self.after_object = true;
    }

    /// An envelope's plugin `XP`: the envelope whose dictionary was read last in this container
    /// and its drawn art, the one object read since, become one live envelope, or, where they
    /// can't, the drawn art stays (see [`crate::import::ai::envelope`]). Envelopes left waiting
    /// in containers that closed keep their drawn art.
    pub(super) fn end_envelope(&mut self) {
        let depth = self.frames.len();
        while let Some(p) = self.envelopes.pop_if(|p| p.depth > depth) {
            self.warn(crate::import::ai::envelope::drawn_note(p.typed));
        }
        if self.envelopes.last().is_none_or(|p| p.depth != depth) {
            return;
        }
        let Some(p) = self.envelopes.pop() else { return };
        let drawn = self.frames.last().filter(|f| f.children.len() == p.siblings + 1).and_then(|f| f.children.last());
        let Some(drawn) = drawn else {
            self.warn(crate::import::ai::envelope::drawn_note(p.typed));
            return;
        };
        if p.typed {
            self.wait_for_type(p);
            return;
        }
        let live = (self.envelope_checks > 0).then(|| crate::import::ai::envelope::live(&p.obj, p.before, p.inner)).flatten();
        let Some(kind) = live else {
            self.warn(crate::import::ai::envelope::DRAWN);
            return;
        };
        // Objects the reader couldn't read come in as the page's art in their place: the envelope's
        // drawn art stays round them.
        let mut ids = HashSet::new();
        fn collect(n: &Node, ids: &mut HashSet<NodeId>) {
            ids.insert(n.id);
            for c in n.children().into_iter().flatten() {
                collect(c, ids);
            }
        }
        collect(drawn, &mut ids);
        if let NodeKind::Envelope { content, .. } = &kind {
            content.iter().for_each(|c| collect(c, &mut ids));
        }
        if self.unread.iter().any(|(id, _)| ids.contains(id)) {
            self.warn(crate::import::ai::envelope::DRAWN);
            return;
        }
        self.envelope_checks -= 1;
        // The drawn art's own properties (name, visibility, lock, transparency, appearance, mask,
        // attributes) become the envelope's.
        let mut envelope = drawn.clone();
        (envelope.id, envelope.kind) = (self.doc.alloc_id(), kind);
        let layer = self.doc.alloc_id();
        let shared = crate::import::ai::envelope::Shared { patterns: &self.doc.patterns, images: &self.doc.images };
        let mut shown = envelope.clone();
        (shown.visible, shown.opacity) = (true, 1.0);
        let mut plain = drawn.clone();
        (plain.visible, plain.opacity) = (true, 1.0);
        let page = self.page_in_doc();
        if !crate::import::ai::envelope::drawn_alike(&shown, &plain, layer, &shared, page, &mut self.envelope_spent) {
            self.warn(crate::import::ai::envelope::DRAWN);
            return;
        }
        // The contents are the envelope's own now, not a copy on `%_` lines.
        fn forget(n: &Node, hidden: &mut HashSet<NodeId>) {
            hidden.remove(&n.id);
            for c in n.children().into_iter().flatten() {
                forget(c, hidden);
            }
        }
        forget(&envelope, &mut self.hidden_ids);
        if let Some(f) = self.frames.last_mut()
            && let Some(slot) = f.children.last_mut()
        {
            let id = envelope.id;
            let drawn = std::mem::replace(slot, envelope);
            self.envelopes_drawn.push((id, drawn));
        }
    }
}

impl Reader<'_> {
    /// An envelope of type `p` whose drawn art was just read (the last object of this container):
    /// it waits for its type to be laid out ([`crate::import::ai::envelope::TypedEnvelope`]), unless
    /// objects the reader couldn't read are in it, or too many already wait. Then it keeps its
    /// drawn art and its type, hidden.
    fn wait_for_type(&mut self, p: PendingEnvelope) {
        let Some(drawn) = self.frames.last().and_then(|f| f.children.last()) else { return };
        let mut ids = HashSet::new();
        fn collect(n: &Node, ids: &mut HashSet<NodeId>) {
            ids.insert(n.id);
            for c in n.children().into_iter().flatten() {
                collect(c, ids);
            }
        }
        collect(drawn, &mut ids);
        p.before.iter().chain(&p.inner).for_each(|n| collect(n, &mut ids));
        let unread = self.unread.iter().any(|(id, _)| ids.contains(id));
        if unread || self.typed_envelopes.len() >= crate::import::ai::envelope::MAX_CHECKED {
            self.warn(crate::import::ai::envelope::TYPE_DRAWN);
            return;
        }
        let drawn = drawn.id;
        self.typed_envelopes.push(crate::import::ai::envelope::TypedEnvelope { obj: p.obj, before: p.before, inner: p.inner, drawn });
    }
}

/// Does `node` hold a text slot?
fn has_text_slot(node: &Node) -> bool {
    let mut slots = vec![];
    envelope_text_slots(node, &mut slots);
    !slots.is_empty()
}

/// The source text inside an envelope options dictionary, carried through its containing metadata
/// (not type that a look draws, which isn't the envelope's source text).
fn envelope_text_slots(node: &Node, out: &mut Vec<Node>) {
    if super::is_look_slot(node.name.as_deref()) {
        return;
    }
    if slot_of(node.name.as_deref()).is_some() {
        out.push(node.clone());
    } else if let Some(children) = node.children() {
        for child in children {
            envelope_text_slots(child, out);
        }
    }
}

/// The text objects written twice, plainly and on `%_` lines: the commented copy (`hidden`) of a
/// story that also has a plain one goes.
pub(super) fn drop_commented_copies(layers: &mut [Arc<Node>], hidden: &HashSet<NodeId>) {
    fn plain(nodes: &[Arc<Node>], hidden: &HashSet<NodeId>, out: &mut HashSet<u32>) {
        for n in nodes {
            match slot_of(n.name.as_deref()) {
                Some(Some(story)) if !hidden.contains(&n.id) => {
                    out.insert(story);
                }
                _ => {
                    if let Some(c) = n.children() {
                        plain(c, hidden, out);
                    }
                }
            }
        }
    }
    fn drop(nodes: &mut Vec<Arc<Node>>, hidden: &HashSet<NodeId>, plain: &HashSet<u32>) {
        nodes.retain(|n| !(hidden.contains(&n.id) && slot_of(n.name.as_deref()).flatten().is_some_and(|s| plain.contains(&s))));
        for n in nodes.iter_mut() {
            if n.children().is_some_and(|c| !c.is_empty())
                && let Some(c) = Arc::make_mut(n).children_mut()
            {
                drop(c, hidden, plain);
            }
        }
    }
    let mut stories = HashSet::new();
    plain(layers, hidden, &mut stories);
    if stories.is_empty() {
        return;
    }
    for l in layers.iter_mut() {
        if let Some(c) = Arc::make_mut(l).children_mut() {
            drop(c, hidden, &stories);
        }
    }
}

/// A node and its index among its siblings.
type Indexed = (usize, Node);

/// The names of the symbols the instances in `node`, and in the art of its opacity masks, use.
fn symbol_uses<'n>(node: &'n Node, out: &mut Vec<&'n str>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    if let NodeKind::SymbolInstance { symbol, .. } = &node.kind {
        out.push(symbol);
    }
    if let Some(m) = &node.extras().mask {
        symbol_uses(&m.art, out, depth + 1);
    }
    for c in node.children().into_iter().flatten() {
        symbol_uses(c, out, depth + 1);
    }
}

/// The instances in `node`, and in the art of its opacity masks, named by `names` (a name in the
/// file → the symbol's in the document).
fn rename_uses(node: &mut Node, names: &HashMap<String, String>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    if let NodeKind::SymbolInstance { symbol, .. } = &mut node.kind
        && let Some(named) = names.get(symbol.as_str())
    {
        symbol.clone_from(named);
    }
    if let Some(m) = node.extras_if_any().and_then(|e| e.mask.as_mut()) {
        rename_uses(Arc::make_mut(&mut m.art), names, depth + 1);
    }
    for c in node.children_mut().into_iter().flatten() {
        rename_uses(Arc::make_mut(c), names, depth + 1);
    }
}

/// Whether a symbol instance's dictionary carries appearance of its own (a dynamic symbol's
/// edits), which isn't read.
fn overrides(o: &Obj) -> bool {
    const KEYS: [&str; 5] = [
        "AI19SymbolAppearanceMap",
        "AI19SymbolOverriddenSymbolMap",
        "AI Local Art of Symbol Instance",
        "AI15 Layer In Symbol",
        "AI15 Color Of Layer In Symbol",
    ];
    // In a dictionary or array nested in it, as a keyed entry's value, in an array or among the
    // values closed without a key.
    fn has(o: &Obj, depth: usize) -> bool {
        depth <= 8
            && (o.entries.iter().any(|(k, v)| k.as_deref().is_some_and(|k| KEYS.contains(&k)) || v.iter().any(|v| in_value(v, depth + 1)))
                || o.content.iter().any(|v| in_value(v, depth + 1)))
    }
    fn in_value(v: &V, depth: usize) -> bool {
        match v {
            V::Obj(c) => has(c, depth),
            V::Arr(items) => depth <= 8 && items.iter().any(|v| in_value(v, depth + 1)),
            _ => false,
        }
    }
    has(o, 0)
}

/// Close the subpath `bp` ends with, if it has one open.
pub(super) fn close(bp: &mut BezPath) {
    if bp.elements().last().is_some_and(|e| !matches!(e, PathEl::ClosePath)) {
        bp.close_path();
    }
}

/// How many levels of nodes `n` holds, itself included, its opacity masks' art included.
fn depth(n: &Node) -> usize {
    let mut deepest = 0;
    let mut todo = vec![(n, 1)];
    while let Some((n, d)) = todo.pop() {
        deepest = deepest.max(d);
        todo.extend(n.children().into_iter().flatten().map(|c| (c.as_ref(), d + 1)));
        if let Some(m) = n.extras().mask.as_ref() {
            todo.push((m.art.as_ref(), d + 1));
        }
    }
    deepest
}

/// The fill rule of a path or compound node.
/// `kids` with the members of each plain group among them in the group's place, at any depth.
fn ungrouped(kids: Vec<Node>) -> Vec<Node> {
    let mut out = Vec::with_capacity(kids.len());
    for k in kids {
        match k.kind {
            NodeKind::Group { children, clip: false } => out.extend(ungrouped(children.into_iter().map(Arc::unwrap_or_clone).collect())),
            _ => out.push(k),
        }
    }
    out
}
