//! The launcher window: widgets, keyboard handling and running results.

use crate::alias;
use crate::app_launcher::AppLauncher;
use crate::claude;
use crate::config::Config;
use crate::emoji_picker::EmojiPicker;
use crate::files::{self, FileEntry};
use crate::rows;
use crate::search;
use crate::settings::SettingsWindow;
use crate::state::{
    Ask, AskStatus, LauncherState, ResultItem, WindowList, EMOJI_COLUMNS, EMOJI_VISIBLE_ROWS,
};
use crate::terminal::Terminal;
use crate::utils;
use crate::window_switcher;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use gtk::{
    Application, Box as GtkBox, Button, Entry, EventControllerKey, Grid, ListBox, ScrolledWindow,
    Window,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const ENTRY_HEIGHT: i32 = 60;
/// The window starts taller than its content (the rest is transparent) so the
/// compositor centres it with room for results below the search bar.
const INITIAL_HEIGHT: i32 = 500;
const EMOJI_SPACING: i32 = 6;
const EMOJI_MARGIN: i32 = 10;
/// Time for focus to return to the previous window before typing an emoji.
const FOCUS_RETURN_DELAY: Duration = Duration::from_millis(200);
/// Tallest an answer from Claude gets before it scrolls.
const ANSWER_MAX_HEIGHT: i32 = 380;
const ANSWER_POLL_INTERVAL: Duration = Duration::from_millis(40);
const ANSWER_SCROLL_STEP: f64 = 48.0;

thread_local! {
    static LAUNCHER: RefCell<Option<Rc<Launcher>>> = const { RefCell::new(None) };
}

struct Launcher {
    app: Application,
    window: Window,
    entry: Entry,
    list_box: ListBox,
    emoji_grid: Grid,
    results_container: GtkBox,
    scrolled: ScrolledWindow,
    config: Rc<RefCell<Config>>,
    state: RefCell<LauncherState>,
    emoji_buttons: RefCell<Vec<Button>>,
    emoji_selected: Cell<usize>,
    /// Position while stepping through history with Alt+Up/Down.
    history_position: Cell<Option<usize>>,
    /// Set while we change the entry text ourselves during history stepping.
    recalling: Cell<bool>,
    dismissing: Cell<bool>,
    /// The Claude request in flight, if any.
    claude_request: RefCell<Option<claude::Request>>,
    /// Bumped per request so a stale poll timer stops itself.
    claude_generation: Cell<u64>,
}

/// Entry point for every invocation: the first one builds the window, later
/// ones (forwarded from a second process by GApplication) toggle it.
pub fn activate(app: &Application, config: &Rc<RefCell<Config>>, config_error: Option<&str>) {
    let existing = LAUNCHER.with(|slot| slot.borrow().clone());
    match existing {
        Some(launcher) if launcher.window.is_visible() => launcher.dismiss(),
        Some(launcher) => launcher.show(),
        None => {
            let launcher = Launcher::build(app, config.clone(), config_error);
            LAUNCHER.with(|slot| *slot.borrow_mut() = Some(launcher.clone()));
            launcher.show();
        }
    }
}

impl Launcher {
    fn build(app: &Application, config: Rc<RefCell<Config>>, config_error: Option<&str>) -> Rc<Self> {
        let width = config.borrow().theme.width;

        let window = Window::builder()
            .application(app)
            .title("Poppi Launcher")
            .default_width(width)
            .default_height(INITIAL_HEIGHT)
            .decorated(false)
            .resizable(false)
            .build();
        window.add_css_class("poppi-launcher");

        let main_box = GtkBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Start)
            .build();
        main_box.add_css_class("launcher-box");
        // Clip children to the rounded corners.
        main_box.set_overflow(gtk::Overflow::Hidden);

        let placeholder = match config_error {
            Some(_) => "Config file has errors, using defaults (details on stderr)",
            None => "Search apps, files, emoji, calculate, or ask something?",
        };
        let entry = Entry::builder()
            .placeholder_text(placeholder)
            .primary_icon_name("system-search-symbolic")
            .build();
        entry.set_icon_activatable(gtk::EntryIconPosition::Primary, false);

        let list_box = ListBox::new();
        list_box.set_selection_mode(gtk::SelectionMode::Single);

        let emoji_grid = Grid::new();
        emoji_grid.set_row_spacing(EMOJI_SPACING as u32);
        emoji_grid.set_column_spacing(EMOJI_SPACING as u32);
        emoji_grid.set_column_homogeneous(true);
        emoji_grid.set_margin_start(EMOJI_MARGIN);
        emoji_grid.set_margin_end(EMOJI_MARGIN);
        emoji_grid.set_margin_top(EMOJI_MARGIN);
        emoji_grid.set_margin_bottom(EMOJI_MARGIN);
        emoji_grid.set_visible(false);

        let results_container = GtkBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        results_container.append(&list_box);
        results_container.append(&emoji_grid);

        // Grows with its content up to a maximum set in `render`, then scrolls.
        let scrolled = ScrolledWindow::builder()
            .child(&results_container)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .build();
        scrolled.set_visible(false);

        main_box.append(&entry);
        main_box.append(&scrolled);
        window.set_child(Some(&main_box));

        let state = LauncherState::new(&config.borrow());
        let launcher = Rc::new(Self {
            app: app.clone(),
            window,
            entry,
            list_box,
            emoji_grid,
            results_container,
            scrolled,
            config,
            state: RefCell::new(state),
            emoji_buttons: RefCell::new(Vec::new()),
            emoji_selected: Cell::new(0),
            history_position: Cell::new(None),
            recalling: Cell::new(false),
            dismissing: Cell::new(false),
            claude_request: RefCell::new(None),
            claude_generation: Cell::new(0),
        });
        launcher.connect_signals();
        launcher
    }

    fn connect_signals(self: &Rc<Self>) {
        let this = self.clone();
        self.entry.connect_changed(move |_| {
            if !this.recalling.get() {
                this.history_position.set(None);
            }
            this.refresh();
        });

        let this = self.clone();
        self.entry.connect_activate(move |_| {
            if let Some(index) = this.selected_index() {
                this.activate_index(index, false);
            }
        });

        // Capture phase, so navigation keys are seen before the entry uses them.
        let keys = EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = self.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| this.on_key(key, modifiers));
        self.entry.add_controller(keys);

        let this = self.clone();
        self.list_box.connect_row_activated(move |_, row| {
            if let Ok(index) = usize::try_from(row.index()) {
                this.activate_index(index, false);
            }
        });

        let this = self.clone();
        self.window.connect_is_active_notify(move |window| {
            let enabled = this.config.borrow().behavior.close_on_focus_loss;
            if enabled && !window.is_active() && window.is_visible() {
                this.dismiss();
            }
        });

        // Alt+F4 and friends go through the same path as Escape.
        let this = self.clone();
        self.window.connect_close_request(move |_| {
            if this.config.borrow().behavior.stay_resident {
                this.dismiss();
                glib::Propagation::Stop
            } else {
                LAUNCHER.with(|slot| slot.borrow_mut().take());
                glib::Propagation::Proceed
            }
        });
    }

    // ---------- showing and hiding ----------

    fn show(self: &Rc<Self>) {
        let width = self.config.borrow().theme.width;

        self.cancel_ask();
        self.state.borrow_mut().reset_session();
        self.history_position.set(None);
        if self.entry.text().is_empty() {
            self.refresh();
        } else {
            // Emits `changed`, which refreshes.
            self.entry.set_text("");
        }

        self.window.set_default_size(width, INITIAL_HEIGHT);
        self.window.set_size_request(width, -1);
        self.window.set_opacity(0.0);
        self.window.present();
        self.entry.grab_focus();
        self.fade_in();

        // Read the application list after the first frame. GIO keeps it
        // cached and up to date, so this is cheap on later shows.
        let this = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(30), move || {
            this.state.borrow_mut().apps = AppLauncher::load();
            if this.window.is_visible() {
                this.refresh();
            }
        });
    }

    fn fade_in(&self) {
        let window = self.window.clone();
        let mut opacity = 0.0;
        glib::timeout_add_local(Duration::from_millis(10), move || {
            opacity += 0.05;
            if opacity >= 1.0 || !window.is_visible() {
                window.set_opacity(1.0);
                return glib::ControlFlow::Break;
            }
            window.set_opacity(opacity);
            glib::ControlFlow::Continue
        });
    }

    /// Hide the launcher. In resident mode the process and window survive for
    /// the next invocation; otherwise the window closes and, once no other
    /// window (settings) is open, the application exits.
    fn dismiss(&self) {
        if self.dismissing.replace(true) {
            return;
        }
        // Nobody is left to read the answer.
        self.cancel_ask();
        if self.config.borrow().behavior.stay_resident {
            self.window.set_visible(false);
        } else {
            LAUNCHER.with(|slot| slot.borrow_mut().take());
            self.window.destroy();
        }
        self.dismissing.set(false);
    }

    // ---------- results ----------

    fn refresh(self: &Rc<Self>) {
        self.refresh_with(false);
    }

    fn refresh_with(self: &Rc<Self>, keep_scroll: bool) {
        let query = self.entry.text();
        let config = self.config.borrow().clone();

        // Editing the question abandons the request for the old one.
        let abandoned = self
            .state
            .borrow()
            .ask
            .as_ref()
            .is_some_and(|ask| ask.question != query.trim());
        if abandoned {
            self.cancel_ask();
            self.state.borrow_mut().ask = None;
        }

        let needs = self.state.borrow_mut().update(&query, &config);
        if needs.windows {
            self.load_windows();
        }
        if needs.files {
            self.load_file_index(&config);
        }
        self.render(keep_scroll);
    }

    // ---------- Claude ----------

    fn cancel_ask(&self) {
        self.claude_generation.set(self.claude_generation.get() + 1);
        // Dropping the request kills the CLI process.
        self.claude_request.borrow_mut().take();
    }

    /// Send the question through the Claude Code CLI and stream the answer
    /// into the list. Only ever called on Enter, never while typing.
    fn start_ask(self: &Rc<Self>, question: String, model: String) {
        self.cancel_ask();
        let config = self.config.borrow().claude.clone();

        let mut ask = Ask {
            question: question.clone(),
            model,
            text: String::new(),
            status: AskStatus::Thinking,
        };
        match claude::start(&config, &question) {
            Ok(request) => *self.claude_request.borrow_mut() = Some(request),
            Err(error) => {
                ask.status = AskStatus::Failed;
                ask.text = error;
            }
        }
        {
            let mut state = self.state.borrow_mut();
            state.history.push(&question);
            state.ask = Some(ask);
        }
        self.refresh();

        let generation = self.claude_generation.get();
        let this = self.clone();
        glib::timeout_add_local(ANSWER_POLL_INTERVAL, move || {
            if this.claude_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let events = match this.claude_request.borrow().as_ref() {
                Some(request) => request.poll(),
                None => return glib::ControlFlow::Break,
            };
            if events.is_empty() {
                return glib::ControlFlow::Continue;
            }

            let mut finished = false;
            {
                let mut state = this.state.borrow_mut();
                let Some(ask) = state.ask.as_mut() else {
                    return glib::ControlFlow::Break;
                };
                for event in events {
                    match event {
                        claude::Event::Delta(text) => {
                            ask.text.push_str(&text);
                            ask.status = AskStatus::Answering;
                        }
                        claude::Event::Done(text) => {
                            if !text.is_empty() {
                                ask.text = text;
                            }
                            ask.status = AskStatus::Done;
                            finished = true;
                        }
                        claude::Event::Failed(error) => {
                            ask.text = error;
                            ask.status = AskStatus::Failed;
                            finished = true;
                        }
                    }
                }
            }
            if finished {
                this.claude_request.borrow_mut().take();
            }
            // Keep the reader's place while more text streams in.
            this.refresh_with(true);
            if finished {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// List windows on a worker thread; several helper processes may run.
    fn load_windows(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(window_switcher::list_windows).await;
            this.state.borrow_mut().windows = match result {
                Ok(Ok(windows)) => WindowList::Loaded(windows),
                Ok(Err(error)) => WindowList::Failed(error),
                Err(_) => WindowList::Failed("Listing windows failed".to_string()),
            };
            if this.window.is_visible() {
                this.refresh();
            }
        });
    }

    /// Walk the configured directories on a worker thread.
    fn load_file_index(self: &Rc<Self>, config: &Config) {
        let this = self.clone();
        let files_config = config.files.clone();
        glib::spawn_future_local(async move {
            let entries = gio::spawn_blocking(move || files::build_index(&files_config))
                .await
                .unwrap_or_default();
            this.state.borrow_mut().set_file_index(entries);
            if this.window.is_visible() {
                this.refresh();
            }
        });
    }

    fn render(self: &Rc<Self>, keep_scroll: bool) {
        let (results, emoji_mode, answer_mode) = {
            let state = self.state.borrow();
            (state.results.clone(), state.emoji_mode, state.answer_mode())
        };
        let scroll_position = self.scrolled.vadjustment().value();
        let (width, max_rows) = {
            let config = self.config.borrow();
            (config.theme.width, config.behavior.max_results)
        };

        while let Some(row) = self.list_box.row_at_index(0) {
            self.list_box.remove(&row);
        }
        while let Some(child) = self.emoji_grid.first_child() {
            self.emoji_grid.remove(&child);
        }
        self.emoji_buttons.borrow_mut().clear();
        self.emoji_selected.set(0);

        self.list_box.set_visible(!emoji_mode);
        self.emoji_grid.set_visible(emoji_mode);

        // Height of the part that is visible without scrolling. Measured for
        // the real width, so wrapped text reports its wrapped height.
        let natural_height =
            |widget: &gtk::Widget| widget.measure(gtk::Orientation::Vertical, width).1;
        let mut visible_height = 0;

        if emoji_mode {
            let mut buttons = Vec::with_capacity(results.len());
            for (index, item) in results.iter().enumerate() {
                let ResultItem::Emoji(emoji) = item else {
                    continue;
                };
                let button = rows::emoji_button(emoji);
                let this = self.clone();
                button.connect_clicked(move |_| this.activate_index(index, false));
                self.emoji_grid.attach(
                    &button,
                    (index % EMOJI_COLUMNS) as i32,
                    (index / EMOJI_COLUMNS) as i32,
                    1,
                    1,
                );
                buttons.push(button);
            }
            if let Some(first) = buttons.first() {
                let rows = results
                    .len()
                    .div_ceil(EMOJI_COLUMNS)
                    .min(EMOJI_VISIBLE_ROWS) as i32;
                visible_height = rows * natural_height(first.upcast_ref())
                    + (rows - 1) * EMOJI_SPACING
                    + 2 * EMOJI_MARGIN;
            }
            *self.emoji_buttons.borrow_mut() = buttons;
            self.update_emoji_selection();
        } else {
            for (index, item) in results.iter().enumerate() {
                let row = rows::list_row(item);
                self.list_box.append(&row);
                if index < max_rows {
                    visible_height += natural_height(row.upcast_ref());
                }
            }
            // Highlight what Enter will run.
            match self.list_box.row_at_index(0) {
                Some(first) if first.is_selectable() => self.list_box.select_row(Some(&first)),
                _ => self.list_box.unselect_all(),
            }
        }

        if answer_mode {
            visible_height = visible_height.min(ANSWER_MAX_HEIGHT);
        }
        self.scrolled.set_visible(!results.is_empty());
        self.scrolled.set_max_content_height(visible_height.max(1));
        self.scrolled
            .vadjustment()
            .set_value(if keep_scroll { scroll_position } else { 0.0 });

        // A non-resizable window takes exactly the size we ask for, so ask
        // for what the content needs now that the scroll area is capped.
        let height = self
            .window
            .child()
            .map(|content| content.measure(gtk::Orientation::Vertical, width).1)
            .unwrap_or(ENTRY_HEIGHT + visible_height);
        self.window.set_default_size(width, height);
        self.window.set_size_request(width, height);
    }

    fn update_emoji_selection(&self) {
        let selected = self.emoji_selected.get();
        for (index, button) in self.emoji_buttons.borrow().iter().enumerate() {
            if index == selected {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
    }

    /// Scroll just far enough that `widget` is fully visible.
    fn scroll_into_view(&self, widget: &impl IsA<gtk::Widget>) {
        let Some(bounds) = widget.compute_bounds(&self.results_container) else {
            return;
        };
        let adjustment = self.scrolled.vadjustment();
        let top = f64::from(bounds.y());
        let bottom = top + f64::from(bounds.height());
        if top < adjustment.value() {
            adjustment.set_value(top);
        } else if bottom > adjustment.value() + adjustment.page_size() {
            adjustment.set_value(bottom - adjustment.page_size());
        }
    }

    /// Index Enter acts on: the highlighted item, or the first one.
    fn selected_index(&self) -> Option<usize> {
        let state = self.state.borrow();
        if state.results.is_empty() {
            return None;
        }
        if state.emoji_mode {
            return Some(self.emoji_selected.get().min(state.results.len() - 1));
        }
        let index = self
            .list_box
            .selected_row()
            .and_then(|row| usize::try_from(row.index()).ok())
            .unwrap_or(0);
        Some(index)
    }

    fn selected_item(&self) -> Option<ResultItem> {
        let index = self.selected_index()?;
        self.state.borrow().results.get(index).cloned()
    }

    // ---------- keyboard ----------

    fn on_key(self: &Rc<Self>, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let (count, emoji_mode) = {
            let state = self.state.borrow();
            (state.results.len(), state.emoji_mode)
        };
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);

        // A long answer is read by scrolling; there is nothing to select.
        if !alt && self.state.borrow().answer_mode() {
            let adjustment = self.scrolled.vadjustment();
            let step = match key {
                gdk::Key::Down | gdk::Key::KP_Down => Some(ANSWER_SCROLL_STEP),
                gdk::Key::Up | gdk::Key::KP_Up => Some(-ANSWER_SCROLL_STEP),
                gdk::Key::Page_Down => Some(adjustment.page_size()),
                gdk::Key::Page_Up => Some(-adjustment.page_size()),
                _ => None,
            };
            if let Some(step) = step {
                let max = (adjustment.upper() - adjustment.page_size()).max(0.0);
                adjustment.set_value((adjustment.value() + step).clamp(0.0, max));
                return glib::Propagation::Stop;
            }
        }

        match key {
            gdk::Key::Escape => {
                self.dismiss();
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::KP_Up if alt => {
                self.recall_history(1);
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::KP_Down if alt => {
                self.recall_history(-1);
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter if ctrl => {
                if let Some(index) = self.selected_index() {
                    self.activate_index(index, true);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Tab => {
                if !self.complete_path() {
                    self.move_selection(count, emoji_mode, 1);
                }
                glib::Propagation::Stop
            }
            gdk::Key::ISO_Left_Tab => {
                self.move_selection(count, emoji_mode, -1);
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::KP_Down => {
                if emoji_mode {
                    self.move_emoji(count, |i, last| (i + EMOJI_COLUMNS).min(last));
                } else {
                    self.move_list(count, 1);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::KP_Up => {
                if emoji_mode {
                    self.move_emoji(count, |i, _| if i >= EMOJI_COLUMNS { i - EMOJI_COLUMNS } else { i });
                } else {
                    self.move_list(count, -1);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Page_Down | gdk::Key::Page_Up if !emoji_mode => {
                let page = self.config.borrow().behavior.max_results as i32;
                self.move_list(count, if key == gdk::Key::Page_Down { page } else { -page });
                glib::Propagation::Stop
            }
            gdk::Key::Right | gdk::Key::KP_Right if emoji_mode && count > 0 => {
                self.move_emoji(count, |i, last| (i + 1).min(last));
                glib::Propagation::Stop
            }
            gdk::Key::Left | gdk::Key::KP_Left if emoji_mode && count > 0 => {
                self.move_emoji(count, |i, _| i.saturating_sub(1));
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    fn move_selection(&self, count: usize, emoji_mode: bool, delta: i32) {
        if emoji_mode {
            self.move_emoji(count, |i, last| {
                if delta > 0 {
                    (i + 1).min(last)
                } else {
                    i.saturating_sub(1)
                }
            });
        } else {
            self.move_list(count, delta);
        }
    }

    fn move_emoji(&self, count: usize, step: impl Fn(usize, usize) -> usize) {
        if count == 0 {
            return;
        }
        let last = count - 1;
        let current = self.emoji_selected.get().min(last);
        let next = step(current, last);
        self.emoji_selected.set(next);
        self.update_emoji_selection();
        if let Some(button) = self.emoji_buttons.borrow().get(next) {
            self.scroll_into_view(button);
        }
    }

    fn move_list(&self, count: usize, delta: i32) {
        if count == 0 {
            return;
        }
        let last = count as i32 - 1;
        let target = match self.list_box.selected_row() {
            Some(row) => (row.index() + delta).clamp(0, last),
            None if delta > 0 => 0,
            None => last,
        };
        if let Some(row) = self.list_box.row_at_index(target) {
            self.list_box.select_row(Some(&row));
            self.scroll_into_view(&row);
        }
    }

    /// Alt+Up steps back through past queries, Alt+Down forward again.
    fn recall_history(&self, delta: i32) {
        let entries: Vec<String> = self.state.borrow().history.entries().to_vec();
        if entries.is_empty() {
            return;
        }
        let next = match (self.history_position.get(), delta > 0) {
            (None, true) => Some(0),
            (None, false) => None,
            (Some(position), true) => Some((position + 1).min(entries.len() - 1)),
            (Some(0), false) => None,
            (Some(position), false) => Some(position - 1),
        };
        self.history_position.set(next);
        let text = next.map(|i| entries[i].as_str()).unwrap_or("");
        self.set_query(text);
    }

    /// Replace the search text without ending history stepping.
    fn set_query(&self, text: &str) {
        self.recalling.set(true);
        self.entry.set_text(text);
        self.entry.set_position(-1);
        self.recalling.set(false);
    }

    /// Tab on a file while a path is being typed completes it in the bar.
    fn complete_path(&self) -> bool {
        let text = self.entry.text();
        if !(text.starts_with('/') || text.starts_with('~')) {
            return false;
        }
        let Some(ResultItem::File(file)) = self.selected_item() else {
            return false;
        };
        self.entry.set_text(&files::completion_text(&file));
        self.entry.set_position(-1);
        true
    }

    // ---------- running a result ----------

    fn launch_context(&self) -> Option<gio::AppLaunchContext> {
        // Carries the activation token so the launched app may take focus.
        Some(WidgetExt::display(&self.window).app_launch_context().upcast())
    }

    fn open_file(&self, file: &FileEntry, reveal: bool) -> Result<(), String> {
        // Ctrl+Enter opens the containing folder instead of the file.
        let target = if reveal {
            file.path.parent().unwrap_or(&file.path)
        } else {
            &file.path
        };
        let uri = gio::File::for_path(target).uri();
        gio::AppInfo::launch_default_for_uri(&uri, self.launch_context().as_ref())
            .map_err(|e| format!("Could not open {}: {}", target.display(), e))
    }

    /// `alternate` is Ctrl+Enter: for files, open the containing folder.
    fn activate_index(self: &Rc<Self>, index: usize, alternate: bool) {
        let Some(item) = self.state.borrow().results.get(index).cloned() else {
            return;
        };
        let query = self.entry.text().to_string();
        let mut remember = true;

        let outcome = match item {
            ResultItem::Info(_) => return,
            ResultItem::Ask { question, model } => {
                self.start_ask(question, model);
                return;
            }
            ResultItem::Answer(ask) => {
                if ask.status != AskStatus::Done {
                    return;
                }
                // The question is already in history from when it was asked.
                remember = false;
                utils::copy_to_clipboard(&ask.text).or_else(|_| {
                    WidgetExt::display(&self.window).clipboard().set_text(&ask.text);
                    Ok(())
                })
            }
            ResultItem::History(past) => {
                // Bring the query back for editing or another Enter.
                self.entry.set_text(&past);
                self.entry.set_position(-1);
                return;
            }
            ResultItem::ClearHistory => {
                self.state.borrow_mut().history.clear();
                self.entry.set_text("");
                return;
            }
            ResultItem::Settings => {
                remember = false;
                // Open first so the application always has a window.
                SettingsWindow::open(&self.app, self.config.clone());
                Ok(())
            }
            ResultItem::Emoji(emoji) => {
                self.state.borrow_mut().history.push(&query);
                self.insert_emoji_after_dismiss(emoji.emoji);
                return;
            }
            ResultItem::App(app) => {
                // Launch counts already rank apps; no need to remember "fire".
                remember = false;
                let result = AppLauncher::launch(&app, self.launch_context().as_ref());
                if result.is_ok() {
                    self.state.borrow_mut().usage.record(&app.id);
                }
                result
            }
            ResultItem::Calc { result, .. } => utils::copy_to_clipboard(&result).or_else(|_| {
                // No clipboard tool: GTK's clipboard works while we keep running.
                WidgetExt::display(&self.window).clipboard().set_text(&result);
                Ok(())
            }),
            ResultItem::Terminal(command) => Terminal::execute_command(&command),
            ResultItem::Search { engine, query } => {
                search::open(engine, &query, self.launch_context().as_ref())
            }
            ResultItem::Window { window, .. } => {
                remember = false;
                window_switcher::activate(&window)
            }
            ResultItem::File(file) => self.open_file(&file, alternate),
            ResultItem::Alias { target, .. } => alias::run(&target, self.launch_context().as_ref()),
        };

        match outcome {
            Ok(()) => {
                if remember {
                    self.state.borrow_mut().history.push(&query);
                }
                self.dismiss();
            }
            Err(error) => self.show_error(&error),
        }
    }

    /// Typing tools send keystrokes to whichever window has focus, so the
    /// launcher has to be gone before they run.
    fn insert_emoji_after_dismiss(self: &Rc<Self>, emoji: &'static str) {
        // Keep the application alive until the emoji has been delivered.
        let hold = self.app.hold();
        self.dismiss();
        glib::timeout_add_local_once(FOCUS_RETURN_DELAY, move || {
            if let Err(error) = EmojiPicker::insert(emoji) {
                eprintln!("Could not insert emoji: {}", error);
            }
            drop(hold);
        });
    }

    fn show_error(self: &Rc<Self>, error: &str) {
        eprintln!("Error: {}", error);
        {
            let mut state = self.state.borrow_mut();
            state.emoji_mode = false;
            state.results = vec![ResultItem::Info(format!("Error: {}", error))];
        }
        self.render(false);
    }
}
