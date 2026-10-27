use super::layout::png;
use super::nodes::affine;
use super::text::Placement;
use super::*;

impl Builder {
    /// Drop shadows, inner shadows and outer glows as VectorCraft's Stylize effects. Affinity's
    /// blur radius sets the same Gaussian as VectorCraft's blur (σ = half of it), and its angle
    /// is the direction the shadow goes in.
    pub(super) fn effects(&mut self, effects: &[af::Effect]) -> Vec<Effect> {
        let k = self.k;
        let len = move |v: f64| if v.is_finite() { (v * k).clamp(0.0, 1000.0) } else { 0.0 };
        let num = |v: f64| (v * 1000.0).round() / 1000.0;
        let mut out = Vec::new();
        for e in effects {
            let (id, params) = match e {
                af::Effect::DropShadow { color, opacity, radius, distance, angle } => {
                    let (color, alpha) = Self::color(*color);
                    let d = len(*distance);
                    let a = if angle.is_finite() { *angle } else { 0.0 };
                    let p = json!({
                        "mode": "normal",
                        "opacity": num(opacity * f64::from(alpha) * 100.0),
                        "x": num(d * a.cos()),
                        "y": num(d * a.sin()),
                        "blur": num(len(*radius)),
                        "color": color,
                    });
                    ("stylize.dropShadow", p)
                }
                af::Effect::InnerShadow { color, opacity, radius, distance, .. } => {
                    self.warn("inner shadows (imported as inner glows at the edge, without their offset)");
                    let (color, alpha) = Self::color(*color);
                    let p = json!({
                        "mode": "normal",
                        "opacity": num(opacity * f64::from(alpha) * 100.0),
                        "blur": num(len(*radius) + len(*distance)),
                        "color": color,
                        "source": "edge",
                    });
                    ("stylize.innerGlow", p)
                }
                af::Effect::OuterGlow { color, opacity, radius } => {
                    let (color, alpha) = Self::color(*color);
                    let p =
                        json!({ "mode": "normal", "opacity": num(opacity * f64::from(alpha) * 100.0), "blur": num(len(*radius)), "color": color });
                    ("stylize.outerGlow", p)
                }
                af::Effect::Bevel { .. } => {
                    self.warn("bevel and emboss effects (left out)");
                    continue;
                }
                // Outlines become strokes (`outline_stroke`, `outline_text`), overlays are baked
                // into the paint (`overlaid`).
                af::Effect::Outline { .. } | af::Effect::ColorOverlay { .. } | af::Effect::GradientOverlay { .. } => continue,
            };
            out.push(Effect { id: id.into(), params, visible: true });
        }
        out
    }

    /// An outline effect on a shape: a stroke layer. Affinity outlines what the shape shows, its own
    /// strokes included: an outline outside goes under them (from the path out to as far past them as
    /// it is wide), one centred on or inside that edge over them. A centred one reaches as far into
    /// the shape as out of it here, which only differs by the width of the shape's own stroke.
    pub(super) fn outline_stroke(&mut self, effects: &[af::Effect], appearance: &mut Appearance) {
        // How far the shape's own strokes reach outside its path.
        let reach = appearance
            .items
            .iter()
            .filter_map(|i| match i {
                AppearanceItem::Stroke(s) => Some(match s.align {
                    StrokeAlign::Center => s.width / 2.0,
                    StrokeAlign::Outside => s.width,
                    StrokeAlign::Inside => 0.0,
                }),
                _ => None,
            })
            .filter(|w| w.is_finite())
            .fold(0.0, f64::max);
        for e in effects {
            let af::Effect::Outline { color, opacity, width, align } = e else { continue };
            let w = width * self.k;
            if !(w.is_finite() && w > 0.0) {
                continue;
            }
            let (c, alpha) = Self::color(*color);
            let (align, w) = match align {
                1 => (StrokeAlign::Inside, w),
                2 => (StrokeAlign::Center, w + 2.0 * reach),
                _ => (StrokeAlign::Outside, w + reach),
            };
            let mut l = StrokeLayer::new(Paint::solid(c), w.min(1000.0));
            l.opacity = (opacity.clamp(0.0, 1.0) as f32) * alpha;
            l.join = LineJoin::Round;
            l.align = align;
            let item = AppearanceItem::Stroke(l);
            if align == StrokeAlign::Outside {
                // Under the shape's strokes, over its fills.
                let at = appearance.items.iter().position(|i| matches!(i, AppearanceItem::Stroke(_))).unwrap_or(appearance.items.len());
                appearance.items.insert(at, item);
            } else {
                appearance.items.push(item);
            }
        }
    }

    /// An outline effect on text: the characters' stroke, where they have none of their own.
    pub(super) fn outline_text(&mut self, effects: &[af::Effect], text: &mut TextObject) {
        for e in effects {
            let af::Effect::Outline { color, opacity, width, align } = e else { continue };
            let w = width * self.k;
            if !(w.is_finite() && w > 0.0) {
                continue;
            }
            let (c, alpha) = Self::color(*color);
            if opacity * f64::from(alpha) < 0.999 {
                self.warn("semi-transparent outline effects on text (imported opaque)");
            }
            if *align != 2 {
                self.warn("outline effects on text inside or outside the characters (centred on them)");
            }
            for r in &mut text.runs {
                if r.style.stroke == Paint::None {
                    r.style.stroke = Paint::solid(c);
                    // Centred on the outline, so twice as wide to show as far outside.
                    r.style.stroke_width = if *align == 2 { w } else { 2.0 * w }.min(1000.0);
                }
            }
        }
    }

    pub(super) fn path(&self, p: &af::Path, to_doc: Affine) -> PathData {
        let mut bp = BezPath::new();
        for s in &p.subpaths {
            let pt = |q: af::Point| Point::new(q.x, q.y);
            bp.move_to(pt(s.start));
            for [c1, c2, e] in &s.segments {
                if c1 == c2 && c2 == e {
                    bp.line_to(pt(*e));
                } else {
                    bp.curve_to(pt(*c1), pt(*c2), pt(*e));
                }
            }
            if s.closed {
                bp.close_path();
            }
        }
        PathData::from_bezpath(&(to_doc * bp))
    }

    fn color(c: ap::Color) -> (Color, f32) {
        let f = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 };
        match c {
            ap::Color::Rgb { r, g, b, a } => (Color::Rgb { r: f(r), g: f(g), b: f(b) }, f(a)),
            ap::Color::Cmyk { c, m, y, k, a } => (Color::Cmyk { c: f(c), m: f(m), y: f(y), k: f(k) }, f(a)),
            ap::Color::Gray { v, a } => (Color::Gray { k: 1.0 - f(v) }, f(a)),
            ap::Color::Lab { l, a, b, alpha } => (Color::Lab { l: l as f32, a: a as f32, b: b as f32 }, f(alpha)),
        }
    }

    /// A paint and the opacity its colour's alpha becomes.
    pub(super) fn paint(&mut self, p: &ap::Paint, to_doc: Affine) -> Option<(Paint, f32)> {
        match p {
            ap::Paint::None => None,
            ap::Paint::Solid(c) => {
                let (color, a) = Self::color(*c);
                Some((Paint::solid(color), a))
            }
            ap::Paint::Gradient(g) => {
                let stops = g
                    .stops
                    .iter()
                    .map(|s| {
                        let (color, opacity) = Self::color(s.color);
                        let mut stop = GradientStop::new(s.offset as f32, color);
                        stop.opacity = opacity;
                        stop.midpoint = s.half_point().clamp(0.13, 0.87) as f32;
                        stop
                    })
                    .collect();
                let m = to_doc * affine(g.transform);
                let (o, x, y) = (m * Point::ZERO, m * Point::new(1.0, 0.0), m * Point::new(0.0, 1.0));
                let kind = match g.kind {
                    AfKind::Linear => GradientKind::Linear,
                    AfKind::Radial => GradientKind::Radial,
                    AfKind::Conical => {
                        self.warn("conical gradients import as radial gradients");
                        GradientKind::Radial
                    }
                };
                let (u, v): (Vec2, Vec2) = (x - o, y - o);
                if kind == GradientKind::Radial && u.hypot() > 0.0 && (u.dot(v) / (u.hypot() * v.hypot().max(1e-12))).abs() > 0.01 {
                    self.warn("skewed elliptical gradients import as upright ellipses");
                }
                let aspect = if u.hypot() > 0.0 { (v.hypot() / u.hypot()).clamp(0.01, 100.0) } else { 1.0 };
                let geom = GradientGeom { start: o, end: x, aspect: if kind == GradientKind::Radial { aspect } else { 1.0 }, focal: None };
                let mut gp = GradientPaint::new(Gradient::new(kind, stops));
                gp.angle = (-(u.y)).atan2(u.x).to_degrees();
                gp.geom = Some(geom);
                Some((Paint::Gradient(Box::new(gp)), 1.0))
            }
        }
    }

    pub(super) fn stroke_layer(&mut self, s: &ap::Stroke, to_doc: Affine) -> Option<StrokeLayer> {
        let (paint, opacity) = self.paint(&s.paint, to_doc)?;
        let mut l = StrokeLayer::new(paint, s.width * self.k);
        l.opacity = opacity;
        l.cap = match s.cap {
            Cap::Butt => LineCap::Butt,
            Cap::Round => LineCap::Round,
            Cap::Square => LineCap::Square,
        };
        l.join = match s.join {
            Join::Miter => LineJoin::Miter,
            Join::Round => LineJoin::Round,
            Join::Bevel => LineJoin::Bevel,
        };
        l.miter_limit = s.miter_limit;
        l.align = match s.align {
            ap::Align::Center => StrokeAlign::Center,
            ap::Align::Inside => StrokeAlign::Inside,
            ap::Align::Outside => StrokeAlign::Outside,
        };
        l.dash = s.dash.as_ref().map(|(pattern, phase)| Dash {
            pattern: pattern.iter().map(|v| v * self.k).collect(),
            offset: phase * self.k,
            align_corners: false,
        });
        Some(l)
    }

    pub(super) fn appearance(&mut self, fills: &[ap::Paint], strokes: &[ap::Stroke], to_doc: Affine) -> Appearance {
        let mut items = Vec::new();
        let mut above = Vec::new();
        for f in fills {
            if let Some((paint, opacity)) = self.paint(f, to_doc) {
                let mut l = FillLayer::new(paint);
                l.opacity = opacity;
                items.push(AppearanceItem::Fill(l));
            }
        }
        for s in strokes {
            let Some(l) = self.stroke_layer(s, to_doc) else { continue };
            if s.behind {
                items.insert(0, AppearanceItem::Stroke(l));
            } else {
                above.push(AppearanceItem::Stroke(l));
            }
        }
        items.extend(above);
        Appearance { items, ..Appearance::default() }
    }

    /// Where text placed by `m` (node space to points) goes, its type scaled by `type_m`.
    pub(super) fn placement(&mut self, m: Affine, type_m: Affine) -> Option<Placement> {
        let [a, b, c, d, e, f] = m.as_coeffs();
        let sx = a.hypot(b);
        let det = a * d - b * c;
        if !(sx > 1e-9 && det.is_finite() && det.abs() > 1e-12 && e.is_finite() && f.is_finite()) {
            self.warn("text with an invalid transform (left out)");
            return None;
        }
        let sy = det / sx;
        let shear = (a * c + b * d) / (sx * sx);
        if shear.abs() > 1e-3 || sy < 0.0 {
            self.warn("skewed or flipped text (imported without the skew or flip)");
        }
        // The type's vertical scale gives the font size, its horizontal scale relative to that the
        // characters' width.
        let [ta, tb, tc, td, ..] = type_m.as_coeffs();
        let (tsx, tdet) = (ta.hypot(tb), ta * td - tb * tc);
        let (size_scale, h_scale) = if tsx > 1e-9 && tdet.is_finite() && tdet.abs() > 1e-12 {
            let tsy = (tdet / tsx).abs();
            (tsy, tsx / tsy)
        } else {
            self.warn("text with an invalid text scale (imported unscaled)");
            (det.abs().sqrt(), 1.0)
        };
        Some(Placement {
            to_frame: Affine::scale_non_uniform(sx, sy.abs()),
            place: Affine::translate((e, f)) * Affine::rotate(b.atan2(a)),
            size_scale,
            h_scale,
        })
    }

    /// A CMYK pixel layer as an image file. Through the document's own CMYK profile when it
    /// embeds one (as Affinity shows it): RGBA. Otherwise, opaque, a CMYK image that keeps its
    /// inks (shown and separated through the working CMYK profile, as placed CMYK images are);
    /// with transparency, RGBA through the working CMYK profile.
    fn cmyk_image(&mut self, width: u32, height: u32, cmyka: &[u8]) -> Option<Vec<u8>> {
        let n = (width as usize).checked_mul(height as usize)?;
        let px = cmyka.get(..n.checked_mul(5)?)?.as_chunks::<5>().0;
        if let Some(lut) = self.document_cmyk() {
            let rgba = px
                .iter()
                .flat_map(|p| {
                    let [r, g, b, _] = lut.apply8([p[0], p[1], p[2], p[3]]);
                    [r, g, b, p[4]]
                })
                .collect();
            return png(width, height, rgba);
        }
        if px.iter().all(|p| p[4] == 255) {
            let inks = px.iter().flat_map(|p| [p[0], p[1], p[2], p[3]]).collect();
            if let Some(blob) = vectorcraft_doc::cmyk::Inks::new(width, height, inks).and_then(|i| vectorcraft_doc::ImageBlob::cmyk_tiff(&i)) {
                return Some(Arc::unwrap_or_clone(blob.bytes));
            }
        }
        self.warn("CMYK pixel layers with transparency (converted to RGB through the working CMYK profile)");
        let lut = self
            .working_cmyk
            .get_or_insert_with(|| {
                Arc::new(vectorcraft_color::cms::CmykLut::build(|[c, m, y, k]| {
                    let [r, g, b] = Color::cmyk(c, m, y, k).to_rgb();
                    [r, g, b, 0.0]
                }))
            })
            .clone();
        let rgba = px
            .iter()
            .flat_map(|p| {
                let [r, g, b, _] = lut.apply8([p[0], p[1], p[2], p[3]]);
                [r, g, b, p[4]]
            })
            .collect();
        png(width, height, rgba)
    }

    /// The document's CMYK profile as a table from ink amounts to sRGB (relative colorimetric,
    /// with black-point compensation, as Affinity's pixel layers convert), made once; `None` when
    /// the document embeds none, or not one that converts.
    fn document_cmyk(&mut self) -> Option<Arc<vectorcraft_color::cms::CmykLut>> {
        use vectorcraft_color::cms::{self, CmykLut, IccProfile, Intent};
        if let Some(lut) = &self.cmyk_lut {
            return lut.clone();
        }
        let embedded = self.cmyk_profile.is_some();
        let lut = self.cmyk_profile.take().and_then(|bytes| {
            let profile = IccProfile::from_bytes(None, &bytes).ok()?;
            let srgb = cms::icc::builtin_rgb(cms::SRGB)?;
            profile.to_rgb_of(&srgb, &[0.0; 4], Intent::RelativeColorimetric, true)?;
            Some(Arc::new(CmykLut::build(|inks| {
                let [r, g, b] = profile
                    .to_rgb_of(&srgb, &inks, Intent::RelativeColorimetric, true)
                    .unwrap_or_else(|| Color::cmyk(inks[0], inks[1], inks[2], inks[3]).to_rgb());
                [r, g, b, 0.0]
            })))
        });
        if embedded && lut.is_none() {
            self.warn("CMYK pixel layers whose document's colour profile could not be read (shown through the working CMYK profile)");
        }
        self.cmyk_lut = Some(lut.clone());
        lut
    }

    pub(super) fn image(&mut self, img: &af::Image, to_doc: Affine) -> Option<NodeKind> {
        let bytes = match &img.pixels {
            Pixels::Encoded(b) => b.clone(),
            Pixels::Rgba8(rgba) => png(img.width, img.height, rgba.clone())?,
            Pixels::Cmyka8(cmyka) => self.cmyk_image(img.width, img.height, cmyka)?,
        };
        let r = match crate::cmd::fileio::load::raster_image(&bytes) {
            Ok(r) => r,
            Err(_) => {
                self.warn("an embedded image that could not be decoded was left out");
                return None;
            }
        };
        // The pixel grid maps onto the Affinity image's own size, whatever the stored file's.
        let sx = img.width as f64 / f64::from(r.width.max(1));
        let sy = img.height as f64 / f64::from(r.height.max(1));
        let xf = to_doc * affine(img.transform) * Affine::scale_non_uniform(sx, sy);
        self.doc.images.entry(r.key.clone()).or_insert(r.blob);
        Some(NodeKind::Image(Box::new(ImageObject {
            key: r.key,
            width: r.width,
            height: r.height,
            xf,
            link: None,
            placement: Default::default(),
            pixels: None,
        })))
    }
}
