//! Character Styles and Paragraph Styles panels (one implementation, two kinds).
//!
//! The framework's [`TextStylesPanel`] draws the list and footer and reports what the user asked for; this
//! module runs it as commands on the document ([`run_action`]) and keeps what each panel highlights
//! ([`StylesPanels`]). Alt on New first asks for the style's name (`ui.newCharStyle`, `ui.newParaStyle`).

use std::cell::RefCell;

// PLAN: the framework import paths below are assumed from the crate names; adjust to the app's re-exports.
use craft::behavior::Selection;
use craft::host::{PickKind, PickRequest};
use craft::text::ui::style_panel::{TextStyleAction, TextStyleKind, TextStyleRow, TextStylesPanel};
use craft::ui::contract::Bind;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::{fileio, textstyles};

use super::character::{end_typing, shared, text_editing, text_style};
use super::memo::Memo;
use super::paragraph::shared_para;
use super::selection_len;
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Char,
    Para,
}

impl Kind {
    fn prefix(self) -> &'static str {
        if self == Kind::Char { "charStyle" } else { "paraStyle" }
    }
    #[cfg(test)]
    fn what(self) -> &'static str {
        if self == Kind::Char { "Character" } else { "Paragraph" }
    }
    /// The framework panel's kind.
    fn panel_kind(self) -> TextStyleKind {
        if self == Kind::Char { TextStyleKind::Character } else { TextStyleKind::Paragraph }
    }
}

/// What the two styles panels keep between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct StylesPanels {
    pub char: StylesPanel,
    pub para: StylesPanel,
    /// Pending new style highlights: document uid, paragraph-style flag, and name. Each document and style
    /// kind has at most one pending highlight.
    pub made: Vec<(u64, bool, String)>,
}

impl StylesPanels {
    fn of(&mut self, kind: Kind) -> &mut StylesPanel {
        if kind == Kind::Char { &mut self.char } else { &mut self.para }
    }
}

/// One panel's highlight. Names are the stored form: row ids are list positions, rebuilt every pass.
#[derive(Clone, Debug, Default)]
pub(crate) struct StylesPanel {
    /// The uid of the document the highlight was made in.
    document: Option<u64>,
    /// Changes when the panel switches documents, so reopening a document cannot restore its old popup.
    scope_generation: u64,
    /// The highlighted styles, in the order they were picked.
    highlight: Vec<String>,
    /// The selection's lead style.
    lead: Option<String>,
    /// The style a Shift-click extends the highlight from.
    anchor: Option<String>,
    /// The style row the panel scrolls into view the next time it draws.
    reveal: Option<String>,
}

/// The styles highlighted in the panel, in the order they were picked, among the styles `listed`; a
/// highlight made in another document is dropped. A highlight made before the panel first read one (an
/// action from the panel menu before the list drew) is the active document's.
fn highlighted(app: &mut VectorcraftApp, kind: Kind, listed: &[&str]) -> Vec<String> {
    let document = app.session.active().map(|st| st.uid);
    let panel = app.ui.text_styles.of(kind);
    if panel.document.is_some_and(|made| Some(made) != document) {
        *panel = StylesPanel { scope_generation: panel.scope_generation.wrapping_add(1), ..StylesPanel::default() };
    }
    panel.document = document;
    panel.highlight.retain(|n| listed.contains(&n.as_str()));
    panel.highlight.clone()
}

/// Highlights `names` in the panel; the first of them leads.
fn set_highlighted(app: &mut VectorcraftApp, kind: Kind, names: Vec<String>) {
    let panel = app.ui.text_styles.of(kind);
    panel.lead = names.first().cloned();
    panel.anchor = panel.lead.clone();
    panel.highlight = names;
}

/// Highlights the styles the panel just made and scrolls the first one's row into view.
fn highlight_made(app: &mut VectorcraftApp, kind: Kind, names: Vec<String>) {
    let first = names.first().cloned();
    set_highlighted(app, kind, names);
    app.ui.text_styles.of(kind).reveal = first;
}

/// Duplicates the styles `names` (`charStyle.duplicate`, `paraStyle.duplicate`) in one undo step
/// and highlights the copies; none runs nothing.
fn duplicate(app: &mut VectorcraftApp, kind: Kind, names: &[String]) {
    if names.is_empty() {
        return;
    }
    let p = kind.prefix();
    let made = if let [name] = names {
        app.run(&format!("{p}.duplicate"), json!({ "name": name })).map(|r| vec![r])
    } else {
        let commands: Vec<Value> = names.iter().map(|n| json!({ "command": format!("{p}.duplicate"), "params": { "name": n } })).collect();
        let label = if kind == Kind::Char { "New Character Style" } else { "New Paragraph Style" };
        app.run("command.batch", json!({ "label": label, "commands": commands })).map(|r| r["results"].as_array().cloned().unwrap_or_default())
    };
    if let Ok(made) = made {
        highlight_made(app, kind, made.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect());
    }
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

/// The style name (`None`: the Normal style) that every character the panel acts on has (paragraph
/// styles: every paragraph), with the Selection tool in every frame of the selected stories;
/// `None` when they differ or no type is selected.
fn current(app: &VectorcraftApp, kind: Kind) -> Option<Option<String>> {
    match kind {
        Kind::Char => shared(app, |s| s.style_name.clone()),
        Kind::Para => shared_para(app, |p| p.style_name.clone()),
    }
}

/// Whether any character (paragraph styles: paragraph) the panel acts on differs from the style
/// attributes `attrs` in one of them.
fn has_overrides(app: &VectorcraftApp, kind: Kind, attrs: &Map<String, Value>) -> bool {
    let differs = |m: Map<String, Value>| attrs.iter().any(|(k, v)| m.get(k) != Some(v));
    let same = match kind {
        Kind::Char => shared(app, |s| differs(textstyles::char_attrs(s))),
        Kind::Para => shared_para(app, |p| differs(textstyles::para_attrs(p))),
    };
    same != Some(false)
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

/// [`target`] for applying a style: after Select All in threaded text, all the text of every
/// frame of the story; with the Selection tool, every frame of the selected stories
/// ([`super::character::selected_stories`]).
fn apply_target(app: &VectorcraftApp, kind: Kind, mut p: Value) -> Value {
    if text_editing(app).is_none() {
        super::character::target_selected_stories(app, &mut p);
        return p;
    }
    match super::character::text_story(app) {
        Some(story) => {
            p["ids"] = json!(story.iter().map(|f| f.0).collect::<Vec<_>>());
            p
        }
        None => target(app, kind, p),
    }
}

/// Open New Character Style (`Kind::Para`: New Paragraph Style) to name a style made from the
/// selected text: `ui.newCharStyle` and `ui.newParaStyle`.
pub(crate) fn open_new(app: &mut VectorcraftApp, kind: Kind) -> Result<Value, String> {
    // A later typing preview must not restore the document from before this capture.
    end_typing(app);
    let t = target(app, kind, json!({}));
    crate::dialogs::new_text_style::open(app, kind == Kind::Para, t)
}

/// Opens `name`'s options: Character Style Options for a character style, Paragraph Style Options
/// for a paragraph style.
fn open_style_options(app: &mut VectorcraftApp, kind: Kind, name: &str) {
    let command = if kind == Kind::Char { "ui.charStyleOptions" } else { "ui.paraStyleOptions" };
    app.run(command, json!({ "name": name })).ok();
}

/// Takes the style a New style dialog made for this kind's panel on its own document, and drops the entries of
/// documents that have closed.
fn take_made(app: &mut VectorcraftApp, kind: Kind) -> Option<String> {
    app.ui.text_styles.made.retain(|(uid, _, _)| app.session.documents().iter().any(|doc| doc.uid == *uid));
    let index = app
        .ui
        .text_styles
        .made
        .iter()
        .position(|(uid, para, _)| *para == (kind == Kind::Para) && app.session.active().is_some_and(|doc| doc.uid == *uid))?;
    Some(app.ui.text_styles.made.remove(index).2)
}

/// The style list and its footer: the framework panel over this document's styles. Shift-click and Cmd/Ctrl-click
/// highlight several styles without touching the text; the footer's Delete deletes every highlighted style but
/// the Normal style.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui, kind: Kind) {
    let all = styles(app, kind);
    let listed: Vec<&str> = all.iter().map(|s| s.0.as_str()).collect();
    let id_of = |name: &str| listed.iter().position(|n| *n == name).map(|i| i as u64);
    // Drops a highlight made in another document before the made style replaces it.
    highlighted(app, kind, &listed);
    if let Some(name) = take_made(app, kind) {
        highlight_made(app, kind, vec![name]);
    }
    let names = highlighted(app, kind, &listed);
    let panel = app.ui.text_styles.of(kind);
    let (lead, anchor, reveal) = (panel.lead.clone(), panel.anchor.clone(), panel.reveal.take());
    let scope = (panel.document, panel.scope_generation);
    let mut sel = Selection::new(names.iter().filter_map(|n| id_of(n)).collect(), lead.as_deref().and_then(id_of), anchor.as_deref().and_then(id_of));
    let rows: Vec<TextStyleRow> =
        listed.iter().enumerate().map(|(i, name)| TextStyleRow::new(i as u64, super::label_or_name(name, i == 0))).collect();
    let cur_id = current(app, kind).and_then(|n| match n {
        None => Some(0),
        Some(n) => id_of(&n),
    });
    let overridden = cur_id.and_then(|i| all.get(i as usize)).is_some_and(|s| has_overrides(app, kind, &s.1));
    let label = if kind == Kind::Char { tl!("Character Styles") } else { tl!("Paragraph Styles") };
    let mut list = TextStylesPanel::new(label, kind.panel_kind(), &rows, Bind::bound(&mut sel))
        .current(cur_id, overridden)
        .target(text_style(app).is_some() || selection_len(app) > 0)
        // A character style has nothing to apply to at a bare caret; the panel then reports a status instead.
        .text_selected(text_editing(app).is_none_or(|(_, a, b)| a != b))
        // [Normal Character Style] takes Style Options, Duplicate and Redefine, as [Normal Paragraph Style] does.
        .base_character_style(true)
        .highlighting(true);
    if let Some(id) = reveal.as_deref().and_then(id_of) {
        list = list.reveal(id);
    }
    // The framework's row menu and footer drag use list indices that are local to this document.
    let out = ui.push_id(("text-styles-document", kind.panel_kind(), scope), |ui| list.show(ui)).inner;
    let name_of = |id: &u64| listed.get(*id as usize).map(|n| n.to_string());
    let panel = app.ui.text_styles.of(kind);
    panel.highlight = sel.items.iter().filter_map(name_of).collect();
    panel.lead = sel.lead.as_ref().and_then(name_of);
    panel.anchor = sel.anchor.as_ref().and_then(name_of);
    if let Some(msg) = out.status {
        app.status(msg.text.as_str());
    }
    if let Some(action) = out.action {
        run_action(app, kind, action);
    }
}

/// Runs what a styles panel asked for on the document.
pub(crate) fn run_action(app: &mut VectorcraftApp, kind: Kind, action: TextStyleAction) {
    let all = styles(app, kind);
    let listed: Vec<&str> = all.iter().map(|s| s.0.as_str()).collect();
    let name = |id: u64| listed.get(id as usize).map(|n| n.to_string());
    let normal = listed.first().map(|n| n.to_string()).unwrap_or_default();
    let current_name = current(app, kind).map(|n| n.unwrap_or_else(|| normal.clone()));
    let p = kind.prefix();
    // The highlighted styles in list order, with or without Normal.
    let marked = |app: &mut VectorcraftApp, with_normal: bool| -> Vec<String> {
        let picked = highlighted(app, kind, &listed);
        listed.iter().enumerate().filter(|(i, n)| (with_normal || *i != 0) && picked.iter().any(|s| s == *n)).map(|(_, n)| n.to_string()).collect()
    };
    match action {
        TextStyleAction::Apply { id, clear_overrides, .. } => {
            if let Some(n) = name(id) {
                app.run(&format!("{p}.apply"), apply_target(app, kind, json!({ "name": n, "clearOverrides": clear_overrides }))).ok();
            }
        }
        TextStyleAction::ClearOverride => {
            if let Some(n) = current_name {
                app.run(&format!("{p}.apply"), apply_target(app, kind, json!({ "name": n, "clearOverrides": true }))).ok();
            }
        }
        TextStyleAction::Redefine(id) => {
            if let Some(n) = name(id) {
                end_typing(app);
                app.run(&format!("{p}.redefine"), target(app, kind, json!({ "name": n }))).ok();
            }
        }
        TextStyleAction::New { ask_name: true, .. } => {
            open_new(app, kind).ok();
        }
        TextStyleAction::New { ask_name: false, .. } => {
            end_typing(app);
            if let Ok(r) = app.run(&format!("{p}.new"), target(app, kind, json!({}))) {
                highlight_made(app, kind, r["name"].as_str().map(str::to_string).into_iter().collect());
            }
        }
        TextStyleAction::Duplicate(id) => {
            if let Some(n) = name(id) {
                duplicate(app, kind, &[n]);
            }
        }
        TextStyleAction::DuplicateHighlighted => {
            let names = marked(app, true);
            duplicate(app, kind, &names);
        }
        TextStyleAction::Delete(id) => {
            if let Some(n) = name(id).filter(|_| id != 0)
                && app.run(&format!("{p}.delete"), json!({ "names": [n] })).is_ok()
            {
                set_highlighted(app, kind, vec![]);
            }
        }
        TextStyleAction::DeleteHighlighted => {
            let names = marked(app, false);
            if app.run(&format!("{p}.delete"), json!({ "names": names })).is_ok() {
                set_highlighted(app, kind, vec![]);
            }
        }
        // The Normal style of either kind opens its options too; its name stays fixed there.
        TextStyleAction::Options(id) => {
            if let Some(n) = name(id) {
                open_style_options(app, kind, &n);
            }
        }
        TextStyleAction::SelectUnused => {
            if let Ok(r) = app.run(&format!("{p}.unused"), json!({})) {
                set_highlighted(app, kind, serde_json::from_value(r["names"].clone()).unwrap_or_default());
            }
        }
        // The panel offers no other action.
        _ => {}
    }
}

/// The load command ids of `kind`'s panel menu: its own styles, then every kind.
fn load_commands(kind: Kind) -> [&'static str; 2] {
    [if kind == Kind::Char { "ui.loadCharStyles" } else { "ui.loadParaStyles" }, "ui.loadAllStyles"]
}

/// The panel menu: the framework's rows, then the load commands.
pub(crate) fn menu_rows(_app: &VectorcraftApp, kind: Kind) -> Vec<PanelRow> {
    let mut rows = TextStylesPanel::panel_rows();
    rows.push(PanelRow::Separator);
    rows.extend(load_commands(kind).map(|id| PanelRow::Command(CommandRef::new(id))));
    rows
}

/// Publishes the load rows' state; the framework's rows publish their own.
pub(crate) fn menu_states(app: &VectorcraftApp, kind: Kind, states: &mut CommandStates) {
    let can = app.session.active().is_some() && file_asks::can(app, PickKind::Open);
    for id in load_commands(kind) {
        states.set(id, CommandState::new().enabled(can));
    }
}

/// Load Character Styles…, Load Paragraph Styles… and Load All Styles… (`ui.loadCharStyles`,
/// `ui.loadParaStyles`, `ui.loadAllStyles`): the engine command `load` (`charStyle.load`,
/// `paraStyle.load`, `textStyle.loadAll`) on the document at `path`, else on one picked in an open
/// dialog. A cancelled dialog loads nothing.
pub(crate) fn load(app: &mut VectorcraftApp, load: &str, path: Option<String>) -> Result<Value, String> {
    if let Some(path) = path {
        return app.run(load, json!({ "path": path }));
    }
    let request = fileio::builtin_open_filters().fold(PickRequest::open("textStyles"), |r, (name, exts)| r.filter(name, exts));
    let load = load.to_string();
    Ok(file_asks::ask(app, request, Owner::Document, move |app, picked| match file_asks::open_params(&picked) {
        Some(params) => app.run(&load, Value::Object(params)),
        None => Ok(Value::Null),
    }))
}

#[cfg(test)]
mod tests;
