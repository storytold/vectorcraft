use craft::ui::dialog::{DialogId, PromptChoice};
use craft::ui::menu::PanelMenu;
use egui::{Modifiers, vec2};
use vectorcraft_engine::Session;

use super::*;
use crate::dialogs::{symbol_delete, symbol_options};
use crate::tests::panel_harness::Panel;

fn symbols(app: &mut VectorcraftApp) -> Vec<String> {
    let list = app.run("symbol.list", json!({})).unwrap();
    list["symbols"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap_or_default().to_string()).collect()
}

/// Pixels of `img` that something paints opaquely, and those of them that are red.
fn painted(img: &vectorcraft_render::Rendered) -> (usize, usize) {
    let painted: Vec<&[u8; 4]> = img.pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 200).collect();
    let red = painted.iter().filter(|p| p[0] > 150 && p[1] < 80 && p[2] < 80).count();
    (painted.len(), red)
}

/// A thumbnail is at most 1,024 pixels on a side, whatever the size of the chip it draws.
#[test]
fn thumbnails_are_bounded_in_size() {
    assert_eq!(thumb_px(40.0, 2.0), 80);
    assert_eq!(thumb_px(1.0, 1.0), 8);
    assert_eq!(thumb_px(100_000.0, 2.0), 1024);
    assert_eq!(thumb_px(f32::NAN, 1.0), 8);
}

/// Art drawn into a box fills the box, its bounds stretched to the box's proportions, as the
/// art's map scales it; art with no area draws nothing.
#[test]
fn mapped_art_fills_its_box_in_the_box_s_proportions() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("symbol.new", &json!({"name": "Wide"})).unwrap();
    let doc = &*s.doc().unwrap().doc;
    let art = vectorcraft_effects::graphic_bounds(doc, "Wide").expect("the art's bounds");
    let mut r = vectorcraft_render::Renderer::new();
    // A square box draws the 2:1 art stretched to fill it, as its map scales it.
    let img = render_art(&mut r, doc, "Wide", art, (40, 40)).expect("the art");
    let (painted, red) = painted(&img);
    assert!(red * 10 >= 40 * 40 * 9 && red == painted, "{painted} painted, {red} red");
    assert!(render_art(&mut r, doc, "Wide", vectorcraft_geom::Rect::new(0.0, 0.0, 0.0, 10.0), (40, 40)).is_none());
}

/// A symbol that nests another (a library's included) shows the nested art in its
/// thumbnail: the preview keeps the document's symbols and patterns, not its art.
#[test]
fn thumbnails_show_the_symbols_a_symbol_nests() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    // The instance "Dot" leaves behind becomes all of "Badge".
    s.execute("symbol.new", &json!({"name": "Dot"})).unwrap();
    s.execute("symbol.new", &json!({"name": "Badge"})).unwrap();
    // Other art of the document stays out of the preview.
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    let lib = vectorcraft_engine::cmd::symbollib::SymbolLibrary::from_document(&s.doc().unwrap().doc, &["Badge".into()], "Kit".into()).unwrap();
    let mut r = vectorcraft_render::Renderer::new();
    for doc in [&*s.doc().unwrap().doc, &lib.doc] {
        let img = render_thumb(&mut r, doc, "Badge", 32).expect("a thumbnail");
        let (painted, red) = painted(&img);
        assert!(painted > 0 && red * 10 >= painted * 9, "the nested Dot paints it: {painted} painted, {red} red");
    }
}

/// `draw` in the panel harness over `app`, with the app's dialogs on the fixture's window, as the
/// app's own window hosts them.
fn harness(app: VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Panel {
    let panel = Panel::new(app, draw);
    let ctx = panel.ctx().clone();
    panel.app().ui_ctx = Some(ctx);
    panel
}

/// The Symbols panel in the panel harness.
fn panel(app: VectorcraftApp) -> Panel {
    harness(app, show)
}

/// Art carried off the canvas above the Symbols panel: a row for each named object (a layer too), whose drag
/// carries that object (`PanelDrag::Art`) as a move carried off the canvas does; production's tests set that
/// payload too, as no panel row carries art.
fn art_and_symbols(app: &mut VectorcraftApp, ui: &mut Ui) {
    use craft::ui::contract::{ListDrag, ShowCollection as _};
    if let Some(st) = app.session.active() {
        let (document, reverts) = (st.uid, st.reverts);
        let mut named = Vec::new();
        st.doc.walk(|n| {
            if let Some(name) = &n.name {
                named.push((name.clone(), n.id));
            }
        });
        let keys: Vec<usize> = (0..named.len()).collect();
        let rows = craft::behavior::keyed(&keys, |key: &usize| *key);
        let ids: Vec<NodeId> = named.iter().map(|(_, id)| *id).collect();
        ui.allocate_ui(vec2(ui.available_width(), 200.0), |ui| {
            let _ = craft::ui::list::List::<usize>::new("Art off the canvas")
                .drag_payload(move |drag: &ListDrag<usize>| ids.get(drag.grabbed).map(|id| PanelDrag::Art { document, reverts, ids: vec![*id] }))
                .show_items(ui, &rows, &mut |key, _| craft::ui::list::ListRow::new(named[*key].0.clone()));
        });
    }
    show(app, ui);
}

/// The symbol instances' controls of the Control bar.
fn symbol_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    control_bar(app, ui);
}

/// The whole Control bar.
fn whole_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    crate::chrome::control_bar(app, ui);
}

/// The Symbols panel, with its menu's rows and states read and drawn every frame, as the dock
/// reads them.
fn panel_and_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    show(app, ui);
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    PanelMenu::new(menu_rows(app), Vec::new()).show(ui);
}

/// The Symbols panel, where a second document with a Leaf of its own becomes active while a symbol
/// of the first document (the one with two symbols) is dragged.
fn second_document_mid_drag(app: &mut VectorcraftApp, ui: &mut Ui) {
    let first = app.session.active().is_some_and(|st| st.doc.symbols.len() == 2);
    if first && craft::ui::runtime::drag::active::<ListDrag<SymbolKey>>(ui.ctx()).is_some() {
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
        app.run("symbol.new", json!({"name": "Leaf"})).unwrap();
    }
    show(app, ui);
}

/// The context the app's dialogs open on.
fn window(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the app has a window")
}

use crate::testing::windowed;

/// The dialog on top of the app's window: its id, its key and its fields, as automation reads them.
fn top(app: &VectorcraftApp) -> Option<(DialogId, String, Value)> {
    let open = app.dialogs()?.with(|host| host.open_dialogs());
    let dialog = open.last()?;
    Some((dialog.id, dialog.key.clone().unwrap_or_default(), dialog.fields.clone().unwrap_or_default()))
}

fn dialog_kind(app: &VectorcraftApp) -> Option<String> {
    top(app).map(|(_, key, _)| key)
}

/// Field `name` of the top dialog.
fn field(app: &VectorcraftApp, name: &str) -> Value {
    top(app).map_or(Value::Null, |(_, _, fields)| fields[name].clone())
}

/// Sets field `name` of the top dialog, as automation does.
fn set_field(app: &VectorcraftApp, name: &str, value: Value) {
    let (ctx, (id, ..)) = (window(app), top(app).expect("an open dialog"));
    let host = app.dialogs().expect("a window");
    host.with(|h| h.set_json_field(&ctx, id, name, value)).unwrap_or_else(|e| panic!("{name}: {e}"));
}

/// OK on the top dialog, as automation answers it. Symbol Options holds its OK: the frame's pump
/// runs it, and the dialog closes with the answer its OK accepted, as its next pass closes it; an
/// OK the app refuses keeps the dialog open.
fn ok(app: &mut VectorcraftApp) {
    let (ctx, (id, ..)) = (window(app), top(app).expect("an open dialog"));
    let host = app.dialogs().expect("a window");
    for _ in 0..2 {
        host.with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
    }
}

/// Cancel on the top dialog, as Esc gives it, then the frame's pump.
fn cancel(app: &mut VectorcraftApp) {
    let (ctx, (id, ..)) = (window(app), top(app).expect("an open dialog"));
    app.dialogs().expect("a window").with(|h| h.cancel_by_id(&ctx, id));
    crate::dialogs::host::pump(app, &ctx);
}

/// Answers the open delete question with `choice`, as its button does (`Proceed`: Yes or Expand
/// Instances; `Alternate`: Delete Instances; `Cancel`: No or Cancel), then the frame's pump runs
/// the answer.
// PLAN: `dialogs::host::pump` (hub-w2, host.rs:130) must call `symbol_delete::pump` (ap.AS16); its list leaves it out.
fn choose(app: &mut VectorcraftApp, choice: PromptChoice) {
    set_field(app, "choice", json!(choice));
    let (ctx, (id, ..)) = (window(app), top(app).expect("a delete question"));
    assert!(app.dialogs().expect("a window").with(|h| h.confirm_by_id(&ctx, id)), "the question takes {choice:?}");
    crate::dialogs::host::pump(app, &ctx);
}

/// Which delete question is open: `Some(true)` for the in-use one, `Some(false)` for "Delete the
/// selected symbol?" (or symbols), `None` for neither. Only the in-use question takes the Alternate
/// answer (Delete Instances).
fn delete_question(app: &VectorcraftApp) -> Option<bool> {
    let (id, _, _) = top(app).filter(|(_, key, _)| key == symbol_delete::KIND)?;
    let ctx = window(app);
    app.dialogs()?.with(|h| {
        let in_use = h.set_json_field(&ctx, id, "choice", json!(PromptChoice::Alternate)).is_ok();
        // The probe leaves the question's default answer as it was.
        h.set_json_field(&ctx, id, "choice", json!(PromptChoice::Proceed)).expect("every delete question answers Proceed");
        Some(in_use)
    })
}

/// The panel menu's row of command `id` (`params` tells apart the rows of one command) and whether
/// its state lets it fire.
fn menu_row(app: &VectorcraftApp, id: &str, params: Option<&Value>) -> (CommandRef, bool) {
    let row = menu_rows(app)
        .into_iter()
        .find_map(|row| match row {
            PanelRow::Command(command) if command.id == id && params.is_none_or(|p| command.params == *p) => Some(command),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a {id} row in the panel menu"));
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    let enabled = states.get(&row).is_some_and(CommandState::is_enabled);
    (row, enabled)
}

/// Fires the panel menu's row of `id` with `mods` held, as the fired-command loop runs a panel row
/// (`Fired::mods`): the panel's own rows through [`run`], catalog commands with the params
/// [`fired_params`] gives them.
fn fire_menu_row(app: &mut VectorcraftApp, id: &str, params: Option<&Value>, mods: Mods) {
    let (row, enabled) = menu_row(app, id, params);
    assert!(enabled, "{id} is enabled");
    let mut params = row.params.clone();
    if let Some(ran) = run(app, &row.id, &params) {
        ran.unwrap_or_else(|e| panic!("{id}: {e}"));
        return;
    }
    fired_params(&row.id, &mut params, mods);
    app.run(&row.id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// The views the panel menu's view rows show checked.
fn checked_view(app: &VectorcraftApp) -> Vec<&'static str> {
    let mut states = CommandStates::new();
    menu_states(app, &mut states);
    let checked = |view: &str| {
        let row = CommandRef::with_params("ui.panelView", json!({"panel": "symbols", "view": view}));
        states.get(&row) == Some(&CommandState::new().checked(true))
    };
    ["thumbnail", "smallList", "largeList"].into_iter().filter(|&view| checked(view)).collect()
}

/// Names object `id`, as its Layers row shows it.
fn name_object(app: &mut VectorcraftApp, id: NodeId, name: &str) {
    app.run("layer.setProps", json!({ "id": id.0, "name": name })).unwrap();
}

/// Whether the selection is one ordinary path: the art a Shift drop or click keeps.
fn art_still_selected(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    matches!(st.selection.objects.as_slice(), [id] if st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. })))
}

/// Width over height of symbol `name`'s art at its natural size (the art is kept in a square, its size beside it).
fn art_ratio(app: &VectorcraftApp, name: &str) -> f64 {
    let natural = app.session.active().unwrap().doc.symbol_natural_xf(name);
    symbol_art(app, name).geometric_bounds().map(|b| natural.transform_rect_bbox(b)).map_or(0.0, |b| b.width() / b.height())
}

/// A document with one selected 30 × 20 rectangle.
fn one_rect() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let rect = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({ "ids": [rect] })).unwrap();
    app
}

/// `one_rect` with the rectangle named "Art": a document without symbols.
fn art_only() -> VectorcraftApp {
    let mut app = one_rect();
    let rect = app.session.active().unwrap().selection.objects[0];
    name_object(&mut app, rect, "Art");
    app
}

/// `leaf_dot_and_rect` with the rectangle named "Art" and Leaf's instance named "Placed": Layers
/// rows that share no name with a symbol.
fn drop_doc() -> VectorcraftApp {
    let (mut app, rect) = leaf_dot_and_rect();
    let placed = instance_of(&app, "Leaf").expect("a Leaf instance");
    name_object(&mut app, placed, "Placed");
    name_object(&mut app, rect, "Art");
    app
}

/// Symbols A, B and C, of which only B has an instance (named "Placed").
fn abc_doc() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    for (i, name) in ["A", "B", "C"].into_iter().enumerate() {
        app.run("shape.rectangle", json!({"x": 10.0 + 40.0 * i as f64, "y": 10, "width": 30, "height": 20})).unwrap();
        let id = NodeId(app.run("symbol.new", json!({"name": name})).unwrap()["id"].as_u64().unwrap());
        // A and C lose their instances; B keeps its own.
        if name == "B" {
            name_object(&mut app, id, "Placed");
        } else {
            app.run("edit.clear", json!({"ids": [id.0]})).unwrap();
        }
    }
    app
}

/// `abc_doc` after Select All Unused: A and C are picked.
fn picked_doc() -> VectorcraftApp {
    let mut app = abc_doc();
    fire_menu_row(&mut app, ROW_SELECT_UNUSED, None, Mods::NONE);
    app
}

/// The views a table runs each row in: the grid and a list.
const VIEWS: [&str; 2] = ["thumbnail", "smallList"];

/// What a table row leaves, asserted on the panel with the undo steps before the row and the row's
/// name.
type Check = fn(&mut Panel, usize, &str);

/// The app a table row starts from.
type Setup = fn() -> VectorcraftApp;

/// Drops on the Symbols panel, in Thumbnail View and a list view, run the command each stands for:
/// art dragged from a Layers row onto the list makes a symbol of it (Shift at the release keeps the
/// art as ordinary art), and onto a symbol without Alt makes a new symbol too; a symbol dragged
/// from the panel onto New Symbol is duplicated, with art selected or none, and onto Delete Symbol
/// is deleted as a click deletes it: asked about, at once with Alt when unused, and with every
/// symbol Select All Unused picked when it is one of them. Each is one undo step.
#[test]
fn drop_to_command() {
    const EMPTY: &str = "No symbols in this document";
    let rows: [(&str, Setup, &str, &str, Modifiers, Check); 11] = [
        ("art on the list", art_only, "Art", EMPTY, Modifiers::NONE, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(symbols(&mut app), ["New Symbol"], "{what}");
            assert_eq!(art(&app, "New Symbol"), (1, 0), "{what}: the art became an instance");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        ("art on the list with Shift", art_only, "Art", EMPTY, Modifiers::SHIFT, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(symbols(&mut app), ["New Symbol"], "{what}");
            assert_eq!(art(&app, "New Symbol"), (0, 1), "{what}: the art stays ordinary art");
            assert!(art_still_selected(&app), "{what}: and stays selected");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        ("plain art on a symbol", drop_doc, "Art", "Leaf", Modifiers::NONE, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(symbols(&mut app), ["Leaf", "Dot", "New Symbol"], "{what}: a new symbol");
            assert_eq!(art(&app, "New Symbol"), (1, 0), "{what}: the art became its instance");
            assert!((art_ratio(&app, "Leaf") - 1.0).abs() < 1e-6, "{what}: Leaf keeps its square art");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        ("a symbol on New Symbol", drop_doc, "Leaf", "New Symbol", Modifiers::NONE, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(symbols(&mut app), ["Leaf", "Dot", "Leaf copy"], "{what}: a copy of the dragged symbol");
            assert_eq!(dialog_kind(&app), None, "{what}: no Symbol Options");
            assert_eq!(art(&app, "Leaf copy").0, 0, "{what}: no instance placed");
            assert_eq!(current(&app).as_deref(), Some("Dot"), "{what}: Dot stays current");
            assert!(art_still_selected(&app), "{what}: the selection is unchanged");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        (
            "a symbol on New Symbol with nothing selected",
            || {
                let mut app = leaf_and_dot();
                app.run("select.none", json!({})).unwrap();
                app
            },
            "Dot",
            "New Symbol",
            Modifiers::NONE,
            |p, undo, what| {
                let mut app = p.app();
                assert_eq!(symbols(&mut app), ["Leaf", "Dot", "Dot copy"], "{what}: the button takes the drop though clicks are off");
                assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
            },
        ),
        ("an unused symbol on Delete Symbol", drop_doc, "Dot", "Delete Symbol", Modifiers::NONE, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(delete_question(&app), Some(false), "{what}: asks first");
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{what}: nothing goes while it asks");
            choose(&mut app, PromptChoice::Proceed);
            assert_eq!(symbols(&mut app), ["Leaf"], "{what}: Yes deletes the dragged symbol");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        ("an unused symbol on Delete Symbol with Alt", drop_doc, "Dot", "Delete Symbol", Modifiers::ALT, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(dialog_kind(&app), None, "{what}: asks nothing");
            assert_eq!(symbols(&mut app), ["Leaf"], "{what}: deleted at once");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        ("a used symbol on Delete Symbol with Alt", drop_doc, "Leaf", "Delete Symbol", Modifiers::ALT, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(delete_question(&app), Some(true), "{what}: the in-use question, Alt or not");
            choose(&mut app, PromptChoice::Alternate);
            assert_eq!((symbols(&mut app), art(&app, "Leaf").0), (vec!["Dot".to_string()], 0), "{what}: Delete Instances");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        ("a picked symbol on Delete Symbol", picked_doc, "A", "Delete Symbol", Modifiers::NONE, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(delete_question(&app), Some(false), "{what}: one question");
            choose(&mut app, PromptChoice::Proceed);
            assert_eq!(symbols(&mut app), ["B"], "{what}: about every picked symbol");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        ("a picked symbol on Delete Symbol with Alt", picked_doc, "C", "Delete Symbol", Modifiers::ALT, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(dialog_kind(&app), None, "{what}: asks nothing");
            assert_eq!(symbols(&mut app), ["B"], "{what}: the picked symbols go");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
        ("a symbol the pick left out on Delete Symbol", picked_doc, "B", "Delete Symbol", Modifiers::ALT, |p, undo, what| {
            let mut app = p.app();
            assert_eq!(delete_question(&app), Some(true), "{what}: asked about on its own");
            choose(&mut app, PromptChoice::Proceed);
            assert_eq!((symbols(&mut app), art(&app, "B")), (vec!["A".to_string(), "C".to_string()], (0, 1)), "{what}: Expand Instances");
            assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
        }),
    ];
    for (what, setup, from, to, mods, check) in rows {
        for view in VIEWS {
            let mut app = setup();
            app.run("ui.panelView", json!({"panel": "symbols", "view": view})).unwrap();
            let undo = undo_len(&app);
            let mut p = harness(app, art_and_symbols);
            p.drag(from, to, mods);
            check(&mut p, undo, &format!("{what}, {view}"));
        }
    }
}

/// The panel's footer buttons, a click and a double click on a symbol, its item menu, the panel
/// menu's rows and the Control bar's symbol controls run their commands, with what the modifiers
/// held decide: Shift on New Symbol (the button or the menu row) and on Redefine Symbol keeps the
/// selected art as ordinary art, Alt on New Symbol skips Symbol Options; Symbol Options edits the
/// current symbol; Sort by Name sorts in one undo step and adds none when in order.
#[test]
fn footer_or_menu_row_with_modifiers_to_command() {
    let with_all = || {
        let mut app = leaf_and_dot();
        app.run("select.all", json!({})).unwrap();
        app
    };
    let rect = || leaf_dot_and_rect().0;
    let ab = || {
        let mut app = abc_doc();
        app.run("symbol.delete", json!({"name": "C"})).unwrap();
        app.run("symbol.setCurrent", json!({"name": "B"})).unwrap();
        app
    };
    let leaf_selected = || {
        let mut app = leaf_and_dot();
        let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
        app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
        app
    };
    type Show = fn(&mut VectorcraftApp, &mut Ui);
    type Act = fn(&mut Panel);
    let rows: [(&str, Setup, Show, Act, Check); 22] = [
        (
            "New Symbol with Shift",
            rect,
            show,
            |p| p.click_with("New Symbol", Modifiers::SHIFT),
            |p, undo, what| {
                let mut app = p.app();
                assert_eq!(dialog_kind(&app).as_deref(), Some(symbol_options::KIND), "{what}: asks for Symbol Options");
                assert_eq!(symbols(&mut app).len(), 2, "{what}: no symbol before OK");
                ok(&mut app);
                assert_eq!(art(&app, "New Symbol"), (0, 1), "{what}: the rectangle stays ordinary art");
                assert!(art_still_selected(&app), "{what}: and stays selected");
                assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
            },
        ),
        (
            "New Symbol with Shift and Alt",
            rect,
            show,
            |p| p.click_with("New Symbol", Modifiers::SHIFT | Modifiers::ALT),
            |p, undo, what| {
                let mut app = p.app();
                assert_eq!(dialog_kind(&app), None, "{what}: Alt skips Symbol Options");
                assert_eq!(symbols(&mut app), ["Leaf", "Dot", "New Symbol"], "{what}");
                assert_eq!(art(&app, "New Symbol"), (0, 1), "{what}: the rectangle stays ordinary art");
                assert!(art_still_selected(&app), "{what}: and stays selected");
                assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
            },
        ),
        (
            "New Symbol with Alt",
            rect,
            show,
            |p| p.click_with("New Symbol", Modifiers::ALT),
            |p, _, what| {
                let app = p.app();
                assert_eq!(dialog_kind(&app), None, "{what}: Alt skips Symbol Options");
                assert_eq!(art(&app, "New Symbol"), (1, 0), "{what}: the rectangle becomes an instance");
            },
        ),
        (
            "Symbol Options",
            with_all,
            show,
            |p| p.click("Symbol Options"),
            |p, undo, what| {
                let mut app = p.app();
                assert_eq!(dialog_kind(&app).as_deref(), Some(symbol_options::KIND), "{what}");
                assert_eq!(field(&app, "name"), json!("Dot"), "{what}: on the current symbol");
                cancel(&mut app);
                assert_eq!(undo_len(&app), undo, "{what}: opening it adds no undo step");
                assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{what}");
            },
        ),
        (
            "Symbol Options with Alt",
            || {
                let mut app = leaf_and_dot();
                app.run("select.all", json!({})).unwrap();
                app.run("symbol.setCurrent", json!({"name": "Leaf"})).unwrap();
                app
            },
            show,
            |p| p.click_with("Symbol Options", Modifiers::ALT),
            |p, _, what| {
                let app = p.app();
                assert_eq!(dialog_kind(&app).as_deref(), Some(symbol_options::KIND), "{what}");
                assert_eq!(field(&app, "name"), json!("Leaf"), "{what}: on the current symbol");
            },
        ),
        (
            "Symbol Options without symbols",
            art_only,
            show,
            |p| p.click("Symbol Options"),
            |p, _, what| {
                assert!(!p.enabled("Symbol Options"), "{what}: disabled without a current symbol");
                assert_eq!(dialog_kind(&p.app()), None, "{what}: no symbol, no dialog");
            },
        ),
        (
            "a click on a symbol",
            leaf_and_dot,
            show,
            |p| p.click("Leaf"),
            |p, _, what| {
                let app = p.app();
                assert_eq!((editing(&app), current(&app)), (None, Some("Leaf".into())), "{what}: makes it current");
            },
        ),
        (
            "a double click on a symbol",
            leaf_and_dot,
            show,
            |p| p.double_click("Leaf"),
            |p, _, what| {
                let app = p.app();
                assert_eq!(editing(&app).as_deref(), Some("Leaf"), "{what}: edits it");
                assert_eq!(art(&app, "Leaf").0, 1, "{what}: no instance is placed");
            },
        ),
        (
            "a double click on a symbol in Large List View",
            || {
                let mut app = leaf_and_dot();
                app.run("ui.panelView", json!({"panel": "symbols", "view": "largeList"})).unwrap();
                app
            },
            show,
            |p| p.double_click("Dot"),
            |p, _, what| {
                let app = p.app();
                assert_eq!(editing(&app).as_deref(), Some("Dot"), "{what}: edits it");
                assert_eq!(art(&app, "Dot").0, 0, "{what}: no instance is placed");
            },
        ),
        (
            "Redefine Symbol in the menu with Shift",
            rect,
            show,
            |p| fire_menu_row(&mut p.app(), "symbol.update", None, Mods::SHIFT),
            |p, undo, what| {
                let app = p.app();
                assert!((art_ratio(&app, "Dot") - 3.0).abs() < 1e-6, "{what}: Dot is the 60 × 20 rectangle");
                assert_eq!(art(&app, "Dot"), (0, 1), "{what}: the rectangle stays ordinary art");
                assert!(art_still_selected(&app), "{what}: and stays selected");
                assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
            },
        ),
        (
            "Redefine Symbol in the menu",
            rect,
            show,
            |p| fire_menu_row(&mut p.app(), "symbol.update", None, Mods::NONE),
            |p, _, what| {
                assert_eq!(art(&p.app(), "Dot"), (1, 0), "{what}: the rectangle becomes an instance");
            },
        ),
        (
            "New Symbol… in the menu with Shift",
            rect,
            show,
            |p| fire_menu_row(&mut p.app(), "ui.symbolOptions", Some(&json!({})), Mods::SHIFT),
            |p, undo, what| {
                let mut app = p.app();
                assert_eq!(dialog_kind(&app).as_deref(), Some(symbol_options::KIND), "{what}: asks for Symbol Options");
                ok(&mut app);
                assert_eq!(art(&app, "New Symbol"), (0, 1), "{what}: the rectangle stays ordinary art, though Shift was let go before OK");
                assert!(art_still_selected(&app), "{what}: and stays selected");
                assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
            },
        ),
        (
            "New Symbol… in the menu",
            rect,
            show,
            |p| fire_menu_row(&mut p.app(), "ui.symbolOptions", Some(&json!({})), Mods::NONE),
            |p, _, what| {
                let mut app = p.app();
                ok(&mut app);
                assert_eq!(art(&app, "New Symbol"), (1, 0), "{what}: the rectangle becomes an instance");
            },
        ),
        (
            "Sort by Name",
            || {
                let mut app = VectorcraftApp::new(Session::new(), Default::default());
                app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
                for (i, name) in ["bee", "Cat", "ant"].into_iter().enumerate() {
                    app.run("shape.rectangle", json!({"x": 10.0 + 40.0 * i as f64, "y": 10, "width": 30, "height": 20})).unwrap();
                    app.run("symbol.new", json!({"name": name})).unwrap();
                }
                app
            },
            show,
            |p| fire_menu_row(&mut p.app(), "symbol.sortByName", None, Mods::NONE),
            |p, undo, what| {
                let mut app = p.app();
                assert_eq!(symbols(&mut app), ["ant", "bee", "Cat"], "{what}: name order, ignoring case");
                assert_eq!(undo_len(&app), undo + 1, "{what}: one undo step");
                fire_menu_row(&mut app, "symbol.sortByName", None, Mods::NONE);
                assert_eq!(undo_len(&app), undo + 1, "{what}: already in order, no undo step");
            },
        ),
        (
            "Sort by Name with one symbol",
            || {
                let mut app = one_rect();
                app.run("symbol.new", json!({})).unwrap();
                app
            },
            show,
            |_| {},
            |p, _, what| assert!(!menu_row(&p.app(), "symbol.sortByName", None).1, "{what}: needs two symbols"),
        ),
        (
            "a view row",
            leaf_and_dot,
            show,
            |p| fire_menu_row(&mut p.app(), "ui.panelView", Some(&json!({"panel": "symbols", "view": "largeList"})), Mods::NONE),
            |p, _, what| {
                let app = p.app();
                assert_eq!(app.ui.symbols_view, View::LargeList, "{what}");
                assert_eq!(checked_view(&app), ["largeList"], "{what}: one view row checked");
            },
        ),
        (
            "the item menu's Duplicate Symbol",
            ab,
            show,
            |p| {
                p.secondary_click("A");
                p.click("Duplicate Symbol");
            },
            |p, _, what| {
                let mut app = p.app();
                assert_eq!(symbols(&mut app), ["A", "B", "A copy"], "{what}: duplicates the symbol under the pointer");
            },
        ),
        (
            "the item menu's Edit Symbol",
            ab,
            show,
            |p| {
                p.secondary_click("A");
                p.click("Edit Symbol");
            },
            |p, _, what| {
                let app = p.app();
                assert_eq!(editing(&app).as_deref(), Some("A"), "{what}: edits the symbol under the pointer, not the current B");
            },
        ),
        (
            "the item menu's Select All Instances",
            || {
                let mut app = abc_doc();
                for _ in 0..2 {
                    app.run("symbol.place", json!({"name": "A"})).unwrap();
                }
                app.run("select.none", json!({})).unwrap();
                app
            },
            show,
            |p| {
                p.secondary_click("A");
                p.click("Select All Instances");
            },
            |p, _, what| {
                let app = p.app();
                let st = app.session.active().unwrap();
                let picked: Vec<bool> = st
                    .selection
                    .objects
                    .iter()
                    .map(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::SymbolInstance { symbol, .. }) if symbol == "A"))
                    .collect();
                assert_eq!(picked, [true, true], "{what}: both instances of A");
            },
        ),
        (
            "the Control bar's Edit Symbol",
            leaf_selected,
            symbol_bar,
            |p| p.click("Edit Symbol"),
            |p, _, what| {
                let app = p.app();
                assert_eq!(editing(&app).as_deref(), Some("Leaf"), "{what}: the selected instance's symbol");
                assert!(instance_of(&app, "Leaf").is_some(), "{what}: the instance keeps its link");
            },
        ),
        (
            "the Control bar's Replace",
            || {
                let mut app = VectorcraftApp::new(Session::new(), Default::default());
                app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
                for (name, x) in [("Leaf", 10), ("Star", 150)] {
                    app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
                    app.run("symbol.new", json!({"name": name})).unwrap();
                }
                let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
                app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
                app
            },
            symbol_bar,
            |p| {
                p.click("Replace:");
                p.click("Star");
            },
            |p, _, what| {
                let app = p.app();
                let st = app.session.active().unwrap();
                let replaced = st.selection.objects.first().and_then(|id| st.doc.node(*id)).map(|n| &n.kind);
                assert!(matches!(replaced, Some(NodeKind::SymbolInstance { symbol, .. }) if symbol == "Star"), "{what}: the instance is replaced");
            },
        ),
        (
            "the Control bar with art selected",
            rect,
            symbol_bar,
            |_| {},
            |p, _, what| {
                assert!(!p.shows("Replace:"), "{what}: no Replace menu for art that is not an instance");
            },
        ),
    ];
    for (what, setup, draw, act, check) in rows {
        let app = setup();
        let undo = undo_len(&app);
        let mut p = harness(app, draw);
        act(&mut p);
        check(&mut p, undo, what);
    }
}

/// Symbol Options' fields reach `symbol.new` and `symbol.options`: a new symbol opens with a free
/// name, Movie Clip, Dynamic and Align to Pixel Grid off, and OK makes it in the art's place; the
/// edit flow renames, keeps Export Type and sets Symbol Type; Align to Pixel Grid and the 9-slice
/// guides go both ways, a guide kept exactly while its field is untouched; an empty or taken name
/// keeps the dialog open. The New Symbol button opens what F8's command opens, and with nothing
/// selected neither opens a dialog.
#[test]
fn dialog_fields_to_symbol_params() {
    type Fill = fn(&mut VectorcraftApp, &str);
    let rows: [(&str, Setup, Fill); 7] = [
        ("a new symbol's defaults", one_rect, |app, what| {
            app.run("ui.symbolOptions", json!({})).unwrap();
            assert_eq!(dialog_kind(app).as_deref(), Some(symbol_options::KIND), "{what}");
            assert!(symbols(app).is_empty(), "{what}: no symbol before OK");
            let fields = ["name", "exportType", "type", "alignToPixelGrid", "nineSlice"].map(|key| field(app, key));
            assert_eq!(fields, [json!("New Symbol"), json!("movieClip"), json!("dynamic"), json!(false), json!(false)], "{what}");
            ok(app);
            assert_eq!(dialog_kind(app), None, "{what}: OK closes it");
            let list = app.run("symbol.list", json!({})).unwrap();
            assert_eq!((&list["symbols"][0]["exportType"], &list["symbols"][0]["type"]), (&json!("movieClip"), &json!("dynamic")), "{what}");
            assert_eq!(art(app, "New Symbol"), (1, 0), "{what}: an instance takes the rectangle's place");
        }),
        (
            "the edit flow",
            || {
                let mut app = one_rect();
                app.run("symbol.new", json!({})).unwrap();
                app.run("shape.rectangle", json!({"x": 60, "y": 10, "width": 30, "height": 20})).unwrap();
                app
            },
            |app, what| {
                // The next one is offered a name no symbol has, and OK keeps the name typed and Export Type.
                app.run("ui.symbolOptions", json!({})).unwrap();
                assert_eq!(field(app, "name"), json!("New Symbol 2"), "{what}");
                set_field(app, "name", json!("Badge"));
                set_field(app, "exportType", json!("graphic"));
                ok(app);
                assert_eq!(symbols(app), ["New Symbol", "Badge"], "{what}");
                assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][1]["exportType"], "graphic", "{what}");
                // Symbol Options on a symbol renames it, keeps Export Type and sets Symbol Type.
                app.run("ui.symbolOptions", json!({"edit": true, "name": "Badge"})).unwrap();
                assert_eq!(field(app, "exportType"), json!("graphic"), "{what}");
                set_field(app, "name", json!("Mark"));
                set_field(app, "type", json!("static"));
                ok(app);
                assert_eq!(symbols(app), ["New Symbol", "Mark"], "{what}");
                let list = app.run("symbol.list", json!({})).unwrap();
                assert_eq!((&list["symbols"][1]["exportType"], &list["symbols"][1]["type"]), (&json!("graphic"), &json!("static")), "{what}");
                app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
                assert_eq!(field(app, "type"), json!("static"), "{what}: reopens as set");
                cancel(app);
            },
        ),
        (
            "Align to Pixel Grid",
            || {
                let mut app = VectorcraftApp::new(Session::new(), Default::default());
                app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
                app.run("shape.rectangle", json!({"x": 10.3, "y": 10.6, "width": 30, "height": 20})).unwrap();
                app
            },
            |app, what| {
                app.run("ui.symbolOptions", json!({})).unwrap();
                assert_eq!(field(app, "alignToPixelGrid"), json!(false), "{what}: off for a new symbol");
                set_field(app, "alignToPixelGrid", json!(true));
                ok(app);
                let name = symbols(app)[0].clone();
                assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][0]["alignToPixelGrid"], true, "{what}");
                app.run("ui.symbolOptions", json!({"edit": true, "name": name})).unwrap();
                assert_eq!(field(app, "alignToPixelGrid"), json!(true), "{what}: reopens with the symbol's setting");
                set_field(app, "alignToPixelGrid", json!(false));
                ok(app);
                assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][0]["alignToPixelGrid"], false, "{what}");
            },
        ),
        (
            "the 9-slice guides",
            || {
                let mut app = one_rect();
                app.run("symbol.new", json!({"name": "Mark"})).unwrap();
                app
            },
            |app, what| {
                // Enable Guides for 9-Slice Scaling stores the guides (percent in the dialog) and reopens as set.
                assert!(app.run("symbol.list", json!({})).unwrap()["symbols"][0]["nineSlice"].is_null());
                app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
                set_field(app, "nineSlice", json!(true));
                set_field(app, "sliceLeft", json!(20.0));
                ok(app);
                let nine = app.run("symbol.list", json!({})).unwrap()["symbols"][0]["nineSlice"].clone();
                assert_eq!(nine[0], json!(0.2), "{what}");
                app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
                assert_eq!((field(app, "nineSlice"), field(app, "sliceLeft").as_f64()), (json!(true), Some(20.0)), "{what}: reopens as set");
                cancel(app);
                // Guides outside the art, and ones that are not whole percents, open as they are and come
                // back unchanged from OK; a field changed to a value past 100 % puts that guide outside.
                let guides = [-0.123456789, 1.5, 0.013, -2.75];
                app.run("symbol.options", json!({"name": "Mark", "nineSliceGuides": guides})).unwrap();
                app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
                for (key, want) in [("sliceLeft", -12.3456789), ("sliceTop", 150.0), ("sliceRight", 1.3), ("sliceBottom", -275.0)] {
                    let shown = field(app, key).as_f64().unwrap_or(f64::NAN);
                    assert!((shown - want).abs() < 1e-9, "{what}: {key}: {shown}");
                }
                ok(app);
                assert_eq!(
                    app.run("symbol.list", json!({})).unwrap()["symbols"][0]["nineSlice"],
                    json!(guides),
                    "{what}: untouched guides stay exact"
                );
                app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
                set_field(app, "sliceRight", json!(-125.5));
                ok(app);
                let nine = app.run("symbol.list", json!({})).unwrap()["symbols"][0]["nineSlice"].clone();
                assert_eq!(nine, json!([-0.123456789, 1.5, -1.255, -2.75]), "{what}");
            },
        ),
        ("an empty name", one_rect, |app, what| {
            app.run("ui.symbolOptions", json!({})).unwrap();
            set_field(app, "name", json!(""));
            ok(app);
            assert_eq!(dialog_kind(app).as_deref(), Some(symbol_options::KIND), "{what}: the refused OK keeps the dialog open");
            assert!(symbols(app).is_empty(), "{what}: no symbol");
            cancel(app);
            assert_eq!(dialog_kind(app), None, "{what}");
            assert!(symbols(app).is_empty(), "{what}: Cancel makes no symbol");
        }),
        ("a taken name", leaf_and_dot, |app, what| {
            app.run("ui.symbolOptions", json!({"edit": true, "name": "Dot"})).unwrap();
            set_field(app, "name", json!("Leaf"));
            ok(app);
            assert_eq!(dialog_kind(app).as_deref(), Some(symbol_options::KIND), "{what}: the refused OK keeps the dialog open");
            assert_eq!(symbols(app), ["Leaf", "Dot"], "{what}: nothing renamed");
            cancel(app);
        }),
        ("New Symbol and F8's command", one_rect, |app, what| {
            app.run("ui.symbolOptions", json!({})).unwrap();
            let keyed = top(app).map(|(_, key, fields)| (key, fields)).expect("F8's command opens a dialog");
            let mut p = panel(one_rect());
            p.click("New Symbol");
            let clicked = top(&p.app()).map(|(_, key, fields)| (key, fields));
            assert_eq!(clicked, Some(keyed), "{what}: the button opens what the command opens");
            cancel(app);
            // With nothing selected, neither opens a dialog or makes a symbol.
            app.run("select.none", json!({})).unwrap();
            let e = app.run("ui.symbolOptions", json!({})).unwrap_err();
            assert!(e.contains("nothing selected"), "{what}: {e}");
            assert_eq!(dialog_kind(app), None, "{what}");
            cancel(&mut p.app());
            p.app().run("select.none", json!({})).unwrap();
            p.frame();
            assert!(!p.enabled("New Symbol"), "{what}: the button is off with nothing selected");
            assert!(symbols(app).is_empty() && symbols(&mut p.app()).is_empty(), "{what}: no symbol");
        }),
    ];
    for (what, setup, steps) in rows {
        steps(&mut windowed(setup()), what);
    }
}

/// The Control bar drops its rows by rank as it narrows: the symbol instances' controls (Edit
/// Symbol, Replace and Reset) leave as one group, by `rank::GROUP`, while the Transform link, which
/// leaves last, still shows.
#[test]
fn a_narrow_control_bar_drops_the_symbol_rows_before_the_transform_link() {
    /// Whether the bar `width` points wide shows Edit Symbol, Replace and the Transform link.
    fn shown_at(p: &mut Panel, width: f32) -> (bool, bool, bool) {
        p.room(vec2(width, 60.0));
        p.frame();
        (p.shows("Edit Symbol"), p.shows("Replace:"), p.shows("Transform"))
    }
    let mut app = leaf_and_dot();
    let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
    app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
    let mut p = harness(app, whole_bar);
    assert_eq!(shown_at(&mut p, 4000.0), (true, true, true), "a wide bar shows the symbol controls and the Transform link");
    let left_out = (1..80).map(|step| 4000.0 - 50.0 * step as f32).find_map(|width| {
        let shown = shown_at(&mut p, width);
        (!shown.0 || !shown.1).then_some((width, shown))
    });
    let (width, (edit, replace, transform)) = left_out.expect("a narrow bar leaves the symbol controls out");
    assert!(!edit && !replace, "{width} pt: Edit Symbol and Replace leave together");
    assert!(transform, "{width} pt: the Transform link stays");
}

/// `ui.symbolOptions` takes `keepArt` only as true or false, and only for a new symbol.
#[test]
fn symbol_options_checks_keep_art() {
    let mut app = windowed(leaf_dot_and_rect().0);
    for bad in [json!("yes"), json!(1)] {
        let e = app.run("ui.symbolOptions", json!({ "keepArt": bad })).unwrap_err();
        assert!(e.contains("`keepArt` must be true or false"), "{bad}: {e}");
    }
    let e = app.run("ui.symbolOptions", json!({"edit": true, "name": "Dot", "keepArt": true})).unwrap_err();
    assert!(e.contains("only to a new symbol"), "{e}");
    assert_eq!(dialog_kind(&app), None);
    app.run("ui.symbolOptions", json!({"edit": true, "name": "Dot", "keepArt": false})).unwrap();
    assert_eq!(dialog_kind(&app).as_deref(), Some(symbol_options::KIND));
}

/// #4918: Select All Unused picks the symbols nothing places, Delete Symbol asks once and
/// deletes them all in one undo step, and a click on a symbol picks one again.
#[test]
fn select_all_unused_picks_the_unused_symbols_and_delete_deletes_them() {
    let mut p = panel(abc_doc());
    fire_menu_row(&mut p.app(), ROW_SELECT_UNUSED, None, Mods::NONE);
    assert_eq!(marked(&p.app()), ["A", "C"]);
    let undo = undo_len(&p.app());
    fire_menu_row(&mut p.app(), ROW_DELETE, None, Mods::NONE);
    assert_eq!(delete_question(&p.app()), Some(false), "asked once, about both");
    choose(&mut p.app(), PromptChoice::Proceed);
    assert_eq!(symbols(&mut p.app()), ["B"]);
    assert_eq!(undo_len(&p.app()), undo + 1, "one undo step");
    assert!(marked(&p.app()).is_empty(), "the deleted ones are no longer picked");
    // Undone, they are back; Select All Unused again, then a click on a symbol picks just it.
    p.app().run("edit.undo", json!({})).unwrap();
    fire_menu_row(&mut p.app(), ROW_SELECT_UNUSED, None, Mods::NONE);
    assert_eq!(marked(&p.app()).len(), 2);
    p.click("A");
    assert!(marked(&p.app()).is_empty());
    // With nothing unused, the row reports it and picks nothing.
    p.app().run("symbol.delete", json!({"names": ["A", "C"]})).unwrap();
    fire_menu_row(&mut p.app(), ROW_SELECT_UNUSED, None, Mods::NONE);
    assert!(marked(&p.app()).is_empty());
}

/// Picks made in one document don't carry to another with symbols of the same names, and a
/// picked symbol that gained a use since (here in a brush's art) isn't deleted (#4918 review).
#[test]
fn picked_symbols_stay_with_their_document_and_are_rechecked_before_deleting() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let make = |app: &mut VectorcraftApp| {
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let id = app.run("symbol.new", json!({"name": "A"})).unwrap()["id"].as_u64().unwrap();
        app.run("edit.clear", json!({"ids": [id]})).unwrap();
    };
    make(&mut app);
    fire_menu_row(&mut app, ROW_SELECT_UNUSED, None, Mods::NONE);
    assert_eq!(marked(&app), ["A"]);
    make(&mut app);
    assert!(marked(&app).is_empty(), "not in the other document");
    // Back in the first one: A gains a use in a brush's art, then Delete (Alt: no question).
    fire_menu_row(&mut app, ROW_SELECT_UNUSED, None, Mods::NONE);
    let id = app.run("symbol.place", json!({"name": "A"})).unwrap()["id"].as_u64().unwrap();
    app.run("brush.new", json!({"type": "art", "name": "B", "ids": [id]})).unwrap();
    app.run("edit.clear", json!({"ids": [id]})).unwrap();
    let picked = marked(&app);
    assert_eq!(picked, ["A"]);
    delete(&mut app, &picked, None, true);
    assert_eq!(symbols(&mut app), ["A"], "still used by the brush, so kept");
}

/// Does the selection hold a symbol instance that `wanted` takes, found by walking every
/// selected subtree on each call: what the cached flags must match.
fn walked(app: &VectorcraftApp, wanted: impl Fn(&Node) -> bool) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| {
            let mut any = false;
            if let Some(n) = st.doc.node(*id) {
                n.walk(&mut |c| any |= matches!(c.kind, NodeKind::SymbolInstance { .. }) && wanted(c));
            }
            any
        })
    })
}

/// Any instance, one with appearance changes, one Reset Transformation changes: walked again.
fn walked_flags(app: &VectorcraftApp) -> [bool; 3] {
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return [false; 3] };
    [
        walked(app, |_| true),
        walked(app, |c| !c.extras().symbol_overrides.is_empty()),
        walked(app, |c| vectorcraft_engine::cmd::reset_transformation_changes(&doc, c)),
    ]
}

fn cached_flags(app: &mut VectorcraftApp) -> [bool; 3] {
    [has_instances_selected(app), changed_instances_selected(app), transformed_instances_selected(app)]
}

fn scans() -> usize {
    INSTANCE_SCANS.with(std::cell::Cell::get)
}

/// The Symbols and Properties panels' instance flags (Break Link, Reset Symbol Instance and the
/// Properties panel's Reset, Reset Transformation) are those a walk of the selection gives, for
/// plain, changed, scaled and grouped instances, several selected objects, nothing selected,
/// Undo and another document; the selection is walked once per document revision, however
/// often they are read.
#[test]
fn instance_flags_are_found_once_per_revision_and_match_a_walk_of_the_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap());
    let second = id(app.run("shape.rectangle", json!({"x": 60, "y": 10, "width": 40, "height": 40})).unwrap());
    let other = id(app.run("shape.rectangle", json!({"x": 300, "y": 10, "width": 40, "height": 40})).unwrap());
    // Reads every flag several times; the walk runs only when the revision changed.
    let check = |app: &mut VectorcraftApp, want: [bool; 3], what: &str| {
        let before = scans();
        for _ in 0..3 {
            assert_eq!(cached_flags(app), want, "{what}");
        }
        assert_eq!(walked_flags(app), want, "{what}: a walk of the selection agrees");
        assert!(scans() - before <= 1, "{what}: walked {} times", scans() - before);
    };
    app.run("select.set", json!({"ids": [rect]})).unwrap();
    check(&mut app, [false; 3], "no instance");
    // Two objects, so the instance's art is a group whose first object can be painted.
    let inst = id(app.run("symbol.new", json!({"name": "S", "ids": [rect, second], "type": "dynamic"})).unwrap());
    app.run("select.set", json!({"ids": [inst]})).unwrap();
    check(&mut app, [true, false, false], "a plain instance");
    app.run("symbol.instance.setAppearance", json!({"id": inst, "path": [0], "fill": "#ff0000"})).unwrap();
    app.run("select.set", json!({"ids": [inst]})).unwrap();
    check(&mut app, [true, true, false], "an instance with appearance changes");
    let scaled = id(app.run("symbol.place", json!({"name": "S", "x": 200, "y": 100})).unwrap());
    app.run("select.set", json!({"ids": [scaled]})).unwrap();
    app.run("object.scale", json!({"sx": 150, "sy": 50})).unwrap();
    check(&mut app, [true, false, true], "a scaled instance");
    app.run("select.set", json!({"ids": [scaled, other]})).unwrap();
    let group = id(app.run("object.group", json!({})).unwrap());
    app.run("select.set", json!({"ids": [group]})).unwrap();
    check(&mut app, [true, false, true], "an instance in a selected group");
    app.run("select.set", json!({"ids": [inst, group]})).unwrap();
    check(&mut app, [true, true, true], "several selected objects");
    app.run("select.set", json!({"ids": [other]})).unwrap();
    check(&mut app, [false; 3], "select.set to a path");
    app.run("select.none", json!({})).unwrap();
    check(&mut app, [false; 3], "nothing selected");
    // Undo takes the group apart again.
    app.run("edit.undo", json!({})).unwrap();
    let undone = walked_flags(&app);
    check(&mut app, undone, "after Undo");
    // At one revision the walk runs once; a new revision walks again.
    app.run("select.set", json!({"ids": [inst, scaled]})).unwrap();
    let before = scans();
    cached_flags(&mut app);
    cached_flags(&mut app);
    assert_eq!(scans(), before + 1);
    app.run("select.set", json!({"ids": [inst]})).unwrap();
    cached_flags(&mut app);
    assert_eq!(scans(), before + 2);
    // Another document has its own uid, so its flags are found again.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let plain = id(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap());
    app.run("select.set", json!({"ids": [plain]})).unwrap();
    check(&mut app, [false; 3], "another document");
}

/// The Properties panel (its Reset quick action) and the Symbols panel (Break Link) share one
/// walk of the selection while the document does not change.
#[test]
fn properties_and_symbols_panels_walk_the_selection_once_per_revision() {
    /// A selected instance with appearance changes of its own.
    fn changed_instance() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let rect = |app: &mut VectorcraftApp, x: f64| {
            app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap()
        };
        let ids = [rect(&mut app, 10.0), rect(&mut app, 60.0)];
        let inst = app.run("symbol.new", json!({"name": "S", "ids": ids, "type": "dynamic"})).unwrap()["id"].as_u64().unwrap();
        app.run("symbol.instance.setAppearance", json!({"id": inst, "path": [0], "fill": "#ff0000"})).unwrap();
        app.run("select.set", json!({"ids": [inst]})).unwrap();
        app
    }
    fn properties_panel(app: &mut VectorcraftApp, ui: &mut Ui) {
        super::super::properties::show(app, ui);
    }
    fn both_panels(app: &mut VectorcraftApp, ui: &mut Ui) {
        super::super::properties::show(app, ui);
        show(app, ui);
    }
    let before = scans();
    harness(changed_instance(), properties_panel);
    assert_eq!(scans(), before + 1, "the Properties panel reads the flags");
    let before = scans();
    let mut p = harness(changed_instance(), both_panels);
    for _ in 0..3 {
        p.frame();
    }
    assert_eq!(scans(), before + 1, "one walk for both panels over five frames");
    assert_eq!(cached_flags(&mut p.app()), [true, true, false]);
    assert_eq!(scans(), before + 1);
}

/// The picture the panel's tile shows of an imported symbol frames the symbol's whole art.
#[test]
fn an_imported_symbols_thumbnail_frames_its_whole_art() {
    // The symbol of the SVG import tests: an orange 20 × 20 square from (0, 0) with a white dot.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Star" data-name="Gold Star"><path d="M0 0H20V20H0Z" fill="#e0a000"/><circle cx="10" cy="10" r="4" fill="#fff"/></symbol></defs><use href="#Star" x="20" y="30"/></svg>"##;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("document.open", json!({"name": "star.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    let name = symbols(&mut app).into_iter().next().expect("the imported symbol");
    let st = app.session.active().unwrap();
    let key = document_symbols(&st.doc, st.revision).key(&name).expect("the symbol's thumbnail key");
    // The tile's picture, as the thumbnail store has the panel's picture source render it.
    let pictures = SymbolPictures { doc: &st.doc, name: &name };
    let ThumbRender::Ready(thumb) = pictures.render(&SymbolThumb::Symbol { key }, &ThumbRequest::for_cell(40.0, 1.0)) else {
        panic!("the thumbnail renders at once")
    };
    // The art fills it, a few pixels from each edge: its top-left corner is orange too, not only
    // the quarter the 20 × 20 square around the origin would frame.
    let [w, h] = thumb.size;
    let orange = |x: usize, y: usize| {
        let [r, g, b, a] = thumb.pixels[y * w + x].to_array();
        a == 255 && r > 200 && (130..190).contains(&g) && b < 40
    };
    let (inset_x, inset_y) = (w * 3 / 40, h * 3 / 40);
    assert!(orange(inset_x, inset_y) && orange(w - 1 - inset_x, h - 1 - inset_y), "{w} × {h}");
}

/// A document with symbol "Leaf", whose instance stands where its rectangle was, and symbol
/// "Dot", which no instance uses; "Dot" is current.
fn leaf_and_dot() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    for (name, x) in [("Leaf", 10), ("Dot", 150)] {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
        app.run("symbol.new", json!({"name": name})).unwrap();
    }
    // Dot's instance goes: no art uses Dot.
    app.run("edit.clear", json!({})).unwrap();
    app.run("symbol.setCurrent", json!({"name": "Dot"})).unwrap();
    app
}

/// `leaf_and_dot` with a selected 60 × 20 rectangle; returns its id.
fn leaf_dot_and_rect() -> (VectorcraftApp, NodeId) {
    let mut app = leaf_and_dot();
    let rect = NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [rect.0]})).unwrap();
    (app, rect)
}

/// The instances of symbol `name` in the active document, and the ordinary paths in it.
fn art(app: &VectorcraftApp, name: &str) -> (usize, usize) {
    let (mut instances, mut paths) = (0, 0);
    for l in &app.session.active().unwrap().doc.layers {
        l.walk(&mut |n| match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } if symbol == name => instances += 1,
            NodeKind::Path { .. } => paths += 1,
            _ => {}
        });
    }
    (instances, paths)
}

/// The bounds of the art on the symbol editing layer, rounded to 0.001 pt.
fn editing_art(app: &VectorcraftApp) -> Option<[f64; 4]> {
    let d = &app.session.active()?.doc;
    let layer = d.node(d.symbol_edit.as_ref()?.layer)?;
    let ids: Vec<NodeId> = layer.children()?.iter().map(|n| n.id).collect();
    let r = d.bounds_of(&ids, false)?;
    Some([r.x0, r.y0, r.x1, r.y1].map(|v| (v * 1000.0).round() / 1000.0))
}

/// Edit Symbol without an instance centers the symbol's art on the active artboard, and in a
/// document without artboards on the view's center, in a batch too.
#[test]
fn edit_symbol_from_the_panel_centers_the_art_on_the_active_artboard_or_the_view() {
    let mut app = leaf_and_dot();
    app.run("artboard.new", json!({"x": 500, "y": 0, "width": 200, "height": 100})).unwrap();
    app.run("select.none", json!({})).unwrap();
    crate::panels::artboards::select(&mut app, 1);
    app.run("symbol.edit", json!({"name": "Dot"})).unwrap();
    assert_eq!(editing_art(&app), Some([585.0, 35.0, 615.0, 65.0]));
    app.run("symbol.exitEditing", json!({})).unwrap();
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).artboards.clear();
    // A view with no artboard to fit to is never fitted.
    if let Some(v) = app.view_mut() {
        (v.fitted, v.center) = (false, vectorcraft_geom::Point::new(1000.0, 1000.0));
    }
    app.run("symbol.edit", json!({"name": "Dot"})).unwrap();
    assert_eq!(editing_art(&app), Some([985.0, 985.0, 1015.0, 1015.0]));
    // A coordinate the call gives stays; the other is the view's.
    app.run("symbol.exitEditing", json!({})).unwrap();
    app.run("symbol.edit", json!({"name": "Dot", "x": 2000})).unwrap();
    assert_eq!(editing_art(&app), Some([1985.0, 985.0, 2015.0, 1015.0]));
    // A batch's symbol.edit step gets the view's center as well.
    app.run("symbol.exitEditing", json!({})).unwrap();
    app.run("command.batch", json!({"commands": [{"command": "symbol.edit", "params": {"name": "Dot"}}]})).unwrap();
    assert_eq!(editing_art(&app), Some([985.0, 985.0, 1015.0, 1015.0]));
    // An artboard an earlier step makes is there when the symbol.edit step runs.
    app.run("symbol.exitEditing", json!({})).unwrap();
    let steps = json!([
        {"command": "artboard.new", "params": {"x": 500, "y": 0, "width": 200, "height": 100}},
        {"command": "symbol.edit", "params": {"name": "Dot"}},
    ]);
    app.run("command.batch", json!({ "commands": steps })).unwrap();
    assert_eq!(editing_art(&app), Some([585.0, 35.0, 615.0, 65.0]));
}

/// The symbol in symbol editing mode, if any.
fn editing(app: &VectorcraftApp) -> Option<String> {
    app.session.active()?.doc.symbol_edit.as_ref().map(|e| e.symbol.clone())
}

/// The first instance of symbol `name` in the active document.
fn instance_of(app: &VectorcraftApp, name: &str) -> Option<NodeId> {
    let mut found = None;
    for l in &app.session.active()?.doc.layers {
        l.walk(&mut |n| {
            if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
                && symbol == name
                && found.is_none()
            {
                found = Some(n.id);
            }
        });
    }
    found
}

/// `ui.panelView` sets the Symbols, Graphic Styles and Swatches panels' views as the view rows
/// of their panel menus do, and answers the current view without `view`. An unknown panel or
/// view, or a view of another panel, is an error and changes nothing.
#[test]
fn panel_view_sets_the_view_of_each_panel() {
    use super::super::{graphic_styles, swatches};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    for (i, name) in ["ant", "bee"].into_iter().enumerate() {
        app.run("shape.rectangle", json!({"x": 10.0 + 40.0 * i as f64, "y": 10, "width": 30, "height": 20})).unwrap();
        app.run("symbol.new", json!({"name": name})).unwrap();
    }
    assert_eq!(app.run("ui.panelView", json!({"panel": "symbols"})).unwrap(), json!({"panel": "symbols", "view": "thumbnail"}));
    assert_eq!(checked_view(&app), ["thumbnail"], "Thumbnail View by default");
    let r = app.run("ui.panelView", json!({"panel": "symbols", "view": "largeList"})).unwrap();
    assert_eq!(r, json!({"panel": "symbols", "view": "largeList"}));
    assert_eq!(checked_view(&app), ["largeList"], "the panel menu checks it");
    fire_menu_row(&mut app, "ui.panelView", Some(&json!({"panel": "symbols", "view": "smallList"})), Mods::NONE);
    assert_eq!(app.run("ui.panelView", json!({"panel": "symbols"})).unwrap()["view"], "smallList", "the menu row sets it too");
    for bad in [
        json!({"panel": "symbols", "view": "mediumThumb"}),
        json!({"panel": "symbols", "view": 3}),
        json!({"panel": "brushes", "view": "smallList"}),
        json!({"view": "largeList"}),
    ] {
        assert!(app.run("ui.panelView", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(app.ui.symbols_view, View::SmallList, "unchanged");
    let r = app.run("ui.panelView", json!({"panel": "graphicStyles", "view": "largeList"})).unwrap();
    assert_eq!(r, json!({"panel": "graphicStyles", "view": "largeList"}));
    assert_eq!(app.ui.graphic_styles_view, graphic_styles::View::LargeList);
    assert_eq!(app.run("ui.panelView", json!({"panel": "swatches"})).unwrap()["view"], "mediumThumb");
    app.run("ui.panelView", json!({"panel": "swatches", "view": "largeThumb"})).unwrap();
    assert!(app.run("ui.panelView", json!({"panel": "swatches", "view": "thumbnail"})).is_err());
    assert_eq!(app.ui.swatches_view, swatches::View::LargeThumb, "unchanged by the error");
    assert_eq!(app.ui.symbols_view, View::SmallList, "each panel keeps its own view");
}

/// The panel menu's Edit Symbol edits the selected instance's symbol, and with no instance
/// selected the symbol current in the panel.
#[test]
fn the_panel_menus_edit_symbol_edits_the_instances_symbol_or_the_panels() {
    let mut app = leaf_and_dot();
    fire_menu_row(&mut app, "symbol.edit", None, Mods::NONE);
    assert_eq!(editing(&app).as_deref(), Some("Dot"), "no instance selected: the panel's symbol");
    app.run("symbol.exitEditing", json!({})).unwrap();
    let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
    app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
    fire_menu_row(&mut app, "symbol.edit", None, Mods::NONE);
    assert_eq!(editing(&app).as_deref(), Some("Leaf"), "the selected instance's symbol");
    assert_eq!(instance_of(&app, "Leaf"), Some(leaf), "the instance keeps its link");
}

/// Delete Symbol, from the panel's button or its menu, asks "Delete the selected symbol?" for a
/// symbol no instance uses, and the in-use question for one with instances, before changing
/// the document; No leaves it as it was and Yes deletes the symbol. Alt-click on the button
/// deletes an unused symbol without asking, and still asks for one with instances.
#[test]
fn delete_symbol_asks_first_and_alt_click_deletes_an_unused_symbol_at_once() {
    let mut p = panel(leaf_and_dot());
    let before = p.app().session.active().unwrap().doc.clone();
    let unchanged = |p: &Panel| Arc::ptr_eq(&before, &p.app().session.active().unwrap().doc);
    p.click("Delete Symbol");
    assert_eq!(delete_question(&p.app()), Some(false));
    assert!(unchanged(&p), "nothing changes while it asks");
    choose(&mut p.app(), PromptChoice::Cancel);
    assert_eq!(dialog_kind(&p.app()), None);
    assert!(unchanged(&p), "No changes nothing");
    fire_menu_row(&mut p.app(), ROW_DELETE, None, Mods::NONE);
    assert_eq!(delete_question(&p.app()), Some(false), "the menu row asks too");
    assert!(unchanged(&p));
    choose(&mut p.app(), PromptChoice::Proceed);
    assert_eq!(dialog_kind(&p.app()), None);
    assert_eq!(symbols(&mut p.app()), ["Leaf"], "Yes deletes it");
    // Alt-click on the button: an unused symbol goes without a question.
    p.app().run("edit.undo", json!({})).unwrap();
    p.app().run("symbol.setCurrent", json!({"name": "Dot"})).unwrap();
    p.click_with("Delete Symbol", Modifiers::ALT);
    assert_eq!(dialog_kind(&p.app()), None);
    assert_eq!(symbols(&mut p.app()), ["Leaf"]);
    // A symbol with an instance asks, with Alt held too, from the button and from the menu.
    p.app().run("symbol.setCurrent", json!({"name": "Leaf"})).unwrap();
    let before = p.app().session.active().unwrap().doc.clone();
    for alt in [Modifiers::NONE, Modifiers::ALT] {
        p.click_with("Delete Symbol", alt);
        assert_eq!(delete_question(&p.app()), Some(true), "{alt:?}");
        choose(&mut p.app(), PromptChoice::Cancel);
    }
    fire_menu_row(&mut p.app(), ROW_DELETE, None, Mods::NONE);
    assert_eq!(delete_question(&p.app()), Some(true));
    choose(&mut p.app(), PromptChoice::Cancel);
    assert!(Arc::ptr_eq(&before, &p.app().session.active().unwrap().doc), "Cancel changes nothing");
    assert_eq!(art(&p.app(), "Leaf"), (1, 0));
}

/// The in-use question: Expand Instances deletes the symbol and turns its instance into
/// ordinary art, Delete Instances deletes the symbol and its instance, each in one undo step.
#[test]
fn the_in_use_question_expands_or_deletes_the_instances() {
    for (choice, left) in [(PromptChoice::Proceed, (0, 1)), (PromptChoice::Alternate, (0, 0))] {
        let mut app = windowed(leaf_and_dot());
        symbol_delete::ask(&mut app, "Leaf", false);
        assert_eq!(delete_question(&app), Some(true));
        choose(&mut app, choice);
        assert_eq!(dialog_kind(&app), None);
        assert_eq!(symbols(&mut app), ["Dot"], "{choice:?}");
        assert_eq!(art(&app, "Leaf"), left, "{choice:?}");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!((symbols(&mut app), art(&app, "Leaf")), (vec!["Leaf".to_string(), "Dot".to_string()], (1, 0)), "one undo step");
    }
}

/// A question asked in one document deletes nothing once another is active, and Yes for a
/// symbol that has gained an instance since asks the in-use question instead.
#[test]
fn a_stale_delete_symbol_question_deletes_nothing() {
    let mut app = windowed(leaf_and_dot());
    symbol_delete::ask(&mut app, "Dot", false);
    assert_eq!(delete_question(&app), Some(false));
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    app.run("symbol.new", json!({"name": "Dot"})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    choose(&mut app, PromptChoice::Proceed);
    assert_eq!(dialog_kind(&app), None);
    assert_eq!(symbols(&mut app), ["Dot"], "the other document keeps its symbol");
    // Back in the first document: Dot gains an instance while the question is open.
    app.run("document.activate", json!({"index": 0})).unwrap();
    symbol_delete::ask(&mut app, "Dot", false);
    assert_eq!(delete_question(&app), Some(false));
    app.run("symbol.place", json!({"name": "Dot"})).unwrap();
    choose(&mut app, PromptChoice::Proceed);
    assert_eq!(delete_question(&app), Some(true));
    assert_eq!(symbols(&mut app), ["Leaf", "Dot"]);
    assert_eq!(art(&app, "Dot"), (1, 0));
}

fn refuses_symbol_approval_after_revert(name: &str, choice: PromptChoice, after_fresh_approval: (usize, usize)) {
    let mut app = windowed(leaf_and_dot());
    let file = std::env::temp_dir().join(format!("vectorcraft-symbol-delete-{name}-{choice:?}-{}.vectorcraft", std::process::id()));
    app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|err| err.to_string())));
    app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|err| err.to_string())));
    app.run("file.saveAs", json!({"path": file.to_string_lossy()})).unwrap();
    let leaf = instance_of(&app, "Leaf").unwrap();
    let st = app.session.active().unwrap();
    let (uid, reverts) = (st.uid, st.reverts);

    app.run("symbol.place", json!({"name": "Leaf"})).unwrap();
    assert_eq!(art(&app, "Leaf"), (2, 0));
    symbol_delete::ask(&mut app, name, false);
    assert_eq!(delete_question(&app), Some(name == "Leaf"));
    app.run("file.revert", json!({"confirmed": true})).unwrap();
    let _ = std::fs::remove_file(file);
    let st = app.session.active().unwrap();
    assert_eq!((st.uid, st.reverts), (uid, reverts + 1));
    assert_eq!(instance_of(&app, "Leaf"), Some(leaf));
    assert_eq!(art(&app, "Leaf"), (1, 0));
    let undo = st.history.undo.len();

    choose(&mut app, choice);
    assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "an old approval keeps the restored symbols");
    assert_eq!(art(&app, "Leaf"), (1, 0), "the restored instance stays unmodified");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    assert_eq!(app.ui.status, "The document was reverted. Reopen this dialog to continue.");

    symbol_delete::ask(&mut app, name, false);
    choose(&mut app, choice);
    assert!(!symbols(&mut app).iter().any(|symbol| symbol == name));
    assert_eq!(art(&app, "Leaf"), after_fresh_approval);
}

#[test]
fn old_delete_symbol_yes_keeps_the_symbol_restored_by_revert() {
    refuses_symbol_approval_after_revert("Dot", PromptChoice::Proceed, (1, 0));
}

#[test]
fn old_expand_instances_keeps_the_instance_restored_by_revert() {
    refuses_symbol_approval_after_revert("Leaf", PromptChoice::Proceed, (0, 1));
}

#[test]
fn old_delete_instances_keeps_the_instance_restored_by_revert() {
    refuses_symbol_approval_after_revert("Leaf", PromptChoice::Alternate, (0, 0));
}

/// The panel menu's Reset Transformation and the Control bar's Reset are enabled only while a
/// selected instance's scale, rotation or shear differs from its symbol's, and give the
/// instance its symbol's again.
#[test]
fn reset_transformation_is_enabled_for_a_transformed_instance_and_resets_it() {
    let mut app = leaf_and_dot();
    let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
    app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
    let xf = |app: &VectorcraftApp| match app.session.active().unwrap().doc.node(leaf).map(|n| &n.kind) {
        Some(NodeKind::SymbolInstance { xf, .. }) => *xf,
        _ => panic!("the Leaf instance"),
    };
    let master = xf(&app);
    let row_enabled = |app: &VectorcraftApp| menu_row(app, "symbol.instance.resetTransform", None).1;
    // At its symbol's scale the menu row is disabled.
    assert!(!row_enabled(&app), "the row is disabled at the symbol's scale");
    app.run("object.rotate", json!({"angle": 30})).unwrap();
    assert!(row_enabled(&app), "a turned instance enables it");
    fire_menu_row(&mut app, "symbol.instance.resetTransform", None, Mods::NONE);
    let close = |a: vectorcraft_geom::Affine, b: vectorcraft_geom::Affine| a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-9);
    assert!(close(xf(&app), master), "the menu row resets the instance: {:?} vs {master:?}", xf(&app));
    // The Control bar's Reset.
    let mut p = harness(app, symbol_bar);
    assert!(!p.enabled("Reset"), "Reset is disabled at the symbol's scale");
    let before = p.app().session.active().unwrap().doc.clone();
    p.click("Reset");
    assert!(Arc::ptr_eq(&before, &p.app().session.active().unwrap().doc), "a click on the disabled Reset changes nothing");
    p.app().run("object.scale", json!({"sx": 150, "sy": 50})).unwrap();
    p.frame();
    assert!(p.enabled("Reset"));
    p.click("Reset");
    assert!(close(xf(&p.app()), master), "the Control bar's Reset resets the instance: {:?} vs {master:?}", xf(&p.app()));
}

/// The current symbol as `symbol.list` answers it, read by running the command.
fn listed_current(app: &mut VectorcraftApp) -> Option<String> {
    app.run("symbol.list", json!({})).ok().and_then(|v| v["current"].as_str().map(str::to_string))
}

fn store_current(app: &mut VectorcraftApp, v: Value) {
    Arc::make_mut(&mut app.session.doc_mut().unwrap().doc).unknown.insert("currentSymbol".into(), v);
}

/// The panel's current symbol is the one `symbol.list` answers: with no document, with no
/// symbols, with several symbols, with an instance of another symbol selected, and with a
/// stored name no symbol has.
#[test]
fn current_is_the_current_symbol_list_answers() {
    fn check(app: &mut VectorcraftApp, want: Option<&str>, case: &str) {
        let read = current(app);
        assert_eq!(read.as_deref(), want, "{case}");
        assert_eq!(read, listed_current(app), "{case}: symbol.list");
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    check(&mut app, None, "no document");
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    check(&mut app, None, "a document without symbols");
    for (name, x) in [("Leaf", 10), ("Dot", 80), ("Star", 150)] {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
        app.run("symbol.new", json!({"name": name})).unwrap();
    }
    check(&mut app, Some("Star"), "several symbols, the newest current");
    app.run("symbol.setCurrent", json!({"name": "Leaf"})).unwrap();
    check(&mut app, Some("Leaf"), "the first symbol made current");
    let dot = instance_of(&app, "Dot").expect("a Dot instance");
    app.run("select.set", json!({"ids": [dot.0]})).unwrap();
    check(&mut app, Some("Leaf"), "an instance of Dot selected");
    app.run("symbol.setCurrent", json!({"name": "Dot"})).unwrap();
    check(&mut app, Some("Dot"), "a symbol that is not first");
    store_current(&mut app, json!("Gone"));
    check(&mut app, Some("Leaf"), "a stored name no symbol has");
    store_current(&mut app, json!(7));
    check(&mut app, Some("Leaf"), "a stored value that is not a name");
}

/// Idle frames of the panel and of its menu run no command (`symbol.list` included), so
/// they leave `run_count`, which the native menus watch, as it was.
#[test]
fn idle_frames_of_the_panel_and_its_menu_run_no_command() {
    let mut p = harness(leaf_and_dot(), panel_and_menu);
    let commands = p.app().run_count;
    p.frame();
    p.frame();
    assert_eq!(p.app().run_count, commands, "the panel and its menu");
    assert_eq!(current(&p.app()).as_deref(), Some("Dot"));
}

/// A symbol dragged from one document's panel and dropped on New Symbol after another document
/// with a symbol of the same name became active is duplicated in neither document.
#[test]
fn a_symbol_dragged_from_another_document_is_not_duplicated() {
    let mut p = harness(leaf_and_dot(), second_document_mid_drag);
    p.drag("Leaf", "New Symbol", Modifiers::NONE);
    let mut app = p.app();
    assert_eq!(symbols(&mut app), ["Leaf"], "no copy in the active document");
    assert_eq!(dialog_kind(&app), None);
    assert!(app.session.set_active(0));
    assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "no copy in the first document");
}

/// The art of symbol `name` in the active document.
fn symbol_art(app: &VectorcraftApp, name: &str) -> Arc<Node> {
    let doc = &app.session.active().unwrap().doc;
    doc.symbols.iter().find(|s| s.name == name).map(|s| s.art.clone()).unwrap_or_else(|| panic!("a {name} symbol"))
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// `leaf_and_dot` in `view` with a selected 40 × 20 rectangle named "Art" and Leaf's instance
/// named "Placed": Layers rows that share no name with a symbol. Returns the rectangle and the
/// instance.
fn alt_drop_doc(view: &str) -> (VectorcraftApp, NodeId, NodeId) {
    let mut app = leaf_and_dot();
    app.run("ui.panelView", json!({"panel": "symbols", "view": view})).unwrap();
    let shape = NodeId(app.run("shape.rectangle", json!({"x": 200, "y": 120, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap());
    name_object(&mut app, shape, "Art");
    let placed = instance_of(&app, "Leaf").expect("a Leaf instance");
    name_object(&mut app, placed, "Placed");
    (app, shape, placed)
}

/// Art dragged from a Layers row and released with Alt (Option on macOS) held on a symbol's tile
/// or row, in each view, redefines that symbol and not the current one: its instances take the
/// art, the art becomes an instance of it, and no symbol is added, in one undo step.
#[test]
fn alt_art_drop_on_a_symbol_redefines_that_symbol_in_one_step() {
    for view in ["thumbnail", "smallList", "largeList"] {
        let (app, _, placed) = alt_drop_doc(view);
        let (leaf, dot) = (symbol_art(&app, "Leaf"), symbol_art(&app, "Dot"));
        let undo = undo_len(&app);
        let mut p = harness(app, art_and_symbols);
        p.drag("Art", "Leaf", Modifiers::ALT);
        let mut app = p.app();
        assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{view}: no symbol added");
        assert_eq!(undo_len(&app), undo + 1, "{view}: one undo step");
        // The Leaf instance there was takes the 40 × 20 rectangle's shape, where the old art was square.
        let doc = app.session.active().unwrap().doc.clone();
        let shape = doc.node(placed).and_then(|n| n.geometric_bounds()).map(|b| b.width() / b.height());
        assert!(shape.is_some_and(|r| (r - 2.0).abs() < 1e-6), "{view}: Leaf is the dropped art: {shape:?}");
        assert!(Arc::ptr_eq(&dot, &symbol_art(&app, "Dot")), "{view}: the current symbol Dot is unchanged");
        assert_eq!(art(&app, "Leaf"), (2, 0), "{view}: the art became a Leaf instance");
        app.run("edit.undo", json!({})).unwrap();
        assert!(Arc::ptr_eq(&leaf, &symbol_art(&app, "Leaf")), "{view}: undo restores Leaf");
        assert_eq!(art(&app, "Leaf"), (1, 1), "{view}: undo restores the art");
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(art(&app, "Leaf"), (2, 0), "{view}: redo");
    }
}

/// An Alt drop that cannot redefine the symbol under the pointer changes nothing, makes no new
/// symbol and adds no undo step: art holding an instance of the symbol, which the redefine
/// refuses, a layer, and art released outside the list.
#[test]
fn an_alt_art_drop_that_cannot_redefine_changes_nothing() {
    let (app, _, _) = alt_drop_doc("thumbnail");
    let leaf = symbol_art(&app, "Leaf");
    let undo = undo_len(&app);
    let mut p = harness(app, art_and_symbols);
    let cases = [("a Leaf instance", "Placed", "Leaf"), ("a layer", "Layer 1", "Leaf"), ("art released outside the list", "Art", "Symbol Options")];
    for (case, from, to) in cases {
        p.drag(from, to, Modifiers::ALT);
        let mut app = p.app();
        assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{case}: no symbol added");
        assert!(Arc::ptr_eq(&leaf, &symbol_art(&app, "Leaf")), "{case}: Leaf is unchanged");
        assert_eq!(undo_len(&app), undo, "{case}: no undo step");
    }
}
