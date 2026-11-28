//! Save Swatch Library: a name, a file format and where it goes: the user library folder (listed
//! under Window → Swatch Libraries → User Defined) or a file picked in a save dialog (a download
//! on the web). OK runs `swatch.library.save`.
//!
//! Fields: those of [`super::save_library`] and `format` (`vcswatches`, `gpl`, `ase` or `css`).

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind};
use serde_json::Value;
use vectorcraft_color::palette_io::PaletteFormat;

use super::save_library::{self, Library};
use crate::VectorcraftApp;

/// The dialog kind of Save Swatch Library.
pub const KIND: &str = "saveSwatchLibrary";

/// Open Save Swatch Library for the document's swatches (`names`: the ones selected in the panel).
pub fn open(app: &mut VectorcraftApp, names: Vec<String>) -> Result<Value, String> {
    let formats = PaletteFormat::ALL.map(|f| (f.id().to_string(), f.label().into())).to_vec();
    let rows = vec![
        Param::new("format", tl!("Format:"), ParamKind::choice(formats, Some(PaletteFormat::Native.id().into()))),
        Param::read_only_text("gplNote", "", tl!("GPL palettes keep solid colours only, as RGB.")).full_width().visible_when("format", "gpl"),
        Param::read_only_text("aseNote", "", tl!("Swatch exchange files keep solid colors only.")).full_width().visible_when("format", "ase"),
    ];
    let library = Library {
        key: KIND,
        title: tl!("Save Swatch Library").to_string(),
        cmd: "swatch.library.save",
        only: tl!("Selected Swatches Only").to_string(),
        ext: |fields| format_of(fields).id().to_string(),
        has_user_dir: app.session.swatch_libraries.user_dir().is_some(),
    };
    save_library::open(app, library, names, rows)
}

fn format_of(fields: &DialogFields) -> PaletteFormat {
    PaletteFormat::parse(fields.text("format").unwrap_or_default()).unwrap_or(PaletteFormat::Native)
}
