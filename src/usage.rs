//! How often and how recently things were launched, used for ranking.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DAY: u64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Record {
    count: u32,
    /// Unix time of the last launch; 0 when unknown.
    last: u64,
}

/// Older files stored a bare launch count per id.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Record(Record),
    Count(u32),
}

#[derive(Debug, Default)]
pub struct Usage {
    records: HashMap<String, Record>,
    path: Option<PathBuf>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Recent use counts fully; old habits fade but never vanish.
fn recency_weight(age_secs: u64) -> f64 {
    match age_secs / DAY {
        0..=3 => 1.0,
        4..=14 => 0.8,
        15..=60 => 0.5,
        _ => 0.3,
    }
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
        let stored: HashMap<String, Stored> = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let records = stored
            .into_iter()
            .map(|(id, stored)| {
                let record = match stored {
                    Stored::Record(record) => record,
                    Stored::Count(count) => Record { count, last: 0 },
                };
                (id, record)
            })
            .collect();
        Self { records, path }
    }

    #[cfg(test)]
    pub fn count(&self, id: &str) -> u32 {
        self.records.get(id).map(|r| r.count).unwrap_or(0)
    }

    /// Frecency: logarithmic in the number of launches, scaled down as the
    /// last launch recedes. 0 for things never launched.
    pub fn frecency_at(&self, id: &str, now: u64) -> f64 {
        match self.records.get(id) {
            Some(record) if record.count > 0 => {
                (f64::from(record.count) + 1.0).ln()
                    * recency_weight(now.saturating_sub(record.last))
            }
            _ => 0.0,
        }
    }

    pub fn frecency(&self, id: &str) -> f64 {
        self.frecency_at(id, now())
    }

    /// Score bonus: enough to win ties and near ties, never enough to bury a
    /// clearly better text match.
    pub fn boost(&self, id: &str) -> i64 {
        (self.frecency(id) * 10.0).min(50.0) as i64
    }

    pub fn record(&mut self, id: &str) {
        let record = self
            .records
            .entry(id.to_string())
            .or_insert(Record { count: 0, last: 0 });
        record.count += 1;
        record.last = now();
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
        let text = serde_json::to_string(&self.records).map_err(|e| e.to_string())?;
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
        usage.records.get_mut("a").unwrap().count = 1_000_000;
        assert_eq!(usage.boost("a"), 50);
    }

    #[test]
    fn recent_use_outranks_old_habits() {
        let mut usage = Usage::load_from(None);
        let now = 1_000 * DAY;
        usage.records.insert(
            "old".into(),
            Record {
                count: 40,
                last: now - 200 * DAY,
            },
        );
        usage.records.insert(
            "new".into(),
            Record {
                count: 8,
                last: now - DAY,
            },
        );
        assert!(usage.frecency_at("new", now) > usage.frecency_at("old", now));
        // Still remembered, just weaker.
        assert!(usage.frecency_at("old", now) > 0.0);
        assert_eq!(usage.frecency_at("never", now), 0.0);
    }

    #[test]
    fn counts_persist_and_old_format_loads() {
        let path =
            std::env::temp_dir().join(format!("poppi-usage-test-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"legacy.desktop": 7}"#).unwrap();

        let mut usage = Usage::load_from(Some(path.clone()));
        assert_eq!(usage.count("legacy.desktop"), 7);
        usage.record("firefox.desktop");
        usage.record("firefox.desktop");

        let reloaded = Usage::load_from(Some(path.clone()));
        assert_eq!(reloaded.count("firefox.desktop"), 2);
        assert_eq!(reloaded.count("legacy.desktop"), 7);
        assert!(reloaded.frecency("firefox.desktop") > 0.0);
        let _ = std::fs::remove_file(&path);
    }
}
