mod alias;
mod app_launcher;
mod calculator;
mod claude;
mod config;
mod emoji_picker;
mod files;
mod history;
mod matching;
mod query;
mod rows;
mod search;
mod settings;
mod state;
mod style;
mod terminal;
mod ui;
mod usage;
mod utils;
mod window_switcher;

use gtk::prelude::*;
use gtk::{glib, Application};
use std::cell::RefCell;
use std::rc::Rc;

const APP_ID: &str = "com.poppi.launcher";

fn main() -> glib::ExitCode {
    let (config, config_error) = config::Config::load();
    if let Some(error) = &config_error {
        eprintln!("{}", error);
    }
    let config = Rc::new(RefCell::new(config));

    // Single instance: a second `poppi_launcher` process forwards its
    // activation to the running one and exits, which toggles the window.
    let app = Application::builder().application_id(APP_ID).build();

    let startup_config = config.clone();
    app.connect_startup(move |_| {
        // Both windows are dark; ask the theme for dark widgets too, so parts
        // we do not restyle (title bar, popovers, selections) match.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(true);
        }
        style::apply(&startup_config.borrow());
    });

    app.connect_activate(move |app| ui::activate(app, &config, config_error.as_deref()));

    app.run()
}
