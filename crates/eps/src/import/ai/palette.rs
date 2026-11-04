//! The swatches of the editing data: the section between `%AI5_BeginPalette` and
//! `%AI5_EndPalette`, as written in files users sent.
//!
//! - `0 0 Pb` starts the swatches and `PB` ends them.
//! - Each swatch is its paint, then an optional `(name)`, then `Pc`. The paint is a colour operator
//!   (`g`, `k`, `Xa`: a process colour; `x`, `Xx`: a spot colour; `Xk`: a global process colour), a
//!   gradient (`Bb` … `flag (gradient) … Bg` … `BB`) or a pattern (`(pattern) … p`). `Xs` and `Xz`
//!   paint with Registration, which every document has.
//! - A swatch without a name (version 7 files write none) is named by its spot colour, gradient or
//!   pattern, or by its values as [`color_name`] gives them. `Pn` (the None swatch) and a `Pc` with
//!   no paint before it add nothing: the document has `[None]` already.
//! - `flag (name) count Pg` starts a colour group: the swatches up to the next `Pg` or `PB` are
//!   its own.
//! - Newer files write the swatches between `%AI17_Begin_Content_if_version_gt` and
//!   `%AI17_Alternate_Content`, then again for older versions up to `%AI17_End_Versioned_Content`:
//!   the first are read, the others skipped.
//!
//! Anything else in the section (an operator it doesn't know) is left alone: the swatches around
//! it still come in, and a paint before it makes no swatch.
//!
//! "AI7 spec": Adobe Illustrator File Format Specification, version 7.0 (1998), listed by PRONOM at
//! https://www.nationalarchives.gov.uk/PRONOM/fmt/423; §11 documents the palette operators, and its
//! example writes the swatches without names.

use vectorcraft_color::Color;
use vectorcraft_doc::swatches::color_name;

use super::lex::{Lexer, Tok};
use super::obj::V;
use super::paint::{self, Named};

/// Largest number of swatches read from one palette.
const MAX_SWATCHES: usize = 20_000;
/// Largest number of values kept between two operators.
const MAX_VALUES: usize = 64;

/// A swatch's paint, as the palette gives it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Cell {
    /// A colour: its full colour, whether it's global and whether it's an ink of its own.
    Color { color: Color, global: bool, spot: bool },
    /// The gradient defined under this name (`%AI5_BeginGradient`).
    Gradient(String),
    /// The pattern defined under this name (`%AI3_BeginPattern`).
    Pattern(String),
}

/// A swatch of the palette: its name and paint.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub name: String,
    pub cell: Cell,
}

/// The palette: the swatches outside any group, then each colour group's, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Palette {
    pub swatches: Vec<Entry>,
    pub groups: Vec<(String, Vec<Entry>)>,
}

impl Palette {
    pub fn is_empty(&self) -> bool {
        self.swatches.is_empty() && self.groups.iter().all(|(_, s)| s.is_empty())
    }
}

/// The palette of `data`: what follows `%AI5_BeginPalette`, up to its end.
pub(super) fn palette(data: &[u8]) -> Palette {
    let mut lex = Lexer::new(data);
    let mut out = Palette::default();
    let mut vals: Vec<V> = Vec::new();
    // The paint of the swatch being read, with the name it gets when `Pc` gives none.
    let mut cell: Option<Entry> = None;
    // How many swatches have been read.
    let mut count = 0usize;
    // Inside a gradient's `Bb` … `BB`.
    let mut in_gradient = false;
    // How deep in versioned content, and the depth of the block whose later alternatives (which
    // repeat its first) are being skipped, nested blocks in them included.
    let (mut depth, mut skipping) = (0usize, None::<usize>);
    while let Some(t) = lex.next_token() {
        if let Tok::Comment(c) = t.tok {
            let line = String::from_utf8_lossy(c);
            if line.contains("_Begin_Content_if_version") {
                depth = depth.saturating_add(1);
            } else if line.contains("_Alternate_Content") {
                skipping = skipping.or(Some(depth));
            } else if line.contains("_End_Versioned_Content") {
                if skipping == Some(depth) {
                    skipping = None;
                }
                depth = depth.saturating_sub(1);
            }
            continue;
        }
        if skipping.is_some() {
            continue;
        }
        let w = match t.tok {
            Tok::Num(v) => {
                push(&mut vals, V::Num(v));
                continue;
            }
            Tok::Str(s) | Tok::Hex(s) => {
                push(&mut vals, V::Str(s));
                continue;
            }
            Tok::Word(w) => std::str::from_utf8(w).unwrap_or("?"),
            _ => continue,
        };
        match w {
            // Array brackets: their numbers are a pattern's matrix, which the swatch doesn't need.
            "[" | "]" => continue,
            "Bb" => {
                in_gradient = true;
                cell = None;
            }
            "Bg" if in_gradient => {
                if let Some(name) = last_text(&vals) {
                    cell = Some(Entry { cell: Cell::Gradient(name.clone()), name });
                }
            }
            "BB" => in_gradient = false,
            "p" => cell = first_text(&vals).map(|name| Entry { cell: Cell::Pattern(name.clone()), name }),
            "Xs" | "Xz" | "XS" | "XZ" => cell = None,
            "g" | "k" | "x" | "Xa" | "Xx" | "Xk" => {
                cell = paint::color_op(w, &vals).and_then(|(p, named)| match named {
                    Some(Named { name, color, spot, .. }) => Some(Entry { name, cell: Cell::Color { color, global: true, spot } }),
                    None => p.color().map(|color| Entry { name: color_name(color), cell: Cell::Color { color, global: false, spot: false } }),
                });
            }
            "Pg" => {
                cell = None;
                // Capped as swatches are: a file of many `Pg` would make as many empty groups.
                if let Some(name) = first_text(&vals)
                    && out.groups.len() < MAX_SWATCHES
                {
                    out.groups.push((name, Vec::new()));
                }
            }
            "Pc" => {
                // The name before `Pc`, or the one the paint gives; `[Registration]` and the other
                // built-in swatches every document has are left out.
                let name = last_text(&vals);
                if let Some(mut e) = cell.take()
                    && !name.as_ref().is_some_and(|n| n.starts_with('['))
                    && count < MAX_SWATCHES
                {
                    if let Some(name) = name {
                        e.name = name;
                    }
                    count += 1;
                    match out.groups.last_mut() {
                        Some((_, g)) => g.push(e),
                        None => out.swatches.push(e),
                    }
                }
            }
            // A gradient's other operators (its focal point, its matrices) are part of its paint.
            _ if in_gradient => {}
            // Anything else ends the paint before it, which no `Pc` then makes a swatch of.
            _ => cell = None,
        }
        vals.clear();
    }
    out
}

fn push(vals: &mut Vec<V>, v: V) {
    if vals.len() < MAX_VALUES {
        vals.push(v);
    }
}

fn first_text(vals: &[V]) -> Option<String> {
    vals.iter().find(|v| matches!(v, V::Str(_))).and_then(V::text)
}

fn last_text(vals: &[V]) -> Option<String> {
    vals.iter().rev().find(|v| matches!(v, V::Str(_))).and_then(V::text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(name: &str, color: Color, global: bool, spot: bool) -> Entry {
        Entry { name: name.into(), cell: Cell::Color { color, global, spot } }
    }

    #[test]
    fn a_palette_gives_its_swatches_in_order() {
        let p = palette(
            b"0 0 Pb\n1 1 1 1 ([Registration]) 0 Xs\n([Registration])\nPc\n\
              0 0 0 1 k\n(Black)\nPc\n\
              0.1 0.9 0.8 0 (Ink 185) 0 x\n(Ink 185)\nPc\n\
              0 0.5 1 0 1 0.5 0 (Brand) 0 1 Xk\n(Brand)\nPc\n\
              0.5 0.5 0.5 0 50 40 -20 (Lab ink) 0 2 Xx\n(Lab ink)\nPc\n\
              Bb\n2 (Sky) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n(Sky)\nPc\n\
              (Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n(Dots)\nPc\nPB\n",
        );
        assert!(p.groups.is_empty());
        assert_eq!(
            p.swatches,
            vec![
                color("Black", Color::cmyk(0.0, 0.0, 0.0, 1.0), false, false),
                color("Ink 185", Color::cmyk(0.1, 0.9, 0.8, 0.0), true, true),
                color("Brand", Color::rgb(1.0, 0.5, 0.0), true, false),
                color("Lab ink", Color::lab(50.0, 40.0, -20.0), true, true),
                Entry { name: "Sky".into(), cell: Cell::Gradient("Sky".into()) },
                Entry { name: "Dots".into(), cell: Cell::Pattern("Dots".into()) },
            ]
        );
    }

    #[test]
    fn colour_groups_hold_the_swatches_after_them() {
        let p = palette(
            b"0 0 Pb\n0 0 0 0 k\n(White)\nPc\n1 (Greys) 1 Pg\n0 0 0 0.5 k\n(K50)\nPc\n0 (Inks) 1 Pg\n0 1 1 0 (Red ink) 0 x\n(Red ink)\nPc\nPB\n",
        );
        assert_eq!(p.swatches.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["White"]);
        let groups: Vec<(&str, Vec<&str>)> = p.groups.iter().map(|(n, s)| (n.as_str(), s.iter().map(|e| e.name.as_str()).collect())).collect();
        assert_eq!(groups, [("Greys", vec!["K50"]), ("Inks", vec!["Red ink"])]);
    }

    #[test]
    fn versioned_swatches_are_read_once() {
        let p = palette(
            b"0 0 Pb\n%AI17_Begin_Content_if_version_gt:24 15\n0 1 1 0 (Red ink) 0 x\n(Red ink)\nPc\n\
              %AI17_Alternate_Content\n0 1 1 0 (Red ink) 0 x\n(Red ink)\nPc\n%AI17_End_Versioned_Content\nPB\n",
        );
        assert_eq!(p.swatches.len(), 1);
    }

    /// Three alternatives, and versioned content nested in a later one: still one swatch.
    #[test]
    fn versioned_swatches_with_more_alternatives_are_read_once() {
        let p = palette(
            b"0 0 Pb\n%AI17_Begin_Content_if_version_gt:24 15\n0 1 1 0 (Red ink) 0 x\n(Red ink)\nPc\n\
              %AI17_Alternate_Content_if_version_gt:24 10\n0 1 1 0 (Red ink) 0 x\n(Red ink)\nPc\n\
              %AI17_Begin_Content_if_version_gt:23 1\n0 0 0 1 k\n(Black)\nPc\n%AI17_Alternate_Content\n%AI17_End_Versioned_Content\n\
              0 0 1 0 k\n(Yellow)\nPc\n\
              %AI17_Alternate_Content\n0 1 1 0 (Red ink) 0 x\n(Red ink)\nPc\n%AI17_End_Versioned_Content\n0 0 0 1 k\n(After)\nPc\nPB\n",
        );
        assert_eq!(p.swatches.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Red ink", "After"]);
    }

    #[test]
    fn what_it_doesnt_know_leaves_the_other_swatches() {
        let p = palette(b"0 0 Pb\n1 2 3 Zz\n(Odd)\nPc\n(no paint) Pc\n0 0 0 1 k\n(Black)\nPc\nPB\n");
        assert_eq!(p.swatches.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Black"]);
    }

    /// An operator it doesn't know after a paint ends that paint: the `Pc` after it makes no swatch.
    #[test]
    fn a_paint_before_an_unknown_operator_makes_no_swatch() {
        let p = palette(b"0 0 Pb\n0 1 1 0 k\nZz\n(Odd)\nPc\n0.5 g\n1 (Group) 1 Pg\nPc\n0 0 0 1 k\n(Black)\nPc\nPB\n");
        assert_eq!(p.swatches.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), Vec::<&str>::new());
        let groups: Vec<(&str, Vec<&str>)> = p.groups.iter().map(|(n, s)| (n.as_str(), s.iter().map(|e| e.name.as_str()).collect())).collect();
        assert_eq!(groups, [("Group", vec!["Black"])]);
    }

    /// The palette of the AI7 spec's example (§11): `Pn`, then each swatch's paint and `Pc` with no
    /// name, then a last `Pc`. Each swatch is named by its spot colour or gradient, or by its values;
    /// `1 g` is white (§5.4.1).
    #[test]
    fn swatches_without_names_are_named_by_their_paint() {
        let p = palette(
            b"0 0 Pb\nPn\nPc\n1 g\nPc\n0 g\nPc\n0 0 0 0 k\nPc\n0.75 g\nPc\n\
              Bb\n2 (B&W) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\nPc\n0.25 0 0 0 k\nPc\n\
              0 1 1 0 (Spot A) 0 x\nPc\n0.75 g\nPc\n(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\nPc\nPc\nPB\n",
        );
        assert!(p.groups.is_empty());
        assert_eq!(
            p.swatches,
            vec![
                color("Gray K=0", Color::gray(0.0), false, false),
                color("Gray K=100", Color::gray(1.0), false, false),
                color("C=0 M=0 Y=0 K=0", Color::cmyk(0.0, 0.0, 0.0, 0.0), false, false),
                color("Gray K=25", Color::gray(0.25), false, false),
                Entry { name: "B&W".into(), cell: Cell::Gradient("B&W".into()) },
                color("C=25 M=0 Y=0 K=0", Color::cmyk(0.25, 0.0, 0.0, 0.0), false, false),
                color("Spot A", Color::cmyk(0.0, 1.0, 1.0, 0.0), true, true),
                color("Gray K=25", Color::gray(0.25), false, false),
                Entry { name: "Dots".into(), cell: Cell::Pattern("Dots".into()) },
            ]
        );
        // RGB and global process colours too.
        let p = palette(b"0 0 Pb\n0 0 0 0 1 0.5 0 Xa\nPc\n0 0.5 1 0 1 0.5 0 (Brand) 0 1 Xk\nPc\nPB\n");
        assert_eq!(p.swatches.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["R=255 G=128 B=0", "Brand"]);
    }

    /// Named and unnamed swatches in one palette: the named ones keep their names, and built-in
    /// swatches are still left out.
    #[test]
    fn named_swatches_keep_their_names_beside_unnamed_ones() {
        let p = palette(
            b"0 0 Pb\n1 1 1 1 ([Registration]) 0 Xs\n([Registration])\nPc\n0 0 0 1 k\n(Black)\nPc\n0 0 0 0.5 k\nPc\n\
              1 (Greys) 1 Pg\n0.4 g\n(Mid)\nPc\n0.4 g\nPc\nPB\n",
        );
        assert_eq!(p.swatches.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Black", "C=0 M=0 Y=0 K=50"]);
        let groups: Vec<(&str, Vec<&str>)> = p.groups.iter().map(|(n, s)| (n.as_str(), s.iter().map(|e| e.name.as_str()).collect())).collect();
        assert_eq!(groups, [("Greys", vec!["Mid", "Gray K=60"])]);
    }

    /// No more than `MAX_SWATCHES` swatches, counted across the colour groups.
    #[test]
    fn swatches_are_capped_across_groups() {
        let mut data = b"0 0 Pb\n".to_vec();
        for i in 0..MAX_SWATCHES / 1000 + 1 {
            data.extend(format!("1 (G{i}) 1 Pg\n").as_bytes());
            data.extend("0.5 g\nPc\n".repeat(1000).as_bytes());
        }
        data.extend(b"PB\n");
        let p = palette(&data);
        assert_eq!(p.groups.len(), MAX_SWATCHES / 1000 + 1);
        assert_eq!(p.swatches.len() + p.groups.iter().map(|(_, s)| s.len()).sum::<usize>(), MAX_SWATCHES);
        assert!(p.groups.last().is_some_and(|(_, s)| s.is_empty()));
    }
}
