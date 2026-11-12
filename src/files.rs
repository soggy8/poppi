//! File search (`f name`) over an in-memory index, and path browsing
//! (`~/Doc`, `/etc/`) straight from the file system.

use crate::config::FilesConfig;
use crate::matching::Matcher;
use crate::usage::Usage;
use gtk::gio;
use gtk::prelude::*;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Upper bound on indexed entries; the walk is breadth-first, so when the
/// cap is hit it is the deepest files that are left out.
const MAX_ENTRIES: usize = 200_000;
/// Largest `.gitignore` file read during indexing.
const MAX_GITIGNORE_BYTES: u64 = 65_536;
/// Upper bound on directories watched for live invalidation.
const MAX_WATCHED_DIRS: usize = 48;
/// Largest text file read during `g <pattern>` content search (1 MiB).
const MAX_GREP_BYTES: u64 = 1_048_576;
/// Maximum candidate files scanned in one content search.
const MAX_GREP_FILES: usize = 25_000;

#[derive(Debug, Clone, PartialEq)]
pub struct ContentMatch {
    pub file: FileEntry,
    pub line_number: usize,
    pub line_preview: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    /// Unix time of the last modification; 0 when unknown.
    pub modified: u64,
    name_lower: String,
    depth: usize,
}

impl FileEntry {
    fn new(path: PathBuf, name: String, is_dir: bool, depth: usize, modified: u64) -> Self {
        Self {
            modified,
            name_lower: name.to_lowercase(),
            path,
            name,
            is_dir,
            depth,
        }
    }

    fn from_path(path: PathBuf, name: String, is_dir: bool, depth: usize) -> Self {
        let modified = std::fs::symlink_metadata(&path)
            .ok()
            .and_then(|m| modified_secs(&m))
            .unwrap_or(0);
        Self::new(path, name, is_dir, depth, modified)
    }

    /// Key under which opening this file is counted.
    pub fn usage_id(&self) -> String {
        format!("file:{}", self.path.display())
    }

    /// The containing directory, with the home directory shortened to `~`.
    pub fn parent_display(&self) -> String {
        self.path
            .parent()
            .map(tilde_path)
            .unwrap_or_else(|| "/".to_string())
    }
}

fn modified_secs(metadata: &std::fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

// ---------- .gitignore matching ----------

#[derive(Debug, Clone)]
enum GlobToken {
    Literal(char),
    AnyNonSlash,
    Star,
    DoubleStar,
    DoubleStarSlash,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

#[derive(Debug, Clone)]
struct IgnoreRule {
    negated: bool,
    dir_only: bool,
    anchored: bool,
    tokens: Vec<GlobToken>,
}

#[derive(Debug)]
struct IgnoreSet {
    base: PathBuf,
    rules: Vec<IgnoreRule>,
}

fn tokenize_glob(pattern: &str) -> Vec<GlobToken> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                tokens.push(GlobToken::Literal(chars[i + 1]));
                i += 2;
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                if chars.get(i + 2) == Some(&'/') {
                    tokens.push(GlobToken::DoubleStarSlash);
                    i += 3;
                } else {
                    tokens.push(GlobToken::DoubleStar);
                    i += 2;
                }
            }
            '*' => {
                tokens.push(GlobToken::Star);
                i += 1;
            }
            '?' => {
                tokens.push(GlobToken::AnyNonSlash);
                i += 1;
            }
            '[' => {
                let mut j = i + 1;
                let negated = matches!(chars.get(j), Some('!' | '^'));
                if negated {
                    j += 1;
                }
                let mut ranges = Vec::new();
                while j < chars.len() && chars[j] != ']' {
                    let start = if chars[j] == '\\' && j + 1 < chars.len() {
                        j += 1;
                        chars[j]
                    } else {
                        chars[j]
                    };
                    if chars.get(j + 1) == Some(&'-') && chars.get(j + 2).is_some_and(|&c| c != ']')
                    {
                        let end = chars[j + 2];
                        ranges.push((start, end));
                        j += 3;
                    } else {
                        ranges.push((start, start));
                        j += 1;
                    }
                }
                if j < chars.len() && chars[j] == ']' && !ranges.is_empty() {
                    tokens.push(GlobToken::Class { negated, ranges });
                    i = j + 1;
                } else {
                    tokens.push(GlobToken::Literal('['));
                    i += 1;
                }
            }
            c => {
                tokens.push(GlobToken::Literal(c));
                i += 1;
            }
        }
    }
    tokens
}

fn glob_matches(tokens: &[GlobToken], text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut dp = vec![false; n + 1];
    dp[0] = true;

    for token in tokens {
        let mut next = vec![false; n + 1];
        match token {
            GlobToken::Literal(expected) => {
                for j in 0..n {
                    if dp[j] && chars[j] == *expected {
                        next[j + 1] = true;
                    }
                }
            }
            GlobToken::AnyNonSlash => {
                for j in 0..n {
                    if dp[j] && chars[j] != '/' {
                        next[j + 1] = true;
                    }
                }
            }
            GlobToken::Class { negated, ranges } => {
                for j in 0..n {
                    if dp[j] && chars[j] != '/' {
                        let c = chars[j];
                        let in_range = ranges.iter().any(|&(lo, hi)| c >= lo && c <= hi);
                        if in_range != *negated {
                            next[j + 1] = true;
                        }
                    }
                }
            }
            GlobToken::Star => {
                next.copy_from_slice(&dp);
                for j in 0..n {
                    if next[j] && chars[j] != '/' {
                        next[j + 1] = true;
                    }
                }
            }
            GlobToken::DoubleStar => {
                if let Some(first) = dp.iter().position(|&ok| ok) {
                    for slot in &mut next[first..] {
                        *slot = true;
                    }
                }
            }
            GlobToken::DoubleStarSlash => {
                next.copy_from_slice(&dp);
                if let Some(first) = dp.iter().position(|&ok| ok) {
                    for j in (first + 1)..=n {
                        if chars[j - 1] == '/' {
                            next[j] = true;
                        }
                    }
                }
            }
        }
        dp = next;
        if !dp.iter().any(|&ok| ok) {
            return false;
        }
    }
    dp[n]
}

fn trim_trailing_unescaped_spaces(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b' ' {
        if end >= 2 && bytes[end - 2] == b'\\' {
            break;
        }
        end -= 1;
    }
    &line[..end]
}

fn parse_gitignore(base: PathBuf, text: &str) -> Option<IgnoreSet> {
    let mut rules = Vec::new();
    for raw in text.lines() {
        let line = trim_trailing_unescaped_spaces(raw.trim_end_matches('\r'));
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (negated, rest) = match line.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let rest = rest.strip_prefix("\\#").unwrap_or(rest);
        let (dir_only, rest) = match rest.strip_suffix('/') {
            Some(stripped) => (true, stripped),
            None => (false, rest),
        };
        if rest.is_empty() {
            continue;
        }
        // A slash anywhere in the remaining pattern anchors it to `base`.
        let anchored = rest.contains('/');
        let pattern = rest.strip_prefix('/').unwrap_or(rest);
        if pattern.is_empty() {
            continue;
        }
        rules.push(IgnoreRule {
            negated,
            dir_only,
            anchored,
            tokens: tokenize_glob(pattern),
        });
    }
    (!rules.is_empty()).then_some(IgnoreSet { base, rules })
}

fn load_gitignore(dir: &Path) -> Option<Arc<IgnoreSet>> {
    let path = dir.join(".gitignore");
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_GITIGNORE_BYTES {
        return None;
    }
    let contents = std::fs::read_to_string(&path).ok()?;
    parse_gitignore(dir.to_path_buf(), &contents).map(Arc::new)
}

fn is_ignored(stack: &[Arc<IgnoreSet>], path: &Path, name: &str, is_dir: bool) -> bool {
    let mut ignored = false;
    for set in stack {
        let Ok(rel) = path.strip_prefix(&set.base) else {
            continue;
        };
        let rel_str = rel.to_string_lossy();
        let rel_unix;
        let rel_text: &str = if std::path::MAIN_SEPARATOR == '/' {
            &rel_str
        } else {
            rel_unix = rel_str.replace(std::path::MAIN_SEPARATOR, "/");
            &rel_unix
        };
        for rule in &set.rules {
            if rule.dir_only && !is_dir {
                continue;
            }
            let matched = if rule.anchored {
                glob_matches(&rule.tokens, rel_text)
            } else {
                glob_matches(&rule.tokens, name) || glob_matches(&rule.tokens, rel_text)
            };
            if matched {
                ignored = !rule.negated;
            }
        }
    }
    ignored
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

#[derive(Clone)]
struct DirTask {
    dir: PathBuf,
    depth: usize,
    ignores: Vec<Arc<IgnoreSet>>,
}

fn scan_dir(
    task: &DirTask,
    config: &FilesConfig,
    exclude: &HashSet<&str>,
) -> (Vec<FileEntry>, Vec<DirTask>) {
    let mut ignores = task.ignores.clone();
    if config.respect_gitignore {
        if let Some(local) = load_gitignore(&task.dir) {
            ignores.push(local);
        }
    }

    let Ok(read_dir) = std::fs::read_dir(&task.dir) else {
        return (Vec::new(), Vec::new());
    };

    let mut found = Vec::new();
    let mut next_dirs = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if (!config.include_hidden && name.starts_with('.'))
            || (config.respect_gitignore && name == ".git")
            || exclude.contains(name.as_str())
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
        if config.respect_gitignore
            && !ignores.is_empty()
            && is_ignored(&ignores, &path, &name, is_dir)
        {
            continue;
        }
        // DirEntry::metadata on Unix uses lstat (does not follow symlinks).
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| modified_secs(&m))
            .unwrap_or(0);
        if is_dir && task.depth < config.max_depth {
            next_dirs.push(DirTask {
                dir: path.clone(),
                depth: task.depth + 1,
                ignores: ignores.clone(),
            });
        }
        found.push(FileEntry::new(path, name, is_dir, task.depth, modified));
    }
    (found, next_dirs)
}

/// Walk the configured roots breadth-first, scanning each depth level in
/// parallel across worker threads. Blocking; run it off the main thread.
pub fn build_index(config: &FilesConfig) -> Vec<FileEntry> {
    let exclude: HashSet<&str> = config.exclude.iter().map(String::as_str).collect();
    let workers = std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 8))
        .unwrap_or(4);

    let mut entries = Vec::new();
    let mut current_level: Vec<DirTask> = config
        .roots
        .iter()
        .map(|root| DirTask {
            dir: expand_tilde(root),
            depth: 1,
            ignores: Vec::new(),
        })
        .collect();

    while !current_level.is_empty() && entries.len() < MAX_ENTRIES {
        let level_results: Vec<(Vec<FileEntry>, Vec<DirTask>)> = if current_level.len() >= 4 {
            let chunk_size = current_level.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = current_level
                    .chunks(chunk_size)
                    .map(|chunk| {
                        let exclude = &exclude;
                        scope.spawn(move || {
                            chunk
                                .iter()
                                .map(|task| scan_dir(task, config, exclude))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().unwrap_or_default())
                    .collect()
            })
        } else {
            current_level
                .iter()
                .map(|task| scan_dir(task, config, &exclude))
                .collect()
        };

        let mut next_level = Vec::new();
        for (found, subdirs) in level_results {
            let remaining = MAX_ENTRIES.saturating_sub(entries.len());
            if remaining == 0 {
                return entries;
            }
            entries.extend(found.into_iter().take(remaining));
            if entries.len() >= MAX_ENTRIES {
                return entries;
            }
            next_level.extend(subdirs);
        }
        current_level = next_level;
    }
    entries
}

/// Watch the configured roots and their top-level subdirectories so creating,
/// renaming or removing files marks the in-memory index stale right away.
pub fn watch_directories(
    config: &FilesConfig,
    on_change: impl Fn() + Clone + 'static,
) -> Vec<gio::FileMonitor> {
    if !config.enabled {
        return Vec::new();
    }
    let exclude: HashSet<&str> = config.exclude.iter().map(String::as_str).collect();
    let mut dirs = Vec::new();
    for root in &config.roots {
        let root_path = expand_tilde(root);
        if !root_path.is_dir() {
            continue;
        }
        dirs.push(root_path.clone());
        if config.max_depth > 1 {
            if let Ok(read_dir) = std::fs::read_dir(&root_path) {
                for entry in read_dir.flatten() {
                    if dirs.len() >= MAX_WATCHED_DIRS {
                        break;
                    }
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if (!config.include_hidden && name.starts_with('.'))
                        || exclude.contains(name.as_str())
                    {
                        continue;
                    }
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        dirs.push(entry.path());
                    }
                }
            }
        }
        if dirs.len() >= MAX_WATCHED_DIRS {
            break;
        }
    }

    dirs.into_iter()
        .filter_map(|dir| {
            let monitor = gio::File::for_path(dir)
                .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
                .ok()?;
            let callback = on_change.clone();
            monitor.connect_changed(move |_, _, _, event| {
                use gio::FileMonitorEvent::{Created, Deleted, MovedIn, MovedOut, Renamed};
                if matches!(event, Created | Deleted | Renamed | MovedIn | MovedOut) {
                    callback();
                }
            });
            Some(monitor)
        })
        .collect()
}

/// Bonus for files touched lately: what you are looking for is usually
/// something you worked on recently.
fn recency_bonus(modified: u64, now: u64) -> i64 {
    const DAY: u64 = 86_400;
    if modified == 0 {
        return 0;
    }
    match now.saturating_sub(modified) / DAY {
        0 => 40,
        1..=6 => 25,
        7..=29 => 10,
        _ => 0,
    }
}

/// Rank indexed files by name, then by how recently they changed and how
/// often they were opened from here. Uses the strict tiers only (no fuzzy),
/// which keeps a keystroke over a few hundred thousand names fast.
pub fn search<'a>(
    query: &str,
    entries: &'a [FileEntry],
    usage: &Usage,
    now: u64,
    limit: usize,
) -> Vec<&'a FileEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut results: Vec<(&FileEntry, i64)> = entries
        .iter()
        .filter_map(|entry| {
            Matcher::strict(&query, &entry.name_lower)
                // Prefer files closer to the root among similar matches.
                .map(|score| score - entry.depth as i64 * 3 + recency_bonus(entry.modified, now))
                .map(|score| {
                    // Only look up usage for actual matches.
                    (entry, score + usage.boost(&entry.usage_id()))
                })
        })
        .collect();
    results.sort_by_key(|item| std::cmp::Reverse(item.1));
    results
        .into_iter()
        .take(limit)
        .map(|(entry, _)| entry)
        .collect()
}

/// Splits a partially typed path into the directory to list and the
/// lowercase name prefix to filter it by: `~/Do` is `(~, "do")`;
/// `~/Documents/` is `(~/Documents, "")`.
pub fn split_path_input(input: &str) -> (PathBuf, String) {
    let expanded = expand_tilde(input);
    if input.ends_with('/') || input == "~" {
        return (expanded, String::new());
    }
    let partial = expanded
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let dir = expanded
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    (dir, partial)
}

/// Every entry of `dir`, directories first; empty when it can't be read.
/// Blocks on slow or hung mounts, so it runs off the UI thread.
pub fn list_dir(dir: &Path) -> Vec<FileEntry> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<FileEntry> = read_dir
        .flatten()
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_dir = path.is_dir();
            FileEntry::from_path(path, name, is_dir, 0)
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name_lower.cmp(&b.name_lower))
    });
    entries
}

/// The entries of a directory listing whose names start with `partial`.
/// Hidden entries only show once the prefix starts with a dot.
pub fn filter_listing(listing: &[FileEntry], partial: &str, limit: usize) -> Vec<FileEntry> {
    let show_hidden = partial.starts_with('.');
    listing
        .iter()
        .filter(|e| show_hidden || !e.name.starts_with('.'))
        .filter(|e| e.name_lower.starts_with(partial))
        .take(limit)
        .cloned()
        .collect()
}

#[cfg(test)]
fn complete_path(input: &str, limit: usize) -> Vec<FileEntry> {
    let (dir, partial) = split_path_input(input);
    filter_listing(&list_dir(&dir), &partial, limit)
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

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff", "avif", "heic", "pdf", "zip", "gz",
    "tgz", "bz2", "xz", "zst", "tar", "7z", "rar", "mp3", "mp4", "mkv", "avi", "mov", "webm",
    "ogg", "flac", "wav", "m4a", "ttf", "otf", "woff", "woff2", "so", "o", "a", "rlib", "rmeta",
    "wasm", "class", "jar", "pyc", "pyo", "sqlite", "sqlite3", "db", "bin", "iso", "img", "exe",
    "dll", "deb", "rpm",
];

fn has_binary_extension(name_lower: &str) -> bool {
    let Some((_, ext)) = name_lower.rsplit_once('.') else {
        return false;
    };
    BINARY_EXTENSIONS.contains(&ext)
}

fn contains_case_insensitive(haystack: &str, needle_lower: &str, ascii_only: bool) -> bool {
    if ascii_only && haystack.is_ascii() {
        let h = haystack.as_bytes();
        let n = needle_lower.as_bytes();
        if n.len() > h.len() {
            return false;
        }
        return h
            .windows(n.len())
            .any(|w| w.iter().zip(n).all(|(a, b)| a.to_ascii_lowercase() == *b));
    }
    haystack.to_lowercase().contains(needle_lower)
}

fn trim_preview(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() > 110 {
        let cut: String = trimmed.chars().take(109).collect();
        format!("{}…", cut)
    } else {
        trimmed.to_string()
    }
}

fn scan_file_contents(
    entry: &FileEntry,
    needle_lower: &str,
    ascii_only: bool,
    buf: &mut Vec<u8>,
) -> Vec<(usize, String)> {
    let Ok(mut file) = std::fs::File::open(&entry.path) else {
        return Vec::new();
    };
    let Ok(meta) = file.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    if len == 0 || len > MAX_GREP_BYTES || !meta.is_file() {
        return Vec::new();
    }
    buf.clear();
    if file.read_to_end(buf).is_err() {
        return Vec::new();
    }
    if buf[..buf.len().min(1024)].contains(&0) {
        return Vec::new();
    }
    let Ok(text) = std::str::from_utf8(buf) else {
        return Vec::new();
    };

    let mut hits = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if contains_case_insensitive(line, needle_lower, ascii_only) {
            hits.push((idx + 1, trim_preview(line)));
            if hits.len() >= 2 {
                break;
            }
        }
    }
    hits
}

/// Search inside indexed text files on worker threads. Returns `None` if a
/// newer generation arrived on `cancel_gen` while scanning.
pub fn search_contents(
    query: &str,
    entries: &[FileEntry],
    now: u64,
    cancel_gen: &AtomicU64,
    my_gen: u64,
    limit: usize,
) -> Option<Vec<ContentMatch>> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Some(Vec::new());
    }
    let ascii_only = needle.is_ascii();

    let mut candidates: Vec<(&FileEntry, i64)> = entries
        .iter()
        .filter(|e| !e.is_dir && !has_binary_extension(&e.name_lower))
        .map(|e| (e, recency_bonus(e.modified, now) - e.depth as i64 * 3))
        .collect();
    candidates.sort_by_key(|item| std::cmp::Reverse(item.1));
    candidates.truncate(MAX_GREP_FILES);

    if cancel_gen.load(Ordering::Relaxed) != my_gen {
        return None;
    }

    let workers = std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 8))
        .unwrap_or(4);
    let chunk_size = candidates.len().div_ceil(workers).max(1);

    let mut scored_matches: Vec<(ContentMatch, i64)> = std::thread::scope(|scope| {
        let handles: Vec<_> = candidates
            .chunks(chunk_size)
            .map(|chunk| {
                let needle = needle.as_str();
                scope.spawn(move || {
                    let mut buf = Vec::with_capacity(16 * 1024);
                    let mut out = Vec::new();
                    for &(entry, priority) in chunk {
                        if cancel_gen.load(Ordering::Relaxed) != my_gen {
                            return None;
                        }
                        for (line_number, line_preview) in
                            scan_file_contents(entry, needle, ascii_only, &mut buf)
                        {
                            out.push((
                                ContentMatch {
                                    file: entry.clone(),
                                    line_number,
                                    line_preview,
                                },
                                priority,
                            ));
                        }
                    }
                    Some(out)
                })
            })
            .collect();

        let mut combined = Vec::new();
        for handle in handles {
            match handle.join().ok().flatten() {
                Some(part) => combined.extend(part),
                None => return None,
            }
        }
        Some(combined)
    })?;

    scored_matches.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.file.name_lower.cmp(&b.0.file.name_lower))
            .then_with(|| a.0.line_number.cmp(&b.0.line_number))
    });
    scored_matches.truncate(limit);
    Some(scored_matches.into_iter().map(|(m, _)| m).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("poppi-files-{}-{}", tag, std::process::id()));
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
        assert!(build_index(&config)
            .iter()
            .any(|e| e.name == "report.secret"));
    }

    #[test]
    fn search_ranks_prefix_first_and_supports_words() {
        let tree = TempTree::new("search");
        let index = build_index(&tree.config());

        let reports = names(&search("report", &index, &Usage::load_from(None), 0, 10));
        assert_eq!(reports[0], "reports");
        assert!(reports.contains(&"report-2026.pdf".to_string()));
        assert!(reports.contains(&"annual report.txt".to_string()));

        assert_eq!(
            names(&search("ann rep", &index, &Usage::load_from(None), 0, 10)),
            vec!["annual report.txt"]
        );
        assert_eq!(
            names(&search("NOTES", &index, &Usage::load_from(None), 0, 10)),
            vec!["notes.md"]
        );
        assert!(search("zzzz", &index, &Usage::load_from(None), 0, 10).is_empty());
        assert_eq!(
            search("report", &index, &Usage::load_from(None), 0, 1).len(),
            1
        );
    }

    #[test]
    fn recent_and_often_opened_files_rank_higher() {
        const DAY: u64 = 86_400;
        let now = 1_000 * DAY;
        let entry = |name: &str, modified: u64| FileEntry {
            path: PathBuf::from(format!("/x/{}", name)),
            name: name.to_string(),
            is_dir: false,
            modified,
            name_lower: name.to_lowercase(),
            depth: 1,
        };
        let index = vec![
            entry("report-old.txt", now - 400 * DAY),
            entry("report-new.txt", now - 2 * DAY),
        ];
        let mut usage = Usage::load_from(None);
        assert_eq!(
            names(&search("report", &index, &usage, now, 10))[0],
            "report-new.txt"
        );

        // Opening the old one repeatedly outweighs the newer file's recency.
        for _ in 0..20 {
            usage.record(&index[0].usage_id());
        }
        assert_eq!(
            names(&search("report", &index, &usage, now, 10))[0],
            "report-old.txt"
        );

        assert_eq!(recency_bonus(0, now), 0);
        assert!(recency_bonus(now, now) > recency_bonus(now - 3 * DAY, now));
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

    #[test]
    fn gitignore_rules_and_parallel_levels() {
        let root =
            std::env::temp_dir().join(format!("poppi-files-gitignore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for sub in ["a", "b", "c", "d", "e", "dist"] {
            fs::create_dir_all(root.join(sub)).unwrap();
        }
        fs::write(
            root.join(".gitignore"),
            "# comment\n*.log\n!keep.log\ndist/\n/root-only.tmp\n**/generated/*.out\n",
        )
        .unwrap();
        fs::write(root.join("debug.log"), "").unwrap();
        fs::write(root.join("keep.log"), "").unwrap();
        fs::write(root.join("root-only.tmp"), "").unwrap();
        fs::write(root.join("a/root-only.tmp"), "").unwrap();
        fs::write(root.join("dist/bundle.js"), "").unwrap();
        fs::create_dir_all(root.join("b/generated")).unwrap();
        fs::write(root.join("b/generated/schema.out"), "").unwrap();
        fs::write(root.join("c/.gitignore"), "*.bak\n").unwrap();
        fs::write(root.join("c/old.bak"), "").unwrap();
        fs::write(root.join("d/old.bak"), "").unwrap();
        fs::write(root.join("e/readme.md"), "").unwrap();

        let mut config = FilesConfig {
            roots: vec![root.to_string_lossy().into_owned()],
            ..FilesConfig::default()
        };
        let index = build_index(&config);
        let paths: Vec<String> = index
            .iter()
            .filter_map(|e| e.path.strip_prefix(&root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        assert!(paths.contains(&"keep.log".to_string()));
        assert!(paths.contains(&"a/root-only.tmp".to_string()));
        assert!(paths.contains(&"d/old.bak".to_string()));
        assert!(paths.contains(&"e/readme.md".to_string()));
        assert!(!paths.contains(&"debug.log".to_string()));
        assert!(!paths.contains(&"root-only.tmp".to_string()));
        assert!(!paths.contains(&"dist".to_string()));
        assert!(!paths.contains(&"dist/bundle.js".to_string()));
        assert!(!paths.contains(&"b/generated/schema.out".to_string()));
        assert!(!paths.contains(&"c/old.bak".to_string()));

        config.respect_gitignore = false;
        let unfiltered = build_index(&config);
        assert!(unfiltered.iter().any(|e| e.name == "bundle.js"));
        assert!(unfiltered.iter().any(|e| e.name == "debug.log"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn content_search_finds_lines_skips_binaries_and_cancels() {
        let root = std::env::temp_dir().join(format!("poppi-files-grep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("notes.md"),
            "first line\nRemember the SECRET_TOKEN here\nlast line\n",
        )
        .unwrap();
        fs::write(root.join("binary.dat"), b"not text\0SECRET_TOKEN\n").unwrap();

        let config = FilesConfig {
            roots: vec![root.to_string_lossy().into_owned()],
            ..FilesConfig::default()
        };
        let index = build_index(&config);
        let gen = AtomicU64::new(1);

        let hits = search_contents("secret_token", &index, 0, &gen, 1, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file.name, "notes.md");
        assert_eq!(hits[0].line_number, 2);
        assert!(hits[0].line_preview.contains("SECRET_TOKEN"));

        // Advancing the generation cancels the scan.
        gen.store(2, Ordering::Relaxed);
        assert!(search_contents("secret_token", &index, 0, &gen, 1, 10).is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
