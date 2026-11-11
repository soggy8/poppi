//! Turns the text in the search bar into an intent. Pure: no I/O, no
//! process spawning, nothing that depends on what is installed.

use crate::calculator::Calculator;
use crate::search::Engine;
use crate::units;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Empty,
    /// A question for Claude: anything ending in `?`.
    Ask(String),
    Settings,
    /// Window switcher with an optional filter.
    Windows(String),
    Search { engine: Engine, query: String },
    Emoji(String),
    /// `5 km in miles`, `20 usd to eur`: resolved against units or rates.
    Convert(units::Request),
    /// Clipboard history, optionally filtered.
    Clipboard(String),
    /// A calculation and its formatted result.
    Calc { expression: String, result: String },
    /// File search by name.
    Files(String),
    /// A typed path (`/etc/`, `~/Doc`) to browse.
    Path(String),
    /// Past queries, optionally filtered.
    History(String),
    /// A user-defined alias with the text typed after it.
    Alias {
        name: String,
        template: String,
        args: String,
    },
    /// An explicitly prefixed terminal command, prefix removed.
    Terminal(String),
    Apps(String),
}

#[derive(Debug, Clone)]
pub struct ParseOptions {
    pub youtube: bool,
    pub chatgpt: bool,
    pub calculator: bool,
    pub files: bool,
    pub claude: bool,
    pub clipboard: bool,
    pub aliases: BTreeMap<String, String>,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            youtube: true,
            chatgpt: true,
            calculator: true,
            files: true,
            claude: true,
            clipboard: true,
            aliases: BTreeMap::new(),
        }
    }
}

/// `keyword` alone gives `Some("")`; `keyword rest` gives `Some("rest")`.
fn strip_keyword<'a>(query: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = query.strip_prefix(keyword)?;
    if rest.is_empty() {
        Some("")
    } else if rest.starts_with(char::is_whitespace) {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn strip_any<'a>(query: &'a str, keywords: &[&str]) -> Option<&'a str> {
    keywords.iter().find_map(|k| strip_keyword(query, k))
}

fn terminal_command(query: &str) -> Option<&str> {
    let rest = if let Some(rest) = query.strip_prefix('>').or_else(|| query.strip_prefix('$')) {
        rest
    } else {
        strip_any(query, &["!", "term", "cmd"])?
    };
    let rest = rest.trim();
    (!rest.is_empty()).then_some(rest)
}

pub fn parse(query: &str, options: &ParseOptions) -> Intent {
    let query = query.trim();
    if query.is_empty() {
        return Intent::Empty;
    }

    // A trailing question mark wins over every other mode.
    if options.claude && query.ends_with('?') && query.trim_end_matches('?').trim().chars().count() >= 2 {
        return Intent::Ask(query.to_string());
    }

    if strip_keyword(query, "poppi settings").is_some() {
        return Intent::Settings;
    }

    // Aliases come first so they can shadow the built-in keywords.
    let (first_word, rest) = match query.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (query, ""),
    };
    if let Some(template) = options.aliases.get(first_word) {
        return Intent::Alias {
            name: first_word.to_string(),
            template: template.clone(),
            args: rest.to_string(),
        };
    }

    // Explicit ways to reach Claude without a question mark.
    if options.claude {
        if let Some(rest) = strip_any(query, &["ask", "ai"]).filter(|r| r.chars().count() >= 2) {
            return Intent::Ask(rest.to_string());
        }
    }

    if options.clipboard {
        if let Some(filter) = strip_any(query, &["clip", "cb", "clipboard"]) {
            return Intent::Clipboard(filter.to_string());
        }
    }

    if let Some(filter) = strip_keyword(query, "history") {
        return Intent::History(filter.to_string());
    }

    if let Some(filter) = strip_any(query, &["sw", "switch"]) {
        return Intent::Windows(filter.to_string());
    }

    let engines: [(Engine, &[&str], bool); 4] = [
        (Engine::Youtube, &["yt", "youtube"], options.youtube),
        (Engine::Chatgpt, &["gpt", "chatgpt"], options.chatgpt),
        (Engine::Google, &["google"], true),
        (Engine::DuckDuckGo, &["ddg"], true),
    ];
    for (engine, keywords, enabled) in engines {
        if !enabled {
            continue;
        }
        if let Some(rest) = strip_any(query, keywords).filter(|r| !r.is_empty()) {
            return Intent::Search {
                engine,
                query: rest.to_string(),
            };
        }
    }

    if let Some(rest) = query.strip_prefix(':') {
        return Intent::Emoji(rest.trim().to_string());
    }
    if let Some(rest) = strip_keyword(query, "emoji") {
        return Intent::Emoji(rest.to_string());
    }

    if options.files {
        if let Some(rest) = strip_any(query, &["f", "file"]).filter(|r| !r.is_empty()) {
            return Intent::Files(rest.to_string());
        }
    }

    if query.starts_with('/') || query == "~" || query.starts_with("~/") {
        return Intent::Path(query.to_string());
    }

    if let Some(command) = terminal_command(query) {
        return Intent::Terminal(command.to_string());
    }

    if options.calculator {
        if let Some(request) = units::parse(query) {
            return Intent::Convert(request);
        }
    }

    if options.calculator && Calculator::is_calculation(query) {
        if let Ok(value) = Calculator::evaluate(query) {
            return Intent::Calc {
                expression: query.to_string(),
                result: Calculator::format_result(value),
            };
        }
    }

    Intent::Apps(query.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(query: &str) -> Intent {
        parse(query, &ParseOptions::default())
    }

    #[test]
    fn empty_and_whitespace() {
        assert_eq!(p(""), Intent::Empty);
        assert_eq!(p("   "), Intent::Empty);
    }

    #[test]
    fn plain_words_are_app_searches_even_if_they_are_commands() {
        assert_eq!(p("firefox"), Intent::Apps("firefox".into()));
        assert_eq!(p("code"), Intent::Apps("code".into()));
        assert_eq!(p("git"), Intent::Apps("git".into()));
        assert_eq!(p("settings"), Intent::Apps("settings".into()));
        // Shell metacharacters are inert text here.
        assert_eq!(p("foo;poweroff"), Intent::Apps("foo;poweroff".into()));
    }

    #[test]
    fn terminal_prefixes_are_stripped() {
        assert_eq!(p("> ls -la"), Intent::Terminal("ls -la".into()));
        assert_eq!(p(">ls"), Intent::Terminal("ls".into()));
        assert_eq!(p("$ git status"), Intent::Terminal("git status".into()));
        assert_eq!(p("! htop"), Intent::Terminal("htop".into()));
        assert_eq!(p("term cargo build"), Intent::Terminal("cargo build".into()));
        assert_eq!(p("cmd df -h"), Intent::Terminal("df -h".into()));
    }

    #[test]
    fn prefix_words_without_a_command_fall_through() {
        assert_eq!(p("term"), Intent::Apps("term".into()));
        assert_eq!(p("terminal"), Intent::Apps("terminal".into()));
        assert_eq!(p(">"), Intent::Apps(">".into()));
    }

    #[test]
    fn window_switcher() {
        assert_eq!(p("sw"), Intent::Windows("".into()));
        assert_eq!(p("switch"), Intent::Windows("".into()));
        assert_eq!(p("sw kit"), Intent::Windows("kit".into()));
        assert_eq!(p("swift"), Intent::Apps("swift".into()));
    }

    #[test]
    fn web_search() {
        assert_eq!(
            p("yt rust tutorial"),
            Intent::Search {
                engine: Engine::Youtube,
                query: "rust tutorial".into()
            }
        );
        assert_eq!(
            p("gpt hello"),
            Intent::Search {
                engine: Engine::Chatgpt,
                query: "hello".into()
            }
        );
        assert_eq!(
            p("google rust"),
            Intent::Search {
                engine: Engine::Google,
                query: "rust".into()
            }
        );
        assert_eq!(p("yt"), Intent::Apps("yt".into()));
        assert_eq!(p("google"), Intent::Apps("google".into()));
    }

    #[test]
    fn disabled_engines_are_not_matched() {
        let options = ParseOptions {
            youtube: false,
            chatgpt: false,
            ..ParseOptions::default()
        };
        assert_eq!(parse("yt cats", &options), Intent::Apps("yt cats".into()));
        assert_eq!(parse("gpt hi", &options), Intent::Apps("gpt hi".into()));
    }

    #[test]
    fn emoji() {
        assert_eq!(p(":heart"), Intent::Emoji("heart".into()));
        assert_eq!(p("emoji smile"), Intent::Emoji("smile".into()));
        assert_eq!(p("emoji"), Intent::Emoji("".into()));
    }

    #[test]
    fn calculator() {
        let calc = |expression: &str, result: &str| Intent::Calc {
            expression: expression.into(),
            result: result.into(),
        };
        assert_eq!(p("2+2"), calc("2+2", "4"));
        assert_eq!(p("(5+3)*2"), calc("(5+3)*2", "16"));
        assert_eq!(p("3x4"), calc("3x4", "12"));
        assert_eq!(p("2048"), Intent::Apps("2048".into()));
        assert_eq!(p("gtk4-demo"), Intent::Apps("gtk4-demo".into()));

        let options = ParseOptions {
            calculator: false,
            ..ParseOptions::default()
        };
        assert_eq!(parse("2+2", &options), Intent::Apps("2+2".into()));
    }

    #[test]
    fn files_and_paths() {
        assert_eq!(p("f report"), Intent::Files("report".into()));
        assert_eq!(p("file annual report"), Intent::Files("annual report".into()));
        assert_eq!(p("f"), Intent::Apps("f".into()));
        assert_eq!(p("files"), Intent::Apps("files".into()));
        assert_eq!(p("~/Doc"), Intent::Path("~/Doc".into()));
        assert_eq!(p("~"), Intent::Path("~".into()));
        assert_eq!(p("/etc/"), Intent::Path("/etc/".into()));

        let options = ParseOptions {
            files: false,
            ..ParseOptions::default()
        };
        assert_eq!(parse("f report", &options), Intent::Apps("f report".into()));
    }

    #[test]
    fn questions_go_to_claude() {
        assert_eq!(
            p("what is the capital of slovenia?"),
            Intent::Ask("what is the capital of slovenia?".into())
        );
        // The question mark beats prefixes, calculations and app names.
        assert_eq!(p("yt or vimeo?"), Intent::Ask("yt or vimeo?".into()));
        assert_eq!(p("2+2?"), Intent::Ask("2+2?".into()));
        assert_eq!(p("firefox?  "), Intent::Ask("firefox?".into()));
        // Not a question yet.
        assert_eq!(p("?"), Intent::Apps("?".into()));
        assert_eq!(p("a?"), Intent::Apps("a?".into()));
        assert_eq!(p("what is rust"), Intent::Apps("what is rust".into()));
        assert_eq!(p("why? because"), Intent::Apps("why? because".into()));

        let options = ParseOptions {
            claude: false,
            ..ParseOptions::default()
        };
        assert_eq!(parse("what is rust?", &options), Intent::Apps("what is rust?".into()));
    }

    #[test]
    fn ask_prefixes_and_clipboard() {
        assert_eq!(p("ask tell me a joke"), Intent::Ask("tell me a joke".into()));
        assert_eq!(p("ai rust vs go"), Intent::Ask("rust vs go".into()));
        assert_eq!(p("ask"), Intent::Apps("ask".into()));
        assert_eq!(p("airplane"), Intent::Apps("airplane".into()));
        assert_eq!(p("clip"), Intent::Clipboard("".into()));
        assert_eq!(p("cb http"), Intent::Clipboard("http".into()));
        assert_eq!(p("clipboard"), Intent::Clipboard("".into()));

        let options = ParseOptions {
            claude: false,
            clipboard: false,
            ..ParseOptions::default()
        };
        assert_eq!(parse("ask why", &options), Intent::Apps("ask why".into()));
        assert_eq!(parse("clip", &options), Intent::Apps("clip".into()));
    }

    #[test]
    fn conversions() {
        assert!(matches!(p("5 km in miles"), Intent::Convert(r) if r.from == "km" && r.to == "miles"));
        assert!(matches!(p("100 usd to eur"), Intent::Convert(r) if r.amount == 100.0));
        // No separator, no conversion: still arithmetic or an app search.
        assert!(matches!(p("5 * 3"), Intent::Calc { .. }));
        assert_eq!(p("5 km"), Intent::Apps("5 km".into()));
    }

    #[test]
    fn history() {
        assert_eq!(p("history"), Intent::History("".into()));
        assert_eq!(p("history yt"), Intent::History("yt".into()));
    }

    #[test]
    fn aliases_take_the_first_word_and_shadow_builtins() {
        let mut options = ParseOptions::default();
        options
            .aliases
            .insert("gh".into(), "https://github.com/search?q={}".into());
        options.aliases.insert("yt".into(), "mpv".into());

        assert_eq!(
            parse("gh gtk4 rust", &options),
            Intent::Alias {
                name: "gh".into(),
                template: "https://github.com/search?q={}".into(),
                args: "gtk4 rust".into()
            }
        );
        assert_eq!(
            parse("gh", &options),
            Intent::Alias {
                name: "gh".into(),
                template: "https://github.com/search?q={}".into(),
                args: "".into()
            }
        );
        assert!(matches!(parse("yt cats", &options), Intent::Alias { .. }));
        assert_eq!(parse("ghost", &options), Intent::Apps("ghost".into()));
    }

    #[test]
    fn settings() {
        assert_eq!(p("poppi settings"), Intent::Settings);
    }
}
