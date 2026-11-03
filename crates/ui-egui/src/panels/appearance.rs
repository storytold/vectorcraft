//! Appearance panel: the full stack as in the reference app — the object row (its thumbnail drags
//! the appearance onto art), each stroke/fill row (eye, disclosure, link label, swatch, weight,
//! brush/dash/profile notes) with "Opacity: Default" sub-rows, fx rows (the name opens the
//! effect's dialog; the disclosure an inline editor), the object's Opacity row and the bottom bar.
//! Groups and layers show a Contents row and type a Characters row where their members
//! (characters) paint among the object's own fills and strokes: double-clicking Contents targets
//! the members, double-clicking Characters edits the characters' paint and its name opens the
//! Character panel. The panels a row's link opens (Color, Stroke, Transparency, Character) are
//! popups: a press outside closes them, and their first text field takes the keyboard.
//!
//! Rows drag: a fill/stroke row reorders the stack (across the Contents row too), the Contents row
//! moves between the fills and strokes, an effect row reorders its list or moves into another item
//! (dropped on its row) or onto the object; Alt copies instead. Dropped on Duplicate Selected Item
//! or Delete Selected Item, a fill/stroke row (with the other selected rows, when it is one of
//! them) or an effect row is copied or removed. Cmd-click and Shift-click select several
//! fills/strokes for Duplicate and Delete.

use std::sync::OnceLock;

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{AppearanceItem, Effect, Node, NodeId};

use super::{pstate, set_pstate};
use crate::panels::stroke::paint_profile;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item};
use crate::{VectorcraftApp, icons};

const ROW: f32 = 30.0;
pub(super) const EYE_W: f32 = 26.0;

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
    /// Is a fill/stroke or effect row selected (what Duplicate and Delete act on)? An Opacity row
    /// is not an item: it can't be copied or removed.
    fn editable(self) -> bool {
        matches!(self, Sel::Item(_) | Sel::Effect(_) | Sel::ItemEffect(..))
    }
}

/// The selected row of `node` (the first selected object): a selected effect row while that
/// effect still exists and agrees with the engine's active item, else the active item's row.
fn current_sel(app: &VectorcraftApp, ctx: &egui::Context, node: Option<&Node>) -> Sel {
    let Some(n) = node else { return Sel::None };
    let (owner, sel): (Option<NodeId>, Sel) = pstate(ctx, "ap-sel");
    let exists = |item: Option<usize>, k: usize| owner == Some(n.id) && n.appearance.effects_at(item).is_some_and(|fx| k < fx.len());
    match (app.session.appearance_item(), sel) {
        (Some(i), Sel::ItemEffect(j, k)) if i == j && exists(Some(i), k) => sel,
        (Some(i), Sel::ItemOpacity(j)) if i == j && owner == Some(n.id) => sel,
        (Some(i), _) => Sel::Item(i),
        (None, Sel::Effect(k)) if exists(None, k) => sel,
        (None, Sel::Contents) if owner == Some(n.id) && n.contents_label().is_some() => sel,
        (None, Sel::Opacity) if owner == Some(n.id) => sel,
        (None, _) => Sel::None,
    }
}

/// Which rows a highlight is made of: fills/strokes, or the effects of one list (`None`: the
/// object's own). Rows of different kinds or lists are never highlighted together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Rows {
    #[default]
    Items,
    Effects(Option<usize>),
}

impl Rows {
    /// The row `index` of this kind as a selection.
    fn sel(self, index: usize) -> Sel {
        match self {
            Rows::Items => Sel::Item(index),
            Rows::Effects(None) => Sel::Effect(index),
            Rows::Effects(Some(i)) => Sel::ItemEffect(i, index),
        }
    }
    /// The rows of this kind on `n`, listed top-down as the panel shows them (fills/strokes from the
    /// top of the paint order, effects in list order).
    fn order(self, n: &Node) -> Vec<usize> {
        match self {
            Rows::Items => (0..n.appearance.items.len()).rev().collect(),
            Rows::Effects(item) => (0..n.appearance.effects_at(item).map_or(0, |fx| fx.len())).collect(),
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

/// The highlighted rows of an object (panel state `"ap-multi"`): their kind and list, the rows in
/// the order they were highlighted (as `layer.highlight` keeps the Layers panel's), and the anchor
/// Shift ranges start from (the last row clicked without Shift), and the row that was active when
/// they were highlighted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Highlight {
    owner: Option<NodeId>,
    rows: Rows,
    order: Vec<usize>,
    anchor: Option<usize>,
    active: Option<usize>,
}

/// Make `sel` the active row; its fill/stroke becomes the active item that paint and effect edits
/// target. The highlighted rows stay as they are.
fn set_active(app: &mut VectorcraftApp, ctx: &egui::Context, sel: Sel) {
    if app.session.appearance_item() != sel.item() {
        app.run("appearance.setActiveItem", json!({ "index": sel.item() })).ok();
    }
    let owner = app.session.active().and_then(|d| d.selection.subjects().first().copied());
    set_pstate(ctx, "ap-sel", (owner, sel));
}

/// Select a row; its fill/stroke becomes the active item that paint and effect edits target.
/// It is the only highlighted row and the anchor.
pub(crate) fn select_row(app: &mut VectorcraftApp, ctx: &egui::Context, sel: Sel) {
    set_active(app, ctx, sel);
    let owner = app.session.active().and_then(|d| d.selection.subjects().first().copied());
    let (rows, order) = sel.rows().map_or((Rows::Items, vec![]), |(r, k)| (r, vec![k]));
    let anchor = order.first().copied();
    set_pstate(ctx, "ap-multi", Highlight { owner, rows, order, anchor, active: anchor });
}

/// The highlighted rows of `n` with `sel` active: the stored ones while they belong to `n`, are of
/// the active row's kind and list and hold it (rows that no longer exist dropped), else just the
/// active row. When the active row has changed outside the panel since (Add New Fill makes the new
/// fill active), also just the active row. With no row active, none (the anchor stays).
fn highlight(ctx: &egui::Context, n: &Node, sel: Sel) -> Highlight {
    let mut h: Highlight = pstate(ctx, "ap-multi");
    let ours = h.owner == Some(n.id);
    let Some((rows, active)) = sel.rows() else {
        return Highlight { owner: Some(n.id), order: vec![], anchor: h.anchor.filter(|_| ours), rows: h.rows, active: None };
    };
    if !ours || h.rows != rows || h.active != Some(active) || !h.order.contains(&active) {
        return Highlight { owner: Some(n.id), rows, order: vec![active], anchor: Some(active), active: Some(active) };
    }
    let len = rows.order(n).len();
    h.order.retain(|k| *k < len);
    h
}

/// The highlighted fill/stroke rows of `n` (paint order, ascending): the active row plus those
/// Cmd/Shift-clicked with it.
fn selected_items(ctx: &egui::Context, n: Option<&Node>, sel: Sel) -> Vec<usize> {
    let (Some(n), Sel::Item(_)) = (n, sel) else { return vec![] };
    let mut set = highlight(ctx, n, sel).order;
    set.sort_unstable();
    set
}

/// The highlighted effect rows of `n`: their list (`None`: the object's) and positions, ascending.
fn selected_effects(ctx: &egui::Context, n: Option<&Node>, sel: Sel) -> Option<(Option<usize>, Vec<usize>)> {
    let (n, (Rows::Effects(list), _)) = (n?, sel.rows()?) else { return None };
    let mut set = highlight(ctx, n, sel).order;
    set.sort_unstable();
    Some((list, set))
}

/// Click on row `k` of kind `rows` with modifiers `m`, by the Layers panel's rules
/// ([`widgets::row_click`]): Shift highlights the range from the anchor, Shift+Ctrl/Cmd adds it,
/// Ctrl/Cmd toggles the row, a plain click highlights only it. A row of another kind or list starts
/// a new highlight. The clicked row becomes active, or the first highlighted one when its
/// highlight went off.
fn click_row(app: &mut VectorcraftApp, ctx: &egui::Context, n: &Node, sel: Sel, rows: Rows, k: usize, m: egui::Modifiers) {
    let h = highlight(ctx, n, sel);
    let (mut order, anchor) = if h.rows == rows { (h.order, h.anchor) } else { (vec![], None) };
    let click = widgets::row_click(&rows.order(n), &order, anchor, k, m);
    widgets::apply_row_click(&mut order, &click);
    let anchor = if click.moves_anchor() { Some(k) } else { anchor };
    let active = if order.contains(&k) { Some(k) } else { order.first().copied() };
    set_active(app, ctx, active.map_or(Sel::None, |a| rows.sel(a)));
    set_pstate(ctx, "ap-multi", Highlight { owner: Some(n.id), rows, order, anchor, active });
    // The panel has the keyboard now (Delete removes the highlighted rows).
    super::set_row_keys(ctx, ROW_KEYS, true);
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

/// Is `id` one of our own effects? A plug-in's effect name and parameters come from the plug-in
/// and are shown as they are.
fn built_in_effect(id: &str) -> bool {
    catalog().iter().any(|c| c.0 == id)
}

/// [`effect_label`] as shown: a built-in effect's in the UI language (an `@id` row for the
/// effect's id wins over the plain row of its English label), a plug-in's as it is.
pub(crate) fn shown_effect_label(id: &str) -> String {
    match catalog().iter().find(|c| c.0 == id) {
        Some(c) => crate::i18n::t_id(id, &c.1).to_string(),
        None => effect_label(id),
    }
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

fn row(ui: &mut Ui, selected: bool) -> (Rect, egui::Response) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click_and_drag());
    if selected {
        ui.painter().rect_filled(r, 0.0, t.row_selected);
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
    ui.painter().line_segment([pos2(r.left() + EYE_W, r.top()), pos2(r.left() + EYE_W, r.bottom())], Stroke::new(1.0, t.border));
    (r, resp)
}

fn eye(ui: &mut Ui, r: Rect, id: impl std::hash::Hash + std::fmt::Debug, on: bool, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let er = Rect::from_center_size(pos2(r.left() + EYE_W / 2.0, r.center().y), vec2(16.0, 16.0));
    let resp = ui.interact(er, ui.scope_id().with(id), if enabled { Sense::click() } else { Sense::hover() });
    let col = if !enabled {
        t.text_disabled
    } else if resp.hovered() {
        t.text_strong
    } else {
        t.icon
    };
    icons::paint(ui, if on { "eye" } else { "eye-off" }, er, col);
    enabled && resp.on_hover_text(tl!("Click to toggle visibility")).clicked()
}

fn chevron(ui: &mut Ui, r: Rect, id: impl std::hash::Hash + std::fmt::Debug, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let cr = Rect::from_center_size(pos2(r.left() + EYE_W + 12.0, r.center().y), vec2(12.0, 12.0));
    let resp = ui.interact(cr, ui.scope_id().with(id), Sense::click());
    icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, cr, if resp.hovered() { t.text_strong } else { t.icon });
    resp.clicked()
}

/// Text `s` at `pos` (left, vertically centred), strong or plain.
fn text(ui: &Ui, pos: egui::Pos2, s: &str, strong: bool) {
    let t = Tokens::get(ui.ctx());
    ui.painter().text(pos, egui::Align2::LEFT_CENTER, s, egui::FontId::proportional(12.5), if strong { t.text_strong } else { t.text });
}

/// Paint `p` as a swatch in `r`, with a black edge and a white frame; `mixed` (objects whose
/// fills differ) shows "?".
fn chip(ui: &Ui, r: Rect, p: &Paint, mixed: bool) {
    ui.painter().rect_filled(r.expand(1.0), 0.0, egui::Color32::BLACK);
    widgets::proxy_chip(ui, r, p, mixed, None);
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, egui::Color32::WHITE), StrokeKind::Inside);
}

/// A dotted-underline link drawn at `pos`; returns clicked.
/// A row's dotted-underlined label at `pos`, clickable whatever modifiers are held.
fn link(ui: &mut Ui, pos: egui::Pos2, id: impl std::hash::Hash + std::fmt::Debug, s: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(s.to_string(), egui::FontId::proportional(12.5), t.text_strong);
    let r = Rect::from_min_size(pos2(pos.x, pos.y - galley.size().y / 2.0), galley.size());
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    ui.painter().galley(r.min, galley, t.text_strong);
    let mut x = r.left();
    while x < r.right() {
        ui.painter().line_segment([pos2(x, r.bottom()), pos2((x + 1.0).min(r.right()), r.bottom())], Stroke::new(1.0, t.text_dim));
        x += 2.5;
    }
    resp
}

/// Where a row's popup opens: the height of the click that opened it (panel state).
const POPUP_AT: &str = "ap-popup-at";
/// The frame the row's popup was last drawn in (panel state).
const POPUP_FRAME: &str = "ap-popup-frame";
/// The row's popup's width, a floating panel's.
const POPUP_W: f32 = 260.0;
/// The room a row's popup keeps from the Appearance panel and the window's edges.
const POPUP_GAP: f32 = 6.0;

/// Open panel `id` as a row's popup ([`row_popup`]) at the height of the click that opened it; its
/// first text field takes the keyboard. A panel showing already (in the dock, a floating window or
/// an icon column's flyout) opens no popup.
pub(crate) fn open_popup(app: &mut VectorcraftApp, ctx: &egui::Context, id: &str) {
    if crate::panel_docking::showing(&app.ui, id) {
        app.ui.transient_panel = None;
        return;
    }
    app.ui.transient_panel = Some(id.into());
    app.ui.transient_focus = true;
    set_pstate(ctx, POPUP_AT, ctx.pointer_interact_pos().map(|p| p.y));
    // Drawn from the next frame on: this one has gone by already for it.
    set_pstate(ctx, POPUP_FRAME, Some(ctx.cumulative_frame_nr()));
}

/// A row's popup ([`open_popup`]): its panel beside the Appearance panel `panel` (left of it, else
/// right of it when there is no room), level with the row that opened it, so it shows which row it
/// belongs to. A press outside it, Escape, the Appearance panel going out of sight, or the panel
/// showing elsewhere puts it away.
pub(crate) fn row_popup(app: &mut VectorcraftApp, ctx: &egui::Context, panel: Rect) {
    let Some(id) = app.ui.transient_panel.clone() else { return };
    let close = |app: &mut VectorcraftApp| {
        app.ui.transient_panel = None;
        app.ui.transient_focus = false;
    };
    // Drawn in every frame since it opened: the Appearance panel stayed in sight.
    let frame = ctx.cumulative_frame_nr();
    let last: Option<u64> = pstate(ctx, POPUP_FRAME);
    if last.is_none_or(|f| f + 1 < frame) || crate::panel_docking::showing(&app.ui, &id) {
        return close(app);
    }
    set_pstate(ctx, POPUP_FRAME, Some(frame));
    let area_id = egui::Id::unique("ap-row-popup");
    let shown = ctx.memory(|m| m.area_rect(area_id));
    // A press outside it (not one that closes a dropdown the popup opened) or Escape puts it away.
    let pressed_outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| shown.is_some_and(|r| !r.contains(p))));
    if pressed_outside && !egui::Popup::is_any_open(ctx) || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        return close(app);
    }
    let screen = ctx.content_rect();
    let left = panel.left() - POPUP_GAP - POPUP_W;
    let x = if left >= screen.left() + POPUP_GAP { left } else { (panel.right() + POPUP_GAP).min(screen.right() - POPUP_W - POPUP_GAP) };
    let at: Option<f32> = pstate(ctx, POPUP_AT);
    let y = at.unwrap_or(panel.top()) - 14.0;
    let label = crate::state::all_panels().find(|(p, _)| *p == id).map_or(id.as_str(), |(_, l)| l).to_string();
    let tall = (screen.height() * 0.6).max(200.0);
    egui::Area::new(area_id).order(egui::Order::Foreground).fixed_pos(pos2(x, y)).constrain(true).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::ZERO).show(ui, |ui| {
            ui.set_width(POPUP_W);
            let t = Tokens::get(ui.ctx());
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.label(egui::RichText::new(tl!(&label)).strong().color(t.text_strong));
            });
            // Just opened: its first enabled text field takes the keyboard, as a dialog's does.
            let focus = app.ui.transient_focus;
            if focus {
                ui.data_mut(|m| m.insert_temp(widgets::dialog_focus_flag(), true));
            }
            crate::dock::panel_body(app, ui, &id, POPUP_W, tall);
            // Taken (the flag is gone): done. Still there (nothing could take it yet): next frame.
            if focus && ui.data_mut(|m| m.remove_temp::<bool>(widgets::dialog_focus_flag())).is_none() {
                app.ui.transient_focus = false;
            }
        });
    });
}

/// A row's name that edits the row when clicked ([`link`]), with the tooltip "Click to edit".
fn name_link(ui: &mut Ui, pos: egui::Pos2, id: impl std::hash::Hash + std::fmt::Debug, s: &str) -> egui::Response {
    link(ui, pos, id, s).on_hover_text(tl!("Click to edit"))
}

/// The tooltip of an effect row's empty space, "Double-click to edit", unless the pointer is on its
/// name (`name`), which has its own.
fn edit_tip(resp: egui::Response, name: &egui::Response) -> egui::Response {
    if name.hovered() { resp } else { resp.on_hover_text(tl!("Double-click to edit")) }
}

/// The Appearance panel: the object row, the stack of the selected object (or of new art with nothing selected) and the bottom bar; Delete/Backspace after a row click remove the highlighted rows.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let node = first_subject(app);
    let sel = current_sel(app, ui.ctx(), node.as_ref());
    let hide_thumb: bool = pstate(ui.ctx(), "ap-hide-thumb");
    widgets::list_box(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // Object row (clicking it targets the whole object again).
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW + 4.0), Sense::click());
        if resp.clicked() && sel != Sel::None {
            select_row(app, ui.ctx(), Sel::None);
        }
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
        let mix = mixed(app);
        let mixed = mix.is_some();
        let label = tl!(object_label(app));
        // A linked object names its graphic style ("Rectangle: Sunshine"), and so does new art
        // with nothing selected.
        let style = match app.session.selection_graphic_style() {
            Some((g, true)) if !mixed => Some(g.name.as_str()),
            _ if node.is_none() => app.session.new_art_transparency().2,
            _ => None,
        };
        let label = match style {
            Some(name) => std::borrow::Cow::Owned(format!("{label}: {name}")),
            None => std::borrow::Cow::Borrowed(label),
        };
        if !hide_thumb {
            let th = Rect::from_min_size(r.left_center() + vec2(6.0, -12.0), vec2(24.0, 24.0));
            // Objects whose fills differ show "?", like the Properties panel's Fill chip.
            let fill = match &node {
                Some(n) => n.appearance.fill_paint(),
                None => app.session.paint.fill.clone(),
            };
            chip(ui, th, &fill, mix.as_ref().is_some_and(|m| m.fills_differ));
            if let Some(n) = node.as_ref().filter(|_| !mixed) {
                thumbnail_drag(ui, th, n.id);
            }
        }
        text(ui, r.left_center() + vec2(if hide_thumb { 8.0 } else { 46.0 }, 0.0), &label, true);
        match &node {
            // Objects that differ list what they share, under a note that they differ.
            Some(n) if let Some(m) = &mix => {
                let (mr, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                let mut job = egui::text::LayoutJob::default();
                let format = egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: t.text_dim, italics: true, ..Default::default() };
                job.append(tl!("Mixed Appearances"), 0.0, format);
                let g = ui.painter().layout_job(job);
                ui.painter().galley(mr.left_center() + vec2(EYE_W + 24.0, -g.size().y / 2.0), g, t.text_dim);
                if !(m.shared.items.is_empty() && m.shared.effects.is_empty()) {
                    stack(app, ui, n, sel, r, Some(m));
                }
            }
            Some(n) => stack(app, ui, n, sel, r, None),
            None => default_stack(app, ui),
        }
        // Spacer row like Illustrator's empty tail.
        ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    });
    bottom(app, ui, node.as_ref(), sel);
    delete_key(app, ui);
    // A row's popup, beside the panel (in the same window when the panel floats).
    let panel = ui.max_rect();
    row_popup(app, ui.ctx(), panel);
}

/// The panel state that says the Appearance panel's rows have the keyboard ([`super::has_row_keys`]).
pub(crate) const ROW_KEYS: &str = "ap-keys";

/// Delete or Backspace removes the highlighted rows while the panel has the keyboard: a row was
/// clicked last, not somewhere outside the panel, the panel was drawn in every frame since, and no
/// text field has the focus (the Layers panel's rule). The app's shortcuts leave the keys to the
/// panel meanwhile ([`super::rows_take_delete`]); otherwise they delete the selected art.
fn delete_key(app: &mut VectorcraftApp, ui: &mut Ui) {
    let panel = ui.min_rect();
    let ctx = ui.ctx().clone();
    let pressed_outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !panel.contains(p)));
    let keys = super::has_row_keys(&ctx, ROW_KEYS) && !pressed_outside;
    super::set_row_keys(&ctx, ROW_KEYS, keys);
    let node = first_subject(app);
    let sel = current_sel(app, &ctx, node.as_ref());
    if keys
        && sel.editable()
        && ctx.memory(|m| m.focused().is_none())
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        delete_selected(app, ui, node.as_ref(), sel);
    }
}

/// The object row's thumbnail drags this object's appearance onto art ([`PanelDrag::Appearance`];
/// a chip of its fill follows the pointer).
fn thumbnail_drag(ui: &mut Ui, th: Rect, id: NodeId) {
    let resp = ui.interact(th, ui.scope_id().with("ap-thumb"), Sense::drag()).on_hover_text(tl!("Drag onto art to apply this appearance"));
    widgets::drag_source(ui, &resp, || PanelDrag::Appearance(id));
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
}

/// The object the panel lists: the targeted layer, group or object (`layer.target`), else the
/// first selected one.
fn first_subject(app: &VectorcraftApp) -> Option<Node> {
    let st = app.session.active()?;
    st.selection.subjects().first().and_then(|id| st.doc.node(*id)).cloned()
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

/// `params` of an item or effect command the panel runs, with `aligned` when the selected objects'
/// stacks differ: its indices are then rows of the first object's stack, which each other object
/// edits where it has them (the panel lists only the rows they share).
fn aligned(app: &VectorcraftApp, mut params: Value) -> Value {
    let differ = app.session.active().is_some_and(|st| {
        let mut aps = st.selection.subjects().iter().filter_map(|id| st.doc.node(*id)).map(|n| &n.appearance);
        aps.next().is_some_and(|first| aps.any(|a| a != first))
    });
    if differ && let Some(m) = params.as_object_mut() {
        m.insert("aligned".into(), json!(true));
    }
    params
}

/// What the next object drawn gets when nothing is selected (`Session::new_art`): its fills and
/// strokes, effects and transparency as rows. "Stroke:" opens the Stroke panel, which sets up its
/// stroke, and "Fill:" the Color panel on its fill; the swatches and the weight field work as an
/// object's ([`swatch_button`], [`weight_field`]), on its top fill and stroke.
fn default_stack(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ap = app.session.new_art();
    let (opacity, blend, _) = app.session.new_art_transparency();
    let fx = |ui: &mut Ui, e: &Effect, id: (Option<usize>, usize)| {
        let (r, _) = row(ui, false);
        eye(ui, r, ("ap-def-fx", id), e.visible, false);
        text(ui, pos2(r.left() + EYE_W + 24.0 + if id.0.is_some() { 12.0 } else { 0.0 }, r.center().y), &shown_effect_label(&e.id), false);
        icons::paint(ui, "dc-fx", Rect::from_center_size(r.right_center() - vec2(14.0, 0.0), vec2(16.0, 16.0)), t.icon);
    };
    for (i, it) in ap.items.iter().enumerate().rev() {
        let (r, _) = row(ui, false);
        eye(ui, r, ("ap-def-eye", i), it.visible(), false);
        let lx = r.left() + EYE_W + 24.0;
        let label = if it.is_fill() { tl!("Fill:") } else { tl!("Stroke:") };
        if name_link(ui, pos2(lx, r.center().y), ("ap-def-link", i), label).clicked() {
            // The Color panel then edits this one of the next object's paints.
            app.run("paint.toggleActive", json!({ "fill": it.is_fill() })).ok();
            open_popup(app, ui.ctx(), if it.is_fill() { "color" } else { "stroke" });
        }
        // A graphic style can give the next object two fills or strokes with the same target: a
        // child Ui salted by the row keeps their widget ids apart (and, unlike `push_id`, takes no
        // layout space).
        let mut cells = ui.new_child(egui::UiBuilder::new().id_salt(("ap-def-row", i)).max_rect(r));
        swatch_button(app, &mut cells, r, it.paint(), Painted::NewArt { fill: it.is_fill() });
        if let AppearanceItem::Stroke(st) = it {
            weight_field(app, &mut cells, r, st, Painted::NewArt { fill: false });
        }
        for (k, e) in it.effects().iter().enumerate() {
            fx(ui, e, (Some(i), k));
        }
    }
    for (k, e) in ap.effects.iter().enumerate() {
        fx(ui, e, (None, k));
    }
    let (r, _) = row(ui, false);
    eye(ui, r, "ap-def-op", true, false);
    text(
        ui,
        r.left_center() + vec2(EYE_W + 24.0, 0.0),
        &crate::i18n::fmt(tl!("Opacity: {value}"), &[("value", &opacity_text(opacity, blend))]),
        false,
    );
}

/// A stack row being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dragged {
    Item(usize),
    /// The Contents (Characters) row.
    Contents,
    /// Effect `k` of fill/stroke `item` (`None`: of the object).
    Effect(Option<usize>, usize),
}

/// What a stack row stands for when a dragged row is dropped on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// Fill/stroke row `index` (`sub`: one of its indented sub-rows).
    Item { index: usize, sub: bool },
    /// Effect `k` of fill/stroke `item` (`None`: of the object).
    Effect(Option<usize>, usize),
    /// The object row (`top`) or the object's Opacity row.
    Object { top: bool },
    /// The Contents (Characters) row.
    Contents,
}

/// The rows of object `n` (fills/strokes with their Opacity and effect rows, the Contents row, the object's effects and Opacity) with the click rules, row drags and drop markers; `sel` is the active row.
/// `only`: a mixed selection (`n` its first object): only the rows its objects share are listed,
/// and rows aren't dragged (where one would go in each of the different stacks isn't one place).
fn stack(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, sel: Sel, object_row: Rect, only: Option<&Mixed>) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let dragging: Option<Dragged> = pstate::<Option<Dragged>>(&ctx, "ap-drag").filter(|_| only.is_none());
    let items = selected_items(&ctx, Some(n), sel);
    let effects = selected_effects(&ctx, Some(n), sel);
    let lit = |item: Option<usize>, k: usize| effects.as_ref().is_some_and(|(list, ks)| *list == item && ks.contains(&k));
    let mut rows = vec![(Slot::Object { top: true }, object_row)];
    let mut stopped = false;
    // Starts or ends a row drag.
    let frozen = only.is_some();
    let mut track = |resp: &egui::Response, what: Dragged| {
        if frozen {
            return;
        }
        if resp.drag_started() {
            set_pstate(&ctx, "ap-drag", Some(what));
            // Where the row was grabbed, for its dimmed copy under the pointer.
            // (From where the button went down: the drag starts once the pointer has moved on.)
            let grab = ctx.input(|i| i.pointer.press_origin()).map_or(vec2(EYE_W, ROW / 2.0), |p| p - resp.rect.min);
            set_pstate(&ctx, "ap-grab", (grab, resp.rect.width()));
        }
        stopped |= resp.drag_stopped();
    };
    // A mixed selection's Contents rows differ object by object: none is listed.
    let contents = n.contents_label().filter(|_| only.is_none());
    let below = n.appearance.contents_at();
    for (i, it) in n.appearance.items.iter().enumerate().rev() {
        if only.is_some_and(|m| !m.shared.has_item(i)) {
            continue;
        }
        // The Contents row sits right above the items painted below the members.
        if let Some(label) = contents.filter(|_| i + 1 == below) {
            let (r, resp) = contents_row(app, ui, n, label, sel);
            rows.push((Slot::Contents, r));
            track(&resp, Dragged::Contents);
        }
        let open: bool = pstate(ui.ctx(), &format!("ap-open-{i}"));
        let (r, resp) = row(ui, items.contains(&i));
        rows.push((Slot::Item { index: i, sub: false }, r));
        let (visible, paint) = (it.visible(), it.paint());
        let is_stroke = !it.is_fill();
        if eye(ui, r, ("ap-eye", i), visible, true) {
            app.run("appearance.setItem", aligned(app, json!({"index": i, "visible": !visible}))).ok();
        }
        if chevron(ui, r, ("ap-chev", i), open) {
            set_pstate(ui.ctx(), &format!("ap-open-{i}"), !open);
        }
        let lx = r.left() + EYE_W + 24.0;
        // The label edits the row (the Stroke or Color panel); the empty space only takes the click
        // rules.
        let label = if is_stroke { tl!("Stroke:") } else { tl!("Fill:") };
        if name_link(ui, pos2(lx, r.center().y), ("ap-link", i), label).clicked() {
            select_row(app, ui.ctx(), Sel::Item(i));
            // The panel has the keyboard (Delete removes this row, not the art).
            super::set_row_keys(ui.ctx(), ROW_KEYS, true);
            open_popup(app, ui.ctx(), if is_stroke { "stroke" } else { "color" });
        }
        swatch_button(app, ui, r, paint, Painted::Item(i));
        if let AppearanceItem::Stroke(st) = it {
            weight_field(app, ui, r, st, Painted::Item(i));
        }
        if resp.clicked() {
            let m = ui.input(|inp| inp.modifiers);
            click_row(app, ui.ctx(), n, sel, Rows::Items, i, m);
        }
        track(&resp, Dragged::Item(i));
        if open {
            let r = opacity_row(app, ui, Some(i), it.opacity(), it.blend(), sel, false);
            rows.push((Slot::Item { index: i, sub: true }, r));
            for (k, e) in it.effects().iter().enumerate() {
                let (r, resp) = effect_row(app, ui, Some(i), k, e, lit(Some(i), k));
                if resp.clicked() {
                    let m = ui.input(|inp| inp.modifiers);
                    click_row(app, ui.ctx(), n, sel, Rows::Effects(Some(i)), k, m);
                }
                rows.push((Slot::Effect(Some(i), k), r));
                track(&resp, Dragged::Effect(Some(i), k));
            }
        }
    }
    if let Some(label) = contents.filter(|_| below == 0) {
        let (r, resp) = contents_row(app, ui, n, label, sel);
        rows.push((Slot::Contents, r));
        track(&resp, Dragged::Contents);
    }
    // Object-level effects.
    for (k, e) in n.appearance.effects.iter().enumerate() {
        if only.is_some_and(|m| !m.shared.has_effect(k)) {
            continue;
        }
        let (r, resp) = effect_row(app, ui, None, k, e, lit(None, k));
        if resp.clicked() {
            let m = ui.input(|inp| inp.modifiers);
            click_row(app, ui.ctx(), n, sel, Rows::Effects(None), k, m);
        }
        rows.push((Slot::Effect(None, k), r));
        track(&resp, Dragged::Effect(None, k));
    }
    let r = opacity_row(app, ui, None, n.opacity, n.blend, sel, only.is_some_and(|m| m.transparency));
    rows.push((Slot::Object { top: false }, r));
    let Some(what) = dragging else { return };
    let pointer = ui.ctx().pointer_interact_pos();
    if let Some(p) = pointer
        && let Some((slot, r)) = slot_at(&rows, p)
    {
        // Drop marker: a line where the row lands, or a frame round the row it goes into.
        let stroke = Stroke::new(2.0, t.accent);
        let line = |y: f32| {
            ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], stroke);
        };
        match (what, slot) {
            (Dragged::Effect(..), Slot::Effect(..)) => line(if p.y < r.center().y { r.top() } else { r.bottom() }),
            (Dragged::Effect(..), _) => {
                ui.painter().rect_stroke(r.shrink(1.0), 0.0, stroke, StrokeKind::Inside);
            }
            // Onto the Contents row: an item from above lands just below it, one from below just above.
            (Dragged::Item(from), Slot::Contents) => line(if from >= below { r.bottom() } else { r.top() }),
            (Dragged::Item(from), _) => line(if item_at(n, slot) > from { r.top() } else { r.bottom() }),
            (Dragged::Contents, _) => line(if contents_slot(n, slot).is_some_and(|k| k > below) { r.top() } else { r.bottom() }),
        }
    }
    // The drag ends in `bottom`, which takes drops on its Duplicate and Delete buttons.
    // Several highlighted rows, one of them grabbed, drop together.
    let set = match what {
        Dragged::Item(i) if items.contains(&i) => items.clone(),
        Dragged::Effect(item, k) => effects.as_ref().filter(|(list, ks)| *list == item && ks.contains(&k)).map_or(vec![], |(_, ks)| ks.clone()),
        _ => vec![],
    };
    if ui.ctx().input(|i| i.pointer.any_down())
        && let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos())
    {
        let order = if set.len() > 1 { highlight(&ctx, n, sel).order } else { vec![] };
        drag_ghost(app, ui, n, what, &order, pos, object_row.width());
    }
    if stopped
        && set.len() > 1
        && let Some(p) = pointer
    {
        block_drop(app, ui.ctx(), n, sel, &rows, what, &set, p);
        return;
    }
    if stopped
        && let Some(p) = pointer
        && let Some((cmd, params)) = drop_command(n, &rows, what, p, ui.input(|i| i.modifiers.alt))
        && let Ok(r) = app.run(cmd, params)
        && let Some(k) = r["index"].as_u64().filter(|_| cmd == "effect.move")
    {
        // The dropped effect becomes the selected row (a moved fill/stroke stays active in the engine).
        let k = k as usize;
        select_row(app, ui.ctx(), r["item"].as_u64().map_or(Sel::Effect(k), |i| Sel::ItemEffect(i as usize, k)));
    }
}

/// Drop the highlighted rows `set` (ascending; `what` grabbed) at `p`: they move (Alt: are copied)
/// as one block ([`block_drop_command`]) and the moved rows (the copies) stay highlighted, the
/// active row with its row.
#[allow(clippy::too_many_arguments)]
fn block_drop(app: &mut VectorcraftApp, ctx: &egui::Context, n: &Node, sel: Sel, rows: &[(Slot, Rect)], what: Dragged, set: &[usize], p: egui::Pos2) {
    let copy = ctx.input(|i| i.modifiers.alt);
    let Some((cmd, params)) = block_drop_command(n, rows, what, set, p, copy) else { return };
    let Ok(r) = app.run(cmd, params.clone()) else { return };
    let h = highlight(ctx, n, sel);
    // Where the lowest of them landed, and in which kind of rows.
    let (landed, kind) = match what {
        Dragged::Effect(..) => (r["index"].as_u64().map(|k| k as usize), Rows::Effects(r["item"].as_u64().map(|i| i as usize))),
        _ if copy => (params["to"].as_u64().map(|t| (t as usize).min(n.appearance.items.len())), Rows::Items),
        _ => (r["index"].as_u64().map(|k| k as usize), Rows::Items),
    };
    let Some(landed) = landed else { return };
    let moved = |i: usize| set.iter().position(|s| *s == i).map(|k| landed + k);
    let order: Vec<usize> = h.order.iter().filter_map(|i| moved(*i)).collect();
    let active = sel.rows().and_then(|(_, a)| moved(a)).or(order.first().copied());
    set_active(app, ctx, active.map_or(Sel::None, |a| kind.sel(a)));
    let anchor = h.anchor.and_then(moved).or(active);
    set_pstate(ctx, "ap-multi", Highlight { owner: Some(n.id), rows: kind, order, anchor, active });
}

/// The dimmed copy of the dragged rows under the pointer at `pos` ([`widgets::drag_ghost`]): the
/// row [`widgets::ghost_row`] picks from the dragged rows `order` (highlight order; empty: only the
/// grabbed one), with its label, colour swatch and stroke weight, or its effect's name.
fn drag_ghost(app: &VectorcraftApp, ui: &Ui, n: &Node, what: Dragged, order: &[usize], pos: egui::Pos2, width: f32) {
    let (grab, w): (egui::Vec2, f32) = pstate(ui.ctx(), "ap-grab");
    let r = Rect::from_min_size(pos - grab, vec2(if w > 0.0 { w } else { width }, ROW));
    let shown = match what {
        Dragged::Item(i) => Dragged::Item(widgets::ghost_row(order, i)),
        Dragged::Effect(item, k) => Dragged::Effect(item, widgets::ghost_row(order, k)),
        Dragged::Contents => Dragged::Contents,
    };
    let weight = match shown {
        Dragged::Item(i) => match n.appearance.items.get(i) {
            Some(AppearanceItem::Stroke(st)) => Some(app.session.stroke_unit().format(st.width)),
            _ => None,
        },
        _ => None,
    };
    widgets::drag_ghost(ui.ctx(), ui.scope_id().with("ap-ghost"), r, |ui, r| {
        let y = r.center().y;
        match shown {
            Dragged::Item(i) => {
                let Some(it) = n.appearance.items.get(i) else { return };
                text(ui, pos2(r.left() + EYE_W + 24.0, y), if it.is_fill() { tl!("Fill:") } else { tl!("Stroke:") }, false);
                chip(ui, Rect::from_min_size(pos2(r.left() + EYE_W + 76.0, y - 9.0), vec2(18.0, 18.0)), it.paint(), false);
                if let Some(w) = &weight {
                    text(ui, pos2(r.left() + EYE_W + 104.0, y), w, false);
                }
            }
            Dragged::Effect(item, k) => {
                let Some(e) = n.appearance.effects_at(item).and_then(|fx| fx.get(k)) else { return };
                let indent = if item.is_some() { 12.0 } else { 0.0 };
                text(ui, pos2(r.left() + indent + EYE_W + 24.0, y), &shown_effect_label(&e.id), false);
            }
            Dragged::Contents => {
                if let Some(label) = n.contents_label() {
                    text(ui, pos2(r.left() + EYE_W + 24.0, y), tl!(label), false);
                }
            }
        }
    });
}

/// The row under `p`.
fn slot_at(rows: &[(Slot, Rect)], p: egui::Pos2) -> Option<(Slot, Rect)> {
    rows.iter().find(|(_, r)| r.contains(p)).copied()
}

/// Paint-order index a dragged fill/stroke lands on when dropped on row `slot`: that row's item;
/// the object row stands above the topmost item, the rows below the items under item 0, the
/// Contents row for the first item above it.
fn item_at(n: &Node, slot: Slot) -> usize {
    match slot {
        Slot::Item { index, .. } | Slot::Effect(Some(index), _) => index,
        Slot::Object { top: true } => n.appearance.items.len().saturating_sub(1),
        Slot::Object { top: false } | Slot::Effect(None, _) => 0,
        Slot::Contents => n.appearance.contents_at(),
    }
}

/// How many items paint below the Contents row after it is dropped on row `slot`: it goes above a
/// fill or stroke above it and below one below it; the object row is above everything, the rows
/// under the stack below everything. `None` on itself.
fn contents_slot(n: &Node, slot: Slot) -> Option<usize> {
    let k = n.appearance.contents_at();
    match slot {
        Slot::Item { index, .. } | Slot::Effect(Some(index), _) => Some(if index >= k { index + 1 } else { index }),
        Slot::Object { top: true } => Some(n.appearance.items.len()),
        Slot::Object { top: false } | Slot::Effect(None, _) => Some(0),
        Slot::Contents => None,
    }
}

/// The command a drop of `what` at `p` runs (`copy`: Alt held), or None when it would change
/// nothing or lands outside the stack.
fn drop_command(n: &Node, rows: &[(Slot, Rect)], what: Dragged, p: egui::Pos2, copy: bool) -> Option<(&'static str, Value)> {
    let (slot, r) = slot_at(rows, p)?;
    match what {
        Dragged::Item(from) => {
            let to = item_at(n, slot);
            if copy {
                // The copy goes where the row was dropped: above that row when dragged up, below it
                // when dragged down.
                let at = if to >= from { to + 1 } else { to };
                return Some(("appearance.duplicateItem", json!({"index": from, "to": at})));
            }
            // The Contents row ends up below an item dropped on the object row, above one dropped
            // under the stack, and on the far side of one dropped on it (elsewhere it stays put).
            let k = n.appearance.contents_at();
            let others = k - usize::from(from < k);
            let contents = match slot {
                Slot::Contents if from < k => Some(others),
                Slot::Contents | Slot::Object { top: false } | Slot::Effect(None, _) => Some(others + 1),
                Slot::Object { top: true } => Some(others),
                _ => None,
            }
            .filter(|_| n.contents_label().is_some());
            let to = if slot == Slot::Contents { others } else { to };
            if to == from && contents.is_none_or(|c| c == k) {
                return None;
            }
            let mut params = json!({"from": from, "to": to});
            if let Some(c) = contents {
                params["contents"] = json!(c);
            }
            Some(("appearance.moveItem", params))
        }
        Dragged::Contents => {
            let to = contents_slot(n, slot)?;
            (to != n.appearance.contents_at()).then(|| ("appearance.moveItem", json!({"from": "contents", "to": to})))
        }
        Dragged::Effect(from_item, k) => {
            let (item, pos) = match slot {
                Slot::Effect(item, k2) => (item, k2 + usize::from(p.y > r.center().y)),
                Slot::Item { index, .. } => (Some(index), n.appearance.effects_at(Some(index))?.len()),
                Slot::Object { .. } | Slot::Contents => (None, n.appearance.effects.len()),
            };
            // Within its own list the effect leaves its old place first.
            let same = item == from_item && !copy;
            let pos = if same && pos > k { pos - 1 } else { pos };
            if same && pos == k {
                return None;
            }
            Some(("effect.move", json!({"from": k, "fromItem": from_item, "to": pos, "toItem": item, "copy": copy})))
        }
    }
}

/// The command a drop of several highlighted rows `set` (ascending; `what` is the row grabbed, one
/// of them) at `p` runs (`copy`: Alt held), or None on one of their own rows or outside the stack.
/// Fill/strokes land as one block in their order: next to the row dropped on and on its side of the
/// Contents row; on the Contents row on the far side from the grabbed row; on the object row on
/// top, under the stack at the bottom. Effects land together in the list dropped on.
fn block_drop_command(n: &Node, rows: &[(Slot, Rect)], what: Dragged, set: &[usize], p: egui::Pos2, copy: bool) -> Option<(&'static str, Value)> {
    let (slot, r) = slot_at(rows, p)?;
    let m = set.len();
    match what {
        Dragged::Item(grabbed) => {
            let len = n.appearance.items.len();
            let k = n.appearance.contents_at();
            let unmoved_below = (0..k).filter(|i| !set.contains(i)).count();
            // (lowest copy's index among all items, items painting below Contents afterwards)
            let place = |t: usize, above: bool| {
                if copy {
                    (if above { t + 1 } else { t }, if t < k { k + m } else { k })
                } else {
                    let rank = (0..t).filter(|i| !set.contains(i)).count();
                    (if above { rank + 1 } else { rank }, if t < k { unmoved_below + m } else { unmoved_below })
                }
            };
            let (to, contents) = match slot {
                Slot::Item { index, .. } | Slot::Effect(Some(index), _) if set.contains(&index) => return None,
                Slot::Item { index, .. } | Slot::Effect(Some(index), _) => place(index, index > grabbed),
                // From above the Contents row the block lands just below it, from below just above.
                Slot::Contents => {
                    let below = if copy { k } else { unmoved_below };
                    (below, if grabbed >= k { below + m } else { below })
                }
                Slot::Object { top: true } => (if copy { len } else { len.saturating_sub(m) }, if copy { k } else { unmoved_below }),
                Slot::Object { top: false } | Slot::Effect(None, _) => (0, if copy { k + m } else { unmoved_below + m }),
            };
            let mut params = json!({"indices": set, "to": to});
            if n.contents_label().is_some() {
                params["contents"] = json!(contents);
            }
            Some((if copy { "appearance.duplicateItem" } else { "appearance.moveItem" }, params))
        }
        Dragged::Effect(from_item, _) => {
            let (item, pos) = match slot {
                Slot::Effect(item, k2) if item == from_item && set.contains(&k2) => return None,
                Slot::Effect(item, k2) => (item, k2 + usize::from(p.y > r.center().y)),
                Slot::Item { index, .. } => (Some(index), n.appearance.effects_at(Some(index))?.len()),
                Slot::Object { .. } | Slot::Contents => (None, n.appearance.effects.len()),
            };
            // Within their own list the effects leave their places first.
            let same = item == from_item && !copy;
            let pos = if same { pos - set.iter().filter(|s| **s < pos).count() } else { pos };
            let contiguous = set.windows(2).all(|w| w.get(1).copied() == w.first().map(|a| a + 1));
            if same && contiguous && set.first() == Some(&pos) {
                return None;
            }
            Some(("effect.move", json!({"from": set, "fromItem": from_item, "to": pos, "toItem": item, "copy": copy})))
        }
        Dragged::Contents => None,
    }
}

/// The Contents (type: Characters) row of `n`: clicking it sends paint edits to the members
/// (characters); double-clicking Contents targets the members, Characters opens the Color panel
/// on the characters' paint (whose chips it shows), and its name the Character panel. Returns its
/// rect and response (for dragging).
fn contents_row(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, label: &str, sel: Sel) -> (Rect, egui::Response) {
    let (r, resp) = row(ui, sel == Sel::Contents);
    eye(ui, r, "ap-contents-eye", true, false);
    let characters = matches!(n.kind, vectorcraft_doc::NodeKind::Text(_));
    let at = pos2(r.left() + EYE_W + 24.0, r.center().y);
    let name = if characters {
        let name = name_link(ui, at, "ap-characters-link", tl!(label));
        if name.clicked() {
            select_row(app, ui.ctx(), Sel::Contents);
            super::set_row_keys(ui.ctx(), ROW_KEYS, true);
            open_popup(app, ui.ctx(), "character");
        }
        Some(name)
    } else {
        text(ui, at, tl!(label), false);
        None
    };
    if characters {
        for (k, stroke) in [false, true].into_iter().enumerate() {
            let cr = Rect::from_min_size(pos2(r.left() + EYE_W + 100.0 + 24.0 * k as f32, r.center().y - 9.0), vec2(18.0, 18.0));
            chip(ui, cr, n.proxy_paint(stroke, None).map_or(&Paint::None, |(p, ..)| p), false);
        }
    }
    // An image's row has no members to target.
    let image = matches!(n.kind, vectorcraft_doc::NodeKind::Image(_));
    let resp = if image || name.is_some_and(|n| n.hovered()) {
        resp
    } else {
        resp.on_hover_text(if characters { tl!("Double-click to edit the characters' paint") } else { tl!("Double-click to target the contents") })
    };
    if resp.clicked() || resp.double_clicked() {
        select_row(app, ui.ctx(), Sel::Contents);
        // The panel has the keyboard (Delete then removes nothing, rather than the object).
        super::set_row_keys(ui.ctx(), ROW_KEYS, true);
    }
    if resp.double_clicked() {
        if characters {
            open_popup(app, ui.ctx(), "color");
        } else if !image {
            app.run("appearance.targetContents", json!({})).ok();
        }
    }
    (r, resp)
}

/// A stroke row's notes after the weight: its brush, "Dashed" and its width profile (drawn).
fn stroke_notes(ui: &Ui, r: Rect, st: &vectorcraft_doc::StrokeLayer) {
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter_at(r);
    let dashed = st.dash.as_ref().is_some_and(|d| d.is_dashed()).then_some(tl!("Dashed"));
    let notes: Vec<&str> = st.brush.as_deref().into_iter().chain(dashed).collect();
    let mut x = r.left();
    if !notes.is_empty() {
        let g = painter.layout_no_wrap(notes.join(", "), egui::FontId::proportional(11.5), t.text_dim);
        x += g.size().x + 6.0;
        painter.galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, t.text_dim);
    }
    if let Some(p) = st.profile.as_ref().filter(|p| p.preset_id() != Some("uniform")) {
        let pr = Rect::from_min_size(pos2(x, r.center().y - 5.0), vec2(28.0, 10.0));
        if pr.right() <= r.right() {
            paint_profile(ui, pr, Some(&p.points), t.text);
        }
    }
}

/// An Opacity row: of the object (`item: None`) or, indented, of fill/stroke `item`. A click
/// highlights the row itself (any modifiers: it is never one of several); only "Opacity:" edits
/// it: it opens the Transparency panel on it (the engine's active item is the row's fill/stroke,
/// whose opacity and blend the panel edits). Returns the row's rect.
///
/// `mixed`: the selected objects' opacity or blend mode differ: it reads "Mixed".
fn opacity_row(app: &mut VectorcraftApp, ui: &mut Ui, item: Option<usize>, opacity: f32, blend: BlendMode, sel: Sel, mixed: bool) -> Rect {
    let this = item.map_or(Sel::Opacity, Sel::ItemOpacity);
    let (r, resp) = row(ui, sel == this);
    eye(ui, r, ("ap-op-eye", item), true, false);
    let lx = r.left() + EYE_W + 24.0 + if item.is_some() { 12.0 } else { 0.0 };
    let link = name_link(ui, pos2(lx, r.center().y), ("ap-op-link", item), tl!("Opacity:")).clicked();
    if link || resp.clicked() {
        select_row(app, ui.ctx(), this);
        // The panel has the keyboard (Delete then removes nothing, rather than the object).
        super::set_row_keys(ui.ctx(), ROW_KEYS, true);
    }
    if link {
        open_popup(app, ui.ctx(), "transparency");
    }
    let shown = if mixed { tl!("Mixed").to_string() } else { opacity_text(opacity, blend) };
    text(ui, pos2(lx + 56.0, r.center().y), &shown, false);
    r
}

/// An effect row: of the object (`item: None`) or, indented under it, of fill/stroke `item`.
/// Clicking the name opens the effect's dialog to edit it (the disclosure: an inline editor).
/// Returns the row's rect and response (for dragging).
fn effect_row(app: &mut VectorcraftApp, ui: &mut Ui, item: Option<usize>, k: usize, e: &Effect, selected: bool) -> (Rect, egui::Response) {
    let t = Tokens::get(ui.ctx());
    let open_key = format!("ap-fx-open-{item:?}-{k}");
    let open: bool = pstate(ui.ctx(), &open_key);
    let this = item.map_or(Sel::Effect(k), |i| Sel::ItemEffect(i, k));
    let (row_rect, resp) = row(ui, selected);
    let r = row_rect;
    if eye(ui, r, ("ap-fx-eye", item, k), e.visible, true) {
        app.run("effect.setParams", aligned(app, json!({"item": item, "index": k, "visible": !e.visible}))).ok();
    }
    let indent = if item.is_some() { 12.0 } else { 0.0 };
    let r = Rect::from_min_max(pos2(r.left() + indent, r.top()), r.max);
    if chevron(ui, r, ("ap-fx-chev", item, k), open) {
        set_pstate(ui.ctx(), &open_key, !open);
    }
    let lx = r.left() + EYE_W + 24.0;
    // The name edits the effect on a click; the empty space takes the click rules (in `stack`)
    // and edits it on a double-click: its settings dialog, or the inline editor of an effect
    // without settings.
    let name = name_link(ui, pos2(lx, r.center().y), ("ap-fx-link", item, k), &shown_effect_label(&e.id));
    let resp = edit_tip(resp, &name);
    let edit = name.clicked() || resp.double_clicked();
    if edit {
        select_row(app, ui.ctx(), this);
        super::set_row_keys(ui.ctx(), ROW_KEYS, true);
        if has_options(&e.id) {
            app.run("effect.dialog", aligned(app, json!({"effect": e.id, "index": k, "item": item}))).ok();
        } else {
            set_pstate(ui.ctx(), &open_key, !open);
        }
    }
    icons::paint(ui, "dc-fx", Rect::from_center_size(r.right_center() - vec2(14.0, 0.0), vec2(16.0, 16.0)), t.icon);
    if open {
        effect_editor(app, ui, item, k, e);
    }
    (row_rect, resp)
}

/// Does the effect have a dialog (any parameters)?
fn has_options(id: &str) -> bool {
    vectorcraft_effects::default_params(id).is_some_and(|d| d.as_object().is_some_and(|o| !o.is_empty()))
}

/// Inline editor for an applied effect's parameters, labeled as the effect dialogs' generic fields
/// are: numbers, booleans, strings, colors, the values a parameter picks from as a dropdown
/// ([`crate::dialogs::shown_choices`]), and Drop Shadow's Color or Darkness as radio buttons;
/// edits go through `effect.setParams`.
fn effect_editor(app: &mut VectorcraftApp, ui: &mut Ui, item: Option<usize>, k: usize, e: &Effect) {
    let t = Tokens::get(ui.ctx());
    let params = editor_params(e);
    let mut change: Option<(String, Value)> = None;
    let mut edit = false;
    let built_in = built_in_effect(&e.id);
    // A plug-in effect's parameters are labelled, measured and shown as its manifest says.
    let plugin = vectorcraft_plugins::effect::installed(&e.id);
    let manifest = plugin.as_ref().map(|p| p.manifest());
    let visible = manifest.map(|m| m.visible(&params));
    let (unit, relative) = (app.session.general_unit(), params.get("relative").and_then(Value::as_bool).unwrap_or(false));
    let cmyk = app.session.active().is_some_and(|s| s.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk);
    egui::Frame::NONE.fill(t.panel_darker).inner_margin(egui::Margin { left: (EYE_W + 12.0) as i8, right: 6, top: 4, bottom: 4 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        if params.is_empty() {
            widgets::dim_label(ui, tl!("No options"));
        }
        for (key, v) in &params {
            if visible.as_ref().is_some_and(|v| !v.contains(key.as_str())) {
                continue;
            }
            let ui_keys = manifest.and_then(|m| m.param(key)).map(|p| &p.ui);
            ui.horizontal(|ui| {
                let label = ui_keys.and_then(|u| u.label.clone()).unwrap_or_else(|| param_label(key, built_in));
                let label_node =
                    ui.add_sized(vec2(84.0, 22.0), egui::Label::new(egui::RichText::new(label.as_str()).size(11.5).color(t.text)).truncate());
                let label_node = if let Some(help) = ui_keys.and_then(|u| u.help.as_deref()) { label_node.on_hover_text(help) } else { label_node };
                // The row's actual label node names its fields, including a plug-in's plain text
                // and number fields (egui 0.37's strict a11y gate).
                ui.label_inputs_by(label_node.id, |ui| {
                    if e.id == "stylize.dropShadow" && key == "shade" {
                        let cur = v.as_str().unwrap_or_default();
                        for (label, value) in crate::dialogs::SHADES {
                            if widgets::radio(ui, label, cur.eq_ignore_ascii_case(value), true) && !cur.eq_ignore_ascii_case(value) {
                                change = Some((key.clone(), json!(value)));
                            }
                        }
                        return;
                    }
                    if let Some(options) = crate::dialogs::shown_choices(&e.id, key, &params, cmyk) {
                        let cur = v.as_str().unwrap_or_default();
                        let shown = options.iter().find(|(_, value)| value == cur).map_or(cur, |(label, _)| label.as_str());
                        let labels: Vec<&str> = options.iter().map(|(label, _)| label.as_str()).collect();
                        if let Some((_, value)) = widgets::dropdown_names(ui, ("fx-choice", item, k, key.as_str()), shown, &labels, 90.0)
                            .and_then(|i| options.get(i))
                            .filter(|(_, value)| value != cur)
                        {
                            change = Some((key.clone(), json!(value)));
                        }
                        return;
                    }
                    // A plug-in's number list is typed as its dialog's is: "45, 135".
                    if let Some(spec) = manifest.and_then(|m| m.param(key)).map(|p| &p.spec)
                        && matches!(spec, vectorcraft_plugins::manifest::ParamSpec::NumberList { .. })
                    {
                        let typed = widgets::exact_text_field(ui, ("fx", item, k, key.as_str()), &crate::dialogs::number_list_text(v), 90.0);
                        if let Some(list) = typed.and_then(|text| crate::dialogs::read_number_list(&text, spec)) {
                            change = Some((key.clone(), list));
                        }
                        if let Some(unit) = ui_keys.and_then(|u| u.unit.as_deref()) {
                            widgets::dim_label(ui, unit);
                        }
                        return;
                    }
                    // Only our own effects show a `mode` parameter as the blend mode dropdown; a plug-in's `mode` stays a plain field.
                    if built_in && let Some(cur) = widgets::blend_param(key, v) {
                        if let Some(m) = widgets::blend_param_dropdown(ui, ("fx-blend", item, k, key.as_str()), cur) {
                            change = Some((key.clone(), m));
                        }
                        return;
                    }
                    match v {
                        Value::Bool(b) => {
                            if widgets::check_box(ui, &label, *b, true) {
                                change = Some((key.clone(), json!(!b)));
                            }
                        }
                        Value::Number(n) => {
                            let (x, id) = (n.as_f64().unwrap_or(0.0), ("fx", item, k, key.as_str()));
                            let suffix = ui_keys.and_then(|u| u.unit.as_deref()).map(crate::dialogs::unit_suffix).unwrap_or_default();
                            let nx = if vectorcraft_effects::is_length(&e.id, key, relative) {
                                widgets::num_field(ui, id, Some(x), unit, 70.0)
                            } else {
                                widgets::plain_field(ui, id, x, &suffix, 2, 70.0)
                            };
                            if let Some(nx) = nx {
                                change = Some((key.clone(), json!(nx)));
                            }
                        }
                        // Glass's loaded texture shows its size; its dialog's Load Texture… replaces it.
                        Value::String(s) if key == vectorcraft_effects::pixel::TEXTURE_IMAGE => {
                            let size = vectorcraft_effects::pixel::texture(s).map(|t| t.size());
                            widgets::dim_label(ui, &size.map_or_else(String::new, |(w, h)| format!("{w} × {h} px")));
                        }
                        Value::String(s) => {
                            if let Some(typed) = widgets::exact_text_field(ui, ("fx", item, k, key.as_str()), s, 90.0) {
                                change = Some((key.clone(), json!(typed)));
                            }
                        }
                        // Free Distort's corners are dragged in its dialog.
                        _ if e.id == crate::dialogs::free_distort::FREE_DISTORT => {
                            edit |= widgets::flat_button(ui, tl!("Edit…"), 48.0).clicked();
                        }
                        other => {
                            widgets::dim_label(ui, &other.to_string());
                        }
                    }
                });
            });
        }
    });
    if let Some((key, v)) = change {
        app.run("effect.setParams", aligned(app, json!({"item": item, "index": k, "params": {key: v}}))).ok();
    }
    if edit {
        app.run("effect.dialog", aligned(app, json!({"effect": e.id, "index": k, "item": item}))).ok();
    }
}

/// The values [`effect_editor`] lists: the effect's own values over its defaults, with earlier keys
/// in their current form (Scribble's one `variation` as its Path Overlap and Spacing variations).
fn editor_params(e: &Effect) -> serde_json::Map<String, Value> {
    match vectorcraft_effects::merged_params(&e.id, &e.params) {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// The label of an effect's parameter `key` in [`effect_editor`]: the label the effect dialog's
/// generic fields give it ([`crate::dialogs::humanize`]) without the ":" ("offsetX" → "Offset X",
/// "maxRadius" → "Max. Radius"), in the UI language for one of our own effects (a plug-in's
/// parameter names are shown as they are).
fn param_label(key: &str, built_in: bool) -> String {
    let label = crate::dialogs::humanize(key);
    super::label_or_name(label.trim_end_matches(':'), built_in).to_string()
}

/// The paint a fill/stroke row's swatch sets: of the selection's fill/stroke row `index`, or, with
/// nothing selected, the next object drawn's top fill (`fill`) or stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Painted {
    Item(usize),
    NewArt { fill: bool },
}

/// The swatch of fill/stroke row `r` showing `paint`: a click opens a popup of the document's
/// swatches that sets the paint `target`; Shift-click opens the Color panel on it instead.
fn swatch_button(app: &mut VectorcraftApp, ui: &mut Ui, r: Rect, paint: &Paint, target: Painted) {
    let cr = Rect::from_min_size(pos2(r.left() + EYE_W + 76.0, r.center().y - 9.0), vec2(18.0, 18.0));
    chip(ui, cr, paint, false);
    let resp = ui
        .interact(cr.expand(2.0), ui.scope_id().with(("ap-chip", target)), Sense::click())
        .on_hover_text(tl!("Click to choose a swatch, Shift-click to mix a colour"));
    let mixer = resp.clicked() && ui.input(|inp| inp.modifiers.shift);
    if mixer {
        match target {
            Painted::Item(i) => {
                select_row(app, ui.ctx(), Sel::Item(i));
                super::set_row_keys(ui.ctx(), ROW_KEYS, true);
            }
            Painted::NewArt { fill } => {
                app.run("paint.toggleActive", json!({ "fill": fill })).ok();
            }
        }
        open_popup(app, ui.ctx(), "color");
    }
    egui::Popup::menu(&resp).open_memory((resp.clicked() && !mixer).then_some(egui::SetOpenCommand::Toggle)).show(|ui| {
        swatch_picker(app, ui, target);
    });
}

/// The weight of stroke row `r` (stroke `st`) and its notes after it: the Stroke panel's spinner
/// (arrows, Units > Stroke, preset dropdown); a weight typed, stepped or picked goes to the stroke
/// `target`, with nothing selected the next object drawn's (`stroke.set`).
fn weight_field(app: &mut VectorcraftApp, ui: &mut Ui, r: Rect, st: &vectorcraft_doc::StrokeLayer, target: Painted) {
    const W: f32 = 100.0;
    let fr = Rect::from_min_size(pos2(r.left() + EYE_W + 104.0, r.center().y - 13.0), vec2(W, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fr).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let unit = app.session.stroke_unit();
    let presets = super::stroke::weight_presets(unit);
    if let Some(w) = widgets::spin_field_stops_named(&mut child, ("ap-w", target), Some(tl!("Weight:")), Some(st.width), unit, W, 1.0, 0.0, &presets)
    {
        match target {
            Painted::Item(index) => app.run("appearance.setItem", aligned(app, json!({"index": index, "weight": w}))),
            Painted::NewArt { .. } => app.run("stroke.set", json!({ "weight": w })),
        }
        .ok();
    }
    stroke_notes(ui, Rect::from_min_max(pos2(fr.right() + 6.0, r.top()), pos2(r.right() - 4.0, r.bottom())), st);
}

/// Small swatch grid inside the appearance chip popup; a pick paints `target`.
fn swatch_picker(app: &mut VectorcraftApp, ui: &mut Ui, target: Painted) {
    let Some(st) = app.session.active() else { return };
    let mut all: Vec<(String, Paint)> = st.doc.swatches.iter().map(|s| (s.name.clone(), s.paint.clone())).collect();
    for g in &st.doc.swatch_groups {
        all.extend(g.swatches.iter().map(|s| (s.name.clone(), s.paint.clone())));
    }
    ui.set_max_width(12.0 * 17.0 + 8.0);
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.5, 1.5);
        for (name, p) in &all {
            let (r, resp) = ui.allocate_exact_size(vec2(15.5, 15.5), Sense::click());
            widgets::swatch_tile(ui, r, p, false, resp.hovered());
            let resp = resp.on_hover_text(name);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
            if resp.clicked() {
                chosen = Some((name.clone(), p.is_none()));
            }
        }
    });
    if let Some((name, none)) = chosen {
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
        ui.close();
    }
}

/// The bottom bar: Add New Stroke, Add New Fill, Add New Effect, Clear Appearance, and Duplicate and Delete Selected Item, which also take dropped rows.
fn bottom(app: &mut VectorcraftApp, ui: &mut Ui, node: Option<&Node>, sel: Sel) {
    let has = node.is_some();
    widgets::bottom_bar(ui, |ui| {
        if widgets::icon_button_enabled(ui, "dc-new-stroke", tl!("Add New Stroke"), false, has, 24.0).clicked() {
            app.run("appearance.addStroke", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "dc-new-fill", tl!("Add New Fill"), false, has, 24.0).clicked() {
            app.run("appearance.addFill", json!({})).ok();
        }
        let fx = widgets::icon_button_enabled(ui, "dc-fx", tl!("Add New Effect"), false, has, 24.0);
        egui::Popup::menu(&fx).show(|ui| {
            fx_menu(app, ui);
        });
        ui.add_space((ui.available_width() - 3.0 * 28.0).max(0.0));
        let basic = node.is_none_or(|n| n.appearance.is_basic());
        if widgets::icon_button_enabled(ui, "dc-clear", tl!("Clear Appearance"), false, has, 24.0).clicked() {
            app.run("appearance.clear", json!({})).ok();
        }
        // A dragged fill/stroke or effect row can be dropped on them (not the Contents row), also
        // with nothing selected: they light up while it is held.
        let dragged = matches!(pstate::<Option<Dragged>>(ui.ctx(), "ap-drag"), Some(Dragged::Item(_) | Dragged::Effect(..)));
        let can = has && (sel.editable() || dragged);
        let duplicate = widgets::icon_button_enabled(ui, "dc-new-item", tl!("Duplicate Selected Item"), false, can, 24.0);
        if duplicate.clicked() {
            duplicate_selected(app, ui, node, sel);
        }
        let delete = widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Selected Item"), false, can, 24.0);
        if delete.clicked() {
            delete_selected(app, ui, node, sel);
        }
        drop_on_buttons(app, ui, node, sel, &duplicate, &delete);
        let _ = basic;
    });
}

/// A stack row dragged onto Duplicate Selected Item or Delete Selected Item: the button is outlined
/// while the row hangs over it, and on release the row is copied or removed, as rows dropped on
/// the Layers panel's buttons are. The buttons take the row even when nothing is selected (the row
/// names itself). Ends the row drag once the pointer is up.
fn drop_on_buttons(app: &mut VectorcraftApp, ui: &Ui, node: Option<&Node>, sel: Sel, duplicate: &egui::Response, delete: &egui::Response) {
    let ctx = ui.ctx();
    let dragging: Option<Dragged> = pstate(ctx, "ap-drag");
    if !ctx.input(|i| i.pointer.any_down()) {
        set_pstate::<Option<Dragged>>(ctx, "ap-drag", None);
    }
    let (Some(what), Some(n), Some(p)) = (dragging, node, ctx.pointer_interact_pos()) else { return };
    let items = selected_items(ctx, Some(n), sel);
    let effects = selected_effects(ctx, Some(n), sel);
    let released = ctx.input(|i| i.pointer.any_released());
    for (button, copy) in [(duplicate, true), (delete, false)] {
        let Some((cmd, params)) = button_drop_command(&items, effects.as_ref(), what, copy).filter(|_| button.rect.contains(p)) else { continue };
        if !released {
            widgets::drop_target_outline(ui, button.rect);
            continue;
        }
        // The rows copied or removed with the dropped one (its highlighted list, when it is in it).
        let gone: Vec<u64> = params["indices"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
        let ran = app.run(cmd, aligned(app, params)).is_ok();
        let removed = ran && !copy;
        if ran && copy {
            let rows = if let Dragged::Effect(item, _) = what { Rows::Effects(item) } else { Rows::Items };
            let copied: Vec<usize> = gone.iter().map(|k| *k as usize).collect();
            highlight_copies(app, ctx, n, sel, rows, &copied);
        }
        // When the active effect was among them, its fill/stroke (or nothing) is selected; the
        // engine clears or renumbers the active fill/stroke itself.
        let active_gone = match (what, sel) {
            (Dragged::Effect(None, _), Sel::Effect(a)) => gone.contains(&(a as u64)),
            (Dragged::Effect(Some(i), _), Sel::ItemEffect(j, a)) => i == j && gone.contains(&(a as u64)),
            _ => false,
        };
        if removed
            && active_gone
            && let Dragged::Effect(item, _) = what
        {
            select_row(app, ctx, item.map_or(Sel::None, Sel::Item));
        } else if removed {
            let gone: Vec<usize> = gone.iter().map(|k| *k as usize).collect();
            let rows = if let Dragged::Effect(item, _) = what { Rows::Effects(item) } else { Rows::Items };
            highlight_after_removal(app, ctx, n, sel, rows, &gone);
        }
    }
}

/// After the rows `gone` of kind `rows` were removed while the active row survived, the active row,
/// the highlight and its anchor move up past the removed rows below them; removed rows drop out of
/// the highlight. `n` is the object as it was before the removal.
fn highlight_after_removal(app: &mut VectorcraftApp, ctx: &egui::Context, n: &Node, sel: Sel, rows: Rows, gone: &[usize]) {
    let h = highlight(ctx, n, sel);
    if h.rows != rows {
        return;
    }
    let shift = |k: usize| (!gone.contains(&k)).then(|| k - gone.iter().filter(|g| **g < k).count());
    let order = h.order.iter().filter_map(|k| shift(*k)).collect();
    let anchor = h.anchor.and_then(shift);
    // A fill/stroke's Opacity row stays selected (renumbered) while its fill/stroke survives.
    let active = match (sel, rows) {
        (Sel::ItemOpacity(i), Rows::Items) => shift(i).map(Sel::ItemOpacity),
        _ => sel.rows().filter(|(r, _)| *r == rows).and_then(|(_, a)| shift(a)).map(|a| rows.sel(a)),
    };
    set_active(app, ctx, active.unwrap_or(Sel::None));
    let active = active.and_then(|a| a.rows()).map(|(_, a)| a);
    set_pstate(ctx, "ap-multi", Highlight { owner: Some(n.id), rows, order, anchor, active });
}

/// The command a stack row `what` dropped on Duplicate Selected Item (`copy`) or Delete Selected
/// Item runs: a fill/stroke row acts on the highlighted rows `items` when it is one of them, an
/// effect row on the highlighted `effects` (list, positions) when it is one of them, else either on
/// itself alone. The Contents row is neither copied nor removed (None).
fn button_drop_command(items: &[usize], effects: Option<&(Option<usize>, Vec<usize>)>, what: Dragged, copy: bool) -> Option<(&'static str, Value)> {
    match what {
        Dragged::Item(i) => {
            let indices = if items.contains(&i) { items.to_vec() } else { vec![i] };
            Some((if copy { "appearance.duplicateItem" } else { "appearance.removeItem" }, json!({ "indices": indices })))
        }
        Dragged::Effect(item, k) => {
            let indices = match effects {
                Some((list, ks)) if *list == item && ks.contains(&k) => ks.clone(),
                _ => vec![k],
            };
            Some((if copy { "effect.duplicate" } else { "effect.remove" }, json!({"item": item, "indices": indices})))
        }
        Dragged::Contents => None,
    }
}

/// Duplicate the highlighted fills/strokes or effects: each copy shows right above its row and the
/// copies are highlighted ([`highlight_copies`]).
fn duplicate_selected(app: &mut VectorcraftApp, ui: &Ui, node: Option<&Node>, sel: Sel) {
    let (rows, indices) = match selected_effects(ui.ctx(), node, sel) {
        Some((item, ks)) => (Rows::Effects(item), ks),
        None if matches!(sel, Sel::Item(_)) => (Rows::Items, selected_items(ui.ctx(), node, sel)),
        None => return,
    };
    let (cmd, params) = match rows {
        Rows::Effects(item) => ("effect.duplicate", json!({"item": item, "indices": indices})),
        Rows::Items => ("appearance.duplicateItem", json!({ "indices": indices })),
    };
    if let (Ok(_), Some(n)) = (app.run(cmd, aligned(app, params)), node) {
        highlight_copies(app, ui.ctx(), n, sel, rows, &indices);
    }
}

/// After the rows `dup` of kind `rows` were copied in place, the copies are highlighted, as the
/// Layers panel highlights its duplicates: each shows right above its original, a fill/stroke's at
/// the next paint index, an effect's where the original was (the original one below it). The copy of
/// the active row becomes active (else the first copy), the copy of the anchor the anchor. `n` is
/// the object as it was before the copy.
fn highlight_copies(app: &mut VectorcraftApp, ctx: &egui::Context, n: &Node, sel: Sel, rows: Rows, dup: &[usize]) {
    let below = |k: usize| dup.iter().filter(|d| **d < k).count();
    let copy = |k: usize| match rows {
        Rows::Items => k + below(k) + 1,
        Rows::Effects(_) => k + below(k),
    };
    let h = highlight(ctx, n, sel);
    let ours = |k: &usize| h.rows == rows && dup.contains(k);
    // The copies of the highlighted rows in their highlight order, then any other copied row.
    let mut order: Vec<usize> = h.order.iter().filter(|k| ours(k)).map(|k| copy(*k)).collect();
    for c in dup.iter().map(|k| copy(*k)) {
        if !order.contains(&c) {
            order.push(c);
        }
    }
    let active = sel.rows().filter(|(r, a)| *r == rows && dup.contains(a)).map(|(_, a)| copy(a)).or(order.first().copied());
    let anchor = h.anchor.filter(|a| ours(a)).map(copy).or(active);
    set_active(app, ctx, active.map_or(Sel::None, |a| rows.sel(a)));
    set_pstate(ctx, "ap-multi", Highlight { owner: Some(n.id), rows, order, anchor, active });
}

/// Remove the highlighted fills/strokes or effects; a removed item effect leaves its fill/stroke selected.
fn delete_selected(app: &mut VectorcraftApp, ui: &Ui, node: Option<&Node>, sel: Sel) {
    let ok = match selected_effects(ui.ctx(), node, sel) {
        Some((item, ks)) => app.run("effect.remove", aligned(app, json!({"item": item, "indices": ks}))).is_ok(),
        None if matches!(sel, Sel::Item(_)) => {
            app.run("appearance.removeItem", aligned(app, json!({"indices": selected_items(ui.ctx(), node, sel)}))).is_ok()
        }
        None => false,
    };
    // Removing the active item clears it in the engine; a removed item effect leaves its item
    // selected.
    if ok {
        select_row(app, ui.ctx(), if let Sel::ItemEffect(i, _) = sel { Sel::Item(i) } else { Sel::None });
    }
}

/// The fx menu (Appearance panel, Properties): effects grouped by their Effect-menu submenu;
/// opens the effect dialog.
pub(crate) fn fx_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mut groups: Vec<(String, Vec<(String, String)>)> = vec![];
    let plugins: Vec<_> = vectorcraft_effects::plugin_effects().into_iter().map(catalog_entry).collect();
    // Built-in effects in the UI language; a plug-in's by the name it gives.
    let built_in = catalog().iter().map(|(id, label, menu)| (id, tl!(label), menu));
    for (id, label, menu) in built_in.chain(plugins.iter().map(|(id, label, menu)| (id, label.as_str(), menu))) {
        let g = menu.get(1).cloned().unwrap_or_else(|| "Other".into());
        match groups.iter_mut().find(|(n, _)| *n == g) {
            Some((_, v)) => v.push((id.clone(), label.to_string())),
            None => groups.push((g, vec![(id.clone(), label.to_string())])),
        }
    }
    for (g, items) in groups {
        ui.menu_button(tl!(&g), |ui| {
            crate::widgets::menu_scroll(ui, |ui| {
                for (id, label) in items {
                    if ui.button(format!("{label}…")).clicked() {
                        app.run("effect.dialog", json!({"effect": id})).ok();
                        ui.close();
                    }
                }
            });
        });
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let node = first_subject(app);
    let has = node.is_some();
    let sel = current_sel(app, ui.ctx(), node.as_ref());
    if menu_item(ui, tl!("Add New Fill"), has, false) {
        app.run("appearance.addFill", json!({})).ok();
    }
    if menu_item(ui, tl!("Add New Stroke"), has, false) {
        app.run("appearance.addStroke", json!({})).ok();
    }
    ui.separator();
    if menu_item(ui, tl!("Duplicate Item"), has && sel.editable(), false) {
        duplicate_selected(app, ui, node.as_ref(), sel);
    }
    if menu_item(ui, tl!("Remove Item"), has && sel.editable(), false) {
        delete_selected(app, ui, node.as_ref(), sel);
    }
    if menu_item(ui, tl!("Clear Appearance"), has, false) {
        app.run("appearance.clear", json!({})).ok();
    }
    if menu_item(ui, tl!("Reduce to Basic Appearance"), has && !node.as_ref().is_some_and(|n| n.appearance.is_basic()), false) {
        app.run("appearance.reduceToBasic", json!({})).ok();
    }
    ui.separator();
    let basic = app.session.prefs.new_art_basic;
    if menu_item(ui, tl!("New Art Has Basic Appearance"), true, basic) {
        app.run("appearance.setNewArtBasic", json!({ "on": !basic })).ok();
    }
    let hide: bool = pstate(ui.ctx(), "ap-hide-thumb");
    if menu_item(ui, if hide { tl!("Show Thumbnail") } else { tl!("Hide Thumbnail") }, true, false) {
        set_pstate(ui.ctx(), "ap-hide-thumb", !hide);
    }
    ui.separator();
    let style = app.session.selection_graphic_style().map(|(g, _)| g.name.clone());
    let redefine = style
        .as_ref()
        .map_or_else(|| tl!("Redefine Graphic Style").into(), |n| crate::i18n::fmt(tl!("Redefine Graphic Style “{name}”"), &[("name", n)]));
    // Translated above, around the style's name.
    if widgets::menu_item_name(ui, &redefine, style.is_some(), false) {
        app.run("graphicStyle.redefine", json!({})).ok();
    }
    if menu_item(ui, tl!("Show All Hidden Attributes"), node.as_ref().is_some_and(|n| n.appearance.has_hidden()), false) {
        app.run("appearance.showAllHidden", json!({})).ok();
    }
}

#[cfg(test)]
mod tests;
