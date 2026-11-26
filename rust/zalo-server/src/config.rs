//! Server configuration loaded from environment variables or programmatic construction.

use std::env;
use std::path::{Path, PathBuf};

/// Runtime configuration for the Zalỏ HTTP/WebSocket server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    /// If set, every API/WebSocket request must present this token.
    pub access_token: Option<String>,
    /// Send "seen" receipts to Zalo when a chat is read, like the official app.
    pub read_receipts: bool,
    /// Optional rotating log file path (`LOG_FILE`, set by the Android app).
    pub log_file: Option<PathBuf>,
    /// Optional directory serving static web assets (`PUBLIC_DIR`).
    pub public_dir: Option<PathBuf>,
}

/// Returns true if `host` refers to the local loopback interface.
pub fn is_loopback_host(host: &str) -> bool {
    let trimmed = host.trim();
    trimmed.eq_ignore_ascii_case("localhost")
        || trimmed == "127.0.0.1"
        || trimmed == "::1"
        || trimmed == "[::1]"
}

fn resolve_path(p: &str) -> PathBuf {
    let path = PathBuf::from(p);
    if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn resolve_optional_path(raw: Option<String>) -> Option<PathBuf> {
    raw.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(resolve_path(trimmed))
        }
    })
}

impl Config {
    /// Loads configuration from process environment variables (`HOST`, `PORT`, `DATA_DIR`,
    /// `ACCESS_TOKEN`, `READ_RECEIPTS`, `LOG_FILE`, `PUBLIC_DIR`).
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// Loads configuration using a custom environment lookup function (useful in tests).
    pub fn from_lookup<F>(get: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let host = get("HOST").unwrap_or_else(|| "127.0.0.1".to_string());
        let port = match get("PORT") {
            Some(raw) => {
                let parsed: i64 = raw
                    .trim()
                    .parse()
                    .map_err(|_| format!("Invalid PORT: {raw}"))?;
                if !(1..=65535).contains(&parsed) {
                    return Err(format!("Invalid PORT: {raw}"));
                }
                parsed as u16
            }
            None => 3000,
        };

        let access_token = get("ACCESS_TOKEN").and_then(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let data_dir_raw = get("DATA_DIR").unwrap_or_else(|| "data".to_string());
        let data_dir = resolve_path(&data_dir_raw);
        let read_receipts = get("READ_RECEIPTS").as_deref() != Some("false");
        let log_file = resolve_optional_path(get("LOG_FILE"));
        let public_dir = resolve_optional_path(get("PUBLIC_DIR"));

        let cfg = Self {
            host,
            port,
            data_dir,
            access_token,
            read_receipts,
            log_file,
            public_dir,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validates security and networking invariants.
    pub fn validate(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err("Invalid PORT: 0".to_string());
        }
        // Whoever reaches the server can read and send messages as you,
        // so refuse to listen beyond this machine without a token.
        if !is_loopback_host(&self.host) && self.access_token.is_none() {
            return Err(format!(
                "HOST={} is not loopback; set ACCESS_TOKEN to expose the server.",
                self.host
            ));
        }
        Ok(())
    }

    /// Helper to resolve the path to the legacy single-file message cache inside `data_dir`.
    pub fn messages_file(&self) -> PathBuf {
        self.data_dir.join("messages.json")
    }

    /// Helper to resolve the directory for `data_dir`.
    pub fn data_dir_path(&self) -> &Path {
        &self.data_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn loopback_host_detection() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LOCALHOST"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.1.5"));
    }

    #[test]
    fn refuses_non_loopback_host_without_access_token() {
        let err = Config::from_lookup(lookup(&[("HOST", "0.0.0.0")])).unwrap_err();
        assert!(err.contains("ACCESS_TOKEN"), "unexpected error: {err}");

        let ok = Config::from_lookup(lookup(&[
            ("HOST", "0.0.0.0"),
            ("ACCESS_TOKEN", "  s3cret  "),
        ]))
        .unwrap();
        assert_eq!(ok.access_token.as_deref(), Some("s3cret"));
    }

    #[test]
    fn defaults_and_port_validation() {
        let cfg = Config::from_lookup(lookup(&[])).unwrap();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 3000);
        assert!(cfg.read_receipts);
        assert!(cfg.access_token.is_none());
        assert!(cfg.log_file.is_none());
        assert!(cfg.public_dir.is_none());

        let cfg2 = Config::from_lookup(lookup(&[("READ_RECEIPTS", "false")])).unwrap();
        assert!(!cfg2.read_receipts);

        assert!(Config::from_lookup(lookup(&[("PORT", "0")])).is_err());
        assert!(Config::from_lookup(lookup(&[("PORT", "70000")])).is_err());
        assert!(Config::from_lookup(lookup(&[("PORT", "abc")])).is_err());
    }
}
