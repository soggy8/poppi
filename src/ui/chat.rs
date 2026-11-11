//! The conversation with Claude.
//!
//! One CLI process (`claude::Session`) lives for as long as a conversation
//! does. It is started as soon as a question is being typed, so that by the
//! time Enter is pressed the CLI has finished starting up; nothing is sent
//! to Claude before Enter.

use super::Launcher;
use crate::claude::{self, Event};
use crate::state::{Turn, TurnStatus};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(40);

impl Launcher {
    /// Start the CLI if it is not running. Errors surface when asking.
    pub(super) fn warm_up_claude(&self) {
        let _ = self.ensure_session();
    }

    fn ensure_session(&self) -> Result<(), String> {
        if self.claude.borrow().is_some() {
            return Ok(());
        }
        let config = self.config.borrow().claude.clone();
        let session = claude::Session::start(&config)?;
        *self.claude.borrow_mut() = Some(session);
        Ok(())
    }

    /// Stop the CLI and forget the conversation.
    pub(super) fn end_conversation(&self) {
        self.claude.borrow_mut().take();
        self.chat_follow.set(false);
        self.state.borrow_mut().chat.clear();
    }

    /// Backspace on an empty bar: back to normal search.
    pub(super) fn leave_conversation(self: &Rc<Self>) {
        self.end_conversation();
        self.refresh();
    }

    /// Send a question (the first, or a follow-up) and stream the answer.
    pub(super) fn ask(self: &Rc<Self>, question: String) {
        if self.state.borrow().chat_busy() {
            // One answer at a time; the footer says it is still answering.
            return;
        }

        let sent = self.ensure_session().and_then(|()| {
            self.claude
                .borrow_mut()
                .as_mut()
                .ok_or_else(|| "Claude is not running".to_string())?
                .send(&question)
        });
        let turn = match sent {
            Ok(()) => Turn {
                question: question.clone(),
                text: String::new(),
                status: TurnStatus::Thinking,
            },
            Err(error) => {
                // A broken process is not worth keeping.
                self.claude.borrow_mut().take();
                Turn {
                    question: question.clone(),
                    text: error,
                    status: TurnStatus::Failed,
                }
            }
        };
        {
            let mut state = self.state.borrow_mut();
            state.history.push(&self.entry.text());
            state.chat.push(turn);
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
        if self.claude_polling.replace(true) {
            return;
        }
        let this = self.clone();
        glib::timeout_add_local(POLL_INTERVAL, move || {
            let events = match this.claude.borrow().as_ref() {
                Some(session) => session.poll(),
                None => Vec::new(),
            };
            let mut session_broken = false;
            let changed = !events.is_empty();
            {
                let mut state = this.state.borrow_mut();
                for event in events {
                    let Some(turn) = state.chat.last_mut().filter(|turn| !turn.is_finished()) else {
                        // An event with no question waiting for it: the
                        // process failed while idle.
                        session_broken |= matches!(event, Event::Failed(_));
                        continue;
                    };
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
                }
            }
            if session_broken {
                this.claude.borrow_mut().take();
            }
            if changed && this.window.is_visible() {
                this.refresh();
            }

            if this.state.borrow().chat_busy() && this.claude.borrow().is_some() {
                glib::ControlFlow::Continue
            } else {
                this.claude_polling.set(false);
                glib::ControlFlow::Break
            }
        });
    }
}
