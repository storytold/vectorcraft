//! Helpers the encoders' tests share.

use crate::{BarcodeSpec, Code, Symbol};

/// The module row of a finished linear symbol as a string of `1` (dark) and `0`, quiet zones
/// excluded: the shape a reference implementation's output can be compared against.
///
/// A symbol whose elements are not whole modules — Code 39 and ITF at a fractional wide-to-narrow
/// ratio — comes back with every module a bar covers marked, which is enough to compare the
/// element runs. Symbols with bearer bars are not meaningful here: those run the full width.
pub fn module_row(symbol: &Symbol) -> String {
    let module = symbol.module_mm;
    if !module.is_finite() || module <= 0.0 {
        return String::new();
    }
    let quiet_left = symbol.symbology.quiet_modules().0 * module;
    let total = symbol.modules.round().max(1.0) as usize;
    let mut row = vec![false; total];
    for bar in &symbol.bars {
        let from = ((bar.x - quiet_left) / module).round().max(0.0) as usize;
        let to = (((bar.x - quiet_left) + bar.width) / module).round().max(0.0) as usize;
        for slot in row.iter_mut().take(to.min(total)).skip(from) {
            *slot = true;
        }
    }
    row.iter().map(|d| if *d { '1' } else { '0' }).collect()
}

/// Encode `data` and return the linear symbol.
pub fn linear(symbology: crate::Symbology, data: &str) -> Symbol {
    match crate::encode(&BarcodeSpec::new(symbology, data)) {
        Ok(Code::Linear(s)) => s,
        other => panic!("{symbology:?} {data:?}: {other:?}"),
    }
}
