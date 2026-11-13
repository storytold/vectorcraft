//! Prepared, platform-neutral requests. The desktop host alone owns native cursor state.

use super::{Cursor, rasterize};
use egui::{Context, CursorIcon, CustomCursorImage, Id, ViewportId};
use std::sync::Arc;

const CAPACITY: usize = 128;
const MAX_PADDED_SIDE: u16 = 258;

/// Opaque identity: the resolved tool cursor and logical scale, independent of display scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key(Cursor, u32);

/// The same artwork at one and two pixels per logical point, aligned without resampling.
#[derive(Clone, Debug)]
pub struct Bitmap {
    /// Straight-alpha RGBA representations; the second size/hotspot is exactly twice the first.
    pub representations: [CustomCursorImage; 2],
}

/// One canvas request, valid only for its viewport and pass.
#[derive(Clone, Debug)]
pub struct Request {
    /// Identity owned by the UI; the host need not depend on the tools crate.
    pub key: Key,
    /// Shared prepared pixels, retained independently of cache eviction.
    pub bitmap: Arc<Bitmap>,
    /// Visible fallback, also used to detect a later UI override.
    pub fallback: CursorIcon,
    /// Viewport that selected this cursor.
    pub viewport: ViewportId,
    /// Cumulative pass number when the request was made.
    pub pass: u64,
}

#[derive(Clone, Default)]
struct Cache(Vec<(Key, Option<Arc<Bitmap>>)>);

impl Cache {
    fn get(&mut self, cursor: Cursor, scale: f32) -> Option<(Key, Arc<Bitmap>)> {
        // Bound before tessellation, including malicious/non-finite preference values.
        if !scale.is_finite() || !(0.125..=4.0).contains(&scale) {
            return None;
        }
        let key = Key(cursor, scale.to_bits());
        if let Some((_, bitmap)) = self.0.iter().find(|(k, _)| *k == key) {
            return bitmap.clone().map(|bitmap| (key, bitmap));
        }
        let bitmap = prepare(cursor, scale).map(Arc::new);
        if self.0.len() >= CAPACITY {
            self.0.clear();
        }
        self.0.push((key, bitmap.clone()));
        bitmap.map(|bitmap| (key, bitmap))
    }
}

fn prepare(cursor: Cursor, scale: f32) -> Option<Bitmap> {
    align([rasterize(cursor, scale)?, rasterize(cursor, scale * 2.0)?])
}

fn valid(image: &CustomCursorImage) -> bool {
    let [w, h] = image.size;
    w > 0 && h > 0 && w <= 256 && h <= 256 && image.hotspot[0] < w && image.hotspot[1] < h && image.rgba.len() == usize::from(w) * usize::from(h) * 4
}

fn align(source: [CustomCursorImage; 2]) -> Option<Bitmap> {
    if !source.iter().all(valid) {
        return None;
    }
    let [one, two] = &source;
    let mut size = [0; 2];
    let mut hotspot = [0; 2];
    for axis in 0..2 {
        hotspot[axis] = one.hotspot[axis].max(two.hotspot[axis].div_ceil(2));
        let after = (one.size[axis] - one.hotspot[axis]).max((two.size[axis] - two.hotspot[axis]).div_ceil(2));
        size[axis] = hotspot[axis].checked_add(after)?;
        if size[axis].checked_mul(2)? > MAX_PADDED_SIDE {
            return None;
        }
    }
    Some(Bitmap { representations: [pad(one, size, hotspot, 1)?, pad(two, size, hotspot, 2)?] })
}

fn pad(source: &CustomCursorImage, size: [u16; 2], hotspot: [u16; 2], factor: u16) -> Option<CustomCursorImage> {
    let size = [size[0].checked_mul(factor)?, size[1].checked_mul(factor)?];
    let hotspot = [hotspot[0].checked_mul(factor)?, hotspot[1].checked_mul(factor)?];
    let x = usize::from(hotspot[0].checked_sub(source.hotspot[0])?);
    let y = usize::from(hotspot[1].checked_sub(source.hotspot[1])?);
    let stride = usize::from(size[0]) * 4;
    let row = usize::from(source.size[0]) * 4;
    let mut rgba = vec![0; stride.checked_mul(usize::from(size[1]))?];
    for (i, pixels) in source.rgba.chunks_exact(row).enumerate() {
        let start = (y + i) * stride + x * 4;
        rgba.get_mut(start..start + row)?.copy_from_slice(pixels);
    }
    Some(CustomCursorImage { rgba: rgba.into(), size, hotspot })
}

fn pending_id(ctx: &Context) -> Id {
    Id::unique(("native-canvas-cursor", ctx.viewport_id()))
}

/// Expire the current viewport's request at the start of every host pass.
pub fn clear(ctx: &Context) {
    let id = pending_id(ctx);
    ctx.data_mut(|data| data.remove::<Request>(id));
}

/// Consume only this viewport's request. The host validates its pass at finalization.
pub fn take(ctx: &Context) -> Option<Request> {
    let id = pending_id(ctx);
    ctx.data_mut(|data| {
        let request = data.get_temp::<Request>(id);
        data.remove::<Request>(id);
        request
    })
}

/// Publish eligible pixels and always return a visible system fallback, even without a host.
pub fn request(ctx: &Context, cursor: Cursor, fallback: CursorIcon, custom: bool, scale: f32) -> CursorIcon {
    clear(ctx);
    let fallback = if fallback == CursorIcon::None { CursorIcon::Default } else { fallback };
    if custom && ctx.input(|i| i.focused && i.pointer.hover_pos().is_some()) {
        let prepared = ctx.data_mut(|data| data.get_temp_mut_or_default::<Cache>(Id::unique("native-cursor-bitmaps")).get(cursor, scale));
        if let Some((key, bitmap)) = prepared {
            let request = Request { key, bitmap, fallback, viewport: ctx.viewport_id(), pass: ctx.cumulative_pass_nr() };
            let id = pending_id(ctx);
            ctx.data_mut(|data| data.insert_temp(id, request));
        }
    }
    fallback
}

#[cfg(test)]
mod tests;
