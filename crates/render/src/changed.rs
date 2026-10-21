//! What draws differently after an edit, so a view can draw again only there.

use std::sync::Arc;

use vectorcraft_doc::{Document, Node, NodeKind, SymbolBoxes};
use vectorcraft_geom::{Affine, Rect};

/// What may draw differently in a document than in its version before an edit ([`changed`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Changed {
    Nothing,
    /// This area of the document, outside of which every pixel draws as before.
    Area(Rect),
    Everything,
}

/// What may draw differently in `new` than in `old`, the version before an edit: the bounds of
/// everything the objects that changed paint, before and after the edit ([`crate::painted_bounds`]),
/// or everything when more than the art in the layers changed, or while a pattern is edited (its
/// art also draws in the copies round it). Objects that an edit doesn't touch
/// keep their identity, so this costs what changed, not the size of the document.
pub fn changed(old: &Document, new: &Document) -> Changed {
    if new.pattern_edit.is_some() || !old.same_outside_layers(new) {
        return Changed::Everything;
    }
    let mut diff = Diff { old: old.symbol_boxes(), new: new.symbol_boxes(), area: None };
    diff.children(&old.layers, &new.layers);
    diff.area.map_or(Changed::Nothing, Changed::Area)
}

/// The pixels of a `w`×`h` render through `view` that drawing `area` again covers, as x0, y0, x1,
/// y1: its box in the view, grown by a pixel for the anti-aliased edges. None when it covers none.
pub fn view_pixels(area: Rect, view: Affine, w: u32, h: u32) -> Option<[u32; 4]> {
    let r = view.transform_rect_bbox(area).inflate(1.0, 1.0);
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|c| c.is_finite()) {
        return None;
    }
    let (x0, y0) = (r.x0.floor().max(0.0), r.y0.floor().max(0.0));
    let (x1, y1) = (r.x1.ceil().min(f64::from(w)), r.y1.ceil().min(f64::from(h)));
    (x1 > x0 && y1 > y0).then_some([x0 as u32, y0 as u32, x1 as u32, y1 as u32])
}

/// [`changed`] in progress: the symbols' boxes of both versions, and the area so far.
struct Diff<'a> {
    old: SymbolBoxes<'a>,
    new: SymbolBoxes<'a>,
    area: Option<Rect>,
}

impl Diff<'_> {
    /// Members in the same order compare one by one. Members added, removed or restacked count
    /// before and after, except those that kept their place unchanged.
    fn children(&mut self, old: &[Arc<Node>], new: &[Arc<Node>]) {
        if old.len() == new.len() && old.iter().zip(new).all(|(a, b)| a.id == b.id) {
            for (a, b) in old.iter().zip(new) {
                self.node(a, b);
            }
            return;
        }
        for i in 0..old.len().max(new.len()) {
            match (old.get(i), new.get(i)) {
                (Some(a), Some(b)) if Arc::ptr_eq(a, b) => {}
                (a, b) => {
                    self.add_old(a.map(|a| &**a));
                    self.add_new(b.map(|b| &**b));
                }
            }
        }
    }

    /// A group or layer that only its members changed in, without a clipping mask or effects of
    /// its own that draw from all of them, changed where they did. Any other object that changed
    /// counts whole, before and after.
    fn node(&mut self, old: &Arc<Node>, new: &Arc<Node>) {
        if Arc::ptr_eq(old, new) {
            return;
        }
        if let (Some(a), Some(b)) = (members(old), members(new))
            && same_but_members(old, new)
        {
            self.children(a, b);
        } else {
            self.add_old(Some(old));
            self.add_new(Some(new));
        }
    }

    fn add_old(&mut self, n: Option<&Node>) {
        self.add(n.and_then(|n| crate::brush_fx::cull_bounds(&self.old, n)));
    }

    fn add_new(&mut self, n: Option<&Node>) {
        self.add(n.and_then(|n| crate::brush_fx::cull_bounds(&self.new, n)));
    }

    fn add(&mut self, r: Option<Rect>) {
        self.area = vectorcraft_geom::union_opt(self.area, r);
    }
}

/// The members of a group or layer whose members draw each on their own: no clipping mask, and no
/// effects of its own.
fn members(n: &Node) -> Option<&[Arc<Node>]> {
    match &n.kind {
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } if !crate::fx::has_object_fx(n) => Some(children),
        _ => None,
    }
}

/// Are `a` and `b` the same but for their members?
fn same_but_members(a: &Node, b: &Node) -> bool {
    let strip = |n: &Node| {
        let mut n = n.clone();
        if let Some(children) = n.children_mut() {
            children.clear();
        }
        n
    };
    strip(a) == strip(b)
}
