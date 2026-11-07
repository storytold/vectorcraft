//! References between the steps of a batch (`vectorcraft-cli run`, MCP `command_batch`): a
//! parameter string that is exactly `$N` or `$N.path` takes that value from the result of step `N`
//! (0-based, in the order the steps ran), and `$last…` from the step before. A path goes into
//! objects by key and into arrays by index: `$1.id`, `$2.ids.0`, `$last.ids[0]`. `$$` at the start
//! of a string stands for one `$` and is never a reference (text such as `$$1.99`). A string that
//! looks like a reference but names no value is an error, so a step never runs with a reference
//! left in it.

use serde_json::Value;

/// Most nesting [`resolve`] walks into: deeper parameters are left as they are.
const MAX_DEPTH: usize = 64;

/// The most the values references copy in may weigh together ([`weight`]: about their JSON size),
/// for one step's parameters. Each reference copies its value, so a short list of references to
/// one large result multiplied it.
const MAX_COPIED: usize = 256 << 20;

/// `params` with every step reference replaced from `results` (the results of the steps run so
/// far, in order); an error names a reference that resolves to nothing, or the reference past
/// which the copied values weigh more than [`MAX_COPIED`].
pub fn resolve(params: &Value, results: &[Value]) -> Result<Value, String> {
    let mut allowance = MAX_COPIED;
    resolve_at(params, results, 0, &mut allowance)
}

fn resolve_at(v: &Value, results: &[Value], depth: usize, allowance: &mut usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Ok(v.clone());
    }
    Ok(match v {
        Value::String(s) => match s.strip_prefix("$$") {
            Some(rest) => Value::String(format!("${rest}")),
            None => match parse(s) {
                Some((step, path)) => {
                    let found = lookup(s, step, &path, results)?;
                    *allowance = weight(found, *allowance)
                        .and_then(|w| allowance.checked_sub(w))
                        .ok_or_else(|| format!("`{s}`: the step's references copy in more than {} MB of values", MAX_COPIED >> 20))?;
                    found.clone()
                }
                None => v.clone(),
            },
        },
        Value::Array(a) => Value::Array(a.iter().map(|x| resolve_at(x, results, depth + 1, allowance)).collect::<Result<_, _>>()?),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, x)| Ok((k.clone(), resolve_at(x, results, depth + 1, allowance)?))).collect::<Result<_, String>>()?)
        }
        _ => v.clone(),
    })
}

/// About the bytes `v` takes as JSON (8 per value, plus its strings and keys); `None` once that
/// passes `limit`, without looking further.
fn weight(v: &Value, limit: usize) -> Option<usize> {
    let mut total = 0usize;
    let mut todo = vec![v];
    while let Some(v) = todo.pop() {
        total = total.checked_add(8)?;
        match v {
            Value::String(s) => total = total.checked_add(s.len())?,
            Value::Array(a) => todo.extend(a),
            Value::Object(o) => {
                for (k, x) in o {
                    total = total.checked_add(k.len())?;
                    todo.push(x);
                }
            }
            _ => {}
        }
        if total > limit {
            return None;
        }
    }
    Some(total)
}

/// Which step a reference names.
#[derive(Debug, PartialEq)]
enum Step {
    Index(usize),
    Last,
}

/// A path segment: an object key or an array index (`.0` is either, as the value decides).
#[derive(Debug, PartialEq)]
enum Seg {
    Key(String),
    Index(usize),
}

/// `$N…` or `$last…` → the step and the path after it; `None` for any other string.
fn parse(s: &str) -> Option<(Step, Vec<Seg>)> {
    let rest = s.strip_prefix('$')?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let (step, mut rest) = if digits > 0 {
        let (n, rest) = rest.split_at_checked(digits)?;
        (Step::Index(n.parse().ok()?), rest)
    } else {
        (Step::Last, rest.strip_prefix("last")?)
    };
    let mut path = vec![];
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let len = r.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
            let (key, r) = r.split_at_checked(len)?;
            if key.is_empty() {
                return None;
            }
            path.push(Seg::Key(key.to_string()));
            rest = r;
        } else {
            let (n, r) = rest.strip_prefix('[')?.split_once(']')?;
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            path.push(Seg::Index(n.parse().ok()?));
            rest = r;
        }
    }
    Some((step, path))
}

fn lookup<'a>(text: &str, step: Step, path: &[Seg], results: &'a [Value]) -> Result<&'a Value, String> {
    let (i, found) = match step {
        Step::Index(i) => (i, results.get(i)),
        Step::Last => (results.len().saturating_sub(1), results.last()),
    };
    let mut cur = found.ok_or_else(|| format!("`{text}`: there is no step {i} before this one (steps count from 0; write $$ for a literal $)"))?;
    for seg in path {
        let next = match (seg, cur) {
            (Seg::Key(k), Value::Object(o)) => o.get(k),
            (Seg::Key(k), Value::Array(a)) => k.parse::<usize>().ok().and_then(|n| a.get(n)),
            (Seg::Index(n), Value::Array(a)) => a.get(*n),
            _ => None,
        };
        cur = next.ok_or_else(|| format!("`{text}`: step {i}'s result has no such value (write $$ for a literal $)"))?;
    }
    Ok(cur)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn references_take_values_from_earlier_results() {
        let results = [json!({"index": 0}), json!({"id": 2}), json!({"ids": [5, 6]})];
        let p = json!({"id": "$1.id", "ids": ["$2.ids.1", "$last.ids[0]"], "nested": {"all": "$2"}, "n": 3, "flag": true});
        assert_eq!(resolve(&p, &results).unwrap(), json!({"id": 2, "ids": [6, 5], "nested": {"all": {"ids": [5, 6]}}, "n": 3, "flag": true}));
    }

    /// References may copy one result many times, but not past [`MAX_COPIED`] together.
    #[test]
    fn copies_of_a_large_result_are_bounded() {
        let big = json!({"text": "x".repeat(MAX_COPIED / 16)});
        let results = [big];
        let few = json!(["$0", "$0", "$0"]);
        assert!(resolve(&few, &results).is_ok());
        let many = Value::Array(vec![json!("$0"); 32]);
        let e = resolve(&many, &results).unwrap_err();
        assert!(e.contains("MB of values"), "{e}");
    }

    #[test]
    fn array_index_must_be_plain_digits() {
        let results = [json!({"ids": [5, 6]})];
        assert_eq!(resolve(&json!("$0.ids[1]"), &results).unwrap(), json!(6));
        for s in ["$0.ids[+1]", "$0.ids[]", "$0.ids[ 1]", "$0.ids[-1]"] {
            let p = json!(s);
            assert!(resolve(&p, &results).map_or(true, |v| v == p), "{s}");
        }
    }

    #[test]
    fn plain_strings_and_escapes_are_left_alone() {
        let results = [json!({"id": 2})];
        let p = json!({"text": "costs $1", "price": "$$1.99", "name": "a$1.id", "dollar": "$", "word": "$lastname"});
        assert_eq!(
            resolve(&p, &results).unwrap(),
            json!({"text": "costs $1", "price": "$1.99", "name": "a$1.id", "dollar": "$", "word": "$lastname"})
        );
    }

    #[test]
    fn a_reference_to_nothing_is_an_error() {
        let results = [json!({"id": 2})];
        for bad in ["$1", "$1.id", "$0.ids", "$0.id.x", "$last.nope", "$7[0]"] {
            let e = resolve(&json!({"id": bad}), &results).unwrap_err();
            assert!(e.contains(bad), "{bad}: {e}");
        }
        assert!(resolve(&json!({"id": "$last"}), &[]).is_err(), "no step before the first");
        // Huge indexes and deep nesting don't panic.
        assert!(resolve(&json!("$99999999999999999999999.id"), &results).is_ok_and(|v| v == json!("$99999999999999999999999.id")));
        let mut deep = json!("$0.id");
        for _ in 0..200 {
            deep = json!([deep]);
        }
        assert!(resolve(&deep, &results).is_ok());
    }
}
