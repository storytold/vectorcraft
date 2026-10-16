//! Affinity documents: native import (layers, artboards, curves, paints) through Open, Place and
//! templates, and the embedded preview only as an explicitly warned fallback.

use super::*;
use proptest::prelude::*;
use serde_json::json;
use std::io::Cursor;

fn png() -> Vec<u8> {
    let im = image::RgbaImage::from_fn(2, 1, |x, _| if x == 0 { image::Rgba([220, 40, 60, 255]) } else { image::Rgba([0, 0, 0, 0]) });
    let mut bytes = Vec::new();
    im.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

/// Synthetic envelope without a native object graph; not a native Affinity writer.
fn file(png: &[u8]) -> Vec<u8> {
    let mut b = vec![0; 72];
    b[..4].copy_from_slice(vectorcraft_affinity::MAGIC);
    b[4..6].copy_from_slice(&12u16.to_le_bytes());
    b[8..12].copy_from_slice(b"nsrP");
    b[12..16].copy_from_slice(b"#Inf");
    b[24..32].copy_from_slice(&72u64.to_le_bytes());
    b[64..68].copy_from_slice(b"Prot");
    b.extend(b"\xff\xff\xff\xffThmb");
    b.extend(1u32.to_le_bytes());
    b.extend((png.len() as u32 + 13).to_le_bytes());
    b.extend(29u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend((png.len() as u32).to_le_bytes());
    b.push(1);
    b.extend(png);
    b
}

#[test]
fn preview_opens_with_a_warning_and_never_saves_to_the_source() {
    let mut s = Session::new();
    let bytes = file(&png());
    for name in ["drawing.af", "renamed.png", "unknown"] {
        let result = open_bytes(&mut s, name, &bytes, Some(format!("/source/{name}"))).unwrap();
        assert_eq!(result["format"], "affinity");
        let w = result["warnings"][0].as_str().unwrap();
        assert!(w.contains("2×1") && w.contains("native Affinity document could not be read") && w.contains("no layers"), "{w}");
        let st = s.doc().unwrap();
        assert!(st.path.is_none());
        assert_eq!(st.format, "vectorcraft");
        assert_eq!((st.doc.artboards[0].rect.width(), st.doc.artboards[0].rect.height()), (2.0, 1.0));
        assert_eq!(st.doc.images.values().next().unwrap().bytes.as_slice(), png());
        let saved = vectorcraft_format::save_file(&st.doc);
        let reopened = vectorcraft_format::load_file(&saved).unwrap();
        assert_eq!(reopened.doc.images, st.doc.images);
        let plan = save_plan(&s, SaveMode::Save, &json!({})).unwrap();
        assert!(plan.path.is_none(), "Save must ask for a new destination");
        assert_eq!(plan.format.id, "vectorcraft");
        assert!(plan.name.ends_with(".vectorcraft"));
    }
    let f = format("affinity").unwrap();
    assert!(f.read && !f.write);
    for ext in ["af", "afdesign", "afpub"] {
        assert!(OPEN_EXTS.contains(&ext) && PLACE_EXTS.contains(&ext), "{ext}");
    }
    assert!(!SAVE_FORMATS.contains(&"affinity"));
    assert!(s.execute("document.serialize", &json!({"format":"af"})).is_err());
    assert!(s.execute("document.export", &json!({"format":"affinity"})).is_err());
    assert!(save_format(None, Some("drawing.af")).is_err());
}

#[test]
fn preview_reaches_the_document_open_command() {
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"name":"test.af", "dataBase64":vectorcraft_format::base64_encode(&file(&png()))})).unwrap();
    assert_eq!(r["format"], "affinity");
    assert_eq!(r["warnings"].as_array().unwrap().len(), 1);
}

#[test]
fn placing_a_preview_is_rejected_even_when_renamed_or_queued() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
    let original = s.doc().unwrap().doc.clone();
    for name in ["test.af", "renamed.png", "renamed.txt"] {
        let p = json!({"name":name, "dataBase64":vectorcraft_format::base64_encode(&file(&png())), "link":true});
        for cmd in ["file.place", "file.place.info"] {
            let e = s.execute(cmd, &p).unwrap_err().to_string();
            assert!(e.contains("File › Open") && e.contains("embedded preview"), "{e}");
        }
        let e = s.execute("file.place.queue", &json!({"files":[p]})).unwrap_err().to_string();
        assert!(e.contains("File › Open") && e.contains("embedded preview"), "{e}");
    }
    assert_eq!(s.doc().unwrap().doc, original);
}

#[test]
fn templates_and_libraries_cannot_silently_extract_preview_content() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
    let original = s.doc().unwrap().doc.clone();
    for name in ["test.af", "renamed.svg"] {
        let p = json!({"name":name, "dataBase64":vectorcraft_format::base64_encode(&file(&png()))});
        for cmd in ["file.newFromTemplate", "swatch.library.load", "graphicStyle.loadLibrary"] {
            let e = s.execute(cmd, &p).unwrap_err().to_string();
            assert!(e.contains("File › Open"), "{cmd}: {e}");
        }
    }
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.doc().unwrap().doc, original);
}

#[test]
fn malformed_crc_and_every_truncation_fail() {
    let b = file(&png());
    for end in 0..b.len() {
        assert!(load("bad.af", &b[..end]).is_err(), "{end}");
    }
    let mut crc = b.clone();
    crc[130] ^= 1;
    assert!(load("bad.af", &crc).is_err());
    // Affinity 1 and 2 (container versions 8 to 11) store the same preview record.
    let mut legacy = b.clone();
    legacy[4..6].copy_from_slice(&10u16.to_le_bytes());
    assert!(load("old.afdesign", &legacy).is_ok());
    let mut unknown = b.clone();
    unknown[4..6].copy_from_slice(&13u16.to_le_bytes());
    assert!(load("new.af", &unknown).err().unwrap().to_string().contains("unknown container version"));
}

#[test]
fn end_and_post_image_chunk_crcs_cannot_escape_pixel_decoding() {
    let mut end_crc = png();
    *end_crc.last_mut().unwrap() ^= 1;
    assert!(load("bad-end.af", &file(&end_crc)).is_err());

    let body = b"Note\0Synthetic";
    let mut text = (body.len() as u32).to_be_bytes().to_vec();
    text.extend(b"tEXt");
    text.extend(body);
    let mut h = crc32fast::Hasher::new();
    h.update(b"tEXt");
    h.update(body);
    text.extend(h.finalize().to_be_bytes());
    let mut post_image = png();
    let at = post_image.len() - 12;
    post_image.splice(at..at, text);
    assert!(load("valid-text.af", &file(&post_image)).is_ok());
    post_image[at + 8 + body.len()] ^= 1;
    assert!(load("bad-text.af", &file(&post_image)).is_err());
}

#[test]
fn a_previewless_file_fails_without_replacing_the_active_document() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
    let original = s.doc().unwrap().doc.clone();
    let mut bytes = file(&png());
    bytes[24..32].copy_from_slice(&0u64.to_le_bytes());
    let e = s.execute("document.open", &json!({"name":"no-preview.af", "dataBase64":vectorcraft_format::base64_encode(&bytes)})).unwrap_err();
    assert!(e.to_string().contains("no embedded preview"));
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.doc().unwrap().doc, original);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn bounded_mutations_do_not_panic(edits in prop::collection::vec((0usize..300, any::<u8>()), 0..16)) {
        let mut b = file(&png());
        for (i, value) in edits { if let Some(byte) = b.get_mut(i) { *byte = value; } }
        let _ = load("mutated.af", &b);
    }
}

mod native {
    use super::*;
    use vectorcraft_affinity::synth::{self, F, Method, tag};
    use vectorcraft_doc::{NodeKind, appearance::AppearanceItem};

    fn rgba(r: f32, g: f32, b: f32, a: f32) -> F {
        F::Struct([r, g, b, a].iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    fn solid(id: u32, c: F) -> F {
        F::Def(
            id,
            vec![tag(b"FDsc")],
            vec![(
                tag(b"FDeF"),
                F::Def(id + 1, vec![tag(b"FilS")], vec![(tag(b"Colr"), F::Def(id + 2, vec![tag(b"RGBA")], vec![(tag(b"_col"), c)]))]),
            )],
        )
    }

    fn node(x: f64, y: f64, kind: u8, role: u8) -> Vec<u8> {
        let mut r = x.to_le_bytes().to_vec();
        r.extend(y.to_le_bytes());
        r.extend([kind, role]);
        r
    }

    /// A version-12 document at 144 ppi: an artboard "Board" (moved 20, 10) holding a closed
    /// triangle with a red fill and a 4 px blue stroke, and a hidden ellipse on a layer.
    pub(super) fn document(method: Method) -> Vec<u8> {
        let triangle = F::Def(
            20,
            vec![tag(b"PCrv"), tag(b"VNod"), tag(b"Node")],
            vec![
                (tag(b"Desc"), F::Str("Triangle".into())),
                (
                    tag(b"Crvs"),
                    F::Obj(
                        tag(b"PCvD"),
                        vec![(
                            tag(b"Data"),
                            F::Pos(vec![
                                F::U8(0),
                                F::U32(1),
                                F::Bool(true),
                                F::Records(18, vec![node(0.0, 0.0, 1, 0), node(100.0, 0.0, 1, 0), node(50.0, 80.0, 1, 0), node(0.0, 0.0, 1, 0)]),
                            ]),
                        )],
                    ),
                ),
                (tag(b"BFFl"), F::Shared(vec![solid(21, rgba(1.0, 0.0, 0.0, 1.0))])),
                (tag(b"LIFl"), F::Shared(vec![solid(24, rgba(0.0, 0.0, 1.0, 1.0))])),
                (
                    tag(b"LILn"),
                    F::Shared(vec![F::Def(
                        27,
                        vec![tag(b"LDsc")],
                        vec![(tag(b"LDeL"), F::Def(28, vec![tag(b"LSty")], vec![(tag(b"Wght"), F::F64(4.0))])), (tag(b"LDSc"), F::Bool(true))],
                    )]),
                ),
            ],
        );
        let board = F::Def(
            10,
            vec![tag(b"ShpN"), tag(b"VNod"), tag(b"Node")],
            vec![
                (tag(b"Desc"), F::Str("Board".into())),
                (tag(b"ABEn"), F::Bool(true)),
                (tag(b"Shpe"), F::Def(11, vec![tag(b"ShNR")], vec![])),
                (tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 200.0, 100.0])),
                (tag(b"Xfrm"), F::F64s(vec![1.0, 0.0, 20.0, 0.0, 1.0, 10.0])),
                (tag(b"BFFl"), F::Shared(vec![solid(12, rgba(1.0, 1.0, 1.0, 1.0))])),
                (tag(b"Chld"), F::Shared(vec![triangle])),
            ],
        );
        let ellipse = F::Def(
            40,
            vec![tag(b"ShpN")],
            vec![
                (tag(b"Visi"), F::Bool(false)),
                (tag(b"Shpe"), F::Def(41, vec![tag(b"ShpE")], vec![])),
                (tag(b"ShpB"), F::F64s(vec![300.0, 0.0, 400.0, 50.0])),
                (tag(b"BFFl"), F::Shared(vec![F::Def(42, vec![tag(b"FDsc")], vec![(tag(b"FDeF"), F::Def(43, vec![tag(b"FilN")], vec![]))])])),
            ],
        );
        let layer = F::Def(30, vec![tag(b"Scop")], vec![(tag(b"Desc"), F::Str("Shapes".into())), (tag(b"Chld"), F::Shared(vec![ellipse]))]);
        let spread =
            F::Def(2, vec![tag(b"Sprd")], vec![(tag(b"SprB"), F::F64s(vec![0.0, 0.0, 420.0, 120.0])), (tag(b"Chld"), F::Shared(vec![board, layer]))]);
        let doc = synth::stream(&[
            (tag(b"UVCn"), F::Obj(tag(b"UVCn"), vec![(tag(b"UPPI"), F::F64(144.0))])),
            (tag(b"DocR"), F::Def(1, vec![tag(b"DocN")], vec![(tag(b"Chld"), F::Shared(vec![spread]))])),
        ]);
        synth::container(&[("doc.dat", &doc, method)], Some(&png()))
    }

    /// A version-12 document at 144 ppi holding a 100 px square filled with a red-to-blue conical
    /// gradient centred at (50, 40) px, radius 40 px (`mirror`: its unit +y points up).
    fn conical_document(mirror: bool) -> Vec<u8> {
        let stop = |id: u32, c: F| F::Def(id, vec![tag(b"RGBA")], vec![(tag(b"_col"), c)]);
        let positions = [[0.0f32, 0.5], [1.0, 0.5]].iter().map(|p| p.iter().flat_map(|v| v.to_le_bytes()).collect()).collect();
        let gradient = F::Def(
            21,
            vec![tag(b"FDsc")],
            vec![
                (tag(b"FDeX"), F::F64s(vec![40.0, 0.0, 50.0, 0.0, if mirror { -40.0 } else { 40.0 }, 40.0])),
                (
                    tag(b"FDeF"),
                    F::Def(
                        22,
                        vec![tag(b"FilG")],
                        vec![
                            (tag(b"Type"), F::Enum(3, 0)),
                            (
                                tag(b"Grad"),
                                F::Obj(
                                    tag(b"Grad"),
                                    vec![
                                        (tag(b"Cols"), F::Shared(vec![stop(23, rgba(1.0, 0.0, 0.0, 1.0)), stop(24, rgba(0.0, 0.0, 1.0, 1.0))])),
                                        (tag(b"Posn"), F::Records(8, positions)),
                                    ],
                                ),
                            ),
                        ],
                    ),
                ),
            ],
        );
        let square = F::Def(
            20,
            vec![tag(b"ShpN")],
            vec![
                (tag(b"Shpe"), F::Def(25, vec![tag(b"ShNR")], vec![])),
                (tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 100.0, 100.0])),
                (tag(b"BFFl"), F::Shared(vec![gradient])),
            ],
        );
        let spread =
            F::Def(2, vec![tag(b"Sprd")], vec![(tag(b"SprB"), F::F64s(vec![0.0, 0.0, 200.0, 200.0])), (tag(b"Chld"), F::Shared(vec![square]))]);
        let doc = synth::stream(&[
            (tag(b"UVCn"), F::Obj(tag(b"UVCn"), vec![(tag(b"UPPI"), F::F64(144.0))])),
            (tag(b"DocR"), F::Def(1, vec![tag(b"DocN")], vec![(tag(b"Chld"), F::Shared(vec![spread]))])),
        ]);
        synth::container(&[("doc.dat", &doc, Method::Stored)], Some(&png()))
    }

    #[test]
    fn conical_gradients_open_as_conic_gradients() {
        use vectorcraft_color::{GradientKind, Paint};
        for mirror in [false, true] {
            let mut s = Session::new();
            let r = open_bytes(&mut s, "conic.af", &conical_document(mirror), None).unwrap();
            assert!(r["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap().contains("conical")), "{r}");
            let mut paint = None;
            s.doc().unwrap().doc.walk(|n| {
                if let Paint::Gradient(g) = n.appearance.fill_paint() {
                    paint = Some(g.clone());
                }
            });
            let g = paint.expect("a gradient fill");
            assert_eq!(g.gradient.kind, GradientKind::Conic);
            let geom = g.geom.unwrap();
            // 144 ppi: centre (25, 20) pt, radius 20 pt, red at the start of the sweep.
            assert!(geom.start.distance(kurbo::Point::new(25.0, 20.0)) < 1e-9 && (geom.length() - 20.0).abs() < 1e-9, "{geom:?}");
            assert_eq!(geom.aspect < 0.0, mirror, "{geom:?}");
            // A quarter turn on from the start lies along the unit +y: below the centre, or
            // above it when mirrored.
            let quarter = kurbo::Point::new(25.0, if mirror { 10.0 } else { 30.0 });
            assert!((geom.param_at(GradientKind::Conic, quarter) - 0.25).abs() < 1e-9, "{geom:?}");
            assert_eq!(g.gradient.stops[0].color.to_hex(), "#ff0000");
        }
    }

    #[test]
    fn layers_artboards_curves_and_paints_open_as_editable_art() {
        for method in [Method::Stored, Method::Zlib, Method::Zstd] {
            let mut s = Session::new();
            let r = open_bytes(&mut s, "native.af", &document(method), Some("/source/native.af".into())).unwrap();
            assert_eq!(r["format"], "affinity");
            // Affinity Photo's extension isn't listed, but its documents still open by their content.
            assert_eq!(open_bytes(&mut Session::new(), "native.afphoto", &document(method), None).unwrap()["format"], "affinity");
            assert!(r["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap().contains("preview")), "{r}");
            let st = s.doc().unwrap();
            assert!(st.path.is_none(), "Save must not write back over the Affinity file");
            let d = &st.doc;
            // 144 ppi: two document pixels per point.
            assert_eq!(d.artboards.len(), 1);
            assert_eq!(d.artboards[0].name, "Board");
            assert_eq!(d.artboards[0].rect, kurbo::Rect::new(10.0, 5.0, 110.0, 55.0));
            let names: Vec<_> = d.layers.iter().map(|l| l.name.clone().unwrap()).collect();
            assert_eq!(names, ["Board", "Shapes"]);
            let NodeKind::Layer { children, .. } = &d.layers[0].kind else { panic!() };
            let NodeKind::Group { children: clipped, clip: true } = &children[0].kind else { panic!("{:?}", children[0].kind) };
            let NodeKind::Path { clipping: true, .. } = &clipped[0].kind else { panic!() };
            let tri = &clipped[1];
            assert_eq!(tri.name.as_deref(), Some("Triangle"));
            let NodeKind::Path { path, .. } = &tri.kind else { panic!() };
            let pts: Vec<_> = path.subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect();
            assert_eq!(pts, [(10.0, 5.0), (60.0, 5.0), (35.0, 45.0)]);
            assert!(path.subpaths[0].closed);
            let [AppearanceItem::Fill(f), AppearanceItem::Stroke(k)] = tri.appearance.items.as_slice() else { panic!("{:?}", tri.appearance) };
            assert_eq!(f.paint, vectorcraft_color::Paint::solid(vectorcraft_color::Color::Rgb { r: 1.0, g: 0.0, b: 0.0 }));
            assert_eq!(k.width, 2.0);
            let NodeKind::Layer { children, .. } = &d.layers[1].kind else { panic!() };
            assert!(!children[0].visible, "hidden layers stay hidden");
        }
    }

    #[test]
    fn native_documents_save_reopen_export_place_and_template() {
        let bytes = document(Method::Zstd);
        let mut s = Session::new();
        open_bytes(&mut s, "native.af", &bytes, None).unwrap();
        let doc = s.doc().unwrap().doc.clone();
        let reopened = vectorcraft_format::load_file(&vectorcraft_format::save_file(&doc)).unwrap();
        assert_eq!(reopened.doc.layers, doc.layers);
        for format in ["svg", "pdf", "png"] {
            assert!(s.execute("document.export", &json!({"format": format})).is_ok(), "{format}");
        }
        let p = json!({"name":"native.afdesign", "dataBase64":vectorcraft_format::base64_encode(&bytes)});
        s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
        s.execute("file.place", &p).unwrap();
        assert!(s.doc().unwrap().doc.layers.iter().any(|l| !l.children().unwrap().is_empty()));
        s.execute("file.newFromTemplate", &p).unwrap();
        assert!(s.doc().unwrap().path.is_none());
    }

    #[test]
    fn a_damaged_native_document_falls_back_to_its_preview_with_the_reason() {
        let mut bytes = document(Method::Zlib);
        // Break the doc.dat CRC: the preview still opens, saying why.
        bytes[80] ^= 0xFF;
        let r = load("broken.af", &bytes).unwrap();
        assert!(r.preview_only);
        assert!(r.warnings[0].contains("could not be read (damaged Affinity file"), "{}", r.warnings[0]);
    }
}
