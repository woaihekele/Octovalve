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

pub fn resolve_octovalve_proxy_bin() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("OCTOVALVE_PROXY_BIN") {
        let candidate = PathBuf::from(value);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    let candidate = sidecar_path("octovalve-proxy")?;
    if candidate.exists() {
        return Ok(candidate);
    }

    let mut cursor = std::env::current_exe().map_err(|err| err.to_string())?;
    for _ in 0..8 {
        if let Some(parent) = cursor.parent() {
            let release = parent
                .join("target")
                .join("release")
                .join("octovalve-proxy");
            if release.exists() {
                return Ok(release);
            }
            let debug = parent.join("target").join("debug").join("octovalve-proxy");
            if debug.exists() {
                return Ok(debug);
            }
            cursor = parent.to_path_buf();
        } else {
            break;
        }
    }

    Err("failed to locate octovalve-proxy binary (set OCTOVALVE_PROXY_BIN to override)".to_string())
}
