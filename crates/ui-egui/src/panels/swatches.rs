//! Swatches panel: proxy, Recent Colors (hidden by the panel menu's Show Recent Colors), find
//! field, thumbnail grid or list, colour groups as folders, None/Registration first, footer and
//! panel menu. The browser, its selection rules and its keyboard are the framework's
//! [`SwatchesPanel`]; this file feeds it the document's swatches and runs what it reports.
//!
//! Swatches drag to reorder, into and out of colour groups (`swatch.move`) and onto art (the active
//! proxy's paint command with the object's id) and onto the Gradient panel's ramp; a Fill/Stroke
//! proxy or the Gradient panel's thumbnail dropped on the panel becomes a swatch (`swatch.new`).

use std::sync::Arc;

use craft::behavior::{EditEvent, IconId, Selection};
use craft::color_ui::palette::{SwatchColor, SwatchEntry, SwatchGroup, SwatchKind, SwatchPaint};
use craft::color_ui::ui::{SwatchKey, SwatchesPanel, SwatchesState};
use craft::theme::Srgba;
use craft::ui::choice::SwatchFace;
use craft::ui::contract::{Bind, DropPlace, ListDrag, MemoProps, SelectMode};
use craft::ui::contract::{Entry, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::patterns::presets::{PresetEvent, PresetView, ThumbInfo};
use craft::ui::primitives::{ContentPainterCx, Glyph, IconSpec, PaintShape, StrokeWidth};
use craft::ui::runtime::env::Label;
use craft::ui::sections::{FooterAction, FooterBar};
use craft::ui::style::{DataColor, UiTokens};
use egui::{Color32, Painter, Rect, Response, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, GradientKind, Paint, Swatch};
use vectorcraft_doc::Document;

use super::library_panel::{self, LibraryKind};
use super::{active_paint, apply_click, color_json, proxy_cmd};
use crate::VectorcraftApp;
use crate::panel_drag::PanelDrag;

mod actions;
mod dnd;
mod libraries;

#[cfg(test)]
use actions::{deletable, delete, mergeable, new_swatch, selected_group};
pub(crate) use actions::{menu_rows, menu_states, run};
pub(crate) use libraries::{
    DOCUMENT_SWATCHES, SwatchLibraries, USER_SLOT, colour_libraries, library_entries, limit_key, limit_name, load_library, open_library,
    other_library, user_library, window_menu,
};

/// View modes of the swatch list (and of the library panels: `ui.libraryPanel {view}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum View {
    SmallThumb,
    #[default]
    MediumThumb,
    LargeThumb,
    SmallList,
    LargeList,
}

impl View {
    pub(crate) const ALL: [(View, &'static str); 5] = [
        (View::SmallThumb, "Small Thumbnail View"),
        (View::MediumThumb, "Medium Thumbnail View"),
        (View::LargeThumb, "Large Thumbnail View"),
        (View::SmallList, "Small List View"),
        (View::LargeList, "Large List View"),
    ];
    /// The browser's view and thumbnail zoom this mode stands for: thumbnails of about 12, 16 and 32 points on the
    /// 48-point large thumbnail, the 11, 15.5 and 30 point tiles of production.
    pub(crate) fn preset(self) -> (PresetView, f64) {
        match self {
            View::SmallThumb => (PresetView::Grid, 0.25),
            View::MediumThumb => (PresetView::Grid, 1.0 / 3.0),
            View::LargeThumb => (PresetView::Grid, 2.0 / 3.0),
            View::SmallList => (PresetView::List, 0.5),
            View::LargeList => (PresetView::List, 1.0),
        }
    }
}

/// Swatch-kind filter (Show Swatch Kinds menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    All,
    Color,
    Gradient,
    Pattern,
    Groups,
}

impl Kind {
    const ALL: [(Kind, &'static str); 5] = [
        (Kind::All, "Show All Swatches"),
        (Kind::Color, "Show Color Swatches"),
        (Kind::Gradient, "Show Gradient Swatches"),
        (Kind::Pattern, "Show Pattern Swatches"),
        (Kind::Groups, "Show Color Groups"),
    ];
    /// Does a swatch with `paint` (in a group or not) pass the filter?
    pub fn accepts(self, paint: &Paint, in_group: bool) -> bool {
        match self {
            Kind::All => true,
            Kind::Color => matches!(paint, Paint::Solid { .. } | Paint::None),
            Kind::Gradient => matches!(paint, Paint::Gradient(_)),
            Kind::Pattern => matches!(paint, Paint::Pattern { .. }),
            Kind::Groups => in_group,
        }
    }
}

/// The document and the art selected in it, which the panel's selection follows.
type SelectedArt = (u64, Vec<u64>);

/// What the panel keeps between passes (`UiState::swatches_panel`): the framework's state (browser view, open
/// groups, the selected rows), the swatch kept lit for the active paint, what the last pass saw and the panel's own
/// switches.
#[derive(Clone, Debug, Default)]
pub(crate) struct Model {
    /// The framework's panel state; its `selection` holds the selected groups and swatches by name.
    pub(crate) state: SwatchesState,
    /// The Show Swatch Kinds filter.
    pub(crate) kind: Kind,
    /// Whether the find field shows (the panel menu's Show Find Field).
    pub(crate) show_find: bool,
    /// The view last applied to the browser, so a view command and the browser's own toolbar do not fight.
    view: Option<View>,
    /// The paints new art gets and the swatch lit for them ([`Model::kept`]).
    kept: Option<(String, Option<String>)>,
    /// The art and the active swatch the last pass saw ([`Model::focus`]).
    seen: Option<(Option<SelectedArt>, Option<String>)>,
}

impl Model {
    fn art(app: &VectorcraftApp) -> Option<SelectedArt> {
        app.session.active().map(|st| (st.uid, st.selection.objects.iter().map(|id| id.0).collect()))
    }

    /// The selected names, in click order, whether they are groups or swatches.
    pub(crate) fn names(&self) -> Vec<String> {
        self.state.selection.items.iter().map(|k| name_of(k).to_string()).collect()
    }

    /// Selects the swatches `names` (the copies a duplicate made, the unused ones).
    pub(crate) fn set_swatches(&mut self, names: Vec<String>) {
        let keys: Vec<SwatchKey> = names.into_iter().map(SwatchKey::Swatch).collect();
        self.state.selection = Selection::new(keys.clone(), keys.last().cloned(), keys.first().cloned());
    }

    /// Whether the selection is still the one the panel showed last: neither another document nor other art has been
    /// selected since, which [`Model::focus`] clears it for on the next pass.
    fn selection_is_current(&self, app: &VectorcraftApp) -> bool {
        let art = Self::art(app);
        self.seen.as_ref().is_none_or(|(last, _)| *last == art)
    }

    /// The swatch highlighted for the active paint (`active`): with art selected, its swatch; once the art is
    /// deselected, still that swatch, until another document shows or the paints for new art change (a swatch or
    /// colour picked with nothing selected). Deselecting doesn't jump to the swatch of the paint new art gets (White).
    fn kept(&mut self, app: &VectorcraftApp, active: Option<String>) -> Option<String> {
        let Some(st) = app.session.active() else { return active };
        let p = &app.session.paint;
        let defaults = format!("{}|{:?}|{:?}|{}", st.uid, p.fill, p.stroke, app.session.fill_active);
        if !st.selection.objects.is_empty() {
            self.kept = Some((defaults, active.clone()));
            return active;
        }
        match &self.kept {
            Some((d, kept)) if *d == defaults => kept.clone(),
            Some(_) => {
                self.kept = None;
                active
            }
            None => active,
        }
    }

    /// The swatch the browser scrolls to this pass, once: the active paint's swatch when it changed, or when other art
    /// was selected. Selecting other art also clears the panel's own selection, so its swatch is the one highlighted.
    fn focus(&mut self, app: &VectorcraftApp, active: Option<&str>) -> Option<String> {
        let art = Self::art(app);
        let reselected = self.seen.as_ref().is_some_and(|(last, _)| *last != art);
        if reselected {
            self.state.selection = Selection::default();
        }
        let changed = reselected || self.seen.as_ref().is_none_or(|(_, swatch)| swatch.as_deref() != active);
        self.seen = Some((art, active.map(str::to_string)));
        active.filter(|_| changed).map(str::to_string)
    }

    /// Selected `old` renamed to `new`, or dropped from the selection with `None`; the lead and the anchor follow.
    fn rename(&mut self, old: &str, new: Option<&str>) {
        let moved = |k: &SwatchKey| -> Option<SwatchKey> {
            if name_of(k) != old {
                return Some(k.clone());
            }
            let new = new?.to_string();
            Some(match k {
                SwatchKey::Group(_) => SwatchKey::Group(new),
                SwatchKey::Swatch(_) => SwatchKey::Swatch(new),
                _ => k.clone(),
            })
        };
        let s = &mut self.state.selection;
        s.items = s.items.iter().filter_map(moved).collect();
        s.lead = s.lead.as_ref().and_then(moved);
        s.anchor = s.anchor.as_ref().and_then(moved);
    }
}

fn name_of(key: &SwatchKey) -> &str {
    match key {
        SwatchKey::Group(n) | SwatchKey::Swatch(n) => n,
        _ => "",
    }
}

/// The swatches and colour groups selected in the panel, in click order, without names that no longer exist
/// (deleted, renamed or undone).
pub(crate) fn selected(app: &VectorcraftApp) -> Vec<String> {
    let Some(st) = app.session.active() else { return vec![] };
    let mut names = app.ui.swatches_panel.names();
    names.retain(|n| n == REGISTRATION || st.doc.swatch_name_taken(n));
    names
}

/// The names the panel highlights: `sel` plus the swatches of selected colour groups (clicking a folder selects the
/// whole group).
fn highlighted<'a>(d: &'a Document, sel: &'a [String]) -> Vec<&'a str> {
    let mut out: Vec<&str> = sel.iter().map(String::as_str).collect();
    for g in d.swatch_groups.iter().filter(|g| sel.contains(&g.name)) {
        out.extend(g.swatches.iter().map(|w| w.name.as_str()));
    }
    out
}

/// The `swatch`'s paint as the panel shows it, and whether it is global and spot. A tint swatch shows its base's kind;
/// a spot colour shows the colour its art shows (a Lab one's CMYK equivalent when the Spot Colors options use those).
fn shown(d: &Document, s: &Swatch) -> (Paint, bool, bool) {
    let (global, spot) = s.tint_of().and_then(|(base, _)| d.swatch(base)).map_or((s.global, s.spot), |b| (b.global, b.spot));
    let paint = match s.paint {
        Paint::Solid { color, swatch: None, .. } if spot => Paint::solid(d.linked_color(color, true)),
        _ => s.paint.clone(),
    };
    (paint, global, spot)
}

fn swatch_color(c: &Color) -> SwatchColor {
    match *c {
        Color::Rgb { r, g, b } => SwatchColor::Rgb([r, g, b]),
        Color::Cmyk { c, m, y, k } => SwatchColor::Cmyk([c, m, y, k]),
        Color::Gray { k } => SwatchColor::Gray(k),
        Color::Lab { l, a, b } => SwatchColor::Lab([l, a, b]),
    }
}

fn entry(d: &Document, s: &Swatch) -> SwatchEntry {
    let (paint, global, spot) = shown(d, s);
    let held = match &paint {
        Paint::Solid { color, .. } => match s.tint_of() {
            Some((base, tint)) => SwatchPaint::Tint { base: base.to_string(), tint, color: swatch_color(color) },
            None => SwatchPaint::Color(swatch_color(color)),
        },
        Paint::Gradient(_) => SwatchPaint::Gradient,
        Paint::Pattern { .. } => SwatchPaint::Pattern,
        Paint::None => SwatchPaint::None,
    };
    let mut e = SwatchEntry::with_paint(s.name.clone(), held);
    e.kind = if spot {
        SwatchKind::Spot
    } else if global {
        SwatchKind::Global
    } else {
        SwatchKind::Process
    };
    e
}

fn registration_entry() -> SwatchEntry {
    let mut e = SwatchEntry::with_paint(REGISTRATION, SwatchPaint::Color(SwatchColor::Cmyk([1.0; 4])));
    e.kind = SwatchKind::Registration;
    e
}

/// The panel's groups for the kind filter and the find field's `query` (a case-insensitive name match; a colour
/// group whose name matches shows all its swatches). The first group holds the swatches outside any colour group:
/// None first, then Registration, then the rest (the reference app's order).
fn groups(app: &VectorcraftApp, kind: Kind, query: &str) -> Vec<SwatchGroup> {
    let Some(st) = app.session.active() else { return vec![] };
    let d = &st.doc;
    let q = query.trim().to_lowercase();
    let found = |n: &str| q.is_empty() || n.to_lowercase().contains(&q);
    let mut top = vec![];
    let (specials, rest): (Vec<_>, Vec<_>) = d.swatches.iter().partition(|s| s.paint.is_none());
    top.extend(specials.iter().filter(|s| kind.accepts(&s.paint, false) && found(&s.name)).map(|s| entry(d, s)));
    if matches!(kind, Kind::All | Kind::Color) && found(REGISTRATION) {
        top.push(registration_entry());
    }
    top.extend(rest.iter().filter(|s| kind.accepts(&s.paint, false) && found(&s.name)).map(|s| entry(d, s)));
    // PLAN: assumed the panel shows the first group, named "", without a folder header (the ungrouped swatches).
    let mut out = vec![SwatchGroup { name: String::new(), entries: top }];
    for g in &d.swatch_groups {
        let all = found(&g.name);
        let items: Vec<_> = g.swatches.iter().filter(|s| kind.accepts(&s.paint, true) && (all || found(&s.name))).collect();
        // Empty groups show only among all swatches or groups, and when the find field names them.
        if items.is_empty() && !(all && matches!(kind, Kind::Groups | Kind::All)) {
            continue;
        }
        out.push(SwatchGroup { name: g.name.clone(), entries: items.into_iter().map(|s| entry(d, s)).collect() });
    }
    out
}

/// The swatch the active paint comes from: a linked colour's global swatch (a tint's tint swatch when there is one),
/// a process colour's swatch of that colour (a colour no swatch has: none, so text in a colour of its own highlights
/// nothing), a gradient's gradient swatch, or None; nothing for selected images.
fn active_swatch(app: &VectorcraftApp) -> Option<String> {
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

/// The paint command params that apply swatch `name` (`paint.setFill`).
fn swatch_params(d: &Document, name: &str) -> Value {
    if d.swatch(name).is_some_and(|s| s.paint.is_none()) { json!({"none": true}) } else { json!({"swatch": name}) }
}

/// Apply a clicked swatch to the active proxy (`alt`: the inactive one). An Alt-click on a color swatch with a gradient
/// stop selected recolors the stop as dropping the swatch on it does, and with a freeform gradient point selected it
/// recolors the point. A color swatch also becomes the Color Guide's base color, linked to the spot color it is or is a
/// tint of.
fn apply(app: &mut VectorcraftApp, name: &str, alt: bool) {
    let Some(d) = app.session.active().map(|st| &st.doc) else { return };
    let mut params = swatch_params(d, name);
    let color = if name == REGISTRATION { Paint::registration().color() } else { d.swatch(name).and_then(|s| s.paint.color()) };
    let link = d.swatch_link(name);
    if let Some(c) = color {
        super::color_guide::choose_linked_base(app, c, link);
    }
    if alt
        && let Some(c) = color
        && super::gradient::recolor_selected(app, c, &params)
    {
        return;
    }
    params["focus"] = json!(!alt);
    app.run(proxy_cmd(app, alt), params).ok();
}

/// The panel's body: the proxy, Recent Colors, the browser and the footer.
fn body(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        super::empty_state(ui, "swatch-book", tl!("No document"), tl!("Open a document to see its swatches."));
        return;
    }
    super::proxy(app, ui);
    // A recent colour paints the active proxy (Alt: the inactive one), as a swatch does, and becomes the Color Guide's
    // base color. The panel menu's Show Recent Colors hides the row.
    if app.ui.swatches_show_recent_colors
        && let Some(c) = super::recent_colors(app, ui)
    {
        super::color_guide::choose_base(app, c);
        apply_click(app, ui, json!({"color": color_json(&c)}));
    }
    // The model leaves the app while the browser borrows the document, and comes back before the events run.
    let mut model = std::mem::take(&mut app.ui.swatches_panel);
    let view = app.ui.swatches_view;
    if model.view != Some(view) {
        (model.state.browser.view, model.state.browser.zoom) = view.preset();
        model.view = Some(view);
    }
    let active = active_swatch(app);
    let active = model.kept(app, active);
    let focus = model.focus(app, active.as_deref());
    let query = if model.show_find { model.state.browser.filter.clone() } else { String::new() };
    let data = groups(app, model.kind, &query);
    // Without a selection the swatch of the active paint is highlighted.
    let picked = model.names();
    let base = if picked.is_empty() { active.into_iter().collect() } else { picked };
    let Some(st) = app.session.active() else {
        app.ui.swatches_panel = model;
        return;
    };
    let thumbs = Thumbs {
        revision: (st.uid, st.revision),
        lit: highlighted(&st.doc, &base).into_iter().map(str::to_string).collect(),
        doc: Arc::clone(&st.doc),
    };
    let dragged_doc = Arc::clone(&thumbs.doc);
    let document = st.uid;
    let shown_app: &VectorcraftApp = app;
    let out = SwatchesPanel::new(&data, Bind::bound(&mut model.state), Srgba::rgb(0.0, 0.0, 0.0))
        .drag_payload(move |drag: &ListDrag<SwatchKey>| Some(dnd::drag_of(&dragged_doc, document, &drag.grabbed, dragged_rows(drag))))
        .thumb(thumbs, thumb)
        .row_icons(|key| match key {
            SwatchKey::Swatch(name) => row_icons(shown_app, name),
            _ => vec![],
        })
        .row_items(|key| row_items(shown_app, key))
        .takes_drop::<PanelDrag>(DropPlace::Between, |drag, _| dnd::accepts(drag, document))
        .takes_drop::<PanelDrag>(DropPlace::Onto, |drag, _| dnd::accepts(drag, document))
        .select(SelectMode::Multi)
        .search(model.show_find)
        .reveal(focus.map(SwatchKey::Swatch))
        .footer(None)
        .show(ui);
    app.ui.swatches_panel = model;
    let shown = shown_keys(&data);
    if let Some((at, _, alt)) = out.used
        && let Some(entry) = data.get(at.group).and_then(|g| g.entries.get(at.index))
    {
        apply(app, &entry.name, alt);
    }
    for event in out.events {
        on_event(app, &shown, event);
    }
    footer(app, ui);
}

/// The rows `drag` moves: the dragged rows of the grabbed row's kind (colour groups or swatches), in panel order.
fn dragged_rows(drag: &ListDrag<SwatchKey>) -> Vec<String> {
    let groups = matches!(drag.grabbed, SwatchKey::Group(_));
    drag.keys.iter().filter(|k| matches!(k, SwatchKey::Group(_)) == groups).map(|k| name_of(k).to_string()).collect()
}

/// The rows the browser shows for `groups`, in its order: each group's row, then its swatches.
fn shown_keys(groups: &[SwatchGroup]) -> Vec<SwatchKey> {
    groups
        .iter()
        .flat_map(|g| std::iter::once(SwatchKey::Group(g.name.clone())).chain(g.entries.iter().map(|e| SwatchKey::Swatch(e.name.clone()))))
        .collect()
}

/// Runs a browser event over the `shown` rows (a click on a swatch comes as the panel's `used`): a double click opens
/// its editor, a row menu item runs on its row, and drops and reorders change the document.
fn on_event(app: &mut VectorcraftApp, shown: &[SwatchKey], event: PresetEvent<SwatchKey>) {
    match event {
        PresetEvent::Activate { key, .. } => activate(app, &key),
        PresetEvent::Menu { target, command, .. } => menu_command(app, &target, &command),
        PresetEvent::Dropped { at, payload, mods, .. } => {
            if let Some(drag) = payload.get::<PanelDrag>() {
                dnd::on_dropped(app, shown, &at, drag, mods);
            }
        }
        // Only a finished gesture changes the document; Begin and Update are previews.
        PresetEvent::Reorder(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => dnd::on_reorder(app, shown, &after),
        _ => {}
    }
}

/// A double click or Enter: Swatch Options for a swatch, Edit or Apply Color Group for a group.
fn activate(app: &mut VectorcraftApp, key: &SwatchKey) {
    match key {
        SwatchKey::Group(g) => actions::edit_group(app, g),
        // PLAN: the sheet sends a pattern swatch to `object.pattern.edit`; production opens Swatch Options for it.
        SwatchKey::Swatch(n) => {
            app.run("ui.swatchOptions", json!({"name": n})).ok();
        }
        _ => {}
    }
}

/// The row menu's items for `key`: item actions target the clicked swatch, independently of the paint or other
/// selected chips. A swatch can be deleted while its menu is open, so the document is asked each pass.
fn row_items(app: &VectorcraftApp, key: &SwatchKey) -> Vec<Entry<String>> {
    let SwatchKey::Swatch(name) = key else { return vec![] };
    let enabled = app.session.active().and_then(|st| st.doc.swatch(name)).is_some_and(|s| !s.is_reserved());
    [("options", tl!("Swatch Options…")), ("duplicate", tl!("Duplicate Swatch")), ("delete", tl!("Delete Swatch"))]
        .into_iter()
        .map(|(id, label)| Entry::new(id.to_string(), label).enabled(enabled))
        .collect()
}

fn menu_command(app: &mut VectorcraftApp, key: &SwatchKey, command: &str) {
    let name = name_of(key);
    match command {
        "options" => {
            app.run("ui.swatchOptions", json!({"name": name})).ok();
        }
        "duplicate" => {
            app.run("swatch.duplicate", json!({"name": name})).ok();
        }
        "delete" => actions::delete(app, vec![name.to_string()], false),
        _ => {}
    }
}

/// What the swatch thumbnails read: the active document at its revision and the names drawn selected.
#[derive(MemoProps)]
struct Thumbs {
    /// The document's uid and revision.
    revision: (u64, u64),
    /// The selected swatches, those of selected colour groups, or the active paint's swatch.
    lit: Vec<String>,
    #[memo_props(covered_by = "revision", reason = "a document's uid and revision change with every edit of it")]
    doc: Arc<Document>,
}

/// The thumbnail of swatch cell `info` (a colour group is the browser's folder row, not a thumbnail).
fn thumb(t: &Thumbs, cx: &ContentPainterCx<'_>, rect: Rect, info: ThumbInfo<'_, SwatchKey>) {
    let SwatchKey::Swatch(name) = info.key else { return };
    let selected = info.selected || t.lit.contains(name);
    if name == REGISTRATION {
        paint_swatch(cx, rect, &Paint::registration(), selected, (false, false), None);
        return;
    }
    let Some(s) = t.doc.swatch(name) else { return };
    let (paint, global, spot) = shown(&t.doc, s);
    paint_swatch(cx, rect, &paint, selected, (global, spot), Some(&t.doc));
}

/// A swatch's tile: the framework's swatch face (its edge and selected ring) in the paint's colour, with a gradient,
/// a pattern (its rendered tile from `pattern`'s document), None's slash or Registration's mark inside, and the corner
/// of a global colour (with the dot of a spot one). `kind` is (global, spot).
pub(super) fn paint_swatch(cx: &ContentPainterCx<'_>, rect: Rect, paint: &Paint, selected: bool, kind: (bool, bool), pattern: Option<&Document>) {
    let face = paint.color().filter(|_| !paint.is_registration()).map_or(Srgba::rgb(1.0, 1.0, 1.0), |c| super::srgba(&c));
    SwatchFace::new(face).selected(selected).paint(cx, rect);
    let inner = rect.shrink(cx.stroke_width(StrokeWidth::Medium));
    match paint {
        _ if paint.is_registration() => registration_mark(cx, inner),
        Paint::Solid { .. } => {}
        Paint::None => none_slash(cx, inner),
        Paint::Pattern { .. } if pattern.is_some_and(|doc| pattern_thumb_in(doc, cx, inner, paint)) => {}
        _ => crate::paint_preview::paint_rect(cx, inner, paint),
    }
    if kind.0 || kind.1 {
        global_mark(cx, rect, kind.1);
    }
}

/// Draws a pattern swatch from the pattern of its name in `doc` (cached by the definition's identity and size);
/// `false` when the pattern renders nothing.
fn pattern_thumb_in(doc: &Document, cx: &ContentPainterCx<'_>, r: Rect, paint: &Paint) -> bool {
    use std::collections::HashMap;
    type Key = (String, Vec<usize>, String, u32);
    thread_local! {
        static CACHE: crate::graphics::TexCache<HashMap<Key, Option<egui::TextureHandle>>> = crate::graphics::TexCache::default();
    }
    let Paint::Pattern { pattern, .. } = paint else { return false };
    let Some(def) = doc.pattern(pattern) else { return false };
    let ctx = cx.painter.ctx();
    let px = (r.width() * ctx.pixels_per_point()).round().max(4.0) as u32;
    let key = (
        pattern.clone(),
        def.art.iter().map(|a| Arc::as_ptr(a) as usize).collect(),
        format!("{:?}{:?}{:?}", def.tile, def.tile_type, def.overlap),
        px,
    );
    let tex = CACHE.with(|c| c.borrow().get(&key).cloned()).unwrap_or_else(|| {
        let tex = vectorcraft_render::render_pattern_swatch(doc, pattern, px).map(|img| {
            let color = crate::canvas::color_image(&img);
            ctx.load_texture(format!("pattern-swatch-{pattern}-{px}"), color, egui::TextureOptions::LINEAR)
        });
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 256 {
                c.clear();
            }
            c.insert(key, tex.clone());
        });
        tex
    });
    let Some(tex) = tex else { return false };
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    cx.data_shapes([Shape::rect_filled(r, 0.0, Color32::WHITE), Shape::image(tex.id(), r, uv, Color32::WHITE)]);
    true
}

/// The red of None's slash on a swatch tile and of a disabled opacity mask's cross, drawn on the document's white or
/// black paper. It is the same in every theme.
pub(super) const OFF_MARK: Color32 = Color32::from_rgb(0xe0, 0x20, 0x20);

/// None's slash across `r`, from its lower left to its upper right, in [`OFF_MARK`] at the medium stroke width.
fn none_slash(cx: &ContentPainterCx<'_>, r: Rect) {
    cx.with_draw(|draw| draw.stroke(&PaintShape::Path(vec![r.left_bottom(), r.right_top()]), StrokeWidth::Medium, DataColor::new(OFF_MARK)));
}

/// Registration's mark: a black target on white.
fn registration_mark(cx: &ContentPainterCx<'_>, r: Rect) {
    let c = r.center();
    let s = Stroke::new(cx.stroke_width(StrokeWidth::Thin), Color32::BLACK);
    let inset = cx.stroke_width(StrokeWidth::Medium);
    cx.data_shapes([
        Shape::circle_stroke(c, r.width() * 0.28, s),
        Shape::line_segment([pos2(r.left() + inset, c.y), pos2(r.right() - inset, c.y)], s),
        Shape::line_segment([pos2(c.x, r.top() + inset), pos2(c.x, r.bottom() - inset)], s),
    ]);
}

/// The white corner of a global colour's tile; a spot colour has a dot in it.
fn global_mark(cx: &ContentPainterCx<'_>, cell: Rect, spot: bool) {
    let k = cell.shrink(cx.stroke_width(StrokeWidth::Thin)).right_bottom();
    let side = cell.width().min(cell.height()) * 0.3;
    let mut shapes = vec![Shape::convex_polygon(vec![k, k - vec2(side, 0.0), k - vec2(0.0, side)], Color32::WHITE, Stroke::NONE)];
    if spot {
        shapes.push(Shape::circle_filled(k - vec2(side, side) * 0.32, side * 0.2, Color32::BLACK));
    }
    cx.data_shapes(shapes);
}

/// The ink colours of the CMYK glyph (cyan, magenta, yellow, black) and the RGB glyph's bars.
pub(crate) const CMYK_INKS: [Color32; 4] =
    [Color32::from_rgb(0, 174, 239), Color32::from_rgb(236, 0, 140), Color32::from_rgb(255, 242, 0), Color32::BLACK];

const RGB_BARS: [Color32; 3] = [Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 200, 0), Color32::from_rgb(0, 0, 255)];

/// Lab's opponent axes: green | red (a) over blue | yellow (b).
const LAB_AXES: [Color32; 4] =
    [Color32::from_rgb(0, 166, 81), Color32::from_rgb(230, 30, 60), Color32::from_rgb(0, 90, 220), Color32::from_rgb(250, 205, 0)];

/// A colour model's glyph: its fixed data colours in a box, which a tinted SVG cannot carry. It ignores the tint
/// except for the outline.
#[derive(MemoProps)]
struct ModelGlyph {
    cells: &'static [Color32],
    /// Three vertical bars instead of a two-by-two grid of quarters.
    bars: bool,
}

impl Glyph for ModelGlyph {
    fn paint(&self, painter: &Painter, _tokens: &UiTokens, rect: Rect, color: Color32, _mirrored: bool) {
        let side = rect.width().min(rect.height());
        let r = Rect::from_center_size(rect.center(), vec2(side, side));
        if self.bars {
            let w = r.width() / self.cells.len().max(1) as f32;
            for (i, c) in self.cells.iter().enumerate() {
                painter.rect_filled(Rect::from_min_size(r.min + vec2(i as f32 * w, 0.0), vec2(w, r.height())), 0.0, *c);
            }
        } else {
            let q = r.size() / 2.0;
            for (i, c) in self.cells.iter().enumerate() {
                painter.rect_filled(Rect::from_min_size(r.min + vec2((i % 2) as f32 * q.x, (i / 2) as f32 * q.y), q), 0.0, *c);
            }
        }
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, color), StrokeKind::Outside);
    }
}

static CMYK_MODE: ModelGlyph = ModelGlyph { cells: &CMYK_INKS, bars: false };
static LAB_MODE: ModelGlyph = ModelGlyph { cells: &LAB_AXES, bars: false };
static RGB_MODE: ModelGlyph = ModelGlyph { cells: &RGB_BARS, bars: true };

/// The CMYK glyph: four ink quarters, the mark of a colour that prints as process inks.
pub(crate) fn cmyk_glyph() -> IconSpec<'static> {
    IconSpec::glyph(&CMYK_MODE)
}

/// A swatch's kind in words, for list tooltips ("Global Process Color, CMYK").
pub(crate) fn describe(paint: &Paint, global: bool, spot: bool) -> String {
    match paint {
        Paint::Solid { color, .. } => {
            let kind = if spot {
                tl!("Spot Color")
            } else if global {
                tl!("Global Process Color")
            } else {
                tl!("Process Color")
            };
            crate::i18n::fmt(tl!("{kind}, {model}"), &[("kind", kind), ("model", color.model_name())])
        }
        Paint::Gradient(g) => crate::i18n::fmt(
            tl!("{kind} Gradient"),
            &[("kind", crate::i18n::tr_ctx(crate::i18n::current(), "gradient-kind", g.gradient.kind.label()))],
        ),
        Paint::Pattern { .. } => tl!("Pattern").into(),
        Paint::None => tl!("None").into(),
    }
}

/// The icons at the end of a list row, each with its name: spot (a dot in a ring), global (a square with a filled
/// corner) or process colours (a square), gradients and patterns; then a colour's model (CMYK as four ink quarters, RGB
/// as three bars, Gray as a grey square, Lab as its two opponent axes).
pub(super) fn kind_icons(paint: &Paint, global: bool, spot: bool) -> Vec<(IconSpec<'static>, String)> {
    let svg = |id: &'static str| IconSpec::svg(IconId::new(id));
    match paint {
        Paint::Solid { color, .. } => {
            let (kind, name) = if spot {
                ("dc-swatch-spot", tl!("Spot Color"))
            } else if global {
                ("dc-swatch-global", tl!("Global Process Color"))
            } else {
                ("dc-swatch-process", tl!("Process Color"))
            };
            let model = match color {
                Color::Cmyk { .. } => cmyk_glyph(),
                Color::Lab { .. } => IconSpec::glyph(&LAB_MODE),
                Color::Rgb { .. } => IconSpec::glyph(&RGB_MODE),
                Color::Gray { .. } => IconSpec::swatch(Srgba::rgb(0.5, 0.5, 0.5)),
            };
            vec![(svg(kind), name.to_string()), (model, color.model_name().to_string())]
        }
        Paint::Gradient(g) => {
            let icon = match g.gradient.kind {
                GradientKind::Linear => "dc-grad-linear",
                GradientKind::Radial => "dc-grad-radial",
                GradientKind::Conic => "dc-grad-conic",
                GradientKind::Diamond => "dc-grad-diamond",
                GradientKind::Freeform => "dc-grad-freeform",
            };
            vec![(svg(icon), describe(paint, global, spot))]
        }
        Paint::Pattern { .. } => vec![(svg("grid-3x3"), tl!("Pattern").to_string())],
        Paint::None => vec![],
    }
}

fn row_icons(app: &VectorcraftApp, name: &str) -> Vec<(IconSpec<'static>, String)> {
    let Some(st) = app.session.active() else { return vec![] };
    let Some(s) = st.doc.swatch(name) else { return vec![] };
    let (paint, global, spot) = shown(&st.doc, s);
    kind_icons(&paint, global, spot)
}

/// A row of a framework menu: whether it was picked. `checked` shows a radio mark.
pub(super) fn item(ui: &mut Ui, label: impl Into<Label>, enabled: bool, checked: Option<bool>) -> bool {
    let mut row = MenuItem::new(label).enabled(enabled);
    if let Some(on) = checked {
        row = row.radio_mark(on);
    }
    row.show(ui).ran
}

/// The menu named `label` that the footer button `opener` pops up, with the rows of `body`.
fn popup(ui: &mut Ui, label: &'static str, opener: Option<&Response>, mut body: impl FnMut(&mut Ui)) {
    if let Some(opener) = opener {
        Menu::new(label).opened_by(opener, Trigger::Press).show(ui, |ui, _| body(ui));
    }
}

// The footer's actions, in order.
const LIBRARIES: usize = 0;
const KINDS: usize = 1;
const EDIT: usize = 2;
const DELETE: usize = 3;
const NEW: usize = 4;
const NEW_GROUP: usize = 5;

/// The panel's footer: Swatch Libraries, Show Swatch Kinds, Edit Color Group or Swatch Options, New Color Group, New
/// Swatch and Delete. Alt-click on Delete deletes without asking; Cmd/Ctrl-click on New Swatch makes a spot color, and
/// Cmd/Ctrl+Shift-click a global process color. Swatches dragged from the list and dropped on New Swatch are
/// duplicated, and rows dropped on Delete are deleted, also with nothing selected.
fn footer(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel = selected(app);
    let kind = app.ui.swatches_panel.kind;
    let del = actions::deletable(app, &sel);
    let opts = actions::editable(app, &sel);
    let group = actions::selected_group(app, &sel).filter(|_| opts.is_none());
    let icon = |id: &'static str| IconSpec::svg(IconId::new(id));
    let rows_drag = |d: &PanelDrag| matches!(d, PanelDrag::Paint { rows: Some(_), .. });
    let edit = if group.is_some() {
        FooterAction::new(icon("palette"), tl!("Edit or Apply Color Group"))
    } else {
        FooterAction::new(icon("dc-options"), tl!("Swatch Options")).enabled(opts.is_some())
    };
    // Libraries, Show Swatch Kinds and the edit action sit at the start edge; Delete at the end edge, then New Swatch
    // and New Color Group before it.
    let actions = [
        FooterAction::new(icon("library"), SwatchLibraries::MENU).pressed(library_panel::any_open::<SwatchLibraries>(&app.ui)),
        FooterAction::new(icon("dc-swatch-kinds"), tl!("Show Swatch Kinds")).pressed(kind != Kind::All),
        edit,
        FooterAction::new(icon("trash-2"), tl!("Delete Swatch"))
            .enabled(!del.is_empty())
            .takes_drops_disabled(del.is_empty())
            .accepts_drops::<PanelDrag>(rows_drag),
        FooterAction::new(icon("dc-new-item"), tl!("New Swatch")).accepts_drops::<PanelDrag>(rows_drag),
        FooterAction::new(icon("dc-folder"), tl!("New Color Group")),
    ];
    let out = FooterBar::actions(actions).at_start(EDIT + 1).show_footer(ui);
    match out.picked {
        Some(DELETE) if !del.is_empty() => actions::delete(app, del, out.mods.alt),
        Some(NEW) => actions::new_swatch(app, &sel, out.mods),
        Some(NEW_GROUP) => {
            app.run("ui.newColorGroup", json!({"swatches": del})).ok();
        }
        Some(EDIT) => match (group, opts) {
            (Some(g), _) => actions::edit_group(app, &g),
            (None, Some(n)) => {
                app.run("ui.swatchOptions", json!({"name": n})).ok();
            }
            _ => {}
        },
        _ => {}
    }
    if let Some(drop) = &out.dropped
        && let Some(drag) = drop.payload::<PanelDrag>()
    {
        match drop.index {
            DELETE => dnd::on_footer_drop(app, dnd::FooterTarget::Delete, drag),
            NEW => dnd::on_footer_drop(app, dnd::FooterTarget::New, drag),
            _ => {}
        }
    }
    popup(ui, tl!("Show Swatch Kinds"), out.responses.get(KINDS), |ui| {
        for (k, label) in Kind::ALL {
            if item(ui, label, true, Some(k == kind)) {
                app.ui.swatches_panel.kind = k;
            }
        }
    });
    popup(ui, tl!("Swatch Libraries"), out.responses.get(LIBRARIES), |ui| {
        ui.set_min_width(200.0);
        library_panel::library_menu::<SwatchLibraries>(app, ui);
    });
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    body(app, ui);
}

/// The color group selected alone in the panel (what Color Group Options opens on), if any.
pub(crate) fn sole_selected_group(app: &VectorcraftApp) -> Option<String> {
    if !app.ui.swatches_panel.selection_is_current(app) {
        return None;
    }
    actions::sole_group(app, &selected(app))
}

/// Command `id` ran with `params` and replied `reply`: a color group that `swatch.renameGroup` or `swatch.editGroup`
/// renamed stays selected in the panel under its new name, and Shift-click still extends the selection from it.
/// Ungroup Color Group and Delete Swatch take the names they removed out of the selection, so a group that later takes
/// such a name is not selected in its place. The steps of a `command.batch` are followed in order up to the first step
/// that is not one of [`KEEPS_GROUPS`]: such a step may switch documents or replace a group, and the selection stays as
/// it is from there.
pub(crate) fn after_command(app: &mut VectorcraftApp, id: &str, params: &Value, reply: &Value) {
    match id {
        "swatch.renameGroup" | "swatch.editGroup" => {
            if let (Some(old), Some(new)) = (params["group"].as_str(), reply["name"].as_str()) {
                app.ui.swatches_panel.rename(old, Some(new));
            }
        }
        "swatch.ungroup" => {
            if let Some(name) = params["name"].as_str() {
                app.ui.swatches_panel.rename(name, None);
            }
        }
        "swatch.delete" => {
            let names = params["names"].as_array().map(Vec::as_slice).unwrap_or_default();
            for name in names.iter().chain(std::iter::once(&params["name"])).filter_map(Value::as_str) {
                app.ui.swatches_panel.rename(name, None);
            }
        }
        "command.batch" => {
            let steps = params["commands"].as_array().map(Vec::as_slice).unwrap_or_default();
            let results = reply["results"].as_array().map(Vec::as_slice).unwrap_or_default();
            for (step, r) in steps.iter().zip(results) {
                let id = step["command"].as_str().unwrap_or_default();
                if !KEEPS_GROUPS.contains(&id) {
                    break;
                }
                after_command(app, id, &step["params"], r);
            }
        }
        _ => {}
    }
}

/// The batch steps the panel's selection follows: they act on the active document only, and
/// remove no color group except the ones they name.
const KEEPS_GROUPS: &[&str] = &[
    "swatch.renameGroup",
    "swatch.ungroup",
    "swatch.delete",
    "swatch.new",
    "swatch.newGroup",
    "swatch.edit",
    "swatch.move",
    "swatch.merge",
    "swatch.duplicate",
    "swatch.addUsedColors",
    "swatch.sortByKind",
    "swatch.sortByName",
    "swatch.editGroup",
    "swatch.spotOptions",
    "swatch.setSpot",
    "swatch.list",
    "swatch.unused",
];

/// The panel in a Fill/Stroke chip popover (the Control bar's and Properties'): the same body, under the popover's own
/// scope so it can be open beside the panel.
pub(crate) fn popover(app: &mut VectorcraftApp, ui: &mut Ui) {
    body(app, ui);
}

#[cfg(test)]
mod tests;
