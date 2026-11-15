use crate::logger::{log_error, log_info};
use crate::state::{AppState, CREATE_NO_WINDOW};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};
use walkdir::WalkDir;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x00001000;
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x00040000;
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x00400000;
pub const MAX_SCAN_WORKERS: usize = 6;
pub const ALLOWED_VIDEO_EXTENSIONS: [&str; 5] = [".mp4", ".mov", ".mkv", ".avi", ".webm"];

/// Vérifie si un nom ou chemin de fichier correspond à un format vidéo supporté
/// (`.mp4`, `.mov`, `.mkv`, `.avi`, `.webm`) tout en excluant les fichiers temporaires
/// `.tmp.mp4` et les sorties déjà compressées `_compressed.mp4`.
pub fn is_supported_video_extension(path_or_name: &str) -> bool {
    let lower = path_or_name.trim().to_lowercase();
    if lower.ends_with(".tmp.mp4") || lower.ends_with("_compressed.mp4") {
        return false;
    }
    ALLOWED_VIDEO_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(ext))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VideoItem {
    pub path: String,
    pub name: String,
    pub size_gb: f64,
    pub size_bytes: u64,
    pub duration: Option<f64>,
    pub resolution: Option<String>,
    pub bitrate: Option<u64>,
    pub relative_path: String,
    pub is_already_compressed: bool,
    pub compressed_path: Option<String>,
    pub compressed_size_gb: Option<f64>,
    pub reduction_percent: Option<f64>,
    #[serde(default)]
    pub is_cloud_only: bool,
    #[serde(default)]
    pub is_low_bitrate: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScanResponse {
    pub count: usize,
    pub total_size_gb: f64,
    pub videos: Vec<VideoItem>,
    pub scanned_directory: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScanProgressPayload {
    pub current: usize,
    pub total: usize,
    pub current_file: String,
}

/// Détecte si les attributs de fichier Windows correspondent à un fichier OneDrive "En ligne uniquement" (Cloud-only placeholder).
pub fn is_cloud_only_attributes(file_attributes: u32) -> bool {
    (file_attributes
        & (FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS))
        != 0
}

/// Vérifie à partir des métadonnées système si le fichier est un placeholder Cloud-only OneDrive sans l'ouvrir.
pub fn is_metadata_cloud_only(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        is_cloud_only_attributes(metadata.file_attributes())
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

/// Détecte si une vidéo a déjà un bitrate très bas (< 1.5 Mbps en HD/standard, ou < 800 kbps si largeur <= 640).
pub fn is_low_bitrate_video(bitrate: Option<u64>, resolution: Option<&str>) -> bool {
    let Some(br) = bitrate else {
        return false;
    };
    if br == 0 {
        return false;
    }

    let width = resolution
        .and_then(|res| res.split('x').next())
        .and_then(|w| w.trim().parse::<u64>().ok());

    let threshold = match width {
        Some(w) if w > 0 && w <= 640 => 800_000,
        _ => 1_500_000,
    };

    br < threshold
}

/// Détermine si la piste audio principale est déjà au format AAC mono/stéréo avec un débit <= 135 kbps (~128 kbps)
/// et peut être copiée telle quelle (`-c:a copy`) sans ré-encodage.
pub fn can_passthrough_audio(info: &Value) -> bool {
    let Some(streams) = info.get("streams").and_then(|s| s.as_array()) else {
        return false;
    };

    let Some(audio_stream) = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|c| c.as_str()) == Some("audio"))
    else {
        return false;
    };

    let is_aac = audio_stream
        .get("codec_name")
        .and_then(|c| c.as_str())
        .map(|name| name.trim().eq_ignore_ascii_case("aac"))
        .unwrap_or(false);
    if !is_aac {
        return false;
    }

    let valid_channels = audio_stream
        .get("channels")
        .and_then(|c| c.as_u64())
        .map(|ch| (1..=2).contains(&ch))
        .unwrap_or(false);
    if !valid_channels {
        return false;
    }

    let bit_rate = audio_stream.get("bit_rate").and_then(|br| {
        br.as_str()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .or_else(|| br.as_u64())
    });

    matches!(bit_rate, Some(br) if br > 0 && br <= 135_000)
}

/// Supprime le préfixe `\\?\` des chemins longs Windows.
pub fn strip_long_prefix(p: &str) -> String {
    if let Some(stripped) = p.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        p.to_string()
    }
}

/// Normalise un chemin : supprime les guillemets parasites entourant le chemin,
/// unifie les séparateurs Windows, résout `.` et `..`, et active le préfixe `\\?\`
/// pour les chemins longs (> 240 caractères) sous Windows.
pub fn normalize_path(p: &str) -> String {
    let trimmed = p.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let unquoted = if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].trim()
    } else {
        trimmed
    };

    if unquoted.is_empty() {
        return String::new();
    }

    let has_long_prefix = unquoted.starts_with(r"\\?\");
    let body = if has_long_prefix {
        &unquoted[4..]
    } else {
        unquoted
    };

    let unified = body.replace('/', r"\");

    // Gérer le préfixe de lecteur (ex: "C:") ou UNC
    let (prefix, rest) = if unified.len() >= 2 && unified.as_bytes()[1] == b':' {
        (&unified[..2], &unified[2..])
    } else {
        ("", unified.as_str())
    };

    let is_abs = rest.starts_with('\\');
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                if !parts.is_empty() && parts.last() != Some(&"..") {
                    parts.pop();
                } else if !is_abs && prefix.is_empty() {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }

    let mut normalized = String::new();
    if !prefix.is_empty() {
        normalized.push_str(prefix);
        if is_abs || !parts.is_empty() {
            normalized.push('\\');
        }
        normalized.push_str(&parts.join(r"\"));
    } else if is_abs {
        normalized.push('\\');
        normalized.push_str(&parts.join(r"\"));
    } else if parts.is_empty() {
        normalized.push('.');
    } else {
        normalized.push_str(&parts.join(r"\"));
    }

    if has_long_prefix || (cfg!(windows) && normalized.len() > 240 && !normalized.starts_with(r"\\?\")) {
        format!(r"\\?\{}", normalized)
    } else {
        normalized
    }
}

pub fn get_file_size_gb(size_bytes: u64) -> f64 {
    (size_bytes as f64) / 1_073_741_824.0
}

pub fn round_1(val: f64) -> f64 {
    (val * 10.0).round() / 10.0
}

pub fn round_2(val: f64) -> f64 {
    (val * 100.0).round() / 100.0
}

/// Exécute `ffprobe` sans fenêtre console et retourne le JSON brut.
pub fn get_video_info(ffprobe_path: &Path, file_path: &str) -> Value {
    let mut cmd = Command::new(ffprobe_path);
    cmd.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        file_path,
    ]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.output() {
        Ok(output) if output.status.success() => {
            serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// Extrait `(duration, resolution, bitrate)` depuis le JSON retourné par `ffprobe`.
pub fn extract_metadata_from_probe(info: &Value) -> (Option<f64>, Option<String>, Option<u64>) {
    let mut duration: Option<f64> = None;
    let mut bitrate: Option<u64> = None;
    let mut resolution: Option<String> = None;

    if let Some(format_obj) = info.get("format") {
        if let Some(dur_val) = format_obj.get("duration") {
            duration = dur_val
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| dur_val.as_f64());
        }
        if let Some(br_val) = format_obj.get("bit_rate") {
            bitrate = br_val
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| br_val.as_u64());
        }
    }

    if let Some(streams) = info.get("streams").and_then(|s| s.as_array()) {
        for stream in streams {
            if stream.get("codec_type").and_then(|c| c.as_str()) == Some("video") {
                let width = stream.get("width").and_then(|w| w.as_u64());
                let height = stream.get("height").and_then(|h| h.as_u64());
                if let (Some(w), Some(h)) = (width, height) {
                    if w > 0 && h > 0 {
                        resolution = Some(format!("{}x{}", w, h));
                    }
                }
                break;
            }
        }
    }

    (duration, resolution, bitrate)
}

pub fn compute_relative_path(file_path: &str, base_dir: &str) -> String {
    let clean_file = strip_long_prefix(file_path);
    let clean_base = strip_long_prefix(base_dir);
    let file_p = Path::new(&clean_file);
    let base_p = Path::new(&clean_base);

    match file_p.strip_prefix(base_p) {
        Ok(rel) => rel.to_string_lossy().to_string(),
        Err(_) => clean_file,
    }
}

struct CandidateVideoEntry {
    file_path: String,
    file_name: String,
    size_bytes: u64,
    size_gb: f64,
    is_cloud_only: bool,
}

pub fn scan_directory_with_progress<F>(
    ffprobe_path: &Path,
    raw_dir: &str,
    min_size_gb: f64,
    cancel_flag: Option<&AtomicBool>,
    on_progress: F,
) -> Result<ScanResponse, String>
where
    F: Fn(usize, usize, &str) + Sync,
{
    let scan_directory = normalize_path(raw_dir);
    if scan_directory.is_empty() {
        return Err("Veuillez spécifier un répertoire à scanner.".to_string());
    }

    let walk_root = if cfg!(windows) && !scan_directory.starts_with(r"\\?\") {
        let candidate = format!(r"\\?\{}", scan_directory);
        if Path::new(&candidate).exists() {
            candidate
        } else {
            scan_directory.clone()
        }
    } else {
        scan_directory.clone()
    };

    if !Path::new(&walk_root).exists() && !Path::new(&scan_directory).exists() {
        return Err(format!(
            "Le répertoire '{}' n'existe pas ou est inaccessible.",
            strip_long_prefix(&scan_directory)
        ));
    }

    // Étape 1 : Collecte rapide des candidats éligibles via WalkDir
    let mut candidates: Vec<CandidateVideoEntry> = Vec::new();

    for entry in WalkDir::new(&walk_root).into_iter().filter_map(|e| e.ok()) {
        if cancel_flag.map_or(false, |f| f.load(Ordering::Relaxed)) {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !is_supported_video_extension(&file_name) {
            continue;
        }

        let raw_entry_path = entry.path().to_string_lossy().to_string();
        // Conserver le chemin sans préfixe \\?\ sauf si > 240 caractères (parité exacte avec app.py)
        let stripped_entry = strip_long_prefix(&raw_entry_path);
        let file_path = normalize_path(&stripped_entry);

        let metadata = match std::fs::metadata(entry.path()).or_else(|_| {
            entry
                .metadata()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        }) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size_bytes = metadata.len();
        let size_gb = get_file_size_gb(size_bytes);

        if size_gb >= min_size_gb {
            let is_cloud_only = is_metadata_cloud_only(&metadata);
            candidates.push(CandidateVideoEntry {
                file_path,
                file_name,
                size_bytes,
                size_gb,
                is_cloud_only,
            });
        }
    }

    let total_candidates = candidates.len();
    let videos_mutex: Mutex<Vec<VideoItem>> = Mutex::new(Vec::with_capacity(total_candidates));

    if total_candidates > 0 && !cancel_flag.map_or(false, |f| f.load(Ordering::Relaxed)) {
        on_progress(0, total_candidates, "");

        // Étape 2 : Extraction parallèle des métadonnées ffprobe (jusqu'à MAX_SCAN_WORKERS threads)
        let worker_count = total_candidates.min(MAX_SCAN_WORKERS).max(1);
        let next_idx = AtomicUsize::new(0);
        let done_count = AtomicUsize::new(0);
        let candidates_ref = &candidates;
        let scan_dir_ref = &scan_directory;
        let videos_ref = &videos_mutex;
        let progress_ref = &on_progress;

        std::thread::scope(|s| {
            for _ in 0..worker_count {
                let next_idx_ref = &next_idx;
                let done_count_ref = &done_count;
                s.spawn(move || {
                    let mut local_items: Vec<VideoItem> = Vec::new();
                    loop {
                        if cancel_flag.map_or(false, |f| f.load(Ordering::Relaxed)) {
                            break;
                        }
                        let idx = next_idx_ref.fetch_add(1, Ordering::Relaxed);
                        if idx >= total_candidates {
                            break;
                        }

                        let cand = &candidates_ref[idx];
                        let path_obj = PathBuf::from(&cand.file_path);
                        let stem = path_obj
                            .file_stem()
                            .map(|st| st.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let parent = path_obj.parent().unwrap_or_else(|| Path::new(""));
                        let compressed_path_buf =
                            parent.join(format!("{}_compressed.mp4", stem));
                        let compressed_path_str =
                            compressed_path_buf.to_string_lossy().to_string();
                        let is_already_compressed = compressed_path_buf.exists();

                        let (compressed_size_gb, reduction_percent) = if is_already_compressed {
                            match std::fs::metadata(&compressed_path_buf) {
                                Ok(comp_meta) => {
                                    let comp_bytes = comp_meta.len();
                                    let comp_size_gb = get_file_size_gb(comp_bytes);
                                    let reduction = if cand.size_gb > 0.0 {
                                        Some(round_1(
                                            ((cand.size_gb - comp_size_gb) / cand.size_gb) * 100.0,
                                        ))
                                    } else {
                                        None
                                    };
                                    (Some(round_2(comp_size_gb)), reduction)
                                }
                                Err(_) => (None, None),
                            }
                        } else {
                            (None, None)
                        };

                        // Ne jamais exécuter ffprobe sur un fichier OneDrive "Cloud-only" pour éviter son téléchargement forcé
                        let (duration, resolution, bitrate) = if cand.is_cloud_only {
                            (None, None, None)
                        } else {
                            let info = get_video_info(ffprobe_path, &cand.file_path);
                            extract_metadata_from_probe(&info)
                        };

                        let is_low_bitrate = !is_already_compressed
                            && !cand.is_cloud_only
                            && is_low_bitrate_video(bitrate, resolution.as_deref());

                        let relative_path = compute_relative_path(&cand.file_path, scan_dir_ref);

                        local_items.push(VideoItem {
                            path: cand.file_path.clone(),
                            name: cand.file_name.clone(),
                            size_gb: round_2(cand.size_gb),
                            size_bytes: cand.size_bytes,
                            duration,
                            resolution,
                            bitrate,
                            relative_path,
                            is_already_compressed,
                            compressed_path: if is_already_compressed {
                                Some(compressed_path_str)
                            } else {
                                None
                            },
                            compressed_size_gb,
                            reduction_percent,
                            is_cloud_only: cand.is_cloud_only,
                            is_low_bitrate,
                        });

                        let completed = done_count_ref.fetch_add(1, Ordering::SeqCst) + 1;
                        progress_ref(completed, total_candidates, &cand.file_name);
                    }

                    if !local_items.is_empty() {
                        if let Ok(mut guard) = videos_ref.lock() {
                            guard.extend(local_items);
                        }
                    }
                });
            }
        });
    }

    let mut videos = videos_mutex.into_inner().unwrap_or_default();

    // Étape 3 : Tri déterministe par taille décroissante (size_bytes puis nom)
    videos.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.name.cmp(&b.name))
    });

    let total_size_gb = round_2(videos.iter().map(|v| v.size_gb).sum());
    let count = videos.len();

    Ok(ScanResponse {
        count,
        total_size_gb,
        videos,
        scanned_directory: scan_directory,
    })
}

pub fn scan_directory_sync(
    ffprobe_path: &Path,
    raw_dir: &str,
    min_size_gb: f64,
) -> Result<ScanResponse, String> {
    scan_directory_with_progress(ffprobe_path, raw_dir, min_size_gb, None, |_, _, _| {})
}

#[tauri::command]
pub fn cancel_scan(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.scan_cancelled.store(true, Ordering::SeqCst);
    log_info(
        &app,
        "[SCAN] Arrêt du scan demandé par l'utilisateur.".to_string(),
    );
    Ok(())
}

#[tauri::command]
pub async fn scan_videos(
    app: AppHandle,
    state: State<'_, AppState>,
    directory: Option<String>,
    min_size_gb: Option<f64>,
) -> Result<ScanResponse, String> {
    let raw_dir = directory
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| state.default_directory.clone());
    let min_size = min_size_gb.unwrap_or(0.0);
    let ffprobe_path = state.ffprobe_path.clone();

    state.scan_cancelled.store(false, Ordering::SeqCst);
    let cancel_flag = state.scan_cancelled.clone();
    let cancel_flag_for_check = state.scan_cancelled.clone();
    let app_for_progress = app.clone();

    log_info(
        &app,
        format!(
            "[SCAN] Analyse du répertoire '{}' (taille min : {} GB)...",
            raw_dir, min_size
        ),
    );

    let result = tokio::task::spawn_blocking(move || {
        scan_directory_with_progress(
            &ffprobe_path,
            &raw_dir,
            min_size,
            Some(&cancel_flag),
            |current, total, current_file| {
                let _ = app_for_progress.emit(
                    "scan://progress",
                    ScanProgressPayload {
                        current,
                        total,
                        current_file: current_file.to_string(),
                    },
                );
            },
        )
    })
    .await
    .map_err(|e| format!("Erreur interne lors du scan : {}", e))?;

    match &result {
        Ok(resp) => {
            if cancel_flag_for_check.load(Ordering::Relaxed) {
                log_info(
                    &app,
                    format!(
                        "[SCAN] Scan interrompu par l'utilisateur : {} vidéo(s) retenue(s) ({} GB au total)",
                        resp.count, resp.total_size_gb
                    ),
                );
            } else {
                let cloud_count = resp.videos.iter().filter(|v| v.is_cloud_only).count();
                let low_br_count = resp.videos.iter().filter(|v| v.is_low_bitrate).count();
                log_info(
                    &app,
                    format!(
                        "[SCAN] {} vidéo(s) trouvée(s) ({} GB au total • Cloud-only OneDrive : {} • Déjà optimisées/bas bitrate : {})",
                        resp.count, resp.total_size_gb, cloud_count, low_br_count
                    ),
                );
            }
        }
        Err(err) => {
            log_error(&app, format!("[SCAN] Erreur : {}", err));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_strip_long_prefix() {
        assert_eq!(strip_long_prefix(r"\\?\C:\Videos\test.mp4"), r"C:\Videos\test.mp4");
        assert_eq!(strip_long_prefix(r"C:\Videos\test.mp4"), r"C:\Videos\test.mp4");
    }

    #[test]
    fn test_normalize_path_quotes_and_separators() {
        assert_eq!(
            normalize_path(r#"  "C:/Users/test/Videos/../Videos/clip.mp4"  "#),
            r"C:\Users\test\Videos\clip.mp4"
        );
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn test_normalize_path_long_windows_path() {
        let long_segment = "a".repeat(250);
        let raw = format!(r"C:\Users\test\{}\video.mp4", long_segment);
        let norm = normalize_path(&raw);
        if cfg!(windows) {
            assert!(norm.starts_with(r"\\?\C:\Users\test\"));
            assert_eq!(strip_long_prefix(&norm), raw);
        }
    }

    #[test]
    fn test_extract_metadata_from_probe() {
        let sample = json!({
            "format": {
                "duration": "125.5",
                "bit_rate": "4500000"
            },
            "streams": [
                { "codec_type": "audio" },
                { "codec_type": "video", "width": 1920, "height": 1080 }
            ]
        });
        let (duration, resolution, bitrate) = extract_metadata_from_probe(&sample);
        assert_eq!(duration, Some(125.5));
        assert_eq!(resolution.as_deref(), Some("1920x1080"));
        assert_eq!(bitrate, Some(4_500_000));
    }

    #[test]
    fn test_scan_directory_sync_finds_mp4_and_compressed_counterpart() {
        let dir = tempdir().unwrap();
        let video_a = dir.path().join("lesson1.mp4");
        let video_a_comp = dir.path().join("lesson1_compressed.mp4");
        let temp_artifact = dir.path().join("lesson2_compressed.tmp.mp4");
        let non_video = dir.path().join("notes.txt");

        let mut fa = File::create(&video_a).unwrap();
        fa.write_all(&vec![0u8; 2048]).unwrap();

        let mut fac = File::create(&video_a_comp).unwrap();
        fac.write_all(&vec![0u8; 1024]).unwrap();

        let mut ftmp = File::create(&temp_artifact).unwrap();
        ftmp.write_all(&vec![0u8; 512]).unwrap();

        let mut ftxt = File::create(&non_video).unwrap();
        ftxt.write_all(b"hello").unwrap();

        let resp = scan_directory_sync(
            Path::new("nonexistent_ffprobe"),
            &dir.path().to_string_lossy(),
            0.0,
        )
        .unwrap();

        // Doit trouver uniquement lesson1.mp4 et exclure lesson1_compressed.mp4, .tmp.mp4 et .txt
        assert_eq!(resp.count, 1);
        let lesson1 = resp.videos.iter().find(|v| v.name == "lesson1.mp4").unwrap();
        assert!(lesson1.is_already_compressed);
        assert!(lesson1.compressed_path.is_some());
        assert!(lesson1.compressed_size_gb.is_some());
        assert_eq!(lesson1.reduction_percent, Some(50.0));
    }

    #[test]
    fn test_scan_directory_sync_excludes_compressed_case_insensitive_and_calculates_gain() {
        let dir = tempdir().unwrap();
        let source_video = dir.path().join("moduleA.mp4");
        let source_comp = dir.path().join("moduleA_compressed.mp4");
        let uppercase_comp = dir.path().join("orphan_COMPRESSED.MP4");
        let mixedcase_comp = dir.path().join("other_Compressed.Mp4");

        File::create(&source_video)
            .unwrap()
            .write_all(&vec![0u8; 4000])
            .unwrap();
        File::create(&source_comp)
            .unwrap()
            .write_all(&vec![0u8; 1000])
            .unwrap();
        File::create(&uppercase_comp)
            .unwrap()
            .write_all(&vec![0u8; 2000])
            .unwrap();
        File::create(&mixedcase_comp)
            .unwrap()
            .write_all(&vec![0u8; 2000])
            .unwrap();

        let resp = scan_directory_sync(
            Path::new("nonexistent_ffprobe"),
            &dir.path().to_string_lossy(),
            0.0,
        )
        .unwrap();

        assert_eq!(resp.count, 1);
        let item = &resp.videos[0];
        assert_eq!(item.name, "moduleA.mp4");
        assert!(item.is_already_compressed);
        assert!(item.compressed_size_gb.is_some());
        assert_eq!(item.reduction_percent, Some(75.0));
    }

    #[test]
    fn test_scan_directory_sync_windows_extended_long_paths() {
        let dir = tempdir().unwrap();
        let mut deep_dir = dir.path().to_path_buf();
        for i in 0..6 {
            deep_dir = deep_dir.join(format!(
                "sous_dossier_tres_long_numero_{}_{}",
                i,
                "x".repeat(35)
            ));
        }

        let deep_dir_norm = normalize_path(&deep_dir.to_string_lossy());
        std::fs::create_dir_all(&deep_dir_norm).unwrap();

        let video_long = PathBuf::from(&deep_dir_norm).join("cours_long_path.mp4");
        let video_long_comp = PathBuf::from(&deep_dir_norm).join("cours_long_path_compressed.mp4");
        let video_long_tmp =
            PathBuf::from(&deep_dir_norm).join("cours_long_path_compressed.tmp.mp4");

        assert!(
            video_long.to_string_lossy().len() > 260,
            "Test path should exceed 260 chars"
        );

        File::create(&video_long)
            .unwrap()
            .write_all(&vec![0u8; 4096])
            .unwrap();
        File::create(&video_long_comp)
            .unwrap()
            .write_all(&vec![0u8; 2048])
            .unwrap();
        File::create(&video_long_tmp)
            .unwrap()
            .write_all(&vec![0u8; 512])
            .unwrap();

        let resp = scan_directory_sync(
            Path::new("nonexistent_ffprobe"),
            &dir.path().to_string_lossy(),
            0.0,
        )
        .unwrap();

        assert_eq!(resp.count, 1);
        let found = resp
            .videos
            .iter()
            .find(|v| v.name == "cours_long_path.mp4")
            .expect("Should find cours_long_path.mp4 inside >260 char directory");
        assert!(found.is_already_compressed);
        assert!(found.compressed_path.is_some());
        assert!(found.compressed_size_gb.is_some());
        assert_eq!(found.reduction_percent, Some(50.0));
    }

    #[test]
    fn test_is_cloud_only_attributes() {
        // Normal / Archive attributes
        assert!(!is_cloud_only_attributes(0));
        assert!(!is_cloud_only_attributes(0x00000020)); // FILE_ATTRIBUTE_ARCHIVE
        assert!(!is_cloud_only_attributes(0x00000080)); // FILE_ATTRIBUTE_NORMAL

        // OneDrive Cloud-only flags
        assert!(is_cloud_only_attributes(FILE_ATTRIBUTE_OFFLINE));
        assert!(is_cloud_only_attributes(FILE_ATTRIBUTE_RECALL_ON_OPEN));
        assert!(is_cloud_only_attributes(FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS));
        assert!(is_cloud_only_attributes(
            0x00000020 | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
        ));
    }

    #[test]
    fn test_is_low_bitrate_video() {
        assert!(!is_low_bitrate_video(None, Some("1920x1080")));
        assert!(!is_low_bitrate_video(Some(0), Some("1920x1080")));

        // Standard / HD (> 640 width): threshold is 1_500_000 bps (1.5 Mbps)
        assert!(is_low_bitrate_video(Some(1_200_000), Some("1920x1080")));
        assert!(is_low_bitrate_video(Some(1_499_999), Some("1280x720")));
        assert!(!is_low_bitrate_video(Some(1_500_000), Some("1280x720")));
        assert!(!is_low_bitrate_video(Some(4_500_000), Some("1920x1080")));

        // Small resolution (width <= 640): threshold is 800_000 bps (800 kbps)
        assert!(is_low_bitrate_video(Some(600_000), Some("640x360")));
        assert!(!is_low_bitrate_video(Some(900_000), Some("640x360")));

        // Unknown resolution defaults to 1.5 Mbps threshold
        assert!(is_low_bitrate_video(Some(1_100_000), None));
        assert!(!is_low_bitrate_video(Some(2_000_000), None));
    }

    #[test]
    fn test_scan_directory_with_progress_parallel_and_cancellation() {
        let dir = tempdir().unwrap();
        let sizes = [1024usize, 8192, 4096, 2048, 16384, 512, 32768, 65536];
        for (i, sz) in sizes.iter().enumerate() {
            let file_path = dir.path().join(format!("clip_{}.mp4", i));
            File::create(&file_path)
                .unwrap()
                .write_all(&vec![0u8; *sz])
                .unwrap();
        }

        let cancel_flag = AtomicBool::new(false);
        let progress_events: Mutex<Vec<(usize, usize, String)>> = Mutex::new(Vec::new());

        let resp = scan_directory_with_progress(
            Path::new("nonexistent_ffprobe"),
            &dir.path().to_string_lossy(),
            0.0,
            Some(&cancel_flag),
            |current, total, file_name| {
                progress_events.lock().unwrap().push((
                    current,
                    total,
                    file_name.to_string(),
                ));
            },
        )
        .unwrap();

        assert_eq!(resp.count, sizes.len());
        // Check descending sort by size_bytes
        for pair in resp.videos.windows(2) {
            assert!(
                pair[0].size_bytes >= pair[1].size_bytes,
                "Expected descending size_bytes order: {} >= {}",
                pair[0].size_bytes,
                pair[1].size_bytes
            );
        }

        let events = progress_events.into_inner().unwrap();
        assert!(!events.is_empty());
        assert!(events.iter().any(|(cur, tot, _)| *cur == sizes.len() && *tot == sizes.len()));

        // Test cancellation flag stops scan
        let pre_cancelled = AtomicBool::new(true);
        let cancelled_resp = scan_directory_with_progress(
            Path::new("nonexistent_ffprobe"),
            &dir.path().to_string_lossy(),
            0.0,
            Some(&pre_cancelled),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(cancelled_resp.count, 0);
    }

    #[test]
    fn test_is_supported_video_extension_and_multiformat_scan() {
        assert!(is_supported_video_extension("video.mp4"));
        assert!(is_supported_video_extension("iphone_clip.MOV"));
        assert!(is_supported_video_extension("obs_record.mkv"));
        assert!(is_supported_video_extension("archive.Avi"));
        assert!(is_supported_video_extension("web_capture.WEBM"));

        assert!(!is_supported_video_extension("video_compressed.mp4"));
        assert!(!is_supported_video_extension("VIDEO_COMPRESSED.MP4"));
        assert!(!is_supported_video_extension("video_compressed.tmp.mp4"));
        assert!(!is_supported_video_extension("document.pdf"));
        assert!(!is_supported_video_extension("audio.mp3"));

        let dir = tempdir().unwrap();
        let mov_file = dir.path().join("iphone.MOV");
        let mov_comp = dir.path().join("iphone_compressed.mp4");
        let mkv_file = dir.path().join("recording.mkv");
        let avi_file = dir.path().join("old_clip.avi");
        let webm_file = dir.path().join("browser.webm");
        let ignored_doc = dir.path().join("readme.txt");

        File::create(&mov_file)
            .unwrap()
            .write_all(&vec![0u8; 4000])
            .unwrap();
        File::create(&mov_comp)
            .unwrap()
            .write_all(&vec![0u8; 1000])
            .unwrap();
        File::create(&mkv_file)
            .unwrap()
            .write_all(&vec![0u8; 3000])
            .unwrap();
        File::create(&avi_file)
            .unwrap()
            .write_all(&vec![0u8; 2000])
            .unwrap();
        File::create(&webm_file)
            .unwrap()
            .write_all(&vec![0u8; 1500])
            .unwrap();
        File::create(&ignored_doc)
            .unwrap()
            .write_all(b"ignore me")
            .unwrap();

        let resp = scan_directory_sync(
            Path::new("nonexistent_ffprobe"),
            &dir.path().to_string_lossy(),
            0.0,
        )
        .unwrap();

        // Doit détecter les 4 vidéos sources (.MOV, .mkv, .avi, .webm) et exclure iphone_compressed.mp4 + readme.txt
        assert_eq!(resp.count, 4);

        let mov_item = resp
            .videos
            .iter()
            .find(|v| v.name == "iphone.MOV")
            .expect("iphone.MOV should be in scan results");
        assert!(mov_item.is_already_compressed);
        assert!(mov_item
            .compressed_path
            .as_deref()
            .unwrap_or("")
            .ends_with("iphone_compressed.mp4"));
        assert_eq!(mov_item.reduction_percent, Some(75.0));

        let mkv_item = resp
            .videos
            .iter()
            .find(|v| v.name == "recording.mkv")
            .expect("recording.mkv should be in scan results");
        assert!(!mkv_item.is_already_compressed);
    }

    #[test]
    fn test_can_passthrough_audio() {
        // AAC stéréo 128 kbps (string) -> true
        let aac_stereo_128k = json!({
            "streams": [
                { "codec_type": "video", "codec_name": "h264" },
                { "codec_type": "audio", "codec_name": "aac", "channels": 2, "bit_rate": "128000" }
            ]
        });
        assert!(can_passthrough_audio(&aac_stereo_128k));

        // AAC mono 96 kbps (u64, majuscules) -> true
        let aac_mono_96k = json!({
            "streams": [
                { "codec_type": "audio", "codec_name": "AAC", "channels": 1, "bit_rate": 96_000 }
            ]
        });
        assert!(can_passthrough_audio(&aac_mono_96k));

        // AAC 5.1 (6 canaux) -> false
        let aac_surround = json!({
            "streams": [
                { "codec_type": "audio", "codec_name": "aac", "channels": 6, "bit_rate": "128000" }
            ]
        });
        assert!(!can_passthrough_audio(&aac_surround));

        // AAC haut débit (256 kbps > 135 kbps) -> false
        let aac_high_br = json!({
            "streams": [
                { "codec_type": "audio", "codec_name": "aac", "channels": 2, "bit_rate": "256000" }
            ]
        });
        assert!(!can_passthrough_audio(&aac_high_br));

        // Autre codec audio (mp3 / pcm_s16le / opus) -> false
        let mp3_stream = json!({
            "streams": [
                { "codec_type": "audio", "codec_name": "mp3", "channels": 2, "bit_rate": "128000" }
            ]
        });
        assert!(!can_passthrough_audio(&mp3_stream));

        // Bitrate manquant ou pas de flux audio -> false
        let no_bitrate = json!({
            "streams": [
                { "codec_type": "audio", "codec_name": "aac", "channels": 2 }
            ]
        });
        assert!(!can_passthrough_audio(&no_bitrate));

        let no_audio = json!({
            "streams": [
                { "codec_type": "video", "codec_name": "h264" }
            ]
        });
        assert!(!can_passthrough_audio(&no_audio));
    }
}
