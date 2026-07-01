use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const APP_DIR: &str = ".config/sekiro-launcher";
const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub proton: ProtonConfig,
    pub game_prefix: GamePrefixConfig,
    pub game_directories: GameDirectoriesConfig,
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProtonConfig {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameDirectory {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GameDirectoriesConfig {
    pub directories: Vec<GameDirectory>,
    pub selected: Option<String>,
}

impl GameDirectoriesConfig {
    /// Returns the resolved path of the currently selected game directory.
    pub fn selected_path(&self) -> Option<PathBuf> {
        let name = self.selected.as_ref()?;
        let dir = self.directories.iter().find(|d| d.name == *name)?;
        let expanded = shellexpand::full(&dir.path).ok()?;
        let path = PathBuf::from(expanded.into_owned());
        if path.is_dir() {
            Some(path)
        } else {
            None
        }
    }

    /// Add a new game directory. If it's the first, set it as selected.
    pub fn add(&mut self, name: String, path: PathBuf) {
        self.directories.push(GameDirectory {
            name: name.clone(),
            path: path.to_string_lossy().to_string(),
        });
        if self.selected.is_none() || self.directories.len() == 1 {
            self.selected = Some(name);
        }
    }

    /// Remove a game directory by name. Auto-selects the first remaining if the
    /// removed one was the active selection.
    pub fn remove(&mut self, name: &str) {
        self.directories.retain(|d| d.name != name);
        if self.selected.as_deref() == Some(name) {
            self.selected = self.directories.first().map(|d| d.name.clone());
        }
    }

    /// Rename a game directory entry.
    pub fn rename(&mut self, old_name: &str, new_name: &str) {
        if let Some(dir) = self.directories.iter_mut().find(|d| d.name == old_name) {
            dir.name = new_name.to_string();
        }
        if self.selected.as_deref() == Some(old_name) {
            self.selected = Some(new_name.to_string());
        }
    }

    /// Returns true if no game directories are configured.
    pub fn is_empty(&self) -> bool {
        self.directories.is_empty()
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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
        Self::load_from(&Self::default_path())
    }

    pub fn save(&self) -> Result<(), anyhow::Error> {
        self.save_to(&Self::default_path())
    }

    /// Load config from an explicit path (used in tests).
    pub fn load_from(path: &Path) -> Result<Self, anyhow::Error> {
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            // Try full parse first. If it fails (e.g. missing new fields added in newer
            // versions), fall back to merging what we can with defaults.
            match toml::from_str::<Config>(&content) {
                Ok(config) => Ok(config),
                Err(_parse_err) => {
                    log::warn!("Config parse failed (likely missing new fields). Merging old config with defaults...");
                    // Parse as a loose value, then fill defaults for any missing sections
                    let value: toml::Value = toml::from_str(&content)
                        .unwrap_or(toml::Value::Table(toml::value::Table::new()));

                    let mut config = Config::default();

                    if let Some(table) = value.get("proton").and_then(|v| v.as_table()) {
                        if let Some(path) = table.get("path").and_then(|v| v.as_str()) {
                            config.proton.path = Some(path.to_string());
                        }
                    }

                    if let Some(table) = value.get("game_prefix").and_then(|v| v.as_table()) {
                        if let Some(path) = table.get("path").and_then(|v| v.as_str()) {
                            config.game_prefix.path = Some(path.to_string());
                        }
                    }

                    if let Some(table) = value.get("tools").and_then(|v| v.as_table()) {
                        if let Some(sel) = table.get("selected").and_then(|v| v.as_array()) {
                            config.tools.selected = sel.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect();
                        }
                        if let Some(vis) = table.get("visible").and_then(|v| v.as_array()) {
                            config.tools.visible = vis.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect();
                        }
                    }

                    log::info!("Merged config: proton={:?}, prefix={:?}, {} tools selected, game_directories=[]",
                        config.proton.path.as_deref().unwrap_or("(none)"),
                        config.game_prefix.path.as_deref().unwrap_or("(default)"),
                        config.tools.selected.len(),
                    );

                    Ok(config)
                }
            }
        } else {
            Ok(Config::default())
        }
    }

    /// Save config to an explicit path (used in tests).
    pub fn save_to(&self, path: &Path) -> Result<(), anyhow::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_returns_default_when_no_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn load_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = Config {
            proton: ProtonConfig { path: Some("/test/proton".into()) },
            game_prefix: GamePrefixConfig { path: Some("/test/prefix".into()) },
            tools: ToolsConfig {
                selected: vec!["livesplit".into()],
                visible: vec![],
            },
            ..Config::default()
        };
        original.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(original, loaded);
    }

    #[test]
    fn load_merges_partial_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"[proton]
path = "/custom/proton"
"#).unwrap();
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.proton.path, Some("/custom/proton".into()));
        assert_eq!(config.tools, ToolsConfig::default());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("sub/deep/config.toml");
        let config = Config::default();
        config.save_to(&nested).unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn game_directories_add_auto_selects_first() {
        let mut gdc = GameDirectoriesConfig::default();
        assert!(gdc.selected.is_none());
        gdc.add("game1".into(), PathBuf::from("/games/1"));
        assert_eq!(gdc.selected.as_deref(), Some("game1"));
    }

    #[test]
    fn game_directories_remove_reselects() {
        let mut gdc = GameDirectoriesConfig::default();
        gdc.add("a".into(), PathBuf::from("/a"));
        gdc.add("b".into(), PathBuf::from("/b"));
        gdc.selected = Some("a".into());
        gdc.remove("a");
        assert_eq!(gdc.selected.as_deref(), Some("b"));
    }

    #[test]
    fn game_directories_rename_updates_selection() {
        let mut gdc = GameDirectoriesConfig::default();
        gdc.add("old".into(), PathBuf::from("/path"));
        gdc.rename("old", "new");
        assert_eq!(gdc.selected.as_deref(), Some("new"));
        assert_eq!(gdc.directories[0].name, "new");
    }

    #[test]
    fn game_directories_is_empty() {
        let gdc = GameDirectoriesConfig::default();
        assert!(gdc.is_empty());
        let mut gdc = GameDirectoriesConfig::default();
        gdc.add("x".into(), PathBuf::from("/x"));
        assert!(!gdc.is_empty());
    }
}
