//! HEIC and HEIF images for File › Open and File › Place, decoded with heic-rs when the engine's
//! `heif` feature is on (the default). The image is bounded before decoding and converted to RGBA
//! for the common PNG storage path. A build without the feature still recognizes the files and
//! refuses them with a message.

#[cfg(all(test, feature = "heif"))]
pub(super) use decoder::{active_sps, picture_bytes, sps_signal, sps_size};
#[cfg(feature = "heif")]
pub(super) use decoder::{decode, quarter_turned, with_exif};

/// The brands heic-rs 0.1.4 reads: HEVC-coded images and image sequences (ISO/IEC 23008-12 annex
/// B) and the general image and sequence brands.
const BRANDS: [&[u8; 4]; 7] = [b"heic", b"heix", b"heim", b"heis", b"hevc", b"mif1", b"msf1"];

/// Recognize a HEIF file by its `ftyp` box: one of [`BRANDS`] as the major brand or a compatible
/// one, and no AVIF brand (`avif`, `avis`), as heic-rs refuses a file that lists one. The box is
/// read as JP2 boxes are ([`super::jp2read::boxes`]): ISO BMFF sizes, the 64-bit one included.
pub(super) fn is_heif(bytes: &[u8]) -> bool {
    let Some((b"ftyp", ftyp)) = super::jp2read::boxes(bytes).next() else { return false };
    let Some(major) = ftyp.get(..4).and_then(|b| <&[u8; 4]>::try_from(b).ok()) else { return false };
    if ftyp.len() < 8 {
        return false;
    }
    let compatible = ftyp.get(8..).unwrap_or_default().as_chunks::<4>().0;
    let brands = || std::iter::once(major).chain(compatible);
    !brands().any(|brand| matches!(brand, b"avif" | b"avis")) && brands().any(|brand| BRANDS.contains(&brand))
}

/// Without the `heif` feature every HEIF file is refused.
#[cfg(not(feature = "heif"))]
pub(super) fn decode(_bytes: &[u8], _max_alloc: u64) -> Result<image::RgbaImage, String> {
    Err("HEIC and HEIF images can't be opened in this build".into())
}

/// Without the `heif` feature no image is decoded, so none is turned.
#[cfg(not(feature = "heif"))]
pub(super) fn quarter_turned(_bytes: &[u8]) -> bool {
    false
}

/// Without the `heif` feature no resolution is read.
#[cfg(not(feature = "heif"))]
pub(super) fn with_exif<T>(_bytes: &[u8], _read: impl FnOnce(&[u8]) -> Option<T>) -> Option<T> {
    None
}

#[cfg(feature = "heif")]
mod decoder {
    use heic_rs::context::Context;
    use heic_rs::{DecodeOptions, Error, MatrixCoefficients, Nclx, PixelLayout, Range};

    /// The message for a file that isn't a readable HEIF image.
    const UNREADABLE: &str = "the HEIC or HEIF file can't be read";
    /// The message for an image sequence, an overlay, an image that isn't HEVC-coded, a grid whose
    /// tiles are cropped, turned or mirrored, or an HEVC coding tool the decoder doesn't implement.
    const UNSUPPORTED: &str = "the HEIC or HEIF file holds a kind of image that can't be read";

    /// Decode the primary image within `max_alloc` bytes, counting the decoder's planes, the RGBA
    /// output and the copy that rotating or cropping makes, with the container's rotation, mirror
    /// and crop applied. An embedded RGB ICC profile, or `nclx` Display P3 primaries with the sRGB
    /// curve, is converted to sRGB; other `nclx` values are read as sRGB. A panic inside the
    /// decoder is reported as an unreadable file.
    pub(in super::super) fn decode(bytes: &[u8], max_alloc: u64) -> Result<image::RgbaImage, String> {
        crate::guard::catch_panic(|| decode_unguarded(bytes, max_alloc)).unwrap_or_else(|_| Err(UNREADABLE.into()))
    }

    /// Whether the container's rotations of the primary image add up to an odd number of quarter
    /// turns, so its width and height swap. The decoder applies every `irot` the item lists.
    pub(in super::super) fn quarter_turned(bytes: &[u8]) -> bool {
        let turned = || {
            let ctx = Context::open(bytes).ok()?;
            let props = ctx.props(ctx.meta.primary).ok()?;
            let quarters = props.transforms.iter().filter(|t| matches!(t, heic_rs::props::Transform::Rotate(r) if r.swaps_axes())).count();
            Some(quarters % 2 == 1)
        };
        crate::guard::catch_panic(turned).ok().flatten().unwrap_or(false)
    }

    /// `read` of the primary image's EXIF block, a TIFF structure inside `bytes`, for its
    /// resolution tags.
    pub(in super::super) fn with_exif<T>(bytes: &[u8], read: impl FnOnce(&[u8]) -> Option<T>) -> Option<T> {
        let find = || {
            let ctx = Context::open(bytes).ok()?;
            ctx.exif(ctx.meta.primary).ok().flatten()
        };
        crate::guard::catch_panic(find).ok().flatten().and_then(read)
    }

    fn decode_unguarded(bytes: &[u8], max_alloc: u64) -> Result<image::RgbaImage, String> {
        let refused = |e: Error| match e {
            Error::Unsupported(_) | Error::MissingBox("hvcC" | "meta") => UNSUPPORTED.to_string(),
            _ => UNREADABLE.to_string(),
        };
        let info = heic_rs::probe(bytes).map_err(refused)?;
        let ctx = Context::open(bytes).map_err(refused)?;
        if unsupported_items(&ctx).ok_or(UNREADABLE)? {
            return Err(UNSUPPORTED.into());
        }
        // The stored size, before a crop: the size the decoder holds.
        let too_large = || format!("an image of {} × {} pixels is too large to draw", info.coded_width, info.coded_height);
        let declared = u64::from(info.coded_width).checked_mul(u64::from(info.coded_height)).ok_or_else(too_large)?;
        // A grid is composed at the size its own data gives, which need not match `ispe`.
        let grid = grid_pixels(&ctx).ok_or(UNREADABLE)?;
        // A single picture is converted at the size its stream gives, which need not match `ispe`
        // either.
        let coded = declared.max(grid).max(stream_pixels(&ctx).ok_or(UNREADABLE)?);
        let shown = u64::from(info.width).checked_mul(u64::from(info.height)).ok_or_else(too_large)?;
        // The RGBA output, and a second RGBA image when the decoder rotates, mirrors or crops it,
        // which is no larger than the first.
        let output = coded.checked_mul(8).ok_or_else(too_large)?;
        let pictures = picture_bytes_in(&ctx).ok_or(UNREADABLE)?;
        // An alpha grid is composed into a frame of 16-bit planes (at most three samples a pixel),
        // and so is the image's grid when it is decoded with a color description of its tiles or
        // its stream. A single picture is converted in place.
        let composed = if grid > 0 { coded.checked_mul(if info.has_alpha { 12 } else { 6 }).ok_or_else(too_large)? } else { 0 };
        if coded == 0 || shown == 0 || output.checked_add(pictures).and_then(|n| n.checked_add(composed)).is_none_or(|n| n > max_alloc) {
            return Err(too_large());
        }
        let max_pixels = max_alloc / 4;
        let decoded = match tile_nclx(&ctx).or_else(|| stream_nclx(&ctx)) {
            Some(nclx) => decode_with_nclx(&ctx, nclx, max_pixels),
            None => heic_rs::decode(bytes, &DecodeOptions::default().with_layout(PixelLayout::Rgba8).with_max_pixels(Some(max_pixels))),
        }
        .map_err(|e| match e {
            Error::PixelLimit { .. } | Error::BoxTooLarge { .. } => too_large(),
            e => refused(e),
        })?;
        let mut img = image::RgbaImage::from_raw(decoded.width, decoded.height, decoded.data).ok_or(UNREADABLE)?;
        if let Some(profile) = source_profile(&ctx) {
            for pixel in img.pixels_mut() {
                let [r, g, b, a] = pixel.0;
                let device = [r, g, b].map(|v| f32::from(v) / 255.0);
                if let Some(rgb) = profile.to_srgb_into(&device, vectorcraft_color::cms::Intent::RelativeColorimetric) {
                    let [r, g, b] = rgb.map(|v| (v * 255.0).round() as u8);
                    pixel.0 = [r, g, b, a];
                }
            }
        }
        Ok(img)
    }

    /// The primary image and its alpha image, when it has one.
    fn primary_and_alpha(ctx: &Context<'_>) -> Option<impl Iterator<Item = u32>> {
        let primary = ctx.meta.primary;
        Some(std::iter::once(primary).chain(ctx.alpha_item(primary).ok()?))
    }

    /// Whether the primary image or its alpha image is neither an HEVC-coded item (`hvc1`, `hev1`)
    /// nor a grid of them, or is a grid with a tile that carries a crop, rotation or mirror. heic-rs
    /// 0.1.4 decodes any item with an `hvcC` whatever its type, and composes a grid from its tiles
    /// as coded, without their transforms: an encoder that pads tiles smaller than its coding block
    /// (64 × 64 for x265) and crops them back with `clap` gets the wrong pixels.
    fn unsupported_items(ctx: &Context<'_>) -> Option<bool> {
        let hevc = |id: u32| ctx.meta.item(id).is_some_and(|info| info.is_hevc());
        for item in primary_and_alpha(ctx)? {
            let Some((_, tiles)) = ctx.grid(item).ok()? else {
                if !hevc(item) {
                    return Some(true);
                }
                continue;
            };
            for tile in tiles {
                if !hevc(tile) || !ctx.props(tile).ok()?.transforms.is_empty() {
                    return Some(true);
                }
            }
        }
        Some(false)
    }

    /// The pixels of the larger of the primary image's grid and its alpha image's grid, at the
    /// output size each grid gives; 0 when neither is a grid.
    fn grid_pixels(ctx: &Context<'_>) -> Option<u64> {
        let mut largest = 0u64;
        for item in primary_and_alpha(ctx)? {
            if let Some((grid, _)) = ctx.grid(item).ok()? {
                largest = largest.max(u64::from(grid.output_width) * u64::from(grid.output_height));
            }
        }
        Some(largest)
    }

    /// The pixels of the larger of the primary image and its alpha image when either is a single
    /// picture, at the size of its largest sequence parameter set (before its conformance window);
    /// 0 when both are grids. `None` when a size can't be read.
    fn stream_pixels(ctx: &Context<'_>) -> Option<u64> {
        let mut largest = 0u64;
        for item in primary_and_alpha(ctx)? {
            if ctx.grid(item).ok()?.is_some() {
                continue;
            }
            // An item without `hvcC` isn't decoded: heic-rs refuses the file.
            let Some(hvcc) = ctx.props(item).ok()?.hvcc else { continue };
            for nal in hvcc.parameter_sets().into_iter().filter(|nal| nal_type(nal) == Some(SPS)) {
                let (width, height, _) = sps_size(nal)?;
                largest = largest.max(width.checked_mul(height)?);
            }
        }
        Some(largest)
    }

    /// The first tile's `nclx` when the primary image is a grid without an `nclx` of its own and
    /// its tiles carry one. heic-rs 0.1.4 converts a grid's samples to RGB with the grid item's
    /// `nclx` alone, and with its BT.709 limited-range default when the grid has none
    /// (tbraun96/heic-rs#6).
    fn tile_nclx(ctx: &Context<'_>) -> Option<Nclx> {
        let primary = ctx.meta.primary;
        if ctx.props(primary).ok()?.nclx.is_some() {
            return None;
        }
        let (_, tiles) = ctx.grid(primary).ok()??;
        ctx.props(*tiles.first()?).ok()?.nclx
    }

    /// The color description in the HEVC stream's video usability information (ITU-T H.265
    /// E.2.1) of the primary image, or of its grid's first tile, when neither carries an `nclx`
    /// (a file whose only color box is an ICC profile, or one without a color box). heic-rs 0.1.4
    /// reads only the container's `nclx` and otherwise converts with BT.709 limited range.
    fn stream_nclx(ctx: &Context<'_>) -> Option<Nclx> {
        let primary = ctx.meta.primary;
        if ctx.props(primary).ok()?.nclx.is_some() {
            return None;
        }
        let coded = match ctx.grid(primary).ok()? {
            Some((_, tiles)) => *tiles.first()?,
            None => primary,
        };
        let props = ctx.props(coded).ok()?;
        if props.nclx.is_some() {
            return None;
        }
        let hvcc = props.hvcc?;
        let data = ctx.item_data(coded).ok()?;
        let slices = hvcc.split_nals(&data).ok()?;
        active_sps(&hvcc.parameter_sets(), &slices).and_then(sps_signal)
    }

    /// The SPS the first slice of `slices` uses: the one its PPS names (ITU-T H.265 7.4.3.3.1).
    pub(in super::super) fn active_sps<'a>(parameter_sets: &[&'a [u8]], slices: &[&[u8]]) -> Option<&'a [u8]> {
        let slice = slices.iter().find(|nal| nal_type(nal).is_some_and(|t| t < 32))?;
        let bits = &mut Bits { data: &payload(slice), at: 0 };
        // first_slice_segment_in_pic_flag, no_output_of_prior_pics_flag in an IRAP picture (types 16
        // to 23), slice_pic_parameter_set_id.
        bits.skip(1)?;
        if nal_type(slice).is_some_and(|t| (16..=23).contains(&t)) {
            bits.skip(1)?;
        }
        let pps_id = bits.exp_golomb()?;
        // pps_pic_parameter_set_id, pps_seq_parameter_set_id.
        let pps_ids = |nal: &[u8]| {
            let bits = &mut Bits { data: &payload(nal), at: 0 };
            Some((bits.exp_golomb()?, bits.exp_golomb()?))
        };
        let sps_id = parameter_sets.iter().filter(|nal| nal_type(nal) == Some(PPS)).find_map(|nal| pps_ids(nal).filter(|(id, _)| *id == pps_id))?.1;
        let sps_ids = |nal: &[u8]| {
            let rbsp = sps_payload(nal)?;
            sps_head(&mut Bits { data: &rbsp, at: 0 }).map(|(_, id, ..)| id)
        };
        parameter_sets.iter().copied().find(|nal| sps_ids(nal) == Some(sps_id))
    }

    /// The primary image decoded as heic-rs 0.1.4 decodes it (a grid's tiles one after another),
    /// with its samples converted to RGBA with `nclx`, the alpha image applied and the image's
    /// rotation, mirror and crop applied.
    fn decode_with_nclx(ctx: &Context<'_>, nclx: Nclx, max_pixels: u64) -> heic_rs::Result<heic_rs::Image> {
        use heic_rs::hevc::Frame;

        fn coded(ctx: &Context<'_>, id: u32) -> heic_rs::Result<Frame> {
            let props = ctx.props(id)?;
            let hvcc = props.hvcc.as_ref().ok_or(Error::MissingBox("hvcC"))?;
            let data = ctx.item_data(id)?;
            let frame = heic_rs::hevc::decode_still(&hvcc.parameter_sets(), &hvcc.split_nals(&data)?)?;
            frame.validate()?;
            Ok(frame)
        }
        fn item(ctx: &Context<'_>, id: u32, max_pixels: u64) -> heic_rs::Result<Frame> {
            match ctx.grid(id)? {
                Some((grid, tiles)) => {
                    // The tiles decode in parallel on native, as heic-rs's own decode does.
                    #[cfg(not(target_arch = "wasm32"))]
                    let frames = {
                        use rayon::prelude::*;
                        tiles.par_iter().map(|&t| coded(ctx, t)).collect::<heic_rs::Result<Vec<_>>>()?
                    };
                    #[cfg(target_arch = "wasm32")]
                    let frames = tiles.iter().map(|&t| coded(ctx, t)).collect::<heic_rs::Result<Vec<_>>>()?;
                    heic_rs::grid::compose(&grid, &frames, max_pixels)
                }
                None => coded(ctx, id),
            }
        }

        let primary = ctx.meta.primary;
        let props = ctx.props(primary)?;
        let frame = item(ctx, primary, max_pixels)?;
        let alpha = ctx.alpha_item(primary)?.map(|a| item(ctx, a, max_pixels)).transpose()?;
        // `None`: rayon's global pool on native, where heic-rs is built with `parallel`.
        let image = heic_rs::color::convert(&frame, alpha.as_ref(), nclx, PixelLayout::Rgba8, max_pixels, None)?;
        heic_rs::transform::apply_all(image, &props.transforms)
    }

    /// The bytes heic-rs 0.1.4 allocates for the coded pictures of `bytes` it decodes (see
    /// [`picture_bytes_in`]); `None` when they can't be read.
    #[cfg(test)]
    pub(in super::super) fn picture_bytes(bytes: &[u8]) -> Option<u64> {
        picture_bytes_in(&Context::open(bytes).ok()?)
    }

    /// The bytes heic-rs 0.1.4 allocates for the coded pictures it decodes: the primary image's,
    /// or each tile of its grid, and its alpha image's the same way. Each picture is sized by the
    /// largest of its sequence parameter sets (the size the decoder allocates, rounded up to whole
    /// 64-sample coding blocks), not by the container, and takes three sets of 16-bit planes: the
    /// picture, a filtered copy and the frame returned. `None` when an item's sizes can't be read.
    fn picture_bytes_in(ctx: &Context<'_>) -> Option<u64> {
        let mut total = 0u64;
        for item in primary_and_alpha(ctx)? {
            let coded = match ctx.grid(item).ok()? {
                Some((_, tiles)) => tiles,
                None => vec![item],
            };
            for id in coded {
                let props = ctx.props(id).ok()?;
                // An item without `hvcC` isn't decoded: heic-rs refuses the file.
                let Some(hvcc) = props.hvcc else { continue };
                // The slices pick their SPS through the PPS, so the largest one is counted. Every
                // SPS must be read.
                let mut largest = None;
                for nal in hvcc.parameter_sets().into_iter().filter(|nal| nal_type(nal) == Some(SPS)) {
                    let (width, height, chroma) = sps_size(nal)?;
                    let blocks = |n: u64| n.div_ceil(64) * 64;
                    // Bytes a sample position takes in one set of planes: luma and the two chroma
                    // planes.
                    let planes = match chroma {
                        0 => 2,
                        1 => 3,
                        2 => 4,
                        _ => 6,
                    };
                    let picture = blocks(width).checked_mul(blocks(height))?.checked_mul(planes * 3)?;
                    largest = largest.max(Some(picture));
                }
                total = total.checked_add(largest?)?;
            }
        }
        Some(total)
    }

    /// The NAL unit type of an HEVC sequence parameter set.
    const SPS: u8 = 33;

    /// The type of an HEVC NAL unit, from its header.
    fn nal_type(nal: &[u8]) -> Option<u8> {
        nal.first().map(|b| (b >> 1) & 0x3f)
    }

    /// The NAL unit type of an HEVC picture parameter set.
    const PPS: u8 = 34;

    /// A NAL unit's payload, without its two-byte header and the emulation prevention bytes (the 03
    /// of 00 00 03), up to its first 4,096 bytes.
    fn payload(nal: &[u8]) -> Vec<u8> {
        let mut rbsp = Vec::with_capacity(256);
        let mut zeros = 0;
        for &b in nal.get(2..).unwrap_or_default().iter().take(4096) {
            if b == 3 && zeros >= 2 {
                zeros = 0;
                continue;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            rbsp.push(b);
        }
        rbsp
    }

    /// An SPS NAL unit's [`payload`]; `None` for another NAL unit.
    fn sps_payload(nal: &[u8]) -> Option<Vec<u8>> {
        (nal_type(nal) == Some(SPS)).then(|| payload(nal))
    }

    /// The fields of an SPS (ITU-T H.265 7.3.2.2) up to the picture height:
    /// `sps_max_sub_layers_minus1`, `sps_seq_parameter_set_id`, the width, the height and
    /// `chroma_format_idc`.
    fn sps_head(bits: &mut Bits<'_>) -> Option<(u64, u64, u64, u64, u64)> {
        // sps_video_parameter_set_id, sps_max_sub_layers_minus1, sps_temporal_id_nesting_flag.
        bits.skip(4)?;
        let sub_layers = bits.read(3)?;
        bits.skip(1)?;
        // profile_tier_level (7.3.3): the general profile, tier and level, then the sub-layers'.
        bits.skip(96)?;
        let mut present = Vec::new();
        for _ in 0..sub_layers {
            present.push((bits.read(1)?, bits.read(1)?));
        }
        if sub_layers > 0 {
            bits.skip(2 * (8 - sub_layers as usize))?;
        }
        for (profile, level) in present {
            bits.skip(88 * profile as usize + 8 * level as usize)?;
        }
        let id = bits.exp_golomb()?;
        let chroma = bits.exp_golomb()?;
        if chroma == 3 {
            bits.skip(1)?;
        }
        let width = bits.exp_golomb()?;
        let height = bits.exp_golomb()?;
        Some((sub_layers, id, width, height, chroma))
    }

    /// The picture width, height and `chroma_format_idc` of an HEVC sequence parameter set NAL
    /// unit, before its conformance window; `None` for another NAL unit.
    pub(in super::super) fn sps_size(nal: &[u8]) -> Option<(u64, u64, u64)> {
        let rbsp = sps_payload(nal)?;
        let (_, _, width, height, chroma) = sps_head(&mut Bits { data: &rbsp, at: 0 })?;
        Some((width, height, chroma))
    }

    /// The video signal type of an HEVC sequence parameter set NAL unit's VUI (ITU-T H.265 E.2.1)
    /// as an `nclx`: the full-range flag and, when the VUI describes them, the color primaries,
    /// transfer characteristics and matrix coefficients (otherwise 2, unspecified). `None` when the
    /// SPS has no VUI, the VUI has no video signal type, or the SPS can't be read that far.
    pub(in super::super) fn sps_signal(nal: &[u8]) -> Option<Nclx> {
        let rbsp = sps_payload(nal)?;
        let bits = &mut Bits { data: &rbsp, at: 0 };
        let (sub_layers, ..) = sps_head(bits)?;
        skip_to_vui(bits, sub_layers)?;
        // vui_parameters_present_flag, then aspect_ratio_info_present_flag (an Extended_SAR
        // aspect_ratio_idc of 255 adds its 32-bit ratio) and overscan_info_present_flag.
        if bits.read(1)? == 0 {
            return None;
        }
        if bits.read(1)? == 1 && bits.read(8)? == 255 {
            bits.skip(32)?;
        }
        if bits.read(1)? == 1 {
            bits.skip(1)?;
        }
        // video_signal_type_present_flag: video_format, video_full_range_flag and the color
        // description.
        if bits.read(1)? == 0 {
            return None;
        }
        bits.skip(3)?;
        let range = if bits.read(1)? == 1 { Range::Full } else { Range::Limited };
        let (primaries, transfer, matrix) = if bits.read(1)? == 1 { (bits.read(8)?, bits.read(8)?, bits.read(8)?) } else { (2, 2, 2) };
        let code = |v: u64| u16::try_from(v).ok();
        let matrix_code = code(matrix)?;
        Some(Nclx { primaries: code(primaries)?, transfer: code(transfer)?, matrix: MatrixCoefficients::from_code(matrix_code), matrix_code, range })
    }

    /// Skips the fields of an SPS after [`sps_head`], up to `vui_parameters_present_flag`.
    fn skip_to_vui(bits: &mut Bits<'_>, sub_layers: u64) -> Option<()> {
        // conformance_window_flag and its four offsets.
        if bits.read(1)? == 1 {
            for _ in 0..4 {
                bits.exp_golomb()?;
            }
        }
        // bit_depth_luma_minus8, bit_depth_chroma_minus8.
        bits.exp_golomb()?;
        bits.exp_golomb()?;
        let poc_bits = bits.exp_golomb()?.checked_add(4).filter(|&n| n <= 16)?;
        // sps_sub_layer_ordering_info_present_flag, then three values for each sub-layer given.
        let ordered = if bits.read(1)? == 1 { sub_layers + 1 } else { 1 };
        for _ in 0..ordered * 3 {
            bits.exp_golomb()?;
        }
        // The coding and transform block sizes and the transform hierarchy depths.
        for _ in 0..6 {
            bits.exp_golomb()?;
        }
        // scaling_list_enabled_flag, sps_scaling_list_data_present_flag.
        if bits.read(1)? == 1 && bits.read(1)? == 1 {
            skip_scaling_lists(bits)?;
        }
        // amp_enabled_flag, sample_adaptive_offset_enabled_flag.
        bits.skip(2)?;
        // pcm_enabled_flag: the PCM sample depths, block sizes and loop filter flag.
        if bits.read(1)? == 1 {
            bits.skip(8)?;
            bits.exp_golomb()?;
            bits.exp_golomb()?;
            bits.skip(1)?;
        }
        skip_reference_picture_sets(bits)?;
        // long_term_ref_pics_present_flag: each picture's POC bits and used flag.
        if bits.read(1)? == 1 {
            let count = bits.exp_golomb()?;
            if count > 32 {
                return None;
            }
            for _ in 0..count {
                bits.skip(poc_bits as usize + 1)?;
            }
        }
        // sps_temporal_mvp_enabled_flag, strong_intra_smoothing_enabled_flag.
        bits.skip(2)
    }

    /// Skips `scaling_list_data()` (ITU-T H.265 7.3.4).
    fn skip_scaling_lists(bits: &mut Bits<'_>) -> Option<()> {
        for size in 0..4 {
            // The 32 × 32 lists are given for matrices 0 and 3 only.
            for _ in 0..if size == 3 { 2 } else { 6 } {
                if bits.read(1)? == 0 {
                    // scaling_list_pred_matrix_id_delta.
                    bits.exp_golomb()?;
                } else {
                    // scaling_list_dc_coef_minus8, then each scaling_list_delta_coef (se(v) codes,
                    // as long as ue(v) codes).
                    if size > 1 {
                        bits.exp_golomb()?;
                    }
                    for _ in 0..if size == 0 { 16 } else { 64 } {
                        bits.exp_golomb()?;
                    }
                }
            }
        }
        Some(())
    }

    /// Skips `num_short_term_ref_pic_sets` and each `st_ref_pic_set()` (ITU-T H.265 7.3.7) of an
    /// SPS. A set predicted from the one before it has one or two flags for each of that set's
    /// pictures and one more.
    fn skip_reference_picture_sets(bits: &mut Bits<'_>) -> Option<()> {
        let sets = bits.exp_golomb()?;
        if sets > 64 {
            return None;
        }
        // NumDeltaPocs of each set read so far.
        let mut pictures: Vec<u64> = Vec::new();
        for index in 0..sets {
            let predicted = index != 0 && bits.read(1)? == 1;
            let count = if predicted {
                // delta_rps_sign, abs_delta_rps_minus1.
                bits.skip(1)?;
                bits.exp_golomb()?;
                let mut count = 0;
                for _ in 0..=*pictures.last()? {
                    // used_by_curr_pic_flag, and use_delta_flag when it is 0.
                    if bits.read(1)? == 1 || bits.read(1)? == 1 {
                        count += 1;
                    }
                }
                count
            } else {
                let (negative, positive) = (bits.exp_golomb()?, bits.exp_golomb()?);
                if negative > 16 || positive > 16 {
                    return None;
                }
                // Each picture's delta_poc_s0_minus1 or delta_poc_s1_minus1 and its used flag.
                for _ in 0..negative + positive {
                    bits.exp_golomb()?;
                    bits.skip(1)?;
                }
                negative + positive
            };
            pictures.push(count);
        }
        Some(())
    }

    /// A big-endian bit reader over an RBSP.
    struct Bits<'a> {
        data: &'a [u8],
        at: usize,
    }

    impl Bits<'_> {
        fn bit(&mut self) -> Option<u64> {
            let byte = self.data.get(self.at / 8)?;
            let bit = (byte >> (7 - self.at % 8)) & 1;
            self.at += 1;
            Some(u64::from(bit))
        }

        fn read(&mut self, n: u32) -> Option<u64> {
            (0..n).try_fold(0u64, |v, _| Some(v << 1 | self.bit()?))
        }

        fn skip(&mut self, n: usize) -> Option<()> {
            self.at = self.at.checked_add(n)?;
            (self.at <= self.data.len() * 8).then_some(())
        }

        /// An unsigned Exp-Golomb code, ue(v), of at most 32 leading zeros.
        fn exp_golomb(&mut self) -> Option<u64> {
            let mut zeros = 0;
            while self.bit()? == 0 {
                zeros += 1;
                if zeros > 32 {
                    return None;
                }
            }
            Some((1u64 << zeros) - 1 + self.read(zeros)?)
        }
    }

    /// `nclx` color primaries for Display P3 (SMPTE EG 432-1) and transfer characteristics for
    /// the sRGB curve (IEC 61966-2-1), ITU-T H.273.
    const P3_PRIMARIES: u16 = 12;
    const SRGB_TRANSFER: u16 = 13;

    /// The profile the decoded RGB values are in when it isn't sRGB: the primary item's RGB ICC
    /// profile, else Display P3 when its `nclx` names P3 primaries with the sRGB curve. A grid
    /// without its own `colr` takes its first tile's, as cameras write it on the tiles. A profile
    /// that can't be read, or isn't RGB, is left out and the values are read as sRGB. Without any
    /// `colr`, the HEVC stream's color description ([`stream_nclx`]) is read the same way.
    fn source_profile(ctx: &Context<'_>) -> Option<vectorcraft_color::cms::IccProfile> {
        use vectorcraft_color::cms::{self, IccProfile, ProfileKind};

        let id = ctx.meta.primary;
        let own = ctx.props(id).ok()?;
        let described = |p: &heic_rs::props::ItemProps<'_>| p.icc.is_some() || p.nclx.is_some();
        let tile = || ctx.grid(id).ok().flatten().and_then(|(_, tiles)| tiles.first().and_then(|&t| ctx.props(t).ok())).filter(described);
        let props = if described(&own) { Some(own) } else { tile() };
        if let Some(icc) = props.as_ref().and_then(|p| p.icc) {
            return IccProfile::from_bytes(None, icc).ok().filter(|p| p.kind == ProfileKind::Rgb);
        }
        let nclx = match props {
            Some(props) => props.nclx?,
            None => stream_nclx(ctx)?,
        };
        (nclx.primaries == P3_PRIMARIES && nclx.transfer == SRGB_TRANSFER).then(|| cms::icc::builtin_rgb(cms::DISPLAY_P3)).flatten()
    }
}
