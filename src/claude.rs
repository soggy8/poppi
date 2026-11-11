//! Ask Claude questions through the Claude Code CLI.
//!
//! This deliberately has no API key handling: it runs `claude -p`, which uses
//! whatever account the CLI is logged in with (a Pro/Max subscription after
//! `claude` + `/login`). `ANTHROPIC_API_KEY` is removed from the child's
//! environment so a key that happens to be exported is never billed instead.
//!
//! One CLI process serves a whole conversation: questions are written to its
//! stdin as JSON lines and answers stream back on stdout. That keeps the
//! context for follow-up questions and pays the CLI's start-up time once.
//! The process runs with no tools, no MCP servers and no session
//! persistence: it can only answer in text.

use crate::config::ClaudeConfig;
use crate::utils;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A piece of the answer as it streams in.
    Delta(String),
    /// The request finished; carries the complete answer.
    Done(String),
    Failed(String),
}

/// A running conversation. Dropping it kills the process.
pub struct Session {
    events: Receiver<Event>,
    child: Arc<Mutex<Option<Child>>>,
    stdin: Option<ChildStdin>,
}

impl Session {
    /// Put a question to Claude. The answer arrives through `poll`.
    pub fn send(&mut self, question: &str) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("Claude is not running")?;
        writeln!(stdin, "{}", user_message(question))
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("Could not reach Claude: {}", e))
    }

    /// Events that have arrived since the last call, without blocking.
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Closing stdin asks the CLI to finish; killing makes sure it does.
        self.stdin.take();
        if let Some(mut child) = self.child.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// One user turn in the CLI's `--input-format stream-json` protocol.
pub fn user_message(question: &str) -> String {
    json!({
        "type": "user",
        "message": { "role": "user", "content": question }
    })
    .to_string()
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

pub fn build_args(config: &ClaudeConfig) -> Vec<String> {
    let mut args: Vec<String> = [
        "--print",
        "--input-format",
        "stream-json",
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

impl Session {
    /// Start the CLI. Nothing is sent to Claude until `send` is called, so
    /// this can run ahead of time to hide the start-up delay.
    pub fn start(config: &ClaudeConfig) -> Result<Session, String> {
        let program = resolve_command(&config.command).ok_or_else(|| {
            format!(
                "Could not find `{}`. Install Claude Code and run `claude` once to log in.",
                config.command
            )
        })?;

        let mut command = Command::new(program);
        command
            .args(build_args(config))
            .current_dir(working_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Subscription login only: never fall back to pay-per-use API billing.
        for key in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
            command.env_remove(key);
        }
        // If the launcher was started from inside a Claude Code session, do
        // not let the child think it is part of that session.
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name == "CLAUDECODE" || name == "CLAUDE_PID" || name.starts_with("CLAUDE_CODE_") {
                command.env_remove(&key);
            }
        }

        let mut child = command
            .spawn()
            .map_err(|e| format!("Could not start Claude: {}", e))?;
        let stdin = child.stdin.take();
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

            if let Some(stdout) = stdout {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Some(event) = parse_line(&line) {
                        if sender.send(event).is_err() {
                            return;
                        }
                    }
                }
            }

            // stdout closed: the process is gone. If the slot is empty we
            // killed it ourselves and nobody is listening.
            let errors = stderr_reader.join().unwrap_or_default();
            let exited = child_for_thread
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
                .map(|mut child| child.wait());
            if let Some(status) = exited {
                let detail = errors.trim();
                let message = if !detail.is_empty() {
                    detail.lines().last().unwrap_or(detail).to_string()
                } else {
                    match status {
                        Ok(status) => format!("Claude stopped unexpectedly ({})", status),
                        Err(e) => format!("Claude failed: {}", e),
                    }
                };
                let _ = sender.send(Event::Failed(message));
            }
        });

        Ok(Session {
            events,
            child,
            stdin,
        })
    }
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
    fn arguments_disable_tools_and_use_the_streaming_protocol() {
        let config = ClaudeConfig::default();
        let args = build_args(&config);
        let position = |flag: &str| args.iter().position(|a| a == flag).unwrap();

        assert_eq!(args[position("--tools") + 1], "");
        assert_eq!(args[position("--model") + 1], "haiku");
        assert_eq!(args[position("--input-format") + 1], "stream-json");
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        assert!(args.contains(&"--no-session-persistence".to_string()));
        assert!(!args.iter().any(|a| a.to_lowercase().contains("api")));
    }

    #[test]
    fn questions_are_json_encoded() {
        let line = user_message("say \"hi\"\nplease --now");
        assert!(!line.contains('\n'));
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["type"], "user");
        assert_eq!(value["message"]["role"], "user");
        assert_eq!(value["message"]["content"], "say \"hi\"\nplease --now");
    }

    /// Manual check that really asks Claude (two small requests on the
    /// logged-in account): `cargo test -- --ignored --nocapture asks_claude`
    #[test]
    #[ignore = "needs a logged-in Claude Code CLI and network access"]
    fn asks_claude_for_real_and_remembers_context() {
        fn answer(session: &Session) -> String {
            let started = std::time::Instant::now();
            let mut deltas = String::new();
            loop {
                for event in session.poll() {
                    match event {
                        Event::Delta(text) => deltas.push_str(&text),
                        Event::Done(answer) => {
                            println!("answer {:?} after {:?}", answer, started.elapsed());
                            assert_eq!(deltas.trim(), answer);
                            return answer;
                        }
                        Event::Failed(error) => panic!("request failed: {}", error),
                    }
                }
                assert!(started.elapsed().as_secs() < 60, "timed out");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }

        let mut session = Session::start(&ClaudeConfig::default()).expect("could not start the CLI");
        session.send("My favourite number is 41. Reply with just: ok").unwrap();
        answer(&session);
        session.send("What is my favourite number plus one? Reply with the number only.").unwrap();
        assert!(answer(&session).contains("42"));
    }

    #[test]
    fn empty_system_prompt_is_omitted() {
        let config = ClaudeConfig {
            system_prompt: "  ".to_string(),
            ..ClaudeConfig::default()
        };
        assert!(!build_args(&config).contains(&"--system-prompt".to_string()));
    }
}
