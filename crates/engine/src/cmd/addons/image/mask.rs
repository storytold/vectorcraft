//! Image Tools › Color Mask (`addon.image.colorMask`, `addon.image.removeColorMask`,
//! `addon.image.applyColorMask`): colors of the selected images are hidden while their pixels
//! stay as they are. The colors are matched as Remove Background matches them ([`super::clear`]);
//! the image keeps its own pixels, and the mask's settings and the masked pixels are kept with it
//! ([`vectorcraft_doc::color_mask`]), so the mask can be changed, removed, or applied to the
//! pixels later. When an image's own pixels are replaced (a link updated or relinked, the image
//! cropped or recolored), its mask is applied to the new pixels ([`refresh`]).

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::color_mask::{ColorMask, MaskBackground, MaskMatch};
use vectorcraft_doc::{Document, ImageBlob, Node, NodeId, NodeKind};
use vectorcraft_geom::Affine;

use super::super::super::paint::{PROCESS_ON_TRANSPARENCY, colorized_with_process};
use super::super::super::{CommandSpec, Result, Session, bad, cmd};
use super::{Background, DecodedImages, MAX_COLORS, Match, Settings, clear, images, targets};

/// Color Mask's command.
pub const COLOR_MASK: &str = "addon.image.colorMask";
/// Remove Color Mask's command.
pub const REMOVE_COLOR_MASK: &str = "addon.image.removeColorMask";
/// Apply Color Mask's command.
pub const APPLY_COLOR_MASK: &str = "addon.image.applyColorMask";

/// The params that set a mask; with none of them, Color Mask only reports.
const SETTINGS: [&str; 6] = ["background", "colors", "contiguous", "match", "tolerance", "softness"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.image.colorMask",
            "Color Mask…",
            ["Object", "Plug-ins", "Image Tools"],
            None,
            "{background?: \"edges\" (default: the most common color along the image's edges) | \"corners\" (the average of the image's four corner pixels that are not clear) | \"colors\" (the colors in `colors`), colors?: [color…] or one string of them split by commas or spaces (1 to 16 hex colors or black, white, red, green and blue, as addon.image.removeBackground takes them; needed with background: colors), contiguous?: bool (true: only matching pixels that reach in from an edge are hidden; false: wherever they are; true), match?: \"distance\" (default: a color's RGB distance from a hidden color) | \"channels\" (its largest difference on one of red, green and blue), tolerance?: 0..100 (% as addon.image.removeBackground measures it; a pixel this near a hidden color is hidden; 10), softness?: 0..100 (% past the tolerance over which pixels fade from hidden to shown; 10), ids?: [id…] (default: the selected images)} hide colors of each image without changing its pixels, as one undo step: the image keeps its own pixels, and the mask (these settings, and the image's pixels with the hidden colors made clear, made as Remove Background makes them) is kept with the image and saved with the document; the canvas, printing and separations, PDF, SVG, EPS, raster formats, DXF, EMF, WMF, the legacy .ai and the .ai editing data, copy and paste, and placing a document show the masked pixels, and a click on any part of the image's frame selects it (the Eyedropper, Object Mosaic, Create Gradient Mesh, Crop Image's trim and Image Trace read the image's own pixels); setting the mask again replaces it, and an image whose mask does not change is left as it is (no undo step when none changes); when the image's own pixels are replaced later (Update Link, Relink, Crop Image, a recolor, a linked file read again on open), the mask is made again for the new pixels; an object in `ids` that is not an image, a CMYK image, a linked image that shows its preview (embed or relink it first), an image too large to change, a mask that would hide every pixel, and a mask that would hide pixels of a grayscale image colorized with a process color (masked gray pixels have transparency, which only a spot color, black or white colorizes; each image is checked on the pixels its new mask shows) are refused. With none of background, colors, contiguous, match, tolerance and softness given, nothing changes and the mask settings of the first image (or the defaults when it has no mask) are returned → {changed: the images changed, ids, hidden: the pixels made fully clear in them} or {background, colors, contiguous, match, tolerance, softness, masked}",
            super::has_image,
            color_mask
        ),
        cmd!(
            "addon.image.removeColorMask",
            "Remove Color Mask",
            ["Object", "Plug-ins", "Image Tools"],
            None,
            "{ids?: [id…] (default: the selected images)} take the color mask off each image, as one undo step: the image shows its own pixels again; images without a mask are left as they are (no undo step when none has one), and an object in `ids` that is not an image is refused → {changed, ids}",
            has_color_mask,
            remove_color_mask
        ),
        cmd!(
            "addon.image.applyColorMask",
            "Apply Color Mask",
            ["Object", "Plug-ins", "Image Tools"],
            None,
            "{ids?: [id…] (default: the selected images)} make each image's color mask part of its pixels, as one undo step: the masked pixels become the image's own, embedded as a PNG with an alpha channel (a linked image is embedded), and the mask is taken off; a mask that hides nothing is only taken off (the image keeps its pixels and its link); images without a mask are left as they are (no undo step when none has one), and an object in `ids` that is not an image, and a mask made again for new pixels that would give a grayscale image colorized with a process color an alpha channel, are refused → {changed, ids}",
            has_color_mask,
            apply_color_mask
        ),
    ]
}

/// Enabled when a selected image has a color mask.
fn has_color_mask(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.objects.iter().any(|id| st.doc.node(*id).and_then(Document::color_mask_of).is_some()) {
        Ok(())
    } else {
        Err("select an image with a color mask".into())
    }
}

impl Settings {
    /// The settings a mask was made with, `None` when a file gave ones out of range.
    fn of(m: &ColorMask) -> Option<Self> {
        let percent = |v: f64| v.is_finite() && (0.0..=100.0).contains(&v);
        if !percent(m.tolerance) || !percent(m.softness) {
            return None;
        }
        let background = match m.background {
            MaskBackground::Edges => Background::Edges,
            MaskBackground::Corners => Background::Corners,
            MaskBackground::Colors if (1..=MAX_COLORS).contains(&m.colors.len()) => Background::Colors(m.colors.clone()),
            MaskBackground::Colors => return None,
        };
        let measure = match m.measure {
            MaskMatch::Distance => Match::Distance,
            MaskMatch::Channels => Match::Channels,
        };
        Some(Self { background, contiguous: m.contiguous, measure, tolerance: m.tolerance, softness: m.softness })
    }

    /// A mask with these settings whose masked pixels are `key`, made from the pixels `from`.
    fn mask(&self, key: String, from: String) -> ColorMask {
        let (background, colors) = match &self.background {
            Background::Edges => (MaskBackground::Edges, vec![]),
            Background::Corners => (MaskBackground::Corners, vec![]),
            Background::Colors(c) => (MaskBackground::Colors, c.clone()),
        };
        let measure = match self.measure {
            Match::Distance => MaskMatch::Distance,
            Match::Channels => MaskMatch::Channels,
        };
        ColorMask { key, from, background, colors, contiguous: self.contiguous, measure, tolerance: self.tolerance, softness: self.softness }
    }
}

/// The masked pixels of `blob` (`None`: no pixel is hidden, the image's own pixels show) as
/// `settings` say, with errors of command `cmd`.
fn masked(blob: &ImageBlob, settings: &Settings, kept: Option<&mut DecodedImages>, cmd: &str) -> Result<Option<(String, ImageBlob, u64)>> {
    Ok(clear(blob, settings, kept, cmd)?.map(|out| (out.blob.content_key(), out.blob, out.cleared)))
}

fn color_mask(s: &mut Session, p: &Value) -> Result<Value> {
    if !SETTINGS.iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
        return report(s, p);
    }
    let settings = Settings::read(p, COLOR_MASK)?;
    // A preview step keeps the pixels it decodes for the next step; other runs keep none.
    let mut kept = std::mem::take(&mut s.decoded_images);
    let previewing = s.previewing;
    let found = mask_changes(s, p, &settings, previewing.then_some(&mut kept));
    s.decoded_images = kept;
    let changes = found?;
    let ids: Vec<u64> = changes.iter().map(|c| c.0.0).collect();
    let hidden: u64 = changes.iter().map(|c| c.3).sum();
    if !changes.is_empty() {
        s.edit("Color Mask", |d, _| {
            for (id, mask, blob, _) in changes {
                if let Some(blob) = blob {
                    d.images.entry(mask.key.clone()).or_insert(blob);
                }
                if let Some(n) = d.node_mut(id) {
                    n.extras_mut().color_mask = Some(Box::new(mask));
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({ "changed": ids.len(), "ids": ids, "hidden": hidden }))
}

/// The images `p` names whose mask changes, each with its new mask, the masked pixels to add (when
/// the document doesn't have them) and how many pixels they hide.
type MaskChange = (NodeId, ColorMask, Option<ImageBlob>, u64);

fn mask_changes(s: &Session, p: &Value, settings: &Settings, mut kept: Option<&mut DecodedImages>) -> Result<Vec<MaskChange>> {
    let st = s.doc()?;
    // Each image's pixels once, however many objects show them.
    let mut made: HashMap<&str, (String, Option<ImageBlob>, u64)> = HashMap::new();
    let mut out = vec![];
    for (id, im, blob) in images(s, p, COLOR_MASK)? {
        let (key, pixels, hidden) = match made.get(im.key.as_str()) {
            Some(m) => m.clone(),
            None => {
                let m = match masked(blob, settings, kept.as_deref_mut(), COLOR_MASK)? {
                    Some((key, blob, hidden)) => (key, Some(blob), hidden),
                    None => (im.key.clone(), None, 0),
                };
                made.insert(&im.key, m.clone());
                m
            }
        };
        let mask = settings.mask(key, im.key.clone());
        if st.doc.node(id).and_then(Document::color_mask_of) == Some(&mask) {
            continue;
        }
        // Masked pixels that hide something have an alpha channel, which a grayscale image
        // colorized with a process color can't take. The pixels the new mask shows are checked,
        // made from the image's own pixels, not the ones an earlier mask shows.
        let shown = pixels.as_ref().or_else(|| st.doc.images.get(&mask.key));
        if st.doc.node(id).is_some_and(|n| refused_colorize(&st.doc, n, shown)) {
            return Err(bad(COLOR_MASK, PROCESS_ON_TRANSPARENCY));
        }
        let pixels = pixels.filter(|_| !st.doc.images.contains_key(&mask.key));
        out.push((id, mask, pixels, hidden));
    }
    Ok(out)
}

/// Whether masked pixels `shown`, made for image node `n`, have transparency that `n`'s colorize
/// color can't take: gray pixels with an alpha channel and a process color
/// ([`colorized_with_process`]).
fn refused_colorize(doc: &Document, n: &Node, shown: Option<&ImageBlob>) -> bool {
    shown.and_then(ImageBlob::gray_source).is_some_and(|g| g.alpha) && colorized_with_process(doc, n)
}

/// Color Mask without settings: the first image's mask settings, or the defaults. It reads them
/// for images a change would refuse too (CMYK, a linked image at its preview), so the dialog opens.
fn report(s: &Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let first = targets(s, p, COLOR_MASK)?.first().and_then(|id| st.doc.node(*id)).and_then(Document::color_mask_of).cloned();
    let m = first.as_ref();
    let hex = |c: &[u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let colors =
        m.filter(|m| !m.colors.is_empty()).map_or_else(|| "#ffffff".to_string(), |m| m.colors.iter().map(hex).collect::<Vec<_>>().join(", "));
    let name = |v: Value| v.as_str().map(str::to_string).unwrap_or_default();
    Ok(json!({
        "background": name(serde_json::to_value(m.map_or(MaskBackground::Edges, |m| m.background)).unwrap_or_default()),
        "colors": colors,
        "contiguous": m.is_none_or(|m| m.contiguous),
        "match": name(serde_json::to_value(m.map_or(MaskMatch::Distance, |m| m.measure)).unwrap_or_default()),
        "tolerance": m.map_or(10.0, |m| m.tolerance),
        "softness": m.map_or(10.0, |m| m.softness),
        "masked": m.is_some(),
    }))
}

/// The image objects `p` names ([`targets`]: `ids`, or the selected images) that have a color mask.
fn masked_images(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    Ok(targets(s, p, cmd)?.into_iter().filter(|id| st.doc.node(*id).and_then(Document::color_mask_of).is_some()).collect())
}

fn remove_color_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = masked_images(s, p, REMOVE_COLOR_MASK)?;
    if !ids.is_empty() {
        s.edit("Remove Color Mask", |d, _| {
            for id in &ids {
                if let Some(n) = d.node_mut(*id) {
                    n.extras_mut().color_mask = None;
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({ "changed": ids.len(), "ids": ids.iter().map(|id| id.0).collect::<Vec<_>>() }))
}

fn apply_color_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = masked_images(s, p, APPLY_COLOR_MASK)?;
    // Each image's masked pixels, made again first when its own pixels changed since. A mask that
    // hides nothing (its masked pixels are the image's own) is only taken off: the image keeps its
    // pixels and its link.
    let mut made = vec![];
    {
        let st = s.doc()?;
        for id in &ids {
            let Some(n) = st.doc.node(*id) else { continue };
            let (NodeKind::Image(im), Some(m)) = (&n.kind, Document::color_mask_of(n)) else { continue };
            let mask = match fresh(&st.doc, n, APPLY_COLOR_MASK) {
                Some(Ok(fresh)) => fresh,
                Some(Err(e)) => return Err(e),
                None => (m.clone(), None),
            };
            if mask.0.key == im.key {
                made.push((*id, None));
                continue;
            }
            let shown = st.doc.images.get(&mask.0.key).or(mask.1.as_ref());
            if refused_colorize(&st.doc, n, shown) {
                return Err(bad(APPLY_COLOR_MASK, PROCESS_ON_TRANSPARENCY));
            }
            let size = mask_size(shown, im.width, im.height);
            made.push((*id, Some((mask, size))));
        }
    }
    if !made.is_empty() {
        s.edit("Apply Color Mask", |d, _| {
            for (id, applied) in made {
                let Some(n) = d.node_mut(id) else { continue };
                n.extras_mut().color_mask = None;
                let Some(((mask, blob), (w, h))) = applied else { continue };
                if let Some(blob) = blob {
                    d.images.entry(mask.key.clone()).or_insert(blob);
                }
                let Some(n) = d.node_mut(id) else { continue };
                if let NodeKind::Image(im) = &mut n.kind {
                    // Masked pixels of another size than the object's keep its place and size.
                    if (w, h) != (im.width, im.height) {
                        im.xf *= Affine::scale_non_uniform(f64::from(im.width) / f64::from(w), f64::from(im.height) / f64::from(h));
                        (im.width, im.height) = (w, h);
                    }
                    im.key = mask.key;
                    im.link = None;
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({ "changed": ids.len(), "ids": ids.iter().map(|id| id.0).collect::<Vec<_>>() }))
}

/// The size in pixels of masked pixels `blob` (from its header), or the object's when it can't be
/// read.
fn mask_size(blob: Option<&ImageBlob>, width: u32, height: u32) -> (u32, u32) {
    blob.and_then(|b| image::ImageReader::new(std::io::Cursor::new(b.bytes.as_slice())).with_guessed_format().ok()?.into_dimensions().ok())
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or((width, height))
}

/// The mask of image node `n` made again for the image's own pixels, with the masked pixels when
/// `doc` doesn't have them: `None` when the mask is up to date (or `n` has none), an error when the
/// pixels can't be masked (missing, CMYK, a linked preview, too large, every pixel hidden, or
/// settings a file gave out of range).
fn fresh(doc: &Document, n: &Node, cmd: &str) -> Option<Result<(ColorMask, Option<ImageBlob>)>> {
    let (NodeKind::Image(im), Some(m)) = (&n.kind, Document::color_mask_of(n)) else { return None };
    if !stale_mask(doc, n) {
        return None;
    }
    let made = || -> Result<(ColorMask, Option<ImageBlob>)> {
        let settings = Settings::of(m).ok_or_else(|| bad(cmd, "the color mask's settings are out of range"))?;
        let blob = doc.images.get(&im.key).ok_or_else(|| bad(cmd, "the image's pixels are missing"))?;
        if im.link.is_some() && blob.is_proxy() {
            return Err(bad(cmd, "the linked image shows its preview: embed or relink it first"));
        }
        Ok(match masked(blob, &settings, None, cmd)? {
            Some((key, blob, _)) => {
                let blob = (!doc.images.contains_key(&key)).then_some(blob);
                (settings.mask(key, im.key.clone()), blob)
            }
            None => (settings.mask(im.key.clone(), im.key.clone()), None),
        })
    };
    Some(made())
}

/// After an edit from `before` (`None`: everything is new, as after a document's links are read):
/// the color masks of images in `doc`'s art ([`Document::art_roots`]: layers, symbols, graph
/// designs, patterns, the original of the pattern being edited and the art of the stored brushes
/// while it is set on the document, opacity mask art included)
/// whose own pixels were replaced are made again for the new pixels ([`Refresh::made`]). Only the
/// art that changed since `before` is looked at.
pub(crate) fn refresh(before: Option<&Document>, doc: &mut Document) {
    let old: Vec<&Arc<Node>> = before.map(|b| b.art_roots().collect()).unwrap_or_default();
    let mut pass = Refresh { doc: &*doc, made: HashMap::new(), blobs: vec![] };
    let mut fixed = vec![];
    for (i, root) in doc.art_roots().enumerate() {
        let counterpart = old.get(i).copied().filter(|o| o.id == root.id);
        if stale_in(doc, root, counterpart) {
            let mut root = root.clone();
            pass.fix(Arc::make_mut(&mut root));
            fixed.push((i, root));
        }
    }
    let blobs = pass.blobs;
    if fixed.is_empty() {
        return;
    }
    for (key, blob) in blobs {
        doc.images.entry(key).or_insert(blob);
    }
    let mut fixed = fixed.into_iter().peekable();
    for (i, root) in doc.art_roots_mut().enumerate() {
        if fixed.peek().is_some_and(|(at, _)| *at == i)
            && let Some((_, new)) = fixed.next()
        {
            *root = new;
        }
    }
}

/// Whether `n`, or anything in it (opacity-mask art included), is an image whose mask is out of
/// date ([`stale_mask`]). What is the same as in `old`, `n`'s counterpart before an edit, is not
/// looked at.
fn stale_in(doc: &Document, n: &Arc<Node>, old: Option<&Arc<Node>>) -> bool {
    if old.is_some_and(|o| Arc::ptr_eq(o, n)) {
        return false;
    }
    if stale_mask(doc, n) {
        return true;
    }
    let old_mask = old.and_then(|o| o.extras().mask.as_ref()).map(|m| &m.art);
    if n.extras().mask.as_ref().is_some_and(|m| stale_in(doc, &m.art, old_mask)) {
        return true;
    }
    let Some(children) = n.children() else { return false };
    let before = old.and_then(|o| o.children()).map_or(&[][..], Vec::as_slice);
    let mut by_id: Option<HashMap<NodeId, &Arc<Node>>> = None;
    children.iter().enumerate().any(|(i, c)| {
        let b = match before.get(i) {
            Some(b) if b.id == c.id => Some(b),
            _ => by_id.get_or_insert_with(|| before.iter().map(|b| (b.id, b)).collect()).get(&c.id).copied(),
        };
        stale_in(doc, c, b)
    })
}

/// Whether `n` is an image whose color mask was not made from its own pixels (or whose masked
/// pixels `doc` doesn't have). A mask that hides nothing (its masked pixels are the image's own)
/// is up to date while the image's key stays the same, even when its pixels are missing.
fn stale_mask(doc: &Document, n: &Node) -> bool {
    matches!((&n.kind, Document::color_mask_of(n)), (NodeKind::Image(im), Some(m))
        if im.pixels.is_none() && !(m.made_for(im) && (m.key == im.key || doc.images.contains_key(&m.key))))
}

/// One [`refresh`] pass: the masks made so far, by the image's pixels, the mask's settings and
/// whether a process color colorizes the image (an image shown by many objects is decoded once),
/// and the masked pixels to add to the document.
struct Refresh<'a> {
    doc: &'a Document,
    made: HashMap<(String, String, bool), ColorMask>,
    blobs: Vec<(String, ImageBlob)>,
}

impl Refresh<'_> {
    /// Make the masks out of date in `n` (a copy of the document's art) again.
    fn fix(&mut self, n: &mut Node) {
        if let Some(mask) = self.made(n) {
            n.extras_mut().color_mask = Some(Box::new(mask));
        }
        if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut())
            && stale_in(self.doc, &m.art, None)
        {
            self.fix(Arc::make_mut(&mut m.art));
        }
        for c in n.children_mut().into_iter().flatten() {
            if stale_in(self.doc, c, None) {
                self.fix(Arc::make_mut(c));
            }
        }
    }

    /// The mask of image node `n` made again for the image's own pixels ([`fresh`]), `None` when
    /// it is up to date. A mask that can't be made for them (every pixel hidden, CMYK pixels, a
    /// linked preview, too large, gray pixels with transparency under a process color) keeps its
    /// settings and hides nothing: the image shows its own pixels, and setting the mask again
    /// reports why.
    fn made(&mut self, n: &Node) -> Option<ColorMask> {
        let (NodeKind::Image(im), Some(m)) = (&n.kind, Document::color_mask_of(n)) else { return None };
        if !stale_mask(self.doc, n) {
            return None;
        }
        let settings = ColorMask { key: String::new(), from: String::new(), ..m.clone() };
        let at = (im.key.clone(), format!("{settings:?}"), colorized_with_process(self.doc, n));
        if let Some(made) = self.made.get(&at) {
            return Some(made.clone());
        }
        let hides_nothing = || ColorMask { key: im.key.clone(), from: im.key.clone(), ..m.clone() };
        let mask = match fresh(self.doc, n, COLOR_MASK)? {
            Ok((mask, blob)) if refused_colorize(self.doc, n, blob.as_ref().or_else(|| self.doc.images.get(&mask.key))) => hides_nothing(),
            Ok((mask, blob)) => {
                self.blobs.extend(blob.map(|b| (mask.key.clone(), b)));
                mask
            }
            Err(_) => hides_nothing(),
        };
        self.made.insert(at, mask.clone());
        Some(mask)
    }
}
