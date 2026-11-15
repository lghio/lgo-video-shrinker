pub mod compressor;
pub mod files;
pub mod logger;
pub mod scanner;
pub mod state;

use crate::state::{
    resolve_binary_path, resolve_default_scan_directory, AppInfoResponse, AppState,
};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, State, WindowEvent};

const CHANGELOG_MARKDOWN: &str = include_str!("../../CHANGELOG.md");

fn log_hardware_status(
    app: &AppHandle,
    hw_encoder: &str,
    hw_encoder_hevc: &str,
    gpu_name: &Option<String>,
    is_rtx: bool,
) {
    if is_rtx {
        let gpu_label = gpu_name.as_deref().unwrap_or("NVIDIA RTX");
        logger::log_info(
            app,
            format!(
                "[HW] Carte NVIDIA RTX détectée : {} — Moteur NVENC avancé actif (H.264: {} / HEVC: {})",
                gpu_label, hw_encoder, hw_encoder_hevc
            ),
        );
    } else if hw_encoder == "libx264" {
        if let Some(ref gpu) = gpu_name {
            logger::log_info(
                app,
                format!(
                    "[HW] Carte détectée ({}) mais aucun encodeur hardware disponible — utilisation CPU ({} / {})",
                    gpu, hw_encoder, hw_encoder_hevc
                ),
            );
        } else {
            logger::log_info(
                app,
                format!(
                    "[HW] Aucun encodeur hardware, utilisation CPU ({} / {})",
                    hw_encoder, hw_encoder_hevc
                ),
            );
        }
    } else if let Some(ref gpu) = gpu_name {
        logger::log_info(
            app,
            format!(
                "[HW] Encodeur hardware détecté : {} / {} ({})",
                hw_encoder, hw_encoder_hevc, gpu
            ),
        );
    } else {
        logger::log_info(
            app,
            format!(
                "[HW] Encodeur hardware détecté : {} / {}",
                hw_encoder, hw_encoder_hevc
            ),
        );
    }
}

#[tauri::command]
fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfoResponse {
    let version = app.package_info().version.to_string();
    log_hardware_status(
        &app,
        &state.hw_encoder,
        &state.hw_encoder_hevc,
        &state.gpu_name,
        state.is_rtx,
    );
    AppInfoResponse {
        version,
        hw_encoder: state.hw_encoder.clone(),
        hw_encoder_hevc: state.hw_encoder_hevc.clone(),
        gpu_name: state.gpu_name.clone(),
        is_rtx: state.is_rtx,
        default_directory: state.default_directory.clone(),
        changelog_markdown: CHANGELOG_MARKDOWN.to_string(),
    }
}

#[tauri::command]
fn probe_hardware(app: AppHandle, state: State<'_, AppState>) -> AppInfoResponse {
    let version = app.package_info().version.to_string();
    let probe = compressor::probe_gpu_and_encoder(&state.ffmpeg_path);
    log_hardware_status(
        &app,
        &probe.hw_encoder,
        &probe.hw_encoder_hevc,
        &probe.gpu_name,
        probe.is_rtx,
    );
    AppInfoResponse {
        version,
        hw_encoder: probe.hw_encoder,
        hw_encoder_hevc: probe.hw_encoder_hevc,
        gpu_name: probe.gpu_name,
        is_rtx: probe.is_rtx,
        default_directory: state.default_directory.clone(),
        changelog_markdown: CHANGELOG_MARKDOWN.to_string(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let ffmpeg_path = resolve_binary_path("ffmpeg");
    let ffprobe_path = resolve_binary_path("ffprobe");
    let gpu_probe = compressor::probe_gpu_and_encoder(&ffmpeg_path);

    let app_state = AppState {
        hw_encoder: gpu_probe.hw_encoder,
        hw_encoder_hevc: gpu_probe.hw_encoder_hevc,
        gpu_name: gpu_probe.gpu_name,
        is_rtx: gpu_probe.is_rtx,
        ffmpeg_path,
        ffprobe_path,
        default_directory: resolve_default_scan_directory(),
        active_processes: Arc::new(Mutex::new(HashMap::new())),
        scan_cancelled: Arc::new(AtomicBool::new(false)),
    };

    let active_processes_for_exit = app_state.active_processes.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(|app| {
            let about_item =
                MenuItemBuilder::with_id("menu_about", "À propos (About)").build(app)?;
            let changelog_item = MenuItemBuilder::with_id(
                "menu_changelog",
                "Journal des modifications (Changelog)",
            )
            .build(app)?;
            let probe_gpu_item = MenuItemBuilder::with_id(
                "menu_probe_gpu",
                "Détecter le GPU RTX / Encodeur",
            )
            .build(app)?;

            let help_submenu = SubmenuBuilder::new(app, "Menu / Aide")
                .item(&about_item)
                .item(&changelog_item)
                .separator()
                .item(&probe_gpu_item)
                .build()?;

            MenuBuilder::new(app).item(&help_submenu).build()
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "menu_about" => {
                let _ = app.emit("menu://action", "about");
            }
            "menu_changelog" => {
                let _ = app.emit("menu://action", "changelog");
            }
            "menu_probe_gpu" => {
                let _ = app.emit("menu://action", "probe_gpu");
            }
            _ => {}
        })
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
                    state::set_prevent_sleep(false);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            probe_hardware,
            scanner::scan_videos,
            scanner::cancel_scan,
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
