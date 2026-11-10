//! Color Effects' Vector Halftone: the art sampled as the dots read it ([`sample`], which Object ›
//! Vector Halftone uses too), and the art of an object with the live effect ([`art`]), which the
//! renderer draws, and the exporters and Expand Appearance write, in its place.
//!
//! The art is kept by the object's content and what it reads from the document (its symbols,
//! patterns, fonts and colour settings, as the live cache finds them): drawing, measuring or
//! exporting an object that hasn't changed takes the dots made before instead of sampling it again.

use std::sync::{Arc, Mutex, PoisonError};

use vectorcraft_doc::{Document, LayerColor, Node, NodeId};
use vectorcraft_effects::addons::halftone::{Halftone, Samples, art_outline, dot_nodes, split};
use vectorcraft_geom::Rect;

use crate::live::Found;
use crate::{RenderOptions, Renderer, region_pixels};

/// How many halftone arts are kept, the most recently used first.
const KEPT: usize = 8;

/// A halftone art kept: the object it was made for, what that read from the document, and the art
/// (`None`: the object draws as it is).
type Kept = (Node, Found, Option<Node>);

static CACHE: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

#[cfg(test)]
thread_local! {
    /// How many times this thread sampled art for the live effect.
    pub(crate) static SAMPLED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `nodes` from `doc` drawn over `region` at `scale` pixels per point, as the dots read them: on
/// white as RGBA for a mono screen, as CMYK ink amounts for CMYK screens (`mono` false). `None`
/// when the pixels couldn't be made.
pub fn sample(doc: &Document, nodes: Vec<Node>, region: Rect, scale: f64, mono: bool, invert: bool) -> Option<Samples> {
    let mut tmp = doc.clone();
    let mut layer = Node::layer(NodeId(u64::MAX), "halftone", LayerColor::Preset(0));
    if let Some(ch) = layer.children_mut() {
        *ch = nodes.into_iter().map(Arc::new).collect();
    }
    tmp.layers = vec![Arc::new(layer)];
    let mut r = Renderer::new();
    let (w, h) = region_pixels(region, scale);
    let px = if mono {
        r.render_region(&tmp, region, scale, true).pixels
    } else {
        r.render_region_inks(&tmp, region, scale, &RenderOptions { skip_templates: true, ..Default::default() })
    };
    Samples::new(px, (w as usize, h as usize), region, scale, mono, invert)
}

/// The art of `n`, an object with a visible Vector Halftone effect, in `doc`
/// ([`vectorcraft_effects::addons::halftone::halftone_art`]): `n`'s dots as the effect's last
/// visible settings make them from `n` drawn without the effect, over its painted bounds, in a
/// group that takes `n`'s place. `None` without the effect, or when the art has no area or would
/// take too many cells: `n` then draws as it is.
pub fn art(doc: &Document, n: &Node) -> Option<Node> {
    let ht = Halftone::of(n)?;
    let found = Renderer::new().live_found(doc, &Arc::new(n.clone()));
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = cache.iter().position(|(node, was, _)| was.0 == found.0 && node == n) {
        let kept = cache.remove(at);
        let art = kept.2.clone();
        cache.insert(0, kept);
        return art;
    }
    // Sampling draws other objects that may have the effect: not under the lock.
    drop(cache);
    let art = make(doc, n, &ht);
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    cache.insert(0, (n.clone(), found, art.clone()));
    cache.truncate(KEPT);
    art
}

/// [`art`] made: `n` sampled and its dots built.
fn make(doc: &Document, n: &Node, ht: &Halftone) -> Option<Node> {
    #[cfg(test)]
    SAMPLED.with(|c| c.set(c.get() + 1));
    let (object, mut group) = split(n);
    let bounds = crate::drawn_bounds(doc, &object)?;
    let (region, scale) = ht.plan(bounds)?;
    let outline = if ht.clip { art_outline(std::slice::from_ref(&object)) } else { None };
    let samples = sample(doc, vec![object], region, scale, ht.mono, ht.invert)?;
    let (layers, _) = ht.layers(&samples, bounds, outline.as_ref());
    if let Some(ch) = group.children_mut() {
        *ch = dot_nodes(layers, ht.mono, &mut || n.id);
    }
    Some(group)
}

#[cfg(test)]
mod tests;
