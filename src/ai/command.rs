//! Other command-line assistants, one process per question, each on its own
//! login: Gemini CLI, Codex, or any command that reads a prompt on stdin and
//! prints the answer.

use super::{
    resolve_command, single_prompt, working_dir, Event, Job, Kind, Message, Outcome, Resolved,
    Stream,
};
use std::path::PathBuf;
use std::process::Command;

pub fn job(model: &Resolved, history: &[Message], question: &str) -> Result<Job, String> {
    let prompt = single_prompt(&model.system_prompt, history, question);
    let program = || {
        resolve_command(&model.command).ok_or_else(|| {
            format!(
                "Could not find `{}`. Install {} and log in once in a terminal.",
                model.command, model.provider_name
            )
        })
    };

    let job = match model.kind {
        Kind::GeminiCli => {
            let mut command = Command::new(program()?);
            command.args(gemini_args(&model.model, &prompt));
            Job {
                command,
                input: None,
                stream: Box::new(TextStream::new(&model.provider_name)),
            }
        }
        Kind::Codex => {
            let answer_file = std::env::temp_dir().join(format!(
                "poppi-codex-{}-{}.txt",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let mut command = Command::new(program()?);
            command.args(codex_args(&model.model, &answer_file));
            Job {
                command,
                input: Some(prompt.into_bytes()),
                stream: Box::new(CodexStream { answer_file }),
            }
        }
        _ => {
            let mut command = Command::new("sh");
            command.args(["-c", &model.command]);
            Job {
                command,
                input: Some(prompt.into_bytes()),
                stream: Box::new(TextStream::new(&model.provider_name)),
            }
        }
    };
    let mut job = job;
    job.command.current_dir(working_dir());
    Ok(job)
}

/// Headless mode, plain text out. Tools that would need approval are not
/// available without a terminal, so it can only answer.
pub fn gemini_args(model: &str, prompt: &str) -> Vec<String> {
    let mut args = vec![
        "--output-format".to_string(),
        "text".to_string(),
        "--prompt".to_string(),
        prompt.to_string(),
    ];
    if !model.is_empty() {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    args
}

/// Read-only sandbox; the final answer is written to a file, progress
/// goes to the terminal output, which is ignored.
pub fn codex_args(model: &str, answer_file: &std::path::Path) -> Vec<String> {
    let mut args: Vec<String> = [
        "exec",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--color",
        "never",
        "--output-last-message",
    ]
    .map(String::from)
    .to_vec();
    args.push(answer_file.to_string_lossy().to_string());
    if !model.is_empty() {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    // The prompt comes on stdin.
    args.push("-".to_string());
    args
}

/// Plain text on stdout, streamed line by line.
pub struct TextStream {
    name: String,
    text: String,
}

impl TextStream {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            text: String::new(),
        }
    }
}

impl Stream for TextStream {
    fn line(&mut self, line: &str) -> Option<Event> {
        let delta = if self.text.is_empty() {
            if line.trim().is_empty() {
                return None;
            }
            line.to_string()
        } else {
            format!("\n{}", line)
        };
        self.text.push_str(&delta);
        Some(Event::Delta(delta))
    }

    fn finish(self: Box<Self>, outcome: Outcome) -> Option<Event> {
        let text = self.text.trim();
        Some(if outcome.success && !text.is_empty() {
            Event::Done(text.to_string())
        } else if outcome.success {
            Event::Failed(format!("{} sent an empty answer.", self.name))
        } else {
            Event::Failed(
                outcome.error_line().unwrap_or_else(|| {
                    format!("{} failed (exit code {:?}).", self.name, outcome.code)
                }),
            )
        })
    }
}

struct CodexStream {
    answer_file: PathBuf,
}

impl Stream for CodexStream {
    fn line(&mut self, _line: &str) -> Option<Event> {
        None
    }

    fn finish(self: Box<Self>, outcome: Outcome) -> Option<Event> {
        let answer = std::fs::read_to_string(&self.answer_file).unwrap_or_default();
        let _ = std::fs::remove_file(&self.answer_file);
        Some(if outcome.success && !answer.trim().is_empty() {
            Event::Done(answer.trim().to_string())
        } else {
            Event::Failed(
                outcome
                    .error_line()
                    .unwrap_or_else(|| "Codex did not answer.".to_string()),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_keeps_line_breaks_and_skips_leading_blanks() {
        let mut stream = TextStream::new("Gemini CLI");
        let events: Vec<Event> = ["", "Hello", "", "- one"]
            .iter()
            .filter_map(|l| stream.line(l))
            .collect();
        assert_eq!(
            events,
            [
                Event::Delta("Hello".into()),
                Event::Delta("\n".into()),
                Event::Delta("\n- one".into())
            ]
        );
        assert_eq!(
            Box::new(stream).finish(Outcome {
                success: true,
                ..Outcome::default()
            }),
            Some(Event::Done("Hello\n\n- one".into()))
        );
    }

    #[test]
    fn failures_show_the_last_error_line() {
        let stream = Box::new(TextStream::new("Gemini CLI"));
        let outcome = Outcome {
            success: false,
            code: Some(41),
            stderr: "Loaded cached credentials.\nPlease set an Auth method\n\n".into(),
        };
        assert_eq!(
            stream.finish(outcome),
            Some(Event::Failed("Please set an Auth method".into()))
        );
    }

    #[test]
    fn cli_arguments() {
        let gemini = gemini_args("gemini-flash-latest", "hi");
        assert_eq!(
            gemini[gemini.iter().position(|a| a == "--prompt").unwrap() + 1],
            "hi"
        );
        assert!(gemini.contains(&"gemini-flash-latest".to_string()));
        assert!(!gemini_args("", "hi").contains(&"--model".to_string()));

        let codex = codex_args("", std::path::Path::new("/tmp/a.txt"));
        assert_eq!(codex.first().map(String::as_str), Some("exec"));
        assert_eq!(codex.last().map(String::as_str), Some("-"));
        assert!(codex.windows(2).any(|w| w == ["--sandbox", "read-only"]));
    }
}
