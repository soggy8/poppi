//! Turns the text in the search bar into an intent. Pure: no I/O, no
//! process spawning, nothing that depends on what is installed.

use crate::calculator::Calculator;
use crate::search::Engine;
use crate::units;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Empty,
    /// A question for an AI model: anything ending in `?`, or after `ask `
    /// or a model's name. `model` is `None` for the default model.
    Ask {
        question: String,
        model: Option<String>,
    },
    Settings,
    Setup,
    /// Window switcher with an optional filter.
    Windows(String),
    Search {
        engine: Engine,
        query: String,
    },
    Emoji(String),
    /// `5 km in miles`, `20 usd to eur`: resolved against units or rates.
    Convert(units::Request),
    /// Clipboard history, optionally filtered.
    Clipboard(String),
    /// A calculation, base/color/timezone conversion, and optional RGBA swatch.
    Calc {
        expression: String,
        result: String,
        swatch: Option<(u8, u8, u8, u8)>,
    },
    /// File search by name.
    Files(String),
    /// File content search (`g` / `grep`).
    Grep(String),
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
    /// A script in `~/.config/poppi_launcher/scripts/` matched by its keyword.
    Script {
        keyword: String,
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
    pub ai: bool,
    pub clipboard: bool,
    pub aliases: BTreeMap<String, String>,
    /// Keywords of executable scripts in `~/.config/poppi_launcher/scripts/`.
    pub scripts: BTreeSet<String>,
    /// Names of the AI models, each usable as a prefix.
    pub models: BTreeSet<String>,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            youtube: true,
            chatgpt: true,
            calculator: true,
            files: true,
            ai: true,
            clipboard: true,
            aliases: BTreeMap::new(),
            scripts: BTreeSet::new(),
            models: ["haiku", "sonnet"].map(String::from).into(),
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

/// `sonnet why is the sky blue` -> (`sonnet`, `why is the sky blue`).
pub fn model_prefix<'a>(query: &'a str, models: &BTreeSet<String>) -> Option<(&'a str, &'a str)> {
    let (first, rest) = query.trim().split_once(char::is_whitespace)?;
    let rest = rest.trim();
    (models.contains(first) && rest.chars().count() >= 2).then_some((first, rest))
}

fn ask(question: &str, model: Option<&str>) -> Intent {
    Intent::Ask {
        question: question.to_string(),
        model: model.map(String::from),
    }
}

pub fn parse(query: &str, options: &ParseOptions) -> Intent {
    let query = query.trim();
    if query.is_empty() {
        return Intent::Empty;
    }

    // A trailing question mark wins over every other mode.
    if options.ai && query.ends_with('?') && query.trim_end_matches('?').trim().chars().count() >= 2
    {
        return match model_prefix(query, &options.models) {
            Some((model, question)) => ask(question, Some(model)),
            None => ask(query, None),
        };
    }

    if strip_keyword(query, "poppi settings").is_some() {
        return Intent::Settings;
    }
    if strip_keyword(query, "poppi setup").is_some() {
        return Intent::Setup;
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

    let first_lower = first_word.to_lowercase();
    if options.scripts.contains(&first_lower) {
        return Intent::Script {
            keyword: first_lower,
            args: rest.to_string(),
        };
    }

    // Explicit ways to ask without a question mark: a model's name, or
    // `ask`/`ai` for the default model.
    if options.ai {
        if let Some((model, question)) = model_prefix(query, &options.models) {
            return ask(question, Some(model));
        }
        if let Some(rest) = strip_any(query, &["ask", "ai"]).filter(|r| r.chars().count() >= 2) {
            return ask(rest, None);
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
        if let Some(rest) = strip_any(query, &["g", "grep"]).filter(|r| !r.is_empty()) {
            return Intent::Grep(rest.to_string());
        }
    }

    if query.starts_with('/') || query == "~" || query.starts_with("~/") {
        return Intent::Path(query.to_string());
    }

    if let Some(command) = terminal_command(query) {
        return Intent::Terminal(command.to_string());
    }

    if options.calculator {
        if let Some(conv) = crate::conversions::evaluate(query) {
            return Intent::Calc {
                expression: conv.expression,
                result: conv.result,
                swatch: conv.swatch,
            };
        }
        if let Some(request) = units::parse(query) {
            return Intent::Convert(request);
        }
    }

    if options.calculator && Calculator::is_calculation(query) {
        if let Ok(value) = Calculator::evaluate(query) {
            return Intent::Calc {
                expression: query.to_string(),
                result: Calculator::format_result(value),
                swatch: None,
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
        assert_eq!(
            p("term cargo build"),
            Intent::Terminal("cargo build".into())
        );
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
            swatch: None,
        };
        assert_eq!(p("2+2"), calc("2+2", "4"));
        assert_eq!(p("(5+3)*2"), calc("(5+3)*2", "16"));
        assert_eq!(p("3x4"), calc("3x4", "12"));
        assert_eq!(p("15% of 240"), calc("15% of 240", "36"));
        assert_eq!(p("255 in hex"), calc("255 in hex", "0xff"));
        assert!(matches!(
            p("#4a9eff"),
            Intent::Calc {
                swatch: Some((74, 158, 255, 255)),
                ..
            }
        ));
        assert_eq!(p("2048"), Intent::Apps("2048".into()));
        assert_eq!(p("gtk4-demo"), Intent::Apps("gtk4-demo".into()));

        let options = ParseOptions {
            calculator: false,
            ..ParseOptions::default()
        };
        assert_eq!(parse("2+2", &options), Intent::Apps("2+2".into()));
        assert_eq!(parse("#4a9eff", &options), Intent::Apps("#4a9eff".into()));
    }

    #[test]
    fn files_and_paths() {
        assert_eq!(p("f report"), Intent::Files("report".into()));
        assert_eq!(
            p("file annual report"),
            Intent::Files("annual report".into())
        );
        assert_eq!(p("g TODO"), Intent::Grep("TODO".into()));
        assert_eq!(p("grep"), Intent::Apps("grep".into()));
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

    fn asked(question: &str) -> Intent {
        ask(question, None)
    }

    #[test]
    fn questions_go_to_the_default_model() {
        assert_eq!(
            p("what is the capital of slovenia?"),
            asked("what is the capital of slovenia?")
        );
        // The question mark beats prefixes, calculations and app names.
        assert_eq!(p("yt or vimeo?"), asked("yt or vimeo?"));
        assert_eq!(p("2+2?"), asked("2+2?"));
        assert_eq!(p("firefox?  "), asked("firefox?"));
        // Not a question yet.
        assert_eq!(p("?"), Intent::Apps("?".into()));
        assert_eq!(p("a?"), Intent::Apps("a?".into()));
        assert_eq!(p("what is rust"), Intent::Apps("what is rust".into()));
        assert_eq!(p("why? because"), Intent::Apps("why? because".into()));

        let options = ParseOptions {
            ai: false,
            ..ParseOptions::default()
        };
        assert_eq!(
            parse("what is rust?", &options),
            Intent::Apps("what is rust?".into())
        );
        assert_eq!(
            parse("sonnet why", &options),
            Intent::Apps("sonnet why".into())
        );
    }

    #[test]
    fn model_names_pick_the_model() {
        assert_eq!(
            p("sonnet explain lifetimes"),
            ask("explain lifetimes", Some("sonnet"))
        );
        assert_eq!(
            p("haiku what is rust?"),
            ask("what is rust?", Some("haiku"))
        );
        // The name alone, or with too little after it, is still a search.
        assert_eq!(p("sonnet"), Intent::Apps("sonnet".into()));
        assert_eq!(p("sonnet x"), Intent::Apps("sonnet x".into()));
        assert_eq!(
            p("sonnets of shakespeare?"),
            asked("sonnets of shakespeare?")
        );
        // An alias with the same name wins.
        let mut options = ParseOptions::default();
        options
            .aliases
            .insert("sonnet".into(), "https://x/{}".into());
        assert!(matches!(
            parse("sonnet why not", &options),
            Intent::Alias { .. }
        ));
    }

    #[test]
    fn ask_prefixes_and_clipboard() {
        assert_eq!(p("ask tell me a joke"), asked("tell me a joke"));
        assert_eq!(p("ai rust vs go"), asked("rust vs go"));
        assert_eq!(p("ask"), Intent::Apps("ask".into()));
        assert_eq!(p("airplane"), Intent::Apps("airplane".into()));
        assert_eq!(p("clip"), Intent::Clipboard("".into()));
        assert_eq!(p("cb http"), Intent::Clipboard("http".into()));
        assert_eq!(p("clipboard"), Intent::Clipboard("".into()));

        let options = ParseOptions {
            ai: false,
            clipboard: false,
            ..ParseOptions::default()
        };
        assert_eq!(parse("ask why", &options), Intent::Apps("ask why".into()));
        assert_eq!(parse("clip", &options), Intent::Apps("clip".into()));
    }

    #[test]
    fn conversions() {
        assert!(
            matches!(p("5 km in miles"), Intent::Convert(r) if r.from == "km" && r.to == "miles")
        );
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
        assert_eq!(p("poppi setup"), Intent::Setup);
    }

    #[test]
    fn scripts_match_first_word_after_aliases() {
        let mut options = ParseOptions::default();
        options.scripts.insert("ssh".into());
        assert_eq!(
            parse("ssh prod-1", &options),
            Intent::Script {
                keyword: "ssh".into(),
                args: "prod-1".into(),
            }
        );
        options.aliases.insert("ssh".into(), "kitty ssh {}".into());
        assert!(matches!(
            parse("ssh prod-1", &options),
            Intent::Alias { .. }
        ));
    }
}
