//! Queries that were run, most recent first.

use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct History {
    entries: Vec<String>,
    path: Option<PathBuf>,
    capacity: usize,
}

impl History {
    fn default_path() -> Option<PathBuf> {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|dir| dir.join("poppi_launcher").join("history.json"))
    }

    pub fn load(capacity: usize) -> Self {
        Self::load_from(Self::default_path(), capacity)
    }

    pub fn load_from(path: Option<PathBuf>, capacity: usize) -> Self {
        let mut entries: Vec<String> = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        entries.truncate(capacity);
        Self {
            entries,
            path,
            capacity,
        }
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// A capacity of zero turns history off and forgets what was stored.
    pub fn set_capacity(&mut self, capacity: usize) {
        if capacity == self.capacity {
            return;
        }
        self.capacity = capacity;
        if self.entries.len() > capacity {
            self.entries.truncate(capacity);
            self.save();
        }
    }

    pub fn push(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() || self.capacity == 0 {
            return;
        }
        self.entries.retain(|entry| entry != query);
        self.entries.insert(0, query.to_string());
        self.entries.truncate(self.capacity);
        self.save();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let result = path
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .map_err(|e| e.to_string())
            .and_then(|()| serde_json::to_string(&self.entries).map_err(|e| e.to_string()))
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        if let Err(e) = result {
            eprintln!("Could not save history: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_recent_first_without_duplicates() {
        let mut history = History::load_from(None, 10);
        history.push("firefox");
        history.push("2+2");
        history.push("  firefox  ");
        history.push("");
        assert_eq!(history.entries(), ["firefox", "2+2"]);
    }

    #[test]
    fn capacity_is_enforced() {
        let mut history = History::load_from(None, 2);
        history.push("a");
        history.push("b");
        history.push("c");
        assert_eq!(history.entries(), ["c", "b"]);

        history.set_capacity(1);
        assert_eq!(history.entries(), ["c"]);

        history.set_capacity(0);
        history.push("d");
        assert!(history.entries().is_empty());
    }

    #[test]
    fn persists_and_clears() {
        let path = std::env::temp_dir().join(format!("poppi-history-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut history = History::load_from(Some(path.clone()), 10);
        history.push("yt cats");
        history.push("sw kit");
        assert_eq!(
            History::load_from(Some(path.clone()), 10).entries(),
            ["sw kit", "yt cats"]
        );

        history.clear();
        assert!(History::load_from(Some(path.clone()), 10).entries().is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
