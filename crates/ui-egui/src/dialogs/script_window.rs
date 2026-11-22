//! A dialog a script builds with ScriptUI's `new Window("dialog")` and shows with `show()`
//! (kind `scriptWindow`). It draws the controls the running script's dialog holds, at the places
//! the engine laid them out, and gives the script each click and change.
//!
//! Its fields are `title` and, for `ui.dialog.set` and `ui.dialog.confirm`, `event` (`click`,
//! `change` or `close`), `control` (a control's index in the dialog's list of elements, or its
//! name or text) and `value` (an edit field's text, a slider's number, or a list's item index or
//! null). `ui.dialog.confirm` gives the script that event and leaves the dialog open; an event the
//! dialog cannot take is an error. An edit field gives its text when it loses the keyboard focus,
//! when Enter is pressed in a one-line field, or before a click on another control or a close. Events given in
//! one frame reach the script one at a time, in order. Enter outside a field clicks the default button,
//! and Escape or `ui.dialog.cancel` clicks the cancel button, or closes the dialog when it has
//! none. Stop ends the run and puts back everything it did.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_engine::script::scriptui::UiNode;
use vectorcraft_engine::script::{ScriptUiDialog, UiEvent, UiKind, UiValue};

use super::{DialogAction, DialogSpec};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of a script's ScriptUI dialog.
pub const KIND: &str = "scriptWindow";

/// The most of the dialog's content shown at once; a larger dialog scrolls.
const MAX_SHOWN: egui::Vec2 = egui::vec2(960.0, 640.0);

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading,
    body_with_actions: Some(body),
    confirm,
    ok: None,
    inputs: |_| &["event", "control", "value"],
    min_width: 240.0,
    max_width: Some(1000.0),
    ..DialogSpec::FORM
};

/// Shows the ScriptUI dialog the running script waits at.
pub(crate) fn open(app: &mut VectorcraftApp, w: &ScriptUiDialog) {
    app.ui.dialog = Some(Dialog::new(KIND, json!({"title": w.title(), SHOWN_KEY: w.shown})));
}

/// The field that holds which `show()` of the run the open dialog is for.
const SHOWN_KEY: &str = "shown";

/// Is `d` open for the dialog `w`, and not for one the script showed before it?
pub(crate) fn is_open_for(d: &Dialog, w: &ScriptUiDialog) -> bool {
    d.kind == KIND && d.fields.get(SHOWN_KEY).and_then(Value::as_u64) == Some(w.shown)
}

fn heading(d: &Dialog) -> String {
    let title = d.str("title");
    if title.is_empty() { tl!("Script Alert").into() } else { vectorcraft_engine::script::dialogs::shown_text(&title, false) }
}

fn memory_id() -> egui::Id {
    egui::Id::unique("scriptWindow.shown")
}

/// The dialog to draw: the one the run waits at, or while a handler runs, the one it last waited
/// at, with every control disabled.
fn shown(app: &VectorcraftApp, ctx: &egui::Context) -> Option<(Arc<ScriptUiDialog>, bool)> {
    if let Some(w) = app.session.script_window() {
        let w = Arc::new(w);
        ctx.data_mut(|m| m.insert_temp(memory_id(), w.clone()));
        return Some((w, true));
    }
    ctx.data(|m| m.get_temp::<Arc<ScriptUiDialog>>(memory_id())).map(|w| (w, false))
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let Some((w, waiting)) = shown(app, ui.ctx()) else { return DialogAction::None };
    let mut events = Vec::new();
    let (width, height) = w.size();
    let size = egui::vec2(width.max(1.0), height.max(1.0));
    egui::ScrollArea::both().max_width(MAX_SHOWN.x).max_height(MAX_SHOWN.y).auto_shrink([true, true]).show(ui, |ui| {
        let (area, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.add_enabled_ui(waiting, |ui| {
            for (i, n) in w.nodes.iter().enumerate().skip(1) {
                if n.visible
                    && let Some(e) = control(ui, d, area.min, i, n, w.default_element == Some(i))
                {
                    events.push(e);
                }
            }
        });
    });
    ui.add_space(12.0);
    let mut stop = false;
    let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        stop = widgets::secondary_button(ui, tl!("Stop")).clicked();
    });
    if stop {
        app.session.script_request_stop();
        app.ui.dialog = None;
        return DialogAction::None;
    }
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if events.is_empty() && waiting && !typing && crate::shortcuts::pressed_anew(ui.ctx(), egui::Key::Enter) {
        events.extend(w.default_element.map(UiEvent::Click));
    }
    if events.iter().any(|e| matches!(e, UiEvent::Click(_))) {
        commit_drafts(&w, d, &mut events);
    }
    let mut queue: Vec<Value> = match d.fields.remove(QUEUE_KEY) {
        Some(Value::Array(queue)) => queue,
        _ => Vec::new(),
    };
    queue.extend(events.iter().map(event_fields));
    if !waiting || queue.is_empty() {
        if !queue.is_empty() {
            d.fields.insert(QUEUE_KEY.into(), Value::Array(queue));
        }
        return DialogAction::None;
    }
    let next = queue.remove(0);
    if !queue.is_empty() {
        d.fields.insert(QUEUE_KEY.into(), Value::Array(queue));
    }
    if let Value::Object(fields) = next {
        d.fields.extend(fields);
    }
    DialogAction::Confirm
}

/// The field that holds the events the person gave in one frame that the script has not had yet;
/// the script takes one event each time it waits.
const QUEUE_KEY: &str = "queued";

/// Before a click or a close, gives the script the text typed in each edit field that has not left
/// the field yet, so the handler of a button that closes the dialog, or `onClose`, reads it.
pub(crate) fn commit_drafts(w: &ScriptUiDialog, d: &mut Dialog, events: &mut Vec<UiEvent>) {
    let mut changes = Vec::new();
    for (i, n) in w.nodes.iter().enumerate() {
        if n.kind != UiKind::EditText || n.readonly || events.iter().any(|e| matches!(e, UiEvent::Change(c, _) if *c == i)) {
            continue;
        }
        if let Some(Value::String(text)) = d.fields.remove(&draft_key(i))
            && text != n.text
        {
            changes.push(UiEvent::Change(i, UiValue::Text(text)));
        }
    }
    let closing = |e: &UiEvent| matches!(e, UiEvent::Click(_) | UiEvent::Close);
    events.sort_by_key(closing);
    let at = events.iter().position(closing).unwrap_or(events.len());
    events.splice(at..at, changes);
}

/// The field that holds an edit field's text while it is typed.
fn draft_key(i: usize) -> String {
    format!("text{i}")
}

/// Draws control `i` at its place from `origin`. → the event it gives.
fn control(ui: &mut egui::Ui, d: &mut Dialog, origin: egui::Pos2, i: usize, n: &UiNode, default: bool) -> Option<UiEvent> {
    let rect = egui::Rect::from_min_size(origin + egui::vec2(n.rect.x, n.rect.y), egui::vec2(n.rect.w, n.rect.h));
    let t = Tokens::get(ui.ctx());
    let mut event = None;
    ui.push_id(i, |ui| {
        ui.add_enabled_ui(n.enabled, |ui| match n.kind {
            UiKind::Dialog | UiKind::Group => {}
            UiKind::Panel => {
                let frame = rect.shrink(1.0);
                let (enabled, name) = (ui.is_enabled(), n.text.clone());
                let r = ui.interact(rect, ui.scope_id().with("panel"), egui::Sense::hover());
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, enabled, &name));
                ui.painter().rect_stroke(frame, 4.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                if !n.text.is_empty() {
                    ui.painter().text(frame.min + egui::vec2(8.0, -6.0), egui::Align2::LEFT_TOP, &n.text, egui::FontId::proportional(11.0), t.text);
                }
            }
            UiKind::StaticText => {
                ui.put(rect, egui::Label::new(&n.text).wrap());
            }
            UiKind::Button => {
                let mut button = egui::Button::new(&n.text);
                if default {
                    button = button.fill(t.accent_strong);
                }
                if ui.put(rect, button).clicked() {
                    event = Some(UiEvent::Click(i));
                }
            }
            UiKind::CheckBox => {
                let mut on = n.checked;
                if ui.put(rect, egui::Checkbox::new(&mut on, &n.text)).clicked() {
                    event = Some(UiEvent::Click(i));
                }
            }
            UiKind::RadioButton => {
                if ui.put(rect, egui::RadioButton::new(n.checked, &n.text)).clicked() {
                    event = Some(UiEvent::Click(i));
                }
            }
            UiKind::EditText => event = edit(ui, d, rect, i, n),
            UiKind::DropDownList => {
                let shown = n.selection.and_then(|k| n.items.get(k)).cloned().unwrap_or_default();
                let mut picked = None;
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    craft_ui::ComboBox::from_id_salt(("script-window-list", i)).selected_text(shown).width(rect.width()).show_ui(ui, |ui| {
                        for (k, item) in n.items.iter().enumerate() {
                            if ui.selectable_label(n.selection == Some(k), item).clicked() {
                                picked = Some(k);
                            }
                        }
                    });
                });
                if let Some(k) = picked.filter(|k| n.selection != Some(*k)) {
                    event = Some(UiEvent::Change(i, UiValue::Index(Some(k))));
                }
            }
            UiKind::ListBox => {
                ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(3.0)), |ui| {
                    egui::ScrollArea::vertical().id_salt(("script-window-listbox", i)).max_height(rect.height() - 6.0).show(ui, |ui| {
                        for (k, item) in n.items.iter().enumerate() {
                            if ui.selectable_label(n.selection == Some(k), item).clicked() && n.selection != Some(k) {
                                event = Some(UiEvent::Change(i, UiValue::Index(Some(k))));
                            }
                        }
                    });
                });
            }
            UiKind::Slider => {
                let mut v = n.value;
                let r = ui.put(rect, egui::Slider::new(&mut v, n.min..=n.max).show_value(false));
                name_for_screen_readers(ui, &r, n);
                if (r.drag_stopped() || (r.changed() && !r.dragged())) && v != n.value {
                    event = Some(UiEvent::Change(i, UiValue::Number(v)));
                }
            }
            UiKind::ProgressBar => {
                let span = n.max - n.min;
                let part = if span > 0.0 { ((n.value - n.min) / span) as f32 } else { 0.0 };
                let r = ui.put(rect, egui::ProgressBar::new(part));
                name_for_screen_readers(ui, &r, n);
            }
        });
    });
    event
}

/// Screen readers call a control with no visible label of its own (an edit field, a slider or a
/// progress bar) by the name the script gave it, or else a slider or progress bar by its text.
fn name_for_screen_readers(ui: &egui::Ui, r: &egui::Response, n: &UiNode) {
    let label = if !n.name.is_empty() {
        n.name.clone()
    } else if n.kind != UiKind::EditText {
        n.text.clone()
    } else {
        String::new()
    };
    if !label.is_empty() {
        ui.ctx().accesskit_node_builder(r.id, |node| node.set_label(label));
    }
}

/// An edit field: its text is kept in the dialog while it is typed and given to the script when
/// the field loses the focus, or at Enter in a one-line field.
fn edit(ui: &mut egui::Ui, d: &mut Dialog, rect: egui::Rect, i: usize, n: &UiNode) -> Option<UiEvent> {
    let key = draft_key(i);
    let mut text = d.fields.get(&key).and_then(Value::as_str).map_or_else(|| n.text.clone(), str::to_string);
    let field = if n.multiline { egui::TextEdit::multiline(&mut text) } else { egui::TextEdit::singleline(&mut text) };
    let r = ui.put(rect, field.interactive(!n.readonly).desired_width(rect.width()));
    name_for_screen_readers(ui, &r, n);
    if r.changed() {
        d.fields.insert(key.clone(), json!(text));
    }
    let entered = !n.multiline && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if (r.lost_focus() || entered) && text != n.text && !n.readonly {
        d.fields.remove(&key);
        return Some(UiEvent::Change(i, UiValue::Text(text)));
    }
    if !r.has_focus() && !r.changed() {
        d.fields.remove(&key);
    }
    None
}

/// `event` as the dialog's `event`, `control` and `value` fields.
fn event_fields(event: &UiEvent) -> Value {
    let (name, control, value) = match event {
        UiEvent::Click(i) => ("click", json!(i), Value::Null),
        UiEvent::Change(i, UiValue::Text(t)) => ("change", json!(i), json!(t)),
        UiEvent::Change(i, UiValue::Number(v)) => ("change", json!(i), json!(v)),
        UiEvent::Change(i, UiValue::Index(k)) => ("change", json!(i), json!(k)),
        UiEvent::Close => ("close", Value::Null, Value::Null),
    };
    json!({"event": name, "control": control, "value": value})
}

/// The event in the dialog's fields, for the dialog `w`.
fn event_of(w: &ScriptUiDialog, d: &Dialog) -> Result<UiEvent, String> {
    let control = match d.fields.get("control") {
        Some(Value::Number(n)) => n.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("`control` must be a control's index, name or text")?,
        Some(Value::String(s)) => {
            w.find(s).ok_or_else(|| format!("the dialog has no control named {}", vectorcraft_engine::script::dialogs::cut_to(s, 64)))?
        }
        _ => 0,
    };
    let list = w.nodes.get(control).is_some_and(|n| matches!(n.kind, UiKind::DropDownList | UiKind::ListBox));
    match (d.str("event").as_str(), d.fields.get("value")) {
        ("close", _) => Ok(UiEvent::Close),
        ("click", _) => Ok(UiEvent::Click(control)),
        ("change", Some(Value::String(t))) => Ok(UiEvent::Change(control, UiValue::Text(t.clone()))),
        ("change", Some(Value::Number(v))) if list => match v.as_u64() {
            Some(k) => Ok(UiEvent::Change(control, UiValue::Index(Some(usize::try_from(k).unwrap_or(usize::MAX))))),
            None => Err("a list's `value` must be an item index or null".into()),
        },
        ("change", Some(Value::Number(v))) => Ok(UiEvent::Change(control, UiValue::Number(v.as_f64().unwrap_or(f64::NAN)))),
        ("change", Some(Value::Null) | None) if list => Ok(UiEvent::Change(control, UiValue::Index(None))),
        ("change", _) => Err("a change's `value` must be a string, a number or null".into()),
        _ => Err("`event` must be click, change or close".into()),
    }
}

/// Gives the running script the event in the dialog's fields. The dialog stays open for the
/// script's next wait; an event it cannot take is an error.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let Some(w) = app.session.script_window() else { return Err("no script dialog waits".into()) };
    let event = event_of(&w, d)?;
    app.session.script_window_answer(event)?;
    if let Some(open) = app.ui.dialog.as_mut().filter(|open| open.kind == KIND) {
        for k in ["event", "control", "value"] {
            open.fields.remove(k);
        }
    }
    Ok(Value::Null)
}

/// Escape or `ui.dialog.cancel`: a click on the cancel button, or the close box when the dialog
/// has none or it cannot be clicked, after the text still typed in its fields. Events the script
/// has not had yet go after the ones before them.
pub(super) fn cancel(app: &mut VectorcraftApp, d: &mut Dialog) {
    let Some(w) = app.session.script_window() else { return };
    let click = w.cancel_element.map(UiEvent::Click).filter(|e| vectorcraft_engine::script::scriptui::check_event(&w, e.clone()).is_ok());
    let mut events = vec![click.unwrap_or(UiEvent::Close)];
    commit_drafts(&w, d, &mut events);
    let mut queue: Vec<Value> = match d.fields.remove(QUEUE_KEY) {
        Some(Value::Array(queue)) => queue,
        _ => Vec::new(),
    };
    let mut events = events.into_iter();
    if queue.is_empty()
        && let Some(first) = events.next()
    {
        let _ = app.session.script_window_answer(first);
    }
    queue.extend(events.map(|e| event_fields(&e)));
    if !queue.is_empty() {
        d.fields.insert(QUEUE_KEY.into(), Value::Array(queue));
    }
}
