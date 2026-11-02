use serde_json::{Value, json};
use vectorcraft_doc::Effect;
use vectorcraft_doc::color::Color;

use super::*;
use crate::{ColorMap, color_map};

fn map(params: Value) -> ColorMap {
    color_map(&[Effect { id: DUOTONE.into(), params, visible: true }]).unwrap()
}

/// `c` with link `link` through `m`, the document having spot swatch "Ink" and process swatch
/// "Blue": (colour, link, tint, changed).
fn linked(m: &ColorMap, c: Color, link: Option<&str>) -> (Color, Option<String>, f32, bool) {
    let (mut c, mut link, mut tint) = (c, link.map(str::to_string), 1.0);
    let kinds = |n: &str| match n {
        "Ink" => Some(true),
        "Blue" => Some(false),
        _ => None,
    };
    let changed = m.apply_link(&mut c, &mut link, &mut tint, &kinds);
    (c, link, tint, changed)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn looks(c: Color, rgb: [f32; 3]) -> bool {
    c.to_rgb().iter().zip(rgb).all(|(a, b)| close(*a, b))
}

const INK: Color = Color::Cmyk { c: 0.0, m: 0.5, y: 1.0, k: 0.0 };

fn ink_tone(curve: &str) -> Value {
    json!({"tones": [{"color": INK, "swatch": "Ink", "curve": curve}]})
}

#[test]
fn a_spot_tone_turns_greys_into_tints_of_its_swatch() {
    let m = map(ink_tone("0,0 100,100"));
    for k in [0.0, 0.25, 0.6, 1.0] {
        let (c, link, tint, _) = linked(&m, Color::gray(k), None);
        assert_eq!(link.as_deref(), Some("Ink"), "linked at {k}");
        assert!(close(tint, k), "the tint is the grey's ink: {k} → {tint}");
        assert_eq!(c, INK.tinted(tint), "its colour is the swatch's at that tint");
    }
    // A colour reads as its grey equivalent: white is no ink.
    let (_, _, tint, _) = linked(&m, Color::rgb(1.0, 1.0, 1.0), None);
    assert!(close(tint, 0.0));
    // A swatch the document hasn't: an unlinked tint of the tone's colour.
    let (c, link, ..) = linked(&map(json!({"tones": [{"color": INK, "swatch": "Gone"}]})), Color::gray(0.5), None);
    assert!(link.is_none() && looks(c, INK.tinted(0.5).to_rgb()));
    // Without the links, colours (mesh points) take the tone's colour too.
    assert!(looks(m.apply(Color::gray(0.5)), INK.tinted(0.5).to_rgb()));
}

#[test]
fn the_curve_changes_the_mapping() {
    // Inverse: dark greys get little ink.
    let (_, _, tint, _) = linked(&map(ink_tone("0,100 100,0")), Color::gray(0.8), None);
    assert!(close(tint, 0.2), "{tint}");
    // Through its points exactly, flat past its ends.
    let lifted = map(ink_tone("0,0 50,80 100,100"));
    assert!(close(linked(&lifted, Color::gray(0.5), None).2, 0.8));
    assert!(close(linked(&map(ink_tone("20,0 80,100")), Color::gray(0.1), None).2, 0.0));
    // Unreadable or missing curves are linear; points past the cap are left out.
    for curve in [json!("junk"), json!(null), json!([[0, 0]])] {
        let p = json!({"tones": [{"color": INK, "swatch": "Ink", "curve": curve}]});
        assert!(close(linked(&map(p), Color::gray(0.3), None).2, 0.3), "{curve}");
    }
    let many: Vec<[f64; 2]> = (0..200).map(|i| [f64::from(i) / 2.0, 100.0 - f64::from(i) / 2.0]).collect();
    assert_eq!(tone_curve_points(Some(&json!(many))).len(), MAX_CURVE_POINTS);
}

#[test]
fn several_tones_multiply_as_cmyk_inks() {
    let cyan = json!({"color": {"model": "cmyk", "c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0}, "swatch": "Ink"});
    let half_black = json!({"color": {"model": "cmyk", "c": 0.6, "m": 0.0, "y": 0.0, "k": 1.0}, "curve": "0,0 100,50"});
    let m = map(json!({"tones": [cyan, half_black]}));
    let (c, link, tint, changed) = linked(&m, Color::gray(1.0), Some("Ink"));
    assert!(changed && link.is_none() && tint == 1.0, "a process mix, unlinked");
    // Cyan 100% over the second ink at 50% (60% cyan, 100% black at half): cyan 1-(1-1)(1-0.3).
    let Color::Cmyk { c, m: mg, y, k } = c else { panic!("CMYK: {c:?}") };
    assert!(close(c, 1.0) && close(mg, 0.0) && close(y, 0.0) && close(k, 0.5), "{c} {mg} {y} {k}");
    let (mid, ..) = linked(&m, Color::gray(0.5), None);
    let Color::Cmyk { c, k, .. } = mid else { panic!("CMYK") };
    // Cyan 50% over the second ink at 25% (15% cyan, 25% black): 1 - 0.5 * 0.85 = 0.575.
    assert!(close(c, 0.575) && close(k, 0.25), "{c} {k}");
    // Inactive tones and tones past the fourth don't count; none active leaves the colours.
    let one = json!({"color": "#000000"});
    let off = json!({"color": "#ff0000", "active": false});
    let only_black = map(json!({"tones": [one, off, off, off, {"color": "#ff0000"}]}));
    assert!(looks(only_black.apply(Color::gray(0.4)), [0.6; 3]));
    assert!(color_map(&[Effect { id: DUOTONE.into(), params: json!({"tones": [off]}), visible: true }]).is_none());
    assert!(color_map(&[Effect { id: DUOTONE.into(), params: json!({"tones": "x"}), visible: true }]).is_none());
}

#[test]
fn skip_spot_keeps_spot_colours() {
    let mut p = json!({"tones": [{"color": "#000000"}], "skipSpot": true});
    let m = map(p.clone());
    let spot = Color::cmyk(0.0, 1.0, 0.0, 0.0);
    assert_eq!(linked(&m, spot, Some("Ink")), (spot, Some("Ink".to_string()), 1.0, false));
    // A process swatch's colour, and an unlinked one, still change.
    assert!(linked(&m, spot, Some("Blue")).3);
    assert!(linked(&m, spot, None).3);
    p["skipSpot"] = json!(false);
    assert!(linked(&map(p), spot, Some("Ink")).3);
}

#[test]
fn pixels_and_other_adjustments_compose() {
    let m = map(json!({"tones": [{"color": "#ff0000"}]}));
    // A grey pixel half way: half way to red from white.
    let px = m.apply_rgb([0.5, 0.5, 0.5]);
    assert!(close(px[0], 1.0) && close(px[1], 0.5) && close(px[2], 0.5), "{px:?}");
    // Invert, then Duotone: white becomes full red.
    let both = color_map(&[
        Effect { id: crate::addons::color::INVERT.into(), params: json!({}), visible: true },
        Effect { id: DUOTONE.into(), params: json!({"tones": [{"color": "#ff0000"}]}), visible: true },
    ])
    .unwrap();
    assert!(looks(both.apply(Color::WHITE), [1.0, 0.0, 0.0]));
    // Out of range colours are brought into range.
    let wild = map(json!({"tones": [{"color": [1e30, -5, 0.5]}]}));
    assert!(looks(wild.apply(Color::gray(1.0)), [1.0, 0.0, 0.5]));
}
