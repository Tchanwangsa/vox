//! How much of Groq's allowance is left, as its last answers said: each
//! response's `x-ratelimit-*` headers (requests per day, tokens per minute)
//! plus `429` cooldowns for limits Groq omits from `200` headers (RPM, TPD,
//! ASH, ASD). Reading it never calls Groq; bucket counts refill linearly in
//! real time using Groq's reset duration.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::data_dir;

const DAY_MS: u64 = 24 * 3600 * 1000;
const DEFAULT_RATE_LIMIT_MS: u64 = 60_000;

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

/// Which Groq rate-limit dimension a [`UsageRow`] reflects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    RequestsPerDay,
    RequestsPerMinute,
    TokensPerMinute,
    TokensPerDay,
    AudioSecondsPerHour,
    AudioSecondsPerDay,
}

impl Metric {
    /// Compact unit label shown beside counts, e.g. `"5 of 2,000 req/day"`.
    pub fn unit_label(self) -> &'static str {
        match self {
            Self::RequestsPerDay => "req/day",
            Self::RequestsPerMinute => "req/min",
            Self::TokensPerMinute => "tok/min",
            Self::TokensPerDay => "tok/day",
            Self::AudioSecondsPerHour => "sec/hr",
            Self::AudioSecondsPerDay => "sec/day",
        }
    }

    /// Short label used in the compact pill header/rows.
    pub fn short_label(self) -> &'static str {
        self.unit_label()
    }

    /// Human-readable name used in `429` error messages.
    pub fn error_label(self) -> &'static str {
        match self {
            Self::RequestsPerDay => "requests/day",
            Self::RequestsPerMinute => "requests/min",
            Self::TokensPerMinute => "tokens/min",
            Self::TokensPerDay => "tokens/day",
            Self::AudioSecondsPerHour => "audio/hour",
            Self::AudioSecondsPerDay => "audio/day",
        }
    }
}

/// A `429 Too Many Requests` response from Groq, capturing whichever limit
/// tripped (including RPM, TPD, ASH, and ASD which `200` headers omit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateLimitHit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<Metric>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
    /// Seconds after `at` until the caller may retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<f64>,
    /// Milliseconds since the Unix epoch when the `429` arrived.
    pub at: u64,
}

impl RateLimitHit {
    pub fn retry_at_ms(&self) -> Option<u64> {
        self.retry_after.and_then(|secs| {
            (secs.is_finite() && secs > 0.0)
                .then(|| self.at.saturating_add((secs * 1000.0).round() as u64))
        })
    }

    pub fn active_until_ms(&self) -> u64 {
        self.retry_at_ms()
            .unwrap_or_else(|| self.at.saturating_add(DEFAULT_RATE_LIMIT_MS))
    }

    pub fn is_active(&self, now_ms: u64) -> bool {
        now_ms < self.active_until_ms()
    }

    pub fn error_message(&self) -> String {
        let what = match self.metric {
            Some(metric) => format!("Groq {} limit reached.", metric.error_label()),
            None => "Groq rate limit reached.".into(),
        };
        let wait = self
            .retry_after
            .and_then(|secs| {
                (secs.is_finite() && secs > 0.0)
                    .then(|| format_duration_ms((secs * 1000.0).round() as u64))
            })
            .flatten();
        match wait {
            Some(dur) => format!("{what} Try again in {dur}."),
            None => format!("{what} Try again soon."),
        }
    }
}

/// One Groq response's rate-limit headers (and optional `429` hit).
/// Reset durations are seconds after `read_at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub model: String,
    pub role: Role,
    /// Requests per day (RPD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_reset: Option<f64>,
    /// Tokens per minute (TPM).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_reset: Option<f64>,
    /// Milliseconds since the Unix epoch when the headers above were read.
    pub read_at: u64,
    /// Active `429` rate-limit hit, if the latest refusal was a `429`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitHit>,
}

impl Reading {
    pub fn has_headers(&self) -> bool {
        self.requests_limit.is_some()
            || self.requests_remaining.is_some()
            || self.requests_reset.is_some()
            || self.tokens_limit.is_some()
            || self.tokens_remaining.is_some()
            || self.tokens_reset.is_some()
    }

    pub fn latest_at(&self) -> u64 {
        self.rate_limit
            .as_ref()
            .map_or(self.read_at, |hit| self.read_at.max(hit.at))
    }

    /// Parse the rate-limit headers of one `200` response (names in any case),
    /// or `None` when it carries none of them.
    pub fn from_headers(
        role: Role,
        model: &str,
        headers: &[(String, String)],
        read_at: u64,
    ) -> Option<Self> {
        let reading = Self::parse_headers_raw(role, model, headers, read_at);
        reading.has_headers().then_some(reading)
    }

    /// Parse a `429 Too Many Requests` response's headers and JSON body.
    pub fn from_rate_limit(
        role: Role,
        model: &str,
        headers: &[(String, String)],
        body: &str,
        at: u64,
    ) -> Self {
        let mut reading = Self::parse_headers_raw(role, model, headers, at);
        let retry_header = find_header(headers, "retry-after").and_then(parse_retry_after);
        let parsed = parse_rate_limit_body(body);
        let metric = parsed.metric.or_else(|| {
            if reading.requests_remaining == Some(0) {
                Some(Metric::RequestsPerDay)
            } else if reading.tokens_remaining == Some(0) {
                Some(Metric::TokensPerMinute)
            } else {
                None
            }
        });
        let retry_after = retry_header.or(parsed.retry_after).or_else(|| match metric {
            Some(Metric::RequestsPerDay) => reading.requests_reset,
            Some(Metric::TokensPerMinute) => reading.tokens_reset,
            _ => None,
        });
        reading.rate_limit = Some(RateLimitHit {
            metric,
            limit: parsed.limit,
            used: parsed.used,
            retry_after,
            at,
        });
        reading
    }

    pub fn rate_limit_message(&self) -> String {
        match &self.rate_limit {
            Some(hit) => hit.error_message(),
            None => "Groq rate limit reached. Try again soon.".into(),
        }
    }

    fn parse_headers_raw(
        role: Role,
        model: &str,
        headers: &[(String, String)],
        read_at: u64,
    ) -> Self {
        let count = |name: &str| find_header(headers, name).and_then(|v| v.parse::<u64>().ok());
        let reset = |name: &str| find_header(headers, name).and_then(duration_secs);
        Self {
            model: model.to_string(),
            role,
            requests_limit: count("x-ratelimit-limit-requests"),
            requests_remaining: count("x-ratelimit-remaining-requests"),
            requests_reset: reset("x-ratelimit-reset-requests"),
            tokens_limit: count("x-ratelimit-limit-tokens"),
            tokens_remaining: count("x-ratelimit-remaining-tokens"),
            tokens_reset: reset("x-ratelimit-reset-tokens"),
            read_at,
            rate_limit: None,
        }
    }

    /// Convert to the most constrained displayable row at `now_ms`:
    /// an active `429` cooldown wins; otherwise picks the bucket (RPD vs TPM)
    /// with the highest live refill-adjusted used share (ties prefer RPD).
    pub fn row_at(&self, now_ms: u64) -> Option<UsageRow> {
        if let Some(hit) = &self.rate_limit
            && hit.is_active(now_ms)
        {
            let limit = hit.limit.unwrap_or(0);
            let used = if limit > 0 {
                hit.used.unwrap_or(limit).min(limit)
            } else {
                0
            };
            let remaining = limit.saturating_sub(used);
            return Some(UsageRow {
                role: self.role,
                metric: hit.metric,
                used,
                remaining,
                limit,
                warn: true,
                rate_limited: true,
                resets_at_ms: hit.retry_at_ms(),
            });
        }

        let rpd = live_bucket(
            self.role,
            Metric::RequestsPerDay,
            self.requests_limit,
            self.requests_remaining,
            self.requests_reset,
            self.read_at,
            now_ms,
        );
        let tpm = live_bucket(
            self.role,
            Metric::TokensPerMinute,
            self.tokens_limit,
            self.tokens_remaining,
            self.tokens_reset,
            self.read_at,
            now_ms,
        );

        match (rpd, tpm) {
            (Some(rpd), Some(tpm)) => {
                if tpm.used_fraction_f64() > rpd.used_fraction_f64() {
                    Some(tpm)
                } else {
                    Some(rpd)
                }
            }
            (Some(row), None) | (None, Some(row)) => Some(row),
            (None, None) => None,
        }
    }
}

/// Evaluate one token-bucket header trio at `now_ms`, interpolating linear
/// refill until `read_at + reset_secs` when the bucket returns to full.
fn live_bucket(
    role: Role,
    metric: Metric,
    limit: Option<u64>,
    remaining: Option<u64>,
    reset_secs: Option<f64>,
    read_at: u64,
    now_ms: u64,
) -> Option<UsageRow> {
    let limit = limit.filter(|&l| l > 0)?;
    let rem_at_read = remaining?.min(limit);
    let used_at_read = limit.saturating_sub(rem_at_read);

    let (used, remaining, resets_at_ms) = if used_at_read == 0 {
        (0, limit, None)
    } else if let Some(secs) = reset_secs.filter(|s| s.is_finite() && *s >= 0.0) {
        let reset_ms = (secs * 1000.0).round() as u64;
        let resets_at = read_at.saturating_add(reset_ms);
        if reset_ms == 0 || now_ms >= resets_at {
            (0, limit, None)
        } else {
            let left_ms = resets_at - now_ms;
            let ratio = (left_ms as f64 / reset_ms as f64).clamp(0.0, 1.0);
            let live_used = ((used_at_read as f64) * ratio)
                .ceil()
                .clamp(1.0, used_at_read as f64) as u64;
            let live_rem = limit.saturating_sub(live_used);
            (live_used, live_rem, Some(resets_at))
        }
    } else {
        (used_at_read, rem_at_read, None)
    };

    Some(UsageRow {
        role,
        metric: Some(metric),
        used,
        remaining,
        limit,
        // Strictly under 10% left.
        warn: remaining.saturating_mul(10) < limit,
        rate_limited: false,
        resets_at_ms,
    })
}

/// One displayable quota bar: how much is used right now, how much is left,
/// and when the bucket finishes refilling (or when a `429` retry is allowed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRow {
    pub role: Role,
    pub metric: Option<Metric>,
    pub used: u64,
    pub remaining: u64,
    pub limit: u64,
    /// True when fewer than 10% of the limit remains, or actively rate-limited.
    pub warn: bool,
    /// True when an active `429` cooldown is in effect.
    pub rate_limited: bool,
    /// When the bucket is full again (or when retry is allowed), in ms since epoch.
    pub resets_at_ms: Option<u64>,
}

impl UsageRow {
    fn used_fraction_f64(&self) -> f64 {
        if self.rate_limited && self.limit == 0 {
            return 1.0;
        }
        if self.limit == 0 {
            return 0.0;
        }
        (self.used as f64 / self.limit as f64).clamp(0.0, 1.0)
    }

    /// Share of the limit consumed, in `0.0..=1.0`.
    pub fn used_fraction(&self) -> f32 {
        self.used_fraction_f64() as f32
    }

    pub fn exhausted(&self) -> bool {
        self.rate_limited || (self.limit > 0 && self.remaining == 0)
    }

    /// `"full in 3m"` for refilling buckets, `"retry in 43s"` for active `429`s;
    /// `None` once the timestamp has passed.
    pub fn countdown(&self, now_ms: u64) -> Option<String> {
        let prefix = if self.rate_limited { "retry in" } else { "full in" };
        countdown_text(prefix, self.resets_at_ms, now_ms)
    }
}

/// Snapshot of visible usage values so periodic UI ticks repaint only when
/// a rendered metric, count, rate-limit state, or countdown text changes.
pub type DisplaySig = [Option<(Option<Metric>, u64, u64, bool, Option<String>)>; 2];

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

    /// Load persisted readings from disk, dropping any older than 24 hours and
    /// any expired `429` cooldowns.
    pub fn load() -> Self {
        let now = now_ms();
        let mut usage: Self = std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let clean = |mut r: Reading| -> Option<Reading> {
            if let Some(hit) = &r.rate_limit
                && !hit.is_active(now)
            {
                r.rate_limit = None;
            }
            let alive = (r.has_headers() || r.rate_limit.is_some())
                && now.saturating_sub(r.latest_at()) < DAY_MS;
            alive.then_some(r)
        };
        usage.transcription = usage.transcription.and_then(clean);
        usage.cleanup = usage.cleanup.and_then(clean);
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
    /// hit a `429` or newly crossed into low quota (<10% left).
    pub fn merge_and_take_warnings(&mut self, incoming: &Self, now_ms: u64) -> Self {
        let mut changed = false;
        let mut warnings = Self::default();
        if let Some(reading) = &incoming.transcription {
            merge_slot(&mut self.transcription, reading);
            let row = self.transcription.as_ref().and_then(|r| r.row_at(now_ms));
            let rate_limited = row.as_ref().is_some_and(|r| r.rate_limited);
            let low = row.as_ref().is_some_and(|r| r.warn);
            if rate_limited {
                self.warned_transcription = true;
                warnings.transcription = self.transcription.clone();
            } else if low {
                if !self.warned_transcription {
                    self.warned_transcription = true;
                    warnings.transcription = self.transcription.clone();
                }
            } else {
                self.warned_transcription = false;
            }
            changed = true;
        }
        if let Some(reading) = &incoming.cleanup {
            merge_slot(&mut self.cleanup, reading);
            let row = self.cleanup.as_ref().and_then(|r| r.row_at(now_ms));
            let rate_limited = row.as_ref().is_some_and(|r| r.rate_limited);
            let low = row.as_ref().is_some_and(|r| r.warn);
            if rate_limited {
                self.warned_cleanup = true;
                warnings.cleanup = self.cleanup.clone();
            } else if low {
                if !self.warned_cleanup {
                    self.warned_cleanup = true;
                    warnings.cleanup = self.cleanup.clone();
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
            Role::Transcription => merge_slot(&mut self.transcription, &reading),
            Role::Cleanup => merge_slot(&mut self.cleanup, &reading),
        }
    }

    pub fn row_for(&self, role: Role, now_ms: u64) -> Option<UsageRow> {
        match role {
            Role::Transcription => self.transcription.as_ref().and_then(|r| r.row_at(now_ms)),
            Role::Cleanup => self.cleanup.as_ref().and_then(|r| r.row_at(now_ms)),
        }
    }

    pub fn rows_at(&self, now_ms: u64) -> Vec<UsageRow> {
        let mut out = Vec::with_capacity(2);
        if let Some(row) = self.row_for(Role::Transcription, now_ms) {
            out.push(row);
        }
        if let Some(row) = self.row_for(Role::Cleanup, now_ms) {
            out.push(row);
        }
        out
    }

    pub fn is_empty_at(&self, now_ms: u64) -> bool {
        self.rows_at(now_ms).is_empty()
    }

    /// Snapshot of visible usage values so periodic UI ticks only repaint
    /// when a rendered metric, count, or countdown actually changes.
    pub fn display_signature(&self, now_ms: u64) -> DisplaySig {
        let sig = |role: Role| {
            self.row_for(role, now_ms).map(|r| {
                let text = r.countdown(now_ms);
                (r.metric, r.used, r.remaining, r.rate_limited, text)
            })
        };
        [sig(Role::Transcription), sig(Role::Cleanup)]
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
                rate_limit: None,
            }),
            cleanup: Some(Reading {
                model: crate::engines::groq::TIDY_MODEL.into(),
                role: Role::Cleanup,
                requests_limit: Some(1_000),
                requests_remaining: Some(cl_rem),
                requests_reset: Some(cl_rst),
                tokens_limit: Some(8_000),
                tokens_remaining: Some(8_000),
                tokens_reset: None,
                read_at: now,
                rate_limit: None,
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
                rate_limit: None,
            }),
            cleanup: Some(Reading {
                model: crate::engines::groq::TIDY_MODEL.into(),
                role: Role::Cleanup,
                requests_limit: Some(1_000),
                requests_remaining: Some(62),
                requests_reset: Some(4_320.0),
                tokens_limit: Some(8_000),
                tokens_remaining: Some(8_000),
                tokens_reset: None,
                read_at: now,
                rate_limit: None,
            }),
            warned_transcription: false,
            warned_cleanup: false,
        }
    }
}

/// Merge a new reading into an existing role slot. A `429` with no rate-limit
/// headers preserves the prior reading's header snapshot while attaching the
/// cooldown, so the bar recovers cleanly once the `429` expires.
fn merge_slot(slot: &mut Option<Reading>, incoming: &Reading) {
    if incoming.rate_limit.is_some()
        && !incoming.has_headers()
        && let Some(existing) = slot.as_mut()
        && existing.model == incoming.model
    {
        existing.rate_limit = incoming.rate_limit.clone();
        return;
    }
    *slot = Some(incoming.clone());
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
}

fn parse_retry_after(text: &str) -> Option<f64> {
    let text = text.trim();
    if let Ok(secs) = text.parse::<f64>()
        && secs.is_finite()
        && secs >= 0.0
    {
        return Some(secs);
    }
    duration_secs(text)
}

#[derive(Default)]
struct ParsedRateLimitBody {
    metric: Option<Metric>,
    limit: Option<u64>,
    used: Option<u64>,
    retry_after: Option<f64>,
}

/// Extract the tripped limit, cap/usage counts, and retry duration from Groq's
/// `429` JSON error body, e.g.:
/// `"Rate limit reached for model ... on tokens per day (TPD): Limit 500000, Used 499999, Requested 120. Please try again in 1m30.5s."`
fn parse_rate_limit_body(body: &str) -> ParsedRateLimitBody {
    let owned: String;
    let message = if let Ok(value) = serde_json::from_str::<Value>(body)
        && let Some(msg) = value["error"]["message"].as_str()
    {
        owned = msg.to_string();
        owned.as_str()
    } else {
        body
    };
    let lower = message.to_ascii_lowercase();
    let metric = if lower.contains("(tpd)") || lower.contains("tokens per day") {
        Some(Metric::TokensPerDay)
    } else if lower.contains("(tpm)") || lower.contains("tokens per minute") {
        Some(Metric::TokensPerMinute)
    } else if lower.contains("(rpd)") || lower.contains("requests per day") {
        Some(Metric::RequestsPerDay)
    } else if lower.contains("(rpm)") || lower.contains("requests per minute") {
        Some(Metric::RequestsPerMinute)
    } else if lower.contains("(ash)") || lower.contains("audio seconds per hour") {
        Some(Metric::AudioSecondsPerHour)
    } else if lower.contains("(asd)") || lower.contains("audio seconds per day") {
        Some(Metric::AudioSecondsPerDay)
    } else {
        None
    };

    let limit = extract_number_after(&lower, "limit ");
    let used = extract_number_after(&lower, "used ");
    let retry_after = extract_duration_after(&lower, "please try again in ")
        .or_else(|| extract_duration_after(&lower, "try again in "));

    ParsedRateLimitBody { metric, limit, used, retry_after }
}

fn extract_number_after(text: &str, prefix: &str) -> Option<u64> {
    let mut slice = text;
    while let Some(pos) = slice.find(prefix) {
        let rest = slice[pos + prefix.len()..].trim_start();
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == ',')
            .filter(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty()
            && let Ok(n) = digits.parse()
        {
            return Some(n);
        }
        slice = &slice[pos + prefix.len()..];
    }
    None
}

fn extract_duration_after(text: &str, prefix: &str) -> Option<f64> {
    let start = text.find(prefix)? + prefix.len();
    let rest = text[start..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .unwrap_or(rest.len());
    let token = rest[..end].trim_end_matches('.');
    duration_secs(token)
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

fn format_duration_ms(ms: u64) -> Option<String> {
    let seconds = (ms as f64 / 1000.0).round() as u64;
    if seconds == 0 {
        return None;
    }
    if seconds < 60 {
        return Some(format!("{seconds}s"));
    }
    let minutes = (seconds + 30) / 60;
    if minutes < 60 {
        return Some(format!("{minutes}m"));
    }
    let hours = minutes / 60;
    let rest = minutes % 60;
    if rest == 0 {
        Some(format!("{hours}h"))
    } else {
        Some(format!("{hours}h {rest}m"))
    }
}

/// `"{prefix} 3h 12m"`, `"{prefix} 43s"`; `None` once the target has passed.
pub fn countdown_text(prefix: &str, target_ms: Option<u64>, now_ms: u64) -> Option<String> {
    let target = target_ms?;
    if target <= now_ms {
        return None;
    }
    let dur = format_duration_ms(target - now_ms)?;
    Some(format!("{prefix} {dur}"))
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
    fn picks_tightest_active_bucket_and_refills_over_time() {
        // Real Groq Qwen reading: 8/1000 RPD (0.8%, refills in 691.2s) and 682/8000 TPM (8.5%, refills in 5.115s).
        let read_at = 1_000_000;
        let reading = Reading::from_headers(
            Role::Cleanup,
            "qwen/qwen3.8-27b",
            &headers(&[
                ("X-RateLimit-Limit-Requests", "1000"),
                ("x-ratelimit-remaining-requests", "992"),
                ("x-ratelimit-reset-requests", "11m31.2s"),
                ("x-ratelimit-limit-tokens", "8000"),
                ("x-ratelimit-remaining-tokens", "7318"),
                ("x-ratelimit-reset-tokens", "5.115s"),
            ]),
            read_at,
        )
        .unwrap();

        // Immediately after the call, TPM (8.5%) is tighter than RPD (0.8%).
        let at_0 = reading.row_at(read_at).unwrap();
        assert_eq!(at_0.metric, Some(Metric::TokensPerMinute));
        assert_eq!(at_0.used, 682);
        assert_eq!(at_0.remaining, 7318);
        assert_eq!(at_0.countdown(read_at).as_deref(), Some("full in 5s"));

        // After 6s, TPM has fully refilled to 0%, so RPD becomes the active bucket.
        let at_6s = reading.row_at(read_at + 6_000).unwrap();
        assert_eq!(at_6s.metric, Some(Metric::RequestsPerDay));
        assert_eq!(at_6s.used, 8);
        assert_eq!(at_6s.remaining, 992);
        assert_eq!(at_6s.countdown(read_at + 6_000).as_deref(), Some("full in 11m"));

        // Halfway through the RPD refill (~345.6s), half the requests have refilled.
        let at_half = reading.row_at(read_at + 345_600).unwrap();
        assert_eq!(at_half.metric, Some(Metric::RequestsPerDay));
        assert_eq!(at_half.used, 4);
        assert_eq!(at_half.remaining, 996);

        // Once the full reset time has elapsed, bucket returns to 0 used / 1000 left (prefers RPD on tie).
        let at_full = reading.row_at(read_at + 700_000).unwrap();
        assert_eq!(at_full.metric, Some(Metric::RequestsPerDay));
        assert_eq!(at_full.used, 0);
        assert_eq!(at_full.remaining, 1000);
        assert_eq!(at_full.countdown(read_at + 700_000), None);
    }

    #[test]
    fn warns_when_tpm_drops_under_tenth_even_if_rpd_is_healthy() {
        let read_at = 5_000;
        let reading = Reading::from_headers(
            Role::Cleanup,
            "qwen/qwen3.8-27b",
            &headers(&[
                ("x-ratelimit-limit-requests", "1000"),
                ("x-ratelimit-remaining-requests", "990"),
                ("x-ratelimit-reset-requests", "14m24s"),
                ("x-ratelimit-limit-tokens", "8000"),
                ("x-ratelimit-remaining-tokens", "500"),
                ("x-ratelimit-reset-tokens", "43s"),
            ]),
            read_at,
        )
        .unwrap();

        let row = reading.row_at(read_at).unwrap();
        assert_eq!(row.metric, Some(Metric::TokensPerMinute));
        assert_eq!(row.used, 7500);
        assert_eq!(row.remaining, 500);
        assert!(row.warn);
        assert!(!row.rate_limited);
        assert_eq!(row.countdown(read_at).as_deref(), Some("full in 43s"));
    }

    #[test]
    fn parses_429_body_for_tpd_ash_asd_rpm_and_expires_after_retry() {
        let at = 100_000;
        let tpd_body = r#"{"error":{"message":"Rate limit reached for model `qwen/qwen3.8-27b` in organization `org_1` service tier `on_demand` on tokens per day (TPD): Limit 500000, Used 499950, Requested 120. Please try again in 2m15s.","type":"tokens","code":"rate_limit_exceeded"}}"#;
        let reading = Reading::from_rate_limit(
            Role::Cleanup,
            "qwen/qwen3.8-27b",
            &headers(&[("retry-after", "135")]),
            tpd_body,
            at,
        );
        assert_eq!(
            reading.rate_limit_message(),
            "Groq tokens/day limit reached. Try again in 2m."
        );
        let row = reading.row_at(at).unwrap();
        assert_eq!(row.metric, Some(Metric::TokensPerDay));
        assert_eq!(row.limit, 500_000);
        assert_eq!(row.used, 499_950);
        assert!(row.rate_limited);
        assert!(row.exhausted());
        assert!(row.warn);
        assert_eq!(row.countdown(at).as_deref(), Some("retry in 2m"));

        // After retry passes and with no header buckets, row clears.
        assert_eq!(reading.row_at(at + 135_000), None);

        // Whisper audio seconds per hour (ASH).
        let ash_body = r#"{"error":{"message":"Rate limit reached for model `whisper-large-v3-turbo` on audio seconds per hour (ASH): Limit 7200, Used 7190, Requested 25. Please try again in 45s."}}"#;
        let ash = Reading::from_rate_limit(Role::Transcription, "whisper-large-v3-turbo", &[], ash_body, at);
        assert_eq!(ash.rate_limit_message(), "Groq audio/hour limit reached. Try again in 45s.");
        let ash_row = ash.row_at(at).unwrap();
        assert_eq!(ash_row.metric, Some(Metric::AudioSecondsPerHour));
        assert_eq!(ash_row.limit, 7200);
        assert_eq!(ash_row.used, 7190);
        assert_eq!(ash_row.countdown(at).as_deref(), Some("retry in 45s"));

        // Audio seconds per day (ASD) & RPM.
        let asd = Reading::from_rate_limit(
            Role::Transcription,
            "whisper-large-v3-turbo",
            &[],
            "Rate limit reached on audio seconds per day (ASD): Limit 28800, Used 28800. Please try again in 1h5m.",
            at,
        );
        assert_eq!(asd.row_at(at).unwrap().metric, Some(Metric::AudioSecondsPerDay));
        assert_eq!(asd.rate_limit_message(), "Groq audio/day limit reached. Try again in 1h 5m.");

        let rpm = Reading::from_rate_limit(
            Role::Transcription,
            "whisper-large-v3-turbo",
            &headers(&[("retry-after", "12")]),
            "Rate limit reached on requests per minute (RPM): Limit 20, Used 20.",
            at,
        );
        assert_eq!(rpm.row_at(at).unwrap().metric, Some(Metric::RequestsPerMinute));
        assert_eq!(rpm.rate_limit_message(), "Groq requests/min limit reached. Try again in 12s.");
    }

    #[test]
    fn rate_limit_without_headers_preserves_prior_bucket_after_cooldown() {
        let mut stored = GroqUsage::default();
        let ok = Reading::from_headers(
            Role::Cleanup,
            "qwen/qwen3.8-27b",
            &headers(&[
                ("x-ratelimit-limit-requests", "1000"),
                ("x-ratelimit-remaining-requests", "990"),
                ("x-ratelimit-reset-requests", "14m24s"),
            ]),
            10_000,
        )
        .unwrap();
        stored.merge_and_take_warnings(&GroqUsage { cleanup: Some(ok), ..Default::default() }, 10_000);

        let hit = Reading::from_rate_limit(
            Role::Cleanup,
            "qwen/qwen3.8-27b",
            &headers(&[("retry-after", "30")]),
            "Rate limit reached on tokens per day (TPD): Limit 500000, Used 500000.",
            20_000,
        );
        let warn = stored.merge_and_take_warnings(&GroqUsage { cleanup: Some(hit), ..Default::default() }, 20_000);
        assert!(warn.cleanup.is_some());

        // During cooldown: shows TPD 429.
        let during = stored.row_for(Role::Cleanup, 25_000).unwrap();
        assert!(during.rate_limited);
        assert_eq!(during.metric, Some(Metric::TokensPerDay));

        // After cooldown: falls back to the preserved RPD bucket.
        let after = stored.row_for(Role::Cleanup, 60_000).unwrap();
        assert!(!after.rate_limited);
        assert_eq!(after.metric, Some(Metric::RequestsPerDay));
        assert_eq!(after.limit, 1000);
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
        assert_eq!(zero.row_at(1), None);
    }

    #[test]
    fn countdown_formats_seconds_minutes_and_hours() {
        let base = 10_000;
        assert_eq!(countdown_text("full in", Some(base + 43_000), base).as_deref(), Some("full in 43s"));
        assert_eq!(countdown_text("full in", Some(base + 180_000), base).as_deref(), Some("full in 3m"));
        assert_eq!(countdown_text("retry in", Some(base + 3_600_000), base).as_deref(), Some("retry in 1h"));
        assert_eq!(countdown_text("full in", Some(base + 11_520_000), base).as_deref(), Some("full in 3h 12m"));
        assert_eq!(countdown_text("full in", Some(base), base), None);
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
            rate_limit: None,
        };
        let mut stored = GroqUsage::default();

        // Both healthy: no warning.
        let w1 = stored.merge_and_take_warnings(
            &GroqUsage {
                transcription: Some(mk(Role::Transcription, 1500, 2000)),
                cleanup: Some(mk(Role::Cleanup, 800, 1000)),
                ..Default::default()
            },
            10_000,
        );
        assert!(w1.is_empty_at(10_000));

        // Cleanup drops below 10%: warns only Cleanup.
        let w2 = stored.merge_and_take_warnings(
            &GroqUsage {
                transcription: Some(mk(Role::Transcription, 1400, 2000)),
                cleanup: Some(mk(Role::Cleanup, 90, 1000)),
                ..Default::default()
            },
            10_000,
        );
        assert!(w2.transcription.is_none());
        assert!(w2.cleanup.is_some());

        // Cleanup stays below 10%: does NOT warn again.
        let w3 = stored.merge_and_take_warnings(
            &GroqUsage {
                transcription: Some(mk(Role::Transcription, 1300, 2000)),
                cleanup: Some(mk(Role::Cleanup, 80, 1000)),
                ..Default::default()
            },
            10_000,
        );
        assert!(w3.is_empty_at(10_000));

        // Transcription now also drops below 10%: warns only Transcription.
        let w4 = stored.merge_and_take_warnings(
            &GroqUsage {
                transcription: Some(mk(Role::Transcription, 150, 2000)),
                cleanup: Some(mk(Role::Cleanup, 70, 1000)),
                ..Default::default()
            },
            10_000,
        );
        assert!(w4.transcription.is_some());
        assert!(w4.cleanup.is_none());

        // Next day: Cleanup recovers to >=10%, then later drops below 10% again -> warns once more.
        let w5 = stored.merge_and_take_warnings(
            &GroqUsage { cleanup: Some(mk(Role::Cleanup, 995, 1000)), ..Default::default() },
            10_000,
        );
        assert!(w5.is_empty_at(10_000));
        assert!(!stored.warned_cleanup);
        assert!(stored.warned_transcription);

        let w6 = stored.merge_and_take_warnings(
            &GroqUsage { cleanup: Some(mk(Role::Cleanup, 50, 1000)), ..Default::default() },
            10_000,
        );
        assert!(w6.cleanup.is_some());
        assert!(w6.transcription.is_none());
    }
}
