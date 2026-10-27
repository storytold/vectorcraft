use super::model::{ObjectKind, RowState, names, parse_names, toggled};
use super::*;

/// The Filter button, lit while a filter is on, and its popup: Clear All, the object kinds under
/// By Object and the row states under By Layer, each ticked or not. A tick, or Clear All, runs
/// `ui.layersFilter`; the popup stays open until a click outside it or Escape.
pub(super) fn filter_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let f = app.ui.layers_filter.clone();
    let button = widgets::icon_button(ui, "dc-filter", tl!("Filter"), f.is_on(), 24.0);
    let mut set = None;
    egui::Popup::menu(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
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
    // A target circle dropped on the trash clears that appearance (dragged rows: below). Peek at
    // the payload's type before taking it: egui's take drops a payload of another type.
    if trash.dnd_hover_payload::<PanelDrag>().is_some()
        && let Some(d) = trash.dnd_release_payload::<PanelDrag>()
        && let PanelDrag::Appearance(id) = *d
    {
        actions.push(("appearance.clear".into(), json!({"ids": [id.0]})));
    } else if trash.clicked() {
        if view.rows.is_empty() && app.session.active().is_some_and(|d| !d.selection.is_empty()) {
            actions.push(("edit.clear".into(), json!({})));
        } else {
            actions.push(("layer.delete".into(), json!({})));
        }
    }
    // Create New Layer: Alt asks for its options first, Ctrl/Cmd puts it on top of every layer.
    let new_layer = widgets::icon_button(&mut child, "file-plus", tl!("Create New Layer"), false, 24.0);
    if new_layer.clicked() {
        if m.alt {
            actions.push(("ui.newLayer".into(), json!({})));
        } else {
            actions.push(("layer.new".into(), json!({"top": m.command})));
        }
    }
    // Rows dropped on the trash are deleted, on New Layer duplicated, as in Illustrator. Both are
    // outlined while rows (or, on the trash, a target circle) hang over them.
    for (button, cmd) in [(&trash, "layer.delete"), (&new_layer, "layer.duplicate")] {
        if let Some(d) = button.dnd_release_payload::<LayersDrag>()
            && let LayersDrag::Rows(ids) = &*d
        {
            actions.push((cmd.into(), json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})));
        }
        let over = button.dnd_hover_payload::<LayersDrag>().is_some_and(|d| matches!(*d, LayersDrag::Rows(_)))
            || (cmd == "layer.delete" && button.dnd_hover_payload::<PanelDrag>().is_some_and(|d| matches!(*d, PanelDrag::Appearance(_))));
        if over {
            child.painter().rect_stroke(button.rect, 3.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
        }
    }
    if widgets::icon_button(&mut child, "plus", tl!("Create New Sublayer"), false, 24.0).clicked() {
        actions.push(if m.alt { ("ui.newLayer".into(), json!({"sublayer": true})) } else { ("layer.newSublayer".into(), json!({})) });
    }
    if widgets::icon_button(&mut child, "frame", tl!("Make/Release Clipping Mask"), false, 24.0).clicked() {
        actions.push(("layer.clippingMask.toggle".into(), json!({})));
    }
    if widgets::icon_button(&mut child, "search", tl!("Locate Object"), false, 24.0).clicked() {
        actions.push(("layer.locate".into(), json!({})));
    }
}

/// A panel menu item: label, command, params, enabled, checked.
type MenuItem<'a> = (&'a str, &'a str, Value, bool, bool);

/// The Layers panel's (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.clone();
    let rows = st.highlighted_rows();
    let current = st.current_layer();
    let has_sel = !st.selection.is_empty();
    let isolated = st.isolation.is_some();
    let targets: Vec<NodeId> = if rows.is_empty() { current.into_iter().collect() } else { rows.clone() };
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
        Some(("New Layer…", "ui.newLayer", json!({}), true, false)),
        Some(("New Sublayer…", "ui.newLayer", json!({"sublayer": true}), current.is_some(), false)),
        Some(("Duplicate Selection", "layer.duplicate", json!({}), !targets.is_empty(), false)),
        Some(("Delete Selection", "layer.delete", json!({}), !targets.is_empty(), false)),
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
        Some(("Locate Object", "layer.locate", json!({}), has_sel, false)),
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
    // Each pair reads by state: Hide Others while another layer shows, else Show All Layers.
    let any_other = |f: &dyn Fn(&Node) -> bool| others.iter().any(|l| f(l));
    items.push(Some(if any_other(&|l| l.visible) {
        ("Hide Others", "layer.hideOthers", json!({}), true, false)
    } else {
        ("Show All Layers", "layer.showAll", json!({}), all_layers.iter().any(|l| !l.visible), false)
    }));
    items.push(Some(if any_other(&|l| !matches!(l.kind, NodeKind::Layer { preview: false, .. })) {
        ("Outline Others", "layer.outlineOthers", json!({}), true, false)
    } else {
        (
            "Preview All Layers",
            "layer.previewAll",
            json!({}),
            all_layers.iter().any(|l| matches!(l.kind, NodeKind::Layer { preview: false, .. })),
            false,
        )
    }));
    items.push(Some(if any_other(&|l| !l.locked) {
        ("Lock Others", "layer.lockOthers", json!({}), true, false)
    } else {
        ("Unlock All Layers", "layer.unlockAll", json!({}), all_layers.iter().any(|l| l.locked), false)
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
        ui.menu_button("Collect for Export", |ui| {
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
    ui.menu_button(format!("   {}", tl!("Save Selection")), |ui| {
        for (label, id) in [("Save Selection…", "select.save"), ("Update Selection", "select.update"), ("Edit Selection…", "select.editSaved")] {
            if widgets::menu_item(ui, label, crate::menus::enabled(app, id), false) {
                crate::menus::invoke(app, id, json!({}));
                ui.close();
            }
        }
        let names: Vec<String> = app.session.active().map(|d| d.doc.saved_selections.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
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
