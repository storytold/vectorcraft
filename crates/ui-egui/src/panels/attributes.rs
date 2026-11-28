//! Attributes panel: Overprint Fill / Overprint Stroke, Show / Don't Show Center, Reverse Path
//! Direction Off / On, the fill rule (non-zero winding or even-odd), Image Map, URL (with the
//! recent URLs and a Browser button) and the note. The menu hides the options and shows the note,
//! and its Panel Options… sets how many recent URLs the URL menu lists ([`PanelOptions`]).
//! Everything reads `attributes.info` ([`vectorcraft_engine::Session::attributes_info`]) and acts
//! through `attributes.set`, `path.reverse` and `path.setFillRule`.

use craft::behavior::{IconId, Mixed, Value as Valued};
use craft::keymap::CommandRef;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::PanelRow;
use craft::ui::pickers::{ComboBox, Select};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::text::TextField;
use egui::Ui;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{ImageMap, NodeId, NodeKind};
use vectorcraft_engine::cmd::{AttributesInfo, RECENT_URLS};
use vectorcraft_geom::FillRule;

use super::{part_hidden, selection_len, set_part_hidden};
use crate::VectorcraftApp;

/// The part store's key for the options (centre, direction, fill rule, image map, URL).
const OPTIONS_KEY: &str = "attr-hide-options";
/// The part store's key for the note field.
const NOTE_KEY: &str = "attr-hide-note";
/// The UI command that opens Panel Options.
const PANEL_OPTIONS: &str = "ui.attributesPanelOptions";

/// How many recent URLs the URL menu lists by default (VectorCraft's own value).
pub const DEFAULT_RECENT_URLS: u8 = 10;

/// Attributes panel options (≡ › Panel Options…), kept with the UI state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PanelOptions {
    /// How many of the recent URLs the URL menu lists, 1 to [`RECENT_URLS`]. A saved value that is
    /// not a whole number in that range reads as [`DEFAULT_RECENT_URLS`].
    #[serde(deserialize_with = "saved_count")]
    pub recent_urls: u8,
}

impl Default for PanelOptions {
    fn default() -> Self {
        Self { recent_urls: DEFAULT_RECENT_URLS }
    }
}

impl PanelOptions {
    /// How many recent URLs the URL menu lists: [`Self::recent_urls`], or the default when it is
    /// out of range.
    pub fn shown(&self) -> usize {
        Some(usize::from(self.recent_urls)).filter(|n| (1..=RECENT_URLS).contains(n)).unwrap_or(usize::from(DEFAULT_RECENT_URLS))
    }
}

/// A saved count of recent URLs: a whole number from 1 to [`RECENT_URLS`], anything else the default.
fn saved_count<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(count(&v).unwrap_or(DEFAULT_RECENT_URLS))
}

/// `v` as a count of recent URLs, when it is a whole number from 1 to [`RECENT_URLS`].
fn count(v: &Value) -> Option<u8> {
    let n = v.as_f64().filter(|n| n.fract() == 0.0 && (1.0..=RECENT_URLS as f64).contains(n))?;
    u8::try_from(n as u64).ok()
}

/// `ui.attributesPanelOptions {count?}`: without `count`, open Panel Options on the panel's
/// options; otherwise set how many recent URLs the URL menu lists and answer the options. A count
/// that is not a whole number from 1 to 30 is an error, and nothing changes.
pub fn set_options(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let Some(c) = p.get("count") else {
        return crate::dialogs::attributes_panel_options::open(app);
    };
    let n = count(c).ok_or("`count` must be a whole number from 1 to 30")?;
    app.ui.attributes_panel.recent_urls = n;
    Ok(json!({ "count": n }))
}

/// The recent URLs the URL menu lists, newest first: as many as Panel Options sets.
fn menu_urls(app: &VectorcraftApp) -> &[String] {
    let urls = &app.session.recent_urls;
    &urls[..urls.len().min(app.ui.attributes_panel.shown())]
}

/// The key the memo of [`info`] is valid for: the document and revision, and the selection's size
/// and ends.
pub(crate) type AttrKey = (u64, u64, usize, Option<NodeId>, Option<NodeId>);

/// The panel's values and whether the targets hold paths (direction and fill rule apply),
/// recomputed only when the document or the selection changes.
fn info(app: &mut VectorcraftApp) -> (AttributesInfo, bool) {
    let Some(st) = app.session.active() else { return Default::default() };
    let sel = &st.selection.objects;
    let key: AttrKey = (st.uid, st.revision, sel.len(), sel.first().copied(), sel.last().copied());
    if let Some((k, v)) = &app.canvas.attributes_info
        && *k == key
    {
        return v.clone();
    }
    let info = app.session.attributes_info();
    let mut paths = false;
    for n in info.ids.iter().filter_map(|id| st.doc.node(*id)) {
        n.walk(&mut |c| paths |= matches!(c.kind, NodeKind::Path { guide: false, .. }));
    }
    let v = (info, paths);
    app.canvas.attributes_info = Some((key, v.clone()));
    v
}

/// A value the targets share, or mixed where they differ (`None`).
fn or_mixed<T>(v: Option<T>) -> Mixed<T> {
    v.map_or(Mixed::Mixed, Mixed::Same)
}

/// The name a framework label shows for a catalog string written with a trailing colon: the
/// framework draws the colon itself.
pub(crate) fn side_name(label: &str) -> String {
    label.trim_end().trim_end_matches([':', '：']).trim_end().to_owned()
}

/// A pair of icon segments choosing one of two values (`current`: the targets' value, mixed where
/// they differ, which picks none); returns the value picked. Each item is the icon, its tip and
/// the value.
fn pair<T: Valued>(ui: &mut Ui, name: &'static str, enabled: bool, current: Option<T>, items: [(&'static str, &'static str, T); 2]) -> Option<T> {
    let entries = items.map(|(icon, tip, v)| Entry::new(v, tip).icon(IconId::new(icon)));
    Segmented::new(name, Bind::given(or_mixed(current)), &entries).enabled(enabled).show(ui).events.committed().cloned()
}

/// The panel: the rows only read; their edits are collected and run once the rows are drawn.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (info, paths) = info(app);
    let has = !info.ids.is_empty();
    // (command, params) for each edit of this pass; errors show in the status bar through `run`.
    let mut acts: Vec<(&'static str, Value)> = Vec::new();
    Row::new().show(ui, |ui, _| {
        for (label, key, value) in
            [(tl!("Overprint Fill"), "overprintFill", info.overprint_fill), (tl!("Overprint Stroke"), "overprintStroke", info.overprint_stroke)]
        {
            // A click on a mixed or unchecked box turns it on; on a checked box, off.
            let edit = Checkbox::new(Bind::given(or_mixed(value))).label(label).enabled(has).show(ui);
            if let Some(on) = edit.events.committed() {
                acts.push(("attributes.set", json!({ key: *on })));
            }
        }
    });
    if !part_hidden(app, OPTIONS_KEY) {
        Divider::horizontal().show(ui);
        Row::new().wrap(true).show(ui, |ui, _| {
            let center = [("dc-center-hide", tl!("Don't Show Center"), false), ("dc-center-show", tl!("Show Center"), true)];
            acts.extend(pair(ui, tl!("Show Center"), has, info.show_center, center).map(|v| ("attributes.set", json!({ "showCenter": v }))));
            let dir = [("dc-dir-off", tl!("Reverse Path Direction Off"), false), ("dc-dir-on", tl!("Reverse Path Direction On"), true)];
            acts.extend(pair(ui, tl!("Reverse Path Direction"), paths, info.reversed, dir).map(|v| ("path.reverse", json!({ "reversed": v }))));
            let rules = [
                ("dc-rule-nonzero", tl!("Use Non-Zero Winding Fill Rule"), FillRule::NonZero),
                ("dc-rule-evenodd", tl!("Use Even-Odd Fill Rule"), FillRule::EvenOdd),
            ];
            let rule = pair(ui, tl!("Fill Rule"), paths, info.fill_rule, rules);
            acts.extend(rule.map(|r| ("path.setFillRule", json!({ "rule": if r == FillRule::EvenOdd { "evenOdd" } else { "nonZero" } }))));
        });
        let url = info.url.clone().filter(|u| !u.is_empty());
        let maps = ImageMap::ALL.map(|m| Entry::new(m, m.label()));
        let name = side_name(tl!("Image Map:"));
        Field::new(name.clone()).show(ui, |ui, _| {
            Row::new().wrap(true).show(ui, |ui, _| {
                let pick = Select::new(name.clone(), Bind::given(or_mixed(info.image_map)), &maps).enabled(has).show(ui);
                acts.extend(pick.events.committed().map(|m| ("attributes.set", json!({ "imageMap": m }))));
                let browser = Button::new(tl!("Browser")).enabled(url.is_some()).tooltip(tl!("Open the URL in the web browser")).show(ui);
                if browser.clicked() {
                    acts.push(("attributes.openUrl", json!({})));
                }
            });
        });
        // With no recent URLs the options have nothing to list and the chevron does nothing.
        let recent: Vec<Entry<String>> = menu_urls(app).iter().map(|u| Entry::new(u.clone(), u.clone())).collect();
        let name = side_name(tl!("URL:"));
        Field::new(name.clone()).show(ui, |ui, _| {
            let edit = ComboBox::new(name.clone(), Bind::given(or_mixed(info.url.clone())), &recent).enabled(has).show(ui);
            acts.extend(edit.edit.events.committed().map(|u| ("attributes.set", json!({ "url": u }))));
        });
    }
    if !part_hidden(app, NOTE_KEY) {
        Divider::horizontal().show(ui);
        let name = side_name(tl!("Note:"));
        Field::new(name.clone()).layout(FieldLayout::Above).show(ui, |ui, _| {
            let edit = TextField::new(name.clone(), Bind::given(or_mixed(info.note.clone()))).lines(3).enabled(has).show(ui);
            acts.extend(edit.events.committed().map(|n| ("attributes.set", json!({ "note": n }))));
        });
    }
    if !has && selection_len(app) == 0 {
        Text::new(tl!("No Selection")).shade(Shade::Muted).show(ui);
    }
    for (id, params) in acts {
        app.run(id, params).ok();
    }
}

/// The panel's hideable parts, each as its UI command, part-store key and menu titles (shown, hidden).
fn parts() -> [(&'static str, &'static str, &'static str, &'static str); 2] {
    [
        ("ui.attributes.options", OPTIONS_KEY, tl!("Hide Options"), tl!("Show Options")),
        ("ui.attributes.note", NOTE_KEY, tl!("Hide Note"), tl!("Show Note")),
    ]
}

/// The panel menu's rows: hide or show the options and the note, then Panel Options….
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    parts().into_iter().map(|p| p.0).chain([PANEL_OPTIONS]).map(|id| PanelRow::Command(CommandRef::new(id))).collect()
}

/// The titles of the rows of [`menu_rows`]: Hide or Show by whether the part is hidden.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for (id, key, hide, show) in parts() {
        states.set(CommandRef::new(id), CommandState::new().title(if part_hidden(app, key) { show } else { hide }));
    }
    states.set(CommandRef::new(PANEL_OPTIONS), CommandState::new().title(tl!("Panel Options…")));
}

/// Runs the panel's own UI commands: flips the options or the note; `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let (_, key, ..) = parts().into_iter().find(|p| p.0 == id)?;
    let hidden = !part_hidden(app, key);
    set_part_hidden(app, key, hidden);
    Some(Ok(json!({ "hidden": hidden })))
}

/// `attributes.openUrl {url?}`: open `url`, else the selection's URL, in the web browser.
pub(crate) fn open_url(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    app.refuse_automation("attributes.openUrl")?;
    let url = match p.get("url").and_then(Value::as_str) {
        Some(u) => u.to_string(),
        None => app.session.attributes_info().url.filter(|u| !u.is_empty()).ok_or("the selection has no URL (or they differ)")?,
    };
    app.open_document_url(&url)?;
    Ok(json!({ "url": url }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use vectorcraft_engine::Session;

    fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
        app.session.execute(id, &p).unwrap()
    }

    /// The URLs the app asked egui to open in one frame.
    fn opened(app: &mut VectorcraftApp) -> Vec<String> {
        let mut out = crate::testing::ui_context().run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
        out.textures_delta.clear();
        out.platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn panel_shows_the_selections_attributes() {
        let mut panel = Panel::new(VectorcraftApp::new(Session::new(), Default::default()), show);
        run(&mut panel.app(), "file.new", json!({"width": 100, "height": 100}));
        panel.frame();
        assert!(panel.shows("No Selection") && panel.shows("Overprint Fill"));
        assert!(!panel.enabled("Overprint Fill"), "nothing selected to overprint");
        run(&mut panel.app(), "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        run(&mut panel.app(), "attributes.set", json!({"url": "https://example.com/a", "note": "first note", "imageMap": "polygon"}));
        panel.frame();
        assert_eq!(panel.value("URL").as_deref(), Some("https://example.com/a"));
        assert_eq!(panel.value("Note").as_deref(), Some("first note"));
        assert_eq!(panel.value("Image Map").as_deref(), Some("Polygon"));
        assert!(panel.shows("Browser") && panel.enabled("Browser") && !panel.shows("No Selection"));
        // A click turns overprint on, and a click on it turns it off.
        let fill = |panel: &Panel| info(&mut panel.app()).0.overprint_fill;
        panel.click("Overprint Fill");
        assert_eq!(fill(&panel), Some(true));
        panel.click("Overprint Fill");
        assert_eq!(fill(&panel), Some(false));
        // The URL button opens the selection's URL.
        let (found, paths) = info(&mut panel.app());
        assert!(paths && found.url.as_deref() == Some("https://example.com/a"));
        assert_eq!(open_url(&mut panel.app(), &json!({})).unwrap()["url"], "https://example.com/a");
        // Two objects whose values differ show blanks.
        let b = run(&mut panel.app(), "shape.ellipse", json!({"x": 60, "y": 0, "width": 30, "height": 30}))["id"].clone();
        let a = panel.app().session.active().unwrap().doc.layers[0].children().unwrap()[0].id.0;
        run(&mut panel.app(), "select.set", json!({"ids": [a, b]}));
        panel.frame();
        assert_ne!(panel.value("URL").as_deref(), Some("https://example.com/a"));
        assert_ne!(panel.value("Note").as_deref(), Some("first note"));
        assert!(!panel.enabled("Browser"));
        assert!(open_url(&mut panel.app(), &json!({})).is_err());
    }

    /// Only web and mail URLs reach the browser.
    #[test]
    fn the_browser_opens_web_and_mail_urls_only() {
        for ok in ["https://example.com/a", "HTTP://example.com", "mailto:someone@example.com"] {
            assert!(crate::browsable_url(ok).is_ok(), "{ok}");
        }
        for refused in [
            "file:///tmp/a.html",
            "javascript:void(0)",
            "ms-settings:",
            "smb://host/share",
            "x-app://run",
            "example.com",
            " https://example.com",
            "https://a\nb",
            "",
        ] {
            assert!(crate::browsable_url(refused).is_err(), "{refused}");
        }
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        run(&mut app, "attributes.set", json!({"url": "file:///tmp/run.desktop"}));
        assert!(open_url(&mut app, &json!({})).is_err());
        assert!(open_url(&mut app, &json!({"url": "vscode://file/x"})).is_err());
        assert!(opened(&mut app).is_empty());
        open_url(&mut app, &json!({"url": "https://example.com/b"})).unwrap();
        assert_eq!(opened(&mut app), ["https://example.com/b"]);
    }

    /// The URL menu lists the newest 10 recent URLs by default, and `ui.attributesPanelOptions`
    /// sets how many it lists, from 1 to 30, kept with the UI state. A lower count hides the older
    /// URLs without forgetting them, and a count that is not a whole number from 1 to 30 is an
    /// error that changes nothing.
    #[test]
    fn panel_options_set_how_many_recent_urls_the_menu_lists() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        for i in 0..32 {
            run(&mut app, "attributes.set", json!({"url": format!("https://example.com/{i}")}));
        }
        assert!(menu_rows(&app).iter().any(|r| matches!(r, PanelRow::Command(c) if c.id == PANEL_OPTIONS)));
        assert_eq!((menu_urls(&app).len(), menu_urls(&app)[0].as_str()), (10, "https://example.com/31"));
        assert_eq!(app.run("ui.attributesPanelOptions", json!({"count": 1})).unwrap(), json!({"count": 1}));
        assert_eq!(menu_urls(&app), ["https://example.com/31"]);
        app.run("ui.attributesPanelOptions", json!({"count": 30.0})).unwrap();
        assert_eq!((menu_urls(&app).len(), menu_urls(&app)[29].as_str()), (30, "https://example.com/2"));
        for bad in [json!(0), json!(31), json!(2.5), json!(-1), json!("5"), Value::Null] {
            assert!(app.run("ui.attributesPanelOptions", json!({"count": bad})).is_err(), "{bad}");
        }
        assert_eq!(app.ui.attributes_panel.recent_urls, 30, "a refused count changes nothing");
        assert_eq!(app.session.recent_urls.len(), 30);
        // Kept with the UI state; UI state saved without it, or with a count out of range, lists 10.
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(back.attributes_panel.recent_urls, 30);
        let old: crate::UiState = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old.attributes_panel, PanelOptions::default());
        for saved in [json!(0), json!(99), json!(1000), json!(4.5), json!("12"), Value::Null] {
            let s: crate::UiState = serde_json::from_value(json!({"attributes_panel": {"recentUrls": saved}})).unwrap();
            assert_eq!(s.attributes_panel.shown(), 10, "{saved}");
        }
        assert_eq!(PanelOptions { recent_urls: 200 }.shown(), 10);
    }
}
