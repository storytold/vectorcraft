//! `readhexstring` and `readline` (PLRM 3rd ed., §8.2) read the data left in a file in one pass.
//! The tests run the one-pass readers next to reference readers that take one byte per call, on the
//! same data in the same interpreter state: the program's own file after the whitespace that ends
//! the last token, a decoded buffer read from its start or part way in, and a filter decoded when it
//! is first read. The bytes, the result or error, and every byte left unread must agree.

use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;
use std::time::{Duration, Instant};

use vectorcraft_doc::Document;
use vectorcraft_geom::{Affine, Rect};

use super::data::Stream;
use super::graphics::{GState, Out};
use super::interp::Interp;
use super::obj::{Obj, Res};

type File = Rc<RefCell<Stream>>;

/// Bytes as hexadecimal digits.
fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// An interpreter that has run `src` up to its `quit`, and the file object the program left on the
/// operand stack. What follows `quit` in `src` is still unread.
fn file_after(src: &[u8]) -> (Interp<'_>, File) {
    let mut doc = Document::new(100.0, 100.0);
    doc.layers.clear();
    let page = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 100.0]);
    let mut it = Interp::new(src, GState::default(), Out::new(doc, page, Rect::new(0.0, 0.0, 100.0, 100.0)));
    assert_eq!(it.run(), Ok(()), "{}", String::from_utf8_lossy(src));
    let Ok(Obj::File { stream, .. }) = it.pop() else { panic!("no file on the stack: {}", String::from_utf8_lossy(src)) };
    (it, stream)
}

/// The reference `readline`: one byte per call, a CR followed by an LF ending the line once.
fn line_by_bytes(it: &mut Interp, f: &File, n: usize) -> Res<(Vec<u8>, bool)> {
    let mut line = vec![];
    let mut ended = false;
    while line.len() < n {
        match it.read(f, 1)?.first() {
            Some(b'\n') | None => {
                ended = true;
                break;
            }
            Some(b'\r') => {
                match &mut *f.borrow_mut() {
                    Stream::Current => {
                        if it.lex.rest().first() == Some(&b'\n') {
                            it.lex.advance(1);
                        }
                    }
                    Stream::Buf { data, pos, .. } => {
                        if data.get(*pos) == Some(&b'\n') {
                            *pos += 1;
                        }
                    }
                    Stream::Pending { .. } => {}
                }
                ended = true;
                break;
            }
            Some(b) => line.push(*b),
        }
    }
    Ok((line, ended))
}

/// The reference `readhexstring`: one byte per call, characters that are not hexadecimal digits
/// skipped.
fn hex_by_bytes(it: &mut Interp, f: &File, n: usize) -> Res<Vec<u8>> {
    let mut out = Vec::with_capacity(n);
    let mut high: Option<u8> = None;
    while out.len() < n {
        let Some(&b) = it.read(f, 1)?.first() else { break };
        let Some(v) = char::from(b).to_digit(16) else { continue };
        match high.take() {
            Some(h) => out.push(h << 4 | v as u8),
            None => high = Some(v as u8),
        }
    }
    Ok(out)
}

fn line_one_pass(it: &mut Interp, f: &File, n: usize) -> Res<(Vec<u8>, bool)> {
    it.read_line(f, n)
}

fn hex_one_pass(it: &mut Interp, f: &File, n: usize) -> Res<Vec<u8>> {
    it.read_hex(f, n)
}

/// Hexadecimal data as CID `StartData` reads it: the digits, then a closing `>` if one is next.
fn hex_and_end_one_pass(it: &mut Interp, f: &File, n: usize) -> Res<Vec<u8>> {
    let d = it.read_hex(f, n)?;
    it.skip_hex_end(f)?;
    Ok(d)
}

fn hex_and_end_by_bytes(it: &mut Interp, f: &File, n: usize) -> Res<Vec<u8>> {
    let d = hex_by_bytes(it, f, n)?;
    it.skip_hex_end(f)?;
    Ok(d)
}

/// How the program reaches the data.
#[derive(Clone, Copy, Debug)]
enum Form {
    /// `currentfile`, with this whitespace after the token before the data.
    Current(&'static [u8]),
    /// A buffer decoded at once, with this many bytes of it read before the data.
    Buf(usize),
    /// An `ASCIIHexDecode` filter over `currentfile`, decoded when it is first read, with this
    /// whitespace after the token before the digits.
    Pending(&'static [u8]),
}

const FORMS: [Form; 13] = [
    Form::Current(b" "),
    Form::Current(b"\n"),
    Form::Current(b"\r"),
    Form::Current(b"\r\n"),
    Form::Current(b"\r\r\n"),
    Form::Current(b"\n\r"),
    Form::Current(b"\t\t"),
    Form::Buf(0),
    Form::Buf(2),
    Form::Buf(3),
    Form::Pending(b"\n"),
    Form::Pending(b"\r\n"),
    Form::Pending(b" "),
];

/// A program that leaves a file over `data` on the operand stack, in form `form`.
fn program(form: Form, data: &[u8]) -> Vec<u8> {
    match form {
        Form::Current(lead) => [b"currentfile quit".as_slice(), lead, data].concat(),
        Form::Buf(0) => format!("<{}> /ReusableStreamDecode filter quit", hex(data)).into_bytes(),
        Form::Buf(skip) => {
            let before = b"\r\n\r".get(..skip).unwrap_or(b"\r\n\r".as_slice());
            format!("<{}{}> /ReusableStreamDecode filter dup {skip} string readstring pop pop quit", hex(before), hex(data)).into_bytes()
        }
        Form::Pending(lead) => [b"currentfile /ASCIIHexDecode filter quit".as_slice(), lead, hex(data).as_bytes(), b"> tail"].concat(),
    }
}

type Reader<T> = fn(&mut Interp, &File, usize) -> Res<T>;

/// Read `data` in `form` with `one_pass` and with `by_bytes` in two interpreters in the same state,
/// and check that they agree on the result and on everything left to read.
fn agree<T: PartialEq + Debug>(form: Form, data: &[u8], n: usize, one_pass: Reader<T>, by_bytes: Reader<T>) {
    let src = program(form, data);
    let (mut a, fa) = file_after(&src);
    let (mut b, fb) = file_after(&src);
    let at = || format!("{form:?}, {:?}, n = {n}", String::from_utf8_lossy(data));
    assert_eq!(one_pass(&mut a, &fa, n), by_bytes(&mut b, &fb, n), "result: {}", at());
    assert_eq!(a.lex.rest(), b.lex.rest(), "program left unread: {}", at());
    assert_eq!(a.read(&fa, usize::MAX), b.read(&fb, usize::MAX), "file left unread: {}", at());
    assert_eq!(a.lex.rest(), b.lex.rest(), "program left after the file: {}", at());
    a.release();
    b.release();
}

/// A fixed sequence of pseudo-random numbers (xorshift), so a failure repeats.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
}

/// `chosen`, then `count` strings of up to 24 bytes from `alphabet`.
fn samples(chosen: &[&[u8]], alphabet: &[u8], count: usize, rng: &mut Rng) -> Vec<Vec<u8>> {
    let random = (0..count).map(|_| {
        let len = rng.below(25);
        (0..len).filter_map(|_| alphabet.get(rng.below(alphabet.len())).copied()).collect()
    });
    chosen.iter().map(|s| s.to_vec()).chain(random).collect()
}

/// String lengths to read `data` with: none, one, past its end, exactly its length and one more,
/// and one at random.
fn lengths(data: &[u8], rng: &mut Rng) -> [usize; 6] {
    [0, 1, 3, data.len(), data.len() + 1, rng.below(data.len() + 3)]
}

#[test]
fn readline_reads_in_one_pass_as_byte_by_byte() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let chosen: [&[u8]; 16] = [
        b"",
        b"\n",
        b"\r",
        b"\r\n",
        b"\n\r",
        b"\r\r\n",
        b"abc",
        b"abc\n",
        b"abc\r",
        b"abc\rX",
        b"abc\r\nX",
        b"abc\n\nX",
        b"abcdefgh\r\nX",
        b"abcdefgh\nX",
        b"a\0b\xff\rX",
        b"abc\r\n",
    ];
    for data in samples(&chosen, b"ab\r\n\r\n x\0\xff", 40, &mut rng) {
        for form in FORMS {
            for n in lengths(&data, &mut rng) {
                agree(form, &data, n, line_one_pass, line_by_bytes);
            }
        }
    }
}

#[test]
fn readhexstring_reads_in_one_pass_as_byte_by_byte() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let chosen: [&[u8]; 14] = [
        b"",
        b"0",
        b"a",
        b"0a",
        b"0a1",
        b"0 a\r\n1b>X",
        b"zz0g1",
        b"ff\r\n00\nAA",
        b"0a1b2c3d",
        b"0a1b\r\n2c>",
        b"\r\n0a",
        b"a b c d e",
        b"0a>",
        b"0a \r\n >X",
    ];
    for data in samples(&chosen, b"0123456789abcdefABCDEFg \r\n\t>z%", 40, &mut rng) {
        for form in FORMS {
            for n in lengths(&data, &mut rng) {
                agree(form, &data, n, hex_one_pass, hex_by_bytes);
                agree(form, &data, n, hex_and_end_one_pass, hex_and_end_by_bytes);
            }
        }
    }
}

/// `readline` of `data` into `n` bytes, after `currentfile` and a CR LF, gives `line` and `ended`
/// and leaves `left` unread.
fn line_case(data: &[u8], n: usize, line: &[u8], ended: bool, left: &[u8]) {
    let src = program(Form::Current(b"\r\n"), data);
    let (mut it, f) = file_after(&src);
    assert_eq!(it.read_line(&f, n), Ok((line.to_vec(), ended)), "{:?}", String::from_utf8_lossy(data));
    assert_eq!(it.read(&f, usize::MAX), Ok(left.to_vec()), "{:?}", String::from_utf8_lossy(data));
    it.release();
}

/// `readhexstring` of `data` into `n` bytes, after `currentfile` and a CR LF, gives `bytes` and
/// leaves `left` unread in the program.
fn hex_case(data: &[u8], n: usize, bytes: &[u8], left: &[u8]) {
    let src = program(Form::Current(b"\r\n"), data);
    let (mut it, f) = file_after(&src);
    assert_eq!(it.read_hex(&f, n), Ok(bytes.to_vec()), "{:?}", String::from_utf8_lossy(data));
    assert_eq!(it.lex.rest(), left, "{:?}", String::from_utf8_lossy(data));
    it.release();
}

/// The results the one-pass readers give at the edges of the data, stated directly.
#[test]
fn one_pass_reads_stop_where_their_data_ends() {
    // A CR LF ends one line; a lone CR leaves the next byte unread.
    line_case(b"ab\r\nX", 8, b"ab", true, b"X");
    line_case(b"ab\rX", 8, b"ab", true, b"X");
    line_case(b"ab\nX", 8, b"ab", true, b"X");
    // A line that fills the string leaves its line ending unread.
    line_case(b"abcd\nX", 4, b"abcd", false, b"\nX");
    line_case(b"abc\r\nX", 3, b"abc", false, b"\r\nX");
    // The end of the data ends the line.
    line_case(b"ab", 8, b"ab", true, b"");
    line_case(b"", 8, b"", true, b"");
    // The read stops on the digit that completes the last byte.
    hex_case(b"0a\r\n1b>X", 2, &[0x0a, 0x1b], b">X");
    // Characters that are not digits are skipped.
    hex_case(b"z0 g\ta%", 1, &[0x0a], b"%");
    // A lone digit at the end of the data gives no byte.
    hex_case(b"0a1", 2, &[0x0a], b"");
    hex_case(b"\r\n", 2, &[], b"");
    // No byte to read takes nothing, not even the whitespace after the token.
    hex_case(b"0a", 0, &[], b"\r\n0a");
}

/// The median of `runs`.
fn median(runs: &[Duration]) -> Duration {
    let mut sorted = runs.to_vec();
    sorted.sort();
    sorted.get(sorted.len() / 2).copied().unwrap_or_default()
}

/// Read all of `src`'s data with `reader` in strings of `n` bytes, `calls` times, and how long it
/// took. The interpreter is made before the clock starts.
fn timed<T>(src: &[u8], reader: Reader<T>, n: usize, calls: usize) -> (Vec<T>, Duration) {
    let (mut it, f) = file_after(src);
    let start = Instant::now();
    let got: Vec<T> = (0..calls).map(|_| reader(&mut it, &f, n).unwrap_or_else(|e| panic!("{e:?}"))).collect();
    let took = start.elapsed();
    it.release();
    (got, took)
}

/// Time both readers on `src` in 7 runs, alternating which goes first, check that they read the
/// same, and print every run and the medians.
fn compare<T: PartialEq>(what: &str, src: &[u8], one_pass: Reader<T>, by_bytes: Reader<T>, n: usize, calls: usize) {
    let (mut old, mut new) = (vec![], vec![]);
    for run in 0..7 {
        let order: [bool; 2] = if run % 2 == 0 { [false, true] } else { [true, false] };
        let mut got = vec![];
        for one in order {
            let (read, took) = timed(src, if one { one_pass } else { by_bytes }, n, calls);
            if one {
                new.push(took)
            } else {
                old.push(took)
            }
            got.push(read);
        }
        assert!(got.first() == got.get(1), "{what}: the readers differ");
    }
    let ms = |d: &Duration| format!("{:.1}", d.as_secs_f64() * 1000.0);
    println!("{what}: {} bytes of data, {calls} calls of {n} bytes", src.len());
    println!("  byte by byte, ms: {}", old.iter().map(ms).collect::<Vec<_>>().join(" "));
    println!("  one pass, ms:     {}", new.iter().map(ms).collect::<Vec<_>>().join(" "));
    let (o, w) = (median(&old), median(&new));
    println!("  median: byte by byte {} ms, one pass {} ms, {:.1}x", ms(&o), ms(&w), o.as_secs_f64() / w.as_secs_f64().max(1e-9));
}

/// Times the one-pass readers against the byte-at-a-time reference readers on the same data in one
/// process: the hexadecimal samples of a 1000 × 1000 RGB image in lines of 64 digits, read a row
/// of 3,000 bytes at a time, and 200,000 lines of 40 bytes read by `readline` into an 80-byte
/// string. Run it in release:
/// `cargo test --release -p vectorcraft-eps --lib tests_reads::one_pass_reads_timed -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn one_pass_reads_timed() {
    let rgb: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    let digits = hex(&rgb);
    let mut data = Vec::with_capacity(digits.len() + digits.len() / 64 + 1);
    for line in digits.as_bytes().chunks(64) {
        data.extend_from_slice(line);
        data.push(b'\n');
    }
    compare("readhexstring", &program(Form::Current(b"\n"), &data), hex_one_pass, hex_by_bytes, 3_000, 1_000);
    let lines: Vec<u8> = (0..200_000).flat_map(|i| format!("{:<39}\n", format!("line {i}")).into_bytes()).collect();
    compare("readline", &program(Form::Current(b"\n"), &lines), line_one_pass, line_by_bytes, 80, 200_000);
}
