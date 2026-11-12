//! Claude through the Claude Code CLI.
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

use super::{resolve_command, working_dir, Event};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// A running CLI process. Dropping it kills the process.
pub struct Process {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Option<ChildStdin>,
}

impl Process {
    /// Put a message to Claude. The answer arrives on the event channel.
    pub fn send(&mut self, text: &str) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("Claude is not running")?;
        writeln!(stdin, "{}", user_message(text))
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("Could not reach Claude: {}", e))
    }
}

impl Drop for Process {
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

pub fn build_args(model: &str, system_prompt: &str) -> Vec<String> {
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
    if !model.trim().is_empty() {
        args.extend(["--model".to_string(), model.trim().to_string()]);
    }
    if !system_prompt.trim().is_empty() {
        args.extend(["--system-prompt".to_string(), system_prompt.to_string()]);
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

impl Process {
    /// Start the CLI. Nothing is sent to Claude until `send` is called, so
    /// this can run ahead of time to hide the start-up delay.
    pub fn start(
        program: &str,
        model: &str,
        system_prompt: &str,
        sender: Sender<Event>,
    ) -> Result<Process, String> {
        let path = resolve_command(program).ok_or_else(|| {
            format!(
                "Could not find `{}`. Install Claude Code and run `claude` once to log in.",
                program
            )
        })?;

        let mut command = Command::new(path);
        command
            .args(build_args(model, system_prompt))
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

        Ok(Process { child, stdin })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_deltas_are_streamed() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"The capital of Sl"}}}"#;
        assert_eq!(
            parse_line(line),
            Some(Event::Delta("The capital of Sl".into()))
        );
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
        let ok =
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Ljubljana.\n"}"#;
        assert_eq!(parse_line(ok), Some(Event::Done("Ljubljana.".into())));

        let failed =
            r#"{"type":"result","subtype":"success","is_error":true,"result":"Please run /login"}"#;
        assert_eq!(
            parse_line(failed),
            Some(Event::Failed("Please run /login".into()))
        );

        let bare = r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#;
        assert_eq!(
            parse_line(bare),
            Some(Event::Failed("error_during_execution".into()))
        );
    }

    #[test]
    fn arguments_disable_tools_and_use_the_streaming_protocol() {
        let args = build_args("haiku", "be brief");
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

    #[test]
    fn empty_system_prompt_is_omitted() {
        assert!(!build_args("haiku", "  ").contains(&"--system-prompt".to_string()));
        assert!(!build_args("", "").contains(&"--model".to_string()));
    }
}
