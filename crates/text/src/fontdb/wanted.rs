use super::names::norm;
#[cfg(not(target_arch = "wasm32"))]
use super::scan::{file_face_names_within, is_font_file};
use super::traits::FaceStyle;
use super::*;

/// A font a document names that isn't available, to look for in font files ([`WantedFonts`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WantedFont {
    pub family: String,
    pub style: String,
    /// When the family is installed but not the style ([`FontMatch::Style`]): the family of the
    /// face [`FontDb::resolve`] gives instead. `None` when the family is missing.
    pub installed: Option<String>,
}

/// Fonts to look for in font files: [`WantedFonts::provided_by`] tells which of them a file
/// provides.
#[derive(Debug, Default)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct WantedFonts {
    fonts: Vec<WantedFont>,
    /// The indexes of the fonts by every name a face could answer to them by: the family as given
    /// and the installed family (ASCII-lowercased), and the family normalized ([`norm`]).
    by_name: HashMap<String, Vec<usize>>,
}

impl WantedFonts {
    /// The most faces and bytes of one file a search reads.
    #[cfg(not(target_arch = "wasm32"))]
    const SEARCH_FACES: u32 = 64;
    #[cfg(not(target_arch = "wasm32"))]
    const SEARCH_READ: u64 = 8 << 20;

    pub fn new(fonts: &[WantedFont]) -> Self {
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, w) in fonts.iter().enumerate() {
            let mut names = vec![w.family.to_ascii_lowercase(), norm(&w.family)];
            names.extend(w.installed.as_ref().map(|c| c.to_ascii_lowercase()));
            names.sort();
            names.dedup();
            for n in names.into_iter().filter(|n| !n.is_empty()) {
                by_name.entry(n).or_default().push(i);
            }
        }
        Self { fonts: fonts.to_vec(), by_name }
    }

    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// The indexes of the fonts that the font file at `path` provides: copied into a folder the
    /// font scan reads, the file makes [`FontDb::resolve`] find each of them exactly, in one of its
    /// faces or a variable font's named instances. The file is read as the scan reads it (its
    /// table directories and its `name`, `fvar` and `OS/2` tables), at most 64 faces and 8 MB of
    /// it, when it is a font file by its extension ([`is_font_file`]) or a suitcase font whose
    /// resource fork is at most 8 MB long. Sorted; none on the web.
    pub fn provided_by(&self, path: &Path) -> Vec<usize> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !is_font_file(path) && !crate::suitcase::is_suitcase(path) {
                return Vec::new();
            }
            let faces = file_face_names_within(path, Self::SEARCH_FACES, Self::SEARCH_READ);
            self.provided_by_faces(&faces)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            Vec::new()
        }
    }

    /// [`Self::provided_by`] for the faces of a file.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn provided_by_faces(&self, faces: &[FaceStyle]) -> Vec<usize> {
        let mut candidates: Vec<usize> = Vec::new();
        for f in faces {
            let names = std::iter::once(f.family.to_ascii_lowercase())
                .chain(f.keys.families.iter().cloned())
                .chain(f.keys.legacy.iter().map(|(family, _)| family.clone()))
                .chain(f.keys.full_names.iter().cloned())
                .chain(f.keys.postscript.iter().cloned());
            for n in names {
                candidates.extend(self.by_name.get(&n).into_iter().flatten());
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates.retain(|i| self.fonts.get(*i).is_some_and(|w| provides(w, faces)));
        candidates
    }
}

/// Whether the faces of a font file, cataloged by a rescan, make [`FontDb::resolve`] find `w`
/// exactly in one of them: [`FontDb::canonical`] and [`FontDb::find`] over these faces, as they
/// would run once the file is cataloged.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn provides(w: &WantedFont, faces: &[FaceStyle]) -> bool {
    let sk = norm(&w.style);
    // A face of `family` with the style among its style names, as `find` looks for it once the
    // family's files are loaded.
    let styled = |family: &str| faces.iter().any(|f| f.family.eq_ignore_ascii_case(family) && f.keys.styles.contains(&sk));
    let named_as_given = faces.iter().any(|f| f.family.eq_ignore_ascii_case(&w.family));
    if let Some(installed) = &w.installed {
        // A family cataloged by the name as given is found by it.
        if named_as_given && !w.family.eq_ignore_ascii_case(installed) {
            return styled(&w.family);
        }
        // The installed family is loaded: a face of it is loaded from the file only when its own
        // style is the one asked for (`is_cataloged`).
        return faces.iter().any(|f| f.family.eq_ignore_ascii_case(installed) && norm(&f.style) == sk);
    }
    if named_as_given {
        return styled(&w.family);
    }
    let key = norm(&w.family);
    if key.is_empty() {
        return false;
    }
    // Another of a family's names keeps the style asked for.
    if let Some(f) = faces.iter().find(|f| f.keys.families.contains(&key)) {
        return styled(&f.family);
    }
    // A legacy family with its legacy style, a full name or a PostScript name names its face
    // whatever style is asked for.
    if faces.iter().any(|f| {
        f.keys.legacy.iter().any(|(family, style)| *family == key && *style == sk)
            || f.keys.full_names.contains(&key)
            || f.keys.postscript.contains(&key)
    }) {
        return true;
    }
    // A legacy family with another style: that style of the family.
    faces.iter().find(|f| f.keys.legacy.iter().any(|(family, _)| *family == key)).is_some_and(|f| styled(&f.family))
}
