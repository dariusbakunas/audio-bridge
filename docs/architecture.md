# Audio Hub Architecture

This document complements the README with deeper architecture diagrams for implementation and maintenance work.

## 1) Container view

```mermaid
flowchart LR
    subgraph Clients
      WEB[Web UI]
      IOS[iOS app]
      CLI[Other API clients]
    end

    subgraph HubHost["Media host"]
      HUB[audio-hub-server]
      DB[(metadata.sqlite)]
      LIB[(Music library)]
      CFG[(config.toml)]
    end

    subgraph Receivers
      BR1[bridge #1]
      BR2[bridge #N]
    end

    WEB --> HUB
    IOS --> HUB
    CLI --> HUB

    HUB <--> BR1
    HUB <--> BR2

    HUB <--> DB
    HUB <--> LIB
    HUB <--> CFG
```

## 2) Server component view (`audio-hub-server`)

```mermaid
flowchart TB
    API[HTTP API + SSE]
    Sessions[Session manager]
    Locks[Output/bridge lock manager]
    Outputs[Output/provider registry]
    Playback[Playback router + stream source]
    Metadata[Metadata service]
    Index[Library index/cache]
    DB[(metadata.sqlite)]
    FS[(Music FS)]

    API --> Sessions
    API --> Outputs
    API --> Metadata
    API --> Playback

    Sessions <--> Locks
    Sessions <--> Outputs
    Sessions <--> Playback

    Metadata <--> DB
    Metadata <--> FS
    Metadata --> Index

    Playback --> FS
    Playback --> Outputs
```

## 3) Session lifecycle and ownership

```mermaid
stateDiagram-v2
    [*] --> Created
    Created --> Active: heartbeat/session use
    Active --> Active: queue/play/status operations
    Active --> Expired: lease timeout (if ttl > 0)
    Active --> Deleted: explicit delete
    Created --> Deleted: explicit delete
    Expired --> Deleted
```

Notes:
- Sessions scope queue, playback control, status, and volume/mute.
- Output/bridge locks are session-bound.
- Local sessions and remote sessions share session APIs, with different playback execution paths.

## 4) Output lock model

```mermaid
sequenceDiagram
    participant C1 as Client A
    participant C2 as Client B
    participant HUB as Hub

    C1->>HUB: POST /sessions/{A}/outputs/select (output X)
    HUB-->>C1: 200 lock acquired

    C2->>HUB: POST /sessions/{B}/outputs/select (output X)
    HUB-->>C2: 409/locked (unless force)

    C2->>HUB: POST /sessions/{B}/outputs/select (output Y)
    HUB-->>C2: 200 lock acquired
```

## 5) Remote playback flow (session -> bridge)

```mermaid
sequenceDiagram
    participant UI as Client (web/iOS)
    participant HUB as audio-hub-server
    participant BR as bridge
    participant AP as audio-player

    UI->>HUB: Queue/play command (/sessions/{id}/...)
    HUB-->>UI: 200 + session updates

    BR->>HUB: GET /stream/track/{id} (range)
    HUB-->>BR: 206 audio bytes

    BR->>AP: decode/resample/output
    AP-->>BR: playback state/metrics
    BR-->>HUB: bridge status

    HUB-->>UI: SSE /sessions/{id}/status/stream
    HUB-->>UI: SSE /sessions/{id}/queue/stream
```

## 6) Local playback flow (client-managed renderer)

```mermaid
sequenceDiagram
    participant UI as Client (browser/iOS local)
    participant HUB as audio-hub-server

    UI->>HUB: Create/select local session
    UI->>HUB: Queue/control commands (/sessions/{id}/...)
    HUB-->>UI: Queue/status payloads

    UI->>HUB: Register local renderer (local-playback/register)
    UI->>HUB: Resolve/play local URL(s)
    Note over UI: Client decodes/plays locally

    HUB-->>UI: SSE status/queue for that session
```

## 7) Event/update model

```mermaid
flowchart LR
    HubState[Hub internal state changes]
    QEvt[Queue events]
    SEvt[Status events]
    LEvt[Locks/output events]
    MetaEvt[Metadata/log events]

    HubState --> QEvt --> QSSE["/sessions/:id/queue/stream"]
    HubState --> SEvt --> SSSE["/sessions/:id/status/stream"]
    HubState --> LEvt --> OSSE["/outputs/stream"]
    HubState --> MetaEvt --> MSSE["/metadata/stream, /logs/stream"]
```

Notes:
- Web UI is SSE-first for queue/status/outputs.
- Local-mode mobile clients may reduce/suspend streams in background and rely on snapshots/polling when needed.

## 8) Metadata pipeline

```mermaid
flowchart LR
    Scan[Library scan/rescan]
    Probe[Probe tags/format]
    Normalize[Normalize entities]
    Upsert[Upsert albums/tracks/artists]
    Marker[Album marker read/write]
    Cover[Cover art resolution]
    DB[(metadata.sqlite)]
    FS[(Music folders)]

    Scan --> Probe --> Normalize --> Upsert --> DB
    Scan --> Marker --> FS
    Normalize --> Cover --> DB
```

## 9) Web UI architecture (high level)

```mermaid
flowchart TB
    App[App.tsx composition]
    Hooks[Domain hooks\nsessions/playback/albums/streams]
    Components[Presentational components]
    API[HTTP + SSE API layer]

    App --> Hooks
    App --> Components
    Hooks --> API
```

The current refactor direction is to keep `App.tsx` as composition/orchestration and move behavior into focused hooks/components.
