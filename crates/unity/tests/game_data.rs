//! Tests against the user's game installation. They skip themselves when `AERUGO_GAME_DIR` is not set.

use std::path::PathBuf;

use aerugo_unity::{Bundle, ReadOpts, SerializedFile, TypeTreeRegistry, ValueReader};

fn game_dir() -> Option<PathBuf> {
    std::env::var_os("AERUGO_GAME_DIR").map(PathBuf::from)
}

/// The scene files are saved without type trees; the bundles' trees (same engine version) decode them.
#[test]
fn scene_objects_decode_with_trees_from_the_bundles() {
    let Some(game) = game_dir() else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let data_dir = game.join("smt3hd_Data");
    let mut reg = TypeTreeRegistry::new();
    for name in ["common", "battle", "title", "nameent", "staff"] {
        let data = std::fs::read(data_dir.join("StreamingAssets/PC").join(name)).expect("bundle");
        let b = Bundle::parse(&data).expect("parse bundle");
        for n in b.serialized_nodes() {
            reg.add_file(&SerializedFile::parse(b.node_data(n)).expect("parse serialized file"));
        }
    }
    assert!(!reg.is_empty());

    let scene = std::fs::read(data_dir.join("level1")).expect("scene file");
    let mut sf = SerializedFile::parse(&scene).expect("parse scene");
    reg.complete(&mut sf);

    let mut decoded = 0;
    let mut canvas_root = false;
    for o in &sf.objects {
        let Some(tree) = sf.tree(o) else { continue };
        let bytes = sf.object_bytes(o).unwrap();
        let mut vr = ValueReader::new(tree, bytes, sf.big_endian, ReadOpts::default());
        let v = vr.read_root().expect("value");
        assert_eq!(vr.consumed(), bytes.len(), "{} {} not fully read", sf.class_name(o), o.path_id);
        decoded += 1;
        if sf.class_name(o) == "GameObject" && v["m_Name"] == "Canvas_UI" {
            canvas_root = true;
        }
    }
    assert!(decoded > sf.objects.len() * 9 / 10, "only {decoded} of {} objects decoded", sf.objects.len());
    assert!(canvas_root, "the UI root canvas was not found");
}
