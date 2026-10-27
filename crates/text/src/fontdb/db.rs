use super::build::make_face;
#[cfg(not(target_arch = "wasm32"))]
use super::build::{style_italic, style_weight};
#[cfg(not(target_arch = "wasm32"))]
use super::cache::CatalogFace;
use super::cache::{CanonicalNames, Catalog, GlyphCache, PostScriptNames};
#[cfg(not(target_arch = "wasm32"))]
use super::dirs::confined_font_dirs;
use super::face::{BUNDLED, FontBytes, FontFace, Instances};
#[cfg(not(target_arch = "wasm32"))]
use super::names::SysFallback;
use super::names::name;
use super::scan::enumerate_faces;
#[cfg(not(target_arch = "wasm32"))]
use super::scan::{file_face_names, is_font_file};
#[cfg(test)]
use super::traits::faces_published;
#[cfg(not(target_arch = "wasm32"))]
use super::traits::{Alias, FaceStyle};
use super::*;

impl FontDb {
    /// A database holding the bundled fonts, whose system font scan reads `font_dirs` (folders,
    /// read with their subfolders, and font files).
    pub fn with_font_dirs(font_dirs: Vec<PathBuf>) -> Self {
        Self::new(FontPaths::Fixed(font_dirs))
    }

    /// Same scan paths as [`FontDb::global`] (platform lister and font folders), for tests.
    #[cfg(test)]
    pub(crate) fn for_system_scan_test() -> Self {
        Self::new(FontPaths::System)
    }

    fn new(font_paths: FontPaths) -> Self {
        let mut faces = Vec::new();
        // Bundled fonts first, then explicitly supplied craft-fonts faces: Japanese (Mincho
        // first), Arabic, Thai, then remaining faces in manifest order. Existing script fallback
        // priorities stay ahead of the additional families users can now pick by name.
        let craft = crate::craft_fonts::document_fonts().into_iter().map(|f| f.bytes);
        for data in BUNDLED.iter().copied().chain(craft) {
            for (i, spec) in enumerate_faces(data) {
                if let Some(f) = make_face(FontBytes::Static(data), i, spec, None) {
                    faces.push(Arc::new(f));
                }
            }
        }
        Self {
            faces: RwLock::new(faces),
            outlines: Mutex::new(HashMap::new()),
            images: Mutex::new(GlyphCache::default()),
            colours: Mutex::new(GlyphCache::default()),
            catalog: RwLock::new(Catalog::new()),
            postscript: RwLock::new(PostScriptNames::new()),
            font_paths,
            aliases: RwLock::new(HashMap::new()),
            canonical_cache: Mutex::new(CanonicalNames::new()),
            #[cfg(not(target_arch = "wasm32"))]
            cataloged: std::sync::OnceLock::new(),
            #[cfg(not(target_arch = "wasm32"))]
            version_scanned: RwLock::new(std::collections::HashSet::new()),
            family_cache: Mutex::new(None),
            menu_cache: Mutex::new(None),
            generation: AtomicU64::new(0),
            #[cfg(not(target_arch = "wasm32"))]
            sys: Mutex::new(SysFallback { enabled: true, ..Default::default() }),
            #[cfg(not(target_arch = "wasm32"))]
            stamp: Mutex::new(None),
            instances: Mutex::new(Instances::default()),
        }
    }

    /// Enable or disable the lazy system-font fallback for characters the loaded fonts lack
    /// (native only; on by default).
    pub fn set_system_fallback(&self, on: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.sys.lock().unwrap_or_else(|e| e.into_inner()).enabled = on;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = on;
    }

    /// Process-wide database: the bundled fonts, then the installed system fonts, cataloged the
    /// first time a lookup by family name needs them (or ahead of time by
    /// [`FontDb::scan_in_background`]).
    pub fn global() -> &'static FontDb {
        static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
        DB.get_or_init(|| FontDb::new(FontPaths::System))
    }

    pub(super) fn read_faces(&self) -> std::sync::RwLockReadGuard<'_, Vec<Arc<FontFace>>> {
        self.faces.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn read_catalog(&self) -> std::sync::RwLockReadGuard<'_, Catalog> {
        self.ensure_catalog();
        self.catalog.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Fonts were added or the catalog changed: cached family lists and [`FontDb::canonical`]'s
    /// answers are stale.
    pub(super) fn changed(&self) {
        *self.family_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.canonical_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A number that changes whenever the available fonts do (fonts added, system fonts
    /// rescanned): lists built from [`FontDb::family_list`] are current while it doesn't.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Family names available (loaded plus installed system fonts), sorted and deduplicated.
    pub fn families(&self) -> Vec<String> {
        self.family_list().to_vec()
    }

    /// [`FontDb::families`], shared: cheap to call every frame.
    pub fn family_list(&self) -> Arc<[String]> {
        let catalog = self.read_catalog();
        let mut cache = self.family_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = cache.as_ref() {
            return list.clone();
        }
        let mut v: Vec<String> = self.read_faces().iter().map(|f| f.family.clone()).collect();
        v.extend(catalog.values().map(|c| c.name.clone()));
        v.sort_by_key(|a| a.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let list: Arc<[String]> = v.into();
        *cache = Some(list.clone());
        list
    }

    /// The families font menus and lists show: [`FontDb::family_list`] without the system's hidden
    /// families, whose names start with "." (macOS keeps ".SF NS", ".LastResort" and others for its
    /// own interface, and Mac apps don't list them). Those still resolve by name, for documents and
    /// fallbacks that name them. Cheap to call every frame.
    pub fn menu_family_list(&self) -> Arc<[String]> {
        let all = self.family_list();
        let mut cache = self.menu_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((from, list)) = cache.as_ref()
            && Arc::ptr_eq(from, &all)
        {
            return list.clone();
        }
        let list: Arc<[String]> = all.iter().filter(|f| !f.starts_with('.')).cloned().collect();
        *cache = Some((all, list.clone()));
        list
    }

    /// How a font menu labels `family`: the English name when `english` is on (Preferences › Type ›
    /// Show Font Names in English, the default), else the font's native-language name when it has
    /// one. The document and commands still use the English family ([`Self::canonical`]).
    pub fn family_display_name(&self, family: &str, english: bool) -> String {
        let (family, _) = self.canonical(family, "");
        if english {
            return family;
        }
        if let Some(native) = self.read_faces().iter().find(|f| f.family.eq_ignore_ascii_case(&family)).and_then(|f| f.native_family.clone()) {
            return native;
        }
        if let Some(native) = self.read_catalog().get(&family.to_ascii_lowercase()).and_then(|c| c.native.clone()) {
            return native;
        }
        family
    }

    /// Style names available for `family`: upright styles by weight, then italics.
    pub fn styles(&self, family: &str) -> Vec<String> {
        let (family, _) = self.canonical(family, "");
        let family = family.as_str();
        let mut v: Vec<(bool, f32, String)> = Vec::new();
        // Each style once, as its first loaded version describes it (versions can disagree on weight).
        for f in self.read_faces().iter().filter(|f| f.family.eq_ignore_ascii_case(family)) {
            if !v.iter().any(|(_, _, s)| s.eq_ignore_ascii_case(&f.style)) {
                v.push((f.italic, f.weight, f.style.clone()));
            }
        }
        if let Some(c) = self.read_catalog().get(&family.to_ascii_lowercase()) {
            for cf in &c.faces {
                if !v.iter().any(|(_, _, s)| s.eq_ignore_ascii_case(&cf.style)) {
                    v.push((cf.italic, cf.weight, cf.style.clone()));
                }
            }
        }
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        v.dedup_by(|a, b| a.2 == b.2);
        v.into_iter().map(|t| t.2).collect()
    }

    /// Add a user font (TTF/OTF/TTC bytes). Returns the number of faces added (0 if unparseable or
    /// every face was already present).
    pub fn add_font(&self, bytes: Vec<u8>) -> usize {
        self.add_font_from(bytes, None)
    }

    /// [`FontDb::add_font`] for the bytes of the file at `path` (when known).
    pub(super) fn add_font_from(&self, bytes: Vec<u8>, path: Option<&Path>) -> usize {
        self.add_fonts([(bytes, path.map(Path::to_path_buf))])
    }

    /// Add the faces of `files` (each file's bytes, and the file when known) all at once: a lookup
    /// on another thread finds all of them or none, never a family with only some of its styles
    /// loaded, whose nearest style it would take instead (Arial Bold Italic, the first of Arial's
    /// files, for upright Regular text). Returns the number of faces added.
    pub(super) fn add_fonts(&self, files: impl IntoIterator<Item = (Vec<u8>, Option<PathBuf>)>) -> usize {
        // Another version of a family and style is kept: a document may name it (the first one
        // loaded stays the one the family and style alone resolve to).
        let same = |f: &FontFace, family: &str, style: &str, version: &str| {
            f.family.eq_ignore_ascii_case(family) && f.style.eq_ignore_ascii_case(style) && f.version == version
        };
        let mut made: Vec<FontFace> = Vec::new();
        for (bytes, path) in files {
            let data = Arc::new(bytes);
            for (i, spec) in enumerate_faces(&data) {
                let version = skrifa::FontRef::from_index(&data, i).ok().and_then(|f| name(&f, &[StringId::VERSION_STRING])).unwrap_or_default();
                let present = |f: &FontFace| same(f, &spec.family, &spec.style, &version);
                if self.read_faces().iter().any(|f| present(f)) || made.iter().any(present) {
                    continue;
                }
                if let Some(f) = make_face(FontBytes::Owned(data.clone()), i, spec, path.clone()) {
                    made.push(f);
                }
            }
        }
        let mut faces = self.faces.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = faces.len();
        for f in made {
            // Another thread may have added the same face meanwhile.
            if !faces.iter().any(|g| same(g, &f.family, &f.style, &f.version)) {
                faces.push(Arc::new(f));
            }
        }
        let added = faces.len() - before;
        drop(faces);
        #[cfg(test)]
        faces_published();
        if added > 0 {
            self.changed();
        }
        added
    }

    /// Catalog the installed fonts unless that has been done: the first caller scans the font
    /// folders, any other waits for that scan to finish. A no-op on wasm.
    pub(super) fn ensure_catalog(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.cataloged.get_or_init(|| {
            self.scan_font_dirs();
        });
    }

    /// Catalog the installed fonts on a background thread, so the first lookup by family name
    /// (opening a file, the font menus) doesn't wait for the scan. A no-op once they are
    /// cataloged, and on wasm.
    pub fn scan_in_background(&'static self) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.cataloged.get().is_none() {
            // A failed spawn leaves the scan to the first lookup that needs it.
            let _ = std::thread::Builder::new().name("font-scan".into()).spawn(move || self.ensure_catalog());
        }
    }

    /// Scan the font folders again (fonts installed or removed since), cataloging the faces
    /// found (native only). Font data is loaded when a cataloged family is first resolved. Faces
    /// loaded from files the scan no longer finds are dropped. Returns the number of faces
    /// cataloged.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_system_fonts(&self) -> usize {
        // The first scan, or a rescan once it is done (never both at once).
        let mut first = None;
        self.cataloged.get_or_init(|| first = Some(self.scan_font_dirs()));
        first.unwrap_or_else(|| self.scan_font_dirs())
    }

    /// Whether fonts were installed or removed since the last scan (a font folder or font file it
    /// read changed, or a new one is to be read), for [`FontDb::load_system_fonts`] to catalog
    /// them. Costs a look at each folder the scan read, not a scan. `false` before the first scan
    /// and on wasm.
    pub fn installed_fonts_changed(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let paths = self.font_paths.get();
            let stamp = self.stamp.lock().unwrap_or_else(|e| e.into_inner());
            stamp.as_ref().is_some_and(|s| s.paths != paths || s.modified.iter().any(|(p, t)| modified(p) != *t))
        }
        #[cfg(target_arch = "wasm32")]
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn scan_font_dirs(&self) -> usize {
        // Files may have been installed since: other versions are looked for again.
        self.version_scanned.write().unwrap_or_else(|e| e.into_inner()).clear();
        let mut catalog = Catalog::new();
        let mut aliases: HashMap<String, Vec<Alias>> = HashMap::new();
        let mut postscript = PostScriptNames::new();
        let mut n = 0;
        let paths = self.font_paths.get();
        let mut modified_at = Vec::new();
        // Each file with whether it was named by itself rather than found in a folder.
        let mut files = Vec::new();
        let mut stack = paths.clone();
        // Each folder once, however links lead back to it.
        let mut visited = std::collections::HashSet::new();
        let confined = confined_font_dirs();
        let inside_confined = |p: &Path| std::fs::canonicalize(p).is_ok_and(|r| confined.iter().any(|root| r.starts_with(root)));
        while let Some(d) = stack.pop() {
            if !visited.insert(std::fs::canonicalize(&d).unwrap_or_else(|_| d.clone())) {
                continue;
            }
            // In a confined folder, a link is followed only to somewhere inside one.
            let in_confined = !confined.is_empty() && inside_confined(&d);
            // Before listing it: a font installed meanwhile changes it after this.
            modified_at.push((d.clone(), modified(&d)));
            // Not a folder: a font file named by itself (or nothing, yet).
            let Ok(rd) = std::fs::read_dir(&d) else {
                files.push((d, true));
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if in_confined && e.file_type().is_ok_and(|t| t.is_symlink()) && !inside_confined(&p) {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else {
                    files.push((p, false));
                }
            }
        }
        for (p, named) in files {
            // A file named by itself is a font whatever its name (font services keep fonts in
            // files without an extension); one of a folder's only with a font's extension, or
            // a suitcase font (whose fonts are in its resource fork, whatever its name).
            if !named && !is_font_file(&p) && !crate::suitcase::is_suitcase(&p) {
                continue;
            }
            for FaceStyle { family, native_family, style, keys, weight, italic, traits, .. } in file_face_names(&p) {
                for (k, a) in Alias::of(&family, &style, &keys) {
                    aliases.entry(k).or_default().push(a);
                }
                // `keys.postscript` is normalized ([`norm`]), as `by_postscript_name` looks it up.
                for ps in &keys.postscript {
                    postscript.insert(ps.clone(), (family.clone(), style.clone()));
                }
                let entry = catalog.entry(family.to_ascii_lowercase()).or_default();
                let kind = entry.traits.get_or_insert(traits);
                kind.variable |= traits.variable;
                if entry.name.is_empty() {
                    entry.name = family;
                }
                if entry.native.is_none() {
                    entry.native = native_family;
                }
                let (weight, italic) = (weight.unwrap_or_else(|| style_weight(&style)), italic || style_italic(&style));
                entry.faces.push(CatalogFace { style, path: p.clone(), weight, italic });
                n += 1;
            }
        }
        log::debug!("cataloged {n} system font faces");
        let installed: std::collections::HashSet<PathBuf> = catalog.values().flat_map(|c| c.faces.iter().map(|cf| cf.path.clone())).collect();
        *self.stamp.lock().unwrap_or_else(|e| e.into_inner()) = Some(ScanStamp { paths, modified: modified_at });
        *self.catalog.write().unwrap_or_else(|e| e.into_inner()) = catalog;
        *self.aliases.write().unwrap_or_else(|e| e.into_inner()) = aliases;
        *self.postscript.write().unwrap_or_else(|e| e.into_inner()) = postscript;
        // Faces loaded from a file this scan did not catalog (the font was removed, or its folder
        // is no longer read) are dropped with their outlines. A family with no face left and no
        // cataloged file leaves the lists, and type set in it resolves to the fallback font. A face
        // without a file (bundled, or added as bytes) stays.
        let mut dropped = std::collections::HashSet::new();
        self.faces.write().unwrap_or_else(|e| e.into_inner()).retain(|f| {
            let keep = f.path.as_ref().is_none_or(|p| installed.contains(p));
            if !keep {
                dropped.insert(f.id);
            }
            keep
        });
        if !dropped.is_empty() {
            log::debug!("dropped {} font faces whose files are no longer installed", dropped.len());
            self.outlines.lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _), _| !dropped.contains(id));
        }
        // A font installed since may cover the characters no font did: they are looked for again.
        {
            let mut sys = self.sys.lock().unwrap_or_else(|e| e.into_inner());
            sys.scans = sys.scans.wrapping_add(1);
            sys.misses.clear();
        }
        self.changed();
        n
    }
}
