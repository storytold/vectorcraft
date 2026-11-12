//! The layouts of a document that is opening, kept for its first frame: opening lays out every type
//! object for its bounds and baselines, and the first frame needs the same glyphs. A kept layout is
//! found by comparing the whole text object, and only while the fonts and the hyphenation exceptions
//! are the ones it was made with. Each is taken once, the frame that takes one drops the rest, a run of
//! [`KEPT_LAYOUTS_FRAMES`] frames that take none drops them all, and together they hold at most
//! [`KEPT_LAYOUTS_BUDGET`] bytes.

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::{size_of, size_of_val};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use vectorcraft_doc::color::Paint;
use vectorcraft_doc::{CharStyle, ParaStyle, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Anchor, PathData};

use crate::{FontDb, PositionedGlyph, TextLayout};

/// The most bytes of layouts kept at once; the oldest are dropped first.
const KEPT_LAYOUTS_BUDGET: usize = 128 << 20;

/// How many frames may end with kept layouts on the shelf: the frames of other documents, or of a view
/// that shows no type, take none.
const KEPT_LAYOUTS_FRAMES: u32 = 8;

/// The most layouts kept under one hash ([`text_hash`]): taking one compares it with those.
const KEPT_LAYOUTS_PER_HASH: usize = 32;

/// What a layout reads besides its text object, in the global font database. Take it before laying
/// out: a layout that loads a font ends with another stamp, and is not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutStamp {
    fonts: u64,
    system_fallback: bool,
    hyphenation: u64,
}

impl LayoutStamp {
    pub fn now() -> Self {
        let db = FontDb::global();
        Self { fonts: db.generation(), system_fallback: db.system_fallback_on(), hyphenation: crate::hyphen::exceptions_generation() }
    }
}

struct Kept {
    /// Tells this one from the others under its hash, in [`Shelf::order`].
    number: u64,
    text: TextObject,
    layout: TextLayout,
    bytes: usize,
}

#[derive(Default)]
struct Shelf {
    stamp: Option<LayoutStamp>,
    kept: HashMap<u64, Vec<Kept>>,
    /// Hash and number of each kept layout, the oldest first. Layouts already taken stay listed
    /// until they reach the front.
    order: VecDeque<(u64, u64)>,
    next: u64,
    bytes: usize,
    taken: bool,
    /// Frames that have ended since the last keep.
    frames: u32,
    /// Frames being drawn now, on any thread.
    drawing: u32,
}

impl Shelf {
    fn clear(&mut self) {
        self.kept.clear();
        self.order.clear();
        self.bytes = 0;
        self.taken = false;
        self.frames = 0;
    }

    fn remove(&mut self, hash: u64, found: impl Fn(&Kept) -> bool) -> Option<Kept> {
        let list = self.kept.get_mut(&hash)?;
        let kept = list.swap_remove(list.iter().position(found)?);
        if list.is_empty() {
            self.kept.remove(&hash);
        }
        self.bytes -= kept.bytes;
        Some(kept)
    }

    /// Keep `layout` of `text`, laid out at `stamp`, when that is still `now`. Layouts of an older
    /// stamp are dropped. A layout whose memory is not counted ([`counts_its_memory`]) and one past
    /// [`KEPT_LAYOUTS_PER_HASH`] are not kept.
    fn keep(&mut self, text: &TextObject, stamp: LayoutStamp, now: LayoutStamp, layout: TextLayout, budget: usize) {
        if stamp != now {
            return;
        }
        if self.stamp != Some(now) {
            self.clear();
            self.stamp = Some(now);
        }
        let bytes = layout_bytes(&layout) + text_bytes(text);
        if bytes > budget || !counts_its_memory(text, &layout) {
            return;
        }
        let (hash, number) = (text_hash(text), self.next);
        if self.kept.get(&hash).is_some_and(|list| list.len() >= KEPT_LAYOUTS_PER_HASH) {
            return;
        }
        while self.bytes + bytes > budget {
            let Some((hash, number)) = self.order.pop_front() else { break };
            self.remove(hash, |k| k.number == number);
        }
        self.frames = 0;
        self.next += 1;
        self.bytes += bytes;
        self.order.push_back((hash, number));
        self.kept.entry(hash).or_default().push(Kept { number, text: text.clone(), layout, bytes });
    }

    /// The layout kept for a text object equal to `text`, no longer kept after this.
    fn take(&mut self, text: &TextObject, now: LayoutStamp) -> Option<TextLayout> {
        if self.kept.is_empty() {
            return None;
        }
        if self.stamp != Some(now) {
            self.clear();
            return None;
        }
        let kept = self.remove(text_hash(text), |k| k.text == *text)?;
        if self.kept.is_empty() {
            self.order.clear();
        }
        self.taken = true;
        Some(kept.layout)
    }

    fn frame_begin(&mut self) {
        self.drawing += 1;
    }

    /// A frame ended: when no other frame is being drawn and a frame took a layout, or
    /// [`KEPT_LAYOUTS_FRAMES`] frames have ended since the last keep, the others are dropped.
    fn frame_done(&mut self) {
        self.drawing = self.drawing.saturating_sub(1);
        self.frames = self.frames.saturating_add(1);
        if self.drawing == 0 && (self.taken || self.frames >= KEPT_LAYOUTS_FRAMES) {
            self.clear();
        }
    }
}

fn text_hash(t: &TextObject) -> u64 {
    let mut h = DefaultHasher::new();
    t.runs.iter().for_each(|r| {
        r.text.hash(&mut h);
        r.style.font_family.hash(&mut h);
        r.style.font_style.hash(&mut h);
        r.style.size.to_bits().hash(&mut h);
    });
    t.xf.as_coeffs().iter().for_each(|c| c.to_bits().hash(&mut h));
    h.finish()
}

/// Does [`text_bytes`] and [`layout_bytes`] count everything `t` and `layout` hold? They do not count
/// owned file-font resources, a gradient's stops, or a glyph's bitmap or color art.
fn counts_its_memory(t: &TextObject, layout: &TextLayout) -> bool {
    let gradient = |p: &Paint| matches!(p, Paint::Gradient(_));
    t.file_fonts.is_none()
        && !t.runs.iter().any(|r| gradient(&r.style.fill) || gradient(&r.style.stroke))
        && !layout.glyphs.iter().chain(&layout.ruby).any(|g| g.image.is_some() || g.colour.is_some())
}

fn paint_bytes(p: &Paint) -> usize {
    match p {
        Paint::Solid { swatch: Some(name), .. } | Paint::Pattern { pattern: name, .. } => name.len(),
        _ => 0,
    }
}

fn path_bytes(p: &PathData) -> usize {
    size_of_val(p.subpaths.as_slice()) + p.anchor_count() * size_of::<Anchor>()
}

fn style_bytes(s: &CharStyle) -> usize {
    s.font_family.len()
        + s.font_style.len()
        + s.font_version.as_ref().map_or(0, String::len)
        + s.style_name.as_ref().map_or(0, String::len)
        + s.language.as_ref().map_or(0, String::len)
        + size_of_val(s.features.as_slice())
        + s.features.iter().map(String::len).sum::<usize>()
        + s.font_axes.keys().map(|k| size_of::<(String, f64)>() + k.len()).sum::<usize>()
        + paint_bytes(&s.fill)
        + paint_bytes(&s.stroke)
        + s.stroke_dash.as_ref().map_or(0, |d| size_of_val(d.pattern.as_slice()))
        + s.glyph_alternate.as_ref().map_or(0, |g| g.font.len() + g.version.len())
}

fn run_bytes(r: &TextRun) -> usize {
    size_of::<TextRun>() + r.text.len() + style_bytes(&r.style) + r.inline.as_ref().map_or(0, |i| i.symbol.len())
}

fn para_bytes(p: &ParaStyle) -> usize {
    size_of_val(p.tabs.as_slice()) + p.tabs.iter().map(|t| t.leader.len()).sum::<usize>() + p.style_name.as_ref().map_or(0, String::len)
}

fn text_bytes(t: &TextObject) -> usize {
    let kind = match &t.kind {
        TextKind::Point => 0,
        TextKind::Area { frame } => path_bytes(frame),
        TextKind::OnPath { path, .. } => path_bytes(path),
    };
    size_of::<TextObject>()
        + kind
        + t.runs.iter().map(run_bytes).sum::<usize>()
        + t.ruby.iter().map(|r| size_of_val(r) + r.text.len()).sum::<usize>()
        + para_bytes(&t.para)
        + t.paras.iter().map(|p| size_of_val(p) + para_bytes(p)).sum::<usize>()
        + size_of_val(t.cached_baselines.as_slice())
        + t.wrap.iter().map(|w| size_of_val(w) + path_bytes(&w.path)).sum::<usize>()
}

fn glyph_bytes(glyphs: &[PositionedGlyph]) -> usize {
    size_of_val(glyphs) + glyphs.iter().map(|g| size_of_val(g.outline.elements()) + size_of_val(g.carets.as_slice())).sum::<usize>()
}

fn layout_bytes(l: &TextLayout) -> usize {
    size_of::<TextLayout>()
        + glyph_bytes(&l.glyphs)
        + glyph_bytes(&l.ruby)
        + size_of_val(l.lines.as_slice())
        + size_of_val(l.frames.as_slice())
        + size_of_val(l.inlines.as_slice())
        + l.markers.iter().map(|m| size_of_val(m) + m.text.len()).sum::<usize>()
}

fn shelf() -> MutexGuard<'static, Shelf> {
    static SHELF: LazyLock<Mutex<Shelf>> = LazyLock::new(Mutex::default);
    SHELF.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keep `layout` of `t`, made from the global font database after `stamp` was taken, for the first
/// frame ([`take_kept_layout`]). `t` is the object as the document holds it.
pub fn keep_layout(t: &TextObject, stamp: LayoutStamp, layout: TextLayout) {
    let now = LayoutStamp::now();
    shelf().keep(t, stamp, now, layout, KEPT_LAYOUTS_BUDGET);
}

/// The layout [`keep_layout`] kept for a text object equal to `t`, when a frame is being drawn
/// ([`KeptLayoutsFrame`]) and the fonts and the hyphenation exceptions have not changed since. Taking
/// it removes it. A thumbnail, which draws outside a frame, leaves the layouts for the first frame.
pub fn take_kept_layout(t: &TextObject) -> Option<TextLayout> {
    let now = LayoutStamp::now();
    let mut shelf = shelf();
    if shelf.drawing == 0 {
        return None;
    }
    shelf.take(t, now)
}

/// A frame being drawn. When the last frame being drawn ends, the kept layouts are dropped if a frame
/// took one, or after a run of frames that took none.
pub struct KeptLayoutsFrame(());

impl KeptLayoutsFrame {
    #[must_use]
    pub fn begin() -> Self {
        shelf().frame_begin();
        Self(())
    }
}

impl Drop for KeptLayoutsFrame {
    fn drop(&mut self) {
        shelf().frame_done();
    }
}

/// Drop every kept layout (tests in other crates, to lay out again what a frame would have taken).
#[cfg(any(test, feature = "test-fonts"))]
pub(crate) fn drop_kept_layouts() {
    shelf().clear();
}

#[cfg(test)]
pub(crate) mod test_shelf {
    use super::*;

    /// A shelf of its own, so that tests do not share the process's or its font database's stamp.
    #[derive(Default)]
    pub(crate) struct TestShelf(Shelf);

    impl TestShelf {
        pub(crate) fn keep(&mut self, t: &TextObject, stamp: LayoutStamp, now: LayoutStamp, layout: TextLayout, budget: usize) {
            self.0.keep(t, stamp, now, layout, budget);
        }
        pub(crate) fn take(&mut self, t: &TextObject, now: LayoutStamp) -> Option<TextLayout> {
            self.0.take(t, now)
        }
        pub(crate) fn frame_begin(&mut self) {
            self.0.frame_begin();
        }
        pub(crate) fn frame_done(&mut self) {
            self.0.frame_done();
        }
        pub(crate) fn bytes(&self) -> usize {
            self.0.bytes
        }
    }

    pub(crate) const FRAMES: u32 = KEPT_LAYOUTS_FRAMES;
    pub(crate) const PER_HASH: usize = KEPT_LAYOUTS_PER_HASH;

    pub(crate) fn stamp(fonts: u64, system_fallback: bool, hyphenation: u64) -> LayoutStamp {
        LayoutStamp { fonts, system_fallback, hyphenation }
    }

    pub(crate) fn bytes_of(t: &TextObject, layout: &TextLayout) -> usize {
        layout_bytes(layout) + text_bytes(t)
    }
}
