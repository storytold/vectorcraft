//! Object › Intertwine (#1763): objects that pass over and under each other.
//!
//! `object.intertwine.make` puts the selected objects in an Intertwine group
//! (`NodeKind::Intertwine`), `object.intertwine.release` gives them back as they were. In between,
//! `object.intertwine.arrange` picks overlap areas, by a click (`at`) or by an enclosed path drawn
//! around them (`area`), and changes the order the members are drawn in there: each member keeps
//! the areas where it is drawn over others, by tier (`NodeExtras::intertwine` and
//! `NodeExtras::intertwine_tiers`, see `vectorcraft_doc::intertwine`). Object › Expand turns the
//! group into a plain group of what it draws ([`expand_intertwines`]), which Ungroup takes apart.
//!
//! An overlap area is a face of the planar arrangement of what the members paint (their fills and
//! strokes, type outlined; other objects by their bounds): the part where two or more of them
//! overlap, split wherever a member's edge crosses it. The area a member is raised in is that
//! face grown a little ([`SEAM`]) into the parts where only that member paints ([`raised_area`]),
//! so its edge does not lie on the edge of the member it passes over: there, both edges would be
//! anti-aliased and the member below would show through as a hairline (#5337).
//!
//! Behaviour: <https://helpx.adobe.com/illustrator/using/intertwine-objects.html> (Make,
//! Release, clicking or drawing around an overlap, Bring to Front, Bring Forward, Send Backward
//! and Send to Back where three or more objects overlap:
//! <https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/create-intertwined-objects.html>).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Point, SubPath};
use vectorcraft_pathops as po;
use vectorcraft_tools::intertwine::{Face, drawn_order, face_at, faces_in, faces_of, is_intertwine, nonempty, painted_areas};

use super::edit::selected_roots;
use super::pathops::Beside;
use super::{CommandSpec, EngineError, Result, Session, bad, checked_id, checked_ids_param, cmd, has_doc, has_selection, point_param, str_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.intertwine.make",
            "Make",
            ["Object", "Intertwine"],
            None,
            "{ids?: two or more objects (default: the selection)} put the objects in an Intertwine group and choose the Intertwine tool, which arranges its overlap areas (object.intertwine.arrange) → {id}",
            has_selection,
            make
        ),
        cmd!(
            "object.intertwine.edit",
            "Edit",
            ["Object", "Intertwine"],
            None,
            "{} choose the Intertwine tool for the selected Intertwine group (Esc or Enter goes back to the tool before)",
            has_intertwine,
            edit
        ),
        cmd!(
            "object.intertwine.release",
            "Release",
            ["Object", "Intertwine"],
            None,
            "{} give the objects of the selected Intertwine groups back as they were made, dropping the arranged areas → {ids}",
            has_intertwine,
            release
        ),
        cmd!(
            "object.intertwine.arrange",
            "Arrange Intertwine Area",
            [],
            None,
            "{id?: Intertwine group (default: the selected one), at?: [x,y] (the overlap area under this point, as a click picks it) | area?: [[x,y]…] (a closed polygon drawn around overlap areas: every area whose inside it encloses), order?: toggle|front|forward|backward|back (toggle: the object drawn right below the top comes to the front, so two objects swap; forward and backward move an object one place up or down among the objects in that area; the others keep their order there), member?: id (front, forward, backward and back: the object to move; default: front and forward move the object right below the top, back and backward the top one)} → {count: areas changed (an area already in that order is not changed, and with none changed there is no undo step), top: [id of the object now on top in each changed area]}",
            has_doc,
            arrange
        ),
    ]
}

// ---------- predicates ----------

/// The Intertwine groups of the selection: selected ones, and those around a selected object.
/// None while an object of a dynamic symbol instance's art is selected.
fn selected_groups(s: &Session) -> Vec<NodeId> {
    let Some(st) = s.active().filter(|st| st.selection.symbol_part.is_none()) else { return vec![] };
    let mut out = vec![];
    for id in &st.selection.objects {
        let group = st.doc.ancestry(*id).unwrap_or_default().into_iter().rev().find(|a| st.doc.node(*a).is_some_and(is_intertwine));
        if let Some(g) = group
            && !out.contains(&g)
        {
            out.push(g);
        }
    }
    out
}

fn has_intertwine(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    super::edit::no_symbol_part(s)?;
    if selected_groups(s).is_empty() { Err("select an Intertwine group".into()) } else { Ok(()) }
}

// ---------- Make / Release ----------

/// Why `n` can't go into an Intertwine group, if it can't (the help page's limits: no nested
/// Intertwine groups, Live Paint, Repeat or graphs).
fn refused(d: &Document, n: &Node) -> Option<&'static str> {
    let around = d.ancestry(n.id).unwrap_or_default();
    if around.iter().filter(|a| **a != n.id).filter_map(|a| d.node(*a)).any(is_intertwine) {
        return Some("an object already in an Intertwine group");
    }
    let mut why = None;
    n.walk(&mut |c| {
        why = why.or(match &c.kind {
            NodeKind::Intertwine { .. } => Some("an Intertwine group"),
            NodeKind::Repeat(_) => Some("a Repeat"),
            NodeKind::Group { .. } if c.extras().graph.is_some() => Some("a graph"),
            NodeKind::Group { .. } if vectorcraft_tools::builder::is_live_paint(c) => Some("a Live Paint group"),
            _ => None,
        });
    });
    why
}

fn make(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.intertwine.make";
    let roots = match p.get("ids") {
        Some(_) => super::edit::roots_of(&s.doc()?.doc, checked_ids_param(s, p, "ids", C)?),
        None => selected_roots(s)?,
    };
    if roots.len() < 2 {
        return Err(bad(C, "select two or more objects to intertwine"));
    }
    {
        let d = &s.doc()?.doc;
        for r in &roots {
            let n = d.node(*r).ok_or(EngineError::NoNode(*r))?;
            if n.is_layer() {
                return Err(bad(C, "layers can't be intertwined: select the objects in them"));
            }
            if let Some(what) = refused(d, n) {
                return Err(bad(C, format!("{what} can't be intertwined")));
            }
        }
    }
    let id = s.edit("Make Intertwine", |d, sel| {
        let id = d.alloc_id();
        d.wrap(&roots, |children| Node::new(id, NodeKind::Intertwine { children }))?;
        sel.set([id]);
        Ok(id)
    })?;
    choose_tool(s)?;
    Ok(json!({ "id": id.0 }))
}

/// Choose the Intertwine tool, which goes back to the tool chosen now.
fn choose_tool(s: &mut Session) -> Result<()> {
    let prev = Some(s.tool_id()).filter(|t| *t != "intertwine");
    let view = s.last_view;
    s.select_tool("intertwine", view)?;
    s.set_tool_option("prev", &json!(prev));
    Ok(())
}

fn edit(s: &mut Session, _: &Value) -> Result<Value> {
    let [g] = selected_groups(s)[..] else { return Err(bad("object.intertwine.edit", "select one Intertwine group")) };
    s.select(|_, sel| sel.set([g]))?;
    choose_tool(s)?;
    Ok(json!({ "id": g.0 }))
}

fn release(s: &mut Session, _: &Value) -> Result<Value> {
    let groups = selected_groups(s);
    let ids = s.edit("Release Intertwine", |d, sel| {
        let mut out = vec![];
        let mut released = Beside::replacing(d, &groups);
        for g in &groups {
            let Some(n) = d.node(*g).cloned() else { continue };
            let members: Vec<Node> = n
                .children()
                .into_iter()
                .flatten()
                .map(|c| {
                    let mut c = (**c).clone();
                    let mut e = c.extras_mut();
                    e.intertwine = None;
                    e.intertwine_tiers = Vec::new();
                    drop(e);
                    c
                })
                .collect();
            out.extend(members.iter().map(|c| c.id));
            released.put(d, *g, members)?;
        }
        released.finish(d)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

// ---------- Expand ----------

/// Replace Intertwine group `id` by a plain group of the art it draws
/// ([`vectorcraft_doc::intertwine::expand`]: its objects, then each part drawn over the others as a
/// clip group), keeping everything about the group itself; the generated art gets ids of its own.
fn expand_group(d: &mut Document, id: NodeId) -> Result<()> {
    let Some(mut g) = d.node(id).cloned() else { return Ok(()) };
    let NodeKind::Intertwine { children } = &g.kind else { return Ok(()) };
    g.kind = NodeKind::Group { children: vectorcraft_doc::intertwine::expand(children).into_iter().map(Arc::new).collect(), clip: false };
    super::live::fix_ids(d, &mut g);
    *d.node_mut(id).ok_or(EngineError::NoNode(id))? = g;
    Ok(())
}

/// Object › Expand: every Intertwine group in the subtrees of `roots` becomes a group of the art
/// it draws ([`expand_group`]), inner ones first so an outer one copies plain art. Returns how
/// many. A live Intertwine is expanded before it can be ungrouped:
/// <https://community.adobe.com/questions-652/expand-ungroup-800281>.
pub(crate) fn expand_intertwines(d: &mut Document, roots: &[NodeId]) -> Result<usize> {
    let mut groups = vec![];
    for r in roots {
        if let Some(n) = d.node(*r) {
            n.walk(&mut |c| {
                if is_intertwine(c) {
                    groups.push(c.id);
                }
            });
        }
    }
    for g in groups.iter().rev() {
        expand_group(d, *g)?;
    }
    Ok(groups.len())
}

// ---------- Arrange ----------

#[derive(Clone, Copy, PartialEq)]
enum Order {
    Toggle,
    Front,
    Forward,
    Backward,
    Back,
}

/// The most points a drawn `area` may have (a long freehand loop has a few thousand).
const MAX_AREA_POINTS: usize = 100_000;

/// A closed polygon from `[[x,y]…]` (three to [`MAX_AREA_POINTS`] points, finite).
fn polygon(v: &Value) -> Option<PathData> {
    let pts = v.as_array().filter(|a| a.len() <= MAX_AREA_POINTS)?;
    let pts: Vec<Point> = pts.iter().map(|q| Some(Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?))).collect::<Option<_>>()?;
    if pts.len() < 3 || pts.iter().any(|q| !(q.x.is_finite() && q.y.is_finite())) {
        return None;
    }
    Some(PathData::single(SubPath::polyline(&pts, true)))
}

/// Area `old` with `face` added (`add`) or taken out.
fn with_face(old: Option<PathData>, face: &PathData, add: bool) -> Option<PathData> {
    match (old, add) {
        (None, true) => nonempty(face.clone()),
        (None, false) => None,
        (Some(a), true) => nonempty(po::boolean(&a, FillRule::NonZero, face, FillRule::NonZero, po::BoolOp::Union)),
        (Some(a), false) => nonempty(po::boolean(&a, FillRule::NonZero, face, FillRule::NonZero, po::BoolOp::Difference)),
    }
}

/// Draw `n` at `tier` in `face` (0: in its own order): `face` joins its area of that tier and
/// leaves its areas of the other tiers.
fn set_tier(n: &mut Node, face: &PathData, tier: usize) {
    let e = n.extras();
    let mut areas: Vec<Option<PathData>> =
        std::iter::once(e.intertwine.as_deref().cloned()).chain(e.intertwine_tiers.iter().map(|a| (!a.is_empty()).then(|| a.clone()))).collect();
    if areas.len() < tier {
        areas.resize(tier, None);
    }
    for (k, a) in areas.iter_mut().enumerate() {
        *a = with_face(a.take(), face, k + 1 == tier);
    }
    while areas.len() > 1 && areas.last().is_some_and(Option::is_none) {
        areas.pop();
    }
    let mut areas = areas.into_iter();
    let first = areas.next().flatten();
    let rest: Vec<PathData> = areas.map(Option::unwrap_or_default).collect();
    if first.as_ref() != n.extras().intertwine.as_deref() || rest != n.extras().intertwine_tiers {
        let mut e = n.extras_mut();
        e.intertwine = first.map(Box::new);
        e.intertwine_tiers = rest;
    }
}

/// The tiers that draw members in `order` (bottom first): each member one tier above the member
/// below it when it comes before that member in the group's order, else at the same tier.
fn tiers(order: &[usize]) -> Vec<(usize, usize)> {
    let mut tier = 0;
    let mut below: Option<usize> = None;
    order
        .iter()
        .map(|&i| {
            if below.is_some_and(|b| i < b) {
                tier += 1;
            }
            below = Some(i);
            (i, tier)
        })
        .collect()
}

/// How far (pt) the area a member is raised in reaches past its overlap face, into the parts only
/// that member paints ([`raised_area`]).
const SEAM: f64 = 2.0;

/// The area member `i` is raised in for `face`: the face, and the parts within [`SEAM`] of it
/// where only `i` paints (`areas`: what each member paints, [`painted_areas`]). Drawing `i` again
/// there changes nothing, and the edge of the area then lies inside `i`, not on the edge of a
/// member it passes over (where the anti-aliased edges of both would let that member show
/// through as a hairline).
fn raised_area(face: &PathData, i: usize, areas: &[Option<PathData>]) -> PathData {
    let Some(own) = areas.get(i).and_then(Option::as_ref) else { return face.clone() };
    let ring = po::offset_path(face, SEAM, po::Join::Round, 4.0);
    let Some(near) = ring.bounds() else { return face.clone() };
    let mut paths = vec![(&ring, FillRule::NonZero), (own, FillRule::NonZero)];
    paths.extend(
        areas
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != i)
            .filter_map(|(_, a)| a.as_ref())
            .filter(|a| a.bounds().is_some_and(|b| b.overlaps(near)))
            .map(|a| (a, FillRule::NonZero)),
    );
    let only = po::boolean_n(&paths, |inside| inside.first() == Some(&true) && inside.get(1) == Some(&true) && !inside.iter().skip(2).any(|b| *b));
    if only.is_empty() {
        return face.clone();
    }
    nonempty(po::boolean(face, FillRule::NonZero, &only, FillRule::NonZero, po::BoolOp::Union)).unwrap_or_else(|| face.clone())
}

/// Draw the members of `face` in `order` there (bottom first), leaving the other areas as they
/// are (`areas`: what each member paints, [`painted_areas`]).
fn arrange_face(children: &mut [Arc<Node>], face: &Face, order: &[usize], areas: &[Option<PathData>]) {
    for (i, tier) in tiers(order) {
        let Some(c) = children.get_mut(i) else { continue };
        // A member drawn in its own order there with no areas has nothing to change.
        if tier == 0 && c.extras().intertwine.is_none() && c.extras().intertwine_tiers.is_empty() {
            continue;
        }
        set_tier(Arc::make_mut(c), &raised_area(&face.path, i, areas), tier);
    }
}

/// `drawn` (the members of an area, bottom first) with member `x` moved as `order` says.
fn moved(drawn: &[usize], x: usize, order: Order) -> Vec<usize> {
    let mut out = drawn.to_vec();
    let Some(at) = out.iter().position(|i| *i == x) else { return out };
    match order {
        Order::Toggle | Order::Front => {
            out.remove(at);
            out.push(x);
        }
        Order::Back => {
            out.remove(at);
            out.insert(0, x);
        }
        Order::Forward if at + 1 < out.len() => out.swap(at, at + 1),
        Order::Backward if at > 0 => out.swap(at - 1, at),
        Order::Forward | Order::Backward => {}
    }
    out
}

fn arrange(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.intertwine.arrange";
    let group = match p.get("id") {
        Some(v) => {
            let id = checked_id(s, v, C)?;
            if !s.doc()?.doc.node(id).is_some_and(is_intertwine) {
                return Err(bad(C, format!("object {} is not an Intertwine group", id.0)));
            }
            id
        }
        None => match selected_groups(s).as_slice() {
            [g] => *g,
            [] => return Err(bad(C, "select an Intertwine group")),
            _ => return Err(bad(C, "select one Intertwine group")),
        },
    };
    let at = match p.get("at") {
        Some(_) => Some(point_param(p, "at").filter(|q| q.x.is_finite() && q.y.is_finite()).ok_or_else(|| bad(C, "at must be [x, y]"))?),
        None => None,
    };
    let lasso = match p.get("area") {
        Some(v) => Some(polygon(v).ok_or_else(|| bad(C, "area must be a closed polygon of 3 to 100000 [x, y] points"))?),
        None => None,
    };
    if at.is_some() == lasso.is_some() {
        return Err(bad(C, "give either at (a click) or area (a path drawn around the overlaps)"));
    }
    let order = match str_param(p, "order").unwrap_or("toggle") {
        "toggle" => Order::Toggle,
        "front" => Order::Front,
        "forward" => Order::Forward,
        "backward" => Order::Backward,
        "back" => Order::Back,
        o => return Err(bad(C, format!("unknown order `{o}`: toggle, front, forward, backward or back"))),
    };
    let member = match p.get("member") {
        Some(v) => Some(checked_id(s, v, C)?),
        None => None,
    };
    if member.is_some() && order == Order::Toggle {
        return Err(bad(C, "member is for order front, forward, backward or back"));
    }
    let children: Vec<Arc<Node>> = s.doc()?.doc.node(group).and_then(Node::children).cloned().unwrap_or_default();
    let member = match member {
        Some(m) => Some(children.iter().position(|c| c.id == m).ok_or_else(|| bad(C, format!("object {} is not in this Intertwine group", m.0)))?),
        None => None,
    };
    let painted = painted_areas(&children);
    let all = faces_of(&painted);
    let found = match (at, &lasso) {
        (Some(p), _) => face_at(&all, p).into_iter().collect(),
        (None, Some(l)) => faces_in(&all, l),
        (None, None) => vec![],
    };
    if found.is_empty() {
        return Err(bad(
            C,
            if at.is_some() {
                "no overlap there: click where two or more objects of the group overlap"
            } else {
                "the path encloses no overlap of the group's objects"
            },
        ));
    }
    if let Some(m) = member
        && !found.iter().any(|f| f.members.contains(&m))
    {
        return Err(bad(C, "that object is not in the overlap"));
    }
    // The new order of each area that changes; areas already in that order are left out.
    let mut plans: Vec<(&Face, Vec<usize>)> = vec![];
    for face in found.iter().filter(|f| member.is_none_or(|m| f.members.contains(&m))) {
        let drawn = drawn_order(&children, face);
        let x = match (order, member) {
            (_, Some(m)) => Some(m),
            (Order::Toggle | Order::Front | Order::Forward, None) => drawn.iter().rev().nth(1).copied(),
            (Order::Back | Order::Backward, None) => drawn.last().copied(),
        };
        let Some(x) = x else { continue };
        let new = moved(&drawn, x, order);
        if new != drawn {
            plans.push((face, new));
        }
    }
    let tops: Vec<u64> = plans.iter().filter_map(|(_, new)| children.get(*new.last()?)).map(|c| c.id.0).collect();
    if plans.is_empty() {
        return Ok(json!({ "count": 0, "top": tops }));
    }
    s.edit("Intertwine", |d, _| {
        let n = d.node_mut(group).ok_or(EngineError::NoNode(group))?;
        let NodeKind::Intertwine { children } = &mut n.kind else { return Err(EngineError::NoNode(group)) };
        for (face, new) in &plans {
            arrange_face(children, face, new, &painted);
        }
        Ok(())
    })?;
    Ok(json!({ "count": plans.len(), "top": tops }))
}
