// SPDX-License-Identifier: AGPL-3.0-only
use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Administrator-owned configuration. Never sent to the HTTP client.
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub chart_dir: PathBuf,
    pub listen: String,
    pub timeout_seconds: u64,
    pub etag_salt: Option<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            chart_dir: "charts".into(),
            listen: "127.0.0.1:3000".into(),
            timeout_seconds: 15,
            etag_salt: None,
        }
    }
}
impl Config {
    pub fn load(explicit_path: Option<&Path>) -> Result<Self> {
        let path = explicit_path.unwrap_or_else(|| Path::new("config.json"));
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if explicit_path.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(_) => return Err("Cannot read configuration file".into()),
        };
        if !file
            .metadata()
            .map_err(|_| "Cannot inspect configuration")?
            .is_file()
        {
            return Err("Configuration must be a regular file".into());
        }
        let mut bytes = Vec::new();
        file.take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read configuration")?;
        if bytes.len() > 64 * 1024 {
            return Err("Configuration exceeds 64 KiB".into());
        }
        // Do not surface serde errors: they can quote secret field values.
        let mut config: Self = serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid configuration JSON or unknown fields")?;
        if config.chart_dir.as_os_str().is_empty() {
            return Err("chart_dir must not be empty".into());
        }
        if config.chart_dir.is_relative() {
            config.chart_dir = path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&config.chart_dir);
        }
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if self.chart_dir.as_os_str().is_empty() {
            return Err("chart_dir must not be empty".into());
        }
        if self.listen.parse::<std::net::SocketAddr>().is_err() {
            return Err("listen must be an IP address and port".into());
        }
        if !(1..=30).contains(&self.timeout_seconds) {
            return Err("timeout_seconds must be between 1 and 30".into());
        }
        if self
            .etag_salt
            .as_ref()
            .is_some_and(|s| !(32..=4096).contains(&s.len()))
        {
            return Err("etag_salt must contain 32 to 4096 bytes".into());
        }
        Ok(())
    }
    pub fn initialize(path: &Path) -> Result<()> {
        let mut secret = [0; 32];
        getrandom::fill(&mut secret).map_err(|_| "Cannot generate ETag salt")?;
        let salt: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        let config = Self {
            etag_salt: Some(salt),
            ..Self::default()
        };
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(path)
            .map_err(|_| "Cannot create configuration (existing files are never overwritten)")?;
        let mut bytes =
            serde_json::to_vec_pretty(&config).map_err(|_| "Cannot encode configuration")?;
        bytes.push(b'\n');
        file.write_all(&bytes)
            .map_err(|_| "Cannot write configuration")?;
        file.sync_all()
            .map_err(|_| "Cannot save configuration".into())
    }
}
