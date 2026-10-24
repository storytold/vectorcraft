//! Dimension arrow style JSON (legacy four choices and full stroke arrowheads).
#![allow(clippy::unwrap_used)]

use crate::appearance::Arrowhead;
use crate::dimension::{DimensionOptions, parse_arrow_style};
use serde_json::json;

#[test]
fn legacy_arrow_style_strings_deserialize() {
    assert_eq!(parse_arrow_style(&json!("filled")).unwrap(), Some(Arrowhead::Triangle));
    assert_eq!(parse_arrow_style(&json!("open")).unwrap(), Some(Arrowhead::TriangleOpen));
    assert_eq!(parse_arrow_style(&json!("tick")).unwrap(), Some(Arrowhead::Slash));
    assert_eq!(parse_arrow_style(&json!("none")).unwrap(), None);
}

#[test]
fn stroke_arrowheads_round_trip_in_options() {
    let o: DimensionOptions = serde_json::from_value(json!({"arrowStyle": "Barbed"})).unwrap();
    assert_eq!(o.arrow_style, Some(Arrowhead::Barbed));
    let back = serde_json::to_value(&o).unwrap();
    assert_eq!(back["arrowStyle"], "Barbed");
}
