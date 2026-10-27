use super::*;

/// `key`'s pixels in the shared image store; `touch` marks them used.
pub(crate) fn lookup(key: &images::ImageKey, touch: bool) -> Option<Arc<Pixmap>> {
    let mut s = images::store();
    if touch { s.get(key) } else { s.peek(key) }
}

/// The key of `blob`'s decoded pixels in the shared store, the display table a CMYK image's inks
/// show through (its generation is in the key), and the pixels' size once decoded.
pub(crate) fn blob_key(blob: &vectorcraft_doc::ImageBlob) -> (images::ImageKey, Option<Arc<ink::InkLut>>, Option<(u16, u16)>) {
    let id = images::blob_id(blob);
    let lut = id.cmyk.then(ink::display_lut);
    let key =
        images::ImageKey { content: id.content, adjust: None, grey: false, ink: Ink::Display, luts: (lut.as_ref().map_or(0, |l| l.1), 0), mip: 0 };
    (key, lut.map(|l| l.0), id.size)
}

/// The decoded pixels of `blob` (a CMYK image's inks shown through the working CMYK profile, again
/// when the colour settings change), from the shared store, and their key there. An image whose
/// pixels can't be decoded is noted in the store and not decoded again; each request for it counts
/// a failure in [`decode_failures`].
pub(crate) fn blob_pixmap(blob: &vectorcraft_doc::ImageBlob, touch: bool) -> Option<(images::ImageKey, Arc<Pixmap>)> {
    let (key, lut, size) = blob_key(blob);
    let pm = match lookup(&key, touch) {
        Some(pm) => pm,
        None => {
            if images::store().is_undecodable(&key.content) {
                paint::add_decode_failures(1);
                return None;
            }
            // Decoded outside the lock: other renderers carry on meanwhile.
            let pm = match lut.and_then(|lut| ink::cmyk_pixmap(blob, &lut)) {
                Some(pm) => pm,
                None => match paint::decode_pixmap_or_why(&blob.bytes) {
                    Ok(pm) => pm,
                    Err(why) => {
                        if why == paint::Undecoded::Unreadable {
                            images::store().set_undecodable(key.content.clone());
                        }
                        return None;
                    }
                },
            };
            images::store().insert(key.clone(), Arc::new(pm))
        }
    };
    let decoded = (pm.width(), pm.height());
    if size != Some(decoded) {
        images::remember_size(blob, decoded);
    }
    Some((key, pm))
}

/// How many times to halve image pixels of `w`×`h` drawn through `m` (unit square → device
/// pixels): [`images::mip_level`].
pub(crate) fn mip_level(m: Affine, w: u16, h: u16) -> u8 {
    images::mip_level(m * Affine::scale_non_uniform(1.0 / w.max(1) as f64, 1.0 / h.max(1) as f64), w, h)
}

/// Image pixels `pm` (stored as `key`) to draw through `m` (unit square → device pixels), shrunk
/// by halves while still at least as fine as the device pixels ([`images::mip_level`]); the
/// pixels drawn are marked used in the store.
pub(crate) fn image_mip(key: &images::ImageKey, pm: Arc<Pixmap>, m: Affine) -> Arc<Pixmap> {
    let level = mip_level(m, pm.width(), pm.height());
    let mut s = images::store();
    if level == 0 {
        let _ = s.get(key); // Marks the full pixels used (they were only looked at).
        return pm;
    }
    let at = |l: u8| images::ImageKey { mip: l, ..key.clone() };
    if let Some(p) = s.get(&at(level)) {
        return p;
    }
    // From the finest level kept below it.
    let (mut l, mut cur) = (1..level).rev().find_map(|l| s.peek(&at(l)).map(|p| (l, p))).unwrap_or((0, pm));
    drop(s);
    while l < level {
        l += 1;
        let half = Arc::new(images::half(&cur));
        cur = images::store().insert(at(l), half);
    }
    cur
}

/// Can image node `n` (`im`) clipped by `region` be painted straight into the region: visible,
/// with no transparency, effects or paint of its own, and the region inside the image's frame
/// (within a pixel), so no paint past the image's edge shows?
pub(crate) fn plain_image(n: &Node, im: &vectorcraft_doc::ImageObject, region: &BezPath) -> bool {
    if !(n.visible && n.has_default_transparency() && n.appearance.items.is_empty() && n.appearance.effects.is_empty()) {
        return false;
    }
    let det = im.xf.determinant();
    if !(det.abs() > 1e-12 && det.is_finite()) {
        return false;
    }
    let b = im.xf.inverse().transform_rect_bbox(region.bounding_box());
    b.x0 >= -1.0 && b.y0 >= -1.0 && b.x1 <= im.width as f64 + 1.0 && b.y1 <= im.height as f64 + 1.0
}

/// Opacity-mask coverage of one premultiplied pixel: luminance, with the area outside the mask
/// art black (clip) or white (no clip), optionally inverted.
pub(crate) fn mask_value(r: u8, g: u8, b: u8, a: u8, clip: bool, invert: bool) -> u8 {
    let [kr, kg, kb] = vectorcraft_color::blend::MASK_LUM;
    let mut l = (kr * r as f32 + kg * g as f32 + kb * b as f32) / 255.0;
    if !clip {
        l += 1.0 - a as f32 / 255.0;
    }
    if invert {
        l = 1.0 - l;
    }
    (l.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}
