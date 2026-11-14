//! Preferences › Performance › Use Monitor Profile for Canvas: the canvas's art, its artboards'
//! paper and the transparency grid are converted from sRGB into the ICC profile of the display
//! that shows most of the main window, as the last step before they are drawn. Translucent art
//! and the anti-aliased edges of art over the paper or the grid are blended with it in sRGB first,
//! and the blend is converted ([`convert_over`]).
//!
//! The desktop app reads the display's profile on a background thread and hands the reading to
//! [`DisplayProfile::report`], which checks the profile and builds its conversion table there.
//! Where the system converts the window to the display itself (macOS, Windows with Advanced
//! Color, a Wayland session), or the display has no usable profile, nothing is converted and the
//! canvas stays sRGB. Exports, `ui.render`, proofs and the document are not affected: a proof is
//! drawn first and then converted like any other frame.

use std::sync::{Arc, RwLock};

use serde_json::{Value, json};
use vectorcraft_color::cms::display::DisplayTransform;
pub use vectorcraft_color::cms::display::MAX_PROFILE_BYTES;
use vectorcraft_geom::{Affine, Point, Rect};

/// What the desktop app read for the display showing the main window.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// The system's name for the display (Windows: `\\.\DISPLAY1`; X11: the output's name).
    pub display: Option<String>,
    pub found: Found,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// The display's ICC profile, and where it was read from.
    Profile { icc: Vec<u8>, from: String },
    /// The system converts the window to the display: VectorCraft converts nothing. The reason.
    System(String),
    /// The display has no profile VectorCraft can read: the canvas stays sRGB. The reason.
    NoProfile(String),
}

/// What the canvas does with the reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Nothing has been read: the web build, tests, or a platform without a reader.
    Unsupported,
    /// The first reading has not finished.
    Pending,
    /// The profile is converted into.
    Applied,
    /// The profile shows sRGB unchanged.
    Identity,
    /// The system converts the window.
    System,
    /// No profile, or one that can't be used.
    NoProfile,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Pending => "pending",
            Self::Applied => "profile",
            Self::Identity => "srgbProfile",
            Self::System => "system",
            Self::NoProfile => "none",
        }
    }
}

#[derive(Debug)]
struct State {
    /// Changes whenever the conversion changes, so frames rendered for the old one are rendered again.
    generation: u64,
    kind: Kind,
    display: Option<String>,
    /// The profile's description and where it was read from.
    profile: Option<(String, String)>,
    reason: Option<String>,
    transform: Option<Arc<DisplayTransform>>,
    /// The bytes last reported, so a reading of the same profile builds nothing.
    reported: Option<(Option<String>, Vec<u8>)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            generation: 0,
            kind: Kind::Unsupported,
            display: None,
            profile: None,
            reason: Some("this build doesn't read display profiles".into()),
            transform: None,
            reported: None,
        }
    }
}

/// The display profile of the main window, shared with the thread that reads it.
#[derive(Clone, Debug, Default)]
pub struct DisplayProfile(Arc<RwLock<State>>);

impl DisplayProfile {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.0.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The desktop app has started reading the display's profile.
    pub fn pending(&self) {
        let mut s = self.write();
        if s.kind == Kind::Unsupported {
            s.kind = Kind::Pending;
            s.reason = Some("the display profile hasn't been read yet".into());
        }
    }

    /// Takes `reading` in. A profile is checked, and its conversion built, on the calling thread;
    /// a profile that can't be used leaves the canvas sRGB, with the reason in [`Self::status`].
    /// The same profile reported again for the same display changes nothing. Returns whether the
    /// canvas's conversion changed.
    pub fn report(&self, reading: Reading) -> bool {
        if let Found::Profile { icc, .. } = &reading.found
            && self.read().reported.as_ref().is_some_and(|(d, b)| *d == reading.display && b == icc)
        {
            return false;
        }
        let (kind, profile, reason, transform, reported) = match reading.found {
            Found::Profile { icc, from } => {
                let reported = Some((reading.display.clone(), icc.clone()));
                match DisplayTransform::from_icc(&icc) {
                    Ok(t) if t.is_identity() => (Kind::Identity, Some((t.name().to_string(), from)), None, None, reported),
                    Ok(t) => (Kind::Applied, Some((t.name().to_string(), from)), None, Some(Arc::new(t)), reported),
                    Err(e) => (Kind::NoProfile, None, Some(format!("{from}: {e}")), None, reported),
                }
            }
            Found::System(r) => (Kind::System, None, Some(r), None, None),
            Found::NoProfile(r) => (Kind::NoProfile, None, Some(r), None, None),
        };
        let mut s = self.write();
        let changed = s.kind != kind || s.display != reading.display || s.profile != profile || s.reason != reason;
        let converts_differently = match (&s.transform, &transform) {
            (None, None) => false,
            (Some(a), Some(b)) => a.fingerprint() != b.fingerprint(),
            _ => true,
        };
        if converts_differently {
            s.generation += 1;
        }
        (s.kind, s.display, s.profile, s.reason, s.transform, s.reported) = (kind, reading.display, profile, reason, transform, reported);
        changed || converts_differently
    }

    /// The conversion the canvas draws through: `None` when `enabled` (the preference) is off, or
    /// when nothing is to be converted.
    pub fn transform(&self, enabled: bool) -> Option<Arc<DisplayTransform>> {
        if !enabled {
            return None;
        }
        self.read().transform.clone()
    }

    /// A number that changes whenever [`Self::transform`] does, 0 while it is `None`: the canvas's
    /// frames depend on it ([`crate::CacheKey::display`]).
    pub fn key(&self, enabled: bool) -> u64 {
        let s = self.read();
        if enabled && s.transform.is_some() { s.generation } else { 0 }
    }

    /// The status `ui.inspect` reports as `displayProfile`: the display, what was found
    /// (`profile`, `srgbProfile`, `system`, `none`, `pending` or `unsupported`), the profile's
    /// description and where it was read from, whether the canvas is converted, why not, and
    /// whether the system converts the window.
    pub fn status(&self, enabled: bool) -> Value {
        let s = self.read();
        json!({
            "enabled": enabled,
            "display": s.display,
            "source": s.kind.id(),
            "profile": s.profile.as_ref().map(|p| &p.0),
            "from": s.profile.as_ref().map(|p| &p.1),
            "applied": enabled && s.transform.is_some(),
            "reason": s.reason,
            "osManaged": s.kind == Kind::System,
        })
    }

    /// The line the Preferences dialog shows under the checkbox, translated.
    pub(crate) fn note(&self, enabled: bool) -> String {
        let s = self.read();
        match (enabled, s.kind, &s.profile) {
            (false, ..) => tl!("The canvas is drawn in sRGB.").to_string(),
            (true, Kind::Applied | Kind::Identity, Some((name, _))) => format!("{} {name}", tl!("Canvas display profile:")),
            (true, Kind::System, _) => tl!("The system matches the window to the display's colors.").to_string(),
            _ => tl!("No usable display profile was found: the canvas is drawn in sRGB.").to_string(),
        }
    }
}

/// Why `icc` can't be used as a display profile, or `Ok` when [`DisplayProfile::report`] would
/// convert through it (or find that it shows sRGB unchanged). Builds no conversion table.
pub fn check_profile(icc: &[u8]) -> Result<(), String> {
    DisplayTransform::check(icc).map_err(|e| e.to_string())
}

/// `pixels` (premultiplied RGBA8) through `t`, using every core for a large frame.
pub(crate) fn convert(t: Option<&DisplayTransform>, pixels: &mut [u8]) {
    let Some(t) = t else { return };
    #[cfg(not(target_arch = "wasm32"))]
    {
        const PIECE: usize = 256 * 1024 * 4;
        let threads = std::thread::available_parallelism().map_or(1, usize::from).min(16);
        if threads > 1 && pixels.len() > PIECE {
            let per = pixels.len().div_ceil(threads).div_ceil(4) * 4;
            std::thread::scope(|scope| {
                for piece in pixels.chunks_mut(per) {
                    scope.spawn(move || t.apply_premultiplied(piece));
                }
            });
            return;
        }
    }
    t.apply_premultiplied(pixels);
}

/// The document's paper under the canvas's art, which the canvas draws converted into the display
/// profile before the art: [`convert_over`] blends translucent art with it before converting.
/// The pasteboard and Outline's white canvas are interface colors and are not part of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Backdrop {
    /// View › Hide Artboards: the sRGB color of the paper that covers the whole canvas.
    pub(crate) everywhere: Option<[u8; 3]>,
    /// The artboards in the order they are drawn (the last on top), each with its paper's sRGB color.
    pub(crate) artboards: Vec<(Rect, [u8; 3])>,
    /// The transparency grid, which covers every artboard's paper: its two sRGB colors, and its cell
    /// in screen points, counted from each artboard's top left corner.
    pub(crate) grid: Option<([[u8; 3]; 2], f32)>,
}

/// A color of the backdrop: in sRGB, and as the canvas shows it, converted.
#[derive(Clone, Copy, PartialEq)]
struct Paper {
    srgb: [u8; 3],
    shown: [u8; 3],
}

/// An artboard of the backdrop, as [`Over`] looks it up.
struct Board {
    /// Its rectangle in the document, normalized.
    rect: Rect,
    /// The corner the transparency grid's cells are counted from.
    origin: Point,
    paper: Paper,
}

/// No artboard at a pixel of a row ([`Over::owners`]).
const NO_BOARD: u32 = u32::MAX;

/// The first pixel at or after `x` that no artboard has taken yet, in the chain `next` (each
/// pixel's next candidate; a pixel not taken is its own), shortening the chain on the way.
fn untaken(next: &mut [u32], x: usize) -> usize {
    let mut root = x;
    while next[root] as usize != root {
        root = next[root] as usize;
    }
    let mut at = x;
    while next[at] as usize != root {
        let n = next[at] as usize;
        next[at] = root as u32;
        at = n;
    }
    root
}

/// [`Backdrop`] converted for a frame: what [`convert_over`] needs for each pixel.
struct Over<'a> {
    t: &'a DisplayTransform,
    /// The frame's pixels → the document.
    to_doc: Affine,
    everywhere: Option<Paper>,
    boards: Vec<Board>,
    /// The grid's two colors and its cell in document points.
    grid: Option<([Paper; 2], f64)>,
}

impl<'a> Over<'a> {
    /// None when the backdrop has no paper, or `view` (the document → the pixels) can't be undone.
    fn new(t: &'a DisplayTransform, b: &Backdrop, view: Affine, zoom: f64) -> Option<Self> {
        let det = view.determinant();
        if (b.everywhere.is_none() && b.artboards.is_empty()) || !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let paper = |srgb: [u8; 3]| Paper { srgb, shown: t.apply8(srgb) };
        let boards = b.artboards.iter().map(|&(r, color)| Board { rect: r.abs(), origin: r.origin(), paper: paper(color) }).collect();
        let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
        Some(Self {
            t,
            to_doc: view.inverse(),
            everywhere: b.everywhere.map(paper),
            boards,
            grid: b.grid.map(|([a, c], cell)| ([paper(a), paper(c)], (f64::from(cell) / zoom).max(5e-10))),
        })
    }

    /// The pixels `start..end` of the row whose centers lie at height `y` that artboard `b` covers,
    /// their centers inside its rectangle. None when it covers none of the row's `width` pixels.
    fn span(&self, b: &Board, y: f64, width: usize) -> Option<(usize, usize)> {
        let at = self.to_doc * Point::new(0.0, y);
        let [dx, dy, ..] = self.to_doc.as_coeffs();
        // Where along the row the document point stays within the rectangle, axis by axis. The
        // rectangle holds its minimum edge and not its maximum one, so a row that runs toward the
        // minimum holds its end there and not its start: each bound keeps whether it is held.
        let (mut lo, mut hi) = ((f64::NEG_INFINITY, true), (f64::INFINITY, true));
        for (start, slope, min, max) in [(at.x, dx, b.rect.x0, b.rect.x1), (at.y, dy, b.rect.y0, b.rect.y1)] {
            if slope == 0.0 {
                if !(min <= start && start < max) {
                    return None;
                }
                continue;
            }
            let (t0, t1) = ((min - start) / slope, (max - start) / slope);
            let (from, to) = if slope > 0.0 { ((t0, true), (t1, false)) } else { ((t1, false), (t0, true)) };
            // The tighter bound wins, and of two equal bounds the one not held.
            if from.0 > lo.0 || (from.0 == lo.0 && !from.1) {
                lo = from;
            }
            if to.0 < hi.0 || (to.0 == hi.0 && !to.1) {
                hi = to;
            }
        }
        // Pixel i's center is i + 0.5: the first pixel whose center is past `lo` (or on it, when
        // held), up to the first one past `hi` (or on it, when not held).
        let start = if lo.1 { (lo.0 - 0.5).ceil() } else { (lo.0 - 0.5).floor() + 1.0 };
        let end = if hi.1 { (hi.0 - 0.5).floor() + 1.0 } else { (hi.0 - 0.5).ceil() };
        let clamp = |v: f64| v.clamp(0.0, width as f64) as usize;
        let (mut start, mut end) = (clamp(start), clamp(end));
        end = end.max(start);
        // Rounding can put a bound a pixel away from where the rectangle's own test of each pixel's
        // center puts it: move each end until the two agree.
        let inside = |i: usize| b.rect.contains(self.to_doc * Point::new(i as f64 + 0.5, y));
        for _ in 0..2 {
            if start < end && !inside(start) {
                start += 1;
            } else if start > 0 && inside(start - 1) {
                start -= 1;
            }
        }
        for _ in 0..2 {
            if end > start && !inside(end - 1) {
                end -= 1;
            } else if end < width && inside(end) {
                end += 1;
            }
        }
        (start < end).then_some((start, end))
    }

    /// `owner` becomes the artboard on top at each of the `width` pixels of the row whose centers
    /// lie at height `y` ([`NO_BOARD`] where there is none). Each pixel is taken once, by the
    /// topmost artboard over it, so a row costs its width plus its artboards however they overlap.
    fn owners(&self, y: f64, width: usize, owner: &mut Vec<u32>, next: &mut Vec<u32>) {
        owner.clear();
        owner.resize(width, NO_BOARD);
        next.clear();
        next.extend(0..=width as u32);
        for (k, b) in self.boards.iter().enumerate().rev() {
            let Some((start, end)) = self.span(b, y, width) else { continue };
            let mut x = untaken(next, start);
            while x < end {
                owner[x] = k as u32;
                next[x] = x as u32 + 1;
                x = untaken(next, x + 1);
            }
        }
    }

    /// The paper at pixel `i` of the row at height `y`, whose artboard on top is `owner`.
    fn paper_at(&self, owner: u32, i: usize, y: f64) -> Option<Paper> {
        let b = self.boards.get(owner as usize)?;
        Some(match &self.grid {
            Some((colors, cell)) => {
                let p = self.to_doc * Point::new(i as f64 + 0.5, y);
                let cells = ((p.x - b.origin.x) / cell).floor() + ((p.y - b.origin.y) / cell).floor();
                colors[usize::from(cells.rem_euclid(2.0) >= 1.0)]
            }
            None => b.paper,
        })
    }

    /// Rows `first_row..` of the frame, `width` pixels each, in `pixels`.
    fn rows(&self, pixels: &mut [u8], first_row: usize, width: usize) {
        let mut last_px = ([0u8; 4], [0u8; 4]);
        let mut last_mix = (([0u8; 4], [0u8; 3]), [0u8; 4]);
        let (mut owner, mut next) = (Vec::new(), Vec::new());
        for (j, line) in pixels.chunks_exact_mut(width * 4).enumerate() {
            let y = (first_row + j) as f64 + 0.5;
            // Found for the row's first translucent pixel.
            let mut found = false;
            for (i, px) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let a = px[3];
                if a == 0 {
                    continue;
                }
                let paper = if a == 255 {
                    None
                } else if self.everywhere.is_some() {
                    self.everywhere
                } else {
                    if !found {
                        self.owners(y, width, &mut owner, &mut next);
                        found = true;
                    }
                    self.paper_at(owner[i], i, y)
                };
                match paper {
                    Some(p) if last_mix.0 == (*px, p.srgb) => *px = last_mix.1,
                    Some(p) => {
                        let input = (*px, p.srgb);
                        // Drawn over the converted paper, the pixel shows `px` over the paper in sRGB, converted.
                        let under = 255 - u32::from(a);
                        let mix = |c: usize| (u32::from(px[c]) + (u32::from(p.srgb[c]) * under + 127) / 255).min(255) as u8;
                        let shown = self.t.apply8([mix(0), mix(1), mix(2)]);
                        for ((o, s), u) in px.iter_mut().zip(shown).zip(p.shown) {
                            let paper_part = ((u32::from(u) * under + 127) / 255) as i32;
                            *o = (i32::from(s) - paper_part).clamp(0, 255) as u8;
                        }
                        last_mix = (input, *px);
                    }
                    None if last_px.0 == *px => *px = last_px.1,
                    None => {
                        let input = *px;
                        self.t.apply_premultiplied(px);
                        last_px = (input, *px);
                    }
                }
            }
        }
    }
}

/// [`convert`], with each translucent pixel over the paper or the transparency grid of `backdrop`
/// blended with it in sRGB before the conversion. The canvas draws the art over the paper
/// converted: such a pixel keeps its alpha and takes the color that, drawn over the converted
/// paper, shows the blend converted. Opaque and fully transparent pixels, and pixels over the
/// pasteboard, convert as [`convert`] converts them. The pixels are rows of `width`; `view` maps the
/// document to them, and the backdrop's `zoom` is the canvas's screen points per document point,
/// in which the grid's cells are measured.
pub(crate) fn convert_over(t: Option<&DisplayTransform>, pixels: &mut [u8], width: u32, view: Affine, backdrop: Option<(&Backdrop, f64)>) {
    let Some(t) = t else { return };
    let width = width as usize;
    let over = backdrop.filter(|_| width > 0 && pixels.len().is_multiple_of(width * 4)).and_then(|(b, zoom)| Over::new(t, b, view, zoom));
    let Some(over) = over else { return convert(Some(t), pixels) };
    #[cfg(not(target_arch = "wasm32"))]
    {
        const PIECE: usize = 256 * 1024 * 4;
        let threads = std::thread::available_parallelism().map_or(1, usize::from).min(16);
        if threads > 1 && pixels.len() > PIECE {
            let rows = (pixels.len() / (width * 4)).div_ceil(threads);
            let over = &over;
            std::thread::scope(|scope| {
                for (k, piece) in pixels.chunks_mut(rows * width * 4).enumerate() {
                    scope.spawn(move || over.rows(piece, k * rows, width));
                }
            });
            return;
        }
    }
    over.rows(pixels, 0, width);
}

/// The opaque or translucent color `c` through `t`.
pub(crate) fn color32(t: Option<&DisplayTransform>, c: egui::Color32) -> egui::Color32 {
    let Some(t) = t else { return c };
    let mut px = c.to_array();
    t.apply_premultiplied(&mut px);
    egui::Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Display P3's ICC file as a display profile.
    pub(crate) fn p3() -> Vec<u8> {
        let mut b = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::DISPLAY_P3).unwrap().to_vec();
        b[12..16].copy_from_slice(b"mntr");
        b
    }

    fn reading(found: Found) -> Reading {
        Reading { display: Some("DP-1".into()), found }
    }

    #[test]
    fn a_display_profile_is_converted_into_while_the_preference_is_on() {
        let d = DisplayProfile::default();
        assert_eq!(d.status(true)["source"], "unsupported");
        assert!(d.transform(true).is_none() && d.key(true) == 0);
        d.pending();
        assert_eq!(d.status(true)["source"], "pending");
        assert!(d.report(reading(Found::Profile { icc: p3(), from: "colord".into() })));
        let st = d.status(true);
        assert_eq!((st["source"].as_str(), st["applied"].as_bool(), st["profile"].as_str()), (Some("profile"), Some(true), Some("Display P3")));
        assert_eq!((st["display"].as_str(), st["from"].as_str(), st["osManaged"].as_bool()), (Some("DP-1"), Some("colord"), Some(false)));
        assert!(d.transform(true).is_some());
        let key = d.key(true);
        assert_ne!(key, 0);
        // Off: nothing is converted, and frames don't depend on the profile.
        assert!(d.transform(false).is_none());
        assert_eq!((d.key(false), d.status(false)["applied"].as_bool()), (0, Some(false)));
        // The same profile again changes nothing.
        assert!(!d.report(reading(Found::Profile { icc: p3(), from: "colord".into() })));
        assert_eq!(d.key(true), key);
        // The system takes over: nothing is converted.
        assert!(d.report(reading(Found::System("Windows manages this display's colors".into()))));
        let st = d.status(true);
        assert_eq!((st["source"].as_str(), st["applied"].as_bool(), st["osManaged"].as_bool()), (Some("system"), Some(false), Some(true)));
        assert_eq!(d.key(true), 0);
        // The profile back: a new key, so frames drawn before are drawn again.
        d.report(reading(Found::Profile { icc: p3(), from: "colord".into() }));
        assert!(d.key(true) > key);
    }

    #[test]
    fn a_profile_that_can_t_be_used_leaves_the_canvas_srgb_and_says_why() {
        let d = DisplayProfile::default();
        let cmyk = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::GENERIC_CMYK).unwrap().to_vec();
        assert!(d.report(reading(Found::Profile { icc: cmyk, from: "printer.icc".into() })));
        let st = d.status(true);
        assert_eq!((st["source"].as_str(), st["applied"].as_bool()), (Some("none"), Some(false)));
        assert!(st["reason"].as_str().is_some_and(|r| r.starts_with("printer.icc: ") && r.contains("not a display profile")), "{st}");
        assert!(d.transform(true).is_none());
        d.report(reading(Found::Profile { icc: vec![1, 2, 3], from: "broken.icc".into() }));
        assert!(d.status(true)["reason"].as_str().is_some_and(|r| r.contains("not an ICC profile")));
        // An sRGB display profile converts nothing.
        let mut srgb = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::SRGB).unwrap().to_vec();
        srgb[12..16].copy_from_slice(b"mntr");
        d.report(reading(Found::Profile { icc: srgb, from: "sRGB.icc".into() }));
        assert_eq!((d.status(true)["source"].as_str(), d.status(true)["applied"].as_bool()), (Some("srgbProfile"), Some(false)));
        assert!(d.transform(true).is_none());
    }

    #[test]
    fn large_frames_convert_the_same_on_several_threads() {
        let t = DisplayTransform::from_icc(&p3()).unwrap();
        let mut big: Vec<u8> = (0..(600 * 1024 * 4)).map(|i| (i * 7 % 251) as u8).collect();
        for px in big.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        let mut one = big.clone();
        t.apply_premultiplied(&mut one);
        convert(Some(&t), &mut big);
        assert_eq!(big, one);
        assert_eq!(color32(None, egui::Color32::RED), egui::Color32::RED);
        let [r, g, b] = t.apply8([255, 0, 0]);
        assert_eq!(color32(Some(&t), egui::Color32::RED), egui::Color32::from_rgb(r, g, b));
    }

    /// What the canvas shows of pixel `px` drawn over the converted paper `under`, as egui blends it.
    fn shown_over(px: [u8; 4], under: [u8; 3]) -> [u32; 3] {
        let k = 255 - u32::from(px[3]);
        [0, 1, 2].map(|i| u32::from(px[i]) + (u32::from(under[i]) * k + 127) / 255)
    }

    /// `px` blended with the sRGB paper `paper`, converted.
    fn blended(t: &DisplayTransform, px: [u8; 4], paper: [u8; 3]) -> [u8; 3] {
        let k = 255 - u32::from(px[3]);
        t.apply8([0, 1, 2].map(|i| (u32::from(px[i]) + (u32::from(paper[i]) * k + 127) / 255).min(255) as u8))
    }

    fn assert_near(got: [u32; 3], want: [u8; 3], at: &str) {
        assert!((0..3).all(|i| got[i].abs_diff(u32::from(want[i])) <= 1), "{at}: {got:?} shown, {want:?} wanted");
    }

    /// One row of `n` pixels, every one `px`.
    fn row(n: usize, px: [u8; 4]) -> Vec<u8> {
        px.repeat(n)
    }

    #[test]
    fn translucent_pixels_over_the_paper_are_blended_before_they_are_converted() {
        let t = DisplayTransform::from_icc(&p3()).unwrap();
        let half_red = [128, 0, 0, 128];
        let b = Backdrop { everywhere: None, artboards: vec![(Rect::new(0.0, 0.0, 10.0, 1.0), [255; 3])], grid: None };
        let mut px = row(20, half_red);
        px[12..16].copy_from_slice(&[255, 0, 0, 255]);
        px[16..20].copy_from_slice(&[0; 4]);
        convert_over(Some(&t), &mut px, 20, Affine::IDENTITY, Some((&b, 1.0)));
        let at = |i: usize| -> [u8; 4] { px[i * 4..i * 4 + 4].try_into().unwrap() };
        let white = t.apply8([255; 3]);
        assert_near(shown_over(at(0), white), blended(&t, half_red, [255; 3]), "over the paper");
        // Converted alone and drawn over the converted paper, the pixel shows another color.
        let mut alone = half_red;
        t.apply_premultiplied(&mut alone);
        let want = blended(&t, half_red, [255; 3]);
        assert!((0..3).any(|i| shown_over(alone, white)[i].abs_diff(u32::from(want[i])) > 2), "{alone:?}");
        assert_eq!(at(0)[3], 128, "the alpha is kept");
        // Opaque and empty pixels, and pixels over the pasteboard, convert as before.
        let red = t.apply8([255, 0, 0]);
        assert_eq!((at(3), at(4)), ([red[0], red[1], red[2], 255], [0; 4]));
        assert_eq!(at(15), alone, "over the pasteboard");
        // Without a backdrop, and without a profile, as before.
        let mut plain = row(20, half_red);
        convert_over(Some(&t), &mut plain, 20, Affine::IDENTITY, None);
        assert_eq!(plain, row(20, alone));
        let mut srgb = row(20, half_red);
        convert_over(None, &mut srgb, 20, Affine::IDENTITY, Some((&b, 1.0)));
        assert_eq!(srgb, row(20, half_red));
    }

    #[test]
    fn the_backdrop_is_found_through_the_view() {
        let t = DisplayTransform::from_icc(&p3()).unwrap();
        // Papers far apart: a pixel blended with the wrong one shows a color several levels off.
        let half = [128, 0, 0, 128];
        let (light, dark) = ([255, 230, 230], [40, 40, 30]);
        // A later artboard is drawn over an earlier one.
        let b = Backdrop {
            everywhere: None,
            artboards: vec![(Rect::new(0.0, 0.0, 100.0, 100.0), light), (Rect::new(50.0, 0.0, 100.0, 100.0), dark)],
            grid: None,
        };
        // A view turned a quarter clockwise and moved: pixel x runs down the document's y axis.
        let view = Affine::translate((10.0, 0.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2) * Affine::translate((-20.0, -60.0));
        let mut px = row(10, half);
        convert_over(Some(&t), &mut px, 10, view, Some((&b, 1.0)));
        // Pixel (2.5, 0.5) is document point (20.5, 67.5): on the first artboard only.
        let at = |i: usize| -> [u8; 4] { px[i * 4..i * 4 + 4].try_into().unwrap() };
        assert_near(shown_over(at(2), t.apply8(light)), blended(&t, half, light), "on the first artboard");
        let view = Affine::translate((-60.0, -5.0));
        let mut px = row(10, half);
        convert_over(Some(&t), &mut px, 10, view, Some((&b, 1.0)));
        let at = |i: usize| -> [u8; 4] { px[i * 4..i * 4 + 4].try_into().unwrap() };
        assert_near(shown_over(at(0), t.apply8(dark)), blended(&t, half, dark), "where the second covers the first");
        // View › Hide Artboards: the paper everywhere.
        let all = Backdrop { everywhere: Some(dark), artboards: vec![], grid: None };
        let mut px = row(10, half);
        convert_over(Some(&t), &mut px, 10, Affine::translate((-5000.0, 0.0)), Some((&all, 1.0)));
        assert_near(shown_over(px[..4].try_into().unwrap(), t.apply8(dark)), blended(&t, half, dark), "Hide Artboards");
        // Overlapping artboards under a turned and scaled view, against a direct lookup of the
        // artboard on top whose rectangle holds each pixel's center.
        let boards: Vec<(Rect, [u8; 3])> = (0..12u8)
            .map(|k| {
                let (x, y) = (f64::from(k) * 7.0, f64::from(k % 3) * 9.0);
                (Rect::new(x, y, x + 15.0, y + 20.0), if k % 2 == 0 { [255, 255, 255 - k] } else { [20 + k, 20, 30] })
            })
            .collect();
        let check = |boards: &[(Rect, [u8; 3])], view: Affine, (w, h): (usize, usize)| {
            let b = Backdrop { everywhere: None, artboards: boards.to_vec(), grid: None };
            let mut px = row(w * h, half);
            convert_over(Some(&t), &mut px, w as u32, view, Some((&b, 1.0)));
            let to_doc = view.inverse();
            let mut on_paper = 0;
            for i in 0..w * h {
                let p = to_doc * Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
                let mut want = half.to_vec();
                match boards.iter().rev().find(|(r, _)| r.contains(p)) {
                    Some(&(_, paper)) => {
                        let one = Backdrop { everywhere: Some(paper), artboards: vec![], grid: None };
                        convert_over(Some(&t), &mut want, 1, Affine::IDENTITY, Some((&one, 1.0)));
                        on_paper += 1;
                    }
                    None => t.apply_premultiplied(&mut want),
                }
                assert_eq!(&px[i * 4..i * 4 + 4], &want[..], "pixel {i} at {p:?} through {view:?}");
            }
            assert!(on_paper > 100 && on_paper < w * h, "{on_paper} pixels on the artboards");
        };
        check(&boards, Affine::rotate(0.3) * Affine::scale(1.7) * Affine::translate((5.0, -3.0)), (64, 48));
        // Rows that run backward across the artboards, with pixel centers on their edges: an edge
        // at the minimum is on the artboard, one at the maximum is not.
        check(&boards, Affine::new([-1.0, 0.0, 0.0, 1.0, 92.5, 0.0]), (96, 44));
        check(&boards, Affine::new([-1.0, 0.0, 0.0, -1.0, 92.5, 40.5]), (96, 44));
        // A sheared view where a computed bound falls on a pixel center that rounding puts just
        // outside the artboard: the pixel's own center decides.
        check(&[(Rect::new(-100.0, -2.0, 100.0, 100.0), [40, 40, 30])], Affine::new([3.0, 1.5, 0.1, 0.5, -0.3, 0.0]), (64, 48));
    }

    #[test]
    fn the_transparency_grid_is_blended_cell_by_cell() {
        let t = DisplayTransform::from_icc(&p3()).unwrap();
        let half = [128, 0, 0, 128];
        // Far apart, so that a pixel blended with the other cell's color shows several levels off.
        let (light, dark) = ([255; 3], [40; 3]);
        // Cells of 10 points from the artboard's corner at (5, 0), here 1 pixel a point.
        let b = Backdrop { everywhere: None, artboards: vec![(Rect::new(5.0, 0.0, 65.0, 20.0), [255, 0, 0])], grid: Some(([light, dark], 10.0)) };
        let check = |zoom: f64, cells: [(usize, [u8; 3]); 3]| {
            let mut px = row(60, half);
            convert_over(Some(&t), &mut px, 60, Affine::IDENTITY, Some((&b, zoom)));
            for (i, paper) in cells {
                assert_near(
                    shown_over(px[i * 4..i * 4 + 4].try_into().unwrap(), t.apply8(paper)),
                    blended(&t, half, paper),
                    &format!("pixel {i} at {zoom}"),
                );
            }
        };
        check(1.0, [(6, light), (16, dark), (26, light)]);
        // At 200% a cell is 5 document points.
        check(2.0, [(6, light), (11, dark), (16, light)]);
    }

    #[test]
    fn large_frames_blend_the_same_on_several_threads() {
        let t = DisplayTransform::from_icc(&p3()).unwrap();
        let b = Backdrop {
            everywhere: None,
            artboards: vec![(Rect::new(0.0, 0.0, 500.0, 900.0), [240, 230, 220])],
            grid: Some(([[255; 3], [200; 3]], 8.0)),
        };
        let (w, h) = (1000, 1000);
        let mut big: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 % 251) as u8).collect();
        for px in big.as_chunks_mut::<4>().0 {
            let a = px[3];
            for c in &mut px[..3] {
                *c = (*c).min(a);
            }
        }
        let mut one = big.clone();
        Over::new(&t, &b, Affine::IDENTITY, 1.0).unwrap().rows(&mut one, 0, w);
        convert_over(Some(&t), &mut big, w as u32, Affine::IDENTITY, Some((&b, 1.0)));
        assert_eq!(big, one);
    }
}
