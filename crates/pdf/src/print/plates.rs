//! Separations: the inks of a job ([`print_inks`]) and the document as one ink's plate, every
//! colour (and image pixel) replaced by the grey of that ink's coverage.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, Cms, Model, ProfileKind};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_doc::inks::{Link, inks, plates, visit_node_colors};
use vectorcraft_doc::overprint::darken_overprints;
use vectorcraft_doc::{ColorMode, Document, Node, NodeKind};

use super::{DEFAULT_FREQUENCY, PrintInk, PrintSettings};

/// The inks of a separation of `doc`: the process inks, then one per spot swatch (none when
/// spots print as process), with the options of [`super::PrintOutput::inks`]; and a warning for
/// each option naming no ink.
pub fn print_inks(doc: &Document, set: &PrintSettings) -> (Vec<PrintInk>, Vec<String>) {
    let out = &set.output;
    let list: Vec<PrintInk> = plates(doc)
        .into_iter()
        .filter(|p| !(p.spot && out.spots_to_process))
        .map(|p| {
            let o = out.inks.iter().find(|i| i.name == p.name);
            PrintInk {
                print: o.is_none_or(|o| o.print),
                frequency: o.and_then(|o| o.frequency).unwrap_or(DEFAULT_FREQUENCY),
                angle: o.and_then(|o| o.angle).unwrap_or_else(|| PrintInk::default_angle(&p.name)),
                spot: p.spot,
                name: p.name,
            }
        })
        .collect();
    let warnings = out
        .inks
        .iter()
        .filter(|i| !list.iter().any(|p| p.name == i.name))
        .map(|i| {
            format!("there is no ink named `{}` to print (inks: {})", i.name, list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "))
        })
        .collect();
    (list, warnings)
}

/// The colour settings separations of `doc` convert with: the document's colour spaces
/// ([`crate::output::document_cms`]), with the printer profile as their CMYK profile when it is a
/// CMYK one.
fn separation_cms(doc: &Document, set: &PrintSettings) -> Arc<Cms> {
    let (own, _) = crate::output::document_cms(doc);
    match cms::profile(set.color.profile.trim()) {
        Some(p) if p.kind == ProfileKind::Cmyk && cms::canonical_name(&own.settings().cmyk) != p.name => {
            let settings = cms::ColorSettings { cmyk: p.name, ..own.settings().clone() };
            Cms::new(&settings).map_or(own, Arc::new)
        }
        _ => own,
    }
}

/// Separates colours into inks as the job's colour options say.
pub(crate) struct Separator<'a> {
    pub doc: &'a Document,
    cms: Arc<Cms>,
    set: &'a PrintSettings,
}

impl<'a> Separator<'a> {
    pub fn new(doc: &'a Document, set: &'a PrintSettings) -> Self {
        Self { doc, cms: separation_cms(doc, set), set }
    }

    /// How much of ink `plate` colour `c` (with swatch link `link`) prints, 0..1.
    fn ink(&self, plate: &str, c: &Color, link: Link) -> f32 {
        let color = &self.set.color;
        // Spot colours as process: only Registration keeps its link.
        let link = link.filter(|(name, _)| !self.set.output.spots_to_process || *name == REGISTRATION);
        // Without Preserve Numbers, CMYK colours are separated again from their appearance.
        let lab;
        let c = if !color.preserve_numbers && matches!(c, Color::Cmyk { .. }) {
            lab = self.cms.convert(c, Model::Lab, color.intent);
            &lab
        } else {
            c
        };
        inks(self.doc, &self.cms, c, link, color.intent).on_plate(plate)
    }

    /// Recolour `n`'s subtree as plate `plate`: the grey of each colour's coverage, unlinked.
    pub fn node(&self, plate: &str, n: &mut Node) {
        visit_node_colors(n, &mut |c, swatch, tint| {
            *c = Color::Gray { k: self.ink(plate, c, swatch.as_deref().map(|s| (s, *tint))) };
            *swatch = None;
            *tint = 1.0;
        });
    }

    /// The document as plate `plate`: art, symbols, pattern tiles and images in the grey of the
    /// ink's coverage. Overprinting fills and strokes darken, so where they have none of this
    /// ink the plate below shows through instead of being knocked out.
    pub fn plate(&self, plate: &str) -> Document {
        let mut d = self.doc.clone();
        let discard_white = d.setup.discard_white_overprint;
        let mut colorized = BTreeSet::new();
        let nodes = d.layers.iter_mut().chain(d.symbols.iter_mut().map(|s| &mut s.art)).chain(d.patterns.iter_mut().flat_map(|p| p.art.iter_mut()));
        for n in nodes {
            darken_overprints(n, discard_white);
            self.node(plate, Arc::make_mut(n));
            keep_colorized_pixels(n, &mut colorized);
        }
        // A colorized image's pixels say where its color prints, on every plate: they are kept
        // under their own key, and only the color is separated.
        let mut kept = BTreeMap::new();
        for key in colorized {
            if let Some(blob) = d.images.get(&key) {
                kept.insert(colorized_key(&key), blob.clone());
            }
        }
        for blob in d.images.values_mut() {
            let gray = blob.map_rgb(|[r, g, b]| {
                let rgb = |v: u8| v as f32 / 255.0;
                let v = (255.0 * (1.0 - self.ink(plate, &Color::rgb(rgb(r), rgb(g), rgb(b)), None))).round() as u8;
                [v; 3]
            });
            if let Some(gray) = gray {
                *blob = gray;
            }
        }
        d.images.extend(kept);
        // Every colour is a grey now: nothing to separate on the way out.
        d.color_mode = ColorMode::Rgb;
        d
    }
}

/// The key a plate keeps colorized image `key`'s own pixels under.
fn colorized_key(key: &str) -> String {
    format!("{key}\u{0}colorized")
}

/// Point the colorized images in `n`'s subtree at [`colorized_key`], collecting their keys in
/// `keys`.
fn keep_colorized_pixels(n: &mut Arc<Node>, keys: &mut BTreeSet<String>) {
    let colorized = |n: &Node| matches!(n.kind, NodeKind::Image(ref im) if im.pixels.is_none()) && n.extras().colorize.is_some();
    if !colorized(n) && n.children().is_none_or(|ch| ch.is_empty()) {
        return;
    }
    let node = Arc::make_mut(n);
    if colorized(node)
        && let NodeKind::Image(im) = &mut node.kind
    {
        keys.insert(im.key.clone());
        im.key = colorized_key(&im.key);
    }
    if let Some(ch) = node.children_mut() {
        for c in ch.iter_mut() {
            keep_colorized_pixels(c, keys);
        }
    }
}
