//! Helpers for the number, blend and kerning fields the panels build from framework components.

use craft::behavior::NumberCodec;
use craft::ui::contract::{Bind, Entry};
use craft::ui::patterns::blend_keys::{BlendKeyEntry, BlendShortcut, blend_entries as framework_blend_entries};
use craft::ui::runtime::env::{Env, Label, LengthUnits, UnitKind, Units};
use craft::ui::text::NumberField;
use egui::Ui;
use vectorcraft_color::BlendMode;

/// Every blend mode with its Shift+Alt letter, in the order of the menu.
fn blend_table() -> Vec<(BlendMode, BlendShortcut)> {
    use BlendShortcut as S;
    BlendMode::ALL
        .into_iter()
        .map(|mode| {
            let shortcut = match mode {
                BlendMode::Normal => S::Normal,
                BlendMode::Darken => S::Darken,
                BlendMode::Multiply => S::Multiply,
                BlendMode::ColorBurn => S::ColorBurn,
                BlendMode::Lighten => S::Lighten,
                BlendMode::Screen => S::Screen,
                BlendMode::ColorDodge => S::ColorDodge,
                BlendMode::Overlay => S::Overlay,
                BlendMode::SoftLight => S::SoftLight,
                BlendMode::HardLight => S::HardLight,
                BlendMode::Difference => S::Difference,
                BlendMode::Exclusion => S::Exclusion,
                BlendMode::Hue => S::Hue,
                BlendMode::Saturation => S::Saturation,
                BlendMode::Color => S::Color,
                BlendMode::Luminosity => S::Luminosity,
            };
            (mode, shortcut)
        })
        .collect()
}

/// The blend-mode list: Normal | darken | lighten | contrast | inversion | component modes, the
/// groups separated where the framework's blend list separates them.
pub fn blend_entries() -> Vec<Entry<BlendMode>> {
    let entries: Vec<_> = blend_table().into_iter().map(|(mode, shortcut)| BlendKeyEntry::new(mode, shortcut)).collect();
    framework_blend_entries(&entries, |entry| mode_label(entry.key))
}

/// `mode`'s name in the interface language.
fn mode_label(mode: BlendMode) -> Label {
    Label::from(crate::i18n::t(mode.label()))
}

/// `lengths` with `bare_points` (Numbers Without Units Are Points) applied: a bare number in a picas
/// field counts as points.
pub fn bare_points(lengths: LengthUnits, on: bool) -> LengthUnits {
    if on && lengths.shown == UnitKind::Picas { lengths.bare_in(UnitKind::Points) } else { lengths }
}

/// The codec of a size field: a length field's reader at the shown unit's places, and a lone `50%` read as that share
/// of the size the edit began on.
pub fn size_codec(units: &Units, lengths: LengthUnits) -> NumberCodec {
    NumberCodec::new(units.length_reader(lengths), *lengths.shown.decimals().end()).share_of_start()
}

/// A length field over a size (a width, height, radius or handle length) of `value`, in `lengths.stored`, read with
/// [`size_codec`].
pub fn size_field(ui: &Ui, label: impl Into<Label>, value: Option<f64>, lengths: LengthUnits) -> NumberField<'static> {
    NumberField::length(label, Bind::given(value), lengths).codec(size_codec(&Env::of(ui).units(), lengths))
}

/// `v` moved `n` steps of `size` (down when negative): the first step goes to the next multiple of
/// `size` in its direction, each further one adds or takes one `size`. By 1, 5.1 goes up to 6 and
/// down to 5, and 5 goes up to 6 and down to 4. A value within a millionth of a step of a multiple
/// counts as that multiple.
fn step_to_multiple(v: f64, size: f64, n: f64) -> f64 {
    if n == 0.0 {
        return v;
    }
    let q = v / size;
    let from = if n > 0.0 { (q + 1e-6).floor() } else { (q - 1e-6).ceil() };
    (from + n) * size
}

/// `v` moved `n` steps of `size` (down when negative), where each step goes to the nearer of the
/// next multiple of `size` and the next of `stops` in its direction. By 1 with stops at 0.25, 0.5
/// and 0.75, 0.52 goes down to 0.5 and up to 0.75, 1 goes down to 0.75, and 14 goes up to 15. A
/// stop or multiple that `same` finds equal to the value counts as reached: a stop shown with the
/// same text as the value (`0.167 p` for 2 pt) moves on to the next.
fn step_through(v: f64, size: f64, n: f64, stops: &[f64], same: impl Fn(f64, f64) -> bool) -> f64 {
    let mut v = v;
    // One call takes at most 1,000 steps.
    for _ in 0..n.abs().min(1000.0) as u32 {
        let mut multiple = step_to_multiple(v, size, n.signum());
        if same(multiple, v) {
            multiple = step_to_multiple(multiple, size, n.signum());
        }
        let ahead = |s: f64| (if n > 0.0 { s > v } else { s < v }) && !same(s, v);
        let stop = stops.iter().copied().filter(|&s| ahead(s));
        v = if n > 0.0 { stop.fold(multiple, f64::min) } else { stop.fold(multiple, f64::max) };
    }
    v
}

/// The stepper of a field whose plain step stops at each of `stops` (a stroke weight at its
/// presets: 0.52 pt goes down to 0.5 pt, not to 0 pt). `size` is the step, `shown` the text of a
/// value, so a stop shown as the value is passed.
pub fn stops_stepper<'a>(size: f64, stops: &'a [f64], shown: impl Fn(f64) -> String + 'a) -> impl Fn(f64, i32, f64) -> f64 + 'a {
    move |from, direction, scale| step_through(from, size * scale, f64::from(direction), stops, |a, b| shown(a) == shown(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn within_a_millionth(size: f64) -> impl Fn(f64, f64) -> bool {
        move |a, b| (a - b).abs() <= size.abs() * 1e-6
    }

    /// Picas to three places: 2 pt and 2.004 pt both show as `0.167`.
    fn picas(v: f64) -> String {
        format!("{:.3}", v / 12.0)
    }

    #[test]
    fn a_plain_spinner_step_stops_at_the_stops() {
        let stops = [0.25, 0.5, 0.75, 1.0, 2.0, 10.0, 20.0];
        let step = |v: f64, n: f64| step_through(v, 1.0, n, &stops, within_a_millionth(1.0));
        assert_eq!(step(0.52, -1.0), 0.5);
        assert_eq!(step(0.52, 1.0), 0.75);
        assert_eq!(step(0.1, -1.0), 0.0);
        assert_eq!(step(14.0, 1.0), 15.0);
    }

    /// A stop the field's text shows as the value counts as reached: 2.004 pt and the 2 pt stop
    /// both show as `0.167 p`, so a step down goes on to 1 pt.
    #[test]
    fn a_spinner_stop_shown_as_the_value_is_passed() {
        let same = |a: f64, b: f64| picas(a) == picas(b);
        assert_eq!(step_through(2.004, 1.0, -1.0, &[1.0, 2.0, 3.0], same), 1.0);
        assert_eq!(step_through(2.004, 1.0, 1.0, &[1.0, 2.0, 3.0], same), 3.0);
    }

    /// Each step goes to the nearer of the next whole step and the next stop in its direction.
    #[test]
    fn a_step_goes_to_the_nearer_of_the_next_multiple_and_the_next_stop() {
        let stops = [0.25, 0.5, 0.75, 1.0, 2.0, 3.0, 10.0, 20.0, 100.0];
        for (v, n, want) in [
            (0.52, -1.0, 0.5),
            (0.51, -1.0, 0.5),
            (0.4, -1.0, 0.25),
            (0.52, 1.0, 0.75),
            (0.1, 1.0, 0.25),
            (0.1, -1.0, 0.0),
            (0.25, -1.0, 0.0),
            (0.75, -1.0, 0.5),
            (1.0, -1.0, 0.75),
            (1.0, 1.0, 2.0),
            (1.5, -1.0, 1.0),
            (14.0, 1.0, 15.0),
            (14.0, -1.0, 13.0),
            (14.5, 1.0, 15.0),
            (0.4, -2.0, 0.0),
            (0.4, 3.0, 1.0),
            (100.0, 1.0, 101.0),
            (0.52, 0.0, 0.52),
        ] {
            let got = step_through(v, 1.0, n, &stops, within_a_millionth(1.0));
            assert!((got - want).abs() < 1e-9, "{v} × {n}: {got}");
        }
    }

    /// A value a float error away from a stop moves on to the next one.
    #[test]
    fn a_value_next_to_a_stop_moves_past_it() {
        assert_eq!(step_through(0.5 + 1e-9, 1.0, -1.0, &[0.25, 0.5], within_a_millionth(1.0)), 0.25);
        assert_eq!(step_through(0.5 - 1e-9, 1.0, 1.0, &[0.5, 0.75], within_a_millionth(1.0)), 0.75);
    }

    /// A stop shown with the same text as the value moves on to the next one: 2 pt shows as
    /// `0.167 p`.
    #[test]
    fn a_stop_shown_as_the_value_moves_past_it() {
        let same = |a: f64, b: f64| picas(a * 12.0) == picas(b * 12.0);
        let stops = [1.0 / 12.0, 2.0 / 12.0, 3.0 / 12.0];
        assert_eq!(step_through(0.167, 1.0, -1.0, &stops, same), 1.0 / 12.0);
        assert_eq!(step_through(0.083, 1.0, 1.0, &stops, same), 2.0 / 12.0);
    }
}
