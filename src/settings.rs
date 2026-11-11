use crate::config::Config;
use crate::search::Engine;
use crate::style;
use gtk::prelude::*;
use gtk::{
    Application, Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, Orientation,
    ScrolledWindow, SpinButton, Window,
};
use std::cell::RefCell;
use std::rc::Rc;

const WINDOW_NAME: &str = "poppi-settings";

pub struct SettingsWindow;

fn section(parent: &GtkBox, title: &str) {
    let label = Label::new(Some(title));
    label.set_halign(gtk::Align::Start);
    label.add_css_class("section-title");
    parent.append(&label);
}

fn labelled(parent: &GtkBox, text: &str, widget: &impl IsA<gtk::Widget>) {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    let label = Label::new(Some(text));
    label.set_halign(gtk::Align::Start);
    label.set_hexpand(true);
    label.set_xalign(0.0);
    row.append(&label);
    row.append(widget);
    parent.append(&row);
}

fn color_entry(parent: &GtkBox, text: &str, value: &str) -> Entry {
    let entry = Entry::new();
    entry.set_text(value);
    entry.set_width_chars(22);
    entry.connect_changed(|entry| entry.remove_css_class("error"));
    labelled(parent, text, &entry);
    entry
}

fn spin(parent: &GtkBox, text: &str, min: f64, max: f64, step: f64, value: f64) -> SpinButton {
    let spin = SpinButton::with_range(min, max, step);
    spin.set_value(value);
    labelled(parent, text, &spin);
    spin
}

fn check(parent: &GtkBox, text: &str, active: bool) -> CheckButton {
    let check = CheckButton::with_label(text);
    check.set_active(active);
    parent.append(&check);
    check
}

impl SettingsWindow {
    pub fn open(app: &Application, config: Rc<RefCell<Config>>) {
        if let Some(existing) = app
            .windows()
            .into_iter()
            .find(|w| w.widget_name() == WINDOW_NAME)
        {
            existing.present();
            return;
        }

        let current = config.borrow().clone();

        let window = Window::builder()
            .application(app)
            .title("Poppi Launcher Settings")
            .default_width(520)
            .default_height(720)
            .build();
        window.set_widget_name(WINDOW_NAME);
        window.add_css_class(WINDOW_NAME);

        let main_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .margin_start(20)
            .margin_end(20)
            .margin_top(20)
            .margin_bottom(20)
            .build();

        section(&main_box, "Theme");
        let background = color_entry(&main_box, "Background color", &current.theme.background_color);
        let text = color_entry(&main_box, "Text color", &current.theme.text_color);
        let accent = color_entry(&main_box, "Accent color", &current.theme.accent_color);
        let font_size = spin(&main_box, "Font size (pt)", 6.0, 48.0, 1.0, f64::from(current.theme.font_size));
        let radius = spin(&main_box, "Corner radius (px)", 0.0, 50.0, 1.0, current.theme.border_radius);
        let width = spin(&main_box, "Width (px)", 300.0, 2000.0, 10.0, f64::from(current.theme.width));

        section(&main_box, "Behavior");
        let max_results = spin(
            &main_box,
            "Maximum results",
            1.0,
            20.0,
            1.0,
            current.behavior.max_results as f64,
        );
        let history_size = spin(
            &main_box,
            "Remembered queries (0 = off)",
            0.0,
            500.0,
            10.0,
            current.behavior.history_size as f64,
        );
        let focus_loss = check(
            &main_box,
            "Close when the launcher loses focus",
            current.behavior.close_on_focus_loss,
        );
        let resident = check(
            &main_box,
            "Keep running in the background for instant startup",
            current.behavior.stay_resident,
        );

        section(&main_box, "Search");
        let engine_labels: Vec<&str> = Engine::ALL.iter().map(|e| e.label()).collect();
        let default_engine = DropDown::from_strings(&engine_labels);
        let selected = Engine::from_id(&current.search.default_engine).unwrap_or(Engine::Google);
        default_engine.set_selected(
            Engine::ALL.iter().position(|e| *e == selected).unwrap_or(0) as u32,
        );
        labelled(&main_box, "Default search engine", &default_engine);
        let youtube = check(&main_box, "Enable YouTube search (yt)", current.search.youtube_enabled);
        let chatgpt = check(&main_box, "Enable ChatGPT search (gpt)", current.search.chatgpt_enabled);

        section(&main_box, "Calculator");
        let calculator = check(&main_box, "Enable calculator", current.calculator.enabled);

        section(&main_box, "Claude");
        let claude_enabled = check(
            &main_box,
            "Ask Claude when the text ends with ?",
            current.claude.enabled,
        );
        let claude_model = Entry::new();
        claude_model.set_text(&current.claude.model);
        claude_model.set_width_chars(22);
        labelled(&main_box, "Model (haiku, sonnet, opus)", &claude_model);
        let claude_note = Label::new(Some(
            "Uses the Claude Code login on this computer (your subscription). No API key is used.",
        ));
        claude_note.set_halign(gtk::Align::Start);
        claude_note.set_xalign(0.0);
        claude_note.set_wrap(true);
        claude_note.add_css_class("hint");
        main_box.append(&claude_note);

        section(&main_box, "Files");
        let files_enabled = check(&main_box, "Enable file search (f name)", current.files.enabled);
        let files_hidden = check(&main_box, "Include hidden files", current.files.include_hidden);

        section(&main_box, "Aliases and more");
        let advanced = Label::new(Some(
            "Aliases, custom search engines, indexed folders and exclusions are edited in the config file.",
        ));
        advanced.set_halign(gtk::Align::Start);
        advanced.set_xalign(0.0);
        advanced.set_wrap(true);
        advanced.add_css_class("hint");
        main_box.append(&advanced);
        let open_config = Button::with_label("Open config file");
        open_config.add_css_class("settings-button");
        open_config.set_halign(gtk::Align::Start);
        open_config.connect_clicked(|button| {
            let uri = gtk::gio::File::for_path(Config::config_path()).uri();
            let context = WidgetExt::display(button).app_launch_context();
            if let Err(e) = gtk::gio::AppInfo::launch_default_for_uri(&uri, Some(&context)) {
                eprintln!("Could not open config file: {}", e);
            }
        });
        main_box.append(&open_config);

        let hint = Label::new(Some("Changes apply as soon as you save."));
        hint.set_halign(gtk::Align::Start);
        hint.add_css_class("hint");
        main_box.append(&hint);

        let buttons = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .halign(gtk::Align::End)
            .margin_top(8)
            .build();
        let cancel_button = Button::with_label("Cancel");
        cancel_button.add_css_class("settings-button");
        let save_button = Button::with_label("Save");
        save_button.add_css_class("settings-button");
        save_button.add_css_class("primary");
        buttons.append(&cancel_button);
        buttons.append(&save_button);
        main_box.append(&buttons);

        let window_for_cancel = window.clone();
        cancel_button.connect_clicked(move |_| window_for_cancel.close());

        let window_for_save = window.clone();
        save_button.connect_clicked(move |_| {
            // Refuse colors GTK cannot parse instead of saving a broken theme.
            let mut colors_ok = true;
            for entry in [&background, &text, &accent] {
                if !style::is_valid_color(&entry.text()) {
                    entry.add_css_class("error");
                    colors_ok = false;
                }
            }
            if !colors_ok {
                return;
            }

            let updated = {
                let mut config = config.borrow_mut();
                config.theme.background_color = background.text().trim().to_string();
                config.theme.text_color = text.text().trim().to_string();
                config.theme.accent_color = accent.text().trim().to_string();
                config.theme.font_size = font_size.value_as_int();
                config.theme.border_radius = radius.value();
                config.theme.width = width.value_as_int();
                config.behavior.max_results = max_results.value_as_int().max(1) as usize;
                config.behavior.history_size = history_size.value_as_int().max(0) as usize;
                config.claude.enabled = claude_enabled.is_active();
                config.claude.model = claude_model.text().trim().to_string();
                config.files.enabled = files_enabled.is_active();
                config.files.include_hidden = files_hidden.is_active();
                config.behavior.close_on_focus_loss = focus_loss.is_active();
                config.behavior.stay_resident = resident.is_active();
                config.search.default_engine = Engine::ALL
                    .get(default_engine.selected() as usize)
                    .copied()
                    .unwrap_or(Engine::Google)
                    .id()
                    .to_string();
                config.search.youtube_enabled = youtube.is_active();
                config.search.chatgpt_enabled = chatgpt.is_active();
                config.calculator.enabled = calculator.is_active();
                *config = config.clone().sanitized();
                config.clone()
            };

            // Apply even if the file could not be written, but say so.
            style::apply(&updated);
            match updated.save() {
                Ok(()) => window_for_save.close(),
                Err(e) => {
                    eprintln!("Error saving config: {}", e);
                    hint.set_text(&format!("Applied, but could not save the config file: {}", e));
                }
            }
        });

        let scrolled = ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.set_child(Some(&main_box));
        window.set_child(Some(&scrolled));
        window.present();
    }
}
