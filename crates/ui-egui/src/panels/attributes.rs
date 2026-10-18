//! Attributes panel: Overprint Fill / Overprint Stroke, Show / Don't Show Center, Reverse Path
//! Direction Off / On, the fill rule (non-zero winding or even-odd), Image Map, URL (with the
//! recent URLs and a Browser button) and the note. The menu hides the options and shows the note.
//! Everything reads `attributes.info` ([`vectorcraft_engine::Session::attributes_info`]) and acts
//! through `attributes.set`, `path.reverse` and `path.setFillRule`.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{ImageMap, NodeId, NodeKind};
use vectorcraft_engine::cmd::AttributesInfo;
use vectorcraft_geom::FillRule;

use super::{pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

/// Panel state: the options (centre, direction, fill rule, image map, URL) are hidden.
const HIDE_OPTIONS: &str = "attr-hide-options";
/// Panel state: the note field is hidden.
const HIDE_NOTE: &str = "attr-hide-note";
const LABEL_W: f32 = 70.0;

/// The panel's values and whether the targets hold paths (direction and fill rule apply),
/// recomputed only when the document or the selection changes.
fn state(app: &VectorcraftApp, ctx: &egui::Context) -> (AttributesInfo, bool) {
    type Key = (u64, u64, usize, Option<NodeId>, Option<NodeId>);
    let Some(st) = app.session.active() else { return Default::default() };
    let sel = &st.selection.objects;
    let key: Key = (st.uid, st.revision, sel.len(), sel.first().copied(), sel.last().copied());
    let id = egui::Id::new("attr-state");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<(Key, (AttributesInfo, bool))>(id))
        && k == key
    {
        return v;
    }
    let info = app.session.attributes_info();
    let mut paths = false;
    for n in info.ids.iter().filter_map(|id| st.doc.node(*id)) {
        n.walk(&mut |c| paths |= matches!(c.kind, NodeKind::Path { guide: false, .. }));
    }
    let v = (info, paths);
    ctx.data_mut(|d| d.insert_temp(id, (key, v.clone())));
    v
}

/// Run `attributes.set` with `p` on the selection (errors show in the status bar).
fn set(app: &mut VectorcraftApp, p: Value) {
    app.run("attributes.set", p).ok();
}

/// A pair of icon buttons choosing one of two values (`current`: the targets' value, `None` where
/// they differ); returns the value clicked.
fn pair<T: Copy + PartialEq>(ui: &mut Ui, enabled: bool, current: Option<T>, items: [(&str, &str, T); 2]) -> Option<T> {
    let mut clicked = None;
    for (icon, tip, v) in items {
        if widgets::icon_button_enabled(ui, icon, tip, enabled && current == Some(v), enabled, 24.0).clicked() {
            clicked = Some(v);
        }
    }
    clicked
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (info, paths) = state(app, ui.ctx());
    let has = !info.ids.is_empty();
    ui.horizontal(|ui| {
        for (label, key, value) in
            [(tl!("Overprint Fill"), "overprintFill", info.overprint_fill), (tl!("Overprint Stroke"), "overprintStroke", info.overprint_stroke)]
        {
            if widgets::check3(ui, label, value, has) {
                set(app, json!({ key: value != Some(true) }));
            }
            ui.add_space(8.0);
        }
    });
    if !pstate::<bool>(ui.ctx(), HIDE_OPTIONS) {
        widgets::divider(ui);
        ui.horizontal(|ui| {
            let center = [("dc-center-hide", tl!("Don't Show Center"), false), ("dc-center-show", tl!("Show Center"), true)];
            if let Some(v) = pair(ui, has, info.show_center, center) {
                set(app, json!({ "showCenter": v }));
            }
            ui.add_space(10.0);
            let dir = [("dc-dir-off", tl!("Reverse Path Direction Off"), false), ("dc-dir-on", tl!("Reverse Path Direction On"), true)];
            if let Some(v) = pair(ui, paths, info.reversed, dir) {
                app.run("path.reverse", json!({ "reversed": v })).ok();
            }
            ui.add_space(10.0);
            let rules = [
                ("dc-rule-nonzero", tl!("Use Non-Zero Winding Fill Rule"), FillRule::NonZero),
                ("dc-rule-evenodd", tl!("Use Even-Odd Fill Rule"), FillRule::EvenOdd),
            ];
            if let Some(r) = pair(ui, paths, info.fill_rule, rules) {
                app.run("path.setFillRule", json!({ "rule": if r == FillRule::EvenOdd { "evenOdd" } else { "nonZero" } })).ok();
            }
        });
        ui.add_space(4.0);
        let url = info.url.clone().filter(|u| !u.is_empty());
        widgets::label_row(ui, tl!("Image Map:"), LABEL_W, |ui| {
            ui.add_enabled_ui(has, |ui| {
                let labels = ImageMap::ALL.map(ImageMap::label);
                if let Some(i) = widgets::dropdown(ui, "attr-map", info.image_map.map_or("", ImageMap::label), &labels, 100.0) {
                    set(app, json!({ "imageMap": ImageMap::ALL[i] }));
                }
            });
            ui.add_space(6.0);
            let r = ui.add_enabled_ui(url.is_some(), |ui| widgets::flat_button(ui, tl!("Browser"), 64.0)).inner;
            if r.on_hover_text(tl!("Open the URL in the web browser")).clicked() {
                app.run("attributes.openUrl", json!({})).ok();
            }
        });
        widgets::label_row(ui, tl!("URL:"), LABEL_W, |ui| {
            ui.add_enabled_ui(has, |ui| {
                let width = (ui.available_width() - 26.0).max(80.0);
                if let Some(u) = widgets::text_field(ui, "attr-url", info.url.as_deref(), width, 1) {
                    set(app, json!({ "url": u }));
                }
                recent_urls(app, ui);
            });
        });
    }
    if !pstate::<bool>(ui.ctx(), HIDE_NOTE) {
        widgets::divider(ui);
        widgets::subheader(ui, tl!("Note:"));
        ui.add_enabled_ui(has, |ui| {
            if let Some(n) = widgets::text_field(ui, "attr-note", info.note.as_deref(), ui.available_width(), 3) {
                set(app, json!({ "note": n }));
            }
        });
    }
    if !has {
        widgets::dim_label(ui, if selection_len(app) == 0 { tl!("No Selection") } else { "" });
    }
}

/// The recent URLs button: a menu of the URLs given lately; picking one sets it.
fn recent_urls(app: &mut VectorcraftApp, ui: &mut Ui) {
    let resp = widgets::icon_button_enabled(ui, "chevron-down", tl!("Recent URLs"), false, !app.session.recent_urls.is_empty(), 20.0);
    let mut chosen = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(220.0);
        for u in &app.session.recent_urls {
            if widgets::menu_item_name(ui, u, true, false) {
                chosen = Some(u.clone());
            }
        }
    });
    if let Some(u) = chosen {
        set(app, json!({ "url": u }));
    }
}

pub fn menu(_app: &mut VectorcraftApp, ui: &mut Ui) {
    for (key, hide, show) in [(HIDE_OPTIONS, tl!("Hide Options"), tl!("Show Options")), (HIDE_NOTE, tl!("Hide Note"), tl!("Show Note"))] {
        let hidden: bool = pstate(ui.ctx(), key);
        if menu_item(ui, if hidden { show } else { hide }, true, false) {
            set_pstate(ui.ctx(), key, !hidden);
        }
    }
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
    use vectorcraft_engine::Session;

    /// Run the panel and its menu for one headless frame: the texts drawn.
    fn frame(app: &mut VectorcraftApp) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
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

    #[test]
    fn panel_shows_the_selections_attributes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let texts = frame(&mut app);
        assert!(texts.iter().any(|t| t == "No Selection") && texts.iter().any(|t| t == "Overprint Fill"), "{texts:?}");
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        run(&mut app, "attributes.set", json!({"url": "https://example.com/a", "note": "first note", "imageMap": "polygon"}));
        let texts = frame(&mut app);
        for want in ["https://example.com/a", "first note", "Polygon", "Image Map:", "URL:", "Browser", "   Hide Options", "   Hide Note"] {
            assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
        }
        assert!(!texts.iter().any(|t| t == "No Selection"));
        // The URL button opens the selection's URL.
        let (info, paths) = state(&app, &egui::Context::default());
        assert!(paths && info.url.as_deref() == Some("https://example.com/a"));
        assert_eq!(open_url(&mut app, &json!({})).unwrap()["url"], "https://example.com/a");
        // Two objects whose values differ show blanks.
        let b = run(&mut app, "shape.ellipse", json!({"x": 60, "y": 0, "width": 30, "height": 30}))["id"].clone();
        let a = app.session.active().unwrap().doc.layers[0].children().unwrap()[0].id.0;
        run(&mut app, "select.set", json!({"ids": [a, b]}));
        let texts = frame(&mut app);
        assert!(!texts.iter().any(|t| t == "https://example.com/a" || t == "first note"), "{texts:?}");
        assert!(open_url(&mut app, &json!({})).is_err());
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
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let o = opened.clone();
        let services = crate::Services { open_url: Some(Box::new(move |u: &str| o.borrow_mut().push(u.to_string()))), ..Default::default() };
        let mut app = VectorcraftApp::new(Session::new(), services);
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        run(&mut app, "attributes.set", json!({"url": "file:///tmp/run.desktop"}));
        assert!(open_url(&mut app, &json!({})).is_err());
        assert!(open_url(&mut app, &json!({"url": "vscode://file/x"})).is_err());
        assert!(opened.borrow().is_empty());
        open_url(&mut app, &json!({"url": "https://example.com/b"})).unwrap();
        assert_eq!(*opened.borrow(), ["https://example.com/b"]);
    }
}
