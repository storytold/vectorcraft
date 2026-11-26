use std::sync::{Arc, Weak};

use super::{Document, Paint, PanelDrag, REGISTRATION, Swatch, Value, VectorcraftApp, json, name_of, swatch_params};
use crate::panel_drag::SwatchRows;
use craft::color_ui::ui::SwatchKey;
use craft::keymap::Mods;
use craft::ui::contract::{DropAt, ReorderMove};
use vectorcraft_color::SwatchGroup;
use vectorcraft_engine::{DocState, History, HistoryEntry};

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
    /// What releasing `d` before or `after` row `target` (`None`: at the end) with `mods` held does:
    /// move the rows it carries (Alt: a swatch grabbed may replace `target`), make a swatch of a
    /// paint dragged from elsewhere (Cmd/Ctrl: a spot color), or a pattern swatch of art dragged off
    /// the canvas (appearances: nothing).
    fn of(d: &PanelDrag, target: Option<String>, after: bool, mods: Mods) -> Option<Self> {
        match d {
            PanelDrag::Paint { rows: Some(r), .. } => {
                let with = (mods.alt && !r.groups).then(|| r.grabbed.clone());
                Some(Drop::Move { names: r.names.clone(), target, after, with })
            }
            PanelDrag::Paint { paint, .. } => Some(Drop::New { paint: paint.clone(), target, replace: mods.alt, spot: mods.cmd }),
            // PLAN: assumed `Art { document: u64, ids: Vec<NodeId> }` (widgets-w13).
            PanelDrag::Art { ids, .. } => Some(Drop::Pattern { ids: ids.iter().map(|id| id.0).collect(), target, replace: mods.alt }),
            _ => None,
        }
    }
}

/// The drag a row of `key` starts in document `start`, moving rows `names`. Colour groups have no
/// paint of their own (the chip under the pointer draws Registration's mark for Registration).
pub(super) fn drag_of(start: &DragStart, key: &SwatchKey, names: Vec<String>) -> PanelDrag {
    let d = &start.doc;
    let name = name_of(key);
    let (paint, params) = match key {
        SwatchKey::Swatch(_) if name == REGISTRATION => (Paint::registration(), swatch_params(d, name)),
        SwatchKey::Swatch(_) => match d.swatch(name) {
            Some(w) => (w.paint.clone(), swatch_params(d, name)),
            None => (Paint::None, Value::Null),
        },
        _ => (Paint::None, Value::Null),
    };
    let held = Arc::new(start.rows(&names));
    let rows = SwatchRows { grabbed: name.to_string(), names, groups: matches!(key, SwatchKey::Group(_)), held };
    PanelDrag::Paint { paint, params, rows: Some(rows), library: None }
}

/// A row dragged from the list as it was when the drag started: its swatch or its colour group.
#[derive(Clone, PartialEq)]
enum HeldRow {
    Swatch(Swatch),
    Group(SwatchGroup),
}

impl HeldRow {
    /// Row `name` of `d`: its swatch, else the colour group of that name.
    fn of(d: &Document, name: &str) -> Option<Self> {
        match d.swatch(name) {
            Some(w) => Some(HeldRow::Swatch(w.clone())),
            None => d.swatch_groups.iter().find(|g| g.name == name).cloned().map(HeldRow::Group),
        }
    }
    /// Whether `d` holds this row unchanged.
    fn unchanged_in(&self, d: &Document) -> bool {
        match self {
            HeldRow::Swatch(w) => d.swatch(&w.name) == Some(w),
            HeldRow::Group(g) => d.swatch_groups.iter().any(|x| x == g),
        }
    }
}

/// The newest undo step of `st` and the one under it, and the next redo step and the one under it
/// ([`HistoryEntry::serial`]), 0 for none.
fn steps(st: &DocState) -> [u64; 4] {
    let serial = |list: &[HistoryEntry], back: usize| list.len().checked_sub(back).and_then(|i| list.get(i)).map_or(0, HistoryEntry::serial);
    let h = &st.history;
    [serial(&h.undo, 1), serial(&h.undo, 2), serial(&h.redo, 1), serial(&h.redo, 2)]
}

/// A document as a drag from the list finds it: its `uid` and its count of File › Revert
/// ([`DocState::reverts`]), its state, its newest steps ([`steps`]) and the state undoing the newest
/// undo step gives back (that step's document).
#[derive(Clone)]
pub(super) struct DragStart {
    uid: u64,
    reverts: u64,
    doc: Arc<Document>,
    steps: [u64; 4],
    back: Option<Arc<Document>>,
}

impl DragStart {
    pub(super) fn of(st: &DocState) -> Self {
        let back = st.history.undo.last().map(|e| Arc::clone(&e.doc));
        DragStart { uid: st.uid, reverts: st.reverts, doc: Arc::clone(&st.doc), steps: steps(st), back }
    }

    /// Rows `names` as this document holds them.
    fn rows(&self, names: &[String]) -> HeldRows {
        let rows: Vec<_> = names.iter().map(|n| (n.clone(), HeldRow::of(&self.doc, n))).collect();
        let back = rows.iter().map(|(_, r)| r.as_ref().zip(self.back.as_deref()).is_some_and(|(r, d)| r.unchanged_in(d))).collect();
        HeldRows { uid: self.uid, reverts: self.reverts, doc: Arc::downgrade(&self.doc), steps: self.steps, rows, back }
    }
}

/// The rows a drag from the list carries, in the order of its names, and the document (`uid`, and
/// its count of File › Revert) and the state and undo history they were last checked against. A
/// row is `None` once a state of the document since the drag started did not hold it unchanged:
/// deleted, edited, or deleted and made again under its name, also with the same paint.
#[derive(Clone)]
pub(crate) struct HeldRows {
    uid: u64,
    reverts: u64,
    doc: Weak<Document>,
    /// The newest undo step of the document and the one under it, and the next redo step and the
    /// one under it ([`steps`]).
    steps: [u64; 4],
    rows: Vec<(String, Option<HeldRow>)>,
    /// For each row, whether the document undoing the newest step gives back (that step's
    /// document) holds it unchanged.
    back: Vec<bool>,
}

impl PartialEq for HeldRows {
    fn eq(&self, other: &Self) -> bool {
        self.uid == other.uid
            && self.reverts == other.reverts
            && self.doc.ptr_eq(&other.doc)
            && self.steps == other.steps
            && self.rows == other.rows
            && self.back == other.back
    }
}

impl std::fmt::Debug for HeldRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let held: Vec<(&str, bool)> = self.rows.iter().map(|(n, r)| (n.as_str(), r.is_some())).collect();
        f.debug_struct("HeldRows").field("uid", &self.uid).field("steps", &self.steps).field("held", &held).finish_non_exhaustive()
    }
}

/// `held` checked against every state document `st` passed through since it was last checked, and
/// the current one. Those states are the ones the redo steps made since then keep (the states an
/// undo left), and:
/// - While the newest undo step of then is still in the history, the states the undo steps made
///   since then keep.
/// - After an undo of that step, or an edit merged into it, the state before that step and the
///   states the undo steps made since then keep.
///
/// Otherwise (more than one step undone, or steps dropped past the history's limits) the states in
/// between are not known, and every row is left out of the drop. The panel checks the rows on every
/// pass and after every command the app runs ([`follow_drag`]), so only the steps of one command go
/// unseen. One undo step is one change: the states between the commands of a `command.batch` are
/// not kept, and a row the batch leaves as it found it stays in the drop.
fn checked(held: HeldRows, st: &DocState) -> HeldRows {
    let now = steps(st);
    let same = std::ptr::eq(held.doc.as_ptr(), Arc::as_ptr(&st.doc));
    if same && held.steps == now {
        return held;
    }
    let (undo, redo) = (&st.history.undo, &st.history.redo);
    // Where the steps made after the step with `serial` start in `list`: 0 for none.
    let after = |list: &[HistoryEntry], serial: u64| {
        if serial == 0 { Some(0) } else { list.iter().rposition(|e| e.serial() == serial).map(|i| i + 1) }
    };
    let [newest, below, next, further] = held.steps;
    // The redo steps made since: above the next redo step of then, or above the one under it once a
    // redo took that step.
    let left = redo.get(after(redo, next).or_else(|| after(redo, further)).unwrap_or(0)..).unwrap_or_default();
    // The states in between, and whether they start with the state an undo of the newest step gave back.
    let (between, undone) = match after(undo, newest) {
        // With no step before it, the first step keeps the last checked state, unless steps were
        // dropped past the history's limits, which leave at least that many steps.
        Some(0) => {
            let kept = same
                || undo.len() < st.history.limit.min(History::MIN_STEPS)
                || undo.first().is_none_or(|e| std::ptr::eq(held.doc.as_ptr(), Arc::as_ptr(&e.doc)));
            (kept.then_some(undo.as_slice()), false)
        }
        Some(i) => (undo.get(i..), false),
        None => (after(undo, below).and_then(|i| undo.get(i..)), true),
    };
    let unchanged = |r: &HeldRow, b: &[HistoryEntry]| b.iter().chain(left).all(|e| r.unchanged_in(&e.doc)) && r.unchanged_in(&st.doc);
    let rows: Vec<_> = held
        .rows
        .into_iter()
        .zip(held.back)
        .map(|((name, row), back)| (name, row.filter(|r| (back || !undone) && between.is_some_and(|b| unchanged(r, b)))))
        .collect();
    let last = undo.last();
    let back = rows.iter().map(|(_, r)| r.as_ref().zip(last).is_some_and(|(r, e)| r.unchanged_in(&e.doc))).collect();
    HeldRows { uid: held.uid, reverts: held.reverts, doc: Arc::downgrade(&st.doc), steps: now, rows, back }
}

/// The names of drag `r` that the active document has held unchanged since the drag started
/// ([`checked`]), in the drag's order. Empty when the drag started in another document, or before
/// File › Revert replaced the document. The panel keeps what it found for the drag, so the next
/// check starts from there.
pub(super) fn follow(app: &mut VectorcraftApp, r: &SwatchRows) -> Vec<String> {
    let Some(st) = app.session.active() else { return vec![] };
    if st.uid != r.held.uid || st.reverts != r.held.reverts {
        return vec![];
    }
    let kept = &mut app.ui.swatches_panel.drag;
    let from = match kept {
        Some((start, now)) if Arc::ptr_eq(start, &r.held) => now.clone(),
        _ => (*r.held).clone(),
    };
    let now = checked(from, st);
    let names = if now.rows.len() == r.names.len() {
        r.names
            .iter()
            .zip(&now.rows)
            .filter(|(n, (h, row))| *n == h && row.as_ref().is_some_and(|row| row.unchanged_in(&st.doc)))
            .map(|(n, _)| n.clone())
            .collect()
    } else {
        vec![]
    };
    *kept = Some((Arc::clone(&r.held), now));
    names
}

/// Checks the rows being dragged from the list against the active document after a command ran
/// ([`follow`]), so that the commands after it cannot replace one unseen.
pub(crate) fn follow_drag(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else { return };
    if let Some(PanelDrag::Paint { rows: Some(r), .. }) = craft::ui::runtime::drag::active::<PanelDrag>(&ctx).as_deref() {
        follow(app, r);
    }
}

/// The row `at` names and whether the drop lands after it: onto a row it is that row; in a gap it is
/// the row under the pointer, after it when the gap lies beyond its position in `shown`; a gap with
/// no row under the pointer is the end of the list.
fn place(shown: &[SwatchKey], at: &DropAt<SwatchKey>) -> (Option<String>, bool) {
    match at {
        DropAt::Onto(k) => (Some(name_of(k).to_string()), false),
        DropAt::Gap { key: Some(k), insert, .. } => {
            let after = shown.iter().position(|s| name_of(s) == name_of(k)).is_some_and(|p| *insert > p);
            (Some(name_of(k).to_string()), after)
        }
        _ => (None, false),
    }
}

/// `drag` dropped onto a swatch or between rows of the `shown` list (ap.AP2 `Dropped`); a gap with no
/// row under the pointer is the end of the list.
pub(super) fn on_dropped(app: &mut VectorcraftApp, shown: &[SwatchKey], at: &DropAt<SwatchKey>, drag: &PanelDrag, mods: Mods) {
    let (target, after) = place(shown, at);
    if let Some(drop) = Drop::of(drag, target, after, mods) {
        apply_drop(app, drop);
    }
}

/// A reorder of the browser: one `swatch.move`, one undo step. `to` counts the `shown` rows left
/// once the moved ones are taken out, so it names the row the move lands before (or the end).
pub(super) fn on_reorder(app: &mut VectorcraftApp, shown: &[SwatchKey], mv: &ReorderMove<SwatchKey>) {
    let Some(d) = app.session.active().map(|st| &st.doc) else { return };
    let names: Vec<String> = mv.keys.iter().map(|k| name_of(k).to_string()).collect();
    let rest: Vec<&SwatchKey> = shown.iter().filter(|s| !mv.keys.iter().any(|k| name_of(k) == name_of(s))).collect();
    let (target, after) = match rest.get(mv.to) {
        Some(k) => (Some(name_of(k)), false),
        None => (rest.last().map(|k| name_of(k)), true),
    };
    let target = target.or_else(|| mv.parent.as_ref().map(name_of));
    if let Some(p) = move_params(d, &names, target, after)
        && let Err(e) = app.run("swatch.move", p)
    {
        app.status(e);
    }
}

/// The footer buttons a drag can be released on.
pub(super) enum FooterTarget {
    /// New Swatch: the dragged swatches are duplicated.
    New,
    /// Delete Swatch: the dragged swatches and groups are deleted after asking.
    Delete,
}

/// Swatches or groups dragged from the list and released on a footer button (ap.AP8 `FooterDrop`):
/// New Swatch duplicates the swatches among them, Delete Swatch asks to delete them as a click on
/// the button does. Only rows the active document has held unchanged since the drag started count
/// ([`follow`]): a row deleted, edited or replaced under its name meanwhile, and every row of a drag
/// started in another document, is left alone. Other drags are left to their own targets.
pub(super) fn on_footer_drop(app: &mut VectorcraftApp, target: FooterTarget, drag: &PanelDrag) {
    let PanelDrag::Paint { rows: Some(r), .. } = drag else { return };
    let held = follow(app, r);
    if held.is_empty() {
        return;
    }
    // Rows of the drag must be unchanged since it started; selected rows the drag does not hold come with it.
    let rows: Vec<String> = footer_rows(app, r).into_iter().filter(|n| !r.names.contains(n) || held.contains(n)).collect();
    match target {
        FooterTarget::Delete => {
            let names = super::actions::deletable(app, &rows);
            if !names.is_empty() {
                super::actions::delete(app, names, false);
            }
        }
        FooterTarget::New => {
            let Some(st) = app.session.active() else { return };
            let names: Vec<String> = rows.iter().filter(|n| st.doc.swatch(n).is_some_and(|w| !w.is_reserved())).cloned().collect();
            if !names.is_empty() {
                super::actions::duplicate(app, &names);
            }
        }
    }
}

/// The rows a drop on a footer button takes. A grabbed row that is part of the panel's selection
/// carries every selected row of its kind (swatches or color groups), also those that the find
/// field or the kind filter hides, in panel order, as a click on the button takes the whole
/// selection. Any other grabbed row carries the rows `r` holds.
fn footer_rows(app: &VectorcraftApp, r: &SwatchRows) -> Vec<String> {
    let sel = super::selected(app);
    if !sel.contains(&r.grabbed) {
        return r.names.clone();
    }
    super::shown_keys(&super::groups(app, super::Kind::All, ""))
        .iter()
        .filter(|k| matches!(k, SwatchKey::Group(_)) == r.groups && sel.iter().any(|s| s == name_of(k)))
        .map(|k| name_of(k).to_string())
        .collect()
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
