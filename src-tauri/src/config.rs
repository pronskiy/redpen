//! The config the app and `evals/run.sh` share. One file, one shape — the harness has been
//! exercising this contract since before there was any Rust (decision #19).
//!
//! The system prompt is a *path*, not an inline string (decision #23): a 141-line prompt
//! escaped into JSON would be unreadable, which defeats the whole reason config is a file
//! you edit in an editor (decision #8). So hot reload watches **two** files — the JSON and
//! whatever `system_prompt_path` points at.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

fn default_base_url() -> String { "https://api.anthropic.com".into() }
fn default_model() -> String { "claude-sonnet-5".into() }   // decision #25
fn default_effort() -> String { "medium".into() }
fn default_hotkey() -> String { "Alt+Cmd+E".into() }   // same string evals/run.sh writes
// Integer points, not f64: `Config` derives `Eq` so that hot reload can compare two loads
// for equality, and f64 is not `Eq`. Half-point control is not worth losing that.
fn default_font_size() -> u8 { 15 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Base size for the card, in points. Everything else scales off it in `rem`.
    #[serde(default = "default_font_size")]
    pub font_size: u8,
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
            font_size: default_font_size(),
            system_prompt_path: String::new(),
        }
    }
}

impl Config {
    /// `~/Library/Application Support/redpen/config.json` — deliberately not the
    /// identifier-derived Tauri path, so the app and the harness share one file.
    pub fn path() -> PathBuf {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join("Library/Application Support/redpen")
            .join("config.json")
    }

    /// Never fails. A malformed file degrades to defaults rather than stopping the app —
    /// you will be editing this JSON by hand many times a day, and a stray comma should
    /// cost you a hotkey, not a running app.
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                eprintln!("[redpen] {} is not valid JSON ({e}); using defaults", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Written so "Open Config" always has something to open.
    pub fn ensure_exists(path: &Path) -> std::io::Result<()> {
        if path.exists() {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&Config::default()).unwrap_or_default())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

/// The config plus the prompt text it points at — resolved together so nothing downstream
/// has to know the prompt lives in a second file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub system_prompt: String,
}

impl Loaded {
    pub fn load_from(config_path: &Path) -> Self {
        let config = Config::load_from(config_path);
        let system_prompt = if config.system_prompt_path.is_empty() {
            String::new()
        } else {
            std::fs::read_to_string(&config.system_prompt_path).unwrap_or_else(|e| {
                eprintln!("[redpen] cannot read prompt {}: {e}", config.system_prompt_path);
                String::new()
            })
        };
        Self { config, system_prompt }
    }
}

#[derive(Clone)]
pub struct ConfigStore {
    path: PathBuf,
    inner: Arc<RwLock<Loaded>>,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        let loaded = Loaded::load_from(&path);
        Self { path, inner: Arc::new(RwLock::new(loaded)) }
    }

    pub fn current(&self) -> Loaded {
        self.inner.read().expect("config lock poisoned").clone()
    }

    /// Re-reads from disk. Returns `Some((old, new))` only when something actually changed.
    ///
    /// Content comparison, not event counting: one save fires several fs events, and an
    /// editor writing a temp file and renaming it over the target fires more still. Doing
    /// the cheap re-read and diffing the result is simpler than debouncing, and it cannot
    /// miss a change the way a time window can.
    pub fn reload(&self) -> Option<(Loaded, Loaded)> {
        let fresh = Loaded::load_from(&self.path);
        let mut guard = self.inner.write().expect("config lock poisoned");
        if *guard == fresh {
            return None;
        }
        let old = std::clone::Clone::clone(&*guard);
        *guard = fresh.clone();
        Some((old, fresh))
    }

    fn dirs_to_watch(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(d) = self.path.parent() {
            dirs.push(d.to_path_buf());
        }
        let prompt = self.current().config.system_prompt_path;
        if !prompt.is_empty() {
            if let Some(d) = Path::new(&prompt).parent() {
                if d.exists() && !dirs.contains(&d.to_path_buf()) {
                    dirs.push(d.to_path_buf());
                }
            }
        }
        dirs
    }
}

/// Watch config.json and the prompt file, calling `on_change(old, new)` when either really
/// changes.
///
/// Watches the *directories*, not the files. Editors overwhelmingly save by writing a temp
/// file and renaming it over the target — a watch on the file itself follows the old inode
/// and goes silent after the first save.
#[cfg(target_os = "macos")]
pub fn watch<F>(store: ConfigStore, on_change: F) -> notify::Result<()>
where
    F: Fn(&Loaded, &Loaded) + Send + 'static,
{
    use notify::{RecursiveMode, Watcher};

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| { let _ = tx.send(res); })?;

    let mut watched: HashSet<PathBuf> = HashSet::new();
    for dir in store.dirs_to_watch() {
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
            watched.insert(dir);
        }
    }

    std::thread::spawn(move || {
        let mut watcher = watcher; // keep alive for the life of the thread
        while rx.recv().is_ok() {
            let Some((old, new)) = store.reload() else { continue };
            // The prompt may now live somewhere else entirely.
            for dir in store.dirs_to_watch() {
                if !watched.contains(&dir) && watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
                    watched.insert(dir);
                }
            }
            on_change(&old, &new);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp(name: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "redpen-{}-{}-{}", name, std::process::id(), N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_file_gives_defaults() {
        let c = Config::load_from(&tmp("missing").join("nope.json"));
        assert_eq!(c, Config::default());
        assert_eq!(c.hotkey, "Alt+Cmd+E");
    }

    #[test]
    fn partial_json_fills_the_rest_from_defaults() {
        let d = tmp("partial");
        let p = d.join("config.json");
        std::fs::write(&p, r#"{"api_key":"sk-test"}"#).unwrap();
        let c = Config::load_from(&p);
        assert_eq!(c.api_key, "sk-test");
        assert_eq!(c.model, "claude-sonnet-5", "missing keys must not blank out defaults");
        assert_eq!(c.hotkey, "Alt+Cmd+E");
        assert_eq!(c.font_size, 15, "an older config without font_size still works");
    }

    #[test]
    fn malformed_json_degrades_to_defaults_instead_of_failing() {
        let d = tmp("bad");
        let p = d.join("config.json");
        std::fs::write(&p, "{ this is not json ").unwrap();
        assert_eq!(Config::load_from(&p), Config::default());
    }

    #[test]
    fn ensure_exists_writes_once_and_does_not_clobber() {
        let d = tmp("ensure");
        let p = d.join("config.json");
        Config::ensure_exists(&p).unwrap();
        std::fs::write(&p, r#"{"api_key":"mine"}"#).unwrap();
        Config::ensure_exists(&p).unwrap();
        assert_eq!(Config::load_from(&p).api_key, "mine", "must never overwrite a real config");
    }

    #[test]
    fn loaded_resolves_the_prompt_file() {
        let d = tmp("prompt");
        let prompt = d.join("critique.md");
        std::fs::write(&prompt, "# the prompt").unwrap();
        let p = d.join("config.json");
        std::fs::write(&p, format!(r#"{{"system_prompt_path":"{}"}}"#, prompt.display())).unwrap();
        assert_eq!(Loaded::load_from(&p).system_prompt, "# the prompt");
    }

    #[test]
    fn reload_reports_a_change_once_and_then_stays_quiet() {
        let d = tmp("reload");
        let p = d.join("config.json");
        std::fs::write(&p, r#"{"hotkey":"Alt+Cmd+E"}"#).unwrap();
        let store = ConfigStore::new(p.clone());

        assert!(store.reload().is_none(), "no edit, no change");

        std::fs::write(&p, r#"{"hotkey":"Alt+Cmd+R"}"#).unwrap();
        let (old, new) = store.reload().expect("edit must be seen");
        assert_eq!(old.config.hotkey, "Alt+Cmd+E");
        assert_eq!(new.config.hotkey, "Alt+Cmd+R");
        assert_eq!(store.current().config.hotkey, "Alt+Cmd+R");

        // The duplicate fs events a single save produces must not re-fire.
        assert!(store.reload().is_none(), "same content must not look like a change");
    }

    #[test]
    fn editing_only_the_prompt_file_counts_as_a_change() {
        let d = tmp("promptedit");
        let prompt = d.join("critique.md");
        std::fs::write(&prompt, "v1").unwrap();
        let p = d.join("config.json");
        std::fs::write(&p, format!(r#"{{"system_prompt_path":"{}"}}"#, prompt.display())).unwrap();
        let store = ConfigStore::new(p);
        assert!(store.reload().is_none());

        std::fs::write(&prompt, "v2").unwrap();
        let (old, new) = store.reload().expect("a prompt edit is a config change");
        assert_eq!(old.system_prompt, "v1");
        assert_eq!(new.system_prompt, "v2");
    }
}
