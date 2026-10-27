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
//!
//! The list fills the panel down to its bottom bar and scrolls (also while a drag nears its top or
//! bottom edge). An item dragged within it lands before or after another of its kind, in that one's
//! group (`library.moveItem` with `before`/`after`), or, dropped elsewhere on a group or on the
//! ungrouped items, goes into that group or out of its own; a group dragged by its header goes
//! before or after another group (`library.moveGroup`).
//!
//! The buttons beside the library picker show the items as icons (colour chips and graphic tiles,
//! the default) or in a list, one row each with its name (a colour's hex value too).

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
/// The height of a colour's or style's row in the list view.
const ROW: f32 = 22.0;
/// The size of a graphic's thumbnail in the list view (its row is a little taller).
const LIST_TILE: f32 = 28.0;
/// The panel state that holds whether the items show in a list (else as icons).
const LIST_VIEW: &str = "library-list-view";
/// The height under the list the bottom bar takes (its divider and buttons) until it has been
/// measured.
const BAR: f32 = 42.0;
/// The least height the list takes.
const MIN_LIST: f32 = 140.0;
/// How near the list's top or bottom edge a drag scrolls it.
const EDGE: f32 = 28.0;

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

/// What a drag in the panel holds of the library shown: one of its items, or one of its groups
/// (by the group's header).
#[derive(Clone, Debug, PartialEq)]
enum Grab {
    Item(ItemKind, String),
    Group(String),
}

/// The item or group of library `library` that `drag` holds, if it holds one.
fn grabbed(drag: &PanelDrag, library: &str) -> Option<Grab> {
    match drag {
        PanelDrag::Paint { library: Some((l, item)), .. } if l == library => Some(Grab::Item(ItemKind::Color, item.clone())),
        PanelDrag::LibraryGraphic { library: l, item } if l == library => Some(Grab::Item(ItemKind::Graphic, item.clone())),
        PanelDrag::LibraryItem { library: l, kind, item } if l == library => Some(Grab::Item(*kind, item.clone())),
        PanelDrag::LibraryGroup { library: l, group } if l == library => Some(Grab::Group(group.clone())),
        _ => None,
    }
}

/// What the item list draws from.
struct Items<'a> {
    library: &'a str,
    lib: &'a Library,
    /// The group of each grouped item ([`Library::group_index`]).
    groups: HashMap<(ItemKind, &'a str), usize>,
    fill_active: bool,
    /// What a drag in progress holds of this library.
    grab: Option<Grab>,
    /// Whether the items show in a list (else colours and graphics as icons).
    list: bool,
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
    let list: bool = super::pstate(ui.ctx(), LIST_VIEW);
    // The library picker, and the list / icon view toggles at its right.
    ui.horizontal(|ui| {
        let names: Vec<&str> = libs.all().iter().map(|(_, l)| l.name.as_str()).collect();
        let width = (ui.available_width() - 2.0 * (24.0 + ui.spacing().item_spacing.x)).max(60.0);
        if let Some((id, _)) = widgets::dropdown_names(ui, "lib-picker", &lib.name, &names, width).and_then(|i| libs.all().get(i)) {
            act = Some(Act::Run("library.setCurrent", json!({ "library": id })));
        }
        if widgets::icon_button(ui, "dc-list-view", tl!("Show List View"), list, 24.0).clicked() {
            super::set_pstate(ui.ctx(), LIST_VIEW, true);
        }
        if widgets::icon_button(ui, "dc-grid-view", tl!("Show Thumbnail View"), !list, 24.0).clicked() {
            super::set_pstate(ui.ctx(), LIST_VIEW, false);
        }
    });
    ui.add_space(4.0);
    let empty = lib.items().next().is_none() && lib.groups.is_empty();
    let query = if empty { String::new() } else { widgets::search_field(ui, egui::Id::unique("library-search"), tl!("Search")) };
    let query = query.trim().to_lowercase();
    let searching = !query.is_empty();
    let shown = |name: &str| !searching || name.to_lowercase().contains(&query);
    let grab = egui::DragAndDrop::payload::<PanelDrag>(ui.ctx()).and_then(|d| grabbed(&d, &lib_id));
    let items = Items { library: &lib_id, lib, groups: lib.group_index(), fill_active, grab, list };
    // The items shown in each section: the ungrouped ones first, then each group's.
    let mut counts = vec![0usize; lib.groups.len() + 1];
    for (kind, key, _) in lib.items().filter(|(_, _, name)| shown(name)) {
        if let Some(n) = counts.get_mut(items.group_of(kind, key).map_or(0, |i| i + 1)) {
            *n += 1;
        }
    }
    ui.add_space(4.0);
    // The list takes the panel's height down to the bottom bar (as tall as it was last frame), and
    // scrolls.
    let bar_id = egui::Id::unique("library-bottom-bar");
    let bar: f32 = ui.data(|d| d.get_temp(bar_id)).unwrap_or(BAR);
    let list_h = ui.available_height() - bar;
    let list_h = if list_h.is_finite() { list_h.max(MIN_LIST) } else { MIN_LIST };
    // Each section's rows: the ungrouped items (None), then each group (its index), header included.
    let mut sections: Vec<(Option<usize>, Rect)> = vec![];
    // Whether an item takes the drag in progress (dropped next to it), so its section doesn't.
    let mut over_item = false;
    let list_rect = widgets::list_box(ui, |ui| {
        let out = egui::ScrollArea::vertical().id_salt("library-scroll").max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if empty {
                super::empty_state(
                    ui,
                    "library",
                    tl!("This library is empty"),
                    tl!("Drag art or swatches here, or use + to add the selection's colors and text styles."),
                );
                return;
            }
            if searching && counts.iter().all(|n| *n == 0) {
                let body = crate::i18n::fmt(tl!("No items in this library match “{query}”."), &[("query", &query)]);
                super::empty_state(ui, "search", tl!("No results"), &body);
                return;
            }
            egui::Frame::NONE.inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                let span = |ui: &Ui, top: f32| Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
                let top = ui.cursor().top();
                if counts.first().is_some_and(|n| *n > 0) {
                    items.show(ui, &mut act, &mut over_item, |kind, key, name| items.group_of(kind, key).is_none() && shown(name));
                } else if let Some(Grab::Item(kind, item)) = &items.grab
                    && items.group_of(*kind, item).is_some()
                {
                    no_group_strip(ui);
                }
                sections.push((None, span(ui, top)));
                for (i, g) in lib.groups.iter().enumerate() {
                    let n = counts.get(i + 1).copied().unwrap_or_default();
                    if searching && n == 0 {
                        continue;
                    }
                    let top = ui.cursor().top();
                    // Open unless closed; open while searching, to show what matched.
                    let key = format!("library-group-closed:{lib_id}:{}", g.name);
                    let closed: bool = super::pstate(ui.ctx(), &key);
                    let open = searching || !closed;
                    // The header also drags the group to another place in the list.
                    let header = widgets::section_toggle_name(ui, &g.name, open).interact(Sense::drag());
                    widgets::drag_source(ui, &header, || PanelDrag::LibraryGroup { library: lib_id.clone(), group: g.name.clone() });
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
                    if open {
                        ui.add_space(2.0);
                        if n == 0 {
                            ui.label(
                                egui::RichText::new(tl!("Drag an item here, or right-click one, to put it in this group."))
                                    .size(11.5)
                                    .color(t.text_dim),
                            );
                            ui.add_space(6.0);
                        } else {
                            items.show(ui, &mut act, &mut over_item, |kind, key, name| items.group_of(kind, key) == Some(i) && shown(name));
                        }
                    }
                    sections.push((Some(i), span(ui, top)));
                }
            });
            if items.grab.is_some() {
                scroll_at_edges(ui);
            }
        });
        out.inner_rect
    });
    if let Some(grab) = &items.grab
        && !over_item
    {
        section_drop(ui, &items, grab, &sections, list_rect, &mut act);
    }
    // Art dragged off the canvas, or a swatch (not one of this library's colours), dropped on the
    // list is added to the library.
    let zone = ui.interact(list_rect, ui.scope_id().with("libraries-drop"), Sense::hover());
    if let Some(ids) = widgets::art_drop(ui, &zone) {
        act = Some(Act::Run("library.add", json!({ "kind": "graphic", "ids": ids })));
    }
    if let Some(drag) = zone.dnd_hover_payload::<PanelDrag>()
        && let PanelDrag::Paint { paint: Paint::Solid { color, .. }, library, .. } = &*drag
        && library.as_ref().is_none_or(|(l, _)| *l != lib_id)
    {
        ui.painter().rect_stroke(zone.rect, 0.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
        let color = *color;
        if zone.dnd_release_payload::<PanelDrag>().is_some() {
            act = Some(Act::Run("library.add", json!({ "kind": "fillColor", "color": color })));
        }
    }
    let bar_top = ui.cursor().top();
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
    let bar_h = ui.min_rect().bottom() - bar_top;
    if bar_h.is_finite() && bar_h > 0.0 {
        ui.data_mut(|d| d.insert_temp(bar_id, bar_h));
    }
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
    /// and name of each), each kind under its heading. Each drags to another place in the list;
    /// `over` is set while one of them takes the drag in progress.
    fn show(&self, ui: &mut Ui, act: &mut Option<Act>, over: &mut bool, keep: impl Fn(ItemKind, &str, &str) -> bool) {
        let t = Tokens::get(ui.ctx());
        let lib = self.lib;
        let colors: Vec<_> = lib.colors.iter().filter(|c| keep(ItemKind::Color, &c.name, &c.name)).collect();
        if !colors.is_empty() {
            heading(ui, tl!("Colors"));
            self.cells(ui, |ui| {
                for c in colors {
                    let paint = Paint::solid(c.color);
                    let (r, resp) = self.cell(ui, CHIP, ROW);
                    let chip = if self.list {
                        let hex = c.color.to_hex();
                        let detail = (!hex.eq_ignore_ascii_case(&c.name)).then_some(hex.as_str());
                        list_row(ui, r, &resp, &c.name, detail);
                        Rect::from_center_size(r.left_center() + vec2(10.0, 0.0), vec2(14.0, 14.0))
                    } else {
                        r
                    };
                    widgets::swatch_tile(ui, chip, &paint, false, false);
                    ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
                    widgets::drag_source(ui, &resp, || PanelDrag::Paint {
                        paint: paint.clone(),
                        params: json!({ "color": c.color }),
                        rows: None,
                        library: Some((self.library.to_string(), c.name.clone())),
                    });
                    self.drop_next_to(ui, &resp, ItemKind::Color, &c.name, self.list, act, over);
                    // The list shows the name; a chip says it when hovered.
                    let resp = if self.list { resp } else { resp.on_hover_text(&c.name) };
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
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click_and_drag());
                list_row(ui, r, &resp, &st.name, None);
                let icon = if kind == ItemKind::ParaStyle { "pilcrow" } else { "type" };
                icons::paint(ui, icon, Rect::from_center_size(r.left_center() + vec2(10.0, 0.0), vec2(14.0, 14.0)), t.icon);
                widgets::drag_source(ui, &resp, || PanelDrag::LibraryItem { library: self.library.to_string(), kind, item: st.name.clone() });
                self.drop_next_to(ui, &resp, kind, &st.name, true, act, over);
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
            self.cells(ui, |ui| {
                for g in graphics {
                    let (r, resp) = self.cell(ui, TILE, LIST_TILE + 4.0);
                    let thumb = if self.list {
                        list_row(ui, r, &resp, &g.name, None);
                        Rect::from_min_size(r.left_center() + vec2(3.0, -LIST_TILE / 2.0), vec2(LIST_TILE, LIST_TILE))
                    } else {
                        r
                    };
                    tile(ui, thumb, self.library, g);
                    let edge = if resp.hovered() && !self.list { t.accent } else { t.border };
                    ui.painter().rect_stroke(thumb, 0.0, Stroke::new(1.0, edge), StrokeKind::Inside);
                    // Dragged onto the canvas, a copy is placed where it is dropped.
                    widgets::drag_source(ui, &resp, || PanelDrag::LibraryGraphic { library: self.library.to_string(), item: g.id.clone() });
                    self.drop_next_to(ui, &resp, ItemKind::Graphic, &g.id, self.list, act, over);
                    let resp = if self.list { resp } else { resp.on_hover_text(&g.name) };
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

    /// The cells `add` lays out: rows stacked downwards in the list view, else chips and tiles
    /// running across and wrapping.
    fn cells(&self, ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
        if self.list {
            add(ui);
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                add(ui);
            });
        }
    }

    /// The next item's cell: a `side`-point square in the icon view, else a row `row` points tall
    /// across the list.
    fn cell(&self, ui: &mut Ui, side: f32, row: f32) -> (Rect, egui::Response) {
        let size = if self.list { vec2(ui.available_width(), row) } else { vec2(side, side) };
        ui.allocate_exact_size(size, Sense::click_and_drag())
    }

    /// An item of `kind` dragged over item `key`'s cell `resp` lands just before or after it, by
    /// the pointer's half (`row`: rows stack downwards; else chips and tiles run across), in its
    /// group: an accent line shows where, and `over` is set. Dropped on itself it stays.
    #[allow(clippy::too_many_arguments)]
    fn drop_next_to(&self, ui: &Ui, resp: &egui::Response, kind: ItemKind, key: &str, row: bool, act: &mut Option<Act>, over: &mut bool) {
        let Some(Grab::Item(grabbed_kind, moving)) = &self.grab else { return };
        if *grabbed_kind != kind || resp.dnd_hover_payload::<PanelDrag>().is_none() {
            return;
        }
        *over = true;
        if moving == key {
            resp.dnd_release_payload::<PanelDrag>();
            return;
        }
        let r = resp.rect;
        let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.center());
        let after = if row { at.y > r.center().y } else { at.x > r.center().x };
        let accent = Stroke::new(2.0, Tokens::get(ui.ctx()).accent);
        if row {
            let y = if after { r.bottom() } else { r.top() };
            ui.painter().line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], accent);
        } else {
            let x = if after { r.right() + 2.0 } else { r.left() - 2.0 };
            ui.painter().line_segment([egui::pos2(x, r.top()), egui::pos2(x, r.bottom())], accent);
        }
        if resp.dnd_release_payload::<PanelDrag>().is_some() {
            let mut p = json!({ "kind": kind_param(kind), "item": moving });
            p[if after { "after" } else { "before" }] = json!(key);
            if let Some(g) = self.group_of(kind, key).and_then(|i| self.lib.groups.get(i)) {
                p["group"] = json!(g.name);
            }
            *act = Some(Act::Run("library.moveItem", p));
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

/// Where an item dragged out of its group can be dropped when no ungrouped item is showing.
fn no_group_strip(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        tl!("Drop here to take it out of its group"),
        egui::FontId::proportional(11.5),
        t.text_dim,
    );
    ui.add_space(6.0);
}

/// While something is dragged near the top or bottom edge of the scrolled list (`ui`: its
/// contents), scroll that way, faster nearer the edge.
fn scroll_at_edges(ui: &mut Ui) {
    let Some(pos) = ui.ctx().pointer_hover_pos() else { return };
    let view = ui.clip_rect();
    if !view.x_range().contains(pos.x) {
        return;
    }
    let speed = |depth: f32| if (0.0..EDGE).contains(&depth) { 2.0 + 10.0 * (1.0 - depth / EDGE) } else { 0.0 };
    let delta = speed(pos.y - view.top()) - speed(view.bottom() - pos.y);
    if delta != 0.0 {
        ui.scroll_with_delta(vec2(0.0, delta));
        ui.ctx().request_repaint();
    }
}

/// The drag in progress held over a section of the list (`sections`, seen through `clip`), away
/// from the items that take one: an item goes into that section's group (the ungrouped section:
/// out of its group), the section outlined; a group goes before or after that group, by the
/// pointer's half, an accent line showing where.
fn section_drop(ui: &Ui, items: &Items, grab: &Grab, sections: &[(Option<usize>, Rect)], clip: Rect, act: &mut Option<Act>) {
    // By the pointer, not by hover zones: the headers and items over the sections take the hover.
    let Some(pos) = ui.ctx().pointer_latest_pos() else { return };
    let released = ui.input(|i| i.pointer.any_released());
    let take = || egui::DragAndDrop::take_payload::<PanelDrag>(ui.ctx()).is_some();
    let t = Tokens::get(ui.ctx());
    let Some((group, rect)) = sections.iter().find(|(_, rect)| rect.intersect(clip).contains(pos)) else { return };
    let r = rect.intersect(clip);
    let name = |i: &usize| items.lib.groups.get(*i).map(|g| g.name.clone());
    match grab {
        Grab::Item(kind, item) => {
            if items.group_of(*kind, item) == *group {
                return;
            }
            ui.painter().rect_stroke(r, 3.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
            if released && take() {
                let mut p = json!({ "kind": kind_param(*kind), "item": item });
                if let Some(g) = group.as_ref().and_then(name) {
                    p["group"] = json!(g);
                }
                *act = Some(Act::Run("library.moveItem", p));
            }
        }
        Grab::Group(moving) => {
            // Groups only go among the groups; dropped on itself, one stays.
            let (Some(to), Some(from)) = (*group, items.lib.groups.iter().position(|g| g.name == *moving)) else { return };
            if from == to {
                return;
            }
            let after = pos.y > rect.center().y;
            let y = if after { rect.bottom() } else { rect.top() };
            ui.painter().line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], Stroke::new(2.0, t.accent));
            if released && take() {
                let to = to + usize::from(after);
                let to = if from < to { to - 1 } else { to };
                *act = Some(Act::Run("library.moveGroup", json!({ "group": moving, "to": to })));
            }
        }
    }
}

/// An item's row `r` in the list (`resp`: its response): highlighted while hovered, `name` after
/// the item's icon and `detail` dimmed at the right, where the name leaves room for it.
fn list_row(ui: &Ui, r: Rect, resp: &egui::Response, name: &str, detail: Option<&str>) {
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter_at(r);
    if resp.hovered() {
        painter.rect_filled(r, 0.0, t.hover);
    }
    let left = if r.height() > ROW { LIST_TILE + 10.0 } else { 24.0 };
    let font = egui::FontId::proportional(12.5);
    let name = painter.text(r.left_center() + vec2(left, 0.0), egui::Align2::LEFT_CENTER, name, font.clone(), t.text);
    if let Some(d) = detail {
        let d = painter.layout_no_wrap(d.to_string(), font, t.text_dim);
        let at = r.right_center() - vec2(6.0 + d.size().x, d.size().y / 2.0);
        if at.x > name.right() + 12.0 {
            painter.galley(at, d, t.text_dim);
        }
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
    // As many pixels as the tile shows (the art fits inside a 3-point margin).
    let px = (r.width().max(r.height()) - 6.0) * ui.ctx().pixels_per_point();
    let px = if px.is_finite() { px.ceil().clamp(8.0, 256.0) as u32 } else { 256 };
    if let Some(tex) = thumbnail(ui.ctx(), library, g, px) {
        let size = tex.size_vec2();
        let k = ((r.width() - 6.0) / size.x).min((r.height() - 6.0) / size.y).min(1.0);
        let fit = Rect::from_center_size(r.center(), size * k);
        ui.painter().image(tex.id(), fit, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

/// A thumbnail texture's library, graphic id, thumbnail length (for a changed graphic) and size.
type ThumbKey = (String, String, usize, u32);

/// The texture of graphic `g`'s PNG thumbnail at most `px` pixels a side, made once for each size.
/// It is shrunk here, smoothly: the GPU draws it without mipmaps, so a texture much larger than its
/// tile would break thin strokes up.
fn thumbnail(ctx: &egui::Context, library: &str, g: &LibraryGraphic, px: u32) -> Option<egui::TextureHandle> {
    thread_local! {
        static CACHE: crate::graphics::TexCache<HashMap<ThumbKey, Option<egui::TextureHandle>>> = crate::graphics::TexCache::default();
    }
    let key = (library.to_string(), g.id.clone(), g.thumbnail.len(), px);
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return t;
    }
    let tex = vectorcraft_format::base64_decode(&g.thumbnail)
        .and_then(|png| image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok())
        .map(|img| {
            let img = if img.width().max(img.height()) > px { img.resize(px, px, image::imageops::FilterType::Triangle) } else { img };
            let rgba = img.to_rgba8();
            let image = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            ctx.load_texture(format!("library-{library}-{}-{px}", g.id), image, egui::TextureOptions::LINEAR)
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
    let pick = crate::FilePick { filters: vec![crate::FilePick::filter("Libraries", LIBRARY_EXTS)], ..Default::default() };
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
mod tests;
