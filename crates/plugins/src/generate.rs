//! Generator plug-ins: a plug-in of kind `generator` draws art from its parameters alone. The host
//! keeps the art in a generated art object with the parameters it was drawn with
//! ([`vectorcraft_doc::generated`]), so the art can be drawn again when the parameters change.
//!
//! A run is given no objects (`{"objects": []}`) and its checked parameters with
//! `_context.mode` `"generate"`, and returns the art's objects in the form filters return new
//! objects ([`crate::objects`]): `{"objects": [...]}`, or `{"error": "…"}` to decline. Each object
//! is new (no `id`) and goes through a filter's checks. The parameters a generated art object keeps
//! are at most [`MAX_PARAMS_BYTES`] of JSON. A run takes the plug-in's own budget, and Cancel stops
//! it between two slices of it ([`Error::Cancelled`]).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::objects::{OutObject, declined, printable};
use crate::{Error, Kind, Limits, Plugin, Result, registry};

pub use vectorcraft_doc::generated::MAX_PARAMS_BYTES;

/// The keys a generator's output may have.
const OUTPUT_KEYS: &[&str] = &["objects", "error"];

/// What a generator is given in place of objects.
const INPUT: &[u8] = br#"{"objects":[]}"#;

/// The installed generators, by plug-in id.
pub fn installed() -> Vec<Arc<Plugin>> {
    registry::list().into_iter().filter(|p| p.manifest().kind == Kind::Generator).collect()
}

/// The installed generator with plug-in id `id`. A plug-in of another kind, and one that isn't
/// installed, are errors that say so.
pub fn named(id: &str) -> Result<Arc<Plugin>> {
    let plugin = registry::get(id).ok_or_else(|| Error::NotFound(id.into()))?;
    match plugin.manifest().kind {
        Kind::Generator => Ok(plugin),
        _ => Err(not_generator(plugin.manifest())),
    }
}

/// The error of plug-in `m`, which is not a generator, asked to draw generated art.
fn not_generator(m: &crate::Manifest) -> Error {
    Error::Params(format!("`{}` is a plug-in of kind `{}`, not a generator", m.id, m.kind.name()))
}

/// The parameters `params` checked against generator `plugin`'s manifest (missing ones take their
/// defaults, others are errors), at most [`MAX_PARAMS_BYTES`] of JSON: what a generated art object
/// keeps.
pub fn resolve(plugin: &Plugin, params: &Value) -> Result<Map<String, Value>> {
    resolve_over(plugin, &Map::new(), params)
}

/// [`resolve`] for parameters `params` that change the `saved` ones of a generated art object:
/// `params` is checked as [`resolve`] checks it, and the saved values the manifest still declares
/// fill in the rest, brought into its ranges as a filter's are.
pub fn resolve_over(plugin: &Plugin, saved: &Map<String, Value>, params: &Value) -> Result<Map<String, Value>> {
    let m = plugin.manifest();
    if m.kind != Kind::Generator {
        return Err(not_generator(m));
    }
    let given = match params {
        Value::Null => &Map::new(),
        Value::Object(given) => given,
        _ => return Err(Error::Params("plug-in parameters must be a JSON object".into())),
    };
    strict(m, given)?;
    let mut merged: Map<String, Value> =
        saved.iter().filter(|(k, _)| m.params.iter().any(|(name, _)| name == *k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    merged.extend(given.iter().map(|(k, v)| (k.clone(), v.clone())));
    let resolved = m.resolve_params(&Value::Object(merged))?;
    if serde_json::to_vec(&resolved).map_or(usize::MAX, |b| b.len()) > MAX_PARAMS_BYTES {
        return Err(Error::Params(format!("the parameters are larger than the {MAX_PARAMS_BYTES} bytes a generated art object keeps")));
    }
    Ok(resolved)
}

/// Refuses what [`crate::Manifest::resolve_params`] would bring into range: a parameter `m` does not
/// declare, a number (or a number of a list) outside its range and text longer than its limit are
/// errors, so the parameters a generated art object keeps are the ones it was given.
fn strict(m: &crate::Manifest, given: &Map<String, Value>) -> Result<()> {
    use crate::manifest::ParamSpec;
    for (k, v) in given {
        let Some((_, p)) = m.params.iter().find(|(name, _)| name == k) else {
            return Err(Error::Params(format!("`{k}` is not a parameter of `{}`", m.id)));
        };
        if v.is_null() {
            continue;
        }
        let range = |min: &dyn std::fmt::Display, max: &dyn std::fmt::Display| Error::Params(format!("`{k}` must be from {min} to {max}"));
        match &p.spec {
            ParamSpec::Number { min, max, .. } if v.as_f64().is_some_and(|f| !(*min..=*max).contains(&f)) => return Err(range(min, max)),
            ParamSpec::Int { min, max, .. } if int_outside(v, *min, *max) => return Err(range(min, max)),
            ParamSpec::NumberList { min, max, .. }
                if v.as_array().is_some_and(|items| items.iter().any(|x| x.as_f64().is_some_and(|f| !(*min..=*max).contains(&f)))) =>
            {
                return Err(Error::Params(format!("each number of `{k}` must be from {min} to {max}")));
            }
            ParamSpec::Text { max_length, .. } if v.as_str().is_some_and(|s| s.chars().count() > *max_length) => {
                return Err(Error::Params(format!("`{k}` must be at most {max_length} characters")));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Is `v`, a number given for a whole-number parameter, outside `min..=max` (rounded, as
/// [`crate::Manifest::resolve_params`] rounds it)? A value that is not a number is not. The rounded
/// value is compared with the bounds as integers: an `i64` bound past 2^53 has no exact `f64`.
fn int_outside(v: &Value, min: i64, max: i64) -> bool {
    // -2^63 and 2^63, exact as f64: a whole number in between converts to i64 exactly.
    const LOW: f64 = -9_223_372_036_854_775_808.0;
    const HIGH: f64 = 9_223_372_036_854_775_808.0;
    match v.as_i64() {
        Some(i) => !(min..=max).contains(&i),
        None => v.is_u64() || v.as_f64().map(f64::round).is_some_and(|r| !(LOW..HIGH).contains(&r) || !(min..=max).contains(&(r as i64))),
    }
}

/// Runs generator `plugin` with `params` ([`resolve`]d) under `limits` → the objects of its art, in
/// paint order (bottom first), checked as a filter's new objects are.
pub fn run(plugin: &Plugin, params: &Map<String, Value>, limits: &Limits) -> Result<Vec<OutObject>> {
    let m = plugin.manifest();
    if m.kind != Kind::Generator {
        return Err(not_generator(m));
    }
    let mut params = Value::Object(params.clone());
    params["_context"] = json!({"mode": "generate"});
    let params = serde_json::to_vec(&params).map_err(|e| Error::Params(e.to_string()))?;
    decode(&plugin.run_limited(INPUT, &params, limits)?)
}

/// A generator's output: `{"objects": [...]}` of new objects (no `id`), or `{"error": "…"}` to
/// decline with a message for the user. An output without objects, with an object that has an
/// `id`, or with keys of its own, is refused.
pub fn decode(bytes: &[u8]) -> Result<Vec<OutObject>> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Output(format!("not JSON: {e}")))?;
    if let Some(e) = declined(&v) {
        return Err(e);
    }
    if let Some(key) = v.as_object().and_then(|o| o.keys().find(|k| !OUTPUT_KEYS.contains(&k.as_str()))) {
        let key = printable(&json!(key), 64);
        return Err(Error::Output(format!("a generator returns `objects`, not `{key}`")));
    }
    let objects = crate::objects::decode(bytes)?;
    if objects.is_empty() {
        return Err(Error::Output("the generator returned no objects".into()));
    }
    if let Some(i) = objects.iter().position(|o| o.id.is_some()) {
        return Err(crate::objects::out_err(&format!("objects[{i}]"), "a generator's objects are new: leave `id` out"));
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r#"{"type":"path","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[10,0]},{"p":[10,10]}]}]}}"#;

    #[test]
    fn output_is_new_objects_or_a_decline() {
        let out = decode(format!(r#"{{"objects":[{SQUARE},{SQUARE}]}}"#).as_bytes()).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|o| o.id.is_none() && o.path.is_some()));
        assert_eq!(decode(br#"{"error": "No\nstars."}"#), Err(Error::Failed("Nostars.".into())));
    }

    /// Nothing a generator returns reaches a document unchecked: malformed JSON, no objects, keys
    /// the host doesn't read, objects with ids and bad objects are all errors.
    #[test]
    fn bad_output_is_refused() {
        let far = r#"{"objects":[{"path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,0]},{"p":[1e9,0]}]}]}}]}"#;
        let with_id = format!(r#"{{"objects":[{}]}}"#, SQUARE.replacen('{', r#"{"id":4,"#, 1));
        for (bad, says) in [
            (&b"[]"[..], "objects"),
            (b"{}", "objects"),
            (b"not json", "not JSON"),
            (br#"{"objects":[]}"#, "no objects"),
            (br#"{"objects":[1]}"#, "JSON object"),
            (far.as_bytes(), "out of range"),
            (br#"{"objects":[],"documentData":{}}"#, "not `documentData`"),
            (with_id.as_bytes(), "leave `id` out"),
        ] {
            let e = decode(bad).unwrap_err().to_string();
            assert!(e.contains(says), "{}: {e}", String::from_utf8_lossy(bad));
        }
    }

    /// A number of a list outside the list's range is an error, not brought into range.
    #[test]
    fn a_number_list_entry_out_of_range_is_refused() {
        let m: crate::Manifest = serde_json::from_value(json!({
            "id": "t.gen", "name": "T", "kind": "generator",
            "params": {"stops": {"type": "numberList", "min": 0, "max": 10, "default": [1]}}
        }))
        .unwrap();
        let given = |v: Value| json!({"stops": v}).as_object().cloned().unwrap();
        assert!(strict(&m, &given(json!([0, 5, 10]))).is_ok());
        // A declared name may be null (its default); an undeclared one is an error even when null.
        assert!(strict(&m, &given(Value::Null)).is_ok());
        let typo = json!({"stosp": null}).as_object().cloned().unwrap();
        let e = strict(&m, &typo).unwrap_err().to_string();
        assert!(e.contains("`stosp` is not a parameter of `t.gen`"), "{e}");
        for bad in [json!([100]), json!([1, -0.5])] {
            let e = strict(&m, &given(bad.clone())).unwrap_err().to_string();
            assert!(e.contains("each number of `stops` must be from 0 to 10"), "{bad}: {e}");
        }
    }

    /// A whole-number parameter's bounds are compared exactly, also past 2^53, where `max as f64`
    /// rounds up to the next value an f64 holds; a fraction is still rounded first.
    #[test]
    fn a_whole_number_just_past_a_large_maximum_is_refused() {
        let m: crate::Manifest = serde_json::from_value(json!({
            "id": "t.gen", "name": "T", "kind": "generator",
            "params": {"n": {"type": "int", "min": -9007199254740995_i64, "max": 9007199254740995_i64}}
        }))
        .unwrap();
        let given = |v: Value| json!({"n": v}).as_object().cloned().unwrap();
        for ok in [json!(9007199254740995_i64), json!(3.6), json!(-9007199254740995_i64), json!(9007199254740994.0)] {
            assert!(strict(&m, &given(ok.clone())).is_ok(), "{ok}");
        }
        for bad in [json!(9007199254740996.0), json!(-9007199254740996.0), json!(1e300), json!(-1e300), json!(u64::MAX)] {
            let e = strict(&m, &given(bad.clone())).unwrap_err().to_string();
            assert!(e.contains("`n` must be from -9007199254740995 to 9007199254740995"), "{bad}: {e}");
        }
    }
}
