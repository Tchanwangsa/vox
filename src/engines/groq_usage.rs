//! How much of Groq's allowance is left, as its last answers said: each
//! successful response's `x-ratelimit-*` headers (requests per day, tokens
//! per minute, per role). Reading it never calls Groq.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::data_dir;

const DAY_MS: u64 = 24 * 3600 * 1000;

/// What the Groq model was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Transcription,
    Cleanup,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Self::Transcription => "Transcribe",
            Self::Cleanup => "Clean up",
        }
    }
}

/// Which quota window a [`UsageRow`] reflects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    Today,
    ThisMinute,
}

impl Window {
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::ThisMinute => "this minute",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::ThisMinute => "this min",
        }
    }
}

/// One Groq response's rate-limit headers. Resets are seconds after `read_at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub model: String,
    pub role: Role,
    /// Requests per day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_reset: Option<f64>,
    /// Tokens per minute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_reset: Option<f64>,
    /// Milliseconds since the Unix epoch.
    pub read_at: u64,
}

impl Reading {
    /// Parse the rate-limit headers of one response (names in any case), or
    /// `None` when it carries none of them.
    pub fn from_headers(
        role: Role,
        model: &str,
        headers: &[(String, String)],
        read_at: u64,
    ) -> Option<Self> {
        let header = |name: &str| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim())
        };
        let count = |name: &str| header(name).and_then(|v| v.parse::<u64>().ok());
        let reset = |name: &str| header(name).and_then(duration_secs);
        let reading = Self {
            model: model.to_string(),
            role,
            requests_limit: count("x-ratelimit-limit-requests"),
            requests_remaining: count("x-ratelimit-remaining-requests"),
            requests_reset: reset("x-ratelimit-reset-requests"),
            tokens_limit: count("x-ratelimit-limit-tokens"),
            tokens_remaining: count("x-ratelimit-remaining-tokens"),
            tokens_reset: reset("x-ratelimit-reset-tokens"),
            read_at,
        };
        let any = reading.requests_limit.is_some()
            || reading.requests_remaining.is_some()
            || reading.requests_reset.is_some()
            || reading.tokens_limit.is_some()
            || reading.tokens_remaining.is_some()
            || reading.tokens_reset.is_some();
        any.then_some(reading)
    }

    /// Convert to a displayable row: prefers requests per day, else tokens per
    /// minute; returns `None` if neither limit+remaining pair is present.
    pub fn row(&self) -> Option<UsageRow> {
        let figure = |remaining: Option<u64>, limit: Option<u64>, reset: Option<f64>| {
            let remaining = remaining?;
            let limit = limit.filter(|&l| l > 0)?;
            Some((remaining, limit, reset))
        };
        let (window, remaining, limit, reset) =
            if let Some((rem, lim, rst)) =
                figure(self.requests_remaining, self.requests_limit, self.requests_reset)
            {
                (Window::Today, rem, lim, rst)
            } else if let Some((rem, lim, rst)) =
                figure(self.tokens_remaining, self.tokens_limit, self.tokens_reset)
            {
                (Window::ThisMinute, rem, lim, rst)
            } else {
                return None;
            };

        let remaining = remaining.min(limit);
        let used = limit.saturating_sub(remaining);
        let resets_at_ms = reset.and_then(|secs| {
            (secs.is_finite() && secs >= 0.0)
                .then(|| self.read_at.saturating_add((secs * 1000.0).round() as u64))
        });

        Some(UsageRow {
            role: self.role,
            window,
            used,
            remaining,
            limit,
            // Strictly under 10% left.
            warn: remaining.saturating_mul(10) < limit,
            resets_at_ms,
        })
    }
}

/// One displayable quota bar: how much has been used, how much is left, and
/// when the window resets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRow {
    pub role: Role,
    pub window: Window,
    pub used: u64,
    pub remaining: u64,
    pub limit: u64,
    /// True when fewer than 10% of the limit remains.
    pub warn: bool,
    /// When the quota resets, in ms since the Unix epoch.
    pub resets_at_ms: Option<u64>,
}

impl UsageRow {
    /// Share of the limit consumed, in `0.0..=1.0`.
    pub fn used_fraction(&self) -> f32 {
        if self.limit == 0 {
            return 0.0;
        }
        (self.used as f32 / self.limit as f32).clamp(0.0, 1.0)
    }

    pub fn exhausted(&self) -> bool {
        self.remaining == 0
    }

    /// `"resets in 3h 12m"`, `"resets in 43s"`; `None` once the reset has passed.
    pub fn resets_in(&self, now_ms: u64) -> Option<String> {
        resets_in(self.resets_at_ms, now_ms)
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The Groq readings from one dictation or remembered for the Engine tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroqUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcription: Option<Reading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<Reading>,
    /// True once the low-quota (<10%) pill warning has been shown for each role;
    /// cleared automatically once that role recovers to >=10% remaining.
    #[serde(default, skip_serializing_if = "is_false")]
    pub warned_transcription: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub warned_cleanup: bool,
}

impl GroqUsage {
    fn path() -> PathBuf {
        data_dir().join("groq_usage.json")
    }

    /// Load persisted readings from disk, dropping any older than 24 hours.
    pub fn load() -> Self {
        let now = now_ms();
        let mut usage: Self = std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let fresh = |r: &Reading| now.saturating_sub(r.read_at) < DAY_MS;
        usage.transcription = usage.transcription.filter(fresh);
        usage.cleanup = usage.cleanup.filter(fresh);
        if usage.transcription.is_none() {
            usage.warned_transcription = false;
        }
        if usage.cleanup.is_none() {
            usage.warned_cleanup = false;
        }
        usage
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_vec(self) {
            std::fs::write(Self::path(), json).ok();
        }
    }

    pub fn clear(&mut self) {
        self.transcription = None;
        self.cleanup = None;
        self.warned_transcription = false;
        self.warned_cleanup = false;
        std::fs::remove_file(Self::path()).ok();
    }

    /// Merge incoming readings, persist them, and return only the role(s) that
    /// newly crossed into low quota (<10% left) and haven't been warned yet.
    pub fn merge_and_take_warnings(&mut self, incoming: &Self) -> Self {
        let mut changed = false;
        let mut warnings = Self::default();
        if let Some(reading) = &incoming.transcription {
            self.transcription = Some(reading.clone());
            let low = reading.row().is_some_and(|r| r.warn);
            if low {
                if !self.warned_transcription {
                    self.warned_transcription = true;
                    warnings.transcription = Some(reading.clone());
                }
            } else {
                self.warned_transcription = false;
            }
            changed = true;
        }
        if let Some(reading) = &incoming.cleanup {
            self.cleanup = Some(reading.clone());
            let low = reading.row().is_some_and(|r| r.warn);
            if low {
                if !self.warned_cleanup {
                    self.warned_cleanup = true;
                    warnings.cleanup = Some(reading.clone());
                }
            } else {
                self.warned_cleanup = false;
            }
            changed = true;
        }
        if changed {
            self.save();
        }
        warnings
    }

    pub fn record_reading(&mut self, reading: Reading) {
        match reading.role {
            Role::Transcription => self.transcription = Some(reading),
            Role::Cleanup => self.cleanup = Some(reading),
        }
    }

    pub fn row_for(&self, role: Role) -> Option<UsageRow> {
        match role {
            Role::Transcription => self.transcription.as_ref().and_then(Reading::row),
            Role::Cleanup => self.cleanup.as_ref().and_then(Reading::row),
        }
    }

    pub fn rows(&self) -> Vec<UsageRow> {
        let mut out = Vec::with_capacity(2);
        if let Some(row) = self.row_for(Role::Transcription) {
            out.push(row);
        }
        if let Some(row) = self.row_for(Role::Cleanup) {
            out.push(row);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.rows().is_empty()
    }

    /// Snapshot of formatted reset strings, so periodic UI ticks only repaint
    /// when a visible countdown actually changes.
    pub fn reset_signature(&self, now_ms: u64) -> [Option<String>; 2] {
        [
            self.row_for(Role::Transcription).and_then(|r| r.resets_in(now_ms)),
            self.row_for(Role::Cleanup).and_then(|r| r.resets_in(now_ms)),
        ]
    }

    /// Synthetic readings for `VOX_DEMO=done-groq`, `done-groq-low`, and `done-groq-low-both`.
    pub fn demo(low: bool) -> Self {
        let now = now_ms();
        let (tr_rem, cl_rem, tr_rst, cl_rst) = if low {
            (140, 62, 11_520.0, 4_320.0)
        } else {
            (1_642, 812, 1_920.0, 2_700.0)
        };
        Self {
            transcription: Some(Reading {
                model: crate::engines::groq::HEAR_MODEL.into(),
                role: Role::Transcription,
                requests_limit: Some(2_000),
                requests_remaining: Some(tr_rem),
                requests_reset: Some(tr_rst),
                tokens_limit: None,
                tokens_remaining: None,
                tokens_reset: None,
                read_at: now,
            }),
            cleanup: Some(Reading {
                model: crate::engines::groq::TIDY_MODEL.into(),
                role: Role::Cleanup,
                requests_limit: Some(1_000),
                requests_remaining: Some(cl_rem),
                requests_reset: Some(cl_rst),
                tokens_limit: None,
                tokens_remaining: None,
                tokens_reset: None,
                read_at: now,
            }),
            warned_transcription: false,
            warned_cleanup: false,
        }
    }

    /// Synthetic readings where only Clean up is below 10% (the typical one-item warning).
    pub fn demo_low_cleanup() -> Self {
        let now = now_ms();
        Self {
            transcription: Some(Reading {
                model: crate::engines::groq::HEAR_MODEL.into(),
                role: Role::Transcription,
                requests_limit: Some(2_000),
                requests_remaining: Some(1_642),
                requests_reset: Some(1_920.0),
                tokens_limit: None,
                tokens_remaining: None,
                tokens_reset: None,
                read_at: now,
            }),
            cleanup: Some(Reading {
                model: crate::engines::groq::TIDY_MODEL.into(),
                role: Role::Cleanup,
                requests_limit: Some(1_000),
                requests_remaining: Some(62),
                requests_reset: Some(4_320.0),
                tokens_limit: None,
                tokens_remaining: None,
                tokens_reset: None,
                read_at: now,
            }),
            warned_transcription: false,
            warned_cleanup: false,
        }
    }
}

/// Groq's reset durations in seconds: `7.66s`, `2m59.56s`, `1h2m3s`, `120ms`.
pub fn duration_secs(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut total = 0.0;
    let mut rest = text;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let value: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = rest
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(rest.len());
        total += value
            * match &rest[..unit] {
                "h" => 3600.0,
                "m" => 60.0,
                "s" => 1.0,
                "ms" => 0.001,
                _ => return None,
            };
        rest = &rest[unit..];
    }
    Some(total)
}

/// `"resets in 3h 12m"`, `"resets in 43s"`; `None` once the reset has passed.
pub fn resets_in(resets_at_ms: Option<u64>, now_ms: u64) -> Option<String> {
    let resets_at = resets_at_ms?;
    if resets_at <= now_ms {
        return None;
    }
    let seconds = ((resets_at - now_ms) as f64 / 1000.0).round() as u64;
    if seconds == 0 {
        return None;
    }
    if seconds < 60 {
        return Some(format!("resets in {seconds}s"));
    }
    let minutes = (seconds + 30) / 60;
    if minutes < 60 {
        return Some(format!("resets in {minutes}m"));
    }
    let hours = minutes / 60;
    let rest = minutes % 60;
    if rest == 0 {
        Some(format!("resets in {hours}h"))
    } else {
        Some(format!("resets in {hours}h {rest}m"))
    }
}

/// 12345 as `"12,345"`.
pub fn grouped_u64(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn resets_read_groqs_duration_formats() {
        assert_eq!(duration_secs("7.66s"), Some(7.66));
        assert!((duration_secs("2m59.56s").unwrap() - 179.56).abs() < 1e-9);
        assert_eq!(duration_secs("1h2m3s"), Some(3723.0));
        assert_eq!(duration_secs("120ms"), Some(0.12));
        assert_eq!(duration_secs(" 30s "), Some(30.0));
        for bad in ["", "soon", "7", "7.66", "2x", "m5s"] {
            assert_eq!(duration_secs(bad), None, "{bad}");
        }
    }

    #[test]
    fn reading_parses_headers_case_insensitively_and_computes_row() {
        let reading = Reading::from_headers(
            Role::Transcription,
            "whisper-large-v3-turbo",
            &headers(&[
                ("X-RateLimit-Limit-Requests", "2000"),
                ("x-ratelimit-remaining-requests", "1996"),
                ("x-ratelimit-reset-requests", "2m59.56s"),
                ("x-ratelimit-limit-tokens", "6000"),
                ("x-ratelimit-remaining-tokens", "5800"),
                ("x-ratelimit-reset-tokens", "7.66s"),
            ]),
            1_000_000,
        )
        .unwrap();

        let row = reading.row().unwrap();
        assert_eq!(row.role, Role::Transcription);
        assert_eq!(row.window, Window::Today);
        assert_eq!(row.used, 4);
        assert_eq!(row.remaining, 1996);
        assert_eq!(row.limit, 2000);
        assert!(!row.warn);
        assert!((row.used_fraction() - 0.002).abs() < 1e-6);
        assert_eq!(row.resets_in(1_000_000).as_deref(), Some("resets in 3m"));
    }

    #[test]
    fn row_falls_back_to_tokens_when_requests_absent_and_warns_under_tenth() {
        let reading = Reading::from_headers(
            Role::Cleanup,
            "qwen/qwen3.8-27b",
            &headers(&[
                ("x-ratelimit-limit-tokens", "6000"),
                ("x-ratelimit-remaining-tokens", "500"),
                ("x-ratelimit-reset-tokens", "43s"),
            ]),
            5_000,
        )
        .unwrap();

        let row = reading.row().unwrap();
        assert_eq!(row.window, Window::ThisMinute);
        assert_eq!(row.used, 5500);
        assert_eq!(row.remaining, 500);
        assert!(row.warn);
        assert_eq!(row.resets_in(5_000).as_deref(), Some("resets in 43s"));
        assert_eq!(row.resets_in(50_000), None);
    }

    #[test]
    fn missing_or_zero_limits_produce_no_row() {
        assert_eq!(
            Reading::from_headers(
                Role::Transcription,
                "m",
                &headers(&[("content-type", "application/json")]),
                1,
            ),
            None
        );
        let zero = Reading::from_headers(
            Role::Transcription,
            "m",
            &headers(&[
                ("x-ratelimit-limit-requests", "0"),
                ("x-ratelimit-remaining-requests", "0"),
            ]),
            1,
        )
        .unwrap();
        assert_eq!(zero.row(), None);
    }

    #[test]
    fn resets_in_formats_seconds_minutes_and_hours() {
        let base = 10_000;
        assert_eq!(resets_in(Some(base + 43_000), base).as_deref(), Some("resets in 43s"));
        assert_eq!(resets_in(Some(base + 180_000), base).as_deref(), Some("resets in 3m"));
        assert_eq!(resets_in(Some(base + 3_600_000), base).as_deref(), Some("resets in 1h"));
        assert_eq!(resets_in(Some(base + 11_520_000), base).as_deref(), Some("resets in 3h 12m"));
        assert_eq!(resets_in(Some(base), base), None);
    }

    #[test]
    fn grouped_u64_adds_thousands_commas() {
        assert_eq!(grouped_u64(0), "0");
        assert_eq!(grouped_u64(999), "999");
        assert_eq!(grouped_u64(1_000), "1,000");
        assert_eq!(grouped_u64(12_345), "12,345");
    }

    #[test]
    fn low_usage_warns_only_low_role_and_only_once_until_recovery() {
        let mk = |role: Role, rem: u64, lim: u64| Reading {
            model: "m".into(),
            role,
            requests_limit: Some(lim),
            requests_remaining: Some(rem),
            requests_reset: Some(3_600.0),
            tokens_limit: None,
            tokens_remaining: None,
            tokens_reset: None,
            read_at: 10_000,
        };
        let mut stored = GroqUsage::default();

        // Both healthy: no warning.
        let w1 = stored.merge_and_take_warnings(&GroqUsage {
            transcription: Some(mk(Role::Transcription, 1500, 2000)),
            cleanup: Some(mk(Role::Cleanup, 800, 1000)),
            ..Default::default()
        });
        assert!(w1.is_empty());

        // Cleanup drops below 10%: warns only Cleanup.
        let w2 = stored.merge_and_take_warnings(&GroqUsage {
            transcription: Some(mk(Role::Transcription, 1400, 2000)),
            cleanup: Some(mk(Role::Cleanup, 90, 1000)),
            ..Default::default()
        });
        assert!(w2.transcription.is_none());
        assert!(w2.cleanup.is_some());

        // Cleanup stays below 10%: does NOT warn again.
        let w3 = stored.merge_and_take_warnings(&GroqUsage {
            transcription: Some(mk(Role::Transcription, 1300, 2000)),
            cleanup: Some(mk(Role::Cleanup, 80, 1000)),
            ..Default::default()
        });
        assert!(w3.is_empty());

        // Transcription now also drops below 10%: warns only Transcription.
        let w4 = stored.merge_and_take_warnings(&GroqUsage {
            transcription: Some(mk(Role::Transcription, 150, 2000)),
            cleanup: Some(mk(Role::Cleanup, 70, 1000)),
            ..Default::default()
        });
        assert!(w4.transcription.is_some());
        assert!(w4.cleanup.is_none());

        // Next day: Cleanup recovers to >=10%, then later drops below 10% again -> warns once more.
        let w5 = stored.merge_and_take_warnings(&GroqUsage {
            cleanup: Some(mk(Role::Cleanup, 995, 1000)),
            ..Default::default()
        });
        assert!(w5.is_empty());
        assert!(!stored.warned_cleanup);
        assert!(stored.warned_transcription);

        let w6 = stored.merge_and_take_warnings(&GroqUsage {
            cleanup: Some(mk(Role::Cleanup, 50, 1000)),
            ..Default::default()
        });
        assert!(w6.cleanup.is_some());
        assert!(w6.transcription.is_none());
    }
}
