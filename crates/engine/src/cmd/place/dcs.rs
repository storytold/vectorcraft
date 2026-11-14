//! DCS files (`file.place` with Link): an EPS file with one program per printing plate
//! ([`vectorcraft_eps::dcs`]). It is placed as one linked image whose plates are kept with its
//! pixels ([`vectorcraft_doc::separated`]): the screen and composite output show the plates mixed
//! into CMYK, and print separations put each plate on its own ink. Its spot plates become spot
//! swatches, and the clipping path its composite is drawn through clips it. A DCS file can't be
//! opened or embedded (Embed, Place without Link): the image stays linked to its file, which the
//! Links commands read again ([`raster`]).

use vectorcraft_color::swatch::Swatch;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::separated::PlateFile;
use vectorcraft_doc::{Document, ImageBlob, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect};

use super::super::fileio::{self, RasterImage};
use super::super::{EngineError, Result};

/// The largest plate file read (bytes).
const MAX_PLATE_FILE: u64 = 1 << 30;

/// What refuses to open or embed a DCS file.
pub(crate) const LINK_ONLY: &str = "DCS files can only be placed linked: use File › Place with Link on";

/// A DCS file read for placing.
pub(crate) struct DcsArt {
    pub img: RasterImage,
    /// The image's pixels onto the file's page (its bounding box, top-left at the origin).
    pub xf: Affine,
    pub page: Rect,
    /// The clipping path the composite is drawn through, on the page.
    pub clip: Option<Node>,
    /// The spot plates' inks and their process equivalents.
    pub spots: Vec<(String, [f32; 4])>,
    pub warnings: Vec<String>,
}

/// Is `bytes` a DCS file (an EPS file naming its plates)?
pub(crate) fn is_dcs(bytes: &[u8]) -> bool {
    vectorcraft_eps::dcs::read(bytes).is_some()
}

/// The most bytes read from the start of a file to look at its DCS header.
pub(crate) const HEAD_BYTES: usize = 1 << 20;

/// Does the header of the PostScript in file `head` (the first bytes of a file `size` bytes long,
/// at most [`HEAD_BYTES`]) name DCS plates? `None` when `head` can't tell: that PostScript doesn't
/// start within it (a binary EPS header that puts it further on), or its header goes on past it. A
/// `head` that holds the whole file is read as [`is_dcs`] reads it, and a binary EPS header whose
/// PostScript or TIFF preview ends past `size` names none, as [`is_dcs`] finds for the whole file.
pub(crate) fn head_is_dcs(head: &[u8], size: u64) -> Option<bool> {
    const DOS_MAGIC: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];
    if head.len() < HEAD_BYTES || head.len() as u64 >= size {
        return Some(is_dcs(head));
    }
    let (ps, whole) = if head.starts_with(&DOS_MAGIC) {
        let word = |at: usize| head.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(|b| u32::from_le_bytes(b) as usize);
        let fits = |at: usize| Some((word(at)? as u64).checked_add(word(at + 4)? as u64).is_some_and(|end| end <= size));
        if !fits(4)? || (word(24)? != 0 && !fits(20)?) {
            return Some(false);
        }
        let (start, len) = (word(4)?, word(8)?);
        let ps = head.get(start..).filter(|ps| !ps.is_empty())?;
        (ps.get(..len).unwrap_or(ps), start.checked_add(len).is_some_and(|end| end <= head.len()))
    } else {
        (head, false)
    };
    if is_dcs(ps) {
        return Some(true);
    }
    (whole || header_ended(ps)).then_some(false)
}

/// Does `ps`, the start of some PostScript, hold all of the header [`vectorcraft_eps::dcs::read`]
/// looks at (up to `%%EndComments`, a line that isn't a comment, or its 10,000th line)? Its last
/// line may be cut off, so it doesn't count.
fn header_ended(ps: &[u8]) -> bool {
    let mut lines = ps.split(|b| matches!(b, b'\n' | b'\r')).filter(|l| !l.is_empty()).peekable();
    let mut n = 0;
    while let Some(line) = lines.next() {
        if lines.peek().is_none() {
            return false;
        }
        n += 1;
        if line.starts_with(b"%%EndComments") || !line.starts_with(b"%") || n >= 10_000 {
            return true;
        }
    }
    false
}

/// DCS file `bytes`, read from `path` (when there is one: plate files of their own are found in its
/// folder), with every plate.
pub(crate) fn read(bytes: &[u8], path: Option<&str>) -> Result<DcsArt> {
    let dcs = vectorcraft_eps::dcs::read(bytes).ok_or_else(|| EngineError::Other("not a DCS file".into()))?;
    let mut files = vec![];
    let mut file = |name: &str| plate_file(path, name, &mut files);
    let p = vectorcraft_eps::dcs::place(bytes, &dcs, &mut file).map_err(EngineError::Other)?;
    // The clipping path in the image's pixel space, where the Links commands put it again; the
    // image keeps it, so they know a clip group of that path as the file's.
    let to_pixels = (p.xf.determinant().abs() > 1e-12).then(|| p.xf.inverse());
    let clip = match (&p.clip, to_pixels) {
        (Some(Node { kind: NodeKind::Path { path, .. }, .. }), Some(inv)) => Some(path.transformed(inv)),
        _ => None,
    };
    let blob = ImageBlob::separated_tiff_clipped(&p.separated, p.ppi, &files, clip.as_ref())
        .ok_or_else(|| EngineError::Other("the plates can't be kept".into()))?;
    let img = RasterImage { key: blob.content_key(), blob, width: p.separated.width, height: p.separated.height, ppi: p.ppi, clip };
    let spots = p.separated.spots().map(|s| (s.name.clone(), s.cmyk)).collect();
    Ok(DcsArt { img, xf: p.xf, page: p.page, clip: p.clip, spots, warnings: p.warnings })
}

/// A linked file's image, as the Links commands read it again: a DCS file's plates
/// ([`read`]), else the image file ([`fileio::raster_image`]).
pub(crate) fn raster(bytes: &[u8], path: Option<&str>) -> Result<RasterImage> {
    if is_dcs(bytes) { read(bytes, path).map(|a| a.img) } else { fileio::raster_image(bytes) }
}

/// The path of the plate file a DCS file at `main` names `name`: the file of that name (its last
/// component, of any system's path) in `main`'s folder, with that name.
fn plate_path(main: Option<&str>, name: &str) -> std::result::Result<(String, String), String> {
    let base = name.rsplit(['/', '\\', ':']).next().unwrap_or(name);
    let folder = main.map(std::path::Path::new).and_then(std::path::Path::parent);
    let Some(folder) = folder.filter(|_| !base.is_empty()) else {
        return Err(format!("`{name}` is a file of its own, which can only be found next to the DCS file's path"));
    };
    Ok((folder.join(base).to_string_lossy().into_owned(), base.to_string()))
}

/// The path of the plate file named `name` that the DCS file at `main` (a path as a link keeps
/// it) was read with ([`plate_path`]); `None` when `main` has no folder.
pub(crate) fn plate_file_path(main: &str, name: &str) -> Option<String> {
    plate_path(Some(main), name).ok().map(|(path, _)| path)
}

/// The plate file a DCS file at `main` names `name` ([`plate_path`]), noted in `files` as it is now.
fn plate_file(main: Option<&str>, name: &str, files: &mut Vec<PlateFile>) -> std::result::Result<Vec<u8>, String> {
    let (path, base) = plate_path(main, name)?;
    match fileio::file_stamp(&path) {
        None => Err(format!("`{base}` is not in the DCS file's folder")),
        Some((size, _)) if size > MAX_PLATE_FILE => Err(format!("`{base}` is larger than {} MB", MAX_PLATE_FILE >> 20)),
        Some((size, modified)) => {
            let bytes = fileio::read_file(&path).map_err(|e| e.to_string())?;
            files.push(PlateFile { name: base, size, modified: modified.unwrap_or(0) });
            Ok(bytes)
        }
    }
}

/// Whether one of `files`, the plate files of their own a separated image was read from
/// ([`ImageBlob::plate_files`]), next to the DCS file at `main`, is gone or changed since.
pub(crate) fn plate_files_changed(files: &[PlateFile], main: &str) -> bool {
    files.iter().any(|f| {
        let stamp = plate_path(Some(main), &f.name).ok().and_then(|(path, _)| fileio::file_stamp(&path));
        stamp.is_none_or(|(size, modified)| size != f.size || modified.unwrap_or(0) != f.modified)
    })
}

/// Add `spots` to `d` as spot swatches, each whose name no swatch has, in its process equivalent.
pub(crate) fn add_spot_swatches(d: &mut Document, spots: &[(String, [f32; 4])]) {
    for (name, [c, m, y, k]) in spots {
        if d.swatch(name).is_none() {
            d.swatches.push(Swatch { name: name.clone(), paint: Paint::solid(Color::cmyk(*c, *m, *y, *k)), global: true, spot: true });
        }
    }
}

/// A warning for each of `spots` whose name a swatch of `d` already has that isn't a spot color in
/// the same process equivalent: the document's swatch stays, and the plate prints by it.
pub(crate) fn swatch_notes(d: &Document, spots: &[(String, [f32; 4])]) -> Vec<String> {
    let mut out = vec![];
    for (name, cmyk) in spots {
        let Some(sw) = d.swatch(name) else { continue };
        let same = sw.spot
            && matches!(sw.paint.color(), Some(Color::Cmyk { c, m, y, k }) if [c, m, y, k].iter().zip(cmyk).all(|(a, b)| (a - b).abs() < 0.005));
        if !same {
            let how = if sw.spot { "a different color" } else { "a process color, so the plate prints as process inks" };
            out.push(format!("the document's swatch `{name}` is kept: it is {how}"));
        }
    }
    out
}
