//! Media asset storage helpers (images, etc.).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use reqwest::Client;

const ASSETS_DIR: &str = ".audio-hub/assets";
const MAX_IMAGE_BYTES: usize = 6_000_000;

pub struct StoredAsset {
    pub local_path: String,
    pub checksum: String,
    pub source_url: String,
    pub updated_at_ms: i64,
}

pub struct MediaAssetStore {
    root: PathBuf,
    client: Client,
}

impl MediaAssetStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            client: Client::new(),
        }
    }

    pub fn assets_root(&self) -> PathBuf {
        self.root.join(ASSETS_DIR)
    }

    pub async fn store_image_from_url(
        &self,
        owner_type: &str,
        owner_id: i64,
        kind: &str,
        url: &str,
    ) -> Result<StoredAsset> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("url is required"));
        }
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return Err(anyhow!("url must start with http:// or https://"));
        }

        let resp = self
            .client
            .get(trimmed)
            .send()
            .await
            .context("fetch image")?;
        if !resp.status().is_success() {
            return Err(anyhow!("image fetch failed with status {}", resp.status()));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let ext = extension_for_mime(&content_type)
            .ok_or_else(|| anyhow!("unsupported image content-type"))?;

        let bytes = resp.bytes().await.context("read image bytes")?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(anyhow!("image exceeds {} bytes", MAX_IMAGE_BYTES));
        }

        let checksum = hash_bytes(&bytes);
        let relative = PathBuf::from(ASSETS_DIR)
            .join(owner_type)
            .join(owner_id.to_string())
            .join(format!("{}-{}.{}", kind, checksum, ext));
        let full_path = self.root.join(&relative);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create assets dir {:?}", parent))?;
        }
        if !full_path.exists() {
            std::fs::write(&full_path, &bytes)
                .with_context(|| format!("write asset {:?}", full_path))?;
        }

        Ok(StoredAsset {
            local_path: relative.to_string_lossy().to_string(),
            checksum,
            source_url: trimmed.to_string(),
            updated_at_ms: now_ms(),
        })
    }

    pub fn resolve_asset_path(&self, local_path: &str) -> Result<PathBuf> {
        let full_path = self.root.join(local_path);
        let full_path = full_path
            .canonicalize()
            .with_context(|| format!("canonicalize asset {:?}", full_path))?;
        let assets_root = self
            .assets_root()
            .canonicalize()
            .with_context(|| "canonicalize assets root")?;
        if !full_path.starts_with(&assets_root) {
            return Err(anyhow!("asset path outside assets root"));
        }
        Ok(full_path)
    }

    pub fn delete_asset_file(&self, local_path: &str) -> Result<()> {
        let path = Path::new(local_path);
        if path.is_absolute() {
            return Err(anyhow!("asset path must be relative"));
        }
        if path.components().any(|component| matches!(component, std::path::Component::ParentDir)) {
            return Err(anyhow!("asset path must not contain parent segments"));
        }
        let full_path = self.root.join(path);
        let assets_root = self.assets_root();
        if !full_path.starts_with(&assets_root) {
            return Err(anyhow!("asset path outside assets root"));
        }
        if full_path.exists() {
            std::fs::remove_file(&full_path)
                .with_context(|| format!("remove asset {:?}", full_path))?;
        }
        Ok(())
    }
}

fn extension_for_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_bytes_is_stable() {
        let first = hash_bytes(b"test");
        let second = hash_bytes(b"test");
        assert_eq!(first, second);
    }

    #[test]
    fn extension_for_mime_handles_common_types() {
        assert_eq!(extension_for_mime("image/jpeg"), Some("jpg"));
        assert_eq!(extension_for_mime("image/png"), Some("png"));
        assert_eq!(extension_for_mime("image/webp"), Some("webp"));
        assert_eq!(extension_for_mime("image/gif"), None);
    }
}
