//! Bounded reader for CorelDRAW documents (`.cdr`):
//! RIFF and ZIP container detection, embedded preview extraction (CF_DIB / PNG),
//! CPng stream decompression and document model mapping.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod container;
pub mod cpng;
pub mod error;
pub mod model;
pub mod preview;

pub use container::{ContainerType, is_cdr};
pub use error::Error;
pub use model::{CdrArtboard, CdrBitmap, CdrColor, CdrObject, CdrStroke, Document, LayerInfo, Limits, PathCmd, read};
pub use preview::{Preview, preview};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sniffing() {
        assert!(!is_cdr(b""));
        assert!(!is_cdr(b"RIFF\x00\x00\x00\x00WAVE"));
        assert!(is_cdr(b"RIFF\x00\x00\x00\x00CDR9extra"));
        assert!(is_cdr(b"RIFF\x00\x00\x00\x00CDRFextra"));
    }

    #[test]
    fn test_local_files_if_available() {
        let files = [
            "/home/deck/Pictures/cdr/zawiercie_logo.cdr",
            "/home/deck/Pictures/cdr/stowarzyszenie_logo.cdr",
            "/home/deck/Pictures/cdr/biala-folia.cdr",
            "/home/deck/Pictures/cdr/czarna folia.cdr",
            "/home/deck/Pictures/cdr/pomaranczowe-napisy.cdr",
            "/home/deck/Pictures/cdr/2010_KAL.cdr",
            "/home/deck/Pictures/cdr/spiewnik_CMC/ZAPROSZENIE_CMC.cdr",
            "/home/deck/Pictures/cdr/spiewnik_2010.cdr",
            "/home/deck/Pictures/cdr/ulotka.cdr",
            "/home/deck/Pictures/cdr/art_jura.CDR",
            "/home/deck/Pictures/cdr/sylwester_ognie.cdr",
            "/home/deck/Pictures/cdr/szlaki.net.pl.cdr",
        ];
        for file in files {
            let path = std::path::Path::new(file);
            if let Ok(bytes) = std::fs::read(path) {
                assert!(is_cdr(&bytes));
                let doc = read(&bytes, Limits::default());
                assert!(doc.is_ok(), "failed to read {file}: {:?}", doc.err());
                let d = doc.unwrap();
                assert!(d.width > 0.0);
                assert!(d.height > 0.0);
                assert!(!d.objects.is_empty(), "{file} should have imported objects");
                eprintln!(
                    "{file}: {:.1}x{:.1} pt, pages: {}, version: {:?}, layers: {}, objects: {}, bitmaps: {}",
                    d.width,
                    d.height,
                    d.artboards.len(),
                    d.version,
                    d.layers.len(),
                    d.objects.len(),
                    d.bitmaps.len()
                );
            }
        }
    }
}
