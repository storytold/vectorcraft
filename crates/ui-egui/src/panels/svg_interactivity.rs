//! SVG Interactivity panel (Window → SVG Interactivity): the JavaScript the selected objects run on
//! each event in web output. Pick an Event, type its JavaScript and press Enter to add it
//! (`svgInteractivity.setEvent`, which an empty field turns into a removal). The list shows the
//! objects' events, and a click on one picks it to edit. Remove Selected Entry
//! (`svgInteractivity.removeEvent`) and the panel menu's Clear Events (`svgInteractivity.clearEvents`)
//! remove them; Link JavaScript Files… opens the JavaScript Files dialog (`ui.javascriptFilesDialog`).
//! Everything reads `svgInteractivity.info`.

use craft::behavior::{IconId, Selection, keyed};
use craft::keymap::CommandRef;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Sizing};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
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
}

/// The panel's values: whether there are targets, and their events (`None` where they differ).
#[derive(Clone, Debug, Default)]
pub(crate) struct Info {
    has: bool,
    events: Option<Vec<(String, String)>>,
    script_target: Option<egui::IdSalt>,
}

/// What [`info`] was read for: (document uid, revision, selection size, first and last selected object).
pub(crate) type SelKey = (u64, u64, usize, Option<NodeId>, Option<NodeId>);

/// A side label's text without its colon: the label draws the colon itself
/// (`CommonWords::caption`), so the catalog's "Event:" keeps its translation here.
// PLAN: the catalog keys carry the colon the side label now adds; strip it where a label is given.
pub(crate) fn caption(label: &'static str) -> &'static str {
    crate::i18n::t(label).trim_end_matches(':')
}

fn sel_key(app: &VectorcraftApp) -> Option<SelKey> {
    let st = app.session.active()?;
    let sel = &st.selection.objects;
    Some((st.uid, st.revision, sel.len(), sel.first().copied(), sel.last().copied()))
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
    // The editor's draft belongs to the actual targets, even if the selection is reordered.
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
    let selection = if events.iter().any(|(e, _)| e == event) { Selection::single(event.to_string()) } else { Default::default() };
    let list = List::<String>::options(tl!("Events"))
        .selection(Bind::given(selection))
        .sizing(Sizing::Fill)
        // Event names and scripts are data: shown as they are.
        .show_items(ui, &source, &mut |key, _| {
            let js = events.iter().find(|(e, _)| e == key).map_or("", |(_, js)| js.as_str());
            ListRow::new(format!("{key}: {js}"))
        });
    for e in list.events {
        if let CollectionEvent::Activate(key) = e {
            pick = Some(key);
        }
    }
    if !info.has && selection_len(app) == 0 {
        Text::new(tl!("No Selection")).shade(Shade::Muted).show(ui);
    }
    let has_doc = app.session.active().is_some();
    let listed = events.iter().any(|(e, _)| e == event);
    let footer = FooterBar::actions([
        FooterAction::new(IconId::new("trash"), tl!("Remove Selected Entry")).enabled(listed),
        FooterAction::new(IconId::new("new_file"), tl!("Link JavaScript Files…")).enabled(has_doc),
    ])
    .show_footer(ui);
    if let Some(e) = pick {
        app.ui.svg_interactivity_panel.event = e;
    }
    if let Some(js) = typed {
        app.run("svgInteractivity.setEvent", json!({ "event": event, "script": js })).ok();
    }
    match footer.picked {
        Some(0) => {
            app.run("svgInteractivity.removeEvent", json!({ "event": event })).ok();
        }
        Some(1) => {
            crate::menus::invoke(app, "ui.javascriptFilesDialog", json!({}));
        }
        _ => {}
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
}
