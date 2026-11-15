pub mod compressor;
pub mod files;
pub mod logger;
pub mod scanner;
pub mod state;

use crate::state::{
    resolve_binary_path, resolve_default_scan_directory, AppInfoResponse, AppState,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State, WindowEvent};

#[tauri::command]
fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfoResponse {
    let version = app.package_info().version.to_string();
    if state.hw_encoder == "libx264" {
        logger::log_info(
            &app,
            "[HW] Aucun encodeur hardware, utilisation CPU (libx264)",
        );
    } else {
        logger::log_info(
            &app,
            format!("[HW] Encodeur hardware détecté : {}", state.hw_encoder),
        );
    }
    AppInfoResponse {
        version,
        hw_encoder: state.hw_encoder.clone(),
        default_directory: state.default_directory.clone(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let ffmpeg_path = resolve_binary_path("ffmpeg");
    let ffprobe_path = resolve_binary_path("ffprobe");
    let hw_encoder = compressor::detect_hw_encoder(&ffmpeg_path);

    let app_state = AppState {
        hw_encoder,
        ffmpeg_path,
        ffprobe_path,
        default_directory: resolve_default_scan_directory(),
        active_processes: Arc::new(Mutex::new(HashMap::new())),
    };

    let active_processes_for_exit = app_state.active_processes.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(app_state)
        .on_window_event(move |_window, event| {
            if matches!(
                event,
                WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
            ) {
                if let Ok(mut map) = active_processes_for_exit.lock() {
                    for (_, mut active) in map.drain() {
                        if let Some(tx) = active.cancel_tx.take() {
                            let _ = tx.send(());
                        }
                        if let Some(pid) = active.pid {
                            state::kill_process_tree(pid);
                        }
                        if active.temp_path.exists() {
                            let _ = std::fs::remove_file(&active.temp_path);
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            scanner::scan_videos,
            compressor::start_compression,
            compressor::stop_compression,
            compressor::cleanup_all_compressions,
            files::browse_directory,
            files::delete_file,
            files::delete_batch,
            files::open_folder,
            files::play_video,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
