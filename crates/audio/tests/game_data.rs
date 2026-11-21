//! Tests against the user's game installation. They skip themselves when `AERUGO_GAME_DIR` is not set.

use std::path::PathBuf;

use aerugo_audio::{Bank, SetupHeaders, decode};
use aerugo_unity::Bundle;

#[test]
fn sound_effects_bank_decodes_to_its_stated_length() {
    let Some(game) = std::env::var_os("AERUGO_GAME_DIR").map(PathBuf::from) else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let headers = SetupHeaders::load(&game.join("UnityPlayer.dll")).unwrap();
    assert!(!headers.is_empty());
    let data = std::fs::read(game.join("smt3hd_Data/StreamingAssets/PC/skillse")).unwrap();
    let bundle = Bundle::parse(&data).unwrap();
    let node = bundle.nodes.iter().find(|n| n.path.ends_with(".resource")).unwrap();
    let d = bundle.node_data(node);
    let mut decoded = 0;
    let mut pos = 0;
    while let Some(i) = d[pos..].windows(4).position(|w| w == b"FSB5") {
        let bank = Bank::parse(&d[pos + i..]).unwrap();
        for s in &bank.samples {
            let pcm = decode(&bank, s, &headers).unwrap();
            assert_eq!(pcm.frames() as u64, s.frames);
            assert!(pcm.pcm.iter().any(|v| v.abs() > 1e-4), "silent sample");
            decoded += 1;
        }
        pos += i + 4;
    }
    assert_eq!(decoded, 256);
}
