use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use lofty::{Accessor, AudioFile, ItemKey, ItemValue, Tag, TagType, TaggedFileExt, read_from_path};

const STANDARD_VORBIS_KEYS: &[&str] = &[
    "TITLE",
    "ARTIST",
    "ALBUM",
    "ALBUMARTIST",
    "YEAR",
    "DATE",
    "TRACKNUMBER",
    "DISCNUMBER",
];

const STANDARD_TRACK_FIELDS: &[&str] = &[
    "title",
    "artist",
    "album",
    "album_artist",
    "year",
    "track_number",
    "disc_number",
];

/// Mutable track-tag update payload used by metadata write endpoints.
pub struct TrackTagUpdate<'a> {
    pub title: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
    pub album_artist: Option<&'a str>,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub extra_tags: Option<&'a BTreeMap<String, String>>,
    pub clear_title: bool,
    pub clear_artist: bool,
    pub clear_album: bool,
    pub clear_album_artist: bool,
    pub clear_year: bool,
    pub clear_track_number: bool,
    pub clear_disc_number: bool,
    pub clear_extra_tags: Option<&'a HashSet<String>>,
}

/// Write selected metadata fields into track tags using lofty.
pub fn write_track_tags(path: &Path, update: TrackTagUpdate<'_>) -> Result<()> {
    let mut tagged_file = read_from_path(path).context("read tags")?;
    let mut tag_type = tagged_file.primary_tag_type();
    if tagged_file.tag(tag_type).is_none() {
        if let Some(tag) = tagged_file.first_tag() {
            tag_type = tag.tag_type();
        } else if let Some(default) = default_tag_type(path) {
            tag_type = default;
        }
    }
    let tag = match tagged_file.tag_mut(tag_type) {
        Some(tag) => tag,
        None => {
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file
                .tag_mut(tag_type)
                .context("create tag container")?
        }
    };

    if update.clear_title {
        tag.remove_title();
    }
    if update.clear_artist {
        tag.remove_artist();
    }
    if update.clear_album {
        tag.remove_album();
    }
    if update.clear_album_artist {
        tag.remove_key(&ItemKey::AlbumArtist);
    }
    if update.clear_year {
        tag.remove_year();
    }
    if update.clear_track_number {
        tag.remove_track();
    }
    if update.clear_disc_number {
        tag.remove_disk();
    }
    if let Some(clear_extra_tags) = update.clear_extra_tags {
        for key in clear_extra_tags {
            if key.trim().is_empty() {
                continue;
            }
            if tag_type == TagType::VorbisComments {
                remove_vorbis_key(tag, key);
            } else {
                tag.remove_key(&ItemKey::from_key(tag_type, key));
            }
        }
    }

    if let Some(value) = update.title {
        tag.set_title(value.to_string());
    }
    if let Some(value) = update.artist {
        tag.set_artist(value.to_string());
    }
    if let Some(value) = update.album {
        tag.set_album(value.to_string());
    }
    if let Some(value) = update.album_artist {
        tag.insert_text(ItemKey::AlbumArtist, value.to_string());
    }
    if let Some(value) = update.year {
        if value > 0 {
            tag.insert_text(ItemKey::Year, value.to_string());
        }
    }
    if let Some(value) = update.track_number {
        if value > 0 {
            tag.set_track(value);
        }
    }
    if let Some(value) = update.disc_number {
        if value > 0 {
            tag.set_disk(value);
        }
    }
    if let Some(extra_tags) = update.extra_tags {
        for (key, value) in extra_tags {
            if key.trim().is_empty() || value.trim().is_empty() {
                continue;
            }
            if tag_type == TagType::VorbisComments {
                let normalized = key.trim().to_ascii_uppercase();
                remove_vorbis_key(tag, &normalized);
                tag.insert_text(
                    ItemKey::from_key(TagType::VorbisComments, &normalized),
                    value.trim().to_string(),
                );
            } else {
                tag.insert_text(
                    ItemKey::from_key(tag_type, key.trim()),
                    value.trim().to_string(),
                );
            }
        }
    }

    tagged_file.save_to_path(path).context("write tags")?;
    Ok(())
}

/// Return default tag type inferred from file extension.
pub fn default_tag_type(path: &Path) -> Option<TagType> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let tag_type = match ext.as_str() {
        "flac" | "ogg" | "oga" | "opus" => TagType::VorbisComments,
        "mp4" | "m4a" | "m4b" | "m4p" | "aac" => TagType::Mp4Ilst,
        "mp3" => TagType::Id3v2,
        "wav" | "aif" | "aiff" => TagType::Id3v2,
        _ => TagType::Id3v2,
    };
    Some(tag_type)
}

/// Return editable track fields supported for this file/tag type.
pub fn supported_track_fields(path: &Path) -> (Option<TagType>, Vec<String>) {
    let tag_type = detect_tag_type(path).or_else(|| default_tag_type(path));
    let fields = match tag_type {
        Some(TagType::VorbisComments)
        | Some(TagType::Mp4Ilst)
        | Some(TagType::Id3v2)
        | Some(TagType::Id3v1)
        | Some(TagType::Ape) => {
            let mut names: Vec<String> = STANDARD_TRACK_FIELDS
                .iter()
                .map(|field| (*field).to_string())
                .collect();
            if tag_type == Some(TagType::VorbisComments) {
                if let Ok(tags) = read_vorbis_comment_tags(path) {
                    for key in tags.keys() {
                        if !names.contains(key) {
                            names.push(key.clone());
                        }
                    }
                }
            }
            names
        }
        _ => Vec::new(),
    };
    (tag_type, fields)
}

/// Read all Vorbis comment tags as uppercase keys.
pub fn read_vorbis_comment_tags(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    let tagged_file = read_from_path(path).context("read tags")?;
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
        .context("locate tag")?;
    if tag.tag_type() != TagType::VorbisComments {
        return Ok(values);
    }

    for item in tag.items() {
        let key = match item
            .key()
            .map_key(TagType::VorbisComments, true)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(key) => key.to_ascii_uppercase(),
            None => continue,
        };
        let ItemValue::Text(text) = item.value() else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(existing) = values.get_mut(&key) {
            existing.push_str("; ");
            existing.push_str(text);
        } else {
            values.insert(key, text.to_string());
        }
    }

    Ok(values)
}

/// Read only non-standard/editable Vorbis comment tags.
pub fn read_editable_vorbis_tags(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut tags = read_vorbis_comment_tags(path)?;
    let reserved: HashSet<&str> = STANDARD_VORBIS_KEYS.iter().copied().collect();
    tags.retain(|key, _| !reserved.contains(key.as_str()));
    Ok(tags)
}

/// Detect effective tag type from existing file tags.
fn detect_tag_type(path: &Path) -> Option<TagType> {
    let tagged_file = read_from_path(path).ok()?;
    let mut tag_type = tagged_file.primary_tag_type();
    if tagged_file.tag(tag_type).is_none() {
        if let Some(tag) = tagged_file.first_tag() {
            tag_type = tag.tag_type();
        }
    }
    Some(tag_type)
}

/// Remove Vorbis tag key case-insensitively.
fn remove_vorbis_key(tag: &mut Tag, key: &str) {
    tag.retain(|item| {
        item.key()
            .map_key(TagType::VorbisComments, true)
            .map(|existing| !existing.eq_ignore_ascii_case(key))
            .unwrap_or(true)
    });
}

/// Convert lofty [`TagType`] into API-friendly label.
pub fn tag_type_label(tag_type: TagType) -> &'static str {
    match tag_type {
        TagType::VorbisComments => "vorbis_comments",
        TagType::Mp4Ilst => "mp4_ilst",
        TagType::Id3v2 => "id3v2",
        TagType::Id3v1 => "id3v1",
        TagType::Ape => "ape",
        TagType::RiffInfo => "riff_info",
        TagType::AiffText => "aiff_text",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tag_type_maps_known_extensions() {
        assert_eq!(
            default_tag_type(Path::new("song.flac")),
            Some(TagType::VorbisComments)
        );
        assert_eq!(
            default_tag_type(Path::new("song.m4a")),
            Some(TagType::Mp4Ilst)
        );
        assert_eq!(
            default_tag_type(Path::new("song.mp3")),
            Some(TagType::Id3v2)
        );
    }

    #[test]
    fn default_tag_type_falls_back_to_id3v2() {
        assert_eq!(
            default_tag_type(Path::new("song.unknown")),
            Some(TagType::Id3v2)
        );
    }
}
