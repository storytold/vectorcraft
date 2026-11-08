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
    let piece =
        |fills: Option<Vec<Paint>>, strokes: Option<Vec<OutStroke>>| Piece { path: PathData::default(), rule: FillRule::NonZero, fills, strokes };
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
