use super::*;

/// Stable per-document object id. Never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u64);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Layer colours used for selection highlighting (index into [`LAYER_COLORS`]) or a custom RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerColor {
    Preset(u8),
    Custom([u8; 3]),
}

/// Layer highlight colours, assigned in sequence to new layers. First is Illustrator-like light blue.
pub const LAYER_COLORS: [(&str, [u8; 3]); 27] = [
    ("Light Blue", [0x4f, 0x80, 0xff]),
    ("Red", [0xff, 0x4f, 0x4f]),
    ("Green", [0x4f, 0xff, 0x4f]),
    ("Blue", [0x4f, 0x4f, 0xff]),
    ("Yellow", [0xff, 0xff, 0x4f]),
    ("Magenta", [0xff, 0x4f, 0xff]),
    ("Cyan", [0x4f, 0xff, 0xff]),
    ("Gray", [0x80, 0x80, 0x80]),
    ("Black", [0x00, 0x00, 0x00]),
    ("Orange", [0xff, 0x66, 0x00]),
    ("Dark Green", [0x00, 0x80, 0x00]),
    ("Teal", [0x00, 0x80, 0x80]),
    ("Tan", [0xcc, 0x99, 0x66]),
    ("Brown", [0x99, 0x33, 0x00]),
    ("Violet", [0x99, 0x33, 0xff]),
    ("Gold", [0xff, 0x99, 0x00]),
    ("Dark Blue", [0x00, 0x00, 0x80]),
    ("Pink", [0xff, 0x99, 0xcc]),
    ("Lavender", [0x99, 0x99, 0xff]),
    ("Brick Red", [0x99, 0x00, 0x00]),
    ("Olive Green", [0x66, 0x66, 0x00]),
    ("Peach", [0xff, 0x99, 0x99]),
    ("Burgundy", [0x99, 0x00, 0x33]),
    ("Grass Green", [0x99, 0xcc, 0x00]),
    ("Ochre", [0x99, 0x66, 0x00]),
    ("Purple", [0x66, 0x00, 0x99]),
    ("Light Gray", [0xbb, 0xbb, 0xbb]),
];

impl LayerColor {
    pub fn rgb(self) -> [u8; 3] {
        match self {
            LayerColor::Preset(i) => LAYER_COLORS[i as usize % LAYER_COLORS.len()].1,
            LayerColor::Custom(c) => c,
        }
    }
}
