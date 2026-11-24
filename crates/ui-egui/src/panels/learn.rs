//! Learn panel (Window › Learn, and Explore tutorials at the right end of the app bar): short
//! hands-on tutorials grouped by skill ([`catalog::SKILLS`]). The For you tab holds the Get started
//! section with the Quick skill guide card, which opens the Skills tab. The Skills tab lists each
//! skill with a progress ring that fills as its tutorials are finished and the count of tutorials
//! finished of all; a skill lists its tutorials, each finished one with a check mark, and a
//! tutorial previews its numbered steps. A tool's name in a step is a link that shows where the tool
//! is ([`locate`]). Back goes up one level. Under the steps, View more opens the tutorial's practice
//! sample as a new untitled document; the check box before each step then marks it done, Finish
//! tutorial marks the tutorial finished with all its steps, and Next Up shows the next tutorial in
//! the same skill. The preferences keep the progress ([`progress`]); Reset progress under the
//! skills clears it. `learn.list` lists the tutorials with their progress, `learn.tab` chooses a
//! tab, `learn.skills` shows the list of skills, `learn.skill` opens one skill, `learn.show` opens
//! one tutorial, `learn.back` goes up one level, `learn.locateTool` shows where a tool is,
//! `learn.openSample`, `learn.markStep`, `learn.finish` and `learn.next` run the practice controls,
//! `learn.resetProgress` clears the progress, and `window.learn` shows or hides the panel. The Home
//! screen's Learn section shows the same card ([`guide_card`]) and the tutorials, which open the
//! panel.

pub mod catalog;
pub mod locate;
pub mod progress;
pub mod samples;

use craft::behavior::{IconId, SelectionChange, keyed};
use craft::ui::buttons::{Button, IconButton, Link};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, Gap, LabelKind, MemoProps, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, SideLabel, Sizing,
    clicked,
};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::primitives::{Glyph, HAlign, IconSpec};
use craft::ui::style::{TypeRole, UiTokens, rgba_color};
use craft::ui::tabs::{Tab as TabItem, TabKind, TabStrip};
use egui::{Color32, Painter, Rect, Shape, Stroke, Ui, vec2};
use serde_json::{Value, json};

use crate::VectorcraftApp;

pub const ID: &str = "learn";

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

/// The tutorials whose samples View more opened while the app runs. Each id is listed once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Practice {
    pub opened: Vec<&'static str>,
}

/// Each tab with the name `learn.tab` takes for it.
const TABS: [(Tab, &str); 2] = [(Tab::ForYou, "forYou"), (Tab::Skills, "skills")];

/// Runs `cmd`; a failure goes to the status line.
fn run(app: &mut VectorcraftApp, cmd: &str, params: Value) {
    if let Err(e) = app.run(cmd, params) {
        app.status(e);
    }
}

/// The key a collection's last selection change set, if any: a click on a row or a tab.
fn picked<K, E>(events: Vec<CollectionEvent<K, E>>) -> Option<K> {
    events.into_iter().rev().find_map(|event| match event {
        CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => keys.into_iter().last(),
        _ => None,
    })
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // The keys are the indexes of `TABS`.
    let keys = [0_usize, 1];
    let source = keyed(&keys, |key: &usize| *key);
    let shown = TABS.iter().position(|(tab, _)| *tab == app.ui.learn.tab).unwrap_or(0);
    let out = TabStrip::new(tl!("Learn"))
        .kind(TabKind::Panel)
        .shown(shown)
        .show_items(ui, &source, &mut |key, _| TabItem::new(if *key == 0 { tl!("For you") } else { tl!("Skills") }));
    if let Some((tab, name)) = picked(out.response.events).and_then(|i| TABS.get(i))
        && *tab != app.ui.learn.tab
    {
        run(app, "learn.tab", json!({ "tab": name }));
    }
    match app.ui.learn.tab {
        Tab::ForYou => for_you(app, ui),
        Tab::Skills => skills(app, ui),
    }
}

/// The For you tab: Get started, with the Quick skill guide card.
fn for_you(app: &mut VectorcraftApp, ui: &mut Ui) {
    Text::new(tl!("Get started")).role(TypeRole::BodyStrong).show(ui);
    guide_card(app, ui);
}

/// The Quick skill guide card, as one button: a click, or Enter or Space while it has the keyboard
/// focus, shows the panel's list of skills (`learn.skills`). The For you tab and the Home screen's
/// Learn section show it.
pub(crate) fn guide_card(app: &mut VectorcraftApp, ui: &mut Ui) {
    // PLAN: assumed `Button::detail(impl Into<Label>)` (dp.R3, document-panels-w4).
    let card = Button::new(tl!("Quick skill guide"))
        .icon(IconSpec::svg(IconId::new("graduation-cap")))
        .detail(tl!("Short hands-on tutorials for the essential tools, grouped by skill. Choose a skill to see its tutorials."))
        .sizing(Sizing::FillWidth)
        .align(HAlign::Start);
    if clicked(&card.show(ui)) {
        run(app, "learn.skills", json!({}));
    }
}

/// The Skills tab: the skills, a skill's tutorials, or a tutorial's steps.
fn skills(app: &mut VectorcraftApp, ui: &mut Ui) {
    if let Some((_, tutorial)) = app.ui.learn.tutorial.and_then(catalog::tutorial) {
        if back(ui) {
            return run(app, "learn.back", json!({}));
        }
        steps(app, ui, tutorial);
        practice(app, ui, tutorial);
    } else if let Some(skill) = app.ui.learn.skill.and_then(catalog::skill) {
        if back(ui) {
            return run(app, "learn.back", json!({}));
        }
        tutorial_list(app, ui, skill);
    } else {
        skill_list(app, ui);
    }
}

/// The Back button above a skill's tutorials and a tutorial's steps (`learn.back`); true when it
/// was pressed.
fn back(ui: &mut Ui) -> bool {
    clicked(&IconButton::new(IconId::new("chevron-left")).label(tl!("Back")).show(ui).response)
}

/// `row` with the chevron that says it opens.
fn opens(row: ListRow<'_>) -> ListRow<'_> {
    row.trailing_icon(IconSpec::svg(IconId::new("chevron-right")), tl!("Open"))
}

/// The list of skills, each with its progress ring and count, and Reset progress once any progress
/// is kept.
fn skill_list(app: &mut VectorcraftApp, ui: &mut Ui) {
    let progress = &app.ui.learn_progress;
    let keys: Vec<&'static str> = catalog::SKILLS.iter().map(|s| s.id).collect();
    let rings: Vec<(&'static catalog::Skill, SkillRing)> =
        catalog::SKILLS.iter().map(|s| (s, SkillRing { done: progress.done(s), total: s.tutorials.len() })).collect();
    let source = keyed(&keys, |key: &&'static str| *key);
    // PLAN: a single-select list keeps its last pick highlighted and reports no change when the same key is
    // picked again; the keys differ between the levels, but the list is the framework's to reset on a new view.
    // A few skills: the list is as tall as its rows, so Reset progress shows under them.
    let out = List::new(tl!("Skills")).select(SelectMode::Single).natural_height(true).show_items(ui, &source, &mut |key, _| {
        let row = ListRow::new(*key);
        let Some((skill, ring)) = rings.iter().find(|(s, _)| s.id == *key) else { return row };
        let count = format!("{}/{}", ring.done, ring.total);
        let named = format!("{} {count}", tl!("Finished"));
        opens(ListRow::new(skill.title).icon(IconSpec::glyph(ring)).trailing_named(count, named))
    });
    let pick = picked(out.events);
    let reset = !app.ui.learn_progress.is_empty() && clicked(&Button::new(tl!("Reset progress")).show(ui));
    if let Some(id) = pick {
        run(app, "learn.skill", json!({ "skill": id }));
    } else if reset {
        run(app, "learn.resetProgress", json!({}));
    }
}

/// A skill's title and its tutorials, each finished one with a check mark.
fn tutorial_list(app: &mut VectorcraftApp, ui: &mut Ui, skill: &'static catalog::Skill) {
    Text::new(skill.title).role(TypeRole::BodyStrong).show(ui);
    let progress = &app.ui.learn_progress;
    let keys: Vec<&'static str> = skill.tutorials.iter().map(|t| t.id).collect();
    let checks: Vec<(&'static catalog::Tutorial, TutorialCheck)> =
        skill.tutorials.iter().map(|t| (t, TutorialCheck(progress.is_finished(t.id)))).collect();
    let source = keyed(&keys, |key: &&'static str| *key);
    let out = List::new(skill.title).select(SelectMode::Single).show_items(ui, &source, &mut |key, _| {
        let row = ListRow::new(*key);
        let Some((tutorial, check)) = checks.iter().find(|(t, _)| t.id == *key) else { return row };
        let row = ListRow::new(tutorial.title).icon(IconSpec::glyph(check));
        opens(if check.0 { row.accessible_name(format!("{}, {}", tutorial.title, tl!("Finished"))) } else { row })
    });
    if let Some(id) = picked(out.events) {
        run(app, "learn.show", json!({ "tutorial": id }));
    }
}

/// A tutorial's title and its numbered steps, each with a check box that shows whether it is done.
/// The check boxes can be changed once the tutorial's sample is open and until it is finished
/// (`learn.markStep`). The tool's name in a step is a link that shows where the tool is
/// (`learn.locateTool`).
fn steps(app: &mut VectorcraftApp, ui: &mut Ui, tutorial: &'static catalog::Tutorial) {
    Text::new(tutorial.title).role(TypeRole::BodyStrong).show(ui);
    let markable = app.ui.learn_practice.opened.contains(&tutorial.id) && !app.ui.learn_progress.is_finished(tutorial.id);
    let mut locate = None;
    let mut marked = None;
    for (i, step) in tutorial.steps.iter().enumerate() {
        let done = app.ui.learn_progress.step_done(tutorial, i);
        // PLAN: assumed the row aligns its members to the top by default or by a `Row` option; a wrapped step
        // text should keep its number and box on its first line.
        Row::new().gap(Gap::Item).show(ui, |ui, _| {
            let name = format!("{}. {}", i + 1, step.plain());
            // PLAN: assumed `Field::id_salt` takes any hashable value.
            let next = Field::new(name)
                .side(SideLabel::new().kind(LabelKind::Hidden))
                .id_salt(i)
                .show(ui, |ui, _| Checkbox::new(Bind::given(done)).enabled(markable).show(ui).value().copied())
                .inner
                .flatten();
            if let Some(next) = next {
                marked = Some((i, next));
            }
            // PLAN: the number should read in the muted ink; the caption role stands in until `Text` names it.
            Text::new(format!("{}.", i + 1)).role(TypeRole::Caption).show(ui);
            if step_text(ui, step) {
                locate = step.tool;
            }
        });
    }
    if let Some((step, complete)) = marked {
        run(app, "learn.markStep", json!({ "tutorial": tutorial.id, "step": step, "complete": complete }));
    }
    if let Some(tool) = locate {
        run(app, "learn.locateTool", json!({ "tool": tool }));
    }
}

/// A step's text: with a tool, the tool's name as a link inside the sentence. True when the link was
/// activated, by a click or by Enter or Space with the keyboard focus on it. The Learn panel and
/// Discover show steps with it.
pub(crate) fn step_text(ui: &mut Ui, step: &catalog::Step) -> bool {
    match (step.tool, step.tool_name()) {
        // PLAN: assumed `Link::in_sentence(before, after)` (dp.R2, document-panels-w4).
        (Some(_), Some((before, name, after))) => {
            clicked(&Link::new(name).in_sentence(before, after).tooltip(tl!("Show where this tool is")).show(ui))
        }
        _ => {
            Text::new(step.text).show(ui);
            false
        }
    }
}

/// The controls under a tutorial's steps: View more; Finish tutorial once its sample is open; and
/// once it is finished, Next Up with the next tutorial's title, or for the skill's last tutorial a
/// line saying so and Back to skills.
fn practice(app: &mut VectorcraftApp, ui: &mut Ui, tutorial: &'static catalog::Tutorial) {
    let id = tutorial.id;
    let params = json!({ "tutorial": id });
    let opened = app.ui.learn_practice.opened.contains(&id);
    let finished = app.ui.learn_progress.is_finished(id);
    let mut pick: Option<(&str, Value)> = None;
    Row::new().gap(Gap::Item).show(ui, |ui, _| {
        if clicked(&Button::new(tl!("View more")).show(ui)) {
            pick = Some(("learn.openSample", params.clone()));
        }
        if opened && !finished && clicked(&Button::new(tl!("Finish tutorial")).show(ui)) {
            pick = Some(("learn.finish", params.clone()));
        }
    });
    if finished {
        match catalog::next(id) {
            Some(next) => {
                Row::new().gap(Gap::Item).show(ui, |ui, _| {
                    if clicked(&Button::new(tl!("Next Up")).show(ui)) {
                        pick = Some(("learn.next", params.clone()));
                    }
                    Text::new(next.title).show(ui);
                });
            }
            None => {
                Text::new(tl!("You finished the last tutorial in this skill.")).role(TypeRole::Caption).show(ui);
                if clicked(&Button::new(tl!("Back to skills")).show(ui)) {
                    pick = Some(("learn.skills", json!({})));
                }
            }
        }
    }
    if let Some((cmd, params)) = pick {
        run(app, cmd, params);
    }
}

/// A skill's progress ring: an outline, with the share `done` of `total` drawn over it clockwise
/// from the top. With no tutorials it stays empty.
#[derive(MemoProps)]
struct SkillRing {
    done: usize,
    total: usize,
}

impl Glyph for SkillRing {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, _color: Color32, _mirrored: bool) {
        let width = tokens.stroke.thin;
        let center = rect.center();
        let radius = rect.width().min(rect.height()) / 2.0 - width;
        painter.circle_stroke(center, radius, Stroke::new(width, rgba_color(tokens.color.line.border)));
        if self.done == 0 || self.total == 0 {
            return;
        }
        let stroke = Stroke::new(width, rgba_color(tokens.color.accent.solid));
        if self.done >= self.total {
            painter.circle_stroke(center, radius, stroke);
            return;
        }
        let share = self.done as f32 / self.total as f32;
        let sweep = std::f32::consts::TAU * share;
        // About one segment per 1/32 of a turn, and never fewer than two.
        let segments = ((32.0 * share).ceil() as usize).max(2);
        let points = (0..=segments)
            .map(|k| {
                let angle = -std::f32::consts::FRAC_PI_2 + sweep * k as f32 / segments as f32;
                center + radius * vec2(angle.cos(), angle.sin())
            })
            .collect();
        painter.add(Shape::line(points, stroke));
    }
}

/// A tutorial's mark: an outline, filled with a check mark once it is finished (true).
#[derive(MemoProps)]
struct TutorialCheck(bool);

impl Glyph for TutorialCheck {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, _color: Color32, _mirrored: bool) {
        let width = tokens.stroke.thin;
        let center = rect.center();
        let radius = rect.width().min(rect.height()) / 2.0 - width;
        if !self.0 {
            painter.circle_stroke(center, radius, Stroke::new(width, rgba_color(tokens.color.line.border)));
            return;
        }
        painter.circle_filled(center, radius, rgba_color(tokens.color.accent.solid));
        let tick = vec![center + radius * vec2(-0.5, 0.0), center + radius * vec2(-0.14, 0.36), center + radius * vec2(0.5, -0.36)];
        painter.add(Shape::line(tick, Stroke::new(width, rgba_color(tokens.color.surface.panel))));
    }
}

/// The Learn panel's UI commands: `window.learn`, `learn.list`, `learn.tab`, `learn.skills`,
/// `learn.skill`, `learn.show`, `learn.back`, `learn.locateTool`, `learn.openSample`,
/// `learn.markStep`, `learn.finish`, `learn.next` and `learn.resetProgress`.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "window.learn" => toggle(app, p),
        "learn.list" => Ok(list(&app.ui.learn_progress)),
        "learn.tab" => tab(app, p),
        "learn.skills" => skills_list(app),
        "learn.skill" => open_skill(app, p),
        "learn.show" => open(app, p),
        "learn.back" => back_up(app),
        "learn.locateTool" => locate::locate_tool(app, p),
        "learn.openSample" => open_sample(app, p),
        "learn.markStep" => mark_step(app, p),
        "learn.finish" => finish(app, p),
        "learn.next" => next(app, p),
        "learn.resetProgress" => {
            app.ui.learn_progress = progress::Progress::default();
            Ok(list(&app.ui.learn_progress))
        }
        _ => return None,
    })
}

/// Whether the panel can be seen: it is the front panel, the dock is not hidden and no full screen
/// mode hides the panels.
pub(crate) fn visible(app: &VectorcraftApp) -> bool {
    // PLAN: assumed `pub(crate) fn panels_shown(app: &VectorcraftApp) -> bool` (ch.P2, chrome-w7).
    crate::panel_docking::panels_shown(app) && crate::panel_docking::showing(&app.ui, ID)
}

/// `window.learn {show?}`: show the panel (true), hide it (false) or toggle it (omitted), as the
/// app bar's Explore tutorials button and Window › Learn do → whether it shows ([`visible`]).
/// Showing it also shows a hidden dock; in a full screen mode it makes the panel the front one.
fn toggle(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let shown = visible(app);
    let show = crate::menus::opt_bool(p, "show")?.unwrap_or(!shown);
    if show && !shown {
        crate::panel_docking::reveal_panel(app, ID)?;
    } else if !show && shown {
        app.run("window.panel", json!({ "panel": ID, "toggle": true }))?;
    }
    Ok(json!(visible(app)))
}

/// `learn.list`: every skill with its count of tutorials finished (`done`) of all (`total`), and its
/// tutorials with their step counts, whether each is finished, and the zero-based indexes of its
/// steps marked done (`completedSteps`).
fn list(progress: &progress::Progress) -> Value {
    json!({ "skills": catalog::SKILLS.iter().map(|s| skill_json(progress, s)).collect::<Vec<_>>() })
}

/// A skill as `learn.list` lists it: `{id, title, done, total, tutorials: [{id, title, steps,
/// finished, completedSteps}]}`.
fn skill_json(progress: &progress::Progress, s: &catalog::Skill) -> Value {
    let tutorials: Vec<Value> = s
        .tutorials
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "title": t.title,
                "steps": t.steps.len(),
                "finished": progress.is_finished(t.id),
                "completedSteps": progress.completed_steps(t),
            })
        })
        .collect();
    json!({"id": s.id, "title": s.title, "done": progress.done(s), "total": s.tutorials.len(), "tutorials": tutorials})
}

/// What the panel shows: `{tab: forYou|skills, skill, tutorial}`, with null for no skill or no
/// tutorial open.
fn view_json(view: &View) -> Value {
    let tab = TABS.iter().find(|(t, _)| *t == view.tab).map(|(_, name)| *name);
    json!({"tab": tab, "skill": view.skill, "tutorial": view.tutorial})
}

/// `learn.tab {tab}`: the For you and Skills tabs. Show the panel on that tab (`forYou` or
/// `skills`); the Skills tab keeps the skill and tutorial it had open → the view ([`view_json`]). A
/// missing or unknown tab is an error and changes nothing.
fn tab(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "tab";
    let name = match p.get(key) {
        None | Some(Value::Null) => return Err(format!("missing `{key}`")),
        Some(Value::String(name)) => name.as_str(),
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    let names = || TABS.map(|(_, name)| name).join(", ");
    let tab = TABS.iter().find(|(_, n)| *n == name).map(|(t, _)| *t).ok_or_else(|| format!("`{key}` must be one of {}", names()))?;
    crate::panel_docking::reveal_panel(app, ID)?;
    app.ui.learn.tab = tab;
    Ok(view_json(&app.ui.learn))
}

/// `learn.skill {skill}`: a skill's row on the Skills tab. Show the panel on the Skills tab with
/// that skill's tutorials → the skill, as `learn.list` lists it. A missing or unknown id is an error
/// and changes nothing.
fn open_skill(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "skill";
    let id = match p.get(key) {
        None | Some(Value::Null) => return Err(format!("missing `{key}`")),
        Some(Value::String(id)) => id.as_str(),
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    let ids = || catalog::SKILLS.iter().map(|s| s.id).collect::<Vec<_>>().join(", ");
    let skill = catalog::skill(id).ok_or_else(|| format!("`{key}` must be one of {}", ids()))?;
    crate::panel_docking::reveal_panel(app, ID)?;
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: None };
    Ok(skill_json(&app.ui.learn_progress, skill))
}

/// `learn.back`: the Back button on the Skills tab. Show the panel and go up one level: from a
/// tutorial's steps to its skill's tutorials, and from a skill's tutorials to the list of skills →
/// the view ([`view_json`]). On the list of skills or the For you tab the view stays as it is.
fn back_up(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panel_docking::reveal_panel(app, ID)?;
    let view = &mut app.ui.learn;
    if view.tab == Tab::Skills {
        if let Some((skill, _)) = view.tutorial.and_then(catalog::tutorial) {
            view.skill = Some(skill.id);
            view.tutorial = None;
        } else if view.skill.is_some() {
            view.skill = None;
            view.tutorial = None;
        }
    }
    Ok(view_json(view))
}

/// `learn.skills`: Back to skills, and the Quick skill guide card. Show the panel on the Skills tab
/// with the list of skills, with no skill and no tutorial open → what `learn.list` answers.
fn skills_list(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panel_docking::reveal_panel(app, ID)?;
    app.ui.learn = View { tab: Tab::Skills, skill: None, tutorial: None };
    Ok(list(&app.ui.learn_progress))
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

/// `learn.markStep {tutorial, step, complete?}`: a step's check box. Mark step `step` (zero-based)
/// of a tutorial whose sample was opened done (`complete` true or omitted) or not done (false) →
/// `{tutorial, step, complete}`. A finished tutorial keeps every step done, so marking one of its
/// steps is an error; so are a step the tutorial does not have and a `complete` that is not true or
/// false. An error changes nothing.
fn mark_step(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (_, tutorial) = tutorial_param(p)?;
    let id = tutorial.id;
    let last = tutorial.steps.len().saturating_sub(1);
    let step = match p.get("step") {
        None | Some(Value::Null) => return Err("missing `step` (see learn.show)".into()),
        Some(v) => v.as_u64().and_then(|s| usize::try_from(s).ok()).filter(|s| *s < tutorial.steps.len()),
    };
    let step = step.ok_or_else(|| format!("`step` must be a step number from 0 to {last}"))?;
    let complete = crate::menus::opt_bool(p, "complete")?.unwrap_or(true);
    if !app.ui.learn_practice.opened.contains(&id) {
        return Err(format!("open the practice sample of `{id}` first (see learn.openSample)"));
    }
    if app.ui.learn_progress.is_finished(id) {
        return Err(format!("`{id}` is finished (see learn.resetProgress)"));
    }
    app.ui.learn_progress.mark(id, step, complete);
    Ok(json!({"tutorial": id, "step": step, "complete": complete}))
}

/// `learn.finish {tutorial}`: Finish tutorial. Mark a tutorial whose sample was opened finished,
/// with all its steps done → `{tutorial, finished: true, next}`, where `next` is the id of the next
/// tutorial in its skill, or null after the skill's last one. Finishing it again keeps one entry,
/// also after a restart, when its sample has not been opened again. The Learn view stays as it is.
fn finish(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (_, tutorial) = tutorial_param(p)?;
    let id = tutorial.id;
    if !app.ui.learn_practice.opened.contains(&id) && !app.ui.learn_progress.is_finished(id) {
        return Err(format!("open the practice sample of `{id}` first (see learn.openSample)"));
    }
    app.ui.learn_progress.finish(id);
    Ok(json!({"tutorial": id, "finished": true, "next": catalog::next(id).map(|t| t.id)}))
}

/// `learn.next {tutorial}`: Next Up. Show the panel on the tutorial after a finished one in the
/// same skill → `{skill, tutorial, title}` of that tutorial. It opens no sample. An unfinished
/// tutorial, or a skill's last tutorial, is an error and changes nothing.
fn next(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (skill, tutorial) = tutorial_param(p)?;
    let id = tutorial.id;
    if !app.ui.learn_progress.is_finished(id) {
        return Err(format!("finish `{id}` first (see learn.finish)"));
    }
    let after = catalog::next(id).ok_or_else(|| format!("`{id}` is the last tutorial in its skill"))?;
    crate::panel_docking::reveal_panel(app, ID)?;
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: Some(after.id) };
    Ok(json!({"skill": skill.id, "tutorial": after.id, "title": after.title}))
}

#[cfg(test)]
mod tests;
