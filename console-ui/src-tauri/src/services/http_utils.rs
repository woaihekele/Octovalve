pub fn join_base_path(base: &str, path: &str) -> Result<String, String> {
    if base.trim().is_empty() {
        return Err("base_url is empty".to_string());
    }
    let normalized_base = base.trim_end_matches('/');
    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    Ok(format!("{normalized_base}{normalized_path}"))
}
