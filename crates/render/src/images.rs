//! Decoded images, shared by every [`Renderer`](crate::Renderer) in the process.
//!
//! The canvas, the render worker, thumbnails and panels each keep a renderer; were the decoded
//! pixels cached per renderer, a 24-megapixel photo would take 96 MB in each. Here they are kept
//! once, keyed by the image's content (not the document's key for it, which another document may
//! use for other pixels) and by what was made of them: greyscale (outline mode), recoloured by a
//! colour adjustment, separated into ink planes, or shrunk ([`RenderOptions::image_mips`](crate::RenderOptions::image_mips)).
//!
//! The store holds at most [`ImageStore::budget`] bytes, dropping the least recently used entries
//! beyond that, except ones still held elsewhere (a render context drawing them): dropping those
//! would free nothing and decode them again on the next request.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};

use vectorcraft_doc::ImageBlob;
use vello_cpu::Pixmap;
use vello_cpu::color::PremulRgba8;

use crate::ink::Ink;

/// Largest default budget of the shared image store, in bytes; less on machines with under 8 GiB
/// of memory (an eighth of it). [`set_image_cache_budget`] changes it.
pub const IMAGE_CACHE_MAX: usize = 1 << 30;

/// The default budget on the web (a tab's memory is tight).
#[cfg(target_arch = "wasm32")]
fn default_budget() -> usize {
    256 << 20
}

#[cfg(not(target_arch = "wasm32"))]
fn default_budget() -> usize {
    physical_memory().map_or(IMAGE_CACHE_MAX, |m| usize::try_from(m / 8).unwrap_or(usize::MAX).min(IMAGE_CACHE_MAX))
}

/// Installed memory in bytes, where the system says without a native call (Linux).
#[cfg(target_os = "linux")]
fn physical_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
fn physical_memory() -> Option<u64> {
    None
}

/// What an image entry holds: an image's pixels as decoded or made from them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ImageKey {
    /// The encoded image's content ([`ImageBlob::content_key`] and length).
    pub content: Arc<str>,
    /// Recoloured by the colour adjustment with this hash (see `Renderer::adjusted_image`).
    pub adjust: Option<u64>,
    /// Greyscale (outline mode).
    pub grey: bool,
    /// The ink plane painted ([`Ink::Display`]: the colours themselves).
    pub ink: Ink,
    /// Generations of the colour tables the pixels were made with (0 = none): a CMYK image's
    /// display table, and the separation table of ink planes (see `ink`).
    pub luts: (u64, u64),
    /// Halved this many times ([`half`]).
    pub mip: u8,
}

/// A blob's content key and whether it holds CMYK inks, worked out once per blob allocation, and
/// the size of its pixels once they have been decoded ([`remember_size`]).
#[derive(Clone)]
pub(crate) struct BlobId {
    pub content: Arc<str>,
    pub cmyk: bool,
    pub size: Option<(u16, u16)>,
}

struct Entry {
    pm: Arc<Pixmap>,
    /// [`ImageStore::tick`] of the last request.
    used: u64,
}

/// Decoded images within a byte budget (see the module docs).
pub(crate) struct ImageStore {
    entries: HashMap<ImageKey, Entry>,
    /// Bytes of pixels in `entries`.
    bytes: usize,
    budget: usize,
    /// Request counter: the least recently used entries go first.
    tick: u64,
    /// [`BlobId`]s per blob byte buffer, by address (the `Weak` keeps it from being reused).
    ids: HashMap<usize, (Weak<Vec<u8>>, BlobId)>,
}

fn pixmap_bytes(pm: &Pixmap) -> usize {
    pm.width() as usize * pm.height() as usize * 4
}

impl ImageStore {
    pub(crate) fn new(budget: usize) -> Self {
        Self { entries: HashMap::new(), bytes: 0, budget, tick: 0, ids: HashMap::new() }
    }

    /// The pixels of `key`, marked as just used.
    pub(crate) fn get(&mut self, key: &ImageKey) -> Option<Arc<Pixmap>> {
        self.tick += 1;
        let e = self.entries.get_mut(key)?;
        e.used = self.tick;
        Some(e.pm.clone())
    }

    /// The pixels of `key` without marking them used: a shrunk copy is drawn instead, so the full
    /// image may be dropped first.
    pub(crate) fn peek(&self, key: &ImageKey) -> Option<Arc<Pixmap>> {
        self.entries.get(key).map(|e| e.pm.clone())
    }

    /// Drop `key`'s pixels, as the budget drops the least recently used.
    #[cfg(test)]
    pub(crate) fn remove(&mut self, key: &ImageKey) {
        if let Some(e) = self.entries.remove(key) {
            self.bytes = self.bytes.saturating_sub(pixmap_bytes(&e.pm));
        }
    }

    /// Keep `pm` as `key` (or, when another renderer made it meanwhile, return theirs), then drop
    /// entries over budget.
    pub(crate) fn insert(&mut self, key: ImageKey, pm: Arc<Pixmap>) -> Arc<Pixmap> {
        self.tick += 1;
        let tick = self.tick;
        if let Some(e) = self.entries.get_mut(&key) {
            e.used = tick;
            return e.pm.clone();
        }
        self.bytes += pixmap_bytes(&pm);
        self.entries.insert(key, Entry { pm: pm.clone(), used: tick });
        self.evict();
        pm
    }

    /// Drop the least recently used entries no one else holds until within budget.
    fn evict(&mut self) {
        if self.bytes <= self.budget {
            return;
        }
        let mut free: Vec<(u64, ImageKey)> =
            self.entries.iter().filter(|(_, e)| Arc::strong_count(&e.pm) == 1).map(|(k, e)| (e.used, k.clone())).collect();
        free.sort_unstable_by_key(|(used, _)| *used);
        for (_, k) in free {
            if self.bytes <= self.budget {
                break;
            }
            if let Some(e) = self.entries.remove(&k) {
                self.bytes = self.bytes.saturating_sub(pixmap_bytes(&e.pm));
            }
        }
    }

    pub(crate) fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.evict();
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    fn known_id(&self, bytes: &Arc<Vec<u8>>) -> Option<BlobId> {
        let (w, id) = self.ids.get(&(Arc::as_ptr(bytes) as usize))?;
        std::ptr::eq(w.as_ptr(), Arc::as_ptr(bytes)).then(|| id.clone())
    }

    fn remember_id(&mut self, bytes: &Arc<Vec<u8>>, id: BlobId) {
        if self.ids.len() > 1024 {
            self.ids.retain(|_, (w, _)| w.strong_count() > 0);
        }
        self.ids.insert(Arc::as_ptr(bytes) as usize, (Arc::downgrade(bytes), id));
    }

    fn remember_size(&mut self, bytes: &Arc<Vec<u8>>, size: (u16, u16)) {
        let at = Arc::as_ptr(bytes);
        if let Some((w, id)) = self.ids.get_mut(&(at as usize))
            && std::ptr::eq(w.as_ptr(), at)
        {
            id.size = Some(size);
        }
    }
}

static STORE: LazyLock<Mutex<ImageStore>> = LazyLock::new(|| Mutex::new(ImageStore::new(default_budget())));

/// The process's image store (still usable after a panic elsewhere while it was locked: every
/// change to it is complete when made).
pub(crate) fn store() -> MutexGuard<'static, ImageStore> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Set the byte budget of the decoded images all renderers share (see [`IMAGE_CACHE_MAX`]).
pub fn set_image_cache_budget(bytes: usize) {
    store().set_budget(bytes);
}

/// Bytes of decoded images the renderers share now.
pub fn image_cache_bytes() -> usize {
    store().bytes()
}

/// `blob`'s content key and kind, hashed and sniffed once per blob allocation.
pub(crate) fn blob_id(blob: &ImageBlob) -> BlobId {
    if let Some(id) = store().known_id(&blob.bytes) {
        return id;
    }
    // Outside the lock: hashing a large image takes a while.
    let id = BlobId { content: format!("{}:{}", blob.content_key(), blob.bytes.len()).into(), cmyk: blob.is_cmyk(), size: None };
    store().remember_id(&blob.bytes, id.clone());
    id
}

/// Note that `blob`'s pixels are `size`: [`blob_id`] tells it from then on, so that the mips made
/// of them are found while the store holds the mips but not the pixels.
pub(crate) fn remember_size(blob: &ImageBlob, size: (u16, u16)) {
    store().remember_size(&blob.bytes, size);
}

/// `pm` at half the size (rounded up), each pixel the mean of the 2×2 it covers (the last row and
/// column repeat on odd sizes).
pub(crate) fn half(pm: &Pixmap) -> Pixmap {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let (ow, oh) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
    let src = pm.data();
    let at = |x: usize, y: usize| {
        src.get(y.min(h.saturating_sub(1)) * w + x.min(w.saturating_sub(1))).copied().unwrap_or(PremulRgba8 { r: 0, g: 0, b: 0, a: 0 })
    };
    let mut data = Vec::with_capacity(ow * oh);
    for y in 0..oh {
        for x in 0..ow {
            let q = [at(2 * x, 2 * y), at(2 * x + 1, 2 * y), at(2 * x, 2 * y + 1), at(2 * x + 1, 2 * y + 1)];
            let mean = |c: fn(&PremulRgba8) -> u8| ((q.iter().map(|p| u32::from(c(p))).sum::<u32>() + 2) / 4) as u8;
            data.push(PremulRgba8 { r: mean(|p| p.r), g: mean(|p| p.g), b: mean(|p| p.b), a: mean(|p| p.a) });
        }
    }
    Pixmap::from_parts(data, ow as u16, oh as u16)
}

/// How many times to halve an image drawn with `m` (image pixels → device pixels): while a pixel
/// of the halved image still covers at most one device pixel each way, at most down to one pixel.
pub(crate) fn mip_level(m: vectorcraft_geom::Affine, w: u16, h: u16) -> u8 {
    let [a, b, c, d, _, _] = m.as_coeffs();
    // The largest stretch of the linear part (its greater singular value).
    let (p, q) = (a * a + b * b + c * c + d * d, a * d - b * c);
    let s = ((p + ((p * p - 4.0 * q * q).max(0.0)).sqrt()) / 2.0).sqrt();
    if !(s.is_finite() && s > 0.0 && s <= 0.5) {
        return 0;
    }
    // Halvings until the longer side is one pixel: ⌈log₂ side⌉.
    let most = (u32::BITS - (u32::from(w.max(h).max(1)) - 1).leading_zeros()) as u8;
    ((-s.log2()).floor().clamp(0.0, 15.0) as u8).min(most)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(content: &str) -> ImageKey {
        ImageKey { content: content.into(), adjust: None, grey: false, ink: Ink::Display, luts: (0, 0), mip: 0 }
    }

    #[test]
    fn budget_drops_least_recently_used_unless_held() {
        // 10×10 pixmaps: 400 bytes each; room for two.
        let mut s = ImageStore::new(800);
        let a = s.insert(key("a"), Arc::new(Pixmap::new(10, 10)));
        drop(a);
        let held = s.insert(key("b"), Arc::new(Pixmap::new(10, 10)));
        assert!(s.get(&key("a")).is_some(), "a is now the most recent");
        let _ = s.insert(key("c"), Arc::new(Pixmap::new(10, 10)));
        assert_eq!(s.bytes(), 800);
        assert!(s.peek(&key("b")).is_some(), "b is held outside the store: kept though least recently used");
        assert!(s.peek(&key("a")).is_none(), "a went instead");
        drop(held);
        s.set_budget(400);
        assert!(s.peek(&key("b")).is_none() && s.peek(&key("c")).is_some(), "now b goes first");
        assert_eq!(s.bytes(), 400);
    }

    #[test]
    fn half_averages_and_rounds_up_odd_sizes() {
        let px = |v: u8| PremulRgba8 { r: v, g: v, b: v, a: 255 };
        let pm = Pixmap::from_parts(vec![px(0), px(100), px(200), px(40), px(60), px(80)], 3, 2);
        let h = half(&pm);
        assert_eq!((h.width(), h.height()), (2, 1));
        assert_eq!(h.data()[0].r, 50);
        assert_eq!(h.data()[1].r, 140, "the last column repeats");
    }

    #[test]
    fn mip_level_follows_the_largest_stretch() {
        use vectorcraft_geom::Affine;
        assert_eq!(mip_level(Affine::scale(1.0), 1000, 1000), 0);
        assert_eq!(mip_level(Affine::scale(0.6), 1000, 1000), 0);
        assert_eq!(mip_level(Affine::scale(0.5), 1000, 1000), 1);
        assert_eq!(mip_level(Affine::scale(0.2), 1000, 1000), 2);
        assert_eq!(mip_level(Affine::scale_non_uniform(0.1, 0.9), 1000, 1000), 0, "never blurrier than one axis needs");
        assert_eq!(mip_level(Affine::scale(0.001), 8, 8), 3, "down to one pixel at most");
    }
}
