//! After an edit, a clip group or clipping layer keeps its clipping path wherever the edit puts it
//! among its members, and stops clipping when the edit takes the clipping path out of it.

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_doc::{Document, Node, NodeId};

use super::compound::release_clip;

/// What becomes of a clip the edit changed.
enum Outcome {
    /// Its member at this index is its clipping path.
    Keep(usize),
    /// It stops clipping; the clipping path it had, when the edit moved it elsewhere.
    Release(Option<NodeId>),
}

/// What [`refresh`] changes, found before it changes anything.
#[derive(Default)]
struct Changes {
    /// Objects to mark as clipping paths.
    mark: Vec<NodeId>,
    /// Objects to take the mark off.
    unmark: Vec<NodeId>,
    /// Clip groups and clipping layers to release.
    release: Vec<NodeId>,
    /// The clipping paths of those, moved out of them.
    moved_out: Vec<NodeId>,
}

/// After any edit ([`crate::Session::edit`]), for each layer and group the edit changed:
/// - A clip group or clipping layer whose clipping path is still one of its members keeps it,
///   wherever it now is among them: reordering them, or putting art below the clipping path,
///   changes nothing that shows. The clipping path is marked ([`Node::set_clipping_path`]) when its
///   place no longer makes it the one the clip finds ([`Node::clipping_path_index`]), and any other
///   member marked as one (a copy of it) loses the mark.
/// - One whose clipping path the edit deleted, or moved out (into another layer or group, or into
///   a group made of it and art inside the clip), is released as Release Clipping Mask releases it
///   ([`release_clip`]), so no other member becomes its clipping path. The clipping path the edit
///   moved loses the mark, unless it is the clipping path of the clip it went into.
/// - When the edit put new art in the clipping path's place (an object it made of the clipping
///   path, such as type's outlines), that art is the clipping path. When it made members of the
///   clipping path's parts (a compound path released), the bottom one is, wherever it is.
/// - A layer or group that starts clipping in the edit (Make Clipping Mask) keeps the clipping path
///   it was made with, and its other members lose the mark.
/// - In a layer or group that doesn't clip, an object the edit put there loses the mark: a copy of
///   a clipping path, or one taken out of its clip, is a plain object.
///
/// Layers and groups the edit did not change are not visited.
pub(crate) fn refresh(before: &Document, doc: &mut Document) {
    let mut out = Changes::default();
    walk(before, doc, &before.layers, &doc.layers, &mut out);
    for id in &out.release {
        release_clip(doc, *id);
    }
    mark(doc, &out.mark, true);
    mark(doc, &out.unmark, false);
    for path in out.moved_out {
        let clips_there = doc.parent_of(path).and_then(|p| doc.node(p)).and_then(Node::clipping_path).is_some_and(|c| c.id == path);
        if !clips_there {
            mark(doc, &[path], false);
        }
    }
}

/// [`decide`] for the objects with members among `after` (the list `before` was before the edit)
/// and inside them that are not the same nodes as before, outer ones first.
fn walk<'a>(old: &'a Document, doc: &Document, before: &'a [Arc<Node>], after: &[Arc<Node>], out: &mut Changes) {
    let mut by_id: Option<HashMap<NodeId, &'a Arc<Node>>> = None;
    for (i, a) in after.iter().enumerate() {
        let b = match before.get(i) {
            Some(b) if b.id == a.id => Some(b),
            _ => by_id.get_or_insert_with(|| before.iter().map(|b| (b.id, b)).collect()).get(&a.id).copied(),
        };
        if b.is_some_and(|b| Arc::ptr_eq(b, a)) {
            continue;
        }
        let Some(children) = a.children() else { continue };
        // One the edit moved is found wherever it was (one the edit made, with an id given out
        // after `old`, is not looked for).
        let was = b.map(|b| &**b).or_else(|| if a.id.0 < old.peek_next_id() { old.node(a.id) } else { None });
        decide(was, a, doc, out);
        walk(old, doc, was.and_then(Node::children).map_or(&[], Vec::as_slice), children, out);
    }
}

/// What [`refresh`] changes in `now`, an object with members that the edit changed (`was`: it
/// before the edit, if it was in the document).
fn decide(was: Option<&Node>, now: &Node, doc: &Document, out: &mut Changes) {
    let Some(children) = now.children() else { return };
    if !now.clips() {
        let came = |c: &&Arc<Node>| was.and_then(Node::children).is_none_or(|w| w.iter().all(|o| o.id != c.id));
        out.unmark.extend(children.iter().filter(|c| c.is_clipping_path()).filter(came).map(|c| c.id));
        return;
    }
    match outcome(was, now, doc) {
        Some(Outcome::Keep(k)) => {
            if now.clipping_path_index() != Some(k) {
                out.mark.extend(children.get(k).map(|c| c.id));
            }
            out.unmark.extend(children.iter().enumerate().filter(|(i, c)| *i != k && c.is_clipping_path()).map(|(_, c)| c.id));
        }
        Some(Outcome::Release(path)) => {
            out.release.push(now.id);
            out.moved_out.extend(path);
        }
        None => {}
    }
}

/// What becomes of `now`, a clip group or clipping layer the edit changed, which was `was` before
/// it. None when it has no members.
fn outcome(was: Option<&Node>, now: &Node, doc: &Document) -> Option<Outcome> {
    let children = now.children()?;
    let before = was.filter(|w| w.clips()).and_then(|w| {
        let k = w.clipping_path_index()?;
        Some((w, k, w.children()?.get(k)?.id))
    });
    // A clip made in this edit: as it was made.
    let Some((was, k, path)) = before else { return now.clipping_path_index().map(Outcome::Keep) };
    if let Some(i) = children.iter().position(|c| c.id == path) {
        return Some(Outcome::Keep(i));
    }
    if doc.node(path).is_some() {
        return Some(Outcome::Release(Some(path)));
    }
    // Deleted, unless new art took its place.
    let old: std::collections::HashSet<NodeId> = was.children().into_iter().flatten().map(|o| o.id).collect();
    let had = |id: NodeId| old.contains(&id);
    // Its parts, members of their own now (a compound path released): the bottom one, wherever
    // the edit put it among the others, which other objects released with it may have moved.
    let mut parts = std::collections::HashSet::new();
    if let Some(old) = was.children().and_then(|w| w.get(k)) {
        old.walk(&mut |n| {
            parts.insert(n.id);
        });
        parts.remove(&old.id);
    }
    if let Some(i) = children.iter().position(|c| !had(c.id) && parts.contains(&c.id)) {
        return Some(Outcome::Keep(i));
    }
    Some(match children.get(k) {
        Some(c) if !had(c.id) => Outcome::Keep(k),
        _ => Outcome::Release(None),
    })
}

/// Mark the objects `ids` as clipping paths, or take the mark off them.
fn mark(doc: &mut Document, ids: &[NodeId], on: bool) {
    for id in ids {
        if let Some(n) = doc.node_mut(*id)
            && n.is_clipping_path() != on
        {
            n.set_clipping_path(on);
        }
    }
}
