//! The message tables: the set loaded at start-up and the tables screens load and remove.

use aerugo_ui::localize::{Localize, MessageTable, table_location};

use crate::assets::Assets;

/// Bundle key of the shared tables.
const COMMON: &str = "common";
/// Tables loaded at start-up, in lookup order.
const DEFAULT_TABLES: [&str; 14] = [
    "COMMON", "FLDTERM", "DEVIL", "RACE", "ITEM", "GUIDE", "AUTOMAP", "HEARTS", "SKILL", "ETC", "AISYO",
    "AREA", "BASYO", "FLOOR",
];
/// The platform's own table, loaded after the shared ones: `ETC` + the platform tag, from the `common` bundle
/// with the same tag.
const PLATFORM_TAG: &str = "STEAM";
const PLATFORM_TABLE: &str = "ETC";

/// Load table `filename` from the bundle named by `key` in language `lang` (once; failures are logged).
pub fn load(assets: &mut Assets, messages: &mut Localize, key: &str, filename: &str, lang: usize) {
    if messages.contains(filename) {
        return;
    }
    let (bundle, asset) = table_location(key, filename, lang);
    let table = assets.bundle(&bundle).and_then(|ar| MessageTable::load(ar, &asset, filename));
    match table {
        Ok(t) => messages.add(t),
        Err(e) => log::error!("message table {filename}: {e:#}"),
    }
}

/// The tables loaded at start-up.
pub fn load_default(assets: &mut Assets, lang: usize) -> Localize {
    let mut messages = Localize::default();
    for t in DEFAULT_TABLES {
        load(assets, &mut messages, COMMON, t, lang);
    }
    let key = format!("{COMMON}_{PLATFORM_TAG}");
    load(assets, &mut messages, &key, &format!("{PLATFORM_TABLE}_{PLATFORM_TAG}"), lang);
    messages
}
