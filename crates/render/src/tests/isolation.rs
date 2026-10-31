//! Non-isolated transparency groups: blending inside a group with opacity or a blend mode reaches
//! the art below the group unless Isolate Blending is on (checked against the reference formulas).

use crate::*;
use vectorcraft_color::blend::{blend_rgb, composite};
use vectorcraft_color::{BlendMode as Blend, Color, Paint};
use vectorcraft_doc::{Appearance, Knockout, Node};
use vectorcraft_geom::shapes;

/// The backdrop colour (a full-page rectangle) and the blended child's colour.
const B: [f32; 3] = [0.8, 0.6, 0.2];
const S: [f32; 3] = [0.5, 0.5, 1.0];

fn rect(d: &mut Document, r: Rect, c: [f32; 3]) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(Color::rgb(c[0], c[1], c[2])), Paint::None, 0.0))
}

/// An unpainted clipping path, as Make Clipping Mask leaves it (a painted one paints too).
fn clip_path(d: &mut Document, r: Rect) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::None, Paint::None, 0.0))
}

/// A Multiply rectangle at 20..80 (the group's content).
fn multiply(d: &mut Document) -> Node {
    let mut n = rect(d, Rect::new(20.0, 20.0, 80.0, 80.0), S);
    n.blend = Blend::Multiply;
    n
}

/// A Multiply rectangle of color `c` at `r`.
fn multiply_in(d: &mut Document, r: Rect, c: [f32; 3]) -> Node {
    let mut n = rect(d, r, c);
    n.blend = Blend::Multiply;
    n
}

/// The backdrop, then `top` (built by `make`), in a 100 × 100 document.
fn doc(make: impl FnOnce(&mut Document) -> Node) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let below = rect(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), B);
    let top = make(&mut d);
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, below).unwrap();
    d.insert(Some(l), usize::MAX, top).unwrap();
    d
}

/// A group of `children` with `opacity`, `blend` and isolation.
fn group(d: &mut Document, children: Vec<Node>, opacity: f32, blend: Blend, isolate: bool) -> Node {
    let mut g = Node::group(d.alloc_id(), children.into_iter().map(Arc::new).collect());
    (g.opacity, g.blend, g.isolate) = (opacity, blend, isolate);
    g
}

fn render_with(r: &mut Renderer, d: &Document, opts: &RenderOptions) -> Rendered {
    r.render(d, 100, 100, Affine::IDENTITY, opts)
}

fn render(d: &Document) -> Rendered {
    render_with(&mut Renderer::new(), d, &RenderOptions::default())
}

fn rgb(p: [u8; 4]) -> [u8; 3] {
    [p[0], p[1], p[2]]
}

fn to8(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn assert_near(got: [u8; 4], want: [f32; 3], what: &str) {
    let w = to8(want);
    assert!(rgb(got).iter().zip(w).all(|(a, b)| (*a as i32 - b as i32).abs() <= 2) && got[3] == 255, "{what}: {got:?}, want {w:?}");
}

#[test]
fn a_multiply_child_of_a_half_opaque_group_multiplies_with_the_backdrop() {
    let multiplied = blend_rgb(Blend::Multiply, B, S);
    for isolate in [false, true] {
        let d = doc(|d| {
            let m = multiply(d);
            group(d, vec![m], 0.5, Blend::Normal, isolate)
        });
        let r = render(&d);
        // Non-isolated: the child multiplies with the backdrop, then the group is half opaque.
        // Isolated: the child multiplies with nothing, so the group shows its plain colour.
        let inside = if isolate { lerp(B, S, 0.5) } else { lerp(B, multiplied, 0.5) };
        assert_near(r.pixel(50, 50), inside, &format!("isolate {isolate}"));
        assert_near(r.pixel(10, 10), B, "outside the group");
    }
}

#[test]
fn a_group_blend_mode_applies_again_to_the_non_isolated_result() {
    // The child (opaque) multiplies with the backdrop; the group then screens that result onto the
    // backdrop at 60%.
    let d = doc(|d| {
        let m = multiply(d);
        group(d, vec![m], 0.6, Blend::Screen, false)
    });
    let g = blend_rgb(Blend::Multiply, B, S);
    let o = composite(Blend::Screen, [B[0], B[1], B[2], 1.0], [g[0], g[1], g[2], 0.6]);
    assert_near(render(&d).pixel(50, 50), [o[0], o[1], o[2]], "screened multiply");
}

#[test]
fn a_clip_group_does_not_isolate_its_children() {
    let d = doc(|d| {
        let clip = clip_path(d, Rect::new(30.0, 30.0, 70.0, 70.0));
        let m = multiply(d);
        Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(m)], clip: true })
    });
    let r = render(&d);
    assert_near(r.pixel(50, 50), blend_rgb(Blend::Multiply, B, S), "inside the clip");
    assert_near(r.pixel(25, 25), B, "clipped away");
}

#[test]
fn a_non_isolated_group_inside_a_clip_group_stays_clipped() {
    // The half-opaque group copies its backdrop while the clip is in force: the clip still applies
    // to it and to what follows.
    let d = doc(|d| {
        let clip = clip_path(d, Rect::new(30.0, 30.0, 70.0, 70.0));
        let m = multiply(d);
        let g = group(d, vec![m], 0.5, Blend::Normal, false);
        let after = rect(d, Rect::new(0.0, 60.0, 100.0, 100.0), S);
        let children = [clip, g, after].into_iter().map(Arc::new).collect();
        Node::new(d.alloc_id(), NodeKind::Group { children, clip: true })
    });
    let r = render(&d);
    assert_near(r.pixel(50, 40), lerp(B, blend_rgb(Blend::Multiply, B, S), 0.5), "inside the clip");
    assert_near(r.pixel(25, 50), B, "group clipped away");
    assert_near(r.pixel(50, 65), S, "later art inside the clip");
    assert_near(r.pixel(50, 85), B, "later art clipped away");
}

#[test]
fn a_masked_object_blends_with_the_backdrop() {
    let d = doc(|d| {
        let m = multiply(d);
        let mut g = group(d, vec![m], 1.0, Blend::Normal, false);
        // A 50% grey mask over the whole group.
        let art = rect(d, Rect::new(0.0, 0.0, 100.0, 100.0), [0.5; 3]);
        g.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)));
        g
    });
    assert_near(render(&d).pixel(50, 50), lerp(B, blend_rgb(Blend::Multiply, B, S), 0.5), "masked multiply");
}

#[test]
fn a_non_isolated_group_inside_an_isolated_one_blends_with_its_siblings_below() {
    // An isolated group holding a green square, then a 50% group with the Multiply child: the
    // child multiplies with the green (the isolated group's own art), not with the backdrop.
    const G: [f32; 3] = [0.2, 0.9, 0.4];
    let d = doc(|d| {
        let green = rect(d, Rect::new(10.0, 10.0, 90.0, 90.0), G);
        let m = multiply(d);
        let inner = group(d, vec![m], 0.5, Blend::Normal, false);
        group(d, vec![green, inner], 1.0, Blend::Normal, true)
    });
    assert_near(render(&d).pixel(50, 50), lerp(G, blend_rgb(Blend::Multiply, G, S), 0.5), "multiplied with the sibling");
}

#[test]
fn a_non_isolated_knockout_group_composites_its_children_against_the_backdrop() {
    // Two Multiply squares overlapping at 40..60 in a knockout group: in the overlap the top one
    // multiplies with the backdrop only (it knocks out the one below).
    const T: [f32; 3] = [1.0, 0.4, 0.6];
    for isolate in [false, true] {
        let d = doc(|d| {
            let mut lower = rect(d, Rect::new(10.0, 10.0, 60.0, 90.0), S);
            let mut upper = rect(d, Rect::new(40.0, 10.0, 90.0, 90.0), T);
            (lower.blend, upper.blend) = (Blend::Multiply, Blend::Multiply);
            let mut g = group(d, vec![lower, upper], 1.0, Blend::Normal, isolate);
            g.knockout = Knockout::On;
            g
        });
        let r = render(&d);
        let (overlap, alone) = if isolate { (T, S) } else { (blend_rgb(Blend::Multiply, B, T), blend_rgb(Blend::Multiply, B, S)) };
        assert_near(r.pixel(50, 50), overlap, &format!("overlap, isolate {isolate}"));
        assert_near(r.pixel(20, 50), alone, &format!("lower alone, isolate {isolate}"));
    }
}

#[test]
fn a_knockout_group_weights_a_partly_covered_pixel_by_the_coverage_once() {
    // The upper Multiply square's left edge covers half of pixel column 40. There the upper square
    // takes half of the pixel with its color composited against the backdrop, and the lower one
    // keeps the other half (ISO 32000-1 §11.4.6). Right of x = 70 the backdrop is a stripe of
    // another color, which the upper square multiplies with.
    const T: [f32; 3] = [1.0, 0.4, 0.6];
    const STRIPE: [f32; 3] = [0.3, 0.7, 0.9];
    for isolate in [false, true] {
        let mut d = doc(|d| {
            let mut lower = rect(d, Rect::new(10.0, 10.0, 60.0, 90.0), S);
            let mut upper = rect(d, Rect::new(40.5, 10.0, 90.0, 90.0), T);
            (lower.blend, upper.blend) = (Blend::Multiply, Blend::Multiply);
            let mut g = group(d, vec![lower, upper], 1.0, Blend::Normal, isolate);
            g.knockout = Knockout::On;
            g
        });
        let stripe = rect(&mut d, Rect::new(70.0, 0.0, 100.0, 100.0), STRIPE);
        let l = d.layers[0].id;
        d.insert(Some(l), 1, stripe).unwrap();
        let r = render(&d);
        // Isolated, each square multiplies with nothing and shows its own color.
        let multiplied = |b: [f32; 3], c: [f32; 3]| if isolate { c } else { blend_rgb(Blend::Multiply, b, c) };
        assert_near(r.pixel(40, 50), lerp(multiplied(B, S), multiplied(B, T), 0.5), &format!("half-covered edge, isolate {isolate}"));
        assert_near(r.pixel(80, 50), multiplied(STRIPE, T), &format!("over the stripe, isolate {isolate}"));
    }
}

#[test]
fn trim_view_page_group_and_threads_keep_the_non_isolated_look() {
    let d = doc(|d| {
        let m = multiply(d);
        group(d, vec![m], 0.5, Blend::Normal, false)
    });
    let want = lerp(B, blend_rgb(Blend::Multiply, B, S), 0.5);
    let trim = RenderOptions { trim: true, ..Default::default() };
    assert_near(render_with(&mut Renderer::new(), &d, &trim).pixel(50, 50), want, "trim view");
    let mut paged = d.clone();
    paged.page_isolate = true;
    assert_near(render(&paged).pixel(50, 50), want, "isolated page group");
    let mut threaded = Renderer::new();
    threaded.threads = 2;
    let single = render(&d);
    let multi = render_with(&mut threaded, &d, &RenderOptions::default());
    assert_eq!(single.pixels, multi.pixels, "multithreaded rendering copies the backdrop the same way");
}

#[test]
fn copying_the_backdrop_changes_nothing_without_blending() {
    // A plain 50% group renders the same with or without an invisible blending group drawn first
    // (which makes the renderer start again from a picture of the page and mix it back in).
    let plain = doc(|d| {
        let c = rect(d, Rect::new(20.0, 20.0, 80.0, 80.0), S);
        group(d, vec![c], 0.5, Blend::Normal, false)
    });
    let mut flattened = plain.clone();
    let m = multiply(&mut flattened);
    let g = group(&mut flattened, vec![m], 0.5, Blend::Normal, false);
    let l = flattened.layers[0].id;
    // Behind everything but the backdrop, and fully transparent: only the copy happens.
    let mut g = g;
    g.children_mut().unwrap().iter_mut().for_each(|c| Arc::make_mut(c).opacity = 0.0);
    flattened.insert(Some(l), 1, g).unwrap();
    let (a, b) = (render(&plain).pixels, render(&flattened).pixels);
    let worst = a.iter().zip(&b).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap();
    assert!(worst <= 1, "differs by {worst}/255");
}

/// A 300 × 300 page: the backdrop with a 60% stripe over its right half, then nine objects that
/// do not overlap. Each object copies its backdrop or is drawn between objects that do. Object `i`
/// is on the page when `with(i)` is true.
fn objects_apart(with: impl Fn(usize) -> bool) -> Document {
    let mut d = Document::new(300.0, 300.0);
    let below = rect(&mut d, Rect::new(0.0, 0.0, 300.0, 300.0), B);
    let mut stripe = rect(&mut d, Rect::new(150.0, 0.0, 300.0, 300.0), [0.1, 0.3, 0.9]);
    stripe.opacity = 0.6;
    let cell = |x: f64, y: f64| Rect::new(x, y, x + 50.0, y + 50.0);
    let mut objects = vec![];
    // Objects 0 and 1 are a 50% group and a 70% Screen group, each holding a Multiply child.
    let m = multiply_in(&mut d, cell(10.0, 10.0), S);
    objects.push(group(&mut d, vec![m], 0.5, Blend::Normal, false));
    let m = multiply_in(&mut d, cell(110.0, 10.0), S);
    objects.push(group(&mut d, vec![m], 0.7, Blend::Screen, false));
    // Object 2 is a masked group holding a Multiply child. Its mask art holds a 50% group with a
    // Multiply child.
    let m = multiply_in(&mut d, cell(210.0, 10.0), S);
    let mut masked = group(&mut d, vec![m], 1.0, Blend::Normal, false);
    let white = rect(&mut d, cell(205.0, 5.0), [1.0; 3]);
    let gray = multiply_in(&mut d, Rect::new(220.0, 20.0, 250.0, 50.0), [0.5; 3]);
    let half = group(&mut d, vec![gray], 0.5, Blend::Normal, false);
    let art = group(&mut d, vec![white, half], 1.0, Blend::Normal, false);
    masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)));
    objects.push(masked);
    // Object 3 is a knockout group of two Multiply rectangles that overlap.
    let lower = multiply_in(&mut d, Rect::new(10.0, 110.0, 45.0, 160.0), S);
    let upper = multiply_in(&mut d, Rect::new(25.0, 110.0, 60.0, 160.0), [1.0, 0.4, 0.6]);
    let mut knockout = group(&mut d, vec![lower, upper], 1.0, Blend::Normal, false);
    knockout.knockout = Knockout::On;
    objects.push(knockout);
    // Object 4 is an isolated group that holds a 50% group with a Multiply child.
    let green = rect(&mut d, cell(110.0, 110.0), [0.2, 0.9, 0.4]);
    let m = multiply_in(&mut d, Rect::new(120.0, 120.0, 150.0, 150.0), S);
    let half = group(&mut d, vec![m], 0.5, Blend::Normal, false);
    objects.push(group(&mut d, vec![green, half], 1.0, Blend::Normal, true));
    // Object 5 is a clip group that holds a 50% group with a Multiply child and a rectangle after
    // the group.
    let clip = clip_path(&mut d, Rect::new(215.0, 115.0, 255.0, 155.0));
    let m = multiply_in(&mut d, cell(210.0, 110.0), S);
    let half = group(&mut d, vec![m], 0.5, Blend::Normal, false);
    let after = rect(&mut d, Rect::new(210.0, 140.0, 260.0, 160.0), [0.9, 0.9, 0.1]);
    let children = [clip, half, after].into_iter().map(Arc::new).collect();
    objects.push(Node::new(d.alloc_id(), NodeKind::Group { children, clip: true }));
    // Object 6 is a 50% rectangle that is not in a group.
    let mut plain = rect(&mut d, cell(10.0, 210.0), [0.3, 0.2, 0.8]);
    plain.opacity = 0.5;
    objects.push(plain);
    // Object 7 is a Screen group at 100% holding a Multiply child.
    let m = multiply_in(&mut d, cell(110.0, 210.0), S);
    objects.push(group(&mut d, vec![m], 1.0, Blend::Screen, false));
    // Object 8 is a 50% group whose Multiply child reaches past the page's corner.
    let m = multiply_in(&mut d, Rect::new(260.0, 260.0, 330.0, 330.0), S);
    objects.push(group(&mut d, vec![m], 0.5, Blend::Normal, false));
    let l = d.layers[0].id;
    let kept = objects.into_iter().enumerate().filter(|(i, _)| with(*i)).map(|(_, n)| n);
    for n in [below, stripe].into_iter().chain(kept) {
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    d
}

/// The renderer copies the backdrop of several objects in one frame, each from the picture of what
/// it drew before that object. The frame equals the page drawn with each object alone, pixel for
/// pixel, when it is drawn on one thread and on two, turned, composited in floating point, inside
/// an isolated page group, zoomed in until the first group fills the frame, and as the renderer's
/// next frame.
#[test]
fn objects_that_copy_their_backdrop_draw_as_they_do_alone() {
    let (all, none) = (objects_apart(|_| true), objects_apart(|_| false));
    let alone: Vec<Document> = (0..9).map(|i| objects_apart(|j| j == i)).collect();
    let (plain, precise) = (RenderOptions::default(), RenderOptions { precise: true, ..Default::default() });
    let turned = Affine::rotate_about(0.35, vectorcraft_geom::Point::new(150.0, 150.0));
    let zoomed = Affine::translate((150.0, 150.0)) * Affine::scale(6.0) * Affine::translate((-35.0, -35.0));
    for (what, threads, view, opts, isolate) in [
        ("one thread", 0, Affine::IDENTITY, &plain, false),
        ("two threads", 2, Affine::IDENTITY, &plain, false),
        ("turned", 0, turned, &plain, false),
        ("floating point", 0, Affine::IDENTITY, &precise, false),
        ("isolated page", 0, Affine::IDENTITY, &plain, true),
        ("zoomed", 0, zoomed, &plain, false),
    ] {
        let mut r = Renderer::new();
        r.threads = threads;
        let mut draw = |d: &Document| {
            let mut d = d.clone();
            d.page_isolate = isolate;
            r.render(&d, 300, 300, view, opts).pixels
        };
        let below = draw(&none);
        let mut want = below.clone();
        for d in &alone {
            let one = draw(d);
            for ((w, o), b) in want.as_chunks_mut::<4>().0.iter_mut().zip(one.as_chunks::<4>().0).zip(below.as_chunks::<4>().0) {
                if o != b {
                    assert_eq!(*w, *b, "{what}: two objects draw on one pixel");
                    *w = *o;
                }
            }
        }
        for frame in 1..=2 {
            let got = draw(&all);
            let differ: Vec<usize> = (0..300 * 300).filter(|i| got[i * 4..i * 4 + 4] != want[i * 4..i * 4 + 4]).collect();
            let at = differ.first().map(|i| (i % 300, i / 300));
            assert!(differ.is_empty(), "{what}, frame {frame}: {} pixels differ, the first at {at:?}", differ.len());
        }
    }
}

/// The renderer copies the backdrop of twenty 50% groups with Multiply children on a 400 × 400
/// page. The images it draws in that frame add up to fewer pixels than the page: it draws no
/// picture of the page back after a copy.
#[test]
fn backdrop_copies_draw_no_picture_of_the_page_back() {
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    let below = rect(&mut d, Rect::new(0.0, 0.0, 400.0, 400.0), B);
    d.insert(Some(l), usize::MAX, below).unwrap();
    for i in 0..20u32 {
        let (x, y) = (10.0 + f64::from(i % 5) * 80.0, 10.0 + f64::from(i / 5) * 80.0);
        let m = multiply_in(&mut d, Rect::new(x, y, x + 20.0, y + 20.0), S);
        let g = group(&mut d, vec![m], 0.5, Blend::Normal, false);
        d.insert(Some(l), usize::MAX, g).unwrap();
    }
    let drawn = || crate::group::PIXELS_DRAWN.with(std::cell::Cell::get);
    for threads in [0, 2] {
        let mut r = Renderer::new();
        r.threads = threads;
        let before = drawn();
        let frame = r.render(&d, 400, 400, Affine::IDENTITY, &RenderOptions::default());
        let images = drawn() - before;
        assert!(images < 400 * 400, "threads {threads}: {images} pixels drawn as images");
        assert_near(frame.pixel(20, 20), lerp(B, blend_rgb(Blend::Multiply, B, S), 0.5), "inside a group");
    }
}

/// Isolation mode (#833): the art around the isolated group draws at half opacity, above it as
/// well as below; the group's own art as usual.
#[test]
fn isolation_mode_dims_the_art_around_the_isolated_group() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let below = rect(&mut d, Rect::new(0.0, 0.0, 40.0, 40.0), [0.0, 0.0, 0.0]);
    let inside = rect(&mut d, Rect::new(60.0, 60.0, 100.0, 100.0), [0.0, 0.0, 0.0]);
    let above = rect(&mut d, Rect::new(60.0, 0.0, 100.0, 40.0), [0.0, 0.0, 0.0]);
    let g = group(&mut d, vec![inside], 1.0, Blend::Normal, false);
    let gid = g.id;
    for n in [below, g, above] {
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    let alpha = |img: &Rendered, x: usize, y: usize| img.pixels[(y * 100 + x) * 4 + 3];
    let plain = render(&d);
    let isolated = |id| render_with(&mut Renderer::new(), &d, &RenderOptions { isolated: Some(id), ..Default::default() });
    let dimmed = isolated(gid);
    assert_eq!(alpha(&dimmed, 80, 80), 255, "the isolated group's art");
    for (x, y) in [(20, 20), (80, 20)] {
        assert_eq!(alpha(&plain, x, y), 255);
        assert!((120..=135).contains(&alpha(&dimmed, x, y)), "({x}, {y}): {}", alpha(&dimmed, x, y));
    }
    // An object that isn't in the document dims nothing.
    assert_eq!(isolated(NodeId(u64::MAX)).pixels, plain.pixels);
}

/// Deeply nested isolated blending groups (#3581): each level is drawn offscreen only while the
/// offscreen pixels alive stay within a few frames' worth; deeper levels are drawn as layers.
#[test]
fn nested_isolated_blending_groups_stay_within_the_offscreen_budget() {
    let d = doc(|d| {
        let mut n = multiply(d);
        for _ in 0..30 {
            n = group(d, vec![n], 1.0, Blend::Multiply, true);
        }
        n
    });
    let drawn = || crate::group::PIXELS_DRAWN.with(std::cell::Cell::get);
    let before = drawn();
    let frame = render(&d);
    let images = drawn() - before;
    // Each level's offscreen region is the content's bounds (20..80) plus `region`'s 2 px
    // antialiasing margin on each side: 64 × 64. A budget of 4 frames fits whole levels only.
    let level = 64 * 64;
    let budget = 4 * 100 * 100 / level * level;
    assert!(images <= budget, "{images} pixels drawn from offscreen groups, budget {budget}");
    assert_near(frame.pixel(50, 50), blend_rgb(Blend::Multiply, B, S), "inside the groups");
    assert_near(frame.pixel(10, 10), B, "outside the groups");
}
