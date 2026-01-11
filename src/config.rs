use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use dirs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub theme: ThemeConfig,
    pub shortcuts: ShortcutConfig,
    pub search: SearchConfig,
    pub calculator: CalculatorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub background_color: String,
    pub text_color: String,
    pub accent_color: String,
    pub border_radius: f64,
    pub font_size: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutConfig {
    pub show_launcher: String, // e.g., "Super+Space"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub default_engine: String,
    pub youtube_enabled: bool,
    pub chatgpt_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalculatorConfig {
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemeConfig {
                background_color: "#2e2e2e".to_string(),
                text_color: "#ffffff".to_string(),
                accent_color: "#5294e2".to_string(),
                border_radius: 12.0,
                font_size: 16,
                width: 800,
                height: 600,
            },
            shortcuts: ShortcutConfig {
                show_launcher: "Super+Space".to_string(),
            },
            search: SearchConfig {
                default_engine: "google".to_string(),
                youtube_enabled: true,
                chatgpt_enabled: true,
            },
            calculator: CalculatorConfig {
                enabled: true,
            },
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("poppi_launcher")
            .join("config.toml")
    }

    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::config_path();
        
        if !path.exists() {
            let default = Config::default();
            default.save()?;
            return Ok(default);
        }

        let contents = fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string(self)?;
        fs::write(&path, contents)?;
        Ok(())
    }
}

