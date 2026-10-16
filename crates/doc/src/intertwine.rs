//! Object › Intertwine: objects that pass over and under each other.
//!
//! An Intertwine group ([`NodeKind::Intertwine`]) keeps its members as they are, in their own
//! stacking order. A member can be drawn over the other members in part of the group: that area
//! is the member's [`crate::node::NodeExtras::intertwine`], in document coordinates. The group
//! draws (and exports, and is hit) as its live expansion ([`expand`]): the members, then, for
//! each member with an area, a copy of it clipped to that area. Where several members are drawn
//! over the others in one place, the upper one in the stacking order ends on top.
//!
//! The members stay editable, and Release gives them back as they were (the areas let go).
//! Behaviour: <https://helpx.adobe.com/illustrator/using/intertwine-objects.html>.

use std::sync::Arc;

use vectorcraft_geom::{FillRule, PathData};

use crate::{Node, NodeId, NodeKind};

/// The area where `n` is drawn over the other members of its Intertwine group (empty: nowhere).
pub fn over_area(n: &Node) -> Option<&PathData> {
    n.extras().intertwine.as_deref().filter(|a| !a.is_empty())
}

/// The members of an Intertwine group evaluated as plain art: the members (ids kept), then each
/// member with an area drawn over the others, as a clip group of that area around a copy of it
/// (ids 0: generated art).
pub fn expand(children: &[Arc<Node>]) -> Vec<Node> {
    let mut out: Vec<Node> = children.iter().map(|c| plain(c)).collect();
    for c in children {
        let Some(area) = over_area(c) else { continue };
        if !c.visible {
            continue;
        }
        let mut clip = Node::new(NodeId(0), NodeKind::Path { path: area.clone(), rule: FillRule::NonZero, live: None, clipping: true, guide: false });
        clip.appearance = crate::Appearance::default().into();
        let mut copy = plain(c);
        zero_ids(&mut copy);
        out.push(Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(copy)], clip: true }));
    }
    out
}

/// `n` without its Intertwine area (the expansion is plain art).
fn plain(n: &Node) -> Node {
    let mut n = n.clone();
    if n.extras().intertwine.is_some() {
        n.extras_mut().intertwine = None;
    }
    n
}

fn zero_ids(n: &mut Node) {
    n.id = NodeId(0);
    for c in n.children_mut().into_iter().flatten() {
        zero_ids(Arc::make_mut(c));
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_geom::{Rect, shapes};

    use super::*;

    fn square(id: u64, x: f64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), crate::Appearance::default()))
    }

    #[test]
    fn members_without_areas_expand_to_themselves() {
        let kids = vec![square(1, 0.0), square(2, 5.0)];
        let out = expand(&kids);
        assert_eq!(out.len(), 2);
        assert_eq!(out.iter().map(|n| n.id).collect::<Vec<_>>(), [NodeId(1), NodeId(2)]);
    }

    #[test]
    fn a_member_with_an_area_is_drawn_again_clipped_to_it_on_top() {
        let mut a = (*square(1, 0.0)).clone();
        a.extras_mut().intertwine = Some(Box::new(shapes::rectangle(Rect::new(5.0, 0.0, 10.0, 10.0))));
        let kids = vec![Arc::new(a), square(2, 5.0)];
        let out = expand(&kids);
        assert_eq!(out.len(), 3, "both members, then the raised copy");
        assert!(out.iter().all(|n| n.extras().intertwine.is_none()), "the expansion is plain art");
        let NodeKind::Group { children, clip: true } = &out[2].kind else { panic!("a clip group: {:?}", out[2].kind) };
        assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
        assert_eq!(children[1].path_data(), kids[0].path_data());
        assert_eq!((out[2].id, children[1].id), (NodeId(0), NodeId(0)), "generated art has no ids of its own");
    }

    #[test]
    fn a_hidden_member_is_not_drawn_over_the_others() {
        let mut a = (*square(1, 0.0)).clone();
        a.visible = false;
        a.extras_mut().intertwine = Some(Box::new(shapes::rectangle(Rect::new(5.0, 0.0, 10.0, 10.0))));
        assert_eq!(expand(&[Arc::new(a), square(2, 5.0)]).len(), 2);
    }
}
