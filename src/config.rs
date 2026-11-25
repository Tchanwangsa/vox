//! What Vox remembers between launches: its settings and the dictation
//! history, both JSON in the app's data directory.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const APP_ID: &str = "com.tchan.vox";

/// `~/Library/Application Support/com.tchan.vox`, created on first use.
/// `VOX_DATA_DIR` points a development run somewhere else.
pub fn data_dir() -> PathBuf {
    let dir = std::env::var_os("VOX_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_ID)
    });
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// The two ways a dictation is heard and tidied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Avenue {
    /// Groq: Whisper large v3 turbo, tidied by Qwen.
    Cloud,
    /// This Mac: whisper.cpp, tidied by S1-mini by Superwhisper.
    Local,
}

impl Avenue {
    pub fn other(self) -> Self {
        match self {
            Self::Cloud => Self::Local,
            Self::Local => Self::Cloud,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cloud => "Groq",
            Self::Local => "This Mac",
        }
    }
}

/// A family of speech models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Whisper,
    Cohere,
}

impl Family {
    pub const ALL: [Family; 2] = [Self::Whisper, Self::Cohere];

    pub fn label(self) -> &'static str {
        match self {
            Self::Whisper => "OpenAI Whisper",
            Self::Cohere => "Cohere Transcribe",
        }
    }
}

/// The speech model used when transcription runs on this Mac: one variant of
/// a family. The bare `whisper` and `cohere` of older settings files still load.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LocalModel {
    #[serde(rename = "whisper_tiny")]
    WhisperTiny,
    #[serde(rename = "whisper_base")]
    WhisperBase,
    #[serde(rename = "whisper_small")]
    WhisperSmall,
    #[serde(rename = "whisper_medium")]
    WhisperMedium,
    #[default]
    #[serde(rename = "whisper_large_v3_turbo_q5_0", alias = "whisper")]
    WhisperLargeV3TurboQ5,
    #[serde(rename = "whisper_large_v3_turbo")]
    WhisperLargeV3Turbo,
    #[serde(rename = "whisper_large_v3")]
    WhisperLargeV3,
    #[serde(rename = "cohere_4bit", alias = "cohere")]
    Cohere4bit,
    #[serde(rename = "cohere_8bit")]
    Cohere8bit,
    #[serde(rename = "cohere_fp16")]
    CohereFp16,
}

impl LocalModel {
    pub fn family(self) -> Family {
        match self {
            Self::Cohere4bit | Self::Cohere8bit | Self::CohereFp16 => Family::Cohere,
            _ => Family::Whisper,
        }
    }
}

/// The global shortcut that starts and stops a dictation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shortcut {
    CtrlOptSpace,
    OptSpace,
    CmdShiftSpace,
}

impl Shortcut {
    pub const ALL: [Shortcut; 3] = [Self::CtrlOptSpace, Self::OptSpace, Self::CmdShiftSpace];

    /// The keys as keycap labels, in the order they are pressed.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Self::CtrlOptSpace => &["⌃", "⌥", "Space"],
            Self::OptSpace => &["⌥", "Space"],
            Self::CmdShiftSpace => &["⌘", "⇧", "Space"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub avenue: Avenue,
    pub local_model: LocalModel,
    /// The variant that was ready when `local_model` was last changed. It
    /// answers while the chosen one is still downloading.
    pub last_ready_model: Option<LocalModel>,
    /// Tidy the transcript (fillers, self-corrections, punctuation) before typing it.
    pub tidy: bool,
    /// When the chosen avenue is not set up or fails, the other one answers.
    pub fallback: bool,
    pub shortcut: Shortcut,
    /// Paste into the app in front; off leaves the text on the clipboard.
    pub type_into_app: bool,
    /// Put back whatever was on the clipboard once the text is typed.
    pub keep_clipboard: bool,
    /// A Whisper language code, or `auto`.
    pub language: String,
    /// Keep each dictation's audio and result (`crate::recordings`).
    pub keep_recordings: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            avenue: Avenue::Local,
            local_model: LocalModel::default(),
            last_ready_model: None,
            tidy: true,
            fallback: true,
            shortcut: Shortcut::CtrlOptSpace,
            type_into_app: true,
            keep_clipboard: true,
            language: "auto".into(),
            keep_recordings: true,
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        data_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            std::fs::write(Self::path(), json).ok();
        }
    }
}

/// One finished dictation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Seconds since the Unix epoch.
    pub at: u64,
    /// What was typed.
    pub text: String,
    /// What the engine heard, before tidying.
    pub heard: String,
    pub avenue: Avenue,
    /// How long the speaker talked, in seconds.
    pub spoken_secs: f32,
    /// The app in front when the dictation began: where it was typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

impl Entry {
    pub fn words(&self) -> usize {
        self.text.split_whitespace().count()
    }
}

const KEPT: usize = 300;

#[derive(Default)]
pub struct History {
    /// Oldest first.
    pub entries: Vec<Entry>,
}

impl History {
    fn path() -> PathBuf {
        data_dir().join("history.json")
    }

    pub fn load() -> Self {
        let entries = std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { entries }
    }

    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        let extra = self.entries.len().saturating_sub(KEPT);
        self.entries.drain(..extra);
        self.save();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(&self.entries) {
            std::fs::write(Self::path(), json).ok();
        }
    }

    /// Words and speaking time over the last seven days.
    pub fn week(&self) -> (usize, f32) {
        let since = now().saturating_sub(7 * 24 * 3600);
        self.entries
            .iter()
            .filter(|e| e.at >= since)
            .fold((0, 0.0), |(words, secs), e| {
                (words + e.words(), secs + e.spoken_secs)
            })
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_settings_default_to_whisper_and_keep_other_preferences() {
        let settings: Settings = serde_json::from_str(r#"{"avenue":"cloud","tidy":false}"#).unwrap();
        assert_eq!(settings.local_model, LocalModel::WhisperLargeV3TurboQ5);
        assert_eq!(settings.last_ready_model, None);
        assert_eq!(settings.avenue, Avenue::Cloud);
        assert!(!settings.tidy);
    }

    #[test]
    fn older_model_names_map_to_the_variants_they_meant() {
        let load = |name: &str| -> LocalModel {
            let json = format!(r#"{{"local_model":"{name}","last_ready_model":"{name}"}}"#);
            let settings: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(settings.last_ready_model, Some(settings.local_model));
            settings.local_model
        };
        assert_eq!(load("whisper"), LocalModel::WhisperLargeV3TurboQ5);
        assert_eq!(load("cohere"), LocalModel::Cohere4bit);
    }

    #[test]
    fn chosen_variant_survives_a_save_and_load() {
        for model in [LocalModel::WhisperMedium, LocalModel::Cohere8bit, LocalModel::CohereFp16] {
            let settings = Settings { local_model: model, ..Settings::default() };
            let json = serde_json::to_string(&settings).unwrap();
            assert_eq!(serde_json::from_str::<Settings>(&json).unwrap().local_model, model);
        }
    }
}
