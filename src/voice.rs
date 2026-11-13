//! Voice dictation: stream the microphone from PipeWire (`pw-record`) or
//! ALSA (`arecord`) into memory as 16 kHz mono 16-bit PCM, and turn it into
//! text with a speech-to-text command (whisper.cpp's `whisper-cli` unless
//! configured). With whisper-cli the audio so far is transcribed again and
//! again while recording, so the text appears as you speak.
//!
//! Audio only touches the disk as a short-lived WAV file per transcription
//! pass, deleted as soon as that pass is done.

use crate::config::VoiceConfig;
use crate::files::expand_tilde;
use crate::utils;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const RECORDERS: [&str; 2] = ["pw-record", "arecord"];
const WHISPER_CLI: &str = "whisper-cli";
const SAMPLE_RATE: u32 = 16_000;
/// 16-bit mono.
pub const BYTES_PER_SECOND: usize = SAMPLE_RATE as usize * 2;
/// Less than this is a click, not speech.
const MIN_AUDIO: usize = BYTES_PER_SECOND / 4;
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
/// whisper's encoder covers 30 s in 1500 frames (20 ms each).
const FULL_AUDIO_CTX: usize = 1500;
/// Narrower windows than this (~10 s) lose accuracy.
const MIN_AUDIO_CTX: usize = 512;

/// The recorder found on `$PATH`, PipeWire first.
fn recorder() -> Option<&'static str> {
    RECORDERS
        .into_iter()
        .find(|program| utils::has_command(program))
}

/// Raw 16 kHz mono s16le PCM on stdout.
fn recorder_args(program: &str) -> Vec<&'static str> {
    match program {
        "pw-record" => vec![
            "--raw",
            "--rate",
            "16000",
            "--channels",
            "1",
            "--format",
            "s16",
            "-",
        ],
        _ => vec!["-q", "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw"],
    }
}

/// Why dictation can't be used with this configuration, or `None` if it can.
pub fn unavailable_reason(config: &VoiceConfig) -> Option<String> {
    if !config.enabled {
        return Some("Voice input is turned off in Settings".to_string());
    }
    if recorder().is_none() {
        return Some("Voice input needs pw-record (PipeWire) or arecord".to_string());
    }
    if config.command.trim().is_empty() {
        if !utils::has_command(WHISPER_CLI) {
            return Some(
                "Voice input needs whisper-cli (whisper.cpp) or a [voice] command".to_string(),
            );
        }
        let model = expand_tilde(&config.model);
        if !model.is_file() {
            return Some(format!("Speech model not found: {}", config.model));
        }
    }
    None
}

/// Whether text is shown while speaking. Only with the built-in whisper-cli:
/// a custom command may be slow, or a paid service billed per call.
pub fn live(config: &VoiceConfig) -> bool {
    config.command.trim().is_empty()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pass {
    /// While speaking: fast greedy decoding, may be revised.
    Partial,
    /// After speaking: beam search for the best result.
    Final,
}

/// whisper-cli's `-ac`: an encoder window just wide enough for the audio
/// makes short clips several times faster. 0 means the full 30 s.
fn audio_ctx(audio_bytes: usize) -> usize {
    let frames = audio_bytes * 50 / BYTES_PER_SECOND + 64;
    if frames >= FULL_AUDIO_CTX {
        0
    } else {
        frames.next_multiple_of(64).max(MIN_AUDIO_CTX)
    }
}

/// The program and arguments that transcribe `file`, printing the text.
fn transcriber(
    config: &VoiceConfig,
    file: &Path,
    audio_bytes: usize,
    pass: Pass,
) -> (String, Vec<String>) {
    let file = file.to_string_lossy().into_owned();
    let command = config.command.trim();
    if !command.is_empty() {
        let script = command.replace("{file}", &utils::shell_quote(&file));
        return ("sh".to_string(), vec!["-c".to_string(), script]);
    }
    let model = expand_tilde(&config.model).to_string_lossy().into_owned();
    let language = match config.language.trim() {
        "" => "auto".to_string(),
        language => language.to_string(),
    };
    let ctx = audio_ctx(audio_bytes).to_string();
    let mut args: Vec<String> = ["-m", &model, "-l", &language, "-nt", "-np", "-ac", &ctx]
        .map(String::from)
        .to_vec();
    if pass == Pass::Partial {
        args.extend(["-bs", "1", "-bo", "1", "-nf"].map(String::from));
    }
    args.extend(["-f".to_string(), file]);
    (WHISPER_CLI.to_string(), args)
}

/// Speech-to-text output as a search query: one line, without non-speech
/// markers like `[BLANK_AUDIO]` or `(wind blowing)` and without the full
/// stop that ends a sentence (a question mark stays: it asks the AI).
fn clean_transcript(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut depth = 0usize;
    for c in raw.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.strip_suffix('.').unwrap_or(&text).trim().to_string()
}

/// 16 kHz mono 16-bit PCM wrapped in a WAV header.
fn wav(pcm: &[u8]) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(BYTES_PER_SECOND as u32).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// A fresh file per transcription pass: passes can overlap, and each one
/// deletes its own file when done.
fn audio_path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("poppi-voice-{}-{}.wav", std::process::id(), n))
}

/// The recorder process, with `POPPI_UI_RECORDER` standing in for the
/// microphone in UI tests.
fn recorder_command() -> Result<(Command, &'static str), String> {
    #[cfg(feature = "ui-test")]
    if let Ok(script) = std::env::var("POPPI_UI_RECORDER") {
        let mut command = Command::new("sh");
        command.args(["-c", &script]);
        return Ok((command, "the test recorder"));
    }
    let program = recorder().ok_or("Voice input needs pw-record or arecord")?;
    let mut command = Command::new(program);
    command.args(recorder_args(program));
    Ok((command, program))
}

/// The microphone being recorded into memory. Dropping it stops the
/// recorder and discards the audio, which is how a recording is cancelled.
pub struct Recording {
    child: Child,
    audio: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
}

impl Recording {
    pub fn start() -> Result<Self, String> {
        let (mut command, name) = recorder_command()?;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start {}: {}", name, e))?;
        let mut stdout = child.stdout.take().ok_or("Recorder has no output")?;
        let audio = Arc::new(Mutex::new(Vec::with_capacity(BYTES_PER_SECOND * 10)));
        let sink = audio.clone();
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Ok(mut audio) = sink.lock() {
                    audio.extend_from_slice(&buf[..n]);
                }
            }
        });
        Ok(Self {
            child,
            audio,
            reader: Some(reader),
        })
    }

    /// How much has been recorded so far, in bytes.
    pub fn len(&self) -> usize {
        self.audio.lock().map(|audio| audio.len()).unwrap_or(0)
    }

    /// A copy of the audio so far, for a partial transcription.
    pub fn snapshot(&self) -> Vec<u8> {
        self.audio
            .lock()
            .map(|audio| audio.clone())
            .unwrap_or_default()
    }

    /// Stops recording and returns all the audio. Blocks briefly while the
    /// recorder flushes what it still holds.
    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        if let Ok(Some(_)) = self.child.try_wait() {
            return Err("Recording failed. Is a microphone connected?".to_string());
        }
        // SIGINT, not SIGKILL, so the recorder writes out its last buffer.
        let _ = Command::new("kill")
            .args(["-INT", &self.child.id().to_string()])
            .status();
        let deadline = Instant::now() + STOP_TIMEOUT;
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        let audio = std::mem::take(&mut *self.audio.lock().map_err(|_| "Recording failed")?);
        if audio.len() < MIN_AUDIO {
            return Err("Nothing was recorded".to_string());
        }
        Ok(audio)
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Turns 16 kHz mono PCM into text. Blocking.
pub fn transcribe(config: &VoiceConfig, pcm: &[u8], pass: Pass) -> Result<String, String> {
    if pcm.len() < MIN_AUDIO {
        return Err("Didn't catch that".to_string());
    }
    let file = audio_path();
    std::fs::write(&file, wav(pcm)).map_err(|e| format!("Could not save the recording: {}", e))?;
    let (program, args) = transcriber(config, &file, pcm.len(), pass);
    let output = Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .output();
    let _ = std::fs::remove_file(&file);
    let output = output.map_err(|e| format!("Could not run {}: {}", program, e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr.lines().rev().find(|line| !line.trim().is_empty());
        return Err(match line {
            Some(line) => format!("Speech to text failed: {}", line.trim()),
            None => "Speech to text failed".to_string(),
        });
    }
    let text = clean_transcript(&String::from_utf8_lossy(&output.stdout));
    if text.is_empty() {
        return Err("Didn't catch that".to_string());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(command: &str) -> VoiceConfig {
        VoiceConfig {
            command: command.to_string(),
            ..VoiceConfig::default()
        }
    }

    fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let i = args.iter().position(|a| a == flag)?;
        args.get(i + 1).map(String::as_str)
    }

    #[test]
    fn recorders_stream_raw_16khz_mono_to_stdout() {
        assert_eq!(
            recorder_args("pw-record").join(" "),
            "--raw --rate 16000 --channels 1 --format s16 -"
        );
        let alsa = recorder_args("arecord").join(" ");
        assert!(alsa.ends_with("-r 16000 -c 1 -t raw"));
    }

    #[test]
    fn wav_header_describes_the_pcm() {
        let out = wav(&[1, 2, 3, 4]);
        assert_eq!(out.len(), 48);
        assert_eq!(&out[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), 40);
        assert_eq!(&out[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(out[24..28].try_into().unwrap()), 16_000);
        assert_eq!(&out[36..40], b"data");
        assert_eq!(u32::from_le_bytes(out[40..44].try_into().unwrap()), 4);
        assert_eq!(&out[44..], &[1, 2, 3, 4]);
    }

    #[test]
    fn encoder_window_fits_the_audio() {
        assert_eq!(audio_ctx(2 * BYTES_PER_SECOND), 512);
        assert_eq!(audio_ctx(15 * BYTES_PER_SECOND), 832);
        assert_eq!(audio_ctx(29 * BYTES_PER_SECOND), 0);
    }

    #[test]
    fn whisper_cli_by_default_or_a_custom_command() {
        let file = Path::new("/tmp/it's.wav");
        let two_seconds = 2 * BYTES_PER_SECOND;
        let (program, args) = transcriber(&config(""), file, two_seconds, Pass::Final);
        assert_eq!(program, "whisper-cli");
        assert_eq!(args[args.len() - 1], "/tmp/it's.wav");
        assert_eq!(arg_after(&args, "-l"), Some("auto"));
        assert_eq!(arg_after(&args, "-ac"), Some("512"));
        assert_eq!(
            arg_after(&args, "-bs"),
            None,
            "final passes keep beam search"
        );

        let (_, partial) = transcriber(&config(""), file, two_seconds, Pass::Partial);
        assert_eq!(arg_after(&partial, "-bs"), Some("1"));
        assert!(partial.contains(&"-nf".to_string()));

        let custom = config("stt --in {file}");
        let (program, args) = transcriber(&custom, file, two_seconds, Pass::Partial);
        assert_eq!(program, "sh");
        assert_eq!(
            args[1],
            format!("stt --in {}", utils::shell_quote("/tmp/it's.wav"))
        );
        assert!(live(&config("")));
        assert!(!live(&custom));
    }

    #[test]
    fn each_pass_gets_its_own_file() {
        assert_ne!(audio_path(), audio_path());
    }

    #[test]
    fn transcripts_become_one_line_queries() {
        assert_eq!(clean_transcript(" Open Firefox.\n"), "Open Firefox");
        assert_eq!(
            clean_transcript(" [BLANK_AUDIO]\n What is the\n capital of France?\n"),
            "What is the capital of France?"
        );
        assert_eq!(clean_transcript("(wind blowing) [Music]"), "");
        assert_eq!(
            clean_transcript("calculate 2.5 times 4."),
            "calculate 2.5 times 4"
        );
    }

    /// Records the default microphone for a second; run by hand with
    /// `cargo test records_from_the_microphone -- --ignored`.
    #[test]
    #[ignore]
    fn records_from_the_microphone() {
        let recording = Recording::start().expect("recorder starts");
        std::thread::sleep(Duration::from_millis(1000));
        assert!(
            recording.len() > BYTES_PER_SECOND / 2,
            "audio streams in while recording"
        );
        let audio = recording.finish().expect("recording finishes");
        assert!(
            audio.len() > BYTES_PER_SECOND * 3 / 4,
            "only {} bytes",
            audio.len()
        );
    }

    #[test]
    fn custom_commands_get_a_wav_file_and_failures_are_explained() {
        let second = vec![0u8; BYTES_PER_SECOND];
        let check = "test \"$(head -c 4 {file})\" = RIFF && echo ' Lock screen.'";
        assert_eq!(
            transcribe(&config(check), &second, Pass::Final),
            Ok("Lock screen".to_string())
        );
        assert_eq!(
            transcribe(
                &config("echo 'model missing' >&2; exit 3"),
                &second,
                Pass::Final
            ),
            Err("Speech to text failed: model missing".to_string())
        );
        assert_eq!(
            transcribe(&config("echo '[BLANK_AUDIO]'"), &second, Pass::Final),
            Err("Didn't catch that".to_string())
        );
        assert_eq!(
            transcribe(&config("echo never run"), &[0u8; 100], Pass::Final),
            Err("Didn't catch that".to_string())
        );
        let leftovers = std::fs::read_dir(audio_path().parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with(&format!("poppi-voice-{}-", std::process::id()))
            })
            .count();
        assert_eq!(leftovers, 0, "every pass deletes its file");
    }
}
