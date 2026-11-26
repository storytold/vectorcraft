//! Ruler guides in `.vectorcraft` files of every version: the saved `guides` list becomes ruler
//! guide nodes on their layers when a file opens, and is written back as the same list.

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_doc::{Document, LayerColor, Node, NodeId};
use vectorcraft_format::{MIN_VERSION, SaveOptions, VERSION, load, save_with};
use vectorcraft_testkit::fixtures;

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

fn save_v(d: &Document, version: u32) -> Vec<u8> {
    save_with(d, &SaveOptions { version, ..SaveOptions::default() }).unwrap()
}

/// The document of a saved file without its guide list and its next id (giving guides ids
/// raises the next id).
fn without_guides(v: &Value) -> Value {
    let mut d = v["document"].clone();
    let o = d.as_object_mut().unwrap();
    o.remove("guides");
    o.remove("next_id");
    d
}

/// A document with art on two layers, a sublayer in the bottom one, and two artboards:
/// (document, bottom layer, sublayer, top layer, second artboard's id).
fn doc() -> (Document, NodeId, NodeId, NodeId, u32) {
    let mut d = (*fixtures::rich_session().doc().unwrap().doc).clone();
    let bottom = d.layers[0].id;
    let sub = d.alloc_id();
    d.insert(Some(bottom), usize::MAX, Node::layer(sub, "Sub", LayerColor::Preset(1))).unwrap();
    let top = d.add_layer(Some("Top"));
    let mut ab = d.artboards[0].clone();
    ab.id = d.next_artboard_id();
    d.artboards.push(ab);
    let board = d.artboards[1].id;
    (d, bottom, sub, top, board)
}

/// Each version's file with a guide on no layer, one on a sublayer, one on a layer that is gone,
/// and an artboard guide, as files written before ruler guides were nodes hold them: it opens
/// with the guides on their layers, saves the same lines with their layers and ids, and saving
/// what that file opens to gives the same `guides` list again. The rest of the document is saved
/// as it was.
#[test]
fn every_version_reads_and_writes_back_the_same_guides() {
    let (d, bottom, sub, top, board) = doc();
    for version in MIN_VERSION..=VERSION {
        let mut v = json_of(&save_v(&d, version));
        assert_eq!(v["version"], version);
        let plain = without_guides(&v);
        v["document"]["guides"] = json!([
            {"vertical": true, "pos": 12.5},
            {"vertical": false, "pos": 40.0, "layer": sub},
            {"vertical": true, "pos": 70.0, "layer": 99_999},
            {"vertical": false, "pos": 90.0, "layer": bottom, "artboard": board},
        ]);
        let opened = load(&serde_json::to_vec(&v).unwrap()).unwrap();
        let guides = opened.ruler_guides();
        let lines: Vec<_> = guides.iter().map(|g| (g.vertical, g.pos, g.artboard, g.layer)).collect();
        // On no layer and on a deleted layer: the top visible, unlocked layer.
        assert_eq!(
            lines,
            [(true, 12.5, None, Some(top)), (false, 40.0, None, Some(sub)), (true, 70.0, None, Some(top)), (false, 90.0, Some(board), Some(bottom))],
            "version {version}"
        );
        for g in &guides {
            let n = opened.node(g.id.unwrap()).unwrap();
            assert!(n.is_ruler_guide() && n.visible && !n.locked && n.name.is_none(), "version {version}");
            // Each sits at the top of its layer, above the art.
            let kids = opened.node(g.layer.unwrap()).unwrap().children().unwrap();
            let first_guide = kids.iter().position(|c| c.is_ruler_guide()).unwrap();
            assert!(kids[first_guide..].iter().all(|c| c.is_ruler_guide()), "version {version}");
        }

        let once = json_of(&save_v(&opened, version));
        assert_eq!(once["version"], version);
        let saved: Vec<_> = once["document"]["guides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| (g["vertical"].clone(), g["pos"].clone(), g.get("artboard").cloned(), g["layer"].clone(), g["id"].is_u64()))
            .collect();
        assert_eq!(
            saved,
            [
                (json!(true), json!(12.5), None, json!(top), true),
                (json!(false), json!(40.0), None, json!(sub), true),
                (json!(true), json!(70.0), None, json!(top), true),
                (json!(false), json!(90.0), Some(json!(board)), json!(bottom), true),
            ],
            "version {version}"
        );
        assert_eq!(without_guides(&once), plain, "version {version}: the rest of the document");

        let reopened = load(&serde_json::to_vec(&once).unwrap()).unwrap();
        assert_eq!(reopened.ruler_guides(), guides, "version {version}: same ids, layers and lines");
        let twice = json_of(&save_v(&reopened, version));
        assert_eq!(twice["document"]["guides"], once["document"]["guides"], "version {version}");
    }
}

/// The guide entry of a build from before ruler guides were nodes, field for field.
#[derive(Debug, PartialEq, Deserialize)]
struct OldGuide {
    vertical: bool,
    pos: f64,
    #[serde(default)]
    artboard: Option<u32>,
    #[serde(default)]
    layer: Option<NodeId>,
}

/// An older build reads the guides a file now saves, the new fields (id, name, lock, visibility)
/// included, as the same lines on the same layers.
#[test]
fn an_older_guide_entry_reads_the_saved_guides() {
    let (mut d, bottom, sub, _, board) = doc();
    let mut g = vectorcraft_doc::Guide::new(true, 15.0);
    g.layer = Some(sub);
    g.name = Some("Fold".into());
    g.locked = true;
    g.hidden = true;
    d.add_ruler_guide(&g);
    d.add_ruler_guide(&vectorcraft_doc::Guide { layer: Some(bottom), artboard: Some(board), ..vectorcraft_doc::Guide::new(false, 25.0) });
    for version in MIN_VERSION..=VERSION {
        let v = json_of(&save_v(&d, version));
        let entries = v["document"]["guides"].as_array().unwrap();
        assert_eq!(entries[0]["name"], "Fold", "the new fields are written");
        let old: Vec<OldGuide> = serde_json::from_value(v["document"]["guides"].clone()).unwrap();
        assert_eq!(
            old,
            [
                OldGuide { vertical: true, pos: 15.0, artboard: None, layer: Some(sub) },
                OldGuide { vertical: false, pos: 25.0, artboard: Some(board), layer: Some(bottom) },
            ],
            "version {version}"
        );
        // And this build reads its own lock, visibility and name back.
        let back = load(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(back.ruler_guides(), d.ruler_guides(), "version {version}");
    }
}
