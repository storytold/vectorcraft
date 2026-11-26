//! Color Guide panel on the framework's `HarmonyPanel`: the base colour, the harmony-rule select
//! with the harmony strip, and the variation grid: a row per harmony colour with the colour itself
//! in the centre column, shades/cool/muted to the left and tints/warm/vivid to the right (steps
//! and reach from Color Guide Options). The base color is the color last chosen in the Color
//! panel, the Swatches panel or the Color Picker ([`choose_base`]); selecting artwork leaves it
//! as it is, and the base chip ("Set base color to the current color") takes the current color,
//! such as a cell applied from the guide or the selected artwork's color. A click on a cell
//! applies it to the active proxy, without changing the base, and selects it; Shift- or
//! Cmd/Ctrl-click adds cells to the selection that Save Colors as Swatches saves as swatches and
//! Save Color Group to Swatches saves as one group (without a selection, the group holds the
//! harmony colors). The framework drops the selection when the guide changes. A harmony color or
//! a cell dragged carries its color alone, as shown; the Swatches panel makes a swatch of it.
//! When the base is a spot color, or a tint of one, chosen from the Swatches panel, the Color
//! panel's tint slider or the base chip ([`SpotBase`]), the cells right of the center in the base
//! row of Tints/Shades are tints of that spot color: a click applies the tint, a drag carries it,
//! and saving makes tint swatches linked to the spot color. Every other cell is a process color.
//! Limit to Library (the library button, `ui.colorGuideLimit`) snaps every colour to a swatch
//! library or the document's swatches; Edit or Apply Colors opens Recolor Artwork with the
//! harmony colours, limited the same way.

use craft::behavior::IconId;
use craft::color_ui::ui::{GuideCell, GuideCellAt, HarmonyCommand, HarmonyMenu, HarmonyMenuAction, HarmonyPanel, HarmonyPanelState, HarmonyPicked};
use craft::theme::Srgba;
use craft::ui::contract::{Bind, PopupKind, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::Divider;
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::CommandState;
use craft::ui::sections::{FooterAction, FooterBar, Panel};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_color::harmony::{self, Guide, GuideOptions, Harmony, Variation};
use vectorcraft_color::recolor::Palette;
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::cmd::swatchlib;

use super::swatches::{self, colour_libraries, limit_key, limit_name};
use super::{active_paint, apply_click, color_json, library_panel, paint_params};
use crate::VectorcraftApp;
use crate::panel_drag::PanelDrag;

/// The base colour before one is set: an orange.
const DEFAULT_BASE: Color = Color::Rgb { r: 230.0 / 255.0, g: 120.0 / 255.0, b: 40.0 / 255.0 };

/// The spot color swatch the guide's base is a tint of (`UiState::color_guide_spot`).
#[derive(Clone, Debug, PartialEq)]
pub struct SpotBase {
    /// The document the swatch belongs to (its `uid`).
    pub doc: u64,
    /// The spot color swatch's name.
    pub swatch: String,
    /// The base's tint of it, 0..1.
    pub tint: f32,
    /// The names it had before renames, oldest first (at most as many as the History panel keeps
    /// states), for an Undo or Redo that gives it one of them back.
    pub earlier: Vec<String>,
    /// Its swatch went without a command the panel followed: the link waits for an Undo or Redo
    /// ([`check_spot`]).
    pub resting: bool,
}

/// Whether `name` is a spot color of `doc`.
fn is_spot(doc: &vectorcraft_doc::Document, name: &str) -> bool {
    doc.swatch(name).is_some_and(|w| w.spot) && doc.global_color(name).is_some()
}

/// The spot color the base is a tint of, while its swatch is still a spot color of the active
/// document.
fn live_spot(app: &VectorcraftApp) -> Option<&SpotBase> {
    let s = app.ui.color_guide_spot.as_ref()?;
    let st = app.session.active()?;
    (st.uid == s.doc && !s.resting && is_spot(&st.doc, &s.swatch)).then_some(s)
}

/// The color of tint `tint` (0..1) of global swatch `name` in the active document.
fn tint_color(app: &VectorcraftApp, name: &str, tint: f32) -> Option<Color> {
    app.session.active()?.doc.tint_paint(name, tint)?.color()
}

/// The guide's base color: the one last chosen or set with the base chip (a spot color base
/// follows edits to its swatch); before that, the current color.
pub(crate) fn base_color(app: &VectorcraftApp) -> Color {
    if let Some(c) = live_spot(app).and_then(|s| tint_color(app, &s.swatch, s.tint)) {
        return c;
    }
    app.ui.color_guide_base.unwrap_or_else(|| active_paint(app).color().unwrap_or(DEFAULT_BASE))
}

/// Make `c` the guide's base color: a color chosen in the Color panel, the Swatches panel or the
/// Color Picker. A color applied from the guide itself leaves the base as it is.
pub(crate) fn choose_base(app: &mut VectorcraftApp, c: Color) {
    choose_linked_base(app, c, None);
}

/// [`choose_base`] for a color linked to a global swatch at a tint (0..1): when that swatch is a
/// spot color of the active document, the base is that tint of it ([`SpotBase`]).
pub(crate) fn choose_linked_base(app: &mut VectorcraftApp, c: Color, link: Option<(String, f32)>) {
    app.ui.color_guide_base = Some(c);
    app.ui.color_guide_spot = link.and_then(|(swatch, tint)| {
        let st = app.session.active()?;
        st.doc.swatch(&swatch).filter(|w| w.spot)?;
        Some(SpotBase { doc: st.uid, swatch, tint: tint.clamp(0.0, 1.0), earlier: Vec::new(), resting: false })
    });
}

/// The spot color base before a command runs ([`before_command`]).
pub(crate) struct LinkBefore {
    /// Its document is the active one.
    active: bool,
    /// The color group that holds its swatch.
    group: Option<String>,
    /// The color the guide shows as the base.
    color: Option<Color>,
    /// The open documents' `uid`s in order, while there is a spot color base.
    docs: Vec<u64>,
    /// The active document's index.
    active_index: Option<usize>,
}

/// The spot color base before a command runs, for [`after_command`].
pub(crate) fn before_command(app: &VectorcraftApp) -> LinkBefore {
    let s = app.ui.color_guide_spot.as_ref();
    let active = app.session.active().is_some_and(|st| s.is_some_and(|s| s.doc == st.uid));
    // Its document, active or not.
    let st = s.and_then(|s| app.session.documents().iter().find(|d| d.uid == s.doc));
    let group =
        st.zip(s).and_then(|(st, s)| st.doc.swatch_groups.iter().find(|g| g.swatches.iter().any(|w| w.name == s.swatch)).map(|g| g.name.clone()));
    let docs = if s.is_some() { app.session.documents().iter().map(|d| d.uid).collect() } else { Vec::new() };
    // The color it shows: its swatch's tint in its document, while that is a spot color there and
    // the link does not rest (a resting link shows the color it last showed).
    let shown = st
        .zip(s)
        .filter(|(st, s)| !s.resting && st.doc.swatch(&s.swatch).is_some_and(|w| w.spot))
        .and_then(|(st, s)| st.doc.tint_paint(&s.swatch, s.tint)?.color());
    let color = s.map(|_| shown.unwrap_or_else(|| base_color(app)));
    LinkBefore { active, group, color, docs, active_index: app.session.active_index() }
}

/// Where the steps of a `command.batch` run, as far as the steps before each one show: new
/// documents open after the others and become the active one, and Close and Move Document shift
/// the documents after them.
struct BatchDocs {
    /// The tab index of the base's document, when known.
    base: Option<usize>,
    /// The base's document was closed.
    closed: bool,
    /// The tab index of the active document, when known.
    active: Option<usize>,
    /// The number of open documents, when known.
    len: Option<usize>,
}

impl BatchDocs {
    /// Whether the next step runs in the base's document, when that is known.
    fn in_base(&self) -> Option<bool> {
        if self.closed {
            return Some(false);
        }
        Some(self.active? == self.base?)
    }

    /// Follow step `id`, which ran with params `p` and replied `r`.
    fn step(&mut self, id: &str, p: &Value, r: &Value) {
        let index = |key: &str| p[key].as_u64().and_then(|i| usize::try_from(i).ok());
        // The tab index a step that opens a document replies with: a new one after the others, or
        // one already open made active.
        let opened = r["index"].as_u64().and_then(|i| usize::try_from(i).ok());
        match id {
            "document.activate" => self.active = index("index"),
            "file.new" | "file.newFromTemplate" | "edit.newDocumentFromState" => {
                self.len = self.len.map(|n| n + 1);
                self.active = opened.or(self.len.map(|n| n - 1));
            }
            "file.close" | "document.close" => {
                let i = index("index").or(self.active);
                let len = self.len.map(|n| n.saturating_sub(1));
                match (i, self.base) {
                    (Some(i), Some(b)) if i == b => (self.closed, self.base) = (true, None),
                    (Some(i), Some(b)) if i < b => self.base = Some(b - 1),
                    (Some(_), Some(_)) => {}
                    _ => self.base = None,
                }
                // Closing another document keeps the active one; closing the active one makes the
                // one at its index active (the last one when it was last).
                self.active = match (i, self.active) {
                    (Some(i), Some(a)) if i != a => Some(if i < a { a - 1 } else { a }),
                    (Some(i), Some(_)) => len.and_then(|n| n.checked_sub(1)).map(|last| i.min(last)),
                    _ => None,
                };
                self.len = len;
            }
            "file.closeAll" => self.closed = true,
            "document.move" => {
                let moved = |x: usize, from: usize, to: usize| {
                    if x == from {
                        return to;
                    }
                    let x = if from < x { x - 1 } else { x };
                    if to <= x { x + 1 } else { x }
                };
                match (index("from").or(self.active), index("to"), self.len) {
                    (Some(from), Some(to), Some(n)) if n > 0 => {
                        let to = to.min(n - 1);
                        self.base = self.base.map(|b| moved(b, from, to));
                        self.active = self.active.map(|a| moved(a, from, to));
                    }
                    _ => (self.base, self.active) = (None, None),
                }
            }
            _ if id.starts_with("file.open")
                || id.starts_with("recovery.")
                || matches!(id, "document.open" | "document.compare" | "stroke.arrowheads.edit") =>
            {
                match (opened, self.len) {
                    (Some(i), Some(n)) if i <= n => (self.active, self.len) = (Some(i), Some(n.max(i + 1))),
                    _ => (self.active, self.len) = (None, None),
                }
            }
            _ => {}
        }
    }
}

/// The name the spot color base's swatch `name` of document `base` has after a `command.batch`
/// ran `steps` and replied `results`, or `None` when a step may have
/// removed it or made it a process color ([`follow_step`]). Steps in the base's document are
/// followed and steps in another document are skipped. A step in a document that is not known
/// keeps the name, and a rename there adds a second name to choose from once the batch is done:
/// the one that is a spot color of the base's document. Scripts and plug-ins may do anything, so
/// they end the link.
fn follow_batch(app: &VectorcraftApp, base: u64, name: String, steps: &[Value], results: &[Value], before: &LinkBefore) -> Option<String> {
    let mut group = before.group.clone();
    let at = before.docs.iter().position(|&u| u == base);
    let mut docs = BatchDocs { base: at, closed: at.is_none(), active: before.active_index, len: Some(before.docs.len()) };
    let mut names = vec![name];
    for (i, step) in steps.iter().enumerate() {
        let sid = step["command"].as_str().unwrap_or_default();
        let (p, r) = (&step["params"], results.get(i).unwrap_or(&Value::Null));
        if runs_code(sid) {
            return None;
        }
        match docs.in_base() {
            Some(true) => {
                let one = names.len() == 1;
                let mut next = Vec::with_capacity(names.len());
                for n in names {
                    let mut scratch = group.clone();
                    next.push(follow_step(sid, p, r, n, if one { &mut group } else { &mut scratch })?);
                }
                names = next;
            }
            Some(false) => {}
            None => {
                // The step may have run elsewhere, so the name stays a candidate whatever the step
                // did; a rename adds the new name.
                let mut next = Vec::with_capacity(names.len() + 1);
                for n in names {
                    if let Some(m) = follow_step(sid, p, r, n.clone(), &mut group.clone())
                        && m != n
                    {
                        next.push(m);
                    }
                    next.push(n);
                }
                next.sort();
                next.dedup();
                names = next;
            }
        }
        docs.step(sid, p, r);
    }
    if names.len() == 1 {
        return names.pop();
    }
    // The one name that is a spot color of the base's document.
    let st = app.session.documents().iter().find(|d| d.uid == base)?;
    let mut spots = names.into_iter().filter(|n| is_spot(&st.doc, n));
    let first = spots.next()?;
    spots.next().is_none().then_some(first)
}

/// Whether command `id` runs a script or a plug-in (Run Script, a plug-in filter or a plug-in's own
/// command), whose commands the panel does not follow.
fn runs_code(id: &str) -> bool {
    matches!(id, "script.run" | "plugin.run") || vectorcraft_engine::cmd::plugin::is_command_id(id)
}

/// Command `id` ran with `params` and replied `reply`: a spot color base linked to a swatch that
/// Swatch Options renamed links to the new name, and one whose swatch a command deleted or made a
/// process color is no longer linked to it, so a later spot color of the same name does not
/// become the base; the base keeps the color it showed. A `command.batch`'s steps are followed in
/// order ([`follow_batch`]). A script or a plug-in may do anything, so it ends the link.
pub(crate) fn after_command(app: &mut VectorcraftApp, id: &str, params: &Value, reply: &Value, before: LinkBefore) {
    let Some(s) = app.ui.color_guide_spot.as_ref() else { return };
    let (base, n) = (s.doc, s.swatch.clone());
    let name = if runs_code(id) {
        None
    } else if id == "command.batch" {
        let steps = params["commands"].as_array().map(Vec::as_slice).unwrap_or_default();
        let results = reply["results"].as_array().map(Vec::as_slice).unwrap_or_default();
        follow_batch(app, base, n, steps, results, &before)
    } else if before.active {
        follow_step(id, params, reply, n, &mut before.group.clone())
    } else {
        Some(n)
    };
    let keep = usize::try_from(app.session.prefs.history_states).unwrap_or(usize::MAX).max(1);
    let Some(s) = app.ui.color_guide_spot.as_mut() else { return };
    match name {
        Some(n) => {
            if n != s.swatch {
                let old = std::mem::replace(&mut s.swatch, n);
                s.earlier.retain(|e| *e != old && *e != s.swatch);
                s.earlier.push(old);
                if s.earlier.len() > keep {
                    s.earlier.drain(..s.earlier.len() - keep);
                }
            }
        }
        None => app.ui.color_guide_spot = None,
    }
    let undo = |id: &str| matches!(id, "edit.undo" | "edit.redo");
    let undone = undo(id)
        || (id == "command.batch"
            && params["commands"].as_array().is_some_and(|a| a.iter().any(|st| undo(st["command"].as_str().unwrap_or_default()))));
    check_spot(app, if undone { Ran::UndoRedo } else { Ran::Command });
    if app.ui.color_guide_spot.is_none()
        && let Some(c) = before.color
    {
        app.ui.color_guide_base = Some(c);
    }
}

/// The name swatch `name` (in color group `group`) has after command `id` ran with `params` and
/// replied `reply` in its document, or `None` when the command may have removed it or made it a
/// process color: Swatch Options making it a process color, Delete Swatch naming it or its group,
/// Merge Swatches merging it into another and Edit Color Group leaving it out of its group.
/// `group` follows the step: New Color Group, Move Swatch, Edit Color Group, Ungroup Color Group
/// and Color Group Options.
fn follow_step(id: &str, p: &Value, reply: &Value, name: String, group: &mut Option<String>) -> Option<String> {
    let ours = |v: &Value| v == name.as_str();
    let in_group = |key: &str| group.as_deref().is_some_and(|g| p[key] == g);
    let named = |key: &str| p[key].as_array().is_some_and(|a| a.iter().any(|n| ours(n) || group.as_deref().is_some_and(|g| n == g)));
    let members = |key: &str| reply[key].as_array().is_some_and(|a| a.iter().any(ours));
    let removed = match id {
        "swatch.edit" => ours(&p["name"]) && p["spot"] == false,
        "swatch.delete" => ours(&p["name"]) || in_group("name") || named("names"),
        "swatch.merge" => named("names") && !ours(&p["names"][0]),
        "swatch.editGroup" => in_group("group") && !members("swatches"),
        _ => false,
    };
    if removed {
        return None;
    }
    // The color group that holds it after the step.
    let moved = match id {
        "swatch.newGroup" | "swatch.editGroup" if members("swatches") => Some(reply["name"].as_str().map(str::to_string)),
        "swatch.ungroup" if in_group("name") => Some(None),
        "swatch.renameGroup" if in_group("group") => Some(reply["name"].as_str().map(str::to_string)),
        "swatch.move" if ours(&p["name"]) || p["names"].as_array().is_some_and(|a| a.iter().any(ours)) => {
            Some(p["group"].as_str().map(str::to_string))
        }
        _ => None,
    };
    if let Some(m) = moved {
        *group = m;
    }
    // A rename by Swatch Options, or of the swatch Merge Swatches keeps.
    let renamed = match id {
        "swatch.edit" if p["name"] == name.as_str() => reply["name"].as_str(),
        "swatch.merge" if p["names"][0] == name.as_str() => reply["name"].as_str(),
        _ => None,
    };
    Some(renamed.map_or(name, str::to_string))
}

/// What ran before the spot color base's link is checked ([`check_spot`]).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Ran {
    /// Nothing the panel followed: a frame of the panel (a script's commands, for example).
    Frame,
    /// A command other than Undo and Redo.
    Command,
    /// Undo or Redo, or a batch with one of them.
    UndoRedo,
}

/// Check the spot color base's link after `ran`. A closed document ends it. When its swatch is no
/// longer a spot color of its document (deleted, renamed or made a process color by something the
/// panel did not follow), after an Undo or Redo a name it had before that is one again takes its
/// place (an Undo or Redo of a rename); otherwise the link rests until an Undo or Redo makes it a spot color again, and
/// the next other command ends it. The base keeps the color it last showed.
pub(crate) fn check_spot(app: &mut VectorcraftApp, ran: Ran) {
    let Some(s) = app.ui.color_guide_spot.as_mut() else { return };
    if !app.session.documents().iter().any(|d| d.uid == s.doc) {
        app.ui.color_guide_spot = None;
        return;
    }
    let Some(st) = app.session.active().filter(|st| st.uid == s.doc) else { return };
    if s.resting && ran != Ran::UndoRedo {
        if ran == Ran::Command {
            app.ui.color_guide_spot = None;
        }
        return;
    }
    if ran == Ran::UndoRedo && !is_spot(&st.doc, &s.swatch) {
        // An Undo or Redo of a rename gave it a name it had before.
        if let Some(i) = s.earlier.iter().rposition(|n| is_spot(&st.doc, n)) {
            let name = s.earlier.remove(i);
            let gone = std::mem::replace(&mut s.swatch, name);
            s.earlier.push(gone);
        }
    }
    if is_spot(&st.doc, &s.swatch) {
        s.resting = false;
        // Kept for when the link ends where the color can no longer be read (a closed document).
        if let Some(c) = st.doc.tint_paint(&s.swatch, s.tint).and_then(|p| p.color()) {
            app.ui.color_guide_base = Some(c);
        }
        return;
    }
    if ran != Ran::Command {
        s.resting = true;
        return;
    }
    if let Some(c) = st.doc.tint_paint(&s.swatch, s.tint).and_then(|p| p.color()) {
        app.ui.color_guide_base = Some(c);
    }
    app.ui.color_guide_spot = None;
}

/// The global swatch and tint (0..1) a solid paint links to.
fn paint_link(p: &Paint) -> Option<(String, f32)> {
    match p {
        Paint::Solid { swatch: Some(n), tint, .. } => Some((n.clone(), *tint)),
        _ => None,
    }
}

/// The tint of the spot color base that cell (`row`, `col`) of the guide `g` is: a cell right of
/// the center in the base row of Tints/Shades, without a library limit. Its tint is the base's
/// tint less the share the cell goes towards white. → (swatch, tint 0..1)
fn cell_tint(app: &VectorcraftApp, g: &Guide, row: usize, col: usize) -> Option<(String, f32)> {
    let opts = app.ui.color_guide.clamped();
    let centre = g.centre();
    if row != 0 || col <= centre || opts.variation != Variation::TintsShades || !app.ui.color_guide_limit.is_empty() {
        return None;
    }
    let s = live_spot(app)?;
    let step = i32::try_from(col - centre).ok()?;
    Some((s.swatch.clone(), s.tint * (1.0 - harmony::step_fraction(&opts, step))))
}

/// The paint cell (`row`, `col`) of `g` applies, carries and saves: a tint of the spot color base
/// ([`cell_tint`], in its own color) or the color the cell shows.
fn cell_paint(app: &VectorcraftApp, g: &Guide, row: usize, col: usize) -> Option<Paint> {
    let color = *g.grid.get(row)?.get(col)?;
    Some(match cell_tint(app, g, row, col) {
        Some((name, tint)) => Paint::Solid { color: tint_color(app, &name, tint).unwrap_or(color), swatch: Some(name), tint },
        None => Paint::solid(color),
    })
}

/// The `swatch.new` / `swatch.newGroup` entry for cell (`row`, `col`) of `g`: `{swatch, tint}` for
/// a tint of the spot color base, the color otherwise.
fn cell_entry(app: &VectorcraftApp, g: &Guide, row: usize, col: usize) -> Option<Value> {
    Some(match cell_paint(app, g, row, col)? {
        p @ Paint::Solid { swatch: Some(_), .. } => paint_params(&p),
        p => color_json(&p.color()?),
    })
}

/// What the limit palette is made from: the library key and, while limited to the document's
/// swatches, the document's identity and revision.
type LimitKey = (String, Option<(u64, u64)>);

/// The colours Limit to Library snaps to, for the guide (`palette`) and for the panel (`shown`).
#[derive(Clone, Debug)]
struct Limited {
    key: LimitKey,
    palette: Palette,
    shown: Vec<Srgba>,
}

/// The Color Guide panel's session state: the harmony rule, the cells picked for Save Colors as
/// Swatches and the limit palette. The saved options and limit stay in `UiState::color_guide` and
/// `UiState::color_guide_limit`.
#[derive(Clone, Debug)]
pub struct Model {
    rule: Harmony,
    picked: HarmonyPicked,
    /// Snapping every cell to a library is slow, so the palette is kept until its key changes.
    palette: Option<Limited>,
}

impl Default for Model {
    fn default() -> Self {
        Model { rule: Harmony::Complementary, picked: HarmonyPicked::default(), palette: None }
    }
}

/// The framework's colour for a document colour, as it displays.
fn srgba(c: &Color) -> Srgba {
    Srgba::from_hex(&c.to_hex()).unwrap_or_else(|| Srgba::rgb(0.0, 0.0, 0.0))
}

/// The document colour of a cell the framework shows (sRGB).
fn color_of(c: Srgba) -> Color {
    Color::from_hex(&c.to_hex()).unwrap_or(DEFAULT_BASE)
}

/// The panel's settings as the framework keeps them.
// `HarmonyPanelState` is `#[non_exhaustive]`: only `Default` and its fields build one.
#[allow(clippy::field_reassign_with_default)]
fn panel_state(rule: Harmony, options: GuideOptions) -> HarmonyPanelState {
    let mut state = HarmonyPanelState::default();
    state.rule = rule;
    state.options = options;
    state
}

/// The key of the limit palette now; `None` without a limit.
fn limit_key_now(app: &VectorcraftApp) -> Option<LimitKey> {
    let limit = &app.ui.color_guide_limit;
    if limit.is_empty() {
        return None;
    }
    let doc = app.session.active().filter(|_| limit == swatchlib::DOCUMENT_SWATCHES).map(|d| (d.uid, d.revision));
    Some((limit.clone(), doc))
}

/// Bring the kept limit palette up to date with the limit and, for the document's swatches, the
/// document.
fn refresh_palette(app: &mut VectorcraftApp) {
    let key = limit_key_now(app);
    if app.ui.color_guide_panel.palette.as_ref().map(|l| &l.key) == key.as_ref() {
        return;
    }
    let built = key.and_then(|key| {
        let colors = swatchlib::library_colors(&app.session, &key.0)?;
        Some(Limited { shown: colors.iter().map(srgba).collect(), palette: Palette::new(colors), key })
    });
    app.ui.color_guide_panel.palette = built;
}

/// The guide the actions read: the harmony colours and variations of the base, limited to the
/// Limit to Library colours. Built where a guide is read, not each pass.
fn guide(app: &mut VectorcraftApp) -> Guide {
    refresh_palette(app);
    let model = &app.ui.color_guide_panel;
    let g = Guide::new(base_color(app), model.rule, &app.ui.color_guide);
    match &model.palette {
        Some(l) => g.limited(&l.palette),
        None => g,
    }
}

/// The paints of the spot color base's tints in the base row (column, paint): the cells right of
/// the centre of Tints/Shades without a library limit. Empty otherwise.
fn spot_paints(app: &VectorcraftApp, rule: Harmony) -> Vec<(usize, Paint)> {
    if live_spot(app).is_none() || !app.ui.color_guide_limit.is_empty() || app.ui.color_guide.variation != Variation::TintsShades {
        return Vec::new();
    }
    let g = Guide::new(base_color(app), rule, &app.ui.color_guide);
    let cols = g.grid.first().map_or(0, Vec::len);
    (g.centre() + 1..cols)
        .filter_map(|col| cell_paint(app, &g, 0, col).filter(|p| matches!(p, Paint::Solid { swatch: Some(_), .. })).map(|p| (col, p)))
        .collect()
}

/// The grid row and column of a cell at `at`; none for a cell of the harmony strip.
fn grid_spot(at: GuideCellAt) -> Option<(usize, usize)> {
    match at {
        GuideCellAt::Grid { row, column, .. } => Some((row, column)),
        _ => None,
    }
}

/// The spot tint of base-row column `column` ([`spot_paints`]), if it has one.
fn tint_at(tints: &[(usize, Paint)], spot: Option<(usize, usize)>) -> Option<&Paint> {
    let (0, column) = spot? else { return None };
    tints.iter().find(|(col, _)| *col == column).map(|(_, p)| p)
}

/// What a cell at grid spot `spot` showing `color` applies, carries and saves: the spot tint of the
/// base row or the color it shows.
fn tint_or_solid(tints: &[(usize, Paint)], spot: Option<(usize, usize)>, color: Srgba) -> Paint {
    tint_at(tints, spot).cloned().unwrap_or_else(|| Paint::solid(color_of(color)))
}

/// What a cell carries when dragged.
fn drag_payload(tints: &[(usize, Paint)], spot: Option<(usize, usize)>, color: Srgba) -> PanelDrag {
    PanelDrag::paint(tint_or_solid(tints, spot, color))
}

/// `ui.colorGuideLimit {library}`: limit the panel's colours to a swatch library (an id or name),
/// the document's swatches ("document"), or nothing (`""` or null). → {limitTo, name}
pub(crate) fn set_limit(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = match p.get("library").and_then(Value::as_str).unwrap_or_default() {
        "" => String::new(),
        l => limit_key(app, l)?,
    };
    let name = limit_name(app, &key);
    app.ui.color_guide_limit = key;
    Ok(json!({"limitTo": app.ui.color_guide_limit, "name": name}))
}

/// Edit or Apply Colors: Recolor Artwork with the harmony colours as the new colours (of the
/// selected art, or of the colours alone), limited to the panel's library.
fn edit_or_apply(app: &mut VectorcraftApp) {
    let g = guide(app);
    let mut p = json!({ "colors": g.colors.iter().map(color_json).collect::<Vec<_>>() });
    if limit_name(app, &app.ui.color_guide_limit).is_some() {
        p["library"] = json!(app.ui.color_guide_limit);
    }
    app.run("ui.recolorDialog", p).ok();
}

/// The `swatch.new` / `swatch.newGroup` entries of the grid cells at `cells` (row, column), in the
/// order given: the framework keeps picked cells in grid order, each once.
fn cell_entries(app: &VectorcraftApp, g: &Guide, cells: &[(usize, usize)]) -> Vec<Value> {
    cells.iter().filter_map(|&(row, column)| cell_entry(app, g, row, column)).collect()
}

/// The grid cells picked for Save Colors as Swatches.
fn picked(app: &VectorcraftApp) -> Vec<(usize, usize)> {
    app.ui.color_guide_panel.picked.cells().iter().filter_map(|cell| grid_spot(cell.at)).collect()
}

/// Save `cells`' colors as swatches (one undo step), each in its own model; a tint of a spot color
/// base becomes a tint swatch of it.
fn save_selected(app: &mut VectorcraftApp, cells: &[(usize, usize)]) {
    let g = guide(app);
    let colors = cell_entries(app, &g, cells);
    if !colors.is_empty() {
        app.run("swatch.new", json!({ "colors": colors, "keepModel": true })).ok();
    }
}

/// Save Color Group to Swatches: `cells` as one swatch group in the order given; with none, the
/// harmony colors. The group is named after the harmony rule, saved in one undo step, and each
/// color keeps its own model; a tint of a spot color base becomes a tint swatch of it.
fn save_group(app: &mut VectorcraftApp, cells: &[(usize, usize)]) {
    let g = guide(app);
    let mut colors = cell_entries(app, &g, cells);
    if colors.is_empty() {
        colors = g.colors.iter().map(color_json).collect();
    }
    app.run("swatch.newGroup", json!({"name": harmony::label(app.ui.color_guide_panel.rule), "colors": colors, "keepModel": true})).ok();
}

/// A cell of the guide was clicked: apply its paint to the active proxy (the inactive one with Alt
/// held), leaving the base as it is.
fn chosen(app: &mut VectorcraftApp, ui: &Ui, spot: Option<(usize, usize)>, color: Srgba) {
    let paint = tint_or_solid(&spot_paints(app, app.ui.color_guide_panel.rule), spot, color);
    apply_click(app, ui, paint_params(&paint));
}

/// The base chip was clicked: the current color becomes the base.
fn set_base(app: &mut VectorcraftApp) {
    let current = active_paint(app);
    if let Some(c) = current.color() {
        choose_linked_base(app, c, paint_link(&current));
    }
}

// The footer's actions, in order: the limit at the start edge, the rest from the end edge, so they read Edit or
// Apply, Save Selected, Save Group from left to right.
const LIMIT: usize = 0;
const SAVE_GROUP: usize = 1;
const SAVE_SELECTED: usize = 2;
const EDIT: usize = 3;

/// The Limit to Library popup's rows: None, Document Swatches, then the colour libraries. → the
/// key of the row that ran.
fn limit_rows(app: &VectorcraftApp, ui: &mut Ui) -> Option<String> {
    let cur = &app.ui.color_guide_limit;
    let mut picked = None;
    for (key, label) in [("", tl!("None")), (swatchlib::DOCUMENT_SWATCHES, swatches::DOCUMENT_SWATCHES)] {
        if MenuItem::new(label.to_string()).check(cur == key).show(ui).ran {
            picked = Some(key.to_string());
        }
    }
    Divider::horizontal().show(ui);
    library_panel::library_items(ui, &colour_libraries(app), &|id| id == cur).or(picked)
}

/// The button's tip, with the library's name when one limits the colors.
fn limit_tip(app: &VectorcraftApp) -> (String, bool) {
    let key = &app.ui.color_guide_limit;
    match limit_name(app, key).map(|n| library_panel::library_name(key, &n).to_string()) {
        // The tip is translated around the library's name, not looked up whole.
        Some(name) => (crate::i18n::fmt(tl!("Limit colors to swatch library: {name}"), &[("name", &name)]), true),
        None => (tl!("Limit colors to swatch library").to_string(), false),
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    check_spot(app, Ran::Frame);
    // The current color becomes the base when the panel first shows; selecting artwork later
    // leaves it as it is.
    if app.ui.color_guide_base.is_none() {
        let current = active_paint(app);
        choose_linked_base(app, current.color().unwrap_or(DEFAULT_BASE), paint_link(&current));
    }
    refresh_palette(app);
    // The model leaves the app for the pass so the panel can borrow its palette while the footer
    // reads the app; it goes back before any action runs.
    let mut model = std::mem::take(&mut app.ui.color_guide_panel);
    let mut state = panel_state(model.rule, app.ui.color_guide);
    let base = srgba(&base_color(app));
    let tints = spot_paints(app, model.rule);
    let tint_colors: Vec<(usize, Srgba)> = tints.iter().filter_map(|(col, p)| Some((*col, srgba(&p.color()?)))).collect();
    let limit = model.palette.as_ref().map(|l| l.shown.as_slice()).filter(|s| !s.is_empty());
    let has_cells = !model.picked.cells().is_empty();
    let doc = app.session.active().is_some();
    let (tip, limited) = limit_tip(app);
    let shared: &VectorcraftApp = app;
    let (mut cell, mut base_clicked, mut footer_picked, mut limit_picked) = (None, false, None, None);
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    // A docked panel shows no header: its dock tab names it.
    let _shown = Panel::new(tl!("Color Guide").to_string())
        .footer(|ui, _| {
            let actions = [
                FooterAction::new(icon("library"), tip.clone()).pressed(limited).popup(PopupKind::Menu),
                FooterAction::new(icon("dc-folder"), tl!("Save color group to Swatch panel").to_string()),
                FooterAction::new(icon("dc-new-item"), tl!("Save selected colors as swatches").to_string()).enabled(has_cells),
                FooterAction::new(icon("palette"), tl!("Edit or Apply Colors").to_string()).enabled(doc),
            ];
            let out = FooterBar::actions(actions).at_start(LIMIT + 1).show_footer(ui);
            footer_picked = out.picked;
            limit_picked = None;
            if let Some(button) = out.responses.get(LIMIT) {
                let _ = Menu::new(tip.clone()).opened_by(button, Trigger::Press).show(ui, |ui, _| limit_picked = limit_rows(shared, ui));
            }
        })
        .show(ui, |ui, _| {
            // The body can run more than once in a pass: each run starts from nothing chosen.
            let payload_tints = tints.clone();
            let mut panel = HarmonyPanel::new(Bind::bound(&mut state), Bind::bound(&mut model.picked), base)
                .cell_color(|c: &GuideCell| match grid_spot(c.at) {
                    Some((0, column)) => tint_colors.iter().find(|(col, _)| *col == column).map_or(c.color, |(_, s)| *s),
                    _ => c.color,
                })
                .drag_payload(move |c: &GuideCell| Some(drag_payload(&payload_tints, grid_spot(c.at), c.color)));
            if let Some(palette) = limit {
                panel = panel.limit(palette);
            }
            let out = panel.show(ui);
            (cell, base_clicked) = (out.chosen, out.set_base);
        });
    model.rule = state.rule;
    if state.options != app.ui.color_guide {
        app.ui.color_guide = state.options;
    }
    app.ui.color_guide_panel = model;
    if base_clicked {
        set_base(app);
    }
    if let Some(cell) = cell {
        chosen(app, ui, grid_spot(cell.at), cell.color);
    }
    if let Some(key) = limit_picked {
        set_limit(app, &json!({ "library": key })).ok();
    }
    let cells = picked(app);
    match footer_picked {
        Some(EDIT) => edit_or_apply(app),
        Some(SAVE_SELECTED) => save_selected(app, &cells),
        Some(SAVE_GROUP) => save_group(app, &cells),
        _ => {}
    }
}

/// The rows of the panel menu: the variation, Save Colors as Swatches and Color Guide Options.
pub(crate) fn menu_rows() -> Vec<PanelRow> {
    HarmonyMenu::rows()
}

/// The state of each menu command.
pub(crate) fn menu_states(app: &VectorcraftApp) -> Vec<(HarmonyCommand, CommandState)> {
    HarmonyMenu::states(&panel_state(app.ui.color_guide_panel.rule, app.ui.color_guide), &app.ui.color_guide_panel.picked)
}

/// Carry out menu command `command`.
pub(crate) fn run(app: &mut VectorcraftApp, command: HarmonyCommand) {
    let mut state = panel_state(app.ui.color_guide_panel.rule, app.ui.color_guide);
    let action = HarmonyMenu::run_command(&mut state, &app.ui.color_guide_panel.picked, command);
    app.ui.color_guide = state.options;
    match action {
        // The cells go through `save_selected`, not the colours the action carries: each swatch
        // keeps its model and a spot tint stays a tint swatch.
        Some(HarmonyMenuAction::SaveAsSwatches(_)) => {
            let cells = picked(app);
            save_selected(app, &cells);
        }
        Some(HarmonyMenuAction::Options) => {
            app.run("ui.colorGuideOptions", json!({})).ok();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app
    }

    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
    }

    /// The guide of the base without a library limit, as the panel makes it.
    fn guide_of(app: &VectorcraftApp) -> Guide {
        Guide::new(base_color(app), app.ui.color_guide_panel.rule, &app.ui.color_guide)
    }

    /// Where a cell lies: the harmony strip's cell at an index, or the grid's at a row and column.
    #[derive(Clone, Copy, Debug)]
    enum At {
        Strip(usize),
        Grid(usize, usize),
    }

    /// The grid spot and colour of the cell at `at` of `g`, as the panel reports a click on it.
    fn cell_at(g: &Guide, at: At) -> (Option<(usize, usize)>, Srgba) {
        match at {
            At::Strip(i) => (None, srgba(&g.colors[i])),
            At::Grid(row, column) => (Some((row, column)), srgba(&g.grid[row][column])),
        }
    }

    /// A click on the cell at `at` of the guide shown now, through the panel's click glue.
    fn click(app: &mut VectorcraftApp, at: At) {
        let (spot, color) = cell_at(&guide_of(app), at);
        let mut out = crate::testing::ui_context().run_ui(egui::RawInput::default(), |ui| chosen(app, ui, spot, color));
        out.textures_delta.clear();
    }

    /// A cell of grid row `row` and column `column`.
    fn grid(row: usize, column: usize) -> At {
        At::Grid(row, column)
    }

    /// The first show takes the current color; a color chosen in the Color panel, Swatches or the
    /// Color Picker becomes the base; selecting artwork with another color leaves it.
    #[test]
    fn a_chosen_color_becomes_the_base_and_selecting_art_keeps_it() {
        let mut app = app();
        let red = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        let blue = app.run("shape.rectangle", json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"color": "#3366cc"})).unwrap();
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        assert_eq!(base_color(&app).to_hex(), "#3366cc", "the first show takes the current color");
        app.run("select.set", json!({"ids": [red]})).unwrap();
        frame(&mut app, &ctx);
        assert_eq!(base_color(&app).to_hex(), "#3366cc", "selecting art keeps the base");
        choose_base(&mut app, Color::from_hex("#20a060").unwrap());
        frame(&mut app, &ctx);
        app.ui.color_guide.steps = 6;
        frame(&mut app, &ctx);
        let g = guide(&mut app);
        assert_eq!(g.grid[0].len(), 13);
        assert_eq!(g.grid[0][6].to_hex(), "#20a060", "the chosen base sits in the centre of the first row");
        app.run("select.set", json!({"ids": [blue]})).unwrap();
        frame(&mut app, &ctx);
        assert_eq!(base_color(&app).to_hex(), "#20a060");
    }

    /// A harmony color or a variation clicked in the guide paints the art without changing the
    /// base; the base chip then makes the current color the base.
    #[test]
    fn a_guide_color_applies_without_moving_the_base_until_set_as_base() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("paint.setFill", json!({"color": "#3366cc"})).unwrap();
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let g = guide_of(&app);
        let fill = |app: &VectorcraftApp| crate::panels::current_paints(app).0.color().map(|c| c.to_hex());
        click(&mut app, At::Strip(1));
        assert_eq!(fill(&app), Some(g.colors[1].to_hex()), "the harmony color paints the art");
        assert_eq!(base_color(&app).to_hex(), "#3366cc", "and leaves the base");
        click(&mut app, grid(0, 2));
        let variation = g.grid[0][2].to_hex();
        assert_eq!(fill(&app), Some(variation.clone()), "a variation paints the art");
        assert_eq!(base_color(&app).to_hex(), "#3366cc");
        assert_eq!(guide_of(&app).grid, g.grid, "the guide stays as it was");
        // Set base color to the current color: the applied variation becomes the base.
        set_base(&mut app);
        assert_eq!(base_color(&app).to_hex(), variation, "the base chip adopts the current color");
    }

    #[test]
    fn save_colors_as_swatches_saves_each_selected_cell() {
        let mut app = app();
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let before = app.session.doc().unwrap().doc.swatches.len();
        let cells = &[(0, 0), (1, 4), (0, 8)].to_vec();
        let undo = app.session.doc().unwrap().history.undo.len();
        save_selected(&mut app, cells);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.swatches.len(), before + 3);
        assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
        let g = guide_of(&app);
        assert_eq!(st.doc.swatches.last().unwrap().paint.color().map(|c| c.to_hex()), Some(g.grid[0][8].to_hex()));
    }

    /// In a CMYK document the cells of an RGB guide are saved in RGB, their own model.
    #[test]
    fn saved_cells_keep_their_model_in_a_cmyk_document() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("paint.setFill", json!({"color": "#3366cc", "keepModel": true})).unwrap();
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let cells = &[(0, 0), (0, 6)].to_vec();
        save_selected(&mut app, cells);
        let g = guide_of(&app);
        assert!(matches!(g.grid[0][6], Color::Rgb { .. }), "{:?}", g.grid[0][6]);
        let sw = &app.session.doc().unwrap().doc.swatches;
        let saved: Vec<Color> = sw[sw.len() - 2..].iter().filter_map(|w| w.paint.color()).collect();
        assert_eq!(saved, [g.grid[0][0], g.grid[0][6]]);
    }

    /// The colors of the document's newest swatch group, and the group's name.
    fn last_group(app: &VectorcraftApp) -> (String, Vec<Color>) {
        let g = app.session.doc().unwrap().doc.swatch_groups.last().cloned().expect("a swatch group");
        (g.name, g.swatches.iter().filter_map(|w| w.paint.color()).collect())
    }

    /// With cells selected, Save Color Group to Swatches saves exactly those cells as one group, in
    /// grid order and each cell once, in one undo step that Undo removes.
    #[test]
    fn save_color_group_saves_the_selected_cells_in_grid_order() {
        let mut app = app();
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let st = app.session.doc().unwrap();
        let (groups, swatches, undo) = (st.doc.swatch_groups.len(), st.doc.swatches.len(), st.history.undo.len());
        let cells = &[(0, 0), (0, 8), (1, 4)].to_vec();
        save_group(&mut app, cells);
        let g = guide_of(&app);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.swatch_groups.len(), groups + 1, "one new group");
        assert_eq!(st.doc.swatches.len(), swatches, "no loose swatches");
        assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
        let (name, made) = last_group(&app);
        assert_eq!(name, harmony::label(app.ui.color_guide_panel.rule));
        assert_eq!(made, [g.grid[0][0], g.grid[0][8], g.grid[1][4]]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.swatch_groups.len(), groups, "Undo removes the group");
    }

    /// With no cell selected, the group holds the harmony colors.
    #[test]
    fn save_color_group_without_a_selection_saves_the_harmony_colors() {
        let mut app = app();
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let g = guide_of(&app);
        save_group(&mut app, &[]);
        assert_eq!(last_group(&app).1, g.colors);
    }

    /// A guide limited to a library saves its snapped cells, and an RGB cell stays RGB in a CMYK
    /// document.
    #[test]
    fn save_color_group_saves_limited_cells_in_their_own_model() {
        let mut app = app();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("paint.setFill", json!({"color": "#c04020", "keepModel": true})).unwrap();
        app.run("ui.colorGuideLimit", json!({"library": "web-safe-216"})).unwrap();
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let g = guide(&mut app);
        let cells = [(0, 1), (0, 7)];
        save_group(&mut app, &cells);
        let made = last_group(&app).1;
        assert_eq!(made, [g.grid[0][1], g.grid[0][7]]);
        assert!(made.iter().all(|c| matches!(c, Color::Rgb { .. })), "{made:?}");
        let members = library_hexes(&app, "web-safe-216");
        assert!(made.iter().all(|c| members.contains(&c.to_hex())), "the snapped colors");
    }

    /// A document with a selected rectangle and the spot color swatch "Ink", made the guide's base
    /// as clicking it in the Swatches panel makes it, at `tint` (0..1) of the spot color.
    fn spot_app(tint: f32) -> VectorcraftApp {
        let mut app = app();
        app.run("swatch.new", json!({"name": "Ink", "color": "#0066cc", "spot": true})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("paint.setFill", json!({"color": "#808080"})).unwrap();
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        choose_linked_base(&mut app, ink.tinted(tint), Some(("Ink".to_string(), tint)));
        app
    }

    /// The selected art's fill: (the swatch it links to, its tint 0..1, its color).
    fn fill_link(app: &VectorcraftApp) -> (Option<String>, f32, String) {
        match crate::panels::current_paints(app).0 {
            Paint::Solid { color, swatch, tint } => (swatch, tint, color.to_hex()),
            other => panic!("a solid fill: {other:?}"),
        }
    }

    /// The tint the cell `steps` right of the centre has with a base at `base` tint.
    fn expected_tint(app: &VectorcraftApp, base: f32, steps: i32) -> f32 {
        base * (1.0 - harmony::step_fraction(&app.ui.color_guide.clamped(), steps))
    }

    /// With a spot color base, a cell right of the centre in the base row of Tints/Shades paints the
    /// art with a tint of the spot color, which follows an edit of the spot color; the shades, the
    /// other rows, the other variation modes and a library limit give process colors.
    #[test]
    fn a_spot_color_base_gives_tints_of_that_spot_color() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let g = guide_of(&app);
        let c = g.centre();
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        assert_eq!(g.grid[0][c].to_hex(), ink.to_hex(), "the spot color in the centre");
        click(&mut app, grid(0, c + 2));
        let (swatch, tint, hex) = fill_link(&app);
        assert_eq!(swatch.as_deref(), Some("Ink"), "a tint of the spot color");
        let want = expected_tint(&app, 1.0, 2);
        assert!((tint - want).abs() < 1e-4, "{tint} vs {want}");
        assert_eq!(hex, ink.tinted(tint).to_hex(), "the color the cell shows");
        // The painted tint follows an edit of the spot color, and so does the base.
        app.run("swatch.edit", json!({"name": "Ink", "color": "#cc0000"})).unwrap();
        let red = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        assert_eq!(fill_link(&app).2, red.tinted(tint).to_hex(), "the art follows the spot color");
        assert_eq!(base_color(&app), red, "the base follows the spot color");
        frame(&mut app, &ctx);
        // Shades, another harmony row and the harmony strip are process colors.
        for at in [grid(0, c - 1), grid(1, c + 1), At::Strip(0)] {
            click(&mut app, at);
            assert_eq!(fill_link(&app).0, None, "{at:?}");
        }
        // Warm/Cool and a library limit: process colors.
        app.ui.color_guide.variation = Variation::WarmCool;
        frame(&mut app, &ctx);
        click(&mut app, grid(0, c + 1));
        assert_eq!(fill_link(&app).0, None, "Warm/Cool");
        app.ui.color_guide.variation = Variation::TintsShades;
        app.run("ui.colorGuideLimit", json!({"library": "web-safe-216"})).unwrap();
        frame(&mut app, &ctx);
        click(&mut app, grid(0, c + 1));
        assert_eq!(fill_link(&app).0, None, "limited to a library");
    }

    /// A base that is already a tint of the spot color gives lighter tints of it; a plain color with
    /// the spot color's values, a deleted swatch and a swatch that is no longer a spot color give
    /// process colors.
    #[test]
    fn a_tinted_spot_base_gives_lighter_tints_and_a_lost_link_gives_process_colors() {
        let mut app = spot_app(0.5);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let c = guide_of(&app).centre();
        let cell = |app: &VectorcraftApp, _: &egui::Context, col| cell_paint(app, &guide_of(app), 0, col).unwrap();
        let Paint::Solid { swatch, tint, .. } = cell(&app, &ctx, c + 1) else { panic!("a solid") };
        assert_eq!(swatch.as_deref(), Some("Ink"));
        let want = expected_tint(&app, 0.5, 1);
        assert!((tint - want).abs() < 1e-4 && tint < 0.5, "{tint} vs {want}");
        // The same values without the link.
        choose_base(&mut app, Color::from_hex("#0066cc").unwrap());
        assert_eq!(cell(&app, &ctx, c + 1), Paint::solid(guide_of(&app).grid[0][c + 1]));
        // The spot color made a process color, then deleted.
        let link = Some(("Ink".to_string(), 1.0));
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        choose_linked_base(&mut app, ink, link.clone());
        assert!(matches!(cell(&app, &ctx, c + 1), Paint::Solid { swatch: Some(_), .. }));
        app.run("swatch.edit", json!({"name": "Ink", "spot": false, "global": false})).unwrap();
        assert!(matches!(cell(&app, &ctx, c + 1), Paint::Solid { swatch: None, .. }), "no longer a spot color");
        choose_linked_base(&mut app, ink, link.clone());
        assert!(app.ui.color_guide_spot.is_none(), "a process swatch gives no spot base");
        app.run("swatch.edit", json!({"name": "Ink", "spot": true})).unwrap();
        choose_linked_base(&mut app, ink, link);
        app.run("swatch.delete", json!({"name": "Ink"})).unwrap();
        assert!(matches!(cell(&app, &ctx, c + 1), Paint::Solid { swatch: None, .. }), "a deleted swatch");
    }

    /// A spot color base stays linked to its swatch when Swatch Options renames it: the tints link to
    /// the new name and follow an edit of it.
    #[test]
    fn a_renamed_spot_color_stays_the_base() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let c = guide_of(&app).centre();
        app.run("swatch.edit", json!({"name": "Ink", "newName": "Deep Ink"})).unwrap();
        let Paint::Solid { swatch, .. } = cell_paint(&app, &guide_of(&app), 0, c + 1).unwrap() else { panic!("a solid") };
        assert_eq!(swatch.as_deref(), Some("Deep Ink"));
        app.run("swatch.edit", json!({"name": "Deep Ink", "color": "#cc0000"})).unwrap();
        let red = app.session.active().unwrap().doc.global_color("Deep Ink").unwrap();
        assert_eq!(base_color(&app), red, "the base follows the renamed spot color");
        // Renames in a batch, in step order.
        let steps = json!([
            {"command": "swatch.edit", "params": {"name": "Deep Ink", "newName": "Ink A"}},
            {"command": "swatch.edit", "params": {"name": "Ink A", "newName": "Ink B"}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        let Paint::Solid { swatch, .. } = cell_paint(&app, &guide_of(&app), 0, c + 1).unwrap() else { panic!("a solid") };
        assert_eq!(swatch.as_deref(), Some("Ink B"), "renamed in a batch");
    }

    /// Deleting the spot color, or making it a process color, ends the base's link: a new spot color
    /// of the same name, or the swatch made a spot color again, gives process colors until it is
    /// chosen again.
    #[test]
    fn a_deleted_spot_color_ends_the_link() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let c = guide_of(&app).centre();
        let linked = |app: &VectorcraftApp, _: &egui::Context| {
            matches!(cell_paint(app, &guide_of(app), 0, c + 1).unwrap(), Paint::Solid { swatch: Some(_), .. })
        };
        assert!(linked(&app, &ctx));
        app.run("swatch.delete", json!({"name": "Ink"})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": "#00cc66", "spot": true})).unwrap();
        assert!(!linked(&app, &ctx), "a new spot color of the same name");
        assert!(app.ui.color_guide_spot.is_none());
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        choose_linked_base(&mut app, ink, Some(("Ink".to_string(), 1.0)));
        assert!(linked(&app, &ctx));
        app.run("swatch.edit", json!({"name": "Ink", "spot": false, "global": false})).unwrap();
        app.run("swatch.edit", json!({"name": "Ink", "spot": true})).unwrap();
        assert!(!linked(&app, &ctx), "a spot color again");
        // The same in a batch: a step that deletes the swatch, or one that makes it a process color.
        let relink = |app: &mut VectorcraftApp| {
            let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
            choose_linked_base(app, ink, Some(("Ink".to_string(), 1.0)));
        };
        for steps in [
            json!([
                {"command": "swatch.edit", "params": {"name": "Ink", "newName": "Ink 2"}},
                {"command": "swatch.delete", "params": {"name": "Ink 2"}},
                {"command": "swatch.new", "params": {"name": "Ink 2", "color": "#cc6600", "spot": true}},
                {"command": "swatch.edit", "params": {"name": "Ink 2", "newName": "Ink"}},
            ]),
            json!([
                {"command": "swatch.edit", "params": {"name": "Ink", "spot": false, "global": false}},
                {"command": "swatch.edit", "params": {"name": "Ink", "spot": true}},
            ]),
        ] {
            relink(&mut app);
            assert!(linked(&app, &ctx));
            app.run("command.batch", json!({"commands": steps})).unwrap();
            assert!(!linked(&app, &ctx), "{steps}");
        }
        // A batch that edits the spot color's values and deletes another swatch keeps the link.
        relink(&mut app);
        app.run("swatch.new", json!({"name": "Spare", "color": "#999999"})).unwrap();
        let steps = json!([
            {"command": "swatch.edit", "params": {"name": "Ink", "color": "#cc0000"}},
            {"command": "swatch.delete", "params": {"name": "Spare"}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(linked(&app, &ctx), "an unrelated batch");
        // `global: false` leaves a spot color global, so the link stays.
        let steps = json!([{"command": "swatch.edit", "params": {"name": "Ink", "global": false}}, {"command": "swatch.sortByName"}]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(linked(&app, &ctx), "global: false on a spot color");
        // Closing its document ends the link, and the base keeps the color it showed.
        app.run("swatch.edit", json!({"name": "Ink", "color": "#00aa00"})).unwrap();
        let shown = base_color(&app);
        // Closed the way the tab's close button closes it, outside `app.run`.
        app.session.execute("file.new", &json!({})).unwrap();
        app.session.execute("file.close", &json!({"index": 0})).unwrap();
        frame(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1);
        assert!(app.ui.color_guide_spot.is_none(), "its document closed");
        assert_eq!(base_color(&app), shown, "the color it showed before the close");
    }

    /// An Undo or a Redo of a rename keeps the spot color base's link under the name the swatch has
    /// again, also after more renames than a short list would hold and after a rename the panel did
    /// not follow (a script's); another command after such a rename ends the link.
    #[test]
    fn an_undone_rename_keeps_the_link() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let c = guide_of(&app).centre();
        let linked = |app: &VectorcraftApp, _: &egui::Context| {
            matches!(cell_paint(app, &guide_of(app), 0, c + 1).unwrap(), Paint::Solid { swatch: Some(_), .. })
        };
        let spot_name = |app: &VectorcraftApp| app.ui.color_guide_spot.as_ref().map(|s| s.swatch.clone());
        app.run("swatch.edit", json!({"name": "Ink", "newName": "Ink U"})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        assert!(linked(&app, &ctx), "a rename undone");
        assert_eq!(spot_name(&app).as_deref(), Some("Ink"));
        app.run("edit.redo", json!({})).unwrap();
        assert!(linked(&app, &ctx), "a rename redone");
        assert_eq!(spot_name(&app).as_deref(), Some("Ink U"));
        app.run("edit.undo", json!({})).unwrap();
        // Forty renames undone one by one.
        for i in 0..40 {
            let from = if i == 0 { "Ink".to_string() } else { format!("Ink {i}") };
            app.run("swatch.edit", json!({"name": from, "newName": format!("Ink {}", i + 1)})).unwrap();
        }
        for _ in 0..40 {
            app.run("edit.undo", json!({})).unwrap();
        }
        assert_eq!(spot_name(&app).as_deref(), Some("Ink"), "forty renames undone");
        assert!(linked(&app, &ctx));
        // A rename the panel did not follow: the link rests, and the Undo brings it back.
        app.session.execute("swatch.edit", &json!({"name": "Ink", "newName": "Ink S"})).unwrap();
        frame(&mut app, &ctx);
        assert!(!linked(&app, &ctx), "resting");
        app.run("edit.undo", json!({})).unwrap();
        assert!(linked(&app, &ctx), "an unfollowed rename undone");
        assert_eq!(spot_name(&app).as_deref(), Some("Ink"));
        // Another command while it rests ends it.
        app.session.execute("swatch.edit", &json!({"name": "Ink", "newName": "Ink S"})).unwrap();
        frame(&mut app, &ctx);
        app.run("swatch.new", json!({"name": "Spare", "color": "#999999"})).unwrap();
        assert!(app.ui.color_guide_spot.is_none(), "another command while it rests");
        // Outside an Undo or Redo, a new spot color with an earlier name does not take its place.
        app.run("swatch.edit", json!({"name": "Ink S", "newName": "Ink"})).unwrap();
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        choose_linked_base(&mut app, ink, Some(("Ink".to_string(), 1.0)));
        app.run("swatch.edit", json!({"name": "Ink", "newName": "Ink U"})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": "#00cc66", "spot": true})).unwrap();
        app.session.execute("swatch.edit", &json!({"name": "Ink U", "newName": "Ink V"})).unwrap();
        frame(&mut app, &ctx);
        assert!(!linked(&app, &ctx), "a new spot color with an earlier name");
        assert_eq!(spot_name(&app).as_deref(), Some("Ink U"));
    }

    /// While the link rests, a new spot color of the same name that the panel did not follow does
    /// not give the base its color when the next command ends the link.
    #[test]
    fn a_resting_link_keeps_the_color_it_showed() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let shown = base_color(&app);
        app.session.execute("swatch.delete", &json!({"name": "Ink"})).unwrap();
        frame(&mut app, &ctx);
        app.session.execute("swatch.new", &json!({"name": "Ink", "color": "#00cc66", "spot": true})).unwrap();
        frame(&mut app, &ctx);
        assert!(app.ui.color_guide_spot.as_ref().is_some_and(|s| s.resting), "resting");
        assert_eq!(base_color(&app), shown, "a resting link shows the color it showed");
        app.run("swatch.new", json!({"name": "Spare", "color": "#999999"})).unwrap();
        assert!(app.ui.color_guide_spot.is_none(), "another command ends it");
        assert_eq!(base_color(&app), shown, "the base keeps the color it showed");
    }

    /// A script run ends the link: a script that deletes the spot color and makes another of the
    /// same name does not make the new one the base, and the base keeps the color it showed. Listing
    /// scripts or plug-ins keeps it.
    #[test]
    fn a_script_run_ends_the_link() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        // Commands that only list scripts or plug-ins keep the link, also as batch steps.
        app.run("script.engines", json!({})).unwrap();
        app.run("plugin.list", json!({})).unwrap();
        app.run("command.batch", json!({"commands": [{"command": "script.engines"}]})).unwrap();
        assert!(app.ui.color_guide_spot.is_some(), "a script or plug-in listing");
        let shown = base_color(&app);
        let source = "vc.run('swatch.delete', {name: 'Ink'}); vc.run('swatch.new', {name: 'Ink', color: '#00cc66', spot: true});";
        app.run("script.run", json!({"source": source, "name": "t.js"})).unwrap();
        assert!(app.ui.color_guide_spot.is_none(), "a script ran");
        assert_eq!(base_color(&app), shown, "the base keeps the color it showed");
    }

    /// A `command.batch` that switches, opens, closes or moves documents follows the spot color
    /// base's swatch in its own document and leaves the link alone for steps in the others.
    #[test]
    fn a_batch_follows_the_spot_color_across_documents() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let c = guide_of(&app).centre();
        let linked = |app: &VectorcraftApp, _: &egui::Context| {
            matches!(cell_paint(app, &guide_of(app), 0, c + 1).unwrap(), Paint::Solid { swatch: Some(_), .. })
        };
        let relink = |app: &mut VectorcraftApp| {
            let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
            choose_linked_base(app, ink, Some(("Ink".to_string(), 1.0)));
        };
        assert!(linked(&app, &ctx));
        // Undo, Activate Document and New in a batch keep the link while the swatch is a spot color,
        // and the steps in the base's document are still followed.
        app.run("swatch.new", json!({"name": "Spare 2", "color": "#777777"})).unwrap();
        let steps = json!([
            {"command": "edit.undo"},
            {"command": "document.activate", "params": {"index": 0}},
            {"command": "swatch.sortByName"},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(linked(&app, &ctx), "an undo, an activate and a sort");
        let steps = json!([
            {"command": "file.new"},
            {"command": "swatch.new", "params": {"name": "Other", "color": "#123456", "spot": true}},
            {"command": "swatch.delete", "params": {"name": "Other"}},
            {"command": "document.activate", "params": {"index": 0}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(linked(&app, &ctx), "steps in a new document");
        let spot_name = |app: &VectorcraftApp| app.ui.color_guide_spot.as_ref().map(|s| s.swatch.clone());
        let steps = json!([
            {"command": "file.new"},
            {"command": "document.activate", "params": {"index": 0}},
            {"command": "swatch.edit", "params": {"name": "Ink", "newName": "Ink C"}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(linked(&app, &ctx), "renamed after a new document");
        assert_eq!(spot_name(&app).as_deref(), Some("Ink C"));
        // Moves and closes shift the tabs: [base, 1, 2] becomes [1, 2, base], then [2, base], then
        // [base, 2]; the steps in tab 2 leave the link alone.
        let steps = json!([
            {"command": "document.move", "params": {"from": 0, "to": 2}},
            {"command": "document.activate", "params": {"index": 2}},
            {"command": "swatch.edit", "params": {"name": "Ink C", "newName": "Ink M"}},
            {"command": "file.close", "params": {"index": 0}},
            {"command": "document.activate", "params": {"index": 0}},
            {"command": "swatch.new", "params": {"name": "Ink M", "color": "#123456", "spot": true}},
            {"command": "swatch.delete", "params": {"name": "Ink M"}},
            {"command": "document.activate", "params": {"index": 1}},
            {"command": "document.move", "params": {"from": 1, "to": 0}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert_eq!(app.session.active_index(), Some(0));
        assert!(linked(&app, &ctx), "moved and closed tabs");
        assert_eq!(spot_name(&app).as_deref(), Some("Ink M"));
        app.run("swatch.edit", json!({"name": "Ink M", "newName": "Ink"})).unwrap();
        assert!(linked(&app, &ctx), "renamed back");
        // A batch that starts in another document follows the base's color group too.
        app.run("swatch.newGroup", json!({"name": "Held", "swatches": ["Ink"]})).unwrap();
        app.run("document.activate", json!({"index": 1})).unwrap();
        let steps = json!([
            {"command": "document.activate", "params": {"index": 0}},
            {"command": "swatch.delete", "params": {"names": ["Held"]}},
            {"command": "swatch.new", "params": {"name": "Ink", "color": "#cc6600", "spot": true}},
            {"command": "document.activate", "params": {"index": 1}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert!(!linked(&app, &ctx), "its group deleted in a batch that started elsewhere");
        relink(&mut app);
        // Where an opened document's steps run is not known: deleting a swatch of the same name
        // there keeps the link.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
        let steps = json!([
            {"command": "document.open", "params": {"name": "a.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())}},
            {"command": "swatch.new", "params": {"name": "Ink", "color": "#123456", "spot": true}},
            {"command": "swatch.delete", "params": {"name": "Ink"}},
            {"command": "document.activate", "params": {"index": 0}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(linked(&app, &ctx), "deleted in an opened document");
        // The opened document's tab is known from its reply: a rename there is not the base's,
        // even when the base's document has spot colors of both names.
        app.run("swatch.new", json!({"name": "Ink Alt", "color": "#654321", "spot": true})).unwrap();
        let steps = json!([
            {"command": "document.open", "params": {"name": "b.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())}},
            {"command": "swatch.new", "params": {"name": "Ink", "color": "#123456", "spot": true}},
            {"command": "swatch.edit", "params": {"name": "Ink", "newName": "Ink Alt"}},
            {"command": "document.activate", "params": {"index": 0}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(linked(&app, &ctx), "renamed in an opened document");
        assert_eq!(spot_name(&app).as_deref(), Some("Ink"));
        // Deleted by a batch that starts in another tab: the base keeps the color it showed.
        app.run("swatch.edit", json!({"name": "Ink", "color": "#00aa00"})).unwrap();
        let shown = base_color(&app);
        app.run("document.activate", json!({"index": 1})).unwrap();
        let steps = json!([
            {"command": "document.activate", "params": {"index": 0}},
            {"command": "swatch.delete", "params": {"name": "Ink"}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(app.ui.color_guide_spot.is_none());
        assert_eq!(base_color(&app), shown, "the edited color");
        app.run("swatch.new", json!({"name": "Ink", "color": "#cc0000", "spot": true})).unwrap();
        relink(&mut app);
        let steps = json!([
            {"command": "document.activate", "params": {"index": 1}},
            {"command": "swatch.new", "params": {"name": "Ink", "color": "#123456", "spot": true}},
            {"command": "swatch.delete", "params": {"name": "Ink"}},
            {"command": "document.activate", "params": {"index": 0}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(linked(&app, &ctx), "a swatch of the same name deleted in another document");
        let steps = json!([
            {"command": "document.activate", "params": {"index": 0}},
            {"command": "swatch.delete", "params": {"name": "Ink"}},
            {"command": "swatch.new", "params": {"name": "Ink", "color": "#123456", "spot": true}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(!linked(&app, &ctx), "deleted after an activate");
        relink(&mut app);
        assert!(linked(&app, &ctx));
    }

    /// Deleting the color group that holds the spot color base's swatch ends the link, also under
    /// a name Color Group Options gave it; an Edit Color Group that keeps it does not.
    #[test]
    fn a_deleted_color_group_ends_the_link() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let c = guide_of(&app).centre();
        let linked = |app: &VectorcraftApp, _: &egui::Context| {
            matches!(cell_paint(app, &guide_of(app), 0, c + 1).unwrap(), Paint::Solid { swatch: Some(_), .. })
        };
        let relink = |app: &mut VectorcraftApp| {
            let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
            choose_linked_base(app, ink, Some(("Ink".to_string(), 1.0)));
        };
        // Deleting the color group that holds it, then a new spot color of the same name.
        app.run("swatch.newGroup", json!({"name": "Inks", "swatches": ["Ink"]})).unwrap();
        assert!(linked(&app, &ctx), "moved into a group");
        let steps = json!([
            {"command": "swatch.delete", "params": {"names": ["Inks"]}},
            {"command": "swatch.new", "params": {"name": "Ink", "color": "#cc6600", "spot": true}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(!linked(&app, &ctx), "its group deleted");
        // Moved into a group and the group deleted in the same batch.
        relink(&mut app);
        let steps = json!([
            {"command": "swatch.newGroup", "params": {"name": "Spots", "swatches": ["Ink"]}},
            {"command": "swatch.delete", "params": {"names": ["Spots"]}},
            {"command": "swatch.new", "params": {"name": "Ink", "color": "#336699", "spot": true}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(!linked(&app, &ctx), "moved into a group that was deleted");
        // Its group renamed by Color Group Options and deleted under the new name in the same batch.
        relink(&mut app);
        let steps = json!([
            {"command": "swatch.newGroup", "params": {"name": "Spots", "swatches": ["Ink"]}},
            {"command": "swatch.renameGroup", "params": {"group": "Spots", "name": "Spot Inks"}},
            {"command": "swatch.delete", "params": {"names": ["Spot Inks"]}},
            {"command": "swatch.new", "params": {"name": "Ink", "color": "#669933", "spot": true}},
        ]);
        app.run("command.batch", json!({"commands": steps})).unwrap();
        assert!(!linked(&app, &ctx), "its group renamed, then deleted");
        // Edit Color Group that keeps it, with a new group name, keeps the link.
        relink(&mut app);
        app.run("swatch.newGroup", json!({"name": "Spots", "swatches": ["Ink"]})).unwrap();
        let ink = app.session.active().unwrap().doc.global_color("Ink").unwrap();
        app.run("swatch.editGroup", json!({"group": "Spots", "colors": [ink.to_hex()], "rename": "Spot Inks"})).unwrap();
        assert!(linked(&app, &ctx), "a group edit that keeps it");
        // A deleted base keeps the color it showed after an edit.
        app.run("swatch.edit", json!({"name": "Ink", "color": "#cc0000"})).unwrap();
        let shown = base_color(&app);
        app.run("swatch.delete", json!({"name": "Ink"})).unwrap();
        assert_eq!(base_color(&app), shown, "the color it showed");
    }

    /// Save Colors as Swatches and Save Color Group save a spot color base's tints as tint swatches
    /// linked to it, beside process colors for the other cells, each in one undo step.
    #[test]
    fn saved_spot_tints_are_tint_swatches_of_the_spot_color() {
        let mut app = spot_app(1.0);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx);
        let g = guide_of(&app);
        let c = g.centre();
        let undo = app.session.doc().unwrap().history.undo.len();
        let cells = &[(0, c + 1), (0, c - 1), (0, c + 3)].to_vec();
        save_selected(&mut app, cells);
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
        let made: Vec<_> = st.doc.swatches[st.doc.swatches.len() - 3..].to_vec();
        let tint = |w: &vectorcraft_color::Swatch| w.tint_of().map(|(b, t)| (b.to_string(), t));
        assert_eq!(tint(&made[0]).map(|(b, _)| b).as_deref(), Some("Ink"));
        assert!((tint(&made[0]).unwrap().1 - expected_tint(&app, 1.0, 1)).abs() < 1e-4);
        assert_eq!(made[1].paint, Paint::solid(g.grid[0][c - 1]), "a shade is a process color");
        assert_eq!(tint(&made[2]).map(|(b, _)| b).as_deref(), Some("Ink"));
        assert!(made[0].name.starts_with("Ink ") && made[0].name.ends_with('%'), "{}", made[0].name);
        // A group of the selected cells, in grid order.
        let cells = &[(0, c + 2), (1, c + 1)].to_vec();
        save_group(&mut app, cells);
        let group = app.session.doc().unwrap().doc.swatch_groups.last().cloned().unwrap();
        assert_eq!(group.swatches.len(), 2);
        assert_eq!(tint(&group.swatches[0]).map(|(b, _)| b).as_deref(), Some("Ink"), "the base row's tint");
        assert_eq!(group.swatches[1].paint, Paint::solid(g.grid[1][c + 1]), "another row is a process color");
        // The saved tints follow an edit of the spot color.
        app.run("swatch.edit", json!({"name": "Ink", "color": "#cc0000"})).unwrap();
        let d = &app.session.active().unwrap().doc;
        let red = d.global_color("Ink").unwrap();
        let first = d.swatch(&made[0].name).unwrap();
        assert_eq!(first.paint.color(), Some(red.tinted(first.tint_of().unwrap().1)));
    }

    /// A spot color base's tint dragged from the guide carries the tint (not its color), and a
    /// swatch made from that paint is a tint swatch.
    #[test]
    fn a_dragged_spot_tint_becomes_a_tint_swatch() {
        let mut app = spot_app(1.0);
        let c = guide_of(&app).centre();
        let tints = spot_paints(&app, app.ui.color_guide_panel.rule);
        let (spot, color) = cell_at(&guide_of(&app), grid(0, c + 1));
        let PanelDrag::Paint { paint, .. } = drag_payload(&tints, spot, color) else { panic!("a paint drag") };
        let Paint::Solid { swatch, tint, .. } = &paint else { panic!("a solid paint: {paint:?}") };
        assert_eq!(swatch.as_deref(), Some("Ink"));
        assert!((tint - expected_tint(&app, 1.0, 1)).abs() < 1e-4);
        // A cell outside the tint row carries its color alone.
        let (spot, color) = cell_at(&guide_of(&app), grid(0, c - 1));
        assert!(matches!(drag_payload(&tints, spot, color), PanelDrag::Paint { paint: Paint::Solid { swatch: None, .. }, .. }));
        app.run("swatch.new", paint_params(&paint)).unwrap();
        let newest = app.session.doc().unwrap().doc.swatches.last().cloned().expect("a swatch");
        let (base, made) = newest.tint_of().unwrap_or_else(|| panic!("a tint swatch: {newest:?} {}", app.ui.status));
        assert_eq!(base, "Ink");
        assert!((made - tint).abs() < 1e-4, "{made} vs {tint}");
    }

    /// Every colour of the panel's guide, limited as the actions read it.
    fn shown(app: &mut VectorcraftApp) -> Vec<String> {
        let g = guide(app);
        g.grid.iter().flatten().chain(&g.colors).map(Color::to_hex).collect()
    }

    fn library_hexes(app: &VectorcraftApp, id: &str) -> Vec<String> {
        let (_, lib) = swatchlib::library(&app.session, id).unwrap();
        lib.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect()
    }

    #[test]
    fn limit_to_library_snaps_the_panel_and_follows_the_document_swatches() {
        let mut app = app();
        let ctx = crate::testing::ui_context();
        app.run("paint.setFill", json!({"color": "#3a7bd5"})).unwrap();
        frame(&mut app, &ctx);
        assert!(shown(&mut app).contains(&"#3a7bd5".to_string()), "no limit: the base itself");
        let r = app.run("ui.colorGuideLimit", json!({"library": "Earth Tones"})).unwrap();
        assert_eq!((r["limitTo"].as_str(), r["name"].as_str()), (Some("earth-tones"), Some("Earth Tones")));
        frame(&mut app, &ctx);
        let members = library_hexes(&app, "earth-tones");
        assert!(shown(&mut app).iter().all(|h| members.contains(h)), "every cell is a library colour");
        // The document's swatches: the kept palette follows a new swatch.
        app.run("ui.colorGuideLimit", json!({"library": "document"})).unwrap();
        frame(&mut app, &ctx);
        assert!(!shown(&mut app).contains(&"#3a7bd4".to_string()));
        app.run("swatch.new", json!({"colors": ["#3a7bd4"]})).unwrap();
        frame(&mut app, &ctx);
        let doc: Vec<String> = app.session.doc().unwrap().doc.swatches_iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect();
        let cells = shown(&mut app);
        assert!(cells.iter().all(|h| doc.contains(h)));
        assert_eq!(guide(&mut app).colors[0].to_hex(), "#3a7bd4", "the base snaps to the new swatch");
        // The popup lists libraries with colours only; unknown libraries are refused and "" lifts
        // the limit.
        let libs = colour_libraries(&app);
        assert!(libs.iter().any(|l| l.id == "earth-tones") && !libs.iter().any(|l| l.id == "metallic-gradients"));
        assert!(app.run("ui.colorGuideLimit", json!({"library": "nope"})).is_err());
        assert_eq!(app.ui.color_guide_limit, "document");
        assert_eq!(app.run("ui.colorGuideLimit", json!({"library": ""})).unwrap()["name"], Value::Null);
        assert!(shown(&mut app).contains(&"#3a7bd5".to_string()));
    }

    #[test]
    fn edit_or_apply_without_art_saves_the_limited_colours_as_a_group() {
        let mut app = app();
        let ctx = app.ui_ctx.clone().expect("a window");
        app.run("paint.setFill", json!({"color": "#c04020"})).unwrap();
        app.run("select.none", json!({})).unwrap();
        app.run("ui.colorGuideLimit", json!({"library": "web-safe-216"})).unwrap();
        frame(&mut app, &ctx);
        edit_or_apply(&mut app);
        // PLAN: the app under test needs a window host for `dialogs()` (hub-w2's fixture), and the
        // result pump that saves the group on OK is hub-w2's: this trial stops at the confirmed dialog.
        let dialogs = app.dialogs().expect("the app has a window host");
        let open = dialogs.with(|host| host.open_dialogs());
        let [recolor] = open.as_slice() else { panic!("Recolor opens on the colours alone: {open:?}") };
        let fields = recolor.fields.clone().expect("Recolor lists its fields");
        assert_eq!(fields["limitTo"], "web-safe-216", "limited to the same library");
        let members = library_hexes(&app, "web-safe-216");
        let rows = fields["rows"].as_array().expect("a row per harmony color");
        let g = guide(&mut app);
        assert_eq!(rows.len(), g.colors.len());
        assert!(g.colors.iter().all(|c| members.contains(&c.to_hex())), "the colors are the library's");
        let groups = |app: &VectorcraftApp| app.session.active().map_or(0, |st| st.doc.swatch_groups.len());
        let before = groups(&app);
        crate::tests::picks::answer_dialog(&mut app, recolor.id, &[], true);
        assert!(dialogs.with(|host| host.open_dialogs().is_empty()), "OK confirms");
        assert_eq!(groups(&app), before + 1, "OK saves the colours as a group");
    }
}
