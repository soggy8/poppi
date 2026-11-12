//! Asking AI models.
//!
//! Three kinds of backend, all delivering `Event`s on one channel:
//! - `claude_cli`: one Claude Code process per conversation, on the CLI's
//!   own login (a Pro/Max subscription; API keys are removed from its
//!   environment).
//! - `command`: one process per question for other CLIs (Gemini CLI,
//!   Codex, or any shell command), each on its own login.
//! - `http`: one `curl` per question for APIs (OpenAI-compatible or
//!   Anthropic), with the key from the config, the GNOME keyring or an
//!   environment variable. Local servers such as Ollama need no key.
//!
//! Only the CLI keeps context by itself; for the others the earlier turns
//! are sent along with every question.

mod claude_cli;
mod command;
mod http;
pub mod providers;

pub use providers::{Kind, Resolved};

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A piece of the answer as it streams in.
    Delta(String),
    /// The request finished; carries the complete answer.
    Done(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: &str) -> Self {
        Self {
            role: Role::User,
            content: content.to_string(),
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: Role::Assistant,
            content: content.to_string(),
        }
    }
}

/// The earlier turns written out, for backends that take a single prompt.
pub fn with_context(history: &[Message], question: &str) -> String {
    if history.is_empty() {
        return question.to_string();
    }
    let mut text = String::from("Earlier in this conversation:\n\n");
    for message in history {
        let speaker = match message.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        text.push_str(&format!("{}: {}\n\n", speaker, message.content.trim()));
    }
    text.push_str("Now reply to this message:\n\n");
    text.push_str(question);
    text
}

/// System prompt, context and question as one prompt.
pub fn single_prompt(system: &str, history: &[Message], question: &str) -> String {
    let body = with_context(history, question);
    if system.trim().is_empty() {
        body
    } else {
        format!("{}\n\n{}", system.trim(), body)
    }
}

/// Find a CLI. A launcher started from a desktop shortcut often has a
/// shorter `$PATH` than a terminal, so the usual install locations are tried.
pub fn resolve_command(command: &str) -> Option<PathBuf> {
    if let Some(path) = crate::utils::find_in_path(command) {
        return Some(path);
    }
    if command.contains('/') || command.is_empty() {
        return None;
    }
    let home = dirs::home_dir()?;
    [
        ".local/bin",
        ".claude/local",
        ".npm-global/bin",
        ".bun/bin",
        ".cargo/bin",
        "bin",
    ]
    .iter()
    .map(|dir| home.join(dir).join(command))
    .chain([PathBuf::from("/usr/local/bin").join(command)])
    .find(|candidate| crate::utils::is_executable(candidate))
}

/// A neutral, empty working directory, so CLIs do not pick up a project's
/// instructions or settings from wherever the launcher was started.
pub fn working_dir() -> PathBuf {
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("poppi_launcher")
        .join("ai");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// How a one-shot process ended.
#[derive(Debug, Default)]
pub struct Outcome {
    pub success: bool,
    pub code: Option<i32>,
    pub stderr: String,
}

impl Outcome {
    /// The most useful line of the error output.
    pub fn error_line(&self) -> Option<String> {
        self.stderr
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .map(|line| line.chars().take(300).collect())
    }
}

/// Turns a process's output lines into events.
pub trait Stream: Send {
    fn line(&mut self, line: &str) -> Option<Event>;
    /// The final event once the process has exited, if any is due.
    fn finish(self: Box<Self>, outcome: Outcome) -> Option<Event>;
}

#[derive(Default)]
struct SlotState {
    child: Option<Child>,
    cancelled: bool,
}

/// A one-shot process that dies with the session.
#[derive(Clone, Default)]
struct Slot(Arc<Mutex<SlotState>>);

impl Slot {
    fn cancel(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.cancelled = true;
            if let Some(mut child) = state.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    fn finished(&self) -> bool {
        self.0
            .lock()
            .map(|state| state.child.is_none())
            .unwrap_or(true)
    }
}

/// What to run, prepared on the worker thread (key lookups may block).
pub struct Job {
    pub command: Command,
    pub input: Option<Vec<u8>>,
    pub stream: Box<dyn Stream>,
}

fn run_once(
    slot: Slot,
    prepare: impl FnOnce() -> Result<Job, String> + Send + 'static,
    sender: Sender<Event>,
) {
    std::thread::spawn(move || {
        let Job {
            mut command,
            input,
            mut stream,
        } = match prepare() {
            Ok(job) => job,
            Err(error) => {
                let _ = sender.send(Event::Failed(error));
                return;
            }
        };
        command
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                let program = command.get_program().to_string_lossy().to_string();
                let _ = sender.send(Event::Failed(format!("Could not run {}: {}", program, e)));
                return;
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        {
            let Ok(mut state) = slot.0.lock() else { return };
            if state.cancelled {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            state.child = Some(child);
        }

        // Feed and drain on their own threads so a full pipe cannot block.
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            std::thread::spawn(move || {
                let _ = stdin.write_all(&input);
            });
        }
        let stderr_reader = std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut stderr) = stderr {
                let _ = stderr.read_to_string(&mut text);
            }
            text
        });

        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(event) = stream.line(&line) {
                    if sender.send(event).is_err() {
                        return;
                    }
                }
            }
        }
        let stderr_text = stderr_reader.join().unwrap_or_default();

        // An empty slot means the session killed it and nobody is listening.
        let Some(mut child) = slot.0.lock().ok().and_then(|mut state| state.child.take()) else {
            return;
        };
        let status = child.wait().ok();
        let outcome = Outcome {
            success: status.is_some_and(|s| s.success()),
            code: status.and_then(|s| s.code()),
            stderr: stderr_text,
        };
        if let Some(event) = stream.finish(outcome) {
            let _ = sender.send(event);
        }
    });
}

/// A conversation with one model. Dropping it stops whatever is running.
pub struct Session {
    pub model: Resolved,
    sender: Sender<Event>,
    events: Receiver<Event>,
    claude: Option<claude_cli::Process>,
    /// The CLI process has not seen any of the conversation yet.
    claude_fresh: bool,
    requests: Vec<Slot>,
}

impl Session {
    /// Get ready to ask `model`. For the Claude CLI this starts the process
    /// (nothing is sent yet), which hides its start-up time.
    pub fn start(model: Resolved) -> Result<Session, String> {
        let (sender, events) = mpsc::channel();
        let claude = if model.kind == Kind::ClaudeCli {
            Some(claude_cli::Process::start(
                &model.command,
                &model.model,
                &model.system_prompt,
                sender.clone(),
            )?)
        } else {
            None
        };
        Ok(Session {
            model,
            sender,
            events,
            claude,
            claude_fresh: true,
            requests: Vec::new(),
        })
    }

    /// Ensure a warm CLI process is running when `model` uses Claude Code.
    pub fn warm_up(&mut self) -> Result<(), String> {
        if self.model.kind == Kind::ClaudeCli && self.claude.is_none() {
            self.claude = Some(claude_cli::Process::start(
                &self.model.command,
                &self.model.model,
                &self.model.system_prompt,
                self.sender.clone(),
            )?);
            self.claude_fresh = true;
        }
        Ok(())
    }

    /// Stop the current answer without discarding the session. A later
    /// follow-up reuses the session (restarting the Claude CLI if needed).
    pub fn cancel(&mut self) {
        for slot in &self.requests {
            slot.cancel();
        }
        self.requests.clear();
        if self.claude.take().is_some() {
            self.claude_fresh = true;
        }
        let (sender, events) = mpsc::channel();
        self.sender = sender;
        self.events = events;
    }

    /// Ask `question`; `history` is the conversation so far (finished turns).
    pub fn send(&mut self, history: &[Message], question: &str) -> Result<(), String> {
        if self.model.kind == Kind::ClaudeCli && self.claude.is_none() {
            self.warm_up()?;
        }
        if let Some(process) = self.claude.as_mut() {
            // A process started mid-conversation (switched model, or restarted
            // after stopping an answer) gets the earlier turns written out once.
            let text = if self.claude_fresh {
                with_context(history, question)
            } else {
                question.to_string()
            };
            process.send(&text)?;
            self.claude_fresh = false;
            return Ok(());
        }

        self.requests.retain(|slot| !slot.finished());
        let slot = Slot::default();
        self.requests.push(slot.clone());
        let model = self.model.clone();
        let history = history.to_vec();
        let question = question.to_string();
        let prepare = move || match model.kind {
            Kind::OpenAi | Kind::Anthropic => http::job(&model, &history, &question),
            Kind::GeminiCli | Kind::Codex | Kind::Command => {
                command::job(&model, &history, &question)
            }
            Kind::ClaudeCli => Err("Claude is not running".to_string()),
        };
        run_once(slot, prepare, self.sender.clone());
        Ok(())
    }

    /// Events that have arrived since the last call, without blocking.
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for slot in &self.requests {
            slot.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_carry_the_conversation() {
        assert_eq!(with_context(&[], "hi"), "hi");
        assert_eq!(single_prompt("Be brief.", &[], "hi"), "Be brief.\n\nhi");
        let history = [Message::user("my number is 41"), Message::assistant("ok")];
        let prompt = single_prompt("", &history, "plus one?");
        assert!(prompt.starts_with("Earlier in this conversation:"));
        assert!(prompt.contains("User: my number is 41\n\nAssistant: ok"));
        assert!(prompt.ends_with("plus one?"));
    }

    struct Lines(Vec<String>);
    impl Stream for Lines {
        fn line(&mut self, line: &str) -> Option<Event> {
            self.0.push(line.to_string());
            Some(Event::Delta(line.to_string()))
        }
        fn finish(self: Box<Self>, outcome: Outcome) -> Option<Event> {
            Some(if outcome.success {
                Event::Done(self.0.join("+"))
            } else {
                Event::Failed(outcome.error_line().unwrap_or_default())
            })
        }
    }

    fn collect(receiver: &Receiver<Event>) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.recv_timeout(std::time::Duration::from_secs(5)) {
            let last = !matches!(event, Event::Delta(_));
            events.push(event);
            if last {
                break;
            }
        }
        events
    }

    #[test]
    fn one_shot_processes_stream_and_finish() {
        let (sender, receiver) = mpsc::channel();
        run_once(
            Slot::default(),
            || {
                let mut command = Command::new("sh");
                command.args(["-c", "cat; echo second"]);
                Ok(Job {
                    command,
                    input: Some(b"first\n".to_vec()),
                    stream: Box::new(Lines(Vec::new())),
                })
            },
            sender.clone(),
        );
        assert_eq!(
            collect(&receiver),
            [
                Event::Delta("first".into()),
                Event::Delta("second".into()),
                Event::Done("first+second".into())
            ]
        );

        run_once(
            Slot::default(),
            || {
                let mut command = Command::new("sh");
                command.args(["-c", "echo 'not logged in' >&2; exit 3"]);
                Ok(Job {
                    command,
                    input: None,
                    stream: Box::new(Lines(Vec::new())),
                })
            },
            sender.clone(),
        );
        assert_eq!(collect(&receiver), [Event::Failed("not logged in".into())]);

        run_once(Slot::default(), || Err("no key".to_string()), sender);
        assert_eq!(collect(&receiver), [Event::Failed("no key".into())]);
    }

    /// Manual check against a real model, two turns to check the context:
    /// `POPPI_AI_TEST=provider[:model] cargo test -- --ignored --nocapture asks_for_real`
    /// (optionally `POPPI_AI_TEST_URL`, `POPPI_AI_TEST_KEY`).
    #[test]
    #[ignore = "talks to a real model"]
    fn asks_for_real() {
        let spec = std::env::var("POPPI_AI_TEST").unwrap_or_else(|_| "claude:haiku".into());
        let (provider, model) = spec.split_once(':').unwrap_or((&spec, ""));
        let mut entry = crate::config::ModelConfig::new(provider, model);
        entry.base_url = std::env::var("POPPI_AI_TEST_URL").unwrap_or_default();
        entry.api_key = std::env::var("POPPI_AI_TEST_KEY").unwrap_or_default();
        let resolved =
            providers::resolve_entry("test", &entry, crate::config::DEFAULT_SYSTEM_PROMPT).unwrap();
        println!("asking {}", resolved.label);

        let answer = |session: &Session| -> String {
            let started = std::time::Instant::now();
            let mut deltas = String::new();
            loop {
                for event in session.poll() {
                    match event {
                        Event::Delta(text) => deltas.push_str(&text),
                        Event::Done(answer) => {
                            println!(
                                "answer {:?} after {:?} ({} streamed chars)",
                                answer,
                                started.elapsed(),
                                deltas.len()
                            );
                            return answer;
                        }
                        Event::Failed(error) => panic!("request failed: {}", error),
                    }
                }
                assert!(started.elapsed().as_secs() < 180, "timed out");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };

        let mut session = Session::start(resolved).unwrap();
        let first = "My favourite number is 41. Reply with just: ok";
        session.send(&[], first).unwrap();
        let reply = answer(&session);
        let history = [Message::user(first), Message::assistant(&reply)];
        session
            .send(
                &history,
                "What is my favourite number plus one? Reply with the number only.",
            )
            .unwrap();
        println!("second: {}", answer(&session));
    }

    #[test]
    fn cancelled_processes_stay_silent() {
        let (sender, receiver) = mpsc::channel();
        let slot = Slot::default();
        run_once(
            slot.clone(),
            || {
                let mut command = Command::new("sleep");
                command.arg("30");
                Ok(Job {
                    command,
                    input: None,
                    stream: Box::new(Lines(Vec::new())),
                })
            },
            sender,
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
        let started = std::time::Instant::now();
        slot.cancel();
        assert!(started.elapsed().as_secs() < 2);
        assert!(slot.finished());
        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(500))
            .is_err());
    }

    #[test]
    fn session_cancel_stops_in_flight_turn_and_allows_follow_up() {
        let mut entry = crate::config::ModelConfig::new("command", "");
        entry.command =
            "if grep -q 'stop-me' ; then echo partial; sleep 30; else echo recovered; fi".into();
        let resolved = providers::resolve_entry("cmd", &entry, "").unwrap();
        let mut session = Session::start(resolved).unwrap();

        session.send(&[], "stop-me").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        session.cancel();
        assert!(session.poll().is_empty());

        session.send(&[], "next").unwrap();
        let started = std::time::Instant::now();
        let mut done = None;
        while started.elapsed().as_secs() < 5 {
            for event in session.poll() {
                if let Event::Done(text) = event {
                    done = Some(text);
                }
            }
            if done.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(done.as_deref(), Some("recovered"));
    }
}
