//! DCS files (Desktop Color Separation): an EPS file whose header comments point at one EPS
//! program per printing plate. A DCS 2.0 file names each plate with `%%PlateFile: (name) EPS
//! #offset size`, the plate inside the file `offset` bytes from its start, or `%%PlateFile: (name)
//! EPS Local file`, the plate in a file of its own; a DCS 1.0 file names the four process plates'
//! files with `%%CyanPlate:`, `%%MagentaPlate:`, `%%YellowPlate:` and `%%BlackPlate:`. Spot inks
//! take their process equivalents from `%%CMYKCustomColor:` (else `%%RGBCustomColor:`). The comments
//! are described at http://www.telegraphics.net/doc/DCS.txt.
//!
//! [`read`] finds those comments; [`place`] reads every plate's image (its ink amounts: a dark
//! plate pixel is ink) into one [`Separated`] image, with the clipping path the main file's
//! composite is drawn through.

use vectorcraft_color::cms::PROCESS_PLATES;
use vectorcraft_doc::separated::{InkPlate, MAX_NAME, MAX_PLATES, MAX_SAMPLES, Separated};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect};

/// Where a plate's program is.
#[derive(Clone, Debug, PartialEq)]
pub enum PlateAt {
    /// In the DCS file itself: `len` bytes from byte `offset` of the file.
    Here { offset: usize, len: usize },
    /// In a file of its own, as the DCS file names it (a path or a file name).
    File(String),
}

/// One plate a DCS file names.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateRef {
    /// "Cyan", "Magenta", "Yellow", "Black" or a spot ink's name.
    pub name: String,
    pub at: PlateAt,
}

/// The DCS comments of a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Dcs {
    /// 1 or 2.
    pub version: u8,
    pub plates: Vec<PlateRef>,
    /// Spot inks' process equivalents (C, M, Y, K, 0..1), by name.
    pub equivalents: Vec<(String, [f32; 4])>,
}

impl Dcs {
    /// The names of the plate files kept apart from the DCS file.
    pub fn plate_files(&self) -> impl Iterator<Item = &str> {
        self.plates.iter().filter_map(|p| match &p.at {
            PlateAt::File(f) => Some(f.as_str()),
            PlateAt::Here { .. } => None,
        })
    }
}

/// The DCS comments of EPS file `bytes` (behind its binary header, if it has one); `None` when it
/// names no plate.
pub fn read(bytes: &[u8]) -> Option<Dcs> {
    let (ps, _) = crate::sections(bytes)?;
    if !ps.starts_with(b"%!") {
        return None;
    }
    // The plates of the DCS 2.0 comments and of the DCS 1.0 ones.
    let mut plates: Vec<PlateRef> = vec![];
    let mut old: Vec<PlateRef> = vec![];
    let mut equivalents: Vec<(String, [f32; 4])> = vec![];
    let mut rgb: Vec<(String, [f32; 4])> = vec![];
    // The comment a `%%+` line continues.
    let mut last = String::new();
    for raw in ps.split(|b| matches!(b, b'\n' | b'\r')).filter(|l| !l.is_empty()).take(MAX_HEADER_LINES) {
        let line = String::from_utf8_lossy(raw);
        let line = line.trim_end();
        if line.starts_with("%%EndComments") || !line.starts_with('%') {
            break;
        }
        let (key, value) = match line.strip_prefix("%%+") {
            Some(v) => (last.clone(), v.trim()),
            None => match line.split_once(':') {
                Some((k, v)) => (k.to_string(), v.trim()),
                None => (line.to_string(), ""),
            },
        };
        match key.as_str() {
            "%%PlateFile" => {
                if let Some(p) = plate_file(value)
                    && !plates.iter().any(|q| q.name == p.name)
                {
                    plates.push(p);
                }
            }
            "%%CyanPlate" | "%%MagentaPlate" | "%%YellowPlate" | "%%BlackPlate" => {
                let name = key.trim_start_matches("%%").trim_end_matches("Plate").to_string();
                if !value.is_empty() && !old.iter().any(|q| q.name == name) {
                    old.push(PlateRef { name, at: PlateAt::File(value.to_string()) });
                }
            }
            "%%CMYKCustomColor" => equivalents.extend(custom_colors(value, false)),
            "%%RGBCustomColor" => rgb.extend(custom_colors(value, true)),
            _ => {}
        }
        last = key;
    }
    // A file is one version or the other; the plate comments of DCS 2.0 win.
    let version = if plates.is_empty() {
        plates = old;
        1
    } else {
        2
    };
    if plates.is_empty() {
        return None;
    }
    for (name, c) in rgb {
        if !equivalents.iter().any(|(n, _)| *n == name) {
            equivalents.push((name, c));
        }
    }
    Some(Dcs { version, plates, equivalents })
}

/// Most header lines looked at for the DCS comments.
const MAX_HEADER_LINES: usize = 10_000;

/// A `%%PlateFile:` comment's plate: `(name) EPS #offset size` or `(name) EPS Local file`; `None`
/// for other locations, which only a print server reads.
fn plate_file(v: &str) -> Option<PlateRef> {
    let (name, rest) = ps_string(v)?;
    let mut words = rest.split_whitespace();
    let _kind = words.next()?;
    let location = words.next()?;
    let name = canonical(name)?;
    if let Some(offset) = location.strip_prefix('#') {
        let len = words.next()?.parse().ok()?;
        return Some(PlateRef { name, at: PlateAt::Here { offset: offset.parse().ok()?, len } });
    }
    if !location.eq_ignore_ascii_case("Local") {
        return None;
    }
    let file = rest.split_whitespace().skip(2).collect::<Vec<_>>().join(" ");
    (!file.is_empty()).then_some(PlateRef { name, at: PlateAt::File(file) })
}

/// A plate's name as it is kept: a process ink's in its usual case ("cyan" is "Cyan"); `None` when
/// it is empty or longer than [`MAX_NAME`].
fn canonical(name: String) -> Option<String> {
    if name.is_empty() || name.len() > MAX_NAME {
        return None;
    }
    Some(PROCESS_PLATES.iter().find(|p| p.eq_ignore_ascii_case(&name)).map_or(name, |p| p.to_string()))
}

/// The custom colours of a `%%CMYKCustomColor:` (four numbers then a name, repeated) or, with
/// `rgb`, a `%%RGBCustomColor:` comment (three numbers), the RGB ones as the process inks that
/// take their light away (`1 − r`, `1 − g`, `1 − b`, no black). Names may be bare words.
fn custom_colors(v: &str, rgb: bool) -> Vec<(String, [f32; 4])> {
    let mut out = vec![];
    let mut rest = v.trim();
    let n = if rgb { 3 } else { 4 };
    while !rest.is_empty() && out.len() < MAX_PLATES {
        let mut nums = [0.0f32; 4];
        for v in nums.iter_mut().take(n) {
            let Some((word, after)) = rest.split_once(char::is_whitespace) else { return out };
            let Some(x) = word.parse::<f32>().ok().filter(|x| (0.0..=1.0).contains(x)) else { return out };
            *v = x;
            rest = after.trim_start();
        }
        let (name, after) = match ps_string(rest) {
            Some((s, a)) => (s, a),
            None => {
                let (w, a) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                (w.to_string(), a)
            }
        };
        rest = after.trim_start();
        let cmyk = if rgb { [1.0 - nums[0], 1.0 - nums[1], 1.0 - nums[2], 0.0] } else { nums };
        if let Some(name) = canonical(name) {
            out.push((name, cmyk));
        }
    }
    out
}

/// The PostScript string at the start of `v` (`(…)`, with nested parentheses and backslash
/// escapes) and what follows it.
fn ps_string(v: &str) -> Option<(String, &str)> {
    let body = v.trim_start().strip_prefix('(')?;
    let mut out = String::new();
    let mut depth = 0usize;
    let mut chars = body.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                d @ '0'..='7' => {
                    let mut code = d.to_digit(8)?;
                    for _ in 0..2 {
                        match chars.peek().and_then(|(_, e)| e.to_digit(8)) {
                            Some(e) => {
                                chars.next();
                                code = code * 8 + e;
                            }
                            None => break,
                        }
                    }
                    // Octal escapes name bytes; names are read as Latin-1.
                    out.push(char::from_u32(code & 0xff)?);
                }
                e => out.push(e),
            },
            '(' => {
                depth += 1;
                out.push(c);
            }
            ')' if depth == 0 => return Some((out, body.get(i + 1..)?)),
            ')' => {
                depth -= 1;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    None
}

/// The main part of DCS file `bytes`: its PostScript (behind its binary header, if it has one), up
/// to the first plate it holds.
pub fn main_program<'a>(bytes: &'a [u8], dcs: &Dcs) -> Option<&'a [u8]> {
    let (ps, _) = crate::sections(bytes)?;
    let start = ps.as_ptr() as usize - bytes.as_ptr() as usize;
    let first = dcs.plates.iter().filter_map(|p| match p.at {
        PlateAt::Here { offset, .. } if offset > start => Some(offset - start),
        _ => None,
    });
    Some(&ps[..first.min().unwrap_or(ps.len()).min(ps.len())])
}

/// A DCS file read for placing.
#[derive(Debug)]
pub struct Placed {
    pub separated: Separated,
    /// The file's page (its bounding box) with its top-left corner at the origin.
    pub page: Rect,
    /// The image's pixels onto the page.
    pub xf: Affine,
    /// The clipping path the composite is drawn through, on the page.
    pub clip: Option<Node>,
    /// The resolution of the plates (pixels per inch), from how large they print.
    pub ppi: Option<(f64, f64)>,
    pub warnings: Vec<String>,
}

/// One plate's image: its ink amounts, one byte a pixel, and where its pixels print on its page.
struct PlateImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
    xf: Affine,
}

/// Read DCS file `bytes` (its comments `dcs`) for placing: every plate's image, the plates kept
/// apart read with `file` (the name the file gives → the plate file's bytes). The plates are put on
/// the pixel grid of the first plate's image, grown to hold every plate's image where it prints;
/// another plate is resampled onto it. An error when the file names more than [`MAX_PLATES`]
/// plates or a plate can't be read.
pub fn place(bytes: &[u8], dcs: &Dcs, file: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>) -> Result<Placed, String> {
    if dcs.plates.len() > MAX_PLATES {
        return Err(format!("it names {} plates, and at most {MAX_PLATES} are read", dcs.plates.len()));
    }
    let main = main_program(bytes, dcs).ok_or("the file's preview header points outside the file")?;
    let page = crate::page(main)?;
    let mut warnings = vec![];
    let (mut images, mut samples) = (vec![], 0u64);
    for p in &dcs.plates {
        let read = match &p.at {
            PlateAt::Here { offset, len } => {
                let eps = offset.checked_add(*len).and_then(|end| bytes.get(*offset..end));
                let eps = eps.ok_or_else(|| format!("the {} plate lies outside the file", p.name))?;
                plate_image(eps)
            }
            PlateAt::File(f) => file(f).and_then(|eps| plate_image(&eps)),
        };
        let img = read.map_err(|e| format!("the {} plate can't be read: {e}", p.name))?;
        // Checked before the other plates are read.
        samples += u64::from(img.width) * u64::from(img.height);
        if samples > MAX_SAMPLES {
            return Err(format!("its plates are too large: more than {MAX_SAMPLES} pixels together"));
        }
        images.push((p, img));
    }
    let Some((w, h, xf)) = grid(images.iter().map(|(_, img)| img)) else { return Err("the file names no plate".into()) };
    if !Separated::fits(w, h, images.len()) {
        return Err(format!("its plates are too large: {} plates of {w} × {h} pixels", images.len()));
    }
    let mut plates = vec![];
    for (p, img) in images {
        let name = p.name.clone();
        let aligned = img.xf.as_coeffs().iter().zip(xf.as_coeffs()).all(|(a, b)| (a - b).abs() < 1e-9);
        let data = if (img.width, img.height) == (w, h) && aligned { img.data } else { resample(&img, w, h, xf) };
        let cmyk = match dcs.equivalents.iter().find(|(n, _)| *n == name) {
            Some((_, c)) => *c,
            None if PROCESS_PLATES.contains(&name.as_str()) => [0.0; 4],
            None => {
                warnings.push(format!("the file gives no process equivalent for the {name} ink: it shows as black"));
                [0.0, 0.0, 0.0, 1.0]
            }
        };
        plates.push(InkPlate { name, cmyk, data });
    }
    let separated = Separated::new(w, h, plates).ok_or("its plates can't be kept together")?;
    let clip = clip_path(main);
    // Points one pixel spans along each axis.
    let [a, b, c, d, ..] = xf.as_coeffs();
    let ppi = [a.hypot(b), c.hypot(d)].map(|l| (l > 1e-12).then(|| 72.0 / l));
    let ppi = match ppi {
        [Some(x), Some(y)] => Some((x, y)),
        _ => None,
    };
    Ok(Placed { separated, page, xf, clip, ppi, warnings })
}

/// The pixel grid the plates `images` go on: the first image's pixels, grown by whole pixels to
/// hold where every image prints, as its width, height and where its pixels print; `None` without
/// an image. It is the first image's own when its transform can't be inverted.
fn grid<'a>(mut images: impl Iterator<Item = &'a PlateImage>) -> Option<(u32, u32, Affine)> {
    let first = images.next()?;
    let own = Some((first.width, first.height, first.xf));
    if first.xf.determinant().abs() <= 1e-12 {
        return own;
    }
    let to_first = first.xf.inverse();
    let mut r = Rect::new(0.0, 0.0, f64::from(first.width), f64::from(first.height));
    for img in images {
        r = r.union((to_first * img.xf).transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(img.width), f64::from(img.height))));
    }
    // Edges within a millionth of a pixel of a whole one are on it.
    let snap = |v: f64, up: bool| {
        let n = v.round();
        if (v - n).abs() < 1e-6 {
            n
        } else if up {
            v.ceil()
        } else {
            v.floor()
        }
    };
    let (x0, y0, x1, y1) = (snap(r.x0, false), snap(r.y0, false), snap(r.x1, true), snap(r.y1, true));
    let size = |a: f64, b: f64| Some(b - a).filter(|n| n.is_finite() && (1.0..=f64::from(u32::MAX)).contains(n)).map(|n| n as u32);
    match (size(x0, x1), size(y0, y1)) {
        (Some(w), Some(h)) => Some((w, h, first.xf * Affine::translate((x0, y0)))),
        // Larger than any separated image may be ([`Separated::fits`]).
        _ => Some((u32::MAX, u32::MAX, first.xf)),
    }
}

/// The ink amounts of `img` on the `w` × `h` pixels that `onto` puts on the page (each the
/// nearest pixel of `img` where it prints there, none outside it). When either transform can't be
/// inverted, the pixels are stretched over one another instead.
fn resample(img: &PlateImage, w: u32, h: u32, onto: Affine) -> Vec<u8> {
    let (sw, sh) = (img.width as usize, img.height as usize);
    let mut out = Vec::with_capacity(w as usize * h as usize);
    let invertible = |m: Affine| m.determinant().abs() > 1e-12;
    // From a pixel of the image made to a pixel of `img`.
    let to_plate = (invertible(img.xf) && invertible(onto)).then(|| img.xf.inverse() * onto);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let at = match to_plate {
                Some(m) => {
                    let p = m * vectorcraft_geom::Point::new(x as f64 + 0.5, y as f64 + 0.5);
                    let inside = p.x >= 0.0 && p.y >= 0.0 && p.x < sw as f64 && p.y < sh as f64;
                    inside.then(|| (p.y as usize).min(sh - 1) * sw + (p.x as usize).min(sw - 1))
                }
                None => Some((y * sh / h as usize).min(sh - 1) * sw + (x * sw / w as usize).min(sw - 1)),
            };
            out.push(at.and_then(|i| img.data.get(i).copied()).unwrap_or(0));
        }
    }
    out
}

/// The first image plate program `eps` draws, as ink amounts (how dark each pixel is: 255 less its
/// luma), and where it prints on its page.
fn plate_image(eps: &[u8]) -> Result<PlateImage, String> {
    let r = crate::import_with(eps, false)?;
    if r.preview {
        return Err(r.warnings.first().cloned().unwrap_or_else(|| "its PostScript can't be read".into()));
    }
    let mut found = None;
    r.document.walk(|n| {
        if found.is_none()
            && let NodeKind::Image(im) = &n.kind
        {
            found = Some((im.key.clone(), im.xf, im.width, im.height));
        }
    });
    let (key, xf, iw, ih) = found.ok_or("it draws no image")?;
    let blob = r.document.images.get(&key).ok_or("it draws no image")?;
    let (width, height, data) = blob.coverage().ok_or("its image is too large to read")?;
    // The object's pixel space may differ from the pixels it keeps.
    let xf = xf * Affine::scale_non_uniform(f64::from(iw) / f64::from(width.max(1)), f64::from(ih) / f64::from(height.max(1)));
    Ok(PlateImage { width, height, data, xf })
}

/// The clipping path the first image of `main` is drawn through, when it is drawn through one.
fn clip_path(main: &[u8]) -> Option<Node> {
    let r = crate::import_with(main, false).ok().filter(|r| !r.preview)?;
    first_clip(&r.document)
}

/// The clipping path of the innermost clipping group of `d` that holds an image.
fn first_clip(d: &Document) -> Option<Node> {
    fn find(n: &Node) -> Option<Node> {
        let NodeKind::Group { children, clip } = &n.kind else {
            return n.children().and_then(|ch| ch.iter().find_map(|c| find(c)));
        };
        if let Some(inner) = children.iter().find_map(|c| find(c)) {
            return Some(inner);
        }
        let holds_image = || {
            let mut any = false;
            n.walk(&mut |c: &Node| any |= matches!(c.kind, NodeKind::Image(_)));
            any
        };
        let first = children.first()?;
        (*clip && matches!(first.kind, NodeKind::Path { clipping: true, .. }) && holds_image()).then(|| (**first).clone())
    }
    d.layers.iter().find_map(|l| find(l))
}

#[cfg(test)]
mod tests;
