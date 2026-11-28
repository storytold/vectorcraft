use super::{Deserialize, NodeId, Serialize, Vec2};

/// A ruler guide as a value: an entry of the saved `guides` list, or a ruler guide node of a
/// document as [`crate::Document::ruler_guides`] lists it. In a document, ruler guides are
/// [`crate::NodeKind::RulerGuide`] nodes on their layers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    /// true = vertical guide at `pos` (x), false = horizontal at `pos` (y).
    pub vertical: bool,
    pub pos: f64,
    /// An artboard guide: the [`Artboard::id`] it belongs to. It runs across that artboard only
    /// and moves, is copied and is deleted with it. None: a canvas guide, across the whole canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artboard: Option<u32>,
    /// The layer (or sublayer) it is on: it shows, hides and locks with that layer and is listed
    /// in it in the Layers panel, and goes when the layer is deleted. None: on no layer (guides
    /// from before guides had layers, and from files that don't say), always there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<NodeId>,
    /// The id of its node. None in an entry written before ruler guides were nodes: it gets a new
    /// id when the document is read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<NodeId>,
    /// The name it shows in the Layers panel, if it was given one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Locked on its own (its layer's lock is separate).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// Hidden on its own (its layer's visibility is separate).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

impl Guide {
    /// A canvas guide.
    pub fn new(vertical: bool, pos: f64) -> Self {
        Self { vertical, pos, artboard: None, layer: None, id: None, name: None, locked: false, hidden: false }
    }

    /// Moved by `d` (a vertical guide across, a horizontal one down).
    pub fn moved(&self, d: Vec2) -> Self {
        Self { pos: self.pos + if self.vertical { d.x } else { d.y }, ..self.clone() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridPrefs {
    pub spacing: f64,
    pub subdivisions: u32,
}

impl Default for GridPrefs {
    fn default() -> Self {
        Self { spacing: 72.0, subdivisions: 8 }
    }
}

/// What the rulers measure from (View › Rulers): the active artboard's origin, the default of a
/// new document, or one origin for the whole document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RulerMode {
    #[default]
    Artboard,
    Global,
}

impl RulerMode {
    /// The mode of a document saved before the choice existed: global, as its rulers measured.
    pub fn legacy() -> Self {
        Self::Global
    }
}
