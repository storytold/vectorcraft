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
/// `values` are errors, and so are parameters that are not an object. With none of `key` and
/// `values`, or with no parameters (null), the options are empty: the command reads them.
pub fn set_option_params(p: &Value) -> Result<(Option<&str>, Map<String, Value>), String> {
    if !p.is_object() && !p.is_null() {
        return Err(format!("the params must be an object, not {}", shown(p)));
    }
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

/// The JSON text of `v` as a parameter error shows it: cut to 40 characters, with `…` when it was
/// longer. The text is written only up to the cut.
fn shown(v: &Value) -> String {
    struct Cut {
        text: String,
        left: usize,
    }
    impl std::fmt::Write for Cut {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for c in s.chars() {
                if self.left == 0 {
                    return Err(std::fmt::Error);
                }
                self.left -= 1;
                self.text.push(c);
            }
            Ok(())
        }
    }
    let mut out = Cut { text: String::new(), left: 40 };
    if std::fmt::Write::write_fmt(&mut out, format_args!("{v}")).is_err() {
        out.text.push('…');
    }
    out.text
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

    /// Every tool takes back the options it reports, each on its own and all together: a dialog's
    /// OK sends every field the tool showed it. The state a tool reports and doesn't take (the
    /// picked mesh points, the text being edited, the pins, the queue of files to place) is left
    /// out.
    #[test]
    fn every_tool_takes_the_options_it_reports() {
        const TYPE: &[&str] = &["type", "areaType", "typeOnPath", "verticalType", "verticalAreaType", "verticalTypeOnPath"];
        const READ_ONLY: &[(&[&str], &[&str])] = &[
            (&["directSelection", "groupSelection", "lasso", "polygonalLasso", "mesh", "magicWand"], &["meshPoints"]),
            (TYPE, &["editing"]),
            (&["puppetWarp"], &["pins", "selected"]),
            // With nothing queued, `current` (0) is no file's index either.
            (&["place"], &["count", "current", "sourceIndex", "name", "width", "height", "names"]),
        ];
        let read_only = |id: &str, k: &str| READ_ONLY.iter().any(|(ids, keys)| ids.contains(&id) && keys.contains(&k));
        // Tools that `create` makes and the catalog doesn't list.
        let extra = ["place", "dimension", crate::cropimage::ID, "intertwine"];
        let mut refused = Vec::new();
        let mut check = |id: &str, tool: &dyn Tool, after: &str| {
            let Some(mut options) = tool.options().as_object().cloned() else { return };
            options.retain(|k, _| !read_only(id, k));
            for (k, v) in &options {
                if let Err(e) = tool.check_option(k, v) {
                    refused.push(format!("tool `{id}`{after}, option `{k}` = {v}: {e}"));
                }
            }
            if let Err(e) = check_values(tool, &options) {
                refused.push(format!("{e}{after}"));
            }
        };
        for id in crate::catalog::all_tools().map(|t| t.id).chain(extra) {
            let tool = crate::create(id);
            check(id, tool.as_ref(), "");
            // A number set past either end, or a true-or-false option flipped, is still taken back
            // as the tool then reports it.
            let Some(defaults) = tool.options().as_object().cloned() else { continue };
            for (k, v) in defaults.iter().filter(|(k, _)| !read_only(id, k)) {
                let tries = match v {
                    Value::Number(_) => vec![json!(-1e9), json!(1e9)],
                    Value::Bool(b) => vec![json!(!b)],
                    _ => continue,
                };
                for value in tries {
                    let mut changed = crate::create(id);
                    changed.set_option(k, &value);
                    check(id, changed.as_ref(), &format!(" after setting `{k}` to {value}"));
                }
            }
        }
        assert!(refused.is_empty(), "{}", refused.join("\n"));
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
        // Omitted parameters arrive as null: a read, like `{}`.
        let (tool, values) = set_option_params(&Value::Null).unwrap();
        assert!(tool.is_none() && values.is_empty());
        for (bad, got) in [(json!(true), "true"), (json!(7), "7"), (json!([]), "[]"), (json!("x"), "\"x\"")] {
            assert_eq!(set_option_params(&bad).err(), Some(format!("the params must be an object, not {got}")), "{bad}");
        }
        // A long value is shown cut, as the other parameter errors show it.
        let long = json!("a".repeat(100));
        let shown = format!("\"{}…", "a".repeat(39));
        assert_eq!(set_option_params(&long).err(), Some(format!("the params must be an object, not {shown}")));
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
