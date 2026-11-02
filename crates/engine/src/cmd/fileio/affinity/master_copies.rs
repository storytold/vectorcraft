use super::nodes::MAX_DEPTH;
use super::{Kind, af};

/// What copying a subtree takes: its nodes, characters and table cells (`work`), and its pixels.
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
        Kind::Image(img) => CopyCost { work: 1, pixels: u64::from(img.width) * u64::from(img.height) },
        _ => CopyCost { work: 1, pixels: 0 },
    };
    n.children.iter().fold(own, |c, ch| c.plus(copy_cost(ch, depth + 1)))
}
