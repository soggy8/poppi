//! Dictation into the search bar: Ctrl+D or the microphone button starts
//! listening, Enter (or Ctrl+D again) finishes, Escape cancels. With
//! whisper-cli the text appears in the bar while speaking and is replaced
//! by a more careful final pass at the end.

use super::Launcher;
use crate::actions::Action;
use crate::state::ResultItem;
use crate::voice::{self, Pass, Recording, BYTES_PER_SECOND};
use crate::wake;
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
/// Hands-free: this much quiet after speaking ends the dictation…
const PAUSE_FRAMES: usize = 50;
/// …and this long without speech gives up.
const NOTHING_SAID: usize = BYTES_PER_SECOND * 6;
/// Levels (RMS of 16-bit samples) that count as speech, and as quiet
/// relative to the loudest moment.
const SPEECH_LEVEL: f32 = 1200.0;
const QUIET_SHARE: f32 = 0.12;
const WAKE_POLL: Duration = Duration::from_millis(150);
/// Verbs a spoken command may start with before an app's name.
const OPEN_VERBS: [&str; 5] = ["open ", "launch ", "start ", "run ", "show "];

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
    /// Started by the wake word: stops by itself when the speaker pauses.
    hands_free: bool,
}

/// What a hands-free recording sounds like so far: finished (speech, then
/// a pause), abandoned (nothing said for a while), or still going.
fn pause_state(audio: &[u8]) -> Option<bool> {
    let levels: Vec<f32> = audio
        .chunks(BYTES_PER_SECOND / 50)
        .map(wake::level)
        .collect();
    let peak = levels.iter().cloned().fold(0.0, f32::max);
    if peak < SPEECH_LEVEL {
        return (audio.len() >= NOTHING_SAID).then_some(false);
    }
    let quiet = (peak * QUIET_SHARE).max(300.0);
    let trailing = levels.iter().rev().take_while(|l| **l < quiet).count();
    (trailing >= PAUSE_FRAMES).then_some(true)
}

/// What may run without Enter when Poppi was asked by voice: opening and
/// finding things, and asking the AI. Not commands, copying or power.
fn safe_hands_free(item: &ResultItem) -> bool {
    match item {
        ResultItem::App(_)
        | ResultItem::Window { .. }
        | ResultItem::File(_)
        | ResultItem::Search { .. }
        | ResultItem::Ask { .. } => true,
        ResultItem::Proposed(action) => matches!(
            action,
            Action::Open(_) | Action::FindFiles(_) | Action::SwitchWindow(_) | Action::OpenApp(_)
        ),
        _ => false,
    }
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
            self.start_dictation(false);
        }
    }

    fn start_dictation(self: &Rc<Self>, hands_free: bool) {
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
            hands_free,
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
                match this.hands_free_pause() {
                    Some(true) => this.finish_dictation(),
                    Some(false) => {
                        this.cancel_dictation();
                        this.dismiss();
                    }
                    None => this.run_live_pass(generation),
                }
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

    /// For hands-free dictation, whether the speaker has finished (or never
    /// started); `None` while they may still be talking.
    fn hands_free_pause(&self) -> Option<bool> {
        let dictation = self.dictation.borrow();
        let Phase::Recording(recording) = &dictation.phase else {
            return None;
        };
        if !dictation.hands_free {
            return None;
        }
        pause_state(&recording.snapshot())
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
        let hands_free = self.dictation.borrow().hands_free;
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
                Ok(text) => {
                    this.show_dictated_after(&prefix, &text);
                    if hands_free {
                        this.run_hands_free();
                    }
                }
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

    // ---------- wake word and hands-free use ----------

    /// Start or stop listening for the wake word to match the config. Only
    /// a launcher that stays resident can be woken.
    pub(super) fn update_wake_listener(self: &Rc<Self>) {
        let (wanted, voice) = {
            let config = self.config.borrow();
            (
                config.voice.wake_word && config.behavior.stay_resident,
                config.voice.clone(),
            )
        };
        if !wanted {
            self.wake.borrow_mut().take();
            return;
        }
        if self.wake.borrow().is_some() {
            return;
        }
        match wake::Listener::start(voice) {
            Ok(listener) => {
                listener.set_paused(self.window.is_visible());
                *self.wake.borrow_mut() = Some(listener);
            }
            Err(error) => {
                eprintln!("Wake word: {}", error);
                return;
            }
        }
        if self.wake_polling.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(WAKE_POLL, move || {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let events = match this.wake.borrow().as_ref() {
                Some(listener) => listener.poll(),
                None => {
                    this.wake_polling.set(false);
                    return glib::ControlFlow::Break;
                }
            };
            for event in events {
                match event {
                    wake::Event::Heard(command) => this.on_wake(command),
                    wake::Event::Failed(error) => {
                        eprintln!("Wake word: {}", error);
                        this.wake.borrow_mut().take();
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    pub(super) fn pause_wake_listener(&self, paused: bool) {
        if let Some(listener) = self.wake.borrow().as_ref() {
            listener.set_paused(paused);
        }
    }

    /// "Poppi" alone opens the launcher and listens; "Poppi, open Firefox"
    /// goes straight to the command.
    fn on_wake(self: &Rc<Self>, command: String) {
        if self.window.is_visible() {
            return;
        }
        self.show();
        self.hands_free.set(true);
        let command = command.trim_end_matches('.').trim().to_string();
        if command.is_empty() {
            self.start_dictation(true);
        } else {
            self.restore_text(&command);
            self.run_hands_free();
        }
    }

    /// A spoken command is in the bar: run it if the top result is
    /// harmless and clearly what was meant; otherwise let the AI handle it.
    fn run_hands_free(self: &Rc<Self>) {
        let first = self.state.borrow().results.first().cloned();
        if matches!(first, Some(ResultItem::App(_) | ResultItem::Window { .. })) {
            self.activate_index(0, false);
            return;
        }
        // "open firefox": the app, if the rest names one.
        let text = self.entry.text().to_string();
        let lower = text.to_lowercase();
        if let Some(verb) = OPEN_VERBS.iter().find(|v| lower.starts_with(*v)) {
            let rest = text[verb.len()..].trim().to_string();
            let app = {
                let state = self.state.borrow();
                state
                    .apps
                    .search(&rest, &state.usage, 1)
                    .first()
                    .map(|a| (*a).clone())
            };
            if app.is_some() && rest.split_whitespace().count() <= 3 {
                self.restore_text(&rest);
                if matches!(
                    self.state.borrow().results.first(),
                    Some(ResultItem::App(_))
                ) {
                    self.activate_index(0, false);
                    return;
                }
            }
        }
        let (enabled, model) = {
            let config = self.config.borrow();
            (config.ai.enabled, config.ai.default.clone())
        };
        if enabled {
            self.ask(text, model);
        }
    }

    /// After an answer to a spoken request: run its first proposed action
    /// without Enter when that is harmless.
    pub(super) fn run_proposed_hands_free(self: &Rc<Self>) {
        if !self.hands_free.get() {
            return;
        }
        let first_action = self.state.borrow().results.get(1).cloned();
        if first_action.as_ref().is_some_and(safe_hands_free) {
            self.activate_index(1, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seconds` of a square wave at `level`.
    fn sound(level: i16, seconds: f32) -> Vec<u8> {
        let samples = (seconds * 16_000.0) as usize;
        (0..samples)
            .flat_map(|i| (if i % 2 == 0 { level } else { -level }).to_le_bytes())
            .collect()
    }

    #[test]
    fn hands_free_dictation_ends_after_a_pause() {
        let mut audio = sound(100, 0.5);
        audio.extend(sound(5000, 1.5));
        assert_eq!(pause_state(&audio), None, "still talking");
        audio.extend(sound(100, 0.5));
        assert_eq!(pause_state(&audio), None, "a breath, not the end");
        audio.extend(sound(100, 0.6));
        assert_eq!(pause_state(&audio), Some(true), "done speaking");
    }

    #[test]
    fn hands_free_dictation_gives_up_on_silence() {
        assert_eq!(pause_state(&sound(100, 3.0)), None);
        assert_eq!(pause_state(&sound(100, 6.5)), Some(false));
    }
}
