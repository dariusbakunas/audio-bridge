# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.16.0] - 2026-03-04

### Added
- Compact Web UI mode for narrow screens (mobile-style navigation, dedicated `Now Playing`/`Queue`/`Sessions` screens, and small-screen queue-first flow).
- Bridge graceful-unregister callback path for clean disconnect handling:
  - hub endpoint: `POST /providers/bridge/unregister`
  - bridge shutdown notification to known hub origins.
- E2E fixture generation tooling:
  - YAML-driven album fixture definitions
  - helper scripts to generate fixture audio from YAML and from existing albums.
- Isolated Docker Compose E2E environment for Playwright runs with per-browser stack separation.
- Bridge dummy output mode to support deterministic E2E execution.
- Additional E2E coverage for library browsing, queue interactions, connection gate behavior, and previous/next control semantics.

### Changed
- E2E workflow now auto-generates audio fixtures before test runs and avoids regenerating them repeatedly in the same run.
- Signal modal diagnostics now prioritize effective source/processing rates and clearer playback pipeline details.
- Album notes UX now expands inline from album detail instead of relying on modal-style presentation.

### Fixed
- End-of-queue behavior now clears stale `now_playing` state and updates controls/UI correctly (`Nothing playing`, no stale active track, `Next` disabled at queue end).
- Bridge/output cleanup now releases stale active output/session locks when a bridge exits gracefully.
- Bridge exclusive-mode handling and status propagation for output/sample-rate reporting.
- Settings view layout regressions where content could render behind the header after navigating from a scrolled album grid.
- Albums live-stream recovery now clears stale disconnect error state after SSE reconnection/open.

## [0.15.0] - 2026-03-02

### Added
- New architecture reference document at `docs/architecture.md` with system/component/session/playback/event diagrams.
- Additional Storybook coverage for core UI building blocks:
  - `SideNav`
  - `ViewHeader`
  - `NotificationsPanel`
  - `CreateSessionModal`
- Signal dialog pipeline visualization showing:
  - source format/rate/depth
  - bridge processing stage (direct vs resample)
  - output format/channels/rate

### Changed
- Continued web UI refactor to reduce `App.tsx` complexity by extracting orchestration into focused hooks/utilities (`session streams`, `session context`, `main content actions`, `chrome actions`, `UI state`, and related helpers).
- Signal dialog now avoids duplicated field presentation; pipeline view is primary, with diagnostics focused on bitrate and buffer state.

### Fixed
- Web UI session request storm regression after refactor (unstable `getClientId` callback causing repeated `/sessions` and `/sessions/locks` refresh loops).
- Signal dialog source-rate display now prefers pre-resample source rate (`resample_from_hz`) when available.
- TypeScript typing issues introduced during hook extraction (track menu action and play handler signatures).
- Missing Vite client type reference causing IDE asset import warning for `.png` modules.

## [0.14.1] - 2026-02-27

### Changed
- Docker Compose now supports image override via `AUDIO_HUB_IMAGE` (default remains `audio-hub-server:local`).
- Docker examples now document running containers as the current host `UID:GID` for writable bind mounts.
- Docker image workflow now routes `linux/arm64` builds to a native Ubuntu ARM runner instead of emulation.

### Fixed
- Container startup no longer fails when running with non-default runtime users and generating `web-ui/dist/runtime-config.js`.
- Entrypoint now logs a warning and continues if runtime web config is not writable, instead of aborting startup.

## [0.14.0] - 2026-02-27

### Changed
- Metadata DB now stores track paths relative to the configured media root, while API/runtime path resolution remains absolute at the boundaries.
- Startup now normalizes existing absolute track paths in metadata DB to relative paths (with duplicate-safe migration behavior).
- Docker builds now pass the Git commit SHA into the Web UI build stage so containerized UI version info can reflect source revision.

### Fixed
- Dockerized deployments no longer break metadata path resolution when the host media mount path differs from the in-container media root.
- Added structured warning logs for ambiguous `404`/lookup failures across stream, metadata, session queue/output, and local playback APIs to improve operator diagnostics.

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
