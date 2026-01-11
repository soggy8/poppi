use crate::app_launcher::{App, AppLauncher};
use crate::calculator::Calculator;
use crate::config::Config;
use crate::emoji_picker::{Emoji, EmojiPicker};
use crate::search::WebSearch;
use crate::terminal::Terminal;
use gtk::prelude::*;
use gtk::glib;
use gtk::gdk;
use gtk::{Application, Entry, ListBox, ListBoxRow, Box as GtkBox, Label, Window, ScrolledWindow, EventControllerKey, Grid, Button};
use gtk4_layer_shell::{Layer, Edge, LayerShell};
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
        let app_results: Vec<ResultItem> = self.app_launcher
            .search(query)
            .into_iter()
            .map(|(app, _)| ResultItem::App((*app).clone()))
            .collect();
        
        // If no app results found, add search options as fallback
        if app_results.is_empty() && !query.is_empty() {
            self.results = vec![
                ResultItem::SearchQuery {
                    engine: "youtube".to_string(),
                    query: query.to_string(),
                },
                ResultItem::SearchQuery {
                    engine: "google".to_string(),
                    query: query.to_string(),
                },
                ResultItem::SearchQuery {
                    engine: "chatgpt".to_string(),
                    query: query.to_string(),
                },
            ];
        } else {
            self.results = app_results;
        }
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

    // Initialize layer shell for GNOME/Wayland positioning
    window.init_layer_shell();
    window.set_layer(Layer::Overlay); // Above everything
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::Exclusive);
    
    // Anchor to top edge, centered horizontally
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, false);
    window.set_anchor(Edge::Right, false);
    window.set_anchor(Edge::Bottom, false);
    
    // Set margin from top (150px from top of screen)
    window.set_margin(Edge::Top, 150);

    // Main container
    let main_box = GtkBox::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .margin_top(3)
        .margin_bottom(3)
        .margin_start(3)
        .margin_end(3)
        .build();

    // Search entry
    let entry = Entry::builder()
        .placeholder_text("Search apps, calculate, emoji, or run commands...")
        .build();

    // Results list
    let list_box = ListBox::new();
    // Grid for emoji display
    let emoji_grid = Grid::new();
    emoji_grid.set_row_spacing(8);
    emoji_grid.set_column_spacing(8);
    emoji_grid.set_margin_start(10);
    emoji_grid.set_margin_end(10);
    emoji_grid.set_margin_top(10);
    emoji_grid.set_margin_bottom(10);
    
    // Container box that can hold either list_box or emoji_grid
    let results_container = GtkBox::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    results_container.append(&list_box);
    
    let scrolled = ScrolledWindow::builder()
        .child(&results_container)
        .min_content_width(window_width - 40)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .build();
    
    // Initially hide the results list
    scrolled.set_visible(false);

    // Apply CSS styling with animations
    let css = format!(
        r#"
        @keyframes fadeIn {{
            from {{
                opacity: 0;
            }}
            to {{
                opacity: 1;
            }}
        }}
        
        @keyframes slideDown {{
            from {{
                opacity: 0;
                transform: translateY(-10px);
            }}
            to {{
                opacity: 1;
                transform: translateY(0);
            }}
        }}
        
        window {{
            background-color: {};
            border-radius: 0px;
            border: 0.5px solid rgba(46, 46, 46, 0.2);
            box-shadow: 0 4px 20px rgba(0, 0, 0, 0.8);
            animation: fadeIn 0.15s ease-out;
        }}
        
        decoration {{
            border: none;
        }}
        
        entry {{
            background-color: rgba(20, 20, 20, 0.9);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 0px;
            padding: 12px;
            font-size: {}pt;
            color: {};
            transition: border-color 0.2s ease, background-color 0.2s ease;
        }}
        
        entry:focus {{
            border-color: {};
            background-color: rgba(15, 15, 15, 0.95);
            box-shadow: inset 0 0 0 1px {};
        }}
        
        list {{
            background-color: transparent;
        }}
        
        row {{
            background-color: rgba(20, 20, 20, 0.6);
            border: none;
            border-top: 1px solid rgba(255, 255, 255, 0.05);
            border-bottom: 1px solid rgba(255, 255, 255, 0.05);
            border-radius: 0px;
            padding: 12px;
            margin: 0px;
            transition: background-color 0.15s ease, transform 0.1s ease;
            animation: slideDown 0.2s ease-out backwards;
        }}
        
        row:nth-child(1) {{
            animation-delay: 0.02s;
        }}
        
        row:nth-child(2) {{
            animation-delay: 0.04s;
        }}
        
        row:nth-child(3) {{
            animation-delay: 0.06s;
        }}
        
        row:nth-child(4) {{
            animation-delay: 0.08s;
        }}
        
        row:nth-child(5) {{
            animation-delay: 0.10s;
        }}
        
        row:hover {{
            background-color: rgba(30, 30, 30, 0.8);
            transform: translateX(2px);
        }}
        
        row:selected {{
            background-color: rgba(40, 40, 40, 0.95);
            border-top: 1px solid rgba(255, 255, 255, 0.15);
            border-bottom: 1px solid rgba(255, 255, 255, 0.15);
        }}
        
        label {{
            color: {};
            font-size: {}pt;
        }}
        
        button.emoji-button {{
            background-color: transparent;
            border: none;
            border-radius: 0px;
            padding: 8px;
            font-size: 24px;
            transition: background-color 0.15s ease;
        }}
        
        button.emoji-button:hover {{
            background-color: rgba(40, 40, 40, 0.8);
        }}
        
        button.emoji-button.selected {{
            background-color: rgba(40, 40, 40, 0.95);
            border: 1px solid rgba(255, 255, 255, 0.15);
        }}
        "#,
        config.theme.background_color,
        config.theme.font_size,
        config.theme.text_color,
        config.theme.accent_color,
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
    let emoji_grid_clone = emoji_grid.clone();
    let results_container_clone = results_container.clone();
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
        
        // Check if emoji mode
        let is_emoji_mode = matches!(state.current_mode, Mode::Emoji);
        let limit = if is_emoji_mode { 30 } else { max_results_clone };
        
        // Limit results to max_results (5) and store in displayed_results
        // Build displayed_results in one step to avoid borrowing issues
        let displayed: Vec<_> = state.results.iter().take(limit).cloned().collect();
        state.displayed_results = displayed.clone();
        state.selected_index = 0; // Reset selection when query changes
        let selected_idx = state.selected_index;
        update_results_list(&list_box_clone, &emoji_grid_clone, &results_container_clone, &displayed, is_emoji_mode, &window_clone, selected_idx);
        
        // Clear selection when results change
        list_box_clone.unselect_all();
        
        // Calculate height based on number of results
        let num_results = state.displayed_results.len();
        if num_results > 0 {
            // Show results and adjust height
            scrolled_clone.set_visible(true);
            let total_height = if is_emoji_mode {
                // For emoji grid, calculate height based on grid rows (assuming 8 columns)
                let rows = (num_results + 7) / 8; // Round up division
                entry_height_clone + (rows as i32 * 50) + 40
            } else {
                entry_height_clone + (num_results as i32 * row_height_clone) + 20
            };
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
    let emoji_grid_clone = emoji_grid.clone();
    let results_container_clone = results_container.clone();
    entry.connect_activate(move |_entry| {
        let state = state_clone.lock().unwrap();
        if !state.displayed_results.is_empty() {
            // Check if emoji mode
            let is_emoji_mode = matches!(state.current_mode, Mode::Emoji);
            let selected_index = if is_emoji_mode {
                state.selected_index
            } else {
                // Get selected row or use first item
                if let Some(selected_row) = list_box_clone.selected_row() {
                    selected_row.index() as usize
                } else {
                    0
                }
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
    let emoji_grid_clone = emoji_grid.clone();
    let results_container_clone = results_container.clone();
    let state_clone = state.clone();
    entry_key_controller.connect_key_pressed(move |_, keyval, _, _| {
        match keyval {
            gdk::Key::Escape => {
                // Close window
                window_clone.close();
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::KP_Down => {
                let mut state = state_clone.lock().unwrap();
                let is_emoji_mode = matches!(state.current_mode, Mode::Emoji);
                if !state.displayed_results.is_empty() {
                    if is_emoji_mode {
                        // Grid navigation: move down (8 columns)
                        let columns = 8;
                        let max_index = state.displayed_results.len().saturating_sub(1);
                        let new_index = (state.selected_index + columns).min(max_index);
                        state.selected_index = new_index;
                        // Update UI
                        let displayed: Vec<_> = state.displayed_results.clone();
                        let selected_idx = state.selected_index;
                        drop(state);
                        update_results_list(&list_box_clone, &emoji_grid_clone, &results_container_clone, &displayed, is_emoji_mode, &window_clone, selected_idx);
                    } else {
                        // List navigation
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
                }
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::KP_Up => {
                let mut state = state_clone.lock().unwrap();
                let is_emoji_mode = matches!(state.current_mode, Mode::Emoji);
                if !state.displayed_results.is_empty() {
                    if is_emoji_mode {
                        // Grid navigation: move up (8 columns)
                        let columns = 8;
                        let new_index = state.selected_index.saturating_sub(columns);
                        state.selected_index = new_index;
                        // Update UI
                        let displayed: Vec<_> = state.displayed_results.clone();
                        let selected_idx = state.selected_index;
                        drop(state);
                        update_results_list(&list_box_clone, &emoji_grid_clone, &results_container_clone, &displayed, is_emoji_mode, &window_clone, selected_idx);
                    } else {
                        // List navigation
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
                }
                glib::Propagation::Stop
            }
            gdk::Key::Right | gdk::Key::KP_Right => {
                let mut state = state_clone.lock().unwrap();
                let is_emoji_mode = matches!(state.current_mode, Mode::Emoji);
                if is_emoji_mode && !state.displayed_results.is_empty() {
                    // Grid navigation: move right
                    let max_index = state.displayed_results.len().saturating_sub(1);
                    let new_index = (state.selected_index + 1).min(max_index);
                    state.selected_index = new_index;
                    // Update UI
                    let displayed: Vec<_> = state.displayed_results.clone();
                    let selected_idx = state.selected_index;
                    drop(state);
                    update_results_list(&list_box_clone, &emoji_grid_clone, &results_container_clone, &displayed, is_emoji_mode, &window_clone, selected_idx);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            gdk::Key::Left | gdk::Key::KP_Left => {
                let mut state = state_clone.lock().unwrap();
                let is_emoji_mode = matches!(state.current_mode, Mode::Emoji);
                if is_emoji_mode && !state.displayed_results.is_empty() {
                    // Grid navigation: move left
                    let new_index = state.selected_index.saturating_sub(1);
                    state.selected_index = new_index;
                    // Update UI
                    let displayed: Vec<_> = state.displayed_results.clone();
                    let selected_idx = state.selected_index;
                    drop(state);
                    update_results_list(&list_box_clone, &emoji_grid_clone, &results_container_clone, &displayed, is_emoji_mode, &window_clone, selected_idx);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
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

    // Show window with fade-in animation
    window.set_opacity(0.0);
    window.present();
    entry.grab_focus();
    
    // Animate window fade-in
    let window_clone = window.clone();
    let mut opacity = 0.0;
    let step = 0.05;
    glib::timeout_add_local(std::time::Duration::from_millis(10), move || {
        opacity += step;
        if opacity >= 1.0 {
            window_clone.set_opacity(1.0);
            return glib::ControlFlow::Break;
        }
        window_clone.set_opacity(opacity);
        glib::ControlFlow::Continue
    });

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

fn update_results_list(list_box: &ListBox, emoji_grid: &Grid, results_container: &GtkBox, results: &[ResultItem], is_emoji_mode: bool, window: &Window, selected_index: usize) {
    // Clear existing content
    while let Some(child) = results_container.first_child() {
        results_container.remove(&child);
    }
    
    // Remove all children from grid
    while let Some(child) = emoji_grid.first_child() {
        emoji_grid.remove(&child);
    }
    
    if is_emoji_mode {
        // Display emojis in a grid
        let columns = 8;
        let mut row = 0;
        let mut col = 0;
        
        for result in results.iter() {
            if let ResultItem::Emoji(emoji) = result {
                // Create button with emoji
                let button = Button::builder()
                    .label(&emoji.emoji)
                    .build();
                button.add_css_class("emoji-button");
                button.set_hexpand(true);
                button.set_vexpand(true);
                
                // Connect click handler
                let emoji_emoji = emoji.emoji.clone();
                let window_clone = window.clone();
                button.connect_clicked(move |_| {
                    let _ = crate::emoji_picker::EmojiPicker::insert_emoji(&emoji_emoji);
                    window_clone.close();
                });
                
                emoji_grid.attach(&button, col, row, 1, 1);
                
                col += 1;
                if col >= columns {
                    col = 0;
                    row += 1;
                }
            }
        }
        
        results_container.append(emoji_grid);
    } else {
        // Clear existing rows from list_box
        while let Some(row) = list_box.row_at_index(0) {
            list_box.remove(&row);
        }
        
        // Display regular results in list
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
                ResultItem::Emoji(_) => {
                    // Should not happen in non-emoji mode, but handle it
                    Label::new(Some(""))
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
        
        results_container.append(list_box);
    }
}
