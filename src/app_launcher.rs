use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::path::PathBuf;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    pub name_lower: String, // Pre-computed lowercase for faster search
    pub exec: String,
    pub icon: Option<String>,
    pub comment: Option<String>,
    pub comment_lower: Option<String>, // Pre-computed lowercase for faster search
    pub desktop_file: PathBuf,
}

pub struct AppLauncher {
    apps: Vec<App>,
    matcher: SkimMatcherV2,
}

fn parse_desktop_file(path: &std::path::Path) -> Option<HashMap<String, String>> {
    // Use mmap for faster file reading if possible, otherwise use standard read
    let content = std::fs::read_to_string(path).ok()?;
    let mut section = HashMap::with_capacity(16); // Pre-allocate capacity
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let line = line.trim();
        // Fast path: skip empty lines and comments early
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        // Use split_once for better performance
        if let Some((key, value)) = line.split_once('=') {
            section.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    Some(section)
}

impl AppLauncher {
    /// Create an empty AppLauncher (for lazy loading)
    pub fn empty() -> Self {
        Self {
            apps: Vec::new(),
            matcher: SkimMatcherV2::default(),
        }
    }

    pub fn apps(&self) -> &[App] {
        &self.apps
    }

    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut apps = Vec::with_capacity(200); // Pre-allocate for typical number of apps
        let matcher = SkimMatcherV2::default();

        // Common desktop entry paths
        let paths = vec![
            PathBuf::from("/usr/share/applications"),
            PathBuf::from("/usr/local/share/applications"),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".local/share/applications"),
        ];

        for path in paths {
            if let Ok(entries) = std::fs::read_dir(&path) {
                // Collect entries first to avoid holding file handles
                let entries: Vec<_> = entries.flatten().collect();
                for entry in entries {
                    let file_path = entry.path();
                    if file_path.extension().and_then(|s| s.to_str()) == Some("desktop") {
                        if let Some(section) = parse_desktop_file(&file_path) {
                            // Skip NoDisplay entries (fast check first)
                            if section.get("NoDisplay")
                                .map(|v| v == "true")
                                .unwrap_or(false) {
                                continue;
                            }

                            // Skip hidden entries
                            if section.get("Hidden")
                                .map(|v| v == "true")
                                .unwrap_or(false) {
                                continue;
                            }

                            let name = section.get("Name")
                                .or_else(|| section.get("GenericName"))
                                .map(|s| s.as_str())
                                .unwrap_or_else(|| {
                                    file_path.file_stem()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("Unknown")
                                })
                                .to_string();

                            if let Some(exec) = section.get("Exec") {
                                let comment = section.get("Comment").map(|s| s.as_str());
                                apps.push(App {
                                    name: name.clone(),
                                    name_lower: name.to_lowercase(), // Pre-compute lowercase
                                    exec: exec.clone(),
                                    icon: section.get("Icon").map(|s| s.as_str()).map(|s| s.to_string()),
                                    comment: comment.map(|s| s.to_string()),
                                    comment_lower: comment.map(|s| s.to_lowercase()), // Pre-compute lowercase
                                    desktop_file: file_path,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Sort apps by name for better cache locality
        apps.sort_unstable_by(|a, b| a.name_lower.cmp(&b.name_lower));

        Ok(Self { apps, matcher })
    }

    pub fn search(&self, query: &str) -> Vec<(&App, i64)> {
        if query.is_empty() {
            return self
                .apps
                .iter()
                .map(|app| (app, 0))
                .take(20)
                .collect();
        }

        let query_lower = query.to_lowercase(); // Compute once
        let mut results: Vec<(&App, i64)> = Vec::with_capacity(20); // Pre-allocate

        for app in &self.apps {
            // Use pre-computed lowercase names - no allocation
            let name_score = self.matcher.fuzzy_match(&app.name_lower, &query_lower);
            
            // Use pre-computed lowercase comments
            let comment_score = app.comment_lower.as_ref().and_then(|c| {
                self.matcher.fuzzy_match(c, &query_lower)
            });

            let score = name_score
                .unwrap_or(0)
                .max(comment_score.unwrap_or(0));

            if score > 0 {
                results.push((app, score));
            }
        }

        // Sort by score (higher is better) - use unstable sort for speed
        results.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        results.truncate(20);
        results
    }

    pub fn launch(&self, app: &App) -> Result<(), Box<dyn std::error::Error>> {
        use std::process::Command;

        // Use gio launch for proper desktop entry execution (fastest method)
        if Command::new("gio")
            .arg("launch")
            .arg(&app.desktop_file)
            .spawn()
            .is_ok() {
            return Ok(());
        }

        // Fallback: direct execution (avoid string allocations)
        let exec = &app.exec;
        let parts: Vec<&str> = exec.split_whitespace().take(1).collect();
        if !parts.is_empty() {
            Command::new(parts[0])
                .args(exec.split_whitespace().skip(1))
                .spawn()?;
        }

        Ok(())
    }
}

impl Default for AppLauncher {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| Self {
            apps: Vec::new(),
            matcher: SkimMatcherV2::default(),
        })
    }
}
