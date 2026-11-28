use craft::menu::Node;
use serde_json::json;
use vectorcraft_tools::addons::enclosing_lasso;

use crate::menus::cp;

pub fn menu() -> Vec<Node> {
    vec![cp("Enclosing Lasso Tool", "tool.select", json!({"tool": enclosing_lasso::ID}))]
}
