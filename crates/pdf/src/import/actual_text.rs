//! Bounded ownership of authored replacement text (ISO 32000-1, 14.9.4).

use super::state::Builder;
use super::{Affine, Look, Placement, Shape, TextLine, Upright};
use crate::import_text::TextContent;

/// Only one replacement is retained: the outermost owner, even after it is consumed.
/// Depth is counted separately so ignored deep tags still unwind to the right owner.
#[derive(Default)]
pub(super) struct ActualText {
    depth: usize,
    owner: Option<usize>,
    pending: Option<String>,
    /// The owner's replacement is empty: its content has no text, so its glyphs stay art
    /// ([`Self::textless`], #6971).
    empty: bool,
}

impl ActualText {
    pub(super) fn begin(&mut self) {
        self.depth = self.depth.saturating_add(1);
    }

    pub(super) fn end(&mut self) {
        if self.owner == Some(self.depth) {
            self.owner = None;
            self.pending = None;
            self.empty = false;
        }
        self.depth = self.depth.saturating_sub(1);
    }

    pub(super) fn active(&self) -> bool {
        self.owner.is_some()
    }

    /// Inside a span whose replacement text is empty: the glyphs there are no text, but they
    /// still draw, so they come in as outlines rather than as type with no characters (#6971).
    pub(super) fn textless(&self) -> bool {
        self.owner.is_some() && self.empty
    }

    pub(super) fn set(&mut self, bytes: &[u8]) -> bool {
        if self.active() {
            return true;
        }
        // Bound before decoding: at most 64 KiB encoded / 192 KiB UTF-8, and no
        // allocation per nested tag. Oversized/deep replacements use ordinary mappings.
        if self.depth == 0 || self.depth > 128 || bytes.len() > 64 * 1024 {
            return false;
        }
        let text = crate::import_scan::text_string(bytes);
        self.owner = Some(self.depth);
        self.empty = text.is_empty();
        self.pending = Some(text);
        true
    }
}

impl Builder<'_> {
    /// The authored path never enters ordinary visual glyph reordering. The font was matched
    /// against the glyph's original Unicode, not against this replacement's characters.
    pub(super) fn actual_glyph(&mut self, o: &craft_pdf::interpret::font::OutlineGlyph, m: Affine, at: Placement, look: Look, opacity: f32) -> bool {
        let key = o.font_cache_key();
        self.flush_glyphs();
        // A fill and stroke callback for the same glyph consumes its text only once.
        if let (Some(stroke), Some(line)) = (&look.stroke, &mut self.text)
            && look.fill.is_none()
            && line.stroke_actual(key, at, stroke.clone(), look.overprint_stroke)
        {
            return true;
        }
        let width = o.advance_width().filter(|w| w.is_finite());
        let advance = width.map_or(at.size * 0.5, |w| w as f64 / 1000.0 * at.size * at.h_scale / 100.0);
        let top = width.filter(|w| w.abs() <= 1.0).map(|_| {
            let vx = self.fonts.get(&key).and_then(|f| f.vertical.as_ref()).map_or(500.0, |v| v.centre(o.glyph_id().to_u32()));
            Upright { top: m * kurbo::Point::new(vx, 880.0) }
        });
        // Keep pending until placement succeeds, including across a line flush.
        let pending = self.actual.pending.take();
        let content = match pending.as_deref() {
            Some(text) if !text.is_empty() => TextContent::Actual(text),
            _ => TextContent::Pen,
        };
        let place = |line: &mut TextLine| match top {
            Some(top) => line.push_upright_content(&look, at, opacity, top, content),
            None => line.push_content(&look, at, opacity, advance, content),
        };
        if !self.text.as_mut().is_some_and(place) {
            self.flush_text();
            let mut line = TextLine::new(at, opacity);
            if !place(&mut line) {
                self.actual.pending = pending;
                return false;
            }
            self.text = Some(line);
        }
        let mut ink = o.outline();
        if let Some(line) = &mut self.text
            && !ink.elements().is_empty()
        {
            ink.apply_affine(m);
            line.ink.push(ink.bounding_box());
        }
        true
    }

    /// A Type 3 glyph under an active ActualText span.
    pub(super) fn actual_type3_glyph(&mut self, key: u128, at: Placement, look: Look, opacity: f32, advance: f64) -> bool {
        self.flush_glyphs();
        self.flush_type3();
        if let (Some(stroke), Some(line)) = (&look.stroke, &mut self.text)
            && look.fill.is_none()
            && line.stroke_actual(key, at, stroke.clone(), look.overprint_stroke)
        {
            return true;
        }
        let pending = self.actual.pending.take();
        let content = match pending.as_deref() {
            Some(text) if !text.is_empty() => TextContent::Actual(text),
            _ => TextContent::Pen,
        };
        let place = |line: &mut TextLine| line.push_content(&look, at, opacity, advance, content);
        if !self.text.as_mut().is_some_and(place) {
            self.flush_text();
            let mut line = TextLine::new(at, opacity);
            if !place(&mut line) {
                self.actual.pending = pending;
                return false;
            }
            self.text = Some(line);
        }
        if let Some(line) = &mut self.text {
            line.has_type3 = true;
        }
        true
    }
}
