# audio-bridge

Stream audio files from your laptop/desktop to a small network “receiver” (perfect for a Raspberry Pi connected to a USB DAC).

This repo is a Rust workspace with two main apps:

- **`audio-bridge`** (receiver): runs on the target machine (e.g. RPi). Listens on TCP, decodes and plays audio through the selected output device.
- **`audio-send`** (sender): runs on your machine. A small TUI that scans a directory for audio files and streams the selected track to the receiver—plus basic transport controls.

## What this is for

If you have a quiet little box on your network (RPi + USB DAC) and you want:

- “Pick a FLAC/WAV on my laptop”
- “Play it on the Pi”
- “Pause/Resume/Next from the sender UI”

…this project is for you.

## Workspace layout

```text
. 
├─ src/ # audio-bridge (receiver) main crate 
├─ crates/ 
│ ├─ audio-send/ # sender TUI app 
│ └─ audio-bridge-proto/ # shared protocol types/utilities 
├─ Cross.toml 
├─ Dockerfile.cross 
└─ Cargo.toml

```

## Supported formats

- Sender (`audio-send`) currently focuses on: **`.flac`** and **`.wav`**
- Receiver (`audio-bridge`) uses Symphonia for decoding (FLAC enabled).

## Quick start (local network)

### 1) Run the receiver on the Pi (or any Linux box)

Pick a bind address/port (example uses `:5555`):

```bash
cargo run --release -p audio-bridge -- listen --bind 0.0.0.0:5555
```

Optional: list output devices and choose one by substring:

```bash
cargo run --release -p audio-bridge -- --list-devices
cargo run --release -p audio-bridge -- --device "USB" listen --bind 0.0.0.0:5555
```

### 2) Run the sender on your machine

Point it at the receiver and a directory to scan:

```bash
 cargo run --release -p audio-send -- --addr <RECEIVER_IP>:5555 --dir <MUSIC_DIR>
```

## audio-send keys (TUI)

- **↑/↓**: select track
- **Enter**: play selected track (starts streaming immediately)
- **Space**: pause/resume
- **n**: next (skip immediately)
- **r**: rescan directory
- **q**: quit

## Tuning playback stability vs latency

`audio-bridge` exposes a few knobs that trade latency for underrun resistance.

- **Default (USB stable)**  
  `--buffer-seconds 2.0 --chunk-frames 1024 --refill-max-frames 4096`

- **Paranoid stable (busy CPU / recording session vibes)**  
  `--buffer-seconds 4.0 --chunk-frames 2048 --refill-max-frames 8192`

- **Lower latency (snappier start/stop, requires a happier system)**  
  `--buffer-seconds 0.75 --chunk-frames 512 --refill-max-frames 2048`

Example:

```bash
cargo run --release -p audio-bridge --
--buffer-seconds 2.0
--chunk-frames 1024
--refill-max-frames 4096
listen --bind 0.0.0.0:5555
```

## Building for Raspberry Pi / Linux with cross

This repo includes a `cross` Docker image setup for Linux builds.

```bash
docker build --platform linux/amd64 -t audio-bridge-cross:x86_64-gnu -f Dockerfile.cross . 
cargo install cross 
rustup toolchain install stable-x86_64-unknown-linux-gnu --force-non-host
CROSS_CONTAINER_OPTS="--platform linux/amd64"
cross build --release --target x86_64-unknown-linux-gnu -p audio-bridge -p audio-send
```

> Note: the included `Cross.toml` / `Dockerfile.cross` are geared toward a GNU Linux target. Adjust targets as needed for your Pi model/toolchain.

## Roadmap (nice-to-haves)

- Recursive library scanning / playlists
- More codecs/containers (and sender-side filtering)
- Better metadata (duration, sample rate) and progress UI polish
- Multiple receivers / discovery

## License

TODO