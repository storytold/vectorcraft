use super::containers::close;
use super::document::slot_of;
use super::types::{BLENDS, FLIP, Frame, IGNORED, Kind, MAX_DEPTH};
use super::{
    Affine, Appearance, AppearanceItem, Arc, BlendMode, Dash, FillLayer, FillRule, Gradient, GradientDef, HashMap, HashSet, Instance, Knockout,
    LayerColor, LineCap, LineJoin, Node, NodeId, NodeKind, Paint, Reader, Rect, StrokeLayer, V, obj, paint,
};

impl<'a> Reader<'a> {
    // ---- operators -------------------------------------------------------------------------

    fn end_array(&mut self) -> Result<(), String> {
        // The innermost `[` still on the stack (the stack is cut back by operators).
        let mut at = self.stack.len();
        while let Some(m) = self.marks.pop() {
            if matches!(self.stack.get(m), Some(V::Mark)) {
                at = m;
                break;
            }
        }
        let items: Vec<V> = self.stack.split_off(at).into_iter().skip(1).collect();
        self.push(V::Arr(items))
    }

    fn add_object_entry(&mut self) {
        if let Some(Frame { kind: Kind::Obj(o), base, .. }) = self.frames.last_mut() {
            obj::add_entry(o, &mut self.stack, *base);
            // Binary data in ASCII85 follows, up to its `~>`: a `/Binary` dictionary's
            // (`/ASCII85Decode ,`) or a non-native art object's (`/ForeignObject`'s
            // `/Data ,`, the PDF it keeps), in comment lines. ASCII85 has `_`, so a line
            // of it may start with `%_` and read as hidden tokens: a `(` there would open
            // a string that swallows the rest of the layer.
            let key = o.entries.last().and_then(|(k, _)| k.as_deref());
            if o.ty == "Binary" && key == Some("ASCII85Decode") {
                self.lex.skip_past(b"~>");
            } else if o.ty == "ForeignObject" && key == Some("Data") {
                self.foreign_data = Some(self.lex.skip_past(b"~>").to_vec());
            }
        }
    }

    fn open_mask_object(&mut self, hidden: bool) -> Result<(), String> {
        if !matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(o), .. }) if o.ty == "Mask") {
            // `X=` also introduces legacy text frame art, read by `text_frames`.
            return Ok(());
        }
        self.end_path_quietly();
        self.open(Kind::Mask, hidden)
    }

    pub(super) fn op(&mut self, w: &str, hidden: bool) -> Result<(), String> {
        if self.blend_record.is_some() {
            self.end_blend();
        }
        match w {
            // The writer's placeholder for a number the object doesn't have (a radial gradient's
            // highlight `-- -- angle length Bh`): a NaN keeps the other operands where they are.
            "--" => return self.push(V::Num(f64::NAN)),
            "[" => {
                self.marks.push(self.stack.len());
                return self.push(V::Mark);
            }
            "]" => return self.end_array(),
            ":" => return self.begin_obj(hidden),
            "," => {
                self.add_object_entry();
                return Ok(());
            }
            ";" => return self.end_obj(),
            "X=" => return self.open_mask_object(hidden),
            "X+" if self.frames.iter().any(|f| matches!(f.kind, Kind::Mask)) => return self.end_mask(),
            "X+" => return Ok(()),
            _ => {}
        }
        // What follows an object (its name, transparency and style) ends with anything else. The
        // state block of a layer whose art ended (`Xy`, `Xd`, `XW`, then `LB`) is read at the `LB`.
        if !matches!(w, "Xy" | "Xd" | "XW" | "BB" | "LB") {
            self.after_object = false;
            self.layer_state = None;
        }
        let group_ended = std::mem::take(&mut self.group_ended);
        let vals = self.operands();
        let vals = self.take_head(vals);
        // Only `Bh` reads an undefined number. Anywhere else it would reach a point or a size, so
        // the operator is one the reader doesn't know, as before.
        if w != "Bh" && !self.frames.is_empty() && vals.iter().any(|v| matches!(v, V::Num(n) if n.is_nan())) {
            self.note_unknown("--");
            return Ok(());
        }
        if let Some(d) = self.gradient_def.as_mut() {
            match w {
                "Bs" => d.stop(&vals, &mut self.stop_spots),
                "BD" => {
                    if let Some((name, g)) = self.gradient_def.take().and_then(GradientDef::finish) {
                        self.gradients.insert(name, g);
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        if w == "X!" || self.mesh.is_some() {
            let name = vals.iter().rev().find_map(|v| if let V::Name(n) = v { Some(n.as_str()) } else { None });
            match (w, name) {
                ("X!", Some("Mesh")) => {
                    self.end_path_quietly();
                    self.mesh = Some((crate::import::ai::mesh::MeshReader::default(), hidden));
                    return Ok(());
                }
                ("X!", Some("End")) if self.mesh.is_some() => return self.end_mesh(),
                ("X#", _) => {
                    let to_doc = self.space();
                    if let Some((m, _)) = self.mesh.as_mut() {
                        m.entry(&vals, to_doc);
                    }
                    return Ok(());
                }
                _ if self.mesh.is_some() => return Ok(()),
                _ => {}
            }
        }
        match w {
            // Containers.
            "u" => {
                self.end_path_quietly();
                self.open(Kind::Group, hidden)?
            }
            "*u" => {
                self.end_path_quietly();
                self.open(Kind::Compound, hidden)?
            }
            "q" => {
                self.end_path_quietly();
                self.open(Kind::Clip, hidden)?
            }
            "U" | "*U" | "Q" => self.end_container(w, hidden)?,
            "Lb" => self.layer_attrs(&vals),
            "Ln" => {
                if let Some(Frame { kind: Kind::Layer(a), .. }) = self.frames.last_mut() {
                    a.name = vals.iter().rev().find_map(V::text).unwrap_or_default();
                }
            }
            "LB" => self.end_layer()?,
            // Paths.
            "m" | "l" | "L" | "c" | "C" | "v" | "V" | "y" | "Y" => self.path_op(w, &vals)?,
            "h" => close(&mut self.path),
            // Ends a path that stays open.
            "H" => {}
            "W" => self.clip_next = true,
            // A text object's frame or path.
            "N" | "n" | "F" | "f" | "S" | "s" | "B" | "b" if self.in_text_path() => self.text_path(),
            "N" | "n" | "F" | "f" | "S" | "s" | "B" | "b" => self.paint(w, hidden, false)?,
            "*" => {
                let op = vals.iter().rev().find_map(V::text).unwrap_or_else(|| "N".into());
                self.paint(&op, hidden, true)?;
            }
            // Colours.
            "g" | "G" | "k" | "K" | "x" | "X" | "Xa" | "XA" | "Xx" | "XX" | "Xk" | "XK" | "Xs" | "XS" | "Xz" | "XZ" => self.color(w, &vals),
            "p" | "P" => self.pattern_paint(w, &vals),
            // A pattern's definition ends.
            "E" if self.pattern.is_some() => self.done = true,
            "O" => self.gs.overprint_fill = Self::nums(&vals).last() == Some(&1.0),
            "R" => self.gs.overprint_stroke = Self::nums(&vals).last() == Some(&1.0),
            "XR" => self.gs.rule = if Self::nums(&vals).last() == Some(&1.0) { FillRule::EvenOdd } else { FillRule::NonZero },
            "w" => {
                if let Some(v) = Self::nums(&vals).last().filter(|v| v.is_finite() && **v >= 0.0) {
                    self.gs.width = *v;
                }
            }
            "J" => {
                self.gs.cap = match Self::nums(&vals).last().map(|v| *v as i64) {
                    Some(1) => LineCap::Round,
                    Some(2) => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "j" => {
                self.gs.join = match Self::nums(&vals).last().map(|v| *v as i64) {
                    Some(1) => LineJoin::Round,
                    Some(2) => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "M" => {
                if let Some(v) = Self::nums(&vals).last().filter(|v| v.is_finite() && **v >= 1.0) {
                    self.gs.miter = v.min(500.0);
                }
            }
            "d" => self.dash(&vals),
            "D" => {
                // The published Adobe Illustrator File Format Specification (version 3.0), §6.8,
                // permits `u` groups of compound members.
                // Their explicit winding declarations describe subpaths, not separate paints;
                // modern streams can also end these groups with `9 () XW`.
                let n = self.frames.len();
                if n >= 2
                    && self.frames.get(n - 2).is_some_and(|f| matches!(f.kind, Kind::Compound))
                    && let Some(f) = self.frames.last_mut().filter(|f| matches!(f.kind, Kind::Group))
                {
                    f.compound_members = true;
                }
            }
            // Transparency and state.
            "Xy" => self.transparency(&vals),
            // A group's own state (`Xd`, `6 () XW`) without an `Xy` keeps the current transparency, as
            // the file's PDF part draws it. A look's group (`9 () XW`) has no state of its own.
            "Xd" if group_ended => {
                let gs = self.gs.clone();
                if let Some(n) = self.last_object().filter(|n| matches!(n.kind, NodeKind::Group { .. })) {
                    (n.opacity, n.blend, n.isolate, n.knockout) = (gs.opacity, gs.blend, gs.isolate, gs.knockout);
                }
            }
            "Xw" => self.gs.hidden = Self::nums(&vals).last() == Some(&1.0),
            "A" => self.gs.locked = Self::nums(&vals).last() == Some(&1.0),
            "XW" => self.style_marker(&vals, hidden),
            // Gradients.
            "Bd" => self.gradient_def = GradientDef::begin(&vals),
            "Bs" | "BD" => {}
            "Bb" => self.instance = Some(Instance::begin(&vals)),
            "Bg" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bg(&vals);
                }
            }
            "Bm" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bm(&vals);
                }
            }
            "Xm" => {
                if let Some(i) = self.instance.as_mut() {
                    i.xm(&vals);
                }
            }
            "Bh" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bh(&vals);
                }
            }
            "BB" => {
                // An instance that closes with no path to paint is an object's own note of its
                // gradient, written after it: it paints nothing (not the next path either).
                if let Some(i) = self.instance.take()
                    && !self.path.elements().is_empty()
                {
                    self.set_instance(&i);
                }
                // `1 BB` strokes the path the gradient just filled, `2 BB` closes it first (§3.5.3).
                if matches!(Self::nums(&vals).last(), Some(f) if *f == 1.0 || *f == 2.0) {
                    self.stroke_last();
                }
            }
            // Images.
            "XN" => {
                if let Some(r) = self.raster.as_mut() {
                    r.space = vals.iter().rev().find_map(V::text);
                }
            }
            // Placed files.
            "`" if self.place.is_some() => {
                let matrix = vals.iter().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None });
                let path = vals.iter().rev().find_map(V::text).unwrap_or_default();
                if let Some(p) = self.place.as_mut() {
                    p.matrix = matrix.and_then(|m| <[f64; 6]>::try_from(m).ok()).map(Affine::new);
                    p.bounds =
                        Self::nums(&vals).get(..4).and_then(|b| <[f64; 4]>::try_from(b).ok()).map(|[x0, y0, x1, y1]| Rect::new(x0, y0, x1, y1));
                    p.path = path;
                }
            }
            "~" if self.place.is_some() => self.end_place(hidden)?,
            // Type in the legacy format.
            "To" => self.begin_text(&vals, hidden),
            "Tp" => self.begin_text_path(&vals),
            "TP" => self.end_text_path(),
            "TO" => self.end_text(hidden)?,
            "Tx" | "Tj" | "TX" => self.text_body(w, &vals),
            "Tr" | "Tf" | "Ts" | "Tz" | "Tt" | "TA" | "Tl" | "Ta" | "Ti" => self.text_attr(w, &vals),
            // Attributes the reader doesn't keep (kerning pairs, spacing, hanging punctuation,
            // Far-Eastern type) and final-form positions (§15.3).
            "Tk" | "TK" | "TC" | "TW" | "Tq" | "Tc" | "Tw" | "Tm" | "Td" | "T*" | "TR" | "T+" | "T-" | "TV" | "Tv" | "Ty" | "TY" | "TG" | "Tg"
            | "Tu" | "Xu" => {}
            // Hyphenation and tab-ruler attributes the reader doesn't keep either (§22): `Th`
            // hyphen control, `XL` hyphenation language, `Xb`/`XB` a tab definition (and the `Xe`
            // tab descriptions between them).
            "Th" | "XL" | "Xb" | "XB" | "Xe" => {}
            "TE" => self.te(&vals),
            // An image whose samples follow in comment lines (read at `%AI5_EndRaster`).
            "XI" if self.raster.is_some() => {
                if let Some(r) = self.raster.as_mut() {
                    r.operands = Some(vals);
                }
            }
            // A plugin group ends; a blend's record follows on `%` lines.
            "XP" => {
                let name = vals.iter().find_map(V::text).unwrap_or_default();
                let len = Self::nums(&vals).last().copied().unwrap_or(0.0);
                if name == crate::import::ai::blend::PLUGIN && len.is_finite() && len > 0.0 && len <= crate::import::ai::blend::MAX_RECORD as f64 {
                    self.blend_record = Some((len as usize, vec![]));
                }
            }
            _ if IGNORED.contains(&w) => {}
            // Before the art, the setup runs procedures of the printing format.
            _ if self.frames.is_empty() => {}
            _ => self.note_unknown(w),
        }
        Ok(())
    }

    /// `… Lb`: a layer's options. The legacy format's has ten operands, its colour index one place
    /// earlier (`visible preview unlocked printing dimmed masks colorIndex r g b`).
    fn layer_attrs(&mut self, vals: &[V]) {
        let n = Self::nums(vals);
        let Some(Frame { kind: Kind::Layer(a), .. }) = self.frames.last_mut() else { return };
        let flag = |i: usize, default: bool| n.get(i).map_or(default, |v| *v != 0.0);
        a.visible = flag(0, true);
        a.preview = flag(1, true);
        a.locked = !flag(2, true);
        a.printable = flag(3, true);
        let dim_pct = n.get(12).copied().filter(|v| v.is_finite()).unwrap_or(50.0).clamp(0.0, 100.0);
        a.dim = flag(4, false).then_some(dim_pct.round() as u8);
        let at = if n.len() <= 10 { 6 } else { 7 };
        let index = n.get(at).copied().unwrap_or(0.0);
        let rgb = |i: usize| n.get(at + i).map_or(0, |v| v.clamp(0.0, 255.0) as u8);
        a.color = Some(if (0.0..vectorcraft_doc::LAYER_COLORS.len() as f64).contains(&index) && index.fract() == 0.0 {
            LayerColor::Preset(index as u8)
        } else {
            LayerColor::Custom([rgb(1), rgb(2), rgb(3)])
        });
    }

    /// `(name) x y scaleX scaleY angle reflect axis shear shearAxis [matrix] p` (`P`: the stroke's):
    /// the pattern swatch `name`, its space taken to art space by the matrix. A pattern this hasn't
    /// read can't be drawn.
    fn pattern_paint(&mut self, w: &str, vals: &[V]) {
        // In the setup it paints nothing (and the art's space isn't fixed yet).
        if self.frames.is_empty() {
            return;
        }
        let name = vals.iter().find_map(|v| if let V::Str(_) = v { v.text() } else { None });
        let Some(tile) = name.as_deref().and_then(|n| self.pattern_tiles.get(n)).copied() else {
            self.unreadable("pattern fills");
            return;
        };
        let matrix = vals.iter().rev().find_map(|v| if let V::Arr(a) = v { paint::affine(&Self::nums(a)) } else { None });
        let Some(xf) = pattern_xf(self.space(), matrix, tile) else {
            self.unreadable("pattern fills");
            return;
        };
        let p = Paint::Pattern { pattern: name.unwrap_or_default(), xf };
        if w == "p" {
            self.gs.fill = p;
        } else {
            self.gs.stroke = p;
        }
    }

    fn color(&mut self, w: &str, vals: &[V]) {
        let Some((p, named)) = paint::color_op(w, vals) else { return };
        // Registration (`Xs`, `Xz`) says nothing of the document's colour model.
        let registration = matches!(w, "Xs" | "XS" | "Xz" | "XZ");
        match p.color().filter(|_| !registration) {
            Some(vectorcraft_color::Color::Rgb { .. }) => self.rgb_colors += 1,
            Some(vectorcraft_color::Color::Cmyk { .. }) => self.cmyk_colors += 1,
            _ => {}
        }
        if let Some(n) = named {
            self.spots.push(n);
        }
        // A colour set after a gradient instance that painted nothing (`Bb` … `0 BB` with no
        // path) is the next object's paint, not the gradient.
        if w.ends_with(|c: char| c.is_ascii_lowercase()) {
            self.gs.fill = p;
            self.pending_fill = None;
        } else {
            self.gs.stroke = p;
            self.pending_stroke = None;
        }
    }

    /// `[dashes] phase d`.
    fn dash(&mut self, vals: &[V]) {
        let pattern: Vec<f64> = vals.iter().rev().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None }).unwrap_or_default();
        let offset = Self::nums(vals).last().copied().unwrap_or(0.0);
        self.gs.dash = (!pattern.is_empty() && pattern.len() <= 64 && pattern.iter().all(|v| v.is_finite() && *v >= 0.0)).then(|| Dash {
            pattern,
            offset: if offset.is_finite() { offset } else { 0.0 },
            align_corners: false,
        });
    }

    /// `mode opacity isolate knockout shape Xy`; after a group's end, the group's too.
    fn transparency(&mut self, vals: &[V]) {
        let n = Self::nums(vals);
        let [mode, opacity, rest @ ..] = n.as_slice() else { return };
        let gs = &mut self.gs;
        gs.blend = BLENDS.get(*mode as usize).copied().filter(|_| *mode >= 0.0).unwrap_or(BlendMode::Normal);
        gs.opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) as f32 } else { 1.0 };
        gs.isolate = rest.first().is_some_and(|v| *v != 0.0);
        gs.knockout = match rest.get(1).map(|v| *v as i64) {
            Some(1) => Knockout::On,
            Some(2) => Knockout::Neutral,
            _ => Knockout::Off,
        };
        gs.knockout_shape = rest.get(2).is_some_and(|v| *v != 0.0);
        let gs = self.gs.clone();
        if self.last_object().is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Compound { .. })) {
            if let Some(n) = self.last_object() {
                n.opacity = gs.opacity;
                n.blend = gs.blend;
                n.isolate = gs.isolate;
                n.knockout = gs.knockout;
            }
            self.layer_state = None;
        } else {
            // After a layer's art and before its `LB`: the layer's own state (#2003). Kept until
            // an operator that isn't part of the state block (or the `LB` that takes it) says the
            // art moves on, so a path's state before its paint isn't read as its layer's.
            self.layer_state =
                matches!(self.frames.last(), Some(Frame { kind: Kind::Layer(_), .. })).then_some((gs.opacity, gs.blend, gs.isolate, gs.knockout));
        }
    }

    /// `n (style) XW` after an object: `1 (style)` after the object written on `%_` lines, its
    /// drawn look before it (all on `%_` lines inside another such object). Other markers end what
    /// follows an object.
    fn style_marker(&mut self, vals: &[V], hidden: bool) {
        let code = Self::nums(vals).last().copied().unwrap_or(0.0);
        let style = vals.iter().rev().find_map(V::text).unwrap_or_default();
        let after = std::mem::take(&mut self.after_object);
        if code == 9.0
            && let Some(look) = self
                .frames
                .last()
                .and_then(|f| f.children.last())
                .filter(|n| matches!(n.kind, NodeKind::Group { .. }) && !self.compound_groups.contains(&n.id))
        {
            self.looks.insert(look.id);
        }
        if code != 1.0 || style.is_empty() || !after {
            return;
        }
        let space = self.space();
        let Some(f) = self.frames.last_mut() else { return };
        let n = f.children.len();
        let (Some(object), Some(look)) = (n.checked_sub(1).and_then(|i| f.children.get(i)), n.checked_sub(2).and_then(|i| f.children.get(i))) else {
            return;
        };
        // The look is a group of paths, or one image where an effect (a blur, a shadow) rasterizes it.
        if !self.hidden_ids.contains(&object.id)
            || self.hidden_ids.contains(&look.id) != hidden
            || self.text_looks.contains(&look.id)
            || !matches!(look.kind, NodeKind::Group { clip: false, .. } | NodeKind::Image(_))
        {
            return;
        }
        // A style read here: the object, painted as it says.
        if let Some(appearance) = self
            .styles
            .get(&style)
            .and_then(|st| styled(object, look, st, &StylePaints { gradients: &self.gradients, patterns: &self.pattern_tiles, space }))
        {
            let Some(mut object) = f.children.pop() else { return };
            let Some(look) = f.children.pop() else { return };
            if let Some(b) = look.visual_bounds().filter(|_| !hidden && object.visible && look.visible) {
                self.restyled.push(b);
            }
            object.appearance = appearance.into();
            // The object's transparency is written on its look, after the look's group ends.
            (object.opacity, object.blend, object.isolate, object.knockout) = (look.opacity, look.blend, look.isolate, look.knockout);
            // Now the object itself, not a copy an enclosing look's `strip` would remove.
            self.hidden_ids.remove(&object.id);
            f.children.push(object);
            return;
        }
        let is_text = slot_of(object.name.as_deref()).is_some();
        let Some(mut object) = f.children.pop() else { return };
        let Some(look) = f.children.last_mut() else { return };
        // Inside a copy on `%_` lines everything is, so nothing tells the look's own copies apart.
        if !hidden {
            strip(look, &self.hidden_ids);
        }
        if is_text {
            // A text object stays the type it is: over what else its look draws, or in place of a
            // look that was only its type. Its transparency is written on its look.
            let empty = look.children().is_some_and(|c| c.is_empty());
            if empty || (look.opacity >= 1.0 && look.blend == BlendMode::Normal && !look.isolate) {
                (object.opacity, object.blend, object.isolate, object.knockout) = (look.opacity, look.blend, look.isolate, look.knockout);
                if empty {
                    *look = object;
                } else {
                    f.children.push(object);
                }
                return;
            }
            // The look and the type go into a group that has the transparency.
            let Some(mut look) = f.children.pop() else { return };
            let state = (look.opacity, look.blend, look.isolate, look.knockout);
            (look.opacity, look.blend, look.isolate, look.knockout) = (1.0, BlendMode::Normal, false, Knockout::Neutral);
            (object.opacity, object.blend, object.isolate) = (1.0, BlendMode::Normal, false);
            let mut group = Node::group(self.doc.alloc_id(), vec![Arc::new(look), Arc::new(object)]);
            (group.opacity, group.blend, group.isolate, group.knockout) = state;
            self.text_looks.insert(group.id);
            if let Some(f) = self.frames.last_mut() {
                f.children.push(group);
            }
            return;
        }
        look.name = object.name.clone().or(look.name.take());
        look.visible = object.visible;
        look.locked = object.locked;
        if !hidden {
            self.drawn_looks += 1;
        }
    }
}

/// Whether `look`'s paths don't all paint the same.
pub(super) fn paints_apart(look: &Node) -> bool {
    fn leaves<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
        match &n.kind {
            NodeKind::Group { children, .. } => children.iter().for_each(|c| leaves(c, out)),
            _ => out.push(n),
        }
    }
    let mut paths = vec![];
    leaves(look, &mut paths);
    paths.windows(2).any(|w| matches!(w, [a, b] if a.appearance != b.appearance))
}

pub(super) fn path_rule(n: &Node) -> FillRule {
    match &n.kind {
        NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
        _ => FillRule::NonZero,
    }
}

/// The tile's art (y down from the tile's top left) → the pattern's space (`matrix`) → art space
/// → the document (`space`). The tile's lower left is the pattern's origin wherever the
/// definition drew it, as the file's PDF part tiles it. `None` when it can't be inverted.
fn pattern_xf(space: Affine, matrix: Option<Affine>, tile: Rect) -> Option<Affine> {
    let xf = space * matrix.unwrap_or(Affine::IDENTITY) * FLIP * Affine::translate((0.0, -tile.height()));
    (xf.determinant().abs() >= 1e-12 && xf.as_coeffs().iter().all(|v| v.is_finite())).then_some(xf)
}

/// What a style's fills paint with: the file's gradients and pattern tiles, and art space → the document.
struct StylePaints<'a> {
    gradients: &'a HashMap<String, Gradient>,
    patterns: &'a HashMap<String, Rect>,
    space: Affine,
}

/// The gradients and patterns `look` paints with, placed as it draws them, by their swatch's name
/// (the first of each; a pattern's name after `PATTERN_KEY`).
fn placed_paints(look: &Node, out: &mut HashMap<String, Paint>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for item in &look.appearance.items {
        let key = match item.paint() {
            Paint::Gradient(g) => g.swatch.clone(),
            Paint::Pattern { pattern, .. } => Some(format!("{PATTERN_KEY}{pattern}")),
            _ => None,
        };
        if let Some(key) = key {
            out.entry(key).or_insert_with(|| item.paint().clone());
        }
    }
    for c in look.children().into_iter().flatten() {
        placed_paints(c, out, depth + 1);
    }
}

/// What [`placed_paints`] keys a pattern's name with, apart from the gradients' names.
const PATTERN_KEY: &str = "\u{1}pattern ";

/// `object`'s appearance with style `st`, its drawn look `look`: `None` when the style isn't one
/// this reads, or asks a group for fills or strokes.
fn styled(object: &Node, look: &Node, st: &crate::import::ai::style::ArtStyle, paints: &StylePaints) -> Option<Appearance> {
    use crate::import::ai::style::StyleItem;
    // A text object's slot (an empty group standing for it) takes the style as the type will.
    let slot = slot_of(object.name.as_deref()).is_some();
    if !st.plain
        || (!slot && !st.simple)
        || (object.is_container() && !slot && !matches!(object.kind, NodeKind::Compound { .. }) && !st.items.is_empty())
    {
        return None;
    }
    // A style with one gradient fill: the object's own gradient, placed as the object's fill is.
    let gradients = st.items.iter().filter(|i| matches!(i, StyleItem::Gradient { .. })).count();
    let own_gradient = object.appearance.fill().filter(|f| matches!(f.paint, Paint::Gradient(_)));
    // A style's gradients are written in a space of the object's bounds that isn't documented; its
    // drawn look has each where the app placed it.
    let mut placed = HashMap::new();
    placed_paints(look, &mut placed, 0);
    let mut items = vec![];
    for (n, item) in st.items.iter().enumerate() {
        let mut layer = match item {
            StyleItem::Fill(paint) => AppearanceItem::Fill(FillLayer::new(paint.clone())),
            StyleItem::Gradient { name, angle } => {
                let paint = match (name.as_ref().and_then(|n| placed.get(n)), gradients, own_gradient) {
                    // Other objects than type: the object's own, as they always were.
                    (_, 1, Some(own)) if !slot => own.paint.clone(),
                    (Some(p), ..) => p.clone(),
                    (None, 1, Some(own)) => own.paint.clone(),
                    _ => {
                        let gradient = name.as_ref().and_then(|n| paints.gradients.get(n))?;
                        let mut g = vectorcraft_color::gradient::GradientPaint::new(gradient.clone());
                        // Fitted to the object's bounds at its angle (art space turns counter-clockwise, as the document's angles do).
                        g.angle = *angle;
                        Paint::Gradient(Box::new(g))
                    }
                };
                AppearanceItem::Fill(FillLayer::new(paint))
            }
            StyleItem::Pattern { name, matrix } => match placed.get(&format!("{PATTERN_KEY}{name}")) {
                Some(p) => AppearanceItem::Fill(FillLayer::new(p.clone())),
                None => {
                    let tile = paints.patterns.get(name).copied()?;
                    let xf = pattern_xf(paints.space, matrix.and_then(|m| paint::affine(&m)), tile)?;
                    AppearanceItem::Fill(FillLayer::new(Paint::Pattern { pattern: name.clone(), xf }))
                }
            },
            StyleItem::Stroke { paint, width, cap, join, miter, dash, profile } => {
                let mut layer = StrokeLayer::new(paint.clone(), *width);
                (layer.cap, layer.join, layer.miter_limit, layer.dash, layer.profile) = (*cap, *join, *miter, dash.clone(), profile.clone());
                AppearanceItem::Stroke(layer)
            }
        };
        if let Some(look) = st.looks.get(n) {
            match &mut layer {
                AppearanceItem::Fill(f) => {
                    (f.visible, f.opacity, f.blend, f.effects) = (look.visible, look.opacity, look.blend, look.effects.clone())
                }
                AppearanceItem::Stroke(s) => {
                    (s.visible, s.opacity, s.blend, s.effects) = (look.visible, look.opacity, look.blend, look.effects.clone())
                }
            }
        }
        items.push(layer);
    }
    let contents_index = if object.is_container() || matches!(object.kind, NodeKind::Text(_) | NodeKind::Image(_)) {
        st.contents_index
    } else {
        object.appearance.contents_index
    };
    Some(Appearance { items, effects: st.effects.clone(), contents_index })
}

/// Remove from `n` the children read from `%_` lines (`hidden`: the object itself, not its look).
fn strip(n: &mut Node, hidden: &HashSet<NodeId>) {
    if let Some(children) = n.children_mut() {
        children.retain(|c| !hidden.contains(&c.id));
        for c in children.iter_mut() {
            strip(Arc::make_mut(c), hidden);
        }
    }
}
