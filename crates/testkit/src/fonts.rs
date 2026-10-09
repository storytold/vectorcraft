//! Font files for tests, made from the bundled fonts when a test runs: no font file is committed.

/// The bundled Source Sans 3 Regular with its family renamed `family` in every record of its name
/// table, UTF-16 and ASCII alike. `family` must be as long as "Source Sans 3" (13 bytes). The
/// PostScript name stays `SourceSans3-Regular`.
pub fn renamed(family: &str) -> Vec<u8> {
    const ORIGINAL: &str = "Source Sans 3";
    assert_eq!(family.len(), ORIGINAL.len(), "`{family}` must be 13 bytes long");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/SourceSans3-Regular.ttf");
    let mut data = std::fs::read(path).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16(ORIGINAL), utf16(family)), (ORIGINAL.as_bytes().to_vec(), family.as_bytes().to_vec())] {
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}
