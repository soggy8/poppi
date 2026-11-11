use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Look `cmd` up in the given directories without involving a shell.
pub fn find_in_dirs<I, P>(cmd: &str, dirs: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    if cmd.is_empty() {
        return None;
    }
    if cmd.contains('/') {
        let path = PathBuf::from(cmd);
        return is_executable(&path).then_some(path);
    }
    dirs.into_iter()
        .map(|dir| dir.as_ref().join(cmd))
        .find(|candidate| is_executable(candidate))
}

/// Resolve an executable through `$PATH`.
pub fn find_in_path(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_in_dirs(cmd, std::env::split_paths(&path))
}

pub fn has_command(cmd: &str) -> bool {
    find_in_path(cmd).is_some()
}

pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|s| s == "wayland")
            .unwrap_or(false)
}

/// Upper-case the first character, leaving the rest untouched (UTF-8 safe).
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Quote a string for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Spawn a process we never talk to again, reaping it in the background so a
/// long-running launcher does not collect zombies.
pub fn spawn_detached(command: &mut Command) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start {}: {}", program, e))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn pipe_to(program: &str, args: &[&str], text: &str) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start {}: {}", program, e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("Could not write to {}: {}", program, e))?;
    }
    let status = child
        .wait()
        .map_err(|e| format!("{} failed: {}", program, e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {}", program, status))
    }
}

/// Copy text using whichever clipboard tool is installed. These tools keep
/// serving the selection after the launcher window is gone.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let tools: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    let wayland = is_wayland();
    let mut last_error = None;
    for (program, args) in tools {
        if program == "wl-copy" && !wayland {
            continue;
        }
        if !has_command(program) {
            continue;
        }
        match pipe_to(program, args, text) {
            Ok(()) => return Ok(()),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error
        .unwrap_or_else(|| "No clipboard tool available (wl-copy, xclip or xsel)".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capitalize_handles_ascii_and_unicode() {
        assert_eq!(capitalize("kitty"), "Kitty");
        assert_eq!(capitalize("über"), "Über");
        assert_eq!(capitalize("x"), "X");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("ls -la"), "'ls -la'");
        assert_eq!(shell_quote("echo 'hi'"), "'echo '\\''hi'\\'''");
    }

    #[test]
    fn find_in_dirs_never_runs_a_shell() {
        let dirs = ["/usr/bin", "/bin"];
        assert!(find_in_dirs("sh", dirs).is_some());
        assert!(find_in_dirs("", dirs).is_none());
        // Shell metacharacters are just part of a file name that does not exist.
        assert!(find_in_dirs("sh;poweroff", dirs).is_none());
        assert!(find_in_dirs("$(poweroff)", dirs).is_none());
        assert!(find_in_dirs("definitely-not-a-command-xyz", dirs).is_none());
    }
}
