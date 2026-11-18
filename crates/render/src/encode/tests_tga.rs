//! Targa export: depths, alpha and the footer.

use super::on_white;
use super::tests_bmp::{art, decode, u16_at};
use super::tga::{self, TgaOptions};

#[test]
fn targa_depths_alpha_and_footer() {
    let (w, h) = (7, 3);
    let px = art(w, h);
    for (depth, alpha_bits) in [(16, 1), (24, 0), (32, 8)] {
        let file = tga::encode(&px, w, h, &TgaOptions { depth }).unwrap();
        assert_eq!((file[2], file[16], file[17]), (2, depth, alpha_bits), "true colour, the depth byte, alpha bits");
        assert_eq!((u16_at(&file, 12), u16_at(&file, 14)), (7, 3));
        assert_eq!(file.len(), 18 + (w * h) as usize * usize::from(depth / 8) + 26);
        assert!(file.ends_with(b"TRUEVISION-XFILE.\0"));
        let img = decode(&file, craft_image_codecs::Format::Tga);
        assert_eq!(img.dimensions(), (w, h));
        let want = on_white(px[(2 * w as usize + 3) * 4..][..4].try_into().unwrap());
        let got = img.get_pixel(3, 2).0;
        assert!(got[..3].iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 8), "depth {depth}: {got:?} vs {want:?}");
        match depth {
            32 => assert_eq!((img.get_pixel(0, 0).0[3], img.get_pixel(1, 0).0[3]), (0, 128), "8-bit alpha"),
            24 => assert_eq!(img.get_pixel(0, 0).0, [255; 4], "flattened on white"),
            _ => {}
        }
    }
    assert!(tga::encode(&px, w, h, &TgaOptions { depth: 8 }).is_err());
    assert!(tga::encode(&px, 70_000, 1, &TgaOptions::default()).is_err(), "too wide");
}
