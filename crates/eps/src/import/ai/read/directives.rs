use super::art::{four, sane, two};
use super::types::{FLIP, Kind, MAX_HEX, MAX_PATTERNS, MAX_STACK, PatternHead, Place, Raster, SKIPPED};
use super::*;

impl<'a> Reader<'a> {
    pub(super) fn push(&mut self, v: V) -> Result<(), String> {
        if self.stack.len() >= MAX_STACK {
            return Err("it has too many values in a row".into());
        }
        self.stack.push(v);
        Ok(())
    }

    /// The operands of the operator being run (above the innermost dictionary's), taken off.
    pub(super) fn operands(&mut self) -> Vec<V> {
        let base = self.frames.last().map_or(0, |f| f.base).min(self.stack.len());
        self.stack.split_off(base)
    }

    pub(super) fn nums(vals: &[V]) -> Vec<f64> {
        vals.iter().filter_map(V::num).collect()
    }

    // ---- comments --------------------------------------------------------------------------

    pub(super) fn comment(&mut self, c: &[u8]) -> Result<(), String> {
        let line = String::from_utf8_lossy(c);
        let line = line.trim_end();
        if let Some((len, hex)) = self.blend_record.as_mut() {
            if !line.is_empty() && line.bytes().all(|b| b.is_ascii_hexdigit()) {
                // As many digits as the record says it has, no more.
                let wanted = len.saturating_mul(2);
                let take = wanted.saturating_sub(hex.len()).min(line.len());
                hex.extend_from_slice(line.as_bytes().get(..take).unwrap_or_default());
                if hex.len() >= wanted {
                    self.end_blend();
                }
                return Ok(());
            }
            self.end_blend();
        }
        if let Some(r) = self.raster.as_mut()
            && r.operands.is_some()
            && !line.is_empty()
            && line.bytes().all(|b| b.is_ascii_hexdigit())
        {
            if r.hex.len() < MAX_HEX {
                r.hex.extend_from_slice(line.as_bytes());
            }
            return Ok(());
        }
        let m = marker(line);
        // A pattern's definition (one inside another's isn't read).
        if m == "BeginPattern" && self.pattern.is_none() {
            let section = self.section("EndPattern")?;
            self.define_pattern(section);
            return Ok(());
        }
        if m == "BeginArtStyles" {
            let section = self.section("EndArtStyles")?;
            self.styles = crate::import::ai::style::art_styles(section);
            return Ok(());
        }
        if m == "BeginPalette" {
            let section = self.section("EndPalette")?;
            self.palette = crate::import::ai::palette::palette(section);
            return Ok(());
        }
        if m == "BeginEncoding" {
            self.type_state.encoding_comment(line);
        }
        if let Some((_, end)) = SKIPPED.iter().find(|(b, _)| *b == m) {
            return self.skip_to(end);
        }
        match m {
            "%BeginProlog" => return self.skip_to("%EndProlog"),
            "%BeginResource" => return self.skip_to("%EndResource"),
            // The art ends with the page.
            "%PageTrailer" | "%Trailer" => {
                self.done = true;
                return Ok(());
            }
            _ => {}
        }
        if line.starts_with("AI5_BeginLayer") {
            return self.open(Kind::Layer(Box::default()), false);
        }
        if line.starts_with("AI5_BeginRaster") {
            self.raster = Some(Raster::default());
        } else if line.starts_with("AI5_EndRaster") {
            if let Some(Raster { operands: Some(vals), hex, .. }) = self.raster.take() {
                return self.image_from(vals, &hex);
            }
        } else if line.starts_with("AI5_BeginPlace") {
            self.place = Some(Place::default());
        } else if line.starts_with("AI5_EndPlace") {
            // One that `~` didn't end isn't read.
            if self.place.take().is_some() {
                self.unreadable("placed files");
            }
        } else if self.frames.is_empty()
            && let Some(v) = line.strip_prefix("%HiResBoundingBox:")
        {
            self.hires = four(v).or(self.hires);
        } else if self.frames.is_empty()
            && let Some(v) = line.strip_prefix("%BoundingBox:")
        {
            self.bbox = four(v).or(self.bbox);
        } else if let Some(v) = line.strip_prefix("AI3_Cropmarks:") {
            let n: Vec<f64> = v.split_whitespace().filter_map(|w| w.parse().ok()).collect();
            if let [x0, y0, x1, y1] = n.as_slice() {
                let r = Rect::new(*x0, *y0, *x1, *y1);
                if sane(r) {
                    self.cropmarks = Some(r);
                }
            }
        } else if self.frames.is_empty()
            && let Some(v) = line.strip_prefix("AI5_RulerUnits:")
            && let Ok(n) = v.trim().parse::<u8>()
        {
            self.ruler_units = Some(n);
        } else if let Some(v) = line.strip_prefix("AI5_ArtSize:") {
            self.art_size = two(v);
        } else if let Some(v) = line.strip_prefix("AI3_TemplateBox:") {
            self.template_center = two(v);
            self.template = four(v).or(self.template);
        } else if let Some(v) = line.strip_prefix("AI24_LargeCanvasScale:") {
            // Illustrator's Large Canvas writes the editing data (art and boxes) at 1/N scale, N
            // being this; the printed page applies the same factor, so the art reaches the page's
            // size once it is applied to the art space ([`Self::ensure_space`]). A value outside
            // the range Illustrator writes is left alone.
            if let (Some(scale), Ok(n)) = (&mut self.large_canvas, v.trim().parse::<f64>())
                && (1.0..=100.0).contains(&n)
            {
                *scale = n;
            }
        } else if let Some(v) = line.strip_prefix("AI9_ColorModel:") {
            self.color_model = Some(v.trim() == "2");
        }
        Ok(())
    }

    /// Skip to the comment with marker `end` (sections of the same kind nested in it count, and
    /// versioned content in an alternate).
    fn skip_to(&mut self, end: &str) -> Result<(), String> {
        self.section(end).map(|_| ())
    }

    /// [`Self::skip_to`] → what was skipped, up to the comment that ends it.
    fn section(&mut self, end: &str) -> Result<&'a [u8], String> {
        let begin = SKIPPED.iter().find(|(_, e)| *e == end).map_or("", |(b, _)| *b);
        let begin = if end == "End_Versioned_Content" { "Begin_Content_if_version_gt" } else { begin };
        let start = self.lex.offset();
        let mut depth = 0usize;
        loop {
            let before = self.lex.offset();
            let Some(t) = self.lex.next_token() else { break };
            if let Tok::Comment(c) = t.tok {
                let line = String::from_utf8_lossy(c);
                let m = marker(&line);
                if !begin.is_empty() && m == begin {
                    depth += 1;
                } else if m == end {
                    if depth == 0 {
                        return Ok(self.lex.slice(start, before));
                    }
                    depth -= 1;
                }
            }
        }
        Err(format!("its section ending with `%{end}` doesn't end"))
    }

    /// A pattern's definition (`section`: what follows `%AI3_BeginPattern`), read by a reader of its
    /// own into a pattern swatch, its art in the pattern's space (y down). A definition it can't
    /// read whole (an operator it doesn't know, type, unbalanced groups, no tile) isn't kept: art
    /// filled with it can't be read.
    fn define_pattern(&mut self, section: &[u8]) {
        if self.pattern_tiles.len() >= MAX_PATTERNS {
            return;
        }
        let mut sub = Reader::new(section);
        sub.pattern = Some(PatternHead::default());
        // The gradients defined so far, lent (and those it defines kept).
        sub.gradients = std::mem::take(&mut self.gradients);
        sub.nodes = self.nodes;
        sub.template = self.template;
        let read = sub.open(Kind::Group, false).and_then(|()| sub.run());
        self.nodes = sub.nodes;
        self.gradients = std::mem::take(&mut sub.gradients);
        let whole = read.is_ok() && sub.unsupported.is_empty() && sub.unknown.is_empty() && sub.frames.len() == 1;
        let (Some(head), Some(frame)) = (sub.pattern.take(), sub.frames.pop()) else { return };
        let Some(tile) = head.tile.filter(|t| whole && sane(*t) && t.width() > 0.0 && t.height() > 0.0 && !frame.children.is_empty()) else {
            return;
        };
        let art: Vec<Arc<Node>> = frame.children.iter().map(|n| Arc::new(self.doc.reid(n))).collect();
        let mut def = PatternDef::new(&head.name, art);
        def.tile = FLIP.transform_rect_bbox(tile);
        if self.pattern_tiles.insert(def.name.clone(), def.tile).is_some() {
            self.doc.patterns.retain(|p| p.name != def.name);
        }
        self.doc.patterns.push(def);
        for (key, blob) in sub.doc.images {
            self.doc.images.entry(key).or_insert(blob);
        }
        self.spots.extend(sub.spots);
        self.stop_spots.extend(sub.stop_spots);
        self.cmyk_colors += sub.cmyk_colors;
        self.rgb_colors += sub.rgb_colors;
        for w in &sub.warnings {
            self.warn(w);
        }
    }

    /// In a pattern's definition, the operands of its first operator start with its head,
    /// `(name) llx lly urx ury`: taken off them.
    pub(super) fn take_head(&mut self, mut vals: Vec<V>) -> Vec<V> {
        let Some(head) = self.pattern.as_mut().filter(|h| !h.taken) else { return vals };
        head.taken = true;
        let Some(at) = vals.iter().position(|v| matches!(v, V::Str(_))) else { return vals };
        let tile = Self::nums(vals.get(at + 1..at + 5).unwrap_or_default());
        let ([x0, y0, x1, y1], Some(name)) = (tile.as_slice(), vals.get(at).and_then(V::text)) else { return vals };
        head.name = name;
        head.tile = Some(Rect::new(*x0, *y0, *x1, *y1));
        vals.split_off(at + 5)
    }
}
