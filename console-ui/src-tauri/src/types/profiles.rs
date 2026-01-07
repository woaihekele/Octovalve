use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub struct ProfileRecord {
    pub name: String,
    pub proxy_path: String,
    pub broker_path: String,
    #[serde(default)]
    pub remote_dir_alias: String,
    #[serde(default = "default_remote_listen_port")]
    pub remote_listen_port: u16,
    #[serde(default = "default_remote_control_port")]
    pub remote_control_port: u16,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ProfileRuntimeSettings {
    pub remote_dir_alias: String,
    pub remote_listen_port: u16,
    pub remote_control_port: u16,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ProfilesFile {
    pub current: String,
    pub profiles: Vec<ProfileRecord>,
}

#[derive(Clone, Serialize)]
pub struct ProfileSummary {
    pub name: String,
}

#[derive(Clone, Serialize)]
pub struct ProfilesStatus {
    pub current: String,
    pub profiles: Vec<ProfileSummary>,
}

fn default_remote_listen_port() -> u16 {
    19307
}

fn default_remote_control_port() -> u16 {
    19308
}
