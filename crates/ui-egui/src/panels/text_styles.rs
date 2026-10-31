//! Character Styles and Paragraph Styles panels (one implementation, two kinds).
//!
//! Click a style to apply it to the selected text (or the Type tool's selected characters);
//! Alt-click applies it and clears overrides. The style of the selection is highlighted, with a
//! "+" when the text overrides some of its attributes. New makes a style from the selected text;
//! Alt-click on New first asks for its name (`ui.newCharStyle`, `ui.newParaStyle`).

use std::cell::RefCell;

use egui::{Color32, Sense, Ui, vec2};
use serde_json::{Map, Value, json};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::textstyles;

use super::character::{end_typing, text_editing, text_style};
use super::memo::Memo;
use super::{alt_held, pstate, selection_len, set_pstate};
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
    /// Panel state holding the styles highlighted together, with the style [`Kind::key`] held
    /// when they were highlighted.
    fn many_key(self) -> &'static str {
        if self == Kind::Char { "char-style-many" } else { "para-style-many" }
    }
    /// Panel state holding the style a Shift-click extends the highlight from.
    fn anchor_key(self) -> &'static str {
        if self == Kind::Char { "char-style-anchor" } else { "para-style-anchor" }
    }
    /// The style row the panel scrolls into view the next time it draws.
    fn scroll_key(self) -> &'static str {
        if self == Kind::Char { "char-style-scroll" } else { "para-style-scroll" }
    }
    /// Panel state holding the `uid` of the document the highlight was made in.
    fn document_key(self) -> &'static str {
        if self == Kind::Char { "char-style-document" } else { "para-style-document" }
    }
}

/// The styles highlighted in the panel, in the order they were picked, among the styles `listed`:
/// the styles a Shift-click, a Cmd/Ctrl-click or Select All Unused highlighted together while
/// [`Kind::key`] still holds the style it held then, else the one style in [`Kind::key`].
fn highlighted(app: &VectorcraftApp, ctx: &egui::Context, kind: Kind, listed: &[&str]) -> Vec<String> {
    let document = app.session.active().map(|st| st.uid);
    if pstate::<Option<u64>>(ctx, kind.document_key()) != document {
        set_pstate(ctx, kind.document_key(), document);
        set_highlighted(ctx, kind, vec![]);
        set_pstate(ctx, kind.anchor_key(), String::new());
        set_pstate::<Option<String>>(ctx, kind.scroll_key(), None);
    }
    let one: Option<String> = pstate(ctx, kind.key());
    let (with, many): (Option<String>, Vec<String>) = pstate(ctx, kind.many_key());
    let mut names = if with == one && !many.is_empty() { many } else { one.into_iter().collect() };
    names.retain(|n| listed.contains(&n.as_str()));
    names
}

/// Highlights `names` in the panel: [`Kind::key`] holds the first of them.
fn set_highlighted(ctx: &egui::Context, kind: Kind, names: Vec<String>) {
    let one = names.first().cloned();
    set_pstate(ctx, kind.key(), one.clone());
    set_pstate(ctx, kind.many_key(), (one, names));
}

/// Highlights the styles the panel just made and scrolls the first one's row into view the next
/// time it draws.
fn highlight_made(ctx: &egui::Context, kind: Kind, names: Vec<String>) {
    set_pstate(ctx, kind.scroll_key(), names.first().cloned());
    set_highlighted(ctx, kind, names);
}

/// Duplicates the styles `names` (`charStyle.duplicate`, `paraStyle.duplicate`) in one undo step
/// and highlights the copies.
fn duplicate(app: &mut VectorcraftApp, ctx: &egui::Context, kind: Kind, names: &[String]) {
    let p = kind.prefix();
    let made = if let [name] = names {
        app.run(&format!("{p}.duplicate"), json!({ "name": name })).map(|r| vec![r])
    } else {
        let commands: Vec<Value> = names.iter().map(|n| json!({ "command": format!("{p}.duplicate"), "params": { "name": n } })).collect();
        let label = if kind == Kind::Char { "New Character Style" } else { "New Paragraph Style" };
        app.run("command.batch", json!({ "label": label, "commands": commands })).map(|r| r["results"].as_array().cloned().unwrap_or_default())
    };
    if let Ok(made) = made {
        highlight_made(ctx, kind, made.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect());
    }
}

/// A row dragged in the list: the style's kind and name. Dropped on Create New Style it is
/// duplicated (`charStyle.duplicate`, `paraStyle.duplicate`); dropped on Delete it is deleted
/// (`charStyle.delete`, `paraStyle.delete`) unless it is Normal. A highlighted row carries every
/// highlighted style with it. Each panel's buttons accept only rows of their own kind.
#[derive(Clone)]
struct RowDrag {
    kind: Kind,
    name: String,
}

/// The styles a row dragged from the list carries: every highlighted style (`sel`) in the order
/// the panel lists them when the row is one of them, else the row's style alone.
fn dragged(sel: &[String], listed: &[&str], name: &str) -> Vec<String> {
    if sel.iter().any(|s| s == name) {
        listed.iter().filter(|n| sel.iter().any(|s| s == *n)).map(|n| n.to_string()).collect()
    } else {
        vec![name.to_string()]
    }
}

/// The style row of `kind` released on `button` this frame, if any; `button` is outlined while one
/// is held over it. `take`: whether a row with that name may be dropped there.
fn row_dropped(ui: &Ui, button: &egui::Response, kind: Kind, take: impl Fn(&str) -> bool) -> Option<String> {
    let held = button.dnd_hover_payload::<RowDrag>().filter(|d| d.kind == kind && take(&d.name))?;
    ui.painter().rect_stroke(button.rect, 3.0, egui::Stroke::new(1.5, Tokens::get(ui.ctx()).accent), egui::StrokeKind::Inside);
    button.dnd_release_payload::<RowDrag>().map(|_| held.name.clone())
}

/// (name, attrs, uses) for every style, Normal first, listed once per document version.
fn styles(app: &mut VectorcraftApp, kind: Kind) -> Vec<(String, Map<String, Value>, u64)> {
    thread_local! {
        static MEMO: RefCell<[Memo<Document, Value>; 2]> = RefCell::new([Memo::default(), Memo::default()]);
    }
    let Some((doc, revision)) = app.session.active().map(|st| (st.doc.clone(), st.revision)) else { return vec![] };
    let list = || {
        #[cfg(test)]
        LISTED.with(|n| n.set(n.get() + 1));
        app.session.execute(&format!("{}.list", kind.prefix()), &json!({})).unwrap_or_default()
    };
    let v = MEMO.with(|m| m.borrow_mut()[kind as usize].get(&doc, revision, list));
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

#[cfg(test)]
thread_local! {
    /// How many times [`styles`] has run `charStyle.list` or `paraStyle.list` on this thread.
    static LISTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
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

/// Open New Character Style (`Kind::Para`: New Paragraph Style) to name a style made from the
/// selected text: `ui.newCharStyle` and `ui.newParaStyle`.
pub(crate) fn open_new(app: &mut VectorcraftApp, kind: Kind) -> Result<Value, String> {
    // A later typing preview must not restore the document from before this capture.
    end_typing(app);
    let t = target(app, kind, json!({}));
    crate::dialogs::new_text_style::open(app, kind == Kind::Para, t)
}

/// A style row's fill, and whether it gets an accent outline: the current style's row is filled
/// with `row_selected` and outlined when it is also highlighted, so Cmd/Ctrl-clicking it shows.
fn row_look(is_cur: bool, in_highlight: bool, hovered: bool, t: &Tokens) -> (Option<Color32>, bool) {
    let fill = if is_cur {
        Some(t.row_selected)
    } else if in_highlight || hovered {
        Some(t.hover)
    } else {
        None
    };
    (fill, is_cur && in_highlight)
}

/// The style list and its New and Delete buttons. Shift-click highlights every style from the
/// last one clicked without Shift through this one, and Cmd/Ctrl-click adds or removes one; both
/// leave the text as it is. Delete deletes every highlighted style but the Normal style.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui, kind: Kind) {
    let t = Tokens::get(ui.ctx());
    let all = styles(app, kind);
    let cur = current(app, kind);
    let normal = all.first().map(|s| s.0.clone()).unwrap_or_default();
    let cur_name = cur.as_ref().map(|(n, _)| n.clone().unwrap_or_else(|| normal.clone()));
    let mut clicked: Option<(String, egui::Modifiers)> = None;
    let listed: Vec<&str> = all.iter().map(|s| s.0.as_str()).collect();
    // A highlighted style that is no longer listed (renamed in Style Options, deleted by a command
    // or undone) is no longer highlighted.
    let mut sel = highlighted(app, ui.ctx(), kind, &listed);
    // A style made by the New style dialog waits for this kind's panel on its own document.
    app.ui.made_text_style.retain(|(uid, _, _)| app.session.documents().iter().any(|doc| doc.uid == *uid));
    if let Some(index) = app
        .ui
        .made_text_style
        .iter()
        .position(|(uid, para, _)| *para == (kind == Kind::Para) && app.session.active().is_some_and(|doc| doc.uid == *uid))
    {
        let (_, _, name) = app.ui.made_text_style.remove(index);
        highlight_made(ui.ctx(), kind, vec![name]);
        sel = highlighted(app, ui.ctx(), kind, &listed);
    }
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
            let (fill, outline) = row_look(is_cur, sel.contains(name), resp.hovered(), &t);
            if let Some(fill) = fill {
                ui.painter().rect_filled(r, 0.0, fill);
            }
            if outline {
                ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            }
            // The built-in "[Normal … Style]" (listed first) is translated where painted; other names
            // are user data, brackets or not.
            let shown = super::label_or_name(name, *name == normal);
            let label = if overridden { format!("{shown}+") } else { shown.to_string() };
            ui.painter().text(r.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), t.text);
            if resp.clicked() {
                clicked = Some((name.clone(), ui.input(|i| i.modifiers)));
            }
        }
    });
    if scroll_to.is_some() {
        set_pstate::<Option<String>>(ui.ctx(), kind.scroll_key(), None);
    }
    let has_text = cur.is_some();
    if let Some((name, m)) = clicked {
        sel = super::swatches::click_selection(ui, kind.anchor_key(), sel, &listed, &name, m);
        // Shift and Cmd/Ctrl clicks only highlight, as in the Graphic Styles panel.
        if has_text && !(m.shift || m.command) {
            let p = target(app, kind, json!({ "name": name, "clearOverrides": m.alt }));
            app.run(&format!("{}.apply", kind.prefix()), p).ok();
        }
    }
    set_highlighted(ui.ctx(), kind, sel.clone());
    let doomed: Vec<&String> = sel.iter().filter(|s| **s != normal).collect();
    let deletable = !doomed.is_empty();
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
            duplicate(app, ui.ctx(), kind, &dragged(&sel, &listed, &name));
        } else if new.clicked() {
            end_typing(app);
            if alt_held(ui) {
                app.run(if kind == Kind::Char { "ui.newCharStyle" } else { "ui.newParaStyle" }, json!({})).ok();
            } else if let Ok(r) = app.run(&format!("{}.new", kind.prefix()), target(app, kind, json!({}))) {
                highlight_made(ui.ctx(), kind, r["name"].as_str().map(str::to_string).into_iter().collect());
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
        let drop_names = |n: &str| dragged(&sel, &listed, n).into_iter().filter(|s| *s != normal).collect::<Vec<_>>();
        if let Some(name) = row_dropped(ui, &delete, kind, |n| !drop_names(n).is_empty()) {
            // Deleting the highlighted styles clears the highlight; a row dropped from outside it
            // leaves it as it is.
            if app.run(&format!("{}.delete", kind.prefix()), json!({ "names": drop_names(&name) })).is_ok() && sel.contains(&name) {
                set_highlighted(ui.ctx(), kind, vec![]);
            }
        } else if delete.clicked() && app.run(&format!("{}.delete", kind.prefix()), json!({ "names": doomed })).is_ok() {
            set_highlighted(ui.ctx(), kind, vec![]);
        }
    });
}

/// The panel menu. Select All Unused highlights the styles no text uses (`charStyle.unused`,
/// `paraStyle.unused`), Duplicate copies every highlighted style and highlights the copies, and
/// Delete deletes every highlighted style but the Normal style.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui, kind: Kind) {
    let all = styles(app, kind);
    let listed: Vec<&str> = all.iter().map(|s| s.0.as_str()).collect();
    let many = highlighted(app, ui.ctx(), kind, &listed);
    // The items for one style act on the highlighted style while it is the only one.
    let sel: Option<String> = if many.len() == 1 { many.first().cloned() } else { None };
    let normal = if kind == Kind::Char { "[Normal Character Style]" } else { "[Normal Paragraph Style]" };
    let user = sel.as_ref().is_some_and(|s| s != normal);
    let doomed: Vec<&String> = many.iter().filter(|s| *s != normal).collect();
    let has_text = current(app, kind).is_some() || selection_len(app) > 0;
    let p = kind.prefix();
    if menu_item(ui, &crate::i18n::fmt(tl!("New {kind} Style…"), &[("kind", kind.what_label())]), true, false) {
        // A later typing preview must not restore the document from before this capture.
        end_typing(app);
        if let Ok(r) = app.run(&format!("{p}.new"), target(app, kind, json!({}))) {
            highlight_made(ui.ctx(), kind, r["name"].as_str().map(str::to_string).into_iter().collect());
        }
    }
    if menu_item(ui, &crate::i18n::fmt(tl!("Duplicate {kind} Style"), &[("kind", kind.what_label())]), !many.is_empty(), false) {
        let names: Vec<String> = listed.iter().filter(|n| many.iter().any(|s| s == *n)).map(|n| n.to_string()).collect();
        duplicate(app, ui.ctx(), kind, &names);
    }
    if menu_item(ui, &crate::i18n::fmt(tl!("Delete {kind} Style"), &[("kind", kind.what_label())]), !doomed.is_empty(), false)
        && app.run(&format!("{p}.delete"), json!({ "names": doomed })).is_ok()
    {
        set_highlighted(ui.ctx(), kind, vec![]);
    }
    if menu_item(ui, tl!("Select All Unused"), app.session.active().is_some(), false)
        && let Ok(r) = app.run(&format!("{p}.unused"), json!({}))
    {
        set_highlighted(ui.ctx(), kind, serde_json::from_value(r["names"].clone()).unwrap_or_default());
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
