//! Text Tweaker: the values each shape gives each character, what it skips and changes, one undo
//! step, and bad params refused without a change.

use serde_json::json;
use vectorcraft_doc::{CharStyle, NodeId, NodeKind, TextObject};

use super::amount;
use crate::Session;

const C: &str = "addon.type.textTweaker";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn text(s: &mut Session, words: &str) -> NodeId {
    NodeId(s.execute("text.create", &json!({"x": 40, "y": 100, "text": words, "size": 20})).unwrap()["id"].as_u64().unwrap())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.select(|_, sel| sel.set(ids.iter().copied())).unwrap();
}

fn obj(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

/// Each character of `id` with its style.
fn styles(s: &Session, id: NodeId) -> Vec<(char, CharStyle)> {
    obj(s, id).runs.iter().flat_map(|r| r.text.chars().map(|c| (c, r.style.clone())).collect::<Vec<_>>()).collect()
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn near(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn the_shapes_run_from_the_start_value_to_the_end_value() {
    let at = |shape, n| (0..n).map(|i| amount(shape, i, n, 0.5)).collect::<Vec<_>>();
    assert!(near(&at("straight", 5), &[0.0, 0.25, 0.5, 0.75, 1.0]));
    assert!(near(&at("ease", 5), &[0.0, 0.15625, 0.5, 0.84375, 1.0]), "slow at both ends");
    let bow = at("bow", 5);
    assert!(near(&[bow[0], bow[2], bow[4]], &[0.0, 1.0, 0.0]) && (bow[1] - 0.5f64.sqrt()).abs() < 1e-9, "a smooth arc");
    assert!(near(&at("spike", 5), &[0.0, 0.5, 1.0, 0.5, 0.0]), "straight up and down");
    assert!(near(&at("random", 3), &[0.5; 3]));
    assert!(near(&at("straight", 1), &[0.0]), "one character takes the start value");
}

#[test]
fn a_straight_ramp_sizes_each_letter_and_skips_spaces() {
    let mut s = session();
    let id = text(&mut s, "ab cd");
    select(&mut s, &[id]);
    let n = steps(&s);
    let out = s.execute(C, &json!({"attribute": "size", "from": 10, "to": 40})).unwrap();
    assert_eq!(out, json!({"count": 4, "ids": [id.0]}));
    assert_eq!(steps(&s), n + 1, "one undo step");
    let sizes: Vec<(char, f64)> = styles(&s, id).into_iter().map(|(c, st)| (c, st.size)).collect();
    assert_eq!(sizes, [('a', 10.0), ('b', 20.0), (' ', 20.0), ('c', 30.0), ('d', 40.0)], "the space keeps its size");
    assert_eq!(obj(&s, id).plain_text(), "ab cd", "the text stays live");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(styles(&s, id).iter().all(|(_, st)| st.size == 20.0), "undone");
}

#[test]
fn spaces_can_count_too() {
    let mut s = session();
    let id = text(&mut s, "a b");
    select(&mut s, &[id]);
    s.execute(C, &json!({"attribute": "baselineShift", "from": 0, "to": 10, "skipSpaces": false})).unwrap();
    let shifts: Vec<f64> = styles(&s, id).iter().map(|(_, st)| st.baseline_shift).collect();
    assert_eq!(shifts, [0.0, 5.0, 10.0]);
}

#[test]
fn every_attribute_is_set_as_the_character_panel_sets_it() {
    let mut s = session();
    for (attribute, get) in [
        ("size", (|st: &CharStyle| st.size) as fn(&CharStyle) -> f64),
        ("baselineShift", |st| st.baseline_shift),
        ("rotation", |st| st.rotation),
        ("hScale", |st| st.h_scale),
        ("vScale", |st| st.v_scale),
        ("tracking", |st| st.tracking),
        ("leading", |st| st.leading.unwrap_or(-1.0)),
        ("strokeWidth", |st| st.stroke_width),
    ] {
        let id = text(&mut s, "xyz");
        select(&mut s, &[id]);
        s.execute(C, &json!({"attribute": attribute, "from": 2, "to": 6})).unwrap();
        let got: Vec<f64> = styles(&s, id).iter().map(|(_, st)| get(st)).collect();
        assert!(near(&got, &[2.0, 4.0, 6.0]), "{attribute}: {got:?}");
    }
}

#[test]
fn each_object_gets_its_own_ramp() {
    let mut s = session();
    let (a, b) = (text(&mut s, "abc"), text(&mut s, "de"));
    select(&mut s, &[a, b]);
    let out = s.execute(C, &json!({"attribute": "rotation", "from": 0, "to": 30})).unwrap();
    assert_eq!(out["count"], json!(5));
    let turns = |id| styles(&s, id).iter().map(|(_, st)| st.rotation).collect::<Vec<_>>();
    assert_eq!((turns(a), turns(b)), (vec![0.0, 15.0, 30.0], vec![0.0, 30.0]));
}

#[test]
fn a_range_changes_only_its_characters() {
    let mut s = session();
    let id = text(&mut s, "abcde");
    s.execute(C, &json!({"id": id.0, "start": 1, "end": 4, "attribute": "tracking", "from": 100, "to": 300})).unwrap();
    let tracking: Vec<f64> = styles(&s, id).iter().map(|(_, st)| st.tracking).collect();
    assert_eq!(tracking, [0.0, 100.0, 200.0, 300.0, 0.0]);
}

#[test]
fn random_values_stay_in_range_and_repeat_with_the_seed() {
    let mut s = session();
    let id = text(&mut s, "randomness");
    select(&mut s, &[id]);
    let run = |s: &mut Session, seed| {
        s.execute(C, &json!({"attribute": "vScale", "from": 50, "to": 150, "shape": "random", "seed": seed})).unwrap();
        styles(s, id).iter().map(|(_, st)| st.v_scale).collect::<Vec<_>>()
    };
    let first = run(&mut s, 7);
    assert!(first.iter().all(|v| (50.0..=150.0).contains(v)), "{first:?}");
    assert!(first.windows(2).any(|w| w[0] != w[1]), "values vary");
    assert_eq!(run(&mut s, 7), first, "same seed, same values");
    assert_ne!(run(&mut s, 8), first, "another seed, other values");
}

#[test]
fn values_follow_the_text_transform_as_the_character_panel_does() {
    let mut s = session();
    let id = text(&mut s, "ab");
    select(&mut s, &[id]);
    s.execute("object.scale", &json!({"sx": 200, "sy": 200})).unwrap();
    s.execute(C, &json!({"attribute": "size", "from": 20, "to": 40})).unwrap();
    let sizes: Vec<f64> = styles(&s, id).iter().map(|(_, st)| st.size).collect();
    // Shown sizes are the asked ones: stored sizes are half, the text being scaled twice.
    let scale = obj(&s, id).style_scale().unwrap().points;
    assert!(near(&sizes.iter().map(|v| v * scale).collect::<Vec<_>>(), &[20.0, 40.0]), "{sizes:?} at {scale}");
}

#[test]
fn empty_text_and_only_spaces_change_nothing() {
    let mut s = session();
    let id = text(&mut s, "   ");
    select(&mut s, &[id]);
    let n = steps(&s);
    assert_eq!(s.execute(C, &json!({})).unwrap(), json!({"count": 0, "ids": []}));
    assert_eq!(steps(&s), n, "no step");
}

#[test]
fn bad_params_are_refused_without_a_change() {
    let mut s = session();
    let id = text(&mut s, "abc");
    select(&mut s, &[id]);
    let (before, n) = (s.doc().unwrap().doc.clone(), steps(&s));
    for p in [
        json!({"attribute": "colour"}),
        json!({"attribute": 3}),
        json!({"shape": "zigzag"}),
        json!({"attribute": "size", "from": 0}),
        json!({"attribute": "size", "to": 5000}),
        json!({"attribute": "hScale", "from": -1}),
        json!({"from": "big"}),
        json!({"to": f64::MAX}),
        json!({"seed": -1}),
        json!({"skipSpaces": "yes"}),
        json!({"ids": [999_999]}),
    ] {
        assert!(s.execute(C, &p).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().doc, before, "{p}");
        assert_eq!(steps(&s), n, "{p}");
    }
}

#[test]
fn it_needs_a_selection() {
    let mut s = session();
    text(&mut s, "abc");
    select(&mut s, &[]);
    assert!(s.execute(C, &json!({})).is_err());
}
