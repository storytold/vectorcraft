//! File → File Info: the document title, author, description, rating, keywords (chips) and
//! copyright, with the created and modified dates.
//!
//! Fields: what `file.info` reports (and takes back) plus `__keyword`, the keyword being typed
//! (Enter or Add turns it into chips; OK keeps it too). OK runs `file.info` with them: one undo
//! step. On an error the dialog stays open.

use serde_json::{Value, json};
use vectorcraft_doc::CopyrightStatus;
use vectorcraft_doc::metadata::MAX_RATING;

use super::document_setup::{LABEL, choice};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, icons, widgets};

pub(super) const KIND: &str = "fileInfo";
/// The keyword being typed.
const PENDING: &str = "__keyword";
/// Width of the text fields.
const FIELD: f32 = 320.0;

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("File Info").into(), body, confirm, min_width: 500.0, max_width: Some(500.0), ..DialogSpec::FORM };

/// Open File Info on the active document's.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut fields = app.session.execute("file.info", &json!({})).map_err(|e| e.to_string())?;
    fields[PENDING] = json!("");
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    // A keyword typed but not added yet is kept.
    add_keywords(&mut d);
    let r = app.run("file.info", form::params(&d));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

/// The keywords in `d`.
fn keywords(d: &Dialog) -> Vec<String> {
    d.fields.get("keywords").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// Move the typed keywords (comma- or semicolon-separated) into the list, each once.
fn add_keywords(d: &mut Dialog) {
    let mut list = keywords(d);
    for k in d.str(PENDING).split([',', ';']).map(str::trim).filter(|k| !k.is_empty()) {
        if !list.iter().any(|o| o.to_lowercase() == k.to_lowercase()) {
            list.push(k.to_string());
        }
    }
    d.fields.insert("keywords".into(), json!(list));
    d.fields.insert(PENDING.into(), json!(""));
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    for (key, label) in [("title", tl!("Document Title:")), ("author", tl!("Author:")), ("authorTitle", tl!("Author Title:"))] {
        widgets::label_row(ui, label, LABEL, |ui| {
            form::text(ui, d, key, FIELD);
        });
        ui.add_space(4.0);
    }
    top_row(ui, tl!("Description:"), |ui| {
        form::text_area(ui, d, "description", FIELD, 3);
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Rating:"), LABEL, |ui| rating(ui, d));
    ui.add_space(4.0);
    top_row(ui, tl!("Keywords:"), |ui| {
        ui.vertical(|ui| {
            ui.set_max_width(FIELD + 12.0);
            chips(ui, d);
            ui.horizontal(|ui| {
                let r = form::text_edit(ui, d, PENDING, FIELD - 70.0);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter {
                    // Enter adds the keyword (it doesn't press OK) and keeps typing in the field.
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    r.request_focus();
                }
                let typed = !d.str(PENDING).trim().is_empty();
                if (widgets::flat_button(ui, tl!("Add"), 56.0).clicked() || enter) && typed {
                    add_keywords(d);
                }
            });
            form::caption(ui, tl!("Separate several keywords with commas."));
        });
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Copyright"));
    let statuses: Vec<(&str, &str)> = CopyrightStatus::ALL.iter().map(|c| (c.id(), c.label())).collect();
    choice(ui, d, "copyrightStatus", tl!("Copyright Status:"), &statuses);
    ui.add_space(4.0);
    top_row(ui, tl!("Copyright Notice:"), |ui| {
        form::text_area(ui, d, "copyrightNotice", FIELD, 2);
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Copyright Info URL:"), LABEL, |ui| {
        form::text(ui, d, "copyrightUrl", FIELD);
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Dates"));
    for (key, label) in [("created", tl!("Created:")), ("modified", tl!("Modified:"))] {
        // ISO 8601 UTC from the engine, shown as "2026-10-05 14:03:00 UTC".
        let shown = d
            .fields
            .get(key)
            .and_then(Value::as_str)
            .map_or_else(|| "—".to_string(), |s| format!("{} UTC", s.replace('T', " ").trim_end_matches('Z')));
        widgets::label_row(ui, label, LABEL, |ui| {
            ui.label(egui::RichText::new(shown).color(t.text_dim));
        });
    }
    false
}

/// A form row whose label sits at the top of a taller field.
fn top_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(LABEL, 24.0), egui::Sense::hover());
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, tl!(label), egui::FontId::proportional(12.5), t.text);
        add(ui);
    });
}

/// Five stars: a click sets the rating, a click on the current one clears it.
fn rating(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let current = d.f64("rating", 0.0) as u8;
    ui.spacing_mut().item_spacing.x = 2.0;
    for i in 1..=MAX_RATING {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
        let on = i <= current;
        let tint = if on {
            t.accent
        } else if resp.hovered() {
            t.text
        } else {
            t.text_dim
        };
        icons::paint(ui, "star", rect.shrink(2.0), tint);
        let resp = resp.on_hover_text(crate::i18n::tn(u64::from(i), "{n} star", "{n} stars"));
        if resp.clicked() {
            d.fields.insert("rating".into(), json!(if i == current { 0 } else { i }));
        }
    }
}

/// The keywords as chips; a chip's × removes it.
fn chips(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let list = keywords(d);
    if list.is_empty() {
        return;
    }
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (i, k) in list.iter().enumerate() {
            let galley = ui.painter().layout_no_wrap(k.clone(), egui::FontId::proportional(12.0), t.text_strong);
            let size = egui::vec2(galley.size().x + 30.0, 22.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().rect_filled(rect, egui::CornerRadius::same(11), t.hover);
            ui.painter().rect_stroke(rect, egui::CornerRadius::same(11), egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
            ui.painter().galley(egui::pos2(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0), galley, t.text_strong);
            let x = egui::Rect::from_center_size(egui::pos2(rect.right() - 11.0, rect.center().y), egui::vec2(14.0, 14.0));
            let resp = ui
                .interact(x, ui.scope_id().with(("keyword-remove", i)), egui::Sense::click())
                .on_hover_text(crate::i18n::fmt(tl!("Remove “{keyword}”"), &[("keyword", k.as_str())]));
            icons::paint(ui, "x", x.shrink(2.0), if resp.hovered() { t.text_strong } else { t.text_dim });
            if resp.clicked() {
                remove = Some(i);
            }
        }
    });
    if let Some(i) = remove {
        let mut list = list;
        list.remove(i);
        d.fields.insert("keywords".into(), json!(list));
    }
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn file_info_draws_and_ok_is_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        // The menu item and its shortcut open the dialog; agents use ui.fileInfoDialog.
        crate::menus::invoke(&mut app, "file.info", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(d.fields["created"].is_string(), "a new document has its creation date");
        for (k, v) in [("author", json!("Ada")), ("keywords", json!(["poster"])), ("rating", json!(4)), ("copyrightStatus", json!("copyrighted"))] {
            d.fields.insert(k.into(), v);
        }
        d.fields.insert(super::PENDING.into(), json!("print, Poster; blue"));
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        let m = &st.doc.metadata;
        assert_eq!((m.author.as_str(), m.rating, m.copyright_status), ("Ada", 4, vectorcraft_doc::CopyrightStatus::Copyrighted));
        assert_eq!(m.keywords, ["poster", "print", "blue"], "typed keywords join the chips, each once");
        assert_eq!(st.history.undo.len(), 1);
        // A bad value keeps the dialog open.
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("rating".into(), json!(9));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
    }
}
