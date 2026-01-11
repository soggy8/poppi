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
        // Common emojis with names and keywords
        vec![
            Emoji { emoji: "😀".to_string(), name: "grinning".to_string(), keywords: vec!["happy", "smile", "grin"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "😂".to_string(), name: "laughing".to_string(), keywords: vec!["laugh", "tears", "happy"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "❤️".to_string(), name: "heart".to_string(), keywords: vec!["love", "red", "heart"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🔥".to_string(), name: "fire".to_string(), keywords: vec!["fire", "hot", "flame"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "👍".to_string(), name: "thumbs up".to_string(), keywords: vec!["thumbs", "up", "good", "like"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "👎".to_string(), name: "thumbs down".to_string(), keywords: vec!["thumbs", "down", "bad", "dislike"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🎉".to_string(), name: "party".to_string(), keywords: vec!["party", "celebration", "confetti"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "✨".to_string(), name: "sparkles".to_string(), keywords: vec!["sparkle", "star", "magic"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "⭐".to_string(), name: "star".to_string(), keywords: vec!["star", "favorite"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "😊".to_string(), name: "smiling".to_string(), keywords: vec!["smile", "happy", "pleased"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "😍".to_string(), name: "heart eyes".to_string(), keywords: vec!["love", "heart", "eyes"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🤔".to_string(), name: "thinking".to_string(), keywords: vec!["think", "thinking", "hmm"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "😎".to_string(), name: "cool".to_string(), keywords: vec!["cool", "sunglasses", "awesome"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🎯".to_string(), name: "target".to_string(), keywords: vec!["target", "goal", "aim"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🚀".to_string(), name: "rocket".to_string(), keywords: vec!["rocket", "launch", "fast"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "💡".to_string(), name: "lightbulb".to_string(), keywords: vec!["idea", "light", "bulb"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "✅".to_string(), name: "checkmark".to_string(), keywords: vec!["check", "ok", "yes"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "❌".to_string(), name: "cross".to_string(), keywords: vec!["no", "wrong", "cancel"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🎨".to_string(), name: "art".to_string(), keywords: vec!["art", "paint", "color"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🎵".to_string(), name: "music".to_string(), keywords: vec!["music", "song", "note"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "📝".to_string(), name: "memo".to_string(), keywords: vec!["note", "write", "memo"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "📷".to_string(), name: "camera".to_string(), keywords: vec!["photo", "camera", "picture"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🌍".to_string(), name: "earth".to_string(), keywords: vec!["world", "earth", "globe"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🌟".to_string(), name: "glowing star".to_string(), keywords: vec!["star", "glow", "bright"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "💻".to_string(), name: "computer".to_string(), keywords: vec!["computer", "laptop", "code"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "📱".to_string(), name: "phone".to_string(), keywords: vec!["phone", "mobile", "smartphone"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🎮".to_string(), name: "game".to_string(), keywords: vec!["game", "gaming", "controller"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🍕".to_string(), name: "pizza".to_string(), keywords: vec!["pizza", "food"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "☕".to_string(), name: "coffee".to_string(), keywords: vec!["coffee", "drink"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🌙".to_string(), name: "moon".to_string(), keywords: vec!["moon", "night"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "☀️".to_string(), name: "sun".to_string(), keywords: vec!["sun", "day", "sunny"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🌈".to_string(), name: "rainbow".to_string(), keywords: vec!["rainbow", "color"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🎁".to_string(), name: "gift".to_string(), keywords: vec!["gift", "present"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🎂".to_string(), name: "cake".to_string(), keywords: vec!["cake", "birthday"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "😢".to_string(), name: "crying".to_string(), keywords: vec!["sad", "cry", "tears"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "😴".to_string(), name: "sleeping".to_string(), keywords: vec!["sleep", "tired", "zzz"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🤗".to_string(), name: "hugging".to_string(), keywords: vec!["hug", "hugging"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🙏".to_string(), name: "praying".to_string(), keywords: vec!["pray", "thanks", "please"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "👏".to_string(), name: "clapping".to_string(), keywords: vec!["clap", "applause"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "💪".to_string(), name: "muscle".to_string(), keywords: vec!["strong", "muscle", "power"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "🎓".to_string(), name: "graduation".to_string(), keywords: vec!["graduate", "degree", "cap"].iter().map(|s| s.to_string()).collect() },
            Emoji { emoji: "💰".to_string(), name: "money".to_string(), keywords: vec!["money", "cash", "dollar"].iter().map(|s| s.to_string()).collect() },
        ]
    }

    pub fn search(&self, query: &str) -> Vec<(&Emoji, i64)> {
        if query.is_empty() {
            return self.emojis.iter().map(|e| (e, 0)).take(20).collect();
        }

        let query_lower = query.to_lowercase();
        let mut results: Vec<(&Emoji, i64)> = self
            .emojis
            .iter()
            .filter_map(|emoji| {
                // Match against name
                let name_score = self.matcher.fuzzy_match(&emoji.name.to_lowercase(), &query_lower);
                
                // Match against keywords
                let keyword_score = emoji.keywords.iter()
                    .filter_map(|kw| self.matcher.fuzzy_match(&kw.to_lowercase(), &query_lower))
                    .max();

                let score = name_score.unwrap_or(0).max(keyword_score.unwrap_or(0));
                
                if score > 0 {
                    Some((emoji, score))
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| b.1.cmp(&a.1));
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

