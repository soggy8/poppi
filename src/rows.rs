//! Widgets for individual results, and the key hints that go with them.

use crate::alias::AliasTarget;
use crate::clipboard;
use crate::emoji_picker::Emoji;
use crate::markdown;
use crate::matching::{self, Matcher};
use crate::state::{ResultItem, Turn, TurnStatus};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Image, Label, ListBoxRow};

const ICON_SIZE: i32 = 32;
const SYMBOL_SIZE: i32 = 18;

/// What rows need to know besides their item.
pub struct RowContext<'a> {
    pub matcher: &'a Matcher,
    /// The text titles were matched against; matched characters are emphasised.
    pub highlight: &'a str,
    /// `#rrggbb` used for that emphasis.
    pub accent: &'a str,
}

fn text_label(text: &str, css_class: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class(css_class);
    label
}

/// Pango markup for `text` with the characters at `positions` emphasised.
pub fn highlight_markup(text: &str, positions: &[usize], accent: &str) -> String {
    let mut out = String::new();
    let mut open = false;
    for (index, c) in text.chars().enumerate() {
        let marked = positions.contains(&index);
        if marked && !open {
            out.push_str(&format!("<span foreground=\"{}\" weight=\"bold\">", accent));
        } else if !marked && open {
            out.push_str("</span>");
        }
        open = marked;
        out.push_str(&glib::markup_escape_text(c.encode_utf8(&mut [0; 4])));
    }
    if open {
        out.push_str("</span>");
    }
    out
}

fn title_label(text: &str, context: &RowContext, highlight: bool) -> Label {
    let label = text_label(text, "row-title");
    if highlight && !context.highlight.is_empty() {
        let positions = matching::highlight(context.matcher, context.highlight, text);
        if !positions.is_empty() {
            label.set_markup(&highlight_markup(text, &positions, context.accent));
        }
    }
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

fn row_content(icon: Image, title: Label, subtitle: Option<&str>) -> GtkBox {
    if icon.has_css_class("row-symbol") {
        let margin = (ICON_SIZE - SYMBOL_SIZE) / 2;
        icon.set_pixel_size(SYMBOL_SIZE);
        icon.set_margin_start(margin);
        icon.set_margin_end(margin);
    } else {
        icon.set_pixel_size(ICON_SIZE);
    }

    let text_box = GtkBox::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    text_box.append(&title);
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
    container.append(&icon);
    container.append(&text_box);
    container
}

/// The conversation: each question, then its answer rendered from markdown.
fn transcript_content(turns: &[Turn], model: &str) -> GtkBox {
    let container = GtkBox::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();

    for (index, turn) in turns.iter().enumerate() {
        if index > 0 {
            let gap = GtkBox::new(gtk::Orientation::Horizontal, 0);
            gap.add_css_class("chat-gap");
            container.append(&gap);
        }

        let question = text_label(&turn.question, "chat-question");
        question.set_ellipsize(gtk::pango::EllipsizeMode::None);
        question.set_wrap(true);
        question.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        container.append(&question);

        if !turn.text.is_empty() {
            let answer = Label::new(None);
            answer.set_xalign(0.0);
            answer.set_wrap(true);
            answer.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            if turn.status == TurnStatus::Failed {
                answer.set_text(&turn.text);
                answer.add_css_class("chat-error");
            } else {
                let markup = markdown::to_pango(turn.text.trim());
                // Never trust generated markup blindly: fall back to text.
                if gtk::pango::parse_markup(&markup, '\0').is_ok() {
                    answer.set_markup(&markup);
                } else {
                    answer.set_text(turn.text.trim());
                }
                answer.add_css_class("chat-answer");
            }
            container.append(&answer);
        }

        let status = match turn.status {
            TurnStatus::Thinking => Some(format!("Claude ({}) is thinking…", model)),
            TurnStatus::Answering => Some("…".to_string()),
            TurnStatus::Done | TurnStatus::Failed => None,
        };
        if let Some(status) = status {
            let line = GtkBox::new(gtk::Orientation::Horizontal, 8);
            if turn.status == TurnStatus::Thinking {
                let spinner = gtk::Spinner::new();
                spinner.start();
                line.append(&spinner);
            }
            line.append(&text_label(&status, "chat-status"));
            container.append(&line);
        }
    }
    container
}

pub fn list_row(item: &ResultItem, context: &RowContext) -> ListBoxRow {
    let row = ListBoxRow::new();
    let plain = |text: &str| title_label(text, context, false);
    let matched = |text: &str| title_label(text, context, true);

    let content = match item {
        ResultItem::App(app) => {
            let icon = match &app.icon {
                Some(icon) => Image::from_gicon(icon),
                None => themed("application-x-executable"),
            };
            row_content(icon, matched(&app.name), app.description.as_deref())
        }
        ResultItem::Window { window, icon } => {
            let icon = match icon {
                Some(icon) => Image::from_gicon(icon),
                None => symbolic("focus-windows-symbolic"),
            };
            row_content(icon, matched(&window.title), Some(&window.app_name))
        }
        ResultItem::File(file) => row_content(
            file_icon(&file.name, file.is_dir),
            matched(&file.name),
            Some(&file.parent_display()),
        ),
        ResultItem::System(action) => row_content(
            symbolic(action.icon()),
            matched(action.name()),
            Some(action.description()),
        ),
        ResultItem::Confirm(action) => row_content(
            symbolic("dialog-warning-symbolic"),
            plain(&format!("{} now?", action.name())),
            Some("Press Enter again to confirm, or keep typing to cancel"),
        ),
        ResultItem::Settings => row_content(
            symbolic("preferences-system-symbolic"),
            plain("Poppi Launcher Settings"),
            Some("Appearance, behavior, search, Claude, files and aliases"),
        ),
        ResultItem::Calc { expression, result } => {
            let title = plain(result);
            title.add_css_class("result");
            row_content(
                symbolic("accessories-calculator-symbolic"),
                title,
                Some(expression),
            )
        }
        ResultItem::Terminal(command) => row_content(
            symbolic("utilities-terminal-symbolic"),
            plain(command),
            Some("Run in terminal"),
        ),
        ResultItem::Search { engine, query } => row_content(
            symbolic("system-search-symbolic"),
            plain(query),
            Some(&format!("Search {}", engine.label())),
        ),
        ResultItem::Alias { name, target } => {
            let icon = match target {
                AliasTarget::Url(_) => "web-browser-symbolic",
                AliasTarget::Command(_) => "system-run-symbolic",
            };
            row_content(
                symbolic(icon),
                plain(target.text()),
                Some(&format!("Alias {}", name)),
            )
        }
        ResultItem::History(query) => {
            row_content(symbolic("document-open-recent-symbolic"), plain(query), None)
        }
        ResultItem::ClearHistory => row_content(
            symbolic("edit-clear-all-symbolic"),
            plain("Clear history"),
            Some("Forget all remembered queries"),
        ),
        ResultItem::Clipboard(text) => {
            let lines = text.lines().count();
            let detail = if lines > 1 {
                format!("{} lines, {} characters", lines, text.chars().count())
            } else {
                format!("{} characters", text.chars().count())
            };
            row_content(
                symbolic("edit-paste-symbolic"),
                plain(&clipboard::preview(text)),
                Some(&detail),
            )
        }
        ResultItem::Ask { question, model } => row_content(
            symbolic("dialog-question-symbolic"),
            plain(question),
            Some(&format!("Ask Claude ({})", model)),
        ),
        ResultItem::Transcript { turns, model } => {
            row.add_css_class("chat-row");
            row.set_selectable(false);
            transcript_content(turns, model)
        }
        ResultItem::Info(message) => {
            row.set_activatable(false);
            row.set_selectable(false);
            row.add_css_class("info-row");
            let title = plain(message);
            title.set_ellipsize(gtk::pango::EllipsizeMode::None);
            title.set_wrap(true);
            row_content(symbolic("dialog-information-symbolic"), title, None)
        }
        // Emojis are shown in the grid, never in the list.
        ResultItem::Emoji(emoji) => {
            row_content(symbolic("face-smile-symbolic"), plain(emoji.emoji), Some(emoji.name))
        }
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

/// Key hints for the footer: what the keys do for the selected item.
pub fn hints(item: Option<&ResultItem>, typing_path: bool, in_chat: bool) -> Vec<(&'static str, &'static str)> {
    let mut hints = match item {
        None => Vec::new(),
        Some(item) => match item {
            ResultItem::App(_) => vec![("↵", "Launch")],
            ResultItem::Window { .. } => vec![("↵", "Switch")],
            ResultItem::File(_) if typing_path => {
                vec![("↵", "Open"), ("Tab", "Complete"), ("Ctrl ↵", "Folder")]
            }
            ResultItem::File(_) => vec![("↵", "Open"), ("Ctrl ↵", "Folder")],
            ResultItem::Calc { .. } | ResultItem::Clipboard(_) => vec![("↵", "Copy")],
            ResultItem::Emoji(_) => vec![("↵", "Insert"), ("←↑↓→", "Move")],
            ResultItem::Terminal(_) => vec![("↵", "Run")],
            ResultItem::Search { .. } => vec![("↵", "Search")],
            ResultItem::Alias { target: AliasTarget::Url(_), .. } => vec![("↵", "Open")],
            ResultItem::Alias { .. } => vec![("↵", "Run")],
            ResultItem::History(_) => vec![("↵", "Recall")],
            ResultItem::ClearHistory => vec![("↵", "Clear")],
            ResultItem::System(action) => vec![("↵", action.name())],
            ResultItem::Confirm(_) => vec![("↵", "Confirm")],
            ResultItem::Ask { .. } => vec![("↵", "Ask Claude")],
            ResultItem::Transcript { .. } => vec![("↵", "Copy answer"), ("↑↓", "Scroll")],
            ResultItem::Settings => vec![("↵", "Open")],
            ResultItem::Info(_) => Vec::new(),
        },
    };
    if in_chat {
        hints.push(("⌫", "Leave chat"));
    }
    hints.push(("Esc", "Close"));
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_markup_wraps_runs_and_escapes() {
        assert_eq!(
            highlight_markup("Firefox", &[0, 1, 2, 3], "#4a9eff"),
            "<span foreground=\"#4a9eff\" weight=\"bold\">Fire</span>fox"
        );
        assert_eq!(
            highlight_markup("a<b>&c", &[1, 5], "red"),
            "a<span foreground=\"red\" weight=\"bold\">&lt;</span>b&gt;&amp;<span foreground=\"red\" weight=\"bold\">c</span>"
        );
        assert_eq!(highlight_markup("plain", &[], "red"), "plain");
    }

    #[test]
    fn hints_always_end_with_close() {
        assert_eq!(hints(None, false, false), vec![("Esc", "Close")]);
        let file = ResultItem::Info("x".into());
        assert_eq!(hints(Some(&file), false, true), vec![("⌫", "Leave chat"), ("Esc", "Close")]);
        let calc = ResultItem::Calc { expression: "1+1".into(), result: "2".into() };
        assert_eq!(hints(Some(&calc), false, false)[0], ("↵", "Copy"));
    }
}
