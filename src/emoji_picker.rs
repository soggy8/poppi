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

/// Config value to skin tone; anything unknown means no modifier.
pub fn skin_tone(name: &str) -> emojis::SkinTone {
    match name.trim().to_lowercase().as_str() {
        "light" => emojis::SkinTone::Light,
        "medium-light" => emojis::SkinTone::MediumLight,
        "medium" => emojis::SkinTone::Medium,
        "medium-dark" => emojis::SkinTone::MediumDark,
        "dark" => emojis::SkinTone::Dark,
        _ => emojis::SkinTone::Default,
    }
}

pub const SKIN_TONES: [&str; 6] = ["default", "light", "medium-light", "medium", "medium-dark", "dark"];

impl EmojiPicker {
    /// `tone` is applied to every emoji that supports a skin tone.
    pub fn new(tone: emojis::SkinTone) -> Self {
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
                let shown = data.with_skin_tone(tone).unwrap_or(data);
                Entry {
                    emoji: Emoji {
                        emoji: shown.as_str(),
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

    /// With an empty query: recently picked emojis first, then the standard
    /// order. `recent` holds glyphs, most recent first.
    pub fn search(&self, query: &str, recent: &[String], limit: usize) -> Vec<Emoji> {
        if query.is_empty() {
            let mut results: Vec<Emoji> = recent
                .iter()
                .filter_map(|glyph| {
                    let data = emojis::get(glyph)?;
                    Some(Emoji {
                        emoji: data.as_str(),
                        name: data.name(),
                    })
                })
                .collect();
            for entry in &self.entries {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|e| e.emoji == entry.emoji.emoji) {
                    results.push(entry.emoji.clone());
                }
            }
            results.truncate(limit);
            return results;
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

    fn picker() -> EmojiPicker {
        EmojiPicker::new(emojis::SkinTone::Default)
    }

    #[test]
    fn empty_query_returns_recents_then_the_first_emojis() {
        let picker = picker();
        assert_eq!(picker.search("", &[], 24).len(), 24);

        let recent = vec!["🔥".to_string(), "not an emoji".to_string(), "👍".to_string()];
        let results = picker.search("", &recent, 24);
        assert_eq!(results.len(), 24);
        assert_eq!(results[0].emoji, "🔥");
        assert_eq!(results[1].emoji, "👍");
        assert_eq!(results.iter().filter(|e| e.emoji == "🔥").count(), 1);
    }

    #[test]
    fn search_finds_by_name_and_shortcode() {
        let picker = picker();
        assert!(picker.search("fire", &[], 24).iter().any(|e| e.emoji == "🔥"));
        assert!(picker.search("+1", &[], 24).iter().any(|e| e.emoji == "👍"));
        assert!(picker.search("zzzzqqqqxxxx", &[], 24).is_empty());
        assert_eq!(picker.search("FIRE", &[], 5), picker.search("fire", &[], 5));
    }

    #[test]
    fn skin_tone_is_applied_where_supported() {
        let toned = EmojiPicker::new(skin_tone("medium"));
        let thumbs = toned.search("thumbs up", &[], 5);
        assert_eq!(thumbs[0].emoji, "👍🏽");
        // Emojis without skin tones are unchanged.
        assert!(toned.search("fire", &[], 24).iter().any(|e| e.emoji == "🔥"));
        assert_eq!(skin_tone("nonsense"), emojis::SkinTone::Default);
        assert_eq!(skin_tone(SKIN_TONES[5]), emojis::SkinTone::Dark);
    }
}
