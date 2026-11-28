//! Compound Shapes in the interface: its submenu (Object › Plug-ins › Compound Shapes) puts Make,
//! Release and Expand Compound Shape (also in the Pathfinder panel's menu) in the menus, and holds the
//! setting that makes the Pathfinder panel's Shape Mode buttons make compound shapes on a click. It
//! acts through `object.compoundShape.*` and `window.pathfinder.compoundShapes`; nothing else
//! about the Pathfinder panel changes.

use craft::menu::Node;
use serde_json::json;

use crate::menus::{Sep, c, cp};

pub fn menu() -> Vec<Node> {
    vec![
        cp("Make Compound Shape", "object.compoundShape.make", json!({})),
        cp("Release Compound Shape", "object.compoundShape.release", json!({})),
        cp("Expand Compound Shape", "object.compoundShape.expand", json!({})),
        Sep,
        c("Shape Mode Buttons Make Compound Shapes", "window.pathfinder.compoundShapes"),
    ]
}
