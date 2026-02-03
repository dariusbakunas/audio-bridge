# audio-hub

[![CI](https://github.com/dariusbakunas/audio-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/dariusbakunas/audio-bridge/actions/workflows/ci.yml)

Stream audio files from your laptop/desktop to a small network “receiver” (perfect for a Raspberry Pi connected to a USB DAC).

This repo is a Rust workspace with two main apps:

- **`bridge`** (receiver): runs on the target machine (e.g. RPi). Listens on TCP, decodes and plays audio through the selected output device.
- **`audio-hub-server`** (server): runs on the media rack. Scans your library and exposes a small HTTP API for control.
- **`hub-cli`** (client): runs on your machine. A small TUI that connects to the server to browse and control playback.

Each binary supports `--version`, which includes the crate version, git SHA, and build date.

## What this is for

If you have a quiet little box on your network (RPi + USB DAC) and you want:

- “Pick a FLAC/WAV on my laptop”
- “Play it on the Pi”
- “Pause/Resume/Next from the sender UI”

…this project is for you.

## Workspace layout

```text
. 
├─ crates/ 
│ ├─ bridge/ # audio receiver (bridge)
│ ├─ hub-cli/ # HUB client, TUI app 
│ ├─ audio-hub-server/ # HTTP control server, audio library scanner, audio source
│ └─ audio-bridge-proto/ # shared protocol types/utilities 
├─ Cross.toml 
├─ dist-workspace.toml
└─ Cargo.toml

```

## Supported formats

Library scanning recognizes: **flac, wav, aiff/aif, mp3, m4a, aac, alac, ogg/oga, opus**.  
Decoding is provided by Symphonia; exact coverage depends on enabled features and container support.

## Quick start (local network)

### 1) Run the receiver on the Pi (or any Linux box)

Pick a bind address/port (example uses `:5555`):

```bash
cargo run --release -p bridge -- listen --bind 0.0.0.0:5555
```

Optional: list output devices and choose one by substring:

```bash
cargo run --release -p bridge -- --list-devices
cargo run --release -p bridge -- --device "USB" listen --bind 0.0.0.0:5555
```

### 2) Run the sender on your machine

First start the server on the machine that hosts your media (config is required):

```bash
cargo run --release -p audio-hub-server -- --bind 0.0.0.0:8080 --config crates/audio-hub-server/config.example.toml
```

Then point the TUI at the server:

```bash
 cargo run --release -p hub-cli -- --server http://<SERVER_IP>:8080 --dir <SERVER_MUSIC_DIR>
```

## Server config

Use a TOML config to define the media path, outputs, and default output:

```toml
bind = "0.0.0.0:8080"
media_dir = "/srv/music"
active_output = "bridge:living-room:Built-in Output"

[[bridges]]
id = "living-room"
name = "Living Room"
addr = "192.168.1.50:5555"
api_port = 5556
```

Pass it via `--config` (you can still override the media path via `--media-dir`). If `--config` is omitted, the server will look for `config.toml` next to the binary.

```bash
cargo run --release -p audio-hub-server -- --bind 0.0.0.0:8080 --config crates/audio-hub-server/config.example.toml
```

## hub-cli keys (TUI)

- **↑/↓**: select track
- **Enter**: play selected track (starts streaming immediately)
- **Space**: pause/resume
- **n**: next (skip immediately)
- **r**: rescan directory
- **q**: quit

### Hub-CLI screenshot

![hub-cli TUI screenshot](docs/screenshots/hub-cli.png)

## Tuning playback stability vs latency

`bridge` exposes a few knobs that trade latency for underrun resistance.

- **Default (USB stable)**  
  `--buffer-seconds 2.0 --chunk-frames 1024 --refill-max-frames 4096`

- **Paranoid stable (busy CPU / recording session vibes)**  
  `--buffer-seconds 4.0 --chunk-frames 2048 --refill-max-frames 8192`

- **Lower latency (snappier start/stop, requires a happier system)**  
  `--buffer-seconds 0.75 --chunk-frames 512 --refill-max-frames 2048`

Example:

```bash
cargo run --release -p bridge --
--buffer-seconds 2.0
--chunk-frames 1024
--refill-max-frames 4096
listen --bind 0.0.0.0:5555
```

## Server API (quick map)

- `GET /library` (list a directory; use `?dir=...`)
- `POST /library/rescan`
- `POST /play`
- `POST /pause`
- `GET /queue`
- `POST /queue`
- `POST /queue/remove`
- `POST /queue/clear`
- `POST /queue/next`
- `GET /status`
- `GET /bridges`
- `GET /bridges/{id}/outputs`
- `GET /outputs`
- `POST /outputs/select`
- `GET /swagger-ui/` (OpenAPI UI)

## Releases

Releases are handled by `cargo-dist` via GitHub Actions. Tag a version (e.g. `v0.1.1`) to trigger builds for all configured targets.

### Releasing a single crate

If you only want to publish one binary (e.g. `hub-cli`), bump that crate’s version and tag using the `package/version` format:

```bash
git tag hub-cli/0.1.2
git push origin hub-cli/0.1.2
```

This triggers a release for just that package (the tag name must match the Cargo package name).

## Why not AirPlay?

This is a direct, local-network stream to a dedicated receiver. Audio is decoded on the receiver and resampled to the output device’s native rate, avoiding protocol-level caps and keeping the path simple and controllable.

## Roadmap (nice-to-haves)

- Recursive library scanning / playlists
- More codecs/containers (and sender-side filtering)
- Better metadata (duration, sample rate) and progress UI polish
- Multiple receivers / discovery

## License

Licensed under the Apache License, Version 2.0. See `LICENSE` and `NOTICE`.
