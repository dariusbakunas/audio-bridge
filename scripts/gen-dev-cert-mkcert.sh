#!/usr/bin/env bash
set -euo pipefail

if ! command -v mkcert >/dev/null 2>&1; then
  echo "mkcert is required. Install with: brew install mkcert"
  exit 1
fi

mkcert -install

mkdir -p certs

hosts=("${@}")
if [ "${#hosts[@]}" -eq 0 ]; then
  hosts=("localhost")
fi

mkcert -key-file certs/local.key -cert-file certs/local.crt "${hosts[@]}"

echo "Generated certs:"
echo "  certs/local.crt"
echo "  certs/local.key"
