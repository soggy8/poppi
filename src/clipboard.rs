//! Recent clipboard texts, kept in memory only (never written to disk, so
//! copied passwords do not outlive the process). Snippets the user explicitly
//! pins are listed first and saved in `clipboard_pins.json` (mode `0600`).

use std::collections::VecDeque;
use std::path::PathBuf;

const MAX_ENTRY_CHARS: usize = 100_000;
const MAX_PINS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEntry {
    pub text: String,
    pub pinned: bool,
}

#[derive(Debug, Default)]
pub struct ClipboardHistory {
    entries: VecDeque<String>,
    pinned: Vec<String>,
    pins_path: Option<PathBuf>,
    capacity: usize,
}

impl ClipboardHistory {
    #[cfg(test)]
    pub fn new(capacity: usize) -> Self {
        Self::from_pins(None, capacity)
    }

    pub fn load(capacity: usize) -> Self {
        let path = dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|dir| dir.join("poppi_launcher").join("clipboard_pins.json"));
        Self::from_pins(path, capacity)
    }

    pub fn from_pins(pins_path: Option<PathBuf>, capacity: usize) -> Self {
        let mut pinned: Vec<String> = pins_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        pinned.truncate(MAX_PINS);
        Self {
            entries: VecDeque::new(),
            pinned,
            pins_path,
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
        if self.pinned.iter().any(|p| p == text) {
            return;
        }
        self.entries.retain(|entry| entry != text);
        self.entries.push_front(text.to_string());
        self.entries.truncate(self.capacity);
    }

    #[cfg(test)]
    pub fn is_pinned(&self, text: &str) -> bool {
        self.pinned.iter().any(|p| p == text)
    }

    /// Pin or unpin `text`. Returns `true` when it is now pinned.
    pub fn toggle_pin(&mut self, text: &str) -> bool {
        if let Some(pos) = self.pinned.iter().position(|p| p == text) {
            let unpinned = self.pinned.remove(pos);
            if self.capacity > 0 {
                self.entries.retain(|e| e != &unpinned);
                self.entries.push_front(unpinned);
                self.entries.truncate(self.capacity);
            }
            self.save_pins();
            false
        } else {
            self.entries.retain(|e| e != text);
            self.pinned.insert(0, text.to_string());
            self.pinned.truncate(MAX_PINS);
            self.save_pins();
            true
        }
    }

    pub fn remove(&mut self, text: &str) {
        let before = self.pinned.len();
        self.pinned.retain(|p| p != text);
        self.entries.retain(|e| e != text);
        if self.pinned.len() != before {
            self.save_pins();
        }
    }

    pub fn search(&self, filter: &str, limit: usize) -> Vec<ClipboardEntry> {
        if self.capacity == 0 {
            return Vec::new();
        }
        let filter = filter.trim().to_lowercase();
        let matches = |entry: &String| filter.is_empty() || entry.to_lowercase().contains(&filter);
        self.pinned
            .iter()
            .filter(|e| matches(e))
            .map(|e| ClipboardEntry {
                text: e.clone(),
                pinned: true,
            })
            .chain(
                self.entries
                    .iter()
                    .filter(|e| matches(e))
                    .map(|e| ClipboardEntry {
                        text: e.clone(),
                        pinned: false,
                    }),
            )
            .take(limit)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.capacity == 0 || (self.pinned.is_empty() && self.entries.is_empty())
    }

    fn save_pins(&self) {
        let Some(path) = &self.pins_path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(&self.pinned) {
            if std::fs::write(path, json).is_ok() {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
        }
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

    fn texts(entries: Vec<ClipboardEntry>) -> Vec<String> {
        entries.into_iter().map(|e| e.text).collect()
    }

    #[test]
    fn most_recent_first_without_duplicates() {
        let mut history = ClipboardHistory::new(3);
        history.push("one");
        history.push("two");
        history.push("one");
        history.push("   ");
        assert_eq!(texts(history.search("", 10)), vec!["one", "two"]);
        history.push("three");
        history.push("four");
        assert_eq!(texts(history.search("", 10)), vec!["four", "three", "one"]);
        assert_eq!(texts(history.search("THR", 10)), vec!["three"]);
        assert_eq!(history.search("", 1).len(), 1);
    }

    #[test]
    fn pinned_entries_lead_survive_eviction_and_persist() {
        let path = std::env::temp_dir().join(format!("poppi-pins-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut history = ClipboardHistory::from_pins(Some(path.clone()), 2);
        history.push("alpha");
        history.push("beta");
        assert!(history.toggle_pin("alpha"));
        assert!(history.is_pinned("alpha"));
        history.push("gamma");
        history.push("delta");

        let listed = history.search("", 10);
        assert_eq!(
            listed[0],
            ClipboardEntry {
                text: "alpha".into(),
                pinned: true
            }
        );
        assert_eq!(texts(listed), vec!["alpha", "delta", "gamma"]);

        // Reloading from disk restores only the pinned entry.
        let reloaded = ClipboardHistory::from_pins(Some(path.clone()), 2);
        assert_eq!(
            reloaded.search("", 10),
            vec![ClipboardEntry {
                text: "alpha".into(),
                pinned: true
            }]
        );

        history.remove("delta");
        assert!(!history.toggle_pin("alpha"));
        assert_eq!(texts(history.search("", 10)), vec!["alpha", "gamma"]);

        let _ = std::fs::remove_file(&path);
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
