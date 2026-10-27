use super::*;

/// The most data entries `object.setProps` takes at once.
const MAX_DATA: usize = 256;

/// `object.setProps`'s `data`: each key (a data-* attribute's name: letters, digits, `-`, `_`,
/// `.`) with its value, or `None` to remove it.
fn data_param(m: &serde_json::Map<String, Value>) -> Result<Vec<(String, Option<String>)>> {
    const C: &str = "object.setProps";
    if m.len() > MAX_DATA {
        return Err(bad(C, format!("data takes at most {MAX_DATA} keys at once")));
    }
    m.iter()
        .map(|(k, v)| {
            let k = k.strip_prefix("data-").unwrap_or(k);
            if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) || k == "name" || k.starts_with("vc-") {
                return Err(bad(C, format!("data key `{k}`: letters, digits, -, _ and . (not name or vc-…)")));
            }
            let v = match v {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            };
            Ok((k.to_string(), v))
        })
        .collect()
}

pub(super) fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &["visible", "locked", "isolate", "knockoutShape"], "object.setProps")?;
    check_numbers(p, &["opacity"], "object.setProps")?;
    let ids = targets(s, p)?;
    let opacity = p.get("opacity").and_then(Value::as_f64).map(percent);
    let blend = match str_param(p, "blend") {
        Some(b) => Some(BlendMode::parse(b).ok_or_else(|| bad("object.setProps", format!("unknown blend mode `{b}`")))?),
        None => None,
    };
    let data = match p.get("data") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => Some(data_param(m)?),
        Some(v) => return Err(bad("object.setProps", format!("data must be an object of key: value, not {v}"))),
    };
    let knockout = match p.get("knockout") {
        Some(v) => Some(
            Knockout::from_value(v)
                .ok_or_else(|| bad("object.setProps", format!("knockout must be \"on\", \"off\", \"neutral\" or a bool, not {v}")))?,
        ),
        None => None,
    };
    let hides = p.get("visible").and_then(Value::as_bool) == Some(false) || p.get("locked").and_then(Value::as_bool) == Some(true);
    // When every object already has every value given, the document and its undo history stay as
    // they are.
    let flag = |k: &str| p.get(k).and_then(Value::as_bool);
    let has_values = |n: &Node| {
        str_param(p, "name").is_none_or(|v| n.name.as_deref() == Some(v).filter(|v| !v.is_empty()))
            && flag("visible").is_none_or(|v| n.visible == v)
            && flag("locked").is_none_or(|v| n.locked == v)
            && opacity.is_none_or(|v| n.opacity == v)
            && blend.is_none_or(|b| n.blend == b)
            && flag("isolate").is_none_or(|v| n.isolate == v)
            && knockout.is_none_or(|k| n.knockout == k)
            && flag("knockoutShape").is_none_or(|v| n.knockout_shape == v)
            && data.iter().flatten().all(|(k, v)| {
                n.extras().attrs.as_ref().and_then(|a| a.data.iter().find(|(key, _)| key == k)).map(|(_, now)| now.as_str()) == v.as_deref()
            })
    };
    let d = &s.doc()?.doc;
    if ids.iter().all(|id| d.node(*id).is_some_and(has_values)) {
        // Hidden or locked objects can't stay selected.
        if hides {
            s.select(|d, sel| sel.deselect_uneditable(d))?;
        }
        return ok();
    }
    s.edit("Object Properties", |d, sel| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            if let Some(v) = str_param(p, "name") {
                n.name = if v.is_empty() { None } else { Some(v.to_string()) };
            }
            if let Some(v) = p.get("visible").and_then(Value::as_bool) {
                n.visible = v;
            }
            if let Some(v) = p.get("locked").and_then(Value::as_bool) {
                n.locked = v;
            }
            if let Some(v) = opacity {
                n.opacity = v;
            }
            if let Some(b) = blend {
                n.blend = b;
            }
            if let Some(v) = p.get("isolate").and_then(Value::as_bool) {
                n.isolate = v;
            }
            if let Some(k) = knockout {
                n.knockout = k;
            }
            if let Some(v) = p.get("knockoutShape").and_then(Value::as_bool) {
                n.knockout_shape = v;
            }
            if let Some(changes) = &data {
                n.edit_attrs(|a| {
                    for (k, v) in changes {
                        match (a.data.iter().position(|(key, _)| key == k), v) {
                            (Some(i), Some(v)) => {
                                if let Some(e) = a.data.get_mut(i) {
                                    e.1 = v.clone();
                                }
                            }
                            (Some(i), None) => {
                                a.data.remove(i);
                            }
                            (None, Some(v)) => a.data.push((k.clone(), v.clone())),
                            (None, None) => {}
                        }
                    }
                });
            }
        }
        // Hidden or locked objects can't stay selected.
        if hides {
            sel.deselect_uneditable(d);
        }
        Ok(())
    })?;
    ok()
}
