//! The slices a Photoshop document (PSD or PSB) stores, for `document.open` and `file.place`
//! with `importSlices`: image resource 1050, read from the "Slices resource format" of Adobe's
//! public Photoshop File Formats Specification
//! (<https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/>). Version 6 stores the slices
//! as records; versions 7 and 8 store them in a descriptor. The descriptor's keys, and the meaning
//! of the origin, type and background values, are read as the MIT-licensed ag-psd reads them
//! (<https://github.com/Agamnentzar/ag-psd>, `src/imageResources.ts` and `src/descriptor.ts`),
//! which agrees with psd.rb (<https://github.com/layervault/psd.rb>,
//! `lib/psd/resources/slices.rb`) on the keys both read.
//!
//! A slice the file marks as generated automatically is left out: the document lays out its own
//! auto slices around the others. A slice that follows a layer comes in as a user slice over the
//! rectangle the file stores for it. Each slice keeps its name, its type (Image or No Image), its
//! link, target, message, alternative text, cell text and background color; text alignment isn't
//! read. The file is untrusted: every read is checked, and the slice count, the strings and the
//! descriptor's nesting and item count are capped.

use vectorcraft_doc::{SliceKind, SliceOptions};
use vectorcraft_geom::Rect;

use super::psdread::{Reader, TRUNCATED, header, resource};

/// The image resource that holds the slices.
const SLICES: u16 = 1050;

/// The most slices one file brings in.
const MAX_SLICES: usize = 10_000;

/// The deepest descriptor objects and lists nest.
const MAX_DEPTH: usize = 16;

/// The most values the descriptors of one slices resource hold together, nested ones included.
const MAX_VALUES: usize = 200_000;

/// The error of descriptors that hold more than [`MAX_VALUES`].
const TOO_MANY_VALUES: &str = "the slices' descriptor holds too many values";

/// One slice as the file stores it.
#[derive(Clone, Debug, PartialEq)]
struct PsdSlice {
    /// Left, top, right and bottom, in the canvas's pixels.
    rect: [i64; 4],
    options: SliceOptions,
}

/// The slices a file stores, those generated automatically left out, and its canvas.
struct Stored {
    slices: Vec<PsdSlice>,
    /// The canvas's width and height, in pixels.
    size: (u32, u32),
}

/// The slices PSD or PSB file `bytes` stores. `None` when it stores none (no `8BIM` resource 1050
/// before the image resources end, or before a block that is cut short ahead of it). An error when
/// the file isn't a PSD or PSB file, resource 1050 is cut short, or its slices can't be read.
fn read(bytes: &[u8]) -> Result<Option<Stored>, String> {
    let mut r = Reader::new(bytes);
    let h = header(&mut r)?;
    let Some(data) = resource(h.resources, SLICES)? else { return Ok(None) };
    Ok(Some(Stored { slices: decode(data)?, size: (h.width, h.height) }))
}

/// The slices of resource 1050's `data`.
fn decode(data: &[u8]) -> Result<Vec<PsdSlice>, String> {
    let mut r = Reader::new(data);
    // The values every descriptor of the resource may hold together.
    let mut budget = MAX_VALUES;
    match r.u32()? {
        6 => version_6(&mut r, &mut budget),
        7 | 8 => {
            if r.u32()? != 16 {
                return Err("the slices' descriptor has a version other than 16".into());
            }
            let d = descriptor(&mut r, 0, &mut budget)?;
            from_descriptor(&d)
        }
        v => Err(format!("the slices are stored in version {v}, which isn't read")),
    }
}

/// Version 6: the bounds of all the slices, the group's name, the count, then each slice's record.
/// The descriptors that follow the records spend one `budget` of values between them.
fn version_6(r: &mut Reader<'_>, budget: &mut usize) -> Result<Vec<PsdSlice>, String> {
    r.take(16)?;
    unicode(r)?;
    let count = r.u32()? as usize;
    if count > MAX_SLICES {
        return Err(format!("the file has {count} slices, more than the {MAX_SLICES} read"));
    }
    let one_after_another = records_one_after_another(Reader::new(r.rest()), count, *budget);
    let mut out = vec![];
    for i in 0..count {
        // The ID and group ID.
        r.take(8)?;
        let origin = r.u32()?;
        // A slice that follows a layer gives the layer's ID.
        if origin == 1 {
            r.take(4)?;
        }
        let name = unicode(r)?;
        let kind = if r.u32()? == 0 { SliceKind::NoImage } else { SliceKind::Image };
        let mut rect = [0i64; 4];
        for v in &mut rect {
            *v = i64::from(r.u32()? as i32);
        }
        let (url, target, message, alt) = (unicode(r)?, unicode(r)?, unicode(r)?, unicode(r)?);
        let html = r.u8()? != 0;
        let cell = unicode(r)?;
        // The horizontal and vertical alignment.
        r.take(8)?;
        let [a, red, green, blue] = r.array::<4>()?;
        let background = if a == 0 && red == 0 && green == 0 && blue == 0 {
            String::new()
        } else if a == 0 {
            "matte".into()
        } else {
            format!("#{red:02x}{green:02x}{blue:02x}")
        };
        // Photoshop 7 and later may follow a record with a descriptor (version 16); a record that
        // doesn't start one is the next slice. When the records follow one another, only the
        // last one may be followed by a descriptor. A descriptor read that uses up the budget is
        // an error.
        if (!one_after_another || i + 1 == count) && r.rest().starts_with(&16u32.to_be_bytes()) {
            let mut ahead = Reader::new(r.rest());
            ahead.u32()?;
            match descriptor(&mut ahead, 0, budget) {
                Ok(_) => {
                    let used = r.rest().len() - ahead.rest().len();
                    r.take(used)?;
                }
                Err(e) if e == TOO_MANY_VALUES => return Err(e),
                Err(_) => {}
            }
        }
        // Origin 0 is a slice generated automatically.
        if origin != 0 {
            let options = SliceOptions { kind, name, url, target, message, alt, text: cell_text(cell, html), background, ..Default::default() };
            out.push(PsdSlice { rect, options: for_kind(options) });
        }
    }
    Ok(out)
}

/// Whether `r` holds `count` version 6 records one after another, followed by nothing or by one
/// descriptor (version 16) that reads within `budget`, as ag-psd writes them. A record whose ID is
/// 16 then isn't taken for a descriptor. Otherwise each record may be followed by a descriptor.
fn records_one_after_another(mut r: Reader<'_>, count: usize, mut budget: usize) -> bool {
    if !(0..count).all(|_| skip_record(&mut r).is_ok()) {
        return false;
    }
    r.rest().is_empty() || (r.rest().starts_with(&16u32.to_be_bytes()) && r.u32().is_ok() && descriptor(&mut r, 0, &mut budget).is_ok())
}

/// Reads past the version 6 record `r` starts with, in the layout [`version_6`] reads. Its strings
/// are checked and read past without being decoded.
fn skip_record(r: &mut Reader<'_>) -> Result<(), String> {
    // The ID and group ID.
    r.take(8)?;
    if r.u32()? == 1 {
        r.take(4)?;
    }
    units(r)?;
    // The type and the rectangle.
    r.take(20)?;
    for _ in 0..4 {
        units(r)?;
    }
    r.take(1)?;
    units(r)?;
    // The alignments and the background color.
    r.take(12)?;
    Ok(())
}

/// Versions 7 and 8: the descriptor's `slices` list, each slice an object.
fn from_descriptor(d: &[(String, Value)]) -> Result<Vec<PsdSlice>, String> {
    let Some(Value::List(list)) = field(d, "slices") else { return Err("the slices' descriptor has no list of slices".into()) };
    let count = list.len();
    if count > MAX_SLICES {
        return Err(format!("the file has {count} slices, more than the {MAX_SLICES} read"));
    }
    let mut out = vec![];
    for s in list {
        let Value::Object(s) = s else { return Err("the slices' descriptor lists something other than a slice".into()) };
        if matches!(field(s, "origin"), Some(Value::Enum(_, v)) if v == "autoGenerated") {
            continue;
        }
        let Some(Value::Object(b)) = field(s, "bounds") else { return Err("a slice in the slices' descriptor has no bounds".into()) };
        let edge = |k: &str| number(field(b, k)).ok_or_else(|| "a slice's bounds in the slices' descriptor aren't numbers".to_string());
        let rect = [edge("Left")?, edge("Top ")?, edge("Rght")?, edge("Btom")?];
        let text = |k: &str| match field(s, k) {
            Some(Value::Text(t)) => t.clone(),
            _ => String::new(),
        };
        let kind = if matches!(field(s, "Type"), Some(Value::Enum(_, v)) if v == "noImage") { SliceKind::NoImage } else { SliceKind::Image };
        let html = matches!(field(s, "cellTextIsHTML"), Some(Value::Bool(true)));
        let background = match field(s, "bgColorType") {
            Some(Value::Enum(_, v)) if v == "matte" => "matte".into(),
            Some(Value::Enum(_, v)) if v == "Clr " => match field(s, "bgColor") {
                Some(Value::Object(c)) => {
                    let channel = |k: &str| number(field(c, k)).map(|v| v.clamp(0, 255));
                    match (channel("Rd  "), channel("Grn "), channel("Bl  ")) {
                        (Some(r), Some(g), Some(b)) => format!("#{r:02x}{g:02x}{b:02x}"),
                        _ => String::new(),
                    }
                }
                _ => String::new(),
            },
            _ => String::new(),
        };
        let options = SliceOptions {
            kind,
            name: text("Nm  "),
            url: text("url"),
            target: text("null"),
            message: text("Msge"),
            alt: text("altTag"),
            text: cell_text(text("cellText"), html),
            background,
            ..Default::default()
        };
        out.push(PsdSlice { rect, options: for_kind(options) });
    }
    Ok(out)
}

/// `o` with only the options of its slice type: an Image slice's cell text isn't shown, and a No
/// Image slice has no link, target, message or alternative text.
fn for_kind(mut o: SliceOptions) -> SliceOptions {
    if o.kind == SliceKind::Image {
        o.text.clear();
    } else {
        for s in [&mut o.url, &mut o.target, &mut o.message, &mut o.alt] {
            s.clear();
        }
    }
    o
}

/// A cell's text as the HTML a No Image slice shows: plain text has `&`, `<` and `>` escaped.
fn cell_text(text: String, html: bool) -> String {
    if html { text } else { text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }
}

/// The longest text one string, key or class ID holds (UTF-16 units or bytes).
const MAX_TEXT: usize = 65_536;

/// An error unless `n`, the length a string or ID declares, is at most [`MAX_TEXT`].
fn text_len(n: usize) -> Result<usize, String> {
    if n > MAX_TEXT {
        return Err(format!("a text in the slices has more than the {MAX_TEXT} characters read"));
    }
    Ok(n)
}

/// A Unicode string: a count of UTF-16 units, then the units; trailing NULs dropped.
fn unicode(r: &mut Reader<'_>) -> Result<String, String> {
    let bytes = units(r)?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|u| u16::from_be_bytes(*u)).collect();
    Ok(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
}

/// A Unicode string's units, as bytes: a count of UTF-16 units, then the units.
fn units<'a>(r: &mut Reader<'a>) -> Result<&'a [u8], String> {
    let n = text_len(r.u32()? as usize)?;
    r.take(n.checked_mul(2).ok_or(TRUNCATED)?)
}

/// A descriptor's value, as far as the slices need it.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Long(i32),
    Double(f64),
    Bool(bool),
    Text(String),
    /// An enumeration's type and value.
    Enum(String, String),
    Object(Vec<(String, Value)>),
    List(Vec<Value>),
    /// A value of another type, read past.
    Other,
}

/// The value with key `key`.
fn field<'a>(d: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    d.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// A whole number: a long, or a finite double rounded.
fn number(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Long(n) => Some(i64::from(*n)),
        Value::Double(d) if d.is_finite() && d.abs() < 1e12 => Some(d.round() as i64),
        _ => None,
    }
}

/// A key or class ID: a length, then that many characters, or four when the length is 0.
fn id(r: &mut Reader<'_>) -> Result<String, String> {
    let n = text_len(r.u32()? as usize)?;
    let b = r.take(if n == 0 { 4 } else { n })?;
    Ok(String::from_utf8_lossy(b).into_owned())
}

/// A class: its Unicode name, then its ID.
fn class(r: &mut Reader<'_>) -> Result<(), String> {
    unicode(r)?;
    id(r)?;
    Ok(())
}

/// A descriptor: its class, the count of its items, then each item's key, type and value.
fn descriptor(r: &mut Reader<'_>, depth: usize, budget: &mut usize) -> Result<Vec<(String, Value)>, String> {
    if depth > MAX_DEPTH {
        return Err("the slices' descriptor nests too deep".into());
    }
    class(r)?;
    let count = r.u32()? as usize;
    let mut out = vec![];
    for _ in 0..count {
        let key = id(r)?;
        let kind = r.array::<4>()?;
        out.push((key, value(r, &kind, depth, budget)?));
    }
    Ok(out)
}

/// A value of type `kind`.
fn value(r: &mut Reader<'_>, kind: &[u8; 4], depth: usize, budget: &mut usize) -> Result<Value, String> {
    *budget = budget.checked_sub(1).ok_or(TOO_MANY_VALUES)?;
    Ok(match kind {
        b"long" => Value::Long(r.u32()? as i32),
        b"doub" => Value::Double(f64::from_be_bytes(r.array()?)),
        b"UntF" => {
            r.take(4)?;
            Value::Double(f64::from_be_bytes(r.array()?))
        }
        b"UnFl" => {
            r.take(4)?;
            Value::Double(f64::from(f32::from_be_bytes(r.array()?)))
        }
        b"bool" => Value::Bool(r.u8()? != 0),
        b"TEXT" => Value::Text(unicode(r)?),
        b"enum" => Value::Enum(id(r)?, id(r)?),
        b"Objc" | b"GlbO" => Value::Object(descriptor(r, depth + 1, budget)?),
        b"VlLs" => {
            if depth >= MAX_DEPTH {
                return Err("the slices' descriptor nests too deep".into());
            }
            let count = r.u32()? as usize;
            let mut items = vec![];
            for _ in 0..count {
                let kind = r.array::<4>()?;
                items.push(value(r, &kind, depth + 1, budget)?);
            }
            Value::List(items)
        }
        b"comp" => {
            r.take(8)?;
            Value::Other
        }
        b"type" | b"GlbC" => {
            class(r)?;
            Value::Other
        }
        b"tdta" | b"alis" => {
            let n = r.u32()? as usize;
            r.take(n)?;
            Value::Other
        }
        b"obj " => {
            let count = r.u32()? as usize;
            for _ in 0..count {
                *budget = budget.checked_sub(1).ok_or(TOO_MANY_VALUES)?;
                match &r.array::<4>()? {
                    b"prop" => {
                        class(r)?;
                        id(r)?;
                    }
                    b"Clss" => class(r)?,
                    b"Enmr" => {
                        class(r)?;
                        id(r)?;
                        id(r)?;
                    }
                    b"rele" => {
                        class(r)?;
                        r.take(4)?;
                    }
                    b"Idnt" | b"indx" => {
                        r.take(4)?;
                    }
                    b"name" => {
                        class(r)?;
                        unicode(r)?;
                    }
                    k => return Err(format!("the slices' descriptor has a reference of an unknown kind ({})", String::from_utf8_lossy(k))),
                }
            }
            Value::Other
        }
        k => return Err(format!("the slices' descriptor has a value of an unknown type ({})", String::from_utf8_lossy(k))),
    })
}

/// The slices of PSD or PSB file `bytes` laid over `canvas`, the box its canvas takes (points),
/// each clipped to the canvas, with warnings: for slices that lie wholly outside it or have no
/// size, and for slices that can't be read (the file comes in without them). Empty, without a
/// warning, when the file stores no slices.
pub(crate) fn over(bytes: &[u8], canvas: Rect) -> (Vec<(Rect, SliceOptions)>, Vec<String>) {
    match read(bytes) {
        Ok(None) => (vec![], vec![]),
        Ok(Some(s)) => laid_over(&s.slices, s.size, canvas),
        Err(e) => (vec![], vec![format!("the slices can't be read ({e}) and were left out")]),
    }
}

/// `slices` of a canvas of `size` pixels laid over `canvas` (points).
fn laid_over(slices: &[PsdSlice], (w, h): (u32, u32), canvas: Rect) -> (Vec<(Rect, SliceOptions)>, Vec<String>) {
    let (sx, sy) = (canvas.width() / f64::from(w.max(1)), canvas.height() / f64::from(h.max(1)));
    let mut out = vec![];
    for s in slices {
        let [l, t, r, b] = s.rect;
        let (l, r) = (l.clamp(0, i64::from(w)), r.clamp(0, i64::from(w)));
        let (t, b) = (t.clamp(0, i64::from(h)), b.clamp(0, i64::from(h)));
        if l < r && t < b {
            let x = |v: i64| canvas.x0 + v as f64 * sx;
            let y = |v: i64| canvas.y0 + v as f64 * sy;
            out.push((Rect::new(x(l), y(t), x(r), y(b)), s.options.clone()));
        }
    }
    let warnings = match slices.len() - out.len() {
        0 => vec![],
        1 => vec!["1 slice lies outside the canvas or has no size and was left out".to_string()],
        n => vec![format!("{n} slices lie outside the canvas or have no size and were left out")],
    };
    (out, warnings)
}

/// How many slices of PSD or PSB file `bytes` come in (`document.psdInfo`): 0 when it stores none
/// or they can't be read.
pub(crate) fn count(bytes: &[u8]) -> usize {
    match read(bytes) {
        Ok(Some(Stored { slices, size: (w, h) })) => laid_over(&slices, (w, h), Rect::new(0.0, 0.0, f64::from(w), f64::from(h))).0.len(),
        _ => 0,
    }
}
