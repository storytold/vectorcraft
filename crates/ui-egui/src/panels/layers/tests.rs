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
    fixture.root(crate::app::root_env(&vectorcraft_engine::Prefs::default(), 1.0, None)).frame(vec![]);
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

#[test]
fn layer_row_footer_drops_after_next_document_leave_the_new_document_alone() {
    use crate::tests::panel_harness::Panel;
    use craft::layers::ui::panel::{LayersDrag, document_hash};
    use craft::ui::contract::Role;
    use craft::ui::runtime::drag;
    use egui::{Key, Modifiers};

    for (button, deletes) in [("Delete Selection", true), ("Create New Layer", false)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let mut setup = |x| {
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let source = app.session.active().unwrap().doc.layers[0].id;
            name_row(&mut app, source, "Dragged Layer");
            app.run("shape.rectangle", json!({"x": x, "y": 20, "width": 40, "height": 30})).unwrap();
            app.run("layer.new", json!({"name": "Other Layer"})).unwrap();
            let st = app.session.active().unwrap();
            ((st.uid, st.reverts), source)
        };
        let (first_doc, first_layer) = setup(10);
        let (second_doc, second_layer) = setup(60);
        assert_eq!(first_layer, second_layer, "the documents have different layers with the same local ID");
        assert!(app.session.set_active(0));

        let mut panel = Panel::new(app, show);
        let from = panel.rect_role("Dragged Layer", Role::TREE_ITEM).center();
        let to = panel.rect(button).center();
        panel.drag_hold_role("Dragged Layer", Role::TREE_ITEM, to - from);
        assert!(drag::mark(panel.ctx()).is_some(), "{button} accepts a drag from its own list");
        panel.key(Key::F6, Modifiers::COMMAND);
        {
            let app = panel.app();
            let st = app.session.active().unwrap();
            assert_eq!((st.uid, st.reverts), second_doc);
        }
        let carried = drag::active::<LayersDrag<RowId>>(panel.ctx()).expect("the shortcut leaves the drag held");
        assert_eq!(carried.document(), Some(document_hash(first_doc)));
        assert!(drag::mark(panel.ctx()).is_none(), "{button} rejects a drag belonging to another document");
        panel.release();
        assert_eq!(panel.app().session.active().unwrap().doc.layers.len(), 2);
        assert!(crate::dialogs::confirm::open_question(&panel.app()).is_none());

        panel.drag("Dragged Layer", button, Modifiers::NONE);
        if deletes {
            assert!(crate::dialogs::confirm::open_question(&panel.app()).is_some(), "a same-document drop still asks to delete its layer");
        } else {
            assert_eq!(panel.app().session.active().unwrap().doc.layers.len(), 3, "a same-document drop still duplicates its layer");
        }
    }
}

/// The footer's layer count is in the accessibility tree, and it follows the document's layers.
#[test]
fn the_footer_counts_the_layers() {
    use crate::tests::panel_harness::Panel;
    let (app, _, _) = two_rects();
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("1 Layer"));
    panel.app().session.execute("layer.new", &json!({})).unwrap();
    panel.frame();
    assert!(panel.shows("2 Layers") && !panel.shows("1 Layer"));
}

/// Names object `id` `name`, so its target circle is "Target {name}".
fn name_row(app: &mut VectorcraftApp, id: NodeId, name: &str) {
    app.session.execute("layer.setProps", &json!({"ids": [id.0], "name": name})).unwrap();
}

#[test]
fn a_selection_square_drop_after_next_document_leaves_the_new_artwork_in_place() {
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use craft::ui::runtime::drag;
    use egui::{Key, Modifiers};

    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let mut setup = |x| {
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let source = app.session.active().unwrap().doc.layers[0].id;
        name_row(&mut app, source, "Source");
        let id = app.run("shape.rectangle", json!({"x": x, "y": 20, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap();
        let dst = app.run("layer.new", json!({"name": "Destination"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let st = app.session.active().unwrap();
        ((st.uid, st.reverts), NodeId(id), source, NodeId(dst))
    };
    let (first_doc, first, _, _) = setup(10);
    let (second_doc, second, source, destination) = setup(60);
    assert_eq!(first, second, "the documents have different rectangles with the same local ID");
    assert!(app.session.set_active(0));

    let mut panel = Panel::new(app, show);
    let from = panel.rect_role("Select art of Source", Role::BUTTON).center();
    let to = panel.rect("Destination").center();
    panel.drag_hold_role("Select art of Source", Role::BUTTON, to - from);
    assert!(drag::mark(panel.ctx()).is_some(), "the destination accepts artwork in its own document");
    panel.key(Key::F6, Modifiers::COMMAND);
    {
        let app = panel.app();
        let st = app.session.active().unwrap();
        assert_eq!((st.uid, st.reverts), second_doc);
    }
    let carried = drag::active::<cells::ArtDrag>(panel.ctx()).expect("the shortcut leaves the drag held");
    assert_eq!((carried.document, carried.ids.as_slice()), (first_doc, &[first][..]));
    assert!(drag::mark(panel.ctx()).is_none(), "the destination rejects the foreign artwork");
    panel.release();
    let layer = |app: &VectorcraftApp| app.session.active().unwrap().doc.layer_of(second);
    assert_eq!(layer(&panel.app()), Some(source), "the same-ID rectangle in this document has not moved");

    panel.drag("Select art of Source", "Destination", Modifiers::NONE);
    assert_eq!(layer(&panel.app()), Some(destination), "a same-document selection-square drag still moves the rectangle");
}

#[test]
fn a_selection_square_drop_after_file_revert_leaves_the_restored_artwork_in_place() {
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use craft::ui::runtime::drag;
    use egui::Modifiers;

    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let source = app.session.active().unwrap().doc.layers[0].id;
    name_row(&mut app, source, "Source");
    let rect = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let destination = NodeId(app.run("layer.new", json!({"name": "Destination"})).unwrap()["id"].as_u64().unwrap());
    let folder = vectorcraft_testkit::TempDir::new("layers-art-drag-revert");
    let path = folder.join("drawing.vectorcraft");
    app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|err| err.to_string())));
    app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|err| err.to_string())));
    app.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
    app.run("shape.rectangle", json!({"x": 120, "y": 30, "width": 30, "height": 30})).unwrap();
    app.run("select.set", json!({"ids": [rect.0]})).unwrap();

    let mut panel = Panel::new(app, show);
    let from = panel.rect_role("Select art of Source", Role::BUTTON).center();
    let to = panel.rect("Destination").center();
    panel.drag_hold_role("Select art of Source", Role::BUTTON, to - from);
    assert!(drag::mark(panel.ctx()).is_some(), "the destination accepts the selected artwork before the revert");
    let opening = {
        let app = panel.app();
        let st = app.session.active().unwrap();
        (st.uid, st.reverts)
    };

    panel.app().run("file.revert", json!({"confirmed": true})).unwrap();
    panel.frame();
    {
        let app = panel.app();
        let st = app.session.active().unwrap();
        assert_eq!((st.uid, st.reverts), (opening.0, opening.1 + 1));
    }
    assert!(drag::active::<cells::ArtDrag>(panel.ctx()).is_some(), "the revert leaves the drag held");
    assert!(drag::mark(panel.ctx()).is_none(), "the destination rejects artwork carried from before the revert");
    panel.release();
    let layer = |app: &VectorcraftApp| app.session.active().unwrap().doc.layer_of(rect);
    assert_eq!(layer(&panel.app()), Some(source), "the restored rectangle with the same ID has not moved");

    panel.app().run("select.set", json!({"ids": [rect.0]})).unwrap();
    panel.frame();
    panel.drag("Select art of Source", "Destination", Modifiers::NONE);
    assert_eq!(layer(&panel.app()), Some(destination), "a selection-square drag started after the revert still moves the rectangle");
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

/// Shift-clicking a selection square selects the art of every row from the last square clicked to
/// it, down or up and across a sublayer and a group, leaving out locked and hidden rows, and
/// Shift+Ctrl/Cmd adds that art. A plain click selects one row's art, and Ctrl/Cmd adds it or takes
/// it out again.
#[test]
fn shift_clicking_a_selection_square_selects_the_art_of_a_range_of_rows() {
    use crate::tests::panel_harness::Panel;
    use egui::Modifiers;
    let (mut app, [_, _, g, b, a, c]) = nested();
    for (id, name) in [(g, "G"), (b, "B"), (a, "A"), (c, "C")] {
        name_row(&mut app, id, name);
    }
    // Rows top down: the layer, the sublayer, G, B (in G), A, then C on the layer itself.
    let mut panel = Panel::new(app, show);
    panel.click_with("Select art of A", Modifiers::NONE);
    assert_eq!(targeted(&panel.app()), (None, vec![a]), "a plain click selects one row's art");
    panel.click_with("Select art of C", Modifiers::SHIFT);
    let (_, sel) = targeted(&panel.app());
    assert!(sel.len() == 2 && sel.contains(&a) && sel.contains(&c), "a range down, out of the sublayer: {sel:?}");
    // A range starts at the last square clicked, not at the end of the last range.
    panel.click_with("Select art of G", Modifiers::SHIFT);
    let (_, sel) = targeted(&panel.app());
    assert!(sel.contains(&g) && sel.contains(&a) && !sel.contains(&c), "a range up, across the group: {sel:?}");
    // Shift+Ctrl/Cmd adds a range: from A down to C.
    panel.click_with("Select art of C", Modifiers::SHIFT | Modifiers::COMMAND);
    let (_, sel) = targeted(&panel.app());
    assert!(sel.contains(&g) && sel.contains(&a) && sel.contains(&c), "{sel:?}");
    // Ctrl/Cmd takes one row's art out again.
    panel.click_with("Select art of C", Modifiers::COMMAND);
    let (_, sel) = targeted(&panel.app());
    assert!(sel.contains(&g) && !sel.contains(&c), "{sel:?}");
    // Locked and hidden rows in a range stay unselected.
    panel.app().session.execute("layer.setProps", &json!({"ids": [a.0], "locked": true})).unwrap();
    panel.app().session.execute("layer.setProps", &json!({"ids": [b.0], "visible": false})).unwrap();
    panel.click_with("Select art of G", Modifiers::NONE);
    panel.click_with("Select art of C", Modifiers::SHIFT);
    let (_, sel) = targeted(&panel.app());
    assert!(sel.contains(&g) && sel.contains(&c) && !sel.contains(&a) && !sel.contains(&b), "{sel:?}");
}

/// With no selection square clicked yet, Shift-clicking one adds its row's art, or takes it out
/// when it is all selected, and that square becomes where the next Shift-click starts.
#[test]
fn shift_clicking_the_first_selection_square_adds_or_removes_one_row() {
    use crate::tests::panel_harness::Panel;
    use egui::Modifiers;
    let (mut app, [_, _, g, _, a, c]) = nested();
    for (id, name) in [(g, "G"), (a, "A"), (c, "C")] {
        name_row(&mut app, id, name);
    }
    app.session.execute("select.set", &json!({"ids": [a.0, c.0]})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.click_with("Select art of A", Modifiers::SHIFT);
    assert_eq!(targeted(&panel.app()), (None, vec![c]), "a selected row's art is taken out");
    panel.click_with("Select art of C", Modifiers::SHIFT);
    let (_, sel) = targeted(&panel.app());
    assert!(sel.contains(&a) && sel.contains(&c) && !sel.contains(&g), "the range from A to C: {sel:?}");
}

/// Each document keeps the selection square clicked last in it: after a click in a second
/// document, a Shift-click back in the first selects the range from the first document's square.
#[test]
fn each_document_keeps_its_selection_square_anchor() {
    use crate::tests::panel_harness::Panel;
    use egui::Modifiers;
    let (mut app, _, [a, b]) = two_rects();
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    let c = NodeId(run(&mut app, "shape.rectangle", json!({"x": 120, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap());
    for (id, name) in [(a, "A"), (b, "B"), (c, "C")] {
        name_row(&mut app, id, name);
    }
    run(&mut app, "select.none", json!({}));
    let mut panel = Panel::new(app, show);
    panel.click_with("Select art of A", Modifiers::NONE);
    {
        let mut app = panel.app();
        run(&mut app, "file.new", json!({"width": 200, "height": 200}));
        let other = NodeId(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap());
        name_row(&mut app, other, "D");
        run(&mut app, "select.none", json!({}));
    }
    panel.click_with("Select art of D", Modifiers::NONE);
    {
        let mut app = panel.app();
        run(&mut app, "document.activate", json!({"index": 0}));
        run(&mut app, "select.none", json!({}));
    }
    panel.click_with("Select art of C", Modifiers::SHIFT);
    assert_eq!(targeted(&panel.app()), (None, vec![a, b, c]), "the range from A, the first document's square, to C");
}

/// The rows among `rows` whose selection square is full-sized, then those whose square is the
/// small one, in the order given.
fn squares(app: &VectorcraftApp, rows: &[NodeId]) -> (Vec<NodeId>, Vec<NodeId>) {
    let st = app.session.active().unwrap();
    let tree = DocLayers::new(app, st);
    let (mut full, mut small) = (vec![], vec![]);
    for &id in rows {
        match cells::square_marks(&tree, st.doc.node(id).unwrap()) {
            (true, _) => full.push(id),
            (false, true) => small.push(id),
            (false, false) => {}
        }
    }
    (full, small)
}

/// A layer or sublayer whose selectable art is all selected shows the full-sized square; one
/// holding some of its art, or a group's member instead of the group, shows the small one.
#[test]
fn a_layer_with_all_its_art_selected_shows_the_full_square() {
    use crate::tests::panel_harness::Panel;
    use egui::Modifiers;
    let (mut app, [layer, sub, g, b, a, c]) = nested();
    let rows = [layer, sub, g, b, a, c];
    name_row(&mut app, layer, "Top");
    name_row(&mut app, sub, "Sub");
    name_row(&mut app, a, "A");
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(squares(&app, &rows), (vec![a], vec![layer, sub]), "the rectangle's square, small ones on the sublayer and the layer");
    app.session.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!(squares(&app, &rows), (vec![b, a], vec![layer, sub, g]), "a group's member is not the group: the sublayer's square stays small");
    let mut panel = Panel::new(app, show);
    panel.click_with("Select art of Sub", Modifiers::NONE);
    assert_eq!(squares(&panel.app(), &rows), (vec![sub, g, a], vec![layer]), "the sublayer, its group and rectangle full, the layer small");
    panel.click_with("Select art of Top", Modifiers::NONE);
    assert_eq!(squares(&panel.app(), &rows), (vec![layer, sub, g, a, c], vec![]), "the layer and the sublayer full, with each row's art");
    panel.click_with("Select art of A", Modifiers::COMMAND);
    assert_eq!(squares(&panel.app(), &rows), (vec![g, c], vec![layer, sub]), "one rectangle let go: the layer and the sublayer small again");
    // Locked art cannot be selected, so it does not keep the layer's square small.
    panel.app().session.execute("layer.setProps", &json!({"ids": [c.0], "locked": true})).unwrap();
    panel.click_with("Select art of Sub", Modifiers::NONE);
    assert_eq!(squares(&panel.app(), &rows), (vec![layer, sub, g, a], vec![]), "the layer, the sublayer, the group and the rectangle");
    // Hidden art neither.
    panel.app().session.execute("layer.setProps", &json!({"ids": [c.0], "locked": false, "visible": false})).unwrap();
    panel.app().session.execute("select.set", &json!({"ids": [a.0, g.0]})).unwrap();
    assert_eq!(squares(&panel.app(), &rows), (vec![layer, sub, g, a], vec![]), "hidden art is left out");
    // With nothing targeted, the full layers' target circles show as targeted too.
    let app = panel.app();
    let st = app.session.active().unwrap();
    let tree = DocLayers::new(&app, st);
    assert!(rows::shows_targeted(&tree, st.doc.node(layer).unwrap(), false), "the layer shows as targeted");
}

/// A layer with no selectable art shows no square, whatever else is selected.
#[test]
fn a_layer_without_art_shows_no_square() {
    let (mut app, [layer, sub, g, b, a, c]) = nested();
    let empty = NodeId(app.session.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    app.session.execute("select.set", &json!({"ids": [c.0, a.0, g.0]})).unwrap();
    let rows = [empty, layer, sub, g, b, a, c];
    assert_eq!(squares(&app, &rows), (vec![layer, sub, g, a, c], vec![]), "the first layer's rows; the new, empty layer's none");
}

/// The panel's set of full layers is the layers and sublayers whose `layer_art(true)` is not empty
/// and all selected, for every selection of the document's objects, hidden and locked ones included.
#[test]
fn full_layers_are_the_layers_with_all_their_art_selected() {
    let (mut app, [_, _, _, _, _, c]) = nested();
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
    let hidden = id(run(&mut app, "layer.newSublayer", json!({"name": "Hidden"})));
    run(&mut app, "shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20}));
    run(&mut app, "layer.setProps", json!({"ids": [hidden.0], "visible": false}));
    run(&mut app, "layer.setProps", json!({"ids": [c.0], "locked": true}));
    run(&mut app, "layer.new", json!({}));
    let doc = app.session.active().unwrap().doc.clone();
    let (mut layers, mut objects) = (vec![], vec![]);
    for l in &doc.layers {
        l.walk(&mut |n| if n.is_layer() { layers.push(n) } else { objects.push(n.id) });
    }
    assert_eq!((layers.len(), objects.len()), (4, 5), "the layers, the sublayers and the objects");
    for bits in 0u32..(1 << objects.len()) {
        let sel: HashSet<NodeId> = objects.iter().enumerate().filter(|(i, _)| (bits >> i) & 1 == 1).map(|(_, id)| *id).collect();
        let want: HashSet<NodeId> = layers
            .iter()
            .filter(|l| {
                let art = l.layer_art(true);
                !art.is_empty() && art.iter().all(|i| sel.contains(i))
            })
            .map(|l| l.id)
            .collect();
        assert_eq!(rows::full_layers(&doc, &sel), want, "selection {bits:b}");
    }
}
