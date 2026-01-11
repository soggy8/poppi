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

pub struct LauncherState {
    pub app_launcher: AppLauncher,
    pub emoji_picker: EmojiPicker,
    pub current_mode: Mode,
    pub results: Vec<ResultItem>,
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
            app_launcher: AppLauncher::default(),
            emoji_picker: EmojiPicker::new(),
            current_mode: Mode::Apps,
            results: Vec::new(),
        }
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
        if index >= self.results.len() {
            return Err("Index out of bounds".into());
        }

        match &self.results[index] {
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

    // Create main window
    let window = Window::builder()
        .application(app)
        .title("Poppi Launcher")
        .default_width(config.theme.width)
        .default_height(config.theme.height)
        .decorated(false)
        .resizable(false)
        .build();

    // Main container
    let main_box = GtkBox::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(20)
        .margin_bottom(20)
        .margin_start(20)
        .margin_end(20)
        .build();

    // Search entry
    let entry = Entry::builder()
        .placeholder_text("Search apps, calculate, emoji, or run commands...")
        .build();

    // Results list
    let list_box = ListBox::new();
    let scrolled = ScrolledWindow::builder()
        .child(&list_box)
        .min_content_width(config.theme.width - 40)
        .max_content_height(config.theme.height - 100)
        .build();

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

    // Update results when entry changes
    let state_clone = state.clone();
    let list_box_clone = list_box.clone();
    entry.connect_changed(move |entry| {
        let query = entry.text();
        let mut state = state_clone.lock().unwrap();
        state.update_query(&query);
        update_results_list(&list_box_clone, &state.results);
    });

    // Handle Enter key
    let state_clone = state.clone();
    let window_clone = window.clone();
    entry.connect_activate(move |_entry| {
        let state = state_clone.lock().unwrap();
        if !state.results.is_empty() {
            if let Err(e) = state.execute_selected(0) {
                eprintln!("Error executing: {}", e);
            }
            window_clone.close();
        }
    });

    // Handle Escape key
    let key_controller = EventControllerKey::new();
    let window_clone = window.clone();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::Escape {
            window_clone.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

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

    // Initial search
    {
        let mut state = state.lock().unwrap();
        state.update_query("");
        update_results_list(&list_box, &state.results);
    }

    // Assemble UI
    main_box.append(&entry);
    main_box.append(&scrolled);
    window.set_child(Some(&main_box));

    // Show window (it will be centered by default)
    window.present();
    entry.grab_focus();
}

fn update_results_list(list_box: &ListBox, results: &[ResultItem]) {
    // Clear existing rows
    while let Some(row) = list_box.row_at_index(0) {
        list_box.remove(&row);
    }

    // Add new rows
    for result in results.iter().take(10) {
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

