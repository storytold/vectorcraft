use std::collections::HashSet;

use egui::accesskit::Node;
use serde_json::json;
use vectorcraft_engine::Session;

use super::{ID, Tab, View, catalog, show};
use crate::VectorcraftApp;
use crate::tests_labels::painted_text;

const SCREEN: egui::Rect = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1400.0, 800.0));

fn app() -> VectorcraftApp {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    VectorcraftApp::new(Session::new(), Default::default())
}

/// One frame of `draw` with screen-reader nodes on → the nodes.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<Node> {
    let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(SCREEN), ..Default::default() }, |ui| draw(app, ui));
    out.textures_delta.clear();
    out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
}

/// The center of the node labeled `name`.
fn center(nodes: &[Node], name: &str) -> Option<egui::Pos2> {
    center_where(nodes, |label| label == name)
}

/// The center of the first node whose label `pick` takes.
fn center_where(nodes: &[Node], pick: impl Fn(&str) -> bool) -> Option<egui::Pos2> {
    let b = nodes.iter().find(|n| n.label().is_some_and(&pick))?.bounds()?;
    Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// Click the node labeled `name` in what `draw` draws.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    frame(app, ctx, vec![], draw);
    let at = center(&frame(app, ctx, vec![], draw), name).unwrap_or_else(|| panic!("{name} is drawn"));
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![egui::Event::PointerMoved(at), press(true)], draw);
    frame(app, ctx, vec![press(false)], draw);
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    ctx
}

fn app_bar(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    crate::chrome::app_bar(app, ui);
}

fn shown(app: &VectorcraftApp) -> bool {
    crate::panel_docking::showing(&app.ui, ID)
}

/// Every skill has tutorials and every tutorial steps; ids are unique; every tool and command a
/// step names exists.
#[test]
fn the_catalog_is_valid() {
    let mut skills = HashSet::new();
    let mut tutorials = HashSet::new();
    assert!(!catalog::SKILLS.is_empty());
    for skill in catalog::SKILLS {
        assert!(skills.insert(skill.id), "skill id {} is used twice", skill.id);
        assert!(!skill.title.is_empty() && !skill.tutorials.is_empty(), "skill {} is empty", skill.id);
        for tutorial in skill.tutorials {
            assert!(tutorials.insert(tutorial.id), "tutorial id {} is used twice", tutorial.id);
            assert!(!tutorial.title.is_empty() && !tutorial.steps.is_empty(), "tutorial {} is empty", tutorial.id);
            for step in tutorial.steps {
                assert!(step.text.ends_with('.'), "{}: {:?}", tutorial.id, step.text);
                if let Some(tool) = step.tool {
                    assert!(vectorcraft_tools::catalog::tool_info(tool).is_some(), "{}: no tool {tool}", tutorial.id);
                }
                if let Some(cmd) = step.command {
                    let known =
                        crate::menus::UI_COMMANDS.iter().any(|c| c.0 == cmd) || vectorcraft_engine::cmd::command_specs().iter().any(|c| c.id == cmd);
                    assert!(known, "{}: no command {cmd}", tutorial.id);
                }
            }
        }
    }
}

/// Explore tutorials at the right end of the app bar shows the Learn panel, and hides it again.
#[test]
fn explore_tutorials_on_the_app_bar_shows_and_hides_the_learn_panel() {
    let mut app = app();
    let ctx = context();
    assert!(!shown(&app));
    let nodes = frame(&mut app, &ctx, vec![], app_bar);
    let search = center_where(&nodes, |label| label.starts_with("Search commands and tools"));
    let learn = center(&nodes, "Explore tutorials");
    let (search, learn) = (search.expect("the search icon shows"), learn.expect("the app bar has Explore tutorials"));
    assert!(learn.x < search.x && (learn.y - search.y).abs() < 1.0, "beside the search icon: {learn:?}, {search:?}");
    click(&mut app, &ctx, "Explore tutorials", app_bar);
    assert!(shown(&app), "{}", app.ui.status);
    click(&mut app, &ctx, "Explore tutorials", app_bar);
    assert!(!shown(&app));
}

/// The command and params of Window › Learn.
fn window_learn() -> (&'static str, serde_json::Value) {
    use crate::menus::Item;
    let window = crate::menus::menu_tree().into_iter().find(|(title, _)| *title == "Window").map(|(_, items)| items).unwrap();
    window
        .iter()
        .find_map(|item| match item {
            Item::Cmd("Learn", cmd, p) => Some((*cmd, p.clone())),
            _ => None,
        })
        .expect("Window › Learn")
}

/// Window › Learn shows and hides the panel, as `window.learn` does.
#[test]
fn window_learn_shows_and_hides_the_panel() {
    let mut app = app();
    let (cmd, params) = window_learn();
    app.run(cmd, params.clone()).unwrap();
    assert!(shown(&app));
    assert_eq!(crate::menus::checked(&app, cmd, &params), Some(true), "Window › Learn is checked");
    app.run(cmd, params.clone()).unwrap();
    assert!(!shown(&app));
    assert_eq!(crate::menus::checked(&app, cmd, &params), Some(false));
    assert_eq!(app.run("window.learn", json!({"show": true})).unwrap(), json!(true));
    assert_eq!(app.run("window.learn", json!({"show": true})).unwrap(), json!(true), "shown stays shown");
    assert_eq!(app.run("window.learn", json!({})).unwrap(), json!(false));
    assert!(app.run("window.learn", json!({"show": "yes"})).is_err());
    assert!(!shown(&app));
}

/// With the dock hidden, Learn as the front panel is not visible: `window.learn`, Window › Learn
/// and Explore tutorials show the dock instead of closing the panel.
#[test]
fn window_learn_shows_a_hidden_dock() {
    let mut app = app();
    let (cmd, params) = window_learn();
    assert_eq!(app.run("window.learn", json!({"show": true})).unwrap(), json!(true));
    app.ui.dock = false;
    assert_eq!(crate::menus::checked(&app, cmd, &params), Some(false), "Window › Learn is unchecked");
    assert_eq!(app.run("window.learn", json!({"show": false})).unwrap(), json!(false));
    assert!(shown(&app) && !app.ui.dock, "hiding a panel that cannot be seen changes nothing");
    app.run(cmd, params).unwrap();
    assert!(app.ui.dock && shown(&app), "Window › Learn shows the dock");
    // A full screen mode hides the panels: the panel stays the front one and shows when it ends.
    app.ui.screen_mode = 3;
    assert_eq!(app.run("window.learn", json!({})).unwrap(), json!(false));
    assert!(shown(&app));
    app.ui.screen_mode = 0;
    assert_eq!(app.run("window.learn", json!({})).unwrap(), json!(false));
    assert!(!shown(&app));
}

/// The For you tab's Quick skill guide card opens the Skills tab, which lists every skill with its
/// count of tutorials.
#[test]
fn the_skills_tab_lists_every_skill() {
    let mut app = app();
    let ctx = context();
    let text = painted_text(&mut app, show);
    assert!(text.contains("Get started") && text.contains("Quick skill guide"), "{text}");
    click(&mut app, &ctx, "Quick skill guide", show);
    assert_eq!(app.ui.learn.tab, Tab::Skills);
    let text = painted_text(&mut app, show);
    for skill in catalog::SKILLS {
        assert!(text.contains(skill.title), "{} in {text}", skill.title);
        assert!(text.contains(&format!("0/{}", skill.tutorials.len())), "{text}");
    }
    let skill = &catalog::SKILLS[1];
    click(&mut app, &ctx, skill.title, show);
    let text = painted_text(&mut app, show);
    for tutorial in skill.tutorials {
        assert!(text.contains(tutorial.title), "{} in {text}", tutorial.title);
    }
}

/// Choosing a tutorial previews its numbered steps; Back goes to its skill, then to the skills.
#[test]
fn a_tutorial_previews_its_steps() {
    let mut app = app();
    let ctx = context();
    let (skill, tutorial) = catalog::tutorial("pen-paths").unwrap();
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: None };
    click(&mut app, &ctx, tutorial.title, show);
    assert_eq!(app.ui.learn.tutorial, Some(tutorial.id));
    let text = painted_text(&mut app, show);
    for (i, step) in tutorial.steps.iter().enumerate() {
        assert!(text.contains(&format!("{}.", i + 1)) && text.contains(step.text), "{} in {text}", step.text);
    }
    click(&mut app, &ctx, "Back", show);
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: Some(skill.id), tutorial: None });
    click(&mut app, &ctx, "Back", show);
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: None, tutorial: None });
}

/// `learn.show` opens the panel on a tutorial's steps; `learn.list` lists every tutorial.
#[test]
fn learn_show_opens_a_tutorial_and_learn_list_lists_them() {
    let mut app = app();
    let shown_tutorial = app.run("learn.show", json!({"tutorial": "gradient"})).unwrap();
    assert_eq!(shown_tutorial["skill"], "color");
    assert_eq!(shown_tutorial["steps"].as_array().map(Vec::len), Some(3));
    assert!(shown(&app));
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: Some("color"), tutorial: Some("gradient") });
    let list = app.run("learn.list", json!({})).unwrap();
    let skills = list["skills"].as_array().unwrap();
    assert_eq!(skills.len(), catalog::SKILLS.len());
    let count: usize = skills.iter().map(|s| s["tutorials"].as_array().map_or(0, Vec::len)).sum();
    assert_eq!(count, catalog::SKILLS.iter().map(|s| s.tutorials.len()).sum::<usize>());
    assert_eq!(skills[2]["tutorials"][1], json!({"id": "gradient", "title": "Add a gradient", "steps": 3}));
}

/// A missing, mistyped or unknown tutorial id is an error and leaves the panel as it was.
#[test]
fn learn_show_rejects_a_missing_or_unknown_tutorial() {
    let mut app = app();
    let before = app.ui.learn.clone();
    for (params, error) in [
        (json!({}), "missing `tutorial` (see learn.list)"),
        (json!({"tutorial": 3}), "`tutorial` must be a string"),
        (json!({"tutorial": "nope"}), "unknown tutorial `nope` (see learn.list)"),
    ] {
        assert_eq!(app.run("learn.show", params).unwrap_err(), error);
        assert_eq!(app.ui.learn, before);
        assert!(!shown(&app));
    }
}
