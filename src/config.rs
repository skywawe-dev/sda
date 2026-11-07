//! The app's own settings, kept in `%APPDATA%\SDA\config.json` so they do not
//! depend on which `maFiles` folder is open.

use crate::i18n::Lang;
use std::path::PathBuf;
/// Where the app keeps its own settings: %APPDATA%\SDA, deliberately outside maFiles.
fn app_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("SDA")
}

fn config_path() -> PathBuf {
    app_dir().join("config.json")
}

pub(crate) fn default_mafiles_dir() -> PathBuf {
    app_dir().join("maFiles")
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct Config {
    #[serde(default)]
    pub(crate) mafiles_dir: Option<String>,
    /// Pull the persona name and avatar from the public profile.
    #[serde(default)]
    pub(crate) enrich: bool,
    /// Experimental: the system acrylic backdrop behind the window.
    #[serde(default)]
    pub(crate) blur: bool,
    /// Make the minimise button send the window to the tray instead.
    #[serde(default = "yes")]
    pub(crate) minimize_to_tray: bool,
    /// Interface language, `None` meaning "follow the system".
    #[serde(default)]
    pub(crate) lang: Option<String>,
}

/// `#[serde(default)]` yields false for a bool, and this one defaults to on.
fn yes() -> bool {
    true
}

/// The language to start in: an explicit choice if one was saved, else the system's.
pub(crate) fn startup_lang(cfg: &Config) -> Lang {
    cfg.lang.as_deref().and_then(Lang::from_code).unwrap_or_else(crate::i18n::system_default)
}

pub(crate) fn load_config() -> Config {
    std::fs::read_to_string(config_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub(crate) fn save_config(cfg: &Config) -> Result<(), String> {
    std::fs::create_dir_all(app_dir()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), text).map_err(|e| e.to_string())
}

pub(crate) fn save_mafiles_dir(dir: &std::path::Path) -> Result<(), String> {
    let mut cfg = load_config();
    cfg.mafiles_dir = Some(dir.to_string_lossy().into_owned());
    save_config(&cfg)
}

/// The maFiles directory from config.json, falling back to %APPDATA%\SDA\maFiles.
pub(crate) fn mafiles_dir() -> PathBuf {
    load_config().mafiles_dir.filter(|s| !s.trim().is_empty()).map(PathBuf::from).unwrap_or_else(default_mafiles_dir)
}
