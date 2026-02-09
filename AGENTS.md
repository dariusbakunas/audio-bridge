# audio-hub (audio-bridge) agent notes

## Project summary
- Rust workspace for a networked audio hub + receiver.
- Main binaries: `bridge` (receiver), `audio-hub-server` (server), `hub-cli` (TUI client).
- Experimental web UI in `web-ui` (Vite + React).

## Repo layout
- `crates/audio-hub-server`: HTTP API, library scan, output management, stream source.
- `crates/bridge`: HTTP-controlled receiver, playback pipeline.
- `crates/audio-player`: shared decode/resample/playback building blocks.
- `crates/hub-cli`: terminal UI client.
- `crates/audio-bridge-types`: shared types.
- `web-ui`: experimental dashboard (built to `web-ui/dist`).
- `docs`: screenshots, docs.

## Common commands
- Build all binaries (host): `make build` or `cargo build --release -p bridge -p audio-hub-server -p hub-cli`
- Tests (workspace): `cargo test`
- Clean: `make clean`

## Run (quick start)
- Receiver (Pi/target): `cargo run --release -p bridge -- --http-bind 0.0.0.0:5556 listen`
- Server (host): `cargo run --release -p audio-hub-server -- --bind 0.0.0.0:8080 --config crates/audio-hub-server/config.example.toml`
- TUI client: `cargo run --release -p hub-cli -- --server http://<SERVER_IP>:8080 --dir <SERVER_MUSIC_DIR>`

## Web UI
- Dev: `cd web-ui && npm install && npm run dev`
- Build: `cd web-ui && npm install && npm run build`
- Serve: place `web-ui/dist` next to the server binary or repo root; open `http://<SERVER_IP>:8080/`.
- API base override: `VITE_API_BASE` for `npm run dev`.

## Notes
- Workspace edition: Rust 2024.
- Output IDs are namespaced (e.g. `bridge:<bridge_id>:<device_id>`).
- Config file example: `crates/audio-hub-server/config.example.toml`.
- API handlers live under `crates/audio-hub-server/src/api/*`; OpenAPI `paths(...)` should use module-qualified handlers (e.g. `api::outputs::outputs_select`).
- `rg` is not available in this environment; use `grep` for searches.
