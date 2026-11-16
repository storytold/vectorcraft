//! Access keys for the in-window menu bar. Native menu labels and command shortcuts stay separate.

use std::collections::HashSet;
use std::ops::Range;

use egui::{Context, Event, Id, Key, Modifiers, Response};
use serde_json::Value;

use super::{Entry, Item, VectorcraftApp, entry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SubKey {
    source: String,
    contents: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourcePath {
    root: String,
    parents: Vec<SubKey>,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Sub(SubKey),
    Command(String, Value),
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TargetKey {
    path: SourcePath,
    kind: Kind,
    // Dynamic slots can keep their command id while naming a different saved item.
    binding: Id,
}

impl TargetKey {
    pub(super) fn salt(&self) -> String {
        format!("menu-access:{self:?}")
    }
}

#[derive(Clone)]
pub(super) struct Cue {
    folded: String,
    range: Range<usize>,
}

#[derive(Clone)]
pub(super) struct AccessRow {
    pub key: TargetKey,
    pub label: String,
    pub cue: Option<Cue>,
    pub enabled: bool,
    preferred: Option<char>,
}

type Field = (Id, egui::text_edit::TextEditState);

#[derive(Clone, Default)]
struct State {
    active: bool,
    root: Option<String>,
    open: bool,
    path: Vec<SubKey>,
    focus: Option<TargetKey>,
    focus_request: Option<TargetKey>,
    scroll_request: Option<TargetKey>,
    first_request: Option<SourcePath>,
    root_request: bool,
    activate: Option<TargetKey>,
    collision: Option<(String, Vec<TargetKey>, usize)>,
    invalidated: bool,
    alt_down: bool,
    alt_armed: bool,
    alt_used: bool,
    composing: bool,
    modifiers: Modifiers,
    keyboard_pointer: bool,
    origin: Option<Id>,
    document: Option<u64>,
    titles: Vec<(String, Id, Id)>,
    registry: Vec<(Id, TargetKey)>,
    shown: Vec<SourcePath>,
}

fn state_id(ctx: &Context) -> Id {
    Id::unique("menu-bar-access").with(ctx.viewport_id())
}

fn read(ctx: &Context) -> State {
    let id = state_id(ctx);
    ctx.data(|d| d.get_temp(id)).unwrap_or_default()
}

fn write(ctx: &Context, state: State) {
    let id = state_id(ctx);
    ctx.data_mut(|d| d.insert_temp(id, state));
}

// Row rendering borrows the stored state in place: never clone its whole growing registry.
// Context/focus/scroll calls belong outside this data lock.
fn with_state<R>(ctx: &Context, f: impl FnOnce(&mut State) -> R) -> R {
    let id = state_id(ctx);
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<State>(id)))
}

fn field_id(ctx: &Context) -> Id {
    Id::unique("menu-bar-field").with(ctx.viewport_id())
}

pub(super) fn remember_field(ctx: &Context) {
    if let Some(id) = super::invoke::focused_text_field(ctx)
        && let Some(state) = egui::TextEdit::load_state(ctx, id)
    {
        let key = field_id(ctx);
        ctx.data_mut(|d| d.insert_temp(key, (id, state)));
    }
}

pub(super) fn field(ctx: &Context) -> Option<Field> {
    let id = field_id(ctx);
    ctx.data(|d| d.get_temp(id))
}

// A menu's focus return resumes the same edit, rather than beginning a new numeric edit.
// Keep the marker for every pass of this frame; a sizing/discard pass must not reselect all.
fn restoration_id(ctx: &Context, field: Id) -> Id {
    field.with(("menu-bar-resume-selection", ctx.viewport_id()))
}

pub(super) fn restore_field(ctx: &Context, id: Id, saved: egui::text_edit::TextEditState) {
    let marker = restoration_id(ctx, id);
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(marker, frame));
    saved.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

pub(crate) fn restoring_field_focus(ctx: &Context, field: Id) -> bool {
    let marker = restoration_id(ctx, field);
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let restoring = d.get_temp::<u64>(marker) == Some(frame);
        if !restoring {
            d.remove_temp::<u64>(marker);
        }
        restoring
    })
}

pub(super) fn forget_field(ctx: &Context) {
    let id = field_id(ctx);
    ctx.data_mut(|d| d.remove::<Field>(id));
}

fn supported(ctx: &Context) -> bool {
    matches!(ctx.os(), egui::os::OperatingSystem::Windows | egui::os::OperatingSystem::Nix)
}

pub(super) fn active(ctx: &Context) -> bool {
    with_state(ctx, |state| state.active)
}

fn cancel(ctx: &Context, state: &mut State, restore: bool) {
    for (_, _, popup) in &state.titles {
        egui::Popup::close_id(ctx, *popup);
    }
    state.active = false;
    state.open = false;
    state.root_request = false;
    state.focus_request = None;
    state.scroll_request = None;
    state.first_request = None;
    state.activate = None;
    state.collision = None;
    state.path.clear();
    if restore {
        if let Some((id, saved)) = field(ctx) {
            restore_field(ctx, id, saved);
        } else if let Some(id) = state.origin {
            ctx.memory_mut(|m| m.request_focus(id));
        } else if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
    }
    if !restore
        && let Some(focused) = ctx.memory(|m| m.focused())
        && (state.titles.iter().any(|(_, id, _)| *id == focused) || state.registry.iter().any(|(id, _)| *id == focused))
    {
        ctx.memory_mut(|m| m.surrender_focus(focused));
    }
    state.origin = None;
    ctx.request_repaint();
}

fn enter(app: &VectorcraftApp, ctx: &Context, state: &mut State) {
    if !state.active {
        remember_field(ctx);
        state.origin = ctx.memory(|m| m.focused());
        state.document = app.session.active().map(|d| d.uid);
        state.active = true;
    }
    state.keyboard_pointer = true;
}

fn choose_root(app: &VectorcraftApp, ctx: &Context, state: &mut State, root: String, open: bool) {
    enter(app, ctx, state);
    for (_, _, popup) in &state.titles {
        egui::Popup::close_id(ctx, *popup);
    }
    // Closing a popup alone keeps egui's open-submenu state for the next pass. A newly
    // selected root must discard those descendants before reopening in this same pass.
    ctx.data_mut(|data| data.remove_by_type::<egui::containers::menu::MenuState>());
    state.root = Some(root.clone());
    state.invalidated = false;
    state.open = open;
    state.path.clear();
    state.focus = None;
    state.focus_request = None;
    state.scroll_request = None;
    state.activate = None;
    state.collision = None;
    state.root_request = true;
    state.first_request = open.then_some(SourcePath { root, parents: vec![] });
    ctx.request_repaint();
}

fn folded(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

fn characters(label: &str) -> Vec<Cue> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    let mut push = |at: usize, ch: char| {
        let key = folded(&ch.to_string());
        if seen.insert(key.clone()) {
            out.push(Cue { folded: key, range: at..at + ch.len_utf8() });
        }
    };
    // Word initials first, then all letters/digits. Literal punctuation is a last resort for a
    // user-named row containing no other character; it is never interpreted as markup.
    let mut initial = true;
    for (at, ch) in label.char_indices() {
        if ch.is_alphanumeric() && initial {
            push(at, ch);
        }
        initial = !ch.is_alphanumeric();
    }
    for (at, ch) in label.char_indices().filter(|(_, c)| c.is_alphanumeric()) {
        push(at, ch);
    }
    for (at, ch) in label.char_indices().filter(|(_, c)| !c.is_whitespace() && !c.is_control()) {
        push(at, ch);
    }
    out
}

fn assign(rows: &mut [AccessRow]) {
    let mut occupied = HashSet::new();
    // Reserve the reported paths before earlier siblings take their letters.
    for row in rows.iter_mut() {
        if let Some(wanted) = row.preferred.map(|c| folded(&c.to_string()))
            && let Some(cue) = characters(&row.label).into_iter().find(|c| c.folded == wanted)
        {
            occupied.insert(cue.folded.clone());
            row.cue = Some(cue);
        }
    }
    for row in rows.iter_mut().filter(|r| r.cue.is_none()) {
        let choices = characters(&row.label);
        row.cue = choices.iter().find(|c| !occupied.contains(&c.folded)).or_else(|| choices.first()).cloned();
        if let Some(cue) = &row.cue {
            occupied.insert(cue.folded.clone());
        }
    }
}

fn roots(app: &VectorcraftApp) -> Vec<(&'static str, Vec<Item>)> {
    super::menu_tree_named(app.session.prefs.font_names_in_english).into_iter().filter(|(name, _)| *name != "VectorCraft").collect()
}

pub(super) fn root_rows(tree: &[(&str, Vec<Item>)]) -> Vec<AccessRow> {
    let mut rows: Vec<_> = tree
        .iter()
        .map(|(source, _)| AccessRow {
            key: TargetKey {
                path: SourcePath { root: (*source).into(), parents: vec![] },
                kind: Kind::Unavailable((*source).into()),
                binding: Id::unique(("menu-root-binding", source, crate::i18n::t(source))),
            },
            label: crate::i18n::t(source).into(),
            cue: None,
            enabled: true,
            preferred: match *source {
                "File" => Some('f'),
                "Edit" => Some('e'),
                "Object" => Some('o'),
                "Type" => Some('t'),
                "Select" => Some('s'),
                "Effect" => Some('c'),
                "View" => Some('v'),
                "Window" => Some('w'),
                "Help" => Some('h'),
                _ => None,
            },
        })
        .collect();
    assign(&mut rows);
    rows
}

fn sub_key(source: &str, items: &[Item]) -> SubKey {
    // Source names, command ids and params (not translated labels or visible row positions).
    // A changed dynamic folder definition invalidates its old selection rather than retargeting it.
    SubKey { source: source.into(), contents: Id::unique(("menu-submenu-definition", format!("{items:?}"))) }
}

pub(super) fn root_path(source: &str) -> SourcePath {
    SourcePath { root: source.into(), parents: vec![] }
}

pub(super) fn child_path(key: &TargetKey) -> SourcePath {
    let mut path = key.path.clone();
    if let Kind::Sub(sub) = &key.kind {
        path.parents.push(sub.clone());
    }
    path
}

pub(super) fn level(app: &VectorcraftApp, path: &SourcePath, items: &[Item]) -> Vec<AccessRow> {
    let mut rows = vec![];
    for item in items {
        match entry(app, item) {
            Some(Entry::Sub(label, children)) => {
                let Item::Sub(source, _) = item else { continue };
                rows.push(AccessRow {
                    key: TargetKey {
                        path: path.clone(),
                        kind: Kind::Sub(sub_key(source, children)),
                        binding: Id::unique(("menu-submenu-binding", source, label)),
                    },
                    label: label.into(),
                    cue: None,
                    enabled: true,
                    preferred: (path.root == "Object" && path.parents.is_empty() && *source == "Path").then_some('p'),
                });
            }
            Some(Entry::Item(row)) => {
                let target = row.target();
                let preferred = row.command.and_then(|(id, _)| match id {
                    "object.path.offsetPath" => Some('o'),
                    "select.save" if path.root == "Select" && path.parents.is_empty() => Some('s'),
                    "select.editSaved" if path.root == "Select" && path.parents.is_empty() => Some('e'),
                    _ => None,
                });
                rows.push(AccessRow {
                    key: TargetKey {
                        path: path.clone(),
                        kind: target.map_or_else(|| Kind::Unavailable(row.source.into()), |(id, params)| Kind::Command(id, params)),
                        binding: Id::unique(("menu-row-binding", row.source, row.label.as_ref())),
                    },
                    label: row.label.into_owned(),
                    cue: None,
                    enabled: row.enabled && row.command.is_some(),
                    preferred,
                });
            }
            _ => {}
        }
    }
    assign(&mut rows);
    rows
}

fn current_rows(app: &VectorcraftApp, state: &State) -> Option<Vec<AccessRow>> {
    let root = state.root.as_deref()?;
    let tree = roots(app);
    let (_, items) = tree.iter().find(|(name, _)| *name == root)?;
    let mut items = items.as_slice();
    for wanted in &state.path {
        items = items.iter().find_map(|item| match item {
            Item::Sub(source, children) if *source == wanted.source.as_str() && sub_key(source, children) == *wanted => Some(children.as_slice()),
            _ => None,
        })?;
    }
    Some(level(app, &SourcePath { root: root.into(), parents: state.path.clone() }, items))
}

fn mnemonic(rows: &[AccessRow], text: &str) -> Vec<TargetKey> {
    let folded = folded(text);
    rows.iter().filter(|r| r.enabled && r.cue.as_ref().is_some_and(|c| c.folded == folded)).map(|r| r.key.clone()).collect()
}

fn letter(app: &VectorcraftApp, ctx: &Context, state: &mut State, text: &str, fallback: Option<&str>, root_chord: bool) -> bool {
    // A newly pressed Alt chord addresses the bar, even while a submenu is open.
    let rows = if state.active && state.open && !root_chord { current_rows(app, state) } else { Some(root_rows(&roots(app))) };
    let Some(rows) = rows else {
        cancel(ctx, state, true);
        return true;
    };
    let mut matches = mnemonic(&rows, text);
    if matches.is_empty()
        && let Some(fallback) = fallback
    {
        matches = mnemonic(&rows, fallback);
    }
    if matches.is_empty() {
        return state.active;
    }
    if root_chord || !state.active || !state.open {
        if let Some(target) = matches.first() {
            choose_root(app, ctx, state, target.path.root.clone(), true);
        }
        return true;
    }
    let chosen = if matches.len() == 1 {
        state.collision = None;
        matches.first().cloned()
    } else {
        let token = folded(text);
        let index = match &state.collision {
            Some((old, keys, at)) if old == &token && keys == &matches => (at + 1) % matches.len(),
            _ => 0,
        };
        let chosen = matches.get(index).cloned();
        state.collision = Some((token, matches, index));
        chosen
    };
    if let Some(chosen) = chosen {
        state.invalidated = false;
        state.focus_request = Some(chosen.clone());
        state.focus = Some(chosen.clone());
        // A submenu mnemonic focuses its genuine button; real Right/Enter enters it through
        // egui, including its RTL and stationary-pointer handling. Collisions require Enter.
        if matches!(chosen.kind, Kind::Command(..)) && state.collision.is_none() {
            state.activate = Some(chosen);
        }
        state.keyboard_pointer = true;
        ctx.request_repaint();
    }
    true
}

fn owner_blocks(app: &VectorcraftApp, ctx: &Context, state: &State) -> bool {
    !supported(ctx)
        || !ctx.input(|i| i.focused)
        || app.file_dialog_open()
        || app.ui.dialog.is_some()
        || app.ui.palette_open
        || app.ui.screen_mode == 3
        || app.ui.tools_drawer
        || crate::updates_ui::blocks_input(app)
        || !app.held_tool.is_empty()
        || app.session.tool_busy()
        || app.session.tool_composing()
        || state.composing
        || crate::scrub::phase(ctx) != crate::scrub::Phase::Idle
        || crate::canvas::dragging(ctx)
        || ctx.dragged_id().is_some()
}

fn update_composition(ctx: &Context, state: &mut State, events: &[Event]) -> bool {
    if !events.iter().any(|e| matches!(e, Event::ModifiersChanged(_) | Event::Key { .. })) {
        state.modifiers = ctx.input(|i| i.modifiers);
    }
    let mut ime = false;
    for event in events {
        match event {
            Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                state.composing = !text.is_empty();
                ime = true;
            }
            Event::Ime(egui::ImeEvent::Commit(_)) => {
                state.composing = false;
                ime = true;
            }
            #[allow(deprecated)]
            Event::Ime(egui::ImeEvent::Disabled) => {
                state.composing = false;
                ime = true;
            }
            Event::Ime(_) => ime = true,
            Event::WindowFocused(false) => {
                state.alt_down = false;
                state.alt_armed = false;
                state.composing = false;
            }
            _ => {}
        }
    }
    ime
}

fn prepare_input(app: &VectorcraftApp, ctx: &Context, state: &mut State, events: &[Event]) -> Option<bool> {
    let ime = update_composition(ctx, state, events);
    if ime || owner_blocks(app, ctx, state) {
        if state.active {
            cancel(ctx, state, ime && app.ui.dialog.is_none());
        }
        state.alt_armed = false;
        return None;
    }
    if !state.active && egui::Popup::is_any_open(ctx) {
        if let Some((source, _, _)) = state.titles.iter().find(|(_, _, id)| egui::Popup::is_id_open(ctx, *id)).cloned() {
            enter(app, ctx, state);
            state.root = Some(source);
            state.open = true;
            state.keyboard_pointer = false;
        } else {
            state.alt_armed = false;
            return None; // context menu/popover owns its own keys
        }
    }
    refresh_targets(app, ctx, state);
    let pointer_activity = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO || i.pointer.any_click() || i.pointer.any_down());
    if pointer_activity {
        state.invalidated = false;
        state.keyboard_pointer = false;
        state.alt_armed = false;
    }
    Some(pointer_activity)
}

fn refresh_targets(app: &VectorcraftApp, ctx: &Context, state: &mut State) {
    if state.active && state.document != app.session.active().map(|d| d.uid) {
        cancel(ctx, state, false);
    }
    if !state.root_request
        && !state.invalidated
        && let Some(focused) = ctx.memory(|m| m.focused())
        && let Some((_, key)) = state.registry.iter().find(|(id, _)| *id == focused)
    {
        state.focus = Some(key.clone());
        state.path = key.path.parents.clone();
    }
    // Re-resolve current labels/targets; a vanished dynamic target never invokes its replacement.
    if state.active && state.open {
        match current_rows(app, state) {
            Some(rows) => {
                if state.focus.as_ref().is_some_and(|key| !rows.iter().any(|r| &r.key == key)) {
                    if let Some(id) = ctx.memory(|m| m.focused()) {
                        ctx.memory_mut(|m| m.surrender_focus(id));
                    }
                    state.invalidated = true;
                    state.first_request = None;
                    state.scroll_request = None;
                    state.focus = None;
                    state.focus_request = None;
                    state.activate = None;
                    state.collision = None;
                }
            }
            None => {
                if let Some(id) = ctx.memory(|m| m.focused()) {
                    ctx.memory_mut(|m| m.surrender_focus(id));
                }
                state.invalidated = true;
                state.first_request = None;
                state.scroll_request = None;
                state.path.clear();
                state.focus = None;
                state.focus_request = None;
                state.activate = None;
                state.collision = None;
            }
        }
    }
}

fn modifiers_changed(app: &VectorcraftApp, ctx: &Context, state: &mut State, m: &Modifiers, pointer_activity: bool, owned: &mut bool) {
    let plain_alt = m.alt && !m.ctrl && !m.command && !m.mac_cmd;
    if plain_alt && !state.alt_down {
        state.alt_armed = !pointer_activity;
        state.alt_used = false;
    }
    if !m.alt && state.alt_down {
        if state.alt_armed && !state.alt_used {
            *owned = true;
            if state.active {
                cancel(ctx, state, true);
            } else {
                choose_root(app, ctx, state, "File".into(), false);
            }
        }
        state.alt_armed = false;
    }
    if m.ctrl || m.command || m.mac_cmd {
        state.alt_armed = false;
        state.alt_used = true;
        if state.active {
            cancel(ctx, state, true);
        }
        *owned = false;
    }
    state.alt_down = m.alt;
}

fn consume_event(consumed: &mut [bool], at: usize) {
    if let Some(c) = consumed.get_mut(at) {
        *c = true;
    }
}

fn horizontal_navigation(app: &VectorcraftApp, ctx: &Context, state: &mut State, key: Key) -> bool {
    let towards = if ctx.global_style().right_to_left { Key::ArrowLeft } else { Key::ArrowRight };
    let submenu = state.focus.as_ref().is_some_and(|f| matches!(f.kind, Kind::Sub(_)));
    if key != towards || !submenu {
        let roots = roots(app);
        if let Some(index) = roots.iter().position(|(s, _)| Some(*s) == state.root.as_deref()) {
            let forward = (key == Key::ArrowRight) != ctx.global_style().right_to_left;
            let next = if forward { (index + 1) % roots.len() } else { (index + roots.len() - 1) % roots.len() };
            if let Some((root, _)) = roots.get(next) {
                let was_open = state.open;
                choose_root(app, ctx, state, (*root).into(), was_open);
            }
        }
        return true;
    }
    false
}

fn navigation_key(app: &VectorcraftApp, ctx: &Context, state: &mut State, key: Key, consumed: &mut [bool], at: usize, owned: &mut bool) -> bool {
    match key {
        Key::Enter | Key::Space if state.invalidated => {
            // A disappeared item is not permission to activate the next slot,
            // including when egui gives that replacement its default focus.
            consume_event(consumed, at);
            *owned = true;
            true
        }
        Key::Escape if state.path.is_empty() => {
            cancel(ctx, state, true);
            *owned = true;
            consume_event(consumed, at);
            true
        }
        Key::Escape | Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight | Key::Enter | Key::Space => {
            if matches!(key, Key::ArrowLeft | Key::ArrowRight) && state.path.is_empty() {
                if horizontal_navigation(app, ctx, state, key) {
                    consume_event(consumed, at);
                }
            } else if !state.open && matches!(key, Key::ArrowDown | Key::Enter | Key::Space) {
                if let Some(root) = state.root.clone() {
                    choose_root(app, ctx, state, root, true);
                }
                consume_event(consumed, at);
            }
            *owned = true;
            true // egui owns the remaining navigation keys
        }
        _ => false,
    }
}

fn pressed_key(app: &VectorcraftApp, ctx: &Context, state: &mut State, events: &[Event], at: usize, consumed: &mut [bool], owned: &mut bool) {
    let Event::Key { key, physical_key, pressed: true, repeat, modifiers } = &events[at] else {
        return;
    };
    if modifiers.ctrl || modifiers.command || modifiers.mac_cmd || (modifiers.alt && matches!(key, Key::Tab | Key::Space)) {
        state.alt_armed = false;
        state.alt_used = true;
        if state.active {
            cancel(ctx, state, true);
            *owned = false;
        }
        return;
    }
    let access = state.active || modifiers.alt;
    if !access {
        return;
    }
    if modifiers.alt && crate::shortcuts::runs_a_shortcut(key.name(), physical_key.unwrap_or(*key).name(), *modifiers) {
        state.alt_armed = false;
        state.alt_used = true;
        if state.active {
            cancel(ctx, state, true);
            *owned = false;
        }
        return;
    }
    if state.active {
        state.keyboard_pointer = true;
        if matches!(key, Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight) {
            state.invalidated = false;
        }
        if navigation_key(app, ctx, state, *key, consumed, at, owned) {
            return;
        }
    }
    let root_chord = modifiers.alt && !state.alt_used;
    state.alt_used = true;
    state.alt_armed = false;
    // Native input is Key then Text. Prefer its delivered Unicode text, while keeping
    // the logical/Latin key as fallback. The pair is consumed together, exactly once.
    let text_at = events
        .iter()
        .enumerate()
        .skip(at + 1)
        .take_while(|(_, e)| !matches!(e, Event::Key { pressed: true, .. } | Event::Ime(_)))
        .find_map(|(i, e)| matches!(e, Event::Text(_)).then_some(i));
    let text = text_at.and_then(|i| events.get(i)).and_then(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None });
    let fallback = key.name();
    if !*repeat && letter(app, ctx, state, text.unwrap_or(fallback), Some(fallback), root_chord) || state.active {
        *owned = true;
        consume_event(consumed, at);
        if let Some(i) = text_at {
            consume_event(consumed, i);
        }
    }
}

fn text_event(app: &VectorcraftApp, ctx: &Context, state: &mut State, text: &str, consumed: &mut [bool], at: usize, owned: &mut bool) {
    if state.modifiers.ctrl || state.modifiers.command || state.modifiers.mac_cmd {
        state.alt_armed = false;
        state.alt_used = true;
        if state.active {
            cancel(ctx, state, true);
        }
        *owned = false;
        // AltGr/Ctrl text belongs downstream, even without a Key event.
    } else if state.active || state.alt_down && state.alt_armed {
        let root_chord = state.alt_down && !state.alt_used;
        state.alt_used = true;
        state.alt_armed = false;
        if letter(app, ctx, state, text, None, root_chord) || state.active {
            *owned = true;
            consume_event(consumed, at);
        }
    }
}

/// Called only by App's existing once-per-frame gate, BEFORE ordinary shortcuts.
pub(crate) fn handle_input(app: &mut VectorcraftApp, ctx: &Context) -> bool {
    let mut state = read(ctx);
    let events = ctx.input(|i| i.events.clone());
    let Some(pointer_activity) = prepare_input(app, ctx, &mut state, &events) else {
        write(ctx, state);
        return false;
    };
    let mut owned = state.active;
    let mut consumed = vec![false; events.len()];
    for (at, event) in events.iter().enumerate() {
        if consumed.get(at).copied().unwrap_or(false) {
            continue;
        }
        match event {
            Event::ModifiersChanged(m)
            | Event::Key { modifiers: m, .. }
            | Event::PointerButton { modifiers: m, .. }
            | Event::MouseWheel { modifiers: m, .. } => state.modifiers = *m,
            _ => {}
        }
        match event {
            Event::ModifiersChanged(m) => modifiers_changed(app, ctx, &mut state, m, pointer_activity, &mut owned),
            Event::Key { pressed: true, .. } => pressed_key(app, ctx, &mut state, &events, at, &mut consumed, &mut owned),
            Event::Text(text) => text_event(app, ctx, &mut state, text, &mut consumed, at, &mut owned),
            Event::PointerButton { .. } | Event::MouseWheel { .. } => {
                // Preserve event order: wheel hands scrolling to the real pointer, but a later
                // navigation key owns it again. Smoothed wheel tails must not undo that choice.
                state.keyboard_pointer = false;
                state.alt_armed = false;
                state.alt_used = true;
            }
            _ => {}
        }
    }
    ctx.input_mut(|i| {
        let mut at = 0;
        i.events.retain(|_| {
            let keep = !consumed.get(at).copied().unwrap_or(false);
            at += 1;
            keep
        });
    });
    write(ctx, state);
    owned
}

pub(super) fn begin_bar(ctx: &Context) {
    with_state(ctx, |state| {
        state.registry.clear();
        state.shown.clear();
    });
}

pub(super) fn cues(ctx: &Context) -> bool {
    supported(ctx) && with_state(ctx, |state| state.active || state.alt_down && state.alt_armed)
}

pub(super) fn suppress_stationary_hover(ctx: &Context) -> bool {
    let keyboard = with_state(ctx, |state| state.active && state.keyboard_pointer);
    keyboard && ctx.input(|i| i.pointer.delta() == egui::Vec2::ZERO && !i.pointer.any_click() && !i.pointer.any_down())
}

pub(super) fn bind_root(ctx: &Context, source: &str, response: &Response) {
    let popup = egui::Popup::default_response_id(response);
    let requested = with_state(ctx, |state| {
        if let Some(root) = state.titles.iter_mut().find(|(name, _, _)| name == source) {
            *root = (source.into(), response.id, popup);
        } else {
            state.titles.push((source.into(), response.id, popup));
        }
        if state.active && state.root_request && state.root.as_deref() == Some(source) {
            state.root_request = false;
            Some(state.open)
        } else {
            None
        }
    });
    if let Some(open) = requested {
        response.request_focus();
        if open {
            egui::Popup::open_id(ctx, popup);
        } else {
            egui::Popup::close_id(ctx, popup);
        }
    }
}

pub(super) fn note_level(ctx: &Context, path: &SourcePath) {
    with_state(ctx, |state| state.shown.push(path.clone()));
}

// The public SubMenuButton uses this same next_auto_id for its genuine button. Let its own
// Right/Enter/RTL path open it. Only request focus after releasing the data lock.
pub(super) fn prepare_submenu(ctx: &Context, row: &AccessRow, id: Id) {
    let request = with_state(ctx, |state| {
        let focus_here = match &state.focus_request {
            Some(wanted) => wanted == &row.key,
            None => state.first_request.as_ref() == Some(&row.key.path),
        };
        if state.active && row.enabled && focus_here {
            state.focus = Some(row.key.clone());
            state.focus_request = None;
            state.first_request = None;
            state.scroll_request = Some(row.key.clone());
            true
        } else {
            false
        }
    });
    if request {
        ctx.memory_mut(|m| m.request_focus(id));
    }
}

pub(super) fn bind_response(ctx: &Context, row: &AccessRow, response: &Response) -> bool {
    let has_focus = response.has_focus();
    let (request_focus, scroll, activate) = with_state(ctx, |state| {
        state.registry.push((response.id, row.key.clone()));
        let scroll = state.scroll_request.as_ref() == Some(&row.key);
        if scroll {
            state.scroll_request = None;
        }
        let focus_here = match &state.focus_request {
            Some(wanted) => wanted == &row.key,
            None => state.first_request.as_ref() == Some(&row.key.path),
        };
        let request_focus = state.active && row.enabled && focus_here;
        if request_focus {
            state.focus_request = None;
            state.first_request = None;
            state.focus = Some(row.key.clone());
        }
        if has_focus && !state.invalidated {
            state.focus = Some(row.key.clone());
        }
        // Real egui arrow navigation changes focus without a helper focus_request.
        // Keep that keyboard row visible, while pointer/wheel scrolling keeps its own position.
        let focused_keyboard_row = state.active && state.keyboard_pointer && !state.invalidated && row.enabled && has_focus;
        let activate = state.active && row.enabled && state.activate.as_ref() == Some(&row.key);
        if activate {
            state.activate = None;
            state.active = false;
            state.open = false;
            state.root_request = false;
        }
        (request_focus, scroll || request_focus || focused_keyboard_row, activate)
    });
    if request_focus {
        response.request_focus();
    }
    if scroll {
        response.scroll_to_me(None);
    }
    activate
}

pub(super) fn chosen(ctx: &Context) {
    with_state(ctx, |state| {
        state.active = false;
        state.open = false;
        state.activate = None;
        state.root_request = false;
        state.collision = None;
    });
}

pub(super) fn finish_bar(ctx: &Context, open: bool, command_chosen: bool) {
    let mut state = read(ctx);
    if state.active {
        if let Some((source, _, _)) = state.titles.iter().find(|(_, _, id)| egui::Popup::is_id_open(ctx, *id)).cloned() {
            state.root = Some(source.clone());
            state.open = true;
            state.path =
                state.shown.iter().filter(|p| p.root == source).max_by_key(|p| p.parents.len()).map(|p| p.parents.clone()).unwrap_or_default();
        } else if !open && (state.open || ctx.input(|i| i.pointer.any_click())) {
            // Bare Alt exposes cues without a popup; an outside click still ends access mode.
            let restore = !command_chosen && !ctx.input(|i| i.pointer.any_click());
            cancel(ctx, &mut state, restore);
        }
    }
    write(ctx, state);
}

/// A layout job changes only presentation; literal '&'/'_' and all native/shared labels survive.
pub(super) fn text(ui: &egui::Ui, label: &str, prefix: &str, cue: Option<&Cue>, size: Option<f32>) -> egui::WidgetText {
    let Some(cue) = cue else {
        let text = format!("{prefix}{label}");
        return match size {
            Some(size) => egui::RichText::new(text).size(size).color(super::Tokens::get(ui.ctx()).text).into(),
            None => text.into(),
        };
    };
    let Some((before, rest)) = label.split_at_checked(cue.range.start) else { return format!("{prefix}{label}").into() };
    let Some((letter, after)) = rest.split_at_checked(cue.range.len()) else { return format!("{prefix}{label}").into() };
    let mut font = egui::TextStyle::Button.resolve(ui.style());
    if let Some(size) = size {
        font.size = size;
    }
    let format = egui::TextFormat { font_id: font, color: ui.visuals().text_color(), ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("{prefix}{before}"), 0.0, format.clone());
    job.append(letter, 0.0, egui::TextFormat { underline: egui::Stroke::new(1.0, format.color), ..format.clone() });
    job.append(after, 0.0, format);
    job.into()
}
