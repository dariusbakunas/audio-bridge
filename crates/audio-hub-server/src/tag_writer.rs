use std::path::Path;

use anyhow::{Context, Result};
use lofty::{read_from_path, Accessor, AudioFile, ItemKey, Tag, TagType, TaggedFileExt};

pub struct TrackTagUpdate<'a> {
    pub title: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
    pub album_artist: Option<&'a str>,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
}

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

    tagged_file.save_to_path(path).context("write tags")?;
    Ok(())
}

fn default_tag_type(path: &Path) -> Option<TagType> {
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
