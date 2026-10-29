//! The Layers panel: a row for every layer, sublayer, group and object, nested and indented, with
//! the eye (visibility) and lock columns, the layer colour bar, a disclosure triangle, a thumbnail
//! and the name, then the target circle and the selection column; a bottom bar with the panel's
//! buttons and the ≡ menu.
//!
//! - Clicking a row highlights it and makes its layer current (`layer.setCurrent`); Shift-click
//!   highlights the range of rows down to it, Ctrl/Cmd-click toggles one (`layer.highlight`). Panel
//!   commands (Duplicate, Delete, Merge, Options…) act on the highlighted rows. Alt-click on a
//!   layer's or sublayer's name also selects its art (`layer.selectAll`), as its selection square
//!   does.
//! - The selection column selects a row's art (`layer.selectAll`, Shift adds or removes); a row
//!   whose art is selected shows a square in its layer's colour, and dragging that square onto
//!   another row moves the selected art there (Alt copies it).
//! - The eye and lock columns toggle a row (`layer.setProps`); dragging down a column sets every
//!   row it passes to the same state, in one undo step. Ctrl/Cmd-click an eye switches the layer
//!   between Preview and Outline. Alt-click an eye hides the other layers (`layer.hideOthers`), or
//!   shows every layer when they are hidden (`layer.showAll`); Alt-click a lock does the same with
//!   `layer.lockOthers` and `layer.unlockAll`. Ctrl/Cmd+Alt-click an eye switches the other layers
//!   to Outline and the row's layer to Preview (`layer.outlineOthers`), or switches every layer to
//!   Preview when the others are in Outline (`layer.previewAll`).
//! - Dragging rows moves them above, below or into a layer or group (`layer.move`, a drop line or
//!   a box shows where); Alt copies them. A dimmed copy of the row follows the pointer. Layers
//!   dropped in the empty space under the last row go below the bottom layer. Dropped on the trash
//!   they are deleted, on New Layer duplicated.
//! - The triangle opens or closes a row; Alt-click does the same to everything inside it.
//! - Double-clicking a name renames it; double-clicking elsewhere on a row opens its options.
//! - A target circle targets its layer, group or object (`layer.target`); dragging it onto another
//!   row's circle moves the appearance there (Alt copies it, `appearance.transfer`) and dropping it
//!   on the trash clears it.
//! - Search All and the Filter button at its right (`ui.layersFilter`, lit while a filter is on)
//!   narrow the rows, and every row that holds others shows open while they do. A row stays when
//!   its name holds the text typed, it is one of the object kinds ticked under By Object and it is
//!   in one of the states ticked under By Layer (a section with nothing ticked takes every row), or
//!   when a row inside it stays. Neither changes the document, its open rows or the selection.
//!
//! While an opacity mask is edited the panel lists only its art, under an `<Opacity Mask>` entry.
//!
//! The rows open are the document's ([`OpenRows`], saved in native files): a document opens with
//! only its top-level layers open. The rows shown are listed once per document change, and only
//! those scrolled into view are laid out, so a document of many thousand objects costs no more
//! per frame than a small one.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use egui::{Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveShape, Node, NodeId, NodeKind};
use vectorcraft_engine::OpenRows;

use crate::theme::Tokens;
use crate::thumb_worker::{Settle, ThumbWorker};
use crate::widgets::{Live, PanelDrag};
use crate::{VectorcraftApp, icons, widgets};

mod controls;
mod model;
mod rows;
mod thumbs;

use controls::{bottom_bar, filter_button};
pub use controls::{expand, menu, set_filter};
pub(crate) use model::shown_name;
pub use model::{Filter, ObjectKind, RowState};
#[cfg(test)]
use model::{Row, list_rows, painted_name};
use model::{push_rows, rows_of_doc};
use rows::{drag_ghost, key, rename_id, resolve_click, row};
#[cfg(test)]
use thumbs::THUMBS;
pub(crate) use thumbs::{ThumbSource, node_arc, real_thumb, thumb_options};

/// Width of the eye and the lock columns.
const COLUMN: f32 = 25.0;
/// Indent per nesting level.
const INDENT: f32 = 14.0;
/// Right edge offsets of the target circle and the selection square.
const TARGET_X: f32 = 28.0;
const SQUARE_X: f32 = 11.0;

/// Layers panel options (≡ › Panel Options…), kept with the UI state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PanelOptions {
    /// Show Layers Only: no rows for groups and objects.
    pub layers_only: bool,
    /// Locate Selected Objects (off by default): when the last selected object changes outside the
    /// panel, on the canvas or by a command, the panel opens the rows around it and scrolls to its
    /// row, as after Locate Object, without highlighting the row. When Show Layers Only, a search or
    /// the filter leaves the object without a row, the panel opens no rows for it.
    pub locate_selected: bool,
    /// Row height in points: [`ROW_SMALL`], [`ROW_MEDIUM`], [`ROW_LARGE`] or another (12–100).
    pub row_size: f32,
    /// Thumbnails on layer, group and object rows.
    pub thumb_layers: bool,
    pub thumb_groups: bool,
    pub thumb_objects: bool,
}

pub const ROW_SMALL: f32 = 20.0;
pub const ROW_MEDIUM: f32 = 26.0;
pub const ROW_LARGE: f32 = 40.0;

impl Default for PanelOptions {
    fn default() -> Self {
        Self { layers_only: false, locate_selected: false, row_size: ROW_MEDIUM, thumb_layers: true, thumb_groups: true, thumb_objects: true }
    }
}

impl PanelOptions {
    /// The row height, kept to 12–100 points.
    pub fn row(&self) -> f32 {
        if self.row_size.is_finite() { self.row_size.clamp(12.0, 100.0) } else { ROW_MEDIUM }
    }
    /// Whether `n`'s row shows a thumbnail.
    fn thumb(&self, n: &Node) -> bool {
        self.row() >= 18.0
            && match n.kind {
                NodeKind::Layer { .. } => self.thumb_layers,
                NodeKind::Group { .. } => self.thumb_groups,
                _ => self.thumb_objects,
            }
    }
}

/// What a drag in the panel carries.
#[derive(Clone, Debug)]
enum LayersDrag {
    /// Rows dragged to move (or, with Alt, copy) them.
    Rows(Vec<NodeId>),
    /// The selected-art square: the selected objects go to the row it is dropped on.
    Art,
    /// Ruler guide rows (indexes into the document's guides): they go to the layer of the row
    /// they are dropped on.
    Guides(Vec<usize>),
}

/// A drag down the eye or lock column: every row it passes takes `value`.
#[derive(Clone, Debug, Default)]
struct ColumnDrag {
    /// The eye column (visibility), else the lock column.
    eye: bool,
    value: bool,
    ids: Vec<u64>,
}

/// Everything one frame of the panel reads.
struct View<'a> {
    doc: &'a Arc<Document>,
    /// Where the row thumbnails come from.
    thumbs: ThumbSource<'a>,
    sel: HashSet<NodeId>,
    /// The selected ruler guides.
    guides: Vec<usize>,
    target: Option<NodeId>,
    current: Option<NodeId>,
    rows: Vec<NodeId>,
    opts: PanelOptions,
    t: Tokens,
    h: f32,
    column: Option<ColumnDrag>,
    /// The row to scroll into view.
    reveal: Option<NodeId>,
    /// Preferences › Units › Identify Objects By › XML ID.
    identify_xml_id: bool,
}

/// What one frame of the panel decided.
#[derive(Default)]
struct Out {
    actions: Vec<(String, Value)>,
    /// A row clicked with these modifiers (resolved once every row is listed).
    click: Option<(NodeId, egui::Modifiers)>,
    /// The column drag after this frame (`Some(None)`: it ended).
    column: Option<Option<ColumnDrag>>,
    /// Rows to open or close, and whether to open them.
    toggle: Vec<(NodeId, bool)>,
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    // A rename belongs to the document it started in (node ids are per document): drop it when
    // another document shows, so it can't rename that document's node with the same id.
    let shown_doc = egui::Id::unique("layers-doc");
    let before = ui.data(|d| d.get_temp::<u64>(shown_doc));
    if before != Some(st.uid) {
        ui.data_mut(|d| {
            if before.is_some() {
                d.remove::<(u64, String)>(rename_id());
            }
            d.insert_temp(shown_doc, st.uid);
        });
    }
    let doc = st.doc.clone();
    let rows = st.highlighted_rows();
    let current = st.current_layer();
    // Rows highlighted anew (a click, Locate Object, a new layer): open the layers around them.
    let seen: Vec<NodeId> = ui.data(|d| d.get_temp(key("seen"))).unwrap_or_default();
    let mut reveal = None;
    if rows != seen {
        open_around(app, &doc, &rows);
        reveal = rows.first().copied();
        ui.data_mut(|d| d.insert_temp(key("seen"), rows.clone()));
    }
    let highlighted = reveal.is_some();
    // A new active layer (a canvas selection highlights no row): reveal it too, unless a row
    // highlighted in the panel holds the view.
    let seen_current: Option<NodeId> = ui.data(|d| d.get_temp(key("seen-current"))).flatten();
    if current != seen_current {
        if rows.is_empty()
            && let Some(layer) = current
        {
            let around = doc.ancestry(layer).unwrap_or_default().split_last().map(|(_, a)| a.to_vec()).unwrap_or_default();
            if let Some(st) = app.session.active_mut() {
                for a in around {
                    st.layers_open.set(a, true);
                }
            }
            reveal = Some(layer);
        }
        ui.data_mut(|d| d.insert_temp(key("seen-current"), current));
    }
    // With Locate Selected Objects on, an object selected anew outside the panel (below).
    let located = newly_selected(app, ui).filter(|_| !highlighted);
    // Search All, and the Filter button at its right.
    let query = ui
        .horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                filter_button(app, ui);
                crate::widgets::search_field(ui, egui::Id::unique("layers-search"), tl!("Search All"))
            })
            .inner
        })
        .inner
        .trim()
        .to_lowercase();
    // The located object, when it has a row once the rows around it are open (Show Layers Only, a
    // search or the filter can leave it out): those rows open, and its row is revealed instead of
    // the row revealed above. Otherwise nothing opens.
    let located = located.filter(|id| {
        app.session.active().is_some_and(|st| listed_when_opened(&doc, &st.layers_open, &app.ui.layers_panel, &query, &app.ui.layers_filter, *id))
    });
    if let Some(id) = located {
        open_around(app, &doc, &[id]);
    }
    let reveal = located.or(reveal);
    let Some(st) = app.session.active() else { return };
    let list = rows_of_doc(ui, st, &app.ui.layers_panel, &query, &app.ui.layers_filter);
    let view = View {
        doc: &doc,
        thumbs: ThumbSource::of(app, st, &doc),
        sel: st.selection.objects.iter().copied().collect(),
        guides: st.selection.guides.clone(),
        target: st.selection.target,
        current: st.current_layer(),
        rows,
        opts: app.ui.layers_panel.clone(),
        t,
        h: app.ui.layers_panel.row(),
        column: ui.data(|d| d.get_temp(key("column"))),
        reveal,
        identify_xml_id: app.session.prefs.identify_objects_by == "xmlId",
    };
    let mut out = Out::default();
    ui.add_space(6.0);
    let list_h = ui.available_height() - 34.0;
    // Only the rows in view are laid out; one revealed out of view is scrolled to.
    let revealed = reveal.and_then(|id| list.iter().position(|r| r.node.id == id));
    let scrolled = ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // The bar takes a column of its own, so it never covers the target and selection marks at
        // the rows' right edge (#3443). Whether the rows overflow is known before they are laid
        // out, so the bar is decided up front and the rows never shift when it appears.
        ui.spacing_mut().scroll = crate::widgets::list_scroll_style();
        let overflows = view.h * list.len() as f32 > list_h;
        let bar =
            if overflows { egui::scroll_area::ScrollBarVisibility::AlwaysVisible } else { egui::scroll_area::ScrollBarVisibility::AlwaysHidden };
        egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).scroll_bar_visibility(bar).show_rows(
            ui,
            view.h,
            list.len(),
            |ui, range| {
                if let Some(i) = revealed.filter(|i| !range.contains(i)) {
                    let top = ui.max_rect().top() + (i as f32 - range.start as f32) * view.h;
                    ui.scroll_to_rect(egui::Rect::from_min_size(egui::pos2(ui.max_rect().left(), top), vec2(ui.max_rect().width(), view.h)), None);
                }
                for r in list.get(range).unwrap_or_default() {
                    row(ui, &view, r, &mut out);
                }
            },
        )
    });
    let list_rect = scrolled.inner.inner_rect;
    resolve_click(ui, &view, &list, &mut out);
    if let Some(st) = app.session.active_mut() {
        for (id, open) in out.toggle.drain(..) {
            st.layers_open.set(id, open);
        }
    }
    let selected = last_selected(app);
    // The column drag: live while it lasts, one undo step when the button is released.
    let released = ui.input(|i| i.pointer.any_released());
    // Layers dragged into the empty space under the last row go below the bottom layer (there is
    // no row there to take the drop).
    let below = list_rect.top() - scrolled.inner.state.clamped_offset().y + list.len() as f32 * view.h;
    if query.is_empty()
        && !app.ui.layers_filter.is_on()
        && let Some(drag) = egui::DragAndDrop::payload::<LayersDrag>(ui.ctx())
        && let LayersDrag::Rows(ids) = &*drag
        && let Some(bottom) = doc.layers.first()
        && !ids.is_empty()
        && !ids.contains(&bottom.id)
        && ids.iter().all(|i| doc.node(*i).is_some_and(Node::is_layer))
        && let Some(p) = ui.input(|i| i.pointer.hover_pos())
        && list_rect.contains(p)
        && p.y > below
    {
        let stroke = Stroke::new(2.0, view.t.accent);
        ui.painter().line_segment([egui::pos2(list_rect.left(), below), egui::pos2(list_rect.right(), below)], stroke);
        if released {
            let copy = ui.input(|i| i.modifiers.alt);
            let ids: Vec<u64> = ids.iter().map(|i| i.0).collect();
            out.actions.push(("layer.move".into(), json!({"ids": ids, "target": bottom.id.0, "place": "below", "copy": copy})));
        }
    }
    let changed = out.column.is_some();
    let column = out.column.take().unwrap_or_else(|| view.column.clone());
    if let Some(c) = column.as_ref().filter(|_| changed || released) {
        let (prop, label) = if c.eye { ("visible", "Visibility") } else { ("locked", "Lock") };
        let phase = if released { Live::Released } else { Live::Dragging };
        let params = json!({"ids": c.ids, prop: c.value});
        super::live_run(app, label, "layer.setProps", params, phase);
    }
    ui.data_mut(|d| match column.filter(|_| !released) {
        Some(c) => {
            d.insert_temp(key("column"), c);
        }
        None => d.remove::<ColumnDrag>(key("column")),
    });
    if released {
        ui.data_mut(|d| d.remove::<u64>(egui::Id::unique("layers-drag")));
    }
    // Dragged rows show as a dimmed row under the pointer.
    if !released
        && let Some(drag) = egui::DragAndDrop::payload::<LayersDrag>(ui.ctx())
        && let LayersDrag::Rows(ids) = &*drag
        && let Some(src) = ids.first()
        && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
    {
        drag_ghost(ui, &view, *src, pos);
    }
    // Keyboard: Delete removes the highlighted rows while the panel has the keyboard (a row was
    // clicked last, not the canvas).
    let pressed_outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !list_rect.contains(p)));
    if pressed_outside {
        ui.data_mut(|d| d.insert_temp(key("focus"), false));
    }
    let focus: bool = ui.data(|d| d.get_temp(key("focus"))).unwrap_or(false);
    if focus
        && !view.rows.is_empty()
        && ui.ctx().memory(|m| m.focused().is_none())
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        out.actions.push(("layer.delete".into(), json!({})));
    }
    bottom_bar(app, ui, &view, &doc, &mut out.actions);
    for (c, p) in out.actions {
        app.run(&c, p).ok();
    }
    see_selection(app, ui, selected);
}

/// Open the layers and groups around rows `ids`.
fn open_around(app: &mut VectorcraftApp, doc: &Document, ids: &[NodeId]) {
    let around: Vec<NodeId> =
        ids.iter().flat_map(|r| doc.ancestry(*r).unwrap_or_default().split_last().map(|(_, a)| a.to_vec()).unwrap_or_default()).collect();
    if let Some(st) = app.session.active_mut() {
        for a in around {
            st.layers_open.set(a, true);
        }
    }
}

/// Whether [`list_rows`] gives `id` a row once the layers and groups around it are open as well as
/// `open`. Only the rows of `id`'s top-level layer are listed.
fn listed_when_opened(doc: &Document, open: &OpenRows, opts: &PanelOptions, query: &str, filter: &Filter, id: NodeId) -> bool {
    let Some(ancestry) = doc.ancestry(id) else { return false };
    let Some(layer) = doc.layers.iter().find(|l| ancestry.first() == Some(&l.id)) else { return false };
    if doc.mask_edit.is_some_and(|m| m.layer != layer.id) {
        return false;
    }
    let mut open = open.clone();
    for a in ancestry.split_last().map(|(_, a)| a).unwrap_or_default() {
        open.set(*a, true);
    }
    let mut rows = vec![];
    push_rows(layer, 0, false, (Color32::PLACEHOLDER, true), (&open, opts, query, filter, doc), &mut rows);
    rows.iter().any(|r| r.node.id == id && r.guide.is_none())
}

/// The active document's uid and its last selected object, the one Locate Object locates.
fn last_selected(app: &VectorcraftApp) -> Option<(u64, Option<NodeId>)> {
    app.session.active().map(|st| (st.uid, st.selection.objects.last().copied()))
}

/// Locate Selected Objects: the object to locate in this frame. With the option on, that is the
/// last selected object when it differs from the one noted in an earlier frame for the same
/// document. [`see_selection`] notes the panel's own changes, so only a change made on the canvas
/// or by a command is located. This function notes nothing during a drag, so a selection made on
/// the canvas during one is located when the drag ends. When another document becomes active,
/// nothing is located.
fn newly_selected(app: &VectorcraftApp, ui: &Ui) -> Option<NodeId> {
    let busy = app.session.active().is_some_and(|st| st.interaction.is_some()) || app.session.tool_busy();
    let now = last_selected(app).filter(|_| !busy)?;
    let was = ui.data(|d| d.get_temp::<(u64, Option<NodeId>)>(key("selected")));
    ui.data_mut(|d| d.insert_temp(key("selected"), now));
    let anew = was.is_some_and(|w| w.0 == now.0 && w.1 != now.1);
    now.1.filter(|_| anew && app.ui.layers_panel.locate_selected)
}

/// Note the selection as seen when the panel's rows, buttons or menu changed it (`before`: what
/// [`last_selected`] gave before they ran), so Locate Selected Objects does not locate it.
fn see_selection(app: &VectorcraftApp, ui: &Ui, before: Option<(u64, Option<NodeId>)>) {
    if let Some(now) = last_selected(app).filter(|now| Some(*now) != before) {
        ui.data_mut(|d| d.insert_temp(key("selected"), now));
    }
}

#[cfg(test)]
mod tests;
