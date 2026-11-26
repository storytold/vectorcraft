use std::cell::{RefCell, RefMut};
use std::rc::Rc;

use craft::behavior::keyed as keyed_rows;
use craft::ui::contract::{ListDrag, Role, ShowCollection as _};
use craft::ui::display::Picture;
use craft::ui::list::{List, ListRow, ThumbAspect};
use craft::ui::testing::layout::in_region;
use craft::ui::testing::{Fixture, Target, pointer};
use egui::{Modifiers, PointerButton, Pos2, vec2};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::*;
use crate::dialogs::{brush_options, new_brush};
use crate::i18n::LangExt as _;

/// Where the panel sits in the trial's window, 236 pt wide as in the dock, and the drag source under it.
const PANEL: Rect = Rect { min: Pos2 { x: 0.0, y: 0.0 }, max: Pos2 { x: 236.0, y: 1000.0 } };
const SOURCE: Rect = Rect { min: Pos2 { x: 0.0, y: 1010.0 }, max: Pos2 { x: 236.0, y: 1050.0 } };
/// The drag source's one row.
const DRAGGED: &str = "Dragged";

type Shared = Rc<RefCell<VectorcraftApp>>;
type Source = Rc<RefCell<Option<Box<dyn Fn(&mut Ui)>>>>;

/// The Brushes panel of an app, shown in a fixture with a drag source under it: art off the canvas or a library
/// brush, dragged and dropped through the drag session.
struct Trial {
    app: Shared,
    fixture: Fixture,
    source: Source,
}

impl Trial {
    /// A trial of a new 200 × 100 pt document, shown.
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let mut trial = Self::of(app);
        trial.fixture.settle();
        trial
    }

    /// A trial of `app`, not shown yet.
    fn of(app: VectorcraftApp) -> Self {
        let app: Shared = Rc::new(RefCell::new(app));
        let source: Source = Rc::default();
        let (shown, dragged) = (app.clone(), source.clone());
        let mut fixture = Fixture::new(move |ui| {
            in_region(ui, PANEL, |ui| show(&mut shown.borrow_mut(), ui));
            if let Some(source) = dragged.borrow().as_ref() {
                in_region(ui, SOURCE, |ui| source(ui));
            }
        });
        fixture.room(vec2(260.0, 1060.0));
        app.borrow_mut().ui_ctx = Some(fixture.ctx().clone());
        Self { app, fixture, source }
    }

    fn app(&self) -> RefMut<'_, VectorcraftApp> {
        self.app.borrow_mut()
    }

    fn run(&self, id: &str, params: Value) -> Value {
        self.app().run(id, params).unwrap()
    }

    /// The brush row, footer action or drag source named `label`.
    fn target(&self, label: &str) -> Target {
        self.fixture.by_label(label).unwrap_or_else(|| panic!("no `{label}`"))
    }

    /// The open menu's row named `label`.
    fn menu_row(&self, label: &str) -> Target {
        let tree = self.fixture.tree();
        let rows = self.fixture.by_role(Role::MENU_ITEM);
        rows.into_iter().find(|row| tree.name_of(row.node) == Some(label)).unwrap_or_else(|| panic!("no menu row `{label}`"))
    }

    fn enabled(&self, target: &Target) -> bool {
        !self.fixture.tree().is_disabled_key(target.node)
    }

    /// `button` pressed at `from`, moved to `to` in four steps when they differ, and released there, with `modifiers`
    /// held throughout and carried by the press and the release; then a frame with no input.
    fn gesture(&mut self, from: Pos2, to: Pos2, button: PointerButton, modifiers: Modifiers) {
        let event = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button, pressed, modifiers };
        let fixture = &mut self.fixture;
        fixture.hold(modifiers);
        fixture.frame(vec![pointer::moved(from)]);
        fixture.frame(vec![event(from, true)]);
        if from != to {
            for step in 1..=4 {
                fixture.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 4.0))]);
            }
        }
        fixture.frame(vec![event(to, false)]);
        fixture.frame(Vec::new());
        fixture.hold(Modifiers::NONE);
    }

    /// A primary click on `label` with `modifiers`.
    fn click(&mut self, label: &str, modifiers: Modifiers) {
        let at = self.target(label).center();
        self.gesture(at, at, PointerButton::Primary, modifiers);
    }

    /// A secondary click on `label`, which opens its menu.
    fn right_click(&mut self, label: &str) {
        let at = self.target(label).center();
        self.gesture(at, at, PointerButton::Secondary, Modifiers::NONE);
        self.fixture.settle();
    }

    /// The drag source under the panel carries `payload`.
    fn drags<P: Clone + Send + Sync + 'static>(&mut self, payload: P) {
        *self.source.borrow_mut() = Some(Box::new(move |ui: &mut Ui| {
            let payload = payload.clone();
            let _ = List::<u32>::new("Drag source").drag_payload(move |_: &ListDrag<u32>| Some(payload.clone())).show_items(
                ui,
                &keyed_rows(&[0u32], |key: &u32| *key),
                &mut |_, _| ListRow::new(DRAGGED),
            );
        }));
        self.fixture.settle();
    }

    /// The drag source's payload dropped at `at` with `modifiers`.
    fn drop_at(&mut self, at: Pos2, modifiers: Modifiers) {
        let from = self.target(DRAGGED).center();
        self.gesture(from, at, PointerButton::Primary, modifiers);
    }

    /// The key and fields of the dialog open on top.
    fn dialog(&self) -> Option<(Option<String>, Value)> {
        let dialogs = self.app.borrow().dialogs()?;
        dialogs.with(|host| host.open_dialogs().pop().map(|d| (d.key, d.fields.unwrap_or_default())))
    }

    fn dialog_key(&self) -> Option<String> {
        self.dialog().and_then(|d| d.0)
    }

    /// OK (`ok`) or Cancel on the dialog open on top, then the pump that takes its answer.
    fn answer(&mut self, ok: bool) {
        let ctx = self.fixture.ctx().clone();
        let dialogs = self.app.borrow().dialogs().expect("a window");
        dialogs.with(|host| {
            let id = host.open_dialogs().last().expect("an open dialog").id;
            if ok { host.confirm_by_id(&ctx, id) } else { host.cancel_by_id(&ctx, id) }
        });
        crate::dialogs::host::pump(&mut self.app(), &ctx);
    }

    /// What the panel shows selected: the brushes Delete Brush and Duplicate Brush act on.
    fn shown_selected(&self) -> Vec<String> {
        let app = self.app.borrow();
        let (list, current) = brushes(&app);
        targets(&app, &list, current.as_deref())
    }
}

/// A rectangle at `x`, selected.
fn rect(t: &Trial, x: f64) -> NodeId {
    NodeId(t.run("shape.rectangle", json!({"x": x, "y": 10, "width": 40, "height": 40}))["id"].as_u64().unwrap())
}

/// Stroke previews rendered while the panel settles.
fn rendered(t: &mut Trial) -> usize {
    counters::RENDERED.with(|r| r.set(0));
    t.fixture.settle();
    counters::RENDERED.with(std::cell::Cell::get)
}

/// Pointer and menu events on the panel run their commands: a click makes a brush current and opens nothing, a
/// double click opens the brush's Brush Options, the footer's New Brush and the menu's New Brush… open New Brush
/// and make no brush, and Delete Brush released with Alt (Option on macOS) deletes a brush no stroke uses at once,
/// in one undo step.
#[test]
fn panel_events_run_their_commands() {
    type Act = fn(&mut Trial, &str);
    type Check = fn(&Trial, &str, &[String], usize);
    let nothing: Act = |_, _| {};
    let unchanged: Check = |t, what, before, _| assert_eq!(names(&t.app()), before, "{what}: no brush made or removed");
    // (what, setup, act, the dialog it opens, what else it did); the undo steps are counted after the setup.
    let cases: [(&str, Act, Act, Option<&str>, Check); 5] = [
        (
            "a click",
            nothing,
            |t, first| t.click(first, Modifiers::NONE),
            None,
            |t, what, before, _| {
                assert_eq!(brushes(&t.app()).1.as_deref(), Some(before[0].as_str()), "{what}: the brush is current");
            },
        ),
        (
            "a double click",
            nothing,
            |t, first| {
                t.click(first, Modifiers::NONE);
                t.click(first, Modifiers::NONE);
            },
            Some(brush_options::KIND),
            |t, what, before, _| {
                let (_, fields) = t.dialog().unwrap();
                assert_eq!((fields["name"].clone(), fields["sizeMode"].clone()), (json!(before[0]), json!("fixed")), "{what}");
            },
        ),
        ("the footer's New Brush", nothing, |t, _| t.click("New Brush", Modifiers::NONE), Some(new_brush::KIND), unchanged),
        (
            "the menu's New Brush…",
            nothing,
            |t, _| {
                let row = menu_rows(&t.app()).into_iter().find(|row| matches!(row, PanelRow::Command(c) if c.id == "ui.newBrush"));
                assert!(row.is_some(), "the panel menu has New Brush…");
                let mut states = CommandStates::new();
                menu_states(&t.app(), &mut states);
                assert_eq!(states.get(&CommandRef::new("ui.newBrush")), Some(&CommandState::new().enabled(true)));
                t.run("ui.newBrush", json!({}));
            },
            Some(new_brush::KIND),
            unchanged,
        ),
        (
            "Delete Brush released with Alt",
            |t, first| t.click(first, Modifiers::NONE),
            |t, _| t.click("Delete Brush", Modifiers::ALT),
            None,
            |t, what, before, steps| {
                assert!(!names(&t.app()).contains(&before[0]), "{what}: deleted");
                assert_eq!(undo_count(&t.app()), steps + 1, "{what}: one step");
            },
        ),
    ];
    for (what, setup, act, dialog, check) in cases {
        let mut t = Trial::new();
        let before = names(&t.app());
        assert_eq!(brushes(&t.app()).0[0].1, "calligraphic");
        setup(&mut t, &before[0]);
        let steps = undo_count(&t.app());
        act(&mut t, &before[0]);
        assert_eq!(t.dialog_key().as_deref(), dialog, "{what}");
        check(&t, what, &before, steps);
    }
}

/// Drops on the panel run their commands. Art from the canvas dropped on the list opens New Brush for that art, whose
/// OK twice makes an art brush of it; with Alt (Option on macOS) on a scatter, art or pattern brush it replaces that
/// brush's art in one undo step without asking (no stroke uses it), and on a calligraphic or bristle brush it opens New
/// Brush too. A brush library's brush is added; a library folder or another kind's item adds nothing. A brush dragged
/// out of the panel carries its key.
#[test]
fn drops_on_the_panel_run_their_commands() {
    // Plain art on Dots, and Alt art on a calligraphic and a bristle brush: New Brush, and no brush changes.
    for (on, modifiers) in [("Dots", Modifiers::NONE), ("3 pt. Round", Modifiers::ALT), ("Bristle Round", Modifiers::ALT)] {
        let mut t = Trial::new();
        let star = t.run("shape.star", json!({"cx": 50, "cy": 50, "radius1": 20, "radius2": 9, "points": 5}))["id"].as_u64().unwrap();
        // Nothing selected: what New Brush makes comes from the dropped art.
        t.run("select.none", json!({}));
        let document = t.app().session.active().unwrap().uid;
        t.drags(PanelDrag::Art { document, ids: vec![NodeId(star)] });
        let (lib, before) = (t.run("brush.list", json!({})), brushes(&t.app()).0);
        let at = t.target(on).center();
        t.drop_at(at, modifiers);
        assert_eq!(t.dialog_key().as_deref(), Some(new_brush::KIND), "{on}");
        assert_eq!(t.run("brush.list", json!({})), lib, "{on}");
        t.answer(true);
        t.answer(true);
        let after = brushes(&t.app()).0;
        let made: Vec<_> = after.iter().filter(|b| !before.contains(b)).collect();
        assert!(matches!(made.as_slice(), [(_, ty)] if ty == "art"), "{on}: {made:?}");
    }
    // Alt art on Dots (scatter), Arrow (art) and Chain (pattern, its side tile): the rectangle is its art now.
    for (name, part) in [("Dots", "art"), ("Arrow", "art"), ("Chain", "side")] {
        let mut t = Trial::new();
        let id = t.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 12}))["id"].as_u64().unwrap();
        let document = t.app().session.active().unwrap().uid;
        t.drags(PanelDrag::Art { document, ids: vec![NodeId(id)] });
        let steps = undo_count(&t.app());
        let at = t.target(name).center();
        t.drop_at(at, Modifiers::ALT);
        assert_eq!(t.dialog_key(), None, "no stroke uses {name}: nothing asks");
        assert_eq!(undo_count(&t.app()), steps + 1, "{name}");
        let art: vectorcraft_doc::Node = serde_json::from_value(t.run("brush.get", json!({"name": name}))[part].clone()).unwrap();
        let b = art.geometric_bounds().unwrap();
        assert!((b.width() - 30.0).abs() < 1e-6 && (b.height() - 12.0).abs() < 1e-6, "{name}: {b:?}");
    }
    // Library brushes: a brush is added; a folder and another kind's item are not.
    let library = |kind: &str, id: &str, name: &str, folder: bool| {
        let key = LibraryKey { panel: library_panel::panel_id(kind, id), name: name.into(), folder };
        ListDrag::new(egui::Id::unique("library"), vec![key.clone()], key)
    };
    for (drag, added) in [
        (library(BrushLibraries::KIND, "pattern-borders", "Beads", false), true),
        (library(BrushLibraries::KIND, "pattern-borders", "Beads", true), false),
        (library("symbols", "pattern-borders", "Beads", false), false),
    ] {
        let mut t = Trial::new();
        t.drags(drag);
        let at = t.target("Dots").center() + vec2(0.0, 40.0);
        t.drop_at(at, Modifiers::NONE);
        let found = vectorcraft_brush::find(&t.app().session.active().unwrap().doc, "Beads").is_some();
        assert_eq!(found, added);
    }
    // A brush dragged out of the panel carries its key.
    let mut t = Trial::new();
    let first = names(&t.app())[0].clone();
    let from = t.target(&first).center();
    t.fixture.drag_at_pausing(from, SOURCE.center() - from, |f| {
        let drag = craft::ui::runtime::drag::active::<ListDrag<BrushKey>>(f.ctx()).expect("a brush drag");
        assert_eq!((drag.grabbed.name.as_str(), drag.grabbed.group), (first.as_str(), false));
    });
}

fn brush_of(app: &VectorcraftApp, id: NodeId) -> Option<String> {
    app.session.active().unwrap().doc.node(id).unwrap().appearance.stroke().and_then(|s| s.brush.clone())
}

fn names(app: &VectorcraftApp) -> Vec<String> {
    brushes(app).0.into_iter().map(|b| b.0).collect()
}

fn undo_count(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

fn delete_button(t: &mut Trial, modifiers: Modifiers) {
    t.click("Delete Brush", modifiers);
}

fn assert_delete_dialog(t: &Trial, message: &str) {
    assert_eq!(t.dialog_key().as_deref(), Some(crate::dialogs::brush_delete::KIND), "Delete Brush asks first");
    let (said, used) = crate::dialogs::brush_delete::open_question(&t.app()).expect("the Delete Brush question");
    assert_eq!(said, message);
    assert_eq!(used, message == IN_USE);
}

/// The Delete Brush question when a stroke uses one of the brushes.
const IN_USE: &str = "Strokes use one or more of these brushes. Take the brushes off those strokes and delete the brushes?";

/// Deleting several unused brushes asks one question about the selection, with no count in it.
#[test]
fn deleting_five_brushes_uses_one_selection_question() {
    let t = Trial::new();
    let five: Vec<String> = names(&t.app()).into_iter().take(5).collect();
    assert_eq!(five.len(), 5);
    let previous = crate::i18n::current();
    let polish = crate::i18n::Lang::from_code("pl").unwrap();
    crate::i18n::set_current(polish);
    delete(&mut t.app(), &five, false);
    let message = crate::dialogs::brush_delete::open_question(&t.app()).map(|q| q.0);
    crate::i18n::set_current(previous);
    assert_eq!(message, Some("Usunąć zaznaczone pędzle?".to_string()));
}

/// Runs the panel menu's own row `id`.
fn menu_run(t: &Trial, id: &str) {
    run(&mut t.app(), id, &json!({})).expect("a panel menu row").unwrap();
}

/// The first three brushes (the Calligraphic ones): a click on the first applies it to the
/// selected rectangle, and then a Shift-click on the third selects all three without applying
/// the third.
fn select_three(t: &mut Trial, rect: NodeId) -> Vec<String> {
    let (list, _) = brushes(&t.app());
    assert!(list[..3].iter().all(|b| b.1 == "calligraphic"), "{list:?}");
    let three: Vec<String> = list[..3].iter().map(|b| b.0.clone()).collect();
    t.click(&three[0], Modifiers::NONE);
    assert_eq!(brush_of(&t.app(), rect).as_deref(), Some(three[0].as_str()));
    t.click(&three[2], Modifiers::SHIFT);
    assert_eq!(brush_of(&t.app(), rect).as_deref(), Some(three[0].as_str()), "a Shift-click only selects");
    three
}

/// Selecting other art ends the multi-selection: selecting the first art again does not bring
/// it back, and with a path stroked with another brush selected, the panel highlights that
/// brush and Delete Brush deletes it alone.
#[test]
fn selecting_other_art_ends_the_brush_multi_selection() {
    let mut t = Trial::new();
    let a = rect(&t, 10.0);
    let three = select_three(&mut t, a);
    let mut selected = t.shown_selected();
    selected.sort();
    let mut all_three = three.clone();
    all_three.sort();
    assert_eq!(selected, all_three, "the Shift-click selected the three");
    // A path without a brush, then the first one again: the current brush is highlighted.
    rect(&t, 60.0);
    t.fixture.settle();
    assert_eq!(t.shown_selected(), [three[0].clone()]);
    t.run("select.set", json!({"ids": [a.0]}));
    t.fixture.settle();
    assert_eq!(t.shown_selected(), [three[0].clone()]);
    // A path stroked with the second brush.
    let c = rect(&t, 110.0);
    t.run("brush.apply", json!({"name": three[1]}));
    t.fixture.settle();
    assert_eq!(t.shown_selected(), [three[1].clone()]);
    menu_run(&t, DELETE_BRUSH);
    assert_delete_dialog(&t, IN_USE);
    assert!(names(&t.app()).contains(&three[1]));
    t.answer(true);
    let left = names(&t.app());
    assert!(left.contains(&three[0]) && !left.contains(&three[1]) && left.contains(&three[2]), "{left:?}");
    let brushes = {
        let app = t.app();
        (brush_of(&app, a), brush_of(&app, c))
    };
    assert_eq!((brushes.0.as_deref(), brushes.1), (Some(three[0].as_str()), None));
}

/// After other art is selected, a Shift-click selects the range from that art's brush, not from
/// the brush clicked before.
#[test]
fn a_shift_click_after_selecting_other_art_starts_from_its_brush() {
    let mut t = Trial::new();
    rect(&t, 10.0);
    let (list, _) = brushes(&t.app());
    t.click(&list[0].0, Modifiers::NONE);
    rect(&t, 100.0);
    t.run("brush.apply", json!({"name": list[2].0}));
    t.fixture.settle();
    t.click(&list[1].0, Modifiers::SHIFT);
    let mut selected = t.shown_selected();
    selected.sort();
    let mut range = vec![list[1].0.clone(), list[2].0.clone()];
    range.sort();
    assert_eq!(selected, range);
}

/// Select All Unused in the panel menu selects every brush but the one the rectangle is
/// painted with, and Delete Brush then deletes all of them.
#[test]
fn select_all_unused_selects_the_brushes_no_stroke_paints_with() {
    let mut t = Trial::new();
    let (list, _) = brushes(&t.app());
    let a = rect(&t, 10.0);
    t.run("brush.apply", json!({"name": list[1].0}));
    menu_run(&t, SELECT_UNUSED);
    t.fixture.settle();
    let mut selected = t.shown_selected();
    selected.sort();
    let mut unused: Vec<String> = list.iter().filter(|b| b.0 != list[1].0).map(|b| b.0.clone()).collect();
    unused.sort();
    assert_eq!(selected, unused);
    menu_run(&t, DELETE_BRUSH);
    assert_delete_dialog(&t, "Delete the selected brushes?");
    assert_eq!(names(&t.app()).len(), list.len());
    t.answer(true);
    assert_eq!(names(&t.app()), [list[1].0.clone()]);
    assert_eq!(brush_of(&t.app(), a), Some(list[1].0.clone()));
}

#[test]
fn delete_button_cancel_keeps_selected_brushes_strokes_and_history() {
    let mut t = Trial::new();
    let rect = rect(&t, 10.0);
    let three = select_three(&mut t, rect);
    let before = {
        let app = t.app();
        (names(&app), brush_of(&app, rect), undo_count(&app))
    };
    delete_button(&mut t, Modifiers::NONE);
    assert_delete_dialog(&t, IN_USE);
    assert_eq!(
        {
            let app = t.app();
            (names(&app), brush_of(&app, rect), undo_count(&app))
        },
        before
    );
    t.answer(false);
    assert_eq!(t.dialog(), None);
    assert_eq!(
        {
            let app = t.app();
            (names(&app), brush_of(&app, rect), undo_count(&app))
        },
        before
    );
    assert_eq!(before.1.as_deref(), Some(three[0].as_str()));
    t.run("brush.delete", json!({"name": three[0]}));
    assert_eq!(t.dialog(), None);
    assert_eq!(undo_count(&t.app()), before.2 + 1);
    t.run("brush.delete", json!({"names": [three[1], three[2]]}));
    assert_eq!(t.dialog(), None);
    assert_eq!(undo_count(&t.app()), before.2 + 2);
}

#[test]
fn alt_delete_button_asks_when_a_selected_brush_is_used() {
    let mut t = Trial::new();
    let rect = rect(&t, 10.0);
    let three = select_three(&mut t, rect);
    let before = undo_count(&t.app());
    delete_button(&mut t, Modifiers::NONE);
    assert_delete_dialog(&t, IN_USE);
    t.answer(false);
    delete_button(&mut t, Modifiers::ALT);
    assert_delete_dialog(&t, IN_USE);
    t.answer(false);
    let left = names(&t.app());
    assert!(three.iter().all(|name| left.contains(name)));
    assert_eq!(brush_of(&t.app(), rect).as_deref(), Some(three[0].as_str()));
    assert_eq!(undo_count(&t.app()), before);
}

#[test]
fn menu_delete_used_brush_requires_remove_strokes_or_cancel() {
    let mut t = Trial::new();
    let rect = rect(&t, 10.0);
    let three = select_three(&mut t, rect);
    let before = undo_count(&t.app());
    menu_run(&t, DELETE_BRUSH);
    assert_delete_dialog(&t, IN_USE);
    t.answer(false);
    assert_eq!(undo_count(&t.app()), before);
    assert_eq!(brush_of(&t.app(), rect).as_deref(), Some(three[0].as_str()));
    menu_run(&t, DELETE_BRUSH);
    t.answer(true);
    assert_eq!(undo_count(&t.app()), before + 1);
    assert_eq!(brush_of(&t.app(), rect), None);
    assert!(t.app().session.active().unwrap().doc.node(rect).unwrap().appearance.stroke().is_some());
    assert!(three.iter().all(|name| !names(&t.app()).contains(name)));
}

/// A brush dragged onto the left half of another brush of its type goes before it, as one undo
/// step; dropped on a brush of another type, it stays where it is.
#[test]
fn a_brush_dragged_onto_another_of_its_type_moves_before_it() {
    let mut t = Trial::new();
    let app = t.app();
    let st = app.session.active().unwrap();
    let order: Vec<String> = rows(st.uid, &document_brushes(st), &[]).into_iter().filter(|row| !row.is_group()).map(|row| row.label).collect();
    drop(app);
    assert_eq!(order[..4], ["3 pt. Round", "6 pt. Flat", "10 pt. Oval", "Dots"]);
    let undo = |t: &Trial| undo_count(&t.app());
    let steps = undo(&t);
    let calligraphic = |t: &Trial| -> Vec<String> { brushes(&t.app()).0.into_iter().filter(|b| b.1 == "calligraphic").map(|b| b.0).collect() };
    // "10 pt. Oval" onto the left half of "3 pt. Round".
    let (round, oval) = (t.target("3 pt. Round"), t.target("10 pt. Oval"));
    t.gesture(oval.center(), round.center() - vec2(round.rect.width() / 4.0, 0.0), PointerButton::Primary, Modifiers::NONE);
    assert_eq!(calligraphic(&t), ["10 pt. Oval", "3 pt. Round", "6 pt. Flat"]);
    assert_eq!(undo(&t), steps + 1);
    t.run("edit.undo", json!({}));
    assert_eq!(calligraphic(&t), ["3 pt. Round", "6 pt. Flat", "10 pt. Oval"]);
    // "3 pt. Round" onto "Dots", a scatter brush: nothing moves.
    let list = brushes(&t.app()).0;
    t.fixture.settle();
    let (round, dots) = (t.target("3 pt. Round"), t.target("Dots"));
    t.gesture(round.center(), dots.center(), PointerButton::Primary, Modifiers::NONE);
    assert_eq!(brushes(&t.app()).0, list);
    assert_eq!(undo(&t), steps);
}

/// Opaque pixels of brush definition `def`'s 80 × 40 pt preview with the resources of `doc`,
/// and those of them that are Pinstripes blue.
fn pixels(doc: &Document, def: &Value) -> (usize, usize) {
    let preview = preview_doc(doc, def, 80.0, 40.0).expect("a preview");
    let img = vectorcraft_render::Renderer::new().render_region(&preview, vectorcraft_geom::Rect::new(0.0, 0.0, 80.0, 40.0), 2.0, false);
    let painted: Vec<&[u8; 4]> = img.pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 200).collect();
    let blue = painted.iter().filter(|p| p[2] > 170 && p[0] < 90 && (100..160).contains(&p[1])).count();
    (painted.len(), blue)
}

/// A library brush's preview shows the pattern its art is filled with: it renders with the
/// library's resources, which a bare document doesn't have.
#[test]
fn library_brush_previews_show_the_patterns_their_art_uses() {
    let s = Session::new();
    let (_, lib) = brushlib::library(&s, "art-ribbons").unwrap();
    let def = serde_json::to_value(lib.brush("Striped Ribbon").unwrap()).unwrap();
    let (painted, blue) = pixels(&lib.doc, &def);
    assert!(painted > 0 && blue * 5 >= painted, "Pinstripes paints it: {painted} painted, {blue} blue");
    let (painted, blue) = pixels(&Document::new(10.0, 10.0), &def);
    assert_eq!(blue, 0, "without the pattern: {painted} painted");
    // A square tile still shows a stroke.
    let square = preview_doc(&lib.doc, &def, 40.0, 40.0).unwrap();
    let img = vectorcraft_render::Renderer::new().render_region(&square, vectorcraft_geom::Rect::new(0.0, 0.0, 40.0, 40.0), 2.0, false);
    assert!(img.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 200));
}

/// A 200 × 200 pt document's app.
fn square_document() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// A preview is kept by its brush and the patterns and symbols its art shows: adding other
/// brushes (and the patterns and symbols they bring) renders only the new one, every preview
/// renders as the panel first settles, and changing a pattern a brush shows renders that brush
/// again. Symbol thumbnails are keyed the same way.
#[test]
fn previews_render_again_only_when_what_they_show_changes() {
    let mut t = Trial::of(square_document());
    let n = brushes(&t.app()).0.len();
    assert_eq!(rendered(&mut t), n, "the panel's first frames render every preview");
    assert_eq!(rendered(&mut t), 0, "then they are kept");
    // Striped Ribbon brings its pattern, Sparkles its symbol: only they render.
    t.run("brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Striped Ribbon"}));
    assert_eq!(rendered(&mut t), 1);
    let symbol_key = |t: &Trial, name: &str| {
        let app = t.app();
        let st = app.session.active().unwrap();
        super::super::symbols::document_symbols(&st.doc, st.revision).key(name).unwrap_or(0)
    };
    let sparkle_before = symbol_key(&t, "Sparkle");
    t.run("brush.addFromLibrary", json!({"library": "scatter-shapes", "name": "Sparkles"}));
    assert_eq!(rendered(&mut t), 1);
    // A symbol added beside Sparkle leaves its thumbnail key alone.
    let sparkle = symbol_key(&t, "Sparkle");
    assert_ne!(sparkle, sparkle_before, "0 before Sparkle came in");
    t.run("symbol.addFromLibrary", json!({"library": "basic-shapes", "name": "Star"}));
    assert_eq!(symbol_key(&t, "Sparkle"), sparkle);
    assert_eq!(rendered(&mut t), 0);
    // A new Pinstripes tile: Striped Ribbon renders again, the others don't.
    t.app()
        .session
        .edit("Pattern", |d, _| {
            let p = d.patterns.iter_mut().find(|p| p.name == "Pinstripes").unwrap();
            let dot =
                Node::path(NodeId(0), vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(0.0, 0.0, 3.0, 3.0)), Appearance::default_art());
            p.art = vec![std::sync::Arc::new(dot)];
            Ok(())
        })
        .unwrap();
    assert_eq!(rendered(&mut t), 1);
}

fn keyed() -> usize {
    counters::KEYED.with(|k| k.replace(0))
}

/// The Brushes panel serialises and hashes the brushes of a document version once: frames
/// after the first, of it or of it again after another document was active, do neither; an
/// edit, an undo and a redo each work them out once again and render no preview again. Brush
/// Options works its preview's key out again only when its definition or the document changes.
#[test]
fn frames_after_the_first_serialise_and_hash_no_brush() {
    let app = square_document();
    let n = vectorcraft_brush::library(&app.session.active().unwrap().doc).len();
    let mut t = Trial::of(app);
    keyed();
    assert_eq!(rendered(&mut t), n);
    assert_eq!(keyed(), n, "the first frame works each brush out");
    for _ in 0..3 {
        assert_eq!(rendered(&mut t), 0);
    }
    assert_eq!(keyed(), 0, "later frames serialise and hash none");
    // Another document, then this one again.
    t.run("file.new", json!({"width": 100, "height": 100}));
    t.fixture.frame(Vec::new());
    keyed();
    t.run("document.activate", json!({"index": 0}));
    assert_eq!(rendered(&mut t), 0);
    assert_eq!(keyed(), 0, "the document's brushes are still known");
    // An edit to something else: the brushes are worked out again, no preview renders.
    t.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
    assert_eq!(rendered(&mut t), 0);
    assert_eq!(keyed(), n);
    for cmd in ["edit.undo", "edit.redo"] {
        t.run(cmd, json!({}));
        assert_eq!(rendered(&mut t), 0, "{cmd}");
        t.fixture.frame(Vec::new());
        assert!(keyed() <= n, "{cmd}: once at most");
    }
    // Brush Options' preview of a definition, painted from the document it shows.
    let (_, lib) = brushlib::library(&t.app().session, "art-ribbons").unwrap();
    let def = serde_json::to_value(lib.brush("Striped Ribbon").unwrap()).unwrap();
    let painted: Rc<RefCell<(Option<Arc<Document>>, Value)>> = Rc::new(RefCell::new((None, def)));
    let shown = painted.clone();
    let mut options = Fixture::new(move |ui| {
        let held = shown.borrow();
        let paint = |cx: &ContentPainterCx, rect: Rect| paint_definition(cx, rect, held.0.as_deref(), &held.1);
        let _ = Picture::painter("Stroke preview", ThumbAspect::new(200, 40), paint).show(ui);
    });
    let document = |t: &Trial| t.app().session.active().map(|st| st.doc.clone());
    painted.borrow_mut().0 = document(&t);
    options.frame(Vec::new());
    assert_eq!(keyed(), 1);
    options.idle(3);
    assert_eq!(keyed(), 0, "the same definition in the same document");
    painted.borrow_mut().1["width"] = json!(150.0);
    options.idle(2);
    assert_eq!(keyed(), 1, "a field changed");
    t.run("shape.rectangle", json!({"x": 40, "y": 10, "width": 20, "height": 20}));
    painted.borrow_mut().0 = document(&t);
    options.idle(2);
    assert_eq!(keyed(), 1, "the document changed");
}

/// The Symbols panel of the trial's app in a fixture of its own.
fn symbols_panel(app: &Shared) -> Fixture {
    let shown = app.clone();
    let mut fixture = Fixture::new(move |ui| super::super::symbols::show(&mut shown.borrow_mut(), ui));
    fixture.room(vec2(236.0, 600.0));
    fixture
}

/// Symbol thumbnails rendered while the Symbols panel settles.
fn symbols_rendered(symbols: &mut Fixture) -> usize {
    use super::super::symbols::counters::RENDERED;
    RENDERED.with(|c| c.set(0));
    symbols.settle();
    RENDERED.with(std::cell::Cell::get)
}

/// A 4 × 4 PNG of colour `rgb`.
fn png(rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    let mut out = vec![];
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| out.extend_from_slice(&__b))
    }
    .unwrap();
    out
}

/// Brush previews and symbol thumbnails render again when a swatch, an image, or a pattern's
/// tile type, overlap or spacing their art uses changes (only those that use it), and not when
/// one they don't use does.
#[test]
fn previews_follow_the_swatches_images_and_pattern_options_their_art_uses() {
    use vectorcraft_doc::pattern::{Overlap, TileType};
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind, Symbol};
    let mut app = square_document();
    // Striped Ribbon brings Pinstripes.
    app.run("brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Striped Ribbon"})).unwrap();
    let (_, lib) = brushlib::library(&app.session, "art-ribbons").unwrap();
    let wave = lib.brush("Wave").unwrap().clone();
    let striped = lib.brush("Striped Ribbon").unwrap().art()[0].clone();
    let ink = Paint::Solid { color: Color::BLACK, swatch: Some("Ink".into()), tint: 1.0 };
    let inked = Node::path(
        NodeId(0),
        vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 20.0, 10.0)),
        Appearance::basic(ink, Paint::None, 0.0),
    );
    let im = ImageObject {
        key: "pic".into(),
        width: 4,
        height: 4,
        xf: vectorcraft_geom::Affine::scale(5.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let pictured = Node::new(NodeId(0), NodeKind::Image(Box::new(im)));
    // Filled with the gradient swatch Fade.
    let fade = |c: Color| {
        use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
        GradientPaint::new(Gradient::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, c)]))
    };
    let mut faded_paint = fade(Color::BLACK);
    faded_paint.swatch = Some("Fade".into());
    let faded = Node::path(
        NodeId(0),
        vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 20.0, 10.0)),
        Appearance::basic(Paint::Gradient(Box::new(faded_paint)), Paint::None, 0.0),
    );
    app.session
        .edit("Resources", |d, _| {
            for name in ["Ink", "Other"] {
                d.swatches.push(vectorcraft_color::Swatch { name: name.into(), paint: Paint::solid(Color::BLACK), global: true, spot: false });
            }
            let paint = Paint::Gradient(Box::new(fade(Color::BLACK)));
            d.swatches.push(vectorcraft_color::Swatch { name: "Fade".into(), paint, global: false, spot: false });
            d.images.insert("pic".into(), ImageBlob::png(png([255, 0, 0])));
            let mut brushes = vectorcraft_brush::library(d);
            for (name, art) in [("Inked", &inked), ("Pictured", &pictured), ("Faded", &faded)] {
                let mut b = wave.clone();
                b.name = name.into();
                if let vectorcraft_brush::BrushKind::Art(a) = &mut b.kind {
                    a.art = art.clone();
                }
                brushes.push(b);
            }
            vectorcraft_brush::store(d, &brushes);
            for (name, art) in [("Ink Symbol", &inked), ("Picture Symbol", &pictured), ("Stripes Symbol", &striped), ("Fade Symbol", &faded)] {
                d.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art.clone()) });
            }
            Ok(())
        })
        .unwrap();
    let n = brushes(&app).0.len();
    let mut t = Trial::of(app);
    let mut symbols = symbols_panel(&t.app);
    assert_eq!((rendered(&mut t), symbols_rendered(&mut symbols)), (n, 4));
    assert_eq!((rendered(&mut t), symbols_rendered(&mut symbols)), (0, 0));
    let mut change = |what: &str, f: &dyn Fn(&mut Document)| {
        t.app()
            .session
            .edit("Change", |d, _| {
                f(d);
                Ok(())
            })
            .unwrap();
        (rendered(&mut t), symbols_rendered(&mut symbols), what.to_string())
    };
    let swatch =
        |name: &'static str| move |d: &mut Document| d.swatches.iter_mut().find(|w| w.name == name).unwrap().paint = Paint::solid(Color::WHITE);
    fn pinstripes(d: &mut Document) -> &mut vectorcraft_doc::pattern::PatternDef {
        d.patterns.iter_mut().find(|p| p.name == "Pinstripes").unwrap()
    }
    type Case<'a> = (&'a str, (usize, usize), &'a dyn Fn(&mut Document));
    let cases: [Case; 8] = [
        ("Ink", (1, 1), &swatch("Ink")),
        ("Other", (0, 0), &swatch("Other")),
        ("Fade", (1, 1), &|d| d.swatches.iter_mut().find(|w| w.name == "Fade").unwrap().paint = Paint::Gradient(Box::new(fade(Color::WHITE)))),
        ("pic", (1, 1), &|d| {
            d.images.insert("pic".into(), ImageBlob::png(png([0, 0, 255])));
        }),
        ("another image", (0, 0), &|d| {
            d.images.insert("pic2".into(), ImageBlob::png(png([0, 255, 0])));
        }),
        ("tile type", (1, 1), &|d| pinstripes(d).tile_type = TileType::BrickByRow { offset: 0.5 }),
        ("overlap", (1, 1), &|d| pinstripes(d).overlap = Overlap { right_in_front: true, bottom_in_front: true }),
        ("spacing", (1, 1), &|d| pinstripes(d).h_spacing = 4.0),
    ];
    for (what, (brushes, symbols), f) in cases {
        assert_eq!(change(what, f), (brushes, symbols, what.to_string()));
    }
}

/// Of two brushes with one name, the row of that name previews and drags the first, the one
/// `brush.get` gives and a stroke that names it draws with. Reading the library numbers the second
/// ("Twin 2"), so it gets a row of its own.
#[test]
fn rows_of_two_brushes_with_one_name_show_the_first() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut twins: Vec<Brush> = Vec::new();
    app.session
        .edit("Brushes", |d, _| {
            let mut lib = vectorcraft_brush::library(d);
            let kinds: Vec<_> = ["Charcoal", "Dots"].iter().map(|n| lib.iter().find(|b| b.name == *n).unwrap().kind.clone()).collect();
            twins = kinds.into_iter().map(|kind| Brush { name: "Twin".into(), kind }).collect();
            lib.extend(twins.iter().cloned());
            vectorcraft_brush::store(d, &lib);
            Ok(())
        })
        .unwrap();
    let rows_named: Vec<(String, String)> = brushes(&app).0.into_iter().filter(|b| b.0.starts_with("Twin")).map(|b| (b.0, b.1)).collect();
    assert_eq!(rows_named, [("Twin".to_string(), "art".to_string()), ("Twin 2".to_string(), "scatter".to_string())], "the panel lists both");
    let got = app.run("brush.get", json!({"name": "Twin"})).unwrap();
    let memo = document_brushes(app.session.active().unwrap());
    let shown = by_name(&memo)["Twin"];
    assert_eq!(shown.ty, "art");
    assert_eq!(shown.def, got, "the row shows and drags what brush.get gives");
    // Rows of one name that reach the panel unnumbered still show the first, in one row.
    let st = app.session.active().unwrap();
    let raw: Vec<Shown> = twins.iter().map(|b| Shown::of(&st.doc, b)).collect();
    let shown = by_name(&raw)["Twin"];
    assert_eq!(shown.ty, "art");
    assert_eq!(shown.def, got, "the first of two rows of one name is the one shown");
    let keys: Vec<BrushKey> = rows(st.uid, &raw, &[]).into_iter().filter(|row| !row.is_group()).map(|row| row.key).collect();
    assert_eq!(keys, [BrushKey::brush(st.uid, "Twin")], "one row, under the first one's type");
}

/// A brush's menu acts on it alone, or on the multi-selection when the brush is one of several
/// selected. A secondary click on a brush not selected selects it and makes it current without
/// applying it to the selected art; Brush Options… opens its options; Delete Brush is off for the
/// last brush.
#[test]
fn a_brushs_menu_acts_on_it_or_on_the_selection_it_is_in() {
    let mut t = Trial::new();
    rect(&t, 10.0);
    t.run("brush.apply", json!({"name": "Tapered Stroke"}));
    t.fixture.settle();
    assert_ne!(brushes(&t.app()).1.as_deref(), Some("Charcoal"));
    let rows_of = |t: &Trial, labels: &[&str]| -> Vec<bool> { labels.iter().map(|label| t.enabled(&t.menu_row(label))).collect() };

    // One brush: its three rows, on.
    t.right_click("Charcoal");
    assert_eq!(brushes(&t.app()).1.as_deref(), Some("Charcoal"));
    assert_eq!(selected_brush(&t.app()).as_deref(), Some("Tapered Stroke"), "a secondary click applies nothing");
    assert_eq!(t.shown_selected(), ["Charcoal"]);
    assert_eq!(rows_of(&t, &["Brush Options…", "Duplicate Brush", "Delete Brush"]), [true, true, true]);
    let duplicate = t.menu_row("Duplicate Brush");
    t.fixture.click(&duplicate);
    assert!(names(&t.app()).contains(&"Charcoal copy".to_string()));
    t.right_click("Charcoal");
    let options = t.menu_row("Brush Options…");
    t.fixture.click(&options);
    assert_eq!(t.dialog_key().as_deref(), Some(brush_options::KIND));
    assert_eq!(t.dialog().unwrap().1["name"], json!("Charcoal"));
    t.answer(false);

    // One of several selected: the plural rows, on, acting on all of them.
    let selection = Selection::new(vec!["Charcoal".to_string(), "Tapered Stroke".to_string()], None, None);
    store_selection(&mut t.app(), selection);
    t.fixture.settle();
    t.right_click("Tapered Stroke");
    assert_eq!(t.shown_selected(), ["Charcoal", "Tapered Stroke"], "a secondary click keeps the selection it lands in");
    assert_eq!(rows_of(&t, &["Duplicate Brushes", "Delete Brushes"]), [true, true]);
    let duplicate = t.menu_row("Duplicate Brushes");
    t.fixture.click(&duplicate);
    let all = names(&t.app());
    assert!(all.contains(&"Charcoal copy 2".to_string()) && all.contains(&"Tapered Stroke copy".to_string()), "{all:?}");

    // The last brush cannot be deleted.
    let others: Vec<String> = names(&t.app()).into_iter().filter(|n| n != "Charcoal").collect();
    t.run("brush.delete", json!({ "names": others }));
    t.fixture.settle();
    t.right_click("Charcoal");
    assert_eq!(rows_of(&t, &["Delete Brush"]), [false]);
}
