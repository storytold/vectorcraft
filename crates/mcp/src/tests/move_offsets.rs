use serde_json::{Value, json};

use super::{Server, call, id_of, server, text_of};

fn inspect(s: &mut Server, id: u64) -> Value {
    serde_json::from_str(&text_of(&call(s, id, "doc_inspect", json!({})))).unwrap()
}

fn path(s: &mut Server, id: u64, d: &str) -> u64 {
    id_of(&call(s, id, "draw_path", json!({"d": d})))
}

fn child(doc: &Value, id: u64) -> Value {
    doc["layers"][0]["children"].as_array().unwrap().iter().find(|node| node["id"] == id).unwrap().clone()
}

#[test]
fn transform_rejects_malformed_move_offsets_before_selection_or_copy() {
    let mut s = server();
    let first = path(&mut s, 1, "M40 40 L60 40 L60 60 L40 60 Z");
    let second = path(&mut s, 2, "M100 100 L130 100 L130 140 L100 140 Z");
    let before = inspect(&mut s, 3);
    assert_eq!(before["selection"], json!([second]));

    let cases = [("dx", json!({"ids": [first], "dx": "bad", "copy": true})), ("dy", json!({"ids": [first], "dx": 10, "dy": "bad", "copy": true}))];
    for (index, (field, args)) in cases.into_iter().enumerate() {
        let request_id = 4 + index as u64 * 2;
        let result = call(&mut s, request_id, "transform", args);
        assert_eq!(result["isError"], true, "{field}: {result}");
        assert!(text_of(&result).contains(field), "{field}: {result}");
        assert_eq!(inspect(&mut s, request_id + 1), before, "{field}");
    }

    // The recorded request omits dx and must still move a copy down by 10 points.
    let result = call(&mut s, 8, "transform", json!({"ids": [first], "dy": 10, "copy": true}));
    assert_eq!(result["isError"], false, "{result}");
    let reply: Value = serde_json::from_str(&text_of(&result)).unwrap();
    assert_eq!(reply["applied"], json!(["move"]));
    let after = inspect(&mut s, 9);
    let copied = after["selection"][0].as_u64().unwrap();
    assert_ne!(copied, first);
    assert_ne!(copied, second);
    let original_before = child(&before, first);
    let original_after = child(&after, first);
    let second_after = child(&after, second);
    let copy_after = child(&after, copied);
    assert_eq!(original_after["bounds"], original_before["bounds"]);
    assert_eq!(second_after["bounds"], child(&before, second)["bounds"]);
    assert_eq!(copy_after["bounds"]["x"], original_before["bounds"]["x"]);
    assert_eq!(copy_after["bounds"]["y"], original_before["bounds"]["y"].as_f64().unwrap() + 10.0);
    assert_eq!(copy_after["bounds"]["width"], original_before["bounds"]["width"]);
    assert_eq!(copy_after["bounds"]["height"], original_before["bounds"]["height"]);
    let before_history = before["history"].as_array().unwrap();
    let after_history = after["history"].as_array().unwrap();
    assert_eq!(after_history.len(), before_history.len() + 1);
    assert_eq!(after_history.last(), Some(&json!("Move")));

    // Null keeps the engine's optional-number default while a negative numeric offset is applied.
    let result = call(&mut s, 10, "transform", json!({"ids": [second], "dx": null, "dy": -7}));
    assert_eq!(result["isError"], false, "{result}");
    let after_null = inspect(&mut s, 11);
    let moved = child(&after_null, second);
    let second_before = child(&before, second);
    assert_eq!(moved["bounds"]["x"], second_before["bounds"]["x"]);
    assert_eq!(moved["bounds"]["y"], second_before["bounds"]["y"].as_f64().unwrap() - 7.0);
    assert_eq!(after_null["history"].as_array().unwrap().last(), Some(&json!("Move")));
}
