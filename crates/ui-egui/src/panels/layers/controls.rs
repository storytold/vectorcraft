use craft::behavior::IconId;
use craft::keymap::Mods;
use craft::layers::ui::panel::MenuTarget; // PLAN: assumed path of craft-layers' panel types
use craft::layers::ui::tree::LayerRowKey;
use craft::ui::contract::{Entry, ListDrag, Show as _};
use craft::ui::display::{Elide, Text};
use craft::ui::menu::{MenuRow, PanelRow};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::sections::{FooterAction, FooterBar};

// PLAN: assumed document-panels-w7's `model` exports `DocLayers` (accessors used here: `doc() -> &Arc<Document>`,
// `state() -> &DocState`, `root() -> Option<NodeId>`, `layers_only() -> bool`, `guides() -> &[usize]`,
// `targets(&MenuTarget<RowId>) -> Vec<u64>`: the ids a menu command acts on) and `RowId::{Node(NodeId), Guide(usize), Spine(NodeId)}`.
use super::model::{DocLayers, FilterObject, FilterState, RowId, names, painted_name, parse_names};
use super::{Arc, HashSet, Node, NodeId, NodeKind, Ui, Value, VectorcraftApp, json};
use crate::panel_drag::PanelDrag;

/// The command of a Save Selection row that recalls one of the document's saved selections.
const RECALL: &str = "ui.layers.recall";

// The footer's actions, in the order given to the bar.
const DELETE: usize = 0;
const NEW_LAYER: usize = 1;
const NEW_SUBLAYER: usize = 2;
const MASK: usize = 3;
pub(super) const LOCATE: usize = 4;

/// One row of the panel menu with the state its command has.
enum Item {
    Separator,
    Command(CommandRef, CommandState),
    Submenu(String, Vec<Item>),
}

impl Item {
    /// The row the dock's panel menu shows for it.
    fn row(&self) -> PanelRow {
        match self {
            Item::Separator => PanelRow::Separator,
            Item::Command(command, _) => PanelRow::Command(command.clone()),
            Item::Submenu(label, items) => PanelRow::Submenu(label.clone().into(), items.iter().map(Item::row).collect()),
        }
    }

    /// Give each command of it and of its submenus its state.
    fn set_states(&self, states: &mut CommandStates) {
        match self {
            Item::Separator => {}
            Item::Command(command, state) => states.set(command.clone(), state.clone()),
            Item::Submenu(_, items) => items.iter().for_each(|item| item.set_states(states)),
        }
    }
}

/// A row running `id` with `params`, shown as `title`.
fn command(id: &str, params: Value, title: &str, enabled: bool) -> Item {
    Item::Command(CommandRef::with_params(id, params), CommandState::new().title(title).enabled(enabled))
}

/// A row running `id` with `params`, shown as `title` and checked when `checked`.
fn toggle(id: &str, params: Value, title: &str, checked: bool) -> Item {
    Item::Command(CommandRef::with_params(id, params), CommandState::new().title(title).checked(checked))
}

/// The rows of the Layers panel menu with their states: what `menu_rows` shows and `menu_states` answers.
fn items(app: &VectorcraftApp) -> Vec<Item> {
    let Some(st) = app.session.active() else { return vec![] };
    let doc = &st.doc;
    let rows = listed_highlighted_rows(st);
    let current = st.current_layer();
    let has_sel = !st.selection.is_empty();
    let isolated = st.isolation.is_some();
    let layers_only = app.ui.layers_panel.layers_only;
    let targets: Vec<NodeId> = if rows.is_empty() { listed_current_layer(st).into_iter().collect() } else { rows.clone() };
    let ids: Vec<u64> = targets.iter().map(|i| i.0).collect();
    let (new_layer_ok, new_sublayer_ok) = (!new_layer_unlisted(st, false, false), !new_layer_unlisted(st, true, false));
    // A copy of the isolated node goes beside it, outside the isolated art.
    let duplicate_ok = !targets.is_empty() && !st.isolation.is_some_and(|r| targets.contains(&r));
    let one_container = match targets.as_slice() {
        [id] => doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Layer { .. } | NodeKind::Group { .. })),
        _ => false,
    };
    let layers_in: Vec<&Node> = targets.iter().filter_map(|id| doc.node(*id)).filter(|n| n.is_layer()).collect();
    let template = layers_in.first().is_some_and(|l| l.is_template());
    // The top-level layers holding the rows, and the others.
    let kept: HashSet<NodeId> = targets.iter().filter_map(|id| doc.layer_of(*id)).collect();
    let others: Vec<&Node> = doc.layers.iter().map(|l| &**l).filter(|l| !kept.contains(&l.id)).collect();
    let mut all_layers = vec![];
    doc.walk(|n| {
        if n.is_layer() {
            all_layers.push(n);
        }
    });
    let export = |multiple: bool| {
        let mut p = json!({ "multiple": multiple });
        if !ids.is_empty() {
            p["ids"] = json!(ids);
        }
        p
    };
    let export_ok = !targets.is_empty() || has_sel;
    let mut items = vec![
        command("ui.newLayer", json!({}), tl!("New Layer…"), new_layer_ok),
        command("ui.newLayer", json!({"sublayer": true}), tl!("New Sublayer…"), current.is_some() && new_sublayer_ok),
        // The rows listed, so no highlighted row outside the isolated art is copied.
        command("layer.duplicate", json!({ "ids": ids }), tl!("Duplicate Selection"), duplicate_ok),
        command("ui.layerDelete", json!({}), tl!("Delete Selection"), !targets.is_empty()),
        command("ui.layerOptions", json!({}), tl!("Options for Selection…"), !targets.is_empty()),
        Item::Separator,
        command("layer.clippingMask.toggle", json!({}), tl!("Make/Release Clipping Mask"), true),
        Item::Separator,
        if isolated {
            command("object.exitIsolation", json!({}), tl!("Exit Isolation Mode"), true)
        } else {
            command("object.isolate", json!({ "id": targets.first().map(|i| i.0) }), tl!("Enter Isolation Mode"), one_container)
        },
        Item::Separator,
        if layers_only {
            command("layer.locate", json!({"layersOnly": true}), tl!("Locate Layer"), has_sel && super::rows::locate_layer_listed(doc, st.isolation))
        } else {
            command("layer.locate", json!({}), tl!("Locate Object"), has_sel)
        },
        command("layer.selectHighlighted", json!({}), tl!("Select Art of Highlighted Rows"), !rows.is_empty()),
        save_selection(app),
        Item::Separator,
        command("layer.merge", json!({}), tl!("Merge Selected"), rows.len() >= 2),
        command("layer.flatten", json!({}), tl!("Flatten Artwork"), true),
        command("layer.collectInNew", json!({}), tl!("Collect in New Layer"), !rows.is_empty() || has_sel),
        Item::Submenu(
            tl!("Collect for Export").to_string(),
            vec![
                command("assets.add", export(false), "As Single Asset", export_ok),
                command("assets.add", export(true), "As Multiple Assets", export_ok),
            ],
        ),
        command("layer.releaseToLayers", json!({}), tl!("Release to Layers (Sequence)"), one_container),
        command("layer.releaseToLayersBuild", json!({}), tl!("Release to Layers (Build)"), one_container),
        command("layer.reverse", json!({}), tl!("Reverse Order"), rows.len() >= 2),
        Item::Separator,
        toggle_or_enabled_template(!layers_in.is_empty(), template),
    ];
    // Each pair reads by state: Hide Others while another layer shows, else Show All Layers. In
    // isolation mode the other layers are outside the isolated art, so the pairs are off.
    let any_other = |f: &dyn Fn(&Node) -> bool| others.iter().any(|l| f(l));
    let outlined = |l: &Node| matches!(l.kind, NodeKind::Layer { preview: false, .. });
    items.push(if any_other(&|l| l.visible) {
        command("layer.hideOthers", json!({}), tl!("Hide Others"), !isolated)
    } else {
        command("layer.showAll", json!({}), tl!("Show All Layers"), !isolated && all_layers.iter().any(|l| !l.visible))
    });
    items.push(if any_other(&|l| !outlined(l)) {
        command("layer.outlineOthers", json!({}), tl!("Outline Others"), !isolated)
    } else {
        command("layer.previewAll", json!({}), tl!("Preview All Layers"), !isolated && all_layers.iter().any(|l| outlined(l)))
    });
    items.push(if any_other(&|l| !l.locked) {
        command("layer.lockOthers", json!({}), tl!("Lock Others"), !isolated)
    } else {
        command("layer.unlockAll", json!({}), tl!("Unlock All Layers"), !isolated && all_layers.iter().any(|l| l.locked))
    });
    let remembers = doc.paste_remembers_layers;
    items.extend([
        Item::Separator,
        toggle("layer.pasteRemembersLayers", json!({"on": !remembers}), tl!("Paste Remembers Layers"), remembers),
        Item::Separator,
        command("ui.layersPanelOptions", json!({}), tl!("Panel Options…"), true),
    ]);
    items
}

/// The Template row: enabled with a layer among the targets, checked while the first one is one.
fn toggle_or_enabled_template(enabled: bool, checked: bool) -> Item {
    let state = CommandState::new().title(&*tl!("Template")).enabled(enabled).checked(checked);
    Item::Command(CommandRef::with_params("layer.template", json!({})), state)
}

/// Save Selection › Save Selection…, Update Selection and Edit Selection…, enabled as in the Select
/// menu, then the active document's saved selections: choosing one recalls it.
fn save_selection(app: &VectorcraftApp) -> Item {
    let mut rows: Vec<Item> = [("Save Selection…", "select.save"), ("Update Selection", "select.update"), ("Edit Selection…", "select.editSaved")]
        .into_iter()
        .map(|(label, id)| command(id, json!({}), label, crate::menus::enabled(app, id)))
        .collect();
    let names: Vec<String> = app.session.active().map(|d| d.doc.saved_selections.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
    if !names.is_empty() {
        rows.push(Item::Separator);
    }
    rows.extend(names.into_iter().map(|name| command(RECALL, json!({ "name": name }), &name, true)));
    Item::Submenu(tl!("Save Selection").to_string(), rows)
}

/// The rows of the Layers panel's (≡) menu.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    items(app).iter().map(Item::row).collect()
}

/// The states of the menu's commands (enabled, checked and the titles that follow the state), for
/// the frame's one set of command states.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    items(app).iter().for_each(|item| item.set_states(states));
}

/// Run this panel's own menu command: `ui.layers.recall {name}` recalls a saved selection and
/// answers `None` for any other id. The panel's own command reveals nothing for the selection it
/// makes ([`super::unrevealed`]).
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != RECALL {
        return None;
    }
    let Some(name) = params.get("name").and_then(Value::as_str) else { return Some(Err("`name` must be a name".into())) };
    Some(super::unrevealed(app, |app| app.run("select.recall", json!({ "name": name }))))
}

/// An entry of a row menu whose pick is the command `id` with `params`.
fn entry(id: &str, params: Value, label: impl Into<String>, enabled: bool) -> MenuRow<String> {
    Entry::new(CommandRef::with_params(id, params).key(), label.into()).enabled(enabled).into()
}

/// A separator of a row menu.
fn separator() -> MenuRow<String> {
    Entry::<String>::separator().into()
}

/// A row's name cut to 20 characters, as the Duplicate and Delete entries show it.
fn short_name(n: &Node) -> String {
    let raw = painted_name(n, &n.display_name(), crate::i18n::current());
    if raw.chars().count() > 20 { format!("{}…", raw.chars().take(20).collect::<String>()) } else { raw }
}

/// The entries of the menu on a row, with each pick's command as its value. It acts on the
/// highlighted rows when the row is one of them. The panel adds its own Label Color submenu.
pub(crate) fn row_menu(tree: &DocLayers, target: &MenuTarget<RowId>) -> Vec<MenuRow<String>> {
    let Some(LayerRowKey::Layer(row)) = &target.row else { return vec![] };
    match row {
        RowId::Guide(i) => guide_menu(tree, *i),
        RowId::Spine(blend) => spine_menu(tree, *blend),
        RowId::Node(id) => object_menu(tree, *id, &tree.targets(target)),
    }
}

/// The entries of a layer, sublayer, group or object row `id`; `ids` are the rows they act on.
fn object_menu(tree: &DocLayers, id: NodeId, ids: &[u64]) -> Vec<MenuRow<String>> {
    let doc = tree.doc();
    let Some(n) = doc.node(id) else { return vec![] };
    let multi = ids.len() > 1;
    if n.is_guide() {
        let any = |f: fn(&Node) -> bool| ids.iter().filter_map(|&id| doc.node(NodeId(id))).any(f);
        let (unlocked, visible) = (any(|n| !n.locked), any(|n| n.visible));
        return vec![
            entry("view.guides.release", json!({ "ids": ids }), if multi { "Release Guides" } else { "Release Guide" }, true),
            entry("layer.duplicate", json!({ "ids": ids }), if multi { "Duplicate Selection" } else { "Duplicate" }, true),
            entry("layer.delete", json!({ "ids": ids }), if multi { "Delete Guides" } else { "Delete Guide" }, true),
            separator(),
            entry("layer.setProps", json!({ "ids": ids, "locked": unlocked }), if unlocked { "Lock" } else { "Unlock" }, true),
            entry("layer.setProps", json!({ "ids": ids, "visible": !visible }), if visible { "Hide" } else { "Show" }, true),
        ];
    }
    let options = if multi {
        "Options for Selection…"
    } else if n.is_layer() {
        "Layer Options…"
    } else {
        "Options…"
    };
    let name = short_name(n);
    let duplicate = if multi { tl!("Duplicate Selection").to_string() } else { format!("{} \"{name}\"", tl!("Duplicate")) };
    let delete = if multi { tl!("Delete Selection").to_string() } else { format!("{} \"{name}\"", tl!("Delete")) };
    let listed: HashSet<NodeId> = ids.iter().map(|&id| NodeId(id)).collect();
    // A copy of the isolated node goes beside it, outside the isolated art.
    let duplicate_ok = !tree.root().is_some_and(|r| ids.contains(&r.0));
    let delete_ok = !doc.layers.iter().all(|l| listed.contains(&l.id));
    // In isolation mode the other layers are outside the isolated art, so these pairs are off.
    let others_ok = tree.root().is_none();
    let (lock, lock_params, lock_ok) = super::rows::multi_others_action(doc, ids, false);
    let (hide, hide_params, hide_ok) = super::rows::multi_others_action(doc, ids, true);
    let lock_label = if lock == "layer.lockOthers" { "Lock Others" } else { "Unlock All Layers" };
    let hide_label = if hide == "layer.hideOthers" { "Hide Others" } else { "Show All Layers" };
    let export = |multiple: bool| json!({ "ids": ids, "multiple": multiple });
    vec![
        entry("ui.layerOptions", json!({ "ids": ids }), options, true),
        entry("layer.duplicate", json!({ "ids": ids }), duplicate, duplicate_ok),
        entry("layer.delete", json!({ "ids": ids }), delete, delete_ok),
        separator(),
        entry(&lock, lock_params, lock_label, lock_ok && others_ok),
        entry(&hide, hide_params, hide_label, hide_ok && others_ok),
        separator(),
        locate_entry(tree, Some(id.0)),
        separator(),
        MenuRow::Submenu(
            tl!("Collect for Export").to_string().into(),
            vec![entry("assets.add", export(false), "As Single Asset", true), entry("assets.add", export(true), "As Multiple Assets", true)],
        ),
    ]
}

/// Locate Object, or Locate Layer with Show Layers Only on, for row `id`.
fn locate_entry(tree: &DocLayers, id: Option<u64>) -> MenuRow<String> {
    let layers_only = tree.layers_only();
    let ok = !layers_only || super::rows::locate_layer_listed(tree.doc(), tree.root());
    entry("layer.locate", json!({ "id": id, "layersOnly": layers_only }), if layers_only { "Locate Layer" } else { "Locate Object" }, ok)
}

/// The entries of ruler guide row `i`: they act on the selected guides when it is one of them.
fn guide_menu(tree: &DocLayers, i: usize) -> Vec<MenuRow<String>> {
    let doc = tree.doc();
    let guides = doc.ruler_guides();
    let Some(g) = guides.get(i) else { return vec![] };
    let selected = tree.guides().contains(&i);
    let editable = doc.guide_editable(g);
    let acts: Vec<usize> = if selected { tree.guides().to_vec() } else { vec![i] };
    let nodes: Vec<u64> = acts.iter().filter_map(|&idx| guides.get(idx)?.id.map(|id| id.0)).collect();
    let node = g.id.and_then(|id| doc.node(id));
    let (visible, locked) = (node.is_none_or(|n| n.visible), node.is_some_and(|n| n.locked));
    let release = if acts.len() > 1 { json!({}) } else { json!({ "index": i }) };
    vec![
        entry("layer.setProps", json!({ "ids": nodes, "visible": !visible }), if visible { "Hide" } else { "Show" }, !nodes.is_empty()),
        entry("layer.setProps", json!({ "ids": nodes, "locked": !locked }), if locked { "Unlock" } else { "Lock" }, !nodes.is_empty()),
        separator(),
        entry("view.guides.release", release, "Release Guides", editable),
        entry("guide.remove", json!({ "indexes": acts }), "Delete", editable),
        separator(),
        locate_entry(tree, g.id.map(|id| id.0)),
    ]
}

/// The entries of the spine row of blend `blend`. They act on the selection, which the row's press
/// makes the blend (a click selects it), as the Object menu's blend commands do.
// PLAN: assumed document-panels-w7 selects the blend on the row's secondary press: a menu pick is one command, so it cannot select first.
fn spine_menu(tree: &DocLayers, blend: NodeId) -> Vec<MenuRow<String>> {
    let doc = tree.doc();
    let editable = (doc.editable())(blend);
    let has_spine = !matches!(doc.node(blend).map(|n| &n.kind), Some(NodeKind::Blend { spec, .. }) if spec.no_spine);
    vec![
        entry("select.set", json!({ "ids": [blend.0] }), "Select", editable),
        entry("ui.blendOptions", json!({}), "Blend Options…", editable),
        separator(),
        entry("object.blend.reverseSpine", json!({}), "Reverse Spine", editable && has_spine),
        entry("object.blend.reverseFrontToBack", json!({}), "Reverse Front to Back", editable),
        entry("object.blend.expand", json!({}), "Expand", editable),
        entry("object.blend.release", json!({}), "Release", editable),
        separator(),
        locate_entry(tree, Some(blend.0)),
    ]
}

/// What was dropped on a footer action.
pub(super) enum FooterPayload<'a> {
    /// Rows of the panel.
    Rows(&'a [LayerRowKey<RowId>]),
    /// The target circle of an appearance (its node id).
    Appearance(u64),
}

/// The node ids and the ruler guide indexes among `keys`.
fn row_ids(keys: &[LayerRowKey<RowId>]) -> (Vec<u64>, Vec<usize>) {
    let (mut nodes, mut guides) = (vec![], vec![]);
    for key in keys {
        match key {
            LayerRowKey::Layer(RowId::Node(id) | RowId::Spine(id)) => nodes.push(id.0),
            LayerRowKey::Layer(RowId::Guide(i)) => guides.push(*i),
            _ => {}
        }
    }
    (nodes, guides)
}

/// The command a press of footer action `index` runs, with the modifiers held at the press. Delete
/// with no row highlighted deletes the selected art and ruler guides. New Layer: Alt asks for its
/// options first, Ctrl/Cmd puts it on top of every layer, Ctrl/Cmd+Alt directly below the current
/// layer. New Sublayer: Alt asks for its options first, Ctrl/Cmd+Alt puts it under the layer's contents.
pub(super) fn footer_pick(tree: &DocLayers, index: usize, mods: Mods) -> Option<(String, Value)> {
    let (id, params) = match index {
        DELETE => {
            let st = tree.state();
            if listed_highlighted_rows(st).is_empty() && st.selection.has_objects_or_guides() {
                ("edit.clear", json!({}))
            } else {
                ("ui.layerDelete", json!({}))
            }
        }
        NEW_LAYER => match (mods.alt, mods.cmd) {
            (true, true) => ("layer.new", json!({"below": true})),
            (true, false) => ("ui.newLayer", json!({})),
            _ => ("layer.new", json!({"top": mods.cmd})),
        },
        NEW_SUBLAYER => match (mods.alt, mods.cmd) {
            (true, true) => ("layer.newSublayer", json!({"bottom": true})),
            (true, false) => ("ui.newLayer", json!({"sublayer": true})),
            _ => ("layer.newSublayer", json!({})),
        },
        MASK => ("layer.clippingMask.toggle", json!({})),
        LOCATE => ("layer.locate", json!({ "layersOnly": tree.layers_only() })),
        _ => return None,
    };
    Some((id.into(), params))
}

/// The command for `drop` dropped on footer action `index`: rows on Delete delete them (the node
/// rows, else the ruler guides), rows on New Layer duplicate them (not the isolated node, not
/// guides), an appearance's target circle on Delete reduces it to one fill and one stroke.
pub(super) fn footer_drop(tree: &DocLayers, index: usize, drop: FooterPayload<'_>) -> Option<(String, Value)> {
    match (index, drop) {
        (DELETE, FooterPayload::Rows(keys)) => {
            let (nodes, guides) = row_ids(keys);
            if !nodes.is_empty() {
                Some(("ui.layerDelete".into(), json!({ "ids": nodes })))
            } else if !guides.is_empty() {
                Some(("guide.remove".into(), json!({ "indexes": guides })))
            } else {
                None
            }
        }
        (NEW_LAYER, FooterPayload::Rows(keys)) => {
            let (nodes, _) = row_ids(keys);
            let isolated = tree.root().is_some_and(|r| nodes.contains(&r.0));
            (!nodes.is_empty() && !isolated).then(|| ("layer.duplicate".into(), json!({ "ids": nodes })))
        }
        (DELETE, FooterPayload::Appearance(id)) => Some(("appearance.reduceToBasic".into(), json!({ "ids": [id] }))),
        _ => None,
    }
}

/// The panel's footer: the layer count and the buttons. A press or a drop adds its command to `out`.
pub(crate) fn footer(tree: &DocLayers, ui: &mut Ui, out: &mut Vec<(String, Value)>) {
    let st = tree.state();
    let mods = ui.input(|i| i.modifiers);
    // In isolation mode New Layer and New Sublayer are off while the layer they make would be
    // outside the isolated art, where the panel does not list it.
    let new_layer_ok = !new_layer_unlisted(st, false, mods.command && !mods.alt);
    let new_sublayer_ok = !new_layer_unlisted(st, true, false);
    let layers_only = tree.layers_only();
    let locate_ok = !layers_only || super::rows::locate_layer_listed(tree.doc(), tree.root());
    let root = tree.root();
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    let layer_rows = |keys: &[LayerRowKey<RowId>]| keys.iter().all(|key| matches!(key, LayerRowKey::Layer(_)));
    let actions = [
        // PLAN: the framework's `accepts_drops` takes one payload type, so Delete cannot also take
        // `PanelDrag` (the appearance drop `footer_drop` maps); it needs a second payload there.
        FooterAction::new(icon("trash-2"), tl!("Delete Selection").to_string())
            .takes_drops_disabled(true)
            .accepts_drops(move |d: &ListDrag<LayerRowKey<RowId>>| layer_rows(&d.keys)),
        FooterAction::new(icon("file-plus"), tl!("Create New Layer").to_string()).enabled(new_layer_ok).accepts_drops(
            move |d: &ListDrag<LayerRowKey<RowId>>| {
                let (nodes, guides) = row_ids(&d.keys);
                layer_rows(&d.keys) && guides.is_empty() && !root.is_some_and(|r| nodes.contains(&r.0))
            },
        ),
        FooterAction::new(icon("plus"), tl!("Create New Sublayer").to_string()).enabled(new_sublayer_ok),
        FooterAction::new(icon("frame"), tl!("Make/Release Clipping Mask").to_string()),
        FooterAction::new(icon("search"), if layers_only { tl!("Locate Layer").to_string() } else { tl!("Locate Object").to_string() })
            .enabled(locate_ok),
    ];
    let count = crate::i18n::tn(tree.doc().layers.len() as u64, "{n} Layer", "{n} Layers").to_string();
    // The count is a unit of the footer's row, centred on its buttons' line in the text ink.
    let (output, _) = FooterBar::actions(actions).at_start(0).show_footer_leading(ui, |ui, _| {
        Text::new(count.clone()).elide(Elide::End).show(ui);
    });
    // The footer runs twice per pass: the sizing pass measures it and must report nothing.
    if ui.is_sizing_pass() {
        return;
    }
    out.extend(output.picked.and_then(|index| footer_pick(tree, index, output.mods)));
    if let Some(drop) = &output.dropped {
        let run = if let Some(d) = drop.payload::<ListDrag<LayerRowKey<RowId>>>() {
            footer_drop(tree, drop.index, FooterPayload::Rows(&d.keys))
        } else if let Some(PanelDrag::Appearance { document, source }) = drop.payload::<PanelDrag>()
            && *document == st.uid
        {
            footer_drop(tree, drop.index, FooterPayload::Appearance(source.0))
        } else {
            None
        };
        out.extend(run);
    }
}

/// `ui.layerDelete {ids?|id?, doc?, confirmed?}`: Delete Selection as the Layers panel runs it (the
/// trash button, rows dropped on it, the panel menu, Delete and Backspace), on the rows
/// `layer.delete` takes (default: the highlighted rows, else the current layer; in isolation mode a
/// current layer outside the isolated art is an error). When a layer or sublayer among them holds art (an object other than a
/// guide, hidden, locked and nested ones included), it asks first: OK deletes those rows, by id, in
/// the same document (`doc`, its uid, with `confirmed`); with another document active then, nothing
/// is deleted. In isolation mode a row outside the isolated art is an error, and nothing is deleted. Other rows, and rows `layer.delete` refuses (an unknown id, every top-level layer),
/// go to `layer.delete` at once. → `layer.delete`'s result, or null while the question is open
pub fn delete(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    super::unrevealed(app, |app| delete_rows(app, p))
}

/// [`delete`], which locates nothing.
fn delete_rows(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let confirmed = crate::menus::opt_bool(p, "confirmed")?.unwrap_or(false);
    let key = "doc";
    let doc = match p.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| format!("`{key}` must be a whole number of 0 or more"))?),
    };
    let st = app.session.active().ok_or("no document open")?;
    if doc.is_some_and(|d| d != st.uid) {
        return Err("the active document changed while the question was open; nothing was deleted".into());
    }
    let rows: serde_json::Map<String, Value> = ["ids", "id"].iter().filter_map(|k| Some((k.to_string(), p.get(*k)?.clone()))).collect();
    // An id the engine refuses: `layer.delete` gives the error, and deletes nothing.
    let row = |v: &Value| v.as_u64().map(NodeId).filter(|id| st.doc.node(*id).is_some_and(|n| !n.is_ruler_guide()));
    let ids: Option<Vec<NodeId>> = match (p.get("ids"), p.get("id")) {
        (Some(v), _) => v.as_array().and_then(|a| a.iter().map(row).collect()),
        (None, Some(v)) => row(v).map(|id| vec![id]),
        (None, None) => {
            let rows = listed_highlighted_rows(st);
            if rows.is_empty() && st.current_layer().is_some() && listed_current_layer(st).is_none() {
                return Err("the object is outside the isolated group".into());
            }
            Some(if rows.is_empty() { st.current_layer().into_iter().collect() } else { rows })
        }
    };
    // Isolation mode can start while rows are dragged to the trash or while the question is open:
    // nothing is deleted when a row is outside the isolated art.
    if ids.as_ref().is_some_and(|ids| ids.iter().any(|id| !in_isolated_art(st, *id))) {
        return Err("the object is outside the isolated group".into());
    }
    let ids = match ids {
        // Confirmed without rows given: the rows the panel lists, not every highlighted row.
        Some(ids) if confirmed && rows.is_empty() => return app.run("layer.delete", json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })),
        Some(ids) if !confirmed => ids,
        _ => return app.run("layer.delete", Value::Object(rows)),
    };
    let listed: HashSet<NodeId> = ids.iter().copied().collect();
    let mut roots = vec![];
    row_roots(&st.doc.layers, &listed, &mut roots);
    let every_layer = st.doc.layers.iter().all(|l| listed.contains(&l.id));
    if every_layer || !roots.iter().any(|n| n.is_layer() && holds_art(n)) {
        return app.run("layer.delete", json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }));
    }
    let message = match roots.as_slice() {
        [n] if n.is_layer() => tl!("Delete this layer and its artwork?"),
        _ => tl!("Delete the selected items and their artwork?"),
    };
    let params = json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "doc": st.uid, "confirmed": true});
    crate::dialogs::confirm::ask(app, message, "", "ui.layerDelete", params);
    Ok(Value::Null)
}

/// Whether `id` is the isolated node or inside it: a row the panel can list. Always true outside
/// isolation mode.
fn in_isolated_art(st: &vectorcraft_engine::DocState, id: NodeId) -> bool {
    let root = st.isolation.filter(|i| st.doc.node(*i).is_some());
    root.is_none_or(|r| st.doc.ancestry(id).is_some_and(|a| a.contains(&r)))
}

/// The highlighted rows the panel lists: in isolation mode only those in the isolated art.
fn listed_highlighted_rows(st: &vectorcraft_engine::DocState) -> Vec<NodeId> {
    st.highlighted_rows().into_iter().filter(|id| in_isolated_art(st, *id)).collect()
}

/// The current layer, which the panel's commands act on when no row is highlighted. In isolation
/// mode it is None unless the layer is inside the isolated art, the only art the panel lists.
fn listed_current_layer(st: &vectorcraft_engine::DocState) -> Option<NodeId> {
    st.current_layer().filter(|l| in_isolated_art(st, *l))
}

/// In isolation mode, whether the layer that New Layer (`sublayer` false; `top`: on top of every
/// layer) or New Sublayer makes would be outside the isolated art, where the panel does not list
/// it. New Layer goes beside the current layer, New Sublayer into the layer of the last highlighted
/// row, else into the current layer, as `layer.new` and `layer.newSublayer` place them.
pub(crate) fn new_layer_unlisted(st: &vectorcraft_engine::DocState, sublayer: bool, top: bool) -> bool {
    let Some(root) = st.isolation.filter(|i| st.doc.node(*i).is_some()) else { return false };
    if sublayer {
        let row = st.highlighted_rows().last().and_then(|r| st.doc.layer_containing(*r));
        !row.or(st.current_layer()).is_some_and(|p| in_isolated_art(st, p))
    } else {
        top || !st.current_layer().is_some_and(|c| c != root && in_isolated_art(st, c))
    }
}

/// The rows among `nodes` (and the rows inside them) that are in `listed` and not inside another
/// listed row, top-level first.
fn row_roots<'a>(nodes: &'a [Arc<Node>], listed: &HashSet<NodeId>, out: &mut Vec<&'a Node>) {
    for n in nodes {
        if out.len() == listed.len() {
            return;
        }
        if listed.contains(&n.id) {
            out.push(n);
        } else if let Some(children) = n.children() {
            row_roots(children, listed, out);
        }
    }
}

/// Whether a layer holds art: an object other than a guide, in it or in one of its sublayers.
fn holds_art(layer: &Node) -> bool {
    layer.children().into_iter().flatten().any(|c| if c.is_layer() { holds_art(c) } else { !c.is_guide() })
}

/// `ui.layersExpand {ids?, open?}`: open (or close) rows of the active document's Layers panel;
/// without `ids`, every row that holds others.
pub fn expand(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let open = p.get("open").and_then(Value::as_bool).unwrap_or(true);
    let ids: Vec<u64> = match p.get("ids").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_u64).filter(|i| st.doc.node(NodeId(*i)).is_some()).collect(),
        None => {
            let mut v = vec![];
            st.doc.walk(|n| {
                if n.is_container() {
                    v.push(n.id.0);
                }
            });
            v
        }
    };
    let st = app.session.active_mut().ok_or("no document open")?;
    for i in &ids {
        st.layers_open.set(NodeId(*i), open);
    }
    Ok(json!({ "count": ids.len() }))
}

/// `ui.layersFilter {objects?, layers?}`: set the Layers panel's Filter, `objects` to the object
/// kinds ticked under By Object and `layers` to the row states ticked under By Layer, by parameter
/// name ([`FilterObject::ALL`], [`FilterState::ALL`]). A key left out or set to null keeps its list; any
/// other value that is not an array of those names is an error, and then nothing changes. The
/// panel deselects nothing.
/// → {objects, layers}
pub fn set_filter(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let objects = parse_names(p, "objects", &FilterObject::ALL)?;
    let layers = parse_names(p, "layers", &FilterState::ALL)?;
    let f = &mut app.ui.layers_ui.filter;
    if let Some(objects) = objects {
        f.objects = objects;
    }
    if let Some(layers) = layers {
        f.states = layers;
    }
    Ok(json!({"objects": names(&FilterObject::ALL, &f.objects), "layers": names(&FilterState::ALL, &f.states)}))
}
