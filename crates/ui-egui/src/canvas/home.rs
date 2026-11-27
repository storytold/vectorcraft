use craft::behavior::{ColumnWidth, IconId};
use craft::ui::buttons::{Link, LinkLook};
use craft::ui::contract::{Bind, Gap, Pad, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::display::Text;
use craft::ui::layout::CellAlign;
use craft::ui::layout_components::{Grid, Stack};
use craft::ui::runtime::env::{Env, Overrides, with_env};
use craft::ui::sections::Section;
use craft::ui::shell::{RecentFile, StartAction, StartLayout, StartLink, StartPlace, StartScreen, Template};
use craft::ui::style::{SurfaceKind, TypeRole};

use super::{Ui, VectorcraftApp, json};

/// The presets the Home screen offers as templates (`file.newPresets`).
const TEMPLATES: [&str; 6] = ["Letter", "A4", "Web 1920×1080", "Phone 390×844", "Postcard", "Social Square Post 1080×1080"];

/// The icons of the three `community::link_urls` links, in their order.
const LINK_ICONS: [&str; 3] = ["globe", "external-link", "git-branch"];

/// The Learn cards across a full-width column, and across a narrower one.
const FOUR: &[ColumnWidth] = &[ColumnWidth::fill(1.0); 4];
const TWO: &[ColumnWidth] = &[ColumnWidth::fill(1.0); 2];

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
        .map(|s| Template::new(s.name.clone(), s.name.clone(), s.size_label()).page(s.width, s.height))
        .collect();
    let mut links = vec![];
    let discord = crate::credits::discord_url(app);
    if !discord.is_empty() {
        links.push(StartLink::url(tl!("Join our Discord"), Some(IconId::new("message-circle")), discord).primary());
    }
    for ((label, url), icon) in crate::community::link_urls(app).into_iter().zip(LINK_ICONS) {
        links.push(StartLink::url(tl!(label), Some(IconId::new(icon)), url));
    }
    let mut pressed = None;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
    // Production's community links read in the accent alone, with no underline.
    let plain_links = |overrides: &mut Overrides| {
        overrides.values.set(LinkLook::Plain);
    };
    // Learn follows the presets, which stand right under New and Open as in production 0.24.1.
    let start = StartScreen::new(Bind::bound(&mut app.ui.home_view), Bind::bound(&mut app.ui.home_sort), &recent)
        .layout(StartLayout::Column)
        .title(tl!("Welcome to VectorCraft"))
        .subtitle(tl!("Vector illustration — fast, open, scriptable."))
        .new_label(tl!("New file"))
        .open_label(tl!("Open"))
        .max_recent(6)
        .templates(&templates)
        .templates_title(tl!("Quickly start a new file"))
        .links(&links)
        .links_title(tl!("Community"))
        .section(StartPlace::AfterSections, |ui, _| learn(ui, &mut pressed));
    let out = with_env(&mut child, plain_links, |ui| start.show(ui));
    for action in out.actions {
        run_action(app, action);
    }
    if let Some((command, args)) = pressed
        && let Err(e) = app.run(command, args)
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

/// The Home screen's Learn section, after the new-file presets: a Column page section whose heading row ends in the
/// Quick skill guide link, which shows the Learn panel's list of skills (`learn.skills`), over a card per skill: its
/// title over its tutorials as links, each showing the panel at its steps (`learn.show`). The four cards stand in one
/// row in a column at its full `size.page.column` width, and two by two in a narrower one. The start screen may run
/// this twice in a pass, so it only draws and records the command chosen in `pressed`.
fn learn(ui: &mut Ui, pressed: &mut Option<(&'static str, serde_json::Value)>) {
    use crate::panels::learn::catalog::SKILLS;
    let (mut guide, mut tutorial) = (false, None);
    let across = if ui.available_width() + 0.5 >= Env::of(ui).tokens.size.page.column { FOUR } else { TWO };
    let section = Section::fixed(tl!("Learn")).quiet_header().header_gap(Gap::PageHeading);
    let _ = section.controls(|ui, _| guide |= clicked(&Link::new(tl!("Quick skill guide")).show(ui))).show(ui, |ui, _| {
        let _ = Grid::new(across).gap(Gap::Section).cells(CellAlign::Top).fill_width().show(ui, |ui, _| {
            for skill in SKILLS {
                let card = Stack::new().surface(SurfaceKind::Raised).pad(Pad::Regular).gap(Gap::Stack).fill_width();
                let _ = card.show(ui, |ui, _| {
                    let _ = Text::new(skill.title).role(TypeRole::BodyStrong).show(ui);
                    let _ = Stack::new().gap(Gap::PageLink).show(ui, |ui, _| {
                        for t in skill.tutorials {
                            if clicked(&Link::new(t.title).id_salt(t.id).show(ui)) {
                                tutorial = Some(t.id);
                            }
                        }
                    });
                });
            }
        });
    });
    if guide {
        *pressed = Some(("learn.skills", json!({})));
    } else if let Some(id) = tutorial {
        *pressed = Some(("learn.show", json!({ "tutorial": id })));
    }
}
