//! The settings of a filter-gallery effect in an art style: a basic filter's `/Binary :
//! /ASCII85Decode ,` block, whose comment lines hold the bytes until `~>`, then `; (data) ,` and the
//! byte count `n /Int (dataSize) ,`. The bytes are a descriptor, all numbers big-endian:
//!
//! ```text
//! version (u32, 1)
//! field*      key (4 chars) marker (u32) …
//!   marker 0:          type (4 chars: long, doub, bool) value (i32, f64 or one byte)
//!   marker 0x2000:     an enumeration: type (4 chars) value (4 chars)
//!   marker 0x80000000: an object: class (4 chars) length (u32) and that many bytes, a descriptor
//! ```
//!
//! A filter-gallery effect's first field is `GEfk`, an enumeration of type `GEft` whose value is the
//! effect's four-character code. The keys and the enumerations' values are the Photoshop plug-in
//! terminology's (https://adobesdk.neocities.org/pluginsdk/documentation/html/_p_i_terminology_8h_source).
//! Anything else (another marker or type, a field past the end, bytes after the last field, more
//! than [`MAX_FIELDS`] fields or objects nested more than [`MAX_DEPTH`] deep) isn't read.

use serde_json::{Value as Json, json};

/// Most bytes a block's text is read from (its comment lines and `~>`).
pub(super) const MAX_ENCODED: usize = 6 * 1024;
/// Most bytes a block decodes to.
pub(super) const MAX_DECODED: usize = 4 * 1024;
/// Most fields read in one descriptor, nested ones included.
const MAX_FIELDS: usize = 32;
/// Most levels of objects in objects.
const MAX_DEPTH: usize = 2;

/// A field's value.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Long(i32),
    Doub(f64),
    Bool(bool),
    /// An enumeration's type and value.
    Enum([u8; 4], [u8; 4]),
    /// An object's class and fields.
    Object([u8; 4], Vec<(Key, Value)>),
}

/// A field's key.
pub(super) type Key = [u8; 4];

/// The fields of descriptor `data`; `None` when it isn't one read here.
pub(super) fn decode(data: &[u8]) -> Option<Vec<(Key, Value)>> {
    let mut count = 0;
    fields(data, 0, &mut count)
}

fn fields(data: &[u8], depth: usize, count: &mut usize) -> Option<Vec<(Key, Value)>> {
    let mut r = Reader { data, at: 0 };
    if r.u32()? != 1 {
        return None;
    }
    let mut out = vec![];
    while r.at < data.len() {
        *count += 1;
        if *count > MAX_FIELDS {
            return None;
        }
        let key = r.four()?;
        let value = match r.u32()? {
            0 => match &r.four()? {
                b"long" => Value::Long(i32::from_be_bytes(r.four()?)),
                b"doub" => Value::Doub(f64::from_be_bytes(r.eight()?)).finite()?,
                b"bool" => match r.take(1)? {
                    [0] => Value::Bool(false),
                    [1] => Value::Bool(true),
                    _ => return None,
                },
                _ => return None,
            },
            0x2000 => Value::Enum(r.four()?, r.four()?),
            0x8000_0000 if depth < MAX_DEPTH => {
                let class = r.four()?;
                let n = usize::try_from(r.u32()?).ok()?;
                Value::Object(class, fields(r.take(n)?, depth + 1, count)?)
            }
            _ => return None,
        };
        out.push((key, value));
    }
    Some(out)
}

impl Value {
    fn finite(self) -> Option<Self> {
        match self {
            Value::Doub(v) if !v.is_finite() => None,
            v => Some(v),
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let bytes = self.data.get(self.at..end)?;
        self.at = end;
        Some(bytes)
    }

    fn four(&mut self) -> Option<[u8; 4]> {
        self.take(4)?.try_into().ok()
    }

    fn eight(&mut self) -> Option<[u8; 8]> {
        self.take(8)?.try_into().ok()
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.four()?))
    }
}

/// The fields of a filter-gallery effect `code`, its `GEfk` field taken off; `None` when that field
/// names another effect.
fn gallery<'a>(fields: &'a [(Key, Value)], code: &[u8; 4]) -> Option<&'a [(Key, Value)]> {
    match fields.split_first() {
        Some((([b'G', b'E', b'f', b'k'], Value::Enum(kind, value)), rest)) => (kind == b"GEft" && value == code).then_some(rest),
        _ => Some(fields),
    }
}

/// A whole number in `range`.
fn long(v: &Value, range: std::ops::RangeInclusive<i32>) -> Option<f64> {
    match v {
        Value::Long(n) if range.contains(n) => Some(f64::from(*n)),
        _ => None,
    }
}

/// Sketch › Bas Relief's params (`sketch.basRelief`) from its fields: `Dtl ` (detail), `Smth`
/// (smoothness) and `LghD` (the light's direction), each once; `None` when one is missing, out of
/// range or not read here, or another field is there.
pub(super) fn bas_relief(fields: &[(Key, Value)]) -> Option<Json> {
    let (mut detail, mut smoothness, mut light) = (None, None, None);
    for (key, value) in gallery(fields, b"BsRl")? {
        let slot = match key {
            b"Dtl " => &mut detail,
            b"Smth" => &mut smoothness,
            b"LghD" => &mut light,
            _ => return None,
        };
        if slot.is_some() {
            return None;
        }
        *slot = Some(match key {
            b"LghD" => json!(light_direction(value)?),
            _ => json!(long(value, 1..=15)?),
        });
    }
    Some(json!({"detail": detail?, "smoothness": smoothness?, "lightDirection": light?}))
}

/// The light direction of a `LghD` enumeration's value (`typeLightDirection`).
fn light_direction(v: &Value) -> Option<&'static str> {
    let Value::Enum(kind, value) = v else { return None };
    if kind != b"LghD" {
        return None;
    }
    Some(match value {
        b"LDBt" => "bottom",
        b"LDBL" => "bottomLeft",
        b"LDLf" => "left",
        b"LDTL" => "topLeft",
        b"LDTp" => "top",
        b"LDTR" => "topRight",
        b"LDRg" => "right",
        b"LDBR" => "bottomRight",
        _ => return None,
    })
}
