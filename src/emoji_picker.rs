use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::io::Write;

#[derive(Debug, Clone)]
pub struct Emoji {
    pub emoji: String,
    pub name: String,
    pub keywords: Vec<String>,
}

pub struct EmojiPicker {
    emojis: Vec<Emoji>,
    matcher: SkimMatcherV2,
}

impl EmojiPicker {
    pub fn new() -> Self {
        let emojis = Self::load_emojis();
        let matcher = SkimMatcherV2::default();
        Self { emojis, matcher }
    }

    fn load_emojis() -> Vec<Emoji> {
        // Load all emojis from the emojis crate
        let mut emoji_list = Vec::with_capacity(3000); // Pre-allocate for performance
        
        for emoji_data in emojis::iter() {
            let emoji_char = emoji_data.as_str();
            let name = emoji_data.name().to_string();
            
            // Build keywords from name (split by space, underscore, hyphen)
            let mut keywords = Vec::new();
            // Add the full name as a keyword
            keywords.push(name.clone());
            // Split name into words for better search
            for word in name.split(&[' ', '_', '-'][..]) {
                if !word.is_empty() && word.len() > 2 {
                    keywords.push(word.to_lowercase());
                }
            }
            
            // Add shortcodes as keywords if available
            for shortcode in emoji_data.shortcodes() {
                keywords.push(shortcode.to_string());
                // Also add shortcode parts
                for part in shortcode.split('-') {
                    if !part.is_empty() {
                        keywords.push(part.to_string());
                    }
                }
            }
            
            emoji_list.push(Emoji {
                emoji: emoji_char.to_string(),
                name,
                keywords,
            });
        }
        
        emoji_list
    }

    pub fn search(&self, query: &str) -> Vec<(&Emoji, i64)> {
        if query.is_empty() {
            return self.emojis.iter().map(|e| (e, 0)).take(24).collect();
        }

        let query_lower = query.to_lowercase();
        let mut results: Vec<(&Emoji, i64)> = Vec::with_capacity(24);
        
        for emoji in &self.emojis {
            // Match against name
            let name_score = self.matcher.fuzzy_match(&emoji.name, &query_lower);
            
            // Match against keywords (compute lowercase on the fly is fine for small set)
            let keyword_score = emoji.keywords.iter()
                .filter_map(|kw| self.matcher.fuzzy_match(kw, &query_lower))
                .max();

            let score = name_score.unwrap_or(0).max(keyword_score.unwrap_or(0));
            
            if score > 0 {
                results.push((emoji, score));
            }
        }

        // Use unstable sort for better performance
        results.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        results.truncate(24);
        results
    }

    pub fn insert_emoji(emoji: &str) -> Result<(), Box<dyn std::error::Error>> {
        use std::process::Command;
        use std::io::Write;
        
        // Check if we're on Wayland or X11
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok() || 
                         std::env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false);
        
        let mut insertion_successful = false;
        
        if is_wayland {
            // Try wtype for Wayland (if available)
            if Self::has_command("wtype") {
                let output = Command::new("wtype")
                    .arg(emoji)
                    .output();
                
                if let Ok(output) = output {
                    if output.status.success() {
                        insertion_successful = true;
                    }
                }
            }
            
            // Fallback: try ydotool for Wayland (if available)
            if !insertion_successful && Self::has_command("ydotool") {
                let mut child = Command::new("ydotool")
                    .arg("type")
                    .arg("--file")
                    .arg("-")
                    .stdin(std::process::Stdio::piped())
                    .spawn();
                
                if let Ok(mut child) = child {
                    if let Some(stdin) = child.stdin.as_mut() {
                        if stdin.write_all(emoji.as_bytes()).is_ok() {
                            if child.wait().map(|s| s.success()).unwrap_or(false) {
                                insertion_successful = true;
                            }
                        }
                    }
                }
            }
        } else {
            // Try xdotool for X11
            if Self::has_command("xdotool") {
                let output = Command::new("xdotool")
                    .arg("type")
                    .arg("--clearmodifiers")
                    .arg(emoji)
                    .output();
                
                if let Ok(output) = output {
                    if output.status.success() {
                        insertion_successful = true;
                    }
                }
            }
        }
        
        // If insertion failed or no tool available, copy to clipboard
        if !insertion_successful {
            Self::copy_to_clipboard(emoji)?;
        }
        
        Ok(())
    }
    
    fn copy_to_clipboard(text: &str) -> Result<(), Box<dyn std::error::Error>> {
        use std::process::{Command, Stdio};
        
        // Try wl-copy for Wayland
        if Self::has_command("wl-copy") {
            let mut child = Command::new("wl-copy")
                .stdin(Stdio::piped())
                .spawn()?;
            
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()?;
            return Ok(());
        }
        
        // Try xclip for X11
        if Self::has_command("xclip") {
            let mut child = Command::new("xclip")
                .arg("-selection")
                .arg("clipboard")
                .stdin(Stdio::piped())
                .spawn()?;
            
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()?;
            return Ok(());
        }
        
        // Try xsel as fallback
        if Self::has_command("xsel") {
            let mut child = Command::new("xsel")
                .arg("--clipboard")
                .arg("--input")
                .stdin(Stdio::piped())
                .spawn()?;
            
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()?;
            return Ok(());
        }
        
        Err("No clipboard tool available (wl-copy, xclip, or xsel)".into())
    }
    
    fn has_command(cmd: &str) -> bool {
        use std::process::Command;
        Command::new("sh")
            .arg("-c")
            .arg(&format!("command -v {}", cmd))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

