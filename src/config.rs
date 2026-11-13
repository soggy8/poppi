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
    pub ai: AiConfig,
    /// The `[claude]` section of earlier versions; read once and migrated
    /// into `[ai]`, never written back.
    #[serde(skip_serializing)]
    pub claude: Option<LegacyClaudeConfig>,
    pub clipboard: ClipboardConfig,
    pub emoji: EmojiConfig,
    pub voice: VoiceConfig,
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

/// Dictation into the search bar (Ctrl+D or the microphone button).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub enabled: bool,
    /// Turns a recording into text: `{file}` is a 16 kHz mono WAV file and
    /// the text is read from stdout. Empty uses whisper.cpp's `whisper-cli`.
    pub command: String,
    /// whisper.cpp model (`ggml-*.bin`) used when `command` is empty.
    pub model: String,
    /// Spoken language code for whisper-cli ("en", "de", …) or "auto".
    pub language: String,
    /// Keep listening for `wake_phrase` while Poppi runs in the background;
    /// needs whisper-cli. Off unless turned on: the microphone stays open.
    pub wake_word: bool,
    pub wake_phrase: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            command: String::new(),
            model: "~/.local/share/poppi_launcher/ggml-base.en.bin".to_string(),
            language: "auto".to_string(),
            wake_word: false,
            wake_phrase: "Poppi".to_string(),
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
    /// Skip files and folders matched by `.gitignore` files.
    pub respect_gitignore: bool,
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

/// The default instructions given to every model.
pub const DEFAULT_SYSTEM_PROMPT: &str = "You answer questions inside a small desktop launcher. \
    Be brief: usually one to three sentences, more only when asked. \
    Light markdown is rendered: `code`, **bold**, short bullet lists \
    and fenced code blocks. No headings or tables.";

/// Questions (anything ending in `?`, or after `ask `) go to the default
/// model; `<name> question` picks another one by its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub enabled: bool,
    /// Name of the entry in `models` that answers plain questions.
    pub default: String,
    pub system_prompt: String,
    /// Name (also the prefix you type) -> model.
    pub models: BTreeMap<String, ModelConfig>,
}

/// One model you can ask. Only `provider` is required; everything else
/// falls back to the provider's defaults (see `ai::providers`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    /// `claude`, `gemini-cli`, `codex`, `ollama`, `openai`, `anthropic`,
    /// `gemini`, `openrouter`, ... or `openai-compatible` / `command`.
    pub provider: String,
    /// Model id as the provider names it. Empty: the provider's default.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Shown in the launcher instead of the generated name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// API endpoint, for `openai-compatible` or to override a provider's.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    /// Environment variable holding the API key, overriding the provider's
    /// usual one (`OPENAI_API_KEY`, ...).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub api_key_env: String,
    /// An API key in plain text. Prefer the keyring (settings window) or an
    /// environment variable; this is for setups without either.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// The program for CLI providers, or the shell command for `command`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,
}

impl ModelConfig {
    pub fn new(provider: &str, model: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            ..Self::default()
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        let models = [
            ("groq", ModelConfig::new("groq", "")),
            ("haiku", ModelConfig::new("claude", "haiku")),
            ("sonnet", ModelConfig::new("claude", "sonnet")),
            ("opus", ModelConfig::new("claude", "opus")),
            ("gemini", ModelConfig::new("gemini-cli", "")),
            ("codex", ModelConfig::new("codex", "")),
            ("llama", ModelConfig::new("ollama", "llama3.2")),
        ]
        .into_iter()
        .map(|(name, model)| (name.to_string(), model))
        .collect();
        Self {
            enabled: true,
            // Free with an API key from console.groq.com; the first-run
            // setup explains this and offers the alternatives.
            default: "groq".to_string(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            models,
        }
    }
}

/// `[claude]` from version 0.2.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct LegacyClaudeConfig {
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub command: Option<String>,
    pub system_prompt: Option<String>,
}

/// A model name is typed as the first word, so it must be one word.
/// The default model of configs written before 0.4.
const LEGACY_DEFAULT_MODEL: &str = "haiku";

pub fn is_valid_model_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            roots: vec!["~".to_string()],
            max_depth: 6,
            include_hidden: false,
            respect_gitignore: true,
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
        let mut config = toml::from_str::<Config>(contents).map_err(|e| e.to_string())?;
        // Files from before 0.4 never named a default model and got Claude
        // Haiku; keep it rather than switching them to the new default.
        let names_default = toml::from_str::<toml::Value>(contents)
            .ok()
            .and_then(|v| v.get("ai")?.get("default").cloned())
            .is_some();
        if !names_default && !contents.trim().is_empty() {
            config.ai.default = LEGACY_DEFAULT_MODEL.to_string();
        }
        Ok(config.sanitized())
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
        self.migrate_claude_section();
        self.ai
            .models
            .retain(|name, model| is_valid_model_name(name) && !model.provider.trim().is_empty());
        if self.ai.models.is_empty() {
            self.ai.models = AiConfig::default().models;
        }
        if !self.ai.models.contains_key(&self.ai.default) {
            let fallback = AiConfig::default().default;
            self.ai.default = if self.ai.models.contains_key(&fallback) {
                fallback
            } else {
                self.ai.models.keys().next().cloned().unwrap_or_default()
            };
        }
        // An alias is matched against the first typed word.
        self.aliases.retain(|name, target| {
            !name.is_empty() && !name.contains(char::is_whitespace) && !target.trim().is_empty()
        });
        self
    }

    /// Carry settings from a `[claude]` section over to `[ai]`.
    fn migrate_claude_section(&mut self) {
        let Some(old) = self.claude.take() else {
            return;
        };
        if let Some(enabled) = old.enabled {
            self.ai.enabled = enabled;
        }
        if let Some(prompt) = old.system_prompt.filter(|p| !p.trim().is_empty()) {
            self.ai.system_prompt = prompt;
        }
        let model = old
            .model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        let command = old
            .command
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty() && c != "claude");
        if let Some(command) = &command {
            for entry in self
                .ai
                .models
                .values_mut()
                .filter(|m| m.provider == "claude")
            {
                entry.command = command.clone();
            }
        }
        if let Some(model) = model {
            let existing = self
                .ai
                .models
                .iter()
                .find(|(_, m)| m.provider == "claude" && m.model == model)
                .map(|(name, _)| name.clone());
            self.ai.default = existing.unwrap_or_else(|| {
                let mut entry = ModelConfig::new("claude", &model);
                entry.command = command.clone().unwrap_or_default();
                self.ai.models.insert("claude".to_string(), entry);
                "claude".to_string()
            });
        }
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
        fs::write(&path, contents).map_err(|e| e.to_string())?;
        // A key written into the file is for this user's eyes only.
        if self
            .ai
            .models
            .values()
            .any(|m| !m.api_key.trim().is_empty())
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
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
    fn ai_defaults_to_free_groq_and_old_files_keep_haiku() {
        let config = Config::default();
        assert!(config.ai.enabled);
        assert_eq!(config.ai.default, "groq");
        assert_eq!(config.ai.models["groq"], ModelConfig::new("groq", ""));
        assert_eq!(
            config.ai.models["haiku"],
            ModelConfig::new("claude", "haiku")
        );
        // A saved default round-trips; a file without one predates 0.4.
        let saved = toml::to_string(&config).unwrap();
        assert_eq!(Config::parse(&saved).unwrap().ai.default, "groq");
        assert_eq!(
            Config::parse("[theme]\nmode = \"dark\"\n")
                .unwrap()
                .ai
                .default,
            "haiku"
        );
        // Nothing secret is written by default.
        let text = toml::to_string(&config).unwrap().to_lowercase();
        assert!(!text.contains("api_key") && !text.contains("token"));
    }

    #[test]
    fn old_claude_section_is_migrated() {
        let config = Config::parse("[claude]\nenabled = false\nmodel = \"sonnet\"\n").unwrap();
        assert!(!config.ai.enabled);
        assert_eq!(config.ai.default, "sonnet");
        assert!(config.claude.is_none());
        assert!(!toml::to_string(&config).unwrap().contains("[claude]"));

        let custom =
            Config::parse("[claude]\nmodel = \"claude-opus-4-1\"\ncommand = \"/opt/claude\"\n")
                .unwrap();
        assert_eq!(custom.ai.default, "claude");
        assert_eq!(custom.ai.models["claude"].model, "claude-opus-4-1");
        assert_eq!(custom.ai.models["haiku"].command, "/opt/claude");

        // An empty model in the old section keeps the default.
        assert_eq!(
            Config::parse("[claude]\nmodel = \"\"\n")
                .unwrap()
                .ai
                .default,
            "haiku"
        );
    }

    #[test]
    fn models_are_validated() {
        let config = Config::parse(
            "[ai]\ndefault = \"nope\"\n[ai.models.gpt]\nprovider = \"openai\"\nmodel = \"gpt-5-mini\"\n[ai.models.\"two words\"]\nprovider = \"openai\"\n[ai.models.empty]\nmodel = \"x\"\n",
        )
        .unwrap();
        assert_eq!(config.ai.models.keys().collect::<Vec<_>>(), ["gpt"]);
        assert_eq!(config.ai.default, "gpt");
    }

    #[test]
    fn theme_mode_is_validated() {
        assert_eq!(
            Config::parse("[theme]\nmode = \"light\"")
                .unwrap()
                .theme
                .mode,
            "light"
        );
        assert_eq!(
            Config::parse("[theme]\nmode = \"neon\"")
                .unwrap()
                .theme
                .mode,
            "dark"
        );
        assert_eq!(Config::default().theme.mode, "dark");
    }

    #[test]
    fn round_trips_through_toml() {
        let mut config = Config::default();
        config.aliases.insert(
            "gh".to_string(),
            "https://github.com/search?q={}".to_string(),
        );
        let text = toml::to_string(&config).unwrap();
        assert_eq!(Config::parse(&text).unwrap(), config);
    }
}
