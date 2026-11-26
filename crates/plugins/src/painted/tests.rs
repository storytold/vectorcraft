use serde_json::json;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, StrokeLayer};

use super::*;

const SQUARE: &str = r#"{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[10,0]},{"p":[10,10]},{"p":[0,10]}]}]}"#;

fn red() -> Value {
    serde_json::to_value(Paint::solid(Color::rgb(1.0, 0.0, 0.0))).unwrap()
}

fn out(pieces: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({ "objects": pieces })).unwrap()
}

fn square() -> Value {
    serde_json::from_str(SQUARE).unwrap()
}

/// Each piece keeps its own paints in the order returned; a piece without `fills` or `strokes`
/// keeps the ones it was given (`None`) and an empty list paints none.
#[test]
fn pieces_keep_their_own_paints_in_order() {
    let blue = serde_json::to_value(Paint::solid(Color::rgb(0.0, 0.0, 1.0))).unwrap();
    let pieces = decode(
        &out(json!([
            {"path": square(), "fills": [red()], "strokes": [{"paint": blue, "width": 4, "join": "round", "cap": "square", "miterLimit": 2}]},
            {"type": "path", "path": square(), "fillRule": "evenodd", "fills": [], "bounds": [0, 0, 10, 10]},
            {"path": square()},
        ])),
        Scope::Object,
        FillRule::NonZero,
    )
    .unwrap();
    assert_eq!(pieces.len(), 3);
    assert_eq!(pieces[0].fills, Some(vec![Paint::solid(Color::rgb(1.0, 0.0, 0.0))]));
    let s = &pieces[0].strokes.as_ref().unwrap()[0];
    assert_eq!((s.width, s.join, s.cap, s.miter_limit), (Some(4.0), Some(LineJoin::Round), Some(LineCap::Square), Some(2.0)));
    assert_eq!((pieces[1].rule, pieces[1].fills.as_deref(), &pieces[1].strokes), (FillRule::EvenOdd, Some(&[][..]), &None));
    assert_eq!((pieces[2].rule, &pieces[2].fills, &pieces[2].strokes), (FillRule::NonZero, &None, &None));
}

/// A solid paint or gradient stop linked to a global swatch takes the swatch's color at its own
/// tint. A link to a swatch that isn't global and a paint without a link keep the color given.
#[test]
fn linked_paints_take_the_global_swatch_color() {
    let brand = Color::rgb(0.2, 0.4, 0.6);
    let mut doc = Document::new(100.0, 100.0);
    doc.swatches.push(Swatch { name: "Brand".into(), paint: Paint::solid(brand), global: true, spot: false });
    doc.swatches.push(Swatch { name: "Plain".into(), paint: Paint::solid(brand), global: false, spot: false });
    let given = json!({"model": "rgb", "r": 1, "g": 1, "b": 0});
    let solid = |extra: Value| {
        let mut p = json!({"type": "solid", "color": given});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    let stop = json!({"offset": 0, "color": given, "swatch": "Brand", "tint": 0.5});
    let gradient = json!({"type": "gradient", "gradient": {"kind": "Linear", "stops": [stop]}});
    let fills = json!([solid(json!({"swatch": "Brand", "tint": 0.5})), solid(json!({"swatch": "Plain"})), solid(json!({})), gradient]);
    let strokes = json!([{"paint": solid(json!({"swatch": "Brand"}))}]);
    let pieces = decode(&out(json!([{"path": square(), "fills": fills, "strokes": strokes}])), Scope::Object, FillRule::NonZero).unwrap();
    let pieces = with_swatch_colors(std::sync::Arc::new(pieces), &doc);
    let color = |p: &Paint| match p {
        Paint::Solid { color, .. } => *color,
        Paint::Gradient(g) => g.gradient.stops[0].color,
        _ => panic!("{p:?}"),
    };
    let fills = pieces[0].fills.as_ref().unwrap();
    let old = Color::rgb(1.0, 1.0, 0.0);
    assert_eq!([color(&fills[0]), color(&fills[1]), color(&fills[2]), color(&fills[3])], [brand.tinted(0.5), old, old, brand.tinted(0.5)]);
    assert_eq!(color(&pieces[0].strokes.as_ref().unwrap()[0].paint), brand);
}

/// What a piece is drawn with: the object's appearance with the paints the piece gives in place of
/// its own (kept, cleared or replaced), and for an effect on one fill or stroke, copies of that item.
#[test]
fn a_piece_s_appearance_inherits_or_replaces() {
    let base = Appearance {
        items: vec![
            AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::WHITE))),
            AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 3.0)),
        ],
        ..Appearance::default()
    };
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let piece = |fills: Option<Vec<Paint>>, strokes: Option<Vec<OutStroke>>| Piece {
        path: PathData::default(),
        rule: FillRule::NonZero,
        fills,
        strokes,
        look: Look::default(),
        fill_looks: vec![],
        stroke_looks: vec![],
    };
    assert_eq!(appearance(&base, &piece(None, None)), base, "inherited");
    let ap = appearance(&base, &piece(Some(vec![red.clone()]), Some(vec![])));
    assert_eq!(ap.items.len(), 1);
    assert_eq!(ap.items[0].paint(), &red);
    let ap = appearance(&base, &piece(None, Some(vec![OutStroke { width: Some(8.0), ..OutStroke::paint(red.clone()) }])));
    let AppearanceItem::Stroke(s) = &ap.items[1] else { panic!("a stroke") };
    assert_eq!((s.width, &s.paint), (8.0, &red));
    let two = items(&base.items[0], &piece(Some(vec![red.clone(), Paint::None]), None));
    assert_eq!(two.iter().map(|i| i.paint().clone()).collect::<Vec<_>>(), [red.clone(), Paint::None]);
    assert_eq!(items(&base.items[1], &piece(None, None)), vec![base.items[1].clone()]);
    let none = items(&base.items[1], &piece(None, Some(vec![])));
    assert!(none.is_empty());
}

/// An effect on a fill returns only fills, one on a stroke only strokes.
#[test]
fn an_item_s_effect_paints_only_its_kind() {
    let fills = out(json!([{"path": square(), "fills": [red()]}]));
    let strokes = out(json!([{"path": square(), "strokes": [{"paint": red()}]}]));
    assert!(decode(&fills, Scope::Fill, FillRule::NonZero).is_ok());
    assert!(decode(&strokes, Scope::Stroke, FillRule::NonZero).is_ok());
    let e = decode(&strokes, Scope::Fill, FillRule::NonZero).unwrap_err().to_string();
    assert!(e.contains("objects[0].strokes"), "{e}");
    let e = decode(&fills, Scope::Stroke, FillRule::NonZero).unwrap_err().to_string();
    assert!(e.contains("objects[0].fills"), "{e}");
}

/// Every fault refuses the whole result and names its field; nothing is clamped.
#[test]
fn faults_refuse_the_whole_result_by_field() {
    let ok = json!({"path": square(), "fills": [red()]});
    let stop = |offset: f64| json!({"type": "gradient", "gradient": {"kind": "Linear", "stops": [{"offset": offset, "color": {"model": "rgb", "r": 0, "g": 0, "b": 0}}]}});
    let many: Vec<Value> = (0..=MAX_ITEMS).map(|_| red()).collect();
    let stops: Vec<Value> = (0..=MAX_ITEMS).map(|_| json!({"offset": 0, "color": {"model": "gray", "k": 0}})).collect();
    for (piece, at) in [
        (json!({"path": square(), "fills": [{"type": "plaid"}]}), "objects[1].fills[0]"),
        (json!({"path": square(), "fills": [{"type": "solid", "color": {"model": "rgb", "r": 2, "g": 0, "b": 0}}]}), "objects[1].fills[0].color"),
        (json!({"path": square(), "fills": [{"type": "solid", "color": {"model": "rgb", "r": 1e300, "g": 0, "b": 0}}]}), "objects[1].fills[0].color"),
        (json!({"path": square(), "fills": [{"type": "solid", "color": {"model": "gray", "k": 0.5}, "tint": 1.5}]}), "objects[1].fills[0].tint"),
        (json!({"path": square(), "strokes": [{"paint": red(), "width": 1001}]}), "objects[1].strokes[0].width"),
        (json!({"path": square(), "strokes": [{"paint": red(), "width": -1}]}), "objects[1].strokes[0].width"),
        (json!({"path": square(), "strokes": [{"paint": red(), "miterLimit": 0.5}]}), "objects[1].strokes[0].miterLimit"),
        (json!({"path": square(), "strokes": [{"paint": red(), "join": "wobbly"}]}), "objects[1].strokes[0].join"),
        (json!({"path": square(), "strokes": [{"paint": red(), "dashes": [1, 2]}]}), "objects[1].strokes[0].dashes"),
        (json!({"path": square(), "fills": [stop(1.5)]}), "objects[1].fills[0].gradient.stops[0].offset"),
        (
            json!({"path": square(), "fills": [{"type": "gradient", "gradient": {"kind": "Linear", "stops": stops}}]}),
            "objects[1].fills[0].gradient.stops",
        ),
        (json!({"path": square(), "fills": many}), "objects[1].fills"),
        (json!({"path": square(), "fills": [red()], "opacity": 0.5}), "objects[1].opacity"),
        (json!({"path": square(), "effects": []}), "objects[1].effects"),
        (json!({"path": square(), "guide": true}), "objects[1].guide"),
        (json!({"path": square(), "locked": true}), "objects[1].locked"),
        (json!({"fills": [red()]}), "objects[1].path"),
        (json!({"path": square(), "fillRule": "inside"}), "objects[1].fillRule"),
        (json!({"path": {"subpaths": [{"closed": true, "anchors": [{"p": [1e9, 0]}]}]}}), "objects[1].path"),
    ] {
        let e = decode(&out(json!([ok, piece])), Scope::Object, FillRule::NonZero).unwrap_err();
        let e = e.to_string();
        assert!(e.contains(&format!("{at}:")), "{at}: {e}");
    }
    let e = decode(&serde_json::to_vec(&json!({"objects": [ok], "documentData": {}})).unwrap(), Scope::Object, FillRule::NonZero).unwrap_err();
    assert!(e.to_string().contains("documentData"), "{e}");
    let declined = decode(br#"{"error":"nothing to do"}"#, Scope::Object, FillRule::NonZero).unwrap_err();
    assert_eq!(declined, Error::Failed("nothing to do".into()));
}

/// A top-level `notice` is ignored, as a filter-only key a live effect may hand back; other
/// unknown top-level keys still refuse the result.
#[test]
fn a_notice_is_ignored_and_other_top_level_keys_refused() {
    let ok = json!({"path": square(), "fills": [red()]});
    let decode_top = |top: Value| decode(&serde_json::to_vec(&top).unwrap(), Scope::Object, FillRule::NonZero);
    let pieces = decode_top(json!({"objects": [ok], "notice": "Done"})).unwrap();
    assert_eq!(pieces.len(), 1);
    assert_eq!(decode_top(json!({"objects": [ok], "notice": 12})).unwrap().len(), 1);
    for key in ["note", "notices", "message"] {
        let e = decode_top(json!({"objects": [ok], key: "Done"})).unwrap_err().to_string();
        assert!(e.contains(&format!("{key}:")) && e.contains("returns only `objects` and `notice`"), "{key}: {e}");
    }
}

/// A key a paint does not have, at any level of it, refuses the result and is named; it is not
/// dropped.
#[test]
fn unknown_keys_inside_a_paint_are_refused_by_field() {
    let gray = json!({"model": "gray", "k": 0});
    let stop = |extra: Value| {
        let mut s = json!({"offset": 0, "color": gray});
        s.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json!({"type": "gradient", "gradient": {"kind": "Linear", "stops": [s]}})
    };
    let point = json!({"x": 0, "y": 0});
    for (fill, at) in [
        (json!({"type": "solid", "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}, "opacity": 0.5}), "objects[0].fills[0].opacity"),
        (json!({"type": "solid", "color": {"model": "gray", "k": 0, "alpha": 0.5}}), "objects[0].fills[0].color.alpha"),
        (json!({"type": "none", "color": gray}), "objects[0].fills[0].color"),
        (json!({"type": "pattern", "pattern": "Dots", "scale": 2}), "objects[0].fills[0].scale"),
        (stop(json!({"blend": "multiply"})), "objects[0].fills[0].gradient.stops[0].blend"),
        (stop(json!({"color": {"model": "lab", "l": 50, "a": 0, "b": 0, "c": 1}})), "objects[0].fills[0].gradient.stops[0].color.c"),
        (
            json!({"type": "gradient", "gradient": {"kind": "Linear", "stops": [{"offset": 0, "color": gray}], "opacity": 0.5}}),
            "objects[0].fills[0].gradient.opacity",
        ),
        (
            json!({"type": "gradient", "gradient": {"kind": "Linear", "stops": [{"offset": 0, "color": gray}]}, "opacity": 0.5}),
            "objects[0].fills[0].opacity",
        ),
        (
            json!({"type": "gradient", "gradient": {"kind": "Radial", "stops": [{"offset": 0, "color": gray}]},
                   "geom": {"start": point, "end": {"x": 1, "y": 0, "z": 0}}}),
            "objects[0].fills[0].geom.end.z",
        ),
        (
            json!({"type": "gradient", "gradient": {"kind": "Freeform", "stops": [{"offset": 0, "color": gray}]},
                   "freeform": {"points": [{"at": point, "color": gray, "glow": 1}]}}),
            "objects[0].fills[0].freeform.points[0].glow",
        ),
    ] {
        let e = decode(&out(json!([{"path": square(), "fills": [fill]}])), Scope::Object, FillRule::NonZero).unwrap_err().to_string();
        assert!(e.contains(&format!("{at}:")), "{at}: {e}");
    }
    let e = decode(
        &out(json!([{"path": square(), "strokes": [{"paint": {"type": "solid", "color": gray, "opacity": 0.5}}]}])),
        Scope::Object,
        FillRule::NonZero,
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("objects[0].strokes[0].paint.opacity:"), "{e}");
    // Every key a paint does have is still taken.
    let full = json!({"type": "gradient", "angle": 30, "swatch": "Ramp",
        "gradient": {"kind": "Freeform", "dither": true, "method": "Perceptual",
                     "stops": [{"offset": 0, "color": gray, "opacity": 1, "midpoint": 0.5, "swatch": "Ink", "tint": 0.5, "plates": [0, 0, 0, 0]}]},
        "geom": {"start": point, "end": {"x": 1, "y": 0}, "aspect": 1, "focal": point},
        "freeform": {"points": [{"at": point, "color": gray, "opacity": 1, "spread": 0.5}], "lines": [[0]], "mode": "lines"}});
    let solid = json!({"type": "solid", "color": {"model": "cmyk", "c": 0, "m": 0, "y": 0, "k": 1}, "swatch": "Ink", "tint": 0.5});
    let pattern = json!({"type": "pattern", "pattern": "Dots", "xf": [1, 0, 0, 1, 0, 0]});
    assert!(
        decode(&out(json!([{"path": square(), "fills": [full, solid, pattern, {"type": "none"}, null]}])), Scope::Object, FillRule::NonZero).is_ok()
    );
}

/// The swatches and patterns a piece's paints name must be in the document.
#[test]
fn named_swatches_and_patterns_must_exist() {
    let mut doc = Document::new(100.0, 100.0);
    doc.swatches.push(Swatch { name: "Brand".into(), paint: Paint::solid(Color::rgb(0.2, 0.4, 0.6)), global: true, spot: false });
    let linked = |name: &str| json!({"type": "solid", "color": {"model": "rgb", "r": 0.2, "g": 0.4, "b": 0.6}, "swatch": name});
    let pieces = |pieces: Value| decode(&out(pieces), Scope::Object, FillRule::NonZero).unwrap();
    let check = |p: Value| check_references(&pieces(p), Some(&doc));
    assert!(check(json!([{"path": square(), "fills": [linked("Brand")]}])).is_ok());
    let e = check(json!([{"path": square()}, {"path": square(), "strokes": [{"paint": linked("Gone")}]}])).unwrap_err().to_string();
    assert!(e.contains("objects[1].strokes[0].paint.swatch"), "{e}");
    let stop =
        json!({"type": "gradient", "gradient": {"kind": "Linear", "stops": [{"offset": 0, "color": {"model": "gray", "k": 0}, "swatch": "Gone"}]}});
    let e = check(json!([{"path": square(), "fills": [stop]}])).unwrap_err().to_string();
    assert!(e.contains("objects[0].fills[0].gradient.stops[0].swatch"), "{e}");
    let e = check(json!([{"path": square(), "fills": [{"type": "pattern", "pattern": "Dots"}]}])).unwrap_err().to_string();
    assert!(e.contains("objects[0].fills[0].pattern"), "{e}");
    // Without a document to look them up in, no swatch or pattern may be named.
    assert!(check_references(&pieces(json!([{"path": square(), "fills": [red()]}])), None).is_ok());
    let e = check_references(&pieces(json!([{"path": square(), "fills": [linked("Brand")]}])), None).unwrap_err().to_string();
    assert!(e.contains("objects[0].fills[0].swatch"), "{e}");
    let e = check_references(&pieces(json!([{"path": square(), "fills": [{"type": "pattern", "pattern": "Dots"}]}])), None).unwrap_err().to_string();
    assert!(e.contains("objects[0].fills[0].pattern"), "{e}");
}

/// A gradient of two stops: a plain first one, then `stop`, with `extra` keys on the paint.
fn ramp(stop: Value, extra: Value) -> Value {
    let mut g = json!({"type": "gradient", "gradient": {"kind": "Linear", "stops": [{"offset": 0, "color": {"model": "gray", "k": 0}}, stop]}});
    g.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    g
}

/// A second stop with process `plates`, and `extra` keys.
fn plated(extra: Value) -> Value {
    let mut s = json!({"offset": 1, "color": {"model": "gray", "k": 1}, "plates": [0.1, 0.2, 0.3, 0.4]});
    s.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s
}

/// A gradient stop's process `plates` need the stop's own link to a swatch: a stop without one
/// (no `swatch` or a `null` one) refuses the whole result, naming the stop's `plates`, in a fill or
/// a stroke at every scope.
#[test]
fn unlinked_gradient_stop_plates_refuse_the_whole_painted_result() {
    const WHY: &str = "`plates` requires a stop linked to a spot swatch";
    for orphan in [ramp(plated(json!({})), json!({})), ramp(plated(json!({"swatch": null})), json!({}))] {
        let fills = out(json!([{"path": square(), "fills": [red()]}, {"path": square(), "fills": [red(), orphan]}]));
        for scope in [Scope::Object, Scope::Fill] {
            let e = decode(&fills, scope, FillRule::NonZero).unwrap_err();
            assert_eq!(e.to_string(), format!("plug-in output: objects[1].fills[1].gradient.stops[1].plates: {WHY}"), "{scope:?}");
        }
        let strokes = out(json!([{"path": square(), "strokes": [{"paint": red()}]}, {"path": square(), "strokes": [{"paint": orphan, "width": 2}]}]));
        for scope in [Scope::Object, Scope::Stroke] {
            let e = decode(&strokes, scope, FillRule::NonZero).unwrap_err();
            assert_eq!(e.to_string(), format!("plug-in output: objects[1].strokes[0].paint.gradient.stops[1].plates: {WHY}"), "{scope:?}");
        }
    }
    // `plugin.info`'s `lastError` names the plug-in before it.
    let e = decode(&out(json!([{"path": square(), "fills": [ramp(plated(json!({})), json!({}))]}])), Scope::Object, FillRule::NonZero).unwrap_err();
    assert_eq!(named("Test Tone", &e), format!("Test Tone: plug-in output: objects[0].fills[0].gradient.stops[1].plates: {WHY}"));
    // A plate out of range keeps its own error.
    let wide = ramp(json!({"offset": 1, "color": {"model": "gray", "k": 1}, "plates": [2, 0, 0, 0]}), json!({}));
    let e = decode(&out(json!([{"path": square(), "fills": [wide]}])), Scope::Object, FillRule::NonZero).unwrap_err().to_string();
    assert!(e.contains("objects[0].fills[0].gradient.stops[1].plates: a plate must be from 0 to 1"), "{e}");
}

/// The gradient's own `swatch` does not link its stops, so it does not admit a stop's `plates`.
#[test]
fn gradient_swatch_does_not_link_its_stops_plates() {
    let fill = ramp(plated(json!({})), json!({"swatch": "Ink"}));
    let e = decode(&out(json!([{"path": square(), "fills": [fill]}])), Scope::Object, FillRule::NonZero).unwrap_err().to_string();
    assert!(e.contains("objects[0].fills[0].gradient.stops[1].plates: `plates` requires a stop linked to a spot swatch"), "{e}");
}

/// A stop's `plates` beside a link to a swatch that is not a spot swatch (a process swatch or
/// Registration) refuse the result at the reference check; a link to a missing swatch keeps the
/// error that names the swatch.
#[test]
fn process_swatch_cannot_back_stop_plates() {
    let mut doc = Document::new(100.0, 100.0);
    doc.swatches.push(Swatch { name: "Ink".into(), paint: Paint::solid(Color::cmyk(0.0, 0.5, 1.0, 0.0)), global: true, spot: true });
    doc.swatches.push(Swatch { name: "Process".into(), paint: Paint::solid(Color::cmyk(0.0, 0.5, 1.0, 0.0)), global: true, spot: false });
    let check = |pieces: Value| check_references(&decode(&out(pieces), Scope::Object, FillRule::NonZero).unwrap(), Some(&doc));
    for name in ["Process", vectorcraft_color::swatch::REGISTRATION] {
        let fill = ramp(plated(json!({"swatch": name})), json!({}));
        let e = check(json!([{"path": square(), "fills": [fill.clone()]}])).unwrap_err().to_string();
        assert!(e.contains("objects[0].fills[0].gradient.stops[1].plates: `plates` requires a stop linked to a spot swatch"), "{name}: {e}");
        let e = check(json!([{"path": square()}, {"path": square(), "strokes": [{"paint": fill}]}])).unwrap_err().to_string();
        assert!(e.contains("objects[1].strokes[0].paint.gradient.stops[1].plates:"), "{name}: {e}");
        // Without `plates`, a stop may link to any swatch of the document.
        let plain = ramp(json!({"offset": 1, "color": {"model": "gray", "k": 1}, "swatch": name}), json!({}));
        assert!(check(json!([{"path": square(), "fills": [plain]}])).is_ok(), "{name}");
    }
    let gone = ramp(plated(json!({"swatch": "Gone"})), json!({}));
    let e = check(json!([{"path": square(), "fills": [gone]}])).unwrap_err().to_string();
    assert!(e.contains("objects[0].fills[0].gradient.stops[1].swatch:"), "{e}");
}

/// A stop linked to a spot swatch keeps its `plates` as given, all-zero ones included, through the
/// reference check and the swatch colors.
#[test]
fn linked_spot_stop_plates_are_kept() {
    let mut doc = Document::new(100.0, 100.0);
    doc.swatches.push(Swatch { name: "Ink".into(), paint: Paint::solid(Color::cmyk(0.0, 0.5, 1.0, 0.0)), global: true, spot: true });
    let zero = json!({"offset": 0, "color": {"model": "gray", "k": 0}, "swatch": "Ink", "plates": [0, 0, 0, 0]});
    let fill = json!({"type": "gradient", "gradient": {"kind": "Linear", "stops": [zero, plated(json!({"swatch": "Ink"}))]}});
    let stroke = ramp(plated(json!({"swatch": "Ink"})), json!({}));
    let pieces =
        decode(&out(json!([{"path": square(), "fills": [fill], "strokes": [{"paint": stroke}]}])), Scope::Object, FillRule::NonZero).unwrap();
    assert!(check_references(&pieces, Some(&doc)).is_ok());
    let pieces = with_swatch_colors(std::sync::Arc::new(pieces), &doc);
    let plates = |p: &Paint| match p {
        Paint::Gradient(g) => g.gradient.stops.iter().map(|s| s.plates).collect::<Vec<_>>(),
        _ => panic!("{p:?}"),
    };
    assert_eq!(plates(&pieces[0].fills.as_ref().unwrap()[0]), [Some([0.0; 4]), Some([0.1, 0.2, 0.3, 0.4])]);
    assert_eq!(plates(&pieces[0].strokes.as_ref().unwrap()[0].paint), [None, Some([0.1, 0.2, 0.3, 0.4])]);
}

/// A host check for these tests: Drop Shadow anywhere, Roughen on a piece only, nothing else.
fn shadows_only(e: &Effect, row: bool) -> std::result::Result<(), String> {
    match e.id.as_str() {
        "stylize.dropShadow" => Ok(()),
        "distort.roughen" if !row => Ok(()),
        id => Err(format!("not taken: {id}")),
    }
}

fn with_looks(pieces: Value) -> Result<Vec<Piece>> {
    decode_with(&out(pieces), Scope::Object, FillRule::NonZero, Some(shadows_only))
}

/// Without `appearanceOutput`, a piece, a fill and a stroke still have no opacity, blending mode
/// or effects: each refuses the result, naming the field.
#[test]
fn api2_and_undeclared_effects_still_refuse_piece_appearance() {
    for (piece, at) in [
        (json!({"path": square(), "opacity": 0.5}), "objects[0].opacity"),
        (json!({"path": square(), "blend": "Multiply"}), "objects[0].blend"),
        (json!({"path": square(), "effects": []}), "objects[0].effects"),
        (json!({"path": square(), "fills": [{"paint": red(), "opacity": 0.5}]}), "objects[0].fills[0]"),
        (json!({"path": square(), "strokes": [{"paint": red(), "blend": "Screen"}]}), "objects[0].strokes[0].blend"),
    ] {
        let e = decode(&out(json!([piece])), Scope::Object, FillRule::NonZero).unwrap_err().to_string();
        assert!(e.contains(&format!("{at}:")), "{at}: {e}");
    }
}

/// With `appearanceOutput`, a piece and each fill and stroke it returns take an opacity, a blending
/// mode by its identifier and effect records the host lets in; a fill carrying them is
/// `{"paint", …}`. The fills and strokes are dressed with them, with the effects after their own;
/// a piece without any compares as one from an effect that declares nothing.
#[test]
fn piece_and_row_opacity_blend_and_effects_decode_in_order() {
    let shadow = json!({"id": "stylize.dropShadow", "params": {"x": 2}});
    let pieces = with_looks(json!([
        {"path": square(), "opacity": 0.5, "blend": "Multiply", "effects": [{"id": "distort.roughen"}, shadow],
         "fills": [red(), {"paint": red(), "opacity": 0.25, "blend": "ColorBurn", "effects": [shadow]}],
         "strokes": [{"paint": red(), "width": 2, "opacity": 0, "effects": [{"id": "stylize.dropShadow", "visible": false}]}]},
        {"path": square(), "fills": [red()]},
    ]))
    .unwrap();
    let look = &pieces[0].look;
    assert_eq!((look.opacity, look.blend), (Some(0.5), Some(BlendMode::Multiply)));
    assert_eq!(look.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["distort.roughen", "stylize.dropShadow"]);
    assert_eq!(look.effects[1].params, json!({"x": 2}));
    assert_eq!(pieces[0].fill_looks.len(), 2);
    assert!(pieces[0].fill_looks[0].is_empty());
    assert_eq!((pieces[0].fill_looks[1].opacity, pieces[0].fill_looks[1].blend), (Some(0.25), Some(BlendMode::ColorBurn)));
    assert_eq!(pieces[0].stroke_looks[0].opacity, Some(0.0));
    assert!(!pieces[0].stroke_looks[0].effects[0].visible);
    let plain = decode(&out(json!([{"path": square(), "fills": [red()]}])), Scope::Object, FillRule::NonZero).unwrap();
    assert_eq!(pieces[1], plain[0], "no look");
    // Dressed: the second fill and the stroke take their looks, the stroke's effect after its own.
    let mut own = StrokeLayer::new(Paint::solid(Color::BLACK), 3.0);
    own.effects.push(Effect { id: "stylize.outerGlow".into(), params: Value::Null, visible: true });
    let base = Appearance {
        items: vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::WHITE))), AppearanceItem::Stroke(own)],
        ..Default::default()
    };
    let ap = appearance(&base, &pieces[0]);
    let fills: Vec<&FillLayer> = ap.items.iter().filter_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None }).collect();
    assert_eq!((fills[0].opacity, fills[0].blend), (1.0, BlendMode::Normal));
    assert_eq!((fills[1].opacity, fills[1].blend, fills[1].effects.len()), (0.25, BlendMode::ColorBurn, 1));
    let Some(AppearanceItem::Stroke(s)) = ap.items.iter().find(|i| !i.is_fill()) else { panic!("a stroke") };
    assert_eq!(s.opacity, 0.0);
    assert_eq!(s.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.outerGlow", "stylize.dropShadow"]);
    // On one fill: each copy takes its own fill's look.
    let copies = items(&base.items[0], &pieces[0]);
    let opacity: Vec<f32> = copies.iter().map(|i| if let AppearanceItem::Fill(f) = i { f.opacity } else { -1.0 }).collect();
    assert_eq!(opacity, [1.0, 0.25]);
}

/// Every bad look refuses the whole result, naming the field: an opacity that is not a number from
/// 0 to 1, a blending mode that is not an identifier exactly, an `effects` that is not a list, a
/// record with another key or an id the host refuses (a plug-in effect, a geometry effect on a
/// fill), and the limits of 16 records in a list and 4,096 in a result.
#[test]
fn invalid_appearance_field_refuses_every_piece_and_names_field() {
    let ok = json!({"path": square(), "opacity": 0.5});
    let shadow = json!({"id": "stylize.dropShadow"});
    for (piece, at) in [
        (json!({"path": square(), "opacity": 1.5}), "objects[1].opacity"),
        (json!({"path": square(), "opacity": -0.01}), "objects[1].opacity"),
        (json!({"path": square(), "opacity": "0.5"}), "objects[1].opacity"),
        (json!({"path": square(), "opacity": null}), "objects[1].opacity"),
        (json!({"path": square(), "blend": "multiply"}), "objects[1].blend"),
        (json!({"path": square(), "blend": "Color Burn"}), "objects[1].blend"),
        (json!({"path": square(), "blend": 2}), "objects[1].blend"),
        (json!({"path": square(), "effects": shadow}), "objects[1].effects"),
        (json!({"path": square(), "effects": [{"id": "stylize.dropShadow", "order": 1}]}), "objects[1].effects[0].order"),
        (json!({"path": square(), "effects": [{"id": ""}]}), "objects[1].effects[0]"),
        (json!({"path": square(), "effects": [{"id": "plugin.test.self"}]}), "objects[1].effects[0]"),
        (json!({"path": square(), "effects": [shadow, {"id": "stylize.nope"}]}), "objects[1].effects[1]"),
        (json!({"path": square(), "fills": [{"paint": red(), "effects": [{"id": "distort.roughen"}]}]}), "objects[1].fills[0].effects[0]"),
        (json!({"path": square(), "fills": [{"paint": red(), "tint": 1}]}), "objects[1].fills[0].tint"),
        (json!({"path": square(), "strokes": [{"paint": red(), "opacity": 2}]}), "objects[1].strokes[0].opacity"),
        (json!({"path": square(), "effects": vec![shadow.clone(); MAX_LOOK_EFFECTS + 1]}), "objects[1].effects"),
    ] {
        let e = with_looks(json!([ok, piece])).unwrap_err().to_string();
        assert!(e.contains(&format!("{at}:")), "{at}: {e}");
    }
    let e = with_looks(json!([{"path": square(), "effects": [{"id": "plugin.test.self"}]}])).unwrap_err().to_string();
    assert!(e.contains("not taken: plugin.test.self"), "{e}");
    // 256 pieces of 16 records each fit; one more record does not.
    let full = json!({"path": square(), "effects": vec![shadow.clone(); MAX_LOOK_EFFECTS]});
    let mut many = vec![full.clone(); MAX_RESULT_EFFECTS / MAX_LOOK_EFFECTS];
    assert_eq!(with_looks(Value::from(many.clone())).unwrap().len(), MAX_RESULT_EFFECTS / MAX_LOOK_EFFECTS);
    many.push(json!({"path": square(), "fills": [{"paint": red(), "effects": [shadow]}]}));
    let e = with_looks(Value::from(many)).unwrap_err().to_string();
    assert!(
        e.contains(&format!("objects[{}].fills[0].effects[0]: a result carries at most {MAX_RESULT_EFFECTS}", MAX_RESULT_EFFECTS / MAX_LOOK_EFFECTS)),
        "{e}"
    );
    // A host that has set no check lets no record in.
    let e = decode_with(&out(json!([{"path": square(), "effects": [shadow]}])), Scope::Object, FillRule::NonZero, Some(no_effects)).unwrap_err();
    assert!(e.to_string().contains("objects[0].effects[0]:"), "{e}");
}

/// An effect on a fill or a stroke sits among that fill's or stroke's own effects, so the fills
/// and strokes it returns take an opacity and a blending mode but no `effects`, even an empty
/// list; its pieces still take effects of their own.
#[test]
fn rows_of_an_effect_on_a_fill_or_stroke_take_no_effects() {
    let shadow = json!({"id": "stylize.dropShadow"});
    for (scope, piece, at) in [
        (Scope::Fill, json!({"path": square(), "fills": [{"paint": red(), "effects": [shadow]}]}), "objects[0].fills[0].effects"),
        (Scope::Fill, json!({"path": square(), "fills": [{"paint": red(), "effects": []}]}), "objects[0].fills[0].effects"),
        (Scope::Stroke, json!({"path": square(), "strokes": [{"paint": red(), "effects": [shadow]}]}), "objects[0].strokes[0].effects"),
    ] {
        let e = decode_with(&out(json!([piece])), scope, FillRule::NonZero, Some(shadows_only)).unwrap_err().to_string();
        assert!(e.contains(&format!("{at}:")), "{at}: {e}");
    }
    for (scope, piece) in [
        (Scope::Fill, json!({"path": square(), "effects": [shadow], "fills": [{"paint": red(), "opacity": 0.5, "blend": "Screen"}]})),
        (Scope::Stroke, json!({"path": square(), "effects": [shadow], "strokes": [{"paint": red(), "width": 1, "opacity": 0.5}]})),
        (Scope::Object, json!({"path": square(), "fills": [{"paint": red(), "effects": [shadow]}]})),
    ] {
        let pieces = decode_with(&out(json!([piece])), scope, FillRule::NonZero, Some(shadows_only));
        assert!(pieces.is_ok(), "{scope:?}: {pieces:?}");
    }
}
