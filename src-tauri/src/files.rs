use crate::logger::{log_error, log_info, log_success};
use crate::scanner::{normalize_path, strip_long_prefix, ALLOWED_VIDEO_EXTENSIONS};
use crate::state::CREATE_NO_WINDOW;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Serialize, Clone, Debug)]
pub struct BrowseResponse {
    pub status: String,
    pub path: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DeleteResponse {
    pub status: String,
    pub message: String,
    pub renamed_to: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct BatchDeleteItemResult {
    pub path: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub renamed_to: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct BatchDeleteResponse {
    pub status: String,
    pub success_count: usize,
    pub total: usize,
    pub results: Vec<BatchDeleteItemResult>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ActionResponse {
    pub status: String,
    pub message: String,
}

/// Valide qu'un chemin pointe vers un fichier vidéo supporté (`.mp4`, `.mov`, `.mkv`, `.avi`, `.webm`)
/// existant sur le disque avant envoi vers la corbeille Windows.
pub fn validate_video_for_deletion(raw_path: &str) -> Result<String, String> {
    let normalized = normalize_path(raw_path);
    if normalized.is_empty() {
        return Err("Chemin de fichier manquant".to_string());
    }
    let lower = normalized.to_lowercase();
    let is_allowed_video = ALLOWED_VIDEO_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(ext));
    if !is_allowed_video {
        return Err(
            "Seuls les fichiers vidéo (.mp4, .mov, .mkv, .avi, .webm) peuvent être supprimés"
                .to_string(),
        );
    }
    if !Path::new(&normalized).exists() {
        return Err("Fichier introuvable".to_string());
    }
    Ok(normalized)
}

/// Alias de compatibilité vers `validate_video_for_deletion`.
pub fn validate_mp4_for_deletion(raw_path: &str) -> Result<String, String> {
    validate_video_for_deletion(raw_path)
}

/// Calcule le chemin du fichier compressé correspondant `<parent>/<stem>_compressed.mp4`.
pub fn compute_compressed_counterpart(original_video_path: &Path) -> PathBuf {
    let stem = original_video_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = original_video_path.parent().unwrap_or_else(|| Path::new(""));
    parent.join(format!("{}_compressed.mp4", stem))
}

/// Renomme `<stem>_compressed.mp4` en `<stem>.mp4` une fois le fichier original envoyé à la corbeille.
pub fn rename_compressed_to_original(original_path: &Path) -> Result<Option<String>, String> {
    if original_path.exists() {
        return Err("Le fichier original existe encore sur le disque".to_string());
    }
    let compressed_path = compute_compressed_counterpart(original_path);
    if !compressed_path.exists() {
        return Ok(None);
    }
    let stem = original_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = original_path.parent().unwrap_or_else(|| Path::new(""));
    let target_path = parent.join(format!("{}.mp4", stem));

    std::fs::rename(&compressed_path, &target_path).map_err(|e| {
        format!(
            "Impossible de renommer '{}' en '{}' : {}",
            strip_long_prefix(&compressed_path.to_string_lossy()),
            strip_long_prefix(&target_path.to_string_lossy()),
            e
        )
    })?;
    Ok(Some(strip_long_prefix(&target_path.to_string_lossy())))
}

#[tauri::command]
pub async fn browse_directory(app: AppHandle) -> Result<BrowseResponse, String> {
    let app_clone = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_clone
            .dialog()
            .file()
            .set_title("Choisir le répertoire à scanner")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| format!("Erreur boîte de dialogue : {}", e))?;

    if let Some(folder) = picked {
        let folder_str = folder.to_string();
        Ok(BrowseResponse {
            status: "ok".to_string(),
            path: Some(folder_str),
        })
    } else {
        Ok(BrowseResponse {
            status: "cancelled".to_string(),
            path: None,
        })
    }
}

#[tauri::command]
pub async fn delete_file(
    app: AppHandle,
    file_path: String,
    rename_compressed: Option<bool>,
) -> Result<DeleteResponse, String> {
    let normalized = validate_video_for_deletion(&file_path)?;
    let clean_path = strip_long_prefix(&normalized);
    let should_rename = rename_compressed.unwrap_or(false);

    let target_for_trash = clean_path.clone();
    tokio::task::spawn_blocking(move || trash::delete(&target_for_trash))
        .await
        .map_err(|e| format!("Erreur interne corbeille : {}", e))?
        .map_err(|e| {
            let err_msg = format!("Impossible d'envoyer à la corbeille : {}", e);
            log_error(&app, format!("[ERROR] {}", err_msg));
            err_msg
        })?;

    let mut msg = format!("Fichier envoyé à la corbeille : {}", clean_path);
    log_success(&app, format!("[CORBEILLE] {}", msg));

    let mut renamed_to: Option<String> = None;
    if should_rename {
        match rename_compressed_to_original(Path::new(&normalized)) {
            Ok(Some(final_path)) => {
                log_success(
                    &app,
                    format!(
                        "[CORBEILLE] Fichier compressé renommé avec le nom original : {}",
                        final_path
                    ),
                );
                msg = format!(
                    "{} (fichier compressé renommé en {})",
                    msg, final_path
                );
                renamed_to = Some(final_path);
            }
            Ok(None) => {}
            Err(rename_err) => {
                log_error(&app, format!("[ERROR] {}", rename_err));
            }
        }
    }

    Ok(DeleteResponse {
        status: "success".to_string(),
        message: msg,
        renamed_to,
    })
}

#[tauri::command]
pub async fn delete_batch(
    app: AppHandle,
    file_paths: Vec<String>,
    rename_compressed: Option<bool>,
) -> Result<BatchDeleteResponse, String> {
    if file_paths.is_empty() {
        return Err("Aucun fichier à supprimer".to_string());
    }

    let total = file_paths.len();
    let should_rename = rename_compressed.unwrap_or(false);
    let app_clone = app.clone();

    let results = tokio::task::spawn_blocking(move || {
        let mut items = Vec::with_capacity(file_paths.len());
        for raw_path in file_paths {
            let normalized = match validate_video_for_deletion(&raw_path) {
                Ok(n) => n,
                Err(err_msg) => {
                    items.push(BatchDeleteItemResult {
                        path: raw_path,
                        status: "error".to_string(),
                        message: Some(err_msg),
                        renamed_to: None,
                    });
                    continue;
                }
            };

            let clean_path = strip_long_prefix(&normalized);
            match trash::delete(&clean_path) {
                Ok(_) => {
                    log_success(
                        &app_clone,
                        format!("[CORBEILLE] Fichier envoyé à la corbeille : {}", clean_path),
                    );
                    let mut renamed_to: Option<String> = None;
                    if should_rename {
                        match rename_compressed_to_original(Path::new(&normalized)) {
                            Ok(Some(final_path)) => {
                                log_success(
                                    &app_clone,
                                    format!(
                                        "[CORBEILLE] Fichier compressé renommé avec le nom original : {}",
                                        final_path
                                    ),
                                );
                                renamed_to = Some(final_path);
                            }
                            Ok(None) => {}
                            Err(rename_err) => {
                                log_error(&app_clone, format!("[ERROR] {}", rename_err));
                            }
                        }
                    }
                    items.push(BatchDeleteItemResult {
                        path: raw_path,
                        status: "success".to_string(),
                        message: None,
                        renamed_to,
                    });
                }
                Err(err) => {
                    log_error(
                        &app_clone,
                        format!(
                            "[ERROR] Échec envoi corbeille pour '{}' : {}",
                            clean_path, err
                        ),
                    );
                    items.push(BatchDeleteItemResult {
                        path: raw_path,
                        status: "error".to_string(),
                        message: Some(err.to_string()),
                        renamed_to: None,
                    });
                }
            }
        }
        items
    })
    .await
    .map_err(|e| format!("Erreur interne suppression en lot : {}", e))?;

    let success_count = results.iter().filter(|r| r.status == "success").count();
    log_info(
        &app,
        format!(
            "[CORBEILLE] Suppression en lot terminée : {}/{} fichier(s) envoyé(s) à la corbeille",
            success_count, total
        ),
    );

    Ok(BatchDeleteResponse {
        status: "done".to_string(),
        success_count,
        total,
        results,
    })
}

#[tauri::command]
pub async fn open_folder(file_path: String) -> Result<ActionResponse, String> {
    let normalized = normalize_path(&file_path);
    if normalized.is_empty() {
        return Err("Chemin de fichier manquant".to_string());
    }
    if !Path::new(&normalized).exists() {
        return Err("Fichier introuvable".to_string());
    }

    let clean_path = strip_long_prefix(&normalized);
    Command::new("explorer")
        .arg(format!("/select,{}", clean_path))
        .spawn()
        .map_err(|e| format!("Erreur lors de l'ouverture de l'explorateur : {}", e))?;

    let parent_dir = Path::new(&clean_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(ActionResponse {
        status: "success".to_string(),
        message: format!("Dossier ouvert : {}", parent_dir),
    })
}

#[tauri::command]
pub async fn play_video(app: AppHandle, file_path: String) -> Result<ActionResponse, String> {
    let normalized = normalize_path(&file_path);
    if normalized.is_empty() {
        return Err("Chemin de fichier manquant".to_string());
    }
    if !Path::new(&normalized).exists() {
        return Err("Fichier introuvable".to_string());
    }

    let clean_path = strip_long_prefix(&normalized);
    if app.opener().open_path(&clean_path, None::<&str>).is_err() {
        #[cfg(windows)]
        {
            Command::new("cmd")
                .args(["/C", "start", "", &clean_path])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| format!("Impossible d'ouvrir le lecteur vidéo : {}", e))?;
        }
    }

    Ok(ActionResponse {
        status: "success".to_string(),
        message: format!("Lecture de : {}", clean_path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_validate_mp4_for_deletion_rejects_empty_and_non_mp4() {
        assert_eq!(
            validate_mp4_for_deletion(""),
            Err("Chemin de fichier manquant".to_string())
        );
        assert_eq!(
            validate_mp4_for_deletion(r"C:\Videos\document.pdf"),
            Err(
                "Seuls les fichiers vidéo (.mp4, .mov, .mkv, .avi, .webm) peuvent être supprimés"
                    .to_string()
            )
        );
        assert_eq!(
            validate_mp4_for_deletion(r"C:\Videos\introuvable_12345.mp4"),
            Err("Fichier introuvable".to_string())
        );
    }

    #[test]
    fn test_validate_video_for_deletion_accepts_all_supported_formats() {
        let dir = tempdir().unwrap();
        for file_name in [
            "sample.MP4",
            "iphone.MOV",
            "capture.mkv",
            "legacy.AVI",
            "clip.WebM",
        ] {
            let video_path = dir.path().join(file_name);
            File::create(&video_path).unwrap();
            let validated = validate_video_for_deletion(&video_path.to_string_lossy());
            assert!(
                validated.is_ok(),
                "Expected '{}' to be accepted for deletion, got {:?}",
                file_name,
                validated
            );
        }
    }

    #[test]
    fn test_compute_compressed_counterpart() {
        let p = Path::new(r"C:\Videos\Cours\module_1.mp4");
        assert_eq!(
            compute_compressed_counterpart(p),
            PathBuf::from(r"C:\Videos\Cours\module_1_compressed.mp4")
        );

        let p_upper = Path::new(r"C:\Videos\CLIP.MP4");
        assert_eq!(
            compute_compressed_counterpart(p_upper),
            PathBuf::from(r"C:\Videos\CLIP_compressed.mp4")
        );

        let p_mov = Path::new(r"C:\Videos\iphone_record.MOV");
        assert_eq!(
            compute_compressed_counterpart(p_mov),
            PathBuf::from(r"C:\Videos\iphone_record_compressed.mp4")
        );
    }

    #[test]
    fn test_rename_compressed_to_original_renames_and_preserves_content() {
        use std::io::{Read, Write};

        let dir = tempdir().unwrap();
        let orig_path = dir.path().join("lesson1.mp4");
        let comp_path = dir.path().join("lesson1_compressed.mp4");

        // Tant que le fichier original existe encore sur le disque, le renommage est refusé
        File::create(&orig_path)
            .unwrap()
            .write_all(b"original-video-bytes")
            .unwrap();
        File::create(&comp_path)
            .unwrap()
            .write_all(b"compressed-video-bytes")
            .unwrap();

        assert!(rename_compressed_to_original(&orig_path).is_err());

        // Simuler que le fichier original a été envoyé à la corbeille (n'est plus dans le dossier)
        let trashed_simulation_path = dir.path().join("recycled_lesson1.bak");
        std::fs::rename(&orig_path, &trashed_simulation_path).unwrap();
        assert!(!orig_path.exists());

        // Renommer _compressed.mp4 avec le nom original
        let renamed = rename_compressed_to_original(&orig_path)
            .expect("rename_compressed_to_original should succeed")
            .expect("Expected Some(renamed_path)");

        assert_eq!(renamed, orig_path.to_string_lossy().to_string());
        assert!(orig_path.exists());
        assert!(!comp_path.exists());

        let mut content = String::new();
        File::open(&orig_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "compressed-video-bytes");

        // Tester également avec un fichier source .mov : <stem>_compressed.mp4 doit devenir <stem>.mp4
        let orig_mov = dir.path().join("iphone_clip.mov");
        let comp_mov_mp4 = dir.path().join("iphone_clip_compressed.mp4");
        let expected_final_mp4 = dir.path().join("iphone_clip.mp4");
        File::create(&comp_mov_mp4)
            .unwrap()
            .write_all(b"converted-mp4-bytes")
            .unwrap();

        let renamed_mov = rename_compressed_to_original(&orig_mov)
            .expect("rename_compressed_to_original for .mov should succeed")
            .expect("Expected Some(renamed_path) for .mov");
        assert_eq!(renamed_mov, expected_final_mp4.to_string_lossy().to_string());
        assert!(expected_final_mp4.exists());
        assert!(!comp_mov_mp4.exists());
    }

    #[test]
    fn test_rename_compressed_to_original_returns_none_when_counterpart_missing() {
        let dir = tempdir().unwrap();
        let missing_orig = dir.path().join("no_counterpart.mp4");
        let res = rename_compressed_to_original(&missing_orig).unwrap();
        assert!(res.is_none());
    }
}
