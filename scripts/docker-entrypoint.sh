#!/bin/sh
set -eu

RUNTIME_CFG="/app/web-ui/dist/runtime-config.js"
API_BASE="${AUDIO_HUB_WEB_API_BASE:-}"

if [ -f "$RUNTIME_CFG" ]; then
  if [ -w "$RUNTIME_CFG" ]; then
    printf 'window.__AUDIO_HUB_CONFIG__ = { apiBase: %s };\n' "$(printf '%s' "$API_BASE" | sed 's/\\/\\\\/g; s/"/\\"/g; s/.*/"&"/')" > "$RUNTIME_CFG"
  else
    echo "WARN: cannot write $RUNTIME_CFG as uid=$(id -u); skipping AUDIO_HUB_WEB_API_BASE runtime override" >&2
  fi
fi

exec /usr/local/bin/audio-hub-server "$@"
