use super::containers::close;
use super::document::TEXT_SLOT;
use super::types::{
    FLIP, IMAGE_TOO_LARGE, LINKED_PREVIEWS, MAX_PIXELS, MAX_PLACED_ELSEWHERE, MAX_POINTS, MAX_SIDE, NON_NATIVE_ART, NON_NATIVE_ART_UNREAD, Place,
};
use super::{
    Affine, Appearance, AppearanceItem, Arc, BezPath, CANVAS_CENTRE, FillLayer, ImageBlob, ImageObject, Inks, Instance, LinkInfo, Node, NodeId,
    NodeKind, Obj, OpacityMask, Paint, PathData, Point, Reader, Rect, StrokeLayer, V, ps,
};

impl<'a> Reader<'a> {
    /// A text object (`/AI11Text`): an empty group standing for it, named after its story.
    pub(super) fn text_slot(&mut self, o: &Obj) -> Result<(), String> {
        if self.pattern.is_some() {
            self.unreadable("type in a pattern");
        }
        // `1 /FreeUndo`: the type in the drawn look of a text object that has an appearance. Its
        // story is one of another text document (`/AI11UndoFreeTextDocument`), which counts its
        // stories on its own; the object itself follows the look, on `%_` lines, and is the type.
        if o.nums("FreeUndo").first() == Some(&1.0) {
            return Ok(());
        }
        let index = |key: &str| o.nums(key).first().copied().filter(|v| (0.0..f64::from(u32::MAX)).contains(v)).map(|v| v as u32);
        let (story, frame) = (index("StoryIndex"), index("FrameIndex").unwrap_or(0));
        let gs = self.gs.clone();
        let mut n = self.new_node(NodeKind::Group { children: vec![], clip: false }, &gs);
        n.name = Some(story.map_or_else(|| TEXT_SLOT.to_string(), |s| format!("{TEXT_SLOT}{s}:{frame}")));
        // The type takes the state's transparency, as a path does; an `Xy` after it sets the slot's own.
        n.opacity = gs.opacity;
        n.blend = gs.blend;
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
        match w {
            "m" => {
                let Some(a) = p(2) else { return Ok(()) };
                self.path.move_to(a);
            }
            _ if current.is_none() => {}
            "l" | "L" => {
                if let Some(a) = p(2) {
                    self.path.line_to(a);
                }
            }
            "c" | "C" => {
                if let (Some(a), Some(b), Some(c)) = (p(6), p(4), p(2)) {
                    self.path.curve_to(a, b, c);
                }
            }
            "v" | "V" => {
                if let (Some(cur), Some(b), Some(c)) = (current, p(4), p(2)) {
                    self.path.curve_to(cur, b, c);
                }
            }
            _ => {
                if let (Some(a), Some(c)) = (p(4), p(2)) {
                    self.path.curve_to(a, c, c);
                }
            }
        }
        Ok(())
    }

    /// A path left unpainted before a container opens.
    pub(crate) fn end_path_quietly(&mut self) {
        self.path = BezPath::new();
        self.points = 0;
        self.clip_next = false;
    }

    /// Paint the current path (`op`), as a guide when `guide`.
    pub(super) fn paint(&mut self, op: &str, hidden: bool, guide: bool) -> Result<(), String> {
        let mut bp = std::mem::take(&mut self.path);
        self.points = 0;
        let clip = std::mem::take(&mut self.clip_next);
        if let Some(i) = self.instance.take() {
            self.set_instance(&i);
        }
        let (fill_paint, stroke_paint) = (self.pending_fill.take(), self.pending_stroke.take());
        if bp.elements().is_empty() {
            return Ok(());
        }
        if op.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            close(&mut bp);
        }
        let to_doc = self.space();
        let path = PathData::from_bezpath(&(to_doc * bp));
        let gs = self.gs.clone();
        let (fill, stroke) = match op.to_ascii_lowercase().as_str() {
            "f" => (true, false),
            "s" => (false, true),
            "b" => (true, true),
            _ => (false, false),
        };
        let mut items = Vec::new();
        if fill && !clip {
            items.push(AppearanceItem::Fill(FillLayer {
                overprint: gs.overprint_fill,
                ..FillLayer::new(fill_paint.clone().unwrap_or(gs.fill.clone()))
            }));
        }
        if stroke && !clip {
            let mut st = StrokeLayer::new(stroke_paint.unwrap_or(gs.stroke.clone()), gs.width);
            st.cap = gs.cap;
            st.join = gs.join;
            st.miter_limit = gs.miter;
            st.dash = gs.dash.clone();
            st.overprint = gs.overprint_stroke;
            items.push(AppearanceItem::Stroke(st));
        }
        let kind = NodeKind::Path { path, rule: gs.rule, live: None, clipping: clip, guide };
        let mut node = self.new_node(kind, &gs);
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
        self.add(node, hidden)?;
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
            self.unreadable("gradient meshes it can't read");
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
        self.add(node, hidden)?;
        self.mesh_ended = true;
        Ok(())
    }

    /// Stroke the path just painted (a gradient-filled one, `1 BB` or `2 BB`) with the stroke's state.
    pub(super) fn stroke_last(&mut self) {
        let gs = self.gs.clone();
        let Some(n) = self.last_object() else { return };
        if !matches!(n.kind, NodeKind::Path { clipping: false, .. }) || n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(_))) {
            return;
        }
        let mut st = StrokeLayer::new(gs.stroke.clone(), gs.width);
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
        // origin, as the file's PDF part draws them.
        if self.pattern.is_some()
            && i.has_matrix()
            && let Some(t) = self.template
        {
            let (cx, cy) = ((t[0] + t[2]) / 2.0, (t[1] + t[3]) / 2.0);
            to_doc *= Affine::new([1.0, 0.0, 0.0, -1.0, CANVAS_CENTRE - cx, CANVAS_CENTRE + cy]);
        }
        match i.paint(&self.gradients, to_doc) {
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

    /// An image of operands `vals` and samples `data`.
    pub(super) fn image_from(&mut self, vals: Vec<V>, data: &[u8]) -> Result<(), String> {
        let space = self.raster.as_ref().and_then(|r| r.space.clone()).unwrap_or_default();
        let Some(m) = vals.iter().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None }) else {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let n = Self::nums(&vals);
        let [_, _, _, _, w, h, bits, kind, alpha, _, binary, ..] = n.as_slice() else {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let (w, h) = (*w as u64, *h as u64);
        if w.saturating_mul(h) > MAX_PIXELS {
            self.warn(IMAGE_TOO_LARGE);
            return Ok(());
        }
        if w == 0 || h == 0 || ![1.0, 8.0].contains(bits) {
            self.warn("an image VectorCraft can't read was left out");
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
            self.warn("an image whose data is cut short was left out");
            return Ok(());
        };
        let opacity = opacity.filter(|_| alpha_len > 0);
        // CMYK samples keep their ink amounts as a CMYK TIFF, as the PDF importer keeps a CMYK
        // image's, and an alpha channel that hides part of the image becomes its opacity mask.
        if channels == 4
            && *bits == 8.0
            && let Some(blob) = Inks::new(w as u32, h as u32, samples.to_vec()).and_then(|i| ImageBlob::cmyk_tiff(&i))
        {
            return self.add_raster(blob, opacity, &m, (w as u32, h as u32));
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
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let mut png = vec![];
        if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        }
        self.add_raster(ImageBlob::new("image/png", png), None, &m, (w as u32, h as u32))
    }

    /// Add the image `blob` (`w` × `h` pixels), which the matrix `m` maps into art space, with
    /// `alpha` (its alpha channel, one byte a pixel) as its opacity mask when it hides part of the
    /// image.
    fn add_raster(&mut self, blob: ImageBlob, alpha: Option<&[u8]>, m: &[f64], (w, h): (u32, u32)) -> Result<(), String> {
        // The matrix is an art-space matrix (y up) applied to the pixels with their rows running
        // downwards, (u, -v): pixels → (u, -v) → art space. Flipping the matrix's result instead
        // gives the same place only when the image isn't rotated (b = c = 0).
        let [a, b, c, d, tx, ty] = m else {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let to_doc = self.space();
        let xf = to_doc * Affine::new([*a, *b, *c, *d, *tx, *ty]) * FLIP;
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let mask = alpha.and_then(|a| self.alpha_mask(a, w, h, xf));
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        self.add_image_masked(ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None }, mask, false)
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
        let gs = self.gs.clone();
        let mut node = self.new_node(NodeKind::Image(Box::new(im)), &gs);
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        node.extras_mut().mask = mask.map(Box::new);
        self.add(node, hidden)
    }

    /// `~`: the placed file read since `%AI5_BeginPlace` becomes a linked image in its place that
    /// shows the preview the data keeps of it, as a linked image whose file is missing does. One
    /// without a preview, a path or a matrix isn't read.
    pub(super) fn end_place(&mut self, hidden: bool) -> Result<(), String> {
        let Some(Place { matrix: Some(m), bounds: Some(b), path, preview: Some((png, w, h)) }) = self.place.take().filter(|p| !p.path.is_empty())
        else {
            self.unreadable("placed files");
            return Ok(());
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
            self.unreadable("placed files");
            return Ok(());
        }
        // The preview is all the document has of the file: what a missing link shows.
        let bytes = Arc::new(png);
        let blob = ImageBlob { mime: "image/png".into(), bytes: bytes.clone(), proxy: Some(bytes) };
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        self.warn(LINKED_PREVIEWS);
        if let Some(shift) = elsewhere
            && self.placed_elsewhere.len() < MAX_PLACED_ELSEWHERE
        {
            self.placed_elsewhere.push((NodeId(self.doc.peek_next_id()), shift));
        }
        self.add_image(
            ImageObject { key, width: w, height: h, xf, link: Some(LinkInfo::new(path)), placement: Default::default(), pixels: None },
            hidden,
        )
    }

    pub(super) fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) && self.warnings.len() < 64 {
            self.warnings.push(w.to_string());
        }
    }
}

/// The name of the XML node `o` (its `xmlnode-nodename`) without a namespace prefix (`v:`).
pub(super) fn element_name(o: &Obj) -> Option<String> {
    let name = o.text("xmlnode-nodename")?;
    Some(name.rsplit(':').next().unwrap_or(&name).to_string())
}

/// The child nodes named `name` (see [`element_name`]) of the XML node `o`.
pub(super) fn elements<'o>(o: &'o Obj, name: &'o str) -> impl Iterator<Item = &'o Obj> {
    o.obj("xmlnode-children").into_iter().flat_map(Obj::items).filter(move |c| c.ty == "XMLNode" && element_name(c).as_deref() == Some(name))
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
pub(super) fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
