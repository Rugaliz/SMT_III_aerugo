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

#[test]
fn press_start_animation_drives_its_sprites() {
    let Some(ar) = bundle("common") else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let p = Prefab::load(&ar, "common", "Assets/AssetBundle/AssetData/dds3data/UI/titleUI.prefab").unwrap();
    let mut ui = UiTree::new("Canvas_UI", Canvas::DESIGN);
    let root = ui.instantiate(&p, ui.root());
    // The title group starts inactive; the title shows it when it loads.
    let title = ui.find(root, "title").expect("title node");
    ui.set_active(title, true);
    let press = ui.find(root, "title/title_press").expect("press node");
    let alpha =
        |ui: &UiTree, path: &str| ui.node(ui.find(press, path).unwrap()).image.as_ref().unwrap().color.a;
    assert!(ui.animator_enabled(press));
    // The 2-second loop: base and text pulse 0.5 -> 1 -> 0.5; the glow rises to 1 at 1.17 s and fades out.
    ui.animator_play(press, "title_press", 0.0);
    ui.advance_animators(0.0);
    assert_eq!(alpha(&ui, "title_press_base"), 0.5);
    assert_eq!(alpha(&ui, "titletex_pressg"), 0.0);
    assert_eq!(alpha(&ui, "title_press_base02"), 0.0);
    ui.advance_animators(1.0);
    assert_eq!(alpha(&ui, "title_press_base"), 1.0);
    assert_eq!(alpha(&ui, "titletex_press"), 1.0);
    ui.advance_animators(0.5);
    assert!(alpha(&ui, "titletex_pressg") < 1.0);
    // Disabled and enabled again without keeping its state: it starts over.
    ui.set_animator_enabled(press, false);
    ui.set_animator_enabled(press, true);
    ui.advance_animators(0.0);
    assert_eq!(alpha(&ui, "title_press_base"), 0.5);
}
