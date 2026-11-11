use crate::config::SearchConfig;
use crate::utils;
use gtk::gio;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Google,
    DuckDuckGo,
    Youtube,
    Chatgpt,
}

impl Engine {
    pub const ALL: [Engine; 4] = [
        Engine::Google,
        Engine::DuckDuckGo,
        Engine::Youtube,
        Engine::Chatgpt,
    ];

    /// The name used in the config file.
    pub fn id(self) -> &'static str {
        match self {
            Engine::Google => "google",
            Engine::DuckDuckGo => "duckduckgo",
            Engine::Youtube => "youtube",
            Engine::Chatgpt => "chatgpt",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Engine::Google => "Google",
            Engine::DuckDuckGo => "DuckDuckGo",
            Engine::Youtube => "YouTube",
            Engine::Chatgpt => "ChatGPT",
        }
    }

    pub fn from_id(id: &str) -> Option<Engine> {
        let id = id.trim().to_lowercase();
        Engine::ALL.into_iter().find(|e| e.id() == id)
    }

    pub fn is_enabled(self, config: &SearchConfig) -> bool {
        match self {
            Engine::Youtube => config.youtube_enabled,
            Engine::Chatgpt => config.chatgpt_enabled,
            Engine::Google | Engine::DuckDuckGo => true,
        }
    }

    pub fn url(self, query: &str) -> String {
        let q = urlencoding::encode(query);
        match self {
            Engine::Google => format!("https://www.google.com/search?q={}", q),
            Engine::DuckDuckGo => format!("https://duckduckgo.com/?q={}", q),
            Engine::Youtube => format!("https://www.youtube.com/results?search_query={}", q),
            Engine::Chatgpt => format!("https://chatgpt.com/?q={}", q),
        }
    }
}

/// Engines offered when a query matches nothing else: the configured default
/// first, then the other enabled ones. DuckDuckGo only appears as the default.
pub fn fallback_engines(config: &SearchConfig) -> Vec<Engine> {
    let default = Engine::from_id(&config.default_engine)
        .filter(|e| e.is_enabled(config))
        .unwrap_or(Engine::Google);
    let mut engines = vec![default];
    for engine in [Engine::Google, Engine::Youtube, Engine::Chatgpt] {
        if engine.is_enabled(config) && !engines.contains(&engine) {
            engines.push(engine);
        }
    }
    engines
}

pub fn open(
    engine: Engine,
    query: &str,
    context: Option<&gio::AppLaunchContext>,
) -> Result<(), String> {
    open_url(&engine.url(query), context)
}

pub fn open_url(url: &str, context: Option<&gio::AppLaunchContext>) -> Result<(), String> {
    if gio::AppInfo::launch_default_for_uri(url, context).is_ok() {
        return Ok(());
    }
    if utils::has_command("xdg-open") {
        return utils::spawn_detached(Command::new("xdg-open").arg(url));
    }
    Err("Could not open a browser".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_carry_the_encoded_query() {
        assert_eq!(
            Engine::Google.url("rust gtk"),
            "https://www.google.com/search?q=rust%20gtk"
        );
        assert_eq!(
            Engine::Youtube.url("a&b"),
            "https://www.youtube.com/results?search_query=a%26b"
        );
        assert_eq!(Engine::Chatgpt.url("hello"), "https://chatgpt.com/?q=hello");
    }

    #[test]
    fn fallback_order_follows_config() {
        let mut config = SearchConfig::default();
        assert_eq!(
            fallback_engines(&config),
            vec![Engine::Google, Engine::Youtube, Engine::Chatgpt]
        );

        config.default_engine = "youtube".to_string();
        assert_eq!(fallback_engines(&config)[0], Engine::Youtube);

        config.youtube_enabled = false;
        config.chatgpt_enabled = false;
        assert_eq!(fallback_engines(&config), vec![Engine::Google]);

        config.default_engine = "duckduckgo".to_string();
        assert_eq!(
            fallback_engines(&config),
            vec![Engine::DuckDuckGo, Engine::Google]
        );

        config.default_engine = "nonsense".to_string();
        assert_eq!(fallback_engines(&config), vec![Engine::Google]);
    }
}
