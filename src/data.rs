use anyhow::Result;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::fs;
use std::io::{self, Read, Write};
use std::time::Duration;

/// Core metadata for a game build, flattened from various JSON sources.
#[derive(Debug, Clone)]
pub struct BuildInfo {
    /// The unique build identifier (e.g., "2024-01-01" or "v0.9.1").
    pub build_number: String,
    /// The human-readable tag name (often matches build_number or is more descriptive).
    pub tag_name: String,
    /// Whether this is a prerelease/nightly build.
    pub prerelease: bool,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
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

#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

impl<'de> Deserialize<'de> for BuildInfo {
    /// Custom deserializer to flatten the potential nesting of `release.tag_name`
    /// from Github-style JSON responses into a flat domain model.
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

        // Extract flattened fields from the optional nested `release` object
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

pub fn get_cache_dir() -> Result<std::path::PathBuf> {
    let project_dirs = directories::ProjectDirs::from("com", "cataclysmbn", "cbn-tui")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
    let cache_dir = project_dirs.cache_dir().to_path_buf();
    fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir)
}

pub fn get_data_dir() -> Result<std::path::PathBuf> {
    let project_dirs = directories::ProjectDirs::from("com", "cataclysmbn", "cbn-tui")
        .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?;
    let data_dir = project_dirs.data_dir().to_path_buf();
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir)
}

pub fn fetch_builds(force: bool) -> Result<Vec<BuildInfo>> {
    fetch_builds_with_progress(force, |_| {})
}

pub fn fetch_builds_with_progress<F>(force: bool, mut on_progress: F) -> Result<Vec<BuildInfo>>
where
    F: FnMut(DownloadProgress),
{
    let cache_dir = get_cache_dir()?;
    let builds_path = cache_dir.join("builds.json");

    let mut should_download = force || !builds_path.exists();
    if !should_download
        && let Ok(metadata) = fs::metadata(&builds_path)
        && let Ok(modified) = metadata.modified()
        && let Ok(elapsed) = modified.elapsed()
        && elapsed.as_secs() > 3600
    {
        should_download = true;
    }

    let content = if should_download {
        let client = http_client()?;
        let url = "https://data.cataclysmbn-guide.com/builds.json";
        download_to_path(&client, url, &builds_path, Some(&mut on_progress))?;
        fs::read_to_string(&builds_path)?
    } else {
        on_progress(DownloadProgress {
            downloaded: 1,
            total: Some(1),
        });
        fs::read_to_string(&builds_path)?
    };

    let mut builds: Vec<BuildInfo> = serde_json::from_str(&content)?;
    builds.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(builds)
}

pub fn fetch_game_data_with_progress<F>(
    version: &str,
    force: bool,
    mut on_progress: F,
) -> Result<std::path::PathBuf>
where
    F: FnMut(DownloadProgress),
{
    let cache_dir = get_cache_dir()?;
    let version_cache_dir = cache_dir.join(version);
    fs::create_dir_all(&version_cache_dir)?;

    let target_path = version_cache_dir.join("all.json");

    let mut should_download = force || !target_path.exists();
    let expiration = match version {
        "nightly" => Some(Duration::from_secs(12 * 3600)),
        "stable" => Some(Duration::from_secs(30 * 24 * 3600)),
        _ => None,
    };

    if !should_download
        && let Some(exp) = expiration
        && let Ok(metadata) = fs::metadata(&target_path)
        && let Ok(modified) = metadata.modified()
        && modified
            .elapsed()
            .map(|elapsed| elapsed > exp)
            .unwrap_or(false)
    {
        should_download = true;
    }

    if should_download {
        let client = http_client()?;
        let url = format!(
            "https://data.cataclysmbn-guide.com/data/{}/all.json",
            version
        );
        download_to_path(&client, &url, &target_path, Some(&mut on_progress))?;
    } else {
        on_progress(DownloadProgress {
            downloaded: 1,
            total: Some(1),
        });
    }

    Ok(target_path)
}

fn download_to_path(
    client: &reqwest::blocking::Client,
    url: &str,
    path: &std::path::Path,
    mut on_progress: Option<&mut dyn FnMut(DownloadProgress)>,
) -> Result<()> {
    let mut response = client.get(url).send()?;
    if !response.status().is_success() {
        anyhow::bail!("Failed to download {}: {}", url, response.status());
    }
    let total = response.content_length();
    let mut file = fs::File::create(path)?;
    let mut downloaded = 0u64;
    let mut buffer = [0u8; 65536];

    if let Some(cb) = on_progress.as_deref_mut() {
        cb(DownloadProgress {
            downloaded,
            total,
        });
    }

    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        downloaded += read as u64;
        if let Some(cb) = on_progress.as_deref_mut() {
            cb(DownloadProgress {
                downloaded,
                total,
            });
        }
    }

    Ok(())
}

fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder().build()?)
}

pub fn load_root(file_path: &str) -> Result<Root> {
    if !std::path::Path::new(file_path).exists() {
        if file_path == "all.json" {
            anyhow::bail!(
                "Default 'all.json' not found in current directory. Use --file or --game to specify data source."
            );
        } else {
            anyhow::bail!("File not found: {}", file_path);
        }
    }
    let file = fs::File::open(file_path)?;
    let reader = io::BufReader::new(file);
    let root: Root = serde_json::from_reader(reader)?;
    Ok(root)
}
