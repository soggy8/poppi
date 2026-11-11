//! Running the selected result.

use super::Launcher;
use crate::alias;
use crate::app_launcher::AppLauncher;
use crate::emoji_picker::EmojiPicker;
use crate::files::FileEntry;
use crate::search;
use crate::settings::SettingsWindow;
use crate::state::{ResultItem, TurnStatus};
use crate::terminal::Terminal;
use crate::window_switcher;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;
use std::time::Duration;

/// Time for focus to return to the previous window before typing an emoji.
const FOCUS_RETURN_DELAY: Duration = Duration::from_millis(200);

impl Launcher {
    /// `alternate` is Ctrl+Enter: for files, open the containing folder.
    pub(super) fn activate_index(self: &Rc<Self>, index: usize, alternate: bool) {
        let Some(item) = self.state.borrow().results.get(index).cloned() else {
            return;
        };
        let query = self.entry.text().to_string();
        // Most runs are worth recalling later; some would only be noise.
        let mut remember = true;

        let outcome = match item {
            ResultItem::Info(_) => return,
            ResultItem::Ask { question, .. } => {
                self.ask(question);
                return;
            }
            ResultItem::Transcript { turns, .. } => {
                // Enter on the conversation copies the latest answer.
                let Some(last) = turns.iter().rev().find(|turn| turn.status == TurnStatus::Done)
                else {
                    return;
                };
                remember = false;
                self.copy_text(last.text.trim())
            }
            ResultItem::History(past) => {
                // Bring the query back for editing or another Enter.
                self.entry.set_text(&past);
                self.entry.set_position(-1);
                return;
            }
            ResultItem::ClearHistory => {
                self.state.borrow_mut().history.clear();
                self.entry.set_text("");
                return;
            }
            ResultItem::System(action) if action.needs_confirmation() => {
                {
                    let mut state = self.state.borrow_mut();
                    state.results = vec![ResultItem::Confirm(action)];
                }
                self.render();
                return;
            }
            ResultItem::System(action) | ResultItem::Confirm(action) => {
                remember = false;
                action.run()
            }
            ResultItem::Settings => {
                remember = false;
                // Open first so the application always has a window.
                SettingsWindow::open(&self.app, self.config.clone());
                Ok(())
            }
            ResultItem::Emoji(emoji) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.emoji_recent.push(emoji.emoji);
                }
                self.insert_emoji_after_dismiss(emoji.emoji);
                return;
            }
            ResultItem::App(app) => {
                // Launch counts already rank apps; no need to remember "fire".
                remember = false;
                let result = AppLauncher::launch(&app, self.launch_context().as_ref());
                if result.is_ok() {
                    self.state.borrow_mut().usage.record(&app.id);
                }
                result
            }
            ResultItem::Calc { result, .. } => self.copy_text(&result),
            ResultItem::Clipboard(text) => {
                remember = false;
                self.copy_text(&text)
            }
            ResultItem::Terminal(command) => Terminal::execute_command(&command),
            ResultItem::Search { engine, query } => {
                search::open(engine, &query, self.launch_context().as_ref())
            }
            ResultItem::Window { window, .. } => {
                remember = false;
                window_switcher::activate(&window)
            }
            ResultItem::File(file) => {
                let result = self.open_file(&file, alternate);
                if result.is_ok() {
                    self.state.borrow_mut().usage.record(&file.usage_id());
                }
                result
            }
            ResultItem::Alias { target, .. } => alias::run(&target, self.launch_context().as_ref()),
        };

        match outcome {
            Ok(()) => {
                if remember {
                    self.state.borrow_mut().history.push(&query);
                }
                self.dismiss();
            }
            Err(error) => self.show_error(&error),
        }
    }

    fn open_file(&self, file: &FileEntry, reveal: bool) -> Result<(), String> {
        // Ctrl+Enter opens the containing folder instead of the file.
        let target = if reveal {
            file.path.parent().unwrap_or(&file.path)
        } else {
            &file.path
        };
        let uri = gio::File::for_path(target).uri();
        gio::AppInfo::launch_default_for_uri(&uri, self.launch_context().as_ref())
            .map_err(|e| format!("Could not open {}: {}", target.display(), e))
    }

    /// Typing tools send keystrokes to whichever window has focus, so the
    /// launcher has to be gone before they run.
    fn insert_emoji_after_dismiss(self: &Rc<Self>, emoji: &'static str) {
        // Keep the application alive until the emoji has been delivered.
        let hold = self.app.hold();
        self.dismiss();
        glib::timeout_add_local_once(FOCUS_RETURN_DELAY, move || {
            if let Err(error) = EmojiPicker::insert(emoji) {
                eprintln!("Could not insert emoji: {}", error);
            }
            drop(hold);
        });
    }

    pub(super) fn show_error(self: &Rc<Self>, error: &str) {
        eprintln!("Error: {}", error);
        {
            let mut state = self.state.borrow_mut();
            state.emoji_mode = false;
            state.results = vec![ResultItem::Info(format!("Error: {}", error))];
        }
        self.render();
    }
}
