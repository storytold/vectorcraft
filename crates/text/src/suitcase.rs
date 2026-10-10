//! Suitcase fonts: classic Mac OS font files whose data fork is empty and whose fonts are `sfnt`
//! resources in the resource fork (Microsoft Office installs its fonts this way in
//! `/Library/Fonts/Microsoft`). macOS lists them as installed; a reader of the file's data finds
//! nothing, so the scan reads the resource fork. The fork's layout is Apple's published resource
//! file format (Inside Macintosh: More Macintosh Toolbox, "Resource Manager").

use std::path::Path;

/// Caps on what a (possibly damaged) fork can make the scan read.
const MAX_FORK: u64 = 256 << 20;
const MAX_TYPES: usize = 1 << 12;
const MAX_FONTS: usize = 256;

fn be16(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at.checked_add(2)?).and_then(|s| s.try_into().ok()).map(|s| u16::from_be_bytes(s) as usize)
}

fn be32(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at.checked_add(4)?).and_then(|s| s.try_into().ok()).map(|s| u32::from_be_bytes(s) as usize)
}

/// The font files (`sfnt` resources) of a resource fork, in the order its map lists them. A
/// fork that isn't well formed gives the fonts read before the damage, or none.
pub(crate) fn sfnt_resources(fork: &[u8]) -> Vec<&[u8]> {
    let mut fonts = Vec::new();
    let _ = collect(fork, &mut fonts);
    fonts
}

fn collect<'a>(fork: &'a [u8], fonts: &mut Vec<&'a [u8]>) -> Option<()> {
    // The header: where the data and the map start.
    let data = be32(fork, 0)?;
    let map = fork.get(be32(fork, 4)?..)?;
    // The map: its type list starts at the offset kept 24 bytes in, and counts from the map.
    let types = be16(map, 24)?;
    // Counts are stored less one, so a list of none is 0xFFFF.
    let n_types = (be16(map, types)? + 1) & 0xFFFF;
    let mut total = 0usize;
    for t in 0..n_types.min(MAX_TYPES) {
        let at = types + 2 + t * 8;
        if map.get(at..at + 4)? != b"sfnt" {
            continue;
        }
        let count = (be16(map, at + 4)? + 1) & 0xFFFF;
        // Each reference: its data's offset is the low three bytes of the word 4 bytes in.
        let refs = types + be16(map, at + 6)?;
        for r in 0..count {
            let start = data.checked_add(be32(map, refs + r * 12 + 4)? & 0x00FF_FFFF)?;
            let len = be32(fork, start)?;
            // References can share data: cap the total too, as every font is copied out.
            total = total.checked_add(len)?;
            if fonts.len() >= MAX_FONTS || total as u64 > MAX_FORK {
                return Some(());
            }
            fonts.push(fork.get(start + 4..start.checked_add(4)?.checked_add(len)?)?);
        }
    }
    Some(())
}

/// The fonts of the suitcase font at `path`, read from its resource fork; none for a file
/// without one, and off macOS, the only place such a fork is a part of a file.
pub(crate) fn read_fonts(path: &Path) -> Vec<Vec<u8>> {
    let Some(fork) = read_fork(path) else { return vec![] };
    sfnt_resources(&fork).into_iter().map(<[u8]>::to_vec).collect()
}

/// Whether the file at `path` is a suitcase font: nothing in its data fork and something in its
/// resource fork.
pub(crate) fn is_suitcase(path: &Path) -> bool {
    // Off macOS no file has a fork: don't look at every font file twice.
    cfg!(target_os = "macos") && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == 0) && fork_len(path) > 0
}

#[cfg(target_os = "macos")]
fn fork_path(path: &Path) -> std::path::PathBuf {
    path.join("..namedfork/rsrc")
}

#[cfg(target_os = "macos")]
pub(crate) fn fork_len(path: &Path) -> u64 {
    std::fs::metadata(fork_path(path)).map_or(0, |m| m.len())
}

#[cfg(target_os = "macos")]
fn read_fork(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut fork = Vec::new();
    std::fs::File::open(fork_path(path)).ok()?.take(MAX_FORK).read_to_end(&mut fork).ok()?;
    Some(fork)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn fork_len(_: &Path) -> u64 {
    0
}

#[cfg(not(target_os = "macos"))]
fn read_fork(_: &Path) -> Option<Vec<u8>> {
    let _ = MAX_FORK;
    None
}

/// A resource fork holding `fonts` as `sfnt` resources and one other resource (a `vers`), as
/// a font utility writes them.
#[cfg(test)]
pub(crate) fn fork_of(fonts: &[Vec<u8>]) -> Vec<u8> {
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
