//! Real text: type exported as selectable, searchable text in embedded subset fonts with a
//! ToUnicode map, where the outlines would be; fonts whose licence forbids it stay outlines.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::text::{Bullet, NumberStyle, ParaList};
use vectorcraft_doc::{CharStyle, Document, Effect, Node, NodeKind, ParaStyle, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect, shapes};
use vectorcraft_text::{FALLBACK_FAMILY, FontDb};

use crate::*;

fn style(size: f64) -> CharStyle {
    CharStyle { size, fill: Paint::solid(Color::BLACK), ..Default::default() }
}

/// A document holding `texts`.
fn doc(texts: Vec<TextObject>) -> Document {
    let mut d = Document::new(400.0, 300.0);
    let l = d.layers[0].id;
    for (i, t) in texts.into_iter().enumerate() {
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        d.insert(Some(l), i, n).unwrap();
    }
    d
}

/// The export of `d` with real text (or outlines), content streams left readable.
fn pdf(d: &Document, outline_text: bool) -> ExportReport {
    let settings: PdfSettings =
        serde_json::from_value(json!({"compression": {"compressText": false}, "advanced": {"outlineText": outline_text}})).unwrap();
    export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap()
}

fn import_as(bytes: &[u8], text_as: TextAs) -> Document {
    import_with_report(bytes, &ImportOptions { text_as, ..Default::default() }).unwrap().document
}

/// The text of the type objects of `d`.
fn texts(d: &Document) -> Vec<String> {
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            out.push(t.plain_text());
        }
    });
    out
}

/// The bounds of the paths of `d` (the outlines type imports as), all together.
fn ink(d: &Document) -> Rect {
    let mut b: Option<Rect> = None;
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
            && let Some(r) = n.geometric_bounds()
        {
            b = Some(b.map_or(r, |b| b.union(r)));
        }
    });
    b.unwrap()
}

#[test]
fn type_is_written_as_text_in_embedded_subset_fonts() {
    let d = doc(vec![TextObject::point(Point::new(20.0, 60.0), "Hello World", style(24.0))]);
    let r = pdf(&d, false);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let text = String::from_utf8_lossy(&r.bytes);
    assert!(text.contains("/FontFile2") || text.contains("/FontFile3"), "an embedded font program");
    assert!(text.contains("/ToUnicode") && text.contains("/Type0"));
    // A subset: the font's name carries the subset tag.
    assert!(text.contains(&format!("+{}", FALLBACK_FAMILY.replace(' ', ""))), "a subset");
    assert!(text.contains(" Tf") && text.contains("TJ"));
    // Selectable and searchable: reopened, it is type again, spaces included.
    assert_eq!(texts(&import_as(&r.bytes, TextAs::Text)), ["Hello World"]);
    // Outlined text has no font.
    assert!(!String::from_utf8_lossy(&pdf(&d, true).bytes).contains("/Font"));
}

/// Type that a Transform effect only moves and copies, on the type or on its group, is written as
/// text, the copies too.
#[test]
fn type_copied_by_a_transform_effect_is_written_as_text() {
    let copies = Effect { id: "distort.transform".into(), params: json!({"moveH": 200, "copies": 1}), visible: true };
    for on_group in [false, true] {
        let mut d = Document::new(400.0, 300.0);
        let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 60.0), "Hello", style(24.0)))));
        if on_group {
            n = Node::group(d.alloc_id(), vec![Arc::new(n)]);
        }
        n.appearance.effects.push(copies.clone());
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
        assert_eq!(texts(&back), ["Hello", "Hello"], "on the group: {on_group}");
    }
}

/// Area type 120 points wide holding `text` in `style`, hyphenated (Paragraph panel › Hyphenate)
/// when `hyphenate` is set.
fn hyphenated(text: &str, style: CharStyle, hyphenate: bool) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, style);
    t.kind = TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(Rect::new(20.0, 20.0, 140.0, 280.0)) };
    t.xf = Affine::IDENTITY;
    t.para.hyphenate = hyphenate;
    t
}

/// The characters the canvas draws for the type objects of `d`, in layout order (a hyphen for
/// each glyph that stands for no character), and the origin of each such glyph in document space.
fn drawn_with_hyphens(d: &Document) -> (String, Vec<Point>) {
    let (mut drawn, mut hyphens) = (String::new(), vec![]);
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            let text = t.plain_text();
            for g in &vectorcraft_text::layout(FontDb::global(), t).glyphs {
                if g.len == 0 {
                    drawn.push('-');
                    hyphens.push(t.xf * g.origin);
                } else if let Some(c) = text.get(g.byte..).and_then(|s| s.chars().next()).filter(|c| !c.is_control() && *c != '\u{AD}') {
                    drawn.push(c);
                }
            }
        }
    });
    (drawn, hyphens)
}

/// The origins in document space of the glyphs of the type objects of `d` that stand for '-'.
fn hyphen_origins(d: &Document) -> Vec<Point> {
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            let text = t.plain_text();
            let lay = vectorcraft_text::layout(FontDb::global(), t);
            out.extend(lay.glyphs.iter().filter(|g| g.len > 0 && text.get(g.byte..).is_some_and(|s| s.starts_with('-'))).map(|g| t.xf * g.origin));
        }
    });
    out
}

/// The hyphen at the end of a line that area type breaks inside a word (Hyphenate) is written as
/// text: reopened, the export reads each line's characters and then the hyphen where the canvas
/// draws one, the hyphen sits where the canvas draws it, and the type's ink bounds match those of
/// outlined type. Type without paint, written as invisible text, holds the hyphens too.
#[test]
fn line_break_hyphens_are_written_as_text_where_the_canvas_draws_them() {
    let text = "Extraordinarily comprehensive typographical considerations notwithstanding everything.";
    for paint in [Paint::solid(Color::BLACK), Paint::None] {
        let d = doc(vec![hyphenated(text, CharStyle { fill: paint.clone(), ..style(14.0) }, true)]);
        let (drawn, hyphens) = drawn_with_hyphens(&d);
        assert!(!hyphens.is_empty(), "the layout hyphenates: {drawn}");
        let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
        let all = texts(&back).concat();
        let solid = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(solid(&all), solid(&drawn), "{paint:?}");
        // Each hyphen follows its line's last character, with no space between them.
        assert!(!all.chars().zip(all.chars().skip(1)).any(|(a, b)| a.is_whitespace() && b == '-'), "{paint:?}: {all:?}");
        let got = hyphen_origins(&back);
        for p in &hyphens {
            assert!(got.iter().any(|g| g.distance(*p) < 0.1), "{paint:?}: a hyphen at {p:?}: {got:?}");
        }
    }
    let d = doc(vec![hyphenated(text, style(14.0), true)]);
    let (real, outlined) = (ink(&import_as(&pdf(&d, false).bytes, TextAs::Outlines)), ink(&import_as(&pdf(&d, true).bytes, TextAs::Outlines)));
    let off = [real.x0 - outlined.x0, real.y0 - outlined.y0, real.x1 - outlined.x1, real.y1 - outlined.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(off < 0.5, "{real:?} vs {outlined:?}");
}

/// Right-to-left area type broken at its soft hyphens writes each hyphen as text where the canvas
/// draws it, and its letters come back in their order. Skipped without a font that covers Hebrew.
#[test]
fn line_break_hyphens_of_right_to_left_lines_are_written_as_text_where_the_canvas_draws_them() {
    let db = FontDb::global();
    let Some(face) =
        ["Arial", "Noto Sans Hebrew", "DejaVu Sans", "Liberation Sans"].into_iter().filter_map(|f| db.face(f, "Regular")).find(|f| f.covers('ש'))
    else {
        return;
    };
    let text = "שלוםעולם\u{AD}גדולמאוד\u{AD}ומאוד ירושלים\u{AD}שלזהב\u{AD}ובתלאביב";
    let mut t = hyphenated(text, CharStyle { font_family: face.family.clone(), ..style(14.0) }, true);
    t.para.justify = vectorcraft_doc::Justify::Auto;
    let lines = vectorcraft_text::layout(db, &t).lines.len();
    let d = doc(vec![t]);
    let (_, hyphens) = drawn_with_hyphens(&d);
    assert!(!hyphens.is_empty(), "the lines break at soft hyphens");
    let r = pdf(&d, false);
    // Each line's letters are still marked as written in reverse (/ReversedChars), the hyphenated
    // lines too.
    let reversed = String::from_utf8_lossy(&r.bytes).matches("/ReversedChars").count();
    assert!(reversed >= lines, "{reversed} reversed runs for {lines} lines");
    let back = import_as(&r.bytes, TextAs::Text);
    let got = hyphen_origins(&back);
    for p in &hyphens {
        assert!(got.iter().any(|g| g.distance(*p) < 0.1), "a hyphen at {p:?}: {got:?}");
    }
    let letters = |s: &str| s.chars().filter(|c| c.is_alphabetic()).collect::<String>();
    assert_eq!(letters(&texts(&back).concat()), letters(text));
}

/// The kashidas of justified Arabic type (Insert Kashidas), which stand for no character either,
/// are drawn as paths: reopened, the type holds its letters and no hyphen or tatweel. Skipped
/// without an OpenType font that covers Arabic and the tatweel, when the layout adds no kashida,
/// or when PDF text import returns no strings on this platform.
#[test]
fn kashidas_are_not_written_as_hyphens() {
    let db = FontDb::global();
    let Some(face) = ["Noto Sans Arabic", "Arial", "Geeza Pro"]
        .into_iter()
        .filter_map(|f| db.face(f, "Regular"))
        .find(|f| f.covers('ب') && f.covers('د') && f.covers('\u{0640}'))
    else {
        return;
    };
    let text = "كتب ورد محمد";
    let mut t = TextObject::point(Point::ZERO, text, CharStyle { font_family: face.family.clone(), ..style(20.0) });
    t.kind = TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(Rect::new(20.0, 20.0, 320.0, 280.0)) };
    t.xf = Affine::IDENTITY;
    t.para.justify = vectorcraft_doc::Justify::JustifyAll;
    t.para.kashida = vectorcraft_doc::text::Kashida::Medium;
    let tatweel = face.glyph_for('\u{0640}');
    if !vectorcraft_text::layout(db, &t).glyphs.iter().any(|g| g.len == 0 && g.gid == tatweel) {
        return;
    }
    let back = texts(&import_as(&pdf(&doc(vec![t]), false).bytes, TextAs::Text));
    if back.is_empty() {
        return;
    }
    assert!(!back.concat().contains(['-', '\u{2010}', '\u{0640}']), "{back:?}");
    // Every letter comes back, at least as often as the text holds it (the justified words may
    // come back as type objects of their own).
    let all = back.concat();
    if all.is_empty() {
        return;
    }
    let count = |s: &str, c: char| s.chars().filter(|&x| x == c).count();
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        assert!(count(&all, c) >= count(text, c), "{c}: {back:?}");
    }
}

/// Area type without Hyphenate comes back with its characters only, and no hyphen.
#[test]
fn type_without_line_break_hyphens_comes_back_without_hyphens() {
    let text = "Extraordinarily comprehensive typographical considerations notwithstanding everything.";
    let d = doc(vec![hyphenated(text, style(14.0), false)]);
    let (drawn, hyphens) = drawn_with_hyphens(&d);
    assert!(hyphens.is_empty(), "{drawn}");
    let all = texts(&import_as(&pdf(&d, false).bytes, TextAs::Text)).concat();
    assert_eq!(all.split_whitespace().collect::<String>(), text.split_whitespace().collect::<String>());
}

/// The marker of a hyphenated bulleted list, which stands for no character of the text either, is
/// not written as a hyphen: reopened, the type holds one hyphen for each line-break hyphen only.
#[test]
fn list_markers_are_not_written_as_line_break_hyphens() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let text = "Extraordinarily comprehensive typographical considerations notwithstanding everything.";
    for bullet in [Bullet::Dash, Bullet::Dot] {
        let mut t = hyphenated(text, style(14.0), true);
        t.para.list = Some(ParaList::bulleted(bullet));
        let lay = vectorcraft_text::layout(FontDb::global(), &t);
        assert!(!lay.markers.is_empty(), "{bullet:?}: the paragraph has a marker");
        let breaks = lay.glyphs.iter().enumerate().filter(|&(i, g)| g.len == 0 && lay.marker_of(i).is_none()).count();
        assert!(breaks > 0, "{bullet:?}: the layout hyphenates");
        let all = texts(&import_as(&pdf(&doc(vec![t]), false).bytes, TextAs::Text)).concat();
        assert_eq!(all.matches(['-', '\u{2010}']).count(), breaks, "{bullet:?}: {all:?}");
    }
}

/// A curve type runs along.
fn arc() -> PathData {
    let mut p = BezPath::new();
    p.move_to((20.0, 250.0));
    p.curve_to((120.0, 150.0), (260.0, 150.0), (380.0, 250.0));
    PathData::from_bezpath(&p)
}

#[test]
fn real_text_lands_within_half_a_point_of_the_outlines() {
    let plain = TextObject::point(Point::new(20.0, 60.0), "Hg fly", style(36.0));
    let mut turned = TextObject::point(Point::new(60.0, 160.0), "Wave AV", CharStyle { h_scale: 80.0, tracking: 120.0, ..style(30.0) });
    turned.xf *= Affine::rotate(-0.5) * Affine::skew(0.2, 0.0);
    let mut mixed = TextObject::point(Point::new(200.0, 60.0), "", style(20.0));
    mixed.runs = vec![
        TextRun { text: "x".into(), style: style(20.0), inline: None },
        TextRun { text: "2".into(), style: CharStyle { baseline_shift: 8.0, v_scale: 140.0, ..style(12.0) }, inline: None },
        TextRun { text: " ok".into(), style: CharStyle { rotation: 20.0, ..style(20.0) }, inline: None },
    ];
    let mut on_path = TextObject::point(Point::ZERO, "along the arc", style(18.0));
    on_path.kind = TextKind::OnPath { path: arc(), start: 0.0, end: None };
    on_path.xf = Affine::IDENTITY;
    for t in [plain, turned, mixed, on_path] {
        let what = t.plain_text();
        let d = doc(vec![t]);
        let real = import_as(&pdf(&d, false).bytes, TextAs::Outlines);
        let outlined = import_as(&pdf(&d, true).bytes, TextAs::Outlines);
        let (a, b) = (ink(&real), ink(&outlined));
        let off = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.5, "{what}: {a:?} vs {b:?}");
        assert_eq!(texts(&import_as(&pdf(&d, false).bytes, TextAs::Text)).concat().replace(' ', ""), what.replace(' ', ""), "{what}");
    }
}

/// Area type 200 points wide holding `text`, its paragraphs in lists `lists`, with an 18 pt
/// hanging indent.
fn listed(text: &str, lists: &[Option<ParaList>]) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, style(12.0));
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(20.0, 20.0, 220.0, 280.0)) };
    t.xf = Affine::IDENTITY;
    t.para.left_indent = 18.0;
    t.para.first_line_indent = -18.0;
    let styles = lists.iter().map(|&list| ParaStyle { list, ..t.para.clone() }).collect();
    t.set_paragraph_styles(styles);
    t
}

/// The origins (document space) of the glyphs of each list marker of the type objects of `d`, in
/// the order they are drawn, the marker's tab left out.
fn marker_origins(d: &Document) -> Vec<Vec<Point>> {
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            let lay = vectorcraft_text::layout(FontDb::global(), t);
            for m in &lay.markers {
                out.push(m.glyphs.clone().filter(|&i| i != m.tab).map(|i| t.xf * lay.glyphs[i].origin).collect());
            }
        }
    });
    out
}

/// Each glyph of the type objects of `d` that stands for characters of its text: the first of
/// them and the glyph's origin in document space.
fn glyph_chars(d: &Document) -> Vec<(char, Point)> {
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            let text = t.plain_text();
            for g in vectorcraft_text::layout(FontDb::global(), t).glyphs.iter().filter(|g| g.len > 0) {
                if let Some(c) = text.get(g.byte..).and_then(|s| s.chars().next()) {
                    out.push((c, t.xf * g.origin));
                }
            }
        }
    });
    out
}

/// Does `text` hold `a` before `b`?
fn before(text: &str, a: &str, b: &str) -> bool {
    matches!((text.find(a), text.find(b)), (Some(x), Some(y)) if x < y)
}

/// `back`, the export of `d` reopened, has glyphs of the characters `drawn` (each list marker's,
/// in the order the canvas draws them) within `near` points of where `d` draws its markers'
/// glyphs.
fn assert_markers_in_place(d: &Document, back: &Document, drawn: &[&str], near: f64) {
    let want = marker_origins(d);
    assert_eq!(want.len(), drawn.len(), "{drawn:?}");
    let got = glyph_chars(back);
    for (origins, chars) in want.iter().zip(drawn) {
        assert_eq!(origins.len(), chars.chars().count(), "{chars}");
        for (p, c) in origins.iter().zip(chars.chars()) {
            assert!(got.iter().any(|(gc, gp)| *gc == c && gp.distance(*p) < near), "{c:?} at {p:?}: {got:?}");
        }
    }
}

/// List markers (Paragraph panel › Bullets and Numbered List) are written as text: reopened, the
/// export holds each bullet and number as characters, before its paragraph's text and where the
/// canvas draws them, and the page looks as it does with outlined type.
#[test]
fn list_markers_are_written_as_text_where_the_canvas_draws_them() {
    let number = Some(ParaList::numbered(NumberStyle::Arabic));
    for (bullet, character) in [(Bullet::Dot, "\u{2022}"), (Bullet::Dash, "\u{2013}"), (Bullet::Circle, "\u{25E6}"), (Bullet::Square, "\u{25AA}")] {
        let bullet = Some(ParaList::bulleted(bullet));
        let t = listed("alpha beta gamma delta epsilon zeta eta theta\nbullet\nfirst\nsecond", &[bullet, bullet, number, number]);
        let d = doc(vec![t]);
        let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
        let all = texts(&back).concat();
        assert_eq!(all.matches(character).count(), 2, "{all}");
        assert!(before(&all, character, "alpha") && before(&all, "1.", "first") && before(&all, "2.", "second"), "{all}");
        assert_markers_in_place(&d, &back, &[character, character, "1.", "2."], 0.1);
        let (real, outlined) = (ink(&import_as(&pdf(&d, false).bytes, TextAs::Outlines)), ink(&import_as(&pdf(&d, true).bytes, TextAs::Outlines)));
        let off =
            [real.x0 - outlined.x0, real.y0 - outlined.y0, real.x1 - outlined.x1, real.y1 - outlined.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.5, "{character}: {real:?} vs {outlined:?}");
    }
}

/// A right-to-left numbered paragraph's number is written as its characters where the canvas
/// draws them (the period, then 1 and 2, from the left), and the paragraph's own text comes back
/// as it does without a list.
#[test]
fn a_right_to_left_number_is_written_as_text_where_the_canvas_draws_it() {
    let hebrew = "שלום עולם";
    let mut plain = listed(hebrew, &[None]);
    plain.edit_paras(None, |p| p.justify = vectorcraft_doc::Justify::Auto);
    let mut numbered = plain.clone();
    numbered.edit_paras(None, |p| p.list = Some(ParaList::numbered(NumberStyle::Arabic)));
    numbered.list_count = 11;
    let d = doc(vec![numbered]);
    let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
    // The reader tracks a reopened right-to-left line as a whole, and the gap before the number
    // comes back as a space of the Hebrew font's width, so the number may sit up to a tenth of an
    // em (1.2 pt) away; its characters keep their spacing to within 0.1 pt.
    assert_markers_in_place(&d, &back, &[".12"], 1.2);
    let drawn = marker_origins(&d).concat();
    let got = glyph_chars(&back);
    let at = |c: char| got.iter().find(|(gc, _)| *gc == c).map(|(_, p)| p.x);
    for (k, (a, b)) in [('.', '1'), ('1', '2')].into_iter().enumerate() {
        let (Some(pa), Some(pb), Some(da), Some(db)) = (at(a), at(b), drawn.get(k), drawn.get(k + 1)) else {
            panic!("{a:?} and {b:?} in {got:?}");
        };
        assert!(((pb - pa) - (db.x - da.x)).abs() < 0.1, "{a:?} to {b:?}: {} against {}", pb - pa, db.x - da.x);
    }
    let without = texts(&import_as(&pdf(&doc(vec![plain]), false).bytes, TextAs::Text)).concat();
    let all = texts(&back).concat();
    assert!(all.contains(without.trim()), "{all:?} holds {without:?}");
}

/// Type on a path writes its bullet as text where the canvas draws it along the path.
#[test]
fn a_bullet_on_a_path_is_written_as_text_where_the_canvas_draws_it() {
    let mut t = TextObject::point(Point::ZERO, "along the arc", style(18.0));
    t.kind = TextKind::OnPath { path: arc(), start: 0.0, end: None };
    t.xf = Affine::IDENTITY;
    t.para.list = Some(ParaList::bulleted(Bullet::Dot));
    let d = doc(vec![t]);
    let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
    assert!(texts(&back).concat().contains('\u{2022}'), "{:?}", texts(&back));
    assert_markers_in_place(&d, &back, &["\u{2022}"], 0.3);
}

/// Threaded frames write each marker their layouts draw as text, the number of the paragraph the
/// first frame splits included, in each frame where the canvas draws it.
#[test]
fn numbers_in_threaded_frames_are_written_as_text_in_each_frame() {
    let numbered =
        ParaStyle { list: Some(ParaList::numbered(NumberStyle::Arabic)), left_indent: 18.0, first_line_indent: -18.0, ..Default::default() };
    let frame = |r: Rect| {
        let mut t = TextObject::point(Point::ZERO, "", style(12.0));
        t.kind = TextKind::Area { frame: shapes::rectangle(r) };
        t.xf = Affine::IDENTITY;
        t
    };
    let (mut a, mut b) = (frame(Rect::new(20.0, 20.0, 180.0, 80.0)), frame(Rect::new(200.0, 20.0, 360.0, 280.0)));
    let text = format!("One\nTwo\n{}\nFour", "The quick brown fox jumps over the lazy dog. ".repeat(4));
    let story = vec![TextRun { text, style: style(12.0), inline: None }];
    let parts = vectorcraft_text::thread::distribute(FontDb::global(), &[&a, &b], &story, &vec![numbered; 4]);
    for (f, part) in [&mut a, &mut b].into_iter().zip(&parts) {
        f.runs = part.runs.clone();
        f.set_paragraph_styles(part.paras.clone());
        f.list_count = part.list_count;
    }
    let first = a.plain_text();
    assert!(first.contains("quick") && !first.ends_with('\n'), "the first frame splits the third paragraph: {first:?}");
    let d = doc(vec![a, b]);
    let mut drawn: Vec<String> = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            drawn.extend(vectorcraft_text::layout(FontDb::global(), t).markers.into_iter().map(|m| m.text));
        }
    });
    assert!(drawn.contains(&"4.".to_string()), "{drawn:?}");
    let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
    assert_markers_in_place(&d, &back, &drawn.iter().map(String::as_str).collect::<Vec<_>>(), 0.1);
}

/// A number set in oldstyle figures (the first character's OpenType features) is written as its
/// digits, though the character map gives the digits other glyphs. The reopened type has no
/// OpenType features and sets lining figures, so only where each number starts is compared.
#[test]
fn a_number_in_oldstyle_figures_is_written_as_its_digits() {
    let number = Some(ParaList::numbered(NumberStyle::Arabic));
    let mut t = listed("first\nsecond", &[number, number]);
    for r in &mut t.runs {
        r.style.features = vec!["onum".into()];
    }
    let lay = vectorcraft_text::layout(FontDb::global(), &t);
    let one = &lay.glyphs[lay.markers[0].glyphs.start];
    let face = FontDb::global().face_by_id(one.font_id).unwrap();
    assert_ne!(face.glyph_for('1'), one.gid, "an oldstyle 1, not the character map's");
    let d = doc(vec![t]);
    let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
    let all = texts(&back).concat();
    assert!(before(&all, "1.", "first") && before(&all, "2.", "second"), "{all:?}");
    let got = glyph_chars(&back);
    for (origins, digit) in marker_origins(&d).iter().zip(['1', '2']) {
        assert!(got.iter().any(|(c, p)| *c == digit && p.distance(origins[0]) < 0.1), "{digit} at {:?}: {got:?}", origins[0]);
    }
}

/// A run shifted two billion points off its baseline is left out, as real text or outlines, and
/// the rest of the type is written. On a page 50 points high, the page's y flip puts the shifted
/// run's baseline at -2,147,483,648, which the PDF writer panics on in builds with overflow checks.
#[test]
fn a_run_shifted_two_billion_points_is_left_out() {
    let mut t = TextObject::point(Point::new(5.0, 40.0), "", style(12.0));
    t.runs = vec![
        TextRun { text: "in".into(), style: style(12.0), inline: None },
        TextRun { text: "out".into(), style: CharStyle { baseline_shift: -2_147_483_649.0, ..style(12.0) }, inline: None },
    ];
    let mut d = doc(vec![t]);
    d.artboards[0].rect = Rect::new(0.0, 0.0, 50.0, 50.0);
    assert_eq!(texts(&import_as(&pdf(&d, false).bytes, TextAs::Text)), ["in"]);
    let b = ink(&import_as(&pdf(&d, true).bytes, TextAs::Outlines));
    assert!(b.y0 > 0.0 && b.y1 < 50.0, "{b:?}");
}

/// An overprinting run is written, or left out, as the same run is without overprint: the form
/// marked to overprint the run holds the type's transform. For type moved 2,147,483,689 points down
/// with its run shifted back onto a page 50 points high, a form placed through that transform would
/// sit at y = -2,147,483,648, which the PDF writer panics on in builds with overflow checks. For
/// type scaled 3 times, a form without that transform would hold a run shifted 500,000,000 points
/// up, and the export would write the run 1,500,000,000 points up.
#[test]
fn overprinting_runs_are_written_or_left_out_as_other_runs_are() {
    let moved = |overprint_fill| {
        let back = CharStyle { baseline_shift: 2_147_483_649.0, overprint_fill, ..style(12.0) };
        TextObject::point(Point::new(5.0, 2_147_483_689.0), "moved", back)
    };
    let scaled = |overprint_fill| {
        let mut t = TextObject::point(Point::new(5.0, 40.0), "", style(12.0));
        t.xf *= Affine::scale(3.0);
        let up = CharStyle { baseline_shift: 500_000_000.0, overprint_fill, ..style(12.0) };
        t.runs = vec![TextRun { text: "in".into(), style: style(12.0), inline: None }, TextRun { text: "out".into(), style: up, inline: None }];
        t
    };
    let written = |mut t: TextObject| {
        // The export culls type by the laid-out bounds that the engine keeps in `cached_bounds`.
        t.cached_bounds = Some(vectorcraft_text::layout(FontDb::global(), &t).bounds);
        let mut d = doc(vec![t]);
        d.artboards[0].rect = Rect::new(0.0, 0.0, 50.0, 50.0);
        pdf(&d, false).bytes
    };
    let text_of = |bytes: &[u8]| texts(&import_as(bytes, TextAs::Text));
    let (on, off) = (written(moved(true)), written(moved(false)));
    assert!(String::from_utf8_lossy(&on).contains("/VCop gs"), "an overprinting form");
    assert_eq!(text_of(&on), ["moved"]);
    assert_eq!(text_of(&off), ["moved"]);
    assert_eq!(text_of(&written(scaled(true))), ["in"]);
    assert_eq!(text_of(&written(scaled(false))), ["in"]);
}

/// A named instance of a variable font is real text in that instance: its embedded subset draws
/// the instance's glyphs where the outlines would be, not the default instance's (#296).
#[test]
fn variable_font_instances_are_real_text_in_their_own_glyphs() {
    use vectorcraft_text::test_fonts::{VARIABLE_CHARS, VARIABLE_FAMILY, variable_font};
    FontDb::global().add_font(variable_font().unwrap());
    let mut widths = vec![];
    for s in ["Regular", "Bold"] {
        let st = CharStyle { font_family: VARIABLE_FAMILY.into(), font_style: s.into(), ..style(48.0) };
        let d = doc(vec![TextObject::point(Point::new(20.0, 80.0), VARIABLE_CHARS, st)]);
        let r = pdf(&d, false);
        assert!(r.warnings.is_empty(), "{s}: {:?}", r.warnings);
        assert_eq!(texts(&import_as(&r.bytes, TextAs::Text)), [VARIABLE_CHARS], "{s}: real text");
        let (real, outlined) = (ink(&import_as(&r.bytes, TextAs::Outlines)), ink(&import_as(&pdf(&d, true).bytes, TextAs::Outlines)));
        let off =
            [real.x0 - outlined.x0, real.y0 - outlined.y0, real.x1 - outlined.x1, real.y1 - outlined.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.5, "{s}: {real:?} vs {outlined:?}");
        widths.push(real.width());
    }
    assert!(widths[1] > widths[0] * 1.2, "Bold is wider: {widths:?}");
}

/// Type with Variable Font settings is real text in a font embedded at those settings: it draws
/// where its outlines would, between the named instances on either side.
#[test]
fn variable_font_settings_are_real_text_in_their_own_glyphs() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, VARIABLE_CHARS, two_axis_font};
    FontDb::global().add_font(two_axis_font().unwrap());
    let mut widths = vec![];
    for (s, wght) in [("Regular", None), ("Regular", Some(650.0)), ("Bold", None)] {
        let font_axes = wght.map(|w| [("wght".to_string(), w)].into_iter().collect()).unwrap_or_default();
        let st = CharStyle { font_family: TWO_AXIS_FAMILY.into(), font_style: s.into(), font_axes, ..style(48.0) };
        let d = doc(vec![TextObject::point(Point::new(20.0, 80.0), VARIABLE_CHARS, st)]);
        let r = pdf(&d, false);
        assert!(r.warnings.is_empty(), "{s} {wght:?}: {:?}", r.warnings);
        assert_eq!(texts(&import_as(&r.bytes, TextAs::Text)), [VARIABLE_CHARS], "{s} {wght:?}: real text");
        let (real, outlined) = (ink(&import_as(&r.bytes, TextAs::Outlines)), ink(&import_as(&pdf(&d, true).bytes, TextAs::Outlines)));
        let off =
            [real.x0 - outlined.x0, real.y0 - outlined.y0, real.x1 - outlined.x1, real.y1 - outlined.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.5, "{s} {wght:?}: {real:?} vs {outlined:?}");
        widths.push(real.width());
    }
    assert!(widths[0] * 1.1 < widths[1] && widths[1] < widths[2], "{widths:?}");
}

#[test]
fn gradients_on_real_text_stay_where_they_were() {
    let fill = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let mut t = TextObject::point(Point::new(30.0, 80.0), "Gradient", CharStyle { fill, ..style(40.0) });
    t.xf *= Affine::rotate(0.3);
    let d = doc(vec![t]);
    let geom = |outline: bool| {
        let back = import_as(&pdf(&d, outline).bytes, TextAs::Outlines);
        let mut g = None;
        back.walk(|n| {
            if let Paint::Gradient(p) = n.appearance.fill_paint() {
                g = g.or(p.geom);
            }
        });
        g.unwrap()
    };
    let (real, outlined) = (geom(false), geom(true));
    assert!((real.start - outlined.start).hypot() < 0.1 && (real.end - outlined.end).hypot() < 0.1, "{real:?} vs {outlined:?}");
}

/// The bundled fallback face as another family `family` (13 characters, like the bundled name),
/// whose licence flags (OS/2 `fsType`) are `fs`.
fn licensed(family: &str, fs: u16) -> String {
    let face = FontDb::global().face(FALLBACK_FAMILY, "Regular").unwrap();
    let mut bytes = face.file_data().to_vec();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16(FALLBACK_FAMILY), utf16(family)), (FALLBACK_FAMILY.as_bytes().to_vec(), family.as_bytes().to_vec())] {
        assert_eq!(from.len(), to.len());
        let mut at = 0;
        while let Some(i) = bytes[at..].windows(from.len()).position(|w| w == from) {
            bytes[at + i..at + i + from.len()].copy_from_slice(&to);
            at += i + from.len();
        }
    }
    let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let os2 = (0..tables).map(|i| 12 + 16 * i).find(|r| &bytes[*r..*r + 4] == b"OS/2").unwrap();
    let off = u32::from_be_bytes(bytes[os2 + 8..os2 + 12].try_into().unwrap()) as usize;
    bytes[off + 8..off + 10].copy_from_slice(&fs.to_be_bytes());
    FontDb::global().add_font(bytes);
    assert!(FontDb::global().has_family(family));
    family.into()
}

#[test]
fn fonts_whose_licence_forbids_embedding_fall_back_to_outlines() {
    let locked = licensed("Locked Sans 3", 0x0002);
    let whole = licensed("Entire Sans 3", 0x0100);
    let d = doc(vec![
        TextObject::point(Point::new(20.0, 40.0), "Open", style(24.0)),
        TextObject::point(Point::new(20.0, 100.0), "Shut", CharStyle { font_family: locked.clone(), ..style(24.0) }),
        TextObject::point(Point::new(20.0, 160.0), "Full", CharStyle { font_family: whole.clone(), ..style(24.0) }),
    ]);
    let r = pdf(&d, false);
    assert!(r.warnings.iter().any(|w| w.contains(&locked) && w.contains("doesn't allow embedding")), "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains(&whole) && w.contains("whole")), "{:?}", r.warnings);
    // Only the open font is text; the others are drawn as their outlines.
    let back = import_as(&r.bytes, TextAs::Text);
    assert_eq!(texts(&back), ["Open"]);
    let mut paths = 0;
    back.walk(|n| paths += usize::from(matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    assert!(paths >= 2, "{paths} outlined words");
}

#[test]
fn type_with_outline_object_is_written_as_paths_with_no_font() {
    let mut d = doc(vec![TextObject::point(Point::new(20.0, 60.0), "Hello World", style(24.0))]);
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().appearance.effects.push(vectorcraft_doc::Effect { id: "path.outlineObject".into(), params: json!({}), visible: true });
    let r = pdf(&d, false);
    let text = String::from_utf8_lossy(&r.bytes);
    assert!(!text.contains("/FontFile") && !text.contains("/Font"), "no font");
    // Reopened, it is paths, even when type is asked for.
    let back = import_as(&r.bytes, TextAs::Text);
    assert!(texts(&back).is_empty());
    let b = ink(&back);
    assert!(b.width() > 50.0 && b.height() > 10.0, "{b:?}");
}

#[test]
fn subset_thresholds_below_100_percent_are_accepted_with_a_warning() {
    let d = doc(vec![TextObject::point(Point::new(20.0, 60.0), "Subset", style(24.0))]);
    let settings: PdfSettings = serde_json::from_value(json!({"advanced": {"outlineText": false, "fontSubsetPercent": 40}})).unwrap();
    let r = export_with_report(&d, &PdfOptions { settings, ..Default::default() }).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("subset")), "{:?}", r.warnings);
    assert_eq!(texts(&import_as(&r.bytes, TextAs::Text)), ["Subset"]);
}

/// Characters with neither fill nor stroke (a scanned page's recognized text, as a PDF opens) are
/// written as invisible text, in rendering mode 3: searchable, drawing nothing, and type without
/// paint again when the file opens. Compressed or not, in PDF/X-1a and PDF/A too (#2675).
#[test]
fn type_without_paint_is_written_as_invisible_text() {
    let unpainted = CharStyle { fill: Paint::None, ..style(24.0) };
    let d = doc(vec![
        TextObject::point(Point::new(20.0, 60.0), "Recognized", unpainted),
        TextObject::point(Point::new(20.0, 160.0), "Visible", style(24.0)),
    ]);
    let r = pdf(&d, false);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let ops = String::from_utf8_lossy(&r.bytes).split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(ops.contains("3 Tr") && ops.contains("0 Tr"), "the invisible text in mode 3, the visible in mode 0");
    let written = |v: serde_json::Value| {
        let settings: PdfSettings = serde_json::from_value(v).unwrap();
        export(&d, &PdfOptions { settings, created: Some(0), ..Default::default() }).unwrap()
    };
    let files = [
        ("uncompressed", r.bytes.clone()),
        ("compressed", written(json!({"advanced": {"outlineText": false}}))),
        ("PDF/X-1a", written(json!({"standard": "pdfX1a", "compatibility": "1.3", "advanced": {"outlineText": false}}))),
        ("PDF/A-2b", written(json!({"standard": "pdfA2b", "advanced": {"outlineText": false}}))),
    ];
    for (what, bytes) in files {
        let back = import_as(&bytes, TextAs::Text);
        assert_eq!(texts(&back), ["Recognized", "Visible"], "{what}");
        let mut paints = vec![];
        back.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                paints.push(t.runs.iter().all(|r| r.style.fill.is_none() && !r.style.has_stroke()));
            }
        });
        assert_eq!(paints, [true, false], "{what}: only the recognized text has no paint");
        // It draws nothing: as outlines, only the visible text is there (its baseline on 160).
        let drawn = ink(&import_as(&bytes, TextAs::Outlines));
        assert!(drawn.y0 > 100.0, "{what}: {drawn:?}");
    }
    // Text exported as outlines has no invisible text: there is nothing to draw.
    let outlined = import_as(&pdf(&d, true).bytes, TextAs::Text);
    assert!(texts(&outlined).is_empty());
    assert!(ink(&outlined).y0 > 100.0);
}

#[test]
fn pdf_a_takes_real_text() {
    let d = doc(vec![TextObject::point(Point::new(20.0, 60.0), "Archive", style(24.0))]);
    let settings: PdfSettings = serde_json::from_value(json!({"standard": "pdfA2b", "advanced": {"outlineText": false}})).unwrap();
    let bytes = export(&d, &PdfOptions { settings, created: Some(0), ..Default::default() }).unwrap();
    assert_eq!(texts(&import_as(&bytes, TextAs::Text)), ["Archive"]);
}

/// A named instance comes back as live type in that style (not the default instance's).
#[test]
fn a_variable_font_instance_comes_back_in_its_own_style() {
    use vectorcraft_text::test_fonts::{VARIABLE_CHARS, VARIABLE_FAMILY, variable_font};
    FontDb::global().add_font(variable_font().unwrap());
    let st = CharStyle { font_family: VARIABLE_FAMILY.into(), font_style: "Bold".into(), ..style(48.0) };
    let d = doc(vec![TextObject::point(Point::new(20.0, 80.0), VARIABLE_CHARS, st)]);
    let back = crate::import_with_report(&pdf(&d, false).bytes, &ImportOptions { text_as: TextAs::Text, ..Default::default() }).unwrap().document;
    let mut styles = vec![];
    back.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            styles.extend(t.runs.iter().map(|r| r.style.font_style.clone()));
        }
    });
    assert_eq!(styles, ["Bold"]);
}

/// The export of `text` (one point type object) with its ToUnicode map rewritten so that the
/// digits map to U+FFFD, as some files have it (#708). Each `<003D>` of a digit becomes `<FFFD>`, so
/// the file's offsets still hold.
fn digits_unnamed(text: &str) -> Vec<u8> {
    let d = doc(vec![TextObject::point(Point::new(20.0, 50.0), text, style(24.0))]);
    let bytes = pdf(&d, false).bytes;
    let at = bytes.windows(11).position(|w| w == b"beginbfchar").expect("a readable ToUnicode map");
    let end = at + bytes[at..].windows(9).position(|w| w == b"endbfchar").unwrap();
    let mut out = bytes.clone();
    for i in at..end.saturating_sub(5) {
        // `<0030>` … `<0039>` after a space: a digit's character (each line's code comes first).
        if &bytes[i..i + 4] == b"<003" && bytes[i + 4].is_ascii_digit() && bytes[i + 5] == b'>' && bytes[i - 1] == b' ' {
            out[i + 1..i + 5].copy_from_slice(b"FFFD");
        }
    }
    assert_ne!(out, bytes, "digits were mapped");
    out
}

/// #708, #811: a glyph whose ToUnicode value is U+FFFD isn't type showing "�". This subset font
/// has no character map of its own, but the font is installed: each digit is found by its outline
/// there, so the whole text stays one type object, with no outlines left behind.
#[test]
fn unnamed_glyphs_of_an_installed_font_are_found_by_their_outline() {
    let bytes = digits_unnamed("Tel 02-12345");
    let report = import_with_report(&bytes, &ImportOptions { text_as: TextAs::Text, ..Default::default() }).unwrap();
    assert_eq!(texts(&report.document), ["Tel 02-12345"]);
    let mut paths = 0;
    report.document.walk(|n| paths += usize::from(matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    assert_eq!(paths, 0, "no outlines left: {:?}", report.warnings);
    assert!(!report.warnings.iter().any(|w| w.contains("U+FFFD")), "{:?}", report.warnings);
}

/// #811: an outline is named only by a glyph that draws it exactly; one moved off its place, or an
/// empty glyph (which every space draws), stays unnamed and keeps its outlines.
#[test]
fn glyphs_are_named_only_by_an_exact_outline() {
    let db = FontDb::global();
    let face = db.face("Source Sans 3", "Regular").unwrap();
    let k = 1000.0 / face.units_per_em();
    let pdf_space = |c: char| Affine::new([k, 0.0, 0.0, -k, 0.0, 0.0]) * db.outline(&face, face.glyph_for(c)).as_ref().clone();
    for c in ['R', 'a', '7', '&', 'é'] {
        assert_eq!(crate::import::identify_glyph(&face, &pdf_space(c)), Some(c));
    }
    assert_eq!(crate::import::identify_glyph(&face, &(Affine::translate((100.0, 0.0)) * pdf_space('R'))), None);
    assert_eq!(crate::import::identify_glyph(&face, &BezPath::new()), None);
}

/// #708: where the embedded font program has a character map, a glyph's character comes from it.
#[test]
fn an_embedded_fonts_own_cmap_names_its_glyphs() {
    let data = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    let chars = crate::import::font_chars(&data).unwrap();
    use skrifa::MetadataProvider;
    let font = skrifa::FontRef::new(&data).unwrap();
    for c in ['0', '7', 'A', 'z'] {
        let g = font.charmap().map(c).unwrap().to_u32();
        assert_eq!(chars.get(&g), Some(&c));
    }
    assert!(crate::import::font_chars(b"not a font").is_none());
}

/// Type whose text matrix shears its glyphs (slanted as a whole, as Illustrator writes type
/// turned and skewed) reopens as type that still leans: drawn, it covers what its outlines do.
#[test]
fn slanted_type_reopens_as_type_that_still_leans() {
    for (turn, lean) in [(0.0, 0.25), (-0.2, 0.25), (0.3, -0.15)] {
        let mut t = TextObject::point(Point::new(80.0, 150.0), "LEAN", style(60.0));
        t.xf *= Affine::rotate(turn) * Affine::skew(lean, 0.0);
        let d = doc(vec![t]);
        let reopened = import_as(&pdf(&d, false).bytes, TextAs::Text);
        assert_eq!(texts(&reopened), ["LEAN"], "{turn} {lean}: reopened as type");
        let (a, b) = (ink(&import_as(&pdf(&reopened, true).bytes, TextAs::Outlines)), ink(&import_as(&pdf(&d, true).bytes, TextAs::Outlines)));
        let off = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.5, "{turn} {lean}: {a:?} vs {b:?}");
    }
}

/// Type set in an installed font's older version (a library keeps both, one name) reopens as type
/// in that version: the file's glyphs are matched to it rather than kept as outlines, and the type
/// names the version, which the family and style alone don't pick.
#[test]
fn type_in_another_installed_version_reopens_in_that_version() {
    use vectorcraft_text::test_fonts::{TWIN_FAMILY, TWIN_VERSIONS, twin_font};
    // The newer version first: the one the family and style resolve to.
    FontDb::global().add_font(twin_font(true).unwrap());
    FontDb::global().add_font(twin_font(false).unwrap());
    for version in [None, Some(TWIN_VERSIONS[0])] {
        let st = CharStyle { font_family: TWIN_FAMILY.into(), font_version: version.map(Into::into), ..style(40.0) };
        let d = doc(vec![TextObject::point(Point::new(20.0, 80.0), "Hamburgefonstiv", st)]);
        let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
        let mut found = vec![];
        back.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                found.push((t.plain_text(), t.first_style().font_version.clone()));
            }
        });
        assert_eq!(found, [("Hamburgefonstiv".to_string(), version.map(String::from))], "{version:?}");
    }
}

/// The glyph origins of the type objects of `d`, in document space, in order.
fn origins(d: &Document) -> Vec<Point> {
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            out.extend(vectorcraft_text::layout(FontDb::global(), t).glyphs.iter().map(|g| t.xf * g.origin));
        }
    });
    out
}

/// Type spaced glyph by glyph (character tightening, optical kerning) comes back with each glyph
/// in its place, not evened out by one tracking value; evenly spaced type keeps one run.
#[test]
fn glyphs_spaced_each_their_own_way_keep_their_place() {
    let runs = [("T", 0.0), ("i", -120.0), ("g", 160.0), ("h", -60.0), ("tl", 0.0), ("y", 0.0)];
    let mut t = TextObject::point(Point::new(20.0, 100.0), "", style(40.0));
    t.runs = runs.iter().map(|(s, tr)| TextRun { text: s.to_string(), style: CharStyle { tracking: *tr, ..style(40.0) }, inline: None }).collect();
    let d = doc(vec![t]);
    let want = origins(&d);
    let back = import_as(&pdf(&d, false).bytes, TextAs::default());
    let got = origins(&back);
    assert_eq!(got.len(), want.len(), "{:?}", texts(&back));
    for (g, w) in got.iter().zip(&want) {
        assert!((g.x - w.x).abs() < 0.1 && (g.y - w.y).abs() < 0.1, "{got:?}\nvs {want:?}");
    }
    let mut runs_back = 0;
    back.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            runs_back += t.runs.len();
        }
    });
    assert!(runs_back > 1, "one tracking can't place them");
    // Evenly tracked: one run.
    let even = doc(vec![TextObject::point(Point::new(20.0, 100.0), "Tightly", CharStyle { tracking: -50.0, ..style(40.0) })]);
    let back = import_as(&pdf(&even, false).bytes, TextAs::default());
    back.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            assert_eq!(t.runs.len(), 1, "{:?}", t.runs.iter().map(|r| r.style.tracking).collect::<Vec<_>>());
        }
    });
}

/// Words set apart by spaces tracked past an em (#2384) come back with each glyph in its place:
/// the file draws each space and leaves the gap after it. Evenly tracked words keep one run.
#[test]
fn words_set_apart_by_spaces_tracked_past_an_em_keep_their_place() {
    let runs = [("Hooks", 0.0), (" ", 1800.0), ("and", 0.0), (" ", 2400.0), ("leads", 0.0)];
    let mut t = TextObject::point(Point::new(20.0, 100.0), "", style(12.0));
    t.runs = runs.iter().map(|(s, tr)| TextRun { text: s.to_string(), style: CharStyle { tracking: *tr, ..style(12.0) }, inline: None }).collect();
    let d = doc(vec![t]);
    let want = origins(&d);
    let back = import_as(&pdf(&d, false).bytes, TextAs::default());
    assert_eq!(texts(&back), ["Hooks and leads"]);
    let got = origins(&back);
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        assert!((g.x - w.x).abs() < 0.1 && (g.y - w.y).abs() < 0.1, "{got:?}\nvs {want:?}");
    }
    // Evenly tracked, the spaces too: one run.
    let even = doc(vec![TextObject::point(Point::new(20.0, 100.0), "Hooks and leads", CharStyle { tracking: 150.0, ..style(12.0) })]);
    let back = import_as(&pdf(&even, false).bytes, TextAs::default());
    assert_eq!(texts(&back), ["Hooks and leads"]);
    back.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            assert_eq!(t.runs.len(), 1, "{:?}", t.runs.iter().map(|r| r.style.tracking).collect::<Vec<_>>());
        }
    });
}

/// Evenly tracked type on a path comes back with its tracking: each glyph where it was along the
/// curve, not closed up to the font's advances.
#[test]
fn evenly_tracked_type_on_a_path_keeps_its_length() {
    let mut arc = BezPath::new();
    arc.move_to((40.0, 200.0));
    arc.curve_to((100.0, 80.0), (260.0, 80.0), (320.0, 200.0));
    let mut t = TextObject::point(Point::ORIGIN, "ARCHING TYPE", CharStyle { tracking: 80.0, ..style(24.0) });
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&arc), start: 0.0, end: None };
    let d = doc(vec![t]);
    let want = origins(&d);
    let back = import_as(&pdf(&d, false).bytes, TextAs::default());
    let got = origins(&back);
    assert_eq!(got.len(), want.len(), "{:?}", texts(&back));
    for (g, w) in got.iter().zip(&want) {
        assert!(g.distance(*w) < 0.3, "{got:?}\nvs {want:?}");
    }
}

/// Type filled with a dithered gradient is an image clipped to its outlines, sampled at the raster
/// effects resolution on the page however the type is scaled, with its characters under it as
/// invisible text (rendering mode 3), so they can still be selected and searched.
#[test]
fn dithered_type_keeps_its_resolution_when_scaled_and_stays_searchable() {
    let fill = Paint::Gradient(Box::new(GradientPaint::new(Gradient { dither: true, ..Default::default() })));
    let big = TextObject::point(Point::new(20.0, 80.0), "Dither", CharStyle { fill: fill.clone(), ..style(40.0) });
    let mut scaled = TextObject::point(Point::new(5.0, 20.0), "Dither", CharStyle { fill, ..style(10.0) });
    scaled.xf = Affine::scale(4.0) * scaled.xf;
    let width = |bytes: &[u8]| -> u32 {
        let s = String::from_utf8_lossy(bytes);
        s.split("/Width ").nth(1).and_then(|w| w.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap()
    };
    let (big, scaled) = (pdf(&doc(vec![big]), false).bytes, pdf(&doc(vec![scaled]), false).bytes);
    let (wb, ws) = (width(&big), width(&scaled));
    // At 72 ppi a pixel a point: 40 pt "Dither" is about 100 points wide.
    assert!(wb >= 80 && wb.abs_diff(ws) <= 2, "40 pt type {wb} pixels wide, 10 pt type scaled 4 times {ws}");
    let ops = String::from_utf8_lossy(&scaled).split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(ops.contains("3 Tr"), "the characters as invisible text");
    assert_eq!(texts(&import_as(&scaled, TextAs::Text)), ["Dither"]);
}
