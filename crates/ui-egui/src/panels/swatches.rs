//! Swatches panel: proxy, Recent Colors, find field, thumbnail grid (15.5 pt tiles on a 17 pt pitch)
//! or list, colour groups as folders, None/Registration first, bottom bar and panel menu.
//!
//! Swatches drag to reorder, into and out of colour groups (`swatch.move`) and onto art (the active
//! proxy's paint command with the object's id) and onto the Gradient panel's ramp; a Fill/Stroke
//! proxy or the Gradient panel's thumbnail dropped on the panel becomes a swatch (`swatch.new`).

use std::sync::Arc;

use egui::{Color32, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::libraries::{BuiltinLibrary, GRADIENT_LIBRARIES, SWATCH_LIBRARIES};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, GradientKind, Paint, Swatch, SwatchLibrary};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::swatchlib;

use super::library_panel::{self, LibraryKind, LibraryRef, Row};
use super::{active_paint, pstate, set_pstate};
use crate::menus::Item;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, SwatchRows, menu_item, swatch_tile};
use crate::{VectorcraftApp, icons};

/// View modes of the swatch list (and of the library panel).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    pub(crate) fn is_list(self) -> bool {
        matches!(self, View::SmallList | View::LargeList)
    }
    /// (tile, pitch) in points. Medium is the measured 15.5 / 17 pt.
    pub(crate) fn tile(self) -> (f32, f32) {
        match self {
            View::SmallThumb => (11.0, 12.5),
            View::MediumThumb => (15.5, 17.0),
            View::LargeThumb => (30.0, 32.0),
            View::SmallList => (12.0, 17.0),
            View::LargeList => (18.0, 25.0),
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

/// A row of the swatch list: special tiles, plain swatches and group folders.
#[derive(Clone, Debug)]
enum Entry {
    Registration,
    Swatch { name: String, paint: Paint, global: bool, spot: bool },
    Folder(String),
}

impl Entry {
    fn name(&self) -> &str {
        match self {
            Entry::Registration => REGISTRATION,
            Entry::Swatch { name, .. } | Entry::Folder(name) => name,
        }
    }
    fn is_folder(&self) -> bool {
        matches!(self, Entry::Folder(_))
    }
}

/// The panel's rows for the kind filter and the find field's `query` (a case-insensitive name
/// match; a colour group whose name matches shows all its swatches).
fn entries(app: &VectorcraftApp, kind: Kind, query: &str) -> Vec<Entry> {
    let Some(st) = app.session.active() else { return vec![] };
    let d = &st.doc;
    let q = query.trim().to_lowercase();
    let found = |n: &str| q.is_empty() || n.to_lowercase().contains(&q);
    let mut out = vec![];
    // A tint swatch shows its base's kind (global or spot).
    let sw = |s: &vectorcraft_color::Swatch| {
        let (global, spot) = s.tint_of().and_then(|(base, _)| d.swatch(base)).map_or((s.global, s.spot), |b| (b.global, b.spot));
        // A spot colour shows the colour its art shows (a Lab one's CMYK equivalent when the Spot
        // Colors options use those).
        let paint = match s.paint {
            Paint::Solid { color, swatch: None, .. } if spot => Paint::solid(d.linked_color(color, true)),
            _ => s.paint.clone(),
        };
        Entry::Swatch { name: s.name.clone(), paint, global, spot }
    };
    // None first, then Registration, then the rest (the reference app's order).
    let (specials, rest): (Vec<_>, Vec<_>) = d.swatches.iter().partition(|s| s.paint.is_none());
    for s in specials.iter().filter(|s| kind.accepts(&s.paint, false) && found(&s.name)) {
        out.push(sw(s));
    }
    if matches!(kind, Kind::All | Kind::Color) && found(REGISTRATION) {
        out.push(Entry::Registration);
    }
    for s in rest.iter().filter(|s| kind.accepts(&s.paint, false) && found(&s.name)) {
        out.push(sw(s));
    }
    for g in &d.swatch_groups {
        let all = found(&g.name);
        let items: Vec<_> = g.swatches.iter().filter(|s| kind.accepts(&s.paint, true) && (all || found(&s.name))).collect();
        // Empty groups show only among all swatches or groups, and when the find field names them.
        if items.is_empty() && !(all && matches!(kind, Kind::Groups | Kind::All)) {
            continue;
        }
        out.push(Entry::Folder(g.name.clone()));
        out.extend(items.into_iter().map(sw));
    }
    out
}

/// The paint command params that apply entry `e` (`paint.setFill`); none for a colour group.
fn swatch_params(e: &Entry) -> Option<Value> {
    Some(match e {
        Entry::Registration => json!({"swatch": REGISTRATION}),
        Entry::Swatch { paint, .. } if paint.is_none() => json!({"none": true}),
        Entry::Swatch { name, .. } => json!({"swatch": name}),
        Entry::Folder(_) => return None,
    })
}

/// Apply a clicked swatch to the active proxy (Alt: the inactive one).
fn apply(app: &mut VectorcraftApp, ui: &Ui, e: &Entry) {
    if let Some(params) = swatch_params(e) {
        super::apply_click(app, ui, params);
    }
}

/// A pattern swatch drawn as a rendered tile (cached by the definition's identity and size).
fn pattern_thumb(app: &VectorcraftApp, ui: &Ui, r: Rect, paint: &Paint) {
    use std::collections::HashMap;
    type Key = (String, Vec<usize>, String, u32);
    thread_local! {
        static CACHE: crate::graphics::TexCache<HashMap<Key, Option<egui::TextureHandle>>> = crate::graphics::TexCache::default();
    }
    let Paint::Pattern { pattern, .. } = paint else { return };
    let Some(st) = app.session.active() else { return };
    let Some(def) = st.doc.pattern(pattern) else { return };
    let px = (r.width() * ui.ctx().pixels_per_point()).round().max(4.0) as u32;
    let key = (
        pattern.clone(),
        def.art.iter().map(|a| std::sync::Arc::as_ptr(a) as usize).collect(),
        format!("{:?}{:?}{:?}", def.tile, def.tile_type, def.overlap),
        px,
    );
    let tex = CACHE.with(|c| c.borrow().get(&key).cloned()).unwrap_or_else(|| {
        let tex = vectorcraft_render::render_pattern_swatch(&st.doc, pattern, px).map(|img| {
            let color = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
            ui.ctx().load_texture(format!("pattern-swatch-{pattern}-{px}"), color, egui::TextureOptions::LINEAR)
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
    if let Some(tex) = tex {
        ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
        ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

fn draw_registration(ui: &Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    let c = r.center();
    let rad = r.width() * 0.28;
    let s = Stroke::new(1.0, egui::Color32::BLACK);
    ui.painter().circle_stroke(c, rad, s);
    ui.painter().line_segment([pos2(r.left() + 2.0, c.y), pos2(r.right() - 2.0, c.y)], s);
    ui.painter().line_segment([pos2(c.x, r.top() + 2.0), pos2(c.x, r.bottom() - 2.0)], s);
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
}

pub(crate) fn draw_folder(ui: &Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    icons::paint(ui, "dc-folder", r.expand(1.0), t.icon);
}

/// The ink colours of the CMYK glyph (cyan, magenta, yellow, black) and the RGB glyph's bars.
const CMYK_INKS: [Color32; 4] = [Color32::from_rgb(0, 174, 239), Color32::from_rgb(236, 0, 140), Color32::from_rgb(255, 242, 0), Color32::BLACK];
const RGB_BARS: [Color32; 3] = [Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 200, 0), Color32::from_rgb(0, 0, 255)];
/// Lab's opponent axes: green | red (a) over blue | yellow (b).
const LAB_AXES: [Color32; 4] =
    [Color32::from_rgb(0, 166, 81), Color32::from_rgb(230, 30, 60), Color32::from_rgb(0, 90, 220), Color32::from_rgb(250, 205, 0)];

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
        Paint::Gradient(g) => crate::i18n::fmt(tl!("{kind} Gradient"), &[("kind", tl!(g.gradient.kind.label()))]),
        Paint::Pattern { .. } => tl!("Pattern").into(),
        Paint::None => tl!("None").into(),
    }
}

/// The kind and colour-mode icons at the right of list row `r`: spot (a dot in a ring), global (a
/// square with a filled corner) or process colours (a square), gradients and patterns; then a
/// colour's model (CMYK as four ink quarters, RGB as three bars, Gray as a grey square, Lab as its
/// two opponent axes).
pub(crate) fn list_icons(ui: &Ui, r: Rect, paint: &Paint, global: bool, spot: bool) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let mode = Rect::from_center_size(r.right_center() - vec2(12.0, 0.0), vec2(10.0, 10.0));
    let kind = mode.translate(vec2(-16.0, 0.0));
    let line = Stroke::new(1.0, t.icon);
    match paint {
        Paint::Solid { color, .. } => {
            if spot {
                p.circle_stroke(kind.center(), 4.5, line);
                p.circle_filled(kind.center(), 1.8, t.icon);
            } else {
                p.rect_stroke(kind, 0.0, line, StrokeKind::Inside);
                if global {
                    let c = kind.right_bottom();
                    p.add(Shape::convex_polygon(vec![c, c - vec2(6.0, 0.0), c - vec2(0.0, 6.0)], t.icon, Stroke::NONE));
                }
            }
            let quarters = |colors: [Color32; 4]| {
                let q = mode.size() / 2.0;
                for (i, c) in colors.into_iter().enumerate() {
                    p.rect_filled(Rect::from_min_size(mode.min + vec2((i % 2) as f32 * q.x, (i / 2) as f32 * q.y), q), 0.0, c);
                }
            };
            match color {
                Color::Cmyk { .. } => quarters(CMYK_INKS),
                Color::Lab { .. } => quarters(LAB_AXES),
                Color::Rgb { .. } => {
                    let w = mode.width() / 3.0;
                    for (i, bar) in RGB_BARS.into_iter().enumerate() {
                        p.rect_filled(Rect::from_min_size(mode.min + vec2(i as f32 * w, 0.0), vec2(w, mode.height())), 0.0, bar);
                    }
                }
                Color::Gray { .. } => {
                    p.rect_filled(mode, 0.0, Color32::from_gray(128));
                }
            }
            p.rect_stroke(mode, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
        }
        Paint::Gradient(g) => {
            let icon = match g.gradient.kind {
                GradientKind::Linear => "dc-grad-linear",
                GradientKind::Radial => "dc-grad-radial",
                GradientKind::Freeform => "dc-grad-freeform",
            };
            icons::paint(ui, icon, kind.expand(1.0), t.icon);
        }
        Paint::Pattern { .. } => icons::paint(ui, "grid-3x3", kind.expand(1.0), t.icon),
        Paint::None => {}
    }
}

/// The swatches and colour groups selected in the panel, in click order, without names that no
/// longer exist (deleted, renamed or undone).
fn selection(app: &VectorcraftApp, ui: &Ui) -> Vec<String> {
    let Some(st) = app.session.active() else { return vec![] };
    let mut names: Vec<String> = pstate(ui.ctx(), "swatch-selected");
    names.retain(|n| n == REGISTRATION || st.doc.swatch_name_taken(n));
    names
}

/// The names the panel highlights: `sel` plus the swatches of selected colour groups (clicking a
/// folder selects the whole group).
fn highlighted<'a>(d: &'a Document, sel: &'a [String]) -> Vec<&'a str> {
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
enum Drop {
    /// Swatches (or colour groups) dropped before or `after` row `target`, into folder `target`, or
    /// at the end (`None`).
    Move { names: Vec<String>, target: Option<String>, after: bool },
    /// A paint from elsewhere (a Fill/Stroke proxy, the Gradient panel's thumbnail) dropped on row
    /// `target` (or between rows) becomes a swatch; with Alt held (`replace`) it replaces swatch
    /// `target` when that is of its kind.
    New { paint: Paint, target: Option<String>, replace: bool },
    /// Art dragged off the canvas becomes a pattern swatch made of a copy of it (the art stays).
    Pattern { ids: Vec<u64> },
}

impl Drop {
    /// What releasing `d` before or `after` row `target` (`None`: at the end) does: move the rows
    /// it carries, make a swatch of a paint dragged from elsewhere, or a pattern swatch of art dragged
    /// off the canvas (appearances: nothing).
    fn of(d: &PanelDrag, target: Option<String>, after: bool, replace: bool) -> Option<Self> {
        match d {
            PanelDrag::Paint { rows: Some(r), .. } => Some(Drop::Move { names: r.names.clone(), target, after }),
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
fn drag_of(e: &Entry, names: Vec<String>) -> PanelDrag {
    let paint = match e {
        Entry::Swatch { paint, .. } => paint.clone(),
        Entry::Registration => Paint::registration(),
        Entry::Folder(_) => Paint::None,
    };
    let rows = SwatchRows { grabbed: e.name().to_string(), names, groups: e.is_folder() };
    PanelDrag::Paint { paint, params: swatch_params(e).unwrap_or_default(), rows: Some(rows) }
}

/// What the tiles or rows saw this frame.
#[derive(Default)]
struct TileEvents {
    clicked: Option<(Entry, egui::Modifiers)>,
    /// Double-click opens the swatch's editor (Swatch Options, Gradient panel or pattern editing),
    /// or Edit or Apply Color Group on a colour group.
    edit: Option<Entry>,
    drop: Option<Drop>,
    /// A drag is held over a tile or row.
    over: bool,
}

/// Clicks, double-clicks, tooltips and drag and drop of the tile or row `resp` of entry `e`
/// (`items`: the panel's rows, `sel`: its selection, `list`: rows stack downwards). A drag from it
/// starts a [`PanelDrag`] carrying the rows it moves; see [`tile_drop`] for drags held over it.
fn tile_input(ui: &Ui, resp: Response, e: &Entry, items: &[Entry], sel: &[String], list: bool, ev: &mut TileEvents) {
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
    let resp = match e {
        Entry::Folder(n) => resp.on_hover_text(crate::i18n::fmt(tl!("Color Group: {name}"), &[("name", n)])),
        Entry::Swatch { paint, global, spot, .. } if list => resp.on_hover_ui(|ui| {
            ui.label(format!("{name} ({})", describe(paint, *global, *spot)));
        }),
        _ => resp.on_hover_text(name),
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
        ev.drop = Drop::of(&d, Some(name.to_string()), after, super::alt_held(ui));
    }
}

/// Drops between and after the tiles (`zone`: the list's viewport) go to the end of the ungrouped
/// swatches (colour groups to the end of the groups); the list is outlined while one is held there.
fn zone_input(ui: &Ui, zone: &Response, ev: &mut TileEvents) {
    let Some(d) = held(zone) else { return };
    if !ev.over {
        ui.painter().rect_stroke(zone.rect, 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Inside);
    }
    if zone.dnd_release_payload::<PanelDrag>().is_some() {
        ev.drop = Drop::of(&d, None, false, false);
    }
}

/// The colour group `name` is, or the one swatch `name` belongs to.
fn group_of(d: &Document, name: &str) -> Option<String> {
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
                None => (None, &d.swatches),
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

/// Run a drop on the panel: move the swatches or make a swatch of the proxy's paint (a colour
/// dropped on a folder or a grouped swatch joins that group).
fn apply_drop(app: &mut VectorcraftApp, drop: Drop) {
    let Some(d) = app.session.active().map(|st| &st.doc) else { return };
    let (cmd, params) = match drop {
        Drop::Move { names, target, after } => match move_params(d, &names, target.as_deref(), after) {
            Some(p) => ("swatch.move", p),
            None => return,
        },
        // Alt: an edited gradient (or colour) replaces the swatch it is dropped on.
        Drop::New { paint, target: Some(t), replace: true } if d.swatch(&t).is_some_and(|w| same_kind(&w.paint, &paint)) => {
            ("swatch.edit", json!({"name": t, "paint": super::paint_params(&paint)}))
        }
        Drop::New { paint, target, .. } => {
            let mut p = super::paint_params(&paint);
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
        PanelDrag::GraphicStyle(name) => return pointer_chip(ctx, |ui, r| super::graphic_styles::paint_style(app, ui, r, name)),
        PanelDrag::Symbol(name) => match app.session.active() {
            Some(st) => return pointer_chip(ctx, |ui, r| super::symbols::chip(ui, &st.doc, r, name)),
            None => return,
        },
        PanelDrag::Brush { def, .. } => return pointer_chip(ctx, |ui, r| super::brushes::chip(ui, r, def)),
        PanelDrag::LibraryGraphic { library, item } => return pointer_chip(ctx, |ui, r| super::libraries::chip(app, ui, r, library, item)),
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
    let area = egui::Area::new(egui::Id::new("swatch-drag-preview")).order(egui::Order::Tooltip).fixed_pos(at + vec2(12.0, 12.0));
    area.interactable(false).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        draw(ui, r);
    });
}

/// The id of the find field's text (shown with the panel menu's Show Find Field).
fn find_id() -> egui::Id {
    egui::Id::new("swatch-find")
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    body(app, ui, TILE);
}

/// The panel in a Fill/Stroke chip popover (the Control bar's and Properties'): the same body, its
/// tiles under their own ids so it can be open beside the panel.
pub(crate) fn popover(app: &mut VectorcraftApp, ui: &mut Ui) {
    body(app, ui, POPOVER_TILE);
}

/// Id salts of the panel's tiles and of a popover's.
const TILE: &str = "swatch-tile";
const POPOVER_TILE: &str = "swatch-pop-tile";

/// The panel's body; `salt` keys its tiles (see [`tile_id`]).
fn body(app: &mut VectorcraftApp, ui: &mut Ui, salt: &'static str) {
    if app.session.active().is_none() {
        super::empty_state(ui, "swatch-book", tl!("No document"), tl!("Open a document to see its swatches."));
        return;
    }
    let view: View = pstate(ui.ctx(), "swatch-view");
    let kind: Kind = pstate(ui.ctx(), "swatch-kind");
    // Top row: proxy at left, list / grid view toggles at right.
    ui.horizontal(|ui| {
        super::proxy(app, ui, 34.0);
        ui.add_space(ui.available_width() - 58.0);
        if widgets::icon_button(ui, "dc-list-view", tl!("Show List View"), view.is_list(), 26.0).clicked() {
            set_pstate(ui.ctx(), "swatch-view", View::SmallList);
        }
        if widgets::icon_button(ui, "dc-grid-view", tl!("Show Thumbnail View"), !view.is_list(), 26.0).clicked() {
            set_pstate(ui.ctx(), "swatch-view", View::MediumThumb);
        }
    });
    ui.add_space(4.0);
    // A recent colour paints the active proxy (Alt: the inactive one), as a swatch does.
    if let Some(c) = super::recent_colors_row(app, ui) {
        super::apply_click(app, ui, json!({"color": super::color_json(&c)}));
    }
    widgets::divider(ui);
    widgets::subheader(ui, tl!("Swatch Tiles"));
    let query = if pstate(ui.ctx(), "swatch-show-find") { widgets::search_field(ui, find_id(), tl!("Find")) } else { String::new() };
    let items = entries(app, kind, &query);
    let active_swatch = active_swatch(app);
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
                    match e {
                        Entry::Registration => draw_registration(ui, chip),
                        Entry::Swatch { paint, global, spot, .. } => {
                            swatch_tile(ui, chip, paint, false, false);
                            pattern_thumb(app, ui, chip, paint);
                            list_icons(ui, r, paint, *global, *spot);
                        }
                        Entry::Folder(_) => draw_folder(ui, chip),
                    }
                    list_row_name(ui, r, chip, name);
                    tile_input(ui, resp, e, &items, sel, true, &mut ev);
                }
            } else {
                for (e, cell) in tile_grid(ui, &items, view, Entry::is_folder) {
                    let resp = ui.interact(cell, tile_id(salt, e), Sense::click_and_drag());
                    let name = e.name();
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
        let zone = ui.interact(out.inner_rect, ui.id().with("swatch-drop"), Sense::hover());
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

/// The swatch the active paint comes from: a linked colour's global swatch (a tint's tint swatch
/// when there is one), a gradient's gradient swatch, or None.
fn active_swatch(app: &VectorcraftApp) -> Option<String> {
    match active_paint(app) {
        Paint::Solid { swatch: Some(n), tint, .. } => {
            let d = &app.session.active()?.doc;
            let tint_swatch = d.swatches_iter().find(|w| w.tint_of() == Some((n.as_str(), tint)));
            Some(tint_swatch.map_or(n.clone(), |w| w.name.clone()))
        }
        Paint::Gradient(g) => g.swatch,
        Paint::None => Some("[None]".to_string()),
        _ => None,
    }
}

/// The thumbnail cells of `items` for `view` in the full available width, row by row; a folder
/// (`is_folder`) starts a new row.
pub(crate) fn tile_grid<'a, T>(ui: &mut Ui, items: &'a [T], view: View, is_folder: impl Fn(&T) -> bool) -> Vec<(&'a T, Rect)> {
    let (tile, pitch) = view.tile();
    let w = ui.available_width();
    let per_row = ((w - 2.0) / pitch).floor().max(1.0) as usize;
    let mut rows: Vec<Vec<&T>> = vec![vec![]];
    for e in items {
        match rows.last_mut() {
            Some(row) if row.len() < per_row && (row.is_empty() || !is_folder(e)) => row.push(e),
            _ => rows.push(vec![e]),
        }
    }
    let mut out = Vec::with_capacity(items.len());
    for row in rows {
        let (r, _) = ui.allocate_exact_size(vec2(w, pitch), Sense::hover());
        let cell = |c: usize| Rect::from_min_size(r.min + vec2(1.0 + c as f32 * pitch, (pitch - tile) / 2.0), vec2(tile, tile));
        out.extend(row.into_iter().enumerate().map(|(c, e)| (e, cell(c))));
    }
    out
}

/// A list-view row for `view` taking clicks and drags as `id`, shaded when `selected` or hovered
/// → (the row, its response, the chip at its left).
pub(crate) fn list_row(ui: &mut Ui, id: egui::Id, view: View, selected: bool) -> (Rect, Response, Rect) {
    let t = Tokens::get(ui.ctx());
    let (tile, pitch) = view.tile();
    let (_, r) = ui.allocate_space(vec2(ui.available_width(), pitch));
    let resp = ui.interact(r, id, Sense::click_and_drag());
    if selected {
        ui.painter().rect_filled(r, 0.0, t.row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover);
    }
    (r, resp, Rect::from_min_size(r.left_center() + vec2(4.0, -tile / 2.0), vec2(tile, tile)))
}

/// A list row's name, right of its `chip`.
pub(crate) fn list_row_name(ui: &Ui, r: Rect, chip: Rect, name: &str) {
    let t = Tokens::get(ui.ctx());
    ui.painter().text(pos2(chip.right() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), t.text);
}

/// The white corner of a global colour's tile; a spot colour has a dot in it.
pub(crate) fn global_mark(ui: &Ui, cell: Rect, spot: bool) {
    let k = cell.shrink(1.0).right_bottom();
    let corner = vec![k, k - vec2(5.0, 0.0), k - vec2(0.0, 5.0)];
    ui.painter().add(Shape::convex_polygon(corner, Color32::WHITE, Stroke::NONE));
    if spot {
        ui.painter().circle_filled(k - vec2(1.6, 1.6), 1.0, Color32::BLACK);
    }
}

/// A colour group's folder in the thumbnail view, outlined when `selected`.
pub(crate) fn folder_tile(ui: &Ui, cell: Rect, selected: bool) {
    draw_folder(ui, cell);
    if selected {
        ui.painter().rect_stroke(cell.expand(1.0), 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Outside);
    }
}

/// The interaction id of a tile or list row (stable per swatch or group name and host `salt`).
fn tile_id(salt: &'static str, e: &Entry) -> egui::Id {
    egui::Id::new((salt, e.name()))
}

/// The selected swatches and groups that can be deleted (all but None and Registration).
fn deletable(sel: &[String]) -> Vec<String> {
    sel.iter().filter(|n| !n.starts_with('[')).cloned().collect()
}

/// The selected swatch when exactly one swatch with an editor is selected (not None, Registration
/// or a group).
fn editable(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    match sel {
        [n] if !n.starts_with('[') && app.session.active().is_some_and(|st| st.doc.swatch(n).is_some()) => Some(n.clone()),
        _ => None,
    }
}

/// The selected solid-colour swatches in selection order (Merge Swatches keeps the first).
fn mergeable(app: &VectorcraftApp, sel: &[String]) -> Vec<String> {
    let Some(st) = app.session.active() else { return vec![] };
    sel.iter().filter(|n| st.doc.swatch(n).is_some_and(|w| w.paint.color().is_some())).cloned().collect()
}

/// The first selected colour group (Ungroup Color Group).
fn selected_group(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    let d = &app.session.active()?.doc;
    sel.iter().find(|n| d.swatch_groups.iter().any(|g| g.name == **n)).cloned()
}

/// Edit or Apply Color Group: Recolor Artwork on colour group `group` (and the selected art).
fn edit_group(app: &mut VectorcraftApp, group: &str) {
    app.run("ui.recolorDialog", json!({ "group": group })).ok();
}

/// Delete `names` (swatches and colour groups) after asking, or at once with `now` (Alt-click).
fn delete(app: &mut VectorcraftApp, names: Vec<String>, now: bool) {
    let params = json!({"names": names});
    if now {
        app.run("swatch.delete", params).ok();
        return;
    }
    let message = match names.as_slice() {
        [n] => crate::i18n::fmt(tl!("Delete “{name}”?"), &[("name", n)]),
        _ => crate::i18n::fmt(tl!("Delete these {count} swatches and groups?"), &[("count", &names.len().to_string())]),
    };
    crate::dialogs::confirm::ask(app, &message, tl!("Art using a deleted global swatch keeps its colour."), "swatch.delete", params);
}

fn bottom(app: &mut VectorcraftApp, ui: &mut Ui, sel: &[String]) {
    let kind: Kind = pstate(ui.ctx(), "swatch-kind");
    widgets::bottom_bar(ui, |ui| {
        let open = app.ui.library_panel.as_ref().is_some_and(|o| o.kind == SwatchLibraries::KIND);
        let lr = widgets::icon_button(ui, "library", SwatchLibraries::MENU, open, 24.0);
        egui::Popup::menu(&lr).show(|ui| {
            ui.set_min_width(200.0);
            library_panel::library_menu::<SwatchLibraries>(app, ui);
        });
        let kr = widgets::icon_button(ui, "dc-swatch-kinds", tl!("Show Swatch Kinds"), kind != Kind::All, 24.0);
        egui::Popup::menu(&kr).show(|ui| {
            for (k, label) in Kind::ALL {
                if menu_item(ui, label, true, k == kind) {
                    set_pstate(ui.ctx(), "swatch-kind", k);
                }
            }
        });
        // With a colour group selected the options button edits or applies the group.
        let opts = editable(app, sel);
        if let Some(g) = selected_group(app, sel).filter(|_| opts.is_none()) {
            if widgets::icon_button(ui, "palette", tl!("Edit or Apply Color Group"), false, 24.0).clicked() {
                edit_group(app, &g);
            }
        } else if widgets::icon_button_enabled(ui, "dc-options", tl!("Swatch Options"), false, opts.is_some(), 24.0).clicked()
            && let Some(n) = opts
        {
            app.run("ui.swatchOptions", json!({"name": n})).ok();
        }
        ui.add_space((ui.available_width() - 3.0 * 28.0).max(0.0));
        if widgets::icon_button(ui, "dc-folder", tl!("New Color Group"), false, 24.0).clicked() {
            app.run("ui.newColorGroup", json!({"swatches": deletable(sel)})).ok();
        }
        // Alt-click skips the dialog; Ctrl/Cmd-click makes a spot colour.
        if widgets::icon_button(ui, "dc-new-item", tl!("New Swatch"), false, 24.0).clicked() {
            let m = ui.input(|i| i.modifiers);
            new_swatch(app, sel, m.command, m.alt);
        }
        // Alt-click deletes without asking.
        let del = deletable(sel);
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Swatch"), false, !del.is_empty(), 24.0).clicked() {
            delete(app, del, ui.input(|i| i.modifiers.alt));
        }
    });
}

/// The colour group the selection points at: the last selected group, or the group of the last
/// selected swatch.
fn target_group(app: &VectorcraftApp, sel: &[String]) -> Option<String> {
    group_of(&app.session.active()?.doc, sel.last()?)
}

/// New Swatch from the active paint (a colour goes into the selected colour group): opens the
/// dialog, or saves at once with `now` (Alt-click). `spot` (Cmd/Ctrl-click) makes a spot colour.
fn new_swatch(app: &mut VectorcraftApp, sel: &[String], spot: bool, now: bool) {
    let paint = active_paint(app);
    let group = paint.color().and_then(|_| target_group(app, sel));
    if !now {
        app.run("ui.newSwatch", json!({"spot": spot, "group": group})).ok();
        return;
    }
    let mut params = super::paint_params(&paint);
    params["spot"] = json!(spot);
    if let Some(g) = group {
        params["group"] = json!(g);
    }
    app.run("swatch.new", params).ok();
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let view: View = pstate(ui.ctx(), "swatch-view");
    let selected = selection(app, ui);
    let del = deletable(&selected);
    let opts = editable(app, &selected);
    if menu_item(ui, tl!("New Swatch…"), true, false) {
        new_swatch(app, &selected, false, false);
    }
    if menu_item(ui, tl!("New Color Group…"), true, false) {
        app.run("ui.newColorGroup", json!({"swatches": del})).ok();
    }
    if menu_item(ui, tl!("Duplicate Swatch"), opts.is_some(), false)
        && let Some(n) = &opts
    {
        app.run("swatch.duplicate", json!({"name": n})).ok();
    }
    // The first selected colour is kept.
    let merge = mergeable(app, &selected);
    if menu_item(ui, tl!("Merge Swatches"), merge.len() > 1, false) {
        app.run("swatch.merge", json!({ "names": merge })).ok();
    }
    if menu_item(ui, tl!("Delete Swatch"), !del.is_empty(), false) {
        delete(app, del, false);
    }
    let group = selected_group(app, &selected);
    if menu_item(ui, tl!("Edit or Apply Color Group…"), group.is_some(), false)
        && let Some(g) = &group
    {
        edit_group(app, g);
    }
    if menu_item(ui, tl!("Ungroup Color Group"), group.is_some(), false)
        && let Some(g) = group
    {
        app.run("swatch.ungroup", json!({ "name": g })).ok();
    }
    if menu_item(ui, tl!("Select All Unused"), true, false)
        && let Ok(r) = app.run("swatch.unused", json!({}))
    {
        let names: Vec<String> = r["names"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        set_pstate(ui.ctx(), "swatch-selected", names);
    }
    if menu_item(ui, tl!("Add Used Colors"), true, false) {
        app.run("swatch.addUsedColors", json!({})).ok();
    }
    ui.separator();
    if menu_item(ui, tl!("Sort by Name"), true, false) {
        app.run("swatch.sortByName", json!({})).ok();
    }
    if menu_item(ui, tl!("Sort by Kind"), true, false) {
        app.run("swatch.sortByKind", json!({})).ok();
    }
    ui.separator();
    for (v, label) in View::ALL {
        if menu_item(ui, label, true, v == view) {
            set_pstate(ui.ctx(), "swatch-view", v);
        }
    }
    ui.separator();
    let find: bool = pstate(ui.ctx(), "swatch-show-find");
    if menu_item(ui, tl!("Show Find Field"), true, find) {
        set_pstate(ui.ctx(), "swatch-show-find", !find);
    }
    ui.separator();
    if menu_item(ui, tl!("Swatch Options…"), opts.is_some(), false)
        && let Some(n) = opts
    {
        app.run("ui.swatchOptions", json!({"name": n})).ok();
    }
    if menu_item(ui, tl!("Spot Colors…"), app.session.active().is_some(), false) {
        app.run("ui.spotColors", json!({})).ok();
    }
    ui.separator();
    ui.menu_button(tl!("Open Swatch Library"), |ui| library_panel::library_menu::<SwatchLibraries>(app, ui));
    if menu_item(ui, tl!("Save Swatch Library…"), true, false) {
        app.run("ui.saveSwatchLibrary", json!({ "names": deletable(&selected) })).ok();
    }
}

fn library_refs(libs: Vec<swatchlib::LibraryInfo>) -> Vec<LibraryRef> {
    libs.into_iter().map(|l| LibraryRef { submenu: library_panel::submenu(l.category), id: l.id, name: l.name }).collect()
}

/// The swatch libraries with solid colours (not the gradient-only ones), for Limit to Library.
pub(crate) fn colour_libraries(app: &VectorcraftApp) -> Vec<LibraryRef> {
    let has_colours = |id: &str| swatchlib::library(&app.session, id).is_some_and(|(_, lib)| lib.iter().any(|w| w.paint.color().is_some()));
    library_refs(swatchlib::libraries(&app.session).into_iter().filter(|l| l.category != "gradients" && has_colours(&l.id)).collect())
}

/// What Limit to Library menus call the document's own swatches ([`swatchlib::DOCUMENT_SWATCHES`]).
pub(crate) const DOCUMENT_SWATCHES: &str = "Document Swatches";

/// The name of Limit to Library choice `key` (a library id or [`swatchlib::DOCUMENT_SWATCHES`]);
/// `None` for no limit or a library that is gone.
pub(crate) fn limit_name(app: &VectorcraftApp, key: &str) -> Option<String> {
    match key {
        "" => None,
        swatchlib::DOCUMENT_SWATCHES => Some(DOCUMENT_SWATCHES.into()),
        _ => swatchlib::library(&app.session, key).map(|(info, _)| info.name),
    }
}

/// The Limit to Library key for `l`: a library's id (from its id or name), or
/// [`swatchlib::DOCUMENT_SWATCHES`] as it is.
pub(crate) fn limit_key(app: &VectorcraftApp, l: &str) -> Result<String, String> {
    if l == swatchlib::DOCUMENT_SWATCHES {
        return Ok(l.into());
    }
    swatchlib::library(&app.session, l).map(|(info, _)| info.id).ok_or_else(|| format!("no swatch library `{l}` (see swatch.library.list)"))
}

/// Swatch libraries in the library panel (engine: `swatch.library.*`).
pub(crate) struct SwatchLibraries;

impl LibraryKind for SwatchLibraries {
    const KIND: &'static str = "swatches";
    const OPEN: &'static str = "window.swatchLibrary";
    const MENU: &'static str = "Swatch Libraries Menu";
    const ADD: &'static str = "Add to Swatches";
    type Lib = Arc<SwatchLibrary>;
    type Item = Swatch;

    fn list(app: &VectorcraftApp) -> Vec<LibraryRef> {
        library_refs(swatchlib::libraries(&app.session))
    }
    fn get(app: &VectorcraftApp, id: &str) -> Option<(String, Self::Lib)> {
        swatchlib::library(&app.session, id).map(|(info, lib)| (info.name, lib))
    }
    /// Ungrouped swatches, then each colour group's folder and swatches (a group whose name
    /// matches shows all of them), as in the Swatches panel.
    fn rows<'a>(lib: &'a Self::Lib, query: &str) -> Vec<Row<'a, Swatch>> {
        let found = |n: &str| query.is_empty() || n.to_lowercase().contains(query);
        let row = |w: &'a Swatch| Row { name: &w.name, item: Some(w) };
        let mut out: Vec<Row<Swatch>> = lib.swatches.iter().filter(|w| found(&w.name)).map(row).collect();
        for g in &lib.groups {
            let all = found(&g.name);
            let mut items = g.swatches.iter().filter(|w| all || found(&w.name)).map(row).peekable();
            if items.peek().is_some() {
                out.push(Row { name: &g.name, item: None });
                out.extend(items);
            }
        }
        out
    }
    fn draw(_: &VectorcraftApp, ui: &Ui, r: Rect, w: &Swatch, selected: bool, hovered: bool) {
        swatch_tile(ui, r, &w.paint, selected, hovered);
        if w.global {
            global_mark(ui, r, w.spot);
        }
    }
    fn row_icons(ui: &Ui, r: Rect, w: &Swatch) {
        list_icons(ui, r, &w.paint, w.global, w.spot);
    }
    fn describe(w: &Swatch) -> String {
        describe(&w.paint, w.global, w.spot)
    }
    /// Adds the swatch and applies it to the active proxy (Alt: the inactive one), as one step.
    fn click(app: &mut VectorcraftApp, ui: &Ui, id: &str, name: &str) {
        let alt = super::alt_held(ui);
        let apply = if super::proxy_cmd(app, alt) == "paint.setFill" { "fill" } else { "stroke" };
        add_from_library(app, json!({"library": id, "names": [name], "apply": apply, "focus": !alt}));
    }
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>) {
        add_from_library(app, json!({"library": id, "names": names}));
    }
    fn menu_head(app: &mut VectorcraftApp, ui: &mut Ui) {
        if menu_item(ui, tl!("Default Swatches"), app.session.active().is_some(), false) {
            app.run("swatch.resetDefaults", json!({})).ok();
        }
        ui.separator();
    }
    fn menu_tail(app: &mut VectorcraftApp, ui: &mut Ui) {
        ui.separator();
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        if menu_item(ui, tl!("Other Library…"), true, false)
            && let Err(e) = app.run("window.swatchLibrary.other", json!({}))
        {
            app.status(e);
        }
        if menu_item(ui, tl!("Save Swatch Library…"), app.session.active().is_some(), false) {
            let names = deletable(&selection(app, ui));
            app.run("ui.saveSwatchLibrary", json!({ "names": names })).ok();
        }
    }
}

fn add_from_library(app: &mut VectorcraftApp, params: Value) {
    if let Err(e) = app.run("swatch.library.add", params) {
        app.status(e);
    }
}

/// `window.swatchLibrary {library}`: open a swatch library in the library panel (`library` null
/// closes it).
pub(crate) fn open_library(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    library_panel::open_command::<SwatchLibraries>(app, p, "swatch.library.list", |app, key| {
        swatchlib::library(&app.session, key).map(|(info, lib)| (info.id, info.name, lib.len()))
    })
}

/// `swatch.library.load` params, then open the library in the panel.
pub(crate) fn load_library(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let r = app.run("swatch.library.load", params)?;
    open_library(app, &json!({ "library": r["library"] }))
}

/// Other Library…: load the library (or document) at `path`, else one picked in an open dialog
/// (on the web the picked file arrives later and opens through [`crate::io::open_bytes`]).
pub(crate) fn other_library(app: &mut VectorcraftApp, path: Option<String>) -> Result<Value, String> {
    match library_panel::pick_library_file(app, path)? {
        Some(path) => load_library(app, json!({ "path": path })),
        None => Ok(Value::Null),
    }
}

/// The id prefix of the Window → Swatch Libraries → User Defined slots.
pub(crate) const USER_SLOT: &str = "window.userSwatchLibrary";

/// The User Defined library slot `id` (`window.userSwatchLibrary3`) stands for.
pub(crate) fn user_library(app: &VectorcraftApp, id: &str) -> Option<swatchlib::LibraryInfo> {
    library_panel::user_slot(id, USER_SLOT, swatchlib::libraries(&app.session))
}

/// Window → Swatch Libraries.
pub(crate) fn window_menu() -> Vec<Item> {
    const SLOTS: [&str; 10] = [
        "window.userSwatchLibrary1",
        "window.userSwatchLibrary2",
        "window.userSwatchLibrary3",
        "window.userSwatchLibrary4",
        "window.userSwatchLibrary5",
        "window.userSwatchLibrary6",
        "window.userSwatchLibrary7",
        "window.userSwatchLibrary8",
        "window.userSwatchLibrary9",
        "window.userSwatchLibrary10",
    ];
    let open = |b: &'static BuiltinLibrary| Item::Cmd(b.name, "window.swatchLibrary", json!({ "library": b.id }));
    let mut items = vec![Item::Cmd("Default Swatches", "swatch.resetDefaults", Value::Null), Item::Sep];
    items.extend(SWATCH_LIBRARIES.iter().map(open));
    items.extend([
        Item::Sub("Gradients", GRADIENT_LIBRARIES.iter().map(open).collect()),
        Item::Sep,
        Item::Sub("User Defined", SLOTS.iter().map(|id| Item::Cmd("User Library", id, Value::Null)).collect()),
        Item::Sep,
        Item::Cmd("Other Library…", "window.swatchLibrary.other", Value::Null),
        Item::Cmd("Save Swatch Library…", "ui.saveSwatchLibrary", Value::Null),
    ]);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::swatch_options::KIND;
    use egui::{Event, Modifiers, PointerButton, Pos2};
    use vectorcraft_color::Color;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app
    }

    /// One frame of `draw` on a persistent context (clicks need the previous frame's layout).
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, time: f64, draw: fn(&mut VectorcraftApp, &mut Ui)) {
        frame_with(app, ctx, events, Modifiers::NONE, time, draw);
    }

    /// [`frame`] with `modifiers` held.
    fn frame_with(
        app: &mut VectorcraftApp,
        ctx: &egui::Context,
        events: Vec<Event>,
        modifiers: Modifiers,
        time: f64,
        draw: fn(&mut VectorcraftApp, &mut Ui),
    ) {
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0)));
        let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| draw(app, ui));
        out.textures_delta.clear();
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    }

    fn double_click(at: Pos2) -> Vec<Event> {
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        vec![Event::PointerMoved(at), button(true), button(false), button(true), button(false)]
    }

    /// Double-click the thumbnail of swatch `name` in the panel.
    fn double_click_tile(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
        frame(app, ctx, vec![], 0.0, show);
        let tile = ctx.read_response(egui::Id::new(("swatch-tile", name))).unwrap_or_else(|| panic!("no tile for {name}")).rect;
        frame(app, ctx, double_click(tile.center()), 1.0, show);
    }

    /// Click the thumbnail of swatch or group `name` with `modifiers` held (frames at `time`, `time + 0.5`).
    fn click_tile(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, modifiers: Modifiers, time: f64) {
        frame(app, ctx, vec![], time, show);
        let at = ctx.read_response(egui::Id::new(("swatch-tile", name))).unwrap_or_else(|| panic!("no tile for {name}")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
        frame_with(app, ctx, vec![Event::PointerMoved(at), button(true), button(false)], modifiers, time + 0.5, show);
    }

    fn selected(ctx: &egui::Context) -> Vec<String> {
        pstate(ctx, "swatch-selected")
    }

    fn dialog_frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
        frame(app, ctx, vec![], 2.0, |app, ui| crate::dialogs::show(app, ui.ctx()));
    }

    #[test]
    fn double_click_opens_swatch_options_and_ok_edits_linked_art() {
        let mut app = app();
        app.run("swatch.edit", json!({"name": "Red", "global": true})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "swatch": "Red"})).unwrap();
        let ctx = context();
        double_click_tile(&mut app, &ctx, "Red");
        let d = app.ui.dialog.as_ref().expect("double-click opens Swatch Options");
        assert_eq!((d.kind.as_str(), d.str("__swatch").as_str(), d.str("mode").as_str()), (KIND, "Red", "rgb"));
        // The dialog previews live on the canvas; OK keeps the edit as one undo step.
        let undo = app.session.doc().unwrap().history.undo.len();
        dialog_frame(&mut app, &ctx);
        assert!(app.session.in_interaction(), "Preview runs as an interaction");
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("color".into(), json!("#00ff00"));
        d.fields.insert("name".into(), json!("Signal"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(doc.swatch("Signal").and_then(|w| w.paint.color()).map(|c| c.to_hex()), Some("#00ff00".into()));
        let fill = doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
        assert_eq!(fill, Paint::Solid { color: Color::from_hex("#00ff00").unwrap(), swatch: Some("Signal".into()), tint: 1.0 });
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    }

    #[test]
    fn cancel_rolls_the_preview_back_and_other_kinds_open_their_editors() {
        let mut app = app();
        let ctx = context();
        let before = app.session.doc().unwrap().doc.clone();
        app.run("ui.swatchOptions", json!({"name": "Bright Blue"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!("#000000"));
        dialog_frame(&mut app, &ctx);
        assert_ne!(app.session.doc().unwrap().doc, before, "previewed");
        crate::dialogs::cancel(&mut app);
        assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
        assert_eq!(app.session.doc().unwrap().doc, before);
        // A gradient opens Swatch Options with its name (and the gradient shown): OK renames it.
        double_click_tile(&mut app, &ctx, "Sunset");
        let d = app.ui.dialog.as_mut().expect("Swatch Options for a gradient");
        assert!(d.kind == KIND && d.fields.contains_key("__gradient") && !d.fields.contains_key("color"));
        d.fields.insert("name".into(), json!("Evening"));
        dialog_frame(&mut app, &ctx);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(matches!(app.session.doc().unwrap().doc.swatch("Evening").map(|w| &w.paint), Some(Paint::Gradient(_))));
        // Patterns open pattern editing, None nothing.
        assert!(app.run("ui.swatchOptions", json!({"name": "[None]"})).is_err());
        assert!(app.run("ui.swatchOptions", json!({})).is_err());
    }

    #[test]
    fn modifier_clicks_select_several_swatches_and_groups() {
        let mut app = app();
        let ctx = context();
        click_tile(&mut app, &ctx, "Red", Modifiers::NONE, 0.0);
        assert_eq!(selected(&ctx), ["Red"]);
        assert_eq!(app.session.paint.fill.color().map(|c| c.to_hex()), Some("#ed1c24".into()), "a plain click applies");
        click_tile(&mut app, &ctx, "Amber", Modifiers::SHIFT, 2.0);
        assert_eq!(selected(&ctx), ["Red", "Orange Red", "Orange", "Amber"], "Shift extends over the shown order");
        click_tile(&mut app, &ctx, "Orange", Modifiers::COMMAND, 4.0);
        click_tile(&mut app, &ctx, "Brights", Modifiers::COMMAND, 6.0);
        assert_eq!(selected(&ctx), ["Red", "Orange Red", "Amber", "Brights"], "Cmd toggles swatches and colour groups");
        assert_eq!(app.session.paint.fill.color().map(|c| c.to_hex()), Some("#ed1c24".into()), "modifier clicks only select");
        click_tile(&mut app, &ctx, "Grays", Modifiers::NONE, 8.0);
        assert_eq!(selected(&ctx), ["Grays"], "a plain click on a group selects it alone");
    }

    #[test]
    fn delete_asks_then_removes_the_selection_and_unlinks_art() {
        let mut app = app();
        let ctx = context();
        app.run("swatch.edit", json!({"name": "Red", "global": true})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "swatch": "Red"})).unwrap();
        delete(&mut app, vec!["Red".into(), "Brights".into()], false);
        let d = app.ui.dialog.as_ref().expect("delete asks first");
        assert_eq!((d.kind.as_str(), d.str("message").as_str()), (crate::dialogs::confirm::KIND, "Delete these 2 swatches and groups?"));
        dialog_frame(&mut app, &ctx);
        assert!(app.session.doc().unwrap().doc.swatch("Red").is_some(), "nothing is deleted before OK");
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let doc = &app.session.doc().unwrap().doc;
        assert!(doc.swatch("Red").is_none() && doc.swatch("Bright Red").is_none() && doc.swatch_groups.iter().all(|g| g.name != "Brights"));
        let fill = doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
        assert_eq!(fill, Paint::solid(Color::from_hex("#ed1c24").unwrap()), "the art keeps its colour, unlinked");
        // Cancel keeps the swatch; Alt-click (now) deletes without asking.
        delete(&mut app, vec!["Orange".into()], false);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("message"), "Delete “Orange”?");
        crate::dialogs::cancel(&mut app);
        assert!(app.session.doc().unwrap().doc.swatch("Orange").is_some());
        delete(&mut app, vec!["Orange".into()], true);
        assert!(app.ui.dialog.is_none() && app.session.doc().unwrap().doc.swatch("Orange").is_none());
        assert_eq!(deletable(&["[None]".into(), REGISTRATION.into(), "Grays".into()]), ["Grays"]);
    }

    #[test]
    fn new_swatch_dialog_prefills_the_active_colour_and_saves_into_the_selected_group() {
        let mut app = app();
        let ctx = context();
        app.run("paint.setFill", json!({"color": "#ff8000"})).unwrap();
        new_swatch(&mut app, &["Bright Red".into()], true, false);
        let d = app.ui.dialog.as_ref().expect("New Swatch opens its dialog");
        assert_eq!(
            (d.kind.as_str(), d.str("name").as_str(), d.str("group").as_str()),
            (crate::dialogs::new_swatch::KIND, "R=255 G=128 B=0", "Brights")
        );
        assert!(d.bool("spot") && d.bool("global"), "Cmd/Ctrl-click makes a spot colour");
        dialog_frame(&mut app, &ctx);
        crate::dialogs::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        let w = doc.swatch("R=255 G=128 B=0").expect("created");
        assert!(w.spot && doc.swatch_group_of(&w.name).is_some_and(|g| doc.swatch_groups[g].name == "Brights"));
        // Alt-click saves at once; a gradient takes only a name and stays out of colour groups.
        app.run("paint.setFill", json!({"swatch": "Sunset"})).unwrap();
        new_swatch(&mut app, &["Brights".into()], false, true);
        assert!(app.ui.dialog.is_none() && app.session.doc().unwrap().doc.swatch("New Gradient Swatch 1").is_some());
        new_swatch(&mut app, &[], false, false);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("name").as_str(), d.fields.contains_key("__paint")), ("New Gradient Swatch 2", true));
        dialog_frame(&mut app, &ctx);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.session.doc().unwrap().doc.swatch("New Gradient Swatch 2").is_some_and(|w| matches!(w.paint, Paint::Gradient(_))));
    }

    #[test]
    fn new_color_group_dialog_makes_a_group_from_swatches_or_artwork() {
        let mut app = app();
        let ctx = context();
        app.run("ui.newColorGroup", json!({"swatches": ["Red", "Sunset"]})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("name").as_str(), d.bool("fromArtwork")), (crate::dialogs::new_color_group::KIND, "Color Group", false));
        dialog_frame(&mut app, &ctx);
        crate::dialogs::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        let g = doc.swatch_groups.iter().find(|g| g.name == "Color Group").expect("group made");
        assert_eq!(g.swatches.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["Red"], "gradients stay out");
        // With art selected (and no swatches) it starts from the artwork: its colours become global.
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": "#123456"})).unwrap();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("ui.newColorGroup", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("name").as_str(), d.bool("fromArtwork"), d.bool("toGlobal")), ("Color Group 2", true, true));
        dialog_frame(&mut app, &ctx);
        crate::dialogs::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        let g = doc.swatch_groups.iter().find(|g| g.name == "Color Group 2").expect("group made");
        assert_eq!(g.swatches.iter().map(|w| (w.name.as_str(), w.global)).collect::<Vec<_>>(), [("R=18 G=52 B=86", true)]);
        let fill = doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
        assert_eq!(fill, Paint::Solid { color: Color::from_hex("#123456").unwrap(), swatch: Some("R=18 G=52 B=86".into()), tint: 1.0 });
    }

    /// The centre of the tile or row of `name` as laid out by the last frame.
    fn tile_center(ctx: &egui::Context, name: &str) -> Pos2 {
        ctx.read_response(egui::Id::new(("swatch-tile", name))).unwrap_or_else(|| panic!("no tile for {name}")).rect.center()
    }

    /// Press at `from`, drag to `to` and release there, a frame per step from `time`.
    fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, from: Pos2, to: Pos2, time: f64) {
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(app, ctx, vec![Event::PointerMoved(from), button(from, true)], time, show);
        frame(app, ctx, vec![Event::PointerMoved(from + vec2(6.0, 4.0))], time + 0.1, show);
        frame(app, ctx, vec![Event::PointerMoved(to)], time + 0.2, show);
        frame(app, ctx, vec![Event::PointerMoved(to), button(to, false)], time + 0.3, show);
    }

    fn names_of(app: &VectorcraftApp, group: Option<&str>) -> Vec<String> {
        let d = &app.session.active().unwrap().doc;
        let list = match group {
            Some(g) => &d.swatch_groups.iter().find(|x| x.name == g).unwrap().swatches,
            None => &d.swatches,
        };
        list.iter().map(|w| w.name.clone()).collect()
    }

    #[test]
    fn find_field_filters_and_shift_extends_over_what_is_shown() {
        let mut app = app();
        let ctx = context();
        set_pstate(&ctx, "swatch-show-find", true);
        ctx.data_mut(|d| d.insert_temp(find_id(), "BRIGHT".to_string()));
        frame(&mut app, &ctx, vec![], 0.0, show);
        assert!(ctx.read_response(egui::Id::new(("swatch-tile", "Red"))).is_none(), "Red doesn't match");
        click_tile(&mut app, &ctx, "Bright Red", Modifiers::NONE, 1.0);
        click_tile(&mut app, &ctx, "Bright Blue", Modifiers::SHIFT, 2.0);
        assert_eq!(selected(&ctx), ["Bright Red", "Bright Yellow", "Bright Green", "Bright Blue"]);
        // A group whose name matches shows all its swatches.
        ctx.data_mut(|d| d.insert_temp(find_id(), "grays".to_string()));
        frame(&mut app, &ctx, vec![], 3.0, show);
        frame(&mut app, &ctx, vec![], 3.5, show);
        assert!(ctx.read_response(egui::Id::new(("swatch-tile", "K=50"))).is_some());
        assert!(ctx.read_response(egui::Id::new(("swatch-tile", "Bright Red"))).is_none());
    }

    #[test]
    fn list_view_rows_take_double_clicks_and_folder_clicks_select_the_group() {
        let mut app = app();
        let ctx = context();
        set_pstate(&ctx, "swatch-view", View::SmallList);
        double_click_tile(&mut app, &ctx, "Red");
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "double-click in a list opens Swatch Options");
        let d = &app.session.active().unwrap().doc;
        let sel = vec!["Brights".to_string()];
        assert!(highlighted(d, &sel).contains(&"Bright Violet"), "a selected folder highlights its swatches");
        assert_eq!(describe(&Paint::solid(Color::cmyk(0.0, 1.0, 1.0, 0.0)), true, false), "Global Process Color, CMYK");
        assert_eq!(describe(&Paint::solid(Color::gray(0.5)), true, true), "Spot Color, Grayscale");
    }

    #[test]
    fn dragging_tiles_reorders_and_moves_into_groups_as_one_undo_step_each() {
        let mut app = app();
        let ctx = context();
        frame(&mut app, &ctx, vec![], 0.0, show);
        let undo = app.session.doc().unwrap().history.undo.len();
        // Onto the left half of Red: before it.
        let (from, red) = (tile_center(&ctx, "Amber"), tile_center(&ctx, "Red"));
        drag(&mut app, &ctx, from, red - vec2(4.0, 0.0), 1.0);
        let n = names_of(&app, None);
        assert_eq!(n[n.iter().position(|x| x == "Red").unwrap() - 1], "Amber");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        // Onto a folder: into that group, at its end. (Two frames, so tile rects are current.)
        frame(&mut app, &ctx, vec![], 2.0, show);
        frame(&mut app, &ctx, vec![], 2.5, show);
        drag(&mut app, &ctx, tile_center(&ctx, "Amber"), tile_center(&ctx, "Grays"), 3.0);
        assert_eq!(names_of(&app, Some("Grays")).last().map(String::as_str), Some("Amber"));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 2);
        // A selection drags together, in panel order; a gradient can't join a group.
        set_pstate(&ctx, "swatch-selected", vec!["Lime".to_string(), "Sunset".to_string(), "Orange".to_string()]);
        frame(&mut app, &ctx, vec![], 4.0, show);
        frame(&mut app, &ctx, vec![], 4.5, show);
        drag(&mut app, &ctx, tile_center(&ctx, "Lime"), tile_center(&ctx, "Bright Red") - vec2(4.0, 0.0), 5.0);
        assert_eq!(names_of(&app, Some("Brights"))[..3], ["Orange", "Lime", "Bright Red"]);
        assert!(names_of(&app, None).contains(&"Sunset".to_string()));
        // Dropped back on itself: nothing moves.
        let (before, undo) = (names_of(&app, None), app.session.doc().unwrap().history.undo.len());
        frame(&mut app, &ctx, vec![], 6.0, show);
        frame(&mut app, &ctx, vec![], 6.5, show);
        let white = tile_center(&ctx, "White");
        drag(&mut app, &ctx, white, white + vec2(2.0, 0.0), 7.0);
        assert_eq!((names_of(&app, None), app.session.doc().unwrap().history.undo.len()), (before, undo));
    }

    #[test]
    fn art_dropped_on_the_panel_becomes_a_pattern_swatch() {
        let mut app = app();
        let id = app.run("shape.star", json!({"cx": 50, "cy": 50, "radius1": 20, "radius2": 9, "points": 5})).unwrap()["id"].as_u64().unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        let ctx = context();
        frame(&mut app, &ctx, vec![], 0.0, show);
        // Released over a swatch tile: the list takes it.
        let at = tile_center(&ctx, "Bright Red");
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![vectorcraft_doc::NodeId(id)]));
        let release = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
        frame(&mut app, &ctx, vec![Event::PointerMoved(at), release], 1.0, show);
        let patterns = app.run("pattern.list", json!({})).unwrap();
        assert_eq!(patterns["patterns"].as_array().map(Vec::len), Some(1), "{patterns}");
        assert_eq!(patterns["editing"], Value::Null, "no pattern editing mode");
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.node(vectorcraft_doc::NodeId(id)), before.node(vectorcraft_doc::NodeId(id)), "the art stays as it was");
        assert!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).is_none());
    }

    #[test]
    fn a_proxy_dropped_on_the_panel_becomes_a_swatch() {
        let mut app = app();
        let ctx = context();
        frame(&mut app, &ctx, vec![], 0.0, show);
        let at = tile_center(&ctx, "Bright Red");
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::paint(Paint::solid(Color::from_hex("#123456").unwrap())));
        let release = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
        frame(&mut app, &ctx, vec![Event::PointerMoved(at), release], 1.0, show);
        assert_eq!(
            names_of(&app, Some("Brights")).last().map(String::as_str),
            Some("R=18 G=52 B=86"),
            "dropped on a group's colour: into that group"
        );
        assert!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).is_none());
    }

    #[test]
    fn alt_dropping_a_gradient_on_a_gradient_swatch_replaces_it() {
        let mut app = app();
        let ctx = context();
        frame(&mut app, &ctx, vec![], 0.0, show);
        let grad = |c: &str| {
            let stops =
                vec![vectorcraft_color::GradientStop::new(0.0, Color::from_hex(c).unwrap()), vectorcraft_color::GradientStop::new(1.0, Color::WHITE)];
            Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient { kind: GradientKind::Radial, stops })))
        };
        let drop_on = |app: &mut VectorcraftApp, name: &str, paint: Paint, m: Modifiers, time: f64| {
            let at = tile_center(&ctx, name);
            egui::DragAndDrop::set_payload(&ctx, PanelDrag::paint(paint));
            let release = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: m };
            frame_with(app, &ctx, vec![Event::PointerMoved(at), release], m, time, show);
        };
        let count = app.session.doc().unwrap().doc.swatches_iter().count();
        drop_on(&mut app, "Ocean", grad("#ff0000"), Modifiers::ALT, 1.0);
        let d = &app.session.doc().unwrap().doc;
        let Some(Paint::Gradient(g)) = d.swatch("Ocean").map(|w| &w.paint) else { panic!("Ocean is a gradient") };
        assert_eq!((g.gradient.kind, d.swatches_iter().count()), (GradientKind::Radial, count), "replaced, not added");
        // Without Alt, or on a swatch of another kind, it becomes a new swatch.
        frame(&mut app, &ctx, vec![], 2.0, show);
        drop_on(&mut app, "Red", grad("#00ff00"), Modifiers::ALT, 3.0);
        assert_eq!(app.session.doc().unwrap().doc.swatches_iter().count(), count + 1);
        assert!(app.session.doc().unwrap().doc.swatch("Red").is_some_and(|w| w.paint.color().is_some()));
    }

    #[test]
    fn menu_targets_follow_the_panel_selection() {
        let app = app();
        let sel: Vec<String> = ["Red", "Sunset", "Grays", "Orange", "Brights"].map(String::from).to_vec();
        assert_eq!(mergeable(&app, &sel), ["Red", "Orange"], "Merge Swatches takes the selected colours, first kept");
        assert_eq!(selected_group(&app, &sel).as_deref(), Some("Grays"));
        assert_eq!(selected_group(&app, &sel[..2]), None);
    }

    #[test]
    fn kind_filter() {
        let solid = Paint::solid(Color::BLACK);
        let grad = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default())));
        assert!(Kind::All.accepts(&grad, false));
        assert!(Kind::Color.accepts(&solid, false));
        assert!(!Kind::Color.accepts(&grad, false));
        assert!(Kind::Gradient.accepts(&grad, true));
        assert!(Kind::Groups.accepts(&solid, true));
        assert!(!Kind::Groups.accepts(&solid, false));
    }

    #[test]
    fn the_active_tint_or_gradient_swatch_is_highlighted_and_a_tint_edits_its_base() {
        let mut app = app();
        app.run("swatch.new", json!({"name": "Ink", "color": "#cc0066", "spot": true})).unwrap();
        app.run("paint.setFill", json!({"swatch": "Ink", "tint": 40})).unwrap();
        assert_eq!(active_swatch(&app).as_deref(), Some("Ink"), "no tint swatch yet: its base");
        app.run("swatch.new", json!({})).unwrap();
        assert_eq!(active_swatch(&app).as_deref(), Some("Ink 40%"));
        let tint = entries(&app, Kind::Color, "Ink 40").into_iter().find(|e| e.name() == "Ink 40%").unwrap();
        assert!(matches!(tint, Entry::Swatch { global: true, spot: true, .. }), "a tint swatch shows its base's kind");
        app.run("paint.setFill", json!({"swatch": "Sunset"})).unwrap();
        assert_eq!(active_swatch(&app).as_deref(), Some("Sunset"), "the applied gradient swatch");
        app.run("ui.swatchOptions", json!({"name": "Ink 40%"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("__swatch").as_str()), (KIND, "Ink"), "a tint swatch opens its base's options");
    }

    #[test]
    fn medium_tiles_match_measured_metrics() {
        assert_eq!(View::MediumThumb.tile(), (15.5, 17.0));
        assert!(View::SmallList.is_list() && !View::LargeThumb.is_list());
    }

    #[test]
    fn lab_spot_tiles_show_the_colour_the_spot_colors_options_pick() {
        let mut app = app();
        let lab = Color::lab(60.0, 50.0, 20.0);
        app.run("swatch.new", json!({"name": "Ink", "color": lab, "spot": true})).unwrap();
        let tile = |app: &VectorcraftApp| {
            let ink = entries(app, Kind::All, "Ink").into_iter().find(|e| e.name() == "Ink");
            let Some(Entry::Swatch { paint, .. }) = ink else { panic!("the Ink tile") };
            paint.color().unwrap()
        };
        assert_eq!(tile(&app), lab);
        app.run("swatch.spotOptions", json!({"useLab": false})).unwrap();
        assert_eq!(tile(&app), app.session.active().unwrap().doc.global_color("Ink").unwrap());
        assert!(matches!(tile(&app), Color::Cmyk { .. }));
    }
}
