//! What the launcher knows and what it would show for a given query.
//! No widgets here; the `ui` module renders `results`.

use crate::alias::{self, AliasTarget};
use crate::app_launcher::{App, AppLauncher};
use crate::clipboard::ClipboardHistory;
use crate::config::Config;
use crate::emoji_picker::{self, Emoji, EmojiPicker};
use crate::files::{self, FileEntry};
use crate::history::History;
use crate::matching::Matcher;
use crate::query::{self, Intent, ParseOptions};
use crate::search::{self, Engine};
use crate::system::SystemAction;
use crate::terminal::Terminal;
use crate::units::{self, Rates};
use crate::usage::{self, Usage};
use crate::window_switcher::{self, OpenWindow};
use gtk::gio;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const EMOJI_COLUMNS: usize = 8;
/// Emoji rows visible at once; the grid scrolls beyond that.
pub const EMOJI_VISIBLE_ROWS: usize = 3;
pub const EMOJI_LIMIT: usize = 96;
const EMOJI_RECENT: usize = 16;
/// Most results kept for one query. `behavior.max_results` only decides how
/// many are visible without scrolling.
pub const RESULT_LIMIT: usize = 50;
/// The file index is rebuilt in the background when older than this.
const FILE_INDEX_MAX_AGE: Duration = Duration::from_secs(300);
const MIN_FILE_QUERY: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub enum TurnStatus {
    Thinking,
    Answering,
    Done,
    Failed,
}

/// One question to Claude and what has come back so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub question: String,
    pub text: String,
    pub status: TurnStatus,
}

impl Turn {
    pub fn is_finished(&self) -> bool {
        matches!(self.status, TurnStatus::Done | TurnStatus::Failed)
    }
}

#[derive(Clone, Debug)]
pub enum ResultItem {
    App(App),
    Calc { expression: String, result: String },
    Emoji(Emoji),
    Terminal(String),
    Search { engine: Engine, query: String },
    Window { window: OpenWindow, icon: Option<gio::Icon> },
    File(FileEntry),
    Alias { name: String, target: AliasTarget },
    /// A past query; activating it puts it back in the search bar.
    History(String),
    ClearHistory,
    /// A text from the clipboard history.
    Clipboard(String),
    System(SystemAction),
    /// Second step for a system action that has no confirmation of its own.
    Confirm(SystemAction),
    /// Offer to send a question to Claude; nothing is sent until Enter.
    Ask { question: String, model: String },
    /// The conversation with Claude so far.
    Transcript { turns: Vec<Turn>, model: String },
    Settings,
    /// A message row; activating it does nothing.
    Info(String),
}

/// What kind of thing the current query is, shown next to the search bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Empty,
    Apps,
    Files,
    Windows,
    Web,
    Emoji,
    Calculator,
    Terminal,
    Alias,
    History,
    Clipboard,
    Claude,
    Settings,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Empty => "",
            Mode::Apps => "Apps",
            Mode::Files => "Files",
            Mode::Windows => "Windows",
            Mode::Web => "Web",
            Mode::Emoji => "Emoji",
            Mode::Calculator => "Calculator",
            Mode::Terminal => "Terminal",
            Mode::Alias => "Alias",
            Mode::History => "History",
            Mode::Clipboard => "Clipboard",
            Mode::Claude => "Claude",
            Mode::Settings => "Settings",
        }
    }
}

pub enum WindowList {
    NotLoaded,
    Loading,
    Loaded(Vec<OpenWindow>),
    Failed(String),
}

/// Background work `update` asks the UI to start.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Needs {
    pub windows: bool,
    pub files: bool,
    pub rates: bool,
    /// Start the Claude CLI now so it is ready when the question is sent.
    pub claude: bool,
}

pub struct LauncherState {
    pub apps: AppLauncher,
    pub usage: Usage,
    pub history: History,
    pub emoji_recent: History,
    pub clipboard: ClipboardHistory,
    pub windows: WindowList,
    pub results: Vec<ResultItem>,
    pub mode: Mode,
    /// The part of the query that result titles were matched against.
    pub highlight: String,
    pub emoji_mode: bool,
    /// The conversation with Claude; cleared when the launcher is dismissed.
    pub chat: Vec<Turn>,
    rates: Option<Rates>,
    rates_loading: bool,
    rates_checked: bool,
    file_index: Option<Rc<Vec<FileEntry>>>,
    file_index_built: Option<Instant>,
    file_index_loading: bool,
    emoji_picker: Option<(String, EmojiPicker)>,
    matcher: Matcher,
}

impl LauncherState {
    pub fn new(config: &Config) -> Self {
        Self::with_stores(
            Usage::load(),
            History::load(config.behavior.history_size),
            History::load_named("emoji_recent.json", EMOJI_RECENT),
            config,
        )
    }

    fn with_stores(usage: Usage, history: History, emoji_recent: History, config: &Config) -> Self {
        Self {
            apps: AppLauncher::empty(),
            usage,
            history,
            emoji_recent,
            clipboard: ClipboardHistory::new(config.clipboard.size),
            windows: WindowList::NotLoaded,
            results: Vec::new(),
            mode: Mode::Empty,
            highlight: String::new(),
            emoji_mode: false,
            chat: Vec::new(),
            rates: None,
            rates_loading: false,
            rates_checked: false,
            file_index: None,
            file_index_built: None,
            file_index_loading: false,
            emoji_picker: None,
            matcher: Matcher::new(),
        }
    }

    /// Forget per-session data when the launcher is shown again.
    pub fn reset_session(&mut self) {
        self.windows = WindowList::NotLoaded;
        self.results.clear();
        self.emoji_mode = false;
        self.chat.clear();
    }

    /// True while Claude's conversation is what is shown.
    pub fn answer_mode(&self) -> bool {
        matches!(self.results.first(), Some(ResultItem::Transcript { .. }))
    }

    /// True while an answer is still on its way.
    pub fn chat_busy(&self) -> bool {
        self.chat.last().is_some_and(|turn| !turn.is_finished())
    }

    pub fn matcher(&self) -> &Matcher {
        &self.matcher
    }

    pub fn set_file_index(&mut self, entries: Vec<FileEntry>) {
        self.file_index = Some(Rc::new(entries));
        self.file_index_built = Some(Instant::now());
        self.file_index_loading = false;
    }

    /// `None` when the download failed; the cached rates, if any, stay.
    pub fn set_rates(&mut self, rates: Option<Rates>) {
        if rates.is_some() {
            self.rates = rates;
        }
        self.rates_loading = false;
    }

    /// Recompute `results` for `query` and report which background loads the
    /// caller must start (calling `update` again once they finish).
    pub fn update(&mut self, query: &str, config: &Config) -> Needs {
        let options = ParseOptions {
            youtube: config.search.youtube_enabled,
            chatgpt: config.search.chatgpt_enabled,
            calculator: config.calculator.enabled,
            files: config.files.enabled,
            claude: config.claude.enabled,
            clipboard: config.clipboard.enabled,
            aliases: config.aliases.clone(),
        };
        self.history.set_capacity(config.behavior.history_size);
        self.clipboard
            .set_capacity(if config.clipboard.enabled { config.clipboard.size } else { 0 });

        self.emoji_mode = false;
        self.highlight.clear();
        let mut needs = Needs::default();

        // A conversation takes over the bar: whatever is typed next is a
        // follow-up, with or without a question mark.
        if !self.chat.is_empty() && config.claude.enabled {
            let mut items = vec![ResultItem::Transcript {
                turns: self.chat.clone(),
                model: config.claude.model.clone(),
            }];
            let question = query.trim();
            if !question.is_empty() {
                items.push(ResultItem::Ask {
                    question: question.to_string(),
                    model: config.claude.model.clone(),
                });
            }
            self.mode = Mode::Claude;
            self.results = items;
            needs.claude = true;
            return needs;
        }

        let (mode, results) = match query::parse(query, &options) {
            Intent::Empty => (
                Mode::Empty,
                self.apps
                    .most_used(&self.usage, config.behavior.max_results)
                    .into_iter()
                    .map(|app| ResultItem::App(app.clone()))
                    .collect(),
            ),
            Intent::Ask(question) => {
                needs.claude = true;
                (
                    Mode::Claude,
                    vec![ResultItem::Ask {
                        question,
                        model: config.claude.model.clone(),
                    }],
                )
            }
            Intent::Settings => (Mode::Settings, vec![ResultItem::Settings]),
            Intent::Windows(filter) => {
                if matches!(self.windows, WindowList::NotLoaded) {
                    self.windows = WindowList::Loading;
                    needs.windows = true;
                }
                let items = self.window_results(&filter);
                self.highlight = filter;
                (Mode::Windows, items)
            }
            Intent::Search { engine, query } => {
                (Mode::Web, vec![ResultItem::Search { engine, query }])
            }
            Intent::Emoji(filter) => {
                self.emoji_mode = true;
                let tone = config.emoji.skin_tone.clone();
                if self.emoji_picker.as_ref().map(|(key, _)| key.as_str()) != Some(tone.as_str()) {
                    self.emoji_picker =
                        Some((tone.clone(), EmojiPicker::new(emoji_picker::skin_tone(&tone))));
                }
                let items = self
                    .emoji_picker
                    .as_ref()
                    .map(|(_, picker)| picker.search(&filter, self.emoji_recent.entries(), EMOJI_LIMIT))
                    .unwrap_or_default()
                    .into_iter()
                    .map(ResultItem::Emoji)
                    .collect();
                (Mode::Emoji, items)
            }
            Intent::Calc { expression, result } => {
                (Mode::Calculator, vec![ResultItem::Calc { expression, result }])
            }
            Intent::Convert(request) => {
                let expression = query.trim().to_string();
                if let Some(result) = units::convert(&request) {
                    (Mode::Calculator, vec![ResultItem::Calc { expression, result }])
                } else if config.calculator.currency && units::looks_like_currency(&request) {
                    (Mode::Calculator, self.currency_results(&request, expression, &mut needs))
                } else {
                    // Not a conversion we know: treat it like any other text.
                    self.highlight = expression.clone();
                    (Mode::Apps, self.app_results(&expression, config))
                }
            }
            Intent::Terminal(command) => (Mode::Terminal, vec![ResultItem::Terminal(command)]),
            Intent::Files(filter) => {
                let stale = self
                    .file_index_built
                    .map(|built| built.elapsed() > FILE_INDEX_MAX_AGE)
                    .unwrap_or(true);
                if stale && !self.file_index_loading {
                    self.file_index_loading = true;
                    needs.files = true;
                }
                let items = self.file_results(&filter);
                self.highlight = filter;
                (Mode::Files, items)
            }
            Intent::Path(path) => {
                let entries = files::complete_path(&path, RESULT_LIMIT);
                let items = if entries.is_empty() {
                    vec![ResultItem::Info("No such file or directory".to_string())]
                } else {
                    entries.into_iter().map(ResultItem::File).collect()
                };
                (Mode::Files, items)
            }
            Intent::History(filter) => (Mode::History, self.history_results(&filter)),
            Intent::Clipboard(filter) => {
                let found = self.clipboard.search(&filter, RESULT_LIMIT);
                let items = if found.is_empty() {
                    let message = if self.clipboard.is_empty() {
                        "Nothing copied yet. Texts on the clipboard while the launcher is open are remembered."
                    } else {
                        "No matching clipboard entries"
                    };
                    vec![ResultItem::Info(message.to_string())]
                } else {
                    found.into_iter().map(ResultItem::Clipboard).collect()
                };
                (Mode::Clipboard, items)
            }
            Intent::Alias {
                name,
                template,
                args,
            } => (
                Mode::Alias,
                vec![ResultItem::Alias {
                    target: alias::expand(&template, &args),
                    name,
                }],
            ),
            Intent::Apps(query) => {
                let items = self.app_results(&query, config);
                self.highlight = query;
                (Mode::Apps, items)
            }
        };

        self.mode = mode;
        self.results = results;
        needs
    }

    fn currency_results(
        &mut self,
        request: &units::Request,
        expression: String,
        needs: &mut Needs,
    ) -> Vec<ResultItem> {
        if !self.rates_checked {
            self.rates_checked = true;
            let (cached, stale) = Rates::load_cached();
            self.rates = cached;
            if stale {
                self.rates_loading = true;
                needs.rates = true;
            }
        }
        match &self.rates {
            Some(rates) => match rates.convert(request) {
                Some(result) => vec![ResultItem::Calc {
                    expression: format!("{}  ·  ECB rate of {}", expression, rates.date),
                    result,
                }],
                None => vec![ResultItem::Info("Unknown currency".to_string())],
            },
            None if self.rates_loading => {
                vec![ResultItem::Info("Fetching exchange rates…".to_string())]
            }
            None => vec![ResultItem::Info(
                "Exchange rates are not available right now".to_string(),
            )],
        }
    }

    fn window_results(&self, filter: &str) -> Vec<ResultItem> {
        match &self.windows {
            WindowList::NotLoaded | WindowList::Loading => {
                vec![ResultItem::Info("Loading windows…".to_string())]
            }
            WindowList::Failed(error) => vec![ResultItem::Info(error.clone())],
            WindowList::Loaded(windows) => {
                let found = window_switcher::search(&self.matcher, filter, windows);
                if found.is_empty() {
                    let message = if windows.is_empty() {
                        "No open windows found"
                    } else {
                        "No matching windows"
                    };
                    return vec![ResultItem::Info(message.to_string())];
                }
                found
                    .into_iter()
                    .take(RESULT_LIMIT)
                    .map(|window| ResultItem::Window {
                        icon: self.apps.icon_for_window_class(&window.class),
                        window: window.clone(),
                    })
                    .collect()
            }
        }
    }

    fn file_results(&self, filter: &str) -> Vec<ResultItem> {
        if filter.chars().count() < MIN_FILE_QUERY {
            return vec![ResultItem::Info(format!(
                "Type at least {} characters to search files",
                MIN_FILE_QUERY
            ))];
        }
        // While a rebuild runs, the previous index keeps answering.
        let Some(index) = &self.file_index else {
            return vec![ResultItem::Info("Indexing files…".to_string())];
        };
        let found = files::search(filter, index, &self.usage, usage::now(), RESULT_LIMIT);
        if found.is_empty() {
            return vec![ResultItem::Info("No matching files".to_string())];
        }
        found
            .into_iter()
            .map(|entry| ResultItem::File(entry.clone()))
            .collect()
    }

    fn history_results(&self, filter: &str) -> Vec<ResultItem> {
        let filter = filter.to_lowercase();
        if filter == "clear" && !self.history.entries().is_empty() {
            return vec![ResultItem::ClearHistory];
        }
        let found: Vec<ResultItem> = self
            .history
            .entries()
            .iter()
            .filter(|entry| entry.to_lowercase().contains(&filter))
            .take(RESULT_LIMIT)
            .map(|entry| ResultItem::History(entry.clone()))
            .collect();
        if found.is_empty() {
            let message = if self.history.entries().is_empty() {
                "No history yet"
            } else {
                "No matching history"
            };
            return vec![ResultItem::Info(message.to_string())];
        }
        found
    }

    /// System actions whose name or keywords match, best first, with whether
    /// the query names the action exactly.
    fn system_results(&self, query: &str) -> Vec<(SystemAction, bool)> {
        let query = query.trim().to_lowercase();
        if query.chars().count() < 3 {
            return Vec::new();
        }
        let mut found: Vec<(SystemAction, i64)> = SystemAction::ALL
            .into_iter()
            .filter_map(|action| {
                action
                    .keywords()
                    .iter()
                    .filter_map(|keyword| Matcher::strict(&query, keyword))
                    .max()
                    // Prefix matches only: "lock" yes, "clock" no.
                    .filter(|score| *score >= 850)
                    .map(|score| (action, score))
            })
            .collect();
        found.sort_by(|a, b| b.1.cmp(&a.1));
        found
            .into_iter()
            .map(|(action, _)| (action, action.keywords().contains(&query.as_str())))
            .collect()
    }

    fn app_results(&self, query: &str, config: &Config) -> Vec<ResultItem> {
        let mut items: Vec<ResultItem> = self
            .apps
            .search(query, &self.usage, RESULT_LIMIT)
            .into_iter()
            .map(|app| ResultItem::App(app.clone()))
            .collect();

        // Typing an action's exact name puts it first; otherwise it follows
        // the applications.
        for (action, exact) in self.system_results(query).into_iter().rev() {
            if exact {
                items.insert(0, ResultItem::System(action));
            } else {
                items.push(ResultItem::System(action));
            }
        }

        // "settings", "poppi", "launcher"... also offer our own settings.
        if query.len() >= 3 && "poppi launcher settings".contains(&query.to_lowercase()) {
            items.push(ResultItem::Settings);
        }

        // Apps always come first. A command on $PATH is offered after them,
        // or leads the list when nothing else matched.
        let had_matches = !items.is_empty();
        if Terminal::looks_like_command(query) {
            items.push(ResultItem::Terminal(query.to_string()));
        }
        if !had_matches {
            items.extend(
                search::fallback_engines(&config.search)
                    .into_iter()
                    .map(|engine| ResultItem::Search {
                        engine,
                        query: query.to_string(),
                    }),
            );
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> LauncherState {
        LauncherState::with_stores(
            Usage::load_from(None),
            History::load_from(None, 50),
            History::load_from(None, EMOJI_RECENT),
            &Config::default(),
        )
    }

    #[test]
    fn empty_query_without_usage_shows_nothing() {
        let mut s = state();
        assert_eq!(s.update("", &Config::default()), Needs::default());
        assert!(s.results.is_empty());
        assert_eq!(s.mode, Mode::Empty);
    }

    #[test]
    fn unknown_text_falls_back_to_web_search() {
        let mut s = state();
        s.update("zzqqy-no-such-thing", &Config::default());
        assert!(matches!(
            s.results.as_slice(),
            [
                ResultItem::Search { engine: Engine::Google, .. },
                ResultItem::Search { engine: Engine::Youtube, .. },
                ResultItem::Search { engine: Engine::Chatgpt, .. },
            ]
        ));
        assert_eq!(s.mode, Mode::Apps);
        assert_eq!(s.highlight, "zzqqy-no-such-thing");
    }

    #[test]
    fn a_path_command_is_offered_before_web_search() {
        let mut s = state();
        s.update("sh -c true", &Config::default());
        assert!(matches!(&s.results[0], ResultItem::Terminal(c) if c == "sh -c true"));
        assert!(matches!(s.results[1], ResultItem::Search { .. }));
    }

    #[test]
    fn window_mode_requests_the_list_once() {
        let mut s = state();
        let config = Config::default();
        assert!(s.update("sw", &config).windows);
        assert!(matches!(s.results.as_slice(), [ResultItem::Info(_)]));
        // Still loading: do not start a second load.
        assert!(!s.update("sw k", &config).windows);
        assert_eq!(s.highlight, "k");

        s.windows = WindowList::Loaded(Vec::new());
        assert!(!s.update("sw", &config).windows);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "No open windows found"));

        s.windows = WindowList::Failed("boom".to_string());
        s.update("sw", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "boom"));
    }

    #[test]
    fn emoji_mode_lists_recents_first() {
        let mut s = state();
        let config = Config::default();
        s.emoji_recent.push("🔥");
        s.update(":", &config);
        assert!(s.emoji_mode);
        assert_eq!(s.mode, Mode::Emoji);
        assert_eq!(s.results.len(), EMOJI_LIMIT);
        assert!(matches!(&s.results[0], ResultItem::Emoji(e) if e.emoji == "🔥"));

        s.update("2+2", &config);
        assert!(!s.emoji_mode);
        assert!(matches!(&s.results[0], ResultItem::Calc { result, .. } if result == "4"));
    }

    #[test]
    fn emoji_skin_tone_follows_the_config() {
        let mut s = state();
        let mut config = Config::default();
        s.update(":thumbs up", &config);
        assert!(matches!(&s.results[0], ResultItem::Emoji(e) if e.emoji == "👍"));
        config.emoji.skin_tone = "dark".to_string();
        s.update(":thumbs up", &config);
        assert!(matches!(&s.results[0], ResultItem::Emoji(e) if e.emoji == "👍🏿"));
    }

    #[test]
    fn settings_is_offered_for_partial_matches() {
        let mut s = state();
        s.update("settings", &Config::default());
        assert!(matches!(s.results.as_slice(), [ResultItem::Settings]));
    }

    #[test]
    fn system_actions_match_by_prefix_and_lead_when_exact() {
        let mut s = state();
        let config = Config::default();

        s.update("lock", &config);
        assert!(matches!(s.results[0], ResultItem::System(SystemAction::Lock)));
        s.update("reb", &config);
        assert!(s.results.iter().any(|r| matches!(r, ResultItem::System(SystemAction::Restart))));
        s.update("shutdown", &config);
        assert!(matches!(s.results[0], ResultItem::System(SystemAction::PowerOff)));

        // Not on a substring, and not for very short input.
        s.update("clock", &config);
        assert!(!s.results.iter().any(|r| matches!(r, ResultItem::System(_))));
        s.update("lo", &config);
        assert!(!s.results.iter().any(|r| matches!(r, ResultItem::System(_))));
    }

    #[test]
    fn unit_conversions_are_calculator_results() {
        let mut s = state();
        let config = Config::default();
        s.update("5 km in miles", &config);
        assert_eq!(s.mode, Mode::Calculator);
        assert!(matches!(&s.results[0], ResultItem::Calc { result, .. } if result == "3.10686 mi"));

        // Unknown units fall through to an ordinary search.
        s.update("5 blorps in km", &config);
        assert_eq!(s.mode, Mode::Apps);
    }

    #[test]
    fn currency_conversion_uses_loaded_rates() {
        let mut s = state();
        let mut config = Config::default();
        s.rates_checked = true;
        s.rates = Rates::parse(r#"{"date":"2026-11-10","rates":{"USD":1.25}}"#);
        s.update("100 usd in eur", &config);
        assert!(matches!(
            &s.results[0],
            ResultItem::Calc { result, expression } if result == "80.00 EUR" && expression.contains("2026-11-10")
        ));
        s.update("100 usd in zzz", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "Unknown currency"));

        config.calculator.currency = false;
        s.update("100 usd in eur", &config);
        assert_eq!(s.mode, Mode::Apps);
    }

    #[test]
    fn file_search_requests_the_index_once_and_then_answers() {
        let mut s = state();
        let config = Config::default();

        assert!(s.update("f report", &config).files);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "Indexing files…"));
        assert!(!s.update("f repor", &config).files);

        let dir = std::env::temp_dir().join(format!("poppi-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("report.txt"), "").unwrap();
        let files_config = crate::config::FilesConfig {
            roots: vec![dir.to_string_lossy().into_owned()],
            ..Default::default()
        };
        s.set_file_index(files::build_index(&files_config));
        let _ = std::fs::remove_dir_all(&dir);

        assert!(!s.update("f report", &config).files);
        assert!(matches!(&s.results[0], ResultItem::File(f) if f.name == "report.txt"));
        assert_eq!(s.highlight, "report");
        s.update("f zzzz", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "No matching files"));
        s.update("f r", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(_)));
    }

    #[test]
    fn history_lists_and_filters() {
        let mut s = state();
        let config = Config::default();
        s.update("history", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "No history yet"));

        s.history.push("yt cats");
        s.history.push("2+2");
        s.update("history", &config);
        assert!(matches!(
            s.results.as_slice(),
            [ResultItem::History(a), ResultItem::History(b)] if a == "2+2" && b == "yt cats"
        ));
        s.update("history YT", &config);
        assert!(matches!(s.results.as_slice(), [ResultItem::History(a)] if a == "yt cats"));
        s.update("history clear", &config);
        assert!(matches!(s.results.as_slice(), [ResultItem::ClearHistory]));
    }

    #[test]
    fn clipboard_history_lists_and_respects_the_switch() {
        let mut s = state();
        let mut config = Config::default();
        s.update("clip", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(_)));

        s.clipboard.push("https://example.com");
        s.clipboard.push("hello");
        s.update("clip", &config);
        assert!(matches!(
            s.results.as_slice(),
            [ResultItem::Clipboard(a), ResultItem::Clipboard(b)] if a == "hello" && b == "https://example.com"
        ));
        s.update("clip http", &config);
        assert_eq!(s.results.len(), 1);

        // Turning it off forgets what was stored.
        config.clipboard.enabled = false;
        s.update("clip", &config);
        assert_eq!(s.mode, Mode::Apps);
        assert!(s.clipboard.is_empty());
    }

    #[test]
    fn aliases_expand() {
        let mut s = state();
        let mut config = Config::default();
        config
            .aliases
            .insert("gh".into(), "https://github.com/search?q={}".into());
        s.update("gh gtk4 rust", &config);
        assert!(matches!(
            &s.results[0],
            ResultItem::Alias { name, target: AliasTarget::Url(url) }
                if name == "gh" && url == "https://github.com/search?q=gtk4%20rust"
        ));
    }

    #[test]
    fn questions_build_a_conversation() {
        let mut s = state();
        let config = Config::default();

        // First question: just the offer, and a request to warm up the CLI.
        let needs = s.update("what is rust?", &config);
        assert!(needs.claude);
        assert!(matches!(
            s.results.as_slice(),
            [ResultItem::Ask { question, model }] if question == "what is rust?" && model == "haiku"
        ));
        assert!(!s.answer_mode());

        // Once asked, the conversation owns the bar. The UI clears the text
        // after sending, so an empty query shows just the transcript.
        s.chat.push(Turn {
            question: "what is rust?".into(),
            text: "A language.".into(),
            status: TurnStatus::Thinking,
        });
        assert!(s.chat_busy());
        s.chat[0].status = TurnStatus::Done;
        assert!(!s.chat_busy());
        s.update("", &config);
        assert!(matches!(s.results.as_slice(), [ResultItem::Transcript { turns, .. }] if turns.len() == 1));
        assert!(s.answer_mode());
        assert_eq!(s.mode, Mode::Claude);

        // Anything typed now is a follow-up, question mark or not.
        let needs = s.update("and go", &config);
        assert!(needs.claude);
        assert!(matches!(
            s.results.as_slice(),
            [ResultItem::Transcript { .. }, ResultItem::Ask { question, .. }] if question == "and go"
        ));
        s.update("firefox", &config);
        assert!(s.answer_mode());

        // Leaving the conversation restores normal behaviour.
        s.chat.clear();
        s.update("firefox", &config);
        assert!(!s.answer_mode());
        s.update("ask and go", &config);
        assert!(matches!(&s.results[0], ResultItem::Ask { question, .. } if question == "and go"));

        s.chat.push(Turn {
            question: "x".into(),
            text: String::new(),
            status: TurnStatus::Done,
        });
        s.reset_session();
        assert!(s.chat.is_empty());
    }
}
