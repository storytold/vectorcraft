use super::document::slot_of;
use super::ops::{paints_apart, path_rule};
use super::types::{Frame, GState, Kind, MAX_DEPTH, MAX_NODES};
use super::{AppearanceItem, Arc, BezPath, BlendMode, FillRule, HashSet, LayerColor, Node, NodeId, NodeKind, Obj, Paint, PathEl, Rc, Reader, V, obj};

impl<'a> Reader<'a> {
    // ---- frames ----------------------------------------------------------------------------

    pub(super) fn open(&mut self, kind: Kind, hidden: bool) -> Result<(), String> {
        if self.frames.len() >= MAX_DEPTH {
            return Err("its objects are nested too deeply".into());
        }
        if matches!(kind, Kind::Layer(_)) {
            self.ensure_space();
        }
        let base = if matches!(kind, Kind::Obj(_)) { self.stack.len() } else { self.frames.last().map_or(0, |f| f.base) };
        self.frames.push(Frame {
            kind,
            children: Vec::new(),
            hidden,
            after: self.after_object,
            mesh_ended: self.mesh_ended,
            gs: self.gs.clone(),
            base,
            clips: Vec::new(),
            compound_members: false,
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
        let f = match what {
            "U" => self.close(what, |k| matches!(k, Kind::Group))?,
            "*U" => self.close(what, |k| matches!(k, Kind::Compound))?,
            _ => self.close(what, |k| matches!(k, Kind::Clip))?,
        };
        let Frame { kind, children: kids, gs, clips, hidden: opened_hidden, compound_members, .. } = f;
        // A group holding only clipping paths (`q … u <path> W n U … Q`) doesn't clip on its own: `u`/`U` don't save
        // the graphics state as `q`/`Q` do, so its paths clip the clipping group around it, as the file's PDF part draws it.
        if matches!(kind, Kind::Group)
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
                let (children, clip) = self.clip_first(kids, &clips);
                self.new_node(NodeKind::Group { children, clip }, &gs)
            }
            Kind::Compound => {
                // A group in a compound path (`*u u … U *U`) only holds its paths. Clipping subpaths'
                // painted copies make a compound of their own, under what it clips.
                let (fills, kids): (Vec<Node>, Vec<Node>) = ungrouped(kids).into_iter().partition(|n| self.clip_fills.contains(&n.id));
                painted = self.compound(fills, &gs);
                let Some(n) = self.compound(kids, &gs) else { return Ok(()) };
                n
            }
            // A group with a clipping path (`q` … `Q`, or a `W` path in a group) is a clipping group.
            _ => {
                let (children, clip) = self.clip_first(kids, &clips);
                self.new_node(NodeKind::Group { children, clip }, &gs)
            }
        };
        if clipping && let Some(f) = self.frames.last_mut() {
            f.clips.push(f.children.len());
        }
        if compound_members {
            self.compound_groups.insert(node.id);
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
        let (appearance, opacity, blend, rule) = (shown.appearance.clone(), shown.opacity, shown.blend, path_rule(shown));
        for k in &mut kids {
            k.opacity = 1.0;
            k.blend = BlendMode::Normal;
            k.visible = true;
            k.locked = false;
        }
        let mut n = self.new_node(NodeKind::Compound { children: kids.into_iter().map(Arc::new).collect(), rule }, gs);
        n.appearance = appearance;
        n.opacity = opacity;
        n.blend = blend;
        Some(n)
    }

    /// `kids` with the clipping paths among them (indexes `clips`) first, several as one compound
    /// path → the children and whether they clip.
    fn clip_first(&mut self, kids: Vec<Node>, clips: &[usize]) -> (Vec<Arc<Node>>, bool) {
        let clips: std::collections::HashSet<usize> = clips.iter().copied().collect();
        let (mut clip, rest): (Vec<Indexed>, Vec<Indexed>) = kids.into_iter().enumerate().partition(|(i, _)| clips.contains(i));
        let clip_node = match clip.len() {
            0 => None,
            1 => clip.pop().map(|(_, n)| n),
            _ => {
                let rule = clip.first().map_or(FillRule::NonZero, |(_, n)| path_rule(n));
                let children = clip.into_iter().map(|(_, n)| Arc::new(n)).collect();
                Some(Node::new(self.doc.alloc_id(), NodeKind::Compound { children, rule }))
            }
        };
        let clips = clip_node.is_some();
        // A clipping path's own fill paints under the art it clips.
        let (fills, rest): (Vec<Indexed>, Vec<Indexed>) = rest.into_iter().partition(|(_, n)| self.clip_fills.contains(&n.id));
        let children = clip_node.into_iter().chain(fills.into_iter().chain(rest).map(|(_, n)| n)).map(Arc::new).collect();
        (children, clips)
    }

    pub(super) fn end_layer(&mut self) -> Result<(), String> {
        let mut f = self.close("LB", |k| matches!(k, Kind::Layer(_)))?;
        let state = self.layer_state.take();
        // An `Xy` right after the layer's last mesh or blend, then `LB`: the layer's, not theirs.
        if let (Some(_), Some(before)) = (state, self.mesh_undo.take())
            && let Some(mesh) = f.children.last_mut().filter(|n| matches!(n.kind, NodeKind::Mesh(_) | NodeKind::Blend { .. }))
        {
            (mesh.opacity, mesh.blend, mesh.isolate, mesh.knockout) = before;
        }
        let Kind::Layer(a) = f.kind else { return Ok(()) };
        let (children, clip) = self.clip_first(f.children, &f.clips);
        let color = a.color.unwrap_or(LayerColor::Preset(0));
        let mut layer = Node::new(
            self.doc.alloc_id(),
            NodeKind::Layer { color, template: false, printable: a.printable, children, clip, preview: a.preview, dim_images: a.dim },
        );
        layer.name = Some(a.name.clone());
        layer.visible = a.visible;
        layer.locked = a.locked;
        // The layer's own opacity, blend and knockout, written after its art (#2003).
        if let Some((opacity, blend, isolate, knockout)) = state {
            (layer.opacity, layer.blend, layer.isolate, layer.knockout) = (opacity, blend, isolate, knockout);
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

    /// An operator the reader doesn't know: on hidden art it is only noted, on shown art the file
    /// is read from its PDF part.
    pub(super) fn note_unknown(&mut self, w: &str) {
        let op: String = w.chars().take(24).collect();
        if !self.shown() {
            if self.hidden_unread.len() < 16 {
                self.hidden_unread.insert(format!("`{op}`"));
            }
        } else if self.unknown.len() < 16 {
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
        Ok(())
    }

    /// Capture mask art in document coordinates, without adding it to the owner's siblings.
    pub(super) fn end_mask(&mut self) -> Result<(), String> {
        self.end_path_quietly();
        let f = self.close("X+", |k| matches!(k, Kind::Mask))?;
        let (children, clip) = self.clip_first(f.children, &f.clips);
        let art = self.new_node(NodeKind::Group { children, clip }, &f.gs);
        self.gs = f.gs;
        self.after_object = f.after;
        self.mesh_ended = f.mesh_ended;
        let Some(Frame { kind: Kind::Obj(o), .. }) = self.frames.last_mut() else {
            return Err("mask art ends outside a mask dictionary".into());
        };
        if o.art.replace(Arc::new(art)).is_some() {
            return Err("a mask has more than one art block".into());
        }
        Ok(())
    }

    /// `;`: the innermost dictionary closes.
    pub(super) fn end_obj(&mut self) -> Result<(), String> {
        if !matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(_), .. })) {
            return Ok(());
        }
        let Some(f) = self.frames.pop() else { return Ok(()) };
        let Frame { kind, children, base, hidden, gs, after, mesh_ended, .. } = f;
        let Kind::Obj(mut o) = kind else { return Ok(()) };
        obj::close(&mut o, &mut self.stack, base);
        self.obj_hidden = hidden;
        // Art made inside a dictionary (type's paths, a mask's art) isn't kept, and doesn't end what
        // follows the object the dictionary is about: a mesh's state comes after its mask.
        self.gs = gs;
        self.after_object = after;
        self.mesh_ended = mesh_ended;
        if matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(_), .. })) {
            return self.push(V::Obj(Rc::new(*o)));
        }
        match o.ty.as_str() {
            "ArtDictionary" => {
                let envelope = o.get("AIDeformOptionsKey").is_some();
                self.art_dictionary(&o);
                if envelope {
                    let mut slots = vec![];
                    for child in &children {
                        envelope_text_slots(child, &mut slots);
                    }
                    for mut slot in slots {
                        // The editing data keeps the source text inside the envelope's options;
                        // its visible result is the outlined art the PDF page carries.
                        slot.visible = false;
                        self.add(slot, false)?;
                    }
                }
            }
            "Document" => self.document(&o),
            "AI11Text" => self.text_slot(&o)?,
            "SymbolInstance" => self.unreadable("symbols"),
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
    pub(super) fn unreadable(&mut self, what: &str) {
        if self.shown() {
            self.unsupported.insert(what.into());
        } else {
            self.hidden_unread.insert(what.into());
        }
    }

    /// Drop the object just read when it is an envelope's own mesh: a group on `%_` lines holding
    /// only gradient meshes. Whether it was dropped.
    pub(super) fn drop_envelope_mesh(&mut self) -> bool {
        fn only_meshes(n: &Node) -> bool {
            match &n.kind {
                NodeKind::Mesh(_) => true,
                NodeKind::Group { children, .. } => !children.is_empty() && children.iter().all(|c| only_meshes(c)),
                _ => false,
            }
        }
        let Some(n) = self.last_object() else { return false };
        let id = n.id;
        if !matches!(n.kind, NodeKind::Group { .. }) || !only_meshes(n) || !self.hidden_ids.contains(&id) {
            return false;
        }
        if let Some(f) = self.frames.last_mut() {
            f.children.pop();
        }
        self.hidden_ids.remove(&id);
        self.after_object = false;
        true
    }

    /// Do the layers being read show?
    fn shown(&self) -> bool {
        self.frames.iter().all(|f| !matches!(&f.kind, Kind::Layer(a) if !a.visible))
    }

    /// A blend's record read: the blend's own art (the group of its key objects and spine, on `%_`
    /// lines) and its drawn steps (the group after it) become one live blend, or, where they can't,
    /// the drawn steps stay, without the blend's own art drawn over them (see [`super::blend`]).
    pub(super) fn end_blend(&mut self) {
        let Some((_, hex)) = self.blend_record.take() else { return };
        let ids = crate::import::ai::blend::hex_bytes(&hex).as_deref().and_then(crate::import::ai::blend::record_ids);
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
        let live = ids.and_then(|ids| crate::import::ai::blend::live_blend(edit, steps, &ids, roles, &mut budget));
        self.blend_budget = budget;
        let Some(mut steps) = f.children.pop() else { return };
        let Some(_edit) = f.children.pop() else { return };
        match live {
            Some(kind) => {
                // The steps the blend lays out count as the objects they draw, as its drawn steps
                // did (they were counted as read).
                let mut blend = Node::new(self.doc.alloc_id(), kind);
                (blend.name, blend.visible, blend.locked) = (steps.name.take(), steps.visible, steps.locked);
                (blend.opacity, blend.blend, blend.isolate, blend.knockout) = (steps.opacity, steps.blend, steps.isolate, steps.knockout);
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
}

/// The source text inside an envelope options dictionary, carried through its containing metadata.
fn envelope_text_slots(node: &Node, out: &mut Vec<Node>) {
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

/// Close the subpath `bp` ends with, if it has one open.
pub(super) fn close(bp: &mut BezPath) {
    if bp.elements().last().is_some_and(|e| !matches!(e, PathEl::ClosePath)) {
        bp.close_path();
    }
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
