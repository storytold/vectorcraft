//! Learn panel (Window › Learn, and Explore tutorials at the right end of the app bar): short
//! hands-on tutorials grouped by skill ([`catalog::SKILLS`]). The For you tab holds the Get started
//! section with the Quick skill guide card, which opens the Skills tab. The Skills tab lists each
//! skill with an empty progress ring and its count of tutorials; a skill lists its tutorials, and a
//! tutorial previews its numbered steps. A tool's name in a step is a link that shows where the tool
//! is ([`locate`]). Back goes up one level. `learn.list` lists the tutorials, `learn.show` opens one,
//! `learn.locateTool` shows where a tool is and `window.learn` shows or hides the panel.

pub mod catalog;
pub mod locate;

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
    let card = egui::Frame::NONE
        .fill(t.panel_darker)
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
    let resp = ui.interact(card.response.rect, ui.scope_id().with("quick-skill-guide"), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Quick skill guide")));
    if resp.clicked() {
        app.ui.learn = View { tab: Tab::Skills, skill: None, tutorial: None };
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

/// The Learn panel's UI commands: `window.learn`, `learn.list`, `learn.show` and `learn.locateTool`.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.learn" => toggle(app, p),
        "learn.list" => Ok(list()),
        "learn.show" => open(app, p),
        "learn.locateTool" => locate::locate_tool(app, p),
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

/// `learn.show {tutorial}`: show the panel on the Skills tab with that tutorial's steps → the
/// tutorial. A missing or unknown id is an error and changes nothing.
fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "tutorial";
    let id = match p.get(key) {
        None | Some(Value::Null) => return Err("missing `tutorial` (see learn.list)".into()),
        Some(Value::String(id)) => id.as_str(),
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    let (skill, tutorial) = catalog::tutorial(id).ok_or_else(|| format!("unknown tutorial `{id}` (see learn.list)"))?;
    crate::panel_docking::reveal_panel(app, ID)?;
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: Some(tutorial.id) };
    let steps: Vec<String> = tutorial.steps.iter().map(catalog::Step::plain).collect();
    Ok(json!({"skill": skill.id, "tutorial": tutorial.id, "title": tutorial.title, "steps": steps}))
}

#[cfg(test)]
mod tests;
