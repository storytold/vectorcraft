use super::tiles::{describe, draw_registration, pattern_thumb};
use super::{
    Document, Entry, Paint, PanelDrag, REGISTRATION, Rect, Response, Sense, Stroke, StrokeKind, SwatchRows, Tokens, Ui, Value, VectorcraftApp, json,
    pos2, pstate, set_pstate, swatch_params, swatch_tile, vec2,
};

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
    /// A paint from elsewhere (a Fill/Stroke proxy, a Recent Colors chip, a Color Guide color, the
    /// Gradient panel's thumbnail) dropped on row `target` (or between rows) becomes a swatch; with Alt held
    /// (`replace`) it replaces swatch `target` when that is of its kind and neither Registration
    /// nor a tint swatch. With Cmd/Ctrl held (`spot`), a solid color other than Registration
    /// becomes a spot color swatch.
    New { paint: Paint, target: Option<String>, replace: bool, spot: bool },
    /// Art dragged off the canvas becomes a pattern swatch made of a copy of it (the art stays);
    /// with Alt held (`replace`) over pattern swatch `target`, a copy of it becomes that pattern's
    /// tile art instead.
    Pattern { ids: Vec<u64>, target: Option<String>, replace: bool },
}

impl Drop {
    /// What releasing `d` before or `after` row `target` (`None`: at the end) does: move the rows
    /// it carries (with Alt held, `replace`, a swatch grabbed may replace `target`), make a swatch of
    /// a paint dragged from elsewhere (a spot color with Cmd/Ctrl held, `spot`), or a pattern swatch
    /// of art dragged off the canvas (appearances: nothing).
    fn of(d: &PanelDrag, target: Option<String>, after: bool, replace: bool, spot: bool) -> Option<Self> {
        match d {
            PanelDrag::Paint { rows: Some(r), .. } => {
                let with = (replace && !r.groups).then(|| r.grabbed.clone());
                Some(Drop::Move { names: r.names.clone(), target, after, with })
            }
            PanelDrag::Paint { paint, .. } => Some(Drop::New { paint: paint.clone(), target, replace, spot }),
            PanelDrag::Art(ids) => Some(Drop::Pattern { ids: ids.iter().map(|id| id.0).collect(), target, replace }),
            _ => None,
        }
    }
}

/// Whether the panel takes drag `d`: paints and art dragged off the canvas, not the Appearance
/// panel's thumbnail.
fn takes(d: &PanelDrag) -> bool {
    matches!(d, PanelDrag::Paint { .. } | PanelDrag::Art(_))
}

/// The drag held over `resp`, if the panel takes it ([`takes`]).
fn held(resp: &Response) -> Option<std::sync::Arc<PanelDrag>> {
    resp.dnd_hover_payload::<PanelDrag>().filter(|d| takes(d))
}

/// The drag in progress, if the panel takes it ([`takes`]), wherever the pointer is.
fn dragged(ui: &Ui) -> Option<std::sync::Arc<PanelDrag>> {
    egui::DragAndDrop::payload::<PanelDrag>(ui.ctx()).filter(|d| takes(d))
}

/// Where the primary button came up this frame and the modifiers held then, when that was over
/// `resp` with no other layer on top (the test [`egui::Context::rect_contains_pointer`] makes of the
/// pointer). A drop lands there with those modifiers, even when the pointer moved on or a key was
/// let go later in the same frame.
fn released_over(ui: &Ui, resp: &Response) -> Option<(egui::Pos2, egui::Modifiers)> {
    let up = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers } => Some((*pos, *modifiers)),
            _ => None,
        })
    })?;
    let ctx = ui.ctx();
    let rect = ctx.layer_transform_to_global(resp.layer_id).map_or(resp.interact_rect, |t| t * resp.interact_rect);
    (rect.is_positive() && rect.contains(up.0) && ctx.layer_id_at(up.0) == Some(resp.layer_id)).then_some(up)
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
    /// A drag of swatches or color groups started on a tile or row.
    pub(super) started: bool,
}

/// Clicks, double-clicks, tooltips and drag and drop of the tile or row `resp` of entry `e`
/// (`items`: the panel's rows, `sel`: its selection, `list`: rows stack downwards). A primary-button
/// drag from it starts a [`PanelDrag`] carrying the rows it moves; see [`tile_drop`] for drags held
/// over it.
pub(super) fn tile_input(ui: &Ui, resp: Response, e: &Entry, items: &[Entry], sel: &[String], list: bool, ev: &mut TileEvents) {
    let name = e.name();
    if matches!(e, Entry::Swatch { .. } | Entry::Folder(_)) && resp.double_clicked() {
        ev.edit = Some(e.clone());
    }
    if resp.drag_started_by(egui::PointerButton::Primary) {
        let with_sel = sel.iter().any(|s| s == name);
        let names = items
            .iter()
            .filter(|x| x.is_folder() == e.is_folder() && (x.name() == name || (with_sel && sel.iter().any(|s| s == x.name()))))
            .map(|x| x.name().to_string())
            .collect();
        egui::DragAndDrop::set_payload(ui.ctx(), drag_of(e, names));
        ev.started = true;
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
    // Only the first click of a run of quick clicks applies and selects the swatch. The second
    // opens the editor, and the later ones (counted as triple clicks) do nothing. Pattern editing
    // selects the tile art, and a later click would paint it with the pattern itself.
    if resp.clicked() && !resp.double_clicked() && !resp.triple_clicked() {
        ev.clicked = Some((e.clone(), ui.input(|i| i.modifiers)));
    }
}

/// A drag held over the tile or row `resp` of entry `e` shows where it would land (before or after
/// it by the pointer's half, or into a folder), by where the pointer is and the keys held now. A
/// release over it ([`released_over`]) sets the drop, by where the button came up and the keys held
/// then.
fn tile_drop(ui: &Ui, resp: &Response, e: &Entry, list: bool, ev: &mut TileEvents) {
    let up = released_over(ui, resp);
    let hover = held(resp);
    let Some(d) = hover.clone().or_else(|| up.and_then(|_| dragged(ui))) else { return };
    let take = || egui::DragAndDrop::take_payload::<PanelDrag>(ui.ctx()).is_some();
    // Art held with Alt over a pattern swatch would replace its tile art, and the swatch is
    // outlined; other art goes to the list (see [`zone_input`]).
    if let PanelDrag::Art(_) = &*d {
        if !matches!(e, Entry::Swatch { paint: Paint::Pattern { .. }, .. }) {
            return;
        }
        if hover.is_some() && crate::panels::alt_held(ui) {
            ev.over = true;
            let t = Tokens::get(ui.ctx());
            ui.painter().rect_stroke(resp.rect.expand(1.0), 0.0, Stroke::new(1.5, t.accent), StrokeKind::Outside);
        }
        if up.is_some_and(|(_, m)| m.alt) && take() {
            ev.drop = Drop::of(&d, Some(e.name().to_string()), false, true, false);
        }
        return;
    }
    let PanelDrag::Paint { rows, .. } = &*d else { return };
    let name = e.name();
    if hover.is_some() {
        ev.over = true;
    }
    // Swatches dropped on one of themselves stay where they are.
    if rows.as_ref().is_some_and(|r| r.names.iter().any(|n| n == name)) {
        if up.is_some() {
            take();
        }
        return;
    }
    let r = resp.rect;
    let side = |at: egui::Pos2| if list { at.y > r.center().y } else { at.x > r.center().x };
    if hover.is_some() {
        let t = Tokens::get(ui.ctx());
        let after = side(ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.center()));
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
    }
    if let Some((at, m)) = up
        && take()
    {
        ev.drop = Drop::of(&d, Some(name.to_string()), side(at), m.alt, m.command);
    }
}

/// Drops between and after the tiles (`zone`: the list's viewport) go to the end of the ungrouped
/// swatches (colour groups to the end of the groups); the list is outlined while one is held there.
/// A tile that took the release first leaves nothing to drop here.
pub(super) fn zone_input(ui: &Ui, zone: &Response, ev: &mut TileEvents) {
    if held(zone).is_some() && !ev.over {
        ui.painter().rect_stroke(zone.rect, 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Inside);
    }
    if let Some((_, m)) = released_over(ui, zone)
        && let Some(d) = dragged(ui)
        && egui::DragAndDrop::take_payload::<PanelDrag>(ui.ctx()).is_some()
    {
        ev.drop = Drop::of(&d, None, false, false, m.command);
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

/// The pattern swatch `name` paints with.
fn swatch_pattern(d: &Document, name: &str) -> Option<String> {
    match &d.swatch(name)?.paint {
        Paint::Pattern { pattern, .. } => Some(pattern.clone()),
        _ => None,
    }
}

/// `paint` as swatch params. A solid color other than Registration is sent as the color the drag
/// carries, not as its base swatch and tint, which the command would resolve again at the drop.
fn shown_color_params(paint: &Paint) -> Value {
    match paint.color() {
        Some(c) if !paint.is_registration() => crate::panels::swatch_paint_params(&Paint::solid(c)),
        _ => crate::panels::swatch_paint_params(paint),
    }
}

/// Run a drop on the panel: move the swatches, give the paint of a swatch Alt-dropped on another of
/// its kind to that one, make a swatch of the proxy's paint (a color dropped on a folder or a
/// grouped swatch joins that group; a color Cmd/Ctrl-dropped becomes a new spot color of the color
/// the drag shows, tints included), or make a pattern swatch of art (Alt-dropped on a pattern
/// swatch: the art becomes that pattern's tile art). With Alt also held, a color dropped on a swatch
/// it can replace replaces that swatch's color with the color the drag shows and makes no spot color.
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
        // Alt: an edited gradient (or colour) replaces the swatch it is dropped on. With Cmd/Ctrl also
        // held, a tint gives the color the drag shows; Alt alone sends the paint as it is.
        Drop::New { paint, target: Some(t), replace: true, spot }
            if d.swatch(&t).is_some_and(|w| !w.is_reserved() && w.tint_of().is_none() && same_kind(&w.paint, &paint)) =>
        {
            let p = if spot { shown_color_params(&paint) } else { crate::panels::swatch_paint_params(&paint) };
            ("swatch.edit", json!({"name": t, "paint": p}))
        }
        Drop::New { paint, target, spot, .. } => {
            // Only a spot color needs a color of its own; other drops keep a tint's link to its base.
            let mut p = if spot { shown_color_params(&paint) } else { crate::panels::swatch_paint_params(&paint) };
            if paint.color().is_some() {
                if let Some(g) = target.and_then(|t| group_of(d, &t)) {
                    p["group"] = json!(g);
                }
                if spot && !paint.is_registration() {
                    p["spot"] = json!(true);
                }
            }
            ("swatch.new", p)
        }
        // Alt: over a pattern swatch, the art becomes that pattern's tile art.
        Drop::Pattern { ids, target, replace } => match target.filter(|_| replace).and_then(|t| swatch_pattern(d, &t)) {
            Some(name) => ("object.pattern.replaceArt", json!({"name": name, "ids": ids})),
            None => ("object.pattern.make", json!({"ids": ids, "edit": false})),
        },
    };
    if let Err(e) = app.run(cmd, params) {
        app.status(e);
    }
}

/// While a panel drags something onto art, a chip follows the pointer: the dragged paint (swatches,
/// a Fill/Stroke proxy, a Recent Colors chip, the Gradient panel's thumbnail), the fill of the
/// object whose appearance the Appearance panel's thumbnail carries (or of the first art dragged
/// off the canvas), or the thumbnail of a dragged graphic style.
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
        // Drawn from the document it was dragged from, whichever document is active now.
        PanelDrag::Symbol { document, name } => match app.session.documents().iter().find(|st| st.uid == *document) {
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
