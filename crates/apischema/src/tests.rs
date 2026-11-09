//! Tests of the validator, the params text and the JSON form, on schemas written here.

use serde_json::{Value, json};

use crate::*;

/// The schema with the one parameter `x` of type `ty`.
fn one(ty: Type) -> ParamSchema {
    let params: &'static [Param] = Box::leak(Box::new([Param::new("x", ty)]));
    ParamSchema::new(params, "")
}

/// Whether `{x: v}` passes the schema of `ty`; on a refusal, the error must name `x`.
fn passes(ty: Type, v: Value) -> bool {
    match one(ty).validate(&json!({ "x": v })) {
        Ok(_) => true,
        Err(e) => {
            assert!(e.path.starts_with('x'), "{e}");
            assert!(e.to_string().contains("`x"), "{e}");
            false
        }
    }
}

#[test]
fn numbers_are_checked_against_their_range() {
    let any = Type::NUMBER;
    assert!(passes(any, json!(-3.5)) && passes(any, json!(7)));
    assert!(!passes(any, json!("7")) && !passes(any, json!(null)) && !passes(any, json!(true)));
    let positive = Type::Number { min: Some(Bound::Exclusive(0.0)), max: None, unit: None };
    assert!(passes(positive, json!(0.001)));
    assert!(!passes(positive, json!(0)) && !passes(positive, json!(-1)));
    let percent = Type::Number { min: Some(Bound::Inclusive(0.0)), max: Some(Bound::Inclusive(100.0)), unit: Some("%") };
    assert!(passes(percent, json!(0)) && passes(percent, json!(100)));
    assert!(!passes(percent, json!(100.5)));
    let below = Type::Number { min: None, max: Some(Bound::Exclusive(1.0)), unit: None };
    assert!(passes(below, json!(0.99)) && !passes(below, json!(1)));
    assert_eq!(one(positive).validate(&json!({"x": 0})).unwrap_err().to_string(), "`x` must be a number > 0, not 0");
    assert_eq!(one(percent).validate(&json!({"x": 101})).unwrap_err().to_string(), "`x` must be a number 0 to 100 (%), not 101");
}

#[test]
fn integers_take_no_fraction_and_keep_their_range() {
    let any = Type::Integer { min: None, max: None };
    assert!(passes(any, json!(-4)) && passes(any, json!(u64::MAX)));
    assert!(!passes(any, json!(1.5)) && !passes(any, json!("2")));
    let small = Type::Integer { min: Some(1), max: Some(25) };
    assert!(passes(small, json!(1)) && passes(small, json!(25)));
    assert!(!passes(small, json!(0)) && !passes(small, json!(26)) && !passes(small, json!(u64::MAX)));
    assert!(passes(small, json!(25.0)) && !passes(small, json!(26.0)) && !passes(small, json!(1e300)) && !passes(small, json!(-1e300)));
    let negative = Type::Integer { min: None, max: Some(-1_000_000_000_000_000_000) };
    assert!(passes(negative, json!(-1e20)) && !passes(negative, json!(1e20)));
    let lowest = Type::Integer { min: Some(i64::MIN), max: None };
    assert!(!passes(lowest, json!(-1e20)) && passes(lowest, json!(1e20)));
}

/// JSON Schema counts a number with no fractional part as an integer: `2.0` passes, and the
/// command gets `2`.
#[test]
fn a_whole_number_written_with_a_fraction_is_an_integer() {
    let any = Type::Integer { min: None, max: None };
    assert_eq!(one(any).validate(&json!({"x": 2.0})).unwrap(), Some(json!({"x": 2})));
    assert_eq!(one(any).validate(&json!({"x": 2})).unwrap(), None);
    assert!(passes(any, json!(1e300)));
    assert_eq!(one(Type::Id).validate(&json!({"x": 7.0})).unwrap(), Some(json!({"x": 7})));
    assert_eq!(one(Type::Ids).validate(&json!({"x": [1, 2.0]})).unwrap(), Some(json!({"x": [1, 2]})));
    assert!(passes(Type::Id, json!(MAX_ID)) && !passes(Type::Id, json!(MAX_ID + 1)) && !passes(Type::Id, json!(1e300)));
    assert_eq!(one(Type::Ids).validate(&json!({"x": [1, "2"]})).unwrap_err().path, "x[1]");
}

/// A value with no finite number in it reads as null in JSON, and a number refuses it; the largest
/// finite numbers pass a number with no range.
#[test]
fn numbers_at_the_edges() {
    assert!(!passes(Type::NUMBER, json!(f64::NAN)) && !passes(Type::NUMBER, json!(f64::INFINITY)));
    assert!(passes(Type::NUMBER, json!(f64::MAX)) && passes(Type::NUMBER, json!(f64::MIN)));
    let unit = Type::Number { min: Some(Bound::Inclusive(0.0)), max: Some(Bound::Inclusive(1.0)), unit: None };
    assert!(passes(unit, json!(0.0)) && passes(unit, json!(1.0)) && !passes(unit, json!(-1e-300)) && !passes(unit, json!(1.0000001)));
}

/// A list too long to name one path per entry costs one pass, and the error names the entry.
#[test]
fn a_huge_list_is_checked_once_and_names_the_entry() {
    let mut ids: Vec<Value> = (0..200_000u64).map(Value::from).collect();
    assert_eq!(one(Type::Ids).validate(&json!({ "x": ids })).unwrap(), None);
    ids.push(json!("no"));
    let e = one(Type::Ids).validate(&json!({ "x": ids })).unwrap_err();
    assert_eq!(e.path, "x[200000]");
    assert!(e.to_string().chars().count() < 120, "{e}");
}

/// A list nested far past anything the schema can take: dropping or writing it out recurses per
/// level, which is why the test leaks it rather than dropping it.
fn nested_lists(levels: usize) -> Value {
    (0..levels).fold(json!(1), |inner, _| Value::Array(vec![inner]))
}

/// A refused value is shown from its start only, however deep or long it is, so an error costs
/// the same for a hostile value as for a small one.
#[test]
fn a_refused_value_is_read_only_as_far_as_it_is_shown() {
    let deep = nested_lists(500_000);
    let params = json!({});
    let mut map = params.as_object().unwrap().clone();
    map.insert("x".into(), deep);
    let params = Value::Object(map);
    let e = one(Type::String).validate(&params).unwrap_err();
    assert!(e.to_string().starts_with("`x` must be a string, not [[[[") && e.to_string().ends_with('…'), "{e}");
    assert!(e.to_string().chars().count() <= 100, "{e}");
    std::mem::forget(params);

    let huge = "x".repeat(5_000_000);
    let e = one(Type::Boolean).validate(&json!({ "x": huge })).unwrap_err();
    assert!(e.to_string().chars().count() <= 100, "{}", e.to_string().len());
    assert!(e.to_string().ends_with('…'));

    let e = one(Type::Boolean).validate(&json!({"x": {"a": 1, "b": [1, 2]}})).unwrap_err();
    assert_eq!(e.to_string(), "`x` must be true or false, not {\"a\":1,\"b\":[1,2]}");
    let e = ParamSchema::new(&[], "").validate(&nested_lists(3)).unwrap_err();
    assert_eq!(e.to_string(), "the params must be an object, not [[[1]]]");
}

/// A list entry made an integer, and a default filled in, don't copy a value that is still to be
/// checked: copying recurses per level.
#[test]
fn a_list_with_a_change_is_not_copied_before_all_of_it_is_checked() {
    let params = Value::Object([("x".to_string(), Value::Array(vec![json!(2.0), nested_lists(500_000)]))].into_iter().collect());
    assert_eq!(one(Type::Ids).validate(&params).unwrap_err().path, "x[1]");
    std::mem::forget(params);
}

#[test]
fn an_object_with_a_default_is_not_copied_before_all_of_it_is_checked() {
    const FILLED_FIRST: &[Param] = &[Param::new("a", Type::NUMBER).defaults_to("0"), Param::new("b", Type::String)];
    let params = Value::Object([("b".to_string(), nested_lists(500_000))].into_iter().collect());
    assert_eq!(ParamSchema::new(FILLED_FIRST, "").validate(&params).unwrap_err().path, "b");
    std::mem::forget(params);
}

/// A value the schema doesn't look inside is measured before a default copies the object it is in.
const WITH_DEFAULT: &[Param] = &[Param::new("a", Type::NUMBER).defaults_to("0"), Param::new("b", Type::Color).optional()];
const LENIENT_AFTER_DEFAULT: &[Param] = &[Param::new("a", Type::NUMBER).defaults_to("0"), Param::new("b", Type::Boolean).optional().lenient()];

fn holding(key: &str, v: Value) -> Value {
    Value::Object([(key.to_string(), v)].into_iter().collect())
}

/// The lists of fixed shape count toward the nesting limit as the other lists do.
#[test]
fn id_lists_points_and_color_lists_count_toward_the_nesting_limit() {
    let check = |depth, ty: &Type, v: &Value| check_value(&At::Root, depth, ty, v, false, false);
    for (ty, v) in [(Type::Ids, json!([1])), (Type::Point, json!([1, 2])), (Type::Color, json!([1, 0, 0]))] {
        assert!(check(MAX_DEPTH - 1, &ty, &v).is_ok(), "{ty:?}");
        assert_eq!(check(MAX_DEPTH, &ty, &v).unwrap_err().problem, Problem::TooDeep, "{ty:?}");
    }
    assert!(check(MAX_DEPTH, &Type::Color, &json!("#ff0000")).is_ok());
}

#[test]
fn a_saved_retired_key_past_the_nesting_limit_is_refused() {
    let params = holding("older", nested_lists(500_000));
    let e = ParamSchema::new(WITH_DEFAULT, "").validate_saved(&params).unwrap_err();
    assert_eq!(e.path, "older");
    assert!(matches!(e.problem, Problem::Unknown { .. }));
    std::mem::forget(params);
    let kept = ParamSchema::new(WITH_DEFAULT, "").validate_saved(&holding("older", nested_lists(MAX_DEPTH))).unwrap().unwrap();
    assert_eq!(kept["a"], json!(0));
    assert_eq!(kept["older"], nested_lists(MAX_DEPTH));
    assert!(ParamSchema::new(WITH_DEFAULT, "").validate_saved(&holding("older", nested_lists(MAX_DEPTH + 1))).is_err());
}

#[test]
fn a_color_object_past_the_nesting_limit_is_refused() {
    let params = holding("b", holding("k", nested_lists(500_000)));
    let e = ParamSchema::new(WITH_DEFAULT, "").validate(&params).unwrap_err();
    assert_eq!(e.path, "b");
    assert!(matches!(e.problem, Problem::WrongType { .. }));
    std::mem::forget(params);
    assert!(ParamSchema::new(WITH_DEFAULT, "").validate(&json!({"b": {"r": 1, "g": 0, "b": 0}})).is_ok());
}

#[test]
fn a_lenient_value_past_the_nesting_limit_is_refused() {
    let params = holding("b", nested_lists(500_000));
    let e = ParamSchema::new(LENIENT_AFTER_DEFAULT, "").validate(&params).unwrap_err();
    assert_eq!(e.path, "b");
    assert!(matches!(e.problem, Problem::WrongType { .. }));
    std::mem::forget(params);
    assert_eq!(ParamSchema::new(LENIENT_AFTER_DEFAULT, "").validate(&json!({"b": [1]})).unwrap().unwrap()["b"], json!([1]));
}

/// A key is cut in the error, and an unknown key is refused whatever it holds.
#[test]
fn a_hostile_key_is_cut_and_refused() {
    let key = "k".repeat(1_000_000);
    let e = one(Type::Boolean).validate(&json!({ key: true, "x": true })).unwrap_err();
    assert_eq!(e.path.chars().count(), KEY_CHARS + 1);
    assert!(e.path.ends_with('…') && e.to_string().chars().count() < 200, "{}", e.path.len());
    for key in ["", "a.b", "a[0]", "`", "\u{0}", "名前"] {
        let e = one(Type::Boolean).validate(&json!({ key: true, "x": true })).unwrap_err();
        assert_eq!(e.path, key);
        assert!(matches!(e.problem, Problem::Unknown { .. }));
    }
    assert_eq!(shown_key("short"), "short");
}

/// A schema that refers to itself, which only a mistake can write: checking values stops at the
/// nesting limit, and the writers finish.
static ENDLESS: Type = Type::Array(&ENDLESS);

#[test]
fn a_schema_that_refers_to_itself_stops_at_the_nesting_limit() {
    let s = one(ENDLESS);
    let e = s.validate(&json!({ "x": nested_lists(MAX_DEPTH + 2) })).unwrap_err();
    assert_eq!(e.problem, Problem::TooDeep);
    assert!(e.path.starts_with("x[0][0]"));
    assert!(s.definition_problem().is_some_and(|p| p.contains("nests deeper")));
    assert!(s.params_text().contains('…'));
    assert!(s.to_json()["properties"]["x"].is_object());
    assert_ne!(hash([("a", &s)]), 0);
}

const BAD_DEFAULT: &[Param] = &[Param::new("x", Type::NUMBER).defaults_to("\"zero\"")];
const NOT_JSON: &[Param] = &[Param::new("x", Type::NUMBER).defaults_to("{")];
const TWICE: &[Param] = &[Param::new("x", Type::NUMBER), Param::new("x", Type::String)];
const EMPTY_NAME: &[Param] = &[Param::new("", Type::NUMBER)];
const PATH_NAME: &[Param] = &[Param::new("a.b", Type::NUMBER)];
const NO_NAMES: &[Param] = &[Param::new("x", Type::Enum(&[]))];
const SAME_NAMES: &[Param] = &[Param::new("x", Type::Enum(&["a", "a"]))];
const EMPTY_RANGE: &[Param] = &[Param::new("x", Type::Number { min: Some(Bound::Exclusive(1.0)), max: Some(Bound::Inclusive(1.0)), unit: None })];
const BACKWARD: &[Param] = &[Param::new("x", Type::Integer { min: Some(2), max: Some(1) })];
const NOT_FINITE: &[Param] = &[Param::new("x", Type::Number { min: None, max: Some(Bound::Inclusive(f64::INFINITY)), unit: None })];
const REQUIRED_DEFAULT: &[Param] = &[Param { required: true, omitted: Omitted::Value("1"), ..Param::new("x", Type::NUMBER) }];
const DEEP_FAULT: &[Param] = &[Param::new("edits", Type::Array(&Type::Object(TWICE)))];

#[test]
fn a_schema_that_is_wrong_in_itself_is_reported() {
    for (params, why) in [
        (BAD_DEFAULT, "doesn't pass its type"),
        (NOT_JSON, "isn't JSON"),
        (TWICE, "listed twice"),
        (EMPTY_NAME, "empty or holds"),
        (PATH_NAME, "empty or holds"),
        (NO_NAMES, "no names"),
        (SAME_NAMES, "repeats"),
        (EMPTY_RANGE, "no value can be"),
        (BACKWARD, "no value can be"),
        (NOT_FINITE, "isn't finite"),
        (REQUIRED_DEFAULT, "required"),
        (DEEP_FAULT, "`edits[].x` is listed twice"),
    ] {
        let problem = ParamSchema::new(params, "").definition_problem();
        assert!(problem.as_deref().is_some_and(|p| p.contains(why)), "{params:?}: {problem:?}");
    }
    assert_eq!(ParamSchema::new(VIEW, "").definition_problem(), None);
    assert_eq!(ParamSchema::new(&[], "").definition_problem(), None);
}

/// Saved steps keep keys the schema doesn't name, at any depth; the values it names are checked.
#[test]
fn saved_steps_pass_retired_keys_and_check_their_values() {
    let edits = one(Type::Array(&Type::Object(EDIT)));
    let retired = json!({"x": [{"name": "A", "old": 1}], "older": true});
    assert!(edits.validate(&retired).is_err());
    let filled = edits.validate_saved(&retired).unwrap().unwrap();
    assert_eq!(filled["x"][0]["delete"], json!(false), "defaults are filled in");
    assert_eq!(filled["older"], json!(true), "the keys stay");
    assert_eq!(edits.validate_saved(&json!({"x": [{"name": 5}], "older": true})).unwrap_err().path, "x[0].name");
    assert_eq!(edits.validate_saved(&json!({"older": true})).unwrap_err().problem, Problem::Missing);
}

/// Every type's JSON form follows the 2020-12 rules, and an independent validator of those rules
/// agrees with [`ParamSchema::validate`] on each value.
#[test]
fn the_json_form_and_the_validator_agree_with_the_2020_12_rules() {
    const NULLABLE: &[Param] = &[Param::new("a", Type::String).optional().null_omitted(), Param::new("b", Type::Boolean).defaults_to("true")];
    let types = [
        Type::NUMBER,
        Type::Number { min: Some(Bound::Exclusive(0.0)), max: Some(Bound::Inclusive(1.0)), unit: Some("pt") },
        Type::Number { min: Some(Bound::Inclusive(-1.0)), max: Some(Bound::Exclusive(1.0)), unit: None },
        Type::Integer { min: None, max: None },
        Type::Integer { min: Some(1), max: Some(25) },
        Type::Boolean,
        Type::String,
        Type::Enum(&["a", "b"]),
        Type::Id,
        Type::Ids,
        Type::Color,
        Type::Point,
        Type::Object(EDIT),
        Type::Object(NULLABLE),
        Type::Object(&[]),
        Type::Array(&Type::NUMBER),
        Type::Array(&Type::Object(EDIT)),
    ];
    let values = [
        json!(null),
        json!(true),
        json!(0),
        json!(1),
        json!(2.0),
        json!(25.5),
        json!(-1),
        json!(-1.0),
        json!(1e300),
        json!(MAX_ID),
        json!(MAX_ID + 1),
        json!(u64::MAX),
        json!(""),
        json!("a"),
        json!("c"),
        json!([]),
        json!([1, 2]),
        json!([1, 2, 3]),
        json!([1, "2"]),
        json!([[1]]),
        json!({}),
        json!({"name": "A"}),
        json!({"name": "A", "delete": false}),
        json!({"name": "A", "delete": null}),
        json!({"name": "A", "other": 1}),
        json!({"a": null, "b": false}),
        json!({"a": 5}),
        json!([{"name": "A"}]),
        json!([{"name": 1}]),
    ];
    for ty in types {
        let schema = one(ty);
        let written = schema.to_json();
        conformance::check_schema(&written, "#").unwrap_or_else(|e| panic!("{ty:?}: {e}\n{written:#}"));
        assert_eq!(written["$schema"], "https://json-schema.org/draft/2020-12/schema");
        for v in &values {
            let params = json!({ "x": v });
            let local = schema.validate(&params).is_ok();
            assert_eq!(local, conformance::instance_valid(&written, &params), "{ty:?} on {v}: {written}");
        }
    }
    for params in [VIEW, EDIT, NULLABLE] {
        let schema = ParamSchema::new(params, "").deprecated(1);
        conformance::check_schema(&schema.to_json(), "#").unwrap();
    }
}

/// The independent checker refuses what the 2020-12 rules refuse, so its passes above mean
/// something.
#[test]
fn the_2020_12_checker_refuses_invalid_schemas() {
    for bad in [
        json!(5),
        json!({"type": "float"}),
        json!({"enum": []}),
        json!({"enum": [1, 1]}),
        json!({"required": ["a", "a"]}),
        json!({"required": [1]}),
        json!({"properties": []}),
        json!({"properties": {"a": 5}}),
        json!({"items": 1}),
        json!({"prefixItems": []}),
        json!({"anyOf": []}),
        json!({"minimum": "0"}),
        json!({"minItems": -1}),
        json!({"description": 5}),
        json!({"deprecated": "yes"}),
        json!({"type": "number", "default": "zero"}),
        json!({"$schema": "not a uri"}),
        json!({"minimun": 0}),
    ] {
        assert!(conformance::check_schema(&bad, "#").is_err(), "{bad}");
    }
    assert!(conformance::instance_valid(&json!({"type": "integer"}), &json!(2.0)));
    assert!(!conformance::instance_valid(&json!({"type": "integer"}), &json!(2.5)));
    assert!(!conformance::instance_valid(&json!({"prefixItems": [{"type": "number"}], "items": false}), &json!([1, 2])));
}

#[test]
fn booleans_and_strings_take_only_their_own_type() {
    assert!(passes(Type::Boolean, json!(true)) && passes(Type::Boolean, json!(false)));
    assert!(!passes(Type::Boolean, json!("true")) && !passes(Type::Boolean, json!(1)) && !passes(Type::Boolean, json!(null)));
    assert!(passes(Type::String, json!("")) && passes(Type::String, json!("View 1")));
    assert!(!passes(Type::String, json!(5)) && !passes(Type::String, json!(["a"])));
    assert_eq!(one(Type::Boolean).validate(&json!({"x": "yes"})).unwrap_err().to_string(), "`x` must be true or false, not \"yes\"");
}

#[test]
fn an_enum_takes_one_of_its_names() {
    let modes = Type::Enum(&["rgb", "cmyk"]);
    assert!(passes(modes, json!("rgb")) && passes(modes, json!("cmyk")));
    assert!(!passes(modes, json!("RGB")) && !passes(modes, json!("lab")) && !passes(modes, json!(0)));
    assert_eq!(one(modes).validate(&json!({"x": "lab"})).unwrap_err().to_string(), "`x` must be one of \"rgb\", \"cmyk\", not \"lab\"");
}

#[test]
fn ids_are_non_negative_integers() {
    assert!(passes(Type::Id, json!(0)) && passes(Type::Id, json!(42)));
    assert!(!passes(Type::Id, json!(-1)) && !passes(Type::Id, json!(1.5)) && !passes(Type::Id, json!("42")));
    assert!(passes(Type::Ids, json!([])) && passes(Type::Ids, json!([3, 4])));
    assert!(!passes(Type::Ids, json!(3)) && !passes(Type::Ids, json!([3, "4"])) && !passes(Type::Ids, json!([-3])));
}

#[test]
fn colors_and_points_have_their_shapes() {
    assert!(passes(Type::Color, json!("#ff0000")) && passes(Type::Color, json!([1, 0, 0])) && passes(Type::Color, json!({"gray": 50})));
    assert!(!passes(Type::Color, json!(5)) && !passes(Type::Color, json!([1, 0])) && !passes(Type::Color, json!([1, 0, "x"])));
    assert!(passes(Type::Point, json!([0, -2.5])));
    assert!(!passes(Type::Point, json!([1])) && !passes(Type::Point, json!([1, 2, 3])) && !passes(Type::Point, json!([1, "2"])));
    assert!(!passes(Type::Point, json!({"x": 1, "y": 2})));
}

const EDIT: &[Param] = &[Param::new("name", Type::String), Param::new("delete", Type::Boolean).defaults_to("false")];

#[test]
fn objects_and_arrays_name_the_path_of_the_value_inside() {
    let edits = Type::Array(&Type::Object(EDIT));
    assert!(passes(edits, json!([])) && passes(edits, json!([{"name": "A"}, {"name": "B", "delete": true}])));
    let err = |v: Value| one(edits).validate(&json!({ "x": v })).unwrap_err();
    assert_eq!(err(json!([{"name": "A"}, {"name": "B", "delete": "no"}])).path, "x[1].delete");
    assert_eq!(err(json!([{"name": "A"}, {"delete": true}])).to_string(), "missing `x[1].name`");
    let unknown = err(json!([{"name": "A", "nmae": "B"}]));
    assert_eq!((unknown.path.as_str(), unknown.problem), ("x[0].nmae", Problem::Unknown { keys: vec!["name", "delete"] }));
    assert_eq!(err(json!({"name": "A"})).to_string(), "`x` must be a list, not {\"name\":\"A\"}");
    assert_eq!(err(json!(["A"])).to_string(), "`x[0]` must be an object, not \"A\"");
    // Defaults inside are filled in too.
    let filled = one(edits).validate(&json!({"x": [{"name": "A"}]})).unwrap();
    assert_eq!(filled, Some(json!({"x": [{"name": "A", "delete": false}]})));
}

const VIEW: &[Param] = &[
    Param::new("name", Type::String).default_note("\"View N\"").null_omitted(),
    Param::new("center", Type::Point),
    Param::new("rotation", Type::Number { min: None, max: None, unit: Some("deg") }).defaults_to("0"),
];

#[test]
fn keys_requirements_nulls_and_defaults() {
    let s = ParamSchema::new(VIEW, "save a view").returns("{name}");
    // Unknown keys are refused first, naming the key and the keys the command takes.
    let e = s.validate(&json!({"centre": [0, 0]})).unwrap_err();
    assert_eq!((e.path.as_str(), e.problem), ("centre", Problem::Unknown { keys: vec!["name", "center", "rotation"] }));
    assert_eq!(s.validate(&json!({})).unwrap_err(), ParamError { path: "center".into(), problem: Problem::Missing });
    // Null counts as omitted only where the schema says so.
    assert_eq!(s.validate(&json!({"center": [1, 2], "name": null, "rotation": 15})).unwrap(), None);
    assert_eq!(s.validate(&json!({"center": [1, 2], "rotation": null})).unwrap_err().path, "rotation");
    // An omitted value with a default gets it; one with a note is left to the command.
    assert_eq!(s.validate(&json!({"center": [1, 2]})).unwrap(), Some(json!({"center": [1, 2], "rotation": 0})));
    // Params that aren't an object are refused; null params are `{}`.
    assert_eq!(s.validate(&json!(5)).unwrap_err().to_string(), "the params must be an object, not 5");
    assert_eq!(s.validate(&Value::Null).unwrap_err().path, "center");
    let none = ParamSchema::new(&[], "");
    assert_eq!(none.validate(&Value::Null).unwrap(), None);
    assert_eq!(none.validate(&json!({"a": 1})).unwrap_err().to_string(), "unknown parameter `a`; the object takes no keys");
}

const LENIENT: &[Param] = &[Param::new("on", Type::Boolean).optional().lenient(), Param::new("size", Type::NUMBER).optional().lenient()];

#[test]
fn lenient_parameters_pass_values_of_another_type_to_the_command() {
    let s = ParamSchema::new(LENIENT, "");
    assert_eq!(s.validate(&json!({"on": "yes", "size": "12"})).unwrap(), None);
    // Unknown keys are still refused.
    assert!(s.validate(&json!({"of": true})).is_err());
    // The JSON form states the documented type, so a validator of that form refuses what passes here.
    let written = s.to_json();
    assert_eq!(written["properties"]["on"]["x-lenient"], true);
    assert!(!conformance::instance_valid(&written, &json!({"on": "yes"})));
    assert!(conformance::instance_valid(&written, &json!({"on": true})));
}

#[test]
fn the_params_text_lists_each_parameter_with_its_type() {
    let s = ParamSchema::new(VIEW, "save a view").returns("{name}");
    assert_eq!(s.params_text(), "{name?: string (default: \"View N\"), center: [x, y], rotation?: number deg (default: 0)} save a view → {name}");
    const ALL: &[Param] = &[
        Param::new("a", Type::Integer { min: Some(1), max: Some(25) }),
        Param::new("b", Type::Enum(&["x", "y"])).optional(),
        Param::new("c", Type::Ids).optional().doc("instead of the selection"),
        Param::new("d", Type::Color).optional().deprecated(2),
        Param::new("e", Type::Array(&Type::Object(EDIT))).optional(),
        Param::new("f", Type::Number { min: Some(Bound::Exclusive(0.0)), max: Some(Bound::Inclusive(10.0)), unit: Some("pt") }).optional(),
    ];
    assert_eq!(
        ParamSchema::new(ALL, "").params_text(),
        "{a: integer 1 to 25, b?: \"x\"|\"y\", c?: [id…] (instead of the selection), d?: color (deprecated), \
         e?: [{name: string, delete?: bool (default: false)}…], f?: number > 0 and <= 10 pt}"
    );
}

#[test]
fn the_json_form_is_an_object_schema() {
    let s = ParamSchema::new(VIEW, "save a view").returns("{name}").writes(Writes::Document).cost(2);
    let j = s.to_json();
    assert_eq!(j["$schema"], DIALECT);
    assert_eq!(j["type"], "object");
    assert_eq!(j["additionalProperties"], false);
    assert_eq!(j["required"], json!(["center"]));
    assert_eq!(j["properties"]["name"]["anyOf"], json!([{"type": "string"}, {"type": "null"}]));
    assert_eq!(j["properties"]["name"]["x-default"], "\"View N\"");
    assert_eq!(j["properties"]["rotation"], json!({"type": "number", "x-unit": "deg", "default": 0, "x-since": 1}));
    assert_eq!(j["properties"]["center"]["prefixItems"], json!([{"type": "number"}, {"type": "number"}]));
    assert_eq!((j["x-cost"].clone(), j["x-writes"].clone(), j["x-returns"].clone()), (json!(2), json!("document"), json!("{name}")));
    let range = one(Type::Number { min: Some(Bound::Exclusive(0.0)), max: Some(Bound::Inclusive(1.0)), unit: None }).to_json();
    assert_eq!(range["properties"]["x"], json!({"type": "number", "exclusiveMinimum": 0.0, "maximum": 1.0, "x-since": 1}));
}

#[test]
fn the_hash_follows_every_schema_but_not_their_order() {
    let a = ParamSchema::new(VIEW, "save a view");
    let b = ParamSchema::new(LENIENT, "");
    let h = hash([("view.a", &a), ("view.b", &b)]);
    assert_eq!(h, hash([("view.b", &b), ("view.a", &a)]));
    assert_ne!(h, hash([("view.a", &a)]));
    assert_ne!(h, hash([("view.a", &a), ("view.c", &b)]));
    let changed = ParamSchema::new(VIEW, "save a view").since(2);
    assert_ne!(h, hash([("view.a", &changed), ("view.b", &b)]));
}
