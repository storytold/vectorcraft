use super::*;
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The data files ship exactly as Fogra publishes them in
/// https://fogra.org/fileadmin/files/7_downloads/Charakterisierungsdaten/F51-52.zip.
#[test]
fn characterization_data_is_the_published_files() {
    let want = [
        ("FOGRA51", "ad472a1c0bdafd558597ff93eeda5175805086565af814cb6346e910e2f25aa5", 57_111),
        ("FOGRA52", "be4f0732328ed62ade63ec7f317d3b9e9dad6bbe2430fefddaf0f901d5ba13d6", 56_781),
    ];
    for (c, (reference, sha, len)) in measured::CONDITIONS.iter().zip(want) {
        assert_eq!(c.reference, reference);
        assert_eq!((c.data.len(), hex(c.data).as_str()), (len, sha), "{reference}");
    }
}

#[test]
fn the_data_parses_into_its_patches() {
    for c in &measured::CONDITIONS {
        let patches = measured::parse(c.data).unwrap();
        assert_eq!(patches.len(), 1617, "{}", c.reference);
        assert_eq!(patches[0].cmyk, [0.0; 4]);
    }
    let p = measured::parse(measured::CONDITIONS[0].data).unwrap();
    assert_eq!(p[0].lab, Lab::new(95.0, 1.5, -6.0));
    assert_eq!(p[1].cmyk, [0.0, 0.1, 0.0, 0.0]);
    assert_eq!(p[1].lab, Lab::new(90.08, 8.15, -7.42));
}

#[test]
fn malformed_data_is_refused() {
    let file = |format: &str, rows: &str| format!("ISO28178\nBEGIN_DATA_FORMAT\n{format}\nEND_DATA_FORMAT\nBEGIN_DATA\n{rows}\nEND_DATA\n");
    let fields = "SAMPLE_ID CMYK_C CMYK_M CMYK_Y CMYK_K LAB_L LAB_A LAB_B";
    let good = file(fields, "1 0 0 0 0 95 1 -6\n2 100 0 0 0 55 -37 -50");
    assert_eq!(measured::parse(good.as_bytes()).map(|p| p.len()), Some(2));
    for bad in [
        file("SAMPLE_ID CMYK_C CMYK_M CMYK_Y LAB_L LAB_A LAB_B", "1 0 0 0 95 1 -6"),
        file(fields, "1 0 0 0 0 95 1"),
        file(fields, "1 0 0 0 0 95 NaN -6"),
        file(fields, "1 0 0 0 0 95 x -6"),
        file(fields, "1 0 0 0 101 95 1 -6"),
        file(fields, "1 -1 0 0 0 95 1 -6"),
        file(fields, ""),
        "BEGIN_DATA\n1 0 0 0 0 95 1 -6\nEND_DATA\n".into(),
    ] {
        assert_eq!(measured::parse(bad.as_bytes()), None, "{bad}");
    }
    assert_eq!(measured::parse(&[0xff, 0xfe, 0x00]), None);
}

/// ΔE2000 between each measured patch and the model's absolute Lab for its inks: (mean, max).
fn accuracy(g: &GenericCmyk, patches: &[measured::Patch]) -> (f32, f32) {
    let errors: Vec<f32> = patches.iter().map(|p| delta_e2000(g.to_lab_abs(p.cmyk), p.lab)).collect();
    (errors.iter().sum::<f32>() / errors.len() as f32, errors.iter().copied().fold(0.0, f32::max))
}

#[test]
fn the_working_spaces_reproduce_the_measured_patches() {
    for c in &measured::CONDITIONS {
        let patches = measured::parse(c.data).unwrap();
        let start = std::time::Instant::now();
        let g = measured::model(c).unwrap();
        let took = start.elapsed();
        let (mean, max) = accuracy(&g, &patches);
        println!("{}: computed in {took:?}, mean ΔE2000 {mean:.3}, max {max:.3}", c.reference);
        assert!(mean < 0.1 && max < 0.6, "{}: mean {mean}, max {max}", c.reference);
        // The paper is the measured blank patch.
        assert!(delta_e2000(g.to_lab_abs([0.0; 4]), patches[0].lab) < 0.05);
    }
}

/// A model fitted without every fourth patch (the paper and the solids and overprints stay in)
/// predicts the patches it left out.
#[test]
fn the_fit_predicts_patches_it_left_out() {
    for c in &measured::CONDITIONS {
        let patches = measured::parse(c.data).unwrap();
        let corner = |p: &measured::Patch| p.cmyk.iter().all(|v| *v == 0.0 || *v == 1.0);
        let (kept, left): (Vec<(usize, &measured::Patch)>, Vec<_>) = patches.iter().enumerate().partition(|(i, p)| i % 4 != 3 || corner(p));
        let kept: Vec<measured::Patch> = kept.into_iter().map(|(_, p)| *p).collect();
        let left: Vec<measured::Patch> = left.into_iter().map(|(_, p)| *p).collect();
        let g = measured::fit(c, &kept, measured::SMOOTHING).unwrap();
        let (mean, max) = accuracy(&g, &left);
        println!("{}: {} patches left out, mean ΔE2000 {mean:.3}, max {max:.3}", c.reference, left.len());
        assert!(mean < 0.3 && max < 2.5, "{}: mean {mean}, max {max}", c.reference);
    }
}

/// More of any ink makes a build lighter by less than 0.3 L*, also where the data has few patches
/// (black above 60%, and builds over the ink limit).
#[test]
fn more_ink_lightens_a_build_by_less_than_0_3_l() {
    let steps: Vec<f32> = (0..=20).map(|i| i as f32 / 20.0).collect();
    let coarse = [0.0, 0.25, 0.5, 0.75, 1.0];
    for c in &measured::CONDITIONS {
        let g = measured::model(c).unwrap();
        let mut worst = 0f32;
        for ink in 0..4 {
            for a in coarse {
                for b in coarse {
                    for d in coarse {
                        let mut others = [a, b, d].into_iter();
                        let mut build = [0f32; 4];
                        for (i, slot) in build.iter_mut().enumerate() {
                            if i != ink {
                                *slot = others.next().unwrap();
                            }
                        }
                        let mut last = f32::INFINITY;
                        for &v in &steps {
                            build[ink] = v;
                            let l = g.to_lab_rel(build).l;
                            worst = worst.max(l - last);
                            last = l;
                        }
                    }
                }
            }
        }
        println!("{}: largest L* rise with more ink {worst:.3}", c.reference);
        assert!(worst < 0.3, "{}: {worst}", c.reference);
    }
}

#[test]
fn the_working_spaces_follow_generic_cmyk_in_the_list() {
    let cmyk: Vec<String> = profiles().into_iter().filter(|p| p.kind == ProfileKind::Cmyk && p.builtin).map(|p| p.name).collect();
    assert_eq!(cmyk, [GENERIC_CMYK, FOGRA51_CMYK, FOGRA52_CMYK, DEVICE_CMYK]);
    assert_eq!(ColorSettings::default().cmyk, GENERIC_CMYK);
    assert_eq!(characterization(FOGRA51_CMYK), Some(("FOGRA51", "http://www.color.org")));
    assert_eq!(characterization(FOGRA52_CMYK), Some(("FOGRA52", "http://www.color.org")));
    assert_eq!(characterization(GENERIC_CMYK), None);
    assert!(matches!(register_icc(&icc_bytes(FOGRA51_CMYK).unwrap(), None), Err(CmsError::Unsupported(_))));
}

#[test]
fn colors_separate_and_come_back_in_the_measured_spaces() {
    for name in [FOGRA51_CMYK, FOGRA52_CMYK] {
        let cms = Cms::new(&ColorSettings { cmyk: name.into(), ..ColorSettings::default() }).unwrap();
        for srgb in [[0.8, 0.3, 0.2], [0.2, 0.5, 0.7], [0.5, 0.5, 0.5], [0.9, 0.85, 0.6], [0.1, 0.1, 0.1]] {
            let inks = cms.srgb_to_cmyk(srgb, Intent::RelativeColorimetric);
            assert!(inks.iter().sum::<f32>() <= 3.0 + 1e-3, "{name} {srgb:?}: {inks:?}");
            if cms.gamut_error(&Color::Rgb { r: srgb[0], g: srgb[1], b: srgb[2] }) < GAMUT_THRESHOLD {
                let back = cms.cmyk_to_srgb(inks, false);
                let de = delta_e2000(lab::srgb_to_lab(srgb), lab::srgb_to_lab(back));
                assert!(de < 1.5, "{name} {srgb:?}: {inks:?} -> {back:?} (ΔE {de})");
            }
        }
        // White is paper with no ink, and black is the darkest build within the ink limit.
        assert_eq!(cms.srgb_to_cmyk([1.0; 3], Intent::RelativeColorimetric), [0.0; 4]);
        let black = cms.srgb_to_cmyk([0.0; 3], Intent::RelativeColorimetric);
        assert!(black[3] > 0.8 && black.iter().sum::<f32>() <= 3.0 + 1e-3, "{name}: {black:?}");
    }
    // The coated paper prints a larger gamut than the uncoated one.
    let blue = Color::Rgb { r: 0.1, g: 0.2, b: 0.6 };
    let coated = Cms::new(&ColorSettings { cmyk: FOGRA51_CMYK.into(), ..ColorSettings::default() }).unwrap();
    let uncoated = Cms::new(&ColorSettings { cmyk: FOGRA52_CMYK.into(), ..ColorSettings::default() }).unwrap();
    assert!(coated.gamut_error(&blue) < uncoated.gamut_error(&blue));
}

#[test]
fn the_profiles_carry_the_paper_and_the_data_source() {
    for c in &measured::CONDITIONS {
        let start = std::time::Instant::now();
        let bytes = icc_bytes(c.name).unwrap();
        println!("{}: {} bytes in {:?}", c.reference, bytes.len(), start.elapsed());
        let p = moxcms::ColorProfile::new_from_slice(&bytes).unwrap();
        assert_eq!(
            (p.profile_class, p.color_space, p.pcs),
            (moxcms::ProfileClass::OutputDevice, moxcms::DataColorSpace::Cmyk, moxcms::DataColorSpace::Lab)
        );
        assert!(p.lut_a_to_b_perceptual.is_some() && p.lut_b_to_a_perceptual.is_some() && p.lut_a_to_b_colorimetric.is_some());
        let paper = lab::lab_to_xyz(measured::parse(c.data).unwrap()[0].lab);
        let white = p.media_white_point.unwrap();
        for (got, want) in [white.x, white.y, white.z].iter().zip(paper) {
            assert!((*got - f64::from(want)).abs() < 2e-3, "{}: {white:?} vs {paper:?}", c.reference);
        }
        let text = format!("{:?}", p.copyright);
        assert!(text.contains(c.reference) && text.contains("Fogra"), "{text}");
    }
}
