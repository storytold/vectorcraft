//! Learn panel (Window › Learn, and Explore tutorials at the right end of the app bar): short
//! hands-on tutorials grouped by skill ([`catalog::SKILLS`]). The For you tab holds the Get started
//! section with the Quick skill guide card, which opens the Skills tab. The Skills tab lists each
//! skill with an empty progress ring and its count of tutorials; a skill lists its tutorials, and a
//! tutorial previews its numbered steps. A tool's name in a step is a link that shows where the tool
//! is ([`locate`]). Back goes up one level. Under the steps, View more opens the tutorial's practice
//! sample as a new untitled document; Finish tutorial then marks it finished, and Next Up shows the
//! next tutorial in the same skill. `learn.list` lists the tutorials, `learn.skills` shows the list
//! of skills, `learn.show` opens one tutorial, `learn.locateTool` shows where a tool is,
//! `learn.openSample`, `learn.finish` and `learn.next` run the three practice controls, and
//! `window.learn` shows or hides the panel. The Home screen's Learn section shows the same card
//! ([`guide_card`]) and the tutorials, which open the panel.

pub mod catalog;
pub mod locate;
pub mod samples;

use egui::{Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::theme::Tokens;

pub const ID: &str = "learn";
/// The height of a skill or tutorial row.
const ROW_HEIGHT: f32 = 28.0;

/// The panel's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    ForYou,
    Skills,
}

/// What the panel shows: its tab and, on the Skills tab, the skill and the tutorial open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub tab: Tab,
    pub skill: Option<&'static str>,
    pub tutorial: Option<&'static str>,
}

/// The tutorials whose samples View more opened, and the tutorials finished, in the order they
/// were finished. Each id is listed once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Practice {
    pub opened: Vec<&'static str>,
    pub finished: Vec<&'static str>,
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for (tab, label) in [(Tab::ForYou, tl!("For you")), (Tab::Skills, tl!("Skills"))] {
            if ui.selectable_label(app.ui.learn.tab == tab, label).clicked() {
                app.ui.learn.tab = tab;
            }
        }
    });
    crate::widgets::divider(ui);
    match app.ui.learn.tab {
        Tab::ForYou => for_you(app, ui),
        Tab::Skills => skills(app, ui),
    }
}

/// The For you tab: Get started, with the Quick skill guide card.
fn for_you(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    crate::widgets::subheader(ui, tl!("Get started"));
    ui.add_space(4.0);
    guide_card(app, ui, ui.scope_id().with("quick-skill-guide"), t.panel_darker);
}

/// The Quick skill guide card, filled with `fill`, as one button with the id `id`: a click, or Enter
/// or Space while it has the keyboard focus, shows the panel's list of skills (`learn.skills`). The
/// For you tab and the Home screen's Learn section show it.
pub(crate) fn guide_card(app: &mut VectorcraftApp, ui: &mut Ui, id: egui::Id, fill: egui::Color32) {
    let t = Tokens::get(ui.ctx());
    let card = egui::Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(egui::CornerRadius::same(t.radius))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                crate::icons::icon(ui, "graduation-cap", 18.0, t.text_strong);
                ui.label(egui::RichText::new(tl!("Quick skill guide")).size(13.0).color(t.text_strong));
            });
            ui.add_space(4.0);
            ui.add(egui::Label::new(
                egui::RichText::new(tl!("Short hands-on tutorials for the essential tools, grouped by skill. Choose a skill to see its tutorials."))
                    .size(11.5)
                    .color(t.text_dim),
            ));
        });
    let resp = ui.interact(card.response.rect, id, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Quick skill guide")));
    if resp.has_focus() {
        ui.painter().rect_stroke(resp.rect, egui::CornerRadius::same(t.radius), Stroke::new(1.0, t.selection), egui::StrokeKind::Inside);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked()
        && let Err(e) = app.run("learn.skills", json!({}))
    {
        app.status(e);
    }
}

/// The Skills tab: the skills, a skill's tutorials, or a tutorial's steps.
fn skills(app: &mut VectorcraftApp, ui: &mut Ui) {
    let open = app.ui.learn.tutorial.and_then(catalog::tutorial);
    if let Some((skill, tutorial)) = open {
        if back(ui) {
            app.ui.learn.skill = Some(skill.id);
            app.ui.learn.tutorial = None;
        }
        steps(app, ui, tutorial);
        practice(app, ui, tutorial);
        return;
    }
    if let Some(skill) = app.ui.learn.skill.and_then(catalog::skill) {
        if back(ui) {
            app.ui.learn.skill = None;
        }
        crate::widgets::subheader(ui, skill.title);
        for tutorial in skill.tutorials {
            if row(ui, tutorial.title, None).clicked() {
                app.ui.learn.tutorial = Some(tutorial.id);
            }
        }
        return;
    }
    for skill in catalog::SKILLS {
        if row(ui, skill.title, Some(skill.tutorials.len())).clicked() {
            app.ui.learn.skill = Some(skill.id);
        }
    }
}

/// The Back button above a skill's tutorials and a tutorial's steps; true when it was clicked.
fn back(ui: &mut Ui) -> bool {
    crate::widgets::icon_button(ui, "chevron-left", tl!("Back"), false, 22.0).clicked()
}

/// A tutorial's title and its numbered steps. The tool's name in a step is a blue link: a click, or
/// Enter or Space while it has the keyboard focus, shows where the tool is (`learn.locateTool`).
fn steps(app: &mut VectorcraftApp, ui: &mut Ui, tutorial: &catalog::Tutorial) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tutorial.title).size(13.0).color(t.text_strong));
    ui.add_space(4.0);
    let mut locate = None;
    for (i, step) in tutorial.steps.iter().enumerate() {
        ui.horizontal_top(|ui| {
            ui.label(egui::RichText::new(format!("{}.", i + 1)).color(t.text_dim));
            let (Some(tool), Some((before, name, after))) = (step.tool, step.tool_name()) else {
                ui.add(egui::Label::new(egui::RichText::new(step.text).color(t.text)).wrap());
                return;
            };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if !before.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(before).color(t.text)).wrap());
                }
                if tool_link(ui, &t, name).clicked() {
                    locate = Some(tool);
                }
                if !after.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(after).color(t.text)).wrap());
                }
            });
        });
        ui.add_space(2.0);
    }
    if let Some(tool) = locate {
        if let Err(e) = app.run("learn.locateTool", json!({ "tool": tool })) {
            app.status(e);
        }
        ui.ctx().request_repaint();
    }
}

/// A tool's name in a step, drawn as a link: blue and underlined, with an outline while it has the
/// keyboard focus.
fn tool_link(ui: &mut Ui, t: &Tokens, name: &str) -> egui::Response {
    let resp = ui.add(egui::Label::new(egui::RichText::new(name).color(t.selection).underline()).sense(Sense::click()));
    if resp.has_focus() {
        ui.painter().rect_stroke(resp.rect.expand(2.0), egui::CornerRadius::same(2), Stroke::new(1.0, t.selection), egui::StrokeKind::Outside);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), name));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tl!("Show where this tool is"))
}

/// The controls under a tutorial's steps: View more; Finish tutorial once its sample is open; and
/// once it is finished, Next Up with the next tutorial's title, or for the skill's last tutorial a
/// line saying so and Back to skills.
fn practice(app: &mut VectorcraftApp, ui: &mut Ui, tutorial: &'static catalog::Tutorial) {
    let t = Tokens::get(ui.ctx());
    let id = tutorial.id;
    let params = json!({ "tutorial": id });
    let mut run = None;
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button(tl!("View more")).clicked() {
            run = Some(("learn.openSample", params.clone()));
        }
        let done = &app.ui.learn_practice;
        if done.opened.contains(&id) && !done.finished.contains(&id) && ui.button(tl!("Finish tutorial")).clicked() {
            run = Some(("learn.finish", params.clone()));
        }
    });
    if app.ui.learn_practice.finished.contains(&id) {
        ui.add_space(6.0);
        match catalog::next(id) {
            Some(next) => {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tl!("Next Up")).clicked() {
                        run = Some(("learn.next", params.clone()));
                    }
                    ui.add(egui::Label::new(egui::RichText::new(next.title).color(t.text)).wrap());
                });
            }
            None => {
                ui.add(egui::Label::new(egui::RichText::new(tl!("You finished the last tutorial in this skill.")).color(t.text_dim)).wrap());
                if ui.button(tl!("Back to skills")).clicked() {
                    run = Some(("learn.skills", json!({})));
                }
            }
        }
    }
    if let Some((cmd, params)) = run
        && let Err(e) = app.run(cmd, params)
    {
        app.status(e);
    }
}

/// A clickable row: `title`, and for a skill (`tutorials`) a progress ring before it and the
/// count of tutorials done of all after it. No progress is kept yet, so the ring is empty.
fn row(ui: &mut Ui, title: &str, tutorials: Option<usize>) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), t.hover);
    }
    let mut left = rect.left() + 6.0;
    let mut right = rect.right() - 6.0;
    let chevron = egui::Rect::from_center_size(egui::pos2(right - 6.0, rect.center().y), vec2(12.0, 12.0));
    crate::icons::paint(ui, "chevron-right", chevron, t.text_dim);
    right = chevron.left() - 6.0;
    if let Some(total) = tutorials {
        ui.painter().circle_stroke(egui::pos2(left + 8.0, rect.center().y), 7.0, Stroke::new(2.0, t.border));
        left += 24.0;
        let count = ui.painter().layout_no_wrap(format!("0/{total}"), egui::FontId::proportional(11.5), t.text_dim);
        right -= count.size().x;
        ui.painter().galley(egui::pos2(right, rect.center().y - count.size().y / 2.0), count, t.text_dim);
        right -= 6.0;
    }
    let name = ui.painter().layout(title.to_string(), egui::FontId::proportional(12.5), t.text, f32::INFINITY);
    let clip = egui::Rect::from_min_max(egui::pos2(left, rect.top()), egui::pos2(right.max(left), rect.bottom()));
    ui.painter().with_clip_rect(clip).galley(egui::pos2(left, rect.center().y - name.size().y / 2.0), name, t.text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), title));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The Learn panel's UI commands: `window.learn`, `learn.list`, `learn.skills`, `learn.show`,
/// `learn.locateTool`, `learn.openSample`, `learn.finish` and `learn.next`.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.learn" => toggle(app, p),
        "learn.list" => Ok(list()),
        "learn.skills" => skills_list(app),
        "learn.show" => open(app, p),
        "learn.locateTool" => locate::locate_tool(app, p),
        "learn.openSample" => open_sample(app, p),
        "learn.finish" => finish(app, p),
        "learn.next" => next(app, p),
        _ => return None,
    })
}

/// Whether the panel can be seen: it is the front panel, the dock is not hidden and no full screen
/// mode hides the panels.
pub(crate) fn visible(ui: &crate::state::UiState) -> bool {
    ui.dock && ui.screen_mode < 3 && crate::panel_docking::showing(ui, ID)
}

/// `window.learn {show?}`: show the panel (true), hide it (false) or toggle it (omitted), as the
/// app bar's Explore tutorials button and Window › Learn do → whether it shows ([`visible`]).
/// Showing it also shows a hidden dock; in a full screen mode it makes the panel the front one.
fn toggle(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let shown = visible(&app.ui);
    let show = crate::menus::opt_bool(p, "show")?.unwrap_or(!shown);
    if show && !shown {
        crate::panel_docking::reveal_panel(app, ID)?;
    } else if !show && shown {
        app.run("window.panel", json!({ "panel": ID, "toggle": true }))?;
    }
    Ok(json!(visible(&app.ui)))
}

/// `learn.list`: every skill with its tutorials and their step counts.
fn list() -> Value {
    let skills = catalog::SKILLS.iter().map(|s| {
        let tutorials: Vec<Value> = s.tutorials.iter().map(|t| json!({"id": t.id, "title": t.title, "steps": t.steps.len()})).collect();
        json!({"id": s.id, "title": s.title, "tutorials": tutorials})
    });
    json!({ "skills": skills.collect::<Vec<_>>() })
}

/// `learn.skills`: Back to skills, and the Quick skill guide card. Show the panel on the Skills tab
/// with the list of skills, with no skill and no tutorial open → what `learn.list` answers.
fn skills_list(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panel_docking::reveal_panel(app, ID)?;
    app.ui.learn = View { tab: Tab::Skills, skill: None, tutorial: None };
    Ok(list())
}

/// `learn.show {tutorial}`: show the panel on the Skills tab with that tutorial's steps → the
/// tutorial. A missing or unknown id is an error and changes nothing.
fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (skill, tutorial) = tutorial_param(p)?;
    crate::panel_docking::reveal_panel(app, ID)?;
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: Some(tutorial.id) };
    let steps: Vec<String> = tutorial.steps.iter().map(catalog::Step::plain).collect();
    Ok(json!({"skill": skill.id, "tutorial": tutorial.id, "title": tutorial.title, "steps": steps}))
}

/// The tutorial `p`'s `tutorial` names, with its skill. A missing, non-string or unknown id is an
/// error.
fn tutorial_param(p: &Value) -> Result<(&'static catalog::Skill, &'static catalog::Tutorial), String> {
    let key = "tutorial";
    let id = match p.get(key) {
        None | Some(Value::Null) => return Err("missing `tutorial` (see learn.list)".into()),
        Some(Value::String(id)) => id.as_str(),
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    catalog::tutorial(id).ok_or_else(|| format!("unknown tutorial `{id}` (see learn.list)"))
}

/// `learn.openSample {tutorial}`: View more. Open that tutorial's practice sample as a new untitled
/// document, which becomes the active one, and move the Learn view to the tutorial → `{tutorial,
/// index, title}` of the new document. Each call opens another copy; open documents stay as they
/// are. An id that is not valid, or a sample that does not read, is an error and changes nothing.
fn open_sample(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (skill, tutorial) = tutorial_param(p)?;
    let id = tutorial.id;
    let svg = samples::sample(id).ok_or_else(|| format!("tutorial `{id}` has no practice sample"))?;
    let name = format!("{id}.svg");
    let opened = vectorcraft_engine::cmd::fileio::open_template(&mut app.session, &name, svg.as_bytes(), None).map_err(|e| e.to_string())?;
    app.sync_views();
    if !app.ui.learn_practice.opened.contains(&id) {
        app.ui.learn_practice.opened.push(id);
    }
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: Some(id) };
    Ok(json!({"tutorial": id, "index": opened["index"], "title": opened["title"]}))
}

/// `learn.finish {tutorial}`: Finish tutorial. Mark a tutorial whose sample was opened finished →
/// `{tutorial, finished: true, next}`, where `next` is the id of the next tutorial in its skill, or
/// null after the skill's last one. Finishing it again keeps one entry. The Learn view stays as it is.
fn finish(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (_, tutorial) = tutorial_param(p)?;
    let id = tutorial.id;
    let practice = &mut app.ui.learn_practice;
    if !practice.opened.contains(&id) {
        return Err(format!("open the practice sample of `{id}` first (see learn.openSample)"));
    }
    if !practice.finished.contains(&id) {
        practice.finished.push(id);
    }
    Ok(json!({"tutorial": id, "finished": true, "next": catalog::next(id).map(|t| t.id)}))
}

/// `learn.next {tutorial}`: Next Up. Show the panel on the tutorial after a finished one in the
/// same skill → `{skill, tutorial, title}` of that tutorial. It opens no sample. An unfinished
/// tutorial, or a skill's last tutorial, is an error and changes nothing.
fn next(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (skill, tutorial) = tutorial_param(p)?;
    let id = tutorial.id;
    if !app.ui.learn_practice.finished.contains(&id) {
        return Err(format!("finish `{id}` first (see learn.finish)"));
    }
    let after = catalog::next(id).ok_or_else(|| format!("`{id}` is the last tutorial in its skill"))?;
    crate::panel_docking::reveal_panel(app, ID)?;
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: Some(after.id) };
    Ok(json!({"skill": skill.id, "tutorial": after.id, "title": after.title}))
}

#[cfg(test)]
mod tests;
