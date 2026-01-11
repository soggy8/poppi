use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::path::PathBuf;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct App {
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub comment: Option<String>,
    pub desktop_file: PathBuf,
}

pub struct AppLauncher {
    apps: Vec<App>,
    matcher: SkimMatcherV2,
}

fn parse_desktop_file(path: &std::path::Path) -> Option<HashMap<String, String>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut section = HashMap::new();
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        if let Some(pos) = line.find('=') {
            let key = line[..pos].trim().to_string();
            let value = line[pos + 1..].trim().to_string();
            section.insert(key, value);
        }
    }
    Some(section)
}

impl AppLauncher {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut apps = Vec::new();
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
                for entry in entries.flatten() {
                    let file_path = entry.path();
                    if file_path.extension().and_then(|s| s.to_str()) == Some("desktop") {
                        if let Some(section) = parse_desktop_file(&file_path) {
                            // Skip NoDisplay entries
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
                                .map(|s| s.clone())
                                .unwrap_or_else(|| {
                                    file_path.file_stem()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("Unknown")
                                        .to_string()
                                });

                            if let Some(exec) = section.get("Exec") {
                                apps.push(App {
                                    name: name.clone(),
                                    exec: exec.clone(),
                                    icon: section.get("Icon").cloned(),
                                    comment: section.get("Comment").cloned(),
                                    desktop_file: file_path.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }

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

        let mut results: Vec<(&App, i64)> = self
            .apps
            .iter()
            .filter_map(|app| {
                // Try matching against name first
                let name_score = self.matcher.fuzzy_match(&app.name.to_lowercase(), &query.to_lowercase());
                
                // Also try matching against comment/description
                let comment_score = app.comment.as_ref().and_then(|c| {
                    self.matcher.fuzzy_match(&c.to_lowercase(), &query.to_lowercase())
                });

                let score = name_score
                    .unwrap_or(0)
                    .max(comment_score.unwrap_or(0));

                if score > 0 {
                    Some((app, score))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score (higher is better)
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.truncate(20);
        results
    }

    pub fn launch(&self, app: &App) -> Result<(), Box<dyn std::error::Error>> {
        use std::process::Command;

        // Parse the exec command - desktop entry spec allows % codes
        let exec = &app.exec;
        
        // Remove desktop entry % codes for now (simplified)
        // In production, you'd want to properly handle %f, %F, %u, %U, etc.
        let command = exec
            .replace("%f", "")
            .replace("%F", "")
            .replace("%u", "")
            .replace("%U", "")
            .replace("%i", "")
            .replace("%c", "")
            .replace("%k", "")
            .trim()
            .to_string();

        // Split command into parts
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Err("Empty exec command".into());
        }

        // Use gio launch for proper desktop entry execution
        // Fallback to direct execution if gio is not available
        if Command::new("gio")
            .arg("launch")
            .arg(&app.desktop_file)
            .spawn()
            .is_err() {
            // Fallback: direct execution
            Command::new(parts[0])
                .args(&parts[1..])
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
