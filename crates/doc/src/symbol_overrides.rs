//! Dynamic symbols (Symbol Options › Symbol Type › Dynamic Symbol): an instance of a dynamic
//! symbol keeps appearance changes of its own to objects in the symbol's art, while the art stays
//! the symbol's. Editing the symbol reshapes every instance and each keeps its changes; Reset drops
//! them. The changes are stored on the instance ([`crate::node::NodeExtras::symbol_overrides`]) and
//! applied wherever an instance is drawn or exported ([`dynamic_art`]).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{BlendMode, Paint};

use crate::{Node, NodeKind};

/// Deepest object an override may name: groups nested this deep in the symbol's art.
pub const MAX_PATH_DEPTH: usize = 64;

/// One instance's appearance changes to one object of its dynamic symbol's art.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SymbolOverride {
    /// The object: child indices (paint order) from the art's root down through its groups; empty
    /// for the root itself.
    #[serde(deserialize_with = "short_path")]
    pub path: Vec<u32>,
    /// The object's (top) fill paint; type takes it as its characters' fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// The object's (top) stroke paint; type takes it as its characters' stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Paint>,
    /// The object's opacity, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// The object's blending mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<BlendMode>,
}

impl SymbolOverride {
    /// Does this override change nothing?
    pub fn is_empty(&self) -> bool {
        self.fill.is_none() && self.stroke.is_none() && self.opacity.is_none() && self.blend.is_none()
    }

    /// Put this override's changes on object `n`.
    fn apply(&self, n: &mut Node) {
        if let NodeKind::Text(t) = &mut n.kind {
            for r in &mut t.runs {
                if let Some(p) = &self.fill {
                    r.style.fill = p.clone();
                }
                if let Some(p) = &self.stroke {
                    r.style.stroke = p.clone();
                }
            }
        } else {
            if let Some(p) = &self.fill {
                n.appearance.set_fill(p.clone());
            }
            if let Some(p) = &self.stroke {
                n.appearance.set_stroke(p.clone());
            }
        }
        if let Some(o) = self.opacity.filter(|o| o.is_finite()) {
            n.opacity = o.clamp(0.0, 1.0);
        }
        if let Some(b) = self.blend {
            n.blend = b;
        }
    }
}

/// An override's path read from a file: at most [`MAX_PATH_DEPTH`] indices, so a compressed file
/// of one endless path stays small once read.
fn short_path<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u32>, D::Error> {
    struct Path;
    impl<'de> serde::de::Visitor<'de> for Path {
        type Value = Vec<u32>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list of child indices")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u32>, A::Error> {
            let mut out = Vec::new();
            while let Some(i) = seq.next_element::<u32>()? {
                if out.len() == MAX_PATH_DEPTH {
                    return Err(serde::de::Error::custom(format!("a symbol override path is deeper than {MAX_PATH_DEPTH}")));
                }
                out.push(i);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Path)
}

/// The children a path steps through: a group's (clip groups included) or a layer's.
fn groups_children(n: &Node) -> Option<&Vec<Arc<Node>>> {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }).then(|| n.children()).flatten()
}

/// The object `path` names in symbol art `art`, if there is one.
pub fn part<'a>(art: &'a Node, path: &[u32]) -> Option<&'a Node> {
    if path.len() > MAX_PATH_DEPTH {
        return None;
    }
    path.iter().try_fold(art, |n, &i| groups_children(n)?.get(i as usize).map(|c| &**c))
}

/// [`part`], to change (the nodes on the way are copied when shared).
fn part_mut<'a>(art: &'a mut Node, path: &[u32]) -> Option<&'a mut Node> {
    let Some((&i, rest)) = path.split_first() else { return Some(art) };
    if !matches!(art.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) {
        return None;
    }
    let c = art.children_mut()?.get_mut(i as usize)?;
    part_mut(Arc::make_mut(c), rest)
}

/// Put `overrides` on symbol art `art`; an override naming no object of it is left out.
pub fn apply(art: &mut Node, overrides: &[SymbolOverride]) {
    for o in overrides.iter().filter(|o| o.path.len() <= MAX_PATH_DEPTH) {
        if let Some(n) = part_mut(art, &o.path) {
            o.apply(n);
        }
    }
}

/// Symbol art `art` as instance `inst` shows it: a copy with the instance's overrides applied.
pub fn dynamic_art(art: &Node, inst: &Node) -> Node {
    let mut a = art.clone();
    apply(&mut a, &inst.extras().symbol_overrides);
    a
}

/// Keep the overrides that name an object of `art` (a symbol's new art), dropping the rest.
pub fn retain_parts(overrides: &mut Vec<SymbolOverride>, art: &Node) {
    overrides.retain(|o| part(art, &o.path).is_some());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, NodeId};
    use vectorcraft_color::Color;
    use vectorcraft_geom::{Rect, shapes};

    fn square(id: u64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art()))
    }

    fn art() -> Node {
        let inner = Node::group(NodeId(3), vec![square(4), square(5)]);
        Node::group(NodeId(1), vec![square(2), Arc::new(inner)])
    }

    #[test]
    fn paths_name_objects_through_groups_only() {
        let a = art();
        assert_eq!(part(&a, &[]).map(|n| n.id), Some(NodeId(1)));
        assert_eq!(part(&a, &[1, 0]).map(|n| n.id), Some(NodeId(4)));
        assert!(part(&a, &[2]).is_none());
        assert!(part(&a, &[0, 0]).is_none(), "a path has no children");
        assert!(part(&a, &[0; MAX_PATH_DEPTH + 1]).is_none());
    }

    #[test]
    fn overrides_paint_their_object_and_skip_missing_ones() {
        let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
        let o = vec![
            SymbolOverride { path: vec![1, 1], fill: Some(red.clone()), opacity: Some(0.5), ..Default::default() },
            SymbolOverride { path: vec![9], fill: Some(red.clone()), ..Default::default() },
        ];
        let mut a = art();
        apply(&mut a, &o);
        let n = part(&a, &[1, 1]).unwrap();
        assert_eq!(n.appearance.fill_paint(), red);
        assert_eq!(n.opacity, 0.5);
        assert_eq!(part(&a, &[1, 0]).unwrap().appearance.fill_paint(), Paint::solid(Color::WHITE), "its sibling keeps the symbol's look");
        let mut kept = o;
        retain_parts(&mut kept, &a);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn a_path_deeper_than_the_cap_does_not_read() {
        let deep = format!("{{\"path\":{:?}}}", vec![0; MAX_PATH_DEPTH + 1]);
        assert!(serde_json::from_str::<SymbolOverride>(&deep).is_err());
        let ok: SymbolOverride = serde_json::from_str("{\"path\":[1,0],\"opacity\":0.25}").unwrap();
        assert_eq!(ok, SymbolOverride { path: vec![1, 0], opacity: Some(0.25), ..Default::default() });
    }
}
