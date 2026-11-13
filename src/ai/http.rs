//! APIs over HTTP, through `curl` (already needed for exchange rates).
//!
//! The request, including the API key, is passed to curl as a config file
//! on stdin, so the key never appears on a command line where other users
//! could see it. Answers stream back as server-sent events.

use super::{Event, Job, Kind, Message, Outcome, Resolved, Stream};
use serde_json::{json, Value};
use std::process::Command;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 4096;

pub fn job(model: &Resolved, history: &[Message], question: &str) -> Result<Job, String> {
    let key = model.key.resolve();
    if key.is_none() && model.needs_key {
        let mut ways = vec!["type \"setup\" in Poppi to add one".to_string()];
        if let Some(env) = &model.key.env {
            ways.push(format!("set {}", env));
        }
        return Err(format!(
            "No API key for {}: {}.",
            model.provider_name,
            ways.join(" or ")
        ));
    }
    let (url, headers, body) = request(model, key.as_deref(), history, question);
    let mut command = Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--no-buffer",
        "--fail-with-body",
        "--connect-timeout",
        "10",
        "--max-time",
        "300",
        "--config",
        "-",
    ]);
    Ok(Job {
        command,
        input: Some(curl_config(&url, &headers, &body).into_bytes()),
        stream: Box::new(SseStream::new(model)),
    })
}

/// URL, headers and JSON body for one question.
pub fn request(
    model: &Resolved,
    key: Option<&str>,
    history: &[Message],
    question: &str,
) -> (String, Vec<(String, String)>, String) {
    let mut messages: Vec<Value> = history
        .iter()
        .map(|m| json!({ "role": m.role.as_str(), "content": m.content }))
        .collect();
    messages.push(json!({ "role": "user", "content": question }));

    let mut headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "text/event-stream".to_string()),
    ];
    let header = |name: &str, value: &str| (name.to_string(), value.to_string());

    if model.kind == Kind::Anthropic {
        if let Some(key) = key {
            headers.push(header("x-api-key", key));
        }
        headers.push(header("anthropic-version", ANTHROPIC_VERSION));
        let mut body = json!({
            "model": model.model,
            "max_tokens": MAX_TOKENS,
            "messages": messages,
            "stream": true,
        });
        if !model.system_prompt.is_empty() {
            body["system"] = json!(model.system_prompt);
        }
        return (
            format!("{}/messages", model.base_url),
            headers,
            body.to_string(),
        );
    }

    if let Some(key) = key {
        headers.push(header("Authorization", &format!("Bearer {}", key)));
    }
    if model.provider_id == "openrouter" {
        headers.push(header("X-Title", "Poppi Launcher"));
        headers.push(header("HTTP-Referer", "https://github.com/soggy8/poppi"));
    }
    if !model.system_prompt.is_empty() {
        messages.insert(
            0,
            json!({ "role": "system", "content": model.system_prompt }),
        );
    }
    let mut body = json!({
        "model": model.model,
        "messages": messages,
        "stream": true,
    });
    // gpt-oss thinks before answering; a launcher wants the answer quickly.
    if model.model.starts_with("openai/gpt-oss") {
        body["reasoning_effort"] = json!("low");
    }
    (
        format!("{}/chat/completions", model.base_url),
        headers,
        body.to_string(),
    )
}

/// A double-quoted value in curl's config syntax.
fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

pub fn curl_config(url: &str, headers: &[(String, String)], body: &str) -> String {
    let mut config = format!("url = {}\nrequest = \"POST\"\n", quote(url));
    for (name, value) in headers {
        // A line break would start a second header.
        let value: String = value.chars().filter(|c| !c.is_control()).collect();
        config.push_str(&format!(
            "header = {}\n",
            quote(&format!("{}: {}", name, value))
        ));
    }
    config.push_str(&format!("data-binary = {}\n", quote(body)));
    config
}

/// GET a JSON document, sending `key` as a bearer token through curl's
/// stdin like every other request. Blocking.
pub fn get_json(url: &str, key: Option<&str>) -> Result<Value, String> {
    let mut config = format!("url = {}\n", quote(url));
    if let Some(key) = key {
        let key: String = key.chars().filter(|c| !c.is_control()).collect();
        config.push_str(&format!(
            "header = {}\n",
            quote(&format!("Authorization: Bearer {}", key))
        ));
    }
    let mut child = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail-with-body",
            "--connect-timeout",
            "10",
            "--max-time",
            "20",
            "--config",
            "-",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run curl: {}", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(config.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("curl failed: {}", e))?;
    let body = String::from_utf8_lossy(&output.stdout);
    let value: Option<Value> = serde_json::from_str(&body).ok();
    if output.status.success() {
        return value.ok_or_else(|| "Unexpected answer from the server".to_string());
    }
    Err(value
        .as_ref()
        .and_then(error_message)
        .or_else(|| {
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim().trim_start_matches("curl: ").to_string())
        })
        .unwrap_or_else(|| "The server refused the request".to_string()))
}

/// The message in an error body, in any of the shapes APIs use.
pub fn error_message(value: &Value) -> Option<String> {
    let value = value.as_array().and_then(|a| a.first()).unwrap_or(value);
    let error = value.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())?;
    Some(message.trim().to_string()).filter(|m| !m.is_empty())
}

pub struct SseStream {
    anthropic: bool,
    provider: String,
    base_url: String,
    local: bool,
    text: String,
    /// Lines that are not events: an error page or JSON error body.
    body: String,
    failed: bool,
}

impl SseStream {
    pub fn new(model: &Resolved) -> Self {
        Self {
            anthropic: model.kind == Kind::Anthropic,
            provider: model.provider_name.clone(),
            base_url: model.base_url.clone(),
            local: model.base_url.contains("://localhost")
                || model.base_url.contains("://127.0.0.1"),
            text: String::new(),
            body: String::new(),
            failed: false,
        }
    }

    fn delta<'a>(&self, value: &'a Value) -> Option<&'a str> {
        if self.anthropic {
            if value.get("type")?.as_str()? != "content_block_delta" {
                return None;
            }
            let delta = value.get("delta")?;
            if delta.get("type")?.as_str()? != "text_delta" {
                return None;
            }
            delta.get("text")?.as_str()
        } else {
            value
                .get("choices")?
                .get(0)?
                .get("delta")?
                .get("content")?
                .as_str()
        }
    }

    fn connection_error(&self, outcome: &Outcome) -> String {
        match outcome.code {
            Some(6) => format!("Could not reach {}: are you offline?", self.provider),
            Some(7) if self.provider == "Ollama" => {
                "Ollama is not running. Start it with `systemctl start ollama` or `ollama serve`."
                    .to_string()
            }
            Some(7) if self.local => format!(
                "Nothing is listening at {}. Is the server running?",
                self.base_url
            ),
            Some(7) => format!("Could not connect to {}.", self.base_url),
            Some(28) => format!("{} took too long to answer.", self.provider),
            _ => outcome
                .error_line()
                .map(|line| line.trim_start_matches("curl: ").to_string())
                .unwrap_or_else(|| format!("The request to {} failed.", self.provider)),
        }
    }
}

impl Stream for SseStream {
    fn line(&mut self, line: &str) -> Option<Event> {
        let line = line.trim_end_matches('\r');
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data == "[DONE]" || self.failed {
                return None;
            }
            let value: Value = serde_json::from_str(data).ok()?;
            if let Some(message) = error_message(&value) {
                self.failed = true;
                return Some(Event::Failed(format!("{}: {}", self.provider, message)));
            }
            let delta = self.delta(&value).filter(|d| !d.is_empty())?.to_string();
            self.text.push_str(&delta);
            return Some(Event::Delta(delta));
        }
        if !(line.is_empty() || line.starts_with("event:") || line.starts_with(':')) {
            self.body.push_str(line);
            self.body.push('\n');
        }
        None
    }

    fn finish(self: Box<Self>, outcome: Outcome) -> Option<Event> {
        if self.failed {
            return None;
        }
        if outcome.success && !self.text.trim().is_empty() {
            return Some(Event::Done(self.text.trim().to_string()));
        }
        let body = self.body.trim();
        if !body.is_empty() {
            let message = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|value| error_message(&value))
                .unwrap_or_else(|| body.chars().take(300).collect());
            return Some(Event::Failed(format!("{}: {}", self.provider, message)));
        }
        if outcome.success {
            return Some(Event::Failed(format!(
                "{} sent an empty answer.",
                self.provider
            )));
        }
        Some(Event::Failed(self.connection_error(&outcome)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::providers::resolve_entry;
    use super::*;
    use crate::config::ModelConfig;

    fn model(provider: &str) -> Resolved {
        resolve_entry("m", &ModelConfig::new(provider, ""), "Be brief.").unwrap()
    }

    fn feed(stream: &mut SseStream, lines: &[&str]) -> Vec<Event> {
        lines.iter().filter_map(|line| stream.line(line)).collect()
    }

    #[test]
    fn openai_stream() {
        let mut stream = SseStream::new(&model("openai"));
        let events = feed(
            &mut stream,
            &[
                r#"data: {"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
                "",
                r#"data: {"choices":[{"delta":{"content":"Ljub"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"ljana."}}]}"#,
                ": keep-alive",
                r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
                "data: [DONE]",
            ],
        );
        assert_eq!(
            events,
            [Event::Delta("Ljub".into()), Event::Delta("ljana.".into())]
        );
        let done = Box::new(stream).finish(Outcome {
            success: true,
            ..Outcome::default()
        });
        assert_eq!(done, Some(Event::Done("Ljubljana.".into())));
    }

    #[test]
    fn anthropic_stream() {
        let mut stream = SseStream::new(&model("anthropic"));
        let events = feed(
            &mut stream,
            &[
                "event: message_start",
                r#"data: {"type":"message_start","message":{"id":"x"}}"#,
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hm"}}"#,
                r#"data: {"type":"message_stop"}"#,
            ],
        );
        assert_eq!(events, [Event::Delta("Hi".into())]);
    }

    #[test]
    fn errors_in_the_stream_and_in_the_body() {
        let mut stream = SseStream::new(&model("anthropic"));
        let events = feed(
            &mut stream,
            &[
                r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            ],
        );
        assert_eq!(events, [Event::Failed("Anthropic API: Overloaded".into())]);
        assert_eq!(Box::new(stream).finish(Outcome::default()), None);

        let mut stream = SseStream::new(&model("openai"));
        feed(
            &mut stream,
            &[
                "{",
                r#"  "error": { "message": "Incorrect API key provided" }"#,
                "}",
            ],
        );
        let failed = Box::new(stream).finish(Outcome {
            success: false,
            code: Some(22),
            stderr: String::new(),
        });
        assert_eq!(
            failed,
            Some(Event::Failed("OpenAI: Incorrect API key provided".into()))
        );

        let gemini_style =
            serde_json::json!([{ "error": { "code": 400, "message": "API key not valid" } }]);
        assert_eq!(
            error_message(&gemini_style).as_deref(),
            Some("API key not valid")
        );
    }

    #[test]
    fn connection_failures_are_explained() {
        let refused = Outcome {
            success: false,
            code: Some(7),
            stderr: "curl: (7) Failed to connect to localhost port 11434".into(),
        };
        let ollama = Box::new(SseStream::new(&model("ollama"))).finish(refused);
        assert!(matches!(ollama, Some(Event::Failed(m)) if m.contains("ollama serve")));

        let other = Outcome {
            success: false,
            code: Some(35),
            stderr: "curl: (35) TLS failure\n".into(),
        };
        let failed = Box::new(SseStream::new(&model("openai"))).finish(other);
        assert_eq!(failed, Some(Event::Failed("(35) TLS failure".into())));
    }

    #[test]
    fn request_shapes() {
        let history = [Message::user("a"), Message::assistant("b")];
        let (url, headers, body) = request(&model("openai"), Some("sk-1"), &history, "c");
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
        assert!(headers.contains(&("Authorization".into(), "Bearer sk-1".into())));
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][3]["content"], "c");

        let (url, headers, body) = request(&model("anthropic"), Some("sk-2"), &history, "c");
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert!(headers.contains(&("x-api-key".into(), "sk-2".into())));
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["system"], "Be brief.");
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);

        let (_, headers, _) = request(&model("ollama"), None, &[], "c");
        assert!(!headers.iter().any(|(name, _)| name == "Authorization"));
    }

    #[test]
    fn curl_config_escapes_values() {
        let config = curl_config(
            "https://x/y",
            &[("Authorization".into(), "Bearer k\n-X DELETE".into())],
            r#"{"q":"say \"hi\"\n"}"#,
        );
        assert_eq!(
            config,
            "url = \"https://x/y\"\nrequest = \"POST\"\nheader = \"Authorization: Bearer k-X DELETE\"\ndata-binary = \"{\\\"q\\\":\\\"say \\\\\\\"hi\\\\\\\"\\\\n\\\"}\"\n"
        );
    }

    #[test]
    fn missing_keys_are_reported_before_anything_runs() {
        let mut entry = ModelConfig::new("openai", "");
        entry.api_key_env = "POPPI_TEST_UNSET_VARIABLE".into();
        let mut model = resolve_entry("m", &entry, "").unwrap();
        model.key.keyring_id = None;
        let error = job(&model, &[], "hi").err().unwrap();
        assert!(error.contains("POPPI_TEST_UNSET_VARIABLE"), "{}", error);
    }
}
