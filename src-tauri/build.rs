fn main() {
    let _ = std::fs::create_dir_all("target/frontend-dist");
    tauri_build::build()
}
