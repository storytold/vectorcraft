//! Adapters from VectorCraft's shaped glyphs to the shared paragraph composer.

pub(crate) use craft_text::composer::{Breakpoint, Exhausted, Measure, Params, compose, compose_with};

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
