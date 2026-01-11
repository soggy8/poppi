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
                background_color: "#0a0a0a".to_string(), // Almost black
                text_color: "#e0e0e0".to_string(), // Slightly softer white
                accent_color: "#4a9eff".to_string(), // Bright blue accent
                border_radius: 0.5, // Completely boxy, no rounding
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

