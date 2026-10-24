//! `command.batch`: several commands as one undo step (in each document they edit).

use serde_json::{Value, json};

use super::super::*;
use crate::{DocState, EngineError};

pub(super) fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let mut cmds = p.get("commands").and_then(Value::as_array).ok_or_else(|| bad("command.batch", "missing commands"))?.clone();
    let label = str_param(p, "label").unwrap_or("Batch").to_string();
    // The active artboard the app passes, for the steps that act on it.
    let artboard = p.get("artboard").and_then(Value::as_u64);
    // What an error rolls back besides the documents: session-level state a step may change
    // (paint defaults, drawing mode, clipboard, the Untitled-N count, a begun undo group).
    let saved = (s.paint.clone(), s.fill_active, s.draw_mode, s.draw_inside, s.clipboard.clone(), s.untitled_counter, s.explicit_undo_group.clone());
    // A drag or typing still in progress is a step of its own, as before Undo: the batch's steps
    // would otherwise run inside it and take its baseline as theirs (#1697).
    if s.active().is_some() {
        super::super::edit::settle_interaction(s)?;
    }
    // The documents as they are now, whole: what a failed step brings back. Their art, history,
    // file, format and saved state share their data with the open ones until a step changes them.
    let start = s.docs.clone();
    let active = s.active;
    s.batch_stash = Some(vec![]);
    let r = run_steps(s, &label, artboard, &mut cmds);
    let stash = s.batch_stash.take().unwrap_or_default();
    let results = match r {
        Ok(results) => results,
        Err(e) => {
            roll_back(s, start, &stash);
            s.active = active;
            (s.paint, s.fill_active, s.draw_mode, s.draw_inside, s.clipboard, s.untitled_counter, s.explicit_undo_group) = saved;
            s.pending_paint = None;
            return Err(e);
        }
    };
    // Each document the steps edited keeps them as one undo step.
    s.remember_pending_paint();
    for st in &mut s.docs {
        if let Some(it) = st.interaction.take() {
            st.keep_interaction(it);
        }
    }
    // The batch journals itself, whichever document it ends in.
    let mut entry = p.clone();
    if let Some(o) = entry.as_object_mut() {
        o.insert("commands".into(), Value::Array(cmds));
    }
    s.journal.push(("command.batch".into(), entry));
    Ok(json!({ "results": results }))
}

/// Run the steps → their results. Each runs inside an interaction of the document active when it
/// starts (one for all the steps in that document), so a step may open, switch or close documents.
/// A step that acts on the active artboard ([`crate::ON_ACTIVE_ARTBOARD`]) and names none gets
/// `artboard`, as the Artboards panel shows it in the document the step runs in: past that
/// document's last artboard, the last one. A step that resizes the active artboard
/// ([`crate::FIT_ACTIVE_ARTBOARD`]) and names no `index` gets that artboard as its `index`.
fn run_steps(s: &mut Session, label: &str, artboard: Option<u64>, cmds: &mut [Value]) -> Result<Vec<Value>> {
    let mut results = Vec::with_capacity(cmds.len());
    for c in cmds {
        let id = c.get("command").and_then(Value::as_str).unwrap_or("").to_string();
        if id == "command.batch" {
            return Err(bad("command.batch", "batches cannot nest"));
        }
        if s.active().is_some() {
            s.begin_interaction(label)?;
        }
        let mut params = c.get("params").cloned().unwrap_or(json!({}));
        let key = if crate::ON_ACTIVE_ARTBOARD.contains(&id.as_str()) {
            Some("artboard")
        } else {
            crate::FIT_ACTIVE_ARTBOARD.contains(&id.as_str()).then_some("index")
        };
        if let Some(a) = artboard
            && let Some(key) = key
            && params.get(key).is_none()
            && let Some(last) = s.active().and_then(|d| d.doc.artboards.len().checked_sub(1))
            && let Some(o) = params.as_object_mut()
        {
            o.insert(key.into(), json!(usize::try_from(a).map_or(last, |a| a.min(last))));
        }
        let (v, noted) =
            s.execute_step(&id, &params).map_err(|e| EngineError::Other(format!("batch step {} (`{id}`) failed: {e}", results.len())))?;
        // The step's journal entry records what it took from the preferences or the clock (a
        // save's date, a new document's), as a command of its own would.
        if noted != params
            && let Some(c) = c.as_object_mut()
        {
            c.insert("params".into(), noted);
        }
        results.push(v);
    }
    Ok(results)
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
