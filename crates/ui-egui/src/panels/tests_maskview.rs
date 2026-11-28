//! Headless frames of the Transparency panel's mask thumbnail (M3.86): Alt-click views only the
//! mask (and edits it), Alt-click again shows the artwork. In isolation mode neither a click nor an
//! Alt-click edits the mask.

use craft::ui::list::ThumbTarget;
use egui::Modifiers;
use serde_json::json;
use vectorcraft_engine::Session;

use super::*;
use crate::tests::panel_harness::Panel;

/// The name of the mask row, which holds the two thumbnails.
const ROW: &str = "Opacity Mask";

fn run(app: &mut VectorcraftApp, id: &str, p: serde_json::Value) -> serde_json::Value {
    app.session.execute(id, &p).unwrap()
}

/// A 100 × 100 document with two rectangles, the second the mask of the first (both selected for Make Mask).
fn masked() -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let b = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    run(&mut app, "transparency.makeOpacityMask", json!({}));
    (app, a)
}

/// Click the mask thumbnail with `mods` held.
fn click_mask(panel: &mut Panel, mods: Modifiers) {
    panel.click_part(ROW, ThumbTarget::Extra(0).part(), mods);
}

#[test]
fn the_mask_thumbnail_only_edits_the_whole_symbol_instance() {
    for mods in [Modifiers::ALT, Modifiers::NONE] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let rect = |app: &mut VectorcraftApp, x| run(app, "shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20}))["id"].clone();
        let left = rect(&mut app, 10);
        let right = rect(&mut app, 50);
        let symbol = run(&mut app, "symbol.new", json!({"name": "Pair", "type": "dynamic", "ids": [left, right]}))["id"].clone();
        let mask = rect(&mut app, 20);
        run(&mut app, "transparency.makeOpacityMask", json!({"ids": [symbol, mask]}));
        run(&mut app, "symbol.instance.selectPart", json!({"id": symbol, "path": [0]}));
        let mut panel = Panel::new(app, transparency::show);
        let undo = panel.app().session.active().unwrap().history.undo.len();

        click_mask(&mut panel, mods);
        {
            let app = panel.app();
            let st = app.session.active().unwrap();
            assert!(st.doc.mask_edit.is_none() && st.shown_mask().is_none(), "the part cannot enter the instance mask with {mods:?}");
            assert_eq!(st.history.undo.len(), undo);
            assert!(st.selection.symbol_part.is_some());
        }
        for label in ["Release", "Clip", "Invert Mask", "Isolate Blending", "Knockout Group", "Opacity & Mask Define Knockout Shape"] {
            assert!(!panel.enabled(label), "{label} applies to the whole instance");
        }
        assert!(panel.enabled("Opacity") && panel.enabled("Blend Mode"));

        run(&mut panel.app(), "select.set", json!({"ids": [symbol]}));
        click_mask(&mut panel, mods);
        let app = panel.app();
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.mask_edit.map(|edit| edit.object.0), symbol.as_u64());
        assert_eq!(st.shown_mask().map(|id| id.0), mods.alt.then(|| symbol.as_u64().unwrap()));
    }
}

/// In pattern editing mode a click or an Alt-click on the mask thumbnail of a masked object in the
/// tile does not edit the mask, and the status bar says to finish editing the pattern first. Make
/// Mask is off for one object, which it would take into mask editing. After Done a click edits the
/// mask.
#[test]
fn the_mask_thumbnail_does_not_edit_the_mask_in_pattern_editing_mode() {
    let (mut app, a) = masked();
    run(&mut app, "select.set", json!({"ids": [a]}));
    run(&mut app, "object.pattern.make", json!({"name": "P"}));
    let tile = app.session.active().unwrap().isolation;
    let mut panel = Panel::new(app, transparency::show);
    for mods in [Modifiers::NONE, Modifiers::ALT] {
        panel.app().ui.status.clear();
        click_mask(&mut panel, mods);
        let app = panel.app();
        let st = app.session.active().unwrap();
        assert!(st.doc.mask_edit.is_none() && st.shown_mask().is_none(), "{mods:?}");
        assert!(st.doc.pattern_edit.is_some() && st.isolation == tile, "{mods:?}");
        assert!(app.ui.status.ends_with("finish editing the pattern first"), "{mods:?}: {}", app.ui.status);
    }
    run(&mut panel.app(), "shape.rectangle", json!({"x": 60, "y": 0, "width": 30, "height": 30}));
    assert!(!crate::menus::enabled(&panel.app(), "transparency.makeOpacityMask"), "Make Mask is off for one object");
    run(&mut panel.app(), "object.pattern.done", json!({}));
    run(&mut panel.app(), "select.set", json!({"ids": [a]}));
    click_mask(&mut panel, Modifiers::NONE);
    assert_eq!(panel.app().session.active().unwrap().doc.mask_edit.map(|e| e.object.0), Some(a));
}

/// What a click on the mask thumbnail with each modifier does, outside and inside mask editing.
#[test]
fn alt_clicking_the_mask_thumbnail_views_only_the_mask() {
    // (modifiers, editing before, viewed after, editing after, disabled after)
    let table = [
        (Modifiers::NONE, false, None, true, false),
        (Modifiers::NONE, true, None, true, false),
        (Modifiers::SHIFT, false, None, false, true),
        (Modifiers::ALT, false, Some(true), true, false),
        (Modifiers::ALT, true, Some(true), true, false),
    ];
    for (mods, editing, viewed, edits, disabled) in table {
        let (mut app, a) = masked();
        if editing {
            run(&mut app, "transparency.editOpacityMask", json!({"id": a}));
        }
        let mut panel = Panel::new(app, transparency::show);
        click_mask(&mut panel, mods);
        let app = panel.app();
        let st = app.session.active().unwrap();
        let shown = st.shown_mask().map(|n| n.0);
        assert_eq!(shown, viewed.map(|_| a), "{mods:?} editing={editing}");
        assert_eq!(st.doc.mask_edit.is_some(), edits, "{mods:?} editing={editing}");
        drop(app);
        let info = panel.app().session.execute("transparency.opacityMaskInfo", &json!({})).unwrap();
        assert_eq!(info[0]["disabled"] == true, disabled, "{mods:?} editing={editing}");
    }
    // Alt again shows the artwork, still editing.
    let (mut app, a) = masked();
    run(&mut app, "transparency.editOpacityMask", json!({"id": a}));
    let mut panel = Panel::new(app, transparency::show);
    for want in [Some(a), None] {
        click_mask(&mut panel, Modifiers::ALT);
        let app = panel.app();
        let st = app.session.active().unwrap();
        assert_eq!(st.shown_mask().map(|n| n.0), want);
        assert!(st.doc.mask_edit.is_some(), "still editing");
    }
}

/// In isolation mode a click or an Alt-click on the mask thumbnail does not edit the mask, and the
/// status bar says to exit isolation mode first. Make Mask is off for one object, which it would
/// take into mask editing. After Exit Isolation Mode a click edits the mask.
#[test]
fn the_mask_thumbnail_does_not_edit_the_mask_in_isolation_mode() {
    let (mut app, a) = masked();
    let c = run(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 30, "height": 30}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, c]}));
    let g = run(&mut app, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut app, "object.isolate", json!({"id": g}));
    run(&mut app, "select.set", json!({"ids": [a]}));
    let mut panel = Panel::new(app, transparency::show);
    for mods in [Modifiers::NONE, Modifiers::ALT] {
        panel.app().ui.status.clear();
        click_mask(&mut panel, mods);
        let app = panel.app();
        let st = app.session.active().unwrap();
        assert!(st.doc.mask_edit.is_none() && st.shown_mask().is_none(), "{mods:?}");
        assert_eq!(st.isolation.map(|i| i.0), Some(g));
        assert!(app.ui.status.ends_with("exit isolation mode first"), "{mods:?}: {}", app.ui.status);
    }
    // One object without a mask: Make Mask is off, and a click on it does nothing.
    run(&mut panel.app(), "select.set", json!({"ids": [c]}));
    panel.app().ui.status.clear();
    panel.frame();
    assert!(!panel.enabled("Make Mask"), "the button is off");
    panel.click("Make Mask");
    let app = panel.app();
    let st = app.session.active().unwrap();
    let id = vectorcraft_doc::NodeId(c.as_u64().unwrap());
    assert!(st.doc.node(id).unwrap().extras().mask.is_none() && st.doc.mask_edit.is_none());
    assert_eq!(app.ui.status, "", "the button is off");
    drop(app);
    run(&mut panel.app(), "object.exitIsolation", json!({}));
    run(&mut panel.app(), "select.set", json!({"ids": [a]}));
    click_mask(&mut panel, Modifiers::NONE);
    assert_eq!(panel.app().session.active().unwrap().doc.mask_edit.map(|e| e.object.0), Some(a));
}
