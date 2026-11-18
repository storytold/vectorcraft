//! Preserve Editing: the native document travels inside the PDF as an embedded file, with a hash
//! of the pages it was written with, so a PDF whose pages another app changed is noticed (and
//! opens as its artwork instead of the stale native document).

use std::collections::HashSet;
use std::sync::Arc;

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{Array, Dict, MaybeRef, Object, ObjectIdentifier, Stream, String as PdfString};
use craft_pdf::syntax::xref::XRef;
use craft_pdf::write::backend as krilla;
use krilla::embed::{AssociationKind, EmbeddedFile, MimeType};
use krilla::metadata::DateTime;

use crate::PdfError;
use crate::import_color::items;
use crate::import_scan::text_string;
use crate::lab_spot::find;

/// The embedded file holding the native document (its name among the PDF's attachments).
pub const EDITING_FILE: &str = "vectorcraft-editing.vectorcraft";
/// The editing file's name from before the project was renamed: still read.
pub const LEGACY_EDITING_FILE: &str = "drawcraft-editing.drawcraft";

/// The editing file's description: this text, then the hash of the pages.
const DESCRIPTION: &str = "VectorCraft editing data; page hash ";
/// Stands for the hash while the PDF is written: as long as a hash, so writing the hash over it
/// keeps every offset.
const PLACEHOLDER: &str = "0000000000000000";
/// The most name tree nodes searched for the editing file.
const MAX_NODES: usize = 1 << 16;

/// The native document a PDF carries ([`crate::PdfSettings::preserve_editing`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editing {
    /// The native document's bytes (empty when they can't be decoded).
    pub data: Vec<u8>,
    /// The pages are still the ones written with it: the hash of their size and content matches.
    /// False once another app changed them; true for editing data written without a hash.
    pub intact: bool,
}

/// The embedded file for `native`, its description holding the hash placeholder [`seal`] fills.
pub(crate) fn embedded_file(native: &[u8], compress: bool, date: Option<DateTime>) -> EmbeddedFile {
    EmbeddedFile {
        path: EDITING_FILE.into(),
        mime_type: MimeType::new("application/json"),
        description: Some(format!("{DESCRIPTION}{PLACEHOLDER}")),
        association_kind: AssociationKind::Source,
        data: native.to_vec().into(),
        modification_date: date,
        compress: Some(compress),
        location: None,
    }
}

/// Write the hash of the pages of `pdf` (written with an [`embedded_file`]) over the placeholder.
pub(crate) fn seal(pdf: Vec<u8>) -> Result<Vec<u8>, PdfError> {
    let data = Arc::new(pdf);
    let hash = page_hash(&Pdf::new(data.clone()).map_err(|e| PdfError::Write(format!("can't read back the written PDF: {e:?}")))?);
    let mut pdf = Arc::try_unwrap(data).unwrap_or_else(|d| (*d).clone());
    let at = find(&pdf, format!("({DESCRIPTION}{PLACEHOLDER})").as_bytes(), 0).map(|i| i + 1 + DESCRIPTION.len());
    let slot =
        at.and_then(|i| pdf.get_mut(i..i + PLACEHOLDER.len())).ok_or_else(|| PdfError::Write("the editing data's hash has no place".into()))?;
    slot.copy_from_slice(hash.as_bytes());
    Ok(pdf)
}

/// The native document `bytes` (a PDF) carries, if any, and whether its pages changed since.
pub fn editing(bytes: &[u8]) -> Option<Editing> {
    editing_with(bytes, None)
}

/// [`editing`] of a PDF that may be encrypted, opened with `password` (its open or permissions
/// password).
pub fn editing_with(bytes: &[u8], password: Option<&str>) -> Option<Editing> {
    editing_in(&crate::pages::open(bytes, password).ok()?)
}

/// What the search of a parsed PDF's attachments for the editing file found ([`editing_file_spec`]).
struct Search<'a> {
    /// The editing file's file specification, if one was found.
    spec: Option<Dict<'a>>,
    /// A name tree key searched names the editing file (set also when its file specification
    /// can't be read).
    keyed: bool,
    /// The name tree has more than [`MAX_NODES`] nodes, so nodes were left unsearched.
    cut: bool,
}

/// The search of a parsed PDF's attachments for the editing file.
fn editing_file_spec(pdf: &Pdf) -> Search<'_> {
    let xref = pdf.xref();
    let Some(tree) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|catalog| catalog.get::<Dict<'_>>(b"Names"))
        .and_then(|names| names.get::<Dict<'_>>(b"EmbeddedFiles"))
    else {
        return Search { spec: None, keyed: false, cut: false };
    };
    let (mut keyed, mut cut) = (false, false);
    let spec = editing_spec(tree, &mut keyed, &mut cut, xref);
    Search { spec, keyed, cut }
}

/// Does a parsed PDF list the editing file among its attachments, also when [`editing_in`] can't
/// read it (its file specification, its `EF` dictionary or its `F` or `UF` stream can't be
/// resolved)? Also true when the name tree is too large to search to its end.
pub(crate) fn has_editing_file_in(pdf: &Pdf) -> bool {
    let search = editing_file_spec(pdf);
    search.spec.is_some() || search.keyed || search.cut
}

/// Is a parsed PDF's attachment name tree too large to search to its end (more than [`MAX_NODES`]
/// nodes)? It may then list an editing file that [`editing_in`] doesn't find, besides the one it
/// reads.
pub(crate) fn editing_search_cut_in(pdf: &Pdf) -> bool {
    editing_file_spec(pdf).cut
}

/// [`editing`] of a parsed PDF.
pub(crate) fn editing_in(pdf: &Pdf) -> Option<Editing> {
    let spec = editing_file_spec(pdf).spec?;
    let files = spec.get::<Dict<'_>>(b"EF")?;
    let stream = files.get::<Stream<'_>>(b"F").or_else(|| files.get::<Stream<'_>>(b"UF"))?;
    let data = stream.decoded().map(|d| d.into_owned()).unwrap_or_default();
    // The hash ends the description (a text string, in any of its encodings); editing data
    // described otherwise is trusted.
    let desc = spec.get::<PdfString<'_>>(b"Desc").map(|d| text_string(d.as_bytes())).unwrap_or_default();
    let intact = match desc.rsplit_once("hash ") {
        Some((_, hash)) => hash.trim() == page_hash(pdf),
        None => true,
    };
    Some(Editing { data, intact })
}

/// The first file specification of the editing file in the name tree `tree`, searched in order at
/// any depth, [`MAX_NODES`] nodes at most. After the editing file is found, the walk goes on to the
/// end of the tree, or to the node limit, without reading the nodes' `Names`. An indirect node is
/// searched once, so a node shared by many `Kids` arrays is not searched once per path. `keyed` is
/// set when a key searched names the editing file, and `cut` when nodes are left unsearched.
fn editing_spec<'a>(tree: Dict<'a>, keyed: &mut bool, cut: &mut bool, xref: &'a XRef) -> Option<Dict<'a>> {
    let mut seen = HashSet::<ObjectIdentifier>::new();
    let mut nodes = vec![tree];
    let mut found = None;
    // Nodes still to be searched, counting the ones waiting in `nodes`.
    let mut left = MAX_NODES - 1;
    while let Some(node) = nodes.pop() {
        // Once the editing file is found, only the `Kids` are read, to learn whether the tree is cut.
        if found.is_none() {
            found = node_spec(&node, keyed, xref);
        }
        let Some(kids) = node.get::<Array<'a>>(b"Kids") else { continue };
        // Only a referenced kid can be shared (a direct kid's `obj_id` is the object holding it). The
        // reference is checked before the kid is read, so a shared kid is read once.
        let mut kids: Vec<Dict<'a>> = items(&kids)
            .filter_map(|raw| match raw {
                MaybeRef::Ref(r) if seen.insert(r.into()) => xref.get::<Dict<'a>>(r.into()),
                MaybeRef::NotRef(Object::Dict(kid)) => Some(kid),
                _ => None,
            })
            .take(left.saturating_add(1))
            .collect();
        if kids.len() > left {
            *cut = true;
            kids.truncate(left);
        }
        left -= kids.len();
        nodes.extend(kids.into_iter().rev());
    }
    found
}

/// The file specification of the editing file among the name tree node `node`'s own `Names`.
/// `keyed` is set when a key names the editing file.
fn node_spec<'a>(node: &Dict<'a>, keyed: &mut bool, xref: &'a XRef) -> Option<Dict<'a>> {
    if let Some(names) = node.get::<Array<'a>>(b"Names") {
        // The keys read as written, so a value that can't be resolved doesn't shift the pairs.
        let mut raw = items(&names);
        while let (Some(key), Some(_)) = (raw.next(), raw.next()) {
            let key = match key {
                MaybeRef::NotRef(Object::String(key)) => Some(key),
                MaybeRef::Ref(r) => xref.get::<PdfString<'a>>(r.into()),
                MaybeRef::NotRef(_) => None,
            };
            if key.is_some_and(|key| is_editing_name(&key)) {
                *keyed = true;
            }
        }
        let mut items = names.iter::<Object<'a>>();
        while let (Some(_), Some(spec)) = (items.next(), items.next()) {
            if let Object::Dict(spec) = spec
                && is_editing_file(&spec)
            {
                return Some(spec);
            }
        }
    }
    None
}

/// A file specification naming the editing file (current or legacy name), in any text string
/// encoding.
fn is_editing_file(spec: &Dict<'_>) -> bool {
    [&b"UF"[..], b"F"].into_iter().filter_map(|k| spec.get::<PdfString<'_>>(k)).any(|name| is_editing_name(&name))
}

/// A text string that is the editing file's name (current or legacy), in any encoding.
fn is_editing_name(name: &PdfString<'_>) -> bool {
    let name = text_string(name.as_bytes());
    name == EDITING_FILE || name == LEGACY_EDITING_FILE
}

/// The most decoded page content [`page_hash`] reads in all (bytes).
const MAX_HASHED: usize = 256 << 20;

/// FNV-1a of the pages' media boxes and decoded content, as 16 hex digits. Past [`MAX_HASHED`]
/// bytes of content in all (pages sharing a stream count it each time) a marker stands for the
/// rest, so writing and reading a file hash it the same way within bounded work.
fn page_hash(pdf: &Pdf) -> String {
    let fnv = |h: u64, bytes: &[u8]| bytes.iter().fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
    let mut left = MAX_HASHED;
    let mut content = |h: u64, stream: &Stream<'_>| {
        if left > 0
            && let Ok(data) = stream.decoded_within(left)
            && data.len() < left
        {
            left -= data.len();
            return fnv(h, &data);
        }
        left = 0;
        fnv(h, b"\0past the hashed content")
    };
    let mut h = 0xcbf29ce484222325;
    for page in pdf.pages().iter() {
        let b = page.media_box();
        h = fnv(h, format!("[{} {} {} {}]", b.x0, b.y0, b.x1, b.y1).as_bytes());
        // Decode into a temporary rather than through `Page::page_stream`, which keeps every
        // page's copy for as long as the PDF lives, even when pages share one stream.
        let contents = page.raw();
        if let Some(stream) = contents.get::<Stream<'_>>(b"Contents") {
            h = content(h, &stream);
        } else if let Some(array) = contents.get::<Array<'_>>(b"Contents") {
            for stream in array.iter::<Stream<'_>>() {
                h = fnv(content(h, &stream), b" ");
            }
        }
    }
    format!("{h:016x}")
}
