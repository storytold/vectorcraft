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

/// A resource fork holding `fonts` as `sfnt` resources and one other resource (a `vers`), as a font
/// utility writes a suitcase font's fork, and as vectorcraft-text's suitcase tests build one. macOS
/// keeps a file's resource fork at `<file>/..namedfork/rsrc`.
pub fn suitcase_fork(fonts: &[Vec<u8>]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    for body in fonts.iter().map(Vec::as_slice).chain([&b"1.0"[..]]) {
        offsets.push(data.len() as u32);
        data.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(body);
    }
    let n = fonts.len();
    // The type list: `vers` (1 resource) then `sfnt`, each entry pointing at its references.
    let list = 2 + 2 * 8;
    let mut types = Vec::new();
    types.extend_from_slice(&1u16.to_be_bytes());
    types.extend_from_slice(b"vers");
    types.extend_from_slice(&0u16.to_be_bytes());
    types.extend_from_slice(&(list as u16).to_be_bytes());
    types.extend_from_slice(b"sfnt");
    types.extend_from_slice(&((n - 1) as u16).to_be_bytes());
    types.extend_from_slice(&((list + 12) as u16).to_be_bytes());
    let reference = |id: i16, offset: u32| {
        let mut r = Vec::new();
        r.extend_from_slice(&id.to_be_bytes());
        r.extend_from_slice(&0xFFFFu16.to_be_bytes());
        r.extend_from_slice(&offset.to_be_bytes());
        r.extend_from_slice(&[0; 4]);
        r
    };
    types.extend(reference(1, offsets[n]));
    for (i, o) in offsets[..n].iter().enumerate() {
        types.extend(reference(5000 + i as i16, *o));
    }
    // The map: a copy of the header, a handle, attributes, then the offsets of the lists.
    let mut map = vec![0; 24];
    map.extend_from_slice(&28u16.to_be_bytes());
    map.extend_from_slice(&((28 + types.len()) as u16).to_be_bytes());
    map.extend(types);
    let mut fork = Vec::new();
    for v in [256u32, 256 + data.len() as u32, data.len() as u32, map.len() as u32] {
        fork.extend_from_slice(&v.to_be_bytes());
    }
    fork.resize(256, 0);
    fork.extend(data);
    fork.extend(map);
    fork
}
