/// Error type for ACP operations.
#[derive(Debug)]
pub struct AcpError(pub String);

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for AcpError {
    fn from(s: String) -> Self {
        AcpError(s)
    }
}

impl From<&str> for AcpError {
    fn from(s: &str) -> Self {
        AcpError(s.to_string())
    }
}

impl From<std::io::Error> for AcpError {
    fn from(e: std::io::Error) -> Self {
        AcpError(e.to_string())
    }
}

impl From<serde_json::Error> for AcpError {
    fn from(e: serde_json::Error) -> Self {
        AcpError(e.to_string())
    }
}
