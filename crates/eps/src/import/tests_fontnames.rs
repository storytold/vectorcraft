//! Type names the available family as the font spells it, not as the PostScript name splits into
//! words (#130).

use vectorcraft_doc::NodeKind;
use vectorcraft_text::FontDb;

use crate::import::import;

/// The bundled Source Sans 3 renamed `family` (as long as the original name).
fn renamed(family: &str) -> Vec<u8> {
    let mut data = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf").to_vec();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16(family)), (b"Source Sans 3".to_vec(), family.as_bytes().to_vec())] {
        assert_eq!(from.len(), to.len());
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

#[test]
fn postscript_names_resolve_to_the_available_family() {
    // Split into words, its PostScript name would read "My Face Ya Hei3".
    assert!(FontDb::global().add_font(renamed("MyFaceYaHei 3")) > 0);
    let body = "/MyFaceYaHei3-Bold findfont 12 scalefont setfont 10 20 moveto (Hi) show";
    let eps = format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{body}\nshowpage\n%%EOF\n");
    let r = import(eps.as_bytes()).unwrap();
    let mut styles = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            styles.extend(t.runs.iter().map(|r| (r.style.font_family.clone(), r.style.font_style.clone())));
        }
    });
    assert_eq!(styles, [("MyFaceYaHei 3".to_string(), "Bold".to_string())]);
}

#[test]
fn reencoded_copies_of_a_font_name_the_font() {
    // A program that re-encodes a font sets the copy under `<subset>+<name>*<n>`.
    assert_eq!(crate::family_style("KBKNNM+RollerBabyBV*1"), ("Roller Baby BV".to_string(), "Regular".to_string()));
    assert_eq!(crate::family_style("ABCDEF+Helvetica-Bold*12"), ("Helvetica".to_string(), "Bold".to_string()));
    // Only `*` and digits at the end go; other names are as they were.
    assert_eq!(crate::family_style("Odd*Name").0, "Odd*Name");
    assert_eq!(crate::family_style("*1").0, "*1");
}

#[test]
fn abbreviated_styles_in_postscript_names_are_spelled_out() {
    // A style word PostScript names shorten is the style the installed font names in full.
    assert_eq!(crate::family_style("AkzidenzGroteskPro-Md"), ("Akzidenz Grotesk Pro".to_string(), "Medium".to_string()));
    assert_eq!(crate::family_style("AkzidenzGroteskPro-XBdCnIt").1, "Extra Bold Condensed Italic");
    assert_eq!(crate::family_style("AkzidenzGroteskPro-LightCn").1, "Light Condensed");
    // Whole words stay as they were.
    assert_eq!(crate::family_style("Helvetica-BoldOblique").1, "Bold Oblique");
}

/// A face of that exact PostScript name gives its own family and style, however the name splits
/// (a family can hold hyphens, as Rounded-X M+ 1c's `rounded-x-mplus-1c-black` does).
#[test]
fn an_installed_face_of_the_postscript_name_names_the_font() {
    assert_eq!(crate::family_style("ABCDEF+SourceSans3-Semibold"), ("Source Sans 3".to_string(), "Semibold".to_string()));
}

/// The PostScript name and version of the face each `show` of a program that sets `fonts` in turn
/// draws its type in.
fn faces_shown(fonts: &[&str]) -> Vec<(String, String)> {
    let body: String = fonts.iter().map(|f| format!("/{f} findfont 12 scalefont setfont 10 20 moveto (Hi) show\n")).collect();
    let eps = format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{body}showpage\n%%EOF\n");
    let r = import(eps.as_bytes()).unwrap();
    let mut faces = vec![];
    r.document.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            let face = FontDb::global().style_face(&t.runs[0].style).unwrap();
            faces.push((face.postscript_name(), face.version.clone()));
        }
    });
    faces
}

/// Type a PostScript program sets is drawn in the installed face of exactly the font's PostScript
/// name, as type from a text document is: not another family spelled alike but for case and
/// punctuation, nor a face whose name differs only in case and loads first.
#[test]
fn shown_type_takes_the_face_of_its_exact_postscript_name() {
    use vectorcraft_text::test_fonts::named_copy;
    for (inter, names) in [
        (true, ("FUZZSHOW INK", "Regular", "Version 1.000", "FUZZSHOWINK-Regular")),
        (false, ("Fuzzshow/Ink", "Regular", "Version 1.000", "FuzzshowInk")),
        (true, ("CASESHOW INK", "Regular", "Version 1.000", "CASESHOWINK")),
        (false, ("Caseshow Ink", "Regular", "Version 2.000", "CaseshowInk")),
    ] {
        assert_eq!(FontDb::global().add_font(named_copy(inter, names).unwrap()), 1, "{names:?}");
    }
    let names: Vec<String> = faces_shown(&["FuzzshowInk", "CaseshowInk", "CASESHOWINK"]).into_iter().map(|f| f.0).collect();
    assert_eq!(names, ["FuzzshowInk", "CaseshowInk", "CASESHOWINK"]);
}
