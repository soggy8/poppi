use crate::config::Config;
use gtk::prelude::*;
use gtk::{Window, Box as GtkBox, Label, Entry, Button, SpinButton, CheckButton, Application};
use std::sync::{Arc, Mutex};

pub struct SettingsWindow;

impl SettingsWindow {
    pub fn open(app: &Application, config: Arc<Mutex<Config>>) {
        // Check if settings window already exists
        if let Some(existing_window) = app.windows().iter().find(|w| {
            w.title().as_ref().map(|s| s.as_str()) == Some("Poppi Launcher Settings")
        }) {
            existing_window.present();
            return;
        }

        let window = Window::builder()
            .application(app)
            .title("Poppi Launcher Settings")
            .default_width(600)
            .default_height(700)
            .resizable(true)
            .build();

        let main_box = GtkBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(20)
            .margin_start(20)
            .margin_end(20)
            .margin_top(20)
            .margin_bottom(20)
            .build();

        // Theme Section
        let theme_label = Label::new(Some("<b>Theme</b>"));
        theme_label.set_use_markup(true);
        theme_label.set_halign(gtk::Align::Start);
        main_box.append(&theme_label);

        // Background Color
        let bg_box = GtkBox::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        let bg_label = Label::new(Some("Background Color:"));
        bg_label.set_halign(gtk::Align::Start);
        let bg_entry = Entry::new();
        let config_clone = config.clone();
        let bg_entry_clone = bg_entry.clone();
        {
            let config_guard = config_clone.lock().unwrap();
            bg_entry.set_text(&config_guard.theme.background_color);
        }
        bg_box.append(&bg_label);
        bg_box.append(&bg_entry);
        main_box.append(&bg_box);

        // Text Color
        let text_box = GtkBox::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        let text_label = Label::new(Some("Text Color:"));
        text_label.set_halign(gtk::Align::Start);
        let text_entry = Entry::new();
        {
            let config_guard = config.lock().unwrap();
            text_entry.set_text(&config_guard.theme.text_color);
        }
        text_box.append(&text_label);
        text_box.append(&text_entry);
        main_box.append(&text_box);

        // Accent Color
        let accent_box = GtkBox::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        let accent_label = Label::new(Some("Accent Color:"));
        accent_label.set_halign(gtk::Align::Start);
        let accent_entry = Entry::new();
        {
            let config_guard = config.lock().unwrap();
            accent_entry.set_text(&config_guard.theme.accent_color);
        }
        accent_box.append(&accent_label);
        accent_box.append(&accent_entry);
        main_box.append(&accent_box);

        // Font Size
        let font_box = GtkBox::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        let font_label = Label::new(Some("Font Size:"));
        font_label.set_halign(gtk::Align::Start);
        let font_spin = SpinButton::with_range(8.0, 32.0, 1.0);
        {
            let config_guard = config.lock().unwrap();
            font_spin.set_value(config_guard.theme.font_size as f64);
        }
        font_box.append(&font_label);
        font_box.append(&font_spin);
        main_box.append(&font_box);

        // Border Radius
        let radius_box = GtkBox::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        let radius_label = Label::new(Some("Border Radius:"));
        radius_label.set_halign(gtk::Align::Start);
        let radius_spin = SpinButton::with_range(0.0, 50.0, 1.0);
        {
            let config_guard = config.lock().unwrap();
            radius_spin.set_value(config_guard.theme.border_radius);
        }
        radius_box.append(&radius_label);
        radius_box.append(&radius_spin);
        main_box.append(&radius_box);

        // Search Section
        let search_label = Label::new(Some("<b>Search</b>"));
        search_label.set_use_markup(true);
        search_label.set_halign(gtk::Align::Start);
        search_label.set_margin_top(20);
        main_box.append(&search_label);

        // YouTube Enabled
        let yt_check = CheckButton::with_label("Enable YouTube Search");
        {
            let config_guard = config.lock().unwrap();
            yt_check.set_active(config_guard.search.youtube_enabled);
        }
        main_box.append(&yt_check);

        // ChatGPT Enabled
        let gpt_check = CheckButton::with_label("Enable ChatGPT Search");
        {
            let config_guard = config.lock().unwrap();
            gpt_check.set_active(config_guard.search.chatgpt_enabled);
        }
        main_box.append(&gpt_check);

        // Calculator Section
        let calc_label = Label::new(Some("<b>Calculator</b>"));
        calc_label.set_use_markup(true);
        calc_label.set_halign(gtk::Align::Start);
        calc_label.set_margin_top(20);
        main_box.append(&calc_label);

        // Calculator Enabled
        let calc_check = CheckButton::with_label("Enable Calculator");
        {
            let config_guard = config.lock().unwrap();
            calc_check.set_active(config_guard.calculator.enabled);
        }
        main_box.append(&calc_check);

        // Buttons
        let button_box = GtkBox::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk::Align::End)
            .margin_top(20)
            .build();

        let save_button = Button::with_label("Save");
        let cancel_button = Button::with_label("Cancel");

        // Clone widgets for closure
        let bg_entry_clone = bg_entry.clone();
        let text_entry_clone = text_entry.clone();
        let accent_entry_clone = accent_entry.clone();
        let font_spin_clone = font_spin.clone();
        let radius_spin_clone = radius_spin.clone();
        let yt_check_clone = yt_check.clone();
        let gpt_check_clone = gpt_check.clone();
        let calc_check_clone = calc_check.clone();
        
        let window_clone = window.clone();
        let config_clone = config.clone();
        save_button.connect_clicked(move |_| {
            let mut config_guard = config_clone.lock().unwrap();
            
            // Update theme
            config_guard.theme.background_color = bg_entry_clone.text().to_string();
            config_guard.theme.text_color = text_entry_clone.text().to_string();
            config_guard.theme.accent_color = accent_entry_clone.text().to_string();
            config_guard.theme.font_size = font_spin_clone.value() as i32;
            config_guard.theme.border_radius = radius_spin_clone.value();
            
            // Update search
            config_guard.search.youtube_enabled = yt_check_clone.is_active();
            config_guard.search.chatgpt_enabled = gpt_check_clone.is_active();
            
            // Update calculator
            config_guard.calculator.enabled = calc_check_clone.is_active();
            
            // Save to file
            if let Err(e) = config_guard.save() {
                eprintln!("Error saving config: {}", e);
            }
            
            window_clone.close();
        });

        let window_clone2 = window.clone();
        cancel_button.connect_clicked(move |_| {
            window_clone2.close();
        });

        button_box.append(&cancel_button);
        button_box.append(&save_button);
        main_box.append(&button_box);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_child(Some(&main_box));
        scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);

        window.set_child(Some(&scrolled));
        window.present();
    }
}

