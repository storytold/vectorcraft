use super::art::{four, sane, two};
use super::types::{FLIP, GState, Kind, MAX_HEX, MAX_PATTERNS, MAX_STACK, MAX_SUBREADERS, PatternHead, Place, Raster, SKIPPED, VERSIONED_UNENDED};
use super::{Arc, Node, PatternDef, Reader, Rect, Tok, V, marker};

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

    pub(super) fn comment(&mut self, c: &[u8], hidden: bool) -> Result<(), String> {
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
        // A pattern's definition. One inside another's isn't read, and the outer tile is then incomplete.
        if m == "BeginPattern" {
            let section = self.section("EndPattern")?;
            if self.pattern.is_some() {
                self.unsupported.insert("patterns inside patterns".into());
            } else {
                self.define_pattern(section);
            }
            return Ok(());
        }
        if m == "BeginSymbol" {
            let section = self.section("EndSymbol")?;
            self.symbol_bounds(section);
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
        // Versioned content holds alternatives of the same art, the newest first: the first is
        // read, and the next one (`%AI17_Alternate_Content`, or `…_if_version_gt:` for a version
        // in between) starts what is skipped, up to the end of the block, nested blocks included.
        if m.starts_with("Alternate_Content") {
            let at = self.lex.offset();
            if self.skip_to("End_Versioned_Content").is_err() {
                // A block that doesn't end: the rest of the file is read rather than lost.
                self.lex.seek(at);
                self.warn(VERSIONED_UNENDED);
            }
            return Ok(());
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
            if self.place.is_some() {
                self.end_place(hidden)?;
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
    /// read whole (an operator it doesn't know, type, a pattern it doesn't have, unbalanced groups,
    /// no tile) isn't kept: art
    /// filled with it can't be read.
    fn define_pattern(&mut self, section: &[u8]) {
        if self.pattern_tiles.len() >= MAX_PATTERNS || self.subreader_depth >= MAX_SUBREADERS {
            return;
        }
        match legacy_pattern(section) {
            Legacy::No => {}
            Legacy::Malformed => return,
            Legacy::Yes(def) => return self.define_legacy_pattern(&def),
        }
        let mut sub = Reader::new(section);
        sub.subreader_depth = self.subreader_depth + 1;
        sub.pattern = Some(PatternHead::default());
        // The gradients defined so far, lent (and those it defines kept).
        sub.gradients = std::mem::take(&mut self.gradients);
        sub.nodes = self.nodes;
        sub.template = self.template;
        let read = sub.open(Kind::Group, false).and_then(|()| sub.run());
        self.nodes = sub.nodes;
        self.gradients = std::mem::take(&mut sub.gradients);
        // Unread art in the definition, hidden or not, leaves the tile incomplete.
        let whole = read.is_ok()
            && sub.unsupported.is_empty()
            && sub.unknown.is_empty()
            && sub.unread.is_empty()
            && sub.hidden_unread.is_empty()
            && sub.frames.len() == 1;
        self.keep_pattern(sub, whole);
    }

    /// A definition in the version 7 form ([`legacy_pattern`]), read layer by layer into one group:
    /// each layer's colors (`@`) set the state its tile string (`&`) is read with, or the fill of the
    /// tile's box (`_ &`). Each layer starts from the state the definition starts with.
    fn define_legacy_pattern(&mut self, def: &LegacyPattern) {
        let mut sub = Reader::new(b"");
        sub.subreader_depth = self.subreader_depth + 1;
        sub.pattern = Some(PatternHead { taken: true, name: def.name.clone(), tile: Some(def.tile) });
        sub.gradients = std::mem::take(&mut self.gradients);
        sub.nodes = self.nodes;
        sub.template = self.template;
        let mut read = sub.open(Kind::Group, false);
        for layer in &def.layers {
            if read.is_err() {
                break;
            }
            read = sub.pattern_layer(layer, def.tile);
        }
        self.nodes = sub.nodes;
        self.gradients = std::mem::take(&mut sub.gradients);
        let whole = read.is_ok()
            && sub.unsupported.is_empty()
            && sub.unknown.is_empty()
            && sub.unread.is_empty()
            && sub.hidden_unread.is_empty()
            && sub.frames.len() == 1;
        self.keep_pattern(sub, whole);
    }

    /// One layer of a definition in the version 7 form, read into the definition's group.
    fn pattern_layer(&mut self, layer: &'a LegacyLayer, tile: Rect) -> Result<(), String> {
        self.gs = GState::default();
        self.pending_fill = None;
        self.pending_stroke = None;
        self.pending_unread_fill = false;
        self.pending_unread_stroke = false;
        self.instance = None;
        self.end_path_quietly();
        // The color part has the overprint and the colors only.
        let mut lex = super::Lexer::new(&layer.color);
        while let Some(t) = lex.next_token() {
            match t.tok {
                Tok::Num(_) | Tok::Str(_) | Tok::Hex(_) => {}
                Tok::Word(w) if LAYER_COLOR_OPS.contains(&w) => {}
                _ => return Err("a pattern layer's colors hold something else".into()),
            }
        }
        self.run_string(&layer.color)?;
        match &layer.tile {
            Some(art) => self.run_string(art)?,
            None => {
                let corner = |x: f64, y: f64| [V::Num(x), V::Num(y)];
                self.path_op("m", &corner(tile.x0, tile.y0))?;
                for (x, y) in [(tile.x0, tile.y1), (tile.x1, tile.y1), (tile.x1, tile.y0)] {
                    self.path_op("L", &corner(x, y))?;
                }
                self.paint("f", false, false)?;
            }
        }
        // A path the layer leaves unpainted paints nothing.
        self.end_path_quietly();
        Ok(())
    }

    /// Run this reader on one of a pattern layer's strings, which must end everything it starts.
    fn run_string(&mut self, bytes: &'a [u8]) -> Result<(), String> {
        let depth = self.frames.len();
        self.lex = super::Lexer::new(bytes);
        let read = self.run();
        let open = self.done
            || self.frames.len() != depth
            || self.stack.iter().any(|v| matches!(v, V::Mark))
            || self.text.is_some()
            || self.raster.is_some()
            || self.place.is_some()
            || self.mesh.is_some()
            || self.gradient_def.is_some();
        self.stack.clear();
        self.marks.clear();
        read?;
        if open {
            return Err("a pattern layer doesn't end what it starts".into());
        }
        Ok(())
    }

    /// Keep the pattern `sub` read, when it read it `whole`, as a pattern swatch.
    fn keep_pattern(&mut self, mut sub: Reader<'_>, whole: bool) {
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

    /// Read a definition's box for an instance whose art must come from the page.
    fn symbol_bounds(&mut self, section: &[u8]) {
        if self.symbol_bounds.len() >= MAX_PATTERNS || self.subreader_depth >= MAX_SUBREADERS {
            return;
        }
        let mut lex = super::Lexer::new(section);
        let name = match lex.next_token().map(|t| t.tok) {
            Some(Tok::Str(bytes)) => super::obj::text(&bytes),
            _ => return,
        };
        let mut sub = Reader::new(section);
        sub.subreader_depth = self.subreader_depth + 1;
        sub.cropmarks = self.cropmarks;
        sub.template = self.template;
        sub.nodes = self.nodes;
        let read = sub.open(Kind::Group, false).and_then(|()| sub.run());
        self.nodes = sub.nodes;
        if read.is_err() || !sub.unsupported.is_empty() || !sub.unknown.is_empty() || sub.frames.len() != 1 {
            return;
        }
        let Some(frame) = sub.frames.pop() else { return };
        let bounds = frame.children.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.visual_bounds()));
        let Some(bounds) = bounds.map(|b| sub.space().inverse().transform_rect_bbox(b)) else { return };
        if sane(bounds) && bounds.area() > 0.0 {
            self.symbol_bounds.insert(name, bounds);
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

/// The operators a pattern layer's colors (`@`) may hold: overprint, gray, process and custom colors.
const LAYER_COLOR_OPS: &[&[u8]] = &[b"O", b"R", b"g", b"G", b"k", b"K", b"x", b"X"];

/// The longest string a pattern layer may have (the format's limit, 64K bytes).
const MAX_LAYER_STRING: usize = 65_535;

/// Most layers in one pattern's definition.
const MAX_LAYERS: usize = 65_536;

/// A pattern's definition in the version 7 form: its head, then a list of layers painted in order,
/// the first the lowest.
struct LegacyPattern {
    name: String,
    /// The tile in the pattern's space (y up).
    tile: Rect,
    layers: Vec<LegacyLayer>,
}

/// `(colors) @ (tile) &`, or `(colors) @ _ &` (no tile: the tile's box is filled with the colors' fill).
struct LegacyLayer {
    color: Vec<u8>,
    tile: Option<Vec<u8>>,
}

enum Legacy {
    /// Another form: the definition's art follows its head as operators.
    No,
    /// The version 7 form, which can't be read.
    Malformed,
    Yes(LegacyPattern),
}

/// The next token that isn't a comment; `Err` at a pattern's definition inside this one.
fn next_operand<'s>(lex: &mut super::Lexer<'s>) -> Result<Option<super::super::lex::Token<'s>>, ()> {
    while let Some(t) = lex.next_token() {
        match t.tok {
            Tok::Comment(c) if marker(&String::from_utf8_lossy(c)) == "BeginPattern" => return Err(()),
            Tok::Comment(_) => {}
            _ => return Ok(Some(t)),
        }
    }
    Ok(None)
}

/// A pattern's definition in the version 7 form: `(name) llx lly urx ury [`, then for each layer
/// `(colors) @` and `(tile) &` or `_ &`, then `] E` (version 7 specification §3.4). It is that form
/// when a `[` that isn't on a `%_` line follows the head.
fn legacy_pattern(section: &[u8]) -> Legacy {
    let mut lex = super::Lexer::new(section);
    let mut next = || next_operand(&mut lex).ok().flatten().map(|t| (t.tok, t.hidden));
    let name = match next() {
        Some((Tok::Str(s), _)) => super::obj::text(&s),
        _ => return Legacy::No,
    };
    let mut tile = [0.0; 4];
    for v in &mut tile {
        match next() {
            Some((Tok::Num(n), _)) => *v = n,
            _ => return Legacy::No,
        }
    }
    if !matches!(next(), Some((Tok::Word(b"["), false))) {
        return Legacy::No;
    }
    let [x0, y0, x1, y1] = tile;
    let mut layers = vec![];
    let string = |t: Option<(Tok, bool)>| match t {
        Some((Tok::Str(s) | Tok::Hex(s), _)) if s.len() <= MAX_LAYER_STRING => Some(s),
        _ => None,
    };
    let word = |t: Option<(Tok, bool)>, w: &[u8]| matches!(t, Some((Tok::Word(x), _)) if x == w);
    loop {
        // Before the first layer, anything but a string or `] E` is the newer form's art (`[]0 d`).
        let first = layers.is_empty();
        let color = match next() {
            Some((Tok::Word(b"]"), _)) if !first => break,
            Some((Tok::Word(b"]"), _)) => {
                return match next() {
                    Some((Tok::Word(b"E"), _)) => Legacy::Yes(LegacyPattern { name, tile: Rect::new(x0, y0, x1, y1), layers }),
                    _ => Legacy::No,
                };
            }
            t @ Some((Tok::Str(_) | Tok::Hex(_), _)) => match string(t) {
                Some(s) => s,
                None => return Legacy::Malformed,
            },
            _ if first => return Legacy::No,
            _ => return Legacy::Malformed,
        };
        if !word(next(), b"@") {
            return Legacy::Malformed;
        }
        let tile = match next() {
            Some((Tok::Word(b"_"), _)) => None,
            t => match string(t) {
                Some(s) => Some(s),
                None => return Legacy::Malformed,
            },
        };
        if !word(next(), b"&") || layers.len() >= MAX_LAYERS {
            return Legacy::Malformed;
        }
        layers.push(LegacyLayer { color, tile });
    }
    if !word(next(), b"E") {
        return Legacy::Malformed;
    }
    Legacy::Yes(LegacyPattern { name, tile: Rect::new(x0, y0, x1, y1), layers })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deeply_nested_symbol_definitions_stop_at_subreader_limit() {
        let mut definition = String::from("(Outer) 0 Ae\n");
        for i in 0..300 {
            definition.push_str(&format!("%AI14_BeginSymbol\n(S{i}) 0 Ae\n"));
        }
        definition.push_str("0 0 m\n4 0 L\n4 4 L\n0 4 L\nf\n");
        for _ in 0..300 {
            definition.push_str("%AI10_EndSymbol\n0 0 m\n4 0 L\n4 4 L\n0 4 L\nf\n");
        }
        let mut reader = Reader::new(b"");
        reader.subreader_depth = MAX_SUBREADERS;
        reader.symbol_bounds(definition.as_bytes());
        assert!(reader.symbol_bounds.is_empty());
    }
}
