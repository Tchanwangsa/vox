//! Dictations kept as data to tune the models on later: each clip's audio,
//! exactly as the engine heard it, and one line in a manifest saying what was
//! heard, what was typed and which models did each.
//!
//! `recordings/audio/<id>.wav` is 16 kHz mono 16-bit PCM. `manifest.jsonl`
//! has one JSON object a line, appended and never rewritten, with `audio` as
//! a path relative to `recordings/`: the shape speech datasets load as is.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::config::{data_dir, Avenue};
use crate::engines::{groq, Local, Outcome};

const AUDIO: &str = "audio";
const MANIFEST: &str = "manifest.jsonl";

pub fn dir() -> PathBuf {
    data_dir().join("recordings")
}

/// One step of a dictation: which avenue took it, with which model.
#[derive(Debug, Serialize)]
struct Step {
    avenue: Avenue,
    model: String,
}

/// A manifest line.
#[derive(Debug, Serialize)]
pub struct Record {
    /// Milliseconds since the Unix epoch when it was kept; also the clip's name.
    id: String,
    /// Seconds since the Unix epoch.
    at: u64,
    audio: String,
    seconds: f32,
    /// The language setting it was heard under: a Whisper code, or `auto`.
    language: String,
    /// The transcript, before tidying.
    heard: String,
    /// What was typed.
    text: String,
    heard_by: Step,
    /// `None` when the transcript was typed as heard.
    tidied_by: Option<Step>,
}

impl Record {
    pub fn new(outcome: &Outcome, seconds: f32, language: &str, local: &Local) -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let id = now.as_millis().to_string();
        let stem = |path: &Option<PathBuf>| {
            path.as_deref()
                .and_then(Path::file_stem)
                .map_or_else(|| "unknown".into(), |stem| stem.to_string_lossy().into_owned())
        };
        let heard_by = Step {
            avenue: outcome.avenue,
            model: outcome.heard_model.clone(),
        };
        let tidied_by = outcome.tidied_by.map(|avenue| Step {
            avenue,
            model: match avenue {
                Avenue::Cloud => groq::TIDY_MODEL.into(),
                Avenue::Local => stem(&local.s1_model),
            },
        });
        Self {
            audio: format!("{AUDIO}/{id}.wav"),
            id,
            at: now.as_secs(),
            seconds,
            language: language.into(),
            heard: outcome.heard.clone(),
            text: outcome.text.clone(),
            heard_by,
            tidied_by,
        }
    }
}

/// Save the clip, then its manifest line, so a line never names audio that
/// is not there.
pub fn keep(wav: &[u8], record: &Record) -> Result<(), String> {
    keep_in(&dir(), wav, record)
}

fn keep_in(dir: &Path, wav: &[u8], record: &Record) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("Could not keep the recording: {e}");
    std::fs::create_dir_all(dir.join(AUDIO)).map_err(failed)?;
    std::fs::write(dir.join(&record.audio), wav).map_err(failed)?;
    let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(MANIFEST))
        .and_then(|mut manifest| manifest.write_all(line.as_bytes()))
        .map_err(failed)
}

/// How much is kept: the clips, and their size on disk.
#[derive(Debug, Clone, Copy, Default)]
pub struct Kept {
    pub clips: usize,
    pub bytes: u64,
}

impl Kept {
    pub fn count() -> Self {
        let Ok(entries) = std::fs::read_dir(dir().join(AUDIO)) else {
            return Self::default();
        };
        entries
            .flatten()
            .filter_map(|entry| entry.metadata().ok())
            .filter(|meta| meta.is_file())
            .fold(Self::default(), |kept, meta| Self {
                clips: kept.clips + 1,
                bytes: kept.bytes + meta.len(),
            })
    }

    pub fn add(&mut self, bytes: usize) {
        self.clips += 1;
        self.bytes += bytes as u64;
    }
}

/// The recordings folder in Finder.
pub fn reveal() {
    let dir = dir();
    std::fs::create_dir_all(&dir).ok();
    std::process::Command::new("open").arg(dir).spawn().ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_clip_has_its_audio_and_one_manifest_line() {
        let dir = std::env::temp_dir().join(format!("vox-recordings-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let outcome = Outcome {
            text: "Send the Q4 report.".into(),
            heard: "um send the q4 report".into(),
            avenue: Avenue::Cloud,
            heard_model: groq::HEAR_MODEL.into(),
            tidied_by: Some(Avenue::Local),
            note: None,
        };
        let local = Local {
            s1_model: Some("/models/s1-mini-q4_k_m.gguf".into()),
            ..Local::default()
        };
        let record = Record::new(&outcome, 2.5, "auto", &local);
        keep_in(&dir, b"RIFF", &record).unwrap();
        keep_in(&dir, b"RIFF", &record).unwrap();

        let manifest = std::fs::read_to_string(dir.join(MANIFEST)).unwrap();
        let lines: Vec<serde_json::Value> =
            manifest.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        let line = &lines[0];
        assert_eq!(std::fs::read(dir.join(line["audio"].as_str().unwrap())).unwrap(), b"RIFF");
        assert_eq!(line["heard"], "um send the q4 report");
        assert_eq!(line["text"], "Send the Q4 report.");
        assert_eq!(line["heard_by"]["avenue"], "cloud");
        assert_eq!(line["heard_by"]["model"], "whisper-large-v3-turbo");
        assert_eq!(line["tidied_by"]["model"], "s1-mini-q4_k_m");
        std::fs::remove_dir_all(&dir).ok();
    }
}
