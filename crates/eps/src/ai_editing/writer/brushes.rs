//! Brushes: the document's brushes as the `.ai` reader reads them ([`crate::import`]'s `.ai` reader,
//! `brush.rs`), and the brush filter of a brushed stroke in an art style (`style.rs`).
//!
//! Each brush is a plug-in object (`%AI8_BeginPluginObject`, the plug-in's name, the brush's name,
//! its parameters as strings that `-` continues and `.` ends, `%AI8_EndPluginObject`), after the
//! brush manager's order of them all, in a non-printing block after the symbols. The art of a
//! Scatter or Art brush is a `%AI8_BeginBrushPattern` section (its name; a group of the art's frame,
//! an unpainted rectangle at the art's geometric bounds, and the art, or the art alone when it has no
//! width or height; its end) in a non-printing block before the symbols. Each brush is checked with
//! the reader's own reading ([`crate::import::read_brush_object`]) before it is written: one the
//! reader would read as another brush is left out.

use std::collections::HashSet;
use std::fmt::Write;
use std::sync::Arc;

use vectorcraft_brush::{ArtBrush, ArtScale, Brush, BrushKind, Calligraphic, Colorization, Direction, Scatter, Variation};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, PathData, SubPath};

use super::{Writer, Xy};
use crate::ai_ops;
use crate::ps;

pub(in crate::ai_editing) const BRUSHES: &str = "Pattern and Bristle brushes, Art brushes that don't run their art from left to right, that overlap or whose width follows the pen, Scatter and Art brushes in Tints and Shades or Hue Shift, brushes whose art holds an empty path or group or more than paths, compound paths and groups with one solid fill and one plain stroke each, brushes whose names hold a slash or repeat an earlier brush's name, brushes that would open with other settings, brushes past the number or the amount of art one file's brushes can hold, and every brush on a large canvas are left out of the Brushes panel in the editing data for other apps";

/// Most brushes written (the reader's limit).
const MAX_BRUSHES: usize = 4096;

/// Most bytes of a plug-in object's parameters in one of its strings.
const CHUNK: usize = 60;

/// A brush written as a plug-in object.
pub(super) struct BrushOut {
    pub brush: Brush,
    pub plugin: &'static str,
    pub params: String,
    /// The art its parameters name, as the reader reads it.
    pub art: Option<Node>,
    /// Its art section to write ([`Writer::brush_blocks`] writes it).
    section: Option<SectionArt>,
    /// A path whose art style names it at its own settings is read with the brush on its stroke.
    pub live: bool,
}

/// A brush's art section to write: its name, the art and the space it is written in
/// ([`Writer::brush_art_section`]).
type SectionArt = (String, Node, Affine);

/// The brushes written and their art.
#[derive(Default)]
pub(super) struct Brushes {
    /// The document's brushes have been worked out.
    done: bool,
    pub written: Vec<BrushOut>,
    /// The art sections named so far.
    sections: usize,
    /// A brush of the document was left out.
    left_out: bool,
    /// How many objects were written with a brush on their stroke.
    pub live_written: usize,
    /// The brushes written as the reader reads them, for checking art styles; made once, when
    /// the first style is checked ([`Writer::brushed_style_reads`]).
    read: Option<crate::import::BrushesRead>,
}

/// A number with at most six decimals, as brush parameters hold factors.
fn num6(s: &mut String, v: f64) {
    let v = if v.is_finite() { (v * 1e6).round() / 1e6 } else { 0.0 };
    if v == v.trunc() && v.abs() < 1e15 {
        let _ = write!(s, "{}", v as i64);
        return;
    }
    let start = s.len();
    let _ = write!(s, "{v:.6}");
    let keep = s[start..].trim_end_matches('0').trim_end_matches('.').len();
    s.truncate(start + keep);
}

/// `vs` as [`num6`] numbers separated by spaces.
fn nums6(vs: &[f64]) -> String {
    let mut s = String::new();
    for (i, v) in vs.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        num6(&mut s, *v);
    }
    s
}

/// The code a brush's parameters give how a value varies, in the order of the Brush Options menu.
fn code(v: Variation) -> f64 {
    match v {
        Variation::Fixed => 0.0,
        Variation::Random => 1.0,
        Variation::Pressure => 2.0,
        Variation::StylusWheel => 3.0,
        Variation::Tilt => 4.0,
        Variation::Bearing => 5.0,
        Variation::Rotation => 6.0,
    }
}

/// The colorization code of Scatter and Art brushes' parameters: `None` for one the reader doesn't read.
fn colorization_code(c: Colorization) -> Option<f64> {
    match c {
        Colorization::None => Some(0.0),
        Colorization::Tints => Some(1.0),
        Colorization::TintsAndShades | Colorization::HueShift { .. } => None,
    }
}

/// A Calligraphic brush's parameters: 1, the diameter, roundness and angle as (low, high) pairs,
/// the codes of how the diameter, roundness and angle vary, and 0.
fn calligraphic_params(c: &Calligraphic) -> String {
    let [angle, roundness, size] = [c.angle, c.roundness, c.size];
    let [av, rv, dv] = c.variation;
    let [am, rm, dm] = c.modes().map(code);
    nums6(&[1.0, size - dv, size + dv, roundness - rv, roundness + rv, angle - av, angle + av, dm, rm, am, 0.0])
}

/// A Scatter brush's parameters, its art the section `art`: `None` for a colorization the reader
/// doesn't read.
fn scatter_params(s: &Scatter, art: &str) -> Option<String> {
    let f = |v: f64| v / 100.0;
    let m = s.modes().map(code);
    let numbers = [
        colorization_code(s.colorization)?,
        f(s.size.0),
        f(s.size.1),
        m[0],
        f(s.spacing.0),
        f(s.spacing.1),
        m[1],
        f(s.scatter.0),
        f(s.scatter.1),
        m[2],
        s.rotation.0,
        s.rotation.1,
        m[3],
        f64::from(u8::from(s.rotation_relative_to_path)),
        0.0,
    ];
    Some(format!("2 /{art}/ 5 0 0 0 / {}", nums6(&numbers)))
}

/// An Art brush's settings as its parameters' numbers 2 to 15 give them and a brushed stroke's filter
/// holds them ([`ART_SETTINGS`]): `None` for options the reader doesn't read (it reads an Art brush
/// that runs its art from left to right without overlap, at a fixed width, in Tints or keeping its
/// colors). The guides of Stretch Between Guides are the lengths of the art before the first and
/// after the second, measured before the art is flipped along the path.
fn art_settings(a: &ArtBrush) -> Option<[f64; 14]> {
    if a.direction != Direction::LeftToRight || a.width_mode.is_some() || a.overlap {
        return None;
    }
    let length = a.art.geometric_bounds().map_or(0.0, |b| b.width());
    let (proportional, segmented, before, after) = match a.scale {
        ArtScale::Stretch => (0.0, 0.0, 0.0, 0.0),
        ArtScale::Proportional => (1.0, 0.0, 0.0, 0.0),
        ArtScale::BetweenGuides { start, end } if start < end => {
            let (before, after) = (start * length, (1.0 - end) * length);
            let (before, after) = if a.flip_along { (after, before) } else { (before, after) };
            (0.0, 1.0, before, after)
        }
        ArtScale::BetweenGuides { .. } => return None,
    };
    let scaling = a.width / 100.0;
    let flip = |on: bool| f64::from(u8::from(on));
    Some([
        colorization_code(a.colorization)?,
        scaling,
        0.0,
        1.0,
        flip(a.flip_along),
        flip(a.flip_across),
        proportional,
        scaling,
        scaling,
        0.0,
        segmented,
        before,
        after,
        1.0,
    ])
}

/// The entries of a brushed stroke's filter that hold an Art brush's settings, in the order of
/// [`art_settings`], with how each is written.
const ART_SETTINGS: [(&str, &str); 14] = [
    ("Colorization", "Int"),
    ("Scaling", "Real"),
    ("Spacing", "Real"),
    ("Adjustment", "Int"),
    ("Counterclockwise", "Int"),
    ("UpsideDown", "Int"),
    ("Proportional", "Bool"),
    ("MinScaling", "Real"),
    ("MaxScaling", "Real"),
    ("ScaleYDependsOn", "Int"),
    ("Segmented", "Bool"),
    ("StartSegmentLength", "Real"),
    ("EndSegmentLength", "Real"),
    ("FoldAdjustment", "Bool"),
];

/// An Art brush's parameters, its art the section `art`: `None` for options the reader doesn't read
/// ([`art_settings`]).
fn art_params(a: &ArtBrush, art: &str) -> Option<String> {
    let settings = art_settings(a)?;
    let mut numbers = vec![1.0];
    numbers.extend(settings);
    Some(format!("2 / {art}/ / / / / 0 0/ 5 0 0 0 /  {}", nums6(&numbers)))
}

/// Is `art` written as the reader reads it back, within a brush: paths and compound paths with
/// points and groups with members (not clipping, without paint of their own), each opaque in the
/// normal blending mode, without effects or opacity masks, with at most one fill and one plain
/// stroke (no brush, width profile, arrowheads, alignment off the path or dashes fitted to corners)
/// over it, each in a color or none? A path without points isn't written ([`Writer::plain_object`]).
fn plain_art(art: &Node) -> bool {
    let mut plain = true;
    art.walk(&mut |n| {
        let kind = match &n.kind {
            NodeKind::Path { path, guide: false, clipping: false, .. } => path.subpaths.iter().any(|sp| !sp.anchors.is_empty()),
            NodeKind::Compound { .. } => super::compound_path(n).subpaths.iter().any(|sp| !sp.anchors.is_empty()),
            NodeKind::Group { children, clip: false } => !children.is_empty(),
            _ => false,
        };
        let items = &n.appearance.items;
        let fills = items.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).count();
        let strokes = items.iter().filter(|i| matches!(i, AppearanceItem::Stroke(_))).count();
        let order = !matches!(items.as_slice(), [AppearanceItem::Stroke(_), AppearanceItem::Fill(_)]);
        let quiet = items.iter().all(|i| {
            i.effects().is_empty()
                && i.opacity() >= 1.0
                && i.blend() == BlendMode::Normal
                && matches!(i.paint(), Paint::Solid { .. } | Paint::None)
                && !matches!(i, AppearanceItem::Stroke(s) if !vectorcraft_effects::stroke::is_plain(s))
        });
        // A group's own fills and strokes aren't written.
        let group_paint = matches!(n.kind, NodeKind::Group { .. }) && items.iter().any(|i| !i.paint().is_none());
        plain &= kind
            && !group_paint
            && fills <= 1
            && strokes <= 1
            && order
            && quiet
            && n.appearance.effects.is_empty()
            && n.extras().mask.is_none()
            && n.opacity >= 1.0
            && n.blend == BlendMode::Normal;
    });
    plain
}

/// `art` (checked by [`plain_art`]) with its structure as the reader reads it back. A path, or a
/// compound path with the subpaths of all its members as one path ([`super::compound_path`]), is
/// written as one path when it has one subpath with points and as a compound path when it has more
/// ([`Writer::plain_object`]); the reader reads that compound path as one path for each subpath,
/// each with the object's paint.
fn as_read(art: &mut Node) {
    let (subpaths, rule) = match &art.kind {
        NodeKind::Path { path, rule, .. } => (path.subpaths.clone(), *rule),
        NodeKind::Compound { rule, .. } => (super::compound_path(art).subpaths, *rule),
        _ => {
            if let Some(children) = art.children_mut() {
                for c in children.iter_mut() {
                    as_read(Arc::make_mut(c));
                }
            }
            return;
        }
    };
    let mut subpaths: Vec<SubPath> = subpaths.into_iter().filter(|sp| !sp.anchors.is_empty()).collect();
    let path = |sp: SubPath| {
        let mut p = Node::path(art.id, PathData::single(sp), Appearance::default());
        p.appearance = art.appearance.clone();
        p
    };
    match subpaths.len() {
        0 => {}
        1 => {
            let Some(sp) = subpaths.pop() else { return };
            art.kind = path(sp).kind;
            if let NodeKind::Path { rule: r, .. } = &mut art.kind {
                *r = rule;
            }
        }
        _ => {
            let children = subpaths.into_iter().map(|sp| Arc::new(path(sp))).collect();
            art.kind = NodeKind::Compound { children, rule };
        }
    }
}

/// Do brushes `a` and `b` have the same settings, within the written numbers' precision, whatever
/// their art and however their modes are given?
fn same_settings(a: &Brush, b: &Brush) -> bool {
    let norm = |b: &Brush| -> serde_json::Value {
        let mut b = b.clone();
        let none = || Node::group(NodeId(0), vec![]);
        match &mut b.kind {
            BrushKind::Calligraphic(c) => c.modes = Some(c.modes()),
            BrushKind::Scatter(s) => {
                s.modes = Some(s.modes());
                s.art = none();
            }
            BrushKind::Art(a) => a.art = none(),
            BrushKind::Pattern(_) | BrushKind::Bristle(_) => {}
        }
        serde_json::to_value(&b).unwrap_or(serde_json::Value::Null)
    };
    super::styles::near(&norm(a), &norm(b))
}

/// A plug-in object: its plug-in's name, its name, and `params` in strings of at most [`CHUNK`] bytes.
fn plugin_object(plugin: &str, name: &str, params: &str) -> String {
    let mut s = format!("%AI8_BeginPluginObject\n{}\n{}\n", ps::bytes(plugin.as_bytes()), ps::bytes(name.as_bytes()));
    let chunks: Vec<&[u8]> = params.as_bytes().chunks(CHUNK).collect();
    if chunks.is_empty() {
        s.push_str("() .\n");
    }
    for (i, c) in chunks.iter().enumerate() {
        let _ = writeln!(s, "{} {}", ps::bytes(c), if i + 1 == chunks.len() { "." } else { "-" });
    }
    s.push_str("%AI8_EndPluginObject\n");
    s
}

impl Writer<'_> {
    /// Work out, once, which of the document's brushes are written: each one the reader reads back
    /// with its settings. Their art is written with the brushes ([`Self::brush_blocks`]).
    pub(super) fn brush_set(&mut self) {
        if self.brush_set.done {
            return;
        }
        self.brush_set.done = true;
        let doc = self.doc;
        let lib = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc)).clone();
        if !self.brushes_live {
            self.brush_set.left_out = !lib.is_empty();
            return;
        }
        // A name is taken by a brush written, as the reader takes it by a brush it keeps. A brush after
        // another of its name isn't the one a path of that name is drawn with, so no path is written
        // with it on its stroke.
        let mut names: HashSet<String> = HashSet::new();
        let mut seen: HashSet<String> = HashSet::new();
        for b in lib {
            let fits = !b.name.contains('/') && !names.contains(&b.name) && self.brush_set.written.len() < MAX_BRUSHES;
            let first = seen.insert(b.name.clone());
            match fits.then(|| self.brush_out(&b)).flatten() {
                Some(mut out) => {
                    names.insert(b.name.clone());
                    out.live &= first;
                    self.brush_set.written.push(out);
                }
                None => self.brush_set.left_out = true,
            }
        }
        // Read together, as one file's: the reader copies only so much art into one file's brushes.
        let objects: Vec<(&str, &str, &str, Option<&Node>)> =
            self.brush_set.written.iter().map(|b| (b.plugin, b.brush.name.as_str(), b.params.as_str(), b.art.as_ref())).collect();
        let kept = crate::import::brushes_kept(&objects);
        if kept.iter().any(|k| !k) {
            self.brush_set.left_out = true;
            let mut keep = kept.into_iter();
            self.brush_set.written.retain(|_| keep.next().unwrap_or(false));
        }
    }

    /// Brush `b` as it is written, with its art section's name, art and placement
    /// ([`Self::brush_art_section`] writes it): `None` when the reader wouldn't read it back.
    fn brush_out(&mut self, b: &Brush) -> Option<BrushOut> {
        let plugin = crate::import::brush_plugin_name(&b.kind);
        let section = format!("Brush Art {}", self.brush_set.sections + 1);
        // The art as the reader reads it: a section is read with y down, and an Art brush's art is
        // turned over after that (its section's y up is the right of the path).
        let (params, art) = match &b.kind {
            BrushKind::Calligraphic(c) => (calligraphic_params(c), None),
            BrushKind::Scatter(s) => (scatter_params(s, &section)?, Some((s.art.clone(), Affine::FLIP_Y))),
            BrushKind::Art(a) => (art_params(a, &section)?, Some((a.art.clone(), Affine::IDENTITY))),
            BrushKind::Pattern(_) | BrushKind::Bristle(_) => return None,
        };
        let read_art = match &art {
            Some((a, to_section)) => {
                if !plain_art(a) {
                    return None;
                }
                let mut read = a.clone();
                if *to_section == Affine::IDENTITY {
                    read.transform(Affine::FLIP_Y, false);
                }
                as_read(&mut read);
                Some(read)
            }
            None => None,
        };
        let read = crate::import::read_brush_object(plugin, &b.name, &params, read_art.as_ref())?;
        if !same_settings(&read.brush, b) {
            return None;
        }
        if art.is_some() {
            self.brush_set.sections += 1;
        }
        let section = art.map(|(a, to_section)| (section, a, to_section));
        Some(BrushOut { brush: b.clone(), plugin, params, art: read_art, section, live: read.live })
    }

    /// A brush's art section `name`: `art` written with `to_section` taking the document's space to
    /// the section's (y up), from the state a definition starts in. Its objects are counted as the
    /// reader counts them ([`Self::take_objects`]: the group, the frame and the art). `None` when
    /// writing it warns: what it took is given back, and when it didn't fit within the object limit,
    /// the limit's warning is kept.
    fn brush_art_section(&mut self, name: &str, art: &Node, to_section: Affine) -> Option<String> {
        let before = self.charges();
        // The art's own warnings, apart from those the document already has (a warning is kept once).
        let warnings = std::mem::take(&mut self.warnings);
        let saved = (
            std::mem::take(&mut self.body),
            (self.to_art, self.grad_space, self.canvas),
            self.state.take(),
            self.gs.replace(Xy::OPAQUE),
            self.clip.take(),
            (self.block_start, self.block_end),
            // The art is the brush's own: not on `%_` lines, in a drawn look or in a symbol that
            // first uses the brush.
            (std::mem::take(&mut self.hidden), std::mem::take(&mut self.in_look), std::mem::take(&mut self.in_symbol)),
        );
        (self.to_art, self.grad_space, self.canvas, self.block_end) = (to_section, to_section, to_section, usize::MAX);
        self.state(false, false);
        // The art in a group after its frame, an unpainted rectangle around it. Art without width or
        // height (a straight line) is written alone: the reader keeps a frame of no area as art.
        let frame = art
            .geometric_bounds()
            .filter(|b| b.width() > 0.0 && b.height() > 0.0)
            .and_then(|b| vectorcraft_geom::shapes::rectangle(b).subpaths.first().cloned());
        match frame {
            Some(frame) if self.take_objects(2) => {
                self.line("u\n0 XR");
                ai_ops::subpath(&mut self.body, to_section, &frame);
                self.line("n");
                self.nest += 1;
                self.node(art);
                self.nest -= 1;
                self.line("U");
            }
            Some(_) => {}
            None => {
                self.nest += 1;
                self.node(art);
                self.nest -= 1;
            }
        }
        let body;
        (
            body,
            (self.to_art, self.grad_space, self.canvas),
            self.state,
            self.gs,
            self.clip,
            (self.block_start, self.block_end),
            (self.hidden, self.in_look, self.in_symbol),
        ) = saved;
        let text = std::mem::replace(&mut self.body, body);
        if !std::mem::replace(&mut self.warnings, warnings).is_empty() {
            self.give_back(before, &text);
            if self.left_out_since(before) {
                self.warn(super::OBJECT_LIMIT);
            }
            return None;
        }
        self.keep_since(before);
        Some(format!("%AI8_BeginBrushPattern\n{}\n{text}%AI8_EndBrushPattern\n", ps::bytes(name.as_bytes())))
    }

    /// The written brush named `name` that a path can be written with live.
    pub(super) fn live_brush(&self, name: &str) -> Option<&BrushOut> {
        self.brush_set.written.iter().find(|b| b.live && b.brush.name == name)
    }

    /// The basic filter of stroke `st` with the brush it names: the brush's plug-in as the filter,
    /// its settings for the object, the stroke's style and the brush by name. `None` when the brush
    /// isn't written live.
    pub(super) fn brush_filter(&mut self, s: &mut String, st: &StrokeLayer, visible: bool) -> Option<()> {
        let name = st.brush.as_deref()?;
        self.brush_set();
        let (plugin, kind) = self.live_brush(name).map(|b| (b.plugin, b.brush.kind.clone()))?;
        if !matches!(st.paint, Paint::Solid { .. }) || st.overprint {
            return None;
        }
        let ops = self.color_ops(&st.paint, true);
        let mut d = String::new();
        let title = match &kind {
            BrushKind::Calligraphic(c) => {
                let modes = c.modes().map(code);
                for (key, value, variation, mode) in [
                    ("Diameter", c.size, c.variation[2], modes[2]),
                    ("Roundness", c.roundness, c.variation[1], modes[1]),
                    ("Angle", c.angle, c.variation[0], modes[0]),
                ] {
                    let _ = writeln!(d, "{} /Real ({key}) ,", ps::num(value));
                    let _ = writeln!(d, "{} /Real ({key}Variation) ,", ps::num(variation));
                    let _ = writeln!(d, "{} /Int ({key}DependsOn) ,", mode as i64);
                }
                d.push_str("0 /Int (AngleRelativeTo) ,\n");
                "(Calligraphic Brush) /Title ,\n"
            }
            BrushKind::Art(a) => {
                d.push_str("15 /Int (Adobe Effect Expand Before Version) ,\n"); // brand-ok: the entry's name in the files
                for ((key, kind), value) in ART_SETTINGS.iter().zip(art_settings(a)?) {
                    let _ = writeln!(d, "{} /{kind} ({key}) ,", nums6(&[value]));
                }
                "(Art Brush) /Title ,\n"
            }
            _ => return None,
        };
        super::styles::basic(s, plugin, true, visible);
        s.push_str(title);
        s.push_str("2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n");
        s.push_str(&d);
        s.push_str("/StrokeStyle : 0 R\n");
        s.push_str(&ops);
        s.push_str("0 1 0 0 0 Xy\n");
        ai_ops::line_style(s, st);
        s.push_str("0 XR\n1 0 Xd\n/Def ; \n (StrokeStyle) ,\n");
        let quoted = ps::bytes(name.as_bytes());
        let _ =
            write!(s, "/GObjRef : {quoted} {} /PluginObject ; (Brush) ,\n{quoted} /UnicodeString (DisplayString) ,\n", ps::bytes(plugin.as_bytes()));
        s.push_str(super::styles::END_BASIC);
        Some(())
    }

    /// Does the reader read the brushed strokes of style text `text` with their brushes on them?
    pub(super) fn brushed_style_reads(&mut self, text: &str) -> bool {
        self.brush_set();
        let section = super::styles::known_style("x", text);
        let written = &self.brush_set.written;
        let read = self.brush_set.read.get_or_insert_with(|| {
            let brushes: Vec<(&str, &str, &str, Option<&Node>)> =
                written.iter().map(|b| (b.plugin, b.brush.name.as_str(), b.params.as_str(), b.art.as_ref())).collect();
            crate::import::BrushesRead::new(&brushes)
        });
        read.style_reads(section.as_bytes(), "x")
    }

    /// The brushes' art in a non-printing block, then the brush manager's order and the brushes in
    /// two more, when the document has brushes of its own or an object is written with a brush on
    /// its stroke (a file without brushes opens with a new document's). Warns when a brush is left
    /// out.
    pub(in crate::ai_editing) fn brush_blocks(&mut self) -> (String, String) {
        let stored = self.doc.unknown.contains_key(vectorcraft_brush::DOC_KEY);
        if !stored && self.brush_set.live_written == 0 {
            return (String::new(), String::new());
        }
        self.brush_set();
        // The art is written now that the file holds it, after the layers' objects. A brush whose art
        // doesn't fit is left out; a path written with it on its stroke opens as its drawn look.
        let mut sections = String::new();
        for mut b in std::mem::take(&mut self.brush_set.written) {
            if let Some((name, art, to_section)) = b.section.take() {
                let Some(text) = self.brush_art_section(&name, &art, to_section) else {
                    self.brush_set.left_out = true;
                    continue;
                };
                sections.push_str(&text);
            }
            self.brush_set.written.push(b);
        }
        if self.brush_set.left_out {
            self.warn(BRUSHES);
        }
        // On a large canvas an empty panel is still written: the manager alone, with no art to scale.
        let empty = self.brushes.as_ref().is_some_and(|l| l.is_empty());
        if !self.brushes_live && !(stored && empty) {
            return (String::new(), String::new());
        }
        let art = if sections.is_empty() { String::new() } else { format!("%AI5_Begin_NonPrinting\nNp\n{sections}%AI5_End_NonPrinting--\n") };
        let manager = crate::import::BRUSH_MANAGER;
        let order: String = self.brush_set.written.iter().map(|b| format!(" {}/ {}/", b.plugin, b.brush.name)).collect();
        let mut objects = format!("%AI5_Begin_NonPrinting\nNp\n{}%AI5_End_NonPrinting--\n", plugin_object(manager, manager, &order));
        if !self.brush_set.written.is_empty() {
            objects.push_str("%AI5_Begin_NonPrinting\nNp\n");
            for b in &self.brush_set.written {
                objects.push_str(&plugin_object(b.plugin, &b.brush.name, &b.params));
            }
            objects.push_str("%AI5_End_NonPrinting--\n");
        }
        (art, objects)
    }
}
