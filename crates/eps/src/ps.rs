//! PostScript writing primitives: numbers, strings, paths, the prolog's short operator names, and
//! the data encodings images and embedded data use (ASCII85, run-length, Flate).

use vectorcraft_geom::{BezPath, FillRule};

pub(crate) use craft_ps::write::{bytes, deflate, inflate, matrix, num, push_num, push_num_places, push_nums, string, written};

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

/// Append path `bp` using the short operators defined by our `VCdict` prolog.
pub(crate) fn push_path(s: &mut String, bp: &BezPath) {
    craft_ps::write::push_path(s, bp, craft_ps::write::PathOps::SHORT);
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
pub(crate) use craft_eps::ascii85::encode as ascii85;

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
    craft_eps::ascii85::decode(text.as_bytes(), max)
}

/// Run-length encoding (PackBits: what PostScript's `RunLengthDecode` reads), shared with TIFF.
pub(crate) use vectorcraft_render::encode::tiff::packbits;

#[cfg(test)]
mod tests {
    use super::{ascii85, ascii85_decode, num, push_num_places};

    #[test]
    fn shared_writer_preserves_vcdict_path_and_operands() {
        let mut path = vectorcraft_geom::BezPath::new();
        path.move_to((1.0, 2.0));
        path.line_to((3.0, 4.0));
        path.close_path();
        let mut output = String::new();
        super::push_path(&mut output, &path);
        assert_eq!(output, "1 2 m\n3 4 l\nh\n");
        assert_eq!(super::matrix(vectorcraft_geom::Affine::IDENTITY), "[1 0 0 1 0 0]");
        assert_eq!(super::fill_op(vectorcraft_geom::FillRule::EvenOdd), "f*\n");
        assert_eq!(super::clip_op(vectorcraft_geom::FillRule::NonZero), "W\n");
    }

    #[test]
    fn shared_writer_keeps_large_integer_zeroes_and_bounded_flate() {
        let mut output = String::new();
        push_num_places(&mut output, 1e15, 0);
        assert_eq!(output, "1000000000000000");
        let packed = super::deflate(b"EPS payload");
        assert_eq!(super::inflate(&packed, 11).as_deref(), Some(&b"EPS payload"[..]));
        assert!(super::inflate(&packed, 10).is_none());
        assert_eq!(super::inflate(&packed, u64::MAX).as_deref(), Some(&b"EPS payload"[..]));
        assert!(super::inflate(&packed[..packed.len() - 1], 11).is_none());
    }

    #[test]
    fn ascii85_first_line_never_starts_with_percent() {
        let data = [0x0C, 0x80, 0, 0, 1, 2, 3, 4];
        let s = ascii85(&data);
        assert!(s.lines().all(|l| !l.starts_with('%')), "got {s:?}");
        assert_eq!(ascii85_decode(&s).as_deref(), Some(&data[..]));
    }

    #[test]
    fn huge_finite_numbers_are_valid_postscript() {
        for v in [1.5e305, -1.5e305, f64::MAX, f64::MIN] {
            let s = num(v);
            assert!(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.'), "{v}: got {s:?}");
        }
    }

    #[test]
    fn numbers_keep_the_decimals_asked_for() {
        let places = |v: f64, n: u32| {
            let mut s = String::new();
            push_num_places(&mut s, v, n);
            s
        };
        assert_eq!(num(0.123_456_78), "0.1235");
        assert_eq!(places(0.123_456_78, 7), "0.1234568");
        assert_eq!(places(f64::from(0.4f32), 7), "0.4");
        assert_eq!(places(2.5, 7), "2.5");
        assert_eq!(places(f64::NAN, 7), "0");
    }
}
