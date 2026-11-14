//! Checks of tool option values ([`crate::Tool::check_option`]) and of `tool.setOption`'s
//! parameters. Each error names the option and what it takes.

use serde_json::{Map, Value};

use crate::Tool;

/// The error for an option the tool doesn't have.
pub fn unknown(key: &str) -> String {
    format!("unknown option `{key}`")
}

/// `v` is a finite number from `lo` to `hi`.
pub fn number(key: &str, v: &Value, lo: f64, hi: f64) -> Result<(), String> {
    match v.as_f64() {
        Some(n) if n.is_finite() && (lo..=hi).contains(&n) => Ok(()),
        _ => Err(format!("`{key}` must be a number from {lo} to {hi}")),
    }
}

/// `v` is a finite number.
pub fn finite(key: &str, v: &Value) -> Result<(), String> {
    match v.as_f64() {
        Some(n) if n.is_finite() => Ok(()),
        _ => Err(format!("`{key}` must be a finite number")),
    }
}

/// `v` is a finite number above 0 and at most `hi`.
pub fn positive(key: &str, v: &Value, hi: f64) -> Result<(), String> {
    match v.as_f64() {
        Some(n) if n > 0.0 && n <= hi => Ok(()),
        _ => Err(format!("`{key}` must be a number above 0 and at most {hi}")),
    }
}

/// `v` is a whole number (written without a fraction) from `lo` to `hi`.
pub fn whole(key: &str, v: &Value, lo: u64, hi: u64) -> Result<(), String> {
    match v.as_u64() {
        Some(n) if (lo..=hi).contains(&n) => Ok(()),
        _ => Err(format!("`{key}` must be a whole number from {lo} to {hi}")),
    }
}

/// `v` is true or false.
pub fn boolean(key: &str, v: &Value) -> Result<(), String> {
    if v.is_boolean() { Ok(()) } else { Err(format!("`{key}` must be true or false")) }
}

/// `v` is one of the strings `choices`.
pub fn one_of(key: &str, v: &Value, choices: &[&str]) -> Result<(), String> {
    if v.as_str().is_some_and(|s| choices.contains(&s)) {
        Ok(())
    } else {
        let list: Vec<String> = choices.iter().map(|c| format!("`{c}`")).collect();
        Err(format!("`{key}` must be one of {}", list.join(", ")))
    }
}

/// `v` is a string, or null.
pub fn string_or_null(key: &str, v: &Value) -> Result<(), String> {
    if v.is_string() || v.is_null() { Ok(()) } else { Err(format!("`{key}` must be a string or null")) }
}

/// What `tool.setOption {tool?, key?, value?, values?}` sets: the tool named (none: the active
/// tool) and the options, `key` and `value` among `values`. A parameter of the wrong type, a
/// `value` without a `key` or a `key` without a `value`, and an option given both as `key` and in
/// `values` are errors. With none of `key` and `values`, the options are empty: the command reads
/// them.
pub fn set_option_params(p: &Value) -> Result<(Option<&str>, Map<String, Value>), String> {
    let tool = match p.get("tool") {
        None => None,
        Some(t) => Some(t.as_str().ok_or("`tool` must be a tool id")?),
    };
    let mut values = match p.get("values") {
        None => Map::new(),
        Some(v) => v.as_object().cloned().ok_or("`values` must be an object")?,
    };
    match (p.get("key"), p.get("value")) {
        (None, None) => {}
        (Some(k), v) => {
            let k = k.as_str().ok_or("`key` must be a string")?;
            let v = v.ok_or_else(|| format!("`value` is missing for `{k}`"))?;
            if values.contains_key(k) {
                return Err(format!("`{k}` is given both as `key` and in `values`"));
            }
            values.insert(k.to_string(), v.clone());
        }
        (None, Some(_)) => return Err("`value` needs a `key`".into()),
    }
    Ok((tool, values))
}

/// Check every one of `values` as an option of `tool` ([`Tool::check_options`]), before any is
/// set: the first one the tool doesn't take is an error that names the tool.
pub fn check_values(tool: &dyn Tool, values: &Map<String, Value>) -> Result<(), String> {
    tool.check_options(values).map_err(|e| format!("tool `{}`: {e}", tool.id()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn numbers_are_checked_against_their_range() {
        assert!(number("fidelity", &json!(0.05), 0.05, 100.0).is_ok());
        assert!(number("fidelity", &json!(100), 0.05, 100.0).is_ok());
        for bad in [json!(-3), json!(1e9), json!("abc"), json!(true), json!(null), json!(0.04)] {
            assert_eq!(number("fidelity", &bad, 0.05, 100.0), Err("`fidelity` must be a number from 0.05 to 100".into()), "{bad}");
        }
        assert!(whole("sides", &json!(3), 3, 1000).is_ok());
        for bad in [json!(2), json!(3.5), json!(1001), json!(-1), json!("6")] {
            assert!(whole("sides", &bad, 3, 1000).is_err(), "{bad}");
        }
        assert!(positive("width", &json!(0), 1e6).is_err());
        assert!(positive("width", &json!(1e6), 1e6).is_ok());
        assert!(finite("angle", &json!(720)).is_ok());
        assert!(finite("angle", &json!("720")).is_err());
    }

    #[test]
    fn other_values_are_checked_by_kind() {
        assert!(boolean("fill", &json!(false)).is_ok());
        assert_eq!(boolean("fill", &json!(0)), Err("`fill` must be true or false".into()));
        assert!(one_of("axis", &json!("free"), &["vertical", "free"]).is_ok());
        assert_eq!(one_of("axis", &json!("diagonal"), &["vertical", "free"]), Err("`axis` must be one of `vertical`, `free`".into()));
        assert!(string_or_null("prev", &json!(null)).is_ok());
        assert!(string_or_null("prev", &json!(3)).is_err());
    }

    #[test]
    fn set_option_params_reads_the_tool_and_the_options() {
        let p = json!({"tool": "smooth", "key": "fidelity", "value": 2, "values": {"x": 1}});
        let (tool, values) = set_option_params(&p).unwrap();
        assert_eq!(tool, Some("smooth"));
        assert_eq!(Value::Object(values), json!({"fidelity": 2, "x": 1}));
        let empty = json!({});
        let (tool, values) = set_option_params(&empty).unwrap();
        assert!(tool.is_none() && values.is_empty());
        assert_eq!(set_option_params(&json!({"tool": 3})).err(), Some("`tool` must be a tool id".into()));
        assert_eq!(set_option_params(&json!({"values": [1]})).err(), Some("`values` must be an object".into()));
        assert_eq!(set_option_params(&json!({"key": 3, "value": 1})).err(), Some("`key` must be a string".into()));
        assert_eq!(set_option_params(&json!({"key": "fill"})).err(), Some("`value` is missing for `fill`".into()));
        assert_eq!(set_option_params(&json!({"value": 1})).err(), Some("`value` needs a `key`".into()));
        assert_eq!(
            set_option_params(&json!({"key": "fill", "value": true, "values": {"fill": false}})).err(),
            Some("`fill` is given both as `key` and in `values`".into())
        );
        // An explicit null is a value: the Type tool's triggers take one.
        assert!(set_option_params(&json!({"key": "stopEditing", "value": null})).is_ok());
    }
}
