//! Gradients and patterns: their definitions in the setup, gradient instances on paths and the
//! pattern operators (see [`crate::ai_editing`] for the form).

use std::fmt::Write;

use vectorcraft_color::gradient::GradientGeom;
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{AppearanceItem, FillLayer, PatternDef, StrokeLayer, TileType};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, BezPath, Rect, Shape, Vec2};

use super::{MAX_NEST, Writer, Xy};
use crate::ai_ops::{self, average, unit};
use crate::ps;

pub(in crate::ai_editing) const GRADIENTS: &str =
    "freeform gradients, and gradients along or across strokes, are written as their average color in the editing data for other apps";
pub(in crate::ai_editing) const GRADIENT_SHAPES: &str = "gradients without stops or with more than 256, gradients whose ramp has no length and radial gradients flattened to a line are written as their average color in the editing data for other apps";
pub(in crate::ai_editing) const SWATCH_PLACEMENT: &str =
    "the placement of pattern swatches that move, turn or scale their pattern is left out of the editing data for other apps";
pub(in crate::ai_editing) const GRADIENT_MIX: &str =
    "gradients that mix their colors perceptually or dither are written as classic gradients in the editing data for other apps";
pub(in crate::ai_editing) const GRADIENT_HUE: &str = "gradients mixed around the hue wheel are written in process colors, with the colors between their stops as stops of their own (at most 256), in the editing data for other apps";
pub(in crate::ai_editing) const CONIC: &str =
    "conic gradients are written as linear gradients along their vector, with the same stops, in the editing data for other apps";
pub(in crate::ai_editing) const STOP_LINKS: &str =
    "gradient stops linked to global process colors are written as their colors in the editing data for other apps";
/// The length of a linear gradient's caps (`Bc`): the canvas's side.
const CAP: f64 = super::super::CANVAS_SIZE as f64;

/// A gradient definition: its name, its gradient, whether it is a gradient swatch's (named after
/// the swatch) and whether the art uses it (else only the palette does).
pub(super) struct GradientDef {
    name: String,
    gradient: Gradient,
    swatch: bool,
    used: bool,
}

/// A pattern definition written: its name, its text (`%AI3_BeginPattern` to `%AI3_EndPattern`)
/// and whether the art uses it.
pub(super) struct PatternOut {
    name: String,
    text: String,
    used: bool,
}

pub(in crate::ai_editing) const DIAMOND: &str =
    "diamond gradients are written as radial gradients through their corners, with the same stops, in the editing data for other apps";
pub(in crate::ai_editing) const SPREAD: &str = "gradients that repeat or reflect are written with their stops repeated across the shape (at most 256 stops, padding past them) in the editing data for other apps";

/// Can `g` be written as a gradient (a linear, radial, conic or diamond one with stops, no more
/// than the `.ai` reader reads; a conic is written linear, [`CONIC`], and a diamond radial,
/// [`DIAMOND`])?
fn writable_gradient(g: &Gradient) -> bool {
    g.kind != GradientKind::Freeform && !g.stops.is_empty() && g.stops.len() <= crate::import::MAX_GRADIENT_STOPS
}

/// `g` as the data holds its colors: one mixed around the hue wheel (the data's gradients mix
/// their stops' colors directly) as a classic gradient of the stops on its curve
/// ([`Gradient::as_classic_curve`]), no more than the `.ai` reader reads when it pads
/// ([`GRADIENT_HUE`]); others as they are.
fn hue_as_stops(g: &Gradient) -> std::borrow::Cow<'_, Gradient> {
    use std::borrow::Cow;
    if !g.method.is_hue() {
        return Cow::Borrowed(g);
    }
    let curve = g.as_classic_curve();
    Cow::Owned(if curve.is_spread() { curve } else { curve.fitted(crate::import::MAX_GRADIENT_STOPS).into_owned() })
}

/// Is a gradient of `kind` written as a radial one: a radial gradient, or a diamond (the ellipse
/// through its square's corners, [`DIAMOND`])?
fn written_radial(kind: GradientKind) -> bool {
    matches!(kind, GradientKind::Radial | GradientKind::Diamond)
}

/// The first name of `Gradient`, `Gradient 2`, `Gradient 3`, … numbered `next` or higher for
/// which `taken` returns false (`Gradient` is number 1); `next` is set past it.
pub(in crate::ai_editing) fn free_gradient_name(next: &mut u64, mut taken: impl FnMut(&str) -> bool) -> String {
    loop {
        let name = if *next <= 1 { String::from("Gradient") } else { format!("Gradient {next}") };
        *next = next.saturating_add(1);
        if !taken(&name) {
            return name;
        }
    }
}

/// The stops of `g` in order of their place on the ramp.
fn sorted_stops(g: &Gradient) -> Vec<&GradientStop> {
    let mut stops: Vec<&GradientStop> = g.stops.iter().collect();
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    stops
}

impl Writer<'_> {
    /// Each gradient swatch's definition, named after the swatch, before the art is written.
    pub(in crate::ai_editing) fn reserve_swatch_gradients(&mut self) {
        let doc = self.doc;
        for sw in doc.swatches_iter() {
            // As `swatch_paint` writes it: one mixed around the hue wheel as the stops on its curve,
            // and the stops of one that repeats or reflects repeated.
            if let Paint::Gradient(g) = &sw.paint
                && let stops = hue_as_stops(&g.gradient)
                && let gradient = stops.unrolled_within(crate::import::MAX_GRADIENT_STOPS)
                && writable_gradient(&gradient)
                && !self.gradients.iter().any(|d| d.name == sw.name)
            {
                self.gradients.push(GradientDef { name: sw.name.clone(), gradient: gradient.into_owned(), swatch: true, used: false });
            }
        }
    }

    /// The name of the definition of `g`: the swatch `swatch` names when it is a gradient swatch
    /// of the same gradient, else a definition of the same gradient that no swatch names, else a
    /// new one ([`free_gradient_name`]: `Gradient`, numbered when taken by another definition, a
    /// swatch or a pattern).
    fn define_gradient(&mut self, g: &Gradient, swatch: Option<&str>) -> String {
        let used = !self.in_palette;
        let found = swatch
            .and_then(|s| self.gradients.iter().position(|d| d.swatch && d.name == s && d.gradient == *g))
            .or_else(|| self.gradients.iter().position(|d| !d.swatch && d.gradient == *g));
        if let Some(d) = found.and_then(|i| self.gradients.get_mut(i)) {
            d.used |= used;
            return d.name.clone();
        }
        let (doc, gradients) = (self.doc, &self.gradients);
        let name = free_gradient_name(&mut self.next_gradient, |name| {
            gradients.iter().any(|d| d.name == name) || doc.swatch(name).is_some() || doc.pattern(name).is_some()
        });
        self.gradients.push(GradientDef { name: name.clone(), gradient: g.clone(), swatch: false, used });
        name
    }

    /// The gradient definitions' state, for [`Self::gradient_rollback`]: their count, which ones the
    /// art uses, and the next name's number.
    pub(super) fn gradient_mark(&self) -> (usize, Vec<bool>, u64) {
        (self.gradients.len(), self.gradients.iter().map(|d| d.used).collect(), self.next_gradient)
    }

    /// The gradient definitions as they were at `mark` ([`Self::gradient_mark`]).
    pub(super) fn gradient_rollback(&mut self, mark: (usize, Vec<bool>, u64)) {
        let (len, used, next) = mark;
        self.gradients.truncate(len);
        for (d, u) in self.gradients.iter_mut().zip(used) {
            d.used = u;
        }
        self.next_gradient = next;
    }

    /// `p` as the paint of a fill or stroke whose gradient is placed on `bounds`: a gradient is
    /// placed there (it then no longer depends on the shape it paints, which may be cut); one the
    /// data can't hold is its average color, with a warning.
    pub(super) fn placed(&mut self, p: &Paint, bounds: Rect) -> Paint {
        self.placed_over(p, bounds, bounds)
    }

    /// [`Self::placed`] for paint that reaches `area`: a gradient that repeats or reflects is
    /// written with its stops repeated over `area`.
    fn placed_over(&mut self, p: &Paint, bounds: Rect, area: Rect) -> Paint {
        let Paint::Gradient(g) = p else { return p.clone() };
        let mut geom = g.resolve(bounds);
        // The data has no spread: a gradient that repeats or reflects is written with its stops
        // repeated over `area` (a conic one, written linear, around its turn).
        let mut g: GradientPaint = (**g).clone();
        if g.gradient.method.is_hue() {
            self.warn(GRADIENT_HUE);
            g.gradient = hue_as_stops(&g.gradient).into_owned();
        }
        if g.gradient.is_spread() {
            self.warn(SPREAD);
            match g.gradient.padded_within(&geom, area, crate::import::MAX_GRADIENT_STOPS) {
                Some(padded) => {
                    g.gradient = padded.gradient;
                    geom = padded.geom;
                }
                None => g.gradient = g.gradient.unrolled_within(crate::import::MAX_GRADIENT_STOPS).into_owned(),
            }
        }
        let finite = [geom.start.x, geom.start.y, geom.end.x, geom.end.y, geom.aspect].iter().all(|v| v.is_finite());
        let radial = written_radial(g.gradient.kind);
        if g.gradient.kind == GradientKind::Freeform {
            self.warn(GRADIENTS);
            return Paint::solid(average(&g.gradient));
        }
        if !writable_gradient(&g.gradient) || !finite || geom.length() < 1e-9 || (radial && geom.aspect <= 1e-9) {
            self.warn(GRADIENT_SHAPES);
            return Paint::solid(average(&g.gradient));
        }
        Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..g }))
    }

    /// The fills and strokes of `n` that paint (outline `bp`), their gradients placed
    /// ([`Self::placed`]): a fill's on the shape's bounds, a stroke's on what it paints, the stops
    /// of one that repeats or reflects repeated over the box the stroke paints, miter spikes
    /// included ([`StrokeLayer::bounds_on`]). A gradient along or across a stroke is its average
    /// color (the stroke is written as its outline).
    pub(super) fn items(&mut self, n: &vectorcraft_doc::Node, bp: &BezPath) -> Vec<AppearanceItem> {
        let bounds = bp.bounding_box();
        super::visible_items(n)
            .into_iter()
            .map(|i| match i {
                AppearanceItem::Fill(f) => AppearanceItem::Fill(FillLayer { paint: self.placed(&f.paint, bounds), ..f.clone() }),
                AppearanceItem::Stroke(s) => {
                    let paint = match s.path_gradient() {
                        Some(g) => {
                            self.warn(GRADIENTS);
                            Paint::solid(average(&g.gradient))
                        }
                        None => {
                            let spread = matches!(&s.paint, Paint::Gradient(g) if g.gradient.is_spread());
                            let reach = if spread { s.bounds_on(bp, bounds) } else { bounds };
                            self.placed_over(&s.paint, s.paint_bounds(bounds), reach)
                        }
                    };
                    AppearanceItem::Stroke(StrokeLayer { paint, ..s.clone() })
                }
            })
            .collect()
    }

    /// Can paint `p` be written as the paint of a path object: a color, a placed gradient, or a
    /// pattern [`Self::pattern_writable`]?
    pub(super) fn direct(&self, p: &Paint) -> bool {
        match p {
            Paint::Solid { .. } => true,
            Paint::Gradient(g) => g.geom.is_some() && writable_gradient(&g.gradient),
            Paint::Pattern { pattern, .. } => self.pattern_writable(pattern),
            Paint::None => false,
        }
    }

    /// Does a plain line of `s`'s paint draw it ([`Self::direct`], and the line style operators can
    /// write it)?
    pub(super) fn direct_stroke(&self, s: &StrokeLayer) -> bool {
        self.direct(&s.paint) && stroke::is_plain(s)
    }

    /// `n`'s paint `items` as one path object paints: no paint, a fill, a plain line, or both, the
    /// line over the fill; `None` when it has other paint.
    pub(super) fn plain_paint<'i>(&self, items: &'i [AppearanceItem]) -> Option<(Option<&'i FillLayer>, Option<&'i StrokeLayer>)> {
        match items {
            [] => Some((None, None)),
            [AppearanceItem::Fill(f)] if self.direct(&f.paint) => Some((Some(f), None)),
            [AppearanceItem::Stroke(s)] if self.direct_stroke(s) => Some((None, Some(s))),
            [AppearanceItem::Fill(f), AppearanceItem::Stroke(s)] if self.direct(&f.paint) && self.direct_stroke(s) => Some((Some(f), Some(s))),
            _ => None,
        }
    }

    /// The painting operator `op` of a subpath (`closed` or open), with the gradient instances of
    /// the fill (`fill`) and the stroke (`stroke`) around it when it is the last subpath of its
    /// object: `Bb` … `op` `flag BB`, the stroke's `1 Bb` … after the fill's `BB`; the flag is 0, or
    /// with a stroke 2 (closed) or 1 (open). `stroked`: the object has a stroke. A clipping path's
    /// instances (`clip`) don't issue a clip of their own. `path`: the path the operator paints (all
    /// its subpaths), in the document.
    pub(super) fn painted(
        &mut self,
        op: char,
        closed: bool,
        gradients: (Option<&GradientPaint>, Option<&GradientPaint>),
        stroked: bool,
        clip: bool,
        path: &BezPath,
    ) {
        let op = if closed { op } else { op.to_ascii_uppercase() };
        let flag = match (stroked, closed) {
            (false, _) => "0 BB",
            (true, true) => "2 BB",
            (true, false) => "1 BB",
        };
        let (fill, stroke) = gradients;
        if let Some(f) = fill {
            self.instance(f, false, clip, path);
            if stroke.is_some() {
                self.line(if closed { "2 BB" } else { "1 BB" });
            }
        }
        if let Some(s) = stroke {
            self.instance(s, true, clip, path);
        }
        self.body.push(op);
        self.body.push('\n');
        if fill.is_some() || stroke.is_some() {
            self.line(flag);
        }
    }

    /// A gradient instance's start (placed gradient `g`, on the stroke when `stroke`, on `path` in
    /// the document), before its painting operator: `Bb` (`1 Bb` on the stroke); a radial
    /// gradient's focal point `x y angle fraction Bh` (its offset from the center in the space of the
    /// gradient's matrices, then its angle and fraction as [`placement`] gives them; `0 0 0 0 Bh` at
    /// the center); `flag (name) x y angle length a b c d tx ty 1 Bg` (flag 1, or 0 for a clipping
    /// path; the rest as [`placement`] gives them); then the gradient's place: a linear one's `a b c d tx ty Xm` (the
    /// unit vector onto its whole ramp), its cap before the first stop `… Bc`, a `… Bm` for each pair
    /// of stops next to each other (the unit vector onto the part between them) and its cap past the
    /// last stop; a radial one's `… Bm` (the unit circle onto its extent ellipse).
    fn instance(&mut self, g: &GradientPaint, stroke: bool, clip: bool, path: &BezPath) {
        let Some(geom) = g.geom else { return };
        self.line(if stroke { "1 Bb" } else { "Bb" });
        self.instance_head(g, clip, path);
        let m = self.grad_space;
        let u = geom.end - geom.start;
        let radial = written_radial(g.gradient.kind);
        // The unit vector along the gradient, and across it as long.
        let frame = |from: f64, to: f64| {
            let p = geom.start + u * from;
            m * Affine::new([u.x * (to - from), u.y * (to - from), -u.y, u.x, p.x, p.y])
        };
        let mut matrix = |a: Affine, op: &str| {
            ps::push_nums(&mut self.body, &a.as_coeffs());
            self.body.push_str(op);
            self.body.push('\n');
        };
        if radial {
            matrix(m * geom.unit_frame().unwrap_or(Affine::new([u.x, u.y, -u.y, u.x, geom.start.x, geom.start.y])), "Bm");
            return;
        }
        matrix(frame(0.0, 1.0), "Xm");
        let offsets: Vec<f64> = sorted_stops(&g.gradient).iter().map(|s| f64::from(s.offset)).collect();
        let len = u.hypot().max(1e-9);
        let (first, last) = (offsets.first().copied().unwrap_or(0.0), offsets.last().copied().unwrap_or(1.0));
        matrix(frame(first - CAP / len, first), "Bc");
        for pair in offsets.windows(2) {
            if let [a, b] = pair {
                matrix(frame(*a, *b), "Bm");
            }
        }
        matrix(frame(last, last + CAP / len), "Bc");
    }

    /// A gradient instance's focal point (`Bh`, radial gradients only) and its `Bg` line, as
    /// [`Self::instance`] writes them; also what a fill's art style holds after its `Bb`.
    pub(super) fn instance_head(&mut self, g: &GradientPaint, clip: bool, path: &BezPath) {
        let Some(mut geom) = g.geom else { return };
        if g.gradient.kind != GradientKind::Radial {
            // Only a radial gradient has a focal point (a plug-in's or a file's other kinds can
            // carry one that draws nothing): a diamond's circle stays centered.
            geom.focal = None;
        }
        let name = self.define_gradient(&g.gradient, g.swatch.as_deref());
        let m = self.grad_space;
        let radial = written_radial(g.gradient.kind);
        let place = placement(&geom, radial, &(self.canvas * path.clone()), self.canvas);
        if radial {
            let c = m * geom.start;
            let off = geom.focal.map_or(Vec2::ZERO, |f| m * f - c);
            let (angle, fraction) = if off.hypot() > 1e-9 { place.focal } else { (0.0, 0.0) };
            let off = if fraction > 0.0 { off } else { Vec2::ZERO };
            ps::push_nums(&mut self.body, &[off.x, off.y, angle, fraction]);
            self.line("Bh");
        }
        let _ = write!(self.body, "{} {} ", u8::from(!clip), ps::bytes(name.as_bytes()));
        ps::push_nums(&mut self.body, &place.bg);
        ps::push_nums(&mut self.body, &place.transform.as_coeffs());
        self.line("1 Bg");
    }

    /// A gradient definition: `%AI5_BeginGradient: (name)`, `(name) type stops Bd` (type 0 linear,
    /// 1 radial), `[`, each stop on a `%_` line (`color style opacity 6 midpoint rampPoint Bs`), a
    /// linear gradient's from the last to the first and a radial one's from the first to the last,
    /// `BD`, `%AI5_EndGradient`.
    fn gradient_def(&mut self, s: &mut String, d: &GradientDef) {
        if d.gradient.dither || !d.gradient.method.is_classic() {
            self.warn(GRADIENT_MIX);
        }
        if d.gradient.kind == GradientKind::Conic {
            self.warn(CONIC);
        }
        if d.gradient.kind == GradientKind::Diamond {
            self.warn(DIAMOND);
        }
        let name = ps::bytes(d.name.as_bytes());
        let radial = written_radial(d.gradient.kind);
        let _ = writeln!(s, "%AI5_BeginGradient: {name}\n{name} {} {} Bd\n[", u8::from(radial), d.gradient.stops.len());
        let mut stops = sorted_stops(&d.gradient);
        if !radial {
            stops.reverse();
        }
        for stop in stops {
            s.push_str("%_");
            self.stop_color(s, stop);
            ps::push_nums(s, &[f64::from(unit(stop.opacity)), 6.0, f64::from(stop.midpoint) * 100.0, f64::from(stop.offset) * 100.0]);
            s.push_str("Bs\n");
        }
        s.push_str("BD\n%AI5_EndGradient\n");
    }

    /// A stop's color and its style: gray `lightness 0`, CMYK `c m y k 1`, RGB (and Lab, as RGB)
    /// `c m y k r g b 2` with its CMYK equivalent; a spot color its full color's CMYK equivalent,
    /// then its RGB values when it isn't CMYK, its name and how much lighter than the full color,
    /// then `3` (CMYK), or `1 4` (RGB, and Lab as RGB). RGB values are an RGB color's own (in the
    /// working RGB space) and a Lab color's display RGB ([`ai_ops::rgb_values`]). An RGB color's CMYK
    /// equivalent is converted from its values as written ([`ai_ops::as_written`]), a Lab color's
    /// from its Lab values, and a spot color's CMYK equivalent and RGB values both come from its full
    /// color as the palette writes it.
    fn stop_color(&mut self, s: &mut String, stop: &GradientStop) {
        let doc = self.doc;
        let intent = self.cms.settings().intent;
        if let Some(name) = stop.swatch.as_deref().filter(|n| *n != REGISTRATION)
            && let Some(sw) = doc.swatch(name)
            && let Paint::Solid { color: full, swatch: None, .. } = sw.paint
        {
            if sw.spot {
                let full = ai_ops::as_written(full);
                let cmyk = self.cms.to_cmyk(&doc.linked_color(full, true), intent).map(unit);
                ps::push_nums(s, &cmyk.map(f64::from));
                let rgb = !matches!(full, Color::Cmyk { .. });
                if rgb {
                    ps::push_nums(s, &ai_ops::rgb_values(full).map(f64::from));
                }
                s.push_str(&ps::bytes(name.as_bytes()));
                s.push(' ');
                ps::push_num(s, f64::from(1.0 - unit(stop.tint)));
                s.push_str(if rgb { " 1 4 " } else { " 3 " });
                return;
            }
            if sw.global {
                self.warn(STOP_LINKS);
            }
        }
        match stop.color {
            Color::Gray { k } => {
                ps::push_num(s, f64::from(1.0 - unit(k)));
                s.push_str(" 0 ");
            }
            Color::Cmyk { c, m, y, k } => {
                ps::push_nums(s, &[c, m, y, k].map(|v| f64::from(unit(v))));
                s.push_str("1 ");
            }
            color @ (Color::Rgb { .. } | Color::Lab { .. }) => {
                let source = if matches!(color, Color::Rgb { .. }) { ai_ops::as_written(color) } else { color };
                let cmyk = self.cms.to_cmyk(&source, intent).map(unit);
                ps::push_nums(s, &cmyk.map(f64::from));
                ps::push_nums(s, &ai_ops::rgb_values(color).map(f64::from));
                s.push_str("2 ");
            }
        }
    }

    // ---------- patterns ----------

    /// Can pattern `name` be written as a definition and painted with from here: the document has
    /// it, its tiles make a grid, its art is within its tile and holds something, and it isn't used
    /// in another pattern's tile or a symbol's art (the reader keeps no symbol whose art paints with
    /// a pattern) or nested more than [`MAX_NEST`] deep?
    pub(super) fn pattern_writable(&self, name: &str) -> bool {
        if !self.defining.is_empty() || self.nest >= MAX_NEST || self.in_symbol {
            return false;
        }
        if self.patterns.iter().any(|p| p.name == name) {
            return true;
        }
        self.doc.pattern(name).is_some_and(tile_holds_art)
    }

    /// Write pattern `name`'s definition, once: `%AI3_BeginPattern: (name)`, `(name) llx lly urx
    /// ury` (the tile, y up), the tile's box (an unpainted path whose dictionary marks it,
    /// `AIPattern_Editor_Backing_Tile_Rect`) and the art on `%_` lines, `E`, `%AI3_EndPattern`.
    /// False when it can't be ([`Self::pattern_writable`]), or when its box and art don't all fit
    /// within the object limit ([`Self::take_objects`]).
    fn define_pattern(&mut self, name: &str) -> bool {
        if !self.pattern_writable(name) {
            return false;
        }
        let used = !self.in_palette;
        if let Some(p) = self.patterns.iter_mut().find(|p| p.name == name) {
            p.used |= used;
            return true;
        }
        let doc = self.doc;
        // The tile's box is an object of the definition ([`Self::take_objects`]).
        let (before, bound) = (self.charges(), self.bound.len());
        let Some(def) = doc.pattern(name).filter(|_| self.take_objects(1)) else { return false };
        self.defining.push(name.to_string());
        // The art is written in the pattern's space, y up, from the state a definition starts in.
        let saved = (
            std::mem::take(&mut self.body),
            (self.to_art, self.grad_space, self.canvas),
            self.state.take(),
            self.gs.replace(Xy::OPAQUE),
            (self.clip.take(), std::mem::take(&mut self.shown)),
            self.block_end,
        );
        // `Bg` places a gradient in a pattern's tile as on the canvas, mirrored top to bottom: its
        // placement is worked out in the tile's space turned y up.
        (self.to_art, self.grad_space, self.canvas, self.block_end) = (Affine::FLIP_Y, self.tile_grad_space, Affine::FLIP_Y, usize::MAX);
        // The tile's box first, unpainted and marked as the box: the tile is that box.
        self.state(false, false);
        self.line("0 XR");
        if let Some(sp) = vectorcraft_geom::shapes::rectangle(def.tile).subpaths.first() {
            crate::ai_ops::subpath(&mut self.body, self.to_art, sp);
        }
        self.line("n\n%_/ArtDictionary :\n%_1 /Bool (AIPattern_Editor_Backing_Tile_Rect) ,\n%_;\n%_");
        self.nest += 1;
        self.hidden += 1;
        // The tile's effects are its own, not those of a drawn look that first uses it.
        let in_look = std::mem::take(&mut self.in_look);
        for a in &def.art {
            self.node(a);
        }
        self.in_look = in_look;
        self.hidden -= 1;
        self.nest -= 1;
        let art;
        (art, (self.to_art, self.grad_space, self.canvas), self.state, self.gs, (self.clip, self.shown), self.block_end) = saved;
        let art = std::mem::replace(&mut self.body, art);
        self.defining.pop();
        if self.left_out_since(before) {
            self.give_back(before, &art);
            self.bound.truncate(bound);
            return false;
        }
        self.keep_since(before);
        let tile = Affine::FLIP_Y.transform_rect_bbox(def.tile);
        let quoted = ps::bytes(name.as_bytes());
        let mut text = format!("%AI3_BeginPattern: {quoted}\n{quoted} ");
        ps::push_nums(&mut text, &[tile.x0, tile.y0, tile.x1, tile.y1]);
        text.pop();
        text.push('\n');
        super::hide(&art, &mut text);
        text.push_str("E\n%AI3_EndPattern\n");
        self.patterns.push(PatternOut { name: name.to_string(), text, used });
        true
    }

    /// Set the fill (or with `stroke`, the stroke) to pattern `name` placed by `xf` (the pattern's
    /// space → the document): `(name) 0 0 1 1 0 0 0 0 0 [matrix] p` (`P` for the stroke), the
    /// matrix taking the pattern's space (y up, the tile's lower left corner its origin) to the
    /// art's. A pattern that can't be defined paints mid gray.
    pub(super) fn pattern_paint(&mut self, name: &str, xf: Affine, stroke: bool) {
        let height = self.doc.pattern(name).map(|d| d.tile.height());
        let Some(h) = height.filter(|_| self.define_pattern(name)) else {
            self.warn(super::PATTERNS);
            return self.plain_color(Color::gray(0.5), stroke);
        };
        let m = self.to_art * xf * Affine::translate((0.0, h)) * Affine::FLIP_Y;
        let _ = writeln!(self.body, "{} 0 0 1 1 0 0 0 0 0 {} {}", ps::bytes(name.as_bytes()), ps::matrix(m), if stroke { 'P' } else { 'p' });
    }

    /// The definitions, in the setup before the palette: the gradients the art uses (`n Bn`, then
    /// each), those only the palette uses between `%AI5_Begin_NonPrinting`, `Np` and
    /// `%AI5_End_NonPrinting--`, then the patterns in the same way (a pattern after those its art
    /// uses).
    pub(in crate::ai_editing) fn definitions(&mut self, s: &mut String) {
        let defs = std::mem::take(&mut self.gradients);
        let (used, unused): (Vec<&GradientDef>, Vec<&GradientDef>) = defs.iter().partition(|d| d.used);
        let _ = writeln!(s, "{} Bn", used.len());
        for d in &used {
            self.gradient_def(s, d);
        }
        if !unused.is_empty() {
            let _ = writeln!(s, "%AI5_Begin_NonPrinting\nNp\n{} Bn", unused.len());
            for d in &unused {
                self.gradient_def(s, d);
            }
            s.push_str("%AI5_End_NonPrinting--\n");
        }
        self.gradients = defs;
        for p in self.patterns.iter().filter(|p| p.used) {
            s.push_str(&p.text);
        }
        if self.patterns.iter().any(|p| !p.used) {
            s.push_str("%AI5_Begin_NonPrinting\nNp\n");
            for p in self.patterns.iter().filter(|p| !p.used) {
                s.push_str(&p.text);
            }
            s.push_str("%AI5_End_NonPrinting--\n");
        }
    }

    /// A gradient swatch's or pattern swatch's paint in the palette: `Bb`, a radial gradient's
    /// `0 0 0 0 Bh`, `2 (name) 0 0 0 1 1 0 0 1 0 0 1 Bg`, `0 BB`; a pattern's
    /// `(name) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p`. False for other paint, and for paint that can't
    /// be written.
    pub(super) fn swatch_paint(&mut self, s: &mut String, sw: &vectorcraft_color::swatch::Swatch) -> bool {
        // A swatch has no placement: one that repeats or reflects holds its stops repeated along
        // its vector, and one mixed around the hue wheel the stops on its curve.
        let stops = match &sw.paint {
            Paint::Gradient(g) => Some(hue_as_stops(&g.gradient)),
            _ => None,
        };
        let unrolled = stops.as_deref().map(|g| g.unrolled_within(crate::import::MAX_GRADIENT_STOPS));
        match &sw.paint {
            Paint::Gradient(g) if unrolled.as_deref().is_some_and(writable_gradient) => {
                if g.gradient.method.is_hue() {
                    self.warn(GRADIENT_HUE);
                }
                if g.gradient.is_spread() {
                    self.warn(SPREAD);
                }
                let swatch = Some(sw.name.as_str());
                let name = self.define_gradient(unrolled.as_deref().unwrap_or(&g.gradient), swatch);
                s.push_str("Bb\n");
                if written_radial(g.gradient.kind) {
                    s.push_str("0 0 0 0 Bh\n");
                }
                let _ = writeln!(s, "2 {} 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB", ps::bytes(name.as_bytes()));
                true
            }
            Paint::Pattern { pattern, xf } if self.define_pattern(pattern) => {
                if *xf != Affine::IDENTITY {
                    self.warn(SWATCH_PLACEMENT);
                }
                let _ = writeln!(s, "{} 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p", ps::bytes(pattern.as_bytes()));
                true
            }
            _ => false,
        }
    }
}

/// Where `Bg` places a gradient: its `x y angle length` operands, its matrix, and a radial
/// gradient's focal point as `Bh`'s angle and fraction ([`placement`]).
struct Placement {
    bg: [f64; 4],
    transform: Affine,
    focal: (f64, f64),
}

impl Placement {
    /// The gradient's place on the path before it is moved: `0 0 0 1`, the identity.
    const UNMOVED: Placement = Placement { bg: [0.0, 0.0, 0.0, 1.0], transform: Affine::IDENTITY, focal: (0.0, 0.0) };
}

/// The `Bg` operands that place gradient `geom` (`radial` or linear) on `path`, both in the canvas's
/// space (y down; `canvas` takes the document there; in a pattern's tile, the tile's space turned y
/// up). `Bg` places a gradient in the space its matrix takes to the canvas's, on the path as it lies
/// there:
///
/// - a linear one's ramp runs at `angle` degrees (counterclockwise, y up) from `x` times the path's
///   extent along that angle past where the path starts along it, `length` times that extent long
///   (`y` doesn't move it);
/// - a radial one's center lies `x` times the width and `y` times the height (y down) of the path's
///   bounds from their center, and its radius is `length` times √((dx² + dy²) / 2), `dx` and `dy` the
///   center's distances to the right side and to the bottom of the bounds, whether or not those are
///   the nearer sides; its focal point lies at `Bh`'s angle (y up), its distance from the center a
///   fraction of the radius.
///
/// A linear gradient is written with the identity, a radial one with the matrix that takes the
/// unit circle onto its extent ellipse at the origin (so that its circle there is the unit circle).
/// [`Placement::UNMOVED`] when the path has no extent along the gradient or the gradient has none.
fn placement(geom: &GradientGeom, radial: bool, path: &BezPath, canvas: Affine) -> Placement {
    let s = canvas * geom.start;
    let u = canvas * geom.end - s;
    if !radial {
        let turn = Affine::rotate(-u.atan2());
        let along = (turn * path.clone()).bounding_box();
        let extent = along.width();
        if !(extent > 1e-9 && extent.is_finite() && u.hypot() > 1e-9) {
            return Placement::UNMOVED;
        }
        let bg = [((turn * s).x - along.x0) / extent, 0.0, (-u.y).atan2(u.x).to_degrees(), u.hypot() / extent];
        return Placement { bg, ..Placement::UNMOVED };
    }
    // The unit circle's image: `u` along the vector, `v` across it (the aspect ratio), turning the same way.
    let across = geom.end - geom.start;
    let mut v = canvas * (geom.start + Vec2::new(-across.y, across.x) * geom.aspect) - s;
    if u.cross(v) < 0.0 {
        v = -v;
    }
    let m = Affine::new([u.x, u.y, v.x, v.y, 0.0, 0.0]);
    let det = m.determinant().abs();
    if det.is_nan() || det <= 1e-12 {
        return Placement::UNMOVED;
    }
    let back = m.inverse();
    let b = (back * path.clone()).bounding_box();
    let c = back * s;
    let (dx, dy) = (b.x1 - c.x, b.y1 - c.y);
    let unit = ((dx * dx + dy * dy) / 2.0).sqrt();
    if !(b.width() > 1e-9 && b.height() > 1e-9 && unit > 1e-9 && unit.is_finite()) {
        return Placement::UNMOVED;
    }
    let off = geom.focal.map_or(Vec2::ZERO, |f| back * (canvas * f) - c);
    let focal = if off.hypot() > 1e-9 { ((-off.y).atan2(off.x).to_degrees(), off.hypot()) } else { (0.0, 0.0) };
    let bg = [(c.x - b.center().x) / b.width(), (c.y - b.center().y) / b.height(), 0.0, 1.0 / unit];
    Placement { bg, transform: m, focal }
}

/// Do `def`'s tiles make a grid, with its art (something) within the tile? Art past the tile would
/// be cut off at the tile's edge.
fn tile_holds_art(def: &PatternDef) -> bool {
    let t = def.tile;
    let tile_ok = [t.x0, t.y0, t.x1, t.y1].iter().all(|v| v.is_finite()) && t.width() > 0.0 && t.height() > 0.0;
    tile_ok
        && def.tile_type == TileType::Grid
        && def.art_bounds().is_some_and(|b| {
            let room = t.inflate(1e-6, 1e-6);
            b.x0 >= room.x0 && b.y0 >= room.y0 && b.x1 <= room.x1 && b.y1 <= room.y1
        })
}
