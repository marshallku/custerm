use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
    #[serde(default)]
    pub panels: Vec<PluginPanelDef>,
    #[serde(default)]
    pub commands: Vec<PluginCommandDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginMeta {
    pub name: String,
    pub title: String,
    pub version: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginPanelDef {
    pub name: String,
    pub title: String,
    pub file: String,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginCommandDef {
    pub name: String,
    pub exec: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub dir: PathBuf,
}

pub fn plugin_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("turm")
        .join("plugins")
}

pub fn discover_plugins() -> Vec<LoadedPlugin> {
    let dir = plugin_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut plugins = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("plugin.toml");
        if !manifest_path.exists() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&manifest_path) else {
            eprintln!("[turm] failed to read {}", manifest_path.display());
            continue;
        };
        match toml::from_str::<PluginManifest>(&content) {
            Ok(manifest) => {
                plugins.push(LoadedPlugin {
                    manifest,
                    dir: path,
                });
            }
            Err(e) => {
                eprintln!("[turm] failed to parse {}: {e}", manifest_path.display());
            }
        }
    }
    plugins
}
