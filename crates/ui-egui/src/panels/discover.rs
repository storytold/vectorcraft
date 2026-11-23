//! Help searches the tools, menu commands, and built-in tutorials.

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::keymap::{Chord, CommandRef};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Kbd, Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::menu::search::{HelpEntry, SearchIndex, SearchWords, Synonyms, rank};
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use egui::Ui;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{VectorcraftApp, menus};

pub const ID: &str = "discover";
const MAX_QUERY: usize = 1024;
/// The sections of the results, in the order they show.
const SECTIONS: [&str; 3] = ["tool", "command", "tutorial"];

#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    pub kind: &'static str,
    pub id: String,
    pub label: String,
    pub path: Vec<String>,
    pub shortcut: String,
    /// The English label and menu path that `label` and `path` translate, and a command's menu
    /// params, kept to bring a retained result up to date ([`refresh`]).
    #[serde(skip)]
    english_label: String,
    #[serde(skip)]
    english_path: Vec<String>,
    #[serde(skip)]
    params: Value,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub query: String,
    pub results: Vec<Hit>,
    pub selected: Option<usize>,
    pub focus: bool,
    /// The menu state ([`menus::model::state_key`]: commands run, shortcuts, the language,
    /// recent files and more) the results were last brought up to date for.
    current_for: Option<u64>,
}

/// The text of a tool's shortcut in the window's keymap, empty when it has none.
fn tool_shortcut(app: &VectorcraftApp, id: &str) -> String {
    menus::shortcut_of(app, &format!("tool:{id}")).map_or_else(String::new, |chord| chord.display(crate::keymap::platform()))
}

/// Updates the retained results' labels, menu paths and shortcuts after the menu state changes (a
/// command, a shortcut or the UI language), keeping the query, the results' order and the selected
/// result. A command result takes its menu item's current label and place (Show Grid becomes Hide
/// Grid), and leaves the results when no menu lists it any more, such as a recent file that was
/// removed.
fn refresh(app: &mut VectorcraftApp) {
    let stamp = menus::model::state_key(app, None);
    if app.ui.discover.current_for == Some(stamp) {
        return;
    }
    let lang = crate::i18n::current();
    let entries = if app.ui.discover.results.iter().any(|hit| hit.kind == "command") { menus::menu_entries(app) } else { Vec::new() };
    let mut by_command: std::collections::HashMap<(&str, String), Vec<&menus::MenuEntry>> = std::collections::HashMap::new();
    for entry in &entries {
        if let Some(id) = entry.command.as_deref() {
            by_command.entry((id, entry.params.to_string())).or_default().push(entry);
        }
    }
    let selected = app.ui.discover.selected.take();
    let mut kept = Vec::with_capacity(app.ui.discover.results.len());
    for (index, mut hit) in std::mem::take(&mut app.ui.discover.results).into_iter().enumerate() {
        match hit.kind {
            "tool" => {
                hit.label = crate::i18n::tr(lang, &hit.english_label).into();
                hit.shortcut = tool_shortcut(app, &hit.id);
            }
            "command" => {
                let same = by_command.get(&(hit.id.as_str(), hit.params.to_string())).map_or(&[][..], Vec::as_slice);
                let Some(entry) = same.iter().find(|e| e.path == hit.english_path).or_else(|| same.first()) else {
                    continue;
                };
                hit.english_label = entry.label.clone();
                hit.english_path = entry.path.clone();
                hit.label = menus::shown_label(app, &hit.id, &hit.params, &hit.english_label);
                hit.path = entry.shown_path.clone();
                hit.shortcut = menus::item_shortcut(app, &hit.id, &hit.params)
                    .or_else(|| menus::shortcut_of(app, &hit.id))
                    .map(|c| c.to_string())
                    .unwrap_or_default()
                    .into();
            }
            _ => {}
        }
        if selected == Some(index) {
            app.ui.discover.selected = Some(kept.len());
        }
        kept.push(hit);
    }
    app.ui.discover.results = kept;
    app.ui.discover.current_for = Some(stamp);
}

/// The tools, each with the entry the search ranks it by.
fn tool_hits(app: &VectorcraftApp) -> Vec<(HelpEntry, Hit)> {
    let lang = crate::i18n::current();
    vectorcraft_tools::catalog::all_tools()
        .into_iter()
        .map(|tool| {
            let label: String = crate::i18n::tr(lang, tool.label).into();
            let entry = HelpEntry::new(CommandRef::with_params("learn.locateTool", json!({"tool": tool.id})), label.clone())
                .with_english(tool.label)
                .with_words(tool.id);
            let hit = Hit {
                kind: "tool",
                id: tool.id.into(),
                label,
                path: vec![],
                shortcut: tool_shortcut(app, tool.id),
                english_label: tool.label.into(),
                english_path: vec![],
                params: Value::Null,
            };
            (entry, hit)
        })
        .collect()
}

/// The menu commands, each with the entry the search ranks it by.
fn command_hits(app: &VectorcraftApp) -> Vec<(HelpEntry, Hit)> {
    let mut hits = Vec::new();
    for entry in menus::menu_entries(app) {
        let Some(id) = entry.command else { continue };
        // The menu bar in the window has no VectorCraft menu; only the native macOS bar shows it.
        if !cfg!(target_os = "macos") && entry.path.first().is_some_and(|title| title == "VectorCraft") {
            continue;
        }
        let label = menus::shown_label(app, &id, &entry.params, &entry.label);
        let help = HelpEntry::new(CommandRef::with_params(id.clone(), entry.params.clone()), label.clone())
            .with_english(entry.label.clone())
            .with_path(entry.shown_path.clone())
            .with_words(format!("{} {id}", entry.path.join(" ")));
        let hit = Hit {
            kind: "command",
            id,
            label,
            path: entry.shown_path,
            shortcut: entry.shortcut,
            english_label: entry.label,
            english_path: entry.path,
            params: entry.params,
        };
        hits.push((help, hit));
    }
    hits
}

/// The tutorials, each with the entry the search ranks it by.
fn tutorial_hits() -> Vec<(HelpEntry, Hit)> {
    let mut hits = Vec::new();
    for skill in super::learn::catalog::SKILLS {
        for tutorial in skill.tutorials {
            let entry = HelpEntry::new(CommandRef::with_params("learn.show", json!({"tutorial": tutorial.id})), tutorial.title)
                .with_path([skill.title])
                .with_words(tutorial.id);
            let hit = Hit {
                kind: "tutorial",
                id: tutorial.id.into(),
                label: tutorial.title.into(),
                path: vec![skill.title.into()],
                shortcut: String::new(),
                english_label: tutorial.title.into(),
                english_path: vec![skill.title.into()],
                params: Value::Null,
            };
            hits.push((entry, hit));
        }
    }
    hits
}

fn search(app: &VectorcraftApp, query: &str) -> Vec<Hit> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    // A command runs outside a frame: the index reads the interface language from the host the app
    // gives the window.
    let host = &app.host;
    let synonyms = Synonyms::of(&host.words().set::<SearchWords>());
    let mut hits = Vec::new();
    for section in [tool_hits(app), command_hits(app), tutorial_hits()] {
        let (entries, candidates): (Vec<HelpEntry>, Vec<Hit>) = section.into_iter().unzip();
        let index = SearchIndex::for_host(host, &entries, &synonyms);
        hits.extend(rank(&index, query, &[], &|_| 0).into_iter().filter_map(|at| candidates.get(at).cloned()));
    }
    hits
}

fn results(app: &VectorcraftApp) -> Value {
    json!({"query": app.ui.discover.query, "results": app.ui.discover.results, "selected": app.ui.discover.selected})
}

fn set_query(app: &mut VectorcraftApp, query: &str) {
    app.ui.discover.current_for = Some(menus::model::state_key(app, None));
    app.ui.discover.results = search(app, query);
    app.ui.discover.query = query.into();
    app.ui.discover.selected = None;
}

pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let spec = menus::UI_COMMANDS.iter().find(|spec| spec.0 == id && id.starts_with("help.discover"))?;
    if !(p.is_object() || p.is_null()) {
        return Some(Err("`params` must be an object".into()));
    }
    if let Err(e) = vectorcraft_engine::cmd::params::check_keys(id, spec.3, p) {
        return Some(Err(e.to_string()));
    }
    refresh(app);
    Some(match id {
        "help.discover" | "help.discover.search" => {
            let query = match p.get("query") {
                Some(Value::String(query)) => Some(query.as_str()),
                None if id == "help.discover" => None,
                _ => {
                    let key = "query";
                    return Some(Err(format!("`{key}` must be a string")));
                }
            };
            if let Some(query) = query {
                if query.chars().take(MAX_QUERY + 1).count() > MAX_QUERY {
                    let label = "query";
                    let max = MAX_QUERY;
                    return Some(Err(format!("{label} must be at most {max} characters")));
                }
                set_query(app, query);
            }
            app.ui.discover.focus = true;
            crate::panel_docking::reveal_panel(app, ID).map(|_| results(app))
        }
        "help.discover.choose" => {
            let hit = p
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| app.ui.discover.results.get(i).cloned().map(|hit| (i, hit)));
            let Some((index, hit)) = hit else {
                return Some(Err("index out of range".into()));
            };
            let outcome = match hit.kind {
                "tool" => app.run("learn.locateTool", json!({"tool": hit.id})),
                "tutorial" => Ok(json!({"tutorial": hit.id})),
                _ => Ok(json!({"command": hit.id, "path": hit.path, "shortcut": hit.shortcut})),
            };
            outcome.and_then(|outcome| {
                crate::panel_docking::reveal_panel(app, ID)?;
                app.ui.discover.selected = Some(index);
                app.ui.discover.focus = true;
                Ok(json!({"result": hit, "location": outcome}))
            })
        }
        "help.discover.back" => {
            app.ui.discover.selected = None;
            app.ui.discover.focus = true;
            Ok(results(app))
        }
        _ => return None,
    })
}

/// One row of the results list: a section heading, or a result by its position in `View::results`.
#[derive(Clone, Copy)]
enum ResultRow {
    Heading(&'static str),
    Hit(usize),
}

/// The results list's rows: each section that has results, under its heading.
struct ResultRows(Vec<ResultRow>);

impl ResultRows {
    fn of(results: &[Hit]) -> Self {
        let mut rows = Vec::with_capacity(results.len() + SECTIONS.len());
        for kind in SECTIONS {
            let hits: Vec<usize> = results.iter().enumerate().filter(|(_, hit)| hit.kind == kind).map(|(index, _)| index).collect();
            if !hits.is_empty() {
                rows.push(ResultRow::Heading(kind));
                rows.extend(hits.into_iter().map(ResultRow::Hit));
            }
        }
        ResultRows(rows)
    }
}

impl DisplayOrder<usize> for ResultRows {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.0.get(at), Some(ResultRow::Heading(_)))
    }
}

impl ItemSource<usize> for ResultRows {}

fn section_title(kind: &str) -> String {
    match kind {
        "tool" => tl!("Tools"),
        "command" => tl!("Search Commands…"),
        _ => tl!("Learn"),
    }
    .to_string()
}

/// The chord of a hit's shortcut text; text that is not a chord shows as written.
fn shortcut_kbd(shortcut: &str) -> Kbd {
    Chord::parse(shortcut).map_or_else(|_| Kbd::new(shortcut.to_owned()), Kbd::chord)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    refresh(app);
    let mut query = app.ui.discover.query.clone();
    let field = TextField::search(tl!("Search").to_string(), Bind::bound(&mut query)).limit(MAX_QUERY).show(ui);
    // A measuring pass (a floating panel's first) moves no focus: the request waits for a pass that takes it.
    if app.ui.discover.focus && focus::request(ui.ctx(), FocusTarget::Id(field.response.id)) {
        app.ui.discover.focus = false;
    }
    // Escape closes the panel and keeps the query the search field clears with it, so Help opens on it again.
    if field.escaped {
        if let Err(e) = app.run("window.panel.close", json!({"panel": ID})) {
            app.status(e);
        }
        return;
    }
    if query != app.ui.discover.query
        && let Err(e) = app.run("help.discover.search", json!({"query": query}))
    {
        app.status(e);
    }
    let mut chosen = None;
    let mut home = false;
    let mut back = false;
    let mut locate = None;
    Row::new().show(ui, |ui, _| {
        home = Button::new(tl!("Home").to_string()).show(ui).clicks == 1;
        if app.ui.discover.selected.is_some() {
            back = Button::new(tl!("Back").to_string()).show(ui).clicks == 1;
        }
    });
    if let Some(hit) = app.ui.discover.selected.and_then(|i| app.ui.discover.results.get(i)) {
        Text::new(hit.label.clone()).role(TypeRole::BodyStrong).show(ui);
        if !hit.path.is_empty() {
            Text::new(hit.path.join(" › ")).shade(Shade::Muted).show(ui);
        }
        if !hit.shortcut.is_empty() {
            shortcut_kbd(&hit.shortcut).show(ui);
        }
        if hit.kind == "tutorial"
            && let Some((_, tutorial)) = super::learn::catalog::tutorial(&hit.id)
        {
            for (index, step) in tutorial.steps.iter().enumerate() {
                Row::new().show(ui, |ui, _| {
                    Text::new(format!("{}.", index + 1)).show(ui);
                    if super::learn::step_text(ui, step) {
                        locate = step.tool;
                    }
                });
            }
        }
    } else {
        if app.ui.discover.results.is_empty() && !query.trim().is_empty() {
            Text::new(tl!("No results").to_string()).show(ui);
        }
        let rows = ResultRows::of(&app.ui.discover.results);
        let hits = &app.ui.discover.results;
        let mut selection = Selection::default();
        List::new(tl!("Search").to_string())
            .id_salt("discover-results")
            .select(SelectMode::Single)
            .selection(Bind::bound(&mut selection))
            .show_items(ui, &rows, &mut |at, _| match rows.0.get(*at) {
                Some(ResultRow::Heading(kind)) => ListRow::new(section_title(kind)).section_heading(),
                Some(ResultRow::Hit(index)) => match hits.get(*index) {
                    Some(hit) => ListRow::new(hit.label.clone()).tooltip(hit.path.join(" › ")),
                    None => ListRow::spacer(),
                },
                None => ListRow::spacer(),
            });
        chosen = selection.items.first().and_then(|at| match rows.0.get(*at) {
            Some(ResultRow::Hit(index)) => Some(*index),
            _ => None,
        });
    }
    if home {
        if let Err(e) = app.run("help.discover.search", json!({"query": ""})) {
            app.status(e);
        }
    } else if back {
        if let Err(e) = app.run("help.discover.back", json!({})) {
            app.status(e);
        }
    } else if let Some(index) = chosen {
        match app.run("help.discover.choose", json!({"index": index})) {
            Ok(picked) => status_reason(app, &picked["location"]),
            Err(e) => app.status(e),
        }
    }
    if let Some(tool) = locate {
        match app.run("learn.locateTool", json!({"tool": tool})) {
            Ok(located) => status_reason(app, &located),
            Err(e) => app.status(e),
        }
    }
}

/// A `learn.locateTool` reply that could not mark its tool (`location: "unavailable"`) shows its
/// `reason` in the status bar.
fn status_reason(app: &mut VectorcraftApp, located: &Value) {
    if let Some(reason) = located.get("reason").and_then(Value::as_str) {
        app.status(reason);
    }
}
