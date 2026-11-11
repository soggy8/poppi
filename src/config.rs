use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// Every section and field is optional in the file; anything missing falls
/// back to its default. Unknown keys from older versions are ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme: ThemeConfig,
    pub behavior: BehaviorConfig,
    pub search: SearchConfig,
    pub calculator: CalculatorConfig,
    pub files: FilesConfig,
    pub claude: ClaudeConfig,
    pub clipboard: ClipboardConfig,
    pub emoji: EmojiConfig,
    /// Keyword -> URL or command. `{}` is replaced by the typed arguments.
    pub aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    /// "dark", "light" or "system" (follow the desktop's colour scheme).
    pub mode: String,
    /// Colours for dark mode.
    pub background_color: String,
    pub text_color: String,
    /// Colours for light mode.
    pub light_background_color: String,
    pub light_text_color: String,
    pub accent_color: String,
    /// Corner radius of the launcher, in pixels.
    pub border_radius: f64,
    /// Font size in points.
    pub font_size: i32,
    /// Launcher width in pixels. Height follows the number of results.
    pub width: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
    /// Hide the launcher when another window takes focus.
    pub close_on_focus_loss: bool,
    /// Keep the process running after the window is dismissed so the next
    /// invocation shows it instantly. When false the process exits instead.
    pub stay_resident: bool,
    /// Rows visible at once; longer result lists scroll.
    pub max_results: usize,
    /// How many past queries to remember. 0 disables history.
    pub history_size: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Engine listed first when nothing else matches:
    /// "google", "duckduckgo", "youtube" or "chatgpt".
    pub default_engine: String,
    pub youtube_enabled: bool,
    pub chatgpt_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalculatorConfig {
    pub enabled: bool,
    /// Currency conversion downloads exchange rates (ECB, via
    /// frankfurter.dev) at most twice a day.
    pub currency: bool,
}

/// Clipboard history is kept in memory only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardConfig {
    pub enabled: bool,
    pub size: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmojiConfig {
    /// "default", "light", "medium-light", "medium", "medium-dark" or "dark".
    pub skin_tone: String,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            size: 30,
        }
    }
}

impl Default for EmojiConfig {
    fn default() -> Self {
        Self {
            skin_tone: "default".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilesConfig {
    /// Enables the `f <name>` file search.
    pub enabled: bool,
    /// Directories to index. `~` is the home directory.
    pub roots: Vec<String>,
    /// How many directory levels below each root are indexed.
    pub max_depth: usize,
    pub include_hidden: bool,
    /// Directory or file names that are skipped entirely.
    pub exclude: Vec<String>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            mode: "dark".to_string(),
            background_color: "#161618".to_string(),
            text_color: "#ececee".to_string(),
            light_background_color: "#f7f7f8".to_string(),
            light_text_color: "#1c1c1e".to_string(),
            accent_color: "#4a9eff".to_string(),
            border_radius: 14.0,
            font_size: 16,
            width: 640,
        }
    }
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            close_on_focus_loss: true,
            stay_resident: true,
            max_results: 5,
            history_size: 50,
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_engine: "google".to_string(),
            youtube_enabled: true,
            chatgpt_enabled: true,
        }
    }
}

impl Default for CalculatorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            currency: true,
        }
    }
}

/// Questions (anything ending in `?`) are answered through the Claude Code
/// CLI, using the account it is logged in with. There is no API key setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    pub enabled: bool,
    /// Model alias or full name passed to `claude --model`.
    pub model: String,
    /// The Claude Code executable.
    pub command: String,
    pub system_prompt: String,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: "haiku".to_string(),
            command: "claude".to_string(),
            system_prompt: "You answer questions inside a small desktop launcher. \
                            Be brief: usually one to three sentences, more only when asked. \
                            Light markdown is rendered: `code`, **bold**, short bullet lists \
                            and fenced code blocks. No headings or tables."
                .to_string(),
        }
    }
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            roots: vec!["~".to_string()],
            max_depth: 6,
            include_hidden: false,
            exclude: ["node_modules", "target", "__pycache__", "venv", "snap"]
                .map(String::from)
                .to_vec(),
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

    pub fn parse(contents: &str) -> Result<Self, String> {
        toml::from_str::<Config>(contents)
            .map(Config::sanitized)
            .map_err(|e| e.to_string())
    }

    /// Clamp numeric values to ranges the UI can actually render.
    pub fn sanitized(mut self) -> Self {
        self.theme.font_size = self.theme.font_size.clamp(6, 48);
        self.theme.width = self.theme.width.clamp(300, 2000);
        self.theme.border_radius = if self.theme.border_radius.is_finite() {
            self.theme.border_radius.clamp(0.0, 50.0)
        } else {
            0.0
        };
        self.behavior.max_results = self.behavior.max_results.clamp(1, 20);
        self.behavior.history_size = self.behavior.history_size.min(500);
        self.clipboard.size = self.clipboard.size.min(200);
        if !["dark", "light", "system"].contains(&self.theme.mode.as_str()) {
            self.theme.mode = "dark".to_string();
        }
        self.files.max_depth = self.files.max_depth.clamp(1, 12);
        let defaults = ClaudeConfig::default();
        if self.claude.model.trim().is_empty() {
            self.claude.model = defaults.model;
        }
        if self.claude.command.trim().is_empty() {
            self.claude.command = defaults.command;
        }
        // An alias is matched against the first typed word.
        self.aliases
            .retain(|name, target| !name.is_empty() && !name.contains(char::is_whitespace) && !target.trim().is_empty());
        self
    }

    /// Load the config. A missing file is created with defaults. A file that
    /// fails to parse is left untouched and reported, and defaults are used.
    pub fn load() -> (Self, Option<String>) {
        let path = Self::config_path();

        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let default = Config::default();
                let error = default
                    .save()
                    .err()
                    .map(|e| format!("Could not write {}: {}", path.display(), e));
                return (default, error);
            }
            Err(e) => {
                return (
                    Config::default(),
                    Some(format!("Could not read {}: {}", path.display(), e)),
                );
            }
        };

        match Self::parse(&contents) {
            Ok(config) => (config, None),
            Err(e) => (
                Config::default(),
                Some(format!("Invalid config {}: {}", path.display(), e)),
            ),
        }
    }

    /// Read the config file as it is now; an error if it is missing or
    /// does not parse. Used when the file changes while we are running.
    pub fn load_existing() -> Result<Self, String> {
        let path = Self::config_path();
        let contents = fs::read_to_string(&path)
            .map_err(|e| format!("Could not read {}: {}", path.display(), e))?;
        Self::parse(&contents).map_err(|e| format!("Invalid config {}: {}", path.display(), e))
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let contents = toml::to_string(self).map_err(|e| e.to_string())?;
        fs::write(&path, contents).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_all_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let config = Config::parse("[theme]\nfont_size = 20\n").unwrap();
        assert_eq!(config.theme.font_size, 20);
        assert_eq!(config.theme.width, 640);
        assert!(config.calculator.enabled);
        assert!(config.behavior.close_on_focus_loss);
    }

    #[test]
    fn keys_from_older_versions_are_ignored() {
        let old = r##"
            [theme]
            background_color = "#2e2e2e"
            height = 600

            [shortcuts]
            show_launcher = "Super+Space"
        "##;
        let config = Config::parse(old).unwrap();
        assert_eq!(config.theme.background_color, "#2e2e2e");
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(Config::parse("[theme\nfont_size = ").is_err());
        assert!(Config::parse("[theme]\nfont_size = \"big\"").is_err());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let config = Config::parse(
            "[theme]\nfont_size = 900\nwidth = 5\nborder_radius = -3.0\n[behavior]\nmax_results = 0\n",
        )
        .unwrap();
        assert_eq!(config.theme.font_size, 48);
        assert_eq!(config.theme.width, 300);
        assert_eq!(config.theme.border_radius, 0.0);
        assert_eq!(config.behavior.max_results, 1);
    }

    #[test]
    fn aliases_and_files_sections() {
        let config = Config::parse(
            "[aliases]\ngh = \"https://github.com/search?q={}\"\n\"bad name\" = \"x\"\nempty = \" \"\n\n[files]\nroots = [\"~/Documents\"]\nmax_depth = 99\n",
        )
        .unwrap();
        assert_eq!(config.aliases.len(), 1);
        assert_eq!(config.aliases["gh"], "https://github.com/search?q={}");
        assert_eq!(config.files.roots, vec!["~/Documents"]);
        assert_eq!(config.files.max_depth, 12);
        assert!(config.files.enabled);
    }

    #[test]
    fn claude_defaults_to_haiku_and_has_no_key_setting() {
        let config = Config::parse("[claude]\nmodel = \"\"\n").unwrap();
        assert_eq!(config.claude.model, "haiku");
        assert_eq!(config.claude.command, "claude");
        assert!(config.claude.enabled);
        let text = toml::to_string(&Config::default()).unwrap().to_lowercase();
        assert!(!text.contains("api_key") && !text.contains("token"));
    }

    #[test]
    fn theme_mode_is_validated() {
        assert_eq!(Config::parse("[theme]\nmode = \"light\"").unwrap().theme.mode, "light");
        assert_eq!(Config::parse("[theme]\nmode = \"neon\"").unwrap().theme.mode, "dark");
        assert_eq!(Config::default().theme.mode, "dark");
    }

    #[test]
    fn round_trips_through_toml() {
        let mut config = Config::default();
        config.aliases.insert("gh".to_string(), "https://github.com/search?q={}".to_string());
        let text = toml::to_string(&config).unwrap();
        assert_eq!(Config::parse(&text).unwrap(), config);
    }
}
