//! `command.batch`: several commands as one undo step (in each document they edit).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::Document;

use super::super::{Result, Session, bad, str_param};
use crate::{DocState, EngineError};

pub(super) fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let mut cmds = p.get("commands").and_then(Value::as_array).ok_or_else(|| bad("command.batch", "missing commands"))?.clone();
    let label = str_param(p, "label").unwrap_or("Batch").to_string();
    // The active artboard the app passes, for the steps that act on it.
    let artboard = p.get("artboard").and_then(Value::as_u64);
    // What an error rolls back besides the documents: session-level state a step may change
    // (paint defaults, drawing mode, clipboard, the Untitled-N count, a begun undo group, the
    // Concept to Vector jobs).
    // A drag or typing still in progress is a step of its own, as before Undo: the batch's steps
    // would otherwise run inside it and take its baseline as theirs (#1697).
    if s.active().is_some() && !s.previewing {
        super::super::edit::settle_interaction(s)?;
    }
    let saved = (s.paint.clone(), s.fill_active, s.draw_mode, s.draw_inside, s.clipboard.clone(), s.untitled_counter, s.explicit_undo_group.clone());
    let concept_jobs = s.concept_jobs_snapshot();
    // The documents as they are now, whole: what a failed step brings back. Their art, history,
    // file, format and saved state share their data with the open ones until a step changes them.
    let start = s.docs.clone();
    let active = s.active;
    s.batch_stash = Some(vec![]);
    let mut grouped = Vec::with_capacity(cmds.len());
    // The Type tool keeps what the steps' Undo and Redo change in it, to put back on a rollback.
    s.notify_text_tool("batch.begin");
    let r = run_steps(s, &label, artboard, &mut cmds, &mut grouped);
    let stash = s.batch_stash.take().unwrap_or_default();
    let results = match r {
        Ok(results) => {
            s.notify_text_tool("batch.end");
            s.batch_steps = Some(grouped);
            s.prune_concept_jobs();
            results
        }
        Err(e) => {
            roll_back(s, start, &stash);
            s.notify_text_tool("batch.rollBack");
            s.active = active;
            s.sync_front_profiles();
            (s.paint, s.fill_active, s.draw_mode, s.draw_inside, s.clipboard, s.untitled_counter, s.explicit_undo_group) = saved;
            s.pending_paint = None;
            s.restore_concept_jobs(concept_jobs);
            return Err(e);
        }
    };
    // Each document the steps edited keeps them as one undo step.
    if !s.previewing {
        s.remember_pending_paint();
        for st in &mut s.docs {
            if let Some(mut it) = st.interaction.take() {
                // Redo restores the new-art colors the whole batch left, not the ones right after a
                // color-mode step.
                if let Some((_, after)) = &mut it.color_mode_paints {
                    after.clone_from(&s.paint);
                    s.batch_paint_docs.push(st.uid);
                }
                st.keep_interaction(it);
            }
        }
    }
    // The batch journals itself, whichever document it ends in.
    let mut entry = p.clone();
    if let Some(o) = entry.as_object_mut() {
        o.insert("commands".into(), Value::Array(cmds));
    }
    if !s.previewing {
        s.journal.push(("command.batch".into(), entry));
    }
    Ok(json!({ "results": results }))
}

/// Run the steps → their results. Each runs inside an interaction of the document active when it
/// starts (one for all the steps in that document), so a step may open, switch or close documents.
/// A step that acts on the active artboard ([`crate::ON_ACTIVE_ARTBOARD`]) and names none gets
/// `artboard`, as the Artboards panel shows it in the document the step runs in: past that
/// document's last artboard, the last one; a Paste in Place, in Front or in Back step that names
/// `artboards` keeps them, and one that names neither, while the Artboard tool is chosen with
/// several artboards selected, pastes on those. A Fit to Artwork Bounds step that names neither
/// `index` nor `indices`, while the Artboard tool is chosen with two or more artboards selected when
/// the step runs, gets those as its `indices`. Otherwise a step that resizes the active artboard
/// ([`crate::FIT_ACTIVE_ARTBOARD`]) and names no `index` (nor `indices`, for Fit to Artwork Bounds)
/// gets that artboard as its `index`. A Fit to Artwork Bounds step that still names neither gets
/// `index` 0, the command's default. A Fit to Artwork Bounds step's journal entry records the
/// `indices` or `index` it got.
/// `grouped` gets each step's undo groups, as [`Session::batch_steps`] keeps them.
fn run_steps(
    s: &mut Session,
    label: &str,
    artboard: Option<u64>,
    cmds: &mut [Value],
    grouped: &mut Vec<(Vec<u64>, Option<usize>)>,
) -> Result<Vec<Value>> {
    let mut results = Vec::with_capacity(cmds.len());
    for c in cmds {
        let id = c.get("command").and_then(Value::as_str).unwrap_or("").to_string();
        if id == "command.batch" {
            return Err(bad("command.batch", "batches cannot nest"));
        }
        if id == "prefs.import" {
            return Err(bad("command.batch", "`prefs.import` runs on its own, not as a batch step"));
        }
        if s.active().is_some() {
            s.begin_interaction(label)?;
        }
        // `"params": null` reads as no params, so the artboards below can still be added.
        let mut params = c.get("params").filter(|v| !v.is_null()).cloned().unwrap_or(json!({}));
        let key = if crate::ON_ACTIVE_ARTBOARD.contains(&id.as_str()) {
            Some("artboard")
        } else {
            crate::FIT_ACTIVE_ARTBOARD.contains(&id.as_str()).then_some("index")
        };
        // A paste step that names its `artboards` keeps them; one that names neither key, while the
        // Artboard tool is chosen with several artboards selected, pastes on those.
        let names_artboards = id.starts_with("edit.pasteIn")
            && (params.get("artboards").is_some() || (params.get("artboard").is_none() && !crate::cmd::edit::artboard_tool_picks(s).is_empty()));
        // A Fit to Artwork Bounds step that names neither `index` nor `indices`, while the Artboard
        // tool is chosen with two or more artboards selected, fits those.
        let fit_unnamed = id == "artboard.fitToArt" && params.get("index").is_none() && params.get("indices").is_none();
        let fit_picks = if fit_unnamed { crate::cmd::edit::artboard_tool_picks(s) } else { Vec::new() };
        if !fit_picks.is_empty()
            && let Some(o) = params.as_object_mut()
        {
            o.insert("indices".into(), json!(fit_picks));
        }
        if let Some(a) = artboard
            && let Some(key) = key
            && params.get(key).is_none()
            && !(id == "artboard.fitToArt" && params.get("indices").is_some())
            && !names_artboards
            && let Some(last) = s.active().and_then(|d| d.doc.artboards.len().checked_sub(1))
            && let Some(o) = params.as_object_mut()
        {
            o.insert(key.into(), json!(usize::try_from(a).map_or(last, |a| a.min(last))));
        }
        // With neither, the step fits the command's default, the first artboard, named here so the
        // journal records it.
        if fit_unnamed
            && params.get("index").is_none()
            && params.get("indices").is_none()
            && let Some(o) = params.as_object_mut()
        {
            o.insert("index".into(), json!(0));
        }
        // The documents with an undo group open, held so that the step's first edit in one copies it.
        let start = s.active().map(|d| d.uid);
        let held: Vec<(u64, Arc<Document>)> = s.docs.iter().filter(|st| st.undo_group.is_some()).map(|st| (st.uid, st.doc.clone())).collect();
        let (v, noted) =
            s.execute_step(&id, &params).map_err(|e| EngineError::Other(format!("batch step {} (`{id}`) failed: {e}", results.len())))?;
        if s.previewing && s.active().map(|d| d.uid) != start {
            return Err(bad("command.batch", "a batch preview cannot switch documents"));
        }
        grouped.push(step_groups(s, start, &held));
        // The step's journal entry records what it took from the preferences or the clock (a
        // save's date, a new document's), as a command of its own would, and the artboards a Fit to
        // Artwork Bounds step took from the Artboard tool, the batch or the command's default, so a
        // replay does not fit the artboards the Artboard tool has selected then.
        if (noted != params || fit_unnamed)
            && let Some(c) = c.as_object_mut()
        {
            c.insert("params".into(), noted);
        }
        results.push(v);
    }
    Ok(results)
}

/// The undo groups a step belongs to, as [`Session::execute`] notes a command's journal entry in
/// them: the group of each document in `held` (the grouped documents before the step) that the step
/// changed, each group it opened and edited, and the group of the document with [`DocState::uid`]
/// `start` when it started there and ended there. With the index of the document it left active
/// when it changed the active document.
fn step_groups(s: &Session, start: Option<u64>, held: &[(u64, Arc<Document>)]) -> (Vec<u64>, Option<usize>) {
    let end = s.active().map(|d| d.uid);
    let uids = s
        .docs
        .iter()
        .filter(|st| {
            st.undo_group.as_ref().is_some_and(|g| {
                let edited = match held.iter().find(|(uid, _)| *uid == st.uid) {
                    Some((_, d)) => !Arc::ptr_eq(d, &st.doc),
                    None => g.first.is_some(),
                };
                edited || (start == Some(st.uid) && end == start)
            })
        })
        .map(|st| st.uid)
        .collect();
    (uids, if end == start { None } else { s.active })
}

/// The documents as they were before the batch (`start`), whatever the steps did to them: opened
/// ones close, closed or replaced ones (`stash`) come back, and each has its art, selection,
/// history, file, format and saved state again (#1697: undoing the batch's interaction alone left
/// a Save As step's new file and an Undo step's history in place). Files the steps wrote stay
/// written. Lock Guides, Lock Slices and Hide Slices stay as the steps left them, and the guides
/// and slices they keep from being selected are deselected.
fn roll_back(s: &mut Session, start: Vec<DocState>, stash: &[DocState]) {
    let open = std::mem::take(&mut s.docs);
    s.docs = start;
    for st in &mut s.docs {
        // Past the newest revision the document reached, so the restored one redraws.
        let newest = open.iter().chain(stash).filter(|d| d.uid == st.uid).map(|d| d.revision).max().unwrap_or(st.revision);
        st.revision = newest.saturating_add(1);
        crate::deselect_hidden(&s.menu, &st.doc, &mut st.selection);
    }
}
