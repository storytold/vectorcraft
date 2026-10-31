use super::*;

#[test]
fn hidden_attributes_show_again() {
    let mut a = Appearance::default_art();
    assert!(!a.has_hidden() && !a.show_all());
    a.stroke_mut().unwrap().visible = false;
    a.items[0].effects_mut().push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: false });
    assert!(a.has_hidden());
    assert!(a.show_all());
    assert!(!a.has_hidden() && a.items.iter().all(AppearanceItem::visible) && a.items[0].effects()[0].visible);
}

#[test]
fn basic_appearance() {
    let a = Appearance::default_art();
    assert!(a.is_basic());
    assert_eq!(a.fill_paint(), Paint::solid(Color::WHITE));
    assert_eq!(a.stroke_width(), 1.0);
}

#[test]
fn rebase_moves_fills_by_the_box_and_strokes_by_their_inflated_box() {
    use vectorcraft_color::{Gradient, GradientGeom, GradientPaint};
    use vectorcraft_geom::{Point, Rect};
    let placed = |x: f64| {
        let mut g = GradientPaint::new(Gradient::default());
        g.geom = Some(GradientGeom { start: Point::new(x, 0.0), end: Point::new(x + 10.0, 0.0), aspect: 1.0, focal: None });
        Paint::Gradient(Box::new(g))
    };
    let mut a = Appearance::basic(placed(0.0), placed(-5.0), 10.0);
    a.rebase_gradients(Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(100.0, 0.0, 300.0, 100.0));
    let start = |p: Paint| match p {
        Paint::Gradient(g) => g.geom.unwrap().start,
        _ => panic!("a gradient"),
    };
    // The fill's start stays on the left edge; the stroke's on its inflated box's (−5 → 95).
    assert_eq!((start(a.fill_paint()), start(a.stroke_paint())), (Point::new(100.0, 0.0), Point::new(95.0, 0.0)));
}

#[test]
fn items_by_index() {
    let mut a = Appearance::default_art();
    a.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None)));
    // [Fill white, Stroke black, Fill none]
    assert_eq!(a.fill_at(None).unwrap().paint, Paint::None);
    assert_eq!(a.fill_at(Some(0)).unwrap().paint, Paint::solid(Color::WHITE));
    assert!(a.fill_at(Some(1)).is_none() && a.stroke_at(Some(0)).is_none() && a.fill_at(Some(9)).is_none());
    assert_eq!(a.item_of_kind(Some(1), false), Some(1));
    assert_eq!(a.item_of_kind(Some(1), true), None);
    assert_eq!(a.fill_for(Some(1)).unwrap().paint, Paint::None);
    assert!(a.set_fill_at(Some(0), Paint::solid(Color::BLACK)));
    assert!(!a.set_stroke_at(Some(0), Paint::None));
    assert_eq!(a.fill_at(Some(0)).unwrap().paint, Paint::solid(Color::BLACK));
    a.effects_mut(Some(1)).unwrap().push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: true });
    assert_eq!(a.effects_at(Some(1)).unwrap().len(), 1);
    assert!(a.effects_at(None).unwrap().is_empty() && a.effects_mut(Some(3)).is_none());
    assert_eq!((a.items[1].kind_name(), a.items[2].kind_name()), ("stroke", "fill"));
}

#[test]
fn basic_means_one_plain_fill_and_stroke() {
    let stroke = || AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 1.0));
    let mut a = Appearance { items: vec![stroke(), stroke()], ..Appearance::default() };
    assert!(!a.is_basic(), "two strokes");
    a.items.pop();
    assert!(a.is_basic() && Appearance::default().is_basic());
    let mut b = Appearance::default_art();
    b.stroke_mut().unwrap().visible = false;
    assert!(!b.is_basic(), "a hidden stroke");
    let mut c = Appearance::default_art();
    c.fill_mut().unwrap().opacity = 0.5;
    assert!(!c.is_basic(), "fill opacity");
    let mut d = Appearance::default_art();
    d.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None)));
    assert!(!d.is_basic(), "two fills");
    let mut e = Appearance::default_art();
    e.effects.push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: true });
    assert!(!e.is_basic(), "an effect");
}

#[test]
fn set_creates_missing() {
    let mut a = Appearance::default();
    a.set_fill(Paint::solid(Color::BLACK));
    a.set_stroke(Paint::solid(Color::WHITE));
    assert_eq!(a.items.len(), 2);
    assert!(matches!(a.items[0], AppearanceItem::Fill(_)));
}

/// #852: pen pressure is kept simplified along the stroke and read back by position.
#[test]
fn pressure_profile_samples_simplify_and_interpolate() {
    // A steady ramp keeps only its ends; a bump keeps its peak.
    let ramp = PressureProfile::from_samples((0..=100).map(|i| (i as f64 / 100.0, i as f64 / 100.0))).unwrap();
    assert_eq!(ramp.points, vec![(0.0, 0.0), (1.0, 1.0)]);
    assert!((ramp.at(0.25) - 0.25).abs() < 1e-9);
    let bump = PressureProfile::from_samples([(0.0, 0.2), (0.3, 0.2), (0.5, 0.9), (0.7, 0.2), (1.0, 0.2)]).unwrap();
    assert!(bump.points.contains(&(0.5, 0.9)) && (bump.at(0.4) - 0.55).abs() < 1e-9, "{bump:?}");
    // Junk is clamped or dropped; positions never go back; nothing at all is no profile.
    let junk = PressureProfile::from_samples([(f64::NAN, 0.5), (0.5, 7.0), (0.2, -1.0), (2.0, f64::INFINITY)]).unwrap();
    assert_eq!(junk.points, vec![(0.5, 1.0), (0.5, 0.0)]);
    assert!(PressureProfile::from_samples([(f64::NAN, 0.5)]).is_none());
    assert_eq!(PressureProfile::default().at(0.3), PressureProfile::MID);
    // A long noisy stroke stays bounded.
    let noisy = PressureProfile::from_samples((0..1_000_000).map(|i| (i as f64 / 1e6, (i % 2) as f64))).unwrap();
    assert!(noisy.points.len() <= PressureProfile::MAX_POINTS);
    // Out-of-order points (a hand-edited file) still read within 0..1.
    let odd = PressureProfile { points: vec![(1.0, 5.0), (0.0, -3.0)] };
    assert!((0.0..=1.0).contains(&odd.at(0.5)));
}

#[test]
fn pressure_profile_extends_and_reverses() {
    let old = PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] };
    let added = PressureProfile { points: vec![(0.0, 1.0), (1.0, 0.0)] };
    // A 30 pt path continued by 10 pt: its ramp fills the first three quarters.
    let joined = PressureProfile::extended(Some(&old), 30.0, Some(&added), 10.0).unwrap();
    assert!((joined.at(0.75) - 1.0).abs() < 1e-9 && (joined.at(0.375) - 0.5).abs() < 1e-9 && joined.at(1.0) == 0.0, "{joined:?}");
    // A side without pressure is half way.
    let half = PressureProfile::extended(None, 10.0, Some(&added), 10.0).unwrap();
    assert_eq!(half.at(0.25), PressureProfile::MID);
    assert!(PressureProfile::extended(None, 10.0, None, 10.0).is_none());
    assert_eq!(PressureProfile::extended(Some(&old), 0.0, None, f64::NAN), Some(old.clone()));
    assert_eq!(old.reversed().points, vec![(0.0, 1.0), (1.0, 0.0)]);
}

/// Strokes saved before pen pressure load as they were, and are written without it.
#[test]
fn strokes_without_pressure_round_trip_unchanged() {
    let json = serde_json::to_value(StrokeLayer::new(Paint::None, 2.0)).unwrap();
    assert!(json.get("pressure").is_none());
    let st: StrokeLayer = serde_json::from_value(json).unwrap();
    assert_eq!(st.pressure, None);
    let pressed = StrokeLayer { pressure: Some(PressureProfile::sample()), ..st };
    let back: StrokeLayer = serde_json::from_value(serde_json::to_value(&pressed).unwrap()).unwrap();
    assert_eq!(back, pressed);
    let mut ap = Appearance::basic(Paint::None, Paint::None, 1.0);
    ap.stroke_mut().unwrap().pressure = Some(PressureProfile::sample());
    assert_eq!(ap.without_pressure().stroke().unwrap().pressure, None);
}

#[test]
fn profile_interp() {
    let p = WidthProfile::lens();
    assert_eq!(p.at(0.5), (1.0, 1.0));
    assert_eq!(p.at(0.25), (0.5, 0.5));
    assert_eq!(p.at(2.0), (0.0, 0.0));
}

#[test]
fn profile_presets_round_trip_their_ids() {
    for p in WidthProfile::PRESETS {
        let prof = WidthProfile::preset(p.id).unwrap();
        assert_eq!(prof.preset_id(), Some(p.id));
        assert_eq!(WidthProfile::id_of(Some(&prof)), p.id);
    }
    assert_eq!(WidthProfile::id_of(None), "uniform");
    assert_eq!(WidthProfile::id_of(Some(&WidthProfile { points: vec![(0.0, 0.3, 0.3)] })), "custom");
    assert!(WidthProfile::preset("nope").is_none());
    assert_eq!(WidthProfile::lens().points, vec![(0.0, 0.0, 0.0), (0.5, 1.0, 1.0), (1.0, 0.0, 0.0)]);
}

#[test]
fn arrowhead_names_are_stable_and_every_one_is_listed_once() {
    // Saved files store these names: they may only ever be appended to.
    const SAVED: [&str; 40] = [
        "Triangle",
        "TriangleOpen",
        "Circle",
        "CircleOpen",
        "Square",
        "SquareOpen",
        "Bar",
        "Diamond",
        "Arrow",
        "ArrowOpen",
        "Barbed",
        "HalfArrowLeft",
        "HalfArrowRight",
        "Concave",
        "DoubleBar",
        "Feather",
        "DotOnBar",
        "Chevron",
        "DoubleArrow",
        "Target",
        "Star",
        "Cross",
        "Plus",
        "Hexagon",
        "HexagonOpen",
        "Tag",
        "TagOpen",
        "HalfCircle",
        "Drop",
        "Slash",
        "DoubleSlash",
        "DiamondOpen",
        "TriangleReverse",
        "Swallowtail",
        "Bracket",
        "Fork",
        "Leaf",
        "Kite",
        "TriangleBar",
        "Oval",
    ];
    for name in SAVED {
        let a: Arrowhead = serde_json::from_value(serde_json::json!(name)).unwrap();
        assert_eq!(serde_json::to_value(a).unwrap(), name);
        assert_eq!(Arrowhead::ALL.iter().filter(|x| **x == a).count(), 1, "{name} in the menu once");
    }
    let labels: std::collections::HashSet<&str> = Arrowhead::ALL.iter().map(|a| a.label()).collect();
    assert_eq!(labels.len(), Arrowhead::ALL.len(), "labels are distinct");
}

#[test]
fn arrow_align_defaults_to_extend_and_round_trips() {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 2.0);
    assert_eq!(st.arrow_align, ArrowAlign::Extend);
    // The default is not written, so older readers see the same JSON as before.
    assert!(!serde_json::to_string(&st).unwrap().contains("arrow_align"));
    let old: StrokeLayer = serde_json::from_str(r#"{"paint":{"type":"none"},"width":2.0}"#).unwrap();
    assert_eq!(old.arrow_align, ArrowAlign::Extend);
    st.arrow_align = ArrowAlign::Tip;
    let back: StrokeLayer = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
    assert_eq!(back, st);
}

#[test]
fn outset_covers_arrowheads() {
    let mut a = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4.0);
    a.stroke_mut().unwrap().join = LineJoin::Round;
    assert_eq!(a.outset(), 2.0);
    let st = a.stroke_mut().unwrap();
    st.end_arrow = Some(Arrowhead::Triangle);
    st.arrow_scale = (100.0, 200.0);
    assert_eq!(st.arrow_weight(false), 4.0);
    assert_eq!(st.arrow_weight(true), 8.0);
    assert_eq!((st.head_reach(false), st.head_reach(true)), (0.0, HEAD_REACH * 8.0 + 2.0));
    // A 32 pt head whose tip sits up to its length (plus the cap) past the end point.
    assert_eq!(a.outset(), HEAD_REACH * 8.0 + 2.0);
    a.stroke_mut().unwrap().width = 0.01;
    assert_eq!(a.stroke().unwrap().arrow_weight(true), 0.25, "heads keep a minimum size");
}

#[test]
fn outset_depends_on_align() {
    let mut a = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 10.0);
    a.stroke_mut().unwrap().join = LineJoin::Round;
    assert_eq!(a.outset(), 5.0);
    a.stroke_mut().unwrap().align = StrokeAlign::Outside;
    assert_eq!(a.outset(), 10.0);
    // Open paths stroke inside-aligned strokes centred.
    a.stroke_mut().unwrap().align = StrokeAlign::Inside;
    assert_eq!(a.outset(), 5.0);
    assert_eq!((a.stroke().unwrap().side_reach(true), a.stroke().unwrap().body(true)), (0.0, (10.0, Some(true))));
}

#[test]
fn outset_covers_miter_spikes_projecting_caps_and_profile_maxima() {
    let mut a = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0);
    // Miter joins can spike out to the limit × half the weight.
    assert_eq!(a.outset(), 10.0);
    let st = a.stroke_mut().unwrap();
    st.join = LineJoin::Bevel;
    st.cap = LineCap::Square;
    assert!((a.outset() - std::f64::consts::SQRT_2).abs() < 1e-12, "a projecting cap's corner");
    let st = a.stroke_mut().unwrap();
    st.cap = LineCap::Round;
    st.profile = Some(WidthProfile { points: vec![(0.0, 0.5, 2.5), (1.0, 1.0, 1.0)] });
    assert_eq!(st.profile_max(), 2.5);
    assert_eq!(a.outset(), 2.5);
    a.stroke_mut().unwrap().paint = Paint::None;
    assert_eq!(a.outset(), 0.0, "an unpainted stroke");
}

#[test]
fn contents_slot_follows_item_edits_and_round_trips() {
    let fill = || AppearanceItem::Fill(FillLayer::new(Paint::None));
    let mut a = Appearance { items: vec![fill(), fill(), fill()], ..Appearance::default() };
    assert_eq!((a.contents_at(), a.split_contents().1.len()), (0, 3), "all items above by default");
    a.set_contents_at(1);
    assert!(!a.is_basic());
    // [0 | 1 2]: removing below shifts the slot, removing above doesn't.
    a.remove_item(2);
    assert_eq!(a.contents_at(), 1);
    a.remove_item(0);
    assert_eq!((a.contents_at(), a.contents_index), (0, None));
    // Inserting next to the slot joins the item below it.
    a.items.push(fill());
    a.set_contents_at(1); // [0 | 1]
    a.insert_item(1, fill());
    assert_eq!(a.contents_at(), 2);
    a.insert_item(3, fill()); // [0 1 | 2 3]
    assert_eq!(a.contents_at(), 2);
    // Moves keep their side when landing next to the slot, cross it otherwise.
    assert_eq!(a.move_item(3, 2), Some(2));
    assert_eq!(a.contents_at(), 2, "stays above");
    a.move_item(3, 0);
    assert_eq!(a.contents_at(), 3, "crossed below");
    a.move_item(0, 3);
    assert_eq!(a.contents_at(), 2, "crossed above");
    assert_eq!(a.move_item(9, 0), None);
    // Saved only when set; older files load with every item above.
    let json = serde_json::to_string(&a).unwrap();
    assert!(json.contains("\"contents_index\":2"));
    assert_eq!(serde_json::from_str::<Appearance>(&json).unwrap(), a);
    let old: Appearance = serde_json::from_str(r#"{"items":[]}"#).unwrap();
    assert_eq!(old.contents_index, None);
    assert!(!serde_json::to_string(&Appearance::default_art()).unwrap().contains("contents_index"));
}

/// The issue's case (#4174): two circles with different fills, the same stroke and the same
/// effect share the stroke and the effect, not the fills; a shared stroke at another place in one
/// stack maps to its own index there; nothing shared shares nothing; equal stacks share all.
#[test]
fn mixed_appearances_share_their_equal_rows() {
    let stroke = AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 11.0));
    let fill = |c: Color| AppearanceItem::Fill(FillLayer::new(Paint::solid(c)));
    let feather = Effect { id: "stylize.feather".into(), params: serde_json::json!({"radius": 5}), visible: true };
    let a = Appearance { items: vec![fill(Color::rgb(1.0, 1.0, 0.0)), stroke.clone()], effects: vec![feather.clone()], ..Appearance::default() };
    let b = Appearance { items: vec![fill(Color::rgb(1.0, 0.0, 0.0)), stroke.clone()], effects: vec![feather.clone()], ..Appearance::default() };
    let shared = SharedStack::of(&[&a, &b]);
    assert_eq!(shared.items, vec![vec![1, 1]], "the stroke, not the fills");
    assert_eq!(shared.effects, vec![vec![0, 0]]);
    // The stroke under an extra fill in the second object.
    let c =
        Appearance { items: vec![fill(Color::BLACK), fill(Color::WHITE), stroke.clone()], effects: vec![feather.clone()], ..Appearance::default() };
    let shared = SharedStack::of(&[&a, &c]);
    assert_eq!(shared.items, vec![vec![1, 2]]);
    assert_eq!(shared.item_in(1, 1), Some(2));
    assert_eq!(shared.item_in(0, 1), None);
    // Nothing alike.
    let d = Appearance { items: vec![fill(Color::WHITE)], ..Appearance::default() };
    let none = SharedStack::of(&[&a, &d]);
    assert!(none.items.is_empty() && none.effects.is_empty());
    // Equal stacks share everything, each row where it is.
    assert_eq!(SharedStack::of(&[&a, &a.clone(), &a.clone()]).items, vec![vec![0, 0, 0], vec![1, 1, 1]]);
}

/// Matches keep their stacking order: two equal rows in the opposite order match once.
#[test]
fn aligned_rows_never_cross() {
    assert_eq!(align(&[1, 2], &[2, 1], |a, b| a == b).iter().filter(|m| m.is_some()).count(), 1);
    assert_eq!(align(&[1, 2, 3], &[9, 1, 3], |a, b| a == b), vec![Some(1), None, Some(2)]);
    assert_eq!(align::<i32>(&[], &[1], |a, b| a == b), Vec::<Option<usize>>::new());
}
