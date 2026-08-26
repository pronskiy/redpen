//! Reads the same `config.json` that `evals/run.sh` writes and reads. One file, one shape —
//! the harness has been exercising this contract since before there was any Rust.
//!
//! A1.2 needs only the hotkey, so this is a plain read. Hot reload, the fs watcher and the
//! prompt-file plumbing are A2.1.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_base_url() -> String { "https://api.anthropic.com".into() }
fn default_model() -> String { "claude-sonnet-5".into() }   // decision #25
fn default_effort() -> String { "medium".into() }
fn default_hotkey() -> String { "Alt+Cmd+E".into() }   // same string evals/run.sh writes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_effort")]
    pub effort: String,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default)]
    pub system_prompt_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: default_base_url(),
            model: default_model(),
            effort: default_effort(),
            hotkey: default_hotkey(),
            system_prompt_path: String::new(),
        }
    }
}

impl Config {
    /// `~/Library/Application Support/redpen/config.json` — deliberately not the
    /// identifier-derived Tauri path, so the app and the harness share one file.
    pub fn path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home)
            .join("Library/Application Support/redpen")
            .join("config.json")
    }

    /// Never fails: a missing or malformed file degrades to defaults so a typo in the JSON
    /// cannot stop the app from starting — it just loses the hotkey override.
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Config>(&raw) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("[redpen] {} is not valid JSON ({e}); using defaults", path.display());
                    Self::default()
                }
            },
            Err(_) => {
                eprintln!("[redpen] no config at {}; using defaults", path.display());
                Self::default()
            }
        }
    }

    /// Written so "Open Config" always has something to open.
    pub fn ensure_exists() -> std::io::Result<PathBuf> {
        let path = Self::path();
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let json = serde_json::to_string_pretty(&Config::default())
                .unwrap_or_else(|_| "{}".into());
            std::fs::write(&path, json)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
        Ok(path)
    }
}
