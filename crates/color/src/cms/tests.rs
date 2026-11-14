use super::*;

fn cms() -> Cms {
    Cms::new(&ColorSettings::default()).unwrap()
}

/// Without black-point compensation, relative colorimetric reproduces in-gamut colours exactly.
fn cms_no_bpc() -> Cms {
    Cms::new(&ColorSettings { bpc: false, ..Default::default() }).unwrap()
}

fn de(a: [f32; 3], b: [f32; 3]) -> f32 {
    delta_e2000(lab::srgb_to_lab(a), lab::srgb_to_lab(b))
}

#[test]
fn intents_parse_and_name() {
    for i in Intent::ALL {
        assert_eq!(Intent::parse(i.id()), Some(i));
        assert_eq!(Intent::parse(i.label()), Some(i));
    }
    assert_eq!(Intent::parse("bogus"), None);
}

#[test]
fn srgb_via_moxcms_matches_analytic() {
    // The built-in sRGB ICC profile through moxcms agrees with our analytic sRGB maths.
    let p = icc::builtin_rgb(SRGB).unwrap();
    for rgb in [[0.2, 0.5, 0.8], [1.0, 0.0, 0.0], [0.5, 0.5, 0.5]] {
        let out = p.to_srgb(&rgb, Intent::RelativeColorimetric).unwrap();
        assert!(de(out, rgb) < 0.5, "{rgb:?} → {out:?}");
    }
}

#[test]
fn paper_white_and_rich_black() {
    let c = cms();
    assert_eq!(c.srgb_to_cmyk([1.0, 1.0, 1.0], Intent::RelativeColorimetric), [0.0; 4]);
    let k = c.srgb_to_cmyk([0.0, 0.0, 0.0], Intent::RelativeColorimetric);
    assert!(k[3] > 0.8, "black generation puts most of the black in K: {k:?}");
    assert!(k.iter().sum::<f32>() <= 3.0 + 1e-3, "TAC limit: {k:?}");
    assert!(k[0] > 0.3 && k[1] > 0.3 && k[2] > 0.3, "rich black: {k:?}");
    assert_eq!(c.cmyk_to_srgb([0.0; 4], false), [1.0, 1.0, 1.0]);
}

#[test]
fn rgb_cmyk_rgb_roundtrip_in_gamut() {
    let c = cms_no_bpc();
    for rgb in [[0.8, 0.6, 0.5], [0.5, 0.5, 0.5], [0.3, 0.45, 0.35], [0.9, 0.85, 0.3], [0.55, 0.4, 0.6], [0.35, 0.55, 0.7], [0.75, 0.75, 0.75]] {
        let cmyk = c.srgb_to_cmyk(rgb, Intent::RelativeColorimetric);
        let back = c.cmyk_to_srgb(cmyk, false);
        assert!(de(rgb, back) < 1.5, "{rgb:?} → {cmyk:?} → {back:?} ΔE {}", de(rgb, back));
    }
}

#[test]
fn cmyk_lab_cmyk_roundtrip() {
    let c = cms_no_bpc();
    for cmyk in [[0.2, 0.4, 0.6, 0.1], [0.0, 0.0, 0.0, 0.5], [0.5, 0.1, 0.0, 0.0], [0.1, 0.7, 0.3, 0.2]] {
        let l = c.cmyk_to_lab(cmyk);
        // Lab here is media-relative, i.e. what relative colorimetric reproduces exactly.
        let back = c.lab_to_cmyk(l, Intent::RelativeColorimetric);
        let l2 = c.cmyk_to_lab(back);
        assert!(delta_e2000(l, l2) < 1.0, "{cmyk:?} → {l:?} → {back:?} → {l2:?}");
    }
}

#[test]
fn gcr_puts_grey_into_black() {
    let k = cms().srgb_to_cmyk([0.4, 0.4, 0.4], Intent::RelativeColorimetric);
    assert!(k[3] > 0.2, "{k:?}");
    let spread = k[0].max(k[1]).max(k[2]) - k[0].min(k[1]).min(k[2]);
    assert!(spread < 0.2, "neutral stays balanced: {k:?}");
}

#[test]
fn gamut_warning() {
    let c = cms();
    assert!(c.out_of_gamut(&Color::rgb(0.0, 0.0, 1.0)), "sRGB blue is outside SWOP-like CMYK");
    assert!(c.out_of_gamut(&Color::rgb(0.0, 1.0, 0.0)), "sRGB green is outside");
    assert!(!c.out_of_gamut(&Color::rgb(0.8, 0.6, 0.5)), "skin tone is printable");
    assert!(!c.out_of_gamut(&Color::rgb(0.5, 0.5, 0.5)));
    assert!(!c.out_of_gamut(&Color::cmyk(1.0, 0.0, 0.0, 0.0)), "CMYK colours are in gamut by definition");
    // The uncalibrated device space has no gamut limits.
    let dev = Cms::new(&ColorSettings { cmyk: DEVICE_CMYK.into(), ..Default::default() }).unwrap();
    assert!(!dev.out_of_gamut(&Color::rgb(0.0, 0.0, 1.0)));
}

#[test]
fn intents_differ() {
    let c = cms();
    let blue = [0.0, 0.0, 1.0];
    let res: Vec<[f32; 4]> = Intent::ALL.iter().map(|i| c.srgb_to_cmyk(blue, *i)).collect();
    let dist = |a: [f32; 4], b: [f32; 4]| (0..4).map(|i| (a[i] - b[i]).abs()).sum::<f32>();
    assert!(dist(res[0], res[1]) > 0.02, "perceptual vs relative: {res:?}");
    assert!(dist(res[2], res[1]) > 0.02, "saturation vs relative: {res:?}");
    // Absolute colorimetry keeps the source white brighter than paper → less ink on light greys.
    let g = [0.88, 0.88, 0.88];
    let rel = c.srgb_to_cmyk(g, Intent::RelativeColorimetric);
    let abs = c.srgb_to_cmyk(g, Intent::AbsoluteColorimetric);
    assert!(rel.iter().sum::<f32>() > abs.iter().sum::<f32>() + 0.02, "{rel:?} vs {abs:?}");
    // Perceptual compresses in-gamut saturated colours too (relative leaves them alone).
    let red = lab::srgb_to_lab([0.8, 0.25, 0.25]);
    let pr = c.cmyk_to_lab(c.srgb_to_cmyk([0.8, 0.25, 0.25], Intent::Perceptual));
    let rr = c.cmyk_to_lab(c.srgb_to_cmyk([0.8, 0.25, 0.25], Intent::RelativeColorimetric));
    assert!(delta_e2000(rr, red) < delta_e2000(pr, red), "relative is closer for in-gamut colours");
}

#[test]
fn simulate_paper_tints_white() {
    let c = cms();
    let rel = c.cmyk_to_srgb([0.0; 4], false);
    let abs = c.cmyk_to_srgb([0.0; 4], true);
    assert_eq!(rel, [1.0; 3]);
    assert!(abs[0] < 0.98 && abs[2] < abs[0], "paper is darker and slightly warm: {abs:?}");
}

#[test]
fn device_cmyk_keeps_legacy_numbers() {
    let dev = Cms::new(&ColorSettings { cmyk: DEVICE_CMYK.into(), ..Default::default() }).unwrap();
    assert_eq!(dev.cmyk_to_srgb([1.0, 0.0, 0.0, 0.0], false), [0.0, 1.0, 1.0]);
    assert_eq!(dev.srgb_to_cmyk([1.0, 0.0, 0.0], Intent::Perceptual), [0.0, 1.0, 1.0, 0.0]);
}

#[test]
fn convert_between_models() {
    let c = cms();
    let red = Color::rgb(0.9, 0.2, 0.2);
    let cmyk = c.convert(&red, Model::Cmyk, Intent::RelativeColorimetric);
    assert!(matches!(cmyk, Color::Cmyk { .. }));
    assert_eq!(c.convert(&cmyk, Model::Cmyk, Intent::Perceptual), cmyk, "idempotent");
    let back = c.convert(&cmyk, Model::Rgb, Intent::RelativeColorimetric);
    assert!(de(back.to_rgb_uncalibrated(), [0.9, 0.2, 0.2]) < 6.0, "{back:?}");
    let g = c.convert(&Color::rgb(0.5, 0.5, 0.5), Model::Gray, Intent::RelativeColorimetric);
    let Color::Gray { k } = g else { panic!() };
    assert!((k - 0.5).abs() < 0.01, "{k}");
}

#[test]
fn wide_gamut_working_space() {
    let c = Cms::new(&ColorSettings { rgb: WIDE_GAMUT_RGB.into(), ..Default::default() }).unwrap();
    assert!(!c.rgb_is_srgb());
    let grey = c.rgb_to_srgb([0.5, 0.5, 0.5]);
    assert!(de(grey, [0.5, 0.5, 0.5]) < 1.5, "{grey:?}");
    // Wide-gamut green is outside sRGB: it clips to (nearly) full sRGB green.
    let g = c.rgb_to_srgb([0.0, 1.0, 0.0]);
    assert!(g[1] > 0.95 && g[0] < 0.1, "{g:?}");
    let back = c.srgb_to_rgb(c.rgb_to_srgb([0.4, 0.5, 0.3]));
    assert!(de(back, [0.4, 0.5, 0.3]) < 1.0);
    assert!(matches!(Cms::new(&ColorSettings { rgb: "Nope".into(), ..Default::default() }), Err(CmsError::UnknownProfile(_))));
}

#[test]
fn legacy_profile_name_resolves() {
    let (old, _) = LEGACY_NAMES[0];
    assert_eq!(canonical_name(old), WIDE_GAMUT_RGB);
    assert_eq!(canonical_name(SRGB), SRGB);
    assert_eq!(profile(old).map(|p| (p.name, p.kind, p.builtin)), Some((WIDE_GAMUT_RGB.to_string(), ProfileKind::Rgb, true)));
    assert!(profile("Nope").is_none());
    assert!(profiles().iter().all(|p| p.name != old), "only the current name is listed");
    // Settings saved with the old name load as the current profile, under its current name.
    let c = Cms::new(&ColorSettings { rgb: old.into(), ..Default::default() }).unwrap();
    assert_eq!(c.settings().rgb, WIDE_GAMUT_RGB);
    let same = Cms::new(&ColorSettings { rgb: WIDE_GAMUT_RGB.into(), ..Default::default() }).unwrap();
    assert_eq!(c.rgb_to_srgb([0.0, 1.0, 0.0]), same.rgb_to_srgb([0.0, 1.0, 0.0]));
}

#[test]
fn proof_lut_matches_direct_proof() {
    let c = cms();
    let p = ProofSetup::default();
    let lut = c.proof_lut(&p);
    for rgb in [[0.8, 0.6, 0.5], [0.0, 0.0, 1.0], [0.3, 0.9, 0.2]] {
        let direct = c.proof_srgb(rgb, &p);
        let via_lut = lut.apply(rgb);
        assert!(de(direct, via_lut) < 2.5, "{rgb:?}: {direct:?} vs {via_lut:?}");
    }
    // Out-of-gamut blue gets visibly duller in the proof.
    let b = c.proof_srgb([0.0, 0.0, 1.0], &p);
    assert!(de(b, [0.0, 0.0, 1.0]) > 5.0, "{b:?}");
}

#[test]
fn colour_blindness_proofs() {
    let c = cms();
    let p = ProofSetup { target: ProofTarget::Protanopia, ..Default::default() };
    let red = c.proof_srgb([0.9, 0.1, 0.1], &p);
    let green = c.proof_srgb([0.1, 0.6, 0.1], &p);
    assert!(de(red, green) < de([0.9, 0.1, 0.1], [0.1, 0.6, 0.1]) * 0.5, "red/green confusion");
    let d = ProofSetup { target: ProofTarget::Deuteranopia, ..Default::default() };
    assert_ne!(c.proof_srgb([0.9, 0.1, 0.1], &d), [0.9, 0.1, 0.1]);
    let m = ProofSetup { target: ProofTarget::MonitorRgb, ..Default::default() };
    assert_eq!(c.proof_srgb([0.2, 0.3, 0.4], &m), [0.2, 0.3, 0.4]);
    assert_eq!(ProofTarget::parse(&ProofTarget::Cmyk("X".into()).id()), Some(ProofTarget::Cmyk("X".into())));
}

/// The expected colors were computed separately with NumPy from the same published inputs (linear
/// sRGB to XYZ from IEC 61966-2-1, Smith & Pokorny's XYZ to LMS, Judd-Vos XYZ at 485 nm and 660 nm),
/// following Brettel, Viénot & Mollon (1997) with the display's white as the neutral axis.
#[test]
fn tritanopia_proof_follows_brettel_1997() {
    let c = cms();
    let t = ProofSetup { target: ProofTarget::Tritanopia, ..Default::default() };
    assert_eq!(ProofTarget::parse("tritanopia"), Some(ProofTarget::Tritanopia));
    assert_eq!(ProofTarget::Tritanopia.id(), "tritanopia");
    for g in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let out = c.proof_srgb([g; 3], &t);
        assert!(out.iter().all(|v| (v - g).abs() < 1e-4), "gray {g} stays gray: {out:?}");
    }
    let cases = [
        // The 660 nm half-plane.
        ([1.0, 0.0, 0.0], [1.0, 0.0, 0.299_231]),
        ([1.0, 0.0, 1.0], [0.934_206, 0.388_351, 0.467_178]),
        ([0.9, 0.6, 0.1], [0.926_825, 0.557_667, 0.599_649]),
        // The 485 nm half-plane.
        ([0.0, 0.0, 1.0], [0.0, 0.377_268, 0.519_933]),
        ([0.0, 1.0, 0.0], [0.480_503, 0.920_568, 1.0]),
        ([0.2, 0.4, 0.8], [0.0, 0.464_73, 0.563_516]),
    ];
    for (rgb, want) in cases {
        let got = c.proof_srgb(rgb, &t);
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-3), "{rgb:?}: {got:?}, expected {want:?}");
    }
}

#[test]
fn bad_icc_rejected() {
    assert!(matches!(register_icc(b"not a profile", None), Err(CmsError::BadProfile(_))));
}

#[cfg(target_os = "macos")]
#[test]
fn loads_system_cmyk_icc() {
    let path = std::path::Path::new("/System/Library/ColorSync/Profiles/Generic CMYK Profile.icc");
    if !path.exists() {
        return;
    }
    let info = load_icc_file(path).unwrap();
    assert_eq!(info.kind, ProfileKind::Cmyk);
    assert!(profiles().iter().any(|p| p.name == info.name && !p.builtin));
    let c = Cms::new(&ColorSettings { cmyk: info.name.clone(), ..Default::default() }).unwrap();
    let rgb = [0.7, 0.5, 0.4];
    let back = c.cmyk_to_srgb(c.srgb_to_cmyk(rgb, Intent::RelativeColorimetric), false);
    assert!(de(rgb, back) < 5.0, "{back:?}");
    assert_eq!(c.cmyk_to_srgb([0.0; 4], false).map(|v| (v * 255.0).round() as u8), [255, 255, 255]);
}

/// With black-point compensation, sRGB black separates to the press's black point and that black
/// displays as black again (hex 000000 -> C75 M68 Y67 K90 -> 000000, not a dark gray).
#[test]
fn generic_cmyk_black_round_trips_with_bpc() {
    let c = cms();
    let cmyk = c.srgb_to_cmyk([0.0; 3], Intent::RelativeColorimetric);
    let back = c.cmyk_to_srgb(cmyk, false).map(|v| (v * 255.0).round() as u8);
    assert_eq!(back, [0, 0, 0], "{cmyk:?}");
    // The paper still shows as white, and without compensation the press black stays a dark grey.
    assert_eq!(c.cmyk_to_srgb([0.0; 4], false), [1.0, 1.0, 1.0]);
    let raw = cms_no_bpc().cmyk_to_srgb(cmyk, false);
    assert!(raw.iter().all(|v| *v > 0.05), "{raw:?}");
}

/// Each ink's 40% and 50% tints gain like CGATS TR 001 (SWOP): the colorimetric tone value increase
/// (Murray–Davies on X for cyan, Y for magenta and black, Z for yellow) is C 20.0, M 17.6, Y 16.8 and
/// K 21.6 points at 50%, and C 20.0, M 17.3, Y 16.7 and K 21.9 points at 40%.
#[test]
fn generic_cmyk_tints_gain_like_tr001() {
    let g = generic();
    let tvi = |ink: usize, a: f32| {
        let ch = [0, 1, 2, 1][ink];
        let mut cmyk = [0.0f32; 4];
        cmyk[ink] = a;
        let tint = g.to_xyz_rel(cmyk)[ch];
        cmyk[ink] = 1.0;
        let solid = g.to_xyz_rel(cmyk)[ch];
        let paper = g.to_xyz_rel([0.0; 4])[ch];
        (paper - tint) / (paper - solid) - a
    };
    for (ink, (at50, at40)) in [(0.200, 0.200), (0.176, 0.173), (0.168, 0.167), (0.216, 0.219)].into_iter().enumerate() {
        assert!((tvi(ink, 0.5) - at50).abs() < 0.002, "ink {ink} at 50%: {}", tvi(ink, 0.5));
        assert!((tvi(ink, 0.4) - at40).abs() < 0.002, "ink {ink} at 40%: {}", tvi(ink, 0.4));
    }
}

/// More of any ink never makes a Generic CMYK color lighter, in any XYZ channel: on a 10% grid of
/// builds within the 300% ink limit, adding 10% of one ink keeps X, Y and Z or lowers them.
#[test]
fn generic_cmyk_more_ink_is_never_lighter() {
    let g = generic();
    let steps = (0..=10).map(|i| i as f32 / 10.0);
    for c in steps.clone() {
        for m in steps.clone() {
            for y in steps.clone() {
                for k in steps.clone() {
                    let base = [c, m, y, k];
                    if base.iter().sum::<f32>() > 3.0 - 0.1 + 1e-4 {
                        continue;
                    }
                    let x0 = g.to_xyz_rel(base);
                    for ink in 0..4 {
                        if base[ink] >= 1.0 {
                            continue;
                        }
                        let mut more = base;
                        more[ink] += 0.1;
                        let x1 = g.to_xyz_rel(more);
                        assert!((0..3).all(|ch| x1[ch] <= x0[ch] + 1e-5), "{base:?} plus ink {ink}: {x0:?} to {x1:?}");
                    }
                }
            }
        }
    }
}

/// sRGB black separates to 90% black under neutral CMY that fills the 300% ink limit, light grays
/// carry no black, and black rises steadily from the light end of the gray ramp to the dark end.
#[test]
fn generic_cmyk_black_generation_follows_the_black_curve() {
    let c = cms();
    let black = c.srgb_to_cmyk([0.0; 3], Intent::RelativeColorimetric);
    assert!((black[3] - 0.9).abs() < 0.005, "{black:?}");
    assert!((black.iter().sum::<f32>() - 3.0).abs() < 0.01, "{black:?}");
    let lab = c.cmyk_to_lab(black);
    assert!(lab.a.abs() < 0.05 && lab.b.abs() < 0.05, "neutral: {lab:?}");
    let light = c.srgb_to_cmyk([0.7; 3], Intent::RelativeColorimetric);
    assert_eq!(light[3], 0.0, "{light:?}");
    let mut prev = 0.0;
    for v in (0..=50).rev().map(|i| i as f32 / 50.0) {
        let k = c.srgb_to_cmyk([v; 3], Intent::RelativeColorimetric)[3];
        assert!(k >= prev - 1e-4, "gray {v}: K {k} after {prev}");
        prev = k;
    }
}

/// A color whose solve holds an ink at a bound separates to the same inks when its values change by
/// a few float steps: hex f01482 (cyan held at 0) and hex ed1e79 (magenta held at 100%), each with
/// its red stepped one `f32` at a time over 4,001 values (about 0.00024 in all), stay within 0.001
/// on every ink.
#[test]
fn generic_cmyk_separates_nearby_colors_at_the_gamut_edge_alike() {
    let c = cms();
    let spread = |pink: [f32; 3]| {
        let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
        for step in -2000i32..=2000 {
            let mut rgb = pink;
            rgb[0] = f32::from_bits(pink[0].to_bits().wrapping_add_signed(step));
            let ink = c.srgb_to_cmyk(rgb, Intent::RelativeColorimetric);
            for i in 0..4 {
                lo[i] = lo[i].min(ink[i]);
                hi[i] = hi[i].max(ink[i]);
            }
        }
        (lo, hi)
    };
    let (lo, hi) = spread([240.0f32 / 255.0, 20.0 / 255.0, 130.0 / 255.0]);
    assert!((0..4).all(|i| hi[i] - lo[i] < 1e-3), "{lo:?} to {hi:?}");
    assert_eq!(lo[0], 0.0, "{lo:?}");
    let (lo, hi) = spread([237.0f32 / 255.0, 30.0 / 255.0, 121.0 / 255.0]);
    assert!((0..4).all(|i| hi[i] - lo[i] < 1e-3), "{lo:?} to {hi:?}");
    assert_eq!(hi[1], 1.0, "{hi:?}");
}

/// A color whose solve holds cyan and magenta at 1 separates to the same inks when its values change
/// by a few float steps: sRGB (0, 0, 0.25) with its blue stepped one `f32` at a time over 121 values
/// stays within 0.001 on every ink.
#[test]
fn generic_cmyk_separates_nearby_colors_with_inks_at_full_alike() {
    let c = cms();
    let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
    for step in -60i32..=60 {
        let blue = f32::from_bits(0.25f32.to_bits().wrapping_add_signed(step));
        let ink = c.srgb_to_cmyk([0.0, 0.0, blue], Intent::RelativeColorimetric);
        for i in 0..4 {
            lo[i] = lo[i].min(ink[i]);
            hi[i] = hi[i].max(ink[i]);
        }
    }
    assert!((0..4).all(|i| hi[i] - lo[i] < 1e-3), "{lo:?} to {hi:?}");
    assert_eq!((lo[0], lo[1]), (1.0, 1.0), "{lo:?}");
}

#[test]
fn lab_colours_round_trip_through_the_cms() {
    let c = cms_no_bpc();
    let ri = Intent::RelativeColorimetric;
    // In-gamut Lab colours separate into working CMYK and come back within a small ΔE.
    for (l, a, b) in [(60.0, 20.0, 30.0), (50.0, -20.0, -10.0), (75.0, 5.0, 40.0), (40.0, 30.0, -25.0)] {
        let lab = Color::lab(l, a, b);
        assert_eq!(c.lab(&lab), Lab::new(l, a, b), "Lab passes through");
        assert_eq!(c.convert(&lab, Model::Lab, ri), lab, "already Lab: unchanged");
        let ink = c.convert(&lab, Model::Cmyk, ri);
        let Color::Cmyk { c: cc, m, y, k } = ink else { panic!("CMYK") };
        assert_eq!([cc, m, y, k], c.to_cmyk(&lab, ri), "separation = conversion");
        let back = c.lab(&ink);
        assert!(delta_e2000(back, Lab::new(l, a, b)) < 1.5, "{l} {a} {b} → {ink:?} → {back:?}");
        assert!(!c.out_of_gamut(&lab), "{l} {a} {b} is printable");
        // Through RGB and back to Lab.
        let rgb = c.convert(&lab, Model::Rgb, ri);
        let Color::Lab { l: l2, a: a2, b: b2 } = c.convert(&rgb, Model::Lab, ri) else { panic!("Lab") };
        assert!(delta_e76(Lab::new(l2, a2, b2), Lab::new(l, a, b)) < 0.05, "{l} {a} {b} → {rgb:?} → {l2} {a2} {b2}");
    }
    // A saturated Lab blue no press reaches is out of gamut; Lab grey converts to a neutral.
    assert!(c.out_of_gamut(&Color::lab(30.0, 60.0, -100.0)));
    let Color::Gray { k } = c.convert(&Color::lab(50.0, 0.0, 0.0), Model::Gray, ri) else { panic!("Gray") };
    assert!((k - 0.53).abs() < 0.01, "{k}");
}

#[test]
fn with_active_puts_settings_in_place_on_this_thread_only() {
    let p3 = Arc::new(Cms::new(&ColorSettings { rgb: DISPLAY_P3.into(), cmyk: DEVICE_CMYK.into(), ..Default::default() }).unwrap());
    let (rgb, ink) = (Color::Rgb { r: 0.2, g: 0.6, b: 0.4 }, Color::Cmyk { c: 0.2, m: 0.3, y: 0.4, k: 0.1 });
    let before = (rgb.to_rgb(), ink.to_rgb());
    let (inside, ink_inside, other_thread, nested) = with_active(p3.clone(), || {
        let other = std::thread::spawn(move || rgb.to_rgb()).join().unwrap();
        let nested = with_active(Arc::new(cms()), || rgb.to_rgb());
        (rgb.to_rgb(), ink.to_rgb(), other, nested)
    });
    assert_eq!(inside, p3.display_rgb(&rgb), "RGB is drawn through the settings given");
    assert_ne!(inside, before.0);
    assert_eq!(ink_inside, naive_cmyk_to_rgb([0.2, 0.3, 0.4, 0.1]), "CMYK takes the device formula of the settings given");
    assert_eq!(other_thread, before.0, "another thread keeps the active settings");
    assert_eq!(nested, before.0, "an inner call puts its own settings in place");
    assert_eq!((rgb.to_rgb(), ink.to_rgb()), before, "the active settings come back after");
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| with_active(p3.clone(), || panic!("unwinds"))));
    assert!(unwound.is_err());
    assert_eq!((rgb.to_rgb(), ink.to_rgb()), before, "the active settings come back after a panic");
}

#[test]
fn a_document_gets_the_same_context_for_the_same_profiles() {
    // Other tests load profiles meanwhile, and a load makes the contexts again: try until one pair
    // of calls sees none.
    let same = (0..100).any(|_| {
        let generation = profiles_generation();
        let (a, b) = (for_document(Some(DISPLAY_P3), None), for_document(Some(DISPLAY_P3), None));
        profiles_generation() == generation && Arc::ptr_eq(&a, &b)
    });
    assert!(same, "the same profiles give the same context");
    let p3 = for_document(Some(DISPLAY_P3), None);
    assert_eq!(p3.settings().rgb, DISPLAY_P3);
    assert_eq!(p3.settings().cmyk, working_settings().cmyk, "the working CMYK space stays");
    assert_eq!(for_document(None, None).settings(), &working_settings(), "no profiles: the working settings");
    let unknown = for_document(Some("no such profile"), Some(SRGB));
    assert_eq!(unknown.settings(), &working_settings(), "an unknown profile or one of the other model leaves the working space");
}
