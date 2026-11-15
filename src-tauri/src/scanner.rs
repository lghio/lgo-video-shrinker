use crate::logger::{log_error, log_info};
use crate::state::{AppState, CREATE_NO_WINDOW};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, State};
use walkdir::WalkDir;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScanResponse {
    pub count: usize,
    pub total_size_gb: f64,
    pub videos: Vec<VideoItem>,
    pub scanned_directory: String,
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

pub fn scan_directory_sync(
    ffprobe_path: &Path,
    raw_dir: &str,
    min_size_gb: f64,
) -> Result<ScanResponse, String> {
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

    let mut videos: Vec<VideoItem> = Vec::new();

    for entry in WalkDir::new(&walk_root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let lower_name = file_name.to_lowercase();
        if !lower_name.ends_with(".mp4")
            || lower_name.ends_with(".tmp.mp4")
            || lower_name.ends_with("_compressed.mp4")
        {
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
            let path_obj = PathBuf::from(&file_path);
            let stem = path_obj
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let suffix = path_obj
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_else(|| ".mp4".to_string());
            let parent = path_obj.parent().unwrap_or_else(|| Path::new(""));
            let compressed_path_buf = parent.join(format!("{}_compressed{}", stem, suffix));
            let compressed_path_str = compressed_path_buf.to_string_lossy().to_string();
            let is_already_compressed = compressed_path_buf.exists();

            let (compressed_size_gb, reduction_percent) = if is_already_compressed {
                match std::fs::metadata(&compressed_path_buf) {
                    Ok(comp_meta) => {
                        let comp_bytes = comp_meta.len();
                        let comp_size_gb = get_file_size_gb(comp_bytes);
                        let reduction = if size_gb > 0.0 {
                            Some(round_1(((size_gb - comp_size_gb) / size_gb) * 100.0))
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

            let info = get_video_info(ffprobe_path, &file_path);
            let (duration, resolution, bitrate) = extract_metadata_from_probe(&info);

            let relative_path = compute_relative_path(&file_path, &scan_directory);

            videos.push(VideoItem {
                path: file_path,
                name: file_name,
                size_gb: round_2(size_gb),
                size_bytes,
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
            });
        }
    }

    videos.sort_by(|a, b| {
        b.size_gb
            .partial_cmp(&a.size_gb)
            .unwrap_or(std::cmp::Ordering::Equal)
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

    log_info(
        &app,
        format!(
            "[SCAN] Analyse du répertoire '{}' (taille min : {} GB)...",
            raw_dir, min_size
        ),
    );

    let result = tokio::task::spawn_blocking(move || {
        scan_directory_sync(&ffprobe_path, &raw_dir, min_size)
    })
    .await
    .map_err(|e| format!("Erreur interne lors du scan : {}", e))?;

    match &result {
        Ok(resp) => {
            log_info(
                &app,
                format!(
                    "[SCAN] {} vidéo(s) trouvée(s) ({} GB au total)",
                    resp.count, resp.total_size_gb
                ),
            );
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
}
