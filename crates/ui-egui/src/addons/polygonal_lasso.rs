use serde_json::json;
use vectorcraft_tools::addons::polygonal_lasso;

use crate::menus::Item;

pub fn menu() -> Vec<Item> {
    vec![Item::Cmd("Polygonal Lasso Tool", "tool.select", json!({"tool": polygonal_lasso::ID}))]
}
