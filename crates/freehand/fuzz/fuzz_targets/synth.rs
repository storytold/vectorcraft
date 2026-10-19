#![no_main]

use libfuzzer_sys::fuzz_target;

// The fuzzer picks a synthetic FreeHand 8–11 file, then changes counts, ids, matrices and
// the truncation point. The reader has to return, and stay inside its own limits.
// Every input also reads the 63-level shared-record shape (group A_k names the list
// [A_{k-1}, A_{k-1}]) under a tightened work budget, so that expansion cannot run away.
fuzz_target!(|bytes: &[u8]| {
    let shared = vectorcraft_freehand::synth::shared_groups(63);
    let tight = vectorcraft_freehand::Limits { max_items: 4_096, ..vectorcraft_freehand::Limits::default() };
    let _ = vectorcraft_freehand::is_freehand(&shared);
    let _ = vectorcraft_freehand::read(&shared, tight);
    let file = vectorcraft_freehand::synth::fuzz_file(bytes);
    let _ = vectorcraft_freehand::is_freehand(&file);
    let _ = vectorcraft_freehand::read(&file, vectorcraft_freehand::Limits::default());
});
