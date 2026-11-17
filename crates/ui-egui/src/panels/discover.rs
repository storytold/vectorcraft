//! Help searches the tools, menu commands, and built-in tutorials.

use egui::Ui;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{VectorcraftApp, menus};

pub const ID: &str = "discover";
const MAX_QUERY: usize = 1024;

#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    pub kind: &'static str,
    pub id: String,
    pub label: String,
    pub path: Vec<String>,
    pub shortcut: String,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub query: String,
    pub results: Vec<Hit>,
    pub selected: Option<usize>,
    pub focus: bool,
}

fn matches(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    query.split_whitespace().all(|word| text.contains(word))
}

fn search(app: &VectorcraftApp, query: &str) -> Vec<Hit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let lang = crate::i18n::current();
    for tool in vectorcraft_tools::catalog::all_tools() {
        let label = crate::i18n::tr(lang, tool.label);
        if matches(&query, &format!("{} {label} {}", tool.label, tool.id)) {
            hits.push(Hit {
                kind: "tool",
                id: tool.id.into(),
                label: label.into(),
                path: vec![],
                shortcut: crate::shortcut_editor::tool_shortcut(tool.id).unwrap_or_default().into(),
            });
        }
    }
    for entry in menus::menu_entries(app) {
        let Some(id) = entry.command else { continue };
        let label = crate::i18n::tr(lang, &entry.label);
        let path: Vec<String> = entry.path.iter().map(|part| crate::i18n::tr(lang, part).into()).collect();
        if matches(&query, &format!("{} {label} {} {} {id}", entry.label, entry.path.join(" "), path.join(" "))) {
            hits.push(Hit { kind: "command", id, label: label.into(), path, shortcut: entry.shortcut });
        }
    }
    for skill in super::learn::catalog::SKILLS {
        for tutorial in skill.tutorials {
            if matches(&query, &format!("{} {} {}", skill.title, tutorial.title, tutorial.id)) {
                hits.push(Hit {
                    kind: "tutorial",
                    id: tutorial.id.into(),
                    label: tutorial.title.into(),
                    path: vec![skill.title.into()],
                    shortcut: String::new(),
                });
            }
        }
    }
    hits
}

fn results(app: &VectorcraftApp) -> Value {
    json!({"query": app.ui.discover.query, "results": app.ui.discover.results, "selected": app.ui.discover.selected})
}

fn set_query(app: &mut VectorcraftApp, query: &str) {
    app.ui.discover.results = search(app, query);
    app.ui.discover.query = query.into();
    app.ui.discover.selected = None;
}

pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let spec = menus::UI_COMMANDS.iter().find(|spec| spec.0 == id && id.starts_with("help.discover"))?;
    if let Err(e) = vectorcraft_engine::cmd::params::check_keys(id, spec.3, p) {
        return Some(Err(e.to_string()));
    }
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

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mut query = app.ui.discover.query.clone();
    let field = ui.add(egui::TextEdit::singleline(&mut query).char_limit(MAX_QUERY).hint_text(tl!("Search")).desired_width(f32::INFINITY));
    if std::mem::take(&mut app.ui.discover.focus) {
        field.request_focus();
    }
    if field.changed()
        && let Err(e) = app.run("help.discover.search", json!({"query": query}))
    {
        app.status(e);
    }
    if (field.has_focus() || field.lost_focus())
        && !app.session.tool_busy()
        && !app.session.tool_composing()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        if let Err(e) = app.run("window.panel.close", json!({"panel": ID})) {
            app.status(e);
        }
        return;
    }
    let mut chosen = None;
    let mut home = false;
    let mut back = false;
    let mut locate = None;
    ui.horizontal(|ui| {
        home = ui.button(tl!("Home")).clicked();
        if app.ui.discover.selected.is_some() {
            back = ui.button(tl!("Back")).clicked();
        }
    });
    crate::widgets::divider(ui);
    if let Some(hit) = app.ui.discover.selected.and_then(|i| app.ui.discover.results.get(i)) {
        ui.heading(&hit.label);
        if !hit.path.is_empty() {
            ui.label(hit.path.join(" › "));
        }
        if !hit.shortcut.is_empty() {
            ui.label(menus::pretty_shortcut(&hit.shortcut));
        }
        if hit.kind == "tutorial"
            && let Some((_, tutorial)) = super::learn::catalog::tutorial(&hit.id)
        {
            for (index, step) in tutorial.steps.iter().enumerate() {
                ui.label(format!("{}. {}", index + 1, step.plain()));
                if let Some(tool) = step.tool
                    && let Some(info) = vectorcraft_tools::tool_info(tool)
                    && ui.link(crate::i18n::tr(crate::i18n::current(), info.label)).clicked()
                {
                    locate = Some(tool);
                }
            }
        }
    } else {
        egui::ScrollArea::vertical().id_salt("discover-results").show(ui, |ui| {
            if app.ui.discover.results.is_empty() && !query.trim().is_empty() {
                ui.label(tl!("No results"));
            }
            for kind in ["tool", "command", "tutorial"] {
                let label = match kind {
                    "tool" => tl!("Tools"),
                    "command" => tl!("Search Commands…"),
                    _ => tl!("Learn"),
                };
                let mut heading = false;
                for (index, hit) in app.ui.discover.results.iter().enumerate().filter(|(_, h)| h.kind == kind) {
                    if !heading {
                        crate::widgets::subheader(ui, label);
                        heading = true;
                    }
                    if ui.selectable_label(false, &hit.label).on_hover_text(hit.path.join(" › ")).clicked() {
                        chosen = Some(index);
                    }
                }
            }
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
    } else if let Some(index) = chosen
        && let Err(e) = app.run("help.discover.choose", json!({"index": index}))
    {
        app.status(e);
    }
    if let Some(tool) = locate
        && let Err(e) = app.run("learn.locateTool", json!({"tool": tool}))
    {
        app.status(e);
    }
}
