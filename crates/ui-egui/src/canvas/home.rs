use craft::behavior::IconId;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::shell::{RecentFile, StartAction, StartLink, StartScreen, Template};

use super::{Ui, VectorcraftApp, json};

/// The presets the Home screen offers as templates (`file.newPresets`).
const TEMPLATES: [&str; 6] = ["Letter", "A4", "Web 1920×1080", "Phone 390×844", "Postcard", "Social Square Post 1080×1080"];

/// The icons of the three `community::link_urls` links, in their order.
const LINK_ICONS: [&str; 3] = ["globe", "external-link", "git-branch"];

/// The Home screen (`app.home`; with no document open, unless its preference is off).
pub(super) fn home(app: &mut VectorcraftApp, ui: &mut Ui, rect: egui::Rect) {
    let recent: Vec<RecentFile> = crate::io::recent_files(app)
        .iter()
        .map(|path| {
            let name = std::path::Path::new(path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            RecentFile::new(path, name)
        })
        .collect();
    let templates: Vec<Template> = TEMPLATES
        .iter()
        .filter_map(|name| vectorcraft_engine::cmd::newdoc::find(&app.session, name))
        .map(|s| Template::new(s.name.clone(), s.name.clone(), s.size_label()))
        .collect();
    let mut links = vec![];
    let discord = crate::credits::discord_url(app);
    if !discord.is_empty() {
        links.push(StartLink::url(tl!("Join our Discord"), Some(IconId::new("message-circle")), discord));
    }
    for ((label, url), icon) in crate::community::link_urls(app).into_iter().zip(LINK_ICONS) {
        links.push(StartLink::url(tl!(label), Some(IconId::new(icon)), url));
    }
    // The bindings hold copies so the Learn section can borrow the app.
    let (mut view, mut sort) = (app.ui.home_view, app.ui.home_sort);
    let mut pressed = None;
    let mut learn = |ui: &mut Ui| learn(app, ui, &mut pressed);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
    let out = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &recent)
        .max_recent(6)
        .templates(&templates)
        .links(&links)
        .app_section(tl!("Learn"), &mut learn)
        .show(&mut child);
    app.ui.home_view = view;
    app.ui.home_sort = sort;
    for action in out.actions {
        run_action(app, action);
    }
    if let Some(id) = pressed
        && let Err(e) = app.run("learn.show", json!({ "tutorial": id }))
    {
        app.status(e);
    }
}

/// What the start screen's user did.
fn run_action(app: &mut VectorcraftApp, action: StartAction) {
    match action {
        StartAction::New => drop(app.run("file.newDialog", json!({}))),
        StartAction::Open => drop(app.run("file.open", json!({}))),
        // A failure is reported to the user there.
        StartAction::OpenRecent(path) => drop(crate::io::open_reporting(app, &path.to_string_lossy())),
        StartAction::UseTemplate(name) => drop(app.run("file.new", json!({ "preset": name }))),
        StartAction::ClearRecent => drop(app.run("file.clearRecent", json!({}))),
        StartAction::RemoveRecent(path) => app.ui.recent_files.retain(|recent| std::path::Path::new(recent) != path),
        StartAction::RevealRecent(path) => {
            // PLAN: `io::export` is a private module of `io`; `reveal_path` needs a re-export from io.rs (unowned).
            if let Err(e) = crate::io::export::reveal_path(app, &path.to_string_lossy()) {
                app.status(e);
            }
        }
        // The Home screen offers no pinned folders.
        _ => {}
    }
}

/// The Home screen's Learn section: the Quick skill guide card, which shows the Learn panel's list
/// of skills, and the Learn panel's tutorials under their skills, each a button that shows the
/// panel at its steps (`learn.show`). The start screen may run this twice in a pass, so it only
/// draws and records the tutorial pressed in `pressed`.
fn learn(app: &mut VectorcraftApp, ui: &mut Ui, pressed: &mut Option<&'static str>) {
    use crate::panels::learn::{catalog, guide_card};
    guide_card(app, ui); // PLAN: assumed `guide_card(app, ui)` as document-panels-w18 leaves it, on `Button::detail`.
    for skill in catalog::SKILLS {
        let _ = Text::new(skill.title).show(ui);
        let _ = Row::new().show(ui, |ui, _| {
            for tutorial in skill.tutorials {
                if Button::new(tutorial.title).id_salt(tutorial.id).show(ui).clicked() {
                    *pressed = Some(tutorial.id);
                }
            }
        });
    }
}
