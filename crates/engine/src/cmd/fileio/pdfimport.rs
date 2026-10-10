//! Opening and placing PDF (and PDF-compatible `.ai`/`.ait`) files: the options `document.open`
//! and `file.place` read ([`LoadOptions`]), the pages they import and `document.pdfInfo` (pages,
//! boxes, a thumbnail) for the Import PDF dialog.

use serde_json::{Value, json};
use vectorcraft_doc::{ColorMode, Document};
use vectorcraft_pdf::{Choice, CropTo, ImportOptions, PdfError, TextAs};

use super::super::colormgmt::Grays;
use super::super::*;
use super::{err, source};

/// The `document.open` options besides the file. The PDF ones (pages, box, password, text,
/// layers) and the DXF ones are ignored by other formats.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadOptions {
    /// 1-based pages to import, such as `"2-3, 5"`; `None` = every page.
    pub pages: Option<String>,
    /// The box each page's artboard (or placed frame) gets.
    pub crop: CropTo,
    /// The password of an encrypted PDF.
    pub password: Option<String>,
    /// The colour mode the document opens in, its colours converted as Document Color Mode does
    /// (default: the file's; a PDF painted mostly in CMYK opens in CMYK).
    pub color_mode: Option<ColorMode>,
    /// How that conversion separates RGB greys (`grays`).
    pub grays: Grays,
    /// What a PDF's text becomes.
    pub text_as: TextAs,
    /// A PDF's optional content groups become layers (else one layer per page, without the art
    /// that is off).
    pub layers: bool,
    /// How a DXF drawing comes in (the `dxf` param, see [`super::dxfimport`]).
    pub dxf: vectorcraft_cad::ImportOptions,
    /// An Illustrator EPS or `.ai` that carries the editing copy of its art opens from it (its
    /// layers, hidden objects and the art outside its artboards); else only what its page or PDF
    /// part prints comes in.
    pub editing_data: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            pages: None,
            crop: CropTo::default(),
            password: None,
            color_mode: None,
            grays: Grays::default(),
            text_as: TextAs::default(),
            layers: true,
            dxf: Default::default(),
            editing_data: true,
        }
    }
}

impl LoadOptions {
    /// From command params: `pages` (`"2-3, 5"`, a page number or `"all"`), `page` (one page, when
    /// `pages` isn't given), `cropTo` (or `crop`), `password`, `colorMode` (`rgb` | `cmyk`) with `grays`,
    /// `textAs` (`text` | `outlines`), `layers` (true | false), `editingData` (true | false) and `dxf`
    /// (the DXF import options).
    pub fn from_params(cmd: &str, p: &Value) -> Result<Self> {
        let pages = match p.get("pages").filter(|v| !v.is_null()).or_else(|| p.get("page").filter(|v| !v.is_null())) {
            None => None,
            Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("all") => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            Some(v) => return Err(bad(cmd, format!("pages must be a range such as \"2-3\" or a page number, not {v}"))),
        };
        let crop = choice(cmd, "cropTo", str_param(p, "cropTo").or_else(|| str_param(p, "crop")))?;
        let password = str_param(p, "password").filter(|s| !s.is_empty()).map(str::to_string);
        let color_mode = match str_param(p, "colorMode").map(str::to_ascii_lowercase).as_deref() {
            None => None,
            Some("rgb") => Some(ColorMode::Rgb),
            Some("cmyk") => Some(ColorMode::Cmyk),
            Some(m) => return Err(bad(cmd, format!("colorMode must be rgb or cmyk, not `{m}`"))),
        };
        let grays = Grays::param(cmd, p)?;
        let text_as = choice(cmd, "textAs", str_param(p, "textAs"))?;
        let layers = match p.get("layers").filter(|v| !v.is_null()) {
            None => true,
            Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "layers must be true or false"))?,
        };
        let editing_data = match p.get("editingData").filter(|v| !v.is_null()) {
            None => true,
            Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "editingData must be true or false"))?,
        };
        let dxf = super::dxfimport::options(cmd, p.get("dxf"))?;
        Ok(Self { pages, crop, password, color_mode, grays, text_as, layers, dxf, editing_data })
    }

    /// Does the document read only part of the file, or read it differently (a page range, another
    /// box, a password, without its hidden layers)? Writing it back would lose the rest, so Save
    /// doesn't (it asks for a name).
    pub fn is_partial(&self) -> bool {
        self.pages.is_some() || self.crop != CropTo::default() || self.password.is_some() || !self.layers
    }

    /// The PDF import options: `pages` resolved against the file's page count.
    fn import_options(&self, bytes: &[u8]) -> Result<ImportOptions> {
        let pages = match &self.pages {
            None => None,
            Some(range) => {
                let count = vectorcraft_pdf::info(bytes, self.password.as_deref()).map_err(err)?.pages.len();
                Some(parse_range(range, count).map_err(|e| err(format!("pages: {e}")))?)
            }
        };
        Ok(ImportOptions {
            pages,
            crop: self.crop,
            password: self.password.clone(),
            text_as: self.text_as,
            layers: self.layers,
            ..Default::default()
        })
    }
}

/// The option of `T` named `id` (any case), for param `key`; not given: the default.
fn choice<T: Choice + Default>(cmd: &str, key: &str, id: Option<&str>) -> Result<T> {
    let Some(id) = id else { return Ok(T::default()) };
    T::ALL
        .iter()
        .zip(T::IDS)
        .find(|(_, i)| i.eq_ignore_ascii_case(id))
        .map(|(t, _)| *t)
        .ok_or_else(|| bad(cmd, format!("{key} must be one of {}", T::IDS.join(", "))))
}

/// Import the pages `o` pick → the document and import notes.
pub(super) fn import(bytes: &[u8], o: &LoadOptions) -> Result<(Document, Vec<String>)> {
    let r = vectorcraft_pdf::import_with_report(bytes, &o.import_options(bytes)?).map_err(err)?;
    Ok((r.document, r.warnings))
}

/// Page `page` (0-based) of a PDF as a document of one artboard (its `o.crop` box) and one
/// layer of what it shows (text as outlines: it looks as printed), and the import notes.
pub fn page_document(bytes: &[u8], page: usize, o: &LoadOptions) -> Result<(Document, Vec<String>)> {
    let opts = ImportOptions {
        pages: Some(vec![page]),
        crop: o.crop,
        password: o.password.clone(),
        text_as: TextAs::Outlines,
        layers: false,
        ..Default::default()
    };
    vectorcraft_pdf::import_with_report(bytes, &opts).map(|r| (r.document, r.warnings)).map_err(err)
}

const INFO: &str = "document.pdfInfo";

/// `document.pdfInfo`.
pub(super) fn pdf_info(_: &mut Session, p: &Value) -> Result<Value> {
    let bytes = source(p, INFO)?.bytes;
    let o = LoadOptions::from_params(INFO, p)?;
    let info = match vectorcraft_pdf::info(&bytes, o.password.as_deref()) {
        Ok(i) => i,
        Err(e @ (PdfError::NeedsPassword | PdfError::WrongPassword)) => {
            return Ok(json!({"pages": 0, "needsPassword": true, "wrongPassword": e == PdfError::WrongPassword, "pageInfo": []}));
        }
        Err(e) => return Err(err(e)),
    };
    let r4 = |r: vectorcraft_geom::Rect| json!([r.x0, r.y0, r.x1, r.y1]);
    let pages: Vec<Value> = info
        .pages
        .iter()
        .map(|pg| {
            let boxes: serde_json::Map<String, Value> = pg.boxes.iter().map(|(c, r)| (c.id().to_string(), r4(*r))).collect();
            json!({"width": pg.width, "height": pg.height, "rotation": pg.rotation, "boxes": boxes})
        })
        .collect();
    let mut out = json!({"pages": pages.len(), "needsPassword": false, "pageInfo": pages});
    if let Some(n) = p.get("thumbnail").and_then(Value::as_u64) {
        let page = usize::try_from(n).ok().and_then(|n| n.checked_sub(1)).filter(|i| *i < info.pages.len());
        let page = page.ok_or_else(|| bad(INFO, format!("thumbnail: no page {n} (the PDF has {})", info.pages.len())))?;
        let (doc, _) = page_document(&bytes, page, &o)?;
        let size = f64_or(p, "thumbnailSize", 160.0).clamp(16.0, 1024.0);
        let r = doc.artboards.first().map(|a| a.rect).ok_or_else(|| err("the page has no area"))?;
        let png = super::encode(&doc, "png", &json!({"scale": size / r.width().max(r.height()).max(1.0)}))?;
        out["thumbnail"] = json!(vectorcraft_format::base64_encode(&png));
    }
    Ok(out)
}
