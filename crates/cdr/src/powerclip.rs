//! PowerClip resource references, verified with original CDR 15/16/27 drawings.
//!
//! `doc / clpt / grp ` stores the contents under its `spnd` identifier. A frame's
//! argument 8005 names that group; the group's argument 3500 names the frame's
//! `spnd`. Contents already carry their complete transforms, including rotated
//! frames. This is distinct from bitmap crop geometry in argument 30.

use super::{Drawing, arguments, metadata};
use crate::{container::Chunk, reader::Reader};
use std::sync::Arc;
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Affine;

impl<'a> Drawing<'a> {
    /// Index borrowed clipping resources without materializing their artwork.
    pub(super) fn index_clips(&mut self, chunks: &'a [Chunk]) -> Result<(), String> {
        for chunk in chunks {
            if chunk.list.as_ref() == Some(b"obj ")
                && let Some(loda) = metadata(chunk, b"loda").or_else(|| metadata(chunk, b"lobj"))
            {
                let (_, args) = arguments(&loda.data)?;
                if let Some(reference) = args.get(&8005)
                    && Reader::new(reference).u32()? != 0
                {
                    let owner = chunk.child(b"spnd").ok_or("CDR PowerClip frame is missing its identifier")?;
                    let owner = Reader::new(&owner.data).u32()?;
                    // External LIST references materialize exact aliases. Compare the
                    // entire borrowed definition, retaining the first; distinct frames
                    // sharing an identifier remain ambiguous. Expansion has its own quotas.
                    if owner == 0 || self.clip_owners.get(&owner).is_some_and(|first| *first != chunk) {
                        return Err("duplicate or invalid CDR PowerClip frame identifier".into());
                    }
                    self.clip_owners.entry(owner).or_insert(chunk);
                }
            }
            if chunk.list.as_ref() == Some(b"clpt") {
                for group in &chunk.children {
                    if group.list.as_ref() != Some(b"grp ") {
                        continue;
                    }
                    let Some(id) = group.child(b"spnd") else { continue };
                    let id = Reader::new(&id.data).u32()?;
                    if id == 0 || self.clips.get(&id).is_some_and(|first| *first != group) {
                        return Err("invalid or duplicate CDR PowerClip resource identifier".into());
                    }
                    self.clips.entry(id).or_insert(group);
                }
            }
            self.index_clips(&chunk.children)?;
        }
        Ok(())
    }

    /// Attach the referenced editable contents to an already decoded vector frame.
    /// The frame's fill paints below the contents and its outline above them.
    pub(super) fn powerclip(&mut self, source: &Chunk, mut frame: Node, reference: &[u8], parent: Affine) -> Result<Node, String> {
        let mut r = Reader::new(reference);
        let id = r.u32()?;
        if id == 0 {
            return Ok(frame);
        }
        if r.u32()? != 1 || r.u32()? != 0 || r.u32()? != 0 || r.pos != reference.len() {
            return Err("unsupported CDR PowerClip reference settings".into());
        }
        let group = self.clips.get(&id).copied().ok_or("unresolved CDR PowerClip contents")?;
        let owner = source.child(b"spnd").ok_or("CDR PowerClip frame is missing its identifier")?;
        let owner = Reader::new(&owner.data).u32()?;
        let loda = metadata(group, b"loda").or_else(|| metadata(group, b"lobj")).ok_or("CDR PowerClip resource is missing its owner")?;
        let (_, args) = arguments(&loda.data)?;
        let target = args.get(&3500).ok_or("CDR PowerClip resource is missing its owner reference")?;
        if Reader::new(target).u32()? != owner {
            return Err("CDR PowerClip owner and contents references disagree".into());
        }
        if !self.active_clips.insert(id) {
            return Err("cyclic CDR PowerClip reference".into());
        }
        // The common traversal depth/visit quotas also cover graph expansion,
        // including many empty or aliased resources that consume no point quota.
        let contents = self.children(&group.children, parent);
        self.active_clips.remove(&id);
        let contents = contents?;
        let NodeKind::Path { clipping, .. } = &mut frame.kind else {
            return Err("unsupported CDR PowerClip frame geometry".into());
        };
        *clipping = true;
        let name = frame.name.take();
        let mut children = Vec::with_capacity(contents.len() + 1);
        children.push(Arc::new(frame));
        children.extend(contents);
        let mut node = Node::new(self.id(), NodeKind::Group { children, clip: true });
        node.name = name;
        Ok(node)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_geom::{FillRule, Shape};

    /// Decode an original generated container, including resources outside pages.
    fn fixture() -> Vec<Chunk> {
        crate::container::load(&crate::synth::powerclip()).unwrap().0
    }

    /// Find the resource pool in this original fixture.
    fn pool(chunks: &mut [Chunk]) -> &mut Vec<Chunk> {
        &mut chunks.iter_mut().find(|c| c.list.as_ref() == Some(b"doc ")).unwrap().children[0].children
    }

    /// Replace or add a generated argument, rebuilding its offset and reversed-type tables.
    fn argument(chunk: &mut Chunk, id: u32, value: Vec<u8>) {
        let loda = chunk.children.iter_mut().find(|c| c.id == *b"loda").unwrap();
        let (kind, args) = arguments(&loda.data).unwrap();
        let mut args: std::collections::BTreeMap<_, _> = args.into_iter().map(|(id, data)| (id, data.to_vec())).collect();
        args.insert(id, value);
        let count = args.len() as u32;
        let table = 20 + args.values().map(|data| data.len() as u32).sum::<u32>();
        let mut data: Vec<u8> = [table + count * 8, count, table, table + count * 4, kind].into_iter().flat_map(u32::to_le_bytes).collect();
        let mut offsets = Vec::new();
        for value in args.values() {
            offsets.push(data.len() as u32);
            data.extend(value);
        }
        data.extend(offsets.into_iter().flat_map(u32::to_le_bytes));
        data.extend(args.keys().rev().flat_map(|id| id.to_le_bytes()));
        loda.data = data;
    }

    /// Two identical references per level form a compact acyclic expansion graph.
    fn branching_graph(name: Option<&[u8]>) -> Vec<Chunk> {
        let mut chunks = fixture();
        let groups = pool(&mut chunks);
        groups.clear();
        for level in 0..15u32 {
            let mut template = fixture();
            let mut group = pool(&mut template).remove(0);
            group.children.iter_mut().find(|c| c.id == *b"spnd").unwrap().data = (8 + level).to_le_bytes().to_vec();
            argument(&mut group, 3500, (10 + level).to_le_bytes().to_vec());
            if level < 14 {
                group.children.retain(|c| c.list.as_ref() != Some(b"obj "));
                for _ in 0..2 {
                    let mut template = fixture();
                    let page = template.iter_mut().find(|c| c.list.as_ref() == Some(b"page")).unwrap();
                    let mut frame = page.children.remove(1);
                    frame.children.iter_mut().find(|c| c.id == *b"spnd").unwrap().data = (11 + level).to_le_bytes().to_vec();
                    argument(&mut frame, 8005, [9 + level, 1, 0, 0].into_iter().flat_map(u32::to_le_bytes).collect());
                    if let Some(name) = name {
                        argument(&mut frame, 1000, name.to_vec());
                    }
                    group.children.push(frame);
                }
            }
            groups.push(group);
        }
        chunks
    }

    /// The mask remains painted geometry, and both mask and oversized content remain editable.
    #[test]
    fn referenced_contents_retain_paths_paint_and_hole_rule() {
        let imported = crate::drawing::import(&fixture(), 1200).unwrap();
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
        assert!(imported.document.images.is_empty());
        let clip = &imported.document.layers[0].children().unwrap()[0];
        assert!(clip.clips());
        let nodes = clip.children().unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(matches!(nodes[0].kind, NodeKind::Path { rule: FillRule::EvenOdd, clipping: true, .. }));
        assert_eq!(nodes[0].appearance.fill_paint(), Paint::solid(Color::rgb8(220, 40, 70)));
        assert_eq!(nodes[1].appearance.fill_paint(), Paint::solid(Color::rgb8(20, 100, 230)));
        assert_eq!(nodes[0].path_data().unwrap().subpaths.len(), 2);
    }

    /// A dangling reference cannot quietly replace a populated frame with its bare outline.
    #[test]
    fn missing_or_mismatched_contents_fail_explicitly() {
        let mut chunks = fixture();
        pool(&mut chunks).clear();
        assert!(crate::drawing::import(&chunks, 1200).unwrap_err().contains("unresolved CDR PowerClip"));
        let mut chunks = fixture();
        let owner = pool(&mut chunks)[0].children.iter_mut().find(|c| c.id == *b"loda").unwrap();
        owner.data[20..24].copy_from_slice(&99u32.to_le_bytes());
        assert!(crate::drawing::import(&chunks, 1200).unwrap_err().contains("references disagree"));
    }

    /// Resource recursion must fail before it can exhaust the call stack or materialize art.
    #[test]
    fn cyclic_powerclip_resources_are_rejected() {
        let mut other = fixture();
        let page = other.iter_mut().find(|c| c.list.as_ref() == Some(b"page")).unwrap();
        let frame = page.children.remove(1);
        let mut chunks = fixture();
        // Start traversal directly at the resource's owning frame to exercise the runtime guard.
        let source = chunks.iter_mut().find(|c| c.list.as_ref() == Some(b"page")).unwrap().children.remove(1);
        pool(&mut chunks)[0].children.push(frame);
        let mut drawing = Drawing::new(1200);
        drawing.index_clips(&chunks).unwrap();
        assert!(drawing.object(&source, Affine::IDENTITY).unwrap_err().contains("cyclic CDR PowerClip"));
    }

    /// Exact aliases retain the first borrowed definitions and still draw on every page.
    #[test]
    fn exact_frame_and_resource_aliases_retain_each_occurrence() {
        let mut chunks = fixture();
        let mut other = fixture();
        let duplicate_group = pool(&mut other).remove(0);
        pool(&mut chunks).push(duplicate_group);
        let duplicate_page = other.into_iter().find(|c| c.list.as_ref() == Some(b"page")).unwrap();
        chunks.push(duplicate_page);
        let mut drawing = Drawing::new(1200);
        drawing.index_clips(&chunks).unwrap();
        let first_page = chunks.iter().find(|c| c.list.as_ref() == Some(b"page")).unwrap();
        let first_group = &chunks.iter().find(|c| c.list.as_ref() == Some(b"doc ")).unwrap().children[0].children[0];
        assert_eq!(drawing.clip_owners.len(), 1);
        assert_eq!(drawing.clips.len(), 1);
        assert!(std::ptr::eq(*drawing.clip_owners.get(&10).unwrap(), &first_page.children[1]));
        assert!(std::ptr::eq(*drawing.clips.get(&8).unwrap(), first_group));
        let imported = crate::drawing::import(&chunks, 1200).unwrap();
        assert!(imported.warnings.is_empty());
        assert_eq!(imported.document.layers.len(), 2);
        let clips: Vec<_> = imported.document.layers.iter().map(|layer| &layer.children().unwrap()[0]).collect();
        assert!(clips.iter().all(|clip| clip.clips() && clip.children().unwrap().len() == 2));
        for (left, right) in clips[0].children().unwrap().iter().zip(clips[1].children().unwrap()) {
            let left = left.path_data().unwrap().to_bezpath().bounding_box();
            let right = right.path_data().unwrap().to_bezpath().bounding_box();
            let offset = imported.document.artboards[1].rect.x0;
            assert!((right.x0 - left.x0 - offset).abs() < 1e-8);
        }
    }

    /// Ambiguous group identifiers cannot bind to whichever resource happens to be last.
    #[test]
    fn conflicting_resource_identifiers_are_rejected_without_replacement() {
        let mut other = fixture();
        let mut duplicate = pool(&mut other).remove(0);
        duplicate.children.push(Chunk { id: *b"data", list: None, data: vec![1], children: Vec::new() });
        let mut chunks = fixture();
        pool(&mut chunks).push(duplicate);
        let mut drawing = Drawing::new(1200);
        assert!(drawing.index_clips(&chunks).unwrap_err().contains("duplicate CDR PowerClip"));
        assert!(drawing.clips.get(&8).unwrap().child(b"data").is_none());
    }

    /// Different definitions sharing an owner fail during indexing, before any graph expansion.
    #[test]
    fn conflicting_branching_frames_are_rejected_before_expansion() {
        let mut chunks = branching_graph(None);
        let frame = pool(&mut chunks)[0].children.last_mut().unwrap();
        frame.children.push(Chunk { id: *b"data", list: None, data: vec![1], children: Vec::new() });
        let mut drawing = Drawing::new(1200);
        assert!(drawing.index_clips(&chunks).unwrap_err().contains("PowerClip frame identifier"));
        assert_eq!(drawing.objects_imported, 0);
        assert_eq!(drawing.visits, 0);
        assert!(drawing.clip_owners.get(&11).unwrap().child(b"data").is_none());
    }

    /// Exact aliases are legal, but their expanded names must hit the cumulative byte quota.
    #[test]
    fn identical_branching_frames_cannot_amplify_names_without_bound() {
        let name = [b'n', 0].repeat(4096);
        let chunks = branching_graph(Some(&name));
        let mut drawing = Drawing::new(1200);
        drawing.index_clips(&chunks).unwrap();
        let page = chunks.iter().find(|c| c.list.as_ref() == Some(b"page")).unwrap();
        let error = drawing.children(&page.children, Affine::IDENTITY).unwrap_err();
        assert!(error.contains("name materialization limit"), "{error}");
        assert_eq!(drawing.name_bytes, super::super::MAX_NAME_BYTES);
        assert!(drawing.points < super::super::MAX_POINTS);
        assert!(drawing.visits < 200_000);
        assert_eq!(drawing.depth, 0);
        assert!(drawing.active_clips.is_empty());
    }

    /// Zero is never a usable frame or resource identifier, even without a repeated definition.
    #[test]
    fn zero_frame_and_resource_identifiers_are_rejected() {
        let mut chunks = fixture();
        let page = chunks.iter_mut().find(|c| c.list.as_ref() == Some(b"page")).unwrap();
        page.children[1].children.iter_mut().find(|c| c.id == *b"spnd").unwrap().data = vec![0; 4];
        assert!(Drawing::new(1200).index_clips(&chunks).unwrap_err().contains("frame identifier"));
        let mut chunks = fixture();
        pool(&mut chunks)[0].children.iter_mut().find(|c| c.id == *b"spnd").unwrap().data = vec![0; 4];
        assert!(Drawing::new(1200).index_clips(&chunks).unwrap_err().contains("resource identifier"));
    }

    /// Reference expansion consumes the same global budget as ordinary page traversal.
    #[test]
    fn referenced_contents_cannot_bypass_the_visit_or_depth_budget() {
        let chunks = fixture();
        let mut drawing = Drawing::new(1200);
        drawing.index_clips(&chunks).unwrap();
        let page = chunks.iter().find(|c| c.list.as_ref() == Some(b"page")).unwrap();
        drawing.visits = 199_998;
        let error = drawing.children(&page.children, Affine::IDENTITY).unwrap_err();
        assert!(error.contains("reference expansion limit"));
        assert!(drawing.active_clips.is_empty(), "failed traversal must release its active reference");
        drawing.visits = 0;
        drawing.depth = 63;
        assert!(drawing.children(&page.children, Affine::IDENTITY).unwrap_err().contains("reference depth limit"));
        assert!(drawing.active_clips.is_empty());
    }
}
