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

/// Modifier keys for a global dictation shortcut.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShortcutModifiers {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub command: bool,
}

impl ShortcutModifiers {
    pub const NONE: Self = Self { control: false, alt: false, shift: false, command: false };
    pub const ALT: Self = Self { alt: true, ..Self::NONE };
    #[cfg(test)]
    pub const SHIFT: Self = Self { shift: true, ..Self::NONE };
    pub const COMMAND: Self = Self { command: true, ..Self::NONE };

    pub const fn is_empty(self) -> bool {
        !self.control && !self.alt && !self.shift && !self.command
    }

    /// True when at least one non-Shift modifier (⌃, ⌥, or ⌘) is held.
    pub const fn has_primary(self) -> bool {
        self.control || self.alt || self.command
    }

    /// Modifier keycap symbols in macOS menu order (⌃ ⌥ ⇧ ⌘).
    pub fn symbols(self) -> Vec<&'static str> {
        let mut out = Vec::with_capacity(4);
        if self.control {
            out.push("⌃");
        }
        if self.alt {
            out.push("⌥");
        }
        if self.shift {
            out.push("⇧");
        }
        if self.command {
            out.push("⌘");
        }
        out
    }
}

/// Non-modifier key of a global dictation shortcut. Every variant maps to a
/// macOS Carbon virtual keycode supported by `global-hotkey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutKey {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Digit0, Digit1, Digit2, Digit3, Digit4,
    Digit5, Digit6, Digit7, Digit8, Digit9,
    Space,
    Tab,
    Enter,
    Backspace,
    Delete,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Backquote,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Comma,
    Period,
    Slash,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10,
    F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
}

impl ShortcutKey {
    #[cfg(test)]
    pub const ALL: [Self; 81] = [
        Self::A, Self::B, Self::C, Self::D, Self::E, Self::F, Self::G, Self::H, Self::I,
        Self::J, Self::K, Self::L, Self::M, Self::N, Self::O, Self::P, Self::Q, Self::R,
        Self::S, Self::T, Self::U, Self::V, Self::W, Self::X, Self::Y, Self::Z,
        Self::Digit0, Self::Digit1, Self::Digit2, Self::Digit3, Self::Digit4,
        Self::Digit5, Self::Digit6, Self::Digit7, Self::Digit8, Self::Digit9,
        Self::Space, Self::Tab, Self::Enter, Self::Backspace, Self::Delete, Self::Escape,
        Self::ArrowUp, Self::ArrowDown, Self::ArrowLeft, Self::ArrowRight,
        Self::Home, Self::End, Self::PageUp, Self::PageDown,
        Self::Backquote, Self::Minus, Self::Equal, Self::BracketLeft, Self::BracketRight,
        Self::Backslash, Self::Semicolon, Self::Quote, Self::Comma, Self::Period, Self::Slash,
        Self::F1, Self::F2, Self::F3, Self::F4, Self::F5, Self::F6, Self::F7, Self::F8,
        Self::F9, Self::F10, Self::F11, Self::F12, Self::F13, Self::F14, Self::F15,
        Self::F16, Self::F17, Self::F18, Self::F19, Self::F20,
    ];

    pub const fn is_function_key(self) -> bool {
        matches!(
            self,
            Self::F1 | Self::F2 | Self::F3 | Self::F4 | Self::F5 | Self::F6 | Self::F7
                | Self::F8 | Self::F9 | Self::F10 | Self::F11 | Self::F12 | Self::F13
                | Self::F14 | Self::F15 | Self::F16 | Self::F17 | Self::F18 | Self::F19
                | Self::F20
        )
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "A", Self::B => "B", Self::C => "C", Self::D => "D", Self::E => "E",
            Self::F => "F", Self::G => "G", Self::H => "H", Self::I => "I", Self::J => "J",
            Self::K => "K", Self::L => "L", Self::M => "M", Self::N => "N", Self::O => "O",
            Self::P => "P", Self::Q => "Q", Self::R => "R", Self::S => "S", Self::T => "T",
            Self::U => "U", Self::V => "V", Self::W => "W", Self::X => "X", Self::Y => "Y",
            Self::Z => "Z",
            Self::Digit0 => "0", Self::Digit1 => "1", Self::Digit2 => "2", Self::Digit3 => "3",
            Self::Digit4 => "4", Self::Digit5 => "5", Self::Digit6 => "6", Self::Digit7 => "7",
            Self::Digit8 => "8", Self::Digit9 => "9",
            Self::Space => "Space",
            Self::Tab => "⇥",
            Self::Enter => "↵",
            Self::Backspace => "⌫",
            Self::Delete => "⌦",
            Self::Escape => "Esc",
            Self::ArrowUp => "↑",
            Self::ArrowDown => "↓",
            Self::ArrowLeft => "←",
            Self::ArrowRight => "→",
            Self::Home => "↖",
            Self::End => "↘",
            Self::PageUp => "⇞",
            Self::PageDown => "⇟",
            Self::Backquote => "`",
            Self::Minus => "-",
            Self::Equal => "=",
            Self::BracketLeft => "[",
            Self::BracketRight => "]",
            Self::Backslash => "\\",
            Self::Semicolon => ";",
            Self::Quote => "'",
            Self::Comma => ",",
            Self::Period => ".",
            Self::Slash => "/",
            Self::F1 => "F1", Self::F2 => "F2", Self::F3 => "F3", Self::F4 => "F4",
            Self::F5 => "F5", Self::F6 => "F6", Self::F7 => "F7", Self::F8 => "F8",
            Self::F9 => "F9", Self::F10 => "F10", Self::F11 => "F11", Self::F12 => "F12",
            Self::F13 => "F13", Self::F14 => "F14", Self::F15 => "F15", Self::F16 => "F16",
            Self::F17 => "F17", Self::F18 => "F18", Self::F19 => "F19", Self::F20 => "F20",
        }
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::A => "a", Self::B => "b", Self::C => "c", Self::D => "d", Self::E => "e",
            Self::F => "f", Self::G => "g", Self::H => "h", Self::I => "i", Self::J => "j",
            Self::K => "k", Self::L => "l", Self::M => "m", Self::N => "n", Self::O => "o",
            Self::P => "p", Self::Q => "q", Self::R => "r", Self::S => "s", Self::T => "t",
            Self::U => "u", Self::V => "v", Self::W => "w", Self::X => "x", Self::Y => "y",
            Self::Z => "z",
            Self::Digit0 => "0", Self::Digit1 => "1", Self::Digit2 => "2", Self::Digit3 => "3",
            Self::Digit4 => "4", Self::Digit5 => "5", Self::Digit6 => "6", Self::Digit7 => "7",
            Self::Digit8 => "8", Self::Digit9 => "9",
            Self::Space => "space",
            Self::Tab => "tab",
            Self::Enter => "enter",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::Escape => "escape",
            Self::ArrowUp => "up",
            Self::ArrowDown => "down",
            Self::ArrowLeft => "left",
            Self::ArrowRight => "right",
            Self::Home => "home",
            Self::End => "end",
            Self::PageUp => "pageup",
            Self::PageDown => "pagedown",
            Self::Backquote => "backquote",
            Self::Minus => "minus",
            Self::Equal => "equal",
            Self::BracketLeft => "bracket_left",
            Self::BracketRight => "bracket_right",
            Self::Backslash => "backslash",
            Self::Semicolon => "semicolon",
            Self::Quote => "quote",
            Self::Comma => "comma",
            Self::Period => "period",
            Self::Slash => "slash",
            Self::F1 => "f1", Self::F2 => "f2", Self::F3 => "f3", Self::F4 => "f4",
            Self::F5 => "f5", Self::F6 => "f6", Self::F7 => "f7", Self::F8 => "f8",
            Self::F9 => "f9", Self::F10 => "f10", Self::F11 => "f11", Self::F12 => "f12",
            Self::F13 => "f13", Self::F14 => "f14", Self::F15 => "f15", Self::F16 => "f16",
            Self::F17 => "f17", Self::F18 => "f18", Self::F19 => "f19", Self::F20 => "f20",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "a" => Some(Self::A), "b" => Some(Self::B), "c" => Some(Self::C),
            "d" => Some(Self::D), "e" => Some(Self::E), "f" => Some(Self::F),
            "g" => Some(Self::G), "h" => Some(Self::H), "i" => Some(Self::I),
            "j" => Some(Self::J), "k" => Some(Self::K), "l" => Some(Self::L),
            "m" => Some(Self::M), "n" => Some(Self::N), "o" => Some(Self::O),
            "p" => Some(Self::P), "q" => Some(Self::Q), "r" => Some(Self::R),
            "s" => Some(Self::S), "t" => Some(Self::T), "u" => Some(Self::U),
            "v" => Some(Self::V), "w" => Some(Self::W), "x" => Some(Self::X),
            "y" => Some(Self::Y), "z" => Some(Self::Z),
            "0" => Some(Self::Digit0), "1" => Some(Self::Digit1), "2" => Some(Self::Digit2),
            "3" => Some(Self::Digit3), "4" => Some(Self::Digit4), "5" => Some(Self::Digit5),
            "6" => Some(Self::Digit6), "7" => Some(Self::Digit7), "8" => Some(Self::Digit8),
            "9" => Some(Self::Digit9),
            "space" => Some(Self::Space),
            "tab" => Some(Self::Tab),
            "enter" | "return" => Some(Self::Enter),
            "backspace" => Some(Self::Backspace),
            "delete" => Some(Self::Delete),
            "escape" | "esc" => Some(Self::Escape),
            "up" | "arrow_up" => Some(Self::ArrowUp),
            "down" | "arrow_down" => Some(Self::ArrowDown),
            "left" | "arrow_left" => Some(Self::ArrowLeft),
            "right" | "arrow_right" => Some(Self::ArrowRight),
            "home" => Some(Self::Home),
            "end" => Some(Self::End),
            "pageup" | "page_up" => Some(Self::PageUp),
            "pagedown" | "page_down" => Some(Self::PageDown),
            "backquote" | "`" => Some(Self::Backquote),
            "minus" | "-" => Some(Self::Minus),
            "equal" | "=" => Some(Self::Equal),
            "bracket_left" | "[" => Some(Self::BracketLeft),
            "bracket_right" | "]" => Some(Self::BracketRight),
            "backslash" | "\\" => Some(Self::Backslash),
            "semicolon" | ";" => Some(Self::Semicolon),
            "quote" | "'" => Some(Self::Quote),
            "comma" | "," => Some(Self::Comma),
            "period" | "." => Some(Self::Period),
            "slash" | "/" => Some(Self::Slash),
            "f1" => Some(Self::F1), "f2" => Some(Self::F2), "f3" => Some(Self::F3),
            "f4" => Some(Self::F4), "f5" => Some(Self::F5), "f6" => Some(Self::F6),
            "f7" => Some(Self::F7), "f8" => Some(Self::F8), "f9" => Some(Self::F9),
            "f10" => Some(Self::F10), "f11" => Some(Self::F11), "f12" => Some(Self::F12),
            "f13" => Some(Self::F13), "f14" => Some(Self::F14), "f15" => Some(Self::F15),
            "f16" => Some(Self::F16), "f17" => Some(Self::F17), "f18" => Some(Self::F18),
            "f19" => Some(Self::F19), "f20" => Some(Self::F20),
            _ => None,
        }
    }

    /// Map a GPUI `Keystroke.key` to `(ShortcutKey, implied_shift)`.
    ///
    /// On macOS, GPUI folds `Shift` into shifted digits and punctuation
    /// (for example `⌘⇧1` arrives as `shift = false, key = "!"`), so we
    /// recover the base key and restore the Shift modifier.
    pub fn from_gpui_key(key: &str) -> Option<(Self, bool)> {
        if key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase() {
            return Self::from_token(key).map(|k| (k, true));
        }
        let shifted = match key {
            "!" => Some(Self::Digit1),
            "@" => Some(Self::Digit2),
            "#" => Some(Self::Digit3),
            "$" => Some(Self::Digit4),
            "%" => Some(Self::Digit5),
            "^" => Some(Self::Digit6),
            "&" => Some(Self::Digit7),
            "*" => Some(Self::Digit8),
            "(" => Some(Self::Digit9),
            ")" => Some(Self::Digit0),
            "~" => Some(Self::Backquote),
            "_" => Some(Self::Minus),
            "+" => Some(Self::Equal),
            "{" => Some(Self::BracketLeft),
            "}" => Some(Self::BracketRight),
            "|" => Some(Self::Backslash),
            ":" => Some(Self::Semicolon),
            "\"" => Some(Self::Quote),
            "<" => Some(Self::Comma),
            ">" => Some(Self::Period),
            "?" => Some(Self::Slash),
            _ => None,
        };
        if let Some(key) = shifted {
            return Some((key, true));
        }
        Self::from_token(key).map(|k| (k, false))
    }
}

/// Why a recorded key combination cannot be used as the global shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutError {
    NeedsModifier,
    ReservedPaste,
    ReservedQuit,
    UnsupportedKey,
}

impl ShortcutError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::NeedsModifier => "Include ⌃, ⌥, or ⌘ — or use an F-key.",
            Self::ReservedPaste => "⌘V is used by Vox to paste. Try another.",
            Self::ReservedQuit => "⌘Q quits apps on macOS. Try another.",
            Self::UnsupportedKey => "That key can’t be used as a global shortcut.",
        }
    }
}

/// The global shortcut that starts and stops a dictation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub modifiers: ShortcutModifiers,
    pub key: ShortcutKey,
}

impl Shortcut {
    pub const CTRL_OPT_SPACE: Self = Self::new(
        ShortcutModifiers { control: true, alt: true, ..ShortcutModifiers::NONE },
        ShortcutKey::Space,
    );
    pub const OPT_SPACE: Self = Self::new(ShortcutModifiers::ALT, ShortcutKey::Space);
    pub const CMD_SHIFT_SPACE: Self = Self::new(
        ShortcutModifiers { shift: true, command: true, ..ShortcutModifiers::NONE },
        ShortcutKey::Space,
    );

    pub const PRESETS: [Self; 3] = [Self::CTRL_OPT_SPACE, Self::OPT_SPACE, Self::CMD_SHIFT_SPACE];

    pub const fn new(modifiers: ShortcutModifiers, key: ShortcutKey) -> Self {
        Self { modifiers, key }
    }

    pub fn is_preset(self) -> bool {
        Self::PRESETS.contains(&self)
    }

    pub fn validate(self) -> Result<(), ShortcutError> {
        if !self.key.is_function_key() && !self.modifiers.has_primary() {
            return Err(ShortcutError::NeedsModifier);
        }
        if self == Self::new(ShortcutModifiers::COMMAND, ShortcutKey::V) {
            return Err(ShortcutError::ReservedPaste);
        }
        if self == Self::new(ShortcutModifiers::COMMAND, ShortcutKey::Q) {
            return Err(ShortcutError::ReservedQuit);
        }
        Ok(())
    }

    /// Build and validate a shortcut from a GPUI keystroke.
    pub fn from_gpui_keystroke(
        mut modifiers: ShortcutModifiers,
        key: &str,
    ) -> Result<Self, ShortcutError> {
        let (key, implied_shift) =
            ShortcutKey::from_gpui_key(key).ok_or(ShortcutError::UnsupportedKey)?;
        modifiers.shift |= implied_shift;
        let shortcut = Self { modifiers, key };
        shortcut.validate()?;
        Ok(shortcut)
    }

    /// The keys as keycap labels, in the order they are shown.
    pub fn keys(self) -> Vec<&'static str> {
        let mut out = self.modifiers.symbols();
        out.push(self.key.label());
        out
    }

    /// Human-readable form for inline messages, e.g. `⌃ ⌥ Space` or `⌥ D`.
    pub fn display(self) -> String {
        self.keys().join(" ")
    }

    /// Canonical string written to `settings.json`. The three presets keep
    /// their original names for compatibility.
    pub fn to_spec(self) -> String {
        if self == Self::CTRL_OPT_SPACE {
            return "ctrl_opt_space".into();
        }
        if self == Self::OPT_SPACE {
            return "opt_space".into();
        }
        if self == Self::CMD_SHIFT_SPACE {
            return "cmd_shift_space".into();
        }
        let mut parts = Vec::with_capacity(5);
        if self.modifiers.control {
            parts.push("ctrl");
        }
        if self.modifiers.alt {
            parts.push("opt");
        }
        if self.modifiers.shift {
            parts.push("shift");
        }
        if self.modifiers.command {
            parts.push("cmd");
        }
        parts.push(self.key.token());
        parts.join("+")
    }

    pub fn from_spec(spec: &str) -> Option<Self> {
        let trimmed = spec.trim();
        match trimmed {
            "ctrl_opt_space" => return Some(Self::CTRL_OPT_SPACE),
            "opt_space" => return Some(Self::OPT_SPACE),
            "cmd_shift_space" => return Some(Self::CMD_SHIFT_SPACE),
            _ => {}
        }
        let mut modifiers = ShortcutModifiers::NONE;
        let mut key = None;
        for part in trimmed.split('+') {
            let token = part.trim();
            if token.is_empty() || key.is_some() {
                return None;
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.control = true,
                "opt" | "option" | "alt" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "cmd" | "command" | "meta" | "super" => modifiers.command = true,
                _ => key = Some(ShortcutKey::from_token(token)?),
            }
        }
        let shortcut = Self { modifiers, key: key? };
        shortcut.validate().ok()?;
        Some(shortcut)
    }
}

impl Serialize for Shortcut {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_spec())
    }
}

impl<'de> Deserialize<'de> for Shortcut {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_spec(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid shortcut: {raw}")))
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
    /// The last custom shortcut recorded by the user, remembered if they
    /// temporarily switch back to a preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_shortcut: Option<Shortcut>,
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
            shortcut: Shortcut::CTRL_OPT_SPACE,
            custom_shortcut: None,
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

    #[test]
    fn legacy_preset_shortcuts_deserialize_and_serialize_unchanged() {
        for (raw, expected) in [
            ("ctrl_opt_space", Shortcut::CTRL_OPT_SPACE),
            ("opt_space", Shortcut::OPT_SPACE),
            ("cmd_shift_space", Shortcut::CMD_SHIFT_SPACE),
        ] {
            let json = format!(r#"{{"shortcut":"{raw}"}}"#);
            let settings: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(settings.shortcut, expected);
            assert_eq!(settings.shortcut.to_spec(), raw);
            assert!(settings.shortcut.is_preset());
        }
    }

    #[test]
    fn custom_shortcuts_round_trip_through_settings_json() {
        let cases = [
            (
                Shortcut::new(ShortcutModifiers::ALT, ShortcutKey::D),
                "opt+d",
                vec!["⌥", "D"],
            ),
            (
                Shortcut::new(
                    ShortcutModifiers { control: true, shift: true, ..ShortcutModifiers::NONE },
                    ShortcutKey::Slash,
                ),
                "ctrl+shift+slash",
                vec!["⌃", "⇧", "/"],
            ),
            (
                Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::F5),
                "f5",
                vec!["F5"],
            ),
            (
                Shortcut::new(ShortcutModifiers::SHIFT, ShortcutKey::F12),
                "shift+f12",
                vec!["⇧", "F12"],
            ),
        ];
        for (shortcut, spec, labels) in cases {
            assert!(!shortcut.is_preset());
            assert_eq!(shortcut.to_spec(), spec);
            assert_eq!(shortcut.keys(), labels);
            let settings = Settings {
                shortcut,
                custom_shortcut: Some(shortcut),
                ..Settings::default()
            };
            let json = serde_json::to_string(&settings).unwrap();
            let loaded: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(loaded.shortcut, shortcut);
            assert_eq!(loaded.custom_shortcut, Some(shortcut));
        }
    }

    #[test]
    fn gpui_keystrokes_map_shifted_symbols_and_validate_shortcuts() {
        // GPUI folds Shift into shifted digits/punctuation on macOS.
        let bang = Shortcut::from_gpui_keystroke(ShortcutModifiers::COMMAND, "!").unwrap();
        assert_eq!(
            bang,
            Shortcut::new(
                ShortcutModifiers { shift: true, command: true, ..ShortcutModifiers::NONE },
                ShortcutKey::Digit1,
            )
        );
        assert_eq!(bang.keys(), vec!["⇧", "⌘", "1"]);

        let question = Shortcut::from_gpui_keystroke(ShortcutModifiers::ALT, "?").unwrap();
        assert_eq!(
            question,
            Shortcut::new(
                ShortcutModifiers { alt: true, shift: true, ..ShortcutModifiers::NONE },
                ShortcutKey::Slash,
            )
        );

        // Bare function keys and Shift+function keys are allowed.
        assert_eq!(
            Shortcut::from_gpui_keystroke(ShortcutModifiers::NONE, "f5"),
            Ok(Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::F5))
        );
        assert_eq!(
            Shortcut::from_gpui_keystroke(ShortcutModifiers::SHIFT, "f5"),
            Ok(Shortcut::new(ShortcutModifiers::SHIFT, ShortcutKey::F5))
        );

        // Bare typing keys and Shift-only typing keys are rejected.
        assert_eq!(
            Shortcut::from_gpui_keystroke(ShortcutModifiers::NONE, "a"),
            Err(ShortcutError::NeedsModifier)
        );
        assert_eq!(
            Shortcut::from_gpui_keystroke(ShortcutModifiers::SHIFT, "a"),
            Err(ShortcutError::NeedsModifier)
        );
        assert_eq!(
            Shortcut::from_gpui_keystroke(ShortcutModifiers::NONE, "space"),
            Err(ShortcutError::NeedsModifier)
        );
        assert_eq!(
            Shortcut::from_gpui_keystroke(ShortcutModifiers::SHIFT, "space"),
            Err(ShortcutError::NeedsModifier)
        );

        // Reserved paste and quit shortcuts are rejected.
        assert_eq!(
            Shortcut::from_gpui_keystroke(ShortcutModifiers::COMMAND, "v"),
            Err(ShortcutError::ReservedPaste)
        );
        assert_eq!(
            Shortcut::from_gpui_keystroke(ShortcutModifiers::COMMAND, "q"),
            Err(ShortcutError::ReservedQuit)
        );
        // With an additional modifier they are allowed.
        assert!(Shortcut::from_gpui_keystroke(
            ShortcutModifiers { shift: true, command: true, ..ShortcutModifiers::NONE },
            "v",
        )
        .is_ok());
    }
}
