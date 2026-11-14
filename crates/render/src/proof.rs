//! Soft proofing (View → Proof Colors), Overprint Preview and Separations Preview.
//!
//! * **Proof Colors** post-processes the rendered frame: every pixel goes display sRGB → proof
//!   space (CMYK press or RGB/colour-blindness simulation) → display, through a cached 17³ LUT.
//! * **Separations Preview** recolours the *document* before rendering: each paint colour is split
//!   into process inks (colour-managed), or a spot ink at the colour's tint when it's linked to a
//!   spot swatch (solid colours and gradient stops alike), with the process plates a gradient stop
//!   prints beside its spot ink ([`vectorcraft_color::GradientStop::plates`]). With one plate visible the plate
//!   renders as greyscale ink coverage (black = 100%); with several, the visible inks are
//!   composited through the proof profile, spots multiplied on top. Colours linked to the
//!   Registration swatch print their tint on every plate, spot plates included.
//!   Placed raster images show their pixels' process inks the same way ([`plate_image`]): a CMYK
//!   image its own inks, an RGB one its pixels separated through the color settings. They carry
//!   no spot ink; a colorized image takes its color's plates, as a fill of it does. An image
//!   placed from a DCS file shows its own plates, spot plates included ([`plates_shown`]).
//! * **Overprint Preview** (and Separations Preview, which implies it): fills and strokes that
//!   overprint ([`vectorcraft_doc::FillLayer::overprint`], characters' too) are drawn with
//!   Darken, which approximates their inks printing over the inks below: a zero ink lets the
//!   inks below show through, where a knockout would replace them.
//!
//! The app's current view state lives in [`view`] / [`set_view`]; the canvas copies it into
//! [`crate::RenderOptions`] with [`active_proof`] and [`overprint_preview_on`].

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use vectorcraft_color::cms::{self, Cms, CmykLut, PROCESS_PLATES, ProofLut, lab};
pub use vectorcraft_color::cms::{Intent, ProofSetup, ProofTarget};
use vectorcraft_color::{Color, Swatch, SwatchGroup};
pub use vectorcraft_doc::inks::{Inks, Link, Plate, inks, map_document_colors, map_node_colors, plates, spot_color};
use vectorcraft_doc::inks::{StopPlates, map_node_inks, stop_inks};
use vectorcraft_doc::overprint::darken_overprints;
use vectorcraft_doc::{Document, ImageBlob, Node, NodeId, Shared};
use vello_cpu::Pixmap;
use vello_cpu::color::PremulRgba8;

use crate::RenderOptions;
use crate::blobs::lookup;
use crate::images::{self, ImageKey};

/// Whether overprinting shows: Overprint Preview, or Separations Preview (which implies it).
pub(crate) fn overprints(opts: &RenderOptions) -> bool {
    opts.overprint_preview || opts.proof.as_ref().is_some_and(|p| p.separations.is_some())
}

/// What the `visible` plates print of `ink`, as display sRGB, without the spot inks over several
/// plates: with one plate visible its ink as a grey (black = 100%), else the visible process inks
/// through the proof profile.
fn plate_rgb(c: &Cms, proof: &ProofSetup, visible: &[String], ink: &Inks) -> [f32; 3] {
    if let [one] = visible {
        return [1.0 - ink.on_plate(one); 3];
    }
    let mut cmyk = ink.cmyk;
    for (i, p) in PROCESS_PLATES.iter().enumerate() {
        if !visible.iter().any(|v| v == p) {
            cmyk[i] = 0.0;
        }
    }
    c.proof_cmyk_to_srgb(cmyk, proof)
}

fn plate_color(doc: &Document, c: &Cms, proof: &ProofSetup, visible: &[String], color: &Color, link: Link, plates: StopPlates) -> Color {
    let ink = stop_inks(doc, c, color, link, plates, proof.intent);
    let mut rgb = plate_rgb(c, proof, visible, &ink);
    // Over several plates the visible spot inks print on top.
    for name in visible.iter().filter(|v| visible.len() != 1 && !PROCESS_PLATES.contains(&v.as_str())) {
        let t = ink.spot_tint(name);
        if t > 0.0
            && let Some(sc) = spot_color(doc, name)
        {
            let s = c.display_rgb(&sc);
            for i in 0..3 {
                rgb[i] *= 1.0 - t * (1.0 - s[i]);
            }
        }
    }
    let [r, g, b] = c.srgb_to_rgb(rgb);
    Color::Rgb { r, g, b }
}

/// Separations Preview's tables for placed images: what the visible plates show of display pixels
/// and of a CMYK image's own inks ([`plate_rgb`]), for one proof setup, color settings and set of
/// profiles. Each table is made when an image first needs it.
pub(crate) struct PlateLut {
    proof: ProofSetup,
    /// The document's solid swatches: name, whether it is a spot color, and its color. An image
    /// placed from a DCS file shows a spot plate on its own ink only when it is one of the spot
    /// ones, in that swatch's color, and else as the swatch of its name ([`plates_shown`]).
    swatches: Vec<(String, bool, Color)>,
    cms: Arc<Cms>,
    /// [`cms::profiles_generation`].
    profiles: u64,
    /// What pixels shown through these tables are kept under ([`ImageKey::plates`]): never reused.
    pub(crate) generation: u64,
    /// Display sRGB → display sRGB.
    rgb: OnceLock<ProofLut>,
    /// Inks → XYZ (inks mix close to linearly in light, as in `ink::InkLut`).
    cmyk: OnceLock<CmykLut>,
}

impl PlateLut {
    /// What the visible plates show of process inks `cmyk`, as display sRGB.
    fn shown(&self, cmyk: [f32; 4]) -> [f32; 3] {
        let visible = self.proof.separations.as_deref().unwrap_or_default();
        plate_rgb(&self.cms, &self.proof, visible, &Inks { cmyk, spot: None, registration: false })
    }
}

/// The tables [`plate_lut`] handed out last, the newest last: a few, so renderers drawing with
/// different proof setups don't make them again and again.
static PLATE_LUTS: Mutex<Vec<Arc<PlateLut>>> = Mutex::new(Vec::new());
/// The last [`PlateLut::generation`] handed out.
static PLATE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The tables the placed images of `doc` show Separations Preview's visible plates through with
/// `opts`, or `None` when it is off.
pub(crate) fn plate_lut(opts: &RenderOptions, doc: &Document) -> Option<Arc<PlateLut>> {
    let proof = opts.proof.as_ref().filter(|p| p.separations.is_some())?;
    let (c, profiles) = (cms::active(), cms::profiles_generation());
    // The spot ones are the spot plates of `vectorcraft_doc::inks::plates`; their colors are part
    // of the tables' identity, so a swatch edit shows.
    let swatches: Vec<(String, bool, Color)> = doc
        .swatches_iter()
        .filter_map(|s| match &s.paint {
            vectorcraft_color::Paint::Solid { color, .. } => Some((s.name.clone(), s.spot, doc.linked_color(*color, s.spot))),
            _ => None,
        })
        .collect();
    let mut luts = PLATE_LUTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(l) = luts.iter().find(|l| Arc::ptr_eq(&l.cms, &c) && l.profiles == profiles && l.proof == *proof && l.swatches == swatches) {
        return Some(l.clone());
    }
    if luts.len() >= 4 {
        luts.remove(0);
    }
    let generation = PLATE_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let l = Arc::new(PlateLut { proof: proof.clone(), swatches, cms: c, profiles, generation, rgb: OnceLock::new(), cmyk: OnceLock::new() });
    luts.push(l.clone());
    Some(l)
}

/// Image pixels `pm` (kept as `key`, from `blob` when it is a document's image as it is) as the
/// visible plates show them through `lut`, kept with the images, and their key: a CMYK image shows
/// its own inks, other pixels their inks separated with the proof's intent. `touch` marks them
/// used there.
pub(crate) fn plate_image(lut: &PlateLut, key: &ImageKey, pm: &Pixmap, blob: Option<&ImageBlob>, touch: bool) -> (ImageKey, Arc<Pixmap>) {
    let want = ImageKey { plates: lut.generation, ..key.clone() };
    if let Some(p) = lookup(&want, touch) {
        return (want, p);
    }
    // An image placed from a DCS file shows its own plates, spot plates included.
    if let Some(s) = blob.filter(|b| crate::ink::shows_own_inks(key) && b.is_separated()).and_then(ImageBlob::separated) {
        let out = images::store().insert(want.clone(), Arc::new(plates_shown(lut, &s, pm)));
        return (want, out);
    }
    let size = (u32::from(pm.width()), u32::from(pm.height()));
    let raw = crate::ink::key_inks(key, blob).filter(|i| (i.width, i.height) == size);
    let mut out = pm.clone();
    match raw {
        Some(inks) => {
            let t = lut.cmyk.get_or_init(|| {
                CmykLut::build(|inks| {
                    let [x, y, z] = lab::srgb_to_xyz(lut.shown(inks));
                    [x, y, z, 0.0]
                })
            });
            for (o, p) in out.data_mut().iter_mut().zip(inks.data.as_chunks::<4>().0) {
                let [x, y, z, _] = t.apply(p.map(|v| f32::from(v) / 255.0));
                let [r, g, b] = lab::xyz_to_srgb([x, y, z]).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                *o = PremulRgba8 { r, g, b, a: 255 };
            }
        }
        None => {
            let t = lut.rgb.get_or_init(|| ProofLut::build(|rgb| lut.shown(lut.cms.srgb_to_cmyk(rgb, lut.proof.intent))));
            for px in out.data_mut().iter_mut().filter(|p| p.a > 0) {
                let a = f32::from(px.a);
                let un = |v: u8| (f32::from(v) / a).min(1.0);
                let [r, g, b] = t.apply([un(px.r), un(px.g), un(px.b)]).map(|v| (v.clamp(0.0, 1.0) * a).round() as u8);
                (px.r, px.g, px.b) = (r, g, b);
            }
        }
    }
    let out = images::store().insert(want.clone(), Arc::new(out));
    (want, out)
}

/// The visible plates of separated image `s` drawn at the size of `pm` (each pixel the mean of the
/// plate pixels it covers, [`Fit`]): with one spot plate visible its ink as a grey, else the
/// visible process inks through `lut` with the visible spot inks multiplied on top, each in the
/// color of the document's spot swatch of its ink. A spot plate whose ink the document has no spot
/// swatch for shows on the process plates as it prints
/// ([`vectorcraft_doc::separated::Separated::coverage_as`]): as the document's swatch of its name
/// when there is one, else as the process equivalent its file gives.
fn plates_shown(lut: &PlateLut, s: &vectorcraft_doc::separated::Separated, pm: &Pixmap) -> Pixmap {
    let visible = lut.proof.separations.as_deref().unwrap_or_default();
    let shown = |name: &str| visible.iter().any(|v| v == name);
    let swatch = |name: &str| lut.swatches.iter().find(|(n, ..)| n == name);
    let own = |name: &str| swatch(name).is_some_and(|(_, spot, _)| *spot);
    let (w, h) = (usize::from(pm.width()).max(1), usize::from(pm.height()).max(1));
    let fit = Fit::new(((s.width as usize).max(1), (s.height as usize).max(1)), (w, h));
    let alone = match visible {
        [one] if !PROCESS_PLATES.contains(&one.as_str()) => Some(s.plates.iter().find(|p| p.name == *one && own(&p.name))),
        _ => None,
    };
    let spots: Vec<(&[u8], [f32; 3])> = s
        .spots()
        .filter(|p| own(&p.name) && shown(&p.name))
        .map(|p| (p.data.as_slice(), swatch(&p.name).map_or_else(|| lut.cms.cmyk_to_srgb(p.cmyk, false), |(.., c)| lut.cms.display_rgb(c))))
        .collect();
    // The plates that print on the process plates, each with the process inks it stands for.
    let process: Vec<(&[u8], [f32; 4])> = s
        .plates
        .iter()
        .filter(|p| !p.spot() || !own(&p.name))
        .map(|p| {
            let inks = match swatch(&p.name) {
                Some((.., c)) if p.spot() => lut.cms.to_cmyk(c, lut.proof.intent),
                _ => p.cmyk,
            };
            (p.data.as_slice(), inks)
        })
        .collect();
    let t = lut.cmyk.get_or_init(|| {
        CmykLut::build(|inks| {
            let [x, y, z] = lab::srgb_to_xyz(lut.shown(inks));
            [x, y, z, 0.0]
        })
    });
    let mut out = pm.clone();
    for (at, o) in out.data_mut().iter_mut().enumerate() {
        let ink = |data: &[u8]| fit.ink(data, at);
        let rgb = match alone {
            Some(p) => [1.0 - p.map_or(0.0, |p| ink(&p.data)); 3],
            None => {
                let mut clear = [1.0f32; 4];
                for (data, inks) in &process {
                    let a = ink(data);
                    for (c, e) in clear.iter_mut().zip(inks) {
                        *c *= 1.0 - a * e;
                    }
                }
                let [x, y, z, _] = t.apply(clear.map(|c| 1.0 - c));
                let mut rgb = lab::xyz_to_srgb([x, y, z]);
                for (data, c) in &spots {
                    let a = ink(data);
                    for (v, c) in rgb.iter_mut().zip(c) {
                        *v *= 1.0 - a * (1.0 - c);
                    }
                }
                rgb
            }
        };
        let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        *o = PremulRgba8 { r, g, b, a: 255 };
    }
    out
}

/// Plates of `from` (width, height) pixels drawn at `to` pixels: each pixel the mean of the plate
/// pixels it covers, so a reduced image (a smaller mip level) keeps the coverage the plate has; the
/// nearest plate pixel when it covers less than one. It keeps only the spans of each column and
/// row, not a copy of any plate.
struct Fit {
    /// The plate's width.
    sw: usize,
    /// The shown width.
    w: usize,
    /// The plate columns each shown column covers, `start..end`.
    columns: Vec<(usize, usize)>,
    /// The plate rows each shown row covers, `start..end`.
    rows: Vec<(usize, usize)>,
}

impl Fit {
    fn new((sw, sh): (usize, usize), (w, h): (usize, usize)) -> Self {
        // The plate pixels each shown column or row covers: at least one.
        // In 64 bits: a plate's width times the shown width may not fit 32 (the web build).
        let span = |i: usize, n: usize, of: usize| {
            let at = |i: usize| usize::try_from(i as u64 * of as u64 / n as u64).unwrap_or(of);
            let start = at(i).min(of - 1);
            (start, at(i + 1).clamp(start + 1, of))
        };
        Self { sw, w, columns: (0..w).map(|x| span(x, w, sw)).collect(), rows: (0..h).map(|y| span(y, h, sh)).collect() }
    }

    /// The ink of plate `data` at shown pixel `at` (row by row), from 0 to 1.
    fn ink(&self, data: &[u8], at: usize) -> f32 {
        let (Some(&(x0, x1)), Some(&(y0, y1))) = (self.columns.get(at % self.w), self.rows.get(at / self.w)) else { return 0.0 };
        let mut sum = 0u64;
        for row in y0..y1 {
            sum += data.get(row * self.sw + x0..row * self.sw + x1).map_or(0, |r| r.iter().map(|&v| u64::from(v)).sum());
        }
        let n = ((y1 - y0) * (x1 - x0)) as u64;
        sum as f32 / n as f32 / 255.0
    }
}

/// The art a renderer drew with Overprint Preview or Separations Preview, kept for its next frame
/// ([`prepare`]): the document's layers and symbol art as they were, each beside the art drawn for
/// it, and what that art was made with. The kept nodes keep their addresses: no other node gets
/// one of them while they are kept.
pub(crate) struct Prepared {
    inputs: Inputs,
    layers: (Vec<Arc<Node>>, Vec<Arc<Node>>),
    art: (Vec<Arc<Node>>, Vec<Arc<Node>>),
}

/// What [`prepare`] reads besides the art.
#[derive(PartialEq)]
struct Inputs {
    discard_white: bool,
    plates: Option<Plates>,
}

/// What the colors of Separations Preview's plates are made from ([`plate_color`]).
struct Plates {
    /// The proof setup, with the visible plates.
    proof: ProofSetup,
    /// The document's swatches, swatch groups and Spot Colors option.
    swatches: Shared<Vec<Swatch>>,
    groups: Shared<Vec<SwatchGroup>>,
    spot_use_lab: bool,
    /// The color settings, held weakly: no other settings get their address while they are kept.
    cms: Weak<Cms>,
    /// [`cms::profiles_generation`].
    profiles: u64,
}

impl PartialEq for Plates {
    fn eq(&self, o: &Self) -> bool {
        (&self.proof, &self.swatches, &self.groups, self.spot_use_lab, self.profiles)
            == (&o.proof, &o.swatches, &o.groups, o.spot_use_lab, o.profiles)
            && Weak::ptr_eq(&self.cms, &o.cms)
    }
}

/// What [`prepare`] does to the fills and strokes of each node: overprinting ones darken
/// ([`darken_overprints`]), and with Separations Preview every color shows as the visible plates
/// print it ([`plate_color`]).
struct How<'a> {
    doc: &'a Document,
    discard_white: bool,
    /// Separations Preview: the color settings, the proof setup and the visible plates.
    plates: Option<(&'a Cms, &'a ProofSetup, &'a [String])>,
}

impl How<'_> {
    /// Whether [`Self::apply`] can change `n`: with Separations Preview any node, else a node with
    /// an overprinting fill or stroke.
    fn changes(&self, n: &Node) -> bool {
        self.plates.is_some() || n.appearance.items.iter().any(|i| i.overprint())
    }

    /// Change the fills, strokes and other colors of node `a`, which holds no members.
    fn apply(&self, a: &mut Arc<Node>) {
        darken_overprints(a, self.discard_white);
        if let Some((c, proof, visible)) = self.plates {
            map_node_inks(Arc::make_mut(a), &mut |col, sw, plates| plate_color(self.doc, c, proof, visible, col, sw, plates));
        }
    }
}

/// The document as it should be drawn for these options (overprints, separations). `kept` holds
/// the art of an earlier frame ([`Prepared`]): when that frame's inputs were the same, each layer,
/// symbol art and member that is still the node its kept art was made from is drawn from that art,
/// with its `Arc`. `kept` then holds this frame's art, or nothing when neither preview is on.
pub(crate) fn prepare<'a>(doc: &'a Document, opts: &RenderOptions, kept: &mut Option<Prepared>) -> Cow<'a, Document> {
    if !overprints(opts) {
        *kept = None;
        return Cow::Borrowed(doc);
    }
    let seps = opts.proof.as_ref().and_then(|p| p.separations.as_ref().map(|s| (p, s)));
    let c = cms::active();
    let inputs = Inputs {
        discard_white: doc.setup.discard_white_overprint,
        plates: seps.map(|(proof, _)| Plates {
            proof: proof.clone(),
            swatches: doc.swatches.clone(),
            groups: doc.swatch_groups.clone(),
            spot_use_lab: doc.spot_use_lab,
            cms: Arc::downgrade(&c),
            profiles: cms::profiles_generation(),
        }),
    };
    let earlier = kept.take().filter(|k| k.inputs == inputs);
    let how = How { doc, discard_white: inputs.discard_white, plates: seps.map(|(proof, visible)| (&*c, proof, visible.as_slice())) };
    let none = (vec![], vec![]);
    let (layers, art) = earlier.as_ref().map_or((&none, &none), |k| (&k.layers, &k.art));
    let made_layers = prepare_list(&doc.layers, &layers.0, &layers.1, &how);
    let symbol_art: Vec<Arc<Node>> = doc.symbols.iter().map(|s| s.art.clone()).collect();
    let made_art = prepare_list(&symbol_art, &art.0, &art.1, &how);
    let same = |made: &[Arc<Node>], from: &[Arc<Node>]| made.iter().zip(from).all(|(a, b)| Arc::ptr_eq(a, b));
    let out = if same(&made_layers, &doc.layers) && same(&made_art, &symbol_art) {
        Cow::Borrowed(doc)
    } else {
        let mut d = doc.clone();
        d.layers.clone_from(&made_layers);
        for (s, a) in d.symbols.iter_mut().zip(&made_art) {
            s.art = a.clone();
        }
        Cow::Owned(d)
    };
    *kept = Some(Prepared { inputs, layers: (doc.layers.clone(), made_layers), art: (symbol_art, made_art) });
    out
}

/// `nodes` as drawn with `how` ([`prepare_node`]), each against its earlier version in `was`: the
/// node at its place when that node has its id, else the node with its id. `made` holds the art
/// drawn for `was`, place by place.
fn prepare_list(nodes: &[Arc<Node>], was: &[Arc<Node>], made: &[Arc<Node>], how: &How) -> Vec<Arc<Node>> {
    let mut places: Option<HashMap<NodeId, usize>> = None;
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let at = match was.get(i) {
                Some(w) if w.id == n.id => Some(i),
                _ => places.get_or_insert_with(|| was.iter().enumerate().map(|(j, w)| (w.id, j)).collect()).get(&n.id).copied(),
            };
            prepare_node(n, at.and_then(|j| Some((was.get(j)?, made.get(j)?))), how)
        })
        .collect()
}

/// `n` as drawn with `how`: the art of `earlier` (a node and the art drawn for it) when that art
/// was made from `n` itself; else a copy of `n` with its own fills and strokes changed
/// ([`How::apply`]) and its members prepared against those of `earlier` ([`prepare_list`]), or `n`
/// itself when neither changes.
fn prepare_node(n: &Arc<Node>, earlier: Option<(&Arc<Node>, &Arc<Node>)>, how: &How) -> Arc<Node> {
    if let Some((was, made)) = earlier
        && Arc::ptr_eq(was, n)
    {
        return made.clone();
    }
    let members = n.children().map(|ch| {
        let (was, made) = earlier.and_then(|(w, m)| Some((w.children()?.as_slice(), m.children()?.as_slice()))).unwrap_or_default();
        prepare_list(ch, was, made, how)
    });
    let same_members = match (&members, n.children()) {
        (Some(m), Some(ch)) => m.iter().zip(ch).all(|(a, b)| Arc::ptr_eq(a, b)),
        _ => true,
    };
    if same_members && !how.changes(n) {
        return n.clone();
    }
    let mut own = Node::clone(n);
    if let Some(ch) = own.children_mut() {
        ch.clear();
    }
    let mut a = Arc::new(own);
    how.apply(&mut a);
    if let (Some(m), Some(ch)) = (members, Arc::make_mut(&mut a).children_mut()) {
        *ch = m;
    }
    a
}

/// Soft-proof the rendered (premultiplied RGBA8) pixels in place.
pub(crate) fn post(pixels: &mut [u8], opts: &RenderOptions) {
    let Some(proof) = opts.proof.as_ref() else { return };
    if proof.separations.is_some() || matches!(proof.target, ProofTarget::MonitorRgb | ProofTarget::Srgb) {
        return;
    }
    let lut = cms::active().proof_lut(proof);
    let mut last_in = [0u8; 4];
    let mut last_out = [0u8; 4];
    for px in pixels.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a == 0 {
            continue;
        }
        if *px == last_in {
            px.copy_from_slice(&last_out);
            continue;
        }
        last_in.copy_from_slice(px);
        let un = |c: u8| if a == 255 { c } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        let out = lut.apply8([un(px[0]), un(px[1]), un(px[2])]);
        for i in 0..3 {
            px[i] = if a == 255 { out[i] } else { ((out[i] as u32 * a as u32 + 127) / 255) as u8 };
        }
        last_out.copy_from_slice(px);
    }
}

/// View-level proof state (View → Proof Setup / Proof Colors / Overprint Preview, Separations
/// Preview panel). Process-wide: the app has one proof state for all windows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProofView {
    /// View → Proof Setup.
    pub setup: ProofSetup,
    /// View → Proof Colors.
    pub proof_colors: bool,
    /// View → Overprint Preview.
    pub overprint: bool,
    /// Separations Preview: `Some(visible plates)` when on.
    pub separations: Option<Vec<String>>,
}

static VIEW: RwLock<Option<ProofView>> = RwLock::new(None);

pub fn view() -> ProofView {
    VIEW.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

pub fn set_view(v: ProofView) {
    *VIEW.write().unwrap_or_else(|e| e.into_inner()) = Some(v);
}

/// The proof to pass in [`RenderOptions::proof`] for the current view state.
pub fn active_proof() -> Option<ProofSetup> {
    let v = view();
    match v.separations {
        Some(s) => Some(ProofSetup { separations: Some(s), ..v.setup }),
        None if v.proof_colors => Some(ProofSetup { separations: None, ..v.setup }),
        None => None,
    }
}

/// Whether [`RenderOptions::overprint_preview`] should be on (Separations Preview implies it).
pub fn overprint_preview_on() -> bool {
    let v = view();
    v.overprint || v.separations.is_some()
}
