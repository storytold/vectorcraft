use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use vectorcraft_doc::{ImageBlob, ImageObject, Node, NodeId, NodeKind, OpacityMask};
use vectorcraft_geom::{Affine, PathData, Point, SubPath};
use vectorcraft_trace::Raster;

use super::offline::{self, ID};
use super::*;
use crate::Session;
use crate::cmd::conceptvector::{ConceptRun, GAP, KEEP_JOBS, PartRun};

/// Shapes in four colors on white: a red disc, a blue bar, a green square and a thin black line.
fn art() -> Raster {
    Raster::from_fn(120, 90, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        if (fx - 30.0).powi(2) + (fy - 30.0).powi(2) <= 400.0 {
            [220, 30, 30, 255]
        } else if (60.0..110.0).contains(&fx) && (10.0..25.0).contains(&fy) {
            [30, 60, 200, 255]
        } else if (70.0..100.0).contains(&fx) && (45.0..75.0).contains(&fy) {
            [40, 160, 60, 255]
        } else if (10.0..110.0).contains(&fx) && (82.0..84.0).contains(&fy) {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    })
}

/// A document with `r` placed at (100, 100), scaled ×2, and selected.
fn doc_with(r: &Raster) -> (Session, NodeId) {
    let mut s = Session::new();
    let id = new_doc_with(&mut s, r);
    (s, id)
}

/// A new document in `s` holding `r` as a selected image.
fn new_doc_with(s: &mut Session, r: &Raster) -> NodeId {
    s.execute("file.new", &json!({"width": 2000, "height": 1000})).unwrap();
    let png = r.encode_png();
    let (w, h) = (r.width, r.height);
    s.edit("img", |d, sel| {
        d.images.insert("img1".into(), ImageBlob::new("image/png", png));
        let id = d.alloc_id();
        let l = d.default_layer();
        let image = ImageObject {
            key: "img1".into(),
            width: w,
            height: h,
            xf: Affine::translate((100.0, 100.0)) * Affine::scale(2.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(l, 0, Node::new(id, NodeKind::Image(Box::new(image))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

fn all_styles() -> Vec<&'static str> {
    offline::STYLES.iter().map(|s| s.id).collect()
}

fn generate(s: &mut Session, p: Value) -> crate::Result<Value> {
    s.execute("conceptVector.generate", &p)
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// A provider for the tests: `styles` ids `s0`, `s1`…, answering with `answer`.
struct Fake {
    id: &'static str,
    styles: usize,
    prompt: bool,
    raster: bool,
    answer: fn(&Request, Steps) -> Result<Vec<Variation>, ProviderError>,
}

const FAKE_STYLES: [&str; 10] = ["s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9"];

impl Provider for Fake {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        "Fake"
    }
    fn capabilities(&self) -> Capabilities {
        let styles = FAKE_STYLES.iter().take(self.styles).map(|id| Style { id, label: id }).collect();
        Capabilities { image_input: true, vector_output: true, prompt: self.prompt, raster_output: self.raster, offline: true, styles }
    }
    fn generate(&self, request: &Request, steps: Steps) -> Result<Vec<Variation>, ProviderError> {
        (self.answer)(request, steps)
    }
}

/// A square in the image's top-left quarter for each style asked.
fn squares(r: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
    let side = f64::from(r.width.min(r.height)) / 2.0;
    let sq = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(side, 0.0), Point::new(side, side), Point::new(0.0, side)], true));
    Ok(r.styles
        .iter()
        .map(|st| Variation {
            style: st.clone(),
            paths: vec![VectorPath { path: sq.clone(), fill: Some([9, 9, 9]), stroke: None, square_caps: false }],
            warnings: vec![],
        })
        .collect())
}

fn with_fake(s: &mut Session, fake: Fake) {
    s.concept_providers_mut().register(Arc::new(fake)).unwrap();
}

fn fake(answer: fn(&Request, Steps) -> Result<Vec<Variation>, ProviderError>) -> Fake {
    Fake { id: "test.fake", styles: 10, prompt: false, raster: false, answer }
}

#[test]
fn the_registry_holds_the_built_in_provider_and_refuses_a_second_id() {
    let mut r = Registry::default();
    assert_eq!(r.iter().map(|p| p.id).collect::<Vec<_>>(), [ID]);
    assert!(r.register(Arc::new(offline::TraceStyles)).is_err());
    assert!(r.register(Arc::new(fake(squares))).is_ok());
    assert!(r.register(Arc::new(fake(squares))).is_err());
    assert!(r.get("test.fake").is_some() && r.get("TEST.FAKE").is_none());
    let caps = offline::TraceStyles.capabilities();
    assert!(caps.offline && caps.vector_output && !caps.prompt && !caps.raster_output);
}

/// A provider whose capabilities panic from the `fails_from`th call on.
struct Faulty {
    calls: AtomicUsize,
    fails_from: usize,
}

impl Provider for Faulty {
    fn id(&self) -> &'static str {
        "test.faulty"
    }
    fn name(&self) -> &'static str {
        "Faulty"
    }
    fn capabilities(&self) -> Capabilities {
        if self.calls.fetch_add(1, Ordering::SeqCst) >= self.fails_from {
            panic!("capabilities failed");
        }
        fake(squares).capabilities()
    }
    fn generate(&self, request: &Request, steps: Steps) -> Result<Vec<Variation>, ProviderError> {
        squares(request, steps)
    }
}

/// A provider's capabilities are read once, when it is registered: one that panics then is not
/// registered, and one that would panic later is not asked again.
#[test]
fn a_provider_is_asked_its_capabilities_once_when_it_is_registered() {
    let mut r = Registry::default();
    let e = r.register(Arc::new(Faulty { calls: AtomicUsize::new(0), fails_from: 0 })).unwrap_err();
    assert!(e.contains("capabilities failed"), "{e}");
    assert!(r.get("test.faulty").is_none());

    let (mut s, img) = doc_with(&art());
    let faulty = Arc::new(Faulty { calls: AtomicUsize::new(0), fails_from: 1 });
    s.concept_providers_mut().register(Arc::clone(&faulty) as Arc<dyn Provider>).unwrap();
    let list = s.execute("conceptVector.providers", &json!({})).unwrap();
    assert_eq!(list["providers"][1]["styles"][0]["id"], "s0");
    let job = generate(&mut s, json!({"provider": "test.faulty", "styles": ["s0", "s1"], "id": img.0})).unwrap();
    assert_eq!(job["variations"][1]["label"], "s1");
    let r = s.execute("conceptVector.insert", &json!({"job": job["job"], "variations": ["s0"]})).unwrap();
    assert_eq!(r["ids"].as_array().map(Vec::len), Some(1));
    assert_eq!(faulty.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn providers_lists_each_provider_and_its_styles() {
    let mut s = Session::new();
    let r = s.execute("conceptVector.providers", &json!({})).unwrap();
    assert_eq!(r["selected"], ID);
    let p = &r["providers"][0];
    assert_eq!(p["id"], ID);
    assert_eq!(p["prompt"], false);
    assert_eq!(p["rasterOutput"], false);
    assert_eq!(p["maxStyles"], 8);
    let ids: Vec<&str> = p["styles"].as_array().unwrap().iter().map(|v| v["id"].as_str().unwrap()).collect();
    assert_eq!(ids, all_styles());
}

/// The checks on a result, whatever provider made it.
#[test]
fn results_are_checked_whatever_provider_made_them() {
    let r = Request {
        image: Arc::new(vec![]),
        width: 100,
        height: 50,
        prompt: None,
        styles: vec!["a".into(), "b".into()],
        detail: 50,
        output: Output::Vector,
    };
    let sq = |x: f64| PathData::single(SubPath::polyline(&[Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0)], true));
    let v = |style: &str, path: PathData, fill: Option<[u8; 3]>, stroke: Option<([u8; 3], f64)>| Variation {
        style: style.into(),
        paths: vec![VectorPath { path, fill, stroke, square_caps: false }],
        warnings: vec![],
    };
    let ok = vec![v("a", sq(0.0), Some([1, 2, 3]), None), v("b", sq(0.0), None, Some(([0, 0, 0], 2.0)))];
    assert_eq!(check_result(&r, &ok), Ok(()));
    // One for each style, in the order asked.
    assert_eq!(check_result(&r, &ok[..1]), Err(ProviderError::InvalidResult));
    let swapped = vec![ok[1].clone(), ok[0].clone()];
    assert_eq!(check_result(&r, &swapped), Err(ProviderError::InvalidResult));
    // Coordinates finite and within one image side of the image.
    for bad in [f64::NAN, f64::INFINITY, -101.0, 201.0] {
        let far = vec![ok[0].clone(), v("b", sq(bad), Some([0, 0, 0]), None)];
        assert_eq!(check_result(&r, &far), Err(ProviderError::InvalidResult), "{bad}");
    }
    // Stroke widths over 0, finite, at most the larger side.
    for w in [0.0, -1.0, f64::NAN, 101.0] {
        let wide = vec![ok[0].clone(), v("b", sq(0.0), None, Some(([0, 0, 0], w)))];
        assert_eq!(check_result(&r, &wide), Err(ProviderError::InvalidResult), "{w}");
    }
    // Painted, with an anchor.
    let unpainted = vec![ok[0].clone(), v("b", sq(0.0), None, None)];
    assert_eq!(check_result(&r, &unpainted), Err(ProviderError::InvalidResult));
    let empty = vec![ok[0].clone(), v("b", PathData::default(), Some([0, 0, 0]), None)];
    assert_eq!(check_result(&r, &empty), Err(ProviderError::InvalidResult));
    // Warnings bounded.
    let mut chatty = ok.clone();
    chatty[1].warnings = vec!["x".into(); MAX_WARNINGS + 1];
    assert_eq!(check_result(&r, &chatty), Err(ProviderError::InvalidResult));
    // The anchors of all the styles together.
    let many = PathData::single(SubPath::polyline(&vec![Point::new(1.0, 1.0); MAX_ANCHORS / 2 + 1], true));
    let heavy = vec![v("a", many.clone(), Some([0, 0, 0]), None), v("b", many, Some([0, 0, 0]), None)];
    assert_eq!(check_result(&r, &heavy), Err(ProviderError::ResourceLimit(MAX_ANCHORS)));
}

#[test]
fn generate_calls_only_the_named_provider() {
    let (mut s, _) = doc_with(&art());
    with_fake(&mut s, fake(squares));
    let r = generate(&mut s, json!({"provider": "test.fake", "styles": ["s1", "s0"]})).unwrap();
    assert_eq!(r["provider"], "test.fake");
    assert_eq!(r["variations"].as_array().unwrap().iter().map(|v| v["style"].as_str().unwrap()).collect::<Vec<_>>(), ["s1", "s0"]);
    // An unknown provider is an error, not the built-in one.
    let e = generate(&mut s, json!({"provider": "cloud.missing", "styles": ["outline"]})).unwrap_err().to_string();
    assert!(e.contains("no Concept to Vector provider `cloud.missing`"), "{e}");
    let e = generate(&mut s, json!({"styles": ["outline"]})).unwrap_err().to_string();
    assert!(e.contains("missing `provider`"), "{e}");
    // A session without the built-in provider does not use another in its place.
    let (mut bare, _) = doc_with(&art());
    *bare.concept_providers_mut() = Registry::empty();
    assert!(generate(&mut bare, json!({"provider": ID, "styles": ["outline"]})).is_err());
}

#[test]
fn styles_are_one_to_eight_different_ids_of_the_provider() {
    let (mut s, _) = doc_with(&art());
    with_fake(&mut s, fake(squares));
    let ids = |n: usize| FAKE_STYLES[..n].to_vec();
    for styles in [json!([]), json!(ids(9)), json!(["s0", "s0"]), json!(["s0", "nope"]), json!("s0"), json!([1]), json!(null)] {
        let e = generate(&mut s, json!({"provider": "test.fake", "styles": styles})).unwrap_err().to_string();
        assert!(e.contains("`styles` must list 1 to 8 different style ids of `test.fake`"), "{styles}: {e}");
    }
    assert_eq!(generate(&mut s, json!({"provider": "test.fake", "styles": ids(1)})).unwrap()["variations"].as_array().unwrap().len(), 1);
    assert_eq!(generate(&mut s, json!({"provider": "test.fake", "styles": ids(8)})).unwrap()["variations"].as_array().unwrap().len(), 8);
}

#[test]
fn a_prompt_is_refused_by_a_provider_without_prompts_and_raster_output_by_every_provider() {
    let (mut s, _) = doc_with(&art());
    let p = |extra: Value| {
        let mut p = json!({"provider": ID, "styles": ["flatColor"], "detail": 10});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    let e = generate(&mut s, p(json!({"prompt": "a dragon"}))).unwrap_err().to_string();
    assert!(e.contains("takes no prompt and makes vector art only"), "{e}");
    assert!(generate(&mut s, p(json!({"prompt": "  "}))).is_ok());
    assert!(generate(&mut s, p(json!({"output": "vector"}))).is_ok());
    for output in ["svg", "raster"] {
        let e = generate(&mut s, p(json!({"output": output}))).unwrap_err().to_string();
        assert!(e.contains("`output` must be \"vector\""), "{output}: {e}");
    }
    // Results are vector art, so raster output is refused even from a provider that makes it.
    let (mut r, _) = doc_with(&art());
    with_fake(&mut r, Fake { raster: true, ..fake(squares) });
    let e = generate(&mut r, json!({"provider": "test.fake", "styles": ["s0"], "output": "raster"})).unwrap_err().to_string();
    assert!(e.contains("`output` must be \"vector\""), "{e}");
    for detail in [json!(101), json!(-1), json!(2.5), json!("50")] {
        assert!(generate(&mut s, p(json!({"detail": detail}))).unwrap_err().to_string().contains("`detail` must be a whole number"), "{detail}");
    }
    // A provider that takes prompts gets the prompt, trimmed.
    fn echo(r: &Request, steps: Steps) -> Result<Vec<Variation>, ProviderError> {
        if r.prompt.as_deref() != Some("a dragon") {
            return Err(ProviderError::InvalidInput("no prompt".into()));
        }
        squares(r, steps)
    }
    with_fake(&mut s, Fake { prompt: true, ..fake(echo) });
    assert!(generate(&mut s, json!({"provider": "test.fake", "styles": ["s0"], "prompt": " a dragon "})).is_ok());
}

#[test]
fn the_source_is_one_shown_unlocked_image() {
    let (mut s, id) = doc_with(&art());
    s.execute("select.none", &json!({})).unwrap();
    let e = generate(&mut s, json!({"provider": ID, "styles": ["outline"]})).unwrap_err().to_string();
    assert!(e.contains("select one image or an unexpanded Image Trace object"), "{e}");
    // By id, with nothing selected.
    assert!(generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": id.0})).is_ok());
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let e = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": rect})).unwrap_err().to_string();
    assert!(e.contains("select one image"), "{e}");
    // Two objects selected.
    s.execute("select.set", &json!({"ids": [id.0, rect]})).unwrap();
    assert!(generate(&mut s, json!({"provider": ID, "styles": ["outline"]})).is_err());
    // Locked.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let e = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": id.0})).unwrap_err().to_string();
    assert!(e.contains("locked or hidden"), "{e}");
    // An Image Trace object that wasn't expanded uses its source image.
    let (mut t, img) = doc_with(&art());
    let traced = t.execute("imageTrace.make", &json!({"preset": "Default"})).unwrap()["id"].as_u64().unwrap();
    let r = generate(&mut t, json!({"provider": ID, "styles": ["flatColor"], "id": traced})).unwrap();
    assert_eq!(r["source"], traced);
    assert_ne!(traced, img.0);
}

/// A group around the image that is locked or hidden refuses it, when generating and again when
/// inserting a job made before the group was locked.
#[test]
fn a_locked_or_hidden_group_around_the_image_refuses_it() {
    for change in ["object.lock", "object.hide"] {
        let (mut s, id) = doc_with(&art());
        let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [id.0, rect]})).unwrap();
        let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
        let p = json!({"provider": ID, "styles": ["outline"], "id": id.0});
        let job = generate(&mut s, p.clone()).unwrap()["job"].as_u64().unwrap();
        s.execute(change, &json!({"ids": [group]})).unwrap();
        let e = generate(&mut s, p.clone()).unwrap_err().to_string();
        assert!(e.contains("locked or hidden"), "{change}: {e}");
        let (before, depth) = (s.doc().unwrap().doc.clone(), undo_len(&s));
        let e = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["outline"]})).unwrap_err().to_string();
        assert!(e.contains("locked or hidden"), "{change}: {e}");
        assert!(s.doc().unwrap().doc == before && undo_len(&s) == depth);
    }
}

#[test]
fn malformed_results_panics_and_cancels_keep_no_job_and_change_nothing() {
    fn wrong_count(_: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
        Ok(vec![])
    }
    #[allow(clippy::panic)]
    fn panics(_: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
        panic!("provider fault")
    }
    fn stops(r: &Request, steps: Steps) -> Result<Vec<Variation>, ProviderError> {
        if steps(0, 1) { squares(r, steps) } else { Err(ProviderError::Canceled) }
    }
    for (answer, expect) in [(wrong_count as fn(&Request, Steps) -> _, "do not match the request"), (panics, "internal error: provider fault")] {
        let (mut s, _) = doc_with(&art());
        with_fake(&mut s, fake(answer));
        let (before, depth) = (s.doc().unwrap().doc.clone(), undo_len(&s));
        let e = generate(&mut s, json!({"provider": "test.fake", "styles": ["s0"]})).unwrap_err().to_string();
        assert!(e.contains(expect), "{e}");
        assert!(s.doc().unwrap().doc == before && undo_len(&s) == depth);
        assert!(s.execute("conceptVector.results", &json!({"job": 1})).is_err(), "no job kept");
    }
    // Cancel: the run stops when its steps say so.
    let (mut s, _) = doc_with(&art());
    with_fake(&mut s, fake(stops));
    let run = ConceptRun::prepare(&s, &json!({"provider": "test.fake", "styles": ["s0"]})).unwrap();
    assert_eq!(run.run(&|_, _| false), Err(ProviderError::Canceled));
    assert!(run.run(&|_, _| true).is_ok());
    // The built-in provider stops too, before and between its styles.
    let run = ConceptRun::prepare(&s, &json!({"provider": ID, "styles": all_styles()})).unwrap();
    let calls = AtomicUsize::new(0);
    let stop_later = |_: u64, _: u64| calls.fetch_add(1, Ordering::Relaxed) < 3;
    assert_eq!(run.run(&stop_later), Err(ProviderError::Canceled));
}

/// A part at a time (the web build's way): the built-in provider decodes the image in one part,
/// finds a style's color layers in another and traces one layer a part (all of them in one part
/// for a style that stacks them), and makes the variations
/// one call makes; Cancel takes effect before the next part, inside a style.
#[test]
fn the_built_in_styles_are_made_a_part_at_a_time() {
    let (s, _) = doc_with(&art());
    let run = ConceptRun::prepare(&s, &json!({"provider": ID, "styles": ["flatColor", "outline"]})).unwrap();
    let mut parts = PartRun::default();
    let mut calls = 0;
    let mut first_style_parts = 0;
    while !run.run_part(&mut parts, &|_, _| true).unwrap() {
        calls += 1;
        assert!(calls < 1000, "the parts never end");
        if parts.styles_done() == 0 {
            first_style_parts = calls;
        }
    }
    // The decode, the layers found, and the four colors' layers each a part.
    assert!(first_style_parts >= 5, "{first_style_parts} parts before the first style was made");
    assert_eq!(parts.made(), run.run(&|_, _| true).unwrap());

    // Stopped after the layers are found, inside the first style.
    let mut parts = PartRun::default();
    for _ in 0..3 {
        assert_eq!(run.run_part(&mut parts, &|_, _| true), Ok(false));
    }
    assert_eq!(parts.styles_done(), 0);
    assert_eq!(run.run_part(&mut parts, &|_, _| false), Err(ProviderError::Canceled));
}

/// One style at a time, for a provider that makes no smaller parts: the same variations as one
/// call, each call adding one; results that don't match their style, or totals past a limit, stop
/// it.
#[test]
fn styles_made_one_at_a_time_match_one_call_and_are_checked_as_they_come() {
    fn many_anchors(r: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
        let pts: Vec<Point> = (0..600_000).map(|i| Point::new(f64::from(i % 100), f64::from(i / 100 % 80))).collect();
        let path = PathData::single(SubPath::polyline(&pts, true));
        Ok(r.styles
            .iter()
            .map(|st| Variation {
                style: st.clone(),
                paths: vec![VectorPath { path: path.clone(), fill: Some([0; 3]), stroke: None, square_caps: false }],
                warnings: vec![],
            })
            .collect())
    }
    fn renamed(r: &Request, steps: Steps) -> Result<Vec<Variation>, ProviderError> {
        let mut v = squares(r, steps)?;
        if r.styles == ["s1"] {
            v[0].style = "s0".into();
        }
        Ok(v)
    }
    let (mut s, _) = doc_with(&art());
    with_fake(&mut s, fake(squares));
    let run = ConceptRun::prepare(&s, &json!({"provider": "test.fake", "styles": ["s2", "s0", "s1"]})).unwrap();
    assert_eq!(run.style_count(), 3);
    let mut made = PartRun::default();
    for k in 1..=3 {
        assert_eq!(run.run_part(&mut made, &|_, _| true), Ok(k == 3));
        assert_eq!(made.styles_done(), k);
    }
    assert_eq!(run.run_part(&mut made, &|_, _| true), Ok(true), "nothing is left to make");
    assert_eq!(made.made(), run.run(&|_, _| true).unwrap());

    let (mut s, _) = doc_with(&art());
    with_fake(&mut s, fake(renamed));
    let run = ConceptRun::prepare(&s, &json!({"provider": "test.fake", "styles": ["s0", "s1"]})).unwrap();
    let mut made = PartRun::default();
    assert_eq!(run.run_part(&mut made, &|_, _| true), Ok(false));
    assert_eq!(run.run_part(&mut made, &|_, _| true), Err(ProviderError::InvalidResult));

    // Each style is under the anchor limit alone; the second passes it with the first.
    let (mut s, _) = doc_with(&art());
    with_fake(&mut s, fake(many_anchors));
    let run = ConceptRun::prepare(&s, &json!({"provider": "test.fake", "styles": ["s0", "s1"]})).unwrap();
    let mut made = PartRun::default();
    assert_eq!(run.run_part(&mut made, &|_, _| true), Ok(false));
    assert_eq!(run.run_part(&mut made, &|_, _| true), Err(ProviderError::ResourceLimit(MAX_ANCHORS)));
}

#[test]
fn the_built_in_styles_differ_and_repeat_exactly() {
    let raster = art();
    let png = Arc::new(raster.encode_png());
    let req = |styles: Vec<&str>, detail: u8| Request {
        image: Arc::clone(&png),
        width: raster.width,
        height: raster.height,
        prompt: None,
        styles: styles.into_iter().map(String::from).collect(),
        detail,
        output: Output::Vector,
    };
    let p = offline::TraceStyles;
    let made = p.generate(&req(all_styles(), 50), &|_, _| true).unwrap();
    check_result(&req(all_styles(), 50), &made).unwrap();
    assert_eq!(made, p.generate(&req(all_styles(), 50), &|_, _| true).unwrap(), "the same request makes the same paths");
    for v in &made {
        assert!(!v.paths.is_empty(), "{} made nothing", v.style);
    }
    let by = |id: &str| made.iter().find(|v| v.style == id).unwrap();
    // Outline: stroked lines only; Ink Lines: black fills only.
    assert!(by("outline").paths.iter().all(|p| p.fill.is_none() && p.stroke.is_some()));
    assert!(by("inkLines").paths.iter().all(|p| p.stroke.is_none() && p.fill == Some([0, 0, 0])));
    // Flat and Detailed Color keep the art's hues; Gray Tones only grays.
    let colors = |id: &str| by(id).paths.iter().filter_map(|p| p.fill).collect::<std::collections::BTreeSet<_>>();
    assert!(colors("flatColor").iter().any(|c| c[0] > 150 && c[1] < 100), "red kept: {:?}", colors("flatColor"));
    assert!(colors("detailedColor").len() >= colors("flatColor").len());
    assert!(colors("grayTones").iter().all(|c| c[0] == c[1] && c[1] == c[2]), "{:?}", colors("grayTones"));
    // The paths differ from style to style.
    for (i, a) in made.iter().enumerate() {
        for b in made.iter().skip(i + 1) {
            assert_ne!(a.paths, b.paths, "{} and {}", a.style, b.style);
        }
    }
    // The paths are in the image's pixel space.
    for v in &made {
        for path in &v.paths {
            let b = path.path.bounds().unwrap();
            assert!(b.x0 >= -1.0 && b.y0 >= -1.0 && b.x1 <= 121.0 && b.y1 <= 91.0, "{}: {b:?}", v.style);
        }
    }
}

#[test]
fn detail_moves_each_style_within_its_range() {
    for style in all_styles() {
        let (lo, hi) = (offline::style_params(style, 0).unwrap(), offline::style_params(style, 100).unwrap());
        assert!(lo.paths < hi.paths && lo.noise > hi.noise, "{style}");
        assert!(lo.colors <= hi.colors, "{style}");
        assert_eq!(offline::style_params(style, 255), offline::style_params(style, 100), "{style}: clamped");
    }
    assert!(offline::style_params("nope", 50).is_none());
    // A photo-like gradient keeps more anchors at Detail 100 than at Detail 0.
    let raster = Raster::from_fn(96, 96, |x, y| [(x * 2) as u8, (y * 2) as u8, ((x + y) % 256) as u8, 255]);
    let png = Arc::new(raster.encode_png());
    let anchors = |detail: u8| {
        let r = Request {
            image: Arc::clone(&png),
            width: 96,
            height: 96,
            prompt: None,
            styles: vec!["detailedColor".into()],
            detail,
            output: Output::Vector,
        };
        offline::TraceStyles.generate(&r, &|_, _| true).unwrap()[0].anchors()
    };
    assert!(anchors(100) > anchors(0), "{} vs {}", anchors(100), anchors(0));
}

/// Insert gives a stroked path the ends its variation names: square or round.
#[test]
fn insert_keeps_the_stroke_caps_of_each_path() {
    fn lines(r: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
        let line = |y: f64| PathData::single(SubPath::polyline(&[Point::new(10.0, y), Point::new(60.0, y)], false));
        let path = |y: f64, square_caps: bool| VectorPath { path: line(y), fill: None, stroke: Some(([0, 0, 0], 4.0)), square_caps };
        Ok(r.styles.iter().map(|st| Variation { style: st.clone(), paths: vec![path(20.0, true), path(40.0, false)], warnings: vec![] }).collect())
    }
    let (mut s, img) = doc_with(&art());
    with_fake(&mut s, fake(lines));
    let job = generate(&mut s, json!({"provider": "test.fake", "styles": ["s0"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["s0"]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let group = d.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    let caps: Vec<_> = group.children().unwrap().iter().map(|n| n.appearance.stroke().unwrap().cap).collect();
    assert_eq!(caps, [vectorcraft_doc::appearance::LineCap::Square, vectorcraft_doc::appearance::LineCap::Round]);
}

/// An image scaled down to fit keeps within the pixel limit whatever its shape, a long thin one
/// included, and one that fits stays as it is.
#[test]
fn an_image_is_scaled_down_within_the_pixel_limit_whatever_its_shape() {
    for (w, h) in [(64, 1), (1, 64), (64, 2), (40, 30), (8, 8)] {
        let fitted = offline::fit_within(Raster::from_fn(w, h, |x, y| [x as u8, y as u8, 0, 255]), 16);
        let n = u64::from(fitted.width) * u64::from(fitted.height);
        assert!(n <= 16 && fitted.width >= 1 && fitted.height >= 1, "{w} × {h} → {} × {}", fitted.width, fitted.height);
        assert_eq!(fitted.rgba.len() as u64, n * 4);
    }
    assert_eq!(offline::fit_within(Raster::from_fn(4, 4, |_, _| [1, 2, 3, 255]), 16).width, 4, "fits already");
}

/// A clear pixel scaled down with opaque ones takes away from the alpha, not from the color, and a
/// box with nothing opaque in it is clear.
#[test]
fn an_image_scaled_down_keeps_the_color_of_its_opaque_pixels() {
    let red_and_clear = Raster::from_fn(2, 2, |x, y| if (x, y) == (1, 1) { [0, 0, 0, 0] } else { [255, 0, 0, 255] });
    assert_eq!(offline::fit_within(red_and_clear, 1).rgba, [255, 0, 0, 191]);
    let half_clear = Raster::from_fn(2, 1, |x, _| if x == 0 { [0, 0, 255, 0] } else { [200, 100, 50, 128] });
    assert_eq!(offline::fit_within(half_clear, 1).rgba, [200, 100, 50, 64]);
    assert_eq!(offline::fit_within(Raster::from_fn(2, 2, |_, _| [9, 9, 9, 0]), 1).rgba, [0, 0, 0, 0]);
}

#[test]
fn generate_and_results_change_nothing_in_the_document() {
    let (mut s, _) = doc_with(&art());
    let (before, depth) = (s.doc().unwrap().doc.clone(), undo_len(&s));
    let r = generate(&mut s, json!({"provider": ID, "styles": ["outline", "flatColor"]})).unwrap();
    let job = r["job"].as_u64().unwrap();
    assert_eq!(r["state"], "done");
    assert_eq!(s.execute("conceptVector.results", &json!({"job": job})).unwrap(), r);
    assert!(s.doc().unwrap().doc == before && undo_len(&s) == depth);
    assert_eq!(s.concept_variations(job).unwrap().1, (120, 90));
}

/// The bounds of object `id` in the document.
fn bounds(s: &Session, id: u64) -> vectorcraft_geom::Rect {
    let d = &s.doc().unwrap().doc;
    vectorcraft_render::painted_bounds(d, d.node(NodeId(id)).unwrap()).unwrap()
}

#[test]
fn insert_puts_the_chosen_styles_right_of_the_image_as_one_undo_step() {
    let (mut s, img) = doc_with(&art());
    let job = generate(&mut s, json!({"provider": ID, "styles": all_styles()})).unwrap()["job"].as_u64().unwrap();
    let (before, depth) = (s.doc().unwrap().doc.clone(), undo_len(&s));
    // Chosen in another order: inserted in the provider's.
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["grayTones", "outline"]})).unwrap();
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(undo_len(&s), depth + 1);
    let d = &s.doc().unwrap().doc;
    let names: Vec<_> = ids.iter().map(|i| d.node(NodeId(*i)).unwrap().name.clone().unwrap()).collect();
    assert_eq!(names, ["Outline", "Gray Tones"]);
    assert!(d.node(img).is_some(), "the image stays");
    assert_eq!(s.doc().unwrap().selection.objects, ids.iter().map(|i| NodeId(*i)).collect::<Vec<_>>());
    // In a row to the right: the image is 240 × 180 pt from (100, 100).
    let (a, b) = (bounds(&s, ids[0]), bounds(&s, ids[1]));
    assert!(a.x0 >= 340.0 + GAP - 2.0 && a.x1 <= 364.0 + 240.0 + 2.0, "{a:?}");
    assert!(b.x0 >= 364.0 + 240.0 + GAP - 2.0, "{b:?}");
    assert!(a.y0 >= 98.0 && a.y1 <= 282.0, "{a:?}");
    let after = s.doc().unwrap().doc.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc == before, "undo restores the document");
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc == after, "redo restores the same groups");
}

#[test]
fn insert_moves_the_row_past_art_in_its_way() {
    let (mut s, img) = doc_with(&art());
    let block = s.execute("shape.rectangle", &json!({"x": 380, "y": 150, "width": 300, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let job = generate(&mut s, json!({"provider": ID, "styles": ["flatColor"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["flatColor"]})).unwrap();
    let placed = bounds(&s, r["ids"][0].as_u64().unwrap());
    assert!(placed.x0 >= bounds(&s, block).x1 + GAP - 2.0, "{placed:?}");
}

/// In a group with art far to its right, the row goes beside the image: the empty space between
/// the group's members is not in its way.
#[test]
fn insert_goes_beside_the_image_in_a_group_with_art_far_away() {
    let (mut s, img) = doc_with(&art());
    let far = s.execute("shape.rectangle", &json!({"x": 900, "y": 150, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [img.0, far]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let job = generate(&mut s, json!({"provider": ID, "styles": ["flatColor"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["flatColor"]})).unwrap();
    let (placed, image) = (bounds(&s, r["ids"][0].as_u64().unwrap()), bounds(&s, img.0));
    assert!(placed.x0 < image.x1 + GAP + 2.0 && placed.x1 < bounds(&s, far).x0, "{placed:?} after {image:?}");

    // The same in a clip group whose clipping path reaches past the far art.
    let (mut s, img) = doc_with(&art());
    let far = s.execute("shape.rectangle", &json!({"x": 900, "y": 150, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let mask = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 900, "height": 180})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [img.0, far, mask]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let job = generate(&mut s, json!({"provider": ID, "styles": ["flatColor"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["flatColor"]})).unwrap();
    let (placed, image) = (bounds(&s, r["ids"][0].as_u64().unwrap()), bounds(&s, img.0));
    assert!(placed.x0 < image.x1 + GAP + 2.0 && placed.x1 < bounds(&s, far).x0, "clip group: {placed:?} after {image:?}");

    // A clipping path given a stroke again paints it: the row goes past it.
    let (mut s, img) = doc_with(&art());
    let mask = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 900, "height": 180})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [img.0, mask]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [mask]})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    assert!(s.doc().unwrap().doc.node(NodeId(mask)).unwrap().clip_paint().stroke.is_some());
    let job = generate(&mut s, json!({"provider": ID, "styles": ["flatColor"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["flatColor"]})).unwrap();
    let placed = bounds(&s, r["ids"][0].as_u64().unwrap());
    assert!(placed.x0 >= 1000.0 + GAP, "painted clipping path: {placed:?}");

    // Hidden, the clipping path still paints its stroke, so the row still goes past it.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.hide", &json!({"ids": [mask]})).unwrap();
    assert!(!s.doc().unwrap().doc.node(NodeId(mask)).unwrap().visible);
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["flatColor"]})).unwrap();
    let placed = bounds(&s, r["ids"][0].as_u64().unwrap());
    assert!(placed.x0 >= 1000.0 + GAP, "hidden painted clipping path: {placed:?}");
}

/// An Image Trace object whose art paints past its image (here a drop shadow) keeps the row clear
/// of what it paints.
#[test]
fn insert_moves_the_row_past_what_a_traced_source_paints() {
    let (mut s, _) = doc_with(&art());
    let traced = s.execute("imageTrace.make", &json!({"preset": "Default"})).unwrap()["id"].as_u64().unwrap();
    let shadow = json!({"effect": "stylize.dropShadow", "params": {"x": 60, "y": 0, "blur": 0}, "ids": [traced]});
    s.execute("effect.apply", &shadow).unwrap();
    let painted = bounds(&s, traced);
    assert!(painted.x1 > 364.0, "the shadow reaches where the row would start: {painted:?}");
    let job = generate(&mut s, json!({"provider": ID, "styles": ["flatColor"], "id": traced})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["flatColor"]})).unwrap();
    let placed = bounds(&s, r["ids"][0].as_u64().unwrap());
    assert!(placed.x0 >= painted.x1 + GAP - 2.0, "{placed:?} after {painted:?}");
}

/// A layer's own drop shadow that reaches where the row would go moves the row past it.
#[test]
fn insert_moves_the_row_past_a_layers_own_shadow() {
    let (mut s, img) = doc_with(&art());
    let layer = s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("shape.rectangle", &json!({"x": 200, "y": 150, "width": 100, "height": 40})).unwrap();
    let shadow = json!({"effect": "stylize.dropShadow", "params": {"x": 150, "y": 0, "blur": 0}, "ids": [layer]});
    s.execute("effect.apply", &shadow).unwrap();
    let painted = bounds(&s, layer);
    assert!(painted.x1 > 400.0, "the shadow reaches where the row would start: {painted:?}");
    s.execute("select.set", &json!({"ids": [img.0]})).unwrap();
    let job = generate(&mut s, json!({"provider": ID, "styles": ["flatColor"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["flatColor"]})).unwrap();
    let placed = bounds(&s, r["ids"][0].as_u64().unwrap());
    assert!(placed.x0 >= painted.x1 + GAP - 2.0, "{placed:?} after {painted:?}");
}

/// A variation's paths reaching past the image's box keep clear of art there too.
#[test]
fn insert_moves_the_row_past_art_that_a_variation_reaches_beyond_the_image_box() {
    /// A bar from the image's left edge to 1.5 image widths, over its top half.
    fn wide(r: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
        let (w, h) = (f64::from(r.width) * 1.5, f64::from(r.height) / 2.0);
        let bar = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)], true));
        Ok(r.styles
            .iter()
            .map(|st| Variation {
                style: st.clone(),
                paths: vec![VectorPath { path: bar.clone(), fill: Some([9, 9, 9]), stroke: None, square_caps: false }],
                warnings: vec![],
            })
            .collect())
    }
    let (mut s, img) = doc_with(&art());
    with_fake(&mut s, fake(wide));
    // The image is 240 pt wide from x 100, so the row starts at 364 with boxes 240 pt wide, and
    // the bar reaches 360 pt from there; this block is past the box but under the bar.
    let block = s.execute("shape.rectangle", &json!({"x": 640, "y": 110, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
    let job = generate(&mut s, json!({"provider": "test.fake", "styles": ["s0"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["s0"]})).unwrap();
    let placed = bounds(&s, r["ids"][0].as_u64().unwrap());
    assert!(placed.x0 >= bounds(&s, block).x1 + GAP - 2.0, "{placed:?}");

    // Two such variations: the second starts GAP past what the first paints.
    let job = generate(&mut s, json!({"provider": "test.fake", "styles": ["s0", "s1"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["s0", "s1"]})).unwrap();
    let (a, b) = (bounds(&s, r["ids"][0].as_u64().unwrap()), bounds(&s, r["ids"][1].as_u64().unwrap()));
    assert!(b.x0 >= a.x1 + GAP - 0.5, "{a:?} {b:?}");
}

/// A variation that paints left of the image's box still starts GAP past the image.
#[test]
fn insert_keeps_the_gap_when_a_variation_reaches_left_of_the_image_box() {
    /// A bar from 10 pixels left of the image to its right edge, over its top half.
    fn left(r: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
        let (w, h) = (f64::from(r.width), f64::from(r.height) / 2.0);
        let bar = PathData::single(SubPath::polyline(&[Point::new(-10.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(-10.0, h)], true));
        Ok(r.styles
            .iter()
            .map(|st| Variation {
                style: st.clone(),
                paths: vec![VectorPath { path: bar.clone(), fill: Some([9, 9, 9]), stroke: None, square_caps: false }],
                warnings: vec![],
            })
            .collect())
    }
    let (mut s, img) = doc_with(&art());
    with_fake(&mut s, fake(left));
    let job = generate(&mut s, json!({"provider": "test.fake", "styles": ["s0", "s1"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["s0", "s1"]})).unwrap();
    let (a, b) = (bounds(&s, r["ids"][0].as_u64().unwrap()), bounds(&s, r["ids"][1].as_u64().unwrap()));
    let image = bounds(&s, img.0);
    assert!((a.x0 - (image.x1 + GAP)).abs() < 0.5, "{a:?} after {image:?}");
    assert!((b.x0 - (a.x1 + GAP)).abs() < 0.5, "{a:?} {b:?}");
}

/// The results go above the outermost clip group around the image, where it does not hide them,
/// and a layer that clips its art refuses them.
/// For an image in an envelope, the results go beside the envelope and its content stays as it is.
#[test]
fn insert_puts_the_results_outside_an_envelope_around_the_image() {
    let (mut s, img) = doc_with(&art());
    s.execute("object.envelope.makeWithWarp", &json!({"style": "arch", "bend": 30})).unwrap();
    let d = &s.doc().unwrap().doc;
    let envelope = d.parent_of(img).unwrap();
    assert!(matches!(d.node(envelope).unwrap().kind, NodeKind::Envelope { .. }));
    let (layer, before) = (d.parent_of(envelope).unwrap(), d.node(envelope).unwrap().clone());
    let job = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["outline"]})).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(group), Some(layer), "beside the envelope, not in it");
    assert!(*d.node(envelope).unwrap() == before, "the envelope is unchanged");
}

#[test]
fn insert_puts_the_results_outside_a_clip_group_around_the_image() {
    let (mut s, img) = doc_with(&art());
    // A clipping path over the image alone, in a group that clips it.
    let mask = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 240, "height": 180})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [img.0, mask]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let clip = d.parent_of(img).unwrap();
    assert!(matches!(d.node(clip).unwrap().kind, NodeKind::Group { clip: true, .. }));
    let layer = d.parent_of(clip).unwrap();
    let job = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["outline"]})).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(group), Some(layer), "beside the clip group, not in it");
    assert_eq!(d.parent_of(img), Some(clip), "the image stays in it");

    // A clipping layer.
    let (mut s, img) = doc_with(&art());
    let job = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 240, "height": 180})).unwrap();
    let layer = s.doc().unwrap().doc.parent_of(img).unwrap();
    s.execute("layer.clippingMask.toggle", &json!({"id": layer.0})).unwrap();
    let (before, depth) = (s.doc().unwrap().doc.clone(), undo_len(&s));
    let e = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["outline"]})).unwrap_err().to_string();
    assert!(e.contains("clipping mask"), "{e}");
    assert!(s.doc().unwrap().doc == before && undo_len(&s) == depth);
}

/// The results go above a group around the image whose opacity mask is on, and into it when
/// the mask is off; a layer whose opacity mask is on is refused.
#[test]
fn insert_puts_the_results_outside_an_opacity_masked_group_around_the_image() {
    let (mut s, img) = doc_with(&art());
    let other = s.execute("shape.rectangle", &json!({"x": 120, "y": 120, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let mask = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 120, "height": 90})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [img.0, other, mask]})).unwrap();
    let masked = NodeId(s.execute("transparency.makeOpacityMask", &json!({"clip": true})).unwrap()["id"].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(img), Some(masked));
    assert!(d.node(masked).unwrap().extras().mask.as_ref().is_some_and(|m| !m.disabled));
    let layer = d.parent_of(masked).unwrap();
    let job = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["outline"]})).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(group), Some(layer), "beside the masked group, not in it");
    assert_eq!(d.parent_of(img), Some(masked), "the image stays in it");

    // With the mask off, the group hides nothing, so the results go beside the image.
    s.execute("transparency.disableOpacityMask", &json!({"id": masked.0})).unwrap();
    let job = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["outline"]})).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(group), Some(masked));

    // A layer whose opacity mask is on is refused, and one whose mask is off is not.
    let (mut s, img) = doc_with(&art());
    let layer = s.doc().unwrap().doc.parent_of(img).unwrap();
    let art = s.doc().unwrap().doc.node(img).unwrap().clone();
    let mask_layer = |s: &mut Session, disabled: bool| {
        let m = OpacityMask { disabled, ..OpacityMask::new(art.clone(), true) };
        s.edit("mask", |d, _| {
            d.node_mut(layer).ok_or(crate::EngineError::NoNode(layer))?.extras_mut().mask = Some(Box::new(m));
            Ok(())
        })
        .unwrap();
    };
    mask_layer(&mut s, false);
    let job = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let (before, depth) = (s.doc().unwrap().doc.clone(), undo_len(&s));
    let e = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["outline"]})).unwrap_err().to_string();
    assert!(e.contains("opacity mask"), "{e}");
    assert!(s.doc().unwrap().doc == before && undo_len(&s) == depth);
    mask_layer(&mut s, true);
    let job = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["outline"]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(NodeId(r["ids"][0].as_u64().unwrap())), Some(layer));
}

/// A run whose document closed before it finished keeps no job.
#[test]
fn a_run_finished_after_its_document_closed_keeps_nothing() {
    let (mut s, _) = doc_with(&art());
    let run = ConceptRun::prepare(&s, &json!({"provider": ID, "styles": ["outline"]})).unwrap();
    let made = run.run(&|_, _| true).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("file.close", &json!({"index": 0})).unwrap();
    assert!(run.finish(&mut s, made).is_none());
    assert!(s.execute("conceptVector.results", &json!({"job": 1})).is_err(), "no job kept");
}

/// Closing a document drops its jobs at once, a background tab's too, and keeps the others'.
#[test]
fn closing_a_document_drops_its_jobs() {
    let (mut s, img) = doc_with(&art());
    let job = |s: &mut Session, id: u64| generate(s, json!({"provider": ID, "styles": ["outline"], "id": id})).unwrap()["job"].as_u64().unwrap();
    let first = job(&mut s, img.0);
    let other = new_doc_with(&mut s, &art());
    let second = job(&mut s, other.0);
    s.execute("file.close", &json!({"index": 0})).unwrap();
    assert!(s.concept_variations(first).is_none(), "the closed document's job is dropped");
    assert!(s.concept_variations(second).is_some(), "the open document's job is kept");
}

/// A batch that fails leaves the jobs as they were: one that closed a document brings it back with
/// its jobs, and a cancel in it is undone. A batch that succeeds drops a closed document's jobs.
#[test]
fn a_failed_batch_leaves_the_jobs_as_they_were() {
    let (mut s, img) = doc_with(&art());
    let job = generate(&mut s, json!({"provider": ID, "styles": ["outline"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let close = json!({"command": "file.close", "params": {"index": 0}});
    let bad = json!({"command": "no.suchCommand", "params": {}});
    assert!(s.execute("command.batch", &json!({"commands": [close.clone(), bad.clone()]})).is_err());
    assert_eq!(s.documents().len(), 1, "the document is back");
    assert!(s.concept_variations(job).is_some(), "the restored document's job is kept");
    // A job command after the close, in the same batch, does not drop the job either.
    let results = json!({"command": "conceptVector.results", "params": {"job": job}});
    new_doc_with(&mut s, &art());
    assert!(s.execute("command.batch", &json!({"commands": [close.clone(), results, bad.clone()]})).is_err());
    assert_eq!(s.documents().len(), 2, "the document is back");
    assert!(s.concept_variations(job).is_some(), "the restored document's job is kept after a job command");
    // A cancel in a batch that fails is undone with it.
    let cancel = json!({"command": "conceptVector.cancel", "params": {"job": job}});
    assert!(s.execute("command.batch", &json!({"commands": [cancel, bad.clone()]})).is_err());
    assert!(s.concept_variations(job).is_some(), "the job is not canceled");
    s.execute("command.batch", &json!({"commands": [close]})).unwrap();
    assert!(s.concept_variations(job).is_none(), "the job goes with the batch that closed its document");
}

#[test]
fn insert_refuses_a_stale_canceled_or_unknown_job_and_changes_nothing() {
    let (mut s, img) = doc_with(&art());
    let job = |s: &mut Session| generate(s, json!({"provider": ID, "styles": ["inkLines"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let insert = |s: &mut Session, job: u64, v: Value| s.execute("conceptVector.insert", &json!({"job": job, "variations": v}));
    let unchanged = |s: &mut Session, job: u64, v: Value, expect: &str| {
        let (before, depth) = (s.doc().unwrap().doc.clone(), undo_len(s));
        let e = insert(s, job, v).unwrap_err().to_string();
        assert!(e.contains(expect), "{e}");
        assert!(s.doc().unwrap().doc == before && undo_len(s) == depth);
    };
    let j = job(&mut s);
    for v in [json!([]), json!(["inkLines", "inkLines"]), json!(["outline"]), json!("inkLines")] {
        unchanged(&mut s, j, v, "`variations` must list different style ids");
    }
    unchanged(&mut s, 999, json!(["inkLines"]), "no Concept to Vector job 999");
    // A selection change alone leaves it current.
    s.execute("select.none", &json!({})).unwrap();
    assert!(insert(&mut s, j, json!(["inkLines"])).is_ok());
    s.execute("edit.undo", &json!({})).unwrap();
    // Other pixels under the same key.
    let other = Raster::from_fn(120, 90, |_, _| [10, 10, 10, 255]).encode_png();
    s.edit("pixels", |d, _| {
        d.images.insert("img1".into(), ImageBlob::new("image/png", other));
        Ok(())
    })
    .unwrap();
    unchanged(&mut s, j, json!(["inkLines"]), "no longer applies");
    s.execute("edit.undo", &json!({})).unwrap();
    // The image moved.
    s.execute("object.transform", &json!({"ids": [img.0], "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    unchanged(&mut s, j, json!(["inkLines"]), "no longer applies");
    s.execute("edit.undo", &json!({})).unwrap();
    // The image removed.
    s.execute("edit.clear", &json!({"ids": [img.0]})).unwrap();
    unchanged(&mut s, j, json!(["inkLines"]), "no longer applies");
    s.execute("edit.undo", &json!({})).unwrap();
    // Another document active.
    s.execute("file.new", &json!({})).unwrap();
    unchanged(&mut s, j, json!(["inkLines"]), "no longer applies");
    s.set_active(0);
    assert!(insert(&mut s, j, json!(["inkLines"])).is_ok(), "current again");
    s.execute("edit.undo", &json!({})).unwrap();
    // Canceled: twice is the same.
    for _ in 0..2 {
        assert_eq!(s.execute("conceptVector.cancel", &json!({"job": j})).unwrap()["state"], "canceled");
    }
    assert_eq!(s.execute("conceptVector.results", &json!({"job": j})).unwrap()["state"], "canceled");
    unchanged(&mut s, j, json!(["inkLines"]), "it was canceled");
    // A closed document's jobs are dropped.
    let j = job(&mut s);
    s.close_document(0);
    assert!(s.execute("conceptVector.results", &json!({"job": j})).is_err());
}

#[test]
fn the_session_keeps_the_newest_jobs() {
    let (mut s, _) = doc_with(&art());
    with_fake(&mut s, fake(squares));
    let jobs: Vec<u64> =
        (0..KEEP_JOBS + 1).map(|_| generate(&mut s, json!({"provider": "test.fake", "styles": ["s0"]})).unwrap()["job"].as_u64().unwrap()).collect();
    assert!(s.execute("conceptVector.results", &json!({"job": jobs[0]})).is_err());
    for j in &jobs[1..] {
        assert!(s.execute("conceptVector.results", &json!({"job": j})).is_ok());
    }
}

/// A list of styles or variations longer than any job can hold is refused by its length, before
/// it is checked for repeats.
#[test]
fn a_long_list_of_styles_or_variations_is_refused() {
    let (mut s, _) = doc_with(&art());
    with_fake(&mut s, fake(squares));
    let long: Vec<String> = (0..20_000).map(|i| format!("style{i}")).collect();
    let e = generate(&mut s, json!({"provider": "test.fake", "styles": long})).unwrap_err().to_string();
    assert!(e.contains("`styles` must list 1 to 8 different style ids"), "{e}");
    let job = generate(&mut s, json!({"provider": "test.fake", "styles": ["s0"]})).unwrap()["job"].as_u64().unwrap();
    let e = s.execute("conceptVector.insert", &json!({"job": job, "variations": long})).unwrap_err().to_string();
    assert!(e.contains(&format!("`variations` must list different style ids of job {job}")), "{e}");
}

/// Parts whose count of the styles made panics from its second call on.
struct CountPanics {
    calls: std::cell::Cell<usize>,
}

impl Parts for CountPanics {
    fn step(&mut self) -> Result<Option<Vec<Variation>>, ProviderError> {
        Ok(None)
    }
    fn styles_done(&self) -> usize {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() >= 2 {
            panic!("count failed");
        }
        0
    }
}

/// A provider with one style, `s0`, made in [`CountPanics`] parts.
struct PanickyParts;

impl Provider for PanickyParts {
    fn id(&self) -> &'static str {
        "test.panickyParts"
    }
    fn name(&self) -> &'static str {
        "Panicky Parts"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { styles: vec![Style { id: "s0", label: "s0" }], ..fake(squares).capabilities() }
    }
    fn generate(&self, request: &Request, steps: Steps) -> Result<Vec<Variation>, ProviderError> {
        squares(request, steps)
    }
    fn parts(&self, _: &Request) -> Option<Box<dyn Parts>> {
        Some(Box::new(CountPanics { calls: std::cell::Cell::new(0) }))
    }
}

/// A panic while a provider counts the styles its parts have made is an error of the part that
/// asked, and the count kept from before it is what the run reports.
#[test]
fn a_panic_in_a_providers_count_of_styles_made_is_an_error() {
    let (mut s, img) = doc_with(&art());
    s.concept_providers_mut().register(Arc::new(PanickyParts)).unwrap();
    let run = ConceptRun::prepare(&s, &json!({"provider": "test.panickyParts", "styles": ["s0"], "id": img.0})).unwrap();
    let mut parts = PartRun::default();
    assert!(matches!(run.run_part(&mut parts, &|_, _| true), Err(ProviderError::Internal(_))));
    assert_eq!(parts.styles_done(), 0);
}

/// Square caps on a slanted line reach past its end by up to √2 times half the width on an axis:
/// the row keeps its gap from the image, and between its members, past those corners.
#[test]
fn insert_keeps_the_gap_past_the_square_caps_of_slanted_lines() {
    /// Two 45° lines 20 px wide with square caps: one from the image's top-left corner, one to its
    /// right edge.
    fn slants(r: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
        let line = |a: Point, b: Point| VectorPath {
            path: PathData::single(SubPath::polyline(&[a, b], false)),
            fill: None,
            stroke: Some(([0, 0, 0], 20.0)),
            square_caps: true,
        };
        let paths = vec![line(Point::new(0.0, 0.0), Point::new(90.0, 90.0)), line(Point::new(30.0, 0.0), Point::new(120.0, 90.0))];
        Ok(r.styles.iter().map(|st| Variation { style: st.clone(), paths: paths.clone(), warnings: vec![] }).collect())
    }
    let (mut s, img) = doc_with(&art());
    with_fake(&mut s, fake(slants));
    let job = generate(&mut s, json!({"provider": "test.fake", "styles": ["s0", "s1"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["s0", "s1"]})).unwrap();
    // The image is scaled ×2, so the strokes are 40 pt wide and each cap's corner lies 20√2 pt
    // past its line's end across; the image ends at x 340.
    let reach = 20.0 * std::f64::consts::SQRT_2;
    let d = &s.doc().unwrap().doc;
    let lines = |i: usize| d.bounds_of(&[NodeId(r["ids"][i].as_u64().unwrap())], false).unwrap();
    let (a, b) = (lines(0), lines(1));
    assert!(a.x0 - reach >= 340.0 + GAP - 0.01, "{a:?}");
    assert!(b.x0 - reach >= a.x1 + reach + GAP - 0.01, "{a:?} {b:?}");
}

/// A long run of objects, each too close to the next for the row to fit between: the row goes
/// past the last one, and is not moved further.
#[test]
fn insert_moves_the_row_past_a_long_run_of_objects_in_its_way() {
    let (mut s, img) = doc_with(&art());
    let n = 400;
    for k in 0..n {
        s.execute("shape.rectangle", &json!({"x": 380.0 + 200.0 * f64::from(k), "y": 150, "width": 10, "height": 10})).unwrap();
    }
    let job = generate(&mut s, json!({"provider": ID, "styles": ["flatColor"], "id": img.0})).unwrap()["job"].as_u64().unwrap();
    let r = s.execute("conceptVector.insert", &json!({"job": job, "variations": ["flatColor"]})).unwrap();
    let placed = bounds(&s, r["ids"][0].as_u64().unwrap());
    let last = 380.0 + 200.0 * f64::from(n - 1) + 10.0;
    assert!((placed.x0 - (last + GAP)).abs() <= 2.0, "{placed:?}");
}
