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
    physical_memory().map_or(IMAGE_CACHE_MAX, budget_for)
}

/// The default budget on a machine with `memory` bytes installed: an eighth of it, at most
/// [`IMAGE_CACHE_MAX`].
#[cfg(not(target_arch = "wasm32"))]
fn budget_for(memory: u64) -> usize {
    usize::try_from(memory / 8).unwrap_or(usize::MAX).min(IMAGE_CACHE_MAX)
}

/// Installed memory in bytes (`MemTotal` in `/proc/meminfo`).
#[cfg(target_os = "linux")]
fn physical_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Installed memory in bytes (`GlobalMemoryStatusEx`).
#[cfg(windows)]
fn physical_memory() -> Option<u64> {
    winsafe::GlobalMemoryStatusEx().ok().map(|m| m.ullTotalPhys)
}

/// Installed memory in bytes, as `sysctl` prints it (`hw.memsize` on macOS, `hw.physmem` on
/// FreeBSD).
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn physical_memory() -> Option<u64> {
    let (sysctl, name) = if cfg!(target_os = "macos") { ("/usr/sbin/sysctl", "hw.memsize") } else { ("/sbin/sysctl", "hw.physmem") };
    let out = std::process::Command::new(sysctl).args(["-n", name]).output().ok().filter(|o| o.status.success())?;
    sysctl_bytes(&out.stdout)
}

/// The byte count `sysctl -n` prints for a memory size.
#[cfg(any(target_os = "macos", target_os = "freebsd", test))]
fn sysctl_bytes(out: &[u8]) -> Option<u64> {
    std::str::from_utf8(out).ok()?.trim().parse().ok()
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos", target_os = "freebsd", target_arch = "wasm32")))]
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
    /// Colorized with this display color ([`colorized`]).
    pub tint: Option<[u8; 3]>,
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
    /// How many entries and undecodable notes have been added: it changes whenever the store
    /// holds pixels, or notes an image as undecodable, that it did not before.
    added: u64,
    /// [`BlobId`]s per blob byte buffer, by address (the `Weak` keeps it from being reused).
    ids: HashMap<usize, (Weak<Vec<u8>>, BlobId)>,
    /// The content keys of images whose pixels can't be decoded ([`Undecoded::Unreadable`]), so
    /// they aren't decoded again, each with whether [`newly_undecodable`] has counted it.
    ///
    /// [`Undecoded::Unreadable`]: crate::paint::Undecoded::Unreadable
    undecodable: HashMap<Arc<str>, bool>,
}

fn pixmap_bytes(pm: &Pixmap) -> usize {
    pm.width() as usize * pm.height() as usize * 4
}

impl ImageStore {
    pub(crate) fn new(budget: usize) -> Self {
        Self { entries: HashMap::new(), bytes: 0, budget, tick: 0, added: 0, ids: HashMap::new(), undecodable: HashMap::new() }
    }

    /// How many entries and undecodable notes have been added to the store.
    pub(crate) fn added(&self) -> u64 {
        self.added
    }

    /// Whether the image with content key `content` is known not to decode.
    pub(crate) fn is_undecodable(&self, content: &str) -> bool {
        self.undecodable.contains_key(content)
    }

    /// Note that the image with content key `content` can't be decoded. Past 1024 such images the
    /// notes are dropped and those images are tried again.
    pub(crate) fn set_undecodable(&mut self, content: Arc<str>) {
        if self.undecodable.len() >= 1024 {
            self.undecodable.clear();
        }
        if !self.undecodable.contains_key(&content) {
            self.added += 1;
            self.undecodable.insert(content, false);
        }
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
        self.added += 1;
        self.entries.insert(key, Entry { pm: pm.clone(), used: tick });
        self.evict();
        pm
    }

    /// Keep `pm` as `key` as [`ImageStore::insert`] does, holding no reference to it afterwards,
    /// then drop entries over budget → whether the store holds `key`'s pixels. Pixels larger than
    /// the budget are not kept.
    pub(crate) fn keep(&mut self, key: ImageKey, pm: Arc<Pixmap>) -> bool {
        if pixmap_bytes(&pm) > self.budget {
            return false;
        }
        drop(self.insert(key.clone(), pm));
        self.evict();
        self.entries.contains_key(&key)
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

    /// Bytes of the entries someone besides the store holds (images being painted): eviction can't
    /// free them.
    pub(crate) fn pinned_bytes(&self) -> usize {
        self.entries.values().filter(|e| Arc::strong_count(&e.pm) > 1).map(|e| pixmap_bytes(&e.pm)).sum()
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

/// Keep `img`, the decoded pixels of `blob`, in the store as a renderer keeps the pixels it
/// decodes, so the first render that draws `blob` uses them instead of decoding it. Returns whether
/// `img` was kept or the store already held `blob`'s pixels. It returns false, without looking at
/// the store, for a CMYK image and for pixels a renderer would refuse to decode: more than the
/// decode allowance as RGBA, or more than fits beside the pixels being drawn now. The store then
/// drops the least recently used entries beyond its budget, as it does whenever a renderer adds
/// pixels, and returns false when that leaves `img` out: pixels larger than the budget are never
/// kept.
pub fn keep_decoded_image(blob: &ImageBlob, img: image::DynamicImage) -> bool {
    let (key, lut, _) = crate::blobs::blob_key(blob);
    if lut.is_some() || !crate::paint::within_allowance(img.width(), img.height()) {
        return false;
    }
    let kept = store().get(&key);
    let size = match kept {
        Some(pm) => (pm.width(), pm.height()),
        None => {
            let Some(pm) = crate::paint::decoded_pixmap(img) else { return false };
            let size = (pm.width(), pm.height());
            if !store().keep(key, Arc::new(pm)) {
                return false;
            }
            size
        }
    };
    remember_size(blob, size);
    true
}

/// Whether the store holds the decoded pixels of `blob` at full size.
pub fn image_cached(blob: &ImageBlob) -> bool {
    // The key first: working it out takes the store's lock too.
    let (key, _, _) = crate::blobs::blob_key(blob);
    store().peek(&key).is_some()
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

/// How many of `blobs` a renderer has found it can't decode (they draw nothing, or a crossed box
/// with [`RenderOptions::mark_undecoded_images`](crate::RenderOptions::mark_undecoded_images)) that
/// no earlier call counted: each such image is counted once in the process.
pub fn newly_undecodable<'a>(blobs: impl IntoIterator<Item = &'a ImageBlob>) -> usize {
    let mut s = store();
    if s.undecodable.values().all(|counted| *counted) {
        return 0;
    }
    let mut n = 0;
    for blob in blobs {
        let Some(id) = s.known_id(&blob.bytes) else { continue };
        if let Some(counted) = s.undecodable.get_mut(&id.content)
            && !*counted
        {
            *counted = true;
            n += 1;
        }
    }
    n
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

/// Pixels `pm` colorized with `rgb`: each pixel takes the color, as opaque as the pixel is dark
/// (255 less its Rec. 709 luma) times its alpha, so white pixels are transparent.
pub(crate) fn colorized(pm: &Pixmap, [r, g, b]: [u8; 3]) -> Pixmap {
    let mut out = pm.clone();
    for px in out.data_mut() {
        // The luma of premultiplied components is the premultiplied luma: the alpha less it is
        // the darkness times the alpha.
        let luma = 0.2126 * px.r as f32 + 0.7152 * px.g as f32 + 0.0722 * px.b as f32;
        let a = (px.a as f32 - luma).clamp(0.0, 255.0);
        let mul = |c: u8| (c as f32 * a / 255.0).round() as u8;
        *px = PremulRgba8 { r: mul(r), g: mul(g), b: mul(b), a: a.round() as u8 };
    }
    out
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
        ImageKey { content: content.into(), adjust: None, grey: false, tint: None, ink: Ink::Display, luts: (0, 0), mip: 0 }
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

    /// Pixels kept for a first frame are dropped at once when the budget has no room for them,
    /// since nothing draws them yet.
    #[test]
    fn kept_pixels_larger_than_the_budget_are_not_kept() {
        // 10×10 pixmaps: 400 bytes each.
        let mut s = ImageStore::new(300);
        assert!(!s.keep(key("big"), Arc::new(Pixmap::new(10, 10))));
        assert_eq!(s.bytes(), 0);
        assert!(s.peek(&key("big")).is_none());
        // Room for one: the older one, held by no one, goes.
        let mut s = ImageStore::new(400);
        assert!(s.keep(key("a"), Arc::new(Pixmap::new(10, 10))));
        assert!(s.keep(key("b"), Arc::new(Pixmap::new(10, 10))));
        assert!(s.peek(&key("a")).is_none() && s.peek(&key("b")).is_some());
        assert_eq!(s.bytes(), 400);
        // The one kept is held while drawn: a new one that doesn't fit beside it is dropped.
        let held = s.get(&key("b"));
        assert!(!s.keep(key("c"), Arc::new(Pixmap::new(10, 10))));
        assert!(s.peek(&key("c")).is_none());
        assert_eq!(s.bytes(), 400);
        drop(held);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn default_budget_is_an_eighth_of_installed_memory_at_most_1_gib() {
        assert_eq!(budget_for(4 << 30), 512 << 20);
        assert_eq!(budget_for(8 << 30), IMAGE_CACHE_MAX);
        assert_eq!(budget_for(64 << 30), IMAGE_CACHE_MAX);
    }

    #[test]
    #[cfg(any(target_os = "linux", windows, target_os = "macos", target_os = "freebsd"))]
    fn installed_memory_is_read() {
        assert!(physical_memory().is_some_and(|m| m >= 256 << 20), "{:?}", physical_memory());
    }

    #[test]
    fn sysctl_prints_the_memory_size_in_bytes() {
        assert_eq!(sysctl_bytes(b"17179869184\n"), Some(16 << 30));
        assert_eq!(sysctl_bytes(b""), None);
        assert_eq!(sysctl_bytes(b"unknown oid 'hw.memsize'\n"), None);
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
