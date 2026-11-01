use super::exporter::{MAX_FIELD_PX, MAX_GRADIENT_PX, MAX_PATTERN_DEPTH, color, ctm, in_range, norm, q, to_path, writable, xf};
use super::{
    Affine, Color, Exporter, GradientKind, LinearGradient, Paint, RadialGradient, Rect, SepColor, SeparationColorant, SeparationSpace, Shape, Size,
    SpreadMethod, Stop, Surface, Vec2, cmyk, krilla, rgb,
};

impl Exporter<'_> {
    /// A colour for the page, as the Output settings write it ([`ColorOut::color`]): without
    /// conversion, RGB and Lab colours of CMYK documents are separated into CMYK through the
    /// document's colour spaces, so the file carries press values.
    fn col(&mut self, c: &Color) -> krilla::color::Color {
        let c = self.page_color(c);
        color(&c)
    }

    /// Color `c` as [`Self::col`] writes it, before it becomes the PDF writer's color.
    pub(super) fn page_color(&mut self, c: &Color) -> Color {
        let cmyk_doc = self.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk;
        if let (true, true, Color::Rgb { r, g, b }) = (cmyk_doc, self.convert, c) {
            self.rgb_in_cmyk.colours.insert([r.to_bits(), g.to_bits(), b.to_bits()]);
        }
        let c = self.out.color(c, self.intent, cmyk_doc, self.convert);
        if let (Color::Rgb { .. }, Some(note)) = (c, &self.out.srgb_note) {
            let note = note.clone();
            self.warn(note);
        }
        c
    }

    /// The Separation colour space of spot swatch `name` (its CMYK equivalent is the alternate
    /// space; a Lab spot colour also gets a Lab one, [`crate::lab_spot`], unless the Spot Colors
    /// options use CMYK values); `None` when `name` isn't a spot colour.
    fn separation(&mut self, name: &str) -> Option<SeparationSpace> {
        let (ink, full) = self.spot_ink(name)?;
        let alt = krilla::color::RegularColor::Cmyk(cmyk::Color::new(q(full[0]), q(full[1]), q(full[2]), q(full[3])));
        Some(SeparationSpace::new(SeparationColorant::Custom(ink), alt))
    }

    /// The ink name of spot swatch `name` and its CMYK equivalent at 100% (a Lab spot color is
    /// noted for [`crate::lab_spot`]); `None` when `name` isn't a spot color.
    pub(super) fn spot_ink(&mut self, name: &str) -> Option<(String, [f32; 4])> {
        let doc = self.doc;
        let sw = doc.swatch(name).filter(|s| s.spot)?;
        let color = doc.linked_color(sw.paint.color()?, true);
        if let Color::Lab { l, a, b } = color
            && !self.lab_spots.iter().any(|(n, _)| n == name)
        {
            self.lab_spots.push((name.to_string(), vectorcraft_color::cms::Lab::new(l, a, b)));
        }
        Some((sw.name.clone(), self.out.cmyk(&color, self.intent)))
    }

    /// A solid paint, in the Separation colour space at its tint when it's linked to a spot swatch
    /// or to Registration (`/All`: every plate).
    fn solid(&mut self, c: &Color, link: Option<&str>, tint: f32) -> krilla::color::Color {
        if link == Some(vectorcraft_color::swatch::REGISTRATION) {
            let alt = krilla::color::RegularColor::Cmyk(cmyk::Color::new(255, 255, 255, 255));
            return SepColor::new(q(tint), SeparationSpace::new(SeparationColorant::AllColorants, alt)).into();
        }
        match link.and_then(|n| self.separation(n)) {
            Some(space) => SepColor::new(q(tint), space).into(),
            None => self.col(c),
        }
    }

    /// The stops of a gradient whose every stop is a tint of one spot ink (or paper white, 0% of
    /// it) as tints of that ink — a Separation shading — with midpoints as explicit stops (the
    /// tint and opacity there are halfway). A gradient that mixes a spot ink with other colors
    /// gets [`Self::device_n_stops`]. `None` for gradients without a spot ink.
    fn spot_stops(&mut self, g: &vectorcraft_color::Gradient) -> Option<Vec<(f32, krilla::color::Color, f32)>> {
        let ink = g.stops.iter().find_map(|s| s.swatch.as_deref().filter(|n| self.doc.swatch(n).is_some_and(|w| w.spot)))?;
        let paper = |c: &Color| c.to_rgba8(1.0)[..3] == [255; 3];
        let tints: Option<Vec<f32>> = g
            .stops
            .iter()
            .map(|s| match s.swatch.as_deref() {
                Some(n) if n == ink => Some(s.tint),
                None if paper(&s.color) => Some(0.0),
                _ => None,
            })
            .collect();
        let Some(tints) = tints else {
            return self.device_n_stops(g);
        };
        let space = self.separation(ink)?;
        let tint = |t: f32| -> krilla::color::Color { SepColor::new(q(t), space.clone()).into() };
        let mut out = vec![];
        for (i, s) in g.stops.iter().enumerate() {
            out.push((s.offset, tint(tints[i]), s.opacity));
            if let Some(n) = g.stops.get(i + 1)
                && (s.midpoint - 0.5).abs() > 1e-3
            {
                let at = s.offset + (n.offset - s.offset) * s.midpoint;
                out.push((at, tint((tints[i] + tints[i + 1]) / 2.0), (s.opacity + n.opacity) / 2.0));
            }
        }
        Some(out)
    }

    /// The stops of a gradient that mixes spot inks with process colors or with each other, in
    /// a DeviceN space of the process inks the stops use (in CMYK order) and the spot inks (in
    /// the order the stops name them), with midpoints as explicit stops (the tints and opacity
    /// there are halfway). Paper white adds no ink. `None`, with a warning, when a stop is
    /// written in RGB or Lab or the spot inks can't share a DeviceN space; the gradient is then
    /// written in the colors of its stops.
    fn device_n_stops(&mut self, g: &vectorcraft_color::Gradient) -> Option<Vec<(f32, krilla::color::Color, f32)>> {
        use krilla::color::device_n::{Color as DnColor, Colorant, DeviceNSpace, Process};
        let paper = |c: &Color| c.to_rgba8(1.0)[..3] == [255; 3];
        let doc = self.doc;
        fn spot_of<'s>(doc: &vectorcraft_doc::Document, s: &'s vectorcraft_color::GradientStop) -> Option<&'s str> {
            s.swatch.as_deref().filter(|n| doc.swatch(n).is_some_and(|w| w.spot))
        }
        let mut spots: Vec<(String, [f32; 4])> = Vec::new();
        for s in &g.stops {
            if let Some(n) = spot_of(doc, s)
                && !spots.iter().any(|(m, _)| m == n)
            {
                spots.push(self.spot_ink(n)?);
            }
        }
        // Each stop's ink coverage: the four process inks, then one tint per spot ink.
        let mut inks: Vec<Vec<f32>> = Vec::with_capacity(g.stops.len());
        for s in &g.stops {
            let mut v = vec![0.0; 4 + spots.len()];
            match spot_of(doc, s) {
                Some(n) => {
                    let i = spots.iter().position(|(m, _)| m == n)?;
                    v[4 + i] = s.tint;
                }
                None if s.swatch.is_none() && paper(&s.color) => {}
                None => match self.page_color(&s.color) {
                    Color::Cmyk { c, m, y, k } => v[..4].copy_from_slice(&[c, m, y, k]),
                    Color::Gray { k } => v[3] = k,
                    Color::Rgb { .. } | Color::Lab { .. } => {
                        self.warn("gradients mixing a spot color with RGB or Lab colors are exported without the spot ink");
                        return None;
                    }
                },
            }
            inks.push(v);
        }
        let process: Vec<usize> = (0..4).filter(|&i| inks.iter().any(|v| q(v[i]) > 0)).collect();
        let colorants = process.iter().map(|&i| Colorant::Process(Process::ALL[i])).chain(
            spots.iter().map(|(name, f)| Colorant::Spot { name: name.clone(), alternate: cmyk::Color::new(q(f[0]), q(f[1]), q(f[2]), q(f[3])) }),
        );
        let Some(space) = DeviceNSpace::new(colorants) else {
            self.warn("gradients whose spot inks can't share a DeviceN color space are exported without the spot inks");
            return None;
        };
        let color = |v: &[f32]| -> Option<krilla::color::Color> {
            let tints = process.iter().map(|&i| q(v[i])).chain(v[4..].iter().map(|&t| q(t))).collect();
            Some(DnColor::new(tints, space.clone())?.into())
        };
        let mut out = vec![];
        for (i, s) in g.stops.iter().enumerate() {
            out.push((s.offset, color(&inks[i])?, s.opacity));
            if let Some(n) = g.stops.get(i + 1)
                && (s.midpoint - 0.5).abs() > 1e-3
            {
                let at = s.offset + (n.offset - s.offset) * s.midpoint;
                let half: Vec<f32> = inks[i].iter().zip(&inks[i + 1]).map(|(a, b)| (a + b) / 2.0).collect();
                out.push((at, color(&half)?, (s.opacity + n.opacity) / 2.0));
            }
        }
        Some(out)
    }

    /// Warn when `effects` (an object's, a fill's or a stroke's) has a visible raster effect: the
    /// app renders them to images before export (`vectorcraft_engine::export_pdf`), so the ones
    /// still here (on `what`) can't be written.
    pub(super) fn warn_raster(&mut self, effects: &[vectorcraft_doc::Effect], what: impl FnOnce() -> String) {
        if effects.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id)) {
            self.warn(format!("raster effects (shadows, glows, blur, feather) on {} are left out of the PDF", what()));
        }
    }

    pub(super) fn warn(&mut self, w: impl Into<String>) {
        let w = w.into();
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    /// Convert a paint to draw on `s`. `bounds` resolves unset gradient geometry (like the
    /// renderer). A gradient placed beyond [`MAX_COORD`] there gives no paint.
    pub(super) fn paint(&mut self, s: &Surface, p: &Paint, bounds: Rect) -> Option<krilla::paint::Paint> {
        self.paint_in(s, p, bounds, Affine::IDENTITY)
    }

    /// [`Self::paint`] drawn in another space: `space` maps the paint's space (where `bounds`
    /// is) to the one drawn in.
    pub(super) fn paint_in(&mut self, s: &Surface, p: &Paint, bounds: Rect, space: Affine) -> Option<krilla::paint::Paint> {
        match p {
            Paint::None => None,
            Paint::Solid { color: c, swatch, tint } => Some(self.solid(c, swatch.as_deref(), *tint).into()),
            Paint::Gradient(g) => {
                let geom = g.resolve(bounds);
                let spot = match g.gradient.kind {
                    GradientKind::Freeform => None,
                    _ => self.perceptual_spot_stops(&g.gradient).or_else(|| self.spot_stops(&g.gradient)),
                };
                let colored = match spot {
                    Some(v) => v,
                    None => {
                        let stops = g.gradient.expanded_stops();
                        // The PDF writer needs every stop in one colour space: stops of mixed
                        // models (midpoints sample in RGB) go through RGB, or stay CMYK in a CMYK
                        // document, where `col` separates the rest.
                        let mixed = stops.windows(2).any(|w| w[0].1.model() != w[1].1.model());
                        let cmyk_doc = self.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk;
                        let one = |c: Color| match c {
                            Color::Cmyk { .. } if cmyk_doc => c,
                            _ if mixed => c.in_model(vectorcraft_color::cms::Model::Rgb),
                            _ => c,
                        };
                        stops.into_iter().map(|(o, c, a)| (o, self.col(&one(c)), a)).collect()
                    }
                };
                let mut stops: Vec<Stop> = Vec::new();
                let mut last = 0.0f32;
                for (o, color, a) in colored {
                    let o = o.clamp(last, 1.0);
                    last = o;
                    stops.push(Stop { offset: norm(o), color, opacity: norm(a) });
                }
                if stops.is_empty() {
                    return None;
                }
                match g.gradient.kind {
                    GradientKind::Linear => {
                        if !writable(s, space, Rect::from_points(geom.start, geom.end)) {
                            return None;
                        }
                        let (s, mut e) = (geom.start, geom.end);
                        if s.distance(e) < 1e-9 {
                            e = s + Vec2::new(1.0, 0.0);
                        }
                        Some(
                            LinearGradient {
                                x1: s.x as f32,
                                y1: s.y as f32,
                                x2: e.x as f32,
                                y2: e.y as f32,
                                transform: xf(space),
                                spread_method: SpreadMethod::Pad,
                                stops,
                                anti_alias: false,
                            }
                            .into(),
                        )
                    }
                    GradientKind::Radial => {
                        let r = geom.length().max(1e-6) as f32;
                        let t = geom.radial_squash();
                        let (cx, cy) = (geom.start.x as f32, geom.start.y as f32);
                        // An off-centre focal point: a two-point radial shading from it.
                        let f = t.inverse() * geom.focal_point();
                        let circles = Rect::from_center_size(geom.start, (2.0 * f64::from(r), 2.0 * f64::from(r))).union_pt(f);
                        if !writable(s, space * t, circles) {
                            return None;
                        }
                        Some(
                            RadialGradient {
                                fx: f.x as f32,
                                fy: f.y as f32,
                                fr: 0.0,
                                cx,
                                cy,
                                cr: r,
                                transform: xf(space * t),
                                spread_method: SpreadMethod::Pad,
                                stops,
                                anti_alias: false,
                            }
                            .into(),
                        )
                    }
                    // Painted areas get an image of the field ([`Self::area`]); elsewhere (gradient
                    // slices) the average colour stands in.
                    GradientKind::Freeform => {
                        self.warn("freeform gradients are exported as their average colour");
                        let n = g.gradient.stops.len().max(1) as f32;
                        let mut acc = [0.0f32; 3];
                        for s in &g.gradient.stops {
                            let c = s.color.to_rgb();
                            for i in 0..3 {
                                acc[i] += c[i] / n;
                            }
                        }
                        Some(self.col(&Color::rgb(acc[0], acc[1], acc[2])).into())
                    }
                }
            }
            // Patterns paint through [`Self::area`]: only a missing one gets here.
            Paint::Pattern { .. } => {
                self.warn("missing patterns are exported as mid-grey");
                Some(rgb::Color::new(128, 128, 128).into())
            }
        }
    }

    /// Pattern swatch `name` placed by `at` (pattern space → the current space) as a PDF tiling
    /// pattern (ISO 32000-1, 8.7.3.1): one period of the tiling ([`vectorcraft_doc::PatternDef::period`],
    /// what the canvas repeats too) with every tile instance reaching into it, in paint order, so
    /// brick and hex tilings and overlapping tiles repeat as they are; clipped to the period when
    /// art reaches out of it. `None` for a pattern nested deeper than [`MAX_PATTERN_DEPTH`], a period
    /// or placement beyond [`MAX_COORD`] or a placement that flattens the tiles.
    pub(super) fn tiling(&mut self, s: &mut Surface, name: &str, at: Affine) -> Option<krilla::paint::Paint> {
        let doc = self.doc;
        let def = doc.pattern(name)?;
        let (pw, ph) = def.period();
        if self.pattern_depth >= MAX_PATTERN_DEPTH || !in_range(&[pw, ph]) || !in_range(&at.as_coeffs()) || at.determinant().abs() <= 1e-12 {
            return None;
        }
        let cell = Rect::new(0.0, 0.0, pw, ph);
        let tiles = def.instances_in(Affine::IDENTITY, cell);
        let mut sb = s.stream_builder();
        let mut cs = sb.surface();
        let reaches_out = tiles.iter().any(|t| t.visual_bounds().is_some_and(|b| cell.intersect(b) != b));
        let clip = reaches_out.then(|| to_path(&cs, &cell.to_path(0.1))).flatten();
        if let Some(c) = &clip {
            cs.push_clip_path(c, &krilla::paint::FillRule::NonZero);
        }
        // Pattern art is a picture of its own: it takes no part in a knockout group around the object.
        let knockout = std::mem::take(&mut self.knockout);
        self.pattern_depth += 1;
        for t in &tiles {
            self.node(&mut cs, t, cell, true);
        }
        self.pattern_depth -= 1;
        self.knockout = knockout;
        if clip.is_some() {
            cs.pop();
        }
        cs.finish();
        Some(krilla::paint::Pattern { stream: sb.finish(), transform: xf(at), width: pw as f32, height: ph as f32 }.into())
    }

    /// The spot swatch whose tints are every stop of `g` that isn't paper white, unlinked; `None`
    /// when no stop is linked to a spot swatch or a stop has another color.
    fn one_spot_ink<'g>(&self, g: &'g vectorcraft_color::Gradient) -> Option<&'g str> {
        let spot = |s: &'g vectorcraft_color::GradientStop| s.swatch.as_deref().filter(|n| self.doc.swatch(n).is_some_and(|w| w.spot));
        let ink = g.stops.iter().find_map(spot)?;
        let paper = |s: &vectorcraft_color::GradientStop| s.swatch.is_none() && s.color.to_rgba8(1.0)[..3] == [255; 3];
        g.stops.iter().all(|s| spot(s) == Some(ink) || paper(s)).then_some(ink)
    }

    /// Is `g` a Perceptual linear or radial gradient of tints of one spot ink that paints a fill
    /// (`fill`) without dither or a stroke, with a curve short enough for
    /// [`vectorcraft_color::Gradient::fits_ink_tints`], which [`Self::perceptual_spot_stops`]
    /// writes as a Separation shading rather than an image?
    pub(super) fn perceptual_spot(&self, g: &vectorcraft_color::GradientPaint, fill: bool) -> bool {
        let gr = &g.gradient;
        !gr.method.is_classic() && gr.kind != GradientKind::Freeform && !(fill && gr.dither) && self.one_spot_ink(gr).is_some() && gr.fits_ink_tints()
    }

    /// The stops of a Perceptual gradient of tints of one spot ink (or paper white, 0% of it) as
    /// tints of that ink, a Separation shading: the stops on its curve with the tints whose
    /// lightness is the curve's there ([`vectorcraft_color::Gradient::ink_tints`]). `None` for
    /// other gradients, and for an ink too light for lightness to tell its tints apart, which
    /// [`Self::spot_stops`] then writes with the stops' own tints.
    fn perceptual_spot_stops(&mut self, g: &vectorcraft_color::Gradient) -> Option<Vec<(f32, krilla::color::Color, f32)>> {
        if g.method.is_classic() {
            return None;
        }
        let ink = self.one_spot_ink(g)?;
        let doc = self.doc;
        let full = doc.linked_color(doc.swatch(ink)?.paint.color()?, true);
        let space = self.separation(ink)?;
        Some(g.ink_tints(&full)?.into_iter().map(|(o, t, a)| (o, SepColor::new(q(t), space.clone()).into(), a)).collect())
    }

    /// Warn when gradient `g`, drawn as an image, has stops linked to a spot swatch: the image is
    /// in process colors, so the spot inks have no plate of their own.
    fn warn_spot_image(&mut self, g: &vectorcraft_color::GradientPaint) {
        let doc = self.doc;
        let from_stops = g.gradient.kind != GradientKind::Freeform || g.freeform.is_none();
        if from_stops && g.gradient.stops.iter().any(|s| s.swatch.as_deref().is_some_and(|n| doc.swatch(n).is_some_and(|w| w.spot))) {
            self.warn("spot colors in gradients exported as images (freeform, dithered or Perceptual gradients that mix inks) are written in process colors");
        }
    }

    /// Draw gradient `g`, placed on `bounds`, as an image over `area`, sampled at the document's
    /// raster effects resolution where it is drawn (`area` as the current transform sizes it, so
    /// scaled type gets its pixels too): a freeform gradient's colour field (at most
    /// [`MAX_FIELD_PX`] a side), or a linear or radial gradient (at most [`MAX_GRADIENT_PX`] a side),
    /// with dither's noise when it paints a fill (`fill`).
    pub(super) fn field_image(&mut self, s: &mut Surface, g: &vectorcraft_color::GradientPaint, bounds: Rect, area: Rect, fill: bool) {
        use vectorcraft_color::freeform::{painted_box, spread_scale};
        let Some(b) = painted_box(bounds) else { return };
        let area = area.abs();
        // The area's sides in points on the page.
        let [m0, m1, m2, m3, _, _] = ctm(s).as_coeffs();
        let (w, h) = (area.width() * m0.hypot(m1), area.height() * m2.hypot(m3));
        let long = w.max(h);
        if !(long > 1e-9 && long.is_finite()) {
            return;
        }
        self.warn_spot_image(g);
        let freeform = g.gradient.kind == GradientKind::Freeform;
        let max = if freeform { MAX_FIELD_PX } else { MAX_GRADIENT_PX };
        let k = (self.doc.raster_effects_ppi / 72.0).min(max / long);
        let along = |len: f64| (len * k).ceil().clamp(1.0, max) as u16;
        let (cols, rows) = (along(w), along(h));
        let rgba: Vec<u8> = if freeform {
            let field = g.freeform_on(b).field_with(spread_scale(b), &|c| c.to_rgb());
            field.grid(area, cols, rows).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a)]).collect()
        } else {
            self.warn("linear and radial gradients drawn as pixels are exported as images");
            g.pixels(b, area, cols, rows, fill, &|c| c.to_rgb()).flatten().collect()
        };
        let pixels = if self.convert { self.out.pixels() } else { None };
        let (Some(img), Some(size)) = (
            crate::images::from_rgba_in(rgba, cols.into(), rows.into(), self.interpolate, pixels),
            Size::from_wh(area.width() as f32, area.height() as f32),
        ) else {
            return;
        };
        s.push_transform(&xf(Affine::translate(area.origin().to_vec2())));
        s.draw_image(img, size);
        s.pop();
    }
}
