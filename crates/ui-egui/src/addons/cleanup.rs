//! Artwork Cleanup's panel and submenu (Object › Plug-ins › Artwork Cleanup). The panel runs the
//! Document Health Check on the selection, or the document while nothing is selected, lists what it
//! found, and shows or fixes the issues picked in the list (Fix All: every issue listed), each fix
//! one undo step. Below, the add-on's one-click tools open their options as their menu items do,
//! and its text tools, which the panel can hide, act on the selected type.

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::keymap::Mods;
use craft::menu::Node;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Divider, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::menus::{Sep, c, cp, sub};
use crate::panels::empty_state;

/// The panel's id (`window.panel {panel: "artworkCleanup"}`).
pub const ID: &str = "artworkCleanup";

/// The one-click tools: (menu label, command, the options their dialog opens with).
fn tools() -> [(&'static str, &'static str, Value); 3] {
    [
        ("Simplify Anchors (Keep Shape)…", "addon.cleanup.simplify", json!({"tolerance": 20, "protectCorners": false})),
        ("Rejoin Path Pieces…", "addon.cleanup.rejoin", json!({"gap": 0.1})),
        ("Deep Ungroup…", "addon.cleanup.deepUngroup", json!({"includeClipGroups": false})),
    ]
}

/// Replace Missing Fonts' options: the default type font.
fn replace_fonts() -> Value {
    json!({ "font": vectorcraft_doc::CharStyle::default().font_family })
}

/// A row of the panel's text tools: a label above (if any), then its buttons (label, what a
/// modifier click does, command, params).
type TextRow = (Option<&'static str>, Vec<(&'static str, Option<&'static str>, &'static str, Value)>);

fn text_rows() -> [TextRow; 5] {
    let align = |a: &str| json!({ "align": a });
    let by = |b: &str| json!({ "by": b });
    [
        (
            Some("Set Alignment, Keep Position:"),
            vec![
                ("Left", None, "addon.cleanup.alignInPlace", align("left")),
                ("Center", None, "addon.cleanup.alignInPlace", align("center")),
                ("Right", None, "addon.cleanup.alignInPlace", align("right")),
            ],
        ),
        (
            Some("Break Text into:"),
            vec![
                ("Lines", Some("Shift-click: break into paragraphs"), "addon.cleanup.breakText", by("lines")),
                ("Words", None, "addon.cleanup.breakText", by("words")),
                ("Glyphs", None, "addon.cleanup.breakText", by("glyphs")),
            ],
        ),
        (
            None,
            vec![
                ("Split at Cursor", None, "addon.cleanup.splitText", json!({})),
                ("Reset Transform", Some("Alt-click: reset the horizontal scale too"), "addon.cleanup.resetTextTransform", json!({})),
            ],
        ),
        (None, vec![("Combine Point Text", None, "addon.cleanup.combineText", json!({}))]),
        (None, vec![("Join Point Text", None, "addon.cleanup.joinText", json!({}))]),
    ]
}

/// The text tools one to a row below the others: (label, command, params).
fn text_singles() -> [(&'static str, &'static str, Value); 3] {
    [
        ("Remove Line Breaks", "addon.cleanup.removeBreaks", json!({ "kind": "line" })),
        ("Remove Paragraph Breaks", "addon.cleanup.removeBreaks", json!({ "kind": "paragraph" })),
        ("Replace Missing Fonts…", "addon.cleanup.replaceFonts", replace_fonts()),
    ]
}

/// `p` for a click with `held`: Shift-click on Lines breaks into paragraphs, Alt-click on Reset
/// Transform resets the horizontal scale too.
fn modified(id: &str, mut p: Value, held: Mods) -> Value {
    match id {
        "addon.cleanup.breakText" if held.shift && p["by"] == "lines" => p["by"] = json!("paragraphs"),
        "addon.cleanup.resetTextTransform" if held.alt => p["scale"] = json!(true),
        _ => {}
    }
    p
}

/// Object › Plug-ins › Artwork Cleanup.
pub fn menu() -> Vec<Node> {
    let mut items = vec![cp("Document Health Check", "window.panel", json!({ "panel": ID })), Sep];
    items.extend(tools().into_iter().map(|(label, id, p)| cp(label, id, p)));
    items.extend([Sep, c("Fix All Issues", "addon.cleanup.fix"), Sep]);
    let by = |b: &str| json!({ "by": b });
    let align = |a: &str| json!({ "align": a });
    items.extend([
        cp("Combine Point Text", "addon.cleanup.combineText", json!({})),
        cp("Join Point Text", "addon.cleanup.joinText", json!({})),
        cp("Break Text into Lines", "addon.cleanup.breakText", by("lines")),
        cp("Break Text into Paragraphs", "addon.cleanup.breakText", by("paragraphs")),
        cp("Break Text into Words", "addon.cleanup.breakText", by("words")),
        cp("Break Text into Glyphs", "addon.cleanup.breakText", by("glyphs")),
        cp("Split Text at Cursor", "addon.cleanup.splitText", json!({})),
        sub(
            "Set Alignment, Keep Position",
            vec![
                cp("Left", "addon.cleanup.alignInPlace", align("left")),
                cp("Center", "addon.cleanup.alignInPlace", align("center")),
                cp("Right", "addon.cleanup.alignInPlace", align("right")),
            ],
        ),
        cp("Reset Text Transform", "addon.cleanup.resetTextTransform", json!({})),
        cp("Reset Text Transform and Scale", "addon.cleanup.resetTextTransform", json!({ "scale": true })),
    ]);
    items.extend(text_singles().into_iter().map(|(label, id, p)| cp(label, id, p)));
    items
}

/// The panel's last check, kept at `app.addon_panels.cleanup` until the next check.
#[derive(Clone, Default)]
pub(crate) struct Results {
    /// The scope it looked at (`selection` or `document`).
    scope: String,
    /// The uid of the document it looked at; another document's panel does not show it.
    doc: u64,
    /// The revision of that document it looked at.
    revision: u64,
    /// What it found: (kind, label, count).
    rows: Vec<(String, String, u64)>,
    /// The rows picked in the list, by position in `rows`.
    picked: Selection<usize>,
    /// What the last fix did.
    status: String,
}

impl Results {
    /// An empty result for `scope` of the active document as it is now.
    fn blank(app: &VectorcraftApp, scope: &str) -> Results {
        let (doc, revision) = app.session.active().map_or((0, 0), |st| (st.uid, st.revision));
        Results { scope: scope.to_string(), doc, revision, ..Results::default() }
    }

    /// The kinds of the picked rows.
    fn picked_kinds(&self) -> Vec<String> {
        self.picked.items.iter().filter_map(|&i| self.rows.get(i)).map(|row| row.0.clone()).collect()
    }
}

/// A slice of rows as the items of a list keyed by position, each named by `label`.
pub(crate) struct Positional<'a, T> {
    /// The rows, in display order.
    pub rows: &'a [T],
    /// The text of a row, which the list matches typed letters against.
    pub label: fn(&T) -> &str,
}

impl<T> DisplayOrder<usize> for Positional<'_, T> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.rows.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.rows.get(at).map(|row| std::borrow::Cow::Borrowed((self.label)(row)))
    }
}

impl<T> ItemSource<usize> for Positional<'_, T> {}

/// The panel's results while they belong to the active document.
fn current(app: &VectorcraftApp) -> Option<&Results> {
    let uid = app.session.active()?.uid;
    app.addon_panels.cleanup.as_ref().filter(|r| r.doc == uid)
}

/// Run the check (in `scope`, or the default one) and keep its results for the panel.
fn check(app: &mut VectorcraftApp, scope: Option<&str>) {
    let p = scope.map_or_else(|| json!({}), |s| json!({ "scope": s }));
    match app.session.execute("addon.cleanup.check", &p) {
        Ok(v) => {
            let mut r = Results::blank(app, v["scope"].as_str().unwrap_or("document"));
            let row = |f: &Value| Some((f["kind"].as_str()?.to_string(), f["label"].as_str()?.to_string(), f["count"].as_u64()?));
            r.rows = v["findings"].as_array().into_iter().flatten().filter_map(row).collect();
            app.addon_panels.cleanup = Some(r);
        }
        Err(e) => app.status(e.to_string()),
    }
}

/// Fix the issues of `kinds` in the last check's scope, then check that scope again.
fn fix(app: &mut VectorcraftApp, kinds: Vec<String>) {
    let Some(scope) = current(app).map(|r| r.scope.clone()) else { return };
    match app.run("addon.cleanup.fix", json!({ "scope": scope, "kinds": kinds })) {
        Ok(v) => {
            let fixed = v["total"].as_u64().unwrap_or(0).to_string();
            // A selection fixed away leaves nothing to check again: nothing is left.
            if scope == "selection" && app.session.active().is_some_and(|d| d.selection.is_empty()) {
                app.addon_panels.cleanup = Some(Results::blank(app, &scope));
            } else {
                check(app, Some(&scope));
            }
            if let Some(r) = app.addon_panels.cleanup.as_mut() {
                r.status = crate::i18n::fmt(tl!("Issues fixed: {count}"), &[("count", &fixed)]);
            }
        }
        Err(e) => app.status(e),
    }
}

/// What the panel's buttons asked for this frame.
enum Action {
    Check,
    Show(String),
    Fix(Vec<String>),
    /// A tool's button: (its label, command, params), run as its menu item is.
    Run(&'static str, &'static str, Value),
}

/// A push button named `label`: the modifiers held while it was clicked, `None` when it was not.
fn push(ui: &mut Ui, label: &str, enabled: bool, tip: Option<&str>) -> Option<Mods> {
    let mut button = Button::new(label.to_string()).enabled(enabled);
    if let Some(tip) = tip {
        button = button.tooltip(tip.to_string());
    }
    let out = button.show(ui);
    out.clicked().then_some(out.mods)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((selected, revision, uid)) = app.session.active().map(|st| (!st.selection.is_empty(), st.revision, st.uid)) else {
        empty_state(ui, "sparkles", tl!("No document"), tl!("Open a document to check it for issues."));
        return;
    };
    // The results of another document are dropped, not kept for its return.
    if app.addon_panels.cleanup.as_ref().is_some_and(|r| r.doc != uid) {
        app.addon_panels.cleanup = None;
    }
    let mut action = None;
    if push(ui, if selected { tl!("Check Selection") } else { tl!("Check Document") }, true, None).is_some() {
        action = Some(Action::Check);
    }
    if let Some(r) = app.addon_panels.cleanup.as_mut() {
        results(ui, r, &mut action);
        if r.revision != revision {
            Text::new(tl!("Changed since the last check.").to_string()).show(ui);
        }
    }
    Divider::horizontal().show(ui);
    for (label, id, p) in tools() {
        if push(ui, tl!(label), selected, None).is_some() {
            action = Some(Action::Run(label, id, p));
        }
    }
    let editing = app.session.tool_options().get("editing").is_some_and(|v| !v.is_null());
    Section::new(tl!("Text").to_string()).show(ui, |ui, _| text_tools(ui, selected, editing, &mut action));
    match action {
        Some(Action::Check) => check(app, None),
        Some(Action::Show(kind)) => {
            let scope = current(app).map(|r| r.scope.clone()).unwrap_or_default();
            if let Err(e) = app.run("addon.cleanup.show", json!({ "kind": kind, "scope": scope })) {
                app.status(e);
            }
        }
        Some(Action::Fix(kinds)) => fix(app, kinds),
        Some(Action::Run(label, id, p)) => {
            let (id, p) = crate::menus::click_target(label, id, &p);
            crate::menus::invoke(app, &id, p);
        }
        None => {}
    }
}

/// The text tools: rows of buttons, then one to a row. Each is enabled when its command can act:
/// Split at Cursor while a Type tool edits text, Replace Missing Fonts always, the others with a
/// selection.
fn text_tools(ui: &mut Ui, selected: bool, editing: bool, action: &mut Option<Action>) {
    let enabled = |id: &str| match id {
        "addon.cleanup.splitText" => editing,
        "addon.cleanup.replaceFonts" => true,
        _ => selected,
    };
    let mut button = |ui: &mut Ui, label: &'static str, tip: Option<&str>, id: &'static str, p: Value| {
        // Lines and words of text, not lines drawn: their own catalog rows (the button translates
        // the others).
        let shown = if id == "addon.cleanup.breakText" { crate::i18n::tr_ctx(crate::i18n::current(), "breakText", label) } else { tl!(label) };
        if let Some(mods) = push(ui, shown, enabled(id), tip.map(|t| tl!(t))) {
            *action = Some(Action::Run(label, id, modified(id, p, mods)));
        }
    };
    for (heading, buttons) in text_rows() {
        if let Some(h) = heading {
            Text::new(tl!(h).to_string()).show(ui);
        }
        Row::new().show(ui, |ui, _| {
            for (label, tip, id, p) in buttons.clone() {
                button(ui, label, tip, id, p);
            }
        });
    }
    for (label, id, p) in text_singles() {
        button(ui, label, None, id, p);
    }
}

/// The list of what the check found (click picks a row, Ctrl/Cmd- or Shift-click adds one), the
/// last fix's status, and Show, Fix Selected and Fix All.
fn results(ui: &mut Ui, r: &mut Results, action: &mut Option<Action>) {
    let source = Positional { rows: &r.rows, label: |row| row.1.as_str() };
    List::<usize>::new(tl!("Issues").to_string()).empty(tl!("No issues found.").to_string()).selection(Bind::bound(&mut r.picked)).show_items(
        ui,
        &source,
        &mut |key, _| match r.rows.get(*key) {
            Some((_, label, count)) => ListRow::new(tl!(label).to_string()).trailing(count.to_string()),
            None => ListRow::spacer(),
        },
    );
    if !r.status.is_empty() {
        Text::new(r.status.clone()).show(ui);
    }
    Row::new().show(ui, |ui, _| {
        let picked = r.picked_kinds();
        let show = push(ui, tl!("Show"), picked.len() == 1, None).is_some();
        if show && let [kind] = picked.as_slice() {
            *action = Some(Action::Show(kind.clone()));
        }
        if push(ui, tl!("Fix Selected"), !picked.is_empty(), None).is_some() {
            *action = Some(Action::Fix(picked));
        }
        if push(ui, tl!("Fix All"), !r.rows.is_empty(), None).is_some() {
            *action = Some(Action::Fix(r.rows.iter().map(|row| row.0.clone()).collect()));
        }
    });
}

#[cfg(test)]
mod tests;
