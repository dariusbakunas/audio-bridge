#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Create a fixture YAML manifest from an existing album directory.

The manifest can be consumed by:
  scripts/gen-audio-fixtures-from-yaml.sh

Usage:
  scripts/gen-audio-fixtures-yaml-from-album.sh \
    --album-dir <path> \
    --config-out <path> \
    [options]

Required:
  -a, --album-dir <path>      Source album folder to scan recursively
  -c, --config-out <path>     Output YAML manifest path

Options:
  -o, --output-dir <path>     Fixture output directory in YAML (default: web-ui/tests/fixtures/media)
  -l, --length <seconds>      Trimmed fixture length (default: 5)
      --overwrite             Set defaults.overwrite=true in generated YAML
      --title-prefix <text>   Prefix generated title metadata with text
      --append                Merge generated entries into existing config instead of replacing
      --include-input-dir     Also emit defaults.input_dir for source-based generation (off by default)
      --absolute-input        With --include-input-dir, write absolute path instead of repo-relative
  -h, --help                  Show this help

Notes:
- This script reads source file properties via ffprobe.
- Generated YAML is synthetic-first by default (portable; no source paths).
- It preserves filename, sample_rate, channels, bitrate (when available),
  sample format hint (when derivable), and common metadata tags.
EOF
}

require_cmd() {
  local cmd="$1"
  local message="$2"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: $message" >&2
    exit 1
  fi
}

ALBUM_DIR=""
CONFIG_OUT=""
OUTPUT_DIR="web-ui/tests/fixtures/media"
LENGTH_SEC="5"
OVERWRITE=0
TITLE_PREFIX=""
ABSOLUTE_INPUT=0
INCLUDE_INPUT_DIR=0
APPEND=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -a|--album-dir)
      ALBUM_DIR="${2:-}"
      shift 2
      ;;
    -c|--config-out)
      CONFIG_OUT="${2:-}"
      shift 2
      ;;
    -o|--output-dir)
      OUTPUT_DIR="${2:-}"
      shift 2
      ;;
    -l|--length)
      LENGTH_SEC="${2:-}"
      shift 2
      ;;
    --overwrite)
      OVERWRITE=1
      shift
      ;;
    --title-prefix)
      TITLE_PREFIX="${2:-}"
      shift 2
      ;;
    --append)
      APPEND=1
      shift
      ;;
    --include-input-dir)
      INCLUDE_INPUT_DIR=1
      shift
      ;;
    --absolute-input)
      ABSOLUTE_INPUT=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$ALBUM_DIR" || -z "$CONFIG_OUT" ]]; then
  echo "error: --album-dir and --config-out are required" >&2
  usage
  exit 2
fi

if [[ ! -d "$ALBUM_DIR" ]]; then
  echo "error: album directory not found: $ALBUM_DIR" >&2
  exit 2
fi

require_cmd ffprobe "ffprobe is required but was not found in PATH"
if [[ "$APPEND" -eq 1 ]]; then
  require_cmd ruby "ruby is required for --append merge mode"
fi

yaml_quote() {
  local s="$1"
  local sq="'"
  s="${s//\'/${sq}${sq}}"
  printf "'%s'" "$s"
}

probe_entry() {
  local file="$1"
  local entry="$2"
  ffprobe -v error -select_streams a:0 -show_entries "stream=${entry}" \
    -of default=noprint_wrappers=1:nokey=1 "$file" 2>/dev/null | head -n1
}

probe_tag() {
  local file="$1"
  local tag="$2"
  ffprobe -v error -show_entries "format_tags=${tag}" \
    -of default=noprint_wrappers=1:nokey=1 "$file" 2>/dev/null | head -n1
}

probe_stream_bitrate() {
  local file="$1"
  ffprobe -v error -select_streams a:0 -show_entries stream=bit_rate \
    -of default=noprint_wrappers=1:nokey=1 "$file" 2>/dev/null | head -n1
}

to_kbps() {
  local bps="$1"
  if [[ -z "$bps" ]]; then
    return 0
  fi
  if [[ ! "$bps" =~ ^[0-9]+$ ]]; then
    return 0
  fi
  local kbps=$(( (bps + 500) / 1000 ))
  printf "%sk" "$kbps"
}

repo_root="$(pwd)"
album_abs="$(cd "$ALBUM_DIR" && pwd)"
config_dir="$(dirname "$CONFIG_OUT")"
mkdir -p "$config_dir"

tmp_files="$(mktemp)"
trap 'rm -f "$tmp_files"' EXIT

find "$album_abs" -type f \
  \( -iname '*.flac' -o -iname '*.wav' -o -iname '*.aiff' -o -iname '*.aif' \
     -o -iname '*.mp3' -o -iname '*.m4a' -o -iname '*.aac' -o -iname '*.ogg' \
     -o -iname '*.oga' -o -iname '*.opus' \) \
  | sort >"$tmp_files"

if [[ ! -s "$tmp_files" ]]; then
  echo "error: no supported audio files found in: $album_abs" >&2
  exit 2
fi

{
  echo "defaults:"
  echo "  output_dir: $(yaml_quote "$OUTPUT_DIR")"
  echo "  source: 'sine'"
  if [[ "$INCLUDE_INPUT_DIR" -eq 1 ]]; then
    album_parent_abs="$(cd "$album_abs/.." && pwd)"
    if [[ "$ABSOLUTE_INPUT" -eq 1 ]]; then
      echo "  input_dir: $(yaml_quote "$album_parent_abs")"
    else
      echo "  input_dir: $(yaml_quote "${album_parent_abs#$repo_root/}")"
    fi
  fi
  echo "  overwrite: $([[ "$OVERWRITE" -eq 1 ]] && echo true || echo false)"
  echo "  length: $LENGTH_SEC"
  echo
  echo "files:"

  while IFS= read -r file; do
    base="$(basename "$file")"
    album_name="$(basename "$album_abs")"
    output_rel="${album_name}/${base}"
    sample_rate="$(probe_entry "$file" sample_rate || true)"
    channels="$(probe_entry "$file" channels || true)"
    codec_name="$(probe_entry "$file" codec_name || true)"
    bits_raw="$(probe_entry "$file" bits_per_raw_sample || true)"
    bits_sample="$(probe_entry "$file" bits_per_sample || true)"
    bit_rate="$(probe_stream_bitrate "$file" || true)"
    bitrate_k="$(to_kbps "$bit_rate" || true)"

    title="$(probe_tag "$file" title || true)"
    artist="$(probe_tag "$file" artist || true)"
    album="$(probe_tag "$file" album || true)"
    album_artist="$(probe_tag "$file" album_artist || true)"
    genre="$(probe_tag "$file" genre || true)"
    track="$(probe_tag "$file" track || true)"
    disc="$(probe_tag "$file" disc || true)"
    date="$(probe_tag "$file" date || true)"
    comment="$(probe_tag "$file" comment || true)"

    if [[ -n "$TITLE_PREFIX" ]]; then
      if [[ -n "$title" ]]; then
        title="${TITLE_PREFIX} ${title}"
      else
        title="${TITLE_PREFIX} ${base}"
      fi
    fi

    echo "  - output: $(yaml_quote "$output_rel")"
    if [[ -n "$sample_rate" && "$sample_rate" =~ ^[0-9]+$ ]]; then
      echo "    sample_rate: $sample_rate"
    fi
    if [[ -n "$channels" && "$channels" =~ ^[0-9]+$ ]]; then
      echo "    channels: $channels"
    fi
    if [[ -n "$bits_raw" && "$bits_raw" =~ ^[0-9]+$ ]]; then
      case "$bits_raw" in
        16) echo "    sample_format: s16" ;;
        24) echo "    sample_format: s24" ;;
        32) echo "    sample_format: s32" ;;
      esac
    elif [[ -n "$bits_sample" && "$bits_sample" =~ ^[0-9]+$ ]]; then
      case "$bits_sample" in
        16) echo "    sample_format: s16" ;;
        24) echo "    sample_format: s24" ;;
        32) echo "    sample_format: s32" ;;
      esac
    fi
    if [[ -n "$bitrate_k" ]]; then
      case "$codec_name" in
        mp3|aac|vorbis|opus)
          echo "    bitrate: $bitrate_k"
          ;;
      esac
    fi
    metadata_lines=0
    if [[ -n "$title" ]]; then
      metadata_lines=1
    fi
    if [[ -n "$artist" ]]; then
      metadata_lines=1
    fi
    if [[ -n "$album" ]]; then
      metadata_lines=1
    fi
    if [[ -n "$album_artist" ]]; then
      metadata_lines=1
    fi
    if [[ -n "$genre" ]]; then
      metadata_lines=1
    fi
    if [[ -n "$track" ]]; then
      metadata_lines=1
    fi
    if [[ -n "$disc" ]]; then
      metadata_lines=1
    fi
    if [[ -n "$date" ]]; then
      metadata_lines=1
    fi
    if [[ -n "$comment" ]]; then
      metadata_lines=1
    fi

    if [[ "$metadata_lines" -eq 0 ]]; then
      echo "    metadata: {}"
    else
      echo "    metadata:"
    fi
    if [[ -n "$title" ]]; then
      echo "      title: $(yaml_quote "$title")"
    fi
    if [[ -n "$artist" ]]; then
      echo "      artist: $(yaml_quote "$artist")"
    fi
    if [[ -n "$album" ]]; then
      echo "      album: $(yaml_quote "$album")"
    fi
    if [[ -n "$album_artist" ]]; then
      echo "      album_artist: $(yaml_quote "$album_artist")"
    fi
    if [[ -n "$genre" ]]; then
      echo "      genre: $(yaml_quote "$genre")"
    fi
    if [[ -n "$track" ]]; then
      echo "      track: $(yaml_quote "$track")"
    fi
    if [[ -n "$disc" ]]; then
      echo "      disc: $(yaml_quote "$disc")"
    fi
    if [[ -n "$date" ]]; then
      echo "      year: $(yaml_quote "$date")"
    fi
    if [[ -n "$comment" ]]; then
      echo "      comment: $(yaml_quote "$comment")"
    fi
  done <"$tmp_files"
} >/tmp/audio-fixtures.generated.$$.yml

if [[ "$APPEND" -eq 1 && -f "$CONFIG_OUT" ]]; then
  ruby - "$CONFIG_OUT" /tmp/audio-fixtures.generated.$$.yml <<'RUBY'
require "yaml"

dst_path = ARGV.fetch(0)
src_path = ARGV.fetch(1)

dst = YAML.load_file(dst_path) || {}
src = YAML.load_file(src_path) || {}

dst = {} unless dst.is_a?(Hash)
src = {} unless src.is_a?(Hash)

dst_defaults = dst["defaults"]
src_defaults = src["defaults"]
dst["defaults"] = dst_defaults.is_a?(Hash) ? dst_defaults : (src_defaults.is_a?(Hash) ? src_defaults : {})

dst_files = dst["files"].is_a?(Array) ? dst["files"] : []
src_files = src["files"].is_a?(Array) ? src["files"] : []

# Merge by output path. Existing order is preserved; matching outputs are replaced in place.
index_by_output = {}
dst_files.each_with_index do |entry, idx|
  next unless entry.is_a?(Hash)
  out = entry["output"]&.to_s
  next if out.nil? || out.empty?
  index_by_output[out] = idx
end

src_files.each do |entry|
  next unless entry.is_a?(Hash)
  out = entry["output"]&.to_s
  if out && !out.empty? && index_by_output.key?(out)
    dst_files[index_by_output[out]] = entry
  else
    dst_files << entry
    index_by_output[out] = dst_files.length - 1 if out && !out.empty?
  end
end

dst["files"] = dst_files

File.write(dst_path, YAML.dump(dst))
RUBY
  rm -f /tmp/audio-fixtures.generated.$$.yml
  echo "Updated manifest (append): $CONFIG_OUT"
else
  mv /tmp/audio-fixtures.generated.$$.yml "$CONFIG_OUT"
  echo "Wrote manifest: $CONFIG_OUT"
fi
