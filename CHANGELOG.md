# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.13.1] - 2026-02-27

### Changed
- Patch release for version bump and release packaging updates; no functional runtime/API changes.

## [0.13.0] - 2026-02-27

### Added
- Configurable metadata DB location for the hub server:
  - config file key: `metadata_db_path`
  - CLI override: `--metadata-db-path <path>`
- Runtime-configurable Web UI API base for container deployments via `AUDIO_HUB_WEB_API_BASE` (no image rebuild required).
- Shared test lock helper in `session_registry` for deterministic tests that touch global session state.

### Changed
- Session queue internals migrated from file paths to track IDs (`now_playing`, queue items, history).
- Queue/session API flow is now ID-first end-to-end, resolving paths only at playback dispatch boundaries.
- Metadata SSE payloads no longer expose filesystem paths; events now use album labels and/or track IDs.
- Docker image entrypoint now generates `web-ui/dist/runtime-config.js` at container start.

### Removed
- Path-based public streaming endpoints:
  - `GET /stream?path=...`
  - `GET /stream/transcode?path=...`
- Path-based artwork endpoint:
  - `GET /art?path=...`
- Path-based track metadata query contract from public endpoints (replaced with `track_id` contracts).

### Fixed
- Local session pause control regression after ID migration.
- Session output-switch test flakiness caused by concurrent registry resets.

## [0.12.0] - 2026-02-26

### Added
- Session volume control API:
  - `GET /sessions/{id}/volume`
  - `POST /sessions/{id}/volume`
  - `POST /sessions/{id}/mute`
- Bridge volume/mute transport support with new bridge endpoints:
  - `GET /volume`
  - `POST /volume`
  - `POST /mute`
- Web UI player volume controls:
  - inline slider + mute action on wide layouts
  - compact popover with vertical slider on collapsed layouts

### Changed
- Bridge provider capabilities now advertise volume control support.
- Session output-switch migration now restores playback position and pause state in a single play request (seek + pause options included), reducing race conditions.
- Session status SSE now performs cast-specific periodic refresh (1s) while keeping non-cast outputs event-driven.
- Web UI playback bar behavior and responsiveness:
  - queue sidebar overlays content with backdrop tint
  - action button collapse breakpoint tuned for tighter layouts
  - compact volume control layered above grid content

### Fixed
- Cast output switching could skip to the next track due to transient idle-state handling.
- Cast pause/play reliability issues caused by stale toggle state and delayed media session readiness.
- Cast playback status mapping edge cases (idle/pause/clear behavior) that could leave controls desynced.
- Bridge elapsed-time reporting drift when device nominal sample rate differed from actual stream sample rate.
- Bridge status polling load reduced by relying on status stream updates and cached status in session paths.
- UI regressions in output/queue/track interactions:
  - track menu placement near bottom bar and item wrapping/spacing
  - queue modal double-scroll behavior
  - disabled input readability in metadata dialogs

## [0.11.0] - 2026-02-24

### Added
- Session-centric playback model in the hub API (`/sessions/{id}/...`) with queue and status streams.
- Session locks for outputs/bridges so one output cannot be controlled by multiple sessions at once.
- Session management UX in web UI:
  - session selector
  - create session modal (including `never expires`)
  - session delete action (default session protected)
- Local playback session mode in web UI (`Local`) with queue support and browser-side playback.
- Local playback status hydration on refresh (track metadata/progress recovery from queue + local snapshot).
- Session heartbeat + lease/TTL handling for expiring sessions.

### Changed
- Queue semantics are session-scoped, including history (`played`) and current item (`now_playing`) behavior.
- Local session playback now uses plain HTTP session queue endpoints for control (no browser transport channel).
- Auto-advance and queue updates are event-driven for session SSE streams.
- Bridge join behavior resets playback state so hub restart does not resume stale tracks automatically.

### Removed
- Legacy browser WebSocket transport (`/browser/ws`) and browser output provider.
- Legacy global playback flow in favor of session-scoped control paths.
- `hub-cli` crate from the workspace.

### Fixed
- Queue stream refresh issues after queue mutations in local sessions.
- Local session metadata rendering showing full file path instead of track metadata.
- Local session previous-button enablement based on queue history.
- Multiple local-session status/UI sync issues after tab refresh.
