//! The settings window. It uses the stock GTK theme (dark or light to match
//! the launcher), so only a few classes of its own are styled.

use crate::ai::providers;
use crate::config::Config;
use crate::emoji_picker::SKIN_TONES;
use crate::model_editor::{self, ModelEditor};
use crate::search::Engine;
use crate::style;
use crate::ui;
use gtk::prelude::*;
use gtk::{
    gdk, Application, Box as GtkBox, Button, CheckButton, ColorDialog, ColorDialogButton, DropDown,
    Entry, Label, Orientation, ScrolledWindow, SpinButton, Window,
};
use std::cell::RefCell;
use std::rc::Rc;

const WINDOW_NAME: &str = "poppi-settings";
const THEME_MODES: [(&str, &str); 3] = [
    ("dark", "Dark"),
    ("light", "Light"),
    ("system", "Follow system"),
];

pub struct SettingsWindow;

fn section(parent: &GtkBox, title: &str) {
    let label = Label::new(Some(title));
    label.set_halign(gtk::Align::Start);
    label.add_css_class("section-title");
    parent.append(&label);
}

/// A scrollable page of the settings stack; widgets are appended to the
/// returned box.
fn page(stack: &gtk::Stack, name: &str, title: &str) -> GtkBox {
    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_start(22)
        .margin_end(22)
        .margin_top(18)
        .margin_bottom(22)
        .build();
    let scrolled = ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_child(Some(&content));
    stack.add_titled(&scrolled, Some(name), title);
    content
}

fn hint(parent: &GtkBox, text: &str) -> Label {
    let label = Label::new(Some(text));
    // Fill the page width and wrap there, rather than at the label's own
    // (narrow) natural width.
    label.set_halign(gtk::Align::Fill);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class("hint");
    parent.append(&label);
    label
}

fn labelled(parent: &GtkBox, text: &str, widget: &impl IsA<gtk::Widget>) {
    let row = GtkBox::new(Orientation::Horizontal, 10);
    let label = Label::new(Some(text));
    label.set_hexpand(true);
    label.set_xalign(0.0);
    row.append(&label);
    row.append(widget);
    parent.append(&row);
}

/// A colour as text (any CSS colour) with a picker button kept in sync.
fn color_row(parent: &GtkBox, text: &str, value: &str) -> Entry {
    let entry = Entry::new();
    entry.set_text(value);
    entry.set_width_chars(20);

    let picker = ColorDialogButton::new(Some(ColorDialog::new()));
    if let Ok(rgba) = gdk::RGBA::parse(value) {
        picker.set_rgba(&rgba);
    }

    let entry_for_picker = entry.clone();
    picker.connect_rgba_notify(move |picker| {
        let rgba = picker.rgba();
        // Keep whatever the user typed if it already means this colour.
        let unchanged =
            gdk::RGBA::parse(entry_for_picker.text()).is_ok_and(|current| current == rgba);
        if !unchanged {
            let text = if rgba.alpha() < 1.0 {
                rgba.to_string()
            } else {
                style::to_hex(&rgba.to_string()).unwrap_or_else(|| rgba.to_string())
            };
            entry_for_picker.set_text(&text);
        }
    });
    let picker_for_entry = picker.clone();
    entry.connect_changed(move |entry| {
        entry.remove_css_class("error");
        if let Ok(rgba) = gdk::RGBA::parse(entry.text()) {
            if picker_for_entry.rgba() != rgba {
                picker_for_entry.set_rgba(&rgba);
            }
        }
    });

    let controls = GtkBox::new(Orientation::Horizontal, 6);
    controls.append(&entry);
    controls.append(&picker);
    labelled(parent, text, &controls);
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

fn dropdown(parent: &GtkBox, text: &str, options: &[&str], selected: usize) -> DropDown {
    let dropdown = DropDown::from_strings(options);
    dropdown.set_selected(selected as u32);
    labelled(parent, text, &dropdown);
    dropdown
}

/// One editable alias: keyword, target, remove button.
fn alias_row(list: &GtkBox, name: &str, target: &str) {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    let name_entry = Entry::new();
    name_entry.set_text(name);
    name_entry.set_width_chars(10);
    name_entry.set_placeholder_text(Some("keyword"));
    let target_entry = Entry::new();
    target_entry.set_text(target);
    target_entry.set_hexpand(true);
    target_entry.set_placeholder_text(Some("https://example.com/search?q={}  or a command"));
    let remove = Button::from_icon_name("user-trash-symbolic");
    remove.set_tooltip_text(Some("Remove this alias"));

    let list_for_remove = list.clone();
    let row_for_remove = row.clone();
    remove.connect_clicked(move |_| list_for_remove.remove(&row_for_remove));

    row.append(&name_entry);
    row.append(&target_entry);
    row.append(&remove);
    list.append(&row);
}

/// Read the alias rows back: (keyword, target) for every complete row.
fn collect_aliases(list: &GtkBox) -> Vec<(String, String)> {
    let mut aliases = Vec::new();
    let mut row = list.first_child();
    while let Some(widget) = row {
        let name = widget.first_child().and_downcast::<Entry>();
        let target = name
            .as_ref()
            .and_then(|name| name.next_sibling())
            .and_downcast::<Entry>();
        if let (Some(name), Some(target)) = (name, target) {
            let name = name.text().trim().to_string();
            let target = target.text().trim().to_string();
            if !name.is_empty() && !target.is_empty() {
                aliases.push((name, target));
            }
        }
        row = widget.next_sibling();
    }
    aliases
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
            .default_width(820)
            .default_height(600)
            .build();
        window.set_widget_name(WINDOW_NAME);
        window.add_css_class(WINDOW_NAME);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        let mut main_box = page(&stack, "appearance", "Appearance");
        let mode_labels: Vec<&str> = THEME_MODES.iter().map(|(_, label)| *label).collect();
        let mode = dropdown(
            &main_box,
            "Theme",
            &mode_labels,
            THEME_MODES
                .iter()
                .position(|(id, _)| *id == current.theme.mode)
                .unwrap_or(0),
        );
        let accent = color_row(&main_box, "Accent color", &current.theme.accent_color);
        let background = color_row(
            &main_box,
            "Background (dark)",
            &current.theme.background_color,
        );
        let text = color_row(&main_box, "Text (dark)", &current.theme.text_color);
        let light_background = color_row(
            &main_box,
            "Background (light)",
            &current.theme.light_background_color,
        );
        let light_text = color_row(&main_box, "Text (light)", &current.theme.light_text_color);
        let font_size = spin(
            &main_box,
            "Font size (pt)",
            6.0,
            48.0,
            1.0,
            f64::from(current.theme.font_size),
        );
        let radius = spin(
            &main_box,
            "Corner radius (px)",
            0.0,
            50.0,
            1.0,
            current.theme.border_radius,
        );
        let width = spin(
            &main_box,
            "Width (px)",
            300.0,
            2000.0,
            10.0,
            f64::from(current.theme.width),
        );

        main_box = page(&stack, "behavior", "Behavior");
        let max_results = spin(
            &main_box,
            "Results visible at once",
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

        main_box = page(&stack, "search", "Search");
        let engine_labels: Vec<&str> = Engine::ALL.iter().map(|e| e.label()).collect();
        let selected_engine =
            Engine::from_id(&current.search.default_engine).unwrap_or(Engine::Google);
        let default_engine = dropdown(
            &main_box,
            "Default search engine",
            &engine_labels,
            Engine::ALL
                .iter()
                .position(|e| *e == selected_engine)
                .unwrap_or(0),
        );
        let youtube = check(
            &main_box,
            "Enable YouTube search (yt)",
            current.search.youtube_enabled,
        );
        let chatgpt = check(
            &main_box,
            "Enable ChatGPT search (gpt)",
            current.search.chatgpt_enabled,
        );

        main_box = page(&stack, "ai", "AI models");
        let ai_enabled = check(
            &main_box,
            "Ask the default model when the text ends with ? (or starts with ask)",
            current.ai.enabled,
        );
        hint(
            &main_box,
            "Type a model's name before a question to ask that model instead, e.g. \"sonnet why is the sky blue\". \
             Claude, Gemini CLI and Codex use their own logins (your subscriptions). \
             Keys for API providers are saved in the GNOME keyring.",
        );
        model_editor::legend(&main_box);
        let models = ModelEditor::build(&main_box, &current.ai);

        main_box = page(&stack, "features", "Features");
        section(&main_box, "Calculator");
        let calculator = check(
            &main_box,
            "Enable calculator and unit conversion",
            current.calculator.enabled,
        );
        let currency = check(
            &main_box,
            "Convert currencies (downloads ECB rates from frankfurter.dev)",
            current.calculator.currency,
        );

        section(&main_box, "Files");
        let files_enabled = check(
            &main_box,
            "Enable file search (f name)",
            current.files.enabled,
        );
        let files_hidden = check(
            &main_box,
            "Include hidden files",
            current.files.include_hidden,
        );
        let files_gitignore = check(
            &main_box,
            "Skip files ignored by .gitignore",
            current.files.respect_gitignore,
        );

        section(&main_box, "Clipboard and emoji");
        let clipboard_enabled = check(
            &main_box,
            "Remember clipboard texts (clip), in memory only",
            current.clipboard.enabled,
        );
        let skin_tone = dropdown(
            &main_box,
            "Emoji skin tone",
            &SKIN_TONES,
            SKIN_TONES
                .iter()
                .position(|tone| *tone == current.emoji.skin_tone)
                .unwrap_or(0),
        );

        main_box = page(&stack, "aliases", "Aliases");
        hint(
            &main_box,
            "A keyword that opens a URL or runs a command. {} is replaced by what you type after it.",
        );
        let alias_list = GtkBox::new(Orientation::Vertical, 6);
        for (name, target) in &current.aliases {
            alias_row(&alias_list, name, target);
        }
        main_box.append(&alias_list);
        let add_alias = Button::with_label("Add alias");
        add_alias.set_halign(gtk::Align::Start);
        let alias_list_for_add = alias_list.clone();
        add_alias.connect_clicked(move |_| alias_row(&alias_list_for_add, "", ""));
        main_box.append(&add_alias);

        main_box = page(&stack, "config", "Config file");
        hint(
            &main_box,
            "Indexed folders, exclusions and the AI instructions (system_prompt) are edited in the file. Changes are picked up when it is saved.",
        );
        let open_config = Button::with_label("Open config file");
        open_config.set_halign(gtk::Align::Start);
        open_config.connect_clicked(|button| {
            let uri = gtk::gio::File::for_path(Config::config_path()).uri();
            let context = WidgetExt::display(button).app_launch_context();
            if let Err(e) = gtk::gio::AppInfo::launch_default_for_uri(&uri, Some(&context)) {
                eprintln!("Could not open config file: {}", e);
            }
        });
        main_box.append(&open_config);

        // Below the pages: status and buttons, always visible.
        let bottom = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .margin_start(16)
            .margin_end(16)
            .margin_top(10)
            .margin_bottom(12)
            .build();
        let status = Label::new(Some("Changes apply as soon as you save."));
        status.add_css_class("hint");
        status.set_wrap(true);
        status.set_xalign(0.0);
        status.set_hexpand(true);
        bottom.append(&status);

        let buttons = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Center)
            .build();
        let cancel_button = Button::with_label("Cancel");
        let save_button = Button::with_label("Save");
        save_button.add_css_class("suggested-action");
        buttons.append(&cancel_button);
        buttons.append(&save_button);
        bottom.append(&buttons);

        let window_for_cancel = window.clone();
        cancel_button.connect_clicked(move |_| window_for_cancel.close());

        let window_for_save = window.clone();
        save_button.connect_clicked(move |_| {
            // Refuse colors GTK cannot parse instead of saving a broken theme.
            let mut colors_ok = true;
            for entry in [&accent, &background, &text, &light_background, &light_text] {
                if !style::is_valid_color(&entry.text()) {
                    entry.add_css_class("error");
                    colors_ok = false;
                }
            }
            if !colors_ok {
                status.set_text("Some colors are not valid; nothing was saved.");
                return;
            }
            let edited = match models.collect() {
                Ok(edited) => edited,
                Err(error) => {
                    status.set_text(&format!("{} Nothing was saved.", error));
                    return;
                }
            };

            // Keys go to the keyring; without one, into the config file.
            let mut key_problems = Vec::new();
            let mut inline_keys = Vec::new();
            for (id, label, key) in &edited.keys {
                if let Err(error) = providers::keyring_store(id, label, key) {
                    key_problems.push(error);
                    inline_keys.push((id.clone(), key.clone()));
                }
            }
            models.keys_saved();

            let updated = {
                let mut config = config.borrow_mut();
                let color = |entry: &Entry| entry.text().trim().to_string();
                config.theme.mode = THEME_MODES
                    .get(mode.selected() as usize)
                    .map(|(id, _)| *id)
                    .unwrap_or("dark")
                    .to_string();
                config.theme.accent_color = color(&accent);
                config.theme.background_color = color(&background);
                config.theme.text_color = color(&text);
                config.theme.light_background_color = color(&light_background);
                config.theme.light_text_color = color(&light_text);
                config.theme.font_size = font_size.value_as_int();
                config.theme.border_radius = radius.value();
                config.theme.width = width.value_as_int();
                config.behavior.max_results = max_results.value_as_int().max(1) as usize;
                config.behavior.history_size = history_size.value_as_int().max(0) as usize;
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
                config.ai.enabled = ai_enabled.is_active();
                config.ai.models = edited.models.clone();
                config.ai.default = edited.default.clone();
                for (name, entry) in config.ai.models.iter_mut() {
                    let id = providers::keyring_id(name, entry);
                    if let Some((_, key)) = inline_keys.iter().find(|(key_id, _)| *key_id == id) {
                        entry.api_key = key.clone();
                    }
                }
                config.calculator.enabled = calculator.is_active();
                config.calculator.currency = currency.is_active();
                config.files.enabled = files_enabled.is_active();
                config.files.include_hidden = files_hidden.is_active();
                config.files.respect_gitignore = files_gitignore.is_active();
                config.clipboard.enabled = clipboard_enabled.is_active();
                config.emoji.skin_tone = SKIN_TONES
                    .get(skin_tone.selected() as usize)
                    .copied()
                    .unwrap_or("default")
                    .to_string();
                config.aliases = collect_aliases(&alias_list).into_iter().collect();
                *config = config.clone().sanitized();
                config.clone()
            };

            // Apply even if the file could not be written, but say so.
            style::apply(&updated);
            ui::config_changed();
            match updated.save() {
                Ok(()) if !key_problems.is_empty() => status.set_text(&format!(
                    "Saved, but the keyring did not take the key, so it is in the config file instead. {}",
                    key_problems[0]
                )),
                Ok(()) => window_for_save.close(),
                Err(e) => {
                    eprintln!("Error saving config: {}", e);
                    status.set_text(&format!("Applied, but could not save the config file: {}", e));
                }
            }
        });

        let sidebar = gtk::StackSidebar::new();
        sidebar.set_stack(&stack);
        sidebar.set_width_request(170);
        let body = GtkBox::new(Orientation::Horizontal, 0);
        body.append(&sidebar);
        body.append(&gtk::Separator::new(Orientation::Vertical));
        body.append(&stack);
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.append(&body);
        content.append(&gtk::Separator::new(Orientation::Horizontal));
        content.append(&bottom);
        window.set_child(Some(&content));

        // The UI test screenshots a chosen page.
        #[cfg(feature = "ui-test")]
        if let Ok(name) = std::env::var("POPPI_UI_SETTINGS_PAGE") {
            stack.set_visible_child_name(&name);
        }
        window.present();
    }
}
