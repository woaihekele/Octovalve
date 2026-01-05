use std::path::PathBuf;

pub fn sidecar_path(name: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let dir = exe
        .parent()
        .ok_or_else(|| "failed to resolve sidecar dir".to_string())?;
    #[cfg(windows)]
    {
        return Ok(dir.join(format!("{name}.exe")));
    }
    #[cfg(not(windows))]
    {
        return Ok(dir.join(name));
    }
}
