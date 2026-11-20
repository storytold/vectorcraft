//! Retype's installed fonts: the faces of installed fonts that nothing has loaded yet are
//! candidates too, as the fonts on the device are among the sources Retype suggests fonts from
//! (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/identify-fonts-using-retype.html>).
//! Each font file is read for the search and let go; a face whose glyphs match a shape is loaded
//! as it was read, without the family's other files. Files are read smallest first, within a budget
//! of bytes read and glyphs drawn, and the search stops between files when it is canceled. The app
//! runs it off the UI thread ([`RetypeRun`]), with a Cancel.

use std::collections::BTreeSet;
use std::sync::Arc;

use kurbo::BezPath;
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_text::retype::{MAX_FACES, RetypeIndex, encoded_glyphs, is_plain, shape_key};
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

/// The installed faces nothing has loaded whose glyphs match at least one of `shapes` that is not
/// plain ([`is_plain`]: a rectangle, a triangle or a bar), each loaded into `db` as it was read.
/// With `every` false the search stops once each such shape matches a face found; with it, every
/// face within the limits ([`LIMITS`]) is compared. No file is read when every shape is plain. At
/// most [`MAX_FACES`] faces are found; the faces not compared past that count as skipped.
pub(super) fn search(db: &FontDb, shapes: &[&BezPath], every: bool) -> Result<Installed> {
    search_within(db, shapes, every, LIMITS, &|_| true)
}

/// [`search`] comparing every face within the limits, for a list of at most `faces` faces: a face
/// found that `listed` turns down is loaded and not listed, and takes no place of the `faces`.
pub(super) fn search_listed(db: &FontDb, shapes: &[&BezPath], faces: usize, listed: &dyn Fn(&FontFace) -> bool) -> Result<Installed> {
    search_within(db, shapes, true, Limits { faces: faces.min(MAX_FACES), ..LIMITS }, listed)
}

/// Would a search for `shapes` read any installed file: is one of them not plain, with a glyph's
/// key ([`shape_key`])?
pub(super) fn starts_search(shapes: &[&BezPath]) -> bool {
    shapes.iter().any(|s| !is_plain(s) && shape_key(s).is_some())
}

/// The loaded faces that are not among `compared` (those past the [`MAX_FACES`] a Retype compares
/// first) whose glyphs match one of `shapes` that is not plain ([`is_plain`]), compared
/// [`MAX_FACES`] at a time. A cancel stops it between those groups.
pub(super) fn loaded_rest(db: &FontDb, shapes: &[&BezPath], compared: &[u32]) -> Result<Vec<Arc<FontFace>>> {
    let shapes: Vec<&BezPath> = shapes.iter().copied().filter(|s| !is_plain(s)).collect();
    let keys: BTreeSet<u64> = shapes.iter().filter_map(|s| shape_key(s)).collect();
    let mut out: Vec<Arc<FontFace>> = vec![];
    if keys.is_empty() {
        return Ok(out);
    }
    let rest: Vec<Arc<FontFace>> = db.loaded_faces().into_iter().filter(|f| !compared.contains(&f.id())).collect();
    for group in rest.chunks(MAX_FACES) {
        vectorcraft_doc::cancel::check()?;
        let index = RetypeIndex::build(db, group.iter().cloned(), &keys);
        for s in &shapes {
            for m in index.all_matches(s) {
                if !out.iter().any(|f| f.id() == m.face.id()) {
                    out.push(m.face);
                }
            }
        }
    }
    Ok(out)
}

/// An installed font file: its path, its length and the (family, style) of the faces wanted from it.
type File = (std::path::PathBuf, u64, Vec<(String, String)>);

/// [`search`] within `limits`, keeping in its faces only those `listed` accepts.
fn search_within(db: &FontDb, shapes: &[&BezPath], every: bool, limits: Limits, listed: &dyn Fn(&FontFace) -> bool) -> Result<Installed> {
    let mut out = Installed::default();
    // A shape without a glyph's key matches no face: the search does not wait for it.
    let shapes: Vec<&BezPath> = shapes.iter().copied().filter(|s| !is_plain(s) && shape_key(s).is_some()).collect();
    let keys: BTreeSet<u64> = shapes.iter().filter_map(|s| shape_key(s)).collect();
    if keys.is_empty() {
        return Ok(out);
    }
    let keys = &keys;
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
    let (mut bytes_left, mut glyphs_left) = (limits.read, limits.glyphs);
    let total = files.len() as u64;
    for (i, (path, len, styles)) in files.iter().enumerate() {
        vectorcraft_doc::cancel::check()?;
        vectorcraft_doc::cancel::progress(i as u64, total, "Retype: reading installed fonts");
        if *len > limits.file || *len > bytes_left {
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
            let index = RetypeIndex::build_unloaded(db, face.clone(), keys);
            if index.is_empty() {
                continue;
            }
            let fits: Vec<usize> = shapes
                .iter()
                .zip(&matched)
                .enumerate()
                .filter(|(_, (s, m))| (every || !**m) && !index.matches(s).is_empty())
                .map(|(k, _)| k)
                .collect();
            if fits.is_empty() {
                continue;
            }
            let loaded = load(db, face);
            let fits = still_fit(db, face, &loaded, &shapes, keys, fits);
            if fits.is_empty() {
                continue;
            }
            // A face `listed` turns down (another version shadows it) is not compared ahead of others.
            if listed(&loaded) {
                remember(db, &loaded);
                if !out.faces.iter().any(|f| f.id() == loaded.id()) {
                    out.faces.push(loaded);
                }
            }
            for k in fits {
                if let Some(m) = matched.get_mut(k) {
                    *m = true;
                }
            }
            if !every && matched.iter().all(|m| *m) {
                return Ok(out);
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

/// Of `fits` (indices into `shapes` that `read` matches), those that `loaded`, the face loading
/// `read` gave, matches too: all of them when it is `read` itself. Another face of the same
/// family, style and version, loaded from another file, may have other glyphs.
fn still_fit(db: &FontDb, read: &FontFace, loaded: &Arc<FontFace>, shapes: &[&BezPath], keys: &BTreeSet<u64>, fits: Vec<usize>) -> Vec<usize> {
    if loaded.id() == read.id() {
        return fits;
    }
    let index = RetypeIndex::build(db, [loaded.clone()], keys);
    fits.into_iter().filter(|k| shapes.get(*k).is_some_and(|s| !index.matches(s).is_empty())).collect()
}

/// `face` (read from an installed file and not loaded) loaded into `db` as it was read
/// ([`FontDb::load_read_face`]): neither its file nor the family's other files are read again.
fn load(db: &FontDb, face: &Arc<FontFace>) -> Arc<FontFace> {
    db.load_read_face(face.clone())
}

/// A search of the installed fonts made away from its command, so the app can run it off the UI
/// thread with a Cancel. [`RetypeRun::prepare`] takes the selected shapes and the loaded faces to
/// compare from the session, checking the params as the command checks them; [`RetypeSearch::run`]
/// compares them and searches the installed fonts on any thread, loading the faces that match; and
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
    shapes: Vec<BezPath>,
    faces: Vec<Arc<FontFace>>,
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
    /// active document. None when it reads no installed font: `installed` is false, or no shape has
    /// a glyph's key. An error where the command fails on its params or on the selection.
    pub fn prepare(s: &Session, command: &str, p: &Value) -> Result<Option<RetypeRun>> {
        Self::prepare_in(s, command, p, FontDb::global())
    }

    /// [`RetypeRun::prepare`] with the fonts of `db`.
    fn prepare_in(s: &Session, command: &str, p: &Value, db: &FontDb) -> Result<Option<RetypeRun>> {
        let (command, fonts) = match command {
            "type.retype" => ("type.retype", Some(super::fonts_param(p)?)),
            "type.retypeFonts" => {
                super::fonts::count_param(p)?;
                ("type.retypeFonts", None)
            }
            other => return Err(EngineError::Other(format!("`{other}` doesn't search the installed fonts"))),
        };
        if !wanted(command, p)? {
            return Ok(None);
        }
        let (pieces, _) = super::selected_pieces(s)?;
        let st = s.doc()?;
        // Retype compares the shapes that may become type; the list compares every shape.
        let shapes: Vec<BezPath> = pieces.into_iter().filter(|pc| fonts.is_none() || !pc.keeps_outline).map(|pc| pc.path).collect();
        if !shapes.iter().any(|sh| shape_key(sh).is_some()) {
            return Ok(None);
        }
        let faces = match &fonts {
            Some(fonts) => super::candidate_faces(db, &st.doc, fonts),
            None => super::fonts::listed_faces(db, &st.doc),
        };
        Ok(Some(RetypeRun {
            params: p.clone(),
            doc: st.uid,
            content: st.doc.clone(),
            selection: st.selection.objects.clone(),
            search: Arc::new(RetypeSearch { command, doc: st.uid, shapes, faces }),
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
        let shapes: Vec<&BezPath> = self.shapes.iter().collect();
        let installed = if self.command == "type.retypeFonts" {
            let (ranked, mut found) = super::fonts::rank_fonts(db, self.faces.clone(), &shapes, true, None)?;
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
            let keys: BTreeSet<u64> = shapes.iter().filter_map(|sh| shape_key(sh)).collect();
            let index = RetypeIndex::build(db, self.faces.iter().cloned(), &keys);
            let left: Vec<&BezPath> = shapes.iter().copied().filter(|sh| index.matches(sh).is_empty()).collect();
            // The first faces that match are handed in too: loading the faces found can move one
            // past the first 96 the command compares.
            let mut first: Vec<Arc<FontFace>> = vec![];
            for m in shapes.iter().flat_map(|sh| index.all_matches(sh)) {
                if !first.iter().any(|f| f.id() == m.face.id()) {
                    first.push(m.face);
                }
            }
            // The loaded faces past the ones compared first, then the installed fonts for what
            // none of them matches; the command takes both from what is handed in.
            let compared: Vec<u32> = self.faces.iter().map(|f| f.id()).collect();
            // Every face loaded now is compared, among the first ones or with the rest.
            let scanned: Vec<u32> = compared.iter().copied().chain(db.loaded_faces().iter().map(|f| f.id())).collect();
            let rest = loaded_rest(db, &left, &compared)?;
            let mut left = left;
            for group in rest.chunks(MAX_FACES) {
                let index = RetypeIndex::build(db, group.iter().cloned(), &keys);
                left.retain(|sh| index.matches(sh).is_empty());
            }
            let mut found = search(db, &left, false)?;
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
