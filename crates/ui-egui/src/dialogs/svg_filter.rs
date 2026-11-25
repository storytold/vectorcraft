//! Apply SVG Filter (Effect › SVG Filters › Apply SVG Filter…): the default SVG filters and the
//! document's own in a list, the defaults first. OK applies the selected one to the selection as a
//! live effect, or, opened from the Appearance panel for an applied SVG filter, puts it in that
//! effect's place (`svgFilter.apply`, with `index`). New and Edit open the filter's code under the
//! list; Save adds the filter or replaces the one being edited (`svgFilter.define`). Delete
//! removes the selected filter (`svgFilter.delete`) and Import… adds the filters of an SVG file
//! (`svgFilter.import`). Those act at once. Effect › SVG Filters lists the same filters below its
//! first items.
//!
//! A default is not in the document until it is applied or edited: the document then holds a copy
//! under the default's name, which is listed in the default's place. Delete is off for a default
//! the document holds no copy of.
//!
//! Fields: `selected` (a filter's name), `code` (the code being edited), `editing` (`""`, `"new"`
//! or the name of the filter being edited), `__action` (the button pressed, run once by [`pump`]),
//! `__list` (the filters listed, pushed by [`pump`]), and with an applied effect `__index`,
//! `__item` and `missing` (the name of a deleted filter the effect names).

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::host::{PickRequest, Picked};
use craft::ui::buttons::Button;
use craft::ui::contract::{ActivateOn, Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::dialog_id;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, RowInk};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use serde_json::{Value, json};
use vectorcraft_effects::svg_filter::defaults::{DEFAULT_FILTERS, Listed, default_filter, listed};
use vectorcraft_engine::doc::SvgFilter;

use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};
use crate::menus::Node;

/// The dialog kind of Apply SVG Filter.
pub const KIND: &str = "svgFilter";

const LIST: &str = "list";
const CODE: &str = "code";
const SELECTED: &str = "selected";
const EDITING: &str = "editing";
const MISSING: &str = "missing";
const ACTION: &str = "__action";
const LISTING: &str = "__list";

/// The dialog's heading.
fn heading() -> String {
    tl!("Apply SVG Filter…").trim_end_matches('…').to_string()
}

/// The filters the dialog lists for the active document, and how many come first as the defaults.
fn filters(app: &VectorcraftApp) -> (Vec<Listed>, usize) {
    listed(app.session.active().map(|st| st.doc.svg_filters.as_slice()).unwrap_or_default())
}

/// The listing the dialog keeps in `__list`: each filter's name, whether it is a default the
/// document holds no copy of, and how many come first as the defaults.
fn listing(app: &VectorcraftApp) -> Value {
    let (list, defaults) = filters(app);
    let filters: Vec<Value> = list.iter().map(|f| json!({ "name": f.filter.name, "default": f.default })).collect();
    json!({ "filters": filters, "defaults": defaults })
}

/// The text under `key`, whether the dialog or automation set it.
fn text(fields: &DialogFields, key: &str) -> String {
    match fields.get(key) {
        Some(FieldValue::Text(s)) => s.clone(),
        Some(FieldValue::Json(Value::String(s))) => s.clone(),
        _ => String::new(),
    }
}

/// The value under `key` as JSON, for the keys that are not text.
fn json_field(fields: &DialogFields, key: &str) -> Option<Value> {
    match fields.get(key) {
        Some(FieldValue::Json(v)) => Some(v.clone()),
        _ => None,
    }
}

/// A field value for `v`: text stays text, anything else is kept as JSON.
fn field_value(v: Value) -> FieldValue {
    match v {
        Value::String(s) => FieldValue::Text(s),
        v => FieldValue::Json(v),
    }
}

/// The filters of `__list`: `(name, default)` each, and how many come first as the defaults.
fn listed_entries(fields: &DialogFields) -> (Vec<(String, bool)>, usize) {
    let Some(l) = json_field(fields, LISTING) else { return (Vec::new(), 0) };
    let entries = l["filters"]
        .as_array()
        .map(|a| a.iter().map(|f| (f["name"].as_str().unwrap_or_default().to_string(), f["default"].as_bool().unwrap_or(false))).collect())
        .unwrap_or_default();
    (entries, l["defaults"].as_u64().unwrap_or(0) as usize)
}

/// What a row of the list is.
enum Kind {
    /// The name of a deleted filter the effect names: dimmed, not selectable.
    Missing,
    /// The line between the defaults and the document's own filters.
    Section,
    Filter,
}

/// The rows of the list in display order, keyed by the filter's name.
struct Rows(Vec<(String, Kind)>);

impl Rows {
    fn of(fields: &DialogFields) -> Rows {
        let (entries, defaults) = listed_entries(fields);
        let missing = text(fields, MISSING);
        let mut rows = Vec::new();
        if !missing.is_empty() {
            rows.push((missing, Kind::Missing));
        }
        for (i, (name, _)) in entries.into_iter().enumerate() {
            // The document's other filters follow the defaults, below a line.
            if i == defaults && i > 0 {
                rows.push((String::new(), Kind::Section));
            }
            rows.push((name, Kind::Filter));
        }
        Rows(rows)
    }
}

impl DisplayOrder<String> for Rows {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn key(&self, at: usize) -> String {
        self.0.get(at).map(|r| r.0.clone()).unwrap_or_default()
    }
    fn is_section(&self, at: usize) -> bool {
        matches!(self.0.get(at), Some((_, Kind::Section)))
    }
    fn enabled(&self, at: usize) -> bool {
        matches!(self.0.get(at), Some((_, Kind::Filter)))
    }
}

impl ItemSource<String> for Rows {}

/// The button `label`, enabled or not; true when pressed.
fn pressed(ui: &mut egui::Ui, label: &'static str, enabled: bool) -> bool {
    Button::new(label).enabled(enabled).show(ui).clicks > 0
}

/// The list and its buttons. A button sets `__action` for [`pump`], which has the app.
fn list_row(ui: &mut egui::Ui, fields: &mut DialogFields, _cx: &mut craft::ui::patterns::form_dialogs::CustomContext) -> egui::Response {
    let rows = Rows::of(fields);
    let (entries, _) = listed_entries(fields);
    let selected = text(fields, SELECTED);
    let editing = text(fields, EDITING);
    let idle = editing.is_empty();
    let mut selection = if selected.is_empty() {
        Selection::new(vec![], None, None)
    } else {
        Selection::new(vec![selected.clone()], Some(selected.clone()), Some(selected.clone()))
    };
    let mut action = None;
    let mut edit = None;
    let shown = Stack::new().show(ui, |ui, _| {
        let out = List::<String>::new(heading())
            .select(SelectMode::Single)
            .activate(ActivateOn::DoubleClick)
            .selection(Bind::bound(&mut selection))
            .show_items(ui, &rows, &mut |key, icx| match rows.0.get(icx.at) {
                Some((_, Kind::Missing)) => ListRow::new(key.clone()).ink(RowInk::Dimmed),
                Some((_, Kind::Section)) => ListRow::new(Label::Empty),
                _ => ListRow::new(key.clone()),
            });
        for event in out.events {
            // A double click edits, only while no code is open.
            if let CollectionEvent::Activate(key) = event
                && idle
            {
                edit = Some(key);
            }
        }
        let chosen = entries.iter().find(|(name, _)| *name == selected);
        let held = entries.iter().filter(|(_, default)| !default).count();
        Row::new().show(ui, |ui, _| {
            if pressed(ui, tl!("New"), idle && held < SvgFilter::MAX) {
                action = Some("new");
            }
            if pressed(ui, tl!("Edit"), idle && chosen.is_some()) {
                action = Some("edit");
            }
            // A default the document holds no copy of has nothing to delete.
            if pressed(ui, tl!("Delete"), idle && chosen.is_some_and(|(_, default)| !default)) {
                action = Some("delete");
            }
            if pressed(ui, tl!("Import…"), idle) {
                action = Some("import");
            }
        });
    });
    let now = selection.items.first().cloned().unwrap_or_default();
    if let Some(name) = edit {
        fields.set(SELECTED, name);
        action = Some("edit");
    } else if now != selected {
        fields.set(SELECTED, now);
    }
    if let Some(a) = action {
        fields.set(ACTION, a);
    }
    shown.output
}

/// The code box with Save and Cancel, shown while a filter's code is open.
fn code_row(ui: &mut egui::Ui, fields: &mut DialogFields, _cx: &mut craft::ui::patterns::form_dialogs::CustomContext) -> egui::Response {
    if text(fields, EDITING).is_empty() {
        return ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover());
    }
    let mut code = text(fields, CODE);
    let before = code.clone();
    let mut action = None;
    let shown = Stack::new().show(ui, |ui, _| {
        // The filter's code is data: edited as it is written.
        TextField::new(tl!("Code"), Bind::bound(&mut code)).mono().lines(10).tab_inserts(true).show(ui);
        Row::new().show(ui, |ui, _| {
            if pressed(ui, tl!("Save"), !before.trim().is_empty()) {
                action = Some("save");
            }
            if pressed(ui, tl!("Cancel"), true) {
                action = Some("discard");
            }
        });
    });
    if code != before {
        fields.set(CODE, code);
    }
    if let Some(a) = action {
        fields.set(ACTION, a);
    }
    shown.output
}

/// `ui.svgFilterDialog {index?, item?}`: open Apply SVG Filter for the active document; with
/// `index`, for the applied SVG filter at `index` of `item` (null: the object's effects): the
/// document's filter of its name selected, or the default of its name when the effect draws with
/// the default's code or with nothing; when neither has its name, the name shown above the list if
/// the filter was deleted, or else its code open as a new filter.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document open".into());
    }
    let mut fields = json!({ "selected": "", "code": "", "editing": "", "__action": "" });
    if let Some(k) = p.get("index").and_then(Value::as_u64) {
        let mut q = json!({});
        if let Some(item) = p.get("item") {
            q["item"] = item.clone();
        }
        let applied = app.session.appearance_effect_target(&q).map(|(_, fx)| fx.to_vec()).unwrap_or_default();
        let e = applied.get(k as usize).filter(|e| e.id == vectorcraft_effects::SVG_FILTER).ok_or_else(|| format!("no SVG filter at index {k}"))?;
        let name = e.params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let xml = e.params.get("xml").and_then(Value::as_str).unwrap_or_default().to_string();
        let held = app.session.active().is_some_and(|st| st.doc.svg_filters.iter().any(|f| f.name == name));
        // A default the document holds no copy of, when the effect draws with its code or with nothing.
        let default = default_filter(&name).is_some_and(|d| xml.is_empty() || xml == d.xml);
        if held || default {
            // The filter it names: selected.
            fields["selected"] = json!(name);
        } else if xml.is_empty() {
            // Deleted from the document: shown above the list, not selectable; OK waits for a choice.
            fields["missing"] = json!(name);
        } else {
            // A filter the document doesn't hold (from an opened file): its code, to add it.
            fields["code"] = json!(xml);
            fields["editing"] = json!("new");
        }
        fields["__index"] = json!(k);
        // As given: `null` is the object's own effects, and none is the active item's.
        if let Some(item) = p.get("item") {
            fields["__item"] = item.clone();
        }
    }
    fields[LISTING] = listing(app);
    let pairs = fields.as_object().into_iter().flatten().map(|(k, v)| (k.clone(), field_value(v.clone())));
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let dialog = HostedParamDialog::new(&ctx, KIND, DialogFields::from_pairs(pairs), build);
    crate::dialogs::host::open_held(app, dialog, |app, result: &ParamResult| confirm(app, &result.fields)).ok_or("no window")?;
    Ok(Value::Null)
}

/// The dialog over its values.
fn build(fields: &mut DialogFields) -> ParamDialog<'_> {
    let (entries, _) = listed_entries(fields);
    let selected = text(fields, SELECTED);
    let ok = text(fields, EDITING).is_empty() && entries.iter().any(|(name, _)| *name == selected);
    let params = vec![
        Param::custom(LIST, Label::Empty, list_row).companions([SELECTED, EDITING, MISSING, ACTION, LISTING]),
        Param::custom(CODE, Label::Empty, code_row),
    ];
    ParamDialog::new(KIND, heading(), params).values(Bind::bound(fields)).ok_enabled(ok).ok_reason(Label::from(tl!("Select an SVG filter")))
}

/// OK: the selected filter to the selection, or in the applied effect's place. A default the
/// document holds no copy of is copied into it in the same undo step.
fn confirm(app: &mut VectorcraftApp, fields: &DialogFields) -> Result<(), String> {
    let name = text(fields, SELECTED);
    let f = filters(app).0.into_iter().find(|f| f.filter.name == name).ok_or("select an SVG filter")?;
    let mut p = json!({ "name": f.filter.name });
    if let Some(k) = json_field(fields, "__index").and_then(|v| v.as_u64()) {
        p["index"] = json!(k);
        if let Some(item) = json_field(fields, "__item") {
            p["item"] = item;
        }
    }
    app.run("svgFilter.apply", p).map(|_| ())
}

/// Runs `f` on the open dialog; `None` when it is not open.
fn with_dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut HostedParamDialog) -> R) -> Option<R> {
    app.dialogs()?.with(|h| h.get_mut_by_id::<HostedParamDialog>(dialog_id(KIND)).map(f))
}

/// Sets `fields` on the open dialog.
fn set(app: &mut VectorcraftApp, fields: &[(&str, Value)]) {
    with_dialog(app, |d| {
        for (k, v) in fields {
            d.set_field(*k, field_value(v.clone()));
        }
    });
}

/// Each frame: pushes the listed filters into the dialog (the build closure has no app) and runs the
/// button pressed, once.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let listing = listing(app);
    let taken = with_dialog(app, |d| {
        if json_field(d.fields(), LISTING).as_ref() != Some(&listing) {
            d.set_field(LISTING, FieldValue::Json(listing));
        }
        let action = text(d.fields(), ACTION);
        if !action.is_empty() {
            d.set_field(ACTION, FieldValue::Text(String::new()));
        }
        (action, text(d.fields(), SELECTED), text(d.fields(), EDITING), text(d.fields(), CODE))
    });
    let Some((action, selected, editing, code)) = taken.filter(|(action, ..)| !action.is_empty()) else { return };
    let result: Result<(), String> = match action.as_str() {
        "new" => {
            set(app, &[(EDITING, json!("new")), (CODE, json!(""))]);
            Ok(())
        }
        "edit" => {
            if let Some(f) = filters(app).0.iter().map(|f| &f.filter).find(|f| f.name == selected) {
                let (name, xml) = (f.name.clone(), f.xml.clone());
                set(app, &[(SELECTED, json!(name)), (EDITING, json!(name)), (CODE, json!(xml))]);
            }
            Ok(())
        }
        "delete" => app.run("svgFilter.delete", json!({ "name": selected })).map(|_| set(app, &[(SELECTED, json!(""))])),
        "save" => {
            let mut p = json!({ "xml": code });
            if editing != "new" {
                p["replace"] = json!(editing);
            }
            app.run("svgFilter.define", p).map(|r| set(app, &[(SELECTED, r["name"].clone()), (EDITING, json!("")), (CODE, json!(""))]))
        }
        "discard" => {
            set(app, &[(EDITING, json!("")), (CODE, json!(""))]);
            Ok(())
        }
        "import" => {
            ask_import(app);
            Ok(())
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        app.status(e);
    }
}

/// The names of `held`, a document's filters, that are not a default's, in their order.
pub(crate) fn own_names(held: &[SvgFilter]) -> Vec<String> {
    held.iter().filter(|f| default_filter(&f.name).is_none()).map(|f| f.name.clone()).collect()
}

/// Effect › SVG Filters: Apply SVG Filter…, Import SVG Filter…, the default filters and then the
/// document's other filters (`names`, from [`own_names`]), each applying itself to the selection.
pub(crate) fn menu_items(names: &[String]) -> Vec<Node> {
    use crate::menus::{cp, named};
    let apply = |n: &str| named(n, "svgFilter.apply", json!({ "name": n }));
    let mut v = vec![cp("Apply SVG Filter…", "ui.svgFilterDialog", Value::Null), cp("Import SVG Filter…", "ui.importSvgFilter", Value::Null)];
    v.push(Node::Separator);
    v.extend(DEFAULT_FILTERS.iter().map(|d| apply(d.name)));
    if !names.is_empty() {
        v.push(Node::Separator);
        v.extend(names.iter().map(|n| apply(n)));
    }
    v
}

/// `ui.importSvgFilter {}`: add the filters of an SVG file the user picks to the active document.
/// The answer is `{pending: "fileDialog"}`; the continuation runs the import.
pub fn import_file(app: &mut VectorcraftApp) -> Result<Value, String> {
    Ok(ask_import(app))
}

/// The extensions of the SVG files Import… reads.
const SVG_EXTS: &[&str] = &["svg"];

/// The open dialog for an SVG file.
pub(crate) fn pick() -> PickRequest {
    PickRequest::open("svg-filter").filter("SVG", SVG_EXTS)
}

/// Asks for an SVG file; once picked, its filters are added to the document and the first one
/// added is selected in the dialog when it is open.
fn ask_import(app: &mut VectorcraftApp) -> Value {
    file_asks::ask(app, pick(), Owner::Document, |app, picked| {
        let params = match picked {
            // The browser answers with the file's contents, not a path.
            Picked::Contents(files) => {
                let (_, bytes) = files.first().ok_or("cancelled")?;
                json!({ "svg": String::from_utf8_lossy(bytes) })
            }
            other => json!({ "path": other.first_path().ok_or("cancelled")?.to_string_lossy() }),
        };
        let r = app.run("svgFilter.import", params)?;
        let added = r["added"].as_array().map_or(0, Vec::len);
        app.status(crate::i18n::msg_n(added as u64, "Imported {n} SVG filter", "Imported {n} SVG filters", &[]));
        if let Some(first) = r["added"].get(0) {
            set(app, &[(SELECTED, first.clone())]);
        }
        Ok(r)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::picks::{answer_dialog, open_dialogs, poll, scripted};
    use vectorcraft_engine::Session;

    const SHIFT: &str = r#"<filter id="Shift"><feOffset dx="3"/></filter>"#;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app
    }

    fn rect(app: &mut VectorcraftApp) -> u64 {
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap()
    }

    /// The fields of the open dialog, as automation lists them.
    fn fields(app: &VectorcraftApp) -> Value {
        open_dialogs(app).last().and_then(|d| d.fields.clone()).unwrap_or(Value::Null)
    }

    fn effects(app: &VectorcraftApp, id: u64) -> Vec<vectorcraft_doc::Effect> {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects.clone()
    }

    #[test]
    fn lists_the_documents_filters_and_ok_applies_the_selected_one() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(open(&mut app, &json!({})).is_err(), "no document");
        let mut app = self::app();
        app.session.execute("svgFilter.define", &json!({"xml": SHIFT})).unwrap();
        let id = rect(&mut app);
        crate::menus::invoke(&mut app, "ui.svgFilterDialog", json!({}));
        let dialog = dialog_id(KIND);
        let names: Vec<String> =
            fields(&app)[LISTING]["filters"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(
            (names.len(), names.last().map(String::as_str)),
            (DEFAULT_FILTERS.len() + 1, Some("Shift")),
            "the document's own after the defaults"
        );
        // Nothing selected: OK is off and the dialog stays.
        answer_dialog(&mut app, dialog, &[], true);
        assert!(effects(&app, id).is_empty());
        assert_eq!(open_dialogs(&app).len(), 1);
        answer_dialog(&mut app, dialog, &[(SELECTED, json!("Shift"))], true);
        let fx = effects(&app, id);
        assert_eq!((fx.len(), fx[0].params["name"].as_str()), (1, Some("Shift")));
        assert!(open_dialogs(&app).is_empty());
    }

    #[test]
    fn opened_for_an_applied_filter_ok_replaces_it() {
        let mut app = app();
        app.session.execute("svgFilter.define", &json!({"xml": SHIFT})).unwrap();
        app.session.execute("svgFilter.define", &json!({"xml": "<filter id=\"Other\"><feOffset dy=\"2\"/></filter>"})).unwrap();
        let id = rect(&mut app);
        app.session.execute("svgFilter.apply", &json!({"name": "Shift"})).unwrap();
        app.session.execute("appearance.setActiveItem", &json!({"index": 0})).unwrap();
        let dialog = dialog_id(KIND);
        let reopen = |app: &mut VectorcraftApp| app.run("effect.dialog", json!({"effect": "svg.filter", "index": 0, "item": null})).unwrap();
        // A held filter is selected; OK puts the other in its place.
        reopen(&mut app);
        assert_eq!(fields(&app)[SELECTED], "Shift");
        answer_dialog(&mut app, dialog, &[(SELECTED, json!("Other"))], true);
        let fx = effects(&app, id);
        assert_eq!((fx.len(), fx[0].params["name"].as_str()), (1, Some("Other")), "in its place, not added");
        // A default the document holds no copy of is selected.
        // The fill row stays active: `item: null` names the object's own effect, as the dialog does.
        app.session.execute("svgFilter.apply", &json!({"name": "Thin", "index": 0, "item": null})).unwrap();
        app.session.execute("svgFilter.delete", &json!({"name": "Thin"})).unwrap();
        reopen(&mut app);
        assert_eq!((fields(&app)[SELECTED].as_str(), fields(&app)[EDITING].as_str()), (Some("Thin"), Some("")));
        answer_dialog(&mut app, dialog, &[(SELECTED, json!("Other"))], false);
        // Deleted: shown as missing, nothing selected, OK off.
        app.session.execute("svgFilter.delete", &json!({"name": "Other"})).unwrap();
        app.session.execute("svgFilter.apply", &json!({"name": "Other", "index": 0, "item": null, "xml": ""})).ok();
        let gone = vectorcraft_doc::Effect { id: "svg.filter".into(), params: json!({"name": "Gone"}), visible: true };
        app.session
            .edit("Plant", |d, _| {
                d.node_mut(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects = vec![gone];
                Ok(())
            })
            .unwrap();
        reopen(&mut app);
        assert_eq!((fields(&app)[MISSING].as_str(), fields(&app)[SELECTED].as_str()), (Some("Gone"), Some("")));
        answer_dialog(&mut app, dialog, &[], true);
        assert_eq!(open_dialogs(&app).len(), 1, "OK waits for a choice");
        // An applied filter the document never held opens as code to save.
        let kept = vectorcraft_doc::Effect {
            id: "svg.filter".into(),
            params: json!({"name": "Kept", "xml": "<filter id=\"Kept\"><feOffset dx=\"1\"/></filter>"}),
            visible: true,
        };
        app.session
            .edit("Plant", |d, _| {
                d.node_mut(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects = vec![kept];
                Ok(())
            })
            .unwrap();
        reopen(&mut app);
        assert_eq!(fields(&app)[EDITING], "new");
        assert!(fields(&app)[CODE].as_str().unwrap().contains("id=\"Kept\""));
    }

    #[test]
    fn the_submenu_lists_the_documents_filters_below_its_commands() {
        let plain = menu_items(&[]);
        assert_eq!(plain.len(), 3 + DEFAULT_FILTERS.len());
        assert_eq!(plain[2], Node::Separator);
        let own = vec!["Shift".to_string(), "A (1)".to_string()];
        let with = menu_items(&own);
        assert_eq!(with[..plain.len()], plain[..]);
        assert_eq!(with[plain.len()], Node::Separator);
        assert_eq!(with.len(), plain.len() + 1 + own.len());
        // A document's filter with a default's name is that default's entry.
        let held = [
            SvgFilter { name: "Glow".into(), xml: String::new() },
            SvgFilter { name: "Shift".into(), xml: String::new() },
            SvgFilter { name: "glow".into(), xml: String::new() },
        ];
        assert_eq!(own_names(&held), ["Shift", "glow"]);
    }

    #[test]
    fn new_saves_the_code_and_selects_the_filter() {
        let mut app = app();
        open(&mut app, &json!({})).unwrap();
        let dialog = dialog_id(KIND);
        answer_dialog(&mut app, dialog, &[(ACTION, json!("new"))], false);
        assert_eq!(fields(&app)[EDITING], "new");
        // OK is off while the code is open.
        answer_dialog(&mut app, dialog, &[(SELECTED, json!("Soft Blur")), (CODE, json!(SHIFT))], true);
        assert_eq!(open_dialogs(&app).len(), 1);
        answer_dialog(&mut app, dialog, &[(ACTION, json!("save"))], false);
        assert_eq!(app.session.active().unwrap().doc.svg_filters.len(), 1);
        assert_eq!((fields(&app)[SELECTED].as_str(), fields(&app)[EDITING].as_str()), (Some("Shift"), Some("")));
        // Cancel discards.
        answer_dialog(&mut app, dialog, &[(ACTION, json!("new")), (CODE, json!("<filter/>"))], false);
        answer_dialog(&mut app, dialog, &[(ACTION, json!("discard"))], false);
        assert_eq!((fields(&app)[EDITING].as_str(), fields(&app)[CODE].as_str()), (Some(""), Some("")));
        assert_eq!(app.session.active().unwrap().doc.svg_filters.len(), 1);
    }

    #[test]
    fn import_adds_the_picked_files_filters() {
        let mut app = app();
        let picker = scripted(&mut app);
        let ctx = app.ui_ctx.clone().unwrap();
        let path = std::env::temp_dir().join(format!("vc-ui-svgfilters-{}.svg", std::process::id()));
        std::fs::write(&path, format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><defs>{SHIFT}</defs></svg>")).unwrap();
        open(&mut app, &json!({})).unwrap();
        let dialog = dialog_id(KIND);
        // A cancelled pick adds nothing.
        answer_dialog(&mut app, dialog, &[(ACTION, json!("import"))], false);
        assert_eq!(picker.requests()[0].filters[0].extensions, ["svg"]);
        picker.cancel();
        poll(&mut app, &ctx);
        assert!(app.session.active().unwrap().doc.svg_filters.is_empty());
        // Through the command, as the menu runs it.
        let r = import_file(&mut app).unwrap();
        assert_eq!(r["pending"], "fileDialog");
        picker.answer(&[path.to_str().unwrap()]);
        poll(&mut app, &ctx);
        std::fs::remove_file(&path).ok();
        assert_eq!(app.session.active().unwrap().doc.svg_filters[0].name, "Shift");
        assert_eq!(fields(&app)[SELECTED], "Shift");
    }
}
