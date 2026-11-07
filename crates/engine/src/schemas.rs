//! The commands' parameter schemas ([`vectorcraft_apischema::ParamSchema`]) and the API level they
//! belong to.
//!
//! A command with a schema (`CommandSpec::schema`) has its params checked against it by
//! [`crate::Session::execute`] before it runs ([`check`]): a value of the wrong type, out of range or
//! not in its list, a missing required parameter or an unknown key is refused with a parameter
//! error naming the parameter, and the command does not run, so the document and its undo history
//! stay as they were. An omitted value with a default takes it. The command's `params` text is the
//! text its schema writes, and `engine.commands` lists the schema of each command that has one.
//!
//! [`API_VERSION`] goes up by one whenever a schema changes, and [`SCHEMA_HASHES`] gains a row with
//! the new level and [`schema_hash`]: a test fails while the newest row does not match the schemas.

use serde_json::Value;
use vectorcraft_apischema::Problem;

use crate::Result;
use crate::cmd::params::unknown;
use crate::cmd::{CommandSpec, bad, command_specs};

/// The API level of the commands' parameters.
pub const API_VERSION: u32 = 1;

/// [`schema_hash`] at each API level, oldest first; the last row is [`API_VERSION`]'s.
pub const SCHEMA_HASHES: &[(u32, u64)] = &[(1, 0x7552_6395_fcbb_1b90)];

/// The hash of every command's schema ([`vectorcraft_apischema::hash`]).
pub fn schema_hash() -> u64 {
    vectorcraft_apischema::hash(command_specs().iter().filter_map(|c| Some((c.id, c.schema?))))
}

/// `params` checked against the schema of `spec`: a parameter error under the command's id for the
/// first problem, or the params with the defaults of omitted values filled in (`None` when the
/// command has no schema or nothing was filled in, so `params` is used as it is). An unknown
/// top-level key is reported as the commands without a schema report it, with the nearest key.
pub(crate) fn check(spec: &CommandSpec, params: &Value) -> Result<Option<Value>> {
    let Some(schema) = spec.schema else { return Ok(None) };
    schema.validate(params).map_err(|e| match &e.problem {
        Problem::Unknown { keys } if !e.path.contains(['.', '[']) => unknown(spec.id, &e.path, keys),
        _ => bad(spec.id, e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::{Map, Value, json};
    use vectorcraft_apischema::{Bound, Omitted, Param, ParamSchema, Type};

    use super::*;
    use crate::{EngineError, Session};

    fn schemas() -> Vec<(&'static str, &'static ParamSchema)> {
        command_specs().iter().filter_map(|c| Some((c.id, c.schema?))).collect()
    }

    /// The params text of a command with a schema is the text the schema writes.
    #[test]
    fn params_text_is_the_text_the_schema_writes() {
        assert!(!schemas().is_empty());
        for c in command_specs() {
            if let Some(schema) = c.schema {
                assert_eq!(c.params, schema.params_text(), "{}", c.id);
            }
        }
    }

    /// A schema changed without a new API level (and a new row of hashes) fails here: add the row
    /// `(API_VERSION + 1, <the hash below>)` and raise [`API_VERSION`].
    #[test]
    fn schema_hash_matches_the_api_version() {
        let last = SCHEMA_HASHES.last().copied();
        assert_eq!(last.map(|(level, _)| level), Some(API_VERSION), "the last row of SCHEMA_HASHES is not API_VERSION's");
        assert_eq!(last.map(|(_, hash)| hash), Some(schema_hash()), "the schemas changed: {:#x}", schema_hash());
        assert!(SCHEMA_HASHES.windows(2).all(|w| w[0].0 < w[1].0), "the API levels go up");
    }

    /// Every default written as JSON parses and passes its parameter's type, and no parameter or
    /// command is newer than the API level.
    #[test]
    fn defaults_parse_and_levels_are_known() {
        fn walk(id: &str, params: &[Param]) {
            for p in params {
                assert!(p.since >= 1 && p.since <= API_VERSION, "{id}.{}", p.name);
                if let Omitted::Value(text) = p.omitted {
                    let v: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{id}.{}: {e}", p.name));
                    let one = ParamSchema::new(Box::leak(Box::new([Param::new(p.name, p.ty)])), "");
                    one.validate(&json!({ p.name: v })).unwrap_or_else(|e| panic!("{id}: {e}"));
                }
                if let Type::Object(inner) | Type::Array(&Type::Object(inner)) = p.ty {
                    walk(id, inner);
                }
            }
        }
        for (id, schema) in schemas() {
            assert!(schema.since >= 1 && schema.since <= API_VERSION, "{id}");
            walk(id, schema.params);
        }
    }

    /// A refusal from the schema names the parameter, and the command does not run.
    #[test]
    fn a_refused_value_names_the_parameter_under_the_command() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let e = s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": -1})).unwrap_err().to_string();
        assert!(e.contains("view.saved.new") && e.contains("`zoom` must be a number > 0, not -1"), "{e}");
        let e = s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": 1, "rotaton": 5})).unwrap_err().to_string();
        assert!(e.contains("unknown parameter `rotaton`; did you mean `rotation`?"), "{e}");
        assert!(s.doc().unwrap().doc.views.is_empty());
        assert!(s.doc().unwrap().history.undo.is_empty());
    }

    /// The engine's own listing carries the schema of each command that has one.
    #[test]
    fn engine_commands_list_the_schemas() {
        let s = Session::new();
        let all = s.commands();
        let grid = all.iter().find(|c| c.id == "view.transparencyGrid").and_then(|c| c.schema.clone()).unwrap();
        assert_eq!(grid["$schema"], vectorcraft_apischema::DIALECT);
        assert_eq!(grid["properties"]["on"]["x-default"], "toggle");
        let listed = serde_json::to_value(all.iter().find(|c| c.id == "view.saved.new").unwrap()).unwrap();
        assert_eq!(listed["schema"]["required"], json!(["center", "zoom"]));
        let without = serde_json::to_value(all.iter().find(|c| c.id == "file.new").unwrap()).unwrap();
        assert!(without.get("schema").is_none());
    }

    /// Values of type `ty` that pass it.
    fn valid(ty: Type) -> BoxedStrategy<Value> {
        fn number(min: Option<Bound>, max: Option<Bound>) -> BoxedStrategy<Value> {
            let low = match min {
                Some(Bound::Inclusive(a)) => a,
                Some(Bound::Exclusive(a)) => a + 1e-6_f64.max(a.abs() * 1e-9),
                None => -1e6,
            };
            let high = match max {
                Some(Bound::Inclusive(b)) => b,
                Some(Bound::Exclusive(b)) => b - 1e-6_f64.max(b.abs() * 1e-9),
                None => low.max(0.0) + 1e6,
            };
            (low..=high).prop_map(|x| json!(x)).boxed()
        }
        match ty {
            Type::Number { min, max, .. } => number(min, max),
            Type::Integer { min, max } => (min.unwrap_or(-1_000_000)..=max.unwrap_or(1_000_000)).prop_map(|x| json!(x)).boxed(),
            Type::Boolean => any::<bool>().prop_map(Value::from).boxed(),
            Type::String => "[ -~]{0,12}".prop_map(Value::from).boxed(),
            Type::Enum(names) => prop::sample::select(names.to_vec()).prop_map(Value::from).boxed(),
            Type::Id => (0u64..10_000).prop_map(Value::from).boxed(),
            Type::Ids => prop::collection::vec(0u64..10_000, 0..4).prop_map(Value::from).boxed(),
            Type::Color => Just(json!("#336699")).boxed(),
            Type::Point => (-1e6..1e6f64, -1e6..1e6f64).prop_map(|(x, y)| json!([x, y])).boxed(),
            Type::Object(params) => valid_object(params),
            Type::Array(item) => prop::collection::vec(valid(*item), 0..3).prop_map(Value::from).boxed(),
        }
    }

    /// Objects that pass `params`: each required parameter, and each optional one or not.
    fn valid_object(params: &'static [Param]) -> BoxedStrategy<Value> {
        let parts: Vec<BoxedStrategy<Option<(String, Value)>>> = params
            .iter()
            .map(|p| {
                let entry = valid(p.ty).prop_map(move |v| Some((p.name.to_string(), v)));
                if p.required { entry.boxed() } else { prop_oneof![Just(None), entry].boxed() }
            })
            .collect();
        parts.prop_map(|entries| Value::Object(entries.into_iter().flatten().collect::<Map<String, Value>>())).boxed()
    }

    /// Commands with a schema and params for them that pass it.
    fn valid_calls() -> BoxedStrategy<(&'static str, Value)> {
        let calls: Vec<BoxedStrategy<(&'static str, Value)>> =
            schemas().into_iter().map(|(id, schema)| valid_object(schema.params).prop_map(move |p| (id, p)).boxed()).collect();
        prop::strategy::Union::new(calls).boxed()
    }

    proptest! {
        /// Params that pass a command's schema never get a parameter error about a value's type or
        /// range from the command itself: the schema refuses whatever the command's own checks would.
        #[test]
        fn schema_valid_params_get_no_type_error_from_the_command((id, params) in valid_calls(), views in 0usize..3) {
            let mut s = Session::new();
            s.execute("file.new", &json!({})).unwrap();
            for i in 0..views {
                s.execute("view.saved.new", &json!({"name": format!("V{i}"), "center": [0, 0], "zoom": 1})).unwrap();
            }
            match s.execute(id, &params) {
                Err(EngineError::BadParams { msg, .. }) => {
                    prop_assert!(!msg.contains(" must be ") && !msg.contains("missing") && !msg.contains("unknown parameter"), "{id} {params}: {msg}");
                }
                Err(EngineError::Internal { msg, .. }) => prop_assert!(false, "{id} {params} panicked: {msg}"),
                _ => {}
            }
        }
    }
}
