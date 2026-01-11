use crate::app_launcher::{App, AppLauncher};
use crate::calculator::Calculator;
use crate::config::Config;
use crate::emoji_picker::{Emoji, EmojiPicker};
use crate::search::WebSearch;
use crate::terminal::Terminal;
use gtk::prelude::*;
use gtk::glib;
use gtk::gdk;
use gtk::{Application, Entry, ListBox, ListBoxRow, Box as GtkBox, Label, Window, ScrolledWindow, EventControllerKey};
use std::sync::{Arc, Mutex};
use std::io::Write;
use std::thread;

pub struct LauncherState {
    pub app_launcher: AppLauncher,
    pub emoji_picker: EmojiPicker,
    pub current_mode: Mode,
    pub results: Vec<ResultItem>,
    pub displayed_results: Vec<ResultItem>, // Results currently shown in UI
    pub selected_index: usize, // Currently selected item index
}

#[derive(Clone, Debug)]
pub enum Mode {
    Apps,
    Calculator,
    Emoji,
    Terminal,
    Search,
}

#[derive(Clone, Debug)]
pub enum ResultItem {
    App(App),
    CalculatorResult(String),
    Emoji(Emoji),
    TerminalCommand(String),
    SearchQuery { engine: String, query: String },
}

impl LauncherState {
    pub fn new() -> Self {
        Self {
            app_launcher: AppLauncher::empty(), // Start with empty launcher for lazy loading
            emoji_picker: EmojiPicker::new(),
            current_mode: Mode::Apps,
            results: Vec::new(),
            displayed_results: Vec::new(),
            selected_index: 0,
        }
    }

    pub fn set_app_launcher(&mut self, app_launcher: AppLauncher) {
        self.app_launcher = app_launcher;
    }

    pub fn is_app_launcher_loaded(&self) -> bool {
        !self.app_launcher.apps().is_empty()
    }

    pub fn update_query(&mut self, query: &str) {
        let query = query.trim();
        
        // Determine mode based on query
        if query.is_empty() {
            self.current_mode = Mode::Apps;
            self.results = self.app_launcher
                .search("")
                .into_iter()
                .map(|(app, _)| ResultItem::App((*app).clone()))
                .collect();
            return;
        }

        // Check for search prefixes
        if query.starts_with("yt ") || query.starts_with("youtube ") {
            self.current_mode = Mode::Search;
            let search_query = query.strip_prefix("yt ").unwrap_or(query.strip_prefix("youtube ").unwrap_or(query));
            self.results = vec![ResultItem::SearchQuery {
                engine: "youtube".to_string(),
                query: search_query.to_string(),
            }];
            return;
        }

        if query.starts_with("gpt ") || query.starts_with("chatgpt ") {
            self.current_mode = Mode::Search;
            let search_query = query.strip_prefix("gpt ").unwrap_or(query.strip_prefix("chatgpt ").unwrap_or(query));
            self.results = vec![ResultItem::SearchQuery {
                engine: "chatgpt".to_string(),
                query: search_query.to_string(),
            }];
            return;
        }

        if query.starts_with("google ") {
            self.current_mode = Mode::Search;
            let search_query = query.strip_prefix("google ").unwrap_or(query);
            self.results = vec![ResultItem::SearchQuery {
                engine: "google".to_string(),
                query: search_query.to_string(),
            }];
            return;
        }

        // Check for emoji mode
        if query.starts_with("emoji ") || query.starts_with(":") {
            self.current_mode = Mode::Emoji;
            let emoji_query = query.strip_prefix("emoji ").unwrap_or(query.strip_prefix(":").unwrap_or(query));
            self.results = self.emoji_picker
                .search(emoji_query)
                .into_iter()
                .map(|(emoji, _)| ResultItem::Emoji((*emoji).clone()))
                .collect();
            return;
        }

        // Check for calculator
        if Calculator::is_calculation(query) {
            self.current_mode = Mode::Calculator;
            match Calculator::evaluate(query) {
                Ok(result) => {
                    self.results = vec![ResultItem::CalculatorResult(
                        Calculator::format_result(result)
                    )];
                }
                Err(_) => {
                    // If calculation fails, fall back to app search
                    self.current_mode = Mode::Apps;
                    self.results = self.app_launcher
                        .search(query)
                        .into_iter()
                        .map(|(app, _)| ResultItem::App((*app).clone()))
                        .collect();
                }
            }
            return;
        }

        // Check for terminal commands
        if Terminal::is_terminal_command(query) {
            self.current_mode = Mode::Terminal;
            self.results = vec![ResultItem::TerminalCommand(query.to_string())];
            return;
        }

        // Default: app search
        self.current_mode = Mode::Apps;
        self.results = self.app_launcher
            .search(query)
            .into_iter()
            .map(|(app, _)| ResultItem::App((*app).clone()))
            .collect();
    }

    pub fn execute_selected(&self, index: usize) -> Result<(), Box<dyn std::error::Error>> {
        // Use displayed_results instead of results
        if index >= self.displayed_results.len() {
            return Err("Index out of bounds".into());
        }

        match &self.displayed_results[index] {
            ResultItem::App(app) => {
                self.app_launcher.launch(app)?;
            }
            ResultItem::CalculatorResult(result) => {
                // Copy result to clipboard
                use std::process::{Command, Stdio};
                let mut child = Command::new("xclip")
                    .arg("-selection")
                    .arg("clipboard")
                    .stdin(Stdio::piped())
                    .spawn()?;
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(result.as_bytes())?;
                }
            }
            ResultItem::Emoji(emoji) => {
                EmojiPicker::insert_emoji(&emoji.emoji)?;
            }
            ResultItem::TerminalCommand(cmd) => {
                Terminal::execute_command(cmd)?;
            }
            ResultItem::SearchQuery { engine, query } => {
                match engine.as_str() {
                    "youtube" => WebSearch::search_youtube(query)?,
                    "chatgpt" => WebSearch::search_chatgpt(query)?,
                    "google" => WebSearch::search_google(query)?,
                    _ => {}
                }
            }
        }

        Ok(())
    }
}

pub fn build_ui(app: &Application, config: Config) {
    let state = Arc::new(Mutex::new(LauncherState::new()));

    // Fixed width, dynamic height based on results
    let window_width = 600;
    let entry_height = 60; // Height for just the search bar
    let row_height = 50; // Approximate height per result row
    let max_results = 5;

    // Create main window - fixed width
    let window = Window::builder()
        .application(app)
        .title("Poppi Launcher")
        .default_width(window_width)
        .default_height(entry_height)
        .decorated(false)
        .resizable(false)
        .build();

    // Main container
    let main_box = GtkBox::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(10)
        .margin_end(10)
        .build();

    // Search entry
    let entry = Entry::builder()
        .placeholder_text("Search apps, calculate, emoji, or run commands...")
        .build();

    // Results list
    let list_box = ListBox::new();
    let scrolled = ScrolledWindow::builder()
        .child(&list_box)
        .min_content_width(window_width - 40)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .build();
    
    // Initially hide the results list
    scrolled.set_visible(false);

    // Apply CSS styling
    let css = format!(
        r#"
        window {{
            background-color: {};
            border-radius: {}px;
        }}
        entry {{
            background-color: rgba(255, 255, 255, 0.1);
            border: 1px solid rgba(255, 255, 255, 0.2);
            border-radius: 8px;
            padding: 10px;
            font-size: {}pt;
            color: {};
        }}
        list {{
            background-color: transparent;
        }}
        row {{
            background-color: rgba(255, 255, 255, 0.05);
            border-radius: 4px;
            padding: 10px;
            margin: 2px;
        }}
        row:selected {{
            background-color: {};
        }}
        label {{
            color: {};
            font-size: {}pt;
        }}
        "#,
        config.theme.background_color,
        config.theme.border_radius,
        config.theme.font_size,
        config.theme.text_color,
        config.theme.accent_color,
        config.theme.text_color,
        config.theme.font_size,
    );

    let provider = gtk::CssProvider::new();
    provider.load_from_data(&css);
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Update results when entry changes and adjust window height
    let state_clone = state.clone();
    let list_box_clone = list_box.clone();
    let scrolled_clone = scrolled.clone();
    let window_clone = window.clone();
    let row_height_clone = row_height;
    let entry_height_clone = entry_height;
    let window_width_clone = window_width;
    let max_results_clone = max_results;
    entry.connect_changed(move |entry| {
        let query = entry.text();
        let mut state = state_clone.lock().unwrap();
        state.update_query(&query);
        
        // Limit results to max_results (5) and store in displayed_results
        // Build displayed_results in one step to avoid borrowing issues
        let displayed: Vec<_> = state.results.iter().take(max_results_clone).cloned().collect();
        state.displayed_results = displayed.clone();
        state.selected_index = 0; // Reset selection when query changes
        update_results_list(&list_box_clone, &displayed);
        
        // Clear selection when results change
        list_box_clone.unselect_all();
        
        // Calculate height based on number of results
        let num_results = state.displayed_results.len();
        if num_results > 0 {
            // Show results and adjust height
            scrolled_clone.set_visible(true);
            let total_height = entry_height_clone + (num_results as i32 * row_height_clone) + 20; // +20 for margins/spacing
            window_clone.set_default_size(window_width_clone, total_height);
            window_clone.set_size_request(window_width_clone, total_height);
        } else {
            // Hide results, just show search bar
            scrolled_clone.set_visible(false);
            window_clone.set_default_size(window_width_clone, entry_height_clone);
            window_clone.set_size_request(window_width_clone, entry_height_clone);
        }
    });

    // Handle Enter key - execute selected item
    let state_clone = state.clone();
    let window_clone = window.clone();
    let list_box_clone = list_box.clone();
    entry.connect_activate(move |_entry| {
        let state = state_clone.lock().unwrap();
        if !state.displayed_results.is_empty() {
            // Get selected row or use first item
            let selected_index = if let Some(selected_row) = list_box_clone.selected_row() {
                selected_row.index() as usize
            } else {
                0
            };
            if let Err(e) = state.execute_selected(selected_index) {
                eprintln!("Error executing: {}", e);
            }
            window_clone.close();
        }
    });

    // Handle keyboard navigation (Escape, Arrow keys) on the entry
    let entry_key_controller = EventControllerKey::new();
    let window_clone = window.clone();
    let list_box_clone = list_box.clone();
    let state_clone = state.clone();
    entry_key_controller.connect_key_pressed(move |_, keyval, _, _| {
        match keyval {
            gdk::Key::Escape => {
                // Close window
                window_clone.close();
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::KP_Down => {
                // Move selection down
                let state = state_clone.lock().unwrap();
                if !state.displayed_results.is_empty() {
                    if let Some(selected_row) = list_box_clone.selected_row() {
                        let current_index = selected_row.index() as usize;
                        let next_index = (current_index + 1).min(state.displayed_results.len() - 1);
                        if let Some(next_row) = list_box_clone.row_at_index(next_index as i32) {
                            list_box_clone.select_row(Some(&next_row));
                        }
                    } else {
                        // Select first row if none selected
                        if let Some(first_row) = list_box_clone.row_at_index(0) {
                            list_box_clone.select_row(Some(&first_row));
                        }
                    }
                }
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::KP_Up => {
                // Move selection up
                let state = state_clone.lock().unwrap();
                if !state.displayed_results.is_empty() {
                    if let Some(selected_row) = list_box_clone.selected_row() {
                        let current_index = selected_row.index() as usize;
                        if current_index > 0 {
                            let prev_index = current_index - 1;
                            if let Some(prev_row) = list_box_clone.row_at_index(prev_index as i32) {
                                list_box_clone.select_row(Some(&prev_row));
                            }
                        }
                    } else {
                        // Select last row if none selected
                        let last_index = state.displayed_results.len().saturating_sub(1);
                        if let Some(last_row) = list_box_clone.row_at_index(last_index as i32) {
                            list_box_clone.select_row(Some(&last_row));
                        }
                    }
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    entry.add_controller(entry_key_controller);

    // Handle list box row activation
    let state_clone = state.clone();
    let window_clone = window.clone();
    list_box.connect_row_activated(move |_, row| {
        let index = row.index();
        let state = state_clone.lock().unwrap();
        if let Err(e) = state.execute_selected(index as usize) {
            eprintln!("Error executing: {}", e);
        }
        window_clone.close();
    });

    // Initial state - compact, no results
    {
        let mut state = state.lock().unwrap();
        state.update_query("");
        // Don't populate results initially
    }

    // Assemble UI
    main_box.append(&entry);
    main_box.append(&scrolled);
    window.set_child(Some(&main_box));

    // Show window immediately
    window.present();
    entry.grab_focus();

    // Load apps in background thread after window appears (lazy loading)
    let state_clone = state.clone();
    thread::spawn(move || {
        // Load apps in background
        if let Ok(app_launcher) = AppLauncher::new() {
            // Update UI from background thread using glib MainContext
            glib::MainContext::default().invoke(move || {
                let mut state = state_clone.lock().unwrap();
                state.set_app_launcher(app_launcher);
                // Apps are now loaded - next search will use them
            });
        }
    });
}

fn update_results_list(list_box: &ListBox, results: &[ResultItem]) {
    // Clear existing rows
    while let Some(row) = list_box.row_at_index(0) {
        list_box.remove(&row);
    }

    // Add new rows (already limited to max_results in caller)
    for result in results.iter() {
        let row = ListBoxRow::new();
        let row_box = GtkBox::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .margin_start(10)
            .margin_end(10)
            .margin_top(5)
            .margin_bottom(5)
            .build();

        let label = match result {
            ResultItem::App(app) => {
                Label::new(Some(&format!("{}", app.name)))
            }
            ResultItem::CalculatorResult(result) => {
                Label::new(Some(&format!("= {}", result)))
            }
            ResultItem::Emoji(emoji) => {
                Label::new(Some(&format!("{} {}", emoji.emoji, emoji.name)))
            }
            ResultItem::TerminalCommand(cmd) => {
                Label::new(Some(&format!("▶ {}", cmd)))
            }
            ResultItem::SearchQuery { engine, query } => {
                Label::new(Some(&format!("🌐 Search {}: {}", engine, query)))
            }
        };

        label.set_xalign(0.0);
        row_box.append(&label);
        row.set_child(Some(&row_box));
        list_box.append(&row);
    }
}
