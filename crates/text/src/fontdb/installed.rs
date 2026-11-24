//! Installed faces that nothing has loaded yet, read for one search without adding them to the
//! database (Retype's matching, `vectorcraft_text::retype`).

#[cfg(not(target_arch = "wasm32"))]
use super::build::make_face;
#[cfg(not(target_arch = "wasm32"))]
use super::face::FontBytes;
#[cfg(not(target_arch = "wasm32"))]
use super::scan::enumerate_faces;
use super::{Arc, FontDb, FontFace, Path, PathBuf};

/// One style of an installed family that is not loaded: no loaded face has its family and style.
#[derive(Clone, Debug, PartialEq)]
pub struct InstalledFace {
    pub family: String,
    pub style: String,
    /// The font file holding it (a suitcase font's file, whose fonts are in its resource fork).
    pub path: PathBuf,
    /// The file's length in bytes (a suitcase font's resource fork's), when the scan read it.
    pub len: u64,
}

impl FontDb {
    /// The installed faces no loaded face has the family and style of, by family name and then
    /// style, as the font scan cataloged them. Reading the list reads no font file. Empty on wasm,
    /// where no fonts are installed.
    pub fn unloaded_installed_faces(&self) -> Vec<InstalledFace> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            // The catalog first: the first scan takes the faces' lock, which a guard held here would block.
            // The bundled faces have no file to read. The catalog lists a bundled face ahead of an
            // installed file of the same face, so each face's file is its first installed version.
            let cataloged: Vec<(String, String, PathBuf)> = self
                .catalog
                .faces()
                .iter()
                .filter_map(|f| {
                    let path =
                        self.catalog.versions(f).into_iter().filter(|v| v.source != craft::font_catalog::Source::Bundled).find_map(|v| v.path)?;
                    Some((f.family.clone(), f.style.clone(), path))
                })
                .collect();
            let unloaded: Vec<(String, String, PathBuf)> = {
                let loaded = self.read_faces();
                let is_loaded =
                    |family: &str, style: &str| loaded.iter().any(|f| f.family.eq_ignore_ascii_case(family) && f.style.eq_ignore_ascii_case(style));
                cataloged.into_iter().filter(|(family, style, _)| !is_loaded(family, style)).collect()
            };
            let mut out: Vec<InstalledFace> =
                unloaded.into_iter().map(|(family, style, path)| InstalledFace { family, style, len: file_len(&path), path }).collect();
            out.sort_by(|a, b| {
                a.family
                    .to_lowercase()
                    .cmp(&b.family.to_lowercase())
                    .then_with(|| a.style.to_lowercase().cmp(&b.style.to_lowercase()))
                    .then(a.path.cmp(&b.path))
            });
            out
        }
        #[cfg(target_arch = "wasm32")]
        {
            vec![]
        }
    }

    /// The faces in the font file at `path` (a suitcase font's in its resource fork), read for one
    /// search and not added to the database, with the bytes read: no faces when the file, or the
    /// fork, is longer than `max_len` bytes ([`UnloadedFile::too_long`]) or holds no font. Each face
    /// read has an id of its own, which no loaded face has, and its path is `path`. Empty on wasm.
    pub fn read_unloaded_file(&self, path: &Path, max_len: u64) -> UnloadedFile {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (files, bytes) = if crate::suitcase::is_suitcase(path) {
                match crate::suitcase::read_fonts_within(path, max_len) {
                    (_, n, true) => return UnloadedFile { bytes: n, too_long: true, ..Default::default() },
                    (fonts, n, false) => (fonts, n),
                }
            } else {
                match read_within(path, max_len) {
                    Some((data, n)) if n <= max_len => (vec![data], n),
                    Some((_, n)) => return UnloadedFile { bytes: n, too_long: true, ..Default::default() },
                    None => return UnloadedFile::default(),
                }
            };
            let mut faces = vec![];
            for data in files {
                let data = Arc::new(data);
                for (i, spec) in enumerate_faces(&data) {
                    if let Some(f) = make_face(FontBytes::Owned(data.clone()), i, spec, Some(path.to_path_buf())) {
                        faces.push(Arc::new(f));
                    }
                }
            }
            UnloadedFile { faces, bytes, too_long: false }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, max_len);
            UnloadedFile::default()
        }
    }
}

/// What [`FontDb::read_unloaded_file`] read from one font file.
#[derive(Default)]
pub struct UnloadedFile {
    /// The faces read, not loaded.
    pub faces: Vec<Arc<FontFace>>,
    /// The bytes read from the file, those of a file longer than the limit included (at most one
    /// past it).
    pub bytes: u64,
    /// The file, or the fork, is longer than the limit: no face was read.
    pub too_long: bool,
}

impl FontDb {
    /// Load `face`, a face [`Self::read_unloaded_file`] read, without reading its file again or
    /// the family's other files: it joins the loaded faces, unless a face of its family, style and
    /// version is loaded already, which is returned instead.
    pub fn load_read_face(&self, face: Arc<FontFace>) -> Arc<FontFace> {
        let same =
            |f: &FontFace| f.family.eq_ignore_ascii_case(&face.family) && f.style.eq_ignore_ascii_case(&face.style) && f.version == face.version;
        let mut faces = self.faces.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(f) = faces.iter().find(|f| same(f)) {
            return f.clone();
        }
        faces.push(face.clone());
        drop(faces);
        self.changed();
        face
    }
}

/// The length of the font file at `path`, or of its resource fork for a suitcase font; 0 when it
/// cannot be read.
#[cfg(not(target_arch = "wasm32"))]
fn file_len(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) if m.len() == 0 => crate::suitcase::fork_len(path),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// The file at `path`, read no further than one byte past `max_len`, with the bytes read: more than
/// `max_len` when the file is longer. None when it cannot be read.
#[cfg(not(target_arch = "wasm32"))]
fn read_within(path: &Path, max_len: u64) -> Option<(Vec<u8>, u64)> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path).ok()?.take(max_len.saturating_add(1)).read_to_end(&mut data).ok()?;
    let n = data.len() as u64;
    Some((data, n))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    /// A database whose catalog holds the bundled font files, with Inter's faces dropped from the
    /// loaded ones: Inter is installed and not loaded.
    fn inter_unloaded() -> FontDb {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        let db = FontDb::with_font_dirs(vec![dir]);
        db.faces.write().unwrap().retain(|f| f.family != "Inter");
        db
    }

    #[test]
    fn installed_faces_not_loaded_are_listed_and_read_without_loading_them() {
        let db = inter_unloaded();
        let list = db.unloaded_installed_faces();
        let inter: Vec<&InstalledFace> = list.iter().filter(|f| f.family == "Inter").collect();
        assert!(inter.iter().any(|f| f.style == "Regular"), "{list:?}");
        // The loaded families are not listed.
        assert!(!list.iter().any(|f| f.family == "Source Sans 3"), "{list:?}");
        let regular = inter.iter().find(|f| f.style == "Regular").unwrap();
        assert_eq!(regular.len, std::fs::metadata(&regular.path).unwrap().len());
        let before = db.loaded_faces().len();
        let file = db.read_unloaded_file(&regular.path, regular.len);
        assert_eq!((file.bytes, file.too_long), (regular.len, false));
        let read = file.faces;
        assert!(read.iter().any(|f| f.family == "Inter" && f.style == "Regular"), "the file's face is read");
        assert!(read.iter().all(|f| f.path() == Some(regular.path.as_path())));
        // Read for the search only: the database does not hold it, and no loaded face has its id.
        assert_eq!(db.loaded_faces().len(), before);
        assert!(read.iter().all(|f| db.face_by_id(f.id()).is_none()));
        assert!(db.unloaded_installed_faces().iter().any(|f| f.family == "Inter"), "still not loaded");
        // A file longer than the limit is not read past one byte over it, and says so.
        let long = db.read_unloaded_file(&regular.path, regular.len - 1);
        assert!(long.faces.is_empty());
        assert_eq!((long.bytes, long.too_long), (regular.len, true));
        let missing = db.read_unloaded_file(&regular.path.with_extension("missing"), u64::MAX);
        assert!(missing.faces.is_empty());
        assert_eq!((missing.bytes, missing.too_long), (0, false));
        // Once loaded, the family is no longer listed.
        assert!(db.face("Inter", "Regular").is_some_and(|f| f.family == "Inter"));
        assert!(!db.unloaded_installed_faces().iter().any(|f| f.family == "Inter" && f.style == "Regular"));
    }
}
