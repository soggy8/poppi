use std::process::Command;

pub struct Terminal;

impl Terminal {
    pub fn execute_command(command: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Detect the default terminal emulator
        let terminal = Self::detect_terminal()?;
        
        // Get the shell
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        
        // Execute command in terminal
        Command::new(terminal)
            .arg("-e")
            .arg(&shell)
            .arg("-c")
            .arg(command)
            .spawn()?;
        
        Ok(())
    }

    fn detect_terminal() -> Result<String, Box<dyn std::error::Error>> {
        // Try common terminal emulators in order of preference
        let terminals = vec![
            "gnome-terminal",
            "tilix",
            "alacritty",
            "kitty",
            "konsole",
            "xterm",
            "termite",
        ];

        for term in terminals {
            if Command::new("which").arg(term).output().is_ok() {
                if Command::new("which").arg(term).output()?.status.success() {
                    return Ok(term.to_string());
                }
            }
        }

        Err("No terminal emulator found".into())
    }

    pub fn is_terminal_command(query: &str) -> bool {
        // Simple heuristic: if it starts with common command patterns
        let trimmed = query.trim();
        
        // Common terminal commands
        let terminal_commands = vec![
            "ls", "cd", "pwd", "grep", "find", "cat", "less", "more",
            "mkdir", "rm", "cp", "mv", "chmod", "sudo", "git", "npm",
            "cargo", "python", "python3", "node", "npm", "yarn",
            "docker", "kubectl", "curl", "wget", "ssh", "scp",
        ];

        // Check if it starts with a known command
        let first_word = trimmed.split_whitespace().next().unwrap_or("");
        terminal_commands.iter().any(|&cmd| first_word == cmd)
    }
}

