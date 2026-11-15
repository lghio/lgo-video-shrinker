# LGO Video Shrinker — Project Context

## What this app is

A Windows desktop application to recursively scan folders for large video files (`.mp4`, `.mov`, `.mkv`, `.avi`, `.webm`), compress them into `.mp4` using FFmpeg (with automatic H.264 & H.265/HEVC hardware encoder detection: NVIDIA NVENC, Intel QuickSync, AMD AMF, or CPU `libx264`/`libx265`), monitor real-time progress, speed, ETA, and server logs in a split-pane UI, and safely move original files to the Windows Recycle Bin once compressed.

The UI is in French.

## Stack

| Layer | Choice |
|---|---|
| Desktop Framework | Tauri v2 (Rust backend in `src-tauri/` + native Windows WebView2) |
| Frontend | HTML5 + Bootstrap 5.3 (local bundle) + Vanilla JS + CSS bundled with Vite (`src/` $\rightarrow$ `src-tauri/target/frontend-dist/`) |
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
│   ├── skills/
│   │   └── update-context/
│   │       └── SKILL.md               # Skill to synchronize CONTEXT.md and CHANGELOG.md
│   └── workflows/
│       └── release.yml                # GitHub Actions CI/CD (TNR gate + signed Tauri v2 NSIS releases & latest.json)
├── PLAN/
│   ├── V1-1-to-tauri.md               # Completed migration plan from Python/Electron to Tauri v2 + auto-update
│   └── v2.md                          # Completed product & technical roadmap v1.1.0 -> v2.0.0 (18/18 tasks across 5 phases)
├── scripts/
│   ├── release.ps1                    # Pre-release Git/TNR check (--lib), UTF-8 SemVer version bump, commit, git tag, push & GH Actions watcher
│   └── setup-updater-keys.ps1         # Generates Minisign keypair, updates tauri.conf.json pubkey & sets GH secret
├── src/                               # Tauri v2 + Vite frontend source
│   ├── index.html                     # French UI: top menu bar, auto-update banner, Drag & Drop, codec/size selectors, scan progress, filter/sort toolbar, lifetime stats & modals (About, Changelog, Batch Delete)
│   ├── css/
│   │   └── style.css                  # Gradient UI theme, top menu bar, RTX & lifetime stats badges, filter toolbar, video cards, dropzone, recent folder chips & resizable log panel
│   └── js/
│       ├── main.js                    # Tauri IPC frontend: parallel scan progress/cancel, instant search/filter/sort, H.264/HEVC compression, speed/ETA, taskbar progress & notifications, Smart Skip, lifetime stats, Recycle Bin + rename, About/Changelog modals
│       └── updater.js                 # Automatic startup & manual update checker, progress bar & relaunch
├── src-tauri/                         # Tauri v2 Rust backend
│   ├── Cargo.toml                     # Rust dependencies (tauri v2, dialog, opener, process, shell, updater, tokio, walkdir, trash)
│   ├── build.rs                       # Tauri build script
│   ├── tauri.conf.json                # Window (1400x900), NSIS currentUser bundle, sidecars, updater endpoint & pubkey
│   ├── capabilities/
│   │   └── default.json               # Tauri v2 permissions (core, window progress/attention, dialog, opener, process, updater, shell)
│   ├── binaries/                      # Bundled Windows x64 sidecars (gitignored; auto-downloaded in CI)
│   │   ├── ffmpeg-x86_64-pc-windows-msvc.exe
│   │   └── ffprobe-x86_64-pc-windows-msvc.exe
│   ├── icons/                         # Application icons (32x32.png, 128x128.png, 128x128@2x.png, icon.png, icon.ico)
│   └── src/
│       ├── main.rs                    # Binary entry point
│       ├── lib.rs                     # Tauri builder, native menu (menu://action), get_app_info & probe_hardware commands, exit cleanup hook
│       ├── state.rs                   # AppState (H.264/HEVC encoders, GPU/RTX info, scan_cancelled flag, set_prevent_sleep, active_processes mutex & cleanup)
│       ├── logger.rs                  # Emits server://log events to the frontend log panel
│       ├── scanner.rs                 # Parallel multi-format scanner (.mp4/.mov/.mkv/.avi/.webm), scan://progress, cancel_scan, OneDrive Cloud-only, low-bitrate & AAC passthrough detection, Windows long-path (\\?\) + tests
│       ├── compressor.rs              # RTX GPU & H.264/HEVC HW encoder probe (256x256), FFmpeg presets, audio passthrough (-c:a copy), metadata (-map_metadata 0) & timestamp preservation, stderr progress/speed/ETA parser, Smart Skip, cancellation + unit/E2E tests
│       └── files.rs                   # Native folder dialog, multi-format Recycle Bin deletion (single/batch) + optional rename of _compressed.mp4 to <stem>.mp4 + tests
├── .gitignore                         # Excludes target/, node_modules/, binaries/*.exe, and *.key
├── package.json                       # Root Vite + Tauri CLI, TNR test & clean scripts (test / test:tnr / test:rust / clean) & frontend dependencies
├── vite.config.js                     # Vite bundler configuration (root: src, outDir: ../src-tauri/target/frontend-dist, port: 1420)
├── STACK.md                           # Tauri v2 + auto-update architecture specification
├── CHANGELOG.md                       # Chronological change history (source for latest changes)
└── CONTEXT.md                         # Architecture and current implementation context
```

## Latest changes

See [CHANGELOG.md](./CHANGELOG.md) for the chronological history of changes.

## Key implementation decisions

### Hardware Encoder Detection & Quality Presets (H.264 & H.265/HEVC)
- At startup (and on demand via `probe_hardware`), the backend probes the GPU model (`detect_nvidia_gpu_name` via `nvidia-smi` with Windows CIM fallback) and tests candidate H.264 and H.265/HEVC encoders against a `0.1s` `256x256` null video (`-f lavfi -i nullsrc=s=256x256:d=0.1`, avoiding NVENC's minimum resolution rejection on `64x64`) with `CREATE_NO_WINDOW` (`0x08000000` on Windows) in priority order:
  1. `h264_nvenc` / `hevc_nvenc` (NVIDIA GPU / RTX)
  2. `h264_qsv` / `hevc_qsv` (Intel QuickSync)
  3. `h264_amf` / `hevc_amf` (AMD GPU)
  4. Fallback: `libx264` / `libx265` (CPU with `-threads 0`)
- Three user-selectable quality modes (`low` / Rapide, `medium` / Équilibré [default], `high` / Lent) and two codecs (`h264` [default], `hevc` with `-tag:v hvc1` for native Windows/macOS/iOS playback):
  - **Resolution scaling**:
    - `low`: `scale=480:-2,format=yuv420p`
    - `medium`: `scale=1280:-2,format=yuv420p`
    - `high`: `format=yuv420p` (preserves original resolution)
  - **H.264 encoder-specific quality & preset flags**:
    - `libx264`: `-preset fast|fast|medium -crf 28|23|20 -threads 0`
    - `h264_nvenc`: `-hwaccel auto` (before `-i`) + `-preset p4|p4|p5 -tune hq -rc vbr -cq 32|26|22 -b:v 0 -multipass qres -spatial-aq 1 -temporal-aq 1`
    - `h264_qsv`: `-preset faster|faster|medium -global_quality 32|26|22`
    - `h264_amf`: `-quality speed|speed|balanced -qp_i 32|26|22 -qp_p 32|26|22`
  - **H.265 / HEVC encoder-specific quality & preset flags**:
    - `libx265`: `-preset fast|fast|medium -crf 30|25|22 -tag:v hvc1 -threads 0`
    - `hevc_nvenc`: `-hwaccel auto` (before `-i`) + `-preset p4|p4|p5 -tune hq -rc vbr -cq 34|28|24 -b:v 0 -multipass qres -spatial-aq 1 -temporal-aq 1 -tag:v hvc1`
    - `hevc_qsv`: `-preset faster|faster|medium -global_quality 34|28|24 -tag:v hvc1`
    - `hevc_amf`: `-quality speed|speed|balanced -qp_i 34|28|24 -qp_p 34|28|24 -tag:v hvc1`
  - **Common audio/container flags**: `-c:a copy` when `can_passthrough_audio` detects an existing AAC stream with `1..=2` channels and `bit_rate <= 135_000` bps ($\le \sim 128\text{ kbps}$); otherwise `-ac 2 -c:a aac -b:a 128k`. Always includes `-map_metadata 0 -pix_fmt yuv420p -movflags +faststart -progress pipe:2 -y`.

### Windows Long Paths (`> 260` characters)
- `normalize_path()` strips stray surrounding quotes, normalizes path separators, and prefixes paths longer than 240 characters on Windows with `\\?\` (extended-length path syntax).
- Directory traversal runs with the `\\?\` prefix so deeply nested OneDrive folders never fail with `FileNotFoundError`.
- Display and relative paths strip the `\\?\` prefix (`strip_long_prefix()` in Python, `dunce::simplified` in Rust).

### Parallel Multi-Format Scan, OneDrive Cloud-Only Protection & Low-Bitrate Detection
- Supported source extensions (`ALLOWED_VIDEO_EXTENSIONS`): `.mp4`, `.mov`, `.mkv`, `.avi`, `.webm` (excluding `*_compressed.mp4` and `*.tmp.mp4`).
- Two-phase scan (`scan_directory_with_progress` in `src-tauri/src/scanner.rs`):
  1. Fast `WalkDir` traversal collecting candidate video paths, NTFS file sizes, and Windows file attributes.
  2. Bounded thread-pool (`MAX_SCAN_WORKERS = 6` via `std::thread::scope`) running `ffprobe` in parallel, emitting `scan://progress` (`{ current, total, current_file }`) after each file, and checking `state.scan_cancelled` (`cancel_scan` command).
- **OneDrive Cloud-only detection (`is_cloud_only_attributes`)**: Inspects Windows NTFS attributes `FILE_ATTRIBUTE_OFFLINE` (`0x00001000`), `FILE_ATTRIBUTE_RECALL_ON_OPEN` (`0x00040000`), and `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (`0x00400000`). Cloud-only files skip `ffprobe` during scan so OneDrive never auto-downloads them, display a `☁️ OneDrive (En ligne uniquement)` badge, and are unchecked by default.
- **Low-bitrate detection (`is_low_bitrate_video`)**: Videos with bitrate `< 1.5 Mbps` (`< 800 kbps` if width $\le 640$) are flagged with `⚠️ Déjà optimisé (bitrate bas)` and unchecked by default.

### Real-Time Progress, Speed, ETA, Smart Skip, Atomic Temp Files & Sleep Prevention
- Compression writes to `<stem>_compressed.tmp.mp4` in the same directory as the source video and always outputs `<stem>_compressed.mp4`.
- Total duration is read beforehand via `ffprobe` (`format.duration`).
- While `ffmpeg` runs with `-progress pipe:2`, `stderr` is parsed for `out_time_ms=(\d+)` (microseconds) and `speed=\s*([\d.]+)x`. Progress updates (`progress`, `current_time`, `speed`, `eta_seconds`) are emitted whenever progress advances by $\ge 0.5\%$.
- **Windows Sleep Prevention (`set_prevent_sleep`)**: Calls `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` while at least one FFmpeg process is active in `state.active_processes`, and restores `SetThreadExecutionState(ES_CONTINUOUS)` as soon as `active_processes` becomes empty.
- **Smart Skip Anti-Inflation (`should_skip_inflated_output`)**: When `ffmpeg` exits with code `0`, if `<stem>_compressed.tmp.mp4` is $\ge$ the original file size in bytes, the temporary file is deleted immediately, the original file is kept untouched, and `status: "skipped_no_gain"` is emitted.
- Otherwise, `<stem>_compressed.tmp.mp4` is atomically renamed to `<stem>_compressed.mp4`, and `preserve_file_timestamps` copies the source file's modification (`mtime`) and Windows creation (`created`) dates onto `<stem>_compressed.mp4` so chronological sorting in Windows Explorer and OneDrive is preserved.
- **Windows Taskbar Progress & Notifications**: The frontend updates the Windows taskbar icon progress (`getCurrentWindow().setProgressBar`, `core:window:allow-set-progress-bar`) during single and batch compression, resets it (`ProgressBarStatus.None`) when finished or cancelled, and alerts the user at the end of a batch via `requestUserAttention` (`core:window:allow-request-user-attention`) and the desktop `Notification` API.
- **Cumulative Lifetime Compression History (`lifetimeCompressionStats`)**: Persisted in `localStorage` across sessions and displayed in `#statsCard` (`#lifetimeStatsBadge`) and `#aboutModal` (`#resetLifetimeStatsBtn`).
- If `ffmpeg` fails, is stopped by the user, or the app closes, the active child process is terminated and `<stem>_compressed.tmp.mp4` is deleted immediately so partial files are never mistaken for completed compressions.

### Native Menu & Top UI Navigation (`À propos` & `Changelog` Modals)
- Both a native Windows Tauri menu (`Menu / Aide`) and a top UI navigation bar (`.top-menu-bar`) provide direct access to:
  - **À propos (About)** (`#aboutModal`): displays version, GitHub link, stack, and live **Accélération Matérielle & GPU** details (`gpu_name`, `is_rtx`, H.264 & HEVC hardware encoders, and `#reprobeGpuBtn` calling `probe_hardware`).
  - **Changelog** (`#changelogModal`): renders the project's `CHANGELOG.md` (embedded at compile time via `include_str!("../../CHANGELOG.md")`).

### Safe Deletion via Windows Recycle Bin & Optional Original Filename Replacement
- Single deletion and batch deletion strictly validate (`validate_video_for_deletion`) that:
  1. The path is non-empty and exists on disk.
  2. The file extension belongs to the allowed video formats (`.mp4`, `.mov`, `.mkv`, `.avi`, `.webm`, case-insensitive).
- Files are never permanently unlinked; they are sent to the **Windows Recycle Bin** (`trash::delete` in Rust).
- When `rename_compressed` is enabled (`true`), after `trash::delete` succeeds on the original video, `<stem>_compressed.mp4` is automatically renamed to `<stem>.mp4` (`rename_compressed_to_original` in `src-tauri/src/files.rs`) so directory filenames and shortcuts remain clean and consistent with the `.mp4` container.
- In the frontend UI, the individual **"🗑️ Supprimer l'original"** button and the **"♻️ Supprimer les originaux"** batch modal filter only allow deleting originals that already have a `_compressed.mp4` file on disk.

### Frontend 2-Column Layout & Resizable Log Panel
- The UI uses a flex 2-column layout (`.app-layout`):
  - Left column (`.main-content`): directory picker, minimum size filter (`0`, `0.1`, `0.5`, `1` [default], `2`, `5` GB), summary stats card (video count, total GB, ~50% estimated savings), batch action bar, and video cards list.
  - Right column (`#logPanel`): real-time server/FFmpeg log stream (capped at 500 DOM entries, auto-scrolling when at bottom, color-coded by `INFO`, `ERROR`, `PROGRESS`, `SUCCESS`), collapsible via `#logToggleBtn`, and horizontally resizable via `#logSplitter` (width persisted in `localStorage` under `logPanelWidth`).

### Automatic Updates (Tauri v2 Target)
- Configured via `tauri-plugin-updater` checking GitHub Releases (`latest.json`).
- Update artifacts are cryptographically signed with a Minisign keypair (`pubkey` in `src-tauri/tauri.conf.json`, `TAURI_SIGNING_PRIVATE_KEY` in GitHub Secrets).
- On startup, the app checks for updates in the background and displays a non-intrusive top banner when a new version is available, supporting one-click download, install, and relaunch (`@tauri-apps/plugin-process`).

### Systematic Non-Regression Testing (TNR Gates)
- `npm test` (`npm run test:tnr` in `package.json`) chains the Vite frontend build (`npm run build`) and the full Rust library unit & E2E test suite (`cargo test --manifest-path src-tauri/Cargo.toml --lib`, 36 tests covering Windows long paths, `*_compressed.mp4` exclusion, multi-format scanning, parallel `ffprobe` & cancellation, OneDrive Cloud-only detection, low-bitrate & Smart Skip anti-inflation, Smart Audio Passthrough `-c:a copy`, file timestamp & `-map_metadata 0` preservation, FFmpeg H.264/HEVC presets, `speed=` & ETA parsing, Windows sleep prevention, GPU/RTX NVENC hardware probing, synthetic video E2E compression on `libx264`, `h264_nvenc`, and `hevc_nvenc` (`hvc1`), and Recycle Bin deletion/renaming).
- `scripts/release.ps1` enforces a clean Git working tree, verifies the target tag does not exist, and executes the full TNR gate before modifying any version files or creating a release tag.
- `.github/workflows/release.yml` runs the full TNR suite on `windows-latest` right after downloading the static `ffmpeg`/`ffprobe` sidecars and before building/publishing the NSIS installer and `latest.json`.

## API & IPC Mapping Reference

| Feature | Tauri v2 Command / Event (`src-tauri/`) |
|---|---|
| App, GPU & Changelog info | `invoke('get_app_info')` $\rightarrow$ `{ version, hw_encoder, hw_encoder_hevc, gpu_name, is_rtx, default_directory, changelog_markdown }` |
| Re-probe GPU & HW encoders | `invoke('probe_hardware')` $\rightarrow$ `AppInfoResponse` |
| Choose folder | `invoke('browse_directory')` / `@tauri-apps/plugin-dialog` `open({ directory: true })` |
| Scan directory (parallel) | `invoke('scan_videos', { directory, minSizeGb })` + `listen('scan://progress')` |
| Cancel active scan | `invoke('cancel_scan')` |
| Start compression (H.264 / HEVC) | `invoke('start_compression', { videoIndex, inputPath, quality, codec })` + `listen('compression://event')` |
| Stop compression | `invoke('stop_compression', { videoIndex })` |
| Cleanup all compressions | `invoke('cleanup_all_compressions')` |
| Delete single original | `invoke('delete_file', { filePath, renameCompressed })` |
| Delete batch originals | `invoke('delete_batch', { filePaths, renameCompressed })` |
| Reveal in Explorer | `invoke('open_folder', { filePath })` |
| Play in default player | `invoke('play_video', { filePath })` |
| Real-time logs | `listen('server://log')` |
| Native menu actions | `listen('menu://action')` (`"about"` \| `"changelog"` \| `"probe_gpu"`) |

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

- Recursive multi-format video directory scanning (`.mp4`, `.mov`, `.mkv`, `.avi`, `.webm`) with parallel `ffprobe` worker pool (`MAX_SCAN_WORKERS = 6`), live progress bar (`scan://progress`), scan cancellation (`cancel_scan`), minimum size filtering (`Toutes les vidéos`, `≥ 100 MB`, `≥ 500 MB`, `≥ 1 GB`, `≥ 2 GB`, `≥ 5 GB`), and descending size sort.
- Detection of OneDrive Cloud-only placeholders (`FILE_ATTRIBUTE_OFFLINE`, `FILE_ATTRIBUTE_RECALL_ON_OPEN`, `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`) without triggering forced cloud downloads during scan, plus low-bitrate warning badges (`⚠️ Déjà optimisé (bitrate bas)`).
- Instant client-side search, status filter (`Toutes`, `À compresser uniquement`, `Déjà compressées`), and multi-criteria sorting (`Taille`, `Durée`, `Bitrate`, `Nom`).
- Persistence of the last scanned directory, up to 5 recent folders (quick-select chips + `<datalist>`), minimum size filter, and selected codec in `localStorage`, plus smart default directory fallback (`%USERPROFILE%\Videos`) and folder/video Drag & Drop onto the window.
- Automatic exclusion of `*_compressed.mp4` and `*.tmp.mp4` artifacts from standalone scan results, with automatic detection of existing `<stem>_compressed.mp4` counterparts (pre-unchecks already compressed videos and displays existing compressed size in GB and `%` reduction).
- Video metadata extraction via `ffprobe` (file size in GB, duration, resolution `WxH`, bitrate in Mbps).
- Hardware-accelerated H.264 (`h264_nvenc`, `h264_qsv`, `h264_amf`, or CPU `libx264`) and H.265/HEVC (`hevc_nvenc`, `hevc_qsv`, `hevc_amf`, or CPU `libx265` with `-tag:v hvc1`) compression with 3 quality presets (`Rapide`, `Équilibré`, `Lent`) per card and a global batch quality selector in `#batchActions`.
- Single-video and sequential batch compression with real-time progress bars, Windows taskbar progress indicator (`getCurrentWindow().setProgressBar`) & batch completion notification (`requestUserAttention` + desktop `Notification`), encoding speed (`x2.4`), per-video & batch ETA, Smart Audio Passthrough (`-c:a copy` when source audio is AAC $\le 2$ ch $\le 128\text{ kbps}$), container metadata (`-map_metadata 0`) & original file timestamp (`mtime`/creation) preservation, Smart Skip anti-inflation protection (`skipped_no_gain`), Windows sleep prevention (`SetThreadExecutionState`), consolidated end-of-batch summary banner, cumulative lifetime space savings counter (`localStorage`), and cancellation support.
- Direct actions per video card to play the original or compressed video in VLC/default player and reveal the file in Windows Explorer.
- Single and batch deletion of original video files (`.mp4`, `.mov`, `.mkv`, `.avi`, `.webm`) to the Windows Recycle Bin with confirmation modal, skipped-file protection for uncompressed videos, and optional automatic renaming of `<stem>_compressed.mp4` to `<stem>.mp4`.
- Resizable, collapsible real-time server/FFmpeg log panel.

## Planned / next features

- **Roadmap v2 ([PLAN/v2.md](./PLAN/v2.md)) — 100% Completed (18/18 tasks)**:
  - **Phase 1 (`v1.1.0`) — Completed**: Exclude `*_compressed.mp4` from scan & show existing compressed size/savings (#4), persist last/recent scan directories & min size filter + Drag & Drop (#1), global batch quality selector (#2), and optional rename of `_compressed.mp4` to original filename after Recycle Bin deletion (#9).
  - **Phase 2 (`v1.2.0`) — Completed**: Client-side search/filter/sort (#3), encoding speed & ETA (#8), end-of-batch summary (#12), and Windows sleep prevention (#13).
  - **Phase 3 (`v1.3.0`) — Completed**: Parallel `ffprobe` scan with progress bar & cancellation (#5), OneDrive Cloud-only placeholder detection (#11), and Smart Skip anti-inflation + low-bitrate badge (#7).
  - **Phase 4 (`v2.0.0`) — Completed**: Hardware & CPU HEVC/H.265 support (#6), multi-container support (`.mov`, `.mkv`, `.avi`, `.webm`) (#10), and automated Git/TNR pre-checks in `scripts/release.ps1` (#14).
  - **Phase 5 (`v2.0.0`) — Completed**: Preserve container metadata (`-map_metadata 0`) & original file timestamps (`mtime`/creation) (#15), Windows taskbar progress bar & batch completion notification (#16), Smart Audio Passthrough (`-c:a copy` for AAC stereo $\le 128\text{ kbps}$) (#17), and cumulative lifetime compression savings history in `#statsCard` & `#aboutModal` (#18).

## Conventions techniques

- **UI Language**: All user-facing labels, buttons, modals, alerts, and status messages are in **French**.
- **Process Spawning on Windows**: Every `ffmpeg` or `ffprobe` subprocess must use `CREATE_NO_WINDOW` (`0x08000000`) on Windows so console windows never pop up over the GUI.
- **File Safety**: Never use permanent file deletion (`os.remove` / `std::fs::remove_file`) on user source videos—always use the Windows Recycle Bin (`send2trash` / `trash::delete`). Temporary `.tmp.mp4` artifacts created by the app are the only files that may be directly removed on failure/cancellation.
- **Secrets & Keys**: Never commit `.env` files, code-signing certificates, or Tauri updater private keys (`TAURI_SIGNING_PRIVATE_KEY`, `*.key`). Only the public updater key (`pubkey`) belongs in `tauri.conf.json`.
