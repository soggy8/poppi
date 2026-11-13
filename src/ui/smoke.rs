//! Scripted driver for UI smoke tests. Only built with `--features ui-test`.
//!
//! `POPPI_UI_SCRIPT` holds steps separated by `|`. A step is either text to
//! put in the search bar or one of `<enter>`, `<ctrl-enter>`, `<down>`,
//! `<down:N>`, `<up>`, `<tab>`, `<alt-up>`, `<backspace>`, `<wait>`.
//! After each step one `UI-TEST` line describing the state is printed to
//! stderr and, if `POPPI_UI_SHOTS` names a directory, a screenshot is saved.
//! After the last step the application quits.

use super::Launcher;
use crate::state::ResultItem;
use gtk::prelude::*;
use gtk::{gdk, glib, Window};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

fn kind(item: &ResultItem) -> &'static str {
    match item {
        ResultItem::App(_) => "app",
        ResultItem::Calc { .. } => "calc",
        ResultItem::Emoji(_) => "emoji",
        ResultItem::Terminal(_) => "terminal",
        ResultItem::Search { .. } => "search",
        ResultItem::Window { .. } => "window",
        ResultItem::File(_) => "file",
        ResultItem::ContentMatch(_) => "grep",
        ResultItem::Alias { .. } => "alias",
        ResultItem::ScriptRun { .. } => "script",
        ResultItem::ScriptOutput(_) => "script-item",
        ResultItem::ItemAction { .. } => "action",
        ResultItem::History(_) => "history",
        ResultItem::ClearHistory => "clear-history",
        ResultItem::Clipboard(_) => "clipboard",
        ResultItem::System(_) => "system",
        ResultItem::Confirm(_) => "confirm",
        ResultItem::Ask { .. } => "ask",
        ResultItem::Transcript { .. } => "transcript",
        ResultItem::Settings => "settings",
        ResultItem::Setup => "setup",
        ResultItem::Proposed(_) => "proposed",
        ResultItem::Info(_) => "info",
    }
}

fn screenshot(window: &Window, path: &str) {
    let width = f64::from(window.width().max(1));
    let height = f64::from(window.height().max(1));
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width, height);
    if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) {
        let _ = renderer.render_texture(&node, None).save_to_png(path);
    }
}

fn perform(launcher: &Rc<Launcher>, step: &str) {
    let none = gdk::ModifierType::empty();
    let press = |key: gdk::Key, modifiers: gdk::ModifierType| {
        launcher.on_key(key, modifiers);
    };
    if let Some(count) = step
        .strip_prefix("<down:")
        .and_then(|r| r.strip_suffix('>'))
    {
        for _ in 0..count.parse::<u32>().unwrap_or(1) {
            press(gdk::Key::Down, none);
        }
        return;
    }
    match step {
        "<wait>" => {}
        "<down>" => press(gdk::Key::Down, none),
        "<up>" => press(gdk::Key::Up, none),
        "<tab>" => press(gdk::Key::Tab, none),
        "<alt-up>" => press(gdk::Key::Up, gdk::ModifierType::ALT_MASK),
        "<backspace>" => press(gdk::Key::BackSpace, none),
        "<escape>" => press(gdk::Key::Escape, none),
        "<ctrl-c>" => press(gdk::Key::c, gdk::ModifierType::CONTROL_MASK),
        "<ctrl-k>" => press(gdk::Key::k, gdk::ModifierType::CONTROL_MASK),
        "<ctrl-d>" => press(gdk::Key::d, gdk::ModifierType::CONTROL_MASK),
        // Enter as a key press, not as running the selected result.
        "<key-enter>" => press(gdk::Key::Return, none),
        "<ctrl-shift-c>" => press(
            gdk::Key::C,
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
        ),
        "<enter>" => {
            if let Some(index) = launcher.selected_index() {
                launcher.activate_index(index, false);
            }
        }
        "<ctrl-enter>" => {
            if let Some(index) = launcher.selected_index() {
                launcher.activate_index(index, true);
            }
        }
        text => launcher.entry.set_text(text),
    }
}

fn report(launcher: &Rc<Launcher>, index: usize, step: &str, shots: Option<&str>) {
    let (mode, count, first, chat) = {
        let state = launcher.state.borrow();
        (
            state.mode.label().to_string(),
            state.results.len(),
            state.results.first().map(kind).unwrap_or("none"),
            state
                .chat
                .last()
                .map(|turn| {
                    format!(
                        "{}:{:?}:{}",
                        turn.model,
                        turn.status,
                        turn.text.chars().count()
                    )
                })
                .unwrap_or_else(|| "-".to_string()),
        )
    };
    let selected = launcher
        .selected_item()
        .as_ref()
        .map(kind)
        .unwrap_or("none");
    let adjustment = launcher.scrolled.vadjustment();
    eprintln!(
        "UI-TEST step={} do={:?} query={:?} mode={:?} results={} first={} selected={} chat={} visible={} size={}x{} scroll={:.0}/{:.0}/{:.0} voice={}",
        index,
        step,
        launcher.entry.text(),
        mode,
        count,
        first,
        selected,
        chat,
        launcher.window.is_visible(),
        launcher.window.width(),
        launcher.window.height(),
        adjustment.value(),
        adjustment.page_size(),
        adjustment.upper(),
        match launcher.dictation.borrow().phase {
            super::dictation::Phase::Idle => "idle",
            super::dictation::Phase::Recording(_) => "recording",
            super::dictation::Phase::Transcribing => "transcribing",
        },
    );
    if let Some(dir) = shots {
        if launcher.window.is_visible() {
            screenshot(&launcher.window, &format!("{}/step{:02}.png", dir, index));
        }
        for window in launcher.app.windows() {
            let name = window.widget_name();
            if let Some(page) = name.strip_prefix("poppi-") {
                screenshot(&window, &format!("{}/{}.png", dir, page));
            }
        }
    }
}

pub(super) fn run(launcher: &Rc<Launcher>) {
    let Ok(script) = std::env::var("POPPI_UI_SCRIPT") else {
        return;
    };
    if STARTED.with(|started| started.replace(true)) {
        return;
    }
    let shots = std::env::var("POPPI_UI_SHOTS")
        .ok()
        .filter(|dir| !dir.is_empty());
    let interval: u64 = std::env::var("POPPI_UI_STEP_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(700);
    let steps: Vec<String> = script.split('|').map(str::to_string).collect();
    let total = steps.len();

    for (index, step) in steps.into_iter().enumerate() {
        let start = 600 + index as u64 * interval;
        let this = launcher.clone();
        let step_for_action = step.clone();
        glib::timeout_add_local_once(Duration::from_millis(start), move || {
            perform(&this, &step_for_action);
        });

        let this = launcher.clone();
        let shots = shots.clone();
        // Report just before the next step, when the UI has settled.
        glib::timeout_add_local_once(Duration::from_millis(start + interval - 80), move || {
            report(&this, index, &step, shots.as_deref());
            if index + 1 == total {
                eprintln!("UI-TEST done");
                this.app.quit();
            }
        });
    }
}
