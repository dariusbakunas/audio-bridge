#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Generate multiple audio fixtures from a YAML manifest.

Usage:
  scripts/gen-audio-fixtures-from-yaml.sh --config <file> [--dry-run]

Options:
  -c, --config <file>   YAML manifest path
      --dry-run         Print generated commands without executing
  -h, --help            Show this help

Manifest schema:
  defaults:             # optional defaults applied to each file item
    output_dir: web-ui/tests/fixtures/media
    input_dir: /path/to/original/album
    overwrite: true
    sample_rate: 44100
    length: 5
    channels: 2
    source: sine
    metadata:
      album: "E2E Fixtures"
      artist: "Audio Hub"
    meta:
      encoder: "fixture-gen"

  files:
    - output: flac-192k.flac
      sample_rate: 192000
      length: 8
      sample_format: s32
      metadata:
        title: "FLAC 192k"
        track: 1
    - output: mp3-320.mp3
      bitrate: 320k
      metadata:
        title: "MP3 320k"
      meta:
        comment2: "lossy fixture"

Supported per-file/default keys:
  output, input, input_dir, sample_rate, length, bitrate, channels, frequency, codec,
  sample_format, source, overwrite, copy_metadata, metadata(map), meta(map)
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

CONFIG=""
DRY_RUN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -c|--config)
      CONFIG="${2:-}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
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

if [[ -z "$CONFIG" ]]; then
  echo "error: --config is required" >&2
  usage
  exit 2
fi

if [[ ! -f "$CONFIG" ]]; then
  echo "error: config file not found: $CONFIG" >&2
  exit 2
fi

require_cmd ruby "ruby is required to parse YAML manifests"
require_cmd ffmpeg "ffmpeg is required to generate audio fixtures"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GENERATOR="${SCRIPT_DIR}/gen-audio-fixture.sh"

if [[ ! -x "$GENERATOR" ]]; then
  echo "error: generator script is missing or not executable: $GENERATOR" >&2
  exit 1
fi

ruby - "$CONFIG" "$GENERATOR" "$DRY_RUN" <<'RUBY'
require "yaml"
require "shellwords"

config_path = ARGV.fetch(0)
generator = ARGV.fetch(1)
dry_run = ARGV.fetch(2) == "1"

manifest = YAML.load_file(config_path) || {}
unless manifest.is_a?(Hash)
  warn "error: manifest root must be a map"
  exit 2
end

defaults = manifest["defaults"] || {}
files = manifest["files"] || []

unless files.is_a?(Array)
  warn "error: 'files' must be a list"
  exit 2
end

flag_map = {
  "input" => "--input",
  "sample_rate" => "--sample-rate",
  "length" => "--length",
  "bitrate" => "--bitrate",
  "channels" => "--channels",
  "frequency" => "--frequency",
  "codec" => "--codec",
  "sample_format" => "--sample-format",
  "source" => "--source"
}

meta_shortcuts = {
  "title" => "--title",
  "artist" => "--artist",
  "album" => "--album",
  "album_artist" => "--album-artist",
  "genre" => "--genre",
  "track" => "--track",
  "disc" => "--disc",
  "year" => "--year",
  "comment" => "--comment"
}

def deep_merge(base, override)
  return base unless override.is_a?(Hash)
  merged = base.dup
  override.each do |k, v|
    if merged[k].is_a?(Hash) && v.is_a?(Hash)
      merged[k] = deep_merge(merged[k], v)
    else
      merged[k] = v
    end
  end
  merged
end

def truthy?(value)
  value == true || value.to_s.strip.downcase == "true" || value.to_s == "1"
end

files.each_with_index do |item, idx|
  unless item.is_a?(Hash)
    warn "error: files[#{idx}] must be a map"
    exit 2
  end

  merged = deep_merge(defaults, item)
  output = merged["output"] || merged["path"]
  if output.nil? || output.to_s.strip.empty?
    warn "error: files[#{idx}] missing required 'output'"
    exit 2
  end

  output_rel = output.to_s
  output = output_rel
  output_dir = merged["output_dir"]&.to_s
  if output_dir && !output.start_with?("/")
    output = File.join(output_dir, output)
  end

  input = merged["input"]&.to_s
  input_dir = merged["input_dir"]&.to_s
  if (input.nil? || input.strip.empty?) && input_dir && !input_dir.strip.empty?
    input = File.join(input_dir, output_rel)
  end

  cmd = [generator, "--output", output]
  if input && !input.strip.empty?
    cmd << "--input" << input
  end

  flag_map.each do |key, flag|
    value = merged[key]
    next if value.nil? || value.to_s.empty?
    cmd << flag << value.to_s
  end

  cmd << "--overwrite" if truthy?(merged["overwrite"])
  cmd << "--copy-metadata" if truthy?(merged["copy_metadata"])

  metadata = merged["metadata"].is_a?(Hash) ? merged["metadata"] : {}
  meta_shortcuts.each do |key, flag|
    value = metadata[key]
    value = merged[key] if value.nil?
    next if value.nil? || value.to_s.empty?
    cmd << flag << value.to_s
  end

  raw_meta = merged["meta"]
  if raw_meta.is_a?(Hash)
    raw_meta.each do |k, v|
      next if k.nil? || v.nil?
      cmd << "--meta" << "#{k}=#{v}"
    end
  elsif raw_meta.is_a?(Array)
    raw_meta.each do |entry|
      next if entry.nil? || entry.to_s.strip.empty?
      cmd << "--meta" << entry.to_s
    end
  elsif !raw_meta.nil?
    warn "error: files[#{idx}] 'meta' must be a map or list"
    exit 2
  end

  if dry_run
    puts cmd.shelljoin
  else
    ok = system(*cmd)
    unless ok
      warn "error: fixture generation failed for files[#{idx}] (output=#{output})"
      exit 1
    end
  end
end
RUBY
