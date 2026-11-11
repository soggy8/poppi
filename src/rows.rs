//! Widgets for individual results.

use crate::alias::AliasTarget;
use crate::emoji_picker::Emoji;
use crate::state::{Ask, AskStatus, ResultItem};
use gtk::gio;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Image, Label, ListBoxRow};

const ICON_SIZE: i32 = 32;
const SYMBOL_SIZE: i32 = 20;

fn text_label(text: &str, css_class: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class(css_class);
    label
}

fn themed(icon_name: &str) -> Image {
    Image::from_icon_name(icon_name)
}

/// A monochrome glyph for rows that are actions rather than things. Drawn
/// smaller and dimmer than app icons, inside the same footprint.
fn symbolic(icon_name: &str) -> Image {
    let image = Image::from_icon_name(icon_name);
    image.add_css_class("row-symbol");
    image
}

fn file_icon(name: &str, is_dir: bool) -> Image {
    if is_dir {
        return themed("folder");
    }
    let (content_type, _uncertain) = gio::content_type_guess(Some(name), &[]);
    Image::from_gicon(&gio::content_type_get_icon(&content_type))
}

/// Icon, title over an optional subtitle, and on the right a hint naming
/// what Enter does (only visible on the selected row).
fn row_content(icon: Image, title: &str, subtitle: Option<&str>, hint: Option<&str>) -> GtkBox {
    if icon.has_css_class("row-symbol") {
        let margin = (ICON_SIZE - SYMBOL_SIZE) / 2;
        icon.set_pixel_size(SYMBOL_SIZE);
        icon.set_margin_start(margin);
        icon.set_margin_end(margin);
    } else {
        icon.set_pixel_size(ICON_SIZE);
    }
    icon.add_css_class("row-icon");

    let text_box = GtkBox::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(1)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    text_box.append(&text_label(title, "row-title"));
    if let Some(subtitle) = subtitle.filter(|s| !s.is_empty()) {
        let subtitle = text_label(subtitle, "row-subtitle");
        // Paths and long titles keep both ends readable.
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        text_box.append(&subtitle);
    }

    let container = GtkBox::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    container.add_css_class("row-content");
    container.append(&icon);
    container.append(&text_box);
    if let Some(hint) = hint {
        let hint = Label::new(Some(hint));
        hint.add_css_class("row-hint");
        hint.set_valign(gtk::Align::Center);
        container.append(&hint);
    }
    container
}

/// Claude's answer: a small header, then wrapped text that can grow tall
/// (the list scrolls), then what Enter does once the answer is complete.
fn answer_content(ask: &Ask) -> GtkBox {
    let container = GtkBox::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    container.add_css_class("answer-content");

    let status = match ask.status {
        AskStatus::Thinking => "thinking…",
        AskStatus::Answering => "answering…",
        AskStatus::Done => "Enter to copy",
        AskStatus::Failed => "failed",
    };
    let header = GtkBox::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let icon = symbolic("dialog-question-symbolic");
    icon.set_pixel_size(14);
    header.append(&icon);
    let title = text_label(&format!("Claude · {}", ask.model), "answer-header");
    title.set_hexpand(true);
    header.append(&title);
    header.append(&text_label(status, "answer-header"));
    container.append(&header);

    if !ask.text.is_empty() {
        let text = Label::new(Some(&ask.text));
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.add_css_class(if ask.status == AskStatus::Failed {
            "answer-error"
        } else {
            "answer-text"
        });
        container.append(&text);
    }
    container
}

pub fn list_row(item: &ResultItem) -> ListBoxRow {
    let row = ListBoxRow::new();

    let content = match item {
        ResultItem::Ask { question, model } => row_content(
            symbolic("dialog-question-symbolic"),
            question,
            Some(&format!("Ask Claude ({})", model)),
            Some("Ask"),
        ),
        ResultItem::Answer(ask) => {
            row.add_css_class("answer-row");
            answer_content(ask)
        }
        ResultItem::App(app) => {
            let icon = match &app.icon {
                Some(icon) => Image::from_gicon(icon),
                None => themed("application-x-executable"),
            };
            row_content(icon, &app.name, app.description.as_deref(), Some("Launch"))
        }
        ResultItem::Window { window, icon } => {
            let icon = match icon {
                Some(icon) => Image::from_gicon(icon),
                None => symbolic("focus-windows-symbolic"),
            };
            row_content(icon, &window.title, Some(&window.app_name), Some("Switch"))
        }
        ResultItem::Settings => row_content(
            symbolic("preferences-system-symbolic"),
            "Poppi Launcher Settings",
            Some("Theme, behavior, search, files and calculator options"),
            Some("Open"),
        ),
        ResultItem::Calc { expression, result } => row_content(
            symbolic("accessories-calculator-symbolic"),
            result,
            Some(&format!("{} =", expression)),
            Some("Copy"),
        ),
        ResultItem::Terminal(command) => row_content(
            symbolic("utilities-terminal-symbolic"),
            command,
            Some("Run in terminal"),
            Some("Run"),
        ),
        ResultItem::Search { engine, query } => row_content(
            symbolic("system-search-symbolic"),
            query,
            Some(&format!("Search {}", engine.label())),
            Some("Search"),
        ),
        ResultItem::File(file) => row_content(
            file_icon(&file.name, file.is_dir),
            &file.name,
            Some(&file.parent_display()),
            Some("Open"),
        ),
        ResultItem::Alias { name, target } => {
            let (icon, hint) = match target {
                AliasTarget::Url(_) => ("web-browser-symbolic", "Open"),
                AliasTarget::Command(_) => ("system-run-symbolic", "Run"),
            };
            row_content(
                symbolic(icon),
                target.text(),
                Some(&format!("Alias {}", name)),
                Some(hint),
            )
        }
        ResultItem::History(query) => row_content(
            symbolic("document-open-recent-symbolic"),
            query,
            None,
            Some("Recall"),
        ),
        ResultItem::ClearHistory => row_content(
            symbolic("edit-clear-all-symbolic"),
            "Clear history",
            Some("Forget all remembered queries"),
            Some("Clear"),
        ),
        ResultItem::Info(message) => {
            row.set_activatable(false);
            row.set_selectable(false);
            row.add_css_class("info-row");
            row_content(symbolic("dialog-information-symbolic"), message, None, None)
        }
        // Emojis are shown in the grid, never in the list.
        ResultItem::Emoji(emoji) => row_content(symbolic("face-smile-symbolic"), emoji.emoji, Some(emoji.name), None),
    };

    row.set_child(Some(&content));
    row
}

pub fn emoji_button(emoji: &Emoji) -> Button {
    let button = Button::builder().label(emoji.emoji).has_frame(false).build();
    button.add_css_class("emoji-button");
    button.set_tooltip_text(Some(emoji.name));
    button.set_focus_on_click(false);
    button
}
