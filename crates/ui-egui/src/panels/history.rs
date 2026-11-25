//! History panel: the document's states; click one to go back (or forward) to it.

use craft::keymap::CommandRef;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::panels::{HistoryEvent, HistoryFooter, HistoryKey, HistoryPanel, HistoryRow};
use craft::ui::runtime::commands::{CommandState, CommandStates};
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
/// document holds it. It returns `None` when the catalog has no row for the template. A step that
/// applied or edited a built-in effect ([`vectorcraft_engine::HistoryEntry::effect`]) takes the
/// effect's name as the Effect menu shows it ([`crate::i18n::effect_name`]), `None` when the
/// catalog has no row for it. Any other step takes [`step_name`] of its label.
pub fn entry_name(lang: crate::i18n::Lang, h: &vectorcraft_engine::HistoryEntry) -> Option<String> {
    use crate::i18n::{effect_name, fmt, has, has_effect_name, label_or_name, tr};
    if let Some(effect) = h.effect() {
        return has_effect_name(lang, effect).then(|| effect_name(lang, effect)).flatten();
    }
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

/// Moves the document to state `target`, stopping at the first step that fails.
fn step_to(app: &mut VectorcraftApp, target: usize) {
    let Some(current) = app.session.active().map(|st| st.history.undo.len()) else { return };
    let s = steps_to(current, target);
    let (cmd, n) = if s < 0 { ("edit.undo", -s) } else { ("edit.redo", s) };
    for _ in 0..n {
        if app.run(cmd, json!({})).is_err() {
            break;
        }
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(undone) = app.session.active().map(|st| st.history.undo.len()) else {
        super::empty_state(ui, "history", tl!("No document"), tl!("Open a document to see its history."));
        return;
    };
    let rows = rows(app);
    // A click on a row moves to its state at once, so the current state is the selected one.
    let out = HistoryPanel::new(&rows, Some(HistoryKey::State(undone as u64)))
        .footer(HistoryFooter::default().new_document(true))
        .can_delete(|_| false)
        .show(ui);
    for event in out.events {
        match event {
            HistoryEvent::Select(HistoryKey::State(n)) => step_to(app, n as usize),
            HistoryEvent::NewDocument => crate::menus::invoke(app, "edit.newDocumentFromState", json!({})),
            _ => {}
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

    /// A step that applies no effect, named like a menu item (Object › Path › Clean Up…), takes its
    /// plain row, else the menu item's row without the ellipsis; a name with no row has none.
    #[test]
    fn plain_steps_read_the_menu_row_and_unknown_steps_have_no_name() {
        for lang in crate::i18n::Lang::all() {
            let menu_row = crate::i18n::has(lang, "Clean Up…").then(|| crate::i18n::tr(lang, "Clean Up…").trim_end_matches('…').to_string());
            let plain_row = crate::i18n::has(lang, "Clean Up").then(|| crate::i18n::tr(lang, "Clean Up").to_string());
            assert_eq!(step_name(lang, "Clean Up"), plain_row.or(menu_row), "{}", lang.code());
        }
        let ja = crate::i18n::Lang::from_code("ja").expect("ja registered");
        assert_eq!(step_name(ja, "No Such Step"), None);
    }

    /// Every built-in effect has one name in each language: History (and Edit › Undo and Redo)
    /// names the step that applied it, and the Appearance panel its row, with the text of its
    /// Effect menu item without the ellipsis, which is the effect's `@id` row or the row of its
    /// menu label with the ellipsis.
    #[test]
    fn effects_have_the_same_name_in_history_appearance_and_the_effect_menu() {
        let mut app = app_with_two_rectangles();
        // The step each effect that applies to a rectangle with its defaults records.
        let mut steps = Vec::new();
        for e in vectorcraft_effects::effect_catalog() {
            if app.session.execute("effect.apply", &json!({ "effect": e.id })).is_ok() {
                let h = app.session.active().unwrap().history.undo.last().cloned().unwrap();
                assert_eq!(h.effect(), Some(e.id), "{} names its step after the effect", e.id);
                steps.push((e, h));
                app.session.execute("edit.undo", &json!({})).unwrap();
            }
        }
        assert!(steps.len() * 2 > vectorcraft_effects::effect_catalog().len(), "only {} effects applied", steps.len());
        let mut differ = Vec::new();
        for lang in crate::i18n::Lang::all() {
            crate::i18n::set_current(lang);
            for (e, h) in &steps {
                let cmd = if e.defaults.as_object().is_some_and(|o| o.is_empty()) { "effect.apply" } else { "effect.dialog" };
                let item = crate::menus::shown_label(&app, cmd, &json!({ "effect": e.id }), e.label);
                let menu = item.trim_end_matches('…');
                let row = crate::i18n::tr_id(lang, e.id, e.label).trim_end_matches('…').to_string();
                let step = shown(h);
                let shown_row = super::super::appearance::shown_effect_label(e.id);
                if menu != row || step != menu || shown_row != menu {
                    differ.push(format!("{} {}: catalog {row:?}, menu {menu:?}, History {step:?}, Appearance {shown_row:?}", lang.code(), e.id));
                }
            }
        }
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert!(differ.is_empty(), "{differ:#?}");
    }

    /// A step another command names like an effect (Object › Transform's `Transform`, the name of
    /// Distort & Transform › Transform) keeps its own name in every language.
    #[test]
    fn a_step_named_like_an_effect_keeps_its_own_name() {
        let mut app = app_with_two_rectangles();
        app.session.execute("object.transform", &json!({ "matrix": [1, 0, 0, 1, 5, 0] })).unwrap();
        let h = app.session.active().unwrap().history.undo.last().cloned().unwrap();
        assert_eq!((h.label.as_str(), h.effect()), ("Transform", None));
        for lang in crate::i18n::Lang::all() {
            assert_eq!(entry_name(lang, &h), step_name(lang, "Transform"), "{}", lang.code());
        }
    }

    /// The Feather dialog's preview, kept with OK, records a step that History and Edit › Undo and
    /// Redo name as the Effect menu names Feather, in French as in English.
    #[test]
    fn an_effect_kept_from_its_dialog_preview_is_named_after_the_effect() {
        let fr = crate::i18n::Lang::from_code("fr").expect("fr registered");
        let mut app = crate::testing::windowed(app_with_two_rectangles());
        crate::i18n::set_current(fr);
        let item = crate::menus::shown_label(&app, "effect.dialog", &json!({ "effect": "stylize.feather" }), "Feather…");
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let menu = item.trim_end_matches('…').to_string();
        let ctx = app.ui_ctx.clone().expect("a window");
        app.run("effect.dialog", json!({ "effect": "stylize.feather", "item": null })).unwrap();
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.session.in_interaction(), "the dialog previews");
        let id = app.effect_dialogs.effect.as_ref().expect("the Feather dialog is open").id;
        app.dialogs().expect("a window").with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(app.effect_dialogs.effect.is_none(), "OK closed the dialog");
        let h = app.session.active().unwrap().history.undo.last().cloned().unwrap();
        assert_eq!(h.effect(), Some("stylize.feather"));
        app.session.execute("edit.undo", &json!({})).unwrap();
        let undone = app.session.active().unwrap().history.redo.last().cloned().unwrap();
        crate::i18n::set_current(fr);
        let names = (shown(&h), shown(&undone));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert_eq!(names, (menu.clone(), menu), "the step, and the step undone");
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
        let ctx = crate::testing::ui_context();
        ctx.enable_accesskit();
        egui_extras::install_image_loaders(&ctx);
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let out = crate::testing::part_frame(app, &ctx, egui::RawInput { events, screen_rect: screen, ..Default::default() }, show);
            out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
        };
        frame(&mut app, vec![]);
        let nodes = frame(&mut app, vec![]);
        // PLAN: the button is named by the framework's words ("Create New Document from Current State"); the name is
        // matched by the words it holds.
        let button = nodes.iter().map(|(_, n)| n).find(|n| n.label().is_some_and(|l| l.contains("New Document"))).expect("the button");
        let at = button.bounds().expect("its bounds");
        let at = egui::pos2((at.x0 + at.x1) as f32 / 2.0, (at.y0 + at.y1) as f32 / 2.0);
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(&mut app, vec![press(false)]);
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.session.active_index(), Some(1), "the new document is the active one");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layers[0].children().unwrap().len(), 1, "the art of the selected state");
        assert!(st.is_dirty(), "the new document is unsaved");
        assert!(std::sync::Arc::ptr_eq(&app.session.documents()[0].doc, &source), "the source stays at that state");
    }
}
