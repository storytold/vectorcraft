use super::*;
use crate::edit::{slice_runs, style_at};
use vectorcraft_doc::{Justify, TextKind};
use vectorcraft_geom::{Affine, PathData, Rect, shapes};

fn frame(x: f64) -> TextObject {
    let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "", CharStyle::default());
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(x, 0.0, x + 120.0, 60.0)) };
    t.xf = Affine::IDENTITY;
    t
}

/// Type on a straight path `length` long.
fn path_frame(length: f64) -> TextObject {
    let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "", CharStyle::default());
    let line = kurbo::Line::new((0.0, 0.0), (length, 0.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Shape::to_path(&line, 0.1)), start: 0.0, end: None };
    t.xf = Affine::IDENTITY;
    t
}

/// Type on a curved path about 300 long.
fn curved_path_frame() -> TextObject {
    let mut t = path_frame(300.0);
    let curve = kurbo::CubicBez::new((0.0, 0.0), (100.0, -120.0), (200.0, 120.0), (300.0, 0.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Shape::to_path(&curve, 0.1)), start: 0.0, end: None };
    t
}

/// Type on a closed path, a circle about 314 round.
fn closed_path_frame() -> TextObject {
    let mut t = path_frame(300.0);
    t.kind = TextKind::OnPath { path: shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), start: 0.0, end: None };
    t
}

/// Type on a path that overflows ends its line where its text stops fitting, as area type's
/// last line does, so a story flows on from there through area type and type on a path alike.
#[test]
fn story_flows_across_area_type_and_type_on_a_path() {
    let db = FontDb::global();
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(6);
    let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
    let mut path = path_frame(100.0);
    path.runs = story.clone();
    let laid = layout(db, &path);
    let end = laid.lines.first().map_or(0, |l| l.end);
    assert!(laid.overflow && end > 0 && end < text.len(), "the path's line ends where its text stops: {end}");
    let (a, b, c) = (frame(0.0), path_frame(100.0), frame(400.0));
    let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b, &c], &story, &[]).into_iter().map(|s| s.runs).collect();
    let lens: Vec<usize> = parts.iter().map(|p| runs_len(p)).collect();
    assert!(lens.iter().all(|l| *l > 0), "each frame takes a share: {lens:?}");
    let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
    assert_eq!(joined, text, "no character lost or duplicated");
    let mut on_path = b.clone();
    on_path.runs = parts[1].clone();
    assert!(!layout(db, &on_path).overflow, "the path's share fits it");
}

/// A path that runs out of room at a space holds only what it shows: the space starts the
/// next frame, as a space with no room on a path overflows.
#[test]
fn a_path_that_runs_out_at_a_space_leaves_it_to_the_next_frame() {
    let db = FontDb::global();
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(6);
    let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
    let mut at_space = 0;
    for length in (60..160).step_by(3) {
        let (a, b) = (path_frame(f64::from(length)), frame(400.0));
        let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b], &story, &[]).into_iter().map(|s| s.runs).collect();
        let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, text, "no character lost or duplicated");
        let mut on_path = a.clone();
        on_path.runs = parts[0].clone();
        assert!(!layout(db, &on_path).overflow, "the path's share fits it ({length})");
        at_space += usize::from(parts[1].iter().map(|r| r.text.as_str()).collect::<String>().starts_with(' '));
    }
    assert!(at_space > 0, "some lengths end at a space");
}

/// A right-to-left story flows on from where its text stops fitting a path, in reading order.
#[test]
fn right_to_left_story_flows_on_from_a_path() {
    let db = FontDb::global();
    let text = "שלום עולם ".repeat(8);
    let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
    let (a, b) = (path_frame(100.0), frame(400.0));
    let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b], &story, &[]).into_iter().map(|s| s.runs).collect();
    let lens: Vec<usize> = parts.iter().map(|p| runs_len(p)).collect();
    assert!(lens.iter().all(|l| *l > 0), "each frame takes a share: {lens:?}");
    let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
    assert_eq!(joined, text, "no character lost or duplicated");
    let mut on_path = a.clone();
    on_path.runs = parts[0].clone();
    assert!(!layout(db, &on_path).overflow, "the path's share fits it");
}

/// A path with no length holds no text, so the story flows past it.
#[test]
fn story_flows_past_a_path_with_no_length() {
    let db = FontDb::global();
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(6);
    let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
    let (a, b, c) = (frame(0.0), path_frame(0.0), frame(400.0));
    let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b, &c], &story, &[]).into_iter().map(|s| s.runs).collect();
    let lens: Vec<usize> = parts.iter().map(|p| runs_len(p)).collect();
    assert!(lens[0] > 0 && lens[1] == 0 && lens[2] > 0, "{lens:?}");
    let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
    assert_eq!(joined, text, "no character lost or duplicated");
}

#[test]
fn story_flows_across_frames_and_rejoins() {
    let db = FontDb::global();
    let (a, b, c) = (frame(0.0), frame(200.0), frame(400.0));
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(6) + "\nSecond paragraph here.";
    let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
    let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b, &c], &story, &[]).into_iter().map(|s| s.runs).collect();
    let lens: Vec<usize> = parts.iter().map(|p| runs_len(p)).collect();
    assert!(lens[0] > 0 && lens[1] > 0, "{lens:?}");
    let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
    assert_eq!(joined, text, "no character lost or duplicated");
    // Each non-last slice fits its frame.
    for (f, p) in [&a, &b].iter().zip(&parts) {
        let mut t = (*f).clone();
        t.runs = p.clone();
        assert!(!layout(db, &t).overflow);
    }
    // Lines never start with the break space.
    assert!(!parts[1][0].text.starts_with(' '));
}

#[test]
fn paragraph_styles_flow_with_their_paragraphs() {
    let db = FontDb::global();
    let (a, b) = (frame(0.0), frame(200.0));
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(4) + "\nSecond paragraph.\nThird.";
    let story = vec![TextRun { text, style: CharStyle::default(), inline: None }];
    let style = |j| ParaStyle { justify: j, ..Default::default() };
    use vectorcraft_doc::Justify::*;
    let paras = vec![style(Left), style(Center), style(Right)];
    let parts = distribute(db, &[&a, &b], &story, &paras);
    assert!(!parts[0].runs.is_empty() && !parts[1].runs.is_empty());
    // The first paragraph overflows into the second frame: it is in both, with its style.
    let pa: Vec<Justify> = parts[0].paras.iter().map(|p| p.justify).collect();
    let pb: Vec<Justify> = parts[1].paras.iter().map(|p| p.justify).collect();
    assert_eq!(pa, [Left]);
    assert_eq!(pb, [Left, Center, Right]);
    // Joined again, the shared paragraph counts once.
    let mut fa = a.clone();
    fa.runs = parts[0].runs.clone();
    fa.set_paragraph_styles(parts[0].paras.clone());
    let mut fb = b.clone();
    fb.runs = parts[1].runs.clone();
    fb.set_paragraph_styles(parts[1].paras.clone());
    assert_eq!(story_paras(&[&fa, &fb], &[]), paras);
    // A frame whose shared first paragraph was restyled wins.
    fb.edit_paras(Some(0..1), |p| p.justify = JustifyAll);
    assert_eq!(story_paras(&[&fa, &fb], &[false, true])[0].justify, JustifyAll);
    assert_eq!(story_paras(&[&fa, &fb], &[false, false])[0].justify, Left);
}

#[test]
fn empty_frames_do_not_replace_an_earlier_paragraph_style() {
    let mut first = frame(0.0);
    first.runs = vec![TextRun { text: "First".into(), style: CharStyle::default(), inline: None }];
    first.set_all_paras(ParaStyle { justify: Justify::Right, ..Default::default() });
    let middle = frame(200.0);
    let last = frame(400.0);
    assert_eq!(story_paras(&[&first, &middle, &last], &[true, false, false])[0].justify, Justify::Right);

    // An empty frame between two parts of the same paragraph changes no style either.
    let mut last_with_text = last.clone();
    last_with_text.runs = vec![TextRun { text: "Tail".into(), style: CharStyle::default(), inline: None }];
    assert_eq!(story_paras(&[&first, &middle, &last_with_text], &[true, false, false])[0].justify, Justify::Right);

    let mut restyled_middle = middle.clone();
    restyled_middle.set_all_paras(ParaStyle { justify: Justify::Center, ..Default::default() });
    assert_eq!(story_paras(&[&first, &restyled_middle, &last], &[false, true, false])[0].justify, Justify::Center);
}

/// When the earlier frame ends with a paragraph break, a restyled empty frame keeps its style
/// against an unedited empty frame after it. An unedited empty frame before it, a frame with
/// text after it, or a restyled frame after it still gives the paragraph its own style.
#[test]
fn a_restyled_empty_frame_keeps_its_style_after_a_paragraph_break() {
    let style = |j| ParaStyle { justify: j, ..Default::default() };
    let mut first = frame(0.0);
    first.runs = vec![TextRun { text: "First\n".into(), style: CharStyle::default(), inline: None }];
    first.set_all_paras(style(Justify::Right));
    let mut restyled = frame(200.0);
    restyled.set_all_paras(style(Justify::Center));
    let empty = frame(400.0);
    let tail_style = |frames: &[&TextObject], wins: &[bool]| story_paras(frames, wins).last().map(|p| p.justify);

    assert_eq!(tail_style(&[&first, &restyled, &empty], &[false, true, false]), Some(Justify::Center));
    assert_eq!(tail_style(&[&first, &restyled, &empty, &empty], &[false, true, false, false]), Some(Justify::Center));
    // An unedited empty frame before the restyled one gives way to it.
    assert_eq!(tail_style(&[&first, &empty, &restyled, &empty], &[false, false, true, false]), Some(Justify::Center));
    // A restyled first frame of an empty thread keeps its style as well.
    assert_eq!(tail_style(&[&restyled, &empty, &empty], &[true, false, false]), Some(Justify::Center));

    // Without an edit, the last empty frame still gives the empty paragraph its style.
    assert_eq!(tail_style(&[&first, &restyled, &empty], &[false, false, false]), Some(Justify::Left));
    // A later frame with text still gives the paragraph it holds its style.
    let mut with_text = empty.clone();
    with_text.runs = vec![TextRun { text: "Tail".into(), style: CharStyle::default(), inline: None }];
    assert_eq!(tail_style(&[&first, &restyled, &with_text], &[false, true, false]), Some(Justify::Left));
    // A later restyled frame wins over the earlier one.
    let mut right = empty.clone();
    right.set_all_paras(style(Justify::Right));
    assert_eq!(tail_style(&[&first, &restyled, &right], &[false, true, true]), Some(Justify::Right));
}

/// A frame whose empty last paragraph (after a final paragraph break) was just restyled keeps that
/// style against the unedited first paragraph of the next frame, empty or with text, and across an
/// unedited empty frame. A restyled first paragraph in the next frame still wins.
#[test]
fn restyled_empty_last_paragraph_wins_over_unedited_next_frame() {
    let style = |j| ParaStyle { justify: j, ..Default::default() };
    let mut first = frame(0.0);
    first.runs = vec![TextRun { text: "a\n".into(), style: CharStyle::default(), inline: None }];
    first.set_paragraph_styles(vec![style(Justify::Left), style(Justify::Center)]);
    let empty = frame(200.0);
    let mut with_text = frame(400.0);
    with_text.runs = vec![TextRun { text: "def".into(), style: CharStyle::default(), inline: None }];
    let justify = |frames: &[&TextObject], later: &[bool], earlier: &[bool]| {
        story_paras_restyled(frames, later, earlier).iter().map(|p| p.justify).collect::<Vec<_>>()
    };
    use Justify::*;

    assert_eq!(justify(&[&first, &empty], &[false, false], &[true, false]), [Left, Center]);
    assert_eq!(justify(&[&first, &with_text], &[false, false], &[true, false]), [Left, Center]);
    assert_eq!(justify(&[&first, &empty, &with_text], &[false, false, false], &[true, false, false]), [Left, Center]);
    assert_eq!(justify(&[&first, &empty, &empty], &[false, false, false], &[true, false, false]), [Left, Center]);
    // Without the edit, the next frame gives the paragraph its style, as before.
    assert_eq!(justify(&[&first, &empty], &[false, false], &[false, false]), [Left, Left]);
    assert_eq!(story_paras(&[&first, &with_text], &[]).iter().map(|p| p.justify).collect::<Vec<_>>(), [Left, Left]);
    // A restyled first paragraph in the next frame wins over the restyled tail.
    let mut right = with_text.clone();
    right.set_all_paras(style(Right));
    assert_eq!(justify(&[&first, &right], &[false, true], &[true, true]), [Left, Right]);
    assert_eq!(justify(&[&first, &empty, &right], &[false, false, true], &[true, false, true]), [Left, Right]);
    // A middle frame is the later side at one boundary and the earlier side at the next.
    let mut middle = frame(200.0);
    middle.runs = vec![TextRun { text: "b\n".into(), style: CharStyle::default(), inline: None }];
    middle.set_paragraph_styles(vec![style(Right), style(JustifyAll)]);
    assert_eq!(justify(&[&first, &middle, &with_text], &[false, true, false], &[false, true, false]), [Left, Right, JustifyAll]);
}

/// Numbered list paragraphs count on from one frame to the next: the second frame's first
/// whole paragraph takes the next number. A paragraph split between the frames has its number
/// in the first frame only, and the paragraph after it takes the next number.
#[test]
fn numbered_list_paragraphs_count_on_through_the_frames() {
    use vectorcraft_doc::text::{NumberStyle, ParaList};
    let db = FontDb::global();
    let (a, mut b) = (frame(0.0), frame(200.0));
    b.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(200.0, 0.0, 320.0, 400.0)) };
    let numbered =
        ParaStyle { list: Some(ParaList::numbered(NumberStyle::Arabic)), left_indent: 18.0, first_line_indent: -18.0, ..Default::default() };
    let numbers = |f: &TextObject, s: &Slice| {
        let mut t = f.clone();
        t.runs = s.runs.clone();
        t.set_paragraph_styles(s.paras.clone());
        t.list_count = s.list_count;
        t.first_para_continues = s.continues;
        layout(db, &t).markers.iter().map(|m| m.text.clone()).collect::<Vec<_>>()
    };
    // Whole paragraphs: the first frame ends with a paragraph break.
    let text = (1..=8).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
    let story = vec![TextRun { text, style: CharStyle::default(), inline: None }];
    let parts = distribute(db, &[&a, &b], &story, &vec![numbered.clone(); 8]);
    let first = numbers(&a, &parts[0]);
    assert!((2..8).contains(&first.len()), "both frames hold items: {first:?}");
    assert_eq!(parts.iter().map(|s| s.list_count).collect::<Vec<_>>(), [0, first.len() as u32]);
    let second = numbers(&b, &parts[1]);
    let expected: Vec<String> = (first.len() + 1..=8).map(|n| format!("{n}.")).collect();
    assert_eq!(second, expected, "{first:?}");
    // A paragraph split between the frames.
    let text = format!("One\nTwo\n{}\nFour", "The quick brown fox jumps over the lazy dog. ".repeat(4));
    let story = vec![TextRun { text, style: CharStyle::default(), inline: None }];
    let parts = distribute(db, &[&a, &b], &story, &vec![numbered; 4]);
    assert_eq!(parts.iter().map(|s| s.continues).collect::<Vec<_>>(), [false, true]);
    assert_eq!((numbers(&a, &parts[0]), numbers(&b, &parts[1])), (vec!["1.".to_string(), "2.".into(), "3.".into()], vec!["4.".to_string()]));
}

/// Frame `f` holding slice `p`, laid out.
fn laid(db: &FontDb, f: &TextObject, p: &Slice) -> crate::TextLayout {
    let mut t = f.clone();
    t.runs = p.runs.clone();
    t.set_paragraph_styles(p.paras.clone());
    t.list_count = p.list_count;
    t.first_para_continues = p.continues;
    layout(db, &t)
}

/// Where line `k` of `lay` starts: its first glyph that stands for a character.
fn line_start(lay: &crate::TextLayout, k: usize) -> f64 {
    let l = &lay.lines[k];
    lay.glyphs[l.glyph_start..l.glyph_end].iter().find(|g| g.len > 0).map_or(f64::NAN, |g| g.origin.x)
}

/// A paragraph's first-line indent and bullet belong to its first line: a paragraph that
/// continues from one frame into the next starts the next frame at its left indent, without its
/// bullet, and every line stays inside its frame. A paragraph that starts at the top of a frame
/// keeps both there.
#[test]
fn a_paragraph_continued_into_the_next_frame_has_no_first_line_indent_or_bullet_there() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let db = FontDb::global();
    let (a, b, mut c) = (frame(0.0), frame(200.0), frame(400.0));
    c.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(400.0, 0.0, 520.0, 300.0)) };
    let bulleted = ParaStyle { left_indent: 18.0, first_line_indent: -18.0, list: Some(ParaList::bulleted(Bullet::Dot)), ..Default::default() };
    let indented = ParaStyle { first_line_indent: 30.0, ..Default::default() };
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(9) + "\nSecond paragraph.";
    for para in [bulleted, indented] {
        let (left, first) = (para.left_indent, para.left_indent + para.first_line_indent);
        let list = usize::from(para.list.is_some());
        let story = vec![TextRun { text: long.clone(), style: CharStyle::default(), inline: None }];
        let parts = distribute(db, &[&a, &b, &c], &story, &[para.clone(), para.clone()]);
        assert_eq!(
            parts.iter().map(|p| p.continues).collect::<Vec<_>>(),
            [false, true, true],
            "{:?}",
            parts.iter().map(|p| runs_len(&p.runs)).collect::<Vec<_>>()
        );
        let joined: String = parts.iter().flat_map(|p| &p.runs).map(|r| r.text.as_str()).collect();
        assert_eq!(joined, long, "no character lost or duplicated");
        // [`continued`] reads the same marks from the frames' text.
        assert_eq!(continued(&filled(&[&a, &b, &c], &parts).iter().collect::<Vec<_>>()), [false, true, true]);
        let lays: Vec<_> = [&a, &b, &c].iter().zip(&parts).map(|(f, p)| laid(db, f, p)).collect();
        // The first frame: the first line at the first-line indent, after the bullet.
        assert_eq!(lays[0].markers.len(), list, "{left}");
        assert!((line_start(&lays[0], 0) - (if list > 0 { left } else { first })).abs() < 1e-6, "{}", line_start(&lays[0], 0));
        if list > 0 {
            assert!(lays[0].glyphs[lays[0].markers[0].glyphs.start].origin.x.abs() < 1e-6, "the bullet starts the first frame");
        }
        // The frames it continues into: every line at the left indent, no bullet.
        for (k, (x0, lay)) in [200.0, 400.0].into_iter().zip(&lays[1..]).enumerate() {
            let on_first = lay.markers.iter().filter(|m| lay.glyphs[m.glyphs.start].line == 0).count();
            assert_eq!(on_first, 0, "frame {} starts without the bullet", k + 1);
            assert!((line_start(lay, 0) - x0 - left).abs() < 1e-6, "frame {} starts at {}", k + 1, line_start(lay, 0) - x0);
            assert!(!lay.overflow, "frame {} holds its share", k + 1);
            for l in &lay.lines {
                assert!(l.x1 <= x0 + 120.0 + 1e-6, "frame {}: a line ends at {}", k + 1, l.x1 - x0);
            }
        }
        // The second paragraph, in the last frame, starts with its own first line.
        let last = &lays[2];
        let shown: String = parts[2].runs.iter().map(|r| r.text.as_str()).collect();
        let second = last.lines.iter().position(|l| shown.get(l.start..).is_some_and(|s| s.starts_with("Second"))).unwrap();
        assert!((line_start(last, second) - 400.0 - if list > 0 { left } else { first }).abs() < 1e-6, "{}", line_start(last, second));
        assert_eq!(last.markers.len(), list, "the second paragraph's bullet");

        // One-line paragraphs: the second frame starts with a paragraph of its own.
        let lines = "Short line.\n".repeat(12);
        let story = vec![TextRun { text: lines.clone(), style: CharStyle::default(), inline: None }];
        let parts = distribute(db, &[&a, &b], &story, std::slice::from_ref(&para));
        assert_eq!(parts.iter().map(|p| p.continues).collect::<Vec<_>>(), [false, false]);
        let lay = laid(db, &b, &parts[1]);
        assert!((line_start(&lay, 0) - 200.0 - if list > 0 { left } else { first }).abs() < 1e-6, "{}", line_start(&lay, 0));
        if list > 0 {
            assert_eq!(lay.glyphs[lay.markers[0].glyphs.start].line, 0, "the bullet starts the second frame");
            assert!((lay.glyphs[lay.markers[0].glyphs.start].origin.x - 200.0).abs() < 1e-6);
        }
        assert_eq!(continued(&filled(&[&a, &b], &parts).iter().collect::<Vec<_>>()), [false, false]);
    }
}

/// A frame that a paragraph continues into holds what fits it laid out that way: its share does
/// not overflow it, and the share with the next word added does.
#[test]
fn a_continued_paragraph_fills_the_frame_it_continues_into() {
    let db = FontDb::global();
    let (a, b, mut c) = (frame(0.0), frame(200.0), frame(400.0));
    c.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(400.0, 0.0, 520.0, 300.0)) };
    let hanging = ParaStyle { left_indent: 18.0, first_line_indent: -18.0, ..Default::default() };
    let indented = ParaStyle { first_line_indent: 30.0, ..Default::default() };
    let mut checked = 0;
    for para in [hanging, indented] {
        for k in 0..8 {
            let text = "a ".repeat(k) + &"The quick brown fox jumps over the lazy dog. ".repeat(9);
            let story = vec![TextRun { text, style: CharStyle::default(), inline: None }];
            let parts = distribute(db, &[&a, &b, &c], &story, std::slice::from_ref(&para));
            assert!(parts[1].continues, "{k}");
            assert!(!laid(db, &b, &parts[1]).overflow, "the second frame holds its share ({k})");
            let next: String = parts[2].runs.iter().map(|r| r.text.as_str()).collect();
            let word = next.split_inclusive(' ').next().unwrap_or_default();
            let mut more = parts[1].clone();
            more.runs.push(TextRun { text: word.to_string(), style: CharStyle::default(), inline: None });
            normalize(&mut more.runs);
            assert!(laid(db, &b, &more).overflow, "the second frame has no room for {word:?} ({k})");
            checked += 1;
        }
    }
    assert_eq!(checked, 16);
}

/// Area type in a frame `w` wide down to `y` and `narrow` wide below it, down to `bottom`.
fn stepped_frame(w: f64, narrow: f64, y: f64, bottom: f64) -> TextObject {
    let mut t = frame(0.0);
    let path = kurbo::BezPath::from_vec(vec![
        kurbo::PathEl::MoveTo((0.0, 0.0).into()),
        kurbo::PathEl::LineTo((w, 0.0).into()),
        kurbo::PathEl::LineTo((w, y).into()),
        kurbo::PathEl::LineTo((narrow, y).into()),
        kurbo::PathEl::LineTo((narrow, bottom).into()),
        kurbo::PathEl::LineTo((0.0, bottom).into()),
        kurbo::PathEl::ClosePath,
    ]);
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&path) };
    t
}

/// A paragraph of mixed type sizes that a stepped frame cannot hold goes on in the next frame. The
/// every-line composer breaks the lines the first frame holds as one paragraph, as in a frame tall
/// enough for all of it and not as the single-line composer breaks them, and the first frame
/// takes the text of those lines. The next frame sets the rest at its own width, without the
/// first-line indent, and no character is lost or repeated. The frames take the same shares as
/// when each is measured with the whole rest of the story.
#[test]
fn a_mixed_size_paragraph_continued_from_a_shaped_frame_is_composed_in_both() {
    use vectorcraft_doc::Composer;
    let db = FontDb::global();
    let small = CharStyle { size: 12.0, ..CharStyle::default() };
    let big = CharStyle { size: 36.0, ..CharStyle::default() };
    let probe = layout(db, &TextObject::point(vectorcraft_geom::Point::ZERO, "Destroy all creatures with toughness X or", small.clone()));
    let w = probe.lines[0].x1 - probe.lines[0].x0 + 2.0;
    let story = vec![
        TextRun { text: "Destroy all creatures with toughness ".into(), style: small.clone(), inline: None },
        TextRun { text: "X".into(), style: big, inline: None },
        TextRun { text: " or less. Then draw a card for each creature destroyed this way.\nSecond paragraph.".into(), style: small, inline: None },
    ];
    let text: String = story.iter().map(|r| r.text.as_str()).collect();
    let (a, tall) = (stepped_frame(w, w * 0.5, 40.0, 85.0), stepped_frame(w, w * 0.5, 40.0, 300.0));
    let b = sized(200.0, w * 0.75, 300.0);
    let texts = |lay: &crate::TextLayout, shown: &str| lay.lines.iter().map(|l| shown[l.start..l.end].trim_end().to_string()).collect::<Vec<_>>();
    let shares = |composer: Composer, indent: f64| {
        let para = ParaStyle { composer, first_line_indent: indent, ..Default::default() };
        let paras = [para.clone(), para];
        let parts = distribute(db, &[&a, &b], &story, &paras);
        assert_eq!(parts.iter().map(|p| p.continues).collect::<Vec<_>>(), [false, true], "{composer:?}");
        let joined: String = parts.iter().flat_map(|p| &p.runs).map(|r| r.text.as_str()).collect();
        assert_eq!(joined, text, "{composer:?}: no character lost or repeated");
        same_as_whole_rest(db, &[&a, &b], &story, &paras, &format!("{composer:?}, indent {indent}"));
        let shown = |p: &Slice| p.runs.iter().map(|r| r.text.as_str()).collect::<String>();
        let lays: Vec<_> = [&a, &b].iter().zip(&parts).map(|(f, p)| laid(db, f, p)).collect();
        assert!(lays.iter().all(|l| !l.overflow), "{composer:?}: each frame holds its share");
        let whole =
            laid(db, &tall, &Slice { runs: story.clone(), paras: paras.to_vec(), list_count: 0, list_level_counts: vec![], continues: false });
        let first = texts(&lays[0], &shown(&parts[0]));
        // The first frame takes the text of the lines it sets.
        assert_eq!(shown(&parts[0]).split_whitespace().collect::<Vec<_>>(), first.join(" ").split_whitespace().collect::<Vec<_>>());
        (first, texts(&whole, &text), lays, parts)
    };
    for indent in [0.0, 20.0] {
        let (every, whole, lays, parts) = shares(Composer::EveryLine, indent);
        assert!(every.len() >= 2, "{every:?}");
        assert_eq!(every, whole[..every.len()], "indent {indent}");
        if indent == 0.0 {
            assert_eq!(every, ["Destroy all creatures with toughness", "X or less. Then", "draw a card for each"]);
            let (greedy, ..) = shares(Composer::SingleLine, indent);
            assert_eq!(greedy, ["Destroy all creatures with toughness X", "or less. Then draw a", "card for each", "creature destroyed"]);
        }
        // The next frame starts at its left edge, and its lines keep within it.
        assert!((line_start(&lays[1], 0) - 200.0).abs() < 1e-6, "{}", line_start(&lays[1], 0));
        assert!(lays[1].lines.iter().all(|l| l.x1 <= 200.0 + w * 0.75 + 1e-6), "{:?}", lays[1].lines);
        assert!((line_start(&lays[0], 0) - indent).abs() < 1e-6, "{}", line_start(&lays[0], 0));
        assert!(parts[1].continues && lays[1].lines.len() >= 2, "{:?}", lays[1].lines);
    }
}

/// `frames` holding the runs of `parts`, for [`continued`].
fn filled(frames: &[&TextObject], parts: &[Slice]) -> Vec<TextObject> {
    frames
        .iter()
        .zip(parts)
        .map(|(f, p)| {
            let mut t = (*f).clone();
            t.runs = p.runs.clone();
            t
        })
        .collect()
}

/// A paragraph that continues onto type on a path starts the path without its bullet.
#[test]
fn a_list_paragraph_continued_onto_a_path_has_no_bullet_there() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let db = FontDb::global();
    let para = ParaStyle { left_indent: 18.0, first_line_indent: -18.0, list: Some(ParaList::bulleted(Bullet::Dot)), ..Default::default() };
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(6);
    let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
    let (a, b) = (frame(0.0), path_frame(300.0));
    let parts = distribute(db, &[&a, &b], &story, std::slice::from_ref(&para));
    assert_eq!(parts.iter().map(|p| p.continues).collect::<Vec<_>>(), [false, true]);
    let (la, lb) = (laid(db, &a, &parts[0]), laid(db, &b, &parts[1]));
    assert_eq!((la.markers.len(), lb.markers.len()), (1, 0));
    // The same paragraph starting on the path has its bullet there.
    let mut alone = b.clone();
    alone.runs = story;
    alone.set_paragraph_styles(vec![para]);
    assert_eq!(layout(db, &alone).markers.len(), 1);
}

/// List levels: each level's count runs on through the frames, a deeper level in the next
/// frame going on with the count its paragraphs reached in the frame before.
#[test]
fn list_levels_count_on_through_the_frames() {
    use vectorcraft_doc::text::{NumberStyle, ParaList};
    let db = FontDb::global();
    let (a, mut b) = (frame(0.0), frame(200.0));
    b.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(200.0, 0.0, 320.0, 400.0)) };
    let at = |level| ParaStyle {
        list: Some(ParaList::numbered(NumberStyle::Arabic).at_level(level)),
        left_indent: 18.0,
        first_line_indent: -18.0,
        ..Default::default()
    };
    let numbers = |f: &TextObject, s: &Slice| {
        let mut t = f.clone();
        t.runs = s.runs.clone();
        t.set_paragraph_styles(s.paras.clone());
        t.set_list_counts(s.list_counts());
        layout(db, &t).markers.iter().map(|m| m.text.clone()).collect::<Vec<_>>()
    };
    // One item at level 1 and eleven at level 2: 1. then 1. to 11. through both frames.
    let paras: Vec<ParaStyle> = (0..12).map(|i| at(if i == 0 { 1 } else { 2 })).collect();
    let text = (1..=12).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
    let story = vec![TextRun { text, style: CharStyle::default(), inline: None }];
    let parts = distribute(db, &[&a, &b], &story, &paras);
    let first = numbers(&a, &parts[0]);
    assert!((2..12).contains(&first.len()), "both frames hold items: {first:?}");
    let both: Vec<String> = [first.clone(), numbers(&b, &parts[1])].concat();
    let expected: Vec<String> = std::iter::once("1.".to_string()).chain((1..=11).map(|n| format!("{n}."))).collect();
    assert_eq!(both, expected);
    assert_eq!((parts[1].list_count, parts[1].list_level_counts.clone()), (1, vec![first.len() as u32 - 1]));
}

/// The shares the frames would take if each one were measured with the whole rest of the story:
/// what [`distribute_continuing_levels`] gives, measuring each frame with less of it.
fn whole_rest_shares(
    db: &FontDb,
    frames: &[&TextObject],
    runs: &[TextRun],
    paras: &[ParaStyle],
    continued: bool,
    mut counts: ListCounts,
) -> Vec<Slice> {
    let fit_whole = |frame: &TextObject, runs: &[TextRun]| {
        let lay = layout(db, frame);
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        if !lay.overflow {
            return text.len();
        }
        let mut end = lay.lines.last().map_or(0, |l| l.end);
        if lay.on_path {
            return end;
        }
        let bytes = text.as_bytes();
        while end < bytes.len() && bytes[end] == b' ' {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'\n' {
            end += 1;
        }
        end
    };
    let mut rest = runs.to_vec();
    let mut out = Vec::with_capacity(frames.len());
    let mut next_para = 0;
    let mut continues = continued;
    for (i, f) in frames.iter().enumerate() {
        let first_para = next_para;
        let len = runs_len(&rest);
        let own = [f.para.clone()];
        let (paras, first_para) = if paras.is_empty() { (&own[..], 0) } else { (paras, first_para) };
        let n = if i + 1 == frames.len() {
            len
        } else {
            let mut probe = (*f).clone();
            probe.runs = rest.clone();
            probe.set_paragraph_styles(slice_paras(paras, first_para, &rest));
            probe.set_list_counts(counts);
            probe.first_para_continues = continues;
            fit_whole(&probe, &rest)
        };
        let mut head = slice_runs(&rest, 0, n);
        if head.is_empty() {
            head.push(TextRun { text: String::new(), style: style_at(&rest, n), inline: None });
        }
        let tail = slice_runs(&rest, n, len);
        rest = if tail.is_empty() { vec![TextRun { text: String::new(), style: style_at(&rest, len), inline: None }] } else { tail };
        let head_paras = slice_paras(paras, first_para, &head);
        next_para += head_paras.len().saturating_sub(1);
        let next_counts = count_list_paragraphs(&head, &head_paras, counts);
        let starts_continued = continues;
        continues = continues_after(head.iter().rev().find_map(|r| r.text.bytes().last()), continues);
        let (list_count, list_level_counts) = counts.parts();
        out.push(Slice { runs: head, paras: head_paras, list_count, list_level_counts, continues: starts_continued });
        counts = next_counts;
    }
    out
}

/// `frames` share `runs` as they would if each frame were measured with the whole rest, from the
/// story's start and continuing from text before it; returns how many frames took text.
fn same_as_whole_rest(db: &FontDb, frames: &[&TextObject], runs: &[TextRun], paras: &[ParaStyle], case: &str) -> usize {
    let got = distribute(db, frames, runs, paras);
    assert_eq!(got, whole_rest_shares(db, frames, runs, paras, false, ListCounts::default()), "{case}");
    let counts = ListCounts::from_parts(3, &[2, 1]);
    let continuing = distribute_continuing_levels(db, frames, runs, paras, true, counts);
    assert_eq!(continuing, whole_rest_shares(db, frames, runs, paras, true, counts), "{case}, continuing");
    got.iter().filter(|s| runs_len(&s.runs) > 0).count()
}

/// An area frame `w` by `h` at `x`.
fn sized(x: f64, w: f64, h: f64) -> TextObject {
    let mut t = frame(x);
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(x, 0.0, x + w, h)) };
    t
}

/// Numbered `n` short paragraphs.
fn paragraphs(n: usize) -> String {
    (1..=n).map(|i| format!("Item {i}: the quick brown fox jumps.")).collect::<Vec<_>>().join("\n")
}

/// Each frame takes the same share of a long story when it is measured with only the paragraphs
/// it needs as when it is measured with the whole rest of the story, for every kind of frame,
/// text and paragraph style that changes how much a frame holds.
#[test]
fn frames_take_the_shares_the_whole_rest_gives_them() {
    use vectorcraft_doc::VerticalAlign;
    use vectorcraft_doc::text::{NumberStyle, ParaList};
    let db = FontDb::global();
    let plain = CharStyle::default();
    let big = CharStyle { size: 20.0, ..Default::default() };
    let small = CharStyle { size: 7.0, ..Default::default() };
    let story = |text: &str| vec![TextRun::new(text, plain.clone())];
    let frames: Vec<TextObject> = (0..6).map(|k| frame(200.0 * k as f64)).collect();
    fn refs(v: &[TextObject]) -> Vec<&TextObject> {
        v.iter().collect()
    }
    let mut took = 0;

    // Many short paragraphs, every one of its own style.
    took += same_as_whole_rest(db, &refs(&frames), &story(&paragraphs(120)), &[], "short paragraphs");
    let spaced: Vec<ParaStyle> = (0..120).map(|i| ParaStyle { space_before: (i % 4) as f64 * 3.0, ..Default::default() }).collect();
    took += same_as_whole_rest(db, &refs(&frames), &story(&paragraphs(120)), &spaced, "spaced paragraphs");

    // Frames of different sizes, one with no height and one too narrow for a word.
    let uneven = vec![
        sized(0.0, 120.0, 30.0),
        sized(200.0, 120.0, 0.0),
        sized(400.0, 200.0, 140.0),
        sized(700.0, 4.0, 200.0),
        sized(800.0, 60.0, 90.0),
        frame(1000.0),
    ];
    took += same_as_whole_rest(db, &refs(&uneven), &story(&paragraphs(120)), &[], "uneven frames");

    // A word longer than a line, and runs of spaces and empty paragraphs where frames end.
    let long_word = format!("{}\n{}\n{}", "x".repeat(700), paragraphs(30), "y".repeat(900));
    took += same_as_whole_rest(db, &refs(&frames), &story(&long_word), &[], "long words");
    let breaks = (0..200).map(|i| if i % 5 == 0 { String::new() } else { format!("Line {i} ends here     ") }).collect::<Vec<_>>().join("\n");
    took += same_as_whole_rest(db, &refs(&frames), &story(&breaks), &[], "spaces and empty paragraphs");
    for k in [0usize, 1, 2, 3] {
        let tall: Vec<TextObject> = (0..4).map(|i| sized(200.0 * i as f64, 120.0, 14.0 * (k + i + 1) as f64)).collect();
        took += same_as_whole_rest(db, &refs(&tall), &story(&breaks), &[], &format!("frame heights {k}"));
    }

    // Style runs and inline graphics cut where frames end, and empty runs as given.
    // Some runs start with a glyph choice, which a run cut after its first character loses.
    let mut styled = vec![];
    let text = paragraphs(90);
    let words: Vec<&str> = text.split_inclusive(' ').collect();
    for (i, three) in words.chunks(3).enumerate() {
        let text = three.concat();
        let mut style = [&plain, &big, &small][i % 3].clone();
        if i % 4 == 0 {
            let ch = text.chars().next().unwrap_or('I');
            style.glyph_alternate = Some(vectorcraft_doc::GlyphChoice { ch, gid: 1, font: "Test".into(), version: String::new() });
        }
        styled.push(TextRun::new(text, style));
        if i % 7 == 0 {
            styled.push(TextRun::inline(vectorcraft_doc::InlineArt::new("sym"), plain.clone()));
        }
        if i % 11 == 0 {
            styled.push(TextRun::new("", big.clone()));
        }
    }
    took += same_as_whole_rest(db, &refs(&frames), &styled, &[], "style runs");

    // Numbered lists at two levels, counted on from text before.
    let at = |level| ParaStyle {
        list: Some(ParaList::numbered(NumberStyle::Arabic).at_level(level)),
        left_indent: 18.0,
        first_line_indent: -18.0,
        ..Default::default()
    };
    let listed: Vec<ParaStyle> = (0..120).map(|i| at(1 + i % 3 / 2)).collect();
    took += same_as_whole_rest(db, &refs(&frames), &story(&paragraphs(120)), &listed, "lists");

    // Justified and hyphenated paragraphs, long enough to run from frame to frame.
    let text = "The quick brown fox jumps over the lazy dog, extraordinarily unbelievably. ".repeat(6);
    let long_paras = (0..14).map(|_| text.trim_end()).collect::<Vec<_>>().join("\n");
    let justified: Vec<ParaStyle> = (0..14)
        .map(|i| ParaStyle { justify: [Justify::JustifyAll, Justify::JustifyLeft][i % 2], hyphenate: i % 3 != 0, ..Default::default() })
        .collect();
    took += same_as_whole_rest(db, &refs(&frames), &story(&long_paras), &justified, "justified and hyphenated");
    // Paragraphs longer than a frame holds, each composed as a whole.
    let tall: Vec<TextObject> = (0..5).map(|k| sized(200.0 * k as f64, 160.0, 160.0 + 40.0 * k as f64)).collect();
    let very_long = (0..6).map(|_| text.repeat(4).trim_end().to_string()).collect::<Vec<_>>().join("\n");
    for justify in [Justify::Left, Justify::JustifyLeft] {
        let composed = [ParaStyle { justify, ..Default::default() }];
        took += same_as_whole_rest(db, &refs(&tall), &story(&very_long), &composed, &format!("long paragraphs, {justify:?}"));
    }

    // Columns, vertical type, Shrink Text to Fit, lines aligned to the center, type on a path,
    // and a last frame the story overflows.
    let mut columns = frames.clone();
    for f in &mut columns {
        f.area.columns = 2;
        f.area.gutter = 6.0;
    }
    took += same_as_whole_rest(db, &refs(&columns), &story(&paragraphs(120)), &[], "columns");
    let mut vertical = frames.clone();
    for f in &mut vertical {
        f.vertical = true;
    }
    took += same_as_whole_rest(db, &refs(&vertical), &story(&paragraphs(80)), &[], "vertical type");
    let mut mixed = frames.clone();
    mixed[1].area.fit = vectorcraft_doc::AreaFit::ShrinkText { min_percent: 60.0 };
    mixed[2].area.vertical_align = vectorcraft_doc::VerticalAlign::Center;
    mixed[3] = path_frame(300.0);
    took += same_as_whole_rest(db, &refs(&mixed), &story(&paragraphs(120)), &[], "other kinds of frame");
    let two = [&frames[0], &frames[1]];
    took += same_as_whole_rest(db, &two, &story(&paragraphs(200)), &[], "overflowing last frame");

    // Lines aligned to the center, the bottom or justified vertically, in rectangles, ellipses and
    // frames that wrap around an object, with Shrink Text to Fit in one frame, and a story whose
    // last paragraph runs through several frames.
    let last_long = format!("{}\n{}", paragraphs(6), "The quick brown fox jumps over the lazy dog. ".repeat(40));
    for align in [VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify] {
        let mut rects = frames.clone();
        for f in &mut rects {
            f.area.vertical_align = align;
        }
        took += same_as_whole_rest(db, &refs(&rects), &story(&paragraphs(120)), &spaced, &format!("{align:?}"));
        took += same_as_whole_rest(db, &refs(&rects), &story(&breaks), &[], &format!("{align:?}, empty paragraphs"));
        took += same_as_whole_rest(db, &refs(&rects), &styled, &[], &format!("{align:?}, style runs"));
        took += same_as_whole_rest(db, &refs(&rects), &story(&paragraphs(120)), &listed, &format!("{align:?}, lists"));
        took += same_as_whole_rest(db, &refs(&rects), &story(&last_long), &[], &format!("{align:?}, long last paragraph"));
        let mut ellipses = rects.clone();
        for (k, f) in ellipses.iter_mut().enumerate() {
            let x = 200.0 * k as f64;
            f.kind = TextKind::Area { frame: shapes::ellipse(Rect::new(x, 0.0, x + 160.0, 90.0)) };
        }
        took += same_as_whole_rest(db, &refs(&ellipses), &story(&paragraphs(120)), &[], &format!("{align:?}, ellipses"));
        took += same_as_whole_rest(db, &refs(&ellipses), &story(&long_paras), &justified, &format!("{align:?}, ellipses, justified"));
        let mut wrapped = rects.clone();
        for (k, f) in wrapped.iter_mut().enumerate() {
            let x = 200.0 * k as f64;
            f.wrap = vec![vectorcraft_doc::WrapShape { path: shapes::ellipse(Rect::new(x + 40.0, 10.0, x + 90.0, 40.0)), wrap: Default::default() }];
        }
        took += same_as_whole_rest(db, &refs(&wrapped), &story(&paragraphs(120)), &[], &format!("{align:?}, wrapped"));
        let mut shrink = rects.clone();
        shrink[2].area.fit = vectorcraft_doc::AreaFit::ShrinkText { min_percent: 60.0 };
        took += same_as_whole_rest(db, &refs(&shrink), &story(&paragraphs(120)), &[], &format!("{align:?}, Shrink Text to Fit"));
    }

    // Type on a path: lines that start at the start bracket, and lines whose place or length
    // depends on the text after them (centered or right-aligned, hanging punctuation, Dynamic
    // Text, right-to-left text), horizontal and vertical.
    let paths = |edit: Edit| -> Vec<TextObject> {
        (0..6)
            .map(|k| {
                let mut t = if k % 2 == 0 { path_frame(300.0) } else { curved_path_frame() };
                if k % 3 == 2 {
                    // Brackets inside the path, with room for the line's ends to hang past them.
                    if let TextKind::OnPath { start, end, .. } = &mut t.kind {
                        (*start, *end) = (0.1, Some(0.8));
                    }
                }
                edit(&mut t);
                t
            })
            .collect()
    };
    let hebrew = "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd} ".repeat(4);
    let rtl_early = format!("{}\n{}\n{}", paragraphs(2), hebrew.trim_end(), paragraphs(100));
    let ends_unhung = format!("{}\nlast words", paragraphs(120));
    let rtl_late = format!("{}\n{}\n{}", paragraphs(80), hebrew.trim_end(), paragraphs(20));
    let whole = story(&paragraphs(120));
    let at = paragraphs(60).len();
    let directed = [
        slice_runs(&whole, 0, at),
        vec![TextRun::new(
            " set right to left ",
            CharStyle { char_direction: Some(vectorcraft_doc::ParaDirection::RightToLeft), ..Default::default() },
        )],
        slice_runs(&whole, at, runs_len(&whole)),
    ]
    .concat();
    type Edit = fn(&mut TextObject);
    let edits: [(&str, Edit); 18] = [
        ("path", |_| {}),
        ("path, Left", |t| t.para.justify = Justify::Left),
        ("path, Auto", |t| t.para.justify = Justify::Auto),
        ("path, Justify Left", |t| t.para.justify = Justify::JustifyLeft),
        ("path, Center", |t| t.para.justify = Justify::Center),
        ("path, Right", |t| t.para.justify = Justify::Right),
        ("path, Justify All", |t| t.para.justify = Justify::JustifyAll),
        ("path, Justify All, spacing", |t| {
            t.para.justify = Justify::JustifyAll;
            t.path_spacing = 8.0;
        }),
        ("path, Justify All, negative spacing", |t| {
            t.para.justify = Justify::JustifyAll;
            t.path_spacing = -8.0;
        }),
        ("closed path, Justify All", |t| {
            t.para.justify = Justify::JustifyAll;
            if let TextKind::OnPath { path, .. } = &mut t.kind {
                *path = shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0));
            }
        }),
        ("path, spacing", |t| t.path_spacing = 8.0),
        ("path, Optical Margin Alignment", |t| t.optical_margin = true),
        ("path, Roman Hanging Punctuation", |t| t.para.roman_hanging = true),
        ("path, Dynamic Text", |t| t.dynamic_text = true),
        ("path, right to left", |t| t.para.direction = Some(vectorcraft_doc::ParaDirection::RightToLeft)),
        ("path, Auto right to left", |t| {
            t.para.justify = Justify::Auto;
            t.para.direction = Some(vectorcraft_doc::ParaDirection::RightToLeft);
        }),
        ("vertical path", |t| t.vertical = true),
        ("vertical path, Center", |t| {
            t.vertical = true;
            t.para.justify = Justify::Center;
        }),
    ];
    for (case, edit) in edits {
        let on = paths(edit);
        took += same_as_whole_rest(db, &refs(&on), &story(&paragraphs(120)), &[], case);
        took += same_as_whole_rest(db, &refs(&on), &story(&rtl_early), &[], &format!("{case}, right-to-left paragraph early"));
        took += same_as_whole_rest(db, &refs(&on), &story(&rtl_late), &[], &format!("{case}, right-to-left paragraph late"));
        took += same_as_whole_rest(db, &refs(&on), &directed, &[], &format!("{case}, right-to-left Character Direction"));
        took += same_as_whole_rest(db, &refs(&on), &story(&last_long), &[], &format!("{case}, long last paragraph"));
        // The story's last character does not hang, the last one measured does.
        took += same_as_whole_rest(db, &refs(&on), &story(&ends_unhung), &[], &format!("{case}, story ending in a letter"));
    }
    // Glyphs whose advance is negative, and punctuation and quotes that end paragraphs, some of
    // them alone after empty paragraphs.
    let tight = CharStyle { tracking: -1200.0, ..Default::default() };
    let text = paragraphs(120);
    let mut negative: Vec<TextRun> =
        text.split_inclusive(' ').enumerate().map(|(i, w)| TextRun::new(w, if i % 4 == 0 { tight.clone() } else { plain.clone() })).collect();
    normalize(&mut negative);
    let punctuated = (1..=150)
        .map(|i| match i % 3 {
            0 => ".".to_string(),
            1 => format!("Item {i}: the \u{201c}fox.\u{201d}"),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    for justify in [Justify::Left, Justify::JustifyAll, Justify::Center, Justify::Right] {
        let mut on = paths(|_| {});
        on.push(closed_path_frame());
        for t in &mut on {
            t.para.justify = justify;
        }
        took += same_as_whole_rest(db, &refs(&on), &negative, &[], &format!("path, {justify:?}, negative tracking"));
        took += same_as_whole_rest(db, &refs(&on), &story(&punctuated), &[], &format!("path, {justify:?}, punctuation at paragraph ends"));
        let mut spaced = on.clone();
        for t in &mut spaced {
            t.path_spacing = -8.0;
        }
        took += same_as_whole_rest(db, &refs(&spaced), &negative, &[], &format!("path, {justify:?}, negative tracking and spacing"));
    }
    let on = paths(|_| {});
    took += same_as_whole_rest(db, &refs(&on), &styled, &[], "path, style runs");
    took += same_as_whole_rest(db, &refs(&on), &story(&paragraphs(120)), &listed, "path, lists");
    took += same_as_whole_rest(db, &refs(&on), &story(&breaks), &[], "path, spaces and empty paragraphs");
    let story_paras: Vec<ParaStyle> = (0..120)
        .map(|i| ParaStyle { justify: [Justify::Left, Justify::Center, Justify::Auto][i % 3], roman_hanging: i % 5 == 0, ..Default::default() })
        .collect();
    took += same_as_whole_rest(db, &refs(&on), &story(&paragraphs(120)), &story_paras, "path, paragraph styles");
    let mut between = frames.clone();
    for (k, f) in between.iter_mut().enumerate() {
        if k % 2 == 1 {
            *f = path_frame(260.0);
        } else {
            f.area.vertical_align = VerticalAlign::Bottom;
        }
    }
    took += same_as_whole_rest(db, &refs(&between), &story(&paragraphs(120)), &[], "paths between area type");

    // Short stories, an empty one and one of empty runs.
    took += same_as_whole_rest(db, &refs(&frames), &story("Hi"), &[], "short story");
    took += same_as_whole_rest(db, &refs(&frames), &story(""), &[], "empty story");
    took += same_as_whole_rest(db, &refs(&frames), &[TextRun::new("", big.clone()), TextRun::new("", small.clone())], &[], "empty runs");
    assert!(took > 40, "the frames took text: {took}");
}

/// A seeded mix of run splits, sizes, paragraph styles and frame sizes: each frame takes the share
/// the whole rest gives it.
#[test]
fn seeded_stories_take_the_shares_the_whole_rest_gives_them() {
    let db = FontDb::global();
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let words = ["fox", "a", "jumps", "extraordinary", "dog,", "the", "lazy", "over", "  ", "quick"];
    for case in 0..12 {
        let mut runs = vec![];
        let mut paras = 1;
        for _ in 0..(150 + next(250)) {
            let mut text = words[next(words.len() as u64) as usize].to_string();
            if next(60) == 0 {
                text = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}".to_string();
            }
            text.push(if next(6) == 0 {
                paras += 1;
                '\n'
            } else {
                ' '
            });
            let style = CharStyle { size: [12.0, 9.0, 16.0][next(3) as usize], tracking: [0.0, 50.0][next(2) as usize], ..Default::default() };
            runs.push(TextRun::new(text, style));
        }
        normalize(&mut runs);
        let styles: Vec<ParaStyle> = (0..paras)
            .map(|_| ParaStyle {
                justify: [Justify::Left, Justify::Center, Justify::JustifyLeft][next(3) as usize],
                first_line_indent: [0.0, 12.0][next(2) as usize],
                space_after: [0.0, 4.0][next(2) as usize],
                ..Default::default()
            })
            .collect();
        let mut frames: Vec<TextObject> =
            (0..2 + next(6)).map(|k| sized(200.0 * k as f64, 40.0 + next(120) as f64, 10.0 + next(120) as f64)).collect();
        // Lines aligned every way, and type on a path, horizontal or vertical.
        for f in &mut frames {
            use vectorcraft_doc::VerticalAlign;
            f.area.vertical_align = [VerticalAlign::Top, VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify][next(4) as usize];
            if next(4) == 0 {
                *f = if next(2) == 0 { path_frame(80.0 + next(400) as f64) } else { curved_path_frame() };
                f.vertical = next(3) == 0;
            }
        }
        same_as_whole_rest(db, &frames.iter().collect::<Vec<_>>(), &runs, &styles, &format!("case {case}"));
    }
}

/// Each of `frames` takes its share of a long story of short paragraphs measured with about the text
/// it takes and a little more, not with the whole rest of the story.
fn measures_about_the_text_taken(db: &FontDb, frames: &[TextObject], case: &str) {
    let text = paragraphs(1500);
    let story = vec![TextRun::new(text.clone(), CharStyle::default())];
    let refs: Vec<&TextObject> = frames.iter().collect();
    let before = fit_bytes();
    let parts = distribute(db, &refs, &story, &[]);
    let measured = fit_bytes() - before;
    let joined: String = parts.iter().flat_map(|p| &p.runs).map(|r| r.text.as_str()).collect();
    assert_eq!(joined, text, "{case}: no character lost or duplicated");
    let placed: usize = parts[..parts.len() - 1].iter().map(|p| runs_len(&p.runs)).sum();
    let longest = text.split('\n').map(str::len).max().unwrap_or(0);
    assert!(placed > 0 && placed < text.len() / 4, "{case}: the frames hold part of the story: {placed} of {}", text.len());
    let bound = 4 * placed + frames.len() * 2 * (MIN_MEASURE + longest);
    assert!(measured <= bound, "{case}: measured {measured} bytes for {placed} placed in {} frames (at most {bound})", frames.len());
}

/// A long story of short paragraphs through many frames: each frame is measured with about the
/// text it takes and a little more, not with the whole rest of the story. That holds for area type
/// aligned to the top, the center or the bottom or justified vertically, in rectangles and other
/// shapes, and for type on a path whose line starts at the start bracket, horizontal or vertical.
#[test]
fn many_frames_measure_about_the_text_they_take() {
    use vectorcraft_doc::VerticalAlign;
    let db = FontDb::global();
    let frames: Vec<TextObject> = (0..40).map(|k| frame(200.0 * k as f64)).collect();
    measures_about_the_text_taken(db, &frames, "top");
    for align in [VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify] {
        let mut aligned = frames.clone();
        for f in &mut aligned {
            f.area.vertical_align = align;
        }
        measures_about_the_text_taken(db, &aligned, &format!("{align:?}"));
        let mut shaped = aligned.clone();
        for (k, f) in shaped.iter_mut().enumerate() {
            let x = 200.0 * k as f64;
            f.kind = TextKind::Area { frame: shapes::ellipse(Rect::new(x, 0.0, x + 160.0, 90.0)) };
        }
        measures_about_the_text_taken(db, &shaped, &format!("{align:?}, ellipses"));
    }
    for justify in [Justify::Left, Justify::JustifyLeft, Justify::JustifyAll, Justify::Auto] {
        let paths: Vec<TextObject> = (0..40)
            .map(|_| {
                let mut t = path_frame(300.0);
                t.para.justify = justify;
                t
            })
            .collect();
        measures_about_the_text_taken(db, &paths, &format!("path, {justify:?}"));
    }
    // Justify All on curved, bracketed and closed paths, and with Spacing.
    type Make = fn() -> TextObject;
    let kinds: [(&str, Make); 4] = [
        ("curved", curved_path_frame),
        ("bracketed", || {
            let mut t = path_frame(300.0);
            if let TextKind::OnPath { start, end, .. } = &mut t.kind {
                (*start, *end) = (0.1, Some(0.8));
            }
            t
        }),
        ("closed", closed_path_frame),
        ("spaced", || {
            let mut t = curved_path_frame();
            t.path_spacing = -20.0;
            t
        }),
    ];
    for (shape, make) in kinds {
        let paths: Vec<TextObject> = (0..40)
            .map(|_| {
                let mut t = make();
                t.para.justify = Justify::JustifyAll;
                t
            })
            .collect();
        measures_about_the_text_taken(db, &paths, &format!("{shape} path, Justify All"));
    }
    let vertical: Vec<TextObject> = (0..40)
        .map(|_| {
            let mut t = path_frame(300.0);
            t.vertical = true;
            t
        })
        .collect();
    measures_about_the_text_taken(db, &vertical, "vertical path");
}

/// Frames that already hold text and take a story over several measurements each: each frame's own
/// text is copied once to measure it, not once for each measurement.
#[test]
fn frames_with_text_copy_it_once_to_measure_it() {
    let db = FontDb::global();
    let text = paragraphs(600);
    let story = vec![TextRun::new(text.clone(), CharStyle::default())];
    let mut frames: Vec<TextObject> = (0..4).map(|k| sized(400.0 * k as f64, 300.0, 600.0)).collect();
    for f in &mut frames {
        f.runs = vec![TextRun::new("Old text. ".repeat(500), CharStyle::default())];
    }
    let refs: Vec<&TextObject> = frames.iter().collect();
    let (fit_before, copied_before) = (fit_bytes(), probe_copied());
    let parts = distribute(db, &refs, &story, &[]);
    let (measured, copied) = (fit_bytes() - fit_before, probe_copied() - copied_before);
    let joined: String = parts.iter().flat_map(|p| &p.runs).map(|r| r.text.as_str()).collect();
    assert_eq!(joined, text, "no character lost or duplicated");
    let first = runs_len(&parts[0].runs);
    assert!(measured > 2 * first, "the first frame is measured more than once: {measured} bytes for {first}");
    let own: usize = frames[..frames.len() - 1].iter().map(|f| runs_len(&f.runs)).sum();
    assert_eq!(copied, own, "each frame before the last copies its own text once to measure it");
}

#[test]
fn short_story_leaves_later_frames_empty_with_a_style() {
    let db = FontDb::global();
    let (a, b) = (frame(0.0), frame(200.0));
    let st = CharStyle { size: 20.0, ..Default::default() };
    let parts: Vec<Vec<TextRun>> =
        distribute(db, &[&a, &b], &[TextRun { text: "Hi".into(), style: st, inline: None }], &[]).into_iter().map(|s| s.runs).collect();
    assert_eq!(parts[0][0].text, "Hi");
    assert_eq!((parts[1][0].text.as_str(), parts[1][0].style.size), ("", 20.0));
}
