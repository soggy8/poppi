//! Text matching shared by apps, windows, emojis and files.
//!
//! Scores are tiered so that an obviously better kind of match always wins:
//!
//! | tier | meaning                                   | example (`query` -> text)      |
//! |------|-------------------------------------------|--------------------------------|
//! | 1000 | exact                                     | `files` -> `files`             |
//! |  900 | prefix                                    | `fire` -> `firefox`            |
//! |  800 | prefix of a word                          | `code` -> `visual studio code` |
//! |  750 | every query word prefixes a word          | `vis co` -> `visual studio code` |
//! |  700 | initials                                  | `vsc` -> `visual studio code`  |
//! |  600 | substring                                 | `office` -> `libreoffice`      |
//! |  550 | every query word is a substring           |                                |
//! | 300+ | compact fuzzy match (typos, skipped chars) | `frfx` -> `firefox`            |
//!
//! Fuzzy matches must be compact and start at the beginning of a word. Loose
//! ones, where the letters are scattered across the text, are rejected: they are what made `fire` match "Fedora Media Writer".
//! Within a tier, shorter texts rank first. Both arguments must be lowercase.

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

pub struct Matcher {
    skim: SkimMatcherV2,
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}

fn length_penalty(text: &str) -> i64 {
    text.chars().count().min(50) as i64
}

impl Matcher {
    pub fn new() -> Self {
        Self {
            skim: SkimMatcherV2::default(),
        }
    }

    /// Exact, prefix and substring tiers only. Cheap enough for very large
    /// lists and strict enough for long texts such as descriptions.
    pub fn strict(query: &str, text: &str) -> Option<i64> {
        if query.is_empty() || text.is_empty() {
            return None;
        }
        let multi_word = query.contains(char::is_whitespace);
        let tier = if text == query {
            1000
        } else if text.starts_with(query) {
            900
        } else if words(text).any(|w| w.starts_with(query)) {
            800
        } else if multi_word
            && query
                .split_whitespace()
                .all(|token| words(text).any(|w| w.starts_with(token)))
        {
            750
        } else if text.contains(query) {
            600
        } else if multi_word && query.split_whitespace().all(|token| text.contains(token)) {
            550
        } else {
            return None;
        };
        Some(tier - length_penalty(text))
    }

    /// All tiers, including initials and compact fuzzy matches.
    pub fn score(&self, query: &str, text: &str) -> Option<i64> {
        let strict = Self::strict(query, text);
        if strict.is_some_and(|s| s >= 750 - 50) {
            return strict;
        }

        let query_len = query.chars().count();
        if query_len >= 2 && !query.contains(char::is_whitespace) {
            let initials: String = words(text).filter_map(|w| w.chars().next()).collect();
            if initials.chars().count() > 1 && initials.starts_with(query) {
                return Some(700 - length_penalty(text));
            }
        }
        if strict.is_some() {
            return strict;
        }

        if query_len < 3 {
            return None;
        }
        let (score, indices) = self.skim.fuzzy_indices(text, query)?;
        let first = *indices.first()?;
        let span = indices.last()? - first + 1;
        // The match has to be compact and begin at the start of a word;
        // otherwise "term" would match "inTERnet Messenger".
        let starts_word = first == 0
            || text
                .chars()
                .nth(first - 1)
                .is_some_and(|previous| !previous.is_alphanumeric());
        if span > query_len * 2 + 1 || !starts_word {
            return None;
        }
        Some(300 + score.clamp(0, 200) - length_penalty(text))
    }
}

/// Character positions in `text` that a query matched, for emphasis in the
/// UI. Follows the same order of preference as scoring. Empty when the
/// match came from somewhere else (a keyword, say) or cannot be mapped.
pub fn highlight(matcher: &Matcher, query: &str, text: &str) -> Vec<usize> {
    let query: Vec<char> = query.trim().to_lowercase().chars().collect();
    // One lowercase char per original char keeps positions aligned.
    let lower: Vec<char> = text
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    if query.is_empty() || lower.is_empty() {
        return Vec::new();
    }

    let starts_word = |i: usize| i == 0 || !lower[i - 1].is_alphanumeric();
    // Where `needle` occurs, preferring an occurrence at the start of a word.
    let locate = |needle: &[char]| -> Option<usize> {
        if needle.is_empty() || needle.len() > lower.len() {
            return None;
        }
        let mut first = None;
        for i in 0..=lower.len() - needle.len() {
            if lower[i..i + needle.len()] == *needle {
                if starts_word(i) {
                    return Some(i);
                }
                first.get_or_insert(i);
            }
        }
        first
    };

    if let Some(start) = locate(&query) {
        return (start..start + query.len()).collect();
    }

    let tokens: Vec<&[char]> = query.split(|c| c.is_whitespace()).filter(|t| !t.is_empty()).collect();
    if tokens.len() > 1 {
        let mut positions = Vec::new();
        for token in &tokens {
            match locate(token) {
                Some(start) => positions.extend(start..start + token.len()),
                None => return Vec::new(),
            }
        }
        positions.sort_unstable();
        positions.dedup();
        return positions;
    }

    let word_starts: Vec<usize> = (0..lower.len())
        .filter(|&i| lower[i].is_alphanumeric() && starts_word(i))
        .collect();
    if query.len() >= 2
        && word_starts.len() >= query.len()
        && query.iter().zip(&word_starts).all(|(q, &i)| lower[i] == *q)
    {
        return word_starts[..query.len()].to_vec();
    }

    let lower_text: String = lower.iter().collect();
    let query_text: String = query.iter().collect();
    if matcher.score(&query_text, &lower_text).is_some() {
        if let Some((_, indices)) = matcher.skim.fuzzy_indices(&lower_text, &query_text) {
            return indices;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, text: &str) -> Option<i64> {
        Matcher::new().score(query, text)
    }

    #[test]
    fn tiers_are_ordered() {
        let exact = score("files", "files").unwrap();
        let prefix = score("fire", "firefox").unwrap();
        let word = score("code", "visual studio code").unwrap();
        let initials = score("vsc", "visual studio code").unwrap();
        let substring = score("office", "libreoffice").unwrap();
        let fuzzy = score("frfx", "firefox").unwrap();
        assert!(exact > prefix);
        assert!(prefix > word);
        assert!(word > initials);
        assert!(initials > substring);
        assert!(substring > fuzzy);
        assert!(fuzzy > 0);
    }

    #[test]
    fn scattered_letters_do_not_match() {
        assert_eq!(score("fire", "fedora media writer"), None);
        assert_eq!(score("fire", "libreoffice impress"), None);
        assert_eq!(score("ter", "text editor"), None);
        assert_eq!(score("term", "internet messenger"), None);
        assert!(score("mesngr", "internet messenger").is_some());
    }

    #[test]
    fn short_queries_never_fuzzy_match() {
        assert_eq!(score("fx", "firefox"), None);
        assert!(score("fi", "firefox").is_some());
    }

    #[test]
    fn multi_word_queries() {
        assert!(score("vis co", "visual studio code").is_some());
        assert!(score("studio visual", "visual studio code").is_some());
        assert_eq!(score("visual nope", "visual studio code"), None);
    }

    #[test]
    fn shorter_text_wins_within_a_tier() {
        assert!(score("fire", "firefox").unwrap() > score("fire", "firewall configuration").unwrap());
    }

    #[test]
    fn highlight_positions() {
        let m = Matcher::new();
        assert_eq!(highlight(&m, "fire", "Firefox"), vec![0, 1, 2, 3]);
        // Prefers the word start over an earlier mid-word occurrence.
        assert_eq!(highlight(&m, "co", "Disco Code"), vec![6, 7]);
        assert_eq!(highlight(&m, "office", "LibreOffice"), vec![5, 6, 7, 8, 9, 10]);
        assert_eq!(highlight(&m, "vsc", "Visual Studio Code"), vec![0, 7, 14]);
        assert_eq!(highlight(&m, "stu vis", "Visual Studio Code"), vec![0, 1, 2, 7, 8, 9]);
        assert_eq!(highlight(&m, "frfx", "Firefox"), vec![0, 2, 4, 6]);
        assert!(highlight(&m, "browser", "Firefox").is_empty());
        assert!(highlight(&m, "", "Firefox").is_empty());
        assert_eq!(highlight(&m, "über", "Über App"), vec![0, 1, 2, 3]);
    }

    #[test]
    fn strict_has_no_fuzzy_or_initials() {
        assert_eq!(Matcher::strict("frfx", "firefox"), None);
        assert_eq!(Matcher::strict("vsc", "visual studio code"), None);
        assert!(Matcher::strict("stud", "visual studio code").is_some());
        assert_eq!(Matcher::strict("", "anything"), None);
    }
}
