use super::nodes::MAX_DEPTH;
use super::{Kind, af};

/// What copying a subtree takes: its nodes, characters, table cells and path segments (`work`),
/// and its pixels.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CopyCost {
    pub(super) work: u64,
    pub(super) pixels: u64,
}

impl CopyCost {
    pub(super) fn plus(self, o: CopyCost) -> CopyCost {
        CopyCost { work: self.work.saturating_add(o.work), pixels: self.pixels.saturating_add(o.pixels) }
    }
    pub(super) fn minus(self, o: CopyCost) -> Option<CopyCost> {
        Some(CopyCost { work: self.work.checked_sub(o.work)?, pixels: self.pixels.checked_sub(o.pixels)? })
    }
}

/// Master page copies of one import together: as many objects, characters and pixels as a
/// document may hold, a few times over. A master shown on thousands of pages, or a huge one
/// on many, would otherwise multiply them.
pub(super) const MAX_MASTER_COPIES: CopyCost = CopyCost { work: 1 << 23, pixels: 512 << 20 };

/// One object a master spread shows on the pages that show it: a top-level node of the spread
/// (`child` `None`) or one object in one of its layers, with its bounds and what copying it takes.
#[derive(Clone, Copy, Debug)]
pub(super) struct MasterPart {
    pub(super) node: usize,
    pub(super) child: Option<usize>,
    pub(super) bounds: Option<af::Rect>,
    pub(super) cost: CopyCost,
}

pub(super) fn copy_cost(n: &af::Node, depth: usize) -> CopyCost {
    if depth > MAX_DEPTH {
        return CopyCost::default();
    }
    let len = |runs: &[af::TextRun]| runs.iter().map(|r| r.text.len() as u64).sum::<u64>();
    let own = match &n.kind {
        Kind::Text(t) => CopyCost { work: 1 + len(&t.runs), pixels: 0 },
        Kind::Table(t) => CopyCost { work: 1 + t.cells.iter().map(|c| 1 + len(&c.runs)).sum::<u64>(), pixels: 0 },
        Kind::Image(img) => CopyCost { work: 1, pixels: pixels(img) },
        // Each copy builds its own path: a curve of many segments costs them all.
        Kind::Shape { path, .. } | Kind::Artboard { path, .. } => CopyCost { work: 1 + segments(path), pixels: 0 },
        _ => CopyCost { work: 1, pixels: 0 },
    };
    // A vector mask is a path of its own, and a pixel mask an image of its own, in every copy.
    let mask =
        CopyCost { work: n.mask.as_ref().map_or(0, segments) + u64::from(n.pixel_mask.is_some()), pixels: n.pixel_mask.as_ref().map_or(0, pixels) };
    n.children.iter().fold(own.plus(mask), |c, ch| c.plus(copy_cost(ch, depth + 1)))
}

/// The subpaths and segments of `p`.
fn segments(p: &af::Path) -> u64 {
    p.subpaths.iter().map(|s| 1 + s.segments.len() as u64).sum()
}

fn pixels(img: &af::Image) -> u64 {
    u64::from(img.width) * u64::from(img.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: Kind, mask: Option<af::Path>) -> af::Node {
        af::Node {
            class: vectorcraft_affinity::stream::Tag::of(b"ShpN"),
            name: String::new(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: af::Blend::Normal,
            kind,
            mask,
            pixel_mask: None,
            effects: vec![],
            children: vec![],
        }
    }

    fn path(n: usize) -> af::Path {
        let p = af::Point { x: 0.0, y: 0.0 };
        af::Path { subpaths: vec![af::SubPath { start: p, segments: vec![[p; 3]; n], closed: false }] }
    }

    /// A curve, and a vector mask, cost every segment each copy rebuilds: a shape of a million
    /// segments shown on many pages can't pass as one object.
    #[test]
    fn paths_and_masks_cost_their_segments() {
        let shape = |n| Kind::Shape { path: path(n), fills: vec![], strokes: vec![], even_odd: false };
        assert!(copy_cost(&node(shape(1_000_000), None), 0).work > 1_000_000);
        assert!(copy_cost(&node(Kind::Group, Some(path(1_000_000))), 0).work > 1_000_000);
        assert!(copy_cost(&node(shape(3), None), 0).work < 10);
    }
}
