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
/// while it is drawn, fit it at its own Fidelity. A coarser Fidelity previews as it is kept.
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
        for (zoom, fidelity, drawn) in [(1.0, 0.01, 0.5), (4.0, 0.01, 0.125), (1.0, 1.5, 1.5)] {
            cx.zoom = zoom;
            let mut t = GestureTool::new(id);
            t.set_option("fidelity", &json!(fidelity));
            let acts = drag(&mut t, &cx, &stroke, Mods::default());
            let (release, during) = acts.split_last().unwrap();
            assert_eq!(fidelities(&during.concat()), [drawn; 3], "{id} at {zoom}x, {fidelity} pt");
            assert_eq!(fidelities(release), [fidelity], "{id} at {zoom}x, {fidelity} pt: the release");
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
            for &(x, y) in &stroke[1..] {
                t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y));
            }
            let acts = t.deactivate(&cx);
            let kept = if drawn > fidelity { [fidelity].to_vec() } else { vec![] };
            assert_eq!(fidelities(&acts), kept, "{id} at {zoom}x, {fidelity} pt: another tool chosen");
            assert_eq!(acts.last(), Some(&Action::Commit));
        }
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
    let (d, _) = doc_with_rect();
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
    let mut t = GestureTool::new("pencil");
    t.set_option("altSmooth", &json!(true));
    assert_eq!(t.cursor(&cx, at, alt), Cursor::Smooth);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0));
    assert_eq!(t.cursor(&cx, at, alt), Cursor::Pencil, "a stroke that has begun");
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 20.0, 0.0).with_mods(alt));
    assert!(matches!(&acts[0], Action::Preview(_, v) if v["closed"] == false), "{acts:?}");
    // A smoothing drag keeps the Smooth cursor when Alt is let go.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0).with_mods(alt));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 5.0).with_mods(alt));
    assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::Smooth, "a smoothing drag");
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
    let mut cx = cx(&d, &s, &p);
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
    assert_eq!(v, json!({"points": [[50.0, 150.0], [150.0, 150.0], [250.0, 160.0]], "radius": cx.tol(12.0), "fidelity": SMOOTH_FIDELITY}));
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
    assert_eq!(Cursor::PencilStraight.precise(), Cursor::Crosshair, "Use Precise Cursors");
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
    assert_eq!(t.cursor(&cx, end, Mods::default()), Cursor::Pencil, "over an end the stroke continues");
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
/// exactly when the press would draw a straight line that continues the path; away from the
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
    // The release's stroke: a straight line that continues the path.
    let continues = |acts: &[Vec<Action>]| {
        acts.last()
            .and_then(|a| a.first())
            .is_some_and(|a| matches!(a, Action::Preview(_, v) if v.get("extend").is_some() && v["points"].as_array().is_some_and(|q| q.len() == 2)))
    };
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
    assert_eq!(t.cursor(&cx, end, Mods::default()), Cursor::Pencil, "without Shift or Alt the stroke is freehand");
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
    assert_eq!(Cursor::PencilContinue.precise(), Cursor::Crosshair, "Use Precise Cursors");
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
    let last = t.pointer(&selected_cx, &PointerEvent::new(PointerKind::Up, pts[2].0, pts[2].1).with_mods(alt));
    assert!(matches!(last.first(), Some(Action::Preview(c, v)) if c == "path.freehand" && v["closed"] == true), "{last:?}");
    let cx = cx(&d, &empty, &p);
    let last = drag_held(&mut t, &cx, &pts, alt);
    assert!(matches!(last.last().and_then(|a| a.first()), Some(Action::Preview(c, v)) if c == "path.freehand" && v["closed"] == true), "{last:?}");
}

/// The Paintbrush shows Smooth before an Alt press only with a selected path.
#[test]
fn paintbrush_alt_cursor_matches_selection_at_press() {
    let (d, id) = doc_with_rect();
    let mut selected = Selection::default();
    selected.set([id]);
    let empty = Selection::default();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let t = GestureTool::new("paintbrush");
    assert_eq!(t.cursor(&cx(&d, &selected, &p), Point::new(50.0, 150.0), alt), Cursor::Smooth);
    assert_eq!(t.cursor(&cx(&d, &empty, &p), Point::new(50.0, 150.0), alt), Cursor::PaintbrushClose);
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

/// The Eraser's Angle and Roundness (Eraser Tool Options) go with its stroke, kept within
/// −180–180° and 1–100%; while it drags, a tip that is not round shows the outline of the area
/// it covers, and a round one the drag as wide as the tip.
#[test]
fn the_eraser_sends_its_tip_and_shows_the_area_it_covers() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
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
