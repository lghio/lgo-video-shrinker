use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone, Debug)]
pub struct LogPayload {
    pub level: String,
    pub msg: String,
}

pub fn emit_log(app: &AppHandle, level: &str, msg: impl Into<String>) {
    let payload = LogPayload {
        level: level.to_string(),
        msg: msg.into(),
    };
    let _ = app.emit("server://log", payload);
}

pub fn log_info(app: &AppHandle, msg: impl Into<String>) {
    emit_log(app, "INFO", msg);
}

pub fn log_error(app: &AppHandle, msg: impl Into<String>) {
    emit_log(app, "ERROR", msg);
}

pub fn log_progress(app: &AppHandle, msg: impl Into<String>) {
    emit_log(app, "PROGRESS", msg);
}

pub fn log_success(app: &AppHandle, msg: impl Into<String>) {
    emit_log(app, "SUCCESS", msg);
}
