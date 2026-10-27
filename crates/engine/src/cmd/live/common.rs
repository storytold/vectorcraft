use super::mesh::whole;
use super::*;

pub(super) fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// Explicit `ids` (in paint order) or the selected roots.
pub(super) fn roots_param(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    if let Some(ids) = ids_param(p, "ids") {
        let st = s.doc()?;
        for id in &ids {
            if st.doc.node(*id).is_none() {
                return Err(EngineError::NoNode(*id));
            }
        }
        let mut sel = Selection::default();
        sel.set(ids.iter().copied().filter(|id| st.doc.node(*id).is_some_and(|n| !n.is_layer())));
        let ordered = sel.in_paint_order(&st.doc);
        // Drop ids nested inside other given ids.
        return Ok(ordered
            .iter()
            .copied()
            .filter(|id| {
                let anc = st.doc.ancestry(*id).unwrap_or_default();
                !anc[..anc.len().saturating_sub(1)].iter().any(|a| ordered.contains(a))
            })
            .collect());
    }
    selected_roots(s)
}

/// Selected nodes of a kind (the selection itself or an ancestor).
pub(super) fn selected_of(s: &Session, pred: fn(&Node) -> bool) -> Vec<NodeId> {
    let Some(st) = s.active() else { return vec![] };
    let mut out = vec![];
    for id in &st.selection.objects {
        for a in st.doc.ancestry(*id).unwrap_or_default().into_iter().rev() {
            if st.doc.node(a).is_some_and(pred) {
                if !out.contains(&a) {
                    out.push(a);
                }
                break;
            }
        }
    }
    out
}

pub(super) fn is_blend(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Blend { .. })
}

pub(super) fn is_envelope(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Envelope { .. })
}

pub(super) fn is_mesh(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Mesh(_))
}

pub(super) fn has_blend(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_of(s, is_blend).is_empty() { Err("select a blend".into()) } else { Ok(()) }
}

pub(super) fn has_envelope(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_of(s, is_envelope).is_empty() { Err("select an envelope".into()) } else { Ok(()) }
}

pub(super) fn has_mesh(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_of(s, is_mesh).is_empty() { Err("select a gradient mesh".into()) } else { Ok(()) }
}

/// Wrap `roots` (paint order) into a new node built by `make`, placed where the topmost root was.
pub(super) fn wrap(d: &mut Document, sel: &mut Selection, roots: &[NodeId], make: impl FnOnce(NodeId, Vec<Arc<Node>>) -> Node) -> Result<NodeId> {
    if roots.is_empty() {
        return Err(EngineError::Other("nothing to wrap".into()));
    }
    let id = d.alloc_id();
    d.wrap(roots, |nodes| make(id, nodes))?;
    sel.set([id]);
    Ok(id)
}

/// Put `nodes` where `id` was (replacing it). Returns their ids.
pub(super) fn replace_with(d: &mut Document, id: NodeId, nodes: Vec<Node>) -> Result<Vec<NodeId>> {
    let (par, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    d.remove(id)?;
    let mut out = vec![];
    for (k, n) in nodes.into_iter().enumerate() {
        out.push(n.id);
        d.insert(par, idx + k, n)?;
    }
    Ok(out)
}

/// Fresh ids for any id-0 (generated) node in `n`'s subtree; keeps real ids.
pub(in super::super) fn fix_ids(d: &mut Document, n: &mut Node) {
    if n.id == NodeId(0) {
        n.id = d.alloc_id();
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            fix_ids(d, Arc::make_mut(c));
        }
    }
}

// ---------- Blend ----------

/// The Specified Steps and Specified Distance a spacing mode starts with.
pub(super) const DEFAULT_STEPS: u32 = 5;

pub(super) const DEFAULT_DISTANCE: f64 = 10.0;

pub(super) fn spacing_param(p: &Value, cmd: &str, current: BlendSpacing) -> Result<BlendSpacing> {
    check_numbers(p, &["value", "steps", "distance"], cmd)?;
    let smooth = switch_or(p, "smooth", false, cmd)?;
    let num = |k: &str| p.get(k).and_then(Value::as_f64);
    let steps = |k: &str, n: f64| whole(n, k, cmd).map(|n| BlendSpacing::Steps(n as u32));
    let mode = str_param(p, "spacing");
    let value = num("value");
    let sp = match mode {
        Some("smooth") | Some("smoothColor") => BlendSpacing::SmoothColor,
        Some("steps") => match value {
            Some(v) => steps("value", v)?,
            None => steps("steps", num("steps").unwrap_or(DEFAULT_STEPS as f64))?,
        },
        Some("distance") => BlendSpacing::Distance(value.or(num("distance")).unwrap_or(DEFAULT_DISTANCE)),
        Some(o) => return Err(bad(cmd, format!("unknown spacing `{o}` (smooth|steps|distance)"))),
        None => {
            if let Some(n) = num("steps") {
                steps("steps", n)?
            } else if let Some(d) = num("distance") {
                BlendSpacing::Distance(d)
            } else if smooth {
                BlendSpacing::SmoothColor
            } else {
                current
            }
        }
    };
    match sp {
        BlendSpacing::Steps(n) if !(1..=1000).contains(&n) => Err(bad(cmd, "steps must be 1..1000")),
        BlendSpacing::Distance(d) if !(d.is_finite() && d > 0.0) => Err(bad(cmd, "distance must be > 0")),
        s => Ok(s),
    }
}

pub(super) fn orientation_param(p: &Value, current: BlendOrientation) -> BlendOrientation {
    match str_param(p, "orientation") {
        Some("path") | Some("alignToPath") => BlendOrientation::AlignToPath,
        Some("page") | Some("alignToPage") => BlendOrientation::AlignToPage,
        _ => current,
    }
}

/// Blend panel Type and Ramp, validated before any document mutation.
pub(super) fn ease_param(p: &Value, cmd: &str, current: BlendEase) -> Result<BlendEase> {
    let kind = match p.get("easingType").filter(|v| !v.is_null()) {
        None => current.kind,
        Some(v) => match v.as_str() {
            Some("easeIn") => BlendEaseType::In,
            Some("easeOut") => BlendEaseType::Out,
            Some("easeInOut") => BlendEaseType::InOut,
            Some("easeOutIn") => BlendEaseType::OutIn,
            _ => return Err(bad(cmd, "easingType must be easeIn, easeOut, easeInOut or easeOutIn")),
        },
    };
    Ok(BlendEase { kind, ramp: percent_param(p, cmd, "ramp", current.ramp * 100.0, 0.0)? / 100.0 })
}

/// Bounded percentage input; missing or null preserves the current value.
pub(super) fn percent_param(p: &Value, cmd: &str, key: &str, current: f64, min: f64) -> Result<f64> {
    match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(current),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && (min..=100.0).contains(n))
            .ok_or_else(|| bad(cmd, format!("{key} must be a percentage {min}..100"))),
    }
}

/// `starts`: per object of `ids` (else of `roots`), the anchor its blend starts from.
pub(super) fn starts_param(p: &Value, roots: &[NodeId]) -> Result<Vec<(NodeId, u32)>> {
    const C: &str = "object.blend.make";
    let Some(v) = p.get("starts").filter(|v| !v.is_null()) else { return Ok(vec![]) };
    let list = v.as_array().ok_or_else(|| bad(C, "`starts` must be an array of anchor indices (or nulls)"))?;
    let owners = ids_param(p, "ids").unwrap_or_else(|| roots.to_vec());
    let mut out = vec![];
    for (id, a) in owners.iter().zip(list) {
        if a.is_null() {
            continue;
        }
        let i = a.as_u64().and_then(|i| u32::try_from(i).ok()).ok_or_else(|| bad(C, "a start must be an anchor index ≥ 0 or null"))?;
        out.push((*id, i));
    }
    Ok(out)
}
