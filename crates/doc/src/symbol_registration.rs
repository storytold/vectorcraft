//! Symbol Options › Registration: the 9-point grid (0 = top-left … 4 = centre … 8 = bottom-right)
//! that `symbol.place` uses as the point that lands on the click.

use vectorcraft_geom::Vec2;

/// Document unknown key: `{ symbolName: 0..8 }` for each symbol's registration point.
pub const SYMBOL_REGISTRATION: &str = "symbolRegistration";

/// Index on the 9-point grid (row-major, 4 = centre).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolRegistration(pub u8);

impl Default for SymbolRegistration {
    fn default() -> Self {
        Self(4)
    }
}

impl SymbolRegistration {
    pub const CENTER: Self = Self(4);

    pub fn from_u64(v: u64) -> Option<Self> {
        (v <= 8).then_some(Self(v as u8))
    }

    pub fn is_center(self) -> bool {
        self.0 == 4
    }

    /// Offset of this registration point from the symbol's centre, in natural-size units
    /// (right +, down +).
    pub fn offset_from_center(self, w: f64, h: f64) -> Vec2 {
        let (col, row) = (self.0 % 3, self.0 / 3);
        Vec2::new((f64::from(col) - 1.0) * (w * 0.5), (f64::from(row) - 1.0) * (h * 0.5))
    }
}
