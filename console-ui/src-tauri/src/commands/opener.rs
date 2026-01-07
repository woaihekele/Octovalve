use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

fn is_allowed_external_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
}

#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    if !is_allowed_external_url(&url) {
        return Err("invalid url".to_string());
    }
    #[allow(deprecated)]
    app.shell().open(url, None).map_err(|err| err.to_string())
}
