# LGO Video Shrinker — Project Context

## What this app is

A Windows desktop application to recursively scan folders for large `.mp4` video files, compress them using FFmpeg (with automatic hardware encoder detection: NVIDIA NVENC, Intel QuickSync, AMD AMF, or CPU `libx264`), monitor real-time progress and server logs in a split-pane UI, and safely move original files to the Windows Recycle Bin once compressed.

The UI is in French.

## Stack

### Primary Stack (Tauri v2 + Auto-Update — see [stack.md](./stack.md) and [PLAN/to-tauri.md](./PLAN/to-tauri.md))
| Layer | Choice |
|---|---|
| Desktop Framework | Tauri v2 (Rust backend in `src-tauri/` + native Windows WebView2) |
| Frontend | HTML5 + Bootstrap 5.3 (local bundle) + Vanilla JS + CSS bundled with Vite (`src/` $\rightarrow$ `dist-frontend/`) |
| Frontend–Backend IPC | `@tauri-apps/api` (`invoke` commands + `emit`/`listen` events) |
| Video Processing | Static `ffmpeg` & `ffprobe` Tauri sidecar binaries (`src-tauri/binaries/`) via `tokio::process::Command` |
| Folder Picker & Shell | `@tauri-apps/plugin-dialog` & `@tauri-apps/plugin-opener` |
| Recycle Bin Deletion | Rust `trash` crate (native Windows Shell Recycle Bin API) |
| Automatic Updates | `tauri-plugin-updater` + `tauri-plugin-process` + Minisign-signed GitHub Releases (`latest.json`) |
| Installer | Windows NSIS (`currentUser` install mode, passive updater mode) |

### Legacy Stack (Preserved Alongside Tauri During Transition)
| Layer | Choice |
|---|---|
| Backend | FastAPI (`app.py`) + Uvicorn on `http://127.0.0.1:8000` |
| Video Processing | FFmpeg / FFprobe via `imageio-ffmpeg` + `subprocess.Popen` |
| Folder Picker | `tkinter.filedialog.askdirectory` in a dedicated thread |
| Recycle Bin Deletion | Python `send2trash` (`1.8.3`) |
| Frontend | Jinja2 HTML (`templates/index.html`) + Bootstrap 5.3 CDN + Vanilla JS (`static/js/app.js`) + CSS (`static/css/style.css`) |
| Desktop Packaging | PyInstaller (`VideoShrinker.spec`) + Electron 31 (`electron/main.js`, `electron-builder` NSIS) |

## Project structure

```text
lgo-video-shrinker/
├── .github/
│   ├── agents/                        # Custom Copilot agents aligned with Tauri v2 + auto-update
│   │   ├── deployment.agent.md        # Git/GitHub release, signing keys, CI/CD & auto-update delivery specialist
│   │   ├── developer.agent.md         # Implementation & bug-fix specialist (Rust/Tauri, Vite/JS, legacy migration)
│   │   ├── orchestrator.agent.md      # Delivery coordinator across PM, Developer, QA, and Deployment
│   │   ├── product-manager.agent.md   # Requirements, user stories, and acceptance criteria specialist
│   │   └── qa.agent.md                # Non-regression testing (TNR) and verification specialist
│   ├── copilot-instructions.md        # Repository-wide Copilot rules and security constraints
│   ├── prompts/
│   │   └── update-context.prompt.md   # /update-context prompt definition
│   ├── skills/
│   │   └── update-context/
│   │       └── SKILL.md               # Skill to synchronize CONTEXT.md and CHANGELOG.md
│   └── workflows/
│       └── release.yml                # GitHub Actions CI/CD for signed Tauri v2 NSIS releases & latest.json
├── PLAN/
│   └── to-tauri.md                    # 6-phase migration plan from Python/Electron to Tauri v2 + auto-update
├── scripts/
│   ├── release.ps1                    # Automated SemVer version bump, commit, git tag, push & GH Actions watcher
│   └── setup-updater-keys.ps1         # Generates Minisign keypair, updates tauri.conf.json pubkey & sets GH secret
├── src/                               # Tauri v2 + Vite frontend source
│   ├── index.html                     # French UI with auto-update banner (#updateBanner) and version/encoder badges
│   ├── css/
│   │   └── style.css                  # Gradient UI theme, video cards, update banner, 2-column layout & resizable log panel
│   └── js/
│       ├── main.js                    # Tauri IPC (invoke/listen) frontend: scan, single/batch compression, Recycle Bin, logs
│       └── updater.js                 # Automatic startup & manual update checker, progress bar & relaunch
├── src-tauri/                         # Tauri v2 Rust backend
│   ├── Cargo.toml                     # Rust dependencies (tauri v2, dialog, opener, process, shell, updater, tokio, walkdir, trash)
│   ├── build.rs                       # Tauri build script
│   ├── tauri.conf.json                # Window (1400x900), NSIS currentUser bundle, sidecars, updater endpoint & pubkey
│   ├── capabilities/
│   │   └── default.json               # Tauri v2 permissions (core, dialog, opener, process, updater, shell)
│   ├── binaries/                      # Bundled Windows x64 sidecars
│   │   ├── ffmpeg-x86_64-pc-windows-msvc.exe
│   │   └── ffprobe-x86_64-pc-windows-msvc.exe
│   ├── icons/                         # Application icons (32x32.png, 128x128.png, 128x128@2x.png, icon.png, icon.ico)
│   └── src/
│       ├── main.rs                    # Binary entry point
│       ├── lib.rs                     # Tauri builder, plugin setup, get_app_info command, exit cleanup hook
│       ├── state.rs                   # AppState (hw_encoder, ffmpeg/ffprobe paths, active_processes mutex & cleanup)
│       ├── logger.rs                  # Emits server://log events to the frontend log panel
│       ├── scanner.rs                 # Recursive MP4 scanner, Windows long-path (\\?\) normalization, ffprobe parser + unit tests
│       ├── compressor.rs              # HW encoder probe, FFmpeg presets, async stderr progress parser, cancellation + unit/E2E tests
│       └── files.rs                   # Native folder dialog, Recycle Bin deletion (single/batch), Explorer reveal, video playback + tests
├── electron/                          # Legacy Electron wrapper
│   ├── main.js                        # Spawns VideoShrinker.exe or app.py, polls :8000, opens BrowserWindow (1400x900)
│   ├── package.json                   # Electron 31 & electron-builder NSIS configuration
│   └── preload.js                     # Preload script (contextIsolation: true, nodeIntegration: false)
├── static/                            # Legacy static frontend assets
│   ├── css/
│   │   └── style.css                  # Legacy CSS styles
│   └── js/
│       └── app.js                     # Legacy fetch/WebSocket frontend script
├── templates/
│   └── index.html                     # Legacy Jinja2 template served by FastAPI
├── package.json                       # Root Vite + Tauri CLI & frontend dependencies
├── vite.config.js                     # Vite bundler configuration (root: src, outDir: ../dist-frontend)
├── VideoShrinker.spec                 # Legacy PyInstaller single-file build specification
├── app.py                             # Legacy FastAPI backend
├── build.bat                          # Legacy PyInstaller build script
├── build_all.bat                      # Legacy two-phase PyInstaller + Electron build script
├── build_electron.bat                 # Legacy Electron-only build script
├── run_electron.bat                   # Legacy Electron dev launcher
├── requirements.txt                   # Legacy Python dependencies
├── stack.md                           # Target Tauri v2 + auto-update architecture specification
├── CHANGELOG.md                       # Chronological change history (source for latest changes)
└── CONTEXT.md                         # Architecture and current implementation context
```

## Latest changes

See [CHANGELOG.md](./CHANGELOG.md) for the chronological history of changes.

## Key implementation decisions

### Hardware Encoder Detection & Quality Presets
- At startup, the backend probes candidate H.264 encoders against a `0.1s` `64x64` null video (`-f lavfi -i nullsrc=s=64x64:d=0.1`) with `CREATE_NO_WINDOW` (`0x08000000` on Windows) in priority order:
  1. `h264_nvenc` (NVIDIA GPU)
  2. `h264_qsv` (Intel QuickSync)
  3. `h264_amf` (AMD GPU)
  4. Fallback: `libx264` (CPU with `-threads 0`)
- Three user-selectable quality modes (`low` / Rapide, `medium` / Équilibré [default], `high` / Lent):
  - **Resolution scaling**:
    - `low`: `scale=480:-2,format=yuv420p`
    - `medium`: `scale=1280:-2,format=yuv420p`
    - `high`: `format=yuv420p` (preserves original resolution)
  - **Encoder-specific quality & preset flags**:
    - `libx264`: `-preset fast|fast|medium -crf 28|23|20 -threads 0`
    - `h264_nvenc`: `-preset p4|p5|p6 -rc vbr -cq 32|26|22 -b:v 0`
    - `h264_qsv`: `-preset faster|medium|slow -global_quality 32|26|22`
    - `h264_amf`: `-quality speed|balanced|quality -qp_i 32|26|22 -qp_p 32|26|22`
  - **Common audio/container flags**: `-ac 2 -c:a aac -b:a 128k -pix_fmt yuv420p -movflags +faststart -progress pipe:2 -y`.

### Windows Long Paths (`> 260` characters)
- `normalize_path()` strips stray surrounding quotes, normalizes path separators, and prefixes paths longer than 240 characters on Windows with `\\?\` (extended-length path syntax).
- Directory traversal runs with the `\\?\` prefix so deeply nested OneDrive folders never fail with `FileNotFoundError`.
- Display and relative paths strip the `\\?\` prefix (`strip_long_prefix()` in Python, `dunce::simplified` in Rust).

### Real-Time Progress & Atomic Temporary File Handling
- Compression writes to `<stem>_compressed.tmp.mp4` in the same directory as the source video.
- Total duration is read beforehand via `ffprobe` (`format.duration`).
- While `ffmpeg` runs with `-progress pipe:2`, `stderr` is parsed for `out_time_ms=(\d+)` (microseconds). Progress updates are emitted whenever progress advances by $\ge 0.5\%$.
- Only when `ffmpeg` exits with code `0` is `<stem>_compressed.tmp.mp4` atomically renamed to `<stem>_compressed.mp4`.
- If `ffmpeg` fails, is stopped by the user, or the app closes, the active child process is terminated and `<stem>_compressed.tmp.mp4` is deleted immediately so partial files are never mistaken for completed compressions.

### Safe Deletion via Windows Recycle Bin
- Single deletion and batch deletion strictly validate that:
  1. The path is non-empty and exists on disk.
  2. The file extension is `.mp4` (case-insensitive).
- Files are never permanently unlinked; they are sent to the **Windows Recycle Bin** (`send2trash` in Python, `trash::delete` in Rust).
- In the frontend UI, the individual **"🗑️ Supprimer l'original"** button and the **"♻️ Supprimer les originaux"** batch modal filter only allow deleting originals that already have a `_compressed.mp4` file on disk.

### Frontend 2-Column Layout & Resizable Log Panel
- The UI uses a flex 2-column layout (`.app-layout`):
  - Left column (`.main-content`): directory picker, minimum size filter (`0`, `0.1`, `0.5`, `1` [default], `2`, `5` GB), summary stats card (video count, total GB, ~50% estimated savings), batch action bar, and video cards list.
  - Right column (`#logPanel`): real-time server/FFmpeg log stream (capped at 500 DOM entries, auto-scrolling when at bottom, color-coded by `INFO`, `ERROR`, `PROGRESS`, `SUCCESS`), collapsible via `#logToggleBtn`, and horizontally resizable via `#logSplitter` (width persisted in `localStorage` under `logPanelWidth`).

### Automatic Updates (Tauri v2 Target)
- Configured via `tauri-plugin-updater` checking GitHub Releases (`latest.json`).
- Update artifacts are cryptographically signed with a Minisign keypair (`pubkey` in `src-tauri/tauri.conf.json`, `TAURI_SIGNING_PRIVATE_KEY` in GitHub Secrets).
- On startup, the app checks for updates in the background and displays a non-intrusive top banner when a new version is available, supporting one-click download, install, and relaunch (`@tauri-apps/plugin-process`).

## API & IPC Mapping Reference

| Feature | Current FastAPI Endpoint (`app.py`) | Target Tauri v2 Command / Event (`src-tauri/`) |
|---|---|---|
| Choose folder | `GET /api/browse-directory` | `@tauri-apps/plugin-dialog` `open({ directory: true })` |
| Scan directory | `GET /api/scan?directory=...&min_size_gb=...` | `invoke('scan_videos', { directory, minSizeGb })` |
| Start compression | `WS /ws/compress/{video_index}` | `invoke('start_compression', { videoIndex, inputPath, quality })` + `listen('compression://event')` |
| Stop compression | `POST /api/stop-compression` | `invoke('stop_compression', { videoIndex })` |
| Delete single original | `POST /api/delete` | `invoke('delete_file', { filePath })` |
| Delete batch originals | `POST /api/delete-batch` | `invoke('delete_batch', { filePaths })` |
| Reveal in Explorer | `POST /api/open-folder` | `invoke('open_folder', { filePath })` |
| Play in default player | `POST /api/play-video` | `invoke('play_video', { filePath })` |
| Real-time logs | `WS /ws/logs` | `listen('server://log')` |

## Common commands

### Current Legacy Commands (Python + Electron)
```powershell
# Run FastAPI backend directly in dev mode (opens browser on http://127.0.0.1:8000)
python app.py

# Run Electron wrapper in dev mode (falls back to python app.py --no-browser if dist exe is absent)
.\run_electron.bat

# Build standalone Python executable with PyInstaller
.\build.bat

# Build full PyInstaller + Electron NSIS installer
.\build_all.bat
```

### Target Tauri v2 Commands (see [PLAN/to-tauri.md](./PLAN/to-tauri.md))
```powershell
# Start Vite + Tauri in development mode with hot reload
npm run tauri dev

# Type-check and test Rust backend
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml

# Build production Windows NSIS installer and signed updater artifacts
npm run tauri build
```

## Current capabilities

- Recursive `.mp4` directory scanning with minimum size filtering (`Tous les MP4`, `≥ 100 MB`, `≥ 500 MB`, `≥ 1 GB`, `≥ 2 GB`, `≥ 5 GB`) and descending size sort.
- Automatic detection of existing `<stem>_compressed.mp4` files (pre-unchecks already compressed videos during scan).
- Video metadata extraction via `ffprobe` (file size in GB, duration, resolution `WxH`, bitrate in Mbps).
- Hardware-accelerated H.264 compression (`h264_nvenc`, `h264_qsv`, `h264_amf`, or CPU `libx264`) with 3 quality presets (`Rapide`, `Équilibré`, `Lent`).
- Single-video and sequential batch compression with real-time progress bars, elapsed time / total duration display, size reduction summary, and cancellation support.
- Direct actions per video card to play the original or compressed video in VLC/default player and reveal the file in Windows Explorer.
- Single and batch deletion of original `.mp4` files to the Windows Recycle Bin with confirmation modal and skipped-file protection for uncompressed videos.
- Resizable, collapsible real-time server/FFmpeg log panel.

## Planned / next features

- **Tauri v2 + Auto-Update Migration**: Full architecture specification in [stack.md](./stack.md) and step-by-step migration plan in [PLAN/to-tauri.md](./PLAN/to-tauri.md).
- **In-App Update Notification Banner**: Automatic startup check against GitHub Releases `latest.json` with one-click download and restart.
- **GitHub Actions Release Workflow**: Automated Windows NSIS build, Minisign update signing, and GitHub Release publication on `v*` tags.

## Conventions techniques

- **UI Language**: All user-facing labels, buttons, modals, alerts, and status messages are in **French**.
- **Process Spawning on Windows**: Every `ffmpeg` or `ffprobe` subprocess must use `CREATE_NO_WINDOW` (`0x08000000`) on Windows so console windows never pop up over the GUI.
- **File Safety**: Never use permanent file deletion (`os.remove` / `std::fs::remove_file`) on user source videos—always use the Windows Recycle Bin (`send2trash` / `trash::delete`). Temporary `.tmp.mp4` artifacts created by the app are the only files that may be directly removed on failure/cancellation.
- **Secrets & Keys**: Never commit `.env` files, code-signing certificates, or Tauri updater private keys (`TAURI_SIGNING_PRIVATE_KEY`, `*.key`). Only the public updater key (`pubkey`) belongs in `tauri.conf.json`.
