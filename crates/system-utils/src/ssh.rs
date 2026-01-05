use anyhow::Context;
use std::path::PathBuf;
use tokio::process::Command;

const ASKPASS_SCRIPT: &str = "#!/bin/sh\nprintf '%s' \"$OCTOVALVE_SSH_PASS\"\n";

pub fn askpass_env(password: &str) -> anyhow::Result<Vec<(String, String)>> {
    let script = ensure_askpass_script()?;
    Ok(vec![
        ("OCTOVALVE_SSH_PASS".to_string(), password.to_string()),
        ("SSH_ASKPASS".to_string(), script.to_string_lossy().to_string()),
        ("SSH_ASKPASS_REQUIRE".to_string(), "force".to_string()),
        ("DISPLAY".to_string(), "1".to_string()),
    ])
}

pub fn ensure_askpass_script() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").context("failed to resolve HOME for askpass")?;
    let dir = PathBuf::from(home).join(".octovalve");
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join("ssh-askpass.sh");
    let mut needs_write = true;
    if let Ok(existing) = std::fs::read(&path) {
        if existing == ASKPASS_SCRIPT.as_bytes() {
            needs_write = false;
        }
    }
    if needs_write {
        std::fs::write(&path, ASKPASS_SCRIPT)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(path)
}

pub fn apply_askpass_env(cmd: &mut Command, password: &str) -> anyhow::Result<()> {
    for (key, value) in askpass_env(password)? {
        cmd.env(key, value);
    }
    Ok(())
}
