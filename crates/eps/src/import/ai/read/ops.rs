use super::containers::close;
use super::document::slot_of;
use super::types::{
    BLENDS, FLIP, Frame, GRAPH_OPS, GRAPH_PLAIN, GRAPH_UNENDED, IGNORED, Kind, MAX_DEPTH, MAX_NODES, MAX_TAG_TEXT, MAX_TAGS, Place, URL_TAG,
    line_width,
};
use super::{
    Affine, Appearance, AppearanceItem, Arc, BlendMode, BrushesKept, Dash, FillLayer, FillRule, Global, Gradient, GradientDef, Graph, HashMap,
    HashSet, Instance, Knockout, LayerColor, LineCap, LineJoin, Node, NodeId, NodeKind, Paint, Reader, Rect, ShownNames, StrokeLayer, V, obj, paint,
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
            if ascii85_follows(&o.ty, key) {
                let data = self.lex.skip_past(b"~>");
                if o.ty == "ForeignObject" {
                    self.foreign_data = Some(data.to_vec());
                }
            }
        }
    }

    /// `X=` in a `/Mask` dictionary: the opacity mask's art starts, read until its `X+`
    /// ([`Self::end_mask`]).
    fn open_mask_object(&mut self, hidden: bool) -> Result<(), String> {
        if !matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(o), .. }) if o.ty == "Mask") {
            // `X=` also introduces legacy text frame art, read by `text_frames`.
            return Ok(());
        }
        self.end_path_quietly();
        // The art goes on from the paint before the dictionaries it is in, which `begin_obj` set
        // aside: the writer leaves out paint that doesn't change, so a mask rectangle with no color
        // of its own takes the white of the art before it, not a fresh state's black, which hides
        // what it masks. Lock, visibility and transparency start afresh, as a dictionary's art does.
        if let Some(f) = self.frames.iter().rev().take_while(|f| matches!(f.kind, Kind::Obj(_))).last() {
            let before = &f.gs;
            let gs = &mut self.gs;
            (gs.fill, gs.stroke, gs.dash) = (before.fill.clone(), before.stroke.clone(), before.dash.clone());
            (gs.width, gs.cap, gs.join, gs.miter, gs.rule) = (before.width, before.cap, before.join, before.miter, before.rule);
            (gs.overprint_fill, gs.overprint_stroke) = (before.overprint_fill, before.overprint_stroke);
            (self.pending_unread_fill, self.pending_unread_stroke) = (f.pending_unread_fill, f.pending_unread_stroke);
        }
        self.open(Kind::Mask, hidden)
    }

    /// `LB`: a graph started in this layer that hasn't ended doesn't go on past it; the layer ends.
    fn layer_end(&mut self) -> Result<(), String> {
        if self.graph.as_ref().is_some_and(|g| g.frames >= self.frames.len()) {
            self.graph = None;
            self.warn(GRAPH_UNENDED);
        }
        self.end_layer_in_mask()
    }

    /// A placed file's `` ` ``: its matrix, box and path; a second one, or operands of another
    /// shape, leave it unreadable.
    fn place_tick(&mut self, vals: &[V], hidden: bool) {
        let unseen = hidden || self.gs.hidden || !self.shown();
        let p = self.place.get_or_insert_with(|| Place { hidden: unseen, ..Place::default() });
        if p.ticked {
            p.broken = true;
        } else {
            p.ticked = true;
            match tick(vals) {
                Some((m, b, path)) => (p.matrix, p.bounds, p.path) = (Some(m), Some(b), path),
                None => p.broken = true,
            }
        }
    }

    /// `/identifier (string) XT` (version 7 specification §7): a tag on the object before it. The
    /// `AdobeURL` tag sets the object's URL, and every other tag is added to the object's data in
    /// order, up to [`MAX_TAGS`]. A tag with no object before it, with an empty identifier, or with
    /// an identifier or string longer than [`MAX_TAG_TEXT`] bytes is ignored, and so are tags in a
    /// pattern's or brush's art, which is not an object of the document. A compound clipping path's tags are its own,
    /// not those of the compound path that paints its fill apart from it.
    fn object_tag(&mut self, vals: &[V], hidden: bool) {
        let [.., V::Name(id), V::Str(value)] = vals else {
            self.note_unknown("XT", hidden);
            return;
        };
        if id.is_empty() || id.len() > MAX_TAG_TEXT || value.len() > MAX_TAG_TEXT || !self.after_object || self.pattern.is_some() {
            return;
        }
        let (id, value) = (id.clone(), obj::text(value));
        let Some(kids) = self.frames.last_mut().map(|f| &mut f.children) else { return };
        let Some(mut at) = kids.len().checked_sub(1) else { return };
        if at > 0 && kids.get(at).is_some_and(|n| self.clip_fills.contains(&n.id)) {
            at -= 1;
        }
        let Some(n) = kids.get_mut(at) else { return };
        n.edit_attrs(|a| {
            if id == URL_TAG {
                a.url = value;
            } else if a.data.len() < MAX_TAGS {
                a.data.push((id, value));
            }
        });
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
                // Drop marks that operators have taken off, so `marks` is the open nesting.
                while self.marks.last().is_some_and(|&m| !matches!(self.stack.get(m), Some(V::Mark))) {
                    self.marks.pop();
                }
                // Arrays nest as deep as their `[`s; dropping a deeper one would overflow the stack.
                if self.marks.len() >= MAX_DEPTH {
                    return Err("its objects are nested too deeply".into());
                }
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
        self.state_block_continues(w);
        let group_ended = std::mem::take(&mut self.group_ended);
        let mesh_ended = std::mem::take(&mut self.mesh_ended);
        let vals = self.operands();
        let vals = self.take_head(vals);
        if self.graph_record(w, hidden) {
            return Ok(());
        }
        // Only `Bh` reads an undefined number. Anywhere else it would reach a point or a size, so
        // the operator is one the reader doesn't know, as before.
        if w != "Bh" && !self.frames.is_empty() && vals.iter().any(|v| matches!(v, V::Num(n) if n.is_nan())) {
            self.note_unknown("--", hidden);
            return Ok(());
        }
        if let Some(d) = self.gradient_def.as_mut() {
            match w {
                "Bs" => d.stop(&vals, &mut self.stop_spots, &mut self.names),
                "BD" => {
                    if let Some((name, g)) = self.gradient_def.take().and_then(GradientDef::finish) {
                        self.global_names.define(Global::Gradient, &name);
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
            // A wraparound group takes no operands and isn't an element of a compound path.
            "*w" | "*W" if !vals.is_empty() => return Err(format!("`{w}` takes no operands")),
            "*w" if self.frames.iter().any(|f| matches!(f.kind, Kind::Compound)) => {
                return Err("a wraparound group is inside a compound path".into());
            }
            "*w" => {
                self.end_path_quietly();
                self.open(Kind::Wrap, hidden)?
            }
            "q" => {
                self.end_path_quietly();
                self.open(Kind::Clip, hidden)?
            }
            "U" | "*U" | "*W" | "Q" => self.end_container(w, hidden)?,
            "Lb" => self.layer_attrs(&vals),
            "Ln" => {
                if let Some(Frame { kind: Kind::Layer(a), .. }) = self.frames.last_mut() {
                    a.name = vals.iter().rev().find_map(|v| self.names.text(v)).unwrap_or_default();
                }
            }
            "LB" => self.layer_end()?,
            // Paths.
            "m" | "l" | "L" | "c" | "C" | "v" | "V" | "y" | "Y" => self.path_op(w, &vals)?,
            "h" => close(&mut self.path),
            // Ends a path that stays open.
            "H" => {}
            "W" => self.clip_next = true,
            "Mb" | "Mv" | "Md" | "ME" | "MB" | "Mi" => self.multi_mask_op(w)?,
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
            // Show Center (§12): 1 shows the center point of the paths that follow, and 0 leaves
            // them at their default, which hides it on an imported path.
            "Ap" => match Self::nums(&vals).last().copied() {
                Some(1.0) => self.gs.show_center = Some(true),
                Some(0.0) => self.gs.show_center = None,
                _ => {}
            },
            // Object tags (§7): `/identifier (string) XT` after the object it tags. The state written
            // after a group or a mesh is still theirs after its tags.
            "XT" => {
                self.object_tag(&vals, hidden);
                (self.group_ended, self.mesh_ended) = (group_ended, mesh_ended);
            }
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
            "Xy" => self.transparency(&vals, mesh_ended),
            // A group's own state (`Xd`, `6 () XW`) without an `Xy` keeps the current transparency, as
            // the file's PDF part draws it. A look's group (`9 () XW`) has no state of its own.
            "Xd" if group_ended => {
                let gs = self.gs.clone();
                if let Some(n) = self.last_object().filter(|n| matches!(n.kind, NodeKind::Group { .. })) {
                    (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape) =
                        (gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape);
                }
            }
            "Xw" => self.gs.hidden = Self::nums(&vals).last() == Some(&1.0),
            "A" => self.gs.locked = Self::nums(&vals).last() == Some(&1.0),
            "XW" => self.state_marker(&vals, hidden),
            // Gradients.
            "Bd" => self.gradient_def = GradientDef::begin(&vals, &mut self.names),
            "Bs" | "BD" => {}
            "Bb" => self.instance = Some(Instance::begin(&vals)),
            "Bg" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bg(&vals, &mut self.names);
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
            // `n [… (name) /Separation] XC`: the image's samples are that ink's tints. A device
            // space (`/DeviceGray XC` after a linked image's file name) changes nothing read here.
            "XC" => {
                let ink = paint::separation_ink(&vals, &mut self.names);
                if let Some(r) = self.raster.as_mut() {
                    r.ink = ink;
                }
            }
            // Placed files.
            // `[a b c d tx ty] llx lly urx ury (path) `` ` ``, three more numbers before the path in
            // newer files: the version 7 form has no `%AI5_BeginPlace` before it. Before the art,
            // the setup's procedures run.
            "`" if self.place.is_some() || !self.frames.is_empty() => self.place_tick(&vals, hidden),
            "~" if self.place.is_some() => self.end_place(hidden)?,
            // Type in the legacy format.
            "To" => self.begin_text(&vals, hidden),
            "Tp" => self.begin_text_path(&vals),
            "TP" => self.end_text_path(),
            "TO" => self.end_text(hidden)?,
            // `(text) justified TS`, special characters (§22.6): the reader adds its characters like `Tx`.
            "Tx" | "Tj" | "TX" | "TS" => self.text_body(w, &vals),
            "Tr" | "Tf" | "Ts" | "Tz" | "Tt" | "TA" | "Tl" | "Ta" | "Ti" | "TC" | "TW" | "Tq" | "Th" | "TV" | "Tv" | "Ty" | "TY" | "Tg" | "Xb"
            | "Xe" | "XB" => {
                self.text_attr(w, &vals);
            }
            // Attributes the reader doesn't keep (kerning pairs, line breaking and Japanese layout
            // rules) and final-form positions (§15.3).
            "Tk" | "TK" | "Tc" | "Tw" | "Tm" | "Td" | "T*" | "TR" | "T+" | "T-" | "TG" | "Tu" | "Xu" => {}
            // `TM` (pop text matrix) and `Te` (end render), listed in §22.6, keep nothing either; the
            // reader takes `Te` with any number of operands.
            "TM" | "Te" => {}
            // The hyphenation language (`XL`), which the reader doesn't keep either (§22).
            "XL" => {}
            "TE" => self.te(&vals),
            "TZ" => self.tz(&vals),
            // An image whose samples follow in comment lines (read at `%AI5_EndRaster`).
            "XI" if self.raster.is_some() => {
                let max = self.sample_bytes(&vals);
                if let Some(r) = self.raster.as_mut() {
                    r.operands = Some(vals);
                    r.max = max;
                }
            }
            // A linked image (§8.2, §8.3): `(path) modified XG` names the file of the `XF` image after
            // it, which has `XI`'s operands and no samples.
            "XG" | "XF" => self.image_link_op(w, vals, hidden)?,
            // A plugin group ends; a blend's record follows on `%` lines.
            "XP" => {
                let name = vals.iter().find_map(V::text).unwrap_or_default();
                let len = Self::nums(&vals).last().copied().unwrap_or(0.0);
                if name == crate::import::ai::blend::PLUGIN && len.is_finite() && len > 0.0 && len <= crate::import::ai::blend::MAX_RECORD as f64 {
                    self.blend_record = Some((len as usize, vec![]));
                } else if name == crate::import::ai::envelope::PLUGIN {
                    self.end_envelope();
                }
            }
            _ if IGNORED.contains(&w) => {}
            // Before the art, the setup runs procedures of the printing format.
            _ if self.frames.is_empty() => {}
            _ => self.note_unknown(w, hidden),
        }
        Ok(())
    }

    /// A graph's records (version 7 specification §17 and appendix A): `%_Gs` starts the graph on
    /// a layer, `GS` ends it, and its data and settings (the operators in [`GRAPH_OPS`], and any
    /// operator on a `%_` line between `Gt` and `GT`) are left out with their operands. Its drawn
    /// art, and anything else, is read as usual. `true` when `w` was one of them.
    fn graph_record(&mut self, w: &str, hidden: bool) -> bool {
        let Some(g) = self.graph.as_mut() else {
            // Outside a graph, its operators stay ones the reader doesn't know.
            if w == "Gs" && hidden && !self.frames.is_empty() {
                self.graph = Some(Graph { settings: false, frames: self.frames.len() });
                self.warn(GRAPH_PLAIN);
                return true;
            }
            return false;
        };
        match w {
            // A graph doesn't start inside another: the one being read goes on.
            "Gs" => self.warn(GRAPH_UNENDED),
            "GS" => self.graph = None,
            "Gt" => g.settings = true,
            "GT" => g.settings = false,
            _ if GRAPH_OPS.contains(&w) => {}
            _ if hidden && g.settings => {}
            _ => return false,
        }
        true
    }

    /// `… Lb`: a layer's options, `visible preview unlocked printing dimmed`, then the colour index
    /// and its RGB. The colour index is at 5 with fewer than ten operands, 6 with ten to twelve
    /// (`masks` before it) and 7 with thirteen or more (one more flag before it). Four places after
    /// the colour index comes the template flag (eleven operands or more), then the dimming
    /// percentage (twelve or more, else 50). A fourteenth operand changes nothing here.
    fn layer_attrs(&mut self, vals: &[V]) {
        let n = Self::nums(vals);
        let Some(Frame { kind: Kind::Layer(a), .. }) = self.frames.last_mut() else { return };
        let flag = |i: usize, default: bool| n.get(i).map_or(default, |v| *v != 0.0);
        a.visible = flag(0, true);
        a.preview = flag(1, true);
        a.locked = !flag(2, true);
        a.printable = flag(3, true);
        let at = 5 + usize::from(n.len() >= 10) + usize::from(n.len() >= 13);
        a.template = flag(at + 4, false);
        let dim_pct = n.get(at + 5).copied().filter(|v| v.is_finite()).unwrap_or(50.0).clamp(0.0, 100.0);
        a.dim = flag(4, false).then_some(dim_pct.round() as u8);
        let index = n.get(at).copied().unwrap_or(0.0);
        let rgb = |i: usize| n.get(at + i).map_or(0, |v| v.clamp(0.0, 255.0) as u8);
        // The file numbers some presets differently from `LAYER_COLORS` (#6502).
        a.color = Some(if (0.0..vectorcraft_doc::LAYER_COLORS.len() as f64).contains(&index) && index.fract() == 0.0 {
            LayerColor::Preset(crate::ai_ops::preset_of_ai_layer_index(index as u8))
        } else {
            LayerColor::Custom([rgb(1), rgb(2), rgb(3)])
        });
    }

    /// `(name) x y scaleX scaleY angle reflect axis shear shearAxis [matrix] p` (`P`: the stroke's):
    /// the pattern swatch `name`, its space taken to art space by the matrix. A pattern this hasn't
    /// read can't be drawn. For a pattern defined in the version 7 form, the other operands are
    /// applied in order before the initial matrix ([`pattern_controls`]).
    fn pattern_paint(&mut self, w: &str, vals: &[V]) {
        // In the setup it paints nothing (and the art's space isn't fixed yet).
        if self.frames.is_empty() {
            return;
        }
        let name = vals.iter().find(|v| matches!(v, V::Str(_))).and_then(|v| self.names.text(v));
        let Some(tile) = name.as_deref().and_then(|n| self.pattern_tiles.get(n)).copied() else {
            self.unread_pattern(w);
            return;
        };
        let mut matrix = vals.iter().rev().find_map(|v| if let V::Arr(a) = v { paint::affine(&Self::nums(a)) } else { None });
        if name.as_ref().is_some_and(|n| self.v7_patterns.contains(n)) {
            let controls: Vec<f64> =
                vals.iter().skip_while(|v| !matches!(v, V::Str(_))).skip(1).take_while(|v| !matches!(v, V::Arr(_))).filter_map(V::num).collect();
            let Some(controls) = pattern_controls(&controls) else {
                self.unread_pattern(w);
                return;
            };
            matrix = Some(matrix.unwrap_or(Affine::IDENTITY) * controls);
        }
        let Some(xf) = pattern_xf(self.space(), matrix, tile) else {
            self.unread_pattern(w);
            return;
        };
        let p = Paint::Pattern { pattern: name.as_deref().map(|n| self.global_names.get(Global::Pattern, n).to_string()).unwrap_or_default(), xf };
        if w == "p" {
            self.pending_unread_fill = false;
            self.gs.fill = p;
        } else {
            self.pending_unread_stroke = false;
            self.gs.stroke = p;
        }
    }

    fn unread_pattern(&mut self, w: &str) {
        if self.shown() {
            if w == "p" {
                self.pending_unread_fill = true;
            } else {
                self.pending_unread_stroke = true;
            }
        }
        // In a pattern's definition, only art painted with it makes the tile unreadable (see `paint`).
        if self.pattern.is_none() && (!self.shown() || self.gs.hidden) {
            self.hidden_unread.insert("pattern fills".into());
        }
    }

    fn color(&mut self, w: &str, vals: &[V]) {
        let Some((p, named)) = paint::color_op_in(w, vals, &mut self.names) else { return };
        // Registration (`Xs`, `Xz`) says nothing of the document's colour model.
        let registration = matches!(w, "Xs" | "XS" | "Xz" | "XZ") || p.is_registration();
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
            self.pending_unread_fill = false;
            if let Some(head) = self.symbol.as_mut() {
                head.missing_fill = false;
            }
        } else {
            self.gs.stroke = p;
            self.pending_stroke = None;
            self.pending_unread_stroke = false;
            if let Some(head) = self.symbol.as_mut() {
                head.missing_stroke = false;
            }
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

    /// Operator `w` ends what follows an object (its name, transparency and style) unless it is part
    /// of a state block.
    /// The state block of a layer whose art ended (`Xy`, `Xd`, `XW`, then `LB`) is read at the
    /// `LB`, and can have the stroke style between its `Xy` and `Xd`: that keeps the layer's state,
    /// and what a mesh or blend before it had (`mesh_undo`).
    /// After an image, the operators that end its raster (`XH`, `Xr` and the `N` after
    /// `%AI5_EndRaster`) keep it the object its name and opacity mask go to.
    fn state_block_continues(&mut self, w: &str) {
        if self.image_tail && matches!(w, "XH" | "XT" | "Xr" | "N") {
            return;
        }
        self.image_tail = false;
        // An object's tags (`XT`) follow it one after another, and an image's follow the `XH` that
        // ends its data.
        if matches!(w, "Xy" | "Xd" | "XW" | "BB" | "LB" | "XT" | "XH") {
            return;
        }
        self.after_object = false;
        if !matches!(w, "J" | "j" | "w" | "M" | "d" | "XR") {
            self.layer_state = None;
            self.mesh_undo = None;
            self.container_undo = None;
        }
    }

    /// `n (style) XW`: the end of a state block, or a style marker ([`Self::style_marker`]).
    fn state_marker(&mut self, vals: &[V], hidden: bool) {
        let code = Self::nums(vals).last().copied();
        // An object's own state block ends `6 () XW`: an `Xy` after a mesh's end was the mesh's.
        if code == Some(6.0) && self.mesh_undo.take().is_some() {
            self.layer_state = None;
        }
        match code {
            Some(6.0) => self.container_undo = None,
            Some(7.0) => self.layer_state_block(),
            _ => {}
        }
        self.style_marker(vals, hidden)
    }

    /// `7 () XW` ends a layer's own state block (after its art, before `LB`): an `Xy` that went to
    /// the layer's last group or compound path, right after its end, was the layer's.
    fn layer_state_block(&mut self) {
        let Some(before) = self.container_undo.take() else { return };
        let gs = &self.gs;
        let state = (gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape);
        let Some(Frame { kind: Kind::Layer(_), children, .. }) = self.frames.last_mut() else { return };
        let Some(n) =
            children.last_mut().filter(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Compound { .. } | NodeKind::SymbolInstance { .. }))
        else {
            return;
        };
        (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape) = before;
        self.layer_state = Some(state);
    }

    /// `mode opacity isolate knockout shape Xy`; after a group's end or a symbol instance, theirs
    /// too, and right after a gradient mesh's `/End X!` or a blend's record (before their `Xd` and
    /// `6 () XW`), theirs, unless the layer's `LB` follows: then it is the layer's (see `end_layer`).
    fn transparency(&mut self, vals: &[V], mesh_ended: bool) {
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
        let in_layer = matches!(self.frames.last(), Some(Frame { kind: Kind::Layer(_), .. }));
        if let Some(n) =
            self.last_object().filter(|n| (mesh_ended && matches!(n.kind, NodeKind::Mesh(_))) || matches!(n.kind, NodeKind::Blend { .. }))
        {
            // The mesh's or blend's, or the layer's when it is the layer's last object: kept for either.
            let before = (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape);
            (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape) = (gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape);
            self.mesh_undo = Some(before);
            self.layer_state = in_layer.then_some((gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape));
        } else if let Some(n) = self.last_object().filter(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Compound { .. })) {
            let before = (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape);
            (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape) = (gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape);
            // The layer's instead when the layer's `7 () XW` follows (`layer_state_block`).
            self.container_undo = in_layer.then_some(before);
            self.layer_state = None;
        } else if let Some(n) = self.last_object().filter(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })) {
            // A symbol instance's, as a group's.
            let before = (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape);
            (n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape) = (gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape);
            self.container_undo = in_layer.then_some(before);
            self.layer_state = None;
        } else {
            // After a layer's art and before its `LB`: the layer's own state (#2003). Kept until
            // an operator that isn't part of the state block (or the `LB` that takes it) says the
            // art moves on, so a path's state before its paint isn't read as its layer's.
            self.layer_state = in_layer.then_some((gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape));
        }
        self.note_state();
    }

    /// Inside a dictionary in the art (a text's frame, a mask's art), the transparency just set
    /// stays after the dictionary: the writer goes on from it, so the objects after an area text
    /// whose frame sets one have no `Xy` of their own.
    fn note_state(&mut self) {
        let gs = &self.gs;
        let state = (gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape);
        if self.frames.iter().any(|f| matches!(f.kind, Kind::Layer(_)))
            && let Some(f) = self.frames.iter_mut().rev().find(|f| matches!(f.kind, Kind::Obj(_)))
        {
            f.state = Some(state);
        }
    }

    /// A dictionary's art set transparency `state` (see [`Self::note_state`]): it is the current
    /// one now, and the enclosing dictionary's.
    pub(super) fn keep_state(&mut self, state: (f32, BlendMode, bool, Knockout, bool)) {
        let gs = &mut self.gs;
        (gs.opacity, gs.blend, gs.isolate, gs.knockout, gs.knockout_shape) = state;
        self.note_state();
    }

    /// Brings [`Reader::brushes_kept`] up to date with the brushes and brush art read so far.
    fn note_brushes_kept(&mut self) {
        use super::super::brush;
        let read = (self.brush_drafts.len(), self.brush_art_sections);
        if self.brushes_kept.read != read {
            self.brushes_kept = BrushesKept {
                read,
                kept: brush::kept(&self.brush_drafts, &self.brush_art),
                bending: brush::bending_art(&self.brush_drafts, &self.brush_art),
            };
        }
    }

    /// `n (style) XW` after an object: `1 (style)` after the object written on `%_` lines, its
    /// drawn look before it (all on `%_` lines inside another such object). Other markers end what
    /// follows an object.
    fn style_marker(&mut self, vals: &[V], hidden: bool) {
        let code = Self::nums(vals).last().copied().unwrap_or(0.0);
        let style = vals.iter().rev().find_map(|v| self.names.text(v)).unwrap_or_default();
        let after = std::mem::take(&mut self.after_object);
        // An object's own state block ends `6 () XW` (a layer's `7 () XW`): an `Xy` before it isn't the layer's.
        if code == 6.0 {
            self.layer_state = None;
        }
        if code == 9.0
            && let Some(look) = self
                .frames
                .last()
                .and_then(|f| f.children.last())
                .filter(|n| matches!(n.kind, NodeKind::Group { .. }) && !self.compound_groups.contains(&n.id))
        {
            self.looks.insert(look.id);
        }
        // The type that a text object's look draws is the object's own, whatever comes of its style.
        let mut look_drew_type = false;
        if after
            && let Some(f) = self.frames.last_mut()
            && let [.., look, object] = f.children.as_mut_slice()
            && slot_of(object.name.as_deref()).is_some()
            && !super::is_look_slot(object.name.as_deref())
            && self.hidden_ids.contains(&object.id)
            && self.hidden_ids.contains(&look.id) == hidden
            && matches!(look.kind, NodeKind::Group { clip: false, .. })
        {
            look_drew_type = strip_look_type(look);
        }
        if code != 1.0 || style.is_empty() || !after {
            return;
        }
        let space = self.space();
        self.note_brushes_kept();
        let page = self.page_in_doc();
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
        // A variable-width stroke's look that draws nothing but copies read from `%_` lines: an
        // empty group, which [`styled`] takes the stroke for.
        let drawn;
        let look = if !hidden
            && slot_of(object.name.as_deref()).is_none()
            && matches!(look.kind, NodeKind::Group { .. })
            && self.styles.get(&style).is_some_and(|st| st.uneven)
            && !draws(look, &self.hidden_ids)
        {
            let mut copy = look.clone();
            if let Some(children) = copy.children_mut() {
                children.clear();
            }
            drawn = copy;
            &drawn
        } else {
            look
        };
        // A style read here: the object, painted as it says.
        let appearance = self.styles.get(&style).and_then(|st| {
            styled(
                object,
                look,
                st,
                &StylePaints {
                    gradients: &self.gradients,
                    patterns: &self.pattern_tiles,
                    names: &self.global_names,
                    v7_patterns: &self.v7_patterns,
                    brushes: &self.brush_drafts,
                    kept: &self.brushes_kept.kept,
                    bending: &self.brushes_kept.bending,
                    space,
                },
            )
            .map(|mut appearance| {
                // Strokes and effects at the document's scale, as the art (a Large Canvas copy's is scaled up).
                let k = space.determinant().abs().sqrt();
                if k.is_finite() && k > 0.0 && (k - 1.0).abs() > 1e-9 {
                    appearance.scale_strokes(k);
                    appearance.scale_effects(k, vectorcraft_effects::scale_effect);
                }
                appearance
            })
        });
        // An image with a Warp is the object only where its live warp draws as its drawn look, and
        // while the import has a check left for it; otherwise its drawn look stays.
        let appearance = match appearance {
            Some(ap) if matches!(object.kind, NodeKind::Image(_)) && ap.effects.iter().any(|e| e.id.starts_with("warp.")) => {
                let alike = self.envelope_checks > 0 && {
                    self.envelope_checks -= 1;
                    let layer = self.doc.alloc_id();
                    let shared = crate::import::ai::envelope::Shared { patterns: &self.doc.patterns, images: &self.doc.images };
                    let shown = !hidden && object.visible && look.visible;
                    image_warp_alike(object, &ap, look, &self.hidden_ids, hidden, layer, &shared, page.filter(|_| shown), &mut self.envelope_spent)
                };
                alike.then_some(ap)
            }
            other => other,
        };
        if let Some(appearance) = appearance {
            let Some(mut object) = f.children.pop() else { return };
            let Some(look) = f.children.pop() else { return };
            let left_out = look.visual_bounds().filter(|_| !hidden && object.visible && look.visible);
            if let Some(b) = left_out {
                self.restyled.push(b);
            }
            // Warped type is checked against its look once it is laid out. The look stands in for
            // the type only when it holds all that the object draws: not when it draws nothing or
            // draws type of its own.
            if slot_of(object.name.as_deref()).is_some() && crate::import::ai::envelope::warps(&appearance) && self.warped_type.len() < MAX_NODES {
                let mut saved = look.clone();
                // When the object itself is on `%_` lines, every child of its look is too, so the
                // look's own copies can't be told apart from the rest.
                if !hidden {
                    strip(&mut saved, &self.hidden_ids);
                }
                if !look_drew_type && !holds_type(&saved) && saved.visual_bounds().is_some() && draws_art(&saved, 0) {
                    take_attributes(&mut saved, &object);
                    self.warped_type.push(crate::import::ai::envelope::WarpedLook { slot: object.id, look: saved, left_out });
                }
            }
            object.appearance = appearance.into();
            // The object's transparency is written on its look, after the look's group ends.
            (object.opacity, object.blend, object.isolate, object.knockout, object.knockout_shape) =
                (look.opacity, look.blend, look.isolate, look.knockout, look.knockout_shape);
            // Now the object itself, not a copy an enclosing look's `strip` would remove.
            self.hidden_ids.remove(&object.id);
            // Linked to the style when the style list has its name ([`Reader::finish`]).
            if self.style_links.len() < MAX_NODES {
                self.style_links.push((object.id, style.clone()));
            }
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
            if empty || (look.opacity >= 1.0 && look.blend == BlendMode::Normal && !look.isolate && !look.knockout_shape) {
                (object.opacity, object.blend, object.isolate, object.knockout, object.knockout_shape) =
                    (look.opacity, look.blend, look.isolate, look.knockout, look.knockout_shape);
                if empty {
                    *look = object;
                } else {
                    f.children.push(object);
                }
                return;
            }
            // The look and the type go into a group that has the transparency.
            let Some(mut look) = f.children.pop() else { return };
            let state = (look.opacity, look.blend, look.isolate, look.knockout, look.knockout_shape);
            (look.opacity, look.blend, look.isolate, look.knockout, look.knockout_shape) = (1.0, BlendMode::Normal, false, Knockout::Neutral, false);
            (object.opacity, object.blend, object.isolate, object.knockout_shape) = (1.0, BlendMode::Normal, false, false);
            let mut group = Node::group(self.doc.alloc_id(), vec![Arc::new(look), Arc::new(object)]);
            (group.opacity, group.blend, group.isolate, group.knockout, group.knockout_shape) = state;
            self.text_looks.insert(group.id);
            if let Some(f) = self.frames.last_mut() {
                f.children.push(group);
            }
            return;
        }
        look.name = object.name.clone().or(look.name.take());
        look.visible = object.visible;
        look.locked = object.locked;
        take_attributes(look, &object);
        if !hidden {
            self.drawn_looks += 1;
        }
    }
}

/// Does `n` or anything shown in it (to [`MAX_DEPTH`]) paint? Any object other than a path, a
/// compound path or a group counts: it draws more than its fills and strokes.
fn draws_art(n: &Node, depth: usize) -> bool {
    depth > MAX_DEPTH
        || !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Group { .. })
        || n.appearance.items.iter().any(|i| i.visible() && !i.paint().is_none())
        || n.children().into_iter().flatten().any(|c| c.visible && draws_art(c, depth + 1))
}

/// Is `n` or anything in it a text slot?
fn holds_type(n: &Node) -> bool {
    slot_of(n.name.as_deref()).is_some() || n.children().into_iter().flatten().any(|c| holds_type(c))
}

/// The note, Show Center, URL and data (object tags) of `object` stay with the `look` that stands
/// for it, in place of the look's own where the object has them.
fn take_attributes(look: &mut Node, object: &Node) {
    if let Some(a) = object.extras().attrs.as_deref().cloned() {
        look.edit_attrs(|l| {
            if a.show_center.is_some() {
                l.show_center = a.show_center;
            }
            if !a.url.is_empty() {
                l.url = a.url;
            }
            if !a.note.is_empty() {
                l.note = a.note;
            }
            let room = MAX_TAGS.saturating_sub(l.data.len());
            l.data.extend(a.data.into_iter().take(room));
        });
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

/// A style's pattern (`StyleItem::Pattern`): the file's pattern `name` placed by `matrix` (its
/// space → art space; `controls` apply first for a pattern defined in the version 7 form, `v7`
/// those patterns' names) and carried into the document by `space`, with the name `names` give
/// it there. `None` when the pattern wasn't read (`tiles`: the tiles of those that were, by their
/// names in the file) or doesn't place.
pub(super) fn style_pattern(
    name: &str,
    matrix: Option<[f64; 6]>,
    controls: &[f64],
    tiles: &HashMap<String, Rect>,
    v7: &HashSet<String>,
    names: &ShownNames,
    space: Affine,
) -> Option<Paint> {
    let tile = tiles.get(name).copied()?;
    let mut matrix = matrix.and_then(|m| paint::affine(&m));
    if v7.contains(name) {
        matrix = Some(matrix.unwrap_or(Affine::IDENTITY) * pattern_controls(controls)?);
    }
    let xf = pattern_xf(space, matrix, tile)?;
    Some(Paint::Pattern { pattern: names.get(Global::Pattern, name).to_string(), xf })
}

/// A pattern fill's `x y scaleX scaleY angle reflect axis shear shearAxis` as one transform of the
/// pattern's space, applied before its initial matrix: the shear (by `shear` degrees along the line
/// at `shearAxis` degrees), the reflection about the line at `axis` degrees when `reflect` isn't
/// 0, the counterclockwise rotation by `angle` degrees, the scale, then the move to `x y`. `None`
/// unless there are nine finite numbers.
fn pattern_controls(n: &[f64]) -> Option<Affine> {
    let &[x, y, sx, sy, angle, reflect, axis, shear, shear_axis] = n else { return None };
    if !n.iter().all(|v| v.is_finite()) {
        return None;
    }
    let turn = |deg: f64| Affine::rotate(deg.to_radians());
    let reflection = if reflect != 0.0 { turn(axis) * Affine::scale_non_uniform(1.0, -1.0) * turn(-axis) } else { Affine::IDENTITY };
    let shearing = if shear != 0.0 {
        turn(shear_axis) * Affine::new([1.0, 0.0, shear.to_radians().tan(), 1.0, 0.0, 0.0]) * turn(-shear_axis)
    } else {
        Affine::IDENTITY
    };
    let xf = Affine::translate((x, y)) * Affine::scale_non_uniform(sx, sy) * turn(angle) * reflection * shearing;
    xf.as_coeffs().iter().all(|v| v.is_finite()).then_some(xf)
}

/// What a style's fills paint with: the file's gradients and pattern tiles with the names they get
/// in the document, the patterns defined in the version 7 form, and art space → the document.
struct StylePaints<'a> {
    gradients: &'a HashMap<String, Gradient>,
    patterns: &'a HashMap<String, Rect>,
    names: &'a ShownNames,
    v7_patterns: &'a HashSet<String>,
    /// The brushes the file defines, which a brushed stroke names.
    brushes: &'a [super::super::brush::BrushDraft],
    /// Which of [`Self::brushes`] are kept as brushes ([`super::super::brush::kept`]).
    kept: &'a [bool],
    /// The art an Art brush draws as the file does ([`super::super::brush::bending_art`]).
    bending: &'a HashSet<String>,
    space: Affine,
}

/// A placed file's `` ` `` operands → its matrix, its box and its path: an array of six numbers, four
/// numbers, three more or none, and a string. `None` for any other shape or a number that isn't
/// finite.
fn tick(vals: &[V]) -> Option<(Affine, Rect, String)> {
    let [V::Arr(m), rest @ .., V::Str(path)] = vals else { return None };
    let m = <[f64; 6]>::try_from(m.iter().map(V::num).collect::<Option<Vec<f64>>>()?).ok()?;
    let nums: Vec<f64> = rest.iter().map(V::num).collect::<Option<_>>()?;
    let [x0, y0, x1, y1] = match nums.as_slice() {
        [x0, y0, x1, y1] | [x0, y0, x1, y1, _, _, _] => [*x0, *y0, *x1, *y1],
        _ => return None,
    };
    m.iter().chain(&nums).all(|v| v.is_finite()).then(|| (Affine::new(m), Rect::new(x0, y0, x1, y1), obj::text(path)))
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

/// Is `n` a path, a compound path or a group without a clip, with no opacity mask, the objects
/// whose outline the effects that move it (Transform, Twist, Free Distort) are read on? Images,
/// type (and a text object's slot, filled with type placed where the page has it) and live objects
/// draw more than their outline, and a mask stays where it is.
fn outlines_only(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Group { clip: false, .. })
        && n.extras().mask.is_none()
        && slot_of(n.name.as_deref()).is_none()
}

/// Does `n`, inside an object whose outline an effect moves, move with it: is it and everything in
/// it [`outlines_only`], painting with no gradient or pattern (which stay where they are placed)
/// and with no effects of its own (drawn after the outer effect, not before)? Deeper than
/// [`MAX_DEPTH`] counts as no.
fn moves_alike(n: &Node, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && outlines_only(n)
        && n.appearance.effects.is_empty()
        && n.appearance.items.iter().all(|i| i.effects().is_empty() && !matches!(i.paint(), Paint::Gradient(_) | Paint::Pattern { .. }))
        && n.children().into_iter().flatten().all(|c| moves_alike(c, depth + 1))
}

/// Does `n` or anything in it (to [`MAX_DEPTH`]) have a stroke that paints?
fn strokes(n: &Node, depth: usize) -> bool {
    depth > MAX_DEPTH
        || n.appearance.items.iter().any(|i| !i.is_fill() && !i.paint().is_none())
        || n.children().into_iter().flatten().any(|c| strokes(c, depth + 1))
}

/// Are all of path or compound path `n`'s segments straight, each anchor's handles on it? Any other
/// object, a group included, counts as no.
fn straight(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Path { path, .. } => path.subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.h_in == a.p && a.h_out == a.p),
        NodeKind::Compound { children, .. } => children.iter().all(|c| matches!(c.kind, NodeKind::Path { .. }) && straight(c)),
        _ => false,
    }
}

/// Is neither `n` nor anything in it a guide? Deeper than [`MAX_DEPTH`] counts as no.
fn no_guides(n: &Node, depth: usize) -> bool {
    depth <= MAX_DEPTH && !n.is_guide() && n.children().into_iter().flatten().all(|c| no_guides(c, depth + 1))
}

/// The largest Twist angle, in degrees either way, read as a live effect on an object that isn't a
/// path or compound path with straight segments: VectorCraft's own limit. An object with a larger
/// Twist keeps the file's drawn look.
const MAX_TWIST: f64 = 90.0;

/// Is `object` one that style `st`, whose effect moves the outline, is read on? It and everything
/// in it [`moves_alike`], and the style paints no gradient. Twist is read only when nothing is
/// stroked: on a path or compound path with straight segments at any angle, or up to [`MAX_TWIST`]
/// degrees either way on another object with no guide in it. Transform that scales, turns or reflects
/// is read only when nothing is stroked (its reference point lies on the bounds without strokes).
fn moves_with(object: &Node, st: &crate::import::ai::style::ArtStyle) -> bool {
    use crate::import::ai::style::StyleItem;
    let stroked = st.items.iter().any(|i| matches!(i, StyleItem::Stroke { paint, .. } if !paint.is_none()))
        || object.children().into_iter().flatten().any(|c| strokes(c, 1));
    !st.items.iter().any(|i| matches!(i, StyleItem::Gradient { .. }))
        && outlines_only(object)
        && object.children().into_iter().flatten().all(|c| moves_alike(c, 1))
        && st.effects.iter().all(|e| match e.id.as_str() {
            "distort.twist" => {
                !stroked && (straight(object) || (no_guides(object, 0) && e.params["angle"].as_f64().is_some_and(|a| a.abs() <= MAX_TWIST)))
            }
            "distort.transform" => {
                let p = &e.params;
                !stroked
                    || (p["scaleH"].as_f64() == Some(100.0)
                        && p["scaleV"].as_f64() == Some(100.0)
                        && p["rotate"].as_f64() == Some(0.0)
                        && p["reflectX"].as_bool() == Some(false)
                        && p["reflectY"].as_bool() == Some(false))
            }
            _ => true,
        })
}

/// Is `object` an image without an opacity mask, and `st` one Warp on it with nothing else that
/// paints ([`crate::import::ai::style::ArtStyle::image_warp`])? Such an image is read with its Warp
/// live where that draws as its drawn look ([`image_warp_alike`]).
fn image_warps(object: &Node, st: &crate::import::ai::style::ArtStyle) -> bool {
    matches!(object.kind, NodeKind::Image(_)) && object.extras().mask.is_none() && st.image_warp
}

/// Does image `object` with `appearance` (its style's Warp) draw as `look`, its drawn look, compared
/// over all that the warp draws ([`crate::import::ai::envelope::warped_alike`]) on `layer`, a layer
/// id no other node has? Both are drawn shown, opaque and in the Normal mode. Unless the look is on
/// `%_` lines itself (`hidden`), the copies in it that are on `%_` lines (`hidden_ids`) are left out.
#[allow(clippy::too_many_arguments)]
fn image_warp_alike(
    object: &Node,
    appearance: &Appearance,
    look: &Node,
    hidden_ids: &HashSet<NodeId>,
    hidden: bool,
    layer: NodeId,
    shared: &crate::import::ai::envelope::Shared<'_>,
    page: Option<Rect>,
    spent: &mut crate::import::ai::envelope::Spent,
) -> bool {
    let mut live = object.clone();
    live.appearance = appearance.clone().into();
    let mut plain = look.clone();
    if !hidden {
        strip(&mut plain, hidden_ids);
    }
    for n in [&mut live, &mut plain] {
        (n.visible, n.opacity, n.blend) = (true, 1.0, BlendMode::Normal);
    }
    crate::import::ai::envelope::warped_alike(&live, &plain, layer, shared, page, spent)
}

/// What [`placed_paints`] keys a pattern's name with, apart from the gradients' names.
const PATTERN_KEY: &str = "\u{1}pattern ";

/// `object`'s appearance with style `st`, its drawn look `look`: `None` when the style isn't one
/// this reads, asks a group for fills or strokes, has Pathfinder effects on anything but a group
/// with no fills or strokes, or moves the outline of an object it isn't read on ([`moves_with`]).
fn styled(object: &Node, look: &Node, st: &crate::import::ai::style::ArtStyle, paints: &StylePaints) -> Option<Appearance> {
    use crate::import::ai::style::StyleItem;
    // A text object's slot (an empty group standing for it) takes the style as the type will.
    let slot = slot_of(object.name.as_deref()).is_some();
    // Several fills and strokes, on a path or a compound path.
    let stacked = !slot && !st.simple && st.stacked && matches!(object.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
    // One Warp on an image, checked against its drawn look by the caller ([`image_warp_alike`]).
    let image_warp = !st.simple && image_warps(object, st);
    // A symbol instance draws its symbol's art: it keeps its drawn look.
    if !st.plain
        || matches!(object.kind, NodeKind::SymbolInstance { .. })
        // A look with no outline in it to keep takes a variable-width stroke as the object's own.
        || (!slot && !st.simple && !stacked && !image_warp && (!st.uneven || draws(look, &HashSet::new())))
        || (object.is_container() && !slot && !matches!(object.kind, NodeKind::Compound { .. }) && !st.items.is_empty())
        || (st.pathfinder
            && (slot || !st.items.is_empty() || object.is_compound_shape() || !matches!(object.kind, NodeKind::Group { clip: false, .. })))
        || (!slot && st.moves_outline() && !moves_with(object, st))
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
    // A stack's gradients are each its own swatch, placed where the drawn look has it: one fitted
    // to the object's bounds instead could lie elsewhere, so such a stack keeps its drawn look.
    if stacked {
        let mut seen = HashSet::new();
        let placed_apart = st.items.iter().all(|i| match i {
            StyleItem::Gradient { name: Some(n), .. } => seen.insert(n) && placed.contains_key(paints.names.get(Global::Gradient, n)),
            StyleItem::Gradient { name: None, .. } => false,
            _ => true,
        });
        if !placed_apart {
            return None;
        }
    }
    let mut items = vec![];
    for (n, item) in st.items.iter().enumerate() {
        let mut layer = match item {
            StyleItem::Fill(paint) => AppearanceItem::Fill(FillLayer::new(paint.clone())),
            StyleItem::Gradient { name, angle } => {
                // The drawn look's gradients are keyed by the names they got in the document.
                let paint = match (name.as_ref().and_then(|n| placed.get(paints.names.get(Global::Gradient, n))), gradients, own_gradient) {
                    // Other objects than type: the object's own, as they always were (a stack's
                    // where its drawn look places it).
                    (_, 1, Some(own)) if !slot && !stacked => own.paint.clone(),
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
            StyleItem::Pattern { name, matrix, controls } => match placed.get(&format!("{PATTERN_KEY}{}", paints.names.get(Global::Pattern, name))) {
                Some(p) => AppearanceItem::Fill(FillLayer::new(p.clone())),
                None => AppearanceItem::Fill(FillLayer::new(style_pattern(
                    name,
                    *matrix,
                    controls,
                    paints.patterns,
                    paints.v7_patterns,
                    paints.names,
                    paints.space,
                )?)),
            },
            StyleItem::Stroke { brush, .. } => {
                // A brushed stroke draws as its look only with the brush the Brushes panel keeps
                // under its name, at the object's settings, on a path, a compound path or type (an
                // image draws no stroke). An Art brush is read on a path or a compound path with
                // no effects.
                if let Some((name, settings)) = brush {
                    let (draft, _) = paints.brushes.iter().zip(paints.kept).find(|(d, kept)| **kept && &d.name == name)?;
                    if !super::super::brush::draws_as(draft, settings, paints.bending) {
                        return None;
                    }
                    let art = matches!(settings, super::super::brush::PathBrush::Art);
                    let on_a_path = matches!(object.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
                    if !on_a_path && (art || !slot) {
                        return None;
                    }
                    if art && !st.effects.is_empty() {
                        return None;
                    }
                }
                stroke_layer(item)?
            }
        };
        if let Some(look) = st.looks.get(n) {
            with_look(&mut layer, look);
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

/// A graphic style's appearance from art style `st`, on no object: `None` when the style isn't one
/// this reads, or paints with a gradient or a pattern (placed on each object as its drawn look has them).
/// A style that is one paint is its fill and stroke ([`crate::import::ai::style::ArtStyle::one_paint`]).
pub(super) fn appearance_from_art_style(st: &crate::import::ai::style::ArtStyle) -> Option<Appearance> {
    use crate::import::ai::style::StyleItem;
    if !st.plain {
        return None;
    }
    let style_items = match &st.one_paint {
        Some(one) => one.as_ref().ok()?,
        None => &st.items,
    };
    let mut items = vec![];
    for (n, item) in style_items.iter().enumerate() {
        let mut layer = match item {
            StyleItem::Fill(paint) => AppearanceItem::Fill(FillLayer::new(paint.clone())),
            // A graphic style's brush isn't checked against the file's brushes: it is left out.
            StyleItem::Stroke { brush: Some(_), .. } => return None,
            StyleItem::Stroke { .. } => stroke_layer(item)?,
            StyleItem::Gradient { .. } | StyleItem::Pattern { .. } => return None,
        };
        if let Some(look) = st.looks.get(n) {
            with_look(&mut layer, look);
        }
        items.push(layer);
    }
    Some(Appearance { items, effects: st.effects.clone(), contents_index: st.contents_index })
}

/// What the reader makes of art style `name` in `section` (the text between `%AI9_BeginArtStyles`
/// and its end), for writers that check a style before they write it.
#[derive(Clone, Debug)]
pub(crate) struct StyleRead {
    /// The style's appearance: a gradient fill is the object's own gradient.
    pub appearance: Appearance,
    /// An object other than type is read with it ([`crate::import::ai::style::ArtStyle::simple`]).
    pub simple: bool,
    /// An image without an opacity mask is read with it where its live warp draws as its drawn
    /// look ([`crate::import::ai::style::ArtStyle::image_warp`]).
    pub image_warp: bool,
    /// It has Effect › Pathfinder effects, which only a group with no fills or strokes is read with.
    pub pathfinder: bool,
    /// The opacity, blending mode, isolation and knockout its blend filter gives.
    pub transparency: Option<(f32, BlendMode, bool, Knockout)>,
}

/// The names a style list section gives graphic styles, as the reader reads them.
pub(crate) fn read_style_list(section: &[u8]) -> Vec<String> {
    crate::import::ai::style::art_style_list(section).iter().map(|raw| obj::text(raw)).filter(|name| !name.is_empty()).collect()
}

/// Art style `name` of `section` as the reader reads it ([`StyleRead`]): [`appearance_from_art_style`],
/// with a style's one gradient fill as `own_gradient`, the object's own gradient, as [`styled`] reads
/// it on objects other than type. A stroke that names a brush is read with that brush's name, as
/// [`styled`] reads it on a path or compound path when the Brushes panel keeps that brush at those
/// settings, which [`crate::import::ai::brush::BrushesRead::style_reads`] checks. `None` when the section
/// doesn't define it, the reader doesn't read it, or it paints with a pattern or a gradient that
/// isn't the object's own.
pub(crate) fn read_art_style(section: &[u8], name: &str, own_gradient: Option<&Paint>) -> Option<StyleRead> {
    use crate::import::ai::style::StyleItem;
    let mut st = crate::import::ai::style::art_styles(section).remove(name)?;
    if st.items.iter().filter(|i| matches!(i, StyleItem::Gradient { .. })).count() == 1 {
        let own = own_gradient?;
        for item in &mut st.items {
            if matches!(item, StyleItem::Gradient { .. }) {
                *item = StyleItem::Fill(own.clone());
            }
        }
    }
    // The brush names, by item; `appearance_from_art_style` reads a style's items in order.
    let brushes: Vec<Option<String>> = st
        .items
        .iter_mut()
        .map(|item| match item {
            StyleItem::Stroke { brush, .. } => brush.take().map(|(name, _)| name),
            _ => None,
        })
        .collect();
    let mut appearance = appearance_from_art_style(&st)?;
    for (item, brush) in appearance.items.iter_mut().zip(brushes) {
        if let (AppearanceItem::Stroke(s), Some(brush)) = (item, brush) {
            s.brush = Some(brush);
        }
    }
    Some(StyleRead { appearance, simple: st.simple, image_warp: st.image_warp, pathfinder: st.pathfinder, transparency: st.transparency })
}

/// Is art style `name` of `section` read on `object` as far as an effect that moves the outline
/// goes: the style has none, or `object` is one [`moves_with`] reads it on, as [`styled`] checks?
/// `false` when the section doesn't define it.
pub(crate) fn style_moves_with(section: &[u8], name: &str, object: &Node) -> bool {
    crate::import::ai::style::art_styles(section).remove(name).is_some_and(|st| !st.moves_outline() || moves_with(object, &st))
}

/// A style's stroke as an appearance item (`None` for a fill).
fn stroke_layer(item: &crate::import::ai::style::StyleItem) -> Option<AppearanceItem> {
    let crate::import::ai::style::StyleItem::Stroke { paint, width, cap, join, miter, dash, profile, align, brush } = item else { return None };
    let mut layer = StrokeLayer::new(paint.clone(), line_width(*width));
    (layer.cap, layer.join, layer.miter_limit, layer.dash, layer.profile, layer.align) = (*cap, *join, *miter, dash.clone(), profile.clone(), *align);
    layer.brush = brush.as_ref().map(|(name, _)| name.clone());
    Some(AppearanceItem::Stroke(layer))
}

/// Give `layer` a fill's or stroke's own visibility, transparency and effects.
fn with_look(layer: &mut AppearanceItem, look: &crate::import::ai::style::ItemLook) {
    match layer {
        AppearanceItem::Fill(f) => (f.visible, f.opacity, f.blend, f.effects) = (look.visible, look.opacity, look.blend, look.effects.clone()),
        AppearanceItem::Stroke(s) => (s.visible, s.opacity, s.blend, s.effects) = (look.visible, look.opacity, look.blend, look.effects.clone()),
    }
}

/// Remove from `n` the text slots of the type its look draws ([`super::is_look_slot`]), in its groups
/// too. Returns whether there were any.
fn strip_look_type(n: &mut Node) -> bool {
    let mut any = false;
    if let Some(children) = n.children_mut() {
        let before = children.len();
        children.retain(|c| !super::is_look_slot(c.name.as_deref()));
        any = children.len() != before;
        for c in children.iter_mut().filter(|c| c.children().is_some_and(|k| !k.is_empty())) {
            any |= strip_look_type(Arc::make_mut(c));
        }
    }
    any
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

/// Whether `n` holds anything that draws, apart from the children read from `%_` lines (`hidden`):
/// a path or a compound path with a paint of its own, or a node other than a group or those.
fn draws(n: &Node, hidden: &HashSet<NodeId>) -> bool {
    if hidden.contains(&n.id) {
        return false;
    }
    let paints = n.appearance.items.iter().any(|i| i.visible() && !i.paint().is_none());
    match (&n.kind, n.children()) {
        (NodeKind::Compound { .. }, Some(children)) => paints && children.iter().any(|c| c.visible && !hidden.contains(&c.id)),
        (NodeKind::Group { .. }, Some(children)) => children.iter().any(|c| c.visible && draws(c, hidden)),
        (NodeKind::Path { .. }, _) => paints,
        _ => true,
    }
}

/// Whether ASCII85 data in comment lines, up to its `~>`, follows the entry keyed `key` of a
/// dictionary of type `ty`: the `/ASCII85Decode` of a `/Binary` dictionary or a text document, or a
/// non-native art object's (`/ForeignObject`'s) `/Data`.
pub(super) fn ascii85_follows(ty: &str, key: Option<&str>) -> bool {
    matches!((ty, key), ("Binary" | "AI11TextDocument" | "AI11UndoFreeTextDocument", Some("ASCII85Decode")) | ("ForeignObject", Some("Data")))
}

/// The dictionaries (`/Type :` … `;`) a scan of tokens passes that the reader doesn't read, kept to
/// tell where an entry's ASCII85 data follows ([`ascii85_follows`]), so the scan can skip past it
/// as the reader does: a `(` on a `%_` line of the data would otherwise open a string.
#[derive(Clone, Default)]
pub(super) struct Dictionaries<'s> {
    /// The open dictionaries' types, innermost last, up to [`MAX_DEPTH`].
    types: Vec<&'s [u8]>,
    /// How many more are open inside the last of `types`.
    deeper: usize,
    /// The name just before the token taken.
    name: Option<&'s [u8]>,
    /// Whether the string just before the token taken is a key that ASCII85 data follows.
    string_key: bool,
    /// The end markers of the sections being scanned (`None`: no section) where the rest of the data
    /// had no `~>` or marker to stop at, so they aren't searched for again in those sections.
    no_end: Vec<Option<String>>,
}

/// Where the ASCII85 data at the start of `rest` stops: just past its `~>`, or at the comment that
/// comes first with a marker of `section`, the begin and end markers of the section being scanned
/// (no begin marker when it's empty). A line of the data that reads as one of them counts as the
/// marker. `None` when `rest` has neither.
fn data_end(rest: &[u8], section: Option<(&str, &str)>) -> Option<usize> {
    let mut at = 0;
    for line in rest.split(|b| matches!(b, b'\r' | b'\n')) {
        // The first line is what is left of the key's line, where a `%` starts no marker.
        if at > 0
            && let Some((comment, (begin, end))) = line.strip_prefix(b"%").zip(section)
        {
            let comment = String::from_utf8_lossy(comment);
            let marker = super::marker(&comment);
            if marker == end || (!begin.is_empty() && marker == begin) {
                return Some(at);
            }
        }
        if let Some(i) = line.windows(2).position(|w| w == b"~>") {
            return Some(at + i + 2);
        }
        at += line.len() + 1;
    }
    None
}

impl<'s> Dictionaries<'s> {
    /// Take `tok`, the token `lex` just gave, and skip the ASCII85 data that follows it, up to where
    /// [`data_end`] stops for `section`, so the scan still finds that section's markers.
    pub(super) fn pass(&mut self, tok: &super::Tok<'s>, lex: &mut super::Lexer<'s>, section: Option<(&str, &str)>) {
        let end = section.map(|(_, end)| end);
        if self.data_follows(tok) && !self.no_end.iter().any(|e| e.as_deref() == end) {
            match data_end(lex.rest(), section) {
                Some(stop) => lex.seek(lex.offset() + stop),
                None => self.no_end.push(end.map(str::to_owned)),
            }
        }
    }

    /// Take the next token → whether ASCII85 data follows it.
    fn data_follows(&mut self, tok: &super::Tok<'s>) -> bool {
        // Comments between a name and what follows it are passed over, as the reader passes them.
        if matches!(tok, super::Tok::Comment(_)) {
            return false;
        }
        let name = self.name.take();
        let string_key = std::mem::take(&mut self.string_key);
        match tok {
            super::Tok::Name(n) => self.name = Some(n),
            // A string keys an entry as a name does, but gives a dictionary no type.
            super::Tok::Str(s) | super::Tok::Hex(s) => self.string_key = self.keys_data(s),
            super::Tok::Word(b":") if self.types.len() < MAX_DEPTH => self.types.push(name.unwrap_or_default()),
            super::Tok::Word(b":") => self.deeper = self.deeper.saturating_add(1),
            super::Tok::Word(b";") if self.deeper > 0 => self.deeper -= 1,
            super::Tok::Word(b";") => {
                self.types.pop();
            }
            super::Tok::Word(b",") => return string_key || name.is_some_and(|key| self.keys_data(key)),
            _ => {}
        }
        false
    }

    /// Whether ASCII85 data follows an entry keyed `key` of the innermost dictionary.
    fn keys_data(&self, key: &[u8]) -> bool {
        let Some(ty) = self.types.last().filter(|_| self.deeper == 0) else { return false };
        let (Ok(ty), Ok(key)) = (std::str::from_utf8(ty), std::str::from_utf8(key)) else { return false };
        ascii85_follows(ty, Some(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::ImageObject;
    use vectorcraft_geom::shapes;

    fn square(id: u64, fill: Paint) -> Arc<Node> {
        Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(fill, Paint::None, 0.0)))
    }

    /// An image draws no stroke, so an image with a brush in its art style keeps its drawn look.
    #[test]
    fn an_image_with_a_brush_keeps_its_drawn_look() {
        use super::super::super::brush::{Tool, parse_tool_object};
        let tool = "Adobe Calligraphic Brush Tool"; // brand-ok: the filter's name in the files
        let text = format!(
            "%AI9_BeginArtStyles\n/KnownStyle :\n(Inked) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
             /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
             /BasicFilter :\n({tool}) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n\
             /StrokeStyle : 0 R\n0 1 1 0 1 0 0 XA\n0 1 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n\
             6 /Real (Diameter) ,\n2 /Real (DiameterVariation) ,\n2 /Int (DiameterDependsOn) ,\n\
             40 /Real (Roundness) ,\n10 /Real (RoundnessVariation) ,\n5 /Int (RoundnessDependsOn) ,\n\
             30 /Real (Angle) ,\n180 /Real (AngleVariation) ,\n6 /Int (AngleDependsOn) ,\n0 /Int (AngleRelativeTo) ,\n\
             /GObjRef : (N) ({tool}) /PluginObject ; (Brush) ,\n; /Dict ;\n /Part ,\n;\n /Part ,\n;\n%AI9_EndArtStyles\n"
        );
        let styles = crate::import::ai::style::art_styles(text.as_bytes());
        let st = &styles["Inked"];
        let brushes = [parse_tool_object(Tool::Calligraphic, "N", "1 4 8 30 50 -150 210 2 5 6 0").unwrap()];
        let (gradients, patterns, names, v7_patterns, bending) =
            (HashMap::new(), HashMap::new(), ShownNames::default(), HashSet::new(), HashSet::new());
        let paints = StylePaints {
            gradients: &gradients,
            patterns: &patterns,
            names: &names,
            v7_patterns: &v7_patterns,
            brushes: &brushes,
            kept: &[true],
            bending: &bending,
            space: Affine::IDENTITY,
        };
        let look = Node::group(NodeId(1), vec![square(2, Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)))]);
        assert!(styled(&square(3, Paint::None), &look, st, &paints).is_some(), "a path takes the style");
        let image =
            ImageObject { key: "a".into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        assert!(styled(&Node::new(NodeId(4), NodeKind::Image(Box::new(image))), &look, st, &paints).is_none());
    }

    /// The style `Fx`: `effects` (each a filter's name and its dictionary's keys) on the whole
    /// object, over a fill and a stroke that `paints` turns on (`0 1`: the fill paints blue).
    fn effects_style(effects: &[(&str, &str)], paints: &str) -> crate::import::ai::style::ArtStyle {
        let filters: String = effects
            .iter()
            .map(|(name, keys)| {
                format!("/BasicFilter :\n({name}) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n{keys}; /Dict ;\n /Part ,\n")
            })
            .collect();
        let text = format!(
            "%AI9_BeginArtStyles\n/KnownStyle :\n(Fx) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
             {filters}/CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
             /BasicFilter :\n(Stroke Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/StrokeStyle : 0 1 0 0 0 Xy\n0 0 Xd\n/Def ;\n (StrokeStyle) ,\n; /Dict ;\n /Part ,\n\
             /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n\
             /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n0 0 0 0 0 0 1 Xa\n{paints} Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
             ;\n /Part ,\n;\n%AI9_EndArtStyles\n"
        );
        crate::import::ai::style::art_styles(text.as_bytes()).remove("Fx").unwrap_or_default()
    }

    /// An image takes a style of one Warp, under fills and strokes that paint nothing, and no
    /// other style with a geometry effect: those keep its drawn look. A path, and an image with an
    /// opacity mask, keep their drawn look with the Warp too.
    #[test]
    fn an_image_takes_a_style_of_one_warp_alone() {
        // GUARDS: an image with a Warp in its art style came in as its drawn look.
        let warp = ("Adobe Deform", "1 /Int (DeformStyle) ,\n0.4 /Real (DeformValue) ,\n"); // brand-ok: the filter's name in the files
        let (gradients, patterns, names, v7_patterns, bending) =
            (HashMap::new(), HashMap::new(), ShownNames::default(), HashSet::new(), HashSet::new());
        let paints = StylePaints {
            gradients: &gradients,
            patterns: &patterns,
            names: &names,
            v7_patterns: &v7_patterns,
            brushes: &[],
            kept: &[],
            bending: &bending,
            space: Affine::IDENTITY,
        };
        let look = Node::group(NodeId(1), vec![square(2, Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)))]);
        let image =
            ImageObject { key: "a".into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        let image = Node::new(NodeId(4), NodeKind::Image(Box::new(image)));
        let st = effects_style(&[warp], "0 0");
        assert!(st.image_warp && !st.simple, "{st:?}");
        let ap = styled(&image, &look, &st, &paints).expect("an image takes one Warp");
        let ids: Vec<&str> = ap.effects.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["warp.arc"]);
        assert!(ap.items.iter().all(|i| i.paint().is_none()), "{ap:?}");
        // Not on a path, nor on an image with an opacity mask.
        assert!(styled(&square(3, Paint::None), &look, &st, &paints).is_none());
        let mut masked = image.clone();
        masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(Node::group(NodeId(7), vec![]), true)));
        assert!(styled(&masked, &look, &st, &paints).is_none());
        // A Warp over a fill that paints, a Warp with another effect, and the other geometry effects.
        let still = ("Adobe Punk and Bloat", "0 /Real (d_factor) ,\n");
        let others: [(&[(&str, &str)], &str); 8] = [
            (&[warp], "0 1"),
            (&[warp, still], "0 0"),
            (&[still, warp], "0 0"),
            (&[("Adobe Twirl", "45 /Real (angle) ,\n")], "0 0"),
            (&[("Adobe Free Distort", "")], "0 0"),
            (&[("Adobe Punk and Bloat", "-50 /Real (d_factor) ,\n")], "0 0"),
            (&[("Adobe Zigzag", "3 /Real (amount) ,\n1 /Real (absoluteness) ,\n")], "0 0"),
            (&[("Adobe Transform", "5 /Real (moveH_Pts) ,\n0.5 /Real (rotate_Radians) ,\n0 /Bool (transformPatterns) ,\n")], "0 0"),
        ];
        for (effects, painted) in others {
            let st = effects_style(effects, painted);
            assert!(!st.image_warp, "{effects:?} {painted}");
            assert!(styled(&image, &look, &st, &paints).is_none(), "{effects:?} {painted}");
        }
    }

    #[test]
    fn data_stops_at_its_end_or_a_marker_of_the_section() {
        let section = Some(("BeginTextDocument", "EndTextDocument"));
        assert_eq!(data_end(b"\n%_(a~>\n%AI11_EndTextDocument\n", section), Some(7));
        assert_eq!(data_end(b"\n%_(a\n%AI11_EndTextDocument\n", section), Some(6));
        assert_eq!(data_end(b"\n%_(a\n%AI11_BeginTextDocument\n", section), Some(6));
        // Bytes that aren't UTF-8 after the marker, as the scan reads a comment.
        assert_eq!(data_end(b"\n%_(a\n%AI11_EndTextDocument \xff\n", section), Some(6));
        // Right after the key a `%` starts no marker, and an empty begin marker matches no comment.
        assert_eq!(data_end(b"%AI11_EndTextDocument\n%_(a\n", section), None);
        assert_eq!(data_end(b"\n%\n", Some(("", "EndPalette"))), None);
        assert_eq!(data_end(b"\n%AI11_EndTextDocument\n", None), None);
    }

    #[test]
    fn only_outlines_without_gradients_move_alike() {
        // GUARDS: an image's pixels and a placed gradient stay behind when an effect moves the outline.
        let solid = || square(2, Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0)));
        assert!(moves_alike(&Node::group(NodeId(1), vec![solid(), solid()]), 1));
        let gradient = square(3, Paint::Gradient(Box::new(vectorcraft_color::gradient::GradientPaint::new(Gradient::default()))));
        assert!(!moves_alike(&Node::group(NodeId(1), vec![solid(), gradient]), 1));
        let image =
            ImageObject { key: "a".into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        let image = Arc::new(Node::new(NodeId(4), NodeKind::Image(Box::new(image))));
        assert!(!moves_alike(&Node::group(NodeId(1), vec![Arc::new(Node::group(NodeId(5), vec![image.clone()]))]), 1));
        assert!(!outlines_only(&image));
        // A text object's slot, and a member with an opacity mask.
        let mut slot = Node::group(NodeId(6), vec![]);
        slot.name = Some(format!("{}0", super::super::document::TEXT_SLOT));
        assert!(!moves_alike(&Node::group(NodeId(1), vec![solid(), Arc::new(slot)]), 1));
        let mut masked = Node::clone(&solid());
        masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(Node::group(NodeId(7), vec![]), true)));
        assert!(!moves_alike(&Node::group(NodeId(1), vec![solid(), Arc::new(masked)]), 1));
        // A member with an effect of its own.
        let mut turned = Node::clone(&solid());
        turned.appearance.effects.push(vectorcraft_doc::Effect {
            id: "distort.twist".into(),
            params: serde_json::json!({"angle": 30.0}),
            visible: true,
        });
        assert!(!moves_alike(&Node::group(NodeId(1), vec![solid(), Arc::new(turned)]), 1));
        // A member painting a pattern, whose tiles stay where they are, and a clipping group.
        let tiled = square(8, Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY });
        assert!(!moves_alike(&Node::group(NodeId(1), vec![solid(), tiled]), 1));
        let mut clipped = Node::group(NodeId(9), vec![solid(), solid()]);
        if let NodeKind::Group { clip, .. } = &mut clipped.kind {
            *clip = true;
        }
        assert!(!moves_alike(&Node::group(NodeId(1), vec![Arc::new(clipped)]), 1));
    }

    #[test]
    fn an_outline_moves_with_its_effect_only_on_outlines() {
        // GUARDS: a top-level image or masked path took an effect that moves the outline.
        use crate::import::ai::style::ArtStyle;
        let effect = |id: &str, params| vectorcraft_doc::Effect { id: id.into(), params, visible: true };
        let style = |e| ArtStyle { effects: vec![e], ..ArtStyle::default() };
        let twist = style(effect("distort.twist", serde_json::json!({"angle": 30.0})));
        let path = square(2, Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0)));
        assert!(moves_with(&path, &twist));
        let mut masked = Node::clone(&path);
        masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(Node::group(NodeId(7), vec![]), true)));
        assert!(!moves_with(&masked, &twist));
        let image =
            ImageObject { key: "a".into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        let image = Node::new(NodeId(4), NodeKind::Image(Box::new(image)));
        let distort = style(effect("distort.freeDistort", serde_json::json!({"corners": [[0.1, 0.0], [1.1, 0.0], [1.0, 1.0], [0.0, 1.0]]})));
        assert!(!moves_with(&image, &distort));
        assert!(moves_with(&path, &distort));
    }
}
