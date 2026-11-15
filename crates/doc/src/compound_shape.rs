//! Compound shapes (Pathfinder panel › Make Compound Shape): a group whose members each have a
//! shape mode. It draws as the one area the modes make, from the backmost member up, painted with
//! the group's own appearance; the members stay editable. Evaluated in `vectorcraft-effects`.

use serde::{Deserialize, Deserializer, Serialize};

use crate::{Node, NodeKind};

/// How a compound shape's member combines with the area of the members below it. The backmost
/// member's mode does not matter: it starts the area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShapeMode {
    /// Adds the member's area.
    #[default]
    Add,
    /// Takes the member's area away.
    Subtract,
    /// Keeps only what the member's area overlaps.
    Intersect,
    /// Keeps where exactly one of the area and the member's area is.
    Exclude,
}

impl ShapeMode {
    /// Every mode, in the Pathfinder panel's order.
    pub const ALL: [ShapeMode; 4] = [ShapeMode::Add, ShapeMode::Subtract, ShapeMode::Intersect, ShapeMode::Exclude];

    /// The mode's name in commands and files.
    pub fn name(self) -> &'static str {
        match self {
            ShapeMode::Add => "add",
            ShapeMode::Subtract => "subtract",
            ShapeMode::Intersect => "intersect",
            ShapeMode::Exclude => "exclude",
        }
    }

    /// The mode named `s` ([`ShapeMode::name`]).
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
}

/// Files are untrusted: a mode that isn't one of the names reads as Add.
impl<'de> Deserialize<'de> for ShapeMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(v.as_str().and_then(ShapeMode::parse).unwrap_or_default())
    }
}

impl Node {
    /// Is this a compound shape? Only a group that doesn't clip can be one: the mark on anything
    /// else (from a file) is ignored.
    pub fn is_compound_shape(&self) -> bool {
        matches!(self.kind, NodeKind::Group { clip: false, .. }) && self.extras().compound_shape
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_read_back_and_unknown_ones_are_add() {
        for m in ShapeMode::ALL {
            let v = serde_json::to_value(m).unwrap();
            assert_eq!(v, serde_json::json!(m.name()));
            assert_eq!(serde_json::from_value::<ShapeMode>(v).unwrap(), m);
        }
        for v in [serde_json::json!("divide"), serde_json::json!(3), serde_json::json!(null), serde_json::json!({"a": [1]})] {
            assert_eq!(serde_json::from_value::<ShapeMode>(v).unwrap(), ShapeMode::Add);
        }
    }
}
