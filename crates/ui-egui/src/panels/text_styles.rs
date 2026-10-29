//! Character Styles and Paragraph Styles panels (one implementation, two kinds).
//!
//! Click a style to apply it to the selected text (or the Type tool's selected characters);
//! Alt-click applies it and clears overrides. The style of the selection is highlighted, with a
//! "+" when the text overrides some of its attributes.

use egui::{Sense, Ui, vec2};
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::textstyles;

use super::character::{end_typing, text_editing, text_style};
use super::{pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Char,
    Para,
}

impl Kind {
    fn prefix(self) -> &'static str {
        if self == Kind::Char { "charStyle" } else { "paraStyle" }
    }
    fn what(self) -> &'static str {
        if self == Kind::Char { "Character" } else { "Paragraph" }
    }
    /// `what` in the UI language, for text shown to the user.
    fn what_label(self) -> &'static str {
        if self == Kind::Char { tl!("Character") } else { tl!("Paragraph") }
    }
    fn key(self) -> &'static str {
        if self == Kind::Char { "char-style-sel" } else { "para-style-sel" }
    }
    /// The style row the panel scrolls into view the next time it draws.
    fn scroll_key(self) -> &'static str {
        if self == Kind::Char { "char-style-scroll" } else { "para-style-scroll" }
    }
}

/// Highlights a style the panel just made and scrolls its row into view the next time it draws.
fn highlight_made(ctx: &egui::Context, kind: Kind, name: Option<String>) {
    set_pstate(ctx, kind.scroll_key(), name.clone());
    set_pstate(ctx, kind.key(), name);
}

/// A row dragged in the list: the style's kind and name. Dropped on Create New Style it is
/// duplicated (`charStyle.duplicate`, `paraStyle.duplicate`); dropped on Delete it is deleted
/// (`charStyle.delete`, `paraStyle.delete`) unless it is Normal. Each panel's buttons accept only rows of
/// their own kind.
#[derive(Clone)]
struct RowDrag {
    kind: Kind,
    name: String,
}

/// The style row of `kind` released on `button` this frame, if any; `button` is outlined while one
/// is held over it. `take`: whether a row with that name may be dropped there.
fn row_dropped(ui: &Ui, button: &egui::Response, kind: Kind, take: impl Fn(&str) -> bool) -> Option<String> {
    let held = button.dnd_hover_payload::<RowDrag>().filter(|d| d.kind == kind && take(&d.name))?;
    ui.painter().rect_stroke(button.rect, 3.0, egui::Stroke::new(1.5, Tokens::get(ui.ctx()).accent), egui::StrokeKind::Inside);
    button.dnd_release_payload::<RowDrag>().map(|_| held.name.clone())
}

/// (name, attrs, uses) for every style, Normal first.
fn styles(app: &mut VectorcraftApp, kind: Kind) -> Vec<(String, Map<String, Value>, u64)> {
    let v = app.session.execute(&format!("{}.list", kind.prefix()), &json!({})).unwrap_or_default();
    v["styles"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| {
                    (
                        s["name"].as_str().unwrap_or("").to_string(),
                        s["attrs"].as_object().cloned().unwrap_or_default(),
                        s["uses"].as_u64().unwrap_or(0),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The selection's style name (Normal when none) and every one of its attributes, written as a
/// style holds them, if text is selected.
fn current(app: &VectorcraftApp, kind: Kind) -> Option<(Option<String>, Map<String, Value>)> {
    let (c, p) = text_style(app)?;
    Some(match kind {
        Kind::Char => (c.style_name.clone(), textstyles::char_attrs(&c)),
        Kind::Para => (p.style_name.clone(), textstyles::para_attrs(&p)),
    })
}

/// Parameters targeting the selection: the Type tool's selected characters (paragraph styles: the
/// paragraphs its selection or caret touches), or the selected objects.
fn target(app: &VectorcraftApp, kind: Kind, mut p: Value) -> Value {
    if let Some((id, a, b)) = text_editing(app) {
        p["id"] = json!(id.0);
        if kind == Kind::Para || b > a {
            p["start"] = json!(a);
            p["end"] = json!(b);
        }
    }
    p
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui, kind: Kind) {
    let t = Tokens::get(ui.ctx());
    let all = styles(app, kind);
    let cur = current(app, kind);
    let normal = all.first().map(|s| s.0.clone()).unwrap_or_default();
    let cur_name = cur.as_ref().map(|(n, _)| n.clone().unwrap_or_else(|| normal.clone()));
    let mut clicked: Option<(String, bool)> = None;
    let mut sel: Option<String> = pstate(ui.ctx(), kind.key());
    let scroll_to: Option<String> = pstate(ui.ctx(), kind.scroll_key());
    widgets::list_box(ui, |ui| {
        for (name, attrs, _) in &all {
            let is_cur = cur_name.as_deref() == Some(name.as_str());
            // Overrides: an attribute the style sets that the selection doesn't match.
            let overridden = is_cur && cur.as_ref().is_some_and(|(_, m)| attrs.iter().any(|(k, v)| m.get(k) != Some(v)));
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
            if resp.drag_started() {
                egui::DragAndDrop::set_payload(ui.ctx(), RowDrag { kind, name: name.clone() });
            }
            if scroll_to.as_deref() == Some(name.as_str()) {
                ui.scroll_to_rect(r, None);
            }
            if is_cur || sel.as_deref() == Some(name.as_str()) {
                ui.painter().rect_filled(r, 0.0, if is_cur { t.row_selected } else { t.hover });
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, t.hover);
            }
            // The built-in "[Normal … Style]" (listed first) is translated where painted; other names
            // are user data, brackets or not.
            let shown = super::label_or_name(name, *name == normal);
            let label = if overridden { format!("{shown}+") } else { shown.to_string() };
            ui.painter().text(r.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), t.text);
            if resp.clicked() {
                clicked = Some((name.clone(), ui.input(|i| i.modifiers.alt)));
            }
        }
    });
    if scroll_to.is_some() {
        set_pstate::<Option<String>>(ui.ctx(), kind.scroll_key(), None);
    }
    let has_text = cur.is_some();
    if let Some((name, clear)) = clicked {
        sel = Some(name.clone());
        if has_text {
            let p = target(app, kind, json!({ "name": name, "clearOverrides": clear }));
            app.run(&format!("{}.apply", kind.prefix()), p).ok();
        }
    }
    // A highlighted style that is no longer listed (renamed in Style Options, deleted by a command
    // or undone) is no longer highlighted.
    if sel.as_ref().is_some_and(|s| !all.iter().any(|(n, ..)| n == s)) {
        sel = None;
    }
    set_pstate(ui.ctx(), kind.key(), sel.clone());
    let deletable = sel.as_ref().is_some_and(|s| *s != normal);
    widgets::bottom_bar(ui, |ui| {
        ui.add_space((ui.available_width() - 2.0 * 28.0).max(0.0));
        let new = widgets::icon_button_enabled(
            ui,
            "dc-new-item",
            &crate::i18n::fmt(tl!("Create New {kind} Style"), &[("kind", kind.what_label())]),
            false,
            true,
            24.0,
        );
        if let Some(name) = row_dropped(ui, &new, kind, |_| true) {
            if let Ok(r) = app.run(&format!("{}.duplicate", kind.prefix()), json!({ "name": name })) {
                highlight_made(ui.ctx(), kind, r["name"].as_str().map(str::to_string));
            }
        } else if new.clicked() {
            end_typing(app);
            if let Ok(r) = app.run(&format!("{}.new", kind.prefix()), target(app, kind, json!({}))) {
                highlight_made(ui.ctx(), kind, r["name"].as_str().map(str::to_string));
            }
        }
        let delete = widgets::icon_button_enabled(
            ui,
            "trash-2",
            &crate::i18n::fmt(tl!("Delete {kind} Style"), &[("kind", kind.what_label())]),
            false,
            deletable,
            24.0,
        );
        if let Some(name) = row_dropped(ui, &delete, kind, |n| n != normal) {
            if app.run(&format!("{}.delete", kind.prefix()), json!({ "name": name })).is_ok() && sel.as_ref() == Some(&name) {
                set_pstate::<Option<String>>(ui.ctx(), kind.key(), None);
            }
        } else if delete.clicked()
            && let Some(n) = &sel
            && app.run(&format!("{}.delete", kind.prefix()), json!({ "name": n })).is_ok()
        {
            set_pstate::<Option<String>>(ui.ctx(), kind.key(), None);
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui, kind: Kind) {
    let sel: Option<String> = pstate(ui.ctx(), kind.key());
    let normal = if kind == Kind::Char { "[Normal Character Style]" } else { "[Normal Paragraph Style]" };
    let user = sel.as_ref().is_some_and(|s| s != normal);
    let has_text = current(app, kind).is_some() || selection_len(app) > 0;
    let p = kind.prefix();
    if menu_item(ui, &crate::i18n::fmt(tl!("New {kind} Style…"), &[("kind", kind.what_label())]), true, false) {
        // A later typing preview must not restore the document from before this capture.
        end_typing(app);
        app.run(&format!("{p}.new"), target(app, kind, json!({}))).ok();
    }
    if menu_item(ui, &crate::i18n::fmt(tl!("Duplicate {kind} Style"), &[("kind", kind.what_label())]), sel.is_some(), false)
        && let Some(n) = &sel
        && let Ok(r) = app.run(&format!("{p}.duplicate"), json!({ "name": n }))
    {
        highlight_made(ui.ctx(), kind, r["name"].as_str().map(str::to_string));
    }
    if menu_item(ui, &crate::i18n::fmt(tl!("Delete {kind} Style"), &[("kind", kind.what_label())]), user, false)
        && let Some(n) = &sel
        && app.run(&format!("{p}.delete"), json!({ "name": n })).is_ok()
    {
        set_pstate::<Option<String>>(ui.ctx(), kind.key(), None);
    }
    ui.separator();
    if menu_item(ui, tl!("Clear Overrides"), sel.is_some() && has_text, false)
        && let Some(n) = &sel
    {
        app.run(&format!("{p}.apply"), target(app, kind, json!({ "name": n, "clearOverrides": true }))).ok();
    }
    if menu_item(ui, &crate::i18n::fmt(tl!("Redefine {kind} Style"), &[("kind", kind.what_label())]), sel.is_some() && has_text, false)
        && let Some(n) = &sel
    {
        end_typing(app);
        app.run(&format!("{p}.redefine"), target(app, kind, json!({ "name": n }))).ok();
    }
    if menu_item(ui, &crate::i18n::fmt(tl!("{kind} Style Options…"), &[("kind", kind.what_label())]), user, false)
        && let Some(n) = &sel
    {
        let label = format!("{} Style Options", kind.what());
        app.run("ui.paramDialog", json!({ "command": format!("{p}.rename"), "label": label, "params": { "name": n, "to": n } })).ok();
    }
}

#[cfg(test)]
mod tests;
