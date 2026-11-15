//! Edit › Spelling › Check Spelling…: walk through unknown words in the selection or the whole
//! document, displaying the unrecognized word in its sentence context, with suggestions, and
//! Ignore, Ignore All, Change, Change All, Add to Dictionary, and Done / Cancel.
//!
//! Fields:
//! - `not_in_dictionary`: the current unrecognized word
//! - `context`: sentence context containing the unrecognized word
//! - `context_before`, `context_word`, `context_after`: parts of the sentence context
//! - `suggestions`: alternative words suggested by the dictionary
//! - `selected_suggestion`: the suggestion currently selected
//! - `change_to`: the replacement word (from suggestions or typed)
//! - `node_id`, `start`, `end`, `lang`: location of the current misspelling
//! - `ids`: object IDs checked (None: entire document)
//! - `skipped`: list of `[node_id, start]` skipped by Ignore
//! - `done`: whether spell checking has finished (or no unrecognized words found)

use std::collections::BTreeMap;
use std::ops::Range;

use serde_json::{Value, json};

use super::{DialogAction, DialogResult, DialogSpec};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Check Spelling.
pub const KIND: &str = "checkSpelling";

/// The command its menu item runs.
pub const COMMAND: &str = "ui.checkSpelling";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Check Spelling").into(),
    body_with_actions: Some(body),
    confirm,
    ok: None,
    min_width: 480.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Ignore,
    IgnoreAll,
    Change,
    ChangeAll,
    AddToDictionary,
    Done,
    Cancel,
}

/// Open Check Spelling on the active document's type.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let doc_uid = app.session.doc().map_err(|e| e.to_string())?.uid;
    let (ids, remaining_ids): (Option<Vec<u64>>, Vec<u64>) = {
        let st = app.session.doc().map_err(|e| e.to_string())?;
        let selected = &st.selection.objects;
        if !selected.is_empty() {
            let selected_ids: Vec<u64> = selected.iter().map(|n| n.0).collect();
            (Some(selected_ids.clone()), selected_ids)
        } else {
            let mut all_ordered = Vec::new();
            st.doc.walk(|n| {
                if matches!(n.kind, vectorcraft_doc::NodeKind::Text(_)) && st.doc.is_editable(n.id) {
                    all_ordered.push(n.id.0);
                }
            });
            (None, all_ordered)
        }
    };
    let mut d = Dialog::new(
        KIND,
        json!({
            "doc_uid": doc_uid,
            "ids": ids,
            "remaining_ids": remaining_ids,
            "skipped": [],
            "not_in_dictionary": "",
            "context": "",
            "context_before": "",
            "context_word": "",
            "context_after": "",
            "change_to": "",
            "suggestions": [],
            "selected_suggestion": null,
            "node_id": null,
            "start": null,
            "end": null,
            "lang": null,
            "done": false,
        }),
    );
    advance(app, &mut d)?;
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

fn get_skipped(d: &Dialog) -> Vec<(u64, u64)> {
    d.fields
        .get("skipped")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|pair| {
                    let a = pair.as_array()?;
                    if a.len() == 2 { Some((a[0].as_u64()?, a[1].as_u64()?)) } else { None }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn set_skipped(d: &mut Dialog, skipped: &[(u64, u64)]) {
    let arr: Vec<Value> = skipped.iter().map(|&(id, start)| json!([id, start])).collect();
    d.fields.insert("skipped".into(), json!(arr));
}

/// Extracts the sentence containing `range` within `text`, returning (before, word, after).
pub fn extract_sentence_context(text: &str, range: Range<usize>) -> (String, String, String) {
    let empty = (String::new(), String::new(), String::new());
    if range.start > range.end {
        return empty;
    }
    let Some(word_slice) = text.get(range.clone()) else {
        return empty;
    };
    let word = word_slice.to_string();

    let Some(before_slice) = text.get(..range.start) else {
        return empty;
    };
    let nl_start = before_slice.rfind('\n').map(|idx| idx + 1);
    let mut punct_start = None;
    for (idx, ch) in before_slice.char_indices() {
        if (ch == '.' || ch == '!' || ch == '?') && before_slice.get(idx + ch.len_utf8()..).is_some_and(|s| s.starts_with(char::is_whitespace)) {
            punct_start = Some(idx + ch.len_utf8());
        }
    }
    let start_idx = match (nl_start, punct_start) {
        (Some(nl), Some(p)) => nl.max(p),
        (Some(nl), None) => nl,
        (None, Some(p)) => p,
        (None, None) => 0,
    };

    let Some(after_slice) = text.get(range.end..) else {
        return empty;
    };
    let nl_end = after_slice.find('\n');
    let mut punct_end = None;
    for (idx, ch) in after_slice.char_indices() {
        if ch == '.' || ch == '!' || ch == '?' {
            let next_idx = idx + ch.len_utf8();
            if next_idx >= after_slice.len() || after_slice.get(next_idx..).is_some_and(|s| s.starts_with(char::is_whitespace)) {
                punct_end = Some(next_idx);
                break;
            }
        }
    }
    let end_offset = match (nl_end, punct_end) {
        (Some(nl), Some(p)) => nl.min(p),
        (Some(nl), None) => nl,
        (None, Some(p)) => p,
        (None, None) => after_slice.len(),
    };

    let mut before = text.get(start_idx..range.start).unwrap_or("").trim_start().to_string();
    let mut after = after_slice.get(..end_offset).unwrap_or("").trim_end().to_string();

    const MAX_SIDE: usize = 120;
    if before.len() > MAX_SIDE {
        let target = before.len() - MAX_SIDE;
        let split_idx = before.char_indices().map(|(idx, _)| idx).find(|&idx| idx >= target).unwrap_or(before.len());
        let suffix = before.get(split_idx..).unwrap_or("");
        before = match suffix.find(' ') {
            Some(space) => format!("…{}", suffix.get(space..).unwrap_or("")),
            None => format!("…{suffix}"),
        };
    }
    if after.len() > MAX_SIDE {
        let split_idx = after.char_indices().map(|(idx, _)| idx).take_while(|&idx| idx <= MAX_SIDE).last().unwrap_or(0);
        let prefix = after.get(..split_idx).unwrap_or("");
        after = match prefix.rfind(' ') {
            Some(space) => format!("{}…", prefix.get(..space).unwrap_or("")),
            None => format!("{prefix}…"),
        };
    }

    (before, word, after)
}

fn set_done(d: &mut Dialog) {
    d.fields.insert("done".into(), json!(true));
    d.fields.insert("not_in_dictionary".into(), json!(""));
    d.fields.insert("context".into(), json!(tl!("The spell check is complete.")));
    d.fields.insert("context_before".into(), json!(""));
    d.fields.insert("context_word".into(), json!(""));
    d.fields.insert("context_after".into(), json!(""));
    d.fields.insert("suggestions".into(), json!([]));
    d.fields.insert("selected_suggestion".into(), Value::Null);
    d.fields.insert("change_to".into(), json!(""));
    d.fields.insert("node_id".into(), Value::Null);
    d.fields.insert("start".into(), Value::Null);
    d.fields.insert("end".into(), Value::Null);
    d.fields.insert("lang".into(), Value::Null);
}

/// Advance to the next unrecognized word or complete the spell check.
pub fn advance(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let mut remaining_ids: Vec<u64> =
        d.fields.get("remaining_ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();

    if remaining_ids.is_empty() && d.fields.get("ids").is_none_or(Value::is_null) {
        let st = app.session.doc().map_err(|e| e.to_string())?;
        st.doc.walk(|n| {
            if matches!(n.kind, vectorcraft_doc::NodeKind::Text(_)) && st.doc.is_editable(n.id) {
                remaining_ids.push(n.id.0);
            }
        });
        d.fields.insert("remaining_ids".into(), json!(remaining_ids));
    }

    let skipped = get_skipped(d);

    loop {
        if remaining_ids.is_empty() {
            set_done(d);
            return Ok(());
        }

        let check_res = app.run("text.spelling.check", json!({ "ids": remaining_ids })).map_err(|e| e.to_string())?;
        let words = check_res.get("words").and_then(Value::as_array).cloned().unwrap_or_default();
        let more = check_res.get("more").and_then(Value::as_bool).unwrap_or(false);

        let next_word = words
            .iter()
            .find(|w| {
                let id = w.get("id").and_then(Value::as_u64).unwrap_or(0);
                let start = w.get("start").and_then(Value::as_u64).unwrap_or(0);
                !skipped.contains(&(id, start))
            })
            .cloned();

        if let Some(w) = next_word {
            let id = w["id"].as_u64().unwrap_or(0);
            let start = w["start"].as_u64().unwrap_or(0) as usize;
            let end = w["end"].as_u64().unwrap_or(0) as usize;
            let word = w["word"].as_str().unwrap_or("").to_string();
            let lang = w["lang"].as_str().unwrap_or("en").to_string();

            let (before, target, after) = {
                let st = app.session.doc().map_err(|e| e.to_string())?;
                if let Some(vectorcraft_doc::NodeKind::Text(t)) = st.doc.node(vectorcraft_doc::NodeId(id)).map(|n| &n.kind) {
                    extract_sentence_context(&t.plain_text(), start..end)
                } else {
                    (String::new(), word.clone(), String::new())
                }
            };

            let sugg_res = app.run("text.spelling.suggest", json!({ "word": word, "lang": lang, "max": 10 })).map_err(|e| e.to_string())?;
            let suggestions: Vec<String> = sugg_res
                .get("suggestions")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default();

            let change_to = suggestions.first().cloned().unwrap_or_else(|| word.clone());
            let full_context = format!("{before}{target}{after}");

            d.fields.insert("not_in_dictionary".into(), json!(word));
            d.fields.insert("context".into(), json!(full_context));
            d.fields.insert("context_before".into(), json!(before));
            d.fields.insert("context_word".into(), json!(target));
            d.fields.insert("context_after".into(), json!(after));
            d.fields.insert("suggestions".into(), json!(suggestions));
            d.fields.insert("selected_suggestion".into(), json!(suggestions.first().cloned()));
            d.fields.insert("change_to".into(), json!(change_to));
            d.fields.insert("node_id".into(), json!(id));
            d.fields.insert("start".into(), json!(start));
            d.fields.insert("end".into(), json!(end));
            d.fields.insert("lang".into(), json!(lang));
            d.fields.insert("done".into(), json!(false));
            return Ok(());
        }

        if more && !words.is_empty() {
            let last_id = words.last().and_then(|w| w.get("id")).and_then(Value::as_u64).unwrap_or(0);
            if let Some(pos) = remaining_ids.iter().position(|&id| id == last_id)
                && pos > 0
            {
                remaining_ids.drain(0..pos);
                d.fields.insert("remaining_ids".into(), json!(remaining_ids));
                continue;
            }
            if remaining_ids.len() > 1 {
                remaining_ids.remove(0);
                d.fields.insert("remaining_ids".into(), json!(remaining_ids));
                continue;
            }
        }

        set_done(d);
        return Ok(());
    }
}

fn check_doc_uid(app: &VectorcraftApp, d: &Dialog) -> Result<(), String> {
    let doc_uid = d.fields.get("doc_uid").and_then(Value::as_u64);
    let active_uid = app.session.active().map(|st| st.uid);
    if doc_uid.is_some() && doc_uid != active_uid {
        return Err("the document was switched".into());
    }
    Ok(())
}

fn ignore(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let id = d.fields.get("node_id").and_then(Value::as_u64).unwrap_or(0);
    let start = d.fields.get("start").and_then(Value::as_u64).unwrap_or(0);
    let mut skipped = get_skipped(d);
    skipped.push((id, start));
    set_skipped(d, &skipped);
    advance(app, d)
}

fn ignore_all(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let word = d.str("not_in_dictionary");
    if !word.is_empty() {
        app.run("text.spelling.ignoreAll", json!({ "word": word })).map_err(|e| e.to_string())?;
    }
    advance(app, d)
}

fn change(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let id = d.fields.get("node_id").and_then(Value::as_u64).unwrap_or(0);
    let start = d.fields.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
    let end = d.fields.get("end").and_then(Value::as_u64).unwrap_or(0) as usize;
    let expected_word = d.str("not_in_dictionary");
    let change_to = d.str("change_to");
    if !change_to.trim().is_empty() && id > 0 {
        let text_matches = {
            let st = app.session.doc().map_err(|e| e.to_string())?;
            if let Some(vectorcraft_doc::NodeKind::Text(t)) = st.doc.node(vectorcraft_doc::NodeId(id)).map(|n| &n.kind) {
                let plain = t.plain_text();
                plain.get(start..end).map(|s| s == expected_word).unwrap_or(false)
            } else {
                false
            }
        };

        if text_matches {
            app.run("text.editRange", json!({ "id": id, "start": start, "end": end, "insert": change_to })).map_err(|e| e.to_string())?;
        } else {
            return advance(app, d);
        }
    }
    advance(app, d)
}

fn change_all(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let word = d.str("not_in_dictionary");
    let change_to = d.str("change_to");
    if word.is_empty() || change_to.trim().is_empty() {
        return advance(app, d);
    }
    let initial_ids = d.fields.get("ids").cloned().unwrap_or(Value::Null);
    let skipped = get_skipped(d);

    let mut undo_group_started = false;
    let mut execute_changes = || -> Result<(), String> {
        let mut replaced_spans: Vec<(u64, usize, usize)> = Vec::new();
        let mut all_commands = Vec::new();
        loop {
            let check_res = app.run("text.spelling.check", json!({ "ids": initial_ids })).map_err(|e| e.to_string())?;
            let words = check_res.get("words").and_then(Value::as_array).cloned().unwrap_or_default();
            let more = check_res.get("more").and_then(Value::as_bool).unwrap_or(false);

            let mut by_node: BTreeMap<u64, Vec<(usize, usize)>> = BTreeMap::new();
            let mut new_edits = 0;
            for w in &words {
                let id = w.get("id").and_then(Value::as_u64).unwrap_or(0);
                let start = w.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
                let end = w.get("end").and_then(Value::as_u64).unwrap_or(0) as usize;
                let w_str = w.get("word").and_then(Value::as_str).unwrap_or("");
                if skipped.contains(&(id, start as u64)) {
                    continue;
                }
                if replaced_spans.iter().any(|&(r_id, r_start, r_end)| r_id == id && start >= r_start && start < r_end) {
                    continue;
                }
                if w_str.eq_ignore_ascii_case(&word) {
                    by_node.entry(id).or_default().push((start, end));
                    new_edits += 1;
                }
            }

            if new_edits == 0 {
                break;
            }

            let new_len = change_to.len() as isize;
            for (r_id, r_start, r_end) in &mut replaced_spans {
                if let Some(ranges) = by_node.get(r_id) {
                    let delta: isize = ranges.iter().filter(|(_, end)| *end <= *r_start).map(|(start, end)| new_len - (*end - *start) as isize).sum();
                    *r_start = (*r_start as isize + delta).max(0) as usize;
                    *r_end = (*r_end as isize + delta).max(0) as usize;
                }
            }
            for (&id, ranges) in &by_node {
                for &(start, _) in ranges {
                    let delta: isize = ranges
                        .iter()
                        .filter(|(_, other_end)| *other_end <= start)
                        .map(|(other_start, other_end)| new_len - (*other_end - *other_start) as isize)
                        .sum();
                    let post_start = (start as isize + delta).max(0) as usize;
                    let post_end = post_start + change_to.len();
                    replaced_spans.push((id, post_start, post_end));
                }
            }

            let mut batch_commands = Vec::new();
            for (id, mut ranges) in by_node {
                ranges.sort_by_key(|a| std::cmp::Reverse(a.0));
                for (start, end) in ranges {
                    batch_commands.push(json!({
                        "command": "text.editRange",
                        "params": {
                            "id": id,
                            "start": start,
                            "end": end,
                            "insert": change_to,
                        }
                    }));
                }
            }

            if !batch_commands.is_empty() {
                if batch_commands.len() == 1 && !more && all_commands.is_empty() && !undo_group_started {
                    let cmd = &batch_commands[0];
                    app.run(cmd["command"].as_str().unwrap_or("text.editRange"), cmd["params"].clone()).map_err(|e| e.to_string())?;
                    break;
                } else if more {
                    if !undo_group_started {
                        app.run("edit.undoGroup.begin", json!({ "label": "Change All" })).map_err(|e| e.to_string())?;
                        undo_group_started = true;
                    }
                    app.run("command.batch", json!({ "label": "Change All", "commands": batch_commands })).map_err(|e| e.to_string())?;
                    continue;
                } else {
                    all_commands.extend(batch_commands);
                    break;
                }
            }

            break;
        }

        if !all_commands.is_empty() {
            if all_commands.len() == 1 && !undo_group_started {
                let cmd = &all_commands[0];
                app.run(cmd["command"].as_str().unwrap_or("text.editRange"), cmd["params"].clone()).map_err(|e| e.to_string())?;
            } else {
                app.run("command.batch", json!({ "label": "Change All", "commands": all_commands })).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    };

    let res = execute_changes();
    if undo_group_started {
        let cancel = res.is_err();
        let _ = app.run("edit.undoGroup.end", json!({ "cancel": cancel }));
    }
    res?;
    advance(app, d)
}

fn add_to_dictionary(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let word = d.str("not_in_dictionary");
    if !word.is_empty() {
        app.run("text.spelling.addWord", json!({ "word": word })).map_err(|e| e.to_string())?;
    }
    advance(app, d)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let doc_uid = d.fields.get("doc_uid").and_then(Value::as_u64);
    let active_uid = app.session.active().map(|st| st.uid);
    if doc_uid.is_some() && doc_uid != active_uid {
        return DialogAction::Cancel;
    }

    if let Some(act) = d.fields.remove("action").and_then(|v| v.as_str().map(String::from)) {
        let res = match act.as_str() {
            "ignore" => ignore(app, d),
            "ignoreAll" => ignore_all(app, d),
            "change" => change(app, d),
            "changeAll" => change_all(app, d),
            "addToDictionary" => add_to_dictionary(app, d),
            "done" | "cancel" => return DialogAction::Cancel,
            _ => Ok(()),
        };
        if let Err(e) = res {
            app.status(&e);
            d.fields.insert("error".into(), json!(e));
        } else {
            d.fields.remove("error");
        }
    }

    let mut action_to_run = None;
    let done = d.bool("done");
    let before = d.str("context_before");
    let target = d.str("context_word");
    let after = d.str("context_after");
    let mut change_to = d.str("change_to");
    let suggestions: Vec<String> = d
        .fields
        .get("suggestions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();

    let can_change = !done && !change_to.trim().is_empty();

    if crate::shortcuts::pressed_anew(ui.ctx(), egui::Key::Enter)
        || ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, .. })))
    {
        if can_change {
            action_to_run = Some(Action::Change);
        } else if done {
            return DialogAction::Cancel;
        }
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        return DialogAction::Cancel;
    }

    let action_col_width = 110.0;
    let col_spacing = 16.0;
    let text_col_width = (ui.available_width() - action_col_width - col_spacing).clamp(200.0, 320.0);

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(text_col_width);
            if done {
                ui.add_space(20.0);
                widgets::dim_label(ui, tl!("The spell check is complete."));
                ui.add_space(80.0);
            } else {
                widgets::dim_label(ui, tl!("Not in Dictionary:"));
                ui.group(|ui| {
                    ui.set_width(text_col_width);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if !before.is_empty() {
                            ui.label(&before);
                        }
                        let t = Tokens::get(ui.ctx());
                        ui.add(egui::Label::new(egui::RichText::new(&target).color(t.accent).strong().underline()));
                        if !after.is_empty() {
                            ui.label(&after);
                        }
                    });
                });

                ui.add_space(8.0);
                widgets::dim_label(ui, tl!("Change to:"));
                let field_id = ui.scope_id().with(("dlg-text", "change_to"));
                widgets::take_dialog_focus(ui, field_id, &change_to);
                if ui.add(egui::TextEdit::singleline(&mut change_to).id(field_id).desired_width(text_col_width)).changed() {
                    d.fields.insert("change_to".into(), json!(change_to));
                }

                ui.add_space(8.0);
                widgets::dim_label(ui, tl!("Suggestions:"));
                widgets::list_box(ui, |ui| {
                    ui.set_width(text_col_width);
                    let row = ui.spacing().interact_size.y;
                    egui::ScrollArea::vertical().id_salt("check-spelling-suggestions").min_scrolled_height(140.0).max_height(140.0).show_rows(
                        ui,
                        row,
                        suggestions.len(),
                        |ui, range| {
                            ui.set_width(ui.available_width());
                            for s in suggestions.get(range).unwrap_or_default() {
                                let is_selected = change_to == *s;
                                if ui.selectable_label(is_selected, s.as_str()).clicked() {
                                    d.fields.insert("change_to".into(), json!(s));
                                    d.fields.insert("selected_suggestion".into(), json!(s));
                                }
                            }
                        },
                    );
                });
            }
        });

        ui.add_space(col_spacing);

        ui.vertical(|ui| {
            let w = action_col_width;
            if ui.add_enabled_ui(!done, |ui| widgets::flat_button_fit(ui, tl!("Ignore"), w).clicked()).inner {
                action_to_run = Some(Action::Ignore);
            }
            ui.add_space(6.0);
            if ui.add_enabled_ui(!done, |ui| widgets::flat_button_fit(ui, tl!("Ignore All"), w).clicked()).inner {
                action_to_run = Some(Action::IgnoreAll);
            }
            ui.add_space(6.0);
            if ui.add_enabled_ui(can_change, |ui| widgets::flat_button_fit(ui, tl!("Change"), w).clicked()).inner {
                action_to_run = Some(Action::Change);
            }
            ui.add_space(6.0);
            if ui.add_enabled_ui(can_change, |ui| widgets::flat_button_fit(ui, tl!("Change All"), w).clicked()).inner {
                action_to_run = Some(Action::ChangeAll);
            }
            ui.add_space(6.0);
            if ui.add_enabled_ui(!done, |ui| widgets::flat_button_fit(ui, tl!("Add to Dictionary"), w).clicked()).inner {
                action_to_run = Some(Action::AddToDictionary);
            }
            ui.add_space(16.0);
            if widgets::flat_button_fit(ui, tl!("Done"), w).clicked() {
                action_to_run = Some(Action::Done);
            }
            ui.add_space(6.0);
            if widgets::flat_button_fit(ui, tl!("Cancel"), w).clicked() {
                action_to_run = Some(Action::Cancel);
            }
        });
    });

    if let Some(act) = action_to_run {
        let res = match act {
            Action::Ignore => ignore(app, d),
            Action::IgnoreAll => ignore_all(app, d),
            Action::Change => change(app, d),
            Action::ChangeAll => change_all(app, d),
            Action::AddToDictionary => add_to_dictionary(app, d),
            Action::Done | Action::Cancel => return DialogAction::Cancel,
        };
        if let Err(e) = res {
            app.status(&e);
            d.fields.insert("error".into(), json!(e));
        } else {
            d.fields.remove("error");
        }
    }

    DialogAction::None
}

fn confirm(app: &mut VectorcraftApp, _: &Dialog) -> DialogResult {
    app.ui.dialog = None;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, Pos2, Rect, vec2};
    use serde_json::json;

    use crate::VectorcraftApp;

    const WORD_A: &str = "zqvspella";
    const WORD_B: &str = "zqvspellb";
    const WORD_C: &str = "zqvspellc";

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app
    }

    fn text(app: &mut VectorcraftApp, y: f64, text: &str) -> u64 {
        app.run("text.create", json!({"x": 10, "y": y, "text": text})).unwrap()["id"].as_u64().unwrap()
    }

    fn forget(app: &mut VectorcraftApp, words: &[&str]) {
        for w in words {
            let _ = app.run("text.spelling.removeWord", json!({ "word": w }));
        }
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    }

    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
        let time = ctx.input(|i| i.time) + 0.25;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let raw = egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
        fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                _ => {}
            }
        }
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    }

    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) -> Vec<(String, Rect)> {
        frame(app, ctx, vec![]);
        let texts = frame(app, ctx, vec![]);
        let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, ctx, vec![Event::PointerMoved(at)]);
        frame(app, ctx, vec![press(true)]);
        frame(app, ctx, vec![press(false)]);
        frame(app, ctx, vec![])
    }

    fn key_press(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key) {
        let down = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(app, ctx, vec![down]);
        let up = Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(app, ctx, vec![up]);
    }

    #[test]
    fn test_extract_sentence_context() {
        let text = "First sentence. Here is teh sample typo! Third sentence.";
        let (before, word, after) = super::extract_sentence_context(text, 24..27);
        assert_eq!(word, "teh");
        assert_eq!(before, "Here is ");
        assert_eq!(after, " sample typo!");

        let single = "Hello world";
        let (b, w, a) = super::extract_sentence_context(single, 6..11);
        assert_eq!(w, "world");
        assert_eq!(b, "Hello ");
        assert_eq!(a, "");

        #[allow(clippy::reversed_empty_ranges)]
        let (b_inv, w_inv, a_inv) = super::extract_sentence_context(single, 10..5);
        assert!(w_inv.is_empty() && b_inv.is_empty() && a_inv.is_empty());

        // Multibyte UTF-8 regression test (e.g. 61 copies of 'é', which is 122 bytes)
        let after_multibyte = format!("word {}", "é".repeat(70));
        let (b_mb, w_mb, a_mb) = super::extract_sentence_context(&after_multibyte, 0..4);
        assert_eq!(w_mb, "word");
        assert_eq!(b_mb, "");
        assert!(a_mb.ends_with('…'));

        let before_multibyte = format!("{} typo end.", "é".repeat(70));
        let typo_start = "é".repeat(70).len() + 1;
        let typo_end = typo_start + 4;
        let (b_mb2, w_mb2, a_mb2) = super::extract_sentence_context(&before_multibyte, typo_start..typo_end);
        assert_eq!(w_mb2, "typo");
        assert!(b_mb2.starts_with('…'));
        assert_eq!(a_mb2, " end.");

        // Stale / out-of-bounds or non-char boundary offsets never panic
        let (b_oob, w_oob, a_oob) = super::extract_sentence_context(single, 5..50);
        assert!(w_oob.is_empty() && b_oob.is_empty() && a_oob.is_empty());

        let utf8_sample = "Héllo world";
        let (b_mid, w_mid, a_mid) = super::extract_sentence_context(utf8_sample, 2..4);
        assert!(w_mid.is_empty() && b_mid.is_empty() && a_mid.is_empty());
    }

    #[test]
    fn menu_opens_check_spelling_dialog() {
        let mut app = app();
        text(&mut app, 50.0, &format!("Hello {WORD_A} world"));

        let e = crate::menus::menu_entries(&app).into_iter().find(|e| e.label == "Check Spelling…").expect("in Edit menu");
        assert_eq!(e.command.as_deref(), Some(super::COMMAND));
        assert!(e.enabled);

        crate::menus::invoke(&mut app, super::COMMAND, json!({}));
        let d = app.ui.dialog.as_ref().expect("Check Spelling dialog opens");
        assert_eq!(d.kind, super::KIND);
        assert_eq!(d.str("not_in_dictionary"), WORD_A);
        assert!(d.str("context").contains(WORD_A));
        assert!(!d.bool("done"));
    }

    #[test]
    fn check_spelling_word_replacement_and_undo_redo() {
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));

        super::open(&mut app).unwrap();
        let ctx = context();

        // Change replacement word
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("replacement"));
        click(&mut app, &ctx, "Change");

        // Verify word replacement in the document
        let plain = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain, "Prefix replacement suffix");

        // Verify one-step undo brings the original word back
        app.run("edit.undo", json!({})).unwrap();
        let plain_undone = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain_undone, format!("Prefix {WORD_A} suffix"));

        // Verify redo re-applies the replacement
        app.run("edit.redo", json!({})).unwrap();
        let plain_redone = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain_redone, "Prefix replacement suffix");
    }

    #[test]
    fn check_spelling_change_all_in_one_undo_step() {
        let mut app = app();
        let a = text(&mut app, 50.0, &format!("First {WORD_B} here and {WORD_B} again"));
        let b = text(&mut app, 100.0, &format!("Second {WORD_B} line"));
        app.run("select.none", json!({})).unwrap();

        super::open(&mut app).unwrap();
        let ctx = context();

        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("replaced"));
        click(&mut app, &ctx, "Change All");

        let plain_a = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(a)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        let plain_b = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(b)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain_a, "First replaced here and replaced again");
        assert_eq!(plain_b, "Second replaced line");

        // Single undo reverts all changes made by Change All
        app.run("edit.undo", json!({})).unwrap();
        let plain_a_undone = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(a)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        let plain_b_undone = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(b)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain_a_undone, format!("First {WORD_B} here and {WORD_B} again"));
        assert_eq!(plain_b_undone, format!("Second {WORD_B} line"));
    }

    #[test]
    fn check_spelling_dictionary_addition() {
        let mut app = app();
        forget(&mut app, &[WORD_C]);
        text(&mut app, 50.0, &format!("Look at {WORD_C} here"));

        super::open(&mut app).unwrap();
        let ctx = context();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), WORD_C);

        click(&mut app, &ctx, "Add to Dictionary");

        // Verify WORD_C was added to user dictionary
        let dict = app.run("text.spelling.dictionary", json!({})).unwrap();
        let words = dict["words"].as_array().unwrap();
        assert!(words.contains(&json!(WORD_C)));

        // Dialog should now have completed because no other unknown words exist
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));

        forget(&mut app, &[WORD_C]);
    }

    #[test]
    fn check_spelling_ignore_and_ignore_all() {
        let mut app = app();
        text(&mut app, 50.0, &format!("{WORD_A} then {WORD_B}"));

        super::open(&mut app).unwrap();
        let ctx = context();

        // First word is WORD_A
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), WORD_A);

        // Clicking Ignore moves to WORD_B without modifying the document or ignore list
        click(&mut app, &ctx, "Ignore");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), WORD_B);
        assert!(!app.session.doc().unwrap().spelling_ignored.contains(WORD_A));

        // Clicking Ignore All adds WORD_B to the document's spelling_ignored list
        click(&mut app, &ctx, "Ignore All");
        assert!(app.session.doc().unwrap().spelling_ignored.contains(WORD_B));
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));
    }

    #[test]
    fn check_spelling_keyboard_navigation() {
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("Item {WORD_A} text"));

        super::open(&mut app).unwrap();
        let ctx = context();

        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));

        // Enter triggers Change
        key_press(&mut app, &ctx, egui::Key::Enter);

        let plain = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain, "Item fixed text");
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));

        // When done, Enter closes the dialog
        key_press(&mut app, &ctx, egui::Key::Enter);
        assert!(app.ui.dialog.is_none(), "Enter on done closes dialog");

        // Enter when change_to is empty and review is unfinished leaves dialog open
        text(&mut app, 100.0, &format!("More {WORD_A}"));
        super::open(&mut app).unwrap();
        assert!(!app.ui.dialog.as_ref().unwrap().bool("done"));
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!(""));
        key_press(&mut app, &ctx, egui::Key::Enter);
        assert!(app.ui.dialog.is_some(), "Enter with empty change_to leaves dialog open");

        // Test Escape closes dialog
        key_press(&mut app, &ctx, egui::Key::Escape);
        assert!(app.ui.dialog.is_none(), "Escape closes dialog");
    }

    #[test]
    fn check_spelling_selected_group_finds_nested_text() {
        let mut app = app();
        let t = text(&mut app, 50.0, &format!("Grouped {WORD_A} item"));
        app.run("select.set", json!({ "ids": [t] })).unwrap();
        let g = app.run("object.group", json!({})).unwrap();
        let group_id = g["id"].as_u64().unwrap();
        app.run("select.set", json!({ "ids": [group_id] })).unwrap();

        super::open(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().expect("dialog opens");
        assert_eq!(d.str("not_in_dictionary"), WORD_A);
    }

    #[test]
    fn check_spelling_when_no_misspellings() {
        let mut app = app();
        text(&mut app, 50.0, "All correctly spelled English words here");

        super::open(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().expect("dialog opens");
        assert!(d.bool("done"));
        assert!(d.str("context").contains("The spell check is complete."));

        let ctx = context();
        click(&mut app, &ctx, "Done");
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn check_spelling_refused_on_document_switch() {
        let mut app = app();
        text(&mut app, 50.0, &format!("Text {WORD_A} here"));

        super::open(&mut app).unwrap();
        assert!(app.ui.dialog.is_some());

        // Create a new document, switching active document UID
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();

        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        assert!(app.ui.dialog.is_none(), "switching documents closes spelling dialog");
    }

    #[test]
    fn test_extract_sentence_context_multiline() {
        let text = "First sentence.\nSecond line with teh typo.\nThird line ends.";
        let range = text.find("teh").map(|idx| idx..idx + 3).unwrap();
        let (before, word, after) = super::extract_sentence_context(text, range);
        assert_eq!(word, "teh");
        assert_eq!(before, "Second line with ");
        assert_eq!(after, " typo.");
        assert!(!before.contains("First"));
        assert!(!after.contains("Third"));

        let text2 = "Far before. Near before.\nLine with teh typo. Near after.\nFar after.";
        let range2 = text2.find("teh").map(|idx| idx..idx + 3).unwrap();
        let (b2, w2, a2) = super::extract_sentence_context(text2, range2);
        assert_eq!(w2, "teh");
        assert_eq!(b2, "Line with ");
        assert_eq!(a2, " typo.");
        assert!(!b2.contains("Near before"));
        assert!(!a2.contains("Near after"));
    }

    #[test]
    fn check_spelling_ignores_uneditable_text() {
        let mut app = app();
        let editable_id = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));
        let locked_id = text(&mut app, 100.0, &format!("Prefix {WORD_B} suffix"));

        // Lock the second text object
        app.run("select.set", json!({ "ids": [locked_id] })).unwrap();
        app.run("object.lock", json!({})).unwrap();
        app.run("select.set", json!({ "ids": [] })).unwrap();

        super::open(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.str("not_in_dictionary"), WORD_A);

        let rem = d.fields.get("remaining_ids").and_then(serde_json::Value::as_array).unwrap();
        assert!(rem.iter().any(|v| v.as_u64() == Some(editable_id)));
        assert!(!rem.iter().any(|v| v.as_u64() == Some(locked_id)));
    }

    #[test]
    fn check_spelling_change_all_containing_target_word_terminates() {
        let mut app = app();
        let t1 = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));

        super::open(&mut app).unwrap();
        assert!(app.ui.dialog.is_some());

        // Replace WORD_A with something containing WORD_A
        let ctx = context();
        let replacement = format!("{WORD_A}abc");
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!(replacement));

        click(&mut app, &ctx, "Change All");
        // Change All successfully replaced WORD_A; advance moves to the new word
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), format!("{WORD_A}abc"));

        let st = app.session.doc().unwrap();
        let t1_text = match &st.doc.node(vectorcraft_doc::NodeId(t1)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        };
        assert!(t1_text.contains(&format!("{WORD_A}abc")));
    }
}
