//! Multi-layer masks in the legacy format (version 7 specification §10): `Mb`, the mask's path
//! painted with `W`, `Md`, the masked objects (in that layer and, past its `LB`, in the layers
//! after it), `MB`. The objects of each layer the mask covers become one clipping group in that
//! layer, its clipping path a copy of the mask's.

use super::types::{Frame, Kind, MAX_NODES};
use super::{Arc, Node, NodeKind, Reader};

/// The note for a file with a multi-layer mask.
pub(crate) const PER_LAYER: &str = "a mask that covers several layers comes in as a clipping group in each layer it covers";

/// A multi-layer mask being read.
#[derive(Debug)]
pub(super) struct MultiMask {
    /// The mask's clipping path, once `Md` has defined it.
    mask: Option<Node>,
    /// What each copy of the mask adds to the objects read: its objects and their points.
    weight: usize,
    /// How many frames were open when the masked objects started in the frame they go in; 0
    /// outside the layers, until a layer starts.
    depth: usize,
    /// Where the masked objects start among that frame's children.
    start: usize,
}

impl Reader<'_> {
    /// A multi-layer mask's operator `w`: `Mb` (or `Mv`) starts the mask, `Md` (or `ME`) defines
    /// it, `MB` ends it, and `Mi` changes nothing here.
    pub(super) fn multi_mask_op(&mut self, w: &str) -> Result<(), String> {
        match w {
            "Mb" | "Mv" => self.begin_multi_mask(),
            "Md" | "ME" => self.define_multi_mask(),
            "MB" => self.end_multi_mask(),
            _ => Ok(()),
        }
    }

    /// `LB`: the layer ends, and a multi-layer mask goes on in the frame around it.
    pub(super) fn end_layer_in_mask(&mut self) -> Result<(), String> {
        self.multi_mask_layer_ends()?;
        self.end_layer()?;
        self.multi_mask_go_on();
        Ok(())
    }

    /// `Mb`: a multi-layer mask starts.
    fn begin_multi_mask(&mut self) -> Result<(), String> {
        if self.multi_mask.is_some() {
            return Err("a multi-layer mask's operators are out of order".into());
        }
        self.end_path_quietly();
        let start = self.frames.last().map_or(0, |f| f.children.len());
        self.multi_mask = Some(MultiMask { mask: None, weight: 0, depth: self.frames.len(), start });
        Ok(())
    }

    /// `Md`: the clipping path painted last since `Mb`, in the same group or layer, is the mask.
    fn define_multi_mask(&mut self) -> Result<(), String> {
        let depth = self.frames.len();
        let Some(m) = self.multi_mask.as_mut() else { return Err("a multi-layer mask's operators are out of order".into()) };
        if m.mask.is_some() || depth == 0 {
            return Err("a multi-layer mask's operators are out of order".into());
        }
        if m.depth != depth {
            return Err("a group crosses a multi-layer mask".into());
        }
        let Some(f) = self.frames.last_mut() else { return Err("a multi-layer mask's operators are out of order".into()) };
        let Some(at) = f.clips.iter().copied().filter(|i| *i >= m.start && *i < f.children.len()).max() else {
            return Err("a multi-layer mask needs one clipping path".into());
        };
        let mask = f.children.remove(at);
        f.clips.retain(|i| *i != at);
        for i in &mut f.clips {
            if *i > at {
                *i -= 1;
            }
        }
        let mut weight = mask.count_with_masks();
        mask.walk(&mut |n| {
            if let NodeKind::Path { path, .. } = &n.kind {
                weight = weight.saturating_add(path.anchor_count());
            }
        });
        m.weight = weight;
        m.mask = Some(mask);
        self.after_object = false;
        Ok(())
    }

    /// `MB`: the masked objects end.
    fn end_multi_mask(&mut self) -> Result<(), String> {
        if !self.multi_mask.as_ref().is_some_and(|m| m.mask.is_some()) {
            return Err("a multi-layer mask's operators are out of order".into());
        }
        self.wrap_masked()?;
        self.multi_mask = None;
        Ok(())
    }

    /// `%AI5_BeginLayer` in a multi-layer mask, before the layer opens: the objects masked so far
    /// in the frame around it are grouped, and the mask goes on in the new layer.
    pub(super) fn multi_mask_layer_starts(&mut self) -> Result<(), String> {
        let Some(m) = self.multi_mask.as_ref() else { return Ok(()) };
        if m.depth != 0 {
            if m.mask.is_none() {
                return Err("a multi-layer mask's operators are out of order".into());
            }
            self.wrap_masked()?;
        }
        Ok(())
    }

    /// A layer opened or closed in a multi-layer mask: the masked objects go on in the frame now
    /// innermost.
    pub(super) fn multi_mask_go_on(&mut self) {
        let (depth, start) = (self.frames.len(), self.frames.last().map_or(0, |f| f.children.len()));
        if let Some(m) = self.multi_mask.as_mut() {
            (m.depth, m.start) = (depth, start);
        }
    }

    /// `LB` in a multi-layer mask, before the layer closes: its masked objects are grouped.
    fn multi_mask_layer_ends(&mut self) -> Result<(), String> {
        let Some(m) = self.multi_mask.as_ref() else { return Ok(()) };
        if m.mask.is_none() {
            return Err("a multi-layer mask's operators are out of order".into());
        }
        if !matches!(self.frames.last(), Some(Frame { kind: Kind::Layer(_), .. })) {
            return Err("a group crosses a multi-layer mask".into());
        }
        self.wrap_masked()
    }

    /// A group, compound path or clipping group closing: it must have opened inside the mask.
    pub(super) fn multi_mask_container_ends(&self) -> Result<(), String> {
        match &self.multi_mask {
            Some(m) if m.depth == self.frames.len() => Err("a group crosses a multi-layer mask".into()),
            _ => Ok(()),
        }
    }

    /// The masked objects in the innermost frame become a clipping group, a copy of the mask first.
    fn wrap_masked(&mut self) -> Result<(), String> {
        let Some(m) = self.multi_mask.as_ref() else { return Ok(()) };
        if m.depth == 0 {
            return Ok(());
        }
        if m.depth != self.frames.len() {
            return Err("a group crosses a multi-layer mask".into());
        }
        let Some(mut mask) = m.mask.clone() else { return Err("a multi-layer mask's operators are out of order".into()) };
        let (start, weight) = (m.start, m.weight);
        let Some(f) = self.frames.last_mut() else { return Ok(()) };
        if start >= f.children.len() {
            return Ok(());
        }
        // A clipping path painted among the masked objects clips the whole layer or group, as
        // outside a mask: it stays in the frame, outside the masked objects' group.
        let painted: std::collections::HashSet<usize> = f.clips.iter().filter_map(|i| i.checked_sub(start)).collect();
        let (clips, kids): (Vec<_>, Vec<_>) = f.children.split_off(start).into_iter().enumerate().partition(|(i, _)| painted.contains(i));
        let (clips, kids): (Vec<Node>, Vec<Node>) = (clips.into_iter().map(|(_, k)| k).collect(), kids.into_iter().map(|(_, k)| k).collect());
        f.clips.retain(|i| *i < start);
        if !kids.is_empty() {
            self.nodes = self.nodes.saturating_add(weight).saturating_add(1);
            if self.nodes > MAX_NODES {
                return Err(format!("it has more than {MAX_NODES} objects"));
            }
            self.reid(&mut mask);
            let children = std::iter::once(mask).chain(kids).map(Arc::new).collect();
            let group = Node::new(self.doc.alloc_id(), NodeKind::Group { children, clip: true });
            if let Some(f) = self.frames.last_mut() {
                f.children.push(group);
            }
            self.warn(PER_LAYER);
        }
        if let Some(f) = self.frames.last_mut() {
            for c in clips {
                f.clips.push(f.children.len());
                f.children.push(c);
            }
        }
        self.after_object = false;
        Ok(())
    }
}
