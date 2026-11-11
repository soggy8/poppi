use crate::utils;
use crate::matching::Matcher;
use std::io::Write;
use std::process::{Command, Stdio};

/// What a result carries: just the glyph and its name.
#[derive(Debug, Clone, PartialEq)]
pub struct Emoji {
    pub emoji: &'static str,
    pub name: &'static str,
}

struct Entry {
    emoji: Emoji,
    /// Lowercase name, name words and shortcodes.
    keywords: Vec<String>,
}

pub struct EmojiPicker {
    entries: Vec<Entry>,
    matcher: Matcher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inserted {
    Typed,
    CopiedToClipboard,
}

impl EmojiPicker {
    pub fn new() -> Self {
        let entries = emojis::iter()
            .map(|data| {
                let name = data.name();
                let mut keywords = vec![name.to_lowercase()];
                keywords.extend(
                    name.split([' ', '_', '-'])
                        .filter(|word| word.len() > 2)
                        .map(str::to_lowercase),
                );
                for shortcode in data.shortcodes() {
                    keywords.push(shortcode.to_lowercase());
                }
                Entry {
                    emoji: Emoji {
                        emoji: data.as_str(),
                        name,
                    },
                    keywords,
                }
            })
            .collect();
        Self {
            entries,
            matcher: Matcher::new(),
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<Emoji> {
        if query.is_empty() {
            return self
                .entries
                .iter()
                .take(limit)
                .map(|e| e.emoji.clone())
                .collect();
        }

        let query = query.to_lowercase();
        let mut results: Vec<(&Entry, i64)> = self
            .entries
            .iter()
            .filter_map(|entry| {
                entry
                    .keywords
                    .iter()
                    .filter_map(|kw| self.matcher.score(&query, kw))
                    .max()
                    .map(|score| (entry, score))
            })
            .collect();

        // Stable sort keeps the Unicode ordering among equal scores.
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results
            .into_iter()
            .take(limit)
            .map(|(entry, _)| entry.emoji.clone())
            .collect()
    }

    /// Type the emoji into the focused window, or copy it to the clipboard
    /// when no typing tool works. Call this after the launcher has given up
    /// focus, otherwise the launcher itself receives the keystrokes.
    pub fn insert(emoji: &str) -> Result<Inserted, String> {
        if Self::type_text(emoji) {
            return Ok(Inserted::Typed);
        }
        utils::copy_to_clipboard(emoji).map(|()| Inserted::CopiedToClipboard)
    }

    fn type_text(text: &str) -> bool {
        let succeeded = |command: &mut Command| {
            command
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };

        if !utils::is_wayland() {
            return utils::has_command("xdotool")
                && succeeded(Command::new("xdotool").args(["type", "--clearmodifiers", text]));
        }

        // wtype needs the virtual-keyboard protocol (wlroots compositors;
        // GNOME does not provide it, so this fails there and we fall through).
        if utils::has_command("wtype") && succeeded(Command::new("wtype").arg(text)) {
            return true;
        }

        if utils::has_command("ydotool") {
            let child = Command::new("ydotool")
                .args(["type", "--file", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(mut child) = child {
                let written = child
                    .stdin
                    .take()
                    .map(|mut stdin| stdin.write_all(text.as_bytes()).is_ok())
                    .unwrap_or(false);
                let exited_ok = child.wait().map(|s| s.success()).unwrap_or(false);
                return written && exited_ok;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_the_first_emojis() {
        let picker = EmojiPicker::new();
        assert_eq!(picker.search("", 24).len(), 24);
    }

    #[test]
    fn search_finds_by_name_and_shortcode() {
        let picker = EmojiPicker::new();
        let fire = picker.search("fire", 24);
        assert!(fire.iter().any(|e| e.emoji == "🔥"));
        let thumbs = picker.search("+1", 24);
        assert!(thumbs.iter().any(|e| e.emoji == "👍"));
        assert!(picker.search("zzzzqqqqxxxx", 24).is_empty());
    }

    #[test]
    fn search_is_case_insensitive() {
        let picker = EmojiPicker::new();
        assert_eq!(picker.search("FIRE", 5), picker.search("fire", 5));
    }
}
