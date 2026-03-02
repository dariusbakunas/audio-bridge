#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Generate an audio fixture file with ffmpeg.

Usage:
  scripts/gen-audio-fixture.sh --output <path> [options]

Required:
  -o, --output <path>         Output file path (extension determines container by default)
      --input <path>          Input audio file (trimmed to --length); if omitted, synthetic source is used

Audio options:
  -r, --sample-rate <hz>      Sample rate (default: 44100)
  -l, --length <seconds>      Duration in seconds (default: 5)
  -b, --bitrate <value>       Bitrate, e.g. 128k, 320k (lossy codecs)
  -c, --channels <count>      Channels (default: 2)
  -f, --frequency <hz>        Tone frequency for sine source (default: 440)
      --codec <name>          Explicit codec (overrides extension mapping)
      --sample-format <fmt>   ffmpeg sample format (e.g. s16, s24, s32, flt)
      --source <kind>         Source generator: sine|pink|white (default: sine)
      --copy-metadata         Copy metadata from --input file
      --overwrite             Overwrite output file if it exists

Metadata shortcuts:
      --title <text>
      --artist <text>
      --album <text>
      --album-artist <text>
      --genre <text>
      --track <num>
      --disc <num>
      --year <yyyy>
      --comment <text>

Advanced metadata:
      --meta <key=value>      Add arbitrary metadata; may be repeated

Examples:
  scripts/gen-audio-fixture.sh -o web-ui/tests/fixtures/media/test.flac \
    -r 192000 -l 8 --title "Fixture FLAC" --artist "Test Artist" --album "Fixtures"

  scripts/gen-audio-fixture.sh -o web-ui/tests/fixtures/media/test.mp3 \
    -r 44100 -b 192k -l 6 --title "Fixture MP3" --meta encoder=audio-hub-test
EOF
}

require_ffmpeg() {
  if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "error: ffmpeg is required but was not found in PATH" >&2
    exit 1
  fi
}

lower() {
  tr '[:upper:]' '[:lower:]' <<<"$1"
}

OUTPUT=""
INPUT_FILE=""
SAMPLE_RATE=""
SAMPLE_RATE_SET=0
LENGTH_SEC="5"
BITRATE=""
BITRATE_SET=0
CHANNELS="2"
CHANNELS_SET=0
FREQUENCY="440"
CODEC=""
SAMPLE_FORMAT=""
SOURCE_KIND="sine"
COPY_METADATA=0
OVERWRITE=0

declare -a META_ARGS
META_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -o|--output)
      OUTPUT="${2:-}"
      shift 2
      ;;
    -r|--sample-rate)
      SAMPLE_RATE="${2:-}"
      SAMPLE_RATE_SET=1
      shift 2
      ;;
    -l|--length)
      LENGTH_SEC="${2:-}"
      shift 2
      ;;
    -b|--bitrate)
      BITRATE="${2:-}"
      BITRATE_SET=1
      shift 2
      ;;
    -c|--channels)
      CHANNELS="${2:-}"
      CHANNELS_SET=1
      shift 2
      ;;
    --input)
      INPUT_FILE="${2:-}"
      shift 2
      ;;
    -f|--frequency)
      FREQUENCY="${2:-}"
      shift 2
      ;;
    --codec)
      CODEC="${2:-}"
      shift 2
      ;;
    --sample-format)
      SAMPLE_FORMAT="${2:-}"
      shift 2
      ;;
    --source)
      SOURCE_KIND="$(lower "${2:-}")"
      shift 2
      ;;
    --copy-metadata)
      COPY_METADATA=1
      shift
      ;;
    --overwrite)
      OVERWRITE=1
      shift
      ;;
    --title)
      META_ARGS+=(-metadata "title=${2:-}")
      shift 2
      ;;
    --artist)
      META_ARGS+=(-metadata "artist=${2:-}")
      shift 2
      ;;
    --album)
      META_ARGS+=(-metadata "album=${2:-}")
      shift 2
      ;;
    --album-artist)
      META_ARGS+=(-metadata "album_artist=${2:-}")
      shift 2
      ;;
    --genre)
      META_ARGS+=(-metadata "genre=${2:-}")
      shift 2
      ;;
    --track)
      META_ARGS+=(-metadata "track=${2:-}")
      shift 2
      ;;
    --disc)
      META_ARGS+=(-metadata "disc=${2:-}")
      shift 2
      ;;
    --year)
      META_ARGS+=(-metadata "date=${2:-}")
      shift 2
      ;;
    --comment)
      META_ARGS+=(-metadata "comment=${2:-}")
      shift 2
      ;;
    --meta)
      META_ARGS+=(-metadata "${2:-}")
      shift 2
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

if [[ -z "$OUTPUT" ]]; then
  echo "error: --output is required" >&2
  usage
  exit 2
fi

require_ffmpeg

ext="${OUTPUT##*.}"
ext="$(lower "$ext")"

if [[ -z "$CODEC" ]]; then
  case "$ext" in
    flac) CODEC="flac" ;;
    wav) CODEC="pcm_s16le" ;;
    aiff|aif) CODEC="pcm_s16be" ;;
    mp3) CODEC="libmp3lame" ;;
    m4a) CODEC="aac" ;;
    aac) CODEC="aac" ;;
    ogg|oga) CODEC="libvorbis" ;;
    opus) CODEC="libopus" ;;
    *)
      echo "error: could not infer codec from extension '.$ext'; use --codec" >&2
      exit 2
      ;;
  esac
fi

if [[ "$BITRATE_SET" -eq 0 ]]; then
  case "$CODEC" in
    libmp3lame|aac|libvorbis) BITRATE="192k" ;;
    libopus) BITRATE="128k" ;;
    *) BITRATE="" ;;
  esac
fi

if [[ -z "$INPUT_FILE" && "$SAMPLE_RATE_SET" -eq 0 ]]; then
  SAMPLE_RATE="44100"
fi
if [[ -z "$INPUT_FILE" && "$CHANNELS_SET" -eq 0 ]]; then
  CHANNELS="2"
fi

mkdir -p "$(dirname "$OUTPUT")"

declare -a FF_ARGS
FF_ARGS=()
declare -a EXTRA_CODEC_ARGS
EXTRA_CODEC_ARGS=()

if [[ "$OVERWRITE" -eq 1 ]]; then
  FF_ARGS+=(-y)
else
  FF_ARGS+=(-n)
fi

if [[ -n "$INPUT_FILE" ]]; then
  if [[ ! -f "$INPUT_FILE" ]]; then
    echo "error: --input file not found: $INPUT_FILE" >&2
    exit 2
  fi
  FF_ARGS+=(-i "$INPUT_FILE")
else
  case "$SOURCE_KIND" in
    sine)
      FF_ARGS+=(
        -f lavfi
        -i "sine=frequency=${FREQUENCY}:sample_rate=${SAMPLE_RATE}:duration=${LENGTH_SEC}"
      )
      ;;
    pink)
      FF_ARGS+=(
        -f lavfi
        -i "anoisesrc=color=pink:sample_rate=${SAMPLE_RATE}:duration=${LENGTH_SEC}"
      )
      ;;
    white)
      FF_ARGS+=(
        -f lavfi
        -i "anoisesrc=color=white:sample_rate=${SAMPLE_RATE}:duration=${LENGTH_SEC}"
      )
      ;;
    *)
      echo "error: unsupported --source value '$SOURCE_KIND' (use sine|pink|white)" >&2
      exit 2
      ;;
  esac
fi

FF_ARGS+=(-t "$LENGTH_SEC" -c:a "$CODEC")
if [[ "$CHANNELS_SET" -eq 1 ]]; then
  FF_ARGS+=(-ac "$CHANNELS")
fi
if [[ "$SAMPLE_RATE_SET" -eq 1 ]]; then
  FF_ARGS+=(-ar "$SAMPLE_RATE")
fi

if [[ -n "$SAMPLE_FORMAT" ]]; then
  sample_fmt_norm="$(lower "$SAMPLE_FORMAT")"
  case "$sample_fmt_norm" in
    16|s16|16bit)
      SAMPLE_FORMAT="s16"
      ;;
    24|s24|24bit)
      # ffmpeg does not accept "s24" as sample_fmt; map to codec/sample_fmt safely.
      case "$CODEC" in
        pcm_*le)
          CODEC="pcm_s24le"
          SAMPLE_FORMAT=""
          ;;
        pcm_*be)
          CODEC="pcm_s24be"
          SAMPLE_FORMAT=""
          ;;
        flac)
          SAMPLE_FORMAT="s32"
          EXTRA_CODEC_ARGS+=(-bits_per_raw_sample 24)
          ;;
        *)
          SAMPLE_FORMAT="s32"
          ;;
      esac
      ;;
    32|s32|32bit)
      SAMPLE_FORMAT="s32"
      ;;
    *)
      SAMPLE_FORMAT="$sample_fmt_norm"
      ;;
  esac
fi

# Re-assert possibly adjusted codec after sample format normalization.
for i in "${!FF_ARGS[@]}"; do
  if [[ "${FF_ARGS[$i]}" == "-c:a" ]]; then
    FF_ARGS[$((i + 1))]="$CODEC"
    break
  fi
done

if [[ -n "$SAMPLE_FORMAT" ]]; then
  # Skip -sample_fmt when using explicit 24-bit PCM codec.
  if [[ "$CODEC" != "pcm_s24le" && "$CODEC" != "pcm_s24be" ]]; then
    FF_ARGS+=(-sample_fmt "$SAMPLE_FORMAT")
  fi
fi

if [[ -n "$BITRATE" ]]; then
  FF_ARGS+=(-b:a "$BITRATE")
fi

if [[ "$COPY_METADATA" -eq 1 ]]; then
  if [[ -z "$INPUT_FILE" ]]; then
    echo "error: --copy-metadata requires --input" >&2
    exit 2
  fi
  FF_ARGS+=(-map_metadata 0)
fi

if ((${#EXTRA_CODEC_ARGS[@]:-0})); then
  FF_ARGS+=("${EXTRA_CODEC_ARGS[@]}")
fi
if ((${#META_ARGS[@]:-0})); then
  FF_ARGS+=("${META_ARGS[@]}")
fi
FF_ARGS+=("$OUTPUT")

echo "Generating fixture:"
echo "  output:      $OUTPUT"
echo "  codec:       $CODEC"
if [[ "$SAMPLE_RATE_SET" -eq 1 || -z "$INPUT_FILE" ]]; then
  echo "  sample rate: $SAMPLE_RATE"
else
  echo "  sample rate: source"
fi
if [[ "$CHANNELS_SET" -eq 1 || -z "$INPUT_FILE" ]]; then
  echo "  channels:    $CHANNELS"
else
  echo "  channels:    source"
fi
echo "  duration:    $LENGTH_SEC s"
if [[ -n "$BITRATE" ]]; then
  echo "  bitrate:     $BITRATE"
fi
if [[ -n "$SAMPLE_FORMAT" ]]; then
  echo "  sample fmt:  $SAMPLE_FORMAT"
fi
if [[ -n "$INPUT_FILE" ]]; then
  echo "  input:       $INPUT_FILE"
  echo "  source:      input file"
else
  echo "  source:      $SOURCE_KIND"
fi

ffmpeg "${FF_ARGS[@]}"

echo "Done: $OUTPUT"
