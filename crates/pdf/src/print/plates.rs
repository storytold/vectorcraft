//! Separations: the inks of a job ([`print_inks`]) and the document as one ink's plate, every
//! colour (and image pixel) replaced by the grey of that ink's coverage. An image placed from a
//! DCS file prints its own plates ([`vectorcraft_doc::separated`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use vectorcraft_color::cms::{self, Cms, Model, ProfileKind};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{BlendMode, Color, GradientKind, Paint};
use vectorcraft_doc::inks::{Link, StopPlates, inks, plates, spot_color, stop_inks, visit_node_colors};
use vectorcraft_doc::overprint::{darken_overprints, is_black_ink, is_white};
use vectorcraft_doc::{AppearanceItem, ColorMode, Document, ImageBlob, Node, NodeKind};

use super::{DEFAULT_FREQUENCY, PrintInk, PrintSettings};

/// The inks of a separation of `doc`: the process inks, then one per spot swatch (none when
/// spots print as process), with the options of [`super::PrintOutput::inks`]; and a warning
/// naming the options that name no ink.
pub fn print_inks(doc: &Document, set: &PrintSettings) -> (Vec<PrintInk>, Vec<String>) {
    let out = &set.output;
    // Options by ink name, the first for a name winning: looked up, not searched for each ink.
    let mut options = std::collections::HashMap::new();
    for o in &out.inks {
        options.entry(o.name.as_str()).or_insert(o);
    }
    let list: Vec<PrintInk> = plates(doc)
        .into_iter()
        .filter(|p| !(p.spot && out.spots_to_process))
        .map(|p| {
            let o = options.get(p.name.as_str()).copied();
            PrintInk {
                print: o.is_none_or(|o| o.print),
                frequency: o.and_then(|o| o.frequency).unwrap_or(DEFAULT_FREQUENCY),
                angle: o.and_then(|o| o.angle).unwrap_or_else(|| PrintInk::default_angle(&p.name)),
                spot: p.spot,
                name: p.name,
            }
        })
        .collect();
    // One warning for all the options naming no ink, each list cut short: the options and the
    // inks both come from the file.
    let names: BTreeSet<&str> = list.iter().map(|p| p.name.as_str()).collect();
    let unknown: Vec<&str> = out.inks.iter().map(|i| i.name.as_str()).filter(|n| !names.contains(n)).collect();
    let warnings = if unknown.is_empty() {
        vec![]
    } else {
        let few = |all: Vec<&str>| {
            let more = all.len().saturating_sub(MAX_NAMED);
            let shown = all.into_iter().take(MAX_NAMED).map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
            if more > 0 { format!("{shown} and {more} more") } else { shown }
        };
        let inks = few(list.iter().map(|p| p.name.as_str()).collect());
        vec![format!("there is no ink named {} to print (inks: {inks})", few(unknown))]
    };
    (list, warnings)
}

/// The most names a warning of [`print_inks`] lists.
const MAX_NAMED: usize = 10;

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
    /// The plates are written as PostScript, which has no blending modes.
    postscript: bool,
}

impl<'a> Separator<'a> {
    pub fn new(doc: &'a Document, set: &'a PrintSettings, postscript: bool) -> Self {
        Self { doc, cms: separation_cms(doc, set), set, postscript }
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

    /// [`Self::ink`] of a colour that prints process plates `plates` with its spot tint
    /// ([`stop_inks`]): the spot plate prints the tint and the process plates print `plates`. With
    /// spots as process, each process plate prints `plates` layered with the spot tint's process
    /// equivalent, as a separated image's plates layer ([`vectorcraft_doc::separated::Separated::coverage_as`]).
    fn stop_ink(&self, plate: &str, c: &Color, link: Link, plates: StopPlates) -> f32 {
        let own = stop_inks(self.doc, &self.cms, c, link, plates, self.set.color.intent);
        let (Some(_), Some((name, tint))) = (plates, own.spot.as_ref()) else {
            return self.ink(plate, c, link);
        };
        if !self.set.output.spots_to_process {
            return own.on_plate(plate);
        }
        let Some(i) = cms::PROCESS_PLATES.iter().position(|p| *p == plate) else {
            return 0.0;
        };
        let spot = spot_color(self.doc, name).map_or(0.0, |s| self.ink(plate, &s.tinted(*tint), None));
        1.0 - (1.0 - own.cmyk[i]) * (1.0 - spot)
    }

    /// Recolour `n`'s subtree as plate `plate`: the grey of each colour's coverage, unlinked and
    /// without process plates.
    pub fn node(&self, plate: &str, n: &mut Node) {
        visit_node_colors(n, &mut |c, swatch, tint, plates| {
            *c = Color::Gray { k: self.stop_ink(plate, c, swatch.as_deref().map(|s| (s, *tint)), *plates) };
            *swatch = None;
            *tint = 1.0;
            *plates = None;
        });
    }

    /// Whether paint `p` puts none of ink `plate` down: a solid color, a gradient whose every stop
    /// does, or a freeform gradient with color points whose every point does (points carry no
    /// swatch link). Any ink above zero puts ink down, except on PostScript plates for a color that
    /// counts as 100% black ([`vectorcraft_doc::overprint::is_black_ink`], as
    /// [`vectorcraft_doc::overprint::OverprintBlack::default`] judges it) and is not linked to a
    /// spot swatch, to Registration or to a swatch the document lacks: up to the half a percent of
    /// another ink such a black may carry counts as none.
    fn no_ink(&self, plate: &str, p: &Paint) -> bool {
        let process = |swatch: Option<&str>| swatch.is_none_or(|n| n != REGISTRATION && self.doc.swatch(n).is_some_and(|w| !w.spot));
        let none = |c: &Color, process: bool, ink: f32| ink <= 0.0 || (self.postscript && process && ink <= 0.005 && is_black_ink(c, 1.0, false));
        match p {
            Paint::Solid { color, swatch, tint } => {
                none(color, process(swatch.as_deref()), self.ink(plate, color, swatch.as_deref().map(|s| (s, *tint))))
            }
            Paint::Gradient(g) => match g.freeform.as_ref().filter(|f| g.gradient.kind == GradientKind::Freeform && !f.points.is_empty()) {
                Some(f) => f.points.iter().all(|pt| none(&pt.color, true, self.ink(plate, &pt.color, None))),
                None => {
                    !g.gradient.stops.is_empty()
                        && g.gradient.stops.iter().all(|s| {
                            let ink = self.stop_ink(plate, &s.color, s.swatch.as_deref().map(|n| (n, s.tint)), s.plates);
                            none(&s.color, s.plates.is_none() && process(s.swatch.as_deref()), ink)
                        })
                }
            },
            _ => false,
        }
    }

    /// Overprinting fills and strokes of `a`'s subtree with a Normal blending mode, and
    /// overprinting character fills and strokes, that put none of ink `plate` down leave its plate
    /// as it is: they are drawn as nothing on it (white ones with `discard_white` aside, which
    /// knock out). A writer without blending modes (PostScript) then keeps the ink below them,
    /// where it would draw their Darken as a knockout. One that puts any ink down is drawn, and on
    /// a PostScript plate its ink replaces the ink below, as a nonzero component of an overprinting
    /// color replaces the backdrop's with overprint mode 1 (ISO 32000-1, 11.7.4.2).
    fn clear_inkless_overprints(&self, plate: &str, a: &mut Arc<Node>, discard_white: bool) {
        let inkless = |overprint: bool, p: &Paint| overprint && !p.is_none() && !(discard_white && is_white(p)) && self.no_ink(plate, p);
        let n = Arc::make_mut(a);
        for it in &mut n.appearance.items {
            let normal = match it {
                AppearanceItem::Fill(l) => l.blend == BlendMode::Normal,
                AppearanceItem::Stroke(l) => l.blend == BlendMode::Normal,
            };
            if normal && inkless(it.overprint(), it.paint()) {
                *it.paint_mut() = Paint::None;
            }
        }
        if let NodeKind::Text(t) = &mut n.kind {
            for r in &mut t.runs {
                if inkless(r.style.overprint_fill, &r.style.fill) {
                    r.style.fill = Paint::None;
                }
                if inkless(r.style.overprint_stroke, &r.style.stroke) {
                    r.style.stroke = Paint::None;
                }
            }
        }
        for c in n.children_mut().into_iter().flatten() {
            self.clear_inkless_overprints(plate, c, discard_white);
        }
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
            self.clear_inkless_overprints(plate, n, discard_white);
            darken_overprints(n, discard_white);
            self.node(plate, Arc::make_mut(n));
            keep_colorized_pixels(n, &d.images, &mut colorized);
        }
        // A colorized image's pixels say where its color prints, on every plate: they are kept
        // under their own key, and only the color is separated.
        let mut kept = BTreeMap::new();
        for key in colorized {
            if let Some(blob) = d.images.get(&key) {
                kept.insert(colorized_key(&key), blob.clone());
            }
        }
        // A DCS file's image prints its own plates: a spot plate on its ink when the job prints
        // that ink, else as its process equivalent.
        let spots: BTreeSet<String> = if self.set.output.spots_to_process {
            BTreeSet::new()
        } else {
            plates(self.doc).into_iter().filter(|p| p.spot).map(|p| p.name).collect()
        };
        let own = |name: &str| spots.contains(name);
        // A spot plate printing as process prints as the document's swatch of its ink does, when
        // there is one; else as the process equivalent its file gives.
        let equivalent = |p: &vectorcraft_doc::separated::InkPlate| match self.doc.swatch(&p.name).and_then(|w| Some((w.paint.color()?, w.spot))) {
            Some((c, spot)) => {
                let c = self.doc.linked_color(c, spot);
                cms::PROCESS_PLATES.map(|q| self.ink(q, &c, None))
            }
            None => p.cmyk,
        };
        for blob in d.images.values_mut() {
            if let Some(s) = blob.separated() {
                if let Some(gray) = s.plate_png(plate, &own, &equivalent) {
                    *blob = gray;
                }
                continue;
            }
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
/// `keys`. A colorized image with a color mask is separated as it shows: its masked pixels
/// ([`Document::shown_image`]). An image whose pixels it shows in `images` don't take its colorize
/// paint ([`ImageBlob::takes_colorize`]) loses the paint instead, and is separated as the image it
/// shows.
fn keep_colorized_pixels(n: &mut Arc<Node>, images: &BTreeMap<String, ImageBlob>, keys: &mut BTreeSet<String>) {
    let colorized = |n: &Node| matches!(n.kind, NodeKind::Image(ref im) if im.pixels.is_none()) && n.extras().colorize.is_some();
    if !colorized(n) && n.children().is_none_or(|ch| ch.is_empty()) {
        return;
    }
    let node = Arc::make_mut(n);
    let shown = match &node.kind {
        NodeKind::Image(im) if colorized(node) => {
            let mask = Document::color_mask_of(node).filter(|m| m.made_for(im) && images.contains_key(&m.key));
            Some(mask.map_or(&im.key, |m| &m.key).clone())
        }
        _ => None,
    };
    if shown.as_ref().and_then(|key| images.get(key)).is_some_and(|b| !b.takes_colorize()) {
        node.extras_mut().colorize = None;
    }
    if let Some(key) = shown.filter(|_| colorized(node)) {
        if let Some(e) = node.extras_if_any() {
            e.color_mask = None;
        }
        if let NodeKind::Image(im) = &mut node.kind {
            im.key = colorized_key(&key);
            keys.insert(key);
        }
    }
    if let Some(ch) = node.children_mut() {
        for c in ch.iter_mut() {
            keep_colorized_pixels(c, images, keys);
        }
    }
}
