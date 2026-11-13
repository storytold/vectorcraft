use super::exporter::{MAX_FIELD_PX, MAX_GRADIENT_PX, MAX_PATTERN_DEPTH, color, ctm, in_range, norm, q, to_path, writable, xf};
use super::{
    Affine, Color, Exporter, GradientKind, LinearGradient, Paint, RadialGradient, Rect, SepColor, SeparationColorant, SeparationSpace, Shape, Size,
    SpreadMethod, Stop, Surface, SweepGradient, Vec2, cmyk, krilla, rgb,
};

/// Most stops a conic gradient's sweep shading holds: past this the PDF writer keeps only some
/// of them (its calculator function tests the stops one inside another, and viewers limit how
/// deep), so such a gradient is drawn as an image instead ([`Exporter::area_paint`]).
pub(super) const MAX_SWEEP_STOPS: usize = 97;

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
        let alt = krilla::color::RegularColor::Cmyk(cmyk::Color::new_f32(full[0], full[1], full[2], full[3]));
        Some(SeparationSpace::new(SeparationColorant::Custom(ink), alt))
    }

    /// The ink name of spot swatch `name` ([`Self::colorant_name`]) and its CMYK equivalent at
    /// 100% (a Lab spot color is noted for [`crate::lab_spot`]); `None` when `name` isn't a spot
    /// color.
    pub(super) fn spot_ink(&mut self, name: &str) -> Option<(String, [f32; 4])> {
        let doc = self.doc;
        let sw = doc.swatch(name).filter(|s| s.spot)?;
        let color = doc.linked_color(sw.paint.color()?, true);
        let ink = self.colorant_name(&sw.name);
        if let Color::Lab { l, a, b } = color
            && !self.lab_spots.iter().any(|(n, _)| *n == ink)
        {
            self.lab_spots.push((ink.clone(), vectorcraft_color::cms::Lab::new(l, a, b)));
        }
        Some((ink, self.out.cmyk(&color, self.intent)))
    }

    /// The colorant name spot swatch `name` is written under: its own name, except that `All`
    /// (every plate in PDF) and `None` (no plate) become `All Spot` and `None Spot`, numbered
    /// `All Spot 2`, `All Spot 3`, … past the names of the document's spot swatches.
    fn colorant_name(&self, name: &str) -> String {
        if name != "All" && name != "None" {
            return name.to_string();
        }
        let taken = |c: &str| self.doc.swatches_iter().any(|s| s.spot && s.name == c);
        let mut colorant = format!("{name} Spot");
        let mut n = 2;
        while taken(&colorant) {
            colorant = format!("{name} Spot {n}");
            n += 1;
        }
        colorant
    }

    /// A solid paint, in the Separation colour space at its tint when it's linked to a spot swatch
    /// or to Registration (`/All`: every plate).
    fn solid(&mut self, c: &Color, link: Option<&str>, tint: f32) -> krilla::color::Color {
        if link == Some(vectorcraft_color::swatch::REGISTRATION) {
            let alt = krilla::color::RegularColor::Cmyk(cmyk::Color::new(255, 255, 255, 255));
            return SepColor::new_f32(tint, SeparationSpace::new(SeparationColorant::AllColorants, alt)).into();
        }
        match link.and_then(|n| self.separation(n)) {
            Some(space) => SepColor::new_f32(tint, space).into(),
            None => self.col(c),
        }
    }

    /// The stops of a gradient whose every stop is a tint of one spot ink (or paper white, 0% of
    /// it) as tints of that ink — a Separation shading — with midpoints as explicit stops (the
    /// tint and opacity there are halfway). A gradient that mixes a spot ink with other colors
    /// gets [`Self::device_n_stops`]. `None` for gradients without a spot ink.
    fn spot_stops(&mut self, g: &vectorcraft_color::Gradient) -> Option<Vec<(f32, krilla::color::Color, f32)>> {
        let ink = g.stops.iter().find_map(|s| s.swatch.as_deref().filter(|n| self.doc.swatch(n).is_some_and(|w| w.spot)))?;
        let tints: Option<Vec<f32>> = g
            .stops
            .iter()
            .map(|s| match s.swatch.as_deref() {
                // A stop that also prints process plates needs a DeviceN space.
                Some(n) if n == ink && s.plates.is_none() => Some(s.tint),
                _ if self.paper_stop(s) => Some(0.0),
                _ => None,
            })
            .collect();
        let Some(tints) = tints else {
            return self.device_n_stops(g);
        };
        let space = self.separation(ink)?;
        let tint = |t: f32| -> krilla::color::Color { SepColor::new_f32(t, space.clone()).into() };
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
    /// there are halfway), or for a Perceptual gradient the stops on its curve
    /// ([`vectorcraft_color::Gradient::ink_mixes`]). Paper white adds no ink; a spot stop with
    /// process plates ([`vectorcraft_color::GradientStop::plates`]) prints them too. `None`, with a
    /// warning, when a stop is written in RGB or Lab or the spot inks can't share a DeviceN space;
    /// the gradient is then written in the colors of its stops.
    fn device_n_stops(&mut self, g: &vectorcraft_color::Gradient) -> Option<Vec<(f32, krilla::color::Color, f32)>> {
        use krilla::color::device_n::{Color as DnColor, Colorant, DeviceNSpace, Process};
        let doc = self.doc;
        fn spot_of<'s>(doc: &vectorcraft_doc::Document, s: &'s vectorcraft_color::GradientStop) -> Option<&'s str> {
            s.swatch.as_deref().filter(|n| doc.swatch(n).is_some_and(|w| w.spot))
        }
        // Each spot ink: its swatch's name, then its colorant name and CMYK equivalent.
        let mut spots: Vec<(&str, (String, [f32; 4]))> = Vec::new();
        for s in &g.stops {
            if let Some(n) = spot_of(doc, s)
                && !spots.iter().any(|(m, _)| *m == n)
            {
                spots.push((n, self.spot_ink(n)?));
            }
        }
        // Each stop's ink coverage: the four process inks, then one tint per spot ink.
        let mut inks: Vec<Vec<f32>> = Vec::with_capacity(g.stops.len());
        for s in &g.stops {
            let mut v = vec![0.0; 4 + spots.len()];
            match spot_of(doc, s) {
                Some(n) => {
                    let i = spots.iter().position(|(m, _)| *m == n)?;
                    v[4 + i] = s.tint;
                    if let Some(p) = s.plates {
                        v[..4].copy_from_slice(&p.map(|t| t.clamp(0.0, 1.0)));
                    }
                }
                None if self.paper_stop(s) => {}
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
        let process: Vec<usize> = (0..4).filter(|&i| inks.iter().any(|v| v[i] > 0.0)).collect();
        let colorants = process
            .iter()
            .map(|&i| Colorant::Process(Process::ALL[i]))
            .chain(spots.iter().map(|(_, (name, f))| Colorant::Spot { name: name.clone(), alternate: cmyk::Color::new_f32(f[0], f[1], f[2], f[3]) }));
        let Some(space) = DeviceNSpace::new(colorants) else {
            self.warn("gradients whose spot inks can't share a DeviceN color space are exported without the spot inks");
            return None;
        };
        let color = |v: &[f32]| -> Option<krilla::color::Color> {
            let tints = process.iter().map(|&i| v[i]).chain(v[4..].iter().copied()).collect();
            Some(DnColor::new_f32(tints, space.clone())?.into())
        };
        // A Perceptual gradient: the stops on its curve, each a mix of two stops' inks.
        if let Some(curve) = g.ink_mixes() {
            return curve
                .into_iter()
                .map(|(o, i, w, opacity)| {
                    let next = inks.get(i + 1).unwrap_or(&inks[i]);
                    let mixed: Vec<f32> = inks[i].iter().zip(next).map(|(a, b)| a + (b - a) * w).collect();
                    Some((o, color(&mixed)?, opacity))
                })
                .collect();
        }
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
                // A linear or radial gradient that repeats or reflects, where it isn't an area
                // paint ([`Self::padded`]): its stops repeated over `bounds`, padding past them.
                if matches!(g.gradient.kind, GradientKind::Linear | GradientKind::Radial) && g.gradient.is_spread() {
                    let paint = match g.gradient.padded_over(&geom, bounds) {
                        Some(padded) => {
                            self.warn_capped(padded.capped);
                            padded_paint(g, padded)
                        }
                        // Levels over `bounds` that are not finite (a vector of no length): the
                        // stops repeated along the vector, padding past it.
                        None => {
                            self.warn_capped(true);
                            let gradient = g.gradient.unrolled().into_owned();
                            Paint::Gradient(Box::new(vectorcraft_color::GradientPaint { gradient, ..(**g).clone() }))
                        }
                    };
                    return self.paint_in(s, &paint, bounds, space);
                }
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
                    // A function-based shading (ISO 32000-1, 8.7.4.5.2) whose calculator function
                    // turns the angle about the origin into the ramp, once around (0 to 360
                    // degrees), carried onto the extent ellipse by its frame. Stops that repeat
                    // or reflect `n` times run over 360/`n` degrees, the function repeating or
                    // reflecting them past it.
                    GradientKind::Conic => {
                        let frame = geom.conic_frame();
                        if !writable(s, space * frame, Rect::new(-1.0, -1.0, 1.0, 1.0)) {
                            return None;
                        }
                        let (end_angle, spread_method) = match g.gradient.spread {
                            _ if !g.gradient.is_spread() => (360.0, SpreadMethod::Pad),
                            vectorcraft_color::GradientSpread::Reflect => (360.0 / g.gradient.repeat_count() as f32, SpreadMethod::Reflect),
                            _ => (360.0 / g.gradient.repeat_count() as f32, SpreadMethod::Repeat),
                        };
                        Some(
                            SweepGradient {
                                cx: 0.0,
                                cy: 0.0,
                                start_angle: 0.0,
                                end_angle,
                                transform: xf(space * frame),
                                spread_method,
                                stops,
                                anti_alias: false,
                            }
                            .into(),
                        )
                    }
                    // Painted areas get an image of the field, or a diamond's four linear gradients
                    // ([`Self::area`]); elsewhere (gradient slices) the average color stands in.
                    GradientKind::Freeform | GradientKind::Diamond => {
                        self.warn(if g.gradient.kind == GradientKind::Diamond {
                            "diamond gradients in gradient slices are exported as their average color"
                        } else {
                            "freeform gradients are exported as their average colour"
                        });
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
        let mut cs = Surface::stream(sb.surface());
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

    /// Is stop `s` paper white ([`paper`]), unlinked or linked to a swatch that isn't a spot color
    /// (a white spot color is an ink of its own)? It adds no ink to a Separation or DeviceN shading.
    fn paper_stop(&self, s: &vectorcraft_color::GradientStop) -> bool {
        !s.swatch.as_deref().is_some_and(|n| self.doc.swatch(n).is_some_and(|w| w.spot)) && paper(&s.color)
    }

    /// The spot swatch whose tints are every stop of `g` that isn't paper white
    /// ([`Self::paper_stop`]); `None` when no stop is linked to a spot swatch or a stop has
    /// another color.
    fn one_spot_ink<'g>(&self, g: &'g vectorcraft_color::Gradient) -> Option<&'g str> {
        let spot = |s: &'g vectorcraft_color::GradientStop| s.swatch.as_deref().filter(|n| self.doc.swatch(n).is_some_and(|w| w.spot));
        let ink = g.stops.iter().find_map(spot)?;
        g.stops.iter().all(|s| (spot(s) == Some(ink) && s.plates.is_none()) || self.paper_stop(s)).then_some(ink)
    }

    /// Can every stop of `g` be inks of a DeviceN space ([`Self::device_n_stops`]): is a stop
    /// linked to a spot swatch, and is every other stop paper white ([`Self::paper_stop`]) or
    /// written in CMYK or gray?
    fn device_n_inks_only(&self, g: &vectorcraft_color::Gradient) -> bool {
        let doc = self.doc;
        let cmyk_doc = doc.color_mode == vectorcraft_doc::ColorMode::Cmyk;
        let spot = |s: &vectorcraft_color::GradientStop| s.swatch.as_deref().is_some_and(|n| doc.swatch(n).is_some_and(|w| w.spot));
        let inks = |s: &vectorcraft_color::GradientStop| {
            self.paper_stop(s) || matches!(self.out.color(&s.color, self.intent, cmyk_doc, self.convert), Color::Cmyk { .. } | Color::Gray { .. })
        };
        g.stops.iter().any(spot) && g.stops.iter().all(|s| spot(s) || inks(s))
    }

    /// Is `g` a Perceptual linear or radial gradient of spot inks, alone or mixed with process
    /// inks, that paints a fill (`fill`) without dither or a stroke, with a curve short enough for
    /// [`vectorcraft_color::Gradient::fits_ink_tints`]: one that [`Self::perceptual_spot_stops`]
    /// writes as a Separation shading, or [`Self::device_n_stops`] as a DeviceN shading, rather
    /// than an image?
    pub(super) fn perceptual_spot(&self, g: &vectorcraft_color::GradientPaint, fill: bool) -> bool {
        let gr = &g.gradient;
        !gr.method.is_classic()
            && gr.kind != GradientKind::Freeform
            && !(fill && gr.dither)
            && (self.one_spot_ink(gr).is_some() || self.device_n_inks_only(gr))
            && gr.fits_ink_tints()
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
        // The stops keep their own tints; paper white is 0% of the ink.
        let own: Vec<f32> = g.stops.iter().map(|s| if s.swatch.as_deref() == Some(ink) { s.tint } else { 0.0 }).collect();
        Some(g.ink_tints(&full, &own)?.into_iter().map(|(o, t, a)| (o, SepColor::new_f32(t, space.clone()).into(), a)).collect())
    }

    /// Warn when gradient `g`, drawn as an image, has stops linked to a spot swatch: the image is
    /// in process colors, so the spot inks have no plate of their own.
    fn warn_spot_image(&mut self, g: &vectorcraft_color::GradientPaint) {
        let from_stops = g.gradient.kind != GradientKind::Freeform || !g.has_freeform_points();
        if from_stops && self.has_spot_stop(&g.gradient) {
            self.warn(
                "spot colors in gradients exported as images (freeform or dithered gradients, and Perceptual ones with RGB or Lab stops or too many stops) are written in process colors",
            );
        }
    }

    /// Is a stop of `g` linked to a spot swatch?
    fn has_spot_stop(&self, g: &vectorcraft_color::Gradient) -> bool {
        let doc = self.doc;
        g.stops.iter().any(|s| s.swatch.as_deref().is_some_and(|n| doc.swatch(n).is_some_and(|w| w.spot)))
    }

    /// Draw diamond gradient `g`, placed on `bounds`, over `area` (inside the current clip) as four
    /// linear gradients, one for each quadrant of its frame, each equal to the diamond there (PDF
    /// has no diamond shading; [`vectorcraft_color::GradientGeom::diamond_quadrants`]). The
    /// quadrants overlap so no hairline shows between them, which needs opaque stops. False
    /// (nothing drawn) for a diamond with translucent stops or without a frame, for one with a
    /// quadrant whose shading or outline PDF can't hold, and for a Perceptual one of spot inks
    /// whose ink curve has too many tints once its stops are repeated, which
    /// [`Self::field_image`] draws instead.
    pub(super) fn diamond(&mut self, s: &mut Surface, g: &vectorcraft_color::GradientPaint, bounds: Rect, area: Rect) -> bool {
        if g.gradient.stops.iter().any(|st| st.opacity < 1.0) {
            return false;
        }
        // A diamond that repeats or reflects: its stops repeated over the levels the area reaches
        // from the center, then split into quadrants, each of which runs those levels.
        let (diamond, geom, capped) = match g.gradient.padded_over(&g.resolve(bounds), area.abs()) {
            Some(padded) => (padded.gradient, padded.geom, padded.capped),
            None => (g.gradient.clone(), g.resolve(bounds), false),
        };
        // A Perceptual one of spot inks whose ink curve no longer fits with its stops repeated.
        if !diamond.method.is_classic() && !diamond.fits_ink_tints() {
            return false;
        }
        let Some(quadrants) = geom.diamond_quadrants(area.abs()) else { return false };
        let gradient = vectorcraft_color::Gradient { kind: GradientKind::Linear, ..diamond };
        let mut drawn = Vec::with_capacity(4);
        for (outline, geom) in quadrants {
            let linear = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint { gradient: gradient.clone(), geom: Some(geom), ..g.clone() }));
            let quadrant = kurbo::BezPath::from_vec(
                outline
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| if i == 0 { kurbo::PathEl::MoveTo(p) } else { kurbo::PathEl::LineTo(p) })
                    .chain([kurbo::PathEl::ClosePath])
                    .collect(),
            );
            let (Some(paint), Some(path)) = (self.paint(s, &linear, bounds), to_path(s, &quadrant)) else { return false };
            drawn.push((paint, path));
        }
        self.warn("diamond gradients are exported as four linear gradients");
        self.warn_capped(capped);
        for (paint, path) in drawn {
            s.set_stroke(None);
            s.set_fill(Some(krilla::paint::Fill { paint, opacity: krilla::num::NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
            s.draw_path(&path);
            s.set_fill(None);
        }
        true
    }

    /// Draw linear or radial gradient `g`, placed on `bounds`, that repeats or reflects, over
    /// `area` (inside the current clip): PDF's axial and radial shadings pad past their ends, so
    /// its stops are repeated as many times as the area needs
    /// ([`vectorcraft_color::Gradient::padded_over`]). False (nothing drawn) for another gradient,
    /// for one whose shading or area PDF can't hold, and for a Perceptual one of spot inks whose
    /// ink curve has too many tints once repeated, which [`Self::field_image`] draws instead.
    pub(super) fn padded(&mut self, s: &mut Surface, g: &vectorcraft_color::GradientPaint, bounds: Rect, area: Rect) -> bool {
        if !matches!(g.gradient.kind, GradientKind::Linear | GradientKind::Radial) {
            return false;
        }
        let Some(padded) = g.gradient.padded_over(&g.resolve(bounds), area.abs()) else { return false };
        // A Perceptual gradient of spot inks whose ink curve no longer fits once its stops are
        // repeated is an image, as one with too many stops is.
        if !g.gradient.method.is_classic() && !padded.gradient.fits_ink_tints() {
            return false;
        }
        let capped = padded.capped;
        let paint = padded_paint(g, padded);
        let (Some(paint), Some(path)) = (self.paint(s, &paint, bounds), to_path(s, &kurbo::Shape::to_path(&area.abs(), 0.1))) else {
            return false;
        };
        self.warn_capped(capped);
        s.set_stroke(None);
        s.set_fill(Some(krilla::paint::Fill { paint, opacity: krilla::num::NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
        s.draw_path(&path);
        s.set_fill(None);
        true
    }

    /// Warn when a gradient that repeats or reflects needed more runs of its stops than were
    /// written.
    fn warn_capped(&mut self, capped: bool) {
        if capped {
            self.warn(format!(
                "gradients that repeat or reflect are exported with at most {} runs of their stops (at most {} stops); they pad past those",
                vectorcraft_color::gradient::MAX_SPREAD_RUNS,
                vectorcraft_color::gradient::MAX_SPREAD_STOPS
            ));
        }
    }

    /// Draw gradient `g`, placed on `bounds`, as an image over `area`, sampled at the document's
    /// raster effects resolution where it is drawn (`area` as the current transform sizes it, so
    /// scaled type gets its pixels too): a freeform gradient's colour field (at most
    /// [`MAX_FIELD_PX`] a side), or a linear, radial, conic or diamond gradient (at most [`MAX_GRADIENT_PX`] a side),
    /// with dither's noise when it paints a fill (`fill`). A linear or radial gradient whose stops
    /// are all CMYK and not linked to a spot swatch is a CMYK image, unless its Perceptual curve
    /// has too many stops; any other is an RGB image.
    pub(super) fn field_image(&mut self, s: &mut Surface, g: &vectorcraft_color::GradientPaint, bounds: Rect, area: Rect, fill: bool) {
        use vectorcraft_color::cms::Model;
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
        let field = freeform.then(|| g.freeform_on(b).model_field(spread_scale(b)));
        let cmyk_doc = self.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk;
        if g.gradient.kind == GradientKind::Conic {
            self.warn("conic gradients drawn as pixels (dithered fills, and Perceptual ones other than spot ink shadings), in a PDF/A file or with more than 97 stops are exported as images");
        } else if g.gradient.kind == GradientKind::Diamond {
            self.warn("diamond gradients drawn as pixels (dithered fills, and Perceptual ones other than spot ink shadings), with translucent stops or without an extent are exported as images");
        } else if !freeform {
            self.warn("linear and radial gradients drawn as pixels are exported as images");
        }
        let ramp_inks = if freeform || self.has_spot_stop(&g.gradient) { None } else { g.ink_pixels(b, area, cols, rows, fill) };
        let ink_image = |inks, alpha| {
            let convert = if self.convert { self.out.cmyk_pixels() } else { None };
            crate::images::from_inks(inks, alpha, cols.into(), rows.into(), self.interpolate, convert)
        };
        let img = match (&field, ramp_inks) {
            (Some(field), _) if field.model() == Some(Model::Cmyk) || (cmyk_doc && field.model() == Some(Model::Gray)) => {
                let (mut inks, mut alpha) = (Vec::new(), Vec::new());
                for (color, a) in field.grid(area, cols, rows) {
                    let [c, m, y, k] = color.to_cmyk();
                    inks.extend([q(c), q(m), q(y), q(k)]);
                    alpha.push(q(a));
                }
                ink_image(inks, alpha)
            }
            (_, Some(pixels)) => {
                let (mut inks, mut alpha) = (Vec::new(), Vec::new());
                for [c, m, y, k, a] in pixels {
                    inks.extend([c, m, y, k]);
                    alpha.push(a);
                }
                ink_image(inks, alpha)
            }
            _ => {
                let rgba: Vec<u8> = match &field {
                    Some(field) => field.display_grid(area, cols, rows).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a)]).collect(),
                    None => g.pixels(b, area, cols, rows, fill, &|c| c.to_rgb()).flatten().collect(),
                };
                let pixels = if self.convert { self.out.pixels() } else { None };
                crate::images::from_rgba_in(rgba, cols.into(), rows.into(), self.interpolate, pixels)
            }
        };
        let (Some(img), Some(size)) = (img, Size::from_wh(area.width() as f32, area.height() as f32)) else {
            return;
        };
        s.push_transform(&xf(Affine::translate(area.origin().to_vec2())));
        s.draw_image(img, size);
        s.pop();
    }
}

/// `g` written as `padded` gives it: its stops repeated, placed there.
fn padded_paint(g: &vectorcraft_color::GradientPaint, padded: vectorcraft_color::gradient::Padded) -> Paint {
    let geom = padded.geom;
    Paint::Gradient(Box::new(vectorcraft_color::GradientPaint { gradient: padded.gradient, geom: Some(geom), angle: geom.angle_deg(), ..g.clone() }))
}

/// Is `c` paper white: no ink in CMYK or gray, white in RGB or Lab?
fn paper(c: &Color) -> bool {
    match *c {
        Color::Cmyk { c, m, y, k } => c <= 0.0 && m <= 0.0 && y <= 0.0 && k <= 0.0,
        Color::Gray { k } => k <= 0.0,
        _ => c.to_rgba8(1.0)[..3] == [255; 3],
    }
}
