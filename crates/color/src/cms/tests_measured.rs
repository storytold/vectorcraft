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
    // The rows end with END_DATA, and a NUMBER_OF_SETS in the header counts them.
    let rows = "1 0 0 0 0 95 1 -6\n2 100 0 0 0 55 -37 -50";
    let header = |sets: &str| format!("ISO28178\nBEGIN_DATA_FORMAT\n{fields}\nEND_DATA_FORMAT\n{sets}\nBEGIN_DATA\n{rows}\n");
    assert_eq!(measured::parse(header("NUMBER_OF_SETS\t2").as_bytes()), None, "no END_DATA");
    for (sets, len) in [
        ("NUMBER_OF_SETS\t2", Some(2)),
        ("NUMBER_OF_SETS\t3", None),
        ("NUMBER_OF_SETS\t1", None),
        ("NUMBER_OF_SETS\tx", None),
        ("NUMBER_OF_SETS", None),
    ] {
        assert_eq!(measured::parse(format!("{}END_DATA\n", header(sets)).as_bytes()).map(|p| p.len()), len, "{sets}");
    }
}

/// A paper measured as black has no XYZ to make the other patches relative to.
#[test]
fn a_paper_without_light_is_refused() {
    let c = &measured::CONDITIONS[0];
    let mut patches = measured::parse(c.data).unwrap();
    for p in patches.iter_mut().filter(|p| p.cmyk == [0.0; 4]) {
        p.lab = Lab::new(0.0, 0.0, 0.0);
    }
    assert!(measured::fit(c, &patches, measured::SMOOTHING).is_none());
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
/// predicts the patches it left out. The data repeats some builds, and every row of a build is
/// on the side of the build's first row, so the fit never sees a left-out build.
#[test]
fn the_fit_predicts_patches_it_left_out() {
    for c in &measured::CONDITIONS {
        let patches = measured::parse(c.data).unwrap();
        let corner = |p: &measured::Patch| p.cmyk.iter().all(|v| *v == 0.0 || *v == 1.0);
        let first = |p: &measured::Patch| patches.iter().position(|q| q.cmyk == p.cmyk).unwrap_or(0);
        let (kept, left): (Vec<&measured::Patch>, Vec<_>) = patches.iter().partition(|p| first(p) % 4 != 3 || corner(p));
        assert!(left.iter().all(|l| kept.iter().all(|k| k.cmyk != l.cmyk)), "{}", c.reference);
        let kept: Vec<measured::Patch> = kept.into_iter().copied().collect();
        let left: Vec<measured::Patch> = left.into_iter().copied().collect();
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
        // A 50% gray (L* 53) lies well inside both measured gamuts (the darkest patches measure
        // L* 11.43 and 26.36), so it comes back whatever the gamut check says.
        let gray = [0.5, 0.5, 0.5];
        for srgb in [[0.8, 0.3, 0.2], [0.2, 0.5, 0.7], gray, [0.9, 0.85, 0.6], [0.1, 0.1, 0.1]] {
            let inks = cms.srgb_to_cmyk(srgb, Intent::RelativeColorimetric);
            assert!(inks.iter().sum::<f32>() <= 3.0 + 1e-3, "{name} {srgb:?}: {inks:?}");
            if srgb == gray || cms.gamut_error(&Color::Rgb { r: srgb[0], g: srgb[1], b: srgb[2] }) < GAMUT_THRESHOLD {
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

/// Absolute Lab of `cmyk` in a context whose CMYK space is one of the Generic-type spaces.
fn absolute_lab(cms: &Cms, cmyk: [f32; 4]) -> Lab {
    match &cms.cmyk {
        CmykSpace::Generic(g) => g.to_lab_abs(cmyk),
        CmykSpace::Device | CmykSpace::Icc(_) => panic!("not a Generic-type CMYK space"),
    }
}

#[test]
fn absolute_cmyk_to_rgb_shows_the_paper() {
    for name in [GENERIC_CMYK, FOGRA51_CMYK, FOGRA52_CMYK] {
        let cms = Cms::new(&ColorSettings { cmyk: name.into(), ..ColorSettings::default() }).unwrap();
        let paper = cms.cmyk_to_rgb([0.0; 4], Intent::AbsoluteColorimetric);
        assert_eq!(paper, cms.srgb_to_rgb(cms.cmyk_to_srgb([0.0; 4], true)), "{name}");
        assert_ne!(paper, cms.cmyk_to_rgb([0.0; 4], Intent::RelativeColorimetric), "{name}");
    }
}

#[test]
fn absolute_cmyk_conversion_keeps_the_source_paper() {
    // Generic CMYK's paper (L* 93, b* 2.5) is darker and yellower than FOGRA51's (L* 95, b* -6),
    // so printing it on FOGRA51 paper takes ink.
    let generic = Cms::new(&ColorSettings { cmyk: GENERIC_CMYK.into(), ..ColorSettings::default() }).unwrap();
    let coated = Cms::new(&ColorSettings { cmyk: FOGRA51_CMYK.into(), ..ColorSettings::default() }).unwrap();
    let Color::Cmyk { c, m, y, k } = generic.convert_into(&coated, &Color::Cmyk { c: 0.0, m: 0.0, y: 0.0, k: 0.0 }, Intent::AbsoluteColorimetric)
    else {
        panic!("a CMYK colour converts to CMYK");
    };
    let inks = [c, m, y, k];
    assert!(inks.iter().sum::<f32>() > 0.01, "{inks:?}");
    let de = delta_e2000(absolute_lab(&coated, inks), absolute_lab(&generic, [0.0; 4]));
    assert!(de < 2.0, "{inks:?}: ΔE {de}");
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
