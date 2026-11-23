//! Separations: the inks of a job ([`print_inks`]) and the document as one ink's plate, every
//! colour (and image pixel) replaced by the grey of that ink's coverage. An image placed from a
//! DCS file prints its own plates ([`vectorcraft_doc::separated`]).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use vectorcraft_color::cms::{self, Cms, Model, ProfileKind};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{BlendMode, Color, GradientKind, GradientStop, Paint};
use vectorcraft_doc::inks::{Link, StopPlates, inks, plates, spot_color, stop_inks, visit_node_colors};
use vectorcraft_doc::overprint::{
    colorized_shown_key, darken_overprints, image_overprints_whole, image_row_draws, is_black_ink, is_colorized_image, is_white, refuses_colorize,
};
use vectorcraft_doc::{AppearanceItem, ColorMode, Document, ImageBlob, Node, NodeKind};

use super::{DEFAULT_FREQUENCY, InkSettings, PrintInk, PrintSettings};

/// The options of [`super::PrintOutput::inks`] by ink name, the first for a name winning: looked
/// up, not searched for each ink.
fn ink_options(set: &PrintSettings) -> HashMap<&str, &InkSettings> {
    let mut options = HashMap::new();
    for o in &set.output.inks {
        options.entry(o.name.as_str()).or_insert(o);
    }
    options
}

/// The inks of a separation of `doc`: the process inks, then one per spot swatch (none for a spot
/// ink that prints as process, [`document_inks`]), with the options of
/// [`super::PrintOutput::inks`]; and the notes of [`document_inks`].
pub fn print_inks(doc: &Document, set: &PrintSettings) -> (Vec<PrintInk>, Vec<String>) {
    let (mut list, notes) = document_inks(doc, set);
    list.retain(|p| !p.convert_to_process);
    (list, notes)
}

/// Every ink of `doc` as a separation lists it: the process inks, then one per spot swatch, with
/// the options of [`super::PrintOutput::inks`]. A spot ink prints as process inks
/// ([`PrintInk::convert_to_process`]) with Convert All Spot Colors to Process or its own
/// `convertToProcess`; its other options are kept for when it prints as a spot ink again. Notes:
/// one warning naming the options that name no ink, and one naming the process inks whose
/// options ask for a conversion they don't take.
pub fn document_inks(doc: &Document, set: &PrintSettings) -> (Vec<PrintInk>, Vec<String>) {
    let out = &set.output;
    let options = ink_options(set);
    let list: Vec<PrintInk> = plates(doc)
        .into_iter()
        .map(|p| {
            let o = options.get(p.name.as_str()).copied();
            PrintInk {
                print: o.is_none_or(|o| o.print),
                frequency: o.and_then(|o| o.frequency).unwrap_or(DEFAULT_FREQUENCY),
                angle: o.and_then(|o| o.angle).unwrap_or_else(|| PrintInk::default_angle(&p.name)),
                convert_to_process: p.spot && (out.spots_to_process || o.is_some_and(|o| o.convert_to_process)),
                spot: p.spot,
                name: p.name,
            }
        })
        .collect();
    let few = |all: Vec<&str>| {
        let more = all.len().saturating_sub(MAX_NAMED);
        let shown = all.into_iter().take(MAX_NAMED).map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
        if more > 0 { format!("{shown} and {more} more") } else { shown }
    };
    // One warning for all the options naming no ink, each list cut short: the options and the
    // inks both come from the file. A spot ink printing as process is still an ink of the job.
    let names: BTreeSet<&str> = list.iter().map(|p| p.name.as_str()).collect();
    let unknown: Vec<&str> = out.inks.iter().map(|i| i.name.as_str()).filter(|n| !names.contains(n)).collect();
    let mut notes = vec![];
    if !unknown.is_empty() {
        let inks = few(list.iter().map(|p| p.name.as_str()).collect());
        notes.push(format!("there is no ink named {} to print (inks: {inks})", few(unknown)));
    }
    let process: Vec<&str> =
        list.iter().filter(|p| !p.spot && options.get(p.name.as_str()).is_some_and(|o| o.convert_to_process)).map(|p| p.name.as_str()).collect();
    if !process.is_empty() {
        notes.push(format!("{} print as process inks already: only spot inks convert to process", few(process)));
    }
    (list, notes)
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
    /// The inks whose own options convert them to process ([`InkSettings::convert_to_process`]).
    converted: HashSet<&'a str>,
    /// The plates are written as PostScript, which has no blending modes.
    postscript: bool,
}

impl<'a> Separator<'a> {
    pub fn new(doc: &'a Document, set: &'a PrintSettings, postscript: bool) -> Self {
        let converted = ink_options(set).into_iter().filter(|(_, o)| o.convert_to_process).map(|(name, _)| name).collect();
        Self { doc, cms: separation_cms(doc, set), set, converted, postscript }
    }

    /// Whether colors linked to swatch `name` print as process inks: every swatch but
    /// Registration with Convert All Spot Colors to Process, else the inks converted one by one.
    /// A link to a process swatch separates as its color either way.
    fn converts(&self, name: &str) -> bool {
        name != REGISTRATION && (self.set.output.spots_to_process || self.converted.contains(name))
    }

    /// How much of ink `plate` colour `c` (with swatch link `link`) prints, 0..1.
    fn ink(&self, plate: &str, c: &Color, link: Link) -> f32 {
        let color = &self.set.color;
        // Spot colors as process lose their link; Registration always keeps its.
        let link = link.filter(|(name, _)| !self.converts(name));
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
    /// ([`stop_inks`]): the spot plate prints the tint and the process plates print `plates`. When
    /// its spot ink prints as process, each process plate prints `plates` layered with the spot
    /// tint's process equivalent, as a separated image's plates layer
    /// ([`vectorcraft_doc::separated::Separated::coverage_as`]).
    fn stop_ink(&self, plate: &str, c: &Color, link: Link, plates: StopPlates) -> f32 {
        let own = stop_inks(self.doc, &self.cms, c, link, plates, self.set.color.intent);
        let (Some(_), Some((name, tint))) = (plates, own.spot.as_ref()) else {
            return self.ink(plate, c, link);
        };
        if !self.converts(name) {
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
    /// spot swatch or to Registration: up to the half a percent of another ink such a black may
    /// carry counts as none. A link to a swatch the document lacks separates as the color's own
    /// values, so such a black counts as unlinked.
    fn no_ink(&self, plate: &str, p: &Paint) -> bool {
        match p {
            Paint::Solid { color, swatch, tint } => {
                self.none(color, self.process(swatch.as_deref()), self.ink(plate, color, swatch.as_deref().map(|s| (s, *tint))))
            }
            Paint::Gradient(g) => match g.freeform.as_ref().filter(|f| g.gradient.kind == GradientKind::Freeform && !f.points.is_empty()) {
                Some(f) => f.points.iter().all(|pt| self.point_none(plate, &pt.color)),
                None => !g.gradient.stops.is_empty() && g.gradient.stops.iter().all(|s| self.stop_none(plate, s)),
            },
            _ => false,
        }
    }

    /// Whether a color linked to swatch `swatch` separates as process inks: no link, or a link
    /// to a process swatch or to one the document lacks, other than Registration.
    fn process(&self, swatch: Option<&str>) -> bool {
        swatch.is_none_or(|n| n != REGISTRATION && self.doc.swatch(n).is_none_or(|w| !w.spot))
    }

    /// Whether `ink` of color `c` (separating as process inks when `process` is true) counts as
    /// none, as [`Self::no_ink`] says.
    fn none(&self, c: &Color, process: bool, ink: f32) -> bool {
        ink <= 0.0 || (self.postscript && process && ink <= 0.005 && is_black_ink(c, 1.0, false))
    }

    /// Whether gradient stop `s` puts none of ink `plate` down, as [`Self::no_ink`] says.
    fn stop_none(&self, plate: &str, s: &GradientStop) -> bool {
        let ink = self.stop_ink(plate, &s.color, s.swatch.as_deref().map(|n| (n, s.tint)), s.plates);
        self.none(&s.color, s.plates.is_none() && self.process(s.swatch.as_deref()), ink)
    }

    /// Whether a freeform gradient's point of color `c` puts none of ink `plate` down, as
    /// [`Self::no_ink`] says.
    fn point_none(&self, plate: &str, c: &Color) -> bool {
        self.none(c, true, self.ink(plate, c, None))
    }

    /// On a PostScript plate, make paper white the stops and points of overprinting gradient `p`
    /// that put none of ink `plate` down while others do ([`Self::no_ink`]'s rule for each): the
    /// writer leaves the plate below as it is where the gradient is paper white.
    fn clear_inkless_stops(&self, plate: &str, p: &mut Paint) {
        let Paint::Gradient(g) = p else { return };
        if !self.postscript {
            return;
        }
        let paper = Color::Gray { k: 0.0 };
        match g.freeform.as_mut().filter(|f| g.gradient.kind == GradientKind::Freeform && !f.points.is_empty()) {
            Some(f) => {
                for pt in f.points.iter_mut().filter(|pt| self.point_none(plate, &pt.color)) {
                    pt.color = paper;
                }
            }
            None => {
                for s in g.gradient.stops.iter_mut().filter(|s| self.stop_none(plate, s)) {
                    *s = GradientStop { color: paper, swatch: None, tint: 1.0, plates: None, ..s.clone() };
                }
            }
        }
    }

    /// Overprinting fills and strokes of `a`'s subtree with a Normal blending mode, and
    /// overprinting character fills and strokes, that put none of ink `plate` down leave its plate
    /// as it is: they are drawn as nothing on it (white ones with `discard_white` aside, which
    /// knock out). A writer without blending modes (PostScript) then keeps the ink below them,
    /// where it would draw their Darken as a knockout. One that puts any ink down is drawn, and on
    /// a PostScript plate its ink replaces the ink below, as a nonzero component of an overprinting
    /// color replaces the backdrop's with overprint mode 1 (ISO 32000-1, 11.7.4.2). On a
    /// PostScript plate, the stops and points of such a gradient that put none of the ink down
    /// become paper white ([`Self::clear_inkless_stops`]), which the writer leaves out there. A
    /// colorized image that overprints as a whole ([`image_overprints_whole`]) and whose color puts
    /// none of the ink down is hidden on the plate once none of its rows draws there; while a row
    /// draws, the PostScript writer leaves out the image's paper-white pixels and draws the row.
    /// A colorized image that does not overprint as a whole, or whose pixels don't take its color
    /// ([`refuses_colorize`]) and so print as they are, knocks out on the plate.
    fn clear_inkless_overprints(&self, plate: &str, a: &mut Arc<Node>, discard_white: bool) {
        let inkless = |overprint: bool, p: &Paint| overprint && !p.is_none() && !(discard_white && is_white(p)) && self.no_ink(plate, p);
        let n = Arc::make_mut(a);
        let whole = image_overprints_whole(n, discard_white, &self.doc.images);
        if n.colorize_overprints() && !whole {
            n.extras_mut().colorize_overprint = false;
        }
        let image = whole && n.extras().colorize.as_deref().is_some_and(|p| self.no_ink(plate, p));
        for it in &mut n.appearance.items {
            let normal = match it {
                AppearanceItem::Fill(l) => l.blend == BlendMode::Normal,
                AppearanceItem::Stroke(l) => l.blend == BlendMode::Normal,
            };
            if normal && inkless(it.overprint(), it.paint()) {
                *it.paint_mut() = Paint::None;
            } else if normal && it.overprint() {
                self.clear_inkless_stops(plate, it.paint_mut());
            }
        }
        if image && !n.appearance.items.iter().any(image_row_draws) {
            n.visible = false;
        }
        if let NodeKind::Text(t) = &mut n.kind {
            for r in &mut t.runs {
                if inkless(r.style.overprint_fill, &r.style.fill) {
                    r.style.fill = Paint::None;
                } else if r.style.overprint_fill {
                    self.clear_inkless_stops(plate, &mut r.style.fill);
                }
                if inkless(r.style.overprint_stroke, &r.style.stroke) {
                    r.style.stroke = Paint::None;
                } else if r.style.overprint_stroke {
                    self.clear_inkless_stops(plate, &mut r.style.stroke);
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
            darken_overprints(n, discard_white, &d.images);
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
        // that ink as a spot ink, else as its process equivalent.
        let spots: BTreeSet<String> = plates(self.doc).into_iter().filter(|p| p.spot && !self.converts(&p.name)).map(|p| p.name).collect();
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
    if !is_colorized_image(n) && n.children().is_none_or(|ch| ch.is_empty()) {
        return;
    }
    let node = Arc::make_mut(n);
    let shown = colorized_shown_key(node, images);
    if refuses_colorize(node, images) {
        node.extras_mut().colorize = None;
    }
    if let Some(key) = shown.filter(|_| is_colorized_image(node)) {
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
