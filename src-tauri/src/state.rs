use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub const DEFAULT_SCAN_DIRECTORY: &str =
    r"C:\Users\ferie\OneDrive - English with Feriel\__Project\Neuro Language Coaching";

pub const CREATE_NO_WINDOW: u32 = 0x08000000;
pub const ES_CONTINUOUS: u32 = 0x80000000;
pub const ES_SYSTEM_REQUIRED: u32 = 0x00000001;

#[cfg(windows)]
extern "system" {
    fn SetThreadExecutionState(es_flags: u32) -> u32;
}

/// Active ou désactive la prévention de la mise en veille Windows pendant l'encodage.
/// Retourne les flags appliqués (`ES_CONTINUOUS | ES_SYSTEM_REQUIRED` ou `ES_CONTINUOUS`).
pub fn set_prevent_sleep(prevent: bool) -> u32 {
    let flags = if prevent {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };

    #[cfg(windows)]
    unsafe {
        let _ = SetThreadExecutionState(flags);
    }

    flags
}

pub struct ActiveCompression {
    pub pid: Option<u32>,
    pub temp_path: PathBuf,
    pub cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

pub struct AppState {
    pub hw_encoder: String,
    pub hw_encoder_hevc: String,
    pub gpu_name: Option<String>,
    pub is_rtx: bool,
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub default_directory: String,
    pub active_processes: Arc<Mutex<HashMap<usize, ActiveCompression>>>,
    pub scan_cancelled: Arc<AtomicBool>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AppInfoResponse {
    pub version: String,
    pub hw_encoder: String,
    pub hw_encoder_hevc: String,
    pub gpu_name: Option<String>,
    pub is_rtx: bool,
    pub default_directory: String,
    pub changelog_markdown: String,
}

impl AppState {
    pub fn cleanup_all(&self) {
        self.scan_cancelled.store(true, Ordering::SeqCst);
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
            set_prevent_sleep(false);
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

pub fn resolve_default_scan_directory() -> String {
    if Path::new(DEFAULT_SCAN_DIRECTORY).exists() {
        return DEFAULT_SCAN_DIRECTORY.to_string();
    }

    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let trimmed = user_profile.trim();
        if !trimmed.is_empty() {
            let videos_dir = PathBuf::from(trimmed).join("Videos");
            if videos_dir.exists() {
                return videos_dir.to_string_lossy().to_string();
            }
            let profile_dir = PathBuf::from(trimmed);
            if profile_dir.exists() {
                return profile_dir.to_string_lossy().to_string();
            }
        }
    }

    DEFAULT_SCAN_DIRECTORY.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_default_scan_directory_returns_non_empty() {
        let resolved = resolve_default_scan_directory();
        assert!(!resolved.is_empty());
        if !Path::new(DEFAULT_SCAN_DIRECTORY).exists() {
            if let Ok(profile) = std::env::var("USERPROFILE") {
                if Path::new(&profile).exists() {
                    assert!(
                        Path::new(&resolved).exists(),
                        "Expected resolved fallback directory '{}' to exist",
                        resolved
                    );
                }
            }
        }
    }

    #[test]
    fn test_set_prevent_sleep_returns_expected_flags() {
        let active_flags = set_prevent_sleep(true);
        assert_eq!(active_flags, ES_CONTINUOUS | ES_SYSTEM_REQUIRED);

        let cleared_flags = set_prevent_sleep(false);
        assert_eq!(cleared_flags, ES_CONTINUOUS);
    }
}
