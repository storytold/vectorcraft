#![no_main]

use libfuzzer_sys::fuzz_target;

// The fuzzer picks a synthetic FreeHand 8–11 file, then changes counts, ids, matrices and
// the truncation point. The reader has to return, and stay inside its own limits.
fuzz_target!(|bytes: &[u8]| {
    let file = vectorcraft_freehand::synth::fuzz_file(bytes);
    let _ = vectorcraft_freehand::is_freehand(&file);
    let _ = vectorcraft_freehand::read(&file, vectorcraft_freehand::Limits::default());
});
