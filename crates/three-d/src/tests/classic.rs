use crate::*;

const TOWARD: [f64; 3] = [0.0, 0.0, 1.0];

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn one(light: [f64; 3], intensity: f64) -> [([f64; 3], f64); 1] {
    [(light, intensity)]
}

#[test]
fn wireframe_and_no_shading_leave_every_face_unlit() {
    for surface in [ClassicSurface::Wireframe, ClassicSurface::NoShading] {
        let s = ClassicShading { surface, ..ClassicShading::default() };
        for normal in [TOWARD, [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]] {
            assert_eq!(s.tone(normal, &one(TOWARD, 1.0)), (1.0, 0.0), "{surface:?}");
        }
    }
}

#[test]
fn diffuse_adds_the_light_to_the_ambient_and_has_no_highlight() {
    let s = ClassicShading { surface: ClassicSurface::Diffuse, ambient: 0.3, blend_steps: 256, ..ClassicShading::default() };
    let (lit, spec) = s.tone(TOWARD, &one(TOWARD, 0.4));
    assert!((lit - 0.7).abs() < 1.0 / 256.0, "{lit}");
    assert_eq!(spec, 0.0);
    // A face turned away from the light gets the ambient light only.
    let (away, _) = s.tone(TOWARD, &one([1.0, 0.0, 0.0], 0.4));
    assert!((away - 0.3).abs() < 1.0 / 256.0, "{away}");
    // A face at 60° to the light gets half of it.
    let (half, _) = s.tone(TOWARD, &one([3f64.sqrt() / 2.0, 0.0, 0.5], 0.4));
    assert!((half - 0.5).abs() < 1.0 / 256.0, "{half}");
    // The tone stays within full shadow and full light.
    assert_eq!(ClassicShading { ambient: 1.0, ..s }.tone(TOWARD, &one(TOWARD, 1.0)).0, 1.0);
    // Without lights a face gets the ambient light only.
    assert!((s.tone(TOWARD, &[]).0 - 0.3).abs() < 1.0 / 256.0);
}

#[test]
fn lights_add_up() {
    let s = ClassicShading { surface: ClassicSurface::Diffuse, ambient: 0.0, blend_steps: 256, ..ClassicShading::default() };
    let left = [-0.6, 0.0, 0.8];
    let right = [0.6, 0.0, 0.8];
    let both = s.tone(TOWARD, &[(left, 0.3), (right, 0.3)]).0;
    let single = s.tone(TOWARD, &one(left, 0.3)).0;
    assert!((both - 2.0 * single).abs() < 2.0 / 256.0, "{both} {single}");
    // A light behind the object lights the faces turned away from the viewer only.
    assert_eq!(s.tone(TOWARD, &one([0.0, 0.0, -1.0], 1.0)).0, 0.0);
    assert!(s.tone([0.0, 0.0, -1.0], &one([0.0, 0.0, -1.0], 1.0)).0 > 0.99);
}

#[test]
fn plastic_highlight_peaks_where_a_face_mirrors_the_light_to_the_eye() {
    let s = ClassicShading { highlight_intensity: 0.8, blend_steps: 256, ..ClassicShading::default() };
    let light = one([0.6, 0.0, 0.8], 1.0);
    // The face halfway between the light and the eye mirrors the light straight to the viewer.
    let length = 0.6f64.hypot(1.8);
    let half = [0.6 / length, 0.0, 1.8 / length];
    let (_, peak) = s.tone(half, &light);
    assert!((peak - 0.8).abs() < 1.0 / 256.0, "{peak}");
    let (_, aside) = s.tone([-0.6, 0.0, 0.8], &light);
    assert!(aside < peak / 4.0, "{aside}");
    // A smaller highlight falls off faster.
    let tilted = [0.3, 0.0, 0.954];
    let broad = s.tone(tilted, &light).1;
    let narrow = ClassicShading { highlight_size: 0.1, ..s }.tone(tilted, &light).1;
    assert!(narrow < broad, "{narrow} {broad}");
    // No highlight on a face the light does not reach, without Highlight Intensity, or on Diffuse.
    assert_eq!(s.tone(TOWARD, &one([0.0, 0.0, -1.0], 1.0)).1, 0.0);
    assert_eq!(ClassicShading { highlight_intensity: 0.0, ..s }.tone(half, &light).1, 0.0);
    assert_eq!(ClassicShading { surface: ClassicSurface::Diffuse, ..s }.tone(half, &light).1, 0.0);
}

#[test]
fn blend_steps_round_the_shading_to_that_many_levels() {
    let s = ClassicShading { surface: ClassicSurface::Diffuse, ambient: 0.0, blend_steps: 2, ..ClassicShading::default() };
    let tones: Vec<f64> = (0..=20).map(|k| f64::from(k) / 20.0).map(|c| s.tone(TOWARD, &one([(1.0 - c * c).sqrt(), 0.0, c], 0.5)).0).collect();
    assert!(tones.iter().all(|t| close(*t, 0.0) || close(*t, 0.5)), "{tones:?}");
    assert!(tones.iter().any(|t| close(*t, 0.0)) && tones.iter().any(|t| close(*t, 0.5)), "{tones:?}");
    // Out-of-range step counts and intensities are kept within their ranges.
    for steps in [0, 100_000] {
        let t = ClassicShading { blend_steps: steps, ..s }.tone(TOWARD, &one(TOWARD, f64::NAN)).0;
        assert!((0.0..=1.0).contains(&t));
    }
}

#[test]
fn surfaces_round_trip_through_their_ids() {
    for surface in ClassicSurface::ALL {
        assert_eq!(ClassicSurface::parse(surface.id()), Some(surface));
    }
    assert_eq!(ClassicSurface::parse("glossy"), None);
    assert_eq!(ClassicSurface::default(), ClassicSurface::Plastic);
}
