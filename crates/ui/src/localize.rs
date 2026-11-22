//! Localised text: the game's message tables and the lookup by message id.

use aerugo_unity::{Archive, ReadOpts};
use anyhow::{Context, Result};
use serde_json::Value;

/// Language tags, by language number.
pub const LANGUAGE_TAGS: [&str; 8] = ["JA", "EN", "FR", "IT", "DE", "ES", "ZH", "KO"];

/// Bundle and asset path of message table `filename` for bundle key `key` (e.g. "common", "CAMP") in language
/// `lang`. Bundle names are lower case on disk; asset lookups are case-insensitive.
pub fn table_location(key: &str, filename: &str, lang: usize) -> (String, String) {
    let appendix = format!("_{}", LANGUAGE_TAGS.get(lang).copied().unwrap_or("EN"));
    let bundle = format!("{}{appendix}", key.replace(&appendix, "")).to_lowercase();
    let tag = LANGUAGE_TAGS.get(lang).copied().unwrap_or("EN");
    let asset =
        format!("Assets/AssetBundle/AssetData/dds3Data/localize_text/{tag}/{filename}{appendix}.asset");
    (bundle, asset)
}

/// One message table: message id → text.
#[derive(Clone, Debug, Default)]
pub struct MessageTable {
    pub name: String,
    pub messages: Vec<(String, String)>,
}

impl MessageTable {
    /// Load the table at `asset_path` (case-insensitive) from `ar`, registered under `name`.
    pub fn load(ar: &Archive, asset_path: &str, name: &str) -> Result<Self> {
        let want = asset_path.to_lowercase();
        let (_, r) = ar
            .container
            .iter()
            .find(|(p, _)| p.to_lowercase() == want)
            .with_context(|| format!("no message table {asset_path}"))?;
        let (_, v, _) = ar.read(*r, ReadOpts::default())?;
        Self::from_value(&v, name)
    }

    fn from_value(v: &Value, name: &str) -> Result<Self> {
        let list = v["_eventEMsgData"].as_array().context("message table without messages")?;
        let mut messages = Vec::with_capacity(list.len());
        for m in list {
            let id = m["msgID"].as_str().context("message without id")?;
            let text = m["msgStr"].as_str().unwrap_or("");
            messages.push((id.to_string(), text.to_string()));
        }
        Ok(Self { name: name.to_string(), messages })
    }

    pub fn get(&self, id: &str) -> Option<&str> {
        self.messages.iter().find(|(k, _)| k == id).map(|(_, v)| v.as_str())
    }
}

/// The loaded message tables, searched in load order.
#[derive(Clone, Debug, Default)]
pub struct Localize {
    tables: Vec<MessageTable>,
}

impl Localize {
    pub fn contains(&self, name: &str) -> bool {
        self.tables.iter().any(|t| t.name == name)
    }

    /// Add a table (a table already loaded under the same name is kept).
    pub fn add(&mut self, table: MessageTable) {
        if !self.contains(&table.name) {
            self.tables.push(table);
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.tables.retain(|t| t.name != name);
    }

    /// The text of message `id` (angle brackets ignored); unknown ids give the id itself, "nulldata" a space.
    pub fn text(&self, id: &str) -> String {
        if id.is_empty() {
            return String::new();
        }
        let key = id.replace(['<', '>'], "");
        for t in &self.tables {
            if let Some(s) = t.get(&key) {
                return s.to_string();
            }
        }
        if key == "nulldata" { " ".to_string() } else { key }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_and_lookup() {
        assert_eq!(
            table_location("common", "CAMP", 1),
            (
                "common_en".to_string(),
                "Assets/AssetBundle/AssetData/dds3Data/localize_text/EN/CAMP_EN.asset".to_string()
            )
        );
        let mut l = Localize::default();
        l.add(MessageTable { name: "CAMP".into(), messages: vec![("CAMP_L0007".into(), "STATS".into())] });
        assert_eq!(l.text("<CAMP_L0007>"), "STATS");
        assert_eq!(l.text("missing"), "missing");
        assert_eq!(l.text("nulldata"), " ");
        assert_eq!(l.text(""), "");
    }
}
