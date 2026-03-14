use std::io::{self, Write};

fn main() -> io::Result<()> {
    let password = std::env::var("OCTOVALVE_SSH_PASS").unwrap_or_default();
    if password.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "OCTOVALVE_SSH_PASS is empty",
        ));
    }
    let mut stdout = io::stdout();
    stdout.write_all(password.as_bytes())?;
    stdout.flush()
}
