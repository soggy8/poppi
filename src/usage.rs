//! Launch counts per application, used to rank frequently used apps higher.

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct Usage {
    counts: HashMap<String, u32>,
    path: Option<PathBuf>,
}

impl Usage {
    fn default_path() -> Option<PathBuf> {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|dir| dir.join("poppi_launcher").join("usage.json"))
    }

    pub fn load() -> Self {
        Self::load_from(Self::default_path())
    }

    pub fn load_from(path: Option<PathBuf>) -> Self {
        let counts = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { counts, path }
    }

    pub fn count(&self, id: &str) -> u32 {
        self.counts.get(id).copied().unwrap_or(0)
    }

    /// Score bonus for an app. Logarithmic, so a favourite wins ties and near
    /// ties without burying a clearly better text match.
    pub fn boost(&self, id: &str) -> i64 {
        let count = self.count(id);
        if count == 0 {
            0
        } else {
            ((f64::from(count) + 1.0).ln() * 10.0).min(50.0) as i64
        }
    }

    pub fn record(&mut self, id: &str) {
        *self.counts.entry(id.to_string()).or_insert(0) += 1;
        if let Err(e) = self.save() {
            eprintln!("Could not save usage data: {}", e);
        }
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string(&self.counts).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boost_grows_with_use_and_is_capped() {
        let mut usage = Usage::load_from(None);
        assert_eq!(usage.boost("a"), 0);
        usage.record("a");
        let once = usage.boost("a");
        assert!(once > 0);
        for _ in 0..10 {
            usage.record("a");
        }
        assert!(usage.boost("a") > once);
        for _ in 0..10_000 {
            *usage.counts.get_mut("a").unwrap() += 1;
        }
        assert_eq!(usage.boost("a"), 50);
    }

    #[test]
    fn counts_persist() {
        let path = std::env::temp_dir().join(format!("poppi-usage-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut usage = Usage::load_from(Some(path.clone()));
        usage.record("firefox.desktop");
        usage.record("firefox.desktop");

        let reloaded = Usage::load_from(Some(path.clone()));
        assert_eq!(reloaded.count("firefox.desktop"), 2);
        assert_eq!(reloaded.count("other.desktop"), 0);

        let _ = std::fs::remove_file(&path);
    }
}
