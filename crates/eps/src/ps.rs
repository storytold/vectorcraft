//! PostScript writing primitives: numbers, strings, paths, the prolog's short operator names, and
//! the data encodings images and embedded data use (ASCII85, run-length, Flate).

use std::fmt::Write;
use std::io::{Read, Write as _};

use kurbo::PathEl;
use vectorcraft_geom::{Affine, BezPath, FillRule};

/// The procedures the page uses, in `VCdict`.
pub(crate) const PROLOG: &str = "/VCdict 64 dict def
VCdict begin
/bd {bind def} bind def
/m {moveto} bd /l {lineto} bd /c {curveto} bd /h {closepath} bd
/f {fill} bd /f* {eofill} bd /S {stroke} bd
/W {clip newpath} bd /W* {eoclip newpath} bd
/q {gsave} bd /Q {grestore} bd /cm {concat} bd
/g {setgray} bd /rg {setrgbcolor} bd /k {setcmykcolor} bd
/w {setlinewidth} bd /J {setlinecap} bd /j {setlinejoin} bd /M {setmiterlimit} bd /d {setdash} bd
/op {setoverprint} bd
end
";

/// Longest line of encoded data (DSC readers take up to 255 characters).
const LINE: usize = 76;

/// `v` as PostScript writes it: at most four decimals, no trailing zeros (0 when not finite).
pub(crate) fn num(v: f64) -> String {
    let mut s = String::new();
    push_num(&mut s, v);
    s
}

/// Append [`num`]`(v)` to `s`.
pub(crate) fn push_num(s: &mut String, v: f64) {
    // Beyond ~1.8e304 the scaling overflows; such values have no fractional digits to round.
    let v = if !v.is_finite() {
        0.0
    } else if (v * 10_000.0).is_finite() {
        (v * 10_000.0).round() / 10_000.0
    } else {
        v
    };
    if v == v.trunc() && v.abs() < 1e15 {
        let _ = write!(s, "{}", v as i64);
        return;
    }
    let start = s.len();
    let _ = write!(s, "{v:.4}");
    let keep = s[start..].trim_end_matches('0').trim_end_matches('.').len();
    s.truncate(start + keep);
}

/// `vs` as numbers separated by spaces, then a space.
pub(crate) fn push_nums(s: &mut String, vs: &[f64]) {
    for v in vs {
        push_num(s, *v);
        s.push(' ');
    }
}

/// `text` as a PostScript string: `(` `)` `\` escaped, anything outside printable ASCII as octal
/// escapes of its UTF-8 bytes.
pub(crate) fn string(text: &str) -> String {
    let mut s = String::with_capacity(text.len() + 2);
    s.push('(');
    for b in text.bytes() {
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(char::from(b));
            }
            0x20..=0x7e => s.push(char::from(b)),
            _ => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s.push(')');
    s
}

/// `bytes` as a PostScript string: `(` `)` `\` escaped, carriage returns, line feeds and tabs as
/// `\r` `\n` `\t`, other bytes outside printable ASCII as octal escapes.
pub(crate) fn bytes(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() + 2);
    s.push('(');
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(char::from(b));
            }
            b'\r' => s.push_str("\\r"),
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            0x20..=0x7e => s.push(char::from(b)),
            _ => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s.push(')');
    s
}

/// A matrix operand `[a b c d e f]`.
pub(crate) fn matrix(a: Affine) -> String {
    let mut s = String::from("[");
    push_nums(&mut s, &a.as_coeffs());
    s.pop();
    s.push(']');
    s
}

/// Append path `bp` (`m`, `l`, `c`, `h`; quadratics raised to cubics), one segment a line.
pub(crate) fn push_path(s: &mut String, bp: &BezPath) {
    let mut last = kurbo::Point::ZERO;
    let mut start = kurbo::Point::ZERO;
    for el in bp.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                push_nums(s, &[p.x, p.y]);
                s.push_str("m\n");
                (last, start) = (p, p);
            }
            PathEl::LineTo(p) => {
                push_nums(s, &[p.x, p.y]);
                s.push_str("l\n");
                last = p;
            }
            PathEl::QuadTo(a, p) => {
                let c1 = last + (a - last) * (2.0 / 3.0);
                let c2 = p + (a - p) * (2.0 / 3.0);
                push_nums(s, &[c1.x, c1.y, c2.x, c2.y, p.x, p.y]);
                s.push_str("c\n");
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                push_nums(s, &[a.x, a.y, b.x, b.y, p.x, p.y]);
                s.push_str("c\n");
                last = p;
            }
            PathEl::ClosePath => {
                s.push_str("h\n");
                last = start;
            }
        }
    }
}

/// The fill operator for `rule`.
pub(crate) fn fill_op(rule: FillRule) -> &'static str {
    match rule {
        FillRule::NonZero => "f\n",
        FillRule::EvenOdd => "f*\n",
    }
}

/// The clip operator for `rule` (it also ends the path).
pub(crate) fn clip_op(rule: FillRule) -> &'static str {
    match rule {
        FillRule::NonZero => "W\n",
        FillRule::EvenOdd => "W*\n",
    }
}

/// `bytes` in ASCII85 with its `~>` end, in lines of about [`LINE`] characters, none starting
/// with `%` (DSC readers would take it for a comment).
pub(crate) fn ascii85(bytes: &[u8]) -> String {
    let mut raw = String::with_capacity(bytes.len() * 5 / 4 + 8);
    let (chunks, rest) = bytes.as_chunks::<4>();
    for c in chunks {
        let v = u32::from_be_bytes(*c);
        if v == 0 {
            raw.push('z');
        } else {
            push85(&mut raw, v, 5);
        }
    }
    if !rest.is_empty() {
        let mut c = [0u8; 4];
        for (d, s) in c.iter_mut().zip(rest) {
            *d = *s;
        }
        push85(&mut raw, u32::from_be_bytes(c), rest.len() + 1);
    }
    raw.push_str("~>");
    let mut out = String::with_capacity(raw.len() + raw.len() / LINE + 2);
    let mut col = 0;
    for ch in raw.chars() {
        if col >= LINE && ch != '%' {
            out.push('\n');
            col = 0;
        }
        out.push(ch);
        col += 1;
    }
    out.push('\n');
    out
}

/// The first `n` base-85 digits of `v`.
fn push85(s: &mut String, mut v: u32, n: usize) {
    let mut d = [0u8; 5];
    for slot in d.iter_mut().rev() {
        *slot = (v % 85) as u8 + b'!';
        v /= 85;
    }
    for b in d.iter().take(n) {
        s.push(char::from(*b));
    }
}

/// Most bytes [`ascii85_decode`] makes: what one filter or data section may hold.
pub(crate) const MAX_ASCII85: usize = 1 << 28;

/// ASCII85 `text` (whitespace ignored, up to `~>`) decoded; `None` when it is malformed or longer
/// than [`MAX_ASCII85`].
pub(crate) fn ascii85_decode(text: &str) -> Option<Vec<u8>> {
    ascii85_decode_within(text, MAX_ASCII85)
}

/// [`ascii85_decode`] of at most `max` bytes: `z` is four bytes for one, so the length of the
/// text doesn't bound what it becomes (#1715).
pub(crate) fn ascii85_decode_within(text: &str, max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity((text.len() / 5 * 4).min(max));
    let mut group = [0u8; 5];
    let mut n = 0;
    let mut chars = text.bytes().filter(|b| !b.is_ascii_whitespace()).peekable();
    while let Some(b) = chars.next() {
        if out.len() > max {
            return None;
        }
        match b {
            b'~' => {
                if chars.next() != Some(b'>') {
                    return None;
                }
                if n == 1 {
                    return None;
                }
                if n > 1 {
                    let padded: Vec<u8> = group.iter().take(n).copied().chain(std::iter::repeat(b'u')).take(5).collect();
                    let v = value85(&padded)?;
                    out.extend(v.to_be_bytes().iter().take(n - 1));
                }
                return (out.len() <= max).then_some(out);
            }
            b'z' if n == 0 => out.extend([0; 4]),
            b'!'..=b'u' => {
                *group.get_mut(n)? = b;
                n += 1;
                if n == 5 {
                    out.extend(value85(&group)?.to_be_bytes());
                    n = 0;
                }
            }
            _ => return None,
        }
    }
    None
}

/// The value of five base-85 digits; `None` past `u32::MAX`.
fn value85(digits: &[u8]) -> Option<u32> {
    digits.iter().try_fold(0u32, |acc, d| acc.checked_mul(85)?.checked_add(u32::from(d.checked_sub(b'!')?)))
}

/// Run-length encoding (PackBits: what PostScript's `RunLengthDecode` reads), shared with TIFF.
pub(crate) use vectorcraft_render::encode::tiff::packbits;

/// `data` compressed for `FlateDecode` (zlib).
pub(crate) fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::with_capacity(data.len() / 2), flate2::Compression::default());
    // Writing to a Vec doesn't fail.
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// zlib `data` inflated, when it is valid and at most `max` bytes.
pub(crate) fn inflate(data: &[u8], max: u64) -> Option<Vec<u8>> {
    let mut out = vec![];
    flate2::read::ZlibDecoder::new(data).take(max + 1).read_to_end(&mut out).ok()?;
    (out.len() as u64 <= max).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::num;

    #[test]
    fn huge_finite_numbers_are_valid_postscript() {
        for v in [1.5e305, -1.5e305, f64::MAX, f64::MIN] {
            let s = num(v);
            assert!(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.'), "{v}: got {s:?}");
        }
    }
}
