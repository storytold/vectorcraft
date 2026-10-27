use super::*;

/// Measurement units (display only; the model is always in points).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    #[default]
    Points,
    Picas,
    Inches,
    Millimeters,
    Centimeters,
    Pixels,
    FeetInches,
    Meters,
    Yards,
    Feet,
}

impl Unit {
    pub const ALL: [Unit; 10] = [
        Unit::Points,
        Unit::Picas,
        Unit::Inches,
        Unit::Millimeters,
        Unit::Centimeters,
        Unit::Pixels,
        Unit::FeetInches,
        Unit::Meters,
        Unit::Yards,
        Unit::Feet,
    ];
    /// Points per unit.
    pub fn points(self) -> f64 {
        match self {
            Unit::Points | Unit::Pixels => 1.0,
            Unit::Picas => 12.0,
            Unit::Inches => 72.0,
            Unit::Millimeters => 72.0 / 25.4,
            Unit::Centimeters => 72.0 / 2.54,
            Unit::Meters => 72.0 / 0.0254,
            Unit::Yards => 72.0 * 36.0,
            Unit::Feet | Unit::FeetInches => 72.0 * 12.0,
        }
    }
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Points => "pt",
            Unit::Picas => "p",
            Unit::Inches => "in",
            Unit::Millimeters => "mm",
            Unit::Centimeters => "cm",
            Unit::Pixels => "px",
            Unit::FeetInches | Unit::Feet => "ft",
            Unit::Meters => "m",
            Unit::Yards => "yd",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Unit::Points => "Points",
            Unit::Picas => "Picas",
            Unit::Inches => "Inches",
            Unit::Millimeters => "Millimeters",
            Unit::Centimeters => "Centimeters",
            Unit::Pixels => "Pixels",
            Unit::FeetInches => "Feet & Inches",
            Unit::Meters => "Meters",
            Unit::Yards => "Yards",
            Unit::Feet => "Feet",
        }
    }
    /// The rulers' label step at `zoom` (screen pixels per point), in this unit: the first of
    /// 1, 2, 5 × 10ⁿ that puts labels at least 50 pixels apart. Ticks mark every tenth of it.
    pub fn ruler_step(self, zoom: f64) -> f64 {
        const DECIMAL: [f64; 19] =
            [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0];
        // An inch is divided in half, not in ten: a tenth of an inch is not a mark an inch ruler
        // carries, so below an inch the steps are binary fractions (#2827). Labelling stops at a
        // sixteenth; zooming in past that leaves the labels where they are and only the ticks
        // between them grow denser, down to a hundred and twenty-eighth
        // ([`Self::ruler_minor`]). A label is at most six characters that way ("0.0625"), which
        // the left ruler has room for when it spells one down the ruler a character at a time.
        // From an inch up the ladder is the shared one.
        const BINARY: [f64; 15] = [1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 10000.0];
        let steps: &[f64] = if self.ruler_halves() { &BINARY } else { &DECIMAL };
        steps.iter().copied().find(|s| s * self.points() * zoom >= 50.0).unwrap_or(10000.0)
    }
    /// Does this unit's ruler divide a step in half rather than in ten? Inches do; the metric
    /// units, points, pixels and picas are read in tenths.
    fn ruler_halves(self) -> bool {
        matches!(self, Unit::Inches)
    }
    /// The unlabelled ticks between two labels, `step` apart ([`Self::ruler_step`]): an eighth of
    /// the step on an inch ruler, so every tick is a binary fraction of an inch and they stay about
    /// as dense as the tenths elsewhere; a tenth of it otherwise.
    pub fn ruler_minor(self, step: f64) -> f64 {
        if self.ruler_halves() { step / 8.0 } else { step / 10.0 }
    }
    /// `v` (points) on the nearest ruler tick at `zoom` ([`Self::ruler_step`]).
    pub fn snap_to_ruler_tick(self, v: f64, zoom: f64) -> f64 {
        let tick = self.ruler_minor(self.ruler_step(zoom)) * self.points();
        (v / tick).round() * tick
    }
    pub fn from_pt(self, v: f64) -> f64 {
        v / self.points()
    }
    pub fn to_pt(self, v: f64) -> f64 {
        v * self.points()
    }
    /// Format a point value in this unit the way fields show it (e.g. `12.5 pt`, `3 in`).
    pub fn format(self, pt: f64) -> String {
        format!("{} {}", self.number(pt), self.suffix())
    }
    /// [`Unit::format`] without the suffix (`12.5`), for narrow fields.
    pub fn number(self, pt: f64) -> String {
        // Three decimals for the small units (`595.276 pt`), four for the large ones (`8.2677 in`,
        // `35.2778 mm`, the `0.0078 in` stroke preset); trailing zeros are trimmed (`1 pt` is `1`).
        let decimals = if matches!(self, Unit::Points | Unit::Pixels | Unit::Picas) { 3 } else { 4 };
        let s = format!("{:.*}", decimals, self.from_pt(pt));
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.into() }
    }
    /// Parse `12`, `12pt`, `1in`, `3 mm`, `2p6` (picas+points), simple `+ - * /` arithmetic.
    ///
    /// A unit after a `*` or `/` operand measures the whole expression (`1080/2 px` is 540 px), so
    /// math typed before a field's unit suffix works. After `*` or `/`, a percentage scales the term
    /// before it (`3cm*50%` is 1.5 cm, `3cm/50%` is 6 cm). After `+` or `-`, it adds or takes away
    /// that share of everything before it (`50pt+25%` is 62.5 pt, `100pt+20pt-50%` is 60 pt). A
    /// percentage on its own (`50%`) reads as nothing.
    pub fn parse(self, s: &str) -> Option<f64> {
        parse_measure(s, self).filter(|v| v.is_finite())
    }
    /// The value naming this unit in the Units preferences (`points`, `millimeters`,
    /// `feetInches`).
    pub fn key(self) -> &'static str {
        match self {
            Unit::Points => "points",
            Unit::Picas => "picas",
            Unit::Inches => "inches",
            Unit::Millimeters => "millimeters",
            Unit::Centimeters => "centimeters",
            Unit::Pixels => "pixels",
            Unit::FeetInches => "feetInches",
            Unit::Meters => "meters",
            Unit::Yards => "yards",
            Unit::Feet => "feet",
        }
    }
    /// The unit `name` names: its label, suffix or [`Unit::key`] in any case, ignoring spaces and
    /// punctuation (`Millimeters`, `mm`, `Feet & Inches`, `feetInches`).
    pub fn named(name: &str) -> Option<Unit> {
        let letters = |s: &'static str| s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase());
        let given = || name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase());
        // Feet and Feet & Inches share the suffix `ft`: it names the plain unit, so Feet & Inches
        // is left to its label and key.
        Unit::ALL
            .into_iter()
            .find(|u| letters(u.label()).eq(given()))
            .or_else(|| Unit::ALL.into_iter().filter(|u| *u != Unit::FeetInches).find(|u| u.suffix().chars().eq(given())))
    }
    /// A point value as the canvas measurement labels show it: two decimals and the suffix
    /// (`12.50 mm`).
    pub fn readout(self, pt: f64) -> String {
        format!("{:.2} {}", self.from_pt(pt), self.suffix())
    }
}

fn parse_measure(s: &str, default: Unit) -> Option<f64> {
    let s = s.trim();
    // Simple arithmetic on the right (Illustrator fields accept "10+5", "100/2", "3in*2").
    // `+` and `-` share one precedence level, as do `*` and `/`, so each level splits at the last
    // of its operators (left to right).
    for ops in [&['+', '-'][..], &['*', '/'][..]] {
        // Past the first character, so a leading sign isn't an operator.
        let skip = s.chars().next().map_or(0, char::len_utf8);
        // A `+` or `-` right after another operator is the sign of its operand (`10*-2`, `5--3`).
        let is_sign = |i: usize| {
            s.get(i..).and_then(|r| r.chars().next()).is_some_and(|c| matches!(c, '+' | '-'))
                && s.get(..i).and_then(|l| l.trim_end().chars().last()).is_some_and(|c| "+-*/".contains(c))
        };
        let found = s.char_indices().rev().find(|&(i, c)| i >= skip && ops.contains(&c) && !is_sign(i));
        if let Some(i) = found.map(|(i, _)| i) {
            let (l, r) = (&s[..i], &s[i + 1..]);
            if l.trim().is_empty() {
                continue;
            }
            let Some(op) = s[i..].chars().next() else { continue };
            return match op {
                '+' | '-' => {
                    let a = parse_measure(l, default)?;
                    // A percentage added or taken away is a share of everything before it:
                    // `50pt+25%` is 62.5 pt, `100pt+20%-10pt` is 110 pt.
                    let b = match percent(r) {
                        Some(k) => a * k,
                        None => parse_measure(r, default)?,
                    };
                    Some(if op == '+' { a + b } else { a - b })
                }
                _ => {
                    // A factor or divisor is a plain number or a percentage (`3cm*50%` is 1.5 cm);
                    // a unit after it is the expression's.
                    let (k, unit) = match percent(r) {
                        Some(k) => (k, None),
                        None => number_unit(r)?,
                    };
                    let a = parse_measure(l, unit.unwrap_or(default))?;
                    if op == '*' {
                        Some(a * k)
                    } else if k == 0.0 {
                        None
                    } else {
                        Some(a / k)
                    }
                }
            };
        }
    }
    // Plain decimals only, as in `number_unit` (`str::parse` would accept `1e5`, `inf`, `nan`).
    let plain = |t: &str| t.trim().chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') && t.trim().parse::<f64>().is_ok();
    if let Some((p, pt)) = s.split_once('p')
        && !p.is_empty()
        && plain(p)
        && (pt.is_empty() || plain(pt))
        && !s.ends_with("pt")
        && !s.ends_with("px")
    {
        // The sign belongs to the whole value: `-2p6` is -(2 picas + 6 points).
        let sign = if p.trim_start().starts_with('-') { -1.0 } else { 1.0 };
        let picas = p.trim().parse::<f64>().ok()?.abs();
        return Some(sign * (picas * 12.0 + pt.trim().parse::<f64>().unwrap_or(0.0)));
    }
    let (v, unit) = number_unit(s)?;
    Some(unit.unwrap_or(default).to_pt(v))
}

/// A number with an optional unit suffix (`12`, `3 mm`, `2in`) → (the number, its unit).
fn number_unit(s: &str) -> Option<(f64, Option<Unit>)> {
    let s = s.trim();
    // A leading `+` is a sign (`+5`), like `-`.
    let sign = usize::from(s.starts_with('+'));
    let num_end = s.get(sign..)?.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).map_or(s.len(), |i| i + sign);
    let v: f64 = s.get(..num_end)?.trim().parse().ok()?;
    let unit = match s.get(num_end..)?.trim() {
        "" => None,
        "pt" => Some(Unit::Points),
        "px" => Some(Unit::Pixels),
        "in" | "\"" => Some(Unit::Inches),
        "mm" => Some(Unit::Millimeters),
        "cm" => Some(Unit::Centimeters),
        "m" => Some(Unit::Meters),
        "ft" | "'" => Some(Unit::Feet),
        "yd" => Some(Unit::Yards),
        "p" | "pc" => Some(Unit::Picas),
        _ => return None,
    };
    Some((v, unit))
}

/// A number followed by `%` (`50%`, `12.5 %`) → its fraction (`0.5`, `0.125`).
fn percent(s: &str) -> Option<f64> {
    match number_unit(s.trim().strip_suffix('%')?)? {
        (p, None) => Some(p / 100.0),
        _ => None,
    }
}

/// A unitless field value (percent, degrees, counts) with the same `+ - * /` arithmetic as
/// [`Unit::parse`] (`45*2`, `100/3`); text other than digits, `.` and operators reads as nothing.
pub fn parse_number(s: &str) -> Option<f64> {
    if !s.chars().all(|c| c.is_ascii_digit() || " .+-*/".contains(c)) {
        return None;
    }
    // A leading `+` is a sign (`+5`), as a plain number read it.
    let s = s.trim();
    Unit::Points.parse(s.strip_prefix('+').unwrap_or(s))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMode {
    #[default]
    Rgb,
    Cmyk,
}

impl ColorMode {
    /// The colour model new colours take in a document of this mode.
    pub fn model(self) -> vectorcraft_color::cms::Model {
        match self {
            ColorMode::Rgb => vectorcraft_color::cms::Model::Rgb,
            ColorMode::Cmyk => vectorcraft_color::cms::Model::Cmyk,
        }
    }
}
