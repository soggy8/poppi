//! Things the AI can propose doing on the computer. The model writes them
//! as `[[name: argument]]` lines in its answer; they are taken out of the
//! text shown and offered as rows to run with Enter. Nothing runs on its
//! own. Plain text markers work with every provider, including CLIs and
//! small local models, unlike each API's own tool-calling format.

use crate::system::SystemAction;

/// Most actions one answer may propose.
const MAX_ACTIONS: usize = 5;

/// Added to the system prompt of every model.
pub const PROMPT: &str = "\
You are Poppi, a launcher on the user's Linux desktop. Besides answering, you can do things \
on the computer: put action lines at the end of your answer, each on its own line, written \
exactly like these:
[[open_app: Firefox]]  start an application by name
[[open: ~/Downloads]]  open a file, a folder or an https:// address
[[find_files: tax return 2025]]  search the user's files by name
[[switch_window: Slack]]  bring an open window to the front
[[web_search: weather in Ljubljana]]  search the web
[[copy: some text]]  copy text to the clipboard
[[run: df -h]]  run a shell command in a terminal
[[system: lock]]  lock, suspend, logout, restart or poweroff
[[remind: in 10 minutes check the oven]]  a reminder (in …, or at 15:30)
[[note: buy milk]]  add a line to the user's notes
The user sees each action as a button and decides whether to run it. Only add actions for \
what the user asked you to do; for questions, just answer. When you add actions, keep the \
text to one short sentence.";

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    OpenApp(String),
    Open(String),
    FindFiles(String),
    SwitchWindow(String),
    WebSearch(String),
    Copy(String),
    Run(String),
    System(SystemAction),
    Remind(String),
    Note(String),
}

impl Action {
    fn parse(name: &str, arg: &str) -> Option<Self> {
        let arg = arg.trim();
        if arg.is_empty() {
            return None;
        }
        let arg_owned = arg.to_string();
        Some(
            match name.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
                "open_app" | "launch" | "app" => Action::OpenApp(arg_owned),
                "open" | "open_file" | "open_folder" | "open_url" => Action::Open(arg_owned),
                "find_files" | "find" | "search_files" => Action::FindFiles(arg_owned),
                "switch_window" | "focus" | "window" => Action::SwitchWindow(arg_owned),
                "web_search" | "search" => Action::WebSearch(arg_owned),
                "copy" => Action::Copy(arg_owned),
                "run" | "shell" | "command" => Action::Run(arg_owned),
                "system" => Action::System(system_action(arg)?),
                "remind" | "reminder" => Action::Remind(arg_owned),
                "note" => Action::Note(arg_owned),
                _ => return None,
            },
        )
    }
}

fn system_action(name: &str) -> Option<SystemAction> {
    let name = name.to_lowercase().replace([' ', '-', '_'], "");
    Some(match name.as_str() {
        "lock" | "lockscreen" => SystemAction::Lock,
        "suspend" | "sleep" => SystemAction::Suspend,
        "logout" | "logoff" | "signout" => SystemAction::LogOut,
        "restart" | "reboot" => SystemAction::Restart,
        "poweroff" | "shutdown" | "turnoff" => SystemAction::PowerOff,
        _ => return None,
    })
}

/// What a marker's text up to its `:` may contain: an action name.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == ' ' || c == '-'
}

/// Takes the markers out of one line (outside code blocks). Returns false
/// when the line ends inside a marker still being streamed.
fn split_line(line: &str, shown: &mut String, actions: &mut Vec<Action>) {
    let mut rest = line;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let name_len = after
            .find(|c: char| !is_name_char(c))
            .unwrap_or(after.len());
        let name = &after[..name_len];
        let looks_like_action = !name.trim().is_empty() && !name.starts_with(' ');
        if !looks_like_action {
            // e.g. bash's `[[ -f file ]]`: plain text.
            shown.push_str(&rest[..start + 2]);
            rest = after;
            continue;
        }
        if name_len == after.len() || after[name_len..].starts_with(':') && !after.contains("]]") {
            // `[[open_app: Fir` while streaming: hide until it is complete.
            shown.push_str(&rest[..start]);
            return;
        }
        let tail = &after[name_len..];
        let parsed = tail
            .strip_prefix(':')
            .and_then(|arg| arg.find("]]").map(|end| (&arg[..end], end)));
        match parsed {
            Some((arg, end))
                if Action::parse(name, "x").is_some()
                    || name.trim().eq_ignore_ascii_case("system") =>
            {
                shown.push_str(&rest[..start]);
                if let Some(action) = Action::parse(name, arg) {
                    if !actions.contains(&action) && actions.len() < MAX_ACTIONS {
                        actions.push(action);
                    }
                }
                rest = &tail[1 + end + 2..];
            }
            _ => {
                shown.push_str(&rest[..start + 2]);
                rest = after;
            }
        }
    }
    shown.push_str(rest);
}

/// The answer as shown, without action markers, and the actions in it.
/// Code blocks are left alone; a marker still being streamed
/// (`[[open_app: Fir`) is hidden until it is complete.
pub fn split(text: &str) -> (String, Vec<Action>) {
    let mut lines = Vec::new();
    let mut actions = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            lines.push(line.to_string());
            continue;
        }
        if in_code {
            lines.push(line.to_string());
            continue;
        }
        let mut shown = String::new();
        split_line(line, &mut shown, &mut actions);
        let had_marker = shown.len() != line.len();
        // A line that held only markers disappears entirely.
        if !(had_marker && shown.trim().is_empty()) {
            lines.push(shown.trim_end().to_string());
        }
    }
    (lines.join("\n").trim().to_string(), actions)
}

/// The system prompt with the actions explained after the user's own.
pub fn system_prompt(user_prompt: &str) -> String {
    if user_prompt.trim().is_empty() {
        PROMPT.to_string()
    } else {
        format!("{}\n\n{}", user_prompt.trim(), PROMPT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_taken_out_of_the_answer() {
        let (shown, actions) =
            split("Opening your downloads.\n\n[[open: ~/Downloads]]\n[[open_app: Files]]\n");
        assert_eq!(shown, "Opening your downloads.");
        assert_eq!(
            actions,
            vec![
                Action::Open("~/Downloads".into()),
                Action::OpenApp("Files".into())
            ]
        );
    }

    #[test]
    fn small_model_variations_are_understood() {
        let (_, actions) =
            split("[[Open App: Spotify]] [[system: Lock Screen]] [[web search: rust]]");
        assert_eq!(
            actions,
            vec![
                Action::OpenApp("Spotify".into()),
                Action::System(SystemAction::Lock),
                Action::WebSearch("rust".into()),
            ]
        );
        let (_, actions) = split("[[system: shut down]][[run: df -h]][[run: df -h]]");
        assert_eq!(
            actions,
            vec![
                Action::System(SystemAction::PowerOff),
                Action::Run("df -h".into())
            ]
        );
    }

    #[test]
    fn only_known_actions_are_markers() {
        let (shown, actions) = split("Sure. [[open: ]] [[dance: now]] [[wiki link]]");
        assert_eq!(shown, "Sure.  [[dance: now]] [[wiki link]]");
        assert!(actions.is_empty());
        let (shown, actions) = split("[[system: explode]]");
        assert_eq!(shown, "");
        assert!(actions.is_empty());
    }

    #[test]
    fn code_and_shell_tests_are_left_alone() {
        let answer = "Check first:\n\n```sh\n[[open: x]] && [[ -f a ]]\n```\nOr run `[[ -d b ]]`.";
        let (shown, actions) = split(answer);
        assert_eq!(shown, answer);
        assert!(actions.is_empty());
    }

    #[test]
    fn a_marker_still_streaming_is_hidden() {
        assert_eq!(split("Opening it.\n[[open_app: Fir").0, "Opening it.");
        assert_eq!(
            split("Plain answer without actions").0,
            "Plain answer without actions"
        );
    }

    #[test]
    fn no_more_than_five() {
        let text: String = (0..9).map(|i| format!("[[copy: {}]]\n", i)).collect();
        assert_eq!(split(&text).1.len(), MAX_ACTIONS);
    }

    #[test]
    fn prompt_follows_the_users_own() {
        assert!(system_prompt("").starts_with("You are Poppi"));
        let combined = system_prompt("Be brief.");
        assert!(combined.starts_with("Be brief.\n\nYou are Poppi"));
    }
}

#[cfg(test)]
mod print_prompt {
    /// `cargo test print_full_prompt -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_full_prompt() {
        print!(
            "<<<{}>>>",
            super::system_prompt(crate::config::DEFAULT_SYSTEM_PROMPT)
        );
    }
}
