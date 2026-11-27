use std::collections::HashSet;

use craft::behavior::{EditEvent, SelectionChange};
use craft::keymap::{CommandRef, Mods};
use craft::layers::ui::panel::{ActivateOn, LayerColumn, LayerDrop, LayerMove, LayerPosition, LayerSelect, LayerToggle, LayersEvent};
use craft::layers::ui::tree::{LayerRowKey, LayerTree};
use craft::ui::contract::{DropPayload, OpenChange, ReorderMove};
use vectorcraft_engine::DocState;
use vectorcraft_tools::intertwine::{in_intertwine, refused_member};

use super::cells::ArtDrag;
use super::model::{DocLayers, RowId, opens};
use super::{Document, LayerFilter, Node, NodeId, NodeKind, Value, VectorcraftApp, json};
use crate::edits::{EditPath, apply_event};
use crate::panels::Live;

/// Whether row `n` can take what is moved (`moving`, objects only when `objects`) inside it. A
/// layer takes them; a group or an Intertwine group takes objects; an Intertwine group, or a group
/// in one, only objects that can be intertwined.
pub(super) fn takes(n: &Node, objects: bool, moving: &[NodeId], doc: &Document) -> bool {
    match n.kind {
        NodeKind::Layer { .. } => true,
        NodeKind::Group { .. } | NodeKind::Intertwine { .. } => {
            objects && (!in_intertwine(doc, n.id) || moving.iter().filter_map(|i| doc.node(*i)).all(|m| refused_member(m).is_none()))
        }
        _ => false,
    }
}

/// Context menu Others action for `ids`: Lock Others / Unlock All Layers or Hide Others / Show All Layers.
/// When any other layer is unlocked (visible), returns `lockOthers` (`hideOthers`) preserving all layers holding
/// `ids`. When all other layers are locked (hidden) already, returns `unlockAll` (`showAll`).
pub(crate) fn multi_others_action(doc: &Document, ids: &[u64], eye: bool) -> (String, Value, bool) {
    let own_layers: std::collections::HashSet<NodeId> = ids.iter().filter_map(|&id| doc.layer_of(NodeId(id))).collect();
    let has_other_active = if eye {
        doc.layers.iter().any(|l| !own_layers.contains(&l.id) && l.visible)
    } else {
        doc.layers.iter().any(|l| !own_layers.contains(&l.id) && !l.locked)
    };
    let cmd = match (eye, has_other_active) {
        (true, true) => "layer.hideOthers",
        (true, false) => "layer.showAll",
        (false, true) => "layer.lockOthers",
        (false, false) => "layer.unlockAll",
    };
    let enabled = match (eye, has_other_active) {
        (true, true) | (false, true) => true,
        (true, false) => any_layer_state(doc, |n| !n.visible),
        (false, false) => any_layer_state(doc, |n| n.locked),
    };
    let params = if cmd.ends_with("Others") { json!({ "ids": ids }) } else { json!({}) };
    (cmd.into(), params, enabled)
}

fn any_layer_state(doc: &Document, pred: impl Fn(&Node) -> bool) -> bool {
    let mut found = false;
    doc.walk(|n| {
        if !found && n.is_layer() && pred(n) {
            found = true;
        }
    });
    found
}

/// Alt-clicking row `n`'s eye (`eye`) or lock: Hide Others or Lock Others, every top-level layer
/// but the one holding the row; when they all are hidden (locked) already, Show All Layers (Unlock
/// All Layers). Each is one undo step.
fn others_action(doc: &Document, n: &Node, eye: bool) -> (String, Value) {
    let own = doc.layer_of(n.id);
    let mut others = doc.layers.iter().filter(|l| Some(l.id) != own);
    let cmd = match (eye, if eye { others.any(|l| l.visible) } else { others.any(|l| !l.locked) }) {
        (true, true) => "layer.hideOthers",
        (true, false) => "layer.showAll",
        (false, true) => "layer.lockOthers",
        (false, false) => "layer.unlockAll",
    };
    // Show All and Unlock All take no rows; a ruler guide's row names its layer.
    let params = if cmd.ends_with("Others") { json!({ "ids": [layer_row(n, own).0] }) } else { json!({}) };
    (cmd.into(), params)
}

/// Ctrl/Cmd+Alt-clicking row `n`'s eye runs Outline Others, which puts every top-level layer but the
/// one holding the row in Outline and puts that one in Preview. When all the other layers are in
/// Outline already, it runs Preview All Layers. Each is one undo step.
fn outline_others_action(doc: &Document, n: &Node) -> (String, Value) {
    let own = doc.layer_of(n.id);
    if doc.layers.iter().any(|l| Some(l.id) != own && !matches!(l.kind, NodeKind::Layer { preview: false, .. })) {
        ("layer.outlineOthers".into(), json!({ "ids": [layer_row(n, own).0] }))
    } else {
        // Preview All takes no rows.
        ("layer.previewAll".into(), json!({}))
    }
}

/// The row the layer commands of an Alt-click on row `n`'s eye or lock name: `n` itself, or for a
/// ruler guide's row the layer `own` that holds the guide (the layer commands refuse a guide).
fn layer_row(n: &Node, own: Option<NodeId>) -> NodeId {
    if n.is_ruler_guide() { own.unwrap_or(n.id) } else { n.id }
}

/// Whether Locate Layer can highlight a row the panel lists: outside isolation mode, or with a
/// layer or sublayer isolated (`root`). An isolated group or object holds no layer row, and the
/// layer holding it is not listed.
pub(super) fn locate_layer_listed(doc: &Document, root: Option<NodeId>) -> bool {
    root.is_none_or(|r| doc.node(r).is_some_and(Node::is_layer))
}

/// Whether `n`'s target circle shows as targeted: `n` is the one target, or, with nothing
/// targeted, it is selected. A layer is never selected itself (its art is), so with nothing
/// targeted a layer shows as targeted while all of its visible, unlocked art is selected, as an
/// object added to the selection from its circle does (#8646).
pub(super) fn shows_targeted(tree: &DocLayers<'_>, n: &Node, is_sel: bool) -> bool {
    match tree.state().selection.target {
        Some(id) => id == n.id,
        None if n.is_layer() => tree.is_full(n.id),
        None => is_sel,
    }
}

/// The layers and sublayers of `doc` whose [`Node::layer_art`] with `editable` is not empty and
/// all in `sel`, found in one pass over the layers and their direct children.
pub(super) fn full_layers(doc: &Document, sel: &HashSet<NodeId>) -> HashSet<NodeId> {
    /// Adds `layer` to `out` when it is full, and returns its art's count and selected count.
    fn visit(layer: &Node, sel: &HashSet<NodeId>, out: &mut HashSet<NodeId>) -> (usize, usize) {
        let (mut art, mut selected) = (0, 0);
        for c in layer.children().into_iter().flatten() {
            let counts = !(c.is_ruler_guide() || !c.visible || c.locked || c.is_template());
            if c.is_layer() {
                let (a, s) = visit(c, sel, out);
                if counts {
                    art += a;
                    selected += s;
                }
            } else if counts {
                art += 1;
                selected += usize::from(sel.contains(&c.id));
            }
        }
        if art > 0 && selected == art {
            out.insert(layer.id);
        }
        (art, selected)
    }
    let mut out = HashSet::new();
    if !sel.is_empty() {
        for layer in doc.layers.iter().filter(|l| l.is_layer()) {
            visit(layer, sel, &mut out);
        }
    }
    out
}

/// The `layer.target` of a click with `mods` on the target circle of `id`, by the rules of a row click (#8639): plain
/// targets it, Ctrl/Cmd adds it to the selection or takes it out, Shift selects the art of every row `filter` lists
/// from `anchor` (the last circle clicked, else the targeted row) to it, and Shift+Ctrl/Cmd adds that art. With it,
/// whether the click moves the anchor to `id`: every click but a Shift range.
pub(super) fn target_click(tree: &DocLayers<'_>, filter: &LayerFilter, anchor: Option<NodeId>, id: NodeId, mods: Mods) -> ((String, Value), bool) {
    if mods.shift
        && let Some(range) = anchor.or(tree.state().selection.target).and_then(|start| listed_range(tree, filter, start, id))
    {
        let params = if mods.cmd { json!({"range": range, "add": true}) } else { json!({"range": range}) };
        return (("layer.target".into(), params), false);
    }
    let params = if mods.cmd { json!({"id": id.0, "add": true}) } else { json!({"id": id.0}) };
    (("layer.target".into(), params), true)
}

/// The `layer.selectAll` of a click with `mods` on the selection square of `id`. A plain click selects the row's art.
/// Shift selects the art of every row `filter` lists from `anchor` (the last square clicked) to it, and Shift+Ctrl/Cmd
/// adds that art to the selection. With no anchor listed, Shift adds the row's art or removes it when it is all
/// selected, and so does Ctrl/Cmd. With it, whether the click moves the anchor to `id`: every click but a Shift range.
pub(super) fn art_click(tree: &DocLayers<'_>, filter: &LayerFilter, anchor: Option<NodeId>, id: NodeId, mods: Mods) -> ((String, Value), bool) {
    if mods.shift
        && let Some(range) = anchor.and_then(|start| listed_range(tree, filter, start, id))
    {
        let params = if mods.cmd { json!({"range": range, "add": true}) } else { json!({"range": range}) };
        return (("layer.selectAll".into(), params), false);
    }
    let params = if mods.shift || mods.cmd { json!({"id": id.0, "add": true}) } else { json!({"id": id.0}) };
    (("layer.selectAll".into(), params), true)
}

/// The rows `filter` lists from `start` to `end`, both included, top-down; `None` when either is not listed.
fn listed_range(tree: &DocLayers<'_>, filter: &LayerFilter, start: NodeId, end: NodeId) -> Option<Vec<u64>> {
    let order = listed_nodes(tree, filter);
    let at = |k: NodeId| order.iter().position(|r| *r == k);
    let (i, j) = (at(start)?, at(end)?);
    Some(order.get(i.min(j)..=i.max(j)).unwrap_or_default().iter().map(|r| r.0).collect())
}

/// The layer, group and object rows the panel over `tree` lists under `filter`, top-down: the children of open rows,
/// and while the filter is on every row it keeps and every row holding one, open or not.
fn listed_nodes(tree: &DocLayers<'_>, filter: &LayerFilter) -> Vec<NodeId> {
    fn walk(tree: &DocLayers<'_>, filter: &LayerFilter, parent: Option<&RowId>, out: &mut Vec<NodeId>) -> bool {
        use craft::layers::ui::tree::LayerTree as _;
        let filtered = filter.active();
        let mut any = false;
        for row in tree.children(parent) {
            let at = out.len();
            let inner = if filtered || tree.is_open(&row) { walk(tree, filter, Some(&row), out) } else { false };
            let keep = !filtered || inner || tree.listed(&row, filter);
            if keep && let RowId::Node(id) = row {
                out.insert(at, id);
            }
            any |= keep;
        }
        any
    }
    let mut out = Vec::new();
    walk(tree, filter, None, &mut out);
    out
}

/// The radius of a target circle's outer ring.
pub(super) const TARGET_RADIUS: f32 = 5.0;
/// The radius of the inner ring of a targeted item's target circle (the double ring).
pub(super) const TARGET_RING: f32 = 2.8;
/// The radius of the outlined diamond marking an item's own appearance or transparency.
pub(super) const APPEARANCE_MARK: f32 = 2.5;
/// A mitered thin stroke extends a diamond's corner by width / sqrt(2). Radius 1 keeps
/// the diamond clear of the targeted ring's inner edge (#8711).
pub(super) const APPEARANCE_MARK_IN_RING: f32 = 1.0;

/// The inner target ring's radius and the own-appearance diamond's radius. Each of the four
/// combinations differs, and the diamond inside a targeted ring leaves that ring visible.
pub(super) fn target_marks(targeted: bool, styled: bool) -> (Option<f32>, Option<f32>) {
    let mark = if targeted { APPEARANCE_MARK_IN_RING } else { APPEARANCE_MARK };
    (targeted.then_some(TARGET_RING), styled.then_some(mark))
}

/// Whether an object's target circle shows its appearance mark: its appearance is not basic (a group or layer:
/// it has fills, strokes or effects of its own) or its transparency (opacity, blend mode,
/// isolation, knockout or an opacity mask) is not the default.
pub(super) fn has_styled_target(n: &Node) -> bool {
    !n.appearance.is_basic()
        || (matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && !n.appearance.items.is_empty())
        || !n.has_default_transparency()
}

/// Applies what the panel reported this pass, in order, each through its commands.
pub(super) fn apply(app: &mut VectorcraftApp, events: Vec<LayersEvent<RowId>>) {
    for event in events {
        match event {
            LayersEvent::Toggle(toggle) => apply_toggle(app, &toggle),
            LayersEvent::Open(change) => set_open(app, &change),
            // A row clicked in view.
            LayersEvent::Select(_) => super::unrevealed(app, |app| run_commands(app, event)),
            event => run_commands(app, event),
        }
    }
}

/// Runs the commands of one event other than a toggle sweep or an open.
fn run_commands(app: &mut VectorcraftApp, event: LayersEvent<RowId>) {
    let Some(st) = app.session.active() else { return };
    for (c, p) in commands(app, st, event) {
        app.run(&c, p).ok();
    }
}

/// The commands of one event other than a toggle sweep or an open.
fn commands(app: &VectorcraftApp, st: &DocState, event: LayersEvent<RowId>) -> Vec<(String, Value)> {
    match event {
        LayersEvent::Select(select) => select_commands(st, &select),
        LayersEvent::ShowOnly(LayerRowKey::Layer(row)) => {
            row_node(st, &row).filter(|_| !isolated(st)).map(|n| others_action(&st.doc, n, true)).into_iter().collect()
        }
        // The Show Only of the sweep that Escape took back was one undo step.
        LayersEvent::UndoShowOnly(_) if !isolated(st) => vec![("edit.undo".into(), json!({}))],
        LayersEvent::CellClick { row: LayerRowKey::Layer(row), column, mods, .. } => cell_click(st, &row, column, mods).into_iter().collect(),
        LayersEvent::Drop(EditEvent::Set { after: LayerDrop::Layers(mv), .. }) => drop_command(&DocLayers::new(app, st), &mv).into_iter().collect(),
        LayersEvent::HostDrop { payload, target: RowId::Node(target), position, copy, .. } => {
            art_drop(st, &payload, target, position, copy).into_iter().collect()
        }
        LayersEvent::Rename { layer: RowId::Node(id), text, .. } => vec![("layer.setProps".into(), json!({"ids": [id.0], "name": text}))],
        LayersEvent::Activate { row: LayerRowKey::Layer(RowId::Node(id)), on: ActivateOn::Row | ActivateOn::Thumbnail(_), .. } => {
            vec![("ui.layerOptions".into(), json!({"ids": [id.0]}))]
        }
        LayersEvent::Delete(rows) => delete_command(&rows).into_iter().collect(),
        LayersEvent::Menu { command, .. } => CommandRef::parse_key(&command).map(|c| (c.id, c.params)).into_iter().collect(),
        _ => vec![],
    }
}

/// Whether isolation mode is on: the other layers are outside the isolated art, so the Alt and
/// Ctrl/Cmd+Alt clicks that change them change nothing.
fn isolated(st: &DocState) -> bool {
    st.isolation.is_some_and(|i| st.doc.node(i).is_some())
}

/// The node of a row's eye and lock: the row's own, a ruler guide's node; a spine has none.
fn row_node<'a>(st: &'a DocState, row: &RowId) -> Option<&'a Node> {
    let id = match *row {
        RowId::Node(id) => id,
        RowId::Guide(i) => st.doc.ruler_guides().get(i).and_then(|g| g.id)?,
        RowId::Spine(_) => return None,
    };
    st.doc.node(id)
}

/// A row click or a selection key. A layer, group or object row highlights rows: one row alone
/// makes its layer current (`layer.setCurrent`) and lets go of the selected ruler guides, keeping
/// the selected art (#7340), and an Alt-click on a layer's or sublayer's row also selects its art,
/// as its selection square does. A ruler guide's row selects the guide and leaves no row
/// highlighted, so Delete then deletes the guide; a hidden or locked guide is not selected. A
/// spine's row selects its blend, unless the blend is hidden or locked. A click on the empty space
/// below the rows keeps the highlighted rows.
fn select_commands(st: &DocState, select: &LayerSelect<RowId>) -> Vec<(String, Value)> {
    let (rows, toggle) = match &select.change {
        SelectionChange::Set(rows) => (rows.clone(), false),
        SelectionChange::Toggle(row) => (vec![*row], true),
        _ => return vec![],
    };
    let mut out: Vec<(String, Value)> = vec![];
    let nodes: Vec<NodeId> = rows.iter().filter_map(|r| if let RowId::Node(id) = r { Some(*id) } else { None }).collect();
    let guides: Vec<usize> = rows.iter().filter_map(|r| if let RowId::Guide(i) = r { Some(*i) } else { None }).collect();
    let blends: Vec<u64> = rows.iter().filter_map(|r| if let RowId::Spine(b) = r { Some(b.0) } else { None }).collect();
    match nodes.as_slice() {
        [] if !guides.is_empty() => {
            let all = st.doc.ruler_guides();
            let editable: Vec<usize> = guides.into_iter().filter(|i| all.get(*i).is_some_and(|g| st.doc.guide_editable(g))).collect();
            if editable.is_empty() {
                return out;
            }
            // The highlighted rows are let go, so Delete then deletes the selected guide (Edit › Clear)
            // rather than those rows.
            if !st.highlighted_rows().is_empty() {
                out.push(("layer.highlight".into(), json!({"ids": [], "mode": "set"})));
            }
            out.push(("guide.select".into(), json!({"indexes": editable, "toggle": toggle})));
        }
        [] => {
            let editable = st.doc.editable();
            let blends: Vec<u64> = blends.into_iter().filter(|b| editable(NodeId(*b))).collect();
            if !blends.is_empty() {
                let action = if toggle { "select.toggle" } else { "select.set" };
                out.push((action.into(), json!({"ids": blends})));
            }
        }
        [id] if toggle => out.push(("layer.highlight".into(), json!({"ids": [id.0], "mode": "toggle"}))),
        [id] => {
            let selected_guides = vectorcraft_engine::cmd::guide_indexes(&st.doc, &st.selection.guides);
            if !selected_guides.is_empty() {
                // Toggled out of the selection, so the selected art stays.
                out.push(("guide.select".into(), json!({"indexes": selected_guides, "toggle": true})));
            }
            out.push(("layer.setCurrent".into(), json!({"id": id.0})));
            let mods = select.mods;
            if mods.alt && !mods.shift && !mods.cmd && st.doc.node(*id).is_some_and(|n| n.is_layer()) {
                out.push(("layer.selectAll".into(), json!({"id": id.0})));
            }
        }
        ids => out.push(("layer.highlight".into(), json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "mode": "set"}))),
    }
    out
}

/// A press on an eye or lock cell that is the app's, not a toggle: Ctrl/Cmd on an eye switches a
/// layer between Preview and Outline (any other row's eye toggles), Ctrl/Cmd+Alt on an eye is
/// Outline Others, Alt on a lock is Lock Others; the last two change nothing in isolation mode.
fn cell_click(st: &DocState, row: &RowId, column: LayerColumn, mods: Mods) -> Option<(String, Value)> {
    let n = row_node(st, row)?;
    match column {
        LayerColumn::Visibility if mods.cmd && mods.alt => (!isolated(st)).then(|| outline_others_action(&st.doc, n)),
        LayerColumn::Visibility if mods.cmd && n.is_layer() => {
            let preview = !matches!(n.kind, NodeKind::Layer { preview: false, .. });
            Some(("layer.setProps".into(), json!({"ids": [n.id.0], "preview": !preview})))
        }
        LayerColumn::Visibility if mods.cmd => Some(("layer.setProps".into(), json!({"ids": [n.id.0], "visible": !n.visible}))),
        LayerColumn::Lock if mods.alt => (!isolated(st)).then(|| others_action(&st.doc, n, false)),
        _ => None,
    }
}

/// A sweep down the eye or lock column: live while it runs, one undo step at its release, taken
/// back by Escape. A ruler guide's row sets its guide alone.
fn apply_toggle(app: &mut VectorcraftApp, toggle: &LayerToggle<RowId>) {
    let Some(st) = app.session.active() else { return };
    let ids: Vec<u64> = toggle
        .rows
        .iter()
        .filter_map(|key| match key {
            LayerRowKey::Layer(row) => row_node(st, row).map(|n| n.id.0),
            _ => None,
        })
        .collect();
    let (prop, label) = match toggle.column {
        LayerColumn::Visibility => ("visible", "Visibility"),
        _ => ("locked", "Lock"),
    };
    apply_event(app, &toggle.edit, &EditPath::Preview, &mut |app: &mut VectorcraftApp, on: &bool, phase: Live| {
        crate::panels::live_run(app, label, "layer.setProps", json!({"ids": ids, prop: on}), phase);
    });
}

/// A row opened or closed; Alt opens or closes every row inside it too. A view change: no undo step.
fn set_open(app: &mut VectorcraftApp, change: &OpenChange<RowId>) {
    let RowId::Node(id) = change.key else { return };
    let opts = &app.ui.layers_panel;
    let Some(st) = app.session.active_mut() else { return };
    st.layers_open.set(id, change.open);
    if change.all {
        let doc = st.doc.clone();
        if let Some(n) = doc.node(id) {
            n.walk(&mut |c| {
                if c.id != id && opens(c, opts) {
                    st.layers_open.set(c.id, change.open);
                }
            });
        }
    }
}

/// Where a place lands as `layer.move` names it.
fn place(position: LayerPosition) -> &'static str {
    match position {
        LayerPosition::Before => "above",
        LayerPosition::After => "below",
        _ => "inside",
    }
}

/// Rows dropped above, below or into a row (Alt copies): layers, groups and objects move there
/// (`layer.move`); ruler guides go to the layer of the row, its own for a layer (`guide.setLayer`).
/// A place beside a ruler guide's row is at the top of its layer's art; a place just above a
/// spine's row is below the blend's last key object. Rows dropped in the empty space under the last
/// row arrive as a place after the bottom layer (`layer_drop` in craft-layers' `panel/drag.rs`); the
/// engine's `layer.move` takes the bottom layer there only as a copy (#8237).
fn drop_command(tree: &DocLayers<'_>, mv: &LayerMove<RowId>) -> Option<(String, Value)> {
    // Only where the list takes the rows (in isolation mode, inside the isolated art): the group they
    // land in is the target, or the one holding it.
    let parent = match mv.position {
        LayerPosition::Into => Some(mv.target),
        _ => tree.parent(&mv.target),
    };
    let keys: Vec<LayerRowKey<RowId>> = mv.layers.iter().map(|row| LayerRowKey::Layer(*row)).collect();
    let landing = ReorderMove { key: keys.first()?.clone(), keys, to: 0, copy: mv.copy, parent: parent.map(LayerRowKey::Layer) };
    if !tree.accepts_drop(&landing, Mods::NONE) {
        return None;
    }
    let doc = tree.doc();
    let (target, position) = match mv.target {
        RowId::Node(id) => (id, mv.position),
        RowId::Guide(i) => (doc.ruler_guides().get(i).and_then(|g| g.layer)?, LayerPosition::Into),
        RowId::Spine(blend) if mv.position == LayerPosition::Before => {
            (doc.node(blend).and_then(|b| b.children()).and_then(|c| c.first()).map(|c| c.id)?, LayerPosition::After)
        }
        RowId::Spine(_) => return None,
    };
    let guides: Vec<usize> = mv.layers.iter().filter_map(|r| if let RowId::Guide(i) = r { Some(*i) } else { None }).collect();
    if !guides.is_empty() {
        let n = doc.node(target)?;
        let layer = if n.is_layer() { target } else { doc.layer_containing(target)? };
        return (guides.len() == mv.layers.len()).then(|| ("guide.setLayer".into(), json!({"indexes": guides, "layer": layer.0})));
    }
    let ids: Vec<u64> = mv.layers.iter().filter_map(|r| if let RowId::Node(id) = r { Some(id.0) } else { None }).collect();
    (ids.len() == mv.layers.len()).then(|| ("layer.move".into(), json!({"ids": ids, "target": target.0, "place": place(position), "copy": mv.copy})))
}

/// The selected art carried by a row's selection square, dropped on `target`: the selection's top
/// objects, as Arrange and Group take them, move there (Alt copies them).
fn art_drop(st: &DocState, payload: &DropPayload, target: NodeId, position: LayerPosition, copy: bool) -> Option<(String, Value)> {
    let art = payload.get::<ArtDrag>().filter(|art| art.document == st.uid)?;
    let ids: Vec<u64> = st.doc.paint_order(art.ids.iter().copied()).iter().map(|i| i.0).collect();
    Some(("layer.move".into(), json!({"ids": ids, "target": target.0, "place": place(position), "copy": copy})))
}

/// Delete or Backspace in the rows: the layer, group and object rows (`ui.layerDelete`, which asks
/// first when a layer among them holds art), else the ruler guides.
fn delete_command(rows: &[LayerRowKey<RowId>]) -> Option<(String, Value)> {
    let row_of = |key: &LayerRowKey<RowId>| if let LayerRowKey::Layer(row) = key { Some(*row) } else { None };
    let ids: Vec<u64> = rows.iter().filter_map(row_of).filter_map(|r| if let RowId::Node(id) = r { Some(id.0) } else { None }).collect();
    if !ids.is_empty() {
        return Some(("ui.layerDelete".into(), json!({"ids": ids})));
    }
    let guides: Vec<usize> = rows.iter().filter_map(row_of).filter_map(|r| if let RowId::Guide(i) = r { Some(i) } else { None }).collect();
    (!guides.is_empty()).then(|| ("guide.remove".into(), json!({"indexes": guides})))
}
