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
    assert!(!passes(any, json!(1.5)) && !passes(any, json!(2.0)) && !passes(any, json!("2")));
    let small = Type::Integer { min: Some(1), max: Some(25) };
    assert!(passes(small, json!(1)) && passes(small, json!(25)));
    assert!(!passes(small, json!(0)) && !passes(small, json!(26)) && !passes(small, json!(u64::MAX)));
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
