//! Window › New Window: more views of an open document, each with its own zoom, center, rotation,
//! artboard and Outline or Preview, shown as tabs titled with the view's number ("Name:2"). Tile,
//! Cascade, Float in Window, Float All in Windows and Consolidate All Windows arrange each view as
//! they arrange a document. Every view shows the same document, so an edit made in one shows in
//! all of them. Closing a view leaves the document open until its last view closes.
//!
//! A window is a document's index and its view's number ([`Win`]). The view a document shows
//! while it is the current one is in [`VectorcraftApp::views`]; its other views are kept in
//! [`Views`].

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::state::View;

/// A window: an open document's index and its view's number (1 for a document's first view).
pub(crate) type Win = (usize, u32);

/// A view of an open document other than the one [`VectorcraftApp::views`] holds for it.
#[derive(Clone, Debug)]
struct OtherView {
    /// The document ([`vectorcraft_engine::DocState::uid`]).
    uid: u64,
    /// The view's number.
    number: u32,
    view: View,
}

/// The views of the open documents beyond the one each document shows.
#[derive(Default)]
pub struct Views {
    others: Vec<OtherView>,
    /// By document uid: the number of the view in [`VectorcraftApp::views`] (1 when absent).
    shown: HashMap<u64, u32>,
}

fn uid(app: &VectorcraftApp, i: usize) -> Option<u64> {
    app.session.documents().get(i).map(|d| d.uid)
}

/// The number of the view of document `i` that [`VectorcraftApp::views`] holds.
pub(crate) fn shown(app: &VectorcraftApp, i: usize) -> u32 {
    uid(app, i).and_then(|u| app.canvas.windows.views.shown.get(&u).copied()).unwrap_or(1)
}

/// The numbers of document `i`'s views, in order; none for a document that is not open.
pub(crate) fn numbers(app: &VectorcraftApp, i: usize) -> Vec<u32> {
    let Some(u) = uid(app, i) else { return vec![] };
    let mut n: Vec<u32> =
        std::iter::once(shown(app, i)).chain(app.canvas.windows.views.others.iter().filter(|o| o.uid == u).map(|o| o.number)).collect();
    n.sort_unstable();
    n
}

/// Every window, by document and then by view number: the tab order.
pub(crate) fn windows(app: &VectorcraftApp) -> Vec<Win> {
    (0..app.session.documents().len()).flat_map(|i| numbers(app, i).into_iter().map(move |n| (i, n))).collect()
}

/// The current window: the current document's shown view.
pub(crate) fn current(app: &VectorcraftApp) -> Option<Win> {
    app.session.active_index().map(|i| (i, shown(app, i)))
}

/// Window `w`'s key, which stays the same while documents before it close: its document's uid and
/// its view's number.
pub(crate) fn key(app: &VectorcraftApp, (i, n): Win) -> Option<(u64, u32)> {
    uid(app, i).map(|u| (u, n))
}

/// The window `key` ([`key`]) names, while its document is open.
pub(crate) fn find(app: &VectorcraftApp, (u, n): (u64, u32)) -> Option<Win> {
    let i = app.session.documents().iter().position(|d| d.uid == u)?;
    numbers(app, i).contains(&n).then_some((i, n))
}

/// Window `w`'s view.
pub(crate) fn view(app: &VectorcraftApp, (i, n): Win) -> Option<&View> {
    if n == shown(app, i) {
        return app.views.get(i);
    }
    let u = uid(app, i)?;
    app.canvas.windows.views.others.iter().find(|o| o.uid == u && o.number == n).map(|o| &o.view)
}

/// Window `w`'s view, to change.
pub(crate) fn view_mut(app: &mut VectorcraftApp, (i, n): Win) -> Option<&mut View> {
    if n == shown(app, i) {
        return app.views.get_mut(i);
    }
    let u = uid(app, i)?;
    app.canvas.windows.views.others.iter_mut().find(|o| o.uid == u && o.number == n).map(|o| &mut o.view)
}

/// Unfit every window's view: each fits where it shows next.
pub(crate) fn unfit_all(app: &mut VectorcraftApp) {
    for v in &mut app.views {
        v.fitted = false;
    }
    for o in &mut app.canvas.windows.views.others {
        o.view.fitted = false;
    }
}

/// The number a window's title shows: its view's while its document has more than one view.
pub(crate) fn label(app: &VectorcraftApp, (i, n): Win) -> Option<u32> {
    (numbers(app, i).len() > 1).then_some(n)
}

/// Make view `n` the one [`VectorcraftApp::views`] holds for document `i`. False when the document
/// has no such view. While document `i` is the current one, the tool is reset first, as a switch to
/// another document resets it, and with the Artboard tool the artboard the view shows becomes the
/// tool's target.
fn bring(app: &mut VectorcraftApp, (i, n): Win) -> bool {
    let old = shown(app, i);
    if n == old {
        return app.views.get(i).is_some();
    }
    let Some(u) = uid(app, i) else { return false };
    if app.views.get(i).is_none() || !app.canvas.windows.views.others.iter().any(|o| o.uid == u && o.number == n) {
        return false;
    }
    if app.session.active_index() == Some(i) {
        app.session.reset_tool();
    }
    let views = &mut app.canvas.windows.views;
    let (Some(other), Some(shown)) = (views.others.iter_mut().find(|o| o.uid == u && o.number == n), app.views.get_mut(i)) else {
        return false;
    };
    std::mem::swap(shown, &mut other.view);
    other.number = old;
    views.shown.insert(u, n);
    if app.session.active_index() == Some(i)
        && app.session.tool_id() == "artboard"
        && let Some(count) = app.session.active().map(|d| d.doc.artboards.len())
    {
        let board = crate::panels::artboards::selected(app, count);
        crate::panels::artboards::select(app, board);
    }
    true
}

/// Make window `w` the current one, as a click on its tab does.
pub(crate) fn show_window(app: &mut VectorcraftApp, w: Win) {
    bring(app, w);
    crate::chrome::show_document(app, w.0);
}

/// Forget the views of documents that have closed, and keep each other view on an artboard its
/// document has. Called by [`VectorcraftApp::sync_views`].
pub(crate) fn retain(app: &mut VectorcraftApp) {
    let views = &mut app.canvas.windows.views;
    if views.others.is_empty() && views.shown.is_empty() {
        return;
    }
    let docs = app.session.documents();
    views.shown.retain(|u, _| docs.iter().any(|d| d.uid == *u));
    views.others.retain_mut(|o| match docs.iter().find(|d| d.uid == o.uid) {
        Some(d) => {
            o.view.artboard = o.view.artboard.min(d.doc.artboards.len().saturating_sub(1));
            true
        }
        None => false,
    });
}

/// `window.newWindow`: another view of the current document, numbered after its others, which
/// starts where the current view is and becomes the current window. It opens as a tab, or in a
/// floating window of its own when the current view is in one. Returns `{index, view}`.
pub(crate) fn new_window(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.sync_views();
    let Some((i, n)) = current(app) else { return Err("no document open".into()) };
    let (Some(u), Some(view)) = (uid(app, i), app.views.get(i).copied()) else { return Err("no document open".into()) };
    let number = numbers(app, i).last().copied().unwrap_or(n) + 1;
    let floats = super::floats(app, (i, n));
    app.canvas.windows.views.others.push(OtherView { uid: u, number, view });
    show_window(app, (i, number));
    if floats {
        super::place(app, &[(i, number)]);
    }
    Ok(json!({ "index": i, "view": number }))
}

/// Close window `w`: its view goes, and its document stays open with its other views. A
/// document's last view closes the document, asking to save it first when it has changed
/// ([`crate::unsaved::close`]). When one view is left, it is numbered 1 again. Nothing closes while
/// a file dialog's sheet is on the window.
pub(crate) fn close_window(app: &mut VectorcraftApp, (i, n): Win) -> Result<Value, String> {
    let nums = numbers(app, i);
    if nums.len() <= 1 {
        return crate::unsaved::close(app, i);
    }
    crate::unsaved::refuse_while_picking(app)?;
    let Some(u) = uid(app, i) else { return Err("no such document".into()) };
    if !nums.contains(&n) {
        return Err("no such document".into());
    }
    let was_current = current(app) == Some((i, n));
    // The document's next view in the tab strip takes the closing one's place.
    let next = nums.iter().copied().find(|m| *m > n).or(nums.iter().copied().rfind(|m| *m != n));
    if n == shown(app, i)
        && let Some(next) = next
    {
        bring(app, (i, next));
    }
    app.canvas.windows.views.others.retain(|o| !(o.uid == u && o.number == n));
    app.ui.floating.retain(|w| !(w.uid == u && w.view == n));
    if app.ui.tab_document == Some((u, n)) {
        app.ui.tab_document = None;
    }
    let left = numbers(app, i);
    if let [only] = left[..] {
        app.canvas.windows.views.shown.remove(&u);
        for w in app.ui.floating.iter_mut().filter(|w| w.uid == u && w.view == only) {
            w.view = 1;
        }
        if app.ui.tab_document == Some((u, only)) {
            app.ui.tab_document = Some((u, 1));
        }
    }
    if was_current {
        crate::chrome::show_document(app, i);
    }
    Ok(json!({ "index": i, "views": left.len() }))
}

/// `file.close` without an `index`: the current window closes ([`close_window`]).
pub(crate) fn close_current(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(w) = current(app) else { return Err("no document open".into()) };
    close_window(app, w)
}

/// `params`' `index` and `view`: the window they name, `Ok(None)` without an `index`. Without a
/// `view`, the document's shown view.
pub(crate) fn window_param(app: &VectorcraftApp, p: &Value) -> Result<Option<Win>, String> {
    let i = match p.get("index") {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < app.session.documents().len()).ok_or("no such document")?,
    };
    let n = match p.get("view") {
        None | Some(Value::Null) => shown(app, i),
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| numbers(app, i).contains(n)).ok_or("no such document")?,
    };
    Ok(Some((i, n)))
}
