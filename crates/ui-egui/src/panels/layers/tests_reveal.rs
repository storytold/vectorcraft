//! The Layers panel's rename, isolation, reveal and thumbnail behaviour, driven through commands, the panel's events
//! and one pass of `show`; nothing here reads painted text.

use craft::behavior::{EditEvent, Mixed, SelectionChange};
use craft::keymap::{CommandRef, Mods};
use craft::layers::ui::filter::FilterObject;
use craft::layers::ui::panel::{LayerColumn, LayerDrop, LayerMove, LayerPosition, LayerSelect, LayersEvent};
use craft::layers::ui::tree::{LayerRowKey, LayerTree};
use craft::ui::contract::ReorderMove;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use egui::vec2;
use serde_json::json;
use vectorcraft_engine::{LayersReveal, Session};

use super::tests::{listed_ids, nested, rows_of, two_rects, undo_len};
use super::*;

/// One pass of the panel in a 300 × 800 window, one second after the previous one.
fn pass(app: &mut VectorcraftApp, ctx: &egui::Context) {
    pass_then(app, ctx, |_| ());
}

/// [`pass`], then `read` at the end of the pass: what it returns.
fn pass_then<T>(app: &mut VectorcraftApp, ctx: &egui::Context, read: impl FnOnce(&egui::Context) -> T) -> T {
    let time = ctx.input(|i| i.time) + 1.0;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 800.0));
    let mut read = Some(read);
    let mut got = None;
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() }, |ui| {
        show(app, ui);
        got = read.take().map(|read| read(ui.ctx()));
    });
    out.textures_delta.clear();
    got.expect("the pass ran")
}

/// The uploads of document `uid`'s Layers thumbnails in one [`pass`]. The store counts the uploads of the frame that
/// runs, so they are read at the end of the pass, before the next frame starts.
fn pass_uploads(app: &mut VectorcraftApp, ctx: &egui::Context, uid: u64) -> usize {
    pass_then(app, ctx, |ctx| stored(ctx, uid).1)
}

/// The active document's pending reveal request.
fn reveal_of(app: &VectorcraftApp) -> Option<LayersReveal> {
    app.session.active().and_then(|st| st.layers_reveal)
}

/// The panel options of the app.
fn options(app: &mut VectorcraftApp) -> &mut PanelOptions {
    &mut app.ui.layers_panel
}

fn is_open(app: &VectorcraftApp, id: NodeId) -> bool {
    app.session.active().is_some_and(|st| st.layers_open.contains(id))
}

fn parent(app: &VectorcraftApp, id: NodeId) -> Option<NodeId> {
    app.session.active().and_then(|st| st.doc.parent_of(id))
}

fn node_count(app: &VectorcraftApp) -> usize {
    let mut n = 0;
    if let Some(st) = app.session.active() {
        st.doc.walk(|_| n += 1);
    }
    n
}

fn row(id: NodeId) -> RowId {
    RowId::Node(id)
}

fn key(id: NodeId) -> LayerRowKey<RowId> {
    LayerRowKey::Layer(row(id))
}

/// Applies a panel event as the panel's output stage does.
fn apply(app: &mut VectorcraftApp, event: LayersEvent<RowId>) {
    rows::apply(app, vec![event]);
}

fn select_rows(app: &mut VectorcraftApp, ids: &[NodeId]) {
    let change = SelectionChange::Set(ids.iter().map(|id| row(*id)).collect());
    apply(app, LayersEvent::Select(LayerSelect::new(change, None)));
}

fn mv(layers: &[NodeId], target: NodeId, position: LayerPosition, copy: bool) -> LayerMove<RowId> {
    LayerMove::new(layers.iter().map(|id| row(*id)).collect(), row(target), position, copy)
}

/// The rows of the active document under the app's Panel Options.
fn doc_layers(app: &VectorcraftApp) -> DocLayers<'_> {
    DocLayers::new(app, app.session.active().expect("a document"))
}

/// Whether the panel takes the drop `m`: its rows landing in the group `m` puts them in, as the list reports it.
fn accepts(app: &VectorcraftApp, m: &LayerMove<RowId>) -> bool {
    let tree = doc_layers(app);
    let parent = match m.position {
        LayerPosition::Into => Some(m.target),
        _ => tree.parent(&m.target),
    };
    let keys: Vec<LayerRowKey<RowId>> = m.layers.iter().map(|id| LayerRowKey::Layer(*id)).collect();
    let reorder = ReorderMove { key: keys[0].clone(), keys, to: 0, copy: m.copy, parent: parent.map(LayerRowKey::Layer) };
    tree.accepts_drop(&reorder, Mods::NONE)
}

fn drop_event(m: LayerMove<RowId>) -> LayersEvent<RowId> {
    let drop = LayerDrop::Layers(m);
    LayersEvent::Drop(EditEvent::set(Mixed::Same(drop.clone()), drop, "Move".into(), None))
}

/// Every row the panel lists under the app's Panel Options, top down, depth first.
fn listed(app: &VectorcraftApp) -> Vec<NodeId> {
    fn walk(layers: &DocLayers, parent: Option<&RowId>, out: &mut Vec<NodeId>) {
        for child in layers.children(parent) {
            if let RowId::Node(id) = child {
                out.push(id);
            }
            walk(layers, Some(&child), out);
        }
    }
    let mut out = vec![];
    walk(&doc_layers(app), None, &mut out);
    out
}

/// Whether the panel menu's row of `command` is enabled.
fn enabled(app: &VectorcraftApp, command: &str) -> bool {
    menu_states(app).get(&menu_command(app, command)).is_some_and(CommandState::is_enabled)
}

/// The panel menu's command states.
fn menu_states(app: &VectorcraftApp) -> CommandStates {
    let mut states = CommandStates::default();
    controls::menu_states(app, &mut states);
    states
}

/// Whether the panel menu titles Locate `title`.
fn locate_titled(app: &VectorcraftApp, title: &str) -> bool {
    let locate = menu_command(app, "layer.locate");
    menu_states(app).get(&locate).is_some_and(|s| *s == CommandState::new().enabled(s.is_enabled()).title(title))
}

/// A menu command run as the panel menu runs it: nothing for a row that is off, else the panel's own commands, else
/// the app's.
fn menu_run(app: &mut VectorcraftApp, command: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let row = CommandRef::with_params(command, params.clone());
    if menu_states(app).get(&row).is_some_and(|s| !s.is_enabled()) {
        return Err(format!("{command} is off"));
    }
    match controls::run(app, command, &params) {
        Some(answer) => answer,
        None => app.run(command, params),
    }
}

/// The panel menu's row of `command`, with the parameters it runs it with: the menu gives the row the state set for
/// it.
fn menu_command(app: &VectorcraftApp, command: &str) -> CommandRef {
    let row = controls::menu_rows(app).into_iter().find_map(|r| match r {
        PanelRow::Command(c) if c.id == command => Some(c),
        _ => None,
    });
    row.expect("the panel menu has the command")
}

/// Presses the footer's Locate: runs the command its pick gives.
fn footer_locate(app: &mut VectorcraftApp) {
    let (id, params) = controls::footer_pick(&doc_layers(app), controls::LOCATE, Mods::NONE).expect("Locate runs a command");
    app.run(&id, params).unwrap();
}

/// What a document's Layers thumbnail store holds and what it uploaded in the running frame: `(entries, uploads)`. It
/// reads the store's statistics, which neither render nor queue a picture.
fn stored(ctx: &egui::Context, uid: u64) -> (usize, usize) {
    // PLAN: assumed `thumbs::layer_thumbnails(&Context, u64)` returns the `Thumbnails` of that document
    let stats = thumbs::layer_thumbnails(ctx, uid).cache().statistics(ctx);
    (stats.entries, stats.uploads)
}

#[test]
fn closing_a_document_releases_only_its_layer_thumbnails() {
    let (mut app, _, _) = two_rects();
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    let first = app.session.active().unwrap().uid;
    pass(&mut app, &ctx);
    assert!(stored(&ctx, first).0 > 0, "the first document's rows render");

    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let second = app.session.active().unwrap().uid;
    pass(&mut app, &ctx);
    assert!(stored(&ctx, second).0 > 0);
    // The app's frames see both documents open.
    crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);

    // The engine closes without the question the window asks; the whole frame drops the view of the closed document.
    app.session.execute("file.close", &json!({"index": 0})).unwrap();
    crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
    assert!(app.session.documents().iter().all(|d| d.uid != first), "the first document closed");
    assert_eq!(stored(&ctx, first).0, 0, "the closed document's pictures are released");
    assert!(stored(&ctx, second).0 > 0, "the open document's pictures stay");
}

#[test]
fn row_thumbnails_wait_for_the_end_of_a_drag() {
    let (mut app, _, _) = two_rects();
    app.session.execute("select.all", &json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    let uid = app.session.active().unwrap().uid;
    for _ in 0..4 {
        pass(&mut app, &ctx);
    }
    assert!(stored(&ctx, uid).0 > 0, "the rows' pictures render at once");
    app.session.begin_interaction("Move").unwrap();
    for i in 1..=10 {
        app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, i, 0]})).unwrap();
        assert_eq!(pass_uploads(&mut app, &ctx, uid), 0, "no picture is replaced while the drag lasts");
    }
    app.session.commit_interaction().unwrap();
    let uploads: usize = (0..4).map(|_| pass_uploads(&mut app, &ctx, uid)).sum();
    assert!(uploads > 0, "the moved rows render after it");
    assert_eq!(pass_uploads(&mut app, &ctx, uid), 0, "once");
}

/// Rename (`ui.rename`) opens the name field on the selected object's row: the panel takes the request in one pass and
/// the committed name is one undo step.
#[test]
fn rename_opens_the_name_field_of_the_selected_object() {
    let (mut app, _, [a, _]) = two_rects();
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    let name = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(a).unwrap().display_name();
    let before = name(&app);
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let steps = undo_len(&app);
    let opened = app.run("ui.rename", json!({})).unwrap();
    assert_eq!(opened, json!({"id": a.0, "name": before}));
    assert_eq!(app.ui.layers_ui.rename, Some(row(a)), "the field is asked for on its row");
    assert_eq!(reveal_of(&app), Some(LayersReveal::Row(a)), "and the row is brought into view");
    pass(&mut app, &ctx);
    assert_eq!(app.ui.layers_ui.rename, None, "the panel took the rename request");
    assert_eq!(reveal_of(&app), None, "and the reveal request");
    apply(&mut app, LayersEvent::rename(row(a), "Sun"));
    assert_eq!(name(&app), "Sun");
    assert_eq!(undo_len(&app), steps + 1, "one undo step");
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(name(&app), before);
}

/// Without a selection Rename names the current layer; it refuses several objects and a row the panel doesn't list,
/// and asks for no field then.
#[test]
fn rename_names_the_current_layer_and_refuses_several_or_hidden_rows() {
    let (mut app, layer, [a, b]) = two_rects();
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    assert_eq!(app.run("ui.rename", json!({})).unwrap()["id"], json!(layer.0), "nothing selected: the current layer");
    app.ui.layers_ui.rename = None;
    app.session.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    assert!(app.run("ui.rename", json!({})).is_err(), "two objects");
    assert_eq!(app.ui.layers_ui.rename, None);
    options(&mut app).layers_only = true;
    assert!(app.run("ui.rename", json!({"id": a.0})).is_err(), "Show Layers Only lists no object rows");
    assert_eq!(app.ui.layers_ui.rename, None);
    assert!(app.run("ui.rename", json!({"id": 999_999})).is_err(), "no such object");
    assert_eq!(app.ui.layers_ui.rename, None);
    assert_eq!(app.run("ui.rename", json!({"id": layer.0})).unwrap()["id"], json!(layer.0));
    assert_eq!(app.ui.layers_ui.rename, Some(row(layer)));
}

/// In isolation mode the panel lists only the isolated sublayer, group or object, as the top row with the open rows
/// inside it; the other layers and groups are listed again when it ends.
#[test]
fn isolation_lists_only_the_isolated_art_and_exit_lists_every_layer_again() {
    let (mut app, [layer, sub, g, b, a, c]) = nested();
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    let top = NodeId(run(&mut app, "layer.new", json!({"top": true, "name": "Top"}))["id"].as_u64().unwrap());
    let all = vec![(top, 0), (layer, 0), (sub, 1), (g, 2), (b, 3), (a, 2), (c, 1)];
    assert_eq!(listed_ids(&app), all);
    run(&mut app, "object.isolate", json!({"id": g.0}));
    assert_eq!(listed_ids(&app), [(g, 0), (b, 1)], "the group and its art");
    run(&mut app, "object.exitIsolation", json!({}));
    assert_eq!(listed_ids(&app), all, "every layer again");
    run(&mut app, "object.isolate", json!({"id": sub.0}));
    assert_eq!(listed_ids(&app), [(sub, 0), (g, 1), (b, 2), (a, 1)], "the sublayer and its art");
    // Isolating an object inside lists that object alone.
    run(&mut app, "object.isolate", json!({"id": a.0}));
    assert_eq!(listed_ids(&app), [(a, 0)]);
    run(&mut app, "object.exitIsolation", json!({}));
    assert_eq!(listed_ids(&app), all);
    // A closed isolated group lists its own row alone, and an isolated object that is gone leaves every layer listed.
    run(&mut app, "object.isolate", json!({"id": g.0}));
    app.session.active_mut().unwrap().layers_open.set(g, false);
    assert_eq!(listed_ids(&app), [(g, 0)]);
    app.session.active_mut().unwrap().isolation = Some(NodeId(999_999));
    assert_eq!(listed_ids(&app), [(top, 0), (layer, 0), (sub, 1), (g, 2), (a, 2), (c, 1)]);
}

/// Entering isolation mode opens the isolated row and leaving it lists every row again; a drop beside the isolated row
/// or into the empty space under it moves nothing out of the isolated art.
#[test]
fn isolation_opens_the_isolated_row_and_keeps_drops_inside_it() {
    let (mut app, [layer, sub, g, _, a, _]) = nested();
    assert_eq!(listed_ids(&app).len(), 6);
    app.session.active_mut().unwrap().layers_open.set(sub, false);
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    assert!(is_open(&app, sub), "the sublayer's row opens as isolation starts");
    assert_eq!(listed_ids(&app).len(), 4, "Sub, g, b, a");
    // Rectangle a above the sublayer's row would leave the sublayer.
    let above = mv(&[a], sub, LayerPosition::Before, false);
    assert!(!accepts(&app, &above));
    apply(&mut app, drop_event(above));
    assert_eq!(parent(&app, a), Some(sub));
    // The sublayer under the last row (the empty space) stays in Layer 1.
    let under = mv(&[sub], a, LayerPosition::After, false);
    assert!(!accepts(&app, &under));
    apply(&mut app, drop_event(under));
    assert_eq!(parent(&app, sub), Some(layer));
    // Rectangle a onto the group's row: into it.
    let into = mv(&[a], g, LayerPosition::Into, false);
    assert!(accepts(&app, &into));
    apply(&mut app, drop_event(into));
    assert_eq!(parent(&app, a), Some(g));
    app.session.execute("object.exitIsolation", &json!({"all": true})).unwrap();
    assert_eq!(listed_ids(&app).len(), 6, "every row again");
}

/// In isolation mode the rows outside the isolated art are no longer highlighted.
#[test]
fn isolation_drops_highlights_outside_the_isolated_art() {
    let (mut app, [_, sub, _, b, _, c]) = nested();
    let ctx = crate::testing::ui_context();
    app.session.execute("layer.highlight", &json!({"ids": [c.0, b.0], "mode": "set"})).unwrap();
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    pass(&mut app, &ctx);
    assert_eq!(rows_of(&app), [b], "the rectangle on Layer 1, outside the sublayer, is let go");
}

/// In isolation mode Rename and the rows opened around a located object stay inside the isolated row.
#[test]
fn isolation_keeps_rename_and_locate_to_the_rows_listed() {
    let (mut app, [layer, sub, g, b, _, c]) = nested();
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    let err = app.run("ui.rename", json!({"id": c.0})).unwrap_err();
    assert!(err.contains("the object is outside the isolated group"), "{err}");
    assert_eq!(app.run("ui.rename", json!({"id": b.0})).unwrap()["id"], json!(b.0));
    {
        let tree = doc_layers(&app);
        let listed = |id| listed_when_opened(&tree, &LayerFilter::default(), id);
        assert_eq!([layer, sub, g, b, c].map(listed), [false, true, true, true, false]);
    }
    let st = app.session.active_mut().unwrap();
    st.layers_open.set(layer, false);
    st.layers_open.set(g, false);
    let doc = st.doc.clone();
    open_around(&mut app, &doc, &[c]);
    open_around(&mut app, &doc, &[b]);
    assert!(is_open(&app, g), "the group inside the sublayer opens");
    assert!(!is_open(&app, layer), "the layer around the sublayer stays closed");
}

/// In isolation mode, with no row highlighted, Delete Selection and the trash do not delete the current layer when it is
/// outside the isolated art, where the panel does not list it.
#[test]
fn isolation_keeps_delete_to_the_rows_listed() {
    let (mut app, [layer, sub, g, b, a, c]) = nested();
    app.session.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let st = app.session.active_mut().unwrap();
    st.layer_rows = vec![c];
    let current = st.current_layer().unwrap();
    assert!([layer, sub].contains(&current), "the current layer holds the isolated group");
    let err = app.run("ui.layerDelete", json!({})).unwrap_err();
    assert!(err.contains("the object is outside the isolated group"), "{err}");
    let doc = &app.session.active().unwrap().doc;
    assert!([layer, sub, g, b, a, c].iter().all(|id| doc.node(*id).is_some()), "nothing was deleted");
    app.session.active_mut().unwrap().layer_rows = vec![b];
    app.run("ui.layerDelete", json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.node(b).is_none(), "a highlighted row inside the isolated group is deleted");
    assert!(doc.node(a).is_some() && doc.node(c).is_some());
}

/// In isolation mode New Layer and New Sublayer are off, and `ui.newLayer` is an error, while the layer they make would
/// be outside the isolated art; a sublayer of an isolated sublayer is listed.
#[test]
fn isolation_keeps_new_layers_to_the_rows_listed() {
    let (app, [_, sub, g, ..]) = nested();
    let mut app = crate::testing::windowed(app);
    app.session.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!([(false, false), (true, false), (false, true)].map(|(s, t)| new_layer_unlisted(st, s, t)), [true; 3]);
    for p in [json!({}), json!({"sublayer": true})] {
        let err = app.run("ui.newLayer", p).unwrap_err();
        assert!(err.contains("exit isolation mode first"), "{err}");
    }
    assert!(!app.any_dialog_open());
    app.session.execute("object.exitIsolation", &json!({})).unwrap();
    app.session.execute("layer.setCurrent", &json!({"id": sub.0})).unwrap();
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    let st = app.session.active().unwrap();
    assert!(new_layer_unlisted(st, false, false), "a new layer would go beside the isolated sublayer");
    assert!(!new_layer_unlisted(st, true, false), "a new sublayer goes inside it");
    app.run("ui.newLayer", json!({"sublayer": true})).unwrap();
    assert!(app.any_dialog_open());
    // Exit Isolation Mode on the sublayer isolates its layer, beside which a new layer would go.
    app.session.execute("object.exitIsolation", &json!({})).unwrap();
    assert!(new_layer_unlisted(app.session.active().unwrap(), false, true));
    app.session.execute("object.exitIsolation", &json!({})).unwrap();
    assert!(!new_layer_unlisted(app.session.active().unwrap(), false, true));
}

/// With Show Layers Only on, an isolated group or object is still listed as the top row, without the art inside it.
#[test]
fn isolation_with_show_layers_only_lists_the_isolated_group_or_object() {
    let (mut app, [_, sub, g, _, a, _]) = nested();
    options(&mut app).layers_only = true;
    app.session.execute("object.isolate", &json!({"id": g.0})).unwrap();
    assert_eq!(listed(&app), [g], "the group's row, without its art");
    assert!(listed_when_opened(&doc_layers(&app), &LayerFilter::default(), g));
    app.session.execute("object.isolate", &json!({"id": a.0})).unwrap();
    assert_eq!(listed(&app), [a]);
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    assert_eq!(listed(&app), [sub], "a sublayer with no sublayers inside lists itself alone");
}

/// In isolation mode the panel menu's Duplicate Selection copies only the rows the panel lists, Hide Others is off, and
/// with a group isolated so is Locate Layer, whose layer row is not listed.
#[test]
fn isolation_keeps_the_panel_menu_to_the_rows_listed() {
    let (mut app, [layer, sub, g, b, _, c]) = nested();
    let top = NodeId(app.session.execute("layer.new", &json!({"top": true, "name": "Top"})).unwrap()["id"].as_u64().unwrap());
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    // A row left highlighted outside the sublayer before the panel shows again.
    app.session.active_mut().unwrap().layer_rows = vec![c, b];
    let duplicate = menu_command(&app, "layer.duplicate").params;
    menu_run(&mut app, "layer.duplicate", duplicate).unwrap();
    let count = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.node(id).unwrap().children().unwrap().len();
    assert_eq!(count(&app, layer), 2, "rectangle c, outside the sublayer, is not copied");
    assert_eq!(count(&app, g), 2, "rectangle b is copied");
    assert!(!enabled(&app, "layer.hideOthers"), "Hide Others is off in isolation mode");
    assert!(app.session.active().unwrap().doc.node(top).unwrap().visible, "the layer outside the isolated art still shows");
    options(&mut app).layers_only = true;
    app.session.execute("object.isolate", &json!({"id": g.0})).unwrap();
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    app.session.active_mut().unwrap().layer_rows = vec![];
    assert!(!enabled(&app, "layer.locate"), "the sublayer holding the group is not listed");
    let _ = menu_run(&mut app, "layer.locate", json!({"layersOnly": true})); // refused: the row is not listed
    assert!(rows_of(&app).is_empty(), "the sublayer holding the group is not highlighted");
}

/// OK in Delete Selection's question deletes nothing when isolation mode started while it asked and a row it asked
/// about is outside the isolated art, and rows given outside it, as rows dropped on the trash are, are not deleted
/// either.
#[test]
fn isolation_keeps_a_confirmed_delete_to_the_rows_listed() {
    let (mut app, [_, sub, g, _, a, c]) = nested();
    let uid = app.session.active().unwrap().uid;
    app.session.active_mut().unwrap().layer_rows = vec![sub];
    assert!(app.run("ui.layerDelete", json!({})).unwrap().is_null(), "it asks first");
    app.session.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let ok = json!({"ids": [sub.0], "doc": uid, "confirmed": true});
    let err = app.run("ui.layerDelete", ok.clone()).unwrap_err();
    assert!(err.contains("the object is outside the isolated group"), "{err}");
    assert!(app.session.active().unwrap().doc.node(sub).is_some(), "nothing was deleted");
    let err = app.run("ui.layerDelete", json!({"ids": [c.0]})).unwrap_err();
    assert!(err.contains("the object is outside the isolated group"), "{err}");
    assert!(app.session.active().unwrap().doc.node(c).is_some(), "the rectangle outside the group stays");
    // OK with no rows given deletes the highlighted rows the panel lists, inside the sublayer.
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    app.session.active_mut().unwrap().layer_rows = vec![a, c];
    app.run("ui.layerDelete", json!({"doc": uid, "confirmed": true})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.node(a).is_none() && doc.node(c).is_some(), "only the row inside the sublayer is deleted");
    app.session.execute("object.exitIsolation", &json!({"all": true})).unwrap();
    app.run("ui.layerDelete", ok).unwrap();
    assert!(app.session.active().unwrap().doc.node(sub).is_none());
}

#[test]
fn delete_selection_approval_does_not_remove_restored_layer_art() {
    let (app, [_, sub, _, _, art, _]) = nested();
    let mut app = crate::testing::windowed(app);
    let file = std::env::temp_dir().join(format!("vectorcraft-layer-delete-revert-{}.vectorcraft", std::process::id()));
    app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|err| err.to_string())));
    app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|err| err.to_string())));
    app.run("file.saveAs", json!({"path": file.to_string_lossy()})).unwrap();
    let st = app.session.active().unwrap();
    let (uid, reverts) = (st.uid, st.reverts);

    app.run("layer.setProps", json!({"id": sub.0, "name": "Edited sublayer"})).unwrap();
    assert!(app.run("ui.layerDelete", json!({"ids": [sub.0]})).unwrap().is_null());
    app.run("file.revert", json!({"confirmed": true})).unwrap();
    let _ = std::fs::remove_file(file);
    let st = app.session.active().unwrap();
    assert_eq!((st.uid, st.reverts), (uid, reverts + 1));
    assert_eq!(st.doc.node(sub).unwrap().name.as_deref(), Some("Sub"));
    assert!(st.doc.node(art).is_some());
    let undo = undo_len(&app);

    let confirm = craft::ui::dialog::dialog_id(crate::dialogs::confirm::KIND);
    crate::tests::picks::answer_dialog(&mut app, confirm, &[], true);
    let st = app.session.active().unwrap();
    assert!(st.doc.node(sub).is_some() && st.doc.node(art).is_some(), "old approval cannot delete restored artwork");
    assert_eq!(undo_len(&app), undo);
    assert_eq!(app.ui.status, "The document was reverted. Reopen this dialog to continue.");

    assert!(app.run("ui.layerDelete", json!({"ids": [sub.0]})).unwrap().is_null());
    crate::tests::picks::answer_dialog(&mut app, confirm, &[], true);
    let st = app.session.active().unwrap();
    assert!(st.doc.node(sub).is_none() && st.doc.node(art).is_none(), "a fresh confirmation still deletes the requested layer");
}

/// A document put in isolation mode opens its isolated row at once, and showing the documents again opens nothing.
#[test]
fn isolation_opens_the_isolated_row_of_each_document() {
    let (mut app, [_, _, g, ..]) = nested();
    let ctx = crate::testing::ui_context();
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    pass(&mut app, &ctx);
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    app.session.active_mut().unwrap().layers_open.set(g, false);
    app.session.execute("object.isolate", &json!({"id": g.0})).unwrap();
    assert!(is_open(&app, g), "the isolated group's row opens with the isolation");
    app.session.active_mut().unwrap().layers_open.set(g, false);
    app.session.execute("document.activate", &json!({"index": 1})).unwrap();
    pass(&mut app, &ctx);
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    pass(&mut app, &ctx);
    assert!(!is_open(&app, g), "a row closed in isolation mode stays closed");
}

/// In isolation mode an Alt-drag of the isolated row onto itself has no place to drop and copies nothing, and a guide
/// dragged since before isolation mode began goes to no layer outside the isolated art.
#[test]
fn isolation_keeps_drops_on_the_isolated_row_inside_it() {
    let (mut app, [layer, sub, g, ..]) = nested();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 50, "layer": layer.0})).unwrap();
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    let before = node_count(&app);
    let onto_itself = mv(&[sub], sub, LayerPosition::Into, true);
    assert!(!accepts(&app, &onto_itself));
    apply(&mut app, drop_event(onto_itself));
    assert_eq!(node_count(&app), before, "nothing was copied");
    app.session.execute("object.exitIsolation", &json!({"all": true})).unwrap();
    let guide_onto_group = LayerMove::new(vec![RowId::Guide(0)], row(g), LayerPosition::Into, false);
    assert!(accepts(&app, &guide_onto_group), "the guide would go to the sublayer holding it");
    app.session.execute("object.isolate", &json!({"id": g.0})).unwrap();
    assert!(!accepts(&app, &guide_onto_group), "not onto the isolated group's row, outside which its sublayer is");
    apply(&mut app, drop_event(guide_onto_group));
    assert_eq!(app.session.active().unwrap().doc.ruler_guides()[0].layer, Some(layer), "the guide stays on Layer 1");
}

/// In isolation mode Alt-clicking or Ctrl/Cmd+Alt-clicking an eye, or Alt-clicking a lock, changes no layer outside the
/// isolated art.
#[test]
fn isolation_keeps_alt_clicks_on_eyes_and_locks_inside_it() {
    let (mut app, [_, sub, ..]) = nested();
    let top = NodeId(app.session.execute("layer.new", &json!({"top": true, "name": "Top"})).unwrap()["id"].as_u64().unwrap());
    app.session.execute("object.isolate", &json!({"id": sub.0})).unwrap();
    let alt_cmd = Mods { cmd: true, alt: true, ..Mods::NONE };
    for (column, mods) in [(LayerColumn::Visibility, Mods::ALT), (LayerColumn::Visibility, alt_cmd), (LayerColumn::Lock, Mods::ALT)] {
        apply(&mut app, LayersEvent::cell_click(key(sub), column, mods));
    }
    let t = app.session.active().unwrap().doc.node(top).unwrap().clone();
    assert!(t.visible && !t.locked && !matches!(t.kind, NodeKind::Layer { preview: false, .. }), "the layer outside the isolated art is unchanged");
}

/// With Show Layers Only on, Locate Object becomes Locate Layer in the panel menu and a row's menu, and the footer's
/// Locate highlights the sublayer holding the selected art, opening the layer around it in the next pass. Off, it
/// highlights the art's own row.
#[test]
fn with_show_layers_only_locate_object_becomes_locate_layer() {
    let (mut app, [layer, sub, _, b, _, _]) = nested();
    let ctx = crate::testing::ui_context();
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    assert!(locate_titled(&app, "Locate Object"));
    footer_locate(&mut app);
    assert_eq!(rows_of(&app), vec![b], "off: the art's own row");

    options(&mut app).layers_only = true;
    assert!(locate_titled(&app, "Locate Layer"));
    app.session.active_mut().unwrap().layers_open.set(layer, false);
    pass(&mut app, &ctx);
    footer_locate(&mut app);
    assert_eq!(rows_of(&app), vec![sub], "on: the sublayer holding the art");
    pass(&mut app, &ctx);
    assert!(is_open(&app, layer), "the layer around the sublayer opens");

    // The row menu's Locate entry is the same command with the same parameters.
    app.session.active_mut().unwrap().layer_rows = vec![];
    menu_run(&mut app, "layer.locate", json!({"layersOnly": true})).unwrap();
    assert_eq!(rows_of(&app), vec![sub]);
}

/// Locate Layer or Locate Object run again on the row it already highlighted reveals that row again, once per Locate.
#[test]
fn a_repeated_locate_reveals_the_row_it_already_highlighted() {
    let (mut app, [layer, sub, g, b, _, _]) = nested();
    let ctx = crate::testing::ui_context();
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();

    options(&mut app).layers_only = true;
    app.session.execute("layer.locate", &json!({"layersOnly": true})).unwrap();
    assert_eq!(reveal_of(&app), Some(LayersReveal::Row(sub)));
    pass(&mut app, &ctx);
    assert_eq!(rows_of(&app), vec![sub]);
    app.session.active_mut().unwrap().layers_open.set(layer, false);
    app.session.execute("layer.locate", &json!({"layersOnly": true})).unwrap();
    assert_eq!(reveal_of(&app), Some(LayersReveal::Row(sub)), "the highlighted row is asked for again");
    pass(&mut app, &ctx);
    assert!(is_open(&app, layer), "the layer around the sublayer opens again");
    app.session.active_mut().unwrap().layers_open.set(layer, false);
    pass(&mut app, &ctx);
    assert!(!is_open(&app, layer), "a pass without a Locate opens nothing");

    options(&mut app).layers_only = false;
    app.session.execute("layer.locate", &json!({})).unwrap();
    pass(&mut app, &ctx);
    assert_eq!(rows_of(&app), vec![b]);
    for id in [sub, g] {
        app.session.active_mut().unwrap().layers_open.set(id, false);
    }
    app.session.execute("layer.locate", &json!({})).unwrap();
    assert_eq!(reveal_of(&app), Some(LayersReveal::Row(b)));
    pass(&mut app, &ctx);
    assert!(is_open(&app, layer) && is_open(&app, sub) && is_open(&app, g), "the layers and group around the object open again");
    assert_eq!(rows_of(&app), vec![b]);
}

/// A layer of `n` groups of one path each, with only the layer open, as a document opens → (app, layer, the paths in
/// stacking order, so the first is in the bottom row). The path in group `i` is named `Path {i}`.
fn groups_of_one(n: usize) -> (VectorcraftApp, NodeId, Vec<NodeId>) {
    use vectorcraft_doc::Appearance;
    use vectorcraft_geom::{Rect, shapes};
    let mut d = Document::new(1000.0, 1000.0);
    let mut groups = vec![];
    let mut paths = vec![];
    for i in 0..n {
        let r = Rect::new(0.0, i as f64, 1.0, i as f64 + 1.0);
        let mut path = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default());
        path.name = Some(format!("Path {i}"));
        paths.push(path.id);
        groups.push(Arc::new(Node::group(d.alloc_id(), vec![Arc::new(path)])));
    }
    *Arc::make_mut(&mut d.layers[0]).children_mut().unwrap() = groups;
    let layer = d.layers[0].id;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.add_document(d, None);
    (app, layer, paths)
}

/// Without Locate Selected Objects (the default), an object selected outside the panel leaves the open rows as they
/// were; the request is taken and ignored. (The scroll offset is the framework panel's.)
#[test]
fn without_the_option_a_selection_leaves_the_rows_and_the_scroll_as_they_were() {
    let (mut app, layer, paths) = groups_of_one(500);
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    app.session.execute("select.set", &json!({"ids": [paths[0].0]})).unwrap();
    assert_eq!(reveal_of(&app), Some(LayersReveal::Selected(paths[0])));
    for _ in 0..2 {
        pass(&mut app, &ctx);
    }
    assert_eq!(reveal_of(&app), None, "the pass took the request");
    let group = parent(&app, paths[0]).unwrap();
    assert!(is_open(&app, layer) && !is_open(&app, group));
}

/// With Locate Selected Objects on, an object selected outside the panel is located as Locate Object locates it: the
/// layer and the group around it open. The highlighted row stays as it was.
#[test]
fn with_the_option_a_selected_object_is_opened_and_scrolled_into_view() {
    let (mut app, layer, paths) = groups_of_one(500);
    options(&mut app).locate_selected = true;
    let ctx = crate::testing::ui_context();
    let top = parent(&app, paths[499]).unwrap();
    app.session.execute("layer.highlight", &json!({"ids": [top.0]})).unwrap();
    pass(&mut app, &ctx);
    expand(&mut app, &json!({"ids": [layer.0], "open": false})).unwrap();
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 1, "only the layer's row");
    app.session.execute("select.set", &json!({"ids": [paths[0].0]})).unwrap();
    assert_eq!(reveal_of(&app), Some(LayersReveal::Selected(paths[0])));
    for _ in 0..4 {
        pass(&mut app, &ctx);
    }
    let group = parent(&app, paths[0]).unwrap();
    assert!(is_open(&app, layer) && is_open(&app, group), "the layer and the group open");
    assert_eq!(app.session.active().unwrap().highlighted_rows(), [top], "the highlighted row stays");
}

/// With Locate Selected Objects on, the panel's own selecting locates nothing; an object then selected outside the
/// panel is located, and the highlighted rows stay.
#[test]
fn with_the_option_the_panels_own_clicks_locate_nothing() {
    let (mut app, [layer, sub, g, b, _, c]) = nested();
    options(&mut app).locate_selected = true;
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 3, "Layer 1, Sub (closed) and c");
    select_rows(&mut app, &[layer, sub, c]);
    assert_eq!(rows_of(&app), [layer, sub, c]);
    // Shift-clicks on c's selection square, then Sub's, as the panel's pass runs them: Sub's and c's art is selected.
    unrevealed(&mut app, |app| app.run("layer.selectAll", json!({"id": c.0, "add": true})).unwrap());
    unrevealed(&mut app, |app| app.run("layer.selectAll", json!({"id": sub.0, "add": true})).unwrap());
    let st = app.session.active().unwrap();
    assert!(st.selection.objects.len() >= 3 && st.selection.objects.contains(&c), "c and Sub's art are selected");
    assert_eq!(reveal_of(&app), None, "a selection made in the panel asks for no reveal");
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 3, "no row opens");
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 6, "b's sublayer and group open");
    assert_eq!(rows_of(&app), [layer, sub, c]);
}

/// With Locate Selected Objects on, a selection the panel menu changes is not located: Delete Selection deletes the
/// highlighted object, and the object left selected stays in its closed sublayer and group.
#[test]
fn with_the_option_the_panel_menu_locates_nothing() {
    let (mut app, [_, sub, g, b, _, c]) = nested();
    options(&mut app).locate_selected = true;
    app.session.execute("select.set", &json!({"ids": [b.0, c.0]})).unwrap();
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    app.session.execute("layer.highlight", &json!({"ids": [c.0]})).unwrap();
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 3, "Layer 1, Sub (closed) and c");
    menu_run(&mut app, "ui.layerDelete", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, [b], "c is deleted and b stays selected");
    assert_eq!(reveal_of(&app), None);
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 2, "Sub stays closed");
}

/// With Locate Selected Objects on, a saved selection recalled from the panel menu is not located.
#[test]
fn with_the_option_a_selection_recalled_from_the_panel_menu_is_not_located() {
    let (mut app, [_, sub, g, b, _, c]) = nested();
    options(&mut app).locate_selected = true;
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    app.run("select.save", json!({"name": "Kept"})).unwrap();
    app.run("select.set", json!({"ids": [c.0]})).unwrap();
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 3, "Layer 1, Sub (closed) and c");
    menu_run(&mut app, "ui.layers.recall", json!({"name": "Kept"})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, [b], "the saved selection is recalled");
    assert_eq!(reveal_of(&app), None, "recalling clears the request");
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 3, "Sub stays closed");
}

/// With Locate Selected Objects on, the object selected in a document that becomes active is not located; an object
/// selected after that is.
#[test]
fn with_the_option_a_document_becoming_active_locates_nothing() {
    let (mut app, [_, sub, g, b, a, _]) = nested();
    options(&mut app).locate_selected = true;
    app.session.execute("layer.highlight", &json!({"ids": []})).unwrap();
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    pass(&mut app, &ctx);
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(reveal_of(&app), None, "activating asks for no reveal");
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 3, "b's sublayer and group stay closed");
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(reveal_of(&app), Some(LayersReveal::Selected(a)));
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 5, "a's sublayer opens");
}

/// With Locate Selected Objects on, the copies an Alt-drag selects are located once the drag ends, not while it lasts.
#[test]
fn with_the_option_a_selection_made_during_a_drag_is_located_when_it_ends() {
    let (mut app, [_, sub, g, b, ..]) = nested();
    options(&mut app).locate_selected = true;
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    pass(&mut app, &ctx);
    app.session.begin_interaction("Move").unwrap();
    for i in 1..=3 {
        app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, i * 10, 0], "copy": true})).unwrap();
        assert_eq!(reveal_of(&app), None, "a preview asks for nothing");
        pass(&mut app, &ctx);
        assert_eq!(listed_ids(&app).len(), 3, "nothing opens while the drag lasts");
    }
    app.session.commit_interaction().unwrap();
    let st = app.session.active().unwrap();
    let copy = st.selection.objects.last().copied().unwrap();
    assert!(copy != b && st.doc.parent_of(copy) == Some(g), "the copy is selected, in b's group");
    assert_eq!(reveal_of(&app), Some(LayersReveal::Selected(copy)));
    pass(&mut app, &ctx);
    assert_eq!(listed_ids(&app).len(), 7, "Sub and the group open on the copy's row");
}

/// With Locate Selected Objects on, art selected outside the panel in another layer is located as the art, not as its
/// active layer. (The scroll offset is the framework panel's.)
#[test]
fn with_the_option_the_located_object_is_revealed_instead_of_its_active_layer() {
    let (mut app, layer, paths) = groups_of_one(500);
    options(&mut app).locate_selected = true;
    app.session.execute("layer.new", &json!({})).unwrap();
    assert_ne!(app.session.active().unwrap().current_layer(), Some(layer));
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    app.session.execute("select.set", &json!({"ids": [paths[0].0]})).unwrap();
    assert_eq!(app.session.active().unwrap().current_layer(), Some(layer), "the path's layer is the active one");
    assert_eq!(reveal_of(&app), Some(LayersReveal::Selected(paths[0])), "the art, not the layer");
    for _ in 0..4 {
        pass(&mut app, &ctx);
    }
    assert!(is_open(&app, parent(&app, paths[0]).unwrap()), "the group opens");
}

/// With Locate Selected Objects and Show Layers Only on, art selected outside the panel has no row, so the request
/// is for the layer that the selection makes the active one.
#[test]
fn with_the_option_and_show_layers_only_the_active_layer_is_revealed() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    options(&mut app).locate_selected = true;
    options(&mut app).layers_only = true;
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let rect = |app: &mut VectorcraftApp| run(app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap();
    let bottom = rect(&mut app);
    for _ in 0..59 {
        run(&mut app, "layer.new", json!({}));
        rect(&mut app);
    }
    run(&mut app, "select.none", json!({}));
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    let bottom_layer = app.session.active().unwrap().doc.layer_of(NodeId(bottom)).unwrap();
    run(&mut app, "select.set", json!({"ids": [bottom]}));
    // The session asks for the object (writer note 3); the panel, which lists no row for it, reveals its layer.
    assert_eq!(reveal_of(&app), Some(LayersReveal::Selected(NodeId(bottom))));
    assert_eq!(reveal_for(&mut app), Some(bottom_layer), "the panel reveals the active layer");
    assert_eq!(reveal_of(&app), None, "the panel took the request");
}

/// With Locate Selected Objects on, art selected outside the panel that Show Layers Only or the filter leaves without a
/// row opens nothing, so its group stays closed once the panel lists every row again. Art the filter keeps has its
/// group opened.
#[test]
fn with_the_option_art_the_panel_does_not_list_opens_nothing() {
    let (mut app, _, paths) = groups_of_one(500);
    options(&mut app).locate_selected = true;
    let ctx = crate::testing::ui_context();
    let open = |app: &VectorcraftApp, i: usize| is_open(app, parent(app, paths[i]).unwrap());
    let locate = |app: &mut VectorcraftApp, i: usize| {
        app.session.execute("select.set", &json!({"ids": [paths[i].0]})).unwrap();
        for _ in 0..4 {
            pass(app, &ctx);
        }
    };
    pass(&mut app, &ctx);
    options(&mut app).layers_only = true;
    locate(&mut app, 0);
    options(&mut app).layers_only = false;
    pass(&mut app, &ctx);
    assert!(!open(&app, 0), "Show Layers Only: the group stays closed");
    // The panel's filter, of the one type it offers (Objects).
    app.ui.layers_ui.filter = LayersUi::default().filter;
    app.ui.layers_ui.filter.objects = vec![FilterObject::Text];
    locate(&mut app, 1);
    assert!(!open(&app, 1), "a filter that leaves the path out: the group stays closed");
    app.ui.layers_ui.filter.objects = vec![FilterObject::Paths];
    locate(&mut app, 2);
    assert!(open(&app, 2), "a filter that keeps the path: the group opens");
}

/// Selecting art on the canvas makes its layer the active one and asks to reveal it, but not while a row is
/// highlighted in the panel.
#[test]
fn a_canvas_selection_reveals_its_active_layer() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let mut rects = vec![run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap()];
    for _ in 0..59 {
        run(&mut app, "layer.new", json!({}));
        rects.push(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap());
    }
    run(&mut app, "select.none", json!({}));
    let ctx = crate::testing::ui_context();
    pass(&mut app, &ctx);
    let doc = app.session.active().unwrap().doc.clone();
    let bottom_layer = doc.layer_of(NodeId(rects[0])).unwrap();
    let top_layer = doc.layer_of(NodeId(rects[59])).unwrap();
    run(&mut app, "select.set", json!({"ids": [rects[0]]}));
    assert_eq!(app.session.active().unwrap().current_layer(), Some(bottom_layer));
    assert_eq!(reveal_for(&mut app), Some(bottom_layer), "no row is highlighted: the active layer");
    // A row highlighted in the panel keeps the view when the active layer changes.
    run(&mut app, "layer.highlight", json!({"ids": [top_layer.0], "mode": "set"}));
    pass(&mut app, &ctx);
    // The active layer changes, as production's test set it, with the request a change of it makes.
    let st = app.session.active_mut().unwrap();
    st.active_layer = Some(bottom_layer);
    st.layers_reveal = Some(LayersReveal::Current(bottom_layer));
    assert_eq!(reveal_for(&mut app), None, "a highlighted row keeps the view");
}
