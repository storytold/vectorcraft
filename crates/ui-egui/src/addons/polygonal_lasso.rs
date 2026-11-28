use craft::menu::Node;
use serde_json::json;
use vectorcraft_tools::addons::polygonal_lasso;

use crate::menus::cp;

pub fn menu() -> Vec<Node> {
    vec![cp("Polygonal Lasso Tool", "tool.select", json!({"tool": polygonal_lasso::ID}))]
}
