use super::containers::close;
use super::document::TEXT_SLOT;
use super::types::{
    FLIP, Frame, IMAGE_TOO_LARGE, Kind, LINKED_PLACEHOLDERS, LINKED_PREVIEWS, MAX_PIXELS, MAX_PLACED_ELSEWHERE, MAX_POINTS, MAX_SIDE, NON_NATIVE_ART,
    NON_NATIVE_ART_UNREAD, Place,
};
use super::{
    Affine, AnchorKind, Appearance, AppearanceItem, Arc, BezPath, CANVAS_CENTRE, FillLayer, ImageBlob, ImageObject, Inks, Instance, LinkInfo, Node,
    NodeId, NodeKind, Obj, OpacityMask, Paint, PathData, PathEl, Point, Reader, Rect, StrokeLayer, V, ps,
};
use vectorcraft_doc::links::missing_placeholder;

impl<'a> Reader<'a> {
    /// A text object (`/AI11Text`): an empty group standing for it, named after its story.
    pub(super) fn text_slot(&mut self, o: &Obj) -> Result<(), String> {
        if self.pattern.is_some() {
            self.unreadable("type in a pattern", false);
        }
        self.left_out_of_symbol();
        // `1 /FreeUndo`: type in a drawn look. Its story is one of another text document
        // (`/AI11UndoFreeTextDocument`), which counts its stories on its own, so its slot has none.
        // In the look of a text object that has an appearance, the object itself follows the look,
        // on `%_` lines, and is the type (`style_marker` takes the slot out); in the look of a layer
        // or a group it is the type of a copy that the look draws.
        let look = o.nums("FreeUndo").first() == Some(&1.0);
        let index = |key: &str| o.nums(key).first().copied().filter(|v| (0.0..f64::from(u32::MAX)).contains(v)).map(|v| v as u32);
        let (story, frame) = (index("StoryIndex").filter(|_| !look), index("FrameIndex").unwrap_or(0));
        let gs = self.gs.clone();
        let mut n = self.new_node(NodeKind::Group { children: vec![], clip: false }, &gs);
        n.name = Some(match story {
            _ if look => super::document::LOOK_SLOT.to_string(),
            Some(s) => format!("{TEXT_SLOT}{s}:{frame}"),
            None => TEXT_SLOT.to_string(),
        });
        // The type takes the state's transparency, as a path does; an `Xy` after it sets the slot's own.
        n.opacity = gs.opacity;
        n.blend = gs.blend;
        n.knockout_shape = gs.knockout_shape;
        // An `Xy` before the text object was the object's, not its layer's, when the layer's `LB` follows.
        self.layer_state = None;
        let hidden = self.obj_hidden;
        self.add(n, hidden)
    }

    /// Non-native art (`/ForeignObject`): the content of a placed PDF, which Illustrator shows but
    /// doesn't edit. Its `/Data` is that PDF (ASCII85 in comment lines, kept by the `,` handler),
    /// `/Bounds` the art's box in a space measured from `/Origin` with y down, and `/RTransform`
    /// maps that space onto the art (both read off files: the box lands on the clipping paths
    /// around it). The PDF importer reads the PDF, and its art is fitted to the box, as one
    /// group. Without data, or with a PDF that can't be read, nothing is drawn.
    pub(super) fn foreign_art(&mut self, o: &Obj) -> Result<(), String> {
        let Some(text) = self.foreign_data.take() else { return Ok(()) };
        // Where a symbol's art puts it isn't known.
        if self.symbol.is_some() {
            self.left_out_of_symbol();
            return Ok(());
        }
        // One `%` starts each line; the rest, `_` included, is data.
        let mut encoded = String::with_capacity(text.len());
        for line in text.split(|b| matches!(b, b'\r' | b'\n')) {
            encoded.push_str(&String::from_utf8_lossy(line.strip_prefix(b"%").unwrap_or(line)));
        }
        let Some(pdf) = ps::ascii85_decode(&encoded) else {
            self.warn(NON_NATIVE_ART_UNREAD);
            return Ok(());
        };
        let (rt, origin, bounds) = (o.get("RTransform").map(Self::nums), o.get("Origin").map(Self::nums), o.get("Bounds").map(Self::nums));
        let (Some([a, b, c, d, tx, ty]), Some([ox, oy]), Some([x0, y0, x1, y1])) = (rt.as_deref(), origin.as_deref(), bounds.as_deref()) else {
            self.warn(NON_NATIVE_ART_UNREAD);
            return Ok(());
        };
        let place = Affine::new([*a, *b, *c, *d, *tx, *ty]) * Affine::new([1.0, 0.0, 0.0, -1.0, *ox, *oy]);
        let art_box = place.transform_rect_bbox(Rect::new(*x0, *y0, *x1, *y1));
        if !sane(art_box) || art_box.width() <= 0.0 || art_box.height() <= 0.0 {
            self.warn(NON_NATIVE_ART_UNREAD);
            return Ok(());
        }
        let target = self.space().transform_rect_bbox(art_box);
        let opts = vectorcraft_pdf::ImportOptions { layers: false, ..Default::default() };
        let read = match vectorcraft_pdf::import_with_report(&pdf, &opts) {
            Ok(r) => r,
            Err(e) => {
                self.warn(&format!("{NON_NATIVE_ART_UNREAD} ({e})"));
                return Ok(());
            }
        };
        let mut art: Vec<Node> = read.document.layers.iter().filter_map(|l| l.children()).flatten().map(|n| n.as_ref().clone()).collect();
        let Some(bbox) = art.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.geometric_bounds())) else { return Ok(()) };
        if bbox.width() <= 0.0 || bbox.height() <= 0.0 {
            return Ok(());
        }
        // The art's own box onto the object's box (its measure of the same art). The PDF importer
        // hands the art back as the page shows it, and it keeps that orientation unless its
        // placement mirrors it (a negative determinant): the usual `/RTransform` `1 0 0 -1 …` with
        // the origin's flip doesn't, and the art then shows as the file's own PDF part draws it.
        let fit = if place.determinant() < 0.0 {
            Affine::translate((target.x0, target.y1)) * Affine::scale_non_uniform(target.width() / bbox.width(), -target.height() / bbox.height())
        } else {
            Affine::translate((target.x0, target.y0)) * Affine::scale_non_uniform(target.width() / bbox.width(), target.height() / bbox.height())
        } * Affine::translate((-bbox.x0, -bbox.y0));
        for n in &mut art {
            n.transform(fit, true);
            // The PDF's swatches stay with it: its colours keep their values, unlinked.
            self.reid(n);
        }
        for (key, blob) in read.document.images {
            self.doc.images.entry(key).or_insert(blob);
        }
        let gs = self.gs.clone();
        let mut group = self.new_node(NodeKind::Group { children: art.into_iter().map(Arc::new).collect(), clip: false }, &gs);
        group.name = Some(NON_NATIVE_ART.into());
        self.add(group, self.obj_hidden)
    }

    // ---- paths -----------------------------------------------------------------------------

    pub(super) fn path_op(&mut self, w: &str, vals: &[V]) -> Result<(), String> {
        let n = Self::nums(vals);
        let p = |i: usize| -> Option<Point> {
            let at = n.len().checked_sub(i)?;
            Some(Point::new(*n.get(at)?, *n.get(at + 1)?))
        };
        self.points += 1;
        if self.points > MAX_POINTS {
            return Err("a path has too many points".into());
        }
        let current = self.path.elements().last().and_then(|e| e.end_point());
        let kind = if w.starts_with(|c: char| c.is_ascii_lowercase()) { AnchorKind::Smooth } else { AnchorKind::Corner };
        let added = match w {
            "m" => {
                let Some(a) = p(2) else { return Ok(()) };
                self.path.move_to(a);
                return Ok(());
            }
            _ if current.is_none() => false,
            "l" | "L" => match p(2) {
                Some(a) => {
                    self.path.line_to(a);
                    true
                }
                None => false,
            },
            "c" | "C" => match (p(6), p(4), p(2)) {
                (Some(a), Some(b), Some(c)) => {
                    self.path.curve_to(a, b, c);
                    true
                }
                _ => false,
            },
            "v" | "V" => match (current, p(4), p(2)) {
                (Some(cur), Some(b), Some(c)) => {
                    self.path.curve_to(cur, b, c);
                    true
                }
                _ => false,
            },
            _ => match (p(4), p(2)) {
                (Some(a), Some(c)) => {
                    self.path.curve_to(a, c, c);
                    true
                }
                _ => false,
            },
        };
        if added {
            self.path_kinds.push(kind);
        }
        Ok(())
    }

    /// A path left unpainted before a container opens.
    pub(crate) fn end_path_quietly(&mut self) {
        self.path = BezPath::new();
        self.path_kinds.clear();
        self.points = 0;
        self.clip_next = false;
        if let Some(head) = self.symbol.as_mut() {
            (head.missing_fill, head.missing_stroke) = (false, false);
        }
    }

    /// Paint the current path (`op`), as a guide when `guide`.
    pub(super) fn paint(&mut self, op: &str, hidden: bool, guide: bool) -> Result<(), String> {
        let mut bp = std::mem::take(&mut self.path);
        let kinds = std::mem::take(&mut self.path_kinds);
        self.points = 0;
        let clip = std::mem::take(&mut self.clip_next);
        if let Some(i) = self.instance.take() {
            self.set_instance(&i);
        }
        let (fill_paint, stroke_paint) = (self.pending_fill.take(), self.pending_stroke.take());
        let missing = self.symbol.as_mut().map_or((false, false), |h| (std::mem::take(&mut h.missing_fill), std::mem::take(&mut h.missing_stroke)));
        if bp.elements().is_empty() {
            return Ok(());
        }
        if op.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            close(&mut bp);
        }
        let line = match bp.elements() {
            [PathEl::MoveTo(a), PathEl::LineTo(b)] => Some((*a, *b)),
            _ => None,
        };
        let to_doc = self.space();
        let path = path_data(&(to_doc * bp), &kinds);
        let gs = self.gs.clone();
        let (fill, stroke) = match op.to_ascii_lowercase().as_str() {
            "f" => (true, false),
            "s" => (false, true),
            "b" => (true, true),
            _ => (false, false),
        };
        // A symbol's art painted with a gradient the file doesn't define is left out.
        if (missing.0 && fill) || (missing.1 && stroke) {
            self.left_out_of_symbol();
        }
        // A gradient instance's paint replaces the missing pattern.
        let unread = (fill && self.pending_unread_fill && fill_paint.is_none()) || (stroke && self.pending_unread_stroke && stroke_paint.is_none());
        let mut items = Vec::new();
        if fill && !clip {
            items.push(AppearanceItem::Fill(FillLayer {
                overprint: gs.overprint_fill,
                ..FillLayer::new(fill_paint.clone().unwrap_or(gs.fill.clone()))
            }));
        }
        // A clipping path keeps its stroke, which paints over the art it clips.
        if stroke {
            let mut st = StrokeLayer::new(stroke_paint.unwrap_or(gs.stroke.clone()), gs.line_width());
            st.cap = gs.cap;
            st.join = gs.join;
            st.miter_limit = gs.miter;
            st.dash = gs.dash.clone();
            st.overprint = gs.overprint_stroke;
            items.push(AppearanceItem::Stroke(st));
        }
        let kind = NodeKind::Path { path, rule: gs.rule, live: None, clipping: clip, guide };
        let mut node = self.new_node(kind, &gs);
        // A ruler guide is written as a straight `(N) *` guide path on its layer.
        if guide
            && op == "N"
            && !hidden
            && !gs.hidden
            && !clip
            && self.pattern.is_none()
            && matches!(self.frames.last(), Some(Frame { kind: Kind::Layer(_), .. }))
            && let Some((a, b)) = line
            && [a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite())
        {
            self.ruler_guides.insert(node.id, (a, b));
        }
        node.appearance = Appearance { items, ..Appearance::default() }.into();
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        node.knockout_shape = gs.knockout_shape;
        if clip && let Some(f) = self.frames.last_mut() {
            f.clips.push(f.children.len());
        }
        // A clipping path's fill is painted too, under the art it clips, as the file's PDF part draws it.
        let painted = clip && fill && !matches!(gs.fill, Paint::None);
        let copy = painted.then(|| {
            let mut copy = node.clone();
            copy.id = self.doc.alloc_id();
            if let NodeKind::Path { clipping, .. } = &mut copy.kind {
                *clipping = false;
            }
            let paint = fill_paint.clone().unwrap_or_else(|| gs.fill.clone());
            copy.appearance = Appearance {
                items: vec![AppearanceItem::Fill(FillLayer { overprint: gs.overprint_fill, ..FillLayer::new(paint) })],
                ..Appearance::default()
            }
            .into();
            copy
        });
        // The note and Show Center are the path's, not its painted copy's.
        node.edit_attrs(|a| {
            a.note = gs.note.clone().unwrap_or_default();
            a.show_center = gs.show_center;
        });
        let unread_bounds = unread.then(|| node.visual_bounds()).flatten();
        let unread_id = node.id;
        self.add(node, hidden)?;
        if unread && self.pattern.is_some() {
            // Art in a pattern's definition painted with a pattern it doesn't have: the tile can't be read.
            self.unsupported.insert("pattern fills".into());
        } else if unread && ((hidden && !self.in_mask()) || gs.hidden || !self.shown()) {
            self.hidden_unread.insert("pattern fills".into());
        } else if unread && self.in_mask() {
            self.unreadable("pattern fills", false);
        } else if unread {
            match unread_bounds {
                Some(b) if sane(b) && b.area() > 0.0 => {
                    if !self.mark_unread("pattern fills".into()) {
                        self.unread.push((unread_id, vec!["pattern fills".into()]));
                    }
                }
                _ => {
                    self.unsupported.insert("pattern fills".into());
                }
            }
        }
        if let Some(copy) = copy {
            self.clip_fills.insert(copy.id);
            self.add(copy, hidden)?;
        }
        Ok(())
    }

    /// `/End X!`: the gradient mesh read, as one object.
    pub(super) fn end_mesh(&mut self) -> Result<(), String> {
        let Some((m, hidden)) = self.mesh.take() else { return Ok(()) };
        if m.unread || m.patches.is_empty() {
            self.unreadable("gradient meshes it can't read", hidden && self.pattern.is_none());
            return Ok(());
        }
        let gs = self.gs.clone();
        // One mesh where the patches join up into the grid they say they make; else each patch is
        // a mesh of its own, in a group.
        match m.one_mesh() {
            Ok(one) => {
                let mut node = self.new_node(NodeKind::Mesh(one), &gs);
                node.opacity = gs.opacity;
                node.blend = gs.blend;
                node.knockout_shape = gs.knockout_shape;
                self.add(node, hidden)?;
                self.mesh_ended = true;
                return Ok(());
            }
            Err(crate::import::ai::mesh::NotOne::Unjoined) => {
                self.warn("a gradient mesh whose patches don't join up comes in as a group of its patches")
            }
            Err(crate::import::ai::mesh::NotOne::Unplaced) => {}
        }
        let mut patches: Vec<Node> = m.patches.into_iter().map(|p| self.new_node(NodeKind::Mesh(p), &gs)).collect();
        let mut node = match patches.len() {
            1 => patches.pop().unwrap_or_else(|| self.new_node(NodeKind::Group { children: vec![], clip: false }, &gs)),
            _ => self.new_node(NodeKind::Group { children: patches.into_iter().map(Arc::new).collect(), clip: false }, &gs),
        };
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        node.knockout_shape = gs.knockout_shape;
        self.add(node, hidden)?;
        self.mesh_ended = true;
        Ok(())
    }

    /// Stroke the path just painted (a gradient-filled one, `1 BB` or `2 BB`) with the stroke's state. A clipping path
    /// takes the stroke itself, not the painted copy of its fill.
    pub(super) fn stroke_last(&mut self) {
        let gs = self.gs.clone();
        if !self.after_object {
            return;
        }
        let Some(kids) = self.frames.last_mut().map(|f| &mut f.children) else { return };
        let Some(mut at) = kids.len().checked_sub(1) else { return };
        if kids.get(at).is_some_and(|n| self.clip_fills.contains(&n.id)) && at > 0 {
            at -= 1;
        }
        let Some(n) = kids.get_mut(at) else { return };
        if !matches!(n.kind, NodeKind::Path { .. }) || n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(_))) {
            return;
        }
        let mut st = StrokeLayer::new(gs.stroke.clone(), gs.line_width());
        st.cap = gs.cap;
        st.join = gs.join;
        st.miter_limit = gs.miter;
        st.dash = gs.dash.clone();
        st.overprint = gs.overprint_stroke;
        let mut a = (*n.appearance).clone();
        a.items.push(AppearanceItem::Stroke(st));
        n.appearance = a.into();
    }

    pub(super) fn set_instance(&mut self, i: &Instance) {
        let mut to_doc = self.space();
        // In a pattern's definition a gradient's `Xm` and `Bm` are written y down about the canvas point of art space's
        // origin, as the file's PDF part draws them; in a symbol's, as the instances in its art are.
        if (self.pattern.is_some() || self.symbol.is_some())
            && i.has_matrix()
            && let Some(t) = self.template
        {
            let (cx, cy) = ((t[0] + t[2]) / 2.0, (t[1] + t[3]) / 2.0);
            to_doc *= Affine::new([1.0, 0.0, 0.0, -1.0, CANVAS_CENTRE - cx, CANVAS_CENTRE + cy]);
        }
        let paint = i.paint(&self.gradients, &self.global_names, to_doc);
        if let Some(head) = self.symbol.as_mut() {
            let missing = if i.stroke { &mut head.missing_stroke } else { &mut head.missing_fill };
            *missing = paint.is_none();
        }
        match paint {
            Some(p) if i.stroke => self.pending_stroke = Some(p),
            Some(p) => self.pending_fill = Some(p),
            None => {
                let name = i.name.clone().unwrap_or_default();
                self.warn(&format!("gradient “{name}” isn't defined in the file: a plain colour stands in for it"));
            }
        }
    }

    // ---- images ----------------------------------------------------------------------------

    /// The samples after `XI`, its operands on the stack.
    pub(super) fn image(&mut self, data: &[u8]) -> Result<(), String> {
        let vals = self.operands();
        // Operands that came before an `XI` on their own line.
        let vals = match self.raster.as_mut().and_then(|r| r.operands.take()) {
            Some(before) if !vals.iter().any(|v| matches!(v, V::Arr(_))) => before,
            _ => vals,
        };
        self.image_from(vals, data)
    }

    /// The matrix and the numbers of an image's operands (`[a b c d tx ty] llx lly urx ury w h bits
    /// ImageType AlphaChannelCount reserved bin-ascii ImageMask`, as `XI` and `XF` take them); `None`
    /// without a matrix or with fewer numbers than up to the encoding (bin-ascii).
    fn image_operands(vals: &[V]) -> Option<(Vec<f64>, Vec<f64>)> {
        let m = vals.iter().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None })?;
        let n = Self::nums(vals);
        (n.len() >= 11).then_some((m, n))
    }

    /// `XG` (`w`, which names the file of the image after it) or `XF` (that image), with their
    /// operands `vals`. Before the art, the setup's procedures run: there they do nothing.
    pub(super) fn image_link_op(&mut self, w: &str, vals: Vec<V>, hidden: bool) -> Result<(), String> {
        if self.frames.is_empty() {
            return Ok(());
        }
        if w == "XG" {
            self.image_link = Self::image_link(&vals);
            return Ok(());
        }
        self.linked_image(vals, hidden)
    }

    /// The file `XG`'s operands (`(path) modified`) name; `None` for operands of another shape or
    /// an empty path.
    fn image_link(vals: &[V]) -> Option<String> {
        match vals {
            [.., V::Str(path), V::Num(_)] if !path.is_empty() => Some(super::obj::text(path)),
            _ => None,
        }
    }

    /// An image of operands `vals` and samples `data`.
    pub(super) fn image_from(&mut self, vals: Vec<V>, data: &[u8]) -> Result<(), String> {
        // A file `XG` named before this image isn't the file of a later `XF`.
        self.image_link = None;
        let space = self.raster.as_ref().and_then(|r| r.space.clone()).unwrap_or_default();
        let Some((m, n)) = Self::image_operands(&vals) else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let [_, _, _, _, w, h, bits, kind, alpha, _, binary, rest @ ..] = n.as_slice() else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        // ImageMask 1: the image is transparent and takes a color (version 7 specification §8.1).
        let image_mask = rest.first() == Some(&1.0);
        let (w, h) = (*w as u64, *h as u64);
        if w.saturating_mul(h) > MAX_PIXELS {
            self.image_left_out(IMAGE_TOO_LARGE);
            return Ok(());
        }
        if w == 0 || h == 0 || ![1.0, 8.0].contains(bits) {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let channels: usize = match space.as_str() {
            "DeviceGray" => 1,
            "DeviceCMYK" => 4,
            "DeviceRGB" => 3,
            _ => match *kind as i64 {
                1 => 1,
                4 => 4,
                _ => 3,
            },
        };
        let decoded;
        let data = if *binary == 0.0 {
            let digits: Vec<u8> = data.iter().copied().filter(|b| b.is_ascii_hexdigit()).collect();
            decoded = crate::import::lex::hex_decode(&digits).unwrap_or_default();
            decoded.as_slice()
        } else {
            data
        };
        let row = if *bits == 1.0 { (w as usize * channels).div_ceil(8) } else { w as usize * channels };
        let need = row.saturating_mul(h as usize);
        // An alpha channel follows the colour samples, one byte a pixel.
        let alpha_len = if *alpha > 0.0 { (w * h) as usize } else { 0 };
        let (Some(samples), opacity) = (data.get(..need), data.get(need..need.saturating_add(alpha_len))) else {
            self.image_left_out("an image whose data is cut short was left out");
            return Ok(());
        };
        let opacity = opacity.filter(|_| alpha_len > 0);
        // CMYK samples keep their ink amounts as a CMYK TIFF, as the PDF importer keeps a CMYK
        // image's, and an alpha channel that hides part of the image becomes its opacity mask.
        if channels == 4
            && *bits == 8.0
            && let Some(blob) = Inks::new(w as u32, h as u32, samples.to_vec()).and_then(|i| ImageBlob::cmyk_tiff(&i))
        {
            return self.add_raster(blob, opacity, &m, (w as u32, h as u32), None);
        }
        if image_mask && channels == 1 && *bits == 1.0 {
            return self.colorized_raster(samples, row, opacity, &m, (w as u32, h as u32));
        }
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let line = samples.get(y * row..(y + 1) * row).unwrap_or_default();
            for x in 0..w as usize {
                // A 1-bit row holds each pixel's channels in turn, a bit each, padded to a whole
                // byte (`row`); a set bit is the channel at full strength (255).
                let s = |c: usize| {
                    let i = x * channels + c;
                    if *bits == 1.0 { line.get(i / 8).map_or(0, |b| ((b >> (7 - i % 8)) & 1) * 255) } else { line.get(i).copied().unwrap_or(0) }
                };
                let px = match channels {
                    1 => [s(0); 3],
                    4 => {
                        let key = [s(0), s(1), s(2), s(3)];
                        if self.cmyk_cache.len() > 1 << 16 {
                            self.cmyk_cache.clear();
                        }
                        *self.cmyk_cache.entry(key).or_insert_with(|| cmyk_rgb(key))
                    }
                    _ => [s(0), s(1), s(2)],
                };
                let a = opacity.and_then(|o| o.get(y * w as usize + x)).copied().unwrap_or(255);
                rgba.extend_from_slice(&[px[0], px[1], px[2], a]);
            }
        }
        let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let mut png = vec![];
        if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        }
        self.add_raster(ImageBlob::new("image/png", png), None, &m, (w as u32, h as u32), None)
    }

    /// A 1-bit gray image whose ImageMask is 1 (`samples`, `row` bytes a row): a grayscale image
    /// colorized with the fill color. Its 0 bits, which open black with ImageMask 0, take that color,
    /// and its 1 bits are transparent. A fill that isn't a solid color colorizes it black.
    fn colorized_raster(&mut self, samples: &[u8], row: usize, alpha: Option<&[u8]>, m: &[f64], (w, h): (u32, u32)) -> Result<(), String> {
        let mut gray = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h as usize {
            let line = samples.get(y * row..(y + 1) * row).unwrap_or_default();
            for x in 0..w as usize {
                let v = line.get(x / 8).map_or(0, |b| ((b >> (7 - x % 8)) & 1) * 255);
                gray.push(v);
            }
        }
        let mut png = vec![];
        let written = image::GrayImage::from_raw(w, h, gray).map(|img| img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png));
        if !matches!(written, Some(Ok(()))) {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let paint = self.mask_paint();
        self.add_raster(ImageBlob::new("image/png", png), alpha, m, (w, h), Some(paint))
    }

    /// The color that colorizes an image mask: the fill color, or black when the fill isn't a
    /// solid color.
    fn mask_paint(&self) -> Paint {
        match &self.gs.fill {
            p @ Paint::Solid { .. } => p.clone(),
            _ => Paint::solid(vectorcraft_color::Color::BLACK),
        }
    }

    /// `XF` (§8.2): an image whose pixels are in the file the `XG` before it names becomes a linked
    /// image of that file, of its `w` × `h` pixels placed as the matrix maps them, as `XI` places an
    /// image. The data keeps no pixels or preview of it, so a placeholder is spread over those pixels
    /// until its file is found. A 1-bit image mask is colorized as `XI` colorizes one. One without
    /// that file name, or whose operands `XI` couldn't read (no
    /// matrix, too few numbers, no pixels, or samples of other than 1 or 8 bits), is left out with a
    /// warning. No pixels are read, so its size isn't held to the limit on `XI`'s samples.
    fn linked_image(&mut self, vals: Vec<V>, hidden: bool) -> Result<(), String> {
        let path = self.image_link.take();
        let operands = Self::image_operands(&vals);
        let (Some(path), Some((m, n))) = (path, operands) else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let (&[a, b, c, d, tx, ty], &[_, _, _, _, w, h, bits, ..]) = (m.as_slice(), n.as_slice()) else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        if !(1.0..=MAX_SIDE).contains(&w) || !(1.0..=MAX_SIDE).contains(&h) || ![1.0, 8.0].contains(&bits) {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let (w, h) = (w as u32, h as u32);
        let Some(to_doc) = self.image_space() else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let xf = to_doc * Affine::new([a, b, c, d, tx, ty]) * FLIP;
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let Some(png) = missing_placeholder() else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        // The placeholder is all the document has of the file: what a missing link shows.
        let bytes = Arc::new(png);
        let blob = ImageBlob { mime: "image/png".into(), bytes: bytes.clone(), proxy: Some(bytes) };
        let key = linked_key(&blob, &path);
        self.doc.images.entry(key.clone()).or_insert(blob);
        self.warn(LINKED_PLACEHOLDERS);
        // A PostScript page doesn't draw the placeholder: there its pixels are left out of the
        // comparisons with the page.
        if !hidden && !self.gs.hidden && self.shown() {
            self.placeholders.push(xf.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(w), f64::from(h))));
        }
        let im = ImageObject { key, width: w, height: h, xf, link: Some(LinkInfo::new(path)), placement: Default::default(), pixels: None };
        // A 1-bit image mask (ImageMask 1) keeps the color that colorizes it once its file is relinked.
        let image_mask = bits == 1.0 && n.get(7) == Some(&1.0) && n.get(11) == Some(&1.0);
        let colorize = image_mask.then(|| self.mask_paint());
        self.add_image_colorized(im, None, colorize, hidden)
    }

    /// Add the image `blob` (`w` × `h` pixels), which the matrix `m` maps into art space, with
    /// `alpha` (its alpha channel, one byte a pixel) as its opacity mask when it hides part of the
    /// image, and `colorize` as the color that colorizes it.
    fn add_raster(&mut self, blob: ImageBlob, alpha: Option<&[u8]>, m: &[f64], (w, h): (u32, u32), colorize: Option<Paint>) -> Result<(), String> {
        // The matrix is an art-space matrix (y up) applied to the pixels with their rows running
        // downwards, (u, -v): pixels → (u, -v) → art space. Flipping the matrix's result instead
        // gives the same place only when the image isn't rotated (b = c = 0).
        let [a, b, c, d, tx, ty] = m else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let Some(to_doc) = self.image_space() else {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let xf = to_doc * Affine::new([*a, *b, *c, *d, *tx, *ty]) * FLIP;
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            self.image_left_out("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let mask = alpha.and_then(|a| self.alpha_mask(a, w, h, xf));
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        let im = ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None };
        self.add_image_colorized(im, mask, colorize, false)
    }

    /// An image that isn't read is left out, with `warning`; in a symbol's art, the symbol isn't
    /// read whole, so its instances come in as the page draws them.
    fn image_left_out(&mut self, warning: &str) {
        if self.symbol.is_some() {
            self.left_out_of_symbol();
        } else {
            self.warn(warning);
        }
    }

    /// Note art of a symbol's definition that is left out: the symbol isn't kept, and its
    /// instances come in as the page draws them, within the box of the art that is read.
    pub(super) fn left_out_of_symbol(&mut self) {
        if let Some(head) = self.symbol.as_mut() {
            head.left_out = true;
        }
    }

    /// An image's alpha channel (`w` × `h` bytes) as a grayscale image over the image (`xf`): its
    /// opacity mask, as in PDF import. `None` when it hides nothing.
    fn alpha_mask(&mut self, alpha: &[u8], w: u32, h: u32, xf: Affine) -> Option<OpacityMask> {
        if alpha.iter().all(|v| *v == 255) {
            return None;
        }
        let mut png = Vec::new();
        image::GrayImage::from_raw(w, h, alpha.to_vec())?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        let art = Node::new(
            self.doc.alloc_id(),
            NodeKind::Image(Box::new(ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None })),
        );
        Some(OpacityMask::new(art, true))
    }

    /// Add an image object with the state's transparency.
    fn add_image(&mut self, im: ImageObject, hidden: bool) -> Result<(), String> {
        self.add_image_masked(im, None, hidden)
    }

    /// Add an image object with the state's transparency and `mask` as its opacity mask.
    fn add_image_masked(&mut self, im: ImageObject, mask: Option<OpacityMask>, hidden: bool) -> Result<(), String> {
        self.add_image_colorized(im, mask, None, hidden)
    }

    /// Add an image object with the state's transparency, `mask` as its opacity mask and
    /// `colorize` as the color that colorizes it.
    fn add_image_colorized(&mut self, im: ImageObject, mask: Option<OpacityMask>, colorize: Option<Paint>, hidden: bool) -> Result<(), String> {
        let gs = self.gs.clone();
        let mut node = self.new_node(NodeKind::Image(Box::new(im)), &gs);
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        node.knockout_shape = gs.knockout_shape;
        node.extras_mut().mask = mask.map(Box::new);
        node.extras_mut().colorize = colorize.map(Box::new);
        self.add(node, hidden)?;
        self.image_tail = self.after_object;
        Ok(())
    }

    /// `~`: the placed file read since `%AI5_BeginPlace` becomes a linked image in its place that
    /// shows the preview the data keeps of it, as a linked image whose file is missing does. One
    /// without a preview, a path or a matrix isn't read.
    pub(super) fn end_place(&mut self, hidden: bool) -> Result<(), String> {
        // Where a symbol's art puts a placed file's pixels isn't known.
        if self.symbol.is_some() {
            self.place = None;
            self.left_out_of_symbol();
            return Ok(());
        }
        let (m, b, path, png, w, h, folder) = match self.place.take() {
            Some(Place { broken: true, .. }) => {
                self.unreadable("placed files", hidden);
                return Ok(());
            }
            Some(Place { matrix: Some(m), bounds: Some(b), path, preview: Some((png, w, h)), folder, .. }) if !path.is_empty() => {
                (m, b, path, png, w, h, folder)
            }
            other => {
                let boxed = other.as_ref().and_then(|p| Some((p.matrix?, p.bounds?))).filter(|(_, b)| sane(*b) && b.area() > 0.0);
                let mut have_bounds = false;
                if self.shown()
                    && !self.gs.hidden
                    && !hidden
                    && let Some((m, b)) = boxed
                {
                    let centre = self.template.map(|t| ((t[0] + t[2]) / 2.0, (t[1] + t[3]) / 2.0));
                    let origin = centre.map_or(b.origin(), |(x, y)| Point::new(CANVAS_CENTRE - x, CANVAS_CENTRE + y));
                    let box_at_canvas = Rect::new(origin.x, origin.y, origin.x + b.width(), origin.y + b.height());
                    let bounds = (self.space() * m).transform_rect_bbox(box_at_canvas);
                    have_bounds = sane(bounds) && bounds.area() > 0.0;
                    if have_bounds {
                        let placeholder_id = self.unread_box("placed files", bounds, hidden)?;
                        if let Some(placeholder_id) = placeholder_id.filter(|_| self.unread_elsewhere.len() < MAX_PLACED_ELSEWHERE) {
                            let corner = (self.space() * m).transform_rect_bbox(b);
                            if sane(corner) && corner.area() > 0.0 && corner != bounds {
                                self.unread_elsewhere.push((placeholder_id, corner));
                            }
                        }
                        return Ok(());
                    }
                }
                if self.shown() && !self.gs.hidden && !hidden && !have_bounds {
                    self.unsupported.insert("placed files".into());
                } else {
                    self.unreadable("placed files", hidden);
                }
                return Ok(());
            }
        };
        // The file's pixels (y down) start at the canvas point of art space's origin (the box's own
        // corner when the file doesn't say where the canvas is); the preview's are spread over them.
        let centre = self.template.map(|t| ((t[0] + t[2]) / 2.0, (t[1] + t[3]) / 2.0));
        let origin = centre.map_or(b.origin(), |(x, y)| Point::new(CANVAS_CENTRE - x, CANVAS_CENTRE + y));
        let (sx, sy) = (b.width() / f64::from(w), b.height() / f64::from(h));
        let over_file = Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(sx, sy);
        // Many files put the pixels at the box's own corner instead: that placement is kept for
        // when the layers don't draw as the page does with this one (a move in the preview's
        // pixels, which later moves of the image keep).
        let to_corner = b.origin() - origin;
        let elsewhere = (to_corner.hypot() > 0.01 && sx.abs() > 1e-12 && sy.abs() > 1e-12)
            .then(|| Affine::translate((to_corner.x / sx, to_corner.y / sy)))
            .filter(|a| a.as_coeffs().iter().all(|v| v.is_finite()));
        let xf = self.space() * m * over_file;
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            self.unreadable("placed files", hidden);
            return Ok(());
        }
        // The preview is all the document has of the file: what a missing link shows.
        let bytes = Arc::new(png);
        let blob = ImageBlob { mime: "image/png".into(), bytes: bytes.clone(), proxy: Some(bytes) };
        let key = linked_key(&blob, &path);
        self.doc.images.entry(key.clone()).or_insert(blob);
        self.warn(LINKED_PREVIEWS);
        if let Some(shift) = elsewhere
            && self.placed_elsewhere.len() < MAX_PLACED_ELSEWHERE
        {
            self.placed_elsewhere.push((NodeId(self.doc.peek_next_id()), shift));
        }
        let mut link = LinkInfo::new(path);
        link.relative = folder.and_then(|f| relative_path(&f, link.name()));
        self.add_image(ImageObject { key, width: w, height: h, xf, link: Some(link), placement: Default::default(), pixels: None }, hidden)
    }

    pub(super) fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) && self.warnings.len() < 64 {
            self.warnings.push(w.to_string());
        }
    }
}

/// `bp` as a path whose anchors take the kinds in `kinds`, one for each segment of `bp` in order,
/// for the anchor the segment ends on. A closed subpath's last segment ends on its first anchor
/// when it returns to the start point, and that anchor takes its kind. Anchors no segment ends on
/// (a subpath's start) keep the kind [`PathData::from_bezpath`] gives them, and so does a subpath
/// whose anchors don't match its segments.
pub(super) fn path_data(bp: &BezPath, kinds: &[AnchorKind]) -> PathData {
    let mut path = PathData::from_bezpath(bp);
    // Each subpath's anchors as `from_bezpath` starts them: one for its start, one for each segment.
    let mut subpaths: Vec<Vec<Option<AnchorKind>>> = Vec::new();
    let mut kinds = kinds.iter().copied();
    let mut open = false;
    for el in bp.elements() {
        match el {
            PathEl::MoveTo(_) => {
                subpaths.push(vec![None]);
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {
                let kind = kinds.next();
                match subpaths.last_mut() {
                    Some(s) if open => s.push(kind),
                    _ => {
                        subpaths.push(vec![None, kind]);
                        open = true;
                    }
                }
            }
        }
    }
    if subpaths.len() != path.subpaths.len() {
        return path;
    }
    for (sp, mut set) in path.subpaths.iter_mut().zip(subpaths) {
        // The closing anchor merged into the first: its kind goes to the first.
        if sp.closed
            && set.len() == sp.anchors.len() + 1
            && let Some(last) = set.pop()
            && let Some(first) = set.first_mut()
        {
            *first = last;
        }
        if set.len() != sp.anchors.len() {
            continue;
        }
        for (a, kind) in sp.anchors.iter_mut().zip(set) {
            if let Some(kind) = kind {
                a.kind = kind;
            }
        }
    }
    path
}

/// The name of the XML node `o` (its `xmlnode-nodename`) without a namespace prefix (`v:`).
pub(super) fn element_name(o: &Obj) -> Option<String> {
    let name = o.text("xmlnode-nodename")?;
    Some(name.rsplit(':').next().unwrap_or(&name).to_string())
}

/// A placed file's preview as a PNG, with its size: `w h rowBytes 32 n ` (each number followed by
/// a space), then `h` rows of `rowBytes` bytes, premultiplied ARGB. `None` when it isn't that, is
/// larger than [`MAX_PIXELS`] or is cut short.
pub(super) fn preview_png(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    // A placed file's copy on `%_` lines has its header on one too: `%_512 256 2048 32 17 `.
    let data = data.trim_ascii_start();
    let mut rest = data.strip_prefix(b"%_").unwrap_or(data);
    let mut head = [0usize; 5];
    for v in &mut head {
        let n = rest.iter().position(|b| *b == b' ')?;
        *v = std::str::from_utf8(rest.get(..n)?).ok()?.parse().ok()?;
        rest = rest.get(n + 1..)?;
    }
    let [w, h, row, bits, _] = head;
    let (width, height) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    if w == 0 || h == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS || bits != 32 || row < w * 4 {
        return None;
    }
    // Every row is there before anything is allocated for them.
    if rest.len() < (h - 1).checked_mul(row)?.checked_add(w * 4)? {
        return None;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for line in rest.chunks(row).take(h) {
        for &[a, r, g, b] in line.get(..w * 4)?.as_chunks::<4>().0 {
            let straight = |c: u8| if a == 0 { 0 } else { (u32::from(c) * 255 / u32::from(a)).min(255) as u8 };
            rgba.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
        }
    }
    let mut png = vec![];
    image::RgbaImage::from_raw(width, height, rgba)?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some((png, width, height))
}

/// A placed file's path relative to the document's folder (`/` separators): its relative `folder`
/// (`src/`, `..\art`) followed by its file `name`; the name alone when the folder is empty (the
/// document's own folder). `None` when the folder is absolute or holds control characters, or
/// when there is no name.
pub(super) fn relative_path(folder: &str, name: &str) -> Option<String> {
    let folder = folder.replace('\\', "/");
    let absolute = folder.starts_with('/') || folder.as_bytes().get(1) == Some(&b':') || folder.contains("://");
    if absolute || name.is_empty() || folder.chars().chain(name.chars()).any(char::is_control) {
        return None;
    }
    if folder.is_empty() {
        return Some(name.to_string());
    }
    Some(format!("{}/{name}", folder.trim_end_matches('/')))
}

/// The image key of a linked image of the file at `path` that shows `blob` (its preview or a
/// placeholder): the blob's own key and the path's. Images of other files that show the same pixels
/// have keys of their own, and reading one file's pixels in changes that file's images only.
fn linked_key(blob: &ImageBlob, path: &str) -> String {
    format!("{}-{}", blob.content_key(), vectorcraft_doc::links::hash_bytes(path.as_bytes()))
}

pub(super) fn sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_SIDE)
}

/// The four numbers of a comment's value (a box that may be a point).
pub(super) fn four(v: &str) -> Option<[f64; 4]> {
    let n: Vec<f64> = v.split_whitespace().map_while(|w| w.parse::<f64>().ok()).collect();
    let [a, b, c, d] = n.as_slice() else { return None };
    [a, b, c, d].iter().all(|v| v.is_finite() && v.abs() <= MAX_SIDE).then_some([*a, *b, *c, *d])
}

/// The first two numbers of a comment's value.
pub(super) fn two(v: &str) -> Option<(f64, f64)> {
    let n: Vec<f64> = v.split_whitespace().filter_map(|w| w.parse().ok()).filter(|v: &f64| v.is_finite()).collect();
    match n.as_slice() {
        [a, b, ..] => Some((*a, *b)),
        _ => None,
    }
}

/// A CMYK sample in RGB.
fn cmyk_rgb([c, m, y, k]: [u8; 4]) -> [u8; 3] {
    let f = |v: u8| f32::from(v) / 255.0;
    let rgb = vectorcraft_color::Color::cmyk(f(c), f(m), f(y), f(k)).to_rgba8(1.0);
    [rgb[0], rgb[1], rgb[2]]
}

/// `a`, `a and b`, `a, b and c`.
pub(crate) fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
