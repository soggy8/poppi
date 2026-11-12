use crate::utils;
use std::process::Command;

pub struct Terminal;

impl Terminal {
    /// True when the first word of the query is an executable on `$PATH`.
    /// This is a plain file lookup; nothing is executed.
    pub fn looks_like_command(query: &str) -> bool {
        query
            .split_whitespace()
            .next()
            .map(|word| !word.contains('/') && utils::has_command(word))
            .unwrap_or(false)
    }

    pub fn execute_command(command: &str) -> Result<(), String> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        // Keep the terminal open so the output can be read.
        let full_command = format!("{}; echo ''; echo 'Press Enter to close...'; read", command);
        let terminal = Self::default_terminal()?;
        let argv = Self::terminal_args(&terminal, &shell, &full_command);
        utils::spawn_detached(Command::new(&terminal).args(argv))
    }

    /// Open an interactive terminal in `dir`.
    pub fn open_in_dir(dir: &std::path::Path) -> Result<(), String> {
        let terminal = Self::default_terminal()?;
        let name = terminal.rsplit('/').next().unwrap_or(&terminal);
        let mut cmd = Command::new(&terminal);
        cmd.current_dir(dir);
        if name == "wezterm" {
            cmd.args(["start", "--cwd"]).arg(dir);
        } else if matches!(name, "gnome-terminal" | "gnome-terminal-server" | "ptyxis") {
            cmd.arg(format!("--working-directory={}", dir.display()));
        }
        utils::spawn_detached(&mut cmd)
    }

    fn default_terminal() -> Result<String, String> {
        // 1. $TERMINAL (user preference)
        if let Ok(term) = std::env::var("TERMINAL") {
            if utils::has_command(&term) {
                return Ok(term);
            }
        }

        // 2. x-terminal-emulator (Debian/Ubuntu alternatives system); resolve
        //    the symlink so we know which argument style to use.
        if let Some(path) = utils::find_in_path("x-terminal-emulator") {
            let resolved = std::fs::canonicalize(&path).unwrap_or(path);
            return Ok(resolved.to_string_lossy().into_owned());
        }

        // 3. GNOME's configured terminal
        if let Ok(output) = Command::new("gsettings")
            .args([
                "get",
                "org.gnome.desktop.default-applications.terminal",
                "exec",
            ])
            .output()
        {
            if output.status.success() {
                let term = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .trim_matches('\'')
                    .to_string();
                if utils::has_command(&term) {
                    return Ok(term);
                }
            }
        }

        // 4. Common terminals
        let terminals = [
            "ptyxis",
            "kgx",
            "gnome-terminal",
            "kitty",
            "alacritty",
            "foot",
            "wezterm",
            "konsole",
            "tilix",
            "xterm",
        ];
        terminals
            .into_iter()
            .find(|term| utils::has_command(term))
            .map(str::to_string)
            .ok_or_else(|| "No terminal emulator found".to_string())
    }

    /// Arguments that make `terminal` run `shell -c command`.
    fn terminal_args(terminal: &str, shell: &str, command: &str) -> Vec<String> {
        let name = terminal.rsplit('/').next().unwrap_or(terminal);
        let run = [shell.to_string(), "-c".to_string(), command.to_string()];
        let with = |prefix: &[&str]| -> Vec<String> {
            prefix
                .iter()
                .map(|s| s.to_string())
                .chain(run.iter().cloned())
                .collect()
        };

        match name {
            "gnome-terminal" | "gnome-terminal-server" | "ptyxis" => with(&["--"]),
            "kitty" | "foot" => with(&[]),
            "wezterm" => with(&["start", "--"]),
            "alacritty" | "konsole" | "xterm" | "urxvt" | "rxvt" | "terminator"
            | "mate-terminal" | "xfce4-terminal" | "lxterminal" | "kgx" | "console" => {
                with(&["-e"])
            }
            // tilix, terminology and unknown terminals take one command string.
            _ => vec![
                "-e".to_string(),
                format!("{} -c {}", shell, utils::shell_quote(command)),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_styles() {
        assert_eq!(
            Terminal::terminal_args("/usr/bin/ptyxis", "/bin/zsh", "ls"),
            vec!["--", "/bin/zsh", "-c", "ls"]
        );
        assert_eq!(
            Terminal::terminal_args("kitty", "/bin/zsh", "ls"),
            vec!["/bin/zsh", "-c", "ls"]
        );
        assert_eq!(
            Terminal::terminal_args("alacritty", "/bin/bash", "ls -la"),
            vec!["-e", "/bin/bash", "-c", "ls -la"]
        );
        assert_eq!(
            Terminal::terminal_args("tilix", "/bin/bash", "echo 'hi'"),
            vec!["-e", "/bin/bash -c 'echo '\\''hi'\\'''"]
        );
    }

    #[test]
    fn command_detection_is_a_path_lookup() {
        assert!(Terminal::looks_like_command("sh -c true"));
        assert!(!Terminal::looks_like_command(""));
        assert!(!Terminal::looks_like_command("sh;poweroff"));
        assert!(!Terminal::looks_like_command(
            "definitely-not-a-command-xyz"
        ));
    }
}
