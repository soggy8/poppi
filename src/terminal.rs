use std::process::Command;

pub struct Terminal;

impl Terminal {
    pub fn execute_command(command: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Get the shell
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        
        // Create a command that waits for user input before closing
        let full_command = format!("{}; echo ''; echo 'Press Enter to close...'; read", command);
        
        // Get the user's default terminal
        let terminal = Self::get_default_terminal()?;
        
        // Execute based on terminal type (different terminals have different args)
        Self::run_in_terminal(&terminal, &shell, &full_command)?;
        
        Ok(())
    }
    
    fn get_default_terminal() -> Result<String, Box<dyn std::error::Error>> {
        // 1. Check $TERMINAL environment variable (user preference)
        if let Ok(term) = std::env::var("TERMINAL") {
            if !term.is_empty() && Self::has_command(&term) {
                return Ok(term);
            }
        }
        
        // 2. Try x-terminal-emulator (Debian/Ubuntu system default)
        if Self::has_command("x-terminal-emulator") {
            // Get the actual terminal it points to
            if let Ok(output) = Command::new("readlink")
                .arg("-f")
                .arg("/usr/bin/x-terminal-emulator")
                .output()
            {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        return Ok(path);
                    }
                }
            }
            return Ok("x-terminal-emulator".to_string());
        }
        
        // 3. Try to get GNOME's preferred terminal via gsettings
        if let Ok(output) = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.default-applications.terminal", "exec"])
            .output()
        {
            if output.status.success() {
                let term = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .trim_matches('\'')
                    .to_string();
                if !term.is_empty() && Self::has_command(&term) {
                    return Ok(term);
                }
            }
        }
        
        // 4. Fallback: check common terminals
        let terminals = ["gnome-terminal", "konsole", "kitty", "alacritty", "tilix", "xterm"];
        for term in terminals {
            if Self::has_command(term) {
                return Ok(term.to_string());
            }
        }
        
        Err("No terminal emulator found".into())
    }
    
    fn run_in_terminal(terminal: &str, shell: &str, command: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Get the terminal name (without path)
        let term_name = terminal.rsplit('/').next().unwrap_or(terminal);
        
        match term_name {
            "gnome-terminal" | "gnome-terminal-server" => {
                Command::new(terminal)
                    .arg("--")
                    .arg(shell)
                    .arg("-c")
                    .arg(command)
                    .spawn()?;
            }
            "kitty" => {
                Command::new(terminal)
                    .arg(shell)
                    .arg("-c")
                    .arg(command)
                    .spawn()?;
            }
            "alacritty" | "konsole" | "xterm" | "urxvt" | "rxvt" | "terminator" | "mate-terminal" | "xfce4-terminal" | "lxterminal" => {
                Command::new(terminal)
                    .arg("-e")
                    .arg(shell)
                    .arg("-c")
                    .arg(command)
                    .spawn()?;
            }
            "tilix" | "terminology" => {
                Command::new(terminal)
                    .arg("-e")
                    .arg(format!("{} -c '{}'", shell, command))
                    .spawn()?;
            }
            "kgx" | "console" => {
                // GNOME Console
                Command::new(terminal)
                    .arg("-e")
                    .arg(shell)
                    .arg("-c")
                    .arg(command)
                    .spawn()?;
            }
            "wezterm" => {
                Command::new(terminal)
                    .arg("start")
                    .arg("--")
                    .arg(shell)
                    .arg("-c")
                    .arg(command)
                    .spawn()?;
            }
            // Default: try -e flag (most common)
            _ => {
                Command::new(terminal)
                    .arg("-e")
                    .arg(format!("{} -c '{}'", shell, command))
                    .spawn()?;
            }
        }
        
        Ok(())
    }

    fn has_command(cmd: &str) -> bool {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn is_terminal_command(query: &str) -> bool {
        // Simple heuristic: if it starts with common command patterns
        let trimmed = query.trim();
        
        // Check for explicit terminal prefix
        if trimmed.starts_with("> ") || trimmed.starts_with("$ ") {
            return true;
        }
        
        // Common terminal commands
        let terminal_commands = [
            "ls", "cd", "pwd", "grep", "find", "cat", "less", "more",
            "head", "tail", "mkdir", "rm", "cp", "mv", "chmod", "chown",
            "sudo", "git", "npm", "cargo", "python", "python3", "node",
            "yarn", "pnpm", "docker", "podman", "kubectl", "curl", "wget",
            "ssh", "scp", "rsync", "htop", "top", "ps", "kill", "ping",
            "traceroute", "ifconfig", "ip", "netstat", "df", "du", "free",
            "uname", "whoami", "date", "cal", "man", "which", "whereis",
            "echo", "printf", "touch", "nano", "vim", "vi", "nvim",
            "apt", "dnf", "yum", "pacman", "flatpak", "snap",
        ];

        // Check if it starts with a known command
        let first_word = trimmed.split_whitespace().next().unwrap_or("");
        terminal_commands.iter().any(|&cmd| first_word == cmd)
    }
}

