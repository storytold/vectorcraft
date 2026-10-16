#![no_main]

use libfuzzer_sys::fuzz_target;

// Arbitrary bytes through the sniff and the whole reader, at the default safety limits.
fuzz_target!(|bytes: &[u8]| {
    let _ = vectorcraft_freehand::is_freehand(bytes);
    let _ = vectorcraft_freehand::read(bytes, vectorcraft_freehand::Limits::default());
});
