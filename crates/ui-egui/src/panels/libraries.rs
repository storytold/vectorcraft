//! Libraries panel (Window › Libraries, #745): the current library's colours, character and
//! paragraph styles and graphics, from the engine's `library.*` commands. Art dragged off the canvas
//! onto the panel, and swatches dragged in, are added to it; the + button adds the selection's
//! graphic, fill or stroke colour or text styles. A colour click paints the selection through the
//! active Fill/Stroke proxy, a style click applies it to the selected type, and a graphic is
//! dragged onto the canvas (or double-clicked) to place a copy. Libraries are kept on this machine.
//!
//! The search field filters the items by name; items can be moved into user-named groups, shown as
//! collapsible sections after the ungrouped items; and the panel menu exports the library to a
//! `.vclibrary` file and imports one as a new library (#926).

use std::collections::HashMap;

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_engine::cmd::library::{ItemKind, LIBRARY_EXT, LIBRARY_EXTS, Library, LibraryGraphic};

use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item, menu_item_name};
use crate::{VectorcraftApp, icons};

/// The size of a graphic's tile.
const TILE: f32 = 56.0;
/// The size of a colour chip.
const CHIP: f32 = 22.0;

/// The current library: its id and a copy of it (the panel menu acts on it after reading).
fn current(app: &VectorcraftApp) -> Option<(String, Library)> {
    let libs = &app.session.libraries;
    let id = libs.current()?;
    Some((id.to_string(), libs.get(id)?.clone()))
}

/// Report a failure in the status bar (a file dialog closed or still open says nothing).
fn report<T>(app: &mut VectorcraftApp, r: Result<T, String>) {
    if let Err(e) = r
        && e != "cancelled"
    {
        app.status(e);
    }
}

/// Run `cmd`, reporting a failure in the status bar.
fn run(app: &mut VectorcraftApp, cmd: &str, p: Value) {
    let r = app.run(cmd, p);
    report(app, r);
}

/// The name dialog for a new library, or for renaming the current one.
fn ask_name(app: &mut VectorcraftApp, rename: Option<&str>) {
    let (cmd, label, name) = match rename {
        Some(n) => ("library.rename", "Rename Library", n.to_string()),
        None => ("library.create", "Create New Library", "My Library".to_string()),
    };
    app.ui.dialog = Some(crate::state::Dialog::new("command", json!({ "__command": cmd, "__label": label, "name": name })));
}

/// The name dialog for a new group of `library` (`items`: `[{kind, item}]` moved into it), or for
/// renaming group `rename`.
fn ask_group_name(app: &mut VectorcraftApp, library: &str, rename: Option<&str>, items: Value) {
    let (cmd, label, name, fixed) = match rename {
        Some(g) => ("library.renameGroup", "Rename Library Group", g, json!({ "library": library, "group": g })),
        None => ("library.createGroup", "New Library Group", "Group", json!({ "library": library, "items": items })),
    };
    let d = json!({ "__command": cmd, "__label": label, "name": name, "__params": fixed });
    app.ui.dialog = Some(crate::state::Dialog::new("command", d));
}

/// What a click in the panel asks for, done once the list is drawn.
enum Act {
    Run(&'static str, Value),
    /// The New Group dialog; the items (`[{kind, item}]`) go into the new group.
    NewGroup(Value),
    RenameGroup(String),
}

/// What the item list draws from.
struct Items<'a> {
    library: &'a str,
    lib: &'a Library,
    /// The group of each grouped item ([`Library::group_index`]).
    groups: HashMap<(ItemKind, &'a str), usize>,
    fill_active: bool,
}

impl Items<'_> {
    fn group_of(&self, kind: ItemKind, key: &str) -> Option<usize> {
        self.groups.get(&(kind, key)).copied()
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let fill_active = app.session.fill_active;
    let has_doc = app.session.active().is_some();
    let libs = &app.session.libraries;
    let Some((lib_id, lib)) = libs.current().and_then(|id| Some((id.to_string(), libs.get(id)?))) else {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            icons::icon(ui, "library", 40.0, t.text_dim);
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tl!("Local Libraries")).size(14.0).color(t.text));
            widgets::dim_label(ui, tl!("Keep graphics, colors and text styles here to use them in any document. Libraries are stored on this machine — no account required."));
            ui.add_space(8.0);
            if ui.button(tl!("Create New Library")).clicked() {
                ask_name(app, None);
            }
        });
        return;
    };
    let mut act: Option<Act> = None;
    // The library picker.
    let names: Vec<&str> = libs.all().iter().map(|(_, l)| l.name.as_str()).collect();
    if let Some((id, _)) = widgets::dropdown_names(ui, "lib-picker", &lib.name, &names, ui.available_width()).and_then(|i| libs.all().get(i)) {
        act = Some(Act::Run("library.setCurrent", json!({ "library": id })));
    }
    ui.add_space(4.0);
    let empty = lib.items().next().is_none() && lib.groups.is_empty();
    let query = if empty { String::new() } else { widgets::search_field(ui, egui::Id::new("library-search"), tl!("Search")) };
    let query = query.trim().to_lowercase();
    let searching = !query.is_empty();
    let shown = |name: &str| !searching || name.to_lowercase().contains(&query);
    let items = Items { library: &lib_id, lib, groups: lib.group_index(), fill_active };
    // The items shown in each section: the ungrouped ones first, then each group's.
    let mut counts = vec![0usize; lib.groups.len() + 1];
    for (kind, key, _) in lib.items().filter(|(_, _, name)| shown(name)) {
        if let Some(n) = counts.get_mut(items.group_of(kind, key).map_or(0, |i| i + 1)) {
            *n += 1;
        }
    }
    ui.add_space(4.0);
    let list_rect = widgets::list_box(ui, |ui| {
        ui.set_min_height(140.0);
        ui.set_width(ui.available_width());
        if empty {
            super::empty_state(
                ui,
                "library",
                tl!("This library is empty"),
                tl!("Drag art or swatches here, or use + to add the selection's colors and text styles."),
            );
            return ui.min_rect();
        }
        if searching && counts.iter().all(|n| *n == 0) {
            let body = crate::i18n::fmt(tl!("No items in this library match “{query}”."), &[("query", &query)]);
            super::empty_state(ui, "search", tl!("No results"), &body);
            return ui.min_rect();
        }
        egui::Frame::NONE.inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            items.show(ui, &mut act, |kind, key, name| items.group_of(kind, key).is_none() && shown(name));
            for (i, g) in lib.groups.iter().enumerate() {
                let n = counts.get(i + 1).copied().unwrap_or_default();
                if searching && n == 0 {
                    continue;
                }
                // Open unless closed; open while searching, to show what matched.
                let key = format!("library-group-closed:{lib_id}:{}", g.name);
                let closed: bool = super::pstate(ui.ctx(), &key);
                let open = searching || !closed;
                let header = widgets::section_toggle_name(ui, &g.name, open);
                ui.painter().text(
                    header.rect.right_center() - vec2(6.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    n.to_string(),
                    egui::FontId::proportional(11.5),
                    t.text_dim,
                );
                if header.clicked() && !searching {
                    super::set_pstate(ui.ctx(), &key, !closed);
                }
                header.context_menu(|ui| {
                    if menu_item(ui, "Rename Group…", true, false) {
                        act = Some(Act::RenameGroup(g.name.clone()));
                        ui.close();
                    }
                    if menu_item(ui, "Delete Group", true, false) {
                        act = Some(Act::Run("library.deleteGroup", json!({ "group": g.name })));
                        ui.close();
                    }
                });
                if !open {
                    continue;
                }
                ui.add_space(2.0);
                if n == 0 {
                    ui.label(egui::RichText::new(tl!("Right-click an item to move it into this group.")).size(11.5).color(t.text_dim));
                    ui.add_space(6.0);
                } else {
                    items.show(ui, &mut act, |kind, key, name| items.group_of(kind, key) == Some(i) && shown(name));
                }
            }
        });
        ui.min_rect()
    });
    // Art dragged off the canvas, or a swatch, dropped on the list is added to the library.
    let zone = ui.interact(list_rect, ui.id().with("libraries-drop"), Sense::hover());
    if let Some(ids) = widgets::art_drop(ui, &zone) {
        act = Some(Act::Run("library.add", json!({ "kind": "graphic", "ids": ids })));
    }
    if let Some(drag) = zone.dnd_hover_payload::<PanelDrag>()
        && let PanelDrag::Paint { paint: Paint::Solid { color, .. }, .. } = &*drag
    {
        ui.painter().rect_stroke(zone.rect, 0.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
        let color = *color;
        if zone.dnd_release_payload::<PanelDrag>().is_some() {
            act = Some(Act::Run("library.add", json!({ "kind": "fillColor", "color": color })));
        }
    }
    widgets::bottom_bar(ui, |ui| {
        if widgets::icon_button(ui, "group", tl!("New Group"), false, 24.0).clicked() {
            act = Some(Act::NewGroup(json!([])));
        }
        let add = widgets::icon_button_enabled(ui, "plus", tl!("Add Content"), false, has_doc, 24.0);
        egui::Popup::menu(&add).show(|ui| {
            for (label, kind) in [
                ("Graphic", "graphic"),
                ("Fill Color", "fillColor"),
                ("Stroke Color", "strokeColor"),
                ("Character Style", "charStyle"),
                ("Paragraph Style", "paraStyle"),
            ] {
                if menu_item(ui, label, true, false) {
                    act = Some(Act::Run("library.add", json!({ "kind": kind })));
                    ui.close();
                }
            }
        });
    });
    match act {
        Some(Act::Run(cmd, mut p)) => {
            if cmd != "library.setCurrent" {
                p["library"] = json!(lib_id);
            }
            if cmd == "library.use" && p["kind"] == "graphic" {
                // Placed at the centre of the view.
                if let Some(v) = app.view() {
                    p["center"] = json!([v.center.x, v.center.y]);
                }
            }
            run(app, cmd, p);
        }
        Some(Act::NewGroup(items)) => ask_group_name(app, &lib_id, None, items),
        Some(Act::RenameGroup(g)) => ask_group_name(app, &lib_id, Some(&g), Value::Null),
        None => {}
    }
}

impl Items<'_> {
    /// The colours, character and paragraph styles and graphics `keep` keeps (given the kind, key
    /// and name of each), each kind under its heading.
    fn show(&self, ui: &mut Ui, act: &mut Option<Act>, keep: impl Fn(ItemKind, &str, &str) -> bool) {
        let t = Tokens::get(ui.ctx());
        let lib = self.lib;
        let colors: Vec<_> = lib.colors.iter().filter(|c| keep(ItemKind::Color, &c.name, &c.name)).collect();
        if !colors.is_empty() {
            heading(ui, tl!("Colors"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for c in colors {
                    let (r, resp) = ui.allocate_exact_size(vec2(CHIP, CHIP), Sense::click_and_drag());
                    let paint = Paint::solid(c.color);
                    widgets::swatch_tile(ui, r, &paint, false, false);
                    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
                    widgets::drag_source(ui, &resp, || PanelDrag::Paint { paint: paint.clone(), params: json!({ "color": c.color }), rows: None });
                    let resp = resp.on_hover_text(&c.name);
                    if resp.clicked() {
                        let to = if self.fill_active { "fill" } else { "stroke" };
                        *act = Some(Act::Run("library.use", json!({ "kind": "fillColor", "item": c.name, "to": to })));
                    }
                    resp.context_menu(|ui| self.item_menu(ui, act, ItemKind::Color, &c.name));
                }
            });
            ui.add_space(6.0);
        }
        for (kind, list, title) in
            [(ItemKind::CharStyle, &lib.char_styles, tl!("Character Styles")), (ItemKind::ParaStyle, &lib.para_styles, tl!("Paragraph Styles"))]
        {
            let list: Vec<_> = list.iter().filter(|s| keep(kind, &s.name, &s.name)).collect();
            if list.is_empty() {
                continue;
            }
            heading(ui, title);
            for st in list {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                let icon = if kind == ItemKind::ParaStyle { "pilcrow" } else { "type" };
                icons::paint(ui, icon, Rect::from_center_size(r.left_center() + vec2(10.0, 0.0), vec2(14.0, 14.0)), t.icon);
                ui.painter().text(r.left_center() + vec2(24.0, 0.0), egui::Align2::LEFT_CENTER, &st.name, egui::FontId::proportional(12.5), t.text);
                let resp = resp.on_hover_text(tl!("Click to apply to the selected type"));
                if resp.clicked() {
                    *act = Some(Act::Run("library.use", json!({ "kind": kind_param(kind), "item": st.name })));
                }
                resp.context_menu(|ui| self.item_menu(ui, act, kind, &st.name));
            }
            ui.add_space(6.0);
        }
        let graphics: Vec<_> = lib.graphics.iter().filter(|g| keep(ItemKind::Graphic, &g.id, &g.name)).collect();
        if !graphics.is_empty() {
            heading(ui, tl!("Graphics"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for g in graphics {
                    let (r, resp) = ui.allocate_exact_size(vec2(TILE, TILE), Sense::click_and_drag());
                    tile(ui, r, self.library, g);
                    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, if resp.hovered() { t.accent } else { t.border }), StrokeKind::Inside);
                    // Dragged onto the canvas, a copy is placed where it is dropped.
                    widgets::drag_source(ui, &resp, || PanelDrag::LibraryGraphic { library: self.library.to_string(), item: g.id.clone() });
                    let resp = resp.on_hover_text(&g.name);
                    let place = || Act::Run("library.use", json!({ "kind": "graphic", "item": g.id }));
                    if resp.double_clicked() {
                        *act = Some(place());
                    }
                    resp.context_menu(|ui| {
                        if menu_item(ui, "Place", true, false) {
                            *act = Some(place());
                            ui.close();
                        }
                        self.item_menu(ui, act, ItemKind::Graphic, &g.id);
                    });
                }
            });
            ui.add_space(6.0);
        }
    }

    /// An item's context menu entries: Move to Group (No Group, the library's groups, New
    /// Group…) and Delete.
    fn item_menu(&self, ui: &mut Ui, act: &mut Option<Act>, kind: ItemKind, key: &str) {
        let at = self.group_of(kind, key);
        let item = json!({ "kind": kind_param(kind), "item": key });
        ui.menu_button(format!("   {}", tl!("Move to Group")), |ui| {
            if menu_item(ui, "No Group", true, at.is_none()) {
                if at.is_some() {
                    *act = Some(Act::Run("library.moveItem", item.clone()));
                }
                ui.close();
            }
            for (i, g) in self.lib.groups.iter().enumerate() {
                if menu_item_name(ui, &g.name, true, at == Some(i)) {
                    if at != Some(i) {
                        let mut p = item.clone();
                        p["group"] = json!(g.name);
                        *act = Some(Act::Run("library.moveItem", p));
                    }
                    ui.close();
                }
            }
            ui.separator();
            if menu_item(ui, "New Group…", true, false) {
                *act = Some(Act::NewGroup(json!([item])));
                ui.close();
            }
        });
        if menu_item(ui, "Delete", true, false) {
            *act = Some(Act::Run("library.removeItem", item));
            ui.close();
        }
    }
}

/// `kind` as the `library.*` commands' `kind` param (a colour as a fill colour).
fn kind_param(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Graphic => "graphic",
        ItemKind::CharStyle => "charStyle",
        ItemKind::ParaStyle => "paraStyle",
        ItemKind::Color | ItemKind::Unknown => "fillColor",
    }
}

/// A section heading in the list.
fn heading(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).size(11.5).color(t.text_dim));
    ui.add_space(2.0);
}

/// Graphic `g`'s thumbnail on white in `r`.
fn tile(ui: &Ui, r: Rect, library: &str, g: &LibraryGraphic) {
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    if let Some(tex) = thumbnail(ui.ctx(), library, g) {
        let size = tex.size_vec2();
        let k = ((r.width() - 6.0) / size.x).min((r.height() - 6.0) / size.y).min(1.0);
        let fit = Rect::from_center_size(r.center(), size * k);
        ui.painter().image(tex.id(), fit, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

/// The texture of graphic `g`'s PNG thumbnail, decoded once.
fn thumbnail(ctx: &egui::Context, library: &str, g: &LibraryGraphic) -> Option<egui::TextureHandle> {
    thread_local! {
        static CACHE: crate::graphics::TexCache<HashMap<(String, String, usize), Option<egui::TextureHandle>>> = crate::graphics::TexCache::default();
    }
    let key = (library.to_string(), g.id.clone(), g.thumbnail.len());
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return t;
    }
    let tex = vectorcraft_format::base64_decode(&g.thumbnail)
        .and_then(|png| image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok())
        .map(|img| {
            let rgba = img.to_rgba8();
            let image = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            ctx.load_texture(format!("library-{library}-{}", g.id), image, egui::TextureOptions::LINEAR)
        });
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, tex.clone());
    });
    tex
}

/// Graphic `item` of `library`'s thumbnail in `r` (the chip a dragged graphic shows at the pointer).
pub(crate) fn chip(app: &VectorcraftApp, ui: &Ui, r: Rect, library: &str, item: &str) {
    if let Some(g) = app.session.libraries.get(library).and_then(|l| l.graphics.iter().find(|g| g.id == item)) {
        tile(ui, r, library, g);
    }
}

/// Import a library file (`library.import`'s params), saying so in the status bar.
pub(crate) fn import(app: &mut VectorcraftApp, p: Value) -> Result<Value, String> {
    let r = app.run("library.import", p)?;
    let name = r["name"].as_str().unwrap_or_default().to_string();
    app.status(format!("Imported library {name}"));
    Ok(r)
}

/// Import Library…: a picked `.vclibrary` file becomes a new library (on the web the picked file
/// arrives later through [`crate::io::open_bytes`]).
fn import_picked(app: &mut VectorcraftApp) {
    if let Some(f) = app.services.open_async.as_mut() {
        f();
        return;
    }
    let pick = crate::FilePick { filters: vec![("Libraries", LIBRARY_EXTS)], ..Default::default() };
    if let Some(path) = crate::picks::open(app, &pick) {
        let r = import(app, json!({ "path": path }));
        report(app, r);
    }
}

/// Export Library…: library `id` written to a picked file named after it (the web downloads it).
fn export_picked(app: &mut VectorcraftApp, id: &str, name: &str) {
    let file = format!("{}.{LIBRARY_EXT}", vectorcraft_engine::cmd::library::file_stem_for(name));
    let r = crate::io::save_command_output_named(app, "library.export", &file, json!({ "library": id }));
    report(app, r);
}

/// The panel's ≡ menu: Create New Library, Rename Library…, Delete Library, New Group…, Import
/// Library…, Export Library….
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let cur = current(app);
    if menu_item(ui, "Create New Library…", true, false) {
        ask_name(app, None);
        ui.close();
    }
    if menu_item(ui, "Rename Library…", cur.is_some(), false)
        && let Some((_, lib)) = &cur
    {
        ask_name(app, Some(&lib.name));
        ui.close();
    }
    if menu_item(ui, "Delete Library", cur.is_some(), false)
        && let Some((id, lib)) = &cur
    {
        let message = crate::i18n::fmt(tl!("Delete the library “{name}”?"), &[("name", &lib.name)]);
        crate::dialogs::confirm::ask(
            app,
            &message,
            tl!("Its graphics, colors and text styles are deleted from this machine."),
            "library.delete",
            json!({ "library": id }),
        );
        ui.close();
    }
    ui.separator();
    if menu_item(ui, "New Group…", cur.is_some(), false)
        && let Some((id, _)) = &cur
    {
        ask_group_name(app, id, None, json!([]));
        ui.close();
    }
    ui.separator();
    if menu_item(ui, "Import Library…", true, false) {
        crate::picks::button(app, import_picked);
        ui.close();
    }
    if menu_item(ui, "Export Library…", cur.is_some(), false)
        && let Some((id, lib)) = cur
    {
        crate::picks::button(app, move |app| export_picked(app, &id, &lib.name));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(app, ui);
            menu(app, ui);
        });
        out.textures_delta.clear();
    }

    /// The panel draws with no library, an empty one and one holding every kind of item (#745).
    #[test]
    fn the_panel_draws_its_libraries() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        frame(&mut app);
        app.run("library.create", json!({"name": "Brand"})).unwrap();
        frame(&mut app);
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("library.add", json!({"kind": "graphic"})).unwrap();
        app.run("library.add", json!({"kind": "fillColor"})).unwrap();
        let t = app.run("text.create", json!({"x": 10, "y": 80, "text": "Type"})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [t]})).unwrap();
        app.run("library.add", json!({"kind": "charStyle"})).unwrap();
        app.run("library.add", json!({"kind": "paraStyle"})).unwrap();
        frame(&mut app);
        let lib = app.session.libraries.get("Brand").unwrap();
        assert_eq!((lib.graphics.len(), lib.colors.len(), lib.char_styles.len(), lib.para_styles.len()), (1, 1, 1, 1));
        // The thumbnail decodes.
        let ctx = egui::Context::default();
        assert!(thumbnail(&ctx, "Brand", &lib.graphics[0]).is_some());
    }

    /// The panel's text: every label it shows in one frame (`query` typed in the search field).
    fn texts(app: &mut VectorcraftApp, query: &str) -> Vec<String> {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("library-search"), query.to_string()));
        let mut shapes = vec![];
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        shapes.append(&mut out.shapes);
        let mut texts = vec![];
        for s in shapes {
            if let egui::epaint::Shape::Text(t) = s.shape {
                texts.push(t.galley.text().to_string());
            }
        }
        texts
    }

    /// The search field filters items by name, ignoring case; groups show after the ungrouped
    /// items, and a search nothing matches says so (#926).
    #[test]
    fn the_panel_searches_and_shows_groups() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("library.create", json!({"name": "Brand"})).unwrap();
        for (name, color) in [("Ocean Blue", "#0044aa"), ("Sun Yellow", "#ffcc00")] {
            app.run("library.add", json!({"kind": "fillColor", "color": color, "name": name})).unwrap();
        }
        let t = app.run("text.create", json!({"x": 10, "y": 80, "text": "Type"})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [t]})).unwrap();
        app.run("library.add", json!({"kind": "charStyle", "name": "Headline"})).unwrap();
        app.run("library.createGroup", json!({"name": "Warm", "items": [{"kind": "fillColor", "item": "Sun Yellow"}]})).unwrap();
        let all = texts(&mut app, "");
        let at = |s: &str| all.iter().position(|t| t == s);
        assert!(at("Headline").is_some() && at("Warm").is_some(), "{all:?}");
        assert!(at("Headline") < at("Warm"), "groups come after the ungrouped items: {all:?}");
        // "BLUE" finds the blue swatch only: the style and the group without a match are hidden.
        let found = texts(&mut app, "BLUE");
        assert!(!found.iter().any(|t| t == "Headline" || t == "Warm"), "{found:?}");
        let found = texts(&mut app, "yellow");
        assert!(found.iter().any(|t| t == "Warm") && !found.iter().any(|t| t == "Headline"), "{found:?}");
        let none = texts(&mut app, "zzz");
        assert!(none.iter().any(|t| t == "No results"), "{none:?}");
    }

    /// New Group… and Rename Group… ask for a name and act on the library and group they came
    /// from; the dialog shows only the name.
    #[test]
    fn group_dialogs_run_with_their_library_and_group() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let lib = app.run("library.create", json!({"name": "Brand"})).unwrap()["id"].as_str().unwrap().to_string();
        app.run("library.add", json!({"kind": "fillColor", "color": "#0044aa", "name": "Blue"})).unwrap();
        ask_group_name(&mut app, &lib, None, json!([{"kind": "fillColor", "item": "Blue"}]));
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Cool"));
        crate::dialogs::confirm(&mut app).unwrap();
        ask_group_name(&mut app, &lib, Some("Cool"), Value::Null);
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Cold"));
        crate::dialogs::confirm(&mut app).unwrap();
        let got = app.run("library.get", json!({})).unwrap();
        assert_eq!(got["groups"], json!([{"name": "Cold", "items": [{"kind": "color", "item": "Blue"}]}]));
    }

    /// Export Library… writes the library's file and Import Library… adds it back as a new library.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn libraries_are_exported_and_imported_from_the_panel_menu() {
        let dir = vectorcraft_testkit::temp_dir("libraries-panel");
        let path = dir.join("Brand.vclibrary").to_string_lossy().to_string();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("library.create", json!({"name": "Brand"})).unwrap();
        app.run("library.add", json!({"kind": "fillColor", "color": "#0044aa", "name": "Blue"})).unwrap();
        let picked = path.clone();
        app.services.pick_save = Some(Box::new(move |pick: &crate::FilePick| {
            assert_eq!(pick.name, "Brand.vclibrary", "the library's name suggested");
            Some(picked.clone())
        }));
        app.services.write = Some(Box::new(|p: &str, bytes: &[u8]| std::fs::write(p, bytes).map_err(|e| e.to_string())));
        export_picked(&mut app, "Brand", "Brand");
        assert!(std::path::Path::new(&path).exists());
        let picked = path.clone();
        app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(picked.clone())));
        import_picked(&mut app);
        let list = app.run("library.list", json!({})).unwrap();
        let names: Vec<&str> = list["libraries"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Brand", "Brand (2)"]);
        assert_eq!(app.ui.status, "Imported library Brand (2)");
    }
}
