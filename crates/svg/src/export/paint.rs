use super::nodes::{MASK, doc_path};
use super::output::sanitize_id;
use super::*;

/// Image `b` as SVG viewers show it: PNG, JPEG, GIF, WebP and SVG as they are, other formats (a
/// CMYK TIFF) as PNG.
fn web_image(b: &ImageBlob) -> std::borrow::Cow<'_, ImageBlob> {
    use std::borrow::Cow;
    if matches!(b.mime.as_str(), "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" | "image/svg+xml") {
        return Cow::Borrowed(b);
    }
    let png = image::load_from_memory(&b.bytes).ok().and_then(|img| {
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
        Some(out)
    });
    png.map_or(Cow::Borrowed(b), |png| Cow::Owned(ImageBlob::png(png)))
}

/// The usual file extension of an image MIME type.
fn image_ext(mime: &str) -> &str {
    match mime {
        "image/jpeg" => "jpg",
        "image/svg+xml" => "svg",
        m => m.strip_prefix("image/").filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("bin"),
    }
}

/// How one stroke is written.
pub(super) enum StrokePlan {
    /// Its brush art (document space).
    Brush(Vec<Node>),
    Written(Written),
}

impl Writer<'_> {
    /// The `href` of image `im`: its file with [`ImageMode::Link`] (or when its pixels are
    /// missing), else its bytes. `None` when it has neither.
    pub(super) fn image_href(&mut self, im: &vectorcraft_doc::ImageObject) -> Option<String> {
        let (doc, link) = (self.doc, self.opts.images == ImageMode::Link);
        match (im.link.as_ref().filter(|_| link), doc.image_blob(im)) {
            (Some(l), _) => Some(l.path.clone()),
            (None, Some(b)) if !b.bytes.is_empty() => Some(self.blob_href(&web_image(b))),
            _ => im.link.as_ref().map(|l| l.path.clone()),
        }
    }

    /// Images drawn more than once in `art` (the pieces of a distorted image) go into the defs
    /// once; each piece then draws them with `<use>`.
    pub(super) fn share_images(&mut self, art: &Node) {
        // Each image's count and its first piece, which shows what they all show.
        let mut seen: HashMap<(String, u32, u32), (usize, vectorcraft_doc::ImageObject)> = HashMap::new();
        art.walk(&mut |c| {
            if let NodeKind::Image(im) = &c.kind {
                seen.entry((im.key.clone(), im.width, im.height)).or_insert_with(|| (0, (**im).clone())).0 += 1;
            }
        });
        let mut repeated: Vec<_> = seen.into_iter().filter(|(k, (n, _))| *n > 1 && !self.shared_images.contains_key(k)).collect();
        repeated.sort_by(|a, b| a.0.cmp(&b.0));
        for ((key, width, height), (_, im)) in repeated {
            let Some(href) = self.image_href(&im) else { continue };
            let id = self.fresh_id("image");
            self.def(
                0,
                &format!(
                    "<image id=\"{id}\" width=\"{width}\" height=\"{height}\" preserveAspectRatio=\"none\" xlink:href=\"{}\"/>",
                    xml_escape(&href)
                ),
            );
            self.shared_images.insert((key, width, height), id);
        }
    }

    /// A fill or stroke value. Gradients resolve unset geometry against `bounds` (the space
    /// [`Self::xf`] maps into user space, as the renderer does).
    pub(super) fn paint(&mut self, p: &Paint, bounds: Option<Rect>) -> String {
        match p {
            Paint::None => "none".into(),
            Paint::Pattern { pattern, xf } => match self.pattern_def(pattern, *xf) {
                Some(id) => format!("url(#{id})"),
                None => "none".into(),
            },
            Paint::Solid { color, .. } => color.to_hex(),
            Paint::Gradient(g) => format!("url(#{})", self.gradient_def(g, bounds.unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0)))),
        }
    }

    /// A `<pattern>` def for pattern `name` placed by `xf`: one period (super-tile) of the tiling,
    /// with every instance that reaches into it.
    fn pattern_def(&mut self, name: &str, xf: Affine) -> Option<String> {
        let doc = self.doc;
        let def = doc.pattern(name)?;
        let m = self.xf * xf;
        let key = format!("{name}|{:?}", m.as_coeffs());
        if let Some(id) = self.patterns.get(&key) {
            return Some(id.clone());
        }
        if self.pattern_nest > 4 {
            return None;
        }
        let id = self.fresh_id("pattern");
        self.patterns.insert(key, id.clone());
        let (pw, ph) = def.period();
        let (saved_body, saved_xf, saved_depth) = (std::mem::take(&mut self.body), self.xf, self.depth);
        self.depth = 3;
        self.pattern_nest += 1;
        for o in def.offsets_covering(Rect::new(0.0, 0.0, pw, ph)) {
            self.xf = def.instance_xf(o);
            for a in &def.art {
                self.node(a);
            }
        }
        self.pattern_nest -= 1;
        let content = std::mem::replace(&mut self.body, saved_body);
        self.xf = saved_xf;
        self.depth = saved_depth;
        let head = format!(
            "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"{}\" height=\"{}\" patternTransform=\"{}\">",
            self.num(pw),
            self.num(ph),
            self.matrix(m)
        );
        self.def(1, &head);
        self.defs.push_str(&content);
        self.def(1, "</pattern>");
        Some(id)
    }

    /// The id of a `<linearGradient>`/`<radialGradient>` def for `g` resolved against `bounds`.
    /// Identical gradients share one def; a gradient from a swatch takes the swatch's name as its
    /// id. A midpoint other than halfway is written as an extra stop marked `data-vc-midpoint`
    /// (what import turns back into a midpoint).
    fn gradient_def(&mut self, g: &GradientPaint, bounds: Rect) -> String {
        if g.gradient.kind == GradientKind::Freeform {
            // Object fills and strokes are images (see `raster_paint`); characters' paints aren't.
            self.warn("freeform gradients on characters are written as linear gradients");
        }
        let mut geom = g.resolve(bounds);
        geom.transform(self.xf, g.gradient.kind);
        let (s, e) = (geom.start, geom.end);
        // An off-centre focal point: `fx`/`fy`, in the gradient's own (unsquashed) space.
        let focal = |w: &Self, f: Point| format!(" fx=\"{}\" fy=\"{}\"", w.num(f.x), w.num(f.y));
        let (tag, prefix, attrs) = match g.gradient.kind {
            GradientKind::Radial => {
                let r = self.num((e - s).hypot());
                let attrs = if (geom.aspect - 1.0).abs() < 1e-9 {
                    let f = geom.focal.map(|f| focal(self, f)).unwrap_or_default();
                    format!(" cx=\"{}\" cy=\"{}\" r=\"{r}\"{f}", self.num(s.x), self.num(s.y))
                } else {
                    let v = e - s;
                    let m = Affine::translate(s.to_vec2()) * Affine::rotate(v.y.atan2(v.x)) * Affine::scale_non_uniform(1.0, geom.aspect);
                    let f = geom.focal.map(|f| focal(self, m.inverse() * f)).unwrap_or_default();
                    format!(" cx=\"0\" cy=\"0\" r=\"{r}\"{f} gradientTransform=\"{}\"", self.matrix(m))
                };
                ("radialGradient", "radial-gradient", attrs)
            }
            GradientKind::Linear | GradientKind::Freeform => (
                "linearGradient",
                "linear-gradient",
                format!(" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"", self.num(s.x), self.num(s.y), self.num(e.x), self.num(e.y)),
            ),
        };
        let stops: Vec<String> = g
            .gradient
            .expanded()
            .map(|(offset, c, o, mid)| {
                let op = if o < 1.0 { format!(" stop-opacity=\"{}\"", fmt_num(o as f64, 3)) } else { String::new() };
                let mid = mid.map(|m| format!(" data-vc-midpoint=\"{}\"", fmt_num(m as f64, 4))).unwrap_or_default();
                format!("<stop offset=\"{}\" stop-color=\"{}\"{op}{mid}/>", fmt_num(offset as f64, 4), c.to_hex())
            })
            .collect();
        let key = format!("{tag}{attrs}{}", stops.concat());
        if let Some(id) = self.gradients.get(&key) {
            return id.clone();
        }
        let id = match &g.swatch {
            Some(name) => self.unique_id(&sanitize_id(name)),
            None => self.fresh_id(prefix),
        };
        self.gradients.insert(key, id.clone());
        self.def(1, &format!("<{tag} id=\"{id}\"{attrs} gradientUnits=\"userSpaceOnUse\">"));
        for stop in &stops {
            self.def(2, stop);
        }
        self.def(1, &format!("</{tag}>"));
        id
    }

    pub(super) fn fill_props(&mut self, f: &FillLayer, rule: FillRule, bounds: Option<Rect>, p: &mut Props) {
        let paint = self.paint(&f.paint, bounds);
        p.push(("fill", paint));
        if f.opacity < 1.0 {
            p.push(("fill-opacity", fmt_num(f.opacity as f64, 3)));
        }
        if rule == FillRule::EvenOdd {
            p.push(("fill-rule", "evenodd".into()));
        }
    }

    pub(super) fn stroke_props(&mut self, s: &StrokeLayer, width: f64, bounds: Option<Rect>, p: &mut Props) {
        let paint = self.paint(&s.paint, bounds);
        p.push(("stroke", paint));
        if s.opacity < 1.0 {
            p.push(("stroke-opacity", fmt_num(s.opacity as f64, 3)));
        }
        if (width - 1.0).abs() > 1e-9 {
            p.push(("stroke-width", self.num(width)));
        }
        match s.cap {
            LineCap::Butt => {}
            LineCap::Round => p.push(("stroke-linecap", "round".into())),
            LineCap::Square => p.push(("stroke-linecap", "square".into())),
        }
        match s.join {
            LineJoin::Miter => {
                if (s.miter_limit - 4.0).abs() > 1e-9 {
                    p.push(("stroke-miterlimit", self.num(s.miter_limit)));
                }
            }
            LineJoin::Round => p.push(("stroke-linejoin", "round".into())),
            LineJoin::Bevel => p.push(("stroke-linejoin", "bevel".into())),
        }
        if let Some(d) = &s.dash
            && d.is_dashed()
        {
            p.push(("stroke-dasharray", d.pattern.iter().map(|v| self.num(*v)).collect::<Vec<_>>().join(" ")));
            if d.offset != 0.0 {
                p.push(("stroke-dashoffset", self.num(d.offset)));
            }
        }
    }

    /// How stroke `st` is written (`bp`: the shape in document space, built on first use from
    /// `paths`): its brush art, or a plain stroke or filled outlines matching the canvas.
    pub(super) fn stroke_plan(&mut self, st: &StrokeLayer, bp: &mut Option<BezPath>, paths: &[&PathData]) -> StrokePlan {
        if stroke::is_plain(st) {
            return StrokePlan::Written(stroke::for_writer(&BezPath::new(), st));
        }
        let bp = bp.get_or_insert_with(|| doc_path(paths));
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        match st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) {
            Some(b) => StrokePlan::Brush(vectorcraft_brush::stroke_pieces(doc, b, bp, st)),
            None => StrokePlan::Written(stroke::for_writer(bp, st)),
        }
    }

    /// The `clip-path` (inside) or `mask` (outside) attribute that keeps an aligned stroke on its
    /// side of the shape `d`; `reach` (document space) is what the stroke covers.
    pub(super) fn side_attr(&mut self, side: Option<StrokeAlign>, d: &str, rule: FillRule, reach: Rect) -> String {
        match side {
            None | Some(StrokeAlign::Center) => String::new(),
            Some(StrokeAlign::Inside) => {
                let cid = self.fresh_id("clip-path");
                let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                self.def(2, &format!("<path d=\"{d}\"{r}/>"));
                self.def(1, "</clipPath>");
                format!(" clip-path=\"url(#{cid})\"")
            }
            Some(StrokeAlign::Outside) if self.tiny() => {
                // No masks: clipped by everything around the shape instead.
                let cid = self.fresh_id("clip-path");
                let b = self.xf.transform_rect_bbox(reach).inflate(1.0, 1.0);
                let (x0, y0, x1, y1) = (self.num(b.x0), self.num(b.y0), self.num(b.x1), self.num(b.y1));
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                self.def(2, &format!("<path d=\"M{x0} {y0} H{x1} V{y1} H{x0} Z {d}\" clip-rule=\"evenodd\"/>"));
                self.def(1, "</clipPath>");
                format!(" clip-path=\"url(#{cid})\"")
            }
            Some(StrokeAlign::Outside) => {
                let mid = self.fresh_id("mask");
                let b = self.xf.transform_rect_bbox(reach).inflate(1.0, 1.0);
                let (x, y, w, h) = (self.num(b.x0), self.num(b.y0), self.num(b.width()), self.num(b.height()));
                let fr = if rule == FillRule::EvenOdd { " fill-rule=\"evenodd\"" } else { "" };
                self.def(1, &format!("<mask id=\"{mid}\" {MASK} x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\">"));
                self.def(2, &format!("<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"#fff\"/>"));
                self.def(2, &format!("<path d=\"{d}\" fill=\"#000\"{fr}/>"));
                self.def(1, "</mask>");
                format!(" mask=\"url(#{mid})\"")
            }
        }
    }

    /// The `href` of image bytes `b`: a `data:` URI, or with [`ImageMode::Link`] a file written
    /// next to the SVG, named after the bytes (image keys such as "raster-1" repeat across
    /// documents).
    pub(super) fn blob_href(&mut self, b: &ImageBlob) -> String {
        if self.opts.images != ImageMode::Link {
            return format!("data:{};base64,{}", b.mime, base64_encode(&b.bytes));
        }
        let name = format!("{}.{}", b.content_key(), image_ext(&b.mime));
        if self.linked_names.insert(name.clone()) {
            self.linked.push(LinkedImage { name: name.clone(), bytes: b.bytes.clone() });
        }
        name
    }

    /// A gradient `g` painted over `bounds` (the space [`Self::xf`] maps into user space) that is
    /// drawn as pixels ([`GradientPaint::draws_as_pixels`]) when it paints a fill (`fill`) or a
    /// stroke: an `<image>` over its painted box, clipped to `clip` (outlines in user space filled
    /// by `rule`), in a group with `props` and the `side` attribute. A freeform gradient's colour
    /// field, which SVG can't express, is sampled as the canvas samples it (at twice the box's size
    /// in points); a linear or radial gradient at the document's raster effects resolution (at most
    /// [`MAX_GRADIENT_PX`] a side), with dither's noise on a fill. Nothing is written for an empty
    /// box.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn raster_paint(&mut self, g: &GradientPaint, bounds: Rect, clip: &[String], rule: FillRule, props: &Props, side: &str, fill: bool) {
        use vectorcraft_color::freeform::{grid_size, painted_box, spread_scale};
        let Some(b) = painted_box(bounds) else { return };
        let freeform = g.gradient.kind == GradientKind::Freeform;
        let (rgba, cols, rows): (Vec<u8>, u16, u16) = if freeform {
            let (cols, rows) = grid_size(b, 2.0 * b.width().max(b.height()));
            let field = g.freeform_on(b).field(spread_scale(b));
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            (field.grid(b, cols, rows).flat_map(|([r, g, bl], a)| [q(r), q(g), q(bl), q(a)]).collect(), cols, rows)
        } else {
            let k = self.doc.raster_effects_ppi / 72.0;
            let along = |len: f64| (len * k).ceil().clamp(1.0, MAX_GRADIENT_PX) as u16;
            let (cols, rows) = (along(b.width()), along(b.height()));
            (g.pixels(b, b, cols, rows, fill, &Color::to_rgb).flatten().collect(), cols, rows)
        };
        let mut png = Vec::new();
        let Some(img) = image::RgbaImage::from_raw(u32::from(cols), u32::from(rows), rgba) else { return };
        if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            return;
        }
        self.warn(if freeform {
            "freeform gradients are written as images clipped to their shapes"
        } else {
            "linear and radial gradients drawn as pixels are written as images clipped to their shapes"
        });
        let href = self.blob_href(&ImageBlob::new("image/png", png));
        let cid = self.fresh_id("clip-path");
        let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
        self.def(1, &format!("<clipPath id=\"{cid}\">"));
        for d in clip {
            self.def(2, &format!("<path d=\"{d}\"{r}/>"));
        }
        self.def(1, "</clipPath>");
        let a = self.attrs(props);
        let nested = !side.is_empty();
        if nested {
            self.line(&format!("<g{a}{side}>"));
            self.depth += 1;
            self.line(&format!("<g clip-path=\"url(#{cid})\">"));
        } else {
            self.line(&format!("<g clip-path=\"url(#{cid})\"{a}>"));
        }
        self.depth += 1;
        let (x, y, w, h) = (self.num(b.x0), self.num(b.y0), self.num(b.width()), self.num(b.height()));
        let tr = if self.xf == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(self.xf)) };
        self.line(&format!(
            "<image x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"{tr} preserveAspectRatio=\"none\" xlink:href=\"{}\"/>",
            xml_escape(&href)
        ));
        self.depth -= 1;
        self.line("</g>");
        if nested {
            self.depth -= 1;
            self.line("</g>");
        }
    }

    /// Raster effects (drop shadow, glows, Gaussian blur, feather) as SVG filters: one `<g filter>`
    /// per effect, the first effect innermost (the reference app's stacking order). A drop shadow
    /// with Darkness darkens the screen colors, in a CMYK document too. Photoshop-style
    /// filters have no SVG equivalent: they are left out with a warning (exports from the app turn
    /// such objects into images first).
    pub(super) fn filtered(&mut self, n: &Node) {
        let mut inner = n.clone();
        inner.appearance.effects.retain(|e| !vectorcraft_effects::is_raster(&e.id));
        let opened = self.open_filters(n, &n.appearance.effects);
        self.node_body(&inner);
        self.close_filters(opened);
    }

    /// Open one `<g filter>` per visible raster effect of `effects` (an object's, or one fill or
    /// stroke's) on `n`, outermost last effect; returns how many to close.
    pub(super) fn open_filters(&mut self, n: &Node, effects: &[vectorcraft_doc::Effect]) -> usize {
        use vectorcraft_effects::RasterFx;
        let fx = vectorcraft_effects::raster_effects(effects);
        if self.tiny() {
            if !fx.is_empty() {
                self.warn("SVG Tiny 1.2 has no filters: shadows, glows, blurs and feathers are left out");
            }
            return 0;
        }
        let bounds = n.visual_bounds_with(&self.doc.symbol_boxes());
        // An Outer Glow's silhouette grows by a quarter of a pixel of the document's raster grid first.
        let grow = 18.0 / self.doc.raster_ppi();
        let reach: f64 =
            fx.iter().map(|f| f.outset(bounds.unwrap_or_default()) + if matches!(f, RasterFx::OuterGlow { .. }) { grow } else { 0.0 }).sum::<f64>()
                + 2.0;
        let region = bounds.map(|b| self.xf.transform_rect_bbox(b).inflate(reach, reach));
        let mut opened = 0;
        for f in fx.iter().rev() {
            if matches!(f, RasterFx::Pixel(_) | RasterFx::Rasterize(_)) {
                self.warn("SVG has no pixel effects such as Radial Blur, Smart Blur or Unsharp Mask: they are left out");
                continue;
            }
            let fid = self.fresh_id("filter");
            let region_attr = match region {
                Some(r) => format!(
                    " filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                    self.num(r.x0),
                    self.num(r.y0),
                    self.num(r.width()),
                    self.num(r.height())
                ),
                None => String::new(),
            };
            let sd = |blur: f64| fmt_num((blur / 2.0).max(0.0), 3);
            let flood = |c: &vectorcraft_color::Color, o: f32| {
                format!("<feFlood flood-color=\"{}\" flood-opacity=\"{}\"/>", c.to_hex(), fmt_num(o as f64, 3))
            };
            let body = match f {
                // Darkness: the object blurred and moved, each color channel times 1 − darkness.
                RasterFx::DropShadow { opacity, dx, dy, blur, darkness: Some(darkness), .. } => format!(
                    "<feGaussianBlur in=\"SourceGraphic\" stdDeviation=\"{}\"/><feOffset dx=\"{}\" dy=\"{}\"/><feComponentTransfer result=\"paint\"><feFuncR type=\"linear\" slope=\"{keep}\"/><feFuncG type=\"linear\" slope=\"{keep}\"/><feFuncB type=\"linear\" slope=\"{keep}\"/><feFuncA type=\"linear\" slope=\"{}\"/></feComponentTransfer><feMerge><feMergeNode in=\"paint\"/><feMergeNode in=\"SourceGraphic\"/></feMerge>",
                    sd(*blur),
                    self.num(*dx),
                    self.num(*dy),
                    fmt_num(*opacity as f64, 3),
                    keep = fmt_num(1.0 - *darkness as f64, 3)
                ),
                RasterFx::DropShadow { opacity, dx, dy, blur, color, .. } => format!(
                    "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"{}\"/><feOffset dx=\"{}\" dy=\"{}\" result=\"shadow\"/>{}<feComposite in2=\"shadow\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"paint\"/><feMergeNode in=\"SourceGraphic\"/></feMerge>",
                    sd(*blur),
                    self.num(*dx),
                    self.num(*dy),
                    flood(color, *opacity)
                ),
                // The glow blurs a solid silhouette (alpha × 255 makes any alpha full) grown by a quarter raster pixel.
                RasterFx::OuterGlow { opacity, blur, color, .. } => format!(
                    "<feComponentTransfer in=\"SourceAlpha\"><feFuncA type=\"linear\" slope=\"255\"/></feComponentTransfer><feMorphology operator=\"dilate\" radius=\"{}\"/><feGaussianBlur stdDeviation=\"{}\" result=\"glow\"/>{}<feComposite in2=\"glow\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"paint\"/><feMergeNode in=\"SourceGraphic\"/></feMerge>",
                    fmt_num(grow, 3),
                    sd(*blur),
                    flood(color, *opacity)
                ),
                RasterFx::InnerGlow { opacity, blur, color, center, .. } => {
                    // Edge: the blurred inverse silhouette; Center: the blurred silhouette; both clipped to the shape.
                    let src = if *center {
                        "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"SD\" result=\"glow\"/>".replace("SD", &sd(*blur))
                    } else {
                        format!(
                            "<feComponentTransfer in=\"SourceAlpha\"><feFuncA type=\"table\" tableValues=\"1 0\"/></feComponentTransfer><feGaussianBlur stdDeviation=\"{}\" result=\"glow\"/>",
                            sd(*blur)
                        )
                    };
                    format!(
                        "{src}{}<feComposite in2=\"glow\" operator=\"in\"/><feComposite in2=\"SourceAlpha\" operator=\"in\" result=\"paint\"/><feMerge><feMergeNode in=\"SourceGraphic\"/><feMergeNode in=\"paint\"/></feMerge>",
                        flood(color, *opacity)
                    )
                }
                RasterFx::Feather { radius } => format!(
                    "<feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"{}\" result=\"soft\"/><feComposite in=\"SourceGraphic\" in2=\"soft\" operator=\"in\" result=\"f\"/><feComposite in=\"f\" in2=\"SourceAlpha\" operator=\"in\"/>",
                    sd(*radius)
                ),
                RasterFx::GaussianBlur { radius } => format!("<feGaussianBlur in=\"SourceGraphic\" stdDeviation=\"{}\"/>", sd(*radius)),
                RasterFx::Pixel(_) | RasterFx::Rasterize(_) => continue,
            };
            // Filters composite normally: a shadow's or glow's other blend mode is recorded for import.
            let mode = match f {
                RasterFx::DropShadow { mode, .. } | RasterFx::OuterGlow { mode, .. } | RasterFx::InnerGlow { mode, .. }
                    if *mode != BlendMode::Normal =>
                {
                    format!(" {}=\"{}\"", crate::import::BLEND, blend_css(*mode))
                }
                _ => String::new(),
            };
            self.def(1, &format!("<filter id=\"{fid}\"{region_attr}{mode} color-interpolation-filters=\"sRGB\">{body}</filter>"));
            self.line(&format!("<g filter=\"url(#{fid})\">"));
            self.depth += 1;
            opened += 1;
        }
        opened
    }

    pub(super) fn close_filters(&mut self, opened: usize) {
        for _ in 0..opened {
            self.depth -= 1;
            self.line("</g>");
        }
    }
}

/// Most pixels along a side of a linear or radial gradient's image.
const MAX_GRADIENT_PX: f64 = 4096.0;

/// The gradient a fill (`fill`) or stroke paint is, if it is drawn as pixels
/// ([`GradientPaint::draws_as_pixels`]; SVG has no freeform gradients, for one).
pub(super) fn pixel_gradient(p: &Paint, fill: bool) -> Option<&GradientPaint> {
    match p {
        Paint::Gradient(g) if g.draws_as_pixels(fill) => Some(g),
        _ => None,
    }
}

/// Any visible raster effect (shadow, glow, blur, feather) in `effects`?
pub(super) fn has_raster(effects: &[vectorcraft_doc::Effect]) -> bool {
    effects.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id))
}
