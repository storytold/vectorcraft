//! Brush definitions (not publicly documented): plug-in objects in the setup, as written in files
//! that users own and sent us, brush library files among them (not presets installed with any
//! other app).
//!
//! - Each object is a `%AI8_BeginPluginObject` section of strings: the plug-in's name, the brush's
//!   name, then its parameters, split into strings that `-` continues and `.` ends.
//! - One object, the brush manager's order, has the order of the Brushes panel: its parameters
//!   are the plug-in name and brush name of each brush, each one after a `/` and a space.
//! - A Calligraphic brush's parameters are 11 numbers: the first is 1; the diameter, roundness and
//!   angle as (low, high) pairs; how the diameter, roundness and angle vary; a last number. The
//!   codes follow the order of the Brush Options menu: 0 Fixed, 1 Random, 2 Pressure, 3 Stylus
//!   Wheel, 4 Tilt, 5 Bearing, 6 Rotation. A user's 6D Art Pen brushes (#372) vary the diameter
//!   with 2, the roundness with 5 and the angle with 6, the angle over a whole turn, as the barrel
//!   rotation turns the nib. VectorCraft reads no stylus wheel: 3 is read as Random when the value
//!   varies, as an unknown code is.
//! - A path with a Calligraphic brush is a stroke in its art style ([`super::style`]): the filter
//!   `(Adobe Calligraphic Brush Tool)`, its stroke's paint and weight (the weight scales the brush:
//!   1 pt draws it at its own size), the brush by name (`/GObjRef : (name) (plug-in)
//!   /PluginObject`), and the brush's settings for this path as entries: `Diameter`, `Roundness`,
//!   `Angle`, their `…Variation` either side and `…DependsOn` codes, and `AngleRelativeTo` (0 the
//!   page). Samples of the pen may follow a path's own lines on `%_` lines (`(hex) Xt n XD`). They
//!   aren't read, so such a path keeps its drawn look when a setting varies with the pen.
//! - A path with an Art brush is a stroke in its art style too: the filter
//!   `(Adobe ArtOnPath Brush Tool)`, its stroke's paint and weight (the weight scales the art across
//!   the path), the brush by name, and its settings for this path as entries: `Scaling`,
//!   `MinScaling` and `MaxScaling` (the width as a factor), `Colorization`, `Proportional`,
//!   `Segmented` with `StartSegmentLength` and `EndSegmentLength`, `UpsideDown`, `Counterclockwise`,
//!   `ScaleYDependsOn`, `Spacing`, `Adjustment` and `FoldAdjustment` (1 is
//!   [`vectorcraft_brush::ArtBrush`]'s `overlap: false`).
//! - A Scatter, Art or Pattern brush's parameters are fields split by `/`: a number, the art's
//!   section names (one for Scatter, five for Art and Pattern, empty where there is none, each
//!   after a space for Art and Pattern), more fields, and last 15 numbers. Older files write
//!   brushes with first number 1, one field fewer before the numbers and 8 numbers; an Art brush
//!   with 8 numbers, whatever its first number, is read from its art's name alone (its numbers
//!   aren't known, so it keeps the defaults and its paths their drawn look). A Pattern brush needs
//!   only its first 3 numbers (its scale is the third).
//!   - Scatter: the size (2, 3), spacing (5, 6) and scatter (8, 9) as (low, high) factors of the
//!     art, the rotation in degrees (11, 12), how each varies (4, 7, 10, 13) and whether the
//!     rotation follows the path (14), and 1 Tints (1).
//!   - Art: 1 Tints (2).
//!   - Pattern: its scale as a factor (3).
//! - A Bristle brush's parameters start with 8 numbers: two codes, then the stiffness, bristle
//!   thickness, density, paint opacity and bristle length as fractions and the size in
//!   millimeters.
//! - The art is a `%AI8_BeginBrushPattern` section: its name as a string, then the art (read by
//!   [`super::read`]), usually in a group after its frame, an unpainted rectangle around it. A frame
//!   at the art's bounds isn't kept ([`without_frame`]); a wider one is.
//!
//! What isn't read keeps its default: the Art brush's other options, the pattern brush's tiles
//! (which slot is which tile isn't known) and the colorization of the other brushes, which is left
//! at None so the art keeps its own colors.

use std::collections::{HashMap, HashSet};

use kurbo::Affine;
use vectorcraft_brush::{ArtBrush, Bristle, Brush, BrushKind, Calligraphic, Colorization, Scatter, Variation};
use vectorcraft_color::Paint;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};

use super::lex::{Lexer, Tok};
use super::obj::Names;

/// The brush manager's order: the plug-in name of its object.
pub(super) const MANAGER: &str = "Adobe Brush Manager Order"; // brand-ok: the plug-in name the files use

/// The plug-in names of the brush kinds.
const TOOLS: [(&str, Tool); 5] = [
    ("Adobe Calligraphic Brush Tool", Tool::Calligraphic), // brand-ok: the plug-in name the files use
    ("Adobe Scatter Brush Tool", Tool::Scatter),           // brand-ok: the plug-in name the files use
    ("Adobe ArtOnPath Brush Tool", Tool::Art),             // brand-ok: the plug-in name the files use
    ("Adobe PatternOnPath Brush Tool", Tool::Pattern),     // brand-ok: the plug-in name the files use
    ("Adobe dBrush Brush Tool", Tool::Bristle),            // brand-ok: the plug-in name the files use
];

/// Most brushes read from one file.
pub(super) const MAX_BRUSHES: usize = 4096;
/// Most brush art sections read from one file (a pattern brush has up to five).
pub(super) const MAX_BRUSH_ART: usize = 5 * MAX_BRUSHES;
/// Most named colors kept from one brush art section, and brought in from the art of one file's
/// brushes, each name once per section: the rest are left out.
pub(super) const MAX_BRUSH_ART_COLORS: usize = 20_000;
/// Most [`art_weight`] copied into the brushes of one file, each brush counting its own copy.
const MAX_BRUSH_ART_NODES: usize = 250_000;

/// Most bytes of a plug-in object's parameters read (the brush manager's order of [`MAX_BRUSHES`]
/// brushes with long names fits): the rest is left out.
const MAX_PARAMS: usize = 1 << 20;

/// Millimeters → points.
const MM: f64 = 72.0 / 25.4;

/// A brush's kind, as its plug-in name gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Calligraphic,
    Scatter,
    Art,
    Pattern,
    Bristle,
}

/// The kind of a brush whose object's plug-in name is `name`.
pub(super) fn tool(name: &str) -> Option<Tool> {
    TOOLS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// A brush as its object gives it: its definition, and the names of the art sections it needs.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BrushDraft {
    pub name: String,
    pub kind: DraftKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum DraftKind {
    Calligraphic(Calligraphic),
    /// The art is the section named `art`.
    Scatter {
        brush: Scatter,
        art: String,
    },
    /// The art is the section named `art`. `plain`: the brush stretches its art along the path,
    /// unflipped, at 100% in Tints (see [`art_is_plain`]), so [`Self::Art`]'s `brush` draws as the
    /// file's brush does.
    Art {
        brush: ArtBrush,
        art: String,
        plain: bool,
    },
    /// The five art sections the brush names, in its order (which is which tile isn't known), and
    /// its scale (%).
    Pattern {
        tiles: [Option<String>; 5],
        scale: f64,
    },
    Bristle(Bristle),
}

impl DraftKind {
    pub(super) fn tool(&self) -> Tool {
        match self {
            DraftKind::Calligraphic(_) => Tool::Calligraphic,
            DraftKind::Scatter { .. } => Tool::Scatter,
            DraftKind::Art { .. } => Tool::Art,
            DraftKind::Pattern { .. } => Tool::Pattern,
            DraftKind::Bristle(_) => Tool::Bristle,
        }
    }
}

/// Why a brush's object isn't read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BrushError {
    /// Fewer values than the kind has, or one that isn't a finite number.
    Values,
    /// No art section named.
    NoArt,
}

/// The strings of a plug-in object's section (`section`: what follows `%AI8_BeginPluginObject`):
/// its plug-in name, the bytes of its name and its parameters, joined, read by `names` without
/// notes. The name is `None` with one string, and the whole is `None` with none. A brush's name has
/// its note when the brush is read, and the names of its art when their art is read.
pub(super) fn plugin_strings(section: &[u8], names: &Names) -> Option<(String, Option<Vec<u8>>, String)> {
    let mut lex = Lexer::new(section);
    let mut strings = vec![];
    let mut params = vec![];
    while let Some(t) = lex.next_token() {
        if let Tok::Str(s) | Tok::Hex(s) = t.tok {
            if strings.len() < 2 {
                strings.push(s);
            } else {
                // Joined before they are text: a string may end inside a character.
                let room = MAX_PARAMS.saturating_sub(params.len());
                params.extend_from_slice(s.get(..room.min(s.len())).unwrap_or_default());
            }
        }
    }
    let mut strings = strings.into_iter();
    let kind = names.quiet(&strings.next()?);
    Some((kind, strings.next(), names.quiet(&params)))
}

/// The brush manager's order (`params`: its parameters): the kind and name of each brush. Pairs
/// whose plug-in name isn't a brush kind's are left out; `None` when there are none.
pub(super) fn parse_brush_manager(params: &str) -> Option<Vec<(Tool, String)>> {
    let fields: Vec<&str> = params.split('/').take(2 * MAX_BRUSHES).map(|f| f.strip_prefix(' ').unwrap_or(f)).collect();
    let mut out = vec![];
    for [kind, name] in fields.as_chunks::<2>().0 {
        if let Some(t) = tool(kind)
            && out.len() < MAX_BRUSHES
        {
            out.push((t, (*name).to_string()));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The numbers of `s` (split by white space): `None` when one isn't a finite number.
fn numbers(s: &str) -> Option<Vec<f64>> {
    s.split_whitespace().map(|w| w.parse::<f64>().ok().filter(|v| v.is_finite())).collect()
}

/// How a value varies, by its code: `None` for the stylus wheel and codes that aren't known.
fn variation(code: f64) -> Option<Variation> {
    match code {
        0.0 => Some(Variation::Fixed),
        1.0 => Some(Variation::Random),
        2.0 => Some(Variation::Pressure),
        4.0 => Some(Variation::Tilt),
        5.0 => Some(Variation::Bearing),
        6.0 => Some(Variation::Rotation),
        _ => None,
    }
}

/// [`variation`], or for another code, Random when the value varies and Fixed when it doesn't.
fn mode(code: f64, varies: bool) -> Variation {
    variation(code).unwrap_or(if varies { Variation::Random } else { Variation::Fixed })
}

/// A Calligraphic brush of angle, roundness and size `values`, each varying `variation` either side
/// as its code in `codes` says.
pub(super) fn calligraphic(values: [f64; 3], variation: [f64; 3], codes: [f64; 3]) -> Calligraphic {
    let [angle, roundness, size] = values;
    let modes = [0, 1, 2].map(|i| mode(codes[i], variation[i] != 0.0));
    Calligraphic { angle, roundness, size, variation, modes: Some(modes) }
}

/// The settings a path's art style gives its stroke's brush.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PathBrush {
    Calligraphic(Calligraphic),
    /// An Art brush whose art stretches along the path, at the brush's own width, in Tints.
    Art,
}

/// Does an Art brush draw `art` as the file does? It bends paths in solid colors: other objects
/// are left out of the pieces, a gradient or pattern stays where it was and takes no color from the
/// stroke, an effect isn't bent with its path, a stroke's outline isn't stretched along it, and art
/// with no length along the path draws nothing.
pub(super) fn art_bends(art: &Node) -> bool {
    let mut bends = art.geometric_bounds().is_some_and(|b| b.width() > 1e-9);
    art.walk(&mut |n| {
        bends &= matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Group { .. })
            && n.extras().mask.is_none()
            && n.appearance.effects.is_empty()
            && n.appearance.items.iter().all(|i| i.effects().is_empty() && !matches!(i, AppearanceItem::Stroke(s) if !s.paint.is_none()))
            && vectorcraft_doc::swatches::node_paints(n).all(|p| matches!(p, Paint::Solid { .. } | Paint::None));
    });
    bends
}

/// The art sections of `drafts`' Art brushes that an Art brush draws as the file does.
pub(super) fn bending_art(drafts: &[BrushDraft], art: &HashMap<String, Node>) -> HashSet<String> {
    let names: HashSet<&String> = drafts
        .iter()
        .filter_map(|d| match &d.kind {
            DraftKind::Art { art, .. } => Some(art),
            _ => None,
        })
        .collect();
    names.into_iter().filter(|n| art.get(*n).is_some_and(art_bends)).cloned().collect()
}

/// Does the brush `draft` draw as `settings`, the settings a path's art style gives it? A path whose
/// settings differ (Options of Selected Object) keeps its drawn look: a stroke names a brush, and
/// has no settings of its own. `bending`: [`bending_art`].
pub(super) fn draws_as(draft: &BrushDraft, settings: &PathBrush, bending: &HashSet<String>) -> bool {
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-3 * a.abs().max(b.abs()).max(1.0);
    match (&draft.kind, settings) {
        (DraftKind::Calligraphic(d), PathBrush::Calligraphic(c)) => {
            close(d.angle, c.angle)
                && close(d.roundness, c.roundness)
                && close(d.size, c.size)
                && d.variation.iter().zip(&c.variation).all(|(a, b)| close(*a, *b))
                && d.modes() == c.modes()
        }
        (DraftKind::Art { plain: true, art: name, .. }, PathBrush::Art) => bending.contains(name),
        _ => false,
    }
}

/// Are an Art brush's last 15 numbers those of a brush that stretches its art along the path,
/// unflipped, at 100% in Tints? The numbers aren't read otherwise: any other value keeps its paths'
/// drawn look.
fn art_is_plain(numbers: &[f64; 15]) -> bool {
    *numbers == [1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}

/// The brush named `name` of kind `kind` from its parameters `params`.
pub(super) fn parse_tool_object(kind: Tool, name: &str, params: &str) -> Result<BrushDraft, BrushError> {
    let kind = match kind {
        Tool::Calligraphic => {
            let n = numbers(params).ok_or(BrushError::Values)?;
            let [_, d0, d1, r0, r1, a0, a1, dm, rm, am, ..] = n.as_slice() else { return Err(BrushError::Values) };
            let mid = |a: f64, b: f64| (a + b) / 2.0;
            let half = |a: f64, b: f64| (b - a).abs() / 2.0;
            DraftKind::Calligraphic(calligraphic(
                [mid(*a0, *a1), mid(*r0, *r1), mid(*d0, *d1)],
                [half(*a0, *a1), half(*r0, *r1), half(*d0, *d1)],
                [*am, *rm, *dm],
            ))
        }
        Tool::Scatter | Tool::Art | Tool::Pattern => {
            let fields: Vec<&str> = params.split('/').collect();
            let slots = if kind == Tool::Scatter { 1 } else { 5 };
            if fields.len() < slots + 2 {
                return Err(BrushError::Values);
            }
            let tiles: Vec<Option<String>> = fields
                .iter()
                .skip(1)
                .take(slots)
                .map(|f| {
                    let f = f.strip_prefix(' ').unwrap_or(f);
                    (!f.is_empty()).then(|| f.to_string())
                })
                .collect();
            let n = fields.last().and_then(|f| numbers(f)).ok_or(BrushError::Values)?;
            let art = || tiles.first().cloned().flatten().ok_or(BrushError::NoArt);
            match kind {
                Tool::Scatter => {
                    let [_, p2, p3, p4, p5, p6, p7, p8, p9, p10, p11, p12, p13, p14, _] = n.as_slice() else { return Err(BrushError::Values) };
                    let pct = |a: f64, b: f64| (a * 100.0, b * 100.0);
                    let values = [pct(*p2, *p3), pct(*p5, *p6), pct(*p8, *p9), (*p11, *p12)];
                    let codes = [*p4, *p7, *p10, *p13];
                    let modes = [0, 1, 2, 3].map(|i| mode(codes[i], values[i].0 != values[i].1));
                    let [size, spacing, scatter, rotation] = values;
                    let brush = Scatter {
                        size,
                        spacing,
                        scatter,
                        rotation,
                        rotation_relative_to_path: *p14 == 1.0,
                        // The first of the 15 numbers: 1 is Tints.
                        colorization: if n.first() == Some(&1.0) { Colorization::Tints } else { Colorization::None },
                        modes: Some(modes),
                        ..Scatter::default()
                    };
                    DraftKind::Scatter { brush, art: art()? }
                }
                Tool::Art => {
                    // The older form's 8 numbers aren't known: its brush keeps the defaults and its
                    // paths their drawn look. The count tells the form, not the first number.
                    // Any other count isn't a form this reads.
                    let (colorization, plain) = if let Ok(n15) = <&[f64; 15]>::try_from(n.as_slice()) {
                        (if n15[1] == 1.0 { Colorization::Tints } else { Colorization::None }, art_is_plain(n15))
                    } else if n.len() == 8 {
                        (ArtBrush::default().colorization, false)
                    } else {
                        return Err(BrushError::Values);
                    };
                    DraftKind::Art { brush: ArtBrush { colorization, ..ArtBrush::default() }, art: art()?, plain }
                }
                _ => {
                    let [_, _, p3, ..] = n.as_slice() else { return Err(BrushError::Values) };
                    let tiles: [Option<String>; 5] = tiles.try_into().map_err(|_| BrushError::Values)?;
                    DraftKind::Pattern { tiles, scale: p3 * 100.0 }
                }
            }
        }
        Tool::Bristle => {
            let words: Vec<&str> = params.split_whitespace().take(8).collect();
            let n = numbers(&words.join(" ")).ok_or(BrushError::Values)?;
            let [_, _, stiffness, thickness, density, opacity, length, size] = n.as_slice() else { return Err(BrushError::Values) };
            DraftKind::Bristle(Bristle {
                stiffness: stiffness * 100.0,
                thickness: thickness * 100.0,
                density: density * 100.0,
                opacity: opacity * 100.0,
                length: length * 100.0,
                size: size * MM,
                ..Bristle::default()
            })
        }
    };
    Ok(BrushDraft { name: name.to_string(), kind })
}

/// The brushes of `drafts`, their art from the sections `art` (by name), in the brush manager's
/// `order` (brushes it doesn't list after those it does, in the order of their objects), each
/// kept within its ranges → the brushes and how many were left out: those whose art isn't read,
/// pattern brushes (which slot holds which tile isn't known), a second brush of the same name, and
/// brushes whose copies of art go past [`MAX_BRUSH_ART_NODES`].
pub(super) fn finish_brushes(drafts: Vec<BrushDraft>, art: &HashMap<String, Node>, order: Option<&[(Tool, String)]>) -> (Vec<Brush>, usize) {
    finish_brushes_within(drafts, art, order, MAX_BRUSH_ART_NODES)
}

/// What copying `art` costs: its nodes, the art of their opacity masks, and the anchors of their paths.
pub(super) fn art_weight(art: &Node) -> usize {
    let mut weight = 0usize;
    art.walk(&mut |n| {
        weight = weight.saturating_add(1 + n.path_data().map_or(0, |p| p.anchor_count()));
        if let Some(m) = &n.extras().mask {
            weight = weight.saturating_add(art_weight(&m.art));
        }
    });
    weight
}

/// Which of `drafts` [`finish_brushes`] makes into brushes, with [`MAX_BRUSH_ART_NODES`] of art.
pub(super) fn kept(drafts: &[BrushDraft], art: &HashMap<String, Node>) -> Vec<bool> {
    kept_brushes(drafts, art, MAX_BRUSH_ART_NODES)
}

/// Which of `drafts` become brushes, with at most `budget` of [`art_weight`] copied into them: not
/// a pattern brush, a brush whose art isn't read, a brush whose art would go past `budget`, or a
/// brush named as one kept before it.
pub(super) fn kept_brushes(drafts: &[BrushDraft], art: &HashMap<String, Node>, budget: usize) -> Vec<bool> {
    let mut copied = 0usize;
    let mut weights: HashMap<&str, usize> = HashMap::new();
    let mut names: HashSet<&str> = HashSet::new();
    drafts
        .iter()
        .map(|d| {
            let fits = match &d.kind {
                _ if names.contains(d.name.as_str()) => false,
                DraftKind::Calligraphic(_) | DraftKind::Bristle(_) => true,
                DraftKind::Pattern { .. } => false,
                DraftKind::Scatter { art: name, .. } | DraftKind::Art { art: name, .. } => art.get_key_value(name).is_some_and(|(key, a)| {
                    let weight = *weights.entry(key.as_str()).or_insert_with(|| art_weight(a));
                    let total = copied.saturating_add(weight);
                    let fits = total <= budget;
                    if fits {
                        copied = total;
                    }
                    fits
                }),
            };
            if fits {
                names.insert(&d.name);
            }
            fits
        })
        .collect()
}

/// [`finish_brushes`] with at most `budget` of [`art_weight`] copied into the brushes (each brush
/// that uses a section holds its own copy): a brush whose art would go past it is left out.
pub(super) fn finish_brushes_within(
    drafts: Vec<BrushDraft>,
    art: &HashMap<String, Node>,
    order: Option<&[(Tool, String)]>,
    budget: usize,
) -> (Vec<Brush>, usize) {
    let kept = kept_brushes(&drafts, art, budget);
    let copy = |name: &str| art.get(name).cloned();
    let mut left_out = 0;
    let mut made: Vec<Option<(Tool, Brush)>> = vec![];
    for (d, kept) in drafts.into_iter().zip(kept) {
        let tool = d.kind.tool();
        let kind = match d.kind {
            _ if !kept => None,
            DraftKind::Calligraphic(c) => Some(BrushKind::Calligraphic(c)),
            DraftKind::Scatter { brush, art: name } => copy(&name).map(|a| BrushKind::Scatter(Scatter { art: a, ..brush })),
            DraftKind::Art { brush, art: name, .. } => copy(&name).map(|mut a| {
                // The side of the section with y up draws on the right of the path, and a brush
                // draws art's y (down here) on the left of the path.
                a.transform(Affine::scale_non_uniform(1.0, -1.0), false);
                BrushKind::Art(ArtBrush { art: a, ..brush })
            }),
            DraftKind::Pattern { .. } => None,
            DraftKind::Bristle(b) => Some(BrushKind::Bristle(b)),
        };
        match kind {
            Some(kind) => {
                let mut b = Brush { name: d.name, kind };
                b.sanitize();
                made.push(Some((tool, b)));
            }
            _ => left_out += 1,
        }
    }
    let mut out = Vec::with_capacity(made.len());
    for (tool, name) in order.unwrap_or_default() {
        if let Some(slot) = made.iter_mut().find(|m| m.as_ref().is_some_and(|(t, b)| t == tool && &b.name == name))
            && let Some((_, b)) = slot.take()
        {
            out.push(b);
        }
    }
    out.extend(made.into_iter().flatten().map(|(_, b)| b));
    (out, left_out)
}

/// The note for brushes [`finish_brushes`] left out.
pub(super) fn left_out_note(n: usize) -> String {
    format!("brushes VectorCraft can't read yet were left out of the Brushes panel ({n})")
}

/// Is `n` a frame: an unnamed, unpainted rectangle with nothing of its own (shown, unlocked, at full
/// opacity in Normal, with no effects, note, mask or other settings), a path of one subpath whose
/// anchors, without handles, are its four corners in order around it: closed, with the first again
/// at the end or not, or open with the first again at the end?
fn unpainted_rectangle(n: &Node) -> bool {
    let NodeKind::Path { path, guide: false, clipping: false, .. } = &n.kind else { return false };
    let ([sp], Some(b)) = (path.subpaths.as_slice(), n.geometric_bounds()) else { return false };
    let corner = |p: kurbo::Point| (p.x == b.x0 || p.x == b.x1) && (p.y == b.y0 || p.y == b.y1);
    let mut corners: Vec<kurbo::Point> = vec![];
    for a in &sp.anchors {
        if !corners.contains(&a.p) {
            corners.push(a.p);
        }
    }
    let last_repeats = sp.anchors.len() == 5 && sp.anchors.first().map(|a| a.p) == sp.anchors.last().map(|a| a.p);
    // Each corner to the next goes along a side, not across: one coordinate stays.
    let sides = corners.len() == 4
        && (0..4).all(|i| match (corners.get(i), corners.get((i + 1) % 4)) {
            (Some(a), Some(b)) => (a.x == b.x) != (a.y == b.y),
            _ => false,
        });
    n.name.is_none()
        && *n.extras() == vectorcraft_doc::NodeExtras::default()
        && n.visible
        && !n.locked
        && n.opacity >= 1.0
        && n.blend == vectorcraft_color::BlendMode::Normal
        && !n.isolate
        && n.knockout == vectorcraft_doc::Knockout::Neutral
        && !n.knockout_shape
        && n.appearance.effects.is_empty()
        && n.appearance.items.iter().all(|i| i.paint().is_none())
        && ((sp.closed && sp.anchors.len() == 4) || last_repeats)
        && sides
        && b.width() > 0.0
        && b.height() > 0.0
        && sp.anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p && corner(a.p))
}

/// Brush art `art` without its frame: a group whose first object is an unpainted rectangle around
/// the rest, exactly (within 1/1000 pt), loses that rectangle, and a group without a look or state
/// of its own (hidden, locked, isolation, knockout, opacity, blend mode, name, appearance, or a mask,
/// note or other setting) that is left with one object is that object. A frame wider than the art
/// is kept.
pub(super) fn without_frame(art: Node) -> Node {
    let NodeKind::Group { children, clip: false } = &art.kind else { return art };
    let Some((first, rest)) = children.split_first() else { return art };
    let around = rest.iter().filter_map(|n| n.geometric_bounds()).reduce(|a, b| a.union(b));
    let exact = match (around, first.geometric_bounds()) {
        (Some(a), Some(f)) => [a.x0 - f.x0, a.y0 - f.y0, a.x1 - f.x1, a.y1 - f.y1].iter().all(|d| d.abs() <= 1e-3),
        _ => false,
    };
    if !exact || !unpainted_rectangle(first) {
        return art;
    }
    let plain = art.opacity >= 1.0
        && art.visible
        && !art.locked
        && !art.isolate
        && art.knockout == vectorcraft_doc::Knockout::Neutral
        && !art.knockout_shape
        && art.blend == vectorcraft_color::BlendMode::Normal
        && art.name.is_none()
        && *art.extras() == vectorcraft_doc::NodeExtras::default()
        && art.appearance.effects.is_empty()
        && art.appearance.items.is_empty();
    if let ([only], true) = (rest, plain) {
        return Node::clone(only);
    }
    let mut out = art.clone();
    if let Some(c) = out.children_mut() {
        c.remove(0);
    }
    out
}

/// The plug-in name the objects of brushes of kind `kind` have, for writers.
pub(crate) fn plugin_name(kind: &BrushKind) -> &'static str {
    let tool = match kind {
        BrushKind::Calligraphic(_) => Tool::Calligraphic,
        BrushKind::Scatter(_) => Tool::Scatter,
        BrushKind::Art(_) => Tool::Art,
        BrushKind::Pattern(_) => Tool::Pattern,
        BrushKind::Bristle(_) => Tool::Bristle,
    };
    TOOLS.iter().find(|(_, t)| *t == tool).map_or("", |(n, _)| *n)
}

/// The plug-in name of the brush manager's order, for writers.
pub(crate) const MANAGER_NAME: &str = MANAGER;

/// A brush object as the reader reads it, for writers that check a brush before they write it.
#[derive(Clone, Debug)]
pub(crate) struct BrushRead {
    /// The brush the Brushes panel gets.
    pub brush: Brush,
    /// A path whose art style names the brush, at the brush's own settings, opens with the brush on
    /// its stroke: a Calligraphic brush, or an Art brush whose art an Art brush draws as the file
    /// does ([`art_bends`]) and whose numbers are those of a brush that stretches its art along the
    /// path, unflipped, at 100% in Tints.
    pub live: bool,
}

/// The brush named `name` whose object has the plug-in name `plugin` and the parameters `params`,
/// with `art` as the art its parameters name (as the reader reads that art), as the reader reads
/// it: `None` when the reader doesn't make a brush of it.
pub(crate) fn read_brush_object(plugin: &str, name: &str, params: &str, art: Option<&Node>) -> Option<BrushRead> {
    let draft = parse_tool_object(tool(plugin)?, name, params).ok()?;
    let mut sections = HashMap::new();
    if let (DraftKind::Scatter { art: section, .. } | DraftKind::Art { art: section, .. }, Some(a)) = (&draft.kind, art) {
        sections.insert(section.clone(), a.clone());
    }
    let drafts = [draft];
    let live = kept(&drafts, &sections).first().copied().unwrap_or(false)
        && match &drafts[0].kind {
            DraftKind::Calligraphic(_) => true,
            DraftKind::Art { plain: true, art: section, .. } => bending_art(&drafts, &sections).contains(section),
            _ => false,
        };
    let [draft] = drafts;
    let (mut brushes, _) = finish_brushes(vec![draft], &sections, None);
    Some(BrushRead { brush: brushes.pop()?, live })
}

/// Which of the brush objects `brushes` (plug-in name, name, parameters, the art each names as the
/// reader reads it) the reader makes into brushes when one file holds them all: not one it can't read,
/// nor one past the art the reader copies into one file's brushes ([`kept`]).
pub(crate) fn brushes_kept(brushes: &[(&str, &str, &str, Option<&Node>)]) -> Vec<bool> {
    let mut drafts = vec![];
    let mut at = vec![];
    let mut sections = HashMap::new();
    for (i, (plugin, name, params, art)) in brushes.iter().enumerate() {
        let Some(d) = tool(plugin).and_then(|t| parse_tool_object(t, name, params).ok()) else { continue };
        if let (DraftKind::Scatter { art: s, .. } | DraftKind::Art { art: s, .. }, Some(a)) = (&d.kind, art) {
            sections.insert(s.clone(), (*a).clone());
        }
        drafts.push(d);
        at.push(i);
    }
    let mut out = vec![false; brushes.len()];
    for (i, k) in at.into_iter().zip(kept(&drafts, &sections)) {
        if let Some(slot) = out.get_mut(i) {
            *slot = k;
        }
    }
    out
}

/// Brush objects a writer writes, as the reader reads them together from one file, for checking
/// the art styles that name them ([`Self::style_reads`]).
pub(crate) struct BrushesRead {
    drafts: Vec<BrushDraft>,
    /// The brush a stroke that names a brush is drawn with, by name: the first one kept
    /// ([`kept`]), as an index in [`Self::drafts`].
    by_name: HashMap<String, usize>,
    /// The art an Art brush draws as the file does ([`bending_art`]).
    bending: HashSet<String>,
}

impl BrushesRead {
    /// `brushes`: the brush objects written (plug-in name, name, parameters) and the art each names,
    /// as the reader reads it.
    pub(crate) fn new(brushes: &[(&str, &str, &str, Option<&Node>)]) -> Self {
        let mut drafts = vec![];
        let mut sections = HashMap::new();
        for (plugin, name, params, art) in brushes {
            let Some(d) = tool(plugin).and_then(|t| parse_tool_object(t, name, params).ok()) else { continue };
            if let (DraftKind::Scatter { art: s, .. } | DraftKind::Art { art: s, .. }, Some(a)) = (&d.kind, art) {
                sections.insert(s.clone(), (*a).clone());
            }
            drafts.push(d);
        }
        let mut by_name = HashMap::new();
        for (i, (d, k)) in drafts.iter().zip(kept(&drafts, &sections)).enumerate() {
            if k {
                by_name.entry(d.name.clone()).or_insert(i);
            }
        }
        let bending = bending_art(&drafts, &sections);
        Self { drafts, by_name, bending }
    }

    /// Does the reader read every stroke of art style `style` in `section` (the text between
    /// `%AI9_BeginArtStyles` and its end) that names a brush with that brush on it, at the brush's
    /// own settings? A style the reader doesn't read, or that names a brush no object gives, or an
    /// Art brush in a style with effects, isn't read so.
    pub(crate) fn style_reads(&self, section: &[u8], style: &str) -> bool {
        use super::style::StyleItem;
        let styles = super::style::art_styles(section);
        let Some(st) = styles.get(style).filter(|st| st.plain) else { return false };
        st.items.iter().all(|item| match item {
            StyleItem::Stroke { brush: Some((name, settings)), .. } => {
                let Some(draft) = self.by_name.get(name).and_then(|i| self.drafts.get(*i)) else { return false };
                draws_as(draft, settings, &self.bending) && !(matches!(settings, PathBrush::Art) && !st.effects.is_empty())
            }
            _ => true,
        })
    }
}
