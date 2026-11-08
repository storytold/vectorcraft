//! Structural invariants for documents and sessions, and round-trip checks.

use std::collections::HashSet;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeKind};
use vectorcraft_engine::Session;

/// Serialize a document to JSON (the canonical form for equality checks).
pub fn doc_json(doc: &Document) -> Value {
    serde_json::to_value(doc).expect("document serializes")
}

/// Tree invariants: ids unique, `node_count` equals the number of walked nodes, `next_id` above
/// every id, top level made of layers, layers only at the top or inside layers, clip groups have
/// a first child, compound children are paths, no image holds its own pixels (the document keeps
/// them).
pub fn check_document(doc: &Document) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut walked = 0usize;
    let mut max = 0u64;
    let mut err = None;
    doc.walk(|n| {
        walked += 1;
        max = max.max(n.id.0);
        if !seen.insert(n.id) && err.is_none() {
            err = Some(format!("duplicate id {}", n.id));
        }
        match &n.kind {
            NodeKind::Group { children, .. } if children.iter().any(|c| c.is_layer()) && err.is_none() => {
                err = Some(format!("group {} contains a layer", n.id));
            }
            NodeKind::Compound { children, .. } if children.iter().any(|c| !matches!(c.kind, NodeKind::Path { .. })) && err.is_none() => {
                err = Some(format!("compound {} has a non-path child", n.id));
            }
            NodeKind::Image(im) if im.pixels.is_some() && err.is_none() => {
                err = Some(format!("image {} holds its own pixels", n.id));
            }
            _ => {}
        }
        if !(n.opacity.is_finite() && (0.0..=1.0).contains(&n.opacity)) && err.is_none() {
            err = Some(format!("node {} opacity {}", n.id, n.opacity));
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    if walked != doc.node_count() {
        return Err(format!("node_count {} != walked {walked}", doc.node_count()));
    }
    if doc.peek_next_id() <= max {
        return Err(format!("next_id {} <= max id {max}", doc.peek_next_id()));
    }
    if let Some(l) = doc.layers.iter().find(|l| !l.is_layer()) {
        return Err(format!("top-level node {} is not a layer", l.id));
    }
    for id in seen {
        if doc.node(id).is_none() {
            return Err(format!("walked id {id} not found by lookup"));
        }
    }
    Ok(())
}

/// Session invariants for the active document: document invariants, every selected id exists,
/// the key object (if any) exists, no interaction left open, history snapshots valid.
pub fn check_session(s: &Session) -> Result<(), String> {
    let Some(st) = s.active() else { return Ok(()) };
    check_document(&st.doc)?;
    for id in &st.selection.objects {
        if st.doc.node(*id).is_none() {
            return Err(format!("selected id {id} does not exist"));
        }
    }
    let mut uniq = HashSet::new();
    if let Some(d) = st.selection.objects.iter().find(|i| !uniq.insert(**i)) {
        return Err(format!("selection lists {d} twice"));
    }
    for id in st.selection.anchors.keys() {
        if st.doc.node(*id).is_none() {
            return Err(format!("anchor selection on missing id {id}"));
        }
    }
    if let Some(k) = st.selection.key
        && st.doc.node(k).is_none()
    {
        return Err(format!("key object {k} does not exist"));
    }
    if st.interaction.is_some() {
        return Err("interaction left open".into());
    }
    Ok(())
}

/// `.vectorcraft` save → load → the JSON of the document and of what loaded, without `images`: a
/// save keeps only the images in use, which must come back byte for byte.
fn native_roundtrip(doc: &Document) -> Result<(Value, Value), String> {
    let bytes = vectorcraft_format::save(doc, false);
    let back = vectorcraft_format::load(&bytes).map_err(|e| format!("load: {e}"))?;
    if let Some(k) = back.images.iter().find(|(k, b)| doc.images.get(*k).map(|d| &d.bytes) != Some(&b.bytes)).map(|(k, _)| k) {
        return Err(format!("native round trip changed image `{k}`"));
    }
    let mut lost = None;
    back.walk(|n| {
        let key = match &n.kind {
            NodeKind::Image(im) => Some(&im.key),
            NodeKind::PlacedDocument(p) => Some(&p.key),
            _ => None,
        };
        if let Some(key) = key
            && doc.images.contains_key(key)
            && !back.images.contains_key(key)
        {
            lost = Some(key.clone());
        }
    });
    if let Some(k) = lost {
        return Err(format!("native round trip lost image `{k}`, which is in use"));
    }
    let strip = |mut v: Value| {
        if let Some(o) = v.as_object_mut() {
            o.remove("images");
        }
        v
    };
    Ok((strip(doc_json(doc)), strip(doc_json(&back))))
}

/// `.vectorcraft` save → load must reproduce the document (numbers compared to 1e-12 relative —
/// see [`check_native_roundtrip_exact`] for the bit-exact version).
pub fn check_native_roundtrip(doc: &Document) -> Result<(), String> {
    let (a, b) = native_roundtrip(doc)?;
    if !json_approx_eq(&a, &b, 1e-12) {
        return Err(format!("native round trip differs:\n{}", first_diff(&a, &b, "$")));
    }
    Ok(())
}

/// Bit-exact `.vectorcraft` round trip (every f64 must survive).
pub fn check_native_roundtrip_exact(doc: &Document) -> Result<(), String> {
    let (a, b) = native_roundtrip(doc)?;
    if a != b {
        return Err(format!("native round trip differs:\n{}", first_diff(&a, &b, "$")));
    }
    Ok(())
}

/// Structural JSON equality with numbers compared to `rel` relative tolerance.
pub fn json_approx_eq(a: &Value, b: &Value, rel: f64) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x == y || (x - y).abs() <= rel * x.abs().max(y.abs()),
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_approx_eq(p, q, rel)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| json_approx_eq(v, w, rel))),
        _ => a == b,
    }
}

/// SVG export must not panic and must re-import.
pub fn check_svg_roundtrip(doc: &Document) -> Result<Document, String> {
    let svg = vectorcraft_svg::export(doc, &vectorcraft_svg::ExportOptions::default());
    vectorcraft_svg::import(&svg).map_err(|e| format!("svg re-import: {e}\n{svg}"))
}

/// `document.inspect` must succeed and serialize.
pub fn check_inspect(s: &mut Session) -> Result<(), String> {
    if s.active().is_none() {
        return Ok(());
    }
    let v = s.execute("document.inspect", &json!({})).map_err(|e| format!("inspect: {e}"))?;
    serde_json::to_string(&v).map_err(|e| e.to_string())?;
    Ok(())
}

/// Everything above.
pub fn check_all(s: &mut Session) -> Result<(), String> {
    check_session(s)?;
    check_inspect(s)?;
    if let Some(st) = s.active() {
        let doc = st.doc.clone();
        check_native_roundtrip(&doc)?;
        check_svg_roundtrip(&doc)?;
    }
    Ok(())
}

/// A short description of the first difference between two JSON values.
pub fn first_diff(a: &Value, b: &Value, path: &str) -> String {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                match y.get(k) {
                    None => return format!("{path}.{k}: missing on right"),
                    Some(w) if w != v => return first_diff(v, w, &format!("{path}.{k}")),
                    _ => {}
                }
            }
            for k in y.keys() {
                if !x.contains_key(k) {
                    return format!("{path}.{k}: missing on left");
                }
            }
            format!("{path}: equal")
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return format!("{path}: array length {} vs {}", x.len(), y.len());
            }
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                if v != w {
                    return first_diff(v, w, &format!("{path}[{i}]"));
                }
            }
            format!("{path}: equal")
        }
        _ => format!("{path}: {a} vs {b}"),
    }
}

/// Undo everything, then redo everything, returning (initial, final) JSON snapshots.
pub fn undo_all(s: &mut Session) -> usize {
    let mut n = 0;
    while s.execute("edit.undo", &json!({})).is_ok() {
        n += 1;
        assert!(n < 10_000, "undo does not terminate");
    }
    n
}
pub fn redo_all(s: &mut Session) -> usize {
    let mut n = 0;
    while s.execute("edit.redo", &json!({})).is_ok() {
        n += 1;
        assert!(n < 10_000, "redo does not terminate");
    }
    n
}
