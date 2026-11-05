use super::super::brush;
use super::art::{four, sane, two};
use super::types::{
    FLIP, GState, Kind, MAX_HEX, MAX_NOTE, MAX_PATTERNS, MAX_PLACED_FOLDER, MAX_STACK, MAX_SUBREADERS, PatternHead, Place, Raster, SKIPPED,
    VERSIONED_UNENDED,
};
use super::{Arc, Lexer, Node, PatternDef, Reader, Rect, Tok, V, marker, obj};

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
        // The header's comments (AI7 specification §2.1.1; DSC 3.0).
        let more = self.custom_more.take();
        if !self.header_done && self.frames.is_empty() {
            let bytes = c.trim_ascii_end();
            if let (Some(cmyk), Some(v)) = (more, bytes.strip_prefix(b"%+")) {
                self.custom_color(v, cmyk);
                return Ok(());
            }
            if let Some(v) = bytes.strip_prefix(b"%CMYKCustomColor:") {
                self.custom_color(v, true);
                return Ok(());
            }
            if let Some(v) = bytes.strip_prefix(b"%RGBCustomColor:") {
                self.custom_color(v, false);
                return Ok(());
            }
            if let Some(v) = bytes.strip_prefix(b"%For:") {
                // `(username) (organization)`: the user who created the file.
                if self.author.is_none() {
                    self.author = ps_string(v).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                }
                return Ok(());
            }
            if bytes == b"%EndComments" {
                self.header_done = true;
            }
        }
        if self.note_comment(c) {
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
        // Brushes and their art (those in a pattern's or a brush's art aren't read).
        if m == "BeginPluginObject" && self.pattern.is_none() {
            let section = self.section("EndPluginObject")?;
            self.plugin_object(section);
            return Ok(());
        }
        if m == "BeginBrushPattern" && self.pattern.is_none() {
            let section = self.section("EndBrushPattern")?;
            self.define_brush_art(section);
            return Ok(());
        }
        if m == "BeginArtStyles" {
            let section = self.section("EndArtStyles")?;
            self.styles = crate::import::ai::style::art_styles(section);
            return Ok(());
        }
        if m == "BeginArtStyleList" {
            let section = self.section("EndArtStyleList")?;
            self.style_list = crate::import::ai::style::art_style_list(section);
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
            self.multi_mask_layer_starts()?;
            self.open(Kind::Layer(Box::default()), false)?;
            self.multi_mask_go_on();
            return Ok(());
        }
        if line.starts_with("AI5_BeginRaster") {
            // An `XG` outside a raster object names no image in it.
            self.image_link = None;
            self.raster = Some(Raster::default());
        } else if line.starts_with("AI5_EndRaster") {
            // A file `XG` named in this raster object is no later one's.
            self.image_link = None;
            if let Some(Raster { operands: Some(vals), hex, .. }) = self.raster.take() {
                return self.image_from(vals, &hex);
            }
        } else if line.starts_with("AI5_BeginPlace") {
            // One that is still open (no `~` ended it) isn't read.
            let unseen = hidden || self.gs.hidden || !self.shown();
            if let Some(open) = self.place.replace(Place { hidden: unseen, ..Place::default() }) {
                self.unreadable("placed files", open.hidden);
            }
        } else if line.starts_with("AI5_EndPlace") {
            // One that `~` didn't end isn't read.
            if self.place.is_some() {
                self.end_place(hidden)?;
            }
        } else if m == "BeginPlacedRelativePath" {
            let at = self.stack.len();
            if let Some(p) = self.place.as_mut() {
                p.folder_at = Some(at);
            }
        } else if m == "EndPlacedRelativePath" {
            // The folder is the string between the markers, taken off the stack.
            let base = self.frames.last().map_or(0, |f| f.base);
            if let Some(at) = self.place.as_mut().and_then(|p| p.folder_at.take()).filter(|at| (base..=self.stack.len()).contains(at)) {
                let vals = self.stack.split_off(at);
                if let Some(p) = self.place.as_mut() {
                    p.folder = vals
                        .iter()
                        .find_map(|v| if let V::Str(s) = v { Some(super::obj::text(s)) } else { None })
                        .filter(|f| f.len() <= MAX_PLACED_FOLDER);
                }
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
        } else if let Some(v) = line.strip_prefix("AI3_TemplateBox:").or_else(|| line.strip_prefix("%TemplateBox:")) {
            self.template_center = two(v);
            self.template = four(v).or(self.template);
        } else if let Some(v) = line.strip_prefix("AI24_LargeCanvasScale:") {
            self.large_canvas_scale(v);
        } else if let Some(v) = line.strip_prefix("AI9_ColorModel:") {
            self.color_model = Some(v.trim() == "2");
        }
        Ok(())
    }

    /// `%AI24_LargeCanvasScale:` with its value `v`.
    fn large_canvas_scale(&mut self, v: &str) {
        // Illustrator's Large Canvas writes the editing data (art and boxes) at 1/N scale, N
        // being this; the printed page applies the same factor, so the art reaches the page's
        // size once it is applied to the art space ([`Self::ensure_space`]). A value outside
        // the range Illustrator writes is left alone.
        if let Ok(n) = v.trim().parse::<f64>()
            && (1.0..=100.0).contains(&n)
        {
            self.canvas_scale = n;
            if let Some(scale) = &mut self.large_canvas {
                *scale = n;
            }
        }
    }

    /// `%AI3_Note:` (or `%%Note:`) sets the note of the paths that follow, an empty one clearing it,
    /// and `%AI5_NoteMore:` adds a line to it. The text is kept as written, up to [`MAX_NOTE`]
    /// characters.
    fn note_comment(&mut self, c: &[u8]) -> bool {
        let (text, more) = if let Some(t) = c.strip_prefix(b"AI3_Note:").or_else(|| c.strip_prefix(b"%Note:")) {
            (t, false)
        } else if let Some(t) = c.strip_prefix(b"AI5_NoteMore:") {
            (t, true)
        } else {
            return false;
        };
        // Only as much of the line as the note can keep is decoded; a UTF-8 character the cut
        // splits is left out.
        let mut head = text.get(..text.len().min(4 * MAX_NOTE)).unwrap_or(text);
        if head.len() < text.len()
            && let Err(e) = std::str::from_utf8(head)
            && e.error_len().is_none()
        {
            head = head.get(..e.valid_up_to()).unwrap_or(head);
        }
        let text = super::obj::text(head);
        match (&mut self.gs.note, more) {
            (Some(note), true) => {
                note.push('\n');
                note.push_str(&text);
            }
            // A continuation with no note to continue.
            (None, true) => return true,
            (note, false) => *note = Some(text),
        }
        if let Some(note) = self.gs.note.as_mut()
            && let Some((at, _)) = note.char_indices().nth(MAX_NOTE)
        {
            note.truncate(at);
        }
        true
    }

    /// A custom color of the header, `cyan magenta yellow black (name)` (`cmyk`) or
    /// `red green blue (name)`, each component from 0 to 1. One written otherwise isn't kept.
    fn custom_color(&mut self, v: &[u8], cmyk: bool) {
        self.custom_more = Some(cmyk);
        if self.custom_colors.len() >= MAX_CUSTOM_COLORS {
            return;
        }
        let Some(open) = v.iter().position(|&b| b == b'(') else { return };
        let (Some(head), Some(name)) = (v.get(..open), v.get(open..).and_then(ps_string)) else { return };
        // One more than a color has, so that too many are seen.
        let parts: Vec<Option<f32>> = head
            .split(u8::is_ascii_whitespace)
            .filter(|w| !w.is_empty())
            .take(5)
            .map(|w| std::str::from_utf8(w).ok()?.parse::<f32>().ok().filter(|n| (0.0..=1.0).contains(n)))
            .collect();
        let color = match parts.as_slice() {
            [Some(c), Some(m), Some(y), Some(k)] if cmyk => vectorcraft_color::Color::cmyk(*c, *m, *y, *k),
            [Some(r), Some(g), Some(b)] if !cmyk => vectorcraft_color::Color::rgb(*r, *g, *b),
            _ => return,
        };
        if !name.trim().is_empty() && !self.custom_colors.iter().any(|n| n.name == name) {
            self.custom_colors.push(super::Named { name, color, tint: 1.0, spot: true });
        }
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
        self.keep_pattern(sub, whole, false);
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
        self.keep_pattern(sub, whole, true);
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
            || !self.marks.is_empty()
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

    /// Keep the pattern `sub` read, when it read it `whole`, as a pattern swatch; `v7` when it was
    /// defined in the version 7 form.
    fn keep_pattern(&mut self, mut sub: Reader<'_>, whole: bool, v7: bool) {
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
        if v7 {
            self.v7_patterns.insert(def.name.clone());
        } else {
            self.v7_patterns.remove(&def.name);
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

    /// A plug-in object (`section`: what follows `%AI8_BeginPluginObject`): the brush manager's
    /// order or a brush ([`brush`]); other objects are left alone. A brush without a name is counted
    /// as not read.
    fn plugin_object(&mut self, section: &[u8]) {
        let Some((kind, name, params)) = brush::plugin_strings(section) else { return };
        if kind == brush::MANAGER {
            if name.is_some() {
                self.brush_objects = true;
                self.brush_order = brush::parse_brush_manager(&params);
            }
            return;
        }
        let Some(tool) = brush::tool(&kind) else { return };
        self.brush_objects = true;
        let Some(name) = name else {
            self.brushes_unread += 1;
            return;
        };
        if self.brush_drafts.len() >= brush::MAX_BRUSHES {
            self.brushes_unread += 1;
            return;
        }
        match brush::parse_tool_object(tool, &name, &params) {
            Ok(d) => self.brush_drafts.push(d),
            Err(_) => self.brushes_unread += 1,
        }
    }

    /// A brush's art (`section`: what follows `%AI8_BeginBrushPattern`, its name as a string, then
    /// the art), read by a reader of its own as a pattern's is, in a space with y down. Art it can't
    /// read whole isn't kept: the brushes that use it aren't read.
    fn define_brush_art(&mut self, section: &[u8]) {
        if self.brush_art_sections >= brush::MAX_BRUSH_ART || self.subreader_depth >= MAX_SUBREADERS {
            return;
        }
        self.brush_art_sections += 1;
        let mut lex = Lexer::new(section);
        let Some(Tok::Str(name) | Tok::Hex(name)) = lex.next_token().map(|t| t.tok) else { return };
        let name = obj::text(&name);
        let mut sub = Reader::new(lex.slice(lex.offset(), section.len()));
        // In a space of its own (y down), with no head to take.
        sub.pattern = Some(PatternHead { taken: true, ..PatternHead::default() });
        sub.subreader_depth = self.subreader_depth + 1;
        // The gradients and patterns defined so far, lent.
        sub.gradients = std::mem::take(&mut self.gradients);
        sub.pattern_tiles = std::mem::take(&mut self.pattern_tiles);
        sub.nodes = self.nodes;
        sub.template = self.template;
        let read = sub.open(Kind::Group, false).and_then(|()| sub.run());
        self.nodes = sub.nodes;
        self.gradients = std::mem::take(&mut sub.gradients);
        self.pattern_tiles = std::mem::take(&mut sub.pattern_tiles);
        // Images aren't kept: the document's images don't count a brush's art among their users. Art
        // read with a note (something left out of it) isn't kept either.
        let whole = read.is_ok()
            && sub.unsupported.is_empty()
            && sub.unknown.is_empty()
            && sub.unread.is_empty()
            && sub.hidden_unread.is_empty()
            && sub.warnings.is_empty()
            && sub.frames.len() == 1
            && sub.doc.images.is_empty();
        let Some(frame) = sub.frames.pop().filter(|f| whole && !f.children.is_empty()) else { return };
        let mut art: Vec<Arc<Node>> = frame.children.iter().map(|n| Arc::new(self.doc.reid(n))).collect();
        let node = match art.len() {
            1 => art.pop().map(Arc::unwrap_or_clone),
            _ => Some(Node::group(self.doc.alloc_id(), art)),
        };
        let Some(node) = node else { return };
        self.brush_art.insert(name, node);
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
    // `] E` ends the definition: nothing but comments may follow it.
    if !word(next(), b"E") || !matches!(next_operand(&mut lex), Ok(None)) {
        return Legacy::Malformed;
    }
    Legacy::Yes(LegacyPattern { name, tile: Rect::new(x0, y0, x1, y1), layers })
}

/// How many custom colors the header may define.
pub(super) const MAX_CUSTOM_COLORS: usize = 1000;

/// The longest string the header's comments may give, in bytes.
const MAX_STRING: usize = 4096;

/// The PostScript string `v` starts with (after spaces), its escapes read, when it ends on the line
/// and holds at most [`MAX_STRING`] bytes.
fn ps_string(v: &[u8]) -> Option<String> {
    let mut bytes = v.trim_ascii_start().iter().copied().peekable();
    if bytes.next()? != b'(' {
        return None;
    }
    let (mut out, mut depth) = (Vec::new(), 0usize);
    while let Some(b) = bytes.next() {
        if out.len() > MAX_STRING {
            return None;
        }
        match b {
            b'\\' => match bytes.next()? {
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'b' => out.push(8),
                b'f' => out.push(12),
                d @ b'0'..=b'7' => {
                    let mut n = u32::from(d - b'0');
                    for _ in 0..2 {
                        match bytes.peek() {
                            Some(&o @ b'0'..=b'7') => {
                                n = n * 8 + u32::from(o - b'0');
                                bytes.next();
                            }
                            _ => break,
                        }
                    }
                    out.push((n & 0xff) as u8);
                }
                other => out.push(other),
            },
            b'(' => {
                depth += 1;
                out.push(b);
            }
            b')' => {
                let Some(d) = depth.checked_sub(1) else { return Some(super::obj::text(&out)) };
                depth = d;
                out.push(b);
            }
            _ => out.push(b),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postscript_strings_read_their_escapes_and_end_on_the_line() {
        assert_eq!(ps_string(b" (Ann \\(R\\) (x) \\101\\0618) (Org)").as_deref(), Some("Ann (R) (x) A18"));
        assert_eq!(ps_string(b"(a\\nb\\\\)").as_deref(), Some("a\nb\\"));
        assert_eq!(ps_string(b"(never ends \\)"), None);
        assert_eq!(ps_string(b"(ends in a backslash\\"), None);
        assert_eq!(ps_string(b"no string"), None);
        let long = format!("({})", "x".repeat(MAX_STRING + 1));
        assert_eq!(ps_string(long.as_bytes()), None);
        let longest = format!("({})", "x".repeat(MAX_STRING));
        assert_eq!(ps_string(longest.as_bytes()).map(|s| s.len()), Some(MAX_STRING));
    }

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

    #[test]
    fn brush_art_stops_at_subreader_limit() {
        let art = b"(A)\n0 g\n0 0 m\n4 0 L\n4 4 L\nf\n";
        let mut reader = Reader::new(b"");
        reader.define_brush_art(art);
        assert_eq!(reader.brush_art.len(), 1);
        let mut reader = Reader::new(b"");
        reader.subreader_depth = MAX_SUBREADERS;
        reader.define_brush_art(art);
        assert!(reader.brush_art.is_empty());
    }

    #[test]
    fn brush_art_sections_stop_at_their_limit_with_repeated_names() {
        let mut reader = Reader::new(b"");
        reader.brush_art_sections = brush::MAX_BRUSH_ART - 1;
        reader.define_brush_art(b"(A)\n0 g\n0 0 m\n4 0 L\n4 4 L\nf\n");
        let first = reader.brush_art.get("A").map(|n| n.id);
        assert!(first.is_some());
        // The same name again is past the limit: the first art stays.
        reader.define_brush_art(b"(A)\n0 g\n0 0 m\n8 0 L\n8 8 L\nf\n");
        assert_eq!(reader.brush_art.get("A").map(|n| n.id), first);
    }

    #[test]
    fn brushes_sharing_art_stop_at_the_budget_of_copied_nodes() {
        let mut reader = Reader::new(b"");
        reader.define_brush_art(b"(A)\n0 g\n0 0 m\n4 0 L\n4 4 L\nf\n");
        let nodes = reader.brush_art.get("A").map(brush::art_weight).unwrap_or_default();
        assert!(nodes > 0);
        let draft = |i: usize| brush::BrushDraft {
            name: format!("Brush {i}"),
            kind: brush::DraftKind::Art { brush: vectorcraft_brush::ArtBrush::default(), art: "A".to_string() },
        };
        let drafts: Vec<_> = (0..5).map(draft).collect();
        let (kept, left_out) = brush::finish_brushes_within(drafts.clone(), &reader.brush_art, None, 3 * nodes);
        assert_eq!((kept.len(), left_out), (3, 2));
        let (kept, left_out) = brush::finish_brushes_within(drafts, &reader.brush_art, None, 5 * nodes);
        assert_eq!((kept.len(), left_out), (5, 0));
        // A second brush of the same name is left out without using the budget.
        let mut repeated: Vec<_> = (0..2).map(draft).collect();
        repeated.insert(1, draft(0));
        let (kept, left_out) = brush::finish_brushes_within(repeated, &reader.brush_art, None, 2 * nodes);
        assert_eq!((kept.len(), left_out), (2, 1));
    }

    #[test]
    fn brush_art_with_an_unread_object_is_not_kept() {
        // A placed file the data keeps no preview of, shown and hidden.
        let placed =
            "%AI5_BeginPlace\n%%FileType: 1\n[5 0 0 -5 0 200] 10 20 14 22 4 4 0\n(missing.png)`\n%%IncludeFile:missing.png\n~\n%AI5_EndPlace\n";
        let hidden: String = placed.lines().map(|l| format!("%_{l}\n")).collect();
        for placed in [placed, hidden.as_str()] {
            let art = format!("(A)\n0 g\n0 0 m\n4 0 L\n4 4 L\nf\n{placed}");
            let mut reader = Reader::new(b"");
            reader.define_brush_art(art.as_bytes());
            assert!(reader.brush_art.is_empty(), "{placed:?}");
        }
    }
}
