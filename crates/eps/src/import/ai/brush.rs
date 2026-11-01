//! Brush definitions (not publicly documented): plug-in objects in the setup, as written in files
//! that users own and sent us, brush library files among them (not presets installed with any
//! other app).
//!
//! - Each object is a `%AI8_BeginPluginObject` section of strings: the plug-in's name, the brush's
//!   name, then its parameters, split into strings that `-` continues and `.` ends.
//! - One object, the brush manager's order, has the order of the Brushes panel: its parameters
//!   are the plug-in name and brush name of each brush, each one after a `/` and a space.
//! - A Calligraphic brush's parameters are 11 numbers: the first is 1; the diameter, roundness and
//!   angle as (low, high) pairs; how the diameter, roundness and angle vary (0 Fixed, 1 Random, 2
//!   Pressure, other codes the pen's other channels); a last number.
//! - A Scatter, Art or Pattern brush's parameters are fields split by `/`: a number, the art's
//!   section names (one for Scatter, five for Art and Pattern, empty where there is none, each
//!   after a space for Art and Pattern), more fields, and last 15 numbers.
//!   - Scatter: the size (2, 3), spacing (5, 6) and scatter (8, 9) as (low, high) factors of the
//!     art, the rotation in degrees (11, 12), how each varies (4, 7, 10, 13) and whether the
//!     rotation follows the path (14).
//!   - Pattern: its scale as a factor (3).
//! - A Bristle brush's parameters start with 8 numbers: two codes, then the stiffness, bristle
//!   thickness, density, paint opacity and bristle length as fractions and the size in
//!   millimeters.
//! - The art is a `%AI8_BeginBrushPattern` section: its name as a string, then the art (read by
//!   [`super::read`]).
//!
//! What isn't read keeps its default: the Art brush's options, the pattern brush's tiles (which
//! slot is which tile isn't known) and every brush's colorization, which is left at None so the
//! art keeps its own colors.

use std::collections::HashMap;

use vectorcraft_brush::{ArtBrush, Bristle, Brush, BrushKind, Calligraphic, Colorization, Scatter, Variation};
use vectorcraft_doc::Node;

use super::lex::{Lexer, Tok};
use super::obj;

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
    /// The art is the section named `art`.
    Art {
        brush: ArtBrush,
        art: String,
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
/// its plug-in name, its name and its parameters, joined. `None` with fewer than two strings.
pub(super) fn plugin_strings(section: &[u8]) -> Option<(String, String, String)> {
    let mut lex = Lexer::new(section);
    let mut strings = vec![];
    let mut params = vec![];
    while let Some(t) = lex.next_token() {
        if let Tok::Str(s) | Tok::Hex(s) = t.tok {
            if strings.len() < 2 {
                strings.push(obj::text(&s));
            } else {
                // Joined before they are text: a string may end inside a character.
                let room = MAX_PARAMS.saturating_sub(params.len());
                params.extend_from_slice(s.get(..room.min(s.len())).unwrap_or_default());
            }
        }
    }
    let [kind, name]: [String; 2] = strings.try_into().ok()?;
    Some((kind, name, obj::text(&params)))
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

/// How a value varies, by its code: `None` for a code that isn't Fixed, Random or Pressure.
fn variation(code: f64) -> Option<Variation> {
    match code {
        0.0 => Some(Variation::Fixed),
        1.0 => Some(Variation::Random),
        2.0 => Some(Variation::Pressure),
        _ => None,
    }
}

/// [`variation`], or for another code, Random when the value varies and Fixed when it doesn't.
fn mode(code: f64, varies: bool) -> Variation {
    variation(code).unwrap_or(if varies { Variation::Random } else { Variation::Fixed })
}

/// The brush named `name` of kind `kind` from its parameters `params`.
pub(super) fn parse_tool_object(kind: Tool, name: &str, params: &str) -> Result<BrushDraft, BrushError> {
    let kind = match kind {
        Tool::Calligraphic => {
            let n = numbers(params).ok_or(BrushError::Values)?;
            let [_, d0, d1, r0, r1, a0, a1, dm, rm, am, ..] = n.as_slice() else { return Err(BrushError::Values) };
            let mid = |a: f64, b: f64| (a + b) / 2.0;
            let half = |a: f64, b: f64| (b - a).abs() / 2.0;
            let variation = [half(*a0, *a1), half(*r0, *r1), half(*d0, *d1)];
            let modes = [mode(*am, variation[0] != 0.0), mode(*rm, variation[1] != 0.0), mode(*dm, variation[2] != 0.0)];
            DraftKind::Calligraphic(Calligraphic {
                angle: mid(*a0, *a1),
                roundness: mid(*r0, *r1),
                size: mid(*d0, *d1),
                variation,
                modes: Some(modes),
            })
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
            let [_, p2, p3, p4, p5, p6, p7, p8, p9, p10, p11, p12, p13, p14, _] = n.as_slice() else { return Err(BrushError::Values) };
            let art = || tiles.first().cloned().flatten().ok_or(BrushError::NoArt);
            match kind {
                Tool::Scatter => {
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
                        colorization: Colorization::None,
                        modes: Some(modes),
                        ..Scatter::default()
                    };
                    DraftKind::Scatter { brush, art: art()? }
                }
                Tool::Art => DraftKind::Art { brush: ArtBrush { colorization: Colorization::None, ..ArtBrush::default() }, art: art()? },
                _ => {
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
/// pattern brushes (which slot holds which tile isn't known) and a second brush of the same name.
pub(super) fn finish_brushes(drafts: Vec<BrushDraft>, art: &HashMap<String, Node>, order: Option<&[(Tool, String)]>) -> (Vec<Brush>, usize) {
    let mut left_out = 0;
    let mut made: Vec<Option<(Tool, Brush)>> = vec![];
    for d in drafts {
        let tool = d.kind.tool();
        let kind = match d.kind {
            DraftKind::Calligraphic(c) => Some(BrushKind::Calligraphic(c)),
            DraftKind::Scatter { brush, art: name } => art.get(&name).map(|a| BrushKind::Scatter(Scatter { art: a.clone(), ..brush })),
            DraftKind::Art { brush, art: name } => art.get(&name).map(|a| BrushKind::Art(ArtBrush { art: a.clone(), ..brush })),
            DraftKind::Pattern { .. } => None,
            DraftKind::Bristle(b) => Some(BrushKind::Bristle(b)),
        };
        match kind {
            Some(kind) if !made.iter().flatten().any(|(_, b)| b.name == d.name) => {
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
