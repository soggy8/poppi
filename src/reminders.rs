//! Reminders ("remind 10m tea", "remind me at 15:30 to call Ana") and
//! notes ("note buy milk"). Reminders are saved, so ones that come due
//! while Poppi is not running are shown, as missed, when it starts.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Local time as the parser needs it: Unix seconds and the time of day.
#[derive(Debug, Clone, Copy)]
pub struct Now {
    pub unix: i64,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Now {
    pub fn local() -> Self {
        let now = gtk::glib::DateTime::now_local().ok();
        Self {
            unix: now.as_ref().map(|d| d.to_unix()).unwrap_or(0),
            hour: now.as_ref().map(|d| d.hour() as u32).unwrap_or(0),
            minute: now.as_ref().map(|d| d.minute() as u32).unwrap_or(0),
            second: now.as_ref().map(|d| d.second() as u32).unwrap_or(0),
        }
    }
}

/// Seconds in a unit word: "10 minutes", "2h", "an hour".
fn unit_seconds(unit: &str) -> Option<i64> {
    Some(match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        _ => return None,
    })
}

/// "1h30m", "10m", "90s" as one token.
fn compact_duration(token: &str) -> Option<i64> {
    let mut total = 0;
    let mut number = String::new();
    let mut unit = String::new();
    let mut parts = 0;
    for c in token.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            if !unit.is_empty() {
                total += number.parse::<i64>().ok()? * unit_seconds(&unit)?;
                number.clear();
                unit.clear();
                parts += 1;
            }
            number.push(c);
        } else if c.is_ascii_alphabetic() {
            unit.push(c);
        } else if !number.is_empty() && !unit.is_empty() {
            total += number.parse::<i64>().ok()? * unit_seconds(&unit)?;
            parts += 1;
        } else {
            return None;
        }
    }
    (parts > 0).then_some(total)
}

/// A duration at the start of `words`: "10 minutes", "1h30m", "an hour and
/// 30 minutes". Returns seconds and how many words it took.
fn duration(words: &[String]) -> Option<(i64, usize)> {
    let mut total = 0;
    let mut used = 0;
    loop {
        let rest = &words[used..];
        if used > 0 && rest.first().is_some_and(|w| w == "and") {
            if let Some((more, n)) = duration(&rest[1..]) {
                return Some((total + more, used + 1 + n));
            }
            break;
        }
        if let Some(seconds) = rest.first().and_then(|w| compact_duration(w)) {
            total += seconds;
            used += 1;
            continue;
        }
        let amount = match rest.first().map(String::as_str) {
            Some("a" | "an" | "one") => Some(1),
            Some("half") if rest.get(1).is_some_and(|w| w == "an" || w == "a") => {
                // "half an hour"
                if rest.get(2).is_some_and(|w| w == "hour") {
                    total += 1800;
                    used += 3;
                    continue;
                }
                None
            }
            Some(number) => number.parse::<i64>().ok(),
            None => None,
        };
        let unit = rest.get(1).and_then(|u| unit_seconds(u));
        match (amount, unit) {
            (Some(amount), Some(unit)) => {
                total += amount * unit;
                used += 2;
            }
            _ => break,
        }
    }
    (used > 0).then_some((total, used))
}

/// A time of day at the start of `words`: "15:30", "3pm", "3:15 pm", "noon".
/// Returns seconds since midnight and how many words it took.
fn time_of_day(words: &[String]) -> Option<(i64, usize)> {
    let first = words.first()?;
    if first == "noon" {
        return Some((12 * 3600, 1));
    }
    if first == "midnight" {
        return Some((0, 1));
    }
    let (clock, suffix_attached) = match first.strip_suffix("am").or(first.strip_suffix("pm")) {
        Some(clock) => (clock, Some(&first[clock.len()..])),
        None => (first.as_str(), None),
    };
    let (hour, minute) = match clock.split_once(':').or(clock.split_once('.')) {
        Some((h, m)) => (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?),
        None => (clock.parse::<i64>().ok()?, 0),
    };
    let separate = words
        .get(1)
        .map(String::as_str)
        .filter(|w| *w == "am" || *w == "pm");
    let suffix = suffix_attached.or(separate);
    // A bare "at 5" without am/pm or minutes is too vague.
    if suffix.is_none() && !clock.contains([':', '.']) {
        return None;
    }
    let hour = match suffix {
        Some("pm") if hour < 12 => hour + 12,
        Some("am") if hour == 12 => 0,
        _ => hour,
    };
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    let used = if suffix_attached.is_none() && separate.is_some() {
        2
    } else {
        1
    };
    Some((hour * 3600 + minute * 60, used))
}

/// When, from words like "in 10 minutes" / "10m" / "at 15:30", as Unix
/// time, and how many words it took.
fn when(words: &[String], now: Now) -> Option<(i64, usize)> {
    let (skip, at) = match words.first().map(String::as_str) {
        Some("in") | Some("after") => (1, false),
        Some("at") => (1, true),
        _ => (0, false),
    };
    let rest = &words[skip..];
    if !at {
        if let Some((seconds, used)) = duration(rest) {
            return (seconds > 0).then_some((now.unix + seconds, skip + used));
        }
    }
    let (time, used) = time_of_day(rest)?;
    let since_midnight = i64::from(now.hour * 3600 + now.minute * 60 + now.second);
    let mut target = now.unix - since_midnight + time;
    if target <= now.unix {
        target += 86_400;
    }
    Some((target, skip + used))
}

/// Filler between the time and the message: "to", "that", ",".
fn trim_filler(words: &[&str]) -> String {
    let mut words = words.to_vec();
    while words.first().is_some_and(|w| {
        ["to", "that", "about", ",", "-", ":"].contains(&w.to_lowercase().as_str())
    }) {
        words.remove(0);
    }
    words.join(" ").trim_matches([',', ' ']).to_string()
}

/// "10m tea", "me in an hour to stretch", "me to call Ana at 3pm":
/// when (Unix time) and what to say.
pub fn parse(text: &str, now: Now) -> Option<(i64, String)> {
    let original: Vec<&str> = text.split_whitespace().collect();
    let mut start = 0;
    if original
        .first()
        .is_some_and(|w| w.eq_ignore_ascii_case("me"))
    {
        start = 1;
    }
    let lower: Vec<String> = original
        .iter()
        .map(|w| w.trim_end_matches([',', '.']).to_lowercase())
        .collect();
    let message = |words: &[&str]| {
        let text = trim_filler(words);
        if text.is_empty() {
            "Reminder".to_string()
        } else {
            text
        }
    };
    // The time first: "in 10 minutes to check the oven".
    if let Some((at, used)) = when(&lower[start..], now) {
        return Some((at, message(&original[start + used..])));
    }
    // The time last: "to check the oven in 10 minutes".
    for split in start + 1..lower.len() {
        if let Some((at, used)) = when(&lower[split..], now) {
            if split + used == lower.len() {
                return Some((at, message(&original[start..split])));
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reminder {
    /// Unix time it is due.
    pub at: i64,
    pub text: String,
}

/// Pending reminders, saved after every change.
#[derive(Debug, Default)]
pub struct Store {
    path: Option<PathBuf>,
    pub items: Vec<Reminder>,
}

impl Store {
    pub fn load() -> Self {
        let path = dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|dir| dir.join("poppi_launcher").join("reminders.json"));
        let items = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, items }
    }

    pub fn add(&mut self, reminder: Reminder) {
        self.items.push(reminder);
        self.items.sort_by_key(|r| r.at);
        self.save();
    }

    pub fn remove(&mut self, reminder: &Reminder) {
        self.items.retain(|r| r != reminder);
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.items) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// "15:40", "tomorrow 09:00" or "Mon 14 Dec 18:00" for a Unix time.
pub fn describe(at: i64, now: Now) -> String {
    let Ok(time) = gtk::glib::DateTime::from_unix_local(at) else {
        return String::new();
    };
    let since_midnight = i64::from(now.hour * 3600 + now.minute * 60 + now.second);
    let today_end = now.unix - since_midnight + 86_400;
    let format = if at < today_end {
        "%H:%M"
    } else if at < today_end + 86_400 {
        "tomorrow %H:%M"
    } else {
        "%a %e %b %H:%M"
    };
    time.format(format)
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// "in 10 min", "in 2 h 5 min" for a Unix time.
pub fn from_now(at: i64, now: Now) -> String {
    let minutes = ((at - now.unix).max(0) + 59) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("in {} min", m),
        (h, 0) => format!("in {} h", h),
        (h, m) => format!("in {} h {} min", h, m),
    }
}

/// Add a line to the notes file, creating it if needed.
pub fn add_note(file: &str, text: &str) -> Result<(), String> {
    use std::io::Write;
    let path = crate::files::expand_tilde(file);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let stamp = gtk::glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Could not open {}: {}", file, e))?;
    writeln!(out, "- {} {}", stamp, text.trim()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10:00:00 local, at Unix time 1_000_000.
    const NOW: Now = Now {
        unix: 1_000_000,
        hour: 10,
        minute: 0,
        second: 0,
    };

    fn p(text: &str) -> Option<(i64, String)> {
        parse(text, NOW).map(|(at, text)| (at - NOW.unix, text))
    }

    #[test]
    fn durations() {
        assert_eq!(p("10m tea"), Some((600, "tea".into())));
        assert_eq!(
            p("me in 10 minutes to check the oven"),
            Some((600, "check the oven".into()))
        );
        assert_eq!(p("in 1h30m stretch"), Some((5400, "stretch".into())));
        assert_eq!(
            p("in an hour and 30 minutes call mom"),
            Some((5400, "call mom".into()))
        );
        assert_eq!(p("half an hour laundry"), Some((1800, "laundry".into())));
        assert_eq!(p("in 2 hours"), Some((7200, "Reminder".into())));
    }

    #[test]
    fn times_of_day_today_or_tomorrow() {
        assert_eq!(
            p("at 15:30 call Ana"),
            Some((5 * 3600 + 1800, "call Ana".into()))
        );
        assert_eq!(
            p("me at 3pm to call Ana"),
            Some((5 * 3600, "call Ana".into()))
        );
        assert_eq!(
            p("at 3:15 pm meeting"),
            Some((5 * 3600 + 900, "meeting".into()))
        );
        assert_eq!(p("at 9am standup"), Some((23 * 3600, "standup".into())));
        assert_eq!(p("at noon lunch"), Some((2 * 3600, "lunch".into())));
    }

    #[test]
    fn the_time_may_come_last() {
        assert_eq!(
            p("me to check the oven in 10 minutes"),
            Some((600, "check the oven".into()))
        );
        assert_eq!(
            p("call Ana at 15:30"),
            Some((5 * 3600 + 1800, "call Ana".into()))
        );
    }

    #[test]
    fn vague_or_missing_times_are_not_guessed() {
        assert_eq!(p("call Ana"), None);
        assert_eq!(p("at 5 call Ana"), None);
        assert_eq!(p("in 0 minutes x"), None);
        assert_eq!(p(""), None);
        assert_eq!(p("at 25:00 x"), None);
    }
}
