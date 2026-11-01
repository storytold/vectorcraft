#![no_main]

use libfuzzer_sys::fuzz_target;

// The fuzzer picks a synthetic FreeHand 8–11 file, then changes counts, ids, matrices and
// the truncation point. The reader has to return, and stay inside its own limits.
// Every input also reads three shapes under a tightened budget, so expansion cannot run away:
// the 63-level shared-record chain (group A_k names the list [A_{k-1}, A_{k-1}]), many modest
// paths whose combined geometry is large, and one compound path whose child list repeats a path.
fuzz_target!(|bytes: &[u8]| {
    let tight = vectorcraft_freehand::Limits { max_items: 4_096, max_geometry: 4_096, ..vectorcraft_freehand::Limits::default() };
    let shared = vectorcraft_freehand::synth::shared_groups(63);
    let _ = vectorcraft_freehand::is_freehand(&shared);
    let _ = vectorcraft_freehand::read(&shared, tight);
    let many = vectorcraft_freehand::synth::many_paths(4_000, 8);
    let _ = vectorcraft_freehand::is_freehand(&many);
    let _ = vectorcraft_freehand::read(&many, tight);
    let compound = vectorcraft_freehand::synth::compound_children(4_000, 8);
    let _ = vectorcraft_freehand::is_freehand(&compound);
    let _ = vectorcraft_freehand::read(&compound, tight);
    // Two copies of one compound: the child list parses, and the second walk hits the object budget.
    let repeated = vectorcraft_freehand::synth::repeated_compound(3_000, 8, 2);
    let _ = vectorcraft_freehand::is_freehand(&repeated);
    let _ = vectorcraft_freehand::read(&repeated, tight);
    let file = vectorcraft_freehand::synth::fuzz_file(bytes);
    let _ = vectorcraft_freehand::is_freehand(&file);
    let _ = vectorcraft_freehand::read(&file, vectorcraft_freehand::Limits::default());
});
