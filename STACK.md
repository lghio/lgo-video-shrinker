# LGO Video Shrinker — Target Stack (Tauri v2 + Auto-Update)

## 1. Executive Summary

**LGO Video Shrinker** is transitioning from a dual-runtime desktop architecture (**Python/FastAPI/PyInstaller** wrapped inside **Electron**) to a single-runtime native architecture built on **Tauri v2 (Rust + Windows WebView2)** with built-in cryptographic **automatic updates (`tauri-plugin-updater`)** published via **GitHub Releases**.

| Metric | Legacy Stack (Python + PyInstaller + Electron) | Target Stack (Tauri v2 + Rust) |
| :--- | :--- | :--- |
| **Runtimes Shipped** | Python 3 + Uvicorn + Chromium + Node.js | Native Rust binary + OS WebView2 |
| **Installer Size** | ~180–250 MB | ~10–15 MB (+ `ffmpeg`/`ffprobe` binaries) |
| **Startup Time** | 3–8 seconds (PyInstaller temp extraction + HTTP polling) | < 0.5 seconds (instant native launch) |
| **Frontend-Backend Comm** | Local HTTP `127.0.0.1:8000` + WebSockets | Tauri IPC (`invoke` commands + `emit`/`listen` events) |
| **Folder Dialog** | `tkinter.filedialog` spawned from FastAPI thread | Native OS dialog (`@tauri-apps/plugin-dialog`) |
| **Recycle Bin Deletion** | Python `send2trash` | Rust `trash` crate (native Windows Shell API) |
| **Auto-Update** | None (manual installer distribution) | Built-in `tauri-plugin-updater` + GitHub Releases (`latest.json`) |

---

## 2. Target Architecture

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                      Windows Desktop Window (WebView2)                  │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │ Frontend UI (Vanilla JS / Vite + Bootstrap 5 + CSS)               │  │
│  │  • Directory selector & size filter                               │  │
│  │  • Video cards, quality presets (low / medium / high), progress   │  │
│  │  • Batch compression & batch Recycle Bin modal                    │  │
│  │  • Resizable real-time server/FFmpeg log panel                    │  │
│  │  • Auto-update notification banner ("Update vX.Y.Z available")    │  │
│  └───────────────┬───────────────────────────────────▲───────────────┘  │
└──────────────────┼───────────────────────────────────┼──────────────────┘
                   │ Tauri Commands (invoke)           │ Tauri Events (emit)
                   ▼                                   │
┌──────────────────────────────────────────────────────┴──────────────────┐
│                        Tauri v2 Core (Rust Backend)                     │
│  • Hardware encoder probe at startup (NVENC > QSV > AMF > libx264)      │
│  • Recursive MP4 scanner (`walkdir` + Windows extended-length paths)    │
│  • Metadata extractor (`ffprobe` sidecar JSON parser)                   │
│  • Compression manager (`ffmpeg` sidecar, `-progress pipe:2` parser,    │
│    active PID registry, cancellation, `.tmp.mp4` atomic rename/cleanup) │
│  • File operations (`trash` crate for Recycle Bin, Explorer `/select`,  │
│    default player launch via `tauri-plugin-opener`)                     │
│  • Updater client (`tauri-plugin-updater` + `tauri-plugin-process`)     │
└──────────────────┬───────────────────────────────────┬──────────────────┘
                   │ Sidecar Processes                 │ HTTPS Check
                   ▼                                   ▼
      ┌─────────────────────────┐         ┌─────────────────────────┐
      │ Bundled Sidecars        │         │ GitHub Releases         │
      │  • binaries/ffmpeg.exe  │         │  • latest.json          │
      │  • binaries/ffprobe.exe │         │  • *.nsis.zip / .exe    │
      └─────────────────────────┘         │  • *.sig (minisign)     │
                                          └─────────────────────────┘
```

---

## 3. Technology Stack Breakdown

### 3.1 Frontend Layer (`src/` or `ui/`)
- **Markup & Styling**: Existing HTML5 structure ([templates/index.html](./templates/index.html)) and custom CSS ([static/css/style.css](./static/css/style.css)) with **Bootstrap 5.3** (bundled locally to work 100% offline without CDN dependency).
- **Bundler**: **Vite** (lightweight dev server with HMR and static asset bundling into `dist/` for Tauri).
- **Client SDK**:
  - `@tauri-apps/api` (`core.invoke` for commands, `event.listen` for progress and logs).
  - `@tauri-apps/plugin-dialog` (native directory picker).
  - `@tauri-apps/plugin-opener` (open video in VLC/default player, reveal file in Windows Explorer).
  - `@tauri-apps/plugin-updater` & `@tauri-apps/plugin-process` (check, download, install update, and relaunch app).

### 3.2 Backend Layer (`src-tauri/`)
- **Framework**: **Tauri v2** (`tauri = { version = "2", features = [...] }`).
- **Core Rust Crates**:
  - `serde` / `serde_json` — serialization of scan results, video metadata, progress payloads, and `ffprobe` JSON output.
  - `tokio` — async runtime for non-blocking `ffmpeg` `stderr` progress streaming and concurrent process management.
  - `walkdir` — fast recursive directory traversal with native Windows long-path (`\\?\`) support.
  - `trash` — cross-platform, native Windows Recycle Bin deletion (`trash::delete` / `trash::delete_all`), replacing Python `send2trash`.
  - `tauri-plugin-shell` — executing bundled `ffmpeg` and `ffprobe` sidecar binaries with `CREATE_NO_WINDOW` (`0x08000000`) on Windows.
  - `tauri-plugin-dialog` — native folder selection dialog.
  - `tauri-plugin-opener` — launching default media player and revealing files in Windows Explorer.
  - `tauri-plugin-updater` — checking GitHub Releases `latest.json`, verifying cryptographic signatures, and applying updates.
  - `tauri-plugin-process` — clean application restart after update installation.

### 3.3 External Binaries (Tauri Sidecars)
Instead of relying on Python's `imageio-ffmpeg` at runtime:
- `ffmpeg` and `ffprobe` static Windows binaries are placed in `src-tauri/binaries/` with target-triple suffixes:
  - `src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe`
  - `src-tauri/binaries/ffprobe-x86_64-pc-windows-msvc.exe`
- Configured in `src-tauri/tauri.conf.json` under `bundle.externalBin: ["binaries/ffmpeg", "binaries/ffprobe"]`.

---

## 4. IPC Contract (Commands & Events)

### 4.1 Tauri Commands (`invoke`) — Replacing FastAPI HTTP Endpoints

| Legacy FastAPI Endpoint | Tauri Command (`#[tauri::command]`) | Arguments | Return Type |
| :--- | :--- | :--- | :--- |
| `GET /api/browse-directory` | `browse_directory` (or frontend `open({ directory: true })`) | None | `Option<String>` |
| `GET /api/scan` | `scan_videos` | `directory: String, min_size_gb: f64` | `ScanResponse { count, total_size_gb, videos, scanned_directory }` |
| `WS /ws/compress/{index}` | `start_compression` | `video_index: usize, input_path: String, quality: String` | `Result<(), String>` (streams progress via events) |
| `POST /api/stop-compression` | `stop_compression` | `video_index: usize` | `Result<String, String>` |
| `POST /api/delete` | `delete_file` | `file_path: String` | `Result<DeleteResponse, String>` |
| `POST /api/delete-batch` | `delete_batch` | `file_paths: Vec<String>` | `Result<BatchDeleteResponse, String>` |
| `POST /api/open-folder` | `open_folder` | `file_path: String` | `Result<(), String>` |
| `POST /api/play-video` | `play_video` | `file_path: String` | `Result<(), String>` |
| *(New)* | `get_app_info` | None | `{ version: String, hw_encoder: String, default_directory: String }` |

### 4.2 Tauri Events (`emit` / `listen`) — Replacing WebSockets

| Legacy WebSocket | Tauri Event Name | Payload Structure |
| :--- | :--- | :--- |
| `WS /ws/compress/{video_index}` | `compression://event` | `{ video_index, status: "started" \| "progress" \| "completed" \| "error", progress?, current_time?, total_duration?, output_path?, original_size_gb?, compressed_size_gb?, reduction_percent?, processing_time?, message? }` |
| `WS /ws/logs` | `server://log` | `{ level: "INFO" \| "ERROR" \| "PROGRESS" \| "SUCCESS", msg: String }` |

---

## 5. Automatic Update Architecture

### 5.1 How Users Get Updates Automatically
1. **Release Publication**:
   - Developer bumps version in `src-tauri/tauri.conf.json` and `package.json` (e.g., `1.1.0`) and pushes a git tag `v1.1.0`.
   - GitHub Actions runs `tauri-apps/tauri-action`, which compiles the Windows NSIS installer (`VideoShrinker_1.1.0_x64-setup.exe`), signs the update bundle with `TAURI_SIGNING_PRIVATE_KEY`, and publishes `latest.json` to **GitHub Releases**.
2. **Startup Check in App**:
   - When the user opens **VideoShrinker**, the frontend calls `check()` from `@tauri-apps/plugin-updater`.
   - Tauri fetches `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`.
3. **User Notification & One-Click Update**:
   - If `update.available === true`, a top notification banner appears in the UI showing the new version number and release notes:
     > 🔔 **Nouvelle version disponible (v1.1.0)** — [Mettre à jour maintenant] [Plus tard]
   - Clicking **Mettre à jour maintenant** calls `update.downloadAndInstall(progressCallback)` with a live download progress bar.
   - Before restarting (`relaunch()`), the app ensures any active FFmpeg child processes are terminated and any `.tmp.mp4` files are cleaned up.

### 5.2 Cryptographic Update Security
- Tauri v2 enforces **Minisign** signature verification on all updates:
  - Generate keys once: `npm run tauri signer generate -- -w ~/.tauri/videoshrinker.key`
  - **Public key** is committed in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`).
  - **Private key** (`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) is stored only in GitHub Actions Secrets.
  - Even if a network connection is intercepted, untrusted binaries can never be installed.

---

## 6. Target Repository Structure

```text
lgo-video-shrinker/
├── .github/
│   ├── agents/
│   │   ├── developer.agent.md
│   │   ├── orchestrator.agent.md
│   │   ├── product-manager.agent.md
│   │   └── qa.agent.md
│   └── workflows/
│       └── release.yml                # GitHub Actions CI/CD for signed Tauri releases + latest.json
├── PLAN/
│   └── to-tauri.md                    # Step-by-step migration plan
├── src/                               # Frontend source (bundled by Vite)
│   ├── index.html                     # Migrated from templates/index.html
│   ├── css/
│   │   └── style.css                  # Migrated from static/css/style.css + update banner styles
│   └── js/
│       ├── main.js                    # Migrated from static/js/app.js (using Tauri invoke/listen)
│       └── updater.js                 # Automatic update check, banner UI, download & relaunch
├── src-tauri/                         # Rust backend (Tauri v2)
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json                # Window config, bundle config, sidecars, updater endpoint & pubkey
│   ├── capabilities/
│   │   └── default.json               # Tauri v2 permissions (dialog, shell sidecar, updater, opener, process)
│   ├── binaries/
│   │   ├── ffmpeg-x86_64-pc-windows-msvc.exe
│   │   └── ffprobe-x86_64-pc-windows-msvc.exe
│   ├── icons/
│   │   └── icon.ico
│   └── src/
│       ├── main.rs                    # Entry point
│       ├── lib.rs                     # Tauri builder, plugin registration, state initialization
│       ├── state.rs                   # AppState (detected HW encoder, active FFmpeg child processes map)
│       ├── scanner.rs                 # Recursive MP4 scan, Windows long paths, ffprobe metadata extraction
│       ├── compressor.rs              # Hardware encoder detection, FFmpeg spawning, stderr progress parser, stop/cleanup
│       ├── files.rs                   # Recycle Bin deletion (single/batch), Explorer select, play video
│       └── logger.rs                  # Helper to emit `server://log` events to the frontend log panel
├── package.json                       # Vite + @tauri-apps/cli + @tauri-apps/api + plugins
├── vite.config.js
└── stack.md                           # This architecture document
```
