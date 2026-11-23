use craft::font_catalog::{Catalog, FontEnv, Os};

use super::build::make_face;
use super::cache::{CanonicalNames, GlyphCache};
#[cfg(not(target_arch = "wasm32"))]
use super::dirs::confined_font_dirs;
use super::dirs::system_catalog;
use super::face::{BUNDLED, FontBytes, FontFace, Instances};
#[cfg(not(target_arch = "wasm32"))]
use super::names::SysFallback;
use super::names::{name, pick_native_label};
use super::scan::enumerate_faces;
#[cfg(test)]
use super::traits::faces_published;
use super::{Arc, AtomicU64, FontDb, HashMap, Mutex, Ordering, Path, PathBuf, RwLock, StringId};

/// The next [`FontDb::id`].
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// [`FontDb::global`].
static GLOBAL: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();

impl FontDb {
    /// A database holding the bundled fonts, whose system font scan reads `font_dirs` (folders,
    /// read with their subfolders, and font files) and nothing else.
    pub fn with_font_dirs(font_dirs: Vec<PathBuf>) -> Self {
        // No platform folders, font store or font service.
        let catalog = Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false);
        // A path that isn't a folder is a font file named by itself, read whatever its name (font
        // services keep fonts in files without an extension); one that becomes a folder is read as one.
        let (folders, files): (Vec<PathBuf>, Vec<PathBuf>) = font_dirs.into_iter().partition(|p| p.is_dir());
        let catalog = catalog.with_lister(Arc::new(move || files.clone()));
        catalog.set_app_dirs(folders);
        Self::new(catalog)
    }

    /// Same scan paths as [`FontDb::global`] (platform lister and font folders), for tests.
    #[cfg(test)]
    pub(crate) fn for_system_scan_test() -> Self {
        Self::new(system_catalog())
    }

    fn new(catalog: Catalog) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        catalog.set_confined_dirs(confined_font_dirs());
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
            catalog: Arc::new(catalog),
            file_faces: RwLock::new(super::FileFaces::default()),
            canonical_cache: Mutex::new(CanonicalNames::new()),
            #[cfg(not(target_arch = "wasm32"))]
            version_scanned: RwLock::new(std::collections::HashSet::new()),
            family_cache: Mutex::new(None),
            menu_cache: Mutex::new(None),
            generation: AtomicU64::new(0),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            #[cfg(not(target_arch = "wasm32"))]
            sys: Mutex::new(SysFallback { enabled: true, ..Default::default() }),
            instances: Mutex::new(Instances::default()),
            composites: RwLock::new(HashMap::new()),
            custom_classes: Mutex::new(HashMap::new()),
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

    /// Is the lazy system-font fallback on ([`Self::set_system_fallback`])?
    pub(crate) fn system_fallback_on(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.sys.lock().unwrap_or_else(|e| e.into_inner()).enabled
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Process-wide database: the bundled fonts, then the installed system fonts, cataloged the
    /// first time a lookup by family name needs them (or ahead of time by
    /// [`FontDb::scan_in_background`]).
    pub fn global() -> &'static FontDb {
        GLOBAL.get_or_init(|| FontDb::new(system_catalog()))
    }

    /// [`FontDb::global`] when it has been made, without making it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn global_if_made() -> Option<&'static FontDb> {
        GLOBAL.get()
    }

    /// The installed fonts this database finds by name, shared with the framework's font menus and
    /// pickers. Its lookups wait for its first scan ([`FontDb::scan_in_background`]).
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    pub(super) fn read_faces(&self) -> std::sync::RwLockReadGuard<'_, Vec<Arc<FontFace>>> {
        self.faces.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Fonts were added or the catalog changed: cached family lists and [`FontDb::canonical`]'s
    /// answers are stale.
    pub(super) fn changed(&self) {
        *self.family_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.canonical_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A number that changes whenever the available fonts do (fonts added, system fonts
    /// rescanned, here or by the catalog's own watch): lists built from [`FontDb::family_list`] are
    /// current while it doesn't. While a guard of this thread lends composite fonts other than the
    /// ones the database holds ([`FontDb::lend_composite_fonts`]), it is a number of its own for
    /// those definitions.
    pub fn generation(&self) -> u64 {
        self.lent_generation(self.own_generation())
    }

    /// [`FontDb::generation`] without a loan's: the database's count and its catalog's.
    pub(super) fn own_generation(&self) -> u64 {
        // Both count up from 0, so their sum moves whenever either does.
        self.generation.load(Ordering::Relaxed).wrapping_add(self.catalog.generation())
    }

    /// A number no other database made in this process has: with [`FontDb::generation`], it tells
    /// what a list built from a database was built from.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Family names available (loaded plus installed system fonts), sorted and deduplicated.
    pub fn families(&self) -> Vec<String> {
        self.family_list().to_vec()
    }

    /// [`FontDb::families`], shared: cheap to call every frame.
    pub fn family_list(&self) -> Arc<[String]> {
        // The catalog's list is the same `Arc` until it rescans (here or in its watch).
        let installed = self.catalog.families();
        let mut cache = self.family_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((from, list)) = cache.as_ref()
            && Arc::ptr_eq(from, &installed)
        {
            return list.clone();
        }
        let mut v: Vec<String> = self.read_faces().iter().map(|f| f.family.clone()).collect();
        v.extend(installed.iter().cloned());
        v.sort_by_key(|a| a.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let list: Arc<[String]> = v.into();
        *cache = Some((installed, list.clone()));
        list
    }

    /// The families font menus and lists show: [`FontDb::family_list`] without the system's hidden
    /// families, whose names start with "." (macOS keeps ".SF NS", ".LastResort" and others for its
    /// own interface, and Mac apps don't list them). Those still resolve by name, for documents and
    /// fallbacks that name them. The composite fonts ([`FontDb::composite_font_names`]) come first.
    /// Cheap to call every frame.
    pub fn menu_family_list(&self) -> Arc<[String]> {
        let all = self.family_list();
        let mut cache = self.menu_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((from, list)) = cache.as_ref()
            && Arc::ptr_eq(from, &all)
        {
            return list.clone();
        }
        let composites = self.composite_font_names();
        let list: Arc<[String]> = composites.into_iter().chain(all.iter().filter(|f| !f.starts_with('.')).cloned()).collect();
        *cache = Some((all, list.clone()));
        list
    }

    /// Whether the installed fonts are cataloged, so lookups by family name no longer wait for
    /// the scan. Always true on wasm, which has no installed fonts.
    pub fn installed_fonts_ready(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.catalog.is_ready()
        }
        #[cfg(target_arch = "wasm32")]
        true
    }

    /// [`FontDb::menu_family_list`] without waiting for the installed fonts' scan: until it ends,
    /// the families already loaded (the bundled fonts and any loaded meanwhile), sorted, without
    /// the hidden ones. For what is built every frame, such as the menu bar's Type › Font, so the
    /// window's first frame doesn't wait for the scan of a large font library (#5483).
    /// [`FontDb::generation`] changes when the scan ends.
    pub fn menu_family_list_now(&self) -> Arc<[String]> {
        if self.installed_fonts_ready() {
            return self.menu_family_list();
        }
        let mut v: Vec<String> = self.read_faces().iter().map(|f| f.family.clone()).filter(|f| !f.starts_with('.')).collect();
        v.sort_by_key(|a| a.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        self.composite_font_names().into_iter().chain(v).collect()
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
        // An installed family's, from its names in other languages, as a loaded face's is chosen
        // (every face of a family has the same names: one is enough).
        let native = self.catalog.resolve(&family, "").and_then(|(f, _)| {
            let names: Vec<(Option<String>, String)> = f.localized_families.into_iter().map(|n| (n.language, n.name)).collect();
            pick_native_label(&f.family, &names)
        });
        native.unwrap_or(family)
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
        for f in self.catalog.styles(family) {
            if !v.iter().any(|(_, _, s)| s.eq_ignore_ascii_case(&f.style)) {
                v.push((f.italic, f.weight, f.style));
            }
        }
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        v.dedup_by(|a, b| a.2 == b.2);
        v.into_iter().map(|t| t.2).collect()
    }

    /// The style a style-less request gets for `family`: its face nearest Regular upright, by the
    /// catalog's or the loaded faces' metadata. Unlike [`FontDb::face`] it reads no font files, so
    /// the closest styles of many families cost little.
    pub fn closest_style(&self, family: &str) -> Option<String> {
        let (family, _) = self.canonical(family, "");
        let family = family.as_str();
        let mut best: Option<(f32, String)> = None;
        let consider = |best: &mut Option<(f32, String)>, weight: f32, italic: bool, style: &str| {
            let d = super::build::weight_distance(weight, italic, 400.0, false);
            if best.as_ref().is_none_or(|(b, _)| d < *b) {
                *best = Some((d, style.to_string()));
            }
        };
        for f in self.read_faces().iter().filter(|f| f.family.eq_ignore_ascii_case(family)) {
            consider(&mut best, f.weight, f.italic, &f.style);
        }
        for f in self.catalog.styles(family) {
            consider(&mut best, f.weight, f.italic, &f.style);
        }
        best.map(|(_, style)| style)
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
        #[cfg(any(test, feature = "test-fonts"))]
        let _change = (!made.is_empty()).then(crate::test_fonts::font_change);
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
    /// folders, any other waits for that scan to finish, as every catalog lookup does.
    #[cfg(any(test, feature = "test-fonts"))]
    pub(crate) fn ensure_catalog(&self) {
        let _ = self.catalog.families();
    }

    /// Catalog the installed fonts on a background thread, so the first lookup by family name
    /// (opening a file, the font menus) doesn't wait for the scan. A no-op once they are
    /// cataloged, and on wasm.
    pub fn scan_in_background(&'static self) {
        // A failed spawn leaves the scan to the first lookup that needs it.
        #[cfg(not(target_arch = "wasm32"))]
        self.catalog.scan_in_background();
    }

    /// Scan the font folders again (fonts installed or removed since), cataloging the faces
    /// found (native only). Font data is loaded when a cataloged family is first resolved. Faces
    /// loaded from files the scan no longer finds are dropped. Returns the number of faces
    /// cataloged.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_system_fonts(&self) -> usize {
        // The first scan waits for nothing: a guard is made only for a database already cataloged.
        #[cfg(any(test, feature = "test-fonts"))]
        let _change = self.catalog.is_ready().then(crate::test_fonts::font_change);
        // The first scan, or a rescan once it is done (never both at once).
        let n = self.catalog.rescan();
        // Files may have been installed since: other versions are looked for again, in the catalog
        // the rescan published. A version lookup made during the scan read the previous one.
        self.version_scanned.write().unwrap_or_else(|e| e.into_inner()).clear();
        let installed: std::collections::HashSet<PathBuf> =
            self.catalog.faces().iter().flat_map(|f| self.catalog.versions(f)).filter_map(|f| f.path).collect();
        // Faces loaded from a file this scan did not catalog, as a face or a version of one (the font was removed, or its folder
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
            sys.file_searches = 0;
        }
        self.changed();
        n
    }

    /// Whether fonts were installed or removed since the last scan (a font folder it read
    /// changed, or a new one is to be read), for [`FontDb::load_system_fonts`] to catalog them.
    /// Costs a look at each folder the scan read, not a scan. `false` before the first scan and on
    /// wasm.
    pub fn installed_fonts_changed(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.catalog.installed_fonts_changed()
        }
        #[cfg(target_arch = "wasm32")]
        false
    }
}
