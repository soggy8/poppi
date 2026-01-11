mod app_launcher;
mod calculator;
mod config;
mod emoji_picker;
mod search;
mod terminal;
mod ui;
mod utils;

use gtk::prelude::*;
use gtk::{glib, gio, Application};

const APP_ID: &str = "com.poppi.launcher";

fn main() -> glib::ExitCode {
    // Load configuration
    let config = config::Config::load().unwrap_or_default();

    // Create application - NON_UNIQUE for reliable startup
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(move |app| {
        // Check if window already exists
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        
        // Build new UI
        ui::build_ui(app, config.clone());
    });

    // Run the application
    app.run()
}
