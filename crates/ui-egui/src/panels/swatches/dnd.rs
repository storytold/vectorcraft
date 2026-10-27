use super::tiles::{describe, draw_registration, pattern_thumb};
use super::*;

/// The swatches and colour groups selected in the panel, in click order, without names that no
/// longer exist (deleted, renamed or undone).
pub(super) fn selection(app: &VectorcraftApp, ui: &Ui) -> Vec<String> {
    let Some(st) = app.session.active() else { return vec![] };
    let mut names: Vec<String> = pstate(ui.ctx(), "swatch-selected");
    names.retain(|n| n == REGISTRATION || st.doc.swatch_name_taken(n));
    names
}

/// The names the panel highlights: `sel` plus the swatches of selected colour groups (clicking a
/// folder selects the whole group).
pub(super) fn highlighted<'a>(d: &'a Document, sel: &'a [String]) -> Vec<&'a str> {
    let mut out: Vec<&str> = sel.iter().map(String::as_str).collect();
    for g in d.swatch_groups.iter().filter(|g| sel.contains(&g.name)) {
        out.extend(g.swatches.iter().map(|w| w.name.as_str()));
    }
    out
}

/// The selection after a click on `name`: Cmd/Ctrl toggles it, Shift extends from the last clicked
/// name (kept in panel state `anchor_key`) over `order` (the names as displayed), a plain click
/// selects it alone.
pub(crate) fn click_selection(ui: &Ui, anchor_key: &str, mut sel: Vec<String>, order: &[&str], name: &str, m: egui::Modifiers) -> Vec<String> {
    let anchor: String = pstate(ui.ctx(), anchor_key);
    let pos = |n: &str| order.iter().position(|o| *o == n);
    if m.shift
        && let (Some(a), Some(b)) = (pos(&anchor), pos(name))
    {
        for n in &order[a.min(b)..=a.max(b)] {
            if !sel.iter().any(|s| s == n) {
                sel.push(n.to_string());
            }
        }
        return sel;
    }
    set_pstate(ui.ctx(), anchor_key, name.to_string());
    if !m.command {
        return vec![name.to_string()];
    }
    match sel.iter().position(|s| s == name) {
        Some(i) => {
            sel.remove(i);
        }
        None => sel.push(name.to_string()),
    }
    sel
}

/// A drag released on the panel.
pub(super) enum Drop {
    /// Swatches (or colour groups) dropped before or `after` row `target`, into folder `target`, or
    /// at the end (`None`). With Alt held, `with` names the swatch grabbed: dropped on swatch
    /// `target` of its kind (neither None, Registration nor a tint swatch; see [`replaces`]), it
    /// gives that swatch its paint instead of moving.
    Move { names: Vec<String>, target: Option<String>, after: bool, with: Option<String> },
    /// A paint from elsewhere (a Fill/Stroke proxy, the Gradient panel's thumbnail) dropped on row
    /// `target` (or between rows) becomes a swatch; with Alt held (`replace`) it replaces swatch
    /// `target` when that is of its kind and not a tint swatch.
    New { paint: Paint, target: Option<String>, replace: bool },
    /// Art dragged off the canvas becomes a pattern swatch made of a copy of it (the art stays).
    Pattern { ids: Vec<u64> },
}

impl Drop {
    /// What releasing `d` before or `after` row `target` (`None`: at the end) does: move the rows
    /// it carries (with Alt held, `replace`, a swatch grabbed may replace `target`), make a swatch of
    /// a paint dragged from elsewhere, or a pattern swatch of art dragged off the canvas (appearances:
    /// nothing).
    fn of(d: &PanelDrag, target: Option<String>, after: bool, replace: bool) -> Option<Self> {
        match d {
            PanelDrag::Paint { rows: Some(r), .. } => {
                let with = (replace && !r.groups).then(|| r.grabbed.clone());
                Some(Drop::Move { names: r.names.clone(), target, after, with })
            }
            PanelDrag::Paint { paint, .. } => Some(Drop::New { paint: paint.clone(), target, replace }),
            PanelDrag::Art(ids) => Some(Drop::Pattern { ids: ids.iter().map(|id| id.0).collect() }),
            _ => None,
        }
    }
}

/// The drag held over `resp`, if the panel takes it (paints and art dragged off the canvas, not the
/// Appearance panel's thumbnail).
fn held(resp: &Response) -> Option<std::sync::Arc<PanelDrag>> {
    resp.dnd_hover_payload::<PanelDrag>().filter(|d| matches!(**d, PanelDrag::Paint { .. } | PanelDrag::Art(_)))
}

/// The drag a tile or row of `e` starts, moving rows `names`. Colour groups have no paint of their
/// own (the chip under the pointer draws Registration's mark for Registration).
pub(super) fn drag_of(e: &Entry, names: Vec<String>) -> PanelDrag {
    let paint = match e {
        Entry::Swatch { paint, .. } => paint.clone(),
        Entry::Registration => Paint::registration(),
        Entry::Folder(_) => Paint::None,
    };
    let rows = SwatchRows { grabbed: e.name().to_string(), names, groups: e.is_folder() };
    PanelDrag::Paint { paint, params: swatch_params(e).unwrap_or_default(), rows: Some(rows), library: None }
}

/// What the tiles or rows saw this frame.
#[derive(Default)]
pub(super) struct TileEvents {
    pub(super) clicked: Option<(Entry, egui::Modifiers)>,
    /// Double-click opens the swatch's editor (Swatch Options, Gradient panel or pattern editing),
    /// or Edit or Apply Color Group on a colour group.
    pub(super) edit: Option<Entry>,
    pub(super) drop: Option<Drop>,
    /// A drag is held over a tile or row.
    over: bool,
}

/// Clicks, double-clicks, tooltips and drag and drop of the tile or row `resp` of entry `e`
/// (`items`: the panel's rows, `sel`: its selection, `list`: rows stack downwards). A drag from it
/// starts a [`PanelDrag`] carrying the rows it moves; see [`tile_drop`] for drags held over it.
pub(super) fn tile_input(ui: &Ui, resp: Response, e: &Entry, items: &[Entry], sel: &[String], list: bool, ev: &mut TileEvents) {
    let name = e.name();
    if matches!(e, Entry::Swatch { .. } | Entry::Folder(_)) && resp.double_clicked() {
        ev.edit = Some(e.clone());
    }
    if resp.drag_started() {
        let with_sel = sel.iter().any(|s| s == name);
        let names = items
            .iter()
            .filter(|x| x.is_folder() == e.is_folder() && (x.name() == name || (with_sel && sel.iter().any(|s| s == x.name()))))
            .map(|x| x.name().to_string())
            .collect();
        egui::DragAndDrop::set_payload(ui.ctx(), drag_of(e, names));
    }
    tile_drop(ui, &resp, e, list, ev);
    // The default swatches' names show in the UI's language; the document keeps them as they are.
    let shown = crate::i18n::t_name("swatch", name);
    let resp = match e {
        Entry::Folder(_) => resp.on_hover_text(crate::i18n::fmt(tl!("Color Group: {name}"), &[("name", shown)])),
        Entry::Swatch { paint, global, spot, .. } if list => resp.on_hover_ui(|ui| {
            ui.label(format!("{shown} ({})", describe(paint, *global, *spot)));
        }),
        _ => resp.on_hover_text(shown),
    };
    if resp.clicked() {
        ev.clicked = Some((e.clone(), ui.input(|i| i.modifiers)));
    }
}

/// A drag held over the tile or row `resp` of entry `e` shows where it would land (before or after
/// it by the pointer's half, or into a folder); a release there sets the drop.
fn tile_drop(ui: &Ui, resp: &Response, e: &Entry, list: bool, ev: &mut TileEvents) {
    let Some(d) = held(resp) else { return };
    let PanelDrag::Paint { rows, .. } = &*d else { return };
    let name = e.name();
    ev.over = true;
    // Swatches dropped on one of themselves stay where they are.
    if rows.as_ref().is_some_and(|r| r.names.iter().any(|n| n == name)) {
        resp.dnd_release_payload::<PanelDrag>();
        return;
    }
    let t = Tokens::get(ui.ctx());
    let r = resp.rect;
    let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.center());
    let after = if list { at.y > r.center().y } else { at.x > r.center().x };
    // A paint from elsewhere, or swatches over a folder, go into it; otherwise a bar marks the slot.
    if rows.as_ref().is_none_or(|d| e.is_folder() && !d.groups) {
        ui.painter().rect_stroke(r.expand(1.0), 0.0, Stroke::new(1.5, t.accent), StrokeKind::Outside);
    } else if list {
        let y = if after { r.bottom() } else { r.top() };
        ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(2.0, t.accent));
    } else {
        let x = if after { r.right() + 0.75 } else { r.left() - 0.75 };
        ui.painter().line_segment([pos2(x, r.top() - 1.0), pos2(x, r.bottom() + 1.0)], Stroke::new(2.0, t.accent));
    }
    if resp.dnd_release_payload::<PanelDrag>().is_some() {
        ev.drop = Drop::of(&d, Some(name.to_string()), after, crate::panels::alt_held(ui));
    }
}

/// Drops between and after the tiles (`zone`: the list's viewport) go to the end of the ungrouped
/// swatches (colour groups to the end of the groups); the list is outlined while one is held there.
pub(super) fn zone_input(ui: &Ui, zone: &Response, ev: &mut TileEvents) {
    let Some(d) = held(zone) else { return };
    if !ev.over {
        ui.painter().rect_stroke(zone.rect, 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Inside);
    }
    if zone.dnd_release_payload::<PanelDrag>().is_some() {
        ev.drop = Drop::of(&d, None, false, false);
    }
}

/// The colour group `name` is, or the one swatch `name` belongs to.
pub(super) fn group_of(d: &Document, name: &str) -> Option<String> {
    if d.swatch_groups.iter().any(|g| g.name == name) {
        return Some(name.to_string());
    }
    d.swatch_group_of(name).map(|g| d.swatch_groups[g].name.clone())
}

/// `swatch.move` params for dropping `names` before or `after` row `target`, into folder `target`
/// (swatches), or at the end (`target` None); `None` when nothing would move. Only solid colours go
/// into colour groups; None and Registration stay put.
fn move_params(d: &Document, names: &[String], target: Option<&str>, after: bool) -> Option<Value> {
    if target.is_some_and(|t| names.iter().any(|n| n == t)) {
        return None;
    }
    let group_index = |n: &str| d.swatch_groups.iter().position(|g| g.name == n);
    if names.iter().all(|n| group_index(n).is_some()) {
        let to = match target {
            None => d.swatch_groups.len(),
            // Over a group's folder or one of its swatches: next to that group; else before the first.
            Some(t) => group_index(t).or_else(|| d.swatch_group_of(t)).map_or(0, |i| i + usize::from(after)),
        };
        return Some(json!({"names": names, "to": to}));
    }
    let (group, to) = match target {
        Some(t) if group_index(t).is_some() => (Some(t), None),
        Some(REGISTRATION) => (None, Some(d.swatches.iter().position(|w| !w.paint.is_none()).unwrap_or(0))),
        Some(t) => {
            let (group, list) = match d.swatch_group_of(t) {
                Some(g) => (Some(d.swatch_groups[g].name.as_str()), &d.swatch_groups[g].swatches),
                None => (None, &*d.swatches),
            };
            (group, list.iter().position(|w| w.name == t).map(|i| i + usize::from(after)))
        }
        None => (None, None),
    };
    let movable: Vec<&String> =
        names.iter().filter(|n| d.swatch(n).is_some_and(|w| !w.paint.is_none() && (group.is_none() || w.paint.color().is_some()))).collect();
    (!movable.is_empty()).then(|| json!({"names": movable, "to": to, "group": group}))
}

/// Are both paints colours, gradients or patterns?
fn same_kind(a: &Paint, b: &Paint) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b) && !a.is_none()
}

/// Can swatch `with` give its paint to swatch `target`? Both are swatches of one kind, neither is
/// None or Registration, and `target` is not a tint swatch (a tint swatch only takes a new name).
fn replaces(d: &Document, with: &str, target: &str) -> bool {
    match (d.swatch(with), d.swatch(target)) {
        (Some(w), Some(t)) => with != target && !w.is_reserved() && !t.is_reserved() && t.tint_of().is_none() && same_kind(&t.paint, &w.paint),
        _ => false,
    }
}

/// Run a drop on the panel: move the swatches, give the paint of a swatch Alt-dropped on another of
/// its kind to that one, or make a swatch of the proxy's paint (a color dropped on a folder or a
/// grouped swatch joins that group).
pub(super) fn apply_drop(app: &mut VectorcraftApp, drop: Drop) {
    let Some(d) = app.session.active().map(|st| &st.doc) else { return };
    let (cmd, params) = match drop {
        // Alt: the swatch grabbed replaces the color, gradient or pattern of the swatch of its kind
        // it is dropped on, which keeps its name and place.
        Drop::Move { with: Some(w), target: Some(t), .. } if replaces(d, &w, &t) => ("swatch.edit", json!({"name": t, "paint": {"swatch": w}})),
        Drop::Move { names, target, after, .. } => match move_params(d, &names, target.as_deref(), after) {
            Some(p) => ("swatch.move", p),
            None => return,
        },
        // Alt: an edited gradient (or colour) replaces the swatch it is dropped on.
        Drop::New { paint, target: Some(t), replace: true } if d.swatch(&t).is_some_and(|w| w.tint_of().is_none() && same_kind(&w.paint, &paint)) => {
            ("swatch.edit", json!({"name": t, "paint": crate::panels::swatch_paint_params(&paint)}))
        }
        Drop::New { paint, target, .. } => {
            let mut p = crate::panels::swatch_paint_params(&paint);
            if paint.color().is_some()
                && let Some(g) = target.and_then(|t| group_of(d, &t))
            {
                p["group"] = json!(g);
            }
            ("swatch.new", p)
        }
        Drop::Pattern { ids } => ("object.pattern.make", json!({"ids": ids, "edit": false})),
    };
    if let Err(e) = app.run(cmd, params) {
        app.status(e);
    }
}

/// While a panel drags something onto art, a chip follows the pointer: the dragged paint (swatches,
/// a Fill/Stroke proxy, the Gradient panel's thumbnail), the fill of the object whose appearance
/// the Appearance panel's thumbnail carries (or of the first art dragged off the canvas), or the
/// thumbnail of a dragged graphic style.
pub(crate) fn drag_preview(app: &VectorcraftApp, ctx: &egui::Context) {
    let Some(d) = egui::DragAndDrop::payload::<PanelDrag>(ctx) else { return };
    let fill_of = |id: &vectorcraft_doc::NodeId| app.session.active().and_then(|st| st.doc.node(*id)).map(|n| n.appearance.fill_paint());
    let (paint, registration) = match &*d {
        // Colour groups paint nothing.
        PanelDrag::Paint { params, .. } if params.is_null() => return,
        PanelDrag::Paint { paint, rows, .. } => (std::borrow::Cow::Borrowed(paint), rows.as_ref().is_some_and(|r| r.grabbed == REGISTRATION)),
        PanelDrag::Appearance(id) => match fill_of(id) {
            Some(p) => (std::borrow::Cow::Owned(p), false),
            None => return,
        },
        PanelDrag::Art(ids) => match ids.first().and_then(fill_of) {
            Some(p) => (std::borrow::Cow::Owned(p), false),
            None => return,
        },
        PanelDrag::GraphicStyle(name) => return pointer_chip(ctx, |ui, r| crate::panels::graphic_styles::paint_style(app, ui, r, name)),
        PanelDrag::Symbol(name) => match app.session.active() {
            Some(st) => {
                return pointer_chip(ctx, |ui, r| crate::panels::symbols::chip(ui, &st.doc, &crate::panels::symbols::document_symbols(st), r, name));
            }
            None => return,
        },
        PanelDrag::LibrarySymbol { library, name } => match vectorcraft_engine::cmd::symbollib::library(&app.session, library) {
            Some((_, lib)) => {
                return pointer_chip(ctx, |ui, r| {
                    crate::panels::symbols::chip(ui, &lib.doc, &crate::panels::symbols::library_symbols(&lib), r, name)
                });
            }
            None => return,
        },
        PanelDrag::Brush { name, .. } => match app.session.active() {
            Some(st) => return pointer_chip(ctx, |ui, r| crate::panels::brushes::document_chip(ui, r, st, name)),
            None => return,
        },
        PanelDrag::LibraryBrush { library, name } => match vectorcraft_engine::cmd::brushlib::library(&app.session, library) {
            Some((_, lib)) => return pointer_chip(ctx, |ui, r| crate::panels::brushes::library_chip(ui, r, &lib, name)),
            None => return,
        },
        PanelDrag::LibraryGraphic { library, item } => return pointer_chip(ctx, |ui, r| crate::panels::libraries::chip(app, ui, r, library, item)),
        PanelDrag::LibraryItem { .. } | PanelDrag::LibraryGroup { .. } => return,
    };
    pointer_chip(ctx, |ui, r| {
        if registration {
            draw_registration(ui, r);
        } else {
            swatch_tile(ui, r, &paint, false, false);
            pattern_thumb(app, ui, r.shrink(1.0), &paint);
        }
    });
}

/// A 16-point chip `draw` paints, following the pointer.
fn pointer_chip(ctx: &egui::Context, draw: impl FnOnce(&Ui, Rect)) {
    let Some(at) = ctx.pointer_hover_pos() else { return };
    let area = egui::Area::new(egui::Id::unique("swatch-drag-preview")).order(egui::Order::Tooltip).fixed_pos(at + vec2(12.0, 12.0));
    area.interactable(false).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        draw(ui, r);
    });
}
