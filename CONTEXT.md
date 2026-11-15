# LGO Video Shrinker — Project Context

## What this app is

A Windows desktop application to recursively scan folders for large `.mp4` video files, compress them using FFmpeg (with automatic hardware encoder detection: NVIDIA NVENC, Intel QuickSync, AMD AMF, or CPU `libx264`), monitor real-time progress and server logs in a split-pane UI, and safely move original files to the Windows Recycle Bin once compressed.

The UI is in French.

## Stack

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

## Project structure

```text
lgo-video-shrinker/
├── .github/
│   ├── agents/                        # Custom Copilot agents aligned with Tauri v2 + auto-update
│   │   ├── deployment.agent.md        # Git/GitHub release, signing keys, CI/CD & auto-update delivery specialist
│   │   ├── developer.agent.md         # Implementation & bug-fix specialist (Rust/Tauri, Vite/JS)
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
│   ├── to-tauri.md                    # Completed migration plan from Python/Electron to Tauri v2 + auto-update
│   └── v2.md                          # Product & technical roadmap for v1.1.0 -> v2.0.0 (14 tasks across 4 phases)
├── scripts/
│   ├── release.ps1                    # Pre-release Git/TNR check, SemVer version bump, commit, git tag, push & GH Actions watcher
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
│   ├── binaries/                      # Bundled Windows x64 sidecars (gitignored; auto-downloaded in CI)
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
├── .gitignore                         # Excludes target/, node_modules/, dist-frontend/, binaries/*.exe, and *.key
├── package.json                       # Root Vite + Tauri CLI & frontend dependencies
├── vite.config.js                     # Vite bundler configuration (root: src, outDir: ../dist-frontend, port: 1420)
├── STACK.md                           # Tauri v2 + auto-update architecture specification
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

### Safe Deletion via Windows Recycle Bin & Optional Original Filename Replacement
- Single deletion and batch deletion strictly validate that:
  1. The path is non-empty and exists on disk.
  2. The file extension is `.mp4` (case-insensitive).
- Files are never permanently unlinked; they are sent to the **Windows Recycle Bin** (`trash::delete` in Rust).
- When `rename_compressed` is enabled (`true`), after `trash::delete` succeeds on `<stem>.mp4`, `<stem>_compressed.mp4` is automatically renamed to `<stem>.mp4` (`rename_compressed_to_original` in `src-tauri/src/files.rs`) so directory filenames and shortcuts remain unchanged.
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
| Delete single original | `POST /api/delete` | `invoke('delete_file', { filePath, renameCompressed })` |
| Delete batch originals | `POST /api/delete-batch` | `invoke('delete_batch', { filePaths, renameCompressed })` |
| Reveal in Explorer | `POST /api/open-folder` | `invoke('open_folder', { filePath })` |
| Play in default player | `POST /api/play-video` | `invoke('play_video', { filePath })` |
| Real-time logs | `WS /ws/logs` | `listen('server://log')` |

## Common commands

```powershell
# Start Vite + Tauri in development mode with hot reload
npm run tauri dev

# Build frontend bundle only
npm run build

# Run full TNR suite (Vite frontend build + Rust unit & E2E tests)
npm test

# Type-check and test Rust backend only
cargo check --manifest-path src-tauri/Cargo.toml
npm run test:rust

# Build production Windows NSIS installer and signed updater artifacts locally
npm run tauri build

# Generate Tauri updater Minisign keypair and configure GitHub Secret
npm run setup:keys

# Bump SemVer version, commit, tag vX.Y.Z, push to GitHub, and watch the release workflow
npm run release -- -Version 1.0.1 -Watch
```

## Current capabilities

- Recursive `.mp4` directory scanning with minimum size filtering (`Tous les MP4`, `≥ 100 MB`, `≥ 500 MB`, `≥ 1 GB`, `≥ 2 GB`, `≥ 5 GB`) and descending size sort.
- Persistence of the last scanned directory, up to 5 recent folders (quick-select chips + `<datalist>`), and minimum size filter in `localStorage`, plus smart default directory fallback (`%USERPROFILE%\Videos`) and folder Drag & Drop onto the window.
- Automatic exclusion of `*_compressed.mp4` and `*.tmp.mp4` artifacts from standalone scan results, with automatic detection of existing `<stem>_compressed.mp4` counterparts (pre-unchecks already compressed videos and displays existing compressed size in GB and `%` reduction).
- Video metadata extraction via `ffprobe` (file size in GB, duration, resolution `WxH`, bitrate in Mbps).
- Hardware-accelerated H.264 compression (`h264_nvenc`, `h264_qsv`, `h264_amf`, or CPU `libx264`) with 3 quality presets (`Rapide`, `Équilibré`, `Lent`) per card and a global batch quality selector in `#batchActions`.
- Single-video and sequential batch compression with real-time progress bars, elapsed time / total duration display, size reduction summary, and cancellation support.
- Direct actions per video card to play the original or compressed video in VLC/default player and reveal the file in Windows Explorer.
- Single and batch deletion of original `.mp4` files to the Windows Recycle Bin with confirmation modal, skipped-file protection for uncompressed videos, and optional automatic renaming of `<stem>_compressed.mp4` to `<stem>.mp4`.
- Resizable, collapsible real-time server/FFmpeg log panel.

## Planned / next features

- **Roadmap v2 ([PLAN/v2.md](./PLAN/v2.md))**:
  - **Phase 1 (`v1.1.0`) — Completed**: Exclude `*_compressed.mp4` from scan & show existing compressed size/savings (#4), persist last/recent scan directories & min size filter + Drag & Drop (#1), global batch quality selector (#2), and optional rename of `_compressed.mp4` to original filename after Recycle Bin deletion (#9).
  - **Phase 2 (`v1.2.0`)**: Client-side search/filter/sort (#3), encoding speed & ETA (#8), end-of-batch summary (#12), and Windows sleep prevention (#13).
  - **Phase 3 (`v1.3.0`)**: Parallel `ffprobe` scan with progress bar (#5), OneDrive Cloud-only placeholder detection (#11), and Smart Skip anti-inflation (#7).
  - **Phase 4 (`v2.0.0`)**: Hardware HEVC/H.265 support (#6) and multi-container support (`.mov`, `.mkv`, `.avi`, `.webm`) (#10).

## Conventions techniques

- **UI Language**: All user-facing labels, buttons, modals, alerts, and status messages are in **French**.
- **Process Spawning on Windows**: Every `ffmpeg` or `ffprobe` subprocess must use `CREATE_NO_WINDOW` (`0x08000000`) on Windows so console windows never pop up over the GUI.
- **File Safety**: Never use permanent file deletion (`os.remove` / `std::fs::remove_file`) on user source videos—always use the Windows Recycle Bin (`send2trash` / `trash::delete`). Temporary `.tmp.mp4` artifacts created by the app are the only files that may be directly removed on failure/cancellation.
- **Secrets & Keys**: Never commit `.env` files, code-signing certificates, or Tauri updater private keys (`TAURI_SIGNING_PRIVATE_KEY`, `*.key`). Only the public updater key (`pubkey`) belongs in `tauri.conf.json`.
