//! PSD Options' rule for transparency, and its starting colour model.

use craft::ui::patterns::form_dialogs::DialogFields;
use serde_json::{Map, json};

use super::*;

/// A flat PSD is flattened on white, so its background list starts at White; layers keep
/// transparency. The row `layers` is a two-id radio, so it holds the id text.
#[test]
fn a_flat_psd_is_flattened_and_layers_keep_transparency() {
    let layers = DialogFields::from_pairs([("layers", "true")]);
    let flat = DialogFields::from_pairs([("layers", "false")]);
    assert!(tiff_bmp_tga::keeps_alpha("psd", &layers));
    assert!(!tiff_bmp_tga::keeps_alpha("psd", &flat));
}

/// A CMYK document exports CMYK by default.
#[test]
fn psd_starts_in_cmyk_for_a_cmyk_document() {
    let mut o = Map::new();
    psd_options::defaults(true, &mut o);
    assert_eq!(o["colorModel"], json!("cmyk"));
    let mut o = Map::new();
    psd_options::defaults(false, &mut o);
    assert_eq!(o["colorModel"], json!("rgb"));
}
