//! Minimal plug-ins in WebAssembly text (WAT): the skeleton `docs/plugins.md` describes, which the
//! tests turn into modules (with the `wat` crate) to exercise the host.

/// Where [`returning`] keeps its output in memory (the manifest sits at 16).
const OUTPUT_AT: usize = 0x1_0000;

/// `s` as the body of a WAT string literal (every byte that isn't a letter, digit or space
/// escaped).
pub fn string(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b' ' { char::from(b).to_string() } else { format!("\\{b:02x}") }).collect()
}

/// A plug-in module: `manifest` (JSON), a `vc_run` whose body is `run` (parameters `$in $il $pp
/// $pl`, result i64), extra module fields `extra` and `pages` of initial memory. `vc_alloc` grows
/// memory by whole pages, so its blocks never overlap data segments.
pub fn module(manifest: &str, run: &str, extra: &str, pages: u32) -> String {
    format!(
        r#"(module
  {extra}
  (memory (export "memory") {pages})
  (data (i32.const 16) "{data}")
  (func (export "vc_abi_version") (result i32) (i32.const 1))
  (func (export "vc_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 16)))
  (func (export "vc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1)) (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "vc_run") (param $in i32) (param $il i32) (param $pp i32) (param $pl i32) (result i64)
    {run}))"#,
        data = string(manifest),
        len = manifest.len(),
    )
}

/// A `vc_run` body returning the input unchanged.
pub const ECHO: &str = "(i64.or (i64.shl (i64.extend_i32_u (local.get $il)) (i64.const 32)) (i64.extend_i32_u (local.get $in)))";

/// A plug-in that always returns `output` (whatever it is given).
pub fn returning(manifest: &str, output: &str) -> String {
    then_returning(manifest, "", output)
}

/// A plug-in whose `vc_run` runs `first` (instructions, with the locals it declares first), then
/// returns `output`.
fn then_returning(manifest: &str, first: &str, output: &str) -> String {
    let run = format!("{first} (i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const {OUTPUT_AT}))", output.len());
    let extra = format!(r#"(data (i32.const {OUTPUT_AT}) "{}")"#, string(output));
    module(manifest, &run, &extra, (OUTPUT_AT + output.len()).div_ceil(0x1_0000) as u32 + 1)
}

/// A plug-in that counts `steps` down before it returns `output`. Each step costs a few
/// instructions, so a test picks work that a light budget can't finish and a heavy one can, or
/// that runs until it is cancelled.
pub fn spinning(manifest: &str, steps: u64, output: &str) -> String {
    let spin = format!(
        "(local $n i64) (local.set $n (i64.const {steps}))
    (block $done (loop $step
      (br_if $done (i64.le_s (local.get $n) (i64.const 0)))
      (local.set $n (i64.sub (local.get $n) (i64.const 1)))
      (br $step)))"
    );
    then_returning(manifest, &spin, output)
}

/// What [`when_given`] looks through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Given {
    /// The input JSON (the objects).
    Input,
    /// The parameters JSON, `_context` included.
    Params,
}

/// Where [`when_given`] keeps the text it looks for.
const NEEDLE_AT: usize = 0x8000;

/// A plug-in that returns `output` when what it is given (`given`) holds the text `needle`, and
/// its input unchanged otherwise. A live effect made from it draws one shape or the other, so a
/// test sees from the geometry whether the host handed the plug-in something.
pub fn when_given(manifest: &str, given: Given, needle: &str, output: &str) -> String {
    let (src, len) = match given {
        Given::Input => ("$in", "$il"),
        Given::Params => ("$pp", "$pl"),
    };
    let n = needle.len();
    // For each place `$i` the needle fits, compare it byte by byte (`$j`).
    let run = format!(
        r#"(local $i i32) (local $j i32)
    (block $found
      (block $none
        (loop $at
          (br_if $none (i32.gt_u (i32.add (local.get $i) (i32.const {n})) (local.get {len})))
          (local.set $j (i32.const 0))
          (block $next
            (loop $cmp
              (br_if $found (i32.eq (local.get $j) (i32.const {n})))
              (br_if $next (i32.ne (i32.load8_u (i32.add (local.get {src}) (i32.add (local.get $i) (local.get $j))))
                                   (i32.load8_u (i32.add (i32.const {NEEDLE_AT}) (local.get $j)))))
              (local.set $j (i32.add (local.get $j) (i32.const 1)))
              (br $cmp)))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $at)))
      (return {ECHO}))
    (i64.or (i64.shl (i64.const {out}) (i64.const 32)) (i64.const {OUTPUT_AT}))"#,
        out = output.len(),
    );
    let extra = format!(r#"(data (i32.const {NEEDLE_AT}) "{}") (data (i32.const {OUTPUT_AT}) "{}")"#, string(needle), string(output));
    module(manifest, &run, &extra, (OUTPUT_AT + output.len()).div_ceil(0x1_0000) as u32 + 1)
}

/// The manifest of a test plug-in `id` of `kind` with `params` (a JSON object of specs).
pub fn manifest(id: &str, kind: &str, params: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"{kind}","params":{params}}}"#)
}

/// The manifest of a test exporter `id` writing `extensions` (a JSON list) with `params`.
pub fn exporter(id: &str, extensions: &str, params: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"exporter","extensions":{extensions},"params":{params}}}"#)
}

/// What [`echo_exporter`] writes as its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Echo {
    /// The input: the document, as the host hands it over.
    Input,
    /// The parameters JSON, `_context` included.
    Params,
}

/// Where [`echo_exporter`] keeps the base64 alphabet, then its output's opening and closing text.
const ALPHABET_AT: usize = 0x8000;

/// An exporter (manifest `manifest`) whose file is exactly what it is given (`echo`), so a test
/// reads back what the host hands a plug-in: it returns `{"dataBase64": "…"}`, encoding the bytes
/// in base64 itself, three bytes to four characters, the last group padded with `=`.
pub fn echo_exporter(manifest: &str, echo: Echo) -> String {
    let (src, len) = match echo {
        Echo::Input => ("$in", "$il"),
        Echo::Params => ("$pp", "$pl"),
    };
    let (open, close) = (r#"{"dataBase64":""#, r#""}"#);
    let (open_at, close_at) = (ALPHABET_AT + 64, ALPHABET_AT + 96);
    // One character of the alphabet: the six bits of `$b` from `shift` up.
    let char_at = |shift: u32| {
        format!("(i32.load8_u (i32.add (i32.const {ALPHABET_AT}) (i32.and (i32.shr_u (local.get $b) (i32.const {shift})) (i32.const 63))))")
    };
    let byte = |k: u32| format!("(i32.load8_u (i32.add (local.get {src}) (i32.add (local.get $i) (i32.const {k}))))");
    let run = format!(
        r#"(local $o i32) (local $n i32) (local $w i32) (local $i i32) (local $b i32) (local $rest i32)
    (local.set $n (i32.add (i32.const {frame}) (i32.mul (i32.const 4) (i32.div_u (i32.add (local.get {len}) (i32.const 2)) (i32.const 3)))))
    (local.set $o (memory.grow (i32.add (i32.shr_u (local.get $n) (i32.const 16)) (i32.const 1))))
    (if (i32.eq (local.get $o) (i32.const -1)) (then (return (i64.const -1))))
    (local.set $o (i32.mul (local.get $o) (i32.const 65536)))
    (memory.copy (local.get $o) (i32.const {open_at}) (i32.const {open_len}))
    (local.set $w (i32.add (local.get $o) (i32.const {open_len})))
    (block $done (loop $triple
      (br_if $done (i32.gt_u (i32.add (local.get $i) (i32.const 3)) (local.get {len})))
      (local.set $b (i32.or (i32.or (i32.shl {b0} (i32.const 16)) (i32.shl {b1} (i32.const 8))) {b2}))
      (i32.store8 (local.get $w) {c18}) (i32.store8 offset=1 (local.get $w) {c12})
      (i32.store8 offset=2 (local.get $w) {c6}) (i32.store8 offset=3 (local.get $w) {c0})
      (local.set $w (i32.add (local.get $w) (i32.const 4)))
      (local.set $i (i32.add (local.get $i) (i32.const 3)))
      (br $triple)))
    (local.set $rest (i32.sub (local.get {len}) (local.get $i)))
    (if (i32.ne (local.get $rest) (i32.const 0)) (then
      (local.set $b (i32.shl {b0} (i32.const 16)))
      (if (i32.eq (local.get $rest) (i32.const 2)) (then (local.set $b (i32.or (local.get $b) (i32.shl {b1} (i32.const 8))))))
      (i32.store8 (local.get $w) {c18}) (i32.store8 offset=1 (local.get $w) {c12})
      (i32.store8 offset=2 (local.get $w) (if (result i32) (i32.eq (local.get $rest) (i32.const 2)) (then {c6}) (else (i32.const 61))))
      (i32.store8 offset=3 (local.get $w) (i32.const 61))
      (local.set $w (i32.add (local.get $w) (i32.const 4)))))
    (memory.copy (local.get $w) (i32.const {close_at}) (i32.const {close_len}))
    (i64.or (i64.shl (i64.extend_i32_u (local.get $n)) (i64.const 32)) (i64.extend_i32_u (local.get $o)))"#,
        frame = open.len() + close.len(),
        open_len = open.len(),
        close_len = close.len(),
        b0 = byte(0),
        b1 = byte(1),
        b2 = byte(2),
        c18 = char_at(18),
        c12 = char_at(12),
        c6 = char_at(6),
        c0 = char_at(0),
    );
    let extra = format!(
        r#"(data (i32.const {ALPHABET_AT}) "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/")
  (data (i32.const {open_at}) "{}")
  (data (i32.const {close_at}) "{}")"#,
        string(open),
        string(close)
    );
    module(manifest, &run, &extra, 1)
}
