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
//! - The eye and lock columns toggle a row (`layer.setProps`), a ruler guide's row included;
//!   dragging down a column sets every row it passes to the same state, in one undo step.
//!   Ctrl/Cmd-click a layer's eye switches it between Preview and Outline. Alt-click an eye hides
//!   the other layers (`layer.hideOthers`), or shows every layer when they are hidden
//!   (`layer.showAll`); Alt-click a lock does the same with `layer.lockOthers` and
//!   `layer.unlockAll`. Ctrl/Cmd+Alt-click an eye switches the other layers to Outline and the
//!   row's layer to Preview (`layer.outlineOthers`), or switches every layer to Preview when the
//!   others are in Outline (`layer.previewAll`).
//! - A ruler guide's row selects the guide (`guide.select`) and leaves no row highlighted, so
//!   Delete then deletes the guide; dragged onto another layer's row the guide goes there
//!   (`guide.setLayer`), and onto the trash it is deleted (`guide.remove`).
//! - Dragging rows moves them above, below or into a layer or group (`layer.move`, a drop line or
//!   a box shows where); Alt copies them. A dimmed copy of the row follows the pointer, and rows
//!   held near the top or bottom edge of the list scroll it, faster nearer the edge. Layers
//!   dropped in the empty space under the last row go below the bottom layer. Dropped on the trash
//!   they are deleted, on New Layer duplicated.
//! - Deleting rows from the panel (the trash, rows dropped on it, Delete Selection, Delete and
//!   Backspace) asks first when a layer or sublayer among them holds art ([`delete`]).
//! - The triangle opens or closes a row; Alt-click does the same to everything inside it.
//! - Double-clicking a name renames it; double-clicking elsewhere on a row opens its options.
//! - A target circle targets its layer, group or object (`layer.target`). Its modifier keys
//!   follow the names' rules: Ctrl/Cmd adds the item to the selection or takes it out, Shift
//!   selects the art of every row from the last circle clicked to this one, and Shift+Ctrl/Cmd
//!   adds that art (#8639). Dragging a circle onto another row's circle moves the appearance there
//!   (Alt copies it, `appearance.transfer`) and dropping it on the trash reduces it to basic
//!   appearance (`appearance.reduceToBasic`) without deleting it.
//! - Search All and the Filter button at its right (`ui.layersFilter`, lit while a filter is on)
//!   narrow the rows, and every row that holds others shows open while they do. A row stays when
//!   its name holds the text typed, it is one of the object kinds ticked under By Object and it is
//!   in one of the states ticked under By Layer (a section with nothing ticked takes every row), or
//!   when a row inside it stays. Neither changes the document, its open rows or the selection.
//!
//! While an opacity mask is edited the panel lists only its art, under an `<Opacity Mask>` entry.
//!
//! The rows open are the document's ([`OpenRows`](vectorcraft_engine::OpenRows), saved in native
//! files): a document opens with only its top-level layers open. The rows shown are listed once per
//! document change, and only those scrolled into view are laid out, so a document of many thousand
//! objects costs no more per frame than a small one.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft::layers::ui::filter::{FilterBy, LayerFilter};
use craft::layers::ui::options::LayersSettings;
use craft::layers::ui::panel::{Groups, LayerRowLook, LayersPanel};
use craft::layers::ui::thumbs::ThumbContents;
use craft::ui::contract::{Bind, PlainData, Tracked};
use craft::ui::list::ThumbSize;
use egui::{Color32, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveShape, Node, NodeId, NodeKind};
use vectorcraft_engine::LayersReveal;

use crate::VectorcraftApp;

mod cells;
mod controls;
mod model;
mod rows;
mod thumbs;

pub use controls::{delete, expand, set_filter};
// PLAN: the sheet's `pub use` of these cannot work: document-panels-w8 declares them `pub(crate)`, which `pub use` cannot re-export.
pub(crate) use controls::{menu_rows, menu_states, new_layer_unlisted, run};
pub(crate) use model::shown_name;
use model::{DocLayers, LayerRows, RowId, isolated_root};
pub(crate) use thumbs::{NodeThumb, NodeThumbs, forget, link_thumbnails, node_arc, retain_open, thumb_options};

/// Layers panel options (≡ › Panel Options…), kept with the UI state.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PlainData)]
#[serde(default, rename_all = "camelCase")]
pub struct PanelOptions {
    /// Show Layers Only: no rows for groups and objects.
    pub layers_only: bool,
    /// Locate Selected Objects (off by default): when the last selected object changes outside the
    /// panel, on the canvas or by a command, the panel opens the rows around it and scrolls to its
    /// row, as after Locate Object, without highlighting the row. When Show Layers Only, a search or
    /// the filter leaves the object without a row, the panel opens no rows for it.
    pub locate_selected: bool,
    /// Row Size: the thumbnail size, which sets the rows' height.
    #[serde(deserialize_with = "saved_row_size")]
    pub row_size: RowSize,
    /// Thumbnails on layer, group and object rows.
    pub thumb_layers: bool,
    pub thumb_groups: bool,
    pub thumb_objects: bool,
}

impl Default for PanelOptions {
    fn default() -> Self {
        Self { layers_only: false, locate_selected: false, row_size: RowSize::Medium, thumb_layers: true, thumb_groups: true, thumb_objects: true }
    }
}

/// Panel Options › Row Size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, PlainData)]
#[serde(rename_all = "camelCase")]
pub enum RowSize {
    /// Small rows.
    Small,
    /// Medium rows.
    #[default]
    Medium,
    /// Large rows.
    Large,
}

/// A saved Row Size: its name, or the row height in points that options saved before Row Size had
/// names hold (up to 23 Small, up to 33 Medium, above Large). Anything else loads as Medium, so the
/// rest of the saved options still load.
fn saved_row_size<'de, D: serde::Deserializer<'de>>(d: D) -> Result<RowSize, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Name(RowSize),
        Points(f64),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Saved::deserialize(d)? {
        Saved::Name(size) => size,
        Saved::Points(points) if points.is_finite() && points <= 23.0 => RowSize::Small,
        Saved::Points(points) if points.is_finite() && points > 33.0 => RowSize::Large,
        Saved::Points(_) | Saved::Other(_) => RowSize::Medium,
    })
}

impl PanelOptions {
    /// The rows' thumbnail size, which sets their height.
    pub fn thumb_size(&self) -> ThumbSize {
        match self.row_size {
            RowSize::Small => ThumbSize::Small,
            RowSize::Medium => ThumbSize::Medium,
            RowSize::Large => ThumbSize::Large,
        }
    }

    /// Whether `n`'s row shows a thumbnail.
    fn thumb(&self, n: &Node) -> bool {
        match n.kind {
            NodeKind::Layer { .. } => self.thumb_layers,
            NodeKind::Group { .. } => self.thumb_groups,
            _ => self.thumb_objects,
        }
    }

    /// The settings the panel shows its rows with: the thumbnails frame each row's own art.
    fn settings(&self) -> LayersSettings {
        let mut settings = LayersSettings::default();
        settings.options.thumbnail_size = self.thumb_size();
        settings.options.thumbnail_contents = ThumbContents::LayerBounds;
        settings.show_filters = true;
        settings
    }
}

/// What the app keeps of the Layers panel beside its document: the filter, a rename asked for by
/// `ui.rename`, whether the rows had key focus in the last pass and the rows the panel reads. Not saved.
#[derive(Clone, Debug)]
pub struct LayersUi {
    /// Search All and the Filter's ticks.
    pub filter: LayerFilter,
    /// The row whose name field opens in the next pass.
    pub rename: Option<RowId>,
    /// Key focus was in the rows: Rename then names the highlighted row first.
    pub focused: bool,
    /// The target circle clicked last, with its document's `uid`: where a Shift-click on another circle starts.
    pub target_anchor: Option<(u64, NodeId)>,
    /// The selection square clicked last in each open document, by the document's `uid`: where a Shift-click on another
    /// square starts.
    pub art_anchors: HashMap<u64, NodeId>,
    /// The active document's rows with the [`LayerRows::key`] they were listed at.
    rows: Option<(u64, Tracked<LayerRows>)>,
}

impl Default for LayersUi {
    /// No filter. The panel offers only the Objects filter, so the filter is of that type from the start.
    fn default() -> Self {
        let mut filter = LayerFilter::default();
        filter.by = FilterBy::Objects;
        Self { filter, rename: None, focused: false, target_anchor: None, art_anchors: HashMap::new(), rows: None }
    }
}

/// The Layers panel: the layer tree with its click, drag and rename rules and the bottom buttons; Delete/Backspace in the rows delete the highlighted rows.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // In isolation mode only rows inside the isolated art stay highlighted, since the panel lists
    // no others.
    if let Some(st) = app.session.active_mut()
        && let Some(root) = st.isolation.filter(|i| st.doc.node(*i).is_some())
    {
        let doc = st.doc.clone();
        let inside = |r: &NodeId| doc.ancestry(*r).is_some_and(|a| a.contains(&root));
        if !st.layer_rows.iter().all(&inside) {
            st.layer_rows.retain(&inside);
        }
    }
    let Some(uid) = app.session.active().map(|st| st.uid) else { return };
    // The reveal and the rename are requests: the pass that takes them sends them, and a second pass
    // in the frame sends none, as the panel acts on each once.
    let reveal = reveal_for(app);
    let rename = app.ui.layers_ui.rename.take();
    let mut filter = app.ui.layers_ui.filter.clone();
    let mut footer = Vec::new();
    let mut picks = Vec::new();
    let mut target_click = None;
    let mut art_click = None;
    let thumbnails = thumbs::layer_thumbnails(ui.ctx(), uid);
    // A drag's previews keep the shown pictures; the rows render once it ends.
    thumbnails.hold(app.session.active().is_some_and(|st| st.interaction.is_some()) || app.session.tool_busy());
    let settings = app.ui.layers_panel.settings();
    refresh_rows(app);
    let (Some(st), Some((_, kept))) = (app.session.active(), &app.ui.layers_ui.rows) else { return };
    let tree = DocLayers::of(st, kept);
    let source = thumbs::LayerThumbs { doc: &st.doc };
    let mut panel = LayersPanel::new(kept)
        .settings(&settings)
        .thumbnails(&thumbnails, &source)
        .filter(Bind::bound(&mut filter))
        .filter_types([FilterBy::Objects])
        .row_look(LayerRowLook::ColorBar)
        .lock_column(true)
        .blend_rows(false)
        .deletable(true)
        .takes_drop::<cells::ArtDrag>(cells::accepts_art(&tree))
        .trailing(|id, info| cells::build(&tree, id, info), |id, shown| cells::output(&mut picks, &mut target_click, &mut art_click, id, shown))
        .row_menu(|target| controls::row_menu(&tree, target))
        .footer(|ui| controls::footer(&tree, ui, &mut footer))
        // Revert keeps the UID but replaces the drawing that the row edits belonged to.
        .document((uid, st.reverts));
    if let Some(id) = reveal {
        panel = panel.reveal(RowId::Node(id), Groups::Open);
    }
    if let Some(row) = rename {
        panel = panel.start_rename(row);
    }
    let output = panel.show(ui);
    // A Shift-click starts at the last circle clicked in this document, else at the targeted row.
    let anchor = app.ui.layers_ui.target_anchor.filter(|(doc, _)| *doc == uid).map(|(_, id)| id);
    let target = target_click.map(|(id, mods)| (id, rows::target_click(&tree, &filter, anchor, id, mods)));
    // A Shift-click on a selection square starts at the last square clicked in this document.
    let art_anchor = app.ui.layers_ui.art_anchors.get(&uid).copied();
    let art = art_click.map(|(id, mods)| (id, rows::art_click(&tree, &filter, art_anchor, id, mods)));
    app.ui.layers_ui.filter = filter;
    if let Some((id, (_, true))) = &target {
        app.ui.layers_ui.target_anchor = Some((uid, *id));
    }
    if let Some((id, (_, true))) = &art {
        // Closed documents' starts are dropped, so the map holds one entry per open document at most.
        let open: HashSet<u64> = app.session.documents().iter().map(|st| st.uid).collect();
        let anchors = &mut app.ui.layers_ui.art_anchors;
        anchors.retain(|doc, _| open.contains(doc));
        anchors.insert(uid, *id);
    }
    app.ui.layers_ui.focused = output.focused;
    rows::apply(app, output.events);
    let clicks = art.into_iter().chain(target).map(|(_, (command, _))| command);
    for (c, p) in footer.into_iter().chain(picks).chain(clicks) {
        app.run(&c, p).ok();
    }
    // The panel's own commands reveal nothing for the selection they made: Locate Selected Objects
    // follows selections made on the canvas or by a command.
    if let Some(st) = app.session.active_mut()
        && matches!(st.layers_reveal, Some(LayersReveal::Selected(_)))
    {
        st.layers_reveal = None;
    }
}

/// Runs `f`, a change the Layers panel makes itself, which reveals nothing: the rows it changes are
/// in view, and Locate Selected Objects follows selections made on the canvas or by a command. The
/// request `f` raised is dropped; one pending before stands.
fn unrevealed<T>(app: &mut VectorcraftApp, f: impl FnOnce(&mut VectorcraftApp) -> T) -> T {
    let before = app.session.active().map(|st| (st.uid, st.layers_reveal));
    let r = f(app);
    if let Some(st) = app.session.active_mut()
        && let Some((_, request)) = before.filter(|(uid, _)| *uid == st.uid)
    {
        st.layers_reveal = request;
    }
    r
}

/// Lists the active document's rows again when what they are listed from changed.
fn refresh_rows(app: &mut VectorcraftApp) {
    let Some(st) = app.session.active() else { return };
    let key = LayerRows::key(app, st);
    if app.ui.layers_ui.rows.as_ref().is_none_or(|(kept, _)| *kept != key) {
        let rows = Tracked::new(LayerRows::new(app, st));
        app.ui.layers_ui.rows = Some((key, rows));
    }
}

/// The row the panel reveals this pass, with the layers around it opened: the active document's
/// reveal request, which is consumed whether it reveals anything or not. A row (highlighted,
/// located, isolated) is revealed when the panel lists it once the layers around it are open; a new
/// current layer only while no row is highlighted; a newly selected object only with Locate
/// Selected Objects on and when it has a row, else the current layer as for a new current layer.
fn reveal_for(app: &mut VectorcraftApp) -> Option<NodeId> {
    let request = app.session.active_mut()?.layers_reveal.take()?;
    let st = app.session.active()?;
    let tree = DocLayers::new(app, st);
    let listed = |id: NodeId| listed_when_opened(&tree, &app.ui.layers_ui.filter, id);
    let unhighlighted = st.highlighted_rows().is_empty();
    let id = match request {
        LayersReveal::Row(id) => listed(id).then_some(id),
        LayersReveal::Current(layer) => unhighlighted.then_some(layer),
        LayersReveal::Selected(id) if app.ui.layers_panel.locate_selected && listed(id) => Some(id),
        LayersReveal::Selected(_) => st.current_layer().filter(|_| unhighlighted),
    }?;
    let doc = st.doc.clone();
    open_around(app, &doc, &[id]);
    Some(id)
}

/// `ui.rename {id?}`: open the Layers panel's name field on the row of layer, group or object
/// `id`, as double-clicking the row's name does: Enter renames it (`layer.setProps`, one undo
/// step), Escape leaves the name as it was. Without `id`, the row Rename names is the Layers
/// panel's highlighted row when a row was clicked last, else the one selected object or group,
/// else the one highlighted row, else the current layer. Several of them, a row the panel's
/// search, filter or Show Layers Only leaves out, or in isolation mode an object outside the
/// isolated art, is an error that opens nothing.
pub fn rename(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("the Layers panel is not shown")?;
    let st = app.session.active().ok_or("no document open")?;
    let one = |ids: &[NodeId]| match ids {
        [id] => Ok(*id),
        _ => Err("select one layer, group or object to rename".to_string()),
    };
    let id = match p.get("id") {
        Some(v) => {
            let id = NodeId(v.as_u64().ok_or_else(|| format!("`id` is an object id, not {v}"))?);
            st.doc.node(id).map(|_| id).ok_or("no such layer or object")?
        }
        None => {
            let rows = st.highlighted_rows();
            let selected = &st.selection.objects;
            let rows_first = app.ui.layers_ui.focused || selected.is_empty();
            if rows_first && !rows.is_empty() {
                one(&rows)?
            } else if !selected.is_empty() {
                one(selected)?
            } else {
                st.current_layer().ok_or("no layer to rename")?
            }
        }
    };
    let doc = st.doc.clone();
    let node = doc.node(id).ok_or("no such layer or object")?;
    if st.isolation.and_then(|i| isolated_root(&doc, i)).is_some_and(|(root, ..)| !doc.ancestry(id).is_some_and(|a| a.contains(&root.id))) {
        return Err("the object is outside the isolated group".into());
    }
    if !listed_when_opened(&DocLayers::new(app, st), &app.ui.layers_ui.filter, id) {
        return Err("the Layers panel's search, filter or Show Layers Only hides its row".into());
    }
    let name = node.identify_label(app.session.prefs.identify_objects_by == "xmlId");
    crate::panel_docking::reveal_panel(app, "layers")?;
    open_around(app, &doc, &[id]);
    app.ui.layers_ui.rename = Some(RowId::Node(id));
    if let Some(st) = app.session.active_mut() {
        st.layers_reveal = Some(LayersReveal::Row(id));
    }
    ctx.request_repaint();
    Ok(json!({ "id": id.0, "name": name }))
}

// PLAN: the sheet keeps `last_selected`, but nothing reads it once the session decides what to locate (document-panels-w8's controls.rs no longer calls it); it would be dead code, so it goes.
/// Open the layers and groups around rows `ids`. In isolation mode only those from the isolated
/// row down open, and nothing opens for a row outside it.
fn open_around(app: &mut VectorcraftApp, doc: &Document, ids: &[NodeId]) {
    let root = app.session.active().and_then(|st| st.isolation).filter(|i| doc.node(*i).is_some());
    let around: Vec<NodeId> = ids
        .iter()
        .flat_map(|r| {
            let ancestry = doc.ancestry(*r).unwrap_or_default();
            let from = root.map_or(Some(0), |i| ancestry.iter().position(|a| *a == i));
            let parents = ancestry.split_last().map(|(_, a)| a.to_vec()).unwrap_or_default();
            from.map(|f| parents.into_iter().skip(f).collect::<Vec<_>>()).unwrap_or_default()
        })
        .collect();
    if let Some(st) = app.session.active_mut() {
        for a in around {
            st.layers_open.set(a, true);
        }
    }
}

/// Whether the panel over `tree` gives object `id` a row once the layers and groups around it are
/// open, under `filter`: every row from the top one (the isolated row in isolation mode) down to
/// `id` is listed in the one above it, and `filter` keeps `id`.
fn listed_when_opened(tree: &DocLayers<'_>, filter: &LayerFilter, id: NodeId) -> bool {
    use craft::layers::ui::tree::LayerTree as _;
    let Some(ancestry) = tree.doc().ancestry(id) else { return false };
    let from = tree.root().map_or(Some(0), |root| ancestry.iter().position(|a| *a == root));
    let Some(path) = from.and_then(|f| ancestry.get(f..)) else { return false };
    let mut above: Option<RowId> = None;
    for a in path {
        let row = RowId::Node(*a);
        if !tree.children(above.as_ref()).contains(&row) {
            return false;
        }
        above = Some(row);
    }
    !filter.active() || tree.listed(&RowId::Node(id), filter)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_reveal;
