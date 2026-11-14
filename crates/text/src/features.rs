//! OpenType feature selection (the OpenType panel), resolved per character style.

use harfrust::{Feature, Tag};
use vectorcraft_doc::CharStyle;

/// Tracking (in 1/1000 em) outside which standard ligatures are dropped automatically: tighter
/// than the first, looser than the second.
///
/// Letterspaced type should not ligate: an `fi` glyph keeps its letters tight while everything
/// around it opens up (or stays open while it closes up), which reads as a blot. Small tracking
/// adjustments made to fit a line are not letterspacing and keep the ligatures. The limits are
/// about −8% and +22% of a typical word space (250/1000 em), the thresholds Adobe's composers are
/// described as using (John Hudson, www-style, 2012-07): condensing shows the fixed ligature
/// sooner than expanding. An explicit `liga` / `clig` in the character's own features always wins.
pub const LIGATURE_TRACKING_LIMITS: (f64, f64) = (-20.0, 55.0);

/// OpenType features applied during shaping. Kerning, `case` and ligature suppression are driven
/// by the character style (`kerning`, `all_caps`, `tracking`); the rest are layout-wide options.
/// Standard ligatures are dropped when tracking is outside [`LIGATURE_TRACKING_LIMITS`], unless
/// the character's features turn `liga` (or `clig`) on explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtFeatures {
    /// Vertical glyph alternates (`vert`), set by the writing direction.
    pub vertical: bool,
    /// Standard ligatures (`liga`, `clig`). Suppressed automatically when tracking is outside
    /// [`LIGATURE_TRACKING_LIMITS`], unless the character turns them on explicitly.
    pub ligatures: bool,
    /// Contextual alternates (`calt`).
    pub contextual: bool,
    /// Discretionary ligatures (`dlig`).
    pub discretionary_ligatures: bool,
    /// Small capitals (`smcp`).
    pub small_caps: bool,
    /// Diagonal fractions (`frac`).
    pub fractions: bool,
    /// Oldstyle figures (`onum`).
    pub oldstyle_figures: bool,
    /// Tabular figures (`tnum`).
    pub tabular_figures: bool,
    /// Ordinals (`ordn`).
    pub ordinals: bool,
    /// Swashes (`swsh`).
    pub swash: bool,
    /// The features of [`EXTRA_FEATURES`] turned on, one bit each (off by default): stylistic
    /// sets, alternates, titling, proportional and lining figures, superscript, subscript,
    /// numerators and denominators, `aalt`.
    pub extra: u32,
    /// The value of each feature of [`EXTRA_FEATURES`] that is on, as a tag `salt=2` gives it: for
    /// a feature with alternates, which of them the font draws (the HarfBuzz feature value, 1 for
    /// the first). 0 stands for 1, a plain `salt`. Only read for a feature whose bit of
    /// [`Self::extra`] is on; use [`Self::set_extra`] to change a bit and its value together.
    pub extra_values: [u16; EXTRA_FEATURES.len()],
    /// The value `swsh=N` gives swashes ([`Self::swash`]), as [`Self::extra_values`] for `salt`.
    pub swash_value: u16,
    /// Set a complex script (Indic, Arabic, Hebrew) glyph by glyph as the font maps its
    /// characters, with none of its layout tables: the Standard composers. Set per paragraph by
    /// layout from the paragraph's composer, not by a style.
    pub unshaped_complex: bool,
}

/// The largest value a `tag=N` feature takes: a font's glyph ids are 16 bits, so is an alternate's
/// position in a set.
pub const MAX_FEATURE_VALUE: u16 = u16::MAX;

/// The most entries of a style's feature list read: every segment of its text reads the list again.
pub const MAX_STYLE_FEATURES: usize = 256;

/// Features beyond the named fields, off unless asked for, each bit `i` of [`OtFeatures::extra`]:
/// stylistic sets `ss01`–`ss20`, stylistic alternates `salt`, titling `titl`, proportional `pnum`
/// and lining `lnum` figures, superscript `sups`, subscript `subs`, numerators `numr`,
/// denominators `dnom` and access all alternates `aalt`.
/// <https://learn.microsoft.com/en-us/typography/opentype/spec/features_ae#tag-aalt>
pub const EXTRA_FEATURES: [&str; 29] = [
    "ss01", "ss02", "ss03", "ss04", "ss05", "ss06", "ss07", "ss08", "ss09", "ss10", "ss11", "ss12", "ss13", "ss14", "ss15", "ss16", "ss17", "ss18",
    "ss19", "ss20", "salt", "titl", "pnum", "lnum", "sups", "subs", "numr", "dnom", "aalt",
];

impl Default for OtFeatures {
    fn default() -> Self {
        Self {
            vertical: false,
            ligatures: true,
            contextual: true,
            discretionary_ligatures: false,
            small_caps: false,
            fractions: false,
            oldstyle_figures: false,
            tabular_figures: false,
            ordinals: false,
            swash: false,
            extra: 0,
            extra_values: [0; EXTRA_FEATURES.len()],
            swash_value: 0,
            unshaped_complex: false,
        }
    }
}

/// A feature of `value` over the whole text: 1 and 0 turn it on and off, more picks the font's
/// alternate of that number.
fn f(tag: &[u8; 4], value: impl Into<u32>) -> Feature {
    Feature::new(Tag::new(tag), value.into(), ..)
}

/// What a feature tag of a style's list says about one feature ([`parse_tag`]).
struct ParsedTag<'a> {
    /// The tag without a `-` or `+` prefix or a `=N` value.
    name: &'a str,
    /// On (`tag`, `+tag`, `tag=N` for `N` from 1) or off (`-tag`, `tag=0`).
    on: bool,
    /// The value `tag=N` gives; 1 for a plain tag.
    value: u16,
}

/// Can feature `name` take a value (`salt=2`)? The features of [`EXTRA_FEATURES`] and `swsh` can:
/// each substitutes glyphs, and the value says which alternate.
fn takes_value(name: &str) -> bool {
    name == "swsh" || EXTRA_FEATURES.contains(&name)
}

/// Read a feature tag: `tag`, `+tag`, `-tag` or, for a feature that takes a value, `tag=N` (`N`
/// from 0 to [`MAX_FEATURE_VALUE`] in decimal digits; `-tag=N` is refused). `None` for a tag this
/// set does not understand.
fn parse_tag(tag: &str) -> Option<ParsedTag<'_>> {
    let (off, rest) = match tag.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, tag.strip_prefix('+').unwrap_or(tag)),
    };
    let (name, value) = match rest.split_once('=') {
        None => (rest, None),
        Some((name, digits)) => {
            if off || digits.is_empty() || digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) || !takes_value(name) {
                return None;
            }
            (name, Some(u16::try_from(digits.parse::<u32>().ok()?).ok()?))
        }
    };
    let known = matches!(name, "liga" | "calt" | "dlig" | "smcp" | "frac" | "onum" | "tnum" | "ordn" | "swsh") || EXTRA_FEATURES.contains(&name);
    known.then_some(ParsedTag { name, on: !off && value != Some(0), value: value.unwrap_or(1) })
}

impl OtFeatures {
    /// Parse a feature list like `["dlig", "smcp", "-liga"]` (unknown tags are ignored).
    pub fn from_tags<'a>(tags: impl IntoIterator<Item = &'a str>) -> Self {
        Self::default().with_tags(tags)
    }

    /// The tags that make `change` on any set, for a `change` that turns each feature on, turns it
    /// off or keeps it, whatever the other features are: `tag` for each feature it turns on, `-tag`
    /// for each it turns off, and none for one it keeps. [`Self::with_tags`] applies them.
    pub fn changes(change: impl Fn(&mut Self)) -> Vec<String> {
        let mut off = Self { ligatures: false, contextual: false, ..Self::default() };
        let mut on = Self {
            vertical: false,
            ligatures: true,
            contextual: true,
            discretionary_ligatures: true,
            small_caps: true,
            fractions: true,
            oldstyle_figures: true,
            tabular_figures: true,
            ordinals: true,
            swash: true,
            extra: u32::MAX >> (32 - EXTRA_FEATURES.len()),
            extra_values: [0; EXTRA_FEATURES.len()],
            swash_value: 0,
            unshaped_complex: false,
        };
        change(&mut off);
        change(&mut on);
        let named = ["liga", "calt", "dlig", "smcp", "frac", "onum", "tnum", "ordn", "swsh"];
        let mut v = vec![];
        for tag in named.into_iter().chain(EXTRA_FEATURES) {
            let turn_off = format!("-{tag}");
            let has = |f: &Self| f.with_tags([turn_off.as_str()]) != *f;
            match (has(&off), has(&on)) {
                (true, true) => v.push(tag.to_string()),
                (false, false) => v.push(turn_off.clone()),
                _ => {}
            }
        }
        v
    }

    /// The tags this set differs from the defaults by (what `CharStyle::features` stores).
    pub fn to_tags(&self) -> Vec<String> {
        let d = Self::default();
        let mut v = vec![];
        for (on, def, tag) in [
            (self.ligatures, d.ligatures, "liga"),
            (self.contextual, d.contextual, "calt"),
            (self.discretionary_ligatures, d.discretionary_ligatures, "dlig"),
            (self.small_caps, d.small_caps, "smcp"),
            (self.fractions, d.fractions, "frac"),
            (self.oldstyle_figures, d.oldstyle_figures, "onum"),
            (self.tabular_figures, d.tabular_figures, "tnum"),
            (self.ordinals, d.ordinals, "ordn"),
            (self.swash, d.swash, "swsh"),
        ] {
            if on != def {
                v.push(match (on, tag, self.swash_value) {
                    (true, "swsh", n @ 2..) => format!("{tag}={n}"),
                    (true, ..) => tag.to_string(),
                    (false, ..) => format!("-{tag}"),
                });
            }
        }
        v.extend(EXTRA_FEATURES.iter().enumerate().filter(|(i, _)| self.extra & (1 << i) != 0).map(|(i, t)| match self.extra_values.get(i) {
            None | Some(0 | 1) => t.to_string(),
            Some(n) => format!("{t}={n}"),
        }));
        v
    }

    /// Is `tag` one this set understands? A feature tag, optionally prefixed with `-` or `+`, or
    /// `tag=N` (`N` from 0 to [`MAX_FEATURE_VALUE`], no prefix but `+`) for a feature of
    /// [`EXTRA_FEATURES`] or `swsh`, which picks the font's alternate number `N` (`salt=2`; 0 is
    /// off, 1 is a plain tag).
    pub fn known_tag(tag: &str) -> bool {
        parse_tag(tag).is_some()
    }

    /// The feature a tag of a style's list names, without its `-` or `+` prefix or `=N` value
    /// (`salt` for `-salt`, `+salt` and `salt=2`).
    pub fn tag_name(tag: &str) -> &str {
        let t = tag.strip_prefix(['-', '+']).unwrap_or(tag);
        t.split_once('=').map_or(t, |(name, _)| name)
    }

    /// The value feature `tag` of [`EXTRA_FEATURES`] or `swsh` is on with: `Some(1)` for a plain
    /// tag, `Some(N)` for `tag=N`, `None` when it is off or is another tag.
    pub fn feature_value(&self, tag: &str) -> Option<u16> {
        if tag == "swsh" {
            return self.swash.then_some(self.swash_value.max(1));
        }
        let i = EXTRA_FEATURES.iter().position(|e| *e == tag)?;
        (self.extra & (1 << i) != 0).then(|| self.extra_values.get(i).copied().unwrap_or(0).max(1))
    }

    /// Turn the features of [`EXTRA_FEATURES`] in `mask` (bit `i` for the feature `i`) on, with
    /// value 1 (a plain tag), or off.
    pub fn set_extra(&mut self, mask: u32, on: bool) {
        for (i, value) in self.extra_values.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                *value = 0;
            }
        }
        if on {
            self.extra |= mask;
        } else {
            self.extra &= !mask;
        }
    }

    /// This set with `tags` applied on top.
    pub fn with_tags<'a>(mut self, tags: impl IntoIterator<Item = &'a str>) -> Self {
        let o = &mut self;
        for t in tags {
            let Some(ParsedTag { name, on, value }) = parse_tag(t) else { continue };
            // 1 is a plain tag: only a value from 2 is kept.
            let kept = if on && value > 1 { value } else { 0 };
            match name {
                "liga" => o.ligatures = on,
                "calt" => o.contextual = on,
                "dlig" => o.discretionary_ligatures = on,
                "smcp" => o.small_caps = on,
                "frac" => o.fractions = on,
                "onum" => o.oldstyle_figures = on,
                "tnum" => o.tabular_figures = on,
                "ordn" => o.ordinals = on,
                "swsh" => {
                    o.swash = on;
                    o.swash_value = kept;
                }
                _ => {
                    if let Some(i) = EXTRA_FEATURES.iter().position(|e| *e == name) {
                        o.set_extra(1 << i, on);
                        if let Some(slot) = o.extra_values.get_mut(i) {
                            *slot = kept;
                        }
                    }
                }
            }
        }
        self
    }

    /// The harfrust features for text in style `st`.
    pub(crate) fn resolve(&self, st: &CharStyle) -> Vec<Feature> {
        // The character's own OpenType settings override the layout-wide ones.
        // (A style lists a few features; one with thousands, in a document, is read for the first.)
        let listed = st.features.get(..MAX_STYLE_FEATURES).unwrap_or(&st.features);
        let own = self.with_tags(listed.iter().map(String::as_str));
        let s = &own;
        let mut v = Vec::with_capacity(8);
        if st.kerning.is_some() {
            v.push(f(b"kern", false));
        }
        let liga = s.ligatures && (!ligatures_suppressed_by(st.tracking) || explicit_ligatures(listed));
        if !liga {
            v.push(f(b"liga", false));
            v.push(f(b"clig", false));
        }
        if !s.contextual {
            v.push(f(b"calt", false));
        }
        if st.all_caps {
            v.push(f(b"case", true));
        }
        for (on, tag) in [
            (s.discretionary_ligatures, b"dlig"),
            (s.small_caps, b"smcp"),
            (s.fractions, b"frac"),
            (s.oldstyle_figures, b"onum"),
            (s.tabular_figures, b"tnum"),
            (s.ordinals, b"ordn"),
            (s.swash, b"swsh"),
        ] {
            if on {
                v.push(f(tag, if tag == b"swsh" { u32::from(s.swash_value.max(1)) } else { 1 }));
            }
        }
        for (i, tag) in EXTRA_FEATURES.iter().enumerate() {
            if let (true, Ok(t)) = (s.extra & (1 << i) != 0, <[u8; 4]>::try_from(tag.as_bytes())) {
                v.push(f(&t, u32::from(s.extra_values.get(i).copied().unwrap_or(0).max(1))));
            }
        }
        if self.vertical {
            // Not `vrt2`: its glyphs are already turned on their side, and the layout turns Latin
            // and digits itself (they would lie upside down, and tate-chu-yoko break).
            v.push(f(b"vert", true));
        }
        if self.proportional(st) {
            v.push(f(b"palt", true));
            // Kerning for the glyphs `palt` makes proportional (OpenType `apkn`), with automatic
            // kerning only.
            v.push(f(b"apkn", st.kerning.is_none()));
        }
        v
    }

    /// Does text in style `st` take proportional widths (`palt`)? Proportional Metrics in
    /// horizontal type; vertical type takes proportional heights instead
    /// ([`Self::proportional_vertical`]).
    pub(crate) fn proportional(&self, st: &CharStyle) -> bool {
        st.proportional_metrics && !self.vertical
    }

    /// Does text in style `st` take proportional heights (`vpal`)? Proportional Metrics in
    /// vertical type.
    pub(crate) fn proportional_vertical(&self, st: &CharStyle) -> bool {
        st.proportional_metrics && self.vertical
    }

    /// The harfrust features for vertical text in style `st` shaped top to bottom: [`Self::resolve`],
    /// with `vpal` when `proportional`, and vertical kerning (`vkrn`, and `vapk` for the glyphs
    /// `vpal` makes proportional) when `kerning` and the style's kerning is automatic.
    pub(crate) fn resolve_vertical(&self, st: &CharStyle, proportional: bool, kerning: bool) -> Vec<Feature> {
        let mut v = self.resolve(st);
        if proportional {
            v.push(f(b"vpal", true));
        }
        let kern = kerning && st.kerning.is_none();
        v.push(f(b"vkrn", kern));
        v.push(f(b"vapk", kern));
        v
    }

    /// The harfrust features for text in style `st` with full-width glyphs on their full widths:
    /// [`Self::resolve`] without `palt` (and its kerning, `apkn`).
    pub(crate) fn resolve_fixed_width(&self, st: &CharStyle) -> Vec<Feature> {
        let mut v = self.resolve(st);
        v.retain(|x| x.tag != Tag::new(b"palt") && x.tag != Tag::new(b"apkn"));
        v
    }
}

/// Does `tracking` (1/1000 em) count as letterspacing that drops standard ligatures?
pub fn ligatures_suppressed_by(tracking: f64) -> bool {
    // A non-finite value is not a real letterspacing request; keep the default.
    let (tight, loose) = LIGATURE_TRACKING_LIMITS;
    tracking.is_finite() && (tracking < tight || tracking > loose)
}

/// Do `features` turn standard ligatures on explicitly (`liga`, `+liga`, `clig`, `+clig`)? The last
/// ligature tag wins, matching [`OtFeatures::with_tags`]. Absence of a tag means "default", which
/// [`OtFeatures::to_tags`] never writes, so this is distinguishable from an explicit request.
pub fn explicit_ligatures<S: AsRef<str>>(features: &[S]) -> bool {
    features
        .iter()
        .rev()
        .find_map(|t| match t.as_ref() {
            "liga" | "+liga" | "clig" | "+clig" => Some(true),
            "-liga" | "-clig" => Some(false),
            _ => None,
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1607: stylistic sets, alternates, titling, figure styles and superscript/subscript are
    /// known, round-trip through the stored tags and reach the shaper.
    #[test]
    fn extra_features_are_kept_and_shaped() {
        for t in ["ss01", "ss20", "-ss07", "salt", "titl", "pnum", "lnum", "sups", "subs"] {
            assert!(OtFeatures::known_tag(t), "{t}");
        }
        assert!(!OtFeatures::known_tag("ss21"));
        for tag in ["--liga", "+-dnom", "-+ss01", "++liga"] {
            assert!(!OtFeatures::known_tag(tag), "{tag}");
        }
        let o = OtFeatures::from_tags(["ss03", "pnum", "sups", "-liga"]);
        assert_eq!(o.to_tags(), vec!["-liga", "ss03", "pnum", "sups"]);
        assert_eq!(OtFeatures::from_tags(["ss03", "-ss03"]).to_tags(), Vec::<String>::new(), "turned off again");
        let st = CharStyle { features: vec!["ss05".into(), "titl".into()], ..CharStyle::default() };
        let tags: Vec<String> = OtFeatures::default().resolve(&st).iter().filter(|f| f.value == 1).map(|f| f.tag.to_string()).collect();
        assert!(tags.contains(&"ss05".to_string()) && tags.contains(&"titl".to_string()), "{tags:?}");
    }

    /// Numerators and denominators (the OpenType panel's Position menu) are known, round-trip
    /// through the stored tags and reach the shaper.
    #[test]
    fn numerators_and_denominators_are_kept_and_shaped() {
        for t in ["numr", "dnom", "-numr", "+dnom"] {
            assert!(OtFeatures::known_tag(t), "{t}");
        }
        let o = OtFeatures::from_tags(["dnom", "sups", "numr"]);
        assert_eq!(o.to_tags(), vec!["sups", "numr", "dnom"]);
        assert_eq!(OtFeatures::from_tags(["numr", "dnom", "-numr"]).to_tags(), ["dnom"]);
        for tag in ["numr", "dnom"] {
            let st = CharStyle { features: vec![tag.into()], ..CharStyle::default() };
            let on: Vec<String> = OtFeatures::default().resolve(&st).iter().filter(|f| f.value == 1).map(|f| f.tag.to_string()).collect();
            assert_eq!(on, [tag], "{tag}");
        }
    }

    /// `changes` lists the features a change turns on and off and leaves out the ones it keeps,
    /// and applying them to any set makes the same change.
    #[test]
    fn changes_list_what_turns_on_and_off() {
        let figure = |f: &mut OtFeatures| {
            f.tabular_figures = true;
            f.oldstyle_figures = false;
            f.extra &= !(1 << 22);
        };
        assert_eq!(OtFeatures::changes(figure), ["-onum", "tnum", "-pnum"]);
        assert_eq!(OtFeatures::changes(|f| f.ligatures = false), ["-liga"]);
        assert_eq!(OtFeatures::changes(|f| f.extra |= 1 << 27), ["dnom"]);
        assert!(OtFeatures::changes(|_| {}).is_empty());
        for start in [&["lnum", "smcp"][..], &["pnum", "onum", "-liga"][..], &[][..]] {
            let mut want = OtFeatures::from_tags(start.iter().copied());
            figure(&mut want);
            let tags = OtFeatures::changes(figure);
            assert_eq!(OtFeatures::from_tags(start.iter().copied()).with_tags(tags.iter().map(String::as_str)), want, "{start:?}");
        }
    }

    /// `tag=N` gives a feature a value: parsed, validated, kept in the style's list as written
    /// (1 is a plain tag, 0 is off), replaced by the next value or by `-tag`, and given to the
    /// shaper as that value.
    #[test]
    fn feature_values_are_parsed_validated_and_shaped() {
        for t in ["salt=2", "+salt=2", "ss03=12", "aalt=65535", "swsh=3", "salt=0", "salt=01", "titl=2"] {
            assert!(OtFeatures::known_tag(t), "{t}");
        }
        for t in [
            "salt=",
            "salt=-1",
            "salt=65536",
            "salt=1.5",
            "salt=x",
            "-salt=2",
            "liga=2",
            "dlig=1",
            "zzzz=2",
            "salt==2",
            "=2",
            "salt=2=3",
            "salt=123456",
            "salt= 2",
        ] {
            assert!(!OtFeatures::known_tag(t), "{t}");
        }
        assert_eq!(OtFeatures::tag_name("salt=2"), "salt");
        assert_eq!((OtFeatures::tag_name("-ss01"), OtFeatures::tag_name("+salt=3"), OtFeatures::tag_name("liga")), ("ss01", "salt", "liga"));
        let o = OtFeatures::from_tags(["salt=2", "ss01", "-liga", "swsh=4"]);
        assert_eq!(o.to_tags(), ["-liga", "swsh=4", "ss01", "salt=2"]);
        assert_eq!(
            (o.feature_value("salt"), o.feature_value("ss01"), o.feature_value("swsh"), o.feature_value("ss02")),
            (Some(2), Some(1), Some(4), None)
        );
        // The next value replaces the last, `salt=1` is a plain `salt`, `salt=0` and `-salt` are off.
        assert_eq!(OtFeatures::from_tags(["salt=2", "salt=3"]).to_tags(), ["salt=3"]);
        assert_eq!(OtFeatures::from_tags(["salt=3", "salt"]).to_tags(), ["salt"]);
        assert_eq!(OtFeatures::from_tags(["salt=1"]).to_tags(), ["salt"]);
        assert_eq!(OtFeatures::from_tags(["salt=2", "-salt"]).to_tags(), Vec::<String>::new());
        assert_eq!(OtFeatures::from_tags(["salt=2", "salt=0"]).to_tags(), Vec::<String>::new());
        assert_eq!(OtFeatures::from_tags(["salt=2", "-salt", "salt"]).to_tags(), ["salt"], "a value does not outlive being turned off");
        // A tag with a value this set does not know is ignored.
        assert_eq!(OtFeatures::from_tags(["salt=x", "liga=2"]), OtFeatures::default());
        // The panel's own switches turn a feature on at value 1.
        let mut f = OtFeatures::from_tags(["salt=3"]);
        f.set_extra(1 << 20, false);
        f.set_extra(1 << 20, true);
        assert_eq!(f.to_tags(), ["salt"]);
        let values = |tags: &[&str]| -> Vec<(String, u32)> {
            let st = CharStyle { features: tags.iter().map(|t| t.to_string()).collect(), ..CharStyle::default() };
            OtFeatures::default()
                .resolve(&st)
                .iter()
                .filter(|f| f.value > 0 && f.tag != harfrust::Tag::new(b"kern"))
                .map(|f| (f.tag.to_string(), f.value))
                .collect()
        };
        assert_eq!(values(&["salt=2"]), [("salt".to_string(), 2)]);
        assert_eq!(values(&["salt"]), [("salt".to_string(), 1)]);
        assert_eq!(values(&["swsh=3", "ss01"]), [("swsh".to_string(), 3), ("ss01".to_string(), 1)]);
        assert!(values(&["-salt", "salt=0"]).is_empty());
    }
    use crate::{FontDb, layout};
    use vectorcraft_doc::TextObject;
    use vectorcraft_geom::Point;

    fn glyphs(features: &[&str]) -> usize {
        let st = CharStyle { font_family: "Source Serif 4".into(), features: features.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        layout(FontDb::global(), &TextObject::point(Point::ZERO, "fi", st)).glyphs.len()
    }

    #[test]
    fn per_character_features_drive_shaping() {
        let with = glyphs(&[]);
        let without = glyphs(&["-liga"]);
        assert!(with < without, "the default ligature merges f+i ({with} vs {without})");
        assert_eq!(OtFeatures::from_tags(["-liga", "dlig"]).to_tags(), ["-liga", "dlig"]);
        assert!(OtFeatures::default().to_tags().is_empty());
        assert!(OtFeatures::known_tag("-onum") && !OtFeatures::known_tag("zzzz"));
    }

    /// The characters' language reaches the shaper (#4940): Source Serif 4's Turkish forms
    /// (`locl`) keep f and i apart, and the dot on the i, where English ligates them. Runs in
    /// different languages are shaped apart, each in its own.
    #[test]
    fn language_picks_the_fonts_forms() {
        let st =
            |language: Option<&str>| CharStyle { font_family: "Source Serif 4".into(), language: language.map(str::to_string), ..Default::default() };
        let count = |language| layout(FontDb::global(), &TextObject::point(Point::ZERO, "fi", st(language))).glyphs.len();
        assert_eq!((count(None), count(Some("en-US")), count(Some("tr"))), (1, 1, 2));
        // A tag that isn't one (from a file) shapes without a language.
        assert_eq!((count(Some("tr; DROP")), count(Some(""))), (1, 1));
        let mut t = TextObject::point(Point::ZERO, "", st(None));
        t.runs = vec![vectorcraft_doc::TextRun::new("fi", st(Some("tr"))), vectorcraft_doc::TextRun::new("fi", st(None))];
        let glyphs = layout(FontDb::global(), &t).glyphs;
        let per_run: Vec<usize> = (0..2).map(|r| glyphs.iter().filter(|g| g.run == r).count()).collect();
        assert_eq!(per_run, [2, 1]);
    }

    /// Bulgarian text gets the font's Bulgarian forms (`locl` for `BGR`, #5945): in the bundled
    /// Source Sans 3 Italic, в, д, ж, з, к, л and ю differ from the Russian ones, and letters
    /// the font draws alike stay alike.
    #[test]
    fn bulgarian_gets_the_fonts_bulgarian_forms() {
        let st = |language: &str| CharStyle {
            font_family: "Source Sans 3".into(),
            font_style: "Italic".into(),
            language: Some(language.to_string()),
            ..Default::default()
        };
        // Each letter laid out on its own, so one letter's advance doesn't move the next.
        let outline = |c: char, language| {
            layout(FontDb::global(), &TextObject::point(Point::ZERO, &c.to_string(), st(language)))
                .glyphs
                .into_iter()
                .map(|g| g.outline)
                .collect::<Vec<_>>()
        };
        let differs: String = "вгджзийклптцшщюьъ".chars().filter(|c| outline(*c, "bg") != outline(*c, "ru")).collect();
        assert_eq!(differs, "вджзклю");
    }

    /// Serbian text gets the font's Serbian forms (`locl` for `SRB`): in the bundled Source Sans
    /// 3 Italic, б, г, д, п, т differ from the Russian ones, in either Serbian list entry.
    #[test]
    fn serbian_gets_the_fonts_serbian_forms() {
        let st = |language: &str| CharStyle {
            font_family: "Source Sans 3".into(),
            font_style: "Italic".into(),
            language: Some(language.to_string()),
            ..Default::default()
        };
        let outlines = |language| {
            layout(FontDb::global(), &TextObject::point(Point::ZERO, "бгдпт", st(language))).glyphs.into_iter().map(|g| g.outline).collect::<Vec<_>>()
        };
        let (serbian, russian) = (outlines("sr-Cyrl"), outlines("ru"));
        assert_eq!((serbian.len(), russian.len()), (5, 5));
        assert!(serbian.iter().zip(&russian).all(|(s, r)| s != r), "every letter in its Serbian form");
        assert_eq!(outlines("sr-Latn"), serbian);
    }
}
