use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct BundleConfig {
    pub backend: String,
    pub path: String,
    pub remote: Option<String>,
    pub default_branch: Option<String>,
    pub branch_policy: Option<String>,
    pub auth: Option<AuthConfig>,
    pub write_allowlist: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    pub ssh_key: Option<String>,
    pub token_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    pub index: Option<String>,
    pub watch: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub audit_dir: Option<String>,
    pub search: Option<SearchConfig>,
    pub bundles: HashMap<String, BundleConfig>,
}

impl ServerConfig {
    pub fn from_file(path: &str) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config file: {e}"))?;
        Self::from_toml(&content)
    }

    pub fn from_toml(content: &str) -> Result<Self, String> {
        toml::from_str(content).map_err(|e| format!("failed to parse config: {e}"))
    }

    pub fn audit_dir(&self) -> &str {
        self.audit_dir.as_deref().unwrap_or(".okf-audit")
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedBundleConfig {
    pub name: String,
    pub backend: BundleBackend,
    pub path: PathBuf,
    pub remote: Option<String>,
    pub default_branch: Option<String>,
    pub branch_policy: Option<String>,
    pub auth: Option<AuthConfig>,
    pub write_allowlist: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BundleBackend {
    Fs,
    Git,
}
