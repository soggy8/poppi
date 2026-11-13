//! What the launcher knows and what it would show for a given query.
//! No widgets here; the `ui` module renders `results`.

use crate::actions::{self, Action};
use crate::ai::{self, providers};
use crate::alias::{self, AliasTarget};
use crate::app_launcher::{App, AppLauncher};
use crate::clipboard::{self, ClipboardEntry, ClipboardHistory};
use crate::config::Config;
use crate::context;
use crate::emoji_picker::{self, Emoji, EmojiPicker};
use crate::files::{self, ContentMatch, FileEntry};
use crate::history::History;
use crate::matching::Matcher;
use crate::query::{self, Intent, ParseOptions};
use crate::reminders::{self, Reminder};
use crate::scripts::{Script, ScriptCatalog, ScriptItem, ScriptMode};
use crate::search::{self, Engine};
use crate::system::SystemAction;
use crate::terminal::Terminal;
use crate::units::{self, Rates};
use crate::usage::{self, Usage};
use crate::window_switcher::{self, OpenWindow, WindowId};
use gtk::gio;
use std::path::PathBuf;
use std::sync::Arc;
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
const MIN_GREP_QUERY: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub enum TurnStatus {
    Thinking,
    Answering,
    Done,
    Failed,
}

/// One question to a model and what has come back so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub question: String,
    pub text: String,
    pub status: TurnStatus,
    /// Name of the model asked.
    pub model: String,
    /// Its display name.
    pub label: String,
    /// Text the question was about (the selection or the clipboard),
    /// sent along with it and with every follow-up.
    pub context: Option<String>,
}

impl Turn {
    pub fn is_finished(&self) -> bool {
        matches!(self.status, TurnStatus::Done | TurnStatus::Failed)
    }
}

/// The finished turns as messages, for models that need the context sent.
pub fn chat_history(turns: &[Turn]) -> Vec<ai::Message> {
    turns
        .iter()
        .filter(|turn| turn.status == TurnStatus::Done)
        .flat_map(|turn| {
            [
                ai::Message::user(&crate::context::attach(
                    &turn.question,
                    turn.context.as_deref(),
                )),
                ai::Message::assistant(&turn.text),
            ]
        })
        .collect()
}

#[derive(Clone, Debug)]
pub enum SecondaryAction {
    LaunchApp(App),
    OpenFile { file: FileEntry, reveal: bool },
    OpenTerminalInDir(PathBuf),
    FocusWindow(OpenWindow),
    CloseWindow(OpenWindow),
    CopyText(String),
    ToggleClipboardPin(String),
    RemoveClipboardEntry(String),
}

#[derive(Clone, Debug)]
pub enum ResultItem {
    App(App),
    Calc {
        expression: String,
        result: String,
        swatch: Option<(u8, u8, u8, u8)>,
    },
    Emoji(Emoji),
    Terminal(String),
    Search {
        engine: Engine,
        query: String,
    },
    Window {
        window: OpenWindow,
        icon: Option<gio::Icon>,
    },
    File(FileEntry),
    ContentMatch(ContentMatch),
    Alias {
        name: String,
        target: AliasTarget,
    },
    /// An executable action script from `~/.config/poppi_launcher/scripts/`.
    ScriptRun {
        script: Script,
        args: String,
    },
    /// A row emitted by a `list`-mode script.
    ScriptOutput(ScriptItem),
    /// A secondary action shown in the `Ctrl+K` action menu.
    ItemAction {
        title: String,
        subtitle: String,
        icon: &'static str,
        action: SecondaryAction,
    },
    /// A past query; activating it puts it back in the search bar.
    History(String),
    ClearHistory,
    /// A text from the clipboard history (pinned or transient).
    Clipboard(ClipboardEntry),
    System(SystemAction),
    /// Second step for a system action that has no confirmation of its own.
    Confirm(SystemAction),
    /// Offer to send a question to a model (by name); nothing is sent
    /// until Enter.
    Ask {
        question: String,
        model: String,
        label: String,
    },
    /// Something the AI proposed that has no row of its own (apps, web
    /// searches, commands and system actions use theirs).
    Proposed(Action),
    /// The conversation so far.
    Transcript {
        turns: Vec<Turn>,
    },
    Settings,
    /// The first-run setup: AI service, shortcut, voice.
    Setup,
    /// A reminder to set: due at (Unix time), and what to say.
    Remind {
        at: i64,
        text: String,
    },
    /// A reminder already set; Enter cancels it.
    PendingReminder(Reminder),
    /// A line to add to the notes file.
    Note(String),
    OpenNotes,
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
    Script,
    Actions,
    History,
    Clipboard,
    Ai,
    Settings,
    Reminders,
    Notes,
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
            Mode::Script => "Script",
            Mode::Actions => "Actions",
            Mode::History => "History",
            Mode::Clipboard => "Clipboard",
            Mode::Ai => "AI",
            Mode::Settings => "Settings",
            Mode::Reminders => "Reminder",
            Mode::Notes => "Note",
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
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Needs {
    pub windows: bool,
    pub files: bool,
    pub rates: bool,
    /// Get the model in the `Ask` row ready (starts the Claude CLI) so it
    /// answers quickly once the question is sent.
    pub ai: bool,
    /// Run a `list`-mode script `(script, filter)` on a worker thread.
    pub script: Option<(Script, String)>,
    /// Search file contents `(pattern, index)` on worker threads.
    pub grep: Option<(String, Arc<Vec<FileEntry>>)>,
    /// List a directory for path completion on a worker thread.
    pub dir: Option<PathBuf>,
}

struct ActionMenuState {
    saved_query: String,
    actions: Vec<ResultItem>,
}

/// The row for "remind …": the reminder, or how to say when.
fn remind_item(rest: &str) -> ResultItem {
    if rest.trim().is_empty() {
        return ResultItem::Info(
            "Say when: \"remind 10m tea\", \"remind me at 15:30 to call Ana\"".to_string(),
        );
    }
    match reminders::parse(rest, reminders::Now::local()) {
        Some((at, text)) => ResultItem::Remind { at, text },
        None => ResultItem::Info(
            "When? Try \"remind 10m tea\" or \"remind me at 15:30 to call Ana\"".to_string(),
        ),
    }
}

fn ask_item(config: &Config, question: &str, model: &str) -> ResultItem {
    ResultItem::Ask {
        question: question.to_string(),
        model: model.to_string(),
        label: providers::label(&config.ai, model),
    }
}

impl LauncherState {
    /// The text a question refers to ("explain this"), if there is one.
    pub fn context_for(&self, question: &str) -> Option<(context::Source, String)> {
        let source = context::wanted(question)?;
        let text = match source {
            context::Source::Selection => self.selection.clone(),
            context::Source::Clipboard => self.clipboard_text.clone(),
        }?;
        Some((source, text)).filter(|(_, t)| !t.trim().is_empty())
    }

    /// An offer to ask, saying when a text will be sent along.
    fn ask_row(&self, config: &Config, question: &str, model: &str) -> ResultItem {
        let mut item = ask_item(config, question, model);
        if let (ResultItem::Ask { label, .. }, Some((source, _))) =
            (&mut item, self.context_for(question))
        {
            *label = format!("{} · {}", label, source.describe());
        }
        item
    }
}

pub struct LauncherState {
    /// Text selected in another app, and on the clipboard, when the
    /// launcher was opened; sent only with questions about them.
    pub selection: Option<String>,
    pub clipboard_text: Option<String>,
    pub reminders: reminders::Store,
    pub apps: AppLauncher,
    pub scripts: ScriptCatalog,
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
    /// The conversation; cleared when the launcher is dismissed.
    pub chat: Vec<Turn>,
    action_menu: Option<ActionMenuState>,
    script_cache: Option<(String, String, Result<Vec<ScriptItem>, String>)>,
    script_loading: Option<(String, String)>,
    grep_cache: Option<(String, Vec<ContentMatch>)>,
    grep_loading: Option<String>,
    /// The directory listing path completion filters, and the one on its way.
    dir_cache: Option<(PathBuf, Vec<FileEntry>)>,
    dir_loading: Option<PathBuf>,
    rates: Option<Rates>,
    rates_loading: bool,
    rates_checked: bool,
    file_index: Option<Arc<Vec<FileEntry>>>,
    file_index_built: Option<Instant>,
    file_index_loading: bool,
    emoji_picker: Option<(String, EmojiPicker)>,
    matcher: Matcher,
}

impl LauncherState {
    pub fn new(config: &Config) -> Self {
        let mut s = Self::with_stores(
            Usage::load(),
            History::load(config.behavior.history_size),
            History::load_named("emoji_recent.json", EMOJI_RECENT),
            ClipboardHistory::load(config.clipboard.size),
            config,
        );
        s.scripts = ScriptCatalog::load();
        s.reminders = reminders::Store::load();
        s
    }

    fn with_stores(
        usage: Usage,
        history: History,
        emoji_recent: History,
        clipboard: ClipboardHistory,
        _config: &Config,
    ) -> Self {
        Self {
            selection: None,
            clipboard_text: None,
            reminders: reminders::Store::default(),
            apps: AppLauncher::empty(),
            scripts: ScriptCatalog::default(),
            usage,
            history,
            emoji_recent,
            clipboard,
            windows: WindowList::NotLoaded,
            results: Vec::new(),
            mode: Mode::Empty,
            highlight: String::new(),
            emoji_mode: false,
            chat: Vec::new(),
            action_menu: None,
            script_cache: None,
            script_loading: None,
            grep_cache: None,
            grep_loading: None,
            dir_cache: None,
            dir_loading: None,
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
        self.action_menu = None;
        self.script_cache = None;
        self.script_loading = None;
        self.grep_cache = None;
        self.grep_loading = None;
        self.dir_cache = None;
        self.dir_loading = None;
    }

    /// True while the conversation is what is shown.
    pub fn answer_mode(&self) -> bool {
        matches!(self.results.first(), Some(ResultItem::Transcript { .. }))
    }

    /// True while an answer is still on its way.
    pub fn chat_busy(&self) -> bool {
        self.chat.last().is_some_and(|turn| !turn.is_finished())
    }

    /// The model a follow-up goes to: the one that answered last.
    pub fn chat_model(&self, config: &Config) -> String {
        self.chat
            .last()
            .map(|turn| turn.model.clone())
            .filter(|model| config.ai.models.contains_key(model))
            .unwrap_or_else(|| config.ai.default.clone())
    }

    pub fn matcher(&self) -> &Matcher {
        &self.matcher
    }

    pub fn set_file_index(&mut self, entries: Vec<FileEntry>) {
        self.file_index = Some(Arc::new(entries));
        self.file_index_built = Some(Instant::now());
        self.file_index_loading = false;
        self.grep_cache = None;
    }

    /// Mark the file index due for a background refresh while keeping the
    /// current entries available to answer immediately.
    pub fn invalidate_file_index(&mut self) {
        self.file_index_built = None;
    }

    /// `None` when the download failed; the cached rates, if any, stay.
    pub fn set_rates(&mut self, rates: Option<Rates>) {
        if rates.is_some() {
            self.rates = rates;
        }
        self.rates_loading = false;
    }

    pub fn set_script_results(
        &mut self,
        keyword: String,
        filter: String,
        outcome: Result<Vec<ScriptItem>, String>,
    ) {
        if self.script_loading.as_ref() == Some(&(keyword.clone(), filter.clone())) {
            self.script_loading = None;
        }
        self.script_cache = Some((keyword, filter, outcome));
    }

    pub fn set_grep_results(&mut self, pattern: String, matches: Vec<ContentMatch>) {
        if self.grep_loading.as_deref() == Some(pattern.as_str()) {
            self.grep_loading = None;
        }
        self.grep_cache = Some((pattern, matches));
    }

    pub fn set_dir_listing(&mut self, dir: PathBuf, listing: Vec<FileEntry>) {
        if self.dir_loading.as_ref() == Some(&dir) {
            self.dir_loading = None;
        }
        self.dir_cache = Some((dir, listing));
    }

    pub fn remove_window(&mut self, id: &WindowId) {
        if let WindowList::Loaded(windows) = &mut self.windows {
            windows.retain(|w| &w.id != id);
        }
    }

    pub fn in_action_menu(&self) -> bool {
        self.action_menu.is_some()
    }

    #[cfg(test)]
    pub fn has_actions(item: &ResultItem) -> bool {
        matches!(
            item,
            ResultItem::App(_)
                | ResultItem::File(_)
                | ResultItem::Window { .. }
                | ResultItem::Clipboard(_)
                | ResultItem::Calc { .. }
        )
    }

    /// Open the secondary action menu for `item`, remembering `current_query`
    /// so Escape returns right back to it.
    pub fn open_action_menu(&mut self, item: &ResultItem, current_query: &str) -> bool {
        let act = |title: &str, subtitle: String, icon: &'static str, action: SecondaryAction| {
            ResultItem::ItemAction {
                title: title.to_string(),
                subtitle,
                icon,
                action,
            }
        };

        let actions: Vec<ResultItem> = match item {
            ResultItem::App(app) => {
                let mut list = vec![act(
                    "Launch",
                    app.name.clone(),
                    "media-playback-start-symbolic",
                    SecondaryAction::LaunchApp(app.clone()),
                )];
                for extra in self.apps.actions_for(app) {
                    let extra_name = extra.name.clone();
                    list.push(act(
                        &extra_name,
                        app.name.clone(),
                        "window-new-symbolic",
                        SecondaryAction::LaunchApp(extra),
                    ));
                }
                list.push(act(
                    "Copy Desktop ID",
                    app.id.clone(),
                    "edit-copy-symbolic",
                    SecondaryAction::CopyText(app.id.clone()),
                ));
                list
            }
            ResultItem::File(file) => {
                let folder = if file.is_dir {
                    file.path.clone()
                } else {
                    file.path
                        .parent()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| file.path.clone())
                };
                let folder_display = files::tilde_path(&folder);
                let full_path = file.path.display().to_string();
                let mut list = vec![act(
                    "Open",
                    file.name.clone(),
                    "document-open-symbolic",
                    SecondaryAction::OpenFile {
                        file: file.clone(),
                        reveal: false,
                    },
                )];
                if !file.is_dir {
                    list.push(act(
                        "Open Containing Folder",
                        folder_display.clone(),
                        "folder-open-symbolic",
                        SecondaryAction::OpenFile {
                            file: file.clone(),
                            reveal: true,
                        },
                    ));
                }
                list.push(act(
                    "Open Terminal Here",
                    folder_display,
                    "utilities-terminal-symbolic",
                    SecondaryAction::OpenTerminalInDir(folder),
                ));
                list.push(act(
                    "Copy Path",
                    full_path.clone(),
                    "edit-copy-symbolic",
                    SecondaryAction::CopyText(full_path),
                ));
                list
            }
            ResultItem::ContentMatch(m) => {
                let folder = m
                    .file
                    .path
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| m.file.path.clone());
                let folder_display = files::tilde_path(&folder);
                let loc = format!("{}:{}", m.file.path.display(), m.line_number);
                vec![
                    act(
                        "Open File",
                        format!("{}:{}", m.file.name, m.line_number),
                        "document-open-symbolic",
                        SecondaryAction::OpenFile {
                            file: m.file.clone(),
                            reveal: false,
                        },
                    ),
                    act(
                        "Open Containing Folder",
                        folder_display.clone(),
                        "folder-open-symbolic",
                        SecondaryAction::OpenFile {
                            file: m.file.clone(),
                            reveal: true,
                        },
                    ),
                    act(
                        "Open Terminal Here",
                        folder_display,
                        "utilities-terminal-symbolic",
                        SecondaryAction::OpenTerminalInDir(folder),
                    ),
                    act(
                        "Copy Matching Line",
                        m.line_preview.clone(),
                        "edit-copy-symbolic",
                        SecondaryAction::CopyText(m.line_preview.clone()),
                    ),
                    act(
                        "Copy Path and Line",
                        loc.clone(),
                        "edit-copy-symbolic",
                        SecondaryAction::CopyText(loc),
                    ),
                ]
            }
            ResultItem::Window { window, .. } => vec![
                act(
                    "Switch to Window",
                    window.title.clone(),
                    "focus-windows-symbolic",
                    SecondaryAction::FocusWindow(window.clone()),
                ),
                act(
                    "Close Window",
                    format!("{} · {}", window.app_name, window.title),
                    "window-close-symbolic",
                    SecondaryAction::CloseWindow(window.clone()),
                ),
            ],
            ResultItem::Clipboard(entry) => {
                let preview = clipboard::preview(&entry.text);
                let pin_label = if entry.pinned {
                    "Unpin Snippet"
                } else {
                    "Pin Snippet"
                };
                vec![
                    act(
                        "Copy",
                        preview.clone(),
                        "edit-paste-symbolic",
                        SecondaryAction::CopyText(entry.text.clone()),
                    ),
                    act(
                        pin_label,
                        preview.clone(),
                        "view-pin-symbolic",
                        SecondaryAction::ToggleClipboardPin(entry.text.clone()),
                    ),
                    act(
                        "Remove from Clipboard History",
                        preview,
                        "user-trash-symbolic",
                        SecondaryAction::RemoveClipboardEntry(entry.text.clone()),
                    ),
                ]
            }
            ResultItem::Calc {
                expression, result, ..
            } => {
                let full = format!("{} = {}", expression, result);
                vec![
                    act(
                        "Copy Result",
                        result.clone(),
                        "edit-copy-symbolic",
                        SecondaryAction::CopyText(result.clone()),
                    ),
                    act(
                        "Copy Expression and Result",
                        full.clone(),
                        "edit-copy-symbolic",
                        SecondaryAction::CopyText(full),
                    ),
                ]
            }
            _ => return false,
        };

        self.action_menu = Some(ActionMenuState {
            saved_query: current_query.to_string(),
            actions,
        });
        true
    }

    /// Leave the action menu and return the query that was in the bar before.
    pub fn close_action_menu(&mut self) -> Option<String> {
        self.action_menu.take().map(|menu| menu.saved_query)
    }

    /// Recompute `results` for `query` and report which background loads the
    /// caller must start (calling `update` again once they finish).
    pub fn update(&mut self, query: &str, config: &Config) -> Needs {
        self.history.set_capacity(config.behavior.history_size);
        self.clipboard.set_capacity(if config.clipboard.enabled {
            config.clipboard.size
        } else {
            0
        });

        self.emoji_mode = false;
        self.highlight.clear();
        let mut needs = Needs::default();

        if let Some(menu) = &self.action_menu {
            let filter = query.trim().to_lowercase();
            let items: Vec<ResultItem> = if filter.is_empty() {
                menu.actions.clone()
            } else {
                menu.actions
                    .iter()
                    .filter(|item| match item {
                        ResultItem::ItemAction {
                            title, subtitle, ..
                        } => {
                            self.matcher.score(&filter, &title.to_lowercase()).is_some()
                                || subtitle.to_lowercase().contains(&filter)
                        }
                        _ => false,
                    })
                    .cloned()
                    .collect()
            };
            self.highlight = query.trim().to_string();
            self.mode = Mode::Actions;
            self.results = if items.is_empty() {
                vec![ResultItem::Info("No matching actions".to_string())]
            } else {
                items
            };
            return needs;
        }

        let options = ParseOptions {
            youtube: config.search.youtube_enabled,
            chatgpt: config.search.chatgpt_enabled,
            calculator: config.calculator.enabled,
            files: config.files.enabled,
            ai: config.ai.enabled,
            clipboard: config.clipboard.enabled,
            aliases: config.aliases.clone(),
            scripts: self.scripts.keywords(),
            models: config.ai.models.keys().cloned().collect(),
        };

        // A conversation takes over the bar: whatever is typed next is a
        // follow-up, with or without a question mark. It goes to the model
        // that answered last, unless it starts with another model's name.
        if !self.chat.is_empty() && config.ai.enabled {
            let mut items = vec![ResultItem::Transcript {
                turns: self.chat.clone(),
            }];
            let question = query.trim();
            if !question.is_empty() {
                let (model, question) = match query::model_prefix(question, &options.models) {
                    Some((model, rest)) => (model.to_string(), rest),
                    None => (self.chat_model(config), question),
                };
                items.push(self.ask_row(config, question, &model));
            } else if let Some(last) = self.chat.last().filter(|t| t.status == TurnStatus::Done) {
                // What the latest answer offers to do, ready for Enter.
                let proposed = actions::split(&last.text).1;
                items.extend(proposed.into_iter().map(|a| self.action_item(a, config)));
            }
            self.mode = Mode::Ai;
            self.results = items;
            needs.ai = true;
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
            Intent::Ask { question, model } => {
                needs.ai = true;
                let model = model.unwrap_or_else(|| config.ai.default.clone());
                (Mode::Ai, vec![self.ask_row(config, &question, &model)])
            }
            Intent::Settings => (Mode::Settings, vec![ResultItem::Settings]),
            Intent::Setup => (Mode::Settings, vec![ResultItem::Setup]),
            Intent::Remind(rest) => (Mode::Reminders, vec![remind_item(&rest)]),
            Intent::Reminders => {
                let items: Vec<ResultItem> = self
                    .reminders
                    .items
                    .iter()
                    .cloned()
                    .map(ResultItem::PendingReminder)
                    .collect();
                let items = if items.is_empty() {
                    vec![ResultItem::Info(
                        "No reminders. Set one with \"remind 10m tea\"".to_string(),
                    )]
                } else {
                    items
                };
                (Mode::Reminders, items)
            }
            Intent::Note(text) if text.trim().is_empty() => {
                (Mode::Notes, vec![ResultItem::OpenNotes])
            }
            Intent::Note(text) => (Mode::Notes, vec![ResultItem::Note(text.trim().to_string())]),
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
                    self.emoji_picker = Some((
                        tone.clone(),
                        EmojiPicker::new(emoji_picker::skin_tone(&tone)),
                    ));
                }
                let items = self
                    .emoji_picker
                    .as_ref()
                    .map(|(_, picker)| {
                        picker.search(&filter, self.emoji_recent.entries(), EMOJI_LIMIT)
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .map(ResultItem::Emoji)
                    .collect();
                (Mode::Emoji, items)
            }
            Intent::Calc {
                expression,
                result,
                swatch,
            } => (
                Mode::Calculator,
                vec![ResultItem::Calc {
                    expression,
                    result,
                    swatch,
                }],
            ),
            Intent::Convert(request) => {
                let expression = query.trim().to_string();
                if let Some(result) = units::convert(&request) {
                    (
                        Mode::Calculator,
                        vec![ResultItem::Calc {
                            expression,
                            result,
                            swatch: None,
                        }],
                    )
                } else if config.calculator.currency && units::looks_like_currency(&request) {
                    (
                        Mode::Calculator,
                        self.currency_results(&request, expression, &mut needs),
                    )
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
            Intent::Grep(pattern) => {
                let stale = self
                    .file_index_built
                    .map(|built| built.elapsed() > FILE_INDEX_MAX_AGE)
                    .unwrap_or(true);
                if stale && !self.file_index_loading {
                    self.file_index_loading = true;
                    needs.files = true;
                }
                let items = self.grep_results(&pattern, &mut needs);
                self.highlight = pattern;
                (Mode::Files, items)
            }
            Intent::Path(path) => (Mode::Files, self.path_results(&path, &mut needs)),
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
                    self.highlight = filter;
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
            Intent::Script { keyword, args } => {
                let items = self.script_results(&keyword, &args, &mut needs);
                (Mode::Script, items)
            }
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
                    swatch: None,
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

    fn script_results(&mut self, keyword: &str, args: &str, needs: &mut Needs) -> Vec<ResultItem> {
        let Some(script) = self.scripts.by_keyword(keyword).cloned() else {
            return vec![ResultItem::Info("Script not found".to_string())];
        };
        if script.mode == ScriptMode::Action {
            return vec![ResultItem::ScriptRun {
                script,
                args: args.to_string(),
            }];
        }

        let key = (keyword.to_string(), args.to_string());
        if let Some((cached_kw, cached_args, outcome)) = &self.script_cache {
            if cached_kw == keyword && cached_args == args {
                return match outcome {
                    Ok(items) if items.is_empty() => {
                        vec![ResultItem::Info("No matching script results".to_string())]
                    }
                    Ok(items) => items
                        .iter()
                        .cloned()
                        .map(ResultItem::ScriptOutput)
                        .collect(),
                    Err(error) => vec![ResultItem::Info(error.clone())],
                };
            }
        }
        if self.script_loading.as_ref() != Some(&key) {
            self.script_loading = Some(key);
            needs.script = Some((script.clone(), args.to_string()));
        }
        // Keep showing the most recent rows from the same script while the new filter runs.
        if let Some((cached_kw, _, Ok(items))) = &self.script_cache {
            if cached_kw == keyword && !items.is_empty() {
                return items
                    .iter()
                    .cloned()
                    .map(ResultItem::ScriptOutput)
                    .collect();
            }
        }
        vec![ResultItem::Info(format!("Running {}…", script.name))]
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

    fn grep_results(&mut self, pattern: &str, needs: &mut Needs) -> Vec<ResultItem> {
        if pattern.chars().count() < MIN_GREP_QUERY {
            return vec![ResultItem::Info(format!(
                "Type at least {} characters to search file contents",
                MIN_GREP_QUERY
            ))];
        }
        let Some(index) = &self.file_index else {
            return vec![ResultItem::Info("Indexing files…".to_string())];
        };
        if let Some((cached_pat, matches)) = &self.grep_cache {
            if cached_pat == pattern {
                return if matches.is_empty() {
                    vec![ResultItem::Info("No matching text in files".to_string())]
                } else {
                    matches
                        .iter()
                        .cloned()
                        .map(ResultItem::ContentMatch)
                        .collect()
                };
            }
        }
        if self.grep_loading.as_deref() != Some(pattern) {
            self.grep_loading = Some(pattern.to_string());
            needs.grep = Some((pattern.to_string(), index.clone()));
        }
        vec![ResultItem::Info("Searching file contents…".to_string())]
    }

    /// Completions for a typed path. The directory is read on a worker thread
    /// (it can hang on network mounts); typing within it filters the cache.
    fn path_results(&mut self, input: &str, needs: &mut Needs) -> Vec<ResultItem> {
        let (dir, partial) = files::split_path_input(input);
        if let Some((cached, listing)) = &self.dir_cache {
            if *cached == dir {
                let entries = files::filter_listing(listing, &partial, RESULT_LIMIT);
                return if entries.is_empty() {
                    vec![ResultItem::Info("No such file or directory".to_string())]
                } else {
                    entries.into_iter().map(ResultItem::File).collect()
                };
            }
        }
        if self.dir_loading.as_ref() != Some(&dir) {
            self.dir_loading = Some(dir.clone());
            needs.dir = Some(dir);
        }
        vec![ResultItem::Info("Reading folder…".to_string())]
    }

    /// The row for a proposed action: the existing kind of row where there
    /// is one, so it looks and runs like everything else.
    fn action_item(&self, action: Action, config: &Config) -> ResultItem {
        match action {
            Action::OpenApp(name) => match self.apps.search(&name, &self.usage, 1).first() {
                Some(app) => ResultItem::App((*app).clone()),
                None => ResultItem::Proposed(Action::OpenApp(name)),
            },
            Action::WebSearch(query) => ResultItem::Search {
                engine: Engine::from_id(&config.search.default_engine).unwrap_or(Engine::Google),
                query,
            },
            Action::Run(command) => ResultItem::Terminal(command),
            Action::System(action) => ResultItem::System(action),
            Action::Remind(rest) => match remind_item(&rest) {
                ResultItem::Info(_) => ResultItem::Proposed(Action::Remind(rest)),
                item => item,
            },
            Action::Note(text) => ResultItem::Note(text),
            other => ResultItem::Proposed(other),
        }
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
        found.sort_by_key(|item| std::cmp::Reverse(item.1));
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

        // User scripts whose name or description match appear alongside apps.
        for script in self.scripts.search(&self.matcher, query) {
            items.push(ResultItem::ScriptRun {
                script: script.clone(),
                args: String::new(),
            });
        }

        // "settings", "poppi", "launcher"... also offer our own settings.
        if query.len() >= 3 && "poppi launcher settings".contains(&query.to_lowercase()) {
            items.push(ResultItem::Settings);
        }
        if query.len() >= 3 && "poppi setup".contains(&query.to_lowercase()) {
            items.push(ResultItem::Setup);
        }

        // Apps always come first. A command on $PATH is offered after them,
        // or leads the list when nothing else matched.
        let had_matches = !items.is_empty();
        let is_command = Terminal::looks_like_command(query);
        if is_command {
            items.push(ResultItem::Terminal(query.to_string()));
        }
        // A few words that match nothing read like a request: offer to let
        // the AI handle it ("open my downloads folder").
        if !had_matches && !is_command && config.ai.enabled && query.split_whitespace().count() >= 3
        {
            items.push(self.ask_row(config, query.trim(), &config.ai.default));
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
            ClipboardHistory::new(30),
            &Config::default(),
        )
    }

    #[test]
    fn typed_paths_list_the_folder_once_then_filter_the_cache() {
        let dir = std::env::temp_dir().join(format!("poppi-state-path-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("deploy.sh"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        let root = dir.to_string_lossy().into_owned();
        let config = Config::default();
        let mut s = state();

        // Nothing is read on the UI thread: the folder is requested instead.
        let needs = s.update(&format!("{}/d", root), &config);
        assert_eq!(needs.dir.as_deref(), Some(dir.as_path()));
        assert!(matches!(&s.results[..], [ResultItem::Info(_)]));
        // Typing on before it arrives doesn't ask again.
        assert_eq!(s.update(&format!("{}/de", root), &config).dir, None);

        s.set_dir_listing(dir.clone(), files::list_dir(&dir));
        assert_eq!(s.update(&format!("{}/d", root), &config).dir, None);
        let names: Vec<&str> = s
            .results
            .iter()
            .filter_map(|item| match item {
                ResultItem::File(f) => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["docs", "deploy.sh"]);

        // Another folder is a new request.
        let sub = dir.join("docs");
        assert_eq!(s.update(&format!("{}/docs/", root), &config).dir, Some(sub));
        std::fs::remove_dir_all(&dir).unwrap();
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
                ResultItem::Search {
                    engine: Engine::Google,
                    ..
                },
                ResultItem::Search {
                    engine: Engine::Youtube,
                    ..
                },
                ResultItem::Search {
                    engine: Engine::Chatgpt,
                    ..
                },
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
        s.update("setup", &Config::default());
        assert!(matches!(s.results.as_slice(), [ResultItem::Setup]));
    }

    #[test]
    fn system_actions_match_by_prefix_and_lead_when_exact() {
        let mut s = state();
        let config = Config::default();

        s.update("lock", &config);
        assert!(matches!(
            s.results[0],
            ResultItem::System(SystemAction::Lock)
        ));
        s.update("reb", &config);
        assert!(s
            .results
            .iter()
            .any(|r| matches!(r, ResultItem::System(SystemAction::Restart))));
        s.update("shutdown", &config);
        assert!(matches!(
            s.results[0],
            ResultItem::System(SystemAction::PowerOff)
        ));

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
            ResultItem::Calc { result, expression, .. } if result == "80.00 EUR" && expression.contains("2026-11-10")
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
            [ResultItem::Clipboard(a), ResultItem::Clipboard(b)] if a.text == "hello" && b.text == "https://example.com"
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
    fn proposed_actions_become_rows_under_the_answer() {
        let mut s = state();
        let config = Config::default();
        s.chat.push(Turn {
            question: "lock up and show my downloads".into(),
            text: "Done.\n[[open: ~/Downloads]]\n[[web_search: rust]]\n[[run: df -h]]\n\
                   [[system: lock]]\n[[open_app: Nonexistent Thing]]"
                .into(),
            status: TurnStatus::Done,
            model: "groq".into(),
            label: "Groq".into(),
            context: None,
        });
        s.update("", &config);
        assert!(matches!(
            s.results.as_slice(),
            [
                ResultItem::Transcript { .. },
                ResultItem::Proposed(Action::Open(path)),
                ResultItem::Search { query, .. },
                ResultItem::Terminal(command),
                ResultItem::System(SystemAction::Lock),
                ResultItem::Proposed(Action::OpenApp(_)),
            ] if path == "~/Downloads" && query == "rust" && command == "df -h"
        ));
        // Typing a follow-up replaces them with the question.
        s.update("and my documents", &config);
        assert!(matches!(
            s.results.as_slice(),
            [ResultItem::Transcript { .. }, ResultItem::Ask { .. }]
        ));
    }

    #[test]
    fn questions_about_this_say_the_selection_goes_along() {
        let mut s = state();
        let config = Config::default();
        s.update("explain this?", &config);
        assert!(
            matches!(&s.results[0], ResultItem::Ask { label, .. } if !label.contains("selected"))
        );

        s.selection = Some("fn main() {}".into());
        s.update("explain this?", &config);
        assert!(
            matches!(&s.results[0], ResultItem::Ask { label, .. } if label.ends_with("with your selected text"))
        );
        assert_eq!(s.context_for("explain this?").unwrap().1, "fn main() {}");
        assert!(s.context_for("capital of France?").is_none());
    }

    #[test]
    fn requests_that_match_nothing_can_go_to_the_ai() {
        let mut s = state();
        let config = Config::default();
        s.update("open my downloads folder", &config);
        assert!(matches!(
            s.results.first(),
            Some(ResultItem::Ask { question, model, .. })
                if question == "open my downloads folder" && model == "groq"
        ));
        // A word or two still searches the web first.
        s.update("zzqqy blorp", &config);
        assert!(!s
            .results
            .iter()
            .any(|r| matches!(r, ResultItem::Ask { .. })));
    }

    #[test]
    fn questions_build_a_conversation() {
        let mut s = state();
        let mut config = Config::default();
        config.ai.default = "haiku".to_string();

        // First question: just the offer, and a request to warm up the CLI.
        let needs = s.update("what is rust?", &config);
        assert!(needs.ai);
        assert!(matches!(
            s.results.as_slice(),
            [ResultItem::Ask { question, model, label }]
                if question == "what is rust?" && model == "haiku" && label == "Claude Haiku"
        ));
        assert!(!s.answer_mode());

        // Once asked, the conversation owns the bar. The UI clears the text
        // after sending, so an empty query shows just the transcript.
        s.chat.push(Turn {
            question: "what is rust?".into(),
            text: "A language.".into(),
            status: TurnStatus::Thinking,
            model: "sonnet".into(),
            label: "Claude Sonnet".into(),
            context: None,
        });
        assert!(s.chat_busy());
        s.chat[0].status = TurnStatus::Done;
        assert!(!s.chat_busy());
        s.update("", &config);
        assert!(
            matches!(s.results.as_slice(), [ResultItem::Transcript { turns, .. }] if turns.len() == 1)
        );
        assert!(s.answer_mode());
        assert_eq!(s.mode, Mode::Ai);

        // Anything typed now is a follow-up, question mark or not, for the
        // model that answered last...
        let needs = s.update("and go", &config);
        assert!(needs.ai);
        assert!(matches!(
            s.results.as_slice(),
            [ResultItem::Transcript { .. }, ResultItem::Ask { question, model, .. }]
                if question == "and go" && model == "sonnet"
        ));
        // ...unless it names another one.
        s.update("opus and go", &config);
        assert!(matches!(
            &s.results[1],
            ResultItem::Ask { question, model, .. } if question == "and go" && model == "opus"
        ));
        assert_eq!(
            chat_history(&s.chat),
            [
                ai::Message::user("what is rust?"),
                ai::Message::assistant("A language.")
            ]
        );
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
            status: TurnStatus::Failed,
            model: "haiku".into(),
            label: "Claude Haiku".into(),
            context: None,
        });
        assert!(chat_history(&s.chat).is_empty());
        s.reset_session();
        assert!(s.chat.is_empty());
    }

    #[test]
    fn action_menu_opens_filters_and_restores_query() {
        let mut s = state();
        let config = Config::default();
        s.update("#4a9eff", &config);
        let calc = s.results[0].clone();
        assert!(LauncherState::has_actions(&calc));

        assert!(s.open_action_menu(&calc, "#4a9eff"));
        assert!(s.in_action_menu());
        s.update("", &config);
        assert_eq!(s.mode, Mode::Actions);
        assert_eq!(s.results.len(), 2);

        s.update("expression", &config);
        assert_eq!(s.results.len(), 1);
        assert!(matches!(
            &s.results[0],
            ResultItem::ItemAction { title, .. } if title == "Copy Expression and Result"
        ));

        assert_eq!(s.close_action_menu().as_deref(), Some("#4a9eff"));
        assert!(!s.in_action_menu());
    }
}
