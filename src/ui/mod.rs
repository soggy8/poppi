//! The launcher window.
//!
//! - `mod.rs`: widgets, showing and hiding, turning state into rows
//! - `keys.rs`: keyboard handling
//! - `activate.rs`: running the selected result
//! - `chat.rs`: the conversation with an AI model

mod activate;
mod chat;
mod dictation;
mod keys;
#[cfg(feature = "ui-test")]
mod smoke;

use crate::ai::{self, providers};
use crate::app_launcher::AppLauncher;
use crate::config::{Config, FilesConfig};
use crate::files;
use crate::rows::{self, HintSlot, RowContext, RowSlot, TranscriptView};
use crate::scripts::{self, Script, ScriptCatalog};
use crate::state::{
    LauncherState, Mode, ResultItem, WindowList, EMOJI_COLUMNS, EMOJI_VISIBLE_ROWS, RESULT_LIMIT,
};
use crate::style;
use crate::units::Rates;
use crate::usage;
use crate::window_switcher;
use gtk::prelude::*;
use gtk::{gio, glib};
use gtk::{
    Application, Box as GtkBox, Button, Entry, EventControllerKey, Grid, Image, Label, ListBox,
    ScrolledWindow, Window,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Transparent space around the card where its shadow is drawn.
/// Keep in sync with `.launcher-box` in style.css.
const CARD_MARGIN_SIDE: i32 = 32;
/// Padding inside the result list. Keep in sync with `list` in style.css.
const LIST_PADDING: i32 = 6;
/// The window starts taller than its content (the rest is transparent) so the
/// compositor centres it with room for results below the search bar.
const INITIAL_HEIGHT: i32 = 520;
const EMOJI_SPACING: i32 = 6;
const EMOJI_MARGIN: i32 = 10;
/// Tallest the conversation gets before it scrolls.
const CHAT_MAX_HEIGHT: i32 = 400;
/// Gap between the top of the screen and the launcher as a layer surface,
/// as a fraction of the monitor height.
#[cfg(feature = "layer-shell")]
const LAYER_TOP_FRACTION: f64 = 0.2;

/// On compositors with wlr layer-shell (Sway, Hyprland, Niri, KDE) show the
/// launcher as an overlay; as a plain window it would be tiled like any app.
/// GNOME has no layer-shell, so there the window stays as it is.
#[cfg(feature = "layer-shell")]
fn init_layer_shell(window: &Window) {
    use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
    if !gtk4_layer_shell::is_supported() {
        return;
    }
    window.init_layer_shell();
    window.set_namespace(Some("poppi-launcher"));
    window.set_layer(Layer::Overlay);
    // Takes the keyboard while open; Escape or running a result dismisses it.
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    // Anchored to the top only: centered horizontally, and the search bar
    // stays put while the result list grows and shrinks below it.
    window.set_anchor(Edge::Top, true);
    let monitor_height = WidgetExt::display(window)
        .monitors()
        .item(0)
        .and_downcast::<gtk::gdk::Monitor>()
        .map(|monitor| monitor.geometry().height())
        .unwrap_or(1080);
    window.set_margin(
        Edge::Top,
        (monitor_height as f64 * LAYER_TOP_FRACTION) as i32,
    );
}

/// How long typing a new folder may wait for its listing before showing
/// "Reading folder…" and finishing in the background.
const DIR_LIST_WAIT: Duration = Duration::from_millis(25);
/// Password managers (KeePassXC, KDE apps) mark copied secrets with this.
const PASSWORD_HINT_MIME: &str = "x-kde-passwordManagerHint";
const EMPTY_TIPS: &str = "f files  ·  sw windows  ·  : emoji  ·  clip clipboard  ·  ? ask AI";

thread_local! {
    static LAUNCHER: RefCell<Option<Rc<Launcher>>> = const { RefCell::new(None) };
}

struct Launcher {
    app: Application,
    window: Window,
    entry: Entry,
    mode_chip: Label,
    mic_button: Button,
    divider: GtkBox,
    list_box: ListBox,
    emoji_grid: Grid,
    results_container: GtkBox,
    scrolled: ScrolledWindow,
    footer_status: Label,
    footer_keys: GtkBox,
    config: Rc<RefCell<Config>>,
    state: RefCell<LauncherState>,
    _app_monitor: gio::AppInfoMonitor,
    apps_reloading: Cell<bool>,
    _scripts_monitor: RefCell<Option<gio::FileMonitor>>,
    scripts_reloading: Cell<bool>,
    file_monitors: RefCell<Vec<gio::FileMonitor>>,
    watched_files_config: RefCell<Option<FilesConfig>>,
    grep_gen: Arc<AtomicU64>,
    active_rows: RefCell<Vec<RowSlot>>,
    row_pool: RefCell<Vec<RowSlot>>,
    transcript_view: RefCell<TranscriptView>,
    emoji_buttons: RefCell<Vec<Button>>,
    emoji_visible_count: Cell<usize>,
    emoji_selected: Cell<usize>,
    hint_slots: RefCell<Vec<HintSlot>>,
    footer_notice: RefCell<Option<String>>,
    /// Position while stepping through history with Alt+Up/Down.
    history_position: Cell<Option<usize>>,
    /// Set while we change the entry text ourselves during history stepping.
    recalling: Cell<bool>,
    dismissing: Cell<bool>,
    /// The model session for the current conversation, if started.
    ai: RefCell<Option<ai::Session>>,
    ai_polling: Cell<bool>,
    /// Keep the newest part of the conversation in view as it grows.
    chat_follow: Cell<bool>,
    dictation: RefCell<dictation::Dictation>,
    /// Listens for the wake word while the launcher is hidden, if enabled.
    wake: RefCell<Option<crate::wake::Listener>>,
    wake_polling: Cell<bool>,
    /// Opened by voice: harmless results run without Enter.
    hands_free: Cell<bool>,
    /// Bumped when dictation starts or is cancelled, so a transcript that
    /// arrives afterwards is dropped.
    dictation_gen: Cell<u64>,
}

/// Build the resident launcher in the background without showing the window
/// (`poppi_launcher --daemon`).
pub fn warm_up(app: &Application, config: &Rc<RefCell<Config>>, config_error: Option<&str>) {
    LAUNCHER.with(|slot| {
        if slot.borrow().is_none() {
            *slot.borrow_mut() = Some(Launcher::build(app, config.clone(), config_error));
        }
    });
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

/// The config changed (settings saved, file edited, desktop colour scheme
/// switched): invalidate affected caches and redraw an open launcher.
pub fn config_changed() {
    let launcher = LAUNCHER.with(|slot| slot.borrow().clone());
    if let Some(launcher) = launcher {
        launcher.on_config_changed();
    }
}

impl Launcher {
    fn build(
        app: &Application,
        config: Rc<RefCell<Config>>,
        config_error: Option<&str>,
    ) -> Rc<Self> {
        let window = Window::builder()
            .application(app)
            .title("Poppi Launcher")
            .decorated(false)
            .resizable(false)
            .build();
        window.add_css_class("poppi-launcher");
        #[cfg(feature = "layer-shell")]
        init_layer_shell(&window);

        let main_box = GtkBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Start)
            .build();
        main_box.add_css_class("launcher-box");
        // Clip children to the rounded corners.
        main_box.set_overflow(gtk::Overflow::Hidden);

        // ----- search bar -----
        let search_bar = GtkBox::new(gtk::Orientation::Horizontal, 0);
        search_bar.add_css_class("search-bar");
        let search_icon = Image::from_icon_name("system-search-symbolic");
        search_icon.set_pixel_size(18);
        search_icon.add_css_class("search-icon");

        let placeholder = match config_error {
            Some(_) => "Config file has errors, using defaults (details on stderr)",
            None => "Search, calculate, or ask something?",
        };
        let entry = Entry::builder()
            .placeholder_text(placeholder)
            .has_frame(false)
            .hexpand(true)
            .build();

        let mode_chip = Label::new(None);
        mode_chip.add_css_class("mode-chip");
        mode_chip.set_valign(gtk::Align::Center);
        mode_chip.set_visible(false);

        let mic_button = Button::from_icon_name("audio-input-microphone-symbolic");
        mic_button.add_css_class("mic-button");
        mic_button.set_has_frame(false);
        mic_button.set_focus_on_click(false);
        mic_button.set_can_focus(false);
        mic_button.set_valign(gtk::Align::Center);
        mic_button.set_tooltip_text(Some("Dictate (Ctrl+D)"));
        mic_button.set_visible(false);

        search_bar.append(&search_icon);
        search_bar.append(&entry);
        search_bar.append(&mode_chip);
        search_bar.append(&mic_button);

        let divider = GtkBox::new(gtk::Orientation::Horizontal, 0);
        divider.add_css_class("divider");
        divider.set_visible(false);

        // ----- results -----
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

        let results_container = GtkBox::new(gtk::Orientation::Vertical, 0);
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

        // ----- footer -----
        let footer = GtkBox::new(gtk::Orientation::Horizontal, 12);
        footer.add_css_class("footer");
        let footer_status = Label::new(None);
        footer_status.set_xalign(0.0);
        footer_status.set_hexpand(true);
        footer_status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let footer_keys = GtkBox::new(gtk::Orientation::Horizontal, 6);
        footer.append(&footer_status);
        footer.append(&footer_keys);

        main_box.append(&search_bar);
        main_box.append(&divider);
        main_box.append(&scrolled);
        main_box.append(&footer);
        window.set_child(Some(&main_box));

        let mut state = LauncherState::new(&config.borrow());
        // Load installed applications once up front; AppInfoMonitor refreshes
        // them whenever `.desktop` files change on disk.
        state.apps = AppLauncher::load();

        let launcher = Rc::new(Self {
            app: app.clone(),
            window,
            entry,
            mode_chip,
            mic_button,
            divider,
            list_box,
            emoji_grid,
            results_container,
            scrolled,
            footer_status,
            footer_keys,
            config,
            state: RefCell::new(state),
            _app_monitor: gio::AppInfoMonitor::get(),
            apps_reloading: Cell::new(false),
            _scripts_monitor: RefCell::new(None),
            scripts_reloading: Cell::new(false),
            file_monitors: RefCell::new(Vec::new()),
            watched_files_config: RefCell::new(None),
            grep_gen: Arc::new(AtomicU64::new(0)),
            active_rows: RefCell::new(Vec::new()),
            row_pool: RefCell::new(Vec::new()),
            transcript_view: RefCell::new(TranscriptView::new()),
            emoji_buttons: RefCell::new(Vec::new()),
            emoji_visible_count: Cell::new(0),
            emoji_selected: Cell::new(0),
            hint_slots: RefCell::new(Vec::new()),
            footer_notice: RefCell::new(None),
            history_position: Cell::new(None),
            recalling: Cell::new(false),
            dismissing: Cell::new(false),
            ai: RefCell::new(None),
            ai_polling: Cell::new(false),
            chat_follow: Cell::new(false),
            dictation: RefCell::new(dictation::Dictation::default()),
            wake: RefCell::new(None),
            wake_polling: Cell::new(false),
            hands_free: Cell::new(false),
            dictation_gen: Cell::new(0),
        });
        launcher.connect_signals();
        launcher.update_wake_listener();
        launcher
    }

    fn connect_signals(self: &Rc<Self>) {
        let this = self.clone();
        self.mic_button
            .connect_clicked(move |_| this.toggle_dictation());

        let this = self.clone();
        self.entry.connect_changed(move |_| {
            if !this.recalling.get() {
                this.history_position.set(None);
            }
            *this.footer_notice.borrow_mut() = None;
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
        self.list_box
            .connect_row_selected(move |_, _| this.update_footer());

        // Reload applications only when desktop entries actually change.
        let this = self.clone();
        self._app_monitor.connect_changed(move |_| {
            if this.apps_reloading.replace(true) {
                return;
            }
            let this = this.clone();
            glib::idle_add_local_once(move || {
                this.apps_reloading.set(false);
                this.state.borrow_mut().apps = AppLauncher::load();
                this.refresh_if_visible();
            });
        });

        let weak = Rc::downgrade(self);
        *self._scripts_monitor.borrow_mut() = scripts::watch_scripts_dir(move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.scripts_reloading.replace(true) {
                return;
            }
            glib::idle_add_local_once(move || {
                this.scripts_reloading.set(false);
                this.state.borrow_mut().scripts = ScriptCatalog::load();
                this.refresh_if_visible();
            });
        });

        // While the conversation is being followed, stay at its end as it grows.
        let this = self.clone();
        self.scrolled
            .vadjustment()
            .connect_changed(move |adjustment| {
                if this.chat_follow.get() {
                    adjustment.set_value(adjustment.upper() - adjustment.page_size());
                }
            });

        let this = self.clone();
        self.window.connect_is_active_notify(move |window| {
            if window.is_active() {
                // The clipboard is only readable while we have focus.
                this.remember_clipboard();
                return;
            }
            let enabled = this.config.borrow().behavior.close_on_focus_loss;
            if enabled && window.is_visible() {
                this.dismiss();
            }
        });

        let this = self.clone();
        WidgetExt::display(&self.window)
            .clipboard()
            .connect_changed(move |_| this.remember_clipboard());

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

    fn on_config_changed(self: &Rc<Self>) {
        let files_cfg = self.config.borrow().files.clone();
        let files_changed = self
            .watched_files_config
            .borrow()
            .as_ref()
            .is_some_and(|prev| *prev != files_cfg);
        if files_changed {
            self.file_monitors.borrow_mut().clear();
            *self.watched_files_config.borrow_mut() = None;
            self.state.borrow_mut().invalidate_file_index();
        }
        self.update_wake_listener();
        if self.window.is_visible() {
            self.refresh();
        }
    }

    // ---------- showing and hiding ----------

    fn window_width(&self) -> i32 {
        self.config.borrow().theme.width + 2 * CARD_MARGIN_SIDE
    }

    fn show(self: &Rc<Self>) {
        self.end_conversation();
        self.cancel_dictation();
        self.hands_free.set(false);
        self.pause_wake_listener(true);
        self.update_mic_button();
        self.state.borrow_mut().reset_session();
        self.history_position.set(None);
        *self.footer_notice.borrow_mut() = None;
        if self.entry.text().is_empty() {
            self.refresh();
        } else {
            // Emits `changed`, which refreshes.
            self.entry.set_text("");
        }

        let width = self.window_width();
        self.window.set_size_request(width, -1);
        self.resize_window_to_content();
        self.window.set_opacity(0.0);
        self.window.present();
        self.entry.grab_focus();
        self.fade_in();

        #[cfg(feature = "ui-test")]
        smoke::run(self);
    }

    fn fade_in(&self) {
        let window = self.window.clone();
        let mut opacity = 0.0;
        glib::timeout_add_local(Duration::from_millis(10), move || {
            opacity += 0.07;
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
        // Nobody is left to read the answer, and nobody is talking to us.
        self.end_conversation();
        self.cancel_dictation();
        self.hands_free.set(false);
        self.pause_wake_listener(false);
        if self.config.borrow().behavior.stay_resident {
            self.window.set_visible(false);
        } else {
            LAUNCHER.with(|slot| slot.borrow_mut().take());
            self.window.destroy();
        }
        self.dismissing.set(false);
    }

    /// Remember the text on the clipboard, if any and if enabled.
    fn remember_clipboard(self: &Rc<Self>) {
        if !self.config.borrow().clipboard.enabled {
            return;
        }
        let clipboard = WidgetExt::display(&self.window).clipboard();
        // Never keep passwords, not even in memory.
        if clipboard.formats().contain_mime_type(PASSWORD_HINT_MIME) {
            return;
        }
        let this = self.clone();
        clipboard.read_text_async(gio::Cancellable::NONE, move |result| {
            if let Ok(Some(text)) = result {
                this.state.borrow_mut().clipboard.push(&text);
            }
        });
    }

    // ---------- results ----------

    fn refresh(self: &Rc<Self>) {
        let query = self.entry.text();
        let config = self.config.borrow().clone();
        let mut needs = self.state.borrow_mut().update(&query, &config);
        if let Some(dir) = needs.dir.take() {
            if self.load_dir(dir) {
                // Listed fast enough to show without a "Reading folder…" flash.
                needs = self.state.borrow_mut().update(&query, &config);
            }
        }
        if needs.windows {
            self.load_windows();
        }
        if needs.files {
            self.load_file_index(&config);
        }
        if needs.rates {
            self.load_rates();
        }
        if needs.ai {
            // Start the CLI while the question is still being typed.
            self.warm_up_ai();
        }
        if let Some((script, filter)) = needs.script {
            self.load_script(script, filter);
        }
        if let Some((pattern, index)) = needs.grep {
            self.load_grep(pattern, index);
        } else if !query.starts_with("g ") && !query.starts_with("grep ") {
            // Cancel any in-flight content scan as soon as the user leaves grep mode.
            self.grep_gen.fetch_add(1, Ordering::Relaxed);
        }
        self.render();
    }

    /// Lists `dir` on a worker thread. Returns true if it finished within
    /// `DIR_LIST_WAIT` and is already in the state; otherwise the listing is
    /// applied whenever it arrives, so a hung mount never blocks typing.
    fn load_dir(self: &Rc<Self>, dir: PathBuf) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_dir = dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(files::list_dir(&thread_dir));
        });
        if let Ok(listing) = rx.recv_timeout(DIR_LIST_WAIT) {
            self.state.borrow_mut().set_dir_listing(dir, listing);
            return true;
        }
        let this = self.clone();
        glib::timeout_add_local(Duration::from_millis(30), move || match rx.try_recv() {
            Ok(listing) => {
                this.state
                    .borrow_mut()
                    .set_dir_listing(dir.clone(), listing);
                this.refresh_if_visible();
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        });
        false
    }

    fn load_grep(self: &Rc<Self>, pattern: String, index: Arc<Vec<files::FileEntry>>) {
        let my_gen = self.grep_gen.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel_gen = self.grep_gen.clone();
        let pattern_key = pattern.clone();
        let now = usage::now();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || {
                files::search_contents(&pattern, &index, now, &cancel_gen, my_gen, RESULT_LIMIT)
            })
            .await
            .ok()
            .flatten();
            if let Some(matches) = found {
                this.state
                    .borrow_mut()
                    .set_grep_results(pattern_key, matches);
                this.refresh_if_visible();
            }
        });
    }

    fn load_script(self: &Rc<Self>, script: Script, filter: String) {
        let this = self.clone();
        let keyword = script.keyword.clone();
        let filter_key = filter.clone();
        glib::spawn_future_local(async move {
            let outcome = gio::spawn_blocking(move || scripts::query_list_script(&script, &filter))
                .await
                .unwrap_or_else(|_| Err("Running script failed".to_string()));
            this.state
                .borrow_mut()
                .set_script_results(keyword, filter_key, outcome);
            this.refresh_if_visible();
        });
    }

    pub(super) fn toggle_action_menu(self: &Rc<Self>) -> bool {
        if self.state.borrow().in_action_menu() {
            return self.leave_action_menu();
        }
        let Some(item) = self.selected_item() else {
            return false;
        };
        let current = self.entry.text().to_string();
        if !self.state.borrow_mut().open_action_menu(&item, &current) {
            return false;
        }
        self.recalling.set(true);
        if current.is_empty() {
            self.refresh();
        } else {
            self.entry.set_text("");
        }
        self.recalling.set(false);
        true
    }

    pub(super) fn leave_action_menu(self: &Rc<Self>) -> bool {
        let Some(saved) = self.state.borrow_mut().close_action_menu() else {
            return false;
        };
        self.recalling.set(true);
        if self.entry.text() == saved {
            self.refresh();
        } else {
            self.entry.set_text(&saved);
            self.entry.set_position(-1);
        }
        self.recalling.set(false);
        true
    }

    pub(super) fn close_window_in_place(
        self: &Rc<Self>,
        window: crate::window_switcher::OpenWindow,
    ) {
        match window_switcher::close(&window) {
            Ok(()) => {
                let saved = self.state.borrow_mut().close_action_menu();
                self.state.borrow_mut().remove_window(&window.id);
                if let Some(saved) = saved {
                    self.recalling.set(true);
                    if self.entry.text() == saved {
                        self.refresh();
                    } else {
                        self.entry.set_text(&saved);
                        self.entry.set_position(-1);
                    }
                    self.recalling.set(false);
                } else {
                    self.refresh();
                }
                self.flash_footer_notice("Closed window");
            }
            Err(error) => self.show_error(&error),
        }
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
            this.refresh_if_visible();
        });
    }

    fn ensure_file_monitors(self: &Rc<Self>, files_config: &FilesConfig) {
        if self.watched_files_config.borrow().as_ref() == Some(files_config) {
            return;
        }
        let weak = Rc::downgrade(self);
        let monitors = files::watch_directories(files_config, move || {
            if let Some(this) = weak.upgrade() {
                this.state.borrow_mut().invalidate_file_index();
            }
        });
        *self.file_monitors.borrow_mut() = monitors;
        *self.watched_files_config.borrow_mut() = Some(files_config.clone());
    }

    /// Walk the configured directories on a worker thread.
    fn load_file_index(self: &Rc<Self>, config: &Config) {
        self.ensure_file_monitors(&config.files);
        let this = self.clone();
        let files_config = config.files.clone();
        glib::spawn_future_local(async move {
            let entries = gio::spawn_blocking(move || files::build_index(&files_config))
                .await
                .unwrap_or_default();
            this.state.borrow_mut().set_file_index(entries);
            this.refresh_if_visible();
        });
    }

    /// Download exchange rates on a worker thread.
    fn load_rates(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let rates = gio::spawn_blocking(Rates::fetch)
                .await
                .ok()
                .and_then(Result::ok);
            this.state.borrow_mut().set_rates(rates);
            this.refresh_if_visible();
        });
    }

    fn refresh_if_visible(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.refresh();
        }
    }

    fn detach_active_rows(&self) {
        let mut active = self.active_rows.borrow_mut();
        let mut pool = self.row_pool.borrow_mut();
        while let Some(slot) = active.pop() {
            if slot.row.is_selected() {
                self.list_box.unselect_row(&slot.row);
            }
            self.list_box.remove(&slot.row);
            pool.push(slot);
        }
    }

    fn resize_active_rows(&self, desired: usize) {
        let mut active = self.active_rows.borrow_mut();
        let mut pool = self.row_pool.borrow_mut();
        while active.len() > desired {
            if let Some(slot) = active.pop() {
                if slot.row.is_selected() {
                    self.list_box.unselect_row(&slot.row);
                }
                self.list_box.remove(&slot.row);
                pool.push(slot);
            }
        }
        while active.len() < desired {
            let slot = pool.pop().unwrap_or_else(RowSlot::new);
            self.list_box.append(&slot.row);
            active.push(slot);
        }
    }

    fn resize_window_to_content(&self) {
        let width = self.window_width();
        let height = self
            .window
            .child()
            .map(|content| content.measure(gtk::Orientation::Vertical, width).1)
            .unwrap_or(INITIAL_HEIGHT);
        self.window.set_default_size(width, height);
        self.window.set_size_request(width, height);
    }

    /// Update the streaming conversation in place without rebuilding the list
    /// or re-parsing earlier turns.
    pub(super) fn sync_streaming_chat(self: &Rc<Self>, status_changed: bool) {
        let turns = {
            let mut state = self.state.borrow_mut();
            let turns = state.chat.clone();
            if let Some(ResultItem::Transcript { turns: slot }) = state.results.first_mut() {
                *slot = turns.clone();
            }
            turns
        };
        if !self.window.is_visible() {
            return;
        }
        // A finished answer that proposes actions gets its rows below it.
        let proposes = turns.last().is_some_and(|turn| {
            turn.status == crate::state::TurnStatus::Done
                && !crate::actions::split(&turn.text).1.is_empty()
        });
        if status_changed && proposes {
            self.refresh();
            self.run_proposed_hands_free();
            return;
        }
        self.transcript_view.borrow_mut().sync(&turns);

        let list_width = self.config.borrow().theme.width - 2;
        let natural = self
            .list_box
            .upcast_ref::<gtk::Widget>()
            .measure(gtk::Orientation::Vertical, list_width)
            .1;
        self.scrolled
            .set_max_content_height(natural.clamp(1, CHAT_MAX_HEIGHT));
        if status_changed {
            self.update_footer();
        }
        self.resize_window_to_content();
    }

    pub(super) fn flash_footer_notice(self: &Rc<Self>, message: &str) {
        *self.footer_notice.borrow_mut() = Some(message.to_string());
        self.update_footer();
        let weak = Rc::downgrade(self);
        let expected = message.to_string();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            if let Some(this) = weak.upgrade() {
                let clear = this
                    .footer_notice
                    .borrow()
                    .as_deref()
                    .is_some_and(|current| current == expected);
                if clear {
                    *this.footer_notice.borrow_mut() = None;
                    this.update_footer();
                }
            }
        });
    }

    fn render(self: &Rc<Self>) {
        let (results, emoji_mode, answer_mode, mode, highlight) = {
            let state = self.state.borrow();
            (
                state.results.clone(),
                state.emoji_mode,
                state.answer_mode(),
                state.mode,
                state.highlight.clone(),
            )
        };
        let (card_width, max_rows, accent) = {
            let config = self.config.borrow();
            (
                config.theme.width,
                config.behavior.max_results,
                style::accent_hex(&config),
            )
        };

        self.emoji_selected.set(0);
        self.list_box.set_visible(!emoji_mode);
        self.emoji_grid.set_visible(emoji_mode);
        self.mode_chip.set_text(mode.label());
        self.mode_chip.set_visible(mode != Mode::Empty);

        // Width available to rows, so wrapped text reports its real height.
        let list_width = card_width - 2;
        let natural_height =
            |widget: &gtk::Widget, width: i32| widget.measure(gtk::Orientation::Vertical, width).1;
        // Height of the part that is visible without scrolling.
        let mut visible_height = 0;

        if emoji_mode {
            let transcript_row = self.transcript_view.borrow().row.clone();
            if transcript_row.parent().is_some() {
                self.list_box.remove(&transcript_row);
            }
            self.detach_active_rows();

            {
                let mut buttons = self.emoji_buttons.borrow_mut();
                for (index, item) in results.iter().enumerate() {
                    let ResultItem::Emoji(emoji) = item else {
                        continue;
                    };
                    if let Some(button) = buttons.get(index) {
                        rows::update_emoji_button(button, emoji);
                        button.set_visible(true);
                    } else {
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
                }
                for button in buttons.iter().skip(results.len()) {
                    button.set_visible(false);
                }
                self.emoji_visible_count.set(results.len());
                if let Some(first) = buttons.first().filter(|_| !results.is_empty()) {
                    let rows = results
                        .len()
                        .div_ceil(EMOJI_COLUMNS)
                        .min(EMOJI_VISIBLE_ROWS) as i32;
                    visible_height = rows * natural_height(first.upcast_ref(), -1)
                        + (rows - 1) * EMOJI_SPACING
                        + 2 * EMOJI_MARGIN;
                }
            }
            self.update_emoji_selection();
        } else if !results.is_empty() {
            let state = self.state.borrow();
            let context = RowContext {
                matcher: state.matcher(),
                highlight: &highlight,
                accent: &accent,
            };

            if let Some(ResultItem::Transcript { turns }) = results.first() {
                let transcript_row = {
                    let mut view = self.transcript_view.borrow_mut();
                    view.sync(turns);
                    view.row.clone()
                };
                if transcript_row.parent().is_none() {
                    self.detach_active_rows();
                    self.list_box.append(&transcript_row);
                }
                let extra = &results[1..];
                self.resize_active_rows(extra.len());
                for (slot, item) in self.active_rows.borrow().iter().zip(extra) {
                    slot.update(item, &context);
                }
                drop(state);
                visible_height =
                    natural_height(self.list_box.upcast_ref(), list_width).min(CHAT_MAX_HEIGHT);
            } else {
                let transcript_row = self.transcript_view.borrow().row.clone();
                if transcript_row.parent().is_some() {
                    self.list_box.remove(&transcript_row);
                }
                self.resize_active_rows(results.len());
                let mut first_rows_height = 0;
                for (index, (slot, item)) in
                    self.active_rows.borrow().iter().zip(&results).enumerate()
                {
                    slot.update(item, &context);
                    if index < max_rows {
                        first_rows_height +=
                            natural_height(slot.row.upcast_ref(), list_width - 2 * LIST_PADDING);
                    }
                }
                drop(state);
                visible_height = if results.len() <= max_rows {
                    natural_height(self.list_box.upcast_ref(), list_width)
                } else {
                    first_rows_height + 2 * LIST_PADDING
                };
            }

            // Highlight what Enter will run: the first row; in a
            // conversation the question being typed, or else the first
            // action the answer proposed.
            let preferred = if !answer_mode {
                0
            } else if matches!(results.last(), Some(ResultItem::Ask { .. })) {
                results.len() as i32 - 1
            } else {
                1
            };
            match self.list_box.row_at_index(preferred) {
                Some(row) if row.is_selectable() => self.list_box.select_row(Some(&row)),
                _ => self.list_box.unselect_all(),
            }
        } else {
            let transcript_row = self.transcript_view.borrow().row.clone();
            if transcript_row.parent().is_some() {
                self.list_box.remove(&transcript_row);
            }
            self.detach_active_rows();
            self.list_box.unselect_all();
        }

        let has_results = !results.is_empty();
        self.divider.set_visible(has_results);
        self.scrolled.set_visible(has_results);
        self.scrolled.set_max_content_height(visible_height.max(1));
        if !answer_mode {
            self.chat_follow.set(false);
            self.scrolled.vadjustment().set_value(0.0);
        }
        self.update_footer();
        self.resize_window_to_content();
    }

    fn update_emoji_selection(&self) {
        let selected = self.emoji_selected.get();
        let visible = self.emoji_visible_count.get();
        for (index, button) in self.emoji_buttons.borrow().iter().take(visible).enumerate() {
            if index == selected {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
    }

    /// The footer: a status on the left, and on the right what the keys do
    /// for the item Enter would act on.
    fn update_footer(&self) {
        let selected = self.selected_item();
        let (count, mode, in_chat, busy, in_actions) = {
            let state = self.state.borrow();
            (
                state.results.len(),
                state.mode,
                state.answer_mode(),
                state.chat_busy(),
                state.in_action_menu(),
            )
        };
        let query = self.entry.text();
        let typing_path = !in_actions && (query.starts_with('/') || query.starts_with('~'));

        let status = if let Some(notice) = self.footer_notice.borrow().as_deref() {
            notice.to_string()
        } else if in_actions {
            "Type to filter actions".to_string()
        } else if in_chat {
            let config = self.config.borrow();
            let label = providers::label(&config.ai, &self.state.borrow().chat_model(&config));
            if busy {
                format!("{} · answering…", label)
            } else {
                format!("{} · type a follow-up", label)
            }
        } else if query.is_empty() {
            EMPTY_TIPS.to_string()
        } else if count > 1 && !matches!(mode, Mode::Ai) {
            format!("{} results", count)
        } else {
            String::new()
        };
        self.footer_status.set_text(&status);

        let desired = if self.recording() {
            vec![("↵", "Finish"), ("Esc", "Cancel")]
        } else if self.dictating() {
            vec![("Esc", "Cancel")]
        } else {
            rows::hints(selected.as_ref(), typing_path, in_chat, busy, in_actions)
        };
        let mut slots = self.hint_slots.borrow_mut();
        while slots.len() < desired.len() {
            slots.push(HintSlot::new(&self.footer_keys));
        }
        for (slot, (key, action)) in slots.iter().zip(&desired) {
            slot.set(key, action);
        }
        for slot in slots.iter().skip(desired.len()) {
            slot.hide();
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
        Some(index.min(state.results.len() - 1))
    }

    fn selected_item(&self) -> Option<ResultItem> {
        let index = self.selected_index()?;
        self.state.borrow().results.get(index).cloned()
    }

    fn launch_context(&self) -> Option<gio::AppLaunchContext> {
        // Carries the activation token so the launched app may take focus.
        Some(
            WidgetExt::display(&self.window)
                .app_launch_context()
                .upcast(),
        )
    }

    fn copy_text(&self, text: &str) -> Result<(), String> {
        crate::utils::copy_to_clipboard(text).or_else(|_| {
            // No clipboard tool: GTK's clipboard works while we keep running.
            WidgetExt::display(&self.window).clipboard().set_text(text);
            Ok(())
        })
    }
}
