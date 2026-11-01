use super::*;
use vectorcraft_doc::{CharStyle, Justify, TextKind};
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
