//! Clipped and grouped art read from page descriptions (metafiles, PostScript) into clipping
//! groups and groups.
//!
//! Importers record each object they draw with the chain of clips (and groups) in force at the
//! time, outermost first ([`Drawn`]); [`nest`] then puts runs of objects under the same clip into
//! one clipping group (under the same group into one group), nesting groups for nested clips,
//! keeping the drawing order.

use std::sync::Arc;

use vectorcraft_geom::{BezPath, FillRule, PathData};

use crate::appearance::Appearance;
use crate::{Document, Node, NodeKind};

/// A clip: a region in document space and its rule, or a plain group (one that holds its art
/// without clipping it, such as an Illustrator file's `u` … `U`). Each has an id unique within an
/// import, so what one clip clips (or one group holds) can be grouped.
#[derive(Debug)]
pub struct Clip {
    pub id: u32,
    /// The region and its rule; `None` for a plain group.
    pub region: Option<(BezPath, FillRule)>,
}

/// Deepest nesting of clips, and of groups, an importer records ([`nest`] goes one level deeper
/// for each): deeper ones are left out. As deep as a document's objects may nest
/// ([`crate::MAX_DEPTH`]) under a layer, around the art; clips in groups that deep together are
/// merged by [`Document::limit_depth`].
pub const MAX_NEST: usize = crate::MAX_DEPTH - 2;

/// The warning for clips left out for nesting deeper than [`MAX_NEST`].
pub fn deep_clips() -> String {
    format!("clips nested more than {MAX_NEST} deep were left out: what they clip is clipped by the clips around them")
}

/// Objects in drawing order, each with its clips (outermost first).
pub type Drawn = Vec<(Vec<Arc<Clip>>, Node)>;

/// The nodes of `items`, those under clips grouped into clipping groups and those in groups into
/// groups (ids from `doc`), in order.
pub fn nest(doc: &mut Document, items: Drawn) -> Vec<Arc<Node>> {
    let mut left = MAX_CLIP_ELEMENTS;
    nest_at(doc, items, 0, &mut left)
}

/// Most path elements the clipping paths [`nest`] makes may have together. Each run of objects
/// under a clip gets its own copy of the clip's path, so one large clip used again around many
/// short runs (an EMF restoring one saved clip between drawings) multiplied it. Past it, runs are
/// grouped without their clip.
pub const MAX_CLIP_ELEMENTS: usize = 8_000_000;

/// [`nest`] for the clips at `depth` and deeper, with `left` of [`MAX_CLIP_ELEMENTS`].
fn nest_at(doc: &mut Document, items: Drawn, depth: usize, left: &mut usize) -> Vec<Arc<Node>> {
    let mut out: Vec<Arc<Node>> = vec![];
    let mut group: Drawn = vec![];
    let mut clip: Option<Arc<Clip>> = None;
    for (chain, node) in items {
        let here = chain.get(depth).cloned();
        if here.as_ref().map(|c| c.id) != clip.as_ref().map(|c| c.id) {
            if let Some(c) = clip.take() {
                out.push(clip_group(doc, &c, std::mem::take(&mut group), depth, left));
            }
            clip = here.clone();
        }
        match here {
            Some(_) => group.push((chain, node)),
            None => out.push(Arc::new(node)),
        }
    }
    if let Some(c) = clip {
        out.push(clip_group(doc, &c, group, depth, left));
    }
    out
}

/// A clipping group of `items`, clipped by `c` (their clip at `depth`), or their group when `c` is
/// a plain group.
fn clip_group(doc: &mut Document, c: &Clip, items: Drawn, depth: usize, left: &mut usize) -> Arc<Node> {
    let mut children = vec![];
    let region = c.region.as_ref().filter(|(bp, _)| match left.checked_sub(bp.elements().len()) {
        Some(rest) => {
            *left = rest;
            true
        }
        None => false,
    });
    if let Some((bp, r)) = region {
        let mut path = Node::path(doc.alloc_id(), PathData::from_bezpath(bp), Appearance::default());
        if let NodeKind::Path { clipping, rule, .. } = &mut path.kind {
            *clipping = true;
            *rule = *r;
        }
        children.push(Arc::new(path));
    }
    children.extend(nest_at(doc, items, depth + 1, left));
    Arc::new(Node::new(doc.alloc_id(), NodeKind::Group { children, clip: region.is_some() }))
}
