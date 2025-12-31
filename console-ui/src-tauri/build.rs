use std::path::PathBuf;

fn main() {
    cleanup_legacy_remote_broker();
    tauri_build::build()
}

fn cleanup_legacy_remote_broker() {
    let out_dir = std::env::var("OUT_DIR").ok();
    let Some(out_dir) = out_dir else { return };
    let target_dir = PathBuf::from(out_dir)
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .map(|path| path.to_path_buf());
    let Some(target_dir) = target_dir else { return };
    let legacy_path = target_dir.join("remote-broker");
    let metadata = match std::fs::symlink_metadata(&legacy_path) {
        Ok(metadata) => metadata,
        Err(_) => return,
    };
    if metadata.is_file() {
        let _ = std::fs::remove_file(&legacy_path);
    }
}
