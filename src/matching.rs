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
    fn strict_has_no_fuzzy_or_initials() {
        assert_eq!(Matcher::strict("frfx", "firefox"), None);
        assert_eq!(Matcher::strict("vsc", "visual studio code"), None);
        assert!(Matcher::strict("stud", "visual studio code").is_some());
        assert_eq!(Matcher::strict("", "anything"), None);
    }
}
