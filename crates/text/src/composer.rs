//! Adapters from VectorCraft's shaped glyphs to the shared paragraph composer.

use crate::justify::Flex;
use crate::shape::SGlyph;
pub(crate) use craft_text::composer::{Breakpoint, Exhausted, Measure, Params};

pub(crate) fn compose(
    g: &[SGlyph],
    width: &dyn Fn(usize) -> f64,
    start: &dyn Fn(usize) -> f64,
    end: &dyn Fn(usize) -> f64,
    cands: &[Breakpoint],
    flex: &[Flex],
    params: &Params,
) -> Option<Vec<(usize, bool)>> {
    craft_text::composer::compose(g, width, start, end, cands, flex, params)
}

pub(crate) fn compose_with<M: Measure>(
    g: &[SGlyph],
    measure: &mut M,
    start: &dyn Fn(usize) -> f64,
    end: &dyn Fn(usize) -> f64,
    cands: &[Breakpoint],
    flex: &[Flex],
    params: &Params,
) -> Option<Vec<(usize, bool)>> {
    craft_text::composer::compose_with(g, measure, start, end, cands, flex, params)
}

impl craft_text::composer::Glyph for crate::shape::SGlyph {
    fn advance(&self) -> f64 {
        self.adv
    }
    fn is_space(&self) -> bool {
        crate::shape::SGlyph::is_space(self)
    }
    fn is_soft_hyphen(&self) -> bool {
        crate::shape::SGlyph::is_soft_hyphen(self)
    }
}

impl craft_text::composer::Flexibility for crate::justify::Flex {
    fn stretch(&self) -> f64 {
        crate::justify::Flex::stretch(self)
    }
    fn shrink(&self) -> f64 {
        crate::justify::Flex::shrink(self)
    }
}
