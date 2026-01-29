use std::fs;
use std::path::PathBuf;

const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;

#[tauri::command]
pub async fn read_text_file(path: String) -> Result<String, String> {
    let path = PathBuf::from(path);
    let metadata = fs::metadata(&path).map_err(|err| err.to_string())?;
    if !metadata.is_file() {
        return Err("路径不是文件".to_string());
    }
    if metadata.len() > MAX_IMPORT_BYTES {
        return Err(format!("文件过大（{} bytes）", metadata.len()));
    }
    fs::read_to_string(&path).map_err(|err| err.to_string())
}
