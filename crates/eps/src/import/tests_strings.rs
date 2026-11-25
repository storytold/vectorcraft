//! `token` on a string, `search` and `anchorsearch` (PLRM 3rd ed., pp. 702-703, 655 and 529) read
//! their strings in place. The tests run them next to reference readers that copy the strings
//! first, on the same operands in the same interpreter state: the operand stack, the error and the
//! operations spent must agree.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use vectorcraft_doc::Document;
use vectorcraft_geom::{Affine, Rect};

use super::graphics::{GState, Out};
use super::interp::Interp;
use super::lex::Lexer;
use super::obj::{Obj, Op, PsError, Res, Shared, VALUES_COPIED};

/// An interpreter that has run `src`, with the operands it left.
fn after(src: &[u8]) -> Interp<'_> {
    let mut doc = Document::new(100.0, 100.0);
    doc.layers.clear();
    let page = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 100.0]);
    let mut it = Interp::new(src, GState::default(), Out::new(doc, page, Rect::new(0.0, 0.0, 100.0, 100.0)));
    assert_eq!(it.run(), Ok(()), "{}", String::from_utf8_lossy(src));
    it
}

/// The reference `token`: the string copied before it is scanned.
fn token_by_copy(it: &mut Interp) -> Res {
    match it.pop()? {
        Obj::Str(s) => {
            let bytes = s.to_vec();
            let mut lex = Lexer::new(&bytes);
            let Some(o) = lex.next()? else { return it.push(Obj::Bool(false)) };
            let o = it.scanned(o, lex.immediate)?;
            let used = bytes.len() - lex.data().len();
            it.push_interval(Obj::Str(s), used, bytes.len() - used)?;
            it.push(o)?;
        }
        Obj::File { stream: f, .. } => {
            let (o, immediate) = it.next_in(&f)?;
            let Some(o) = o else { return it.push(Obj::Bool(false)) };
            let o = it.scanned(o, immediate)?;
            it.push(o)?;
        }
        _ => return Err(PsError::Ps("typecheck", "token".into())),
    }
    it.push(Obj::Bool(true))
}

/// The reference `search` and `anchorsearch`: both strings copied before they are compared.
fn search_by_copy(it: &mut Interp, op: Op) -> Res {
    let seek = it.pop_str()?.to_vec();
    let s = it.pop_str()?;
    let text = s.to_vec();
    it.spend((text.len() / 64) as u64)?;
    let at = if op == Op::Search { super::lex::find(&text, &seek) } else { text.starts_with(&seek).then_some(0) };
    match at {
        Some(i) => {
            let o = Obj::Str(s);
            if op == Op::Search {
                it.push_interval(o.clone(), i + seek.len(), text.len() - i - seek.len())?;
                it.push_interval(o.clone(), i, seek.len())?;
                it.push_interval(o, 0, i)?;
            } else {
                it.push_interval(o.clone(), seek.len(), text.len() - seek.len())?;
                it.push_interval(o, 0, seek.len())?;
            }
            it.push(Obj::Bool(true))?;
        }
        None => {
            it.push(Obj::Str(s))?;
            it.push(Obj::Bool(false))?;
        }
    }
    Ok(())
}

fn by_copy(it: &mut Interp, op: Op) -> Res {
    if op == Op::Token { token_by_copy(it) } else { search_by_copy(it, op) }
}

/// The error's name, or the limit reached.
fn outcome(r: Res) -> Result<(), String> {
    r.map_err(|e| match e {
        PsError::Ps(name, _) => name.to_string(),
        e => format!("{e:?}"),
    })
}

/// Everything the operator left: the operand stack with each string's storage, offset and length,
/// and the operations spent.
fn state(it: &Interp) -> (String, u64) {
    (format!("{:?}", it.stack), it.ops_left())
}

/// Run `op` on the operands `src` leaves, in place and by copy, `steps` times (a walk that feeds
/// each call the string the last one left), and check that the two agree after every call.
fn agree(src: &str, op: Op, steps: usize) {
    let (mut new, mut old) = (after(src.as_bytes()), after(src.as_bytes()));
    // `search` and `anchorsearch` look for the same string on each call.
    let seeks = [new.stack.last().cloned(), old.stack.last().cloned()];
    for step in 0..steps {
        let (a, b) = (outcome(new.op(op)), outcome(by_copy(&mut old, op)));
        assert_eq!(a, b, "{src} {op:?}, call {step}");
        assert_eq!(state(&new), state(&old), "{src} {op:?}, call {step}");
        // Keep walking while the operator answered true, with the string it left.
        let more = matches!(new.stack.last(), Some(Obj::Bool(true)));
        if a.is_err() || !more {
            break;
        }
        for (it, seek) in [&mut new, &mut old].into_iter().zip(&seeks) {
            it.stack.pop();
            // `token` leaves the rest under the object; `search` leaves it under match and pre
            // (and `anchorsearch` under the match).
            let keep = it.stack.len() - if op == Op::Search { 2 } else { 1 };
            it.stack.truncate(keep);
            // The string to look for again, or the file `token` reads on from.
            if op != Op::Token || matches!(seek, Some(Obj::File { .. })) {
                it.stack.extend(seek.clone());
            }
        }
    }
    new.release();
    old.release();
}

#[test]
fn token_and_search_agree_with_copying_readers() {
    let tokens = [
        "()",
        "(   )",
        "(\r\n\t )",
        "(12)",
        "(12 34)",
        "(12   34)",
        "(12\r\n34)",
        "(/x {a b} (s) <41> 1.5 x)",
        "(1 {)",
        "(  [ 1 ] )",
        "/x 7 def (//x {//x} y)",
        "(xx12 34 56) 2 7 getinterval",
        "(%comment only)",
        "5",
        "(12 /x {a}) /ReusableStreamDecode filter",
        "currentfile",
    ];
    for src in tokens {
        agree(src, Op::Token, 16);
    }
    let searches = [
        "(abcabc) (bc)",
        "(abc) (x)",
        "(abc) ()",
        "() ()",
        "() (a)",
        "(ab) (abc)",
        "(a\nb\nc\n) (\n)",
        "/s (aa) def s s",
        "(xxab cd ef) 2 8 getinterval ( )",
        "(abc) 5",
        "5 (abc)",
    ];
    for src in searches {
        agree(src, Op::Search, 8);
        agree(src, Op::AnchorSearch, 8);
    }
    // Over the operation budget: the same error, with the same operands popped.
    let big = "/h 65536 string def h (\\001)";
    let (mut new, mut old) = (after(big.as_bytes()), after(big.as_bytes()));
    for it in [&mut new, &mut old] {
        let left = it.ops_left();
        assert_eq!(it.spend(left.saturating_sub(10)), Ok(()));
    }
    let (a, b) = (outcome(new.op(Op::Search)), outcome(by_copy(&mut old, Op::Search)));
    assert!(a.as_ref().is_err_and(|e| e.contains("too long")), "{a:?}");
    assert_eq!((a, state(&new)), (b, state(&old)));
    new.release();
    old.release();
}

/// The results share the string's storage: a write to the string after `token` or `search` shows
/// in what they left.
#[test]
fn token_and_search_results_see_later_writes() {
    let check = super::tests::check;
    check("/s (12 34) def s token pop pop /post exch def s 3 57 put post token pop exch pop 94 eq");
    check("/s (ab cd) def s ( ) search pop /pre exch def pop /post exch def s 0 120 put s 4 121 put pre (xb) eq post (cy) eq and");
    check("/s (ab cd) def s (ab) anchorsearch pop pop /post exch def s 4 122 put post ( cz) eq");
    check("/s (xx12 34) def s 2 5 getinterval token pop pop /post exch def s 5 57 put post token pop exch pop 94 eq");
}

/// `token` on a string, `search` and `anchorsearch` copy neither string.
#[test]
fn token_and_search_copy_no_string() {
    for (src, op) in [
        ("(12 34 56 78)", Op::Token),
        ("(abc def ghi) ( )", Op::Search),
        ("(abc def ghi) (xyz)", Op::Search),
        ("(abc def ghi) (abc)", Op::AnchorSearch),
        ("(abc def ghi) (def)", Op::AnchorSearch),
    ] {
        let mut it = after(src.as_bytes());
        VALUES_COPIED.with(|n| n.set(0));
        assert_eq!(it.op(op), Ok(()), "{src} {op:?}");
        assert_eq!(VALUES_COPIED.with(std::cell::Cell::get), 0, "{src} {op:?}");
        it.release();
    }
}

/// The median of `runs`.
fn median(runs: &[Duration]) -> Duration {
    let mut sorted = runs.to_vec();
    sorted.sort();
    sorted.get(sorted.len() / 2).copied().unwrap_or_default()
}

/// What a walk records of each result: a string's length and first bytes, any other object whole.
fn mark(o: &Obj, h: &mut impl Hasher) {
    match o {
        Obj::Str(s) => {
            s.len().hash(h);
            s.borrow().iter().take(16).for_each(|b| b.hash(h));
        }
        o => format!("{o:?}").hash(h),
    }
}

/// Walk `data` with `op` and `seek` (none for `token`) until the operator answers false, by copy
/// or in place: how long it took, the calls made, and a hash of every result and of the operations
/// spent. The interpreter is made before the clock starts.
fn walk(data: &[u8], op: Op, seek: Option<&[u8]>, copy: bool) -> (Duration, usize, u64) {
    let mut it = after(b"");
    it.stack.push(Obj::Str(Shared::new(data.to_vec())));
    let mut calls = 0;
    let mut h = DefaultHasher::new();
    let start = Instant::now();
    loop {
        if let Some(seek) = seek {
            it.stack.push(Obj::Str(Shared::new(seek.to_vec())));
        }
        let r = if copy { by_copy(&mut it, op) } else { it.op(op) };
        assert_eq!(r, Ok(()));
        calls += 1;
        if !matches!(it.stack.pop(), Some(Obj::Bool(true))) {
            break;
        }
        let keep = it.stack.len() - if op == Op::Search { 2 } else { 1 };
        it.stack.iter().for_each(|o| mark(o, &mut h));
        it.stack.truncate(keep);
    }
    let took = start.elapsed();
    it.stack.iter().for_each(|o| mark(o, &mut h));
    it.ops_left().hash(&mut h);
    it.release();
    (took, calls, h.finish())
}

/// Time both readers on `data` in 7 runs, alternating which goes first, check that they leave the
/// same, and print every run and the medians.
fn compare(what: &str, data: &[u8], op: Op, seek: Option<&[u8]>) {
    let (mut old, mut new) = (vec![], vec![]);
    let mut calls = 0;
    for run in 0..7 {
        let order: [bool; 2] = if run % 2 == 0 { [true, false] } else { [false, true] };
        let mut got = vec![];
        for copy in order {
            let (took, n, last) = walk(data, op, seek, copy);
            if copy {
                old.push(took)
            } else {
                new.push(took)
            }
            calls = n;
            got.push((n, last));
        }
        assert!(got.first() == got.get(1), "{what}: the readers differ");
    }
    let ms = |d: &Duration| format!("{:.2}", d.as_secs_f64() * 1000.0);
    println!("{what}: {} bytes, {calls} calls", data.len());
    println!("  by copy, ms:  {}", old.iter().map(ms).collect::<Vec<_>>().join(" "));
    println!("  in place, ms: {}", new.iter().map(ms).collect::<Vec<_>>().join(" "));
    let (o, w) = (median(&old), median(&new));
    println!("  median: by copy {} ms, in place {} ms, {:.1}x", ms(&o), ms(&w), o.as_secs_f64() / w.as_secs_f64().max(1e-9));
}

/// Times `token` and `search` walking a string by what each call leaves, against the copying
/// reference readers, in one process: strings of 4 KiB, 64 KiB and 512 KiB, of numbers and names
/// for `token` and of 128-byte lines for `search`. Run it in release:
/// `cargo test --release -p vectorcraft-eps --lib tests_strings::string_walks_timed -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn string_walks_timed() {
    for kib in [4, 64, 512] {
        let size = kib * 1024;
        let mut words = Vec::with_capacity(size + 16);
        let mut i = 0u32;
        while words.len() < size {
            let w = if i.is_multiple_of(2) { format!("{} ", i.wrapping_mul(2_654_435_761) % 100_000) } else { format!("/n{i} ") };
            words.extend_from_slice(w.as_bytes());
            i += 1;
        }
        compare(&format!("token, {kib} KiB"), &words, Op::Token, None);
        let lines: Vec<u8> = (0..size / 128).flat_map(|i| format!("{:<127}\n", format!("line {i}")).into_bytes()).collect();
        compare(&format!("search, {kib} KiB"), &lines, Op::Search, Some(b"\n"));
    }
}
