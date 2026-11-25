use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use serde_json::json;

/// The names of the swatches chosen in the Swatches panel are sent only with Selected Swatches Only, and only when
/// there are some; the values go only for the ticked boxes.
#[test]
fn params_sends_names_only_for_selected_swatches_only_with_some_names() {
    let mut fields = DialogFields::from_pairs([
        ("columns", FieldValue::Int(2)),
        ("selectedOnly", FieldValue::Bool(true)),
        ("names", FieldValue::List(vec![FieldValue::Text("Ink".into())])),
    ]);
    assert_eq!(super::params(&fields)["names"], json!(["Ink"]));
    assert_eq!(super::params(&fields)["columns"], json!(2));
    assert!(super::params(&fields).get("values").is_none(), "none ticked: each swatch shows its own values");

    fields.set("selectedOnly", FieldValue::Bool(false));
    assert!(super::params(&fields).get("names").is_none());

    fields.set("selectedOnly", FieldValue::Bool(true));
    fields.set("names", FieldValue::List(Vec::new()));
    assert!(super::params(&fields).get("names").is_none());

    fields.set("hex", FieldValue::Bool(true));
    fields.set("cmyk", FieldValue::Bool(true));
    assert_eq!(super::params(&fields)["values"], json!(["hex", "cmyk"]));
}
