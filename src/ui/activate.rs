//! Running the selected result.

use super::Launcher;
use crate::actions::Action;
use crate::alias;
use crate::app_launcher::AppLauncher;
use crate::emoji_picker::EmojiPicker;
use crate::files::FileEntry;
use crate::scripts;
use crate::search;
use crate::settings::SettingsWindow;
use crate::state::{ResultItem, SecondaryAction, TurnStatus};
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
            ResultItem::Ask {
                question, model, ..
            } => {
                self.ask(question, model);
                return;
            }
            ResultItem::Transcript { turns, .. } => {
                // Enter on the conversation copies the latest answer.
                let Some(last) = turns
                    .iter()
                    .rev()
                    .find(|turn| turn.status == TurnStatus::Done)
                else {
                    return;
                };
                remember = false;
                self.copy_text(&crate::actions::split(&last.text).0)
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
            ResultItem::Proposed(action) => {
                remember = false;
                match self.run_proposed(action) {
                    Some(result) => result,
                    // It put a new search in the bar; stay open.
                    None => return,
                }
            }
            ResultItem::Remind { at, text } => {
                remember = false;
                self.state
                    .borrow_mut()
                    .reminders
                    .add(crate::reminders::Reminder { at, text });
                Ok(())
            }
            ResultItem::PendingReminder(reminder) => {
                self.state.borrow_mut().reminders.remove(&reminder);
                self.flash_footer_notice("Reminder cancelled");
                self.refresh();
                return;
            }
            ResultItem::Note(text) => {
                remember = false;
                let file = self.config.borrow().notes.file.clone();
                crate::reminders::add_note(&file, &text)
            }
            ResultItem::OpenNotes => {
                let file = self.config.borrow().notes.file.clone();
                let path = crate::files::expand_tilde(&file);
                if !path.exists() {
                    let _ = crate::reminders::add_note(&file, "Notes from Poppi");
                }
                let uri = gio::File::for_path(&path).uri();
                gio::AppInfo::launch_default_for_uri(&uri, self.launch_context().as_ref())
                    .map_err(|e| format!("Could not open {}: {}", file, e))
            }
            ResultItem::Setup => {
                remember = false;
                crate::setup::open(&self.app, self.config.clone());
                Ok(())
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
            ResultItem::Clipboard(entry) => {
                remember = false;
                self.copy_text(&entry.text)
            }
            ResultItem::ScriptRun { script, args } => {
                remember = !args.is_empty();
                scripts::run_script(&script, &args)
            }
            ResultItem::ScriptOutput(item) => {
                match scripts::run_item_action(&item.action, self.launch_context().as_ref()) {
                    Ok(Some(text)) => self.copy_text(&text),
                    Ok(None) => Ok(()),
                    Err(e) => Err(e),
                }
            }
            ResultItem::ItemAction { action, .. } => {
                remember = false;
                match action {
                    SecondaryAction::CloseWindow(window) => {
                        self.close_window_in_place(window);
                        return;
                    }
                    SecondaryAction::ToggleClipboardPin(text) => {
                        let pinned = self.state.borrow_mut().clipboard.toggle_pin(&text);
                        self.leave_action_menu();
                        self.flash_footer_notice(if pinned {
                            "Pinned snippet"
                        } else {
                            "Unpinned snippet"
                        });
                        return;
                    }
                    SecondaryAction::RemoveClipboardEntry(text) => {
                        self.state.borrow_mut().clipboard.remove(&text);
                        self.leave_action_menu();
                        self.flash_footer_notice("Removed clipboard entry");
                        return;
                    }
                    SecondaryAction::LaunchApp(app) => {
                        let _ = self.state.borrow_mut().close_action_menu();
                        let res = AppLauncher::launch(&app, self.launch_context().as_ref());
                        if res.is_ok() {
                            self.state.borrow_mut().usage.record(&app.id);
                        }
                        res
                    }
                    SecondaryAction::OpenFile { file, reveal } => {
                        let _ = self.state.borrow_mut().close_action_menu();
                        let res = self.open_file(&file, reveal);
                        if res.is_ok() {
                            self.state.borrow_mut().usage.record(&file.usage_id());
                        }
                        res
                    }
                    SecondaryAction::OpenTerminalInDir(dir) => {
                        let _ = self.state.borrow_mut().close_action_menu();
                        Terminal::open_in_dir(&dir)
                    }
                    SecondaryAction::FocusWindow(window) => {
                        let _ = self.state.borrow_mut().close_action_menu();
                        window_switcher::activate(&window)
                    }
                    SecondaryAction::CopyText(text) => {
                        let _ = self.state.borrow_mut().close_action_menu();
                        self.copy_text(&text)
                    }
                }
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
            ResultItem::ContentMatch(m) => {
                let result = self.open_file(&m.file, alternate);
                if result.is_ok() {
                    self.state.borrow_mut().usage.record(&m.file.usage_id());
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

    /// Run an action the AI proposed. `None` when it turned into a new
    /// search in the bar (finding files, picking a window or an app), so
    /// the launcher stays open to show the results.
    fn run_proposed(&self, action: Action) -> Option<Result<(), String>> {
        let search_for = |text: String| {
            self.end_conversation();
            self.entry.set_text(&text);
            self.entry.set_position(-1);
            None
        };
        match action {
            Action::Open(target) if target.contains("://") || target.starts_with("www.") => {
                let url = if target.contains("://") {
                    target
                } else {
                    format!("https://{}", target)
                };
                Some(search::open_url(&url, self.launch_context().as_ref()))
            }
            Action::Open(target) => {
                let path = crate::files::expand_tilde(&target);
                if !path.exists() {
                    return Some(Err(format!("{} does not exist", target)));
                }
                let uri = gio::File::for_path(&path).uri();
                Some(
                    gio::AppInfo::launch_default_for_uri(&uri, self.launch_context().as_ref())
                        .map_err(|e| format!("Could not open {}: {}", target, e)),
                )
            }
            Action::Copy(text) => Some(self.copy_text(&text)),
            Action::FindFiles(query) => search_for(format!("f {}", query)),
            Action::SwitchWindow(name) => search_for(format!("sw {}", name)),
            Action::OpenApp(name) => search_for(name),
            // The time was unclear: let it be fixed in the bar.
            Action::Remind(text) => search_for(format!("remind {}", text)),
            Action::Note(text) => {
                let file = self.config.borrow().notes.file.clone();
                Some(crate::reminders::add_note(&file, &text))
            }
            // Shown as App, Search, Terminal and System rows instead.
            Action::WebSearch(_) | Action::Run(_) | Action::System(_) => Some(Ok(())),
        }
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
