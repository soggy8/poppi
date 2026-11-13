mod actions;
mod ai;
mod alias;
mod app_launcher;
mod calculator;
mod clipboard;
mod config;
mod context;
mod conversions;
mod emoji_picker;
mod files;
mod history;
mod markdown;
mod matching;
mod model_editor;
mod query;
mod reminders;
mod rows;
mod scripts;
mod search;
mod settings;
mod setup;
mod shortcut;
mod state;
mod style;
mod system;
mod terminal;
mod ui;
mod units;
mod usage;
mod utils;
mod voice;
mod wake;
mod window_switcher;

use config::Config;
use gtk::prelude::*;
use gtk::{gio, glib, Application};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const APP_ID: &str = "com.poppi.launcher";

const HELP: &str = "\
Poppi Launcher

Usage: poppi_launcher [OPTION]

With no option, shows the launcher, or toggles it if it is already running.

Options:
  --daemon       Start in the background without opening the window
  --quit         Stop the instance running in the background
  -V, --version  Print the version
  -h, --help     Print this help";

/// Ask a running instance to exit. Works from a second process because
/// GApplication exports the primary instance's actions over D-Bus.
fn quit_running_instance() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    if let Err(e) = app.register(gio::Cancellable::NONE) {
        eprintln!("Could not reach the session bus: {}", e);
        return glib::ExitCode::FAILURE;
    }
    if app.is_remote() {
        app.activate_action("quit", None);
        println!("Poppi Launcher stopped.");
    } else {
        println!("Poppi Launcher is not running.");
    }
    glib::ExitCode::SUCCESS
}

/// Reload the config whenever its file changes on disk, so hand edits apply
/// without a restart. A file that does not parse is ignored (and reported).
fn watch_config(config: &Rc<RefCell<Config>>) -> Option<gio::FileMonitor> {
    let file = gio::File::for_path(Config::config_path());
    let monitor = file
        .monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .ok()?;
    let config = config.clone();
    monitor.connect_changed(move |_, _, _, event| {
        use gio::FileMonitorEvent::{ChangesDoneHint, Created, Renamed};
        if !matches!(event, ChangesDoneHint | Created | Renamed) {
            return;
        }
        match Config::load_existing() {
            Ok(loaded) => {
                // Our own saves arrive here too; they change nothing.
                if *config.borrow() != loaded {
                    *config.borrow_mut() = loaded;
                    style::apply(&config.borrow());
                    ui::config_changed();
                }
            }
            Err(error) => eprintln!("{}", error),
        }
    });
    Some(monitor)
}

fn main() -> glib::ExitCode {
    let daemon = match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("poppi_launcher {}", env!("CARGO_PKG_VERSION"));
            return glib::ExitCode::SUCCESS;
        }
        Some("--help" | "-h") => {
            println!("{}", HELP);
            return glib::ExitCode::SUCCESS;
        }
        Some("--quit") => return quit_running_instance(),
        Some("--daemon") => true,
        Some(other) => {
            eprintln!("Unknown option: {}\n\n{}", other, HELP);
            return glib::ExitCode::FAILURE;
        }
        None => false,
    };

    // No config file yet: the first time Poppi is opened, show the setup.
    let first_run = Cell::new(!Config::config_path().exists());
    let (config, config_error) = Config::load();
    if let Some(error) = &config_error {
        eprintln!("{}", error);
    }
    let config = Rc::new(RefCell::new(config));

    // Single instance: a second `poppi_launcher` process forwards its
    // activation to the running one and exits, which toggles the window.
    let app = Application::builder().application_id(APP_ID).build();
    if daemon {
        if let Err(e) = app.register(gio::Cancellable::NONE) {
            eprintln!("Could not reach the session bus: {}", e);
            return glib::ExitCode::FAILURE;
        }
        if app.is_remote() {
            return glib::ExitCode::SUCCESS;
        }
    }

    let startup_config = config.clone();
    app.connect_startup(move |app| {
        style::apply(&startup_config.borrow());

        let quit = gio::SimpleAction::new("quit", None);
        let app_for_quit = app.clone();
        quit.connect_activate(move |_, _| app_for_quit.quit());
        app.add_action(&quit);

        // Follow the desktop when it switches between light and dark.
        let config_for_scheme = startup_config.clone();
        style::watch_system_scheme(move || {
            style::apply(&config_for_scheme.borrow());
            ui::config_changed();
        });

        // The monitor has to stay alive for as long as the application.
        if let Some(monitor) = watch_config(&startup_config) {
            std::mem::forget(monitor);
        }
        if daemon {
            std::mem::forget(app.hold());
        }
    });

    let skip_initial_show = Cell::new(daemon);
    app.connect_activate(move |app| {
        if skip_initial_show.replace(false) {
            ui::warm_up(app, &config, config_error.as_deref());
        } else if first_run.replace(false) {
            setup::open(app, config.clone());
        } else {
            ui::activate(app, &config, config_error.as_deref());
        }
    });

    // Arguments were handled above; GApplication gets none.
    app.run_with_args::<&str>(&[])
}
