//! Colour harmony rules and the variation grid of the Color Guide panel (our own colour math).

use serde::{Deserialize, Serialize};

use crate::{Color, keep_model};

/// A harmony rule: a group of colours built from a base colour, base first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Harmony {
    Complementary,
    Complementary2,
    SplitComplementary,
    LeftComplement,
    RightComplement,
    Analogous,
    Analogous2,
    Monochromatic,
    Shades,
    Triad,
    Triad2,
    Triad3,
    Tetrad,
    Tetrad2,
    Tetrad3,
    Compound,
    Compound2,
    HighContrast,
    HighContrast2,
    HighContrast3,
    Pentagram,
}

/// One colour of a rule: the base turned by `turn` degrees of hue, its saturation scaled by `sat`
/// and its brightness by `bright` (or flipped to the contrasting brightness).
#[derive(Clone, Copy)]
struct Tone {
    turn: f32,
    sat: f32,
    bright: Bright,
}

#[derive(Clone, Copy)]
enum Bright {
    Times(f32),
    /// Dark for a light base, light for a dark one ([`contrast`]).
    Contrast,
}

const fn sv(turn: f32, sat: f32, bright: f32) -> Tone {
    Tone { turn, sat, bright: Bright::Times(bright) }
}
/// The base's hue turned by `turn`.
const fn t(turn: f32) -> Tone {
    sv(turn, 1.0, 1.0)
}
/// A softer (half as saturated) partner.
const fn soft(turn: f32) -> Tone {
    sv(turn, 0.5, 1.0)
}
/// A deeper (darker) partner.
const fn deep(turn: f32) -> Tone {
    sv(turn, 1.0, 0.6)
}
/// A partner of contrasting brightness.
const fn hc(turn: f32) -> Tone {
    Tone { turn, sat: 1.0, bright: Bright::Contrast }
}

/// The brightness that contrasts with `v`.
fn contrast(v: f32) -> f32 {
    if v >= 0.5 { v * 0.3 } else { 0.7 + 0.3 * v }
}

impl Harmony {
    pub const ALL: [Harmony; 21] = [
        Harmony::Complementary,
        Harmony::Complementary2,
        Harmony::SplitComplementary,
        Harmony::LeftComplement,
        Harmony::RightComplement,
        Harmony::Analogous,
        Harmony::Analogous2,
        Harmony::Monochromatic,
        Harmony::Shades,
        Harmony::Triad,
        Harmony::Triad2,
        Harmony::Triad3,
        Harmony::Tetrad,
        Harmony::Tetrad2,
        Harmony::Tetrad3,
        Harmony::Compound,
        Harmony::Compound2,
        Harmony::HighContrast,
        Harmony::HighContrast2,
        Harmony::HighContrast3,
        Harmony::Pentagram,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Harmony::Complementary => "Complementary",
            Harmony::Complementary2 => "Complementary 2",
            Harmony::SplitComplementary => "Split Complementary",
            Harmony::LeftComplement => "Left Complement",
            Harmony::RightComplement => "Right Complement",
            Harmony::Analogous => "Analogous",
            Harmony::Analogous2 => "Analogous 2",
            Harmony::Monochromatic => "Monochromatic",
            Harmony::Shades => "Shades",
            Harmony::Triad => "Triad",
            Harmony::Triad2 => "Triad 2",
            Harmony::Triad3 => "Triad 3",
            Harmony::Tetrad => "Tetrad",
            Harmony::Tetrad2 => "Tetrad 2",
            Harmony::Tetrad3 => "Tetrad 3",
            Harmony::Compound => "Compound",
            Harmony::Compound2 => "Compound 2",
            Harmony::HighContrast => "High Contrast",
            Harmony::HighContrast2 => "High Contrast 2",
            Harmony::HighContrast3 => "High Contrast 3",
            Harmony::Pentagram => "Pentagram",
        }
    }

    /// The rule's id in commands: its label in camelCase ("splitComplementary", "triad2").
    pub fn id(self) -> String {
        let mut out = String::new();
        for (i, w) in self.label().split(' ').enumerate() {
            let mut c = w.chars();
            if let Some(f) = c.next() {
                if i == 0 {
                    out.extend(f.to_lowercase());
                } else {
                    out.extend(f.to_uppercase());
                }
                out.push_str(c.as_str());
            }
        }
        out
    }

    /// A rule from its id or label (case and spaces ignored).
    pub fn parse(s: &str) -> Option<Self> {
        let key = |x: &str| x.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        let want = key(s);
        Self::ALL.into_iter().find(|h| key(h.label()) == want)
    }

    fn tones(self) -> &'static [Tone] {
        match self {
            Harmony::Complementary => const { &[t(0.0), t(180.0)] },
            Harmony::Complementary2 => const { &[t(0.0), soft(0.0), t(180.0), soft(180.0)] },
            Harmony::SplitComplementary => const { &[t(0.0), t(150.0), t(210.0)] },
            Harmony::LeftComplement => const { &[t(0.0), t(-30.0), t(150.0)] },
            Harmony::RightComplement => const { &[t(0.0), t(30.0), t(210.0)] },
            Harmony::Analogous => const { &[t(0.0), t(30.0), t(-30.0)] },
            Harmony::Analogous2 => const { &[t(0.0), t(30.0), t(60.0), t(-30.0), t(-60.0)] },
            Harmony::Monochromatic => const { &[t(0.0), sv(0.0, 0.5, 1.0), sv(0.0, 1.0, 0.6), sv(0.0, 0.25, 1.0), sv(0.0, 1.0, 0.35)] },
            Harmony::Shades => const { &[t(0.0), sv(0.0, 1.0, 0.8), sv(0.0, 1.0, 0.6), sv(0.0, 1.0, 0.4), sv(0.0, 1.0, 0.2)] },
            Harmony::Triad => const { &[t(0.0), t(120.0), t(240.0)] },
            Harmony::Triad2 => const { &[t(0.0), t(120.0), soft(120.0), t(240.0), soft(240.0)] },
            Harmony::Triad3 => const { &[t(0.0), t(120.0), deep(120.0), t(240.0), deep(240.0)] },
            Harmony::Tetrad => const { &[t(0.0), t(60.0), t(180.0), t(240.0)] },
            Harmony::Tetrad2 => const { &[t(0.0), t(90.0), t(180.0), t(270.0)] },
            Harmony::Tetrad3 => const { &[t(0.0), t(30.0), t(180.0), t(210.0)] },
            Harmony::Compound => const { &[t(0.0), t(40.0), t(160.0), t(200.0)] },
            Harmony::Compound2 => const { &[t(0.0), t(-40.0), t(160.0), t(200.0)] },
            Harmony::HighContrast => const { &[t(0.0), hc(180.0)] },
            Harmony::HighContrast2 => const { &[t(0.0), hc(0.0), t(180.0), hc(180.0)] },
            Harmony::HighContrast3 => const { &[t(0.0), hc(120.0), hc(240.0)] },
            Harmony::Pentagram => const { &[t(0.0), t(72.0), t(144.0), t(216.0), t(288.0)] },
        }
    }

    /// The rule's colours in the base colour's model, the base colour itself first.
    pub fn apply(self, base: Color) -> Vec<Color> {
        let [h, s, v] = base.to_hsb();
        std::iter::once(base)
            .chain(self.tones()[1..].iter().map(|tone| {
                let b = match tone.bright {
                    Bright::Times(f) => v * f,
                    Bright::Contrast => contrast(v),
                };
                keep_model(base, Color::from_hsb(h + tone.turn, (s * tone.sat).clamp(0.0, 1.0), b.clamp(0.0, 1.0)))
            }))
            .collect()
    }
}

/// What the variation grid varies: towards black and white, towards cool and warm, or towards
/// grey and full saturation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Variation {
    #[default]
    TintsShades,
    WarmCool,
    VividMuted,
}

impl Variation {
    pub const ALL: [Variation; 3] = [Variation::TintsShades, Variation::WarmCool, Variation::VividMuted];

    pub fn id(self) -> &'static str {
        match self {
            Variation::TintsShades => "tintsShades",
            Variation::WarmCool => "warmCool",
            Variation::VividMuted => "vividMuted",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id().eq_ignore_ascii_case(s))
    }
    /// The ends of the grid: (left, right).
    pub fn sides(self) -> (&'static str, &'static str) {
        match self {
            Variation::TintsShades => ("Shades", "Tints"),
            Variation::WarmCool => ("Cool", "Warm"),
            Variation::VividMuted => ("Muted", "Vivid"),
        }
    }
}

/// The Color Guide's options: variation kind, steps on each side and how far they reach.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GuideOptions {
    pub variation: Variation,
    /// Variations on each side of the harmony colour (1..=20).
    pub steps: u32,
    /// How far the outermost variations go, 0..=100 (Less … More).
    pub amount: f32,
}

impl Default for GuideOptions {
    fn default() -> Self {
        Self { variation: Variation::TintsShades, steps: 4, amount: 50.0 }
    }
}

impl GuideOptions {
    pub const MAX_STEPS: u32 = 20;

    /// The options with `steps` and `amount` clamped to their ranges.
    pub fn clamped(self) -> Self {
        Self { steps: self.steps.clamp(1, Self::MAX_STEPS), amount: self.amount.clamp(0.0, 100.0), ..self }
    }
    /// The share of the way to the target that the outermost steps reach (10% … 90%).
    fn reach(&self) -> f32 {
        0.1 + 0.8 * self.amount.clamp(0.0, 100.0) / 100.0
    }
}

/// Variation of `c` at `step` (−steps..=steps: negative towards shades/cool/muted, positive towards
/// tints/warm/vivid; 0 is `c` itself), in `c`'s model.
pub fn variation(c: Color, opts: &GuideOptions, step: i32) -> Color {
    if step == 0 {
        return c;
    }
    let f = (step.unsigned_abs() as f32 / opts.steps.max(1) as f32).min(1.0) * opts.reach();
    let v = match opts.variation {
        Variation::TintsShades => c.lerp(if step < 0 { &Color::BLACK } else { &Color::WHITE }, f),
        Variation::WarmCool => {
            let target = if step < 0 { Color::rgb(0.2, 0.45, 1.0) } else { Color::rgb(1.0, 0.55, 0.1) };
            c.lerp(&target, f * 0.6)
        }
        Variation::VividMuted => {
            let [h, s, b] = c.to_hsb();
            let s2 = if step < 0 { s * (1.0 - f) } else { s + (1.0 - s) * f };
            Color::from_hsb(h, s2.clamp(0.0, 1.0), b)
        }
    };
    keep_model(c, v)
}

/// The Color Guide for a base colour: the harmony colours and, per colour, its row of variations
/// from `-steps` to `+steps` (the colour itself in the centre column).
#[derive(Clone, Debug, PartialEq)]
pub struct Guide {
    pub colors: Vec<Color>,
    pub grid: Vec<Vec<Color>>,
}

impl Guide {
    pub fn new(base: Color, rule: Harmony, opts: &GuideOptions) -> Self {
        let opts = opts.clamped();
        let n = opts.steps as i32;
        let colors = rule.apply(base);
        let grid = colors.iter().map(|c| (-n..=n).map(|step| variation(*c, &opts, step)).collect()).collect();
        Self { colors, grid }
    }
    /// Index of the centre column (the harmony colours) in each grid row.
    pub fn centre(&self) -> usize {
        self.grid.first().map_or(0, |r| r.len() / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complementary_red() {
        let c = Harmony::Complementary.apply(Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(c[1].to_hex(), "#00ffff");
        assert_eq!(Harmony::Triad.apply(Color::rgb(1.0, 0.0, 0.0))[1].to_hex(), "#00ff00");
        assert_eq!(Harmony::Pentagram.apply(Color::rgb(1.0, 0.0, 0.0))[1].to_hsb()[0].round(), 72.0);
    }

    #[test]
    fn every_rule_puts_the_base_first() {
        let counts = [2, 4, 3, 3, 3, 3, 5, 5, 5, 3, 5, 5, 4, 4, 4, 4, 4, 2, 4, 3, 5];
        assert_eq!(Harmony::ALL.len(), counts.len());
        for base in [Color::rgb8(230, 120, 40), Color::cmyk(0.1, 0.8, 0.3, 0.05), Color::gray(0.4)] {
            for (h, n) in Harmony::ALL.into_iter().zip(counts) {
                let c = h.apply(base);
                assert_eq!(c.len(), n, "{}", h.label());
                assert_eq!(c[0], base, "{}: the base comes first, in its own model", h.label());
            }
        }
    }

    #[test]
    fn harmonies_keep_the_base_model() {
        let base = Color::cmyk(0.0, 1.0, 1.0, 0.0);
        for h in Harmony::ALL {
            assert!(h.apply(base).iter().all(|c| matches!(c, Color::Cmyk { .. })), "{}", h.label());
        }
        let Color::Cmyk { c, m, y, .. } = Harmony::Complementary.apply(base)[1] else { panic!("complement should stay CMYK") };
        assert!(c > m && c > y, "the complement of red is a cyan: {c} {m} {y}");
        let g = Guide::new(Color::gray(0.4), Harmony::Triad, &GuideOptions::default());
        assert!(g.grid.iter().flatten().all(|c| matches!(c, Color::Gray { .. })));
    }

    #[test]
    fn rules_parse_by_id_and_label() {
        for h in Harmony::ALL {
            assert_eq!(Harmony::parse(&h.id()), Some(h));
            assert_eq!(Harmony::parse(h.label()), Some(h));
        }
        assert_eq!(Harmony::SplitComplementary.id(), "splitComplementary");
        assert_eq!(Harmony::HighContrast3.id(), "highContrast3");
        assert_eq!(Harmony::parse("square"), None);
    }

    #[test]
    fn high_contrast_flips_brightness() {
        let light = Harmony::HighContrast.apply(Color::rgb(0.9, 0.8, 0.2));
        assert!(light[1].to_hsb()[2] < 0.5);
        let dark = Harmony::HighContrast.apply(Color::rgb(0.2, 0.1, 0.05));
        assert!(dark[1].to_hsb()[2] > 0.5);
    }

    #[test]
    fn variations_step_out_from_the_colour() {
        let c = Color::rgb(0.5, 0.5, 0.5);
        let o = GuideOptions::default();
        assert_eq!(variation(c, &o, 0), c, "step 0 is the colour");
        assert!(variation(c, &o, 4).to_rgb()[0] > variation(c, &o, 2).to_rgb()[0]);
        assert!(variation(c, &o, -4).to_rgb()[0] < 0.5);
        let red = Color::rgb(1.0, 0.2, 0.2);
        let vm = GuideOptions { variation: Variation::VividMuted, ..o };
        assert!(variation(red, &vm, -2).to_hsb()[1] < red.to_hsb()[1]);
        let warm = variation(c, &GuideOptions { variation: Variation::WarmCool, ..o }, 2).to_rgb();
        assert!(warm[0] > warm[2]);
        // More variation reaches further.
        let more = GuideOptions { amount: 100.0, ..o };
        assert!(variation(c, &more, -4).to_rgb()[0] < variation(c, &o, -4).to_rgb()[0]);
    }

    #[test]
    fn guide_rows_hold_the_colour_in_the_centre() {
        let base = Color::rgb8(30, 120, 200);
        let g = Guide::new(base, Harmony::Triad, &GuideOptions { steps: 3, ..Default::default() });
        assert_eq!(g.colors.len(), 3);
        assert!(g.grid.iter().all(|r| r.len() == 7));
        assert_eq!(g.centre(), 3);
        for (row, c) in g.grid.iter().zip(&g.colors) {
            assert_eq!(row[3], *c);
        }
        // Steps are clamped to 1..=20.
        let g = Guide::new(base, Harmony::Triad, &GuideOptions { steps: 99, ..Default::default() });
        assert_eq!(g.grid[0].len(), 41);
    }
}
