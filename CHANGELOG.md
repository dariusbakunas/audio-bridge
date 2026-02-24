# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
