//! Fill/Stroke proxy commands: invert, complement, last colour/gradient, swap/default on type and
//! the Session's recent colours.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::NodeKind;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn rect(s: &mut Session) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn hexes(s: &Session) -> Vec<String> {
    s.recent_colors.iter().map(Color::to_hex).collect()
}

#[test]
fn complement_of_red_on_the_fill_only_gives_cyan() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let r = s.execute("paint.complement", &json!({})).unwrap();
    assert_eq!(r["changed"], 1);
    let n = node(&s, id);
    assert_eq!(n.appearance.fill_paint().color().unwrap().to_hex(), "#00ffff");
    assert_eq!(n.appearance.stroke_paint().color().unwrap().to_hex(), "#ff0000");
    // One undo step, and the new colour is remembered.
    assert_eq!(s.last_solid.to_hex(), "#00ffff");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id).appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");
    // `stroke: true` picks the stroke whatever proxy is in front.
    s.execute("paint.complement", &json!({"stroke": true})).unwrap();
    assert_eq!(node(&s, id).appearance.stroke_paint().color().unwrap().to_hex(), "#00ffff");
}

#[test]
fn invert_keeps_cmyk_and_acts_on_defaults_without_selection() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("paint.setFill", &json!({"color": {"c": 0.0, "m": 1.0, "y": 1.0, "k": 0.0}})).unwrap();
    s.execute("paint.invert", &json!({})).unwrap();
    let c = node(&s, id).appearance.fill_paint().color().unwrap();
    assert!(matches!(c, Color::Cmyk { .. }), "{c:?}");
    assert_ne!(c, Color::cmyk(0.0, 1.0, 1.0, 0.0));
    // A CMYK complement stays CMYK too (K kept).
    s.execute("paint.setFill", &json!({"color": {"c": 0.0, "m": 1.0, "y": 1.0, "k": 0.25}})).unwrap();
    s.execute("paint.complement", &json!({})).unwrap();
    assert_eq!(node(&s, id).appearance.fill_paint().color(), Some(Color::cmyk(1.0, 0.0, 0.0, 0.25)));
    // Nothing selected: the default fill changes and the document doesn't.
    s.execute("select.none", &json!({})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    s.paint.fill = Paint::solid(Color::rgb(1.0, 1.0, 0.0));
    s.execute("paint.invert", &json!({})).unwrap();
    assert_eq!(s.paint.fill.color().unwrap().to_hex(), "#0000ff");
    assert_eq!(s.doc().unwrap().doc, before);
}

#[test]
fn last_color_and_gradient_are_reapplied() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute(
        "paint.setFill",
        &json!({"gradient": {"kind": "radial", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
    )
    .unwrap();
    s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    assert_eq!(node(&s, id).appearance.fill_paint().color().unwrap().to_hex(), "#336699");
    // `.` brings back the gradient after a solid colour, `,` the colour after the gradient.
    s.execute("paint.lastGradient", &json!({})).unwrap();
    let Paint::Gradient(g) = node(&s, id).appearance.fill_paint() else { panic!("not a gradient") };
    assert_eq!(g.gradient.stops[0].color.to_hex(), "#ff0000");
    assert!(g.geom.is_none(), "fitted to the object");
    s.execute("paint.lastColor", &json!({})).unwrap();
    assert_eq!(node(&s, id).appearance.fill_paint().color().unwrap().to_hex(), "#336699");
    // On the stroke proxy they paint the stroke and focus it.
    s.execute("paint.lastColor", &json!({"stroke": true})).unwrap();
    assert_eq!(node(&s, id).appearance.stroke_paint().color().unwrap().to_hex(), "#336699");
    assert!(!s.fill_active);
    let r = s.execute("paint.recent", &json!({})).unwrap();
    assert_eq!(r["lastColor"]["hex"], "#336699");
    assert_eq!(r["lastGradient"]["gradient"]["kind"], "Radial");
}

#[test]
fn swap_and_default_work_on_type() {
    let mut s = session();
    let r = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let style = |s: &Session| match &node(s, id).kind {
        NodeKind::Text(t) => t.first_style(),
        _ => panic!("not text"),
    };
    s.execute("paint.swap", &json!({})).unwrap();
    let st = style(&s);
    assert_eq!(st.fill, Paint::None);
    assert_eq!(st.stroke.color().unwrap().to_hex(), "#ff0000");
    assert!(st.stroke_width > 0.0, "a painted stroke is visible");
    s.execute("paint.default", &json!({})).unwrap();
    let st = style(&s);
    assert_eq!(st.fill, Paint::solid(Color::BLACK));
    assert_eq!(st.stroke, Paint::None);
}

/// How many steps there are to undo and to redo, and whether the document has unsaved changes.
fn history(s: &Session) -> (usize, usize, bool) {
    let st = s.doc().unwrap();
    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
}

/// Select `ids` with a move of them left to redo, and mark the document saved.
fn select_with_a_step_to_redo(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
}

/// Default Fill and Stroke on the default fill and stroke, None on a stroke of None, Invert and
/// Complement on that stroke's Appearance row, Swap Fill and Stroke with the same fill and stroke,
/// and Apply Last Color and Apply Last Gradient on paint that is the last color or gradient already
/// record no undo step: the step to redo stays and the document stays unmodified.
#[test]
fn proxy_commands_that_change_nothing_record_no_undo_step() {
    let mut s = session();
    let id = rect(&mut s);
    let no_step = |s: &mut Session, calls: &[(&str, serde_json::Value)]| {
        select_with_a_step_to_redo(s, &[id]);
        let before = history(s);
        for (cmd, p) in calls {
            s.execute(cmd, p).unwrap();
            assert_eq!(history(s), before, "{cmd} {p}");
        }
    };
    no_step(&mut s, &[("paint.default", json!({}))]);
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    no_step(&mut s, &[("paint.none", json!({}))]);
    // The stroke's Appearance row (item 1) has no color to invert or complement.
    let row = json!({"item": 1, "stroke": true});
    no_step(&mut s, &[("paint.invert", row.clone()), ("paint.complement", row)]);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    no_step(&mut s, &[("paint.swap", json!({})), ("paint.lastColor", json!({})), ("paint.lastColor", json!({"stroke": false}))]);
    s.execute(
        "paint.setFill",
        &json!({"gradient": {"kind": "radial", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
    )
    .unwrap();
    no_step(&mut s, &[("paint.lastGradient", json!({"stroke": false}))]);
    // Swapping a gradient fill and a red stroke is one step.
    let before = history(&s);
    s.execute("paint.swap", &json!({})).unwrap();
    assert_eq!(history(&s), (before.0 + 1, 0, true));
}

#[test]
fn recent_colors_are_fed_by_every_paint_command() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#00ff00"})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(hexes(&s)[..2], ["#ff0000", "#00ff00"]);
    // The eyedropper's colour sample goes through the same path.
    s.execute("paint.sampleColor", &json!({"color": "#0000ff", "stroke": false})).unwrap();
    assert_eq!(hexes(&s)[0], "#0000ff");
    // None and gradients don't add colours.
    s.execute("paint.none", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    assert_eq!(hexes(&s).len(), 3);
    // The list holds the newest `RECENT_MAX` colours.
    for i in 0..20u8 {
        s.execute("paint.setFill", &json!({"color": Color::rgb8(i, 0, 0).to_hex()})).unwrap();
    }
    assert_eq!(s.recent_colors.len(), Session::RECENT_MAX);
    assert_eq!(s.recent_colors[0].to_hex(), "#130000");
    let r = s.execute("paint.recent", &json!({})).unwrap();
    assert_eq!(r["colors"][0]["hex"], "#130000");
    let _ = id;
}

#[test]
fn live_previews_remember_only_the_committed_colour() {
    let mut s = session();
    rect(&mut s);
    s.begin_interaction("Color").unwrap();
    for hex in ["#100000", "#200000", "#300000"] {
        s.preview("paint.setFill", &json!({"color": hex})).unwrap();
    }
    assert!(s.recent_colors.is_empty(), "nothing is remembered while dragging");
    s.commit_interaction().unwrap();
    assert_eq!(hexes(&s), ["#300000"]);
    s.begin_interaction("Color").unwrap();
    s.preview("paint.setFill", &json!({"color": "#400000"})).unwrap();
    s.cancel_interaction().unwrap();
    assert_eq!(hexes(&s), ["#300000"], "a cancelled preview is forgotten");
    s.begin_interaction("Color").unwrap();
    s.preview("paint.setFill", &json!({"color": "#500000"})).unwrap();
    assert!(s.preview("no.such.command", &json!({})).is_err());
    s.commit_interaction().unwrap();
    assert_eq!(hexes(&s), ["#300000"], "a preview undone by a failed one is forgotten");
}

#[test]
fn focus_false_keeps_the_active_proxy() {
    let mut s = session();
    let id = rect(&mut s);
    assert!(s.fill_active);
    s.execute("paint.setStroke", &json!({"color": "#ff0000", "focus": false})).unwrap();
    assert!(s.fill_active);
    assert_eq!(node(&s, id).appearance.stroke_paint().color().unwrap().to_hex(), "#ff0000");
}

/// Toggle Fill/Stroke Focus, Invert, Complement, Apply Last Color, Apply Last Gradient,
/// `paint.setFill` and `paint.setStroke` refuse a `fill`, `stroke`, `focus` or `none` given as text or
/// as another value that is not true or false. The object, the defaults for new art, the proxy in
/// front and the undo history stay as they were, with the object selected and with nothing selected.
/// Null is read as the key left out.
#[test]
fn proxy_commands_refuse_a_boolean_given_as_text() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let calls = [
        ("paint.toggleActive", json!({"fill": "false"}), "fill"),
        ("paint.invert", json!({"stroke": "true"}), "stroke"),
        ("paint.complement", json!({"stroke": 1}), "stroke"),
        ("paint.lastColor", json!({"stroke": "true"}), "stroke"),
        ("paint.lastGradient", json!({"stroke": [true]}), "stroke"),
        ("paint.setStroke", json!({"color": "#0000ff", "focus": "false"}), "focus"),
        ("paint.setFill", json!({"none": "true"}), "none"),
        ("paint.setFill", json!({"color": "#0000ff", "none": "false"}), "none"),
    ];
    let state = |s: &Session| (node(s, id), s.paint.clone(), s.fill_active, history(s));
    for selected in [true, false] {
        if !selected {
            s.execute("select.none", &json!({})).unwrap();
        }
        let before = state(&s);
        for (cmd, p, key) in &calls {
            let r = s.execute(cmd, p);
            let want = format!("`{key}` must be true or false");
            assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
            assert_eq!(state(&s), before, "{cmd} {p}, selected: {selected}");
        }
    }
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let r = s.execute("paint.toggleActive", &json!({"fill": null})).unwrap();
    assert_eq!(r["fillActive"], json!(false), "a null `fill` swaps the proxy in front");
    s.execute("paint.setFill", &json!({"color": "#0000ff", "none": null, "focus": null})).unwrap();
    assert_eq!((node(&s, id).appearance.fill_paint().color().unwrap().to_hex(), s.fill_active), ("#0000ff".to_string(), true));
}

/// `appearance.setItem`, `livePaint.fill`, `livePaint.strokeEdge`, a `shapeBuilder.merge` `fill`
/// object, `swatch.new` and `swatch.edit`'s `paint` refuse a `none` given as text or as another value
/// that is not true or false. The document, the defaults for new art and the undo history stay as
/// they were. Null is read as `none` left out.
#[test]
fn the_commands_that_take_a_paint_refuse_a_none_given_as_text() {
    let mut s = session();
    let id = rect(&mut s);
    let name = s.execute("swatch.new", &json!({"color": "#00ff00"})).unwrap()["name"].clone();
    let refused = |r: &Result<Value>| matches!(r, Err(EngineError::BadParams { msg, .. }) if msg == "`none` must be true or false");
    let state = |s: &Session| (s.doc().unwrap().doc.clone(), s.paint.clone(), history(s));
    let before = state(&s);
    let calls = [
        ("appearance.setItem", json!({"index": 0, "ids": [id.0], "none": "true"})),
        ("livePaint.fill", json!({"ids": [id.0], "point": [20, 30], "none": 1})),
        ("shapeBuilder.merge", json!({"ids": [id.0], "points": [[20, 30]], "fill": {"none": "true"}})),
        ("swatch.new", json!({"none": [true]})),
        ("swatch.edit", json!({"name": name, "paint": {"none": "false"}})),
    ];
    for (cmd, p) in &calls {
        let r = s.execute(cmd, p);
        assert!(refused(&r), "{cmd} {p}: {r:?}");
        assert_eq!(state(&s), before, "{cmd} {p}");
    }
    let r = s.execute("livePaint.fill", &json!({"ids": [id.0], "point": [20, 30], "color": "#ff0000", "none": null})).unwrap();
    let g = NodeId(r["group"].as_u64().unwrap());
    let before = state(&s);
    let r = s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [10, 30], "none": "true"}));
    assert!(refused(&r), "livePaint.strokeEdge: {r:?}");
    assert_eq!(state(&s), before, "livePaint.strokeEdge");
}

#[test]
fn a_mixed_selection_shows_a_question_mark_proxy() {
    let mut s = session();
    let a = rect(&mut s);
    let b = rect(&mut s);
    let q = |s: &mut Session| s.execute("paint.proxies", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let r = q(&mut s);
    assert_eq!((r["fillMixed"].as_bool(), r["strokeMixed"].as_bool()), (Some(false), Some(false)));
    assert_eq!(r["fillActive"], true);
    s.execute("paint.setFill", &json!({"color": "#00ff00", "ids": [b.0]})).unwrap();
    let r = q(&mut s);
    assert_eq!((r["fillMixed"].as_bool(), r["strokeMixed"].as_bool()), (Some(true), Some(false)));
    assert_eq!(r["fill"]["color"], json!(Color::rgb(1.0, 0.0, 0.0)), "the first selected object's fill");
    // One selected object, or the same gradient fitted to different objects, is never mixed.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(q(&mut s)["fillMixed"], false);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"stops": [{"offset": 0, "color": "#000000"}, {"offset": 1, "color": "#ffffff"}]}})).unwrap();
    assert_eq!(s.proxy_mixed(), (false, false));
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(q(&mut s)["fillMixed"], false);
}

#[test]
fn every_selection_change_bumps_the_revision() {
    // The "?" proxy is cached per revision, so selecting by Magic Wand or font must bump it.
    let mut s = session();
    let a = rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let b = rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#fa0000"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let rev = s.doc().unwrap().revision;
    s.execute("select.magicWand", &json!({"id": a.0})).unwrap();
    assert_eq!(s.doc().unwrap().selection.len(), 2, "{a:?} and {b:?} are within the tolerance");
    assert!(s.doc().unwrap().revision > rev);
    assert_eq!(s.proxy_mixed(), (true, false));
    let r = s.execute("text.create", &json!({"x": 10, "y": 150, "text": "Hi"})).unwrap();
    let family = match &node(&s, NodeId(r["id"].as_u64().unwrap())).kind {
        NodeKind::Text(t) => t.first_style().font_family,
        _ => panic!("not text"),
    };
    let rev = s.doc().unwrap().revision;
    s.execute("select.font", &json!({"family": family})).unwrap();
    assert!(s.doc().unwrap().revision > rev);
}

#[test]
fn proxies_show_the_first_selected_object_or_the_defaults() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#123456"})).unwrap();
    assert_eq!(s.proxy_paints().0.color().unwrap().to_hex(), "#123456");
    s.execute("select.none", &json!({})).unwrap();
    s.paint.fill = Paint::solid(Color::WHITE);
    assert_eq!(s.proxy_paints().0, Paint::solid(Color::WHITE));
    // Type shows its first run's paints.
    let r = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap();
    s.execute("select.set", &json!({"ids": [r["id"]]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#654321"})).unwrap();
    assert_eq!(s.proxy_paints().0.color().unwrap().to_hex(), "#654321");
    assert_eq!(s.execute("paint.proxies", &json!({})).unwrap()["fill"], json!(s.proxy_paints().0));
    let _ = id;
}

#[test]
fn live_paint_and_appearance_items_feed_recent_colours() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("appearance.setItem", &json!({"index": 0, "color": "#0a0b0c"})).unwrap();
    assert_eq!(hexes(&s)[0], "#0a0b0c");
    // Opacity alone doesn't change the recent colours.
    s.execute("appearance.setItem", &json!({"index": 0, "opacity": 50})).unwrap();
    assert_eq!(hexes(&s).len(), 1);
    s.execute("livePaint.make", &json!({"ids": [id.0]})).unwrap();
    let g = s.doc().unwrap().selection.objects[0];
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [50, 30], "color": "#00ff00"})).unwrap();
    assert_eq!(hexes(&s)[0], "#00ff00");
}

#[test]
fn a_group_shows_its_contents_in_the_proxies() {
    let mut s = session();
    let a = rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let b = rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    // The group shows what the paint commands change: its contents' red fill, not mixed.
    assert_eq!(s.doc().unwrap().selection.len(), 1);
    assert_eq!(s.proxy_paints().0.color().unwrap().to_hex(), "#ff0000");
    assert_eq!(s.proxy_mixed(), (false, false));
    // Contents that differ make even a single selected group a "?" proxy.
    s.execute("paint.setFill", &json!({"color": "#00ff00", "ids": [b.0]})).unwrap();
    assert_eq!(s.execute("paint.proxies", &json!({})).unwrap()["fillMixed"], true);
    // An empty selection shows the defaults.
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.proxy_paints().0, s.paint.fill);
}
