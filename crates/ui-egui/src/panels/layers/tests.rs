use serde_json::json;
use vectorcraft_engine::Session;

use super::*;
use crate::i18n::LangExt;
// PLAN: assumed paths for the framework and keymap types the table tests build events from.
use craft::behavior::SelectionChange;
use craft::keymap::{Chord, Mods};
use craft::layers::ui::panel::{LayerColumn, LayerSelect, LayersEvent};
use craft::layers::ui::tree::LayerRowKey;
use craft::layers::ui::tree::LayerTree;

/// A generated `<Kind>` name is translated where painted; an unnamed text object's text never
/// is, even when it reads like one.
#[test]
fn only_generated_names_are_translated() {
    let zh = crate::i18n::Lang::from_code("zh-hant").unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let rect = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let text = run(&mut app, "text.create", json!({"x": 10, "y": 40, "text": "<Path>"}))["id"].as_u64().unwrap();
    let doc = &app.session.active().unwrap().doc;
    let names = |id: u64| {
        let n = doc.node(NodeId(id)).unwrap();
        (n.display_name(), model::painted_name(n, &n.display_name(), zh))
    };
    assert_eq!(names(text), ("<Path>".to_string(), "<Path>".to_string()));
    let (stored, painted) = names(rect);
    let inner = stored.trim_start_matches('<').trim_end_matches('>');
    assert_eq!(painted, format!("<{}>", crate::i18n::tr(zh, inner)));
}

/// The key of a layer row.
// PLAN: assumed `RowId::Node(NodeId)` (document-panels-w7 model.rs).
fn layer_key(id: NodeId) -> LayerRowKey<RowId> {
    LayerRowKey::Layer(RowId::Node(id))
}

/// Runs `events` through the panel's event mapping, as the panel does after a pass.
// PLAN: assumed `rows::apply(app: &mut VectorcraftApp, events: Vec<LayersEvent<RowId>>)` (document-panels-w7).
fn feed(app: &mut VectorcraftApp, events: Vec<LayersEvent<RowId>>) {
    rows::apply(app, events);
}

/// Modifiers with only the named keys held.
// PLAN: assumed `Mods` has public `alt` and `cmd` fields and `Mods::NONE` (keymap).
fn mods(alt: bool, cmd: bool) -> Mods {
    let mut m = Mods::NONE;
    m.alt = alt;
    m.cmd = cmd;
    m
}

/// A press on `row`'s name that selects it with `mods` held.
fn select_with(app: &mut VectorcraftApp, row: NodeId, mods: Mods) {
    let mut select = LayerSelect::new(SelectionChange::Set(vec![RowId::Node(row)]), None);
    select.mods = mods;
    feed(app, vec![LayersEvent::Select(select)]);
}

/// Answers the newest open dialog (OK when `ok`, Cancel otherwise) and runs what the answer asks for.
fn answer(app: &mut VectorcraftApp, ctx: &egui::Context, ok: bool) {
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| {
            let Some(id) = host.open_dialogs().last().map(|d| d.id) else { return };
            if ok {
                host.confirm_by_id(ctx, id);
            } else {
                host.cancel_by_id(ctx, id);
            }
        });
    }
    crate::dialogs::host::pump(app, ctx);
}

/// The target-mark rule: each of the four looks (neither, targeted, an appearance, both) differs,
/// and the dot sits clear of the inner ring, drawn in the theme's thin stroke, when both show.
#[test]
fn target_circle_marks_follow_targeted_and_styled() {
    use rows::{TARGET_DOT, TARGET_DOT_IN_RING, TARGET_RING, target_marks};
    assert_eq!(target_marks(false, false), (None, None));
    assert_eq!(target_marks(true, false), (Some(TARGET_RING), None));
    assert_eq!(target_marks(false, true), (None, Some(TARGET_DOT)));
    let (ring, dot) = target_marks(true, true);
    assert_eq!((ring, dot), (Some(TARGET_RING), Some(TARGET_DOT_IN_RING)));
    let thin = std::rc::Rc::new(std::cell::Cell::new(f32::INFINITY));
    let seen = std::rc::Rc::clone(&thin);
    let mut fixture = craft::ui::testing::Fixture::new(move |ui| seen.set(craft::ui::runtime::env::Env::of(ui).tokens.stroke.thin));
    fixture.root(crate::app::root_env(&vectorcraft_engine::Prefs::default(), 1.0)).frame(vec![]);
    assert!(TARGET_DOT_IN_RING + 0.5 < TARGET_RING - thin.get() / 2.0, "the dot covers the ring");
}

/// A document with two rectangles on one layer → (app, layer, [bottom, top]).
pub(super) fn two_rects() -> (VectorcraftApp, NodeId, [NodeId; 2]) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let b = run(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    run(&mut app, "select.none", json!({}));
    let layer = app.session.active().unwrap().doc.layers[0].id;
    (app, layer, [NodeId(a), NodeId(b)])
}

/// An eye or lock click with modifiers is `LayersEvent::CellClick`; the panel's rule turns it into
/// the "others" commands (Alt on an eye hides the others, Alt on a lock locks them, Cmd+Alt on an
/// eye switches the others between Outline and Preview), each in one undo step, and a second click
/// undoes the first's effect on every layer.
#[test]
fn column_events_reach_their_commands() {
    // (column, alt, cmd, clicked layer from the bottom, state after one click, state after two);
    // a state is (visible, locked, preview) per layer, bottom first.
    type State = [(bool, bool, bool); 3];
    const ALL: State = [(true, false, true); 3];
    let cases: [(LayerColumn, bool, bool, usize, State, State); 3] = [
        (LayerColumn::Visibility, true, false, 1, [(false, false, true), (true, false, true), (false, false, true)], ALL),
        (LayerColumn::Lock, true, false, 1, [(true, true, true), (true, false, true), (true, true, true)], ALL),
        (LayerColumn::Visibility, true, true, 0, [(true, false, true), (true, false, false), (true, false, false)], ALL),
    ];
    for (column, alt, cmd, clicked, once, twice) in cases {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("layer.new", &json!({})).unwrap();
        app.session.execute("layer.new", &json!({})).unwrap();
        let state = |app: &VectorcraftApp| -> Vec<(bool, bool, bool)> {
            let doc = &app.session.active().unwrap().doc;
            doc.layers.iter().map(|l| (l.visible, l.locked, !matches!(l.kind, NodeKind::Layer { preview: false, .. }))).collect()
        };
        let row = layer_key(app.session.active().unwrap().doc.layers[clicked].id);
        // Alt alone on an eye is the panel's own Show Only press; the other cases are the app's rule on a cell click.
        let event = |row: LayerRowKey<RowId>| match (column, alt, cmd) {
            (LayerColumn::Visibility, true, false) => LayersEvent::ShowOnly(row),
            _ => LayersEvent::cell_click(row, column, mods(alt, cmd)),
        };
        let click = |app: &mut VectorcraftApp| feed(app, vec![event(row.clone())]);
        let before = undo_len(&app);
        click(&mut app);
        assert_eq!(state(&app), once, "{column:?} alt={alt} cmd={cmd}");
        assert_eq!(undo_len(&app), before + 1, "one undo step");
        click(&mut app);
        assert_eq!(state(&app), twice, "{column:?} alt={alt} cmd={cmd}: the second click");
        assert_eq!(undo_len(&app), before + 2);
    }
}

/// A plain click on an eye (no modifiers) is no `CellClick` rule: only the modifier cases are.
/// Alt alone on a layer row's name adds its art to the selection; with Shift or Cmd it does not.
#[test]
fn row_events_reach_their_commands() {
    let (mut app, layer, [a, b]) = two_rects();
    select_with(&mut app, layer, mods(true, false));
    assert_eq!(app.session.active().unwrap().selection.objects, vec![a, b], "Alt selects the layer's art");
    app.session.execute("select.none", &json!({})).unwrap();
    select_with(&mut app, layer, mods(true, true));
    assert!(app.session.active().unwrap().selection.objects.is_empty(), "Cmd+Alt is not the select-art gesture");
    select_with(&mut app, layer, mods(false, false));
    assert!(app.session.active().unwrap().selection.objects.is_empty(), "a plain press selects no art");
}

/// The message of the confirmation open now, if one is.
fn asked(app: &VectorcraftApp) -> Option<String> {
    // PLAN: assumed `confirm::open_question(&VectorcraftApp) -> Option<(String, String, String)>` (widgets-w10, files-w20).
    crate::dialogs::confirm::open_question(app).map(|(message, _, _)| message)
}

/// `LayersEvent::Delete` on a layer holding art asks first, Cancel keeps it, and OK deletes it in
/// one undo step; hidden and locked art counts.
#[test]
fn deleting_a_layer_that_holds_art_asks_first() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let rect = NodeId(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].as_u64().unwrap());
    run(&mut app, "select.none", json!({}));
    run(&mut app, "layer.setProps", json!({"ids": [rect.0], "visible": false, "locked": true}));
    let art = app.session.active().unwrap().doc.layers[0].id;
    run(&mut app, "layer.new", json!({}));
    let there = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(art).is_some();
    let ctx = app.ui_ctx.clone().unwrap();
    let question = Some("Delete this layer and its artwork?".to_string());
    feed(&mut app, vec![LayersEvent::Delete(vec![layer_key(art)])]);
    assert_eq!(asked(&app), question, "Delete asks");
    assert!(there(&app), "nothing is deleted before OK");
    answer(&mut app, &ctx, false);
    assert!(!app.any_dialog_open() && there(&app), "Cancel keeps the layer");
    let undo = undo_len(&app);
    feed(&mut app, vec![LayersEvent::Delete(vec![layer_key(art)])]);
    answer(&mut app, &ctx, true);
    let st = app.session.active().unwrap();
    assert!(!app.any_dialog_open() && st.doc.node(art).is_none() && st.doc.node(rect).is_none());
    assert_eq!((st.doc.layers.len(), st.history.undo.len()), (1, undo + 1), "one undo step");
    run(&mut app, "edit.undo", json!({}));
    assert!(there(&app));
}

/// The panel menu's Delete Selection (`ui.layerDelete`) asks once for several rows when one of
/// them is a layer with art, art in a sublayer included; an empty layer, a layer with only a
/// guide, and objects alone go at once, and `layer.delete` itself never asks.
#[test]
fn delete_selection_asks_only_for_layers_with_art() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    let ctx = app.ui_ctx.clone().unwrap();
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let bottom = app.session.active().unwrap().doc.layers[0].id.0;
    let loose = id(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})));
    let outer = id(run(&mut app, "layer.new", json!({})));
    let sub = id(run(&mut app, "layer.newSublayer", json!({})));
    let nested = id(run(&mut app, "shape.ellipse", json!({"x": 50, "y": 50, "width": 20, "height": 20})));
    run(&mut app, "select.none", json!({}));
    let empty = id(run(&mut app, "layer.new", json!({"top": true})));
    let guided = id(run(&mut app, "layer.new", json!({"top": true})));
    run(&mut app, "guide.add", json!({"vertical": true, "pos": 50, "layer": guided}));
    run(&mut app, "select.none", json!({}));
    let exists = |app: &VectorcraftApp, id: u64| app.session.active().unwrap().doc.node(NodeId(id)).is_some();
    for (rows, what) in [(json!([empty]), "an empty layer"), (json!([guided]), "a layer with only a guide"), (json!([loose]), "an object")] {
        app.run("ui.layerDelete", json!({ "ids": rows })).unwrap();
        assert!(!app.any_dialog_open(), "{what} goes at once");
        assert!(!exists(&app, rows[0].as_u64().unwrap()), "{what} is deleted");
    }
    run(&mut app, "edit.undo", json!({}));
    // Art in a sublayer counts; the highlighted rows are the default, and a mixed selection asks once.
    run(&mut app, "layer.highlight", json!({"ids": [outer, loose]}));
    app.run("ui.layerDelete", json!({})).unwrap();
    assert_eq!(asked(&app).as_deref(), Some("Delete the selected items and their artwork?"));
    assert!(exists(&app, nested) && exists(&app, loose));
    answer(&mut app, &ctx, true);
    assert!(!exists(&app, outer) && !exists(&app, sub) && !exists(&app, nested) && !exists(&app, loose));
    assert!(!app.any_dialog_open(), "asked once");
    // A sublayer with art asks as a layer does.
    run(&mut app, "edit.undo", json!({}));
    app.run("ui.layerDelete", json!({ "id": sub })).unwrap();
    assert_eq!(asked(&app).as_deref(), Some("Delete this layer and its artwork?"));
    answer(&mut app, &ctx, false);
    // Scripts deleting with layer.delete are not asked.
    app.run("layer.delete", json!({ "ids": [outer] })).unwrap();
    assert!(!app.any_dialog_open() && !exists(&app, outer));
    assert!(exists(&app, bottom));
}

/// Rows `layer.delete` refuses are refused without a question, and OK deletes nothing once another
/// document is active.
#[test]
fn delete_selection_refusals_and_a_document_change() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    let ctx = app.ui_ctx.clone().unwrap();
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}));
    let only = app.session.active().unwrap().doc.layers[0].id.0;
    let e = app.run("ui.layerDelete", json!({ "ids": [only] })).unwrap_err();
    assert!(e.contains("at least one layer") && !app.any_dialog_open(), "{e}");
    for bad in [json!({"ids": [999_999]}), json!({"id": "x"}), json!({"ids": 3}), json!({"confirmed": "yes"}), json!({"doc": -1})] {
        assert!(app.run("ui.layerDelete", bad.clone()).is_err() && !app.any_dialog_open(), "{bad}");
    }
    let art = run(&mut app, "layer.new", json!({}))["id"].as_u64().unwrap();
    run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}));
    app.run("ui.layerDelete", json!({ "ids": [art] })).unwrap();
    assert!(asked(&app).is_some());
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    answer(&mut app, &ctx, true);
    assert!(app.ui.status.contains("nothing was deleted"), "{}", app.ui.status);
    let first = &app.session.documents()[0].doc;
    assert!(first.node(NodeId(art)).is_some(), "the first document keeps the layer");
}

/// Mask editing lists only the mask's layer, and ending it lists the rows again.
#[test]
fn mask_editing_lists_only_the_mask() {
    let (mut app, _, [a, _]) = two_rects();
    let before = listed_ids(&app).len();
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    app.session.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    assert_eq!(listed_ids(&app).len(), 1, "editing the new mask lists one row");
    app.session.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    assert_eq!(listed_ids(&app).len(), before, "the rows are listed again");
}

/// The object rows the panel lists for the active document, top down, with their depths.
pub(super) fn listed_ids(app: &VectorcraftApp) -> Vec<(NodeId, usize)> {
    fn walk(tree: &DocLayers, parent: Option<&RowId>, depth: usize, out: &mut Vec<(NodeId, usize)>) {
        for id in tree.children(parent) {
            if let RowId::Node(n) = &id
                && tree.listed(&id, &LayerFilter::default())
            {
                out.push((*n, depth));
            }
            if tree.is_open(&id) {
                walk(tree, Some(&id), depth + 1, out);
            }
        }
    }
    let tree = DocLayers::new(app, app.session.active().unwrap());
    let mut out = Vec::new();
    walk(&tree, None, 0, &mut out);
    out
}

/// Layer 1 holding rectangle `c` and a sublayer holding rectangle `a` and a group of
/// rectangle `b`, every row open, nothing selected → (app, [layer, sub, g, b, a, c]): the rows
/// top down.
pub(super) fn nested() -> (VectorcraftApp, [NodeId; 6]) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
    run(&mut app, "file.new", json!({"width": 400, "height": 400}));
    let layer = app.session.active().unwrap().doc.layers[0].id;
    let c = id(run(&mut app, "shape.rectangle", json!({"x": 300, "y": 0, "width": 20, "height": 20})));
    let sub = id(run(&mut app, "layer.newSublayer", json!({"name": "Sub"})));
    let a = id(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})));
    let b = id(run(&mut app, "shape.rectangle", json!({"x": 100, "y": 0, "width": 20, "height": 20})));
    run(&mut app, "select.set", json!({"ids": [b.0]}));
    let g = id(run(&mut app, "object.group", json!({})));
    run(&mut app, "select.none", json!({}));
    expand(&mut app, &json!({})).unwrap();
    (app, [layer, sub, g, b, a, c])
}

pub(super) fn rows_of(app: &VectorcraftApp) -> Vec<NodeId> {
    app.session.active().unwrap().highlighted_rows()
}

/// The rows of an open document list by depth, and a ruler guide node gets no object row.
#[test]
fn rows_list_by_depth_and_ruler_guide_nodes_get_no_object_row() {
    let (app, [layer, sub, g, b, a, c]) = nested();
    assert_eq!(listed_ids(&app), [(layer, 0), (sub, 1), (g, 2), (b, 3), (a, 2), (c, 1)]);

    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let rect = NodeId(app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap());
    expand(&mut app, &json!({})).unwrap();
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    let (first, second) = (doc.layers[0].id, doc.add_layer(Some("Guides only")));
    for l in [first, second] {
        let g = Node::new(doc.alloc_id(), NodeKind::RulerGuide { vertical: true, pos: 20.0, artboard: None });
        doc.insert(Some(l), usize::MAX, g).unwrap();
    }
    let ids: Vec<NodeId> = listed_ids(&app).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, [second, first, rect]);
}

/// A horizontal and a vertical ruler guide's rows each show a glyph of their own, in place of the
/// guide kind's ruler, and both glyphs are in the app's icon set.
#[test]
fn horizontal_and_vertical_guide_rows_show_their_own_drawn_glyphs() {
    use craft::ui::primitives::{has_icon, register_app_icons};

    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": false, "pos": 40})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 60})).unwrap();
    let st = app.session.active().unwrap();
    let rows = LayerRows::new(&app, st);
    let icon_of = |vertical: bool| {
        let i = st.doc.ruler_guides().iter().position(|g| g.vertical == vertical).unwrap();
        rows.info(&RowId::Guide(i)).icon.unwrap_or_else(|| panic!("the vertical: {vertical} guide row has no glyph of its own"))
    };
    let (across, down) = (icon_of(false), icon_of(true));
    assert_ne!(across, down, "the two kinds of guide rows look alike");
    register_app_icons(&crate::icons::APP_ICONS);
    for icon in [&across, &down] {
        assert!(has_icon(icon), "{icon:?} is in no icon set, so the row draws nothing");
    }
}

/// The undo steps of the active document.
pub(super) fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// Cmd+Alt+L pressed through the key stage opens Layer Options for a new layer on the host, as
/// Alt-clicking New Layer does; OK adds the layer, Cancel adds none, and Cmd+L adds one with no
/// dialog.
#[test]
fn cmd_alt_l_opens_new_layer_options() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let ctx = app.ui_ctx.clone().unwrap();
    let layers = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers.len();
    // PLAN: assumed `Chord::parse(&str) -> Option<Chord>` and `Chord::to_egui_shortcut() -> egui::KeyboardShortcut`.
    let press = |app: &mut VectorcraftApp, chord: &str| {
        let shortcut = Chord::parse(chord).unwrap().to_egui_shortcut();
        let key = |pressed| egui::Event::Key { key: shortcut.logical_key, physical_key: None, pressed, repeat: false, modifiers: shortcut.modifiers };
        // A keyboard's tap: a second press with no release between is a repeat.
        crate::tests::synthetic::key_frame(app, &ctx, vec![key(true), key(false)]);
    };
    let open = |app: &VectorcraftApp| app.dialogs().map(|d| d.with(|host| host.open_dialogs())).unwrap_or_default();
    // The window's first frame builds its keymap.
    crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
    press(&mut app, "Cmd+Alt+L");
    let dialogs = open(&app);
    assert_eq!(dialogs.len(), 1, "one dialog opens");
    assert_eq!(dialogs[0].fields.as_ref().and_then(|f| f.get("mode")), Some(&json!("new")), "Layer Options for a new layer");
    assert_eq!(layers(&app), 1, "nothing is added before OK");
    answer(&mut app, &ctx, false);
    assert_eq!(layers(&app), 1, "Cancel adds none");
    press(&mut app, "Cmd+Alt+L");
    answer(&mut app, &ctx, true);
    assert_eq!(layers(&app), 2, "OK adds the layer");
    press(&mut app, "Cmd+L");
    assert!(open(&app).is_empty(), "Cmd+L opens no dialog");
    assert_eq!(layers(&app), 3, "and adds a layer");
}

/// `ui.layersFilter` reads its lists by name in the popup's order, keeps a list it is not given,
/// and changes nothing when a list holds anything else.
#[test]
fn the_filter_command_reads_its_lists_by_name() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let r = app.run("ui.layersFilter", json!({"objects": ["guides", "paths", "paths"], "layers": ["hidden"]})).unwrap();
    assert_eq!(r, json!({"objects": ["paths", "guides"], "layers": ["hidden"]}));
    let r = app.run("ui.layersFilter", json!({"layers": []})).unwrap();
    assert_eq!(r, json!({"objects": ["paths", "guides"], "layers": []}));
    for bad in [json!({"objects": ["paths", "layers"]}), json!({"objects": "paths"}), json!({"objects": [], "layers": ["visible"]})] {
        let e = app.run("ui.layersFilter", bad.clone()).unwrap_err();
        let key = if bad["layers"].is_null() { "objects" } else { "layers" };
        assert!(e.starts_with(&format!("`{key}` must be an array of these names: ")), "{bad}: {e}");
    }
    assert_eq!(app.run("ui.layersFilter", json!({})).unwrap(), json!({"objects": ["paths", "guides"], "layers": []}), "unchanged");
}

/// Others actions (Hide Others / Lock Others) retain all multiselected layers.
#[test]
fn context_menu_others_actions_preserve_multiselected_layers() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("layer.new", &json!({})).unwrap();
    app.session.execute("layer.new", &json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 3);
    let l1 = doc.layers[0].id.0;
    let l2 = doc.layers[1].id.0;

    // Hide Others with multiselection [l1, l2] hides only l3.
    let (hide_cmd, hide_params, hide_enabled) = super::rows::multi_others_action(doc, &[l1, l2], true);
    assert_eq!(hide_cmd, "layer.hideOthers");
    assert!(hide_enabled);
    assert_eq!(hide_params["ids"], json!([l1, l2]));

    // Lock Others with multiselection [l1, l2] locks only l3.
    let (lock_cmd, lock_params, lock_enabled) = super::rows::multi_others_action(doc, &[l1, l2], false);
    assert_eq!(lock_cmd, "layer.lockOthers");
    assert!(lock_enabled);
    assert_eq!(lock_params["ids"], json!([l1, l2]));
}

/// Context menu Others action resolves an object row to its holding top-level layer.
#[test]
fn context_menu_others_actions_object_row_resolves_to_holding_layer() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    // Layer 1 (current) holding a rectangle.
    let rect = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let rect_id = rect["id"].as_u64().unwrap();

    // Create Layer 2 and hide it.
    let l2 = app.session.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("layer.setProps", &json!({"ids": [l2], "visible": false})).unwrap();

    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 2);

    // Right-clicking the object in Layer 1: Layer 1 is visible, Layer 2 is hidden.
    // There are no other visible layers, so the action should be Show All Layers, enabled.
    let (hide_cmd, _hide_params, hide_enabled) = super::rows::multi_others_action(doc, &[rect_id], true);
    assert_eq!(hide_cmd, "layer.showAll");
    assert!(hide_enabled, "Show All Layers is enabled because Layer 2 is hidden");

    // Now make Layer 2 visible, but lock it.
    app.session.execute("layer.setProps", &json!({"ids": [l2], "visible": true, "locked": true})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    // Right-clicking the object in Layer 1: Layer 1 is unlocked, Layer 2 is locked.
    // There are no other unlocked layers, so the action should be Unlock All Layers, enabled.
    let (lock_cmd, _lock_params, lock_enabled) = super::rows::multi_others_action(doc, &[rect_id], false);
    assert_eq!(lock_cmd, "layer.unlockAll");
    assert!(lock_enabled, "Unlock All Layers is enabled because Layer 2 is locked");
}

/// Context menu Show/Unlock All Layers are enabled when sublayers are hidden or locked.
#[test]
fn context_menu_others_actions_sublayer_state_enables_all_commands() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    let l1 = doc.layers[0].id.0;

    // Create a sublayer in Layer 1.
    let sub = app.session.execute("layer.newSublayer", &json!({"parent": l1})).unwrap()["id"].as_u64().unwrap();

    // Case 1: Sublayer is hidden while top layer is visible.
    app.session.execute("layer.setProps", &json!({"ids": [sub], "visible": false})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let (hide_cmd, _, hide_enabled) = super::rows::multi_others_action(doc, &[l1], true);
    assert_eq!(hide_cmd, "layer.showAll");
    assert!(hide_enabled, "hidden sublayer enables Show All Layers");

    // Case 2: Sublayer is visible, but locked while top layer is unlocked.
    app.session.execute("layer.setProps", &json!({"ids": [sub], "visible": true, "locked": true})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let (lock_cmd, _, lock_enabled) = super::rows::multi_others_action(doc, &[l1], false);
    assert_eq!(lock_cmd, "layer.unlockAll");
    assert!(lock_enabled, "locked sublayer enables Unlock All Layers");
}

/// Names object `id` `name`, so its target circle is "Target {name}".
fn name_row(app: &mut VectorcraftApp, id: NodeId, name: &str) {
    app.session.execute("layer.setProps", &json!({"ids": [id.0], "name": name})).unwrap();
}

/// The target and the selected objects, sorted.
fn targeted(app: &VectorcraftApp) -> (Option<NodeId>, Vec<NodeId>) {
    let st = app.session.active().unwrap();
    let mut ids = st.selection.objects.clone();
    ids.sort();
    (st.selection.target, ids)
}

/// Shift-clicking a target circle selects the art of every row from the last circle clicked to
/// it, as Shift-clicking a name highlights a range of rows (#8639); Shift+Ctrl/Cmd adds a range to
/// the selection. Nothing is targeted, and hidden art in the range is left out.
#[test]
fn shift_clicking_a_target_circle_selects_a_range_of_rows() {
    use crate::tests::panel_harness::Panel;
    use egui::Modifiers;
    let (mut app, _, [a, b]) = two_rects();
    let rect = |app: &mut VectorcraftApp, x: i32, y: i32| {
        NodeId(app.session.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap())
    };
    let (c, d) = (rect(&mut app, 120, 0), rect(&mut app, 0, 60));
    for (id, name) in [(a, "A"), (b, "B"), (c, "C"), (d, "D")] {
        name_row(&mut app, id, name);
    }
    app.session.execute("select.none", &json!({})).unwrap();
    // Rows top down: the layer, D, C, B, A.
    let mut panel = Panel::new(app, show);
    panel.click_with("Target C", Modifiers::NONE);
    assert_eq!(targeted(&panel.app()), (Some(c), vec![c]));
    panel.click_with("Target A", Modifiers::SHIFT);
    assert_eq!(targeted(&panel.app()), (None, vec![a, b, c]));
    // A Shift range starts at the last circle clicked, not at the end of the last range.
    panel.click_with("Target B", Modifiers::SHIFT);
    assert_eq!(targeted(&panel.app()), (None, vec![b, c]));
    // Shift+Ctrl/Cmd adds a range: from C up to D.
    panel.click_with("Target D", Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(targeted(&panel.app()), (None, vec![b, c, d]));
    // Ctrl/Cmd takes one out again.
    panel.click_with("Target D", Modifiers::COMMAND);
    assert_eq!(targeted(&panel.app()), (None, vec![b, c]));
    // Hidden art in a range stays unselected.
    panel.app().session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    panel.app().session.execute("object.hide", &json!({})).unwrap();
    panel.click_with("Target C", Modifiers::NONE);
    panel.click_with("Target A", Modifiers::SHIFT);
    assert_eq!(targeted(&panel.app()), (None, vec![a, c]));
}

/// The Shift-click anchor of the target circles belongs to the document it was clicked in: node
/// ids are per document, so in another document a Shift-click with no circle clicked there yet
/// targets only its own row, never a range from a row that has the old anchor's id.
#[test]
fn the_target_circle_anchor_does_not_carry_over_to_another_document() {
    use crate::tests::panel_harness::Panel;
    let (mut app, _, [a, b]) = two_rects();
    name_row(&mut app, a, "A");
    name_row(&mut app, b, "B");
    let mut panel = Panel::new(app, show);
    panel.click_with("Target A", egui::Modifiers::NONE);
    let (a2, b2) = {
        let mut app = panel.app();
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 200, "height": 200}));
        let a2 = NodeId(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap());
        let b2 = NodeId(run(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap());
        name_row(&mut app, a2, "A");
        name_row(&mut app, b2, "B");
        run(&mut app, "select.none", json!({}));
        (a2, b2)
    };
    assert_eq!(a2, a, "the new document reuses the old anchor's id, so a stale anchor would find a row");
    panel.click_with("Target B", egui::Modifiers::SHIFT);
    assert_eq!(targeted(&panel.app()), (Some(b2), vec![b2]));
}
