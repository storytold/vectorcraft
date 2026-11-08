//! The global functions (ECMA-262 5.1 section 15.1.2 and 15.1.3): `parseInt`, `parseFloat`,
//! `isNaN`, `isFinite` and the URI functions, and Annex B's `escape` and `unescape` (sections
//! B.2.1 and B.2.2). Each code unit read takes a unit of fuel, and the strings they build are
//! checked against the heap limit as they grow.

use super::{Builder, arg, method};
use crate::heap::NativeFn;
use crate::interp::{ErrorType, Interp, JsResult};
use crate::value::{Value, is_str_space, to_int32};

pub(super) fn install(interp: &mut Interp) -> JsResult<()> {
    let global = interp.global;
    let functions: [(&'static str, NativeFn, u32); 10] = [
        ("parseInt", parse_int, 2),
        ("parseFloat", parse_float, 1),
        ("isNaN", is_nan, 1),
        ("isFinite", is_finite, 1),
        ("decodeURI", decode_uri, 1),
        ("decodeURIComponent", decode_uri_component, 1),
        ("encodeURI", encode_uri, 1),
        ("encodeURIComponent", encode_uri_component, 1),
        ("escape", escape, 1),
        ("unescape", unescape, 1),
    ];
    for (name, f, length) in functions {
        method(interp, global, name, f, length)?;
    }
    Ok(())
}

/// `s` after its leading white space and line terminators.
fn trim_start(s: &[u16]) -> &[u16] {
    s.get(s.iter().position(|&u| !is_str_space(u)).unwrap_or(s.len())..).unwrap_or_default()
}

/// ToString of the first argument, as code units, with each one taking a unit of fuel. A
/// concatenation is flattened into a new buffer, which is charged to the heap.
fn units_of(interp: &mut Interp, args: &[Value]) -> JsResult<std::rc::Rc<[u16]>> {
    let s = interp.to_str(&arg(args, 0))?;
    for _ in 0..s.len() {
        interp.use_fuel()?;
    }
    let rope = s.buffer_addr().is_none();
    let bytes = crate::heap::string_bytes(s.len());
    if rope {
        let mark = interp.temp_roots.len();
        interp.temp_roots.push(Value::String(s.clone()));
        let reserved = interp.reserve_heap(bytes);
        interp.temp_roots.truncate(mark);
        reserved?;
    }
    let units = s.flat();
    if rope {
        interp.heap.bytes = interp.heap.bytes.saturating_add(bytes);
    }
    Ok(units)
}

fn digit_value(u: u16) -> Option<u32> {
    char::from_u32(u32::from(u)).and_then(|c| c.to_digit(36))
}

/// `parseInt` (section 15.1.2.2).
fn parse_int(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = units_of(interp, args)?;
    let radix = to_int32(interp.to_number(&arg(args, 1))?);
    let mut rest = trim_start(&s);
    let sign = match rest.first() {
        Some(&u) if u == u16::from(b'-') => -1.0,
        _ => 1.0,
    };
    if matches!(rest.first(), Some(&u) if u == u16::from(b'-') || u == u16::from(b'+')) {
        rest = rest.get(1..).unwrap_or_default();
    }
    let mut r = radix as u32;
    let mut strip_prefix = true;
    if radix != 0 {
        if !(2..=36).contains(&radix) {
            return Ok(Value::Number(f64::NAN));
        }
        strip_prefix = radix == 16;
    } else {
        r = 10;
    }
    if strip_prefix
        && rest.len() >= 2
        && rest.first() == Some(&u16::from(b'0'))
        && matches!(rest.get(1), Some(&u) if u == u16::from(b'x') || u == u16::from(b'X'))
    {
        rest = rest.get(2..).unwrap_or_default();
        r = 16;
    }
    let n = rest.iter().map_while(|&u| digit_value(u).filter(|&d| d < r)).count();
    let prefix = rest.get(..n).unwrap_or_default();
    if prefix.is_empty() {
        return Ok(Value::Number(f64::NAN));
    }
    let digits = prefix.iter().filter_map(|&u| digit_value(u));
    let value = match r {
        10 => parse_decimal(interp, prefix)?,
        2 | 4 | 8 | 16 | 32 => crate::chars::radix_value(digits, r).unwrap_or(f64::NAN),
        // Other radices may be approximated (section 15.1.2.2, step 13).
        _ => digits.fold(0.0, |n, d| n * f64::from(r) + f64::from(d)),
    };
    Ok(Value::Number(sign * value))
}

/// The number that `text`, ASCII decimal digits with an optional fraction and exponent, stands
/// for. The text is copied into a string of one byte per code unit for parsing, and that copy is
/// checked against the heap limit first.
fn parse_decimal(interp: &mut Interp, text: &[u16]) -> JsResult<f64> {
    interp.reserve_heap(text.len())?;
    let mut ascii = String::with_capacity(text.len());
    ascii.extend(text.iter().filter_map(|&u| char::from_u32(u32::from(u))));
    Ok(ascii.parse::<f64>().unwrap_or(f64::NAN))
}

/// The length of the longest prefix of `s` that is a StrDecimalLiteral without its sign and
/// without `Infinity`.
fn decimal_prefix(s: &[u16]) -> usize {
    let is_digit = |i: usize| s.get(i).is_some_and(|&u| (u16::from(b'0')..=u16::from(b'9')).contains(&u));
    let mut i = 0;
    while is_digit(i) {
        i += 1;
    }
    let mut mantissa = i;
    if s.get(i) == Some(&u16::from(b'.')) {
        let mut j = i + 1;
        while is_digit(j) {
            j += 1;
        }
        mantissa += j - i - 1;
        if mantissa > 0 {
            i = j;
        }
    }
    if mantissa == 0 {
        return 0;
    }
    if matches!(s.get(i), Some(&u) if u == u16::from(b'e') || u == u16::from(b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(&u) if u == u16::from(b'+') || u == u16::from(b'-')) {
            j += 1;
        }
        let digits_start = j;
        while is_digit(j) {
            j += 1;
        }
        if j > digits_start {
            i = j;
        }
    }
    i
}

/// `parseFloat` (section 15.1.2.3).
fn parse_float(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = units_of(interp, args)?;
    let mut rest = trim_start(&s);
    let mut sign = 1.0;
    if let Some(&u) = rest.first()
        && (u == u16::from(b'-') || u == u16::from(b'+'))
    {
        if u == u16::from(b'-') {
            sign = -1.0;
        }
        rest = rest.get(1..).unwrap_or_default();
    }
    let infinity: Vec<u16> = "Infinity".encode_utf16().collect();
    if rest.starts_with(&infinity) {
        return Ok(Value::Number(sign * f64::INFINITY));
    }
    let n = decimal_prefix(rest);
    if n == 0 {
        return Ok(Value::Number(f64::NAN));
    }
    Ok(Value::Number(sign * parse_decimal(interp, rest.get(..n).unwrap_or_default())?))
}

/// `isNaN` (section 15.1.2.4).
fn is_nan(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Bool(interp.to_number(&arg(args, 0))?.is_nan()))
}

/// `isFinite` (section 15.1.2.5).
fn is_finite(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Bool(interp.to_number(&arg(args, 0))?.is_finite()))
}

// ---- URIs ----

const URI_RESERVED: &str = ";/?:@&=+$,";
const URI_MARK: &str = "-_.!~*'()";

fn in_set(u: u16, extra: &str) -> bool {
    u < 0x80 && extra.as_bytes().contains(&(u as u8))
}

fn is_unreserved(u: u16) -> bool {
    u < 0x80 && ((u as u8).is_ascii_alphanumeric() || in_set(u, URI_MARK))
}

fn uri_error(interp: &mut Interp, why: &str) -> crate::interp::Abrupt {
    interp.throw(ErrorType::Uri, why)
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn push_byte(out: &mut Builder, interp: &mut Interp, b: u8) -> JsResult<()> {
    let units = [u16::from(b'%'), u16::from(HEX[usize::from(b >> 4)]), u16::from(HEX[usize::from(b & 15)])];
    out.push_units(interp, &units)
}

/// Encode (section 15.1.3): every code unit outside the unescaped set as the `%XX` escapes of
/// its UTF-8 bytes.
fn encode(interp: &mut Interp, args: &[Value], unescaped: fn(u16) -> bool) -> JsResult<Value> {
    let s = units_of(interp, args)?;
    let mut out = Builder::new();
    let mut k = 0;
    while let Some(&c) = s.get(k) {
        if unescaped(c) {
            out.push_units(interp, &[c])?;
            k += 1;
            continue;
        }
        let v = match c {
            0xDC00..=0xDFFF => return Err(uri_error(interp, "a lone trailing surrogate cannot be encoded")),
            0xD800..=0xDBFF => match s.get(k + 1) {
                Some(&d) if (0xDC00..=0xDFFF).contains(&d) => {
                    k += 1;
                    (u32::from(c) - 0xD800) * 0x400 + (u32::from(d) - 0xDC00) + 0x10000
                }
                _ => return Err(uri_error(interp, "a lone leading surrogate cannot be encoded")),
            },
            _ => u32::from(c),
        };
        let mut buf = [0u8; 4];
        let Some(ch) = char::from_u32(v) else { return Err(uri_error(interp, "the string holds a code point that cannot be encoded")) };
        for &b in ch.encode_utf8(&mut buf).as_bytes() {
            push_byte(&mut out, interp, b)?;
        }
        k += 1;
    }
    Ok(out.finish())
}

fn hex_byte(s: &[u16], at: usize) -> Option<u8> {
    let hi = crate::chars::hex_value(u32::from(*s.get(at)?))?;
    let lo = crate::chars::hex_value(u32::from(*s.get(at + 1)?))?;
    u8::try_from(hi * 16 + lo).ok()
}

/// Decode (section 15.1.3): each `%XX` escape sequence of a UTF-8 character as the character,
/// except an escaped character in `reserved`, which stays escaped.
fn decode(interp: &mut Interp, args: &[Value], reserved: &str) -> JsResult<Value> {
    let s = units_of(interp, args)?;
    let mut out = Builder::new();
    let mut k = 0;
    while let Some(&c) = s.get(k) {
        if c != u16::from(b'%') {
            out.push_units(interp, &[c])?;
            k += 1;
            continue;
        }
        let start = k;
        let Some(b) = hex_byte(&s, k + 1) else { return Err(uri_error(interp, "a % is not followed by two hexadecimal digits")) };
        k += 2;
        if b & 0x80 == 0 {
            let c = u16::from(b);
            if in_set(c, reserved) {
                out.push_units(interp, s.get(start..=k).unwrap_or_default())?;
            } else {
                out.push_units(interp, &[c])?;
            }
            k += 1;
            continue;
        }
        let n = b.leading_ones() as usize;
        if n == 1 || n > 4 {
            return Err(uri_error(interp, "an escape sequence is not the start of a UTF-8 character"));
        }
        let mut octets = vec![b];
        for _ in 1..n {
            k += 1;
            let next = if s.get(k) == Some(&u16::from(b'%')) { hex_byte(&s, k + 1) } else { None };
            let Some(b) = next.filter(|b| b & 0xC0 == 0x80) else { return Err(uri_error(interp, "an escaped UTF-8 character is incomplete")) };
            k += 2;
            octets.push(b);
        }
        let Some(ch) = std::str::from_utf8(&octets).ok().and_then(|t| t.chars().next()) else {
            return Err(uri_error(interp, "an escape sequence is not valid UTF-8"));
        };
        let mut units = [0u16; 2];
        out.push_units(interp, ch.encode_utf16(&mut units))?;
        k += 1;
    }
    Ok(out.finish())
}

fn encode_uri(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    encode(interp, args, |u| is_unreserved(u) || in_set(u, URI_RESERVED) || u == u16::from(b'#'))
}

fn encode_uri_component(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    encode(interp, args, is_unreserved)
}

fn decode_uri(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    decode(interp, args, ";/?:@&=+$,#")
}

fn decode_uri_component(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    decode(interp, args, "")
}

// ---- Annex B ----

/// `escape` (section B.2.1).
fn escape(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = units_of(interp, args)?;
    let mut out = Builder::new();
    for &u in s.iter() {
        if u < 0x80 && ((u as u8).is_ascii_alphanumeric() || b"@*_+-./".contains(&(u as u8))) {
            out.push_units(interp, &[u])?;
        } else if u < 256 {
            push_byte(&mut out, interp, u as u8)?;
        } else {
            let hex = |shift: u16| u16::from(HEX[usize::from((u >> shift) & 15)]);
            out.push_units(interp, &[u16::from(b'%'), u16::from(b'u'), hex(12), hex(8), hex(4), hex(0)])?;
        }
    }
    Ok(out.finish())
}

/// `unescape` (section B.2.2).
fn unescape(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = units_of(interp, args)?;
    let mut out = Builder::new();
    let mut k = 0;
    let hex4 = |at: usize| -> Option<u16> {
        let mut v = 0u16;
        for i in 0..4 {
            v = v * 16 + u16::try_from(crate::chars::hex_value(u32::from(*s.get(at + i)?))?).ok()?;
        }
        Some(v)
    };
    while let Some(&c) = s.get(k) {
        let mut unit = c;
        if c == u16::from(b'%') {
            if s.get(k + 1) == Some(&u16::from(b'u'))
                && let Some(v) = hex4(k + 2)
            {
                unit = v;
                k += 5;
            } else if let Some(b) = hex_byte(&s, k + 1) {
                unit = u16::from(b);
                k += 2;
            }
        }
        out.push_units(interp, &[unit])?;
        k += 1;
    }
    Ok(out.finish())
}
