# audio-hub (audio-bridge) agent notes

## Project summary
- Rust workspace for a networked audio hub + receiver.
- Main binaries: `bridge` (receiver), `audio-hub-server` (server).
- Experimental web UI in `web-ui` (Vite + React).
- Browser local playback is client-managed via local sessions and session HTTP endpoints.

## Repo layout
- `crates/audio-hub-server`: HTTP API, library scan, output management, stream source.
- `crates/bridge`: HTTP-controlled receiver, playback pipeline.
- `crates/audio-player`: shared decode/resample/playback building blocks.
- `crates/audio-bridge-types`: shared types.
- `web-ui`: experimental dashboard (built to `web-ui/dist`).
- `docs`: screenshots, docs.

## Common commands
- Build all binaries (host): `make build` or `cargo build --release -p bridge -p audio-hub-server`
- Tests (workspace): `cargo test`
- Clean: `make clean`
- Live integration tests (MusicBrainz + Cover Art Archive): `cargo test -p audio-hub-server live_ -- --ignored --nocapture`
- Docker image (hub server): `docker build -f Dockerfile.server -t audio-hub-server:local .`
- Docker compose (hub server): `AUDIO_HUB_MEDIA_DIR=/path/to/music docker compose up --build -d`
- Docker Hub multi-arch publish: push tag `vX.Y.Z` or `X.Y.Z` (workflow `.github/workflows/docker-image.yml`, requires `DOCKERHUB_USERNAME` + `DOCKERHUB_TOKEN` secrets)

## Run (quick start)
- Receiver (Pi/target): `cargo run --release -p bridge -- --http-bind 0.0.0.0:5556 listen`
- Server (host): `cargo run --release -p audio-hub-server -- --bind 0.0.0.0:8080 --config crates/audio-hub-server/config.example.toml`
- Server (container): `docker run --network host -v $(pwd)/crates/audio-hub-server/config.example.toml:/config/config.toml -v /path/to/music:/music audio-hub-server:local`

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

## Session model (2026-02)
- Playback control is session-scoped (`/sessions/{id}/...`), not global.
- Outputs are locked per session. An output already locked by one session cannot be selected by another without `force`.
- `active_output` config startup default is removed; output selection should happen via session APIs/UI.
- Session status/queue streams:
  - `/sessions/{id}/status/stream`
  - `/sessions/{id}/queue/stream`

## Session volume (2026-02)
- Session-scoped volume endpoints:
  - `GET /sessions/{id}/volume`
  - `POST /sessions/{id}/volume` (`{ value: 0..100 }`)
  - `POST /sessions/{id}/mute` (`{ muted: bool }`)
- Volume support is provider-specific (bridge supports it; others may report unavailable).

## UI queue semantics (2026-02)
- `/sessions/{id}/queue/stream` refreshes on `StatusChanged` events because queue order depends on `now_playing`.
- Queue items include:
  - `now_playing: bool` (current track)
  - `played: bool` (recent history, currently last 10)
- Queue list prepends last played tracks (oldest → newest) above the current track.
- Previous reinserts the current track at the front of the queue before jumping back.

## Local playback (2026-02)
- Local playback is decoupled from output selection and remote session control.
- Browser local playback does not use `/browser/ws` and does not register browser outputs hub-side.
- Queue/control for local sessions flows through `/sessions/{id}/queue/...` HTTP endpoints.
- Endpoints:
  - `POST /local-playback/register`
  - `POST /local-playback/{session_id}/play`
  - `GET /local-playback/sessions`
- Multiple local playback sessions can exist concurrently.

## Bridge stream resilience (2026-02)
- Bridge HTTP range reader retries transient range failures (default: 5 attempts, 200ms backoff).
- Playback end reason exposed via `BridgeStatus.end_reason`:
  - `eof`, `error`, `stopped`
- Hub auto-advance only on `end_reason = eof`.

## Bridge/Cast status notes (2026-02)
- Bridge-side volume/mute endpoints:
  - `GET /volume`
  - `POST /volume`
  - `POST /mute`
- Cast device status can arrive sparsely/in bursts; session status SSE applies cast-only periodic refresh (1s) to keep UI responsive.
- Cast session auto-advance should only trigger on explicit `idleReason=FINISHED` (`end_reason=eof`), not generic idle transitions.
- Bridge elapsed/status sample-rate must reflect actual stream rate (not nominal hardware rate) to keep `elapsed_ms`/seek restoration accurate.
