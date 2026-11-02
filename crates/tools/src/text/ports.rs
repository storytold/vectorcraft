use super::{FontDb, NodeId, NodeKind, Overlay, Point, Rect, Shape, TextKind, TextObject, ToolContext};

/// How far (screen px) the ports of area type sit from the frame's corners, along its sides: the
/// out port above the bottom-right corner on the right side, the in port below the top-left corner
/// on the left side, clear of the bounding box's corner handles, so both can be grabbed.
pub const PORT_RISE_PX: f64 = 12.0;

/// Half a port's side (screen px).
const PORT_HALF_PX: f64 = 4.5;

/// The red of the out port of text that doesn't fit.
const OVERFLOW_RED: [u8; 3] = [230, 30, 30];

/// One of the two ports of area type or type on a path: its story flows in through the in port
/// and on through the out port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    In,
    Out,
}

/// What a port shows: empty when the frame is not threaded on that side (and, for the out port,
/// all its text is seen), an arrow when it is threaded to another frame there, a red "+" in the out
/// port when its text doesn't fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortState {
    Empty,
    Threaded,
    Overflow,
}

/// Centre of `port` of `t`, `px` document units to the screen pixel: area type's on its frame's
/// sides by [`PORT_RISE_PX`] from the corners; type on a path's along its path, as far past its end
/// bracket (out) or before its start bracket (in), clear of the brackets, which drag.
pub fn port_centre(t: &TextObject, port: Port, px: f64) -> Option<Point> {
    let rise = PORT_RISE_PX * px;
    match &t.kind {
        TextKind::Area { frame } => {
            let b = frame.bounds()?;
            // The corner the port sits by, and the far end of the side it sits on.
            let (corner, far) = match port {
                Port::Out => (Point::new(b.x1, b.y1), Point::new(b.x1, b.y0)),
                Port::In => (Point::new(b.x0, b.y0), Point::new(b.x0, b.y1)),
            };
            let (corner, side) = (t.xf * corner, t.xf * far - t.xf * corner);
            let len = side.hypot();
            Some(if len > 1e-9 && len.is_finite() { corner + side * (rise.min(len / 2.0) / len) } else { corner })
        }
        TextKind::OnPath { path, .. } => {
            let arc = vectorcraft_geom::ArcPath::new(path);
            let (start, end) = t.kind.path_span()?;
            if arc.is_empty() {
                return None;
            }
            let (at, away) = match port {
                Port::Out => (end, rise),
                Port::In => (start, -rise),
            };
            let (p, dir) = arc.at(at * arc.len());
            let (at, ahead) = (t.xf * p, t.xf * (p + dir) - t.xf * p);
            let len = ahead.hypot();
            Some(if len > 1e-9 && len.is_finite() { at + ahead * (away / len) } else { at })
        }
        TextKind::Point => None,
    }
}

/// What the in and out ports of frame `id` of `d` show, its text `overflow`ing or not: an arrow
/// where a frame of its thread comes before (in) or after (out) it, else a red "+" in the out port
/// of text that doesn't fit, else nothing.
pub fn port_states(d: &vectorcraft_doc::Document, id: NodeId, overflow: bool) -> (PortState, PortState) {
    let thread = d.text_threads.iter().find(|t| t.contains(&id));
    let at = thread.and_then(|t| t.iter().position(|x| *x == id));
    let input = if at.is_some_and(|i| i > 0) { PortState::Threaded } else { PortState::Empty };
    let output = match (thread, at) {
        (Some(t), Some(i)) if i + 1 < t.len() => PortState::Threaded,
        _ if overflow => PortState::Overflow,
        _ => PortState::Empty,
    };
    (input, output)
}

/// A port at `c` showing `state`, `px` document units to the screen pixel: a white box outlined in
/// `color` (the frame's layer colour; red for text that doesn't fit), with a red "+" or an arrow
/// pointing the way the story flows.
pub fn push_port(o: &mut Vec<Overlay>, c: Point, state: PortState, color: [u8; 3], px: f64) {
    let h = PORT_HALF_PX * px;
    let r = Rect::new(c.x - h, c.y - h, c.x + h, c.y + h);
    let color = if state == PortState::Overflow { OVERFLOW_RED } else { color };
    let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
    o.push(Overlay::Highlight { quad: corners, color: [255, 255, 255, 255] });
    o.push(Overlay::Path { path: r.to_path(0.1), color, width: 1.0, dashed: false });
    let k = 2.5 * px;
    let line = |a: Point, b: Point| Overlay::Line { a, b, color, dashed: false };
    match state {
        PortState::Empty => {}
        PortState::Overflow => {
            o.push(line(Point::new(c.x - k, c.y), Point::new(c.x + k, c.y)));
            o.push(line(Point::new(c.x, c.y - k), Point::new(c.x, c.y + k)));
        }
        PortState::Threaded => {
            let tip = Point::new(c.x + k, c.y);
            o.push(line(Point::new(c.x - k, c.y), tip));
            o.push(line(Point::new(c.x, c.y - k), tip));
            o.push(line(Point::new(c.x, c.y + k), tip));
        }
    }
}

/// The in and out ports of area type or type on a path `t`, frame `id`, its text `overflow`ing or
/// not ([`port_states`]).
pub(super) fn ports(cx: &ToolContext, id: NodeId, t: &TextObject, overflow: bool, o: &mut Vec<Overlay>) {
    let (input, output) = port_states(cx.doc, id, overflow);
    let (color, px) = (cx.doc.layer_color(id), cx.tol(1.0));
    for (port, state) in [(Port::In, input), (Port::Out, output)] {
        if let Some(c) = port_centre(t, port, px) {
            push_port(o, c, state, color, px);
        }
    }
}

/// The most selected area type and type on a path whose ports are drawn and take clicks: selecting
/// all of an opened PDF can select tens of thousands of type objects, whose ports would only bury
/// the canvas (and the GPU's buffers) in boxes.
pub const PORTS_MAX: usize = 1_000;

/// The text of `id` in `d` when it has ports: area type or type on a path a thread can hold, not in
/// perspective ([`crate::select::is_threadable`]). The one test for drawing ports, clicking them and
/// counting them toward [`PORTS_MAX`].
pub fn ported(d: &vectorcraft_doc::Document, id: NodeId) -> Option<&TextObject> {
    match d.node(id).filter(|n| crate::select::is_threadable(n)).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    }
}

/// Are at most [`PORTS_MAX`] of `selected` [`ported`] (counting stops past it), so that their
/// ports show?
pub fn ports_shown(d: &vectorcraft_doc::Document, selected: &[NodeId]) -> bool {
    selected.iter().filter(|id| ported(d, **id).is_some()).take(PORTS_MAX + 1).count() <= PORTS_MAX
}

/// The selected area type or type on a path with a port under `p`, and which port: a click on the
/// out port loads the Selection tool with its text to thread into another frame after it, a click
/// on the in port to thread into one before it. Every such frame has both ports, threaded,
/// overflowing or not, while their ports show ([`PORTS_MAX`]).
pub(crate) fn port_at(cx: &ToolContext, p: Point) -> Option<(NodeId, Port)> {
    if !ports_shown(cx.doc, &cx.selection.objects) {
        return None;
    }
    let h = cx.tol(PORT_HALF_PX + 1.0);
    let near = |c: Point| (c.x - p.x).abs() <= h && (c.y - p.y).abs() <= h;
    cx.selection.objects.iter().find_map(|id| {
        let t = ported(cx.doc, *id)?;
        [Port::Out, Port::In].into_iter().find(|port| port_centre(t, *port, cx.tol(1.0)).is_some_and(near)).map(|port| (*id, port))
    })
}

/// Which selected text overflows, for [`thread_ports`]: each text object's layout verdict kept
/// with the object and the fonts it was made from, as overlays run every frame.
#[derive(Default)]
pub struct PortMarks {
    seen: std::collections::HashMap<NodeId, ((u64, TextObject), bool)>,
}

/// The in and out ports of the selected area type and type on a path, for every tool but the Type
/// tools (they draw their own): each tells whether the frame is threaded on that side and the out
/// port whether text is hidden by the size of its frame, whatever tool is active. None when more
/// than [`PORTS_MAX`] of them are selected.
pub fn thread_ports(marks: &mut PortMarks, cx: &ToolContext) -> Vec<Overlay> {
    let fonts = FontDb::global().generation();
    let mut o = Vec::new();
    let mut kept = std::collections::HashMap::new();
    if !ports_shown(cx.doc, &cx.selection.objects) {
        marks.seen = kept;
        return o;
    }
    for id in &cx.selection.objects {
        let Some(t) = ported(cx.doc, *id) else { continue };
        let over = match marks.seen.remove(id) {
            Some(((g, was), over)) if g == fonts && &was == t => over,
            _ => vectorcraft_text::layout(FontDb::global(), t).overflow,
        };
        ports(cx, *id, t, over, &mut o);
        kept.insert(*id, ((fonts, t.clone()), over));
    }
    marks.seen = kept;
    o
}
