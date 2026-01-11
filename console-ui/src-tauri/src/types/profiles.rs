use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub struct ProfileRecord {
    pub name: String,
    pub proxy_path: String,
    pub broker_path: String,
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
