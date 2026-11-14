//! The colour index palette is `craft_dxf::model::aci`; here are the indices each colour depth
//! of the writer may use.

use crate::ColorDepth;

/// The RGB of colour index `i` (1–255; anything else is index 7's white).
pub fn aci_rgb(i: u8) -> [u8; 3] {
    craft_dxf::model::aci::rgb(i)
}

/// The indices a colour depth may use (true colour: all of them, for the fallback index).
fn candidates(depth: ColorDepth) -> Box<dyn Iterator<Item = u8>> {
    match depth {
        ColorDepth::Aci8 => Box::new(1..=8),
        ColorDepth::Aci16 => Box::new((1..=9).chain([30]).chain(250..=255)),
        ColorDepth::Aci256 | ColorDepth::True => Box::new(1..=255),
    }
}

/// The index of `depth` nearest to `rgb` (squared RGB distance; ties go to the lower index).
/// Index 7 stands for both white and black, as CAD apps draw it in the background's contrast.
pub fn nearest_aci(rgb: [u8; 3], depth: ColorDepth) -> u8 {
    craft_dxf::model::aci::nearest(rgb, candidates(depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_index_by_depth() {
        assert_eq!(nearest_aci([255, 0, 0], ColorDepth::Aci8), 1);
        assert_eq!(nearest_aci([0, 0, 0], ColorDepth::Aci256), 7, "black is index 7");
        assert_eq!(nearest_aci([255, 255, 255], ColorDepth::Aci16), 7);
        assert_eq!(nearest_aci([255, 127, 0], ColorDepth::Aci256), 30);
        assert_eq!(nearest_aci([255, 127, 0], ColorDepth::Aci16), 30);
        assert_eq!(nearest_aci([255, 127, 0], ColorDepth::Aci8), 1, "8 colours: red is nearest");
        assert_eq!(nearest_aci([60, 60, 60], ColorDepth::Aci16), 250);
        assert_eq!(nearest_aci([100, 100, 100], ColorDepth::Aci8), 8);
        assert_eq!(nearest_aci([60, 60, 60], ColorDepth::Aci8), 7, "dark greys go to black");
    }
}
