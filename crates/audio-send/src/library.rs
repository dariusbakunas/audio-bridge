//! Scan local directories for playable files (MVP: non-recursive `.flac`/`.wav`).

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct Track {
    pub path: PathBuf,
    pub file_name: String,
    pub ext_hint: String,
}

pub fn list_tracks(dir: &Path) -> Result<Vec<Track>> {
    let mut out = Vec::new();

    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {:?}", dir))? {
        let entry = entry.context("read_dir entry")?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .to_ascii_lowercase();

        if ext != "flac" && ext != "wav" {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("<unknown>")
            .to_string();

        out.push(Track {
            path,
            file_name,
            ext_hint: ext,
        });
    }

    out.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
    Ok(out)
}