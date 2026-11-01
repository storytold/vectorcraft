//! Swatches: colour groups as first-class swatch homes (colour mode, spot plates, PDF, Document
//! Info), naming and kinds, Swatch Options edits reaching linked art, deleting, new swatches in
//! groups and colour groups made from artwork.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn rect(s: &mut Session) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap())
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

/// Add a global solid swatch to colour group `group`.
fn grouped_swatch(s: &mut Session, group: &str, name: &str, color: Color) {
    assert_eq!(run(s, "swatch.new", json!({"name": name, "color": color, "global": true, "group": group}))["name"], name);
}

#[test]
fn document_color_mode_converts_grouped_swatches() {
    let mut s = session();
    grouped_swatch(&mut s, "Grays", "Ink", Color::rgb(0.2, 0.4, 0.6));
    run(&mut s, "object.convertDocumentColorMode", json!({"mode": "cmyk"}));
    for g in &doc(&s).swatch_groups {
        for sw in &g.swatches {
            assert!(!matches!(sw.paint.color(), Some(Color::Rgb { .. })), "{} in {} is still RGB", sw.name, g.name);
        }
    }
    assert!(matches!(doc(&s).swatch("Ink").unwrap().paint.color(), Some(Color::Cmyk { .. })));
    // The File menu's mode switch converts them as well.
    let mut s = session();
    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    assert!(matches!(doc(&s).swatch("Bright Red").unwrap().paint.color(), Some(Color::Cmyk { .. })));
}

/// The first solid fill color in the subtree of `n`.
fn first_fill(n: &vectorcraft_doc::Node) -> Color {
    let mut out = None;
    n.walk(&mut |m| {
        if out.is_none() {
            out = m.appearance.fill_paint().color();
        }
    });
    out.unwrap_or_else(|| panic!("no fill in {n:?}"))
}

/// The fill of graph design "Bar", the fill of brush "Holder"'s art and its Hue Shift key color,
/// the fill of graphic style "Red" and the fill of the art of `masked`'s opacity mask.
fn colors_outside_the_layers(s: &Session, masked: NodeId) -> Vec<Color> {
    let d = doc(s);
    let brush = vectorcraft_brush::find(d, "Holder").unwrap();
    let vectorcraft_brush::Colorization::HueShift { key } = brush.kind.colorization() else { panic!("not Hue Shift") };
    vec![
        first_fill(&d.graph_designs.iter().find(|g| g.name == "Bar").unwrap().art),
        first_fill(brush.art()[0]),
        key,
        d.graphic_style("Red").unwrap().appearance.fill_paint().color().unwrap(),
        first_fill(&d.node(masked).unwrap().extras().mask.as_deref().unwrap().art),
    ]
}

/// Document Color Mode converts the colors of graph designs, of the art of brushes (with a Hue
/// Shift brush's key color), of graphic styles and of opacity-mask art as it converts the
/// layers' art, tints of global swatches included, in one undo step.
#[test]
fn document_color_mode_converts_art_outside_the_layers() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Tone", "color": "#ff0000", "global": true}));
    let hue_shift = json!({"colorization": {"method": "hueShift"}});
    for (cmd, p, paint) in [
        ("graph.design", json!({"new": "Bar"}), json!({"swatch": "Tone", "tint": 40})),
        ("brush.new", json!({"type": "art", "name": "Holder", "params": hue_shift}), json!({"color": "#ff0000"})),
        ("graphicStyle.new", json!({"name": "Red"}), json!({"color": "#ff0000"})),
    ] {
        let id = rect(&mut s);
        let mut paint = paint;
        paint["ids"] = json!([id.0]);
        run(&mut s, "paint.setFill", paint);
        run(&mut s, "paint.setStroke", json!({"none": true, "ids": [id.0]}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, cmd, p);
        run(&mut s, "edit.clear", json!({"ids": [id.0]}));
    }
    let masked = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [masked.0]}));
    let m = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#808080", "ids": [m.0]}));
    run(&mut s, "select.set", json!({"ids": [masked.0, m.0]}));
    run(&mut s, "transparency.makeOpacityMask", json!({"clip": true, "invert": false}));
    let before = colors_outside_the_layers(&s, masked);
    assert!(before.iter().all(|c| matches!(c, Color::Rgb { .. })), "{before:?}");
    let undo = s.doc().unwrap().history.undo.len();

    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    let after = colors_outside_the_layers(&s, masked);
    assert!(after.iter().all(|c| matches!(c, Color::Cmyk { .. })), "{after:?}");
    // The same red converts to the same inks wherever it is.
    let red = first_fill(doc(&s).node(masked).unwrap());
    assert_eq!(after[1..4], [red; 3]);
    // A tint of a global swatch is the converted swatch's color at that tint.
    let tone = doc(&s).swatch("Tone").unwrap().paint.color().unwrap();
    assert_eq!(after[0], tone.tinted(0.4));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(colors_outside_the_layers(&s, masked), before);
}

/// A Hue Shift brush whose art is painted with a tint of a global swatch keeps its key color equal
/// to that art color after Document Color Mode.
#[test]
fn document_color_mode_keeps_a_hue_shift_key_on_its_tinted_art() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Tone", "color": "#ff0000", "global": true}));
    let id = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Tone", "tint": 40, "ids": [id.0]}));
    run(&mut s, "paint.setStroke", json!({"none": true, "ids": [id.0]}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Holder", "params": {"colorization": {"method": "hueShift"}}}));
    run(&mut s, "edit.clear", json!({"ids": [id.0]}));
    let key_and_art = |s: &Session| {
        let brush = vectorcraft_brush::find(doc(s), "Holder").unwrap();
        let vectorcraft_brush::Colorization::HueShift { key } = brush.kind.colorization() else { panic!("not Hue Shift") };
        (key, first_fill(brush.art()[0]))
    };
    let (key, art) = key_and_art(&s);
    assert_eq!(key, art);
    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    let (key, art) = key_and_art(&s);
    assert!(matches!(art, Color::Cmyk { .. }), "{art:?}");
    assert_eq!(key, art);
}

/// The reply's `changed` counts the colors of the document's art; the colors of brush art and
/// graphic styles convert without being counted.
#[test]
fn document_color_mode_counts_the_colors_of_the_art_only() {
    let changed = |library: bool| {
        let mut s = session();
        if library {
            for (cmd, p) in [("brush.new", json!({"type": "art", "name": "Holder"})), ("graphicStyle.new", json!({"name": "Red"}))] {
                let id = rect(&mut s);
                run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [id.0]}));
                run(&mut s, "select.set", json!({"ids": [id.0]}));
                run(&mut s, cmd, p);
                run(&mut s, "edit.clear", json!({"ids": [id.0]}));
            }
        }
        let id = rect(&mut s);
        run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [id.0]}));
        let r = run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
        if library {
            let brush = vectorcraft_brush::find(doc(&s), "Holder").unwrap();
            assert!(matches!(first_fill(brush.art()[0]), Color::Cmyk { .. }));
        }
        r["changed"].as_u64().unwrap()
    };
    let in_art = changed(false);
    assert!(in_art > 0);
    assert_eq!(changed(true), in_art);
}

/// Document Color Mode chosen while a pattern is being edited also converts the tile that pattern
/// editing's Cancel puts back.
#[test]
fn document_color_mode_converts_the_tile_that_pattern_cancel_puts_back() {
    let mut s = session();
    let id = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [id.0]}));
    run(&mut s, "paint.setStroke", json!({"none": true, "ids": [id.0]}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let pattern = run(&mut s, "object.pattern.make", json!({"edit": false}))["name"].as_str().unwrap().to_string();
    run(&mut s, "object.pattern.edit", json!({"name": pattern}));
    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    run(&mut s, "object.pattern.cancel", json!({}));
    let red = first_fill(doc(&s).node(id).unwrap());
    assert!(matches!(red, Color::Cmyk { .. }), "{red:?}");
    let tile: Vec<Color> = doc(&s).pattern(&pattern).unwrap().art.iter().map(|n| first_fill(n)).collect();
    assert_eq!(tile, [red]);
}

/// The `color` stored in each effect of `a` (the object's, then each fill's and stroke's) that has
/// one, read as a color, and the `lightColor` of each effect that has one.
fn effect_colors(a: &vectorcraft_doc::Appearance) -> (Vec<Color>, Vec<Value>) {
    let effects: Vec<&vectorcraft_doc::Effect> = a.effects.iter().chain(a.items.iter().flat_map(|i| i.effects())).collect();
    let colors = effects
        .iter()
        .filter_map(|e| e.params.get("color"))
        .map(|v| v.as_str().map_or_else(|| serde_json::from_value(v.clone()).unwrap(), |h| Color::from_hex(h).unwrap()))
        .collect();
    (colors, effects.iter().filter_map(|e| e.params.get("lightColor").cloned()).collect())
}

/// Document Color Mode converts the colors that effects draw with, on objects (the effects of a
/// fill included) and in graphic styles, as it converts the art. An effect that drew its default
/// color keeps the converted default as its own; the light color of a 3D effect stays RGB. The
/// reply counts the object's effect colors, and one undo step takes them back.
#[test]
fn document_color_mode_converts_effect_colors() {
    let build = |effects: bool| {
        let mut s = session();
        let id = rect(&mut s);
        run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [id.0]}));
        run(&mut s, "paint.setStroke", json!({"none": true, "ids": [id.0]}));
        if effects {
            run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "stylize.dropShadow", "params": {"color": "#ff0000"}}));
            run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "stylize.outerGlow", "item": null}));
            run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "stylize.innerGlow", "item": 0, "params": {"color": "#ff0000"}}));
            run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "threeD.rotate", "item": null}));
            // The Outer Glow draws its default color, as in a file that leaves `color` out.
            s.edit("Default color", |d, _| {
                let glow = &mut d.node_mut(id).unwrap().appearance.effects[1];
                assert_eq!(glow.id, "stylize.outerGlow");
                if let Some(p) = glow.params.as_object_mut() {
                    p.remove("color");
                }
                Ok(())
            })
            .unwrap();
        }
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "graphicStyle.new", json!({"name": "Glow"}));
        (s, id)
    };
    let (mut s, id) = build(true);
    let object = |s: &Session| effect_colors(&doc(s).node(id).unwrap().appearance);
    let style = |s: &Session| effect_colors(&doc(s).graphic_style("Glow").unwrap().appearance);
    let before = (object(&s), style(&s));
    assert_eq!(before.0.0.len(), 2, "the Outer Glow stores no color: {before:?}");
    assert_eq!(before.1.0.len(), 2, "{before:?}");
    let undo = s.doc().unwrap().history.undo.len();

    let changed = run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}))["changed"].as_u64().unwrap();
    let red = first_fill(doc(&s).node(id).unwrap());
    assert!(matches!(red, Color::Cmyk { .. }), "{red:?}");
    for (colors, lights) in [object(&s), style(&s)] {
        assert_eq!(colors.len(), 3, "{colors:?}");
        // Drop Shadow, then the Outer Glow's default yellow, then the fill's Inner Glow.
        assert_eq!([colors[0], colors[2]], [red; 2]);
        assert!(matches!(colors[1], Color::Cmyk { .. }), "{colors:?}");
        assert_eq!(lights, [json!("#ffffff")]);
    }
    // The three effect colors on the object count; those of the graphic style, like its paints, do not.
    let (mut plain, _) = build(false);
    let without = run(&mut plain, "file.documentColorMode", json!({"mode": "cmyk"}))["changed"].as_u64().unwrap();
    assert_eq!(changed, without + 3);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((object(&s), style(&s)), before);
}

/// Document Color Mode converts the series paints and label styles a graph keeps, so a graph drawn
/// again after the switch paints its series and labels in the new mode.
#[test]
fn document_color_mode_converts_the_paints_a_graph_keeps() {
    let mut s = session();
    let red = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [red.0]}));
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4";
    let id = NodeId(
        run(&mut s, "graph.create", json!({"type": "column", "x": 10, "y": 10, "width": 60, "height": 40, "csv": csv}))["id"].as_u64().unwrap(),
    );
    let part = |s: &Session, name: &str| -> vectorcraft_doc::Node {
        let g = doc(s).node(id).unwrap();
        g.children().unwrap().iter().find(|c| c.name.as_deref() == Some(name)).map(|c| (**c).clone()).unwrap_or_else(|| panic!("no {name}"))
    };
    let series = part(&s, "2024").id;
    let label = part(&s, "Axes").children().unwrap().iter().find(|c| matches!(c.kind, vectorcraft_doc::NodeKind::Text(_))).unwrap().id;
    for target in [series, label] {
        run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [target.0]}));
    }
    let kept = |s: &Session| -> Vec<Color> {
        let g = doc(s).node(id).unwrap().extras().graph.as_deref().unwrap().clone();
        let labels = [g.label_styles.values, g.label_styles.categories, g.label_styles.legend];
        let labels = labels.into_iter().flatten().map(|st| st.fill.color().unwrap());
        g.series_paints.iter().filter_map(|p| p.fill.as_ref()?.color()).chain(labels).collect()
    };
    let before = kept(&s);
    assert_eq!(before, [Color::rgb(1.0, 0.0, 0.0); 2]);
    let undo = s.doc().unwrap().history.undo.len();

    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    let cmyk = first_fill(doc(&s).node(red).unwrap());
    assert!(matches!(cmyk, Color::Cmyk { .. }), "{cmyk:?}");
    assert_eq!(kept(&s), [cmyk; 2]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Drawn again from new data, the series keeps the converted color.
    run(&mut s, "graph.setData", json!({"id": id.0, "rows": [[2, 3], [4, 5]]}));
    let marks = part(&s, "2024");
    assert!(marks.children().unwrap().iter().all(|m| m.appearance.fill_paint().color() == Some(cmyk)), "{marks:?}");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(kept(&s), before);
}

/// Document Color Mode converts the fill and stroke that new art takes, so an object drawn after
/// the switch gets the same colors in the new mode as the art drawn before it.
#[test]
fn document_color_mode_converts_the_colors_new_art_takes() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"color": "#0000ff"}));
    let old = rect(&mut s);
    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    let paints = |a: &vectorcraft_doc::Appearance| (a.fill_paint().color(), a.stroke().and_then(|st| st.paint.color()));
    let converted = paints(&doc(&s).node(old).unwrap().appearance);
    assert!(matches!(converted, (Some(Color::Cmyk { .. }), Some(Color::Cmyk { .. }))), "{converted:?}");
    assert_eq!(paints(&s.new_art()), converted);
    let new = rect(&mut s);
    assert_eq!(paints(&doc(&s).node(new).unwrap().appearance), converted);

    // As a `command.batch` step too; a batch that fails leaves them as they were.
    for fails in [false, true] {
        let mut s = session();
        run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
        run(&mut s, "paint.setStroke", json!({"color": "#0000ff"}));
        let before = paints(&s.new_art());
        let mut steps = vec![json!({"command": "file.documentColorMode", "params": {"mode": "cmyk"}})];
        if fails {
            steps.push(json!({"command": "no.such.command"}));
        }
        let r = s.execute("command.batch", &json!({"commands": steps}));
        assert_eq!(r.is_err(), fails, "{r:?}");
        assert_eq!(paints(&s.new_art()), if fails { before } else { converted }, "fails {fails}");
    }
}

#[test]
fn color_mode_undo_restores_original_new_art_colors_and_redo_reapplies_them() {
    for batch in [false, true] {
        let mut s = session();
        run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
        run(&mut s, "paint.setStroke", json!({"color": "#123456"}));
        let before = s.paint.clone();
        if batch {
            run(
                &mut s,
                "command.batch",
                json!({"commands": [
                    {"command": "file.documentColorMode", "params": {"mode": "cmyk"}}
                ]}),
            );
        } else {
            run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
        }
        let after = s.paint.clone();
        assert_ne!(before.fill, after.fill);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.paint, before, "batch {batch}: exact original values");
        assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(s.paint, after, "batch {batch}");
        let id = rect(&mut s);
        assert_eq!(doc(&s).node(id).unwrap().appearance.fill_paint(), after.fill);
    }
}

#[test]
fn color_mode_batch_undo_restores_paints_from_before_the_batch() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
    let before = s.paint.clone();
    run(
        &mut s,
        "command.batch",
        json!({"commands": [
            {"command": "paint.setFill", "params": {"color": "#0000ff"}},
            {"command": "file.documentColorMode", "params": {"mode": "cmyk"}}
        ]}),
    );
    let after = s.paint.clone();
    assert_ne!(before.fill, after.fill);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.paint, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.paint, after);
}

#[test]
fn color_mode_group_undo_restores_paints_from_before_the_group() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
    let before = s.paint.clone();
    s.begin_undo_group();
    run(&mut s, "paint.setFill", json!({"color": "#0000ff"}));
    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    s.end_undo_group(false);
    let after = s.paint.clone();
    assert_ne!(before.fill, after.fill);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.paint, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.paint, after);
}

/// Redo of a batch or undo group that converts the document color mode and then changes the paints
/// new art takes restores the paints as the whole step left them.
#[test]
fn color_mode_redo_of_a_group_restores_the_paints_the_group_left() {
    let converted = {
        let mut s = session();
        run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
        run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
        s.paint.clone()
    };
    for batch in [false, true] {
        let mut s = session();
        run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
        let before = s.paint.clone();
        if batch {
            run(
                &mut s,
                "command.batch",
                json!({"commands": [
                    {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
                    {"command": "paint.setFill", "params": {"color": "#0000ff"}}
                ]}),
            );
        } else {
            s.begin_undo_group();
            run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
            run(&mut s, "paint.setFill", json!({"color": "#0000ff"}));
            s.end_undo_group(false);
        }
        let after = s.paint.clone();
        assert_ne!(after.fill, converted.fill, "batch {batch}");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.paint, before, "batch {batch}");
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(s.paint, after, "batch {batch}: the group's final paints");
    }
}

fn color_mode_restores_paints_when_conversion_changes_none(batch: bool, convert: bool) {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let before = s.paint.clone();
    assert!(before.fill.is_none());
    assert!(before.stroke.is_none());
    if batch {
        run(
            &mut s,
            "command.batch",
            json!({"commands": [
                {"command": "file.documentColorMode", "params": {"mode": "cmyk", "convert": convert}},
                {"command": "paint.setFill", "params": {"color": "#0000ff"}}
            ]}),
        );
    } else {
        s.begin_undo_group();
        run(&mut s, "file.documentColorMode", json!({"mode": "cmyk", "convert": convert}));
        assert_eq!(s.paint, before, "the conversion leaves both paints unchanged");
        run(&mut s, "paint.setFill", json!({"color": "#0000ff"}));
        s.end_undo_group(false);
    }
    let after = s.paint.clone();
    assert_ne!(after.fill, before.fill);
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Cmyk);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
    assert_eq!(s.paint, before, "batch {batch}: undo restores the group's starting paints");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Cmyk);
    assert_eq!(s.paint, after, "batch {batch}: redo restores the group's final paints");
}

#[test]
fn color_mode_group_restores_paints_when_conversion_changes_none() {
    color_mode_restores_paints_when_conversion_changes_none(false, true);
}

#[test]
fn color_mode_batch_restores_paints_when_conversion_changes_none() {
    color_mode_restores_paints_when_conversion_changes_none(true, true);
}

#[test]
fn color_mode_group_without_conversion_restores_paints() {
    color_mode_restores_paints_when_conversion_changes_none(false, false);
}

#[test]
fn color_mode_batch_without_conversion_restores_paints() {
    color_mode_restores_paints_when_conversion_changes_none(true, false);
}

#[test]
fn color_mode_ungrouped_skips_unchanged_paint_snapshot() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let before = s.paint.clone();
    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    assert_eq!(s.paint, before);
    assert!(s.doc().unwrap().history.undo.last().unwrap().color_mode_paints.is_none());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
    assert_eq!(s.paint, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Cmyk);
    assert_eq!(s.paint, before);
}

/// A failing batch that settles a pending color-mode preview keeps the converted paints with the
/// converted document.
#[test]
fn a_failed_batch_keeps_the_paints_of_a_settled_color_mode_preview() {
    let converted = {
        let mut s = session();
        run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
        run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
        s.paint.clone()
    };
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
    s.begin_interaction("Document Color Mode").unwrap();
    s.preview("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let r = s.execute("command.batch", &json!({"commands": [{"command": "no.such.command", "params": {}}]}));
    assert!(r.is_err());
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Cmyk);
    assert_eq!(s.paint, converted);
}

#[test]
fn canceling_a_batch_preview_restores_the_document_and_paints() {
    let mut s = color_mode_preview_session(false);
    let before = s.paint.clone();
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.begin_interaction("Batch Preview").unwrap();
    s.preview("command.batch", &json!({"commands": [{"command": "file.documentColorMode", "params": {"mode": "cmyk"}}]})).unwrap();
    s.cancel_interaction().unwrap();
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
    assert_eq!(s.paint, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    assert_eq!(s.journal.len(), journal);
}

#[test]
fn accepting_a_batch_preview_restores_new_art_paints_on_undo_and_redo() {
    let mut s = color_mode_preview_session(false);
    let before = s.paint.clone();
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.begin_interaction("Batch Preview").unwrap();
    s.preview("command.batch", &json!({"commands": [{"command": "file.documentColorMode", "params": {"mode": "cmyk"}}]})).unwrap();
    assert_eq!(s.paint, before);
    s.commit_interaction().unwrap();
    let after = s.paint.clone();
    assert_ne!(after.fill, before.fill);
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Cmyk);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    assert_eq!(s.journal.len(), journal + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
    assert_eq!(s.paint, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Cmyk);
    assert_eq!(s.paint, after);
}

#[test]
fn batch_preview_with_later_paint_restores_or_keeps_the_final_paint() {
    let commands = json!({"commands": [
        {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
        {"command": "paint.setFill", "params": {"color": "#123456"}}
    ]});
    let mut expected = color_mode_preview_session(false);
    run(&mut expected, "command.batch", commands.clone());
    let final_paint = expected.paint.clone();

    let mut s = color_mode_preview_session(false);
    let before = s.paint.clone();
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.begin_interaction("Batch Preview").unwrap();
    s.preview("command.batch", &commands).unwrap();
    s.cancel_interaction().unwrap();
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
    assert_eq!(s.paint, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    assert_eq!(s.journal.len(), journal);

    s.begin_interaction("Batch Preview").unwrap();
    s.preview("command.batch", &commands).unwrap();
    s.commit_interaction().unwrap();
    assert_eq!(s.paint, final_paint);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    assert_eq!(s.journal.len(), journal + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.paint, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.paint, final_paint);
}

#[test]
fn failed_batch_preview_leaves_no_document_paint_or_journal_change() {
    let mut s = color_mode_preview_session(false);
    let before = s.paint.clone();
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.begin_interaction("Batch Preview").unwrap();
    s.preview(
        "command.batch",
        &json!({"commands": [
            {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
            {"command": "paint.setFill", "params": {"color": "#654321"}}
        ]}),
    )
    .unwrap();
    let result = s.preview(
        "command.batch",
        &json!({"commands": [
            {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
            {"command": "paint.setFill", "params": {"color": "#123456"}},
            {"command": "no.such.command", "params": {}}
        ]}),
    );
    assert!(result.is_err());
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
    assert_eq!(s.paint, before);
    s.cancel_interaction().unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    assert_eq!(s.journal.len(), journal);
}

#[test]
fn batch_preview_restores_paint_when_mode_conversion_does_not_change_it() {
    let mut s = session();
    s.paint.fill = Paint::None;
    s.paint.stroke = Paint::None;
    s.paint.appearance = None;
    let before = s.paint.clone();
    let commands = json!({"commands": [
        {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
        {"command": "paint.setFill", "params": {"color": "#123456"}}
    ]});
    s.begin_interaction("Batch Preview").unwrap();
    s.preview("command.batch", &commands).unwrap();
    s.commit_interaction().unwrap();
    let after = s.paint.clone();
    assert_ne!(after, before);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.paint, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.paint, after);
}

#[test]
fn batch_preview_that_switches_documents_rolls_back_every_step() {
    let mut s = color_mode_preview_session(false);
    let before = s.paint.clone();
    let journal = s.journal.len();
    let docs = s.docs.len();
    let active = s.active;
    s.begin_interaction("Batch Preview").unwrap();
    let result = s.preview(
        "command.batch",
        &json!({"commands": [
            {"command": "file.documentColorMode", "params": {"mode": "cmyk"}},
            {"command": "file.new", "params": {"width": 100, "height": 100}}
        ]}),
    );
    assert!(result.is_err());
    assert_eq!(s.docs.len(), docs);
    assert_eq!(s.active, active);
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
    assert_eq!(s.paint, before);
    s.cancel_interaction().unwrap();
    assert_eq!(s.journal.len(), journal);
}

#[test]
fn color_mode_undo_restores_template_paints_and_effect_colors() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
    run(&mut s, "paint.setStroke", json!({"color": "#123456"}));
    let mut appearance = s.new_art();
    appearance.effects.push(vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: json!({"color":"#d37a42"}), visible: true });
    s.paint.appearance = Some(appearance);
    let before = s.paint.clone();
    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    let after = s.paint.clone();
    assert_ne!(before.appearance, after.appearance);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.paint, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.paint, after);
    let id = rect(&mut s);
    assert_eq!(doc(&s).node(id).unwrap().appearance.effects, after.appearance.unwrap().effects);
}

#[test]
fn color_mode_cancelled_joined_scope_restores_paints_and_history_metadata() {
    for converted_before_scope in [false, true] {
        let mut s = session();
        run(&mut s, "paint.setFill", json!({"color": "#ab6739"}));
        let original = s.paint.clone();
        s.begin_undo_group();
        rect(&mut s);
        if converted_before_scope {
            run(&mut s, "file.documentColorMode", json!({"mode":"cmyk"}));
        }
        let before_scope = s.paint.clone();
        let mode = doc(&s).color_mode;
        let scope = s.begin_undo_scope().unwrap();
        run(&mut s, "file.documentColorMode", json!({"mode": if converted_before_scope {"rgb"} else {"cmyk"}}));
        s.end_undo_scope(scope, true);
        assert_eq!(s.paint, before_scope);
        assert_eq!(doc(&s).color_mode, mode);
        s.end_undo_group(false);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.paint, if converted_before_scope { original } else { before_scope.clone() });
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(doc(&s).color_mode, mode);
        assert_eq!(s.paint, before_scope, "redo must not reapply the cancelled conversion");
    }
}

fn color_mode_preview_session(selected: bool) -> Session {
    let mut s = session();
    if selected {
        rect(&mut s);
    }
    run(&mut s, "paint.setFill", json!({"color": "#808080"}));
    run(&mut s, "paint.setStroke", json!({"color": "#123456"}));
    let mut appearance = s.new_art();
    appearance.effects.push(vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: json!({"color":"#d37a42"}), visible: true });
    s.paint.appearance = Some(appearance);
    s
}

#[test]
fn color_mode_preview_commit_converts_new_art_once_and_keeps_exact_history() {
    for selected in [false, true] {
        let mut s = color_mode_preview_session(selected);
        let before = s.paint.clone();
        let depth = s.doc().unwrap().history.undo.len();
        let mut expected = color_mode_preview_session(false);
        expected.paint = before.clone();
        run(&mut expected, "file.documentColorMode", json!({"mode":"cmyk", "grays":"black"}));
        // The template is intentionally independent from selected artwork.
        let after = expected.paint.clone();
        s.begin_interaction("Document Color Mode").unwrap();
        for grays in ["profile", "black", "profile", "black"] {
            s.preview("file.documentColorMode", &json!({"mode":"cmyk", "grays":grays})).unwrap();
            assert_eq!(s.paint, before, "preview must leave new-art paints tentative");
            assert_eq!(s.doc().unwrap().history.undo.len(), depth);
        }
        s.commit_interaction().unwrap();
        assert_eq!(s.paint, after);
        assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
        let e = s.doc().unwrap().history.undo.last().unwrap();
        assert_eq!(e.color_mode_paints, Some((before.clone(), after.clone())));
        let history_bytes = e.bytes;
        assert!(history_bytes >= 2 * std::mem::size_of::<vectorcraft_tools::PaintDefaults>());
        for _ in 0..3 {
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(s.paint, before);
            assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Rgb);
            assert_eq!(s.doc().unwrap().history.redo.last().unwrap().color_mode_paints, Some((before.clone(), after.clone())));
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(s.paint, after);
            assert_eq!(s.doc().unwrap().history.undo.last().unwrap().bytes, history_bytes);
        }
        s.commit_interaction().unwrap();
        assert_eq!(s.paint, after, "a second commit cannot convert twice");
    }
}

#[test]
fn color_mode_preview_cancel_replacement_and_failure_do_not_convert_new_art() {
    for finish in ["cancel", "noop", "failed", "mode-only"] {
        let mut s = color_mode_preview_session(true);
        let before = s.paint.clone();
        let depth = s.doc().unwrap().history.undo.len();
        s.begin_interaction("Document Color Mode").unwrap();
        s.preview("file.documentColorMode", &json!({"mode":"cmyk"})).unwrap();
        match finish {
            "cancel" => s.cancel_interaction().unwrap(),
            "noop" => {
                s.preview("file.documentColorMode", &json!({"mode":"rgb"})).unwrap();
                s.commit_interaction().unwrap();
            }
            "failed" => {
                assert!(s.preview("file.documentColorMode", &json!({"mode":"invalid"})).is_err());
                s.commit_interaction().unwrap();
            }
            _ => {
                s.preview("file.documentColorMode", &json!({"mode":"cmyk", "convert":false})).unwrap();
                s.commit_interaction().unwrap();
            }
        }
        assert_eq!(s.paint, before, "{finish}");
        assert_eq!(s.doc().unwrap().history.undo.len(), depth + usize::from(finish == "mode-only"), "{finish}");
    }
}

#[test]
fn color_mode_preview_commit_joins_paint_history_and_group_cancel_restores_it() {
    for cancel in [false, true] {
        let mut s = color_mode_preview_session(false);
        let before = s.paint.clone();
        let depth = s.doc().unwrap().history.undo.len();
        s.begin_undo_group();
        // Existing grouped metadata must keep its original snapshot when the preview commits.
        run(&mut s, "file.documentColorMode", json!({"mode":"cmyk"}));
        s.begin_interaction("Document Color Mode").unwrap();
        s.preview("object.convertDocumentColorMode", &json!({"mode":"rgb"})).unwrap();
        s.commit_interaction().unwrap();
        let after = s.paint.clone();
        let e = s.doc().unwrap().history.undo.last().unwrap();
        assert_eq!(e.color_mode_paints, Some((before.clone(), after.clone())));
        assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
        s.end_undo_group(cancel);
        if cancel {
            assert_eq!(s.paint, before);
            assert_eq!(s.doc().unwrap().history.undo.len(), depth);
        } else {
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(s.paint, before);
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(s.paint, after);
        }
    }
}

#[test]
fn grouped_spot_swatches_print_on_their_own_plate() {
    let mut s = session();
    grouped_swatch(&mut s, "Brights", "Signal Orange", Color::cmyk(0.0, 0.6, 1.0, 0.0));
    assert_eq!(run(&mut s, "swatch.setSpot", json!({"name": "Signal Orange"})), json!({"name": "Signal Orange", "spot": true}));
    assert!(doc(&s).swatch("Signal Orange").is_some_and(|w| w.spot && w.global));
    let plates = run(&mut s, "color.plates", json!({}));
    assert!(plates["plates"].as_array().unwrap().iter().any(|p| p["name"] == "Signal Orange" && p["spot"] == true), "{plates}");
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "swatch": "Signal Orange"}));
    let bytes =
        vectorcraft_pdf::export(doc(&s), &vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() }).unwrap();
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(pdf.contains("/Separation") && pdf.contains("Signal#20Orange"), "a grouped spot swatch is a Separation");
    let info = run(&mut s, "document.info", json!({}));
    assert_eq!(info["spotColors"], json!(["Signal Orange"]));
}

#[test]
fn document_info_counts_grouped_swatches() {
    let mut s = session();
    let d = doc(&s);
    let all = d.swatches.len() + d.swatch_groups.iter().map(|g| g.swatches.len()).sum::<usize>();
    assert!(all > d.swatches.len());
    assert_eq!(run(&mut s, "document.info", json!({}))["swatches"], json!(all));
}

#[test]
fn new_swatches_are_named_by_their_colour_model() {
    let mut s = session();
    let name = |s: &mut Session, p: Value| run(s, "swatch.new", p)["name"].as_str().unwrap().to_string();
    assert_eq!(name(&mut s, json!({"color": {"c": 10, "m": 20, "y": 30, "k": 0}})), "C=10 M=20 Y=30 K=0");
    assert_eq!(name(&mut s, json!({"color": "#ff8000"})), "R=255 G=128 B=0");
    assert_eq!(name(&mut s, json!({"color": {"gray": 40}})), "Gray K=40");
    // The same colour again (or an explicit name in use) gets the next free number, document-wide.
    assert_eq!(name(&mut s, json!({"color": {"c": 10, "m": 20, "y": 30, "k": 0}})), "C=10 M=20 Y=30 K=0 2");
    assert_eq!(name(&mut s, json!({"name": "Grays", "color": "#000000"})), "Grays 2");
    assert_eq!(name(&mut s, json!({"name": "Bright Red", "color": "#000000"})), "Bright Red 2");
    // A spot swatch is always global; saving a linked colour stores the colour, not the link.
    let spot = name(&mut s, json!({"name": "Ink", "color": "#336699", "spot": true}));
    assert!(doc(&s).swatch(&spot).is_some_and(|w| w.spot && w.global));
    let copy = name(&mut s, json!({"swatch": "Ink"}));
    assert_eq!(doc(&s).swatch(&copy).unwrap().paint, Paint::solid(Color::from_hex("#336699").unwrap()));
    assert!(s.execute("swatch.new", &json!({"none": true})).is_err(), "None is not a swatch");
}

#[test]
fn new_swatch_saves_the_active_pattern_or_gradient() {
    let mut s = session();
    let a = rect(&mut s);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "object.pattern.make", json!({}));
    run(&mut s, "object.pattern.done", json!({}));
    let pattern = doc(&s).patterns[0].name.clone();
    let name = run(&mut s, "swatch.new", json!({"pattern": pattern}))["name"].as_str().unwrap().to_string();
    assert_eq!(name, "New Pattern Swatch 1");
    assert!(matches!(&doc(&s).swatch(&name).unwrap().paint, Paint::Pattern { pattern: p, .. } if *p == pattern));
    assert!(s.execute("swatch.new", &json!({"pattern": "Nope"})).is_err());
    let g = json!({"gradient": {"kind": "radial", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]}});
    let name = run(&mut s, "swatch.new", g)["name"].as_str().unwrap().to_string();
    assert_eq!(name, "New Gradient Swatch 1");
    assert!(matches!(doc(&s).swatch(&name).unwrap().paint, Paint::Gradient(_)));
    assert!(s.execute("swatch.new", &json!({"pattern": pattern, "spot": true})).is_err(), "only colours are spot");
}

#[test]
fn colour_groups_hold_solid_colours_only() {
    let mut s = session();
    let g = run(&mut s, "swatch.newGroup", json!({"name": "Mix", "swatches": ["Sunset", "Red", "[None]", "Bright Blue"], "colors": ["#00ff00"]}));
    let d = doc(&s);
    let group = d.swatch_groups.iter().find(|x| x.name == g["name"]).unwrap();
    let names: Vec<&str> = group.swatches.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Red", "Bright Blue", "R=0 G=255 B=0"]);
    assert!(d.swatches.iter().any(|w| w.name == "Sunset") && d.swatches.iter().any(|w| w.name == "[None]"), "gradients and None stay put");
    assert!(d.swatch_groups.iter().find(|x| x.name == "Brights").unwrap().swatches.iter().all(|w| w.name != "Bright Blue"), "moved, not copied");
}

fn fill_of(s: &Session, id: NodeId) -> Paint {
    doc(s).node(id).unwrap().appearance.fill_paint()
}

fn linked(hex: &str, name: &str) -> Paint {
    Paint::Solid { color: Color::from_hex(hex).unwrap(), swatch: Some(name.into()), tint: 1.0 }
}

#[test]
fn editing_a_global_swatch_recolours_linked_art_in_one_step() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    let (a, b) = (rect(&mut s), rect(&mut s));
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "swatch": "Brand"}));
    run(&mut s, "paint.setFill", json!({"ids": [b.0], "color": "#2a6fb0"}));
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Hi"}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"ids": [t.0], "swatch": "Brand"}));
    run(&mut s, "select.set", json!({"ids": []}));
    run(&mut s, "paint.setStroke", json!({"swatch": "Brand"}));
    let undo_depth = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "swatch.edit", json!({"name": "Brand", "color": "#ff0000"}));
    assert_eq!(r, json!({"name": "Brand", "relinked": 2}));
    assert_eq!(fill_of(&s, a), linked("#ff0000", "Brand"));
    assert_eq!(fill_of(&s, b), Paint::solid(Color::from_hex("#2a6fb0").unwrap()), "unlinked art keeps its colour");
    let text_fill = |s: &Session| match &doc(s).node(t).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(tx) => tx.runs[0].style.fill.clone(),
        _ => panic!("type"),
    };
    assert_eq!(text_fill(&s), linked("#ff0000", "Brand"), "text runs follow the swatch");
    assert_eq!(s.paint.stroke, linked("#ff0000", "Brand"), "so does the default stroke");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_depth + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fill_of(&s, a), linked("#2a6fb0", "Brand"));
    assert_eq!(doc(&s).swatch("Brand").unwrap().paint.color(), Color::from_hex("#2a6fb0"));
}

#[test]
fn renaming_relinks_and_spot_forces_global() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "swatch": "Brand"}));
    // A name in use gets the next free number.
    let r = run(&mut s, "swatch.edit", json!({"name": "Brand", "newName": "Red"}));
    assert_eq!(r, json!({"name": "Red 2", "relinked": 1}));
    assert_eq!(fill_of(&s, a), linked("#2a6fb0", "Red 2"));
    assert!(doc(&s).swatch("Brand").is_none());
    run(&mut s, "swatch.edit", json!({"name": "Red 2", "spot": true, "global": false}));
    assert!(doc(&s).swatch("Red 2").is_some_and(|w| w.spot && w.global), "spot colours stay global");
    // Turning Global off (no longer spot) unlinks the art, which keeps its colour.
    let r = run(&mut s, "swatch.edit", json!({"name": "Red 2", "spot": false, "global": false, "color": "#000000"}));
    assert_eq!(r["relinked"], 1);
    assert_eq!(fill_of(&s, a), Paint::solid(Color::from_hex("#2a6fb0").unwrap()));
    assert!(doc(&s).swatch("Red 2").is_some_and(|w| !w.global && w.paint.color() == Some(Color::BLACK)));
}

#[test]
fn grouped_swatches_edit_and_convert_modes() {
    let mut s = session();
    let r = run(&mut s, "swatch.edit", json!({"name": "Bright Red", "mode": "cmyk", "global": true}));
    assert_eq!(r["name"], "Bright Red");
    let w = doc(&s).swatch("Bright Red").unwrap();
    assert!(w.global && matches!(w.paint.color(), Some(Color::Cmyk { .. })));
    assert_eq!(doc(&s).swatch_group_of("Bright Red").map(|g| doc(&s).swatch_groups[g].name.clone()), Some("Brights".into()));
    run(&mut s, "swatch.edit", json!({"name": "Bright Red", "color": "#123456", "mode": "web"}));
    assert_eq!(doc(&s).swatch("Bright Red").unwrap().paint.color().unwrap().to_hex(), "#003366");
    run(&mut s, "swatch.edit", json!({"name": "Bright Red", "mode": "gray"}));
    assert!(matches!(doc(&s).swatch("Bright Red").unwrap().paint.color(), Some(Color::Gray { .. })));
    // Gradients can be renamed but have no colour, mode or spot; None can't be edited.
    assert!(s.execute("swatch.edit", &json!({"name": "Sunset", "color": "#000000"})).is_err());
    assert!(s.execute("swatch.edit", &json!({"name": "Sunset", "spot": true})).is_err());
    assert!(s.execute("swatch.edit", &json!({"name": "[None]", "newName": "x"})).is_err());
    assert!(s.execute("swatch.edit", &json!({"name": "Bright Red", "mode": "xyz"})).is_err());
    assert_eq!(run(&mut s, "swatch.edit", json!({"name": "Sunset", "newName": "Dusk"}))["name"], "Dusk");
}

#[test]
fn swatch_list_reports_kinds_and_groups() {
    let mut s = session();
    let all = run(&mut s, "swatch.list", json!({}));
    let list = all["swatches"].as_array().unwrap();
    assert_eq!(list.len(), doc(&s).swatches_iter().count() + 1, "and the built-in [Registration]");
    let find = |n: &str| list.iter().find(|w| w["name"] == n).unwrap().clone();
    assert_eq!(find("[None]")["kind"], "none");
    assert_eq!(find("Red")["kind"], "color");
    assert_eq!(find("Red")["hex"], "#ed1c24");
    assert_eq!(find("Sunset")["kind"], "gradient");
    assert_eq!(find("K=50")["group"], "Grays");
    let grays = run(&mut s, "swatch.list", json!({"group": "Grays"}));
    assert_eq!(grays["swatches"].as_array().unwrap().len(), 9);
    assert_eq!(grays["groups"][0]["name"], "Grays");
    assert!(s.execute("swatch.list", &json!({"group": "Nope"})).is_err());
}

#[test]
fn deleting_a_global_swatch_unlinks_art_that_keeps_its_colour() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "swatch": "Brand"}));
    run(&mut s, "select.set", json!({"ids": []}));
    run(&mut s, "paint.setFill", json!({"swatch": "Brand"}));
    let before = doc(&s).clone();
    assert_eq!(run(&mut s, "swatch.delete", json!({"name": "Brand"})), json!({"deleted": ["Brand"], "unlinked": 1}));
    assert!(doc(&s).swatch("Brand").is_none());
    assert_eq!(fill_of(&s, a), Paint::solid(Color::from_hex("#2a6fb0").unwrap()));
    assert_eq!(s.paint.fill, Paint::solid(Color::from_hex("#2a6fb0").unwrap()), "the default fill is unlinked too");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*doc(&s), before, "one undo step restores swatch and link");
    // unlink: false keeps the stale link.
    assert_eq!(run(&mut s, "swatch.delete", json!({"name": "Brand", "unlink": false}))["unlinked"], 0);
    assert_eq!(fill_of(&s, a), linked("#2a6fb0", "Brand"));
}

#[test]
fn deleting_several_swatches_and_groups() {
    let mut s = session();
    let r = run(&mut s, "swatch.delete", json!({"names": ["Red", "Brights", "Sunset"]}));
    assert_eq!(r["deleted"], json!(["Red", "Brights", "Sunset"]));
    let d = doc(&s);
    assert!(d.swatch("Red").is_none() && d.swatch("Sunset").is_none() && d.swatch("Bright Red").is_none());
    assert!(d.swatch_groups.iter().all(|g| g.name != "Brights") && d.swatch_groups.iter().any(|g| g.name == "Grays"));
    assert!(s.execute("swatch.delete", &json!({"name": "[None]"})).is_err(), "None stays");
    assert!(s.execute("swatch.delete", &json!({"names": ["Orange", "Nope"]})).is_err(), "unknown names fail the whole call");
    assert!(doc(&s).swatch("Orange").is_some());
    assert!(s.execute("swatch.delete", &json!({})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert!(doc(&s).swatch("Bright Red").is_some() && doc(&s).swatch("Red").is_some());
}

#[test]
fn new_swatches_go_into_groups_convert_modes_and_can_be_spot() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Ink", "color": "#ff0000", "spot": true, "mode": "cmyk", "group": "Brights"}));
    let d = doc(&s);
    let ink = d.swatch("Ink").unwrap();
    assert!(ink.spot && ink.global, "spot colours are global");
    assert!(matches!(ink.paint.color(), Some(Color::Cmyk { .. })), "mode converts the colour");
    assert_eq!(d.swatch_group_of("Ink").map(|g| d.swatch_groups[g].name.as_str()), Some("Brights"));
    assert!(s.execute("swatch.new", &json!({"color": "#00ff00", "group": "Nope"})).is_err());
    assert!(s.execute("swatch.new", &json!({"swatch": "Sunset", "group": "Brights"})).is_err(), "groups hold solid colours only");
    assert!(s.execute("swatch.new", &json!({"color": "#00ff00", "mode": "xyz"})).is_err());
}

/// Two rectangles: `a` filled red and stroked green, `b` filled red and stroked with global swatch
/// "Brand"; both selected.
fn art(s: &mut Session) -> (NodeId, NodeId) {
    run(s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    let (a, b) = (rect(s), rect(s));
    run(s, "paint.setFill", json!({"ids": [a.0, b.0], "color": "#ff0000"}));
    run(s, "paint.setStroke", json!({"ids": [a.0], "color": "#00ff00"}));
    run(s, "paint.setStroke", json!({"ids": [b.0], "swatch": "Brand"}));
    run(s, "select.set", json!({"ids": [a.0, b.0]}));
    (a, b)
}

#[test]
fn a_group_from_artwork_holds_its_unique_colours_and_links_the_art() {
    let mut s = session();
    let (a, b) = art(&mut s);
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "swatch.newGroup", json!({"name": "Art", "fromArtwork": true}));
    assert_eq!(r["swatches"], json!(["Brand", "R=255 G=0 B=0", "R=0 G=255 B=0"]), "the linked swatch moves in, the rest are new");
    assert_eq!(r["linked"], 3, "both red fills and the green stroke");
    let d = doc(&s);
    assert!(d.swatch("R=255 G=0 B=0").unwrap().global);
    assert_eq!(d.swatch_group_of("Brand").map(|g| d.swatch_groups[g].name.as_str()), Some("Art"));
    assert_eq!(fill_of(&s, a), linked("#ff0000", "R=255 G=0 B=0"));
    assert_eq!(fill_of(&s, b), linked("#ff0000", "R=255 G=0 B=0"));
    assert_eq!(doc(&s).node(b).unwrap().appearance.stroke_paint(), linked("#2a6fb0", "Brand"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // Without Convert to Global the swatches are process colours and the art stays unlinked.
    run(&mut s, "edit.undo", json!({}));
    let r = run(&mut s, "swatch.newGroup", json!({"fromArtwork": true, "toGlobal": false}));
    assert_eq!((r["name"].as_str(), r["linked"].as_u64()), (Some("Color Group"), Some(0)));
    assert!(!doc(&s).swatch("R=0 G=255 B=0").unwrap().global);
    assert_eq!(fill_of(&s, a), Paint::solid(Color::from_hex("#ff0000").unwrap()));
    run(&mut s, "select.set", json!({"ids": []}));
    assert!(s.execute("swatch.newGroup", &json!({"fromArtwork": true})).is_err(), "needs selected art");
}

#[test]
fn tints_of_global_swatches_get_swatches_only_when_asked() {
    let mut s = session();
    let (_, b) = art(&mut s);
    // A 50% tint of Brand on b's stroke.
    let tint = Color::from_hex("#95b7d8").unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).map_solid_paints_in(&[b], &mut |c, l, _| {
        let hit = l.as_deref() == Some("Brand");
        if hit {
            *c = tint;
        }
        hit
    });
    let without = run(&mut s, "swatch.newGroup", json!({"fromArtwork": true}));
    assert_eq!(without["swatches"], json!(["Brand", "R=255 G=0 B=0", "R=0 G=255 B=0"]), "a tint brings its swatch");
    run(&mut s, "edit.undo", json!({}));
    let with = run(&mut s, "swatch.newGroup", json!({"fromArtwork": true, "includeTints": true}));
    assert_eq!(with["swatches"], json!(["Brand", "R=255 G=0 B=0", "R=0 G=255 B=0", "R=149 G=183 B=216"]));
    assert_eq!(
        doc(&s).node(b).unwrap().appearance.stroke_paint(),
        Paint::Solid { color: tint, swatch: Some("Brand".into()), tint: 1.0 },
        "tints stay linked"
    );
}

#[test]
fn group_names_are_never_blank_and_never_shared_with_their_swatches() {
    let mut s = session();
    art(&mut s);
    // A blank name falls back to the default; a swatch never takes its own group's name.
    let r = run(&mut s, "swatch.newGroup", json!({"name": "  ", "colors": ["#0000ff"]}));
    assert_eq!(r["name"], "Color Group");
    let r = run(&mut s, "swatch.newGroup", json!({"name": "R=255 G=0 B=0", "fromArtwork": true}));
    assert_eq!(r["name"], "R=255 G=0 B=0");
    assert_eq!(r["swatches"], json!(["Brand", "R=255 G=0 B=0 2", "R=0 G=255 B=0"]));
}

#[test]
fn a_group_from_a_mesh_holds_its_point_colours() {
    let mut s = session();
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "color": "#ff0000"}));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "object.mesh.create", json!({"rows": 2, "cols": 2, "appearance": "center", "highlight": 100}));
    let r = run(&mut s, "swatch.newGroup", json!({"fromArtwork": true}));
    let names = r["swatches"].as_array().unwrap();
    assert!(names.contains(&json!("R=255 G=0 B=0")) && names.contains(&json!("R=255 G=255 B=255")), "{r}");
}

/// #1735: a global swatch edit reaches opacity-mask art: the mask's colour (and with it what the
/// mask shows) follows the swatch, and a swatch used only in a mask isn't "unused".
#[test]
fn global_swatch_edits_reach_opacity_mask_art() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "MaskTone", "color": "#ffffff", "global": true}));
    let obj = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [obj.0]}));
    let m = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "MaskTone", "ids": [m.0]}));
    run(&mut s, "select.set", json!({"ids": [obj.0, m.0]}));
    run(&mut s, "transparency.makeOpacityMask", json!({"clip": true, "invert": false}));
    let mask_fill = |s: &Session| {
        let n = doc(s).node(obj).unwrap();
        n.extras().mask.as_deref().unwrap().art.appearance.fill_paint()
    };
    assert_eq!(mask_fill(&s).color(), Some(Color::rgb(1.0, 1.0, 1.0)));
    assert!(!run(&mut s, "swatch.unused", json!({}))["names"].as_array().unwrap().iter().any(|n| n == "MaskTone"), "used by the mask");

    let r = run(&mut s, "swatch.edit", json!({"name": "MaskTone", "color": "#000000"}));
    assert_eq!(r["relinked"], 1, "{r}");
    let fill = mask_fill(&s);
    assert_eq!(fill.color(), Some(Color::rgb(0.0, 0.0, 0.0)));
    assert!(matches!(&fill, Paint::Solid { swatch: Some(n), .. } if n == "MaskTone"), "{fill:?}");

    // Undo brings the mask's colour back with the swatch.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(mask_fill(&s).color(), Some(Color::rgb(1.0, 1.0, 1.0)));
}

#[test]
fn color_mode_converts_freeform_defaults_templates_and_stored_graph_paints() {
    use vectorcraft_color::{Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint};
    let mut s = session();
    let id = NodeId(
        run(
            &mut s,
            "graph.create",
            json!({"type":"column", "x":0, "y":0, "width":60, "height":40,
        "csv":",2024\nQ1,3\nQ2,5"}),
        )["id"]
            .as_u64()
            .unwrap(),
    );
    let mut gradient = GradientPaint::new(Gradient::new(GradientKind::Freeform, vec![]));
    gradient.set_freeform(Freeform {
        points: vec![
            FreeformPoint::new((0.0, 0.0).into(), Color::rgb(0.8, 0.2, 0.1)),
            FreeformPoint::new((20.0, 10.0).into(), Color::rgb(0.1, 0.3, 0.9)),
        ],
        lines: vec![vec![0, 1]],
        ..Default::default()
    });
    let paint = Paint::Gradient(Box::new(gradient));
    s.edit("Stored graph paint", |d, _| {
        d.node_mut(id).unwrap().extras_mut().graph.as_mut().unwrap().series_paints =
            vec![vectorcraft_doc::SeriesPaint { fill: Some(paint.clone()), stroke: Some(paint.clone()), ..Default::default() }];
        Ok(())
    })
    .unwrap();
    s.paint.fill = paint.clone();
    s.paint.stroke = paint.clone();
    s.paint.stroke_width = 1.0;
    s.paint.appearance = None;
    s.paint.appearance = Some(s.new_art());
    let before = s.paint.clone();
    let stored = |s: &Session| doc(s).node(id).unwrap().extras().graph.as_ref().unwrap().series_paints.clone();
    let before_graph = stored(&s);
    run(&mut s, "file.documentColorMode", json!({"mode":"cmyk"}));
    let after = s.paint.clone();
    let after_graph = stored(&s);
    let converted = |paint: &Paint| {
        let Paint::Gradient(g) = paint else { panic!("expected gradient") };
        assert!(g.gradient.stops.iter().all(|p| matches!(p.color, Color::Cmyk { .. })));
        assert!(g.freeform.as_ref().unwrap().points.iter().all(|p| matches!(p.color, Color::Cmyk { .. })));
    };
    converted(&after.fill);
    converted(&after.stroke);
    converted(&after.appearance.as_ref().unwrap().fill_paint());
    converted(&after.appearance.as_ref().unwrap().stroke().unwrap().paint);
    converted(after_graph[0].fill.as_ref().unwrap());
    converted(after_graph[0].stroke.as_ref().unwrap());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.paint, before);
    assert_eq!(stored(&s), before_graph);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.paint, after);
    assert_eq!(stored(&s), after_graph);
    let new = rect(&mut s);
    let new_fill = doc(&s).node(new).unwrap().appearance.fill_paint();
    converted(&new_fill);
    let control_colors = |paint: &Paint| {
        let Paint::Gradient(g) = paint else { panic!("expected gradient") };
        g.freeform.as_ref().unwrap().points.iter().map(|p| p.color).collect::<Vec<_>>()
    };
    assert_eq!(control_colors(&new_fill), control_colors(&after.fill));
    run(&mut s, "graph.setData", json!({"id":id.0, "rows":[[2], [4]]}));
    assert_eq!(stored(&s), after_graph);
    let graph = doc(&s).node(id).unwrap();
    let series = graph.children().unwrap().iter().find(|n| n.name.as_deref() == Some("2024")).unwrap();
    for mark in series.children().unwrap() {
        converted(&mark.appearance.fill_paint());
        converted(&mark.appearance.stroke().unwrap().paint);
    }
}

/// The fills of graph design "Bar" and of the art of brush "Holder", as `place: color, link`.
fn fills_outside_the_layers(s: &Session) -> Vec<String> {
    let d = doc(s);
    let mut out = vec![];
    let mut tree = |place: &str, n: &vectorcraft_doc::Node| {
        n.walk(&mut |m| {
            if let Paint::Solid { color, swatch, .. } = m.appearance.fill_paint() {
                out.push(format!("{place}: {}, {}", color.to_hex(), swatch.as_deref().unwrap_or("-")));
            }
        })
    };
    tree("design", &d.graph_designs.iter().find(|g| g.name == "Bar").unwrap().art);
    vectorcraft_brush::find(d, "Holder").unwrap().art().into_iter().for_each(|n| tree("brush art", n));
    out
}

/// A global swatch edit that recolors a Hue Shift brush's key color in its art moves the key to the
/// new color, so strokes still paint that art with the stroke color.
#[test]
fn global_swatch_edits_move_a_hue_shift_key() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Tone", "color": "#ff0000", "global": true}));
    let id = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Tone", "ids": [id.0]}));
    run(&mut s, "paint.setStroke", json!({"none": true, "ids": [id.0]}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Key", "ids": [id.0], "params": {"colorization": {"method": "hueShift"}}}));
    let key = |s: &mut Session| run(s, "brush.get", json!({"name": "Key"}))["colorization"]["key"].clone();
    let (red, blue) = (json!(Color::rgb(1.0, 0.0, 0.0)), json!(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(key(&mut s), red);
    run(&mut s, "swatch.edit", json!({"name": "Tone", "color": "#0000ff"}));
    assert_eq!(key(&mut s), blue);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(key(&mut s), red);

    // A paint in the key color that only an opacity mask uses leaves the key where it is.
    run(&mut s, "swatch.new", json!({"name": "MaskTone", "color": "#ff0000", "global": true}));
    let obj = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [obj.0]}));
    run(&mut s, "paint.setStroke", json!({"none": true, "ids": [obj.0]}));
    let m = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "MaskTone", "ids": [m.0]}));
    run(&mut s, "select.set", json!({"ids": [obj.0, m.0]}));
    run(&mut s, "transparency.makeOpacityMask", json!({"clip": true, "invert": false}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Masked", "ids": [obj.0], "params": {"colorization": {"method": "hueShift"}}}));
    let masked = |s: &mut Session| run(s, "brush.get", json!({"name": "Masked"}))["colorization"]["key"].clone();
    assert_eq!(masked(&mut s), red);
    run(&mut s, "swatch.edit", json!({"name": "MaskTone", "color": "#0000ff"}));
    assert_eq!(masked(&mut s), red);

    // With the art and its mask on one swatch, the key takes the art's new color, not the mask's.
    run(&mut s, "swatch.new", json!({"name": "Both", "color": "#ffffff", "global": true}));
    let obj = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Both", "ids": [obj.0]}));
    run(&mut s, "paint.setStroke", json!({"none": true, "ids": [obj.0]}));
    let m = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Both", "tint": 50, "ids": [m.0]}));
    run(&mut s, "select.set", json!({"ids": [obj.0, m.0]}));
    run(&mut s, "transparency.makeOpacityMask", json!({"clip": true, "invert": false}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Both", "ids": [obj.0], "params": {"colorization": {"method": "hueShift"}}}));
    run(&mut s, "swatch.edit", json!({"name": "Both", "color": "#0000ff"}));
    assert_eq!(run(&mut s, "brush.get", json!({"name": "Both"}))["colorization"]["key"], blue);
}

/// A global swatch edit that recolors brush art keeps the stored library entries that do not parse
/// as brushes.
#[test]
fn global_swatch_edits_keep_unreadable_brush_entries() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Tone", "color": "#ff0000", "global": true}));
    let id = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Tone", "ids": [id.0]}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Holder", "ids": [id.0]}));
    let junk = json!({"name": "Unreadable", "kind": 7});
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let Some(Value::Array(entries)) = doc.unknown.get_mut(vectorcraft_brush::DOC_KEY) else { panic!("no stored brushes") };
    entries.push(junk.clone());
    run(&mut s, "swatch.edit", json!({"name": "Tone", "color": "#0000ff"}));
    let stored = &s.doc().unwrap().doc.unknown[vectorcraft_brush::DOC_KEY];
    assert!(stored.as_array().is_some_and(|a| a.contains(&junk)), "{stored}");
    assert!(vectorcraft_brush::find(&s.doc().unwrap().doc, "Holder").is_some());
}

/// A global swatch edit that recolors brush art keeps the fields of stored brush entries that the
/// brush definition does not read, both on the brush it recolors and on the brushes it leaves alone,
/// and inside the art too. A swatch delete that unlinks the art drops the link and keeps those fields.
#[test]
fn global_swatch_edits_keep_unread_brush_fields() {
    // Adds `"extra": 8` to every object under `v` that links a swatch.
    fn tag_links(v: &mut Value) {
        match v {
            Value::Object(o) => {
                if o.contains_key("swatch") {
                    o.insert("extra".into(), json!(8));
                }
                o.values_mut().for_each(tag_links);
            }
            Value::Array(a) => a.iter_mut().for_each(tag_links),
            _ => {}
        }
    }
    // The objects under `v` that carry `"extra": 8`.
    fn tagged(v: &Value) -> Vec<&Value> {
        match v {
            Value::Object(o) => o.get("extra").filter(|e| **e == json!(8)).map(|_| v).into_iter().chain(o.values().flat_map(tagged)).collect(),
            Value::Array(a) => a.iter().flat_map(tagged).collect(),
            _ => vec![],
        }
    }
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Tone", "color": "#ff0000", "global": true}));
    let id = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Tone", "ids": [id.0]}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Holder", "ids": [id.0]}));
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let Some(Value::Array(entries)) = doc.unknown.get_mut(vectorcraft_brush::DOC_KEY) else { panic!("no stored brushes") };
    for e in entries.iter_mut() {
        if let Some(o) = e.as_object_mut() {
            o.insert("extra".into(), json!(7));
        }
        tag_links(e);
    }
    let links = tagged(&doc.unknown[vectorcraft_brush::DOC_KEY]).len();
    assert!(links > 0, "{}", doc.unknown[vectorcraft_brush::DOC_KEY]);
    run(&mut s, "swatch.edit", json!({"name": "Tone", "color": "#0000ff"}));
    let stored = &s.doc().unwrap().doc.unknown[vectorcraft_brush::DOC_KEY];
    let entries = stored.as_array().map(Vec::as_slice).unwrap_or_default();
    assert!(entries.len() > 1, "{stored}");
    assert!(entries.iter().all(|e| e["extra"] == json!(7)), "{stored}");
    assert_eq!(tagged(stored).len(), links, "{stored}");
    let holder = vectorcraft_brush::find(&s.doc().unwrap().doc, "Holder").unwrap();
    let fill = holder.art().first().map(|n| n.appearance.fill_paint().color().map(|c| c.to_hex()));
    assert_eq!(fill, Some(Some("#0000ff".to_string())));

    run(&mut s, "swatch.delete", json!({"name": "Tone"}));
    let stored = &s.doc().unwrap().doc.unknown[vectorcraft_brush::DOC_KEY];
    let unlinked = tagged(stored);
    assert_eq!(unlinked.len(), links, "{stored}");
    assert!(unlinked.iter().all(|o| o.get("swatch").is_none()), "{stored}");
    let holder = vectorcraft_brush::find(&s.doc().unwrap().doc, "Holder").unwrap();
    let fill = holder.art().first().map(|n| n.appearance.fill_paint().color().map(|c| c.to_hex()));
    assert_eq!(fill, Some(Some("#0000ff".to_string())));
}

/// A global swatch edit, rename, delete and Default Swatches with replace change the art of graph
/// designs and brushes: the art follows the swatch's color and name, or keeps its color unlinked,
/// each in one undo step.
#[test]
fn global_swatch_edits_reach_graph_designs_and_brush_art() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Tone", "color": "#ff0000", "global": true}));
    for (cmd, p) in [("graph.design", json!({"new": "Bar"})), ("brush.new", json!({"type": "art", "name": "Holder"}))] {
        let id = rect(&mut s);
        run(&mut s, "paint.setFill", json!({"swatch": "Tone", "ids": [id.0]}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, cmd, p);
        run(&mut s, "edit.clear", json!({"ids": [id.0]}));
    }
    let both = |fill: &str| vec![format!("design: {fill}"), format!("brush art: {fill}")];
    assert_eq!(fills_outside_the_layers(&s), both("#ff0000, Tone"));
    let undo = s.doc().unwrap().history.undo.len();

    let r = run(&mut s, "swatch.edit", json!({"name": "Tone", "color": "#0000ff"}));
    assert_eq!(r["relinked"], 2, "{r}");
    assert_eq!(fills_outside_the_layers(&s), both("#0000ff, Tone"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fills_outside_the_layers(&s), both("#ff0000, Tone"));

    run(&mut s, "swatch.edit", json!({"name": "Tone", "newName": "Hue"}));
    assert_eq!(fills_outside_the_layers(&s), both("#ff0000, Hue"));
    // Default Swatches with replace drops the links to the swatches it removes.
    run(&mut s, "swatch.resetDefaults", json!({"replace": true}));
    assert_eq!(fills_outside_the_layers(&s), both("#ff0000, -"));
    run(&mut s, "edit.undo", json!({}));
    let r = run(&mut s, "swatch.delete", json!({"name": "Hue"}));
    assert_eq!(r["unlinked"], 2, "{r}");
    assert_eq!(fills_outside_the_layers(&s), both("#ff0000, -"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 2);
}

/// A global swatch edited while a pattern is being edited also changes the tile that pattern
/// editing's Cancel puts back.
#[test]
fn global_swatch_edits_reach_the_tile_that_pattern_cancel_puts_back() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Tone", "color": "#ff0000", "global": true}));
    let id = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Tone", "ids": [id.0]}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let pattern = run(&mut s, "object.pattern.make", json!({"edit": false}))["name"].as_str().unwrap().to_string();
    run(&mut s, "object.pattern.edit", json!({"name": pattern}));
    run(&mut s, "swatch.edit", json!({"name": "Tone", "color": "#0000ff"}));
    run(&mut s, "object.pattern.cancel", json!({}));
    let mut fills = vec![];
    for n in &doc(&s).pattern(&pattern).unwrap().art {
        n.walk(&mut |m| fills.push(m.appearance.fill_paint()));
    }
    assert_eq!(fills, [Paint::Solid { color: Color::rgb(0.0, 0.0, 1.0), swatch: Some("Tone".into()), tint: 1.0 }]);
}
