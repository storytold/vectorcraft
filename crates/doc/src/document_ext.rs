use super::graphic_style::GraphicStyle;
use super::limits::MAX_NEXT_ID;
use super::records::{ImageBlob, SavedSelection, SavedView};
use super::*;

impl Document {
    /// The pixels image `im` shows: its own ([`ImageObject::pixels`]) or this document's.
    pub fn image_blob<'a>(&'a self, im: &'a ImageObject) -> Option<&'a ImageBlob> {
        im.pixels.as_deref().or_else(|| self.images.get(&im.key))
    }

    /// Take the pixels the images in `n` (its opacity masks' art included) hold themselves (art
    /// made outside a document) into [`Self::images`]: under the image's key, or a free one when
    /// the key shows other pixels.
    pub fn adopt_pixels(&mut self, n: &mut Node) {
        // Only what holds such pixels is copied out of the art it shares.
        fn holds(n: &Node) -> bool {
            matches!(&n.kind, NodeKind::Image(im) if im.pixels.is_some())
                || n.extras().mask.as_ref().is_some_and(|m| holds(&m.art))
                || n.children().into_iter().flatten().any(|c| holds(c))
        }
        if let NodeKind::Image(im) = &mut n.kind {
            self.adopt_image_pixels(im);
        }
        if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut())
            && holds(&m.art)
        {
            self.adopt_pixels(Arc::make_mut(&mut m.art));
        }
        for c in n.children_mut().into_iter().flatten() {
            if holds(c) {
                self.adopt_pixels(Arc::make_mut(c));
            }
        }
    }

    pub(crate) fn adopt_image_pixels(&mut self, im: &mut ImageObject) {
        let Some(blob) = im.pixels.take() else { return };
        let mut key = im.key.clone();
        let mut i = 1;
        while self.images.get(&key).is_some_and(|b| b.bytes != blob.bytes) {
            i += 1;
            key = format!("{}-{i}", im.key);
        }
        self.images.entry(key.clone()).or_insert_with(|| (*blob).clone());
        im.key = key;
    }

    /// Saved selections as a file gives them, made safe to list and recall: at most
    /// [`SavedSelection::MAX`], each with a clean, unique name and naming objects this document
    /// has, each once. (An id it doesn't have would select whatever object takes that id later.)
    pub fn tidy_saved_selections(&mut self) {
        if self.saved_selections.is_empty() {
            return;
        }
        let mut ids = std::collections::HashSet::new();
        self.walk(|n| {
            ids.insert(n.id);
        });
        let mut kept: Vec<SavedSelection> = vec![];
        for s in std::mem::take(&mut self.saved_selections) {
            if kept.len() >= SavedSelection::MAX {
                break;
            }
            let Some(name) = SavedSelection::clean_name(&s.name) else { continue };
            if kept.iter().any(|k| k.name == name) {
                continue;
            }
            let mut seen = std::collections::HashSet::new();
            let objects = s.objects.into_iter().filter(|id| ids.contains(id) && seen.insert(*id)).collect();
            kept.push(SavedSelection { name, objects });
        }
        self.saved_selections = kept.into();
    }

    /// Saved views as a file gives them, made safe to list and go to: at most [`SavedView::MAX`],
    /// each with a clean, unique name, a finite centre and a finite, positive zoom (a rotation
    /// that isn't finite becomes 0°).
    pub fn tidy_views(&mut self) {
        if self.views.is_empty() {
            return;
        }
        let mut kept: Vec<SavedView> = vec![];
        for v in std::mem::take(&mut self.views) {
            if kept.len() >= SavedView::MAX {
                break;
            }
            let Some(name) = SavedView::clean_name(&v.name) else { continue };
            if !v.is_usable() || kept.iter().any(|k| k.name == name) {
                continue;
            }
            let rotation = if v.rotation.is_finite() { v.rotation } else { 0.0 };
            kept.push(SavedView { name, rotation, ..v });
        }
        self.views = kept;
    }

    /// Leave opacity-mask editing: drop the temporary editing layer, a working copy of art the
    /// mask already holds (the engine syncs it after every edit).
    pub fn drop_edit_modes(&mut self) {
        if let Some(me) = self.mask_edit.take() {
            self.layers.retain(|l| l.id != me.layer);
        }
    }

    /// The document as saved and exported: without the opacity-mask editing layer. Borrowed when
    /// no mask is being edited. (Pattern editing is saved: its tile layer holds unapplied edits.)
    pub fn without_edit_modes(&self) -> std::borrow::Cow<'_, Document> {
        if self.mask_edit.is_none() {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut d = self.clone();
        d.drop_edit_modes();
        std::borrow::Cow::Owned(d)
    }
}

/// Graphic styles.
impl Document {
    /// Index of the graphic style named `name`.
    pub fn graphic_style_index(&self, name: &str) -> Option<usize> {
        self.graphic_styles.iter().position(|g| g.name == name)
    }

    /// The graphic style named `name`.
    pub fn graphic_style(&self, name: &str) -> Option<&GraphicStyle> {
        self.graphic_styles.iter().find(|g| g.name == name)
    }

    /// The graphic style with id `id` (0 never matches).
    pub fn graphic_style_by_id(&self, id: u32) -> Option<&GraphicStyle> {
        self.graphic_styles.iter().find(|g| g.id != 0 && g.id == id)
    }

    /// An id no graphic style has.
    pub fn next_graphic_style_id(&self) -> u32 {
        free_id(self.graphic_styles.iter().map(|g| g.id))
    }

    /// The id of graphic style `index`, assigning one if it has none yet.
    pub fn graphic_style_id(&mut self, index: usize) -> u32 {
        if self.graphic_styles[index].id == 0 {
            self.graphic_styles[index].id = self.next_graphic_style_id();
        }
        self.graphic_styles[index].id
    }

    /// The default name of a new graphic style: the first free "Graphic Style N", N counting on
    /// from the number of styles.
    pub fn new_graphic_style_name(&self) -> String {
        (self.graphic_styles.len() + 1..).map(|i| format!("Graphic Style {i}")).find(|n| self.graphic_style(n).is_none()).unwrap_or_default()
    }

    /// `base`, or the first free "`base` N" (N from 2) when a symbol already has that name.
    pub fn unique_symbol_name(&self, base: &str) -> String {
        let taken = |n: &str| self.symbols.iter().any(|s| s.name == n);
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
    }

    /// Symbol Options › Registration for `name` (default: centre).
    pub fn symbol_registration(&self, name: &str) -> SymbolRegistration {
        self.unknown
            .get(SYMBOL_REGISTRATION)
            .and_then(|m| m.get(name))
            .and_then(|v| v.as_u64())
            .and_then(SymbolRegistration::from_u64)
            .unwrap_or_default()
    }

    /// Set or clear (when centre) the registration point stored for `name`.
    pub fn set_symbol_registration(&mut self, name: &str, reg: SymbolRegistration) {
        let m = self.unknown.entry(SYMBOL_REGISTRATION.to_string()).or_insert_with(|| serde_json::json!({}));
        let Some(o) = m.as_object_mut() else { return };
        if reg.is_center() {
            o.remove(name);
            if o.is_empty() {
                self.unknown.remove(SYMBOL_REGISTRATION);
            }
        } else {
            o.insert(name.into(), serde_json::json!(reg.0));
        }
    }

    /// Move the stored registration when a symbol is renamed.
    pub fn rename_symbol_registration(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let reg = self.symbol_registration(from);
        self.set_symbol_registration(from, SymbolRegistration::CENTER);
        self.set_symbol_registration(to, reg);
    }

    /// Symbol Options › Export Type for `name` (default: Movie Clip).
    pub fn symbol_export_type(&self, name: &str) -> SymbolExportType {
        self.unknown.get(SYMBOL_EXPORT_TYPE).and_then(|m| m.get(name)).and_then(SymbolExportType::from_value).unwrap_or_default()
    }

    /// Set or clear (when Movie Clip, the default) the export type stored for `name`.
    pub fn set_symbol_export_type(&mut self, name: &str, kind: SymbolExportType) {
        if kind.is_default() {
            self.clear_symbol_export_type(name);
            return;
        }
        let m = self.unknown.entry(SYMBOL_EXPORT_TYPE.to_string()).or_insert_with(|| serde_json::json!({}));
        let Some(o) = m.as_object_mut() else { return };
        o.insert(name.into(), serde_json::json!(kind.id()));
    }

    /// Drop any stored export type for `name` (Movie Clip default).
    pub fn clear_symbol_export_type(&mut self, name: &str) {
        let Some(m) = self.unknown.get_mut(SYMBOL_EXPORT_TYPE) else { return };
        let Some(o) = m.as_object_mut() else { return };
        o.remove(name);
        if o.is_empty() {
            self.unknown.remove(SYMBOL_EXPORT_TYPE);
        }
    }

    /// Move the stored export type when a symbol is renamed.
    pub fn rename_symbol_export_type(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let kind = self.symbol_export_type(from);
        self.clear_symbol_export_type(from);
        self.set_symbol_export_type(to, kind);
    }

    /// Symbol Options › 9-slice guides for `name`: `None` when 9-slice scaling is off (default).
    pub fn symbol_nine_slice(&self, name: &str) -> Option<SymbolNineSlice> {
        self.unknown.get(SYMBOL_NINE_SLICE).and_then(|m| m.get(name)).and_then(SymbolNineSlice::from_value)
    }

    /// Turn 9-slice scaling on with `guides`, or off with `None`, for `name`.
    pub fn set_symbol_nine_slice(&mut self, name: &str, guides: Option<SymbolNineSlice>) {
        let Some(g) = guides else {
            let Some(o) = self.unknown.get_mut(SYMBOL_NINE_SLICE).and_then(|m| m.as_object_mut()) else { return };
            o.remove(name);
            if o.is_empty() {
                self.unknown.remove(SYMBOL_NINE_SLICE);
            }
            return;
        };
        let m = self.unknown.entry(SYMBOL_NINE_SLICE.to_string()).or_insert_with(|| serde_json::json!({}));
        if let Some(o) = m.as_object_mut() {
            o.insert(name.into(), g.to_value());
        }
    }

    /// Move the stored 9-slice guides when a symbol is renamed.
    pub fn rename_symbol_nine_slice(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let g = self.symbol_nine_slice(from);
        self.set_symbol_nine_slice(from, None);
        self.set_symbol_nine_slice(to, g);
    }
}

/// An id (never 0) none of `used` has: one past the largest, or, when that is the largest there
/// is (a damaged file), the smallest one free.
pub(crate) fn free_id(used: impl Iterator<Item = u32>) -> u32 {
    let mut used: Vec<u32> = used.collect();
    if let Some(next) = used.iter().max().copied().unwrap_or(0).checked_add(1) {
        return next;
    }
    used.sort_unstable();
    used.dedup();
    // Ids 1, 2, 3… as far as they are all taken: the first that isn't follows them.
    let taken = used.iter().filter(|&&id| id != 0).zip(1u32..).take_while(|&(&id, n)| id == n).count();
    u32::try_from(taken).map_or(u32::MAX, |n| n.saturating_add(1))
}

impl Document {
    /// Make the next id allocated at least `next`: ids handed out outside the layer tree (such
    /// as an importer's opacity mask and pattern art), which [`Document::fix_next_id`] doesn't see.
    pub fn reserve_ids(&mut self, next: u64) {
        self.next_id = self.next_id.max(next.min(MAX_NEXT_ID));
    }
}
