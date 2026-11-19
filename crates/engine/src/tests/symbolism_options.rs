//! The Symbolism Tools Options the commands take: Method and pen pressure in `symbol.adjust`, the
//! Symbol Sizer's two options, the Symbol Sprayer's `attributes`, and `symbol.setDensity`.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{Affine, Point, Rect};

use crate::*;

/// A document with the 20 pt square symbol "Dot" (its first instance is at 10, 10) and nothing
/// selected.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("symbol.new", &json!({"name": "Dot"})).unwrap();
    s.select(|_, sel| sel.clear()).unwrap();
    s
}

/// An instance of Dot centered on (`x`, `y`), not selected.
fn dot(s: &mut Session, x: f64, y: f64) -> NodeId {
    let id = NodeId(s.execute("symbol.place", &json!({"name": "Dot", "x": x, "y": y})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.clear()).unwrap();
    id
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    node(s, id).geometric_bounds().unwrap()
}

fn xf(s: &Session, id: NodeId) -> Affine {
    match node(s, id).kind {
        NodeKind::SymbolInstance { xf, .. } => xf,
        other => panic!("not an instance: {other:?}"),
    }
}

fn center(s: &Session, id: NodeId) -> Point {
    xf(s, id) * Point::ZERO
}

/// The instance's rotation in radians.
fn angle(s: &Session, id: NodeId) -> f64 {
    let c = xf(s, id).as_coeffs();
    c[1].atan2(c[0])
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn state(s: &Session) -> (Document, usize) {
    ((*s.doc().unwrap().doc).clone(), undo_len(s))
}

fn adjust(s: &mut Session, p: Value) -> Value {
    s.execute("symbol.adjust", &p).unwrap()
}

/// `cmd` with `p` fails with the bad-parameter message `want` and leaves the document and the
/// undo history as they were.
fn refused(s: &mut Session, cmd: &str, p: Value, want: &str) {
    let before = state(s);
    let r = s.execute(cmd, &p);
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == want), "{cmd} {p}: {r:?}");
    assert!(state(s) == before, "{cmd} {p} changed the document");
}

/// The children of symbol set `set`.
fn members(s: &Session, set: NodeId) -> Vec<NodeId> {
    node(s, set).children().unwrap().iter().map(|c| c.id).collect()
}

/// Method Average brings the sizes, rotations and opacities of the instances under the brush
/// toward their mean, whichever way Alt is; User Defined changes each one by the same amount.
#[test]
fn average_brings_the_instances_under_the_brush_toward_their_mean() {
    // Two instances 20 pt apart, the first grown, turned and faded alone.
    let pair = || {
        let mut s = session();
        let (a, b) = (dot(&mut s, 200.0, 150.0), dot(&mut s, 220.0, 150.0));
        let only_a = json!([[200, 150], [200, 150], [200, 150], [200, 150]]);
        for tool in ["size", "spin", "screen"] {
            adjust(&mut s, json!({"tool": tool, "points": only_a, "radius": 5, "intensity": 10}));
        }
        (s, a, b)
    };
    let (mut s, a, b) = pair();
    let (wa, wb) = (bounds(&s, a).width(), bounds(&s, b).width());
    let (ra, oa) = (angle(&s, a), node(&s, a).opacity);
    assert!(wa > wb + 1.0 && ra.abs() > 0.1 && oa < 0.9, "{wa} {wb} {ra} {oa}");
    assert_eq!((angle(&s, b), node(&s, b).opacity), (0.0, 1.0));
    // User Defined grows both by the same factor: they stay as far apart in size.
    let between = json!([[210, 150], [210, 150], [210, 150]]);
    let (mut user, ..) = pair();
    adjust(&mut user, json!({"tool": "size", "points": between, "radius": 40, "intensity": 10}));
    let ratio = |s: &Session| xf(s, a).determinant() / xf(s, b).determinant();
    assert!((ratio(&user) - ratio(&s)).abs() < 1e-9);
    // Average: the two sizes meet at their geometric mean.
    let before = ratio(&s);
    let product = xf(&s, a).determinant() * xf(&s, b).determinant();
    let (mut alt, ..) = pair();
    adjust(&mut s, json!({"tool": "size", "points": between, "radius": 40, "intensity": 10, "method": "average"}));
    assert!((ratio(&s) - 1.0).abs() < (before - 1.0).abs() / 2.0, "{} from {before}", ratio(&s));
    assert!((xf(&s, a).determinant() * xf(&s, b).determinant() / product - 1.0).abs() < 1e-9);
    adjust(&mut alt, json!({"tool": "size", "points": between, "radius": 40, "intensity": 10, "method": "average", "alt": true}));
    assert_eq!([xf(&alt, a), xf(&alt, b)], [xf(&s, a), xf(&s, b)], "Alt does not reverse an average");
    // The rotations and the opacities meet too.
    adjust(&mut s, json!({"tool": "spin", "points": between, "radius": 40, "intensity": 10, "method": "average"}));
    let (na, nb) = (angle(&s, a), angle(&s, b));
    assert!((na - nb).abs() < ra.abs() / 2.0 && nb.abs() > 1e-3, "{na} {nb} from {ra}");
    adjust(&mut s, json!({"tool": "screen", "points": between, "radius": 40, "intensity": 10, "method": "average"}));
    let (pa, pb) = (node(&s, a).opacity, node(&s, b).opacity);
    assert!(pa > oa && pb < 1.0 && (pa - pb).abs() < (1.0 - oa) / 2.0, "{pa} {pb} from {oa}");
}

/// `tool.setOption` refuses a Method, a Symbol Sprayer option or a checkbox value the Symbolism Tools
/// Options do not have, and sets none of the values given with it.
#[test]
fn bad_symbolism_tool_options_are_refused() {
    let mut s = session();
    let before = s.tool_options_of("symbolSizer");
    for (values, want) in [
        (json!({"proportional": false, "method": "smooth"}), "`method` must be one of userDefined, average, random"),
        (json!({"sprayScrunch": "random"}), "`sprayScrunch` must be one of average, userDefined"),
        (json!({"method": "average", "usePressure": 1}), "`usePressure` must be true or false"),
        (json!({"affectsDensity": "yes"}), "`affectsDensity` must be true or false"),
    ] {
        let r = s.set_tool_option_cmd(&json!({"tool": "symbolSizer", "values": values}));
        assert_eq!(r, Err(want.to_string()), "{values}");
        assert_eq!(s.tool_options_of("symbolSizer"), before, "{values}");
    }
    s.set_tool_option_cmd(&json!({"tool": "symbolSizer", "key": "method", "value": "random"})).unwrap();
    assert_eq!(s.tool_options_of("symbolSizer")["method"], json!("random"));
}

/// Average opacities below the Symbol Screener's least opacity (5%) stay below it: with the Screener
/// the instances meet at their mean, and a sprayed instance takes the mean of those under the brush.
#[test]
fn average_keeps_opacities_below_the_screeners_least() {
    let mut s = session();
    let set = NodeId(s.execute("symbol.spray", &json!({"name": "Dot", "points": [[200, 150]], "radius": 4})).unwrap()["id"].as_u64().unwrap());
    let a = members(&s, set)[0];
    let b = dot(&mut s, 220.0, 150.0);
    for (id, opacity) in [(a, 0), (b, 2)] {
        s.select(|_, sel| sel.set([id])).unwrap();
        s.execute("transparency.set", &json!({"opacity": opacity})).unwrap();
    }
    s.select(|_, sel| sel.set([set])).unwrap();
    // A new instance beside the invisible one takes its opacity.
    s.execute("symbol.spray", &json!({"name": "Dot", "points": [[185, 150]], "radius": 20})).unwrap();
    let sprayed = *members(&s, set).last().unwrap();
    assert_eq!(node(&s, sprayed).opacity, 0.0);
    // The Screener with Average brings 0% and 2% toward 1%, not up to 5%.
    s.select(|_, sel| sel.clear()).unwrap();
    adjust(&mut s, json!({"tool": "screen", "points": [[210, 150]], "radius": 40, "intensity": 10, "method": "average"}));
    let (pa, pb) = (node(&s, a).opacity, node(&s, b).opacity);
    assert!(pa > 0.0 && pa < 0.02 && pb < 0.02 && pb > 0.0, "{pa} {pb}");
}

/// Average over instances that already have the mean, or a stroke over no instance, changes
/// nothing and adds no undo step.
#[test]
fn average_at_the_mean_adds_no_undo_step() {
    let mut s = session();
    dot(&mut s, 200.0, 150.0);
    let before = state(&s);
    for tool in ["scrunch", "size", "spin", "stain", "screen", "style"] {
        let r = adjust(&mut s, json!({"tool": tool, "points": [[200, 150], [202, 150]], "radius": 30, "intensity": 10, "method": "average"}));
        assert_eq!(r["count"], 0, "{tool}");
        assert!(state(&s) == before, "{tool} changed the document");
    }
    let r = adjust(&mut s, json!({"tool": "size", "points": [[50, 50]], "radius": 10}));
    assert_eq!(r["count"], 0);
    assert!(state(&s) == before);
}

/// Average evens out the Symbol Stainer's amount and the Symbol Scruncher's spacing.
#[test]
fn average_evens_out_the_stain_and_the_spacing() {
    let mut s = session();
    let (a, b, c) = (dot(&mut s, 180.0, 150.0), dot(&mut s, 200.0, 150.0), dot(&mut s, 260.0, 150.0));
    adjust(&mut s, json!({"tool": "stain", "points": [[180, 150], [180, 150]], "radius": 5, "intensity": 10, "color": "#ff0000"}));
    let amount = |s: &Session, id| node(s, id).appearance.fill().map_or(0.0, |f| f.opacity);
    let stained = amount(&s, a);
    assert!(stained > 0.2 && amount(&s, b) == 0.0);
    adjust(
        &mut s,
        json!({"tool": "stain", "points": [[190, 150], [190, 150]], "radius": 30, "intensity": 10, "method": "average", "color": "#0000ff"}),
    );
    assert!(amount(&s, a) < stained && amount(&s, b) > 0.0, "{} {}", amount(&s, a), amount(&s, b));
    // The stained instance keeps its red, and the other takes the red of the stained one.
    for id in [a, b] {
        assert_eq!(node(&s, id).appearance.fill().and_then(|f| f.paint.color()).map(|c| c.to_hex()).as_deref(), Some("#ff0000"));
    }
    // Scrunch: the three are 20 and 60 apart; their distances from their middle even out.
    let reach = |s: &Session| {
        let xs = [a, b, c].map(|id| center(s, id).x);
        let middle = xs.iter().sum::<f64>() / 3.0;
        xs.map(|x| (x - middle).abs())
    };
    let spread = |r: [f64; 3]| r.iter().cloned().fold(0.0, f64::max) - r.iter().cloned().fold(f64::INFINITY, f64::min);
    let before = spread(reach(&s));
    adjust(&mut s, json!({"tool": "scrunch", "points": [[213, 150], [213, 150]], "radius": 120, "intensity": 10, "method": "average"}));
    assert!(spread(reach(&s)) < before * 0.8, "{} from {before}", spread(reach(&s)));
}

/// Method Random gives each instance its own share of the change, the same for a stroke however
/// often it is applied; User Defined changes instances at the same distance alike.
#[test]
fn random_changes_each_instance_by_its_own_share() {
    let grown = |method: Option<&str>| {
        let mut s = session();
        let ids = [(180.0, 150.0), (220.0, 150.0), (200.0, 130.0), (200.0, 170.0)].map(|(x, y)| dot(&mut s, x, y));
        let mut p = json!({"tool": "size", "points": [[200, 150], [200, 150]], "radius": 60, "intensity": 10});
        if let Some(m) = method {
            p["method"] = json!(m);
        }
        adjust(&mut s, p);
        ids.map(|id| bounds(&s, id).width())
    };
    let user = grown(None);
    assert!(user.iter().all(|w| (w - user[0]).abs() < 1e-9 && *w > 20.5), "{user:?}");
    assert_eq!(grown(Some("userDefined")), user);
    let random = grown(Some("random"));
    assert!(random.iter().any(|w| (w - random[0]).abs() > 0.05), "{random:?}");
    // From none of the User Defined change to twice it.
    let twice = user[0] * user[0] / 20.0;
    assert!(random.iter().all(|w| *w >= 20.0 - 1e-9 && *w <= twice + 1e-9), "{random:?} within 20..{twice}");
    assert_eq!(grown(Some("random")), random);
}

/// With Method Random the Symbol Styler gives each instance its share of the style scaled by the
/// brush strength: at Intensity 1 a tenth of what it gives at Intensity 10.
#[test]
fn random_style_follows_the_brush_strength() {
    let styled = |intensity: u32| {
        let mut s = session();
        let ids = [(200.0, 150.0), (201.0, 150.0), (200.0, 151.0), (199.0, 150.0), (200.0, 149.0)].map(|(x, y)| dot(&mut s, x, y));
        let p = json!({"tool": "style", "points": [[200, 150]], "radius": 1000, "intensity": intensity, "method": "random", "color": "#ff0000"});
        adjust(&mut s, p);
        ids.map(|id| f64::from(node(&s, id).appearance.fill().map_or(0.0, |f| f.opacity)))
    };
    let (full, low) = (styled(10), styled(1));
    assert!(full.iter().any(|o| *o > 0.05), "{full:?}");
    for (f, l) in full.iter().zip(&low) {
        assert!((l - f * 0.1).abs() < 1e-3, "{low:?} against {full:?}");
    }
}

/// A `method` that is none of the three is an error and changes nothing; the Symbol Shifter takes
/// one and shifts as it does without.
#[test]
fn an_unknown_method_is_refused() {
    let mut s = session();
    let a = dot(&mut s, 200.0, 150.0);
    for (bad, shown) in [(json!("smooth"), "\"smooth\""), (json!(3), "3"), (json!(true), "true"), (json!("Average"), "\"Average\"")] {
        for tool in ["size", "shift"] {
            let want = format!("`method` must be userDefined, average or random, not {shown}");
            refused(&mut s, "symbol.adjust", json!({"tool": tool, "points": [[200, 150]], "method": bad}), &want);
        }
    }
    let shifted = |method: Value| {
        let mut s = session();
        let a = dot(&mut s, 200.0, 150.0);
        adjust(&mut s, json!({"tool": "shift", "points": [[200, 150], [215, 150]], "intensity": 10, "method": method}));
        center(&s, a)
    };
    let moved = shifted(Value::Null);
    assert!(moved.x > 205.0);
    for method in ["average", "random", "userDefined"] {
        assert_eq!(shifted(json!(method)), moved, "{method}");
    }
    assert_eq!(center(&s, a), Point::new(200.0, 150.0));
}

/// With `usePressure` each point's pen pressure is the intensity there; without it the pressure is
/// not used. A pressure that isn't a number is an error.
#[test]
fn pen_pressure_is_the_intensity_with_use_pressure() {
    let width = |points: Value, use_pressure: Option<bool>, intensity: f64| {
        let mut s = session();
        let a = dot(&mut s, 200.0, 150.0);
        let mut p = json!({"tool": "size", "points": points, "radius": 40, "intensity": intensity});
        if let Some(on) = use_pressure {
            p["usePressure"] = json!(on);
        }
        let count = adjust(&mut s, p)["count"].as_u64().unwrap();
        (bounds(&s, a).width(), count)
    };
    let (light, _) = width(json!([[200, 150, 0.25]]), Some(true), 5.0);
    let (hard, _) = width(json!([[200, 150, 1.0]]), Some(true), 5.0);
    assert!(20.0 < light && light < hard, "{light} {hard}");
    // Full pressure is intensity 10, whatever `intensity` says; a point without a pressure has 1.
    assert_eq!(width(json!([[200, 150]]), None, 10.0).0, hard);
    assert_eq!(width(json!([[200, 150]]), Some(true), 1.0).0, hard);
    assert_eq!(width(json!([[200, 150, null]]), Some(true), 1.0).0, hard);
    // No pressure: the instance stays as it is.
    assert_eq!(width(json!([[200, 150, 0]]), Some(true), 5.0), (20.0, 0));
    // A pressure outside 0..1 is kept within it.
    assert_eq!(width(json!([[200, 150, 7]]), Some(true), 5.0).0, hard);
    // Without Use Pressure Pen the pressure is not used.
    assert_eq!(width(json!([[200, 150, 0.25]]), None, 10.0).0, hard);
    assert_eq!(width(json!([[200, 150, 0.25]]), Some(false), 10.0).0, hard);
    let mut s = session();
    dot(&mut s, 200.0, 150.0);
    for tool in ["size", "shift"] {
        refused(
            &mut s,
            "symbol.adjust",
            json!({"tool": tool, "points": [[200, 150, "hard"]], "usePressure": true}),
            "a point's pressure must be a number, not \"hard\"",
        );
    }
    for key in ["usePressure", "proportional", "affectsDensity"] {
        let mut p = json!({"tool": "size", "points": [[200, 150]]});
        p[key] = json!("yes");
        refused(&mut s, "symbol.adjust", p, &format!("`{key}` must be true or false"));
    }
}

/// The Symbol Sizer without Proportional Resizing resizes along the drag: a level drag the width,
/// an upright one the height, a click both.
#[test]
fn the_sizer_without_proportional_resizing_resizes_along_the_drag() {
    let size = |points: Value, proportional: Option<bool>| {
        let mut s = session();
        let a = dot(&mut s, 200.0, 150.0);
        let mut p = json!({"tool": "size", "points": points, "radius": 60, "intensity": 10});
        if let Some(on) = proportional {
            p["proportional"] = json!(on);
        }
        adjust(&mut s, p);
        (bounds(&s, a).width(), bounds(&s, a).height())
    };
    let level = json!([[195, 150], [205, 150]]);
    let (w, h) = size(level.clone(), Some(false));
    assert!(w > 21.0 && (h - 20.0).abs() < 1e-9, "{w} x {h}");
    let (w, h) = size(json!([[200, 145], [200, 155]]), Some(false));
    assert!(h > 21.0 && (w - 20.0).abs() < 1e-9, "{w} x {h}");
    let (w, h) = size(json!([[200, 150]]), Some(false));
    assert!(w > 20.5 && (w - h).abs() < 1e-9, "{w} x {h}");
    // Proportional Resizing (the default) keeps the shape for any drag, at the same area.
    let (pw, ph) = size(level.clone(), None);
    assert!(pw > 20.5 && (pw - ph).abs() < 1e-9, "{pw} x {ph}");
    assert_eq!(size(level.clone(), Some(true)), (pw, ph));
    let (w, h) = size(level, Some(false));
    assert!((w * h - pw * ph).abs() < 1e-6, "{w} x {h} against {pw} x {ph}");
}

/// With Resizing Affects Density an instance moves away from the brush's center as it grows and
/// toward it as it shrinks; without, it stays where it is.
#[test]
fn resizing_affects_density_moves_instances_from_the_brush_center() {
    let at = |affects: Option<bool>, alt: bool| {
        let mut s = session();
        let a = dot(&mut s, 220.0, 150.0);
        let mut p = json!({"tool": "size", "points": [[200, 150], [200, 150]], "radius": 60, "intensity": 10, "alt": alt});
        if let Some(on) = affects {
            p["affectsDensity"] = json!(on);
        }
        adjust(&mut s, p);
        (center(&s, a), bounds(&s, a).width())
    };
    let (stays, grown) = at(None, false);
    assert!((stays.x - 220.0).abs() < 1e-9 && grown > 20.5);
    assert_eq!(at(Some(false), false), (stays, grown));
    let (away, width) = at(Some(true), false);
    assert!(away.x > 220.5 && (away.y - 150.0).abs() < 1e-9 && width > 20.5, "{away:?}");
    let (toward, width) = at(Some(true), true);
    assert!(toward.x < 219.5 && width < 19.5, "{toward:?}");
}

/// A new instance of the Symbol Sprayer takes the mean of the selected set's instances under the
/// brush for each attribute that is `average`, and the plain value for each that is `userDefined`.
#[test]
fn the_sprayer_gives_new_instances_the_average_or_the_user_defined_values() {
    let mut s = session();
    let set = NodeId(s.execute("symbol.spray", &json!({"name": "Dot", "points": [[200, 150]], "radius": 4})).unwrap()["id"].as_u64().unwrap());
    let first = members(&s, set)[0];
    // In an area without instances an average is the plain value.
    assert!(node(&s, first).opacity == 1.0 && node(&s, first).appearance.fill().is_none() && angle(&s, first) == 0.0);
    // Grow, fade and stain the set's one instance.
    let on_it = json!([[200, 150], [200, 150], [200, 150], [200, 150], [200, 150], [200, 150]]);
    for tool in ["size", "screen", "stain"] {
        adjust(&mut s, json!({"tool": tool, "points": on_it, "radius": 40, "intensity": 10, "color": "#ff0000"}));
    }
    let look = |s: &Session, id: NodeId| {
        let n = node(s, id);
        (bounds(s, id).width(), n.opacity, n.appearance.fill().map(|f| (f.paint.color().map(|c| c.to_hex()), f.opacity)))
    };
    let (width, opacity, stain) = look(&s, first);
    assert!(width > 24.0 && opacity < 0.7 && stain.as_ref().is_some_and(|(_, amount)| *amount > 0.5), "{width} {opacity} {stain:?}");
    let spray = |s: &mut Session, at: Value, extra: Value| {
        let mut p = json!({"name": "Dot", "points": at, "radius": 30});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        s.execute("symbol.spray", &p).unwrap();
        *members(s, set).last().unwrap()
    };
    // Average (the default): the new instance, sprayed beside it, looks like it, at its size.
    let near = spray(&mut s, json!([[215, 150]]), json!({}));
    let (w, o, f) = look(&s, near);
    assert!((w / width - 1.0).abs() < 1e-9, "{w} of {width}");
    assert_eq!((o, f), (opacity, stain.clone()));
    // The averages of two instances that look alike are theirs.
    let third = spray(&mut s, json!([[208, 165]]), json!({"attributes": {"size": "average", "screen": null}}));
    assert_eq!(look(&s, third).1, opacity);
    // Style: Average replaces a User Defined stain with the fill of the instances under the brush.
    let mixed = spray(&mut s, json!([[205, 160]]), json!({"attributes": {"stain": "userDefined"}, "color": "#0000ff"}));
    assert_eq!(look(&s, mixed).2.map(|(c, _)| c), stain.clone().map(|(c, _)| c));
    // Outside the brush's radius of every instance, the plain values: the symbol's own size.
    let far = spray(&mut s, json!([[330, 250]]), json!({}));
    let (w, o, f) = look(&s, far);
    assert!((w / 20.0 - 1.0).abs() < 1e-9 && o == 1.0 && f.is_none(), "{w} {o} {f:?}");
    // User Defined: the symbol's size and opaque; Style: Average still gives the fill under the brush.
    let user = json!({"size": "userDefined", "screen": "userDefined", "stain": "userDefined", "spin": "userDefined"});
    let plain = spray(&mut s, json!([[212, 140]]), json!({"attributes": user, "color": "#0000ff"}));
    let (w, o, f) = look(&s, plain);
    assert!((w / 20.0 - 1.0).abs() < 0.11 && o == 1.0, "{w} {o}");
    assert_eq!(f.map(|(c, _)| c), stain.clone().map(|(c, _)| c));
    assert_eq!(angle(&s, plain), 0.0, "one point has no direction");
    // With no fill under the brush, Stain: User Defined is the fill color in full, and a User Defined
    // style without a fill (the document has only the default style) leaves it.
    let blue = spray(&mut s, json!([[330, 40]]), json!({"attributes": user, "color": "#0000ff"}));
    assert_eq!(look(&s, blue).2, Some((Some("#0000ff".to_string()), 1.0)));
    let styled = json!({"stain": "userDefined", "style": "userDefined"});
    let blue = spray(&mut s, json!([[330, 100]]), json!({"attributes": styled, "color": "#0000ff"}));
    assert_eq!(look(&s, blue).2, Some((Some("#0000ff".to_string()), 1.0)));
    // Spin: User Defined turns the instances the way the stroke runs (here: down the page).
    let down = spray(&mut s, json!([[100, 40], [100, 140]]), json!({"attributes": {"spin": "userDefined"}}));
    assert!((angle(&s, down) - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{}", angle(&s, down));
    let level = spray(&mut s, json!([[40, 260], [140, 260]]), json!({}));
    assert_eq!(angle(&s, level), 0.0, "Average, with no instance under the brush");
}

/// Size: Average is a size on the page, the same measure for every symbol: a 20 pt symbol sprayed
/// beside an 80 pt one is 80 pt, and the Sizer brings the two to 40 pt. With no instance under the
/// brush a sprayed instance has its symbol's own size, whatever `scale` is.
#[test]
fn average_sizes_are_sizes_on_the_page_for_every_symbol() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 200, "width": 80, "height": 80})).unwrap();
    s.execute("symbol.new", &json!({"name": "Big"})).unwrap();
    s.select(|_, sel| sel.clear()).unwrap();
    let set = NodeId(s.execute("symbol.spray", &json!({"name": "Big", "points": [[200, 150]], "radius": 4})).unwrap()["id"].as_u64().unwrap());
    let big = members(&s, set)[0];
    assert!((bounds(&s, big).width() - 80.0).abs() < 1e-9);
    let spray = |s: &mut Session, at: Value| {
        s.execute("symbol.spray", &json!({"name": "Dot", "points": at, "radius": 30, "scale": 2})).unwrap();
        *members(s, set).last().unwrap()
    };
    let near = spray(&mut s, json!([[215, 150]]));
    assert!((bounds(&s, near).width() - 80.0).abs() < 1e-9, "{}", bounds(&s, near).width());
    let far = spray(&mut s, json!([[350, 40]]));
    assert!((bounds(&s, far).width() - 20.0).abs() < 1e-9, "{}", bounds(&s, far).width());
    // The Sizer: an unscaled Dot and an unscaled Big meet at the geometric mean of 20 and 80.
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 200, "width": 80, "height": 80})).unwrap();
    s.execute("symbol.new", &json!({"name": "Big"})).unwrap();
    s.select(|_, sel| sel.clear()).unwrap();
    let small = dot(&mut s, 200.0, 150.0);
    let big = NodeId(s.execute("symbol.place", &json!({"name": "Big", "x": 220, "y": 150})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.clear()).unwrap();
    let between = json!([[210, 150], [210, 150], [210, 150], [210, 150], [210, 150], [210, 150]]);
    adjust(&mut s, json!({"tool": "size", "points": between, "radius": 30, "intensity": 10, "method": "average"}));
    let (ws, wb) = (bounds(&s, small).width(), bounds(&s, big).width());
    assert!(ws > 30.0 && wb < 55.0 && (ws * wb / 1600.0 - 1.0).abs() < 1e-9, "{ws} {wb}");
    // A symbol without a recorded size is measured by its art: here 80 pt art drawn at its own size.
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 200, "width": 20, "height": 20})).unwrap();
    s.execute("symbol.new", &json!({"name": "Loose"})).unwrap();
    s.edit("Forget size", |d, _| {
        if let Some(m) = d.unknown.get_mut(vectorcraft_doc::SYMBOL_SIZES).and_then(Value::as_object_mut) {
            m.remove("Loose");
        }
        for sym in d.symbols.iter_mut().filter(|sym| sym.name == "Loose") {
            std::sync::Arc::make_mut(&mut sym.art).transform(Affine::scale(4.0), false);
        }
        Ok(())
    })
    .unwrap();
    s.select(|_, sel| sel.clear()).unwrap();
    let small = dot(&mut s, 200.0, 150.0);
    let loose = NodeId(s.execute("symbol.place", &json!({"name": "Loose", "x": 220, "y": 150})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.clear()).unwrap();
    // Its width as drawn, measured by its symbol's art.
    let drawn = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        node(s, loose).geometric_bounds_with(&d.symbol_boxes()).unwrap().width()
    };
    assert!((drawn(&s) - 80.0).abs() < 1e-9, "{}", drawn(&s));
    adjust(&mut s, json!({"tool": "size", "points": between, "radius": 30, "intensity": 10, "method": "average"}));
    let (ws, wl) = (bounds(&s, small).width(), drawn(&s));
    assert!(ws > 30.0 && wl < 55.0, "{ws} {wl}");
    // A new set sprayed with Size: Average is spaced by the size it is placed at, not by `scale`.
    s.select(|_, sel| sel.clear()).unwrap();
    let stroke: Vec<Value> = (0..50).map(|i| json!([100.0 + 4.0 * f64::from(i), 20.0])).collect();
    let r = s.execute("symbol.spray", &json!({"name": "Dot", "points": stroke, "radius": 2, "scale": 100})).unwrap();
    assert!(r["count"].as_u64().unwrap() > 5, "{r}");
    // The 80 pt art of a symbol without a recorded size spaces new instances by its own size.
    s.select(|_, sel| sel.clear()).unwrap();
    let stroke: Vec<Value> = (0..50).map(|i| json!([100.0 + 4.0 * f64::from(i), 280.0])).collect();
    let r = s.execute("symbol.spray", &json!({"name": "Loose", "points": stroke, "radius": 2})).unwrap();
    assert!(r["count"].as_u64().unwrap() <= 5, "{r}");
}

/// Spin: Average turns a new instance to the mean direction of the instances under the brush, and
/// Scrunch: Average spaces new instances as those under the brush are spaced.
#[test]
fn the_sprayer_averages_rotation_and_spacing() {
    // A set of six instances 30 pt apart, turned by a drag of the Symbol Spinner; then a stroke
    // along the row, 4 pt between its points, with Scrunch as given. Returns the rotations of the
    // six, how many instances the stroke added and the rotation of the last one.
    let sprayed = |scrunch: &str| {
        let mut s = session();
        let row: Vec<Value> = (0..6).map(|i| json!([100.0 + 30.0 * f64::from(i), 100.0])).collect();
        let p = json!({"name": "Dot", "points": row, "radius": 2, "density": 10});
        let set = NodeId(s.execute("symbol.spray", &p).unwrap()["id"].as_u64().unwrap());
        assert_eq!(members(&s, set).len(), 6);
        adjust(&mut s, json!({"tool": "spin", "points": [[175, 60], [176, 60], [177, 60], [178, 60]], "radius": 200, "intensity": 10}));
        let turned: Vec<f64> = members(&s, set).iter().map(|id| angle(&s, *id)).collect();
        let stroke: Vec<Value> = (0..40).map(|i| json!([100.0 + 4.0 * f64::from(i), 100.0])).collect();
        let p = json!({"name": "Dot", "points": stroke, "radius": 40, "density": 10, "attributes": {"scrunch": scrunch}});
        let n = s.execute("symbol.spray", &p).unwrap()["count"].as_u64().unwrap();
        let new = *members(&s, set).last().unwrap();
        (turned, n, angle(&s, new))
    };
    let (turned, plain, _) = sprayed("userDefined");
    assert!(turned.iter().all(|a| a.abs() > 0.05 && a.signum() == turned[0].signum()), "{turned:?}");
    // User Defined keeps new instances 3 pt apart (20 pt × 0.15 at density 10); Average keeps them
    // as far apart as the instances under the brush are, about 30 pt.
    let (_, spaced, new_angle) = sprayed("average");
    assert!(spaced < plain, "{spaced} spaced like the row against {plain}");
    assert!(new_angle.signum() == turned[0].signum() && new_angle.abs() > 0.05, "{new_angle} against {turned:?}");
}

/// The Symbol Sprayer's `attributes` must be an object of the six keys, each `average` or
/// `userDefined`; anything else is an error and sprays nothing.
#[test]
fn bad_sprayer_attributes_are_refused() {
    let mut s = session();
    let calls = [
        (json!("average"), "`attributes` must be an object, not \"average\"".to_string()),
        (json!(["size"]), "`attributes` must be an object, not [\"size\"]".to_string()),
        (json!({"density": "average"}), "unknown attribute `density`".to_string()),
        (json!({"size": "big"}), "attribute `size` must be average or userDefined, not \"big\"".to_string()),
        (json!({"spin": true}), "attribute `spin` must be average or userDefined, not true".to_string()),
        (json!({"style": "Average"}), "attribute `style` must be average or userDefined, not \"Average\"".to_string()),
    ];
    for (attributes, want) in calls {
        refused(&mut s, "symbol.spray", json!({"name": "Dot", "points": [[200, 150]], "attributes": attributes}), &want);
    }
    refused(&mut s, "symbol.spray", json!({"name": "Dot", "points": [[200, 150]], "keepModel": "yes"}), "`keepModel` must be true or false");
    refused(&mut s, "symbol.spray", json!({"name": "Dot", "points": [[200, 150, "x"]]}), "a point's pressure must be a number, not \"x\"");
    // Style: User Defined with a style the document does not have; Average does not use it.
    let user = json!({"name": "Dot", "points": [[200, 150]], "attributes": {"style": "userDefined"}, "style": "Nope"});
    refused(&mut s, "symbol.spray", user, "unknown graphic style `Nope`");
    let number = json!({"name": "Dot", "points": [[200, 150]], "attributes": {"style": "userDefined"}, "style": 3});
    refused(&mut s, "symbol.spray", number, "`style` must be a graphic style's name, not 3");
    let average = json!({"name": "Dot", "points": [[200, 150]], "style": "Nope"});
    assert_eq!(s.execute("symbol.spray", &average).unwrap()["count"], 1);
    let all = json!({"scrunch": "average", "size": "userDefined", "spin": null, "screen": "average", "stain": "average", "style": "average"});
    assert_eq!(s.execute("symbol.spray", &json!({"name": "Dot", "points": [[200, 150]], "attributes": all})).unwrap()["count"], 1);
}

/// `symbol.setDensity` spaces a symbol set's instances from its middle as the new density packs
/// them, as one undo step, and adds none when no instance moves.
#[test]
fn set_density_spaces_the_instances_of_a_symbol_set() {
    let mut s = session();
    let row: Vec<Value> = (0..8).map(|i| json!([40.0 + 40.0 * f64::from(i), 150.0])).collect();
    let set = NodeId(s.execute("symbol.spray", &json!({"name": "Dot", "points": row, "radius": 4})).unwrap()["id"].as_u64().unwrap());
    let ids = members(&s, set);
    assert_eq!(ids.len(), 8);
    let centers = |s: &Session| ids.iter().map(|id| center(s, *id)).collect::<Vec<_>>();
    let middle = |c: &[Point]| Point::new(c.iter().map(|p| p.x).sum::<f64>() / 8.0, c.iter().map(|p| p.y).sum::<f64>() / 8.0);
    let (was, steps) = (centers(&s), undo_len(&s));
    // From 5 to 9: 0.25 / 0.65 of each distance from the middle, which stays where it is.
    let r = s.execute("symbol.setDensity", &json!({"from": 5, "to": 9})).unwrap();
    assert_eq!(r, json!({"ids": [set.0], "count": 8}));
    assert_eq!(undo_len(&s), steps + 1);
    let now = centers(&s);
    let (m, ratio) = (middle(&was), 0.25 / 0.65);
    assert!(middle(&now).distance(m) < 1e-9);
    for (before, after) in was.iter().zip(&now) {
        assert!(after.distance(m + (*before - m) * ratio) < 1e-9, "{before:?} -> {after:?}");
    }
    // Back from 9 to 5: where they were, once for a set named twice.
    let r = s.execute("symbol.setDensity", &json!({"from": 9, "to": 5, "ids": [set.0, set.0]})).unwrap();
    assert_eq!(r, json!({"ids": [set.0], "count": 8}));
    assert!(centers(&s).iter().zip(&was).all(|(a, b)| a.distance(*b) < 1e-9));
    // The density the set has (5, a `from` of 7 notwithstanding): no instance moves and no undo
    // step is added.
    let steps = undo_len(&s);
    assert_eq!(s.execute("symbol.setDensity", &json!({"from": 7, "to": 5})).unwrap(), json!({"ids": [], "count": 0}));
    assert_eq!(undo_len(&s), steps);
    // Undo puts a spaced set back.
    s.execute("symbol.setDensity", &json!({"from": 5, "to": 1, "id": set.0})).unwrap();
    assert!(centers(&s)[0].distance(was[0]) > 1.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(centers(&s).iter().zip(&was).all(|(a, b)| a.distance(*b) < 1e-9));
}

/// A set packs from the density it was created at, whatever `from` says and whatever density more
/// instances were sprayed into it at; a set without a recorded density packs from `from`.
#[test]
fn set_density_starts_from_the_density_a_set_was_sprayed_at() {
    let mut s = session();
    let row: Vec<Value> = (0..4).map(|i| json!([60.0 + 60.0 * f64::from(i), 150.0])).collect();
    let set = NodeId(s.execute("symbol.spray", &json!({"name": "Dot", "points": row, "radius": 4, "density": 2})).unwrap()["id"].as_u64().unwrap());
    let ids = members(&s, set);
    let xs = |s: &Session| ids.iter().map(|id| center(s, *id).x).collect::<Vec<_>>();
    let was = xs(&s);
    let middle = was.iter().sum::<f64>() / 4.0;
    s.execute("symbol.setDensity", &json!({"from": 5, "to": 9})).unwrap();
    let ratio = 0.25 / 0.95;
    for (before, after) in was.iter().zip(xs(&s)) {
        assert!((after - (middle + (before - middle) * ratio)).abs() < 1e-9, "{before} -> {after}");
    }
    // Given density 2, the set stays at 2 when more instances are sprayed into it at 9.
    s.execute("symbol.setDensity", &json!({"from": 5, "to": 2, "ids": [set.0]})).unwrap();
    s.execute("symbol.spray", &json!({"name": "Dot", "points": [[300, 200]], "radius": 4, "density": 9})).unwrap();
    assert_eq!(s.execute("symbol.setDensity", &json!({"from": 9, "to": 2, "ids": [set.0]})).unwrap()["count"], 0);
    // Without the record (a set grouped by hand), `from` is the starting density.
    let mut s = session();
    let (a, b) = (dot(&mut s, 100.0, 150.0), dot(&mut s, 200.0, 150.0));
    s.select(|_, sel| sel.set([a, b])).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.edit("Name", |d, _| {
        if let Some(n) = d.node_mut(group) {
            n.name = Some("Symbol Set".into());
        }
        Ok(())
    })
    .unwrap();
    s.execute("symbol.setDensity", &json!({"from": 5, "to": 9, "ids": [group.0]})).unwrap();
    assert!((center(&s, a).x - (150.0 - 50.0 * 0.25 / 0.65)).abs() < 1e-9, "{:?}", center(&s, a));
}

/// A set of one instance takes the new density without moving, as one undo step, so a later change
/// packs it from that density; setting the density it already has adds no undo step.
#[test]
fn set_density_gives_a_set_of_one_instance_the_new_density() {
    let mut s = session();
    let set = NodeId(s.execute("symbol.spray", &json!({"name": "Dot", "points": [[100, 150]], "radius": 4})).unwrap()["id"].as_u64().unwrap());
    let steps = undo_len(&s);
    assert_eq!(s.execute("symbol.setDensity", &json!({"from": 5, "to": 9, "ids": [set.0]})).unwrap(), json!({"ids": [set.0], "count": 0}));
    assert_eq!(undo_len(&s), steps + 1);
    assert_eq!(s.execute("symbol.setDensity", &json!({"from": 5, "to": 9, "ids": [set.0]})).unwrap(), json!({"ids": [], "count": 0}));
    assert_eq!(undo_len(&s), steps + 1);
    // A second instance, then from the set's 9 to 5: 0.65 / 0.25 of each distance from the middle.
    s.select(|_, sel| sel.set([set])).unwrap();
    s.execute("symbol.spray", &json!({"name": "Dot", "points": [[200, 150]], "radius": 4, "density": 9})).unwrap();
    let ids = members(&s, set);
    assert_eq!(ids.len(), 2);
    let was: Vec<Point> = ids.iter().map(|id| center(&s, *id)).collect();
    let middle = Point::new((was[0].x + was[1].x) / 2.0, (was[0].y + was[1].y) / 2.0);
    s.execute("symbol.setDensity", &json!({"from": 5, "to": 5, "ids": [set.0]})).unwrap();
    for (id, before) in ids.iter().zip(&was) {
        let want = middle + (*before - middle) * (0.65 / 0.25);
        assert!(center(&s, *id).distance(want) < 1e-9, "{before:?} -> {:?}", center(&s, *id));
    }
}

/// A locked instance of the set stays where it is, and the others move.
#[test]
fn set_density_leaves_a_locked_instance_in_place() {
    let mut s = session();
    let row: Vec<Value> = (0..3).map(|i| json!([100.0 + 80.0 * f64::from(i), 150.0])).collect();
    let set = NodeId(s.execute("symbol.spray", &json!({"name": "Dot", "points": row, "radius": 4})).unwrap()["id"].as_u64().unwrap());
    let ids = members(&s, set);
    s.edit("Lock", |d, _| {
        if let Some(n) = d.node_mut(ids[0]) {
            n.locked = true;
        }
        Ok(())
    })
    .unwrap();
    let was: Vec<Point> = ids.iter().map(|id| center(&s, *id)).collect();
    let r = s.execute("symbol.setDensity", &json!({"from": 5, "to": 9, "ids": [set.0]})).unwrap();
    assert!(r["count"].as_u64().is_some_and(|n| (1..=2).contains(&n)), "{r}");
    assert_eq!(center(&s, ids[0]), was[0]);
    assert!(center(&s, ids[2]).distance(was[2]) > 1.0);
}

/// `symbol.setDensity` refuses a density left out, not a number or outside 1..10, an id naming no
/// object, and objects with no symbol set among them.
#[test]
fn set_density_refuses_bad_densities_and_objects_without_a_set() {
    let mut s = session();
    let set =
        NodeId(s.execute("symbol.spray", &json!({"name": "Dot", "points": [[100, 150], [200, 150]], "radius": 4})).unwrap()["id"].as_u64().unwrap());
    for (p, want) in [
        (json!({"to": 5}), "missing `from`".to_string()),
        (json!({"from": 5}), "missing `to`".to_string()),
        (json!({"from": "5", "to": 6}), "`from` must be a number from 1 to 10, not \"5\"".to_string()),
        (json!({"from": 5, "to": 11}), "`to` must be a number from 1 to 10, not 11".to_string()),
        (json!({"from": 0, "to": 5}), "`from` must be a number from 1 to 10, not 0".to_string()),
        (json!({"from": null, "to": 5}), "`from` must be a number from 1 to 10, not null".to_string()),
    ] {
        refused(&mut s, "symbol.setDensity", p, &want);
    }
    let before = state(&s);
    assert!(s.execute("symbol.setDensity", &json!({"from": 5, "to": 9, "ids": [987_654]})).is_err());
    assert!(s.execute("symbol.setDensity", &json!({"from": 5, "to": 9, "ids": "all"})).is_err());
    // An instance is not a set, and neither is nothing selected.
    let member = members(&s, set)[0];
    assert!(s.execute("symbol.setDensity", &json!({"from": 5, "to": 9, "ids": [member.0]})).is_err());
    s.select(|_, sel| sel.clear()).unwrap();
    let r = s.execute("symbol.setDensity", &json!({"from": 5, "to": 9}));
    assert!(matches!(&r, Err(EngineError::Other(msg)) if msg == "no symbol set selected"), "{r:?}");
    assert!(state(&s) == before);
}

/// An instance with no size is shifted and screened as before; only Average, which needs its size,
/// leaves it as it is.
#[test]
fn an_instance_without_a_size_is_changed_by_every_method_but_average() {
    // A Dot at 200, 150 whose size is 0, and Dots at the other centers.
    let flat = |others: &[(f64, f64)]| {
        let mut s = session();
        let a = dot(&mut s, 200.0, 150.0);
        for (x, y) in others {
            dot(&mut s, *x, *y);
        }
        s.edit("Flatten", |d, _| {
            if let Some(NodeKind::SymbolInstance { xf, .. }) = d.node_mut(a).map(|n| &mut n.kind) {
                *xf = Affine::new([0.0, 0.0, 0.0, 0.0, 200.0, 150.0]);
            }
            Ok(())
        })
        .unwrap();
        (s, a)
    };
    let (mut s, a) = flat(&[(210.0, 150.0)]);
    let r = adjust(&mut s, json!({"tool": "shift", "points": [[200, 150], [205, 150]], "radius": 40, "intensity": 10}));
    assert_eq!(r["count"], 2);
    assert!(center(&s, a).x > 200.5, "{:?}", center(&s, a));
    let r = adjust(&mut s, json!({"tool": "screen", "points": [[205, 150]], "radius": 40, "intensity": 10}));
    assert_eq!(r["count"], 2);
    assert!(node(&s, a).opacity < 1.0);
    // Average brings the other two toward their mean and leaves the one without a size as it is.
    let (mut s, a) = flat(&[(190.0, 150.0), (210.0, 150.0)]);
    adjust(&mut s, json!({"tool": "screen", "points": [[190, 150]], "radius": 2, "intensity": 10}));
    let r = adjust(&mut s, json!({"tool": "screen", "points": [[200, 150]], "radius": 40, "intensity": 10, "method": "average"}));
    assert_eq!(r["count"], 2);
    assert_eq!(node(&s, a).opacity, 1.0);
    // With only it under the brush there is no mean, and Average changes nothing.
    let before = state(&s);
    let r = adjust(&mut s, json!({"tool": "screen", "points": [[200, 150]], "radius": 5, "intensity": 10, "method": "average"}));
    assert_eq!(r["count"], 0);
    assert!(state(&s) == before);
}

/// A stroke that leaves every instance under the brush as it is (each already at full stain, or
/// at the least opacity) counts none and adds no undo step.
#[test]
fn a_stroke_that_changes_no_instance_adds_no_undo_step() {
    let mut s = session();
    dot(&mut s, 200.0, 150.0);
    let full = json!(vec![[200, 150]; 12]);
    for tool in ["stain", "screen"] {
        adjust(&mut s, json!({"tool": tool, "points": full, "radius": 40, "intensity": 10, "color": "#ff0000"}));
        let before = state(&s);
        let r = adjust(&mut s, json!({"tool": tool, "points": [[200, 150]], "radius": 40, "intensity": 10, "color": "#ff0000"}));
        assert_eq!(r["count"], 0, "{tool}");
        assert!(state(&s) == before, "{tool} changed the document");
    }
}

/// An instance a stroke moves away and back to where it was is not counted, and the stroke adds
/// no undo step: the Shifter's drag out and back moves it 8 pt right, then 8 pt left.
#[test]
fn an_instance_moved_away_and_back_is_not_counted() {
    let mut s = session();
    let id = dot(&mut s, 200.0, 150.0);
    let before = state(&s);
    let r = adjust(&mut s, json!({"tool": "shift", "points": [[196, 166], [212, 166], [196, 166]], "radius": 40, "intensity": 10}));
    assert_eq!(bounds(&s, id).center(), Point::new(200.0, 150.0));
    assert_eq!(r["count"], 0);
    assert!(state(&s) == before, "the stroke added an undo step");
}
