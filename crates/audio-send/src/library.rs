//! Scan local directories for playable files (MVP: non-recursive `.flac`/`.wav`).

use std::ffi::OsStr;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Clone, Debug)]
pub struct Track {
    pub path: PathBuf,
    pub file_name: String,
    pub ext_hint: String,
    pub duration_ms: Option<u64>,
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

        let duration_ms = probe_duration_ms(&path, &ext);

        out.push(Track {
            path,
            file_name,
            ext_hint: ext,
            duration_ms,
        });
    }

    out.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
    Ok(out)
}

fn probe_duration_ms(path: &Path, ext_hint: &str) -> Option<u64> {
    let file = File::open(path).ok()?;
    let mut hint = Hint::new();
    if !ext_hint.is_empty() {
        hint.with_extension(ext_hint);
    }

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;

    let track = probed.format.default_track()?;
    let params = &track.codec_params;
    let frames = params.n_frames?;
    let rate = params.sample_rate? as u64;
    if rate == 0 {
        return None;
    }
    Some(frames.saturating_mul(1000) / rate)
}
