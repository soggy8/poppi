//! First-run setup: which AI answers questions (explaining that it is an
//! online API, and the private alternatives), the keyboard shortcut and
//! voice input. Shown on the first launch and when "setup" is typed.

use crate::ai::{http, providers};
use crate::config::{Config, ModelConfig};
use crate::settings::{hint, section};
use crate::{shortcut, ui, utils, voice};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Application, Box as GtkBox, Button, CheckButton, Entry, Label, Window};
use std::cell::RefCell;
use std::rc::Rc;

const WINDOW_NAME: &str = "poppi-setup";
const SUGGESTED_SHORTCUT: &str = "<Control>space";
const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
const VOICE_HELP_URL: &str = "https://github.com/soggy8/poppi#voice-input";

/// One way to answer questions.
struct Choice {
    /// Model entry name in the config (also what is typed before a question).
    name: &'static str,
    provider: &'static str,
    model: &'static str,
    title: &'static str,
    about: &'static str,
    /// Where to create a key, for providers that need one.
    key_url: Option<&'static str>,
}

const CHOICES: &[Choice] = &[
    Choice {
        name: "groq",
        provider: "groq",
        model: "",
        title: "Groq — free and fast (recommended)",
        about: "Answers come from open models (gpt-oss) running on Groq's servers, so your \
                questions are sent to Groq over the internet. The free plan allows about 1,000 \
                questions a day and needs no credit card: create a key and paste it below.",
        key_url: Some("https://console.groq.com/keys"),
    },
    Choice {
        name: "openrouter",
        provider: "openrouter",
        model: "",
        title: "OpenRouter — free models",
        about: "One key for many models; the free ones allow about 50 questions a day. Your \
                questions go to OpenRouter and the company running the model. Test picks a \
                free model for you.",
        key_url: Some("https://openrouter.ai/keys"),
    },
    Choice {
        name: "gemini-api",
        provider: "gemini",
        model: "",
        title: "Google Gemini API — free tier",
        about: "Google's Gemini Flash, free with a key from Google AI Studio. Google may use what \
                you send on the free tier to improve its products.",
        key_url: Some("https://aistudio.google.com/apikey"),
    },
    Choice {
        name: "llama",
        provider: "ollama",
        model: "llama3.2",
        title: "Ollama — on this computer",
        about: "Nothing leaves your computer. Needs Ollama (ollama.com) and a model, e.g. \
                `ollama pull llama3.2`. Without a good graphics card answers come slower.",
        key_url: None,
    },
    Choice {
        name: "haiku",
        provider: "claude",
        model: "haiku",
        title: "Claude Code — your Claude subscription",
        about: "Uses the claude command you are logged in to. Very good answers; they count \
                against your Claude plan.",
        key_url: None,
    },
    Choice {
        name: "",
        provider: "",
        model: "",
        title: "No AI",
        about: "Poppi works fine without it: apps, files, calculator, windows and the rest stay. \
                You can come back here any time by typing \"setup\".",
        key_url: None,
    },
];

/// The choice matching the current default model, Groq otherwise.
fn current_choice(config: &Config) -> usize {
    if !config.ai.enabled {
        return CHOICES.len() - 1;
    }
    let provider = config
        .ai
        .models
        .get(&config.ai.default)
        .map(|m| m.provider.as_str())
        .unwrap_or("");
    CHOICES
        .iter()
        .position(|c| !c.provider.is_empty() && c.provider == provider)
        .unwrap_or(0)
}

/// A free model from OpenRouter's model list, preferring capable ones.
fn pick_free_model(models: &serde_json::Value) -> Option<String> {
    let ids: Vec<&str> = models
        .get("data")?
        .as_array()?
        .iter()
        .filter_map(|m| m.get("id")?.as_str())
        .filter(|id| id.ends_with(":free"))
        .collect();
    let preferred = ["gpt-oss-120b", "llama-3.3-70b", "deepseek", "qwen", "gemma"];
    preferred
        .iter()
        .find_map(|p| ids.iter().find(|id| id.contains(p)))
        .or(ids.first())
        .map(|id| id.to_string())
}

/// Check that a choice works, with the typed key or else the saved one.
/// Returns the model to use when the check chose one. Blocking.
fn check(choice_index: usize, typed_key: Option<String>) -> Result<Option<String>, String> {
    let choice = &CHOICES[choice_index];
    let provider = providers::provider(choice.provider).ok_or("Unknown provider")?;
    let key = match typed_key {
        Some(key) => Some(key),
        None if choice.key_url.is_some() => Some(
            providers::keyring_lookup(choice.provider)
                .ok_or("Paste a key first (\"Get a free key\" opens the page)")?,
        ),
        None => None,
    };
    match choice.provider {
        "groq" | "gemini" => {
            http::get_json(&format!("{}/models", provider.base_url), key.as_deref())
                .map_err(|e| format!("The key did not work: {}", e))?;
            Ok(None)
        }
        "openrouter" => {
            http::get_json(&format!("{}/key", provider.base_url), key.as_deref())
                .map_err(|e| format!("The key did not work: {}", e))?;
            let models = http::get_json(&format!("{}/models", provider.base_url), None)?;
            pick_free_model(&models)
                .map(Some)
                .ok_or_else(|| "OpenRouter lists no free models right now".to_string())
        }
        "ollama" => {
            let models = http::get_json(&format!("{}/models", provider.base_url), None)
                .map_err(|_| "Ollama is not running. Install it from ollama.com and start it.")?;
            let ids: Vec<String> = models
                .get("data")
                .and_then(|d| d.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.get("id")?.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if ids.is_empty() {
                return Err("Ollama has no models yet: run `ollama pull llama3.2`".to_string());
            }
            let wanted = ids.iter().find(|id| id.starts_with(choice.model));
            Ok(Some(wanted.unwrap_or(&ids[0]).clone()))
        }
        "claude" => {
            if utils::has_command("claude") {
                Ok(None)
            } else {
                Err("The claude command is not installed (claude.com/claude-code)".to_string())
            }
        }
        _ => Ok(None),
    }
}

struct Setup {
    app: Application,
    window: Window,
    config: Rc<RefCell<Config>>,
    radios: Vec<CheckButton>,
    selected: RefCell<usize>,
    about: Label,
    key_row: GtkBox,
    key_link: Button,
    key_entry: Entry,
    /// A key for this provider is already in the keyring.
    key_saved: RefCell<Vec<String>>,
    ai_status: Label,
    /// The choice and key last checked successfully, and the model it chose.
    verified: RefCell<Option<(usize, String, Option<String>)>>,
    shortcut_label: Label,
    shortcut_status: Label,
    shortcut: RefCell<Option<String>>,
    capturing: std::cell::Cell<bool>,
    voice_status: Label,
    autostart: CheckButton,
    wake_word: CheckButton,
    done: Button,
}

pub fn open(app: &Application, config: Rc<RefCell<Config>>) {
    if let Some(existing) = app
        .windows()
        .into_iter()
        .find(|w| w.widget_name() == WINDOW_NAME)
    {
        existing.present();
        return;
    }
    let setup = Setup::build(app, config);
    setup.window.present();
}

impl Setup {
    fn build(app: &Application, config: Rc<RefCell<Config>>) -> Rc<Self> {
        let window = Window::builder()
            .application(app)
            .title("Welcome to Poppi")
            .default_width(640)
            .default_height(720)
            .build();
        window.set_widget_name(WINDOW_NAME);
        window.add_css_class("poppi-settings");

        let content = GtkBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_start(26)
            .margin_end(26)
            .margin_top(22)
            .margin_bottom(22)
            .build();

        let title = Label::new(Some("Welcome to Poppi"));
        title.add_css_class("title-1");
        title.set_halign(gtk::Align::Start);
        content.append(&title);
        hint(
            &content,
            "Open apps, files and windows, calculate, convert, and ask quick questions, all \
             from one bar. Three things to set up; each can be changed later by typing \"setup\".",
        );

        // ----- AI -----
        section(&content, "1. Answering questions");
        hint(
            &content,
            "End a search with ? to ask an AI. Poppi uses an AI service through its online \
             API, and small free models are plenty for quick answers. Pick one:",
        );
        let current = config.borrow().clone();
        let selected = current_choice(&current);
        let mut radios = Vec::new();
        for (i, choice) in CHOICES.iter().enumerate() {
            let radio = CheckButton::with_label(choice.title);
            if let Some(first) = radios.first() {
                radio.set_group(Some(first));
            }
            radio.set_active(i == selected);
            content.append(&radio);
            radios.push(radio);
        }
        let about = Label::new(None);
        about.set_wrap(true);
        about.set_xalign(0.0);
        about.set_margin_start(28);
        content.append(&about);

        let key_row = GtkBox::new(gtk::Orientation::Horizontal, 8);
        key_row.set_margin_start(28);
        let key_link = Button::with_label("Get a free key");
        let key_entry = Entry::builder()
            .placeholder_text("Paste your API key")
            .visibility(false)
            .hexpand(true)
            .build();
        let test = Button::with_label("Test");
        key_row.append(&key_link);
        key_row.append(&key_entry);
        key_row.append(&test);
        content.append(&key_row);

        let ai_status = Label::new(None);
        ai_status.add_css_class("hint");
        ai_status.set_wrap(true);
        ai_status.set_xalign(0.0);
        ai_status.set_margin_start(28);
        content.append(&ai_status);

        // ----- shortcut -----
        section(&content, "2. Keyboard shortcut");
        let shortcut_row = GtkBox::new(gtk::Orientation::Horizontal, 10);
        let shortcut_label = Label::new(None);
        shortcut_label.set_hexpand(true);
        shortcut_label.set_xalign(0.0);
        let change_shortcut = Button::with_label("Change…");
        shortcut_row.append(&shortcut_label);
        shortcut_row.append(&change_shortcut);
        content.append(&shortcut_row);
        let shortcut_status = hint(&content, "");
        let autostart = CheckButton::with_label(
            "Start Poppi in the background when you log in, so it opens instantly",
        );
        // On for a fresh setup (no shortcut yet); afterwards what is set.
        autostart.set_active(shortcut::autostart_enabled() || shortcut::current().is_none());
        content.append(&autostart);

        // ----- voice -----
        section(&content, "3. Voice");
        let voice_status = hint(&content, "");
        let voice_buttons = GtkBox::new(gtk::Orientation::Horizontal, 8);
        let download = Button::with_label("Download speech model (148 MB)");
        let voice_help = Button::with_label("How to install whisper-cli");
        voice_buttons.append(&download);
        voice_buttons.append(&voice_help);
        content.append(&voice_buttons);
        let wake_word = CheckButton::with_label(
            "Say \u{201c}Poppi\u{201d} to open it, e.g. \u{201c}Poppi, open my downloads\u{201d}",
        );
        wake_word.set_active(current.voice.wake_word);
        content.append(&wake_word);
        hint(
            &content,
            "The microphone then stays open while Poppi runs; what you say is checked on this \
             computer only, and nothing is kept. Opening apps, files and searches runs right \
             away when asked by voice; commands and power actions still wait for Enter.",
        );

        // ----- buttons -----
        let buttons = GtkBox::new(gtk::Orientation::Horizontal, 10);
        buttons.set_halign(gtk::Align::End);
        buttons.set_margin_top(14);
        let skip = Button::with_label("Skip");
        let done = Button::with_label("Done");
        done.add_css_class("suggested-action");
        buttons.append(&skip);
        buttons.append(&done);
        content.append(&buttons);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.set_child(Some(&content));
        window.set_child(Some(&scrolled));

        let setup = Rc::new(Self {
            app: app.clone(),
            window,
            config,
            radios,
            selected: RefCell::new(selected),
            about,
            key_row,
            key_link,
            key_entry,
            key_saved: RefCell::new(Vec::new()),
            ai_status,
            verified: RefCell::new(None),
            shortcut_label,
            shortcut_status,
            shortcut: RefCell::new(None),
            capturing: std::cell::Cell::new(false),
            voice_status,
            autostart,
            wake_word,
            done,
        });

        for (i, radio) in setup.radios.iter().enumerate() {
            let this = setup.clone();
            radio.connect_toggled(move |radio| {
                if radio.is_active() {
                    *this.selected.borrow_mut() = i;
                    this.show_choice();
                }
            });
        }
        let this = setup.clone();
        setup.key_link.connect_clicked(move |_| {
            if let Some(url) = CHOICES[*this.selected.borrow()].key_url {
                this.open_uri(url);
            }
        });
        let this = setup.clone();
        setup.key_entry.connect_changed(move |_| {
            this.verified.borrow_mut().take();
        });
        let this = setup.clone();
        test.connect_clicked(move |_| this.run_check(false));
        let this = setup.clone();
        change_shortcut.connect_clicked(move |button| {
            this.capturing.set(true);
            button.set_label("Press the keys…");
            this.shortcut_status
                .set_text("Press the key combination to use, or Escape to keep the current one.");
        });
        let this = setup.clone();
        let change_button = change_shortcut.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if !this.capturing.get() {
                return glib::Propagation::Proceed;
            }
            let mods = modifiers & gtk::accelerator_get_default_mod_mask();
            if key == gdk::Key::Escape && mods.is_empty() {
                this.capturing.set(false);
                change_button.set_label("Change…");
                this.show_shortcut();
                return glib::Propagation::Stop;
            }
            if !gtk::accelerator_valid(key, mods) || mods.is_empty() {
                // A modifier on its own, or a plain key: keep waiting.
                return glib::Propagation::Stop;
            }
            this.capturing.set(false);
            change_button.set_label("Change…");
            *this.shortcut.borrow_mut() = Some(gtk::accelerator_name(key, mods).to_string());
            this.show_shortcut();
            glib::Propagation::Stop
        });
        setup.window.add_controller(keys);

        let this = setup.clone();
        download.connect_clicked(move |button| this.download_model(button));
        let this = setup.clone();
        voice_help.connect_clicked(move |_| this.open_uri(VOICE_HELP_URL));
        let this = setup.clone();
        skip.connect_clicked(move |_| {
            this.window.close();
            ui::activate(&this.app, &this.config, None);
        });
        let this = setup.clone();
        setup.done.connect_clicked(move |_| this.run_check(true));

        setup.show_choice();
        setup.load_shortcut();
        setup.show_voice(&download, &voice_buttons);
        setup.load_saved_keys();
        setup
    }

    fn open_uri(&self, uri: &str) {
        let context = WidgetExt::display(&self.window).app_launch_context();
        if let Err(e) = gio::AppInfo::launch_default_for_uri(uri, Some(&context)) {
            self.ai_status
                .set_text(&format!("Could not open {}: {}", uri, e));
        }
    }

    fn show_choice(&self) {
        let index = *self.selected.borrow();
        let choice = &CHOICES[index];
        self.about.set_text(choice.about);
        // The description and key field sit right under the chosen option.
        if let Some(content) = self.about.parent().and_downcast::<GtkBox>() {
            content.reorder_child_after(&self.about, Some(&self.radios[index]));
            content.reorder_child_after(&self.key_row, Some(&self.about));
            content.reorder_child_after(&self.ai_status, Some(&self.key_row));
        }
        let needs_key = choice.key_url.is_some();
        self.key_row.set_visible(needs_key);
        self.ai_status.set_text("");
        if needs_key && self.key_saved.borrow().iter().any(|p| p == choice.provider) {
            self.key_entry
                .set_placeholder_text(Some("A key is saved. Paste a new one to replace it"));
        } else {
            self.key_entry
                .set_placeholder_text(Some("Paste your API key"));
        }
    }

    /// Which providers already have a key in the keyring.
    fn load_saved_keys(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let saved = gio::spawn_blocking(|| {
                CHOICES
                    .iter()
                    .filter(|c| c.key_url.is_some())
                    .filter(|c| providers::keyring_lookup(c.provider).is_some())
                    .map(|c| c.provider.to_string())
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            *this.key_saved.borrow_mut() = saved;
            this.show_choice();
        });
    }

    /// The key typed in, if any.
    fn typed_key(&self) -> Option<String> {
        Some(self.key_entry.text().trim().to_string()).filter(|k| !k.is_empty())
    }

    /// Check the selected choice; with `then_save`, save and close if it
    /// works (or if it was already checked).
    fn run_check(self: &Rc<Self>, then_save: bool) {
        let index = *self.selected.borrow();
        let choice = &CHOICES[index];
        if choice.provider.is_empty() {
            if then_save {
                self.save(index, None);
            }
            return;
        }
        let key = self.typed_key().filter(|_| choice.key_url.is_some());
        let has_saved = self.key_saved.borrow().iter().any(|p| p == choice.provider);
        if choice.key_url.is_some() && key.is_none() && !has_saved {
            self.ai_status.set_text(
                "Paste a key first (\"Get a free key\" opens the page), or pick another option.",
            );
            return;
        }
        let already_checked = self
            .verified
            .borrow()
            .as_ref()
            .is_some_and(|(i, k, _)| *i == index && *k == key.clone().unwrap_or_default());
        if then_save && already_checked {
            self.save(index, key);
            return;
        }
        self.ai_status.set_text("Checking…");
        self.done.set_sensitive(false);
        let this = self.clone();
        let key_for_check = key.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || check(index, key_for_check))
                .await
                .unwrap_or_else(|_| Err("The check failed".to_string()));
            this.done.set_sensitive(true);
            match result {
                Ok(model) => {
                    *this.verified.borrow_mut() =
                        Some((index, key.clone().unwrap_or_default(), model));
                    if then_save {
                        this.save(index, key);
                    } else {
                        this.ai_status.set_text("✓ Works. Click Done to use it.");
                    }
                }
                Err(error) => this.ai_status.set_text(&error),
            }
        });
    }

    fn save(&self, index: usize, key: Option<String>) {
        let choice = &CHOICES[index];
        let chosen_model = self
            .verified
            .borrow()
            .as_ref()
            .filter(|(i, _, _)| *i == index)
            .and_then(|(_, _, model)| model.clone());
        let mut problems = Vec::new();

        let updated = {
            let mut config = self.config.borrow_mut();
            if choice.provider.is_empty() {
                config.ai.enabled = false;
            } else {
                config.ai.enabled = true;
                let entry = config
                    .ai
                    .models
                    .entry(choice.name.to_string())
                    .or_insert_with(|| ModelConfig::new(choice.provider, choice.model));
                entry.provider = choice.provider.to_string();
                if let Some(model) = &chosen_model {
                    entry.model = model.clone();
                }
                if let Some(key) = key.as_deref().filter(|k| !k.is_empty()) {
                    let id = providers::keyring_id(choice.name, entry);
                    let label = format!("Poppi Launcher: {}", choice.title);
                    if let Err(error) = providers::keyring_store(&id, &label, key) {
                        // No keyring (e.g. not GNOME): keep the key in the
                        // config file, which is then only readable by you.
                        entry.api_key = key.to_string();
                        problems.push(format!("Key saved in the config file: {}", error));
                    }
                }
                config.ai.default = choice.name.to_string();
            }
            config.voice.wake_word = self.wake_word.is_active();
            if config.voice.wake_word {
                // It can only hear "Poppi" while running.
                config.behavior.stay_resident = true;
            }
            config.clone()
        };
        if let Err(error) = shortcut::set_autostart(self.autostart.is_active()) {
            problems.push(format!("Could not set up starting at login: {}", error));
        }

        if let Some(accel) = self.shortcut.borrow().as_deref() {
            if let Err(error) = shortcut::bind(accel, &shortcut::launch_command()) {
                problems.push(format!("Could not set the shortcut: {}", error));
            }
        }
        if let Err(error) = updated.save() {
            problems.push(format!("Could not save the config: {}", error));
        }
        ui::config_changed();
        if !problems.is_empty() {
            // Saved and usable; leave the window open so this can be read.
            self.ai_status.set_text(&problems.join(". "));
            return;
        }
        self.window.close();
        // Straight into the launcher, so the shortcut is not needed yet.
        ui::activate(&self.app, &self.config, None);
    }

    fn load_shortcut(self: &Rc<Self>) {
        if !shortcut::supported() {
            self.shortcut_label.set_text(&format!(
                "Bind a key to `{}` in your desktop's keyboard settings.",
                shortcut::launch_command()
            ));
            return;
        }
        if shortcut::current().is_none() {
            *self.shortcut.borrow_mut() = Some(SUGGESTED_SHORTCUT.to_string());
        }
        self.show_shortcut();
    }

    fn show_shortcut(self: &Rc<Self>) {
        let pending = self.shortcut.borrow().clone();
        let accel = pending.clone().or_else(shortcut::current);
        let label = accel
            .as_deref()
            .and_then(gtk::accelerator_parse)
            .map(|(key, mods)| gtk::accelerator_get_label(key, mods).to_string());
        match (&label, &pending) {
            (Some(label), Some(_)) => self.shortcut_label.set_text(&format!(
                "Open Poppi with {} (set when you click Done)",
                label
            )),
            (Some(label), None) => self
                .shortcut_label
                .set_text(&format!("Open Poppi with {}", label)),
            (None, _) => self.shortcut_label.set_text("No shortcut yet"),
        }
        let Some(accel) = pending else {
            self.shortcut_status.set_text("");
            return;
        };
        let this = self.clone();
        glib::spawn_future_local(async move {
            let clashes = gio::spawn_blocking(move || shortcut::clashes(&accel))
                .await
                .unwrap_or_default();
            this.shortcut_status.set_text(&if clashes.is_empty() {
                String::new()
            } else {
                format!(
                    "Also used by: {}. Both would react to it, so pick another combination \
                     or remove that shortcut in GNOME Settings → Keyboard.",
                    clashes.join(", ")
                )
            });
        });
    }

    fn show_voice(&self, download: &Button, buttons: &GtkBox) {
        let mut voice = self.config.borrow().voice.clone();
        voice.enabled = true;
        let reason = voice::unavailable_reason(&voice);
        let model_missing = !crate::files::expand_tilde(&voice.model).is_file();
        let cli_missing = voice.command.trim().is_empty() && !utils::has_command("whisper-cli");
        self.voice_status.set_text(&match reason {
            None => "Ready: press Ctrl+D in Poppi and talk; the words appear as you speak. \
                     Speech is turned into text on this computer."
                .to_string(),
            Some(reason) => format!(
                "Press Ctrl+D in Poppi to dictate. Speech is turned into text on this computer \
                 by whisper.cpp, which needs setting up: {}.",
                reason
            ),
        });
        download.set_visible(model_missing);
        buttons.set_visible(model_missing || cli_missing);
        let help_visible = cli_missing;
        if let Some(help) = buttons.last_child() {
            help.set_visible(help_visible);
        }
    }

    fn download_model(self: &Rc<Self>, button: &Button) {
        let target = crate::files::expand_tilde(&self.config.borrow().voice.model);
        button.set_sensitive(false);
        button.set_label("Downloading…");
        let this = self.clone();
        let button = button.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || download(MODEL_URL, &target))
                .await
                .unwrap_or_else(|_| Err("Download failed".to_string()));
            match result {
                Ok(()) => {
                    button.set_visible(false);
                    if let Some(parent) = button.parent().and_downcast::<GtkBox>() {
                        this.show_voice(&button, &parent);
                    }
                }
                Err(error) => {
                    button.set_sensitive(true);
                    button.set_label("Download speech model (148 MB)");
                    this.voice_status.set_text(&error);
                }
            }
        });
    }
}

/// Download `url` to `target`, through a temporary file so an interrupted
/// download never looks like a model. Blocking.
fn download(url: &str, target: &std::path::Path) -> Result<(), String> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let partial = target.with_extension("part");
    let status = std::process::Command::new("curl")
        .args(["--silent", "--show-error", "--fail", "--location", "-o"])
        .arg(&partial)
        .arg(url)
        .status()
        .map_err(|e| format!("Could not run curl: {}", e))?;
    if !status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err("Download failed; check your connection and try again".to_string());
    }
    std::fs::rename(&partial, target).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_choice_is_a_known_provider_with_a_valid_name() {
        for choice in CHOICES.iter().filter(|c| !c.provider.is_empty()) {
            assert!(
                providers::provider(choice.provider).is_some(),
                "{}",
                choice.provider
            );
            assert!(crate::config::is_valid_model_name(choice.name));
            let entry = ModelConfig::new(choice.provider, choice.model);
            providers::resolve_entry(choice.name, &entry, "").unwrap();
        }
    }

    #[test]
    fn current_choice_follows_the_default_model() {
        let mut config = Config::default();
        assert_eq!(CHOICES[current_choice(&config)].provider, "groq");
        config.ai.default = "haiku".to_string();
        assert_eq!(CHOICES[current_choice(&config)].provider, "claude");
        config.ai.enabled = false;
        assert_eq!(CHOICES[current_choice(&config)].title, "No AI");
    }

    #[test]
    fn free_openrouter_models_are_picked() {
        let models = json!({ "data": [
            { "id": "openai/gpt-5" },
            { "id": "qwen/qwen3-32b:free" },
            { "id": "openai/gpt-oss-120b:free" },
        ]});
        assert_eq!(
            pick_free_model(&models).as_deref(),
            Some("openai/gpt-oss-120b:free")
        );
        let odd = json!({ "data": [{ "id": "x/y:free" }, { "id": "x/z" }] });
        assert_eq!(pick_free_model(&odd).as_deref(), Some("x/y:free"));
        assert_eq!(pick_free_model(&json!({ "data": [{ "id": "x/z" }] })), None);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Talks to the real services with an invalid key:
    /// `cargo test checks_against_real_services -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn checks_against_real_services() {
        let groq = check(0, Some("gsk_not_a_real_key".to_string()));
        println!("groq, bad key: {:?}", groq);
        assert!(groq.unwrap_err().starts_with("The key did not work"));
        let models = http::get_json("https://openrouter.ai/api/v1/models", None).unwrap();
        println!("openrouter free pick: {:?}", pick_free_model(&models));
        println!("ollama: {:?}", check(3, None));
        println!("claude: {:?}", check(4, None));
    }
}
