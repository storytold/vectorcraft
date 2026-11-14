use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;

fn drag(t: &mut GestureTool, cx: &ToolContext, pts: &[(f64, f64)], m: Mods) -> Vec<Vec<Action>> {
    let mut v = vec![t.pointer(cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1))];
    for &(x, y) in &pts[1..] {
        v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Drag, x, y)));
    }
    let l = pts[pts.len() - 1];
    v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Up, l.0, l.1).with_mods(m)));
    v
}

fn drag_held(t: &mut GestureTool, cx: &ToolContext, pts: &[(f64, f64)], m: Mods) -> Vec<Vec<Action>> {
    let mut v = vec![t.pointer(cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1).with_mods(m))];
    for &(x, y) in &pts[1..] {
        v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Drag, x, y).with_mods(m)));
    }
    let l = pts[pts.len() - 1];
    v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Up, l.0, l.1).with_mods(m)));
    v
}

/// The Pencil's new path starts and ends on Smart Guides (#506), the points between where
/// the hand went.
#[test]
fn pencil_ends_snap_to_smart_guides() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("pencil");
    let acts = drag(&mut t, &cx, &[(102.0, 98.0), (151.0, 300.0), (250.0, 320.0), (198.0, 203.0)], Mods::default());
    let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
    assert_eq!(v["points"], json!([[100.0, 100.0], [151.0, 300.0], [250.0, 320.0], [200.0, 200.0]]));
    // The Paintbrush follows the hand all the way.
    let mut t = GestureTool::new("paintbrush");
    let acts = drag(&mut t, &cx, &[(102.0, 98.0), (151.0, 300.0)], Mods::default());
    let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
    assert_eq!(v["points"][0], json!([102.0, 98.0]));
}

/// #852: the Paintbrush sends each point's pen pressure; a mouse (always fully pressed) and
/// the Pencil send plain points.
#[test]
fn paintbrush_sends_pen_pressure() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let stroke = |id: &str, pressures: [f32; 3]| {
        let mut t = GestureTool::new(id);
        let kinds = [PointerKind::Down, PointerKind::Drag, PointerKind::Up];
        let mut last = vec![];
        for ((k, x), f) in kinds.into_iter().zip([0.0, 40.0, 80.0]).zip(pressures) {
            last = t.pointer(&cx, &PointerEvent { pressure: f, ..PointerEvent::new(k, x, 0.0) });
        }
        let Some(Action::Preview(_, v)) = last.first() else { panic!("{last:?}") };
        v["points"].clone()
    };
    assert_eq!(stroke("paintbrush", [0.25, 0.5, 1.0]), json!([[0.0, 0.0, 0.25], [40.0, 0.0, 0.5], [80.0, 0.0, 1.0]]));
    assert_eq!(stroke("paintbrush", [1.0; 3]), json!([[0.0, 0.0], [40.0, 0.0], [80.0, 0.0]]));
    assert_eq!(stroke("pencil", [0.25, 0.5, 1.0]), json!([[0.0, 0.0], [40.0, 0.0], [80.0, 0.0]]));
}

/// #764: the Paintbrush sends each point's pen tilt and barrel rotation where the pen reads
/// them, `null` for what a point lacks; the Pencil sends plain points.
#[test]
fn paintbrush_sends_pen_tilt_and_rotation() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let stroke = |id: &str, pressure: f32, poses: [Option<PenPose>; 3]| {
        let mut t = GestureTool::new(id);
        let kinds = [PointerKind::Down, PointerKind::Drag, PointerKind::Up];
        let mut last = vec![];
        for ((k, x), pen) in kinds.into_iter().zip([0.0, 40.0, 80.0]).zip(poses) {
            last = t.pointer(&cx, &PointerEvent { pressure, pen, ..PointerEvent::new(k, x, 0.0) });
        }
        let Some(Action::Preview(_, v)) = last.first() else { panic!("{last:?}") };
        v["points"].clone()
    };
    let leaning = |x: f32, r: Option<f32>| Some(PenPose { tilt: Some([x, -10.0]), rotation: r });
    assert_eq!(
        stroke("paintbrush", 0.5, [leaning(30.0, Some(45.0)), leaning(40.0, Some(-90.0)), None]),
        json!([[0.0, 0.0, 0.5, 30.0, -10.0, 45.0], [40.0, 0.0, 0.5, 40.0, -10.0, 270.0], [80.0, 0.0, 0.5, null, null, null]])
    );
    // A pen without a rotation axis, pressing fully: no pressure, the tilt alone.
    assert_eq!(stroke("paintbrush", 1.0, [leaning(30.0, None); 3])[1], json!([40.0, 0.0, null, 30.0, -10.0, null]));
    // A pose that reads nothing is a mouse's.
    assert_eq!(stroke("paintbrush", 1.0, [Some(PenPose::default()); 3]), json!([[0.0, 0.0], [40.0, 0.0], [80.0, 0.0]]));
    assert_eq!(stroke("pencil", 0.5, [leaning(30.0, Some(45.0)); 3]), json!([[0.0, 0.0], [40.0, 0.0], [80.0, 0.0]]));
}

#[test]
fn pencil_previews_and_commits() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("pencil");
    let acts = drag(&mut t, &cx, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], Mods::default());
    assert!(acts[0].is_empty());
    assert_eq!(acts[1][0], Action::Begin("Pencil".into()));
    assert!(matches!(&acts[1][1], Action::Preview(c, v) if c == "path.freehand" && v["points"].as_array().unwrap().len() == 2));
    let last = acts.last().unwrap();
    assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == false && v["style"] == "pencil"));
    assert_eq!(last[1], Action::Commit);
    assert!(!t.busy());
}

/// Live curve fitting is the Pencil's, on by default. Off, the previews while a stroke is drawn
/// ask for the points unfitted (`fit: false`), and the release's preview is fitted as before.
#[test]
fn pencil_with_live_curve_fitting_off_previews_the_stroke_unfitted() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let stroke = [(0.0, 0.0), (10.0, 5.0), (20.0, 0.0), (30.0, 6.0), (40.0, 0.0)];
    // The `fit` each preview sends: those while drawn, then the release's.
    let fits = |t: &mut GestureTool| {
        let acts = drag(t, &cx, &stroke, Mods::default());
        let fit = |a: &Action| if let Action::Preview(_, v) = a { Some(v.get("fit").cloned()) } else { None };
        let (drawn, release) = acts.split_at(acts.len() - 1);
        (drawn.iter().flatten().filter_map(fit).collect::<Vec<_>>(), release[0].iter().filter_map(fit).collect::<Vec<_>>())
    };
    let mut t = GestureTool::new("pencil");
    assert_eq!(t.options()["liveFit"], json!(true));
    let (drawn, release) = fits(&mut t);
    assert!(!drawn.is_empty() && drawn.iter().all(Option::is_none), "{drawn:?}");
    assert_eq!(release, [None]);
    t.set_option("liveFit", &json!(false));
    assert_eq!(t.options()["liveFit"], json!(false));
    let (drawn, release) = fits(&mut t);
    assert!(!drawn.is_empty() && drawn.iter().all(|f| *f == Some(json!(false))), "{drawn:?}");
    assert_eq!(release, [None], "the release fits the stroke");
    // Choosing another tool while the stroke is drawn keeps it fitted, too.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    for &(x, y) in &stroke[1..] {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y));
    }
    let acts = t.deactivate(&cx);
    assert!(matches!(acts.as_slice(), [Action::Preview(_, v), Action::Commit] if v.get("fit").is_none()), "{acts:?}");
    // The Paintbrush has no such option, and its previews are fitted.
    let mut t = GestureTool::new("paintbrush");
    t.set_option("liveFit", &json!(false));
    assert!(t.options().get("liveFit").is_none());
    let (drawn, _) = fits(&mut t);
    assert!(!drawn.is_empty() && drawn.iter().all(Option::is_none), "{drawn:?}");
}

/// A stroke whose Fidelity is finer than half a screen pixel previews fitted at half a pixel, which
/// looks the same at a fraction of the fit's cost (#4565); the release, and another tool chosen
/// while it is drawn, fit it at its own Fidelity. A coarser Fidelity previews as it is kept. Fidelity
/// is in screen pixels, so the stroke is fitted in points at Fidelity / zoom (#5729).
#[test]
fn fine_fidelity_previews_at_half_a_pixel() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    let stroke = [(0.0, 0.0), (10.0, 5.0), (20.0, 0.0), (30.0, 5.0)];
    let fidelities = |acts: &[Action]| -> Vec<f64> {
        acts.iter().filter_map(|a| if let Action::Preview(_, v) = a { v["fidelity"].as_f64() } else { None }).collect()
    };
    for id in ["pencil", "paintbrush"] {
        for (zoom, fidelity, drawn, kept) in [(1.0, 0.01, 0.5, 0.01), (4.0, 0.01, 0.125, 0.0025), (1.0, 1.5, 1.5, 1.5), (4.0, 1.5, 0.375, 0.375)] {
            cx.zoom = zoom;
            let mut t = GestureTool::new(id);
            t.set_option("fidelity", &json!(fidelity));
            let acts = drag(&mut t, &cx, &stroke, Mods::default());
            let (release, during) = acts.split_last().unwrap();
            assert_eq!(fidelities(&during.concat()), [drawn; 3], "{id} at {zoom}x, {fidelity} px");
            assert_eq!(fidelities(release), [kept], "{id} at {zoom}x, {fidelity} px: the release");
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
            for &(x, y) in &stroke[1..] {
                t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y));
            }
            let acts = t.deactivate(&cx);
            let stale = if drawn > kept { [kept].to_vec() } else { vec![] };
            assert_eq!(fidelities(&acts), stale, "{id} at {zoom}x, {fidelity} px: another tool chosen");
            assert_eq!(acts.last(), Some(&Action::Commit));
        }
    }
}

/// #5729: the Pencil's and the Paintbrush's Fidelity is in screen pixels. The same stroke drawn on
/// screen at 1x and at 4x is fitted at the same tolerance on screen, so it keeps as many points
/// whatever the zoom (another tool chosen mid-stroke: `fine_fidelity_previews_at_half_a_pixel`).
#[test]
fn fidelity_is_in_screen_pixels_at_any_zoom() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    let screen = [(0.0, 0.0), (40.0, 20.0), (80.0, 0.0), (120.0, 20.0)];
    for id in ["pencil", "paintbrush"] {
        let on_screen = |cx: &ToolContext, zoom: f64| {
            let pts: Vec<_> = screen.iter().map(|&(x, y)| (x / zoom, y / zoom)).collect();
            let mut t = GestureTool::new(id);
            t.set_option("fidelity", &json!(1.5));
            let acts = drag(&mut t, cx, &pts, Mods::default());
            let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
            v["fidelity"].as_f64().unwrap() * zoom
        };
        cx.zoom = 1.0;
        let at_1 = on_screen(&cx, 1.0);
        cx.zoom = 4.0;
        let at_4 = on_screen(&cx, 4.0);
        assert_eq!(at_1, 1.5, "{id}");
        assert_eq!(at_4, at_1, "{id}: fidelity in screen px at 1x vs 4x");
    }
}

/// The highest Fidelity (100 px) at the lowest zoom (3.13 %) is sent, while the stroke is drawn and
/// at its release, as [`crate::draw2::MAX_FIT_TOLERANCE`], the coarsest tolerance `path.freehand`
/// reads.
#[test]
fn highest_fidelity_at_the_lowest_zoom_sends_the_coarsest_tolerance() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.zoom = 0.0313;
    let stroke: Vec<_> = [(0.0, 0.0), (40.0, 20.0), (80.0, 0.0), (120.0, 20.0)].iter().map(|&(x, y)| (x / cx.zoom, y / cx.zoom)).collect();
    for id in ["pencil", "paintbrush"] {
        let mut t = GestureTool::new(id);
        t.set_option("fidelity", &json!(100.0));
        let acts = drag(&mut t, &cx, &stroke, Mods::default());
        let sent: Vec<_> =
            acts.concat().into_iter().filter_map(|a| if let Action::Preview(_, v) = a { v["fidelity"].as_f64() } else { None }).collect();
        assert_eq!(sent, [crate::draw2::MAX_FIT_TOLERANCE; 4], "{id}");
    }
}

/// `path.freehand` rejects a `fidelity` outside `MIN_FIT_TOLERANCE..=MAX_FIT_TOLERANCE`, so at a zoom
/// beyond the view's range the Pencil and the Paintbrush send the nearer end of it, while the stroke
/// is drawn and at its release.
#[test]
fn fidelity_sent_stays_within_what_path_freehand_takes() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    let (lo, hi) = (crate::draw2::MIN_FIT_TOLERANCE, crate::draw2::MAX_FIT_TOLERANCE);
    for (zoom, fidelity, release) in [(0.01, 100.0, hi), (1000.0, 0.01, lo)] {
        cx.zoom = zoom;
        let stroke: Vec<_> = [(0.0, 0.0), (40.0, 20.0), (80.0, 0.0), (120.0, 20.0)].iter().map(|&(x, y)| (x / zoom, y / zoom)).collect();
        for id in ["pencil", "paintbrush"] {
            let mut t = GestureTool::new(id);
            t.set_option("fidelity", &json!(fidelity));
            let acts = drag(&mut t, &cx, &stroke, Mods::default());
            let sent: Vec<_> =
                acts.concat().into_iter().filter_map(|a| if let Action::Preview(_, v) = a { v["fidelity"].as_f64() } else { None }).collect();
            assert!(sent.iter().all(|f| (lo..=hi).contains(f)), "{id} at {zoom}x: {sent:?}");
            assert_eq!(sent.last(), Some(&release), "{id} at {zoom}x: {sent:?}");
        }
    }
}

/// The Blob Brush's Fidelity stays in points (#5729 is the Pencil's and the Paintbrush's): its
/// stroke, and another tool chosen while it is drawn, send it as set at any zoom.
#[test]
fn blob_brush_fidelity_stays_in_points_at_any_zoom() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    let stroke = [(0.0, 0.0), (40.0, 20.0), (80.0, 0.0), (120.0, 20.0)];
    for zoom in [1.0, 4.0] {
        cx.zoom = zoom;
        let mut t = GestureTool::new("blobBrush");
        t.set_option("fidelity", &json!(1.5));
        let acts = drag(&mut t, &cx, &stroke, Mods::default());
        let sent: Vec<_> = acts
            .concat()
            .into_iter()
            .filter_map(|a| if let Action::Exec(_, v) | Action::Preview(_, v) = a { v["fidelity"].as_f64() } else { None })
            .collect();
        assert_eq!(sent, [1.5], "at {zoom}x");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        for &(x, y) in &stroke[1..] {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y));
        }
        let acts = t.deactivate(&cx);
        assert!(!acts.iter().any(|a| matches!(a, Action::Preview(id, _) if id == "path.freehand")), "at {zoom}x: {acts:?}");
    }
}

/// Keep selected is on by default, and every Pencil and Paintbrush preview sends it.
#[test]
fn pencil_and_paintbrush_send_keep_selected() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    for id in ["pencil", "paintbrush"] {
        let mut t = GestureTool::new(id);
        assert_eq!(t.options()["keepSelected"], json!(true), "{id}");
        for keep in [true, false] {
            t.set_option("keepSelected", &json!(keep));
            assert_eq!(t.options()["keepSelected"], json!(keep), "{id}");
            let acts = drag(&mut t, &cx, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], Mods::default());
            assert!(acts[1..].iter().all(|a| a.iter().all(|a| !matches!(a, Action::Preview(_, v) if v["keepSelected"] != keep))), "{id}");
            assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["keepSelected"] == keep), "{id}");
        }
    }
}

/// With the Pencil's Live preview off, a stroke previews nothing while it is drawn: a thin line
/// follows the pen (an overlay), and the release draws the path with its fill and stroke. On (the
/// default), every step previews the path and no line is shown. The Paintbrush has no such option.
#[test]
fn pencil_with_live_preview_off_shows_a_thin_line_until_the_release() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let previews = |acts: &[Action]| acts.iter().filter(|a| matches!(a, Action::Preview(..))).count();
    let line = |t: &GestureTool| t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { width, .. } if *width == 1.0));
    let pts = [(0.0, 0.0), (10.0, 5.0), (20.0, 0.0), (30.0, 5.0)];
    for live in [true, false] {
        let mut t = GestureTool::new("pencil");
        assert_eq!(t.options()["livePreview"], json!(true), "on by default");
        t.set_option("livePreview", &json!(live));
        assert_eq!(t.options()["livePreview"], json!(live));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1));
        let mut during = vec![];
        for &(x, y) in &pts[1..] {
            during.extend(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y)));
        }
        assert!(matches!(during.first(), Some(Action::Begin(_))), "live {live}: {during:?}");
        assert_eq!(previews(&during), if live { 3 } else { 0 }, "live {live}: {during:?}");
        assert_eq!(line(&t), !live, "live {live}: the thin line shows only with Live preview off");
        let up = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 30.0, 5.0));
        assert!(matches!(up.as_slice(), [Action::Preview(c, _), Action::Commit] if c == "path.freehand"), "live {live}: {up:?}");
        assert!(!line(&t), "live {live}: no line after the release");
    }
    // Escape mid-stroke with Live preview off cancels the stroke it began.
    let mut t = GestureTool::new("pencil");
    t.set_option("livePreview", &json!(false));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0));
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    // The Paintbrush keeps previewing: Live preview isn't one of its options.
    let mut b = GestureTool::new("paintbrush");
    assert!(b.options().get("livePreview").is_none());
    b.set_option("livePreview", &json!(false));
    b.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    let acts = b.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0));
    assert_eq!(previews(&acts), 1, "{acts:?}");
}

/// Holding Alt shows the Paintbrush's closing cursor before the press, and the Pencil's and the
/// Paintbrush's while drawing (before a press, Alt gives the Pencil its straight-line cursor); a release
/// with Alt still held closes the path, and Alt let go before the release leaves it open.
/// The freehand tools take every position the pointer passed (`wants_samples`); the Join tool
/// doesn't need them.
#[test]
fn the_freehand_tools_want_every_sample() {
    for id in ["pencil", "paintbrush", "blobBrush", "eraser", "knife", "smooth", "pathEraser"] {
        assert!(GestureTool::new(id).wants_samples(), "{id}");
    }
    assert!(!GestureTool::new("join").wants_samples());
}

#[test]
fn alt_shows_the_closing_cursor_and_a_release_with_alt_closes() {
    let (d, rect) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let at = Point::new(10.0, 5.0);
    for (id, own, closing, style) in
        [("pencil", Cursor::Pencil, Cursor::PencilClose, "pencil"), ("paintbrush", Cursor::Paintbrush, Cursor::PaintbrushClose, "brush")]
    {
        let mut t = GestureTool::new(id);
        assert_eq!(t.cursor(&cx, at, Mods::default()), if id == "pencil" { Cursor::PencilNew } else { own }, "{id}");
        assert_eq!(t.cursor(&cx, at, alt), if id == "pencil" { Cursor::PencilStraight } else { closing }, "{id}");
        for alt_at_release in [true, false] {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0).with_mods(alt));
            assert_eq!(t.cursor(&cx, at, alt), closing, "{id} while drawing");
            assert_eq!(t.cursor(&cx, at, Mods::default()), own, "{id} while drawing");
            let up = Mods { alt: alt_at_release, ..Default::default() };
            let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 20.0, 0.0).with_mods(up));
            assert!(
                matches!(&acts[0], Action::Preview(_, v) if v["closed"] == alt_at_release && v["style"] == style),
                "{id}, Alt at the release {alt_at_release}: {acts:?}"
            );
        }
    }
    // With Alt key toggles to Smooth Tool on, Alt lends the Pencil the Smooth tool instead, until
    // a stroke has begun to draw: Alt then neither smooths nor closes, and the cursor stays the Pencil's.
    // With nothing selected to smooth, the Smooth tool it lends shows Not Allowed.
    let mut t = GestureTool::new("pencil");
    t.set_option("altSmooth", &json!(true));
    assert_eq!(t.cursor(&cx, at, alt), Cursor::NotAllowed);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0));
    assert_eq!(t.cursor(&cx, at, alt), Cursor::Pencil, "a stroke that has begun");
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 20.0, 0.0).with_mods(alt));
    assert!(matches!(&acts[0], Action::Preview(_, v) if v["closed"] == false), "{acts:?}");
    // A smoothing drag keeps the lent cursor when Alt is let go: Smooth with a selected path, Not
    // Allowed without one.
    let mut selected = Selection::default();
    selected.set([rect]);
    for (sel, c) in [(&s, Cursor::NotAllowed), (&selected, Cursor::Smooth)] {
        let cx = crate::testutil::cx(&d, sel, &p);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(alt));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0).with_mods(alt));
        assert_eq!(t.cursor(&cx, at, Mods::default()), c, "a smoothing drag");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 20.0, 0.0).with_mods(alt));
    }
}

#[test]
fn blob_brush_drag_shows_scrub_length_measurement_labels() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let on = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
    let off = ToolContext { measurement_labels: false, ..cx(&d, &s, &p) };
    let mut t = GestureTool::new("blobBrush");
    t.pointer(&on, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    t.pointer(&on, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
    let want = format!("L: {}", on.len(100.0));
    assert!(t.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
    let mut quiet = GestureTool::new("blobBrush");
    quiet.pointer(&off, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    quiet.pointer(&off, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
    assert!(!quiet.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })));
}

/// #4343: while it is dragged, a round Blob Brush shows the outline of the area `path.blob` adds,
/// which reaches half the brush beyond the ends of the stroke (round tips), not a line as wide as
/// the brush that ends square at them.
#[test]
fn blob_brush_preview_is_the_outline_of_the_area_with_round_tips() {
    use vectorcraft_geom::kurbo::Shape;
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("blobBrush");
    t.size = 20.0;
    assert_eq!(t.roundness, 100.0, "a round tip by default");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 300.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 300.0));
    let outlines: Vec<_> =
        t.overlays(&cx).into_iter().filter_map(|o| if let Overlay::Path { path, width, .. } = o { Some((path, width)) } else { None }).collect();
    assert!(outlines.iter().all(|(_, width)| f64::from(*width) < t.size / 2.0), "no line as wide as the brush: {outlines:?}");
    // The stroke's outline is the widest one (the tip at the pointer is another).
    let Some((path, _)) = outlines.iter().max_by(|a, b| a.0.bounding_box().width().total_cmp(&b.0.bounding_box().width())) else {
        panic!("no outline")
    };
    let b = path.bounding_box();
    assert!((b.x0 - 40.0).abs() < 0.5 && (b.x1 - 160.0).abs() < 0.5, "round tips reach half the size past the ends: {b:?}");
    assert!((b.y0 - 290.0).abs() < 0.5 && (b.y1 - 310.0).abs() < 0.5, "{b:?}");
    // Round, not square: the corner of the box a square-ended line would fill is outside the area.
    assert!(!path.contains(vectorcraft_geom::Point::new(41.0, 291.0)));
    assert!(path.contains(vectorcraft_geom::Point::new(45.0, 300.0)));
}

/// The Blob Brush has Keep selected and Merge only with selection, both off by default, and its
/// stroke sends both.
#[test]
fn blob_brush_sends_keep_selected_and_merge_only_with_selection() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("blobBrush");
    assert_eq!((t.options()["keepSelected"].clone(), t.options()["mergeOnlySelection"].clone()), (json!(false), json!(false)));
    for (keep, only) in [(false, false), (true, false), (false, true)] {
        t.set_option("keepSelected", &json!(keep));
        t.set_option("mergeOnlySelection", &json!(only));
        let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
        assert!(
            matches!(&acts[2][0], Action::Exec(c, v) if c == "path.blob" && v["keepSelected"] == keep && v["mergeOnlySelection"] == only),
            "{acts:?}"
        );
    }
    // The Eraser keeps Size (Fixed by default), Angle and Roundness: a round tip by default.
    assert_eq!(
        GestureTool::new("eraser").options(),
        json!({"size": 10.0, "sizeMode": "fixed", "sizeVariation": 0.0, "angle": 0.0, "roundness": 100.0})
    );
}

/// The Blob Brush has a Fidelity, 1.5 pt by default and 0.01 to 100 pt, and its stroke sends it.
#[test]
fn blob_brush_sends_its_fidelity() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("blobBrush");
    assert_eq!(t.options()["fidelity"], json!(1.5));
    for (set, sent) in [(1.5, 1.5), (6.0, 6.0), (0.001, 0.01), (500.0, 100.0)] {
        t.set_option("fidelity", &json!(set));
        assert_eq!(t.options()["fidelity"], json!(sent));
        let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
        assert!(matches!(&acts[2][0], Action::Exec(c, v) if c == "path.blob" && v["fidelity"] == json!(sent)), "{set}: {acts:?}");
    }
}

/// Shift keeps a Blob Brush stroke on the horizontal or vertical line through the press point,
/// with its axis chosen by the first clear axial movement. Crossing it never moves earlier samples.
#[test]
fn blob_brush_shift_keeps_the_stroke_horizontal_or_vertical() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    // Precise cursors: no tip outline, only the stroke's own overlay.
    let mut cx = ToolContext { precise_cursors: true, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = GestureTool::new("blobBrush");
    let ev = |k, x, y, m| PointerEvent::new(k, x, y).with_mods(m);
    for zoom in [0.5, 1.0, 4.0] {
        cx.zoom = zoom;
        t.pointer(&cx, &ev(PointerKind::Down, 0.0, 0.0, shift));
        t.pointer(&cx, &ev(PointerKind::Drag, 4.0 / zoom, 4.0 / zoom, shift));
        assert_eq!(t.points, vec![Point::ZERO]);
        assert!(t.overlays(&cx).is_empty());
        t.pointer(&cx, &ev(PointerKind::Drag, 1.0 / zoom, 0.5 / zoom, shift));
        assert_eq!(t.points.len(), 1, "small component has a strict bound");
        t.pointer(&cx, &ev(PointerKind::Drag, 1.0 / zoom, 0.0, shift));
        let first = t.points[1];
        t.pointer(&cx, &ev(PointerKind::Drag, 3.0 / zoom, 10.0 / zoom, shift));
        assert_eq!(t.points[1], first);
        assert!(t.points.iter().all(|p| p.y.abs() < 1e-9));
        // Release Shift: freehand resumes without relocating the constrained points.
        t.pointer(&cx, &ev(PointerKind::Drag, 8.0 / zoom, 12.0 / zoom, Mods::default()));
        assert_eq!(t.points.last(), Some(&Point::new(8.0 / zoom, 12.0 / zoom)));
        t.pointer(&cx, &ev(PointerKind::Drag, 0.0, 15.0 / zoom, shift));
        assert_eq!(t.points[1], first);
        assert!(t.points.last().unwrap().x.abs() < 1e-9);
        t.pointer(&cx, &ev(PointerKind::Up, 0.0, 15.0 / zoom, shift));
    }
    // Exact quarter turns preserve the strict half-pixel boundary in either direction.
    for angle in [-450.0, -180.0, -90.0, 90.0, 180.0, 450.0] {
        cx.constrain_angle = angle;
        cx.zoom = 1.0;
        let axes = vectorcraft_geom::constrain_axes(angle).unwrap();
        for (x, y) in [(1.0, 0.5), (0.5, 1.0), (-1.0, -0.5), (-0.5, -1.0)] {
            t.pointer(&cx, &ev(PointerKind::Down, 0.0, 0.0, shift));
            let at = axes * Point::new(x, y);
            t.pointer(&cx, &ev(PointerKind::Drag, at.x, at.y, shift));
            assert_eq!(t.points, vec![Point::ZERO], "angle {angle}, ({x}, {y}) stays pending");
            assert!(t.shift_line.is_none());
            let at = axes * Point::new(x, 0.0);
            t.pointer(&cx, &ev(PointerKind::Drag, at.x, at.y, shift));
            assert_eq!(t.shift_line.is_some(), x.abs() >= 1.0);
            t.clear();
        }
    }
    cx.constrain_angle = 0.0;
    // A pending diagonal released without locking makes the ordinary dot at the press.
    t.pointer(&cx, &ev(PointerKind::Down, 20.0, 20.0, shift));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 30.0, 30.0, shift));
    assert!(matches!(&acts[0], Action::Exec(c, v) if c == "path.blob" && v["points"] == json!([[20.0, 20.0]])), "{acts:?}");
}

/// Alt switches the Blob Brush to the Smooth tool: held before the press, or pressed before the
/// drag has moved, the cursor shows the Smooth tool, the drag shows a thin line and its release
/// smooths the selected paths (`path.smoothRegion`) instead of painting. Shift does not
/// constrain a smoothing drag, a stroke that has moved keeps painting when Alt goes down, and
/// with nothing selected the release changes no path.
#[test]
fn blob_brush_alt_switches_to_the_smooth_tool() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
    let alt = Mods { alt: true, ..Default::default() };
    let at = Point::new(50.0, 50.0);
    let smoothed = |acts: &[Vec<Action>]| -> Option<Value> {
        match acts.last().and_then(|a| a.first()) {
            Some(Action::Exec(c, v)) if c == "path.smoothRegion" => Some(v.clone()),
            _ => None,
        }
    };
    let mut t = GestureTool::new("blobBrush");
    assert_eq!(t.cursor(&cx, at, alt), Cursor::Smooth);
    assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::BlobBrush);
    let stroke = [(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)];
    let acts = drag_held(&mut t, &cx, &stroke, alt);
    assert!(acts[..acts.len() - 1].iter().all(|a| a.is_empty()), "{acts:?}");
    let v = smoothed(&acts).unwrap_or_else(|| panic!("{acts:?}"));
    assert_eq!(v, json!({"points": [[50.0, 150.0], [150.0, 150.0], [250.0, 160.0]], "radius": cx.tol(12.0), "fidelity": cx.tol(SMOOTH_FIDELITY)}));
    // Mid-drag the cursor stays the Smooth tool's when Alt is let go, and the line is thin with
    // no length label.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0).with_mods(alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0).with_mods(alt));
    assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::Smooth);
    let o = t.overlays(&cx);
    assert!(o.iter().any(|o| matches!(o, Overlay::Path { width, .. } if *width == 1.0)), "{o:?}");
    assert!(!o.iter().any(|o| matches!(o, Overlay::Measure { .. })), "{o:?}");
    // After another tool is chosen mid-drag, the Blob Brush shows its own cursor when it is chosen
    // again, and so does the Pencil after a drag that Alt key toggles to Smooth Tool made smooth.
    assert!(t.deactivate(&cx).is_empty());
    assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::BlobBrush);
    let mut pencil = GestureTool::new("pencil");
    pencil.set_option("altSmooth", &json!(true));
    pencil.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0).with_mods(alt));
    pencil.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0).with_mods(alt));
    assert!(pencil.deactivate(&cx).is_empty());
    assert_eq!(pencil.cursor(&cx, at, Mods::default()), Cursor::PencilNew);
    // Alt pressed before the drag has moved smooths too, and Shift leaves the points where the
    // pointer went.
    let alt_shift = Mods { alt: true, shift: true, ..Default::default() };
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 170.0).with_mods(alt_shift));
    let acts = [t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 160.0).with_mods(alt_shift))];
    let v = smoothed(&acts).unwrap_or_else(|| panic!("{acts:?}"));
    assert_eq!(v["points"], json!([[50.0, 150.0], [150.0, 170.0], [250.0, 160.0]]));
    // A stroke that has moved keeps painting with Alt down.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
    assert_eq!(t.cursor(&cx, at, alt), Cursor::BlobBrush);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 150.0).with_mods(alt));
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0).with_mods(alt));
    assert!(matches!(acts.as_slice(), [Action::Exec(c, v)] if c == "path.blob" && v["points"].as_array().unwrap().len() == 4), "{acts:?}");
    // So does a stroke that has moved too little to add a point.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.25, 150.0));
    assert_eq!(t.cursor(&cx, at, alt), Cursor::BlobBrush);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0).with_mods(alt));
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0).with_mods(alt));
    assert!(matches!(acts.as_slice(), [Action::Exec(c, _)] if c == "path.blob"), "{acts:?}");
    // Alt pressed while the pointer stands still at the press point does not paint on the release.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    assert_eq!(t.cursor(&cx, at, alt), Cursor::Smooth);
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 50.0, 150.0).with_mods(alt));
    assert!(acts.is_empty(), "{acts:?}");
    // A release away from the press point with no drag before it paints when Alt goes down.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 150.0).with_mods(alt));
    assert!(matches!(acts.as_slice(), [Action::Exec(c, _)] if c == "path.blob"), "{acts:?}");
    // With nothing selected an Alt drag sends nothing.
    let none = Selection::default();
    let empty = crate::testutil::cx(&d, &none, &p);
    let acts = drag_held(&mut t, &empty, &stroke, alt);
    assert!(acts.iter().all(|a| a.is_empty()), "{acts:?}");
}

/// #3799: Shift held at the press draws a straight Pencil line from the press point, at 45°
/// steps from the Constrain Angle, previewed as it is dragged; the Paintbrush keeps following
/// the hand.
#[test]
fn pencil_shift_draws_a_straight_line_at_45_degree_steps() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let shift = Mods { shift: true, ..Default::default() };
    let pts = |acts: &[Vec<Action>]| -> Vec<Vec<(f64, f64)>> {
        acts.iter()
            .filter_map(|a| a.iter().find_map(|a| if let Action::Preview(_, v) = a { Some(v["points"].clone()) } else { None }))
            .map(|v| v.as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap(), q[1].as_f64().unwrap())).collect())
            .collect()
    };
    let angle = |q: (f64, f64), from: (f64, f64)| (-(q.1 - from.1)).atan2(q.0 - from.0).to_degrees();
    let mut t = GestureTool::new("pencil");
    // A wavy drag heading up and right at about 40°: a line at 45°, as long as the drag.
    let acts = drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, -10.0), (50.0, -60.0), (100.0, -84.0)], shift);
    let all = pts(&acts);
    assert!(all.iter().all(|v| v.len() == 2 && v[0] == (0.0, 0.0)), "{all:?}");
    let end = *all.last().unwrap().last().unwrap();
    assert!((angle(end, (0.0, 0.0)) - 45.0).abs() < 1e-9, "{end:?}");
    assert!((end.0.hypot(end.1) - 100f64.hypot(84.0)).abs() < 1e-9, "{end:?}");
    let last = acts.last().unwrap();
    assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == false) && last[1] == Action::Commit, "{last:?}");
    // The steps turn with the Constrain Angle: at 30°, a drag at 40° gives 30°, one at 60° gives 75°.
    cx.constrain_angle = 30.0;
    for (to, want) in [((100.0, -84.0), 30.0), ((50.0, -86.6), 75.0)] {
        let all = pts(&drag_held(&mut t, &cx, &[(10.0, 10.0), (to.0 + 10.0, to.1 + 10.0)], shift));
        let end = *all.last().unwrap().last().unwrap();
        assert!((angle(end, (10.0, 10.0)) - want).abs() < 1e-9, "{to:?}: {all:?}");
    }
    // Without Shift, and with the Paintbrush, the stroke follows the hand.
    for (id, m) in [("pencil", Mods::default()), ("paintbrush", shift)] {
        let mut t = GestureTool::new(id);
        let all = pts(&drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, -10.0), (50.0, -60.0)], m));
        assert_eq!(all.last().unwrap().len(), 3, "{id}: {all:?}");
    }
}

/// #3799: a Shift line leaves no Smart Guides of its press on screen while it is dragged
/// (its end doesn't snap), and Alt at its release leaves it open.
#[test]
fn pencil_shift_line_drops_the_press_guides_and_stays_open_with_alt() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = GestureTool::new("pencil");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 102.0, 98.0).with_mods(shift));
    assert!(!t.overlays(&cx).is_empty(), "the press snaps to the corner and shows it");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 140.0).with_mods(shift));
    assert!(t.overlays(&cx).is_empty(), "{:?}", t.overlays(&cx));
    let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 300.0, 140.0).with_mods(shift_alt));
    assert!(matches!(&acts[0], Action::Preview(_, v) if v["closed"] == false && v["points"].as_array().unwrap().len() == 2), "{acts:?}");
}

/// Before a press, the Pencil shows its straight-line cursor exactly when the press would draw a
/// straight line; once the press is down, and with the Paintbrush, Shift leaves the tool's own
/// cursor.
#[test]
fn pencil_shows_the_straight_line_cursor_when_a_press_draws_a_line() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let at = Point::new(0.0, 0.0);
    let shift = Mods { shift: true, ..Default::default() };
    let alt = Mods { alt: true, ..Default::default() };
    let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
    // The last drag's preview: two points for a line, all three for a freehand stroke, none
    // for a smoothing drag.
    let straight = |acts: &[Vec<Action>]| {
        acts[acts.len() - 2].iter().any(|a| matches!(a, Action::Preview(_, v) if v["points"].as_array().is_some_and(|q| q.len() == 2)))
    };
    for alt_smooth in [false, true] {
        for m in [Mods::default(), shift, alt, shift_alt] {
            let mut t = GestureTool::new("pencil");
            t.set_option("altSmooth", &json!(alt_smooth));
            let shown = t.cursor(&cx, at, m) == Cursor::PencilStraight;
            let drawn = straight(&drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, -10.0), (50.0, -60.0)], m));
            assert_eq!(shown, drawn, "altSmooth {alt_smooth}, {m:?}");
        }
    }
    let mut t = GestureTool::new("pencil");
    assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::PencilNew);
    assert_eq!(t.cursor(&cx, at, shift), Cursor::PencilStraight);
    assert_eq!(t.cursor(&cx, at, alt), Cursor::PencilStraight, "Alt alone draws a line at any angle");
    assert_eq!(t.cursor(&cx, at, shift_alt), Cursor::PencilStraight, "Alt at the release leaves a straight line open");
    // While the line is drawn, the Pencil's own cursor; after the release, Shift shows it again.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(shift));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, -10.0).with_mods(shift));
    assert_eq!(t.cursor(&cx, at, shift), Cursor::Pencil, "while the line is drawn");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 40.0, -10.0).with_mods(shift));
    assert_eq!(t.cursor(&cx, at, shift), Cursor::PencilStraight, "after the release");
    // Shift pressed during a freehand stroke doesn't make it a line.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, -10.0).with_mods(shift));
    assert_eq!(t.cursor(&cx, at, shift), Cursor::Pencil, "a freehand stroke");
    assert_eq!(GestureTool::new("paintbrush").cursor(&cx, at, shift), Cursor::Paintbrush);
    assert_eq!(Cursor::PencilStraight.precise(), Cursor::PencilStraight, "Use Precise Cursors keeps the badge");
}

/// Before a press, the Pencil shows its new-path cursor (the star) exactly when the press draws
/// a new path: over an end of a selected open path, which the stroke continues, and over a
/// selected path, which it redraws, it does not. With nothing selected, with Edit Selected Paths
/// off and away from the paths, it does; while a stroke is drawn, it does not.
#[test]
fn pencil_shows_the_star_before_a_press_that_draws_a_new_path() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
    ]));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    // The stroke edits the selected path when its previews continue or redraw it.
    let edits = |acts: &[Vec<Action>]| {
        acts.iter().flatten().any(|a| matches!(a, Action::Preview(_, v) if v.get("extend").is_some() || v.get("redraw").is_some()))
    };
    let (end, middle, away) = (Point::new(405.0, 303.0), Point::new(350.0, 304.0), Point::new(350.0, 360.0));
    for edit_within in [12.0, 0.0] {
        for at in [end, Point::new(296.0, 302.0), middle, away, Point::new(200.0, 200.0)] {
            let mut t = GestureTool::new("pencil");
            t.set_option("editWithin", &json!(edit_within));
            let shown = t.cursor(&cx, at, Mods::default()) == Cursor::PencilNew;
            let acts = drag(&mut t, &cx, &[(at.x, at.y), (360.0, 330.0), (420.0, 350.0)], Mods::default());
            assert_eq!(shown, !edits(&acts), "editWithin {edit_within} at {at:?}");
        }
    }
    let mut t = GestureTool::new("pencil");
    assert_eq!(t.cursor(&cx, end, Mods::default()), Cursor::PencilContinue, "over an end the stroke continues");
    assert_eq!(t.cursor(&cx, middle, Mods::default()), Cursor::Pencil, "over the path the stroke redraws");
    assert_eq!(t.cursor(&cx, away, Mods::default()), Cursor::PencilNew, "away from the path");
    // While a stroke is drawn, the Pencil's own cursor.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, away.x, away.y));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 380.0, 380.0));
    assert_eq!(t.cursor(&cx, Point::new(380.0, 380.0), Mods::default()), Cursor::Pencil, "while a stroke is drawn");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 380.0, 380.0));
    assert_eq!(t.cursor(&cx, away, Mods::default()), Cursor::PencilNew, "after the release");
    // With nothing selected, every press draws a new path.
    let none = Selection::default();
    let cx_none = crate::testutil::cx(&d, &none, &p);
    assert_eq!(GestureTool::new("pencil").cursor(&cx_none, middle, Mods::default()), Cursor::PencilNew, "nothing selected");
    // Shift held: a straight line, with its own cursor; the Paintbrush keeps its cursor.
    let shift = Mods { shift: true, ..Default::default() };
    assert_eq!(GestureTool::new("pencil").cursor(&cx, away, shift), Cursor::PencilStraight);
    assert_eq!(GestureTool::new("paintbrush").cursor(&cx, away, Mods::default()), Cursor::Paintbrush);
}

/// #3799: a Shift line started on an end of a selected open path continues the path from that
/// end, constrained from the end itself.
#[test]
fn pencil_shift_line_continues_a_selected_path_from_its_end() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
    ]));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let mut t = GestureTool::new("pencil");
    // Pressed 5 pt off the end, released 100 pt right and 4 pt above it.
    let acts = drag_held(&mut t, &cx, &[(405.0, 303.0), (450.0, 290.0), (500.0, 296.0)], Mods { shift: true, ..Default::default() });
    let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
    assert_eq!(v["extend"], json!({"id": id.0, "end": "end"}));
    assert_eq!(v["points"], json!([[400.0, 300.0], [400.0 + 100f64.hypot(4.0), 300.0]]));
}

/// Before a press over an end of a selected open path, the Pencil shows its continuation cursor
/// exactly when the press continues the path, freehand or with a straight line; away from the
/// ends, with nothing selected, with Edit Selected Paths off, once the press is down and with
/// the Paintbrush, it does not.
#[test]
fn pencil_shows_the_continuation_cursor_over_an_end_a_line_continues() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
    ]));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let shift = Mods { shift: true, ..Default::default() };
    let alt = Mods { alt: true, ..Default::default() };
    let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
    // The release's stroke continues the path.
    let continues =
        |acts: &[Vec<Action>]| acts.last().and_then(|a| a.first()).is_some_and(|a| matches!(a, Action::Preview(_, v) if v.get("extend").is_some()));
    for alt_smooth in [false, true] {
        for m in [Mods::default(), shift, alt, shift_alt] {
            for (at, other_end) in [(Point::new(405.0, 303.0), (500.0, 296.0)), (Point::new(296.0, 302.0), (200.0, 310.0))] {
                let mut t = GestureTool::new("pencil");
                t.set_option("altSmooth", &json!(alt_smooth));
                let shown = t.cursor(&cx, at, m) == Cursor::PencilContinue;
                let acts = drag_held(&mut t, &cx, &[(at.x, at.y), (450.0, 250.0), other_end], m);
                assert_eq!(shown, continues(&acts), "altSmooth {alt_smooth}, {m:?} at {at:?}");
            }
        }
    }
    let mut t = GestureTool::new("pencil");
    let end = Point::new(405.0, 303.0);
    assert_eq!(t.cursor(&cx, end, shift), Cursor::PencilContinue);
    assert_eq!(t.cursor(&cx, end, alt), Cursor::PencilContinue, "Alt alone continues at any angle");
    assert_eq!(t.cursor(&cx, end, Mods::default()), Cursor::PencilContinue, "a freehand stroke continues it too");
    assert_eq!(t.cursor(&cx, Point::new(350.0, 360.0), shift), Cursor::PencilStraight, "away from the ends");
    // While the line is drawn, the Pencil's own cursor.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, end.x, end.y).with_mods(shift));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 303.0).with_mods(shift));
    assert_eq!(t.cursor(&cx, end, shift), Cursor::Pencil, "while the line is drawn");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 450.0, 303.0).with_mods(shift));
    // Edit Selected Paths off: the press draws a new line.
    t.set_option("editWithin", &json!(0.0));
    assert_eq!(t.cursor(&cx, end, shift), Cursor::PencilStraight, "Edit Selected Paths off");
    let none = Selection::default();
    let cx_none = crate::testutil::cx(&d, &none, &p);
    assert_eq!(GestureTool::new("pencil").cursor(&cx_none, end, shift), Cursor::PencilStraight, "nothing selected");
    assert_eq!(GestureTool::new("paintbrush").cursor(&cx, end, shift), Cursor::Paintbrush);
    assert_eq!(Cursor::PencilContinue.precise(), Cursor::PencilContinue, "Use Precise Cursors keeps the badge");
}

/// A Pencil stroke continuing a selected path shows the continuation cursor while a release would
/// connect it to an open end of another path selected at the press, and the closing ring while a
/// release would close the path because the stroke ends within Close Paths When Ends Are Within
/// of the path's other end, a straight line too; the release does what the cursor showed.
#[test]
fn a_continued_stroke_shows_where_it_connects_or_closes() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let add = |d: &mut vectorcraft_doc::Document, pts: &[(f64, f64)]| {
        let id = d.alloc_id();
        let mut els = vec![vectorcraft_geom::PathEl::MoveTo(Point::new(pts[0].0, pts[0].1))];
        els.extend(pts[1..].iter().map(|&(x, y)| vectorcraft_geom::PathEl::LineTo(Point::new(x, y))));
        let pd = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(els));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, pd, vectorcraft_doc::Appearance::default_art())).unwrap();
        id
    };
    let a = add(&mut d, &[(300.0, 300.0), (400.0, 300.0), (400.0, 400.0)]);
    let b = add(&mut d, &[(500.0, 500.0), (600.0, 500.0)]);
    let p = paint();
    let mut both = Selection::default();
    both.set([a, b]);
    let mut cx = cx(&d, &both, &p);
    cx.smart_guides = false;
    cx.snap_to_point = false;
    let none = Mods::default();
    let shift = Mods { shift: true, ..Default::default() };
    // Runs a stroke from `pts[0]`, giving the cursor at its last point before the release and the
    // release's `path.freehand` parameters.
    let run = |t: &mut GestureTool, pts: &[(f64, f64)], m: Mods| {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1).with_mods(m));
        for &(x, y) in &pts[1..] {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y).with_mods(m));
        }
        let (x, y) = pts[pts.len() - 1];
        let shown = t.cursor(&cx, Point::new(x, y), m);
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(m));
        let Some(Action::Preview(_, v)) = acts.first() else { panic!("{acts:?}") };
        (shown, v.clone())
    };
    let mut t = GestureTool::new("pencil");
    // To the other selected path's start: it connects, and doesn't close.
    let (shown, v) = run(&mut t, &[(400.0, 400.0), (450.0, 450.0), (498.0, 502.0)], none);
    assert_eq!(shown, Cursor::PencilContinue, "near the other path's end");
    assert_eq!(
        (v["extend"].clone(), v["connect"].clone(), v["closed"].clone()),
        (json!({"id": a.0, "end": "end"}), json!({"id": b.0, "end": "start"}), json!(false))
    );
    // Back near the path's own start (10 px from it): the ring, and the release closes it.
    let (shown, v) = run(&mut t, &[(400.0, 400.0), (350.0, 420.0), (290.0, 380.0), (292.0, 306.0)], none);
    assert_eq!(shown, Cursor::PencilClose, "near the path's other end");
    assert_eq!((v["extend"].clone(), v["closed"].clone()), (json!({"id": a.0, "end": "end"}), json!(true)));
    assert!(v.get("connect").is_none(), "{v}");
    // Farther than Close Paths When Ends Are Within (15 px): no ring, and it stays open.
    let (shown, v) = run(&mut t, &[(400.0, 400.0), (350.0, 420.0), (290.0, 380.0), (290.0, 330.0)], none);
    assert_eq!((shown, v["closed"].clone()), (Cursor::Pencil, json!(false)), "away from the other end");
    // A straight line from the end to near the start: the ring, and it closes the path.
    let (shown, v) = run(&mut t, &[(400.0, 400.0), (350.0, 350.0), (306.0, 306.0)], shift);
    assert_eq!(shown, Cursor::PencilClose, "a straight line near the other end");
    assert_eq!((v["points"].as_array().map(Vec::len), v["closed"].clone()), (Some(2), json!(true)));
    // Close Paths When Ends Are Within off: neither ring nor close.
    t.set_option("closeWithin", &json!(0.0));
    let (shown, v) = run(&mut t, &[(400.0, 400.0), (350.0, 420.0), (290.0, 380.0), (292.0, 306.0)], none);
    assert_eq!((shown, v["closed"].clone()), (Cursor::Pencil, json!(false)), "at 0 px");
}

/// #3846: Alt during a Shift line doesn't promise a close the release can't do, and with Alt
/// Toggles to Smooth, Alt pressed after the press doesn't turn the line into a smoothing drag.
#[test]
fn pencil_shift_line_ignores_alt_for_cursor_and_smoothing() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let shift = Mods { shift: true, ..Default::default() };
    let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
    let mut t = GestureTool::new("pencil");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(shift));
    assert_eq!(t.cursor(&cx, Point::new(0.0, 0.0), shift_alt), Cursor::Pencil);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 0.0, 0.0).with_mods(shift));

    let mut t = GestureTool::new("pencil");
    t.alt_smooth = true;
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(shift));
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 0.0).with_mods(shift_alt));
    assert!(matches!(acts.as_slice(), [Action::Begin(_), Action::Preview(..)]), "{acts:?}");
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 80.0, 0.0).with_mods(shift_alt));
    assert!(matches!(&acts[0], Action::Preview(_, v) if v["points"].as_array().unwrap().len() == 2), "{acts:?}");
}

/// #3846: a Shift line dragged back to its press point has no line: the preview is cancelled
/// and the release commits nothing (not the previous end of the line).
#[test]
fn pencil_shift_line_back_at_the_press_point_draws_nothing() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let shift = Mods { shift: true, ..Default::default() };
    // Released back at the press point, and dragged back there first.
    for back_in_drag in [false, true] {
        let mut t = GestureTool::new("pencil");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 0.0).with_mods(shift));
        let mut acts = vec![];
        if back_in_drag {
            // The shown line goes as soon as the pointer is back, not only at the release.
            let back = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 0.2, 0.0).with_mods(shift));
            assert_eq!(back, vec![Action::Cancel]);
            acts.extend(back);
        }
        acts.extend(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 0.2, 0.0).with_mods(shift)));
        assert!(acts.contains(&Action::Cancel), "{back_in_drag}: {acts:?}");
        assert!(!acts.contains(&Action::Commit) && !acts.iter().any(|a| matches!(a, Action::Preview(..))), "{back_in_drag}: {acts:?}");
    }
}

/// #3799: Alt held at the press draws a straight Pencil line at any angle from the press point
/// to the pointer, previewed as it is dragged, and left open by a release with Alt still held;
/// with Shift too it keeps Shift's 45° steps. Alt pressed after the press keeps the stroke
/// freehand and closes it at the release, and the Paintbrush keeps following the hand.
#[test]
fn pencil_alt_at_the_press_draws_a_straight_line_at_any_angle() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let alt = Mods { alt: true, ..Default::default() };
    let pts = |acts: &[Vec<Action>]| -> Vec<Vec<(f64, f64)>> {
        acts.iter()
            .filter_map(|a| a.iter().find_map(|a| if let Action::Preview(_, v) = a { Some(v["points"].clone()) } else { None }))
            .map(|v| v.as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap(), q[1].as_f64().unwrap())).collect())
            .collect()
    };
    let wavy = [(0.0, 0.0), (30.0, -10.0), (50.0, -60.0), (100.0, -84.0)];
    let mut t = GestureTool::new("pencil");
    assert_eq!(t.cursor(&cx, Point::new(0.0, 0.0), alt), Cursor::PencilStraight, "before the press");
    let acts = drag_held(&mut t, &cx, &wavy, alt);
    let all = pts(&acts);
    assert_eq!(all.len(), 4, "{all:?}");
    assert!(all.iter().all(|v| v.len() == 2 && v[0] == (0.0, 0.0)), "{all:?}");
    assert_eq!(all[1][1], (50.0, -60.0), "the preview follows the pointer at its own angle");
    assert_eq!(all[3][1], (100.0, -84.0), "{all:?}");
    let last = acts.last().unwrap();
    assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == false) && last[1] == Action::Commit, "{last:?}");
    // While the line is drawn, Alt shows no closing ring.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 40.0, 10.0).with_mods(alt));
    assert_eq!(t.cursor(&cx, Point::new(40.0, 10.0), alt), Cursor::Pencil);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 40.0, 10.0).with_mods(alt));
    // Shift with Alt: 45° steps.
    let all = pts(&drag_held(&mut t, &cx, &wavy, Mods { shift: true, alt: true, ..Default::default() }));
    let end = *all.last().unwrap().last().unwrap();
    assert!(((-end.1).atan2(end.0).to_degrees() - 45.0).abs() < 1e-9, "{end:?}");
    // Alt pressed after the press: freehand, closed at the release.
    let acts = drag(&mut t, &cx, &wavy, alt);
    assert_eq!(pts(&acts).last().unwrap().len(), 4, "{acts:?}");
    assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["closed"] == true), "{acts:?}");
    // The Paintbrush with Alt held follows the hand and closes.
    let mut b = GestureTool::new("paintbrush");
    let acts = drag_held(&mut b, &cx, &wavy, alt);
    assert_eq!(pts(&acts).last().unwrap().len(), 4, "{acts:?}");
    assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["closed"] == true), "{acts:?}");
}

/// #3799: with Alt key toggles to Smooth Tool on, Alt held at the press smooths as before and
/// draws no line.
#[test]
fn pencil_alt_with_alt_to_smooth_on_smooths_instead_of_drawing_a_line() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("pencil");
    t.set_option("altSmooth", &json!(true));
    let acts = drag_held(&mut t, &cx, &[(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)], alt);
    assert!(acts[..acts.len() - 1].iter().all(|a| a.is_empty()), "{acts:?}");
    assert!(matches!(&acts.last().unwrap()[0], Action::Exec(c, v) if c == "path.smoothRegion" && v["points"].as_array().unwrap().len() == 3));
}

/// While Alt switches the Pencil to the Smooth tool, the Pencil smooths within the Smooth tool's
/// radius at any Edit selected paths Within value, 0 included.
#[test]
fn pencil_alt_smooths_within_the_smooth_tools_radius() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let pts = [(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)];
    let radius = |acts: &[Vec<Action>]| match acts.last().and_then(|a| a.first()) {
        Some(Action::Exec(c, v)) if c == "path.smoothRegion" => v["radius"].as_f64().unwrap(),
        a => panic!("expected path.smoothRegion, got: {a:?}"),
    };
    let alt = Mods { alt: true, ..Default::default() };
    for zoom in [1.0, 2.0] {
        cx.zoom = zoom;
        let want = radius(&drag(&mut GestureTool::new("smooth"), &cx, &pts, Mods::default()));
        for within in [0.0, 5.0, 40.0] {
            let mut t = GestureTool::new("pencil");
            t.set_option("altSmooth", &json!(true));
            t.set_option("editWithin", &json!(within));
            assert_eq!(radius(&drag_held(&mut t, &cx, &pts, alt)), want, "zoom {zoom}, editWithin {within}");
        }
    }
}

/// The dashed paths among the overlays.
fn bands(t: &GestureTool, cx: &ToolContext) -> Vec<vectorcraft_geom::BezPath> {
    t.overlays(cx).into_iter().filter_map(|o| if let Overlay::Path { path, dashed: true, .. } = o { Some(path) } else { None }).collect()
}

/// How far `q` is from the polyline through `pts`.
fn dist_to_drag(q: Point, pts: &[Point]) -> f64 {
    pts.windows(2)
        .map(|w| {
            let ab = w[1] - w[0];
            let t = if ab.hypot2() > 0.0 { ((q - w[0]).dot(ab) / ab.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
            q.distance(w[0] + ab * t)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Check the band a smoothing drag through the tool's points shows at radius `r`: one dashed path,
/// every point of which is `r` from the drag, around an area that holds every point nearer than `r`
/// and none farther. Returns the path.
fn check_band(t: &GestureTool, cx: &ToolContext, r: f64) -> vectorcraft_geom::BezPath {
    use vectorcraft_geom::kurbo::Shape;
    let pts = &t.points;
    let found = bands(t, cx);
    let [outline] = found.as_slice() else { panic!("one band, got {found:?}") };
    for seg in outline.segments() {
        for s in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let q = seg.eval(s);
            let dq = dist_to_drag(q, pts);
            assert!((dq - r).abs() < 0.02 * r, "{q:?} on the outline is {dq} from the drag, radius {r}");
        }
    }
    let kept = t.band.take();
    let areas: Vec<_> = kept.iter().flat_map(|b| b.tiles.values().map(|t| t.area.to_bezpath())).collect();
    t.band.set(kept);
    let b = outline.bounding_box();
    let mut probes = 0;
    for i in 0..=80 {
        for j in 0..=80 {
            let q = Point::new(b.x0 - r + (b.width() + 2.0 * r) * f64::from(i) / 80.0, b.y0 - r + (b.height() + 2.0 * r) * f64::from(j) / 80.0);
            let dq = dist_to_drag(q, pts);
            let inside = areas.iter().any(|a| a.contains(q));
            if dq < 0.97 * r {
                assert!(inside, "{q:?} {dq} from the drag is inside the band, radius {r}");
                probes += 1;
            } else if dq > 1.03 * r {
                assert!(!inside, "{q:?} {dq} from the drag is outside the band, radius {r}");
            }
        }
    }
    assert!(probes > 100, "{probes}");
    outline.clone()
}

/// While a smoothing drag has a selected path to smooth, it shows dashed the band its release smooths:
/// everything within the radius it sends of the drag, round at both ends, around the drag's thin
/// line. Alt smoothing with the Pencil, Paintbrush and Blob Brush shows it too; a drag with nothing
/// selected and a Blob Brush stroke that paints do not.
#[test]
fn smoothing_drag_shows_the_band_it_smooths() {
    use vectorcraft_geom::kurbo::{ParamCurveArclen, Shape};
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let none = Selection::default();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let pts = [(50.0, 150.0), (150.0, 150.0)];
    for zoom in [1.0, 2.0, 0.5, 200.0] {
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        cx.zoom = zoom;
        let r = match drag(&mut GestureTool::new("smooth"), &cx, &pts, Mods::default()).last().and_then(|a| a.first()) {
            Some(Action::Exec(c, v)) if c == "path.smoothRegion" => v["radius"].as_f64().unwrap_or(0.0),
            a => panic!("expected path.smoothRegion, got: {a:?}"),
        };
        for (id, alt_smooth, m) in [("smooth", false, Mods::default()), ("pencil", true, alt), ("paintbrush", false, alt), ("blobBrush", false, alt)]
        {
            let mut t = GestureTool::new(id);
            t.set_option("altSmooth", &json!(alt_smooth));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1).with_mods(m));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, pts[1].0, pts[1].1).with_mods(m));
            let band = check_band(&t, &cx, r);
            let b = band.bounding_box();
            let near = |a: f64, w: f64| (a - w).abs() < 0.02 * r;
            assert!(near(b.x0, 50.0 - r) && near(b.x1, 150.0 + r) && near(b.y0, 150.0 - r) && near(b.y1, 150.0 + r), "{id} at zoom {zoom}: {b:?}");
            // The whole outline shows: two sides and two half circles.
            let length: f64 = band.segments().map(|s| s.arclen(1e-6 * r)).sum();
            let want = 200.0 + 2.0 * std::f64::consts::PI * r;
            assert!((length - want).abs() < 0.02 * want, "{id} at zoom {zoom}: outline {length}, band {want}");
            assert!(
                t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { width, dashed: false, .. } if *width == 1.0)),
                "{id}: the drag's line"
            );
        }
        // Nothing selected: the release smooths nothing, so no band shows.
        let mut empty = crate::testutil::cx(&d, &none, &p);
        empty.zoom = zoom;
        let mut t = GestureTool::new("smooth");
        t.pointer(&empty, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1));
        t.pointer(&empty, &PointerEvent::new(PointerKind::Drag, pts[1].0, pts[1].1));
        assert!(bands(&t, &empty).is_empty());
        // A Blob Brush stroke that paints shows its own area, not the band.
        let mut t = GestureTool::new("blobBrush");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, pts[1].0, pts[1].1));
        assert!(bands(&t, &cx).is_empty());
    }
}

/// The band shown while a long drag goes on, grown step by step, is one outline that holds every
/// point within the radius of the drag and none farther, whether the drag crosses itself or runs
/// on across the page; a zoom change mid-drag with the pointer still asks for frames until the band
/// covers the whole drag at the new radius, and the next drag does not keep the last one's band.
#[test]
fn smoothing_band_follows_a_long_drag() {
    use vectorcraft_geom::kurbo::Shape;
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    // A curve that crosses itself, and one that runs on across the page.
    let crossing: Vec<Point> =
        (0..240).map(|i| f64::from(i) * 0.05).map(|a| Point::new(250.0 + 80.0 * (a * 1.3).sin(), 250.0 + 60.0 * (a * 0.7).cos())).collect();
    let across: Vec<Point> =
        (0..900).map(f64::from).map(|i| Point::new(20.0 + 0.5 * i, 250.0 + 100.0 * (i * 0.05).sin() + 30.0 * (i * 0.37).sin())).collect();
    for pts in [crossing, across] {
        cx.zoom = 1.0;
        let mut t = GestureTool::new("smooth");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].x, pts[0].y));
        for q in &pts[1..] {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, q.x, q.y));
            assert_eq!(bands(&t, &cx).len(), 1);
        }
        check_band(&t, &cx, cx.tol(12.0));
        assert!(!t.overlays_pending(), "the band covers the drag");
        // The view zooms in mid-drag while the pointer holds still: the tool asks for frames until
        // the band covers the whole drag at the new radius.
        cx.zoom = 2.0;
        let mut frames = 1;
        bands(&t, &cx);
        while t.overlays_pending() {
            assert!(frames <= pts.len(), "the band reaches the whole drag");
            bands(&t, &cx);
            frames += 1;
        }
        let n = t.points.len();
        assert_eq!(frames, (n - 1).div_ceil(BAND_STEP * BAND_STEPS_PER_CALL), "frames for {n} points");
        check_band(&t, &cx, cx.tol(12.0));
        assert!(!t.overlays_pending());
        let last = pts[pts.len() - 1];
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, last.x, last.y));
        // The next drag shows only its own band.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 450.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 420.0, 450.0));
        let b = check_band(&t, &cx, cx.tol(12.0)).bounding_box();
        assert!(b.x0 > 390.0 && b.y0 > 440.0, "{b:?}");
    }
}

/// A segment of a drag too long for the squares at the current zoom (a jump drawn zoomed out,
/// viewed zoomed in, or one reaching far off the page) shows as the two sides of its band, and the
/// band goes on past it: the parts dragged before and after it show, and nothing stays pending.
/// Two such segments that cross show no edge inside each other's band, and one that runs back
/// along another shows its sides once.
#[test]
fn smoothing_band_shows_a_segment_too_long_for_its_squares() {
    use vectorcraft_geom::kurbo::{ParamCurveArclen, Shape};
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let before = [Point::new(100.0, 100.0), Point::new(130.0, 100.0)];
    let after = [Point::new(20_100.0, 100.0), Point::new(20_100.0, 130.0), Point::new(20_130.0, 130.0)];
    let far = [Point::new(1e17, 100.0)];
    let crossing = [(100.0, 100.0), (30_100.0, 100.0), (30_100.0, 130.0), (15_100.0, -20_000.0), (15_130.0, -20_000.0)].map(Point::from);
    let back = [(100.0, 100.0), (30_100.0, 100.0), (130.0, 100.0)].map(Point::from);
    for (case, pts) in [
        ("jump", [&before[..], &after[..]].concat()),
        ("far", [&before[..], &far[..], &after[..]].concat()),
        ("crossing", crossing.to_vec()),
        ("back", back.to_vec()),
    ] {
        // Drawn zoomed out, where the jump is about 42 half squares, then viewed at 1,000%, where
        // it is more than 4,096.
        cx.zoom = 0.1;
        let mut t = GestureTool::new("smooth");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].x, pts[0].y));
        for q in &pts[1..] {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, q.x, q.y));
            bands(&t, &cx);
        }
        cx.zoom = 10.0;
        let r = cx.tol(12.0);
        let mut frames = 0;
        let mut found = bands(&t, &cx);
        while t.overlays_pending() {
            assert!(frames < pts.len(), "{case}: the band reaches the whole drag");
            found = bands(&t, &cx);
            frames += 1;
        }
        let [outline] = found.as_slice() else { panic!("{case}: one band, got {found:?}") };
        for seg in outline.segments() {
            for s in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let q = seg.eval(s);
                let dq = dist_to_drag(q, &pts);
                assert!((dq - r).abs() < 0.02 * r, "{case}: {q:?} on the outline is {dq} from the drag, radius {r}");
            }
        }
        // The band of each long segment shows on both of its sides, as far as the release reaches,
        // except where the band of another part of the drag holds the side (on the page: a point
        // on a segment to 1e17 resolves only to about 10 units).
        let on_outline = |q: Point| {
            outline.segments().any(|seg| {
                (0..=8).any(|k| seg.eval(f64::from(k) / 8.0).distance(q) < 0.02 * r) || dist_to_drag(q, &[seg.start(), seg.end()]) < 0.02 * r
            })
        };
        for w in pts.windows(2).filter(|w| w[0].distance(w[1]) > 10_000.0 && w[0].x < 1e6 && w[1].x < 1e6) {
            let d = w[1] - w[0];
            let n = vectorcraft_geom::kurbo::Vec2::new(-d.y, d.x) * (r / d.hypot());
            for k in 1..40 {
                for q in [w[0].lerp(w[1], f64::from(k) / 40.0) + n, w[0].lerp(w[1], f64::from(k) / 40.0) - n] {
                    if dist_to_drag(q, &pts) > 0.98 * r {
                        assert!(on_outline(q), "{case}: {q:?} on a side of {w:?} shows");
                    }
                }
            }
        }
        if case == "back" {
            // Running back along the jump draws its sides once: two sides and two half circles.
            let length: f64 = outline.segments().map(|s| s.arclen(1e-6 * r)).sum();
            let want = 2.0 * 30_000.0 + 2.0 * std::f64::consts::PI * r;
            assert!((length - want).abs() < 0.01 * want, "{case}: outline {length}, band {want}");
        }
        let kept = t.band.take();
        let areas: Vec<_> = kept.iter().flat_map(|b| b.tiles.values().map(|t| t.area.to_bezpath())).collect();
        t.band.set(kept);
        // Every point near each short part dragged on either side of a long segment is in the band.
        for w in pts.windows(2).filter(|w| w[0].distance(w[1]) < 1_000.0) {
            for q in (0..=20).map(|k| w[0].lerp(w[1], f64::from(k) / 20.0)) {
                for (dx, dy) in [(0.0, 0.0), (0.9, 0.0), (-0.9, 0.0), (0.0, 0.9), (0.0, -0.9)] {
                    let probe = q + vectorcraft_geom::kurbo::Vec2::new(dx * r, dy * r);
                    assert!(areas.iter().any(|a| a.contains(probe)), "{case}: {probe:?} near the drag is in the band");
                }
            }
        }
    }
}

/// A drag drawn zoomed out whose every segment is too long for the squares once the view zooms in,
/// and whose segments all cross one another, is outlined over several calls: no call does more
/// than about [`BAND_WORK_PER_CALL`] of work on the long segments, and the band then shows in full.
#[test]
fn smoothing_band_of_many_long_segments_is_made_in_bounded_calls() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    // A star whose chords nearly all pass through its middle, 4,000 units each.
    let step = std::f64::consts::PI * (1.0 - 1.0 / 40.3);
    let pts: Vec<Point> = (0..161).map(|k| f64::from(k) * step).map(|a| Point::new(5000.0 + 2000.0 * a.cos(), 5000.0 + 2000.0 * a.sin())).collect();
    cx.zoom = 0.05;
    let mut t = GestureTool::new("smooth");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].x, pts[0].y));
    for q in &pts[1..] {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, q.x, q.y));
        bands(&t, &cx);
    }
    cx.zoom = 640.0;
    let r = cx.tol(SMOOTH_RADIUS);
    let work = |t: &GestureTool| {
        let kept = t.band.take();
        let w = kept.as_ref().map_or(0, |b| b.work);
        t.band.set(kept);
        w
    };
    let mut found = bands(&t, &cx);
    let mut frames = 1;
    let mut most = work(&t);
    while t.overlays_pending() {
        assert!(frames < 10_000, "the band reaches the whole drag");
        found = bands(&t, &cx);
        most = most.max(work(&t));
        frames += 1;
    }
    assert!(most <= 2 * BAND_WORK_PER_CALL, "{most} work in one call");
    assert!(frames > 2, "{frames} calls");
    let kept = t.band.take();
    let long = kept.as_ref().map_or(0, |b| b.long.len());
    t.band.set(kept);
    assert_eq!(long, pts.len() - 1, "every segment is long");
    let [outline] = found.as_slice() else { panic!("one band, got {}", found.len()) };
    for seg in outline.segments().step_by(7) {
        for s in [0.0, 0.5, 1.0] {
            let q = seg.eval(s);
            let dq = dist_to_drag(q, &pts);
            assert!((dq - r).abs() < 0.02 * r, "{q:?} on the outline is {dq} from the drag, radius {r}");
        }
    }
}

/// The band is kept in squares; a straight drag whose band edge runs along a side of a square, or
/// just inside or outside it, still shows that edge, and the sides between two squares the band
/// fills do not show.
#[test]
fn smoothing_band_shows_an_edge_along_a_square_side() {
    use vectorcraft_geom::kurbo::{ParamCurveArclen, Shape};
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    for zoom in [1.0, 3.0] {
        cx.zoom = zoom;
        let r = cx.tol(SMOOTH_RADIUS);
        let side = BAND_TILE_RADII * r;
        // A side of the squares near 230 (across and down: the grid is square).
        let at = (BAND_TILE_OFFSET + (230.0 / side - BAND_TILE_OFFSET).round()) * side;
        let eps = 1e-3 * r;
        for off in [0.0, 0.5 * eps, -0.5 * eps, 1.5 * eps, -1.5 * eps] {
            for (edge, upright) in [(-r, false), (r, false), (-r, true), (r, true)] {
                let c = at + edge + off;
                let ends = if upright {
                    [Point::new(c, 100.0), Point::new(c, 100.0 + 2.5 * side)]
                } else {
                    [Point::new(100.0, c), Point::new(100.0 + 2.5 * side, c)]
                };
                let mut t = GestureTool::new("smooth");
                t.pointer(&cx, &PointerEvent::new(PointerKind::Down, ends[0].x, ends[0].y));
                for i in 1..=20 {
                    let q = ends[0].lerp(ends[1], f64::from(i) / 20.0);
                    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, q.x, q.y));
                }
                let band = check_band(&t, &cx, r);
                // Two sides and two half circles, each shown once.
                let length: f64 = band.segments().map(|s| s.arclen(1e-6 * r)).sum();
                let want = 2.0 * 2.5 * side + 2.0 * std::f64::consts::PI * r;
                assert!(
                    (length - want).abs() < 0.02 * want,
                    "zoom {zoom}, edge {edge}, off {off}, upright {upright}: outline {length}, band {want}, {:?}",
                    band.bounding_box()
                );
            }
        }
    }
}

/// A drag whose band edge runs along a side of a square and then turns across it shows the edge
/// up to the turn and no line where the band goes on across the side.
#[test]
fn smoothing_band_shows_no_seam_where_it_turns_across_a_square_side() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    for zoom in [1.0, 3.0] {
        cx.zoom = zoom;
        let r = cx.tol(SMOOTH_RADIUS);
        let side = BAND_TILE_RADII * r;
        let at = (BAND_TILE_OFFSET + (230.0 / side - BAND_TILE_OFFSET).round()) * side;
        let eps = 1e-3 * r;
        for off in [0.0, 0.5 * eps, -0.5 * eps] {
            for (edge, upright) in [(-r, false), (r, false), (-r, true), (r, true)] {
                let c = at + edge + off;
                // Along the side, then across it.
                let turn = 100.0 + 1.5 * side;
                let corners = [(100.0, c), (turn, c), (turn, c - edge.signum() * 2.0 * side)];
                let pts: Vec<Point> = corners.into_iter().map(|(u, v)| if upright { Point::new(v, u) } else { Point::new(u, v) }).collect();
                let mut t = GestureTool::new("smooth");
                t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pts[0].x, pts[0].y));
                for w in pts.windows(2) {
                    for i in 1..=12 {
                        let q = w[0].lerp(w[1], f64::from(i) / 12.0);
                        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, q.x, q.y));
                    }
                }
                check_band(&t, &cx, r);
            }
        }
    }
}

/// One straight piece of a square's outline along a side, where the band goes on across only part
/// of it, shows only the part with nothing across.
#[test]
fn smoothing_band_square_shows_only_the_open_part_of_a_side() {
    use vectorcraft_geom::kurbo::{ParamCurveArclen, Shape};
    let mut band = SmoothBand::new(Point::ZERO, 10.0);
    let r = band.tile_rect((0, 0));
    let (x, y) = (r.x0, r.y1);
    let area = |x0: f64, y0: f64, x1: f64, y1: f64| vectorcraft_geom::shapes::rectangle(Rect::new(x0, y0, x1, y1));
    let tile = |area| BandTile { area, outline: vectorcraft_geom::BezPath::new(), near_long: vec![], cut: vectorcraft_geom::BezPath::new() };
    // Along the bottom side from 10 to 60, with the band across it from 40 to 60.
    band.tiles.insert((0, 0), tile(area(x + 10.0, y - 20.0, x + 60.0, y)));
    band.tiles.insert((0, 1), tile(area(x + 40.0, y, x + 60.0, y + 20.0)));
    let outline = band.tile_outline((0, 0));
    let along: f64 = outline.segments().filter(|s| s.bounding_box().height() < 1e-9 && (s.start().y - y).abs() < 1e-9).map(|s| s.arclen(1e-9)).sum();
    assert!((along - 30.0).abs() < 1e-6, "{along} of the side shows: {}", outline.to_svg());
    for s in outline.segments() {
        assert!(!((s.start().y - y).abs() < 1e-9 && (s.end().y - y).abs() < 1e-9 && s.start().x.max(s.end().x) > x + 40.0 + 1e-9), "{s:?}");
    }
}

/// A selection with no path the Smooth tool can change (a two-anchor line) shows no band.
#[test]
fn smoothing_drag_shows_no_band_without_a_path_it_smooths() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
    ]));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("smooth");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 400.0, 300.0));
    let o = t.overlays(&cx);
    assert!(!o.iter().any(|o| matches!(o, Overlay::Path { dashed: true, .. })), "{o:?}");
}

/// An Alt smoothing drag uses the Smooth tool's Fidelity for Pencil, Paintbrush and Blob Brush.
#[test]
fn alt_smoothing_uses_the_smooth_tools_fidelity() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let pts = [(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)];
    let fidelity = |acts: &[Vec<Action>]| match acts.last().and_then(|a| a.first()) {
        Some(Action::Exec(c, v)) if c == "path.smoothRegion" => v["fidelity"].as_f64().unwrap(),
        a => panic!("expected path.smoothRegion, got: {a:?}"),
    };
    let alt = Mods { alt: true, ..Default::default() };
    let want = fidelity(&drag(&mut GestureTool::new("smooth"), &cx, &pts, Mods::default()));
    for (id, alt_smooth) in [("pencil", true), ("blobBrush", false), ("paintbrush", false), ("paintbrush", true)] {
        let mut t = GestureTool::new(id);
        t.set_option("altSmooth", &json!(alt_smooth));
        t.set_option("fidelity", &json!(10.0));
        assert_eq!(fidelity(&drag_held(&mut t, &cx, &pts, alt)), want, "{id} altSmooth={alt_smooth}");
        t.set_option("smoothFidelity", &json!(7.5));
        assert_eq!(fidelity(&drag_held(&mut t, &cx, &pts, alt)), 7.5, "{id} altSmooth={alt_smooth}");
        // Drawing keeps the tool's own Fidelity; Smooth's setting stays out of its options.
        assert_eq!(t.options()["fidelity"], json!(10.0), "{id}");
        assert!(t.options().get("smoothFidelity").is_none(), "{id}");
    }
}

/// The Smooth tool's Fidelity is in screen pixels: the Smooth tool and an Alt smoothing drag with
/// the Pencil, Paintbrush or Blob Brush send it in points at Fidelity / zoom, so a drag smooths as
/// much on screen at any zoom.
#[test]
fn smooth_fidelity_is_in_screen_pixels_at_any_zoom() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let pts = [(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)];
    let fidelity = |acts: &[Vec<Action>]| match acts.last().and_then(|a| a.first()) {
        Some(Action::Exec(c, v)) if c == "path.smoothRegion" => v["fidelity"].as_f64().unwrap(),
        a => panic!("expected path.smoothRegion, got: {a:?}"),
    };
    let alt = Mods { alt: true, ..Default::default() };
    for zoom in [0.25, 1.0, 8.0] {
        cx.zoom = zoom;
        let mut t = GestureTool::new("smooth");
        t.set_option("fidelity", &json!(4.0));
        assert_eq!(fidelity(&drag(&mut t, &cx, &pts, Mods::default())), 4.0 / zoom, "smooth at {zoom}x");
        for (id, alt_smooth) in [("pencil", true), ("blobBrush", false), ("paintbrush", false)] {
            let mut t = GestureTool::new(id);
            t.set_option("altSmooth", &json!(alt_smooth));
            t.set_option("smoothFidelity", &json!(4.0));
            assert_eq!(fidelity(&drag_held(&mut t, &cx, &pts, alt)), 4.0 / zoom, "{id} at {zoom}x");
        }
    }
}

/// Alt pressed after a Paintbrush stroke begins closes it, and an empty selection still draws.
#[test]
fn paintbrush_alt_after_press_closes_without_smoothing_and_empty_selection_draws() {
    let (d, id) = doc_with_rect();
    let mut selected = Selection::default();
    selected.set([id]);
    let empty = Selection::default();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let pts = [(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)];
    let mut t = GestureTool::new("paintbrush");
    let selected_cx = cx(&d, &selected, &p);
    t.pointer(&selected_cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1));
    t.pointer(&selected_cx, &PointerEvent::new(PointerKind::Drag, pts[1].0, pts[1].1).with_mods(alt));
    // While drawing, Alt shows the closing ring though the selection could be smoothed.
    assert_eq!(t.cursor(&selected_cx, Point::new(pts[1].0, pts[1].1), alt), Cursor::PaintbrushClose);
    let last = t.pointer(&selected_cx, &PointerEvent::new(PointerKind::Up, pts[2].0, pts[2].1).with_mods(alt));
    assert!(matches!(last.first(), Some(Action::Preview(c, v)) if c == "path.freehand" && v["closed"] == true), "{last:?}");
    let cx = cx(&d, &empty, &p);
    let last = drag_held(&mut t, &cx, &pts, alt);
    assert!(matches!(last.last().and_then(|a| a.first()), Some(Action::Preview(c, v)) if c == "path.freehand" && v["closed"] == true), "{last:?}");
}

/// The Paintbrush shows Smooth before an Alt press only with a selected path, and through the
/// drag that smooths it, with Alt let go too.
#[test]
fn paintbrush_alt_cursor_matches_selection_at_press() {
    let (d, id) = doc_with_rect();
    let mut selected = Selection::default();
    selected.set([id]);
    let empty = Selection::default();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("paintbrush");
    assert_eq!(t.cursor(&cx(&d, &selected, &p), Point::new(50.0, 150.0), alt), Cursor::Smooth);
    assert_eq!(t.cursor(&cx(&d, &empty, &p), Point::new(50.0, 150.0), alt), Cursor::PaintbrushClose);
    let selected_cx = cx(&d, &selected, &p);
    t.pointer(&selected_cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0).with_mods(alt));
    t.pointer(&selected_cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0).with_mods(alt));
    assert_eq!(t.cursor(&selected_cx, Point::new(150.0, 150.0), alt), Cursor::Smooth);
    assert_eq!(t.cursor(&selected_cx, Point::new(150.0, 150.0), Mods::default()), Cursor::Smooth);
}

/// A selected text object leaves Alt Paintbrush drawing a closed stroke with its close cursor.
#[test]
fn paintbrush_alt_with_only_text_selected_draws_a_closed_stroke() {
    let (d, text_id) = doc_with_area_type();
    let mut selected = Selection::default();
    selected.set([text_id]);
    let p = paint();
    let cx = cx(&d, &selected, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("paintbrush");
    assert_eq!(t.cursor(&cx, Point::new(50.0, 150.0), alt), Cursor::PaintbrushClose);
    let acts = drag_held(&mut t, &cx, &[(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)], alt);
    assert!(acts.iter().flatten().any(|a| matches!(a, Action::Preview(c, v) if c == "path.freehand" && v["closed"] == true)), "{acts:?}");
    assert!(matches!(acts.last().and_then(|a| a.get(1)), Some(Action::Commit)), "{acts:?}");
}

/// A selected two-anchor line leaves Alt Paintbrush drawing a closed stroke with its close cursor.
#[test]
fn paintbrush_alt_with_only_two_anchor_line_selected_draws_a_closed_stroke() {
    let (mut d, _) = doc_with_rect();
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
    ]));
    d.insert(Some(layer), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut selected = Selection::default();
    selected.set([id]);
    let p = paint();
    let cx = cx(&d, &selected, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("paintbrush");
    assert_eq!(t.cursor(&cx, Point::new(50.0, 150.0), alt), Cursor::PaintbrushClose);
    let acts = drag_held(&mut t, &cx, &[(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)], alt);
    assert!(acts.iter().flatten().any(|a| matches!(a, Action::Preview(c, v) if c == "path.freehand" && v["closed"] == true)), "{acts:?}");
    assert!(matches!(acts.last().and_then(|a| a.get(1)), Some(Action::Commit)), "{acts:?}");
}

/// A selected two-anchor curve can be smoothed, so Alt Paintbrush smooths it with the Smooth cursor,
/// as it does a segment along a line that runs past its end and turns back; a two-anchor line whose
/// handles lie along it between its ends cannot, so Alt still draws the closed stroke.
#[test]
fn paintbrush_alt_with_a_two_anchor_curve_selected_smooths() {
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let cases = [((340.0, 250.0), (360.0, 350.0), true), ((450.0, 300.0), (250.0, 300.0), true), ((330.0, 300.0), (370.0, 300.0), false)];
    for (h1, h2, smooths) in cases {
        let (mut d, _) = doc_with_rect();
        let layer = d.layers[0].id;
        let id = d.alloc_id();
        let path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
            vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
            vectorcraft_geom::PathEl::CurveTo(Point::new(h1.0, h1.1), Point::new(h2.0, h2.1), Point::new(400.0, 300.0)),
        ]));
        d.insert(Some(layer), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
        let mut selected = Selection::default();
        selected.set([id]);
        let cx = cx(&d, &selected, &p);
        let mut t = GestureTool::new("paintbrush");
        let want = if smooths { Cursor::Smooth } else { Cursor::PaintbrushClose };
        assert_eq!(t.cursor(&cx, Point::new(50.0, 150.0), alt), want, "handles {h1:?} {h2:?}");
        let acts = drag_held(&mut t, &cx, &[(300.0, 300.0), (350.0, 300.0), (400.0, 300.0)], alt);
        let smoothed = matches!(acts.last().and_then(|a| a.first()), Some(Action::Exec(c, _)) if c == "path.smoothRegion");
        assert_eq!(smoothed, smooths, "handles {h1:?} {h2:?}: {acts:?}");
    }
}

/// #3799: Alt's line starts and ends on Smart Guides as a stroke does, and pressed on an end of
/// a selected open path it continues the path from that end to the pointer.
#[test]
fn pencil_alt_line_snaps_its_ends_and_continues_a_selected_path() {
    let (mut d, _) = doc_with_rect();
    let alt = Mods { alt: true, ..Default::default() };
    let p = paint();
    {
        let s = Selection::default();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("pencil");
        let acts = drag_held(&mut t, &cx, &[(102.0, 98.0), (151.0, 300.0), (198.0, 203.0)], alt);
        let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
        assert_eq!(v["points"], json!([[100.0, 100.0], [200.0, 200.0]]));
    }
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
    ]));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let mut t = GestureTool::new("pencil");
    let acts = drag_held(&mut t, &cx, &[(405.0, 303.0), (450.0, 340.0), (470.0, 330.0)], alt);
    let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
    assert_eq!(v["extend"], json!({"id": id.0, "end": "end"}));
    assert_eq!(v["points"], json!([[400.0, 300.0], [470.0, 330.0]]));
}

#[test]
fn pencil_alt_closes_and_continues_selected_path() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("paintbrush");
    let acts = drag(&mut t, &cx, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], Mods { alt: true, ..Default::default() });
    assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["closed"] == true && v["style"] == "brush"));
    // A selected closed rect is not continued; an open path would be (see engine tests).
    let mut s2 = Selection::default();
    s2.set([id]);
    let cx2 = crate::testutil::cx(&d, &s2, &p);
    let acts = drag(&mut t, &cx2, &[(100.0, 100.0), (90.0, 90.0)], Mods::default());
    assert!(matches!(&acts[1][1], Action::Preview(_, v) if v.get("extend").is_none()));
}

/// Edit selected paths within 0 turns editing off: a stroke that starts exactly on an end of a
/// selected open path draws a new path; at 12 px it continues the path from that end.
#[test]
fn edit_selected_paths_within_0_draws_a_new_path_from_an_end() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
    ]));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    for tool in ["pencil", "paintbrush"] {
        let mut t = GestureTool::new(tool);
        for (within, extends) in [(12.0, true), (0.0, false)] {
            t.set_option("editWithin", &json!(within));
            for (x, end) in [(400.0, "end"), (300.0, "start")] {
                let acts = drag(&mut t, &cx, &[(x, 300.0), (x + 20.0, 350.0), (x, 380.0)], Mods::default());
                let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{tool}: {acts:?}") };
                if extends {
                    assert_eq!(v["extend"], json!({"id": id.0, "end": end}), "{tool} from the {end}");
                } else {
                    assert!(v.get("extend").is_none() && v.get("redraw").is_none(), "{tool} from the {end}: {v}");
                }
            }
        }
    }
}

/// A Pencil stroke from an end of a selected open path to an end of another selected open path
/// connects the two; it does not when the other path is not selected, and the Paintbrush does
/// not connect.
#[test]
fn pencil_connects_a_stroke_from_one_selected_path_to_an_end_of_another() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let add = |d: &mut vectorcraft_doc::Document, a: (f64, f64), b: (f64, f64)| {
        let id = d.alloc_id();
        let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
            vectorcraft_geom::PathEl::MoveTo(Point::new(a.0, a.1)),
            vectorcraft_geom::PathEl::LineTo(Point::new(b.0, b.1)),
        ]));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
        id
    };
    let a = add(&mut d, (300.0, 300.0), (400.0, 300.0));
    let b = add(&mut d, (500.0, 400.0), (600.0, 400.0));
    let p = paint();
    let stroke = [(400.0, 300.0), (450.0, 350.0), (499.0, 401.0)];
    let mut both = Selection::default();
    both.set([a, b]);
    let mut one = Selection::default();
    one.set([a]);
    // The previews select the path continued: after the press only it is selected.
    let last = |tool: &str, at_press: &Selection| {
        let mut t = GestureTool::new(tool);
        t.pointer(&cx(&d, at_press, &p), &PointerEvent::new(PointerKind::Down, stroke[0].0, stroke[0].1));
        let after = cx(&d, &one, &p);
        t.pointer(&after, &PointerEvent::new(PointerKind::Drag, stroke[1].0, stroke[1].1));
        t.pointer(&after, &PointerEvent::new(PointerKind::Drag, stroke[2].0, stroke[2].1));
        let acts = t.pointer(&after, &PointerEvent::new(PointerKind::Up, stroke[2].0, stroke[2].1));
        let Some(Action::Preview(_, v)) = acts.first() else { panic!("{acts:?}") };
        v.clone()
    };
    let v = last("pencil", &both);
    assert_eq!((v["extend"].clone(), v["connect"].clone()), (json!({"id": a.0, "end": "end"}), json!({"id": b.0, "end": "start"})));
    assert!(last("paintbrush", &both).get("connect").is_none(), "the Paintbrush");
    let v = last("pencil", &one);
    assert!(v.get("extend").is_some() && v.get("connect").is_none(), "{v}");
}

/// Close paths when ends are within 0 turns closing off: a stroke that ends exactly where it
/// started stays open; at 15 px it is closed.
#[test]
fn close_paths_within_0_leaves_a_stroke_that_ends_at_its_start_open() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    for tool in ["pencil", "paintbrush"] {
        let mut t = GestureTool::new(tool);
        for (within, closes) in [(15.0, true), (0.0, false)] {
            t.set_option("closeWithin", &json!(within));
            let acts = drag(&mut t, &cx, &[(310.0, 330.0), (360.0, 350.0), (330.0, 390.0), (310.0, 330.0)], Mods::default());
            let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{tool}: {acts:?}") };
            assert_eq!(v["points"][0], v["points"][3], "{tool}");
            assert_eq!(v["closed"], json!(closes), "{tool} at {within} px");
        }
    }
}

/// While a stroke is drawn back near its start, the cursor shows the closing ring when a
/// release there closes the path (Close Paths When Ends Are Within), and the tool's own cursor
/// away from the start, at 0 pixels and after the release.
#[test]
fn a_stroke_near_its_start_shows_the_closing_cursor() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let none = Mods::default();
    let tools = [("pencil", Cursor::Pencil, Cursor::PencilClose), ("paintbrush", Cursor::Paintbrush, Cursor::PaintbrushClose)];
    for (tool, own, closing) in tools {
        for within in [15.0, 0.0] {
            let near = if within > 0.0 { closing } else { own };
            let mut t = GestureTool::new(tool);
            t.set_option("closeWithin", &json!(within));
            let mut at = |k, x, y| {
                let acts = t.pointer(&cx, &PointerEvent::new(k, x, y));
                (acts, t.cursor(&cx, Point::new(x, y), none))
            };
            assert_eq!(at(PointerKind::Down, 310.0, 330.0).1, own, "{tool} at the press");
            assert_eq!(at(PointerKind::Drag, 360.0, 350.0).1, own, "{tool} away from the start");
            assert_eq!(at(PointerKind::Drag, 330.0, 390.0).1, own, "{tool} away from the start");
            assert_eq!(at(PointerKind::Drag, 318.0, 335.0).1, near, "{tool} near the start at {within} px");
            assert_eq!(at(PointerKind::Drag, 345.0, 340.0).1, own, "{tool} away from the start again");
            assert_eq!(at(PointerKind::Drag, 318.0, 335.0).1, near, "{tool} back near the start at {within} px");
            let (acts, after) = at(PointerKind::Up, 318.0, 335.0);
            let Some(Action::Preview(_, v)) = acts.first() else { panic!("{tool}: {acts:?}") };
            assert_eq!(v["closed"], json!(within > 0.0), "{tool}: the release does what the cursor showed");
            assert_eq!(after, if tool == "pencil" { Cursor::PencilNew } else { own }, "{tool} after the release");
        }
    }
    // A straight Pencil line (Shift or Alt held at the press) stays open, so it shows no ring
    // near its start.
    for held in [Mods { shift: true, ..Mods::default() }, Mods { alt: true, ..Mods::default() }] {
        let mut t = GestureTool::new("pencil");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 310.0, 330.0).with_mods(held));
        for (x, y) in [(360.0, 330.0), (330.0, 390.0), (318.0, 335.0)] {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y).with_mods(held));
        }
        for m in [held, none] {
            assert_eq!(t.cursor(&cx, Point::new(318.0, 335.0), m), Cursor::Pencil, "a straight line, {held:?}, {m:?}");
        }
    }
}

/// A Paintbrush sample under a pixel from the last point is dropped, and a release there keeps
/// the last point: the closing cursor follows that point, not the pointer.
#[test]
fn the_paintbrush_closing_cursor_follows_the_point_a_release_keeps() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let none = Mods::default();
    let mut t = GestureTool::new("paintbrush");
    t.set_option("closeWithin", &json!(15.0));
    for (k, x, y) in [(PointerKind::Down, 310.0, 330.0), (PointerKind::Drag, 360.0, 350.0), (PointerKind::Drag, 330.0, 390.0)] {
        t.pointer(&cx, &PointerEvent::new(k, x, y));
    }
    // The last point kept is 15.5 from the start, outside Close Paths When Ends Are Within.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 325.5, 330.0));
    assert_eq!(t.cursor(&cx, Point::new(325.5, 330.0), none), Cursor::Paintbrush, "outside the closing distance");
    // The pointer moves 0.7 to 14.8 from the start; the sample is dropped.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 324.8, 330.0));
    assert_eq!(t.cursor(&cx, Point::new(324.8, 330.0), none), Cursor::Paintbrush, "the dropped sample shows no ring");
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 324.8, 330.0));
    let Some(Action::Preview(_, v)) = acts.first() else { panic!("{acts:?}") };
    assert_eq!(v["closed"], json!(false), "the release keeps the last point and leaves the path open");
}

#[test]
fn eraser_and_knife_exec_on_release() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("eraser");
    let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
    assert!(acts[1].is_empty());
    assert!(matches!(&acts[2][0], Action::Exec(c, v) if c == "path.eraseRegion" && v["size"] == 10.0));
    let mut k = GestureTool::new("knife");
    k.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    k.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
    assert_eq!(k.overlays(&cx).len(), 1);
    let a = k.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
    assert!(matches!(&a[0], Action::Exec(c, v) if c == "path.knife" && v["points"].as_array().unwrap().len() == 3));
}

/// The points of the Knife's release command.
fn knife_points(acts: &[Action]) -> Vec<(f64, f64)> {
    let Some(Action::Exec(c, v)) = acts.first() else { panic!("{acts:?}") };
    assert_eq!(c, "path.knife");
    v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap(), q[1].as_f64().unwrap())).collect()
}

fn overlay_points(t: &GestureTool, cx: &ToolContext) -> Vec<(f64, f64)> {
    let o = t.overlays(cx);
    let Some(Overlay::Path { path, dashed: true, .. }) = o.last() else { panic!("{o:?}") };
    path.elements().iter().filter_map(|e| e.end_point()).map(|q| (q.x, q.y)).collect()
}

/// With Alt held at the press, a wavy Knife drag shows and cuts along the straight line from the
/// press point to the pointer, even after Alt is released. Alt first pressed after the press
/// leaves the cut freehand, and a straight cut taken back to the press point sends no command.
#[test]
fn knife_alt_cuts_along_a_straight_line() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let none = Mods::default();
    let ev = |k, x, y, m| PointerEvent::new(k, x, y).with_mods(m);
    let mut t = GestureTool::new("knife");
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 150.0, alt));
    t.pointer(&cx, &ev(PointerKind::Drag, 100.0, 120.0, alt));
    t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 190.0, none));
    assert_eq!(overlay_points(&t, &cx), vec![(50.0, 150.0), (150.0, 190.0)]);
    // Alt let go: the cut stays straight.
    let a = t.pointer(&cx, &ev(PointerKind::Up, 250.0, 160.0, none));
    assert_eq!(knife_points(&a), vec![(50.0, 150.0), (250.0, 160.0)]);
    // Alt pressed on the first Drag event leaves the cut freehand.
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 150.0, none));
    t.pointer(&cx, &ev(PointerKind::Drag, 100.0, 120.0, alt));
    let a = t.pointer(&cx, &ev(PointerKind::Up, 250.0, 160.0, alt));
    assert_eq!(knife_points(&a), vec![(50.0, 150.0), (100.0, 120.0), (250.0, 160.0)]);
    // Alt pressed after the drag moved: the cut follows the drag.
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 150.0, none));
    t.pointer(&cx, &ev(PointerKind::Drag, 100.0, 120.0, none));
    t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 190.0, alt));
    let a = t.pointer(&cx, &ev(PointerKind::Up, 250.0, 160.0, alt));
    assert_eq!(knife_points(&a), vec![(50.0, 150.0), (100.0, 120.0), (150.0, 190.0), (250.0, 160.0)]);
    // Back at the press point the line goes, and the release cuts nothing.
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 150.0, alt));
    t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 190.0, alt));
    t.pointer(&cx, &ev(PointerKind::Drag, 50.2, 150.0, alt));
    assert!(t.overlays(&cx).is_empty());
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 50.2, 150.0, alt)), vec![]);
}

/// Alt first held after the Knife press leaves the cut freehand, including when the pointer has
/// not moved before Alt is pressed. The release command contains every collected point.
#[test]
fn knife_alt_after_press_keeps_freehand_cut() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("knife");
    for stationary_first in [false, true] {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
        if stationary_first {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 150.0).with_mods(alt));
        }
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 120.0).with_mods(alt));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 190.0));
        assert_eq!(overlay_points(&t, &cx), vec![(50.0, 150.0), (100.0, 120.0), (150.0, 190.0)]);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 160.0));
        assert_eq!(knife_points(&a), vec![(50.0, 150.0), (100.0, 120.0), (150.0, 190.0), (250.0, 160.0)]);
    }
}

/// Alt first held at release leaves a freehand Knife cut on its collected drag points.
#[test]
fn knife_alt_only_at_release_keeps_freehand_cut() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("knife");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 120.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 190.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 160.0).with_mods(alt));
    assert_eq!(knife_points(&a), vec![(50.0, 150.0), (100.0, 120.0), (150.0, 190.0), (250.0, 160.0)]);
}

/// With Shift and Alt held, the Knife's straight cut keeps to 45° steps from the canvas axes
/// and is as long as the line to the pointer. Shift is read on each Drag and Up event: when Shift
/// is let go, the line follows the pointer again. With Shift alone, the cut follows the drag.
#[test]
fn knife_shift_alt_cuts_at_45_degree_steps() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
    let alt = Mods { alt: true, ..Default::default() };
    let angle = |q: (f64, f64), from: (f64, f64)| (-(q.1 - from.1)).atan2(q.0 - from.0).to_degrees();
    let mut t = GestureTool::new("knife");
    // A wavy drag heading up and right at about 40°: a cut at 45°, as long as the drag.
    let acts = drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, -10.0), (50.0, -60.0), (100.0, -84.0)], shift_alt);
    let cut = knife_points(acts.last().unwrap());
    assert_eq!(cut.len(), 2, "{cut:?}");
    assert_eq!(cut[0], (0.0, 0.0));
    assert!((angle(cut[1], (0.0, 0.0)) - 45.0).abs() < 1e-9, "{cut:?}");
    assert!((cut[1].0.hypot(cut[1].1) - 100f64.hypot(84.0)).abs() < 1e-9, "{cut:?}");
    // A nearly vertical drag takes the nearest 90° step, with the same overlay and command endpoints.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0).with_mods(shift_alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 15.0, -90.0).with_mods(shift_alt));
    let shown = overlay_points(&t, &cx);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 15.0, -90.0).with_mods(shift_alt));
    let cut = knife_points(&a);
    assert_eq!(shown, cut);
    assert!((cut[1].0 - 10.0).abs() < 1e-9, "{cut:?}");
    assert!((cut[1].1 - (10.0 - 5f64.hypot(100.0))).abs() < 1e-9, "{cut:?}");
    // Shift goes down mid-drag and up again: the overlay steps, then follows the pointer.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, -84.0).with_mods(shift_alt));
    let shown = overlay_points(&t, &cx);
    assert!((angle(shown[1], (0.0, 0.0)) - 45.0).abs() < 1e-9, "{shown:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, -80.0).with_mods(alt));
    assert_eq!(overlay_points(&t, &cx), vec![(0.0, 0.0), (100.0, -80.0)]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 100.0, -84.0).with_mods(shift_alt));
    assert!((angle(knife_points(&a)[1], (0.0, 0.0)) - 45.0).abs() < 1e-9, "{a:?}");
    // A 30° Constrain Angle does not turn the Knife's horizontal or 45° cuts.
    cx.constrain_angle = 30.0;
    let acts = drag_held(&mut t, &cx, &[(10.0, 10.0), (110.0, 3.0)], shift_alt);
    assert_eq!(knife_points(acts.last().unwrap()), vec![(10.0, 10.0), (10.0 + 100f64.hypot(7.0), 10.0)]);
    let acts = drag_held(&mut t, &cx, &[(10.0, 10.0), (110.0, -74.0)], shift_alt);
    let cut = knife_points(acts.last().unwrap());
    assert!((angle(cut[1], (10.0, 10.0)) - 45.0).abs() < 1e-9, "{cut:?}");
    // Shift alone: the cut follows the drag.
    let acts = drag_held(&mut t, &cx, &[(0.0, 0.0), (30.0, -10.0), (50.0, -60.0)], Mods { shift: true, ..Default::default() });
    assert_eq!(knife_points(acts.last().unwrap()), vec![(0.0, 0.0), (30.0, -10.0), (50.0, -60.0)]);
}

/// With Smart Guides on, a straight Knife cut starts at the anchor or path its press snaps to,
/// and its release snaps the same way, as the Pencil's straight line's ends do. Centers and lines
/// up do not move the press. The overlay shows the same line.
#[test]
fn knife_alt_cut_snaps_where_smart_guides_snap() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("knife");
    // Near the rectangle's corner anchor: the cut starts on it, and the release near the opposite
    // corner ends on that one.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 101.0, 99.0).with_mods(alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 201.0, 198.0).with_mods(alt));
    assert_eq!(overlay_points(&t, &cx), vec![(100.0, 100.0), (200.0, 200.0)]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 201.0, 198.0).with_mods(alt));
    assert_eq!(knife_points(&a), vec![(100.0, 100.0), (200.0, 200.0)]);
    assert!(t.overlays(&cx).is_empty());
    // Near the top side, away from its anchors: the cut starts on the path.
    let acts = drag_held(&mut t, &cx, &[(130.0, 101.5), (260.0, 240.0)], alt);
    assert_eq!(knife_points(acts.last().unwrap()), vec![(130.0, 100.0), (260.0, 240.0)]);
    // Near the rectangle's center, or in line with its top side, the press stays where it is.
    for press in [(151.0, 149.0), (50.0, 101.5)] {
        let acts = drag_held(&mut t, &cx, &[press, (260.0, 240.0)], alt);
        assert_eq!(knife_points(acts.last().unwrap()), vec![press, (260.0, 240.0)]);
    }
    // An Alt-click on the snapped press cuts nothing, and neither does a drag back to where the
    // pointer went down.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 101.0, 99.0).with_mods(alt));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 101.0, 99.0).with_mods(alt)), vec![]);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 101.0, 99.0).with_mods(alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 201.0, 198.0).with_mods(alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 101.2, 99.0).with_mods(alt));
    assert!(!t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { dashed: true, .. })), "the line goes");
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 101.2, 99.0).with_mods(alt)), vec![]);
    // A freehand cut keeps its press.
    let acts = drag(&mut t, &cx, &[(101.0, 99.0), (150.0, 120.0), (201.0, 198.0)], Mods::default());
    assert_eq!(knife_points(acts.last().unwrap()), vec![(101.0, 99.0), (150.0, 120.0), (201.0, 198.0)]);
}

/// With Smart Guides off, a straight Knife cut starts where it is pressed, with Snap to Point on
/// or off.
#[test]
fn knife_alt_cut_without_smart_guides_keeps_its_press() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("knife");
    for snap_to_point in [false, true] {
        cx.snap_to_point = snap_to_point;
        let acts = drag_held(&mut t, &cx, &[(101.0, 99.0), (201.0, 198.0)], alt);
        assert_eq!(knife_points(acts.last().unwrap()), vec![(101.0, 99.0), (201.0, 198.0)], "Snap to Point {snap_to_point}");
    }
}

/// With Smart Guides on, a straight Knife cut does not start on art in a hidden layer, whose own
/// visibility is still on.
#[test]
fn knife_alt_cut_does_not_snap_to_hidden_art() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    d.node_mut(l).unwrap().visible = false;
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = GestureTool::new("knife");
    let acts = drag_held(&mut t, &cx, &[(101.0, 99.0), (201.0, 198.0)], alt);
    assert_eq!(knife_points(acts.last().unwrap()), vec![(101.0, 99.0), (201.0, 198.0)]);
}

/// With Smart Guides on, Shift and Alt keep the Knife's cut at 45° steps from the snapped press,
/// and an anchor near the release does not pull the cut off that step.
#[test]
fn knife_shift_alt_cut_steps_from_the_snapped_press() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
    let mut t = GestureTool::new("knife");
    // Pressed by the bottom-left corner, released by the top-right one.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 101.0, 199.0).with_mods(shift_alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 203.0, 104.0).with_mods(shift_alt));
    let shown = overlay_points(&t, &cx);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 203.0, 104.0).with_mods(shift_alt));
    let cut = knife_points(&a);
    assert_eq!(shown, cut);
    assert_eq!(cut[0], (100.0, 200.0));
    let step = 103f64.hypot(96.0) / 2f64.sqrt();
    assert!((cut[1].0 - (100.0 + step)).abs() < 1e-9 && (cut[1].1 - (200.0 - step)).abs() < 1e-9, "{cut:?}");
}

/// The Eraser's Angle and Roundness (Eraser Tool Options) go with its stroke, kept within
/// −180–180° and 1–100%; while it drags, a tip that is not round shows the outline of the area
/// it covers, and a round one the drag as wide as the tip.
#[test]
fn the_eraser_sends_its_tip_and_shows_the_area_it_covers() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    // Precise cursors: no tip outline, only the stroke's own overlay.
    let cx = ToolContext { precise_cursors: true, ..cx(&d, &s, &p) };
    let mut t = GestureTool::new("eraser");
    t.set_option("size", &json!(20));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
    let round = t.overlays(&cx);
    assert!(matches!(&round[..], [Overlay::Path { width, .. }] if *width == 20.0), "{round:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
    for (angle, roundness, sent) in [(90.0, 25.0, (90.0, 25.0)), (400.0, 0.0, (180.0, 1.0)), (-400.0, 250.0, (-180.0, 100.0))] {
        t.set_option("angle", &json!(angle));
        t.set_option("roundness", &json!(roundness));
        assert_eq!((t.options()["angle"].as_f64(), t.options()["roundness"].as_f64()), (Some(sent.0), Some(sent.1)));
        let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
        assert!(
            matches!(&acts[2][0], Action::Exec(c, v) if c == "path.eraseRegion" && v["size"] == 20.0 && v["angle"] == sent.0 && v["roundness"] == sent.1),
            "{acts:?}"
        );
    }
    // A tip 20 pt long and 5 pt wide, standing upright: along a horizontal drag from x 50 to
    // x 150 the outline reaches 2.5 pt either side of the drag and 10 pt above and below
    // its ends.
    t.set_option("angle", &json!(90));
    t.set_option("roundness", &json!(25));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
    let o = t.overlays(&cx);
    let [Overlay::Path { path, width, .. }] = &o[..] else { panic!("{o:?}") };
    assert_eq!(*width, 1.0);
    let b = vectorcraft_geom::Shape::bounding_box(path);
    assert!((b.x0 - 47.5).abs() < 0.1 && (b.x1 - 152.5).abs() < 0.1, "{b:?}");
    assert!((b.y0 - 140.0).abs() < 0.1 && (b.y1 - 160.0).abs() < 0.1, "{b:?}");
}

/// The Blob Brush's Angle and Roundness (Blob Brush Tool Options, 0° and 100% by default) go with
/// its stroke, kept within −180–180° and 0–100%; while it drags, a tip that is not round shows the
/// outline of the area it covers with the length label, and an Alt drag that smooths shows the
/// Smooth tool's thin line and the dashed band it smooths.
#[test]
fn the_blob_brush_sends_its_tip_and_shows_the_area_it_covers() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
    let mut t = GestureTool::new("blobBrush");
    assert_eq!((t.options()["angle"].as_f64(), t.options()["roundness"].as_f64()), (Some(0.0), Some(100.0)));
    t.set_option("size", &json!(20));
    for (angle, roundness, sent) in [(45.0, 25.0, (45.0, 25.0)), (400.0, -5.0, (180.0, 0.0)), (-400.0, 250.0, (-180.0, 100.0))] {
        t.set_option("angle", &json!(angle));
        t.set_option("roundness", &json!(roundness));
        assert_eq!((t.options()["angle"].as_f64(), t.options()["roundness"].as_f64()), (Some(sent.0), Some(sent.1)));
        let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
        assert!(
            matches!(&acts[2][0], Action::Exec(c, v) if c == "path.blob" && v["size"] == 20.0 && v["angle"] == sent.0 && v["roundness"] == sent.1),
            "{acts:?}"
        );
    }
    // A tip 20 pt long and 5 pt wide, standing upright: along a horizontal drag from x 50 to
    // x 150 the outline reaches 2.5 pt either side of the drag and 10 pt above and below its ends.
    t.set_option("angle", &json!(90));
    t.set_option("roundness", &json!(25));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
    let o = t.overlays(&cx);
    let Some(Overlay::Path { path, width, .. }) = o.iter().find(|o| matches!(o, Overlay::Path { .. })) else { panic!("{o:?}") };
    assert_eq!(*width, 1.0);
    let b = vectorcraft_geom::Shape::bounding_box(path);
    assert!((b.x0 - 47.5).abs() < 0.1 && (b.x1 - 152.5).abs() < 0.1, "{b:?}");
    assert!((b.y0 - 140.0).abs() < 0.1 && (b.y1 - 160.0).abs() < 0.1, "{b:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { .. })), "{o:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 150.0));
    let alt = Mods { alt: true, ..Default::default() };
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0).with_mods(alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0).with_mods(alt));
    let o = t.overlays(&cx);
    let [Overlay::Path { dashed: true, .. }, Overlay::Path { path, width, dashed: false, .. }] = &o[..] else { panic!("{o:?}") };
    assert_eq!(*width, 1.0);
    assert!(vectorcraft_geom::Shape::bounding_box(path).height() < 0.1, "{o:?}");
}

/// A drag of `n` points (2 to 10 pt apart) that circles round and crosses itself many times,
/// pressed at `pressure(i)` at point `i`, left down.
fn long_area_drag(t: &mut GestureTool, cx: &ToolContext, n: usize, pressure: impl Fn(usize) -> f32) {
    for i in 0..n {
        let s = i as f64;
        let r = 150.0 + 100.0 * (s * 0.0071).sin();
        let (x, y) = (300.0 + r * (s * 0.04).cos(), 300.0 + r * (s * 0.04).sin());
        let kind = if i == 0 { PointerKind::Down } else { PointerKind::Drag };
        t.pointer(cx, &PointerEvent { pressure: pressure(i), ..PointerEvent::new(kind, x, y) });
    }
    assert_eq!(t.points.len(), n, "every point is a sample");
}

/// The work the last `overlays` did on the area's outline, and whether it left some for later.
fn area_work(t: &GestureTool) -> (area::AreaWork, bool) {
    let Some(a) = t.area.take() else { panic!("no outline") };
    let w = (a.work, a.pending(&t.points));
    t.area.set(Some(a));
    assert_eq!(w.1, t.overlays_pending());
    w
}

/// The most work one `overlays` call can do past [`area::WORK_PER_CALL`]: the one step it is in
/// when it reaches it: a segment of the drag listed in, or checked near, up to 129 × 9 squares of
/// up to 128 segments each, and those segments checked once each.
const AREA_STEP: usize = 2 * (129 * 9 * 129 + 256);

/// Call `overlays` until the outline covers the whole drag, checking each call's work; the
/// number of calls it took.
fn drain_area(t: &GestureTool, cx: &ToolContext) -> usize {
    let mut calls = 0;
    loop {
        t.overlays(cx);
        calls += 1;
        let (w, pending) = area_work(t);
        assert!(w.points <= area::POINTS_PER_CALL, "{w:?}");
        assert!(w.outlined <= 2 * area::POINTS_PER_CALL + 2 * (area::PIECE_SEGMENTS + 1), "{w:?}");
        assert!(w.work <= area::WORK_PER_CALL + AREA_STEP, "{w:?}");
        assert!(w.joined <= area::JOIN_PIECES_PER_CALL + area::BLOCK_PIECES, "{w:?}");
        if !pending {
            return calls;
        }
        assert!(calls < 10_000, "the outline never covers the drag");
    }
}

/// The Blob Brush's and the Eraser's outline of a long drag does a bounded amount of work per
/// frame: each call takes at most `POINTS_PER_CALL` new points, outlines a bounded number of them
/// and checks outlines for a bounded amount of work, leaving the rest for later calls
/// (`overlays_pending`), which finish it with the pointer still. A call after that does nothing
/// until the drag moves, and then only the new step's work. The release sends the whole stroke.
#[test]
fn the_blob_brush_and_the_eraser_outline_a_long_drag_a_bounded_amount_per_frame() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { precise_cursors: true, ..cx(&d, &s, &p) };
    for (id, angle, roundness, pressure) in [
        ("blobBrush", 0.0, 100.0, false),
        ("blobBrush", 30.0, 40.0, false),
        ("eraser", -60.0, 30.0, false),
        ("blobBrush", 0.0, 100.0, true),
        ("eraser", 45.0, 50.0, true),
    ] {
        let mut t = GestureTool::new(id);
        t.set_option("size", &json!(10));
        t.set_option("angle", &json!(angle));
        t.set_option("roundness", &json!(roundness));
        if pressure {
            t.set_option("sizeMode", &json!("pressure"));
            t.set_option("sizeVariation", &json!(4));
        }
        long_area_drag(&mut t, &cx, 3000, |i| if pressure { 0.3 + 0.6 * ((i % 50) as f32 / 50.0) } else { 1.0 });
        let calls = drain_area(&t, &cx);
        assert!(calls > 1, "{id}: a 3,000-point drag takes several frames");
        // Nothing new: nothing to do, at any zoom (the outline is in document space).
        for zoom in [1.0, 4.0] {
            t.overlays(&ToolContext { zoom, ..cx });
            let (w, pending) = area_work(&t);
            assert_eq!((w.points, w.outlined, w.work, w.joined, pending), (0, 0, 0, 0, false), "{id}");
        }
        // One step more: one point, its piece outlined again, and the outlines near it checked.
        t.pointer(&cx, &PointerEvent { pressure: if pressure { 0.5 } else { 1.0 }, ..PointerEvent::new(PointerKind::Drag, 520.0, 300.0) });
        t.overlays(&cx);
        let (w, _) = area_work(&t);
        assert_eq!(w.points, 1, "{id}");
        assert!(w.outlined <= area::PIECE_SEGMENTS + 1, "{id}: {w:?}");
        if pressure {
            // 3,000 one-segment pieces: only the joined paths near the step are joined again.
            assert!(w.joined <= 8 * area::BLOCK_PIECES, "{id}: {w:?}");
        }
        drain_area(&t, &cx);
        let want = t.stroke_json();
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 520.0, 300.0));
        let Some(Action::Exec(_, v)) = acts.first() else { panic!("{acts:?}") };
        assert_eq!(v["points"], want, "{id}: the release sends every point");
        assert_eq!(v["points"].as_array().map(Vec::len), Some(3001), "{id}");
        assert!(t.area.take().is_none() && !t.overlays_pending(), "{id}: the release drops the outline");
    }
}

/// Every flattened point of the paths `o` (the outline shown).
fn shown_points(o: &[Overlay]) -> Vec<(Point, Point)> {
    let mut lines = vec![];
    for o in o {
        let Overlay::Path { path, .. } = o else { continue };
        let mut pen = Point::ZERO;
        let mut start = Point::ZERO;
        vectorcraft_geom::kurbo::flatten(path.elements().iter().copied(), 0.01, |el| match el {
            vectorcraft_geom::PathEl::MoveTo(p) => (pen, start) = (p, p),
            vectorcraft_geom::PathEl::LineTo(p) => {
                lines.push((pen, p));
                pen = p;
            }
            vectorcraft_geom::PathEl::ClosePath => {
                lines.push((pen, start));
                pen = start;
            }
            _ => {}
        });
    }
    lines
}

fn distance_to_line(q: Point, (a, b): (Point, Point)) -> f64 {
    let d = b - a;
    let t = if d.hypot2() > 0.0 { ((q - a).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
    q.distance(a + d * t)
}

/// Once a long drag's outline is finished it is the outline of the whole area its tip covers, as
/// `blob_area` and `eraser_area` give it: no line inside the area (where pieces of the drag
/// overlap or it crosses itself), and no gap along its edge.
#[test]
fn the_outline_of_a_long_drag_is_the_edge_of_the_whole_area() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { precise_cursors: true, ..cx(&d, &s, &p) };
    for (id, angle, roundness) in [("blobBrush", 0.0, 100.0), ("eraser", 30.0, 40.0)] {
        let mut t = GestureTool::new(id);
        t.set_option("size", &json!(12));
        t.set_option("angle", &json!(angle));
        t.set_option("roundness", &json!(roundness));
        long_area_drag(&mut t, &cx, 700, |_| 1.0);
        drain_area(&t, &cx);
        let shown = shown_points(&t.overlays(&cx));
        let tip = if id == "blobBrush" { blob_tip(angle, roundness) } else { eraser_tip(angle, roundness) };
        let squash = tip.inverse();
        let round: Vec<Point> = t.points.iter().map(|p| squash * *p).collect();
        // Nothing shows inside the area: every point shown is about the tip's radius from the drag.
        for &(a, _) in &shown {
            let q = squash * a;
            let inner = round.windows(2).map(|w| distance_to_line(q, (w[0], w[1]))).fold(f64::INFINITY, f64::min);
            assert!(inner > 6.0 * 0.9, "{id}: a line {inner} from the drag, inside the area, at {a:?}");
        }
        // The whole edge shows.
        let exact = if id == "blobBrush" {
            super::super::blob_area(&t.points, 12.0, angle, roundness)
        } else {
            super::super::eraser_area(&t.points, 12.0, angle, roundness)
        };
        let edge = shown_points(&[Overlay::Path { path: exact.to_bezpath(), color: FEEDBACK, width: 1.0, dashed: false }]);
        for &(a, _) in &edge {
            let gap = shown.iter().map(|l| distance_to_line(a, *l)).fold(f64::INFINITY, f64::min);
            assert!(gap < 0.6, "{id}: the edge at {a:?} is {gap} from the outline shown");
        }
    }
}

/// A drag with a very long step (a jump of 20,000 pt with a 1 pt tip) still finishes its outline
/// a bounded amount at a time, and the outline reaches both ends of the jump.
#[test]
fn a_long_jump_in_a_blob_brush_drag_is_outlined_a_bounded_amount_at_a_time() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { precise_cursors: true, ..cx(&d, &s, &p) };
    let mut t = GestureTool::new("blobBrush");
    t.set_option("size", &json!(1));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    for i in 1..400 {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, f64::from(i) * 2.0, 0.0));
    }
    // Over and back across the first steps, then out 20,000 pt and back over the same line.
    for x in [20_000.0, 400.0, 20_000.0, 10.0] {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, 0.3));
    }
    drain_area(&t, &cx);
    let o = t.overlays(&cx);
    let b = o
        .iter()
        .filter_map(|o| if let Overlay::Path { path, .. } = o { Some(vectorcraft_geom::Shape::bounding_box(path)) } else { None })
        .reduce(|a, b| a.union(b));
    let Some(b) = b else { panic!("{o:?}") };
    assert!((b.x0 + 0.5).abs() < 0.01 && (b.x1 - 20_000.5).abs() < 0.01, "{b:?}");
}

/// A drag of many long steps (60 times the tip's size) lists each step in a bounded number of the
/// grid's squares, so the grid does not grow with the steps' length.
#[test]
fn long_steps_of_a_drag_are_listed_in_a_bounded_number_of_squares() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { precise_cursors: true, ..cx(&d, &s, &p) };
    let mut t = GestureTool::new("eraser");
    t.set_option("size", &json!(1));
    t.set_option("roundness", &json!(50));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    for i in 1..=2000 {
        let x = if i % 2 == 0 { 0.0 } else { 60.0 };
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, f64::from(i) * 0.05));
    }
    assert_eq!(t.points.len(), 2001);
    drain_area(&t, &cx);
    let Some(a) = t.area.take() else { panic!("no outline") };
    assert!(a.listed() <= 2000 * 3 * (32 + 1) && a.listed() <= area::LISTINGS, "{}", a.listed());
}

/// Escape, the release and leaving the tool drop the outline built so far; the next drag starts
/// a new one, and its length label counts that drag alone.
#[test]
fn escape_and_leaving_the_tool_drop_the_drag_outline() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
    for id in ["blobBrush", "eraser"] {
        let mut t = GestureTool::new(id);
        t.set_option("roundness", &json!(50));
        long_area_drag(&mut t, &cx, 1000, |_| 1.0);
        t.overlays(&cx);
        assert!(t.overlays_pending(), "{id}: a 1,000-point drag takes more than one frame");
        t.key(&cx, ToolKey::Escape, Mods::default());
        assert!(t.area.take().is_none() && !t.overlays_pending(), "{id}: Escape drops it");
        long_area_drag(&mut t, &cx, 1000, |_| 1.0);
        t.overlays(&cx);
        t.deactivate(&cx);
        assert!(t.area.take().is_none() && !t.overlays_pending(), "{id}: leaving the tool drops it");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
        let o = t.overlays(&cx);
        let want = format!("L: {}", cx.len(100.0));
        assert_eq!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)), id == "blobBrush", "{id}: {o:?}");
    }
}

/// #4977: the Blob Brush and the Eraser show their tip at the pointer before a press and while
/// painting: its outline at its size, angle and roundness, a black line on a white halo, following
/// `[` / `]` and Tool Options at once. It goes when the hover ends, with precise cursors (Caps
/// Lock), while Cmd/Ctrl lends a selection tool and while Alt lends the Blob Brush the Smooth
/// tool. The other gesture tools show none.
#[test]
fn the_blob_brush_and_the_eraser_show_their_tip_at_the_pointer() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let bounds = |t: &GestureTool, cx: &ToolContext| match &t.overlays(cx)[..] {
        [Overlay::Path { path: halo, color: [0xff, 0xff, 0xff], width: 3.0, .. }, Overlay::Path { path, color: [0, 0, 0], width: 1.0, .. }] => {
            assert_eq!(halo, path);
            Some(vectorcraft_geom::Shape::bounding_box(path))
        }
        [] => None,
        o => panic!("{o:?}"),
    };
    let near = |b: Option<vectorcraft_geom::Rect>, want: (f64, f64, f64, f64)| {
        let b = b.unwrap_or_else(|| panic!("no tip"));
        assert!([b.x0 - want.0, b.y0 - want.1, b.x1 - want.2, b.y1 - want.3].iter().all(|e| e.abs() < 0.01), "{b:?} vs {want:?}");
    };
    let hover = |t: &mut GestureTool, cx: &ToolContext, m: Mods| t.pointer(cx, &PointerEvent::new(PointerKind::Move, 100.0, 50.0).with_mods(m));
    for id in ["blobBrush", "eraser"] {
        let mut t = GestureTool::new(id);
        assert_eq!(bounds(&t, &cx), None, "{id}: nothing before the pointer comes");
        hover(&mut t, &cx, Mods::default());
        near(bounds(&t, &cx), (95.0, 45.0, 105.0, 55.0));
        // `]` grows it at once, without a move; Tool Options too.
        t.key(&cx, ToolKey::BracketRight, Mods::default());
        near(bounds(&t, &cx), (94.5, 44.5, 105.5, 55.5));
        t.set_option("size", &json!(20));
        near(bounds(&t, &cx), (90.0, 40.0, 110.0, 60.0));
        // Upright and a quarter as wide: 5 pt across, 20 pt tall.
        t.set_option("angle", &json!(90));
        t.set_option("roundness", &json!(25));
        near(bounds(&t, &cx), (97.5, 40.0, 102.5, 60.0));
        // It follows the drag, over the stroke's footprint.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 50.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 50.0));
        let o = t.overlays(&cx);
        assert!(matches!(&o[o.len() - 2..], [Overlay::Path { width: 3.0, .. }, Overlay::Path { color: [0, 0, 0], width: 1.0, .. }]), "{id}: {o:?}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 200.0, 50.0));
        let precise = ToolContext { precise_cursors: true, ..crate::testutil::cx(&d, &s, &p) };
        assert_eq!(bounds(&t, &precise), None, "{id}: precise cursors show the crosshair alone");
        t.hover_ended(&cx);
        assert_eq!(bounds(&t, &cx), None, "{id}: gone with the hover");
        hover(&mut t, &cx, Mods::default());
        assert!(t.deactivate(&cx).is_empty());
        assert_eq!(bounds(&t, &cx), None, "{id}: gone with the tool");
    }
    // Alt before a press lends the Blob Brush the Smooth tool, and Cmd/Ctrl either tool a
    // selection tool, whose cursors show instead.
    let mut t = GestureTool::new("blobBrush");
    hover(&mut t, &cx, Mods { alt: true, ..Default::default() });
    assert_eq!(bounds(&t, &cx), None);
    let mut t = GestureTool::new("eraser");
    hover(&mut t, &cx, Mods { cmd: true, ..Default::default() });
    assert_eq!(bounds(&t, &cx), None);
    for id in ["pencil", "paintbrush", "smooth", "knife", "pathEraser", "join"] {
        let mut t = GestureTool::new(id);
        hover(&mut t, &cx, Mods::default());
        assert!(!t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { width, .. } if *width == 3.0)), "{id}");
    }
}

#[test]
fn smooth_needs_selection_and_brackets_resize() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GestureTool::new("smooth");
    let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
    assert!(acts.iter().all(|a| a.is_empty()));
    let mut b = GestureTool::new("blobBrush");
    b.key(&cx, ToolKey::BracketRight, Mods::default());
    assert_eq!(b.options()["size"], 11.0);
}

/// The Smooth tool shows Not Allowed while no selected path can be smoothed: nothing selected, only
/// type, a path of two anchors or a locked path. A selected path anywhere, alone or in a selected
/// group, gives the Smooth cursor wherever the pointer is, and the cursor follows the selection while
/// the tool stays chosen.
#[test]
fn smooth_shows_not_allowed_without_a_selected_path_to_smooth() {
    use std::sync::Arc;
    let (mut d, rect) = doc_with_rect();
    let layer = d.layers[0].id;
    let line_id = d.alloc_id();
    let line = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_vec(vec![
        vectorcraft_geom::PathEl::MoveTo(Point::new(300.0, 300.0)),
        vectorcraft_geom::PathEl::LineTo(Point::new(400.0, 300.0)),
    ]));
    d.insert(Some(layer), 1, vectorcraft_doc::Node::path(line_id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let square = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(300.0, 350.0, 350.0, 400.0));
    let inner = vectorcraft_doc::Node::path(d.alloc_id(), square, vectorcraft_doc::Appearance::default_art());
    let group = vectorcraft_doc::Node::group(d.alloc_id(), vec![Arc::new(inner)]);
    let group_id = group.id;
    d.insert(Some(layer), 2, group).unwrap();
    let (text_doc, text) = doc_with_area_type();
    let mut locked = d.clone();
    locked.node_mut(rect).unwrap().locked = true;
    let p = paint();
    let sel = |ids: &[NodeId]| {
        let mut s = Selection::default();
        s.set(ids.iter().copied());
        s
    };
    let t = GestureTool::new("smooth");
    let far = Point::new(480.0, 20.0);
    let cases: [(&vectorcraft_doc::Document, Selection, Cursor, &str); 7] = [
        (&d, Selection::default(), Cursor::NotAllowed, "nothing selected"),
        (&text_doc, sel(&[text]), Cursor::NotAllowed, "type"),
        (&d, sel(&[line_id]), Cursor::NotAllowed, "a two-anchor path"),
        (&locked, sel(&[rect]), Cursor::NotAllowed, "a locked path"),
        (&d, sel(&[rect]), Cursor::Smooth, "a path away from the pointer"),
        (&d, sel(&[line_id, rect]), Cursor::Smooth, "a path beside one it cannot smooth"),
        (&d, sel(&[group_id]), Cursor::Smooth, "a group holding a path"),
    ];
    for (doc, s, c, what) in &cases {
        assert_eq!(t.cursor(&cx(doc, s, &p), far, Mods::default()), *c, "{what}");
    }
    // The cursor changes as the selection does, with the same tool.
    let (none, one) = (Selection::default(), sel(&[rect]));
    for (s, c) in [(&none, Cursor::NotAllowed), (&one, Cursor::Smooth), (&none, Cursor::NotAllowed)] {
        assert_eq!(t.cursor(&cx(&d, s, &p), Point::new(150.0, 150.0), Mods::default()), c);
    }
    // A drag with Not Allowed showing sends nothing.
    let mut t = GestureTool::new("smooth");
    let acts = drag(&mut t, &cx(&d, &none, &p), &[(50.0, 150.0), (150.0, 150.0), (250.0, 150.0)], Mods::default());
    assert!(acts.iter().all(|a| a.is_empty()), "{acts:?}");
}

/// The Blob Brush's Alt shows Not Allowed with no selected path to smooth, and the Smooth cursor with
/// one; the Paintbrush's Alt without one keeps the closing cursor, since its release closes the stroke.
#[test]
fn borrowed_smooth_shows_not_allowed_without_a_selected_path_to_smooth() {
    let (d, rect) = doc_with_rect();
    let none = Selection::default();
    let mut one = Selection::default();
    one.set([rect]);
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let at = Point::new(400.0, 400.0);
    let blob = GestureTool::new("blobBrush");
    assert_eq!(blob.cursor(&cx(&d, &none, &p), at, alt), Cursor::NotAllowed);
    assert_eq!(blob.cursor(&cx(&d, &one, &p), at, alt), Cursor::Smooth);
    assert_eq!(blob.cursor(&cx(&d, &none, &p), at, Mods::default()), Cursor::BlobBrush);
    let mut pencil = GestureTool::new("pencil");
    pencil.set_option("altSmooth", &json!(true));
    assert_eq!(pencil.cursor(&cx(&d, &none, &p), at, alt), Cursor::NotAllowed);
    assert_eq!(pencil.cursor(&cx(&d, &one, &p), at, alt), Cursor::Smooth);
    let brush = GestureTool::new("paintbrush");
    assert_eq!(brush.cursor(&cx(&d, &none, &p), at, alt), Cursor::PaintbrushClose);
    assert_eq!(brush.cursor(&cx(&d, &one, &p), at, alt), Cursor::Smooth);
}

/// A Smooth release sends no command while the selection holds no path it can smooth, from the
/// Smooth tool and from the Blob Brush and Pencil with Alt; with a path selected it smooths.
#[test]
fn smoothing_release_sends_nothing_without_a_selected_path_to_smooth() {
    let (text_doc, text) = doc_with_area_type();
    let (d, rect) = doc_with_rect();
    let mut type_only = Selection::default();
    type_only.set([text]);
    let mut one = Selection::default();
    one.set([rect]);
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let stroke = [(50.0, 150.0), (150.0, 150.0), (250.0, 160.0)];
    let smooths = |acts: &[Vec<Action>]| acts.iter().flatten().any(|a| matches!(a, Action::Exec(c, _) if c == "path.smoothRegion"));
    for (id, mods) in [("smooth", Mods::default()), ("blobBrush", alt), ("pencil", alt)] {
        let tool = || {
            let mut t = GestureTool::new(id);
            if id == "pencil" {
                t.set_option("altSmooth", &json!(true));
            }
            t
        };
        let mut c = cx(&text_doc, &type_only, &p);
        c.smart_guides = false;
        let acts = drag_held(&mut tool(), &c, &stroke, mods);
        assert!(!smooths(&acts), "{id} with only type selected: {acts:?}");
        let mut c = cx(&d, &one, &p);
        c.smart_guides = false;
        let acts = drag_held(&mut tool(), &c, &stroke, mods);
        assert!(smooths(&acts), "{id} with a path selected: {acts:?}");
    }
}

#[test]
fn pencil_alt_toggles_to_smooth_when_enabled_and_closes_when_disabled() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);

    // Off by default, as Pencil Tool Options ships it; on, Alt lends the Smooth tool.
    let mut t = GestureTool::new("pencil");
    assert_eq!(t.options()["altSmooth"], json!(false));
    t.set_option("altSmooth", &json!(true));

    // When alt is pressed, cursor shows smooth tool.
    let alt_mod = Mods { alt: true, ..Default::default() };
    assert_eq!(t.cursor(&cx, Point::new(50.0, 50.0), alt_mod), Cursor::Smooth);
    assert_eq!(t.cursor(&cx, Point::new(50.0, 50.0), Mods::default()), Cursor::PencilNew);

    // Alt-drag over selected path smooths it.
    let acts = drag_held(&mut t, &cx, &[(50.0, 150.0), (150.0, 150.0), (250.0, 150.0)], alt_mod);
    assert!(acts[1].is_empty(), "drag does not preview freehand path while smoothing");
    let last = acts.last().unwrap();
    assert!(
        matches!(&last[0], Action::Exec(c, v) if c == "path.smoothRegion" && v["points"].as_array().unwrap().len() == 3),
        "expected path.smoothRegion, got: {last:?}"
    );

    // When altSmooth is disabled:
    t.set_option("altSmooth", &json!(false));
    assert_eq!(t.options()["altSmooth"], json!(false));
    // Before a press, Alt gives a straight line rather than a closed path.
    assert_eq!(t.cursor(&cx, Point::new(50.0, 50.0), alt_mod), Cursor::PencilStraight);

    // A drag released with Alt held draws a closed pencil path.
    let empty_sel = Selection::default();
    let cx_empty = crate::testutil::cx(&d, &empty_sel, &p);
    let acts = drag(&mut t, &cx_empty, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], alt_mod);
    assert_eq!(acts[1][0], Action::Begin("Pencil".into()));
    let last = acts.last().unwrap();
    assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == true && v["style"] == "pencil"));
    assert_eq!(last[1], Action::Commit);
}
