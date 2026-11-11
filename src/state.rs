//! What the launcher knows and what it would show for a given query.
//! No widgets here; `ui.rs` renders `results`.

use crate::alias::{self, AliasTarget};
use crate::app_launcher::{App, AppLauncher};
use crate::config::Config;
use crate::emoji_picker::{Emoji, EmojiPicker};
use crate::files::{self, FileEntry};
use crate::history::History;
use crate::matching::Matcher;
use crate::query::{self, Intent, ParseOptions};
use crate::search::{self, Engine};
use crate::terminal::Terminal;
use crate::usage::Usage;
use crate::window_switcher::{self, OpenWindow};
use gtk::gio;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const EMOJI_COLUMNS: usize = 8;
/// Emoji rows visible at once; the grid scrolls beyond that.
pub const EMOJI_VISIBLE_ROWS: usize = 3;
pub const EMOJI_LIMIT: usize = 96;
/// Most results kept for one query. `behavior.max_results` only decides how
/// many are visible without scrolling.
pub const RESULT_LIMIT: usize = 50;
/// The file index is rebuilt in the background when older than this.
const FILE_INDEX_MAX_AGE: Duration = Duration::from_secs(300);
const MIN_FILE_QUERY: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub enum AskStatus {
    Thinking,
    Answering,
    Done,
    Failed,
}

/// The question currently put to Claude and what has come back so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub question: String,
    pub model: String,
    pub text: String,
    pub status: AskStatus,
}

#[derive(Clone, Debug)]
pub enum ResultItem {
    /// Offer to send the question; nothing is sent until Enter.
    Ask { question: String, model: String },
    /// The answer to the question in the bar, possibly still streaming.
    Answer(Ask),
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
    Settings,
    /// A message row; activating it does nothing.
    Info(String),
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
}

pub struct LauncherState {
    pub apps: AppLauncher,
    pub usage: Usage,
    pub history: History,
    pub windows: WindowList,
    pub results: Vec<ResultItem>,
    pub emoji_mode: bool,
    pub ask: Option<Ask>,
    file_index: Option<Rc<Vec<FileEntry>>>,
    file_index_built: Option<Instant>,
    file_index_loading: bool,
    emoji_picker: Option<EmojiPicker>,
    matcher: Matcher,
}

impl LauncherState {
    pub fn new(config: &Config) -> Self {
        Self::with_stores(Usage::load(), History::load(config.behavior.history_size))
    }

    fn with_stores(usage: Usage, history: History) -> Self {
        Self {
            apps: AppLauncher::empty(),
            usage,
            history,
            windows: WindowList::NotLoaded,
            results: Vec::new(),
            emoji_mode: false,
            ask: None,
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
        self.ask = None;
    }

    /// True while the only thing shown is Claude's answer.
    pub fn answer_mode(&self) -> bool {
        matches!(self.results.as_slice(), [ResultItem::Answer(_)])
    }

    pub fn set_file_index(&mut self, entries: Vec<FileEntry>) {
        self.file_index = Some(Rc::new(entries));
        self.file_index_built = Some(Instant::now());
        self.file_index_loading = false;
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
            aliases: config.aliases.clone(),
        };
        self.history.set_capacity(config.behavior.history_size);

        self.emoji_mode = false;
        let mut needs = Needs::default();

        self.results = match query::parse(query, &options) {
            Intent::Ask(question) => match &self.ask {
                Some(ask) if ask.question == question => vec![ResultItem::Answer(ask.clone())],
                _ => vec![ResultItem::Ask {
                    question,
                    model: config.claude.model.clone(),
                }],
            },
            Intent::Empty => self
                .apps
                .most_used(&self.usage, config.behavior.max_results)
                .into_iter()
                .map(|app| ResultItem::App(app.clone()))
                .collect(),
            Intent::Settings => vec![ResultItem::Settings],
            Intent::Windows(filter) => {
                if matches!(self.windows, WindowList::NotLoaded) {
                    self.windows = WindowList::Loading;
                    needs.windows = true;
                }
                self.window_results(&filter)
            }
            Intent::Search { engine, query } => vec![ResultItem::Search { engine, query }],
            Intent::Emoji(filter) => {
                self.emoji_mode = true;
                self.emoji_picker
                    .get_or_insert_with(EmojiPicker::new)
                    .search(&filter, EMOJI_LIMIT)
                    .into_iter()
                    .map(ResultItem::Emoji)
                    .collect()
            }
            Intent::Calc { expression, result } => vec![ResultItem::Calc { expression, result }],
            Intent::Terminal(command) => vec![ResultItem::Terminal(command)],
            Intent::Files(filter) => {
                let stale = self
                    .file_index_built
                    .map(|built| built.elapsed() > FILE_INDEX_MAX_AGE)
                    .unwrap_or(true);
                if stale && !self.file_index_loading {
                    self.file_index_loading = true;
                    needs.files = true;
                }
                self.file_results(&filter)
            }
            Intent::Path(path) => {
                let entries = files::complete_path(&path, RESULT_LIMIT);
                if entries.is_empty() {
                    vec![ResultItem::Info("No such file or directory".to_string())]
                } else {
                    entries.into_iter().map(ResultItem::File).collect()
                }
            }
            Intent::History(filter) => self.history_results(&filter),
            Intent::Alias {
                name,
                template,
                args,
            } => vec![ResultItem::Alias {
                target: alias::expand(&template, &args),
                name,
            }],
            Intent::Apps(query) => self.app_results(&query, config),
        };

        needs
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
        let found = files::search(filter, index, RESULT_LIMIT);
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

    fn app_results(&self, query: &str, config: &Config) -> Vec<ResultItem> {
        let mut items: Vec<ResultItem> = self
            .apps
            .search(query, &self.usage, RESULT_LIMIT)
            .into_iter()
            .map(|app| ResultItem::App(app.clone()))
            .collect();

        // "settings", "poppi", "launcher"... also offer our own settings.
        if query.len() >= 3 && "poppi launcher settings".contains(&query.to_lowercase()) {
            items.push(ResultItem::Settings);
        }

        // Apps always come first. A command on $PATH is offered after them,
        // or leads the list when no app matched.
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
        LauncherState::with_stores(Usage::load_from(None), History::load_from(None, 50))
    }

    #[test]
    fn empty_query_without_usage_shows_nothing() {
        let mut s = state();
        assert_eq!(s.update("", &Config::default()), Needs::default());
        assert!(s.results.is_empty());
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

        s.windows = WindowList::Loaded(Vec::new());
        assert!(!s.update("sw", &config).windows);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "No open windows found"));

        s.windows = WindowList::Failed("boom".to_string());
        s.update("sw", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "boom"));
    }

    #[test]
    fn emoji_mode_is_flagged_and_limited() {
        let mut s = state();
        s.update(":", &Config::default());
        assert!(s.emoji_mode);
        assert_eq!(s.results.len(), EMOJI_LIMIT);
        s.update("2+2", &Config::default());
        assert!(!s.emoji_mode);
        assert!(matches!(&s.results[0], ResultItem::Calc { result, .. } if result == "4"));
    }

    #[test]
    fn settings_is_offered_for_partial_matches() {
        let mut s = state();
        s.update("settings", &Config::default());
        assert!(matches!(s.results.as_slice(), [ResultItem::Settings]));
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
        s.update("f zzzz", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(m) if m == "No matching files"));
        s.update("f r", &config);
        assert!(matches!(&s.results[0], ResultItem::Info(_)));
    }

    #[test]
    fn questions_are_offered_then_show_their_answer() {
        let mut s = state();
        let config = Config::default();

        s.update("what is rust?", &config);
        assert!(matches!(
            s.results.as_slice(),
            [ResultItem::Ask { question, model }] if question == "what is rust?" && model == "haiku"
        ));
        assert!(!s.answer_mode());

        s.ask = Some(Ask {
            question: "what is rust?".into(),
            model: "haiku".into(),
            text: "A language.".into(),
            status: AskStatus::Done,
        });
        s.update("what is rust?", &config);
        assert!(matches!(s.results.as_slice(), [ResultItem::Answer(a)] if a.text == "A language."));
        assert!(s.answer_mode());

        // A different question does not show the old answer.
        s.update("what is go?", &config);
        assert!(matches!(s.results.as_slice(), [ResultItem::Ask { .. }]));

        s.reset_session();
        assert!(s.ask.is_none());
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
}
