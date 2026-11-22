//! DCS files made from scratch as http://www.telegraphics.net/doc/DCS.txt describes them: a main
//! EPS file whose header names its plates, each plate an EPS program drawing one grayscale image.

use std::collections::HashMap;

use vectorcraft_doc::NodeKind;

use super::*;

/// A plate program: one `w` × `h` grayscale image (`gray`, row by row from the top) on a 20 × 10
/// pt page.
pub(crate) fn plate_eps(w: u32, h: u32, gray: &[u8]) -> String {
    let hex: String = gray.iter().map(|g| format!("{g:02x}")).collect();
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n%%EndComments\n20 10 scale\n{w} {h} 8 [{w} 0 0 -{h} 0 {h}] <{hex}> image\nshowpage\n%%Trailer\n%%EOF\n"
    )
}

/// The composite: the image `w` × `h` drawn through a clipping path, the left half of the page.
fn composite(w: u32, h: u32) -> String {
    let hex = "80".repeat((w * h * 3) as usize);
    format!(
        "gsave newpath 0 0 moveto 10 0 lineto 10 10 lineto 0 10 lineto closepath clip newpath\n20 10 scale\n{w} {h} 8 [{w} 0 0 -{h} 0 {h}] <{hex}> false 3 colorimage\ngrestore\nshowpage\n%%EOF\n"
    )
}

/// A single-file DCS 2.0 file of `plates` (name, plate program), with the header lines `extra`.
pub(crate) fn single_file(plates: &[(&str, String)], extra: &str) -> Vec<u8> {
    // Offsets and sizes are written ten digits wide, so the header's length is known first.
    let header = |offsets: &[usize]| {
        let mut s = String::from("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n");
        s.push_str(extra);
        for ((name, eps), at) in plates.iter().zip(offsets) {
            s.push_str(&format!("%%PlateFile: ({name}) EPS #{at:010} {:010}\n", eps.len()));
        }
        s.push_str("%%EndComments\n");
        s.push_str(&composite(2, 2));
        s
    };
    let main = header(&vec![0; plates.len()]).len();
    let mut offsets = vec![];
    let mut at = main;
    for (_, eps) in plates {
        offsets.push(at);
        at += eps.len();
    }
    let mut out = header(&offsets).into_bytes();
    assert_eq!(out.len(), main);
    for (_, eps) in plates {
        out.extend(eps.as_bytes());
    }
    out
}

#[test]
fn single_file_plates_and_equivalents_are_read() {
    let plate = plate_eps(1, 1, &[0]);
    let extra = "%%DocumentCustomColors: (PANTONE 185 C)\n%%+ (Gr\\374n \\(A\\))\n%%CMYKCustomColor: 0 .91 .76 0 (PANTONE 185 C)\n%%+ .5 0 1 0 (Gr\\374n \\(A\\))\n%%RGBCustomColor: 1 1 0 (Lemon)\n";
    let bytes = single_file(&[("cyan", plate.clone()), ("PANTONE 185 C", plate.clone()), ("Grün (A)", plate.clone())], extra);
    let dcs = read(&bytes).unwrap();
    assert_eq!(dcs.version, 2);
    let names: Vec<&str> = dcs.plates.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Cyan", "PANTONE 185 C", "Grün (A)"], "a process ink in its usual case; escapes read");
    let PlateAt::Here { offset, len } = dcs.plates[1].at else { panic!("in the file: {:?}", dcs.plates[1]) };
    assert_eq!(&bytes[offset..offset + len], plate.as_bytes());
    assert_eq!(
        dcs.equivalents,
        [
            ("PANTONE 185 C".to_string(), [0.0, 0.91, 0.76, 0.0]),
            ("Grün (A)".to_string(), [0.5, 0.0, 1.0, 0.0]),
            ("Lemon".to_string(), [0.0, 0.0, 1.0, 0.0])
        ]
    );
    assert_eq!(dcs.plate_files().count(), 0);
    let main = main_program(&bytes, &dcs).unwrap();
    assert!(main.ends_with(b"%%EOF\n"), "up to the first plate");
    let PlateAt::Here { offset: first, .. } = dcs.plates[0].at else { panic!("in the file") };
    assert_eq!(main.len(), first);
}

#[test]
fn plate_files_of_both_versions_are_read() {
    let two = "%!PS-Adobe-3.0 EPSF-3.0\n%%PlateFile: (Black) EPS Local pic.K\n%%PlateFile: (Black) TIFF NewServe c:\\scans\\a0K.scn\n%%PlateFile: (TRUMATCH 1-a5) EPS Local my pic.9\n%%EndComments\n";
    let dcs = read(two.as_bytes()).unwrap();
    assert_eq!(dcs.version, 2);
    assert_eq!(
        dcs.plates,
        [
            PlateRef { name: "Black".into(), at: PlateAt::File("pic.K".into()) },
            PlateRef { name: "TRUMATCH 1-a5".into(), at: PlateAt::File("my pic.9".into()) },
        ]
    );
    let one = "%!PS-Adobe-2.0 EPSF-1.2\n%%CyanPlate: pic.C\n%%MagentaPlate: pic.M\n%%YellowPlate: pic.Y\n%%BlackPlate: pic.K\n%%EndComments\n";
    let dcs = read(one.as_bytes()).unwrap();
    assert_eq!(dcs.version, 1);
    assert_eq!(dcs.plate_files().collect::<Vec<_>>(), ["pic.C", "pic.M", "pic.Y", "pic.K"]);
    let both = "%!PS-Adobe-2.0 EPSF-1.2\n%%CyanPlate: old.C\n%%PlateFile: (Cyan) EPS Local new.C\n%%EndComments\n";
    assert_eq!(read(both.as_bytes()).unwrap().plate_files().collect::<Vec<_>>(), ["new.C"], "the DCS 2.0 comments win");
    // Comments after the header are not the file's.
    assert!(read(b"%!PS-Adobe-3.0 EPSF-3.0\n%%EndComments\n%%PlateFile: (Cyan) EPS Local a.C\n").is_none());
    assert!(read(plate_eps(1, 1, &[0]).as_bytes()).is_none(), "a plain EPS file");
    assert!(read(b"\x89PNG").is_none());
}

#[test]
fn a_plate_program_drawing_more_than_its_image_is_an_error() {
    let plate = plate_eps(1, 1, &[0]);
    // Two image tiles, and an image with a filled square beside it.
    let tiles = plate.replacen("showpage", "1 1 8 [1 0 0 -1 0 1] <00> image\nshowpage", 1);
    let art = plate.replacen("20 10 scale", "0 0 moveto 5 0 lineto 5 5 lineto closepath fill\n20 10 scale", 1);
    for (eps, why) in [(tiles, "more than one image"), (art, "art besides its image")] {
        let bytes = single_file(&[("Cyan", eps)], "");
        let err = place(&bytes, &read(&bytes).unwrap(), &mut |_| Err("no files".into())).unwrap_err();
        assert!(err.contains("the Cyan plate can't be read") && err.contains(why), "{err}");
    }
    let bytes = single_file(&[("Cyan", plate)], "");
    assert!(place(&bytes, &read(&bytes).unwrap(), &mut |_| Err("no files".into())).is_ok(), "one image alone");
}

#[test]
fn a_plate_program_clipping_its_image_is_an_error() {
    let clipped = plate_eps(1, 1, &[0]).replacen("20 10 scale", "newpath 0 0 moveto 5 0 lineto 5 5 lineto closepath clip newpath\n20 10 scale", 1);
    let bytes = single_file(&[("Cyan", clipped)], "");
    let err = place(&bytes, &read(&bytes).unwrap(), &mut |_| Err("no files".into())).unwrap_err();
    assert!(err.contains("the Cyan plate can't be read") && err.contains("clipping path"), "{err}");
}

#[test]
fn only_an_image_inside_a_clipping_group_or_layer_is_clipped() {
    use std::sync::Arc;
    use vectorcraft_color::Paint;
    use vectorcraft_doc::NodeId;
    let clip_path = || {
        let rect = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0));
        let mut p = Node::path(NodeId(1), rect, vectorcraft_doc::Appearance::basic(Paint::None, Paint::None, 0.0));
        if let NodeKind::Path { clipping, .. } = &mut p.kind {
            *clipping = true;
        }
        Arc::new(p)
    };
    let image = || {
        let im = vectorcraft_doc::ImageObject {
            key: "k".into(),
            width: 1,
            height: 1,
            xf: Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        Arc::new(Node::new(NodeId(2), NodeKind::Image(Box::new(im))))
    };
    let group = |children: Vec<Arc<Node>>, clip| Node::new(NodeId(3), NodeKind::Group { children, clip });
    let beside = group(vec![Arc::new(group(vec![clip_path()], true)), image()], false);
    assert!(!clips_an_image(&beside, false), "a clipping group beside the image");
    assert!(clips_an_image(&group(vec![clip_path(), image()], true), false), "the image inside the clipping group");
    assert!(clips_an_image(&group(vec![image()], false), true), "inside a clipping layer");
}

#[test]
fn nested_clips_around_the_composite_clip_it_together() {
    // The left two thirds of the page, then the right three quarters inside it: 5 to 13 pt.
    let main = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n%%EndComments\n\
        gsave newpath 0 0 moveto 13 0 lineto 13 10 lineto 0 10 lineto closepath clip newpath\n\
        gsave newpath 5 0 moveto 20 0 lineto 20 10 lineto 5 10 lineto closepath clip newpath\n\
        20 10 scale 2 2 8 [2 0 0 -2 0 2] <808080808080808080808080> false 3 colorimage grestore grestore showpage\n%%EOF\n";
    let clip = clip_path(main.as_bytes()).expect("a clipping path");
    let b = clip.geometric_bounds().expect("bounds");
    assert!((b.x0 - 5.0).abs() < 1e-6 && (b.x1 - 13.0).abs() < 1e-6, "where both clips overlap: {b:?}");
}

#[test]
fn plate_file_names_keep_their_spaces() {
    let two = "%!PS-Adobe-3.0 EPSF-3.0\n%%PlateFile: (Cyan) EPS Local  my  pic .C\n%%EndComments\n";
    assert_eq!(read(two.as_bytes()).unwrap().plate_files().collect::<Vec<_>>(), ["my  pic .C"]);
}

#[test]
fn header_bytes_that_are_not_utf8_are_latin1() {
    // `Grün` with a raw Latin-1 ü (0xFC), in a plate name and in a custom color's name.
    let mut two = b"%!PS-Adobe-3.0 EPSF-3.0\n%%CMYKCustomColor: .5 0 1 0 (Gr\xfcn)\n%%PlateFile: (Gr\xfcn) EPS Local a.9\n".to_vec();
    two.extend(b"%%PlateFile: (Grau \xc3\xa9) EPS Local b.9\n%%EndComments\n");
    let dcs = read(&two).unwrap();
    let names: Vec<&str> = dcs.plates.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Grün", "Grau é"], "a line that is UTF-8 is read as UTF-8");
    assert_eq!(dcs.equivalents, [("Grün".to_string(), [0.5, 0.0, 1.0, 0.0])]);
}

#[test]
fn every_plate_is_read_with_the_clipping_path() {
    // 2 × 2 pixels: cyan solid on the top row, the spot ink at 50% on the first column.
    let bytes = single_file(
        &[("Cyan", plate_eps(2, 2, &[0, 0, 255, 255])), ("PANTONE 185 C", plate_eps(2, 2, &[127, 255, 127, 255]))],
        "%%CMYKCustomColor: 0 .91 .76 0 (PANTONE 185 C)\n",
    );
    let dcs = read(&bytes).unwrap();
    let p = place(&bytes, &dcs, &mut |_| Err("no files".into())).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.page, Rect::new(0.0, 0.0, 20.0, 10.0));
    let s = &p.separated;
    assert_eq!((s.width, s.height), (2, 2));
    assert_eq!(s.plates[0].data, [255, 255, 0, 0], "a dark plate pixel is ink");
    assert_eq!(s.plates[1].data, [128, 0, 128, 0]);
    assert_eq!(s.plates[1].cmyk, [0.0, 0.91, 0.76, 0.0]);
    assert_eq!(p.xf * vectorcraft_geom::Point::new(2.0, 2.0), vectorcraft_geom::Point::new(20.0, 10.0));
    assert_eq!(p.ppi, Some((7.2, 14.4)));
    let clip = p.clip.expect("the composite's clipping path");
    assert!(matches!(clip.kind, NodeKind::Path { clipping: true, .. }));
    assert_eq!(clip.geometric_bounds().map(|b| b.width().round()), Some(10.0), "the left half of the page");
}

#[test]
fn plate_files_are_read_with_the_callback() {
    let main = "%!PS-Adobe-2.0 EPSF-1.2\n%%BoundingBox: 0 0 20 10\n%%CyanPlate: pic.C\n%%BlackPlate: pic.K\n%%EndComments\nshowpage\n";
    let files: HashMap<&str, String> = [("pic.C", plate_eps(1, 1, &[0])), ("pic.K", plate_eps(2, 1, &[255, 0]))].into();
    let dcs = read(main.as_bytes()).unwrap();
    let mut asked = vec![];
    let p = place(main.as_bytes(), &dcs, &mut |f| {
        asked.push(f.to_string());
        files.get(f).map(|e| e.as_bytes().to_vec()).ok_or_else(|| "missing".into())
    })
    .unwrap();
    assert_eq!(asked, ["pic.C", "pic.K"]);
    assert!(p.clip.is_none(), "no composite, no clipping path");
    assert_eq!((p.separated.width, p.separated.height), (1, 1), "the first plate's size");
    assert_eq!(p.separated.plates[1].data, [255], "resampled to it: the pixel at its centre");
    let err = place(main.as_bytes(), &dcs, &mut |_| Err("not found".into())).unwrap_err();
    assert_eq!(err, "the Cyan plate can't be read: not found");
}

#[test]
fn plates_printing_elsewhere_on_the_page_stay_in_register() {
    // The Black plate's one solid pixel prints on the right half of the page only.
    let right = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n%%EndComments\n10 0 translate 10 10 scale\n1 1 8 [1 0 0 -1 0 1] <00> image\nshowpage\n%%EOF\n";
    let bytes = single_file(&[("Cyan", plate_eps(2, 1, &[255, 255])), ("Black", right.to_string())], "");
    let p = place(&bytes, &read(&bytes).unwrap(), &mut |_| Err("no files".into())).unwrap();
    assert_eq!((p.separated.width, p.separated.height), (2, 1));
    assert_eq!(p.separated.plates[1].data, [0, 255], "the ink lands on the right pixel only");
}

/// A plate program whose one solid pixel prints on a 10 × 10 pt square `x` pt from the page's left.
fn square_at(x: u32) -> String {
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n%%EndComments\n{x} 0 translate 10 10 scale\n1 1 8 [1 0 0 -1 0 1] <00> image\nshowpage\n%%EOF\n"
    )
}

#[test]
fn the_plates_grid_grows_to_hold_every_plate() {
    let bytes = single_file(&[("Cyan", square_at(0)), ("Black", square_at(10))], "");
    let p = place(&bytes, &read(&bytes).unwrap(), &mut |_| Err("no files".into())).unwrap();
    assert_eq!((p.separated.width, p.separated.height), (2, 1), "the first plate's pixels, grown over the second");
    assert_eq!(p.separated.plates[0].data, [255, 0]);
    assert_eq!(p.separated.plates[1].data, [0, 255], "nothing of the second plate is cut off");
    assert_eq!(p.xf * vectorcraft_geom::Point::new(2.0, 1.0), vectorcraft_geom::Point::new(20.0, 10.0));
}

#[test]
fn a_file_naming_more_plates_than_are_read_is_an_error() {
    let plate = plate_eps(1, 1, &[0]);
    let names: Vec<String> = (0..=MAX_PLATES).map(|i| format!("Ink {i}")).collect();
    let plates: Vec<(&str, String)> = names.iter().map(|n| (n.as_str(), plate.clone())).collect();
    let bytes = single_file(&plates, "");
    let dcs = read(&bytes).unwrap();
    assert_eq!(dcs.plates.len(), MAX_PLATES + 1, "every plate named");
    let err = place(&bytes, &dcs, &mut |_| Err("no files".into())).unwrap_err();
    assert_eq!(err, "it names 65 plates, and at most 64 are read");
}

#[test]
fn bad_plates_are_errors_and_missing_equivalents_warnings() {
    let mut bytes = single_file(&[("Cyan", plate_eps(1, 1, &[0]))], "");
    let dcs = read(&bytes).unwrap();
    bytes.truncate(bytes.len() - 5);
    assert_eq!(place(&bytes, &dcs, &mut |_| Err(String::new())).unwrap_err(), "the Cyan plate lies outside the file");
    let text = single_file(&[("Cyan", "%!PS-Adobe-3.0\n0 0 moveto 1 1 lineto stroke\n".to_string())], "");
    let err = place(&text, &read(&text).unwrap(), &mut |_| Err(String::new())).unwrap_err();
    assert_eq!(err, "the Cyan plate can't be read: it draws no image");
    let spot = single_file(&[("Mystery", plate_eps(1, 1, &[0]))], "");
    let p = place(&spot, &read(&spot).unwrap(), &mut |_| Err(String::new())).unwrap();
    assert_eq!(p.separated.plates[0].cmyk, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(p.warnings, ["the file gives no process equivalent for the Mystery ink: it shows as black"]);
}
