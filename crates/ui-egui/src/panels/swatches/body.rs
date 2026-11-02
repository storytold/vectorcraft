use super::actions::{bottom, edit_group};
use super::dnd::{TileEvents, apply_drop, click_selection, highlighted, selection, tile_input, zone_input};
use super::tiles::{
    draw_folder, draw_registration, folder_tile, global_mark, list_icons, list_row, list_row_name, pattern_thumb, tile_grid, tile_id,
};
use super::{Entry, Kind, Paint, Sense, Ui, VectorcraftApp, View, active_paint, apply, entries, json, pstate, set_pstate, swatch_tile, widgets};

/// The id of the find field's text (shown with the panel menu's Show Find Field).
pub(super) fn find_id() -> egui::Id {
    egui::Id::unique("swatch-find")
}

/// The panel's body; `salt` keys its tiles (see [`tile_id`]).
pub(super) fn body(app: &mut VectorcraftApp, ui: &mut Ui, salt: &'static str) {
    if app.session.active().is_none() {
        crate::panels::empty_state(ui, "swatch-book", tl!("No document"), tl!("Open a document to see its swatches."));
        return;
    }
    let view: View = pstate(ui.ctx(), "swatch-view");
    let kind: Kind = pstate(ui.ctx(), "swatch-kind");
    // Top row: proxy at left, list / grid view toggles at right.
    ui.horizontal(|ui| {
        crate::panels::proxy(app, ui, 34.0);
        ui.add_space(ui.available_width() - 58.0);
        if widgets::icon_button(ui, "dc-list-view", tl!("Show List View"), view.is_list(), 26.0).clicked() {
            set_pstate(ui.ctx(), "swatch-view", View::SmallList);
        }
        if widgets::icon_button(ui, "dc-grid-view", tl!("Show Thumbnail View"), !view.is_list(), 26.0).clicked() {
            set_pstate(ui.ctx(), "swatch-view", View::MediumThumb);
        }
    });
    ui.add_space(4.0);
    // A recent colour paints the active proxy (Alt: the inactive one), as a swatch does, and
    // becomes the Color Guide's base color.
    if let Some(c) = crate::panels::recent_colors_row(app, ui) {
        crate::panels::color_guide::choose_base(app, c);
        crate::panels::apply_click(app, ui, json!({"color": crate::panels::color_json(&c)}));
    }
    widgets::divider(ui);
    widgets::subheader(ui, tl!("Swatch Tiles"));
    let query = if pstate(ui.ctx(), "swatch-show-find") { widgets::search_field(ui, find_id(), tl!("Find")) } else { String::new() };
    let items = entries(app, kind, &query);
    let active_swatch = kept_swatch(app, ui, active_swatch(app));
    let focus = focus_swatch(app, ui, active_swatch.as_deref());
    let selected = selection(app, ui);
    // Without a selection the swatch of the active paint is highlighted.
    let fallback: Vec<String> = if selected.is_empty() { active_swatch.into_iter().collect() } else { vec![] };
    let sel = if selected.is_empty() { &fallback } else { &selected };
    let lit = app.session.active().map(|st| highlighted(&st.doc, sel)).unwrap_or_default();
    let is_sel = |name: &str| lit.contains(&name);
    let mut ev = TileEvents::default();
    widgets::list_box(ui, |ui| {
        let max_height = if view == View::LargeThumb { 200.0 } else { 150.0 };
        let out = egui::ScrollArea::vertical().id_salt("swatch-scroll").max_height(max_height).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if view.is_list() {
                for e in &items {
                    let name = e.name();
                    let (r, resp, chip) = list_row(ui, tile_id(salt, e), view, is_sel(name));
                    if focus.as_deref() == Some(name) {
                        ui.scroll_to_rect(r, Some(egui::Align::Center));
                    }
                    match e {
                        Entry::Registration => draw_registration(ui, chip),
                        Entry::Swatch { paint, global, spot, .. } => {
                            swatch_tile(ui, chip, paint, false, false);
                            pattern_thumb(app, ui, chip, paint);
                            list_icons(ui, r, paint, *global, *spot);
                        }
                        Entry::Folder(_) => draw_folder(ui, chip),
                    }
                    list_row_name(ui, r, chip, crate::i18n::t_name("swatch", name));
                    tile_input(ui, resp, e, &items, sel, true, &mut ev);
                }
            } else {
                for (e, cell) in tile_grid(ui, &items, view, Entry::is_folder) {
                    let resp = ui.interact(cell, tile_id(salt, e), Sense::click_and_drag());
                    let name = e.name();
                    if focus.as_deref() == Some(name) {
                        ui.scroll_to_rect(cell, Some(egui::Align::Center));
                    }
                    match e {
                        Entry::Registration => draw_registration(ui, cell),
                        Entry::Swatch { paint, global, spot, .. } => {
                            swatch_tile(ui, cell, paint, is_sel(name), resp.hovered());
                            pattern_thumb(app, ui, cell.shrink(1.0), paint);
                            if *global {
                                global_mark(ui, cell, *spot);
                            }
                        }
                        Entry::Folder(_) => folder_tile(ui, cell, is_sel(name)),
                    }
                    tile_input(ui, resp, e, &items, sel, false, &mut ev);
                }
            }
        });
        let zone = ui.interact(out.inner_rect, ui.scope_id().with("swatch-drop"), Sense::hover());
        zone_input(ui, &zone, &mut ev);
    });
    match ev.edit {
        Some(Entry::Folder(g)) => edit_group(app, &g),
        Some(e) => {
            app.run("ui.swatchOptions", json!({"name": e.name()})).ok();
        }
        None => {}
    }
    if let Some(drop) = ev.drop {
        apply_drop(app, drop);
    }
    let selected = match ev.clicked {
        Some((e, m)) => {
            let order: Vec<&str> = items.iter().map(Entry::name).collect();
            let sel = click_selection(ui, "swatch-anchor", selected, &order, e.name(), m);
            set_pstate(ui.ctx(), "swatch-selected", sel.clone());
            // A plain click also applies the swatch; modifier clicks only select.
            if !(m.shift || m.command) {
                apply(app, ui, &e);
            }
            sel
        }
        None => selected,
    };
    bottom(app, ui, &selected);
}

/// The document and the objects selected in it.
type SelectedArt = (u64, Vec<u64>);

/// The swatch highlighted for the active paint (`active`): with art selected, its swatch; once the
/// art is deselected, still that swatch, until another document shows or the paints for new art
/// change (a swatch or colour picked with nothing selected). Deselecting doesn't jump to the
/// swatch of the paint new art gets (White).
pub(super) fn kept_swatch(app: &VectorcraftApp, ui: &Ui, active: Option<String>) -> Option<String> {
    let Some(st) = app.session.active() else { return active };
    let p = &app.session.paint;
    // The document, and the paints new art gets, which picking with nothing selected changes.
    let defaults = format!("{}|{:?}|{:?}|{}", st.uid, p.fill, p.stroke, app.session.fill_active);
    if !st.selection.objects.is_empty() {
        set_pstate(ui.ctx(), "swatch-kept", Some((defaults, active.clone())));
        return active;
    }
    match pstate::<Option<(String, Option<String>)>>(ui.ctx(), "swatch-kept") {
        Some((d, kept)) if d == defaults => kept,
        Some(_) => {
            set_pstate(ui.ctx(), "swatch-kept", None::<(String, Option<String>)>);
            active
        }
        None => active,
    }
}

/// The swatch the tiles scroll to this frame, once: the active paint's swatch when it changed, or
/// when other art was selected. Selecting other art also clears the panel's own selection, so its
/// swatch is the one highlighted.
pub(super) fn focus_swatch(app: &VectorcraftApp, ui: &Ui, active: Option<&str>) -> Option<String> {
    let art: Option<SelectedArt> = app.session.active().map(|st| (st.uid, st.selection.objects.iter().map(|id| id.0).collect()));
    // (None the first time the panel shows.)
    let (last_art, last_swatch): (Option<Option<SelectedArt>>, Option<String>) = pstate(ui.ctx(), "swatch-focus");
    let reselected = last_art.as_ref().is_some_and(|l| *l != art);
    if reselected {
        set_pstate(ui.ctx(), "swatch-selected", Vec::<String>::new());
        set_pstate(ui.ctx(), "swatch-anchor", String::new());
    }
    let changed = reselected || last_art.is_none() || last_swatch.as_deref() != active;
    set_pstate(ui.ctx(), "swatch-focus", (Some(art), active.map(str::to_string)));
    active.filter(|_| changed).map(str::to_string)
}

/// The swatch the active paint comes from: a linked colour's global swatch (a tint's tint swatch
/// when there is one), a process colour's swatch of that colour (a colour no swatch has: none, so
/// text in a colour of its own highlights nothing), a gradient's gradient swatch, or None; nothing
/// for selected images.
pub(super) fn active_swatch(app: &VectorcraftApp) -> Option<String> {
    // Images and placed files have no fill or stroke: no swatch (not None).
    let st = app.session.active()?;
    let unpainted = |id: &vectorcraft_doc::NodeId| {
        st.doc.node(*id).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_) | vectorcraft_doc::NodeKind::PlacedDocument(_)))
    };
    if !st.selection.objects.is_empty() && st.selection.objects.iter().all(unpainted) {
        return None;
    }
    match active_paint(app) {
        Paint::Solid { swatch: Some(n), tint, .. } => {
            let d = &app.session.active()?.doc;
            let tint_swatch = d.swatches_iter().find(|w| w.tint_of() == Some((n.as_str(), tint)));
            Some(tint_swatch.map_or(n.clone(), |w| w.name.clone()))
        }
        // A process colour isn't linked: the swatch of that very colour.
        Paint::Solid { color, swatch: None, .. } => {
            let d = &app.session.active()?.doc;
            d.swatches_iter().find(|w| matches!(w.paint, Paint::Solid { color: c, swatch: None, .. } if c == color)).map(|w| w.name.clone())
        }
        Paint::Gradient(g) => g.swatch,
        Paint::None => Some("[None]".to_string()),
        _ => None,
    }
}
