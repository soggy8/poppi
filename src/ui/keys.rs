//! Keyboard handling for the launcher.

use super::Launcher;
use crate::files;
use crate::state::{ResultItem, EMOJI_COLUMNS};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::rc::Rc;

const CHAT_SCROLL_STEP: f64 = 48.0;

impl Launcher {
    pub(super) fn on_key(
        self: &Rc<Self>,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> glib::Propagation {
        let (count, emoji_mode, answer_mode) = {
            let state = self.state.borrow();
            (state.results.len(), state.emoji_mode, state.answer_mode())
        };
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);

        // While dictating, Enter finishes and Escape cancels.
        if self.dictating() {
            match key {
                gdk::Key::Escape => {
                    self.cancel_dictation();
                    return glib::Propagation::Stop;
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    self.finish_dictation();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        if ctrl && !alt && !shift && matches!(key, gdk::Key::d | gdk::Key::D) {
            self.toggle_dictation();
            return glib::Propagation::Stop;
        }

        if answer_mode && !alt && self.on_chat_key(key, ctrl, shift) {
            return glib::Propagation::Stop;
        }

        if ctrl
            && !alt
            && !shift
            && matches!(key, gdk::Key::k | gdk::Key::K)
            && self.toggle_action_menu()
        {
            return glib::Propagation::Stop;
        }

        if ctrl && !alt && !shift && matches!(key, gdk::Key::w | gdk::Key::W) {
            if let Some(ResultItem::Window { window, .. }) = self.selected_item() {
                self.close_window_in_place(window);
                return glib::Propagation::Stop;
            }
        }

        if !ctrl
            && !alt
            && !shift
            && key == gdk::Key::BackSpace
            && self.entry.text().is_empty()
            && self.leave_action_menu()
        {
            return glib::Propagation::Stop;
        }

        // Alt+1 … Alt+9 run the nth result directly.
        if alt && !emoji_mode {
            if let Some(digit) = key.to_unicode().and_then(|c| c.to_digit(10)) {
                if (1..=9).contains(&digit) && (digit as usize) <= count {
                    self.activate_index(digit as usize - 1, false);
                    return glib::Propagation::Stop;
                }
            }
        }

        match key {
            gdk::Key::Escape => {
                if !self.leave_action_menu() && !self.stop_generation() {
                    self.dismiss();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::KP_Up if alt => {
                self.recall_history(1);
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::KP_Down if alt => {
                self.recall_history(-1);
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter if ctrl => {
                if let Some(index) = self.selected_index() {
                    self.activate_index(index, true);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Tab => {
                if !self.complete_path() {
                    self.move_selection(count, emoji_mode, 1);
                }
                glib::Propagation::Stop
            }
            gdk::Key::ISO_Left_Tab => {
                self.move_selection(count, emoji_mode, -1);
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::KP_Down => {
                if emoji_mode {
                    self.move_emoji(count, |i, last| (i + EMOJI_COLUMNS).min(last));
                } else {
                    self.move_list(count, 1);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::KP_Up => {
                if emoji_mode {
                    self.move_emoji(count, |i, _| {
                        if i >= EMOJI_COLUMNS {
                            i - EMOJI_COLUMNS
                        } else {
                            i
                        }
                    });
                } else {
                    self.move_list(count, -1);
                }
                glib::Propagation::Stop
            }
            gdk::Key::Page_Down | gdk::Key::Page_Up if !emoji_mode => {
                let page = self.config.borrow().behavior.max_results as i32;
                self.move_list(
                    count,
                    if key == gdk::Key::Page_Down {
                        page
                    } else {
                        -page
                    },
                );
                glib::Propagation::Stop
            }
            gdk::Key::Right | gdk::Key::KP_Right if emoji_mode && count > 0 => {
                self.move_emoji(count, |i, last| (i + 1).min(last));
                glib::Propagation::Stop
            }
            gdk::Key::Left | gdk::Key::KP_Left if emoji_mode && count > 0 => {
                self.move_emoji(count, |i, _| i.saturating_sub(1));
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    /// Keys that mean something else during a conversation: the arrows
    /// scroll it, Ctrl+Shift+C copies the latest code block, Ctrl+C stops an
    /// in-flight answer, and Backspace on an empty bar leaves the chat.
    fn on_chat_key(self: &Rc<Self>, key: gdk::Key, ctrl: bool, shift: bool) -> bool {
        if ctrl && shift && matches!(key, gdk::Key::C | gdk::Key::c) {
            return self.copy_latest_code();
        }
        if ctrl
            && !shift
            && matches!(key, gdk::Key::C | gdk::Key::c)
            && self.entry.selection_bounds().is_none()
            && self.stop_generation()
        {
            return true;
        }
        if !ctrl && !shift && key == gdk::Key::BackSpace && self.entry.text().is_empty() {
            self.leave_conversation();
            return true;
        }
        if ctrl {
            return false;
        }

        let adjustment = self.scrolled.vadjustment();
        let step = match key {
            gdk::Key::Down | gdk::Key::KP_Down => CHAT_SCROLL_STEP,
            gdk::Key::Up | gdk::Key::KP_Up => -CHAT_SCROLL_STEP,
            gdk::Key::Page_Down => adjustment.page_size(),
            gdk::Key::Page_Up => -adjustment.page_size(),
            _ => return false,
        };
        let end = (adjustment.upper() - adjustment.page_size()).max(0.0);
        let target = (adjustment.value() + step).clamp(0.0, end);
        // Scrolling up stops following new text; reaching the end resumes.
        self.chat_follow.set(target >= end);
        adjustment.set_value(target);
        true
    }

    fn move_selection(&self, count: usize, emoji_mode: bool, delta: i32) {
        if emoji_mode {
            self.move_emoji(count, |i, last| {
                if delta > 0 {
                    (i + 1).min(last)
                } else {
                    i.saturating_sub(1)
                }
            });
        } else {
            self.move_list(count, delta);
        }
    }

    fn move_emoji(&self, count: usize, step: impl Fn(usize, usize) -> usize) {
        if count == 0 {
            return;
        }
        let last = count - 1;
        let current = self.emoji_selected.get().min(last);
        let next = step(current, last);
        self.emoji_selected.set(next);
        self.update_emoji_selection();
        if let Some(button) = self.emoji_buttons.borrow().get(next) {
            self.scroll_into_view(button);
        }
    }

    fn move_list(&self, count: usize, delta: i32) {
        if count == 0 {
            return;
        }
        let last = count as i32 - 1;
        let target = match self.list_box.selected_row() {
            Some(row) => (row.index() + delta).clamp(0, last),
            None if delta > 0 => 0,
            None => last,
        };
        if let Some(row) = self.list_box.row_at_index(target) {
            if row.is_selectable() {
                self.list_box.select_row(Some(&row));
            }
            self.scroll_into_view(&row);
        }
    }

    /// Alt+Up steps back through past queries, Alt+Down forward again.
    fn recall_history(&self, delta: i32) {
        let entries: Vec<String> = self.state.borrow().history.entries().to_vec();
        if entries.is_empty() {
            return;
        }
        let next = match (self.history_position.get(), delta > 0) {
            (None, true) => Some(0),
            (None, false) => None,
            (Some(position), true) => Some((position + 1).min(entries.len() - 1)),
            (Some(0), false) => None,
            (Some(position), false) => Some(position - 1),
        };
        self.history_position.set(next);
        let text = next.map(|i| entries[i].as_str()).unwrap_or("");

        // Replace the text without ending the stepping.
        self.recalling.set(true);
        self.entry.set_text(text);
        self.entry.set_position(-1);
        self.recalling.set(false);
    }

    /// Tab on a file while a path is being typed completes it in the bar.
    fn complete_path(&self) -> bool {
        let text = self.entry.text();
        if !(text.starts_with('/') || text.starts_with('~')) {
            return false;
        }
        let Some(ResultItem::File(file)) = self.selected_item() else {
            return false;
        };
        self.entry.set_text(&files::completion_text(&file));
        self.entry.set_position(-1);
        true
    }
}
