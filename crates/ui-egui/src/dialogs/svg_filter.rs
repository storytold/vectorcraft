//! Apply SVG Filter (Effect › SVG Filters › Apply SVG Filter…): the document's SVG filters in a
//! list. OK applies the selected one to the selection as a live effect (`svgFilter.apply`), or,
//! opened from the Appearance panel for an applied SVG filter, puts it in that effect's place
//! (`effect.setParams`). New and Edit open the filter's code under the list; Save adds the filter
//! or replaces the one being edited (`svgFilter.define`). Delete removes the selected filter
//! (`svgFilter.delete`) and Import… adds the filters of an SVG file (`svgFilter.import`). Those
//! act at once. Effect › SVG Filters lists the same filters below its first items.
//!
//! Fields: `selected` (a filter's name), `code` (the code being edited), `editing` (`""`, `"new"`
//! or the name of the filter being edited), and with an applied effect `__index`, `__item` and
//! `missing` (the name of a deleted filter the effect names).

use serde_json::{Value, json};
use vectorcraft_engine::doc::SvgFilter;

use super::DialogSpec;
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::{FilePick, VectorcraftApp, widgets};

/// The dialog kind of Apply SVG Filter.
pub const KIND: &str = "svgFilter";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Apply SVG Filter…").trim_end_matches('…').to_string(),
    body,
    confirm,
    ok_enabled: Some(|app, d| d.str("editing").is_empty() && filters(app).iter().any(|f| f.name == d.str("selected"))),
    min_width: 440.0,
    ..DialogSpec::FORM
};

/// The active document's SVG filters.
fn filters(app: &VectorcraftApp) -> Vec<SvgFilter> {
    app.session.active().map(|st| st.doc.svg_filters.clone()).unwrap_or_default()
}

/// `ui.svgFilterDialog {index?, item?}`: open Apply SVG Filter for the active document; with
/// `index`, for the applied SVG filter at `index` of `item` (null: the object's effects): the
/// document's filter of its name selected; when the document has none of its name, the name shown
/// above the list if the filter was deleted, or else its code open as a new filter.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document open".into());
    }
    let mut fields = json!({ "selected": "", "code": "", "editing": "" });
    if let Some(k) = p.get("index").and_then(Value::as_u64) {
        let mut q = json!({});
        if let Some(item) = p.get("item") {
            q["item"] = item.clone();
        }
        let applied = app.session.appearance_effect_target(&q).map(|(_, fx)| fx.to_vec()).unwrap_or_default();
        let e = applied.get(k as usize).filter(|e| e.id == vectorcraft_effects::SVG_FILTER).ok_or_else(|| format!("no SVG filter at index {k}"))?;
        let name = e.params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let xml = e.params.get("xml").and_then(Value::as_str).unwrap_or_default().to_string();
        let held = filters(app).into_iter().find(|f| f.name == name);
        match held {
            // The document's filter it names: selected.
            Some(_) => fields["selected"] = json!(name),
            // Deleted from the document: shown above the list, not selectable; OK waits for a choice.
            None if xml.is_empty() => fields["missing"] = json!(name),
            // A filter the document doesn't hold (from an opened file): its code, to add it.
            None => {
                fields["code"] = json!(xml);
                fields["editing"] = json!("new");
            }
        }
        fields["__index"] = json!(k);
        // As given: `null` is the object's own effects, and none is the active item's.
        if let Some(item) = p.get("item") {
            fields["__item"] = item.clone();
        }
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

/// OK: the selected filter to the selection, or in the applied effect's place.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let name = d.str("selected");
    let f = filters(app).into_iter().find(|f| f.name == name).ok_or("select an SVG filter")?;
    let r = match d.fields.get("__index").and_then(Value::as_u64) {
        Some(k) => {
            let mut p = json!({ "index": k, "params": { "name": f.name, "xml": f.xml } });
            if let Some(item) = d.fields.get("__item") {
                p["item"] = item.clone();
            }
            app.run("effect.setParams", p)?
        }
        None => app.run("svgFilter.apply", json!({ "name": f.name }))?,
    };
    app.ui.dialog = None;
    Ok(r)
}

/// The active document's filter names, mirrored for the (static) menu tree ([`sync`]).
fn names_store() -> &'static std::sync::RwLock<Vec<&'static str>> {
    static NAMES: std::sync::OnceLock<std::sync::RwLock<Vec<&'static str>>> = std::sync::OnceLock::new();
    NAMES.get_or_init(Default::default)
}

/// Mirror the active document's SVG filter names for the menu tree (cheap when unchanged).
pub fn sync(app: &VectorcraftApp) {
    let names: Vec<String> = app.session.active().map(|st| st.doc.svg_filters.iter().map(|f| f.name.clone()).collect()).unwrap_or_default();
    let same = names_store().read().map(|v| v.len() == names.len() && v.iter().zip(&names).all(|(a, b)| *a == b)).unwrap_or(true);
    if !same {
        if let Ok(mut v) = names_store().write() {
            *v = names.iter().map(|n| crate::shortcut_editor::intern(n)).collect();
        }
        crate::shortcut_editor::GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Effect › SVG Filters: Apply SVG Filter…, Import SVG Filter… and then the active document's
/// filters, each applying itself to the selection.
pub fn menu_items() -> Vec<crate::menus::Item> {
    items_for(names_store().read().map(|v| v.clone()).unwrap_or_default())
}

/// [`menu_items`] for filters named `names`.
fn items_for(names: Vec<&'static str>) -> Vec<crate::menus::Item> {
    use crate::menus::Item;
    let mut v =
        vec![Item::Cmd("Apply SVG Filter…", "ui.svgFilterDialog", Value::Null), Item::Cmd("Import SVG Filter…", "ui.importSvgFilter", Value::Null)];
    if !names.is_empty() {
        v.push(Item::Sep);
        v.extend(names.into_iter().map(|n| Item::Cmd(n, "svgFilter.apply", json!({ "name": n }))));
    }
    v
}

/// `ui.importSvgFilter {}`: add the filters of an SVG file the user picks to the active document.
pub fn import_file(app: &mut VectorcraftApp) -> Result<Value, String> {
    let path = crate::picks::open(app, &pick()).ok_or("cancelled")?;
    let r = app.run("svgFilter.import", json!({ "path": path }))?;
    let added = r["added"].as_array().map_or(0, Vec::len);
    app.status(crate::i18n::msg_n(added as u64, "Imported {n} SVG filter", "Imported {n} SVG filters", &[]));
    Ok(r)
}

/// The extensions of the SVG files Import… reads.
const SVG_EXTS: &[&str] = &["svg"];

/// The open dialog for an SVG file.
pub(crate) fn pick() -> FilePick {
    FilePick { filters: vec![FilePick::filter("SVG", SVG_EXTS)], ..Default::default() }
}

/// Import…: the filters of the SVG file picked in the open dialog, added to the document, the first
/// one added selected.
fn import(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let Some(path) = crate::picks::open(app, &pick()) else { return Ok(()) };
    let r = app.run("svgFilter.import", json!({ "path": path }))?;
    if let Some(first) = r["added"].get(0).and_then(Value::as_str) {
        d.fields.insert("selected".into(), json!(first));
    }
    Ok(())
}

/// What a button asked for, done once the dialog is drawn.
enum Action {
    Select(String),
    New,
    /// Open this filter's code (a double-clicked row, or the selected one).
    Edit(String),
    Delete,
    Import,
    Save,
    Discard,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let list = filters(app);
    let selected = d.str("selected");
    let editing = d.str("editing");
    let mut act = None;
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("svg-filters").min_scrolled_height(160.0).max_height(160.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(160.0);
            // Filter names are data: shown as they are. A deleted filter the effect still names
            // comes first, dimmed and not selectable.
            let missing = d.str("missing");
            if !missing.is_empty() {
                ui.add_enabled_ui(false, |ui| ui.selectable_label(false, missing.as_str()));
            }
            for f in &list {
                let r = ui.selectable_label(f.name == selected, f.name.as_str());
                if r.double_clicked() && editing.is_empty() {
                    act = Some(Action::Edit(f.name.clone()));
                } else if r.clicked() {
                    act = Some(Action::Select(f.name.clone()));
                }
            }
        });
    });
    ui.add_space(6.0);
    let has = list.iter().any(|f| f.name == selected);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let idle = editing.is_empty();
        if ui.add_enabled_ui(idle && list.len() < SvgFilter::MAX, |ui| widgets::flat_button(ui, tl!("New"), 64.0)).inner.clicked() {
            act = Some(Action::New);
        }
        if ui.add_enabled_ui(idle && has, |ui| widgets::flat_button(ui, tl!("Edit"), 64.0)).inner.clicked() {
            act = Some(Action::Edit(selected.clone()));
        }
        if ui.add_enabled_ui(idle && has, |ui| widgets::flat_button(ui, tl!("Delete"), 64.0)).inner.clicked() {
            act = Some(Action::Delete);
        }
        if crate::picks::can(app, &PickRequest::Open(pick()))
            && ui.add_enabled_ui(idle, |ui| widgets::flat_button(ui, tl!("Import…"), 64.0)).inner.clicked()
        {
            act = Some(Action::Import);
        }
    });
    if !editing.is_empty() {
        ui.add_space(8.0);
        let mut code = d.str("code");
        // The filter's code is data: edited as it is written.
        let r = ui.add(egui::TextEdit::multiline(&mut code).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(10));
        if r.changed() {
            d.fields.insert("code".into(), json!(code));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.add_enabled_ui(!code.trim().is_empty(), |ui| widgets::flat_button(ui, tl!("Save"), 64.0)).inner.clicked() {
                act = Some(Action::Save);
            }
            if widgets::flat_button(ui, tl!("Cancel"), 64.0).clicked() {
                act = Some(Action::Discard);
            }
        });
    }
    let r = match act {
        None => Ok(()),
        Some(Action::Select(name)) => {
            d.fields.insert("selected".into(), json!(name));
            Ok(())
        }
        Some(Action::New) => {
            d.fields.insert("editing".into(), json!("new"));
            d.fields.insert("code".into(), json!(""));
            Ok(())
        }
        Some(Action::Edit(name)) => {
            if let Some(f) = list.iter().find(|f| f.name == name) {
                d.fields.insert("selected".into(), json!(f.name));
                d.fields.insert("editing".into(), json!(f.name));
                d.fields.insert("code".into(), json!(f.xml));
            }
            Ok(())
        }
        Some(Action::Delete) => app.run("svgFilter.delete", json!({ "name": selected })).map(|_| {
            d.fields.insert("selected".into(), json!(""));
        }),
        // The open dialog, shown off the UI thread, fills the dialog as it is then.
        Some(Action::Import) => crate::picks::in_dialog(app, d, import),
        Some(Action::Save) => {
            let mut p = json!({ "xml": d.str("code") });
            if editing != "new" {
                p["replace"] = json!(editing);
            }
            app.run("svgFilter.define", p).map(|r| {
                d.fields.insert("selected".into(), r["name"].clone());
                d.fields.insert("editing".into(), json!(""));
                d.fields.insert("code".into(), json!(""));
            })
        }
        Some(Action::Discard) => {
            d.fields.insert("editing".into(), json!(""));
            d.fields.insert("code".into(), json!(""));
            Ok(())
        }
    };
    if let Err(e) = r {
        app.status(e);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    const SHIFT: &str = r#"<filter id="Shift"><feOffset dx="3"/></filter>"#;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app
    }

    /// One headless frame of the dialog's body: the texts drawn.
    fn frame(app: &mut VectorcraftApp, d: &mut Dialog) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            body(app, ui, d);
        });
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    }

    #[test]
    fn lists_the_documents_filters_and_ok_applies_the_selected_one() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(open(&mut app, &json!({})).is_err(), "no document");
        let mut app = self::app();
        app.session.execute("svgFilter.define", &json!({"xml": SHIFT})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        crate::menus::invoke(&mut app, "ui.svgFilterDialog", json!({}));
        let mut d = app.ui.dialog.clone().unwrap();
        assert_eq!(d.kind, KIND);
        assert!(frame(&mut app, &mut d).iter().any(|t| t == "Shift"), "the filter's name, as it is");
        assert!(!(SPEC.ok_enabled.unwrap())(&app, &d), "nothing selected");
        d.fields.insert("selected".into(), json!("Shift"));
        assert!((SPEC.ok_enabled.unwrap())(&app, &d));
        confirm(&mut app, &d).unwrap();
        assert!(app.ui.dialog.is_none());
        let fx = &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects;
        assert_eq!((fx.len(), fx[0].params["name"].as_str()), (1, Some("Shift")));
    }

    #[test]
    fn opened_for_an_applied_filter_ok_replaces_it() {
        let mut app = app();
        app.session.execute("svgFilter.define", &json!({"xml": SHIFT})).unwrap();
        app.session.execute("svgFilter.define", &json!({"xml": "<filter id=\"Other\"><feOffset dy=\"2\"/></filter>"})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("svgFilter.apply", &json!({"name": "Shift"})).unwrap();
        // With a fill row active, the object's own effect is still the one edited.
        app.session.execute("appearance.setActiveItem", &json!({"index": 0})).unwrap();
        // The Appearance panel opens the effect's dialog: this one.
        app.run("effect.dialog", json!({"effect": "svg.filter", "index": 0, "item": null})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.kind.as_str(), d.str("selected").as_str()), (KIND, "Shift"));
        d.fields.insert("selected".into(), json!("Other"));
        confirm(&mut app, &d).unwrap();
        let fx = &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects;
        assert_eq!((fx.len(), fx[0].params["name"].as_str()), (1, Some("Other")), "in its place, not added");
        // Edited since it was applied: the effect follows the edit, and the filter stays selected.
        app.session.execute("svgFilter.define", &json!({"xml": "<filter id=\"Other\"><feOffset dy=\"9\"/></filter>", "replace": "Other"})).unwrap();
        app.run("effect.dialog", json!({"effect": "svg.filter", "index": 0, "item": null})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.str("selected").as_str(), d.str("editing").as_str()), ("Other", ""));
        // Deleted: its name shows above the list, nothing is selected, and OK waits for a choice.
        app.session.execute("svgFilter.delete", &json!({"name": "Other"})).unwrap();
        app.run("effect.dialog", json!({"effect": "svg.filter", "index": 0, "item": null})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.str("missing").as_str(), d.str("selected").as_str(), d.str("editing").as_str()), ("Other", "", ""));
        assert!(!(SPEC.ok_enabled.unwrap())(&app, &d));
        // An applied filter the document never held (from an opened file) opens as code to save.
        let e = vectorcraft_doc::Effect {
            id: "svg.filter".into(),
            params: json!({"name": "Kept", "xml": "<filter id=\"Kept\"><feOffset dx=\"1\"/></filter>"}),
            visible: true,
        };
        app.session
            .edit("Plant", |d, _| {
                d.node_mut(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects = vec![e];
                Ok(())
            })
            .unwrap();
        app.run("effect.dialog", json!({"effect": "svg.filter", "index": 0, "item": null})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!(d.str("editing"), "new");
        assert!(d.str("code").contains("id=\"Kept\""));
    }

    /// Edit, started from one object's effect in the Appearance panel, changes every object that uses
    /// the filter; a deleted filter's name is drawn above the list.
    #[test]
    fn an_edit_from_one_objects_effect_changes_every_user() {
        let mut app = app();
        app.session.execute("svgFilter.define", &json!({"xml": SHIFT})).unwrap();
        let rect = |app: &mut VectorcraftApp, x: f64| {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap()
        };
        let (a, b) = (rect(&mut app, 10.0), rect(&mut app, 50.0));
        app.session.execute("svgFilter.apply", &json!({"name": "Shift", "ids": [a, b]})).unwrap();
        app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
        app.run("effect.dialog", json!({"effect": "svg.filter", "index": 0, "item": null})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        // Edit opens the selected filter's code; Save replaces it, as its button does.
        d.fields.insert("editing".into(), json!(d.str("selected")));
        let edited = r#"<filter id="Shift"><feOffset dx="8"/></filter>"#;
        app.run("svgFilter.define", json!({"xml": edited, "replace": d.str("editing")})).unwrap();
        let xml = |app: &VectorcraftApp, id: u64| {
            app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["xml"].clone()
        };
        assert_eq!((xml(&app, a), xml(&app, b)), (json!(edited), json!(edited)));
        app.session.execute("svgFilter.delete", &json!({"name": "Shift"})).unwrap();
        app.run("effect.dialog", json!({"effect": "svg.filter", "index": 0, "item": null})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        assert!(frame(&mut app, &mut d).iter().any(|t| t == "Shift"), "the deleted filter's name");
    }

    #[test]
    fn the_submenu_lists_the_documents_filters_below_its_commands() {
        use crate::menus::Item;
        let ids =
            |items: &[Item]| items.iter().map(|i| if let Item::Cmd(l, c, p) = i { format!("{l} {c} {p}") } else { "—".into() }).collect::<Vec<_>>();
        assert_eq!(ids(&items_for(vec![])), ["Apply SVG Filter… ui.svgFilterDialog null", "Import SVG Filter… ui.importSvgFilter null"]);
        assert_eq!(
            ids(&items_for(vec!["Shift", "A (1)"]))[2..],
            ["—", "Shift svgFilter.apply {\"name\":\"Shift\"}", "A (1) svgFilter.apply {\"name\":\"A (1)\"}"]
        );
        let mut app = app();
        assert!(!crate::menus::enabled(&app, "svgFilter.apply"), "nothing selected");
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        assert!(crate::menus::enabled(&app, "svgFilter.apply"));
        assert_eq!(crate::menus::display_label(&app, "svgFilter.apply", "Shift"), "Shift", "a filter's name as it is");
    }

    #[test]
    fn new_saves_the_code_and_selects_the_filter() {
        let mut app = app();
        open(&mut app, &json!({})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("editing".into(), json!("new"));
        d.fields.insert("code".into(), json!(SHIFT));
        assert!(!(SPEC.ok_enabled.unwrap())(&app, &d), "not while the code is open");
        // Save, as its button does.
        let r = app.run("svgFilter.define", json!({"xml": d.str("code")})).unwrap();
        assert_eq!(r["name"], "Shift");
        assert_eq!(app.session.active().unwrap().doc.svg_filters.len(), 1);
    }

    #[test]
    fn import_adds_the_picked_files_filters() {
        let mut app = app();
        let path = std::env::temp_dir().join(format!("vc-ui-svgfilters-{}.svg", std::process::id()));
        std::fs::write(&path, format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><defs>{SHIFT}</defs></svg>")).unwrap();
        let picked = path.to_string_lossy().into_owned();
        app.services.pick_open = Some(Box::new(move |_: &FilePick| Some(picked.clone())));
        let mut d = Dialog::new(KIND, json!({ "selected": "", "code": "", "editing": "" }));
        import(&mut app, &mut d).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(d.str("selected"), "Shift");
        assert_eq!(app.session.active().unwrap().doc.svg_filters[0].name, "Shift");
    }
}
