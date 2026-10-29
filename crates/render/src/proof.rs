//! Soft proofing (View → Proof Colors), Overprint Preview and Separations Preview.
//!
//! * **Proof Colors** post-processes the rendered frame: every pixel goes display sRGB → proof
//!   space (CMYK press or RGB/colour-blindness simulation) → display, through a cached 17³ LUT.
//! * **Separations Preview** recolours the *document* before rendering: each paint colour is split
//!   into process inks (colour-managed), or a spot ink at the colour's tint when it's linked to a
//!   spot swatch (solid colours and gradient stops alike). With one plate visible the plate
//!   renders as greyscale ink coverage (black = 100%); with several, the visible inks are
//!   composited through the proof profile, spots multiplied on top. Colours linked to the
//!   Registration swatch print their tint on every plate, spot plates included.
//!   Placed raster images show their pixels the same way ([`plate_image`]): RGB pixels are
//!   separated into process inks, CMYK images show their own inks; they print on no spot plate.
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
use vectorcraft_doc::overprint::darken_overprints;
use vectorcraft_doc::{Document, ImageBlob, Node, NodeId, Shared};
use vello_cpu::Pixmap;
use vello_cpu::color::PremulRgba8;

use crate::RenderOptions;
use crate::images::{self, ImageKey};

/// Whether overprinting shows: Overprint Preview, or Separations Preview (which implies it).
pub(crate) fn overprints(opts: &RenderOptions) -> bool {
    opts.overprint_preview || opts.proof.as_ref().is_some_and(|p| p.separations.is_some())
}

fn plate_color(doc: &Document, c: &Cms, proof: &ProofSetup, visible: &[String], color: &Color, link: Link) -> Color {
    let ink = inks(doc, c, color, link, proof.intent);
    let rgb = if visible.len() == 1 {
        [1.0 - ink.on_plate(&visible[0]); 3]
    } else {
        let mut rgb = visible_process_srgb(c, proof, visible, ink.cmyk);
        for name in visible.iter().filter(|v| !PROCESS_PLATES.contains(&v.as_str())) {
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
        rgb
    };
    let [r, g, b] = c.srgb_to_rgb(rgb);
    Color::Rgb { r, g, b }
}

/// The display sRGB of process inks `cmyk` with only the `visible` plates printing, through the
/// proof's CMYK space.
fn visible_process_srgb(c: &Cms, proof: &ProofSetup, visible: &[String], mut cmyk: [f32; 4]) -> [f32; 3] {
    for (i, p) in PROCESS_PLATES.iter().enumerate() {
        if !visible.iter().any(|v| v == p) {
            cmyk[i] = 0.0;
        }
    }
    c.proof_cmyk_to_srgb(cmyk, proof)
}

/// The tables Separations Preview shows placed images' pixels with ([`plate_image`]), for one
/// proof setup and its visible plates.
pub(crate) struct ImagePlates {
    cms: Arc<Cms>,
    /// [`cms::profiles_generation`].
    profiles: u64,
    proof: ProofSetup,
    /// Images made with these tables are kept under it ([`ImageKey::plates`]); never reused.
    generation: u64,
    /// One plate visible: the process plate's index, `None` for a spot plate (no image prints on
    /// it).
    single: Option<Option<usize>>,
    /// Display sRGB → the visible plates as they show (display sRGB).
    rgb: ProofLut,
    /// CMYK inks → the visible plates as they show (XYZ), made when a CMYK image is first shown
    /// with several plates.
    cmyk: OnceLock<CmykLut>,
}

impl ImagePlates {
    fn build(cms: Arc<Cms>, profiles: u64, proof: ProofSetup, generation: u64) -> Self {
        let visible = proof.separations.as_deref().unwrap_or_default();
        let single = match visible {
            [one] => Some(PROCESS_PLATES.iter().position(|p| p == one)),
            _ => None,
        };
        let rgb = ProofLut::build(|rgb| {
            let inks = cms.srgb_to_cmyk(rgb, proof.intent);
            match single {
                Some(i) => [1.0 - i.map_or(0.0, |i| inks[i]); 3],
                None => visible_process_srgb(&cms, &proof, visible, inks),
            }
        });
        Self { cms, profiles, proof, generation, single, rgb, cmyk: OnceLock::new() }
    }

    /// CMYK ink amounts `inks` (0 = no ink) as the visible plates show them.
    fn show_inks(&self, inks: [u8; 4]) -> [u8; 3] {
        match self.single {
            Some(Some(i)) => [255 - inks.get(i).copied().unwrap_or(0); 3],
            Some(None) => [255; 3],
            None => {
                let lut = self.cmyk.get_or_init(|| {
                    let visible = self.proof.separations.as_deref().unwrap_or_default();
                    CmykLut::build(|inks| {
                        let [x, y, z] = lab::srgb_to_xyz(visible_process_srgb(&self.cms, &self.proof, visible, inks));
                        [x, y, z, 0.0]
                    })
                });
                let [x, y, z, _] = lut.apply(inks.map(|v| v as f32 / 255.0));
                lab::xyz_to_srgb([x, y, z]).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            }
        }
    }
}

static IMAGE_PLATES: Mutex<Option<Arc<ImagePlates>>> = Mutex::new(None);
static IMAGE_PLATES_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The tables placed images show with in Separations Preview, `None` when it's off.
pub(crate) fn image_plates(opts: &RenderOptions) -> Option<Arc<ImagePlates>> {
    let proof = opts.proof.as_ref().filter(|p| p.separations.is_some())?;
    let c = cms::active();
    let profiles = cms::profiles_generation();
    let mut slot = IMAGE_PLATES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = &*slot
        && Arc::ptr_eq(&p.cms, &c)
        && p.profiles == profiles
        && p.proof == *proof
    {
        return Some(p.clone());
    }
    let generation = IMAGE_PLATES_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let p = Arc::new(ImagePlates::build(c, profiles, proof.clone(), generation));
    *slot = Some(p.clone());
    Some(p)
}

/// The key [`plate_image`] gives image pixels `key` shown with `plates`.
pub(crate) fn plate_key(key: &ImageKey, plates: &ImagePlates) -> ImageKey {
    ImageKey { plates: plates.generation, ..key.clone() }
}

/// Image pixels `pm` (cached as `key`, from `blob` when it is a document's image as it is) as the
/// visible plates of `plates` print them, cached with the images, and their key. A CMYK image
/// shows its own inks; other pixels are separated.
pub(crate) fn plate_image(key: &ImageKey, pm: &Arc<Pixmap>, blob: Option<&ImageBlob>, plates: &ImagePlates, touch: bool) -> (ImageKey, Arc<Pixmap>) {
    let want = plate_key(key, plates);
    let hit = {
        let mut s = images::store();
        if touch { s.get(&want) } else { s.peek(&want) }
    };
    if let Some(p) = hit {
        return (want, p);
    }
    let size = (pm.width() as u32, pm.height() as u32);
    let cmyk = key.luts.0 != 0 && key.adjust.is_none() && !key.grey;
    let inks = blob.filter(|_| cmyk).and_then(ImageBlob::cmyk).filter(|i| (i.width, i.height) == size);
    let mut out = Pixmap::new(pm.width(), pm.height());
    // Runs of one colour convert once.
    if let Some(inks) = &inks {
        let mut last: Option<([u8; 4], [u8; 3])> = None;
        for (o, p) in out.data_mut().iter_mut().zip(inks.data.as_chunks::<4>().0) {
            let [r, g, b] = match last {
                Some((k, v)) if k == *p => v,
                _ => {
                    let v = plates.show_inks(*p);
                    last = Some((*p, v));
                    v
                }
            };
            *o = PremulRgba8 { r, g, b, a: 255 };
        }
    } else {
        let mut last: Option<(PremulRgba8, PremulRgba8)> = None;
        for (o, s) in out.data_mut().iter_mut().zip(pm.data()) {
            let a = s.a;
            if a == 0 {
                continue;
            }
            *o = match last {
                Some((k, v)) if k == *s => v,
                _ => {
                    let un = |v: u8| (v as f32 / a as f32).min(1.0);
                    let [r, g, b] = plates.rgb.apply([un(s.r), un(s.g), un(s.b)]).map(|v| (v.clamp(0.0, 1.0) * a as f32).round() as u8);
                    let v = PremulRgba8 { r, g, b, a };
                    last = Some((*s, v));
                    v
                }
            };
        }
    }
    let pm = images::store().insert(want.clone(), Arc::new(out));
    (want, pm)
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
            map_node_colors(Arc::make_mut(a), &mut |col, sw| plate_color(self.doc, c, proof, visible, col, sw));
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
