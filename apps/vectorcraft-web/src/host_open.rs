//! The size cap on documents a parent page sends through the host bridge (`host.rs`).

/// Bytes a document sent by the parent page (`vectorcraft:open`) may have. It matches the cap
/// on a library file read from disk (`MAX_FILE` in the engine's `cmd/library.rs`) and the
/// largest raster VectorCraft allocates in the browser (`MAX_RASTER_ALLOC` on wasm32), so the
/// bridge never copies more into the open inbox than the app's other import paths read.
pub const MAX_OPEN_BYTES: u64 = 512 << 20;

/// `Err` with the reason posted back as `vectorcraft:failed` when `len` bytes are over the cap.
pub fn check_open_size(name: &str, len: u64) -> Result<(), String> {
    if len > MAX_OPEN_BYTES {
        return Err(format!("{name} wasn't opened: it is {len} bytes, larger than the {} MB VectorCraft opens from the host page", MAX_OPEN_BYTES >> 20));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_open_bytes() {
        assert!(check_open_size("a.svg", 0).is_ok());
        assert!(check_open_size("a.svg", MAX_OPEN_BYTES).is_ok());
        let e = check_open_size("big.svg", MAX_OPEN_BYTES + 1).unwrap_err();
        assert!(e.contains("big.svg") && e.contains("512 MB"), "{e}");
    }
}
