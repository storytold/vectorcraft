//! Editing with the Quick Pen while no path is being drawn: a drag on an anchor of a selected path
//! moves it, and a drag on one of its segments moves that segment (`path.moveAnchors` with
//! `interpolate`: a segment with one moved end is turned and scaled to its new ends, so a line
//! stays a line and an arc stays an arc of the same angle; at a smooth point, where two arcs join,
//! the handles stay in line and the arcs bend to meet). The moved anchor snaps as a Direct
//! Selection drag's does. A click on an anchor selects it, Shift-click adds it to the selected anchors or takes it
//! out, and the canvas context menu's Delete then removes them (`edit.clear`). A click on an end
//! of an open path goes on drawing from it, as before.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::{AnchorRef, Node, NodeId, Selection};
use vectorcraft_geom::Point;

use super::{same_node, shared_node};
use crate::draw2::{anchor_in, segment_in, shown_paths};
use crate::guides::{PointSnap, Targets};
use crate::{Action, Cursor, Overlay, ToolContext};

/// The notice the tool asks for after each preview of a drag, to see the document's revision then.
pub(super) const MOVED: &str = "moved";

/// What a press holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Grab {
    /// Anchor `ai` of subpath `si` of path `id`, at `at`; `end` when it is an end of an open subpath.
    Anchor { id: NodeId, si: usize, ai: usize, at: Point, end: bool },
    /// The segment from anchor `seg` of subpath `si` of path `id` to the next one.
    Segment { id: NodeId, si: usize, seg: usize },
}

impl Grab {
    /// The path the grab is on.
    pub(super) fn id(self) -> NodeId {
        match self {
            Grab::Anchor { id, .. } | Grab::Segment { id, .. } => id,
        }
    }

    /// The anchors the drag moves when they are not already selected: the anchor, or the
    /// segment's two ends.
    fn anchors(self, cx: &ToolContext) -> Option<Vec<AnchorRef>> {
        match self {
            Grab::Anchor { si, ai, .. } => Some(vec![(si, ai)]),
            Grab::Segment { id, si, seg } => {
                let n = cx.doc.node(id)?.path_data()?.subpaths.get(si)?.anchors.len();
                Some(vec![(si, seg), (si, (seg + 1).checked_rem(n)?)])
            }
        }
    }
}

/// An anchor or a segment of a selected path under `p`, within the tool's point tolerance `tol`.
/// Anchors come before segments.
pub(super) fn grab_at(cx: &ToolContext, p: Point, tol: f64) -> Option<Grab> {
    let shown = shown_paths(cx);
    if let Some((id, si, ai)) = anchor_in(cx, shown.iter().copied(), p, tol) {
        let sp = cx.doc.node(id)?.path_data()?.subpaths.get(si)?;
        let at = sp.anchors.get(ai)?.p;
        let end = !sp.closed && (ai == 0 || ai + 1 == sp.anchors.len());
        return Some(Grab::Anchor { id, si, ai, at, end });
    }
    segment_in(cx, shown, p, tol).map(|(id, si, seg, _)| Grab::Segment { id, si, seg })
}

/// The cursor over `grab`: an open end shows that a click goes on drawing from it, another anchor
/// that it can be selected and moved, and a segment that it moves.
pub(super) fn cursor(cx: &ToolContext, grab: Grab) -> Cursor {
    match grab {
        Grab::Anchor { end: true, .. } => Cursor::PenContinue,
        Grab::Anchor { id, si, ai, .. } if cx.selection.partial(id).is_some_and(|s| s.contains(&(si, ai))) => Cursor::DirectAnchorSelected,
        Grab::Anchor { .. } => Cursor::DirectAnchor,
        Grab::Segment { .. } => Cursor::Move,
    }
}

/// A press on an anchor or a segment of a selected path, while no path is being drawn.
pub(super) struct Edit {
    pub(super) grab: Grab,
    /// Where the press went down.
    pub(super) start: Point,
    pub(super) shift: bool,
    /// Whether the press has dragged far enough to move what it holds (an undo step began).
    pub(super) began: bool,
    /// Whether the last drag left what the press holds away from where it started.
    pub(super) moved: bool,
    /// The document's revision ([`ToolContext::revision`]) after the drag's last preview.
    pub(super) revision: Option<(u64, u64)>,
    /// Whether another command changed the document or the selection during the drag: the drag
    /// then previews no more, and its undo step keeps that edit with the move as last shown.
    pub(super) interrupted: bool,
    /// The grabbed path as the document had it when the press went down.
    pressed: Option<Arc<Node>>,
    /// The selection when the press went down.
    pressed_selection: Selection,
    snap: Option<PointSnap>,
    pub(super) guides: Vec<Overlay>,
}

impl Edit {
    pub(super) fn new(cx: &ToolContext, grab: Grab, start: Point, shift: bool) -> Self {
        let pressed = shared_node(cx.doc, grab.id()).cloned();
        let pressed_selection = cx.selection.clone();
        Self {
            grab,
            start,
            shift,
            began: false,
            moved: false,
            revision: None,
            interrupted: false,
            pressed,
            pressed_selection,
            snap: None,
            guides: vec![],
        }
    }

    /// Whether the drag began and still previews: no other command changed the document or the
    /// selection since its last preview.
    pub(super) fn open(&mut self, cx: &ToolContext) -> bool {
        if self.began && self.revision.is_some_and(|r| r != cx.revision) {
            self.interrupted = true;
        }
        self.began && !self.interrupted
    }

    /// What ends a drag that began when the press is let go: a commit, or a cancel when the drag
    /// came back to where it started and no other command's edit is in its undo step.
    pub(super) fn finish(&mut self, cx: &ToolContext) -> Option<Action> {
        if !self.began {
            return None;
        }
        Some(if self.open(cx) && !self.moved { Action::Cancel } else { Action::Commit })
    }

    /// Whether the grabbed path and the selection are as they were when the press went down.
    pub(super) fn unchanged(&self, cx: &ToolContext) -> bool {
        let path = match (cx.doc.node(self.grab.id()), self.pressed.as_ref()) {
            (Some(now), Some(then)) => same_node(now, then),
            _ => false,
        };
        path && *cx.selection == self.pressed_selection
    }

    /// Moves what the press holds by the drag to `p`. The first move selects the anchor, or the
    /// segment, unless it is already selected, and begins the undo step; a drag that starts on
    /// one of several selected anchors moves them all. When the grabbed path or the selection
    /// changed after the press went down, the drag does not begin, and after another command's edit during the
    /// drag it moves nothing more.
    pub(super) fn drag(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let mut out = vec![];
        if self.began && !self.open(cx) {
            return out;
        }
        if !self.began {
            let id = self.grab.id();
            if !self.unchanged(cx) {
                return out;
            }
            let Some(picked) = self.grab.anchors(cx) else { return out };
            // A drag on one of several selected anchors moves them all; a drag on a segment moves
            // its two ends alone and leaves that segment selected, not its ends as anchors.
            let already = match self.grab {
                Grab::Anchor { .. } => cx.selection.partial(id).is_some_and(|s| picked.iter().all(|a| s.contains(a))),
                Grab::Segment { si, seg, .. } => {
                    cx.selection.objects == [id]
                        && cx.selection.anchors.len() == 1
                        && cx.selection.partial(id).is_some_and(|s| s.len() == picked.len() && picked.iter().all(|a| s.contains(a)))
                        && cx.selection.segments.len() == 1
                        && cx.selection.segments.get(&id).is_some_and(|s| s.len() == 1 && s.contains(&(si, seg)))
                }
            };
            let moving = if already {
                cx.selection.clone()
            } else {
                out.push(match self.grab {
                    Grab::Anchor { si, ai, .. } => Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": "set"})),
                    Grab::Segment { si, seg, .. } => {
                        Action::Exec("select.segments".into(), json!({"id": id.0, "segments": [[si, seg]], "mode": "set"}))
                    }
                });
                let mut sel = Selection { objects: vec![id], ..Selection::default() };
                sel.anchors.insert(id, picked.into_iter().collect::<BTreeSet<_>>());
                sel
            };
            out.push(Action::Begin("Move".into()));
            // A held anchor snaps; the anchors and segments that stay put are targets too.
            if matches!(self.grab, Grab::Anchor { .. }) {
                self.snap = Some(PointSnap::moving(cx, || Targets::for_anchor_drag(cx.doc, &moving)));
            }
            self.began = true;
        }
        let d = match (self.grab, &self.snap) {
            (Grab::Anchor { at, .. }, Some(snap)) => {
                let (q, guides) = snap.snap_from(cx, at + (p - self.start), None);
                self.guides = guides;
                q - at
            }
            _ => p - self.start,
        };
        if d.x.is_finite() && d.y.is_finite() {
            self.moved = d.x != 0.0 || d.y != 0.0;
            out.push(Action::Preview("path.moveAnchors".into(), json!({"dx": d.x, "dy": d.y, "interpolate": true})));
        }
        out.push(Action::Notify(MOVED.into()));
        out
    }
}
