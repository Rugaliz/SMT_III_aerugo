//! Tests against the user's game installation. They skip themselves when `AERUGO_GAME_DIR` is not set.

use std::path::PathBuf;

use aerugo_ui::{Canvas, Prefab, UiTree};
use aerugo_unity::Archive;

fn bundle(name: &str) -> Option<Archive> {
    let game = PathBuf::from(std::env::var_os("AERUGO_GAME_DIR")?);
    Some(Archive::open(&game.join("smt3hd_Data/StreamingAssets/PC").join(name)).expect("open bundle"))
}

#[test]
fn atlus_logo_fills_the_canvas() {
    let Some(ar) = bundle("title") else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let p =
        Prefab::load(&ar, "title", "Assets/AssetBundle/AssetData/dds3data/title/AtlusLogoUI.prefab").unwrap();
    assert_eq!(p.nodes[0].name, "AtlusLogoUI");
    let mut ui = UiTree::new("Canvas_UI", Canvas::DESIGN);
    let root = ui.instantiate(&p, ui.root());
    let logo = ui.find(root, "atluslogo").expect("logo node");
    assert!(!ui.active_self(logo), "the logo starts hidden");
    ui.set_active(logo, true);
    let quads = ui.draw_list();
    assert_eq!(quads.len(), 1);
    assert_eq!(quads[0].pos, [[-960.0, -540.0], [-960.0, 540.0], [960.0, -540.0], [960.0, 540.0]]);
    let tex = ar.texture2d(quads[0].texture.as_ref().unwrap().obj).unwrap();
    assert_eq!((tex.width, tex.height), (1920, 1080));
}

#[test]
fn title_menu_prefab_loads() {
    let Some(ar) = bundle("common") else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let p = Prefab::load(&ar, "common", "Assets/AssetBundle/AssetData/dds3data/UI/titleUI.prefab").unwrap();
    let mut ui = UiTree::new("Canvas_UI", Canvas::DESIGN);
    let root = ui.instantiate(&p, ui.root());
    for path in [
        "title",
        "title/title_logo",
        "title/title_press/titletex_press",
        "WarningState",
        "license_infometion",
    ] {
        assert!(ui.find(root, path).is_some(), "{path} missing");
    }
}
