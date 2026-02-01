# audio-hub

Stream audio files from your laptop/desktop to a small network “receiver” (perfect for a Raspberry Pi connected to a USB DAC).

This repo is a Rust workspace with two main apps:

- **`bridge`** (receiver): runs on the target machine (e.g. RPi). Listens on TCP, decodes and plays audio through the selected output device.
- **`audio-hub-server`** (server): runs on the media rack. Scans your library and exposes a small HTTP API for control.
- **`hub-cli`** (client): runs on your machine. A small TUI that connects to the server to browse and control playback.

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
│ ├─ bridge/ # receiver (bridge)
│ ├─ hub-cli/ # sender TUI app 
│ ├─ audio-hub-server/ # HTTP control server 
│ └─ audio-bridge-proto/ # shared protocol types/utilities 
├─ Cross.toml 
├─ Dockerfile.cross 
└─ Cargo.toml

```

## Supported formats

- Sender (`hub-cli`) currently focuses on: **`.flac`** and **`.wav`**
- Receiver (`bridge`) uses Symphonia for decoding (FLAC enabled).

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
active_output = "bridge:default"

[[outputs]]
id = "bridge:default"
kind = "bridge"
name = "Bridge (default)"
bridge_addr = "192.168.1.50:5555"
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

## Output selection (server API)

The server exposes output-agnostic endpoints for device selection:

- `GET /outputs` (list outputs)
- `POST /outputs/select` (set active output)
- `GET /outputs/{id}/devices` (list devices for an output)
- `POST /outputs/{id}/device` (set device by substring)

## Building for Raspberry Pi / Linux with cross

This repo includes a `cross` Docker image setup for Linux builds.

```bash
docker build --platform linux/amd64 -t audio-bridge-cross:x86_64-gnu -f Dockerfile.cross . 
cargo install cross 
rustup toolchain install stable-x86_64-unknown-linux-gnu --force-non-host
CROSS_CONTAINER_OPTS="--platform linux/amd64"
cross build --release --target x86_64-unknown-linux-gnu -p bridge -p hub-cli
```

> Note: the included `Cross.toml` / `Dockerfile.cross` are geared toward a GNU Linux target. Adjust targets as needed for your Pi model/toolchain.

## Roadmap (nice-to-haves)

- Recursive library scanning / playlists
- More codecs/containers (and sender-side filtering)
- Better metadata (duration, sample rate) and progress UI polish
- Multiple receivers / discovery

## License

Licensed under the Apache License, Version 2.0. See `LICENSE` and `NOTICE`.
