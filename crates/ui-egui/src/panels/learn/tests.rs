use std::collections::HashSet;

use egui::accesskit::{Node, Role, Toggled};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::{ID, Practice, Tab, View, catalog, samples, show};
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
                let brackets = step.text.matches(['[', ']']).count();
                if let Some(tool) = step.tool {
                    let info = vectorcraft_tools::catalog::tool_info(tool);
                    assert!(info.is_some(), "{}: no tool {tool}", tutorial.id);
                    // The step's link names the tool it locates, once.
                    let name = step.tool_name().map(|(_, name, _)| name);
                    assert!(
                        brackets == 2 && name.zip(info).is_some_and(|(n, t)| n.eq_ignore_ascii_case(t.label)),
                        "{}: {:?}",
                        tutorial.id,
                        step.text
                    );
                } else {
                    assert_eq!(brackets, 0, "{}: {:?} links no tool", tutorial.id, step.text);
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
        let parts = step.tool_name().map_or(vec![step.text], |(before, name, after)| vec![before, name, after]);
        assert!(text.contains(&format!("{}.", i + 1)) && parts.iter().all(|p| text.contains(p)), "{} in {text}", step.text);
        assert!(!text.contains('['), "no brackets: {text}");
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
    assert_eq!(skills[2]["tutorials"][1], json!({"id": "gradient", "title": "Add a gradient", "steps": 3, "finished": false, "completedSteps": []}));
    assert_eq!((skills[2]["done"].clone(), skills[2]["total"].clone()), (json!(0), json!(3)));
}

/// Every tutorial has one sample of its own, in the catalog's order, each one small and read
/// without warnings into art of its own.
#[test]
fn every_tutorial_has_its_own_readable_sample() {
    let ids: Vec<&str> = catalog::SKILLS.iter().flat_map(|s| s.tutorials.iter().map(|t| t.id)).collect();
    let sampled: Vec<&str> = samples::SAMPLES.iter().map(|(id, _)| *id).collect();
    assert_eq!(sampled, ids, "one sample for each tutorial, in the catalog's order");
    let mut texts = HashSet::new();
    for (id, svg) in samples::SAMPLES {
        assert!(svg.len() <= samples::MAX_SAMPLE_BYTES, "{id}: {} bytes", svg.len());
        assert!(texts.insert(*svg), "{id} reuses another tutorial's sample");
        let loaded = vectorcraft_engine::cmd::fileio::load(&format!("{id}.svg"), svg.as_bytes()).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(loaded.warnings.is_empty(), "{id}: {:?}", loaded.warnings);
        let mut art = 0;
        loaded.doc.walk(|n| art += usize::from(!n.is_layer()));
        assert!(art > 0, "{id} has no art");
    }
}

/// Next Up stays in the tutorial's skill: a skill's last tutorial, the catalog's last one
/// included, has no next tutorial.
#[test]
fn the_next_tutorial_is_the_next_one_in_the_same_skill() {
    for skill in catalog::SKILLS {
        for pair in skill.tutorials.windows(2) {
            assert_eq!(catalog::next(pair[0].id).map(|t| t.id), Some(pair[1].id));
        }
        let last = skill.tutorials.last().unwrap();
        assert!(catalog::next(last.id).is_none(), "{} is the last of {}", last.id, skill.id);
    }
    assert!(catalog::next("nope").is_none());
}

/// View more opens the tutorial's sample as a new untitled document beside a modified one, which
/// stays as it was; another tutorial opens another sample, and View more again opens another copy.
#[test]
fn view_more_opens_the_tutorial_s_sample_as_a_new_untitled_document() {
    let mut app = app();
    let ctx = context();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    let work = app.session.documents()[0].doc.node_count();
    let (skill, tutorial) = catalog::tutorial("pen-paths").unwrap();
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: Some(tutorial.id) };
    click(&mut app, &ctx, "View more", show);
    let docs = app.session.documents();
    assert_eq!(docs.len(), 2, "{}", app.ui.status);
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(docs[0].doc.node_count(), work, "the open document stays as it was");
    assert!(docs[1].path.is_none() && docs[1].title().starts_with("Untitled-"), "{}", docs[1].title());
    let pen = docs[1].doc.node_count();
    assert_eq!(app.ui.learn_practice, Practice { opened: vec!["pen-paths"] });
    assert!(app.ui.learn_progress.is_empty(), "opening a sample marks nothing");

    let r = app.run("learn.openSample", json!({"tutorial": "gradient"})).unwrap();
    assert_eq!(r["tutorial"], "gradient");
    assert_eq!(r["index"], 2);
    assert_eq!(r["title"], json!(app.session.documents()[2].title()));
    assert_ne!(app.session.documents()[2].doc.node_count(), pen, "another tutorial, another sample");
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: Some("color"), tutorial: Some("gradient") });
    app.run("learn.openSample", json!({"tutorial": "gradient"})).unwrap();
    assert_eq!(app.session.documents().len(), 4, "View more again opens another copy");
    assert_eq!(app.ui.learn_practice.opened, ["pen-paths", "gradient"]);
}

/// Finish tutorial shows once the sample is open; finishing shows Next Up with the next
/// tutorial's title, and Next Up shows that tutorial without opening its sample.
#[test]
fn finishing_a_tutorial_offers_next_up_in_its_skill() {
    let mut app = app();
    let ctx = context();
    let (skill, tutorial) = catalog::tutorial("pen-paths").unwrap();
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: Some(tutorial.id) };
    let text = painted_text(&mut app, show);
    assert!(text.contains("View more") && !text.contains("Finish tutorial") && !text.contains("Next Up"), "{text}");
    click(&mut app, &ctx, "View more", show);
    let text = painted_text(&mut app, show);
    assert!(text.contains("Finish tutorial") && !text.contains("Next Up"), "{text}");
    click(&mut app, &ctx, "Finish tutorial", show);
    assert_eq!(app.ui.learn_progress.finished, ["pen-paths"]);
    let text = painted_text(&mut app, show);
    assert!(text.contains("Next Up") && text.contains("Draw smooth curves") && !text.contains("Finish tutorial"), "{text}");
    let docs = app.session.documents().len();
    click(&mut app, &ctx, "Next Up", show);
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: Some("draw"), tutorial: Some("smooth-curves") });
    assert_eq!(app.session.documents().len(), docs, "Next Up opens no sample");
    let text = painted_text(&mut app, show);
    assert!(text.contains("View more") && !text.contains("Finish tutorial") && !text.contains("Next Up"), "{text}");
}

/// One press and release of `key` in what `draw` draws.
fn key(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let event = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    frame(app, ctx, vec![event(true)], draw);
    frame(app, ctx, vec![event(false)], draw);
}

/// Press Tab until the control labeled `name` has the keyboard focus.
fn tab_to(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    for _ in 0..32 {
        key(app, ctx, egui::Key::Tab, draw);
        let at = center(&frame(app, ctx, vec![], draw), name).unwrap_or_else(|| panic!("{name} is drawn"));
        if ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id)).is_some_and(|r| r.rect.contains(at)) {
            return;
        }
    }
    panic!("Tab did not reach {name}");
}

/// View more, Finish tutorial and Next Up are reached with Tab and pressed with Enter.
#[test]
fn the_practice_controls_work_from_the_keyboard() {
    let mut app = app();
    let ctx = context();
    let (skill, tutorial) = catalog::tutorial("pen-paths").unwrap();
    app.ui.learn = View { tab: Tab::Skills, skill: Some(skill.id), tutorial: Some(tutorial.id) };
    frame(&mut app, &ctx, vec![], show);
    tab_to(&mut app, &ctx, "View more", show);
    key(&mut app, &ctx, egui::Key::Enter, show);
    assert_eq!(app.session.documents().len(), 1, "{}", app.ui.status);
    assert_eq!(app.ui.learn_practice.opened, ["pen-paths"]);
    tab_to(&mut app, &ctx, "Finish tutorial", show);
    key(&mut app, &ctx, egui::Key::Enter, show);
    assert_eq!(app.ui.learn_progress.finished, ["pen-paths"]);
    tab_to(&mut app, &ctx, "Next Up", show);
    key(&mut app, &ctx, egui::Key::Enter, show);
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: Some("draw"), tutorial: Some("smooth-curves") });
    assert_eq!(app.session.documents().len(), 1, "Next Up opens no sample");
}

/// The panel 150 points wide.
fn narrow(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    ui.allocate_ui(egui::vec2(150.0, 800.0), |ui| {
        ui.set_max_width(150.0);
        show(app, ui);
    });
}

/// In a narrow panel the practice controls wrap inside it.
#[test]
fn the_practice_controls_fit_a_narrow_panel() {
    let mut app = app();
    let ctx = context();
    app.run("learn.openSample", json!({"tutorial": "pen-paths"})).unwrap();
    frame(&mut app, &ctx, vec![], narrow);
    let nodes = frame(&mut app, &ctx, vec![], narrow);
    for name in ["View more", "Finish tutorial"] {
        let node = nodes.iter().find(|n| n.label() == Some(name)).unwrap_or_else(|| panic!("{name} is drawn"));
        let b = node.bounds().unwrap();
        assert!(b.x1 <= 151.0, "{name} ends at {}", b.x1);
    }
}

/// A skill's last tutorial, finished, says so and offers Back to skills instead of Next Up, even
/// when another skill follows; `learn.next` there is an error.
#[test]
fn the_last_tutorial_of_a_skill_offers_back_to_skills() {
    let mut app = app();
    let ctx = context();
    for id in ["add-headline", "outline-type"] {
        app.run("learn.openSample", json!({"tutorial": id})).unwrap();
        assert_eq!(app.run("learn.finish", json!({"tutorial": id})).unwrap(), json!({"tutorial": id, "finished": true, "next": null}));
        let text = painted_text(&mut app, show);
        assert!(text.contains("You finished the last tutorial in this skill.") && !text.contains("Next Up"), "{text}");
        let before = app.ui.learn.clone();
        assert_eq!(app.run("learn.next", json!({"tutorial": id})).unwrap_err(), format!("`{id}` is the last tutorial in its skill"));
        assert_eq!(app.ui.learn, before);
        click(&mut app, &ctx, "Back to skills", show);
        assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: None, tutorial: None });
    }
}

/// `learn.finish` needs the sample opened, and finishing again keeps one entry; `learn.next` needs
/// the tutorial finished and answers the next tutorial.
#[test]
fn learn_finish_and_learn_next_follow_the_practice() {
    let mut app = app();
    let open = "open the practice sample of `gradient` first (see learn.openSample)";
    assert_eq!(app.run("learn.finish", json!({"tutorial": "gradient"})).unwrap_err(), open);
    assert_eq!(app.run("learn.next", json!({"tutorial": "gradient"})).unwrap_err(), "finish `gradient` first (see learn.finish)");
    assert_eq!(app.ui.learn_practice, Practice::default());
    assert!(app.ui.learn_progress.is_empty());
    app.run("learn.openSample", json!({"tutorial": "gradient"})).unwrap();
    assert_eq!(app.run("learn.next", json!({"tutorial": "gradient"})).unwrap_err(), "finish `gradient` first (see learn.finish)");
    for _ in 0..2 {
        let r = app.run("learn.finish", json!({"tutorial": "gradient"})).unwrap();
        assert_eq!(r, json!({"tutorial": "gradient", "finished": true, "next": "copy-colors"}));
    }
    assert_eq!(app.ui.learn_progress.finished, ["gradient"], "finishing again keeps one entry");
    let r = app.run("learn.next", json!({"tutorial": "gradient"})).unwrap();
    assert_eq!(r, json!({"skill": "color", "tutorial": "copy-colors", "title": "Copy colors from other art"}));
    assert!(shown(&app));
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: Some("color"), tutorial: Some("copy-colors") });
}

/// `learn.skills` shows the panel on the Skills tab with the list of skills and answers what
/// `learn.list` answers. Back to skills and the Quick skill guide card run it: each one also shows
/// a panel that was hidden, as the command does.
#[test]
fn learn_skills_shows_the_list_of_skills() {
    let skills = View { tab: Tab::Skills, skill: None, tutorial: None };
    assert!(crate::menus::UI_COMMANDS.iter().any(|c| c.0 == "learn.skills"), "learn.skills is in the UI command catalog");
    let mut app = app();
    app.run("learn.show", json!({"tutorial": "gradient"})).unwrap();
    app.run("window.learn", json!({"show": false})).unwrap();
    assert!(!shown(&app));
    let list = app.run("learn.list", json!({})).unwrap();
    assert_eq!(app.run("learn.skills", json!({})).unwrap(), list);
    assert_eq!(app.ui.learn, skills);
    assert!(shown(&app));

    // Back to skills, after a skill's last tutorial.
    let mut app = self::app();
    let ctx = context();
    app.run("learn.openSample", json!({"tutorial": "outline-type"})).unwrap();
    app.run("learn.finish", json!({"tutorial": "outline-type"})).unwrap();
    assert!(!shown(&app));
    click(&mut app, &ctx, "Back to skills", show);
    assert_eq!(app.ui.learn, skills);
    assert!(shown(&app), "Back to skills runs learn.skills");

    // The Quick skill guide card on the For you tab.
    let mut app = self::app();
    let ctx = context();
    assert!(!shown(&app));
    click(&mut app, &ctx, "Quick skill guide", show);
    assert_eq!(app.ui.learn, skills);
    assert!(shown(&app), "the Quick skill guide card runs learn.skills");
}

/// The practice commands reject a missing, mistyped or unknown tutorial id and change nothing.
#[test]
fn the_practice_commands_reject_a_missing_or_unknown_tutorial() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    for cmd in ["learn.openSample", "learn.finish", "learn.next"] {
        for (params, error) in [
            (json!({}), "missing `tutorial` (see learn.list)"),
            (json!({"tutorial": null}), "missing `tutorial` (see learn.list)"),
            (json!({"tutorial": 3}), "`tutorial` must be a string"),
            (json!({"tutorial": "nope"}), "unknown tutorial `nope` (see learn.list)"),
        ] {
            assert_eq!(app.run(cmd, params).unwrap_err(), error, "{cmd}");
            assert_eq!(app.ui.learn, View::default());
            assert_eq!(app.ui.learn_practice, Practice::default());
            assert!(app.ui.learn_progress.is_empty());
            assert_eq!(app.session.documents().len(), 1);
            assert!(!shown(&app));
        }
    }
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

/// The Pen tutorial's steps, as the panel shows them.
fn pen_paths(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    app.ui.learn = View { tab: Tab::Skills, skill: Some("draw"), tutorial: Some("pen-paths") };
    show(app, ui);
}

/// A step's tool name is a link; a click on it locates that tool without choosing it, and
/// `learn.show` gives the step's text without the link's brackets.
#[test]
fn a_tool_name_in_a_step_is_a_link_that_locates_the_tool() {
    let mut app = app();
    let ctx = context();
    let nodes = frame(&mut app, &ctx, vec![], pen_paths);
    let link = nodes.iter().find(|n| n.label().is_some_and(|l| l == "Pen tool")).expect("the Pen tool link");
    assert_eq!(link.role(), egui::accesskit::Role::Link);
    assert_eq!(nodes.iter().filter(|n| n.role() == egui::accesskit::Role::Link).count(), 1, "only the step that names a tool has a link");
    click(&mut app, &ctx, "Pen tool", pen_paths);
    let located = app.ui.learn_locate.as_ref().map(|l| (l.tool, l.target.clone()));
    assert_eq!(located, Some(("pen", super::locate::Target::Tools)), "{}", app.ui.status);
    assert_eq!(app.session.tool_id(), "selection", "the link does not choose the tool");
    let shown_tutorial = app.run("learn.show", json!({"tutorial": "pen-paths"})).unwrap();
    assert_eq!(shown_tutorial["steps"][0], "Choose the Pen tool.");
}

/// Enter on the link, once Tab has moved the keyboard focus to it, locates the tool.
#[test]
fn enter_on_a_focused_tool_link_locates_the_tool() {
    let mut app = app();
    let ctx = context();
    let link = frame(&mut app, &ctx, vec![], pen_paths)
        .into_iter()
        .find(|n| n.label().is_some_and(|l| l == "Pen tool"))
        .and_then(|n| n.bounds())
        .expect("the Pen tool link");
    let key = |key, pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let focused_on_link = |ctx: &egui::Context| {
        ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id)).is_some_and(|r| {
            let c = r.rect.center();
            ((link.x0 + link.x1) / 2.0 - f64::from(c.x)).abs() < 1.0 && ((link.y0 + link.y1) / 2.0 - f64::from(c.y)).abs() < 1.0
        })
    };
    let mut found = false;
    for _ in 0..30 {
        frame(&mut app, &ctx, vec![key(egui::Key::Tab, true), key(egui::Key::Tab, false)], pen_paths);
        if focused_on_link(&ctx) {
            found = true;
            break;
        }
    }
    assert!(found, "Tab reaches the link");
    assert!(app.ui.learn_locate.is_none());
    frame(&mut app, &ctx, vec![key(egui::Key::Enter, true), key(egui::Key::Enter, false)], pen_paths);
    frame(&mut app, &ctx, vec![], pen_paths);
    assert_eq!(app.ui.learn_locate.as_ref().map(|l| l.tool), Some("pen"));
}

/// A missing, mistyped or unknown tool id is an error and changes nothing.
#[test]
fn learn_locate_tool_rejects_a_missing_or_unknown_tool() {
    let mut app = app();
    app.ui.toolbar = false;
    for (params, error) in [
        (json!({}), "tool (a tool id) is required"),
        (json!({"tool": null}), "tool (a tool id) is required"),
        (json!({"tool": 3}), "`tool` must be a string"),
        (json!({"tool": "nope"}), "unknown tool `nope`"),
    ] {
        assert_eq!(app.run("learn.locateTool", params).unwrap_err(), error);
        assert!(app.ui.learn_locate.is_none() && !app.ui.toolbar && !app.ui.tools_drawer && app.ui.flyout.is_none());
    }
}

/// With the toolbars hidden by Presentation Mode or by Tab, nothing can be marked: the reply says
/// so and the screen stays as it is.
#[test]
fn learn_locate_tool_is_unavailable_while_the_toolbars_are_hidden() {
    let mut app = app();
    app.ui.screen_mode = 3;
    let reply = app.run("learn.locateTool", json!({"tool": "pen"})).unwrap();
    assert_eq!((reply["location"].as_str(), reply["reason"].is_string()), (Some("unavailable"), true), "{reply}");
    assert!(app.ui.learn_locate.is_none());
    app.ui.screen_mode = 0;
    app.run("window.dock", json!({})).unwrap();
    assert!(app.ui.tab_hid.is_some() && !app.ui.toolbar);
    let reply = app.run("learn.locateTool", json!({"tool": "pen"})).unwrap();
    assert_eq!(reply["location"], "unavailable", "{reply}");
    assert!(app.ui.learn_locate.is_none() && !app.ui.toolbar && app.ui.tab_hid.is_some(), "Tab's state stays");
}

/// The screen-reader name of step `i` of the Pen tutorial's check box.
fn pen_step(i: usize) -> String {
    let (_, tutorial) = catalog::tutorial("pen-paths").unwrap();
    format!("{}. {}", i + 1, tutorial.steps[i].plain())
}

/// The check boxes of what `draw` draws, from the top, as (enabled, checked).
fn check_boxes(app: &mut VectorcraftApp, ctx: &egui::Context, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<(bool, bool)> {
    let nodes = frame(app, ctx, vec![], draw);
    let mut boxes: Vec<&Node> = nodes.iter().filter(|n| n.role() == Role::CheckBox).collect();
    boxes.sort_by(|a, b| a.bounds().map_or(0.0, |r| r.y0).total_cmp(&b.bounds().map_or(0.0, |r| r.y0)));
    boxes.iter().map(|n| (!n.is_disabled(), n.toggled() == Some(Toggled::True))).collect()
}

/// Each step has a check box, off until the tutorial's sample is open. A click marks only its own
/// step and a second click clears it; marking every step does not finish the tutorial. A finished
/// tutorial shows every step done, and its check boxes and `learn.markStep` no longer change them.
#[test]
fn a_step_s_check_box_marks_only_that_step() {
    let mut app = app();
    let ctx = context();
    assert_eq!(check_boxes(&mut app, &ctx, pen_paths), [(false, false); 3], "off before View more");
    click(&mut app, &ctx, &pen_step(1), pen_paths);
    assert!(app.ui.learn_progress.is_empty(), "a check box that is off changes nothing");
    app.run("learn.openSample", json!({"tutorial": "pen-paths"})).unwrap();
    click(&mut app, &ctx, &pen_step(1), pen_paths);
    assert_eq!(check_boxes(&mut app, &ctx, pen_paths), [(true, false), (true, true), (true, false)], "{}", app.ui.status);
    let list = app.run("learn.list", json!({})).unwrap();
    assert_eq!(list["skills"][1]["tutorials"][0]["completedSteps"], json!([1]));
    assert_eq!(list["skills"][1]["tutorials"][1]["completedSteps"], json!([]), "other tutorials stay as they are");
    click(&mut app, &ctx, &pen_step(1), pen_paths);
    assert!(app.ui.learn_progress.is_empty(), "a second click clears the mark");
    for i in 0..3 {
        click(&mut app, &ctx, &pen_step(i), pen_paths);
    }
    assert_eq!(check_boxes(&mut app, &ctx, pen_paths), [(true, true); 3]);
    assert!(!app.ui.learn_progress.is_finished("pen-paths"), "marking every step does not finish the tutorial");
    assert!(painted_text(&mut app, pen_paths).contains("Finish tutorial"));
    click(&mut app, &ctx, &pen_step(0), pen_paths);
    click(&mut app, &ctx, "Finish tutorial", pen_paths);
    assert_eq!(app.ui.learn_progress.finished, ["pen-paths"]);
    assert!(app.ui.learn_progress.steps.is_empty(), "a finished tutorial keeps no marks of its own");
    assert_eq!(check_boxes(&mut app, &ctx, pen_paths), [(false, true); 3], "Finish marks every step");
    let before = app.ui.learn_progress.clone();
    click(&mut app, &ctx, &pen_step(2), pen_paths);
    let error = app.run("learn.markStep", json!({"tutorial": "pen-paths", "step": 2, "complete": false})).unwrap_err();
    assert_eq!(error, "`pen-paths` is finished (see learn.resetProgress)");
    assert_eq!(app.ui.learn_progress, before);
}

/// The check boxes are reached with Tab and changed with Space.
#[test]
fn a_step_s_check_box_works_from_the_keyboard() {
    let mut app = app();
    let ctx = context();
    app.run("learn.openSample", json!({"tutorial": "pen-paths"})).unwrap();
    frame(&mut app, &ctx, vec![], pen_paths);
    tab_to(&mut app, &ctx, &pen_step(2), pen_paths);
    key(&mut app, &ctx, egui::Key::Space, pen_paths);
    assert_eq!(app.ui.learn_progress.steps.get("pen-paths"), Some(&vec![2]), "{}", app.ui.status);
}

/// Finishing tutorials fills only their own skill's ring and count, once each however often they
/// are finished; the skill's tutorial rows show which are finished; `learn.list` answers the same.
#[test]
fn finishing_tutorials_fills_their_skill_s_ring_and_count() {
    let mut app = app();
    let ctx = context();
    for id in ["pen-paths", "smooth-curves"] {
        app.run("learn.openSample", json!({"tutorial": id})).unwrap();
        for _ in 0..2 {
            app.run("learn.finish", json!({"tutorial": id})).unwrap();
        }
    }
    let values = |app: &mut VectorcraftApp, view: View| {
        app.ui.learn = view;
        let nodes = frame(app, &ctx, vec![], show);
        nodes
            .iter()
            .filter(|n| n.role() == Role::Button)
            .map(|n| (n.label().unwrap_or_default().to_string(), n.value().map(str::to_string)))
            .collect::<Vec<_>>()
    };
    let skills = View { tab: Tab::Skills, skill: None, tutorial: None };
    let rows = values(&mut app, skills.clone());
    for skill in catalog::SKILLS {
        let count = if skill.id == "draw" { "2/3".to_string() } else { format!("0/{}", skill.tutorials.len()) };
        assert!(rows.contains(&(skill.title.to_string(), Some(count.clone()))), "{} {count} in {rows:?}", skill.id);
    }
    assert!(painted_text(&mut app, show).contains("2/3"));
    let list = app.run("learn.list", json!({})).unwrap();
    let draw = &list["skills"][1];
    assert_eq!((draw["done"].clone(), draw["total"].clone()), (json!(2), json!(3)));
    assert_eq!(draw["tutorials"][1]["finished"], json!(true));
    assert_eq!(draw["tutorials"][1]["completedSteps"], json!([0, 1, 2]));
    assert_eq!(list["skills"][2]["done"], json!(0), "other skills stay at 0");

    let rows = values(&mut app, View { tab: Tab::Skills, skill: Some("draw"), tutorial: None });
    assert!(rows.contains(&("Draw paths with the Pen".to_string(), Some("Finished".to_string()))), "{rows:?}");
    assert!(rows.contains(&("Draw smooth curves".to_string(), Some("Finished".to_string()))), "{rows:?}");
    assert!(rows.contains(&("Sketch with the Pencil".to_string(), None)), "{rows:?}");

    app.run("learn.openSample", json!({"tutorial": "pencil-sketch"})).unwrap();
    app.run("learn.finish", json!({"tutorial": "pencil-sketch"})).unwrap();
    let rows = values(&mut app, skills);
    assert!(rows.contains(&("Draw and build".to_string(), Some("3/3".to_string()))), "every tutorial finished: {rows:?}");
}

/// A skill row with no tutorials, which the catalog does not have today, shows an empty ring and
/// 0/0.
fn empty_skill(_: &mut VectorcraftApp, ui: &mut egui::Ui) {
    super::row(ui, "Empty skill", super::Mark::Ring { done: 0, total: 0 });
}

#[test]
fn a_skill_with_no_tutorials_shows_0_of_0() {
    let mut app = app();
    let text = painted_text(&mut app, empty_skill);
    assert!(text.contains("Empty skill") && text.contains("0/0"), "{text}");
}

/// `learn.markStep` marks a step done by default and with `complete: true`, clears it with `false`,
/// and answers what it did; a bad param is an error and changes nothing.
#[test]
fn learn_mark_step_checks_its_params() {
    let mut app = app();
    let open = "open the practice sample of `gradient` first (see learn.openSample)";
    assert_eq!(app.run("learn.markStep", json!({"tutorial": "gradient", "step": 0})).unwrap_err(), open);
    assert!(app.ui.learn_progress.is_empty());
    app.run("learn.openSample", json!({"tutorial": "gradient"})).unwrap();
    assert_eq!(
        app.run("learn.markStep", json!({"tutorial": "gradient", "step": 0})).unwrap(),
        json!({"tutorial": "gradient", "step": 0, "complete": true})
    );
    let before = app.ui.learn_progress.clone();
    let missing = "missing `step` (see learn.show)";
    let range = "`step` must be a step number from 0 to 2";
    for (params, error) in [
        (json!({"tutorial": "gradient"}), missing),
        (json!({"tutorial": "gradient", "step": null}), missing),
        (json!({"tutorial": "gradient", "step": 3}), range),
        (json!({"tutorial": "gradient", "step": -1}), range),
        (json!({"tutorial": "gradient", "step": 1.5}), range),
        (json!({"tutorial": "gradient", "step": "1"}), range),
        (json!({"tutorial": "gradient", "step": u64::MAX}), range),
        (json!({"tutorial": "gradient", "step": 1, "complete": "yes"}), "`complete` must be true or false"),
        (json!({"tutorial": "nope", "step": 1}), "unknown tutorial `nope` (see learn.list)"),
        (json!({"step": 1}), "missing `tutorial` (see learn.list)"),
    ] {
        assert_eq!(app.run("learn.markStep", params.clone()).unwrap_err(), error, "{params}");
        assert_eq!(app.ui.learn_progress, before, "{params}");
    }
    let r = app.run("learn.markStep", json!({"tutorial": "gradient", "step": 2, "complete": null})).unwrap();
    assert_eq!(r, json!({"tutorial": "gradient", "step": 2, "complete": true}));
    let r = app.run("learn.markStep", json!({"tutorial": "gradient", "step": 0, "complete": false})).unwrap();
    assert_eq!(r, json!({"tutorial": "gradient", "step": 0, "complete": false}));
    let r = app.run("learn.markStep", json!({"tutorial": "gradient", "step": 0, "complete": false})).unwrap();
    assert_eq!(r["complete"], json!(false), "clearing a step that is not marked is no error");
    assert_eq!(app.ui.learn_progress.steps.get("gradient"), Some(&vec![2]));
    assert!(app.ui.learn_progress.finished.is_empty());
}

/// `learn.resetProgress` and Reset progress clear every tutorial finished and every step marked;
/// documents, opened samples and the Learn view stay. Reset progress shows only when there is
/// progress.
#[test]
fn reset_progress_clears_only_the_progress() {
    let mut app = app();
    let ctx = context();
    let skills = View { tab: Tab::Skills, skill: None, tutorial: None };
    app.ui.learn = skills.clone();
    assert!(!painted_text(&mut app, show).contains("Reset progress"));
    app.run("learn.openSample", json!({"tutorial": "pen-paths"})).unwrap();
    app.run("learn.finish", json!({"tutorial": "pen-paths"})).unwrap();
    app.run("learn.openSample", json!({"tutorial": "gradient"})).unwrap();
    app.run("learn.markStep", json!({"tutorial": "gradient", "step": 1})).unwrap();
    let view = app.ui.learn.clone();
    let docs = app.session.documents().len();
    let reply = app.run("learn.resetProgress", json!({})).unwrap();
    assert!(app.ui.learn_progress.is_empty());
    assert_eq!(reply, app.run("learn.list", json!({})).unwrap());
    assert_eq!(reply["skills"][1]["done"], json!(0));
    assert_eq!((app.ui.learn.clone(), app.session.documents().len()), (view, docs));
    assert_eq!(app.ui.learn_practice.opened, ["pen-paths", "gradient"], "opened samples stay open for practice");

    app.run("learn.finish", json!({"tutorial": "gradient"})).unwrap();
    app.ui.learn = skills.clone();
    assert!(painted_text(&mut app, show).contains("Reset progress"));
    click(&mut app, &ctx, "Reset progress", show);
    assert!(app.ui.learn_progress.is_empty(), "{}", app.ui.status);
    assert_eq!(app.ui.learn, skills);
    assert!(!painted_text(&mut app, show).contains("Reset progress"));
}

/// The UI state after its preferences are saved and read again, as at the next start.
fn restarted(app: &VectorcraftApp) -> crate::state::UiState {
    let bytes = serde_json::to_vec(&app.ui).unwrap();
    crate::state::UiState::read_preferences(&bytes).expect("the preferences read").ui.sanitized()
}

/// The preferences keep the tutorials finished and the steps marked; the opened samples, the
/// Learn view and a tool's mark last only while the app runs, so a step can be marked again only
/// after View more.
#[test]
fn the_preferences_keep_the_progress() {
    let mut app = app();
    app.run("learn.openSample", json!({"tutorial": "pen-paths"})).unwrap();
    app.run("learn.finish", json!({"tutorial": "pen-paths"})).unwrap();
    app.run("learn.openSample", json!({"tutorial": "gradient"})).unwrap();
    for step in [2, 0] {
        app.run("learn.markStep", json!({"tutorial": "gradient", "step": step})).unwrap();
    }
    let list = app.run("learn.list", json!({})).unwrap();
    let ui = restarted(&app);
    assert_eq!(ui.learn_progress, app.ui.learn_progress);
    assert_eq!(ui.learn_progress.steps.get("gradient"), Some(&vec![0, 2]));
    assert_eq!((ui.learn_practice.clone(), ui.learn.clone(), ui.learn_locate.is_none()), (Practice::default(), View::default(), true));

    let mut next = self::app();
    next.ui = ui;
    assert_eq!(next.run("learn.list", json!({})).unwrap(), list);
    assert_eq!(next.run("learn.next", json!({"tutorial": "pen-paths"})).unwrap()["tutorial"], "smooth-curves");
    let again = next.run("learn.finish", json!({"tutorial": "pen-paths"})).unwrap();
    assert_eq!(again, json!({"tutorial": "pen-paths", "finished": true, "next": "smooth-curves"}), "finishing again needs no sample");
    assert_eq!(next.ui.learn_progress.finished, ["pen-paths"]);
    let open = "open the practice sample of `gradient` first (see learn.openSample)";
    assert_eq!(next.run("learn.markStep", json!({"tutorial": "gradient", "step": 1})).unwrap_err(), open);
}

/// Saved preferences with `learn_progress` set to `progress` (left out for None), and
/// `control_bar` on, read as at a start.
fn read_progress(progress: Option<Value>) -> crate::state::UiState {
    let mut saved = serde_json::to_value(crate::state::UiState::default()).unwrap();
    let fields = saved.as_object_mut().unwrap();
    fields.insert("control_bar".into(), json!(true));
    match progress {
        Some(p) => fields.insert("learn_progress".into(), p),
        None => fields.remove("learn_progress"),
    };
    let ui = crate::state::UiState::read_preferences(&serde_json::to_vec(&saved).unwrap()).expect("the preferences read").ui;
    assert!(ui.control_bar, "the other preferences read as usual");
    ui
}

/// Preferences saved before the progress was kept read with none. Saved progress keeps only the
/// catalog's tutorials and steps, each once, and a value of the wrong kind, or a huge list, does not
/// stop the rest of the preferences from reading.
#[test]
fn saved_progress_is_read_leniently() {
    assert!(read_progress(None).learn_progress.is_empty());
    for bad in [json!("garbage"), json!(3), json!(null), json!([1, 2]), json!({"finished": "pen-paths", "steps": [0]})] {
        assert!(read_progress(Some(bad.clone())).learn_progress.is_empty(), "{bad}");
    }
    let ui = read_progress(Some(json!({
        "finished": ["pen-paths", "nope", 3, "pen-paths", "gradient"],
        "steps": {
            "gradient": [0],
            "smooth-curves": [2, 0, 2, 99, -1, "1", 1.5],
            "nope": [0],
            "copy-colors": "x",
            "add-headline": [],
        },
    })));
    assert_eq!(ui.learn_progress.finished, ["pen-paths", "gradient"]);
    let steps: Vec<(&str, Vec<usize>)> = ui.learn_progress.steps.iter().map(|(id, s)| (*id, s.clone())).collect();
    assert_eq!(steps, [("smooth-curves", vec![0, 2])], "a finished tutorial keeps no marks");
    let ui = read_progress(Some(json!({"finished": vec!["pen-paths"; 100_000], "steps": {"smooth-curves": vec![1; 100_000]}})));
    assert_eq!(ui.learn_progress.finished, ["pen-paths"]);
    assert_eq!(ui.learn_progress.steps.get("smooth-curves"), Some(&vec![1]));
}
