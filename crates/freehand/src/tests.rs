use super::*;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

fn flat(items: &[Item], out: &mut Vec<Item>) {
    for item in items {
        match item {
            Item::Group(kids) | Item::ClipGroup { children: kids, .. } => {
                out.push(item.clone());
                flat(kids, out);
            }
            other => out.push(other.clone()),
        }
    }
}

#[test]
fn demo_file_is_detected_and_places_art_on_the_page() {
    let bytes = synth::demo();
    assert!(is_freehand(&bytes));
    assert!(!is_freehand(b"%PDF-1.4"));
    let doc = read(&bytes, Limits::default()).expect("synthetic file");
    assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
    assert_eq!(doc.pages.len(), 1);
    assert!(near(doc.pages[0].width, 200.0) && near(doc.pages[0].height, 100.0));
    assert!(near(doc.pages[0].x, 0.0) && near(doc.pages[0].y, 0.0));
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layers[0].name, "Ink");
    assert!(doc.layers[0].visible && !doc.layers[0].locked);

    let mut items = Vec::new();
    flat(&doc.layers[0].items, &mut items);
    let paths: Vec<&Path> = items
        .iter()
        .filter_map(|i| match i {
            Item::Path(p) => Some(p),
            _ => None,
        })
        .collect();
    assert!(paths.len() >= 3, "path, rectangle and oval, got {}", paths.len());

    let drawn = paths.iter().find(|p| p.stroke.as_ref().is_some_and(|s| near(s.width, 2.0))).expect("stroked path");
    assert!(drawn.even_odd);
    let Fill::Solid(fill) = &drawn.fill else { panic!("solid fill") };
    assert!(near(fill.r, 1.0) && near(fill.g, 0.0) && near(fill.b, 0.0));
    let stroke = drawn.stroke.as_ref().expect("stroke");
    assert!(near(stroke.color.b, 1.0) && near(stroke.color.r, 0.0));
    assert_eq!(stroke.dash.len(), 2);
    assert!(near(stroke.dash[0], 8.0) && near(stroke.dash[1], 4.0));
    let end = drawn.subpaths[0].cubics.last().expect("segment").get(2).copied().expect("end");
    assert!(near(drawn.subpaths[0].start[0], 15.0) && near(drawn.subpaths[0].start[1], 84.0), "{:?}", drawn.subpaths[0].start);
    assert!(near(end[0], 15.0) && near(end[1], 84.0), "closed back to the start, {end:?}");
    let far = drawn.subpaths[0].cubics.first().expect("first").get(2).copied().expect("corner");
    assert!(near(far[0], 35.0) && near(far[1], 74.0), "{far:?}");

    assert!(paths.iter().any(|p| matches!(p.fill, Fill::Linear { .. })));
}

fn message(bytes: &[u8]) -> String {
    read(bytes, Limits::default()).unwrap_err().to_string()
}

#[test]
fn garbage_and_a_truncated_header_are_errors() {
    assert_eq!(message(b""), "the FreeHand file is empty");
    assert_eq!(message(b"A"), "the FreeHand file is cut off in the header");
    assert_eq!(message(b"AG"), "the FreeHand file is cut off in the header");
    assert_eq!(message(b"AGD"), "the FreeHand file is cut off in the header");
    assert_eq!(message(b"AGD4"), "the FreeHand file is cut off in the header");
    assert_eq!(message(b"not a file"), "not a FreeHand file");
    assert_eq!(message(b"XXXX"), "not a FreeHand file");
}

#[test]
fn an_unknown_record_type_is_refused() {
    let mut bytes = synth::demo();
    // The dictionary is after the zlib payload. Appending a new type without a record still
    // parses. A record whose name is unknown cannot be skipped: build one by renaming a type
    // the demo uses only as a string, which would break the file. Instead, feed AGD4 plus junk.
    bytes.extend_from_slice(b"unknown");
    let _ = read(&bytes, Limits::default());
    let hostile = b"AGD4\x00\x00\x00\x00\x00\x00\x00\x14not-zlib-at-all!!";
    assert!(read(hostile, Limits::default()).is_err());
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 32, failure_persistence: None, ..proptest::test_runner::Config::default() })]

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048)) {
        let _ = is_freehand(&bytes);
        let _ = read(&bytes, Limits::default());
    }

    #[test]
    fn mutated_demo_never_panics(
        edits in proptest::collection::vec((0usize..4000, proptest::prelude::any::<u8>()), 0..12),
    ) {
        let mut bytes = synth::demo();
        let n = bytes.len().max(1);
        for (at, b) in edits {
            if let Some(slot) = bytes.get_mut(at % n) {
                *slot = b;
            }
        }
        let _ = read(&bytes, Limits::default());
    }
}

fn group_depth(items: &[Item]) -> usize {
    items
        .iter()
        .map(|item| match item {
            Item::Group(kids) | Item::ClipGroup { children: kids, .. } => 1 + group_depth(kids),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

fn count_groups(items: &[Item]) -> usize {
    items
        .iter()
        .map(|item| match item {
            Item::Group(kids) | Item::ClipGroup { children: kids, .. } => 1 + count_groups(kids),
            _ => 0,
        })
        .sum()
}

#[test]
fn supported_versions_open_to_the_same_drawing() {
    let base = read(&synth::demo_at(9), Limits::default()).expect("v9");
    for version in [8u8, 10, 11] {
        let doc = read(&synth::demo_at(version), Limits::default()).unwrap_or_else(|e| panic!("v{version}: {e}"));
        assert_eq!(doc.pages, base.pages, "v{version} pages");
        assert_eq!(doc.layers.len(), base.layers.len());
        assert_eq!(doc.layers[0].name, base.layers[0].name);
        assert!(doc.layers[0].visible);
        let mut items = Vec::new();
        flat(&doc.layers[0].items, &mut items);
        let mut base_items = Vec::new();
        flat(&base.layers[0].items, &mut base_items);
        assert_eq!(items.len(), base_items.len(), "v{version}");
        assert!(doc.warnings.is_empty(), "v{version} {:?}", doc.warnings);
    }
}

#[test]
fn old_freehand_versions_are_refused() {
    let tail = "VectorCraft opens FreeHand 8 to MX (11); open it in a newer FreeHand and save, or export it as EPS, PDF or AI";
    for (magic, n) in
        [(&b"acf3"[..], "1"), (b"FHD2", "2"), (b"FH31", "3"), (b"AGD0", "5"), (b"AGD1", "4 or 5"), (b"AGD2", "7"), (b"ALDUS LZW compressed", "1")]
    {
        assert!(is_freehand(magic), "{n}");
        let err = message(magic);
        assert_eq!(err, format!("unsupported FreeHand version {n}: {tail}"), "{err}");
    }
    assert!(is_freehand(b"FreeHand10"));
    assert_eq!(message(b"FreeHand10............"), "the FreeHand file is cut off in the header");
    assert!(is_freehand(b"FreeHand11"));
    let mut wrapped = vec![0x1C, 0x08, 0x0A, 0x80, 0x04];
    let inner = synth::demo();
    wrapped.extend(u32::try_from(inner.len()).unwrap_or(0).to_be_bytes());
    wrapped.extend(inner);
    assert!(is_freehand(&wrapped));
    assert!(read(&wrapped, Limits::default()).is_ok());
    let mut old = vec![0x1C, 0x08, 0x0A, 0x00, 0x04];
    old.extend(b"AGD2");
    assert!(is_freehand(&old));
    assert!(message(&old).starts_with("unsupported FreeHand version 7:"));
}

#[test]
fn structural_damage_is_a_short_error() {
    assert_eq!(message(&b"AGD4\x00\x00\x00\x00\x00\x00\x00\x64junk"[..]), "the FreeHand file's data block runs past the end of the file");

    let mut damaged = b"AGD4\x00\x00\x00\x00".to_vec();
    let payload = b"this is not zlib";
    damaged.extend(u32::try_from(12 + payload.len()).unwrap_or(0).to_be_bytes());
    damaged.extend(payload);
    assert_eq!(message(&damaged), "the FreeHand file's compressed data is damaged");

    let whole = synth::envelope(9, &[0u8; 64], &[0, 0, 0, 0]);
    let data_len = u32::from_be_bytes(whole.get(8..12).and_then(|b| b.try_into().ok()).unwrap_or([0; 4]));
    let payload = usize::try_from(data_len).unwrap_or(12).saturating_sub(12);
    let mut torn = whole;
    let keep = 12 + payload / 2;
    torn.truncate(keep.max(14));
    let new_len = u32::try_from(torn.len()).unwrap_or(0);
    if let Some(slot) = torn.get_mut(8..12) {
        slot.copy_from_slice(&new_len.to_be_bytes());
    }
    assert_eq!(message(&torn), "the FreeHand file is cut off in its compressed data");

    assert_eq!(message(&synth::envelope(9, &[0, 0], &[0x00, 0x01])), "the FreeHand file is cut off in the dictionary");

    let mut dict = vec![0, 1, 0, 0, 0, 1];
    dict.extend(b"MName\0");
    assert_eq!(message(&synth::envelope(9, &[0], &dict)), "the FreeHand file is cut off in the record list");

    let mut listed = dict.clone();
    listed.extend(1u32.to_be_bytes());
    listed.extend(1u16.to_be_bytes());
    assert_eq!(message(&synth::envelope(9, &[0x00], &listed)), "the FreeHand file is cut off in a record");

    let mut huge = vec![0, 0, 0, 0];
    huge.extend(0x8000_0000u32.to_be_bytes());
    assert_eq!(message(&synth::envelope(9, &[0], &huge)), "the FreeHand file claims more records than can be read");

    assert_eq!(message(&synth::envelope(9, &[0], &[0xFF, 0xFF, 0x00, 0x00])), "the FreeHand file claims a dictionary larger than can be read");

    // An MList that claims 65535 entries and then stops.
    let mut names = vec![0, 1, 0, 0, 0, 1];
    names.extend(b"MList\0");
    names.extend(1u32.to_be_bytes());
    names.extend(1u16.to_be_bytes());
    let mut body = Vec::new();
    body.extend(65535u16.to_be_bytes());
    body.extend(65535u16.to_be_bytes());
    body.extend([0u8; 8]);
    assert_eq!(message(&synth::envelope(9, &body, &names)), "the FreeHand file claims more list entries than can be read");

    // A path that claims 65535 nodes.
    let mut names = vec![0, 1, 0, 0, 0, 1];
    names.extend(b"Path\0");
    names.extend(1u32.to_be_bytes());
    names.extend(1u16.to_be_bytes());
    let mut body = Vec::new();
    body.extend(65535u16.to_be_bytes());
    body.extend(0u16.to_be_bytes());
    body.extend(0u16.to_be_bytes());
    body.extend([0u8; 4]);
    body.extend([0u8; 9]);
    body.push(0);
    body.extend(65535u16.to_be_bytes());
    assert_eq!(message(&synth::envelope(9, &body, &names)), "the FreeHand file claims more path nodes than can be read");

    // 0xFFFF id escape for id 0x1FF00, far past a one-record file.
    let mut names = vec![0, 1, 0, 0, 0, 1];
    names.extend(b"MList\0");
    names.extend(1u32.to_be_bytes());
    names.extend(1u16.to_be_bytes());
    let mut body = Vec::new();
    body.extend(1u16.to_be_bytes());
    body.extend(1u16.to_be_bytes());
    body.extend([0u8; 8]);
    body.extend([0xFF, 0xFF, 0x00, 0x00]);
    assert_eq!(message(&synth::envelope(9, &body, &names)), "a FreeHand record id points past the end of the file");

    let err = read(&synth::demo(), Limits { max_decompressed: 32, ..Limits::default() }).unwrap_err().to_string();
    assert_eq!(err, "the FreeHand file decompresses past the safety limit");

    let err = read(&synth::demo(), Limits { max_input: 8, ..Limits::default() }).unwrap_err().to_string();
    assert_eq!(err, "the FreeHand file is larger than can be read");
}

#[test]
fn a_hidden_layer_keeps_its_path_and_stays_hidden() {
    let doc = read(&synth::hidden_layer(), Limits::default()).expect("hidden");
    assert_eq!(doc.layers.len(), 2);
    let hidden = doc.layers.iter().find(|l| l.name == "Guides").expect("guides");
    assert!(!hidden.visible && !hidden.locked);
    assert!(hidden.items.iter().any(|i| matches!(i, Item::Path(_))), "hidden layer keeps its path");
    let shown = doc.layers.iter().find(|l| l.name == "Ink").expect("ink");
    assert!(shown.visible);
    assert!(shown.items.iter().any(|i| matches!(i, Item::Path(_))));
    assert!(doc.warnings.iter().all(|w| !w.contains("left empty")), "{:?}", doc.warnings);
}

#[test]
fn a_content_fill_clips_the_referenced_drawing() {
    let doc = read(&synth::content_fill(), Limits::default()).expect("content");
    assert!(doc.warnings.iter().all(|w| !w.contains("content fills")), "{:?}", doc.warnings);
    let layer = doc.layers.first().expect("layer");
    let Item::ClipGroup { clip, children } = layer.items.first().expect("clip") else {
        panic!("expected a clip group, got {:?}", layer.items);
    };
    assert!(matches!(clip.fill, Fill::None));
    assert!(has_path(children), "content art is inside the clip");
}

fn has_path(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Path(_) => true,
        Item::Group(kids) | Item::ClipGroup { children: kids, .. } => has_path(kids),
        Item::Compound(_) | Item::Text(_) => false,
    })
}

#[test]
fn an_empty_group_is_kept_and_a_content_fill_is_only_noted_when_present() {
    let doc = read(&synth::empty_group_and_content_fill(), Limits::default()).expect("file");
    let layer = doc.layers.first().expect("layer");
    assert_eq!(count_groups(&layer.items), 1, "the empty group stays; the content drawing is a fill, not a child");
    let Item::Group(kids) = layer.items.first().expect("group") else { panic!("group") };
    assert!(kids.is_empty());
    let Item::Path(path) = layer.items.get(1).expect("path") else { panic!("path") };
    assert!(matches!(path.fill, Fill::Solid(_)), "the solid fill is the paint; a zero contents id adds nothing");
    assert!(doc.warnings.iter().any(|w| w.contains("content fills")), "{:?}", doc.warnings);
}

#[test]
fn deep_groups_a_cycle_and_a_symbol_cycle_stop() {
    let deep = read(&synth::nested_groups(200), Limits::default()).expect("deep");
    let deep_items = &deep.layers.first().expect("layer").items;
    assert!(group_depth(deep_items) <= Limits::default().max_depth);
    assert!(deep.warnings.iter().any(|w| w.contains("deeply nested")), "{:?}", deep.warnings);

    let cycle = read(&synth::cyclic_groups(), Limits::default()).expect("cycle");
    let cycle_items = &cycle.layers.first().expect("layer").items;
    assert!(count_groups(cycle_items) >= 1);
    assert!(cycle.warnings.iter().any(|w| w.contains("cyclic")), "{:?}", cycle.warnings);

    let symbol = read(&synth::cyclic_symbol(), Limits::default()).expect("symbol");
    assert!(symbol.warnings.iter().any(|w| w.contains("cyclic")), "{:?}", symbol.warnings);
}

/// Writes synthetic fixtures into `$FREEHAND_SEED_DIR` for a fuzz corpus. Does nothing when the
/// variable is unset. Not run by `cargo test`.
#[test]
#[ignore]
fn write_fuzz_seeds() {
    let Ok(dir) = std::env::var("FREEHAND_SEED_DIR") else { return };
    std::fs::create_dir_all(&dir).expect("seed dir");
    for version in 8u8..=11 {
        std::fs::write(format!("{dir}/demo-v{version}.fh"), synth::demo_at(version)).expect("write");
    }
    std::fs::write(format!("{dir}/hidden.fh"), synth::hidden_layer()).expect("write");
    std::fs::write(format!("{dir}/content.fh"), synth::content_fill()).expect("write");
    std::fs::write(format!("{dir}/empty-group.fh"), synth::empty_group_and_content_fill()).expect("write");
    std::fs::write(format!("{dir}/nested.fh"), synth::nested_groups(4)).expect("write");
}
