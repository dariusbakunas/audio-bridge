//! Media asset storage helpers (images, etc.).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use reqwest::Url;
use tokio::net::lookup_host;

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
        let parsed = validate_public_url(trimmed).await?;

        let resp = self
            .client
            .get(parsed)
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

async fn validate_public_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| anyhow!("url must be a valid http(s) url"))?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(anyhow!("url must use http or https")),
    }
    if url.username() != "" || url.password().is_some() {
        return Err(anyhow!("url must not include credentials"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("url host is required"))?
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".local") {
        return Err(anyhow!("url host is not allowed"));
    }
    if let Some(ip) = url.host().and_then(|h| h.to_string().parse().ok()) {
        if !is_public_ip(&ip) {
            return Err(anyhow!("url host is not public"));
        }
    }

    let port = url.port_or_known_default().unwrap_or(80);
    let addrs = lookup_host((host.as_str(), port))
        .await
        .context("resolve url host")?;
    for addr in addrs {
        if !is_public_ip(&addr.ip()) {
            return Err(anyhow!("url resolves to non-public address"));
        }
    }

    Ok(url)
}

fn is_public_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            if v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || is_v4_cgnat(v4)
                || is_v4_doc(v4)
                || is_v4_benchmark(v4)
                || is_v4_multicast(v4)
                || is_v4_reserved(v4)
            {
                return false;
            }
            true
        }
        std::net::IpAddr::V6(v6) => {
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || is_v6_unique_local(v6)
                || is_v6_link_local(v6)
                || is_v6_documentation(v6)
            {
                return false;
            }
            true
        }
    }
}

fn is_v4_cgnat(ip: &std::net::Ipv4Addr) -> bool {
    matches!(ip.octets(), [100, b, ..] if (64..=127).contains(&b))
}

fn is_v4_doc(ip: &std::net::Ipv4Addr) -> bool {
    matches!(ip.octets(), [192, 0, 2, _])
        || matches!(ip.octets(), [198, 51, 100, _])
        || matches!(ip.octets(), [203, 0, 113, _])
}

fn is_v4_benchmark(ip: &std::net::Ipv4Addr) -> bool {
    matches!(ip.octets(), [198, 18 | 19, _, _])
}

fn is_v4_multicast(ip: &std::net::Ipv4Addr) -> bool {
    ip.is_multicast()
}

fn is_v4_reserved(ip: &std::net::Ipv4Addr) -> bool {
    matches!(ip.octets(), [0, ..]) || (240..=255).contains(&ip.octets()[0])
}

fn is_v6_unique_local(ip: &std::net::Ipv6Addr) -> bool {
    matches!(ip.segments()[0] & 0xfe00, 0xfc00)
}

fn is_v6_link_local(ip: &std::net::Ipv6Addr) -> bool {
    matches!(ip.segments()[0] & 0xffc0, 0xfe80)
}

fn is_v6_documentation(ip: &std::net::Ipv6Addr) -> bool {
    ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8
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
