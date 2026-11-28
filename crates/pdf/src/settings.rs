//! Save PDF settings: the model, presets, checks and warnings are craft-pdf's (what a preset stores
//! and `document.exportPdf` takes, in camelCase JSON). This module re-exports them under this
//! crate's names and keeps the `choice!` macro that the Print settings and the import options use.

use crate::PdfError;

// Listed by name: a glob would bring `ColorMode` and `ProfileKind`, which `output.rs` and
// `vectorcraft_color::cms` already name for their own types.
pub use craft::pdf::settings::{
    AdvancedSettings, BleedSettings, Changes, Choice, ColorConversion, Compatibility, CompressionSettings, DOCUMENT_CMYK, Downsample, Encryption,
    ImageCodec, ImageCompression, JpegQuality, MarkKind, MarkSettings, MonoCodec, MonoCompression, OutputSettings, Overprint, PdfPreset, PdfSettings,
    Printing, ProfileCatalog, ProfileInclusion, SecuritySettings, SettingsError, SettingsWarning, Standard, UNSET_CHOICE, builtin_preset,
    builtin_presets, default_preset_name,
};

/// A closed list of choices: a serde enum (its JSON ids) with a UI label per variant.
macro_rules! choice {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $id:literal, $label:literal;)+ } default $d:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $($(#[$vm])* #[serde(rename = $id)] $v,)+
        }

        impl $crate::Choice for $name {
            const ALL: &'static [Self] = &[$(Self::$v),+];
            const IDS: &'static [&'static str] = &[$($id),+];
            const LABELS: &'static [&'static str] = &[$($label),+];
        }

        impl $name {
            /// The label shown in the PDF dialogs.
            pub fn label(self) -> &'static str {
                match self {
                    $(Self::$v => $label,)+
                }
            }

            /// The JSON id.
            pub fn id(self) -> &'static str {
                match self {
                    $(Self::$v => $id,)+
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::$d
            }
        }
    };
}
pub(crate) use choice;

/// `v` lies in `lo..=hi` (and is a number). The error names the range with its unit as one value
/// (`72–2400 ppi`), so a catalog can match the message and translate it.
pub(crate) fn within(name: &str, v: f64, lo: f64, hi: f64, unit: &str) -> Result<(), PdfError> {
    if (lo..=hi).contains(&v) {
        return Ok(());
    }
    let range = format!("{lo}–{hi}{unit}");
    Err(PdfError::BadSetting(format!("{name} must be {range} (got {v})")))
}
