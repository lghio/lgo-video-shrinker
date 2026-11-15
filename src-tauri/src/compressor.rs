use crate::logger::{log_error, log_info, log_progress, log_success};
use crate::scanner::{
    extract_metadata_from_probe, get_file_size_gb, get_video_info, normalize_path, round_2,
};
use crate::state::{kill_process_tree, ActiveCompression, AppState, CREATE_NO_WINDOW};
use regex::Regex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

#[derive(Serialize, Clone, Debug)]
pub struct CompressionEventPayload {
    pub video_index: usize,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_size_gb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compressed_size_gb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduction_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct StopResponse {
    pub status: String,
    pub message: String,
}

/// Détecte le meilleur encodeur H.264 matériel disponible :
/// `h264_nvenc` > `h264_qsv` > `h264_amf` > `libx264`.
pub fn detect_hw_encoder(ffmpeg_path: &Path) -> String {
    let candidates = ["h264_nvenc", "h264_qsv", "h264_amf"];

    for name in candidates {
        let mut cmd = Command::new(ffmpeg_path);
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=64x64:d=0.1",
            "-c:v",
            name,
            "-f",
            "null",
            "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        if let Ok(status) = cmd.status() {
            if status.success() {
                return name.to_string();
            }
        }
    }

    "libx264".to_string()
}

/// Construit la liste d'arguments FFmpeg en parité exacte avec `app.py`.
/// Retourne `(args, enc_preset, enc_quality)`.
pub fn build_ffmpeg_args(
    input_path: &str,
    temp_path: &str,
    quality: &str,
    encoder: &str,
) -> (Vec<String>, String, String) {
    let normalized_quality = match quality {
        "low" | "medium" | "high" => quality,
        _ => "medium",
    };

    let (cpu_preset, scale) = match normalized_quality {
        "low" => ("fast", "480:-2"),
        "high" => ("medium", "-2:-2"),
        _ => ("fast", "1280:-2"),
    };

    let enc_preset = match (encoder, cpu_preset) {
        ("h264_nvenc", "fast") => "p4",
        ("h264_nvenc", "medium") => "p5",
        ("h264_nvenc", _) => "p6",
        ("h264_qsv", "fast") => "faster",
        ("h264_qsv", "medium") => "medium",
        ("h264_qsv", _) => "slow",
        ("h264_amf", "fast") => "speed",
        ("h264_amf", "medium") => "balanced",
        ("h264_amf", _) => "quality",
        (_, "fast") => "fast",
        (_, "medium") => "fast",
        (_, _) => "medium",
    }
    .to_string();

    let enc_quality = match (encoder, normalized_quality) {
        ("h264_nvenc" | "h264_qsv" | "h264_amf", "low") => "32",
        ("h264_nvenc" | "h264_qsv" | "h264_amf", "high") => "22",
        ("h264_nvenc" | "h264_qsv" | "h264_amf", _) => "26",
        (_, "low") => "28",
        (_, "high") => "20",
        (_, _) => "23",
    }
    .to_string();

    let vf_filter = if scale == "-2:-2" {
        "format=yuv420p".to_string()
    } else {
        format!("scale={},format=yuv420p", scale)
    };

    let mut args: Vec<String> = vec!["-i".to_string(), input_path.to_string()];

    match encoder {
        "h264_nvenc" => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "h264_nvenc".to_string(),
                "-preset".to_string(),
                enc_preset.clone(),
                "-rc".to_string(),
                "vbr".to_string(),
                "-cq".to_string(),
                enc_quality.clone(),
                "-b:v".to_string(),
                "0".to_string(),
            ]);
        }
        "h264_qsv" => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "h264_qsv".to_string(),
                "-preset".to_string(),
                enc_preset.clone(),
                "-global_quality".to_string(),
                enc_quality.clone(),
            ]);
        }
        "h264_amf" => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "h264_amf".to_string(),
                "-quality".to_string(),
                enc_preset.clone(),
                "-qp_i".to_string(),
                enc_quality.clone(),
                "-qp_p".to_string(),
                enc_quality.clone(),
            ]);
        }
        _ => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "libx264".to_string(),
                "-preset".to_string(),
                enc_preset.clone(),
                "-crf".to_string(),
                enc_quality.clone(),
                "-threads".to_string(),
                "0".to_string(),
            ]);
        }
    }

    args.extend([
        "-ac".to_string(),
        "2".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "128k".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        "-progress".to_string(),
        "pipe:2".to_string(),
        "-y".to_string(),
        temp_path.to_string(),
    ]);

    (args, enc_preset, enc_quality)
}

/// Extrait la valeur numérique de `out_time_ms=(\d+)` (en microsecondes) depuis une ligne stderr FFmpeg.
pub fn parse_out_time_ms(line: &str) -> Option<u64> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"out_time_ms=(\d+)").unwrap());
    re.captures(line)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<u64>().ok())
}

/// Formate une durée en secondes sous la forme `Xh Ym Zs`, `Ym Zs`, ou `Zs`.
pub fn format_elapsed_time(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

fn emit_compression_event(app: &AppHandle, payload: CompressionEventPayload) {
    let _ = app.emit("compression://event", payload);
}

#[tauri::command]
pub async fn start_compression(
    app: AppHandle,
    state: State<'_, AppState>,
    video_index: usize,
    input_path: String,
    quality: String,
) -> Result<(), String> {
    let normalized_input = normalize_path(&input_path);
    log_info(
        &app,
        format!(
            "[DEBUG] Compression demandée pour: {} (qualité: {})",
            normalized_input, quality
        ),
    );

    if normalized_input.is_empty() || !Path::new(&normalized_input).exists() {
        let msg = "Fichier introuvable".to_string();
        log_error(&app, format!("[ERROR] {}: {}", msg, normalized_input));
        emit_compression_event(
            &app,
            CompressionEventPayload {
                video_index,
                status: "error".to_string(),
                progress: None,
                current_time: None,
                total_duration: None,
                output_path: None,
                original_size_gb: None,
                compressed_size_gb: None,
                reduction_percent: None,
                processing_time: None,
                message: Some(msg.clone()),
            },
        );
        return Err(msg);
    }

    let path_obj = PathBuf::from(&normalized_input);
    let stem = path_obj
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "video".to_string());
    let suffix = path_obj
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_else(|| ".mp4".to_string());
    let parent = path_obj.parent().unwrap_or_else(|| Path::new(""));

    let output_path_buf = parent.join(format!("{}_compressed{}", stem, suffix));
    let temp_path_buf = parent.join(format!("{}_compressed.tmp.mp4", stem));
    let output_path = output_path_buf.to_string_lossy().to_string();
    let temp_path = temp_path_buf.to_string_lossy().to_string();

    let encoder = state.hw_encoder.clone();
    let ffmpeg_path = state.ffmpeg_path.clone();
    let ffprobe_path = state.ffprobe_path.clone();

    let (args, enc_preset, enc_quality) =
        build_ffmpeg_args(&normalized_input, &temp_path, &quality, &encoder);

    log_info(
        &app,
        format!(
            "[INFO] Encodeur utilisé : {} | preset : {} | qualité : {}",
            encoder, enc_preset, enc_quality
        ),
    );

    let probe_input = normalized_input.clone();
    let info = tokio::task::spawn_blocking(move || get_video_info(&ffprobe_path, &probe_input))
        .await
        .unwrap_or(serde_json::Value::Null);
    let (duration_opt, _, _) = extract_metadata_from_probe(&info);
    let total_duration = duration_opt.unwrap_or(0.0);

    emit_compression_event(
        &app,
        CompressionEventPayload {
            video_index,
            status: "started".to_string(),
            progress: Some(0.0),
            current_time: Some(0.0),
            total_duration: Some(total_duration),
            output_path: Some(output_path.clone()),
            original_size_gb: None,
            compressed_size_gb: None,
            reduction_percent: None,
            processing_time: None,
            message: None,
        },
    );

    let mut cmd = TokioCommand::new(&ffmpeg_path);
    cmd.args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let start_instant = Instant::now();
    let mut child = cmd.spawn().map_err(|e| {
        let msg = format!("Impossible de lancer FFmpeg : {}", e);
        log_error(&app, format!("[ERROR] {}", msg));
        msg
    })?;

    let child_pid = child.id();
    log_info(
        &app,
        format!("[DEBUG] Processus FFmpeg lancé, PID: {:?}", child_pid),
    );

    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();

    if let Ok(mut map) = state.active_processes.lock() {
        map.insert(
            video_index,
            ActiveCompression {
                pid: child_pid,
                temp_path: temp_path_buf.clone(),
                cancel_tx: Some(cancel_tx),
            },
        );
    }

    let stderr = child.stderr.take();
    let app_for_stderr = app.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(stderr_stream) = stderr {
            let mut reader = BufReader::new(stderr_stream).lines();
            let mut line_count: usize = 0;
            let mut last_progress_sent: f64 = -1.0;

            while let Ok(Some(line)) = reader.next_line().await {
                line_count += 1;
                if line_count <= 50 || line_count % 100 == 0 {
                    log_info(&app_for_stderr, format!("[STDERR #{}] {}", line_count, line));
                }

                if let Some(current_time_us) = parse_out_time_ms(&line) {
                    let current_time_s = (current_time_us as f64) / 1_000_000.0;
                    if total_duration > 0.0 {
                        let progress = ((current_time_s / total_duration) * 100.0).min(100.0);
                        if (progress - last_progress_sent).abs() >= 0.5 {
                            last_progress_sent = progress;
                            let rounded_progress = (progress * 10.0).round() / 10.0;
                            let rounded_time = (current_time_s * 10.0).round() / 10.0;

                            emit_compression_event(
                                &app_for_stderr,
                                CompressionEventPayload {
                                    video_index,
                                    status: "progress".to_string(),
                                    progress: Some(rounded_progress),
                                    current_time: Some(rounded_time),
                                    total_duration: Some(total_duration),
                                    output_path: None,
                                    original_size_gb: None,
                                    compressed_size_gb: None,
                                    reduction_percent: None,
                                    processing_time: None,
                                    message: None,
                                },
                            );
                            log_progress(
                                &app_for_stderr,
                                format!("[PROGRESS] {}% envoyé", rounded_progress),
                            );
                        }
                    }
                }
            }
        }
    });

    let mut cancelled = false;
    let status_result = tokio::select! {
        res = child.wait() => res,
        _ = &mut cancel_rx => {
            cancelled = true;
            if let Some(pid) = child_pid {
                kill_process_tree(pid);
            }
            let _ = child.kill().await;
            child.wait().await
        }
    };

    let _ = stderr_task.await;

    // Retirer le processus de la table active
    let was_removed_externally = if let Ok(mut map) = state.active_processes.lock() {
        map.remove(&video_index).is_none()
    } else {
        false
    };

    if cancelled || was_removed_externally {
        if temp_path_buf.exists() {
            let _ = std::fs::remove_file(&temp_path_buf);
            log_info(
                &app,
                format!(
                    "[DEBUG] Fichier temporaire supprimé après arrêt : {}",
                    temp_path
                ),
            );
        }
        emit_compression_event(
            &app,
            CompressionEventPayload {
                video_index,
                status: "cancelled".to_string(),
                progress: None,
                current_time: None,
                total_duration: None,
                output_path: None,
                original_size_gb: None,
                compressed_size_gb: None,
                reduction_percent: None,
                processing_time: None,
                message: Some("Compression annulée".to_string()),
            },
        );
        return Ok(());
    }

    match status_result {
        Ok(exit_status) if exit_status.success() => {
            if temp_path_buf.exists() {
                if output_path_buf.exists() {
                    let _ = std::fs::remove_file(&output_path_buf);
                }
                if let Err(rename_err) = std::fs::rename(&temp_path_buf, &output_path_buf) {
                    let _ = std::fs::remove_file(&temp_path_buf);
                    let msg = format!("Erreur lors du renommage du fichier temporaire : {}", rename_err);
                    log_error(&app, format!("[ERROR] {}", msg));
                    emit_compression_event(
                        &app,
                        CompressionEventPayload {
                            video_index,
                            status: "error".to_string(),
                            progress: None,
                            current_time: None,
                            total_duration: None,
                            output_path: None,
                            original_size_gb: None,
                            compressed_size_gb: None,
                            reduction_percent: None,
                            processing_time: None,
                            message: Some(msg.clone()),
                        },
                    );
                    return Err(msg);
                }
            }

            if output_path_buf.exists() {
                let elapsed_secs = start_instant.elapsed().as_secs();
                let time_str = format_elapsed_time(elapsed_secs);

                let orig_bytes = std::fs::metadata(&normalized_input)
                    .map(|m| m.len())
                    .unwrap_or(1);
                let comp_bytes = std::fs::metadata(&output_path_buf)
                    .map(|m| m.len())
                    .unwrap_or(0);

                let original_size = get_file_size_gb(orig_bytes);
                let compressed_size = get_file_size_gb(comp_bytes);
                let reduction = if original_size > 0.0 {
                    ((original_size - compressed_size) / original_size) * 100.0
                } else {
                    0.0
                };
                let rounded_reduction = (reduction * 10.0).round() / 10.0;

                log_success(
                    &app,
                    format!(
                        "[SUCCESS] Compression réussie ! {:.2} GB -> {:.2} GB ({:.1}% en {})",
                        original_size, compressed_size, rounded_reduction, time_str
                    ),
                );

                emit_compression_event(
                    &app,
                    CompressionEventPayload {
                        video_index,
                        status: "completed".to_string(),
                        progress: Some(100.0),
                        current_time: None,
                        total_duration: None,
                        output_path: Some(output_path),
                        original_size_gb: Some(round_2(original_size)),
                        compressed_size_gb: Some(round_2(compressed_size)),
                        reduction_percent: Some(rounded_reduction),
                        processing_time: Some(time_str),
                        message: None,
                    },
                );
                Ok(())
            } else {
                let msg = "Le fichier de sortie n'a pas été créé".to_string();
                log_error(&app, format!("[ERROR] {}", msg));
                emit_compression_event(
                    &app,
                    CompressionEventPayload {
                        video_index,
                        status: "error".to_string(),
                        progress: None,
                        current_time: None,
                        total_duration: None,
                        output_path: None,
                        original_size_gb: None,
                        compressed_size_gb: None,
                        reduction_percent: None,
                        processing_time: None,
                        message: Some(msg.clone()),
                    },
                );
                Err(msg)
            }
        }
        Ok(exit_status) => {
            if temp_path_buf.exists() {
                let _ = std::fs::remove_file(&temp_path_buf);
            }
            let code = exit_status.code().unwrap_or(-1);
            let msg = format!("Erreur FFmpeg (code {})", code);
            log_error(&app, format!("[ERROR] {}", msg));
            emit_compression_event(
                &app,
                CompressionEventPayload {
                    video_index,
                    status: "error".to_string(),
                    progress: None,
                    current_time: None,
                    total_duration: None,
                    output_path: None,
                    original_size_gb: None,
                    compressed_size_gb: None,
                    reduction_percent: None,
                    processing_time: None,
                    message: Some(msg.clone()),
                },
            );
            Err(msg)
        }
        Err(wait_err) => {
            if temp_path_buf.exists() {
                let _ = std::fs::remove_file(&temp_path_buf);
            }
            let msg = format!("Erreur d'exécution FFmpeg : {}", wait_err);
            log_error(&app, format!("[ERROR] {}", msg));
            emit_compression_event(
                &app,
                CompressionEventPayload {
                    video_index,
                    status: "error".to_string(),
                    progress: None,
                    current_time: None,
                    total_duration: None,
                    output_path: None,
                    original_size_gb: None,
                    compressed_size_gb: None,
                    reduction_percent: None,
                    processing_time: None,
                    message: Some(msg.clone()),
                },
            );
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn stop_compression(
    app: AppHandle,
    state: State<'_, AppState>,
    video_index: usize,
) -> Result<StopResponse, String> {
    let active_opt = if let Ok(mut map) = state.active_processes.lock() {
        map.remove(&video_index)
    } else {
        None
    };

    if let Some(mut active) = active_opt {
        if let Some(tx) = active.cancel_tx.take() {
            let _ = tx.send(());
        }
        if let Some(pid) = active.pid {
            kill_process_tree(pid);
        }
        if active.temp_path.exists() {
            let _ = std::fs::remove_file(&active.temp_path);
        }
        log_info(
            &app,
            format!(
                "[INFO] Compression arrêtée par l'utilisateur pour la vidéo #{}",
                video_index
            ),
        );
        Ok(StopResponse {
            status: "success".to_string(),
            message: "Compression arrêtée avec succès".to_string(),
        })
    } else {
        Ok(StopResponse {
            status: "error".to_string(),
            message: "Aucune compression active pour cette vidéo".to_string(),
        })
    }
}

#[tauri::command]
pub async fn cleanup_all_compressions(state: State<'_, AppState>) -> Result<(), String> {
    state.cleanup_all();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_ffmpeg_args_libx264_presets() {
        let (args_low, preset_low, qual_low) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "low", "libx264");
        assert_eq!(preset_low, "fast");
        assert_eq!(qual_low, "28");
        assert!(args_low.contains(&"scale=480:-2,format=yuv420p".to_string()));
        assert!(args_low.contains(&"-threads".to_string()));

        let (args_med, preset_med, qual_med) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "medium", "libx264");
        assert_eq!(preset_med, "fast");
        assert_eq!(qual_med, "23");
        assert!(args_med.contains(&"scale=1280:-2,format=yuv420p".to_string()));

        let (args_high, preset_high, qual_high) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "high", "libx264");
        assert_eq!(preset_high, "fast");
        assert_eq!(qual_high, "20");
        assert!(args_high.contains(&"format=yuv420p".to_string()));
    }

    #[test]
    fn test_build_ffmpeg_args_hw_encoders() {
        let (nv_args, nv_preset, nv_qual) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "medium", "h264_nvenc");
        assert_eq!(nv_preset, "p4");
        assert_eq!(nv_qual, "26");
        assert!(nv_args.contains(&"-cq".to_string()));
        assert!(nv_args.contains(&"vbr".to_string()));

        let (qsv_args, qsv_preset, qsv_qual) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "high", "h264_qsv");
        assert_eq!(qsv_preset, "medium");
        assert_eq!(qsv_qual, "22");
        assert!(qsv_args.contains(&"-global_quality".to_string()));

        let (amf_args, amf_preset, amf_qual) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "low", "h264_amf");
        assert_eq!(amf_preset, "speed");
        assert_eq!(amf_qual, "32");
        assert!(amf_args.contains(&"-qp_i".to_string()));
        assert!(amf_args.contains(&"-qp_p".to_string()));
    }

    #[test]
    fn test_parse_out_time_ms() {
        assert_eq!(parse_out_time_ms("out_time_ms=15500000"), Some(15_500_000));
        assert_eq!(parse_out_time_ms("frame=120 fps=30.0"), None);
        assert_eq!(parse_out_time_ms("out_time_ms=N/A"), None);
    }

    #[test]
    fn test_format_elapsed_time() {
        assert_eq!(format_elapsed_time(45), "45s");
        assert_eq!(format_elapsed_time(125), "2m 5s");
        assert_eq!(format_elapsed_time(3665), "1h 1m 5s");
    }

    #[test]
    fn test_detect_hw_encoder_with_bundled_ffmpeg() {
        let ffmpeg_path = crate::state::resolve_binary_path("ffmpeg");
        let detected = detect_hw_encoder(&ffmpeg_path);
        assert!(
            matches!(
                detected.as_str(),
                "h264_nvenc" | "h264_qsv" | "h264_amf" | "libx264"
            ),
            "Unexpected encoder detected: {}",
            detected
        );
    }

    #[test]
    fn test_end_to_end_synthetic_video_scan_probe_and_compress() {
        use crate::scanner::scan_directory_sync;
        use crate::state::resolve_binary_path;
        use tempfile::tempdir;

        let ffmpeg_path = resolve_binary_path("ffmpeg");
        let ffprobe_path = resolve_binary_path("ffprobe");
        assert!(
            ffmpeg_path.exists(),
            "Bundled ffmpeg binary must exist at {:?}",
            ffmpeg_path
        );
        assert!(
            ffprobe_path.exists(),
            "Bundled ffprobe binary must exist at {:?}",
            ffprobe_path
        );

        let dir = tempdir().unwrap();
        let input_mp4 = dir.path().join("synthetic_lesson.mp4");
        let temp_mp4 = dir.path().join("synthetic_lesson_compressed.tmp.mp4");
        let output_mp4 = dir.path().join("synthetic_lesson_compressed.mp4");

        // 1. Générer une vidéo MP4 synthétique de 1s (320x240) avec piste audio via le binaire ffmpeg embarqué
        let mut gen_cmd = Command::new(&ffmpeg_path);
        gen_cmd.args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1.0:size=320x240:rate=15",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1.0",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            "-y",
            &input_mp4.to_string_lossy(),
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            gen_cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let gen_out = gen_cmd.output().expect("Failed to execute bundled ffmpeg");
        assert!(
            gen_out.status.success(),
            "Synthetic MP4 generation failed: {}",
            String::from_utf8_lossy(&gen_out.stderr)
        );
        assert!(input_mp4.exists());

        // 2. Vérifier get_video_info (ffprobe) et extract_metadata_from_probe sur la vidéo générée
        let probe_json = get_video_info(&ffprobe_path, &input_mp4.to_string_lossy());
        let (duration, resolution, bitrate) = extract_metadata_from_probe(&probe_json);
        assert!(
            duration.unwrap_or(0.0) >= 0.8,
            "Expected duration >= 0.8s, got {:?}",
            duration
        );
        assert_eq!(resolution.as_deref(), Some("320x240"));
        assert!(bitrate.unwrap_or(0) > 0, "Expected non-zero bitrate");

        // 3. Vérifier scan_directory_sync avant compression
        let scan_before =
            scan_directory_sync(&ffprobe_path, &dir.path().to_string_lossy(), 0.0).unwrap();
        assert_eq!(scan_before.count, 1);
        assert_eq!(scan_before.videos[0].name, "synthetic_lesson.mp4");
        assert!(!scan_before.videos[0].is_already_compressed);
        assert!(scan_before.videos[0].compressed_path.is_none());
        assert_eq!(
            scan_before.videos[0].resolution.as_deref(),
            Some("320x240")
        );

        // 4. Exécuter la compression FFmpeg réelle via build_ffmpeg_args -> .tmp.mp4 puis renommage atomique
        let (args, preset, qual) = build_ffmpeg_args(
            &input_mp4.to_string_lossy(),
            &temp_mp4.to_string_lossy(),
            "medium",
            "libx264",
        );
        assert_eq!(preset, "fast");
        assert_eq!(qual, "23");

        let mut comp_cmd = Command::new(&ffmpeg_path);
        comp_cmd.args(&args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            comp_cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let comp_out = comp_cmd
            .output()
            .expect("Failed to run ffmpeg compression");
        assert!(
            comp_out.status.success(),
            "FFmpeg compression failed: {}",
            String::from_utf8_lossy(&comp_out.stderr)
        );
        assert!(temp_mp4.exists(), "Temporary .tmp.mp4 should exist before atomic rename");

        // Vérifier que la sortie stderr contient bien out_time_ms parseable
        let stderr_text = String::from_utf8_lossy(&comp_out.stderr);
        let parsed_times: Vec<u64> = stderr_text.lines().filter_map(parse_out_time_ms).collect();
        assert!(
            !parsed_times.is_empty(),
            "Expected at least one out_time_ms progress line in FFmpeg stderr"
        );

        // Renommage atomique .tmp.mp4 -> _compressed.mp4
        std::fs::rename(&temp_mp4, &output_mp4).expect("Atomic rename failed");
        assert!(!temp_mp4.exists());
        assert!(output_mp4.exists());
        assert!(std::fs::metadata(&output_mp4).unwrap().len() > 0);

        // 5. Valider le fichier compressé avec ffprobe
        let comp_probe = get_video_info(&ffprobe_path, &output_mp4.to_string_lossy());
        let (comp_dur, comp_res, _) = extract_metadata_from_probe(&comp_probe);
        assert!(comp_dur.unwrap_or(0.0) >= 0.8);
        assert!(comp_res.is_some());

        // 6. Vérifier scan_directory_sync après compression
        let scan_after =
            scan_directory_sync(&ffprobe_path, &dir.path().to_string_lossy(), 0.0).unwrap();
        let orig_item = scan_after
            .videos
            .iter()
            .find(|v| v.name == "synthetic_lesson.mp4")
            .expect("Original video should be present in scan");
        assert!(orig_item.is_already_compressed);
        assert!(orig_item
            .compressed_path
            .as_deref()
            .unwrap_or("")
            .ends_with("synthetic_lesson_compressed.mp4"));
    }

    #[test]
    fn test_cleanup_temp_file_on_cancellation() {
        use crate::state::{ActiveCompression, AppState};
        use std::collections::HashMap;
        use std::fs::File;
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let temp_mp4 = dir.path().join("in_progress_compressed.tmp.mp4");
        let mut f = File::create(&temp_mp4).unwrap();
        f.write_all(b"partial video bytes").unwrap();
        assert!(temp_mp4.exists());

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut map = HashMap::new();
        map.insert(
            0usize,
            ActiveCompression {
                pid: None,
                temp_path: temp_mp4.clone(),
                cancel_tx: Some(tx),
            },
        );

        let state = AppState {
            hw_encoder: "libx264".to_string(),
            ffmpeg_path: PathBuf::from("ffmpeg"),
            ffprobe_path: PathBuf::from("ffprobe"),
            default_directory: String::new(),
            active_processes: Arc::new(Mutex::new(map)),
        };

        state.cleanup_all();

        assert!(
            !temp_mp4.exists(),
            "Temporary .tmp.mp4 file must be deleted on cleanup/cancellation"
        );
        assert!(state.active_processes.lock().unwrap().is_empty());
    }
}
