use super::build::{style_italic, style_weight};
#[cfg(not(target_arch = "wasm32"))]
use super::dirs::sfnt_of;
use super::face::{MAX_AXES, MAX_INSTANCES};
use super::names::{all_names, name, norm};
use super::traits::{FaceKeys, FaceStyle, FontTraits, face_styles};
use super::{FileRef, MetadataProvider, Path, StringId, Tag};

/// The named instances of a variable font face (its `fvar` table) as styles of the face's family,
/// named by their subfamily names, without the default instance (the face itself) and without
/// names already taken. The default instance's name, when it isn't the face's style name (a face
/// `Regular` whose default instance is `Book`), names the face too (`keys`). Damaged tables give
/// fewer instances or none; the counts read are capped.
pub(super) fn named_instances(
    f: &skrifa::FontRef<'_>,
    family: &str,
    style: &str,
    keys: &mut FaceKeys,
    native_family: Option<String>,
) -> (Vec<FaceStyle>, Option<String>) {
    let axes = f.axes();
    if axes.is_empty() || axes.len() > MAX_AXES {
        return (vec![], None);
    }
    let tags: Vec<Tag> = axes.iter().map(|a| a.tag()).collect();
    let defaults: Vec<f32> = axes.iter().map(|a| a.default_value()).collect();
    let is_default = |coords: &[f32]| coords.len() == defaults.len() && coords.iter().zip(&defaults).all(|(c, d)| (c - d).abs() < 1e-3);
    // The face renders as its default instance. When that instance is another weight than the
    // name table says (`Light` face, Regular default instance), the instance names the face.
    let renamed = tags.iter().any(|t| t.to_be_bytes() == *b"wght").then(|| {
        f.named_instances()
            .iter()
            .take(MAX_INSTANCES)
            .filter(|inst| is_default(&inst.user_coords().take(MAX_AXES).collect::<Vec<f32>>()))
            .find_map(|inst| name(f, &[inst.subfamily_name_id()]))
            .filter(|d| style_weight(d) != style_weight(style))
    });
    let renamed = renamed.flatten();
    // Instances without a PostScript name of their own are named `<prefix>-<style>` (Adobe
    // Technical Note #5902), as PDF and `.ai` files name them.
    let prefix = name(f, &[StringId::VARIATIONS_POSTSCRIPT_NAME_PREFIX]).unwrap_or_else(|| family.to_string());
    if let Some(new) = &renamed {
        let old = norm(style);
        keys.styles.retain(|k| *k != old);
        keys.styles.push(norm(new));
    }
    let mut taken = vec![norm(renamed.as_deref().unwrap_or(style))];
    let mut out = Vec::new();
    for inst in f.named_instances().iter().take(MAX_INSTANCES) {
        let coords: Vec<f32> = inst.user_coords().take(MAX_AXES).collect();
        if coords.len() != tags.len() || coords.iter().any(|c| !c.is_finite()) {
            continue;
        }
        let Some(style) = name(f, &[inst.subfamily_name_id()]) else { continue };
        let ns = norm(&style);
        if ns.is_empty() || taken.contains(&ns) {
            continue;
        }
        // The default instance is the face itself.
        if coords.iter().zip(&defaults).all(|(c, d)| (c - d).abs() < 1e-3) {
            keys.styles.push(ns.clone());
            keys.postscript.push(norm(&format!("{prefix}{style}")));
            taken.push(ns);
            continue;
        }
        taken.push(ns);
        let axis = |tag: &[u8; 4]| tags.iter().zip(&coords).find(|(t, _)| t.to_be_bytes() == *tag).map(|(_, v)| *v);
        let weight = axis(b"wght").map(|w| w.clamp(1.0, 1000.0));
        let italic = style_italic(&style) || axis(b"ital").is_some_and(|v| v >= 0.5) || axis(b"slnt").is_some_and(|v| v != 0.0);
        let mut styles: Vec<String> = std::iter::once(style.clone()).chain(all_names(f, inst.subfamily_name_id())).map(|s| norm(&s)).collect();
        let mut postscript: Vec<String> = inst.postscript_name_id().map(|id| all_names(f, id)).unwrap_or_default().iter().map(|n| norm(n)).collect();
        postscript.push(norm(&format!("{prefix}{style}")));
        for v in [&mut styles, &mut postscript] {
            v.retain(|k| !k.is_empty());
            v.sort();
            v.dedup();
        }
        // Name ID 4 describes the default face, not every named instance of its variable font.
        let keys = FaceKeys { families: keys.families.clone(), styles, legacy: vec![], full_names: vec![], postscript };
        let variations = tags.iter().zip(&coords).map(|(t, v)| (t.to_be_bytes(), *v)).collect();
        out.push(FaceStyle {
            family: family.to_string(),
            native_family: native_family.clone(),
            style,
            keys,
            variations,
            weight,
            italic,
            traits: FontTraits::default(),
        });
    }
    (out, renamed)
}

/// The most faces read from a (possibly damaged) collection.
pub(super) const MAX_FACES: u32 = 256;

/// Parse every face in `data` (a font file or collection) and the named instances of its variable
/// ones. Returns each style with the index of its face.
pub(super) fn enumerate_faces(data: &[u8]) -> Vec<(u32, FaceStyle)> {
    let count = match FileRef::new(data) {
        Ok(FileRef::Font(_)) => 1,
        Ok(FileRef::Collection(c)) => c.len().min(MAX_FACES),
        Err(_) => 0,
    };
    (0..count)
        .filter_map(|i| Some((i, skrifa::FontRef::from_index(data, i).ok()?)))
        .flat_map(|(i, f)| face_styles(&f).into_iter().map(move |s| (i, s)))
        .collect()
}

/// The font files held by the file at `path`: its contents, or for a suitcase font the fonts in
/// its resource fork ([`crate::suitcase`]). None when it can't be read.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn font_files(path: &Path) -> Vec<Vec<u8>> {
    if crate::suitcase::is_suitcase(path) {
        return crate::suitcase::read_fonts(path);
    }
    std::fs::read(path).into_iter().collect()
}

/// The tables holding glyphs the font engine draws: TrueType, CFF, CFF2 and VARC outlines, and
/// `sbix` and `CBDT` colour bitmaps. A face with none of them (outlines in Apple's `hvgl` table,
/// monochrome bitmaps only) draws nothing.
#[cfg(not(target_arch = "wasm32"))]
const OUTLINE_TABLES: [&[u8; 4]; 6] = [b"glyf", b"CFF ", b"CFF2", b"VARC", b"sbix", b"CBDT"];

/// The styles of every face in the font file at `path` ([`face_styles`]), reading only its table
/// directories and `name`, `fvar` and `OS/2` tables: a scan opens hundreds of font files, many of
/// them megabytes long. Faces without outlines the font engine draws (no [`OUTLINE_TABLES`] table)
/// are left out.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn file_face_names(path: &Path) -> Vec<FaceStyle> {
    file_face_names_within(path, MAX_FACES, u64::MAX)
}

/// [`file_face_names`], reading at most `max_faces` faces and `max_read` bytes of the file in all.
/// A suitcase font's resource fork is read whole when it is at most `max_read` bytes long, and the
/// limits then apply to each of its fonts; a longer fork gives none.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_face_names_within(path: &Path, max_faces: u32, max_read: u64) -> Vec<FaceStyle> {
    if crate::suitcase::is_suitcase(path) {
        if crate::suitcase::fork_len(path) > max_read {
            return vec![];
        }
        return crate::suitcase::read_fonts(path)
            .into_iter()
            .flat_map(|font| reader_face_names_within(std::io::Cursor::new(font), max_faces, max_read))
            .collect();
    }
    match std::fs::File::open(path) {
        Ok(file) => reader_face_names_within(file, max_faces, max_read),
        Err(_) => vec![],
    }
}

/// [`file_face_names_within`] for the font file `file` reads.
#[cfg(not(target_arch = "wasm32"))]
fn reader_face_names_within(mut file: impl std::io::Read + std::io::Seek, max_faces: u32, max_read: u64) -> Vec<FaceStyle> {
    use std::io::SeekFrom;
    /// The largest table read from a (possibly damaged) file.
    const MAX_NAME_TABLE: u32 = 1 << 20;
    let mut read = 0u64;
    let mut read_at = |offset: u64, len: usize| -> Option<Vec<u8>> {
        read = read.checked_add(u64::try_from(len).ok()?).filter(|r| *r <= max_read)?;
        let mut buf = vec![0; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let be32 = |b: &[u8], at: usize| b.get(at..at + 4).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes);
    let Some(head) = read_at(0, 12) else { return vec![] };
    // A collection lists where each face's table directory starts.
    let starts: Vec<u32> = if head.starts_with(b"ttcf") {
        let n = be32(&head, 8).unwrap_or(0).min(max_faces) as usize;
        read_at(12, n * 4).map(|b| b.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect()).unwrap_or_default()
    } else {
        vec![0]
    };
    starts
        .into_iter()
        .filter_map(|start| {
            let dir = read_at(start.into(), 12)?;
            let tables = u16::from_be_bytes(dir.get(4..6)?.try_into().ok()?) as usize;
            let records = read_at(u64::from(start) + 12, tables * 16)?;
            if !records.as_chunks::<16>().0.iter().any(|r| OUTLINE_TABLES.iter().any(|t| r.starts_with(*t))) {
                return None;
            }
            let mut table = |tag: &[u8; 4]| -> Option<Vec<u8>> {
                let rec: &[u8] = records.as_chunks::<16>().0.iter().find(|r| r.starts_with(tag))?;
                let (offset, len) = (be32(rec, 8)?, be32(rec, 12)?);
                if len > MAX_NAME_TABLE {
                    return None;
                }
                read_at(offset.into(), len as usize)
            };
            let name = table(b"name")?;
            // A variable font's named instances are styles too.
            let fvar = table(b"fvar");
            // Its OS/2 table (the font's classification, for the menus' filters), when it has one.
            let os2 = table(b"OS/2");
            // A font holding just these tables, to read them as the font itself would.
            let mut tables: Vec<(&[u8; 4], &[u8])> = vec![(b"name", &name)];
            if let Some(fvar) = &fvar {
                tables.push((b"fvar", fvar));
            }
            if let Some(os2) = &os2 {
                tables.push((b"OS/2", os2));
            }
            let font = sfnt_of(&tables)?;
            Some(face_styles(&skrifa::FontRef::new(&font).ok()?))
        })
        .flatten()
        .collect()
}

/// Whether the file at `path` is a font file by its extension (`.ttf`, `.otf`, `.ttc` or `.otc`,
/// in any case), as the font scan reads the files in a folder.
pub fn is_font_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["ttf", "otf", "ttc", "otc"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Whether the file at `path` is a suitcase font ([`crate::suitcase`]): on macOS, a file with
/// nothing in its data and its fonts in its resource fork. Never on the web.
pub fn is_suitcase(path: &Path) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::suitcase::is_suitcase(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        false
    }
}
