//! Separated images: an image whose inks come as one plate each (a DCS file placed linked), kept
//! as a TIFF of several images. The first is the composite, the plates mixed into CMYK
//! ([`Separated::composite`]), which every reader of a CMYK TIFF sees. Each later image is one
//! plate's ink amounts in grayscale (0 is no ink), with its ink's name and CMYK equivalent in its
//! ImageDescription. Print separations take each plate from these ([`Separated::coverage`]).

use std::io::Cursor;
use std::sync::Arc;

use vectorcraft_color::cms::PROCESS_PLATES;

use crate::ImageBlob;
use crate::cmyk::{Inks, MAX_CMYK_PIXELS};

/// Most plates a separated image holds.
pub const MAX_PLATES: usize = 64;
/// Longest ink name kept (bytes).
pub const MAX_NAME: usize = 255;
/// Most ink amounts the plates of one image hold together (512 MB).
pub const MAX_SAMPLES: u64 = 1 << 29;
/// How a plate's ImageDescription starts.
const MARK: &str = "VectorCraft plate ";

/// How the composite's ImageDescription starts when plates were read from files of their own.
const FILES_MARK: &str = "VectorCraft plate files";

/// A plate file of its own that a separated image was read from, as it was then: a change to it
/// changes the image as a change to the DCS file does.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateFile {
    /// Its file name, in the DCS file's folder.
    pub name: String,
    /// Its size (bytes).
    pub size: u64,
    /// Its modification time (ms since the Unix epoch; 0 when the file system keeps none).
    pub modified: u64,
}

/// `text` in hexadecimal (TIFF text is ASCII).
fn hex(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

/// The text [`hex`] wrote, of at most `max` bytes.
fn unhex(hex: &str, max: usize) -> Option<String> {
    if !hex.len().is_multiple_of(2) || hex.len() > max * 2 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

/// One ink's plate: its ink amounts, one byte a pixel (0 is none, 255 is solid), row by row from
/// the top.
#[derive(Clone, Debug, PartialEq)]
pub struct InkPlate {
    /// "Cyan", "Magenta", "Yellow", "Black", or a spot ink's name.
    pub name: String,
    /// The process inks the ink stands for (C, M, Y, K, 0..1): one process ink at 100% for a
    /// process plate.
    pub cmyk: [f32; 4],
    pub data: Vec<u8>,
}

impl InkPlate {
    /// Is it a spot ink's plate (not one of the four process inks)?
    pub fn spot(&self) -> bool {
        !PROCESS_PLATES.contains(&self.name.as_str())
    }
}

/// The plates of a separated image, all of one size.
#[derive(Clone, Debug, PartialEq)]
pub struct Separated {
    pub width: u32,
    pub height: u32,
    pub plates: Vec<InkPlate>,
}

impl Separated {
    /// `plates` as a `width` × `height` image; `None` when there are none or more than
    /// [`MAX_PLATES`], two have the same name, a name is empty or longer than [`MAX_NAME`], a plate
    /// doesn't hold that many pixels, an equivalent isn't within 0..1, or the image is empty, has
    /// more than [`MAX_CMYK_PIXELS`], or its plates together hold more than [`MAX_SAMPLES`]
    /// ([`Self::fits`]). A process plate's equivalent is its own ink.
    pub fn new(width: u32, height: u32, mut plates: Vec<InkPlate>) -> Option<Self> {
        let n = u64::from(width) * u64::from(height);
        if !Self::fits(width, height, plates.len()) {
            return None;
        }
        for p in plates.iter_mut() {
            if p.name.is_empty() || p.name.len() > MAX_NAME || p.data.len() as u64 != n {
                return None;
            }
            if let Some(j) = PROCESS_PLATES.iter().position(|q| *q == p.name) {
                p.cmyk = [0.0; 4];
                p.cmyk[j] = 1.0;
            }
            if !p.cmyk.iter().all(|v| (0.0..=1.0).contains(v)) {
                return None;
            }
        }
        let mut names = std::collections::HashSet::new();
        if !plates.iter().all(|p| names.insert(p.name.as_str())) {
            return None;
        }
        Some(Self { width, height, plates })
    }

    /// May an image of `width` × `height` pixels hold `plates` plates? Not when it is empty, has
    /// more than [`MAX_CMYK_PIXELS`], has no plate or more than [`MAX_PLATES`], or the plates
    /// together would hold more than [`MAX_SAMPLES`].
    pub fn fits(width: u32, height: u32, plates: usize) -> bool {
        let n = u64::from(width) * u64::from(height);
        n > 0 && n <= MAX_CMYK_PIXELS && (1..=MAX_PLATES).contains(&plates) && n * plates as u64 <= MAX_SAMPLES
    }

    /// The plates of spot inks.
    pub fn spots(&self) -> impl Iterator<Item = &InkPlate> {
        self.plates.iter().filter(|p| p.spot())
    }

    /// The plates mixed into process inks: each plate's ink amount times its equivalent, the
    /// plates over one another as inks print (`1 − (1 − a)(1 − b)` for each process ink).
    pub fn composite(&self) -> Inks {
        let n = self.data_len();
        let mut data = vec![0u8; n * 4];
        for (i, px) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let mut clear = [1.0f32; 4];
            for p in &self.plates {
                let t = f32::from(p.data[i]) / 255.0;
                for (c, e) in clear.iter_mut().zip(p.cmyk) {
                    *c *= 1.0 - t * e;
                }
            }
            for (o, c) in px.iter_mut().zip(clear) {
                *o = ((1.0 - c) * 255.0).round() as u8;
            }
        }
        Inks { width: self.width, height: self.height, data }
    }

    /// The ink amounts that print on plate `plate` (a process ink's name or a spot ink's), one byte
    /// a pixel. A spot plate for which `own` is true prints on its own plate only; every other spot
    /// plate (the job prints no plate of that name) prints as its process equivalent, mixed as
    /// [`Self::composite`] mixes it. A plate name the image has no plate for, and a spot plate that
    /// prints as process, print nothing.
    pub fn coverage(&self, plate: &str, own: &dyn Fn(&str) -> bool) -> Vec<u8> {
        let n = self.data_len();
        if let Some(j) = PROCESS_PLATES.iter().position(|q| *q == plate) {
            let mixed: Vec<&InkPlate> = self.plates.iter().filter(|p| p.cmyk[j] > 0.0 && (!p.spot() || !own(&p.name))).collect();
            return (0..n)
                .map(|i| {
                    let clear = mixed.iter().fold(1.0f32, |c, p| c * (1.0 - f32::from(p.data[i]) / 255.0 * p.cmyk[j]));
                    ((1.0 - clear) * 255.0).round() as u8
                })
                .collect();
        }
        match self.plates.iter().find(|p| p.name == plate && p.spot() && own(&p.name)) {
            Some(p) => p.data.clone(),
            None => vec![0; n],
        }
    }

    /// [`Self::coverage`] as a grayscale PNG, black for solid ink and white for none, as a plate
    /// prints; `None` when it can't be written.
    pub fn plate_png(&self, plate: &str, own: &dyn Fn(&str) -> bool) -> Option<ImageBlob> {
        let gray: Vec<u8> = self.coverage(plate, own).into_iter().map(|v| 255 - v).collect();
        let img = image::GrayImage::from_raw(self.width, self.height, gray)?;
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(ImageBlob::png(png))
    }

    fn data_len(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// A plate's ImageDescription: the mark, its equivalent and its name in hexadecimal (TIFF text is
/// ASCII).
fn description(p: &InkPlate) -> String {
    let [c, m, y, k] = p.cmyk;
    format!("{MARK}{c} {m} {y} {k} {}", hex(&p.name))
}

/// A plate's name and equivalent from its ImageDescription; `None` when it isn't one.
fn parse_description(s: &str) -> Option<(String, [f32; 4])> {
    let rest = s.strip_prefix(MARK)?;
    let mut words = rest.split(' ');
    let mut cmyk = [0.0f32; 4];
    for v in &mut cmyk {
        *v = words.next()?.parse().ok().filter(|v: &f32| v.is_finite())?;
    }
    let name = unhex(words.next()?, MAX_NAME)?;
    words.next().is_none().then_some((name, cmyk))
}

/// Longest plate file name kept (bytes).
const MAX_FILE_NAME: usize = 4096;

/// The composite's ImageDescription for `files`: the mark, then each file's name in hexadecimal,
/// its size and its modification time.
fn files_description(files: &[PlateFile]) -> String {
    let mut out = String::from(FILES_MARK);
    for f in files.iter().take(MAX_PLATES) {
        out.push_str(&format!(" {} {} {}", hex(&f.name), f.size, f.modified));
    }
    out
}

/// The plate files of a composite's ImageDescription; none when it isn't one.
fn parse_files(s: &str) -> Vec<PlateFile> {
    let Some(rest) = s.strip_prefix(FILES_MARK) else { return vec![] };
    let words: Vec<&str> = rest.split(' ').filter(|w| !w.is_empty()).collect();
    let file =
        |w: &[&str]| Some(PlateFile { name: unhex(w.first()?, MAX_FILE_NAME)?, size: w.get(1)?.parse().ok()?, modified: w.get(2)?.parse().ok()? });
    words.chunks(3).take(MAX_PLATES).filter_map(file).collect()
}

impl ImageBlob {
    /// A TIFF of separated image `s`: its composite in CMYK, then its plates (Deflate-compressed),
    /// declaring the resolution `ppi` when it is given and both are finite and above 0, and noting
    /// `files`, the plate files of their own the plates were read from ([`Self::plate_files`]);
    /// `None` when it can't be written.
    pub fn separated_tiff(s: &Separated, ppi: Option<(f64, f64)>, files: &[PlateFile]) -> Option<Self> {
        use tiff::encoder::{Compression, Rational, TiffEncoder, colortype, compression::DeflateLevel};
        use tiff::tags::{ResolutionUnit, Tag};
        let ppi = ppi.filter(|(x, y)| [x, y].iter().all(|v| v.is_finite() && **v > 0.0));
        // Ten-thousandths of a pixel per inch.
        let r = |v: f64| Rational { n: (v * 10_000.0).round() as u32, d: 10_000 };
        let mut out = Cursor::new(Vec::new());
        let mut enc = TiffEncoder::new(&mut out).ok()?.with_compression(Compression::Deflate(DeflateLevel::Balanced));
        let mut img = enc.new_image::<colortype::CMYK8>(s.width, s.height).ok()?;
        if let Some((x, y)) = ppi {
            img.encoder().write_tag(Tag::XResolution, r(x)).ok()?;
            img.encoder().write_tag(Tag::YResolution, r(y)).ok()?;
            img.encoder().write_tag(Tag::ResolutionUnit, ResolutionUnit::Inch).ok()?;
        }
        if !files.is_empty() {
            img.encoder().write_tag(Tag::ImageDescription, files_description(files).as_str()).ok()?;
        }
        img.write_data(&s.composite().data).ok()?;
        for p in &s.plates {
            // White is no ink, so a viewer that shows a plate shows it as it prints.
            let mut img = enc.new_image::<colortype::Gray8>(s.width, s.height).ok()?;
            img.encoder().write_tag(Tag::ImageDescription, description(p).as_str()).ok()?;
            let inverted: Vec<u8> = p.data.iter().map(|v| 255 - v).collect();
            img.write_data(&inverted).ok()?;
        }
        Some(Self::new("image/tiff", out.into_inner()))
    }

    /// Does the image hold plates ([`ImageBlob::separated_tiff`])? Reads only the TIFF's
    /// directories.
    pub fn is_separated(&self) -> bool {
        let Some((_, mut d)) = crate::cmyk::tiff(&self.bytes) else { return false };
        while d.more_images() {
            if d.next_image().is_err() {
                return false;
            }
            if d.get_tag_ascii_string(tiff::tags::Tag::ImageDescription).ok().is_some_and(|s| s.starts_with(MARK)) {
                return true;
            }
        }
        false
    }

    /// The plate files of their own a separated image was read from, as they were then; none for
    /// other images. Reads only the TIFF's first directory.
    pub fn plate_files(&self) -> Vec<PlateFile> {
        let Some((_, mut d)) = crate::cmyk::tiff(&self.bytes) else { return vec![] };
        d.get_tag_ascii_string(tiff::tags::Tag::ImageDescription).ok().as_deref().map(parse_files).unwrap_or_default()
    }

    /// The spot inks a separated image has plates for, each with its CMYK equivalent; none for
    /// other images. Reads only the TIFF's directories.
    pub fn spot_plates(&self) -> Vec<(String, [f32; 4])> {
        let Some((_, mut d)) = crate::cmyk::tiff(&self.bytes) else { return vec![] };
        let mut inks = vec![];
        while d.more_images() && inks.len() < MAX_PLATES {
            if d.next_image().is_err() {
                break;
            }
            let plate = d.get_tag_ascii_string(tiff::tags::Tag::ImageDescription).ok().as_deref().and_then(parse_description);
            inks.extend(plate.filter(|(name, _)| !PROCESS_PLATES.contains(&name.as_str())));
        }
        inks
    }

    /// The plates of a separated image; `None` for other images, and ones whose plates can't be
    /// read. The plates of the last image read are kept for the next call (print separates one
    /// image plate after plate).
    pub fn separated(&self) -> Option<Arc<Separated>> {
        thread_local! {
            static LAST: std::cell::RefCell<Option<(String, Arc<Separated>)>> = const { std::cell::RefCell::new(None) };
        }
        if !self.is_separated() {
            return None;
        }
        let key = self.content_key();
        LAST.with_borrow_mut(|last| match last {
            Some((k, s)) if *k == key => Some(s.clone()),
            _ => {
                let s = Arc::new(read_plates(&self.bytes)?);
                *last = Some((key, s.clone()));
                Some(s)
            }
        })
    }
}

/// The plates of TIFF `bytes`, each the size of its first image.
fn read_plates(bytes: &[u8]) -> Option<Separated> {
    let (_, mut d) = crate::cmyk::tiff(bytes)?;
    let (w, h) = d.dimensions().ok()?;
    let n = usize::try_from(u64::from(w) * u64::from(h)).ok()?;
    let mut plates = vec![];
    while d.more_images() && plates.len() < MAX_PLATES {
        d.next_image().ok()?;
        let Some((name, cmyk)) = d.get_tag_ascii_string(tiff::tags::Tag::ImageDescription).ok().as_deref().and_then(parse_description) else {
            continue;
        };
        // Checked before the plate is decoded.
        if d.dimensions().ok()? != (w, h) || !matches!(d.colortype().ok()?, tiff::ColorType::Gray(8)) || !Separated::fits(w, h, plates.len() + 1) {
            return None;
        }
        let data = crate::cmyk::read_samples(&mut d, n, 1)?.into_iter().map(|v| 255 - v).collect();
        plates.push(InkPlate { name, cmyk, data });
    }
    Separated::new(w, h, plates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate(name: &str, cmyk: [f32; 4], data: &[u8]) -> InkPlate {
        InkPlate { name: name.into(), cmyk, data: data.to_vec() }
    }

    /// Two pixels: cyan 100% and a spot ink at 50% on the first, nothing on the second.
    fn two() -> Separated {
        let plates = vec![plate("Cyan", [0.0; 4], &[255, 0]), plate("Spot Orange", [0.0, 0.6, 1.0, 0.0], &[128, 0])];
        Separated::new(2, 1, plates).unwrap()
    }

    #[test]
    fn plates_mix_into_the_composite_as_inks_print() {
        let s = two();
        assert_eq!(s.plates[0].cmyk, [1.0, 0.0, 0.0, 0.0], "a process plate stands for its own ink");
        let c = s.composite();
        // Magenta: 0.6 × 128/255; yellow: 128/255.
        assert_eq!(&c.data[..4], &[255, 77, 128, 0]);
        assert_eq!(&c.data[4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn a_spot_plate_prints_on_its_own_plate_or_as_process() {
        let s = two();
        let own = |_: &str| true;
        assert_eq!(s.coverage("Spot Orange", &own), [128, 0]);
        assert_eq!(s.coverage("Magenta", &own), [0, 0], "nothing of the spot on a process plate");
        assert_eq!(s.coverage("Cyan", &own), [255, 0]);
        assert_eq!(s.coverage("Black", &own), [0, 0]);
        let process = |_: &str| false;
        assert_eq!(s.coverage("Spot Orange", &process), [0, 0]);
        assert_eq!(s.coverage("Magenta", &process), [77, 0]);
        assert_eq!(s.coverage("Yellow", &process), [128, 0]);
        assert_eq!(s.coverage("Other", &own), [0, 0]);
    }

    #[test]
    fn the_tiff_keeps_every_plate_and_shows_the_composite() {
        let s = two();
        let files =
            [PlateFile { name: "pic (1).C".into(), size: 1234, modified: 1_700_000_000_000 }, PlateFile { name: "ü.K".into(), size: 0, modified: 0 }];
        let b = ImageBlob::separated_tiff(&s, Some((300.0, 300.0)), &files).unwrap();
        assert_eq!(b.plate_files(), files);
        assert!(ImageBlob::separated_tiff(&s, None, &[]).unwrap().plate_files().is_empty());
        assert!(b.is_separated());
        assert!(b.is_cmyk(), "readers of a CMYK TIFF see the composite");
        assert_eq!(b.cmyk(), Some(s.composite()));
        assert_eq!(*b.separated().unwrap(), s);
        assert_eq!(b.spot_plates(), [("Spot Orange".to_string(), [0.0, 0.6, 1.0, 0.0])]);
        assert!(ImageBlob::cmyk_tiff(&s.composite()).unwrap().spot_plates().is_empty());
        assert!(!ImageBlob::cmyk_tiff(&s.composite()).unwrap().is_separated());
    }

    #[test]
    fn names_that_are_not_ascii_are_kept() {
        let s = Separated::new(1, 1, vec![plate("Grün 100 % (A)", [0.5, 0.0, 1.0, 0.0], &[9])]).unwrap();
        let b = ImageBlob::separated_tiff(&s, None, &[]).unwrap();
        assert_eq!(b.separated().unwrap().plates[0].name, "Grün 100 % (A)");
    }

    #[test]
    fn bad_plates_are_refused() {
        assert!(Separated::new(2, 1, vec![]).is_none(), "no plates");
        assert!(Separated::new(2, 1, vec![plate("Cyan", [0.0; 4], &[1])]).is_none(), "too few pixels");
        assert!(Separated::new(1, 1, vec![plate("A", [0.0; 4], &[1]), plate("A", [0.0; 4], &[1])]).is_none(), "the same name twice");
        assert!(Separated::new(1, 1, vec![plate("", [0.0; 4], &[1])]).is_none(), "no name");
        assert!(Separated::new(1, 1, vec![plate("A", [2.0, 0.0, 0.0, 0.0], &[1])]).is_none(), "equivalent past 100%");
        assert!(Separated::new(0, 1, vec![plate("A", [0.0; 4], &[])]).is_none(), "empty");
        let many: Vec<InkPlate> = (0..=MAX_PLATES).map(|i| plate(&format!("S{i}"), [0.0; 4], &[1])).collect();
        assert!(Separated::new(1, 1, many).is_none(), "too many plates");
        assert!(Separated::fits(1 << 13, 1 << 13, 8), "512 MB of ink amounts");
        assert!(!Separated::fits(1 << 13, 1 << 13, 9), "more than 512 MB together");
        assert!(!Separated::fits(1 << 14, 1 << 13, 1), "more pixels than a CMYK image is decoded with");
    }
}
