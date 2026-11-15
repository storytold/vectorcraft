//! Compound Shapes in the interface: its submenu (Object › Plug-ins › Compound Shapes) puts Make,
//! Release and Expand Compound Shape (also in the Pathfinder panel's menu) in the menus, and holds the
//! setting that makes the Pathfinder panel's Shape Mode buttons make compound shapes on a click. It
//! acts through `object.compoundShape.*` and `window.pathfinder.compoundShapes`; nothing else
//! about the Pathfinder panel changes.

use serde_json::{Value, json};

use crate::menus::Item;

pub fn menu() -> Vec<Item> {
    vec![
        Item::Cmd("Make Compound Shape", "object.compoundShape.make", json!({})),
        Item::Cmd("Release Compound Shape", "object.compoundShape.release", json!({})),
        Item::Cmd("Expand Compound Shape", "object.compoundShape.expand", json!({})),
        Item::Sep,
        Item::Cmd("Shape Mode Buttons Make Compound Shapes", "window.pathfinder.compoundShapes", Value::Null),
    ]
}
