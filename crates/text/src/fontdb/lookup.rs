#[cfg(not(target_arch = "wasm32"))]
use super::PathBuf;
use super::build::{instanced, style_distance, style_italic, style_weight};
use super::cache::Catalog;
use super::face::{FALLBACK_FAMILY, FontFace, MAX_LIVE_INSTANCES, Variation, VariationAxis};
use super::glyph_pen::{FlipPen, svg_drawn};
#[cfg(not(target_arch = "wasm32"))]
use super::names::SYSTEM_FALLBACKS;
use super::names::{norm, norm_chars, script_families};
#[cfg(not(target_arch = "wasm32"))]
use super::scan::{enumerate_faces, font_files};
use super::traits::{Alias, FontMatch, FontTraits};
use super::{
    Arc, BezPath, CANONICAL_CACHE_MAX, Color, ColourContent, DrawSettings, FontDb, GlyphId, GlyphImage, MetadataProvider, NEXT_ID, OUTLINE_CACHE_MAX,
    Ordering, Size,
};

fn has_file_fonts(n: &vectorcraft_doc::Node) -> bool {
    matches!(&n.kind, vectorcraft_doc::NodeKind::Text(t) if t.file_fonts.is_some())
        || n.extras().mask.as_ref().is_some_and(|mask| has_file_fonts(&mask.art))
        || n.children().is_some_and(|children| children.iter().any(|child| has_file_fonts(child)))
}

/// The most faces [`FontDb::postscript_faces`] gives, and the most families it loads.
const MAX_POSTSCRIPT_FACES: usize = 16;

impl FontDb {
    /// The family's own name and the style for a name given in a document: any of a family's
    /// names (`ヒラギノ角ゴシック` is Hiragino Sans), a legacy family name (`Hiragino Sans W6` with its
    /// legacy style names the W6 face), a full name (name ID 4) or a PostScript name
    /// (`HiraginoSans-W6`, as PDF and `.ai` files name fonts). Case and anything but letters and
    /// digits are ignored. An unknown name is returned unchanged.
    pub fn canonical(&self, family: &str, style: &str) -> (String, String) {
        if self.is_loaded(family) || self.read_catalog().contains_key(&family.to_ascii_lowercase()) {
            return (family.to_string(), style.to_string());
        }
        // The lock is held until the answer is kept, and [`FontDb::changed`] waits for it: an
        // answer found from the fonts before a change is cleared by that change.
        let mut cache = self.canonical_cache.lock().unwrap_or_else(|e| e.into_inner());
        let key = (family.to_string(), style.to_string());
        if let Some(answer) = cache.get(&key) {
            return answer.clone();
        }
        let answer = self.canonical_alias(family, style);
        if cache.len() >= CANONICAL_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, answer.clone());
        answer
    }

    /// Finds [`FontDb::canonical`]'s answer for a name that is neither a loaded nor an installed
    /// family among the loaded faces' and the installed fonts' other names.
    fn canonical_alias(&self, family: &str, style: &str) -> (String, String) {
        let key = norm(family);
        let mut found: Vec<Alias> =
            self.read_faces().iter().flat_map(|f| Alias::of(&f.family, &f.style, &f.keys)).filter(|(k, _)| *k == key).map(|(_, a)| a).collect();
        if let Some(v) = self.aliases.read().unwrap_or_else(|e| e.into_inner()).get(&key) {
            found.extend(v.iter().cloned());
        }
        let ns = norm(style);
        // A family and style split from a PostScript name by an importer that didn't have the font
        // (`Honoka-Mincho` → Honoka, Mincho): once it is installed, the face of that PostScript
        // name, which is its own family and style (ほのか明朝, Regular).
        if found.is_empty() && !ns.is_empty() {
            let joined = format!("{key}{ns}");
            let face = |a: &Alias| a.paired.is_none() && a.style.is_some();
            let ps =
                self.read_faces().iter().flat_map(|f| Alias::of(&f.family, &f.style, &f.keys)).find(|(k, a)| *k == joined && face(a)).map(|(_, a)| a);
            let ps = ps.or_else(|| {
                self.aliases.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&joined).and_then(|v| v.iter().find(|a| face(a)).cloned())
            });
            if let Some(a) = ps {
                return (a.family, a.style.unwrap_or_else(|| style.to_string()));
            }
        }
        // A family name keeps the style asked for; a legacy pair names its face; a full or
        // PostScript name its face whatever style is asked for.
        if let Some(a) = found.iter().find(|a| a.style.is_none()) {
            return (a.family.clone(), style.to_string());
        }
        if let Some(a) = found.iter().find(|a| a.paired.as_ref().is_some_and(|p| *p == ns) || a.paired.is_none()) {
            return (a.family.clone(), a.style.clone().unwrap_or_else(|| style.to_string()));
        }
        match found.first() {
            Some(a) => (a.family.clone(), style.to_string()),
            None => (family.to_string(), style.to_string()),
        }
    }

    /// The kind of `family` (whether it sets Japanese, its class), for font menu filters: from a
    /// loaded face, else from the system font scan. Variable when any of its faces, loaded or
    /// scanned, is: a family may install static files beside a variable one. Cheap: no font file
    /// is read, and a loaded family doesn't start the scan. A composite font takes its base font's.
    pub fn family_traits(&self, family: &str) -> FontTraits {
        if let Some((base, _)) = self.composite_base(family) {
            return self.family_traits(&base);
        }
        let loaded = {
            let faces = self.read_faces();
            let mut of_family = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family));
            of_family.next().map(|first| FontTraits { variable: first.traits.variable || of_family.any(|f| f.traits.variable), ..first.traits })
        };
        let scanned = |catalog: &Catalog| catalog.get(&family.to_ascii_lowercase()).and_then(|c| c.traits);
        match loaded {
            None => scanned(&self.read_catalog()).unwrap_or_default(),
            Some(traits) if traits.variable => traits,
            Some(traits) => {
                let variable = scanned(&self.catalog.read().unwrap_or_else(std::sync::PoisonError::into_inner)).is_some_and(|t| t.variable);
                FontTraits { variable, ..traits }
            }
        }
    }

    /// [`face`](Self::face) by any of the family's names ([`canonical`](Self::canonical)), and how
    /// it matched. A composite font matches as its base font does ([`Self::composite_base`]).
    pub fn resolve(&self, family: &str, style: &str) -> Option<(Arc<FontFace>, FontMatch)> {
        if let Some((base, base_style)) = self.composite_base(family) {
            return self.resolve(&base, &base_style);
        }
        let (family, style) = self.canonical(family, style);
        let f = self.face(&family, &style)?;
        let m = if !f.family.eq_ignore_ascii_case(&family) {
            FontMatch::Missing
        } else if norm(&f.style) == norm(&style) || f.keys.styles.contains(&norm(&style)) {
            FontMatch::Exact
        } else {
            FontMatch::Style
        };
        Some((f, m))
    }

    /// The installed face whose PostScript name is `name` (`HiraginoSans-W3`; any case), loaded
    /// from the system fonts if need be. `None` when no installed font has the name.
    pub fn find_postscript(&self, name: &str) -> Option<Arc<FontFace>> {
        let key = norm(name);
        if key.is_empty() {
            return None;
        }
        let loaded = |db: &FontDb| db.read_faces().iter().find(|f| f.keys.postscript.contains(&key)).cloned();
        if let Some(f) = loaded(self) {
            return Some(f);
        }
        self.ensure_catalog();
        let face = self.aliases.read().unwrap_or_else(|e| e.into_inner()).get(&key).and_then(|v| {
            v.iter().find(|a| a.paired.is_none() && a.style.is_some()).map(|a| (a.family.clone(), a.style.clone().unwrap_or_default()))
        });
        let (family, style) = face?;
        self.face(&family, &style)?;
        loaded(self)
    }

    /// Every installed face that [`find_postscript`](Self::find_postscript) could give for `name`
    /// (its PostScript name ignoring case and anything but letters and digits), its answer first,
    /// then the others in load order, at most 16 in all: copies of one font in
    /// other families (`SampleSerif` beside `Sample Serif`) or versions, and names that differ
    /// only in case. Each family that has one is loaded from the system fonts whole, as
    /// [`versions`](Self::versions) loads one. Empty when no installed font has the name.
    pub fn postscript_faces(&self, name: &str) -> Vec<Arc<FontFace>> {
        let Some(first) = self.find_postscript(name) else { return vec![] };
        let key = norm(name);
        let mut families: Vec<String> = vec![first.family.clone()];
        if let Some(v) = self.aliases.read().unwrap_or_else(|e| e.into_inner()).get(&key) {
            for a in v.iter().filter(|a| a.paired.is_none() && a.style.is_some()) {
                if families.len() < MAX_POSTSCRIPT_FACES && !families.iter().any(|f| f.eq_ignore_ascii_case(&a.family)) {
                    families.push(a.family.clone());
                }
            }
        }
        for family in &families {
            self.load_family_files(family);
        }
        let mut out = vec![first.clone()];
        let faces = self.read_faces();
        let others = faces.iter().filter(|f| f.id != first.id && f.keys.postscript.contains(&key));
        out.extend(others.take(MAX_POSTSCRIPT_FACES - 1).cloned());
        out
    }

    /// The (family, style) of the installed face whose PostScript name is `name` (ignoring ASCII
    /// case). Documents name fonts this way, and a family can hold hyphens
    /// ("Rounded-X-Mplus-1c-black" is Rounded-X M+ 1c, black), so the name can't be split.
    pub fn by_postscript_name(&self, name: &str) -> Option<(String, String)> {
        self.ensure_catalog();
        self.postscript.read().unwrap_or_else(|e| e.into_inner()).get(&norm(name)).cloned()
    }

    /// Load the files of the installed `family`, adding all their faces at once
    /// ([`Self::add_fonts`]). Returns whether the family is loaded now: by this call, or by another
    /// thread meanwhile.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_cataloged(&self, family: &str) -> bool {
        let mut paths: Vec<PathBuf> =
            self.read_catalog().get(&family.to_ascii_lowercase()).map(|c| c.faces.iter().map(|cf| cf.path.clone()).collect()).unwrap_or_default();
        paths.sort();
        paths.dedup();
        let files = paths.into_iter().flat_map(|p| font_files(&p).into_iter().map(move |data| (data, Some(p.clone()))));
        self.add_fonts(files) > 0 || self.is_loaded(family)
    }

    /// Resolve a family + style to a face, falling back to the closest style of the family, then to
    /// Source Sans 3 Regular. Installed system fonts are found by name whatever ran before. `None`
    /// only if no font at all is loaded (the bundled fonts failed to parse, or were left out with
    /// the `bundled-fonts` feature and none was added), in which case text has no glyphs. A
    /// composite font gives its base font's face ([`Self::composite_base`]).
    pub fn face(&self, family: &str, style: &str) -> Option<Arc<FontFace>> {
        if let Some((base, base_style)) = self.composite_base(family) {
            return self.face(&base, &base_style);
        }
        let (family, style) = self.canonical(family, style);
        let (family, style) = (family.as_str(), style.as_str());
        let found = self.find(family, style);
        if found.as_ref().is_some_and(|f| norm(&f.style) == norm(style)) {
            return found;
        }
        // The family, or this style of it, is installed but not loaded yet. Looked for again
        // whether or not this call loaded it: another thread may have just done so (then this
        // load adds nothing, but the face is there).
        #[cfg(not(target_arch = "wasm32"))]
        if found.is_none() || self.is_cataloged(family, style) {
            self.load_cataloged(family);
            if let Some(f) = self.find(family, style) {
                return Some(f);
            }
        }
        found
            .or_else(|| self.find(FALLBACK_FAMILY, style))
            .or_else(|| self.find(FALLBACK_FAMILY, "Regular"))
            .or_else(|| self.read_faces().first().cloned())
    }

    /// [`FontDb::face`] in version `version` of the family and style (a [`FontFace::version`]), when
    /// that version is installed; otherwise (or without a version) the face [`FontDb::face`] gives.
    pub fn face_version(&self, family: &str, style: &str, version: Option<&str>) -> Option<Arc<FontFace>> {
        let face = self.face(family, style)?;
        let Some(version) = version.filter(|v| *v != face.version) else { return Some(face) };
        let same =
            |f: &&Arc<FontFace>| f.family.eq_ignore_ascii_case(&face.family) && f.style.eq_ignore_ascii_case(&face.style) && f.version == version;
        if let Some(f) = self.read_faces().iter().find(same) {
            return Some(f.clone());
        }
        // Not loaded: the family's installed files may hold it (the catalog doesn't list versions).
        self.load_family_files(&face.family);
        let found = self.read_faces().iter().find(same).cloned();
        Some(found.unwrap_or(face))
    }

    /// Every installed version of `face`'s family and style, `face` itself first.
    pub fn versions(&self, face: &Arc<FontFace>) -> Vec<Arc<FontFace>> {
        self.load_family_files(&face.family);
        let mut out = vec![face.clone()];
        out.extend(
            self.read_faces()
                .iter()
                .filter(|f| f.family.eq_ignore_ascii_case(&face.family) && f.style.eq_ignore_ascii_case(&face.style) && f.id != face.id)
                .cloned(),
        );
        out
    }

    /// Load the installed files of `family` once (other versions of faces already loaded from
    /// elsewhere); a no-op on wasm, where no fonts are installed.
    fn load_family_files(&self, family: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let key = family.to_ascii_lowercase();
            if self.version_scanned.write().unwrap_or_else(|e| e.into_inner()).insert(key) {
                self.load_cataloged(family);
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = family;
    }

    /// Is `style` of `family` among the installed fonts?
    #[cfg(not(target_arch = "wasm32"))]
    fn is_cataloged(&self, family: &str, style: &str) -> bool {
        let ns = norm(style);
        self.read_catalog().get(&family.to_ascii_lowercase()).is_some_and(|c| c.faces.iter().any(|cf| norm(&cf.style) == ns))
    }

    /// The loaded face with [`FontFace::id`] `id` (the face a laid-out glyph came from), an
    /// instanced face ([`Self::instance`]) included.
    pub fn face_by_id(&self, id: u32) -> Option<Arc<FontFace>> {
        let loaded = self.read_faces().iter().find(|f| f.id == id).cloned();
        loaded.or_else(|| self.instance_by_id(id)).or_else(|| self.read_file_faces().by_id.get(&id).and_then(std::sync::Weak::upgrade))
    }

    /// The face of character style `st`: [`Self::face_version`] of its font, instanced at its
    /// Variable Font settings ([`Self::instance`]).
    pub fn style_face(&self, st: &vectorcraft_doc::CharStyle) -> Option<Arc<FontFace>> {
        if let Some(id) = st.file_font_id
            && let Some(face) = self.read_file_faces().by_id.get(&id).and_then(std::sync::Weak::upgrade)
        {
            return Some(face);
        }
        let face = self.face_version(&st.font_family, &st.font_style, st.font_version.as_deref())?;
        Some(self.instance(&face, &st.font_axes))
    }

    /// The em size characters of style `st` are set at, in `vertical` type or not: its size, or
    /// with a font height reference other than the em box ([`vectorcraft_doc::FontHeightReference`])
    /// the size at which its face ([`Self::style_face`]) has that height
    /// ([`FontFace::font_height`]). The size itself when the face has no such height.
    pub fn em_size(&self, st: &vectorcraft_doc::CharStyle, vertical: bool) -> f64 {
        let reference = st.font_height_reference;
        if reference == vectorcraft_doc::FontHeightReference::EmBox {
            return st.size;
        }
        self.style_face(st).and_then(|f| f.font_height(reference, vertical)).map_or(st.size, |h| st.size / h)
    }

    /// `t` as layout sets it: each run takes its [`Self::em_size`] and the em box as its font
    /// height reference; an inline graphic keeps its size. Explicit leading and baseline shift
    /// stay in points; Auto leading, tracking and kerning follow the em size. `t` itself when every
    /// run measures the em box.
    pub fn font_heights_resolved<'a>(&self, t: &'a vectorcraft_doc::TextObject) -> std::borrow::Cow<'a, vectorcraft_doc::TextObject> {
        use vectorcraft_doc::FontHeightReference;
        if t.runs.iter().all(|r| r.style.font_height_reference == FontHeightReference::EmBox) {
            return std::borrow::Cow::Borrowed(t);
        }
        let mut out = t.clone();
        for r in &mut out.runs {
            if r.inline.is_none() {
                r.style.size = self.em_size(&r.style, t.vertical);
            }
            r.style.font_height_reference = FontHeightReference::EmBox;
        }
        std::borrow::Cow::Owned(out)
    }

    /// `face` with the axis settings `axes` (by tag, in the axes' own units) in place of its own,
    /// each clamped to its axis's range: its outlines, metrics and advances taken there, and its
    /// file embedded instanced there. Tags the font has no axis for are left out. `face` itself
    /// when that changes no setting, and for a static face.
    pub fn instance(&self, face: &Arc<FontFace>, axes: &std::collections::BTreeMap<String, f64>) -> Arc<FontFace> {
        if axes.is_empty() {
            return face.clone();
        }
        let font_axes = face.axes();
        let mut variations = face.variations.clone();
        for (tag, value) in axes {
            let Some(axis) = font_axes.iter().find(|a| a.tag.as_slice() == tag.as_bytes()) else { continue };
            if !value.is_finite() {
                continue;
            }
            let v = (*value as f32).clamp(axis.min.min(axis.max), axis.max.max(axis.min));
            match variations.iter_mut().find(|(t, _)| *t == axis.tag) {
                Some(setting) => setting.1 = v,
                None => variations.push((axis.tag, v)),
            }
        }
        // Every axis at the setting it has in `face`: nothing to instance.
        let at = |vs: &[Variation], a: &VariationAxis| vs.iter().find(|(t, _)| *t == a.tag).map_or(a.default, |(_, v)| *v);
        if font_axes.iter().all(|a| at(&variations, a) == at(&face.variations, a)) {
            return face.clone();
        }
        variations.sort_by_key(|(t, _)| *t);
        let key = (face.id, variations.iter().map(|(t, v)| (*t, v.to_bits())).collect::<Vec<_>>());
        let mut inst = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let id = match inst.ids.get(&key) {
            Some(id) => *id,
            None => {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                inst.ids.insert(key, id);
                inst.recipes.insert(id, (face.id, variations.clone()));
                id
            }
        };
        if let Some(f) = inst.live.get(&id) {
            return f.clone();
        }
        let Some(built) = instanced(face, variations, id) else { return face.clone() };
        let built = Arc::new(built);
        if inst.live.len() >= MAX_LIVE_INSTANCES {
            inst.live.clear();
        }
        inst.live.insert(id, built.clone());
        built
    }

    /// The instanced face with id `id` ([`Self::instance`]), built again when it was dropped.
    fn instance_by_id(&self, id: u32) -> Option<Arc<FontFace>> {
        let (base, variations) = {
            let inst = self.instances.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(f) = inst.live.get(&id) {
                return Some(f.clone());
            }
            inst.recipes.get(&id).cloned()?
        };
        let base = self.read_faces().iter().find(|f| f.id == base).cloned()?;
        let built = Arc::new(instanced(&base, variations, id)?);
        let mut inst = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        if inst.live.len() >= MAX_LIVE_INSTANCES {
            inst.live.clear();
        }
        Some(inst.live.entry(id).or_insert(built).clone())
    }

    /// The available family `name` names, ignoring case and anything but letters and digits, as
    /// PostScript names write families ("MicrosoftYaHei" is Microsoft YaHei).
    pub fn find_family(&self, name: &str) -> Option<String> {
        self.family_list().iter().find(|f| norm_chars(f).eq(norm_chars(name))).cloned()
    }

    /// Is `family` available (loaded, installed on the system, or a composite font)?
    pub fn has_family(&self, family: &str) -> bool {
        if self.composite_font(family).is_some() {
            return true;
        }
        self.has_installed_family(family)
    }

    /// Is `family` available as a loaded or installed font, excluding composite fonts?
    pub fn has_installed_family(&self, family: &str) -> bool {
        let (family, _) = self.canonical(family, "");
        self.is_loaded(&family) || self.read_catalog().contains_key(&family.to_ascii_lowercase())
    }

    pub(crate) fn is_loaded(&self, family: &str) -> bool {
        self.read_faces().iter().any(|f| f.family.eq_ignore_ascii_case(family))
    }

    pub(crate) fn find(&self, family: &str, style: &str) -> Option<Arc<FontFace>> {
        let faces = self.read_faces();
        let cands: Vec<&Arc<FontFace>> = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family)).collect();
        if cands.is_empty() {
            return None;
        }
        let ns = norm(style);
        if let Some(f) = cands.iter().find(|f| norm(&f.style) == ns) {
            return Some((*f).clone());
        }
        // Another of a face's style names: in another language, or a variable font's default
        // instance's name.
        if let Some(f) = cands.iter().find(|f| f.keys.styles.contains(&ns)) {
            return Some((*f).clone());
        }
        let (tw, ti) = (style_weight(style), style_italic(style));
        cands.iter().min_by(|a, b| style_distance(a, tw, ti).total_cmp(&style_distance(b, tw, ti))).map(|f| (*f).clone())
    }

    /// [`fallback_for_style`](Self::fallback_for_style) for upright Regular text.
    pub(crate) fn fallback_for(&self, c: char, exclude: u32) -> Option<Arc<FontFace>> {
        self.fallback_for_style(c, exclude, 400.0, false)
    }

    /// First family (fallback family first, then load order) with a face that covers `c`, in that
    /// family's face nearest `weight` and `italic`; on native, system fonts are loaded lazily the
    /// first time no loaded face covers a character (#2491).
    pub(crate) fn fallback_for_style(&self, c: char, exclude: u32, weight: f32, italic: bool) -> Option<Arc<FontFace>> {
        // Thai (Lao, Khmer, Myanmar) takes a font made for it, when one is installed, rather than
        // whichever face covering it happened to load first (a display face, say).
        let prefer = script_families(c);
        if let Some(f) = self.family_fallback(c, exclude, prefer, weight, italic) {
            return Some(f);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.load_families(c, prefer)
            && let Some(f) = self.family_fallback(c, exclude, prefer, weight, italic)
        {
            return Some(f);
        }
        if let Some(f) = self.loaded_fallback(c, exclude, weight, italic) {
            return Some(f);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.system_fallback(c);
        // Another lookup may have loaded the face while this one searched. Loading no new
        // faces does not mean no face covers the character (as with `face`'s catalog lookup).
        self.loaded_fallback(c, exclude, weight, italic)
    }

    /// A face that covers `c`, for text drawn outside the canvas (the app's own UI): the loaded
    /// fonts first, then (native) an installed one, loaded on demand. Characters no font covers
    /// are remembered, so they are looked for once.
    pub fn face_covering(&self, c: char) -> Option<Arc<FontFace>> {
        self.fallback_for(c, 0)
    }

    /// The loaded face of the first of `families` that covers `c`, nearest `weight` and `italic`.
    fn family_fallback(&self, c: char, exclude: u32, families: &[&str], weight: f32, italic: bool) -> Option<Arc<FontFace>> {
        let faces = self.read_faces();
        families.iter().find_map(|fam| {
            faces
                .iter()
                .filter(|f| f.id != exclude && f.family.eq_ignore_ascii_case(fam) && f.covers(c))
                .min_by(|a, b| style_distance(a, weight, italic).total_cmp(&style_distance(b, weight, italic)))
                .cloned()
        })
    }

    /// Load the first of the installed `families` not loaded yet that covers `c` (with system
    /// fallback on). Returns whether one was loaded.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_families(&self, c: char, families: &[&str]) -> bool {
        if families.is_empty() || !self.sys.lock().unwrap_or_else(|e| e.into_inner()).enabled {
            return false;
        }
        families.iter().any(|fam| !self.is_loaded(fam) && self.load_cataloged(fam) && self.family_fallback(c, 0, &[fam], 400.0, false).is_some())
    }

    /// The first loaded family (the fallback family, then load order) with a face covering `c`,
    /// and in it the face nearest `weight` and `italic`.
    fn loaded_fallback(&self, c: char, exclude: u32, weight: f32, italic: bool) -> Option<Arc<FontFace>> {
        let faces = self.read_faces();
        let mut families: Vec<&str> = Vec::new();
        for f in faces.iter().filter(|f| f.id != exclude && f.covers(c)) {
            if !families.iter().any(|fam| fam.eq_ignore_ascii_case(&f.family)) {
                families.push(&f.family);
            }
        }
        // Stable: the fallback family first, the others in load order.
        families.sort_by_key(|fam| !fam.eq_ignore_ascii_case(FALLBACK_FAMILY));
        let fam = families.first()?;
        faces
            .iter()
            .filter(|f| f.id != exclude && f.family.eq_ignore_ascii_case(fam) && f.covers(c))
            .min_by(|a, b| style_distance(a, weight, italic).total_cmp(&style_distance(b, weight, italic)))
            .cloned()
    }

    /// Load a system font covering `c` (preferred fallback families first, then any cataloged
    /// file under 40 MB). Returns true if one was loaded. Misses are remembered.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn system_fallback(&self, c: char) -> bool {
        if c.is_control() || c.is_whitespace() {
            return false;
        }
        {
            let sys = self.sys.lock().unwrap_or_else(|e| e.into_inner());
            if !sys.enabled || sys.misses.contains(&c) {
                return false;
            }
        }
        // The first scan done before its count is read, so the first miss is remembered too.
        self.ensure_catalog();
        let scans = self.sys.lock().unwrap_or_else(|e| e.into_inner()).scans;
        let covered = |db: &FontDb| db.read_faces().iter().any(|f| f.covers(c));
        for fam in SYSTEM_FALLBACKS {
            if !self.is_loaded(fam) && self.load_cataloged(fam) && covered(self) {
                return true;
            }
        }
        {
            let mut sys = self.sys.lock().unwrap_or_else(|e| e.into_inner());
            if sys.file_searches >= super::names::MAX_FILE_SEARCHES {
                return false;
            }
            sys.file_searches += 1;
        }
        let mut paths: Vec<PathBuf> = self.read_catalog().values().flat_map(|c| c.faces.iter().map(|cf| cf.path.clone())).collect();
        paths.sort();
        paths.dedup();
        for p in paths {
            if std::fs::metadata(&p).map(|m| m.len() > 40 << 20).unwrap_or(true) {
                continue;
            }
            for data in font_files(&p) {
                let hit =
                    enumerate_faces(&data).iter().any(|(i, _)| skrifa::FontRef::from_index(&data, *i).is_ok_and(|f| f.charmap().map(c).is_some()));
                if hit && self.add_font_from(data, Some(&p)) > 0 && covered(self) {
                    return true;
                }
            }
        }
        let mut sys = self.sys.lock().unwrap_or_else(|e| e.into_inner());
        if sys.scans == scans {
            sys.misses.insert(c);
        }
        false
    }

    /// Glyph outline in font units, y-down (flipped), cached per (face, glyph).
    pub fn outline(&self, face: &FontFace, gid: u32) -> Arc<BezPath> {
        let key = (face.id, gid);
        if let Some(p) = self.outlines.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return p.clone();
        }
        let mut pen = FlipPen(BezPath::new());
        if let Some(f) = face.skrifa()
            && let Some(g) = f.outline_glyphs().get(GlyphId::new(gid))
        {
            let _ = g.draw(DrawSettings::unhinted(Size::unscaled(), face.location()), &mut pen);
        }
        let p = Arc::new(pen.0);
        let mut cache = self.outlines.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= OUTLINE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, p.clone());
        p
    }

    /// Glyph `gid`'s colour bitmap (`sbix`, `CBDT`) in `face`, cached per (face, glyph).
    pub fn glyph_image(&self, face: &FontFace, gid: u32) -> Option<Arc<GlyphImage>> {
        if !face.colour_bitmaps {
            return None;
        }
        let key = (face.id, gid);
        if let Some(i) = self.images.lock().unwrap_or_else(std::sync::PoisonError::into_inner).kept.get(&key) {
            return i.clone();
        }
        let image = face.skrifa().and_then(|f| crate::bitmap::glyph_image(&f, face.upem, gid)).map(Arc::new);
        self.images.lock().unwrap_or_else(std::sync::PoisonError::into_inner).keep(key, image.clone());
        image
    }

    /// Is glyph `gid` of face `face_id` drawn from its SVG document (it has one, and no COLR
    /// layers, which come first)? Writers that draw a font's own glyphs can draw COLR and bitmap
    /// glyphs, not always SVG ones.
    pub fn is_svg_glyph(&self, face_id: u32, gid: u32) -> bool {
        let Some(face) = self.face_by_id(face_id) else { return false };
        face.skrifa().is_some_and(|f| svg_drawn(&face, &f, gid) && crate::svg_glyph::has_document(&f, gid))
    }

    /// Glyph `gid`'s colour content in `face`, from its COLR layers or else its SVG document, in
    /// the text's `foreground` (fill) and `stroke` colours where it uses them; cached per (face,
    /// glyph), and per text colours for the glyphs that paint with them.
    pub(crate) fn glyph_colour(&self, face: &FontFace, gid: u32, foreground: Color, stroke: Option<Color>) -> Option<ColourContent> {
        if !face.colour_layers && !face.svg_glyphs {
            return None;
        }
        let f = face.skrifa()?;
        // COLR first; SVG for glyphs without COLR layers (those whose layers don't read keep their
        // outlines, as in fonts without SVG).
        let svg = svg_drawn(face, &f, gid);
        // Kept without colours, or by the text's colours it uses: the fill, and for SVG art the
        // stroke too.
        let coloured = Some((foreground.key(), stroke.filter(|_| svg).map(|c| c.key())));
        {
            let cache = self.colours.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(c) = [None, coloured].iter().find_map(|k| cache.kept.get(&(face.id, gid, *k))) {
                return c.clone();
            }
        }
        let (content, used) = if svg {
            let (art, used) = crate::svg_glyph::svg_glyph(&f, gid, face.upem, foreground, stroke);
            (art.map(ColourContent::Art), used)
        } else {
            crate::colr::colour_glyph(&f, face.location(), face.upem, gid, foreground)
        };
        let key = (face.id, gid, if used { coloured } else { None });
        self.colours.lock().unwrap_or_else(std::sync::PoisonError::into_inner).keep(key, content.clone());
        content
    }

    /// Admit a font resource, sharing its bytes. Its blob and returned face own the admission.
    /// Invalid fonts and the live-face cap return `None`; document keys are never identities.
    pub fn keep_file_font(&self, blob: &vectorcraft_doc::ImageBlob) -> Option<Arc<FontFace>> {
        self.keep_file_font_up_to(blob, super::MAX_FILE_FACES)
    }

    fn keep_file_font_up_to(&self, blob: &vectorcraft_doc::ImageBlob, limit: usize) -> Option<Arc<FontFace>> {
        // Drop this short-lived resource lock before touching registry locks.
        let cached = {
            let mut owner = blob.file_font.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if blob.mime != vectorcraft_doc::FILE_FONT_MIME {
                *owner = None;
                return None;
            }
            if owner.as_ref().is_some_and(|face| !std::ptr::eq(face.bytes(), blob.bytes.as_slice()) && face.bytes() != blob.bytes.as_slice()) {
                *owner = None;
            }
            owner.clone()
        };
        if let Some(cached) = cached
            && let Some(face) = self.read_file_faces().by_id.get(&cached.id()).and_then(std::sync::Weak::upgrade)
            && std::ptr::eq::<dyn vectorcraft_doc::FileFontFace>(face.as_ref(), cached.as_ref())
        {
            return Some(face);
        }
        let face = self.admit_file_font(blob, limit)?;
        // No registry lock is held here; only resource owners, never weak bindings, keep slots.
        *blob.file_font.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(face.clone());
        Some(face)
    }

    fn admit_file_font(&self, blob: &vectorcraft_doc::ImageBlob, limit: usize) -> Option<Arc<FontFace>> {
        let allocation = Arc::as_ptr(&blob.bytes) as usize;
        if let Some(face) = self.read_file_faces().by_blob.get(&allocation).and_then(std::sync::Weak::upgrade) {
            return Some(face);
        }
        let key = blob.content_key();
        let mut faces = self.file_faces.write().unwrap_or_else(|e| e.into_inner());
        if let Some(face) = faces
            .by_key
            .get(&key)
            .and_then(|bucket| bucket.iter().filter_map(std::sync::Weak::upgrade).find(|face| face.data() == blob.bytes.as_slice()))
        {
            return Some(face);
        }
        // Reap on admission, including dead hash buckets, so closed documents recover allowance.
        faces.by_id.retain(|_, face| face.strong_count() != 0);
        faces.by_blob.retain(|_, face| face.strong_count() != 0);
        faces.by_key.retain(|_, bucket| {
            bucket.retain(|face| face.strong_count() != 0);
            !bucket.is_empty()
        });
        if faces.by_id.len() >= limit {
            return None;
        }
        let (i, spec) = super::scan::enumerate_faces(&blob.bytes).into_iter().next()?;
        let face = Arc::new(super::build::make_face(super::face::FontBytes::Owned(blob.bytes.clone()), i, spec, None)?);
        faces.by_id.insert(face.id, Arc::downgrade(&face));
        faces.by_blob.insert(allocation, Arc::downgrade(&face));
        faces.by_key.entry(key).or_default().push(Arc::downgrade(&face));
        self.generation.fetch_add(1, super::Ordering::Relaxed);
        Some(face)
    }

    /// Bind only references whose owning document actually carries the bytes. A serialized
    /// reference without its blob cannot borrow a font another document loaded.
    /// Keep owned fallbacks bound even while an installed exact face wins, in case it is removed.
    /// The return value says whether every active reference was admitted.
    pub fn bind_file_fonts<'a>(&self, t: &mut vectorcraft_doc::TextObject, image: impl Fn(&str) -> Option<&'a vectorcraft_doc::ImageBlob>) -> bool {
        let mut resources: Vec<(String, Arc<dyn vectorcraft_doc::FileFontFace>)> = vec![];
        let mut complete = true;
        for font in t.file_fonts_in_use() {
            if !t.runs.iter().any(|r| r.style.font_family == font.family && r.style.font_style == font.style)
                || resources.iter().any(|(key, _)| *key == font.key)
            {
                continue;
            }
            let face = image(&font.key).and_then(|blob| self.keep_file_font(blob));
            if let Some(face) = face {
                resources.push((font.key.clone(), face));
            } else {
                complete = false;
            }
        }
        if let Some(fonts) = t.file_fonts.as_mut()
            && fonts.bind(resources)
        {
            t.cached_bounds = None;
            t.cached_baselines.clear();
            self.generation.fetch_add(1, super::Ordering::Relaxed);
        }
        complete
    }

    /// Bind text throughout a resource tree, including opacity masks, reporting rejected fonts.
    pub fn bind_node_file_fonts<'a>(
        &self,
        n: &mut vectorcraft_doc::Node,
        image: &impl Fn(&str) -> Option<&'a vectorcraft_doc::ImageBlob>,
    ) -> Vec<String> {
        fn visit<'a>(
            db: &FontDb,
            n: &mut vectorcraft_doc::Node,
            image: &impl Fn(&str) -> Option<&'a vectorcraft_doc::ImageBlob>,
            warnings: &mut Vec<String>,
        ) {
            if let vectorcraft_doc::NodeKind::Text(t) = &mut n.kind
                && !db.bind_file_fonts(t, image)
            {
                for font in t.file_fonts_in_use() {
                    if t.runs.iter().any(|r| r.style.font_family == font.family && r.style.font_style == font.style)
                        && t.file_fonts.as_ref().is_none_or(|fonts| fonts.face(&font.key).is_none())
                        && db.resolve(&font.family, &font.style).is_none_or(|(_, m)| m != super::traits::FontMatch::Exact)
                    {
                        let note = format!(
                            "the file font {} {} could not be used (missing, invalid or live-font limit reached); its text uses a substitute",
                            font.family, font.style
                        );
                        if !warnings.contains(&note) {
                            warnings.push(note);
                        }
                    }
                }
            }
            if let Some(mask) = n.extras_if_any().and_then(|e| e.mask.as_mut())
                && has_file_fonts(&mask.art)
            {
                visit(db, Arc::make_mut(&mut mask.art), image, warnings);
            }
            for child in n.children_mut().into_iter().flatten().filter(|n| has_file_fonts(n)) {
                visit(db, Arc::make_mut(child), image, warnings);
            }
        }
        let mut warnings = vec![];
        visit(self, n, image, &mut warnings);
        warnings
    }

    /// Establish document ownership before layout; unrelated blobs never enter the font cache.
    pub fn bind_document_file_fonts(&self, doc: &mut vectorcraft_doc::Document) -> Vec<String> {
        let vectorcraft_doc::Document { layers, symbols, graph_designs, patterns, pattern_edit, images, .. } = doc;
        let trees = layers.iter_mut().chain(symbols.iter_mut().map(|s| &mut s.art)).chain(graph_designs.iter_mut().map(|g| &mut g.art));
        let patterns = patterns.iter_mut().chain(pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()));
        let mut warnings = vec![];
        for root in trees.chain(patterns.flat_map(|p| &mut p.art)).filter(|n| has_file_fonts(n)) {
            for note in self.bind_node_file_fonts(Arc::make_mut(root), &|key| images.get(key)) {
                if !warnings.contains(&note) {
                    warnings.push(note);
                }
            }
        }
        warnings
    }

    /// Is `id` a live file-font resource?
    pub fn is_file_font(&self, id: u32) -> bool {
        self.read_file_faces().by_id.get(&id).is_some_and(|face| face.strong_count() != 0)
    }

    pub(super) fn read_file_faces(&self) -> std::sync::RwLockReadGuard<'_, super::FileFaces> {
        self.file_faces.read().unwrap_or_else(|e| e.into_inner())
    }

    /// For each run of `t`, the font a file carried that draws it ([`vectorcraft_doc::FileFonts`]):
    /// while its text is the file's, the font the run names isn't available as named (missing, or
    /// another style of its family stands in) and the font kept for that family and style is
    /// bound to its owning blob. `None` for runs drawn in their named font or substitute;
    /// empty when none is drawn in such a font.
    pub fn file_faces(&self, t: &vectorcraft_doc::TextObject) -> Vec<Option<Arc<FontFace>>> {
        if t.file_fonts.is_none() {
            return vec![];
        }
        let fonts = t.file_fonts_in_use();
        if fonts.is_empty() {
            return vec![];
        }
        // Each family and style's answer once: a line has a run per character it spaces apart.
        let mut known: Vec<(&str, &str, Option<Arc<FontFace>>)> = vec![];
        let mut out = Vec::with_capacity(t.runs.len());
        for r in &t.runs {
            let (family, style) = (r.style.font_family.as_str(), r.style.font_style.as_str());
            let face = match known.iter().find(|(f, s, _)| *f == family && *s == style) {
                Some((.., face)) => face.clone(),
                None => {
                    let face = fonts
                        .iter()
                        .find(|f| f.family == family && f.style == style)
                        .filter(|_| self.resolve(family, style).is_none_or(|(_, m)| m != super::traits::FontMatch::Exact))
                        .and_then(|f| t.file_fonts.as_ref()?.face(&f.key))
                        .and_then(|face| self.read_file_faces().by_id.get(&face.id()).and_then(std::sync::Weak::upgrade));
                    known.push((family, style, face.clone()));
                    face
                }
            };
            out.push(face);
        }
        if out.iter().all(Option::is_none) { vec![] } else { out }
    }
}

#[cfg(test)]
#[path = "file_fonts_tests.rs"]
mod file_fonts_tests;
