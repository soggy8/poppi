//! The wake word: while enabled, the microphone is read continuously, a
//! simple voice activity detector cuts out each stretch of speech, and only
//! those (short) clips are transcribed with whisper, locally. A clip that
//! starts with the wake phrase ("Poppi, open Firefox") is reported with
//! whatever followed it. Silence costs next to nothing.

use crate::config::VoiceConfig;
use crate::voice::{self, Pass, BYTES_PER_SECOND};
use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// 20 ms of 16 kHz mono s16le.
const FRAME_BYTES: usize = BYTES_PER_SECOND / 50;
/// Speech kept from just before the level rose, so first syllables survive.
const PRE_ROLL_FRAMES: usize = 15;
/// Quiet this long ends a stretch of speech.
const END_SILENCE_FRAMES: usize = 35;
/// Shorter than this is a click or a cough.
const MIN_VOICED_FRAMES: usize = 10;
/// Longer than this is not someone talking to Poppi.
const MAX_SEGMENT_FRAMES: usize = 50 * 8;
/// Below this level nothing counts as speech, however quiet the room.
const MIN_SPEECH_LEVEL: f32 = 350.0;
/// Speech is this many times louder than the room's noise floor.
const SPEECH_OVER_FLOOR: f32 = 3.0;

/// Root mean square level of s16le samples.
pub fn level(pcm: &[u8]) -> f32 {
    let samples = pcm.len() / 2;
    if samples == 0 {
        return 0.0;
    }
    let sum: f64 = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| {
            let s = f64::from(i16::from_le_bytes(*b));
            s * s
        })
        .sum();
    (sum / samples as f64).sqrt() as f32
}

/// Cuts a stream of frames into stretches of speech.
pub struct Vad {
    floor: f32,
    pre_roll: Vec<Vec<u8>>,
    segment: Vec<u8>,
    in_speech: bool,
    voiced: usize,
    silent: usize,
    frames: usize,
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            floor: MIN_SPEECH_LEVEL / SPEECH_OVER_FLOOR,
            pre_roll: Vec::new(),
            segment: Vec::new(),
            in_speech: false,
            voiced: 0,
            silent: 0,
            frames: 0,
        }
    }
}

impl Vad {
    /// Feed one frame; returns a finished stretch of speech, if any.
    pub fn push(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        let level = level(frame);
        let loud = level > (self.floor * SPEECH_OVER_FLOOR).max(MIN_SPEECH_LEVEL);
        if !self.in_speech {
            // Learn the room while nobody speaks.
            self.floor = self.floor * 0.95 + level * 0.05;
            self.pre_roll.push(frame.to_vec());
            if self.pre_roll.len() > PRE_ROLL_FRAMES {
                self.pre_roll.remove(0);
            }
            if loud {
                self.in_speech = true;
                self.segment = self.pre_roll.concat();
                self.frames = self.pre_roll.len();
                self.pre_roll.clear();
                self.voiced = 1;
                self.silent = 0;
            }
            return None;
        }
        self.segment.extend_from_slice(frame);
        self.frames += 1;
        if loud {
            self.voiced += 1;
            self.silent = 0;
        } else {
            self.silent += 1;
        }
        if self.silent < END_SILENCE_FRAMES && self.frames < MAX_SEGMENT_FRAMES {
            return None;
        }
        self.in_speech = false;
        let segment = std::mem::take(&mut self.segment);
        let speech = self.voiced >= MIN_VOICED_FRAMES && self.frames < MAX_SEGMENT_FRAMES;
        speech.then_some(segment)
    }
}

/// Lowercase letters and digits only: how words are compared.
fn normalize(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// How whisper may spell the phrase. "Poppi" is not a dictionary word, so
/// its sound-alikes count too.
fn spellings(phrase: &str) -> Vec<Vec<String>> {
    let words: Vec<String> = phrase.split_whitespace().map(normalize).collect();
    let mut found = vec![words.clone()];
    if words == ["poppi"] {
        for alike in [
            "poppy", "popi", "poppie", "poppee", "popee", "popey", "pappi", "papi",
        ] {
            found.push(vec![alike.to_string()]);
        }
        found.push(vec!["pop".to_string(), "e".to_string()]);
        // How a short "Poppi" on its own often comes out.
        found.push(vec!["pop".to_string(), "it".to_string()]);
    }
    found
}

/// If `text` starts with the wake phrase (after a word like "hey" or
/// "ok"), what was said after it: `Some("")` for the phrase alone.
pub fn match_wake(text: &str, phrase: &str) -> Option<String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<String> = tokens.iter().map(|t| normalize(t)).collect();
    // "Poppi, …" or "Hey Poppi, …"; not "I told Poppi about it".
    let greeted = words
        .first()
        .is_some_and(|w| ["hey", "hi", "ok", "okay", "yo", "hello"].contains(&w.as_str()));
    let starts: &[usize] = if greeted { &[0, 1] } else { &[0] };
    for spelling in spellings(phrase) {
        if spelling.is_empty() {
            continue;
        }
        for &start in starts {
            let end = start + spelling.len();
            if end <= words.len() && words[start..end] == spelling[..] {
                let rest = tokens[end..].join(" ");
                let rest = rest.trim_start_matches([',', '.', '!', ':', ';', '-', ' ']);
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

pub enum Event {
    /// The wake phrase, with what followed it (possibly nothing).
    Heard(String),
    /// The microphone stopped; listening has ended.
    Failed(String),
}

/// Listens in the background until dropped.
pub struct Listener {
    child: Child,
    events: Receiver<Event>,
    paused: Arc<AtomicBool>,
}

impl Listener {
    pub fn start(config: VoiceConfig) -> Result<Self, String> {
        if let Some(reason) = voice::unavailable_reason(&config) {
            return Err(reason);
        }
        if !voice::live(&config) {
            return Err("The wake word needs whisper-cli".to_string());
        }
        let (mut command, name) = voice::recorder_command()?;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start {}: {}", name, e))?;
        let mut stdout = child.stdout.take().ok_or("Recorder has no output")?;
        let (events_tx, events) = mpsc::channel();
        let (segments_tx, segments) = mpsc::channel::<Vec<u8>>();
        let paused = Arc::new(AtomicBool::new(false));

        // Reading never waits for whisper, so the recorder never stalls.
        let reader_events = events_tx.clone();
        let reader_paused = paused.clone();
        std::thread::spawn(move || {
            let mut vad = Vad::default();
            let mut frame = vec![0u8; FRAME_BYTES];
            while stdout.read_exact(&mut frame).is_ok() {
                if let Some(segment) = vad.push(&frame) {
                    if !reader_paused.load(Ordering::Relaxed) && segments_tx.send(segment).is_err()
                    {
                        return;
                    }
                }
            }
            let _ = reader_events.send(Event::Failed("The microphone stopped".to_string()));
        });
        std::thread::spawn(move || transcribe_segments(&config, segments, events_tx));

        Ok(Self {
            child,
            events,
            paused,
        })
    }

    /// While the launcher is open, speech is meant for it, not for waking.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

/// Transcribe stretches of speech one at a time; when several queued up
/// while whisper was busy, only the newest matters.
fn transcribe_segments(config: &VoiceConfig, segments: Receiver<Vec<u8>>, events: Sender<Event>) {
    while let Ok(mut segment) = segments.recv() {
        loop {
            match segments.try_recv() {
                Ok(newer) => segment = newer,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let Ok(text) = voice::transcribe(config, &segment, Pass::Wake) else {
            continue;
        };
        if let Some(command) = match_wake(&text, &config.wake_phrase) {
            if events.send(Event::Heard(command)).is_err() {
                return;
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(level: i16, count: usize) -> Vec<Vec<u8>> {
        // A square wave: its RMS is `level`.
        let frame: Vec<u8> = (0..FRAME_BYTES / 2)
            .flat_map(|i| (if i % 2 == 0 { level } else { -level }).to_le_bytes())
            .collect();
        vec![frame; count]
    }

    fn run(vad: &mut Vad, input: &[Vec<u8>]) -> Vec<Vec<u8>> {
        input.iter().filter_map(|f| vad.push(f)).collect()
    }

    #[test]
    fn level_is_rms() {
        assert_eq!(level(&frames(1000, 1)[0]), 1000.0);
        assert_eq!(level(&[]), 0.0);
    }

    #[test]
    fn speech_between_silences_is_one_segment_with_pre_roll() {
        let mut vad = Vad::default();
        let mut input = frames(20, 100);
        input.extend(frames(4000, 40));
        input.extend(frames(20, 60));
        let segments = run(&mut vad, &input);
        assert_eq!(segments.len(), 1);
        let len_frames = segments[0].len() / FRAME_BYTES;
        assert_eq!(len_frames, PRE_ROLL_FRAMES + 40 + END_SILENCE_FRAMES - 1);
    }

    #[test]
    fn clicks_and_endless_noise_are_ignored() {
        let mut vad = Vad::default();
        let mut input = frames(20, 50);
        input.extend(frames(4000, 3));
        input.extend(frames(20, 60));
        assert!(run(&mut vad, &input).is_empty(), "a click is not speech");

        let mut vad = Vad::default();
        let mut input = frames(20, 50);
        input.extend(frames(4000, MAX_SEGMENT_FRAMES + 10));
        assert!(
            run(&mut vad, &input).is_empty(),
            "music is not talking to Poppi"
        );
    }

    #[test]
    fn a_noisy_room_raises_the_bar() {
        let mut vad = Vad::default();
        let mut input = frames(600, 300);
        input.extend(frames(1200, 40));
        input.extend(frames(600, 60));
        assert!(
            run(&mut vad, &input).is_empty(),
            "twice the hum is not speech"
        );
    }

    #[test]
    fn the_wake_phrase_and_what_follows() {
        assert_eq!(
            match_wake(" Poppi, open Firefox.", "Poppi"),
            Some("open Firefox.".into())
        );
        assert_eq!(
            match_wake("Hey Poppy! What's the time?", "Poppi"),
            Some("What's the time?".into())
        );
        assert_eq!(match_wake("Poppi.", "Poppi"), Some(String::new()));
        assert_eq!(
            match_wake("pop e lock the screen", "Poppi"),
            Some("lock the screen".into())
        );
        assert_eq!(match_wake("I told Poppi about it", "Poppi"), None);
        assert_eq!(
            match_wake("so what did you think of the poppies", "Poppi"),
            None
        );
        assert_eq!(match_wake("Thank you.", "Poppi"), None);
        assert_eq!(match_wake("Pop it", "Poppi"), Some(String::new()));
        assert_eq!(
            match_wake("Hey computer, lock", "hey computer"),
            Some("lock".into())
        );
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Runs raw 16 kHz s16le files through the detector, whisper and the
    /// matcher: `POPPI_WAKE_SAMPLES="a.raw b.raw" cargo test wake_on_real_speech -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn wake_on_real_speech() {
        let config = VoiceConfig::default();
        for path in std::env::var("POPPI_WAKE_SAMPLES")
            .unwrap()
            .split_whitespace()
        {
            let audio = std::fs::read(path).unwrap();
            let mut vad = Vad::default();
            for frame in audio.as_chunks::<FRAME_BYTES>().0 {
                if let Some(segment) = vad.push(frame) {
                    let started = std::time::Instant::now();
                    let text = voice::transcribe(&config, &segment, Pass::Wake);
                    println!(
                        "{}: {:.1}s speech -> {:?} in {:.2}s -> wake {:?}",
                        path.rsplit('/').next().unwrap(),
                        segment.len() as f32 / BYTES_PER_SECOND as f32,
                        text,
                        started.elapsed().as_secs_f32(),
                        text.as_ref()
                            .ok()
                            .and_then(|t| match_wake(t, &config.wake_phrase)),
                    );
                }
            }
        }
    }
}
