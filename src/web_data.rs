use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// Core metadata for a game build, flattened from various JSON sources.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct BuildInfo {
    /// The unique build identifier (e.g., "2024-01-01" or "v0.9.1").
    pub build_number: String,
    /// The human-readable tag name.
    pub tag_name: String,
    /// Whether this is a prerelease/nightly build.
    pub prerelease: bool,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

/// Represents an indexed item holding its original value and resolved primary fields.
#[derive(Debug, Clone)]
pub struct IndexedItem {
    /// The actual JSON data of the item.
    pub value: Value,
    /// The resolved string ID of the item.
    pub id: String,
    /// The resolved type string of the item.
    pub item_type: String,
}

/// The root structure of the game data JSON (`all.json`).
#[derive(Debug, Deserialize)]
pub struct Root {
    /// Flattened build metadata.
    #[serde(flatten)]
    pub build: BuildInfo,
    /// The actual game data items.
    pub data: Vec<Value>,
}

impl<'de> Deserialize<'de> for BuildInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Proxy {
            build_number: String,
            prerelease: Option<bool>,
            created_at: Option<String>,
            release: Option<Value>,
        }

        let proxy = Proxy::deserialize(deserializer)?;

        let mut tag_name = proxy.build_number.clone();
        let mut prerelease = proxy.prerelease.unwrap_or(false);
        let mut created_at = proxy.created_at.unwrap_or_default();

        if let Some(release) = proxy.release {
            if let Some(tag) = release.get("tag_name").and_then(|v| v.as_str()) {
                tag_name = tag.to_string();
            }
            if let Some(pre) = release.get("prerelease").and_then(|v| v.as_bool()) {
                prerelease = pre;
            }
            if let Some(created) = release.get("created_at").and_then(|v| v.as_str()) {
                created_at = created.to_string();
            }
        }

        Ok(BuildInfo {
            build_number: proxy.build_number,
            tag_name,
            prerelease,
            created_at,
        })
    }
}

pub async fn fetch_game_root(version: &str) -> Result<Root> {
    let url = format!(
        "https://data.cataclysmbn-guide.com/data/{}/all.json",
        version
    );

    let response = reqwest::get(&url).await?;
    if !response.status().is_success() {
        bail!("failed to download {}: HTTP {}", url, response.status());
    }

    let text = response.text().await?;
    let root: Root = serde_json::from_str(&text).context("failed to parse all.json")?;
    Ok(root)
}
