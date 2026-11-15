use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub const DEFAULT_SCAN_DIRECTORY: &str =
    r"C:\Users\ferie\OneDrive - English with Feriel\__Project\Neuro Language Coaching";

pub const CREATE_NO_WINDOW: u32 = 0x08000000;

pub struct ActiveCompression {
    pub pid: Option<u32>,
    pub temp_path: PathBuf,
    pub cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

pub struct AppState {
    pub hw_encoder: String,
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub default_directory: String,
    pub active_processes: Arc<Mutex<HashMap<usize, ActiveCompression>>>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AppInfoResponse {
    pub version: String,
    pub hw_encoder: String,
    pub default_directory: String,
}

impl AppState {
    pub fn cleanup_all(&self) {
        if let Ok(mut map) = self.active_processes.lock() {
            for (_, mut active) in map.drain() {
                if let Some(tx) = active.cancel_tx.take() {
                    let _ = tx.send(());
                }
                if let Some(pid) = active.pid {
                    kill_process_tree(pid);
                }
                if active.temp_path.exists() {
                    let _ = std::fs::remove_file(&active.temp_path);
                }
            }
        }
    }
}

pub fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
}

pub fn resolve_binary_path(bin_name: &str) -> PathBuf {
    let triple_name = format!("{}-x86_64-pc-windows-msvc.exe", bin_name);
    let plain_name = format!("{}.exe", bin_name);

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push(exe_dir.join(&plain_name));
            candidates.push(exe_dir.join(&triple_name));
            candidates.push(exe_dir.join("binaries").join(&triple_name));
            candidates.push(exe_dir.join("binaries").join(&plain_name));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("src-tauri").join("binaries").join(&triple_name));
        candidates.push(cwd.join("binaries").join(&triple_name));
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("binaries").join(&triple_name));
    candidates.push(PathBuf::from(r"C:\_tools\ffmpeg\bin").join(&plain_name));

    for candidate in candidates {
        if candidate.exists() {
            return candidate;
        }
    }

    PathBuf::from(bin_name)
}
