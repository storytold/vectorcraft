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

#[test]
fn recent_colours_remember_the_spot_swatch_they_came_from() {
    let mut s = session();
    rect(&mut s);
    s.execute("swatch.new", &json!({"name": "Ink 185", "color": "#e90029", "spot": true})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Ink 185"})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    let r = s.execute("paint.recent", &json!({})).unwrap();
    assert_eq!(r["colors"][0]["hex"], "#00ff00");
    assert!(r["colors"][0].get("swatch").is_none());
    assert_eq!((r["colors"][1]["swatch"].as_str(), r["colors"][1]["tint"].as_f64()), (Some("Ink 185"), Some(100.0)));
    assert_eq!(s.recent_colors.len(), s.recent_links.len());
}
