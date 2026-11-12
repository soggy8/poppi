use crate::matching::Matcher;
use crate::usage::Usage;
use gtk::gio;
use gtk::prelude::*;

#[derive(Debug, Clone)]
pub struct App {
    /// Desktop file id, e.g. `org.mozilla.firefox.desktop`.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<gio::Icon>,
    /// Set for an extra action of an app ("New Private Window"); the name
    /// is then the action's and the description names the app.
    action: Option<String>,
    info: gio::AppInfo,
    name_lower: String,
    description_lower: Option<String>,
    keywords_lower: Vec<String>,
    /// Lowercase names a window of this app may report as its class.
    window_classes: Vec<String>,
}

pub struct AppLauncher {
    apps: Vec<App>,
    matcher: Matcher,
}

/// Score one app against a lowercase query. The name counts fully; keywords
/// rank below any comparable name match; the description only counts for
/// whole-word or substring hits and ranks last.
pub fn match_score(
    matcher: &Matcher,
    query: &str,
    name: &str,
    description: Option<&str>,
    keywords: &[String],
) -> Option<i64> {
    let name_score = matcher.score(query, name);
    let keyword_score = keywords
        .iter()
        .filter_map(|k| matcher.score(query, k))
        .max()
        .map(|s| s - 150);
    let description_score = if query.chars().count() >= 3 {
        description
            .and_then(|d| Matcher::strict(query, d))
            .map(|s| s - 450)
    } else {
        None
    };

    [name_score, keyword_score, description_score]
        .into_iter()
        .flatten()
        .max()
        .filter(|score| *score > 0)
}

/// `org.gnome.Nautilus.desktop` -> [`org.gnome.nautilus`, `nautilus`].
fn classes_from_id(id: &str) -> Vec<String> {
    let stem = id.strip_suffix(".desktop").unwrap_or(id).to_lowercase();
    let mut classes = vec![stem.clone()];
    if let Some(last) = stem.rsplit('.').next() {
        if last != stem {
            classes.push(last.to_string());
        }
    }
    classes
}

impl AppLauncher {
    pub fn empty() -> Self {
        Self {
            apps: Vec::new(),
            matcher: Matcher::new(),
        }
    }

    /// Read installed applications through GIO, which follows `XDG_DATA_DIRS`
    /// (so Flatpak and Snap exports are included), applies user overrides,
    /// localizes names and honours NoDisplay/Hidden/OnlyShowIn.
    /// Must run on the main thread: `gio::AppInfo` is not `Send`.
    pub fn load() -> Self {
        let mut actions: Vec<App> = Vec::new();
        let mut apps: Vec<App> = gio::AppInfo::all()
            .into_iter()
            .filter(|info| info.should_show())
            .map(|info| {
                let name = info.name().to_string();
                let id = info
                    .id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| name.clone());
                let description = info
                    .description()
                    .map(|d| d.to_string())
                    .filter(|d| !d.is_empty());

                let mut keywords_lower = Vec::new();
                let mut window_classes = classes_from_id(&id);
                if let Some(desktop) = info.downcast_ref::<gio::DesktopAppInfo>() {
                    keywords_lower.extend(desktop.keywords().iter().map(|k| k.to_lowercase()));
                    if let Some(generic) = desktop.generic_name() {
                        keywords_lower.push(generic.to_lowercase());
                    }
                    if let Some(class) = desktop.startup_wm_class() {
                        window_classes.push(class.to_lowercase());
                    }
                    // Desktop actions become their own, lower-ranked entries.
                    for action in desktop.list_actions() {
                        let label = desktop.action_name(&action).to_string();
                        if label.is_empty() {
                            continue;
                        }
                        actions.push(App {
                            id: format!("{}::{}", id, action),
                            name_lower: label.to_lowercase(),
                            description_lower: None,
                            // Also found as "firefox private".
                            keywords_lower: vec![format!("{} {}", name, label).to_lowercase()],
                            window_classes: Vec::new(),
                            icon: info.icon(),
                            action: Some(action.to_string()),
                            description: Some(name.clone()),
                            name: label,
                            info: info.clone(),
                        });
                    }
                }

                App {
                    name_lower: name.to_lowercase(),
                    description_lower: description.as_ref().map(|d| d.to_lowercase()),
                    keywords_lower,
                    window_classes,
                    icon: info.icon(),
                    action: None,
                    id,
                    name,
                    description,
                    info,
                }
            })
            .collect();

        apps.append(&mut actions);
        apps.sort_by(|a, b| a.name_lower.cmp(&b.name_lower));
        Self {
            apps,
            matcher: Matcher::new(),
        }
    }

    pub fn search(&self, query: &str, usage: &Usage, limit: usize) -> Vec<&App> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<(&App, i64)> = self
            .apps
            .iter()
            .filter_map(|app| {
                match_score(
                    &self.matcher,
                    &query,
                    &app.name_lower,
                    app.description_lower.as_deref(),
                    &app.keywords_lower,
                )
                // An action never outranks a comparable match on an app.
                .map(|score| score - if app.action.is_some() { 120 } else { 0 })
                .filter(|score| *score > 0)
                .map(|score| (app, score + usage.boost(&app.id)))
            })
            .collect();

        // Stable sort: equal scores stay in alphabetical order.
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results
            .into_iter()
            .take(limit)
            .map(|(app, _)| app)
            .collect()
    }

    /// Extra desktop actions belonging to `app` (e.g. "New Private Window").
    pub fn actions_for(&self, app: &App) -> Vec<App> {
        if app.action.is_some() {
            return Vec::new();
        }
        let prefix = format!("{}::", app.id);
        self.apps
            .iter()
            .filter(|entry| entry.id.starts_with(&prefix))
            .cloned()
            .collect()
    }

    /// Apps launched from here, ranked by how often and how recently.
    pub fn most_used(&self, usage: &Usage, limit: usize) -> Vec<&App> {
        let mut used: Vec<(&App, f64)> = self
            .apps
            .iter()
            .map(|app| (app, usage.frecency(&app.id)))
            .filter(|(_, frecency)| *frecency > 0.0)
            .collect();
        used.sort_by(|a, b| b.1.total_cmp(&a.1));
        used.into_iter().take(limit).map(|(app, _)| app).collect()
    }

    /// The icon of the app a window belongs to, found through the window
    /// class (`dev.zed.Zed`, or `instance.Class` on X11).
    pub fn icon_for_window_class(&self, class: &str) -> Option<gio::Icon> {
        let class = class.to_lowercase();
        if class.is_empty() {
            return None;
        }
        let find = |candidate: &str| {
            self.apps
                .iter()
                .find(|app| app.window_classes.iter().any(|c| c == candidate))
                .and_then(|app| app.icon.clone())
        };
        find(&class).or_else(|| {
            class
                .rsplit('.')
                .filter(|segment| !segment.is_empty())
                .find_map(find)
        })
    }

    pub fn launch(app: &App, context: Option<&gio::AppLaunchContext>) -> Result<(), String> {
        if let Some(action) = &app.action {
            let desktop = app
                .info
                .downcast_ref::<gio::DesktopAppInfo>()
                .ok_or("Not a desktop application")?;
            desktop.launch_action(action, context);
            return Ok(());
        }
        app.info
            .launch(&[], context)
            .map_err(|e| format!("Could not launch {}: {}", app.name, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, name: &str, description: Option<&str>, keywords: &[&str]) -> Option<i64> {
        let keywords: Vec<String> = keywords.iter().map(|k| k.to_string()).collect();
        match_score(&Matcher::new(), query, name, description, &keywords)
    }

    #[test]
    fn name_beats_keyword_beats_description() {
        let by_name = score("fire", "firefox", None, &[]).unwrap();
        let by_keyword = score("fire", "some app", None, &["firewall"]).unwrap();
        let by_description = score("fire", "some app", Some("firefox helper"), &[]).unwrap();
        assert!(by_name > by_keyword);
        assert!(by_keyword > by_description);
    }

    #[test]
    fn keywords_match() {
        assert!(score("browser", "firefox", None, &["web", "browser"]).is_some());
        assert!(score("browser", "firefox", None, &[]).is_none());
    }

    #[test]
    fn descriptions_do_not_fuzzy_match() {
        // The old matcher listed LibreOffice Impress for "fire" because the
        // letters appear, in order, somewhere in its description.
        assert!(score(
            "fire",
            "libreoffice impress",
            Some("create and edit presentations for slideshows, meetings and web pages."),
            &[]
        )
        .is_none());
        assert!(score(
            "slides",
            "libreoffice impress",
            Some("create slideshows and slides"),
            &[]
        )
        .is_some());
    }

    #[test]
    fn no_match_is_none() {
        assert!(score("zzz", "firefox", Some("web browser"), &["internet"]).is_none());
    }

    #[test]
    fn window_classes_from_ids() {
        assert_eq!(
            classes_from_id("org.gnome.Nautilus.desktop"),
            vec!["org.gnome.nautilus", "nautilus"]
        );
        assert_eq!(classes_from_id("kitty.desktop"), vec!["kitty"]);
    }
}
