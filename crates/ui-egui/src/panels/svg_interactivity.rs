//! SVG Interactivity panel (Window → SVG Interactivity): the JavaScript the selected objects run on
//! each event in web output. Pick an Event, type its JavaScript and press Enter to add it
//! (`svgInteractivity.setEvent`, which an empty field turns into a removal). The list shows the
//! objects' events, and a click on one picks it to edit. Remove Selected Entry
//! (`svgInteractivity.removeEvent`) and the panel menu's Clear Events (`svgInteractivity.clearEvents`)
//! remove them; Link JavaScript Files… opens the JavaScript Files dialog (`ui.javascriptFilesDialog`).
//! Everything reads `svgInteractivity.info`.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, SVG_EVENTS};

use super::{pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

pub const ID: &str = "svgInteractivity";
/// Panel state: the event picked (an entry of [`SVG_EVENTS`]).
const EVENT: &str = "svgi-event";
const LABEL_W: f32 = 70.0;

/// The panel's values: whether there are targets, and their events (`None` where they differ).
#[derive(Clone, Debug, Default)]
struct Info {
    has: bool,
    events: Option<Vec<(String, String)>>,
}

/// `svgInteractivity.info`, read again only when the document or the selection changes.
fn state(app: &mut VectorcraftApp, ctx: &egui::Context) -> Info {
    type Key = (u64, u64, usize, Option<NodeId>, Option<NodeId>);
    let Some(st) = app.session.active() else { return Info::default() };
    let sel = &st.selection.objects;
    let key: Key = (st.uid, st.revision, sel.len(), sel.first().copied(), sel.last().copied());
    let id = egui::Id::unique("svgi-state");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<(Key, Info)>(id))
        && k == key
    {
        return v;
    }
    let r = app.session.execute("svgInteractivity.info", &json!({})).unwrap_or_default();
    let events = r["events"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| Some((e["event"].as_str()?.to_string(), e["script"].as_str()?.to_string()))).collect::<Vec<_>>());
    let v = Info { has: r["ids"].as_array().is_some_and(|a| !a.is_empty()), events };
    ctx.data_mut(|d| d.insert_temp(id, (key, v.clone())));
    v
}

/// The event picked in the panel (the first of [`SVG_EVENTS`] until one is).
fn picked(ctx: &egui::Context) -> &'static str {
    let e: String = pstate(ctx, EVENT);
    SVG_EVENTS.iter().find(|n| **n == e).copied().unwrap_or(SVG_EVENTS[0])
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let info = state(app, &ctx);
    let event = picked(&ctx);
    let events = info.events.clone().unwrap_or_default();
    widgets::label_row(ui, tl!("Event:"), LABEL_W, |ui| {
        ui.add_enabled_ui(info.has, |ui| {
            let width = (ui.available_width() - 4.0).max(80.0);
            if let Some(i) = widgets::dropdown_names(ui, "svgi-event", event, &SVG_EVENTS, width) {
                set_pstate(&ctx, EVENT, SVG_EVENTS.get(i).copied().unwrap_or(event).to_string());
            }
        });
    });
    // Blank where the objects' events differ; empty where none of them has the event.
    let script = info.events.as_ref().map(|l| l.iter().find(|(e, _)| e == event).map_or("", |(_, js)| js.as_str()));
    widgets::label_row(ui, tl!("JavaScript:"), LABEL_W, |ui| {
        ui.add_enabled_ui(info.has, |ui| {
            let width = (ui.available_width() - 4.0).max(80.0);
            if let Some(js) = widgets::text_field(ui, ("svgi-script", event), script, width, 1) {
                app.run("svgInteractivity.setEvent", json!({ "event": event, "script": js })).ok();
            }
        });
    });
    ui.add_space(4.0);
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("svgi-events").max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.set_min_height(90.0);
            for (e, js) in &events {
                // Event names and scripts are data: shown as they are.
                if ui.selectable_label(e == event, format!("{e}: {js}")).clicked() {
                    set_pstate(&ctx, EVENT, e.clone());
                }
            }
        });
    });
    if !info.has {
        widgets::dim_label(ui, if selection_len(app) == 0 { tl!("No Selection") } else { "" });
    }
    let has_doc = app.session.active().is_some();
    let listed = events.iter().any(|(e, _)| e == event);
    let (mut link, mut remove) = (false, false);
    widgets::bottom_bar(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            remove = widgets::icon_button_enabled(ui, "trash-2", tl!("Remove Selected Entry"), false, listed, 24.0).clicked();
            link = widgets::icon_button_enabled(ui, "file-plus", tl!("Link JavaScript Files…"), false, has_doc, 24.0).clicked();
        });
    });
    if remove {
        app.run("svgInteractivity.removeEvent", json!({ "event": event })).ok();
    }
    if link {
        crate::menus::invoke(app, "ui.javascriptFilesDialog", json!({}));
    }
}

/// The panel (≡) menu: Clear Events.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let info = state(app, ui.ctx());
    // (Where the objects' events differ, some of them have events.)
    let any = info.has && info.events.as_ref().is_none_or(|l| !l.is_empty());
    if menu_item(ui, tl!("Clear Events"), any, false) {
        app.run("svgInteractivity.clearEvents", json!({})).ok();
    }
}

/// `ui.javascriptFilesDialog {}`: open the JavaScript Files dialog for the active document.
pub(crate) fn files_dialog(app: &mut VectorcraftApp, _p: &Value) -> Result<Value, String> {
    crate::dialogs::javascript_files::open(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// Run the panel and its menu for one headless frame of `ctx`: the texts drawn.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(app, ui);
            menu(app, ui);
        });
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    }

    fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
        app.session.execute(id, &p).unwrap()
    }

    /// Window › SVG Interactivity opens the panel, as Window's other panel items do.
    #[test]
    fn window_svg_interactivity_opens_the_panel() {
        let app = VectorcraftApp::new(Session::new(), Default::default());
        let entries = crate::menus::menu_entries(&app);
        let panel = entries
            .iter()
            .find(|e| e.command.as_deref() == Some("window.panel") && e.params.get("panel").and_then(Value::as_str) == Some(ID))
            .expect("the SVG Interactivity panel is listed");
        assert_eq!((panel.path.clone(), panel.label.as_str(), panel.enabled), (vec!["Window".to_string()], "SVG Interactivity", true));
    }

    // (Only names and scripts are checked: they are shown as they are, while tests running
    // alongside change the language the labels are drawn in.)
    #[test]
    fn panel_lists_the_selections_events() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let texts = frame(&mut app, &ctx);
        assert!(texts.iter().any(|t| t == "onfocusin"), "the first event is picked: {texts:?}");
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        run(&mut app, "svgInteractivity.setEvent", json!({"event": "onclick", "script": "alert(1)"}));
        run(&mut app, "svgInteractivity.setEvent", json!({"event": "onmouseover", "script": "hover()"}));
        let texts = frame(&mut app, &ctx);
        for want in ["onclick: alert(1)", "onmouseover: hover()"] {
            assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
        }
        assert!(!texts.iter().any(|t| t == "alert(1)"), "{texts:?}");
        // With an event picked, the field shows its script.
        set_pstate(&ctx, EVENT, "onclick".to_string());
        let texts = frame(&mut app, &ctx);
        assert!(texts.iter().any(|t| t == "onclick") && texts.iter().any(|t| t == "alert(1)"), "{texts:?}");
        // Two objects whose events differ show none.
        let b = run(&mut app, "shape.ellipse", json!({"x": 60, "y": 0, "width": 30, "height": 30}))["id"].clone();
        let a = app.session.active().unwrap().doc.layers[0].children().unwrap()[0].id.0;
        run(&mut app, "select.set", json!({"ids": [a, b]}));
        let texts = frame(&mut app, &ctx);
        assert!(!texts.iter().any(|t| t.starts_with("onclick:") || t == "alert(1)"), "{texts:?}");
    }
}
