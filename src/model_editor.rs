//! The "AI models" part of the settings window: one row per model with the
//! name you type, the provider, the model id and, where needed, an API key
//! (saved in the GNOME keyring), a server address or a command.

use crate::ai::providers::{self, Kind, Provider, PROVIDERS};
use crate::config::{is_valid_model_name, AiConfig, ModelConfig};
use gtk::prelude::*;
use gtk::{gio, glib};
use gtk::{Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, Orientation, PasswordEntry};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// How a provider is listed in the picker.
fn picker_name(provider: &Provider) -> String {
    match (provider.kind, provider.id) {
        (Kind::ClaudeCli, _) => "Claude · Claude Code login".to_string(),
        (Kind::GeminiCli, _) => "Gemini CLI · Google login".to_string(),
        (Kind::Codex, _) => "Codex CLI · ChatGPT login".to_string(),
        (Kind::Command, _) => "Custom command".to_string(),
        (_, "openai-compatible") => "OpenAI-compatible server".to_string(),
        (_, _) if !provider.needs_key => format!("{} · local", provider.name),
        (_, _) => format!("{} · API key", provider.name),
    }
}

fn shows_key(provider: &Provider) -> bool {
    provider.needs_key || provider.id == "openai-compatible"
}

struct ModelRow {
    container: GtkBox,
    default: CheckButton,
    name: Entry,
    provider: DropDown,
    model: Entry,
    key_row: GtkBox,
    key: PasswordEntry,
    forget_key: Button,
    base_url: Entry,
    command: Entry,
    /// Fields the editor does not show (label, api_key_env, inline key).
    original: ModelConfig,
    removed: Cell<bool>,
}

impl ModelRow {
    fn selected_provider(&self) -> &'static Provider {
        PROVIDERS
            .get(self.provider.selected() as usize)
            .unwrap_or(&PROVIDERS[0])
    }

    fn keyring_id(&self) -> String {
        let entry = ModelConfig::new(self.selected_provider().id, "");
        providers::keyring_id(self.name.text().trim(), &entry)
    }

    /// Show the fields this provider uses.
    fn update_fields(&self) {
        let provider = self.selected_provider();
        self.model.set_placeholder_text(Some(provider.model_hint));
        self.model.set_sensitive(provider.kind != Kind::Command);
        self.key_row.set_visible(shows_key(provider));
        self.base_url
            .set_visible(provider.id == "openai-compatible" || !self.base_url.text().is_empty());
        self.base_url
            .set_placeholder_text(Some(if provider.base_url.is_empty() {
                "http://host:port/v1"
            } else {
                provider.base_url
            }));
        self.command.set_visible(provider.kind == Kind::Command);
        self.refresh_key_state();
    }

    /// Say whether a key is already available, without blocking the UI on
    /// the keyring.
    fn refresh_key_state(&self) {
        let provider = self.selected_provider();
        if !shows_key(provider) {
            return;
        }
        self.key.set_placeholder_text(Some("API key"));
        self.forget_key.set_visible(false);
        let id = self.keyring_id();
        let env = Some(self.original.api_key_env.trim())
            .filter(|e| !e.is_empty())
            .unwrap_or(provider.key_env)
            .to_string();
        let inline = !self.original.api_key.trim().is_empty();
        let key = self.key.clone();
        let forget = self.forget_key.clone();
        glib::spawn_future_local(async move {
            let saved = gio::spawn_blocking(move || providers::keyring_lookup(&id).is_some())
                .await
                .unwrap_or(false);
            let env_set =
                !env.is_empty() && std::env::var(&env).is_ok_and(|v| !v.trim().is_empty());
            let placeholder = if saved {
                "Saved in the keyring · type to replace".to_string()
            } else if inline {
                "Set in the config file · type to replace".to_string()
            } else if env_set {
                format!("Using ${} · type to save one", env)
            } else if !env.is_empty() {
                format!("API key (or set ${})", env)
            } else {
                "API key, if the server needs one".to_string()
            };
            key.set_placeholder_text(Some(&placeholder));
            forget.set_visible(saved);
        });
    }
}

pub struct ModelEditor {
    list: GtkBox,
    rows: RefCell<Vec<Rc<ModelRow>>>,
    /// Radio group leader for the "default" buttons.
    group: RefCell<Option<CheckButton>>,
}

/// What the editor produced, for the settings window to apply.
pub struct EditedModels {
    pub models: BTreeMap<String, ModelConfig>,
    pub default: String,
    /// (keyring id, label, key) to store.
    pub keys: Vec<(String, String, String)>,
}

impl ModelEditor {
    pub fn build(parent: &GtkBox, ai: &AiConfig) -> Rc<Self> {
        let list = GtkBox::new(Orientation::Vertical, 10);
        parent.append(&list);
        let editor = Rc::new(Self {
            list,
            rows: RefCell::new(Vec::new()),
            group: RefCell::new(None),
        });
        for (name, model) in &ai.models {
            editor.add_row(name, model, *name == ai.default);
        }

        let add = Button::with_label("Add model");
        add.set_halign(gtk::Align::Start);
        let editor_for_add = editor.clone();
        add.connect_clicked(move |_| {
            let row = editor_for_add.add_row("", &ModelConfig::new("openai", ""), false);
            row.name.grab_focus();
        });
        parent.append(&add);
        editor
    }

    fn add_row(self: &Rc<Self>, name: &str, model: &ModelConfig, is_default: bool) -> Rc<ModelRow> {
        let container = GtkBox::new(Orientation::Vertical, 6);
        container.add_css_class("model-row");

        let top = GtkBox::new(Orientation::Horizontal, 6);
        let default = CheckButton::new();
        default.set_tooltip_text(Some("Answers questions that do not name a model"));
        let leader = self.group.borrow().clone();
        match leader {
            Some(leader) => default.set_group(Some(&leader)),
            None => *self.group.borrow_mut() = Some(default.clone()),
        }
        default.set_active(is_default);

        let name_entry = Entry::new();
        name_entry.set_text(name);
        name_entry.set_width_chars(8);
        name_entry.set_placeholder_text(Some("name"));
        name_entry.set_tooltip_text(Some("Type this before a question to ask this model"));

        let labels: Vec<String> = PROVIDERS.iter().map(picker_name).collect();
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let provider = DropDown::from_strings(&label_refs);
        let index = PROVIDERS
            .iter()
            .position(|p| p.id == model.provider)
            .unwrap_or(0);
        provider.set_selected(index as u32);
        // Same width in every row so the columns line up.
        provider.set_width_request(250);

        let model_entry = Entry::new();
        model_entry.set_text(&model.model);
        model_entry.set_hexpand(true);
        model_entry.set_width_chars(12);

        let remove = Button::from_icon_name("user-trash-symbolic");
        remove.set_tooltip_text(Some("Remove this model"));

        top.append(&default);
        top.append(&name_entry);
        top.append(&provider);
        top.append(&model_entry);
        top.append(&remove);
        container.append(&top);

        // Second line: key / server / command, as the provider needs.
        let key_row = GtkBox::new(Orientation::Horizontal, 6);
        key_row.set_margin_start(30);
        let key = PasswordEntry::new();
        key.set_show_peek_icon(true);
        key.set_hexpand(true);
        let forget_key = Button::from_icon_name("edit-clear-symbolic");
        forget_key.set_tooltip_text(Some("Remove the key saved in the keyring"));
        key_row.append(&key);
        key_row.append(&forget_key);
        container.append(&key_row);

        let base_url = Entry::new();
        base_url.set_text(&model.base_url);
        base_url.set_margin_start(30);
        container.append(&base_url);

        let command = Entry::new();
        command.set_text(&model.command);
        command.set_margin_start(30);
        command.set_placeholder_text(Some(
            "Command that reads the prompt on stdin, e.g. llm -m gpt-4o",
        ));
        container.append(&command);

        let row = Rc::new(ModelRow {
            container: container.clone(),
            default,
            name: name_entry,
            provider,
            model: model_entry,
            key_row,
            key,
            forget_key: forget_key.clone(),
            base_url,
            command,
            original: model.clone(),
            removed: Cell::new(false),
        });
        row.update_fields();

        let row_for_provider = row.clone();
        row.provider
            .connect_selected_notify(move |_| row_for_provider.update_fields());

        let row_for_forget = row.clone();
        forget_key.connect_clicked(move |_| {
            if let Err(e) = providers::keyring_clear(&row_for_forget.keyring_id()) {
                eprintln!("{}", e);
            }
            row_for_forget.refresh_key_state();
        });

        let row_for_remove = row.clone();
        let list = self.list.clone();
        remove.connect_clicked(move |_| {
            row_for_remove.removed.set(true);
            list.remove(&row_for_remove.container);
        });

        self.list.append(&container);
        self.rows.borrow_mut().push(row.clone());
        row
    }

    /// Read the rows back. An error names the first problem found.
    pub fn collect(&self) -> Result<EditedModels, String> {
        let mut models = BTreeMap::new();
        let mut default = None;
        let mut keys = Vec::new();
        for row in self.rows.borrow().iter().filter(|row| !row.removed.get()) {
            let name = row.name.text().trim().to_string();
            row.name.remove_css_class("error");
            if name.is_empty() && row.model.text().trim().is_empty() && row.key.text().is_empty() {
                // A row added and left empty.
                continue;
            }
            if !is_valid_model_name(&name) || models.contains_key(&name) {
                row.name.add_css_class("error");
                return Err(if name.is_empty() {
                    "Every model needs a name.".to_string()
                } else if models.contains_key(&name) {
                    format!("Two models are called \"{}\".", name)
                } else {
                    format!("\"{}\" cannot be a model name: use one word.", name)
                });
            }
            let provider = row.selected_provider();
            let mut entry = ModelConfig {
                provider: provider.id.to_string(),
                model: row.model.text().trim().to_string(),
                base_url: row.base_url.text().trim().to_string(),
                command: row.command.text().trim().to_string(),
                ..row.original.clone()
            };
            if provider.kind == Kind::Command {
                entry.model.clear();
            }
            providers::resolve_entry(&name, &entry, "")?;
            let key = row.key.text().trim().to_string();
            if !key.is_empty() {
                let label = format!("Poppi Launcher: {} API key", provider.name);
                keys.push((providers::keyring_id(&name, &entry), label, key));
            }
            if row.default.is_active() {
                default = Some(name.clone());
            }
            models.insert(name, entry);
        }
        if models.is_empty() {
            return Err("Keep at least one model.".to_string());
        }
        let default = default
            .filter(|name| models.contains_key(name))
            .or_else(|| models.keys().next().cloned())
            .unwrap_or_default();
        Ok(EditedModels {
            models,
            default,
            keys,
        })
    }

    /// Clear typed keys once they are saved.
    pub fn keys_saved(&self) {
        for row in self.rows.borrow().iter() {
            row.key.set_text("");
            row.refresh_key_state();
        }
    }
}

/// A short label next to the model list explaining the columns.
pub fn legend(parent: &GtkBox) {
    let label = Label::new(Some("Default · name · provider · model"));
    label.set_halign(gtk::Align::Start);
    label.add_css_class("hint");
    parent.append(&label);
}
