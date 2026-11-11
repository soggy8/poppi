//! Ask Claude a question through the Claude Code CLI.
//!
//! This deliberately has no API key handling: it runs `claude -p`, which uses
//! whatever account the CLI is logged in with (a Pro/Max subscription after
//! `claude` + `/login`). `ANTHROPIC_API_KEY` is removed from the child's
//! environment so a key that happens to be exported is never billed instead.
//!
//! The CLI is run with no tools, no MCP servers and no session persistence:
//! it can only answer in text, which also keeps start-up short.

use crate::config::ClaudeConfig;
use crate::utils;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A piece of the answer as it streams in.
    Delta(String),
    /// The request finished; carries the complete answer.
    Done(String),
    Failed(String),
}

/// A running request. Dropping it (or calling `cancel`) kills the process.
pub struct Request {
    events: Receiver<Event>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Request {
    /// Events that have arrived since the last call, without blocking.
    pub fn poll(&self) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        events
    }

    pub fn cancel(&self) {
        if let Some(mut child) = self.child.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Find the CLI. A launcher started from a desktop shortcut often has a
/// shorter `$PATH` than a terminal, so the usual install locations are tried.
fn resolve_command(command: &str) -> Option<PathBuf> {
    if let Some(path) = utils::find_in_path(command) {
        return Some(path);
    }
    if command.contains('/') {
        return None;
    }
    let home = dirs::home_dir()?;
    [".local/bin", ".claude/local", ".npm-global/bin", "bin"]
        .iter()
        .map(|dir| home.join(dir).join(command))
        .chain([PathBuf::from("/usr/local/bin").join(command)])
        .find(|candidate| utils::is_executable(candidate))
}

pub fn build_args(config: &ClaudeConfig, question: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        // Text answers only: no file access, shell, web or MCP tools.
        "--tools",
        "",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
    ]
    .map(String::from)
    .to_vec();
    args.extend(["--model".to_string(), config.model.clone()]);
    if !config.system_prompt.trim().is_empty() {
        args.extend(["--system-prompt".to_string(), config.system_prompt.clone()]);
    }
    // "--" so a question starting with a dash is never read as an option.
    args.extend(["--".to_string(), question.to_string()]);
    args
}

/// Interpret one line of `--output-format stream-json` output.
pub fn parse_line(line: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    match value.get("type")?.as_str()? {
        "stream_event" => {
            let event = value.get("event")?;
            if event.get("type")?.as_str()? != "content_block_delta" {
                return None;
            }
            let delta = event.get("delta")?;
            if delta.get("type")?.as_str()? != "text_delta" {
                return None;
            }
            Some(Event::Delta(delta.get("text")?.as_str()?.to_string()))
        }
        "result" => {
            let text = value
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let is_error = value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if is_error {
                let reason = if text.is_empty() {
                    value
                        .get("subtype")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string()
                } else {
                    text
                };
                Some(Event::Failed(reason))
            } else {
                Some(Event::Done(text))
            }
        }
        _ => None,
    }
}

/// A neutral, empty working directory, so the CLI does not pick up a
/// project's CLAUDE.md or settings from wherever the launcher was started.
fn working_dir() -> PathBuf {
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("poppi_launcher")
        .join("claude");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn start(config: &ClaudeConfig, question: &str) -> Result<Request, String> {
    let program = resolve_command(&config.command).ok_or_else(|| {
        format!(
            "Could not find `{}`. Install Claude Code and run `claude` once to log in.",
            config.command
        )
    })?;

    let mut command = Command::new(program);
    command
        .args(build_args(config, question))
        .current_dir(working_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Subscription login only: never fall back to pay-per-use API billing.
    for key in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
        command.env_remove(key);
    }
    // If the launcher was started from inside a Claude Code session, do not
    // let the child think it is part of that session.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name == "CLAUDECODE" || name == "CLAUDE_PID" || name.starts_with("CLAUDE_CODE_") {
            command.env_remove(&key);
        }
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not start Claude: {}", e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(Some(child)));
    let (sender, events) = mpsc::channel();

    let child_for_thread = child.clone();
    std::thread::spawn(move || {
        // Drain stderr on its own thread so a chatty CLI cannot block.
        let stderr_reader = std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut stderr) = stderr {
                let _ = stderr.read_to_string(&mut text);
            }
            text
        });

        let mut finished = false;
        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(event) = parse_line(&line) {
                    finished |= !matches!(event, Event::Delta(_));
                    if sender.send(event).is_err() {
                        return;
                    }
                }
            }
        }

        let errors = stderr_reader.join().unwrap_or_default();
        // `None` here means the request was cancelled; nobody is listening.
        let exited = child_for_thread
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
            .map(|mut child| child.wait());
        if finished {
            return;
        }
        if let Some(status) = exited {
            let detail = errors.trim();
            let message = if !detail.is_empty() {
                detail.lines().last().unwrap_or(detail).to_string()
            } else {
                match status {
                    Ok(status) => format!("Claude exited without an answer ({})", status),
                    Err(e) => format!("Claude failed: {}", e),
                }
            };
            let _ = sender.send(Event::Failed(message));
        }
    });

    Ok(Request { events, child })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_deltas_are_streamed() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"The capital of Sl"}}}"#;
        assert_eq!(parse_line(line), Some(Event::Delta("The capital of Sl".into())));
    }

    #[test]
    fn thinking_and_bookkeeping_lines_are_ignored() {
        for line in [
            r#"{"type":"system","subtype":"init","model":"claude-haiku-5-5"}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"x"}]}}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#,
            "not json at all",
            "",
        ] {
            assert_eq!(parse_line(line), None, "{}", line);
        }
    }

    #[test]
    fn result_lines_finish_the_request() {
        let ok = r#"{"type":"result","subtype":"success","is_error":false,"result":"Ljubljana.\n"}"#;
        assert_eq!(parse_line(ok), Some(Event::Done("Ljubljana.".into())));

        let failed = r#"{"type":"result","subtype":"success","is_error":true,"result":"Please run /login"}"#;
        assert_eq!(parse_line(failed), Some(Event::Failed("Please run /login".into())));

        let bare = r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#;
        assert_eq!(
            parse_line(bare),
            Some(Event::Failed("error_during_execution".into()))
        );
    }

    #[test]
    fn arguments_disable_tools_and_pass_the_question_last() {
        let config = ClaudeConfig::default();
        let args = build_args(&config, "--help me?");
        let position = |flag: &str| args.iter().position(|a| a == flag).unwrap();

        assert_eq!(args[position("--tools") + 1], "");
        assert_eq!(args[position("--model") + 1], "haiku");
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        assert!(args.contains(&"--no-session-persistence".to_string()));
        assert_eq!(&args[args.len() - 2..], ["--", "--help me?"]);
        assert!(!args.iter().any(|a| a.to_lowercase().contains("api")));
    }

    /// Manual check that really asks Claude (one small request on the
    /// logged-in account): `cargo test -- --ignored --nocapture asks_claude`
    #[test]
    #[ignore = "needs a logged-in Claude Code CLI and network access"]
    fn asks_claude_for_real() {
        let request = start(&ClaudeConfig::default(), "What is 2+2? Answer with the number only.")
            .expect("could not start the CLI");
        let started = std::time::Instant::now();
        let mut deltas = String::new();
        loop {
            for event in request.poll() {
                match event {
                    Event::Delta(text) => deltas.push_str(&text),
                    Event::Done(answer) => {
                        println!("answer {:?} after {:?}", answer, started.elapsed());
                        assert!(answer.contains('4'));
                        assert_eq!(deltas.trim(), answer);
                        return;
                    }
                    Event::Failed(error) => panic!("request failed: {}", error),
                }
            }
            assert!(started.elapsed().as_secs() < 60, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn empty_system_prompt_is_omitted() {
        let config = ClaudeConfig {
            system_prompt: "  ".to_string(),
            ..ClaudeConfig::default()
        };
        assert!(!build_args(&config, "why?").contains(&"--system-prompt".to_string()));
    }
}
