//! The commands' parameter schemas ([`vectorcraft_apischema::ParamSchema`]) and the API level they
//! belong to.
//!
//! A command with a schema (`CommandSpec::schema`) has its params checked against it by
//! [`crate::Session::execute`] before it runs ([`check`]), and by the steps that run a command's
//! implementation directly ([`crate::cmd::run_checked`]): a value of the wrong type, out of range
//! or not in its list, a missing required parameter or an unknown key is refused with a parameter
//! error naming the parameter, and the command does not run, so the document and its undo history
//! stay as they were. An omitted value with a default takes it. The steps of a `command.batch` or
//! an `action.batch` are checked before the first one runs. Saved steps
//! ([`crate::Session::replay`]) keep their keys a schema doesn't name, and their values are checked.
//! The command's `params` text is the text its schema writes, and `engine.commands` lists the schema
//! of each command that has one.
//!
//! [`API_VERSION`] goes up by one whenever a schema of a built-in command changes, and
//! [`SCHEMA_HASHES`] gains a row with the new level and [`schema_hash`]: a test fails while the
//! newest row does not match the schemas. Commands that plug-ins add have no schema.

use std::sync::OnceLock;

use serde_json::Value;
use vectorcraft_apischema::{ParamError, Problem};

use crate::cmd::params::unknown;
use crate::cmd::{CommandSpec, bad, command_specs};
use crate::{EngineError, Result};

/// The API level of the commands' parameters.
pub const API_VERSION: u32 = 1;

/// [`schema_hash`] at each API level, oldest first; the last row is [`API_VERSION`]'s.
pub const SCHEMA_HASHES: &[(u32, u64)] = &[(1, 0x9cdb_1087_7074_855c)];

/// The hash of every built-in command's schema ([`vectorcraft_apischema::hash`]).
pub fn schema_hash() -> u64 {
    vectorcraft_apischema::hash(command_specs().iter().filter_map(|c| Some((c.id, c.schema?))))
}

/// Each built-in schema that is wrong in itself, with why ([`vectorcraft_apischema::ParamSchema::definition_problem`]).
/// Worked out once, the first time a command with a schema runs.
fn definition_problems() -> &'static [(&'static str, String)] {
    static PROBLEMS: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();
    PROBLEMS.get_or_init(|| command_specs().iter().filter_map(|c| Some((c.id, c.schema?.definition_problem()?))).collect())
}

/// `params` checked against the schema of `spec`: a parameter error under the command's id for the
/// first problem, or the params with the defaults of omitted values filled in (`None` when the
/// command has no schema or nothing was filled in, so `params` is used as it is). A command with no
/// schema costs one branch. `saved`: the params of a saved step, whose keys the schema doesn't name
/// pass. A schema that is wrong in itself is an internal error, and its command does not run.
pub(crate) fn check(spec: &CommandSpec, params: &Value, saved: bool) -> Result<Option<Value>> {
    let Some(schema) = spec.schema else { return Ok(None) };
    if let Some((_, why)) = definition_problems().iter().find(|(id, _)| *id == spec.id) {
        return Err(EngineError::Internal { cmd: spec.id.into(), msg: why.clone() });
    }
    let checked = if saved { schema.validate_saved(params) } else { schema.validate(params) };
    checked.map_err(|e| param_error(spec.id, &e))
}

/// The parameter error of command `id` for `e`, in the words the message catalogs translate. An
/// unknown top-level key is reported as the commands without a schema report it, with the nearest
/// key.
fn param_error(id: &str, e: &ParamError) -> EngineError {
    let path = &e.path;
    match &e.problem {
        Problem::NotAnObject { got } => bad(id, format!("the params must be an object, not {got}")),
        Problem::Unknown { keys } if !path.contains(['.', '[']) => unknown(id, path, keys),
        Problem::Unknown { keys } if keys.is_empty() => bad(id, format!("unknown parameter `{path}`; the object takes no keys")),
        Problem::Unknown { keys } => bad(id, format!("unknown parameter `{path}`; it takes: {keys}", keys = keys.join(", "))),
        Problem::Missing => bad(id, format!("missing `{path}`")),
        Problem::WrongType { expected, got } => bad(id, format!("`{path}` must be {expected}, not {got}")),
        Problem::TooDeep => EngineError::Internal { cmd: id.into(), msg: e.to_string() },
    }
}

#[cfg(test)]
#[path = "../../apischema/src/conformance.rs"]
mod conformance;

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::{Map, Value, json};
    use vectorcraft_apischema::{Bound, Omitted, Param, ParamSchema, Type};

    use super::conformance::{check_schema, instance_valid};
    use super::*;
    use crate::Session;

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

    /// Every schema is sound in itself ([`ParamSchema::definition_problem`]), so no command is
    /// refused for its schema, and no parameter or command is newer than the API level.
    #[test]
    fn schemas_are_sound_and_levels_are_known() {
        fn walk(id: &str, params: &[Param]) {
            for p in params {
                assert!(p.since >= 1 && p.since <= API_VERSION, "{id}.{}", p.name);
                if let Type::Object(inner) | Type::Array(&Type::Object(inner)) = p.ty {
                    walk(id, inner);
                }
            }
        }
        assert_eq!(definition_problems(), &[]);
        for (id, schema) in schemas() {
            assert!(schema.since >= 1 && schema.since <= API_VERSION, "{id}");
            walk(id, schema.params);
        }
    }

    /// Each command's defaults are of its parameter's type (an omitted value takes them), with
    /// which ones are filled in.
    #[test]
    fn declared_defaults_are_filled_in() {
        let new_view = command_specs().iter().find(|c| c.id == "view.saved.new").and_then(|c| c.schema).unwrap();
        let filled = new_view.validate(&json!({"center": [1, 2], "zoom": 1})).unwrap().unwrap();
        assert_eq!(filled["rotation"], json!(0));
        assert!(filled.get("name").is_none(), "the command works out the name");
        assert!(new_view.param("rotation").is_some_and(|p| matches!(p.omitted, Omitted::Value("0"))));
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
        let e = s.execute("view.saved.new", &Value::Null).unwrap_err().to_string();
        assert!(e.contains("view.saved.new") && e.contains("missing `center`"), "{e}");
        assert!(s.doc().unwrap().doc.views.is_empty());
        assert!(s.doc().unwrap().history.undo.is_empty());
    }

    /// The key check runs before the schema, so a misspelled key is the error even when a value is
    /// bad too, and the schema reports the value when the keys are right.
    #[test]
    fn a_misspelled_key_is_reported_before_a_bad_value() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let e = s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": -1, "rotaton": 5})).unwrap_err().to_string();
        assert!(e.contains("did you mean `rotation`?"), "{e}");
        let e = s.execute("view.saved.new", &json!({"center": "here", "zoom": -1})).unwrap_err().to_string();
        assert!(e.contains("`center` must be [x, y] (two numbers), not \"here\""), "{e}");
    }

    /// Saved steps keep a retired key, at the top level and inside `edits`, and a bad value is
    /// still refused, with no undo step; the journal keeps the params as given.
    #[test]
    fn saved_steps_keep_retired_keys_and_check_values() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let retired = json!({"name": "A", "center": [1, 2], "zoom": 2, "fitArt": true});
        assert!(s.execute("view.saved.new", &retired).is_err());
        s.replay("view.saved.new", &retired).unwrap();
        assert_eq!(s.doc().unwrap().doc.views[0].name, "A");
        let last = s.journal.last().unwrap();
        assert_eq!(last.1, retired, "the journal has the params as given, with no default filled in");
        s.replay("view.saved.edit", &json!({"edits": [{"name": "A", "newName": "B", "was": 1}]})).unwrap();
        assert_eq!(s.doc().unwrap().doc.views[0].name, "B");
        assert!(s.execute("view.saved.edit", &json!({"edits": [{"name": "B", "newName": "C", "was": 1}]})).is_err());
        let steps = s.doc().unwrap().history.undo.len();
        let e = s.replay("view.saved.new", &json!({"center": [1, 2], "zoom": 0, "fitArt": true})).unwrap_err().to_string();
        assert!(e.contains("`zoom`"), "{e}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps);
        // A frontend that replays through its own path sets the flag, and gets the same rule.
        let outer = s.set_replaying(true);
        s.execute("view.saved.new", &json!({"name": "C", "center": [1, 2], "zoom": 2, "fitArt": true})).unwrap();
        s.set_replaying(outer);
    }

    /// A batch step a schema refuses stops the batch before its first step runs.
    #[test]
    fn a_batch_is_checked_against_the_schemas_before_its_first_step() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let step = |p: Value| json!({"command": "view.saved.new", "params": p});
        for (last, named) in [
            (step(json!({"center": [0, 0], "zoom": 0})), "`zoom`"),
            (step(json!({"center": [0, 0]})), "`zoom`"),
            (json!({"command": "view.saved.new"}), "`center`"),
        ] {
            let batch = json!({"commands": [step(json!({"name": "First", "center": [0, 0], "zoom": 1})), last]});
            let e = s.execute("command.batch", &batch).unwrap_err().to_string();
            assert!(e.contains("view.saved.new") && e.contains(named), "{e}");
            assert!(s.doc().unwrap().doc.views.is_empty(), "the first step did not run: {e}");
            assert!(s.doc().unwrap().history.undo.is_empty());
        }
    }

    /// Values and keys far larger than any a command takes are refused with a short error and no
    /// panic. Nested values are leaked because dropping one recurses per level.
    #[test]
    fn hostile_values_are_refused_with_a_short_error() {
        fn deep(levels: usize) -> Value {
            (0..levels).fold(json!(1), |inner, _| Value::Array(vec![inner]))
        }
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let huge = "z".repeat(2_000_000);
        for params in [
            json!({ huge.as_str(): 1, "center": [0, 0], "zoom": 1 }),
            json!({"center": [0, 0], "zoom": huge}),
            json!({"center": huge, "zoom": 1}),
            json!({"center": [0, 0], "zoom": 1, "name": [huge]}),
            json!({"center": [0, 0], "zoom": f64::MAX, "rotation": f64::INFINITY}),
        ] {
            let e = s.execute("view.saved.new", &params).unwrap_err().to_string();
            assert!(e.chars().count() < 400, "{} chars", e.chars().count());
        }
        let e = s.execute("view.saved.edit", &json!({"edits": [{"name": "A", huge.as_str(): 1}]})).unwrap_err().to_string();
        assert!(e.chars().count() < 400, "{} chars", e.chars().count());
        let nested = json!({"center": [0, 0]});
        let Value::Object(mut map) = nested else { panic!() };
        map.insert("zoom".into(), deep(500_000));
        let params = Value::Object(map);
        let e = s.execute("view.saved.new", &params).unwrap_err().to_string();
        assert!(e.contains("`zoom`") && e.chars().count() < 400, "{e}");
        std::mem::forget(params);
        assert!(s.doc().unwrap().doc.views.is_empty());
        assert!(s.doc().unwrap().history.undo.is_empty());
    }

    /// Each command's JSON schema follows the 2020-12 rules, and a validator of those rules, which
    /// shares no code with the writer, gives the same answer as the schema check. None of these
    /// schemas has a lenient parameter.
    #[test]
    fn the_listed_schemas_follow_2020_12_and_agree_with_the_check() {
        let cases: &[(&str, Value)] = &[
            ("view.capsLock", json!({})),
            ("view.capsLock", json!({"on": true})),
            ("view.capsLock", json!({"on": "yes"})),
            ("view.capsLock", json!({"on": null})),
            ("view.capsLock", json!({"of": true})),
            ("view.saved.new", json!({"center": [0, 0], "zoom": 1})),
            ("view.saved.new", json!({"center": [0.5, -2], "zoom": 0.001, "rotation": -30, "name": "A"})),
            ("view.saved.new", json!({"center": [0, 0], "zoom": 1, "name": null})),
            ("view.saved.new", json!({"center": [0, 0], "zoom": 1, "rotation": null})),
            ("view.saved.new", json!({"center": [0, 0], "zoom": 0})),
            ("view.saved.new", json!({"center": [0, 0], "zoom": -1})),
            ("view.saved.new", json!({"center": [0], "zoom": 1})),
            ("view.saved.new", json!({"center": [0, 0, 0], "zoom": 1})),
            ("view.saved.new", json!({"center": [0, "0"], "zoom": 1})),
            ("view.saved.new", json!({"zoom": 1})),
            ("view.saved.new", json!({"center": [0, 0]})),
            ("view.saved.new", json!({"center": [0, 0], "zoom": 1, "name": 5})),
            ("view.saved.edit", json!({})),
            ("view.saved.edit", json!({"name": "A", "newName": "B"})),
            ("view.saved.edit", json!({"name": null, "delete": null})),
            ("view.saved.edit", json!({"edits": []})),
            ("view.saved.edit", json!({"edits": null})),
            ("view.saved.edit", json!({"edits": [{"name": "A", "delete": true}, {"name": "B", "newName": null}]})),
            ("view.saved.edit", json!({"edits": [{"newName": "B"}]})),
            ("view.saved.edit", json!({"edits": [{"name": "A", "delete": 1}]})),
            ("view.saved.edit", json!({"edits": [{"name": "A", "x": 1}]})),
            ("view.saved.edit", json!({"edits": {"name": "A"}})),
            ("view.saved.list", json!({})),
            ("view.saved.list", json!({"all": true})),
            ("view.transparencyGrid", json!({})),
            ("view.transparencyGrid", json!({"on": null})),
            ("view.transparencyGrid", json!({"on": false})),
            ("view.transparencyGrid", json!({"on": 1})),
            ("view.snapToGlyph.options", json!({"bounds": false, "angular": true})),
            ("view.snapToGlyph.options", json!({"bounds": null})),
            ("view.capsLock", json!(null)),
            ("view.saved.new", json!(null)),
            ("view.saved.edit", json!(null)),
            ("view.saved.list", json!(null)),
            ("view.transparencyGrid", json!(null)),
            ("view.snapToGlyph.options", json!(null)),
            ("view.capsLock", json!([])),
            ("view.saved.new", json!(5)),
            ("view.transparencyGrid", json!("on")),
        ];
        assert_eq!(schemas().len(), 6);
        for (id, schema) in schemas() {
            let written = schema.to_json();
            check_schema(&written, id).unwrap_or_else(|e| panic!("{e}\n{written:#}"));
            assert!(cases.iter().any(|(c, _)| *c == id), "{id} has no cases");
        }
        for (id, params) in cases {
            let schema = schemas().into_iter().find(|(c, _)| c == id).unwrap().1;
            assert_eq!(schema.validate(params).is_ok(), instance_valid(&schema.to_json(), params), "{id} {params}");
        }
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

    /// The listing says what each View command changes when it runs. Only the commands that make an
    /// undo step list `document`.
    #[test]
    fn each_view_command_lists_what_it_writes() {
        let listed: Vec<(&str, Value)> = schemas().into_iter().map(|(id, schema)| (id, schema.to_json()["x-writes"].clone())).collect();
        let expected = [
            ("view.capsLock", "session"),
            ("view.saved.new", "document"),
            ("view.saved.edit", "document"),
            ("view.saved.list", "nothing"),
            ("view.transparencyGrid", "session"),
            ("view.snapToGlyph.options", "preferences"),
        ];
        for (id, writes) in expected {
            let got = listed.iter().find(|(c, _)| *c == id).map(|(_, w)| w.clone());
            assert_eq!(got, Some(json!(writes)), "{id}");
        }
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
