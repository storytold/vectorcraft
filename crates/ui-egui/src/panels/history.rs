//! History panel: the document's states; click one to go back (or forward) to it.

use std::sync::Arc;

use craft::behavior::{IconId, Selection, SelectionChange, keyed};
use craft::keymap::CommandRef;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, ScrollAnchor, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing};
use craft::ui::display::Divider;
use craft::ui::layout_components::Stack;
use craft::ui::list::{List, ListRow, RowInk};
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::patterns::panels::{HistoryKey, HistoryRow};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::sections::{FooterAction, FooterBar};
use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// Undo (negative) / redo (positive) steps to reach row `target` when `current` is the current
/// row (row 0 = the document as opened).
pub fn steps_to(current: usize, target: usize) -> i64 {
    target as i64 - current as i64
}

/// A step's name in `lang`: its plain row, else the row of the same name with an ellipsis (a menu
/// item's, such as an effect's) without the ellipsis; `None` when the catalog has neither.
pub fn step_name(lang: crate::i18n::Lang, label: &str) -> Option<String> {
    use crate::i18n::{has, tr};
    if has(lang, label) {
        return Some(tr(lang, label).to_string());
    }
    let dotted = format!("{label}…");
    has(lang, &dotted).then(|| tr(lang, &dotted).trim_end_matches('…').to_string())
}

/// A step's name in `lang`. A name made from a template (`Apply {name}`,
/// [`vectorcraft_engine::HistoryEntry::named`]) takes the template's plain row with the name in its
/// placeholder. A name VectorCraft gives is shown in `lang`, and any other name is shown as the
/// document holds it. It returns `None` when the catalog has no row for the template. Any other step
/// takes [`step_name`] of its label.
pub fn entry_name(lang: crate::i18n::Lang, h: &vectorcraft_engine::HistoryEntry) -> Option<String> {
    use crate::i18n::{fmt, has, label_or_name, tr};
    match h.named() {
        Some(n) => has(lang, n.template).then(|| fmt(tr(lang, n.template), &[("name", label_or_name(lang, &n.name, n.built_in))])),
        None => step_name(lang, &h.label),
    }
}

/// A step's name as the panel lists it: in the UI language, else as recorded. Edit › Undo and Redo
/// name the step the same way.
pub(crate) fn shown(h: &vectorcraft_engine::HistoryEntry) -> String {
    entry_name(crate::i18n::current(), h).unwrap_or_else(|| h.label.clone())
}

/// The names of the rows, top to bottom: the document as opened, each step to undo, then each step
/// to redo with the next one first. Empty with no document.
fn names(app: &VectorcraftApp) -> Vec<String> {
    let Some(st) = app.session.active() else { return Vec::new() };
    let mut names = vec![tl!("Open").to_string()];
    names.extend(st.history.undo.iter().map(shown));
    names.extend(st.history.redo.iter().rev().map(shown));
    names
}

/// The rows of the list: row `i` is state `i`, and the rows after the last undo step are redo states.
fn rows(app: &VectorcraftApp) -> Vec<HistoryRow> {
    let undone = app.session.active().map_or(0, |st| st.history.undo.len());
    names(app)
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            // PLAN: assumed `craft::ui::primitives::IconId`; the app has no direct craft-behavior dependency.
            let icon = craft::behavior::IconId::new(if i == 0 { "new_file" } else { "history" });
            HistoryRow::state(i as u64, name).icon(icon).redo(i > undone)
        })
        .collect()
}

#[derive(Clone)]
struct ContextState {
    document: u64,
    snapshot: Arc<vectorcraft_doc::Document>,
}

/// The shared collection supplies row focus, keyboard context requests, scrolling and menu
/// dismissal. The app retains the chosen document snapshot and dispatches its history commands.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let state_id = ui.scope_id().with("history-context-state");
    let document = app.session.active().map(|st| st.uid);
    let mut context = ui.ctx().data(|d| d.get_temp::<ContextState>(state_id));
    if context.as_ref().is_some_and(|state| document != Some(state.document)) {
        craft::ui::runtime::overlay::close_popups(ui.ctx());
        context = None;
    }
    let Some(st) = app.session.active() else {
        ui.ctx().data_mut(|d| d.remove::<ContextState>(state_id));
        super::empty_state(ui, "history", tl!("No document"), tl!("Open a document to see its history."));
        return;
    };
    let document = st.uid;
    let current = st.history.undo.len();
    let rows = rows(app);
    let mut selected = Selection::single(HistoryKey::State(current as u64));
    let mut new_document = false;
    let mut target = None;
    let mut action = None;
    Stack::new()
        .fill()
        .footer(|ui, _| {
            let out = FooterBar::actions([FooterAction::new(IconId::new("new_file"), tl!("Create New Document from Current State"))]).show_footer(ui);
            if !ui.is_sizing_pass() {
                new_document |= out.picked.is_some();
            }
        })
        .show(ui, |ui, _| {
            let list = List::<HistoryKey>::new(tl!("History"))
                .id(ui.scope_id().with(("history-rows", document)))
                .select(SelectMode::Single)
                .arrows_select(false)
                .selection(Bind::bound(&mut selected))
                .row_menu()
                .sizing(Sizing::Fill)
                .scroll_anchor(ScrollAnchor::End);
            let out = list.show_items(ui, &keyed(&rows, |row: &HistoryRow| row.key), &mut |_, icx| {
                let row = &rows[icx.at];
                let mut item = ListRow::new(row.label.clone()).cells(icx.cells()).state(icx.row.clone());
                if let Some(icon) = row.icon.clone() {
                    item = item.icon(icon.into());
                }
                if row.redo {
                    item = item.ink(RowInk::Dimmed);
                }
                item
            });
            let requested = out.events.iter().find_map(|event| match event {
                CollectionEvent::ContextMenu(request) => Some(request.key),
                _ => None,
            });
            if let Some(HistoryKey::State(row)) = requested {
                context = app
                    .session
                    .active()
                    .and_then(|st| st.history.state(&st.doc, row as usize))
                    .cloned()
                    .map(|snapshot| ContextState { document, snapshot });
            } else {
                for event in &out.events {
                    if let CollectionEvent::Activate(HistoryKey::State(row)) = event {
                        target = app.session.active().and_then(|st| st.history.state(&st.doc, *row as usize)).cloned();
                    }
                    if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                        && let Some(HistoryKey::State(row)) = keys.last()
                        && *row as usize != current
                    {
                        target = app.session.active().and_then(|st| st.history.state(&st.doc, *row as usize)).cloned();
                    }
                }
            }
            if let Some(state) = context.as_ref() {
                let snapshot = state.snapshot.clone();
                let can_step = app.session.active().is_some_and(|st| !Arc::ptr_eq(&snapshot, &st.doc));
                let shown = Menu::context(tl!("History"), &out.response).show(ui, |ui, _| {
                    if MenuItem::new(tl!("New Document from State")).enabled(crate::menus::enabled(app, "edit.newDocumentFromState")).show(ui).ran {
                        action = Some(("edit.newDocumentFromState", snapshot.clone()));
                    }
                    if MenuItem::new(tl!("Step to This State")).enabled(can_step).show(ui).ran {
                        target = Some(snapshot.clone());
                    }
                    Divider::horizontal().show(ui);
                    if MenuItem::new(tl!("Clear History")).enabled(crate::menus::enabled(app, "edit.clearHistory")).show(ui).ran {
                        action = Some(("edit.clearHistory", snapshot.clone()));
                    }
                });
                if shown.output.closed.is_some() || action.is_some() || target.is_some() {
                    context = None;
                }
            }
        });
    ui.ctx().data_mut(|d| {
        if let Some(state) = context {
            d.insert_temp(state_id, state);
        } else {
            d.remove::<ContextState>(state_id);
        }
    });
    if let Some((command, snapshot)) = action {
        if command == "edit.newDocumentFromState" {
            if let Some(row) = resolve_state(app, snapshot, false) {
                crate::menus::invoke(app, command, json!({ "index": row }));
            }
        } else {
            crate::menus::invoke(app, command, json!({}));
        }
    }
    if let Some(target) = target {
        step_to_state(app, target);
    }
    if new_document {
        crate::menus::invoke(app, "edit.newDocumentFromState", json!({}));
    }
}

/// Keep the clicked snapshot while pending edits settle: they can add or trim history rows.
fn resolve_state(app: &mut VectorcraftApp, target: Arc<vectorcraft_doc::Document>, keep_typing: bool) -> Option<usize> {
    let requests = app.session.finish_tool_edit();
    let failed = requests.is_err();
    crate::canvas::apply_requests(app, requests);
    if failed {
        return None;
    }
    if keep_typing && let Err(e) = app.keep_typing() {
        app.status(e);
        return None;
    }
    let st = app.session.active()?;
    let current = st.history.undo.len();
    let row = if Arc::ptr_eq(&st.doc, &target) {
        Some(current)
    } else {
        st.history
            .undo
            .iter()
            .position(|h| Arc::ptr_eq(&h.doc, &target))
            .or_else(|| st.history.redo.iter().rev().position(|h| Arc::ptr_eq(&h.doc, &target)).map(|i| current + i + 1))
    };
    if row.is_none() {
        app.status(tl!("That history state is no longer available."));
    }
    row
}

fn step_to_state(app: &mut VectorcraftApp, target: Arc<vectorcraft_doc::Document>) {
    let Some(row) = resolve_state(app, target, true) else { return };
    let Some(st) = app.session.active() else { return };
    let steps = steps_to(st.history.undo.len(), row);
    let (cmd, n) = if steps < 0 { ("edit.undo", -steps) } else { ("edit.redo", steps) };
    for _ in 0..n {
        if let Err(e) = app.run(cmd, json!({})) {
            app.status(e);
            break;
        }
    }
}

/// Set History Limit…: Preferences at the page with History States.
fn limit_row() -> CommandRef {
    CommandRef::with_params("edit.preferences", json!({ "category": "Performance" }))
}

/// The panel menu's rows.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    vec![
        PanelRow::Command(CommandRef::new("ui.history.stepBackward")),
        PanelRow::Command(CommandRef::new("ui.history.stepForward")),
        PanelRow::Separator,
        PanelRow::Command(CommandRef::new("edit.clearHistory")),
        PanelRow::Command(limit_row()),
        PanelRow::Command(CommandRef::new("ui.history.options")),
    ]
}

/// The titles and enabled states of [`menu_rows`].
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let state = |title: &'static str, enabled: bool| CommandState::new().title(title).enabled(enabled);
    states.set(CommandRef::new("ui.history.stepBackward"), state(tl!("Step Backward"), crate::menus::enabled(app, "edit.undo")));
    states.set(CommandRef::new("ui.history.stepForward"), state(tl!("Step Forward"), crate::menus::enabled(app, "edit.redo")));
    states.set(CommandRef::new("edit.clearHistory"), state(tl!("Clear History"), crate::menus::enabled(app, "edit.clearHistory")));
    states.set(limit_row(), state(tl!("Set History Limit…"), true));
    states.set(CommandRef::new("ui.history.options"), state(tl!("History Options…"), false));
}

/// Runs the panel menu's own rows; `None` for an id that is not one of them.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    match id {
        "ui.history.stepBackward" => Some(app.run("edit.undo", json!({}))),
        "ui.history.stepForward" => Some(app.run("edit.redo", json!({}))),
        "ui.history.options" => Some(Err("History Options… is not available".into())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::LangExt;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;

    /// A step named after a style lists the template's translation with the style's name in it as
    /// the document holds it, also when the name looks like a template, like a step with a row of
    /// its own (`Apply Brush`) or like another template's name; the Normal style's name is
    /// translated as the Character Styles panel shows it. Undone steps keep their names.
    #[test]
    fn steps_named_after_a_style_list_the_template_around_the_name() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let mut run = |id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
        run("file.new", json!({"width": 200, "height": 150}));
        let rect = run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].clone();
        run("graphicStyle.new", json!({"name": "S", "id": rect}));
        run("graphicStyle.redefine", json!({"name": "S", "id": rect}));
        let text = run("text.create", json!({"x": 10, "y": 100, "text": "Hello"}))["id"].clone();
        for name in ["Body", "{name}", "Apply", "Brush", "Graphic Style “X”"] {
            run("charStyle.new", json!({"name": name}));
        }
        for name in ["Body", "{name}", "Apply", "Brush", "[Normal Character Style]"] {
            run("charStyle.apply", json!({"name": name, "id": text}));
        }
        run("charStyle.setAttrs", json!({"name": "Graphic Style “X”", "attrs": {"size": 30.0}}));
        let ja = crate::i18n::Lang::from_code("ja").expect("ja registered");
        crate::i18n::set_current(ja);
        let rows = names(&app);
        let step = |template: &str, name: &str| crate::i18n::fmt(crate::i18n::tr(ja, template), &[("name", name)]);
        let apply = |name: &str| step("Apply {name}", name);
        let want = [
            step("Redefine Graphic Style “{name}”", "S"),
            apply("Body"),
            apply("{name}"),
            apply("Apply"),
            apply("Brush"),
            apply(crate::i18n::tr(ja, "[Normal Character Style]")),
            step("Redefine {name}", "Graphic Style “X”"),
        ];
        assert!(rows.contains(&want[0]), "{rows:#?}");
        assert_eq!(rows[rows.len() - 6..], want[1..], "{rows:#?}");
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(names(&app).last(), Some(&want[6]), "an undone step");
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert_eq!(names(&app)[rows.len() - 6..rows.len() - 4], ["Apply Body", "Apply {name}"]);
    }

    /// A step named after an effect whose catalog row has only the menu label with its ellipsis
    /// takes that row's translation without the ellipsis; a name with no row has none.
    #[test]
    fn effect_steps_read_the_menu_row() {
        let ja = crate::i18n::Lang::from_code("ja").expect("ja registered");
        for effect in ["Revolve", "Feather"] {
            let menu_row =
                crate::i18n::has(ja, &format!("{effect}…")).then(|| crate::i18n::tr(ja, &format!("{effect}…")).trim_end_matches('…').to_string());
            let plain_row = crate::i18n::has(ja, effect).then(|| crate::i18n::tr(ja, effect).to_string());
            assert_eq!(step_name(ja, effect), plain_row.or(menu_row), "{effect}");
        }
        assert_eq!(step_name(ja, "No Such Step"), None);
    }

    fn app_with_two_rectangles() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        for x in [10, 60] {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
        }
        app
    }

    /// Returns the number of steps to undo and to redo.
    fn depth(app: &VectorcraftApp) -> (usize, usize) {
        let h = &app.session.active().unwrap().history;
        (h.undo.len(), h.redo.len())
    }

    /// Whether the menu has Step Backward, Step Forward and Clear History enabled.
    fn enabled(app: &VectorcraftApp) -> (bool, bool, bool) {
        let mut states = CommandStates::default();
        menu_states(app, &mut states);
        let on = |id: &str| states.get(&CommandRef::new(id)).is_some_and(CommandState::is_enabled);
        (on("ui.history.stepBackward"), on("ui.history.stepForward"), on("edit.clearHistory"))
    }

    /// The menu's step rows are enabled exactly when there is a step to undo or redo and move one
    /// state; Clear History empties both stacks and leaves the document as it is; a run of a
    /// disabled row changes nothing.
    #[test]
    fn the_history_rows_step_one_state_and_clear_without_touching_the_document() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert_eq!(enabled(&app), (false, false, false), "no document");
        assert!(run(&mut app, "ui.history.stepBackward", &json!({})).is_some_and(|r| r.is_err()), "no document");
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        assert_eq!(enabled(&app), (false, false, false), "a new document");
        let revision = app.session.active().unwrap().revision;
        let _ = run(&mut app, "ui.history.stepBackward", &json!({}));
        let _ = run(&mut app, "ui.history.stepForward", &json!({}));
        assert_eq!((app.session.active().unwrap().revision, depth(&app)), (revision, (0, 0)), "disabled rows change nothing");
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        assert_eq!(enabled(&app), (true, false, true), "one step to undo");
        run(&mut app, "ui.history.stepBackward", &json!({})).unwrap().unwrap();
        assert_eq!((depth(&app), enabled(&app)), ((0, 1), (false, true, true)), "one step to redo");
        run(&mut app, "ui.history.stepForward", &json!({})).unwrap().unwrap();
        assert_eq!(depth(&app), (1, 0));
        let mut app = app_with_two_rectangles();
        run(&mut app, "ui.history.stepBackward", &json!({})).unwrap().unwrap();
        assert_eq!(depth(&app), (1, 1));
        let doc = app.session.active().unwrap().doc.clone();
        crate::menus::invoke(&mut app, "edit.clearHistory", json!({}));
        assert_eq!((depth(&app), enabled(&app)), ((0, 0), (false, false, false)));
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc), "the document stays as it is");
        assert!(run(&mut app, "ui.history.unknown", &json!({})).is_none());
    }

    /// The bottom of the panel's New Document from State button makes a new document, the active
    /// one, from the state a click on a row moved to. The source document stays open at that state.
    #[test]
    fn the_new_document_button_makes_a_document_from_the_selected_state() {
        let mut app = app_with_two_rectangles();
        app.session.execute("edit.undo", &json!({})).unwrap();
        let source = app.session.active().unwrap().doc.clone();
        let mut panel = Panel::new(app, show);
        panel.click("Create New Document from Current State");
        let app = panel.app();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.session.active_index(), Some(1), "the new document is the active one");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layers[0].children().unwrap().len(), 1, "the art of the selected state");
        assert!(st.is_dirty(), "the new document is unsaved");
        assert!(std::sync::Arc::ptr_eq(&app.session.documents()[0].doc, &source), "the source stays at that state");
    }

    fn history_harness(app: VectorcraftApp) -> Panel {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let mut panel = Panel::new(app, show);
        panel.room(egui::vec2(260.0, 400.0));
        panel.frame();
        panel
    }

    fn row_menu_harness(app: VectorcraftApp, row: usize) -> Panel {
        let mut panel = history_harness(app);
        panel.secondary_click_nth(Role::LIST_ITEM, row);
        panel
    }

    fn focus_previous_state(panel: &mut Panel) {
        for _ in 0..4 {
            panel.key(egui::Key::Tab, egui::Modifiers::NONE);
            if panel.focused_nth(Role::LIST_ITEM, 2) {
                break;
            }
        }
        assert!(panel.focused_nth(Role::LIST_ITEM, 2));
        panel.key(egui::Key::ArrowUp, egui::Modifiers::NONE);
        assert!(panel.focused_nth(Role::LIST_ITEM, 1));
        assert_eq!(depth(&panel.app()), (2, 0), "moving focus must not step history");
    }

    #[test]
    fn keyboard_activation_steps_to_the_focused_state() {
        let mut h = history_harness(app_with_two_rectangles());
        focus_previous_state(&mut h);
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        h.frame();
        h.frame();
        assert_eq!(depth(&h.app()), (1, 1));
        assert_eq!(h.app().session.active().unwrap().doc.layers[0].children().unwrap().len(), 1);
    }
    #[test]
    fn switching_documents_dismisses_the_row_menu_without_clearing_either_history() {
        let mut app = app_with_two_rectangles();
        app.session.execute("edit.undo", &json!({})).unwrap();
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        for x in [10, 60] {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
        }
        app.session.execute("edit.undo", &json!({})).unwrap();
        app.session.execute("document.activate", &json!({"index": 0})).unwrap();
        let mut h = row_menu_harness(app, 1);
        assert!(h.shows("Clear History"));
        h.app().session.execute("document.activate", &json!({"index": 1})).unwrap();
        h.frame();
        h.frame();
        assert!(!h.shows("Clear History"), "the other document must not inherit the popup");
        h.app().session.execute("document.activate", &json!({"index": 0})).unwrap();
        h.frame();
        h.frame();
        assert!(!h.shows("Clear History"), "switching back must not revive the popup");
        for st in h.app().session.documents() {
            assert_eq!((st.history.undo.len(), st.history.redo.len()), (1, 1));
        }
    }

    #[test]
    fn keyboard_opens_dismisses_and_activates_the_focused_rows_menu() {
        let app = app_with_two_rectangles();
        let source = app.session.active().unwrap().doc.clone();
        let wanted = app.session.active().unwrap().history.undo[1].doc.clone();
        let mut h = history_harness(app);
        focus_previous_state(&mut h);
        h.key(egui::Key::F10, egui::Modifiers::SHIFT);
        h.frame();
        h.frame();
        assert!(h.focused("New Document from State"));
        assert_eq!(depth(&h.app()), (2, 0), "opening does not step");
        h.key(egui::Key::Escape, egui::Modifiers::NONE);
        h.frame();
        h.frame();
        assert!(!h.shows("New Document from State"));
        assert!(h.focused_nth(Role::LIST_ITEM, 1));
        h.key(egui::Key::F10, egui::Modifiers::SHIFT);
        h.frame();
        h.frame();
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        h.frame();
        h.frame();
        assert_eq!(h.app().session.documents().len(), 2);
        assert_eq!(h.app().session.active().unwrap().doc.layers, wanted.layers);
        assert!(Arc::ptr_eq(&h.app().session.documents()[0].doc, &source));
        assert_eq!(h.app().session.documents()[0].history.undo.len(), 2);
    }

    #[test]
    fn right_clicking_an_earlier_state_opens_its_menu_without_stepping() {
        let h = row_menu_harness(app_with_two_rectangles(), 1);
        assert!(h.shows("New Document from State"));
        assert!(h.shows("Step to This State"));
        assert!(h.shows("Clear History"));
        assert_eq!(depth(&h.app()), (2, 0));
    }

    #[test]
    fn the_row_menu_makes_a_new_document_from_that_snapshot() {
        let mut app = app_with_two_rectangles();
        app.session.execute("edit.undo", &json!({})).unwrap();
        let snapshot = app.session.active().unwrap().doc.clone();
        app.session.execute("edit.redo", &json!({})).unwrap();
        let source = app.session.active().unwrap().doc.clone();
        let mut h = row_menu_harness(app, 1);
        h.click_menu_item("New Document from State");
        h.frame();
        h.frame();
        let app = h.app();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.session.active_index(), Some(1));
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layers, snapshot.layers, "the historic artwork");
        assert!(st.is_dirty());
        assert_eq!(depth(&app), (0, 0));
        assert!(std::sync::Arc::ptr_eq(&app.session.documents()[0].doc, &source));
        assert_eq!(app.session.documents()[0].history.undo.len(), 2);
    }

    #[test]
    fn the_row_menu_steps_backward_and_forward_to_the_target() {
        for (current, target) in [(2, 0), (0, 2), (1, 1)] {
            let mut app = app_with_two_rectangles();
            for _ in current..2 {
                app.session.execute("edit.undo", &json!({})).unwrap();
            }
            let mut h = row_menu_harness(app, target);
            assert_eq!(h.menu_item_enabled("Step to This State"), current != target);
            h.click_menu_item("Step to This State");
            h.frame();
            h.frame();
            assert_eq!(depth(&h.app()), (target, 2 - target));
            assert_eq!(h.app().session.active().unwrap().doc.layers[0].children().unwrap().len(), target);
        }
    }

    #[test]
    fn the_row_menu_clears_undo_and_redo_without_changing_art() {
        let mut app = app_with_two_rectangles();
        app.session.execute("edit.undo", &json!({})).unwrap();
        let source = app.session.active().unwrap().doc.clone();
        let mut h = row_menu_harness(app, 0);
        h.click_menu_item("Clear History");
        h.frame();
        h.frame();
        assert_eq!(depth(&h.app()), (0, 0));
        assert!(std::sync::Arc::ptr_eq(&h.app().session.active().unwrap().doc, &source));
        let mut empty = app_with_two_rectangles();
        empty.session.execute("edit.clearHistory", &json!({})).unwrap();
        let h = row_menu_harness(empty, 0);
        assert!(!h.menu_item_enabled("Clear History"));
    }

    #[test]
    fn stepping_settles_typing_and_follows_the_snapshot_when_rows_shift() {
        for (limit, row) in [(10, 0), (2, 1)] {
            let mut app = app_with_two_rectangles();
            app.session.active_mut().unwrap().history.limit = limit;
            let st = app.session.active().unwrap();
            let wanted = st.history.state(&st.doc, row).unwrap().clone();
            app.session.begin_interaction("Typing").unwrap();
            app.session.preview("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
            let mut h = row_menu_harness(app, row);
            h.click_menu_item("Step to This State");
            h.frame();
            h.frame();
            let app = h.app();
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.layers, wanted.layers, "limit {limit}, row {row}");
            assert!(st.interaction.is_none());
            assert_eq!(st.history.undo.len(), 0);
        }
    }

    #[test]
    fn copying_a_state_resolves_pending_tool_edits_without_copying_a_neighbor() {
        use vectorcraft_tools::{PointerEvent, PointerKind};

        for (row, finishes_while_open) in [(0, false), (1, false), (0, true), (1, true)] {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
            // A Shape Builder merge over this many segments computes on another thread.
            for k in 0..40 {
                app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
            }
            app.run("select.all", json!({})).unwrap();
            app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
            let st = app.session.active_mut().unwrap();
            st.history.limit = 2;
            st.history.trim();
            let wanted = st.history.state(&st.doc, row).unwrap().clone();
            let view = app.view_info();
            crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Move, 15.0, 10.0), view);
            app.session.tool_poll(view, true).unwrap();
            crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Down, 15.0, 10.0), view);
            crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Up, 15.0, 10.0), view);
            assert!(app.session.tool_waiting());
            let mut h = row_menu_harness(app, row);
            if finishes_while_open {
                let requests = h.app().session.finish_tool_edit();
                crate::canvas::apply_requests(&mut h.app(), requests);
                h.frame();
                h.frame();
            }
            h.click_menu_item("New Document from State");
            h.frame();
            h.frame();
            let app = h.app();
            assert!(!app.session.tool_waiting());
            if row == 0 {
                assert_eq!(app.session.documents().len(), 1, "the pruned snapshot must not copy its neighbor");
                assert_eq!(app.ui.status, "That history state is no longer available.");
            } else {
                assert_eq!(app.session.active_index(), Some(1));
                assert_eq!(app.session.active().unwrap().doc.layers, wanted.layers, "the retained snapshot moved from row 1 to row 0");
            }
        }
    }
}
