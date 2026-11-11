//! The launcher window.
//!
//! - `mod.rs`: widgets, showing and hiding, turning state into rows
//! - `keys.rs`: keyboard handling
//! - `activate.rs`: running the selected result
//! - `chat.rs`: the conversation with Claude

mod activate;
mod chat;
mod keys;
#[cfg(feature = "ui-test")]
mod smoke;

use crate::app_launcher::AppLauncher;
use crate::claude;
use crate::config::Config;
use crate::files;
use crate::rows::{self, RowContext};
use crate::state::{
    LauncherState, Mode, ResultItem, WindowList, EMOJI_COLUMNS, EMOJI_VISIBLE_ROWS,
};
use crate::style;
use crate::units::Rates;
use crate::window_switcher;
use gtk::prelude::*;
use gtk::{gio, glib};
use gtk::{
    Application, Box as GtkBox, Button, Entry, EventControllerKey, Grid, Image, Label, ListBox,
    ScrolledWindow, Window,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
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
/// Tallest the conversation with Claude gets before it scrolls.
const CHAT_MAX_HEIGHT: i32 = 400;
const EMPTY_TIPS: &str = "f files  ·  sw windows  ·  : emoji  ·  clip clipboard  ·  ? ask Claude";

thread_local! {
    static LAUNCHER: RefCell<Option<Rc<Launcher>>> = const { RefCell::new(None) };
}

struct Launcher {
    app: Application,
    window: Window,
    entry: Entry,
    mode_chip: Label,
    divider: GtkBox,
    list_box: ListBox,
    emoji_grid: Grid,
    results_container: GtkBox,
    scrolled: ScrolledWindow,
    footer_status: Label,
    footer_keys: GtkBox,
    config: Rc<RefCell<Config>>,
    state: RefCell<LauncherState>,
    emoji_buttons: RefCell<Vec<Button>>,
    emoji_selected: Cell<usize>,
    /// Position while stepping through history with Alt+Up/Down.
    history_position: Cell<Option<usize>>,
    /// Set while we change the entry text ourselves during history stepping.
    recalling: Cell<bool>,
    dismissing: Cell<bool>,
    /// The Claude CLI process for the current conversation, if started.
    claude: RefCell<Option<claude::Session>>,
    claude_polling: Cell<bool>,
    /// Keep the newest part of the conversation in view as it grows.
    chat_follow: Cell<bool>,
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
/// switched): redraw an open launcher with it.
pub fn config_changed() {
    let launcher = LAUNCHER.with(|slot| slot.borrow().clone());
    if let Some(launcher) = launcher {
        if launcher.window.is_visible() {
            launcher.refresh();
        }
    }
}

impl Launcher {
    fn build(app: &Application, config: Rc<RefCell<Config>>, config_error: Option<&str>) -> Rc<Self> {
        let window = Window::builder()
            .application(app)
            .title("Poppi Launcher")
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

        search_bar.append(&search_icon);
        search_bar.append(&entry);
        search_bar.append(&mode_chip);

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

        let state = LauncherState::new(&config.borrow());
        let launcher = Rc::new(Self {
            app: app.clone(),
            window,
            entry,
            mode_chip,
            divider,
            list_box,
            emoji_grid,
            results_container,
            scrolled,
            footer_status,
            footer_keys,
            config,
            state: RefCell::new(state),
            emoji_buttons: RefCell::new(Vec::new()),
            emoji_selected: Cell::new(0),
            history_position: Cell::new(None),
            recalling: Cell::new(false),
            dismissing: Cell::new(false),
            claude: RefCell::new(None),
            claude_polling: Cell::new(false),
            chat_follow: Cell::new(false),
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
        self.list_box.connect_row_selected(move |_, _| this.update_footer());

        // While the conversation is being followed, stay at its end as it grows.
        let this = self.clone();
        self.scrolled.vadjustment().connect_changed(move |adjustment| {
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

    // ---------- showing and hiding ----------

    fn window_width(&self) -> i32 {
        self.config.borrow().theme.width + 2 * CARD_MARGIN_SIDE
    }

    fn show(self: &Rc<Self>) {
        self.end_conversation();
        self.state.borrow_mut().reset_session();
        self.history_position.set(None);
        if self.entry.text().is_empty() {
            self.refresh();
        } else {
            // Emits `changed`, which refreshes.
            self.entry.set_text("");
        }

        let width = self.window_width();
        self.window.set_default_size(width, INITIAL_HEIGHT);
        self.window.set_size_request(width, -1);
        self.window.set_opacity(0.0);
        self.window.present();
        self.entry.grab_focus();
        self.fade_in();

        #[cfg(feature = "ui-test")]
        smoke::run(self);

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
        // Nobody is left to read the answer.
        self.end_conversation();
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
        let this = self.clone();
        WidgetExt::display(&self.window)
            .clipboard()
            .read_text_async(gio::Cancellable::NONE, move |result| {
                if let Ok(Some(text)) = result {
                    this.state.borrow_mut().clipboard.push(&text);
                }
            });
    }

    // ---------- results ----------

    fn refresh(self: &Rc<Self>) {
        let query = self.entry.text();
        let config = self.config.borrow().clone();
        let needs = self.state.borrow_mut().update(&query, &config);
        if needs.windows {
            self.load_windows();
        }
        if needs.files {
            self.load_file_index(&config);
        }
        if needs.rates {
            self.load_rates();
        }
        if needs.claude {
            // Start the CLI while the question is still being typed.
            self.warm_up_claude();
        }
        self.render();
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

    /// Walk the configured directories on a worker thread.
    fn load_file_index(self: &Rc<Self>, config: &Config) {
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
            let rates = gio::spawn_blocking(Rates::fetch).await.ok().and_then(Result::ok);
            this.state.borrow_mut().set_rates(rates);
            this.refresh_if_visible();
        });
    }

    fn refresh_if_visible(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.refresh();
        }
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
        self.mode_chip.set_text(mode.label());
        self.mode_chip.set_visible(mode != Mode::Empty);

        // Width available to rows, so wrapped text reports its real height.
        let list_width = card_width - 2;
        let natural_height = |widget: &gtk::Widget, width: i32| {
            widget.measure(gtk::Orientation::Vertical, width).1
        };
        // Height of the part that is visible without scrolling.
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
                visible_height = rows * natural_height(first.upcast_ref(), -1)
                    + (rows - 1) * EMOJI_SPACING
                    + 2 * EMOJI_MARGIN;
            }
            *self.emoji_buttons.borrow_mut() = buttons;
            self.update_emoji_selection();
        } else if !results.is_empty() {
            let mut first_rows_height = 0;
            {
                let state = self.state.borrow();
                let context = RowContext {
                    matcher: state.matcher(),
                    highlight: &highlight,
                    accent: &accent,
                };
                for (index, item) in results.iter().enumerate() {
                    let row = rows::list_row(item, &context);
                    self.list_box.append(&row);
                    if index < max_rows {
                        first_rows_height +=
                            natural_height(row.upcast_ref(), list_width - 2 * LIST_PADDING);
                    }
                }
            }
            visible_height = if answer_mode {
                natural_height(self.list_box.upcast_ref(), list_width).min(CHAT_MAX_HEIGHT)
            } else if results.len() <= max_rows {
                natural_height(self.list_box.upcast_ref(), list_width)
            } else {
                first_rows_height + 2 * LIST_PADDING
            };

            // Highlight what Enter will run: the first row, or in a
            // conversation the question being typed.
            let preferred = if answer_mode { results.len() as i32 - 1 } else { 0 };
            match self.list_box.row_at_index(preferred) {
                Some(row) if row.is_selectable() => self.list_box.select_row(Some(&row)),
                _ => self.list_box.unselect_all(),
            }
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

        // A non-resizable window takes exactly the size we ask for, so ask
        // for what the content needs now that the scroll area is capped.
        let width = self.window_width();
        let height = self
            .window
            .child()
            .map(|content| content.measure(gtk::Orientation::Vertical, width).1)
            .unwrap_or(INITIAL_HEIGHT);
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

    /// The footer: a status on the left, and on the right what the keys do
    /// for the item Enter would act on.
    fn update_footer(&self) {
        let selected = self.selected_item();
        let (count, mode, in_chat, busy) = {
            let state = self.state.borrow();
            (
                state.results.len(),
                state.mode,
                state.answer_mode(),
                state.chat_busy(),
            )
        };
        let query = self.entry.text();
        let typing_path = query.starts_with('/') || query.starts_with('~');

        let status = if in_chat {
            let model = self.config.borrow().claude.model.clone();
            if busy {
                format!("Claude · {} · answering…", model)
            } else {
                format!("Claude · {} · type a follow-up", model)
            }
        } else if query.is_empty() {
            EMPTY_TIPS.to_string()
        } else if count > 1 && !matches!(mode, Mode::Claude) {
            format!("{} results", count)
        } else {
            String::new()
        };
        self.footer_status.set_text(&status);

        while let Some(child) = self.footer_keys.first_child() {
            self.footer_keys.remove(&child);
        }
        for (key, action) in rows::hints(selected.as_ref(), typing_path, in_chat) {
            let key_label = Label::new(Some(key));
            key_label.add_css_class("key");
            self.footer_keys.append(&key_label);
            let action_label = Label::new(Some(action));
            action_label.set_margin_end(6);
            self.footer_keys.append(&action_label);
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
        Some(WidgetExt::display(&self.window).app_launch_context().upcast())
    }

    fn copy_text(&self, text: &str) -> Result<(), String> {
        crate::utils::copy_to_clipboard(text).or_else(|_| {
            // No clipboard tool: GTK's clipboard works while we keep running.
            WidgetExt::display(&self.window).clipboard().set_text(text);
            Ok(())
        })
    }
}
