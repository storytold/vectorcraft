//! Blending in CMYK documents.
//!
//! In a CMYK document blend modes and opacity composite inks, not screen colours: Multiply of cyan
//! over magenta prints both inks (blue, C100 M100 through the CMYK profile), where screen RGB would
//! give the product of the two display colours. The renderer composites additive values, so a CMYK
//! document that shows transparency is drawn twice, once per ink plane
//! ([`vectorcraft_color::blend::cmyk_planes`]): every colour painted as its complemented C, M and
//! Y ([`Ink::Cmy`]), then as its complemented K, a grey ([`Ink::K`]). Blend modes, opacity,
//! groups, knockouts and effects composite the planes exactly as they composite RGB, which is CMYK
//! blending. [`compose`] reads each pixel's inks back and shows them through the working CMYK
//! profile (a cached 4-D lookup table). Opacity masks keep taking their luminance from screen
//! colours; RGB colours and placed images are separated into the working CMYK space first.
//!
//! RGB documents, and CMYK documents without transparency, draw screen colours ([`Ink::Display`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vectorcraft_color::blend::{cmyk_planes, planes_cmyk};
use vectorcraft_color::cms::{self, Cms, CmykLut, ProofLut, lab};
use vectorcraft_color::{Color, Gradient};
use vectorcraft_doc::cmyk::Inks;
use vectorcraft_doc::{ColorMode, Document, ImageBlob};
use vello_cpu::Pixmap;
use vello_cpu::color::PremulRgba8;
use vello_cpu::peniko;

use crate::images::{self, ImageKey};
use crate::{RenderOptions, Renderer};

/// What a frame paints for colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Ink {
    /// Screen colours.
    #[default]
    Display,
    /// The complemented C, M and Y inks.
    Cmy,
    /// The complemented K ink, as a grey.
    K,
    /// Screen colours, with pure CMYK black as RGB black ([`RenderOptions::rich_black`]).
    RichBlack,
    /// Source-color adjustment before the display/ink conversion. Float bits keep cache keys exact.
    Darkened(InkSpace, u32, bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InkSpace {
    Display,
    Cmy,
    K,
    RichBlack,
}

impl Ink {
    pub(crate) fn base(self) -> Self {
        match self {
            Self::Darkened(InkSpace::Display, _, _) => Self::Display,
            Self::Darkened(InkSpace::Cmy, _, _) => Self::Cmy,
            Self::Darkened(InkSpace::K, _, _) => Self::K,
            Self::Darkened(InkSpace::RichBlack, _, _) => Self::RichBlack,
            other => other,
        }
    }

    pub(crate) fn index(self) -> usize {
        match self.base() {
            Self::Cmy => 1,
            Self::K => 2,
            Self::RichBlack => 3,
            _ => 0,
        }
    }

    fn darkness(self) -> Option<f32> {
        match self {
            Self::Darkened(_, bits, _) => Some(f32::from_bits(bits)),
            _ => None,
        }
    }

    fn black_endpoint(self) -> bool {
        matches!(self, Self::Darkened(_, _, true))
    }

    pub(crate) fn darkened(self, amount: f32) -> Self {
        let amount = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
        let black = self.black_endpoint() || amount >= 0.9999;
        let amount = (self.darkness().unwrap_or(0.0) + amount).min(1.0);
        if amount == 0.0 {
            return self.base();
        }
        let space = match self.base() {
            Self::Cmy => InkSpace::Cmy,
            Self::K => InkSpace::K,
            Self::RichBlack => InkSpace::RichBlack,
            _ => InkSpace::Display,
        };
        Self::Darkened(space, amount.to_bits(), black)
    }

    /// Fixed changes use the stored color's model, before profile conversion or interpolation.
    fn darkened_color(c: &Color, amount: f32, black: bool) -> Color {
        if black {
            return Color::gray(1.0);
        }
        match *c {
            Color::Cmyk { c, m, y, k } => Color::cmyk(c, m, y, (k + amount).min(1.0)),
            Color::Gray { k } => Color::gray((k + amount).min(1.0)),
            _ => {
                let [r, g, b] = c.to_rgb();
                Color::rgb((r - amount).max(0.0), (g - amount).max(0.0), (b - amount).max(0.0))
            }
        }
    }

    /// Transform original stops before a gradient expands its midpoint/perceptual colors.
    pub(crate) fn gradient(self, g: &Gradient) -> Option<Gradient> {
        let amount = self.darkness()?;
        let mut g = g.clone();
        for stop in &mut g.stops {
            stop.color = Self::darkened_color(&stop.color, amount, self.black_endpoint());
        }
        Some(g)
    }

    /// The three values painted for colour `c`.
    pub(crate) fn rgb(self, c: &Color) -> [f32; 3] {
        if let Some(amount) = self.darkness() {
            return self.base().rgb(&Self::darkened_color(c, amount, self.black_endpoint()));
        }
        match (self, *c) {
            (Ink::RichBlack, Color::Cmyk { c, m, y, k }) if is_pure_black([c, m, y, k]) => [0.0; 3],
            (Ink::Display | Ink::RichBlack, _) => c.to_rgb(),
            _ => self.plane(inks(c)),
        }
    }

    /// [`Self::rgb`] as a paint, at `alpha`.
    pub(crate) fn color(self, c: &Color, alpha: f32) -> peniko::Color {
        let [r, g, b] = self.rgb(c);
        peniko::Color::from_rgba8(q(r), q(g), q(b), q(alpha))
    }

    /// A fixed screen colour (pasteboard, guides, tile edges) as a paint.
    pub(crate) fn fixed(self, rgba: [u8; 4]) -> peniko::Color {
        let [r, g, b, a] = self.fixed8(rgba);
        peniko::Color::from_rgba8(r, g, b, a)
    }

    fn fixed8(self, [r, g, b, a]: [u8; 4]) -> [u8; 4] {
        match self.base() {
            Ink::Display | Ink::RichBlack => [r, g, b, a],
            _ => {
                let lab = lab::srgb_to_lab([r, g, b].map(|v| v as f32 / 255.0));
                let [r, g, b] = self.plane(inks(&Color::Lab { l: lab.l, a: lab.a, b: lab.b })).map(q);
                [r, g, b, a]
            }
        }
    }

    fn plane(self, inks: [f32; 4]) -> [f32; 3] {
        cmyk_planes(inks)[(self.base() == Ink::K) as usize]
    }
}

fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Black ink alone at 100%: what Appearance of Black calls pure CMYK black.
fn is_pure_black([c, m, y, k]: [f32; 4]) -> bool {
    c <= 0.0 && m <= 0.0 && y <= 0.0 && k >= 1.0
}

impl Renderer {
    /// Whether `doc` is drawn by ink planes: a CMYK document that shows transparency, unless drawn
    /// in Outline view or Separations Preview (which paints plates).
    pub(crate) fn blends_in_cmyk(&mut self, doc: &Document, opts: &RenderOptions) -> bool {
        doc.color_mode == ColorMode::Cmyk
            && !opts.outline
            && opts.proof.as_ref().is_none_or(|p| p.separations.is_none())
            && self.shows_transparency(doc)
    }

    /// Whether `doc`'s layers, symbol art or pattern art show transparency
    /// ([`vectorcraft_doc::Node::shows_transparency`]). The answer is kept with that art (see
    /// [`Self::transparency`]) and found again only when the document holds other art.
    fn shows_transparency(&mut self, doc: &Document) -> bool {
        let art = || doc.layers.iter().chain(doc.symbols.iter().map(|s| &s.art)).chain(doc.patterns.iter().flat_map(|p| &p.art));
        if let Some((held, shows)) = &self.transparency
            && held.len() == art().count()
            && held.iter().zip(art()).all(|(h, a)| std::ptr::eq(h.as_ptr(), Arc::as_ptr(a)))
        {
            return *shows;
        }
        let shows = art().any(|a| a.shows_transparency());
        self.transparency = Some((art().map(Arc::downgrade).collect(), shows));
        shows
    }
}

/// The inks of `c` in the working CMYK space: CMYK as it is, grey as black ink, RGB and Lab
/// separated with the colour settings' intent (cached: separating searches the profile).
fn inks(c: &Color) -> [f32; 4] {
    match *c {
        Color::Cmyk { c, m, y, k } => return [c, m, y, k],
        Color::Gray { k } => return [0.0, 0.0, 0.0, k],
        Color::Rgb { .. } | Color::Lab { .. } => {}
    }
    let key = c.key();
    let cms = cms::active();
    SEPARATED.with(|s| {
        let (owner, seen) = &mut *s.borrow_mut();
        // Large enough for the many colours of gradient meshes and blends, which otherwise cleared
        // it over and over and separated every colour through the profile again.
        if !owner.as_ref().is_some_and(|o| Arc::ptr_eq(o, &cms)) || seen.len() >= 1 << 16 {
            seen.clear();
            *owner = Some(cms.clone());
        }
        *seen.entry(key).or_insert_with(|| cms.to_cmyk(c, cms.settings().intent))
    })
}

/// Separated colours, for the colour settings they were separated with.
type Separated = (Option<Arc<Cms>>, HashMap<[u32; 5], [f32; 4]>);

thread_local! {
    static SEPARATED: RefCell<Separated> = RefCell::default();
}

/// Inks → display sRGB through the working CMYK profile: a 17⁴ lookup table of XYZ, interpolated
/// quadrilinearly (inks mix close to linearly in light, and out-of-gamut colours are clipped
/// after interpolating, not before).
pub(crate) struct InkLut(CmykLut);

impl InkLut {
    fn build(cms: &Cms) -> Self {
        Self(CmykLut::build(|inks| {
            let [x, y, z] = lab::lab_to_xyz(cms.cmyk_display_lab(inks));
            [x, y, z, 0.0]
        }))
    }

    fn apply(&self, inks: [f32; 4]) -> [f32; 3] {
        let [x, y, z, _] = self.0.apply(inks);
        lab::xyz_to_srgb([x, y, z])
    }
}

/// A table built for the colour settings it was built with, and its generation (see [`cached`]).
type Cached<T> = Mutex<Option<(Arc<Cms>, Arc<T>, u64)>>;

/// The last table generation handed out: images made with a table are cached under its
/// generation (see [`crate::images`]), and none is ever reused.
static GENERATION: AtomicU64 = AtomicU64::new(0);

static DISPLAY: Cached<InkLut> = Mutex::new(None);
/// Screen colours → [`Ink::Cmy`] and [`Ink::K`] values (placed images).
static SEPARATE: Cached<[ProofLut; 2]> = Mutex::new(None);

/// The table in `slot` for the active colour settings, built by `build` when they changed, and
/// its generation.
fn cached<T>(slot: &Cached<T>, build: impl FnOnce(&Cms) -> T) -> (Arc<T>, u64) {
    let cms = cms::active();
    let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
    match &*s {
        Some((owner, t, g)) if Arc::ptr_eq(owner, &cms) => (t.clone(), *g),
        _ => {
            let t = Arc::new(build(&cms));
            let g = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
            *s = Some((cms, t.clone(), g));
            (t, g)
        }
    }
}

/// [`compose`] puts at least this many pixels in each band but the last.
const BAND_PIXELS: usize = 1 << 16;

/// The picture of a CMYK document from its ink planes (premultiplied RGBA8 drawn with
/// [`Ink::Cmy`] and [`Ink::K`]): each pixel's inks shown through the working CMYK profile (pure
/// black as RGB black with [`RenderOptions::rich_black`]). Where only the `background` of `opts`
/// shows it keeps its exact screen colour. The pixels are split into bands of one size (the last can
/// be smaller): `threads` of them, or one per [`BAND_PIXELS`] pixels in the frame when that makes
/// fewer, and at least one. This thread composes the first band, and a thread of its own composes
/// each other band (this thread does when that one can't start). Returns the picture and the number
/// of threads that composed it.
pub(crate) fn compose(cmy: &[u8], k: &[u8], opts: &RenderOptions, threads: u16) -> (Vec<u8>, usize) {
    let (lut, _) = cached(&DISPLAY, InkLut::build);
    let bg = opts.background.map(|b| (Ink::Cmy.fixed8(b), Ink::K.fixed8(b), b));
    let mut out = vec![0; cmy.len().min(k.len()) / 4 * 4];
    let pixels = out.len() / 4;
    let bands = (pixels / BAND_PIXELS).clamp(1, usize::from(threads).max(1));
    let band = |o: &mut [u8], p: &[u8], q: &[u8]| compose_band(o, p, q, &lut, bg, opts.rich_black);
    if bands == 1 {
        band(&mut out, cmy, k);
        return (out, 1);
    }
    let size = pixels.div_ceil(bands).max(1) * 4;
    let mut unstarted = vec![];
    let used = std::thread::scope(|s| {
        let mut parts = out.chunks_mut(size).zip(cmy.chunks(size)).zip(k.chunks(size)).enumerate();
        let first = parts.next();
        let mut used = 1;
        for (i, ((o, p), q)) in parts {
            match std::thread::Builder::new().spawn_scoped(s, move || band(o, p, q)) {
                Ok(_) => used += 1,
                Err(_) => unstarted.push(i),
            }
        }
        if let Some((_, ((o, p), q))) = first {
            band(o, p, q);
        }
        used
    });
    for at in unstarted.into_iter().map(|i| i * size) {
        let end = (at + size).min(out.len());
        if let (Some(o), Some(p), Some(q)) = (out.get_mut(at..end), cmy.get(at..end), k.get(at..end)) {
            band(o, p, q);
        }
    }
    (out, used)
}

/// [`compose`] for one band: the pixels of `cmy` and `k` into `out`. `bg` is the background as
/// the [`Ink::Cmy`] and [`Ink::K`] planes paint it, and its screen color.
fn compose_band(out: &mut [u8], cmy: &[u8], k: &[u8], lut: &InkLut, bg: Option<([u8; 4], [u8; 4], [u8; 4])>, rich_black: bool) {
    // Runs of one colour convert once.
    let mut last: Option<([u8; 8], [u8; 4])> = None;
    for ((o, p), q) in out.as_chunks_mut::<4>().0.iter_mut().zip(cmy.as_chunks::<4>().0).zip(k.as_chunks::<4>().0) {
        let key = [p[0], p[1], p[2], p[3], q[0], q[1], q[2], q[3]];
        *o = match (&last, &bg) {
            _ if p[3] == 0 => [0; 4],
            (Some((k0, v)), _) if *k0 == key => *v,
            (_, Some((bc, bk, b))) if p == bc && q == bk => *b,
            _ => {
                let un = |v: u8, a: u8| if a == 0 { 1.0 } else { (v as f32 / a as f32).min(1.0) };
                let a = p[3];
                let inks = planes_cmyk([un(p[0], a), un(p[1], a), un(p[2], a)], un(q[0], q[3]));
                let rgb = if rich_black && is_pure_black(inks) { [0.0; 3] } else { lut.apply(inks) };
                let pm = |v: f32| (v.clamp(0.0, 1.0) * a as f32).round() as u8;
                let v = [pm(rgb[0]), pm(rgb[1]), pm(rgb[2]), a];
                last = Some((key, v));
                v
            }
        };
    }
}

/// Each pixel's ink amounts (C, M, Y, K bytes, 0 = no ink) from its ink planes (premultiplied
/// RGBA8 drawn with [`Ink::Cmy`] and [`Ink::K`]): a plane holds the complement of its inks, so
/// the ink is coverage minus plane value (partly covered pixels carry that much ink).
pub(crate) fn amounts(cmy: &[u8], k: &[u8]) -> Vec<u8> {
    cmy.as_chunks::<4>()
        .0
        .iter()
        .zip(k.as_chunks::<4>().0)
        .flat_map(|(p, q)| [p[3].saturating_sub(p[0]), p[3].saturating_sub(p[1]), p[3].saturating_sub(p[2]), q[3].saturating_sub(q[0])])
        .collect()
}

/// Ink amounts `inks` shown through `lut` (opaque).
fn inks_pixmap(inks: &Inks, lut: &InkLut) -> Option<Pixmap> {
    let (w, h) = (u16::try_from(inks.width).ok()?, u16::try_from(inks.height).ok()?);
    // Runs of one colour convert once.
    let mut last: Option<([u8; 4], PremulRgba8)> = None;
    let data = inks
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| match last {
            Some((k, v)) if k == *p => v,
            _ => {
                let [r, g, b] = lut.apply(p.map(|v| v as f32 / 255.0)).map(q);
                let v = PremulRgba8 { r, g, b, a: 255 };
                last = Some((*p, v));
                v
            }
        })
        .collect();
    Some(Pixmap::from_parts(data, w, h))
}

/// The working CMYK profile's display table (what CMYK images show through) and its generation.
pub(crate) fn display_lut() -> (Arc<InkLut>, u64) {
    cached(&DISPLAY, InkLut::build)
}

/// The pixels of CMYK image `blob`: its inks shown through `lut`. `None` for other images.
pub(crate) fn cmyk_pixmap(blob: &ImageBlob, lut: &InkLut) -> Option<Pixmap> {
    inks_pixmap(&blob.cmyk()?, lut)
}

/// The separation tables of placed images ([`Ink::Cmy`], [`Ink::K`]) and their generation.
fn separation() -> (Arc<[ProofLut; 2]>, u64) {
    cached(&SEPARATE, |cms| {
        let intent = cms.settings().intent;
        [Ink::Cmy, Ink::K].map(|ink| ProofLut::build(|rgb| ink.plane(cms.srgb_to_cmyk(rgb, intent))))
    })
}

/// The key of image pixels `key` painted as `plane` with the separation tables of `generation`.
fn plane_key(key: &ImageKey, plane: Ink, generation: u64) -> ImageKey {
    ImageKey { ink: plane, luts: (key.luts.0, generation), ..key.clone() }
}

/// The key [`ink_image`] gives image pixels `key` painted with `ink`.
pub(crate) fn ink_key(key: &ImageKey, ink: Ink) -> ImageKey {
    if ink.darkness().is_some() {
        let generation = if matches!(ink.base(), Ink::Display | Ink::RichBlack) { 0 } else { separation().1 };
        plane_key(key, ink, generation)
    } else if matches!(ink, Ink::Display | Ink::RichBlack) {
        key.clone()
    } else {
        plane_key(key, ink, separation().1)
    }
}

/// Color-adjusted image pixels, cached separately for each source adjustment and output plane.
fn darkened_image(key: &ImageKey, pm: &Arc<Pixmap>, ink: Ink, amount: f32, blob: Option<&ImageBlob>, touch: bool) -> (ImageKey, Arc<Pixmap>) {
    let want = ink_key(key, ink);
    let hit = {
        let mut s = images::store();
        if touch { s.get(&want) } else { s.peek(&want) }
    };
    if let Some(p) = hit {
        return (want, p);
    }
    let base = ink.base();
    let display = matches!(base, Ink::Display | Ink::RichBlack);
    let raw_cmyk = (key.luts.0 != 0 && key.adjust.is_none() && !key.grey)
        .then(|| blob.and_then(ImageBlob::cmyk))
        .flatten()
        .filter(|i| (i.width, i.height) == (u32::from(pm.width()), u32::from(pm.height())));
    let sep = (!display).then(separation);
    let shown = (display && raw_cmyk.is_some()).then(display_lut);
    let mut out = Pixmap::new(pm.width(), pm.height());
    for (i, (o, p)) in out.data_mut().iter_mut().zip(pm.data()).enumerate() {
        if p.a == 0 {
            continue;
        }
        let rgb = if ink.black_endpoint() {
            base.rgb(&Color::gray(1.0))
        } else if let Some(raw) = raw_cmyk.as_ref().and_then(|v| v.data.as_chunks::<4>().0.get(i)) {
            let mut inks = raw.map(|v| f32::from(v) / 255.0);
            inks[3] = (inks[3] + amount).min(1.0);
            if base == Ink::RichBlack && is_pure_black(inks) {
                [0.0; 3]
            } else if let Some((lut, _)) = &shown {
                lut.apply(inks)
            } else {
                base.plane(inks)
            }
        } else {
            let rgb = [p.r, p.g, p.b].map(|v| (f32::from(v) / f32::from(p.a) - amount).clamp(0.0, 1.0));
            if let Some((luts, _)) = &sep { luts[(base == Ink::K) as usize].apply(rgb) } else { rgb }
        };
        let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * f32::from(p.a)).round() as u8);
        *o = PremulRgba8 { r, g, b, a: p.a };
    }
    let out = images::store().insert(want.clone(), Arc::new(out));
    (want, out)
}

/// Image pixels `pm` (cached as `key`, from `blob` when it is a document's image as it is) as
/// painted with `ink`, cached with the images, and their key.
pub(crate) fn ink_image(key: &ImageKey, pm: &Arc<Pixmap>, ink: Ink, blob: Option<&ImageBlob>, touch: bool) -> (ImageKey, Arc<Pixmap>) {
    if let Some(amount) = ink.darkness() {
        return darkened_image(key, pm, ink, amount, blob, touch);
    }
    if matches!(ink, Ink::Display | Ink::RichBlack) {
        return (key.clone(), pm.clone());
    }
    let (luts, generation) = separation();
    let keyed = |plane: Ink| plane_key(key, plane, generation);
    let want = keyed(ink);
    let hit = {
        let mut s = images::store();
        if touch { s.get(&want) } else { s.peek(&want) }
    };
    if let Some(p) = hit {
        return (want, p);
    }
    // CMYK images (shown through the display table) paint their own inks; the others are separated.
    let size = (pm.width() as u32, pm.height() as u32);
    let cmyk = key.luts.0 != 0 && key.adjust.is_none() && !key.grey;
    let inks = blob.filter(|_| cmyk).and_then(ImageBlob::cmyk).filter(|i| (i.width, i.height) == size);
    let planes = [Ink::Cmy, Ink::K].map(|plane| {
        let mut out = Pixmap::new(pm.width(), pm.height());
        if let Some(inks) = &inks {
            for (o, p) in out.data_mut().iter_mut().zip(inks.data.as_chunks::<4>().0) {
                let [r, g, b] = plane.plane(p.map(|v| v as f32 / 255.0)).map(q);
                *o = PremulRgba8 { r, g, b, a: 255 };
            }
            return Arc::new(out);
        }
        let lut = &luts[(plane == Ink::K) as usize];
        for (o, s) in out.data_mut().iter_mut().zip(pm.data()) {
            let a = s.a;
            if a == 0 {
                continue;
            }
            let un = |v: u8| (v as f32 / a as f32).min(1.0);
            let [r, g, b] = lut.apply([un(s.r), un(s.g), un(s.b)]).map(|v| (v.clamp(0.0, 1.0) * a as f32).round() as u8);
            *o = PremulRgba8 { r, g, b, a };
        }
        Arc::new(out)
    });
    let [cmy, k] = planes;
    let mut s = images::store();
    let cmy = s.insert(keyed(Ink::Cmy), cmy);
    let k = s.insert(keyed(Ink::K), k);
    (want, if ink == Ink::K { k } else { cmy })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darkness_image_cache_keeps_amount_and_coverage() {
        let key = ImageKey {
            content: Arc::from("darkness-rgb-cache-regression"),
            adjust: None,
            grey: false,
            tint: None,
            ink: Ink::Display,
            luts: (0, 0),
            mip: 0,
        };
        let pm =
            Arc::new(Pixmap::from_parts(vec![PremulRgba8 { r: 204, g: 102, b: 51, a: 255 }, PremulRgba8 { r: 102, g: 51, b: 26, a: 128 }], 2, 1));
        let (k25, p25) = ink_image(&key, &pm, Ink::Display.darkened(0.25), None, false);
        let (k50, p50) = ink_image(&key, &pm, Ink::Display.darkened(0.5), None, false);
        assert_ne!(k25, k50);
        assert_eq!((p25.data()[0].r, p25.data()[0].g, p25.data()[0].b), (140, 38, 0));
        assert_eq!((p25.data()[1].r, p25.data()[1].g, p25.data()[1].b, p25.data()[1].a), (70, 19, 0, 128));
        assert_eq!((p50.data()[0].r, p50.data()[0].g, p50.data()[0].b), (77, 0, 0));
        let (_, warm) = ink_image(&key, &pm, Ink::Display.darkened(0.25), None, false);
        assert_eq!(p25.data(), warm.data());
        assert_eq!(pm.data()[0].r, 204, "source pixels remain unchanged");
    }

    #[test]
    fn the_ink_table_matches_the_profile() {
        let cms = cms::active();
        let lut = InkLut::build(&cms);
        let mut worst = 0.0f32;
        // Between the grid points (where interpolation errs most) and on them.
        for i in 0..=20 {
            for j in 0..=20 {
                let inks = [i as f32 / 20.0, j as f32 / 20.0, ((i * 7 + j * 3) % 21) as f32 / 20.0, ((i + j * 5) % 21) as f32 / 20.0];
                let (a, b) = (lut.apply(inks), cms.cmyk_to_srgb(inks, false));
                worst = worst.max((0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max));
            }
        }
        // Within two 8-bit levels: far below a visible difference.
        assert!(worst * 255.0 < 2.5, "worst error {} levels", worst * 255.0);
    }

    #[test]
    fn cmyk_images_show_through_the_profile_and_paint_their_own_inks() {
        use vectorcraft_color::Paint;
        use vectorcraft_doc::{Appearance, ImageObject, Node, NodeKind};
        use vectorcraft_geom::{Affine, Rect, shapes};

        use crate::Renderer;

        let ink = [0u8, 102, 255, 25];
        let mut d = Document::new_with_mode(20.0, 10.0, ColorMode::Cmyk);
        d.images.insert("img".into(), ImageBlob::cmyk_tiff(&Inks::new(2, 2, ink.repeat(4)).unwrap()).unwrap());
        let l = d.layers[0].id;
        let im =
            ImageObject { key: "img".into(), width: 2, height: 2, xf: Affine::scale(5.0), link: None, placement: Default::default(), pixels: None };
        let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
        d.insert(Some(l), 0, n).unwrap();
        // The same inks as a fill beside it.
        let fill = Color::cmyk(0.0, 0.4, 1.0, 25.0 / 255.0);
        let n =
            Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 0.0, 20.0, 10.0)), Appearance::basic(Paint::solid(fill), Paint::None, 0.0));
        d.insert(Some(l), 1, n).unwrap();
        let mut r = Renderer::new();
        let region = Rect::new(0.0, 0.0, 20.0, 10.0);
        let shown = r.render_region(&d, region, 1.0, true).pixels;
        let at = |px: &[u8], x: usize| px[(5 * 20 + x) * 4..][..4].to_vec();
        let (image, fill) = (at(&shown, 5), at(&shown, 15));
        assert!(image.iter().zip(&fill).all(|(a, b)| a.abs_diff(*b) <= 2), "{image:?} vs {fill:?}");
        // CMYK exports draw ink amounts: the image's own.
        let inks = r.render_region_inks(&d, region, 1.0, &RenderOptions::default());
        assert!(at(&inks, 5).iter().zip(ink).all(|(a, b)| a.abs_diff(b) <= 1), "{:?}", at(&inks, 5));
    }

    #[test]
    fn fixed_colours_and_paper() {
        // White is paper on both planes; the display plane keeps screen colours exactly.
        assert_eq!(Ink::Cmy.fixed8([255; 4]), [255; 4]);
        assert_eq!(Ink::K.fixed8([255; 4]), [255; 4]);
        assert_eq!(Ink::Display.fixed8([12, 34, 56, 78]), [12, 34, 56, 78]);
        assert_eq!(Ink::K.rgb(&Color::gray(0.25)), [0.75; 3]);
        assert_eq!(Ink::Cmy.rgb(&Color::cmyk(0.1, 0.2, 0.3, 0.4)), [0.9, 0.8, 0.7]);
    }
}
