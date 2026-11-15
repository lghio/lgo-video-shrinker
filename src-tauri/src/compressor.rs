use crate::logger::{log_error, log_info, log_progress, log_success};
use crate::scanner::{
    can_passthrough_audio, extract_metadata_from_probe, get_file_size_gb, get_video_info,
    normalize_path, round_2,
};
use crate::state::{
    kill_process_tree, set_prevent_sleep, ActiveCompression, AppState, CREATE_NO_WINDOW,
};
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
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
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

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct GpuProbeInfo {
    pub gpu_name: Option<String>,
    pub is_nvidia: bool,
    pub is_rtx: bool,
    pub hw_encoder: String,
    pub hw_encoder_hevc: String,
    pub rtx_boost_enabled: bool,
}

/// Détecte le nom de la carte graphique NVIDIA (ou GPU principal) sans fenêtre console.
/// Priorise `nvidia-smi --query-gpu=name --format=csv,noheader`, puis fallback CIM Windows.
pub fn detect_nvidia_gpu_name() -> Option<String> {
    let mut smi_cmd = Command::new("nvidia-smi");
    smi_cmd
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        smi_cmd.creation_flags(CREATE_NO_WINDOW);
    }

    if let Ok(output) = smi_cmd.output() {
        if output.status.success() {
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = stdout_str.lines().map(|l| l.trim()).find(|l| !l.is_empty()) {
                return Some(first_line.to_string());
            }
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut ps_cmd = Command::new("powershell");
        ps_cmd
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);

        if let Ok(output) = ps_cmd.output() {
            if output.status.success() {
                let stdout_str = String::from_utf8_lossy(&output.stdout);
                let adapters: Vec<&str> = stdout_str
                    .lines()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .collect();

                if let Some(nvidia_adapter) = adapters.iter().find(|name| {
                    let upper = name.to_uppercase();
                    upper.contains("NVIDIA") || upper.contains("RTX") || upper.contains("GEFORCE")
                }) {
                    return Some((*nvidia_adapter).to_string());
                }

                if let Some(non_virtual) = adapters.iter().find(|name| {
                    let upper = name.to_uppercase();
                    !upper.contains("VIRTUAL")
                        && !upper.contains("BASIC DISPLAY")
                        && !upper.contains("REMOTE")
                }) {
                    return Some((*non_virtual).to_string());
                }

                if let Some(first) = adapters.first() {
                    return Some((*first).to_string());
                }
            }
        }
    }

    None
}

/// Détecte le meilleur encodeur H.264 matériel disponible :
/// `h264_nvenc` > `h264_qsv` > `h264_amf` > `libx264`.
/// Utilise une mire 256x256 car NVENC rejette les résolutions < ~145x49 (ex: 64x64).
pub fn detect_hw_encoder(ffmpeg_path: &Path) -> String {
    let candidates = ["h264_nvenc", "h264_qsv", "h264_amf"];

    for name in candidates {
        let mut cmd = Command::new(ffmpeg_path);
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=256x256:d=0.1",
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

/// Détecte le meilleur encodeur H.265 / HEVC matériel disponible :
/// `hevc_nvenc` > `hevc_qsv` > `hevc_amf` > `libx265`.
/// Utilise une mire 256x256 avec `CREATE_NO_WINDOW`.
pub fn detect_hw_encoder_hevc(ffmpeg_path: &Path) -> String {
    let candidates = ["hevc_nvenc", "hevc_qsv", "hevc_amf"];

    for name in candidates {
        let mut cmd = Command::new(ffmpeg_path);
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=256x256:d=0.1",
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

    "libx265".to_string()
}

/// Combine la détection du modèle GPU (NVIDIA / RTX) et le test des encodeurs matériels H.264 / HEVC FFmpeg.
pub fn probe_gpu_and_encoder(ffmpeg_path: &Path) -> GpuProbeInfo {
    let hw_encoder = detect_hw_encoder(ffmpeg_path);
    let hw_encoder_hevc = detect_hw_encoder_hevc(ffmpeg_path);
    let gpu_name = detect_nvidia_gpu_name();
    let upper_name = gpu_name.as_deref().unwrap_or("").to_uppercase();

    let is_nvidia = hw_encoder == "h264_nvenc"
        || hw_encoder_hevc == "hevc_nvenc"
        || upper_name.contains("NVIDIA")
        || upper_name.contains("GEFORCE")
        || upper_name.contains("RTX");
    let is_rtx = hw_encoder == "h264_nvenc" && upper_name.contains("RTX");
    let rtx_boost_enabled = hw_encoder == "h264_nvenc" && is_rtx;

    GpuProbeInfo {
        gpu_name,
        is_nvidia,
        is_rtx,
        hw_encoder,
        hw_encoder_hevc,
        rtx_boost_enabled,
    }
}

/// Alias de `probe_gpu_and_encoder`.
pub fn detect_gpu_info(ffmpeg_path: &Path) -> GpuProbeInfo {
    probe_gpu_and_encoder(ffmpeg_path)
}

/// Construit la liste d'arguments FFmpeg avec moteur NVENC avancé (RTX / NVIDIA) ou QSV/AMF/CPU
/// pour H.264 (`h264_nvenc`, `h264_qsv`, `h264_amf`, `libx264`) et H.265/HEVC (`hevc_nvenc`, `hevc_qsv`, `hevc_amf`, `libx265`),
/// avec option de copie directe du flux audio (`copy_audio = true` -> `-c:a copy`) et préservation des métadonnées (`-map_metadata 0`).
/// Retourne `(args, enc_preset, enc_quality)`.
pub fn build_ffmpeg_args_with_options(
    input_path: &str,
    temp_path: &str,
    quality: &str,
    encoder: &str,
    copy_audio: bool,
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

    let enc_preset = match (encoder, normalized_quality) {
        ("h264_nvenc" | "hevc_nvenc", "low") => "p4",
        ("h264_nvenc" | "hevc_nvenc", "medium") => "p4",
        ("h264_nvenc" | "hevc_nvenc", "high") => "p5",
        ("h264_nvenc" | "hevc_nvenc", _) => "p4",
        ("hevc_qsv", "low" | "medium") => "faster",
        ("hevc_qsv", "high") => "medium",
        ("hevc_qsv", _) => "faster",
        ("hevc_amf", "low" | "medium") => "speed",
        ("hevc_amf", "high") => "balanced",
        ("hevc_amf", _) => "speed",
        ("libx265", "low" | "medium") => "fast",
        ("libx265", "high") => "medium",
        ("libx265", _) => "fast",
        _ => match (encoder, cpu_preset) {
            ("h264_qsv", "fast") => "faster",
            ("h264_qsv", "medium") => "medium",
            ("h264_qsv", _) => "slow",
            ("h264_amf", "fast") => "speed",
            ("h264_amf", "medium") => "balanced",
            ("h264_amf", _) => "quality",
            (_, "fast") => "fast",
            (_, "medium") => "fast",
            (_, _) => "medium",
        },
    }
    .to_string();

    let enc_quality = match (encoder, normalized_quality) {
        ("hevc_nvenc" | "hevc_qsv" | "hevc_amf", "low") => "34",
        ("hevc_nvenc" | "hevc_qsv" | "hevc_amf", "high") => "24",
        ("hevc_nvenc" | "hevc_qsv" | "hevc_amf", _) => "28",
        ("libx265", "low") => "30",
        ("libx265", "high") => "22",
        ("libx265", _) => "25",
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

    let mut args: Vec<String> = Vec::new();
    if encoder == "h264_nvenc" || encoder == "hevc_nvenc" {
        args.extend(["-hwaccel".to_string(), "auto".to_string()]);
    }
    args.extend(["-i".to_string(), input_path.to_string()]);

    match encoder {
        "h264_nvenc" => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "h264_nvenc".to_string(),
                "-preset".to_string(),
                enc_preset.clone(),
                "-tune".to_string(),
                "hq".to_string(),
                "-rc".to_string(),
                "vbr".to_string(),
                "-cq".to_string(),
                enc_quality.clone(),
                "-b:v".to_string(),
                "0".to_string(),
                "-multipass".to_string(),
                "qres".to_string(),
                "-spatial-aq".to_string(),
                "1".to_string(),
                "-temporal-aq".to_string(),
                "1".to_string(),
            ]);
        }
        "hevc_nvenc" => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "hevc_nvenc".to_string(),
                "-preset".to_string(),
                enc_preset.clone(),
                "-tune".to_string(),
                "hq".to_string(),
                "-rc".to_string(),
                "vbr".to_string(),
                "-cq".to_string(),
                enc_quality.clone(),
                "-b:v".to_string(),
                "0".to_string(),
                "-multipass".to_string(),
                "qres".to_string(),
                "-spatial-aq".to_string(),
                "1".to_string(),
                "-temporal-aq".to_string(),
                "1".to_string(),
                "-tag:v".to_string(),
                "hvc1".to_string(),
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
        "hevc_qsv" => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "hevc_qsv".to_string(),
                "-preset".to_string(),
                enc_preset.clone(),
                "-global_quality".to_string(),
                enc_quality.clone(),
                "-tag:v".to_string(),
                "hvc1".to_string(),
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
        "hevc_amf" => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "hevc_amf".to_string(),
                "-quality".to_string(),
                enc_preset.clone(),
                "-qp_i".to_string(),
                enc_quality.clone(),
                "-qp_p".to_string(),
                enc_quality.clone(),
                "-tag:v".to_string(),
                "hvc1".to_string(),
            ]);
        }
        "libx265" => {
            args.extend([
                "-vf".to_string(),
                vf_filter,
                "-c:v".to_string(),
                "libx265".to_string(),
                "-preset".to_string(),
                enc_preset.clone(),
                "-crf".to_string(),
                enc_quality.clone(),
                "-tag:v".to_string(),
                "hvc1".to_string(),
                "-threads".to_string(),
                "0".to_string(),
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

    if copy_audio {
        args.extend(["-c:a".to_string(), "copy".to_string()]);
    } else {
        args.extend([
            "-ac".to_string(),
            "2".to_string(),
            "-c:a".to_string(),
            "aac".to_string(),
            "-b:a".to_string(),
            "128k".to_string(),
        ]);
    }

    args.extend([
        "-map_metadata".to_string(),
        "0".to_string(),
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

/// Construit la liste d'arguments FFmpeg par défaut (avec ré-encodage audio AAC 128k et `-map_metadata 0`).
pub fn build_ffmpeg_args(
    input_path: &str,
    temp_path: &str,
    quality: &str,
    encoder: &str,
) -> (Vec<String>, String, String) {
    build_ffmpeg_args_with_options(input_path, temp_path, quality, encoder, false)
}

/// Copie la date de modification (`mtime`) et, sous Windows, la date de création (`created`)
/// du fichier source `src_path` vers le fichier de destination `dst_path`.
pub fn preserve_file_timestamps(src_path: &Path, dst_path: &Path) -> Result<(), String> {
    let src_meta = std::fs::metadata(src_path)
        .map_err(|e| format!("Impossible de lire les métadonnées de {:?} : {}", src_path, e))?;

    let mut times = std::fs::FileTimes::new();
    if let Ok(mtime) = src_meta.modified() {
        times = times.set_modified(mtime);
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTimesExt;
        if let Ok(btime) = src_meta.created() {
            times = times.set_created(btime);
        }
    }

    let file = std::fs::File::options()
        .write(true)
        .open(dst_path)
        .map_err(|e| {
            format!(
                "Impossible d'ouvrir {:?} pour mettre à jour les dates : {}",
                dst_path, e
            )
        })?;

    file.set_times(times)
        .map_err(|e| format!("Impossible d'appliquer les dates sur {:?} : {}", dst_path, e))?;

    Ok(())
}

/// Extrait la valeur numérique de `out_time_ms=(\d+)` (en microsecondes) depuis une ligne stderr FFmpeg.
pub fn parse_out_time_ms(line: &str) -> Option<u64> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"out_time_ms=(\d+)").unwrap());
    re.captures(line)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<u64>().ok())
}

/// Extrait la vitesse d'encodage `speed=\s*([\d.]+)x` depuis une ligne stderr FFmpeg.
/// Ignore les valeurs nulles (`<= 0.0`) ou invalides.
pub fn parse_speed(line: &str) -> Option<f64> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"speed=\s*([\d.]+)x").unwrap());
    re.captures(line)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
        .filter(|&val| val > 0.0 && val.is_finite())
}

/// Calcule le temps restant estimé (ETA) en secondes pour la vidéo en cours.
pub fn compute_eta_seconds(
    current_time_s: f64,
    total_duration_s: f64,
    speed_x: Option<f64>,
    elapsed_s: f64,
) -> Option<u64> {
    if total_duration_s <= 0.0 || current_time_s >= total_duration_s {
        return Some(0);
    }

    let remaining = (total_duration_s - current_time_s).max(0.0);

    if let Some(s) = speed_x {
        if s > 0.05 {
            return Some((remaining / s).round() as u64);
        }
    }

    if current_time_s > 0.2 && elapsed_s > 0.1 {
        let effective_speed = current_time_s / elapsed_s;
        if effective_speed > 0.0 {
            return Some((remaining / effective_speed).round() as u64);
        }
    }

    None
}

/// Vérifie si le fichier compressé généré est plus lourd ou de même taille que l'original (protection anti-inflation / Smart Skip).
pub fn should_skip_inflated_output(orig_bytes: u64, comp_bytes: u64) -> bool {
    orig_bytes > 0 && comp_bytes >= orig_bytes
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
    codec: Option<String>,
) -> Result<(), String> {
    let normalized_input = normalize_path(&input_path);
    let requested_codec = codec
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| "h264".to_string());
    let use_hevc = matches!(requested_codec.as_str(), "hevc" | "h265");

    log_info(
        &app,
        format!(
            "[DEBUG] Compression demandée pour: {} (qualité: {}, codec: {})",
            normalized_input,
            quality,
            if use_hevc { "hevc" } else { "h264" }
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
                speed: None,
                eta_seconds: None,
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
    let parent = path_obj.parent().unwrap_or_else(|| Path::new(""));

    let output_path_buf = parent.join(format!("{}_compressed.mp4", stem));
    let temp_path_buf = parent.join(format!("{}_compressed.tmp.mp4", stem));
    let output_path = output_path_buf.to_string_lossy().to_string();
    let temp_path = temp_path_buf.to_string_lossy().to_string();

    let encoder = if use_hevc {
        state.hw_encoder_hevc.clone()
    } else {
        state.hw_encoder.clone()
    };
    let ffmpeg_path = state.ffmpeg_path.clone();
    let ffprobe_path = state.ffprobe_path.clone();

    let probe_input = normalized_input.clone();
    let info = tokio::task::spawn_blocking(move || get_video_info(&ffprobe_path, &probe_input))
        .await
        .unwrap_or(serde_json::Value::Null);
    let (duration_opt, _, _) = extract_metadata_from_probe(&info);
    let total_duration = duration_opt.unwrap_or(0.0);
    let copy_audio = can_passthrough_audio(&info);

    let (args, enc_preset, enc_quality) = build_ffmpeg_args_with_options(
        &normalized_input,
        &temp_path,
        &quality,
        &encoder,
        copy_audio,
    );

    log_info(
        &app,
        format!(
            "[INFO] Encodeur utilisé : {} | preset : {} | qualité : {}",
            encoder, enc_preset, enc_quality
        ),
    );
    if copy_audio {
        log_info(
            &app,
            "[INFO] Audio Passthrough actif (-c:a copy : piste AAC stéréo <= 128 kbps conservée sans ré-encodage)".to_string(),
        );
    }
    if encoder == "h264_nvenc" || encoder == "hevc_nvenc" {
        let gpu_label = state.gpu_name.as_deref().unwrap_or("NVIDIA GPU");
        log_info(
            &app,
            format!(
                "[INFO] Moteur NVENC avancé actif ({}) : -hwaccel auto | -preset {} -tune hq -rc vbr -cq {} -multipass qres -spatial-aq 1 -temporal-aq 1",
                gpu_label, enc_preset, enc_quality
            ),
        );
    }

    emit_compression_event(
        &app,
        CompressionEventPayload {
            video_index,
            status: "started".to_string(),
            progress: Some(0.0),
            current_time: Some(0.0),
            total_duration: Some(total_duration),
            speed: None,
            eta_seconds: None,
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
        set_prevent_sleep(true);
    }

    let stderr = child.stderr.take();
    let app_for_stderr = app.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(stderr_stream) = stderr {
            let mut reader = BufReader::new(stderr_stream).lines();
            let mut line_count: usize = 0;
            let mut last_progress_sent: f64 = -1.0;
            let mut latest_speed: Option<f64> = None;

            while let Ok(Some(line)) = reader.next_line().await {
                line_count += 1;
                if line_count <= 50 || line_count % 100 == 0 {
                    log_info(&app_for_stderr, format!("[STDERR #{}] {}", line_count, line));
                }

                if let Some(spd) = parse_speed(&line) {
                    latest_speed = Some(round_2(spd));
                }

                if let Some(current_time_us) = parse_out_time_ms(&line) {
                    let current_time_s = (current_time_us as f64) / 1_000_000.0;
                    if total_duration > 0.0 {
                        let progress = ((current_time_s / total_duration) * 100.0).min(100.0);
                        if (progress - last_progress_sent).abs() >= 0.5 {
                            last_progress_sent = progress;
                            let rounded_progress = (progress * 10.0).round() / 10.0;
                            let rounded_time = (current_time_s * 10.0).round() / 10.0;
                            let elapsed_s = start_instant.elapsed().as_secs_f64();
                            let eta_seconds = compute_eta_seconds(
                                current_time_s,
                                total_duration,
                                latest_speed,
                                elapsed_s,
                            );
                            let effective_speed = latest_speed.or_else(|| {
                                if current_time_s > 0.2 && elapsed_s > 0.1 {
                                    Some(round_2(current_time_s / elapsed_s))
                                } else {
                                    None
                                }
                            });

                            emit_compression_event(
                                &app_for_stderr,
                                CompressionEventPayload {
                                    video_index,
                                    status: "progress".to_string(),
                                    progress: Some(rounded_progress),
                                    current_time: Some(rounded_time),
                                    total_duration: Some(total_duration),
                                    speed: effective_speed,
                                    eta_seconds,
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
        let removed = map.remove(&video_index).is_none();
        if map.is_empty() {
            set_prevent_sleep(false);
        }
        removed
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
                speed: None,
                eta_seconds: None,
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
                let orig_bytes = std::fs::metadata(&normalized_input)
                    .map(|m| m.len())
                    .unwrap_or(0);
                let temp_comp_bytes = std::fs::metadata(&temp_path_buf)
                    .map(|m| m.len())
                    .unwrap_or(0);

                // Protection anti-inflation (Smart Skip) : si le fichier généré n'est pas plus léger que l'original
                if should_skip_inflated_output(orig_bytes, temp_comp_bytes) {
                    let _ = std::fs::remove_file(&temp_path_buf);
                    let elapsed_secs = start_instant.elapsed().as_secs();
                    let time_str = format_elapsed_time(elapsed_secs);
                    let original_size = get_file_size_gb(orig_bytes);
                    let compressed_size = get_file_size_gb(temp_comp_bytes);

                    log_info(
                        &app,
                        format!(
                            "[INFO] Compression ignorée (Smart Skip) : le fichier généré ({:.2} GB) n'est pas plus léger que l'original ({:.2} GB)",
                            compressed_size, original_size
                        ),
                    );

                    emit_compression_event(
                        &app,
                        CompressionEventPayload {
                            video_index,
                            status: "skipped_no_gain".to_string(),
                            progress: Some(100.0),
                            current_time: None,
                            total_duration: None,
                            speed: None,
                            eta_seconds: Some(0),
                            output_path: None,
                            original_size_gb: Some(round_2(original_size)),
                            compressed_size_gb: Some(round_2(compressed_size)),
                            reduction_percent: Some(0.0),
                            processing_time: Some(time_str),
                            message: Some(
                                "Compression ignorée : le fichier généré n'était pas plus léger que l'original (original conservé)"
                                    .to_string(),
                            ),
                        },
                    );
                    return Ok(());
                }

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
                            speed: None,
                            eta_seconds: None,
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
                let _ = preserve_file_timestamps(Path::new(&normalized_input), &output_path_buf);
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
                        speed: None,
                        eta_seconds: Some(0),
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
                        speed: None,
                        eta_seconds: None,
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
                    speed: None,
                    eta_seconds: None,
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
                    speed: None,
                    eta_seconds: None,
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
        let removed = map.remove(&video_index);
        if map.is_empty() {
            set_prevent_sleep(false);
        }
        removed
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
        let (nv_args_low, nv_preset_low, nv_qual_low) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "low", "h264_nvenc");
        assert_eq!(nv_preset_low, "p4");
        assert_eq!(nv_qual_low, "32");
        assert_eq!(&nv_args_low[0..4], &["-hwaccel", "auto", "-i", "in.mp4"]);

        let (nv_args, nv_preset, nv_qual) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "medium", "h264_nvenc");
        assert_eq!(nv_preset, "p4");
        assert_eq!(nv_qual, "26");
        assert_eq!(&nv_args[0..4], &["-hwaccel", "auto", "-i", "in.mp4"]);
        assert!(nv_args.contains(&"-tune".to_string()));
        assert!(nv_args.contains(&"hq".to_string()));
        assert!(nv_args.contains(&"-cq".to_string()));
        assert!(nv_args.contains(&"vbr".to_string()));
        assert!(nv_args.contains(&"-multipass".to_string()));
        assert!(nv_args.contains(&"qres".to_string()));
        assert!(nv_args.contains(&"-spatial-aq".to_string()));
        assert!(nv_args.contains(&"-temporal-aq".to_string()));

        let (_, nv_preset_high, nv_qual_high) =
            build_ffmpeg_args("in.mp4", "out.tmp.mp4", "high", "h264_nvenc");
        assert_eq!(nv_preset_high, "p5");
        assert_eq!(nv_qual_high, "22");

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
    fn test_build_ffmpeg_args_hevc_encoders() {
        let (nv_args_low, nv_preset_low, nv_qual_low) =
            build_ffmpeg_args("in.mov", "out.tmp.mp4", "low", "hevc_nvenc");
        assert_eq!(nv_preset_low, "p4");
        assert_eq!(nv_qual_low, "34");
        assert_eq!(&nv_args_low[0..4], &["-hwaccel", "auto", "-i", "in.mov"]);
        assert!(nv_args_low.contains(&"hevc_nvenc".to_string()));
        assert!(nv_args_low.contains(&"-tag:v".to_string()));
        assert!(nv_args_low.contains(&"hvc1".to_string()));

        let (nv_args_med, nv_preset_med, nv_qual_med) =
            build_ffmpeg_args("in.mov", "out.tmp.mp4", "medium", "hevc_nvenc");
        assert_eq!(nv_preset_med, "p4");
        assert_eq!(nv_qual_med, "28");
        assert!(nv_args_med.contains(&"-tune".to_string()));
        assert!(nv_args_med.contains(&"hq".to_string()));
        assert!(nv_args_med.contains(&"-multipass".to_string()));
        assert!(nv_args_med.contains(&"qres".to_string()));
        assert!(nv_args_med.contains(&"-spatial-aq".to_string()));
        assert!(nv_args_med.contains(&"-temporal-aq".to_string()));
        assert!(nv_args_med.contains(&"hvc1".to_string()));

        let (nv_args_high, nv_preset_high, nv_qual_high) =
            build_ffmpeg_args("in.mov", "out.tmp.mp4", "high", "hevc_nvenc");
        assert_eq!(nv_preset_high, "p5");
        assert_eq!(nv_qual_high, "24");
        assert!(nv_args_high.contains(&"hvc1".to_string()));

        let (qsv_args, qsv_preset, qsv_qual) =
            build_ffmpeg_args("in.mkv", "out.tmp.mp4", "high", "hevc_qsv");
        assert_eq!(qsv_preset, "medium");
        assert_eq!(qsv_qual, "24");
        assert!(qsv_args.contains(&"hevc_qsv".to_string()));
        assert!(qsv_args.contains(&"-global_quality".to_string()));
        assert!(qsv_args.contains(&"hvc1".to_string()));

        let (amf_args, amf_preset, amf_qual) =
            build_ffmpeg_args("in.avi", "out.tmp.mp4", "medium", "hevc_amf");
        assert_eq!(amf_preset, "speed");
        assert_eq!(amf_qual, "28");
        assert!(amf_args.contains(&"hevc_amf".to_string()));
        assert!(amf_args.contains(&"-qp_i".to_string()));
        assert!(amf_args.contains(&"-qp_p".to_string()));
        assert!(amf_args.contains(&"hvc1".to_string()));

        let (x265_args_low, x265_preset_low, x265_qual_low) =
            build_ffmpeg_args("in.webm", "out.tmp.mp4", "low", "libx265");
        assert_eq!(x265_preset_low, "fast");
        assert_eq!(x265_qual_low, "30");
        assert!(x265_args_low.contains(&"libx265".to_string()));
        assert!(x265_args_low.contains(&"hvc1".to_string()));
        assert!(x265_args_low.contains(&"-threads".to_string()));

        let (x265_args_med, x265_preset_med, x265_qual_med) =
            build_ffmpeg_args("in.webm", "out.tmp.mp4", "medium", "libx265");
        assert_eq!(x265_preset_med, "fast");
        assert_eq!(x265_qual_med, "25");
        assert!(x265_args_med.contains(&"hvc1".to_string()));

        let (x265_args_high, x265_preset_high, x265_qual_high) =
            build_ffmpeg_args("in.webm", "out.tmp.mp4", "high", "libx265");
        assert_eq!(x265_preset_high, "medium");
        assert_eq!(x265_qual_high, "22");
        assert!(x265_args_high.contains(&"hvc1".to_string()));
    }

    #[test]
    fn test_build_ffmpeg_args_audio_passthrough_and_map_metadata() {
        let (args_reencode, _, _) =
            build_ffmpeg_args_with_options("in.mp4", "out.tmp.mp4", "medium", "libx264", false);
        assert!(args_reencode.contains(&"-map_metadata".to_string()));
        assert!(args_reencode.contains(&"aac".to_string()));
        assert!(args_reencode.contains(&"128k".to_string()));
        assert!(!args_reencode.contains(&"copy".to_string()));

        let (args_copy, _, _) =
            build_ffmpeg_args_with_options("in.mp4", "out.tmp.mp4", "medium", "libx264", true);
        assert!(args_copy.contains(&"-map_metadata".to_string()));
        assert!(args_copy.windows(2).any(|w| w == ["-c:a", "copy"]));
        assert!(!args_copy.contains(&"128k".to_string()));
    }

    #[test]
    fn test_preserve_file_timestamps() {
        use std::fs::File;
        use std::io::Write;
        use std::time::{Duration, SystemTime};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let src = dir.path().join("original.mp4");
        let dst = dir.path().join("original_compressed.mp4");

        let mut f_src = File::create(&src).unwrap();
        f_src.write_all(b"source-content").unwrap();
        drop(f_src);

        // Fixer une date de modification ancienne déterministe sur src (ex: il y a ~24h)
        let custom_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let src_file = File::options().write(true).open(&src).unwrap();
        src_file
            .set_times(std::fs::FileTimes::new().set_modified(custom_mtime))
            .unwrap();
        drop(src_file);

        let mut f_dst = File::create(&dst).unwrap();
        f_dst.write_all(b"compressed-content").unwrap();
        drop(f_dst);

        preserve_file_timestamps(&src, &dst).expect("preserve_file_timestamps should succeed");

        let src_meta = std::fs::metadata(&src).unwrap();
        let dst_meta = std::fs::metadata(&dst).unwrap();
        assert_eq!(src_meta.modified().unwrap(), dst_meta.modified().unwrap());

        #[cfg(windows)]
        {
            assert_eq!(src_meta.created().unwrap(), dst_meta.created().unwrap());
        }
    }

    #[test]
    fn test_parse_out_time_ms() {
        assert_eq!(parse_out_time_ms("out_time_ms=15500000"), Some(15_500_000));
        assert_eq!(parse_out_time_ms("frame=120 fps=30.0"), None);
        assert_eq!(parse_out_time_ms("out_time_ms=N/A"), None);
    }

    #[test]
    fn test_parse_speed() {
        assert_eq!(parse_speed("speed=2.45x"), Some(2.45));
        assert_eq!(parse_speed("speed=  1.8x"), Some(1.8));
        assert_eq!(
            parse_speed("frame=120 fps=60.0 q=26.0 size=1024kB time=00:00:04.00 bitrate=2097.2kbits/s speed= 3.12x"),
            Some(3.12)
        );
        assert_eq!(parse_speed("speed=0.0x"), None);
        assert_eq!(parse_speed("speed=0x"), None);
        assert_eq!(parse_speed("speed=N/A"), None);
        assert_eq!(parse_speed("out_time_ms=15500000"), None);
    }

    #[test]
    fn test_compute_eta_seconds() {
        // Cas standard avec speed_x fourni (reste 75s à 2.5x -> 30s)
        assert_eq!(compute_eta_seconds(45.0, 120.0, Some(2.5), 18.0), Some(30));

        // Arrondi (reste 75s à 2.4x -> 31.25 -> 31s)
        assert_eq!(compute_eta_seconds(45.0, 120.0, Some(2.4), 19.0), Some(31));

        // Durée atteinte ou invalide -> Some(0)
        assert_eq!(compute_eta_seconds(120.0, 120.0, Some(2.0), 60.0), Some(0));
        assert_eq!(compute_eta_seconds(125.0, 120.0, Some(2.0), 60.0), Some(0));
        assert_eq!(compute_eta_seconds(10.0, 0.0, Some(2.0), 5.0), Some(0));

        // Fallback sur elapsed_s quand speed_x est None ou <= 0.05 (ex: 20s encodées en 10s -> 2.0x, reste 80s -> 40s)
        assert_eq!(compute_eta_seconds(20.0, 100.0, None, 10.0), Some(40));
        assert_eq!(compute_eta_seconds(20.0, 100.0, Some(0.01), 10.0), Some(40));

        // Tout début d'encodage sans speed_x et elapsed_s trop court -> None
        assert_eq!(compute_eta_seconds(0.1, 100.0, None, 0.05), None);
    }

    #[test]
    fn test_format_elapsed_time() {
        assert_eq!(format_elapsed_time(45), "45s");
        assert_eq!(format_elapsed_time(125), "2m 5s");
        assert_eq!(format_elapsed_time(3665), "1h 1m 5s");
    }

    #[test]
    fn test_should_skip_inflated_output() {
        // Inflated output (compressed > original) -> skip
        assert!(should_skip_inflated_output(10_000_000, 12_500_000));
        // Equal size (no gain) -> skip
        assert!(should_skip_inflated_output(10_000_000, 10_000_000));
        // Smaller output (real savings) -> do not skip
        assert!(!should_skip_inflated_output(10_000_000, 7_500_000));
        // Guard when original size is 0
        assert!(!should_skip_inflated_output(0, 1_000));
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
    fn test_detect_hw_encoder_hevc_with_bundled_ffmpeg() {
        let ffmpeg_path = crate::state::resolve_binary_path("ffmpeg");
        let detected_hevc = detect_hw_encoder_hevc(&ffmpeg_path);
        assert!(
            matches!(
                detected_hevc.as_str(),
                "hevc_nvenc" | "hevc_qsv" | "hevc_amf" | "libx265"
            ),
            "Unexpected HEVC encoder detected: {}",
            detected_hevc
        );
    }

    #[test]
    fn test_probe_gpu_and_encoder_with_bundled_ffmpeg() {
        let ffmpeg_path = crate::state::resolve_binary_path("ffmpeg");
        let probe = probe_gpu_and_encoder(&ffmpeg_path);
        println!("PROBED_GPU_INFO: {:?}", probe);
        assert!(
            matches!(
                probe.hw_encoder.as_str(),
                "h264_nvenc" | "h264_qsv" | "h264_amf" | "libx264"
            ),
            "Unexpected encoder in GpuProbeInfo: {}",
            probe.hw_encoder
        );
        assert!(
            matches!(
                probe.hw_encoder_hevc.as_str(),
                "hevc_nvenc" | "hevc_qsv" | "hevc_amf" | "libx265"
            ),
            "Unexpected HEVC encoder in GpuProbeInfo: {}",
            probe.hw_encoder_hevc
        );
        if probe.is_rtx {
            assert_eq!(probe.hw_encoder, "h264_nvenc");
            assert!(probe.is_nvidia);
            assert!(probe.rtx_boost_enabled);
            assert!(
                probe
                    .gpu_name
                    .as_deref()
                    .unwrap_or("")
                    .to_uppercase()
                    .contains("RTX")
            );
        } else {
            assert!(!probe.rtx_boost_enabled);
        }
    }

    #[test]
    fn test_end_to_end_nvenc_compression_with_build_ffmpeg_args() {
        use crate::state::resolve_binary_path;
        use tempfile::tempdir;

        let ffmpeg_path = resolve_binary_path("ffmpeg");
        let ffprobe_path = resolve_binary_path("ffprobe");
        let probe = probe_gpu_and_encoder(&ffmpeg_path);

        // If this machine has an NVIDIA GPU where h264_nvenc is available, test real E2E NVENC encoding
        if probe.hw_encoder == "h264_nvenc" {
            let dir = tempdir().unwrap();
            let input_mp4 = dir.path().join("rtx_source.mp4");
            let temp_mp4 = dir.path().join("rtx_source_compressed.tmp.mp4");
            let output_mp4 = dir.path().join("rtx_source_compressed.mp4");

            let mut gen_cmd = Command::new(&ffmpeg_path);
            gen_cmd.args([
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1.0:size=640x360:rate=24",
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
            let gen_out = gen_cmd.output().expect("Failed to generate synthetic input MP4");
            assert!(
                gen_out.status.success(),
                "Synthetic input generation failed: {}",
                String::from_utf8_lossy(&gen_out.stderr)
            );

            let (args, preset, qual) = build_ffmpeg_args(
                &input_mp4.to_string_lossy(),
                &temp_mp4.to_string_lossy(),
                "medium",
                "h264_nvenc",
            );
            assert_eq!(preset, "p4");
            assert_eq!(qual, "26");

            let mut comp_cmd = Command::new(&ffmpeg_path);
            comp_cmd.args(&args);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                comp_cmd.creation_flags(CREATE_NO_WINDOW);
            }
            let comp_out = comp_cmd
                .output()
                .expect("Failed to execute FFmpeg h264_nvenc compression");
            assert!(
                comp_out.status.success(),
                "FFmpeg h264_nvenc compression failed: {}",
                String::from_utf8_lossy(&comp_out.stderr)
            );
            assert!(temp_mp4.exists(), "Temporary .tmp.mp4 must exist after NVENC encode");

            let stderr_text = String::from_utf8_lossy(&comp_out.stderr);
            let parsed_times: Vec<u64> = stderr_text.lines().filter_map(parse_out_time_ms).collect();
            assert!(
                !parsed_times.is_empty(),
                "Expected out_time_ms progress lines from h264_nvenc encode"
            );

            std::fs::rename(&temp_mp4, &output_mp4).expect("Atomic rename failed");
            assert!(output_mp4.exists());
            assert!(std::fs::metadata(&output_mp4).unwrap().len() > 0);

            let comp_probe = get_video_info(&ffprobe_path, &output_mp4.to_string_lossy());
            let (comp_dur, comp_res, _) = extract_metadata_from_probe(&comp_probe);
            assert!(comp_dur.unwrap_or(0.0) >= 0.8);
            assert_eq!(comp_res.as_deref(), Some("1280x720"));
            println!(
                "NVENC_E2E_SUCCESS: encoder={}, gpu={:?}, output_size={} bytes, resolution={:?}, duration={:?}",
                probe.hw_encoder,
                probe.gpu_name,
                std::fs::metadata(&output_mp4).unwrap().len(),
                comp_res,
                comp_dur
            );
        }
    }

    #[test]
    fn test_end_to_end_hevc_nvenc_compression_with_build_ffmpeg_args() {
        use crate::state::resolve_binary_path;
        use tempfile::tempdir;

        let ffmpeg_path = resolve_binary_path("ffmpeg");
        let ffprobe_path = resolve_binary_path("ffprobe");
        let probe = probe_gpu_and_encoder(&ffmpeg_path);

        // If this machine has an NVIDIA GPU where hevc_nvenc is available, test real E2E HEVC NVENC encoding (.mov -> _compressed.mp4 with hvc1 tag)
        if probe.hw_encoder_hevc == "hevc_nvenc" {
            let dir = tempdir().unwrap();
            let input_mov = dir.path().join("rtx_hevc_source.mov");
            let temp_mp4 = dir.path().join("rtx_hevc_source_compressed.tmp.mp4");
            let output_mp4 = dir.path().join("rtx_hevc_source_compressed.mp4");

            let mut gen_cmd = Command::new(&ffmpeg_path);
            gen_cmd.args([
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1.0:size=640x360:rate=24",
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
                &input_mov.to_string_lossy(),
            ]);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                gen_cmd.creation_flags(CREATE_NO_WINDOW);
            }
            let gen_out = gen_cmd.output().expect("Failed to generate synthetic input MOV");
            assert!(
                gen_out.status.success(),
                "Synthetic MOV generation failed: {}",
                String::from_utf8_lossy(&gen_out.stderr)
            );

            let (args, preset, qual) = build_ffmpeg_args(
                &input_mov.to_string_lossy(),
                &temp_mp4.to_string_lossy(),
                "medium",
                "hevc_nvenc",
            );
            assert_eq!(preset, "p4");
            assert_eq!(qual, "28");
            assert!(args.contains(&"hvc1".to_string()));

            let mut comp_cmd = Command::new(&ffmpeg_path);
            comp_cmd.args(&args);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                comp_cmd.creation_flags(CREATE_NO_WINDOW);
            }
            let comp_out = comp_cmd
                .output()
                .expect("Failed to execute FFmpeg hevc_nvenc compression");
            assert!(
                comp_out.status.success(),
                "FFmpeg hevc_nvenc compression failed: {}",
                String::from_utf8_lossy(&comp_out.stderr)
            );
            assert!(temp_mp4.exists(), "Temporary .tmp.mp4 must exist after HEVC NVENC encode");

            let stderr_text = String::from_utf8_lossy(&comp_out.stderr);
            let parsed_times: Vec<u64> = stderr_text.lines().filter_map(parse_out_time_ms).collect();
            assert!(
                !parsed_times.is_empty(),
                "Expected out_time_ms progress lines from hevc_nvenc encode"
            );

            std::fs::rename(&temp_mp4, &output_mp4).expect("Atomic rename failed");
            assert!(output_mp4.exists());
            assert!(std::fs::metadata(&output_mp4).unwrap().len() > 0);

            let comp_probe = get_video_info(&ffprobe_path, &output_mp4.to_string_lossy());
            let (comp_dur, comp_res, _) = extract_metadata_from_probe(&comp_probe);
            assert!(comp_dur.unwrap_or(0.0) >= 0.8);
            assert_eq!(comp_res.as_deref(), Some("1280x720"));

            let video_stream = comp_probe
                .get("streams")
                .and_then(|s| s.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|st| st.get("codec_type").and_then(|c| c.as_str()) == Some("video"))
                })
                .expect("Expected video stream in HEVC output MP4");

            assert_eq!(
                video_stream.get("codec_name").and_then(|c| c.as_str()),
                Some("hevc")
            );
            assert_eq!(
                video_stream.get("codec_tag_string").and_then(|c| c.as_str()),
                Some("hvc1")
            );
            println!(
                "HEVC_NVENC_E2E_SUCCESS: encoder={}, gpu={:?}, codec=hevc (hvc1), output_size={} bytes, resolution={:?}, duration={:?}",
                probe.hw_encoder_hevc,
                probe.gpu_name,
                std::fs::metadata(&output_mp4).unwrap().len(),
                comp_res,
                comp_dur
            );
        }
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
        assert_eq!(
            scan_after.count, 1,
            "Compressed output should not be listed as a standalone source video"
        );
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
        assert!(orig_item.compressed_size_gb.is_some());
        assert!(orig_item.reduction_percent.is_some());
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
            hw_encoder_hevc: "libx265".to_string(),
            gpu_name: None,
            is_rtx: false,
            ffmpeg_path: PathBuf::from("ffmpeg"),
            ffprobe_path: PathBuf::from("ffprobe"),
            default_directory: String::new(),
            active_processes: Arc::new(Mutex::new(map)),
            scan_cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        state.cleanup_all();

        assert!(
            !temp_mp4.exists(),
            "Temporary .tmp.mp4 file must be deleted on cleanup/cancellation"
        );
        assert!(state.active_processes.lock().unwrap().is_empty());
    }
}
