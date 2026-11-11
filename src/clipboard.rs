//! Recent clipboard texts, kept in memory only (never written to disk, so
//! copied passwords do not outlive the process).

use std::collections::VecDeque;

const MAX_ENTRY_CHARS: usize = 100_000;

#[derive(Debug, Default)]
pub struct ClipboardHistory {
    entries: VecDeque<String>,
    capacity: usize,
}

impl ClipboardHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
        }
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        self.entries.truncate(capacity);
    }

    /// Remember a text, most recent first, without duplicates.
    pub fn push(&mut self, text: &str) {
        if self.capacity == 0 || text.trim().is_empty() || text.chars().count() > MAX_ENTRY_CHARS {
            return;
        }
        self.entries.retain(|entry| entry != text);
        self.entries.push_front(text.to_string());
        self.entries.truncate(self.capacity);
    }

    pub fn search(&self, filter: &str, limit: usize) -> Vec<String> {
        let filter = filter.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| filter.is_empty() || entry.to_lowercase().contains(&filter))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One line for a list row: whitespace collapsed, long texts cut.
pub fn preview(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 120 {
        let cut: String = collapsed.chars().take(119).collect();
        format!("{}…", cut)
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_recent_first_without_duplicates() {
        let mut history = ClipboardHistory::new(3);
        history.push("one");
        history.push("two");
        history.push("one");
        history.push("   ");
        assert_eq!(history.search("", 10), vec!["one", "two"]);
        history.push("three");
        history.push("four");
        assert_eq!(history.search("", 10), vec!["four", "three", "one"]);
        assert_eq!(history.search("THR", 10), vec!["three"]);
        assert_eq!(history.search("", 1).len(), 1);
    }

    #[test]
    fn capacity_zero_disables_it() {
        let mut history = ClipboardHistory::new(0);
        history.push("secret");
        assert!(history.is_empty());

        let mut history = ClipboardHistory::new(5);
        history.push("a");
        history.set_capacity(0);
        assert!(history.is_empty());
    }

    #[test]
    fn previews_are_single_line_and_bounded() {
        assert_eq!(preview("a\n  b\tc"), "a b c");
        let long = "x".repeat(500);
        assert_eq!(preview(&long).chars().count(), 120);
        assert!(preview(&long).ends_with('…'));
    }
}
