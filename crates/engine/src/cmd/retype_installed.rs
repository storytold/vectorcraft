//! Retype's installed fonts: the faces of installed fonts that nothing has loaded yet are
//! candidates too, as the fonts on the device are among the sources Retype suggests fonts from
//! (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/identify-fonts-using-retype.html>).
//! Each font file is read for the search and let go; a face whose glyphs match a shape is loaded
//! as it was read, without the family's other files. Files are read smallest first, within a budget
//! of bytes read and glyphs drawn, and the search stops between files when it is canceled. The app
//! runs it off the UI thread ([`RetypeRun`]), with a Cancel. A face read is matched as a loaded one
//! is: with Retype's tolerance, near matches count, and a compound path of several glyphs counts
//! when every glyph it splits into matches, in one face or in several.

use std::collections::BTreeSet;
use std::sync::Arc;

use kurbo::BezPath;
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_text::retype::{MAX_FACES, NearKey, RetypeIndex, encoded_glyphs, is_plain, shape_key};
use vectorcraft_text::{FontDb, FontFace};

use super::super::{EngineError, Result, Session, bad};

/// How much one search reads.
#[derive(Clone, Copy)]
struct Limits {
    /// Font files (a suitcase font's resource fork) longer than this are not read.
    file: u64,
    /// The bytes of font files read, in all.
    read: u64,
    /// The glyphs of installed faces drawn, in all, to tell which can match a shape.
    glyphs: usize,
    /// The faces found: the faces after the last one found are not compared.
    faces: usize,
}

const LIMITS: Limits = Limits { file: 256 << 20, read: 2 << 30, glyphs: 500_000, faces: MAX_FACES };

/// The `installed` param: whether to look in the installed fonts (default true).
pub(super) fn wanted(cmd: &str, p: &Value) -> Result<bool> {
    match p.get("installed") {
        None | Some(Value::Null) => Ok(true),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(bad(cmd, "`installed` must be true, false or null")),
    }
}

/// What a search of the installed fonts found and read.
#[derive(Default)]
pub(super) struct Installed {
    /// The faces whose glyphs match a shape, loaded now, in the order they were read.
    pub(super) faces: Vec<Arc<FontFace>>,
    /// Faces read and compared.
    pub(super) read: usize,
    /// Faces left out by the limits ([`LIMITS`]).
    pub(super) skipped: usize,
    /// For a search made away from its command, the ids of the loaded faces it compared: the
    /// command does not compare them again.
    pub(super) scanned: Vec<u32>,
}

impl Installed {
    /// As a command's result reports it.
    pub(super) fn json(&self) -> Value {
        json!({"read": self.read, "skipped": self.skipped})
    }
}

/// A shape to look for: its path, the contour groups it splits into when it may hold several
/// glyphs (none for a shape matched whole only), its fill rule when it has a visible fill
/// (`Some(nonzero)`) and whether it paints its overlaps once ([`super::paints_once`]), as
/// `type.retype` reads them.
#[derive(Clone, Copy)]
pub(super) struct Sought<'a> {
    pub(super) path: &'a BezPath,
    pub(super) groups: &'a [BezPath],
    pub(super) fill: Option<bool>,
    pub(super) once: bool,
}

impl<'a> Sought<'a> {
    /// `path` split into `groups`, with no paint check.
    pub(super) fn unpainted(path: &'a BezPath, groups: &'a [BezPath]) -> Self {
        Sought { path, groups, fill: None, once: true }
    }
}

/// `shapes`, each matched whole only.
fn whole<'a>(shapes: &[&'a BezPath]) -> Vec<Sought<'a>> {
    shapes.iter().map(|path| Sought::unpainted(path, &[])).collect()
}

/// The keys to look glyphs up by for `shapes`, at `tolerance` ([`super::lookup_keys`]).
fn keys_of(shapes: &[Sought], tolerance: f64) -> (BTreeSet<u64>, Vec<NearKey>) {
    super::lookup_keys(shapes.iter().map(|s| (s.path, s.groups)), tolerance)
}

/// Does a search look for `s`: is it not plain ([`is_plain`]: a rectangle, a triangle or a bar),
/// with a key to look glyphs up by? A shape without one matches no face.
fn starts(s: &Sought, tolerance: f64) -> bool {
    if is_plain(s.path) {
        return false;
    }
    let (keys, near) = keys_of(std::slice::from_ref(s), tolerance);
    !keys.is_empty() || !near.is_empty()
}

/// Is `s` found in `indexes` at `tolerance`: does its whole shape match a glyph there, or does
/// every glyph it splits into, painting the area `s` paints ([`super::shape_parts`])?
fn found_in(indexes: &[&RetypeIndex], s: &Sought, tolerance: f64) -> bool {
    !super::shape_parts(s.path, s.groups, s.fill, s.once, indexes, tolerance).is_empty()
}

/// [`search_for`] of `shapes` matched whole, exactly.
#[cfg(test)]
pub(super) fn search(db: &FontDb, shapes: &[&BezPath], every: bool) -> Result<Installed> {
    search_within(db, shapes, every, LIMITS, &|_| true)
}

/// [`search_beside`] with no loaded faces.
#[cfg(test)]
pub(super) fn search_for(db: &FontDb, shapes: &[Sought], tolerance: f64, every: bool) -> Result<Installed> {
    search_beside(db, shapes, &[], tolerance, every)
}

/// The installed faces nothing has loaded that `shapes` that are not plain ([`is_plain`]) are found
/// in at `tolerance` ([`found_in`]), each loaded into `db` as it was read. With `every` false, a
/// face is found when it matches such a shape, or completes the glyphs of a split one with the
/// loaded faces of `loaded` (the indexes of faces the command compares too) and the faces found or
/// read before it (a face read before whose glyphs it completes is found with it), and the search
/// stops once each such shape is found; with it, every face within the limits ([`LIMITS`]) is
/// compared, and found when it matches a shape on its own. No file is read when every shape is
/// plain. At most [`MAX_FACES`] faces are found; the faces not compared past that count as skipped.
/// The faces of `loaded` whose glyphs complete a split shape with a face found are among the faces
/// found too, so the command compares them with it.
pub(super) fn search_beside(db: &FontDb, shapes: &[Sought], loaded: &[&RetypeIndex], tolerance: f64, every: bool) -> Result<Installed> {
    search_sought(db, shapes, loaded, tolerance, every, LIMITS, &|_| true)
}

/// [`search`] comparing every face within the limits, for a list of at most `faces` faces: a face
/// found that `listed` turns down is loaded and not listed, and takes no place of the `faces`.
pub(super) fn search_listed(db: &FontDb, shapes: &[&BezPath], faces: usize, listed: &dyn Fn(&FontFace) -> bool) -> Result<Installed> {
    search_sought(db, &whole(shapes), &[], 0.0, true, Limits { faces: faces.min(MAX_FACES), ..LIMITS }, listed)
}

/// Would a search for `shapes`, matched whole and exactly, read any installed file: is one of them
/// not plain, with a glyph's key ([`shape_key`])?
pub(super) fn starts_search(shapes: &[&BezPath]) -> bool {
    shapes.iter().any(|s| !is_plain(s) && shape_key(s).is_some())
}

/// [`loaded_rest_for`] of `shapes` matched whole, exactly: the faces only.
#[cfg(test)]
pub(super) fn loaded_rest(db: &FontDb, shapes: &[&BezPath], compared: &[u32]) -> Result<Vec<Arc<FontFace>>> {
    Ok(loaded_rest_for(db, &whole(shapes), &[], 0.0, compared)?.0)
}

/// The loaded faces that are not among `compared` (those past the [`MAX_FACES`] a Retype compares
/// first) that one of `shapes` that is not plain ([`is_plain`]) is found in at `tolerance`
/// ([`found_in`]), indexed [`MAX_FACES`] at a time: the faces of the glyphs it matches. The glyphs of
/// a split shape may be spread over those groups and the faces of `beside` (the indexes of faces
/// among `compared`). With them, whether each shape was found, and the indexes of the latest
/// groups that hold glyphs of the shapes while a split shape is not found (at most
/// [`MAX_KEPT_GROUPS`]), for the search of the installed fonts. A cancel stops it between those
/// groups.
pub(super) fn loaded_rest_for(db: &FontDb, shapes: &[Sought], beside: &[&RetypeIndex], tolerance: f64, compared: &[u32]) -> Result<LoadedRest> {
    let mut done = vec![false; shapes.len()];
    let looked: Vec<(usize, &Sought)> = shapes.iter().enumerate().filter(|(_, s)| !is_plain(s.path)).collect();
    let (keys, near) = super::lookup_keys(looked.iter().map(|(_, s)| (s.path, s.groups)), tolerance);
    let mut out: Vec<Arc<FontFace>> = vec![];
    if keys.is_empty() && near.is_empty() {
        return Ok((out, done, vec![]));
    }
    let rest: Vec<Arc<FontFace>> = db.loaded_faces().into_iter().filter(|f| !compared.contains(&f.id())).collect();
    // The earlier groups' indexes that hold glyphs of the shapes, while a split shape is not found
    // yet: the latest [`MAX_KEPT_GROUPS`].
    let mut kept: Vec<RetypeIndex> = vec![];
    for group in rest.chunks(MAX_FACES) {
        vectorcraft_doc::cancel::check()?;
        let index = RetypeIndex::build_near(db, group.iter().cloned(), &keys, &near);
        let with: Vec<&RetypeIndex> = beside.iter().copied().chain(&kept).chain(std::iter::once(&index)).collect();
        for (k, s) in &looked {
            // A split shape not found yet is matched with the faces before the group too (one pass,
            // as near matching spends an index's budget); any other shape with the group alone.
            let combined = !s.groups.is_empty() && !done.get(*k).copied().unwrap_or(true);
            let indexes: &[&RetypeIndex] = if combined { &with } else { &[&index] };
            let parts = super::shape_parts(s.path, s.groups, s.fill, s.once, indexes, tolerance);
            if parts.is_empty() {
                continue;
            }
            if let Some(d) = done.get_mut(*k) {
                *d = true;
            }
            for m in parts.into_iter().flatten() {
                if !compared.contains(&m.face.id()) && !out.iter().any(|f| f.id() == m.face.id()) {
                    out.push(m.face);
                }
            }
        }
        let open = looked.iter().any(|(k, s)| !s.groups.is_empty() && !done.get(*k).copied().unwrap_or(true));
        if open && !index.is_empty() {
            // The latest groups are kept: the oldest goes when the cap is reached.
            if kept.len() >= MAX_KEPT_GROUPS {
                kept.remove(0);
            }
            kept.push(index);
        }
    }
    if !looked.iter().any(|(k, s)| !s.groups.is_empty() && !done.get(*k).copied().unwrap_or(true)) {
        kept.clear();
    }
    Ok((out, done, kept))
}

/// What [`loaded_rest_for`] finds: the faces, whether each shape was found, and the indexes kept
/// for the search of the installed fonts.
type LoadedRest = (Vec<Arc<FontFace>>, Vec<bool>, Vec<RetypeIndex>);

/// Earlier groups of [`MAX_FACES`] loaded faces past the ones compared first that a group's faces
/// complete a split shape's glyphs with, at most: the latest ones.
const MAX_KEPT_GROUPS: usize = 4;

/// An installed font file: its path, its length and the (family, style) of the faces wanted from it.
type File = (std::path::PathBuf, u64, Vec<(String, String)>);

/// [`search`] within `limits`, keeping in its faces only those `listed` accepts.
#[cfg(test)]
fn search_within(db: &FontDb, shapes: &[&BezPath], every: bool, limits: Limits, listed: &dyn Fn(&FontFace) -> bool) -> Result<Installed> {
    search_sought(db, &whole(shapes), &[], 0.0, every, limits, listed)
}

/// [`search_beside`] within `limits`, keeping in its faces only those `listed` accepts.
fn search_sought(
    db: &FontDb,
    shapes: &[Sought],
    beside: &[&RetypeIndex],
    tolerance: f64,
    every: bool,
    limits: Limits,
    listed: &dyn Fn(&FontFace) -> bool,
) -> Result<Installed> {
    let mut out = Installed::default();
    // A shape without a glyph's key matches no face: the search does not wait for it.
    let shapes: Vec<Sought> = shapes.iter().copied().filter(|s| starts(s, tolerance)).collect();
    let (keys, near) = keys_of(&shapes, tolerance);
    if keys.is_empty() && near.is_empty() {
        return Ok(out);
    }
    let (keys, near) = (&keys, near.as_slice());
    // The indexes of the faces found, which the glyphs of a split shape may be spread over.
    let mut found: Vec<RetypeIndex> = vec![];
    // The faces read, not loaded, that hold glyphs of the shapes without completing one, with their
    // indexes: a later face may complete a split shape with them. The latest [`MAX_FACES`].
    let mut partial: Vec<(Arc<FontFace>, RetypeIndex)> = vec![];
    // The faces wanted from each file, smallest file first.
    let mut files: Vec<File> = vec![];
    let mut at: std::collections::HashMap<std::path::PathBuf, usize> = std::collections::HashMap::new();
    for f in db.unloaded_installed_faces() {
        match at.get(&f.path).and_then(|i| files.get_mut(*i)) {
            Some(file) => file.2.push((f.family, f.style)),
            None => {
                at.insert(f.path.clone(), files.len());
                files.push((f.path, f.len, vec![(f.family, f.style)]));
            }
        }
    }
    files.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut matched = vec![false; shapes.len()];
    // Is a split shape not found yet: only then are faces kept in `partial`.
    let open = |matched: &[bool]| shapes.iter().zip(matched).any(|(s, m)| !*m && !s.groups.is_empty());
    let (mut bytes_left, mut glyphs_left) = (limits.read, limits.glyphs);
    let total = files.len() as u64;
    for (i, (path, len, styles)) in files.iter().enumerate() {
        vectorcraft_doc::cancel::check()?;
        vectorcraft_doc::cancel::progress(i as u64, total, "Retype: reading installed fonts");
        // A file too long for what is left, or read when no glyph is left to draw, is not read.
        if *len > limits.file || *len > bytes_left || glyphs_left == 0 {
            out.skipped += styles.len();
            continue;
        }
        // A file that grew since the scan is read only within what is left.
        let file = db.read_unloaded_file(path, limits.file.min(bytes_left));
        // The bytes read, those of a file that grew past the limits too.
        bytes_left = bytes_left.saturating_sub(file.bytes);
        if file.too_long {
            out.skipped += styles.len();
            continue;
        }
        let wanted = |f: &FontFace| styles.iter().any(|(family, style)| f.family.eq_ignore_ascii_case(family) && f.style.eq_ignore_ascii_case(style));
        let faces: Vec<Arc<FontFace>> = file.faces.into_iter().filter(|f| wanted(f)).collect();
        for (j, face) in faces.iter().enumerate() {
            if out.faces.len() >= limits.faces {
                // The faces not compared past the cap: the rest of this file's and the later files'.
                out.skipped += styles.len().saturating_sub(j) + files.iter().skip(i + 1).map(|f| f.2.len()).sum::<usize>();
                return Ok(out);
            }
            let n = encoded_glyphs(face);
            if n > glyphs_left {
                out.skipped += 1;
                continue;
            }
            glyphs_left -= n;
            out.read += 1;
            let index = RetypeIndex::build_unloaded_near(db, face.clone(), keys, near);
            if index.is_empty() {
                continue;
            }
            // With `every`, a face fits a shape it matches on its own; otherwise a shape not found
            // yet that it completes with the loaded faces of `beside`, the faces found before it and
            // the faces read before it that hold some of its glyphs (`partial`).
            let parts: Vec<&RetypeIndex> = partial.iter().map(|(_, ix)| ix).collect();
            let parts_with = |extra: &[&RetypeIndex], ix: &RetypeIndex, k: usize| -> Vec<Vec<super::GlyphMatch>> {
                let Some(s) = shapes.get(k) else { return vec![] };
                if every {
                    return super::shape_parts(s.path, s.groups, s.fill, s.once, &[ix], tolerance);
                }
                if matched.get(k).copied().unwrap_or(true) {
                    return vec![];
                }
                // A shape matched whole only takes one glyph: the faces read before it do not hold it.
                let extra = if s.groups.is_empty() { &[] } else { extra };
                let with: Vec<&RetypeIndex> = beside.iter().copied().chain(&found).chain(extra.iter().copied()).chain(std::iter::once(ix)).collect();
                super::shape_parts(s.path, s.groups, s.fill, s.once, &with, tolerance)
            };
            let fit_with = |extra: &[&RetypeIndex], ix: &RetypeIndex, k: usize| !parts_with(extra, ix, k).is_empty();
            let fit = |ix: &RetypeIndex, k: usize| fit_with(&parts, ix, k);
            // Each shape it fits, with the glyphs it matches: one pass, as near matching spends an
            // index's budget.
            let first: Vec<(usize, Vec<Vec<super::GlyphMatch>>)> =
                (0..shapes.len()).map(|k| (k, parts_with(&parts, &index, k))).filter(|(_, p)| !p.is_empty()).collect();
            let fits: Vec<usize> = first.iter().map(|(k, _)| *k).collect();
            if fits.is_empty() {
                // The face holds glyphs of a shape without completing one: a later face may, while
                // a split shape is not found. The latest are kept: the oldest goes when the cap is
                // reached.
                if !every && open(&matched) {
                    #[cfg(test)]
                    KEPT_PARTIAL.with(|n| n.set(n.get() + 1));
                    if partial.len() >= MAX_FACES {
                        partial.remove(0);
                    }
                    partial.push((face.clone(), index));
                }
                continue;
            }
            let loaded = load(db, face);
            let same = loaded.id() == face.id();
            let (mut fits, index) = still_fit(db, face, &loaded, index, keys, near, fits, &fit);
            if fits.is_empty() {
                continue;
            }
            // The faces read before whose glyphs the shapes it fits use, loaded with it: those
            // that leave room under the cap, or else only the shapes it fits without them. A face
            // loaded in place of the one read is matched again for them.
            let matches: Vec<Vec<super::GlyphMatch>> = if same {
                first.into_iter().flat_map(|(_, p)| p).collect()
            } else {
                fits.iter().flat_map(|k| parts_with(&parts, &index, *k)).collect()
            };
            let held: Vec<&Arc<FontFace>> = index.faces().iter().chain(found.iter().flat_map(|ix| ix.faces())).chain(&out.faces).collect();
            let (mut used, mut with_loaded) = companions(&matches, &held, beside, &partial);
            if out.faces.len() + used.len() + with_loaded.len() >= limits.faces {
                let alone: Vec<&RetypeIndex> = found.iter().chain(std::iter::once(&index)).collect();
                fits.retain(|k| shapes.get(*k).is_some_and(|s| found_in(&alone, s, tolerance)));
                used.clear();
                with_loaded.clear();
                if fits.is_empty() {
                    continue;
                }
            }
            let mut shadowed = false;
            let mut promoted: Vec<Arc<FontFace>> = vec![];
            for at in used.into_iter().rev() {
                let (read, ix) = partial.remove(at);
                let loaded = load(db, &read);
                let ix = if loaded.id() == read.id() {
                    ix
                } else {
                    shadowed = true;
                    RetypeIndex::build_near(db, [loaded.clone()], keys, near)
                };
                promoted.insert(0, loaded);
                found.push(ix);
            }
            // A face loaded in place of one read may have other glyphs: the shapes are checked again.
            if shadowed {
                let with: Vec<&RetypeIndex> = beside.iter().copied().chain(&found).chain(std::iter::once(&index)).collect();
                fits.retain(|k| shapes.get(*k).is_some_and(|s| found_in(&with, s, tolerance)));
                if fits.is_empty() {
                    continue;
                }
            }
            // The faces it completes shapes with, once the match holds.
            out.faces.extend(with_loaded);
            for loaded in promoted {
                if listed(&loaded) {
                    remember(db, &loaded);
                    if !out.faces.iter().any(|f| f.id() == loaded.id()) {
                        out.faces.push(loaded);
                    }
                }
            }
            // A face `listed` turns down (another version shadows it) is not compared ahead of others.
            if listed(&loaded) {
                remember(db, &loaded);
                if !out.faces.iter().any(|f| f.id() == loaded.id()) {
                    out.faces.push(loaded);
                }
            }
            found.push(index);
            for k in fits {
                if let Some(m) = matched.get_mut(k) {
                    *m = true;
                }
            }
            if !every && matched.iter().all(|m| *m) {
                return Ok(out);
            }
            // With every split shape found, the faces kept for them are dropped.
            if !open(&matched) {
                partial.clear();
            }
        }
        // The cap reached with the file's last face: the later files are not compared.
        if out.faces.len() >= limits.faces {
            out.skipped += files.iter().skip(i + 1).map(|f| f.2.len()).sum::<usize>();
            return Ok(out);
        }
    }
    Ok(out)
}

#[cfg(test)]
thread_local! {
    /// How many times a search on this thread kept a face it read in `partial`.
    static KEPT_PARTIAL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// For each of `parts` (each part's candidates) that none of the faces of `held` has a glyph of,
/// one face that does: a loaded face of `beside` (the command compares it with the face found),
/// or else a face of `partial`, read before. The places in `partial` of those, in order, and the
/// loaded faces.
fn companions(
    parts: &[Vec<super::GlyphMatch>],
    held: &[&Arc<FontFace>],
    beside: &[&RetypeIndex],
    partial: &[(Arc<FontFace>, RetypeIndex)],
) -> (Vec<usize>, Vec<Arc<FontFace>>) {
    let (mut used, mut loaded) = (BTreeSet::new(), Vec::<Arc<FontFace>>::new());
    for part in parts {
        let has = |f: &Arc<FontFace>| part.iter().any(|m| m.face.id() == f.id());
        let chosen = used.iter().filter_map(|at: &usize| partial.get(*at).map(|(f, _)| f));
        if held.iter().copied().chain(&loaded).chain(chosen).any(has) {
            continue;
        }
        let of_beside = part.iter().find(|m| beside.iter().any(|ix| ix.faces().iter().any(|f| f.id() == m.face.id())));
        if let Some(m) = of_beside {
            loaded.push(m.face.clone());
        } else if let Some(at) = part.iter().find_map(|m| partial.iter().position(|(f, _)| Arc::ptr_eq(f, &m.face))) {
            used.insert(at);
        }
    }
    (used.into_iter().collect(), loaded)
}

/// The faces searches loaded from installed files, by database ([`FontDb::id`]) and face id, the
/// latest first: at most [`MAX_FACES`].
static FOUND: std::sync::Mutex<Vec<(u64, u32)>> = std::sync::Mutex::new(Vec::new());

/// Keep `face`, which a search loaded into `db`, among [`found`]'s faces.
fn remember(db: &FontDb, face: &FontFace) {
    let mut kept = FOUND.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let at = (db.id(), face.id());
    kept.retain(|k| *k != at);
    kept.insert(0, at);
    kept.truncate(MAX_FACES);
}

/// The faces of `db` that searches of the installed fonts found and loaded, the latest first. A
/// later Retype compares them ahead of the other loaded faces.
pub(super) fn found(db: &FontDb) -> Vec<Arc<FontFace>> {
    let kept = FOUND.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    kept.into_iter().filter(|(d, _)| *d == db.id()).filter_map(|(_, id)| db.face_by_id(id)).collect()
}

/// Of `fits` (the shapes `index`, the index of `read`, fits as `fit` tells), those that `loaded`,
/// the face loading `read` gave, fits too, with the index they fit in: all of them and `index`
/// when it is `read` itself. Another face of the same family, style and version, loaded from
/// another file, may have other glyphs; it is indexed by `keys` and `near`.
#[allow(clippy::too_many_arguments)]
fn still_fit(
    db: &FontDb,
    read: &FontFace,
    loaded: &Arc<FontFace>,
    index: RetypeIndex,
    keys: &BTreeSet<u64>,
    near: &[NearKey],
    fits: Vec<usize>,
    fit: &dyn Fn(&RetypeIndex, usize) -> bool,
) -> (Vec<usize>, RetypeIndex) {
    if loaded.id() == read.id() {
        return (fits, index);
    }
    let index = RetypeIndex::build_near(db, [loaded.clone()], keys, near);
    let fits = fits.into_iter().filter(|k| fit(&index, *k)).collect();
    (fits, index)
}

/// `face` (read from an installed file and not loaded) loaded into `db` as it was read
/// ([`FontDb::load_read_face`]): neither its file nor the family's other files are read again.
fn load(db: &FontDb, face: &Arc<FontFace>) -> Arc<FontFace> {
    db.load_read_face(face.clone())
}

/// A search of the installed fonts made away from its command, so the app can run it off the UI
/// thread with a Cancel. [`RetypeRun::prepare`] takes the selected shapes and the document from the
/// session, checking the params as the command checks them; [`RetypeSearch::run`] picks the loaded
/// faces to compare, compares them and searches the installed fonts on any thread, loading the
/// faces that match; and
/// [`RetypeFound::hand_in`] gives what it found to the command, which then runs with
/// [`RetypeRun::params`] and compares those faces as its own search would, without reading the
/// installed fonts again.
pub struct RetypeRun {
    params: Value,
    /// The document ([`crate::DocState::uid`]), its content and the selection the search is for.
    doc: u64,
    content: Arc<Document>,
    selection: Vec<NodeId>,
    search: Arc<RetypeSearch>,
}

/// What a [`RetypeRun`] compares and searches, without the session: it can be sent to another
/// thread ([`RetypeRun::search`]).
pub struct RetypeSearch {
    command: &'static str,
    /// The document it is for ([`crate::DocState::uid`]).
    doc: u64,
    /// The shapes to match, and the loaded faces they are compared with first.
    shapes: Vec<Shape>,
    faces: Faces,
    /// `type.retype`'s tolerance (0 for `type.retypeFonts`).
    tolerance: f64,
}

/// The loaded faces a [`RetypeSearch`] compares first.
enum Faces {
    /// These faces.
    #[cfg(test)]
    Given(Vec<Arc<FontFace>>),
    /// Those the command picks in `doc`: with `fonts` (`type.retype`'s `fonts` param), the faces of
    /// those families and of the document's live type ([`super::candidate_faces`]), and without, the
    /// faces `type.retypeFonts` lists ([`super::fonts::listed_faces`]). They are picked on the
    /// search's thread, since a family not loaded yet is read from its files.
    Of { doc: Arc<Document>, fonts: Option<Vec<String>> },
}

impl Faces {
    fn get(&self, db: &FontDb) -> Vec<Arc<FontFace>> {
        match self {
            #[cfg(test)]
            Faces::Given(faces) => faces.clone(),
            Faces::Of { doc, fonts: Some(fonts) } => super::candidate_faces(db, doc, fonts),
            Faces::Of { doc, fonts: None } => super::fonts::listed_faces(db, doc),
        }
    }
}

/// A shape a [`RetypeSearch`] compares, as `type.retype` matches it ([`super::shape_parts`]): its
/// path, its contour groups when it may hold several glyphs, its fill rule when it has a visible
/// fill (`Some(nonzero)`), and whether it paints its overlaps once ([`super::paints_once`]).
struct Shape {
    path: BezPath,
    groups: Vec<BezPath>,
    fill: Option<bool>,
    once: bool,
}

impl Shape {
    /// `path`, matched whole, with no paint check.
    fn whole(path: BezPath) -> Self {
        Shape { path, groups: vec![], fill: None, once: true }
    }

    fn sought(&self) -> Sought<'_> {
        Sought { path: &self.path, groups: &self.groups, fill: self.fill, once: self.once }
    }
}

/// What a [`RetypeSearch`] found: for `type.retype`, the faces it compared first that match, the
/// loaded faces past those that match ([`loaded_rest`]), the installed faces whose glyphs match,
/// loaded, and the loaded faces it compared; and what it read of the installed fonts.
pub struct RetypeFound {
    doc: u64,
    installed: Installed,
}

impl RetypeFound {
    /// Hand it to the next `type.retype` or `type.retypeFonts`, which compares the faces found as
    /// its own search of the installed fonts would, in the document the search was made for. Any
    /// such command takes it; one in another document compares only what is loaded.
    pub fn hand_in(self, s: &mut Session) {
        s.retype_hand_off = Some(self);
    }

    /// Drop what was handed in and no command took, as when the command failed before it took
    /// it: a later command searches for itself.
    pub fn drop_handed(s: &mut Session) {
        s.retype_hand_off = None;
    }

    /// What it read, as the commands report it.
    pub fn json(&self) -> Value {
        self.installed.json()
    }
}

/// What a search found for the active document of `s`, handed in for this command ([`RetypeFound`]).
/// It is taken whatever the document, so it is used once.
pub(super) fn handed(s: &mut Session) -> Option<Installed> {
    let found = s.retype_hand_off.take()?;
    let doc = s.doc().ok()?.uid;
    (found.doc == doc).then_some(found.installed)
}

impl RetypeRun {
    /// The search `command` (`type.retype` or `type.retypeFonts`) with params `p` makes in the
    /// active document. None when it reads no installed font: `installed` is false, or no shape
    /// starts a search ([`starts`]: each is plain or has no glyph's key). An error where the command fails on its params or on the selection.
    /// The faces to compare are picked when the search runs.
    pub fn prepare(s: &Session, command: &str, p: &Value) -> Result<Option<RetypeRun>> {
        let (command, fonts, tolerance) = match command {
            "type.retype" => ("type.retype", Some(super::fonts_param(p)?), super::tolerance_param(p)?),
            "type.retypeFonts" => {
                super::fonts::count_param(p)?;
                ("type.retypeFonts", None, 0.0)
            }
            other => return Err(EngineError::Other(format!("`{other}` doesn't search the installed fonts"))),
        };
        // The keys the command refuses (an unknown one) fail here too, before any search.
        s.check_params(command, p)?;
        if !wanted(command, p)? {
            return Ok(None);
        }
        let (pieces, _) = super::selected_pieces(s)?;
        let st = s.doc()?;
        // Retype compares the shapes that may become type, split as it splits them; the list
        // compares every shape whole.
        let shapes: Vec<Shape> = if fonts.is_some() {
            let mut budget = super::MAX_SPLIT_TOTAL_CONTOURS;
            pieces
                .into_iter()
                .filter(|pc| !pc.keeps_outline)
                .map(|pc| Shape {
                    groups: super::split_groups(&pc.path, &mut budget),
                    fill: pc.filled.then_some(pc.nonzero),
                    once: super::paints_once(pc.opacity, pc.blend, &pc.appearance),
                    path: pc.path,
                })
                .collect()
        } else {
            pieces.into_iter().map(|pc| Shape::whole(pc.path)).collect()
        };
        if !shapes.iter().any(|s| starts(&s.sought(), tolerance)) {
            return Ok(None);
        }
        let faces = Faces::Of { doc: st.doc.clone(), fonts };
        Ok(Some(RetypeRun {
            params: p.clone(),
            doc: st.uid,
            content: st.doc.clone(),
            selection: st.selection.objects.clone(),
            search: Arc::new(RetypeSearch { command, doc: st.uid, shapes, faces, tolerance }),
        }))
    }

    /// `type.retype` or `type.retypeFonts`.
    pub fn command(&self) -> &'static str {
        self.search.command
    }

    /// What it compares and searches, for another thread.
    pub fn search(&self) -> Arc<RetypeSearch> {
        self.search.clone()
    }

    /// The command's params: those it was prepared with.
    pub fn params(&self) -> &Value {
        &self.params
    }

    /// Does it still apply: is the document it was prepared in active, with the same content and
    /// the same objects selected?
    pub fn current(&self, s: &Session) -> bool {
        s.doc().is_ok_and(|st| st.uid == self.doc && Arc::ptr_eq(&st.doc, &self.content) && st.selection.objects == self.selection)
    }

    /// [`RetypeSearch::run`] with the fonts of `db`.
    #[cfg(test)]
    fn run_in(&self, db: &FontDb) -> Result<RetypeFound> {
        self.search.run_in(db)
    }
}

impl RetypeSearch {
    /// Compare the shapes with the loaded faces, then search the installed fonts as the command
    /// would, loading the faces that match → what it found ([`EngineError::Cancelled`] when a
    /// cancel stopped it). It touches no document, so it can run on any thread.
    pub fn run(&self) -> Result<RetypeFound> {
        self.run_in(FontDb::global())
    }

    /// [`RetypeSearch::run`] with the fonts of `db`.
    fn run_in(&self, db: &FontDb) -> Result<RetypeFound> {
        let picked = self.faces.get(db);
        let installed = if self.command == "type.retypeFonts" {
            let shapes: Vec<&BezPath> = self.shapes.iter().map(|s| &s.path).collect();
            let (ranked, mut found) = super::fonts::rank_fonts(db, picked, &shapes, true, None)?;
            // The loaded faces that match are handed in too, ahead of the installed ones found:
            // loading those can move a loaded face past the 96 the command lists.
            let mut faces: Vec<Arc<FontFace>> = ranked.iter().map(|r| r.face().clone()).collect();
            for f in std::mem::take(&mut found.faces) {
                if !faces.iter().any(|k| k.id() == f.id()) {
                    faces.push(f);
                }
            }
            found.faces = faces;
            found
        } else {
            // The shapes are matched as the command matches them first.
            let tolerance = self.tolerance;
            let sought: Vec<Sought> = self.shapes.iter().map(Shape::sought).collect();
            let (keys, near) = keys_of(&sought, tolerance);
            let index = RetypeIndex::build_near(db, picked.iter().cloned(), &keys, &near);
            let parts: Vec<Vec<Vec<super::GlyphMatch>>> =
                self.shapes.iter().map(|s| super::shape_parts(&s.path, &s.groups, s.fill, s.once, &[&index], tolerance)).collect();
            let left: Vec<Sought> = sought.iter().zip(&parts).filter(|(_, p)| p.is_empty()).map(|(s, _)| *s).collect();
            // The first faces that match are handed in too: loading the faces found can move one
            // past the first 96 the command compares.
            let mut first: Vec<Arc<FontFace>> = vec![];
            for m in parts.into_iter().flatten().flatten() {
                if !first.iter().any(|f| f.id() == m.face.id()) {
                    first.push(m.face);
                }
            }
            // The loaded faces past the ones compared first, then the installed fonts for what
            // none of them matches; the command takes both from what is handed in.
            let compared: Vec<u32> = picked.iter().map(|f| f.id()).collect();
            // Every face loaded now is compared, among the first ones or with the rest.
            let scanned: Vec<u32> = compared.iter().copied().chain(db.loaded_faces().iter().map(|f| f.id())).collect();
            // A split shape's glyphs may be spread over the faces compared first and the others.
            // Near matching spends an index's comparison budget, so with a tolerance the faces
            // compared first are indexed again for each search that has a split shape to look for.
            let again = |needed: bool| (needed && tolerance > 0.0).then(|| RetypeIndex::build_near(db, picked.iter().cloned(), &keys, &near));
            let fresh = again(left.iter().any(|s| !s.groups.is_empty()));
            let (rest, done, kept) = loaded_rest_for(db, &left, &[fresh.as_ref().unwrap_or(&index)], tolerance, &compared)?;
            let left: Vec<Sought> = left.into_iter().zip(done).filter(|(_, done)| !done).map(|(s, _)| s).collect();
            let fresh = again(left.iter().any(|s| !s.groups.is_empty()));
            let kept: Vec<RetypeIndex> = if tolerance > 0.0 {
                kept.iter().map(|ix| RetypeIndex::build_near(db, ix.faces().iter().cloned(), &keys, &near)).collect()
            } else {
                kept
            };
            let beside: Vec<&RetypeIndex> = std::iter::once(fresh.as_ref().unwrap_or(&index)).chain(&kept).collect();
            let mut found = search_beside(db, &left, &beside, tolerance, false)?;
            found.faces.splice(0..0, first.into_iter().chain(rest));
            found.scanned = scanned;
            found
        };
        Ok(RetypeFound { doc: self.doc, installed })
    }
}

#[cfg(test)]
#[path = "retype_installed_tests.rs"]
mod tests;
