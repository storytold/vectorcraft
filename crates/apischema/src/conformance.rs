//! For tests: a check of JSON Schema 2020-12 documents, and a validator of instances against them,
//! written from the keyword rules of the Core and Validation specifications
//! (<https://json-schema.org/draft/2020-12>). It reads only the JSON and shares no code with the
//! writer it checks. It covers the keywords the writer uses and reports any other keyword that
//! isn't an `x-` annotation.

use serde_json::Value;

const TYPES: &[&str] = &["null", "boolean", "object", "array", "number", "string", "integer"];

fn unique(items: &[Value]) -> bool {
    items.iter().enumerate().all(|(i, x)| items.iter().take(i).all(|y| y != x))
}

fn schema_list(items: &Value, at: &str, key: &str) -> Result<(), String> {
    let list = items.as_array().filter(|l| !l.is_empty()).ok_or_else(|| format!("{at}: `{key}` must be a nonempty array"))?;
    for (i, x) in list.iter().enumerate() {
        check_schema(x, &format!("{at}/{key}/{i}"))?;
    }
    Ok(())
}

/// Whether `schema` follows the 2020-12 rules for the keywords it uses, with where it doesn't.
pub fn check_schema(schema: &Value, at: &str) -> Result<(), String> {
    let object = match schema {
        Value::Bool(_) => return Ok(()),
        Value::Object(o) => o,
        other => return Err(format!("{at}: a schema must be an object or a boolean, not {other}")),
    };
    for (key, value) in object {
        let number = || if value.is_number() { Ok(()) } else { Err(format!("{at}: `{key}` must be a number")) };
        match key.as_str() {
            "$schema" => {
                let uri = value.as_str().ok_or_else(|| format!("{at}: `$schema` must be a string"))?;
                if !uri.contains("://") || uri.ends_with('#') {
                    return Err(format!("{at}: `$schema` must be a normalized URI"));
                }
            }
            "type" => {
                let known = |v: &Value| v.as_str().is_some_and(|s| TYPES.contains(&s));
                let ok = known(value) || value.as_array().is_some_and(|l| !l.is_empty() && unique(l) && l.iter().all(known));
                if !ok {
                    return Err(format!("{at}: `type` must name JSON types, not {value}"));
                }
            }
            "enum" => {
                if !value.as_array().is_some_and(|l| !l.is_empty() && unique(l)) {
                    return Err(format!("{at}: `enum` must be a nonempty array of unique values"));
                }
            }
            "properties" => {
                let props = value.as_object().ok_or_else(|| format!("{at}: `properties` must be an object"))?;
                for (name, x) in props {
                    check_schema(x, &format!("{at}/properties/{name}"))?;
                }
            }
            "required" => {
                if !value.as_array().is_some_and(|l| unique(l) && l.iter().all(Value::is_string)) {
                    return Err(format!("{at}: `required` must be an array of unique strings"));
                }
            }
            "additionalProperties" | "items" => check_schema(value, &format!("{at}/{key}"))?,
            "prefixItems" | "anyOf" => schema_list(value, at, key)?,
            "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" => number()?,
            "minItems" => {
                if value.as_u64().is_none() {
                    return Err(format!("{at}: `minItems` must be a non-negative integer"));
                }
            }
            "description" => {
                if !value.is_string() {
                    return Err(format!("{at}: `description` must be a string"));
                }
            }
            "deprecated" => {
                if !value.is_boolean() {
                    return Err(format!("{at}: `deprecated` must be a boolean"));
                }
            }
            "default" => {
                if !instance_valid(schema, value) {
                    return Err(format!("{at}: `default` {value} does not validate against its schema"));
                }
            }
            k if k.starts_with("x-") => {}
            other => return Err(format!("{at}: unexpected keyword `{other}`")),
        }
    }
    Ok(())
}

fn has_type(v: &Value, name: &str) -> bool {
    match name {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|x| x.fract() == 0.0),
        _ => false,
    }
}

/// Whether the instance `v` is valid against `schema`.
pub fn instance_valid(schema: &Value, v: &Value) -> bool {
    let s = match schema {
        Value::Bool(b) => return *b,
        Value::Object(o) => o,
        _ => return false,
    };
    let type_ok = match s.get("type") {
        None => true,
        Some(Value::String(t)) => has_type(v, t),
        Some(Value::Array(l)) => l.iter().filter_map(Value::as_str).any(|t| has_type(v, t)),
        Some(_) => false,
    };
    if !type_ok {
        return false;
    }
    if let Some(list) = s.get("enum").and_then(Value::as_array)
        && !list.contains(v)
    {
        return false;
    }
    if let Some(any) = s.get("anyOf").and_then(Value::as_array)
        && !any.iter().any(|x| instance_valid(x, v))
    {
        return false;
    }
    if let Some(x) = v.as_f64() {
        let bound = |key: &str| s.get(key).and_then(Value::as_f64);
        if bound("minimum").is_some_and(|m| x < m)
            || bound("maximum").is_some_and(|m| x > m)
            || bound("exclusiveMinimum").is_some_and(|m| x <= m)
            || bound("exclusiveMaximum").is_some_and(|m| x >= m)
        {
            return false;
        }
    }
    if let Some(items) = v.as_array() {
        if s.get("minItems").and_then(Value::as_u64).is_some_and(|n| (items.len() as u64) < n) {
            return false;
        }
        let prefix = s.get("prefixItems").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        if items.iter().zip(prefix).any(|(x, p)| !instance_valid(p, x)) {
            return false;
        }
        if let Some(rest) = s.get("items")
            && items.iter().skip(prefix.len()).any(|x| !instance_valid(rest, x))
        {
            return false;
        }
    }
    if let Some(object) = v.as_object() {
        let required = s.get("required").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        if required.iter().filter_map(Value::as_str).any(|k| !object.contains_key(k)) {
            return false;
        }
        let props = s.get("properties").and_then(Value::as_object);
        for (key, x) in object {
            match props.and_then(|p| p.get(key)) {
                Some(p) if !instance_valid(p, x) => return false,
                Some(_) => {}
                None => {
                    if s.get("additionalProperties").is_some_and(|a| !instance_valid(a, x)) {
                        return false;
                    }
                }
            }
        }
    }
    true
}
