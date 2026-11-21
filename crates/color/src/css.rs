//! Color text in the forms CSS Color 4 defines: hex colors with or without an alpha
//! (<https://www.w3.org/TR/css-color-4/#hex-notation>), `rgb()` and `rgba()`
//! (<https://www.w3.org/TR/css-color-4/#rgb-functions>), `hsl()` and `hsla()`
//! (<https://www.w3.org/TR/css-color-4/#the-hsl-notation>) and `oklch()`
//! (<https://www.w3.org/TR/css-color-4/#specifying-oklab-oklch>). The functions take the
//! space-separated form with an optional `/ alpha`, and `rgb()`, `rgba()`, `hsl()` and `hsla()` also
//! take the comma-separated legacy form (<https://www.w3.org/TR/css-color-4/#legacy-color-syntax>).
//! Hues take the angle units of CSS Values 4 (<https://www.w3.org/TR/css-values-4/#angles>). A hex
//! color may leave out its `#`. [`format`] writes a color in these forms.
//!
//! The module reads a subset of CSS. A function's arguments are literal numbers, percentages,
//! angles and `none`. A parenthesis inside the arguments makes the text invalid, so `calc()`,
//! `var()`, `color-mix()`, relative color syntax and every other nested function are refused, as are
//! named colors and the other color spaces and functions (`hwb()`, `lab()`, `oklab()`, `color()`).

use crate::oklab::{self, CHROMA_FULL};

/// The most characters a color text may have, spaces around it not counted.
pub const MAX_CHARS: usize = 48;

/// The space a parsed color's red, green and blue are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// The numbers as written: a hex color, `rgb()` or `hsl()`.
    Srgb,
    /// sRGB converted from `oklch()`.
    Oklch,
}

/// A color read from CSS text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CssColor {
    /// Red, green and blue from 0 to 1.
    pub rgb: [f32; 3],
    /// The alpha from 0 to 1, when the text gives one (CSS reads a missing alpha as 1).
    pub alpha: Option<f32>,
    pub space: Space,
}

/// The color `text` gives, or `None` when it is not one of the module's forms or has more than
/// [`MAX_CHARS`] characters. Values are clamped where CSS clamps them when it parses them: red, green
/// and blue to 0..=255 (0..=100 %), alpha to 0..=1, a negative HSL saturation to 0, OKLCH lightness
/// to 0..=1 and chroma to 0 or more. `none` reads as 0, as CSS reads a missing component when it
/// converts a color. Spaces, tabs, line breaks and form feeds may surround the text and its
/// arguments; other whitespace is not CSS whitespace and makes the text invalid. An `hsl()` color whose red, green or
/// blue falls outside 0..=1 has that channel clipped, and an `oklch()` color outside sRGB keeps its
/// lightness and hue and takes the largest chroma that fits ([`oklab::from_oklch`]); CSS maps such
/// colors into the gamut by its own rules instead. An `hsl()` color whose saturation and lightness
/// are so large that a channel overflows to an infinity is refused.
pub fn parse(text: &str) -> Option<CssColor> {
    let text = css_trim(text);
    if text.chars().nth(MAX_CHARS).is_some() {
        return None;
    }
    match text.find('(') {
        None => hex(text),
        Some(open) => {
            // CSS allows no space between a function's name and its `(`.
            let name = text.get(..open)?.to_ascii_lowercase();
            let args = text.get(open + 1..)?.strip_suffix(')')?;
            if args.contains(['(', ')']) {
                return None;
            }
            match name.as_str() {
                "rgb" | "rgba" => rgb(args),
                "hsl" | "hsla" => hsl(args),
                "oklch" => oklch(args),
                _ => None,
            }
        }
    }
}

/// `s` without the CSS whitespace (spaces, tabs, line breaks and form feeds, which CSS reads as line
/// breaks) around it (<https://www.w3.org/TR/css-syntax-3/#whitespace>,
/// <https://www.w3.org/TR/css-syntax-3/#input-preprocessing>).
fn css_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_ascii_whitespace())
}

/// `#rrggbb` of red, green and blue from 0 to 1, each rounded to the nearest of 256 steps.
pub fn hex_of(rgb: [f32; 3]) -> String {
    let [r, g, b] = rgb.map(byte);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// `v` (0 to 1) as one of 256 steps.
fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The forms [`format`] writes a color in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notation {
    /// `#rrggbb`.
    Hex,
    /// `#rrggbbaa`, `ff` for a color without an alpha.
    HexAlpha,
    /// `rgb(51 102 153)`.
    Rgb,
    /// `hsl(210 50% 40%)`.
    Hsl,
    /// `oklch(49.93% 0.0987 250.43)`.
    Oklch,
}

/// `rgb` (red, green and blue from 0 to 1) and the alpha `alpha` (0 to 1) as text of `notation`
/// that [`parse`] reads back to the same `#rrggbb`. Each channel is first rounded to the nearest of
/// 256 steps, as [`hex_of`] rounds it. `rgb()`, `hsl()` and `oklch()` use the space-separated form
/// and add the alpha as a percentage after a `/` when it rounds to less than 100 %; `hsl()` and
/// `oklch()` give their numbers to two places (chroma to four), without trailing zeros, and
/// `oklch()` takes one or two more places where fewer would read back to other bytes. The hue of a
/// gray is 0. [`Notation::Hex`] leaves the alpha out.
pub fn format(rgb: [f32; 3], alpha: Option<f32>, notation: Notation) -> String {
    // A NaN alpha reads as none.
    let alpha = alpha.filter(|a| !a.is_nan());
    let bytes = rgb.map(byte);
    let units = bytes.map(|v| f64::from(v) / 255.0);
    let [r, g, b] = bytes;
    match notation {
        Notation::Hex => hex_of(rgb),
        Notation::HexAlpha => format!("{}{:02x}", hex_of(rgb), byte(alpha.unwrap_or(1.0))),
        Notation::Rgb => format!("rgb({r} {g} {b}{})", alpha_part(alpha)),
        Notation::Hsl => {
            let [h, s, l] = rgb_to_hsl(units);
            format!("hsl({} {}% {}%{})", hue_text(h), number(s * 100.0, 2), number(l * 100.0, 2), alpha_part(alpha))
        }
        Notation::Oklch => {
            let lch = oklab::to_oklch(units.map(|v| v as f32));
            // The fewest places that read back to the same bytes, from two (four for chroma).
            let fits = |p: i32| oklch_rounded(lch, p).filter(|&[l, c, h]| oklab::from_oklch(l, c, h).0.map(byte) == bytes);
            let (p, [l, c, h]) = (0..3).find_map(|extra| fits(extra).map(|v| (extra, v))).unwrap_or((2, oklch_rounded(lch, 2).unwrap_or(lch)));
            let places = |base: i32| usize::try_from(base + p).unwrap_or(2);
            let c = number(c, places(4));
            // A chroma that rounds to 0 is a gray, whose hue is 0.
            let h = if c == "0" { "0".to_string() } else { hue_text_to(h, places(2)) };
            format!("oklch({}% {c} {h}{})", number(l * 100.0, places(2)), alpha_part(alpha))
        }
    }
}

/// OKLCH `lch` rounded as [`format`] writes it with `extra` more places than two for the
/// lightness percentage and the hue and four for the chroma, or `None` when no chroma at or under
/// the rounded one within [`OKLCH_CHROMA_STEPS`] steps lies inside sRGB. A color on the edge of
/// sRGB can round to a point just outside it, where [`parse`] would lower the chroma to the edge at
/// the rounded lightness and hue; the chroma is lowered a step at a time instead.
fn oklch_rounded([l, c, h]: [f64; 3], extra: i32) -> Option<[f64; 3]> {
    let (l, h) = (rounded(l, 4 + extra), rounded(h, 2 + extra));
    let step = 10f64.powi(-(4 + extra));
    let mut c = rounded(c, 4 + extra);
    for _ in 0..OKLCH_CHROMA_STEPS {
        if c <= 0.0 || oklab::in_srgb(oklab::oklch_to_srgb(l, c, h)) {
            return Some([l, c.max(0.0), h]);
        }
        c = rounded(c - step, 4 + extra);
    }
    None
}

/// The most steps [`oklch_rounded`] lowers a chroma by to bring it inside sRGB.
const OKLCH_CHROMA_STEPS: usize = 50;

/// `v` rounded to `places` decimal places.
fn rounded(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

/// ` / 50%` for an alpha that rounds to less than 100 %, else nothing.
fn alpha_part(alpha: Option<f32>) -> String {
    let percent = number(f64::from(alpha.unwrap_or(1.0).clamp(0.0, 1.0)) * 100.0, 2);
    if percent == "100" { String::new() } else { format!(" / {percent}%") }
}

/// A hue in degrees to two places, 360 written as 0.
fn hue_text(h: f64) -> String {
    hue_text_to(h, 2)
}

/// A hue in degrees to `places` places, 360 written as 0.
fn hue_text_to(h: f64, places: usize) -> String {
    let text = number(h, places);
    if text == "360" { "0".to_string() } else { text }
}

/// `v` to `places` decimal places, without trailing zeros or a sign on zero.
fn number(v: f64, places: usize) -> String {
    let text = format!("{v:.places$}");
    let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { text.as_str() };
    if text == "-0" { "0".to_string() } else { text.to_string() }
}

/// Hue in degrees (0 to 360), saturation and lightness (0 to 1) of `rgb`, by CSS Color 4's
/// rgbToHsl (<https://www.w3.org/TR/css-color-4/#rgb-to-hsl>); the hue of a gray is 0.
fn rgb_to_hsl([r, g, b]: [f64; 3]) -> [f64; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (d, l) = (max - min, (max + min) / 2.0);
    if d == 0.0 {
        return [0.0, 0.0, l];
    }
    let s = if l == 0.0 || l == 1.0 { 0.0 } else { (max - l) / l.min(1.0 - l) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h * 60.0, s, l]
}

/// A hex color of 3, 4, 6 or 8 digits, with or without `#`.
fn hex(text: &str) -> Option<CssColor> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nibbles: Vec<u8> = digits.bytes().filter_map(|b| (b as char).to_digit(16)).map(|d| d as u8).collect();
    let values: Vec<u8> = match nibbles.len() {
        3 | 4 => nibbles.iter().map(|n| n * 17).collect(),
        6 | 8 => nibbles.chunks(2).map(|p| p.iter().fold(0, |v, n| v * 16 + n)).collect(),
        _ => return None,
    };
    let unit = |i: usize| values.get(i).map(|v| f32::from(*v) / 255.0);
    Some(CssColor { rgb: [unit(0)?, unit(1)?, unit(2)?], alpha: unit(3), space: Space::Srgb })
}

/// One argument of a color function.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Arg {
    Number(f64),
    Percent(f64),
    /// An angle, in degrees.
    Angle(f64),
    None,
}

/// The arguments of a color function, `args` being the text inside its parentheses: the three
/// components and the alpha, if any, and whether they were separated by commas (the legacy form).
fn split(args: &str, legacy_allowed: bool) -> Option<([Arg; 3], Option<Arg>, bool)> {
    let (parts, alpha, legacy): (Vec<&str>, Option<&str>, bool) = if args.contains(',') {
        if !legacy_allowed || args.contains('/') {
            return None;
        }
        let mut parts: Vec<&str> = args.split(',').map(css_trim).collect();
        let alpha = if parts.len() == 4 { parts.pop() } else { None };
        (parts, alpha, true)
    } else {
        let mut halves = args.split('/');
        let colors = halves.next().unwrap_or_default();
        let alpha = halves.next();
        if halves.next().is_some() {
            return None;
        }
        (colors.split_ascii_whitespace().collect(), alpha.map(css_trim), false)
    };
    let [a, b, c] = parts.as_slice() else { return None };
    let one = |s: &str| {
        let arg = arg(s)?;
        // The legacy form has no `none`.
        (!(legacy && arg == Arg::None)).then_some(arg)
    };
    let alpha = match alpha {
        Some(s) => Some(one(s)?),
        None => None,
    };
    Some(([one(a)?, one(b)?, one(c)?], alpha, legacy))
}

/// One argument: `none`, a number, a percentage or an angle (`deg`, `grad`, `rad`, `turn`), with no
/// spaces inside it.
fn arg(s: &str) -> Option<Arg> {
    if s.eq_ignore_ascii_case("none") {
        return Some(Arg::None);
    }
    let end = number_end(s)?;
    let n: f64 = s.get(..end)?.parse().ok().filter(|n: &f64| n.is_finite())?;
    let unit = s.get(end..)?.to_ascii_lowercase();
    let arg = match unit.as_str() {
        "" => Arg::Number(n),
        "%" => Arg::Percent(n),
        "deg" => Arg::Angle(n),
        "grad" => Arg::Angle(n * 0.9),
        "rad" => Arg::Angle(n.to_degrees()),
        "turn" => Arg::Angle(n * 360.0),
        _ => return None,
    };
    match arg {
        Arg::Angle(a) if !a.is_finite() => None,
        _ => Some(arg),
    }
}

/// The byte length of the CSS number at the start of `s`: an optional sign, digits with an
/// optional fraction (at least one digit, and digits after a `.`), and an optional exponent.
fn number_end(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let digits = |from: usize| b.get(from..).map_or(0, |rest| rest.iter().take_while(|c| c.is_ascii_digit()).count());
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let whole = digits(i);
    i += whole;
    let mut fraction = 0;
    if b.get(i) == Some(&b'.') {
        fraction = digits(i + 1);
        if fraction == 0 {
            return None;
        }
        i += 1 + fraction;
    }
    if whole + fraction == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(b.get(i + 1), Some(b'+' | b'-')));
        let exp = digits(i + 1 + sign);
        if exp > 0 {
            i += 1 + sign + exp;
        }
    }
    Some(i)
}

/// The alpha argument from 0 to 1.
fn alpha(a: Option<Arg>) -> Option<Option<f32>> {
    let v = match a {
        None => return Some(None),
        Some(Arg::Number(n)) => n,
        Some(Arg::Percent(p)) => p / 100.0,
        Some(Arg::None) => 0.0,
        Some(Arg::Angle(_)) => return None,
    };
    Some(Some(v.clamp(0.0, 1.0) as f32))
}

/// A hue argument in degrees from 0 to 360.
fn hue(a: Arg) -> Option<f64> {
    match a {
        Arg::Number(d) | Arg::Angle(d) => Some(d.rem_euclid(360.0)).filter(|d| d.is_finite()),
        Arg::None => Some(0.0),
        Arg::Percent(_) => None,
    }
}

fn rgb(args: &str) -> Option<CssColor> {
    let (comps, a, legacy) = split(args, true)?;
    let percents = comps.iter().filter(|c| matches!(c, Arg::Percent(_))).count();
    if legacy && percents != 0 && percents != 3 {
        return None;
    }
    let unit = |c: Arg| match c {
        Arg::Number(n) => Some(n / 255.0),
        Arg::Percent(p) => Some(p / 100.0),
        Arg::None => Some(0.0),
        Arg::Angle(_) => None,
    };
    let [r, g, b] = comps;
    let rgb = [unit(r)?, unit(g)?, unit(b)?].map(|v| v.clamp(0.0, 1.0) as f32);
    Some(CssColor { rgb, alpha: alpha(a)?, space: Space::Srgb })
}

fn hsl(args: &str) -> Option<CssColor> {
    let (comps, a, legacy) = split(args, true)?;
    let [h, s, l] = comps;
    let percent = |c: Arg| match c {
        Arg::Percent(v) => Some(v),
        Arg::Number(v) if !legacy => Some(v),
        Arg::None => Some(0.0),
        _ => None,
    };
    // CSS clamps a negative saturation to 0 and nothing else.
    let (h, s, l) = (hue(h)?, percent(s)?.max(0.0) / 100.0, percent(l)? / 100.0);
    // CSS Color 4's hslToRgb (https://www.w3.org/TR/css-color-4/#hsl-to-rgb); a channel outside
    // 0..=1 is clipped.
    let f = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        let amount = s * l.min(1.0 - l);
        l - amount * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    // Huge saturation and lightness values overflow to an infinity or NaN, which is refused.
    let rgb = [f(0.0), f(8.0), f(4.0)];
    if rgb.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let rgb = rgb.map(|v| v.clamp(0.0, 1.0) as f32);
    Some(CssColor { rgb, alpha: alpha(a)?, space: Space::Srgb })
}

fn oklch(args: &str) -> Option<CssColor> {
    let (comps, a, _) = split(args, false)?;
    let [l, c, h] = comps;
    let l = match l {
        Arg::Number(v) => v,
        Arg::Percent(p) => p / 100.0,
        Arg::None => 0.0,
        Arg::Angle(_) => return None,
    };
    let c = match c {
        Arg::Number(v) => v,
        Arg::Percent(p) => p / 100.0 * CHROMA_FULL,
        Arg::None => 0.0,
        Arg::Angle(_) => return None,
    };
    let (rgb, _) = oklab::from_oklch(l.clamp(0.0, 1.0), c.max(0.0), hue(h)?);
    Some(CssColor { rgb, alpha: alpha(a)?, space: Space::Oklch })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn bytes(text: &str) -> ([u8; 3], Option<u8>) {
        let c = parse(text).unwrap_or_else(|| panic!("{text} should parse"));
        (c.rgb.map(byte), c.alpha.map(|a| (a * 255.0).round() as u8))
    }

    #[test]
    fn hex_colors_take_three_four_six_or_eight_digits_with_or_without_the_hash() {
        assert_eq!(bytes("#336699"), ([0x33, 0x66, 0x99], None));
        assert_eq!(bytes("33669980"), ([0x33, 0x66, 0x99], Some(0x80)));
        assert_eq!(bytes("#369"), ([0x33, 0x66, 0x99], None));
        assert_eq!(bytes("#3698"), ([0x33, 0x66, 0x99], Some(0x88)));
        assert_eq!(bytes("  #AbCdEf  "), ([0xab, 0xcd, 0xef], None));
        for bad in ["", "#", "#12", "#12345", "#1234567", "#123456789", "#ggg", "##123", "#+12", "#12 3", "１２３"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn rgb_reads_the_modern_and_the_legacy_forms() {
        assert_eq!(bytes("rgb(51 102 153)"), ([51, 102, 153], None));
        assert_eq!(bytes("rgb(51 102 153 / 50%)"), ([51, 102, 153], Some(128)));
        assert_eq!(bytes("RGBA(51 102 153/0.5)"), ([51, 102, 153], Some(128)));
        assert_eq!(bytes("rgb(20% 40% 60%)"), ([51, 102, 153], None));
        assert_eq!(bytes("rgb(20% 102 none)"), ([51, 102, 0], None), "the modern form mixes numbers and percentages");
        assert_eq!(bytes("rgb(51, 102, 153)"), ([51, 102, 153], None));
        assert_eq!(bytes("rgba(51,102,153,.5)"), ([51, 102, 153], Some(128)));
        assert_eq!(bytes("rgb(51, 102, 153, 50%)"), ([51, 102, 153], Some(128)));
        assert_eq!(bytes("rgb(300 -20 1e2)"), ([255, 0, 100], None), "components are clamped");
        assert_eq!(bytes("rgb(0 0 0 / 2)"), ([0, 0, 0], Some(255)), "the alpha is clamped");
        assert_eq!(bytes("rgb(0 0 0 / none)"), ([0, 0, 0], Some(0)));
        for bad in [
            "rgb(51, 102%, 153)",
            "rgb(51, 102, none)",
            "rgb(51 102)",
            "rgb(51 102 153 0.5)",
            "rgb(51, 102 153)",
            "rgb(51, 102, 153 / 0.5)",
            "rgb(51 102 153 / 0.5 / 1)",
            "rgb(51 102 153deg)",
            "rgb(1. 2 3)",
            "rgb(1e999 0 0)",
            "rgb(51 102 153",
            "rgb(51 102 153))",
            "rgb(51 102 153) x",
            "rgb(calc(1) 2 3)",
            "rgb(,,)",
            "rgb (1 2 3)",
            "rgb()",
            "rgbx(1 2 3)",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_spaces_tabs_and_line_breaks_are_whitespace() {
        assert_eq!(bytes("\t rgb(51,\n102 ,\t153)\r\n"), ([51, 102, 153], None));
        assert_eq!(bytes("rgb(51 102 153 /\t0.5 )"), ([51, 102, 153], Some(128)));
        assert_eq!(bytes("\u{c}#336699\u{c}"), ([51, 102, 153], None), "CSS reads a form feed as a line break");
        assert_eq!(bytes("rgb(51\u{c}102\u{c}153)"), ([51, 102, 153], None));
        for bad in ["rgb(51,\u{a0}102,153)", "rgb(51,102,153\u{3000})", "\u{a0}#336699", "#336699\u{2003}", "rgb(0 0 0 /\u{a0}1)", "\u{b}#336699"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn hsl_reads_hues_in_every_angle_unit() {
        assert_eq!(bytes("hsl(210 50% 40%)"), ([51, 102, 153], None));
        assert_eq!(bytes("hsl(210deg 50 40 / 25%)"), ([51, 102, 153], Some(64)));
        assert_eq!(bytes("hsla(210, 50%, 40%, 0.5)"), ([51, 102, 153], Some(128)));
        assert_eq!(bytes("hsl(-150 50% 40%)"), ([51, 102, 153], None), "a hue is normalized");
        assert_eq!(bytes("hsl(0.5turn 100% 50%)"), ([0, 255, 255], None));
        assert_eq!(bytes("hsl(200grad 100% 50%)"), ([0, 255, 255], None));
        assert_eq!(bytes("hsl(3.14159265rad 100% 50%)"), ([0, 255, 255], None));
        assert_eq!(bytes("hsl(120 -20% 50%)"), ([128, 128, 128], None), "a negative saturation is 0");
        assert_eq!(bytes("hsl(15 150% 50%)"), ([255, 32, 0], None), "a saturation over 100 % is kept and the channels are clipped");
        assert_eq!(bytes("hsl(0 50% 150%)"), ([255, 255, 255], None));
        assert_eq!(bytes("hsl(none 0% 100%)"), ([255, 255, 255], None));
        for bad in
            ["hsl(210, 50, 40)", "hsl(210% 50% 40%)", "hsl(210 50% 40% 1)", "hsl(210deg, 50%, none)", "hsl(1e999deg 0% 0%)", "hsl(1e306turn 0% 0%)"]
        {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
        // Saturation and lightness this large overflow to an infinity or NaN.
        for bad in ["hsl(90 1e308% 1e308%)", "hsl(90 1e308 -1e308)"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn oklch_converts_through_srgb_and_has_no_legacy_form() {
        let c = parse("oklch(62.8% 0.2577 29.23)").unwrap();
        assert_eq!((c.rgb.map(byte), c.alpha, c.space), ([255, 0, 0], None, Space::Oklch));
        let c = parse("oklch(0.628 64.4% 29.23 / 0.5)").unwrap();
        assert_eq!((c.rgb.map(byte), c.alpha), ([255, 0, 0], Some(0.5)));
        assert_eq!(parse("oklch(1 0 0)").map(|c| c.rgb.map(byte)), Some([255, 255, 255]));
        assert_eq!(parse("oklch(2 -1 none)").map(|c| c.rgb.map(byte)), Some([255, 255, 255]), "L and C are clamped");
        let wide = parse("oklch(0.7 0.4 150)").unwrap();
        assert!(wide.rgb.iter().all(|v| (0.0..=1.0).contains(v)), "a color outside sRGB is brought inside: {:?}", wide.rgb);
        let huge = parse("oklch(0.5 1e308 30)").unwrap();
        assert!(huge.rgb.iter().all(|v| (0.0..=1.0).contains(v)), "a huge chroma is brought inside: {:?}", huge.rgb);
        for bad in ["oklch(0.6, 0.1, 30)", "oklch(0.6 0.1 30% )", "oklch(0.6deg 0.1 30)", "oklch(0.6 0.1)"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn text_longer_than_the_limit_is_refused() {
        let at_limit = format!("rgb(51 102 153{})", " ".repeat(MAX_CHARS - 15));
        assert_eq!(at_limit.chars().count(), MAX_CHARS);
        assert!(parse(&at_limit).is_some());
        let over = format!("rgb(51 102 153{})", " ".repeat(MAX_CHARS - 14));
        assert_eq!(parse(&over), None);
        assert_eq!(parse(&format!("   {at_limit}   ")).map(|c| c.rgb.map(byte)), Some([51, 102, 153]), "spaces around it are not counted");
        assert_eq!(parse(&"(".repeat(100_000)), None);
    }

    #[test]
    fn hex_of_rounds_each_component() {
        assert_eq!(hex_of([0.2, 0.4, 0.6]), "#336699");
        assert_eq!(hex_of([-1.0, 2.0, f32::NAN]), "#00ff00");
    }

    #[test]
    fn format_writes_each_notation_with_the_alpha_where_it_belongs() {
        let rgb = [0.2, 0.4, 0.6];
        let half = Some(0.5);
        assert_eq!(format(rgb, half, Notation::Hex), "#336699");
        assert_eq!(format(rgb, half, Notation::HexAlpha), "#33669980");
        assert_eq!(format(rgb, None, Notation::HexAlpha), "#336699ff");
        assert_eq!(format(rgb, Some(1.0), Notation::HexAlpha), "#336699ff");
        assert_eq!(format(rgb, half, Notation::Rgb), "rgb(51 102 153 / 50%)");
        assert_eq!(format(rgb, None, Notation::Rgb), "rgb(51 102 153)");
        assert_eq!(format(rgb, Some(0.99999), Notation::Rgb), "rgb(51 102 153)", "an alpha that rounds to 100 % is left out");
        assert_eq!(format(rgb, Some(0.125), Notation::Hsl), "hsl(210 50% 40% / 12.5%)");
        assert_eq!(format(rgb, None, Notation::Oklch), "oklch(49.93% 0.0987 250.43)");
        assert_eq!(format([0.5; 3], None, Notation::Hsl), "hsl(0 0% 50.2%)");
        assert_eq!(format([1.0; 3], None, Notation::Oklch), "oklch(100% 0 0)");
        assert_eq!(format([0.0; 3], Some(0.0), Notation::Oklch), "oklch(0% 0 0 / 0%)");
        assert_eq!(format([1.0, 0.0, 0.0], None, Notation::Hsl), "hsl(0 100% 50%)");
        assert_eq!(format([-1.0, 2.0, f32::NAN], Some(f32::NAN), Notation::Rgb), "rgb(0 255 0)");
    }

    proptest! {
        #[test]
        fn random_text_never_panics(text in "\\PC{0,60}") {
            // `contains` is false for NaN, so these also check that every channel is finite.
            if let Some(c) = parse(&text) {
                prop_assert!(c.rgb.iter().all(|v| (0.0..=1.0).contains(v)));
                prop_assert!(c.alpha.is_none_or(|a| (0.0..=1.0).contains(&a)));
            }
        }

        #[test]
        fn function_shaped_text_never_panics(
            name in "(rgb|rgba|hsl|hsla|oklch|RGB)",
            args in "[-+0-9.eE%a-z ,/]{0,40}",
        ) {
            if let Some(c) = parse(&format!("{name}({args})")) {
                prop_assert!(c.rgb.iter().all(|v| (0.0..=1.0).contains(v)));
            }
        }

        #[test]
        fn eight_digit_hex_round_trips(r: u8, g: u8, b: u8, a: u8) {
            let c = parse(&format!("#{r:02x}{g:02x}{b:02x}{a:02x}")).unwrap();
            prop_assert_eq!(c.rgb.map(byte), [r, g, b]);
            prop_assert_eq!(c.alpha.map(|v| (v * 255.0).round() as u8), Some(a));
            prop_assert_eq!(hex_of(c.rgb), format!("#{r:02x}{g:02x}{b:02x}"));
        }

        #[test]
        fn each_notation_reads_back_to_the_same_color(r: u8, g: u8, b: u8, a: u8) {
            let rgb = [r, g, b].map(|v| f32::from(v) / 255.0);
            let alpha = f32::from(a) / 255.0;
            for notation in [Notation::Hex, Notation::HexAlpha, Notation::Rgb, Notation::Hsl, Notation::Oklch] {
                let text = format(rgb, Some(alpha), notation);
                let c = parse(&text).unwrap_or_else(|| panic!("{text} should parse"));
                prop_assert_eq!(hex_of(c.rgb), hex_of(rgb), "{}", text);
                let read = c.alpha.unwrap_or(1.0);
                match notation {
                    Notation::Hex => prop_assert_eq!(c.alpha, None),
                    Notation::HexAlpha => prop_assert_eq!(byte(read), a),
                    // A percentage to two places, left out when it rounds to 100.
                    _ => prop_assert!((read - alpha).abs() <= 0.00006, "{} gives {}", text, read),
                }
            }
        }
    }
}
