//! Symbol Options › Enable Guides for 9-Slice Scaling: four guides that cut a symbol into a 3 × 3
//! grid so that, in Animate / Flash handoff, the corners keep their size while the edges and the
//! centre stretch
//! ([helpx Symbol Options](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/create-and-place-symbols.html)).
//!
//! The guides are stored as fractions (0..1) of the symbol's natural size, measured from the left
//! and top edges and from the right and bottom edges, so they survive resizing the art. Like
//! [`crate::SymbolExportType`] this is handoff metadata: instances place the same on the canvas.
//! Stored under [`SYMBOL_NINE_SLICE`] keyed by symbol name; symbols without 9-slice have no entry.

use serde_json::{Value, json};

/// Document unknown key: `{ symbolName: [left, top, right, bottom] }`.
pub const SYMBOL_NINE_SLICE: &str = "symbolNineSlice";

/// The four 9-slice guides as fractions of the symbol's natural size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymbolNineSlice {
    /// Left guide, from the left edge.
    pub left: f64,
    /// Top guide, from the top edge.
    pub top: f64,
    /// Right guide, from the right edge.
    pub right: f64,
    /// Bottom guide, from the bottom edge.
    pub bottom: f64,
}

impl Default for SymbolNineSlice {
    /// The centre third of each axis is the stretchy cell.
    fn default() -> Self {
        let t = 1.0 / 3.0;
        Self { left: t, top: t, right: t, bottom: t }
    }
}

impl SymbolNineSlice {
    /// Guides from `[left, top, right, bottom]`; `None` unless each is finite, within 0..1, and the
    /// two on each axis leave a gap between them.
    pub fn new(left: f64, top: f64, right: f64, bottom: f64) -> Option<Self> {
        let ok = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        (ok(left) && ok(top) && ok(right) && ok(bottom) && left + right < 1.0 && top + bottom < 1.0).then_some(Self { left, top, right, bottom })
    }

    pub fn to_value(self) -> Value {
        json!([self.left, self.top, self.right, self.bottom])
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let a = v.as_array()?;
        let n = |i: usize| a.get(i).and_then(Value::as_f64);
        if a.len() != 4 {
            return None;
        }
        Self::new(n(0)?, n(1)?, n(2)?, n(3)?)
    }
}
