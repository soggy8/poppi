//! File search (`f name`) over an in-memory index, and path browsing
//! (`~/Doc`, `/etc/`) straight from the file system.

use crate::config::FilesConfig;
use crate::matching::Matcher;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// Upper bound on indexed entries; the walk is breadth-first, so when the
/// cap is hit it is the deepest files that are left out.
const MAX_ENTRIES: usize = 200_000;

#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    name_lower: String,
    depth: usize,
}

impl FileEntry {
    fn new(path: PathBuf, name: String, is_dir: bool, depth: usize) -> Self {
        Self {
            name_lower: name.to_lowercase(),
            path,
            name,
            is_dir,
            depth,
        }
    }

    /// The containing directory, with the home directory shortened to `~`.
    pub fn parent_display(&self) -> String {
        self.path
            .parent()
            .map(tilde_path)
            .unwrap_or_else(|| "/".to_string())
    }
}

/// `~` and `~/x` to absolute paths; anything else unchanged.
pub fn expand_tilde(input: &str) -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        if input == "~" {
            return home;
        }
        if let Some(rest) = input.strip_prefix("~/") {
            return home.join(rest);
        }
    }
    PathBuf::from(input)
}

/// Absolute path to a display string, with the home directory as `~`.
pub fn tilde_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.display())
            };
        }
    }
    path.display().to_string()
}

/// Walk the configured roots. Blocking; run it off the main thread.
pub fn build_index(config: &FilesConfig) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = config
        .roots
        .iter()
        .map(|root| (expand_tilde(root), 1))
        .collect();

    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if (!config.include_hidden && name.starts_with('.'))
                || config.exclude.contains(&name)
            {
                continue;
            }
            // file_type() does not follow symlinks, so linked directories are
            // listed but never descended into (no cycles).
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            let is_dir = file_type.is_dir();
            if is_dir && depth < config.max_depth {
                queue.push_back((path.clone(), depth + 1));
            }
            entries.push(FileEntry::new(path, name, is_dir, depth));
            if entries.len() >= MAX_ENTRIES {
                return entries;
            }
        }
    }
    entries
}

/// Rank indexed files by name. Uses the strict tiers only (no fuzzy), which
/// keeps a keystroke over a few hundred thousand names in the millisecond range.
pub fn search<'a>(query: &str, entries: &'a [FileEntry], limit: usize) -> Vec<&'a FileEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut results: Vec<(&FileEntry, i64)> = entries
        .iter()
        .filter_map(|entry| {
            Matcher::strict(&query, &entry.name_lower)
                // Prefer files closer to the root among similar matches.
                .map(|score| (entry, score - entry.depth as i64 * 3))
        })
        .collect();
    results.sort_by(|a, b| b.1.cmp(&a.1));
    results
        .into_iter()
        .take(limit)
        .map(|(entry, _)| entry)
        .collect()
}

/// Entries for a partially typed path: `~/Do` lists what in `~` starts with
/// "do"; `~/Documents/` lists that directory. Directories come first.
pub fn complete_path(input: &str, limit: usize) -> Vec<FileEntry> {
    let expanded = expand_tilde(input);
    let (dir, partial) = if input.ends_with('/') || input == "~" {
        (expanded, String::new())
    } else {
        let partial = expanded
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let dir = expanded
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"));
        (dir, partial)
    };

    let Ok(read_dir) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let show_hidden = partial.starts_with('.');
    let mut entries: Vec<FileEntry> = read_dir
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !show_hidden {
                return None;
            }
            if !name.to_lowercase().starts_with(&partial) {
                return None;
            }
            let path = entry.path();
            let is_dir = path.is_dir();
            Some(FileEntry::new(path, name, is_dir, 0))
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name_lower.cmp(&b.name_lower))
    });
    entries.truncate(limit);
    entries
}

/// What Tab puts in the search bar for an entry: a path that can be typed
/// further, with a trailing slash on directories.
pub fn completion_text(entry: &FileEntry) -> String {
    let mut text = tilde_path(&entry.path);
    if entry.is_dir && !text.ends_with('/') {
        text.push('/');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("poppi-files-{}-{}", tag, std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("Documents/reports")).unwrap();
            fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
            fs::create_dir_all(root.join(".hidden")).unwrap();
            fs::write(root.join("Documents/report-2026.pdf"), "").unwrap();
            fs::write(root.join("Documents/reports/annual report.txt"), "").unwrap();
            fs::write(root.join("Documents/notes.md"), "").unwrap();
            fs::write(root.join("node_modules/pkg/report.js"), "").unwrap();
            fs::write(root.join(".hidden/report.secret"), "").unwrap();
            fs::write(root.join("deploy.sh"), "").unwrap();
            Self(root)
        }

        fn config(&self) -> FilesConfig {
            FilesConfig {
                roots: vec![self.0.to_string_lossy().into_owned()],
                ..FilesConfig::default()
            }
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn names(entries: &[&FileEntry]) -> Vec<String> {
        entries.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn index_skips_hidden_and_excluded() {
        let tree = TempTree::new("index");
        let index = build_index(&tree.config());
        let all: Vec<&str> = index.iter().map(|e| e.name.as_str()).collect();
        assert!(all.contains(&"report-2026.pdf"));
        assert!(all.contains(&"annual report.txt"));
        assert!(all.contains(&"Documents"));
        assert!(!all.contains(&"report.js"));
        assert!(!all.contains(&"report.secret"));
        assert!(!all.contains(&"node_modules"));
    }

    #[test]
    fn index_respects_depth_and_hidden_options() {
        let tree = TempTree::new("depth");
        let mut config = tree.config();
        config.max_depth = 1;
        let shallow = build_index(&config);
        assert!(shallow.iter().any(|e| e.name == "Documents"));
        assert!(!shallow.iter().any(|e| e.name == "notes.md"));

        config.max_depth = 6;
        config.include_hidden = true;
        assert!(build_index(&config).iter().any(|e| e.name == "report.secret"));
    }

    #[test]
    fn search_ranks_prefix_first_and_supports_words() {
        let tree = TempTree::new("search");
        let index = build_index(&tree.config());

        let reports = names(&search("report", &index, 10));
        assert_eq!(reports[0], "reports");
        assert!(reports.contains(&"report-2026.pdf".to_string()));
        assert!(reports.contains(&"annual report.txt".to_string()));

        assert_eq!(names(&search("ann rep", &index, 10)), vec!["annual report.txt"]);
        assert_eq!(names(&search("NOTES", &index, 10)), vec!["notes.md"]);
        assert!(search("zzzz", &index, 10).is_empty());
        assert_eq!(search("report", &index, 1).len(), 1);
    }

    #[test]
    fn path_completion_lists_directories_first() {
        let tree = TempTree::new("complete");
        let root = tree.0.to_string_lossy().into_owned();

        let top = complete_path(&format!("{}/", root), 50);
        let top_names: Vec<&str> = top.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(top_names, vec!["Documents", "node_modules", "deploy.sh"]);

        let d = complete_path(&format!("{}/d", root), 50);
        let d_names: Vec<&str> = d.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(d_names, vec!["Documents", "deploy.sh"]);

        let hidden = complete_path(&format!("{}/.h", root), 50);
        assert_eq!(hidden.len(), 1);
        assert!(complete_path(&format!("{}/nope/", root), 50).is_empty());

        assert!(completion_text(&top[0]).ends_with("Documents/"));
        assert!(completion_text(&top[2]).ends_with("deploy.sh"));
    }

    #[test]
    fn tilde_round_trip() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand_tilde("~"), home);
            assert_eq!(expand_tilde("~/x/y"), home.join("x/y"));
            assert_eq!(tilde_path(&home.join("x/y")), "~/x/y");
            assert_eq!(tilde_path(&home), "~");
        }
        assert_eq!(expand_tilde("/etc"), PathBuf::from("/etc"));
        assert_eq!(tilde_path(Path::new("/etc/hosts")), "/etc/hosts");
    }
}
