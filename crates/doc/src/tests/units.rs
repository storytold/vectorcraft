//! Units: format/parse round-trips in every unit, typed units winning over the field's unit,
//! percentages in arithmetic, names and canvas readouts.

use proptest::prelude::*;

use crate::Unit;

#[test]
fn every_unit_round_trips_through_its_field_text() {
    for u in Unit::ALL {
        // Field text keeps three decimals of the unit: half a step is the most a round trip loses.
        let tol = 0.0005 * u.points() + 1e-9;
        for pt in [0.0, 1.0, 12.5, 72.0, 100.0, 595.28, 1234.5678, -36.0] {
            let shown = u.format(pt);
            let back = u.parse(&shown).unwrap_or_else(|| panic!("{u:?} can't read back `{shown}`"));
            assert!((back - pt).abs() <= tol, "{u:?}: {pt} → `{shown}` → {back}");
            // The bare number reads in the field's unit, and in any unit with its suffix.
            let bare = u.parse(&u.number(pt)).unwrap();
            assert!((bare - pt).abs() <= tol, "{u:?}: bare `{}` → {bare}", u.number(pt));
            let pts = Unit::Points.parse(&shown).unwrap();
            assert!((pts - pt).abs() <= tol, "{u:?}: `{shown}` in a points field → {pts}");
        }
    }
}

#[test]
fn an_explicit_leading_plus_is_accepted() {
    assert_eq!(Unit::Points.parse("+5"), Some(5.0));
    assert_eq!(Unit::Millimeters.parse("+5mm"), Some(Unit::Millimeters.to_pt(5.0)));
    assert_eq!(Unit::Points.parse("+5+3"), Some(8.0));
    assert_eq!(Unit::Points.parse("5+-3"), Some(2.0));
}

#[test]
fn typed_units_win_over_the_fields_unit() {
    let mm = 72.0 / 25.4;
    assert_eq!(Unit::Millimeters.parse("10"), Some(10.0 * mm));
    assert_eq!(Unit::Millimeters.parse("12 pt"), Some(12.0));
    assert_eq!(Unit::Millimeters.parse("1in"), Some(72.0));
    assert_eq!(Unit::Points.parse("5 mm"), Some(5.0 * mm));
    assert_eq!(Unit::Centimeters.parse("2p6"), Some(30.0));
    assert_eq!(Unit::Millimeters.parse("10+5"), Some(15.0 * mm));
    assert_eq!(Unit::Inches.parse("1 + 36 pt"), Some(108.0));
    assert_eq!(Unit::Pixels.parse("-4"), Some(-4.0));
}

#[test]
fn a_percentage_is_a_share_of_the_value_before_it() {
    let (mm, cm) = (72.0 / 25.4, 72.0 / 2.54);
    for (u, s, want) in [
        (Unit::Centimeters, "3cm*50%", 1.5 * cm),
        (Unit::Points, "50pt+25%", 62.5),
        (Unit::Points, "50pt-25%", 37.5),
        (Unit::Millimeters, "10*50%", 5.0 * mm),
        (Unit::Centimeters, "3cm/50%", 6.0 * cm),
        (Unit::Points, "50 pt + 25 %", 62.5),
        (Unit::Points, "10+20+50%", 45.0),
        // A percentage added or taken away is a share of everything before it. A percentage factor
        // is a share of the term before it.
        (Unit::Points, "100pt+20pt-50%", 60.0),
        (Unit::Points, "100pt-20pt+50%", 120.0),
        (Unit::Points, "100pt+20%-10pt", 110.0),
        (Unit::Points, "10+20*50%", 20.0),
        (Unit::Points, "10*2+50%", 30.0),
        (Unit::Points, "10*2/50%", 40.0),
        (Unit::Points, "10+-50%", 5.0),
        (Unit::Points, "50pt--25%", 62.5),
        (Unit::Centimeters, "3cm*-50%", -1.5 * cm),
    ] {
        let got = u.parse(s).unwrap_or_else(|| panic!("{u:?} can't read `{s}`"));
        assert!((got - want).abs() < 1e-9, "{u:?}: `{s}` → {got}, want {want}");
    }
}

#[test]
fn odd_text_reads_as_nothing_without_panicking() {
    for s in ["", "é", "é5", "°+1", "×2", "5 ×", "—3", "mm", "1/0", "--", "+", "50%", "%", "5%%", "%5", "1/0%", "10+%", "10*5pt%"] {
        assert_eq!(Unit::Millimeters.parse(s), None, "{s}");
    }
}

proptest! {
    /// Field text made of digits, operators, `%`, spaces and unit letters reads as a finite value
    /// or as nothing and does not panic.
    #[test]
    fn arithmetic_text_reads_as_a_finite_value_or_nothing(s in "[0-9.+*/% ptxinmcfyd-]{0,24}") {
        for u in Unit::ALL {
            if let Some(v) = u.parse(&s) {
                prop_assert!(v.is_finite(), "{u:?}: `{s}` → {v}");
            }
        }
    }
}

#[test]
fn units_are_named_by_label_key_or_suffix() {
    for u in Unit::ALL {
        assert_eq!(Unit::named(u.label()), Some(u));
        assert_eq!(Unit::named(u.key()), Some(u));
        assert_eq!(Unit::named(&u.label().to_uppercase()), Some(u));
    }
    assert_eq!(Unit::named("mm"), Some(Unit::Millimeters));
    assert_eq!(Unit::named("Feet & Inches"), Some(Unit::FeetInches));
    assert_eq!(Unit::named("feetInches"), Some(Unit::FeetInches));
    assert_eq!(Unit::named("furlongs"), None);
    assert_eq!(Unit::named(""), None);
}

#[test]
fn a_shared_suffix_names_the_plain_unit() {
    assert_eq!(Unit::named(Unit::Feet.suffix()), Some(Unit::Feet));
    assert_eq!(Unit::named("FT"), Some(Unit::Feet));
    assert_eq!(Unit::named(Unit::FeetInches.label()), Some(Unit::FeetInches));
}

#[test]
fn readouts_show_two_decimals() {
    assert_eq!(Unit::Points.readout(15.0), "15.00 pt");
    assert_eq!(Unit::Millimeters.readout(72.0), "25.40 mm");
    assert_eq!(Unit::Inches.readout(-36.0), "-0.50 in");
    assert_eq!(Unit::Millimeters.number(1.0), "0.3528");
    // Four decimals for the large units, three for the small ones.
    assert_eq!(Unit::Inches.number(0.5616), "0.0078");
    assert_eq!((Unit::Inches.number(595.2756), Unit::Points.number(595.2756)), ("8.2677".into(), "595.276".into()));
    assert_eq!(Unit::Points.number(-0.00001), "0");
}

#[test]
fn a_negative_operand_after_an_operator_is_a_sign() {
    assert_eq!(Unit::Points.parse("10*-2"), Some(-20.0));
    assert_eq!(Unit::Points.parse("-10/-2"), Some(5.0));
    assert_eq!(Unit::Points.parse("5--3"), Some(8.0));
    assert_eq!(Unit::Points.parse("5 - -3"), Some(8.0));
    assert_eq!(Unit::Points.parse("5+-3"), Some(2.0));
    assert_eq!(Unit::Points.parse("10-2"), Some(8.0));
    assert_eq!(crate::parse_number("90*-1"), Some(-90.0));
}

#[test]
fn mixed_mul_div() {
    // `*` and `/` share one precedence level and run left to right.
    assert_eq!(Unit::Points.parse("10/2*5"), Some(25.0));
    assert_eq!(crate::parse_number("100*3/4"), Some(75.0));
    assert_eq!(Unit::Points.parse("10*2/5"), Some(4.0));
    assert_eq!(Unit::Points.parse("10*2/5 px"), Some(Unit::Pixels.to_pt(4.0)));
}

/// Q and Ha are a quarter of a millimetre each, written back and read in any field, with or
/// without their suffix.
#[test]
fn q_and_ha_are_a_quarter_millimetre() {
    for u in [Unit::Q, Unit::Ha] {
        assert!((u.points() - Unit::Millimeters.points() / 4.0).abs() < 1e-12, "{u:?}");
        assert!((u.to_pt(4.0) - Unit::Millimeters.to_pt(1.0)).abs() < 1e-12, "{u:?}");
        let tol = 0.0005 * u.points() + 1e-9;
        for pt in [0.0, 1.0, 12.5, 72.0, 100.0, 595.28, 1234.5678, -36.0] {
            let shown = u.format(pt);
            let back = u.parse(&shown).unwrap_or_else(|| panic!("{u:?} can't read back `{shown}`"));
            assert!((back - pt).abs() <= tol, "{u:?}: {pt} → `{shown}` → {back}");
            let bare = u.parse(&u.number(pt)).unwrap();
            assert!((bare - pt).abs() <= tol, "{u:?}: bare `{}` → {bare}", u.number(pt));
            let pts = Unit::Points.parse(&shown).unwrap();
            assert!((pts - pt).abs() <= tol, "{u:?}: `{shown}` in a points field → {pts}");
        }
    }
    assert_eq!(Unit::Q.format(72.0 / 25.4), "4 Q");
    assert_eq!(Unit::Ha.format(Unit::Millimeters.to_pt(2.5)), "10 H");
    // 13 Q is 3.25 mm, however it is typed.
    let want = Unit::Millimeters.to_pt(3.25);
    for typed in ["13q", "13 Q", "13Q+0", "26/2 q"] {
        let got = Unit::Points.parse(typed).unwrap_or_else(|| panic!("`{typed}` reads as nothing"));
        assert!((got - want).abs() < 1e-9, "`{typed}` → {got}");
    }
    assert!((Unit::Q.parse("13").unwrap() - want).abs() < 1e-9, "a bare number is in the field's unit");
    assert!((Unit::Millimeters.parse("13 h").unwrap() - want).abs() < 1e-9, "a typed unit wins over the field's");
    for typed in ["13h", "13 H", "13Ha", "13 ha"] {
        assert!((Unit::Points.parse(typed).unwrap() - want).abs() < 1e-9, "`{typed}`");
    }
    // Mixed with other units.
    assert!((Unit::Points.parse("4q+1mm").unwrap() - Unit::Millimeters.to_pt(2.0)).abs() < 1e-9);
}

/// Only Preferences › Units › East Asian Type offers Q and Ha: the list every other place shares
/// leaves them out, and `named` answers for it alone.
#[test]
fn only_the_east_asian_type_list_names_q_and_ha() {
    assert_eq!(Unit::EAST_ASIAN[..10], Unit::ALL);
    assert_eq!(Unit::EAST_ASIAN[10..], [Unit::Q, Unit::Ha]);
    for u in Unit::EAST_ASIAN {
        assert_eq!(Unit::named_east_asian(u.label()), Some(u));
        assert_eq!(Unit::named_east_asian(u.key()), Some(u));
        assert_eq!(Unit::named_east_asian(&u.label().to_uppercase()), Some(u));
        assert_eq!(Unit::named_east_asian(u.suffix()), if u == Unit::FeetInches { Some(Unit::Feet) } else { Some(u) });
    }
    for name in ["q", "Q", "ha", "Ha", "H", "h"] {
        assert_eq!(Unit::named(name), None, "`{name}`");
    }
    assert_eq!(Unit::named_east_asian("Q"), Some(Unit::Q));
    assert_eq!(Unit::named_east_asian("h"), Some(Unit::Ha));
    assert_eq!(Unit::named_east_asian("Ha"), Some(Unit::Ha));
    assert_eq!(Unit::named_east_asian("furlongs"), None);
}
