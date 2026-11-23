//! Libraries panel (Window › Libraries, #745): the current library's colors, gradients, patterns,
//! character and paragraph styles and graphics, from the engine's `library.*` commands. Art
//! dragged off the canvas and swatches dragged in are added where they are dropped; Add Content
//! adds the selection's graphic, colors, pattern, gradient or text styles. A click on a color,
//! gradient or pattern paints the selection through the active Fill/Stroke proxy, a style click
//! applies it to the selected type, and a graphic is dragged onto the canvas (or double-clicked)
//! to place a copy. Libraries are kept on this machine.
//!
//! The search field filters the items by name; items move into user-named groups, shown as
//! sections after the ungrouped items that drag to reorder; and the panel menu exports the library
//! to a `.vclibrary` file and imports one as a new library (#926).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft::behavior::{Axis, EditEvent, IconId, keyed};
use craft::host::PickRequest;
use craft::keymap::CommandRef;
use craft::theme::variant::Tone;
use craft::ui::choice::Segmented;
use craft::ui::contract::{
    ActivateOn, Bind, BodyCx, CollectionEvent, DropAt, DropKind, DropPlace, Entry, ListDrag, Props as _, ReorderRule, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _, Sizing, Trigger,
};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::notices::{Notice, NoticeAction, NoticeLayout};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::primitives::{ContentPainterCx, IconSpec, Revision, TextureStore, TextureStoreOptions, fit_rect};
use craft::ui::runtime::access::PopupKind;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::memory::Remember as _;
use craft::ui::sections::{DropAction, FooterAction, FooterBar, Panel, Section, SectionOutput, SectionStack, StackSlot};
use craft::ui::style::DataColor;
use craft::ui::text::TextField;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::{Color32, ColorImage, CornerRadius, Pos2, Rect, Response, Shape, Ui, pos2};
use serde_json::{Value, json};
use vectorcraft_color::{GradientPaint, Paint};
use vectorcraft_doc::resample::{Resample, fitted};
use vectorcraft_engine::cmd::library::{ItemKind, LIBRARY_EXT, LIBRARY_EXTS, Libraries, Library, LibraryGroup};

use crate::VectorcraftApp;
use crate::file_asks::{Owner, open_params};
use crate::panel_drag::PanelDrag;

/// The panel menu's rows, by command id.
const CREATE: &str = "ui.libraries.create";
const RENAME: &str = "ui.libraries.rename";
const DELETE: &str = "ui.libraries.delete";
const NEW_GROUP: &str = "ui.libraries.newGroup";
const IMPORT: &str = "ui.libraries.import";
const EXPORT: &str = "ui.libraries.export";

/// The kinds of item in the order their collections show, each under its heading.
const KINDS: [ItemKind; 6] = [ItemKind::Color, ItemKind::Gradient, ItemKind::Pattern, ItemKind::CharStyle, ItemKind::ParaStyle, ItemKind::Graphic];

/// What Add Content's menu adds: its rows and their `library.add` kinds.
const ADDS: [(&str, &str); 9] = [
    ("Graphic", "graphic"),
    ("Fill Color", "fillColor"),
    ("Stroke Color", "strokeColor"),
    ("Fill Pattern", "fillPattern"),
    ("Stroke Pattern", "strokePattern"),
    ("Fill Gradient", "fillGradient"),
    ("Stroke Gradient", "strokeGradient"),
    ("Character Style", "charStyle"),
    ("Paragraph Style", "paraStyle"),
];

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
// PLAN: renamed from `run`: the panel menu's `run` (ap.AS3, conflicts A15) takes that name in this module.
fn run_and_report(app: &mut VectorcraftApp, cmd: &str, p: Value) {
    let r = app.run(cmd, p);
    report(app, r);
}

/// The name dialog for a new library, or for renaming the current one.
fn ask_name(app: &mut VectorcraftApp, rename: Option<&str>) {
    let (cmd, label, name) = match rename {
        Some(n) => ("library.rename", "Rename Library", n.to_string()),
        None => ("library.create", "Create New Library", "My Library".to_string()),
    };
    ask(app, cmd, label, name, json!({}));
}

/// The name dialog for a new group of `library` (`items`: `[{kind, item}]` moved into it), or for
/// renaming group `rename`.
fn ask_group_name(app: &mut VectorcraftApp, library: &str, rename: Option<&str>, items: Value) {
    let (cmd, label, name, fixed) = match rename {
        Some(g) => ("library.renameGroup", "Rename Library Group", g, json!({ "library": library, "group": g })),
        None => ("library.createGroup", "New Library Group", "Group", json!({ "library": library, "items": items })),
    };
    ask(app, cmd, label, name.to_string(), fixed);
}

/// The name dialog for renaming item `item` (its key) of `kind` in `library`, filled with its name.
fn ask_item_name(app: &mut VectorcraftApp, library: &str, kind: ItemKind, item: &str) {
    let name = app.session.libraries.get(library).and_then(|l| l.items().find(|(k, key, _)| *k == kind && *key == item)).map(|(.., n)| n.to_string());
    let fixed = json!({ "library": library, "kind": kind_param(kind), "item": item });
    ask(app, "library.renameItem", "Rename Library Item", name.unwrap_or_else(|| item.to_string()), fixed);
}

/// The dialog titled `title` that asks for a name, filled with `name`. OK runs `command` with the
/// name and the `fixed` params; a refusal keeps the dialog open with the reason under its row.
fn ask(app: &mut VectorcraftApp, command: &'static str, title: &'static str, name: String, fixed: Value) {
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let fields = DialogFields::from_pairs([("name", name.clone())]);
    let dialog = HostedParamDialog::command(&ctx, command, fields, move |_| {
        ParamDialog::command(command, tl!(title), vec![Param::new("name", tl!("Name"), ParamKind::text(name.clone()))])
    });
    let opened = crate::dialogs::host::open_held(app, dialog, move |app, answer| {
        let mut params = fixed.clone();
        params["name"] = json!(answer.fields.text("name").unwrap_or_default());
        app.run(command, params).map(|_| ())
    });
    if opened.is_none() {
        app.status(tl!("There is no window to show the dialog in"));
    }
}

/// What a part of the panel asks for, run once the panel has shown: the parts read the library
/// while they show.
// PLAN: the sheet lists `Act` as going; the framework parts show while the library is borrowed from the session, so what they ask still runs after the panel.
enum Act {
    Run(&'static str, Value),
    /// The New Group dialog; the items (`[{kind, item}]`) go into the new group.
    NewGroup(Value),
    RenameGroup(String),
    /// The Rename dialog for an item (its kind and key).
    RenameItem(ItemKind, String),
}

/// What a drag in the panel holds for the library shown: one of its items, one of its groups (by
/// the group's header), or something to add to it.
#[derive(Clone, Debug, PartialEq)]
enum Grab {
    Item(ItemKind, String),
    Group(String),
    /// Art dragged off the canvas, or a swatch from elsewhere (a colour, also from another library,
    /// or a gradient or pattern swatch): added where it is dropped, with these `library.add` params.
    New(ItemKind, Value),
}

/// The item or group of library `library` that `drag` holds, or what it would add, if anything.
fn grabbed(drag: &PanelDrag, library: &str) -> Option<Grab> {
    match drag {
        PanelDrag::Paint { library: Some((l, item)), .. } if l == library => Some(Grab::Item(ItemKind::Color, item.clone())),
        PanelDrag::LibraryGraphic { library: l, item } if l == library => Some(Grab::Item(ItemKind::Graphic, item.clone())),
        PanelDrag::LibraryItem { library: l, kind, item } if l == library => Some(Grab::Item(*kind, item.clone())),
        PanelDrag::LibraryGroup { library: l, group } if l == library => Some(Grab::Group(group.clone())),
        PanelDrag::Art { ids, .. } => {
            let ids: Vec<u64> = ids.iter().map(|id| id.0).collect();
            Some(Grab::New(ItemKind::Graphic, json!({ "kind": "graphic", "ids": ids })))
        }
        PanelDrag::Paint { .. } => swatch_add(drag, library).map(|(kind, add)| Grab::New(kind, add)),
        _ => None,
    }
}

/// The kind and `library.add` params of what `drag` holds, to add to library `library`: a color
/// (not one of that library's own; a spot colour with its swatch's name), or a gradient or pattern
/// swatch, by its name.
fn swatch_add(drag: &PanelDrag, library: &str) -> Option<(ItemKind, Value)> {
    let PanelDrag::Paint { paint, params, library: from, .. } = drag else { return None };
    let swatch = || params.get("swatch").and_then(Value::as_str);
    match paint {
        Paint::Solid { color, .. } if from.as_ref().is_none_or(|(l, _)| l != library) => {
            // A spot swatch's name (and tint) come along, so the library keeps the ink.
            let mut add = json!({ "kind": "fillColor", "color": color });
            if let Some(s) = swatch() {
                add["swatch"] = json!(s);
                if let Some(t) = params.get("tint") {
                    add["tint"] = t.clone();
                }
            }
            Some((ItemKind::Color, add))
        }
        Paint::Gradient(_) => swatch().map(|s| (ItemKind::Gradient, json!({ "kind": "fillGradient", "swatch": s }))),
        Paint::Pattern { .. } => swatch().map(|s| (ItemKind::Pattern, json!({ "kind": "fillPattern", "swatch": s }))),
        _ => None,
    }
}

/// Which group each grouped item of the library shown is in, owned so that the drop targets,
/// which keep their test, can read it: what a drop into a group, or out of every group, does.
struct Membership {
    library: String,
    /// The group of each grouped item, by its index in the library's groups.
    index: HashMap<(ItemKind, String), usize>,
    /// The groups' names, in the library's order.
    names: Vec<String>,
}

impl Membership {
    fn of(library: &str, lib: &Library) -> Self {
        let index = lib.group_index().into_iter().map(|((kind, item), group)| ((kind, item.to_string()), group)).collect();
        Membership { library: library.to_string(), index, names: lib.groups.iter().map(|g| g.name.clone()).collect() }
    }

    fn group_of(&self, kind: ItemKind, key: &str) -> Option<usize> {
        self.index.get(&(kind, key.to_string())).copied()
    }

    /// `params` with the name of `group` when there is one; without it an item goes out of every group.
    fn in_group(&self, params: &mut Value, group: Option<usize>) {
        if let Some(name) = group.and_then(|g| self.names.get(g)) {
            params["group"] = json!(name);
        }
    }

    /// What a drop of `drag` on group `group` (`None`: away from every group) runs: an item of
    /// another group moves in, art or a swatch is added there. `None` when nothing would change,
    /// as for an item dropped on its own group, so that the target does not take it.
    fn landing(&self, drag: &PanelDrag, group: Option<usize>) -> Option<Act> {
        let (cmd, mut params) = match grabbed(drag, &self.library)? {
            Grab::Item(kind, item) if self.group_of(kind, &item) != group => ("library.moveItem", json!({ "kind": kind_param(kind), "item": item })),
            Grab::New(_, add) => ("library.add", add),
            Grab::Item(..) | Grab::Group(_) => return None,
        };
        self.in_group(&mut params, group);
        Some(Act::Run(cmd, params))
    }
}

/// An item as the panel shows it: its key and the name it shows, in the library's order.
struct Shown<'a> {
    kind: ItemKind,
    key: &'a str,
    name: &'a str,
    group: Option<usize>,
}

/// Which item's context menu the last context request in a section asked for, kept in the
/// section's memory while the menu shows.
#[derive(Clone, Debug, Default)]
struct ItemMenu(Option<(ItemKind, String)>);

/// How an item paints in its chip, tile or row thumbnail.
enum Look<'a> {
    Color(vectorcraft_color::Color),
    Gradient(&'a GradientPaint),
    /// A PNG thumbnail (base64), stored as a texture under the library and the item's kind and key.
    Png {
        key: (String, String),
        png: &'a str,
    },
}

impl Look<'_> {
    fn paint(&self, cx: &ContentPainterCx<'_>, rect: Rect) {
        match self {
            Look::Color(color) => {
                let [r, g, b, _] = color.to_rgba8(1.0);
                cx.data_shapes([Shape::rect_filled(rect, 0.0, Color32::from_rgb(r, g, b))]);
            }
            Look::Gradient(paint) => paint_gradient(cx, rect, paint),
            Look::Png { key, png } => paint_graphic(cx, rect, key, png),
        }
    }
}

/// What the panel's parts read: the library shown, the items the search leaves, and what the
/// selection and document allow.
struct View<'a> {
    library: &'a str,
    lib: &'a Library,
    membership: Arc<Membership>,
    /// The items whose name the search matches, in the library's order.
    items: Vec<Shown<'a>>,
    /// How many of `items` are ungrouped (first), then in each group.
    counts: Vec<usize>,
    /// The search, trimmed and in lower case; empty while not searching.
    query: String,
    searching: bool,
    /// The library has no item and no group.
    empty: bool,
    /// The items show in lists (else colours, gradients, patterns and graphics as icons).
    list: bool,
    fill_active: bool,
    /// Whether a document is open, and whether type is selected there for a style to apply to.
    has_doc: bool,
    can_apply: bool,
}

impl<'a> View<'a> {
    fn new(library: &'a str, lib: &'a Library, query: &str, list: bool, fill_active: bool, has_doc: bool, can_apply: bool) -> Self {
        let membership = Membership::of(library, lib);
        let query = query.trim().to_lowercase();
        let searching = !query.is_empty();
        let items: Vec<Shown<'a>> = lib
            .items()
            .filter(|(_, _, name)| !searching || name.to_lowercase().contains(&query))
            .map(|(kind, key, name)| Shown { kind, key, name, group: membership.group_of(kind, key) })
            .collect();
        let mut counts = vec![0usize; lib.groups.len() + 1];
        for item in &items {
            if let Some(n) = counts.get_mut(item.group.map_or(0, |g| g + 1)) {
                *n += 1;
            }
        }
        let empty = lib.items().next().is_none() && lib.groups.is_empty();
        View { library, lib, membership: Arc::new(membership), items, counts, query, searching, empty, list, fill_active, has_doc, can_apply }
    }

    /// The list: the ungrouped items, then the groups as sections. A drop anywhere on it away from
    /// the items and groups takes an item out of its group, or adds art or a swatch ungrouped.
    fn show(&self, ui: &mut Ui, closed: &mut HashSet<(String, String)>, acts: &RefCell<Vec<Act>>) {
        let membership = Arc::clone(&self.membership);
        let list = DropAction::new(move |drag: &PanelDrag| membership.landing(drag, None).is_some())
            .sizing(Sizing::Fill)
            .show(ui, |ui, _| Stack::new().fill().show(ui, |ui, cx| self.contents(ui, cx, closed, acts)));
        if let Some(drag) = &list.output.dropped
            && let Some(act) = self.membership.landing(drag, None)
        {
            acts.borrow_mut().push(act);
        }
    }

    fn contents(&self, ui: &mut Ui, cx: &mut BodyCx<'_>, closed: &mut HashSet<(String, String)>, acts: &RefCell<Vec<Act>>) {
        if self.empty {
            Notice::new(Tone::Neutral, tl!("Drag art or swatches here, or use + to add the selection's colors and text styles."))
                .title(tl!("This library is empty"))
                .layout(NoticeLayout::Centred)
                .show(ui);
            return;
        }
        if self.searching && self.counts.iter().all(|n| *n == 0) {
            let message = crate::i18n::fmt(tl!("No items in this library match “{query}”."), &[("query", &self.query)]);
            Notice::new(Tone::Neutral, message).title(tl!("No results")).layout(NoticeLayout::Centred).show(ui);
            return;
        }
        if !self.kinds(ui, cx, None, acts) && self.dragging_grouped(ui) {
            // With no ungrouped item showing, the strip is where an item leaves its group.
            let membership = Arc::clone(&self.membership);
            let strip = DropAction::new(move |drag: &PanelDrag| membership.landing(drag, None).is_some())
                .sizing(Sizing::FillWidth)
                .show(ui, |ui, _| Text::new(tl!("Drop here to take it out of its group")).shade(Shade::Muted).show(ui));
            if let Some(drag) = &strip.output.dropped
                && let Some(act) = self.membership.landing(drag, None)
            {
                acts.borrow_mut().push(act);
            }
        }
        let order: Vec<String> = self
            .lib
            .groups
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.searching || self.counts.get(i + 1).is_some_and(|n| *n > 0))
            .map(|(_, g)| g.name.clone())
            .collect();
        if order.is_empty() {
            return;
        }
        let stack = SectionStack::new(&order);
        let rows = stack.rows();
        let shown = stack.show(ui, |ui, _| rows.each(ui, |ui, name, slot| self.group(ui, name, slot, closed, acts)));
        for event in shown.output.moves {
            if let EditEvent::Set { after, .. } = event
                && let Some(to) = group_to(&self.lib.groups, &order, &after.key, after.to)
            {
                acts.borrow_mut().push(Act::Run("library.moveGroup", json!({ "group": after.key, "to": to })));
            }
        }
    }

    /// Whether the drag in progress holds an item of one of this library's groups.
    fn dragging_grouped(&self, ui: &Ui) -> bool {
        craft::ui::runtime::drag::active::<PanelDrag>(ui.ctx()).is_some_and(
            |drag| matches!(grabbed(&drag, self.library), Some(Grab::Item(kind, item)) if self.membership.group_of(kind, &item).is_some()),
        )
    }

    /// Group `name` as a section of the stack: its items, its count at the header's end and the
    /// header's Rename Group… and Delete Group. The whole section, header and body, takes items
    /// into the group, and art and swatches dropped on it are added to it.
    fn group(&self, ui: &mut Ui, name: &str, slot: StackSlot, closed: &mut HashSet<(String, String)>, acts: &RefCell<Vec<Act>>) -> SectionOutput {
        let index = self.lib.groups.iter().position(|g| g.name == name);
        let count = index.and_then(|i| self.counts.get(i + 1)).copied().unwrap_or_default();
        let key = (self.library.to_string(), name.to_string());
        // A search opens every group it shows, to show what matched.
        let mut open = !closed.contains(&key);
        let mut section = |ui: &mut Ui| {
            let bind = if self.searching { Bind::given(true) } else { Bind::bound(&mut open) };
            Section::new(name.to_string())
                .open(bind)
                .in_stack(slot)
                .controls(move |ui, _| {
                    Text::new(count.to_string()).shade(Shade::Muted).show(ui);
                })
                .menu(|ui, _| {
                    if MenuItem::new(tl!("Rename Group…")).show(ui).ran {
                        acts.borrow_mut().push(Act::RenameGroup(name.to_string()));
                    }
                    if MenuItem::new(tl!("Delete Group")).show(ui).ran {
                        acts.borrow_mut().push(Act::Run("library.deleteGroup", json!({ "group": name })));
                    }
                })
                .show(ui, |ui, cx| match index.filter(|_| count > 0) {
                    Some(group) => {
                        self.kinds(ui, cx, Some(group), acts);
                    }
                    None => {
                        Text::new(tl!("Drag an item here, or right-click one, to put it in this group.")).shade(Shade::Muted).show(ui);
                    }
                })
                .output
        };
        let membership = Arc::clone(&self.membership);
        let wrapped = DropAction::new(move |drag: &PanelDrag| index.is_some() && membership.landing(drag, index).is_some())
            .sizing(Sizing::FillWidth)
            .show(ui, |ui, _| section(ui));
        if let Some(drag) = &wrapped.output.dropped
            && index.is_some()
            && let Some(act) = self.membership.landing(drag, index)
        {
            acts.borrow_mut().push(act);
        }
        // The action shows its child whenever it shows; a section is still due when it ran no body.
        let output = match wrapped.inner {
            Some(output) => output,
            None => section(ui),
        };
        if !self.searching {
            if open {
                closed.remove(&key);
            } else {
                closed.insert(key);
            }
        }
        output
    }

    /// The items of `group` (`None`: the ungrouped ones), each kind under its heading in its own
    /// collection. → whether any item showed.
    fn kinds(&self, ui: &mut Ui, cx: &mut BodyCx<'_>, group: Option<usize>, acts: &RefCell<Vec<Act>>) -> bool {
        let mut any = false;
        for kind in KINDS {
            let shown: Vec<&Shown<'a>> = self.items.iter().filter(|item| item.kind == kind && item.group == group).collect();
            if shown.is_empty() {
                continue;
            }
            any = true;
            Text::new(heading(kind)).shade(Shade::Muted).show(ui);
            self.collection(ui, cx, kind, &shown, group, acts);
        }
        any
    }

    /// The collection of the items `shown` of `kind` in `group`: rows in the list view (text
    /// styles always), else chips and tiles. An item drags to another place in it, onto the
    /// canvas or into another group; one of its kind dragged from another collection, and art or
    /// a swatch, lands where it is dropped.
    fn collection(&self, ui: &mut Ui, cx: &mut BodyCx<'_>, kind: ItemKind, shown: &[&Shown<'a>], group: Option<usize>, acts: &RefCell<Vec<Act>>) {
        let keys: Vec<&str> = shown.iter().map(|item| item.key).collect();
        let source = keyed(shown, |item: &&Shown<'a>| item.key.to_string());
        let takes = DropKind::new::<PanelDrag>(
            DropPlace::Between,
            move |drag: &PanelDrag, _: &DropAt<String>| matches!(grabbed(drag, self.library), Some(Grab::Item(k, _) | Grab::New(k, _)) if k == kind),
        );
        // A graphic is placed by a double click; every other item is used by a click.
        let activate = if kind == ItemKind::Graphic { ActivateOn::DoubleClick } else { ActivateOn::Click };
        let label = heading(kind);
        let salt = ("library-items", kind_param(kind), group);
        let name_at = |at: usize, key: &str| shown.get(at).map_or_else(|| key.to_string(), |item| item.name.to_string());
        let opener = if self.list || matches!(kind, ItemKind::CharStyle | ItemKind::ParaStyle) {
            let out = List::<String>::new(label)
                .id_salt(salt)
                .select(SelectMode::None)
                .natural_height(true)
                .reorder(ReorderRule::new())
                .activate(activate)
                .drag_payload(self.payload(kind))
                .takes_drop(takes)
                .show_items(ui, &source, &mut |key, item| self.row(kind, key, &name_at(item.at, key)));
            self.handle(cx, kind, &keys, group, out.events, acts);
            out.response
        } else {
            let size = if kind == ItemKind::Graphic { ThumbSize::Large } else { ThumbSize::Small };
            let out = TileGrid::<String>::new(label)
                .id_salt(salt)
                .select(SelectMode::None)
                .natural_height(true)
                .reorder(ReorderRule::new())
                .activate(activate)
                .caption(CaptionPlace::None)
                .thumb_size(size)
                .drag_payload(self.payload(kind))
                .takes_drop(takes)
                .show_items(ui, &source, &mut |key, item| self.tile(kind, key, &name_at(item.at, key)));
            self.handle(cx, kind, &keys, group, out.response.events, acts);
            out.response.response
        };
        let target = cx.memory::<ItemMenu>().0.clone();
        if let Some(item) = target.filter(|(k, _)| *k == kind).and_then(|(_, key)| shown.iter().find(|item| item.key == key)) {
            self.item_menu(ui, &opener, item, acts);
        }
    }

    /// What the collection of `kind` in `group` (showing `keys`, in order) reported: a move within
    /// it, a drop from elsewhere, a click or double click that uses an item, and a request for an
    /// item's menu, which the menu shown after the collection reads.
    fn handle<E>(
        &self,
        cx: &mut BodyCx<'_>,
        kind: ItemKind,
        keys: &[&str],
        group: Option<usize>,
        events: Vec<CollectionEvent<String, E>>,
        acts: &RefCell<Vec<Act>>,
    ) {
        for event in events {
            let act = match event {
                CollectionEvent::Reorder(EditEvent::Set { after, .. } | EditEvent::Commit { after, .. }) => {
                    let rest: Vec<&str> = keys.iter().copied().filter(|key| *key != after.key).collect();
                    Some(self.moved(kind, &after.key, beside(&rest, after.to), group))
                }
                CollectionEvent::Dropped { at: DropAt::Gap { insert, .. }, payload, .. } => {
                    payload.get::<PanelDrag>().and_then(|drag| self.dropped(drag, kind, beside(keys, insert), group))
                }
                CollectionEvent::Activate(key) => Some(Act::Run("library.use", self.use_params(kind, &key))),
                CollectionEvent::ContextMenu(request) => {
                    cx.memory::<ItemMenu>().0 = Some((kind, request.key));
                    None
                }
                _ => None,
            };
            if let Some(act) = act {
                acts.borrow_mut().push(act);
            }
        }
    }

    /// Item `item` of `kind` moved beside `at` (the side and the key there) in `group`.
    fn moved(&self, kind: ItemKind, item: &str, at: Option<(&'static str, &str)>, group: Option<usize>) -> Act {
        let mut params = json!({ "kind": kind_param(kind), "item": item });
        if let Some((side, key)) = at {
            params[side] = json!(key);
        }
        self.membership.in_group(&mut params, group);
        Act::Run("library.moveItem", params)
    }

    /// What dropping `drag` beside `at` in the collection of `kind` in `group` runs: an item of
    /// `kind` moves there, art or a swatch of that kind is added there. Nothing for an item dropped
    /// beside itself.
    fn dropped(&self, drag: &PanelDrag, kind: ItemKind, at: Option<(&'static str, &str)>, group: Option<usize>) -> Option<Act> {
        match grabbed(drag, self.library)? {
            Grab::Item(k, item) if k == kind => (at.is_none_or(|(_, key)| key != item)).then(|| self.moved(kind, &item, at, group)),
            Grab::New(k, mut params) if k == kind => {
                if let Some((side, key)) = at {
                    params[side] = json!(key);
                }
                self.membership.in_group(&mut params, group);
                Some(Act::Run("library.add", params))
            }
            _ => None,
        }
    }

    /// The `library.use` params of item `key` of `kind`: a paint goes through the active
    /// Fill/Stroke proxy, a style onto the selected type, a graphic to the view's centre.
    fn use_params(&self, kind: ItemKind, key: &str) -> Value {
        match kind {
            ItemKind::Color | ItemKind::Gradient | ItemKind::Pattern => {
                let to = if self.fill_active { "fill" } else { "stroke" };
                json!({ "kind": kind_param(kind), "item": key, "to": to })
            }
            _ => json!({ "kind": kind_param(kind), "item": key }),
        }
    }

    /// What a drag of an item of `kind` carries for the targets outside its collection: a colour
    /// is a paint (the Swatches panel and art take it), a graphic is placed where it is dropped on
    /// the canvas, and every item moves within the panel.
    fn payload(&self, kind: ItemKind) -> impl Fn(&ListDrag<String>) -> Option<PanelDrag> + Send + Sync + use<> {
        let library = self.library.to_string();
        let colors: HashMap<String, vectorcraft_color::Color> =
            if kind == ItemKind::Color { self.lib.colors.iter().map(|c| (c.name.clone(), c.color)).collect() } else { HashMap::new() };
        move |drag: &ListDrag<String>| {
            let item = drag.grabbed.clone();
            Some(match kind {
                ItemKind::Color => {
                    let color = *colors.get(&item)?;
                    PanelDrag::Paint {
                        paint: Paint::solid(color),
                        params: json!({ "color": color }),
                        rows: None,
                        library: Some((library.clone(), item)),
                    }
                }
                ItemKind::Graphic => PanelDrag::LibraryGraphic { library: library.clone(), item },
                kind => PanelDrag::LibraryItem { library: library.clone(), kind, item },
            })
        }
    }

    /// How item `key` of `kind` paints; `None` for a text style, which shows its icon.
    fn look(&self, kind: ItemKind, key: &str) -> Option<Look<'a>> {
        let lib = self.lib;
        let png = |prefix: &str, png: &'a str| Look::Png { key: (self.library.to_string(), format!("{prefix}:{key}")), png };
        match kind {
            ItemKind::Color => lib.colors.iter().find(|c| c.name == key).map(|c| Look::Color(c.color)),
            ItemKind::Gradient => lib.gradients.iter().find(|g| g.name == key).map(|g| Look::Gradient(&g.gradient)),
            ItemKind::Pattern => lib.patterns.iter().find(|p| p.name == key).map(|p| png("pattern", &p.thumbnail)),
            ItemKind::Graphic => lib.graphics.iter().find(|g| g.id == key).map(|g| png("graphic", &g.thumbnail)),
            ItemKind::CharStyle | ItemKind::ParaStyle | ItemKind::Unknown => None,
        }
    }

    /// Item `key` of `kind`, named `name`, as a row: a text style with its icon, a colour with its
    /// hex value where that is not its name, the others with their thumbnail.
    fn row(&self, kind: ItemKind, key: &str, name: &str) -> ListRow<'a> {
        let row = ListRow::new(name.to_string());
        match (kind, self.look(kind, key)) {
            (ItemKind::CharStyle | ItemKind::ParaStyle, _) => {
                let icon = if kind == ItemKind::ParaStyle { "pilcrow" } else { "type" };
                row.icon(IconSpec::svg(IconId::new(icon))).tooltip(tl!("Click to apply to the selected type"))
            }
            (_, Some(Look::Color(color))) => {
                let hex = color.to_hex();
                // Named, so a screen reader reads the value with the row, as it is seen beside the name.
                let row = if hex.eq_ignore_ascii_case(name) { row } else { row.trailing_named(hex.clone(), hex) };
                row.thumb(ThumbSize::Small, move |cx, rect| Look::Color(color).paint(cx, rect))
            }
            (_, Some(look)) => {
                let size = if kind == ItemKind::Graphic { ThumbSize::Medium } else { ThumbSize::Small };
                row.thumb(size, move |cx, rect| look.paint(cx, rect))
            }
            (_, None) => row,
        }
    }

    /// Item `key` of `kind`, named `name`, as a chip or tile, which says its name when hovered.
    fn tile(&self, kind: ItemKind, key: &str, name: &str) -> Tile<'a> {
        let tile = Tile::new(name.to_string()).tooltip(name.to_string());
        match self.look(kind, key) {
            Some(look) => tile.thumb(ThumbAspect::SQUARE, move |cx, rect| look.paint(cx, rect)),
            None => tile,
        }
    }

    /// The context menu of `item`, hanging from its collection: Place for a graphic; Apply
    /// Character Style (with type selected) and Add to Character Styles (the document's, not
    /// applied) for a character style; Move to Group (No Group, the library's groups, New Group…);
    /// Rename… for a character style; and Delete.
    fn item_menu(&self, ui: &mut Ui, opener: &Response, item: &Shown<'_>, acts: &RefCell<Vec<Act>>) {
        let (kind, key) = (item.kind, item.key);
        let this = json!({ "kind": kind_param(kind), "item": key });
        let shown = Menu::context(item.name.to_string(), opener).show(ui, |ui, _| {
            let mut act = None;
            if kind == ItemKind::Graphic && MenuItem::new(tl!("Place")).show(ui).ran {
                act = Some(Act::Run("library.use", this.clone()));
            }
            if kind == ItemKind::CharStyle {
                if MenuItem::new(tl!("Apply Character Style")).enabled(self.can_apply).show(ui).ran {
                    act = Some(Act::Run("library.use", this.clone()));
                }
                if MenuItem::new(tl!("Add to Character Styles")).enabled(self.has_doc).show(ui).ran {
                    act = Some(Act::Run("library.addToCharacterStyles", json!({ "item": key })));
                }
                Divider::horizontal().show(ui);
            }
            Menu::submenu_row(ui, tl!("Move to Group"), |ui, _| {
                if MenuItem::new(tl!("No Group")).check(item.group.is_none()).show(ui).ran && item.group.is_some() {
                    act = Some(self.moved(kind, key, None, None));
                }
                for (i, g) in self.lib.groups.iter().enumerate() {
                    if MenuItem::new(g.name.clone()).check(item.group == Some(i)).show(ui).ran && item.group != Some(i) {
                        act = Some(self.moved(kind, key, None, Some(i)));
                    }
                }
                Divider::horizontal().show(ui);
                if MenuItem::new(tl!("New Group…")).show(ui).ran {
                    act = Some(Act::NewGroup(json!([this.clone()])));
                }
            });
            if kind == ItemKind::CharStyle && MenuItem::new(tl!("Rename…")).show(ui).ran {
                act = Some(Act::RenameItem(kind, key.to_string()));
            }
            if MenuItem::new(tl!("Delete")).show(ui).ran {
                act = Some(Act::Run("library.removeItem", this.clone()));
            }
            act
        });
        if let Some(act) = shown.inner.flatten() {
            acts.borrow_mut().push(act);
        }
    }
}

/// Where a drop before position `insert` of `keys` (the items shown, in order) lands: before the
/// item at `insert`, after the last item past the end, nowhere among no items. In `[A, B, C]`,
/// before C is `insert` 2 and the end is 3. A move within the collection counts in the order left
/// once the moved item is taken out: C moved to 0 of `[A, B]` lands before A.
fn beside<'k>(keys: &[&'k str], insert: usize) -> Option<(&'static str, &'k str)> {
    match keys.get(insert) {
        Some(key) => Some(("before", *key)),
        None => keys.last().map(|key| ("after", *key)),
    }
}

/// The library position of group `moving` moved to `to` of the groups shown (`shown`, the groups a
/// search leaves, `moving` taken out): before the shown group now at `to`, else after the last
/// shown group, counted among the library's groups with `moving` taken out. With every group
/// shown that is `to` itself; searching `[A, B, C, D]` shows `[A, C, D]`, and D moved to 0 lands
/// before A, at 0 of `[A, B, C]`.
fn group_to(groups: &[LibraryGroup], shown: &[String], moving: &str, to: usize) -> Option<usize> {
    let rest: Vec<&str> = groups.iter().map(|g| g.name.as_str()).filter(|g| *g != moving).collect();
    let shown: Vec<&str> = shown.iter().map(String::as_str).filter(|g| *g != moving).collect();
    match shown.get(to) {
        Some(next) => rest.iter().position(|g| g == next),
        None => shown.last().and_then(|last| rest.iter().position(|g| g == last)).map(|at| at + 1),
    }
}

/// The heading over the items of `kind`.
fn heading(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Color => tl!("Colors"),
        ItemKind::Gradient => tl!("Gradients"),
        ItemKind::Pattern => tl!("Patterns"),
        ItemKind::CharStyle => tl!("Character Styles"),
        ItemKind::ParaStyle => tl!("Paragraph Styles"),
        ItemKind::Graphic => tl!("Graphics"),
        ItemKind::Unknown => "",
    }
}

/// The panel: the library picker, the list and thumbnail views and the search field over the
/// current library's items, and the footer's Add Content and New Group. With no library it offers
/// to create one.
///
/// What the parts report comes from the input of the pass they show in, so a second pass in one
/// frame runs nothing again.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(lib_id) = app.session.libraries.current().map(str::to_string) else {
        if no_library(ui) {
            ask_name(app, None);
        }
        return;
    };
    let acts = RefCell::new(Vec::new());
    let has_doc = app.session.active().is_some();
    {
        // PLAN: assumed `UiState::{libraries_list_view: bool, library_search: String, library_groups_closed: HashSet<(String, String)>}` (ap.AS17, hub-w1).
        let (session, state) = (&app.session, &mut app.ui);
        let Some(lib) = session.libraries.get(&lib_id) else { return };
        // Whether a character style's Apply Character Style is enabled (looked up only when the library has one).
        let can_apply =
            has_doc && !lib.char_styles.is_empty() && vectorcraft_engine::cmd::textstyles::target_style_attrs(session, &json!({}), false).is_some();
        let view = View::new(&lib_id, lib, &state.library_search, state.libraries_list_view, session.fill_active, has_doc, can_apply);
        Panel::new(tl!("Libraries")).footer(|ui, _| footer(ui, has_doc, &acts)).show(ui, |ui, _| {
            let search = (!view.empty).then_some(&mut state.library_search);
            header(ui, &session.libraries, &lib_id, &mut state.libraries_list_view, search, &acts);
            view.show(ui, &mut state.library_groups_closed, &acts);
        });
    }
    for act in acts.into_inner() {
        apply(app, &lib_id, act);
    }
}

/// The panel with no library: what libraries are, and Create New Library. → whether it was pressed.
fn no_library(ui: &mut Ui) -> bool {
    let shown = Panel::new(tl!("Libraries")).show(ui, |ui, _| {
        Notice::new(
            Tone::Neutral,
            tl!(
                "Keep graphics, colors and text styles here to use them in any document. Libraries are stored on this machine — no account required."
            ),
        )
        .title(tl!("Local Libraries"))
        .layout(NoticeLayout::Centred)
        .action(NoticeAction::new(tl!("Create New Library")))
        .show(ui)
        .action
        .is_some()
    });
    shown.inner.unwrap_or_default()
}

/// The library picker and the list and thumbnail view buttons in a row, and under them the search
/// field (`None` for an empty library).
fn header(ui: &mut Ui, libraries: &Libraries, current: &str, list_view: &mut bool, search: Option<&mut String>, acts: &RefCell<Vec<Act>>) {
    let names: Vec<Entry<String>> = libraries.all().iter().map(|(id, l)| Entry::new(id.clone(), l.name.clone())).collect();
    let views = [
        Entry::new(true, tl!("Show List View")).icon(IconId::new("dc-list-view")),
        Entry::new(false, tl!("Show Thumbnail View")).icon(IconId::new("dc-grid-view")),
    ];
    let mut picked = current.to_string();
    Row::new().fill_width().show(ui, |ui, _| {
        Select::new(tl!("Library"), Bind::bound(&mut picked), &names).sizing(Sizing::FillWidth).show(ui);
        Segmented::new(tl!("View"), Bind::bound(&mut *list_view), &views).show(ui);
    });
    if picked != current {
        acts.borrow_mut().push(Act::Run("library.setCurrent", json!({ "library": picked })));
    }
    if let Some(search) = search {
        TextField::search(tl!("Search"), Bind::bound(search)).sizing(Sizing::FillWidth).show(ui);
    }
}

/// The footer: Add Content, whose menu adds the selection's graphic, colours, patterns, gradients
/// or text styles (with a document open), and New Group.
fn footer(ui: &mut Ui, has_doc: bool, acts: &RefCell<Vec<Act>>) {
    let out = FooterBar::actions([
        FooterAction::new(IconId::new("plus"), tl!("Add Content")).enabled(has_doc).popup(PopupKind::Menu),
        FooterAction::new(IconId::new("group"), tl!("New Group")),
    ])
    .show_footer(ui);
    if out.picked == Some(1) {
        acts.borrow_mut().push(Act::NewGroup(json!([])));
    }
    let Some(add) = out.responses.first() else { return };
    let added = Menu::new(tl!("Add Content")).opened_by(add, Trigger::Press).show(ui, |ui, _| {
        let mut added = None;
        for (label, kind) in ADDS {
            if MenuItem::new(tl!(label)).show(ui).ran {
                added = Some(kind);
            }
        }
        added
    });
    if let Some(kind) = added.inner.flatten() {
        acts.borrow_mut().push(Act::Run("library.add", json!({ "kind": kind })));
    }
}

/// Runs what a part of the panel asked for, on library `library`.
fn apply(app: &mut VectorcraftApp, library: &str, act: Act) {
    match act {
        Act::Run(cmd, mut p) => {
            if cmd != "library.setCurrent" {
                p["library"] = json!(library);
            }
            if cmd == "library.use"
                && p["kind"] == "graphic"
                && let Some(v) = app.view()
            {
                // Placed at the centre of the view.
                p["center"] = json!([v.center.x, v.center.y]);
            }
            run_and_report(app, cmd, p);
        }
        Act::NewGroup(items) => ask_group_name(app, library, None, items),
        Act::RenameGroup(g) => ask_group_name(app, library, Some(&g), Value::Null),
        Act::RenameItem(kind, item) => ask_item_name(app, library, kind, &item),
    }
}

/// `kind` as the `library.*` commands' `kind` param (a colour as a fill colour).
fn kind_param(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Graphic => "graphic",
        ItemKind::CharStyle => "charStyle",
        ItemKind::ParaStyle => "paraStyle",
        ItemKind::Gradient => "fillGradient",
        ItemKind::Pattern => "fillPattern",
        ItemKind::Color | ItemKind::Unknown => "fillColor",
    }
}

/// A gradient's stops as a strip from left to right, over the checkerboard where a stop is
/// see-through.
// shortcut: a radial, conic or freeform gradient shows its stops as a strip, not laid out as on
// art; upgrade when the content-paint kit draws a gradient by its kind.
fn paint_gradient(cx: &ContentPainterCx<'_>, rect: Rect, paint: &GradientPaint) {
    let stops: Vec<(f32, [u8; 4])> = paint.gradient.stops.iter().map(|s| (s.offset, s.color.to_rgba8(s.opacity))).collect();
    if stops.iter().any(|(_, [.., a])| *a < u8::MAX) {
        cx.checker(rect);
    }
    let ramp: Vec<(f32, DataColor)> =
        stops.into_iter().map(|(at, [r, g, b, a])| (at, DataColor::new(Color32::from_rgba_unmultiplied(r, g, b, a)))).collect();
    cx.ramp(rect, CornerRadius::ZERO, &ramp, Axis::X);
}

/// The PNG thumbnail `png` (base64) of a graphic or pattern on white in `rect`, stored as a
/// texture under `key` (the library and the item's kind and key) at the size it shows.
// PLAN: a tile's and a row's thumbnail take a painter, not a component, so the picture goes through `Picture`'s own store here rather than `Picture::new`.
fn paint_graphic(cx: &ContentPainterCx<'_>, rect: Rect, key: &(String, String), png: &str) {
    let ctx = cx.painter.ctx().clone();
    // As many pixels as the tile shows.
    let px = rect.width().max(rect.height()) * cx.painter.pixels_per_point();
    let px = if px.is_finite() { px.ceil().clamp(8.0, 256.0) as u32 } else { 256 };
    let store = TextureStore::<(String, String, u32)>::in_context(&ctx, "library-thumbnails", TextureStoreOptions::thumbnails());
    // A changed item has another thumbnail length.
    let texture = store.get_or_put(&ctx, (key.0.clone(), key.1.clone(), px), Revision::of(&png.len()), || thumbnail_image(png, px));
    cx.data_shapes([Shape::rect_filled(rect, 0.0, Color32::WHITE)]);
    let whole = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    cx.picture(texture.id, fit_rect(rect, texture.size_vec2()), whole, 0.0);
}

/// The PNG thumbnail `png` (base64) at most `px` pixels a side, shrunk smoothly: the GPU draws it
/// without mipmaps, so a texture much larger than its tile would break thin strokes up. One clear
/// pixel when it does not decode.
fn thumbnail_image(png: &str, px: u32) -> ColorImage {
    vectorcraft_format::base64_decode(png)
        .and_then(|png| craft_image_codecs::decode_as(craft_image_codecs::Format::Png, &png).ok())
        .and_then(|img| image::RgbaImage::from_raw(img.width(), img.height(), img.to_rgba8()).map(image::DynamicImage::ImageRgba8))
        .map(|img| {
            let rgba = shrunk_thumbnail(img, px);
            ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
        })
        .unwrap_or_else(|| ColorImage::from_rgba_unmultiplied([1, 1], &[0, 0, 0, 0]))
}

/// Thumbnail `img` shrunk smoothly to at most `px` pixels a side, its colors weighted by its alpha
/// so that clear pixels don't darken the edges of the art, as RGBA.
fn shrunk_thumbnail(img: image::DynamicImage, px: u32) -> image::RgbaImage {
    let img = if img.width().max(img.height()) > px { fitted(img, px, px, Resample::Filter(image::imageops::FilterType::Triangle)) } else { img };
    img.into_rgba8()
}

/// Import a library file (`library.import`'s params), saying so in the status bar.
pub(crate) fn import(app: &mut VectorcraftApp, p: Value) -> Result<Value, String> {
    let r = app.run("library.import", p)?;
    let name = r["name"].as_str().unwrap_or_default().to_string();
    app.status(format!("Imported library {name}"));
    Ok(r)
}

/// Import Library…: a picked `.vclibrary` file becomes a new library (on the web, from the bytes
/// the browser read).
fn import_picked(app: &mut VectorcraftApp) -> Value {
    let request = PickRequest::open("library").filter("Libraries", LIBRARY_EXTS);
    crate::file_asks::ask(app, request, Owner::Free, |app, picked| match open_params(&picked) {
        Some(params) => import(app, Value::Object(params)),
        None => Ok(Value::Null),
    })
}

/// Export Library…: library `id` written to a picked file named after it (the web downloads it).
fn export_picked(app: &mut VectorcraftApp, id: &str, name: &str) -> Result<Value, String> {
    let file = format!("{}.{LIBRARY_EXT}", vectorcraft_engine::cmd::library::file_stem_for(name));
    crate::io::save_command_output_named(app, "library.export", &file, json!({ "library": id }), |_, _| Ok(Value::Null))
}

/// The panel menu: Create New Library…, Rename Library…, Delete Library, New Group…, Import
/// Library… and Export Library….
// PLAN: the six `ui.libraries.*` ids need their `UI_COMMANDS` entries (menus, hub-w2); `menu_states` gives their titles meanwhile.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    let row = |id: &str| PanelRow::Command(CommandRef::new(id));
    vec![row(CREATE), row(RENAME), row(DELETE), PanelRow::Separator, row(NEW_GROUP), PanelRow::Separator, row(IMPORT), row(EXPORT)]
}

/// The panel menu's rows' titles, and which are enabled: all but Create New Library… and Import
/// Library… act on the current library.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let open = app.session.libraries.current().is_some();
    for (id, title, on) in [
        (CREATE, "Create New Library…", true),
        (RENAME, "Rename Library…", open),
        (DELETE, "Delete Library", open),
        (NEW_GROUP, "New Group…", open),
        (IMPORT, "Import Library…", true),
        (EXPORT, "Export Library…", open),
    ] {
        states.set(id, CommandState::new().enabled(on).title(tl!(title)));
    }
}

/// Runs the panel menu's row `id`; `None` for an id that is not one of them.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        CREATE => {
            ask_name(app, None);
            Ok(Value::Null)
        }
        IMPORT => Ok(import_picked(app)),
        RENAME | DELETE | NEW_GROUP | EXPORT => on_current(app, id),
        _ => return None,
    })
}

/// The panel menu's row `id` on the current library.
fn on_current(app: &mut VectorcraftApp, id: &str) -> Result<Value, String> {
    let (library, lib) = current(app).ok_or_else(|| tl!("No library is open").to_string())?;
    match id {
        RENAME => ask_name(app, Some(&lib.name)),
        DELETE => {
            let message = crate::i18n::fmt(tl!("Delete the library “{name}”?"), &[("name", &lib.name)]);
            crate::dialogs::confirm::ask(
                app,
                &message,
                tl!("Its graphics, colors and text styles are deleted from this machine."),
                "library.delete",
                json!({ "library": library }),
            );
        }
        NEW_GROUP => ask_group_name(app, &library, None, json!([])),
        EXPORT => return export_picked(app, &library, &lib.name),
        _ => return Err(format!("Unknown command {id}")),
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests;
