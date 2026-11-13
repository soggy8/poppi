//! The conversation with an AI model.
//!
//! One `ai::Session` lives for as long as the conversation keeps talking to
//! the same model. For the Claude CLI it is started as soon as a question is
//! being typed, so that by the time Enter is pressed the CLI has finished
//! starting up; nothing is sent to any model before Enter.

use super::Launcher;
use crate::ai::{self, providers, Event};
use crate::markdown;
use crate::state::{self, ResultItem, Turn, TurnStatus};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(40);

impl Launcher {
    /// Get the model about to be asked ready. Errors surface when asking.
    pub(super) fn warm_up_ai(&self) {
        let model = {
            let state = self.state.borrow();
            // Never replace a session that is still answering.
            if state.chat_busy() {
                return;
            }
            state.results.iter().find_map(|item| match item {
                ResultItem::Ask { model, .. } => Some(model.clone()),
                _ => None,
            })
        };
        if let Some(model) = model {
            let _ = self.ensure_session(&model);
        }
    }

    /// A session for `model`, replacing one for another model (or for an
    /// entry whose settings have changed since).
    fn ensure_session(&self, model: &str) -> Result<(), String> {
        let mut resolved = providers::resolve(&self.config.borrow().ai, Some(model))?;
        // Every model may propose actions; they run only on Enter.
        resolved.system_prompt = crate::actions::system_prompt(&resolved.system_prompt);
        {
            let mut ai = self.ai.borrow_mut();
            if let Some(session) = ai.as_mut() {
                if session.model == resolved {
                    return session.warm_up();
                }
            }
        }
        // Drop the old session first: it may own a process.
        self.ai.borrow_mut().take();
        let session = ai::Session::start(resolved)?;
        *self.ai.borrow_mut() = Some(session);
        Ok(())
    }

    /// Stop whatever is running and forget the conversation.
    pub(super) fn end_conversation(&self) {
        self.ai.borrow_mut().take();
        self.chat_follow.set(false);
        self.state.borrow_mut().chat.clear();
        self.transcript_view.borrow_mut().clear();
    }

    /// Stop an in-flight answer while keeping the launcher and conversation open.
    pub(super) fn stop_generation(self: &Rc<Self>) -> bool {
        if !self.state.borrow().chat_busy() {
            return false;
        }
        if let Some(session) = self.ai.borrow_mut().as_mut() {
            session.cancel();
        }
        {
            let mut state = self.state.borrow_mut();
            if let Some(turn) = state.chat.last_mut().filter(|turn| !turn.is_finished()) {
                if turn.text.trim().is_empty() {
                    turn.text = "Stopped.".to_string();
                    turn.status = TurnStatus::Failed;
                } else {
                    turn.status = TurnStatus::Done;
                }
            }
        }
        self.ai_polling.set(false);
        self.sync_streaming_chat(true);
        self.flash_footer_notice("Stopped");
        true
    }

    /// Copy the latest fenced code block (or the latest answer when there is
    /// no code block) without leaving the conversation.
    pub(super) fn copy_latest_code(self: &Rc<Self>) -> bool {
        let target = {
            let state = self.state.borrow();
            let code = state
                .chat
                .iter()
                .rev()
                .filter(|turn| turn.status == TurnStatus::Done)
                .find_map(|turn| markdown::last_code_block(&turn.text))
                .map(|code| (code, true));
            code.or_else(|| {
                state
                    .chat
                    .iter()
                    .rev()
                    .find(|turn| turn.status == TurnStatus::Done && !turn.text.trim().is_empty())
                    .map(|turn| (turn.text.trim().to_string(), false))
            })
        };
        let Some((text, is_code)) = target else {
            return false;
        };
        if let Err(error) = self.copy_text(&text) {
            self.flash_footer_notice(&error);
        } else if is_code {
            self.flash_footer_notice("Copied code to clipboard");
        } else {
            self.flash_footer_notice("Copied answer to clipboard");
        }
        true
    }

    /// Backspace on an empty bar: back to normal search.
    pub(super) fn leave_conversation(self: &Rc<Self>) {
        self.end_conversation();
        self.refresh();
    }

    /// Send a question (the first, or a follow-up) and stream the answer.
    pub(super) fn ask(self: &Rc<Self>, question: String, model: String) {
        if self.state.borrow().chat_busy() {
            // One answer at a time; the footer says it is still answering.
            return;
        }

        let history = state::chat_history(&self.state.borrow().chat);
        let label = providers::label(&self.config.borrow().ai, &model);
        // "explain this": the selected text goes along, as the row said.
        let context = self
            .state
            .borrow()
            .context_for(&question)
            .map(|(_, text)| text);
        let prompt = crate::context::attach(&question, context.as_deref());
        let sent = self.ensure_session(&model).and_then(|()| {
            self.ai
                .borrow_mut()
                .as_mut()
                .ok_or_else(|| "The model is not ready".to_string())?
                .send(&history, &prompt)
        });
        let (text, status) = match sent {
            Ok(()) => (String::new(), TurnStatus::Thinking),
            Err(error) => {
                // A broken session is not worth keeping.
                self.ai.borrow_mut().take();
                (error, TurnStatus::Failed)
            }
        };
        {
            let mut state = self.state.borrow_mut();
            state.history.push(&self.entry.text());
            state.chat.push(Turn {
                question,
                text,
                status,
                model,
                label,
                context,
            });
        }

        self.chat_follow.set(true);
        if self.entry.text().is_empty() {
            self.refresh();
        } else {
            // Clear the bar for the follow-up; emits `changed`, which refreshes.
            self.entry.set_text("");
        }
        self.start_polling();
    }

    fn start_polling(self: &Rc<Self>) {
        if self.ai_polling.replace(true) {
            return;
        }
        let this = self.clone();
        glib::timeout_add_local(POLL_INTERVAL, move || {
            let events = match this.ai.borrow().as_ref() {
                Some(session) => session.poll(),
                None => Vec::new(),
            };
            let mut session_broken = false;
            let mut status_changed = false;
            let changed = !events.is_empty();
            {
                let mut state = this.state.borrow_mut();
                for event in events {
                    let Some(turn) = state.chat.last_mut().filter(|turn| !turn.is_finished())
                    else {
                        // An event with no question waiting for it: the
                        // process failed while idle.
                        session_broken |= matches!(event, Event::Failed(_));
                        continue;
                    };
                    let prev_status = turn.status.clone();
                    match event {
                        Event::Delta(text) => {
                            turn.text.push_str(&text);
                            turn.status = TurnStatus::Answering;
                        }
                        Event::Done(text) => {
                            if !text.is_empty() {
                                turn.text = text;
                            }
                            turn.status = TurnStatus::Done;
                        }
                        Event::Failed(error) => {
                            turn.text = error;
                            turn.status = TurnStatus::Failed;
                            session_broken = true;
                        }
                    }
                    status_changed |= turn.status != prev_status;
                }
            }
            if session_broken {
                this.ai.borrow_mut().take();
            }
            if changed {
                this.sync_streaming_chat(status_changed);
            }

            if this.state.borrow().chat_busy() && this.ai.borrow().is_some() {
                glib::ControlFlow::Continue
            } else {
                // A session dropped mid-answer leaves the turn unfinished.
                let mut unfinished = false;
                {
                    let mut state = this.state.borrow_mut();
                    if let Some(turn) = state.chat.last_mut().filter(|turn| !turn.is_finished()) {
                        turn.status = TurnStatus::Failed;
                        if turn.text.is_empty() {
                            turn.text = "Stopped.".to_string();
                        }
                        unfinished = true;
                    }
                }
                if unfinished {
                    this.sync_streaming_chat(true);
                }
                this.ai_polling.set(false);
                glib::ControlFlow::Break
            }
        });
    }
}
