use super::model::{ObjectKind, RowState, names, parse_names, toggled};
use super::{
    Arc, Document, HashSet, LayersDrag, Node, NodeId, NodeKind, PanelDrag, Sense, Stroke, Ui, Value, VectorcraftApp, View, json, last_selected,
    see_selection, vec2, widgets,
};

/// The Filter button, lit while a filter is on, and its popup: Clear All, the object kinds under
/// By Object and the row states under By Layer, each ticked or not. A tick, or Clear All, runs
/// `ui.layersFilter`; the popup stays open until a click outside it or Escape.
pub(super) fn filter_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let f = app.ui.layers_filter.clone();
    let button = widgets::icon_button(ui, "dc-filter", tl!("Filter"), f.is_on(), 24.0);
    let mut set = None;
    egui::Popup::menu(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        widgets::menu_scroll(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::subheader(ui, tl!("Filter"));
                ui.add_space(12.0);
                if ui.add_enabled(f.is_on(), egui::Button::new(egui::RichText::new(tl!("Clear All")).size(12.5)).frame(false)).clicked() {
                    set = Some(json!({"objects": [], "layers": []}));
                }
            });
            widgets::subheader(ui, tl!("By Object"));
            for (k, _) in ObjectKind::ALL {
                if widgets::check(ui, k.label(), f.objects.contains(&k), true) {
                    set = Some(json!({"objects": toggled(&ObjectKind::ALL, &f.objects, k)}));
                }
            }
            widgets::subheader(ui, tl!("By Layer"));
            for (s, _) in RowState::ALL {
                if widgets::check(ui, s.label(), f.layers.contains(&s), true) {
                    set = Some(json!({"layers": toggled(&RowState::ALL, &f.layers, s)}));
                }
            }
        });
    });
    if let Some(p) = set {
        app.run("ui.layersFilter", p).ok();
    }
}

/// The bottom bar: the layer count and the panel's buttons.
pub(super) fn bottom_bar(app: &mut VectorcraftApp, ui: &mut Ui, view: &View, doc: &Document, actions: &mut Vec<(String, Value)>) {
    let t = &view.t;
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, t.divider));
    let n = doc.layers.len();
    ui.painter().text(
        bar.left_center() + vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        crate::i18n::tn(n as u64, "{n} Layer", "{n} Layers"),
        egui::FontId::proportional(11.5),
        t.text_dim,
    );
    let m = ui.input(|i| i.modifiers);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let trash = widgets::icon_button(&mut child, "trash-2", tl!("Delete Selection"), false, 24.0);
    // A target circle dropped on the trash reduces that appearance to one fill and one stroke
    // (dragged rows: below). Peek at the payload's type before taking it: egui's take drops a
    // payload of another type.
    if trash.dnd_hover_payload::<PanelDrag>().is_some()
        && let Some(d) = trash.dnd_release_payload::<PanelDrag>()
        && let PanelDrag::Appearance(id) = *d
    {
        actions.push(("appearance.reduceToBasic".into(), json!({"ids": [id.0]})));
    } else if trash.clicked() {
        // With no row highlighted, the selected art and ruler guides go.
        if view.rows.is_empty() && app.session.active().is_some_and(|d| d.selection.has_objects_or_guides()) {
            actions.push(("edit.clear".into(), json!({})));
        } else {
            actions.push(("ui.layerDelete".into(), json!({})));
        }
    }
    // In isolation mode New Layer and New Sublayer are off while the layer they make would be
    // outside the isolated art, where the panel does not list it.
    let unlisted = |sublayer: bool, top: bool| app.session.active().is_some_and(|st| new_layer_unlisted(st, sublayer, top));
    let new_layer_ok = !unlisted(false, m.command && !m.alt);
    let new_sublayer_ok = !unlisted(true, false);
    // Create New Layer: Alt asks for its options first, Ctrl/Cmd puts it on top of every layer,
    // and Ctrl/Cmd+Alt puts it directly below the current layer.
    let new_layer = widgets::icon_button_enabled(&mut child, "file-plus", tl!("Create New Layer"), false, new_layer_ok, 24.0);
    if new_layer.clicked() {
        if m.alt && m.command {
            actions.push(("layer.new".into(), json!({"below": true})));
        } else if m.alt {
            actions.push(("ui.newLayer".into(), json!({})));
        } else {
            actions.push(("layer.new".into(), json!({"top": m.command})));
        }
    }
    // Rows dropped on the trash are deleted, on New Layer duplicated. Both are
    // outlined while rows (or, on the trash, a target circle) hang over them. Guide rows dropped on
    // the trash delete their guides (`guide.remove`); New Layer does not take them.
    for (button, cmd) in [(&trash, "ui.layerDelete"), (&new_layer, "layer.duplicate")] {
        let deletes = cmd == "ui.layerDelete";
        if let Some(d) = button.dnd_release_payload::<LayersDrag>() {
            match &*d {
                // A copy of the isolated node would go beside it, outside the isolated art.
                LayersDrag::Rows(ids) if !deletes && view.root.is_some_and(|r| ids.contains(&r)) => {}
                LayersDrag::Rows(ids) => actions.push((cmd.into(), json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))),
                LayersDrag::Guides(guides) if deletes => actions.push(("guide.remove".into(), json!({"indexes": guides}))),
                _ => {}
            }
        }
        let over = button.dnd_hover_payload::<LayersDrag>().is_some_and(|d| match &*d {
            LayersDrag::Rows(ids) => deletes || !view.root.is_some_and(|r| ids.contains(&r)),
            LayersDrag::Guides(_) => deletes,
            LayersDrag::Art => false,
        }) || (deletes && button.dnd_hover_payload::<PanelDrag>().is_some_and(|d| matches!(*d, PanelDrag::Appearance(_))));
        if over {
            widgets::drop_target_outline(&child, button.rect);
        }
    }
    // Create New Sublayer: Alt asks for its options first, and Ctrl/Cmd+Alt puts it under the
    // layer's contents.
    if widgets::icon_button_enabled(&mut child, "plus", tl!("Create New Sublayer"), false, new_sublayer_ok, 24.0).clicked() {
        actions.push(match (m.alt, m.command) {
            (true, true) => ("layer.newSublayer".into(), json!({"bottom": true})),
            (true, false) => ("ui.newLayer".into(), json!({"sublayer": true})),
            _ => ("layer.newSublayer".into(), json!({})),
        });
    }
    if widgets::icon_button(&mut child, "frame", tl!("Make/Release Clipping Mask"), false, 24.0).clicked() {
        actions.push(("layer.clippingMask.toggle".into(), json!({})));
    }
    // With Show Layers Only on, Locate Object becomes Locate Layer: it highlights the layer row.
    let layers_only = view.opts.layers_only;
    let locate = if layers_only { tl!("Locate Layer") } else { tl!("Locate Object") };
    let locate_ok = !layers_only || super::rows::locate_layer_listed(doc, view.root);
    if widgets::icon_button_enabled(&mut child, "search", locate, false, locate_ok, 24.0).clicked() {
        actions.push(("layer.locate".into(), json!({ "layersOnly": layers_only })));
    }
}

/// A panel menu item: label, command, params, enabled, checked.
type MenuItem<'a> = (&'a str, &'a str, Value, bool, bool);

/// The Layers panel's (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.clone();
    let rows = listed_highlighted_rows(st);
    let current = st.current_layer();
    let has_sel = !st.selection.is_empty();
    let isolated = st.isolation.is_some();
    let layers_only = app.ui.layers_panel.layers_only;
    let targets: Vec<NodeId> = if rows.is_empty() { listed_current_layer(st).into_iter().collect() } else { rows.clone() };
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
    let mut items: Vec<Option<MenuItem>> = vec![
        Some(("New Layer…", "ui.newLayer", json!({}), new_layer_ok, false)),
        Some(("New Sublayer…", "ui.newLayer", json!({"sublayer": true}), current.is_some() && new_sublayer_ok, false)),
        // The rows listed, so no highlighted row outside the isolated art is copied.
        Some(("Duplicate Selection", "layer.duplicate", json!({"ids": targets.iter().map(|i| i.0).collect::<Vec<_>>()}), duplicate_ok, false)),
        Some(("Delete Selection", "ui.layerDelete", json!({}), !targets.is_empty(), false)),
        Some(("Options for Selection…", "ui.layerOptions", json!({}), !targets.is_empty(), false)),
        None,
        Some(("Make/Release Clipping Mask", "layer.clippingMask.toggle", json!({}), true, false)),
        None,
    ];
    items.push(if isolated {
        Some(("Exit Isolation Mode", "object.exitIsolation", json!({}), true, false))
    } else {
        let id = targets.first().map(|i| i.0);
        Some(("Enter Isolation Mode", "object.isolate", json!({ "id": id }), one_container, false))
    });
    items.extend([
        None,
        Some(if layers_only {
            ("Locate Layer", "layer.locate", json!({"layersOnly": true}), has_sel && super::rows::locate_layer_listed(&doc, st.isolation), false)
        } else {
            ("Locate Object", "layer.locate", json!({}), has_sel, false)
        }),
        // Sentinel: rendered as the Save Selection submenu (the Select menu's saved-selection items).
        Some(("Save Selection", "", json!({}), true, false)),
        None,
        Some(("Merge Selected", "layer.merge", json!({}), rows.len() >= 2, false)),
        Some(("Flatten Artwork", "layer.flatten", json!({}), true, false)),
        Some(("Collect in New Layer", "layer.collectInNew", json!({}), !rows.is_empty() || has_sel, false)),
        // Sentinel: rendered as Collect for Export › As Single / As Multiple (assets.add).
        Some(("Collect for Export", "", json!({}), !targets.is_empty() || has_sel, false)),
        Some(("Release to Layers (Sequence)", "layer.releaseToLayers", json!({}), one_container, false)),
        Some(("Release to Layers (Build)", "layer.releaseToLayersBuild", json!({}), one_container, false)),
        Some(("Reverse Order", "layer.reverse", json!({}), rows.len() >= 2, false)),
        None,
        Some(("Template", "layer.template", json!({}), !layers_in.is_empty(), template)),
    ]);
    // Each pair reads by state: Hide Others while another layer shows, else Show All Layers. In
    // isolation mode the other layers are outside the isolated art, so the pairs are off.
    let any_other = |f: &dyn Fn(&Node) -> bool| others.iter().any(|l| f(l));
    items.push(Some(if any_other(&|l| l.visible) {
        ("Hide Others", "layer.hideOthers", json!({}), !isolated, false)
    } else {
        ("Show All Layers", "layer.showAll", json!({}), !isolated && all_layers.iter().any(|l| !l.visible), false)
    }));
    items.push(Some(if any_other(&|l| !matches!(l.kind, NodeKind::Layer { preview: false, .. })) {
        ("Outline Others", "layer.outlineOthers", json!({}), !isolated, false)
    } else {
        (
            "Preview All Layers",
            "layer.previewAll",
            json!({}),
            !isolated && all_layers.iter().any(|l| matches!(l.kind, NodeKind::Layer { preview: false, .. })),
            false,
        )
    }));
    items.push(Some(if any_other(&|l| !l.locked) {
        ("Lock Others", "layer.lockOthers", json!({}), !isolated, false)
    } else {
        ("Unlock All Layers", "layer.unlockAll", json!({}), !isolated && all_layers.iter().any(|l| l.locked), false)
    }));
    items.push(None);
    let remembers = doc.paste_remembers_layers;
    items.push(Some(("Paste Remembers Layers", "layer.pasteRemembersLayers", json!({"on": !remembers}), true, remembers)));
    items.push(None);
    items.push(Some(("Panel Options…", "ui.layersPanelOptions", json!({}), true, false)));
    for item in items {
        match item {
            None => {
                ui.separator();
            }
            Some(("Collect for Export", _, _, enabled, _)) => {
                collect_for_export_menu(ui, enabled, |multiple| {
                    let mut p = json!({ "multiple": multiple });
                    if !targets.is_empty() {
                        p["ids"] = json!(targets.iter().map(|id| id.0).collect::<Vec<_>>());
                    }
                    app.run("assets.add", p).ok();
                });
            }
            Some(("Save Selection", "", ..)) => save_selection_menu(app, ui),
            Some((label, cmd, p, enabled, checked)) => {
                if widgets::menu_item(ui, label, enabled, checked) {
                    let selected = last_selected(app);
                    app.run(cmd, p).ok();
                    see_selection(app, ui, selected);
                }
            }
        }
    }
}

/// Collect for Export › As Single Asset / As Multiple Assets (`assets.add`).
pub(super) fn collect_for_export_menu(ui: &mut Ui, enabled: bool, mut run: impl FnMut(bool)) {
    ui.add_enabled_ui(enabled, |ui| {
        // English labels match the rest of this panel menu (and Object › Collect for Export).
        ui.menu_button(widgets::check_row_text(ui, "", "Collect for Export", None), |ui| {
            if widgets::menu_item(ui, "As Single Asset", true, false) {
                run(false);
                ui.close();
            }
            if widgets::menu_item(ui, "As Multiple Assets", true, false) {
                run(true);
                ui.close();
            }
        });
    });
}

/// Save Selection › Save Selection…, Update Selection and Edit Selection…, enabled and run as in
/// the Select menu, then the active document's saved selections: choosing one recalls it.
fn save_selection_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    ui.menu_button(widgets::check_row_text(ui, "   ", tl!("Save Selection"), None), |ui| {
        widgets::menu_scroll(ui, |ui| {
            for (label, id) in [("Save Selection…", "select.save"), ("Update Selection", "select.update"), ("Edit Selection…", "select.editSaved")]
            {
                if widgets::menu_item(ui, label, crate::menus::enabled(app, id), false) {
                    crate::menus::invoke(app, id, json!({}));
                    ui.close();
                }
            }
            let names: Vec<String> =
                app.session.active().map(|d| d.doc.saved_selections.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
            if !names.is_empty() {
                ui.separator();
            }
            for name in names {
                if widgets::menu_item_name(ui, &name, true, false) {
                    let selected = last_selected(app);
                    app.run("select.recall", json!({ "name": name })).ok();
                    see_selection(app, ui, selected);
                    ui.close();
                }
            }
        });
    });
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
/// name ([`ObjectKind::ALL`], [`RowState::ALL`]). A key left out or set to null keeps its list; any
/// other value that is not an array of those names is an error, and then nothing changes.
/// → {objects, layers}
pub fn set_filter(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let objects = parse_names(p, "objects", &ObjectKind::ALL)?;
    let layers = parse_names(p, "layers", &RowState::ALL)?;
    let f = &mut app.ui.layers_filter;
    if let Some(objects) = objects {
        f.objects = objects;
    }
    if let Some(layers) = layers {
        f.layers = layers;
    }
    Ok(json!({"objects": names(&ObjectKind::ALL, &f.objects), "layers": names(&RowState::ALL, &f.layers)}))
}
