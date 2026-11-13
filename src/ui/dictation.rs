//! Dictation into the search bar: Ctrl+D or the microphone button starts
//! listening, Enter (or Ctrl+D again) finishes, Escape cancels. With
//! whisper-cli the text appears in the bar while speaking and is replaced
//! by a more careful final pass at the end.

use super::Launcher;
use crate::voice::{self, Pass, Recording, BYTES_PER_SECOND};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;
use std::time::Duration;

/// A forgotten recording stops itself and is transcribed.
const MAX_RECORDING: Duration = Duration::from_secs(30);
const LISTENING: &str = "Listening…";
const TRANSCRIBING: &str = "Transcribing…";
/// How often to check whether to transcribe the audio so far again.
const LIVE_TICK: Duration = Duration::from_millis(200);
/// Speech needed before the first live pass, and new audio between passes.
const LIVE_FIRST: usize = BYTES_PER_SECOND / 2;
const LIVE_STEP: usize = BYTES_PER_SECOND * 3 / 10;

#[derive(Default)]
pub(super) enum Phase {
    #[default]
    Idle,
    Recording(Recording),
    Transcribing,
}

#[derive(Default)]
pub(super) struct Dictation {
    pub(super) phase: Phase,
    /// What was in the bar before; dictated text goes after it, and
    /// cancelling puts it back.
    prefix: String,
    /// A live pass is running; only one at a time.
    pass_running: bool,
    /// How much audio the last live pass started with.
    passed_len: usize,
}

impl Launcher {
    /// Show the microphone button only when dictation can work.
    pub(super) fn update_mic_button(&self) {
        let available = voice::unavailable_reason(&self.config.borrow().voice).is_none();
        self.mic_button.set_visible(available);
    }

    pub(super) fn dictating(&self) -> bool {
        !matches!(self.dictation.borrow().phase, Phase::Idle)
    }

    pub(super) fn recording(&self) -> bool {
        matches!(self.dictation.borrow().phase, Phase::Recording(_))
    }

    pub(super) fn toggle_dictation(self: &Rc<Self>) {
        if self.recording() {
            self.finish_dictation();
        } else if !self.dictating() {
            self.start_dictation();
        }
    }

    fn start_dictation(self: &Rc<Self>) {
        let config = self.config.borrow().voice.clone();
        if let Some(reason) = voice::unavailable_reason(&config) {
            self.flash_footer_notice(&reason);
            return;
        }
        let recording = match Recording::start() {
            Ok(recording) => recording,
            Err(error) => {
                self.flash_footer_notice(&error);
                return;
            }
        };
        let generation = self.dictation_gen.get() + 1;
        self.dictation_gen.set(generation);
        *self.dictation.borrow_mut() = Dictation {
            phase: Phase::Recording(recording),
            prefix: self.entry.text().trim_end().to_string(),
            ..Dictation::default()
        };
        self.show_dictation_state(Some("recording"), Some(LISTENING));

        if voice::live(&config) {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local(LIVE_TICK, move || {
                let Some(this) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if this.dictation_gen.get() != generation || !this.recording() {
                    return glib::ControlFlow::Break;
                }
                this.run_live_pass(generation);
                glib::ControlFlow::Continue
            });
        }

        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(MAX_RECORDING, move || {
            if let Some(this) = weak.upgrade() {
                if this.dictation_gen.get() == generation && this.recording() {
                    this.finish_dictation();
                }
            }
        });
    }

    /// Transcribe the audio so far, if there is enough new speech and the
    /// previous pass is done, and show it in the bar.
    fn run_live_pass(self: &Rc<Self>, generation: u64) {
        let audio = {
            let mut dictation = self.dictation.borrow_mut();
            let Phase::Recording(recording) = &dictation.phase else {
                return;
            };
            let len = recording.len();
            if dictation.pass_running || len < LIVE_FIRST || len < dictation.passed_len + LIVE_STEP
            {
                return;
            }
            let audio = recording.snapshot();
            dictation.pass_running = true;
            dictation.passed_len = audio.len();
            audio
        };
        let config = self.config.borrow().voice.clone();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let result =
                gio::spawn_blocking(move || voice::transcribe(&config, &audio, Pass::Partial))
                    .await
                    .ok();
            this.dictation.borrow_mut().pass_running = false;
            if this.dictation_gen.get() != generation || !this.recording() {
                return;
            }
            // "Didn't catch that" while still talking: keep what is shown.
            if let Some(Ok(text)) = result {
                this.show_dictated(&text);
            }
        });
    }

    /// Stop listening and put the final transcript in the search bar.
    pub(super) fn finish_dictation(self: &Rc<Self>) {
        let recording = {
            let mut dictation = self.dictation.borrow_mut();
            match std::mem::replace(&mut dictation.phase, Phase::Transcribing) {
                Phase::Recording(recording) => recording,
                other => {
                    dictation.phase = other;
                    return;
                }
            }
        };
        self.show_dictation_state(Some("transcribing"), Some(TRANSCRIBING));

        let generation = self.dictation_gen.get();
        let config = self.config.borrow().voice.clone();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                recording
                    .finish()
                    .and_then(|audio| voice::transcribe(&config, &audio, Pass::Final))
            })
            .await
            .unwrap_or_else(|_| Err("Speech to text failed".to_string()));
            // Cancelled, or the launcher was closed, while transcribing.
            if this.dictation_gen.get() != generation {
                return;
            }
            let prefix = this.reset_dictation();
            match result {
                Ok(text) => this.show_dictated_after(&prefix, &text),
                Err(error) => {
                    this.restore_text(&prefix);
                    this.flash_footer_notice(&error);
                }
            }
        });
    }

    /// Stop listening, forget what was said and put the bar back as it was.
    /// False if not dictating.
    pub(super) fn cancel_dictation(&self) -> bool {
        if !self.dictating() {
            return false;
        }
        self.dictation_gen.set(self.dictation_gen.get() + 1);
        let prefix = self.reset_dictation();
        self.restore_text(&prefix);
        true
    }

    /// Back to idle (dropping a recording stops the recorder); returns the
    /// text the bar had before dictating.
    fn reset_dictation(&self) -> String {
        let prefix = std::mem::take(&mut *self.dictation.borrow_mut()).prefix;
        self.show_dictation_state(None, None);
        prefix
    }

    fn show_dictation_state(&self, class: Option<&str>, notice: Option<&str>) {
        for state in ["recording", "transcribing"] {
            self.mic_button.remove_css_class(state);
        }
        if let Some(class) = class {
            self.mic_button.add_css_class(class);
            // Visible even if hidden before, e.g. dictation started by key.
            self.mic_button.set_visible(true);
            self.entry.add_css_class("dictating");
        } else {
            self.update_mic_button();
            self.entry.remove_css_class("dictating");
        }
        {
            let mut current = self.footer_notice.borrow_mut();
            let ours = matches!(current.as_deref(), Some(LISTENING | TRANSCRIBING));
            if notice.is_some() || ours {
                *current = notice.map(str::to_string);
            }
        }
        self.update_footer();
    }

    fn show_dictated(&self, text: &str) {
        let prefix = self.dictation.borrow().prefix.clone();
        self.show_dictated_after(&prefix, text);
    }

    /// Dictated text replaces an empty bar or continues what was typed.
    fn show_dictated_after(&self, prefix: &str, text: &str) {
        let text = if prefix.is_empty() {
            text.to_string()
        } else {
            format!("{} {}", prefix, text)
        };
        self.restore_text(&text);
    }

    fn restore_text(&self, text: &str) {
        if self.entry.text() != text {
            // Emits `changed`, which refreshes the results.
            self.entry.set_text(text);
        }
        self.entry.grab_focus();
        self.entry.set_position(-1);
    }
}
