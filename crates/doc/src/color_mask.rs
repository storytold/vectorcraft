//! Image Tools › Color Mask: colors of an image object that are hidden while its pixels stay as
//! they are.
//!
//! A masked image is an ordinary image ([`NodeKind::Image`]) with a [`ColorMask`] in its extras
//! ([`crate::node::NodeExtras::color_mask`]). The record holds the mask's settings and the key of
//! the image's pixels with the hidden colors made clear ([`ColorMask::key`]), made by the engine
//! from the pixels named by [`ColorMask::from`]. The image's own key keeps naming its own pixels,
//! so removing the mask gives them back. While the image's key is [`ColorMask::from`], it is drawn
//! and output with the masked pixels ([`Document::shown_image`]); when another key replaced it
//! (a link updated, the image cropped), it shows its own pixels until the engine makes the masked
//! pixels again.
//!
//! The concept is that of PDF's color key masking (ISO 32000-1, 8.9.6.4): samples in the chosen
//! colors are not painted and what is behind the image shows through. Which colors match, and how
//! near ones fade out, are VectorCraft's own rules.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::{Document, ImageObject, Node, NodeKind};

/// Where a color mask's colors come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskBackground {
    /// The most common color along the image's edges.
    #[default]
    Edges,
    /// The average of the image's corner pixels that are not clear.
    Corners,
    /// The colors in [`ColorMask::colors`].
    Colors,
}

/// How near a color must be to a masked one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskMatch {
    /// The RGB distance, as a percentage of the largest one.
    #[default]
    Distance,
    /// The largest difference on one of red, green and blue, as a percentage of a channel's range.
    Channels,
}

/// The colors an image object hides (see the module docs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorMask {
    /// Key into [`Document::images`]: the pixels of [`Self::from`] with the hidden colors made
    /// clear. Named `key` in the file, so the file keeps those pixels with the document.
    pub key: String,
    /// The image's own key when the masked pixels were made.
    pub from: String,
    #[serde(default)]
    pub background: MaskBackground,
    /// The hidden colors (8-bit RGB) with [`MaskBackground::Colors`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<[u8; 3]>,
    /// Only the matching pixels that reach in from an edge are hidden.
    #[serde(default)]
    pub contiguous: bool,
    #[serde(default, rename = "match")]
    pub measure: MaskMatch,
    /// How near a color must be to be hidden, 0 to 100 (see [`MaskMatch`]).
    #[serde(default)]
    pub tolerance: f64,
    /// The band past the tolerance over which pixels fade from hidden to shown, 0 to 100.
    #[serde(default)]
    pub softness: f64,
}

impl ColorMask {
    /// Whether the masked pixels were made from the pixels of `im`.
    pub fn made_for(&self, im: &ImageObject) -> bool {
        im.pixels.is_none() && self.from == im.key
    }
}

impl Document {
    /// Image `im` of node `n` as it is drawn and output. With a color mask whose masked pixels were
    /// made from the image's own pixels and are in this document ([`ColorMask::made_for`]): a copy
    /// that shows those pixels and links to no file, so an output that refers to a linked file
    /// writes the masked pixels instead. Otherwise, and for a mask that hides nothing (its masked
    /// pixels are the image's own), `im` itself.
    pub fn shown_image<'a>(&self, n: &Node, im: &'a ImageObject) -> Cow<'a, ImageObject> {
        match n.extras().color_mask.as_deref() {
            Some(m) if m.key != im.key && m.made_for(im) && self.images.contains_key(&m.key) => {
                Cow::Owned(ImageObject { key: m.key.clone(), link: None, ..im.clone() })
            }
            _ => Cow::Borrowed(im),
        }
    }

    /// The color mask of image node `n`, if it has one.
    pub fn color_mask_of(n: &Node) -> Option<&ColorMask> {
        matches!(n.kind, NodeKind::Image(_)).then(|| n.extras().color_mask.as_deref()).flatten()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_geom::Affine;

    use super::*;
    use crate::{ImageBlob, NodeId};

    fn image(key: &str) -> ImageObject {
        ImageObject { key: key.into(), width: 2, height: 2, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None }
    }

    fn mask(key: &str, from: &str) -> ColorMask {
        ColorMask {
            key: key.into(),
            from: from.into(),
            background: MaskBackground::Colors,
            colors: vec![[255, 255, 255]],
            contiguous: false,
            measure: MaskMatch::Distance,
            tolerance: 10.0,
            softness: 0.0,
        }
    }

    #[test]
    fn a_masked_image_shows_its_masked_pixels_while_they_were_made_from_its_own() {
        let mut d = Document::new(100.0, 100.0);
        d.images.insert("own".into(), ImageBlob::png(vec![1]));
        d.images.insert("masked".into(), ImageBlob::png(vec![2]));
        let mut n = Node::new(NodeId(5), NodeKind::Image(Box::new(image("own"))));
        let NodeKind::Image(im) = &n.kind else { unreachable_kind() };
        assert_eq!(d.shown_image(&n, im).key, "own", "no mask: its own pixels");
        n.extras_mut().color_mask = Some(Box::new(mask("masked", "own")));
        let NodeKind::Image(im) = &n.kind else { unreachable_kind() };
        assert_eq!(d.shown_image(&n, im).key, "masked");
        // Linked: the shown copy links to nothing, so outputs write the masked pixels.
        let mut linked = (**im).clone();
        linked.link = Some(crate::LinkInfo::new("/a.png"));
        assert_eq!(d.shown_image(&n, &linked).link, None);
        // Made from other pixels: its own until they are made again.
        assert_eq!(d.shown_image(&n, &image("other")).key, "other");
        // Masked pixels the document doesn't have: its own.
        n.extras_mut().color_mask = Some(Box::new(mask("gone", "own")));
        let NodeKind::Image(im) = &n.kind else { unreachable_kind() };
        assert_eq!(d.shown_image(&n, im).key, "own");
        // Pixels an image holds itself are drawn as they are.
        let own = ImageObject { pixels: Some(Arc::new(ImageBlob::png(vec![3]))), ..image("own") };
        n.extras_mut().color_mask = Some(Box::new(mask("masked", "own")));
        assert_eq!(d.shown_image(&n, &own).key, "own");
        // A mask that hides nothing (its masked pixels are the image's own): the image itself, so
        // a linked image stays linked in outputs.
        n.extras_mut().color_mask = Some(Box::new(mask("own", "own")));
        assert_eq!(d.shown_image(&n, &linked).link, linked.link);
    }

    #[test]
    fn a_mask_saves_and_reads_back_with_its_node() {
        let mut n = Node::new(NodeId(5), NodeKind::Image(Box::new(image("own"))));
        n.extras_mut().color_mask = Some(Box::new(mask("masked", "own")));
        let json = serde_json::to_value(&n).unwrap();
        assert_eq!(json["color_mask"]["key"], "masked", "the masked pixels' key is a `key` member: {json}");
        assert_eq!(json["color_mask"]["match"], "distance");
        let back: Node = serde_json::from_value(json).unwrap();
        assert_eq!(back, n);
        // A node without one writes none.
        let plain = Node::new(NodeId(6), NodeKind::Image(Box::new(image("own"))));
        assert!(serde_json::to_value(&plain).unwrap().get("color_mask").is_none());
    }

    fn unreachable_kind() -> ! {
        panic!("an image node")
    }
}
