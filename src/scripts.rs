//! User scripts in `~/.config/poppi_launcher/scripts/`.
//!
//! Any executable file in that directory becomes a command in Poppi. Optional
//! comment headers in the first 30 lines customize how it behaves:
//!
//! ```sh
//! #!/usr/bin/env bash
//! # @poppi.keyword ssh
//! # @poppi.name SSH Hosts
//! # @poppi.description Connect to a host from ~/.ssh/config
//! # @poppi.icon network-server-symbolic
//! # @poppi.mode list
//! # @poppi.terminal false
//! ```
//!
//! - `action` (default): runs the script with typed words as arguments, and is
//!   also searchable by name in normal search.
//! - `list`: runs `<script> <filter>` as you type `<keyword> [filter]` and
//!   renders stdout lines (JSON objects or `Title\tSubtitle\tCommand` TSV) as
//!   interactive result rows.

use crate::config::Config;
use crate::matching::Matcher;
use crate::search;
use crate::terminal::Terminal;
use crate::utils;
use gtk::gio;
use gtk::prelude::*;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const HEADER_LINES: usize = 30;
const LIST_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_SCRIPT_ITEMS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptMode {
    Action,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub path: PathBuf,
    pub keyword: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: String,
    pub mode: ScriptMode,
    pub terminal: bool,
    name_lower: String,
    description_lower: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptItemAction {
    /// Invoke `script_path` with these arguments.
    ScriptArgs {
        script: PathBuf,
        args: Vec<String>,
        terminal: bool,
    },
    /// Run a shell command.
    Shell { command: String, terminal: bool },
    /// Copy text to the clipboard.
    Copy(String),
    /// Open a URL in the browser.
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptItem {
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: String,
    pub action: ScriptItemAction,
}

#[derive(Deserialize)]
struct JsonScriptRow {
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    run: Option<String>,
    #[serde(default)]
    copy: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    terminal: Option<bool>,
}

#[derive(Debug, Default, Clone)]
pub struct ScriptCatalog {
    scripts: Vec<Script>,
}

pub fn scripts_dir() -> PathBuf {
    Config::config_path()
        .parent()
        .map(|p| p.join("scripts"))
        .unwrap_or_else(|| PathBuf::from("scripts"))
}

fn valid_keyword(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

fn strip_comment(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("#!") {
        return None;
    }
    trimmed
        .strip_prefix('#')
        .or_else(|| trimmed.strip_prefix("//"))
        .or_else(|| trimmed.strip_prefix("--"))
        .map(str::trim)
}

fn parse_script_metadata(path: &Path, header: &str) -> Option<Script> {
    let file_name = path.file_name()?.to_string_lossy();
    if file_name.starts_with('.') || file_name.ends_with('~') || file_name.ends_with(".bak") {
        return None;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut keyword = stem.to_lowercase();
    let mut name = utils::capitalize(&stem.replace(['_', '-'], " "));
    let mut description: Option<String> = None;
    let mut icon = "system-run-symbolic".to_string();
    let mut mode = ScriptMode::Action;
    let mut terminal = false;

    for line in header.lines().take(HEADER_LINES) {
        let Some(comment) = strip_comment(line) else {
            continue;
        };
        let Some(rest) = comment.strip_prefix("@poppi.") else {
            continue;
        };
        let Some((key, value)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key {
            "keyword" if valid_keyword(value) => keyword = value.to_lowercase(),
            "name" => name = value.to_string(),
            "description" => description = Some(value.to_string()),
            "icon" => icon = value.to_string(),
            "mode" => {
                mode = match value.to_lowercase().as_str() {
                    "list" | "filter" | "json" => ScriptMode::List,
                    _ => ScriptMode::Action,
                };
            }
            "terminal" => terminal = matches!(value.to_lowercase().as_str(), "true" | "1" | "yes"),
            _ => {}
        }
    }

    if !valid_keyword(&keyword) || name.is_empty() {
        return None;
    }

    Some(Script {
        path: path.to_path_buf(),
        keyword,
        name_lower: name.to_lowercase(),
        description_lower: description.as_ref().map(|d| d.to_lowercase()),
        name,
        description,
        icon,
        mode,
        terminal,
    })
}

impl ScriptCatalog {
    pub fn load() -> Self {
        Self::load_from(&scripts_dir())
    }

    pub fn load_from(dir: &Path) -> Self {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return Self::default();
        };
        let mut scripts: Vec<Script> = read_dir
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if !utils::is_executable(&path) {
                    return None;
                }
                let mut file = std::fs::File::open(&path).ok()?;
                let mut buf = vec![0u8; 4096];
                let n = file.read(&mut buf).ok()?;
                let header = String::from_utf8_lossy(&buf[..n]);
                parse_script_metadata(&path, &header)
            })
            .collect();
        scripts.sort_by(|a, b| a.name_lower.cmp(&b.name_lower));
        Self { scripts }
    }

    pub fn keywords(&self) -> BTreeSet<String> {
        self.scripts.iter().map(|s| s.keyword.clone()).collect()
    }

    pub fn by_keyword(&self, keyword: &str) -> Option<&Script> {
        let lower = keyword.to_lowercase();
        self.scripts.iter().find(|s| s.keyword == lower)
    }

    /// Action scripts whose name, keyword or description match `query`.
    pub fn search<'a>(&'a self, matcher: &Matcher, query: &str) -> Vec<&'a Script> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(&Script, i64)> = self
            .scripts
            .iter()
            .filter_map(|script| {
                let name = matcher.score(&query, &script.name_lower);
                let kw = matcher.score(&query, &script.keyword).map(|s| s - 80);
                let desc = script
                    .description_lower
                    .as_deref()
                    .filter(|_| query.chars().count() >= 3)
                    .and_then(|d| Matcher::strict(&query, d))
                    .map(|s| s - 350);
                [name, kw, desc]
                    .into_iter()
                    .flatten()
                    .max()
                    .filter(|&s| s > 0)
                    .map(|s| (script, s))
            })
            .collect();
        scored.sort_by_key(|item| std::cmp::Reverse(item.1));
        scored.into_iter().map(|(s, _)| s).collect()
    }
}

pub fn watch_scripts_dir(on_change: impl Fn() + 'static) -> Option<gio::FileMonitor> {
    let dir = scripts_dir();
    let _ = std::fs::create_dir_all(&dir);
    let monitor = gio::File::for_path(dir)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .ok()?;
    monitor.connect_changed(move |_, _, _, event| {
        use gio::FileMonitorEvent::{
            ChangesDoneHint, Created, Deleted, MovedIn, MovedOut, Renamed,
        };
        if matches!(
            event,
            ChangesDoneHint | Created | Deleted | Renamed | MovedIn | MovedOut
        ) {
            on_change();
        }
    });
    Some(monitor)
}

fn parse_output_line(script: &Script, line: &str) -> Option<ScriptItem> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('{') {
        if let Ok(row) = serde_json::from_str::<JsonScriptRow>(trimmed) {
            if row.title.trim().is_empty() {
                return None;
            }
            let term = row.terminal.unwrap_or(script.terminal);
            let action = if let Some(copy) = row.copy {
                ScriptItemAction::Copy(copy)
            } else if let Some(url) = row.url {
                ScriptItemAction::Url(url)
            } else if let Some(run) = row.run {
                ScriptItemAction::Shell {
                    command: run,
                    terminal: term,
                }
            } else {
                ScriptItemAction::ScriptArgs {
                    script: script.path.clone(),
                    args: row.args.unwrap_or_else(|| vec![row.title.clone()]),
                    terminal: term,
                }
            };
            return Some(ScriptItem {
                title: row.title,
                subtitle: row.subtitle.filter(|s| !s.trim().is_empty()),
                icon: row
                    .icon
                    .filter(|i| !i.trim().is_empty())
                    .unwrap_or_else(|| script.icon.clone()),
                action,
            });
        }
    }

    let cols: Vec<&str> = line.splitn(3, '\t').map(str::trim).collect();
    let title = cols.first().copied().unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    let subtitle = cols
        .get(1)
        .copied()
        .filter(|s| !s.is_empty())
        .map(String::from);
    let action = match cols.get(2).copied().filter(|s| !s.is_empty()) {
        Some(target) if target.starts_with("http://") || target.starts_with("https://") => {
            ScriptItemAction::Url(target.to_string())
        }
        Some(command) => ScriptItemAction::Shell {
            command: command.to_string(),
            terminal: script.terminal,
        },
        None => ScriptItemAction::ScriptArgs {
            script: script.path.clone(),
            args: vec![title.to_string()],
            terminal: script.terminal,
        },
    };
    Some(ScriptItem {
        title: title.to_string(),
        subtitle,
        icon: script.icon.clone(),
        action,
    })
}

/// Run a `list`-mode script on a worker thread with a strict timeout.
pub fn query_list_script(script: &Script, filter: &str) -> Result<Vec<ScriptItem>, String> {
    let mut cmd = Command::new(&script.path);
    if !filter.trim().is_empty() {
        cmd.arg(filter.trim());
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run {}: {}", script.name, e))?;

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < LIST_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(15));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} timed out", script.name));
            }
        }
    };

    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut stderr);
        }
        let detail = stderr
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .unwrap_or("script failed");
        return Err(format!("{}: {}", script.name, detail.trim()));
    }

    Ok(stdout
        .lines()
        .filter_map(|line| parse_output_line(script, line))
        .take(MAX_SCRIPT_ITEMS)
        .collect())
}

pub fn run_script(script: &Script, args_str: &str) -> Result<(), String> {
    let args: Vec<String> = args_str.split_whitespace().map(String::from).collect();
    run_script_argv(&script.path, &args, script.terminal)
}

fn run_script_argv(path: &Path, args: &[String], in_terminal: bool) -> Result<(), String> {
    if in_terminal {
        let mut quoted = vec![utils::shell_quote(&path.to_string_lossy())];
        quoted.extend(args.iter().map(|a| utils::shell_quote(a)));
        return Terminal::execute_command(&quoted.join(" "));
    }
    utils::spawn_detached(Command::new(path).args(args))
}

pub fn run_item_action(
    action: &ScriptItemAction,
    context: Option<&gio::AppLaunchContext>,
) -> Result<Option<String>, String> {
    match action {
        ScriptItemAction::Copy(text) => Ok(Some(text.clone())),
        ScriptItemAction::Url(url) => search::open_url(url, context).map(|()| None),
        ScriptItemAction::Shell {
            command,
            terminal: true,
        } => Terminal::execute_command(command).map(|()| None),
        ScriptItemAction::Shell {
            command,
            terminal: false,
        } => utils::spawn_detached(Command::new("sh").arg("-c").arg(command)).map(|()| None),
        ScriptItemAction::ScriptArgs {
            script,
            args,
            terminal,
        } => run_script_argv(script, args, *terminal).map(|()| None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn metadata_comments_are_parsed_with_sensible_defaults() {
        let header = "\
#!/usr/bin/env bash
# @poppi.keyword tail
# @poppi.name Tailscale Nodes
# @poppi.description Switch exit node
# @poppi.icon network-vpn-symbolic
# @poppi.mode list
# @poppi.terminal true
";
        let script = parse_script_metadata(Path::new("/tmp/my_script.sh"), header).unwrap();
        assert_eq!(script.keyword, "tail");
        assert_eq!(script.name, "Tailscale Nodes");
        assert_eq!(script.description.as_deref(), Some("Switch exit node"));
        assert_eq!(script.icon, "network-vpn-symbolic");
        assert_eq!(script.mode, ScriptMode::List);
        assert!(script.terminal);

        // Minimal script without headers derives keyword and name from filename.
        let bare =
            parse_script_metadata(Path::new("/tmp/toggle-vpn.sh"), "#!/bin/sh\necho hi\n").unwrap();
        assert_eq!(bare.keyword, "toggle-vpn");
        assert_eq!(bare.name, "Toggle vpn");
        assert_eq!(bare.mode, ScriptMode::Action);
        assert!(!bare.terminal);
    }

    #[test]
    fn json_and_tsv_output_lines_parse_into_items() {
        let script = parse_script_metadata(
            Path::new("/tmp/hosts.sh"),
            "# @poppi.mode list\n# @poppi.icon network-server-symbolic\n",
        )
        .unwrap();

        let json = parse_output_line(
            &script,
            r#"{"title":"prod-1","subtitle":"10.0.0.5","copy":"10.0.0.5"}"#,
        )
        .unwrap();
        assert_eq!(json.title, "prod-1");
        assert_eq!(json.subtitle.as_deref(), Some("10.0.0.5"));
        assert_eq!(json.action, ScriptItemAction::Copy("10.0.0.5".into()));

        let tsv = parse_output_line(&script, "Docs\tOpen documentation\thttps://docs.rs").unwrap();
        assert_eq!(tsv.title, "Docs");
        assert_eq!(tsv.action, ScriptItemAction::Url("https://docs.rs".into()));

        let simple = parse_output_line(&script, "staging").unwrap();
        assert_eq!(
            simple.action,
            ScriptItemAction::ScriptArgs {
                script: PathBuf::from("/tmp/hosts.sh"),
                args: vec!["staging".into()],
                terminal: false,
            }
        );
    }

    #[test]
    fn loads_and_runs_list_scripts_from_directory() {
        let dir = std::env::temp_dir().join(format!("poppi-scripts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let script_path = dir.join("greet.sh");
        std::fs::write(
            &script_path,
            "#!/bin/sh\n# @poppi.keyword greet\n# @poppi.name Greeter\n# @poppi.mode list\nprintf 'Hello %s\\tGreeting\\n' \"${1:-world}\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        // Non-executable files are ignored.
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();

        let catalog = ScriptCatalog::load_from(&dir);
        assert!(catalog.keywords().contains("greet"));
        let script = catalog.by_keyword("greet").unwrap();
        let items = query_list_script(script, "Poppi").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Hello Poppi");
        assert_eq!(items[0].subtitle.as_deref(), Some("Greeting"));

        let matcher = Matcher::new();
        assert_eq!(catalog.search(&matcher, "greeter").len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
