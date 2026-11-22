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
    let quads = ui.quads();
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

#[test]
fn message_tables_and_fonts_load() {
    use aerugo_ui::font::FontAsset;
    use aerugo_ui::localize::{Localize, MessageTable, table_location};
    let (bundle_name, asset) = table_location("common", "CAMP", 1);
    let Some(ar) = bundle(&bundle_name) else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let mut l = Localize::default();
    l.add(MessageTable::load(&ar, &asset, "CAMP").unwrap());
    assert_eq!(l.text("CAMP_L0005"), "COMMAND MENU");
    let fonts = bundle("font-en").unwrap();
    let (_, r) = fonts.container.iter().find(|(p, _)| p.ends_with("fot-matissepron-b sdf.asset")).unwrap();
    // The container lists the asset's objects; the font asset is the one with glyphs.
    let font = fonts
        .container
        .iter()
        .filter(|(p, _)| p.ends_with("fot-matissepron-b sdf.asset"))
        .find_map(|(_, r)| FontAsset::load(&fonts, *r).ok())
        .unwrap_or_else(|| panic!("no font asset at {r:?}"));
    assert_eq!(font.face.line_height, 170.0);
    assert_eq!(font.atlas_size, (8192, 8192));
    let (g, _) = font.glyph_of('A' as u32).unwrap();
    assert!(g.advance > 0.0 && g.rect[2] > 0);
    assert_eq!(font.atlases.len(), 1);
}

#[test]
fn title_menu_text_lays_out_with_the_english_font() {
    use aerugo_ui::font::FontAsset;
    use aerugo_ui::material::TextMaterial;
    use aerugo_ui::text::{MaterialMetrics, align, layout};
    let Some(ar) = bundle("common") else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let p = Prefab::load(&ar, "common", "Assets/AssetBundle/AssetData/dds3data/UI/titleUI.prefab").unwrap();
    let mut ui = UiTree::new("Canvas_UI", Canvas::DESIGN);
    let root = ui.instantiate(&p, ui.root());
    let item = ui.find(root, "titlemain/titlemainmenu/stretch/titlemain_item01/slide/Text_TM").unwrap();
    let t = ui.text(item).unwrap().clone();
    assert!(t.change_font && t.enabled);
    assert_eq!(t.style.alignment, align::MIDDLE | align::LEFT);
    assert_eq!(t.style.font_size, 42.0);

    let fonts = bundle("font-en").unwrap();
    let font = fonts
        .container
        .iter()
        .filter(|(p, _)| p.ends_with("fot-matissepron-b sdf.asset"))
        .find_map(|(_, r)| FontAsset::load(&fonts, *r).ok())
        .unwrap();
    let mut mat = TextMaterial::load(&fonts, font.material.unwrap()).unwrap();
    assert!(mat.has_keyword("UNDERLAY_ON") && mat.is_sdf());
    let m = MaterialMetrics::of(&mut mat, false);
    // Dilate 0.1 and underlay 0.4 at the ratios 10/11 and 7.125/11, times the gradient scale 11, plus 1.25.
    assert!((m.padding - ((0.1 * 10.0 + 0.4 * 7.125) + 1.25)).abs() < 1e-4, "padding {}", m.padding);
    let rect = ui.rect(item);
    let quads = layout("NEW GAME", &font, m, &|_| None, &t.style, rect, 1.0);
    // Seven visible glyphs (the space has none), on one line, left to right.
    assert_eq!(quads.len(), 7);
    assert!(quads.windows(2).all(|w| w[1].pos[0][0] > w[0].pos[0][0]));
    assert!(quads.iter().all(|q| (q.pos[0][1] - quads[0].pos[0][1]).abs() < 30.0));
    assert!((quads[0].scale - 42.0 / 85.0).abs() < 1e-6);
}

#[test]
fn hidden_menu_items_close_up_in_the_layout() {
    let Some(ar) = bundle("common") else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let p = Prefab::load(&ar, "common", "Assets/AssetBundle/AssetData/dds3data/UI/titleUI.prefab").unwrap();
    let mut ui = UiTree::new("Canvas_UI", Canvas::DESIGN);
    let root = ui.instantiate(&p, ui.root());
    let main = ui.find(root, "titlemain").unwrap();
    assert!(ui.node(main).animator.is_some(), "the menu's intro animation loads");
    ui.set_active(main, true);
    let item = |ui: &UiTree, n: u32| {
        ui.find(root, &format!("titlemain/titlemainmenu/stretch/titlemain_item{n:02}")).unwrap()
    };
    for n in [3, 4] {
        let i = item(&ui, n);
        ui.set_active(i, false);
    }
    ui.rebuild_layout();
    // Stacked 70 apart from the top of `stretch`: items 01, 02, then 05 in the third row.
    assert_eq!(ui.node(item(&ui, 5)).transform.anchored_position, [0.0, -140.0]);
    let stretch = ui.find(root, "titlemain/titlemainmenu/stretch").unwrap();
    assert_eq!(ui.node(stretch).transform.size_delta[1], 8.0 * 70.0);
    let bottom = ui.find(root, "titlemain/titlemainmenu/titlemain_bottom").unwrap();
    assert_eq!(ui.node(bottom).transform.anchored_position[1], -(114.0 + 8.0 * 70.0));
}

#[test]
fn menu_choice_window_rests_hidden() {
    let Some(ar) = bundle("common") else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let p = Prefab::load(&ar, "common", "Assets/AssetBundle/AssetData/dds3data/UI/titleUI.prefab").unwrap();
    let mut ui = UiTree::new("Canvas_UI", Canvas::DESIGN);
    let root = ui.instantiate(&p, ui.root());
    let menu = ui.find(root, "titlemain").unwrap();
    ui.set_active(menu, true);
    ui.advance_animators(1.0 / 30.0);
    // The choice window's resting state animates its own object: moved aside and fully transparent.
    let choice = ui.find(root, "titlemain/talk01_choice").unwrap();
    assert_eq!(ui.node(choice).canvas_group.as_ref().unwrap().alpha, 0.0);
    assert_eq!(ui.node(choice).transform.anchored_position[0], 1500.0);
}
