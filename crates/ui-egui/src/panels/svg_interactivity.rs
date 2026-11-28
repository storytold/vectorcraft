//! SVG Interactivity panel (Window → SVG Interactivity): the JavaScript the selected objects run on
//! each event in web output. Pick an Event, type its JavaScript and press Enter to add it
//! (`svgInteractivity.setEvent`, which an empty field turns into a removal). The list shows the
//! objects' events, and a click on one picks it to edit. Remove Selected Entry
//! (`svgInteractivity.removeEvent`) and the panel menu's Clear Events (`svgInteractivity.clearEvents`)
//! remove them; Link JavaScript Files… opens the JavaScript Files dialog (`ui.javascriptFilesDialog`).
//! Everything reads `svgInteractivity.info`.

use std::hash::{DefaultHasher, Hash, Hasher};

use craft::behavior::{IconId, Selection, SelectionChange, keyed};
use craft::keymap::CommandRef;
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Politeness, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, announce,
};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Stack;
use craft::ui::list::{List, ListRow};
use craft::ui::menu::PanelRow;
use craft::ui::pickers::Select;
use craft::ui::primitives::SubmitKey;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::sections::{FooterAction, FooterBar};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, SVG_EVENTS};

use super::selection_len;
use crate::VectorcraftApp;

pub const ID: &str = "svgInteractivity";

/// The engine command of the panel menu's one row.
const CLEAR_EVENTS: &str = "svgInteractivity.clearEvents";

/// The SVG Interactivity panel's own state, kept while the app runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct SvgInteractivityPanel {
    /// The event picked to edit ([`picked`] reads it).
    pub(crate) event: String,
    list_stamp: u64,
}

/// The panel's values: whether there are targets, and their events (`None` where they differ).
#[derive(Clone, Debug, Default)]
pub(crate) struct Info {
    has: bool,
    events: Option<Vec<(String, String)>>,
    script_target: Option<egui::IdSalt>,
}

/// What [`info`] was read for: drawing (uid and File Revert count), revision, and selection size and ends.
pub(crate) type SelKey = ((u64, u64), u64, usize, Option<NodeId>, Option<NodeId>);

/// A side label's text without its colon: the label draws the colon itself
/// (`CommonWords::caption`), so the catalog's "Event:" keeps its translation here.
// PLAN: the catalog keys carry the colon the side label now adds; strip it where a label is given.
pub(crate) fn caption(label: &'static str) -> &'static str {
    crate::i18n::t(label).trim_end_matches(':')
}

fn sel_key(app: &VectorcraftApp) -> Option<SelKey> {
    let st = app.session.active()?;
    let sel = &st.selection.objects;
    Some(((st.uid, st.reverts), st.revision, sel.len(), sel.first().copied(), sel.last().copied()))
}

/// The memoized [`Info`] when it was read for the current document and selection.
fn cached(app: &VectorcraftApp) -> Option<&Info> {
    let key = sel_key(app)?;
    app.canvas.svg_info.as_ref().filter(|(k, _)| *k == key).map(|(_, v)| v)
}

/// `svgInteractivity.info`, read again only when the document or the selection changes.
pub(crate) fn info(app: &mut VectorcraftApp) -> Info {
    if let Some(v) = cached(app) {
        return v.clone();
    }
    let Some(key) = sel_key(app) else { return Info::default() };
    let r = app.session.execute("svgInteractivity.info", &json!({})).unwrap_or_default();
    let events = r["events"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| Some((e["event"].as_str()?.to_string(), e["script"].as_str()?.to_string()))).collect::<Vec<_>>());
    let mut targets: Vec<u64> = r["ids"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
    targets.sort_unstable();
    let has = !targets.is_empty();
    // The editor's draft belongs to this drawing generation and its targets, independent of selection order.
    let script_target = has.then(|| egui::IdSalt::new((key.0, targets)));
    let v = Info { has, events, script_target };
    app.canvas.svg_info = Some((key, v.clone()));
    v
}

/// The event picked in the panel (the first of [`SVG_EVENTS`] until one is).
fn picked(app: &VectorcraftApp) -> &'static str {
    let e = app.ui.svg_interactivity_panel.event.as_str();
    SVG_EVENTS.iter().find(|n| **n == e).copied().unwrap_or(SVG_EVENTS[0])
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let info = info(app);
    let event = picked(app);
    let events = info.events.as_deref().unwrap_or_default();
    let names: Vec<Entry<String>> = SVG_EVENTS.iter().map(|e| Entry::new((*e).to_string(), *e)).collect();
    let mut pick = None;
    Field::new(caption("Event:")).show(ui, |ui, _| {
        let out = Select::new(caption("Event:"), Bind::given(event.to_string()), &names).enabled(info.has).show(ui);
        pick = out.events.committed().cloned();
    });
    let menu_changed = pick.as_deref().is_some_and(|picked| picked != event);
    let reveal = pick.as_ref().filter(|picked| menu_changed && events.iter().any(|(key, _)| key == *picked)).cloned();
    // Blank where the objects' events differ; empty where none of them has the event.
    let script = info.events.as_ref().map(|l| l.iter().find(|(e, _)| e == event).map_or("", |(_, js)| js.as_str()));
    let shown = script.unwrap_or_default();
    let mut typed = None;
    Field::new(caption("JavaScript:")).show(ui, |ui, _| {
        let out = TextField::new(caption("JavaScript:"), Bind::given(shown.to_string()))
            .lines(3)
            .submits()
            .submit_key(SubmitKey::Enter)
            .id_salt((event, info.script_target))
            .enabled(info.has)
            .show(ui);
        typed = out.events.committed().filter(|js| js.as_str() != shown).cloned();
    });
    let source = keyed(events, |(e, _): &(String, String)| e.clone());
    let mut stamp = DefaultHasher::new();
    app.ui.svg_interactivity_panel.list_stamp.hash(&mut stamp);
    for (key, _) in events {
        key.hash(&mut stamp);
    }
    let rows_stamp = stamp.finish();
    let mut remove = None;
    let mut footer_pick = None;
    let has_doc = app.session.active().is_some();
    let listed = events.iter().any(|(e, _)| e == event);
    Stack::new()
        .fill()
        .footer(|ui, _| {
            let footer = FooterBar::actions([
                FooterAction::new(IconId::new("trash"), tl!("Remove Selected Entry")).enabled(listed),
                FooterAction::new(IconId::new("new_file"), tl!("Link JavaScript Files…")).enabled(has_doc),
            ])
            .show_footer(ui);
            if !ui.is_sizing_pass() {
                footer_pick = footer.picked;
            }
        })
        .show(ui, |ui, _| {
            if !info.has && selection_len(app) == 0 {
                Text::new(tl!("No Selection")).shade(Shade::Muted).show(ui);
                return;
            }
            let selection = if listed { Selection::single(event.to_string()) } else { Default::default() };
            let mut list = List::<String>::new(tl!("Events"))
                .select(SelectMode::Single)
                .arrows_select(true)
                .deletable(true)
                .rows_stamp(rows_stamp)
                .selection(Bind::given(selection))
                .sizing(Sizing::Fill);
            if let Some(key) = reveal.as_ref() {
                list = list.reveal(key.clone());
            }
            // Event names and scripts are data: shown as they are.
            let list = list.show_items(ui, &source, &mut |key, _| {
                let js = events.iter().find(|(e, _)| e == key).map_or("", |(_, js)| js.as_str());
                ListRow::new(format!("{key}: {js}"))
            });
            if ui.is_sizing_pass() {
                return;
            }
            for e in list.events {
                match e {
                    CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                        if let Some(key) = keys.into_iter().next() {
                            pick = Some(key);
                        }
                    }
                    CollectionEvent::Activate(key) => pick = Some(key),
                    CollectionEvent::Delete(_) => {
                        remove = pick.as_deref().or(listed.then_some(event)).map(str::to_owned);
                    }
                    _ => {}
                }
            }
        });
    if let Some(e) = pick {
        let state = &mut app.ui.svg_interactivity_panel;
        state.event = e;
        if menu_changed {
            // The list must seed its keyboard position from the new picker selection.
            state.list_stamp = state.list_stamp.wrapping_add(1);
        }
    }
    if let Some(js) = typed {
        app.run("svgInteractivity.setEvent", json!({ "event": event, "script": js })).ok();
    }
    match footer_pick {
        Some(0) => remove = Some(event.to_string()),
        Some(1) => {
            crate::menus::invoke(app, "ui.javascriptFilesDialog", json!({}));
        }
        _ => {}
    }
    if let Some(event) = remove
        && app.run("svgInteractivity.removeEvent", json!({ "event": event })).is_ok_and(|out| out["changed"].as_u64().unwrap_or(0) > 0)
    {
        let message = crate::i18n::fmt(tl!("Removed SVG event {event}"), &[("event", &event)]);
        announce(ui.ctx(), message, Politeness::Polite);
    }
}

/// The panel (≡) menu's rows: Clear Events.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    vec![PanelRow::Command(CommandRef::new(CLEAR_EVENTS))]
}

/// Clear Events is on while the objects have events. The panel reads the events each frame it shows; a
/// panel that is not shown leaves the row off.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    // (Where the objects' events differ, some of them have events.)
    let any = cached(app).is_some_and(|info| info.has && info.events.as_ref().is_none_or(|l| !l.is_empty()));
    states.set(CommandRef::new(CLEAR_EVENTS), CommandState::new().enabled(any));
}

/// The panel's own `ui.svgInteractivity.*` commands: it has none, because its menu row is the engine
/// command `svgInteractivity.clearEvents`.
pub(crate) fn run(_app: &mut VectorcraftApp, _id: &str, _params: &Value) -> Option<Result<Value, String>> {
    None
}

/// `ui.javascriptFilesDialog {}`: open the JavaScript Files dialog for the active document.
pub(crate) fn files_dialog(app: &mut VectorcraftApp, _p: &Value) -> Result<Value, String> {
    crate::dialogs::javascript_files::open(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use vectorcraft_engine::Session;

    #[test]
    fn script_draft_does_not_follow_the_editor_to_other_artwork() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        for name in ["First", "Second"] {
            app.run("file.new", json!({ "name": name, "width": 100, "height": 100 })).unwrap();
            run(&mut app, "shape.rectangle", json!({ "x": 0, "y": 0, "width": 50, "height": 50 }));
        }
        let mut panel = Panel::new(app, show);
        let field = caption("JavaScript:");
        let multiline = craft::ui::contract::Role::MULTILINE_TEXT_INPUT;
        let doc = panel.app().session.active().unwrap().uid;
        panel.click_role(field, multiline);
        panel.type_text("1;");
        assert_eq!(panel.value_role(field, multiline).as_deref(), Some("1;"));

        panel.app().run("window.nextDocument", json!({})).unwrap();
        panel.frame();
        assert_ne!(panel.app().session.active().unwrap().uid, doc);
        assert_eq!(panel.value_role(field, multiline).as_deref(), Some(""));
        assert_eq!(run(&mut panel.app(), "svgInteractivity.info", json!({}))["events"], json!([]));

        panel.click_role(field, multiline);
        panel.type_text("2;");
        let first = panel.app().session.active().unwrap().selection.objects[0].0;
        let second = run(&mut panel.app(), "shape.ellipse", json!({ "x": 0, "y": 0, "width": 10, "height": 20 }))["id"].clone();
        panel.frame();
        assert_eq!(panel.value_role(field, multiline).as_deref(), Some(""));
        assert_eq!(run(&mut panel.app(), "svgInteractivity.info", json!({}))["events"], json!([]));

        run(&mut panel.app(), "select.set", json!({ "ids": [first, second] }));
        panel.frame();
        panel.click_role(field, multiline);
        panel.type_text("3;");
        run(&mut panel.app(), "select.set", json!({ "ids": [second, first] }));
        panel.frame();
        assert_eq!(panel.value_role(field, multiline).as_deref(), Some("3;"));
    }

    fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
        app.session.execute(id, &p).unwrap()
    }

    /// Revert retires an unsubmitted script even when the saved event and object id are unchanged.
    #[test]
    fn native_file_revert_discards_the_unsubmitted_script_draft() {
        struct SavedDrawing(std::path::PathBuf);
        impl Drop for SavedDrawing {
            fn drop(&mut self) {
                // Clean the task-owned file on both a red baseline unwind and a successful test.
                let _ = std::fs::remove_file(self.0.join("saved.vectorcraft"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        for frame_without_selection in [false, true] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
            let saved_script = "saved();\nreturn true;";
            app.run("svgInteractivity.setEvent", json!({"event": SVG_EVENTS[0], "script": saved_script})).unwrap();
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("vectorcraft-svg-revert-{}-{stamp}-{frame_without_selection}", std::process::id()));
            std::fs::create_dir(&dir).unwrap();
            let saved_file = SavedDrawing(dir);
            let path = saved_file.0.join("saved.vectorcraft");
            app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
            app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
            app.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
            app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
            let mut panel = Panel::new(app, show);
            let field = caption("JavaScript:");
            let multiline = craft::ui::contract::Role::MULTILINE_TEXT_INPUT;
            let events = |panel: &Panel| run(&mut panel.app(), "svgInteractivity.info", json!({}))["events"].clone();
            let saved_events = json!([{"event": SVG_EVENTS[0], "script": saved_script}]);
            let discarded = "discarded();\npending();";
            panel.click_role(field, multiline);
            panel.key(egui::Key::A, egui::Modifiers::COMMAND);
            panel.type_text(discarded);
            assert!(panel.focused_nth(multiline, 0));
            assert_eq!(panel.value_role(field, multiline).as_deref(), Some(discarded));
            assert_eq!(events(&panel), saved_events, "the text remains an unsubmitted draft");

            let uid = panel.app().session.active().unwrap().uid;
            let reverts = panel.app().session.active().unwrap().reverts;
            panel.app().run("file.revert", json!({"confirmed": true})).unwrap();
            {
                let app = panel.app();
                let state = app.session.active().unwrap();
                assert_eq!(state.uid, uid);
                assert_eq!(state.reverts, reverts + 1, "the native Revert replaced the drawing contents");
                assert!(state.doc.node(NodeId(id)).is_some(), "the saved object keeps its node id");
                assert!(state.selection.objects.is_empty());
            }
            if frame_without_selection {
                panel.frame();
            }
            panel.app().run("select.set", json!({"ids": [id]})).unwrap();
            panel.frame();
            assert_eq!(events(&panel), saved_events);
            let shown_after_revert = panel.value_role(field, multiline);
            let original = panel.app().session.active().unwrap().doc.clone();
            let revision = panel.app().session.active().unwrap().revision;
            let undo = panel.app().session.active().unwrap().history.undo.len();
            panel.key(egui::Key::Enter, egui::Modifiers::NONE);
            assert_eq!(events(&panel), saved_events, "Enter must not submit the retired draft");
            assert_eq!(shown_after_revert.as_deref(), Some(saved_script), "the discarded script leaves the editor");
            assert_eq!(panel.app().session.active().unwrap().doc, original);
            assert_eq!(panel.app().session.active().unwrap().revision, revision);
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);

            panel.click_role(field, multiline);
            panel.key(egui::Key::A, egui::Modifiers::COMMAND);
            panel.type_text("fresh();");
            panel.key(egui::Key::Enter, egui::Modifiers::NONE);
            assert_eq!(events(&panel), json!([{"event": SVG_EVENTS[0], "script": "fresh();"}]));
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1, "a fresh edit still submits normally");
        }
    }

    #[test]
    fn enter_applies_the_event_script_and_shift_enter_adds_a_line() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        let mut panel = Panel::new(app, show);

        panel.click_role(caption("JavaScript:"), craft::ui::contract::Role::MULTILINE_TEXT_INPUT);
        panel.type_text("1;");
        panel.key(egui::Key::Enter, egui::Modifiers::SHIFT);
        panel.type_text("2;");
        let events = |app: &mut VectorcraftApp| run(app, "svgInteractivity.info", json!({}))["events"].clone();
        assert_eq!(events(&mut panel.app()), json!([]), "the script is still a draft");

        panel.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(events(&mut panel.app()), json!([{"event": SVG_EVENTS[0], "script": "1;\n2;"}]));
    }

    // (Only names and scripts are checked: they are shown as they are, while tests running
    // alongside change the language the labels are drawn in.)
    #[test]
    fn panel_lists_the_selections_events() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let mut panel = Panel::new(app, show);
        panel.frame();
        assert!(!panel.shows("onclick: alert(1)"), "nothing is selected");
        {
            let mut app = panel.app();
            run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
            run(&mut app, "svgInteractivity.setEvent", json!({"event": "onclick", "script": "alert(1)"}));
            run(&mut app, "svgInteractivity.setEvent", json!({"event": "onmouseover", "script": "hover()"}));
        }
        panel.frame();
        for want in ["onclick: alert(1)", "onmouseover: hover()"] {
            assert!(panel.shows(want), "{want} is listed");
        }
        // With an event picked, the field shows its script.
        panel.app().ui.svg_interactivity_panel.event = "onclick".into();
        panel.frame();
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some("alert(1)"));
        // Two objects whose events differ show none.
        {
            let mut app = panel.app();
            let b = run(&mut app, "shape.ellipse", json!({"x": 60, "y": 0, "width": 30, "height": 30}))["id"].clone();
            let a = app.session.active().unwrap().doc.layers[0].children().unwrap()[0].id.0;
            run(&mut app, "select.set", json!({"ids": [a, b]}));
        }
        panel.frame();
        assert!(!panel.shows("onclick: alert(1)"));
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some(""));
        assert!(info(&mut panel.app()).events.is_none());
    }

    #[test]
    fn keyboard_selection_edits_and_removes_the_chosen_event() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        run(&mut app, "svgInteractivity.setEvent", json!({"event": "onclick", "script": "1;"}));
        run(&mut app, "svgInteractivity.setEvent", json!({"event": "onmouseover", "script": "2;"}));
        let mut panel = Panel::new(app, show);

        panel.click("onclick: 1;");
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some("1;"));
        panel.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some("2;"));

        panel.click(caption("Event:"));
        panel.click("onclick");
        panel.focus(tl!("Events"));
        panel.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some("2;"));

        panel.key(egui::Key::ArrowUp, egui::Modifiers::SHIFT);
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some("2;"));
        let delete =
            |pressed| egui::Event::Key { key: egui::Key::Delete, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        panel.frame_with(vec![delete(true)]);
        let removed = crate::i18n::fmt(tl!("Removed SVG event {event}"), &[("event", "onmouseover")]);
        assert!(panel.announces(&removed), "the removal is announced");
        panel.frame_with(vec![delete(false)]);
        panel.frame();
        assert!(panel.shows("onclick: 1;"));
        assert!(!panel.shows("onmouseover: 2;"));
        assert_eq!(panel.app().session.active().unwrap().selection.objects.len(), 1);
        panel.app().run("edit.undo", json!({})).unwrap();
        panel.frame();
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some("2;"));
        panel.key(egui::Key::Delete, egui::Modifiers::NONE);
        assert!(!panel.shows("onmouseover: 2;"));
        panel.key(egui::Key::Delete, egui::Modifiers::NONE);
        assert!(panel.shows("onclick: 1;"), "Delete must leave an unselected event alone");
        panel.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some("1;"));
        panel.click("Remove Selected Entry");
        assert!(!panel.shows("onclick: 1;"));
    }

    #[test]
    fn picking_an_event_reveals_its_row_in_a_short_panel() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({ "width": 100, "height": 100 }));
        run(&mut app, "shape.rectangle", json!({ "x": 0, "y": 0, "width": 50, "height": 50 }));
        for (i, event) in SVG_EVENTS.into_iter().enumerate() {
            run(&mut app, "svgInteractivity.setEvent", json!({ "event": event, "script": i.to_string() }));
        }
        let script = (SVG_EVENTS.len() - 1).to_string();
        let last = format!("{}: {script}", SVG_EVENTS.last().unwrap());
        let mut panel = Panel::new(app, show);
        panel.room(egui::vec2(300.0, 220.0));
        panel.frame();
        assert!(!panel.shows(&last) || !panel.in_view(&last));

        panel.click(caption("Event:"));
        panel.key(egui::Key::End, egui::Modifiers::NONE);
        panel.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(panel.value(caption("JavaScript:")).as_deref(), Some(script.as_str()));
        assert!(panel.in_view(&last), "the picker scrolls the selected event into view");
    }
}
