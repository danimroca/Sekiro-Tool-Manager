use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const APP_DIR: &str = ".config/sekiro-launcher";
const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub proton: ProtonConfig,
    pub game_prefix: GamePrefixConfig,
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProtonConfig {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GamePrefixConfig {
    pub path: Option<String>,
}

impl GamePrefixConfig {
    /// Returns the default Sekiro game prefix path.
    pub fn default_path() -> std::path::PathBuf {
        let home = std::env::var("HOME").expect("HOME env var not set");
        let mut path = std::path::PathBuf::from(home);
        path.push(".local/share/Steam/steamapps/compatdata/814380/pfx/");
        path
    }

    /// Returns the configured path, or the default Sekiro prefix if not set.
    pub fn resolved_path(&self) -> std::path::PathBuf {
        self.path
            .as_ref()
            .and_then(|p| shellexpand::full(p).ok().map(|s| std::path::PathBuf::from(s.into_owned())))
            .filter(|p| p.is_dir())
            .map(|p| {
                // Migration: if path ends with "drive_c", strip it (old buggy default)
                if p.file_name().map_or(false, |n| n == "drive_c" || n == "Drive_C") {
                    p.parent().unwrap_or(p.as_path()).to_path_buf()
                } else {
                    p
                }
            })
            .unwrap_or_else(Self::default_path)
    }
}

impl ProtonConfig {
    /// Returns true if a proton path is configured and points to an existing directory.
    pub fn is_configured(&self) -> bool {
        self.path.as_ref().map(|p| {
            let path = shellexpand::full(p).ok().map(|s| s.into_owned());
            path.and_then(|p| PathBuf::from(&p).canonicalize().ok())
                .map(|p| p.is_dir())
                .unwrap_or(false)
        }).unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub selected: Vec<String>,
    pub visible: Vec<String>,
}

impl Config {
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").expect("HOME env var not set");
        let mut path = PathBuf::from(home);
        path.push(APP_DIR);
        path.push(CONFIG_FILE);
        path
    }

    pub fn load() -> Result<Self, anyhow::Error> {
        let path = Self::default_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self) -> Result<(), anyhow::Error> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}
