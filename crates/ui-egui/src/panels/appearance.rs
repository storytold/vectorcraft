//! Appearance panel: the full stack as in the reference app, as one list. The object row (its
//! thumbnail drags the appearance onto art), each stroke/fill row (eye, disclosure, link label,
//! swatch, weight, brush/dash/profile notes) with its Opacity and effect rows, fx rows (the name
//! opens the effect's dialog; the disclosure an inline editor), the object's Opacity row and the
//! footer. Groups and layers show a Contents row and type a Characters row where their members
//! (characters) paint among the object's own fills and strokes: double-clicking Contents targets
//! the members, double-clicking Characters edits the characters' paint and its name opens the
//! Character panel. The panels a row's link opens (Color, Stroke, Transparency, Character) are
//! popovers beside the panel, level with the row.
//!
//! Rows drag: a fill/stroke row reorders the stack (across the Contents row too), the Contents row
//! moves between the fills and strokes, an effect row reorders its list or moves into another item
//! (dropped into its row) or onto the object; Alt copies instead. Dropped on Duplicate Selected
//! Item or Delete Selected Item, the dragged rows are copied or removed. The list's selection rules
//! pick several fills/strokes or effects of one list for Duplicate and Delete. Delete on a selected
//! Opacity row sets it back to Default (100%, Normal).

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::Hash;
use std::sync::{Arc, OnceLock};

use craft::behavior::overlay::{Align, Side};
use craft::behavior::{ColumnKey, ColumnWidth, CopyKeys, DisplayOrder, EditEvent, IconId, ItemSource, Selection};
use craft::theme::Srgba;
use craft::ui::buttons::{Button, ButtonOutput};
use craft::ui::choice::{Swatch, SwatchOutput};
use craft::ui::contract::overlay::{Anchor, Place};
use craft::ui::contract::{
    Bind, CollectionEvent, CollectionResponse, ColumnSpec, FieldLabelRole, ItemCx, ListDrag, MemoProps, PopupKind, Props as _, ReorderMove,
    ReorderRule, Show as _, ShowCollection as _, ShowContainer as _, SideLabel, ToggleColumn, Trigger, clicked,
};
use craft::ui::display::{Picture, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Stack;
use craft::ui::list::{List, ListRow, RowDragSpot, RowEvent, RowInk, ThumbAspect, ThumbSize, ThumbTarget, ToggleCell, TreeLeaves};
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamRows};
use craft::ui::patterns::presets::{
    PresetBrowser, PresetEvent, PresetFns, PresetRow, PresetRowMenu, PresetSource, PresetState, PresetTiles, PresetView,
};
use craft::ui::popover::Popover;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::runtime::env::Label;
use craft::ui::sections::{FooterAction, FooterBar};
use craft::ui::text::NumberOutput;
use egui::{Rect, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, Node, NodeId};

use super::stroke::{self, WeightTarget};
// PLAN: hub-w2 declares `mod effect_params` private to `dialogs`; Appearance's inline editor (ef.C3: "built one way for the dialog, the gallery catalog and Appearance A3") needs `pub(crate) mod effect_params`.
use crate::dialogs::effect_params::{EffectCx, effect_params, stored_params};
use crate::dialogs::form::{fields_of, json_of};
use crate::panel_drag::PanelDrag;
use crate::{VectorcraftApp, paint_preview};

/// What is selected in the stack (for Duplicate / Delete). A fill/stroke row (also the owner of a
/// selected item effect) is the engine's active appearance item (`appearance.setActiveItem`), so
/// paint edits, the fx button and the Effect menu target it; effect rows are panel state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sel {
    #[default]
    None,
    Item(usize),
    /// An object-level effect.
    Effect(usize),
    /// Effect `k` of fill/stroke item `i`.
    ItemEffect(usize, usize),
    /// The Contents (type: Characters) row: paint edits go to the members (characters).
    Contents,
    /// The Opacity row of fill/stroke item `i` (the Transparency panel edits that item).
    ItemOpacity(usize),
    /// The object's own Opacity row.
    Opacity,
}

impl Sel {
    /// The fill/stroke row this selection makes the active item.
    fn item(self) -> Option<usize> {
        match self {
            Sel::Item(i) | Sel::ItemEffect(i, _) | Sel::ItemOpacity(i) => Some(i),
            Sel::None | Sel::Effect(_) | Sel::Contents | Sel::Opacity => None,
        }
    }
    /// Is a fill/stroke or effect row selected (what Duplicate acts on)? An Opacity row is not an
    /// item: it can't be copied.
    fn editable(self) -> bool {
        matches!(self, Sel::Item(_) | Sel::Effect(_) | Sel::ItemEffect(..))
    }
    /// Does Delete act on the selected row? A fill/stroke or effect row is removed; deleting an
    /// Opacity row sets it back to Default (100%, Normal) (#5742).
    fn deletable(self) -> bool {
        self.editable() || matches!(self, Sel::Opacity | Sel::ItemOpacity(_))
    }
}

/// A row of the panel's list: the rows of the selected object's stack (named as [`Sel`] names
/// them), the object row and the note of a mixed selection, and the rows of what new art gets
/// with nothing selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RowKey {
    /// The object row, or "No Selection": picking it targets the whole object again.
    Object,
    /// The "Mixed Appearances" note of objects whose stacks differ.
    Mixed,
    /// Fill/stroke `i`.
    Item(usize),
    /// Object-level effect `k`.
    Effect(usize),
    /// Effect `k` of fill/stroke `i`.
    ItemEffect(usize, usize),
    /// The Contents (type: Characters) row.
    Contents,
    /// The Opacity row of fill/stroke `i`.
    ItemOpacity(usize),
    /// The object's own Opacity row.
    Opacity,
    /// New art's fill/stroke `i`.
    NewArtItem(usize),
    /// Effect `k` of new art's fill/stroke `item` (`None`: of new art itself).
    NewArtEffect(Option<usize>, usize),
    /// New art's opacity.
    NewArtOpacity,
}

impl RowKey {
    /// The stack row `sel` stands for.
    fn of(sel: Sel) -> Option<RowKey> {
        Some(match sel {
            Sel::None => return None,
            Sel::Item(i) => RowKey::Item(i),
            Sel::Effect(k) => RowKey::Effect(k),
            Sel::ItemEffect(i, k) => RowKey::ItemEffect(i, k),
            Sel::Contents => RowKey::Contents,
            Sel::ItemOpacity(i) => RowKey::ItemOpacity(i),
            Sel::Opacity => RowKey::Opacity,
        })
    }

    /// The row as the active row: none for the object row, the note and new art's rows.
    fn sel(self) -> Sel {
        match self {
            RowKey::Item(i) => Sel::Item(i),
            RowKey::Effect(k) => Sel::Effect(k),
            RowKey::ItemEffect(i, k) => Sel::ItemEffect(i, k),
            RowKey::Contents => Sel::Contents,
            RowKey::ItemOpacity(i) => Sel::ItemOpacity(i),
            RowKey::Opacity => Sel::Opacity,
            RowKey::Object | RowKey::Mixed | RowKey::NewArtItem(_) | RowKey::NewArtEffect(..) | RowKey::NewArtOpacity => Sel::None,
        }
    }

    /// The list (`None`: the object's) and position of an effect row of the stack.
    fn effect(self) -> Option<(Option<usize>, usize)> {
        match self {
            RowKey::Effect(k) => Some((None, k)),
            RowKey::ItemEffect(i, k) => Some((Some(i), k)),
            _ => None,
        }
    }

    /// A fill/stroke row holds its Opacity and effect rows; an effect row its inline editor.
    fn has_children(self) -> bool {
        matches!(self, RowKey::Item(_) | RowKey::Effect(_) | RowKey::ItemEffect(..))
    }

    /// Fill/stroke, effect and Contents rows drag; the others stay where the stack puts them.
    fn drags(self) -> bool {
        matches!(self, RowKey::Item(_) | RowKey::Effect(_) | RowKey::ItemEffect(..) | RowKey::Contents)
    }

    /// A row of the object as a whole: an effect dropped beside it goes to the object's list.
    fn object_level(self) -> bool {
        matches!(self, RowKey::Object | RowKey::Contents | RowKey::Opacity)
    }
}

/// The Appearance panel's state for the session: the rows picked in the list and their object,
/// the open rows, the object row's thumbnail and the popovers rows open.
#[derive(Clone, Debug, Default)]
pub(crate) struct AppearancePanel {
    /// The object the selection's rows belong to.
    pub(crate) owner: Option<NodeId>,
    /// The picked rows: fills/strokes or the effects of one list, else one row; the lead is the
    /// active row.
    pub(crate) selection: Selection<RowKey>,
    /// The fill/stroke rows that show their Opacity and effect rows, and the effect rows that show
    /// their inline editor.
    pub(crate) open: HashSet<RowKey>,
    /// Hide Thumbnail: the object row shows no thumbnail.
    pub(crate) hide_thumb: bool,
    /// The row whose link opened the popover `UiState::transient_panel` names.
    pub(crate) popup_row: Option<RowKey>,
    /// The fill/stroke row whose swatch shows the document's swatches.
    pub(crate) swatches_for: Option<RowKey>,
    /// The row whose context menu the list last asked for.
    pub(crate) menu: Option<RowKey>,
}

/// The selected row of `node` (the first selected object): a selected effect row while that
/// effect still exists and agrees with the engine's active item, else the active item's row.
fn current_sel(app: &VectorcraftApp, node: Option<&Node>) -> Sel {
    let Some(n) = node else { return Sel::None };
    let panel = &app.ui.appearance_panel;
    let sel = panel.selection.lead.filter(|_| panel.owner == Some(n.id)).map_or(Sel::None, RowKey::sel);
    let exists = |item: Option<usize>, k: usize| n.appearance.effects_at(item).is_some_and(|fx| k < fx.len());
    match (app.session.appearance_item(), sel) {
        (Some(i), Sel::ItemEffect(j, k)) if i == j && exists(Some(i), k) => sel,
        (Some(i), Sel::ItemOpacity(j)) if i == j => sel,
        (Some(i), _) => Sel::Item(i),
        (None, Sel::Effect(k)) if exists(None, k) => sel,
        (None, Sel::Contents) if n.contents_label().is_some() => sel,
        (None, Sel::Opacity) => sel,
        (None, _) => Sel::None,
    }
}

/// Which rows a selection is made of: fills/strokes, or the effects of one list (`None`: the
/// object's own). Rows of different kinds or lists are never selected together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Rows {
    #[default]
    Items,
    Effects(Option<usize>),
}

impl Rows {
    /// The row `index` of this kind.
    fn key(self, index: usize) -> RowKey {
        match self {
            Rows::Items => RowKey::Item(index),
            Rows::Effects(None) => RowKey::Effect(index),
            Rows::Effects(Some(i)) => RowKey::ItemEffect(i, index),
        }
    }
    /// How many rows of this kind `n` has.
    fn len(self, n: &Node) -> usize {
        match self {
            Rows::Items => n.appearance.items.len(),
            Rows::Effects(item) => n.appearance.effects_at(item).map_or(0, |fx| fx.len()),
        }
    }
}

impl Sel {
    /// The kind of row selected and its index.
    fn rows(self) -> Option<(Rows, usize)> {
        match self {
            Sel::Item(i) => Some((Rows::Items, i)),
            Sel::Effect(k) => Some((Rows::Effects(None), k)),
            Sel::ItemEffect(i, k) => Some((Rows::Effects(Some(i)), k)),
            Sel::None | Sel::Contents | Sel::ItemOpacity(_) | Sel::Opacity => None,
        }
    }
}

/// The object whose rows the panel lists, which the panel's selection belongs to.
fn subject(app: &VectorcraftApp) -> Option<NodeId> {
    app.session.active().and_then(|d| d.selection.subjects().first().copied())
}

/// Make `sel` the active row; its fill/stroke becomes the active item that paint and effect edits
/// target. The picked rows stay as they are.
fn set_active(app: &mut VectorcraftApp, sel: Sel) {
    if app.session.appearance_item() != sel.item() {
        app.run("appearance.setActiveItem", json!({ "index": sel.item() })).ok();
    }
    app.ui.appearance_panel.owner = subject(app);
}

/// Select a row; its fill/stroke becomes the active item that paint and effect edits target.
/// It is the only picked row and the anchor.
pub(crate) fn select_row(app: &mut VectorcraftApp, sel: Sel) {
    set_active(app, sel);
    app.ui.appearance_panel.selection = RowKey::of(sel).map_or_else(Selection::default, Selection::single);
}

/// The list's selection of `n` with `sel` active: the panel's while it belongs to `n`, its lead is
/// the active row and its rows are of that row's list (rows that no longer exist dropped), else
/// the active row alone. When the active row has changed outside the panel since (Add New Fill
/// makes the new fill active), also the active row alone.
fn list_selection(app: &VectorcraftApp, n: &Node, sel: Sel) -> Selection<RowKey> {
    let Some(active) = RowKey::of(sel) else { return Selection::default() };
    let panel = &app.ui.appearance_panel;
    let held = &panel.selection;
    let Some((rows, _)) = sel.rows().filter(|_| panel.owner == Some(n.id) && held.lead == Some(active)) else {
        return Selection::single(active);
    };
    let fits = |key: &RowKey| key.sel().rows().is_some_and(|(r, k)| r == rows && k < rows.len(n));
    let mut items: Vec<RowKey> = held.items.iter().copied().filter(fits).collect();
    if !items.contains(&active) {
        items.push(active);
    }
    Selection::new(items, Some(active), held.anchor.filter(fits).or(Some(active)))
}

/// A selection change keeps the rows of the lead's list only (fills/strokes, or the effects of
/// one list; another row alone), so a pick in another list starts a new selection.
fn one_list(mut selection: Selection<RowKey>) -> Selection<RowKey> {
    let Some(lead) = selection.lead else { return selection };
    let list = lead.sel().rows().map(|(rows, _)| rows);
    selection.items.retain(|key| match list {
        Some(rows) => key.sel().rows().is_some_and(|(r, _)| r == rows),
        None => *key == lead,
    });
    if selection.anchor.is_none_or(|anchor| !selection.items.contains(&anchor)) {
        selection.anchor = Some(lead);
    }
    selection
}

/// The picked fill/stroke rows of `n` (paint order, ascending): the active row plus those picked
/// with it.
fn selected_items(app: &VectorcraftApp, n: &Node, sel: Sel) -> Vec<usize> {
    if !matches!(sel, Sel::Item(_)) {
        return vec![];
    }
    let mut set: Vec<usize> = list_selection(app, n, sel).items.iter().filter_map(|key| key.sel().rows().map(|(_, k)| k)).collect();
    set.sort_unstable();
    set
}

/// The picked effect rows of `n`: their list (`None`: the object's) and positions, ascending.
fn selected_effects(app: &VectorcraftApp, n: &Node, sel: Sel) -> Option<(Option<usize>, Vec<usize>)> {
    let (Rows::Effects(list), _) = sel.rows()? else { return None };
    let mut set: Vec<usize> = list_selection(app, n, sel).items.iter().filter_map(|key| key.effect().map(|(_, k)| k)).collect();
    set.sort_unstable();
    Some((list, set))
}

/// An effect as the fx menu lists it: (id, label without the ellipsis, Effect-menu path).
fn catalog_entry(e: vectorcraft_effects::EffectInfo) -> (String, String, Vec<String>) {
    (e.id.to_string(), e.label.trim_end_matches('…').to_string(), e.menu.iter().map(|s| s.to_string()).collect())
}

fn catalog() -> &'static [(String, String, Vec<String>)] {
    static C: OnceLock<Vec<(String, String, Vec<String>)>> = OnceLock::new();
    C.get_or_init(|| vectorcraft_effects::effect_catalog().into_iter().map(catalog_entry).collect())
}

/// Display label of an effect id ("stylize.dropShadow" → "Drop Shadow"; plug-in effects by their
/// plug-in's name).
pub fn effect_label(id: &str) -> String {
    catalog()
        .iter()
        .find(|c| c.0 == id)
        .map(|c| c.1.clone())
        .or_else(|| vectorcraft_effects::effect_info(id).map(|e| e.label.trim_end_matches('…').to_string()))
        .unwrap_or_else(|| id.rsplit('.').next().unwrap_or(id).to_string())
}

/// Is `id` one of our own effects? A plug-in's effect name comes from the plug-in and is shown as
/// it is.
fn built_in_effect(id: &str) -> bool {
    catalog().iter().any(|c| c.0 == id)
}

/// [`effect_label`] as shown: a built-in effect's name in the UI language as the Effect menu
/// shows it ([`crate::i18n::effect_name`]), a plug-in's as it is.
pub(crate) fn shown_effect_label(id: &str) -> String {
    match built_in_effect(id).then(|| crate::i18n::effect_name(crate::i18n::current(), id)).flatten() {
        Some(name) => name,
        None => effect_label(id),
    }
}

/// An effect row's name: [`shown_effect_label`], followed by "(Safe Mode)" for a plug-in effect
/// Safe Mode skips (the object keeps it and draws without it).
pub fn effect_row_label(id: &str) -> String {
    let label = shown_effect_label(id);
    if vectorcraft_plugins::effect::skipped(id) { format!("{label} ({})", tl!("Safe Mode")) } else { label }
}

/// The tooltip of an effect row Safe Mode skips: why it draws nothing, and its plug-in's id;
/// `None` for an effect that draws.
pub fn skipped_effect_tip(id: &str) -> Option<String> {
    let plugin = vectorcraft_plugins::effect::plugin_id(id).filter(|_| vectorcraft_plugins::effect::skipped(id))?;
    let why = crate::i18n::msg(&vectorcraft_plugins::Error::SafeMode.to_string());
    Some(format!("{why}\n{}", vectorcraft_plugins::safe_mode::display_name(plugin)))
}

/// "Opacity: Default" or "Opacity: 50% Multiply".
pub fn opacity_text(opacity: f32, blend: BlendMode) -> String {
    if (opacity - 1.0).abs() < 1e-4 && blend == BlendMode::Normal {
        tl!("Default").into()
    } else if blend == BlendMode::Normal {
        format!("{:.0}%", opacity * 100.0)
    } else {
        format!("{:.0}% {}", opacity * 100.0, tl!(blend.label()))
    }
}

/// A row popover's width, a floating panel's.
const POPUP_W: f32 = 260.0;
/// The least height of a row popover; it otherwise takes 60% of the window.
const POPUP_MIN_TALL: f32 = 200.0;
/// The columns of the swatch popover's chips.
const SWATCH_COLUMNS: u16 = 12;
/// The rows of chips the swatch popover shows before its grid scrolls inside the popover's own scroll.
const SWATCH_ROWS: u16 = 12;
/// The width and height of a stroke row's profile picture, as its aspect.
const PROFILE_ASPECT: (u16, u16) = (14, 5);

/// The eye column.
const EYE: ColumnKey = ColumnKey::new("eye");
/// The rest of a row after the eye: the link that stands for its label, and its trailing parts.
const NAME: ColumnKey = ColumnKey::new("name");

/// The footer's actions, in the order given: Add New Stroke, Add New Fill and Add New Effect sit
/// at the start edge, Delete, Duplicate and Clear from the end edge.
const ADD_STROKE: usize = 0;
const ADD_FILL: usize = 1;
const ADD_EFFECT: usize = 2;
const DELETE: usize = 3;
const DUPLICATE: usize = 4;
const CLEAR: usize = 5;

/// The panel menu's rows of the panel's own (not engine) commands.
const DUPLICATE_ITEM: &str = "ui.appearance.duplicateItem";
const REMOVE_ITEM: &str = "ui.appearance.removeItem";
const THUMBNAIL: &str = "ui.appearance.thumbnail";

/// The Appearance panel: the object row, the stack of the selected object (or of new art with
/// nothing selected) and the footer; Delete in the list removes the picked rows.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let node = first_subject(app);
    let summary = summary(app);
    let mix = summary.as_ref().and_then(|s| s.mixed(app));
    let new_art = app.session.new_art();
    let label = object_row_label(app, summary.as_deref(), mix.is_some(), node.is_some());
    let rows = StackRows::new(node.as_deref(), &new_art, mix, &app.ui.appearance_panel.open);
    let order = rows.keys();
    let sel = current_sel(app, node.as_deref());
    let mut selection = node.as_deref().map_or_else(Selection::default, |n| list_selection(app, n, sel));
    let document = app.session.active().map(|st| st.uid);
    let list_id = ui.scope_id().with(("appearance-rows", document));
    let has = node.is_some();
    // A dragged fill/stroke or effect row lights Duplicate and Delete up, also with nothing picked.
    let dragged = craft::ui::runtime::drag::active::<ListDrag<RowKey>>(ui.ctx())
        .is_some_and(|drag| drag.list == list_id && button_drop_command(&drag.keys, true).is_some());
    let can = has && (sel.editable() || dragged);
    let can_delete = can || (has && sel.deletable());
    let body = ui.max_rect();
    let copy = craft::ui::runtime::pointer::frame(ui.ctx()).mods.alt;
    let pass = RefCell::new(Pass::default());
    let footer = RefCell::new(None);
    let shared: &VectorcraftApp = app;
    let cx = RowCx {
        app: shared,
        node: node.as_deref(),
        new_art: &new_art,
        mix,
        label: &label,
        hide_thumb: shared.ui.appearance_panel.hide_thumb,
        open: &shared.ui.appearance_panel.open,
        pass: &pass,
    };
    let shown = Stack::new().fill().footer(|ui, _| *footer.borrow_mut() = Some(footer_bar(has, can, can_delete, list_id).show_footer(ui))).show(
        ui,
        |ui, _| {
            // A pass's own rows report its parts.
            *pass.borrow_mut() = Pass::default();
            show_list(ui, &cx, &rows, &order, &mut selection, list_id, copy)
        },
    );
    let pass = pass.into_inner();
    if let Some(list) = shown.inner {
        let opener = list.response.clone();
        apply_list(app, node.as_deref(), &order, selection, list);
        if let (Some(key), Some(n)) = (app.ui.appearance_panel.menu, node.as_deref()) {
            row_menu(app, ui, &opener, n, key);
        }
    }
    apply_parts(app, node.as_deref(), pass.acts);
    if let Some(out) = footer.into_inner() {
        let sel = current_sel(app, node.as_deref());
        match out.picked {
            Some(ADD_STROKE) => {
                app.run("appearance.addStroke", json!({})).ok();
            }
            Some(ADD_FILL) => {
                app.run("appearance.addFill", json!({})).ok();
            }
            Some(DELETE) => {
                delete_selected(app, node.as_deref(), sel).ok();
            }
            Some(DUPLICATE) => {
                duplicate_selected(app, node.as_deref(), sel).ok();
            }
            Some(CLEAR) => {
                app.run("appearance.clear", json!({})).ok();
            }
            _ => {}
        }
        if let (Some(dropped), Some(n)) = (&out.dropped, node.as_deref())
            && let Some(drag) = dropped.payload::<ListDrag<RowKey>>()
            && drag.list == list_id
        {
            drop_on_footer(app, n, sel, &drag.keys, dropped.index == DUPLICATE);
        }
        if let Some(opener) = out.responses.get(ADD_EFFECT) {
            Menu::new(tl!("Add New Effect")).opened_by(opener, Trigger::Click).show(ui, |ui, _| fx_menu(app, ui));
        }
    }
    show_swatch_popup(app, ui, &pass.swatches);
    show_row_popup(app, ui, &pass.links, body);
}

/// The object row's name: the selection's kind ("Path", "Layer"…), or "No Selection"; a linked
/// object names its graphic style ("Rectangle: Sunshine"), and so does new art with nothing
/// selected.
fn object_row_label(app: &VectorcraftApp, summary: Option<&Summary>, mixed: bool, has_node: bool) -> String {
    let label = tl!(summary.map_or("No Selection", |s| s.label));
    let style = match app.session.selection_graphic_style() {
        Some((g, true)) if !mixed => Some(g.name.as_str()),
        _ if !has_node => app.session.new_art_transparency().2,
        _ => None,
    };
    match style {
        Some(name) => format!("{} {name}", crate::i18n::colon(label)),
        None => label.to_string(),
    }
}

/// The list over `rows` (`order` their keys), bound to `selection`. A mixed selection's rows don't
/// drag: where one would go in each of the different stacks isn't one place.
fn show_list(
    ui: &mut Ui,
    cx: &RowCx<'_>,
    rows: &StackRows,
    order: &[RowKey],
    selection: &mut Selection<RowKey>,
    list_id: egui::Id,
    copy: bool,
) -> CollectionResponse<RowKey, RowEvent> {
    let columns = vec![
        ColumnSpec::toggle(EYE, Label::Empty, ToggleColumn::default()).icon(IconId::new("eye")),
        ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0)),
    ];
    let mut list = List::new(tl!("Appearance"))
        .id(list_id)
        .tree(true)
        // The top-level rows without a disclosure (Opacity, an object's effects) keep its column, so their labels
        // start where the Stroke and Fill labels do.
        .tree_leaves(TreeLeaves::Aligned)
        .columns(columns)
        .deletable(true)
        .selection(Bind::bound(selection))
        .can_drag(|key: &RowKey| key.drags());
    if let Some(n) = cx.node.filter(|_| cx.mix.is_none()) {
        list = list
            .reorder(ReorderRule::new().copy_with(CopyKeys::Alt).into_groups())
            .accepts(move |drag: &ListDrag<RowKey>, over: Option<&RowKey>, insert: usize| accepts(n, order, drag, over, insert, copy));
    }
    list.show_items(ui, rows, &mut |key, item| build_row(cx, *key, item))
}

/// Applies what the list reported: the picks (the one-list rule, then the active row), opened
/// rows, eyes, activations, Delete and a reorder.
fn apply_list(
    app: &mut VectorcraftApp,
    node: Option<&Node>,
    order: &[RowKey],
    selection: Selection<RowKey>,
    list: CollectionResponse<RowKey, RowEvent>,
) {
    let (mut picked, mut deleted) = (false, false);
    let (mut eyes, mut activated, mut moved) = (vec![], vec![], None);
    for event in list.events {
        match event {
            CollectionEvent::Selection { .. } => picked = true,
            CollectionEvent::Toggle(EditEvent::Set { after, .. }) => eyes.push((after.key, after.on)),
            CollectionEvent::Open(change) => {
                let open = &mut app.ui.appearance_panel.open;
                if change.open {
                    open.insert(change.key);
                } else {
                    open.remove(&change.key);
                }
            }
            CollectionEvent::Activate(key) => activated.push(key),
            CollectionEvent::ContextMenu(request) => app.ui.appearance_panel.menu = Some(request.key),
            CollectionEvent::Delete(_) => deleted = true,
            CollectionEvent::Reorder(EditEvent::Set { after, .. } | EditEvent::Commit { after, .. }) => moved = Some(after),
            _ => {}
        }
    }
    if picked {
        let chosen = one_list(selection);
        set_active(app, chosen.lead.map_or(Sel::None, RowKey::sel));
        app.ui.appearance_panel.selection = chosen;
    }
    for (key, on) in eyes {
        toggle_eye(app, key, on);
    }
    let Some(n) = node else { return };
    for key in activated {
        activate(app, n, key);
    }
    if deleted {
        let sel = current_sel(app, node);
        delete_selected(app, node, sel).ok();
    }
    if let Some(mv) = moved {
        reorder(app, n, order, &mv);
    }
}

/// What a row's context menu runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAct {
    Clear,
    Reduce,
    EditEffect,
    Duplicate,
    Delete,
    /// Hide or show the row's fill/stroke or effect.
    Eye(bool),
}

/// The context menu of row `key` of `n`, hanging from the list, which picked the row unless it was picked already:
/// Clear Appearance and Reduce to Basic Appearance on the object row; Duplicate, Delete and Hide or Show on a
/// fill/stroke row, and Edit Effect…, Duplicate, Delete and Hide or Show on an effect row, Duplicate and Delete for
/// every picked row of its kind. The other rows have none.
fn row_menu(app: &mut VectorcraftApp, ui: &mut Ui, opener: &egui::Response, n: &Node, key: RowKey) {
    let sel = current_sel(app, Some(n));
    let rows: Vec<(RowAct, &str, bool)> = match key {
        RowKey::Object => vec![
            (RowAct::Clear, tl!("Clear Appearance"), true),
            (RowAct::Reduce, tl!("Reduce to Basic Appearance"), any_subject_needs_reduction(app)),
        ],
        RowKey::Item(i) => {
            let Some(it) = n.appearance.items.get(i) else { return };
            let stroke = matches!(it, AppearanceItem::Stroke(_));
            let (duplicate, delete) = match (selected_items(app, n, sel).len() > 1, stroke) {
                (true, _) => (tl!("Duplicate Items"), tl!("Delete Items")),
                (false, true) => (tl!("Duplicate Stroke"), tl!("Delete Stroke")),
                (false, false) => (tl!("Duplicate Fill"), tl!("Delete Fill")),
            };
            let eye = match (it.visible(), stroke) {
                (true, true) => tl!("Hide Stroke"),
                (false, true) => tl!("Show Stroke"),
                (true, false) => tl!("Hide Fill"),
                (false, false) => tl!("Show Fill"),
            };
            vec![(RowAct::Duplicate, duplicate, true), (RowAct::Delete, delete, true), (RowAct::Eye(!it.visible()), eye, true)]
        }
        RowKey::Effect(_) | RowKey::ItemEffect(..) => {
            let Some((item, k)) = key.effect() else { return };
            let Some(e) = n.appearance.effects_at(item).and_then(|fx| fx.get(k)) else { return };
            let many = selected_effects(app, n, sel).is_some_and(|(list, ks)| list == item && ks.len() > 1);
            let (duplicate, delete) =
                if many { (tl!("Duplicate Effects"), tl!("Delete Effects")) } else { (tl!("Duplicate Effect"), tl!("Delete Effect")) };
            let eye = if e.visible { tl!("Hide Effect") } else { tl!("Show Effect") };
            vec![
                (RowAct::EditEffect, tl!("Edit Effect…"), true),
                (RowAct::Duplicate, duplicate, true),
                (RowAct::Delete, delete, true),
                (RowAct::Eye(!e.visible), eye, true),
            ]
        }
        _ => return,
    };
    let shown = Menu::context(tl!("Appearance"), opener).show(ui, |ui, _| {
        let mut ran = None;
        for (act, label, enabled) in &rows {
            if MenuItem::new(*label).enabled(*enabled).show(ui).ran {
                ran = Some(*act);
            }
        }
        ran
    });
    match shown.inner.flatten() {
        Some(RowAct::Clear) => {
            app.run("appearance.clear", json!({})).ok();
        }
        Some(RowAct::Reduce) => {
            app.run("appearance.reduceToBasic", json!({})).ok();
        }
        Some(RowAct::EditEffect) => edit_effect(app, n, key),
        Some(RowAct::Duplicate) => {
            duplicate_selected(app, Some(n), sel).ok();
        }
        Some(RowAct::Delete) => {
            delete_selected(app, Some(n), sel).ok();
        }
        Some(RowAct::Eye(on)) => toggle_eye(app, key, on),
        None => {}
    }
}

/// Whether a selected object, or the compound path holding it, has more than a basic appearance to reduce.
fn any_subject_needs_reduction(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    st.selection.subjects().iter().any(|id| {
        let compound = st.doc.parent_of(*id).and_then(|p| st.doc.node(p)).filter(|p| matches!(p.kind, vectorcraft_doc::NodeKind::Compound { .. }));
        compound.or_else(|| st.doc.node(*id)).is_some_and(|n| !n.appearance.is_basic())
    })
}

/// Applies what the rows' parts did: links, swatches, weights and inline editor edits.
fn apply_parts(app: &mut VectorcraftApp, node: Option<&Node>, acts: Vec<Act>) {
    for act in acts {
        match act {
            Act::Link(key) => follow_link(app, node, key),
            Act::Swatch(key, painted, shift) => swatch_pressed(app, key, painted, shift),
            Act::Weight(target, out) => stroke::apply_weight(app, &out, target),
            Act::Params(item, k, params) => {
                app.run("effect.setParams", aligned(app, json!({"item": item, "index": k, "params": params}))).ok();
            }
            Act::Dialog(item, k, effect) => {
                app.run("effect.dialog", aligned(app, json!({"effect": effect, "index": k, "item": item}))).ok();
            }
        }
    }
}

/// A part of a row that acted in a pass, applied once the list is shown.
enum Act {
    /// The row's name link was clicked.
    Link(RowKey),
    /// The row's swatch was clicked, with Shift (the mixer) or without (the swatches).
    Swatch(RowKey, Painted, bool),
    /// The stroke weight field's output.
    Weight(WeightTarget, Box<NumberOutput>),
    /// The inline editor of effect `k` of list `item` changed these parameters.
    Params(Option<usize>, usize, serde_json::Map<String, Value>),
    /// The inline editor of effect `k` of list `item` asks for the effect's dialog.
    Dialog(Option<usize>, usize, String),
}

/// What the rows' parts did in one pass.
#[derive(Default)]
struct Pass {
    acts: Vec<Act>,
    /// The name links shown, by row: a row's popover hangs from its link.
    links: Vec<(RowKey, egui::Response)>,
    /// The swatches shown, by row, with what they paint: the swatch popover hangs from its swatch.
    swatches: Vec<(RowKey, Painted, egui::Response)>,
}

/// What a row reads to build itself.
struct RowCx<'a> {
    app: &'a VectorcraftApp,
    node: Option<&'a Node>,
    new_art: &'a Appearance,
    mix: Option<&'a Mixed>,
    label: &'a str,
    hide_thumb: bool,
    open: &'a HashSet<RowKey>,
    pass: &'a RefCell<Pass>,
}

/// One row of the list.
fn build_row<'r>(cx: &'r RowCx<'_>, key: RowKey, item: &mut ItemCx<'_, 'r, RowKey>) -> ListRow<'r> {
    let row = match key {
        RowKey::Object => object_row(cx),
        RowKey::Mixed => ListRow::new(tl!("Mixed Appearances")).italic(true).ink(RowInk::Muted),
        RowKey::Item(i) | RowKey::NewArtItem(i) => item_row(cx, key, i),
        RowKey::Effect(_) | RowKey::ItemEffect(..) => effect_row(cx, key, item),
        RowKey::NewArtEffect(list, k) => {
            let e = cx.new_art.effects_at(list).and_then(|fx| fx.get(k));
            let label = e.map_or_else(String::new, |e| effect_row_label(&e.id));
            ListRow::new(label).toggle(ToggleCell::eye(EYE, e.is_some_and(|e| e.visible))).trailing_icon(fx_icon(), Label::Empty)
        }
        RowKey::ItemOpacity(_) | RowKey::Opacity => opacity_row(cx, key),
        RowKey::NewArtOpacity => {
            let (opacity, blend, _) = cx.app.session.new_art_transparency();
            let label = crate::i18n::fmt(tl!("Opacity: {value}"), &[("value", &opacity_text(opacity, blend))]);
            ListRow::new(label).toggle(ToggleCell::eye(EYE, true))
        }
        RowKey::Contents => contents_row(cx),
    };
    let row = row.cells(item.cells()).state(item.row.clone()).depth(item.depth);
    if key.has_children() { row.group(cx.open.contains(&key)) } else { row }
}

/// The fx mark at the end of an effect row.
fn fx_icon() -> IconSpec<'static> {
    IconSpec::svg(IconId::new("dc-fx"))
}

/// The output of a row's name link: a click follows it, and the link is where the row's popover
/// hangs.
// PLAN: decisions.md's `ItemCx::part_opener(part)` names a part of the row's own item; the name link is a hosted `Link` (`ListRow::cell_field`), a component with no `PartRef` in the row, so the popover hangs from the link's own `ButtonOutput::response`.
fn link_output(pass: &RefCell<Pass>, key: RowKey) -> impl FnOnce(ButtonOutput) + '_ {
    move |out| {
        let mut pass = pass.borrow_mut();
        if out.clicked() {
            pass.acts.push(Act::Link(key));
        }
        pass.links.push((key, out.response));
    }
}

/// The object row: the selection's name and, unless Hide Thumbnail is on or the objects' fills
/// differ, a thumbnail of the fill that drags this object's appearance onto art.
fn object_row<'r>(cx: &'r RowCx<'_>) -> ListRow<'r> {
    let row = ListRow::new(cx.label.to_string()).strong(true);
    if cx.hide_thumb || cx.mix.is_some_and(|m| m.fills_differ) {
        return row;
    }
    let fill = match cx.node {
        Some(n) => n.appearance.fill_paint(),
        None => cx.app.session.paint.fill.clone(),
    };
    let row = row.thumb(ThumbSize::Small, move |painter, rect| paint_preview::paint_rect(painter, rect, &fill));
    match cx.node.filter(|_| cx.mix.is_none()).zip(cx.app.session.active()) {
        Some((n, st)) => {
            row.part_drag(RowDragSpot::Thumb(ThumbTarget::First), PanelDrag::Appearance { document: st.uid, reverts: st.reverts, source: n.id })
        }
        None => row,
    }
}

/// A fill/stroke row of the object (`Item`) or of new art (`NewArtItem`): its eye, the link that
/// edits it, its swatch and, for a stroke, its weight and notes.
fn item_row<'r>(cx: &'r RowCx<'_>, key: RowKey, i: usize) -> ListRow<'r> {
    let new_art = matches!(key, RowKey::NewArtItem(_));
    let stack: Option<&Appearance> = if new_art { Some(cx.new_art) } else { cx.node.map(|n| &*n.appearance) };
    let Some(it) = stack.and_then(|ap| ap.items.get(i)) else { return ListRow::new(Label::Empty) };
    let fill = it.is_fill();
    let (name, link) = if fill { (tl!("Fill"), tl!("Fill:")) } else { (tl!("Stroke"), tl!("Stroke:")) };
    let mut row = ListRow::new(name).toggle(ToggleCell::eye(EYE, it.visible())).cell_field(
        NAME,
        crate::panels::option_link(link).tooltip(tl!("Click to edit")),
        link_output(cx.pass, key),
    );
    // The first trailing part added is the outermost: the profile closes the row, the swatch sits
    // nearest the label.
    if let AppearanceItem::Stroke(st) = it {
        if let Some(points) = st.profile.as_ref().filter(|p| p.preset_id() != Some("uniform")).map(|p| p.points.clone()) {
            let aspect = ThumbAspect::new(PROFILE_ASPECT.0, PROFILE_ASPECT.1);
            let profile = Picture::painter(Label::Empty, aspect, move |painter, rect| stroke::paint_profile(painter, rect, Some(points.as_slice())));
            row = row.trailing_field(profile, |_| {});
        }
        if let Some(notes) = stroke_notes(st) {
            row = row.trailing_field(Text::new(notes).shade(Shade::Muted), |_| {});
        }
        let target = if new_art { WeightTarget::Shown } else { WeightTarget::Item(i) };
        let pass = cx.pass;
        row = row.trailing_field(stroke::weight_number(cx.app, Some(st.width)), move |out| {
            pass.borrow_mut().acts.push(Act::Weight(target, Box::new(out)))
        });
    } else {
        // A fill row keeps the weight's room empty, so its swatch stands in the stroke rows' swatch column.
        row = row.trailing_space(stroke::weight_number(cx.app, None));
    }
    let painted = if new_art { Painted::NewArt { fill } } else { Painted::Item(i) };
    let pass = cx.pass;
    row.trailing_field(paint_swatch(it.is_fill(), it.paint()), move |out: SwatchOutput| {
        let mut pass = pass.borrow_mut();
        if let Some(activation) = out.activation {
            pass.acts.push(Act::Swatch(key, painted, activation.mods.shift));
        }
        pass.swatches.push((key, painted, out.response));
    })
}

/// The well of a fill (`fill`) or stroke row, named for it: `paint` as its face, its colour (a solid paint's) as its
/// value.
fn paint_swatch(fill: bool, paint: &Paint) -> Swatch {
    let color = paint.color().map_or(Srgba::BLACK, |c| {
        let [r, g, b] = c.to_rgb();
        Srgba::rgb(r, g, b)
    });
    let row = if fill { tl!("Fill") } else { tl!("Stroke") };
    Swatch::new(crate::i18n::fmt(tl!("{row} swatch"), &[("row", row)]), color)
        .face(paint_preview::rect_face(paint))
        .tooltip(tl!("Click to choose a swatch, Shift-click to mix a colour"))
}

/// A stroke row's notes after the weight: its brush and "Dashed".
fn stroke_notes(st: &vectorcraft_doc::StrokeLayer) -> Option<String> {
    let dashed = st.dash.as_ref().is_some_and(|d| d.is_dashed()).then_some(tl!("Dashed"));
    let notes: Vec<&str> = st.brush.as_deref().into_iter().chain(dashed).collect();
    (!notes.is_empty()).then(|| notes.join(", "))
}

/// An Opacity row: of the object or, under it, of a fill/stroke. "Opacity:" opens the
/// Transparency panel on it. A mixed selection whose objects' opacity or blend mode differ
/// reads "Mixed".
fn opacity_row<'r>(cx: &'r RowCx<'_>, key: RowKey) -> ListRow<'r> {
    let shown = match (key, cx.node) {
        (RowKey::ItemOpacity(i), Some(n)) => n.appearance.items.get(i).map_or_else(String::new, |it| opacity_text(it.opacity(), it.blend())),
        _ if cx.mix.is_some_and(|m| m.transparency) => tl!("Mixed").to_string(),
        (_, Some(n)) => opacity_text(n.opacity, n.blend),
        (_, None) => String::new(),
    };
    ListRow::new(tl!("Opacity"))
        .toggle(ToggleCell::eye(EYE, true))
        .cell_field(NAME, crate::panels::option_link(tl!("Opacity:")).tooltip(tl!("Click to edit")), link_output(cx.pass, key))
        // A screen reader reads the shown opacity as the row's description.
        .description(shown.clone())
        .trailing(shown)
}

/// The Contents (type: Characters) row: picking it sends paint edits to the members
/// (characters); double-clicking Contents targets the members, Characters opens the Color panel
/// on the characters' paint (whose chips it shows), and its name the Character panel.
fn contents_row<'r>(cx: &'r RowCx<'_>) -> ListRow<'r> {
    let Some((n, label)) = cx.node.and_then(|n| n.contents_label().map(|label| (n, tl!(label).to_string()))) else {
        return ListRow::new(Label::Empty);
    };
    let characters = matches!(n.kind, vectorcraft_doc::NodeKind::Text(_));
    let mut row = ListRow::new(label.clone()).toggle(ToggleCell::eye(EYE, true));
    if characters {
        row = row.cell_field(NAME, crate::panels::option_link(label).tooltip(tl!("Click to edit")), link_output(cx.pass, RowKey::Contents));
        // The characters' stroke closes the row, their fill before it.
        for stroke in [true, false] {
            let paint = n.proxy_paint_only(stroke, None).cloned().unwrap_or(Paint::None);
            let chip = Picture::painter(Label::Empty, ThumbAspect::SQUARE, move |painter, rect| paint_preview::paint_rect(painter, rect, &paint));
            row = row.trailing_field(chip, |_| {});
        }
    }
    match n.kind {
        // An image's row has no members to target.
        vectorcraft_doc::NodeKind::Image(_) => row,
        _ if characters => row.tooltip(tl!("Double-click to edit the characters' paint")),
        _ => row.tooltip(tl!("Double-click to target the contents")),
    }
}

/// An effect row: of the object or, under it, of a fill/stroke. Its name edits the effect on a
/// click: its settings dialog, or the inline editor of an effect without settings; its disclosure
/// shows the inline editor.
fn effect_row<'r>(cx: &'r RowCx<'_>, key: RowKey, item: &mut ItemCx<'_, 'r, RowKey>) -> ListRow<'r> {
    let Some((list, k)) = key.effect() else { return ListRow::new(Label::Empty) };
    let Some(e) = cx.node.and_then(|n| n.appearance.effects_at(list)).and_then(|fx| fx.get(k)) else { return ListRow::new(Label::Empty) };
    let label = effect_row_label(&e.id);
    let tip = skipped_effect_tip(&e.id).unwrap_or_else(|| tl!("Click to edit").to_string());
    let row = ListRow::new(label.clone())
        .toggle(ToggleCell::eye(EYE, e.visible))
        .cell_field(NAME, crate::panels::option_link(label).tooltip(tip), link_output(cx.pass, key))
        .trailing_icon(fx_icon(), Label::Empty)
        .tooltip(tl!("Double-click to edit"));
    if cx.open.contains(&key) {
        item.detail(effect_editor(cx.app, cx.pass, list, k, e));
    }
    row
}

/// The inline editor of effect `k` of list `item`: the rows of the effect's dialog over its
/// current values ([`editor_params`]), or "No options". Each edit sends the parameters it changed
/// through `effect.setParams`; Free Distort's Edit… opens its dialog, where its corners are
/// dragged.
fn effect_editor<'r>(app: &VectorcraftApp, pass: &'r RefCell<Pass>, item: Option<usize>, k: usize, e: &Effect) -> impl FnOnce(&mut Ui) + 'r {
    let values = editor_params(e);
    let free_distort = e.id == crate::dialogs::free_distort::FREE_DISTORT;
    let params: Vec<Param> = if free_distort || values.is_empty() {
        vec![]
    } else {
        // The rows a plug-in shows depend on its values: the first list reads them, the second
        // shows them.
        let effect = EffectCx::new(app, &e.id, true);
        let opening = fields_of(&effect_params(&effect, &DialogFields::new()), &values);
        let mut rows = effect_params(&effect, &opening);
        // Load Texture… loads its file in the effect's dialog: the editor shows the parameters.
        rows.retain(|row| row.key != "texturePath");
        // An effect with no rows of its own (a plug-in since removed) shows its values as they are.
        if rows.is_empty() { stored_params(&effect, &values) } else { rows }
    };
    let fields = fields_of(&params, &values);
    let (name, id) = (effect_label(&e.id), e.id.clone());
    move |ui| {
        if free_distort {
            // The button keeps its own name: the label says what it edits.
            let edit = Field::new(tl!("Corners"))
                .side(SideLabel::new().role(FieldLabelRole::Describes))
                .show(ui, |ui, _| clicked(&Button::new(tl!("Edit…")).show(ui)))
                .inner;
            if edit == Some(true) {
                pass.borrow_mut().acts.push(Act::Dialog(item, k, id));
            }
            return;
        }
        if params.is_empty() {
            Text::new(tl!("No options")).shade(Shade::Muted).show(ui);
            return;
        }
        let out = ParamRows::new(name, &params, Bind::given(fields.clone())).show(ui);
        let before = json_of(&params, &fields);
        for event in out.events.iter() {
            let (EditEvent::Set { after, .. } | EditEvent::Commit { after, .. }) = event else { continue };
            let changed: serde_json::Map<String, Value> = json_of(&params, after).into_iter().filter(|(key, v)| before.get(key) != Some(v)).collect();
            if !changed.is_empty() {
                pass.borrow_mut().acts.push(Act::Params(item, k, changed));
            }
        }
    }
}

/// The rows of the list, in order: the object row, the note of a mixed selection, then the stack
/// of the object (its fills and strokes with their open Opacity and effect rows, the Contents row
/// in its slot, the object's effects and Opacity) or of new art.
struct StackRows {
    rows: Vec<StackRow>,
}

/// One row of [`StackRows`].
struct StackRow {
    key: RowKey,
    depth: usize,
    /// The eye: on, and whether it toggles.
    eye: Option<(bool, bool)>,
    open: bool,
}

impl StackRows {
    /// The rows of `node`, or of `new_art` with nothing selected; `mix`: a mixed selection (`node`
    /// its first object), which lists only the rows its objects share and no Contents row (each
    /// object's differs).
    fn new(node: Option<&Node>, new_art: &Appearance, mix: Option<&Mixed>, open: &HashSet<RowKey>) -> Self {
        let mut rows = vec![];
        let mut push = |key: RowKey, depth: usize, eye: Option<(bool, bool)>| rows.push(StackRow { key, depth, eye, open: open.contains(&key) });
        push(RowKey::Object, 0, None);
        let Some(n) = node else {
            for (i, it) in new_art.items.iter().enumerate().rev() {
                push(RowKey::NewArtItem(i), 0, Some((it.visible(), false)));
                for (k, e) in it.effects().iter().enumerate() {
                    push(RowKey::NewArtEffect(Some(i), k), 1, Some((e.visible, false)));
                }
            }
            for (k, e) in new_art.effects.iter().enumerate() {
                push(RowKey::NewArtEffect(None, k), 0, Some((e.visible, false)));
            }
            push(RowKey::NewArtOpacity, 0, Some((true, false)));
            return StackRows { rows };
        };
        if let Some(m) = mix {
            push(RowKey::Mixed, 0, None);
            if m.shared.items.is_empty() && m.shared.effects.is_empty() {
                return StackRows { rows };
            }
        }
        let contents = n.contents_label().is_some() && mix.is_none();
        let below = n.appearance.contents_at();
        for (i, it) in n.appearance.items.iter().enumerate().rev() {
            if mix.is_some_and(|m| !m.shared.has_item(i)) {
                continue;
            }
            // The Contents row sits right above the items painted below the members.
            if contents && i + 1 == below {
                push(RowKey::Contents, 0, Some((true, false)));
            }
            push(RowKey::Item(i), 0, Some((it.visible(), true)));
            if open.contains(&RowKey::Item(i)) {
                push(RowKey::ItemOpacity(i), 1, Some((true, false)));
                for (k, e) in it.effects().iter().enumerate() {
                    push(RowKey::ItemEffect(i, k), 1, Some((e.visible, true)));
                }
            }
        }
        if contents && below == 0 {
            push(RowKey::Contents, 0, Some((true, false)));
        }
        for (k, e) in n.appearance.effects.iter().enumerate() {
            if mix.is_none_or(|m| m.shared.has_effect(k)) {
                push(RowKey::Effect(k), 0, Some((e.visible, true)));
            }
        }
        push(RowKey::Opacity, 0, Some((true, false)));
        StackRows { rows }
    }

    /// The keys in display order.
    fn keys(&self) -> Vec<RowKey> {
        self.rows.iter().map(|row| row.key).collect()
    }
}

impl DisplayOrder<RowKey> for StackRows {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> RowKey {
        self.rows.get(at).map_or(RowKey::Object, |row| row.key)
    }

    fn depth(&self, at: usize) -> usize {
        self.rows.get(at).map_or(0, |row| row.depth)
    }

    fn has_children(&self, at: usize) -> bool {
        self.rows.get(at).is_some_and(|row| row.key.has_children())
    }

    fn open(&self, at: usize) -> bool {
        self.rows.get(at).is_some_and(|row| row.open)
    }

    /// The note and new art's rows are never picked: they only show what is there.
    fn enabled(&self, at: usize) -> bool {
        self.rows
            .get(at)
            .is_some_and(|row| !matches!(row.key, RowKey::Mixed | RowKey::NewArtItem(_) | RowKey::NewArtEffect(..) | RowKey::NewArtOpacity))
    }
}

impl ItemSource<RowKey> for StackRows {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == EYE && self.rows.get(at).and_then(|row| row.eye).is_some_and(|(on, _)| on)
    }

    fn toggle_enabled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == EYE && self.rows.get(at).and_then(|row| row.eye).is_some_and(|(_, toggles)| toggles)
    }
}

/// The object the panel lists: the targeted layer, group or object (`layer.target`), else the
/// first selected one, as the `Arc` the document holds.
fn first_subject(app: &VectorcraftApp) -> Option<std::sync::Arc<Node>> {
    let st = app.session.active()?;
    st.selection.subjects().first().and_then(|id| super::layers::node_arc(&st.doc, *id))
}

/// The selection as the Control bar and the object row name it: "No Selection", the kind ("Path",
/// "Type", "Layer"…) of the object, or targeted layer, the panel lists, or of every selected
/// object when they are all of one kind, else "Mixed Objects".
pub(crate) fn object_label(app: &VectorcraftApp) -> &'static str {
    let Some(st) = app.session.active() else { return "No Selection" };
    let mut kinds = st.selection.subjects().iter().map(|id| st.doc.node(*id).map_or("No Selection", Node::kind_label));
    let Some(first) = kinds.next() else { return "No Selection" };
    if kinds.all(|k| k == first) { first } else { "Mixed Objects" }
}

/// The selected objects the panel lists (several, at least one differing from the first in
/// appearance, opacity or blend mode): what their stacks share, and whether their opacity and blend
/// mode differ. `None` for one object, or several alike.
fn mixed(app: &VectorcraftApp) -> Option<Mixed> {
    let st = app.session.active()?;
    let nodes: Vec<&Node> = st.selection.subjects().iter().filter_map(|id| st.doc.node(*id)).collect();
    let (first, rest) = nodes.split_first()?;
    let transparency = rest.iter().any(|n| n.opacity != first.opacity || n.blend != first.blend);
    if rest.is_empty() || (!transparency && rest.iter().all(|n| n.appearance == first.appearance)) {
        return None;
    }
    let aps: Vec<&vectorcraft_doc::Appearance> = nodes.iter().map(|n| &*n.appearance).collect();
    let fills_differ = rest.iter().any(|n| n.appearance.fill_paint() != first.appearance.fill_paint());
    Some(Mixed { shared: vectorcraft_doc::appearance::SharedStack::of(&aps), transparency, fills_differ })
}

/// A mixed selection as the panel lists it ([`mixed`]).
struct Mixed {
    /// The fills, strokes and effects the objects share: the rows listed.
    shared: vectorcraft_doc::appearance::SharedStack,
    /// The objects' opacity or blend mode differ: the Opacity row reads "Mixed".
    transparency: bool,
    /// Their topmost fills differ: the object row shows no fill chip.
    fills_differ: bool,
}

/// The selection's [`object_label`] and [`mixed`] summary for the active document as of one
/// revision and one list of appearance subjects ([`vectorcraft_doc::Selection::subjects`]), shared by
/// the Control bar and the Appearance panel ([`summary`]).
pub(crate) struct Summary {
    uid: u64,
    revision: u64,
    subjects: Vec<NodeId>,
    /// [`object_label`] of these subjects.
    pub(crate) label: &'static str,
    /// [`mixed`] of these subjects, worked out the first time the Appearance panel asks for it.
    mixed: OnceLock<Option<Mixed>>,
}

impl Summary {
    /// [`mixed`] of the subjects this summary was made for.
    fn mixed<'a>(&'a self, app: &VectorcraftApp) -> Option<&'a Mixed> {
        self.mixed.get_or_init(|| mixed(app)).as_ref()
    }
}

/// The active document's [`Summary`], made again only when the document changes (its id or its
/// revision) or the appearance subjects do, compared id by id; kept in
/// `CanvasCache::appearance_summary`. `None` with no document open.
pub(crate) fn summary(app: &mut VectorcraftApp) -> Option<Arc<Summary>> {
    let st = app.session.active()?;
    let subjects = st.selection.subjects();
    if let Some(s) = &app.canvas.appearance_summary
        && s.uid == st.uid
        && s.revision == st.revision
        && s.subjects == subjects
    {
        return Some(s.clone());
    }
    let s = Arc::new(Summary { uid: st.uid, revision: st.revision, subjects: subjects.to_vec(), label: object_label(app), mixed: OnceLock::new() });
    app.canvas.appearance_summary = Some(s.clone());
    Some(s)
}

/// `params` of an item or effect command the panel runs, with `aligned` when the selected objects'
/// stacks differ: its indices are then rows of the first object's stack, which each other object
/// edits where it has them (the panel lists only the rows they share).
pub(super) fn aligned(app: &VectorcraftApp, mut params: Value) -> Value {
    let differ = app.session.active().is_some_and(|st| {
        let mut aps = st.selection.subjects().iter().filter_map(|id| st.doc.node(*id)).map(|n| &n.appearance);
        aps.next().is_some_and(|first| aps.any(|a| a != first))
    });
    if differ && let Some(m) = params.as_object_mut() {
        m.insert("aligned".into(), json!(true));
    }
    params
}

/// Shows or hides a fill/stroke or effect: its eye was clicked.
fn toggle_eye(app: &mut VectorcraftApp, key: RowKey, on: bool) {
    let (cmd, params) = match key {
        RowKey::Item(i) => ("appearance.setItem", json!({"index": i, "visible": on})),
        RowKey::Effect(k) => ("effect.setParams", json!({"item": null, "index": k, "visible": on})),
        RowKey::ItemEffect(i, k) => ("effect.setParams", json!({"item": i, "index": k, "visible": on})),
        _ => return,
    };
    app.run(cmd, aligned(app, params)).ok();
}

/// A double click (or Enter) on a row of `n`: Contents targets the members, Characters opens the
/// Color panel on the characters' paint, an effect row edits the effect.
fn activate(app: &mut VectorcraftApp, n: &Node, key: RowKey) {
    match key {
        RowKey::Contents => {
            select_row(app, Sel::Contents);
            match n.kind {
                vectorcraft_doc::NodeKind::Text(_) => open_row_popup(app, "color", key),
                vectorcraft_doc::NodeKind::Image(_) => {}
                _ => {
                    app.run("appearance.targetContents", json!({})).ok();
                }
            }
        }
        RowKey::Effect(_) | RowKey::ItemEffect(..) => edit_effect(app, n, key),
        _ => {}
    }
}

/// Edits effect row `key` of `n`: its settings dialog, or the inline editor of an effect without
/// settings.
fn edit_effect(app: &mut VectorcraftApp, n: &Node, key: RowKey) {
    let Some((item, k)) = key.effect() else { return };
    let Some(id) = n.appearance.effects_at(item).and_then(|fx| fx.get(k)).map(|e| e.id.clone()) else { return };
    select_row(app, key.sel());
    if has_options(&id) {
        app.run("effect.dialog", aligned(app, json!({"effect": id, "index": k, "item": item}))).ok();
    } else {
        let open = &mut app.ui.appearance_panel.open;
        if !open.remove(&key) {
            open.insert(key);
        }
    }
}

/// A row's name link was clicked: the row is selected and the panel that edits it opens as its
/// popover (an effect edits itself); new art's Fill:/Stroke: first makes that paint of the next
/// object the one the Color panel edits.
fn follow_link(app: &mut VectorcraftApp, node: Option<&Node>, key: RowKey) {
    match key {
        RowKey::Item(i) => {
            select_row(app, Sel::Item(i));
            let stroke = node.and_then(|n| n.appearance.items.get(i)).is_some_and(|it| !it.is_fill());
            open_row_popup(app, if stroke { "stroke" } else { "color" }, key);
        }
        RowKey::ItemOpacity(_) | RowKey::Opacity => {
            select_row(app, key.sel());
            open_row_popup(app, "transparency", key);
        }
        RowKey::Contents => {
            select_row(app, Sel::Contents);
            open_row_popup(app, "character", key);
        }
        RowKey::Effect(_) | RowKey::ItemEffect(..) => {
            if let Some(n) = node {
                edit_effect(app, n, key);
            }
        }
        RowKey::NewArtItem(i) => {
            let fill = app.session.new_art().items.get(i).is_some_and(AppearanceItem::is_fill);
            app.run("paint.toggleActive", json!({ "fill": fill })).ok();
            open_row_popup(app, if fill { "color" } else { "stroke" }, key);
        }
        _ => {}
    }
}

/// A row's swatch was clicked: Shift opens the Color panel on its paint (selecting its row, or for
/// new art making that paint the one the Color panel edits); a plain click shows or hides the
/// document's swatches.
fn swatch_pressed(app: &mut VectorcraftApp, key: RowKey, painted: Painted, shift: bool) {
    if !shift {
        let panel = &mut app.ui.appearance_panel;
        panel.swatches_for = (panel.swatches_for != Some(key)).then_some(key);
        return;
    }
    match painted {
        Painted::Item(i) => select_row(app, Sel::Item(i)),
        Painted::NewArt { fill } => {
            app.run("paint.toggleActive", json!({ "fill": fill })).ok();
        }
    }
    open_row_popup(app, "color", key);
}

/// Opens panel `id` as the popover of row `row`. A panel showing already (in the dock, a floating
/// window or an icon column's flyout) opens no popover.
fn open_row_popup(app: &mut VectorcraftApp, id: &str, row: RowKey) {
    let showing = crate::panel_docking::showing(&app.ui, id);
    app.ui.transient_panel = (!showing).then(|| id.to_string());
    app.ui.appearance_panel.popup_row = (!showing).then_some(row);
}

/// Puts a row's popover away.
fn close_row_popup(app: &mut VectorcraftApp) {
    app.ui.transient_panel = None;
    app.ui.appearance_panel.popup_row = None;
}

/// The popover a row's link opened: its panel beside the panel body `body` (before it, else after
/// it when there is no room), level with the row's link (`links`), so it shows which row it
/// belongs to. Escape, a press outside, the row leaving the view or the panel showing elsewhere
/// puts it away.
fn show_row_popup(app: &mut VectorcraftApp, ui: &mut Ui, links: &[(RowKey, egui::Response)], body: Rect) {
    let (Some(row), Some(id)) = (app.ui.appearance_panel.popup_row, app.ui.transient_panel.clone()) else { return };
    let opener = links.iter().find(|(key, _)| *key == row).map(|(_, response)| response);
    let Some(opener) = opener.filter(|_| !crate::panel_docking::showing(&app.ui, &id)) else { return close_row_popup(app) };
    let band = Rect::from_x_y_ranges(body.x_range(), opener.rect.y_range());
    let label = crate::state::all_panels().find(|(p, _)| *p == id).map_or(id.as_str(), |(_, l)| l).to_string();
    let tall = (ui.ctx().content_rect().height() * 0.6).max(POPUP_MIN_TALL);
    let mut open = true;
    Popover::new(tl!(&label).to_string(), Bind::bound(&mut open))
        .opened_by(opener)
        .overlay(|o| o.place(Place::anchored(Anchor::Rect(band), Side::Start, Align::Start).flipping_across()))
        .show(ui, |ui, _| {
            ui.allocate_ui(vec2(POPUP_W, tall), |ui| crate::dock::panel_body(app, ui, &id));
        });
    if !open {
        close_row_popup(app);
    }
}

/// The swatches a row's swatch shows (`swatches`: the swatches shown, with what they paint); a pick
/// or a press outside puts them away, as does the row leaving the view.
fn show_swatch_popup(app: &mut VectorcraftApp, ui: &mut Ui, swatches: &[(RowKey, Painted, egui::Response)]) {
    let Some(row) = app.ui.appearance_panel.swatches_for else { return };
    let Some((_, target, opener)) = swatches.iter().find(|(key, ..)| *key == row) else {
        app.ui.appearance_panel.swatches_for = None;
        return;
    };
    let mut open = true;
    let picked = Popover::new(tl!("Swatches"), Bind::bound(&mut open)).opened_by(opener).show(ui, |ui, _| swatch_picker(app, ui, *target)).inner;
    if !open || picked == Some(true) {
        app.ui.appearance_panel.swatches_for = None;
    }
}

/// Does the effect have a dialog (any parameters, or a Pathfinder effect's Pathfinder Options)?
fn has_options(id: &str) -> bool {
    vectorcraft_effects::takes_pathfinder_options(id)
        || vectorcraft_effects::default_params(id).is_some_and(|d| d.as_object().is_some_and(|o| !o.is_empty()))
}

/// The values the inline editor lists: the effect's own values over its defaults, with earlier keys
/// in their current form (Scribble's one `variation` as its Path Overlap and Spacing variations).
fn editor_params(e: &Effect) -> serde_json::Map<String, Value> {
    match vectorcraft_effects::merged_params(&e.id, &e.params) {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// The paint a fill/stroke row's swatch sets: of the selection's fill/stroke row `index`, or, with
/// nothing selected, the next object drawn's top fill (`fill`) or stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Painted {
    Item(usize),
    NewArt { fill: bool },
}

/// The swatches of one version of a document: the swatch picker's presets.
#[derive(MemoProps)]
struct DocSwatches<'a> {
    uid: u64,
    revision: u64,
    #[memo_props(covered_by = "revision", reason = "a document version's swatches are fixed")]
    doc: &'a Document,
}

/// The document's swatches (its own, then its groups') with their paints.
fn doc_swatches(doc: &Document) -> impl Iterator<Item = (&String, &Paint)> {
    doc.swatches.iter().chain(doc.swatch_groups.iter().flat_map(|g| &g.swatches)).map(|s| (&s.name, &s.paint))
}

/// The document's swatches (its own, then its groups') as chips; a pick paints `target`. Whether a
/// swatch was picked.
fn swatch_picker(app: &mut VectorcraftApp, ui: &mut Ui, target: Painted) -> bool {
    // The source borrows the document until it drops, before the pick runs its command.
    let (name, none) = {
        let Some(st) = app.session.active() else { return false };
        let fns = PresetFns::<DocSwatches<'_>, String>::new(|s, _| {
            doc_swatches(s.doc)
                .map(|(name, paint)| {
                    PresetRow::item(name.clone(), name.clone(), 0).preview_face(ThumbAspect::SQUARE, paint_preview::rect_face(paint))
                })
                .collect()
        });
        let source = PresetSource::new(DocSwatches { uid: st.uid, revision: st.revision, doc: &st.doc }, fns);
        let out = PresetBrowser::new(tl!("Swatches"), Bind::given(PresetState::default()), &source)
            .tiles(PresetTiles::Chips)
            .view_lock(PresetView::Grid)
            .row_menu(PresetRowMenu::Off)
            .columns(SWATCH_COLUMNS)
            .max_rows(SWATCH_ROWS)
            .show(ui);
        let picked = out.events.into_iter().find_map(|event| match event {
            PresetEvent::Select { key, .. } | PresetEvent::Activate { key, .. } => Some(key),
            _ => None,
        });
        let Some(name) = picked else { return false };
        let none = doc_swatches(&st.doc).any(|(n, p)| *n == name && p.is_none());
        (name, none)
    };
    let paint = if none { json!({"none": true}) } else { json!({ "swatch": name }) };
    let (cmd, mut params) = match target {
        Painted::Item(index) => ("appearance.setItem", json!({ "index": index })),
        // With nothing selected these set up the next object drawn.
        Painted::NewArt { fill: true } => ("paint.setFill", json!({})),
        Painted::NewArt { fill: false } => ("paint.setStroke", json!({})),
    };
    if let (Some(p), Some(extra)) = (params.as_object_mut(), paint.as_object()) {
        p.extend(extra.clone());
    }
    // On a mixed selection, the row is the first object's: each other one paints its equal one.
    let params = if matches!(target, Painted::Item(_)) { aligned(app, params) } else { params };
    app.run(cmd, params).ok();
    true
}

/// The footer: Add New Stroke, Add New Fill and Add New Effect at the start; Clear Appearance,
/// Duplicate Selected Item and Delete Selected Item at the end, the last two taking rows of the
/// list `list` dropped on them, also while disabled.
fn footer_bar(has: bool, can: bool, can_delete: bool, list: egui::Id) -> FooterBar {
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    let drops = move |copy: bool| move |drag: &ListDrag<RowKey>| has && drag.list == list && button_drop_command(&drag.keys, copy).is_some();
    FooterBar::actions([
        FooterAction::new(icon("dc-new-stroke"), tl!("Add New Stroke")).enabled(has),
        FooterAction::new(icon("dc-new-fill"), tl!("Add New Fill")).enabled(has),
        FooterAction::new(icon("dc-fx"), tl!("Add New Effect")).enabled(has).popup(PopupKind::Menu),
        FooterAction::new(icon("trash-2"), tl!("Delete Selected Item")).enabled(can_delete).takes_drops_disabled(true).accepts_drops(drops(false)),
        FooterAction::new(icon("dc-new-item"), tl!("Duplicate Selected Item")).enabled(can).takes_drops_disabled(true).accepts_drops(drops(true)),
        FooterAction::new(icon("dc-clear"), tl!("Clear Appearance")).enabled(has),
    ])
    .at_start(ADD_EFFECT + 1)
}

/// Rows `keys` of `n` dropped on Duplicate Selected Item (`copy`) or Delete Selected Item are
/// copied or removed, as rows dropped on the Layers panel's buttons are, and the selection follows
/// ([`highlight_copies`], [`highlight_after_removal`]); `sel` is the active row.
fn drop_on_footer(app: &mut VectorcraftApp, n: &Node, sel: Sel, keys: &[RowKey], copy: bool) {
    let Some((cmd, params)) = button_drop_command(keys, copy) else { return };
    let gone: Vec<usize> = params["indices"].as_array().map(|a| a.iter().filter_map(Value::as_u64).map(|k| k as usize).collect()).unwrap_or_default();
    let rows = effects_of(keys).map_or(Rows::Items, |(list, _)| Rows::Effects(list));
    if app.run(cmd, aligned(app, params)).is_err() {
        return;
    }
    if copy {
        return highlight_copies(app, n, sel, rows, &gone);
    }
    // When the active effect was among them, its fill/stroke (or nothing) is selected; the engine
    // clears or renumbers the active fill/stroke itself.
    let active_gone = match (rows, sel) {
        (Rows::Effects(None), Sel::Effect(a)) => gone.contains(&a),
        (Rows::Effects(Some(i)), Sel::ItemEffect(j, a)) => i == j && gone.contains(&a),
        _ => false,
    };
    match rows {
        Rows::Effects(item) if active_gone => select_row(app, item.map_or(Sel::None, Sel::Item)),
        _ => highlight_after_removal(app, n, sel, rows, &gone),
    }
}

/// After the rows `gone` of kind `rows` were removed while the active row `sel` survived, the active
/// row, the picked rows and the anchor move up past the removed rows below them; removed rows drop
/// out. `n` is the object as it was before the removal.
fn highlight_after_removal(app: &mut VectorcraftApp, n: &Node, sel: Sel, rows: Rows, gone: &[usize]) {
    if sel.rows().is_some_and(|(r, _)| r != rows) {
        return;
    }
    let held = list_selection(app, n, sel);
    let shift = |k: usize| (!gone.contains(&k)).then(|| k - gone.iter().filter(|g| **g < k).count());
    let moved = |key: &RowKey| key.sel().rows().filter(|(r, _)| *r == rows).and_then(|(_, k)| shift(k)).map(|k| rows.key(k));
    // A fill/stroke's Opacity row stays selected (renumbered) while its fill/stroke survives.
    let lead = match (sel, rows) {
        (Sel::ItemOpacity(i), Rows::Items) => shift(i).map(RowKey::ItemOpacity),
        _ => RowKey::of(sel).and_then(|key| moved(&key)),
    };
    let mut items: Vec<RowKey> = held.items.iter().filter_map(moved).collect();
    if let Some(lead) = lead.filter(|lead| !items.contains(lead)) {
        items.push(lead);
    }
    let anchor = held.anchor.and_then(|anchor| moved(&anchor)).or(lead);
    set_active(app, lead.map_or(Sel::None, RowKey::sel));
    app.ui.appearance_panel.selection = Selection::new(items, lead, anchor);
}

/// The fill/stroke rows of `keys`, ascending, when every one is one.
fn items_of(keys: &[RowKey]) -> Option<Vec<usize>> {
    let mut set = keys
        .iter()
        .map(|key| match key {
            RowKey::Item(i) => Some(*i),
            _ => None,
        })
        .collect::<Option<Vec<usize>>>()?;
    set.sort_unstable();
    (!set.is_empty()).then_some(set)
}

/// The effect list (`None`: the object's) and positions, ascending, of `keys`, when every one is
/// an effect row of one list.
fn effects_of(keys: &[RowKey]) -> Option<(Option<usize>, Vec<usize>)> {
    let mut found = keys.iter().map(|key| key.effect());
    let (list, first) = found.next()??;
    let mut set = vec![first];
    for effect in found {
        let (other, k) = effect?;
        if other != list {
            return None;
        }
        set.push(k);
    }
    set.sort_unstable();
    Some((list, set))
}

/// The command the rows `keys` dropped on Duplicate Selected Item (`copy`) or Delete Selected Item
/// run: the dragged fills/strokes, or effects of one list. The Contents row and the other rows are
/// neither copied nor removed (None).
fn button_drop_command(keys: &[RowKey], copy: bool) -> Option<(&'static str, Value)> {
    if let Some(indices) = items_of(keys) {
        return Some((if copy { "appearance.duplicateItem" } else { "appearance.removeItem" }, json!({ "indices": indices })));
    }
    let (item, indices) = effects_of(keys)?;
    Some((if copy { "effect.duplicate" } else { "effect.remove" }, json!({"item": item, "indices": indices})))
}

/// Duplicate the picked fills/strokes or effects of `node` with `sel` active: each copy shows right
/// above its row and the copies are picked ([`highlight_copies`]).
fn duplicate_selected(app: &mut VectorcraftApp, node: Option<&Node>, sel: Sel) -> Result<(), String> {
    let Some(n) = node else { return Ok(()) };
    let (rows, indices) = match selected_effects(app, n, sel) {
        Some((item, ks)) => (Rows::Effects(item), ks),
        None if matches!(sel, Sel::Item(_)) => (Rows::Items, selected_items(app, n, sel)),
        None => return Ok(()),
    };
    let (cmd, params) = match rows {
        Rows::Effects(item) => ("effect.duplicate", json!({"item": item, "indices": indices})),
        Rows::Items => ("appearance.duplicateItem", json!({ "indices": indices })),
    };
    app.run(cmd, aligned(app, params))?;
    highlight_copies(app, n, sel, rows, &indices);
    Ok(())
}

/// After the rows `dup` of kind `rows` were copied in place, the copies are picked, as the Layers
/// panel picks its duplicates: each shows right above its original, a fill/stroke's at the next
/// paint index, an effect's where the original was (the original one below it). The copy of the
/// active row `sel` becomes active (else the first copy), the copy of the anchor the anchor. `n`
/// is the object as it was before the copy.
fn highlight_copies(app: &mut VectorcraftApp, n: &Node, sel: Sel, rows: Rows, dup: &[usize]) {
    let below = |k: usize| dup.iter().filter(|d| **d < k).count();
    let copy = |k: usize| match rows {
        Rows::Items => k + below(k) + 1,
        Rows::Effects(_) => k + below(k),
    };
    let held = list_selection(app, n, sel);
    let copied = |key: &RowKey| key.sel().rows().filter(|(r, k)| *r == rows && dup.contains(k)).map(|(_, k)| rows.key(copy(k)));
    // The copies of the picked rows in their order, then any other copied row.
    let mut items: Vec<RowKey> = held.items.iter().filter_map(copied).collect();
    for c in dup.iter().map(|k| rows.key(copy(*k))) {
        if !items.contains(&c) {
            items.push(c);
        }
    }
    let lead = RowKey::of(sel).and_then(|key| copied(&key)).or(items.first().copied());
    let anchor = held.anchor.and_then(|anchor| copied(&anchor)).or(lead);
    set_active(app, lead.map_or(Sel::None, RowKey::sel));
    app.ui.appearance_panel.selection = Selection::new(items, lead, anchor);
}

/// Remove the picked fills/strokes or effects of `node` with `sel` active; a removed item effect
/// leaves its fill/stroke selected. A selected Opacity row is set back to Default (100%, Normal)
/// and stays selected.
fn delete_selected(app: &mut VectorcraftApp, node: Option<&Node>, sel: Sel) -> Result<(), String> {
    let Some(n) = node else { return Ok(()) };
    match sel {
        Sel::Opacity => {
            app.run("transparency.set", json!({"item": null, "opacity": 100, "blend": "Normal"}))?;
            return Ok(());
        }
        Sel::ItemOpacity(i) => {
            app.run("transparency.set", aligned(app, json!({"item": i, "opacity": 100, "blend": "Normal"})))?;
            return Ok(());
        }
        _ => {}
    }
    match selected_effects(app, n, sel) {
        Some((item, ks)) => app.run("effect.remove", aligned(app, json!({"item": item, "indices": ks})))?,
        None if matches!(sel, Sel::Item(_)) => app.run("appearance.removeItem", aligned(app, json!({"indices": selected_items(app, n, sel)})))?,
        None => return Ok(()),
    };
    // Removing the active item clears it in the engine; a removed item effect leaves its item
    // selected.
    select_row(app, if let Sel::ItemEffect(i, _) = sel { Sel::Item(i) } else { Sel::None });
    Ok(())
}

/// Whether the rows of `drag` may land before position `insert` of `order` (`over`: the row under
/// the pointer, which takes them when it holds rows): a drop that moves or copies them
/// ([`reorder_command`]).
fn accepts(n: &Node, order: &[RowKey], drag: &ListDrag<RowKey>, over: Option<&RowKey>, insert: usize, copy: bool) -> bool {
    let before = order.get(..insert).map_or(0, |rows| rows.iter().filter(|key| drag.keys.contains(key)).count());
    let mv = ReorderMove::new(drag.grabbed, drag.keys.clone(), insert.saturating_sub(before), copy);
    let into = over.copied().filter(|key| key.has_children()).map(|key| mv.clone().in_group(Some(key)));
    reorder_command(n, order, &mv).is_some() || into.is_some_and(|mv| reorder_command(n, order, &mv).is_some())
}

/// Runs the reorder `mv` of the rows `order` of `n` and picks the moved rows (the copies), the
/// grabbed one the lead; a moved Contents row keeps its selection.
fn reorder(app: &mut VectorcraftApp, n: &Node, order: &[RowKey], mv: &ReorderMove<RowKey>) {
    let Some((cmd, params)) = reorder_command(n, order, mv) else { return };
    let Ok(result) = app.run(cmd, params.clone()) else { return };
    let Some((kind, grabbed)) = mv.key.sel().rows() else { return };
    let mut set: Vec<usize> = mv.keys.iter().filter_map(|key| key.sel().rows().map(|(_, k)| k)).collect();
    set.sort_unstable();
    let (rows, landed) = match kind {
        Rows::Effects(_) => (Rows::Effects(result["item"].as_u64().map(|i| i as usize)), result["index"].as_u64()),
        Rows::Items if mv.copy => (Rows::Items, params["to"].as_u64()),
        Rows::Items => (Rows::Items, result["index"].as_u64().or_else(|| params["to"].as_u64())),
    };
    let Some(landed) = landed.map(|k| k as usize) else { return };
    let keys: Vec<RowKey> = (0..set.len()).map(|at| rows.key(landed + at)).collect();
    let lead = set.iter().position(|k| *k == grabbed).and_then(|at| keys.get(at).copied());
    set_active(app, lead.map_or(Sel::None, RowKey::sel));
    app.ui.appearance_panel.selection = Selection::new(keys, lead, lead);
}

/// The command the reorder `mv` of rows `order` of `n` runs, or None where it would change nothing
/// or the rows can't go.
///
/// `mv.to` is a place between rows of `order` once the moved rows are taken out. Fill/strokes land
/// as one block in their order there, on its side of the Contents row; dropped into a fill/stroke
/// row (`mv.parent`), they land above it when they move up and below it when they move down; Alt
/// copies them there. The Contents row moves the same way. Effects land in the list next to the
/// place (an effect row's, else the object's beside the object's own rows), or at the end of the
/// fill/stroke they are dropped into, or before the effect they are dropped into.
// PLAN: the sheet's `reorder_command(node, &ReorderMove<RowKey>)` cannot place `ReorderMove::to`, a position in the shown rows: it also takes them (`order`).
fn reorder_command(n: &Node, order: &[RowKey], mv: &ReorderMove<RowKey>) -> Option<(&'static str, Value)> {
    if mv.parent.is_some_and(|parent| mv.keys.contains(&parent)) {
        return None;
    }
    let remaining: Vec<RowKey> = order.iter().copied().filter(|key| !mv.keys.contains(key)).collect();
    let gap = mv.to.min(remaining.len());
    if mv.keys == [RowKey::Contents] {
        return contents_move(n, &remaining, gap, mv.parent);
    }
    if let Some(set) = items_of(&mv.keys) {
        return items_move(n, order, &remaining, gap, &set, mv);
    }
    let (from_item, set) = effects_of(&mv.keys)?;
    effects_move(n, &remaining, gap, from_item, &set, mv)
}

/// The place in `remaining` a drop into fill/stroke row `j` stands for: above it for rows that
/// move up to it, below it and its Opacity and effect rows for rows that move down.
fn gap_at(remaining: &[RowKey], j: usize, up: bool) -> Option<usize> {
    let row = remaining.iter().position(|key| *key == RowKey::Item(j))?;
    if up {
        return Some(row);
    }
    let under = remaining.get(row + 1..).map_or(0, |rest| rest.iter().take_while(|key| key.sel().item() == Some(j)).count());
    Some(row + 1 + under)
}

/// How many fill/stroke rows are in `rows`.
fn item_rows(rows: &[RowKey]) -> usize {
    rows.iter().filter(|key| matches!(key, RowKey::Item(_))).count()
}

/// Whether the items `set` (ascending) stand together.
fn contiguous(set: &[usize]) -> bool {
    set.windows(2).all(|w| w.get(1).copied() == w.first().map(|a| a + 1))
}

/// The Contents row moved to `gap` of `remaining` (into fill/stroke `parent`): the items below it
/// afterwards.
fn contents_move(n: &Node, remaining: &[RowKey], gap: usize, parent: Option<RowKey>) -> Option<(&'static str, Value)> {
    let k = n.appearance.contents_at();
    let gap = match parent {
        Some(RowKey::Item(j) | RowKey::ItemEffect(j, _)) => gap_at(remaining, j, j >= k)?,
        _ => gap,
    };
    let to = remaining.get(gap..).map_or(0, item_rows);
    (to != k).then(|| ("appearance.moveItem", json!({"from": "contents", "to": to})))
}

/// The fills/strokes `set` (ascending) moved or copied to `gap` of `remaining`.
fn items_move(
    n: &Node,
    order: &[RowKey],
    remaining: &[RowKey],
    gap: usize,
    set: &[usize],
    mv: &ReorderMove<RowKey>,
) -> Option<(&'static str, Value)> {
    let grabbed = match mv.key {
        RowKey::Item(g) => g,
        _ => *set.first()?,
    };
    let gap = match mv.parent {
        Some(RowKey::Item(j) | RowKey::ItemEffect(j, _)) => gap_at(remaining, j, j > grabbed)?,
        _ => gap,
    };
    let unmoved = remaining.get(gap..).map_or(0, item_rows);
    let k = n.appearance.contents_at();
    let m = set.len();
    let below_contents = remaining.iter().position(|key| *key == RowKey::Contents).is_some_and(|c| gap > c);
    let has_contents = n.contents_label().is_some();
    if mv.copy {
        // The originals stay: those after the place count below the copies too.
        let shown_at = |key: &RowKey| order.iter().position(|o| o == key);
        let above = gap.checked_sub(1).and_then(|at| remaining.get(at)).and_then(shown_at);
        let copied_below = set.iter().filter(|i| above.is_none_or(|a| shown_at(&RowKey::Item(**i)).is_some_and(|p| p > a))).count();
        let to = unmoved + copied_below;
        let params = match set {
            [from] => json!({"index": from, "to": to}),
            _ if has_contents => json!({"indices": set, "to": to, "contents": k + if below_contents { m } else { 0 }}),
            _ => json!({"indices": set, "to": to}),
        };
        return Some(("appearance.duplicateItem", params));
    }
    let contents = has_contents.then(|| (0..k).filter(|i| !set.contains(i)).count() + if below_contents { m } else { 0 });
    if contiguous(set) && set.first() == Some(&unmoved) && contents.is_none_or(|c| c == k) {
        return None;
    }
    let mut params = match set {
        [from] => json!({"from": from, "to": unmoved}),
        _ => json!({"indices": set, "to": unmoved}),
    };
    if let Some(c) = contents {
        params["contents"] = json!(c);
    }
    Some(("appearance.moveItem", params))
}

/// The effects `set` (ascending) of list `from_item` moved or copied to `gap` of `remaining`.
fn effects_move(
    n: &Node,
    remaining: &[RowKey],
    gap: usize,
    from_item: Option<usize>,
    set: &[usize],
    mv: &ReorderMove<RowKey>,
) -> Option<(&'static str, Value)> {
    let (item, pos) = match mv.parent {
        Some(RowKey::Item(i)) => (Some(i), n.appearance.effects_at(Some(i))?.len()),
        Some(key) => key.effect()?,
        None => {
            let above = gap.checked_sub(1).and_then(|at| remaining.get(at)).copied();
            let below = remaining.get(gap).copied();
            match (above.and_then(RowKey::effect), below.and_then(RowKey::effect)) {
                (_, Some(at)) => at,
                (Some((list, k)), None) => (list, k + 1),
                (None, None) if above.into_iter().chain(below).any(RowKey::object_level) => (None, n.appearance.effects.len()),
                (None, None) => return None,
            }
        }
    };
    // Within their own list the effects leave their places first.
    let same = item == from_item && !mv.copy;
    let pos = if same { pos.saturating_sub(set.iter().filter(|s| **s < pos).count()) } else { pos };
    if same && contiguous(set) && set.first() == Some(&pos) {
        return None;
    }
    let from = match set {
        [k] => json!(k),
        _ => json!(set),
    };
    Some(("effect.move", json!({"from": from, "fromItem": from_item, "to": pos, "toItem": item, "copy": mv.copy})))
}

/// The fx menu (Appearance panel, Properties), the body of a framework `Menu`: effects grouped by
/// their Effect-menu submenu; a pick opens the effect dialog.
pub(crate) fn fx_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mut groups: Vec<(String, Vec<(String, String)>)> = vec![];
    let plugins: Vec<_> = vectorcraft_effects::plugin_effects().into_iter().map(catalog_entry).collect();
    // Built-in effects named as the Effect menu names them; a plug-in's by the name it gives.
    let lang = crate::i18n::current();
    let built_in = catalog().iter().map(|(id, label, menu)| (id, crate::i18n::effect_name(lang, id).unwrap_or_else(|| label.clone()), menu));
    for (id, label, menu) in built_in.chain(plugins.iter().map(|(id, label, menu)| (id, label.clone(), menu))) {
        let g = menu.get(1).cloned().unwrap_or_else(|| "Other".into());
        match groups.iter_mut().find(|(n, _)| *n == g) {
            Some((_, v)) => v.push((id.clone(), label.to_string())),
            None => groups.push((g, vec![(id.clone(), label.to_string())])),
        }
    }
    // A menu row reads `&` as its mnemonic marker: `&&` is the character ("Distort & Transform").
    for (g, items) in groups {
        Menu::submenu_row(ui, tl!(&g).replace('&', "&&"), |ui, _| {
            for (id, label) in &items {
                if MenuItem::new(format!("{}…", label.replace('&', "&&"))).show(ui).ran {
                    app.run("effect.dialog", json!({ "effect": id })).ok();
                }
            }
        });
    }
}

/// The panel menu's New Art Has Basic Appearance row: the command turns the preference over.
fn new_art_basic(app: &VectorcraftApp) -> CommandRef {
    CommandRef::with_params("appearance.setNewArtBasic", json!({ "on": !app.session.prefs.new_art_basic }))
}

/// The panel menu's rows, in the reference app's order.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let command = |id: &str| PanelRow::Command(CommandRef::new(id));
    vec![
        command("appearance.addFill"),
        command("appearance.addStroke"),
        PanelRow::Separator,
        command(DUPLICATE_ITEM),
        command(REMOVE_ITEM),
        command("appearance.clear"),
        command("appearance.reduceToBasic"),
        PanelRow::Separator,
        PanelRow::Command(new_art_basic(app)),
        command(THUMBNAIL),
        PanelRow::Separator,
        command("graphicStyle.redefine"),
        command("appearance.showAllHidden"),
    ]
}

/// The enabled, checked and title states of the panel menu's rows.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let node = first_subject(app);
    let has = node.is_some();
    let sel = current_sel(app, node.as_deref());
    let enabled = |on: bool| CommandState::new().enabled(on);
    states.set("appearance.addFill", enabled(has));
    states.set("appearance.addStroke", enabled(has));
    states.set(DUPLICATE_ITEM, enabled(has && sel.editable()));
    states.set(REMOVE_ITEM, enabled(has && sel.deletable()));
    states.set("appearance.clear", enabled(has));
    states.set("appearance.reduceToBasic", enabled(any_subject_needs_reduction(app)));
    states.set(new_art_basic(app), enabled(true).checked(app.session.prefs.new_art_basic));
    let thumbnail = if app.ui.appearance_panel.hide_thumb { tl!("Show Thumbnail") } else { tl!("Hide Thumbnail") };
    states.set(THUMBNAIL, enabled(true).title(thumbnail));
    let style = app.session.selection_graphic_style().map(|(g, _)| g.name.clone());
    // Translated around the style's name.
    let redefine = style
        .as_ref()
        .map_or_else(|| tl!("Redefine Graphic Style").to_string(), |n| crate::i18n::fmt(tl!("Redefine Graphic Style “{name}”"), &[("name", n)]));
    states.set("graphicStyle.redefine", enabled(style.is_some()).title(redefine));
    states.set("appearance.showAllHidden", enabled(node.as_ref().is_some_and(|n| n.appearance.has_hidden())));
}

/// Runs the panel's own menu commands (`ui.appearance.*`); `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let done = match id {
        DUPLICATE_ITEM | REMOVE_ITEM => {
            let node = first_subject(app);
            let sel = current_sel(app, node.as_deref());
            if id == DUPLICATE_ITEM { duplicate_selected(app, node.as_deref(), sel) } else { delete_selected(app, node.as_deref(), sel) }
        }
        THUMBNAIL => {
            let panel = &mut app.ui.appearance_panel;
            panel.hide_thumb = !panel.hide_thumb;
            Ok(())
        }
        _ => return None,
    };
    Some(done.map(|()| Value::Null))
}

#[cfg(test)]
mod tests;
