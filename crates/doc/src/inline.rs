//! Inline graphics in text ([`crate::TextRun::inline`]): resolving each inline symbol's art bounds
//! from the document's symbols, which the text layout needs to size and place it.

use std::borrow::Cow;

use vectorcraft_geom::{Affine, Rect};

use crate::{Document, NodeId, NodeKind, TextObject};

/// The document key under which the app records each symbol's natural size (`{name: [w, h]}`).
pub const SYMBOL_SIZES: &str = "symbolSizes";
/// Half the side of the square symbol art is normalised to.
pub const SYMBOL_HALF: f64 = 10.0;

/// Usable art bounds: finite, non-degenerate.
fn usable(r: Option<Rect>) -> Option<Rect> {
    r.filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) && r.width() >= 0.0 && r.height() > 1e-9)
}

impl Document {
    /// Symbol space → the symbol's art at its natural size. Symbols made in the app keep their
    /// art in a 20 × 20 square around the origin and their size as drawn in the document's
    /// `symbolSizes` (an instance is placed with that scale); other symbols are as they are.
    pub fn symbol_natural_xf(&self, name: &str) -> Affine {
        let size = self.unknown.get(SYMBOL_SIZES).and_then(|m| m.get(name)).and_then(|v| Some((v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)));
        match size {
            Some((w, h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => {
                Affine::scale_non_uniform(w / (2.0 * SYMBOL_HALF), h / (2.0 * SYMBOL_HALF))
            }
            _ => Affine::IDENTITY,
        }
    }

    /// Visual bounds of the art of the symbol named `name` at its natural size (see
    /// [`Self::symbol_natural_xf`]), if it exists and has a usable extent.
    pub fn symbol_art_bounds(&self, name: &str) -> Option<Rect> {
        let s = self.symbols.iter().find(|s| s.name == name)?;
        let b = usable(s.art.visual_bounds().or_else(|| s.art.geometric_bounds()))?;
        usable(Some(self.symbol_natural_xf(name).transform_rect_bbox(b)))
    }

    /// Does `t` have inline graphics whose resolved bounds differ from the document's symbols?
    fn inline_stale(&self, t: &TextObject) -> bool {
        t.runs.iter().filter_map(|r| r.inline.as_ref()).any(|a| a.bounds != self.symbol_art_bounds(&a.symbol))
    }

    /// `t` with its inline graphics' art bounds resolved from this document's symbols (borrowed
    /// when nothing changes). Renderers and exporters lay text out through this, so they never
    /// depend on the cached bounds being current.
    pub fn inline_resolved<'a>(&self, t: &'a TextObject) -> Cow<'a, TextObject> {
        if !self.inline_stale(t) {
            return Cow::Borrowed(t);
        }
        let mut t = t.clone();
        self.resolve_text_inline(&mut t);
        Cow::Owned(t)
    }

    /// Fill in the art bounds of `t`'s inline graphics; returns whether any changed (the text's
    /// cached layout bounds are then cleared).
    pub fn resolve_text_inline(&self, t: &mut TextObject) -> bool {
        let mut changed = false;
        for r in &mut t.runs {
            if let Some(a) = &mut r.inline {
                let b = self.symbol_art_bounds(&a.symbol);
                if a.bounds != b {
                    a.bounds = b;
                    changed = true;
                }
            }
        }
        if changed {
            t.cached_bounds = None;
            t.cached_ink = None;
        }
        changed
    }

    /// Resolve the inline graphics of every text object in the layers (after opening a document
    /// or changing its symbols). Returns the ids of the texts that changed: their cached layout
    /// bounds were cleared and need recomputing.
    pub fn resolve_inline_art(&mut self) -> Vec<NodeId> {
        let mut stale = vec![];
        self.walk(|n| {
            if let NodeKind::Text(t) = &n.kind
                && t.runs.iter().any(|r| r.inline.is_some())
                && self.inline_stale(t)
            {
                stale.push(n.id);
            }
        });
        let mut out = vec![];
        for id in stale {
            let Some(mut t) = self.node(id).and_then(|n| match &n.kind {
                NodeKind::Text(t) => Some(t.clone()),
                _ => None,
            }) else {
                continue;
            };
            if self.resolve_text_inline(&mut t)
                && let Some(n) = self.node_mut(id)
            {
                n.kind = NodeKind::Text(t);
                out.push(id);
            }
        }
        out
    }
}
