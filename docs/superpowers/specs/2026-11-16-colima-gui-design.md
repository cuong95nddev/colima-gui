# colima-gui — lightweight GUI for Colima (design)

Date: 2026-11-16

## Intent
Colima has no GUI. The user switched from Docker Desktop to Colima to save resources and
wants a fast, small, native-feeling window app (Docker Desktop–style) to monitor and
manage containers — without bringing the weight back. "Nhanh gọn lẹ": ship a useful MVP fast.

## Goals (MVP)
- Header: Colima status (running/stopped, CPU, memory, disk, runtime), Start/Stop Colima buttons.
- **Containers** tab: list (name, image, state, ports, CPU %, mem), grouped by Compose project;
  actions start / stop / restart / remove; live stats refresh every ~2 s.
- **Container detail**: streaming logs (tail 500, follow), CPU/RAM line chart (last ~2 min),
  "Shell" button opens Terminal.app running `docker exec -it <id> sh`.
- **Images** tab: list (repo:tag, size, created, in-use), remove, prune dangling.
- **Volumes** tab: list (name, driver, in-use), remove, prune unused.
- Search/filter box on each list.

## Non-goals
- No container runtime / VM of our own (Colima does that).
- No menu bar app (maybe later), no Kubernetes, no networks tab, no image build/pull UI,
  no embedded terminal, no multi-profile Colima support in MVP (uses `default` profile).

## Anti-goals
- Must not become heavy: target < 60 MB RAM idle, < 1% CPU idle, single binary.
- Must not block the UI thread on Docker calls.

## Approaches considered
1. **egui/eframe (chosen)** — pure Rust, single binary, light, `egui_plot` for charts.
   Trade-off: not 100% native macOS look; no embedded terminal (external Terminal instead).
2. Tauri + web frontend — prettier, embedded xterm.js, but needs a TS frontend and
   ~80–150 MB RAM (WebView). Rejected: works against "light".
3. GPUI — native and fast, but sparse docs / unstable API → slow to build. Rejected for speed.

## Technical design
- Crates: `eframe`/`egui`, `egui_plot`, `bollard` (Docker Engine API), `tokio`, `serde_json`.
- Docker connection: socket from `DOCKER_HOST`, else `~/.colima/default/docker.sock`.
- Colima status: `colima status --json` and `colima list --json` (CLI); start/stop via
  `colima start` / `colima stop` run in background tasks.
- Architecture: a tokio runtime on a background thread owns the Docker client. UI sends
  `Command` messages over a channel; backend publishes snapshots (`AppState`) through
  `Arc<Mutex<...>>` / channel and calls `ctx.request_repaint()`. UI never awaits.
- Stats: one `stats` stream per running container (stream=true), aggregated into a ring buffer
  of 60 samples per container.
- Destructive actions (remove container/image/volume, prune) ask for in-app confirmation.
- Errors shown as a dismissible banner; if socket missing → "Colima not running" screen with Start button.

## Layout
```
┌ colima-gui ─ ● colima running · 4 CPU · 6 GB · docker ──── [Stop] ┐
│ [Containers] [Images] [Volumes]            🔍 filter        │
│ ▾ myproj (compose)                                          │
│   ● postgres   postgres:16   5432→5432   0.4%  120MB  ■ ↻ ✕ │
│   ○ api        myproj-api    exited                 ▶   ✕   │
│ ─ detail: postgres ──────────────────────────── [Shell] ─── │
│   [CPU/MEM chart]                                           │
│   logs… (follow)                                            │
└─────────────────────────────────────────────────────────────┘
```

## Left to the builder
- Exact module split, styling/theme, icon set (plain text/unicode is fine for MVP).
- Packaging as `.app` bundle (`cargo bundle` or a small script) — nice-to-have after MVP.
