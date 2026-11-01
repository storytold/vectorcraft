//! Object › Intertwine: objects that pass over and under each other.
//!
//! An Intertwine group ([`NodeKind::Intertwine`]) keeps its members as they are, in their own
//! stacking order. A member can be drawn over other members in part of the group, at a tier: the
//! area of tier 1 is the member's [`crate::node::NodeExtras::intertwine`], and the areas of tiers 2
//! and up are its [`crate::node::NodeExtras::intertwine_tiers`], in document coordinates. In each
//! place the members are drawn by their tier there (0 outside their areas), and members of one
//! tier in their stacking order, so any order of the members can be set for each overlap area.
//! The group draws (and exports, and is hit) as its live expansion ([`expand`]): the members,
//! then, tier by tier, a copy of each member with an area of that tier clipped to it.
//!
//! The members stay editable, and Release gives them back as they were (the areas let go).
//! Behaviour: <https://helpx.adobe.com/illustrator/using/intertwine-objects.html>.

use std::sync::Arc;

use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{FillRule, PathData, Point};

use crate::{Node, NodeId, NodeKind};

/// The areas where `n` is drawn over other members of its Intertwine group, each with its tier
/// (1 and up); empty ones are left out.
pub fn tier_areas(n: &Node) -> impl Iterator<Item = (usize, &PathData)> {
    let e = n.extras();
    e.intertwine
        .as_deref()
        .map(|a| (1, a))
        .into_iter()
        .chain(e.intertwine_tiers.iter().enumerate().map(|(k, a)| (k + 2, a)))
        .filter(|(_, a)| !a.is_empty())
}

/// The tier `n` is drawn at at `p`: the highest tier whose area holds `p` (0: none does).
pub fn tier_at(n: &Node, p: Point) -> usize {
    tier_areas(n).filter(|(_, a)| fill_contains(&a.to_bezpath(), FillRule::NonZero, p)).map(|(t, _)| t).max().unwrap_or(0)
}

/// The members of an Intertwine group evaluated as plain art: the members (ids kept), then, for
/// each tier from 1 up, each member with an area of that tier, as a clip group of that area
/// around a copy of it (ids 0: generated art).
pub fn expand(children: &[Arc<Node>]) -> Vec<Node> {
    let mut out: Vec<Node> = children.iter().map(|c| plain(c)).collect();
    let mut over: Vec<(usize, usize, &PathData)> =
        children.iter().enumerate().filter(|(_, c)| c.visible).flat_map(|(i, c)| tier_areas(c).map(move |(t, a)| (t, i, a))).collect();
    over.sort_by_key(|(t, i, _)| (*t, *i));
    for (_, i, area) in over {
        let Some(c) = children.get(i) else { continue };
        let mut clip = Node::new(NodeId(0), NodeKind::Path { path: area.clone(), rule: FillRule::NonZero, live: None, clipping: true, guide: false });
        clip.appearance = crate::Appearance::default().into();
        let mut copy = plain(c);
        zero_ids(&mut copy);
        out.push(Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(copy)], clip: true }));
    }
    out
}

/// `n` without its Intertwine areas (the expansion is plain art).
fn plain(n: &Node) -> Node {
    let mut n = n.clone();
    if n.extras().intertwine.is_some() || !n.extras().intertwine_tiers.is_empty() {
        let mut e = n.extras_mut();
        e.intertwine = None;
        e.intertwine_tiers = Vec::new();
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
    fn copies_are_drawn_tier_by_tier() {
        let over = || shapes::rectangle(Rect::new(5.0, 0.0, 10.0, 10.0));
        // The lower member at tier 2, the upper one at tier 1: the lower one ends on top.
        let mut a = (*square(1, 0.0)).clone();
        a.extras_mut().intertwine_tiers = vec![over()];
        let mut b = (*square(2, 5.0)).clone();
        b.extras_mut().intertwine = Some(Box::new(over()));
        let kids = vec![Arc::new(a), Arc::new(b)];
        assert_eq!((tier_at(&kids[0], Point::new(7.0, 5.0)), tier_at(&kids[1], Point::new(7.0, 5.0))), (2, 1));
        assert_eq!(tier_at(&kids[0], Point::new(2.0, 5.0)), 0);
        let out = expand(&kids);
        assert_eq!(out.len(), 4, "both members, then the copy of tier 1, then the one of tier 2");
        assert!(out.iter().all(|n| n.extras().intertwine.is_none() && n.extras().intertwine_tiers.is_empty()), "the expansion is plain art");
        let copied = |n: &Node| match &n.kind {
            NodeKind::Group { children, clip: true } => children[1].path_data().cloned(),
            k => panic!("a clip group: {k:?}"),
        };
        assert_eq!(copied(&out[2]).as_ref(), kids[1].path_data());
        assert_eq!(copied(&out[3]).as_ref(), kids[0].path_data());
    }

    #[test]
    fn a_hidden_member_is_not_drawn_over_the_others() {
        let mut a = (*square(1, 0.0)).clone();
        a.visible = false;
        a.extras_mut().intertwine = Some(Box::new(shapes::rectangle(Rect::new(5.0, 0.0, 10.0, 10.0))));
        assert_eq!(expand(&[Arc::new(a), square(2, 5.0)]).len(), 2);
    }
}
