//! Reading PCX files (`.pcx`) for File › Open and File › Place, from the ZSoft PCX Technical
//! Reference Manual (release 5). [`is_pcx`] recognizes a file by the first fields of its 128-byte
//! header, and [`decode`] reads its run-length encoded scan lines into RGBA pixels: 1-bit with two
//! colors; 1-bit in two to four planes and 4-bit with up to 16 colors; 2-bit CGA pictures in the
//! four colors their CGA palette settings pick; 8-bit with the
//! 256-color palette after the image data (gray levels without one); and 24-bit color in three
//! 8-bit planes. The file is untrusted: every read is checked.

/// The size of the header, after which the image data starts.
const HEADER: usize = 128;

/// The message for a file whose header or image data ends early.
const TRUNCATED: &str = "the PCX file is cut short";

/// The message for a header whose window or line length can't hold its image.
const DAMAGED: &str = "the PCX file's header is damaged";

/// The 16 colors of the EGA palette, which files of versions 0 and 3 use instead of the header's.
const EGA: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0xaa],
    [0x00, 0xaa, 0x00],
    [0x00, 0xaa, 0xaa],
    [0xaa, 0x00, 0x00],
    [0xaa, 0x00, 0xaa],
    [0xaa, 0x55, 0x00],
    [0xaa, 0xaa, 0xaa],
    [0x55, 0x55, 0x55],
    [0x55, 0x55, 0xff],
    [0x55, 0xff, 0x55],
    [0x55, 0xff, 0xff],
    [0xff, 0x55, 0x55],
    [0xff, 0x55, 0xff],
    [0xff, 0xff, 0x55],
    [0xff, 0xff, 0xff],
];

/// Does `bytes` start with a PCX header? The manufacturer byte is `0x0A`, the version 0, 2, 3, 4
/// or 5, the encoding 1 (run-length) and the bits per pixel of each plane 1, 2, 4 or 8.
pub(super) fn is_pcx(bytes: &[u8]) -> bool {
    matches!(bytes, [0x0a, 0 | 2 | 3 | 4 | 5, 1, 1 | 2 | 4 | 8, ..])
}

/// The resolution in the header's HDpi and VDpi fields; `None` when either is 0 or the header is
/// cut short.
pub(super) fn resolution(bytes: &[u8]) -> Option<(f64, f64)> {
    let le = |at: usize| Some(u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
    let (x, y) = (le(12)?, le(14)?);
    (x > 0 && y > 0).then(|| (f64::from(x), f64::from(y)))
}

/// The pixels of PCX file `bytes`, which [`is_pcx`] accepts, refused when they or the decoded
/// scan lines take more than `max_alloc` bytes (the pixels counted as RGBA) before anything is
/// decoded.
pub(super) fn decode(bytes: &[u8], max_alloc: u64) -> Result<image::RgbaImage, String> {
    let head = bytes.get(..HEADER).ok_or(TRUNCATED)?;
    let le = |at: usize| usize::from(u16::from_le_bytes([head[at], head[at + 1]]));
    let (version, bits, planes) = (head[1], head[3], head[65]);
    let (x_min, y_min, x_max, y_max) = (le(4), le(6), le(8), le(10));
    let line = le(66);
    if !matches!((bits, planes), (1, 1..=4) | (2 | 4, 1) | (8, 1 | 3)) {
        return Err(format!("PCX files with {bits}-bit pixels in {planes} planes can't be read"));
    }
    if x_max < x_min || y_max < y_min {
        return Err(DAMAGED.into());
    }
    let (w, h) = (x_max - x_min + 1, y_max - y_min + 1);
    // Each plane's line holds the row's pixels, padded to an even number of bytes.
    if line * 8 < w * usize::from(bits) {
        return Err(DAMAGED.into());
    }
    let scan = line * usize::from(planes);
    let rgba = w as u64 * h as u64 * 4;
    if rgba > max_alloc || scan as u64 * h as u64 > max_alloc {
        return Err(format!("an image of {w} × {h} pixels is too large to draw"));
    }
    // An 8-bit image's 256-color palette is its last 769 bytes, `0x0C` and then the colors; the
    // image data ends before it. A `0x0C` there is image data instead when the image data runs to
    // the end of the file.
    let candidate = bytes.len().checked_sub(769).filter(|at| *at >= HEADER && (bits, planes) == (8, 1) && bytes.get(*at) == Some(&0x0c));
    let body = bytes.get(HEADER..).unwrap_or_default();
    let (tail, decoded) = match candidate {
        Some(at) => match unpack(body, scan * h) {
            Ok((data, used)) if used == body.len() => (None, Ok((data, used))),
            _ => (Some(at), unpack(bytes.get(HEADER..at).unwrap_or_default(), scan * h)),
        },
        None => (None, unpack(body, scan * h)),
    };
    let (data, used) = decoded?;
    // A palette marker right after the image data with fewer than 768 colors after it is a palette
    // cut short.
    let after = bytes.get(HEADER + used..).unwrap_or_default();
    if tail.is_none() && (bits, planes) == (8, 1) && after.first() == Some(&0x0c) && after.len() < 769 {
        return Err(TRUNCATED.into());
    }
    let palette: Vec<[u8; 3]> = match (bits, planes) {
        (8, 1) => match tail.and_then(|at| bytes.get(at + 1..)) {
            Some(colors) => colors.as_chunks::<3>().0.to_vec(),
            None => (0..=255).map(|v| [v; 3]).collect(),
        },
        // A 1-bit image takes the header palette's first two colors; versions without a palette,
        // and headers whose two colors are the same (left unset), give black and white.
        (1, 1) if matches!(version, 0 | 3) || head[16..19] == head[19..22] => vec![[0; 3], [255; 3]],
        // A 2-bit image is a CGA picture, whose header holds CGA palette settings instead of colors.
        (2, 1) => cga(head[16], head[19]),
        _ if matches!(version, 0 | 3) => EGA.to_vec(),
        _ => head[16..64].as_chunks::<3>().0.to_vec(),
    };
    let mut img = image::RgbaImage::new(w as u32, h as u32);
    for (y, row) in data.chunks_exact(scan).enumerate() {
        let plane = |p: usize| row.get(p * line..(p + 1) * line).unwrap_or_default();
        for x in 0..w {
            let rgb = if bits == 8 && planes == 3 {
                [0, 1, 2].map(|p| plane(p).get(x).copied().unwrap_or(0))
            } else {
                // Plane p gives bits p × bits and up of the color index.
                let index = (0..usize::from(planes)).fold(0, |index, p| {
                    let at = x * usize::from(bits);
                    let byte = plane(p).get(at / 8).copied().unwrap_or(0);
                    let value = (byte >> (8 - usize::from(bits) - at % 8)) & (0xff >> (8 - bits));
                    index | usize::from(value) << (p * usize::from(bits))
                });
                palette.get(index).copied().unwrap_or([0; 3])
            };
            img.put_pixel(x as u32, y as u32, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
        }
    }
    Ok(img)
}

/// The four colors of a CGA picture from header bytes 16 and 19. The top four bits of `background`
/// are the EGA color of index 0. The top three bits of `settings` are the color burst, palette and
/// intensity bits (C, P, I), which pick the other three: with color burst on (C = 0), P = 0 gives
/// green, red and brown and P = 1 gives cyan, magenta and light gray; with it off (C = 1) they are
/// cyan, red and light gray. I = 1 gives the bright version of each (brown becomes yellow).
fn cga(background: u8, settings: u8) -> Vec<[u8; 3]> {
    let foreground: [usize; 3] = match settings >> 6 {
        0 => [2, 4, 6],
        1 => [3, 5, 7],
        _ => [3, 4, 7],
    };
    let bright = if settings & 0x20 != 0 { 8 } else { 0 };
    let ega = |i: usize| EGA.get(i).copied().unwrap_or([0; 3]);
    let mut colors = vec![ega(usize::from(background >> 4))];
    colors.extend(foreground.map(|i| ega(i + bright)));
    colors
}

/// `len` bytes of run-length encoded image data: a byte with its top two bits set repeats the
/// next byte as many times as its other six bits count, and any other byte stands for itself.
/// Runs may cross the end of a scan line. Also returns how many bytes of `data` were read.
fn unpack(data: &[u8], len: usize) -> Result<(Vec<u8>, usize), String> {
    let mut out = Vec::with_capacity(len);
    let mut bytes = data.iter();
    while out.len() < len {
        let &b = bytes.next().ok_or(TRUNCATED)?;
        if b & 0xc0 == 0xc0 {
            let &v = bytes.next().ok_or(TRUNCATED)?;
            let n = usize::from(b & 0x3f).min(len - out.len());
            out.resize(out.len() + n, v);
        } else {
            out.push(b);
        }
    }
    Ok((out, data.len() - bytes.len()))
}
