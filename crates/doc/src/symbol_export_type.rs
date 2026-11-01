//! Symbol Options › Export Type: Graphic or Movie Clip
//! ([helpx Symbol Options](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/create-and-place-symbols.html)).
//!
//! For Illustrator-only work the choice has no canvas effect; it is kept for Animate / Flash /
//! HTML5 handoff. Illustrator's default is Movie Clip. Stored under [`SYMBOL_EXPORT_TYPE`] keyed
//! by symbol name; the default (Movie Clip) is omitted from the map.

use serde_json::Value;

/// Document unknown key: `{ symbolName: "graphic"|"movieClip" }`.
pub const SYMBOL_EXPORT_TYPE: &str = "symbolExportType";

/// Symbol Options › Export Type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SymbolExportType {
    /// Static / graphic symbol instance (Animate Graphic).
    Graphic,
    /// Default: Movie Clip for Animate / game handoff; instances still place the same on the canvas.
    #[default]
    MovieClip,
}

impl SymbolExportType {
    pub const IDS: [&'static str; 2] = ["movieClip", "graphic"];

    pub fn id(self) -> &'static str {
        match self {
            Self::Graphic => "graphic",
            Self::MovieClip => "movieClip",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "graphic" => Some(Self::Graphic),
            "movieClip" => Some(Self::MovieClip),
            _ => None,
        }
    }

    /// True when this is the document default (nothing stored for the symbol).
    pub fn is_default(self) -> bool {
        matches!(self, Self::MovieClip)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        v.as_str().and_then(Self::from_id)
    }
}
