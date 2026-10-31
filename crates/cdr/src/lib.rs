//! CorelDRAW (`.cdr`) compatibility.
//!
//! CDR has no official public specification. Rather than a speculative binary parser, this crate
//! builds on the clean-room reader and writer of the TraceDraw project (`tracedraw-cdr`, MIT OR
//! Apache-2.0, pinned to a commit), which covers:
//!
//! - **RIFF** (versions 7 to 13; before 6 with 16-bit coordinates), with zlib-compressed `cmpr`
//!   lists;
//! - the **ZIP** packages of versions 14 and 15 (`content/riffData.cdr`) and of version 16 on
//!   (`content/root.dat`, `dataFileList.dat` and 16-byte redirect records).
//!
//! What lives here is VectorCraft's side: signature and version detection and validation, limits
//! for untrusted files, isolation of internal reader failures (they never take the app down), the
//! conversion between the two document models (geometry, colours, transparency, layers, groups,
//! clipping, type, images, pages) and the diagnostics: every loss becomes a warning, nothing is
//! rasterized silently, and the compatibility matrix ([`matrix`]) says what is supported.
//!
//! - [`detect_version`] / [`is_cdr`]: signature and version;
//! - [`validate`]: the whole file is read, without building a document;
//! - [`import`]: a VectorCraft document (Open; the engine places the same for Place);
//! - [`export`]: writes the version asked for and, by default, validates the written file by
//!   reading it back.

#![forbid(unsafe_code)]

pub mod matrix;
mod read;
mod write;

#[cfg(test)]
mod tests;

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub use read::{ImportOptions, Imported};
pub use write::{ExportOptions, Exported, Png, Previews};

/// The largest file read (bytes): anything larger is refused before reading starts.
pub const MAX_FILE_BYTES: usize = 1 << 30;
/// The oldest version the writer produces.
pub const OLDEST_EXPORT_VERSION: u16 = tracedraw_cdr::write::OLDEST_VERSION;
/// The newest version the writer produces.
pub const NEWEST_EXPORT_VERSION: u16 = tracedraw_cdr::write::NEWEST_VERSION;
/// From this version on, files the writer produces were opened in CorelDRAW by the upstream
/// project's tests (type included). Older ones are experimental.
pub const VERIFIED_SINCE: u16 = tracedraw_cdr::write::CHECKED_SINCE;
/// The default export version: the oldest verified one, which the widest range of CorelDRAW
/// releases opens (X6 and later).
pub const DEFAULT_EXPORT_VERSION: u16 = VERIFIED_SINCE;
/// The newest version the reader has seen in real files; newer ones are read with a warning.
pub const NEWEST_KNOWN_VERSION: u16 = 26;

/// Why a CDR file couldn't be read or written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The signature isn't a CorelDRAW document's.
    NotCdr,
    /// A container or version the reader doesn't handle.
    Unsupported(String),
    /// Truncated, badly compressed or inconsistent data.
    Damaged(String),
    /// Larger than [`MAX_FILE_BYTES`].
    TooLarge(usize),
    /// The document can't be exported (no artboard, version out of range…).
    Export(String),
    /// The written file failed post-export validation: nothing was written.
    Validation(String),
    /// An internal failure of the reader or the writer (contained: the app carries on).
    Internal(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCdr => write!(f, "not a CorelDRAW (.cdr) document: no RIFF/CDR signature or CorelDRAW ZIP package"),
            Self::Unsupported(w) => write!(f, "unsupported CorelDRAW document: {w}"),
            Self::Damaged(w) => write!(f, "the CorelDRAW document is damaged or incomplete: {w}"),
            Self::TooLarge(n) => write!(f, "the file is {} MB; CorelDRAW documents are read up to {} MB", n >> 20, MAX_FILE_BYTES >> 20),
            Self::Export(w) => write!(f, "can't export to CorelDRAW: {w}"),
            Self::Validation(w) => write!(f, "the CDR file failed validation and wasn't written: {w}"),
            Self::Internal(w) => write!(f, "internal error while processing the CorelDRAW document ({w}); the file may be damaged"),
        }
    }
}

impl std::error::Error for Error {}

impl From<tracedraw_cdr::Error> for Error {
    fn from(e: tracedraw_cdr::Error) -> Self {
        use tracedraw_cdr::Error as E;
        match e {
            E::NotCdr | E::NotCmx => Self::NotCdr,
            E::UnsupportedContainer(w) => Self::Unsupported(w),
            E::Truncated(at) => Self::Damaged(format!("data cut short at byte {at}")),
            E::Io(e) => Self::Damaged(e.to_string()),
            E::Zip(e) => Self::Damaged(format!("invalid ZIP package: {e}")),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Runs the third-party reader or writer with any panic contained (hostile data can lead code
/// outside this workspace down a path nobody expected): it becomes [`Error::Internal`], never a
/// crash.
pub(crate) fn guarded<T>(what: &str, f: impl FnOnce() -> T) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
        Error::Internal(if msg.is_empty() { what.to_string() } else { format!("{what}: {msg}") })
    })
}

/// The container of a CDR file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    /// A RIFF file (versions up to 13).
    Riff,
    /// A ZIP package holding `content/riffData.cdr` (versions 14 and 15).
    ZipRiffData,
    /// A ZIP package holding `content/root.dat` and external streams (version 16 on).
    ZipRootDat,
}

impl Container {
    /// The container's name, as shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Riff => "RIFF",
            Self::ZipRiffData => "ZIP (content/riffData.cdr)",
            Self::ZipRootDat => "ZIP (content/root.dat)",
        }
    }
}

/// The signature, version and container of a CDR file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CdrInfo {
    /// The format's major version (7 to 26…), as the RIFF form type encodes it.
    pub version: u16,
    pub container: Container,
    /// The product that writes that version ("CorelDRAW X6"…).
    pub product: String,
    /// The reader handles this version (versions before 6 and after [`NEWEST_KNOWN_VERSION`]
    /// are read with reservations).
    pub supported: bool,
    /// A note on how this version is supported.
    pub note: Option<String>,
}

/// The name of the CorelDRAW release that writes format version `v`.
pub fn product_name(v: u16) -> String {
    match v {
        13 => "CorelDRAW X3".into(),
        14 => "CorelDRAW X4".into(),
        15 => "CorelDRAW X5".into(),
        16 => "CorelDRAW X6".into(),
        17 => "CorelDRAW X7".into(),
        18 => "CorelDRAW X8".into(),
        19..=26 => format!("CorelDRAW {}", 2017 + u32::from(v - 19)),
        0 => "CorelDRAW (unknown version)".into(),
        v => format!("CorelDRAW {v}"),
    }
}

/// How an export version is offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportStatus {
    /// Files of this version opened in CorelDRAW in the upstream project's tests.
    Verified,
    /// Written with the layout of that version's sample files, not tested in CorelDRAW.
    Experimental,
}

/// A version the export offers.
#[derive(Clone, Debug)]
pub struct ExportVersion {
    pub version: u16,
    pub product: String,
    pub container: Container,
    pub status: ExportStatus,
}

/// The versions [`export`] writes, oldest first.
pub fn export_versions() -> Vec<ExportVersion> {
    (OLDEST_EXPORT_VERSION..=NEWEST_EXPORT_VERSION)
        .map(|v| ExportVersion {
            version: v,
            product: product_name(v),
            container: container_of(v),
            status: if v >= VERIFIED_SINCE { ExportStatus::Verified } else { ExportStatus::Experimental },
        })
        .collect()
}

/// The container a file of format version `v` uses.
fn container_of(v: u16) -> Container {
    match v {
        ..=13 => Container::Riff,
        14 | 15 => Container::ZipRiffData,
        _ => Container::ZipRootDat,
    }
}

const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// Do the bytes look like a CorelDRAW document? Cheap: the RIFF signature (`CDR?`/`cdr?`) or, in a
/// ZIP, the name of a CorelDRAW package member (nothing is decompressed).
pub fn is_cdr(bytes: &[u8]) -> bool {
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") {
        return bytes.get(8..11).is_some_and(|f| f.eq_ignore_ascii_case(b"cdr"));
    }
    bytes.starts_with(ZIP_MAGIC) && zip_names_cdr(bytes)
}

/// Does the ZIP name the RIFF stream of a CorelDRAW package?
fn zip_names_cdr(bytes: &[u8]) -> bool {
    // Names are in the local headers and in the central directory at the end of the file.
    let head = bytes.get(..bytes.len().min(1 << 16)).unwrap_or_default();
    let tail = bytes.get(bytes.len().saturating_sub(1 << 22)..).unwrap_or_default();
    [b"content/riffData.cdr".as_slice(), b"content/riffdata.cdr", b"content/root.dat"]
        .iter()
        .any(|needle| [head, tail].iter().any(|part| part.windows(needle.len()).any(|w| w == *needle)))
}

/// The version and container of a CDR file, with a descriptive error for what isn't CDR or can't be
/// read.
pub fn detect_version(bytes: &[u8]) -> Result<CdrInfo> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge(bytes.len()));
    }
    if !is_cdr(bytes) {
        return Err(Error::NotCdr);
    }
    let c = guarded("container detection", || tracedraw_cdr::container::detect(bytes))??;
    let container = match &c.kind {
        tracedraw_cdr::container::ContainerKind::Riff => Container::Riff,
        tracedraw_cdr::container::ContainerKind::Zip { riff_member, .. } if riff_member.ends_with("root.dat") => Container::ZipRootDat,
        tracedraw_cdr::container::ContainerKind::Zip { .. } => Container::ZipRiffData,
    };
    let version = c.version.0;
    let (supported, note) = match version {
        0 => (false, Some("the version couldn't be determined; reading is attempted".to_string())),
        1..=5 => (true, Some("a version before 6 (16-bit coordinates): partial support, little tested".to_string())),
        v if v > NEWEST_KNOWN_VERSION => {
            (true, Some(format!("version {v} is newer than the ones tested (up to {NEWEST_KNOWN_VERSION}); reading is attempted")))
        }
        _ => (true, None),
    };
    Ok(CdrInfo { version, container, product: product_name(version), supported, note })
}

/// What [`validate`] found.
#[derive(Clone, Debug, Default)]
pub struct Validation {
    pub info: Option<CdrInfo>,
    /// The file was read from start to end.
    pub readable: bool,
    pub pages: usize,
    pub layers: usize,
    /// Objects (shapes, type, images, groups) on every page and master layer.
    pub objects: usize,
    /// Object records the reader didn't understand and skipped.
    pub skipped: usize,
    pub warnings: Vec<String>,
    /// The size of the first page (mm).
    pub page_size: Option<(f64, f64)>,
    /// Why the file couldn't be read.
    pub error: Option<String>,
}

impl Validation {
    /// Whether the file was read through with no error.
    pub fn ok(&self) -> bool {
        self.readable && self.error.is_none()
    }
}

/// Reads the whole file (signature, version, chunk tree, objects) without making a VectorCraft
/// document: to check a file before opening it and after exporting it.
pub fn validate(bytes: &[u8]) -> Validation {
    let mut v = Validation::default();
    match detect_version(bytes) {
        Ok(info) => v.info = Some(info),
        Err(e) => {
            v.error = Some(e.to_string());
            return v;
        }
    }
    match read::parse(bytes) {
        Ok((doc, report)) => {
            v.readable = true;
            v.pages = doc.pages.len();
            v.layers = doc.all_layers().count();
            v.objects = doc.all_layers().map(|l| read::count_shapes(&l.shapes)).sum();
            v.skipped = report.skipped_objects;
            v.page_size = doc.pages.first().map(|p| (p.size.width, p.size.height));
            v.warnings = report.warnings;
        }
        Err(e) => v.error = Some(e.to_string()),
    }
    v
}

/// Reads a CorelDRAW document as a VectorCraft document (one artboard per page).
pub fn import(bytes: &[u8], title: &str, opts: &ImportOptions) -> Result<Imported> {
    let info = detect_version(bytes)?;
    read::import(bytes, title, info, opts)
}

/// Writes `doc` as CDR of version `opts.version` and, with `opts.validate`, reads the written file
/// back and compares it with what was written; a file that fails is refused ([`Error::Validation`]).
pub fn export(doc: &vectorcraft_doc::Document, opts: &ExportOptions) -> Result<Exported> {
    write::export(doc, opts)
}
