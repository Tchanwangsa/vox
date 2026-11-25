//! Running totals for the Stats page: words, speaking time and dictations
//! per day in this Mac's time zone, and which apps the words went to.
//!
//! The history keeps only the latest dictations; these totals keep every
//! day, so the activity chart reaches back further than the list does.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{data_dir, Entry};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DayTotal {
    pub words: usize,
    /// Seconds spent speaking.
    pub secs: f32,
    pub dictations: usize,
    /// Words by the app they were typed into.
    pub apps: BTreeMap<String, usize>,
}

impl DayTotal {
    /// Words a minute while speaking on this day.
    pub fn wpm(&self) -> Option<f32> {
        (self.secs >= 1. && self.words > 0).then(|| self.words as f32 / (self.secs / 60.))
    }

    /// Words by app on this day, most first.
    pub fn apps(&self) -> Vec<(String, usize)> {
        let mut apps: Vec<_> = self.apps.iter().map(|(a, &w)| (a.clone(), w)).collect();
        apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        apps
    }
}

#[derive(Debug, Default)]
pub struct Stats {
    /// By local date, `YYYY-MM-DD`, so they sort by day.
    pub days: BTreeMap<String, DayTotal>,
}

impl Stats {
    fn path() -> PathBuf {
        data_dir().join("stats.json")
    }

    /// The totals kept so far; the first time, worked out from the history.
    pub fn load(history: &[Entry]) -> Self {
        let days = std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        match days {
            Some(days) => Self { days },
            None => {
                let mut stats = Self::default();
                for entry in history {
                    stats.count(entry);
                }
                stats.save();
                stats
            }
        }
    }

    pub fn add(&mut self, entry: &Entry) {
        self.count(entry);
        self.save();
    }

    fn count(&mut self, entry: &Entry) {
        let day = self.days.entry(date_key(entry.at)).or_default();
        let words = entry.words();
        day.words += words;
        day.secs += entry.spoken_secs;
        day.dictations += 1;
        if let Some(app) = &entry.app {
            *day.apps.entry(app.clone()).or_default() += words;
        }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(&self.days) {
            std::fs::write(Self::path(), json).ok();
        }
    }

    pub fn day(&self, at: u64) -> Option<&DayTotal> {
        self.days.get(&date_key(at))
    }

    /// Words a minute while speaking, over every day kept.
    pub fn wpm(&self) -> Option<f32> {
        let (words, secs) = self
            .days
            .values()
            .fold((0, 0.), |(w, s), d| (w + d.words, s + d.secs));
        (secs >= 1. && words > 0).then(|| words as f32 / (secs / 60.))
    }

    pub fn words(&self) -> usize {
        self.days.values().map(|d| d.words).sum()
    }

    pub fn dictations(&self) -> usize {
        self.days.values().map(|d| d.dictations).sum()
    }

    pub fn spoken_secs(&self) -> f32 {
        self.days.values().map(|d| d.secs).sum()
    }

    /// Days in a row with a dictation, ending today or yesterday.
    pub fn streak(&self, today: u64) -> usize {
        let has = |at: u64| self.day(at).is_some_and(|d| d.dictations > 0);
        let noon = local_noon(today);
        let start = if has(noon) { 0 } else { 1 };
        (start..)
            .take_while(|&i| has(noon.saturating_sub(i * DAY)))
            .count()
    }

    /// Words by app, most first.
    pub fn apps(&self) -> Vec<(String, usize)> {
        let mut apps: BTreeMap<&str, usize> = BTreeMap::new();
        for day in self.days.values() {
            for (app, words) in &day.apps {
                *apps.entry(app).or_default() += words;
            }
        }
        let mut apps: Vec<_> = apps.into_iter().map(|(a, w)| (a.to_string(), w)).collect();
        apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        apps
    }
}

pub const DAY: u64 = 24 * 3600;

pub fn local(at: u64) -> libc::tm {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let seconds = at as libc::time_t;
    unsafe { libc::localtime_r(&seconds, &mut tm) };
    tm
}

pub fn date_key(at: u64) -> String {
    let tm = local(at);
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

/// Midday of `at`'s local day. Stepping back from it a day at a time lands
/// on each earlier day once, whatever daylight saving does.
pub fn local_noon(at: u64) -> u64 {
    let tm = local(at);
    let since_midnight = (tm.tm_hour * 3600 + tm.tm_min * 60 + tm.tm_sec) as u64;
    at.saturating_sub(since_midnight) + 12 * 3600
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Avenue;

    fn entry(at: u64, text: &str, app: Option<&str>) -> Entry {
        Entry {
            at,
            text: text.into(),
            heard: text.into(),
            avenue: Avenue::Local,
            spoken_secs: 6.,
            app: app.map(Into::into),
        }
    }

    #[test]
    fn totals_count_words_by_day_and_app() {
        let noon = local_noon(1_700_000_000);
        let mut stats = Stats::default();
        stats.count(&entry(noon, "one two three", Some("Notes")));
        stats.count(&entry(noon + 60, "four five", Some("Mail")));
        stats.count(&entry(noon - DAY, "six", None));

        assert_eq!(stats.words(), 6);
        assert_eq!(stats.dictations(), 3);
        assert_eq!(stats.day(noon).unwrap().words, 5);
        assert_eq!(stats.day(noon).unwrap().apps(), vec![("Notes".into(), 3), ("Mail".into(), 2)]);
        assert_eq!(stats.day(noon).unwrap().wpm().unwrap().round(), 25.);
        assert_eq!(stats.apps(), vec![("Notes".into(), 3), ("Mail".into(), 2)]);
        assert_eq!(stats.streak(noon), 2);
        assert_eq!(stats.streak(noon + 2 * DAY), 0);
        // Six words in eighteen seconds.
        assert_eq!(stats.wpm().unwrap().round(), 20.);
    }
}
