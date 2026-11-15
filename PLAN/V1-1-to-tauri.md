# Migration Plan: Python/FastAPI + Electron $\rightarrow$ Tauri v2 + Auto-Update

> **Reference Architecture**: See [../stack.md](../stack.md) for the complete target stack specification.

---

## 1. Migration Goals & Non-Regression Requirements

### Goals
1. **Eliminate Dual Runtimes**: Replace Python/FastAPI/Uvicorn/PyInstaller and Electron with a single native **Tauri v2 (Rust)** application.
2. **Preserve 100% Feature Parity**: Keep the existing French UI, Bootstrap 5 layout, resizable log panel, hardware-accelerated FFmpeg presets (`h264_nvenc`, `h264_qsv`, `h264_amf`, `libx264`), real-time progress bars, batch operations, and Windows Recycle Bin safety.
3. **Enable Automatic Updates**: Integrate `tauri-plugin-updater` + GitHub Releases so users are automatically notified of new versions and can update in-app with one click.

### Non-Regression Invariants (Must Not Break)
- **Windows Long Paths (`> 260 chars`)**: Scanning, probing, compressing, and deleting inside deeply nested OneDrive/project paths must continue to work reliably.
- **Hardware Encoder Priority**: Startup detection must probe `h264_nvenc` $\rightarrow$ `h264_qsv` $\rightarrow$ `h264_amf` $\rightarrow$ `libx264` with `CREATE_NO_WINDOW` (`0x08000000`) so no black console windows flash on screen.
- **Atomic Output & Temp File Cleanup**: Compression must write to `<stem>_compressed.tmp.mp4` first, rename to `<stem>_compressed.mp4` only on exit code `0`, and delete `.tmp.mp4` on error, cancellation, or app exit.
- **Safe Deletion**: Deleting originals (single or batch) must only allow `.mp4` files and must send them to the **Windows Recycle Bin** (never permanent unlink).

---

## 2. Feature-by-Feature Mapping (`app.py` $\rightarrow$ Rust `src-tauri/`)

| Current Location | Current Implementation | Target Location | Target Tauri v2 / Rust Implementation |
| :--- | :--- | :--- | :--- |
| [app.py](../app.py) (`detect_hw_encoder`) | `subprocess.run` probing `h264_nvenc`, `h264_qsv`, `h264_amf` | `src-tauri/src/compressor.rs` | Run once at startup via `Command::new(ffmpeg_path)` with `.creation_flags(0x08000000)`, store result in `AppState.hw_encoder` |
| [app.py](../app.py) (`normalize_path`, `strip_long_prefix`) | String manipulation + `\\?\` prefix | `src-tauri/src/scanner.rs` | `dunce::simplified` / `std::fs::canonicalize` for display vs. extended-length path handling |
| [app.py](../app.py) (`GET /api/scan`) | `os.walk` + `get_video_info` (`ffprobe`) | `src-tauri/src/scanner.rs` (`#[tauri::command] async fn scan_videos`) | `walkdir::WalkDir` + async `ffprobe` JSON parsing (`tokio::process::Command`), sorted by `size_gb` descending |
| [app.py](../app.py) (`GET /api/browse-directory`) | `tkinter.filedialog.askdirectory` in thread | `src/js/main.js` or `src-tauri/src/files.rs` | `@tauri-apps/plugin-dialog` `open({ directory: true, title: "Choisir le répertoire à scanner" })` |
| [app.py](../app.py) (`WS /ws/compress/{video_index}`) | FastAPI WebSocket + `subprocess.Popen` + `stderr` thread | `src-tauri/src/compressor.rs` (`#[tauri::command] async fn start_compression`) | `tokio::process::Command` reading `stderr` lines with `tokio::io::BufReader`, parsing `out_time_ms=(\d+)`, emitting `compression://event` |
| [app.py](../app.py) (`POST /api/stop-compression`) | `active_processes[video_index].terminate()` / `.kill()` | `src-tauri/src/compressor.rs` (`#[tauri::command] async fn stop_compression`) | Look up `video_index` in `Mutex<HashMap<usize, Child>>`, kill process, delete `.tmp.mp4`, emit cancelled status |
| [app.py](../app.py) (`POST /api/delete`, `/api/delete-batch`) | `send2trash(file_path)` | `src-tauri/src/files.rs` (`delete_file`, `delete_batch`) | Validate `.mp4` extension & existence, call `trash::delete(&path)` |
| [app.py](../app.py) (`POST /api/open-folder`) | `subprocess.Popen(['explorer', f'/select,{file_path}'])` | `src-tauri/src/files.rs` (`open_folder`) | `tauri_plugin_opener::reveal_item_in_dir(&path)` (or `explorer.exe /select,`) |
| [app.py](../app.py) (`POST /api/play-video`) | `os.startfile(file_path)` | `src-tauri/src/files.rs` (`play_video`) | `tauri_plugin_opener::open_path(&path, None::<&str>)` |
| [app.py](../app.py) (`WS /ws/logs` + `_PrintInterceptor`) | Redirects `sys.stdout`/`stderr` to WebSocket clients | `src-tauri/src/logger.rs` | `emit_log(app_handle, level, msg)` emitting `server://log` events directly to the UI log panel |

---

## 3. Phased Execution Plan

### Phase 0: Prerequisites & Key Generation
1. **Verify Rust & Node Toolchains**:
   - Ensure Rust (`rustup`, `cargo`, MSVC build tools) and Node.js (v18+) are installed on the build machine.
2. **Prepare Static FFmpeg Sidecar Binaries**:
   - Extract or copy `ffmpeg.exe` and `ffprobe.exe` into `src-tauri/binaries/`:
     - `src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe`
     - `src-tauri/binaries/ffprobe-x86_64-pc-windows-msvc.exe`
3. **Generate Tauri Updater Signing Keypair**:
   - Run `npx @tauri-apps/cli signer generate -w ~/.tauri/videoshrinker.key`.
   - Record the **public key** for `src-tauri/tauri.conf.json` and save the **private key** for GitHub Actions Secrets (`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).

---

### Phase 1: Scaffold Tauri v2 + Vite Frontend
1. **Initialize Root `package.json` & Vite**:
   - Create root `package.json` with scripts:
     - `"dev": "vite"`
     - `"build": "vite build"`
     - `"tauri": "tauri"`
   - Install frontend dependencies:
     ```powershell
     npm install @tauri-apps/api @tauri-apps/plugin-dialog @tauri-apps/plugin-opener @tauri-apps/plugin-process @tauri-apps/plugin-shell @tauri-apps/plugin-updater bootstrap
     npm install -D vite @tauri-apps/cli
     ```
2. **Move Frontend Assets to `src/`**:
   - Copy [templates/index.html](../templates/index.html) $\rightarrow$ `src/index.html`:
     - Replace Jinja2 `{{ default_directory }}` placeholder with an empty value populated on `DOMContentLoaded` via `invoke('get_app_info')`.
     - Add an auto-update notification banner container (`#updateBanner`) at the top of `.main-content`.
     - Import Bootstrap CSS/JS from local `node_modules/bootstrap` (or keep local vendor files) so the app works offline.
   - Copy [static/css/style.css](../static/css/style.css) $\rightarrow$ `src/css/style.css`.
   - Copy [static/js/app.js](../static/js/app.js) $\rightarrow$ `src/js/main.js`.

---

### Phase 2: Implement the Rust Backend (`src-tauri/`)
1. **Configure `src-tauri/Cargo.toml`**:
   ```toml
   [dependencies]
   tauri = { version = "2", features = [] }
   tauri-plugin-dialog = "2"
   tauri-plugin-opener = "2"
   tauri-plugin-process = "2"
   tauri-plugin-shell = "2"
   tauri-plugin-updater = "2"
   serde = { version = "1", features = ["derive"] }
   serde_json = "1"
   tokio = { version = "1", features = ["process", "io-util", "sync", "time", "macros"] }
   walkdir = "2"
   trash = "5"
   dunce = "1"
   regex = "1"
   ```
2. **Implement Modules**:
   - **`state.rs`**:
     - `pub struct AppState { pub hw_encoder: String, pub active_processes: Arc<Mutex<HashMap<usize, u32>>> }`
   - **`logger.rs`**:
     - `pub fn log_info(app: &AppHandle, msg: impl Into<String>)` and `log_error(...)` emitting `server://log` with `{ level, msg }`.
   - **`scanner.rs`**:
     - Port `scan_videos(directory, min_size_gb)`:
       - Normalize quotes and path separators; support Windows long paths (`\\?\`) while returning clean display paths via `dunce::simplified`.
       - Walk directory for `.mp4` files, compute `size_gb`, check if `<stem>_compressed.mp4` exists.
       - Execute `ffprobe` sidecar (`-v quiet -print_format json -show_format -show_streams`) with `CREATE_NO_WINDOW` to parse `duration`, `bit_rate`, and `width x height`.
       - Sort descending by `size_gb`.
   - **`compressor.rs`**:
     - Port `detect_hw_encoder()` (exact same candidate order: `h264_nvenc` $\rightarrow$ `h264_qsv` $\rightarrow$ `h264_amf` $\rightarrow$ `libx264`).
     - Port quality presets (`low`, `medium`, `high`), `hw_preset_map`, `hw_quality_map`, and FFmpeg arguments (`-ac 2 -c:a aac -b:a 128k -pix_fmt yuv420p -movflags +faststart -progress pipe:2 -y`).
     - Spawn `ffmpeg` with `CREATE_NO_WINDOW`, stream `stderr` lines asynchronously, parse `out_time_ms=(\d+)`, and emit `compression://event` whenever progress advances by $\ge 0.5\%$.
     - On exit code `0`, rename `<stem>_compressed.tmp.mp4` $\rightarrow$ `<stem>_compressed.mp4`, compute `original_size_gb`, `compressed_size_gb`, `reduction_percent`, and `processing_time`, and emit `status: "completed"`.
     - Implement `stop_compression(video_index)` and window-close cleanup hook to kill any running `ffmpeg` PID and remove leftover `.tmp.mp4` files.
   - **`files.rs`**:
     - `delete_file(file_path)` and `delete_batch(file_paths)` using `trash::delete` after verifying `.mp4` suffix.
     - `open_folder(file_path)` and `play_video(file_path)`.

---

### Phase 3: Port Frontend JS from `fetch` / `WebSocket` to Tauri IPC
Update `src/js/main.js` ([static/js/app.js](../static/js/app.js)):
1. **Replace HTTP `fetch` calls with `invoke(...)`**:
   - `browseDirectory()` $\rightarrow$ `open({ directory: true, multiple: false, title: 'Choisir le répertoire à scanner' })` (or `invoke('browse_directory')`).
   - `scanVideos()` $\rightarrow$ `invoke('scan_videos', { directory, minSizeGb: parseFloat(minSize) })`.
   - `openFolder(index)` $\rightarrow$ `invoke('open_folder', { filePath: video.path })`.
   - `playVideo(filePath)` $\rightarrow$ `invoke('play_video', { filePath })`.
   - `deleteOriginalFile(index)` $\rightarrow$ `invoke('delete_file', { filePath: video.path })`.
   - `executeBatchDelete(indices)` $\rightarrow$ `invoke('delete_batch', { filePaths })`.
   - `stopCompression(index)` $\rightarrow$ `invoke('stop_compression', { videoIndex: index })`.
2. **Replace WebSockets with `listen(...)`**:
   - `initLogWebSocket()` $\rightarrow$ `listen('server://log', (event) => appendLog(event.payload.level, event.payload.msg))` and set status badge to `Connected`.
   - `compressVideo()` & `compressVideoAsync()` $\rightarrow$ listen to `compression://event` filtered by `event.payload.video_index === index`, then call `invoke('start_compression', { videoIndex: index, inputPath: video.path, quality })`.

---

### Phase 4: Implement Automatic Updates (`tauri-plugin-updater`)
1. **Configure `src-tauri/tauri.conf.json`**:
   ```json
   {
     "productName": "VideoShrinker",
     "version": "1.1.0",
     "identifier": "com.lgo.videoshrinker",
     "bundle": {
       "active": true,
       "targets": ["nsis"],
       "createUpdaterArtifacts": true,
       "externalBin": [
         "binaries/ffmpeg",
         "binaries/ffprobe"
       ],
       "windows": {
         "nsis": {
           "installMode": "currentUser"
         }
       }
     },
     "plugins": {
       "updater": {
         "pubkey": "<YOUR_TAURI_UPDATER_PUBLIC_KEY>",
         "endpoints": [
           "https://github.com/<OWNER>/<REPO>/releases/latest/download/latest.json"
         ],
         "windows": {
           "installMode": "passive"
         }
       }
     }
   }
   ```
   *(Note: Setting `installMode: "currentUser"` for NSIS avoids requiring UAC Administrator elevation on every install and update.)*
2. **Configure Permissions in `src-tauri/capabilities/default.json`**:
   - Grant `core:default`, `dialog:default`, `opener:default`, `process:default`, `updater:default`, and `shell:allow-execute` for the `binaries/ffmpeg` and `binaries/ffprobe` sidecars.
3. **Create Frontend Update Module (`src/js/updater.js`)**:
   - On startup (`DOMContentLoaded`), call `check()` from `@tauri-apps/plugin-updater`.
   - If an update is available:
     - Display `#updateBanner` at the top of the window: *"🎉 Une nouvelle version (vX.Y.Z) est disponible !"*.
     - Provide a **"Mettre à jour et redémarrer"** button.
     - Show download progress (`contentLength` / `chunkLength` percentage) in the banner.
     - Once downloaded and installed, call `relaunch()` from `@tauri-apps/plugin-process`.
   - Also add a manual **"Vérifier les mises à jour"** button in the header/footer so users can check on demand.

---

### Phase 5: GitHub Actions Release Pipeline (`.github/workflows/release.yml`)
Create `.github/workflows/release.yml` triggered on `v*` tags:
1. Checkout repository on `windows-latest`.
2. Setup Node.js and Rust stable (`dtolnay/rust-toolchain@stable`).
3. Ensure `ffmpeg` and `ffprobe` binaries are placed in `src-tauri/binaries/`.
4. Run `tauri-apps/tauri-action@v0` with:
   - `env.GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`
   - `env.TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}`
   - `env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}`
   - `tagName: v__VERSION__`
   - `releaseName: "VideoShrinker v__VERSION__"`
5. Verify that the GitHub Release automatically contains:
   - `VideoShrinker_x.y.z_x64-setup.exe`
   - `VideoShrinker_x.y.z_x64-setup.exe.sig`
   - `latest.json`

---

### Phase 6: QA Validation (TNR Campaign) & Legacy Cleanup

#### 6.1 Non-Regression Test Checklist (QA Agent)
- [ ] **Startup & Encoder Probe**: App opens in `< 1s`; log panel immediately reports `[HW] Encodeur hardware détecté : ...`; no black `cmd.exe` flash occurs.
- [ ] **Browse Directory**: Clicking **📁 Parcourir…** opens the native Windows folder picker on top of the main window and populates `#scanDirectory`.
- [ ] **Recursive Scan & Long Paths**: Scanning a directory with paths $> 260$ characters finds all `.mp4` files, accurately reports `size_gb`, `duration`, `resolution`, `bitrate`, and detects existing `_compressed.mp4` files.
- [ ] **Single Compression (Low / Medium / High)**: Progress bar updates smoothly every $0.5\%$; log panel receives FFmpeg info/progress logs; `.tmp.mp4` is renamed to `_compressed.mp4` on completion; size reduction and processing time are displayed.
- [ ] **Stop / Cancel Compression**: Clicking **⏹️ Arrêter** during single or batch compression immediately kills `ffmpeg.exe` and deletes the partial `_compressed.tmp.mp4` file.
- [ ] **Window Close Cleanup**: Closing the app window while a compression is running terminates the child `ffmpeg.exe` process (no orphan process in Task Manager).
- [ ] **Play & Open Folder**: **▶️ Lire l'original**, **▶️ Lire compressé**, and **📂 Ouvrir le dossier** launch the default video player and Windows Explorer with the file selected.
- [ ] **Single & Batch Recycle Bin Deletion**: Deleting original files only works on `.mp4` files with a compressed counterpart and moves them to the **Windows Recycle Bin** (verified recoverable).
- [ ] **Auto-Update Flow**: Building `v1.0.0`, publishing `v1.0.1` to GitHub Releases, and launching `v1.0.0` displays the update banner, downloads `v1.0.1`, installs it, and relaunches into `v1.0.1`.

#### 6.2 Decommissioning Legacy Files (Post-Verification)
Once all TNR checks pass on Tauri v2:
- Remove Python/FastAPI/PyInstaller files: [app.py](../app.py), [app.py.backup](../app.py.backup), [requirements.txt](../requirements.txt), [VideoShrinker.spec](../VideoShrinker.spec), [build.bat](../build.bat), `build/`, `dist/`.
- Remove Electron wrapper files: [electron/](../electron), [build_electron.bat](../build_electron.bat), [build_all.bat](../build_all.bat), [run_electron.bat](../run_electron.bat).
- Remove old `templates/` and `static/` folders once migrated into `src/`.
- Update `.github/agents/*.agent.md` descriptions to reference the new Tauri v2 (`src/`, `src-tauri/`) structure.
