//! Symbols: their definitions, the symbol list and their instances (see [`crate::ai_editing`] for
//! the form).

use std::collections::HashSet;
use std::fmt::Write;

use vectorcraft_color::Paint;
use vectorcraft_doc::{Knockout, Node, NodeKind, SymbolExportType};
use vectorcraft_geom::Affine;

use super::{MAX_NEST, Writer, Xy};
use crate::import::MAX_SYMBOLS;
use crate::ps;

pub(in crate::ai_editing) const SYMBOL_ART: &str = "symbol instances with a fill, appearance changes, isolation, knockout or knockout shape of their own, instances of symbols with 9-slice guides or pattern paint, instances in pattern tiles or in art cut to a clipping object, instances scaled almost flat, instances that turn, mirror or scale symbols holding type, live objects, instances written as their art, shapes with rounded live corners or strokes written as outlines or brush art, instances that scale their symbol more than 100 times, instances on the layers of a large canvas and instances of symbols past 4,096 are written as their art in the editing data for other apps";
pub(in crate::ai_editing) const SYMBOL_OPTIONS: &str =
    "the 9-slice guides, export types and dynamic type of symbols are left out of the editing data for other apps";
pub(in crate::ai_editing) const MISSING_SYMBOL: &str =
    "symbol instances of symbols the document doesn't have are written as empty groups in the editing data for other apps";
/// The most an instance may scale its symbol's art, nested instances' scales included: the art's
/// numbers are written to four decimals, so a scale of 100 places them within 0.005 pt.
const MAX_SCALE: f64 = 100.0;
pub(in crate::ai_editing) const SYMBOL_COUNT: &str = "symbols past 4,096 are left out of the editing data for other apps";

/// Whether the canvas works `n` itself (not its children) out again after placing an instance:
/// type, live objects and shapes with rounded live corners, whose strokes, steps and corners depend
/// on where the instance puts them.
pub(super) fn redrawn(n: &Node) -> bool {
    let rounded = |l: &vectorcraft_doc::LiveShape| match l {
        vectorcraft_doc::LiveShape::Rectangle { radii, .. } => radii.iter().any(|r| *r > 0.0),
        vectorcraft_doc::LiveShape::Polygon { radii, .. } | vectorcraft_doc::LiveShape::Path { radii, .. } => radii.iter().any(|r| *r > 0.0),
        vectorcraft_doc::LiveShape::Ellipse { .. } | vectorcraft_doc::LiveShape::Line { .. } => false,
    };
    match &n.kind {
        NodeKind::Path { live: Some(l), .. } => rounded(l),
        NodeKind::Text(_)
        | NodeKind::Blend { .. }
        | NodeKind::Envelope { .. }
        | NodeKind::Mesh(_)
        | NodeKind::Repeat(_)
        | NodeKind::PlacedDocument(_)
        | NodeKind::Intertwine { .. } => true,
        _ => false,
    }
}

impl Writer<'_> {
    /// Whether instance `n` of symbol `symbol`, placed by `xf`, is written as an instance: not on the
    /// layers of a large canvas, in a pattern's tile or in art cut to a clipping object, without a
    /// fill or appearance changes of its own (which stain or change the art it draws) and without
    /// isolation, knockout or knockout shape of its own (which an instance's transparency doesn't
    /// hold), of a symbol without 9-slice guides and without pattern paint in its art (the canvas
    /// keeps an instance's pattern tiles where the document puts them, and a definition's art moves
    /// with each instance) whose definition can be written ([`Self::define_symbol`]), by a matrix
    /// that keeps the art's area as it is written and scales it as the definition allows
    /// ([`Self::keeps_scale`]).
    pub(super) fn live_instance(&mut self, n: &Node, symbol: &str, xf: Affine) -> bool {
        let own = vectorcraft_brush::stain(n).is_some() || !n.extras().symbol_overrides.is_empty();
        let group_state = n.isolate || n.knockout != Knockout::Neutral || n.knockout_shape;
        // The matrix as written (to ten decimals) must still keep the art's area.
        let m = Affine::new((self.instance_space * xf).as_coeffs().map(|v| (v * 1e10).round() / 1e10));
        (self.symbols_live || self.in_symbol)
            && self.defining.is_empty()
            && self.clip.is_none()
            && !own
            && !group_state
            && self.doc.symbol_nine_slice(symbol).is_none()
            && m.as_coeffs().iter().all(|v| v.is_finite())
            && m.determinant().abs() >= 1e-12
            && self.define_symbol(symbol)
            && !self.symbol_paints_pattern(symbol)
            && self.keeps_scale(symbol, xf)
    }

    /// Whether instance matrix `xf` places symbol `symbol`'s definition as the canvas draws the
    /// instance. A definition holding art the canvas works out again after placing an instance
    /// ([`redrawn`], strokes written as outlines or brush art, and instances written as their art)
    /// takes only matrices that move it;
    /// any other takes a scale up to [`MAX_SCALE`], nested instances' scales included. A live
    /// instance in a symbol's art passes both to that symbol.
    fn keeps_scale(&mut self, symbol: &str, xf: Affine) -> bool {
        let [a, b, c, d, ..] = xf.as_coeffs();
        let fixed = self.fixed_symbols.contains(symbol);
        let scale = a.hypot(b).max(c.hypot(d)) * self.symbol_scales.get(symbol).copied().unwrap_or(1.0);
        let keeps = if fixed { (a - 1.0).abs() < 1e-9 && b.abs() < 1e-9 && c.abs() < 1e-9 && (d - 1.0).abs() < 1e-9 } else { scale <= MAX_SCALE };
        if keeps && self.in_symbol {
            self.wrote_fixed |= fixed;
            self.inner_scale = self.inner_scale.max(scale);
        }
        keeps
    }

    /// Whether the art of symbol `name`, an opacity mask in it or the art of a symbol it holds an
    /// instance of, at any depth, fills or strokes something (type included) with a pattern, or its
    /// definition holds pattern tiles. Worked out once a symbol, after it is defined.
    fn symbol_paints_pattern(&mut self, name: &str) -> bool {
        if self.pattern_written.contains(name) {
            return true;
        }
        if let Some(known) = self.patterned_symbols.get(name) {
            return *known;
        }
        let mut seen = HashSet::from([name.to_string()]);
        let mut todo = vec![name.to_string()];
        let mut found = false;
        while let Some(next) = todo.pop() {
            let (own, used) = self.symbol_own_patterns(&next);
            if own {
                found = true;
                break;
            }
            for u in used {
                if seen.insert(u.clone()) {
                    todo.push(u);
                }
            }
        }
        self.patterned_symbols.insert(name.to_string(), found);
        found
    }

    /// Whether the art of symbol `name` or an opacity mask in it fills or strokes something (type
    /// included) with a pattern, and the symbols its instances and inline graphics there use; looked
    /// at once a symbol.
    fn symbol_own_patterns(&mut self, name: &str) -> (bool, Vec<String>) {
        if let Some(known) = self.pattern_symbols.get(name) {
            return known.clone();
        }
        let doc = self.symbol_doc;
        let mut own = false;
        let mut used = vec![];
        if let Some(sym) = doc.symbols.iter().find(|s| s.name == name) {
            let pattern = |p: &Paint| matches!(p, Paint::Pattern { .. });
            vectorcraft_doc::swatches::walk_with_masks(&sym.art, &mut |m| {
                own |= vectorcraft_doc::pattern::uses_pattern(m, None);
                match &m.kind {
                    NodeKind::Text(t) => {
                        own |= t.runs.iter().any(|r| pattern(&r.style.fill) || pattern(&r.style.stroke));
                        // Inline graphics are written as their symbols' art.
                        used.extend(t.runs.iter().filter_map(|r| r.inline.as_ref()).map(|art| art.symbol.clone()));
                    }
                    NodeKind::SymbolInstance { symbol, .. } => used.push(symbol.clone()),
                    _ => {}
                }
            });
        }
        self.pattern_symbols.insert(name.to_string(), (own, used.clone()));
        (own, used)
    }

    /// An instance of `symbol` placed by `xf`, of transparency `xy`: `/SymbolInstance :`, `(name)
    /// /SymbolRef ,`, the matrix taking the symbol's art to art space (in a symbol's art, to the
    /// canvas's space turned y up about art space's origin there) as `a b c d tx ty
    /// /RTransformMatrix` (to ten decimals), `;`, then its state block as a group's: `Xy`, `0 0 Xd`
    /// and `6 () XW` when it has transparency, else `9 () XW`. Its dictionary goes between the two.
    pub(super) fn symbol_instance(&mut self, symbol: &str, xf: Affine, xy: Xy) {
        let m = self.instance_space * xf;
        let _ = write!(self.body, "/SymbolInstance :\n{} /SymbolRef ,\n", ps::bytes(symbol.as_bytes()));
        for v in m.as_coeffs() {
            self.body.push_str(&super::images::precise(v));
            self.body.push(' ');
        }
        self.body.push_str("/RTransformMatrix\n;\n");
        // An instance with isolation, knockout or knockout shape of its own is written as its art
        // ([`Self::live_instance`]); these are the values of the others.
        let xy = Xy { isolate: false, knockout: Knockout::Neutral, shape: false, ..xy };
        self.block_start = self.body.len();
        if xy == Xy::PLAIN {
            self.line("9 () XW");
        } else {
            self.write_xy(xy);
            self.line("0 0 Xd\n6 () XW");
        }
        self.block_end = self.body.len();
    }

    /// Write symbol `name`'s definition, once: `%AI14_BeginSymbol`, `(name)`, `0 A`, `0 Xw`, `0 Ae`,
    /// the art in the symbol's space (y down: its numbers are the symbol's own) as a group (`u` …
    /// `U`; art that isn't a group, or is a plain group of one object, in a group of its own), the
    /// symbol's registration point (`AI13PatternRegistrationType`, 1 to 9 from the top left, row by
    /// row) in that group's dictionary, `9 () XW`, `%AI10_EndSymbol`. Linked images in the
    /// art are embedded. The symbols its art's instances use are defined first, and the art is
    /// written from the top of the nesting whatever the instance that asks for it, so a definition
    /// is the same wherever its first instance lies. False when it can't be: the document doesn't
    /// have the symbol, it is being written (its art holds an instance of itself), it is reached
    /// through more than [`MAX_NEST`] definitions being written, the definitions written and
    /// being written leave no place under [`MAX_SYMBOLS`], or its art doesn't all fit within the
    /// object limit ([`Self::take_objects`]).
    pub(super) fn define_symbol(&mut self, name: &str) -> bool {
        if self.defined_symbols.contains(name) {
            return true;
        }
        let doc = self.symbol_doc;
        let Some(sym) = doc.symbols.iter().find(|s| s.name == name) else { return false };
        if self.defining_symbols.iter().any(|d| d == name) || self.defining_symbols.len() >= MAX_NEST as usize {
            return false;
        }
        // The reader keeps no more definitions than this, and each definition being written takes
        // one when it ends.
        if self.symbol_defs.len() + self.defining_symbols.len() >= MAX_SYMBOLS {
            self.warn(SYMBOL_COUNT);
            return false;
        }
        if sym.dynamic || doc.symbol_export_type(name) != SymbolExportType::default() || doc.symbol_nine_slice(name).is_some() {
            self.warn(SYMBOL_OPTIONS);
        }
        self.defining_symbols.push(name.to_string());
        // The objects of this definition and of those it defines first.
        let before = self.charges();
        let mut used = vec![];
        vectorcraft_doc::swatches::walk_with_masks(&sym.art, &mut |m| {
            if let NodeKind::SymbolInstance { symbol, .. } = &m.kind {
                used.push(symbol.as_str());
            }
        });
        for u in used {
            self.define_symbol(u);
        }
        if self.symbol_defs.len() + self.defining_symbols.len() > MAX_SYMBOLS {
            self.defining_symbols.pop();
            self.keep_since(before);
            self.warn(SYMBOL_COUNT);
            return false;
        }
        // The group that holds the art is an object of the definition ([`Self::take_objects`]);
        // without room for it, or for all the art, there is no definition.
        let (art_start, bound) = (self.charges(), self.bound.len());
        if !self.take_objects(1) {
            self.defining_symbols.pop();
            self.keep_since(before);
            return false;
        }
        // The art is written in the symbol's space, from the state `0 A` and `0 Xw` set, from the
        // document at its own size.
        let origin = self.canvas_origin;
        let about_origin = Affine::new([1.0, 0.0, 0.0, -1.0, -origin.0, origin.1]);
        let page_doc = std::mem::replace(&mut self.doc, doc);
        let saved = (
            std::mem::take(&mut self.body),
            (self.to_art, self.grad_space, self.canvas, self.instance_space),
            self.state.replace((false, false)),
            self.gs.replace(Xy::OPAQUE),
            self.clip.take(),
            (self.block_start, self.block_end),
            (std::mem::replace(&mut self.in_symbol, true), std::mem::take(&mut self.hidden)),
            self.root_entries.take(),
            std::mem::take(&mut self.wrote_pattern),
            (std::mem::take(&mut self.wrote_fixed), std::mem::replace(&mut self.inner_scale, 1.0)),
        );
        // Gradients' matrices and instances' matrices are written about the canvas point of art
        // space's origin, y up; `Bg` places a gradient on the art as it lies in the symbol's space.
        (self.to_art, self.grad_space, self.canvas, self.instance_space) = (Affine::IDENTITY, about_origin, Affine::IDENTITY, about_origin);
        self.block_end = usize::MAX;
        let registration = u16::from(doc.symbol_registration(name).0.min(8)) + 1;
        let entries = format!("%_{registration} /Int (AI13PatternRegistrationType) ,\n");
        // The canvas draws a symbol's art even when its root is hidden, so the root is written shown.
        let shown;
        let root: &Node = if sym.art.visible {
            &sym.art
        } else {
            let mut a = (*sym.art).clone();
            a.visible = true;
            shown = a;
            &shown
        };
        // A definition holds a group: art of another kind, and a plain group of one object (which
        // the reader takes for that group), are written in one.
        let wrap = !matches!(root.kind, NodeKind::Group { clip: false, .. }) || crate::import::symbol_art_member(root).is_some();
        let nest = std::mem::replace(&mut self.nest, 1);
        if wrap {
            self.line("u");
            self.node(root);
            let _ = write!(self.body, "U\n%_/ArtDictionary :\n{entries}%_;\n%_\n9 () XW\n");
        } else {
            // A group always writes its dictionary ([`Self::dictionary`]), which takes the entries.
            // It counts itself, in the room kept for it above.
            self.root_entries = Some((root.id, entries));
            self.objects -= 1;
            self.node(root);
        }
        self.nest = nest;
        // Objects the writing doesn't pass through one by one (clipping paths, a clipping group's
        // own path) count as well.
        vectorcraft_doc::swatches::walk_with_masks(&sym.art, &mut |m| self.wrote_fixed |= redrawn(m));
        if self.wrote_pattern {
            self.pattern_written.insert(name.to_string());
        }
        if self.wrote_fixed {
            self.fixed_symbols.insert(name.to_string());
        }
        if self.inner_scale > 1.0 {
            self.symbol_scales.insert(name.to_string(), self.inner_scale);
        }
        let art;
        (
            art,
            (self.to_art, self.grad_space, self.canvas, self.instance_space),
            self.state,
            self.gs,
            self.clip,
            (self.block_start, self.block_end),
            (self.in_symbol, self.hidden),
            self.root_entries,
            self.wrote_pattern,
            (self.wrote_fixed, self.inner_scale),
        ) = saved;
        self.doc = page_doc;
        let art = std::mem::replace(&mut self.body, art);
        self.defining_symbols.pop();
        if self.left_out_since(art_start) {
            self.give_back(art_start, &art);
            self.bound.truncate(bound);
            self.keep_since(before);
            return false;
        }
        self.keep_since(before);
        let text = format!("%AI14_BeginSymbol\n{}\n0 A\n0 Xw\n0 Ae\n{art}%AI10_EndSymbol\n", ps::bytes(name.as_bytes()));
        self.defined_symbols.insert(name.to_string());
        self.symbol_defs.push(text);
        true
    }

    /// Every symbol of the document defined, in its order (those its art uses before it): the
    /// Symbols panel keeps the ones no instance uses too, and on a large canvas all of them.
    pub(in crate::ai_editing) fn symbols(&mut self) {
        let doc = self.symbol_doc;
        for s in &doc.symbols {
            self.define_symbol(&s.name);
        }
    }

    /// The symbol definitions, in the setup after the patterns: `%AI5_Begin_NonPrinting`, `Np`, each
    /// definition, `%AI5_End_NonPrinting--`.
    pub(in crate::ai_editing) fn symbol_definitions(&self, s: &mut String) {
        if self.symbol_defs.is_empty() {
            return;
        }
        s.push_str("%AI5_Begin_NonPrinting\nNp\n");
        for d in &self.symbol_defs {
            s.push_str(d);
        }
        s.push_str("%AI5_End_NonPrinting--\n");
    }

    /// The symbol list, after the palette: the names of the symbols defined in the document's order
    /// (the Symbols panel's), between `%AI24_BeginSymbolList` and `%AI24_EndSymbolList` in a
    /// non-printing block.
    pub(in crate::ai_editing) fn symbol_list(&self, s: &mut String) {
        if self.symbol_defs.is_empty() {
            return;
        }
        s.push_str("%AI5_Begin_NonPrinting\nNp\n%AI24_BeginSymbolList\n");
        for sym in &self.symbol_doc.symbols {
            if self.defined_symbols.contains(&sym.name) {
                let _ = writeln!(s, "{}", ps::bytes(sym.name.as_bytes()));
            }
        }
        s.push_str("%AI24_EndSymbolList\n%AI5_End_NonPrinting--\n");
    }
}
