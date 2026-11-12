//! The providers the launcher knows, and turning a `[ai.models.<name>]`
//! entry into everything needed to ask it.

use crate::config::{AiConfig, ModelConfig};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Claude Code CLI on its own login (a Pro/Max subscription).
    ClaudeCli,
    /// Google's Gemini CLI on its own login.
    GeminiCli,
    /// OpenAI's Codex CLI on its own login (a ChatGPT plan).
    Codex,
    /// Any shell command: the prompt on stdin, the answer on stdout.
    Command,
    /// `POST {base_url}/chat/completions`, the format most APIs speak.
    OpenAi,
    /// Anthropic's Messages API.
    Anthropic,
}

#[derive(Debug)]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub base_url: &'static str,
    /// Environment variable read for the API key; empty when none is used.
    pub key_env: &'static str,
    /// Whether a request without a key is pointless.
    pub needs_key: bool,
    pub default_model: &'static str,
    /// Example model ids, shown as a placeholder in the settings window.
    pub model_hint: &'static str,
    /// The CLI program for CLI providers.
    pub program: &'static str,
}

const fn api(
    id: &'static str,
    name: &'static str,
    base_url: &'static str,
    key_env: &'static str,
    default_model: &'static str,
    model_hint: &'static str,
) -> Provider {
    Provider {
        id,
        name,
        kind: Kind::OpenAi,
        base_url,
        key_env,
        needs_key: true,
        default_model,
        model_hint,
        program: "",
    }
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "claude",
        name: "Claude",
        kind: Kind::ClaudeCli,
        base_url: "",
        key_env: "",
        needs_key: false,
        default_model: "haiku",
        model_hint: "haiku, sonnet or opus",
        program: "claude",
    },
    Provider {
        id: "gemini-cli",
        name: "Gemini CLI",
        kind: Kind::GeminiCli,
        base_url: "",
        key_env: "",
        needs_key: false,
        default_model: "",
        model_hint: "empty for the CLI's default",
        program: "gemini",
    },
    Provider {
        id: "codex",
        name: "Codex",
        kind: Kind::Codex,
        base_url: "",
        key_env: "",
        needs_key: false,
        default_model: "",
        model_hint: "empty for the CLI's default",
        program: "codex",
    },
    Provider {
        id: "ollama",
        name: "Ollama",
        kind: Kind::OpenAi,
        base_url: "http://localhost:11434/v1",
        key_env: "",
        needs_key: false,
        default_model: "llama3.2",
        model_hint: "llama3.2, qwen3, gemma3 (ollama list)",
        program: "",
    },
    Provider {
        id: "lmstudio",
        name: "LM Studio",
        kind: Kind::OpenAi,
        base_url: "http://localhost:1234/v1",
        key_env: "",
        needs_key: false,
        default_model: "",
        model_hint: "the model id shown in LM Studio",
        program: "",
    },
    Provider {
        id: "anthropic",
        name: "Anthropic API",
        kind: Kind::Anthropic,
        base_url: "https://api.anthropic.com/v1",
        key_env: "ANTHROPIC_API_KEY",
        needs_key: true,
        default_model: "claude-haiku-5-5",
        model_hint: "claude-haiku-5-5, claude-sonnet-5-5",
        program: "",
    },
    api(
        "openai",
        "OpenAI",
        "https://api.openai.com/v1",
        "OPENAI_API_KEY",
        "gpt-5-mini",
        "gpt-5-mini, gpt-5",
    ),
    api(
        "gemini",
        "Gemini API",
        "https://generativelanguage.googleapis.com/v1beta/openai",
        "GEMINI_API_KEY",
        "gemini-flash-latest",
        "gemini-flash-latest, gemini-pro-latest",
    ),
    api(
        "openrouter",
        "OpenRouter",
        "https://openrouter.ai/api/v1",
        "OPENROUTER_API_KEY",
        "openrouter/auto",
        "openrouter/auto, vendor/model",
    ),
    api(
        "mistral",
        "Mistral",
        "https://api.mistral.ai/v1",
        "MISTRAL_API_KEY",
        "mistral-small-latest",
        "mistral-small-latest, mistral-large-latest",
    ),
    api(
        "groq",
        "Groq",
        "https://api.groq.com/openai/v1",
        "GROQ_API_KEY",
        "llama-3.3-70b-versatile",
        "llama-3.3-70b-versatile, openai/gpt-oss-120b",
    ),
    api(
        "deepseek",
        "DeepSeek",
        "https://api.deepseek.com/v1",
        "DEEPSEEK_API_KEY",
        "deepseek-chat",
        "deepseek-chat, deepseek-reasoner",
    ),
    api(
        "xai",
        "xAI",
        "https://api.x.ai/v1",
        "XAI_API_KEY",
        "grok-4",
        "grok-4, grok-4-fast",
    ),
    api(
        "perplexity",
        "Perplexity",
        "https://api.perplexity.ai",
        "PERPLEXITY_API_KEY",
        "sonar",
        "sonar, sonar-pro",
    ),
    api(
        "together",
        "Together",
        "https://api.together.xyz/v1",
        "TOGETHER_API_KEY",
        "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        "org/model",
    ),
    api(
        "fireworks",
        "Fireworks",
        "https://api.fireworks.ai/inference/v1",
        "FIREWORKS_API_KEY",
        "accounts/fireworks/models/llama-v3p3-70b-instruct",
        "accounts/fireworks/models/…",
    ),
    api(
        "cerebras",
        "Cerebras",
        "https://api.cerebras.ai/v1",
        "CEREBRAS_API_KEY",
        "llama-3.3-70b",
        "llama-3.3-70b",
    ),
    Provider {
        id: "openai-compatible",
        name: "OpenAI-compatible",
        kind: Kind::OpenAi,
        base_url: "",
        key_env: "",
        needs_key: false,
        default_model: "",
        model_hint: "as the server names it",
        program: "",
    },
    Provider {
        id: "command",
        name: "Command",
        kind: Kind::Command,
        base_url: "",
        key_env: "",
        needs_key: false,
        default_model: "",
        model_hint: "unused",
        program: "",
    },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Where an API key may come from, checked in this order when a request
/// is made (on a worker thread: the keyring may take a moment).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeySource {
    pub inline: Option<String>,
    pub keyring_id: Option<String>,
    pub env: Option<String>,
}

impl KeySource {
    pub fn resolve(&self) -> Option<String> {
        let clean = |key: String| Some(key.trim().to_string()).filter(|k| !k.is_empty());
        self.inline
            .clone()
            .and_then(clean)
            .or_else(|| {
                self.keyring_id
                    .as_deref()
                    .and_then(keyring_lookup)
                    .and_then(clean)
            })
            .or_else(|| {
                self.env
                    .as_deref()
                    .and_then(|name| std::env::var(name).ok())
                    .and_then(clean)
            })
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.inline.is_none() && self.keyring_id.is_none() && self.env.is_none()
    }
}

/// A model entry with the provider's defaults filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The name it is typed as.
    pub name: String,
    pub label: String,
    pub provider_id: String,
    pub provider_name: String,
    pub kind: Kind,
    pub model: String,
    pub base_url: String,
    pub key: KeySource,
    pub needs_key: bool,
    /// CLI program, or the shell command for `command`.
    pub command: String,
    pub system_prompt: String,
}

/// The keyring entry for a model's key: shared by every model of a
/// provider, except for self-hosted endpoints, which each get their own.
pub fn keyring_id(name: &str, entry: &ModelConfig) -> String {
    match entry.provider.as_str() {
        "openai-compatible" | "command" => format!("model:{}", name),
        provider => provider.to_string(),
    }
}

fn claude_label(model: &str) -> String {
    match model {
        "haiku" | "sonnet" | "opus" => format!("Claude {}", crate::utils::capitalize(model)),
        "" => "Claude".to_string(),
        other => format!("Claude · {}", other),
    }
}

pub fn resolve_entry(
    name: &str,
    entry: &ModelConfig,
    system_prompt: &str,
) -> Result<Resolved, String> {
    let provider = provider(entry.provider.trim())
        .ok_or_else(|| format!("Unknown AI provider \"{}\" for {}", entry.provider, name))?;
    let model = Some(entry.model.trim())
        .filter(|m| !m.is_empty())
        .unwrap_or(provider.default_model)
        .to_string();
    let base_url = Some(entry.base_url.trim())
        .filter(|u| !u.is_empty())
        .unwrap_or(provider.base_url)
        .trim_end_matches('/')
        .to_string();
    let command = Some(entry.command.trim())
        .filter(|c| !c.is_empty())
        .unwrap_or(provider.program)
        .to_string();

    let label = if !entry.label.trim().is_empty() {
        entry.label.trim().to_string()
    } else if provider.kind == Kind::ClaudeCli {
        claude_label(&model)
    } else if model.is_empty() {
        provider.name.to_string()
    } else {
        format!("{} · {}", provider.name, model)
    };

    // Local servers (Ollama, LM Studio) take no key, so the keyring is not
    // even asked; a self-hosted OpenAI-compatible server may want one.
    let uses_key = provider.needs_key || provider.id == "openai-compatible";
    let key = if uses_key {
        let env = Some(entry.api_key_env.trim())
            .filter(|e| !e.is_empty())
            .or(Some(provider.key_env).filter(|e| !e.is_empty()))
            .map(String::from);
        KeySource {
            inline: Some(entry.api_key.clone()).filter(|k| !k.trim().is_empty()),
            keyring_id: Some(keyring_id(name, entry)),
            env,
        }
    } else {
        KeySource::default()
    };

    if provider.kind == Kind::OpenAi && base_url.is_empty() {
        return Err(format!(
            "{} needs a base URL (base_url in the config)",
            name
        ));
    }
    if provider.kind == Kind::Command && command.is_empty() {
        return Err(format!("{} needs a command (command in the config)", name));
    }

    Ok(Resolved {
        name: name.to_string(),
        label,
        provider_id: provider.id.to_string(),
        provider_name: provider.name.to_string(),
        kind: provider.kind,
        model,
        base_url,
        key,
        needs_key: provider.needs_key,
        command,
        system_prompt: system_prompt.trim().to_string(),
    })
}

/// The model called `name`, or the default one when `name` is `None`.
pub fn resolve(config: &AiConfig, name: Option<&str>) -> Result<Resolved, String> {
    let name = name.unwrap_or(&config.default);
    let entry = config
        .models
        .get(name)
        .ok_or_else(|| format!("No AI model called \"{}\"", name))?;
    resolve_entry(name, entry, &config.system_prompt)
}

/// A display name that never fails, for rows and the footer.
pub fn label(config: &AiConfig, name: &str) -> String {
    resolve(config, Some(name))
        .map(|r| r.label)
        .unwrap_or_else(|_| name.to_string())
}

// ---------- GNOME keyring (libsecret's secret-tool) ----------

const KEYRING_SERVICE: &str = "poppi-launcher";

fn secret_tool() -> Option<std::path::PathBuf> {
    crate::utils::find_in_path("secret-tool")
}

/// Longest a keyring lookup may take. Without a running secret service, or
/// with a locked keyring waiting for a password, secret-tool can block.
const KEYRING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

pub fn keyring_lookup(id: &str) -> Option<String> {
    use std::io::Read;
    let mut child = Command::new(secret_tool()?)
        .args(["lookup", "service", KEYRING_SERVICE, "key", id])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < KEYRING_TIMEOUT => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                eprintln!("The keyring did not answer; continuing without a stored key");
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut key = String::new();
    child.stdout.take()?.read_to_string(&mut key).ok()?;
    Some(key).filter(|key| !key.trim().is_empty())
}

/// Store a key; the secret goes over stdin, never on a command line.
pub fn keyring_store(id: &str, label: &str, secret: &str) -> Result<(), String> {
    use std::io::Write;
    let program = secret_tool().ok_or("secret-tool is not installed (package libsecret)")?;
    let mut child = Command::new(program)
        .args([
            "store",
            "--label",
            label,
            "service",
            KEYRING_SERVICE,
            "key",
            id,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run secret-tool: {}", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(secret.trim().as_bytes())
            .map_err(|e| format!("Could not pass the key to secret-tool: {}", e))?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not save the key: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub fn keyring_clear(id: &str) -> Result<(), String> {
    let program = secret_tool().ok_or("secret-tool is not installed (package libsecret)")?;
    Command::new(program)
        .args(["clear", "service", KEYRING_SERVICE, "key", id])
        .stdin(Stdio::null())
        .output()
        .map(|_| ())
        .map_err(|e| format!("Could not run secret-tool: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str, model: &str) -> ModelConfig {
        ModelConfig::new(provider, model)
    }

    #[test]
    fn provider_ids_are_unique_and_complete() {
        for (index, provider) in PROVIDERS.iter().enumerate() {
            assert!(
                PROVIDERS[index + 1..]
                    .iter()
                    .all(|other| other.id != provider.id),
                "duplicate {}",
                provider.id
            );
            if provider.needs_key {
                assert!(
                    !provider.key_env.is_empty(),
                    "{} needs a key variable",
                    provider.id
                );
                assert!(provider.base_url.starts_with("https://"), "{}", provider.id);
            }
        }
    }

    #[test]
    fn defaults_fill_in_the_entry() {
        let r = resolve_entry("gpt", &entry("openai", ""), "be brief").unwrap();
        assert_eq!(r.kind, Kind::OpenAi);
        assert_eq!(r.model, "gpt-5-mini");
        assert_eq!(r.base_url, "https://api.openai.com/v1");
        assert_eq!(r.label, "OpenAI · gpt-5-mini");
        assert_eq!(r.key.env.as_deref(), Some("OPENAI_API_KEY"));
        assert_eq!(r.key.keyring_id.as_deref(), Some("openai"));
        assert_eq!(r.system_prompt, "be brief");

        let haiku = resolve_entry("haiku", &entry("claude", "haiku"), "").unwrap();
        assert_eq!(haiku.label, "Claude Haiku");
        assert_eq!(haiku.command, "claude");
        // The subscription CLI never gets a key.
        assert!(haiku.key.is_empty());

        let llama = resolve_entry("llama", &entry("ollama", "llama3.2"), "").unwrap();
        assert_eq!(llama.base_url, "http://localhost:11434/v1");
        assert!(llama.key.is_empty() && !llama.needs_key);
    }

    #[test]
    fn overrides_win() {
        let mut custom = entry("openai-compatible", "qwen");
        custom.base_url = "http://box:8080/v1/".to_string();
        custom.api_key_env = "BOX_KEY".to_string();
        custom.label = "Home server".to_string();
        let r = resolve_entry("box", &custom, "").unwrap();
        assert_eq!(r.base_url, "http://box:8080/v1");
        assert_eq!(r.key.env.as_deref(), Some("BOX_KEY"));
        assert_eq!(r.key.keyring_id.as_deref(), Some("model:box"));
        assert_eq!(r.label, "Home server");
    }

    #[test]
    fn incomplete_entries_are_errors() {
        assert!(resolve_entry("x", &entry("nope", ""), "").is_err());
        assert!(resolve_entry("x", &entry("openai-compatible", "m"), "").is_err());
        assert!(resolve_entry("x", &entry("command", ""), "").is_err());
        let mut command = entry("command", "");
        command.command = "llm -m gpt-4o".to_string();
        assert_eq!(
            resolve_entry("x", &command, "").unwrap().kind,
            Kind::Command
        );
    }

    #[test]
    fn inline_key_comes_first() {
        let source = KeySource {
            inline: Some(" sk-inline ".into()),
            keyring_id: None,
            env: Some("POPPI_TEST_UNSET_VARIABLE".into()),
        };
        assert_eq!(source.resolve().as_deref(), Some("sk-inline"));
        let none = KeySource {
            env: Some("POPPI_TEST_UNSET_VARIABLE".into()),
            ..KeySource::default()
        };
        assert_eq!(none.resolve(), None);
    }

    #[test]
    fn default_config_models_resolve() {
        let config = AiConfig::default();
        for name in config.models.keys() {
            resolve(&config, Some(name)).unwrap();
        }
        assert_eq!(resolve(&config, None).unwrap().name, "haiku");
        assert_eq!(label(&config, "missing"), "missing");
    }
}

#[cfg(test)]
mod keyring_tests {
    /// Stores, reads and removes a throwaway entry in the real keyring:
    /// `cargo test -- --ignored keyring_round_trip`
    #[test]
    #[ignore = "uses the desktop keyring"]
    fn keyring_round_trip() {
        let id = "poppi-selftest";
        super::keyring_store(id, "Poppi Launcher self-test", "  sk-test-123\n").unwrap();
        assert_eq!(super::keyring_lookup(id).as_deref(), Some("sk-test-123"));
        super::keyring_clear(id).unwrap();
        assert_eq!(super::keyring_lookup(id), None);
    }
}
