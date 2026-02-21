//! Web-specific data fetching: downloads the game JSON bundle from the CDN.
//! Shared model types live in `crate::model`.

use anyhow::{Context, Result, bail};

pub use crate::model::Root;

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
