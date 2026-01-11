use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

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
            return self.emojis.iter().map(|e| (e, 0)).take(20).collect();
        }

        let query_lower = query.to_lowercase();
        let mut results: Vec<(&Emoji, i64)> = Vec::with_capacity(20);
        
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
        results.truncate(20);
        results
    }

    pub fn insert_emoji(emoji: &str) -> Result<(), Box<dyn std::error::Error>> {
        use std::process::Command;
        
        // Use xdotool to type the emoji (X11)
        // For Wayland, we'd need a different approach
        Command::new("xdotool")
            .arg("type")
            .arg("--clearmodifiers")
            .arg(emoji)
            .output()?;
        
        Ok(())
    }
}

