//! PNG text chunks (`tEXt`, uncompressed `iTXt`) read back; the PNG writer adds them
//! ([`vectorcraft_render::encode::png::with_text`]).

/// The `tEXt` and uncompressed `iTXt` entries of a PNG before its image data: (keyword, text).
pub fn text(png: &[u8]) -> Vec<(String, String)> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return vec![];
    }
    super::ppi::png_chunks(png)
        .take_while(|(ty, _)| *ty != b"IDAT")
        .filter_map(|(ty, data)| {
            let nul = data.iter().position(|b| *b == 0)?;
            let keyword: String = data.get(..nul)?.iter().map(|b| char::from(*b)).collect();
            let rest = data.get(nul + 1..)?;
            let text = match ty {
                b"tEXt" => rest.iter().map(|b| char::from(*b)).collect(),
                // Compression flag 0, method, then the language tag and translated keyword.
                b"iTXt" if rest.first() == Some(&0) => {
                    let mut parts = rest.get(2..)?.splitn(3, |b| *b == 0);
                    let (_, _, text) = (parts.next()?, parts.next()?, parts.next()?);
                    String::from_utf8_lossy(text).into_owned()
                }
                _ => return None,
            };
            Some((keyword, text))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use vectorcraft_render::encode::png::with_text;

    use super::*;

    #[test]
    fn text_chunks_round_trip_and_stay_valid() {
        let png = vectorcraft_render::Rendered { width: 2, height: 1, pixels: vec![255; 8] }.to_png().unwrap();
        let out = with_text(png.clone(), &[("Title", "Café".into()), ("Author", "Łukasz".into()), ("bad\u{1}", "x".into())]);
        assert_eq!(text(&out), [("Title".to_string(), "Café".to_string()), ("Author".into(), "Łukasz".into())]);
        // Still a PNG every decoder reads (the CRCs check out).
        let img = craft_image_codecs::decode(&out)
            .map(|__cimg| {
                let (__w, __h) = __cimg.dimensions();
                match (__cimg.layout(), __cimg.sample_type()) {
                    (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
                }
            })
            .unwrap();
        assert_eq!((img.width(), img.height()), (2, 1));
        assert_eq!(with_text(b"not a png".to_vec(), &[("Title", "x".into())]), b"not a png");
        assert_eq!(with_text(png.clone(), &[]), png);
    }
}
