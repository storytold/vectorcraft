//! Colour harmony rules, the variation grid of the Color Guide panel and the five-colour themes of
//! the Color Themes panel, over document colours. The rules, their options and their colour math are
//! the framework's (`craft_color_ui::harmony`); these keep every colour in the model of the colour
//! it came from.

use craft_color_ui::harmony as rules;
pub use craft_color_ui::harmony::{GuideOptions, Harmony, THEME_SIZE, Variation};
use craft_theme::Srgba;

use crate::recolor::Palette;
use crate::{Color, keep_model};

/// The framework's colour for `c` (display sRGB).
fn srgba(c: Color) -> Srgba {
    let [r, g, b] = c.to_rgb();
    Srgba::rgb(r, g, b)
}

/// `c` in the model of `orig`: how [`move_on_wheel`] writes a moved colour.
pub fn in_model_of(orig: Color, c: Srgba) -> Color {
    keep_model(orig, Color::rgb(c.r, c.g, c.b))
}

/// The rule's name as the interface shows it.
pub fn label(rule: Harmony) -> &'static str {
    match rule {
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

/// A rule from its id or its label (case and spaces ignored).
pub fn parse(s: &str) -> Option<Harmony> {
    let key: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    Harmony::ALL.into_iter().find(|h| h.id().eq_ignore_ascii_case(&key))
}

/// The rule's colours in the base colour's model, the base colour itself first.
pub fn apply(rule: Harmony, base: Color) -> Vec<Color> {
    let rest = rule.apply(srgba(base)).into_iter().skip(1);
    std::iter::once(base).chain(rest.map(|c| in_model_of(base, c))).collect()
}

/// A [`THEME_SIZE`]-colour theme from `base` (the framework's `Harmony::theme`): the base colour
/// first, every colour in its model.
pub fn theme(rule: Harmony, base: Color) -> Vec<Color> {
    let rest = rule.theme(srgba(base)).into_iter().skip(1);
    std::iter::once(base).chain(rest.map(|c| in_model_of(base, c))).collect()
}

/// Move colour `i` of a harmony on the hue and saturation wheel to `hsb` (hue in degrees,
/// saturation and brightness 0..1), as the framework's `move_on_wheel` does; each colour that moves
/// keeps its model and the others keep their values.
pub fn move_on_wheel(colors: &mut [Color], i: usize, hsb: [f32; 3], linked: bool) {
    let mut moved: Vec<Srgba> = colors.iter().map(|c| srgba(*c)).collect();
    rules::move_on_wheel(&mut moved, i, hsb, linked);
    for (c, m) in colors.iter_mut().zip(moved) {
        if m != srgba(*c) {
            *c = in_model_of(*c, m);
        }
    }
}

/// The id of a variation in commands.
fn variation_id(v: Variation) -> &'static str {
    match v {
        Variation::TintsShades => "tintsShades",
        Variation::WarmCool => "warmCool",
        Variation::VividMuted => "vividMuted",
    }
}

/// A variation from its id ("tintsShades", "warmCool" or "vividMuted"; case ignored).
pub fn parse_variation(s: &str) -> Option<Variation> {
    Variation::ALL.into_iter().find(|v| variation_id(*v).eq_ignore_ascii_case(s))
}

/// The share of the way to its target (black or white, cool or warm, gray or full color) that the
/// variation at `step` goes: 0 at the center, the options' reach (10% … 90%) at the outermost steps.
pub fn step_fraction(opts: &GuideOptions, step: i32) -> f32 {
    let reach = 0.1 + 0.8 * opts.amount.clamp(0.0, 100.0) / 100.0;
    (step.unsigned_abs() as f32 / opts.steps.max(1) as f32).min(1.0) * reach
}

/// Variation of `c` at `step` (the framework's `variation`), in `c`'s model; step 0 is `c` itself.
pub fn variation(c: Color, opts: &GuideOptions, step: i32) -> Color {
    if step == 0 {
        return c;
    }
    in_model_of(c, rules::variation(srgba(c), opts, step))
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
        let colors = apply(rule, base);
        let grid = colors.iter().map(|c| (-n..=n).map(|step| variation(*c, &opts, step)).collect()).collect();
        Self { colors, grid }
    }
    /// Index of the centre column (the harmony colours) in each grid row.
    pub fn centre(&self) -> usize {
        self.grid.first().map_or(0, |r| r.len() / 2)
    }

    /// The guide limited to `palette` (Limit to Library): every harmony colour and variation snaps
    /// to the palette's nearest colour (CIEDE2000), in that colour's own model.
    pub fn limited(mut self, palette: &Palette) -> Self {
        if palette.is_empty() {
            return self;
        }
        let centre = self.centre();
        for (row, c) in self.grid.iter_mut().zip(&mut self.colors) {
            for v in row.iter_mut() {
                *v = palette.nearest(*v);
            }
            // The centre column is the harmony colour itself.
            *c = row[centre];
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complementary_red() {
        let c = apply(Harmony::Complementary, Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(c[1].to_hex(), "#00ffff");
        assert_eq!(apply(Harmony::Triad, Color::rgb(1.0, 0.0, 0.0))[1].to_hex(), "#00ff00");
        assert_eq!(apply(Harmony::Pentagram, Color::rgb(1.0, 0.0, 0.0))[1].to_hsb()[0].round(), 72.0);
    }

    #[test]
    fn every_rule_puts_the_base_first() {
        let counts = [2, 4, 3, 3, 3, 3, 5, 5, 5, 3, 5, 5, 4, 4, 4, 4, 4, 2, 4, 3, 5];
        assert_eq!(Harmony::ALL.len(), counts.len());
        for base in [Color::rgb8(230, 120, 40), Color::cmyk(0.1, 0.8, 0.3, 0.05), Color::gray(0.4)] {
            for (h, n) in Harmony::ALL.into_iter().zip(counts) {
                let c = apply(h, base);
                assert_eq!(c.len(), n, "{}", label(h));
                assert_eq!(c[0], base, "{}: the base comes first, in its own model", label(h));
            }
        }
    }

    #[test]
    fn harmonies_keep_the_base_model() {
        let base = Color::cmyk(0.0, 1.0, 1.0, 0.0);
        for h in Harmony::ALL {
            assert!(apply(h, base).iter().all(|c| matches!(c, Color::Cmyk { .. })), "{}", label(h));
        }
        let Color::Cmyk { c, m, y, .. } = apply(Harmony::Complementary, base)[1] else { panic!("a CMYK complement") };
        assert!(c > m && c > y, "the complement of red is a cyan: {c} {m} {y}");
        let g = Guide::new(Color::gray(0.4), Harmony::Triad, &GuideOptions::default());
        assert!(g.grid.iter().flatten().all(|c| matches!(c, Color::Gray { .. })));
    }

    #[test]
    fn rules_parse_by_id_and_label() {
        for h in Harmony::ALL {
            assert_eq!(parse(h.id()), Some(h));
            assert_eq!(parse(label(h)), Some(h));
        }
        assert_eq!(Harmony::SplitComplementary.id(), "splitComplementary");
        assert_eq!(Harmony::HighContrast3.id(), "highContrast3");
        assert_eq!(parse("square"), None);
    }

    #[test]
    fn high_contrast_flips_brightness() {
        let light = apply(Harmony::HighContrast, Color::rgb(0.9, 0.8, 0.2));
        assert!(light[1].to_hsb()[2] < 0.5);
        let dark = apply(Harmony::HighContrast, Color::rgb(0.2, 0.1, 0.05));
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

    #[test]
    fn a_limited_guide_only_holds_palette_colours() {
        let lib =
            [Color::rgb(1.0, 0.0, 0.0), Color::cmyk(1.0, 0.0, 0.0, 0.0), Color::gray(0.5), Color::rgb8(250, 240, 200), Color::lab(30.0, 10.0, -40.0)];
        let palette = Palette::new(lib);
        for rule in Harmony::ALL {
            let g = Guide::new(Color::rgb8(200, 60, 40), rule, &GuideOptions::default()).limited(&palette);
            assert!(g.grid.iter().flatten().chain(&g.colors).all(|c| lib.contains(c)), "{}", label(rule));
            for (row, c) in g.grid.iter().zip(&g.colors) {
                assert_eq!(row[g.centre()], *c, "{}: the harmony colour stays in the centre", label(rule));
            }
        }
        // The nearest colour wins: a pale yellow snaps to the cream and its darkest shade to the grey.
        let g = Guide::new(Color::rgb(0.95, 0.9, 0.75), Harmony::Complementary, &GuideOptions::default()).limited(&palette);
        assert_eq!(g.colors[0], lib[3]);
        assert_eq!(g.grid[0][0], lib[2]);
        // An empty palette limits nothing.
        let free = Guide::new(Color::rgb(0.8, 0.1, 0.1), Harmony::Triad, &GuideOptions::default());
        assert_eq!(free.clone().limited(&Palette::default()), free);
    }

    #[test]
    fn themes_have_five_colours_base_first() {
        for base in [Color::rgb8(230, 120, 40), Color::cmyk(0.1, 0.8, 0.3, 0.05)] {
            for rule in Harmony::ALL {
                let t = theme(rule, base);
                assert_eq!(t.len(), THEME_SIZE, "{}", label(rule));
                assert_eq!(t[0], base, "{}", label(rule));
                assert!(t.iter().all(|c| c.model() == base.model()), "{}: colours keep the base model", label(rule));
            }
        }
        // A two-colour rule fills in variations next to the colour they vary.
        let red = Color::rgb(1.0, 0.0, 0.0);
        let t = theme(Harmony::Complementary, red);
        assert_eq!(t[3].to_hex(), "#00ffff", "the complement follows the base's variations");
        let l = |c: &Color| c.to_lab().l;
        assert!(l(&t[1]) > l(&t[0]) && l(&t[2]) < l(&t[0]), "the base, a lighter and a darker red");
        assert!(l(&t[4]) < l(&t[3]));
        for (i, a) in t.iter().enumerate() {
            assert!(t[i + 1..].iter().all(|b| b.to_hex() != a.to_hex()), "{} repeats", a.to_hex());
        }
        // Five-colour rules are themes as they are.
        assert_eq!(theme(Harmony::Pentagram, red), apply(Harmony::Pentagram, red));
    }

    #[test]
    fn moving_a_linked_colour_moves_the_others_alike() {
        let mut c = [Color::rgb(1.0, 0.0, 0.0), Color::rgb(1.0, 1.0, 0.0), Color::cmyk(0.0, 1.0, 0.0, 0.0)];
        move_on_wheel(&mut c, 0, [120.0, 1.0, 1.0], false);
        assert_eq!(c[0].to_hex(), "#00ff00");
        assert_eq!(c[1].to_hex(), "#ffff00", "unlinked: only that colour moves");
        move_on_wheel(&mut c, 0, [240.0, 0.5, 1.0], true);
        assert_eq!(c[0].to_hex(), "#8080ff");
        let [h, s, v] = c[1].to_hsb();
        assert!((h - 180.0).abs() < 0.5 && (s - 0.5).abs() < 0.01 && v > 0.99, "it turned and paled as much: {h} {s} {v}");
        assert!(matches!(c[2], Color::Cmyk { .. }), "colours keep their model");
        // An index past the end moves nothing.
        let before = c;
        move_on_wheel(&mut c, 5, [0.0, 0.0, 0.0], true);
        assert_eq!(c, before);
    }
}
