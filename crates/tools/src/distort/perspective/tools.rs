use super::*;

#[derive(Default)]
pub struct PerspectiveGridTool {
    drag: Option<Press>,
    /// Dragging a plane widget.
    plane: Option<PlaneDrag>,
}

impl Tool for PerspectiveGridTool {
    fn id(&self) -> &'static str {
        "perspectiveGrid"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        let g = PerspectiveGrid::current(cx.doc);
        match ev.kind {
            PointerKind::Down => {
                let mut pre = vec![];
                if !g.visible {
                    pre.push(Action::Exec("perspective.grid.show".into(), json!({"visible": true})));
                }
                if let Some(pl) = g.widget_hit_cx(cx, p) {
                    pre.push(Action::Exec("perspective.plane.set".into(), json!({"plane": pl.id()})));
                    return pre;
                }
                if let Some(d) = PlaneDrag::hit(&g, cx.tol(6.0), p) {
                    self.plane = Some(d);
                    pre.push(Action::Begin("Move Plane".into()));
                    return pre;
                }
                let tol = cx.tol(6.0);
                if !g.locked
                    && let Some((h, _)) = handles(&g, cx.tol(1.0))
                        .into_iter()
                        .filter(|(_, q)| q.distance(p) <= tol)
                        .min_by(|a, b| a.1.distance(p).total_cmp(&b.1.distance(p)))
                {
                    let steps = cell_steps(&g, cx.tol(1.0));
                    self.drag = Some(Press { handle: h, grid: g, at: p, steps });
                    pre.push(Action::Begin("Edit Perspective Grid".into()));
                }
                pre
            }
            PointerKind::Drag if self.plane.is_some() => self.plane.and_then(|d| d.preview(&g, p, ev.mods)).into_iter().collect(),
            PointerKind::Drag => match &self.drag {
                Some(press) => vec![Action::Preview("perspective.grid.set".into(), drag_handle(press, p, ev.mods).definition_json())],
                None => vec![],
            },
            PointerKind::Up => match (self.drag.take(), self.plane.take()) {
                (None, None) => vec![],
                _ => vec![Action::Commit],
            },
            PointerKind::DoubleClick => PlaneDrag::hit(&g, cx.tol(6.0), p).map(|d| d.options()).into_iter().collect(),
            _ => vec![],
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let g = PerspectiveGrid::current(cx.doc);
        // The grid's handles hide with it.
        if !g.visible {
            return vec![];
        }
        let ink = [0x20, 0x20, 0x20];
        handles(&g, cx.tol(1.0))
            .into_iter()
            .map(|(h, p)| match h {
                Handle::VpLeft | Handle::VpRight | Handle::VpVertical => Overlay::Anchor { p, color: ink, filled: true, size: 7.0 },
                Handle::GroundLeft | Handle::GroundRight => Overlay::Anchor { p, color: ink, filled: true, size: 5.0 },
                Handle::Cell => Overlay::Handle { p, color: ink },
                _ => Overlay::Anchor { p, color: ink, filled: false, size: 6.0 },
            })
            .collect()
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        let g = PerspectiveGrid::current(cx.doc);
        let tol = cx.tol(6.0);
        if self.plane.is_some() || PlaneDrag::hit(&g, tol, p).is_some() {
            return Cursor::Move;
        }
        if g.locked {
            return Cursor::Arrow;
        }
        match handles(&g, cx.tol(1.0)).into_iter().find(|(_, q)| q.distance(p) <= tol).map(|h| h.0) {
            Some(Handle::Horizon | Handle::Height | Handle::Cell) => Cursor::ResizeV,
            Some(Handle::ExtentLeft | Handle::ExtentRight) => Cursor::ResizeH,
            Some(_) => Cursor::Move,
            None => Cursor::Arrow,
        }
        .turned(cx.view_angle())
    }
    fn busy(&self) -> bool {
        self.drag.is_some() || self.plane.is_some()
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        match (self.drag.take(), self.plane.take()) {
            (None, None) => vec![],
            _ => vec![Action::Commit],
        }
    }
}

// ---------- Perspective Selection tool ----------

#[derive(Clone, Debug, Default)]
enum SelDrag {
    #[default]
    Idle,
    /// Pressed on an object: moves `ids` once the pointer leaves the drag threshold. `perp`: along
    /// the plane's normal (5 pressed while dragging); `copy`: Alt at the last event.
    Move { ids: Vec<NodeId>, from: Point, last: Point, began: bool, perp: bool, copy: bool },
    /// Dragging `handle` of the perspective bounding box `rect` (plane space, on `plane` at `depth`).
    Scale { ids: Vec<NodeId>, plane: Plane, depth: f64, rect: Rect, handle: BoxHandle },
    /// Dragging a plane widget.
    Plane(PlaneDrag),
}

/// The Perspective Selection tool: selects, moves (Alt copies; press 5 while dragging to move
/// perpendicular to the plane) and scales (bounding-box handles; Shift proportional, Alt from the
/// centre) objects within their perspective planes; the arrow keys nudge them in perspective.
#[derive(Default)]
pub struct PerspectiveSelectionTool {
    drag: SelDrag,
    measure: Option<(Point, String)>,
}

/// The selection's perspective bounding box: plane, depth, plane-space rect and the homography
/// that draws it.
fn selection_persp_box(cx: &ToolContext, g: &PerspectiveGrid) -> Option<(Plane, f64, Rect, Homography)> {
    let (plane, depth, rect) = g.plane_bounds(cx.doc, &cx.selection.objects)?;
    Some((plane, depth, rect, g.homography_at(plane, depth)?))
}

/// The handle of the perspective box (`rect` drawn by `h`) within `tol` of `p`.
fn persp_handle_at(rect: Rect, h: &Homography, p: Point, tol: f64) -> Option<BoxHandle> {
    BoxHandle::ALL.into_iter().find(|k| h.apply(k.pos(rect)).is_some_and(|q| q.distance(p) <= tol))
}

/// The resize cursor that fits a handle seen in direction `d` from the box's centre.
pub(super) fn resize_cursor(d: Vec2) -> Cursor {
    // Counter-clockwise degrees on screen (y down), folded to 0..180.
    let a = (-d.y).atan2(d.x).to_degrees().rem_euclid(180.0);
    match a {
        a if !(22.5..157.5).contains(&a) => Cursor::ResizeH,
        a if a < 67.5 => Cursor::ResizeNeSw,
        a if a < 112.5 => Cursor::ResizeV,
        _ => Cursor::ResizeNwSe,
    }
}

impl PerspectiveSelectionTool {
    /// The `perspective.move` preview for the move drag (and its measurement label).
    fn move_preview(&mut self, cx: &ToolContext) -> Option<Action> {
        let SelDrag::Move { ids, from, last, perp, copy, .. } = &self.drag else { return None };
        let g = PerspectiveGrid::current(cx.doc);
        let mut v = json!({"ids": crate::json_ids(ids), "from": [from.x, from.y], "to": [last.x, last.y], "copy": copy, "perpendicular": perp});
        if g.plane != Plane::None {
            v["plane"] = json!(g.plane.id());
        }
        let at = ids.first().and_then(|id| g.attachment_of(cx.doc, *id)).or((g.plane != Plane::None).then_some((g.plane, g.offset(g.plane))));
        self.measure = at.and_then(|(pl, depth)| {
            let text = if *perp {
                format!("dZ: {}", cx.len(g.depth_at(pl, depth, *from, *last)? - depth))
            } else {
                let d = g.plane_delta(pl, depth, *from, *last)?;
                cx.offset_label(d.x, d.y)
            };
            Some((*last, text))
        });
        Some(Action::Preview("perspective.move".into(), v))
    }
}

impl Tool for PerspectiveSelectionTool {
    fn id(&self) -> &'static str {
        "perspectiveSelection"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let (p, m) = (ev.pos, ev.mods);
        let g = PerspectiveGrid::current(cx.doc);
        match (ev.kind, std::mem::take(&mut self.drag)) {
            (PointerKind::Down, _) => {
                self.measure = None;
                if let Some(pl) = g.widget_hit_cx(cx, p) {
                    return vec![Action::Exec("perspective.plane.set".into(), json!({"plane": pl.id()}))];
                }
                if let Some(d) = PlaneDrag::hit(&g, cx.tol(6.0), p) {
                    self.drag = SelDrag::Plane(d);
                    return vec![Action::Begin("Move Plane".into())];
                }
                if let Some((plane, depth, rect, h)) = selection_persp_box(cx, &g)
                    && let Some(handle) = persp_handle_at(rect, &h, p, cx.tol(5.0))
                {
                    self.drag = SelDrag::Scale { ids: cx.selection.objects.clone(), plane, depth, rect, handle };
                    return vec![Action::Begin("Scale in Perspective".into())];
                }
                let Some(hit) = vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options()) else {
                    return vec![Action::Exec("select.set".into(), json!({"ids": []}))];
                };
                let top = hit.top_object(cx.isolation);
                let mut acts = vec![];
                let ids = if cx.selection.objects.contains(&top) {
                    cx.selection.objects.clone()
                } else {
                    acts.push(Action::Exec("select.set".into(), json!({"ids": [top.0]})));
                    vec![top]
                };
                // Selecting an object in perspective makes its plane the active one.
                if let Some((pl, _)) = g.attachment_of(cx.doc, top)
                    && pl != g.plane
                {
                    acts.push(Action::Exec("perspective.plane.set".into(), json!({"plane": pl.id()})));
                }
                self.drag = SelDrag::Move { ids, from: p, last: p, began: false, perp: false, copy: m.alt };
                acts
            }
            (PointerKind::Drag, SelDrag::Move { ids, from, began, perp, .. }) => {
                if !began && p.distance(from) < cx.tol(3.0) {
                    self.drag = SelDrag::Move { ids, from, last: p, began, perp, copy: m.alt };
                    return vec![];
                }
                let mut out = vec![];
                if !began {
                    out.push(Action::Begin(if m.alt { "Copy in Perspective" } else { "Move in Perspective" }.into()));
                }
                self.drag = SelDrag::Move { ids, from, last: p, began: true, perp, copy: m.alt };
                out.extend(self.move_preview(cx));
                out
            }
            (PointerKind::Drag, SelDrag::Scale { ids, plane, depth, rect, handle }) => {
                let q = g.homography_at(plane, depth).and_then(|h| h.inverse()).and_then(|hi| hi.apply(p));
                self.drag = SelDrag::Scale { ids: ids.clone(), plane, depth, rect, handle };
                let Some(q) = q else { return vec![] };
                let a = scale_for_drag(rect, handle, q, m.shift, m.alt);
                let nr = a.transform_rect_bbox(rect);
                self.measure = Some((p, cx.size_label(nr.width(), nr.height())));
                vec![Action::Preview("perspective.transform".into(), json!({"ids": crate::json_ids(&ids), "matrix": crate::select::matrix_json(a)}))]
            }
            (PointerKind::Up, SelDrag::Move { began, .. }) => {
                self.measure = None;
                if began { vec![Action::Commit] } else { vec![] }
            }
            (PointerKind::Drag, SelDrag::Plane(d)) => {
                self.drag = SelDrag::Plane(d);
                d.preview(&g, p, m).into_iter().collect()
            }
            (PointerKind::Up, SelDrag::Scale { .. } | SelDrag::Plane(_)) => {
                self.measure = None;
                vec![Action::Commit]
            }
            (PointerKind::DoubleClick, d) => {
                self.drag = d;
                if let Some(w) = PlaneDrag::hit(&g, cx.tol(6.0), p) {
                    return vec![w.options()];
                }
                // Type in perspective: Edit Text, then type.
                match vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options()).map(|h| h.top_object(cx.isolation)) {
                    Some(id) if cx.doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)) && n.projection().is_some()) => {
                        vec![Action::Exec("perspective.editText".into(), json!({"id": id.0})), Action::SwitchTool("type".into())]
                    }
                    _ => vec![],
                }
            }
            (_, d) => {
                self.drag = d;
                vec![]
            }
        }
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, mods: Mods) -> Vec<Action> {
        if let (ToolKey::Digit(5), SelDrag::Move { perp, began, .. }) = (key, &mut self.drag) {
            *perp = !*perp;
            return if *began { self.move_preview(cx).into_iter().collect() } else { vec![] };
        }
        let (dx, dy) = match key {
            ToolKey::Left => (-1, 0),
            ToolKey::Right => (1, 0),
            ToolKey::Up => (0, -1),
            ToolKey::Down => (0, 1),
            _ => return vec![],
        };
        if !self.claims_key(cx, key) {
            return vec![];
        }
        vec![Action::Exec("perspective.nudge".into(), json!({"dx": dx, "dy": dy, "big": mods.shift, "copy": mods.alt}))]
    }
    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        // The arrows nudge a selection in perspective.
        matches!(key, ToolKey::Left | ToolKey::Right | ToolKey::Up | ToolKey::Down)
            && matches!(self.drag, SelDrag::Idle)
            && cx.selection.anchors.is_empty()
            && {
                let g = PerspectiveGrid::current(cx.doc);
                cx.selection.objects.iter().any(|id| g.attachment_of(cx.doc, *id).is_some())
            }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        // The selection's bounding box in perspective, in its plane's colour, with its handles.
        let g = PerspectiveGrid::current(cx.doc);
        let mut out = vec![];
        if let Some((plane, _, rect, h)) = selection_persp_box(cx, &g) {
            let corners: Vec<Point> = crate::xform::rect_corners(rect).iter().filter_map(|q| h.apply(*q)).collect();
            if corners.len() == 4 {
                out.push(Overlay::Path { path: crate::xform::polygon(&corners, true), color: plane.color(), width: 1.0, dashed: false });
            }
            for k in BoxHandle::ALL {
                if let Some(q) = h.apply(k.pos(rect)) {
                    out.push(Overlay::Anchor { p: q, color: plane.color(), filled: false, size: 6.0 });
                }
            }
        }
        if cx.measurement_labels
            && let Some((p, t)) = &self.measure
        {
            out.push(Overlay::Measure { p: *p, text: t.clone() });
        }
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        let g = PerspectiveGrid::current(cx.doc);
        let handle = match &self.drag {
            SelDrag::Scale { handle, .. } => Some(*handle),
            SelDrag::Move { began: true, .. } | SelDrag::Plane(_) => return Cursor::Move,
            _ => None,
        };
        if PlaneDrag::hit(&g, cx.tol(6.0), p).is_some() {
            return Cursor::Move;
        }
        if let Some((_, _, rect, h)) = selection_persp_box(cx, &g)
            && let Some(k) = handle.or_else(|| persp_handle_at(rect, &h, p, cx.tol(5.0)))
            && let (Some(a), Some(c)) = (h.apply(k.pos(rect)), h.apply(rect.center()))
        {
            // The handle's direction on screen: the view turns it clockwise.
            return resize_cursor((Affine::rotate(cx.view_angle().to_radians()) * (a - c).to_point()).to_vec2());
        }
        Cursor::Arrow
    }
    fn busy(&self) -> bool {
        !matches!(self.drag, SelDrag::Idle)
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.measure = None;
        match std::mem::take(&mut self.drag) {
            SelDrag::Move { began: true, .. } | SelDrag::Scale { .. } | SelDrag::Plane(_) => vec![Action::Commit],
            _ => vec![],
        }
    }
}
