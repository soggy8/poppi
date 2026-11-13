//! Widgets for individual results, reusable row slots, and footer key hints.

use crate::alias::AliasTarget;
use crate::clipboard;
use crate::emoji_picker::Emoji;
use crate::markdown::{self, Block};
use crate::matching::{self, Matcher};
use crate::scripts::ScriptItemAction;
use crate::state::{ResultItem, Turn, TurnStatus};
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Image, Label, ListBoxRow};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

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

enum IconSpec<'a> {
    GIcon(&'a gio::Icon),
    Themed(&'static str),
    Symbolic(&'a str),
    Swatch((u8, u8, u8, u8)),
    File { name: &'a str, is_dir: bool },
}

/// Build a 32x32 rounded color swatch texture with checkerboard backing for alpha.
fn swatch_texture((r, g, b, a): (u8, u8, u8, u8)) -> gdk::MemoryTexture {
    const SIZE: i32 = ICON_SIZE;
    const RADIUS: f32 = 7.0;
    const PAD: f32 = 2.0;

    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    let min_c = PAD + RADIUS;
    let max_c = (SIZE as f32) - PAD - RADIUS;
    let af = f32::from(a) / 255.0;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            if fx < PAD || fy < PAD || fx > (SIZE as f32 - PAD) || fy > (SIZE as f32 - PAD) {
                continue;
            }
            let dx = if fx < min_c {
                min_c - fx
            } else if fx > max_c {
                fx - max_c
            } else {
                0.0
            };
            let dy = if fy < min_c {
                min_c - fy
            } else if fy > max_c {
                fy - max_c
            } else {
                0.0
            };
            let dist = (dx * dx + dy * dy).sqrt();
            let edge_alpha = (RADIUS + 0.5 - dist).clamp(0.0, 1.0);
            if edge_alpha <= 0.0 {
                continue;
            }

            let checker = if ((x / 4) + (y / 4)) % 2 == 0 {
                205.0
            } else {
                145.0
            };
            let mut cr = f32::from(r) * af + checker * (1.0 - af);
            let mut cg = f32::from(g) * af + checker * (1.0 - af);
            let mut cb = f32::from(b) * af + checker * (1.0 - af);

            // Subtle border so very light or dark swatches stand out cleanly.
            let on_border = dist > RADIUS - 1.1
                || fx < PAD + 1.1
                || fy < PAD + 1.1
                || fx > (SIZE as f32 - PAD - 1.1)
                || fy > (SIZE as f32 - PAD - 1.1);
            if on_border {
                cr = cr * 0.78 + 128.0 * 0.22;
                cg = cg * 0.78 + 128.0 * 0.22;
                cb = cb * 0.78 + 128.0 * 0.22;
            }

            let idx = ((y * SIZE + x) * 4) as usize;
            pixels[idx] = cr.round() as u8;
            pixels[idx + 1] = cg.round() as u8;
            pixels[idx + 2] = cb.round() as u8;
            pixels[idx + 3] = (edge_alpha * 255.0).round() as u8;
        }
    }

    let bytes = glib::Bytes::from_owned(pixels);
    gdk::MemoryTexture::new(
        SIZE,
        SIZE,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        (SIZE * 4) as usize,
    )
}

/// A reusable standard `ListBoxRow` whose icon, title and subtitle are updated
/// in place instead of destroying and recreating widgets on every keystroke.
pub struct RowSlot {
    pub row: ListBoxRow,
    icon: Image,
    title: Label,
    subtitle: Label,
}

impl RowSlot {
    pub fn new() -> Self {
        let icon = Image::new();
        let title = text_label("", "row-title");
        let subtitle = text_label("", "row-subtitle");
        // Paths and long titles keep both ends readable.
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);

        let text_box = GtkBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        text_box.append(&title);
        text_box.append(&subtitle);

        let container = GtkBox::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .build();
        container.append(&icon);
        container.append(&text_box);

        let row = ListBoxRow::new();
        row.set_child(Some(&container));
        Self {
            row,
            icon,
            title,
            subtitle,
        }
    }

    fn apply_icon(&self, spec: IconSpec<'_>) {
        let is_symbol = matches!(spec, IconSpec::Symbolic(_));
        if is_symbol {
            let margin = (ICON_SIZE - SYMBOL_SIZE) / 2;
            self.icon.add_css_class("row-symbol");
            self.icon.set_pixel_size(SYMBOL_SIZE);
            self.icon.set_margin_start(margin);
            self.icon.set_margin_end(margin);
        } else {
            self.icon.remove_css_class("row-symbol");
            self.icon.set_pixel_size(ICON_SIZE);
            self.icon.set_margin_start(0);
            self.icon.set_margin_end(0);
        }

        match spec {
            IconSpec::GIcon(gicon) => self.icon.set_from_gicon(gicon),
            IconSpec::Themed(name) | IconSpec::Symbolic(name) => {
                self.icon.set_icon_name(Some(name))
            }
            IconSpec::Swatch(rgba) => {
                let texture = swatch_texture(rgba);
                self.icon.set_paintable(Some(&texture));
            }
            IconSpec::File { name, is_dir } => {
                if is_dir {
                    self.icon.set_icon_name(Some("folder"));
                } else {
                    let (content_type, _uncertain) = gio::content_type_guess(Some(name), &[]);
                    self.icon
                        .set_from_gicon(&gio::content_type_get_icon(&content_type));
                }
            }
        }
    }

    fn apply_title(
        &self,
        text: &str,
        context: &RowContext,
        highlight: bool,
        wrap: bool,
        is_result: bool,
    ) {
        if is_result {
            self.title.add_css_class("result");
        } else {
            self.title.remove_css_class("result");
        }
        if wrap {
            self.title.set_ellipsize(gtk::pango::EllipsizeMode::None);
            self.title.set_wrap(true);
        } else {
            self.title.set_wrap(false);
            self.title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        }

        if highlight && !context.highlight.is_empty() {
            let positions = matching::highlight(context.matcher, context.highlight, text);
            if !positions.is_empty() {
                self.title
                    .set_markup(&highlight_markup(text, &positions, context.accent));
                return;
            }
        }
        self.title.set_text(text);
    }

    fn apply_subtitle(&self, subtitle: Option<&str>) {
        match subtitle.filter(|s| !s.is_empty()) {
            Some(text) => {
                self.subtitle.set_text(text);
                self.subtitle.set_visible(true);
            }
            None => self.subtitle.set_visible(false),
        }
    }

    pub fn update(&self, item: &ResultItem, context: &RowContext) {
        let is_info = matches!(item, ResultItem::Info(_));
        self.row.set_activatable(!is_info);
        self.row.set_selectable(!is_info);
        if is_info {
            self.row.add_css_class("info-row");
        } else {
            self.row.remove_css_class("info-row");
        }

        match item {
            ResultItem::App(app) => {
                let icon = match &app.icon {
                    Some(icon) => IconSpec::GIcon(icon),
                    None => IconSpec::Themed("application-x-executable"),
                };
                self.apply_icon(icon);
                self.apply_title(&app.name, context, true, false, false);
                self.apply_subtitle(app.description.as_deref());
            }
            ResultItem::Window { window, icon } => {
                let icon = match icon {
                    Some(icon) => IconSpec::GIcon(icon),
                    None => IconSpec::Symbolic("focus-windows-symbolic"),
                };
                self.apply_icon(icon);
                self.apply_title(&window.title, context, true, false, false);
                self.apply_subtitle(Some(&window.app_name));
            }
            ResultItem::File(file) => {
                self.apply_icon(IconSpec::File {
                    name: &file.name,
                    is_dir: file.is_dir,
                });
                self.apply_title(&file.name, context, true, false, false);
                let parent = file.parent_display();
                self.apply_subtitle(Some(&parent));
            }
            ResultItem::ContentMatch(m) => {
                self.apply_icon(IconSpec::File {
                    name: &m.file.name,
                    is_dir: false,
                });
                self.apply_title(&m.line_preview, context, true, false, false);
                let sub = format!(
                    "{}:{}  ·  {}",
                    m.file.name,
                    m.line_number,
                    m.file.parent_display()
                );
                self.apply_subtitle(Some(&sub));
            }
            ResultItem::System(action) => {
                self.apply_icon(IconSpec::Symbolic(action.icon()));
                self.apply_title(action.name(), context, true, false, false);
                self.apply_subtitle(Some(action.description()));
            }
            ResultItem::Confirm(action) => {
                self.apply_icon(IconSpec::Symbolic("dialog-warning-symbolic"));
                let title = format!("{} now?", action.name());
                self.apply_title(&title, context, false, false, false);
                self.apply_subtitle(Some(
                    "Press Enter again to confirm, or keep typing to cancel",
                ));
            }
            ResultItem::Setup => {
                self.apply_icon(IconSpec::Symbolic("emblem-system-symbolic"));
                self.apply_title("Set up Poppi", context, false, false, false);
                self.apply_subtitle(Some("AI service and key, keyboard shortcut, voice"));
            }
            ResultItem::Settings => {
                self.apply_icon(IconSpec::Symbolic("preferences-system-symbolic"));
                self.apply_title("Poppi Launcher Settings", context, false, false, false);
                self.apply_subtitle(Some(
                    "Appearance, behavior, search, AI models, files and aliases",
                ));
            }
            ResultItem::Calc {
                expression,
                result,
                swatch,
            } => {
                match swatch {
                    Some(rgba) => self.apply_icon(IconSpec::Swatch(*rgba)),
                    None => self.apply_icon(IconSpec::Symbolic("accessories-calculator-symbolic")),
                }
                self.apply_title(result, context, false, false, true);
                self.apply_subtitle(Some(expression));
            }
            ResultItem::Proposed(action) => {
                let (icon, title, subtitle) = proposed_row(action);
                self.apply_icon(IconSpec::Symbolic(icon));
                self.apply_title(&title, context, false, false, false);
                self.apply_subtitle(Some(subtitle));
            }
            ResultItem::Terminal(command) => {
                self.apply_icon(IconSpec::Symbolic("utilities-terminal-symbolic"));
                self.apply_title(command, context, false, false, false);
                self.apply_subtitle(Some("Run in terminal"));
            }
            ResultItem::Search { engine, query } => {
                self.apply_icon(IconSpec::Symbolic("system-search-symbolic"));
                self.apply_title(query, context, false, false, false);
                let sub = format!("Search {}", engine.label());
                self.apply_subtitle(Some(&sub));
            }
            ResultItem::Alias { name, target } => {
                let icon = match target {
                    AliasTarget::Url(_) => "web-browser-symbolic",
                    AliasTarget::Command(_) => "system-run-symbolic",
                };
                self.apply_icon(IconSpec::Symbolic(icon));
                self.apply_title(target.text(), context, false, false, false);
                let sub = format!("Alias {}", name);
                self.apply_subtitle(Some(&sub));
            }
            ResultItem::ScriptRun { script, args } => {
                self.apply_icon(IconSpec::Symbolic(&script.icon));
                let title = if args.is_empty() {
                    script.name.clone()
                } else {
                    format!("{} · {}", script.name, args)
                };
                self.apply_title(&title, context, args.is_empty(), false, false);
                let fallback = format!("Script {}", script.keyword);
                self.apply_subtitle(Some(script.description.as_deref().unwrap_or(&fallback)));
            }
            ResultItem::ScriptOutput(item) => {
                self.apply_icon(IconSpec::Symbolic(&item.icon));
                self.apply_title(&item.title, context, false, false, false);
                self.apply_subtitle(item.subtitle.as_deref());
            }
            ResultItem::ItemAction {
                title,
                subtitle,
                icon,
                ..
            } => {
                self.apply_icon(IconSpec::Symbolic(icon));
                self.apply_title(title, context, true, false, false);
                self.apply_subtitle(Some(subtitle));
            }
            ResultItem::History(query) => {
                self.apply_icon(IconSpec::Symbolic("document-open-recent-symbolic"));
                self.apply_title(query, context, false, false, false);
                self.apply_subtitle(None);
            }
            ResultItem::ClearHistory => {
                self.apply_icon(IconSpec::Symbolic("edit-clear-all-symbolic"));
                self.apply_title("Clear history", context, false, false, false);
                self.apply_subtitle(Some("Forget all remembered queries"));
            }
            ResultItem::Clipboard(entry) => {
                let lines = entry.text.lines().count();
                let counts = if lines > 1 {
                    format!("{} lines, {} characters", lines, entry.text.chars().count())
                } else {
                    format!("{} characters", entry.text.chars().count())
                };
                let detail = if entry.pinned {
                    format!("Pinned · {}", counts)
                } else {
                    counts
                };
                let icon = if entry.pinned {
                    "view-pin-symbolic"
                } else {
                    "edit-paste-symbolic"
                };
                self.apply_icon(IconSpec::Symbolic(icon));
                let preview = clipboard::preview(&entry.text);
                self.apply_title(&preview, context, true, false, false);
                self.apply_subtitle(Some(&detail));
            }
            ResultItem::Ask {
                question, label, ..
            } => {
                self.apply_icon(IconSpec::Symbolic("dialog-question-symbolic"));
                self.apply_title(question, context, false, false, false);
                let sub = format!("Ask {}", label);
                self.apply_subtitle(Some(&sub));
            }
            ResultItem::Info(message) => {
                self.apply_icon(IconSpec::Symbolic("dialog-information-symbolic"));
                self.apply_title(message, context, false, true, false);
                self.apply_subtitle(None);
            }
            ResultItem::Emoji(emoji) => {
                self.apply_icon(IconSpec::Symbolic("face-smile-symbolic"));
                self.apply_title(emoji.emoji, context, false, false, false);
                self.apply_subtitle(Some(emoji.name));
            }
            ResultItem::Transcript { .. } => {}
        }
    }
}

// ---------- In-place streaming AI conversation ----------

enum RenderedBlock {
    Prose {
        widget: Label,
    },
    Code {
        widget: GtkBox,
        lang_label: Label,
        code_label: Label,
        code: Rc<RefCell<String>>,
    },
    Error {
        widget: Label,
    },
}

impl RenderedBlock {
    fn widget(&self) -> &gtk::Widget {
        match self {
            RenderedBlock::Prose { widget } | RenderedBlock::Error { widget } => {
                widget.upcast_ref()
            }
            RenderedBlock::Code { widget, .. } => widget.upcast_ref(),
        }
    }

    fn matches_markdown(&self, block: &Block) -> bool {
        matches!(
            (self, block),
            (RenderedBlock::Prose { .. }, Block::Prose(_))
                | (RenderedBlock::Code { .. }, Block::Code { .. })
        )
    }
}

fn set_pango_or_text(label: &Label, markup: &str, raw_fallback: &str) {
    // Never trust generated markup blindly: fall back to text.
    if gtk::pango::parse_markup(markup, '\0').is_ok() {
        label.set_markup(markup);
    } else {
        label.set_text(raw_fallback);
    }
}

fn build_prose_block(markup: &str, raw_fallback: &str) -> RenderedBlock {
    let label = Label::new(None);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.add_css_class("chat-answer");
    set_pango_or_text(&label, markup, raw_fallback);
    RenderedBlock::Prose { widget: label }
}

fn build_code_block(language: Option<&str>, initial_code: &str) -> RenderedBlock {
    let container = GtkBox::new(gtk::Orientation::Vertical, 4);
    container.add_css_class("chat-code");

    let header = GtkBox::new(gtk::Orientation::Horizontal, 6);
    let lang_label = text_label(language.unwrap_or("code"), "chat-code-lang");
    lang_label.set_hexpand(true);

    let copy_btn = Button::from_icon_name("edit-copy-symbolic");
    copy_btn.add_css_class("chat-code-copy");
    copy_btn.set_has_frame(false);
    copy_btn.set_focus_on_click(false);
    copy_btn.set_can_focus(false);
    copy_btn.set_tooltip_text(Some("Copy code"));

    let shared_code = Rc::new(RefCell::new(initial_code.to_string()));
    let code_for_click = shared_code.clone();
    copy_btn.connect_clicked(move |btn| {
        let text = code_for_click.borrow().clone();
        if crate::utils::copy_to_clipboard(&text).is_err() {
            WidgetExt::display(btn).clipboard().set_text(&text);
        }
        btn.set_icon_name("object-select-symbolic");
        btn.set_tooltip_text(Some("Copied"));
        let btn_reset = btn.clone();
        glib::timeout_add_local_once(Duration::from_millis(1200), move || {
            btn_reset.set_icon_name("edit-copy-symbolic");
            btn_reset.set_tooltip_text(Some("Copy code"));
        });
    });

    header.append(&lang_label);
    header.append(&copy_btn);

    let code_label = Label::new(Some(initial_code));
    code_label.set_xalign(0.0);
    code_label.set_wrap(true);
    code_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    code_label.add_css_class("chat-code-body");

    container.append(&header);
    container.append(&code_label);

    RenderedBlock::Code {
        widget: container,
        lang_label,
        code_label,
        code: shared_code,
    }
}

struct TurnView {
    question: String,
    model: String,
    answer_box: GtkBox,
    blocks: Vec<RenderedBlock>,
    status_line: GtkBox,
    spinner: gtk::Spinner,
    status_label: Label,
}

impl TurnView {
    fn new(parent: &GtkBox, turn: &Turn, prev_model: Option<&str>) -> Self {
        if prev_model.is_some() {
            let gap = GtkBox::new(gtk::Orientation::Horizontal, 0);
            gap.add_css_class("chat-gap");
            parent.append(&gap);
        }
        if prev_model.is_some_and(|prev| prev != turn.model) {
            parent.append(&text_label(&turn.label, "chat-model"));
        }

        let question = text_label(&turn.question, "chat-question");
        question.set_ellipsize(gtk::pango::EllipsizeMode::None);
        question.set_wrap(true);
        question.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        parent.append(&question);

        let answer_box = GtkBox::new(gtk::Orientation::Vertical, 6);
        answer_box.set_visible(false);
        parent.append(&answer_box);

        let status_line = GtkBox::new(gtk::Orientation::Horizontal, 8);
        let spinner = gtk::Spinner::new();
        let status_label = text_label("", "chat-status");
        status_line.append(&spinner);
        status_line.append(&status_label);
        parent.append(&status_line);

        let mut view = Self {
            question: turn.question.clone(),
            model: turn.model.clone(),
            answer_box,
            blocks: Vec::new(),
            status_line,
            spinner,
            status_label,
        };
        view.update(turn);
        view
    }

    fn clear_blocks(&mut self) {
        for block in self.blocks.drain(..) {
            self.answer_box.remove(block.widget());
        }
    }

    fn update(&mut self, turn: &Turn) {
        // Proposed actions are shown as rows below, not as text.
        let shown = if turn.status == TurnStatus::Failed {
            turn.text.trim().to_string()
        } else {
            crate::actions::split(&turn.text).0
        };
        let trimmed = shown.as_str();
        if trimmed.is_empty() {
            self.clear_blocks();
            self.answer_box.set_visible(false);
        } else if turn.status == TurnStatus::Failed {
            self.answer_box.set_visible(true);
            if let [RenderedBlock::Error { widget }] = self.blocks.as_slice() {
                widget.set_text(&turn.text);
            } else {
                self.clear_blocks();
                let label = Label::new(Some(&turn.text));
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                label.add_css_class("chat-error");
                self.answer_box.append(&label);
                self.blocks.push(RenderedBlock::Error { widget: label });
            }
        } else {
            let desired = markdown::blocks(trimmed);
            self.answer_box.set_visible(!desired.is_empty());

            let common = self
                .blocks
                .iter()
                .zip(&desired)
                .take_while(|(rendered, block)| rendered.matches_markdown(block))
                .count();
            for stale in self.blocks.drain(common..) {
                self.answer_box.remove(stale.widget());
            }

            for (rendered, block) in self.blocks.iter().zip(&desired[..common]) {
                match (rendered, block) {
                    (RenderedBlock::Prose { widget }, Block::Prose(markup)) => {
                        set_pango_or_text(widget, markup, trimmed);
                    }
                    (
                        RenderedBlock::Code {
                            lang_label,
                            code_label,
                            code: shared,
                            ..
                        },
                        Block::Code { language, code },
                    ) => {
                        lang_label.set_text(language.as_deref().unwrap_or("code"));
                        code_label.set_text(code);
                        *shared.borrow_mut() = code.clone();
                    }
                    _ => {}
                }
            }

            for block in &desired[common..] {
                let rendered = match block {
                    Block::Prose(markup) => build_prose_block(markup, trimmed),
                    Block::Code { language, code } => build_code_block(language.as_deref(), code),
                };
                self.answer_box.append(rendered.widget());
                self.blocks.push(rendered);
            }
        }

        match turn.status {
            TurnStatus::Thinking => {
                self.spinner.set_visible(true);
                self.spinner.start();
                self.status_label
                    .set_text(&format!("{} is thinking…", turn.label));
                self.status_line.set_visible(true);
            }
            TurnStatus::Answering => {
                self.spinner.stop();
                self.spinner.set_visible(false);
                self.status_label.set_text("…");
                self.status_line.set_visible(true);
            }
            TurnStatus::Done | TurnStatus::Failed => {
                self.spinner.stop();
                self.spinner.set_visible(false);
                self.status_line.set_visible(false);
            }
        }
    }
}

/// Persistent view of the AI conversation: streaming updates touch only the
/// active turn's widgets instead of rebuilding the whole list.
pub struct TranscriptView {
    pub row: ListBoxRow,
    container: GtkBox,
    turns: Vec<TurnView>,
}

impl TranscriptView {
    pub fn new() -> Self {
        let container = GtkBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let row = ListBoxRow::new();
        row.add_css_class("chat-row");
        row.set_selectable(false);
        row.set_child(Some(&container));
        Self {
            row,
            container,
            turns: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        for turn in &self.turns {
            turn.spinner.stop();
        }
        self.turns.clear();
        while let Some(child) = self.container.first_child() {
            self.container.remove(&child);
        }
    }

    pub fn sync(&mut self, turns: &[Turn]) {
        let prefix_matches = turns.len() >= self.turns.len()
            && self
                .turns
                .iter()
                .zip(turns)
                .all(|(view, turn)| view.question == turn.question && view.model == turn.model);
        if !prefix_matches {
            self.clear();
        }

        for (index, turn) in turns.iter().enumerate() {
            if index < self.turns.len() {
                self.turns[index].update(turn);
            } else {
                let prev_model = index.checked_sub(1).map(|i| turns[i].model.as_str());
                self.turns
                    .push(TurnView::new(&self.container, turn, prev_model));
            }
        }
    }
}

/// Icon, title and subtitle for an action the AI proposed.
fn proposed_row(action: &crate::actions::Action) -> (&'static str, String, &'static str) {
    use crate::actions::Action;
    const SUGGESTED: &str = "Suggested by the AI · Enter to run";
    match action {
        Action::Open(target) if target.contains("://") => (
            "web-browser-symbolic",
            format!("Open {}", target),
            SUGGESTED,
        ),
        Action::Open(target) => (
            "folder-open-symbolic",
            format!("Open {}", target),
            SUGGESTED,
        ),
        Action::FindFiles(query) => (
            "edit-find-symbolic",
            format!("Find files: {}", query),
            SUGGESTED,
        ),
        Action::SwitchWindow(name) => (
            "preferences-system-windows-symbolic",
            format!("Switch to {}", name),
            SUGGESTED,
        ),
        Action::Copy(text) => (
            "edit-copy-symbolic",
            format!("Copy “{}”", crate::clipboard::preview(text)),
            SUGGESTED,
        ),
        Action::OpenApp(name) => (
            "application-x-executable-symbolic",
            format!("Open {}", name),
            "No installed app has that name · Enter searches for it",
        ),
        // These are shown with their own kind of row.
        Action::WebSearch(query) | Action::Run(query) => {
            ("system-run-symbolic", query.clone(), SUGGESTED)
        }
        Action::System(action) => ("system-run-symbolic", action.name().to_string(), SUGGESTED),
    }
}

// ---------- Emoji buttons and footer hints ----------

pub fn emoji_button(emoji: &Emoji) -> Button {
    let button = Button::builder()
        .label(emoji.emoji)
        .has_frame(false)
        .build();
    button.add_css_class("emoji-button");
    button.set_tooltip_text(Some(emoji.name));
    button.set_focus_on_click(false);
    button
}

pub fn update_emoji_button(button: &Button, emoji: &Emoji) {
    button.set_label(emoji.emoji);
    button.set_tooltip_text(Some(emoji.name));
}

pub struct HintSlot {
    pub key: Label,
    pub action: Label,
}

impl HintSlot {
    pub fn new(parent: &GtkBox) -> Self {
        let key = Label::new(None);
        key.add_css_class("key");
        let action = Label::new(None);
        action.set_margin_end(6);
        parent.append(&key);
        parent.append(&action);
        Self { key, action }
    }

    pub fn set(&self, key_text: &str, action_text: &str) {
        self.key.set_text(key_text);
        self.action.set_text(action_text);
        self.key.set_visible(true);
        self.action.set_visible(true);
    }

    pub fn hide(&self) {
        self.key.set_visible(false);
        self.action.set_visible(false);
    }
}

fn latest_done_has_code(turns: &[Turn]) -> bool {
    turns
        .iter()
        .rev()
        .find(|turn| turn.status == TurnStatus::Done)
        .and_then(|turn| markdown::last_code_block(&turn.text))
        .is_some()
}

/// Key hints for the footer: what the keys do for the selected item.
pub fn hints(
    item: Option<&ResultItem>,
    typing_path: bool,
    in_chat: bool,
    chat_busy: bool,
    in_actions: bool,
) -> Vec<(&'static str, &'static str)> {
    if in_actions {
        let mut h = match item {
            Some(ResultItem::ItemAction { .. }) => vec![("↵", "Run")],
            _ => Vec::new(),
        };
        h.push(("Esc", "Back"));
        return h;
    }

    let mut hints = match item {
        None => Vec::new(),
        Some(item) => match item {
            ResultItem::App(_) => vec![("↵", "Launch"), ("Ctrl+K", "Actions")],
            ResultItem::Proposed(_) => vec![("↵", "Run")],
            ResultItem::Window { .. } => {
                vec![("↵", "Switch"), ("Ctrl+W", "Close"), ("Ctrl+K", "Actions")]
            }
            ResultItem::File(_) if typing_path => {
                vec![("↵", "Open"), ("Tab", "Complete"), ("Ctrl+K", "Actions")]
            }
            ResultItem::File(_) | ResultItem::ContentMatch(_) => {
                vec![("↵", "Open"), ("Ctrl ↵", "Folder"), ("Ctrl+K", "Actions")]
            }
            ResultItem::Calc { .. } | ResultItem::Clipboard(_) => {
                vec![("↵", "Copy"), ("Ctrl+K", "Actions")]
            }
            ResultItem::Emoji(_) => vec![("↵", "Insert"), ("←↑↓→", "Move")],
            ResultItem::Terminal(_) | ResultItem::ScriptRun { .. } => vec![("↵", "Run")],
            ResultItem::ScriptOutput(item) => match item.action {
                ScriptItemAction::Copy(_) => vec![("↵", "Copy")],
                ScriptItemAction::Url(_) => vec![("↵", "Open")],
                _ => vec![("↵", "Run")],
            },
            ResultItem::ItemAction { .. } => vec![("↵", "Run")],
            ResultItem::Search { .. } => vec![("↵", "Search")],
            ResultItem::Alias {
                target: AliasTarget::Url(_),
                ..
            } => vec![("↵", "Open")],
            ResultItem::Alias { .. } => vec![("↵", "Run")],
            ResultItem::History(_) => vec![("↵", "Recall")],
            ResultItem::ClearHistory => vec![("↵", "Clear")],
            ResultItem::System(action) => vec![("↵", action.name())],
            ResultItem::Confirm(_) => vec![("↵", "Confirm")],
            ResultItem::Ask { .. } => vec![("↵", "Ask")],
            ResultItem::Transcript { turns } if chat_busy => {
                let _ = turns;
                vec![("↑↓", "Scroll")]
            }
            ResultItem::Transcript { turns } if latest_done_has_code(turns) => {
                vec![
                    ("↵", "Copy"),
                    ("Ctrl+Shift+C", "Copy code"),
                    ("↑↓", "Scroll"),
                ]
            }
            ResultItem::Transcript { .. } => vec![("↵", "Copy answer"), ("↑↓", "Scroll")],
            ResultItem::Settings | ResultItem::Setup => vec![("↵", "Open")],
            ResultItem::Info(_) => Vec::new(),
        },
    };
    if in_chat && !chat_busy {
        hints.push(("⌫", "Leave chat"));
    }
    if in_chat && chat_busy {
        hints.push(("Esc", "Stop"));
    } else {
        hints.push(("Esc", "Close"));
    }
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
    fn hints_reflect_close_stop_actions_and_code_blocks() {
        assert_eq!(
            hints(None, false, false, false, false),
            vec![("Esc", "Close")]
        );
        assert_eq!(
            hints(None, false, false, false, true),
            vec![("Esc", "Back")]
        );
        let info = ResultItem::Info("x".into());
        assert_eq!(
            hints(Some(&info), false, true, false, false),
            vec![("⌫", "Leave chat"), ("Esc", "Close")]
        );
        let calc = ResultItem::Calc {
            expression: "1+1".into(),
            result: "2".into(),
            swatch: None,
        };
        assert_eq!(
            hints(Some(&calc), false, false, false, false)[0],
            ("↵", "Copy")
        );
        assert!(hints(Some(&calc), false, false, false, false).contains(&("Ctrl+K", "Actions")));

        let streaming = ResultItem::Transcript {
            turns: vec![Turn {
                question: "hi?".into(),
                text: "thinking".into(),
                status: TurnStatus::Answering,
                model: "haiku".into(),
                label: "Claude Haiku".into(),
            }],
        };
        assert_eq!(
            hints(Some(&streaming), false, true, true, false),
            vec![("↑↓", "Scroll"), ("Esc", "Stop")]
        );

        let with_code = ResultItem::Transcript {
            turns: vec![Turn {
                question: "cmd?".into(),
                text: "Run:\n```sh\nls -la\n```".into(),
                status: TurnStatus::Done,
                model: "haiku".into(),
                label: "Claude Haiku".into(),
            }],
        };
        assert!(hints(Some(&with_code), false, true, false, false)
            .contains(&("Ctrl+Shift+C", "Copy code")));
    }
}
