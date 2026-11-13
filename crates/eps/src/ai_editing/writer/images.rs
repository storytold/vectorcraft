//! Images: embedded ones as a raster with its samples, linked ones as a placed file with a preview
//! (see [`crate::ai_editing`] for the form).

use std::fmt::Write;
use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{FillLayer, ImageBlob, ImageObject, LinkInfo};
use vectorcraft_geom::{Affine, FillRule, PathData, Rect, Shape};

use super::{Writer, Xy};
use crate::{ai_ops, ps};

pub(in crate::ai_editing) const IMAGES: &str = "images that can't be decoded or have more than 33,554,432 pixels are written as their outlines filled mid gray in the editing data for other apps";
pub(in crate::ai_editing) const IMAGE_DEPTH: &str = "images of 16 bits a channel are written at 8 bits a channel in the editing data for other apps";
pub(in crate::ai_editing) const LINKS_EMBEDDED: &str =
    "linked images in opacity masks, pattern tiles and symbols are written as embedded images in the editing data for other apps";

/// The most pixels an image is written with: as many as the reader reads.
pub(super) const MAX_PIXELS: u64 = 1 << 25;
/// The longer side of a placed file's preview at most (a smaller image's preview is its own
/// pixels): the files users sent have previews 256 pixels wide.
const PREVIEW_SIDE: u32 = 256;
/// What marks a binary payload in the program text: `\u{1}`, its index, `\u{1}`
/// ([`super::super::splice`] puts the bytes in its place). Strings never hold the byte unescaped.
pub(in crate::ai_editing) const PAYLOAD: char = '\u{1}';
/// What starts the line of an image's samples: `XI` and its payload, then `%%EndData`.
pub(super) const SAMPLES_LINE: &str = "XI\u{1}";

/// An image's pixels as the data holds them, and whether it had 16 bits a channel ([`Samples::of`]).
pub(super) type Read = Arc<(Samples, bool)>;

/// An image's pixels as the data holds them.
pub(super) struct Samples {
    /// `/DeviceGray`, `/DeviceRGB` or `/DeviceCMYK`.
    space: &'static str,
    /// The image type of `XI` (§8.1): 1 gray, 3 RGB, 4 CMYK.
    kind: u8,
    width: u32,
    height: u32,
    /// The color samples, 8 bits each, rows from the top.
    color: Vec<u8>,
    /// The alpha channel, a byte a pixel, when a pixel isn't opaque.
    alpha: Option<Vec<u8>>,
}

impl Samples {
    /// The pixels of `blob`: a CMYK image's ink amounts, else gray samples when every pixel is gray
    /// or RGB samples, with an alpha channel when they need one. `None` when they can't be decoded or are more than
    /// [`MAX_PIXELS`]; the flag is set when the image had 16 bits a channel.
    fn of(blob: &ImageBlob) -> Option<(Samples, bool)> {
        // The header's size is checked before the inks are decoded.
        if blob.is_cmyk()
            && blob.header_size().is_some_and(|(w, h)| u64::from(w) * u64::from(h) <= MAX_PIXELS)
            && let Some(inks) = blob.cmyk()
            && u64::from(inks.width) * u64::from(inks.height) <= MAX_PIXELS
        {
            let s = Samples { space: "/DeviceCMYK", kind: 4, width: inks.width, height: inks.height, color: inks.data, alpha: None };
            return Some((s, blob.is_cmyk16()));
        }
        let img = super::decode_bounded(&blob.bytes, MAX_PIXELS)?;
        let deep = matches!(
            img.color(),
            image::ColorType::L16
                | image::ColorType::La16
                | image::ColorType::Rgb16
                | image::ColorType::Rgba16
                | image::ColorType::Rgb32F
                | image::ColorType::Rgba32F
        );
        let rgba = img.to_rgba8();
        // Gray when every pixel is, whatever the file's color type (an image read back from the data
        // is RGBA): the same pixels are written the same way.
        let gray = rgba.pixels().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]);
        let (width, height) = rgba.dimensions();
        if width == 0 || height == 0 {
            return None;
        }
        let opaque = rgba.pixels().all(|p| p.0[3] == 255);
        let alpha = (!opaque).then(|| rgba.pixels().map(|p| p.0[3]).collect());
        let color = if gray { rgba.pixels().map(|p| p.0[0]).collect() } else { rgba.pixels().flat_map(|p| [p.0[0], p.0[1], p.0[2]]).collect() };
        let (space, kind) = if gray { ("/DeviceGray", 1) } else { ("/DeviceRGB", 3) };
        Some((Samples { space, kind, width, height, color, alpha }, deep))
    }

    /// The pixels of `window` (`[x0, y0, x1, y1]`, within the image) alone.
    fn window(&self, window: [u32; 4]) -> Samples {
        let [x0, y0, x1, y1] = window;
        Samples {
            space: self.space,
            kind: self.kind,
            width: x1 - x0,
            height: y1 - y0,
            color: ai_ops::window_samples(&self.color, self.width, usize::from(self.kind), window),
            alpha: self.alpha.as_ref().map(|a| ai_ops::window_samples(a, self.width, 1, window)),
        }
    }

    /// `XI`'s fourth flag as the files users sent have it: 0 gray, 1 RGB, 2 CMYK, 16 more with an
    /// alpha channel.
    fn reserved(&self) -> u8 {
        let space = match self.kind {
            1 => 0,
            4 => 2,
            _ => 1,
        };
        space + if self.alpha.is_some() { 16 } else { 0 }
    }
}

impl Writer<'_> {
    /// Image object `im` of transparency `xy`: embedded, a raster ([`Self::raster`]); linked, a
    /// placed file with a preview ([`Self::placed_file`]), except in a part of the data written on
    /// `%_` lines and in a symbol's art. One whose pixels can't be read is its outline filled mid gray. Where the art is
    /// cut to a clipping path, the image is in a clipping group of that path; a raster of pixels
    /// written whole already is then the part of them that the path shows ([`ai_ops::image_window`]).
    pub(super) fn image(&mut self, im: &ImageObject, xy: Xy) {
        let doc = self.doc;
        let blob = doc.image_blob(im);
        let read = blob.and_then(|b| self.samples(b));
        let Some((samples, deep)) = read.as_deref() else {
            let bounds = PathData::from_bezpath(&(im.xf * Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height)).to_path(0.1)));
            self.warn(IMAGES);
            return self.fill_object(&bounds, FillRule::NonZero, &FillLayer::new(Paint::solid(Color::gray(0.5))), xy);
        };
        // A region that has nothing left hides the image.
        if self.clip.as_ref().is_some_and(|c| c.subpaths.iter().all(|sp| sp.anchors.is_empty())) {
            return;
        }
        // The image, and where it is cut, the clipping group and its clipping path.
        let clipping = self.clip.as_ref().map_or(0, |c| 1 + super::object_count(c.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).count()));
        if !self.take_objects(1 + clipping) {
            return;
        }
        if *deep {
            self.warn(IMAGE_DEPTH);
        }
        let cut = self.clip.take();
        let at_q = (self.state, self.gs);
        if cut.is_some() {
            self.line("q");
        }
        self.set_object_xy(xy.object());
        // The object places the image's own pixel grid (its width × height) wherever the file's is.
        let fit =
            Affine::scale_non_uniform(f64::from(im.width.max(1)) / f64::from(samples.width), f64::from(im.height.max(1)) / f64::from(samples.height));
        match &im.link {
            Some(link) if self.hidden == 0 && !self.in_symbol => {
                // Cut to a region, the placed file is in a clipping group.
                self.placed = cut.is_none();
                self.placed_file(link, samples, (im.width.max(1), im.height.max(1)), im.xf);
            }
            other => {
                if other.is_some() {
                    self.warn(LINKS_EMBEDDED);
                }
                // An image written whole already: the part its region shows, placed where it is in
                // the image (the pieces of a warped image each show a small part of the same one).
                // An object's editable original (on `%_` lines) and an image in an opacity mask's art
                // are written whole.
                let data = blob.map(|b| &b.bytes);
                let written = self.hidden == 0 && data.is_some_and(|d| self.whole_images.contains_key(&(Arc::as_ptr(d) as usize)));
                let window = written
                    .then(|| {
                        let clips: Vec<_> = self.shown.iter().cloned().chain(cut.iter().map(PathData::to_bezpath)).collect();
                        ai_ops::image_window(&clips, im.xf * fit, (samples.width, samples.height))
                    })
                    .flatten();
                match window {
                    Some(window) => {
                        self.raster(&samples.window(window), im.xf * fit * Affine::translate((f64::from(window[0]), f64::from(window[1]))))
                    }
                    None => {
                        if let Some(d) = data {
                            // `raster` adds the samples as the next payload. A copy written whole on
                            // `%_` lines after an earlier whole copy keeps the earlier one's entry.
                            let next = self.payloads.len();
                            self.whole_images.entry(Arc::as_ptr(d) as usize).or_insert_with(|| (d.clone(), next));
                        }
                        self.raster(samples, im.xf * fit);
                    }
                }
            }
        }
        if let Some(region) = &cut {
            self.line("0 XR");
            // Several subpaths clip as one compound path (`*u` … `*U`), as a clipping group's do.
            let subpaths: Vec<_> = region.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).collect();
            let several = subpaths.len() > 1;
            if several {
                self.line("*u");
            }
            for sp in subpaths {
                crate::ai_ops::subpath(&mut self.body, self.to_art, sp);
                self.line("h\nW\nn");
            }
            if several {
                self.line("*U");
            }
            (self.state, self.gs) = at_q;
            self.end_group("Q", Xy::PLAIN);
        }
        self.clip = cut;
    }

    /// An embedded image whose pixels `xf` places in the document: `%AI5_File:`,
    /// `%AI5_BeginRaster`, `() 1 XG` (no file), the color space `XN`, `[matrix] w h 0 Xh`,
    /// `[matrix] 0 0 w h w h 8 type alpha reserved 1 0 4 4 0 0`, `%%BeginData:` counting `XI`, its
    /// line end and the color samples, `XI`, the color samples then the alpha channel in binary,
    /// `%%EndData`, `XH`, `%AI5_EndRaster`, `N`. The matrix is an art-space matrix applied to the
    /// pixels with their rows running downwards, `(u, -v)`, as §8.1's image matrix; in a symbol's
    /// art (outside a pattern's tile in it), a matrix in the canvas's space turned y up about art
    /// space's origin there, as an instance's matrix in it is.
    fn raster(&mut self, s: &Samples, xf: Affine) {
        let space = if self.in_symbol && self.defining.is_empty() { self.instance_space } else { self.to_art };
        let m = matrix(space * xf * Affine::FLIP_Y);
        let (w, h) = (s.width, s.height);
        let alpha = u8::from(s.alpha.is_some());
        let mut payload = Vec::with_capacity(2 + s.color.len() + s.alpha.as_ref().map_or(0, Vec::len));
        payload.extend_from_slice(b"\r\n");
        payload.extend_from_slice(&s.color);
        if let Some(a) = &s.alpha {
            payload.extend_from_slice(a);
        }
        let mark = self.payload(payload);
        let _ = writeln!(
            self.body,
            "%AI5_File:\n%AI5_BeginRaster\n() 1 XG\n{} XN\n{m} {w} {h} 0 Xh\n{m} 0 0 {w} {h} {w} {h} 8 {} {alpha} {} 1 0 4 4 0 0\n%%BeginData: {}\nXI{mark}%%EndData\nXH\n%AI5_EndRaster\nN",
            s.space,
            s.kind,
            s.reserved(),
            // `XI`, CR LF.
            4 + s.color.len(),
        );
    }

    /// A linked image whose pixels `xf` places in the document: `%AI5_BeginPlace`,
    /// `%%FileType: 1`, `[matrix] llx lly urx ury 4 4 0` and the file's path with `` ` ``,
    /// `%%IncludeFile:` and the path, the folder relative to the document's between
    /// `%AI28_BeginPlacedRelativePath` and its end (`()` for the document's own folder), the preview between
    /// `%AI26_BeginPlacedObjectPreview` and its end (`w h rowBytes 32 17 ` and rows of
    /// premultiplied ARGB, the top one first, of the pixels `s`), `~`, `%AI5_EndPlace`. The box is
    /// as large as the file has pixels (`w` × `h`, the object's pixel grid, which `xf` places in the
    /// document), at art space's origin on the canvas (y down); the matrix takes it from there into
    /// art space. Each marked part newer releases read is in versioned content, as in the files
    /// users sent.
    fn placed_file(&mut self, link: &LinkInfo, s: &Samples, (w, h): (u32, u32), xf: Affine) {
        let (x, y) = self.canvas_origin;
        let m = matrix(self.to_art * xf * Affine::translate((-x, -y)));
        let (w, h) = (f64::from(w), f64::from(h));
        let path = ps::bytes(link.path.as_bytes());
        let _ = write!(self.body, "%AI5_BeginPlace\n%%FileType: 1\n{m} ");
        ps::push_nums(&mut self.body, &[x, y, x + w, y + h, 4.0, 4.0, 0.0]);
        self.body.pop();
        let _ = writeln!(self.body, "\n{path}`");
        if !link.path.chars().any(char::is_control) {
            let _ = writeln!(self.body, "%%IncludeFile:{}", link.path);
        }
        // The section holds a folder only, the file's name comes from the path: a relative path to a
        // file of another name isn't written. On Windows and macOS, whose file names ignore letter
        // case, names that differ only in case are the same file.
        let same_name = |r: &&str| r.rsplit(['/', '\\']).next().is_some_and(|n| same_file_name(n, link.name()));
        if let Some(folder) = link.relative.as_deref().filter(same_name).map(relative_folder) {
            let _ = writeln!(
                self.body,
                "%AI17_Begin_Content_if_version_gt:24 12\n%AI28_BeginPlacedRelativePath\n{}\n%AI28_EndPlacedRelativePath\n%AI17_Alternate_Content\n%AI17_End_Versioned_Content",
                ps::bytes(folder.as_bytes())
            );
        }
        let (pw, ph, rows) = preview(s);
        let mark = self.payload(rows);
        let _ = writeln!(
            self.body,
            "%AI17_Begin_Content_if_version_gt:24 5\n%AI26_BeginPlacedObjectPreview\n{pw} {ph} {} 32 17 {mark}\n%AI26_EndPlacedObjectPreview\n%AI17_End_Versioned_Content\n~\n%AI5_EndPlace",
            pw * 4
        );
    }

    /// The pixels of `blob` ([`Samples::of`]), read once for images that follow one another with
    /// the same data.
    fn samples(&mut self, blob: &ImageBlob) -> Option<Read> {
        if let Some((last, read)) = &self.last_samples
            && Arc::ptr_eq(&last.bytes, &blob.bytes)
            && last.mime == blob.mime
        {
            return read.clone();
        }
        let read = Samples::of(blob).map(Arc::new);
        self.last_samples = Some((blob.clone(), read.clone()));
        read
    }

    /// Keep `bytes` to go in the program where the mark this returns is.
    fn payload(&mut self, bytes: Vec<u8>) -> String {
        self.payloads.push(bytes);
        format!("{PAYLOAD}{}{PAYLOAD}", self.payloads.len() - 1)
    }
}

/// `[a b c d tx ty]`, each number to ten decimals ([`ps::num`] writes four).
fn matrix(m: Affine) -> String {
    let n: Vec<String> = m.as_coeffs().iter().map(|v| precise(*v)).collect();
    format!("[{}]", n.join(" "))
}

/// `v` rounded to ten decimals, without trailing zeros (0 when not finite).
pub(super) fn precise(v: f64) -> String {
    let r = (v * 1e10).round() / 1e10;
    if !r.is_finite() {
        return if v.is_finite() { format!("{}", v + 0.0) } else { "0".into() };
    }
    format!("{}", r + 0.0)
}

/// The folder of a path relative to the document's (`src/checker.png` → `src/`); empty when the file
/// is in the document's own folder. `\` separators are read as `/`.
fn relative_folder(relative: &str) -> String {
    match relative.replace('\\', "/").rsplit_once('/') {
        Some((folder, _)) if !folder.is_empty() => format!("{folder}/"),
        _ => String::new(),
    }
}

/// Do file names `a` and `b` name the same file in one folder: equal, or on Windows and macOS equal
/// but for letter case?
fn same_file_name(a: &str, b: &str) -> bool {
    a == b || (cfg!(any(windows, target_os = "macos")) && a.to_lowercase() == b.to_lowercase())
}

/// A placed file's preview of `s`: its width, height and rows of premultiplied ARGB, the top one
/// first; the image's own pixels when neither side is longer than [`PREVIEW_SIDE`], else scaled
/// down to it, each preview pixel the average of the premultiplied pixels it covers.
fn preview(s: &Samples) -> (u32, u32, Vec<u8>) {
    let (w, h) = (u64::from(s.width), u64::from(s.height));
    let longer = s.width.max(s.height);
    let (pw, ph) = if longer > PREVIEW_SIDE {
        let k = f64::from(PREVIEW_SIDE) / f64::from(longer);
        let side = |v: u32| ((f64::from(v) * k).round() as u32).clamp(1, PREVIEW_SIDE);
        (side(s.width), side(s.height))
    } else {
        (s.width, s.height)
    };
    let n = usize::from(channels(s.kind));
    let cmyk_memo = std::cell::RefCell::new(std::collections::HashMap::new());
    // Pixel `i` premultiplied: r, g, b, a.
    let pixel = |i: usize| -> [u64; 4] {
        let c = |k: usize| s.color.get(i * n + k).copied().unwrap_or(0);
        let rgb = match s.kind {
            1 => [c(0); 3],
            4 => {
                let mut memo = cmyk_memo.borrow_mut();
                if memo.len() > 1 << 16 {
                    memo.clear();
                }
                *memo.entry([c(0), c(1), c(2), c(3)]).or_insert_with(|| {
                    let f = |v: u8| f32::from(v) / 255.0;
                    let [r, g, b, _] = Color::cmyk(f(c(0)), f(c(1)), f(c(2)), f(c(3))).to_rgba8(1.0);
                    [r, g, b]
                })
            }
            _ => [c(0), c(1), c(2)],
        };
        let a = u64::from(s.alpha.as_ref().and_then(|a| a.get(i)).copied().unwrap_or(255));
        let pre = |v: u8| (u64::from(v) * a + 127) / 255;
        [pre(rgb[0]), pre(rgb[1]), pre(rgb[2]), a]
    };
    let (pw64, ph64) = (u64::from(pw), u64::from(ph));
    let mut rows = Vec::with_capacity((pw64 * ph64 * 4) as usize);
    for py in 0..ph64 {
        let (y0, y1) = (py * h / ph64, ((py + 1) * h / ph64).max(py * h / ph64 + 1));
        for px in 0..pw64 {
            let (x0, x1) = (px * w / pw64, ((px + 1) * w / pw64).max(px * w / pw64 + 1));
            let mut sum = [0u64; 4];
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let p = pixel((y * w + x) as usize);
                    for (t, v) in sum.iter_mut().zip(p) {
                        *t += v;
                    }
                }
            }
            let count = ((y1.min(h) - y0) * (x1.min(w) - x0)).max(1);
            let [r, g, b, a] = sum.map(|t| ((t + count / 2) / count).min(255) as u8);
            rows.extend_from_slice(&[a, r, g, b]);
        }
    }
    (pw, ph, rows)
}

/// The samples a pixel of image type `kind` has.
fn channels(kind: u8) -> u8 {
    match kind {
        1 => 1,
        4 => 4,
        _ => 3,
    }
}
