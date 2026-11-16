//! Symbol Options › Align to Pixel Grid: a symbol whose instances stay on the pixel grid at their
//! actual size wherever they are placed
//! (<https://web.archive.org/web/20220911143322/https://helpx.adobe.com/pdf/cs6/illustrator_reference.pdf#page=28>).
//!
//! Stored under [`SYMBOL_PIXEL_ALIGNED`] keyed by symbol name; a symbol that is not pixel-aligned
//! (the default) is left out of the map.

use std::collections::HashSet;

use crate::Document;

/// Document unknown key: `{ symbolName: true }` for each pixel-aligned symbol.
pub const SYMBOL_PIXEL_ALIGNED: &str = "symbolPixelAligned";

impl Document {
    /// Whether symbol `name` has Symbol Options › Align to Pixel Grid on (default: off).
    pub fn symbol_pixel_aligned(&self, name: &str) -> bool {
        self.unknown.get(SYMBOL_PIXEL_ALIGNED).and_then(|m| m.get(name)).and_then(serde_json::Value::as_bool).unwrap_or(false)
    }

    /// Turn Align to Pixel Grid on or off for symbol `name`. Off removes the symbol from the map,
    /// and the map goes when it is empty.
    pub fn set_symbol_pixel_aligned(&mut self, name: &str, on: bool) {
        if on {
            let m = self.unknown.entry(SYMBOL_PIXEL_ALIGNED.to_string()).or_insert_with(|| serde_json::json!({}));
            if let Some(o) = m.as_object_mut() {
                o.insert(name.into(), serde_json::json!(true));
            }
            return;
        }
        let Some(o) = self.unknown.get_mut(SYMBOL_PIXEL_ALIGNED).and_then(|m| m.as_object_mut()) else { return };
        o.remove(name);
        if o.is_empty() {
            self.unknown.remove(SYMBOL_PIXEL_ALIGNED);
        }
    }

    /// Move the stored setting when a symbol is renamed.
    pub fn rename_symbol_pixel_aligned(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let on = self.symbol_pixel_aligned(from);
        self.set_symbol_pixel_aligned(from, false);
        self.set_symbol_pixel_aligned(to, on);
    }

    /// The names of the pixel-aligned symbols the document holds (empty for most documents).
    pub fn pixel_aligned_symbols(&self) -> HashSet<String> {
        let Some(o) = self.unknown.get(SYMBOL_PIXEL_ALIGNED).and_then(|m| m.as_object()) else { return HashSet::new() };
        o.iter()
            .filter(|(name, v)| v.as_bool() == Some(true) && self.symbols.iter().any(|s| &s.name == *name))
            .map(|(name, _)| name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_is_off_by_default_and_follows_a_rename() {
        let mut d = Document::new(100.0, 100.0);
        let art = std::sync::Arc::new(crate::Node::group(d.alloc_id(), vec![]));
        d.symbols.push(crate::Symbol { dynamic: false, name: "A".into(), art });
        assert!(!d.symbol_pixel_aligned("A"));
        assert!(d.unknown.get(SYMBOL_PIXEL_ALIGNED).is_none());
        d.set_symbol_pixel_aligned("A", true);
        assert!(d.symbol_pixel_aligned("A"));
        assert_eq!(d.pixel_aligned_symbols(), HashSet::from(["A".to_string()]));
        d.symbols[0].name = "B".into();
        d.rename_symbol_pixel_aligned("A", "B");
        assert!(!d.symbol_pixel_aligned("A"));
        assert!(d.symbol_pixel_aligned("B"));
        d.set_symbol_pixel_aligned("B", false);
        assert!(d.unknown.get(SYMBOL_PIXEL_ALIGNED).is_none());
        assert!(d.pixel_aligned_symbols().is_empty());
    }
}
