//! Text Tweaker: the values each shape gives each character, what it skips and changes, one undo
//! step, and bad params refused without a change. Release Type on a Path ([`release`]).

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
    // `b` is 20 pt already: three characters change.
    assert_eq!(out, json!({"count": 3, "ids": [id.0]}));
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
    // Each first character is upright already.
    assert_eq!(out["count"], json!(3));
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
fn a_range_inside_a_multibyte_character_uses_character_boundaries() {
    let mut s = session();
    let id = text(&mut s, "éx");
    let out = s.execute(C, &json!({"id": id.0, "start": 1, "end": 2, "attribute": "tracking", "from": 100, "to": 300})).unwrap();
    assert_eq!(out["count"], 1);
    let tracking: Vec<f64> = styles(&s, id).iter().map(|(_, st)| st.tracking).collect();
    assert_eq!(tracking, [100.0, 0.0]);
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
fn characters_that_already_have_the_value_are_left_out() {
    let mut s = session();
    let id = text(&mut s, "abc");
    select(&mut s, &[id]);
    let p = json!({"attribute": "rotation", "from": 0, "to": 30});
    // The first character already has 0°.
    assert_eq!(s.execute(C, &p).unwrap(), json!({"count": 2, "ids": [id.0]}));
    let (before, n) = (s.doc().unwrap().doc.clone(), steps(&s));
    assert_eq!(s.execute(C, &p).unwrap(), json!({"count": 0, "ids": []}), "again: nothing to change");
    assert_eq!((s.doc().unwrap().doc.clone(), steps(&s)), (before, n), "no step");
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
        json!({"id": id.0, "start": -1}),
        json!({"id": id.0, "start": 1, "end": 1.5}),
        json!({"id": id.0, "end": "2"}),
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

/// Release Type on a Path (#5532): each mode, Keep Original, mixed and empty selections, closed,
/// open, curved and flipped paths, transformed and grouped type, multi-style and unicode text,
/// undo and redo, the preview a dialog shows, and bad params.
mod release {
    use serde_json::{Value, json};
    use vectorcraft_color::Paint;
    use vectorcraft_doc::{Appearance, Document, Justify, NodeId, NodeKind, TextKind, TextObject};
    use vectorcraft_geom::{PathData, Point};

    use crate::Session;

    const C: &str = "addon.type.releasePath";

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        s
    }

    fn new_id(v: &Value) -> NodeId {
        NodeId(v["id"].as_u64().unwrap())
    }

    /// 12 pt type `words` on a path along `d` (SVG path data), starting where the path is nearest
    /// `at`, selected.
    fn path_type(s: &mut Session, d: &str, at: [f64; 2], words: &str) -> NodeId {
        let pid = s.execute("path.create", &json!({ "d": d })).unwrap()["id"].as_u64().unwrap();
        let id = new_id(&s.execute("text.createInPath", &json!({"path": pid, "mode": "onPath", "text": words, "size": 12, "at": at})).unwrap());
        select(s, &[id]);
        id
    }

    fn rect(s: &mut Session, x: f64) -> NodeId {
        new_id(&s.execute("shape.rectangle", &json!({"x": x, "y": 20, "width": 20, "height": 20})).unwrap())
    }

    fn point_type(s: &mut Session) -> NodeId {
        new_id(&s.execute("text.create", &json!({"x": 40, "y": 500, "text": "point", "size": 12})).unwrap())
    }

    fn select(s: &mut Session, ids: &[NodeId]) {
        s.select(|_, sel| sel.set(ids.iter().copied())).unwrap();
    }

    fn doc(s: &Session) -> Document {
        (*s.doc().unwrap().doc).clone()
    }

    fn steps(s: &Session) -> usize {
        s.doc().unwrap().history.undo.len()
    }

    fn selected(s: &Session) -> Vec<NodeId> {
        s.doc().unwrap().selection.objects.clone()
    }

    fn text(s: &Session, id: NodeId) -> TextObject {
        match &s.doc().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Text(t) => (**t).clone(),
            k => panic!("not text: {k:?}"),
        }
    }

    fn path(s: &Session, id: NodeId) -> PathData {
        match &s.doc().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Path { path, .. } => path.clone(),
            k => panic!("not a path: {k:?}"),
        }
    }

    /// (parent, index) of `id`.
    fn place(s: &Session, id: NodeId) -> (Option<NodeId>, usize) {
        let (parent, idx, _) = s.doc().unwrap().doc.position(id).unwrap();
        (parent, idx)
    }

    /// Each glyph's pen position in the document and its baseline angle, in text order.
    fn pens(t: &TextObject) -> Vec<(Point, f64)> {
        let mut g: Vec<_> =
            vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).glyphs.into_iter().map(|g| (g.byte, t.xf * g.origin, g.angle)).collect();
        g.sort_by_key(|g| g.0);
        g.into_iter().map(|(_, p, a)| (p, a)).collect()
    }

    /// How many path objects the document has.
    fn paths_in(s: &Session) -> usize {
        let mut n = 0;
        for l in &s.doc().unwrap().doc.layers {
            l.walk(&mut |c| n += usize::from(matches!(c.kind, NodeKind::Path { .. })));
        }
        n
    }

    fn close(a: Point, b: Point) -> bool {
        (a - b).hypot() < 1e-4
    }

    fn ids(v: &Value, key: &str) -> Vec<NodeId> {
        v[key].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
    }

    /// `after`, released from `before`: point type with the same text and attributes, on one
    /// straight line whose first character starts where `before`'s did on its path, with no Type
    /// on a Path Options left.
    fn assert_released(before: &TextObject, after: &TextObject) {
        assert_eq!(after.kind, TextKind::Point);
        assert_eq!(after.runs, before.runs, "the same text and character attributes");
        assert_eq!((&after.para, &after.paras), (&before.para, &before.paras), "the same paragraph attributes");
        assert_eq!(after.plain_text(), before.plain_text());
        assert_eq!((after.path_effect, after.path_align, after.path_spacing), Default::default(), "path options dropped");
        let (was, now) = (pens(before), pens(after));
        assert!(!now.is_empty());
        if let Some((first, _)) = was.first() {
            assert!(close(now[0].0, *first), "the baseline starts where the first character sat: {:?} vs {first:?}", now[0].0);
        }
        assert!(now.iter().all(|(_, a)| *a == 0.0), "a straight baseline");
        // On one line through the first pen, along the object's own x axis.
        let [a, b, ..] = after.xf.as_coeffs();
        let axis = vectorcraft_geom::Vec2::new(a, b);
        for (p, _) in &now {
            let off = *p - now[0].0;
            assert!(off.cross(axis).abs() < 1e-4 * axis.hypot().max(1.0) * off.hypot().max(1.0), "{p:?} is on the line");
        }
    }

    #[test]
    fn both_gives_point_type_and_the_path_in_the_types_place() {
        let mut s = session();
        let below = rect(&mut s, 10.0);
        let id = path_type(&mut s, "M100 300 C200 150 400 150 500 300", [100.0, 300.0], "Curved line");
        let above = rect(&mut s, 50.0);
        select(&mut s, &[id]);
        let before = text(&s, id);
        let (layer, idx) = place(&s, id);
        let (start, n) = (doc(&s), steps(&s));

        let out = s.execute(C, &json!({})).unwrap();
        assert_eq!(steps(&s), n + 1, "one undo step");
        let (paths, texts) = (ids(&out, "paths"), ids(&out, "texts"));
        assert_eq!((paths.len(), texts.len()), (1, 1));
        let (p, t) = (paths[0], texts[0]);
        assert_eq!(ids(&out, "ids"), [p, t], "bottom to top");
        assert_eq!(selected(&s), [p, t], "the released objects are selected");

        // The path: its exact shape, anchors and handles, with no fill and no stroke, where the type was.
        assert_eq!(path(&s, p), before.type_path().unwrap());
        assert_eq!(*s.doc().unwrap().doc.node(p).unwrap().appearance, Appearance::basic(Paint::None, Paint::None, 1.0));
        assert_eq!(
            (place(&s, below), place(&s, p), place(&s, t), place(&s, above)),
            ((layer, idx - 1), (layer, idx), (layer, idx + 1), (layer, idx + 2))
        );
        // The text replaces the type on a path.
        assert_released(&before, &text(&s, t));
        assert_eq!(t, id, "the same type object, now point type");

        let released = doc(&s);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(doc(&s), start, "undone");
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(doc(&s), released, "redone");
    }

    #[test]
    fn path_only_and_text_only() {
        let mut s = session();
        let id = path_type(&mut s, "M100 300 L500 300", [300.0, 300.0], "Line");
        let before = text(&s, id);
        let (layer, idx) = place(&s, id);
        let out = s.execute(C, &json!({"mode": "path"})).unwrap();
        let p = ids(&out, "paths")[0];
        assert!(ids(&out, "texts").is_empty());
        assert!(s.doc().unwrap().doc.node(id).is_none(), "the type is gone");
        assert_eq!((path(&s, p), place(&s, p)), (before.type_path().unwrap(), (layer, idx)));
        assert_eq!(selected(&s), [p]);

        s.execute("edit.undo", &json!({})).unwrap();
        select(&mut s, &[id]);
        let out = s.execute(C, &json!({"mode": "text"})).unwrap();
        assert!(ids(&out, "paths").is_empty());
        assert_eq!((ids(&out, "texts"), place(&s, id)), (vec![id], (layer, idx)), "the type stays in its place");
        assert_released(&before, &text(&s, id));
        assert_eq!(paths_in(&s), 0, "no path made");
    }

    #[test]
    fn keep_original_adds_the_released_objects_above_it() {
        let mut s = session();
        let id = path_type(&mut s, "M100 300 L500 300", [100.0, 300.0], "Keep");
        let (before, (layer, idx)) = (text(&s, id), place(&s, id));
        let original = s.doc().unwrap().doc.node(id).cloned().unwrap();
        let out = s.execute(C, &json!({"keepOriginal": true})).unwrap();
        let (p, t) = (ids(&out, "paths")[0], ids(&out, "texts")[0]);
        assert_eq!(s.doc().unwrap().doc.node(id), Some(&original), "the type on a path stays as it is");
        assert_eq!((place(&s, id), place(&s, p), place(&s, t)), ((layer, idx), (layer, idx + 1), (layer, idx + 2)));
        assert_ne!(t, id);
        assert_eq!(selected(&s), [p, t], "only the released objects are selected");
        assert_released(&before, &text(&s, t));

        // Text only, kept: the copy goes just above.
        select(&mut s, &[id]);
        let out = s.execute(C, &json!({"mode": "text", "keepOriginal": true})).unwrap();
        let t2 = ids(&out, "texts")[0];
        assert_eq!(place(&s, t2), (layer, idx + 1));
        assert_released(&before, &text(&s, t2));
    }

    #[test]
    fn other_objects_are_left_alone() {
        let mut s = session();
        let r = rect(&mut s, 10.0);
        let pt = point_type(&mut s);
        let a = path_type(&mut s, "M100 300 L500 300", [100.0, 300.0], "One");
        let b = path_type(&mut s, "M100 400 L500 400", [100.0, 400.0], "Two");
        let others = [r, pt].map(|id| s.doc().unwrap().doc.node(id).cloned().unwrap());
        select(&mut s, &[r, pt, a, b]);
        let n = steps(&s);
        let out = s.execute(C, &json!({})).unwrap();
        assert_eq!(steps(&s), n + 1, "one undo step for the whole selection");
        assert_eq!((ids(&out, "paths").len(), ids(&out, "texts")), (2, vec![a, b]));
        assert_eq!([r, pt].map(|id| s.doc().unwrap().doc.node(id).cloned().unwrap()), others);
        assert_eq!(selected(&s).len(), 4, "the two paths and the two texts");
        assert!(!selected(&s).contains(&r) && !selected(&s).contains(&pt));

        // `ids` picks the objects instead of the selection.
        s.execute("edit.undo", &json!({})).unwrap();
        select(&mut s, &[r]);
        let out = s.execute(C, &json!({"ids": [r.0, b.0]})).unwrap();
        assert_eq!(ids(&out, "texts"), [b]);
        assert!(matches!(text(&s, a).kind, TextKind::OnPath { .. }));
    }

    #[test]
    fn nothing_to_release_changes_nothing() {
        let mut s = session();
        let r = rect(&mut s, 10.0);
        let pt = point_type(&mut s);
        select(&mut s, &[r, pt]);
        let (before, n, sel) = (doc(&s), steps(&s), selected(&s));
        assert_eq!(s.execute(C, &json!({})).unwrap(), json!({"ids": [], "paths": [], "texts": []}));
        assert_eq!((doc(&s), steps(&s), selected(&s)), (before, n, sel), "no change, no step");
    }

    #[test]
    fn closed_and_open_paths_keep_their_shape() {
        for (d, closed) in [
            ("M200 200 L400 200 L400 400 L200 400 Z", true),
            ("M100 300 C150 100 250 500 300 300 S450 100 500 300", false),
            ("M300 150 C380 150 450 220 450 300 C450 380 380 450 300 450 C220 450 150 380 150 300 C150 220 220 150 300 150 Z", true),
        ] {
            let mut s = session();
            let id = path_type(&mut s, d, [200.0, 200.0], "Round and round");
            let before = text(&s, id);
            let out = s.execute(C, &json!({})).unwrap();
            let p = path(&s, ids(&out, "paths")[0]);
            assert_eq!(p, before.type_path().unwrap(), "{d}");
            assert_eq!(p.is_closed(), closed, "{d}");
            assert_released(&before, &text(&s, ids(&out, "texts")[0]));
        }
    }

    #[test]
    fn flipped_type_releases_its_path_as_it_runs() {
        let mut s = session();
        let id = path_type(&mut s, "M100 300 C200 150 400 150 500 300", [300.0, 200.0], "Flipped");
        s.execute("type.pathOptions", &json!({"flip": true, "effect": "skew", "alignToPath": "center", "spacing": 4})).unwrap();
        let before = text(&s, id);
        let out = s.execute(C, &json!({})).unwrap();
        assert_eq!(path(&s, ids(&out, "paths")[0]), before.type_path().unwrap());
        assert_released(&before, &text(&s, id));
    }

    #[test]
    fn transformed_type_keeps_its_transform() {
        let mut s = session();
        let id = path_type(&mut s, "M100 300 C200 150 400 150 500 300", [100.0, 300.0], "Turned");
        s.execute("object.rotate", &json!({"angle": 30, "origin": [300, 300]})).unwrap();
        s.execute("object.scale", &json!({"sx": 150, "sy": 150, "origin": [300, 300]})).unwrap();
        let before = text(&s, id);
        let out = s.execute(C, &json!({})).unwrap();
        assert_eq!(path(&s, ids(&out, "paths")[0]), before.type_path().unwrap(), "the transform is in the path");
        let after = text(&s, id);
        assert_eq!(after.xf.as_coeffs()[..4], before.xf.as_coeffs()[..4], "turned and scaled as the type was");
        assert_released(&before, &after);
    }

    #[test]
    fn grouped_type_releases_inside_its_group() {
        let mut s = session();
        let r = rect(&mut s, 10.0);
        let id = path_type(&mut s, "M100 300 L500 300", [100.0, 300.0], "Grouped");
        select(&mut s, &[r, id]);
        s.execute("object.group", &json!({})).unwrap();
        let g = place(&s, id).0.unwrap();
        assert!(matches!(s.doc().unwrap().doc.node(g).unwrap().kind, NodeKind::Group { .. }));
        select(&mut s, &[g]);
        let out = s.execute(C, &json!({})).unwrap();
        let (p, t) = (ids(&out, "paths")[0], ids(&out, "texts")[0]);
        assert_eq!((place(&s, p).0, place(&s, t).0), (Some(g), Some(g)));
    }

    #[test]
    fn styles_and_unicode_text_survive() {
        let mut s = session();
        let words = "Größe ✓ 日本語 éa";
        let id = path_type(&mut s, "M100 300 C200 150 400 150 500 300", [100.0, 300.0], words);
        // "Gr" (bytes 0..2), and "日本" (bytes 12..18, after "Größe ✓ ").
        s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 2, "size": 24, "fill": "#ff0000"})).unwrap();
        s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 12, "end": 18, "tracking": 200, "hScale": 150})).unwrap();
        // A centred paragraph: the first character still starts where it sat.
        s.edit("centre", |d, _| {
            if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
                t.para.justify = Justify::Center;
            }
            Ok(())
        })
        .unwrap();
        let before = text(&s, id);
        assert!(before.runs.len() >= 3, "several styles: {:?}", before.runs);
        let out = s.execute(C, &json!({})).unwrap();
        let after = text(&s, ids(&out, "texts")[0]);
        assert_eq!(after.plain_text(), words);
        assert_released(&before, &after);
    }

    #[test]
    fn empty_type_starts_at_its_start_bracket() {
        let mut s = session();
        let id = path_type(&mut s, "M100 300 L500 300", [300.0, 300.0], "");
        let out = s.execute(C, &json!({})).unwrap();
        assert_eq!(ids(&out, "texts"), [id]);
        let t = text(&s, id);
        assert_eq!(t.kind, TextKind::Point);
        assert!(close(t.xf * Point::ZERO, Point::new(300.0, 300.0)), "{:?}", t.xf * Point::ZERO);
    }

    /// What a dialog's Preview does: each preview replaces the last, Cancel puts the type back with
    /// no step of history, and OK keeps the preview as one step.
    #[test]
    fn previews_cancel_and_commit() {
        let mut s = session();
        let id = path_type(&mut s, "M100 300 L500 300", [100.0, 300.0], "Preview");
        let (start, n, sel) = (doc(&s), steps(&s), selected(&s));
        let count = paths_in;
        s.begin_interaction("Release Type on a Path").unwrap();
        s.preview(C, &json!({"mode": "both"})).unwrap();
        assert_eq!((count(&s), text(&s, id).kind), (1, TextKind::Point));
        s.preview(C, &json!({"mode": "text"})).unwrap();
        assert_eq!(count(&s), 0, "the next preview replaces the last");
        s.preview(C, &json!({"mode": "path", "keepOriginal": true})).unwrap();
        assert_eq!(count(&s), 1);
        assert!(matches!(text(&s, id).kind, TextKind::OnPath { .. }));
        s.cancel_interaction().unwrap();
        assert_eq!((doc(&s), steps(&s), selected(&s)), (start.clone(), n, sel), "Cancel: as it was, no step");

        s.begin_interaction("Release Type on a Path").unwrap();
        s.preview(C, &json!({"mode": "both"})).unwrap();
        s.commit_interaction().unwrap();
        assert_eq!(steps(&s), n + 1, "OK: one step");
        assert_eq!(text(&s, id).kind, TextKind::Point);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(doc(&s), start);
    }

    #[test]
    fn bad_params_are_refused_without_a_change() {
        let mut s = session();
        path_type(&mut s, "M100 300 L500 300", [100.0, 300.0], "Bad");
        let (before, n) = (doc(&s), steps(&s));
        for p in [
            json!({"mode": "all"}),
            json!({"mode": 1}),
            json!({"keepOriginal": "yes"}),
            json!({"ids": [999_999]}),
            json!({"ids": "1"}),
            json!({"id": -1}),
            json!({"colour": "red"}),
        ] {
            assert!(s.execute(C, &p).is_err(), "{p}");
            assert_eq!((doc(&s), steps(&s)), (before.clone(), n), "{p}");
        }
    }
}
